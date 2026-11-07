# The Rust shell's bindings: UniFFI's Kotlin (dev.voltip.rn.uniffi) calls the library through JNA,
# which reads those classes and its own reflectively; VoltipHost is the trait Rust calls back.
-keep class dev.voltip.rn.uniffi.** { *; }
-keep class dev.voltip.rn.VoltipHost { *; }
-dontwarn java.awt.**
-keep class com.sun.jna.** { *; }
-keepclassmembers class * extends com.sun.jna.** { public *; }
# android-native-keyring-store's two JNI entries (Keyring.kt) are bound by name.
-keep class io.crates.keyring.** { *; }
# Only rules about classes: the options that keep the stack traces' line numbers are global, and
# Android Gradle Plugin 9 fails this module's build on a global option here. They are in the app's
# own rules (app.json, expo-build-properties android.extraProguardRules; docs/mobile-rn.md §6).
