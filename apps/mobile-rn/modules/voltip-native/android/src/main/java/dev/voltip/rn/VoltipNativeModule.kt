package dev.voltip.rn

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
 */
class VoltipNativeModule : Module() {
    @Volatile
    private var shell: VoltipShell? = null

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
                null
            } catch (e: ShellException) {
                e.message
            }
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
            val running = shell ?: throw CodedException(ERROR_CODE, "the shell is not running", null)
            try {
                running.invoke(command, args)
            } catch (e: ShellException) {
                throw CodedException(ERROR_CODE, e.message, e)
            }
        }
    }

    private companion object {
        const val LOG_FILTER = "info,voltip_core=debug,voltip_rn=debug"
        const val ERROR_CODE = "ERR_VOLTIP"
    }
}
