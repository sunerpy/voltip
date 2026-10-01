//! 「分享」 on the phone (docs/dictation.md §20.7): a result goes to another app through the system
//! share sheet, `SharePlugin.kt` on Android (`ACTION_SEND` with plain text). Other builds of this
//! crate (the desktop-hosted tests) have no share sheet.

use tauri::{AppHandle, Runtime};

/// Why nothing can be shared on this build.
pub const SHARE_UNAVAILABLE: &str = "share: 这个平台没有系统分享";

/// The Android plugin that opens the share sheet (`SharePlugin.kt`).
#[cfg(target_os = "android")]
pub struct Share<R: Runtime>(tauri::plugin::PluginHandle<R>);

/// Registers the Android share plugin; a no-op elsewhere.
pub fn init<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("voltip-share")
        .setup(|_app, _api| {
            #[cfg(target_os = "android")]
            {
                use tauri::Manager as _;
                let handle = _api.register_android_plugin("dev.voltip.mobile", "SharePlugin")?;
                _app.manage(Share(handle));
            }
            Ok(())
        })
        .build()
}

/// Open the system share sheet with `text`: something to share, no longer than a paste may be
/// (`voltip_core::paste::MAX_PASTE_TEXT_CHARS`, well within what one Android intent carries).
pub async fn share_text<R: Runtime>(app: &AppHandle<R>, text: String) -> Result<(), String> {
    if !voltip_core::paste::valid_paste_text(&text) {
        return Err(format!("share: 文字为空或超过 {} 字", voltip_core::paste::MAX_PASTE_TEXT_CHARS));
    }
    #[cfg(target_os = "android")]
    {
        use tauri::Manager as _;
        let Some(plugin) = app.try_state::<Share<R>>() else { return Err("share: plugin missing".into()) };
        let handle = plugin.0.clone();
        tauri::async_runtime::spawn_blocking(move || {
            handle.run_mobile_plugin::<serde_json::Value>("shareText", serde_json::json!({ "text": text })).map(|_| ()).map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())?
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, text);
        Err(SHARE_UNAVAILABLE.into())
    }
}
