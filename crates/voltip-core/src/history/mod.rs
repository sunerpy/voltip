//! Dictation history (docs/dictation.md §4): an SQLite database in the app data dir,
//! `history.sqlite3`, newest first, capped at [`MAX_ENTRIES`]. The core owns the only writing
//! connection ([`HistoryStore`]); the bridge answers the interface's queries from a read-only one
//! ([`HistoryReader`]). A `history.json` from before is imported once and kept under another name.
//! A file that cannot be used is moved aside and logged — it never stops the app from starting.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::dictation::{ClipboardCode, InjectNote, OutputMode, Segment, TakeKind, Via};
use crate::presets::PresetRef;
use crate::scenes::{AppRef, SceneRef};
use crate::vocabulary::VocabularyHits;

mod derived;
pub mod export;
pub mod process;
mod reader;
mod revs;
mod store;

pub use derived::{corrected_chars, counts_for_stats};
pub use reader::{HistoryHits, HistoryPage, HistoryQuery, HistoryReader, HistoryStats, HistoryStatsBucket, MAX_QUERY_LIMIT, MAX_STATS_BOUNDARIES};
pub use revs::{ChangeBatch, Outbox, SHORTENED_FIELD_CHARS, SHORTENED_TEXT_CHARS, bounded};
pub use store::HistoryStore;
pub(crate) use store::{BUSY_TIMEOUT, SCHEMA as TABLES, insert as insert_entry};

/// The database inside the app data directory.
pub const HISTORY_DB_FILE_NAME: &str = "history.sqlite3";
/// The file the history lived in before the database; imported once, then renamed
/// `history.json.imported-<unix seconds>` and never deleted.
pub const HISTORY_FILE_NAME: &str = "history.json";
/// Oldest entries are dropped past this many (the most `HistorySettings.keep` may ask for).
pub const MAX_ENTRIES: usize = 20_000;
/// The fewest entries `HistorySettings.keep` may ask for.
pub const MIN_KEEP: usize = 10;
/// How many of the newest entries `UiState.history_recent` carries: the home page's recent table
/// fills a tall window with them (30 rows fit a maximised 2560 × 1440 window; 20 until 2026-10-01).
pub const RECENT_ENTRIES: usize = 30;
/// Schema version of the legacy `history.json`.
pub const HISTORY_SCHEMA: u16 = 1;

/// How a dictation ended.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    /// Text delivered.
    Inserted {
        /// Route.
        via: Via,
    },
    /// A paste was requested but the text stayed in the clipboard.
    Clipboard {
        /// Why, as the injector said it (shown only under the technical details).
        reason: String,
        /// The kind of reason the interface explains; absent in entries written before
        /// 2026-09-29, which the interface explains in general terms.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<ClipboardCode>,
    },
    /// The text could not be delivered at all.
    Failed {
        /// Why.
        reason: String,
    },
}

impl Outcome {
    /// The clipboard fallback `note` describes.
    pub fn clipboard(note: InjectNote) -> Self {
        Self::Clipboard { reason: note.detail, code: Some(note.code) }
    }
}

/// What a voice edit worked on (docs/dictation.md §19). The result is the entry's `text`, the
/// instruction as recognised its `raw_text`.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct EditRecord {
    /// The instruction as sent to the LLM (after the dictionary corrections).
    pub instruction: String,
    /// The selection it rewrote (at most `MAX_EDIT_SELECTION_CHARS` characters: a longer one is
    /// refused before the request).
    pub selection: String,
}

/// One finished dictation.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Stable id for delete / star.
    pub id: Uuid,
    /// Unix time in milliseconds.
    pub at_ms: u64,
    /// Transcript as ASR returned it.
    pub raw_text: String,
    /// Text delivered (refined when refinement ran).
    pub text: String,
    /// The refiner's output was used.
    pub refined: bool,
    /// ASR model.
    pub asr_model: String,
    /// Refine model when refinement ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refine_model: Option<String>,
    /// Recording length.
    pub duration_ms: u64,
    /// ASR round trip.
    pub asr_ms: u64,
    /// Refine round trip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refine_ms: Option<u64>,
    /// How it ended.
    pub outcome: Outcome,
    /// User flag.
    #[serde(default)]
    pub starred: bool,
    /// Where the text came from (docs/dictation.md §12); entries written before it read as
    /// `whole_take`.
    #[serde(default)]
    pub mode: OutputMode,
    /// The streaming recogniser's sentences when the text came from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segments: Option<Vec<Segment>>,
    /// Why the streaming path was abandoned or cut short, when a streaming mode was asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_error: Option<String>,
    /// Which dictionary entries and replacement rules fired (docs/dictation.md §16.3); absent when
    /// none did, and in entries written before the vocabulary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vocabulary: Option<VocabularyHits>,
    /// Dictation or voice edit (docs/dictation.md §19); always written, entries from before read as
    /// `dictation`.
    #[serde(default)]
    pub kind: TakeKind,
    /// The voice edit's instruction and selection; `None` for dictation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit: Option<EditRecord>,
    /// The application in front when the take started (docs/dictation.md §18.6); absent when the
    /// shell has no probe or it did not answer, and in entries written before scenes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<AppRef>,
    /// The scene the take ran with, by id and its name at the time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene: Option<SceneRef>,
    /// The preset the clean-up ran with (docs/dictation.md §21), by id and its name at the time;
    /// absent when the text was not cleaned up, and in entries written before presets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<PresetRef>,
    /// A phone's take or text rather than this device's own (docs/dictation.md §20.6); absent for
    /// the device's own takes and in entries written before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<EntryOrigin>,
    /// What 用 AI 预设处理 made of the text (docs/dictation.md §22); the text itself stays as it
    /// was. Absent until it ran. Boxed: most entries have none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub processed: Option<Box<ProcessedText>>,
}

/// 用 AI 预设处理's result (docs/dictation.md §22): the text, the preset it ran with, and when.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ProcessedText {
    /// The processed text.
    pub text: String,
    /// The preset, by its name then.
    pub preset: PresetRef,
    /// Unix time in milliseconds.
    pub at_ms: u64,
}

/// Which phone an entry came from, and how.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct EntryOrigin {
    /// The phone's name at the time.
    pub device: String,
    /// What the phone sent.
    pub kind: OriginKind,
}

/// What a phone sent (docs/dictation.md §20).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OriginKind {
    /// Its audio, recognised here (the phone as microphone, §20.1).
    Take,
    /// Text typed into the phone (§20.6).
    Typed,
    /// The phone's clipboard (§20.6).
    Clipboard,
    /// The phone recognised it on its own and uploaded a copy (§20.8).
    Standalone,
}

#[cfg(test)]
mod tests;
