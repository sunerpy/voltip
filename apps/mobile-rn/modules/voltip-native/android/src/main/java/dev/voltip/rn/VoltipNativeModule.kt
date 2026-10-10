package dev.voltip.rn

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.os.Build
import dev.voltip.rn.uniffi.ShellException
import dev.voltip.rn.uniffi.VoltipShell
import expo.modules.kotlin.exception.CodedException
import expo.modules.kotlin.exception.Exceptions
import expo.modules.kotlin.functions.Coroutine
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import io.crates.keyring.Keyring
import io.crates.keyring.KeyringLog
import java.io.File

/**
 * `VoltipNative` in JavaScript (`modules/voltip-native/src`): `start()` brings the Rust shell up
 * once per process, `invoke(command, argsJson)` runs one of its commands, and the events
 * `onEvent` / `onChannel` carry the core's `UiEvent`s and the level meter's frames. The shell is
 * the UniFFI object [VoltipShell] (`apps/mobile-rn/rust/src/ffi.rs`); [VoltipHost] is its platform.
 * `systemAccent()` is the wallpaper's colour the system themes itself with (Android 12+), which
 * the app's colours follow while its appearance follows the system (docs/mobile-rn.md §5).
 *
 * The phone's only link to its computers is the relay (docs/pairing.md 「只走中继」), and its socket
 * rarely survives a change of network or a long stay in the background: when Android reports
 * another network, or the app comes to the front, the shell checks it at once
 * ([VoltipShell.reconnectRelay]) instead of waiting for the heartbeat (docs/pairing.md 「重连」).
 */
class VoltipNativeModule : Module() {
    @Volatile
    private var shell: VoltipShell? = null
    private var network: ConnectivityManager.NetworkCallback? = null

    override fun definition() = ModuleDefinition {
        Name("VoltipNative")

        Events("onEvent", "onChannel")

        /** `null` once the shell runs, or why it could not start. */
        AsyncFunction("start") {
            val context = appContext.reactContext?.applicationContext ?: throw Exceptions.ReactContextLost()
            // Rust's tracing into logcat, then the Android context `ndk-context` hands the
            // Keystore store and cpal (AAudio); both before the shell starts. Keyring.kt loads
            // libvoltip_rn for these two JNI entries; UniFFI's bindings reach it through JNA.
            KeyringLog.setLog(LOG_FILTER)
            Keyring.initializeNdkContext(context)
            val host = VoltipHost(context, { appContext.currentActivity }) { name, body -> sendEvent(name, body) }
            val dataDir = File(context.filesDir, "voltip").apply { mkdirs() }.absolutePath
            val version = context.packageManager.getPackageInfo(context.packageName, 0).versionName ?: "0.0.0"
            try {
                // A second start (the JavaScript reloaded) hands back the running shell with this
                // host in place of the old one.
                shell = VoltipShell.start(dataDir, version, host)
                watchNetwork(context)
                null
            } catch (e: ShellException) {
                e.message
            }
        }

        OnActivityEntersForeground { shell?.reconnectRelay() }

        OnDestroy { unwatchNetwork() }

        /** `#rrggbb` of the system's accent (Material You), or `null` before Android 12. */
        Function("systemAccent") {
            val context = appContext.reactContext ?: return@Function null
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) return@Function null
            // The middle of the system's primary palette (tone 50): the colour it seeds from.
            String.format("#%06X", context.getColor(android.R.color.system_accent1_500) and 0xFFFFFF)
        }

        /** The command's answer as JSON; a failure rejects with the shell's error text, as Tauri's invoke does. */
        AsyncFunction("invoke") Coroutine { command: String, args: String ->
            val running = shell ?: throw CodedException(ERROR_CODE, "the shell is not running", null)
            try {
                running.invoke(command, args)
            } catch (e: ShellException) {
                throw CodedException(ERROR_CODE, e.message, e)
            }
        }
    }

    /** The default network came or went (another Wi-Fi, mobile data, none): check the relay socket. */
    @Synchronized
    private fun watchNetwork(context: Context) {
        if (network != null) return
        val manager = context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        val callback = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) {
                shell?.reconnectRelay()
            }

            override fun onLost(network: Network) {
                shell?.reconnectRelay()
            }
        }
        try {
            manager.registerDefaultNetworkCallback(callback)
            network = callback
        } catch (e: RuntimeException) {
            // Too many callbacks registered, or no permission: the heartbeat still notices.
            android.util.Log.w("VoltipNative", "network callback not registered: ${e.message}")
        }
    }

    @Synchronized
    private fun unwatchNetwork() {
        val callback = network ?: return
        val context = appContext.reactContext?.applicationContext ?: return
        val manager = context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        try {
            manager.unregisterNetworkCallback(callback)
        } catch (_: IllegalArgumentException) {
            // Not registered any more.
        }
        network = null
    }

    private companion object {
        const val LOG_FILTER = "info,voltip_core=debug,voltip_rn=debug"
        const val ERROR_CODE = "ERR_VOLTIP"
    }
}
