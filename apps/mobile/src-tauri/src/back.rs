//! Android's back (docs/dictation.md §20.10): `BackPlugin.kt` hands every back to the page while
//! it listens, and lets the backs of a short window through to the system, which leaves the app.
//! The page talks to the plugin itself (`apps/mobile/src/app/back.ts`: its `register_listener`,
//! `remove_listener` and `release`, which `build.rs` declares and `capabilities/default.json`
//! allows); the Rust side only registers it. Other builds of this crate (the desktop-hosted tests)
//! have no system back.

use tauri::Runtime;

/// The plugin's name: the page's commands are `plugin:voltip-back|…`.
pub const PLUGIN: &str = "voltip-back";

/// Registers the Android back plugin; a no-op elsewhere.
pub fn init<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new(PLUGIN)
        .setup(|_app, _api| {
            // The Kotlin plugin lives in Tauri's plugin manager; nothing on this side calls it.
            #[cfg(target_os = "android")]
            _api.register_android_plugin("dev.voltip.mobile", "BackPlugin")?;
            Ok(())
        })
        .build()
}
