package dev.voltip.rn

import android.content.Context
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.ProxyInfo
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
 * The same callback hands the shell the network's HTTP proxy, which following the system means on
 * the phone ([VoltipShell.setNetworkProxy], docs/dictation.md §24): Rust's clients cannot read it.
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

    /**
     * The default network came or went (another Wi-Fi, mobile data, none): check the relay socket.
     * Its link properties (reported right after it comes, and whenever they change) carry its
     * proxy, which goes to the shell.
     */
    @Synchronized
    private fun watchNetwork(context: Context) {
        if (network != null) return
        val manager = context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        val callback = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) {
                shell?.reconnectRelay()
            }

            override fun onLinkPropertiesChanged(network: Network, properties: LinkProperties) {
                // A device-wide proxy (set by an administrator) wins over the network's own.
                handProxy(manager.defaultProxy ?: properties.httpProxy)
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

    /**
     * The network's HTTP proxy to the shell: its host, port and excluded hosts; none as an empty
     * host. A proxy given only as a PAC file cannot be followed and counts as none.
     */
    private fun handProxy(proxy: ProxyInfo?) {
        val host = proxy?.host
        if (proxy == null || host.isNullOrBlank() || proxy.port !in 1..65535) {
            if (proxy?.pacFileUrl?.let { it != android.net.Uri.EMPTY } == true) {
                android.util.Log.w("VoltipNative", "the network's proxy is a PAC file; connections go direct")
            }
            shell?.setNetworkProxy("", 0u, "")
            return
        }
        shell?.setNetworkProxy(host, proxy.port.toUShort(), proxy.exclusionList.joinToString(","))
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
