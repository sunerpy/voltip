//! 导出备份 / 导入备份 (docs/dictation.md §4.6) and the automatic backups' folder (§4.6.1): the
//! operating system's save, open and folder dialogs; the bridge writes and reads the files, never
//! the webview, and the webview never names a path.

use std::path::Path;
use std::time::Duration;

use serde::Serialize;
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

/// Ask which backup to import (the dialog is titled `title` and opens in the automatic backups'
/// folder, §4.6.1), then have the bridge read and check it; nothing changes until `backup_import`.
pub async fn open<R: Runtime>(app: &tauri::AppHandle<R>, bridge: Bridge, title: &str) -> OpenOutcome {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut dialog = app.dialog().file().set_title(title).add_filter(FILTER, &[BACKUP_EXTENSION]);
    let automatic = bridge.backup_folder();
    if automatic.is_dir() {
        dialog = dialog.set_directory(automatic);
    }
    dialog.pick_file(move |path| {
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

/// `backup_folder_choose`'s answer (docs/dictation.md §4.6.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FolderOutcome {
    /// The automatic backups go to `folder` from now on.
    Chosen {
        /// Where.
        folder: String,
    },
    /// The dialog was closed.
    Cancelled,
    /// Voltip cannot use the folder chosen; nothing changed.
    Failed {
        /// The system's message (the technical details).
        detail: String,
    },
}

/// 更改… (docs/dictation.md §4.6.1): the folder dialog, titled `title` and opening where the
/// automatic backups go now, then a file written into the folder chosen and deleted again; a folder
/// Voltip cannot write to is refused and nothing changes. The backups already made stay where
/// they are.
pub async fn choose_folder<R: Runtime>(app: &tauri::AppHandle<R>, bridge: Bridge, title: &str) -> FolderOutcome {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut dialog = app.dialog().file().set_title(title);
    let current = bridge.backup_folder();
    if current.is_dir() {
        dialog = dialog.set_directory(current);
    }
    dialog.pick_folder(move |folder| {
        let _ = tx.send(folder);
    });
    let Ok(Some(folder)) = rx.await else { return FolderOutcome::Cancelled };
    let folder = match folder.into_path() {
        Ok(folder) => folder,
        Err(e) => return FolderOutcome::Failed { detail: e.to_string() },
    };
    match tauri::async_runtime::spawn_blocking(move || writable(&folder).map(|()| folder)).await {
        Ok(Ok(folder)) => match bridge.backup_set_folder(folder.clone()) {
            Ok(()) => {
                tracing::info!("automatic backups' folder changed");
                FolderOutcome::Chosen { folder: folder.display().to_string() }
            }
            Err(e) => FolderOutcome::Failed { detail: e.to_string() },
        },
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "automatic backups' folder refused");
            FolderOutcome::Failed { detail: e.to_string() }
        }
        Err(e) => FolderOutcome::Failed { detail: e.to_string() },
    }
}

/// `folder` can take the automatic backups: its path is one the settings can keep, and a file can
/// be written into it (and is deleted again).
pub fn writable(folder: &Path) -> std::io::Result<()> {
    if folder.to_str().is_none() {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "the folder's path has characters that cannot be saved"));
    }
    let probe = folder.join(format!(".voltip-write-test-{}", uuid::Uuid::new_v4()));
    std::fs::OpenOptions::new().write(true).create_new(true).open(&probe)?;
    std::fs::remove_file(&probe)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// docs/dictation.md §4.6.1: a folder that takes a file is accepted and left as it was; one
    /// that is not there or is a file is refused with the system's reason.
    #[test]
    fn a_folder_is_tried_with_a_file_before_it_is_used() {
        let dir = tempfile::tempdir().unwrap();
        writable(dir.path()).unwrap();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0, "the trial file is gone");
        assert!(writable(&dir.path().join("not-there")).is_err());
        let file = dir.path().join("a file");
        std::fs::write(&file, b"x").unwrap();
        assert!(writable(&file).is_err());
        let json = serde_json::to_value(FolderOutcome::Chosen { folder: "D:\\Voltip".into() }).unwrap();
        assert_eq!(json, serde_json::json!({ "kind": "chosen", "folder": "D:\\Voltip" }));
        assert_eq!(serde_json::to_value(FolderOutcome::Cancelled).unwrap(), serde_json::json!({ "kind": "cancelled" }));
        let json = serde_json::to_value(FolderOutcome::Failed { detail: "Access is denied. (os error 5)".into() }).unwrap();
        assert_eq!(json, serde_json::json!({ "kind": "failed", "detail": "Access is denied. (os error 5)" }));
    }
}
