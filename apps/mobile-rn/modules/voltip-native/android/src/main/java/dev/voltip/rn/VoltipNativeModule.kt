package dev.voltip.rn

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Settings
import android.view.Window
import androidx.core.view.WindowCompat
import com.facebook.react.bridge.ReactApplicationContext
import com.facebook.react.interfaces.ExtraWindowEventListener
import dev.voltip.rn.uniffi.ShellException
import expo.modules.kotlin.exception.CodedException
import expo.modules.kotlin.exception.Exceptions
import expo.modules.kotlin.functions.Coroutine
import expo.modules.kotlin.functions.Queues
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import org.json.JSONObject
import java.util.Collections
import java.util.WeakHashMap

/**
 * `VoltipNative` in JavaScript (`modules/voltip-native/src`): `start()` brings the Rust shell up
 * once per process ([VoltipRuntime]), `invoke(command, argsJson)` runs one of its commands, and the
 * events `onEvent` / `onChannel` carry the core's `UiEvent`s and the level meter's frames while the
 * JavaScript runs. The shell is the UniFFI object [dev.voltip.rn.uniffi.VoltipShell]
 * (`apps/mobile-rn/rust/src/ffi.rs`); [VoltipHost] is its platform. `systemAccent()` is the
 * wallpaper's colour the system themes itself with (Android 12+), which the app's colours follow
 * while its appearance follows the system (docs/mobile-rn.md §5). `setNavigationBarButtons(light)`
 * gives the navigation bar's buttons the theme's contrast, in the activity's window and in every
 * window React Native opens over it (a `Modal`'s dialog); the window itself runs edge to edge, as
 * React Native set it up, and no bar colour is set (Android 15 deprecates them). `bubble…` are the
 * floating bubble's state and options ([BubbleService], docs/dictation.md §20.12).
 *
 * When the app comes to the front the shell checks its relay socket at once
 * ([dev.voltip.rn.uniffi.VoltipShell.reconnectRelay]) instead of waiting for the heartbeat
 * (docs/pairing.md 「重连」); the network callback is [VoltipRuntime]'s.
 */
class VoltipNativeModule : Module() {
    /** The activity in front while this module's JavaScript runs. */
    private val front: () -> Activity? = { appContext.currentActivity }

    /** The navigation bar's buttons the theme asks for: dark on a light theme. */
    private var lightButtons: Boolean? = null

    /** The windows React Native opened over the activity (a `Modal`'s dialog), which start with
     *  the system's own button contrast. */
    private val extraWindows = Collections.newSetFromMap(WeakHashMap<Window, Boolean>())

    private val windowListener = object : ExtraWindowEventListener {
        override fun onExtraWindowCreate(window: Window) {
            extraWindows.add(window)
            lightButtons?.let { window.navigationButtons(it) }
        }

        override fun onExtraWindowDestroy(window: Window) {
            extraWindows.remove(window)
        }
    }

    private val listener = object : VoltipRuntime.Listener {
        override fun onEvent(json: String) {
            sendEvent("onEvent", mapOf("json" to json))
        }

        override fun onChannel(channel: ULong, json: String) {
            sendEvent("onChannel", mapOf("id" to channel.toDouble(), "json" to json))
        }
    }

    override fun definition() = ModuleDefinition {
        Name("VoltipNative")

        Events("onEvent", "onChannel")

        /** `null` once the shell runs, or why it could not start. */
        AsyncFunction("start") {
            val context = appContext.reactContext?.applicationContext ?: throw Exceptions.ReactContextLost()
            VoltipRuntime.activity = front
            try {
                // A second start (the JavaScript reloaded) hands back the running shell; this module
                // takes the old one's place as the JavaScript's listener.
                VoltipRuntime.start(context)
                VoltipRuntime.addListener(listener)
                null
            } catch (e: ShellException) {
                e.message
            }
        }

        OnActivityEntersForeground { VoltipRuntime.shell?.reconnectRelay() }

        OnCreate {
            (appContext.reactContext as? ReactApplicationContext)?.addExtraWindowEventListener(windowListener)
        }

        OnDestroy {
            VoltipRuntime.removeListener(listener)
            if (VoltipRuntime.activity === front) VoltipRuntime.activity = null
            (appContext.reactContext as? ReactApplicationContext)?.removeExtraWindowEventListener(windowListener)
        }

        /** Dark navigation bar buttons for a light theme (`light`), light ones for a dark theme. */
        AsyncFunction("setNavigationBarButtons") { light: Boolean ->
            lightButtons = light
            appContext.currentActivity?.window?.navigationButtons(light)
            extraWindows.forEach { it.navigationButtons(light) }
        }.runOnQueue(Queues.MAIN)

        /** `#rrggbb` of the system's accent (Material You), or `null` before Android 12. */
        Function("systemAccent") {
            val context = appContext.reactContext ?: return@Function null
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) return@Function null
            // The middle of the system's primary palette (tone 50): the colour it seeds from.
            String.format("#%06X", context.getColor(android.R.color.system_accent1_500) and 0xFFFFFF)
        }

        /** The command's answer as JSON; a failure rejects with the shell's error text, as Tauri's invoke does. */
        AsyncFunction("invoke") Coroutine { command: String, args: String ->
            val running = VoltipRuntime.shell ?: throw CodedException(ERROR_CODE, "the shell is not running", null)
            try {
                running.invoke(command, args)
            } catch (e: ShellException) {
                throw CodedException(ERROR_CODE, e.message, e)
            }
        }

        /** The bubble's state as JSON (`BubbleStatus`, src/backend/bubble.tsx). */
        AsyncFunction("bubbleStatus") {
            val context = appContext.reactContext?.applicationContext ?: throw Exceptions.ReactContextLost()
            BubbleService.status(context).toString()
        }

        /**
         * Save the bubble's options (`BubbleConfig` as JSON) and show or hide it. It shows once
         * enabled with every permission granted; the app is in front while this runs, which Android
         * requires of a foreground service that records.
         */
        AsyncFunction("bubbleConfigure") { config: String ->
            val context = appContext.reactContext?.applicationContext ?: throw Exceptions.ReactContextLost()
            val prefs = BubblePrefs(context)
            prefs.save(JSONObject(config))
            BubbleService.apply(context)
            BubbleService.status(context).toString()
        }

        /** The system's 显示在其他应用上层 page for this app. */
        AsyncFunction("bubbleOpenOverlaySettings") {
            val context = appContext.reactContext?.applicationContext ?: throw Exceptions.ReactContextLost()
            val intent = Intent(Settings.ACTION_MANAGE_OVERLAY_PERMISSION, Uri.parse("package:${context.packageName}"))
            val front = appContext.currentActivity
            if (front != null) front.startActivity(intent) else context.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        }

        /** The system's Accessibility settings, where 手势粘贴's service is enabled. */
        AsyncFunction("bubbleOpenAccessibilitySettings") {
            val context = appContext.reactContext?.applicationContext ?: throw Exceptions.ReactContextLost()
            val intent = Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)
            val front = appContext.currentActivity
            if (front != null) front.startActivity(intent) else context.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        }

        /** This app's App info page: on Android 13+ its menu allows the restricted settings of an
         *  app installed from outside an app store (手势粘贴's accessibility switch). */
        AsyncFunction("bubbleOpenAppSettings") {
            val context = appContext.reactContext?.applicationContext ?: throw Exceptions.ReactContextLost()
            val intent = Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:${context.packageName}"))
            val front = appContext.currentActivity
            if (front != null) front.startActivity(intent) else context.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        }

        /** The system's notification settings for this app (a permission refused for good). */
        AsyncFunction("bubbleOpenNotificationSettings") {
            val context = appContext.reactContext?.applicationContext ?: throw Exceptions.ReactContextLost()
            val intent = Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName)
            val front = appContext.currentActivity
            if (front != null) front.startActivity(intent) else context.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        }
    }

    private companion object {
        const val ERROR_CODE = "ERR_VOLTIP"

        /**
         * The navigation bar's buttons for a `light` (or dark) theme. From Android 10 the system keeps
         * them readable itself (its contrast scrim follows this setting). Below 10 React Native's
         * edge-to-edge set-up gave the bar a translucent background for the system's dark mode, and
         * the buttons stay as it set them for that background.
         */
        fun Window.navigationButtons(light: Boolean) {
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) return
            WindowCompat.getInsetsController(this, decorView).isAppearanceLightNavigationBars = light
        }
    }
}
