//! The recent activity of the MCP service (docs/dictation.md §25): what each AI tool read and
//! wrote, as the settings list it — names and counts only, never a text of the history — and what
//! undoing a write puts back. Kept in `<data dir>/mcp/activity.json`, newest first, at most
//! [`MAX_ACTIVITY`] entries, on the shared list persistence (an unusable file is moved aside).
//!
//! A write keeps the entry or rule as it was before and as the write left it. Undo puts the
//! "before" back (or removes what the write added) only while the item is still exactly as the
//! write left it: a change the user made since is never thrown away by an undo.

use std::path::Path;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{ACTIVITY_FILE, HistoryRange, mcp_dir};
use crate::list_file::ListFile;
use crate::vocabulary::{DictionaryEntry, ReplacementRule};

/// Most activities kept.
pub const MAX_ACTIVITY: usize = 50;
/// A read of the same kind by the same client within this long replaces the earlier one instead
/// of adding a line (an agent may list the dictionary many times in one conversation).
pub const READ_MERGE_MS: u64 = 10 * 60 * 1000;
/// On-disk schema of `activity.json`.
pub const ACTIVITY_SCHEMA: u16 = 1;

/// What a client did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpAction {
    /// Read the dictionary (`count` entries).
    ListDictionary,
    /// Read the rules (`count` rules).
    ListRules,
    /// Tried a text.
    Preview,
    /// Backtested a draft (`subject` = its spelling or name, `count` = takes it would change).
    Backtest,
    /// Read the correction summary (`count` pairs).
    ReadSummary,
    /// Searched the full history (`count` entries, `range`).
    SearchHistory,
    /// Read one full history entry (`range`).
    ReadEntry,
    /// Added an entry (`subject` = its spelling).
    AddEntry,
    /// Added forms to an entry (`subject` = its spelling, `count` = forms added).
    MergeEntry,
    /// Changed an entry (`subject` = its spelling).
    UpdateEntry,
    /// Created a rule (`subject` = its name, `pending` when it waits for approval).
    AddRule,
    /// Changed a rule (`subject` = its name, `pending` when it still waits).
    UpdateRule,
}

impl McpAction {
    /// Whether it changed the dictionary or the rules.
    pub const fn writes(self) -> bool {
        matches!(self, Self::AddEntry | Self::MergeEntry | Self::UpdateEntry | Self::AddRule | Self::UpdateRule)
    }
}

/// One line of the recent activity (`McpStatus.activity`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpActivity {
    /// Its id (`mcp_undo` names it).
    pub id: Uuid,
    /// Unix milliseconds.
    pub at_ms: u64,
    /// The client's cleaned name.
    pub client: String,
    /// What it did.
    pub action: McpAction,
    /// The entry's spelling or the rule's name it concerned, when any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// How many items it concerned, when that says something.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    /// The full history's range it read under.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<HistoryRange>,
    /// The rule it wrote waits for the user's approval.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pending: bool,
    /// The settings offer 撤销 for it.
    #[serde(default)]
    pub undoable: bool,
    /// It was undone.
    #[serde(default)]
    pub undone: bool,
}

/// An activity before it is noted: what, about what, how many.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivityDraft {
    /// What the client did.
    pub action: McpAction,
    /// The entry's spelling or the rule's name.
    pub subject: Option<String>,
    /// How many items.
    pub count: Option<u32>,
    /// The full history's range.
    pub range: Option<HistoryRange>,
    /// The rule waits for approval.
    pub pending: bool,
}

impl ActivityDraft {
    /// `action` alone.
    pub fn new(action: McpAction) -> Self {
        Self { action, subject: None, count: None, range: None, pending: false }
    }

    /// About `subject`.
    #[must_use]
    pub fn subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = Some(subject.into());
        self
    }

    /// Concerning `count` items.
    #[must_use]
    pub fn count(mut self, count: usize) -> Self {
        self.count = Some(u32::try_from(count).unwrap_or(u32::MAX));
        self
    }

    /// Read under `range`.
    #[must_use]
    pub fn range(mut self, range: HistoryRange) -> Self {
        self.range = Some(range);
        self
    }

    /// The rule waits for approval.
    #[must_use]
    pub fn pending(mut self, pending: bool) -> Self {
        self.pending = pending;
        self
    }

    /// The activity of `client` at `at_ms`.
    pub fn into_activity(self, client: &str, at_ms: u64) -> McpActivity {
        McpActivity {
            id: Uuid::new_v4(),
            at_ms,
            client: client.to_owned(),
            action: self.action,
            subject: self.subject,
            count: self.count,
            range: self.range,
            pending: self.pending,
            undoable: false,
            undone: false,
        }
    }
}

/// What undoing a write puts back: the item before the write (`None`: the write added it) and as
/// the write left it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "list", rename_all = "snake_case")]
pub enum Undo {
    /// A dictionary entry.
    Entry {
        /// Before (`None`: added).
        #[serde(default)]
        before: Option<DictionaryEntry>,
        /// After.
        after: DictionaryEntry,
    },
    /// A rule.
    Rule {
        /// Before (`None`: added).
        #[serde(default)]
        before: Option<ReplacementRule>,
        /// After.
        after: ReplacementRule,
    },
}

/// An activity and, for a write, what undoing it puts back (kept in the file, never sent to the
/// webview).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Record {
    #[serde(flatten)]
    activity: McpActivity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    undo: Option<Undo>,
}

#[derive(Serialize, Deserialize)]
struct ActivityFile {
    schema: u16,
    activity: Vec<Record>,
}

fn records_of(file: ActivityFile) -> Option<Vec<Record>> {
    (file.schema == ACTIVITY_SCHEMA).then_some(file.activity)
}

fn check_records(records: &[Record]) -> Result<(), String> {
    if records.len() > MAX_ACTIVITY {
        return Err(format!("最多 {MAX_ACTIVITY} 条"));
    }
    Ok(())
}

/// Why an undo cannot be done.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum UndoError {
    /// No activity has that id.
    #[error("mcp: 没有这条活动记录")]
    Unknown,
    /// It changed nothing that could be undone (a read).
    #[error("mcp: 这条活动不能撤销")]
    NotUndoable,
    /// It was undone already.
    #[error("mcp: 这条活动已经撤销")]
    AlreadyUndone,
}

/// The recent activity.
#[derive(Debug)]
pub struct ActivityLog {
    dir: std::path::PathBuf,
    file: ListFile,
    records: Vec<Record>,
}

impl ActivityLog {
    /// Open `<data dir>/mcp/activity.json`; the second value is the notice for the UI when the file
    /// could not be used.
    pub fn open(data_dir: &Path) -> (Self, Option<String>) {
        let dir = mcp_dir(data_dir);
        let (file, records, notice) = ListFile::load(&dir, ACTIVITY_FILE, ACTIVITY_SCHEMA, records_of, check_records);
        (Self { dir, file, records }, notice)
    }

    /// The activity, newest first.
    pub fn activities(&self) -> Vec<McpActivity> {
        self.records.iter().map(|r| r.activity.clone()).collect()
    }

    /// What undoing `id` would put back.
    pub fn undo_of(&self, id: Uuid) -> Result<&Undo, UndoError> {
        let record = self.records.iter().find(|r| r.activity.id == id).ok_or(UndoError::Unknown)?;
        if record.activity.undone {
            return Err(UndoError::AlreadyUndone);
        }
        match (&record.undo, record.activity.undoable) {
            (Some(undo), true) => Ok(undo),
            _ => Err(UndoError::NotUndoable),
        }
    }

    /// Note `activity` (with `undo` for a write). A read replaces the same client's read of the
    /// same kind and subject from the last [`READ_MERGE_MS`]; the oldest lines go past
    /// [`MAX_ACTIVITY`]. Returns the activity as kept.
    pub fn record(&mut self, mut activity: McpActivity, undo: Option<Undo>) -> McpActivity {
        activity.undoable = undo.is_some();
        if !activity.action.writes()
            && let Some(i) = self.records.iter().position(|r| {
                let a = &r.activity;
                !a.action.writes()
                    && a.action == activity.action
                    && a.client == activity.client
                    && a.subject == activity.subject
                    && activity.at_ms.saturating_sub(a.at_ms) < READ_MERGE_MS
            })
        {
            self.records.remove(i);
        }
        self.records.insert(0, Record { activity: activity.clone(), undo });
        self.records.truncate(MAX_ACTIVITY);
        self.save();
        activity
    }

    /// Mark `id` undone (its undo was applied).
    pub fn mark_undone(&mut self, id: Uuid) {
        if let Some(record) = self.records.iter_mut().find(|r| r.activity.id == id) {
            record.activity.undone = true;
            record.activity.undoable = false;
            record.undo = None;
            self.save();
        }
    }

    /// Keep the file in step; a failure to write is logged, never fatal (the activity is a record
    /// for the user, not something the service depends on).
    fn save(&self) {
        if let Err(e) = crate::serve::create_private_dir(&self.dir) {
            tracing::warn!(error = %e, "the MCP activity directory could not be created");
            return;
        }
        if let Err(e) = self.file.save(&ActivityFile { schema: ACTIVITY_SCHEMA, activity: self.records.clone() }) {
            tracing::warn!(error = %e, "the MCP activity could not be saved");
        }
    }
}
