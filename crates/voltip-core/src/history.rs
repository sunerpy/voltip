//! Dictation history (docs/dictation.md §4): `history.json` in the app data dir, newest first,
//! capped at [`MAX_ENTRIES`], written atomically. A corrupt file is moved aside and logged — it
//! never stops the app from starting.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::CoreError;
use crate::dictation::{ClipboardCode, InjectNote, OutputMode, Segment, TakeKind, Via};
use crate::presets::PresetRef;
use crate::scenes::{AppRef, SceneRef};
use crate::vocabulary::VocabularyHits;

/// File name inside the app data directory.
pub const HISTORY_FILE_NAME: &str = "history.json";
/// Oldest entries are dropped past this many (the most `HistorySettings.keep` may ask for).
pub const MAX_ENTRIES: usize = 500;
/// The fewest entries `HistorySettings.keep` may ask for.
pub const MIN_KEEP: usize = 10;
/// On-disk schema version.
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
}

#[derive(Serialize, Deserialize)]
struct HistoryFile {
    schema: u16,
    entries: Vec<HistoryEntry>,
}

/// Loads / saves the history list.
#[derive(Debug)]
pub struct HistoryStore {
    path: PathBuf,
    entries: Vec<HistoryEntry>,
}

impl HistoryStore {
    /// Open `dir/history.json`. Missing → empty. Corrupt → renamed to `history.json.corrupt`,
    /// logged, and treated as empty.
    pub fn open(dir: &Path) -> Self {
        let path = dir.join(HISTORY_FILE_NAME);
        let entries = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<HistoryFile>(&bytes) {
                Ok(file) if file.schema == HISTORY_SCHEMA => file.entries,
                Ok(file) => {
                    Self::set_aside(&path, &format!("unsupported history schema {}", file.schema));
                    Vec::new()
                }
                Err(e) => {
                    Self::set_aside(&path, &e.to_string());
                    Vec::new()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                tracing::warn!(error = %e, path = %path.display(), "history unreadable; starting empty");
                Vec::new()
            }
        };
        let mut store = Self { path, entries };
        store.entries.truncate(MAX_ENTRIES);
        store
    }

    fn set_aside(path: &Path, why: &str) {
        let aside = path.with_extension("json.corrupt");
        match std::fs::rename(path, &aside) {
            Ok(()) => tracing::warn!(%why, moved_to = %aside.display(), "history file is corrupt; moved aside"),
            Err(e) => tracing::warn!(%why, error = %e, "history file is corrupt and could not be moved aside"),
        }
    }

    /// Newest first.
    pub fn entries(&self) -> &[HistoryEntry] {
        &self.entries
    }

    /// Prepend; drops the oldest past `keep` (at most [`MAX_ENTRIES`]).
    pub fn push(&mut self, entry: HistoryEntry, keep: usize) -> Result<(), CoreError> {
        self.entries.insert(0, entry);
        self.entries.truncate(keep.min(MAX_ENTRIES));
        self.save()
    }

    /// Drop the oldest entries past `keep`; `Ok(true)` when anything was dropped (and written).
    pub fn retain_newest(&mut self, keep: usize) -> Result<bool, CoreError> {
        let keep = keep.min(MAX_ENTRIES);
        if self.entries.len() <= keep {
            return Ok(false);
        }
        self.entries.truncate(keep);
        self.save().map(|()| true)
    }

    /// Remove one entry; `Ok(false)` when it was not there (nothing written).
    pub fn delete(&mut self, id: Uuid) -> Result<bool, CoreError> {
        let before = self.entries.len();
        self.entries.retain(|e| e.id != id);
        if self.entries.len() == before {
            return Ok(false);
        }
        self.save().map(|()| true)
    }

    /// Remove everything.
    pub fn clear(&mut self) -> Result<(), CoreError> {
        self.entries.clear();
        self.save()
    }

    /// Flag / unflag; `Ok(false)` when the id is unknown.
    pub fn star(&mut self, id: Uuid, starred: bool) -> Result<bool, CoreError> {
        let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) else { return Ok(false) };
        if entry.starred == starred {
            return Ok(true);
        }
        entry.starred = starred;
        self.save().map(|()| true)
    }

    fn save(&self) -> Result<(), CoreError> {
        let err = |e: &dyn std::fmt::Display| CoreError::History(e.to_string());
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| err(&e))?;
        }
        let file = HistoryFile { schema: HISTORY_SCHEMA, entries: self.entries.clone() };
        let bytes = serde_json::to_vec_pretty(&file).map_err(|e| err(&e))?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, bytes).map_err(|e| err(&e))?;
        std::fs::rename(&tmp, &self.path).map_err(|e| err(&e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(text: &str) -> HistoryEntry {
        HistoryEntry {
            id: Uuid::new_v4(),
            at_ms: 1_758_700_000_000,
            raw_text: text.to_owned(),
            text: format!("{text}。"),
            refined: true,
            asr_model: "Qwen/Qwen3-ASR-1.7B".into(),
            refine_model: Some("qwen/qwen3.8-27b".into()),
            duration_ms: 1500,
            asr_ms: 420,
            refine_ms: Some(300),
            outcome: Outcome::Inserted { via: Via::Paste },
            starred: false,
            mode: OutputMode::WholeTake,
            segments: None,
            live_error: None,
            vocabulary: None,
            kind: TakeKind::Dictation,
            edit: None,
            app: None,
            scene: None,
            preset: None,
            origin: None,
        }
    }

    #[test]
    fn retention_keeps_the_newest_and_trims_on_demand() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = HistoryStore::open(dir.path());
        for i in 0..30 {
            store.push(entry(&i.to_string()), 20).unwrap();
        }
        assert_eq!(store.entries().len(), 20, "push trims to the retention");
        assert_eq!(store.entries()[0].raw_text, "29", "newest first");
        assert!(!store.retain_newest(50).unwrap(), "nothing to drop");
        assert!(store.retain_newest(MIN_KEEP).unwrap());
        assert_eq!(store.entries().len(), MIN_KEEP);
        assert_eq!(HistoryStore::open(dir.path()).entries().len(), MIN_KEEP, "the trim was written");
        assert!(!store.retain_newest(usize::MAX).unwrap(), "never above the cap");
    }

    #[test]
    fn roundtrip_delete_star_clear_and_cap() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = HistoryStore::open(dir.path());
        assert!(store.entries().is_empty());
        let first = entry("第一句");
        let second = entry("第二句");
        store.push(first.clone(), MAX_ENTRIES).unwrap();
        store.push(second.clone(), MAX_ENTRIES).unwrap();
        assert_eq!(store.entries()[0].id, second.id, "newest first");
        let reopened = HistoryStore::open(dir.path());
        assert_eq!(reopened.entries(), store.entries());
        assert!(store.star(first.id, true).unwrap());
        assert!(store.star(first.id, true).unwrap(), "idempotent");
        assert!(!store.star(Uuid::new_v4(), true).unwrap());
        assert!(HistoryStore::open(dir.path()).entries()[1].starred);
        assert!(store.delete(second.id).unwrap());
        assert!(!store.delete(second.id).unwrap());
        assert_eq!(HistoryStore::open(dir.path()).entries().len(), 1);
        for i in 0..(MAX_ENTRIES + 5) {
            store.push(entry(&i.to_string()), MAX_ENTRIES).unwrap();
        }
        assert_eq!(store.entries().len(), MAX_ENTRIES);
        assert_eq!(store.entries()[0].raw_text, (MAX_ENTRIES + 4).to_string());
        store.clear().unwrap();
        assert!(HistoryStore::open(dir.path()).entries().is_empty());
        assert!(format!("{store:?}").contains("history.json"));
        assert!(!dir.path().join("history.json.tmp").exists(), "temp file is renamed away");
    }

    #[test]
    fn corrupt_and_foreign_files_are_moved_aside_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(HISTORY_FILE_NAME);
        std::fs::write(&path, b"{not json").unwrap();
        let store = HistoryStore::open(dir.path());
        assert!(store.entries().is_empty());
        assert!(!path.exists());
        assert!(dir.path().join("history.json.corrupt").exists());
        std::fs::write(&path, br#"{"schema":9,"entries":[]}"#).unwrap();
        assert!(HistoryStore::open(dir.path()).entries().is_empty());
        assert_eq!(std::fs::read_to_string(dir.path().join("history.json.corrupt")).unwrap(), r#"{"schema":9,"entries":[]}"#);
        // A directory in place of the file is unreadable (not "not found") and also non-fatal.
        std::fs::create_dir_all(&path).unwrap();
        assert!(HistoryStore::open(dir.path()).entries().is_empty());
    }

    #[test]
    fn save_failure_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        // The data dir is a file: nothing can be written under it.
        let blocked = dir.path().join("blocked");
        std::fs::write(&blocked, b"x").unwrap();
        let mut store = HistoryStore::open(&blocked);
        let err = store.push(entry("x"), MAX_ENTRIES).unwrap_err();
        assert!(matches!(err, CoreError::History(_)), "{err}");
        assert!(err.to_string().starts_with("history:"));
    }

    #[test]
    fn outcome_and_entry_serialize_tagged_and_omit_absent_options() {
        let mut e = entry("你好");
        e.refine_model = None;
        e.refine_ms = None;
        e.outcome = Outcome::Clipboard { reason: "paste failed".into(), code: None };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains(r#""outcome":{"kind":"clipboard","reason":"paste failed"}"#), "{json}");
        assert!(!json.contains("refine_model") && !json.contains("refine_ms"), "{json}");
        let back: HistoryEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);
        assert_eq!(serde_json::to_string(&Outcome::Inserted { via: Via::Paste }).unwrap(), r#"{"kind":"inserted","via":"paste"}"#);
        assert_eq!(serde_json::to_string(&Outcome::Failed { reason: "x".into() }).unwrap(), r#"{"kind":"failed","reason":"x"}"#);
        // `starred` defaults when an older file lacks it.
        let legacy = json.replace(r#","starred":false"#, "");
        assert!(!serde_json::from_str::<HistoryEntry>(&legacy).unwrap().starred);
        assert!(json.ends_with(r#""starred":false,"mode":"whole_take","kind":"dictation"}"#), "segments / live_error / edit are omitted when None: {json}");
        let streamed = HistoryEntry {
            mode: OutputMode::LiveInject,
            segments: Some(vec![Segment { text: "你好。".into(), start_ms: 0, end_ms: 900 }]),
            live_error: Some("decoder panicked".into()),
            ..e
        };
        let json = serde_json::to_string(&streamed).unwrap();
        assert!(json.contains(r#""mode":"live_inject","segments":[{"text":"你好。","start_ms":0,"end_ms":900}],"live_error":"decoder panicked""#), "{json}");
        assert_eq!(serde_json::from_str::<HistoryEntry>(&json).unwrap(), streamed);
    }

    /// docs/dictation.md §4.2: a clipboard fallback carries its code; an entry written before the
    /// codes reads without one, and its wire shape did not change.
    #[test]
    fn clipboard_outcomes_carry_their_code_and_legacy_ones_read_without() {
        let coded = Outcome::clipboard(InjectNote::new(ClipboardCode::NoPermission, "enigo: no permission"));
        assert_eq!(coded, Outcome::Clipboard { reason: "enigo: no permission".into(), code: Some(ClipboardCode::NoPermission) });
        let json = serde_json::to_string(&coded).unwrap();
        assert_eq!(json, r#"{"kind":"clipboard","reason":"enigo: no permission","code":"no_permission"}"#);
        assert_eq!(serde_json::from_str::<Outcome>(&json).unwrap(), coded);
        let legacy: Outcome = serde_json::from_str(r#"{"kind":"clipboard","reason":"目标窗口没有焦点"}"#).unwrap();
        assert_eq!(legacy, Outcome::Clipboard { reason: "目标窗口没有焦点".into(), code: None });
        let names = [
            (ClipboardCode::NoPermission, "no_permission"),
            (ClipboardCode::NoTool, "no_tool"),
            (ClipboardCode::NoDisplay, "no_display"),
            (ClipboardCode::SecureInput, "secure_input"),
            (ClipboardCode::ElevatedTarget, "elevated_target"),
            (ClipboardCode::Other, "other"),
        ];
        for (code, name) in names {
            assert_eq!(serde_json::to_string(&code).unwrap(), format!("\"{name}\""));
        }
    }

    /// docs/dictation.md §16.3: the hits are on the wire only when something fired, and an entry
    /// written before the vocabulary reads without them.
    #[test]
    fn vocabulary_hits_round_trip_and_are_omitted_when_absent() {
        let plain = entry("你好");
        let json = serde_json::to_string(&plain).unwrap();
        assert!(!json.contains("vocabulary"), "{json}");
        let id = Uuid::nil();
        let hits = VocabularyHits { corrections: vec![crate::VocabularyHit { id, count: 2 }], rules: vec![crate::VocabularyHit { id, count: 1 }] };
        let fired = HistoryEntry { vocabulary: Some(hits.clone()), ..plain };
        let json = serde_json::to_string(&fired).unwrap();
        assert!(json.ends_with(r#""vocabulary":{"corrections":[{"id":"00000000-0000-0000-0000-000000000000","count":2}],"rules":[{"id":"00000000-0000-0000-0000-000000000000","count":1}]},"kind":"dictation"}"#), "{json}");
        assert_eq!(serde_json::from_str::<HistoryEntry>(&json).unwrap().vocabulary, Some(hits));
        let dir = tempfile::tempdir().unwrap();
        let mut store = HistoryStore::open(dir.path());
        store.push(fired.clone(), MAX_ENTRIES).unwrap();
        assert_eq!(HistoryStore::open(dir.path()).entries()[0], fired);
    }

    /// docs/dictation.md §19: an edit entry carries its kind, instruction and selection; an entry
    /// written before voice edit reads as dictation without an edit record.
    #[test]
    fn edit_entries_round_trip_and_legacy_entries_read_as_dictation() {
        let edit = HistoryEntry {
            raw_text: "改的更正式".into(),
            text: "尊敬的各位同事：会议改到周四上午十点。".into(),
            kind: TakeKind::Edit,
            edit: Some(EditRecord { instruction: "改得更正式".into(), selection: "会议改到周四十点哈".into() }),
            ..entry("x")
        };
        let json = serde_json::to_string(&edit).unwrap();
        assert!(json.ends_with(r#""kind":"edit","edit":{"instruction":"改得更正式","selection":"会议改到周四十点哈"}}"#), "{json}");
        assert_eq!(serde_json::from_str::<HistoryEntry>(&json).unwrap(), edit);
        let dir = tempfile::tempdir().unwrap();
        let mut store = HistoryStore::open(dir.path());
        store.push(edit.clone(), MAX_ENTRIES).unwrap();
        assert_eq!(HistoryStore::open(dir.path()).entries()[0], edit);
        let legacy = serde_json::to_string(&entry("旧")).unwrap().replace(r#","kind":"dictation""#, "");
        let old: HistoryEntry = serde_json::from_str(&legacy).unwrap();
        assert_eq!((old.kind, old.edit), (TakeKind::Dictation, None));
    }

    /// docs/dictation.md §18.6: the app and the scene are on the wire only when known, and an entry
    /// written before scenes reads without them.
    #[test]
    fn app_and_scene_round_trip_and_are_omitted_when_absent() {
        let plain = entry("你好");
        let json = serde_json::to_string(&plain).unwrap();
        assert!(!json.contains("\"app\"") && !json.contains("\"scene\""), "{json}");
        assert!(!json.contains("\"preset\""), "no clean-up, no preset: {json}");
        let with = HistoryEntry {
            app: Some(AppRef { id: "slack".into(), name: "Slack".into() }),
            scene: Some(SceneRef { id: Uuid::nil(), name: "聊天".into() }),
            preset: Some(PresetRef { id: crate::presets::PresetId::Builtin(crate::presets::BuiltinPreset::Chat), name: "口语聊天".into() }),
            ..plain
        };
        let json = serde_json::to_string(&with).unwrap();
        assert!(
            json.ends_with(r#""app":{"id":"slack","name":"Slack"},"scene":{"id":"00000000-0000-0000-0000-000000000000","name":"聊天"},"preset":{"id":"chat","name":"口语聊天"}}"#),
            "{json}"
        );
        assert_eq!(serde_json::from_str::<HistoryEntry>(&json).unwrap(), with);
        let dir = tempfile::tempdir().unwrap();
        let mut store = HistoryStore::open(dir.path());
        store.push(with.clone(), MAX_ENTRIES).unwrap();
        assert_eq!(HistoryStore::open(dir.path()).entries()[0], with);
    }

    /// A `history.json` written before docs/dictation.md §12 (no `mode` / `segments` /
    /// `live_error`) loads unchanged: every entry reads as a whole take and is re-saved with the
    /// new fields.
    #[test]
    fn legacy_history_without_output_mode_loads_as_whole_take() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = format!(
            r#"{{"schema":{HISTORY_SCHEMA},"entries":[{{"id":"9b1deb4d-3b7d-4bad-9bdd-2b0d7b3dcb6d","at_ms":1758700600000,"raw_text":"把 fetchUser 改成 async","text":"把 fetchUser 改成 async。","refined":true,"asr_model":"Qwen/Qwen3-ASR-1.7B","refine_model":"qwen/qwen3.8-27b","duration_ms":3200,"asr_ms":640,"refine_ms":2400,"outcome":{{"kind":"inserted","via":"paste"}},"starred":true}}]}}"#
        );
        std::fs::write(dir.path().join(HISTORY_FILE_NAME), legacy).unwrap();
        let mut store = HistoryStore::open(dir.path());
        assert_eq!(store.entries().len(), 1, "the legacy file is not set aside");
        assert!(!dir.path().join("history.json.corrupt").exists());
        let old = &store.entries()[0];
        assert_eq!(old.mode, OutputMode::WholeTake);
        assert_eq!(old.segments, None);
        assert_eq!(old.live_error, None);
        assert!(old.starred && old.refined);
        assert_eq!(old.text, "把 fetchUser 改成 async。");
        // Mixed old and new entries coexist and round-trip through disk.
        let new = HistoryEntry { mode: OutputMode::StreamingFinal, segments: Some(Vec::new()), ..entry("新") };
        store.push(new.clone(), MAX_ENTRIES).unwrap();
        let reopened = HistoryStore::open(dir.path());
        assert_eq!(reopened.entries()[0], new);
        assert_eq!(reopened.entries()[1].mode, OutputMode::WholeTake);
        let saved = std::fs::read_to_string(dir.path().join(HISTORY_FILE_NAME)).unwrap();
        assert!(saved.contains(r#""mode": "whole_take""#) && saved.contains(r#""mode": "streaming_final""#), "{saved}");
    }
}
