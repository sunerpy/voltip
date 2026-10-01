package io.crates.keyring

import android.content.Context

/**
 * The entry `android-native-keyring-store` exports for its Android context
 * (`Java_io_crates_keyring_Keyring_00024Companion_initializeNdkContext`, hence this package and
 * class name). The store reads the context through `ndk-context`, which nothing in Tauri's
 * Android runtime sets (tao 0.35), so `MainActivity` calls this before Rust starts. Not generated
 * by `cargo tauri android init`: keep this file when regenerating the project.
 */
class Keyring {
    companion object {
        init {
            // Rust.kt loads the same library later; loading it twice is a no-op.
            System.loadLibrary("voltip_mobile_lib")
        }

        /** Hands the store `context`; calls after the first change nothing. */
        external fun initializeNdkContext(context: Context)
    }
}
