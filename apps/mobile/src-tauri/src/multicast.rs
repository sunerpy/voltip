//! LAN discovery on Android (docs/pairing.md 「局域网发现」): the Wi-Fi driver drops multicast
//! unless an app holds a `WifiManager.MulticastLock`, so `MulticastPlugin.kt` holds one while
//! `Settings.lan_discovery` is on (it costs battery). Other builds of this crate have nothing to
//! take.

use tauri::{AppHandle, Runtime};

/// The Android plugin that holds the lock (`MulticastPlugin.kt`).
#[cfg(target_os = "android")]
pub struct Multicast<R: Runtime>(tauri::plugin::PluginHandle<R>);

/// Registers the Android multicast plugin; a no-op elsewhere.
pub fn init<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("voltip-multicast")
        .setup(|_app, _api| {
            #[cfg(target_os = "android")]
            {
                use tauri::Manager as _;
                let handle = _api.register_android_plugin("dev.voltip.mobile", "MulticastPlugin")?;
                _app.manage(Multicast(handle));
            }
            Ok(())
        })
        .build()
}

/// Take (`held`) or release the lock, off the calling thread (a plugin call from the setup
/// thread would wait on the main thread that runs the setup); mDNS keeps asking, so answers
/// arrive once it is held.
pub fn hold_in_background<R: Runtime>(app: &AppHandle<R>, held: bool) {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager as _;
        let Some(plugin) = app.try_state::<Multicast<R>>() else {
            tracing::warn!("multicast: plugin missing; LAN discovery may hear nothing");
            return;
        };
        let handle = plugin.0.clone();
        let command = if held { "acquire" } else { "release" };
        tauri::async_runtime::spawn_blocking(move || {
            if let Err(e) = handle.run_mobile_plugin::<serde_json::Value>(command, ()) {
                tracing::warn!(error = %e, command, "multicast: the lock call failed; LAN discovery may hear nothing");
            }
        });
    }
    #[cfg(not(target_os = "android"))]
    let _ = (app, held);
}
