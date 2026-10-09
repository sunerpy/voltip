//! 导出字幕（SRT）/ 导出文本（TXT） (docs/dictation.md §22): the operating system's save dialog
//! offers the name the page chose, and the file is written here, never by the webview.

use serde::Serialize;
use tauri::Runtime;
use tauri_plugin_dialog::DialogExt as _;
use voltip_core::HistoryEntry;
use voltip_core::history::export::{self, ExportFormat};

/// Why an export wrote nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFailure {
    /// The entry was deleted meanwhile.
    Gone,
    /// Subtitles of an entry without segments.
    Empty,
    /// The file could not be written.
    Write,
}

/// How an export ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExportOutcome {
    /// Written to `path`.
    Saved {
        /// Where.
        path: String,
    },
    /// The dialog was closed.
    Cancelled,
    /// Nothing written.
    Failed {
        /// What the page says.
        code: ExportFailure,
        /// The system's message (the technical details).
        detail: String,
    },
}

impl ExportOutcome {
    fn failed(code: ExportFailure, detail: impl Into<String>) -> Self {
        Self::Failed { code, detail: detail.into() }
    }
}

/// What `entry` exports to in `format`, or why nothing.
pub fn content(entry: Option<&HistoryEntry>, format: ExportFormat) -> Result<String, ExportOutcome> {
    let entry = entry.ok_or_else(|| ExportOutcome::failed(ExportFailure::Gone, "the entry is not in the history"))?;
    export::render(entry, format).ok_or_else(|| ExportOutcome::failed(ExportFailure::Empty, "the entry has no segments"))
}

/// Ask where `content` goes (the dialog offers `name`), then write it there.
pub async fn save<R: Runtime>(app: &tauri::AppHandle<R>, content: String, format: ExportFormat, name: &str) -> ExportOutcome {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let label = format.extension().to_uppercase();
    app.dialog().file().set_file_name(export::file_name(name, format)).add_filter(label, &[format.extension()]).save_file(move |path| {
        let _ = tx.send(path);
    });
    let Ok(Some(path)) = rx.await else { return ExportOutcome::Cancelled };
    let path = match path.into_path() {
        Ok(path) => path,
        Err(e) => return ExportOutcome::failed(ExportFailure::Write, e.to_string()),
    };
    match tauri::async_runtime::spawn_blocking(move || write(&path, &content).map(|()| path)).await {
        Ok(Ok(path)) => {
            tracing::info!(format = format.extension(), "history entry exported");
            ExportOutcome::Saved { path: path.display().to_string() }
        }
        Ok(Err(e)) => ExportOutcome::failed(ExportFailure::Write, e.to_string()),
        Err(e) => ExportOutcome::failed(ExportFailure::Write, e.to_string()),
    }
}

fn write(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    std::fs::write(path, content)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn an_export_needs_the_entry_and_for_subtitles_its_segments() {
        assert!(matches!(content(None, ExportFormat::Txt), Err(ExportOutcome::Failed { code: ExportFailure::Gone, .. })));
        let text = voltip_core::dictation::fakes::history_entry("会议记录。");
        assert_eq!(content(Some(&text), ExportFormat::Txt).unwrap(), "会议记录。\n");
        assert!(matches!(content(Some(&text), ExportFormat::Srt), Err(ExportOutcome::Failed { code: ExportFailure::Empty, .. })));
        let json = serde_json::to_value(ExportOutcome::failed(ExportFailure::Write, "disk full")).unwrap();
        assert_eq!(json, serde_json::json!({ "kind": "failed", "code": "write", "detail": "disk full" }));
        assert_eq!(serde_json::to_value(ExportOutcome::Cancelled).unwrap(), serde_json::json!({ "kind": "cancelled" }));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        write(&path, "x").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "x");
    }
}
