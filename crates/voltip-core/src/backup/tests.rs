//! Tests of the backup file, its checks, the merges and the history's restore (docs/dictation.md
//! §4.6). Dictionary entries and rules are built from JSON, as a backup carries them, so the tests
//! need no edit when those structs gain a field with a default.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use uuid::Uuid;
use voltip_protocol::Platform;

use super::file::write_backup;
use super::*;
use crate::dictation::fakes::history_entry;
use crate::history::{HISTORY_DB_FILE_NAME, HistoryQuery, HistoryReader, HistoryStore, MAX_ENTRIES, MAX_STATS_BOUNDARIES, OriginKind};
use crate::presets::{BuiltinPreset, MAX_PRESETS, PresetId, PresetRef, PresetStore};
use crate::scenes::{BuiltinScene, MAX_SCENES, SceneRef, SceneStore};
use crate::vocabulary::{DictionaryStore, MAX_DICTIONARY_ENTRIES, MAX_RULES, RuleStore, VocabularyHit, VocabularyHits};

const AT: u64 = 1_760_000_000_000;

fn dict(term: &str, heard: &[&str]) -> DictionaryEntry {
    serde_json::from_value(json!({
        "id": Uuid::new_v4(), "term": term, "heard_as": heard, "enabled": true,
        "source": { "kind": "manual" }, "created_at_ms": 1, "updated_at_ms": 2,
    }))
    .unwrap()
}

fn rule(name: &str, kind: &str, pattern: &str) -> ReplacementRule {
    serde_json::from_value(json!({
        "id": Uuid::new_v4(), "name": name, "kind": kind, "pattern": pattern, "replacement": "R",
        "case_sensitive": true, "enabled": true, "created_at_ms": 1, "updated_at_ms": 2,
    }))
    .unwrap()
}

fn preset(name: &str) -> CustomPreset {
    CustomPreset { id: Uuid::new_v4(), name: name.into(), prompt: format!("{name}的提示词"), created_at_ms: 1, updated_at_ms: 2 }
}

fn scene(name: &str, apps: &[&str]) -> Scene {
    serde_json::from_value(json!({
        "id": Uuid::new_v4(), "name": name, "enabled": true,
        "match": { "apps": apps, "title_contains": [] }, "overrides": {},
        "created_at_ms": 1, "updated_at_ms": 2,
    }))
    .unwrap()
}

fn builtin(category: BuiltinScene, platform: Platform) -> Scene {
    let t = category.template(platform);
    Scene {
        id: Uuid::new_v4(),
        name: t.name,
        enabled: t.enabled,
        matching: t.matching,
        overrides: t.overrides,
        created_at_ms: 1,
        updated_at_ms: 1,
        builtin: Some(category),
    }
}

/// A dictation as the history keeps it, with the parts a backup must carry.
fn take(i: usize, at_ms: u64) -> HistoryEntry {
    HistoryEntry {
        at_ms,
        raw_text: format!("第{i}条 今天下午三点在三号会议室讨论沃提普的发布计划请大家准时参加"),
        text: format!("第{i}条：今天下午三点在三号会议室讨论 Voltip 的发布计划，请大家准时参加。"),
        refined: true,
        refine_model: Some("qwen/qwen3.8-27b".into()),
        refine_ms: Some(300),
        duration_ms: 4_000 + (i as u64 % 7) * 500,
        vocabulary: i.is_multiple_of(3).then(|| VocabularyHits { corrections: vec![VocabularyHit { id: Uuid::nil(), count: 1 }], rules: Vec::new() }),
        ..history_entry("x")
    }
}

/// `n` takes a minute apart, newest first.
fn takes(n: usize) -> Vec<HistoryEntry> {
    (0..n).rev().map(|i| take(i, AT + i as u64 * 60_000)).collect()
}

fn input<'a>(dictionary: &'a [DictionaryEntry], rules: &'a [ReplacementRule], scenes: &'a [Scene], presets: &'a [CustomPreset]) -> ExportInput<'a> {
    ExportInput { app_version: "0.1.4", device: "Studio", platform: Some(Platform::Windows), created_at_ms: AT, dictionary, rules, scenes, presets }
}

/// The history of `dir`, newest first, as the store's JSON.
fn stored(dir: &Path) -> Vec<Value> {
    let reader = HistoryReader::new(dir);
    let mut out = Vec::new();
    let mut offset = 0;
    loop {
        let page = reader.query(&HistoryQuery { offset, limit: 200, ..HistoryQuery::default() }).unwrap();
        if page.entries.is_empty() {
            return out;
        }
        offset += 200;
        out.extend(page.entries.iter().map(|e| serde_json::to_value(e).unwrap()));
    }
}

fn stats(dir: &Path) -> crate::history::HistoryStats {
    let boundaries: Vec<u64> = (0..MAX_STATS_BOUNDARIES as u64).map(|d| AT - 86_400_000 + d * 86_400_000).collect();
    HistoryReader::new(dir).stats(&boundaries).unwrap()
}

/// Every part goes out as the stores keep it and comes back equal; the file is one member and one
/// item per line, the history newest first and the counts last; the statistics, derived from the
/// entries, come back with them.
#[test]
fn a_backup_carries_every_part_as_stored_and_the_statistics_follow() {
    let from = tempfile::tempdir().unwrap();
    let mut history = HistoryStore::open(from.path());
    let mut edit = take(1, AT + 5);
    edit.kind = crate::dictation::TakeKind::Edit;
    edit.edit = Some(crate::history::EditRecord { instruction: "改得正式一点".into(), selection: "原文".into() });
    let mut phone = take(2, AT + 4);
    phone.origin = Some(crate::history::EntryOrigin { device: "Pixel".into(), kind: OriginKind::Typed });
    let mut rich = take(3, AT + 3);
    rich.scene = Some(SceneRef { id: Uuid::new_v4(), name: "coding".into(), builtin: Some(BuiltinScene::Coding) });
    rich.preset = Some(PresetRef { id: PresetId::Builtin(BuiltinPreset::Formal), name: "书面语".into() });
    rich.segments = Some(vec![crate::dictation::Segment { text: "一段".into(), start_ms: 0, end_ms: 900 }]);
    rich.processed = Some(Box::new(crate::history::ProcessedText {
        text: "处理后".into(),
        preset: PresetRef { id: PresetId::Builtin(BuiltinPreset::Notes), name: "要点纪要".into() },
        at_ms: AT + 9,
    }));
    for e in [take(0, AT + 1), rich, phone, edit] {
        history.push(e, MAX_ENTRIES).unwrap();
    }
    history.star(history.recent(1)[0].id, true).unwrap();
    let dictionary = vec![dict("Voltip", &["沃提普"]), dict("GitHub", &[])];
    let rules = vec![rule("句号", "literal", "句号"), rule("PR", "regex", r"\bpr (\d+)")];
    let scenes = vec![scene("Slack", &["slack"]), builtin(BuiltinScene::Coding, Platform::Windows)];
    let presets = vec![preset("周报")];
    let mut file = Vec::new();
    let counts = write_backup(&mut file, &from.path().join(HISTORY_DB_FILE_NAME), &input(&dictionary, &rules, &scenes, &presets)).unwrap();
    assert_eq!(counts, BackupCounts { history: 4, dictionary: 2, rules: 2, scenes: 2, presets: 1 });

    let text = String::from_utf8(file.clone()).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(&lines[..3], ["{", "\"format\":\"voltip-backup\",", "\"version\":1,"]);
    assert!(lines.contains(&"\"platform\":\"windows\","), "{text}");
    assert_eq!(lines[lines.len() - 2], r#""counts":{"history":4,"dictionary":2,"rules":2,"scenes":2,"presets":1}"#);
    assert_eq!(lines.iter().filter(|l| l.starts_with("{\"id\"")).count(), 4 + 2 + 2 + 2 + 1, "one item per line");

    let backup = parse(&file).unwrap();
    assert_eq!(backup.manifest.counts, counts);
    assert_eq!((backup.manifest.version, backup.manifest.device.as_str(), backup.manifest.app_version.as_str()), (1, "Studio", "0.1.4"));
    assert_eq!((backup.manifest.created_at_ms, backup.manifest.platform), (AT, Platform::Windows));
    assert_eq!((backup.dictionary, backup.rules, backup.scenes, backup.presets), (dictionary, rules, scenes, presets));
    let as_stored = stored(from.path());
    assert_eq!(backup.history.iter().map(|e| serde_json::to_value(e).unwrap()).collect::<Vec<_>>(), as_stored, "the history's own order");

    // Restored into another computer's empty history: the same entries, the same statistics.
    let to = tempfile::tempdir().unwrap();
    let mut target = HistoryStore::open(to.path());
    let done = target.restore(&backup.history, false, MAX_ENTRIES).unwrap();
    assert_eq!(done, crate::history::Restored { written: 4, present: 0, left_out: 0, removed: 0 });
    assert_eq!(stored(to.path()), as_stored);
    assert_eq!(stats(to.path()), stats(from.path()));
    assert_eq!(stats(to.path()).total.count, 2, "the voice edit and the typed text do not count");
    assert_eq!(HistoryReader::new(to.path()).hits().unwrap(), HistoryReader::new(from.path()).hits().unwrap());
}

/// An export goes through `<name>.part` and leaves nothing else behind; a database that is not
/// there is an empty history; the file name drops what a file system refuses.
#[test]
fn an_export_writes_the_file_whole_and_names_it_safely() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(file_name("Voltip 备份 2026/10/13"));
    assert_eq!(path.file_name().unwrap().to_str().unwrap(), "Voltip 备份 2026 10 13.voltip-backup");
    assert_eq!(file_name(" .. "), "voltip.voltip-backup");
    let summary = export(&path, &dir.path().join("none").join(HISTORY_DB_FILE_NAME), &input(&[], &[], &[], &[])).unwrap();
    assert_eq!(summary.counts, BackupCounts::default());
    assert_eq!(summary.bytes, std::fs::metadata(&path).unwrap().len());
    let names: Vec<String> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    assert_eq!(names, [path.file_name().unwrap().to_str().unwrap()], "no part file left");
    assert_eq!(read(&path).unwrap().history, Vec::<HistoryEntry>::new());
    // A directory where the file should go: refused, nothing written, nothing left.
    let blocked = dir.path().join("blocked.voltip-backup");
    std::fs::create_dir(&blocked).unwrap();
    assert!(matches!(export(&blocked, &dir.path().join(HISTORY_DB_FILE_NAME), &input(&[], &[], &[], &[])), Err(BackupError::Write(_))));
    assert!(blocked.is_dir() && !dir.path().join("blocked.voltip-backup.part").exists());
    // A name without an extension gets one, unless that name is taken.
    assert_eq!(save_path(&dir.path().join("备份")), dir.path().join("备份.voltip-backup"));
    assert_eq!(save_path(&dir.path().join("备份.json")), dir.path().join("备份.json"));
    assert_eq!(save_path(&dir.path().join("blocked")), dir.path().join("blocked"), "blocked.voltip-backup exists");
}

/// Every refusal before anything changes: not a backup, damaged, newer, too large, unreadable.
#[test]
fn reading_refuses_what_it_cannot_import_and_says_why() {
    let mut good = Vec::new();
    write_backup(&mut good, Path::new("/nonexistent/history.sqlite3"), &input(&[dict("Voltip", &[])], &[], &[], &[])).unwrap();
    assert!(parse(&good).is_ok());
    let text = String::from_utf8(good.clone()).unwrap();
    let cases: Vec<(Vec<u8>, BackupError)> = vec![
        (b"\x89PNG\r\n\x1a\n".to_vec(), BackupError::NotBackup),
        (b"[1, 2, 3]".to_vec(), BackupError::NotBackup),
        (br#"{"format":"something-else","version":1}"#.to_vec(), BackupError::NotBackup),
        (br#"{"version":1,"counts":{}}"#.to_vec(), BackupError::NotBackup),
        (text.replace("\"version\":1,", "\"version\":2,").into_bytes(), BackupError::TooNew(2)),
        (text.replace("\"version\":1,", "").into_bytes(), BackupError::Damaged("no format version".into())),
        (text.replace("\"dictionary\":1", "\"dictionary\":2").into_bytes(), BackupError::Damaged("the counts do not match the content".into())),
    ];
    for (bytes, want) in cases {
        assert_eq!(parse(&bytes), Err(want), "{}", String::from_utf8_lossy(&bytes));
    }
    // Cut short, or a part that is not what the store keeps: damaged, with the parser's message.
    for bytes in [&good[..good.len() / 2], text.replace("\"term\":\"Voltip\"", "\"term\":7").as_bytes()] {
        assert!(matches!(parse(bytes), Err(BackupError::Damaged(why)) if why.contains("line")), "{}", String::from_utf8_lossy(bytes));
    }
    // A byte-order mark an editor added is fine.
    assert!(parse(&[b"\xEF\xBB\xBF".as_slice(), &good].concat()).is_ok());
    // A later version's additions within version 1: an unknown part and unknown fields are ignored.
    let later =
        text.replacen("\"dictionary\":[", "\"settings\":{\"theme\":\"glass\"},\n\"dictionary\":[", 1).replacen("\"term\":", "\"weight\":3,\"term\":", 1);
    assert_eq!(parse(later.as_bytes()).unwrap().dictionary[0].term, "Voltip");

    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(read(&dir.path().join("missing.voltip-backup")), Err(BackupError::Unreadable(_))));
    assert!(matches!(read(dir.path()), Err(BackupError::Unreadable(_))), "a directory");
    let huge = dir.path().join("huge.voltip-backup");
    std::fs::File::create(&huge).unwrap().set_len(MAX_BACKUP_BYTES + 1).unwrap();
    assert_eq!(read(&huge), Err(BackupError::TooLarge));
    // Each refusal as the shells answer it.
    assert_eq!(OpenOutcome::failed(&BackupError::TooNew(3)), OpenOutcome::Failed { code: OpenFailure::TooNew, detail: String::new(), version: Some(3) });
    assert_eq!(
        serde_json::to_value(OpenOutcome::failed(&BackupError::Damaged("x at line 2".into()))).unwrap(),
        json!({ "kind": "failed", "code": "damaged", "detail": "x at line 2" })
    );
    assert_eq!(ExportOutcome::failed(&BackupError::History("locked".into())), ExportOutcome::Failed { code: ExportFailure::History, detail: "locked".into() });
}

/// Dictionary: the same id or term is a duplicate; an entry not in its stored form is invalid; one
/// whose spellings belong to an entry here contradicts it; past 500 the list is full. Replacing
/// keeps the backup's entries alone, under the same checks among themselves.
#[test]
fn the_dictionary_merges_without_duplicates_or_contradictions() {
    let here = vec![dict("Voltip", &["沃提普"]), dict("GitHub", &["给他哈勃"])];
    let mut same_id = dict("同一条", &[]);
    same_id.id = here[0].id;
    let mut untrimmed = dict("Kiro", &[]);
    untrimmed.term = " Kiro ".into();
    let incoming =
        vec![same_id, dict("voltip", &[]), untrimmed, dict("沃提普", &[]), dict("Gitee", &["给他哈勃"]), dict("Rust", &["GitHub"]), dict("Paseo", &["怕手"])];
    let merged = merge::dictionary(&here, &incoming, ImportMode::Merge);
    assert_eq!(merged.list.iter().map(|e| e.term.as_str()).collect::<Vec<_>>(), ["Voltip", "GitHub", "Paseo"]);
    assert_eq!(merged.list[2], incoming[6], "kept as it was, id and times included");
    assert_eq!((merged.report.restored, merged.report.in_file), (1, 7));
    assert_eq!(merged.report.skipped, Skipped { duplicate: 2, conflict: 3, invalid: 1, ..Skipped::default() });
    crate::vocabulary::check_dictionary(&merged.list).unwrap();

    let replaced = merge::dictionary(&here, &incoming, ImportMode::Replace);
    assert_eq!(replaced.list.iter().map(|e| e.term.as_str()).collect::<Vec<_>>(), ["同一条", "voltip", "沃提普", "Gitee", "Rust", "Paseo"]);
    assert_eq!(replaced.report.skipped, Skipped { invalid: 1, ..Skipped::default() }, "without the entries here nothing contradicts");

    let full: Vec<DictionaryEntry> = (0..MAX_DICTIONARY_ENTRIES).map(|i| dict(&format!("词{i}"), &[])).collect();
    let more = merge::dictionary(&full, &[dict("词0", &[]), dict("新词", &[])], ImportMode::Merge);
    assert_eq!((more.list.len(), more.report.skipped.duplicate, more.report.skipped.limit), (MAX_DICTIONARY_ENTRIES, 1, 1));
}

/// Rules: the same id or name is a duplicate, a regex that does not compile is invalid, past 200
/// the list is full; the backup's rules go after the ones here, in their order.
#[test]
fn the_rules_merge_within_their_limit() {
    let here: Vec<ReplacementRule> = (0..MAX_RULES - 2).map(|i| rule(&format!("r{i}"), "literal", "p")).collect();
    let incoming =
        vec![rule("r0", "literal", "x"), rule("坏的", "regex", "("), rule("新 1", "regex", r"\d+"), rule("新 2", "literal", "y"), rule("新 3", "literal", "z")];
    let merged = merge::rules(&here, &incoming, ImportMode::Merge);
    assert_eq!(merged.list.len(), MAX_RULES);
    assert_eq!(merged.list[MAX_RULES - 2..].iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["新 1", "新 2"]);
    assert_eq!(merged.report.skipped, Skipped { duplicate: 1, limit: 1, invalid: 1, ..Skipped::default() });
    assert_eq!(merged.report.restored, 2);
    crate::vocabulary::check_rules(&merged.list).unwrap();
    let replaced = merge::rules(&here, &incoming, ImportMode::Replace);
    assert_eq!(replaced.list.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["r0", "新 1", "新 2", "新 3"]);
}

/// Presets: a name here (ignoring ASCII case) is a duplicate, and the scenes of the backup that
/// name that preset name the one here instead; past 30 the list is full.
#[test]
fn presets_merge_and_the_scenes_follow_a_preset_skipped_for_its_name() {
    let here = vec![preset("周报"), preset("Mail")];
    let theirs = vec![preset("周报"), preset("mail"), preset("纪要")];
    let (merged, same) = merge::presets(&here, &theirs, ImportMode::Merge);
    assert_eq!(merged.list.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["周报", "Mail", "纪要"]);
    assert_eq!((merged.report.restored, merged.report.skipped.duplicate), (1, 2));
    assert_eq!(same, HashMap::from([(theirs[0].id, here[0].id), (theirs[1].id, here[1].id)]));
    let full: Vec<CustomPreset> = (0..MAX_PRESETS).map(|i| preset(&format!("预设{i}"))).collect();
    assert_eq!(merge::presets(&full, &[preset("再一个")], ImportMode::Merge).0.report.skipped.limit, 1);

    let mut weekly = scene("Outlook", &["outlook"]);
    weekly.overrides.refine_preset = Some(PresetId::Custom(theirs[0].id));
    let mut notes = scene("Notion", &["notion"]);
    notes.overrides.refine_preset = Some(PresetId::Custom(theirs[2].id));
    let scenes = merge::scenes(&[], &[weekly, notes], ImportMode::Merge, Platform::Windows, Platform::Windows, &same);
    assert_eq!(scenes.list[0].overrides.refine_preset, Some(PresetId::Custom(here[0].id)), "the preset here of that name");
    assert_eq!(scenes.list[1].overrides.refine_preset, Some(PresetId::Custom(theirs[2].id)), "restored with its own id");
}

/// Scenes: merging keeps the built-in scenes here and puts the backup's own before the first
/// built-in one; replacing gives the built-in scenes here the backup's settings under their own
/// ids, and default applications of another system become this one's; at most 50 of the user's.
#[test]
fn scenes_merge_around_the_built_in_ones_and_replace_them_in_place() {
    let here = vec![scene("Slack", &["slack"]), builtin(BuiltinScene::Coding, Platform::Macos), builtin(BuiltinScene::Chat, Platform::Macos)];
    let mut chat = builtin(BuiltinScene::Chat, Platform::Windows);
    chat.enabled = true;
    chat.overrides.prompt = Some("聊天时口语一些".into());
    let mut coding = builtin(BuiltinScene::Coding, Platform::Windows);
    coding.matching.apps = vec!["code".into(), "my-editor".into()];
    let incoming = vec![scene("slack", &["slack"]), scene("Word", &["winword"]), scene("无应用", &[]), chat.clone(), coding.clone()];
    let merged = merge::scenes(&here, &incoming, ImportMode::Merge, Platform::Macos, Platform::Windows, &HashMap::new());
    assert_eq!(merged.list.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Slack", "Word", "coding", "chat"]);
    assert_eq!(merged.list[2..], here[1..], "the built-in scenes here stay as they are");
    assert_eq!(merged.report.skipped, Skipped { duplicate: 3, invalid: 1, ..Skipped::default() });

    let replaced = merge::scenes(&here, &incoming, ImportMode::Replace, Platform::Macos, Platform::Windows, &HashMap::new());
    assert_eq!(replaced.list.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["slack", "Word", "chat", "coding"]);
    let (new_chat, new_coding) = (&replaced.list[2], &replaced.list[3]);
    assert_eq!((new_chat.id, new_chat.created_at_ms), (here[2].id, here[2].created_at_ms), "the id here: settings may name it");
    assert!(new_chat.enabled && new_chat.overrides.prompt.as_deref() == Some("聊天时口语一些"));
    assert_eq!(new_chat.matching, BuiltinScene::Chat.template(Platform::Macos).matching, "Windows defaults became the Mac's");
    assert_eq!(new_coding.matching.apps, ["code", "my-editor"], "applications the user chose stay");
    assert_eq!(replaced.report.restored, 4);
    let mut twice = chat.clone();
    twice.id = Uuid::new_v4();
    assert_eq!(merge::scenes(&[], &[chat, twice], ImportMode::Replace, Platform::Macos, Platform::Windows, &HashMap::new()).report.skipped.conflict, 1);

    let full: Vec<Scene> = (0..MAX_SCENES).map(|i| scene(&format!("场景{i}"), &["app"])).collect();
    assert_eq!(
        merge::scenes(&full, &[scene("再一个", &["app"])], ImportMode::Merge, Platform::Linux, Platform::Linux, &HashMap::new()).report.skipped.limit,
        1
    );
    // Without built-in scenes (`Other`), a built-in one of the backup has no place.
    assert_eq!(
        merge::scenes(&[], &[builtin(BuiltinScene::Legal, Platform::Linux)], ImportMode::Merge, Platform::Other, Platform::Linux, &HashMap::new())
            .report
            .skipped
            .invalid,
        1
    );
}

/// The stores take a merged list whole, checked with their own rules: one that breaks them is
/// refused and the file stays; the scene list gets back the built-in categories it lacks.
#[test]
fn the_stores_check_a_restored_list_and_write_it_in_one_go() {
    let dir = tempfile::tempdir().unwrap();
    let (mut dictionary, _) = DictionaryStore::open(dir.path());
    let list = vec![dict("Voltip", &["沃提普"]), dict("GitHub", &[])];
    dictionary.restore_list(list.clone()).unwrap();
    assert_eq!(DictionaryStore::open(dir.path()).0.entries(), &list[..]);
    let before = std::fs::read(dir.path().join(crate::vocabulary::DICTIONARY_FILE_NAME)).unwrap();
    let mut untrimmed = dict("x", &[]);
    untrimmed.term = " x ".into();
    for bad in [vec![dict("A", &[]), dict("a", &[])], vec![untrimmed]] {
        assert!(dictionary.restore_list(bad).is_err());
    }
    assert_eq!(std::fs::read(dir.path().join(crate::vocabulary::DICTIONARY_FILE_NAME)).unwrap(), before);
    assert_eq!(dictionary.entries(), &list[..]);

    let (mut rules, _) = RuleStore::open(dir.path());
    assert!(rules.restore_list(vec![rule("坏", "regex", "(")]).is_err());
    rules.restore_list(vec![rule("好", "regex", "a+")]).unwrap();
    assert_eq!(RuleStore::open(dir.path()).0.rules()[0].name, "好");

    let (mut presets, _) = PresetStore::open(dir.path());
    assert!(presets.restore_list(vec![preset("一"), preset("一")]).is_err());
    presets.restore_list(vec![preset("一")]).unwrap();
    assert_eq!(PresetStore::open(dir.path()).0.presets().len(), 1);

    let (mut scenes, _) = SceneStore::open_on(dir.path(), Platform::Linux, AT);
    assert_eq!(scenes.platform(), Platform::Linux);
    assert!(scenes.restore_list(vec![scene("无应用", &[])], AT).is_err(), "a desktop scene names an application");
    scenes.restore_list(vec![scene("Slack", &["slack"])], AT).unwrap();
    let kept = SceneStore::open_on(dir.path(), Platform::Linux, AT).0.scenes().to_vec();
    assert_eq!(kept.len(), 1 + BuiltinScene::ALL.len(), "the built-in categories are back");
    assert_eq!(kept[0].name, "Slack");
}

/// Merging adds the entries not here; the history then holds the newest `keep` of both: the
/// backup's older ones are left out, and the oldest ones here past `keep` go. Within one
/// millisecond the written entry is the newer, as for a push.
#[test]
fn merging_the_history_keeps_the_newest_and_counts_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(dir.path());
    // Here: minutes 0, 2, 4, 6 (newest first in the store).
    let here: Vec<HistoryEntry> = [6u64, 4, 2, 0].iter().map(|m| take(*m as usize, AT + m * 60_000)).collect();
    for e in here.iter().rev() {
        store.push(e.clone(), MAX_ENTRIES).unwrap();
    }
    // The backup: minutes 7, 5, 4 (the same millisecond as one here), 3, 1, an entry here, and
    // one listed twice.
    let at = |m: u64| AT + m * 60_000;
    let incoming = vec![take(70, at(7)), take(50, at(5)), take(40, at(4)), take(30, at(3)), take(10, at(1)), here[1].clone(), take(70, at(7))];
    let mut twice = incoming.clone();
    twice[6].id = twice[0].id;
    let done = store.restore(&twice, false, 6).unwrap();
    assert_eq!(done, crate::history::Restored { written: 4, present: 2, left_out: 1, removed: 2 });
    let texts: Vec<String> = stored(dir.path()).iter().map(|e| e["text"].as_str().unwrap().chars().take(4).collect()).collect();
    assert_eq!(texts, ["第70条", "第6条：", "第50条", "第40条", "第4条：", "第30条"], "newest first; at minute 4 the one written now is the newer");
    assert_eq!(store.total(), 6);
    assert_eq!(HistoryStore::open(dir.path()).total(), 6);
    // Again: everything is here, nothing changes.
    assert_eq!(store.restore(&twice, false, 6).unwrap(), crate::history::Restored { written: 0, present: 6, left_out: 1, removed: 0 });
    assert_eq!(store.total(), 6);
}

/// Replacing clears the history first (the phones' copies start again) and writes the newest
/// `keep` of the backup; the revisions number the new entries.
#[test]
fn replacing_the_history_starts_from_the_backup_alone() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(dir.path());
    for e in takes(5).into_iter().rev() {
        store.push(e, MAX_ENTRIES).unwrap();
    }
    let floor_before = store.changes_since(None, 0, usize::MAX).unwrap().head;
    let backup = takes(30);
    let done = store.restore(&backup, true, 20).unwrap();
    assert_eq!(done, crate::history::Restored { written: 20, present: 0, left_out: 10, removed: 5 });
    let kept = stored(dir.path());
    assert_eq!(kept.len(), 20);
    assert_eq!(kept[0]["id"], json!(backup[0].id), "the newest of the backup first");
    assert_eq!(kept[19]["id"], json!(backup[19].id));
    let batch = store.changes_since(None, 0, usize::MAX).unwrap();
    assert!(batch.reset && batch.head > floor_before && batch.upserts.len() == 20, "a phone gets the new history whole");
}

/// One transaction: an entry the database refuses halfway leaves every entry, the count and the
/// revisions as they were.
#[test]
fn a_restore_that_fails_halfway_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(dir.path());
    for e in takes(3).into_iter().rev() {
        store.push(e, MAX_ENTRIES).unwrap();
    }
    let before = stored(dir.path());
    let head = store.head();
    let conn = rusqlite::Connection::open(dir.path().join(HISTORY_DB_FILE_NAME)).unwrap();
    conn.execute_batch("CREATE TRIGGER refuse BEFORE INSERT ON entries WHEN NEW.at_ms = 42 BEGIN SELECT RAISE(ABORT, 'refused'); END;").unwrap();
    let mut backup = takes(10);
    backup[5].at_ms = 42;
    for replace in [false, true] {
        assert!(store.restore(&backup, replace, MAX_ENTRIES).is_err());
        assert_eq!(stored(dir.path()), before);
        assert_eq!((store.total(), store.head()), (3, head));
    }
    // Without a database every restore is refused.
    let blocked = dir.path().join("blocked");
    std::fs::write(&blocked, b"x").unwrap();
    let mut none = HistoryStore::open(&blocked);
    assert!(!none.is_open() && none.restore(&backup, false, MAX_ENTRIES).is_err());
    assert!(store.is_open());
}

/// docs/dictation.md §4.6: 20 000 entries — the most the history keeps — are written in one go,
/// streamed into the file, read back and restored, with the statistics unchanged. The bounds only
/// catch a gross slowdown; the numbers go to the test's output.
#[test]
fn twenty_thousand_entries_export_and_import_in_time() {
    let from = tempfile::tempdir().unwrap();
    let entries = takes(MAX_ENTRIES);
    let started = Instant::now();
    let mut store = HistoryStore::open(from.path());
    store.restore(&entries, true, MAX_ENTRIES).unwrap();
    let filled = started.elapsed();
    let path = from.path().join(file_name("Voltip 备份"));
    let started = Instant::now();
    let summary = export(&path, &from.path().join(HISTORY_DB_FILE_NAME), &input(&[], &[], &[], &[])).unwrap();
    let exported = started.elapsed();
    let started = Instant::now();
    let backup = read(&path).unwrap();
    let parsed = started.elapsed();
    let to = tempfile::tempdir().unwrap();
    let mut target = HistoryStore::open(to.path());
    target.push(take(99_999, AT - 1), MAX_ENTRIES).unwrap();
    let started = Instant::now();
    let done = target.restore(&backup.history, false, MAX_ENTRIES).unwrap();
    let imported = started.elapsed();
    eprintln!(
        "20 000 entries: {} bytes ({:.1} MiB); first write {filled:?}, export {exported:?}, read {parsed:?}, import {imported:?}",
        summary.bytes,
        summary.bytes as f64 / 1_048_576.0
    );
    assert_eq!(summary.counts.history, MAX_ENTRIES as u32);
    assert_eq!(done, crate::history::Restored { written: MAX_ENTRIES, present: 0, left_out: 0, removed: 1 }, "the one entry here was the oldest");
    assert_eq!(stats(to.path()), stats(from.path()));
    assert!(summary.bytes < 64 * 1_048_576, "{} bytes", summary.bytes);
    for (step, took) in [("export", exported), ("read", parsed), ("import", imported)] {
        assert!(took < Duration::from_secs(60), "{step} took {took:?}");
    }
}
