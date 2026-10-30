//! 导入音频或视频… (docs/dictation.md §31): the open dialog of the history page. It takes several
//! files at once; the core queues them (`import_add`) once the options are chosen.

use tauri::Runtime;
use tauri_plugin_dialog::DialogExt;

/// Ask which files to import (the dialog is titled `title`): the audio and video files the import
/// reads, under the filter named `media`, or any file under `all`. macOS shows one list of every
/// filter's types and cannot switch between them, so it has `media` alone. The paths chosen; none
/// when the dialog was closed.
pub async fn pick<R: Runtime>(app: &tauri::AppHandle<R>, title: &str, media: &str, all: &str) -> Vec<String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut dialog = app.dialog().file().set_title(title).add_filter(media, voltip_media::EXTENSIONS);
    if !cfg!(target_os = "macos") {
        dialog = dialog.add_filter(all, &["*"]);
    }
    dialog.pick_files(move |paths| {
        let _ = tx.send(paths);
    });
    let Ok(Some(paths)) = rx.await else { return Vec::new() };
    paths.into_iter().filter_map(|path| path.into_path().ok()).map(|path| path.to_string_lossy().into_owned()).collect()
}
