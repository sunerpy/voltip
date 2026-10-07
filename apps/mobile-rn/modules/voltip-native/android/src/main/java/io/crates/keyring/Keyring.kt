package io.crates.keyring

import android.content.Context
import androidx.annotation.Keep

/**
 * The entries `android-native-keyring-store` exports (`Java_io_crates_keyring_Keyring_00024Companion_initializeNdkContext`,
 * hence this package and class name). The store reads the Android context through `ndk-context`,
 * which nothing else in a React Native app sets, so `VoltipNativeModule` calls this before the
 * shell starts; cpal (AAudio) reads the same context.
 */
@Keep
class Keyring {
    companion object {
        init {
            System.loadLibrary("voltip_rn")
        }

        /** Hands the store [context]; calls after the first change nothing. */
        @Keep
        external fun initializeNdkContext(context: Context)
    }
}

/** The store's logcat subscriber (`android-log` feature): every `tracing` event of the shell, by target. */
@Keep
class KeyringLog {
    companion object {
        init {
            System.loadLibrary("voltip_rn")
        }

        /** Install the subscriber with an `EnvFilter` directive; a second call keeps the first. */
        @Keep
        external fun setLog(filter: String)
    }
}
