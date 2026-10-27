package dev.voltip.rn

import android.app.Activity
import android.content.Context
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.ProxyInfo
import android.util.Log
import dev.voltip.rn.uniffi.ShellException
import dev.voltip.rn.uniffi.VoltipShell
import io.crates.keyring.Keyring
import io.crates.keyring.KeyringLog
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import java.io.File
import java.util.concurrent.CopyOnWriteArrayList

/**
 * The process's one Rust shell and its one host (docs/mobile-rn.md §8). The app's JavaScript
 * (through [VoltipNativeModule]) and the floating bubble ([BubbleService]) both reach the shell
 * here, so it starts once, whichever of them comes first, and keeps running while either needs
 * it; the host hands every event and stream frame to each [Listener] (the module while the
 * JavaScript runs, the bubble while it shows). A reloaded JavaScript registers again; the shell
 * and its host stay.
 *
 * The phone's only link to its computers is the relay (docs/pairing.md 「只走中继」), and its socket
 * rarely survives a change of network: when Android reports another default network the shell
 * checks it at once ([VoltipShell.reconnectRelay]), and gets the network's HTTP proxy, which
 * following the system means on the phone ([VoltipShell.setNetworkProxy], docs/dictation.md §24).
 * The callback lives as long as the process, as the shell does.
 */
object VoltipRuntime {
    /** What the shell hands the app: events (`voltip://event`) and stream frames (the meter). */
    interface Listener {
        fun onEvent(json: String) {}

        fun onChannel(channel: ULong, json: String) {}
    }

    private val listeners = CopyOnWriteArrayList<Listener>()

    @Volatile
    private var running: VoltipShell? = null

    /** The activity in front, as the JavaScript's module knows it; `null` without one. */
    @Volatile
    var activity: (() -> Activity?)? = null

    private var network: ConnectivityManager.NetworkCallback? = null

    /** The running shell, or `null` before the first [start]. */
    val shell: VoltipShell?
        get() = running

    /**
     * Start the shell once per process and hand it back; later calls return the running one.
     * Throws the shell's [dev.voltip.rn.uniffi.ShellException] when it cannot start.
     */
    @Synchronized
    fun start(context: Context): VoltipShell {
        running?.let { return it }
        val app = context.applicationContext
        // Rust's tracing into logcat, then the Android context `ndk-context` hands the Keystore
        // store and cpal (AAudio); both before the shell starts. Keyring.kt loads libvoltip_rn for
        // these two JNI entries; UniFFI's bindings reach it through JNA.
        KeyringLog.setLog(LOG_FILTER)
        Keyring.initializeNdkContext(app)
        val host = VoltipHost(app, { activity?.invoke() }, ::dispatchEvent, ::dispatchChannel)
        val dataDir = File(app.filesDir, "voltip").apply { mkdirs() }.absolutePath
        val version = app.packageManager.getPackageInfo(app.packageName, 0).versionName ?: "0.0.0"
        val shell = VoltipShell.start(dataDir, version, host)
        running = shell
        watchNetwork(app)
        return shell
    }

    /** Commands nobody waits for (the bubble's stop, cancel and meter): they live as long as the
     *  process, as the shell does, so one sent as the bubble closes still arrives. */
    private val background = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    /** Run `command` with `args` on the shell without waiting for its answer. */
    fun fire(command: String, args: String = "") {
        background.launch {
            try {
                running?.invoke(command, args)
            } catch (e: ShellException) {
                Log.w(TAG, "$command: ${e.message}")
            }
        }
    }

    fun addListener(listener: Listener) {
        listeners.addIfAbsent(listener)
    }

    fun removeListener(listener: Listener) {
        listeners.remove(listener)
    }

    private fun dispatchEvent(json: String) {
        for (listener in listeners) {
            try {
                listener.onEvent(json)
            } catch (e: Exception) {
                Log.w(TAG, "an event listener failed: ${e.message}")
            }
        }
    }

    private fun dispatchChannel(channel: ULong, json: String) {
        for (listener in listeners) {
            try {
                listener.onChannel(channel, json)
            } catch (e: Exception) {
                Log.w(TAG, "a stream listener failed: ${e.message}")
            }
        }
    }

    /**
     * The default network came or went (another Wi-Fi, mobile data, none): check the relay socket.
     * Its link properties (reported right after it comes, and whenever they change) carry its
     * proxy, which goes to the shell.
     */
    private fun watchNetwork(context: Context) {
        if (network != null) return
        val manager = context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        val callback = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) {
                running?.reconnectRelay()
            }

            override fun onLinkPropertiesChanged(network: Network, properties: LinkProperties) {
                // A device-wide proxy (set by an administrator) wins over the network's own.
                handProxy(manager.defaultProxy ?: properties.httpProxy)
            }

            override fun onLost(network: Network) {
                running?.reconnectRelay()
            }
        }
        try {
            manager.registerDefaultNetworkCallback(callback)
            network = callback
        } catch (e: RuntimeException) {
            // Too many callbacks registered, or no permission: the heartbeat still notices.
            Log.w(TAG, "network callback not registered: ${e.message}")
        }
    }

    /**
     * The network's HTTP proxy to the shell: its host, port and excluded hosts; none as an empty
     * host. A proxy given only as a PAC file cannot be followed and counts as none.
     */
    private fun handProxy(proxy: ProxyInfo?) {
        val shell = running ?: return
        val host = proxy?.host
        if (proxy == null || host.isNullOrBlank() || proxy.port !in 1..65535) {
            if (proxy?.pacFileUrl?.let { it != android.net.Uri.EMPTY } == true) {
                Log.w(TAG, "the network's proxy is a PAC file; connections go direct")
            }
            shell.setNetworkProxy("", 0u, "")
            return
        }
        shell.setNetworkProxy(host, proxy.port.toUShort(), proxy.exclusionList.joinToString(","))
    }

    private const val LOG_FILTER = "info,voltip_core=debug,voltip_rn=debug"
    private const val TAG = "VoltipRuntime"
}
