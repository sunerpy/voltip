# The Rust shell's bindings: UniFFI's Kotlin (dev.voltip.rn.uniffi) calls the library through JNA,
# which reads those classes and its own reflectively; VoltipHost is the trait Rust calls back.
-keep class dev.voltip.rn.uniffi.** { *; }
-keep class dev.voltip.rn.VoltipHost { *; }
-dontwarn java.awt.**
-keep class com.sun.jna.** { *; }
-keepclassmembers class * extends com.sun.jna.** { public *; }
# android-native-keyring-store's two JNI entries (Keyring.kt) are bound by name.
-keep class io.crates.keyring.** { *; }
