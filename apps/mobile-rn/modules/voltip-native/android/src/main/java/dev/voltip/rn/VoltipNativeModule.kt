package dev.voltip.rn

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Settings
import dev.voltip.rn.uniffi.ShellException
import expo.modules.kotlin.exception.CodedException
import expo.modules.kotlin.exception.Exceptions
import expo.modules.kotlin.functions.Coroutine
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import org.json.JSONObject

/**
 * `VoltipNative` in JavaScript (`modules/voltip-native/src`): `start()` brings the Rust shell up
 * once per process ([VoltipRuntime]), `invoke(command, argsJson)` runs one of its commands, and the
 * events `onEvent` / `onChannel` carry the core's `UiEvent`s and the level meter's frames while the
 * JavaScript runs. The shell is the UniFFI object [dev.voltip.rn.uniffi.VoltipShell]
 * (`apps/mobile-rn/rust/src/ffi.rs`); [VoltipHost] is its platform. `systemAccent()` is the
 * wallpaper's colour the system themes itself with (Android 12+), which the app's colours follow
 * while its appearance follows the system (docs/mobile-rn.md §5). `bubble…` are the floating
 * bubble's state and options ([BubbleService], docs/dictation.md §20.12).
 *
 * When the app comes to the front the shell checks its relay socket at once
 * ([dev.voltip.rn.uniffi.VoltipShell.reconnectRelay]) instead of waiting for the heartbeat
 * (docs/pairing.md 「重连」); the network callback is [VoltipRuntime]'s.
 */
class VoltipNativeModule : Module() {
    /** The activity in front while this module's JavaScript runs. */
    private val front: () -> Activity? = { appContext.currentActivity }

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

        OnDestroy {
            VoltipRuntime.removeListener(listener)
            if (VoltipRuntime.activity === front) VoltipRuntime.activity = null
        }

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
    }
}
