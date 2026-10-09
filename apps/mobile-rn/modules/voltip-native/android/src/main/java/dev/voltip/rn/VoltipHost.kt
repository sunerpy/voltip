package dev.voltip.rn

import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import androidx.core.content.FileProvider
import dev.voltip.rn.uniffi.HostException
import dev.voltip.rn.uniffi.PlatformHost
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

/**
 * The platform side of the Rust shell: `PlatformHost` of `apps/mobile-rn/rust/src/ffi.rs`, whose
 * Kotlin bindings (`dev.voltip.rn.uniffi`) scripts/build-android-rn.sh generates with UniFFI. It
 * hands the app the core's events and the level meter's frames, and does what only Android's own
 * APIs reach (docs/mobile-rn.md §4). Rust calls from its own threads, never from the main thread,
 * so a method may wait for the main thread here; a failure goes back to Rust as
 * [HostException.Failed] with its reason.
 */
class VoltipHost(
    private val context: Context,
    private val activity: () -> Activity?,
    private val emit: (String, Map<String, Any?>) -> Unit,
) : PlatformHost {
    private var multicastLock: WifiManager.MulticastLock? = null

    /** One `UiEvent` (`voltip://event`). */
    override fun event(json: String) = platform { emit("onEvent", mapOf("json" to json)) }

    /** One frame of the stream [channel] (the level meter). */
    override fun channel(channel: ULong, json: String) =
        platform { emit("onChannel", mapOf("id" to channel.toDouble(), "json" to json)) }

    /** 「发送剪贴板」: Android 10+ lets only the app in front read it, which Voltip is while the user taps. */
    override fun clipboardRead(): String? = platform {
        onMain {
            val manager = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
            val clip = manager.primaryClip
            if (clip != null && clip.itemCount > 0) clip.getItemAt(0).coerceToText(context)?.toString() else null
        }
    }

    /** The result of a take the phone recognised itself, and the history's copy button. */
    override fun clipboardWrite(text: String) = platform {
        onMain {
            val manager = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
            manager.setPrimaryClip(ClipData.newPlainText("Voltip", text))
        }
    }

    /** 「分享」: the system share sheet with plain text. */
    override fun shareText(text: String) = platform {
        val send = Intent(Intent.ACTION_SEND).apply {
            type = "text/plain"
            putExtra(Intent.EXTRA_TEXT, text)
        }
        start(Intent.createChooser(send, null))
    }

    /**
     * A history export (subtitles or text) as a file: written into the cache, read by the share
     * target through the module's FileProvider (`res/xml/voltip_file_paths.xml`).
     */
    override fun shareFile(name: String, text: String, mime: String) = platform {
        // One export at a time: the previous one's target has read it by now.
        val dir = File(context.cacheDir, "exports").apply { mkdirs() }
        dir.listFiles()?.forEach { it.delete() }
        val file = File(dir, File(name).name)
        file.writeText(text, Charsets.UTF_8)
        val uri = FileProvider.getUriForFile(context, "${context.packageName}.voltip.fileprovider", file)
        val send = Intent(Intent.ACTION_SEND).apply {
            type = mime
            putExtra(Intent.EXTRA_STREAM, uri)
            putExtra(Intent.EXTRA_SUBJECT, file.name)
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }
        start(Intent.createChooser(send, null))
    }

    /**
     * LAN discovery (docs/pairing.md 「局域网发现」): without a multicast lock the Wi-Fi driver drops
     * the mDNS packets. Needs CHANGE_WIFI_MULTICAST_STATE (a normal permission, no prompt).
     */
    @Synchronized
    override fun multicast(held: Boolean) {
        platform {
            if (held) {
                val wifi = context.applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
                val lock = multicastLock ?: wifi.createMulticastLock("voltip-lan-discovery").also {
                    it.setReferenceCounted(false)
                    multicastLock = it
                }
                if (!lock.isHeld) lock.acquire()
            } else {
                multicastLock?.let { if (it.isHeld) it.release() }
            }
        }
    }

    /** A page the shell built (a key page, the repository, the guide) in the browser. */
    override fun openUrl(url: String) = platform { start(Intent(Intent.ACTION_VIEW, Uri.parse(url))) }

    /**
     * Who installed the app, as the system recorded it (docs/dictation.md §20.9): Google Play
     * (`com.android.vending`) updates what it installed; anything else (a browser or a file manager
     * with a release's APK, `adb`, nothing recorded) takes its updates from the releases.
     */
    override fun installer(): String? = platform {
        val manager = context.packageManager
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            manager.getInstallSourceInfo(context.packageName).installingPackageName
        } else {
            @Suppress("DEPRECATION")
            manager.getInstallerPackageName(context.packageName)
        }
    }

    /** Start [intent] from the activity in front, or as a new task when there is none. */
    private fun start(intent: Intent) {
        onMain {
            val front = activity()
            if (front != null) {
                front.startActivity(intent)
            } else {
                context.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            }
        }
    }

    /** [call], with any failure as the [HostException] Rust expects (its message as the reason). */
    private fun <T> platform(call: () -> T): T =
        try {
            call()
        } catch (e: HostException) {
            throw e
        } catch (e: Exception) {
            throw HostException.Failed(e.message ?: e.javaClass.simpleName)
        }

    /** Run [call] on the main thread and wait for it (at most [MAIN_TIMEOUT_S] seconds). */
    private fun <T> onMain(call: () -> T): T {
        if (Looper.myLooper() == Looper.getMainLooper()) return call()
        val done = CountDownLatch(1)
        val result = AtomicReference<Result<T>>()
        Handler(Looper.getMainLooper()).post {
            result.set(runCatching(call))
            done.countDown()
        }
        if (!done.await(MAIN_TIMEOUT_S, TimeUnit.SECONDS)) throw IllegalStateException("the main thread did not answer")
        return result.get().getOrThrow()
    }

    private companion object {
        const val MAIN_TIMEOUT_S = 10L
    }
}
