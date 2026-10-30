//! Backup and migration (docs/dictation.md §4.6): one file that carries the user's content from one
//! computer to another — every history entry, the personal dictionary, the replacement rules, the
//! scenes and the custom presets — and the import that restores it, merged with what is there or in
//! its place.
//!
//! The file is UTF-8 JSON, `<name>.voltip-backup`: the manifest (`format`, `version`, the app
//! version, when and on which device it was made), then each part as the stores keep it, serialized
//! by serde as it is, so a field a later version adds round-trips without a field list here. The
//! history comes last, streamed from the database row by row, and the counts per part close the
//! file. Settings, keys and every other secret, the device identity, the paired devices, the local
//! models and the phones' copies stay out.
//!
//! Nothing here changes a store on its own. [`read`] checks a file whole before anything changes;
//! [`merge`] works out each new list and what was skipped and why; the runtime hands the lists to
//! the stores, which check them again with their own rules and write each part in one go.

pub mod auto;
mod file;
pub(crate) mod merge;

use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use voltip_protocol::Platform;

use crate::history::HistoryEntry;
use crate::presets::CustomPreset;
use crate::scenes::Scene;
use crate::vocabulary::{DictionaryEntry, ImportMode, ReplacementRule};

pub use file::{ExportInput, ExportSummary, export, file_name, parse, read, save_path};

/// `format` of a backup file.
pub const BACKUP_FORMAT: &str = "voltip-backup";
/// The `version` this build writes and the newest it reads. A field added inside a part, or a new
/// part, keeps the version (a reader ignores what it does not know); only a change an older build
/// would read wrongly raises it.
pub const BACKUP_VERSION: u32 = 1;
/// The extension the save and open dialogs use.
pub const BACKUP_EXTENSION: &str = "voltip-backup";
/// The largest file an import reads: 512 MiB. Twenty thousand ordinary entries take a few tens of
/// MiB; only a history of long recordings comes near it.
pub const MAX_BACKUP_BYTES: u64 = 512 * 1024 * 1024;

/// One part of a backup.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupPart {
    /// The history entries (their statistics are derived from them).
    History,
    /// The personal dictionary.
    Dictionary,
    /// The replacement rules.
    Rules,
    /// The scenes, the built-in ones included.
    Scenes,
    /// The custom presets.
    Presets,
}

/// How many items each part holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BackupCounts {
    /// History entries.
    pub history: u32,
    /// Dictionary entries.
    pub dictionary: u32,
    /// Replacement rules.
    pub rules: u32,
    /// Scenes, the built-in ones included.
    pub scenes: u32,
    /// Custom presets.
    pub presets: u32,
}

/// Which parts an import restores.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BackupParts {
    /// The history entries.
    pub history: bool,
    /// The personal dictionary.
    pub dictionary: bool,
    /// The replacement rules.
    pub rules: bool,
    /// The scenes.
    pub scenes: bool,
    /// The custom presets.
    pub presets: bool,
}

impl BackupParts {
    /// Every part.
    pub const ALL: Self = Self { history: true, dictionary: true, rules: true, scenes: true, presets: true };

    /// `part` is chosen.
    pub const fn includes(self, part: BackupPart) -> bool {
        match part {
            BackupPart::History => self.history,
            BackupPart::Dictionary => self.dictionary,
            BackupPart::Rules => self.rules,
            BackupPart::Scenes => self.scenes,
            BackupPart::Presets => self.presets,
        }
    }
}

/// What a backup says about itself; the import's preview shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupManifest {
    /// [`BACKUP_FORMAT`].
    pub format: String,
    /// The format version it was written in.
    pub version: u32,
    /// The Voltip version that wrote it.
    pub app_version: String,
    /// When, in Unix milliseconds.
    pub created_at_ms: u64,
    /// The name of the device it was made on.
    pub device: String,
    /// The system of that device: a built-in scene's default applications follow the system they
    /// are restored on.
    pub platform: Platform,
    /// Items per part.
    pub counts: BackupCounts,
}

/// A backup read and checked whole, ready to import.
#[derive(Clone, PartialEq)]
pub struct Backup {
    /// What it says about itself.
    pub manifest: BackupManifest,
    /// The history entries, newest first.
    pub history: Vec<HistoryEntry>,
    /// The dictionary, in its order.
    pub dictionary: Vec<DictionaryEntry>,
    /// The rules, in execution order.
    pub rules: Vec<ReplacementRule>,
    /// The scenes, in matching order.
    pub scenes: Vec<Scene>,
    /// The custom presets.
    pub presets: Vec<CustomPreset>,
}

impl std::fmt::Debug for Backup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Counts only: the content is the user's.
        f.debug_struct("Backup").field("version", &self.manifest.version).field("counts", &self.manifest.counts).finish_non_exhaustive()
    }
}

/// Why a backup could not be written or read. The text is for the logs and the technical details;
/// the interface words each kind itself.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BackupError {
    /// The file could not be read: the system's message.
    #[error("backup: the file cannot be read: {0}")]
    Unreadable(String),
    /// The file is not a Voltip backup.
    #[error("backup: not a Voltip backup")]
    NotBackup,
    /// A Voltip backup whose content does not parse or does not add up.
    #[error("backup: the file is damaged: {0}")]
    Damaged(String),
    /// Written by a newer Voltip in this format version.
    #[error("backup: format version {0} is newer than this Voltip reads")]
    TooNew(u32),
    /// Larger than [`MAX_BACKUP_BYTES`].
    #[error("backup: the file is larger than {MAX_BACKUP_BYTES} bytes")]
    TooLarge,
    /// The file could not be written: the system's message.
    #[error("backup: the file cannot be written: {0}")]
    Write(String),
    /// The history database could not be read.
    #[error("backup: the history cannot be read: {0}")]
    History(String),
}

impl BackupError {
    /// The system's message, for the technical details (empty where there is none).
    pub fn detail(&self) -> String {
        match self {
            Self::Unreadable(why) | Self::Damaged(why) | Self::Write(why) | Self::History(why) => why.clone(),
            Self::NotBackup | Self::TooNew(_) | Self::TooLarge => String::new(),
        }
    }
}

/// Why `backup_export` wrote nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFailure {
    /// The file could not be written where the user chose.
    Write,
    /// The history could not be read.
    History,
    /// This shell cannot make a backup (the phone).
    Unsupported,
}

/// `backup_export`'s answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExportOutcome {
    /// Written to `path`.
    Saved {
        /// Where.
        path: String,
        /// Items per part.
        counts: BackupCounts,
        /// The file's size.
        bytes: u64,
    },
    /// The save dialog was closed.
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
    /// The answer for an export that failed with `error`.
    pub fn failed(error: &BackupError) -> Self {
        let code = if matches!(error, BackupError::History(_)) { ExportFailure::History } else { ExportFailure::Write };
        Self::Failed { code, detail: error.detail() }
    }

    /// The answer of a shell without backups.
    pub fn unsupported() -> Self {
        Self::Failed { code: ExportFailure::Unsupported, detail: String::new() }
    }
}

/// Why `backup_open` has nothing to show.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenFailure {
    /// The file could not be read.
    Unreadable,
    /// It is not a Voltip backup.
    NotBackup,
    /// It is a damaged one.
    Damaged,
    /// A newer Voltip wrote it.
    TooNew,
    /// It is larger than [`MAX_BACKUP_BYTES`].
    TooLarge,
    /// This shell cannot import a backup (the phone).
    Unsupported,
}

/// `backup_open`'s answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OpenOutcome {
    /// Read and checked; nothing has changed yet. `backup_import` with `token` restores it.
    Preview {
        /// Names the backup the shell holds.
        token: Uuid,
        /// What the backup says about itself.
        manifest: BackupManifest,
    },
    /// The open dialog was closed.
    Cancelled,
    /// The file cannot be imported.
    Failed {
        /// What the page says.
        code: OpenFailure,
        /// The system's or the parser's message (the technical details).
        detail: String,
        /// The newer format version, for `too_new`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<u32>,
    },
}

impl OpenOutcome {
    /// The answer for a file that failed with `error`.
    pub fn failed(error: &BackupError) -> Self {
        let (code, version) = match error {
            BackupError::Unreadable(_) | BackupError::Write(_) | BackupError::History(_) => (OpenFailure::Unreadable, None),
            BackupError::NotBackup => (OpenFailure::NotBackup, None),
            BackupError::Damaged(_) => (OpenFailure::Damaged, None),
            BackupError::TooNew(version) => (OpenFailure::TooNew, Some(*version)),
            BackupError::TooLarge => (OpenFailure::TooLarge, None),
        };
        Self::Failed { code, detail: error.detail(), version }
    }

    /// The answer of a shell without backups.
    pub fn unsupported() -> Self {
        Self::Failed { code: OpenFailure::Unsupported, detail: String::new(), version: None }
    }
}

/// Why `backup_import` restored nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportFailure {
    /// The token names no backup the shell holds (closed, imported already, or opened again).
    Expired,
    /// The app did not answer.
    Unavailable,
    /// This shell cannot import a backup (the phone).
    Unsupported,
    /// A replacing import while a take runs (docs/dictation.md §4.6.1): the take would be recorded
    /// into the history being replaced. Nothing changed.
    Busy,
    /// The automatic backup a replacing import makes first could not be written
    /// (docs/dictation.md §4.6.1): nothing was replaced.
    SnapshotFailed,
}

/// `backup_import`'s answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ImportOutcome {
    /// The chosen parts ran; the report says what each restored.
    Done {
        /// Part by part.
        report: ImportReport,
    },
    /// Nothing ran.
    Failed {
        /// Why.
        code: ImportFailure,
        /// The system's message (the technical details), for `snapshot_failed`.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        detail: String,
    },
}

impl ImportOutcome {
    /// Nothing ran, for `code`.
    pub fn failed(code: ImportFailure) -> Self {
        Self::Failed { code, detail: String::new() }
    }
}

/// Why an item of a backup was not restored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skip {
    /// It is here already: the same id, or the same name, term or built-in scene.
    Duplicate,
    /// The list is full.
    Limit,
    /// It contradicts an item here (a dictionary entry whose spellings belong to another entry).
    Conflict,
    /// It does not pass the store's checks.
    Invalid,
    /// A history entry older than the newest `Settings.history.keep`.
    Older,
}

/// How many items were skipped, by reason.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Skipped {
    /// [`Skip::Duplicate`].
    pub duplicate: u32,
    /// [`Skip::Limit`].
    pub limit: u32,
    /// [`Skip::Conflict`].
    pub conflict: u32,
    /// [`Skip::Invalid`].
    pub invalid: u32,
    /// [`Skip::Older`].
    pub older: u32,
}

/// Why a part restored nothing; it stayed exactly as it was.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartFailure {
    /// The history database cannot be opened.
    Unavailable,
    /// The part could not be written.
    Write,
}

/// What an import did with one part.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartReport {
    /// Which part.
    pub part: BackupPart,
    /// Its items in the backup.
    pub in_file: u32,
    /// Items written.
    pub restored: u32,
    /// Items not written, by reason.
    pub skipped: Skipped,
    /// History entries that were here and are gone: every one when replacing, the oldest past
    /// `Settings.history.keep` when merging. Always 0 for the lists.
    #[serde(default)]
    pub removed: u32,
    /// The part restored nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<PartFailure>,
}

impl PartReport {
    /// Nothing restored or skipped yet.
    pub fn new(part: BackupPart, in_file: usize) -> Self {
        Self { part, in_file: count(in_file), restored: 0, skipped: Skipped::default(), removed: 0, error: None }
    }

    /// One more item not restored, for `why`.
    pub fn skip(&mut self, why: Skip) {
        let n = match why {
            Skip::Duplicate => &mut self.skipped.duplicate,
            Skip::Limit => &mut self.skipped.limit,
            Skip::Conflict => &mut self.skipped.conflict,
            Skip::Invalid => &mut self.skipped.invalid,
            Skip::Older => &mut self.skipped.older,
        };
        *n = n.saturating_add(1);
    }

    /// The part could not be written: nothing was restored, skipped or removed.
    pub fn failed(self, why: PartFailure) -> Self {
        Self { restored: 0, skipped: Skipped::default(), removed: 0, error: Some(why), ..self }
    }
}

/// What an import did, part by part in the order it ran them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportReport {
    /// Merged with what was here, or in its place.
    pub mode: ImportMode,
    /// One per part chosen.
    pub parts: Vec<PartReport>,
    /// A replacing import: the automatic backup of what was here before (docs/dictation.md
    /// §4.6.1), the path it was written to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
}

/// Where the core answers a [`crate::CoreCommand::BackupImport`]: once, with the report or why
/// nothing ran.
#[derive(Clone)]
pub struct ImportReply(Arc<Mutex<Option<tokio::sync::oneshot::Sender<ImportOutcome>>>>);

impl ImportReply {
    /// The reply to hand the command, and the receiver the caller waits on.
    pub fn channel() -> (Self, tokio::sync::oneshot::Receiver<ImportOutcome>) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        (Self(Arc::new(Mutex::new(Some(tx)))), rx)
    }

    /// Answer; a second answer, or one nobody waits for any more, goes nowhere.
    pub fn send(&self, outcome: ImportOutcome) {
        if let Some(tx) = self.0.lock().take() {
            let _ = tx.send(outcome);
        }
    }
}

impl std::fmt::Debug for ImportReply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ImportReply")
    }
}

/// A count as the wire carries it.
pub(crate) fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests;
