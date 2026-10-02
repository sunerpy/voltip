use std::time::{Duration, Instant};

use super::store::{HistoryFile, IMPORTED_DIGEST_KEY, IMPORTING_FILE_NAME};
use super::*;
use crate::CoreError;
use crate::dictation::{ClipboardCode, InjectNote, OutputMode, Segment, TakeKind, Via};
use crate::presets::PresetRef;
use crate::scenes::{AppRef, BuiltinScene, MAX_RECENT_APPS, SceneRef};
use crate::vocabulary::VocabularyHits;
use uuid::Uuid;

pub(super) fn entry(text: &str) -> HistoryEntry {
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
        processed: None,
    }
}

/// `n` entries a minute apart, newest first (the order of `history.json`), the oldest at `start`.
fn entries(n: usize, start: u64) -> Vec<HistoryEntry> {
    (0..n).rev().map(|i| HistoryEntry { at_ms: start + i as u64 * 60_000, ..entry(&format!("第{i}句")) }).collect()
}

fn write_legacy(dir: &std::path::Path, entries: &[HistoryEntry]) -> Vec<u8> {
    let bytes = serde_json::to_vec_pretty(&HistoryFile { schema: HISTORY_SCHEMA, entries: entries.to_vec() }).unwrap();
    std::fs::write(dir.join(HISTORY_FILE_NAME), &bytes).unwrap();
    bytes
}

/// The files `history.json` was renamed to by the import.
fn retired(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> =
        std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()?.file_name().into_string().ok()).filter(|n| n.starts_with("history.json.imported-")).collect();
    names.sort();
    names
}

fn stored_json(dir: &std::path::Path) -> Vec<String> {
    let conn = rusqlite::Connection::open(dir.join(HISTORY_DB_FILE_NAME)).unwrap();
    let mut stmt = conn.prepare("SELECT json FROM entries ORDER BY at_ms DESC, rowid DESC").unwrap();
    stmt.query_map([], |r| r.get::<_, String>(0)).unwrap().map(Result::unwrap).collect()
}

fn page(query: HistoryQuery) -> HistoryQuery {
    HistoryQuery { limit: MAX_QUERY_LIMIT, ..query }
}

#[test]
fn retention_keeps_the_newest_and_trims_on_demand() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(dir.path());
    for i in 0..30 {
        store.push(HistoryEntry { at_ms: 1_758_700_000_000 + i, ..entry(&i.to_string()) }, 20).unwrap();
    }
    assert_eq!(store.total(), 20, "push trims to the retention");
    assert_eq!(store.recent(1)[0].raw_text, "29", "newest first");
    assert!(!store.retain_newest(50).unwrap(), "nothing to drop");
    assert!(store.retain_newest(MIN_KEEP).unwrap());
    assert_eq!(store.total(), MIN_KEEP);
    assert_eq!(HistoryStore::open(dir.path()).total(), MIN_KEEP, "the trim was written");
    assert_eq!(HistoryStore::open(dir.path()).recent(RECENT_ENTRIES).last().unwrap().raw_text, "20", "the oldest went");
    assert!(!store.retain_newest(usize::MAX).unwrap(), "never above the cap");
}

#[test]
fn roundtrip_delete_star_clear_and_cap() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(dir.path());
    assert!(store.recent(RECENT_ENTRIES).is_empty());
    let first = entry("第一句");
    let second = HistoryEntry { at_ms: first.at_ms + 1, ..entry("第二句") };
    store.push(first.clone(), MAX_ENTRIES).unwrap();
    store.push(second.clone(), MAX_ENTRIES).unwrap();
    assert_eq!(store.recent(RECENT_ENTRIES)[0].id, second.id, "newest first");
    let reopened = HistoryStore::open(dir.path());
    assert_eq!(reopened.recent(RECENT_ENTRIES), store.recent(RECENT_ENTRIES));
    assert!(store.star(first.id, true).unwrap());
    assert!(store.star(first.id, true).unwrap(), "idempotent");
    assert!(!store.star(Uuid::new_v4(), true).unwrap());
    assert!(HistoryStore::open(dir.path()).recent(RECENT_ENTRIES)[1].starred);
    assert!(store.delete(second.id).unwrap());
    assert!(!store.delete(second.id).unwrap());
    assert_eq!(HistoryStore::open(dir.path()).total(), 1);
    store.clear().unwrap();
    assert_eq!(HistoryStore::open(dir.path()).total(), 0);
    assert!(format!("{store:?}").contains(HISTORY_DB_FILE_NAME));
    // The cap: a full history (imported in one go) keeps MAX_ENTRIES when more arrive.
    let full = tempfile::tempdir().unwrap();
    write_legacy(full.path(), &entries(MAX_ENTRIES, 1_758_700_000_000));
    let mut store = HistoryStore::open(full.path());
    assert_eq!(store.total(), MAX_ENTRIES);
    for i in 0..5u64 {
        store.push(HistoryEntry { at_ms: 1_900_000_000_000 + i, ..entry(&format!("新{i}")) }, MAX_ENTRIES).unwrap();
    }
    assert_eq!(store.total(), MAX_ENTRIES);
    assert_eq!(store.recent(1)[0].raw_text, "新4");
    assert_eq!(HistoryStore::open(full.path()).total(), MAX_ENTRIES);
}

#[test]
fn corrupt_and_foreign_files_are_moved_aside_not_fatal() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(HISTORY_FILE_NAME);
    std::fs::write(&path, b"{not json").unwrap();
    let store = HistoryStore::open(dir.path());
    assert_eq!(store.total(), 0);
    assert!(!path.exists());
    assert!(dir.path().join("history.json.corrupt").exists());
    // A file of another schema is not imported either (into a fresh directory: the database of
    // the first open is there now).
    let foreign = tempfile::tempdir().unwrap();
    std::fs::write(foreign.path().join(HISTORY_FILE_NAME), br#"{"schema":9,"entries":[]}"#).unwrap();
    assert_eq!(HistoryStore::open(foreign.path()).total(), 0);
    assert_eq!(std::fs::read_to_string(foreign.path().join("history.json.corrupt")).unwrap(), r#"{"schema":9,"entries":[]}"#);
    // A directory in place of the file is unreadable (not "not found") and also non-fatal.
    let odd = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(odd.path().join(HISTORY_FILE_NAME)).unwrap();
    let mut store = HistoryStore::open(odd.path());
    assert_eq!(store.total(), 0);
    store.push(entry("照常记录"), MAX_ENTRIES).unwrap();
    assert_eq!(store.total(), 1);
}

#[test]
fn a_file_that_is_not_a_database_is_moved_aside_and_a_new_one_started() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(HISTORY_DB_FILE_NAME), b"this is not an SQLite database, it is a note").unwrap();
    let mut store = HistoryStore::open(dir.path());
    assert_eq!(store.total(), 0);
    assert_eq!(std::fs::read(dir.path().join("history.sqlite3.corrupt")).unwrap(), b"this is not an SQLite database, it is a note");
    store.push(entry("新的一条"), MAX_ENTRIES).unwrap();
    assert_eq!(HistoryStore::open(dir.path()).recent(1)[0].raw_text, "新的一条");
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
    assert!(store.recent(RECENT_ENTRIES).is_empty());
    assert!(store.clear().is_err() && store.delete(Uuid::nil()).is_err() && store.star(Uuid::nil(), true).is_err());
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
    assert_eq!(HistoryStore::open(dir.path()).recent(1)[0], fired);
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
    assert_eq!(HistoryStore::open(dir.path()).recent(1)[0], edit);
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
        scene: Some(SceneRef { id: Uuid::nil(), name: "聊天".into(), builtin: None }),
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
    assert_eq!(HistoryStore::open(dir.path()).recent(1)[0], with);
}

/// A `history.json` written before docs/dictation.md §12 (no `mode` / `segments` /
/// `live_error`) loads unchanged: every entry reads as a whole take and is stored with the new
/// fields.
#[test]
fn legacy_history_without_output_mode_loads_as_whole_take() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = format!(
        r#"{{"schema":{HISTORY_SCHEMA},"entries":[{{"id":"9b1deb4d-3b7d-4bad-9bdd-2b0d7b3dcb6d","at_ms":1758700600000,"raw_text":"把 fetchUser 改成 async","text":"把 fetchUser 改成 async。","refined":true,"asr_model":"Qwen/Qwen3-ASR-1.7B","refine_model":"qwen/qwen3.8-27b","duration_ms":3200,"asr_ms":640,"refine_ms":2400,"outcome":{{"kind":"inserted","via":"paste"}},"starred":true}}]}}"#
    );
    std::fs::write(dir.path().join(HISTORY_FILE_NAME), legacy).unwrap();
    let mut store = HistoryStore::open(dir.path());
    assert_eq!(store.total(), 1, "the legacy file is not set aside");
    assert!(!dir.path().join("history.json.corrupt").exists());
    let old = &store.recent(1)[0];
    assert_eq!(old.mode, OutputMode::WholeTake);
    assert_eq!(old.segments, None);
    assert_eq!(old.live_error, None);
    assert!(old.starred && old.refined);
    assert_eq!(old.text, "把 fetchUser 改成 async。");
    // Mixed old and new entries coexist and round-trip through disk.
    let new = HistoryEntry { mode: OutputMode::StreamingFinal, segments: Some(Vec::new()), at_ms: 1_758_700_700_000, ..entry("新") };
    store.push(new.clone(), MAX_ENTRIES).unwrap();
    let reopened = HistoryStore::open(dir.path());
    assert_eq!(reopened.recent(1)[0], new);
    assert_eq!(reopened.recent(2)[1].mode, OutputMode::WholeTake);
    let saved = stored_json(dir.path()).join("\n");
    assert!(saved.contains(r#""mode":"whole_take""#) && saved.contains(r#""mode":"streaming_final""#), "{saved}");
}

/// docs/dictation.md §4.3: the file of before is imported once, its order kept, and renamed
/// rather than deleted.
#[test]
fn the_legacy_file_is_imported_once_and_kept_under_a_new_name() {
    let dir = tempfile::tempdir().unwrap();
    let list = entries(7, 1_758_700_000_000);
    let bytes = write_legacy(dir.path(), &list);
    let store = HistoryStore::open(dir.path());
    assert_eq!(store.recent(RECENT_ENTRIES), list, "newest first, as the file had them");
    assert!(!dir.path().join(HISTORY_FILE_NAME).exists());
    let kept = retired(dir.path());
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(std::fs::read(dir.path().join(&kept[0])).unwrap(), bytes, "the file is kept as it was");
    drop(store);
    assert_eq!(HistoryStore::open(dir.path()).total(), 7, "a second start imports nothing");
}

/// Step 1: an import that stopped half-way (its database still called `.importing`) is thrown
/// away and done again.
#[test]
fn an_import_that_stopped_is_done_again_from_the_start() {
    let dir = tempfile::tempdir().unwrap();
    let list = entries(3, 1_758_700_000_000);
    write_legacy(dir.path(), &list);
    std::fs::write(dir.path().join(IMPORTING_FILE_NAME), b"half a database").unwrap();
    std::fs::write(dir.path().join(format!("{IMPORTING_FILE_NAME}-journal")), b"its journal").unwrap();
    let store = HistoryStore::open(dir.path());
    assert_eq!(store.recent(RECENT_ENTRIES), list);
    assert!(!dir.path().join(IMPORTING_FILE_NAME).exists());
    assert!(!dir.path().join(format!("{IMPORTING_FILE_NAME}-journal")).exists());
    assert_eq!(retired(dir.path()).len(), 1);
}

/// Step 4: a stop between putting the database in place and renaming the file only renames the
/// file; nothing is imported twice.
#[test]
fn a_stop_before_the_file_was_renamed_only_renames_it() {
    let dir = tempfile::tempdir().unwrap();
    let list = entries(4, 1_758_700_000_000);
    let bytes = write_legacy(dir.path(), &list);
    drop(HistoryStore::open(dir.path()));
    // Put the file back as if the rename had not happened.
    let kept = retired(dir.path());
    std::fs::rename(dir.path().join(&kept[0]), dir.path().join(HISTORY_FILE_NAME)).unwrap();
    let conn = rusqlite::Connection::open(dir.path().join(HISTORY_DB_FILE_NAME)).unwrap();
    let digest: String = conn.query_row("SELECT value FROM meta WHERE key = ?1", [IMPORTED_DIGEST_KEY], |r| r.get(0)).unwrap();
    drop(conn);
    use sha2::Digest as _;
    assert_eq!(digest, hex::encode(sha2::Sha256::digest(&bytes)));
    let store = HistoryStore::open(dir.path());
    assert_eq!(store.total(), 4, "not imported twice");
    assert!(!dir.path().join(HISTORY_FILE_NAME).exists());
    assert_eq!(retired(dir.path()).len(), 1);
    assert!(!dir.path().join("history.json.corrupt").exists());
}

/// Step 4, the other branch: a `history.json` the database did not come from is not imported
/// over it; it is kept aside.
#[test]
fn a_file_beside_a_database_it_did_not_fill_is_moved_aside() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(dir.path());
    store.push(entry("已有的"), MAX_ENTRIES).unwrap();
    drop(store);
    write_legacy(dir.path(), &entries(2, 1_758_700_000_000));
    let store = HistoryStore::open(dir.path());
    assert_eq!(store.total(), 1);
    assert_eq!(store.recent(1)[0].raw_text, "已有的");
    assert!(dir.path().join("history.json.corrupt").exists());
    assert!(retired(dir.path()).is_empty());
}

#[test]
fn corrected_chars_count_changed_characters_without_white_space() {
    assert_eq!(corrected_chars("嗯那个明天开会", "明天开会。"), 4, "three removed, one added");
    assert_eq!(corrected_chars("hello world", "Hello, world."), 3, "case and two marks; the space does not count");
    assert_eq!(corrected_chars("相同", "相 同"), 0);
    assert_eq!(corrected_chars("", "新加的"), 3);
    // Long texts are compared piece by piece: still exact when the change sits inside one piece.
    let long: String = "语音听写".repeat(1500);
    let changed = format!("{}改{}", &long[..3 * 3000], &long[3 * 3001..]);
    assert_eq!(corrected_chars(&long, &changed), 1);
}

#[test]
fn only_dictations_count_toward_the_statistics() {
    assert!(counts_for_stats(&entry("听写")));
    assert!(!counts_for_stats(&HistoryEntry { kind: TakeKind::Edit, ..entry("编辑") }));
    let phone = |kind| HistoryEntry { origin: Some(EntryOrigin { device: "Pixel 8".into(), kind }), ..entry("手机") };
    assert!(counts_for_stats(&phone(OriginKind::Take)), "the phone as a microphone is a dictation");
    assert!(!counts_for_stats(&phone(OriginKind::Typed)));
    assert!(!counts_for_stats(&phone(OriginKind::Clipboard)));
}

/// docs/dictation.md §4.4: filters and search as the history page applied them, now in the
/// database.
#[test]
fn the_reader_pages_filters_and_searches_like_the_page_did() {
    let dir = tempfile::tempdir().unwrap();
    let reader = HistoryReader::new(dir.path());
    assert_eq!(reader.query(&page(HistoryQuery::default())).unwrap(), HistoryPage::default(), "no database yet: empty");
    let mut store = HistoryStore::open(dir.path());
    let base = 1_758_700_000_000;
    let scene = SceneRef { id: Uuid::new_v4(), name: "编程开发".into(), builtin: Some(BuiltinScene::Coding) };
    let rows = [
        HistoryEntry {
            at_ms: base, app: Some(AppRef { id: "code".into(), name: "Visual Studio Code".into() }), scene: Some(scene), ..entry("重构 fetchUser")
        },
        HistoryEntry { at_ms: base + 1, starred: true, outcome: Outcome::Clipboard { reason: "x".into(), code: None }, ..entry("Hello World") },
        HistoryEntry {
            at_ms: base + 2,
            kind: TakeKind::Edit,
            edit: Some(EditRecord { instruction: "改得更正式".into(), selection: "会议改到周四".into() }),
            outcome: Outcome::Failed { reason: "x".into() },
            ..entry("正式一点")
        },
        HistoryEntry { at_ms: base + 3, refine_model: None, asr_model: "sense-voice-small".into(), ..entry("本地识别") },
    ];
    for row in &rows {
        store.push(row.clone(), MAX_ENTRIES).unwrap();
    }
    let ids = |q: HistoryQuery| reader.query(&page(q)).unwrap().entries.iter().map(|e| e.raw_text.clone()).collect::<Vec<_>>();
    assert_eq!(ids(HistoryQuery::default()), ["本地识别", "正式一点", "Hello World", "重构 fetchUser"], "newest first");
    assert_eq!(ids(HistoryQuery { since_ms: Some(base + 2), ..Default::default() }), ["本地识别", "正式一点"]);
    assert_eq!(ids(HistoryQuery { starred: true, ..Default::default() }), ["Hello World"]);
    assert_eq!(ids(HistoryQuery { failed: true, ..Default::default() }), ["正式一点", "Hello World"], "anything not inserted");
    for (needle, expect) in [
        ("  hello world ", "Hello World"),
        ("FETCHUSER", "重构 fetchUser"),
        ("visual studio", "重构 fetchUser"),
        ("code", "重构 fetchUser"),
        ("coding", "重构 fetchUser"),
        ("编程开发", "重构 fetchUser"),
        ("sense-voice", "本地识别"),
        ("更正式", "正式一点"),
        ("周四", "正式一点"),
        ("qwen3.8", "Hello World"),
    ] {
        let found = ids(HistoryQuery { query: needle.into(), ..Default::default() });
        assert!(found.contains(&expect.to_owned()), "{needle:?} → {found:?}");
    }
    assert!(ids(HistoryQuery { query: "没有这句话".into(), ..Default::default() }).is_empty());
    // Pages, with the counts of every page.
    let second = reader.query(&HistoryQuery { offset: 1, limit: 2, ..Default::default() }).unwrap();
    assert_eq!(second.entries.iter().map(|e| e.raw_text.as_str()).collect::<Vec<_>>(), ["正式一点", "Hello World"]);
    assert_eq!((second.matching, second.total), (4, 4));
    let starred = reader.query(&page(HistoryQuery { starred: true, ..Default::default() })).unwrap();
    assert_eq!((starred.matching, starred.total), (1, 4));
    assert_eq!(reader.entry(rows[1].id).unwrap(), Some(rows[1].clone()));
    assert_eq!(reader.entry(Uuid::new_v4()).unwrap(), None);
    for limit in [0, MAX_QUERY_LIMIT + 1] {
        assert!(matches!(reader.query(&HistoryQuery { limit, ..Default::default() }), Err(CoreError::Invalid(_))), "{limit}");
    }
    // A star made after the reader opened is what it reads.
    store.star(rows[0].id, true).unwrap();
    assert!(reader.entry(rows[0].id).unwrap().unwrap().starred);
}

/// docs/dictation.md §4.5: the page sends local midnights (a 23 h and a 25 h day included); the
/// core adds up the dictations between them, and the total, leaving out what does not count.
#[test]
fn stats_add_up_dictations_between_the_boundaries_and_leave_out_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let hour = 3_600_000;
    // Three days: 24 h, 23 h (spring forward), 25 h (fall back).
    let boundaries = [0, 24 * hour, 47 * hour, 72 * hour];
    let reader = HistoryReader::new(dir.path());
    let empty = reader.stats(&boundaries).unwrap();
    assert_eq!(empty.buckets.len(), 3, "no database yet: zeros, one per day");
    assert!(empty.buckets.iter().all(|b| *b == HistoryStatsBucket::default()));
    let mut store = HistoryStore::open(dir.path());
    let dictation = |at_ms, raw: &str, text: &str| HistoryEntry {
        at_ms,
        raw_text: raw.into(),
        text: text.into(),
        duration_ms: 2000,
        asr_ms: 300,
        refine_ms: Some(200),
        ..entry("x")
    };
    store.push(dictation(hour, "嗯明天开会", "明天开会。"), MAX_ENTRIES).unwrap();
    store.push(dictation(46 * hour, "你好", "你好。"), MAX_ENTRIES).unwrap();
    store.push(dictation(47 * hour, "早上好", "早上好。"), MAX_ENTRIES).unwrap();
    store.push(dictation(80 * hour, "以后的", "以后的。"), MAX_ENTRIES).unwrap();
    store.push(HistoryEntry { kind: TakeKind::Edit, ..dictation(2 * hour, "编辑", "编辑后") }, MAX_ENTRIES).unwrap();
    store
        .push(
            HistoryEntry {
                origin: Some(EntryOrigin { device: "Pixel 8".into(), kind: OriginKind::Typed }), ..dictation(3 * hour, "手机打字", "手机打字")
            },
            MAX_ENTRIES,
        )
        .unwrap();
    let stats = reader.stats(&boundaries).unwrap();
    let day = |count, raw_chars, corrected_chars| HistoryStatsBucket {
        count,
        raw_chars,
        corrected_chars,
        spoken_ms: 2000 * u64::from(count),
        latency_ms: 500 * u64::from(count),
    };
    assert_eq!(stats.buckets, vec![day(1, 5, 2), day(1, 2, 1), day(1, 3, 1)]);
    assert_eq!(stats.total, day(4, 13, 5), "every dictation, inside the boundaries or not");
    for bad in [&[][..], &[5][..], &[5, 5][..], &[9, 3][..]] {
        assert!(matches!(reader.stats(bad), Err(CoreError::Invalid(_))), "{bad:?}");
    }
    let too_many: Vec<u64> = (0..=MAX_STATS_BOUNDARIES as u64).collect();
    assert!(matches!(reader.stats(&too_many), Err(CoreError::Invalid(_))));
}

#[test]
fn hits_add_up_per_dictionary_entry_and_rule_and_leave_with_their_entries() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(dir.path());
    let reader = HistoryReader::new(dir.path());
    assert_eq!(reader.hits().unwrap(), HistoryHits::default());
    let (word, rule) = (Uuid::new_v4(), Uuid::new_v4());
    let fired = |corrections: u32, rules: u32| VocabularyHits {
        corrections: vec![crate::VocabularyHit { id: word, count: corrections }],
        rules: vec![crate::VocabularyHit { id: rule, count: rules }],
    };
    let first = HistoryEntry { vocabulary: Some(fired(2, 1)), ..entry("一") };
    store.push(first.clone(), MAX_ENTRIES).unwrap();
    store.push(HistoryEntry { at_ms: first.at_ms + 1, vocabulary: Some(fired(3, 0)), ..entry("二") }, MAX_ENTRIES).unwrap();
    let hits = reader.hits().unwrap();
    assert_eq!(hits.dictionary.get(&word), Some(&5));
    assert_eq!(hits.rules.get(&rule), Some(&1));
    store.delete(first.id).unwrap();
    let hits = reader.hits().unwrap();
    assert_eq!((hits.dictionary.get(&word), hits.rules.get(&rule)), (Some(&3), Some(&0)));
    store.clear().unwrap();
    assert_eq!(reader.hits().unwrap(), HistoryHits::default());
}

#[test]
fn recent_apps_are_newest_first_one_per_id() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(dir.path());
    let app = |id: &str, name: &str| Some(AppRef { id: id.into(), name: name.into() });
    for (i, (id, name)) in [("code", "Code"), ("slack", "Slack"), ("code", "Visual Studio Code"), ("", ""), ("word", "Word")].into_iter().enumerate() {
        let a = if id.is_empty() { None } else { app(id, name) };
        store.push(HistoryEntry { at_ms: 1_758_700_000_000 + i as u64, app: a, ..entry("x") }, MAX_ENTRIES).unwrap();
    }
    let reader = HistoryReader::new(dir.path());
    let names = |limit| reader.recent_apps(limit).unwrap().into_iter().map(|a| a.name).collect::<Vec<_>>();
    assert_eq!(names(10), ["Word", "Visual Studio Code", "Slack"], "the newest name of each id");
    assert_eq!(names(2), ["Word", "Visual Studio Code"]);
    // No history yet, and more distinct applications than the scene editor lists.
    let empty = tempfile::tempdir().unwrap();
    assert!(HistoryReader::new(empty.path()).recent_apps(MAX_RECENT_APPS).unwrap().is_empty());
    for i in 0..30u64 {
        store.push(HistoryEntry { at_ms: 1_758_700_001_000 + i, app: app(&format!("app{i}"), "n"), ..entry("x") }, MAX_ENTRIES).unwrap();
    }
    assert_eq!(reader.recent_apps(MAX_RECENT_APPS).unwrap().len(), MAX_RECENT_APPS);
}

/// WAL: the bridge's reader answers while the core writes, and sees every committed entry.
#[test]
fn the_reader_reads_while_the_core_writes() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(dir.path());
    store.push(entry("第一条"), MAX_ENTRIES).unwrap();
    let reader = std::sync::Arc::new(HistoryReader::new(dir.path()));
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reading = {
        let (reader, done) = (reader.clone(), done.clone());
        std::thread::spawn(move || {
            let mut seen = 0;
            while !done.load(std::sync::atomic::Ordering::SeqCst) {
                let now = reader.query(&page(HistoryQuery::default())).unwrap().total;
                assert!(now >= seen, "a count never goes back: {now} after {seen}");
                seen = now;
            }
            seen
        })
    };
    for i in 0..200u64 {
        store.push(HistoryEntry { at_ms: 1_758_700_000_001 + i, ..entry(&format!("第{i}条")) }, MAX_ENTRIES).unwrap();
    }
    done.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(reading.join().unwrap() <= 201);
    assert_eq!(reader.query(&page(HistoryQuery::default())).unwrap().total, 201);
}

/// docs/dictation.md §4.3: with 20 000 entries the database opens in under 500 ms, a search
/// answers in under 50 ms and the statistics in under 100 ms. Each timing is the best of three
/// runs, so a busy test machine does not fail it by chance.
#[test]
fn twenty_thousand_entries_open_search_and_count_in_time() {
    let dir = tempfile::tempdir().unwrap();
    let start = 1_758_700_000_000;
    write_legacy(dir.path(), &entries(MAX_ENTRIES, start));
    drop(HistoryStore::open(dir.path()));
    let best = |f: &mut dyn FnMut()| {
        (0..3)
            .map(|_| {
                let t = Instant::now();
                f();
                t.elapsed()
            })
            .min()
            .unwrap()
    };
    let opened = best(&mut || assert_eq!(HistoryStore::open(dir.path()).total(), MAX_ENTRIES));
    let reader = HistoryReader::new(dir.path());
    let searched = best(&mut || {
        let page = reader.query(&HistoryQuery { query: "第19999句".into(), limit: 100, ..Default::default() }).unwrap();
        assert_eq!(page.matching, 1);
    });
    let listed = best(&mut || assert_eq!(reader.query(&HistoryQuery { limit: 100, offset: 10_000, ..Default::default() }).unwrap().entries.len(), 100));
    let day = 86_400_000;
    let boundaries: Vec<u64> = (0..MAX_STATS_BOUNDARIES as u64).map(|i| start + i * day).collect();
    let counted = best(&mut || assert_eq!(reader.stats(&boundaries).unwrap().total.count, MAX_ENTRIES as u32));
    assert!(opened < Duration::from_millis(500), "open {opened:?}");
    assert!(searched < Duration::from_millis(50), "search {searched:?}");
    assert!(listed < Duration::from_millis(50), "page {listed:?}");
    assert!(counted < Duration::from_millis(100), "stats {counted:?}");
}

/// 用 AI 预设处理 (docs/dictation.md §22): the result is stored beside the entry's own text (the
/// text is untouched), replaces an earlier one, is found by the search, survives a reopen and is
/// omitted from the JSON until it exists; an unknown id stores nothing.
#[test]
fn a_processed_text_is_stored_beside_the_entry_and_searched() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(dir.path());
    let long = entry("会议记录");
    store.push(long.clone(), MAX_ENTRIES).unwrap();
    assert!(!serde_json::to_string(&long).unwrap().contains("processed"));
    assert_eq!(store.get(long.id).unwrap(), Some(long.clone()));
    assert_eq!(store.get(Uuid::new_v4()).unwrap(), None);
    let processed = |text: &str, at_ms| ProcessedText { text: text.into(), preset: crate::presets::TakePreset::default().to_ref(), at_ms };
    assert!(store.set_processed(long.id, processed("- 第一版要点", 1)).unwrap());
    assert!(store.set_processed(long.id, processed("- 预算已批准", 2)).unwrap(), "replaces the earlier one");
    assert!(!store.set_processed(Uuid::new_v4(), processed("x", 3)).unwrap());
    let stored = HistoryStore::open(dir.path()).get(long.id).unwrap().unwrap();
    assert_eq!(stored.text, long.text, "the entry's own text stays");
    assert_eq!(stored.processed, Some(Box::new(processed("- 预算已批准", 2))));
    let reader = HistoryReader::new(dir.path());
    let found = reader.query(&page(HistoryQuery { query: "预算".into(), ..Default::default() })).unwrap();
    assert_eq!(found.entries.len(), 1);
    assert!(
        reader.query(&page(HistoryQuery { query: "第一版".into(), ..Default::default() })).unwrap().entries.is_empty(),
        "the replaced text is not searched"
    );
    let blocked = dir.path().join("blocked");
    std::fs::write(&blocked, b"x").unwrap();
    let mut broken = HistoryStore::open(&blocked);
    assert!(broken.get(long.id).is_err() && broken.set_processed(long.id, processed("x", 4)).is_err());
}

// ---------------- numbered changes (docs/dictation.md §20.8) ----------------

/// The rows of `sync_revs`, by revision: `(id, rev, deleted)`.
fn revs(dir: &std::path::Path) -> Vec<(Uuid, u64, bool)> {
    let conn = rusqlite::Connection::open(dir.join(HISTORY_DB_FILE_NAME)).unwrap();
    let mut stmt = conn.prepare("SELECT id, rev, deleted FROM sync_revs ORDER BY rev").unwrap();
    stmt.query_map([], |r| Ok((r.get::<_, String>(0)?.parse().unwrap(), r.get::<_, i64>(1)? as u64, r.get::<_, bool>(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn floor(dir: &std::path::Path) -> u64 {
    let conn = rusqlite::Connection::open(dir.join(HISTORY_DB_FILE_NAME)).unwrap();
    conn.query_row("SELECT value FROM meta WHERE key = 'sync_floor'", [], |r| r.get::<_, String>(0)).unwrap().parse().unwrap()
}

const ALL: usize = usize::MAX;

#[test]
fn every_write_takes_a_revision() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(dir.path());
    let [a, b, c] = [0u64, 1, 2].map(|i| HistoryEntry { at_ms: 1_758_700_000_000 + i, ..entry(&format!("第{i}句")) });
    for e in [&a, &b, &c] {
        store.push(e.clone(), MAX_ENTRIES).unwrap();
    }
    let first = store.changes_since(None, 0, ALL).unwrap();
    assert!(first.reset && !first.more);
    assert_eq!(first.upserts.iter().map(|e| e.id).collect::<Vec<_>>(), [a.id, b.id, c.id], "in revision order");
    assert_eq!((first.head, first.to), (3, 3));
    let epoch = Some(first.epoch);
    let since = |store: &HistoryStore, rev| store.changes_since(epoch, rev, ALL).unwrap();

    assert!(store.star(a.id, true).unwrap());
    assert!(store.star(a.id, true).unwrap(), "no change, no revision");
    let starred = since(&store, 3);
    assert!(!starred.reset);
    assert_eq!((starred.upserts.len(), starred.upserts[0].id, starred.upserts[0].starred, starred.to), (1, a.id, true, 4));

    let processed = ProcessedText { text: "处理后".into(), preset: PresetRef { id: crate::presets::PresetId::default(), name: "校对".into() }, at_ms: 9 };
    assert!(store.set_processed(b.id, processed.clone()).unwrap());
    let batch = since(&store, 4);
    assert_eq!(batch.upserts[0].processed.as_deref(), Some(&processed));

    assert!(store.delete(c.id).unwrap());
    assert!(!store.delete(c.id).unwrap(), "not there: no revision");
    let batch = since(&store, 5);
    assert_eq!((batch.upserts.len(), batch.deletes.as_slice(), batch.to), (0, [c.id].as_slice(), 6));

    // `push` past `keep` and `retain_newest` record what they dropped.
    let d = HistoryEntry { at_ms: 1_758_700_000_010, ..entry("第3句") };
    store.push(d.clone(), 2).unwrap();
    let batch = since(&store, 6);
    assert_eq!(batch.upserts.iter().map(|e| e.id).collect::<Vec<_>>(), [d.id]);
    assert_eq!(batch.deletes, [a.id], "the oldest went");
    assert!(store.retain_newest(MIN_KEEP).unwrap() || store.total() <= MIN_KEEP);
    store.push(HistoryEntry { at_ms: 1_758_700_000_011, ..entry("第4句") }, 1).unwrap();
    let batch = since(&store, batch.to);
    assert_eq!(batch.deletes.len(), 2, "two dropped to keep one: {batch:?}");
    assert_eq!(store.total(), 1);

    // `clear` raises the floor past every revision: the next request starts over.
    store.clear().unwrap();
    let after = since(&store, batch.to);
    assert!(after.reset && after.upserts.is_empty() && after.deletes.is_empty());
    assert_eq!(after.to, after.head);
    assert!(revs(dir.path()).is_empty());
    assert_eq!(floor(dir.path()), after.head);
}

#[test]
fn a_database_from_before_gets_its_entries_numbered() {
    let dir = tempfile::tempdir().unwrap();
    let old = entries(5, 1_758_700_000_000);
    {
        let mut store = HistoryStore::open(dir.path());
        for e in old.iter().rev() {
            store.push(e.clone(), MAX_ENTRIES).unwrap();
        }
    }
    // As a version 1 database: no revisions, no identity.
    let conn = rusqlite::Connection::open(dir.path().join(HISTORY_DB_FILE_NAME)).unwrap();
    conn.execute_batch("DELETE FROM sync_revs; DELETE FROM meta WHERE key LIKE 'sync_%'; PRAGMA user_version = 1;").unwrap();
    drop(conn);
    let store = HistoryStore::open(dir.path());
    let numbered = revs(dir.path());
    assert_eq!(numbered.len(), 5);
    assert!(numbered.iter().all(|(_, _, deleted)| !deleted));
    assert_eq!(numbered.iter().map(|(id, _, _)| *id).collect::<Vec<_>>(), old.iter().rev().map(|e| e.id).collect::<Vec<_>>(), "oldest first");
    let batch = store.changes_since(None, 0, ALL).unwrap();
    assert_eq!((batch.upserts.len(), batch.head), (5, 5));
    // An imported history.json is numbered the same way.
    let imported = tempfile::tempdir().unwrap();
    write_legacy(imported.path(), &entries(3, 1_758_700_000_000));
    let store = HistoryStore::open(imported.path());
    assert_eq!(store.changes_since(None, 0, ALL).unwrap().upserts.len(), 3);
    let conn = rusqlite::Connection::open(imported.path().join(HISTORY_DB_FILE_NAME)).unwrap();
    assert_eq!(conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i32>(0)).unwrap(), 2);
}

#[test]
fn a_batch_counts_encoded_bytes_and_leaves_the_segments_out() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(dir.path());
    let segment = Segment { text: "第一段".into(), start_ms: 0, end_ms: 900 };
    let all: Vec<HistoryEntry> =
        (0..5u64).map(|i| HistoryEntry { at_ms: 1_758_700_000_000 + i, segments: Some(vec![segment.clone(); 50]), ..entry(&"字".repeat(1000)) }).collect();
    for e in &all {
        store.push(e.clone(), MAX_ENTRIES).unwrap();
    }
    let one = crate::sync::cbor_len(&HistoryEntry { segments: None, ..all[0].clone() });
    let batch = store.changes_since(None, 0, one * 2 + one / 2).unwrap();
    assert_eq!(batch.upserts.len(), 2, "two fit, a third does not");
    assert!(batch.more && batch.reset);
    assert_eq!(batch.to, 2);
    assert!(batch.upserts.iter().all(|e| e.segments.is_none()), "segments stay on the computer");
    assert_eq!(batch.upserts[0].text, all[0].text, "everything else is sent as it is");
    // The next request goes on from `to` without starting over, and a tiny budget still takes one.
    let next = store.changes_since(Some(batch.epoch), batch.to, 1).unwrap();
    assert!(!next.reset && next.more);
    assert_eq!((next.upserts.len(), next.upserts[0].id, next.to), (1, all[2].id, 3));
    let rest = store.changes_since(Some(batch.epoch), next.to, ALL).unwrap();
    assert!(!rest.more);
    assert_eq!((rest.upserts.len(), rest.to), (2, rest.head));
    assert!(batch.shortened.is_empty() && rest.shortened.is_empty());
}

/// regression (plan gate, M7 design): any field of an entry may be huge (a custom model name has no
/// limit), and an entry larger than a body would stop the sync for good. It goes out as its bounded
/// projection instead, listed in `shortened`, and the revision moves on.
#[test]
fn regression_a_huge_entry_goes_out_bounded_and_the_sync_moves_on() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(dir.path());
    let huge = HistoryEntry {
        at_ms: 1_758_700_000_000,
        asr_model: "m".repeat(20_000_000),
        live_error: Some("e".repeat(20_000_000)),
        text: "字".repeat(SHORTENED_TEXT_CHARS + 10),
        vocabulary: Some(VocabularyHits::default()),
        ..entry("很长")
    };
    let after = HistoryEntry { at_ms: 1_758_700_000_001, ..entry("下一条") };
    store.push(huge.clone(), MAX_ENTRIES).unwrap();
    store.push(after.clone(), MAX_ENTRIES).unwrap();
    let batch = store.changes_since(None, 0, crate::sync::BATCH_BUDGET_BYTES).unwrap();
    assert_eq!(batch.upserts[0].id, huge.id);
    assert_eq!(batch.shortened, [huge.id]);
    let sent = &batch.upserts[0];
    assert_eq!(sent.asr_model.chars().count(), SHORTENED_FIELD_CHARS);
    assert!(sent.asr_model.ends_with('…'));
    assert_eq!(sent.live_error.as_ref().unwrap().chars().count(), SHORTENED_FIELD_CHARS);
    assert_eq!(sent.text.chars().count(), SHORTENED_TEXT_CHARS);
    assert!(sent.vocabulary.is_none() && sent.edit.is_none());
    assert_eq!((sent.raw_text.as_str(), sent.at_ms, sent.duration_ms), (huge.raw_text.as_str(), huge.at_ms, huge.duration_ms));
    assert!(crate::sync::cbor_len(sent) < 1_400_000, "{} bytes", crate::sync::cbor_len(sent));
    // The projection is small enough to share the batch: the sync moves past it.
    assert_eq!(batch.upserts.iter().map(|e| e.id).collect::<Vec<_>>(), [huge.id, after.id]);
    assert!(!batch.more);
    assert_eq!(batch.to, batch.head);
    // The entry on the computer is untouched.
    assert_eq!(store.get(huge.id).unwrap().unwrap().asr_model.len(), 20_000_000);
}

#[test]
fn a_phone_that_cannot_go_on_gets_the_whole_history_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(dir.path());
    for e in entries(4, 1_758_700_000_000).into_iter().rev() {
        store.push(e, MAX_ENTRIES).unwrap();
    }
    let epoch = store.changes_since(None, 0, ALL).unwrap().epoch;
    let other = Some(Uuid::new_v4());
    for (asked, since, why) in
        [(Some(epoch), 0, "an empty copy"), (Some(epoch), 99, "ahead of head"), (other, 99, "another history, ahead"), (other, 2, "another history, behind")]
    {
        let batch = store.changes_since(asked, since, ALL).unwrap();
        assert!(batch.reset, "{why}");
        assert_eq!(batch.upserts.len(), 4, "{why}: every live entry (regression: none of the new history's entries is missed)");
        assert!(batch.deletes.is_empty(), "{why}");
        assert_eq!(batch.epoch, epoch);
        // regression: the reset is not repeated.
        let next = store.changes_since(Some(batch.epoch), batch.to, ALL).unwrap();
        assert!(!next.reset && next.upserts.is_empty(), "{why}");
    }
    // Behind the floor: the deletions it missed are gone, so it starts over (with live entries only).
    // A deletion record is pruned once every entry older than it has gone too.
    let first = store.recent(RECENT_ENTRIES).last().unwrap().id;
    assert!(store.delete(first).unwrap());
    for i in 0..4 {
        store.push(HistoryEntry { at_ms: 1_758_800_000_000 + i, ..entry(&format!("新{i}")) }, 3).unwrap();
    }
    let f = floor(dir.path());
    assert!(f > 0, "the deletions older than every live entry were pruned");
    let batch = store.changes_since(Some(epoch), f - 1, ALL).unwrap();
    assert!(batch.reset);
    assert_eq!(batch.upserts.len(), 3);
    assert!(!store.changes_since(Some(epoch), f, ALL).unwrap().reset, "at the floor it goes on");
}

#[test]
fn pruning_keeps_every_live_revision_above_the_floor() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(dir.path());
    for i in 0..60u64 {
        store.push(HistoryEntry { at_ms: 1_758_700_000_000 + i, ..entry(&i.to_string()) }, MIN_KEEP).unwrap();
        if i % 7 == 0 {
            let newest = store.recent(1)[0].id;
            store.delete(newest).unwrap();
        }
        let rows = revs(dir.path());
        let f = floor(dir.path());
        let live: Vec<u64> = rows.iter().filter(|r| !r.2).map(|r| r.1).collect();
        assert!(live.iter().all(|rev| *rev > f), "step {i}: live {live:?} floor {f}");
        let oldest_live = live.iter().min().copied().unwrap_or(u64::MAX);
        assert!(rows.iter().filter(|r| r.2).all(|r| r.1 > oldest_live), "step {i}: only deletions newer than every live entry stay");
        assert_eq!(live.len(), store.total());
    }
    // Bounded by the history, not by how much was written: the live entries, the deletions of the
    // last `keep` pushes, and the deletes made meanwhile.
    assert!(revs(dir.path()).len() <= 3 * MIN_KEEP, "bounded: {}", revs(dir.path()).len());
}

#[test]
fn a_record_from_a_phone_is_written_once_and_never_comes_back() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(dir.path());
    let now = 1_758_800_000_000;
    let record = HistoryEntry { origin: Some(EntryOrigin { device: "Pixel".into(), kind: OriginKind::Standalone }), ..entry("手机上说的") };
    assert!(store.insert_received(record.clone(), MAX_ENTRIES, now).unwrap());
    assert!(!store.insert_received(record.clone(), MAX_ENTRIES, now).unwrap(), "already there");
    assert_eq!(store.total(), 1);
    let batch = store.changes_since(None, 0, ALL).unwrap();
    assert_eq!(batch.upserts[0].origin, record.origin);
    // Deleted on the computer, then sent again by the phone (its confirmation was lost).
    assert!(store.delete(record.id).unwrap());
    assert!(!store.insert_received(record.clone(), MAX_ENTRIES, now + 1).unwrap(), "regression: a deleted record is not written back");
    assert_eq!(store.total(), 0);
    // After 90 days the id is forgotten; the history's `keep` applies to received records too.
    let later = now + 91 * 24 * 60 * 60 * 1000;
    let other = HistoryEntry { at_ms: 2, ..entry("另一条") };
    assert!(store.insert_received(other, MAX_ENTRIES, later).unwrap());
    assert!(store.insert_received(record.clone(), 1, later).unwrap(), "forgotten after 90 days");
    assert_eq!(store.total(), 1, "keep");
}

/// regression (plan gate, M7 design round 4): a phone remembers which of its records a computer
/// confirmed in `uploaded`. Every way an entry leaves takes its row along, and a confirmation that
/// arrives after the user deleted the record leaves nothing behind, so the table never outgrows the
/// history.
#[test]
fn regression_uploaded_rows_never_outlive_their_entries() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(dir.path());
    let all: Vec<HistoryEntry> = (0..6u64).map(|i| HistoryEntry { at_ms: 1_758_700_000_000 + i, ..entry(&i.to_string()) }).collect();
    for e in &all {
        store.push(e.clone(), MAX_ENTRIES).unwrap();
    }
    let ids: Vec<Uuid> = all.iter().map(|e| e.id).collect();
    assert_eq!(store.mark_uploaded(&ids, "ab12", 5).unwrap(), 6);
    assert_eq!(store.mark_uploaded(&ids, "ab12", 6).unwrap(), 0, "once");
    let uploaded = |dir: &std::path::Path| -> usize {
        let conn = rusqlite::Connection::open(dir.join(HISTORY_DB_FILE_NAME)).unwrap();
        conn.query_row("SELECT COUNT(*) FROM uploaded", [], |r| r.get::<_, i64>(0)).unwrap() as usize
    };
    assert!(store.delete(ids[5]).unwrap());
    assert_eq!(uploaded(dir.path()), 5, "delete");
    store.push(HistoryEntry { at_ms: 1_758_700_000_100, ..entry("新") }, 5).unwrap();
    assert_eq!(uploaded(dir.path()), 4, "push past keep");
    assert!(store.retain_newest(MIN_KEEP).is_ok());
    store.push(HistoryEntry { at_ms: 1_758_700_000_101, ..entry("新2") }, 2).unwrap();
    assert!(uploaded(dir.path()) <= store.total(), "rows never outnumber entries");
    // A late confirmation for a record deleted meanwhile writes nothing.
    assert_eq!(store.mark_uploaded(&[ids[5], Uuid::new_v4()], "ab12", 7).unwrap(), 0);
    store.clear().unwrap();
    assert_eq!(uploaded(dir.path()), 0, "clear");
}

#[test]
fn the_outbox_is_the_phones_own_unconfirmed_records_oldest_first() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(dir.path());
    let own: Vec<HistoryEntry> =
        (0..5u64).map(|i| HistoryEntry { at_ms: 1_758_700_000_000 + i, segments: Some(Vec::new()), ..entry(&format!("本机{i}")) }).collect();
    let taken = HistoryEntry { at_ms: 1_758_600_000_000, origin: Some(EntryOrigin { device: "Pixel".into(), kind: OriginKind::Take }), ..entry("别处") };
    store.push(taken, MAX_ENTRIES).unwrap();
    for e in own.iter().rev() {
        store.push(e.clone(), MAX_ENTRIES).unwrap();
    }
    let batch = store.outbox(ALL, 3).unwrap();
    assert_eq!(
        batch.records.iter().map(|e| e.id).collect::<Vec<_>>(),
        own[..3].iter().map(|e| e.id).collect::<Vec<_>>(),
        "own records only, oldest first, at most 3"
    );
    assert!(batch.records.iter().all(|e| e.segments.is_none()));
    assert!(batch.too_large.is_empty());
    store.mark_uploaded(&[own[0].id, own[1].id], "ab12", 1).unwrap();
    let next = store.outbox(1, 200).unwrap();
    assert_eq!(next.records.iter().map(|e| e.id).collect::<Vec<_>>(), [own[2].id], "a tiny budget still takes one");
    // A record too large to upload is passed over and named; the ones after it still go.
    let big = HistoryEntry { at_ms: 1_758_699_000_000, text: "x".repeat(crate::sync::MAX_ENTRY_BYTES + 1), ..entry("太大") };
    store.push(big.clone(), MAX_ENTRIES).unwrap();
    let with_big = store.outbox(ALL, 200).unwrap();
    assert_eq!(with_big.too_large, [big.id]);
    assert_eq!(with_big.records.len(), 3);
}
