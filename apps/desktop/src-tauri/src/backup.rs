//! 导出备份 / 导入备份 (docs/dictation.md §4.6): the operating system's save and open dialogs; the
//! bridge writes and reads the file, never the webview, and the webview never names a path.

use std::time::Duration;

use tauri::Runtime;
use tauri_plugin_dialog::DialogExt as _;
use voltip_core::backup::{self, BACKUP_EXTENSION, BackupError, ExportOutcome, OpenOutcome};
use voltip_tauri_bridge::Bridge;

/// How long `backup_import` waits for the core's report: an import of 20 000 entries takes a few
/// seconds (docs/dictation.md §4.6); this only ends a wait for a core that stopped answering.
pub const IMPORT_WAIT: Duration = Duration::from_secs(600);

/// The file type both dialogs show.
const FILTER: &str = "Voltip";

/// Ask where the backup goes (the dialog offers `name` with the backup's extension), then have the
/// bridge write it there.
pub async fn export<R: Runtime>(app: &tauri::AppHandle<R>, bridge: Bridge, name: &str) -> ExportOutcome {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().set_file_name(backup::file_name(name)).add_filter(FILTER, &[BACKUP_EXTENSION]).save_file(move |path| {
        let _ = tx.send(path);
    });
    let Ok(Some(path)) = rx.await else { return ExportOutcome::Cancelled };
    let path = match path.into_path() {
        Ok(path) => backup::save_path(&path),
        Err(e) => return ExportOutcome::failed(&BackupError::Write(e.to_string())),
    };
    match tauri::async_runtime::spawn_blocking(move || bridge.backup_export(&path).map(|summary| (path, summary))).await {
        Ok(Ok((path, summary))) => {
            tracing::info!(entries = summary.counts.history, bytes = summary.bytes, "backup exported");
            ExportOutcome::Saved { path: path.display().to_string(), counts: summary.counts, bytes: summary.bytes }
        }
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "backup not exported");
            ExportOutcome::failed(&e)
        }
        Err(e) => ExportOutcome::failed(&BackupError::Write(e.to_string())),
    }
}

/// Ask which backup to import (the dialog is titled `title`), then have the bridge read and check
/// it; nothing changes until `backup_import`.
pub async fn open<R: Runtime>(app: &tauri::AppHandle<R>, bridge: Bridge, title: &str) -> OpenOutcome {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().set_title(title).add_filter(FILTER, &[BACKUP_EXTENSION]).pick_file(move |path| {
        let _ = tx.send(path);
    });
    let Ok(Some(path)) = rx.await else { return OpenOutcome::Cancelled };
    let path = match path.into_path() {
        Ok(path) => path,
        Err(e) => return OpenOutcome::failed(&BackupError::Unreadable(e.to_string())),
    };
    tauri::async_runtime::spawn_blocking(move || bridge.backup_open(&path))
        .await
        .unwrap_or_else(|e| OpenOutcome::failed(&BackupError::Unreadable(e.to_string())))
}
