//! The phone's clipboard for 「发送剪贴板」 (docs/dictation.md §20.6): read through
//! `PhoneClipboardPlugin.kt` on Android (the system `ClipboardManager`, which answers the app in
//! front). Other builds of this crate (the desktop-hosted tests) have no phone clipboard.

use tauri::{AppHandle, Runtime};

/// Why the clipboard cannot be read on this build.
pub const CLIPBOARD_UNAVAILABLE: &str = "clipboard: 这个平台没有手机剪贴板";

/// The Android plugin that reads the clipboard (`PhoneClipboardPlugin.kt`).
#[cfg(target_os = "android")]
pub struct PhoneClipboard<R: Runtime>(tauri::plugin::PluginHandle<R>);

/// Registers the Android clipboard plugin; a no-op elsewhere.
pub fn init<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("voltip-clipboard")
        .setup(|_app, _api| {
            #[cfg(target_os = "android")]
            {
                use tauri::Manager as _;
                let handle = _api.register_android_plugin("dev.voltip.mobile", "PhoneClipboardPlugin")?;
                _app.manage(PhoneClipboard(handle));
            }
            Ok(())
        })
        .build()
}

/// The text on the clipboard, `None` when it holds none (or holds no text).
pub async fn read_text<R: Runtime>(app: &AppHandle<R>) -> Result<Option<String>, String> {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager as _;
        let Some(plugin) = app.try_state::<PhoneClipboard<R>>() else { return Err("clipboard: plugin missing".into()) };
        let handle = plugin.0.clone();
        let answer: serde_json::Value = tauri::async_runtime::spawn_blocking(move || handle.run_mobile_plugin("readText", ()).map_err(|e| e.to_string()))
            .await
            .map_err(|e| e.to_string())??;
        return Ok(text_of(&answer));
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Err(CLIPBOARD_UNAVAILABLE.into())
    }
}

/// The plugin's answer `{ "text": "…" | null }`: an empty clipboard is no text.
pub fn text_of(answer: &serde_json::Value) -> Option<String> {
    answer.get("text").and_then(serde_json::Value::as_str).filter(|t| !t.is_empty()).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plugin_answer_reads_as_text_or_none() {
        assert_eq!(text_of(&serde_json::json!({ "text": "https://example.test" })).as_deref(), Some("https://example.test"));
        assert_eq!(text_of(&serde_json::json!({ "text": "" })), None);
        assert_eq!(text_of(&serde_json::json!({ "text": null })), None);
        assert_eq!(text_of(&serde_json::json!({})), None);
    }
}
