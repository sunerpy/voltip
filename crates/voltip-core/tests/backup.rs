#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Backup and migration (docs/dictation.md §4.6) through the real core task: one computer's
//! history, dictionary, rules, scenes and presets go into a backup file; another computer's core
//! restores them, merged with its own and in their place. Each list's event and the history event
//! follow the import, the statistics count the restored dictations, `Settings.history.keep` holds,
//! and everything is still there after a restart.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use voltip_core::backup::auto::{AUTO_BACKUP_DIR, AutoKind};
use voltip_core::backup::{self, BackupPart, BackupParts, ExportInput, ImportFailure, ImportOutcome, ImportReply, ImportReport, PartReport, Skipped};
use voltip_core::dictation::fakes::history_entry;
use voltip_core::history::{HistoryQuery, HistoryReader};
use voltip_core::presets::PresetStore;
use voltip_core::scenes::SceneStore;
use voltip_core::vocabulary::{DictionaryStore, RuleStore};
use voltip_core::{
    AppCore, CoreCommand, CoreConfig, CoreEvent, CoreHandle, CustomPreset, DictionaryDraft, DictionaryEntry, EntrySource, HistoryEntry, HistorySettings,
    HistoryStore, ImportMode, PresetDraft, ReplacementRule, RuleDraft, RuleKind, Scene, SceneDraft, SceneMatch, SceneOverrides, Settings, SettingsStore,
};
use voltip_identity::MemorySecretStore;
use voltip_protocol::Platform;

const STEP: Duration = Duration::from_secs(20);
const AT: u64 = 1_760_000_000_000;

struct Node {
    handle: CoreHandle,
    events: mpsc::Receiver<CoreEvent>,
}

fn start(dir: &std::path::Path, keep: u32) -> Node {
    start_with(dir, Settings { history: HistorySettings { enabled: true, keep }, ..Settings::default() })
}

fn start_with(dir: &std::path::Path, settings: Settings) -> Node {
    let settings = Settings { relay_enabled: false, engines: voltip_core::dictation::fakes::fake_engines(), ..settings };
    SettingsStore::new(dir).save(&settings).unwrap();
    let mut cfg = CoreConfig::new(dir.to_path_buf());
    cfg.default_device_name = "Backup Test".into();
    let (handle, events) = AppCore::start(cfg, Arc::new(MemorySecretStore::new())).unwrap();
    Node { handle, events }
}

async fn wait<T>(node: &mut Node, mut pick: impl FnMut(&CoreEvent) -> Option<T>) -> T {
    loop {
        let ev = tokio::time::timeout(STEP, node.events.recv()).await.expect("event in time").expect("core alive");
        if let Some(v) = pick(&ev) {
            return v;
        }
    }
}

/// Stop the core and wait until its task has let go of the stores.
async fn stop(mut node: Node) {
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
    tokio::time::timeout(STEP, async { while node.events.recv().await.is_some() {} }).await.expect("the core stops");
}

async fn import(node: &mut Node, backup: &Arc<backup::Backup>, parts: BackupParts, mode: ImportMode) -> ImportReport {
    match import_outcome(node, backup, parts, mode).await {
        ImportOutcome::Done { report } => report,
        other => panic!("not imported: {other:?}"),
    }
}

async fn import_outcome(node: &mut Node, backup: &Arc<backup::Backup>, parts: BackupParts, mode: ImportMode) -> ImportOutcome {
    let (reply, answer) = ImportReply::channel();
    node.handle.send(CoreCommand::BackupImport { backup: backup.clone(), parts, mode, reply }).await.unwrap();
    tokio::time::timeout(STEP, answer).await.expect("an answer in time").expect("the core answers")
}

fn part(report: &ImportReport, part: BackupPart) -> &PartReport {
    report.parts.iter().find(|p| p.part == part).unwrap()
}

fn dictation(i: usize, minute: u64) -> HistoryEntry {
    HistoryEntry {
        at_ms: AT + minute * 60_000,
        raw_text: format!("第{i}句 嗯 原文"),
        text: format!("第{i}句，原文。"),
        duration_ms: 2_000,
        ..history_entry("x")
    }
}

fn scene_draft(name: &str, app: &str) -> SceneDraft {
    SceneDraft {
        name: name.into(),
        enabled: true,
        matching: SceneMatch { apps: vec![app.into()], title_contains: Vec::new() },
        overrides: SceneOverrides::default(),
    }
}

fn rule_draft(name: &str, pattern: &str) -> RuleDraft {
    RuleDraft { name: name.into(), kind: RuleKind::Literal, pattern: pattern.into(), replacement: "X".into(), case_sensitive: true, enabled: true }
}

/// What the old computer holds, as its stores keep it.
struct OldComputer {
    entries: Vec<HistoryEntry>,
    dictionary: Vec<DictionaryEntry>,
    rules: Vec<ReplacementRule>,
    scenes: Vec<Scene>,
    presets: Vec<CustomPreset>,
}

/// The old computer: 12 dictations, two dictionary entries, two rules, a scene of the user's
/// besides the built-in ones, two presets. Its backup file.
fn old_computer(dir: &std::path::Path, file: &std::path::Path) -> OldComputer {
    let mut history = HistoryStore::open(dir);
    for i in 0..12 {
        history.push(dictation(i, i as u64 * 2), 20_000).unwrap();
    }
    let entries = history.recent(100);
    drop(history);
    let (mut dictionary, _) = DictionaryStore::open(dir);
    dictionary.add(&DictionaryDraft { term: "Voltip".into(), heard_as: vec!["沃提普".into()], enabled: true }, EntrySource::Manual, AT).unwrap();
    dictionary.add(&DictionaryDraft { term: "Paseo".into(), heard_as: vec!["怕手".into()], enabled: true }, EntrySource::Manual, AT).unwrap();
    let (mut rules, _) = RuleStore::open(dir);
    rules.add(&rule_draft("句号", "句号"), AT).unwrap();
    rules.add(&rule_draft("逗号", "逗号"), AT).unwrap();
    let (mut presets, _) = PresetStore::open(dir);
    let weekly = presets.add(&PresetDraft { name: "周报".into(), prompt: "整理成周报".into() }, AT).unwrap();
    presets.add(&PresetDraft { name: "纪要".into(), prompt: "整理成纪要".into() }, AT).unwrap();
    let (mut scenes, _) = SceneStore::open_on(dir, Platform::current(), AT);
    let mut outlook = scene_draft("Outlook", "outlook");
    outlook.overrides.refine_preset = Some(voltip_core::PresetId::Custom(weekly));
    scenes.add(&outlook, AT).unwrap();
    let lists = (dictionary.entries().to_vec(), rules.rules().to_vec(), scenes.scenes().to_vec(), presets.presets().to_vec());
    let input = ExportInput {
        app_version: "0.1.4",
        device: "旧电脑",
        platform: None,
        created_at_ms: AT,
        dictionary: &lists.0,
        rules: &lists.1,
        scenes: &lists.2,
        presets: &lists.3,
    };
    let summary = backup::export(file, &dir.join(voltip_core::history::HISTORY_DB_FILE_NAME), &input).unwrap();
    assert_eq!(summary.counts.history, 12);
    OldComputer { entries, dictionary: lists.0, rules: lists.1, scenes: lists.2, presets: lists.3 }
}

#[tokio::test]
async fn a_backup_moves_to_another_computer_merged_or_in_place() {
    let old = tempfile::tempdir().unwrap();
    let file = old.path().join("Voltip 备份.voltip-backup");
    let OldComputer { entries, dictionary, rules, scenes, presets } = old_computer(old.path(), &file);
    let backup = Arc::new(backup::read(&file).unwrap());

    // The new computer has a dictation of its own, the term Voltip and a preset named 周报. It
    // keeps 10 entries.
    let new = tempfile::tempdir().unwrap();
    let own = dictation(99, 100);
    HistoryStore::open(new.path()).push(own.clone(), 20_000).unwrap();
    DictionaryStore::open(new.path()).0.add(&DictionaryDraft { term: "voltip".into(), heard_as: Vec::new(), enabled: true }, EntrySource::Manual, AT).unwrap();
    let mine = PresetStore::open(new.path()).0.add(&PresetDraft { name: "周报".into(), prompt: "我的周报".into() }, AT).unwrap();
    let mut node = start(new.path(), 10);
    wait(&mut node, |e| matches!(e, CoreEvent::Presets(_)).then_some(())).await;

    let report = import(&mut node, &backup, BackupParts::ALL, ImportMode::Merge).await;
    assert_eq!(report.mode, ImportMode::Merge);
    assert_eq!(
        report.parts.iter().map(|p| p.part).collect::<Vec<_>>(),
        [BackupPart::Presets, BackupPart::Scenes, BackupPart::Dictionary, BackupPart::Rules, BackupPart::History]
    );
    let history = part(&report, BackupPart::History);
    // 12 + 1 entries, 10 kept: the own one is the newest; the two oldest of the backup stay out.
    assert_eq!((history.in_file, history.restored, history.removed, history.skipped), (12, 9, 0, Skipped { older: 3, ..Skipped::default() }));
    assert_eq!((part(&report, BackupPart::Dictionary).restored, part(&report, BackupPart::Dictionary).skipped.duplicate), (1, 1));
    assert_eq!(part(&report, BackupPart::Presets).restored, 1);
    assert_eq!(part(&report, BackupPart::Rules).restored, 2);
    let scene_part = part(&report, BackupPart::Scenes);
    assert_eq!((scene_part.restored, scene_part.skipped.duplicate), (1, scenes.len() as u32 - 1), "the built-in scenes here stay");
    assert!(report.parts.iter().all(|p| p.error.is_none()));

    // Every list's event and the history's, in the order the parts ran.
    let presets_here = wait(&mut node, |e| if let CoreEvent::Presets(p) = e { Some(p.clone()) } else { None }).await;
    assert_eq!(presets_here.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["周报", "纪要"]);
    assert_eq!(presets_here[0].id, mine, "the preset here stays");
    let scenes_here = wait(&mut node, |e| if let CoreEvent::Scenes(s) = e { Some(s.clone()) } else { None }).await;
    let outlook = scenes_here.iter().find(|s| s.name == "Outlook").unwrap();
    assert_eq!(outlook.overrides.refine_preset, Some(voltip_core::PresetId::Custom(mine)), "names the 周报 here");
    let dictionary_here = wait(&mut node, |e| if let CoreEvent::Dictionary(d) = e { Some(d.clone()) } else { None }).await;
    assert_eq!(dictionary_here.iter().map(|e| e.term.as_str()).collect::<Vec<_>>(), ["voltip", "Paseo"]);
    let rules_here = wait(&mut node, |e| if let CoreEvent::Rules(r) = e { Some(r.clone()) } else { None }).await;
    assert_eq!(rules_here, rules, "every rule as it was, ids included");
    let total = wait(&mut node, |e| if let CoreEvent::History { total, .. } = e { Some(*total) } else { None }).await;
    assert_eq!(total, 10);

    // The statistics count what was restored: ten dictations of two seconds.
    let reader = HistoryReader::new(new.path());
    let stats = reader.stats(&[AT, AT + 86_400_000]).unwrap();
    assert_eq!((stats.total.count, stats.total.spoken_ms), (10, 20_000));
    let page = reader.query(&HistoryQuery { limit: 200, ..HistoryQuery::default() }).unwrap();
    assert_eq!(page.entries[0], own);
    assert_eq!(page.entries[1..], entries[..9], "the newest of the backup, as they were");

    // Again: everything is here; nothing changes.
    let again = import(&mut node, &backup, BackupParts::ALL, ImportMode::Merge).await;
    assert!(again.parts.iter().all(|p| p.restored == 0), "{again:?}");

    // Replace the dictionary and the presets only: they become the backup's.
    let only = BackupParts { dictionary: true, presets: true, ..BackupParts::default() };
    let replaced = import(&mut node, &backup, only, ImportMode::Replace).await;
    assert_eq!(replaced.parts.len(), 2);
    let presets_now = wait(&mut node, |e| if let CoreEvent::Presets(p) = e { Some(p.clone()) } else { None }).await;
    assert_eq!(presets_now, presets);
    assert_eq!(wait(&mut node, |e| if let CoreEvent::Dictionary(d) = e { Some(d.clone()) } else { None }).await, dictionary);

    // A restart reads all of it back.
    stop(node).await;
    let mut node = start(new.path(), 10);
    let dictionary_after = wait(&mut node, |e| if let CoreEvent::Dictionary(d) = e { Some(d.clone()) } else { None }).await;
    assert_eq!(dictionary_after, dictionary);
    assert_eq!(wait(&mut node, |e| if let CoreEvent::Rules(r) = e { Some(r.clone()) } else { None }).await, rules_here);
    let scenes_after = wait(&mut node, |e| if let CoreEvent::Scenes(s) = e { Some(s.clone()) } else { None }).await;
    assert_eq!(scenes_after, scenes_here);
    assert_eq!(HistoryReader::new(new.path()).query(&HistoryQuery { limit: 200, ..HistoryQuery::default() }).unwrap().total, 10);

    // Replace the history: the backup's newest ten, the own entry gone.
    let replaced = import(&mut node, &backup, BackupParts { history: true, ..BackupParts::default() }, ImportMode::Replace).await;
    let history = part(&replaced, BackupPart::History);
    assert_eq!((history.restored, history.removed, history.skipped.older), (10, 10, 2));
    wait(&mut node, |e| matches!(e, CoreEvent::History { total: 10, .. }).then_some(())).await;
    let page = HistoryReader::new(new.path()).query(&HistoryQuery { limit: 200, ..HistoryQuery::default() }).unwrap();
    assert_eq!(page.entries, entries[..10]);
    stop(node).await;
}

/// A history that cannot be opened is reported for its part; the other parts are restored.
#[tokio::test]
async fn a_part_that_cannot_be_written_is_reported_and_the_rest_restored() {
    let old = tempfile::tempdir().unwrap();
    let file = old.path().join("b.voltip-backup");
    old_computer(old.path(), &file);
    let backup = Arc::new(backup::read(&file).unwrap());
    let new = tempfile::tempdir().unwrap();
    // A directory where the database should be: the core starts without a history.
    std::fs::create_dir(new.path().join(voltip_core::history::HISTORY_DB_FILE_NAME)).unwrap();
    let mut node = start(new.path(), 20_000);
    wait(&mut node, |e| matches!(e, CoreEvent::Presets(_)).then_some(())).await;
    let report = import(&mut node, &backup, BackupParts { history: true, rules: true, ..BackupParts::default() }, ImportMode::Merge).await;
    assert_eq!(part(&report, BackupPart::History).error, Some(backup::PartFailure::Unavailable));
    assert_eq!(part(&report, BackupPart::History).restored, 0);
    assert_eq!(part(&report, BackupPart::Rules).restored, 2);
    assert_eq!(wait(&mut node, |e| if let CoreEvent::Rules(r) = e { Some(r.len()) } else { None }).await, 2);
    stop(node).await;
}

/// The automatic backups in `dir`'s default folder (docs/dictation.md §4.6.1).
fn auto_backups(dir: &std::path::Path) -> Vec<backup::auto::AutoFile> {
    backup::auto::list(&dir.join(AUTO_BACKUP_DIR), None).unwrap()
}

/// What `dir`'s history holds, newest first.
fn history_here(dir: &std::path::Path) -> Vec<HistoryEntry> {
    HistoryReader::new(dir).query(&HistoryQuery { limit: 200, ..HistoryQuery::default() }).unwrap().entries
}

/// Regression (原计划核对 R12, user decision 2026-10-28): a replacing import deleted what was here
/// with no copy of it anywhere. Now it first writes an automatic backup of everything here, names
/// it in the report, and that backup brings the data back.
#[tokio::test]
async fn regression_a_replace_import_keeps_a_snapshot_first() {
    let old = tempfile::tempdir().unwrap();
    let file = old.path().join("b.voltip-backup");
    let OldComputer { entries, .. } = old_computer(old.path(), &file);
    let backup = Arc::new(backup::read(&file).unwrap());
    let new = tempfile::tempdir().unwrap();
    let own = dictation(99, 100);
    HistoryStore::open(new.path()).push(own.clone(), 20_000).unwrap();
    DictionaryStore::open(new.path())
        .0
        .add(&DictionaryDraft { term: "Paseo".into(), heard_as: vec!["帕索".into()], enabled: true }, EntrySource::Manual, AT)
        .unwrap();
    let mut node = start(new.path(), 20_000);
    let dictionary_before = wait(&mut node, |e| if let CoreEvent::Dictionary(d) = e { Some(d.clone()) } else { None }).await;

    let report = import(&mut node, &backup, BackupParts::ALL, ImportMode::Replace).await;
    let snapshot = report.snapshot.clone().expect("the report names the backup made first");
    let made = auto_backups(new.path());
    assert_eq!(made.len(), 1);
    assert_eq!((made[0].kind, made[0].path.display().to_string()), (AutoKind::Import, snapshot.clone()));
    let kept = backup::read(std::path::Path::new(&snapshot)).unwrap();
    assert_eq!(kept.history, vec![own.clone()], "the history as it was before the import");
    assert_eq!(kept.dictionary, dictionary_before);
    assert_eq!(history_here(new.path()), entries, "and the import replaced it");

    // That backup restores what was here.
    let kept = Arc::new(kept);
    import(&mut node, &kept, BackupParts::ALL, ImportMode::Replace).await;
    assert_eq!(history_here(new.path()), vec![own]);
    let dictionary_after = loop {
        let d = wait(&mut node, |e| if let CoreEvent::Dictionary(d) = e { Some(d.clone()) } else { None }).await;
        if d == dictionary_before {
            break d;
        }
    };
    assert_eq!(dictionary_after, dictionary_before);
    assert_eq!(auto_backups(new.path()).len(), 2, "the second replace made its own backup");
    stop(node).await;
}

/// Regression (plan PR-6, the bridge's cache follows the core's events a step behind): a write
/// queued just before a replacing import — not waited for — is in the backup made first, because
/// the core makes it of its own lists, after every command queued before.
#[tokio::test]
async fn regression_a_snapshot_includes_a_write_dispatched_just_before() {
    let old = tempfile::tempdir().unwrap();
    let file = old.path().join("b.voltip-backup");
    old_computer(old.path(), &file);
    let backup = Arc::new(backup::read(&file).unwrap());
    let new = tempfile::tempdir().unwrap();
    let mut node = start(new.path(), 20_000);
    node.handle
        .send(CoreCommand::DictionaryAdd {
            draft: DictionaryDraft { term: "Kiro".into(), heard_as: vec!["基罗".into()], enabled: true },
            source: EntrySource::Manual,
        })
        .await
        .unwrap();
    let report = import(&mut node, &backup, BackupParts { dictionary: true, ..BackupParts::default() }, ImportMode::Replace).await;
    let kept = backup::read(std::path::Path::new(&report.snapshot.unwrap())).unwrap();
    assert_eq!(kept.dictionary.iter().map(|d| d.term.as_str()).collect::<Vec<_>>(), ["Kiro"]);
    stop(node).await;
}

/// A take that just ended is in the backup made before a replacing import: its entry was written
/// on the core task before the import's command came.
#[tokio::test]
async fn a_take_that_just_ended_is_in_the_backup_before_a_replacing_import() {
    let old = tempfile::tempdir().unwrap();
    let file = old.path().join("b.voltip-backup");
    old_computer(old.path(), &file);
    let backup = Arc::new(backup::read(&file).unwrap());
    let new = tempfile::tempdir().unwrap();
    let mut node = start(new.path(), 20_000);
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    wait(&mut node, |e| matches!(e, CoreEvent::Dictation(s) if matches!(s.phase, voltip_core::DictationPhase::Listening { .. })).then_some(())).await;
    node.handle.send(CoreCommand::DictationStop).await.unwrap();
    let recent = wait(&mut node, |e| if let CoreEvent::History { recent, total: 1 } = e { Some(recent.clone()) } else { None }).await;
    let report = import(&mut node, &backup, BackupParts { history: true, ..BackupParts::default() }, ImportMode::Replace).await;
    let kept = backup::read(std::path::Path::new(&report.snapshot.unwrap())).unwrap();
    assert_eq!(kept.history, recent);
    stop(node).await;
}

/// docs/dictation.md §4.6.1: a replacing import while a take runs is refused — the take would be
/// recorded into the history being replaced — and nothing changes: no backup, no part replaced.
#[tokio::test]
async fn a_replacing_import_during_a_take_is_refused_and_changes_nothing() {
    let old = tempfile::tempdir().unwrap();
    let file = old.path().join("b.voltip-backup");
    old_computer(old.path(), &file);
    let backup = Arc::new(backup::read(&file).unwrap());
    let new = tempfile::tempdir().unwrap();
    let own = dictation(99, 100);
    HistoryStore::open(new.path()).push(own.clone(), 20_000).unwrap();
    let mut node = start(new.path(), 20_000);
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    wait(&mut node, |e| matches!(e, CoreEvent::Dictation(s) if matches!(s.phase, voltip_core::DictationPhase::Listening { .. })).then_some(())).await;
    let outcome = import_outcome(&mut node, &backup, BackupParts::ALL, ImportMode::Replace).await;
    assert_eq!(outcome, ImportOutcome::failed(ImportFailure::Busy));
    assert_eq!(auto_backups(new.path()), Vec::new());
    assert_eq!(history_here(new.path()), vec![own]);
    // A merge goes ahead during a take: it deletes nothing.
    node.handle.send(CoreCommand::DictationCancel).await.unwrap();
    stop(node).await;
}

/// docs/dictation.md §4.6.1: when the backup before a replacing import cannot be written, the
/// import is refused with the reason, and the history and the dictionary stay as they were.
#[tokio::test]
async fn a_replacing_import_whose_backup_fails_replaces_nothing() {
    let old = tempfile::tempdir().unwrap();
    let file = old.path().join("b.voltip-backup");
    old_computer(old.path(), &file);
    let backup = Arc::new(backup::read(&file).unwrap());
    let new = tempfile::tempdir().unwrap();
    let own = dictation(99, 100);
    HistoryStore::open(new.path()).push(own.clone(), 20_000).unwrap();
    DictionaryStore::open(new.path()).0.add(&DictionaryDraft { term: "Paseo".into(), heard_as: Vec::new(), enabled: true }, EntrySource::Manual, AT).unwrap();
    let blocker = new.path().join("a file");
    std::fs::write(&blocker, b"x").unwrap();
    let settings =
        Settings { backup: voltip_core::BackupSettings { weekly: true, folder: Some(blocker.join("backups").display().to_string()) }, ..Settings::default() };
    let mut node = start_with(new.path(), settings);
    let dictionary_before = wait(&mut node, |e| if let CoreEvent::Dictionary(d) = e { Some(d.clone()) } else { None }).await;
    let outcome = import_outcome(&mut node, &backup, BackupParts::ALL, ImportMode::Replace).await;
    let ImportOutcome::Failed { code: ImportFailure::SnapshotFailed, detail } = outcome else { panic!("refused: {outcome:?}") };
    assert!(!detail.is_empty());
    assert_eq!(history_here(new.path()), vec![own]);
    stop(node).await;
    assert_eq!(DictionaryStore::open(new.path()).0.entries(), dictionary_before.as_slice());
}

/// A merge deletes nothing here: it makes no backup first.
#[tokio::test]
async fn a_merging_import_makes_no_backup() {
    let old = tempfile::tempdir().unwrap();
    let file = old.path().join("b.voltip-backup");
    old_computer(old.path(), &file);
    let backup = Arc::new(backup::read(&file).unwrap());
    let new = tempfile::tempdir().unwrap();
    let mut node = start(new.path(), 20_000);
    let report = import(&mut node, &backup, BackupParts::ALL, ImportMode::Merge).await;
    assert_eq!(report.snapshot, None);
    assert_eq!(auto_backups(new.path()), Vec::new());
    stop(node).await;
}
