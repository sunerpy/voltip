package dev.voltip.mobile

import android.os.Bundle
import androidx.activity.enableEdgeToEdge
import io.crates.keyring.Keyring

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    // Before super.onCreate, which starts Rust: the first thing Rust does is open the Android
    // Keystore, and without the application context that panics and the app closes on start
    // (0.0.18 and 0.0.19, regression 2026-10-01; the android-device CI job starts the app).
    Keyring.initializeNdkContext(applicationContext)
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // Each instance: Android recreates the activity (an overlay, the font size), and the back
    // handler of the one before goes with it (BackPlugin.kt).
    BackPlugin.attach(this)
  }
}
