#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The personal dictionary and the replacement rules (docs/dictation.md §16) through the real core
//! task: commands in, `Dictionary` / `Rules` / `Error` events out, both files on disk, a restart
//! that reads them back, a quarantined file, and one dictation run on the fakes that the vocabulary
//! really changes.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use voltip_core::dictation::fakes::{FAKE_TRANSCRIPT, FakeAudio, FakeInjector, FakeTranscriber};
use voltip_core::dictation::{DictationPorts, Transcriber};
use voltip_core::vocabulary::{DICTIONARY_FILE_NAME, RULES_FILE_NAME, export_rules_toml, parse_rules_toml};
use voltip_core::{
    AppCore, CoreCommand, CoreConfig, CoreEvent, CoreHandle, DictationPhase, DictionaryDraft, DictionaryEntry, EntrySource, ImportMode, ReplacementRule,
    RuleDraft, RuleKind, Settings, SettingsStore, VocabularyHit, VocabularyHits,
};
use voltip_identity::MemorySecretStore;

const STEP: Duration = Duration::from_secs(10);

struct Node {
    handle: CoreHandle,
    events: mpsc::Receiver<CoreEvent>,
}

fn start(dir: &std::path::Path, injector: Arc<FakeInjector>) -> Node {
    SettingsStore::new(dir).save(&Settings { relay_enabled: false, engines: voltip_core::dictation::fakes::fake_engines(), ..Settings::default() }).unwrap();
    let mut cfg = CoreConfig::new(dir.to_path_buf());
    cfg.default_device_name = "Vocabulary Test".into();
    let ports = DictationPorts {
        audio: Arc::new(FakeAudio::speech()),
        injector,
        // `你好，世界` is what the fake recogniser always says.
        factory: Arc::new(|_| (Arc::new(FakeTranscriber::ok(FAKE_TRANSCRIPT)) as Arc<dyn Transcriber>, None)),
        models: None,
        streaming: None,
        probe: None,
        service_probe: None,
        segmenter: None,
    };
    let (handle, events) = AppCore::start_with(cfg, Arc::new(MemorySecretStore::new()), ports).unwrap();
    Node { handle, events }
}

async fn wait<T>(node: &mut Node, mut pick: impl FnMut(&CoreEvent) -> Option<T>) -> T {
    loop {
        let ev = tokio::time::timeout(STEP, node.events.recv()).await.expect("event within 10 s").expect("core alive");
        if let Some(v) = pick(&ev) {
            return v;
        }
    }
}

async fn dictionary(node: &mut Node) -> Vec<DictionaryEntry> {
    wait(node, |e| if let CoreEvent::Dictionary(d) = e { Some(d.clone()) } else { None }).await
}

async fn rules(node: &mut Node) -> Vec<ReplacementRule> {
    wait(node, |e| if let CoreEvent::Rules(r) = e { Some(r.clone()) } else { None }).await
}

async fn error(node: &mut Node) -> String {
    wait(node, |e| if let CoreEvent::Error(m) = e { Some(m.clone()) } else { None }).await
}

fn draft(term: &str, heard: &[&str]) -> DictionaryDraft {
    DictionaryDraft { term: term.into(), heard_as: heard.iter().map(|h| (*h).to_owned()).collect(), enabled: true }
}

fn rule(name: &str, kind: RuleKind, pattern: &str, replacement: &str) -> RuleDraft {
    RuleDraft { name: name.into(), kind, pattern: pattern.into(), replacement: replacement.into(), case_sensitive: true, enabled: true }
}

/// Every command, the events after each, the refusals (as `error` events, the list unchanged),
/// persistence across a restart, and a real take the vocabulary changes, recorded in the history.
#[tokio::test]
async fn dictionary_and_rules_through_the_core() {
    let dir = tempfile::tempdir().unwrap();
    let injector = Arc::new(FakeInjector::paste());
    let mut node = start(dir.path(), injector.clone());
    // Ready announces both (empty) lists.
    assert!(dictionary(&mut node).await.is_empty());
    assert!(rules(&mut node).await.is_empty());

    // Dictionary: add (manual + from history), update, reorder, remove, refusals.
    node.handle.send(CoreCommand::DictionaryAdd { draft: draft("世界", &["世 界"]), source: EntrySource::Manual }).await.unwrap();
    let list = dictionary(&mut node).await;
    assert_eq!((list.len(), list[0].term.as_str()), (1, "世界"));
    let history_id = uuid::Uuid::new_v4();
    node.handle.send(CoreCommand::DictionaryAdd { draft: draft("World", &["世界"]), source: EntrySource::History { history_id } }).await.unwrap();
    let message = error(&mut node).await;
    assert!(message.starts_with("dictionary: ") && message.contains("是词条「世界」的正确写法"), "{message}");
    node.handle.send(CoreCommand::DictionaryAdd { draft: draft("Planet", &["世界"]), source: EntrySource::History { history_id } }).await.unwrap();
    assert!(error(&mut node).await.contains("正确写法"), "the conflict is refused again");
    node.handle.send(CoreCommand::DictionaryUpdate { id: list[0].id, draft: draft("World", &["世界"]) }).await.unwrap();
    let list = dictionary(&mut node).await;
    assert_eq!((list[0].term.as_str(), list[0].heard_as.clone()), ("World", vec!["世界".to_owned()]));
    node.handle.send(CoreCommand::DictionaryAdd { draft: draft("Voltip", &["沃提普"]), source: EntrySource::History { history_id } }).await.unwrap();
    let list = dictionary(&mut node).await;
    assert_eq!(list[1].source, EntrySource::History { history_id });
    node.handle.send(CoreCommand::DictionaryReorder(vec![list[1].id, list[0].id])).await.unwrap();
    assert_eq!(dictionary(&mut node).await[0].term, "Voltip");
    node.handle.send(CoreCommand::DictionaryReorder(vec![list[1].id])).await.unwrap();
    assert!(error(&mut node).await.contains("全部词条"));
    node.handle.send(CoreCommand::DictionaryRemove(uuid::Uuid::new_v4())).await.unwrap();
    assert!(error(&mut node).await.contains("没有 id"));

    // Rules: add, update, import (merge), reorder, remove, refusals.
    node.handle.send(CoreCommand::RuleAdd(rule("hello", RuleKind::Literal, "你好", "您好"))).await.unwrap();
    let list = rules(&mut node).await;
    assert_eq!(list.len(), 1);
    node.handle.send(CoreCommand::RuleAdd(rule("hello", RuleKind::Literal, "x", "y"))).await.unwrap();
    assert!(error(&mut node).await.contains("已有名为「hello」"));
    node.handle.send(CoreCommand::RuleAdd(rule("bad", RuleKind::Regex, "(", ""))).await.unwrap();
    assert!(error(&mut node).await.contains("正则无法编译"), "the core validates again, whoever sends the command");
    let imported = parse_rules_toml("version = 1\n[[rule]]\nname = \"bang\"\nkind = \"regex\"\npattern = \"World$\"\nreplacement = \"World!\"\n").unwrap();
    node.handle.send(CoreCommand::RulesImport { rules: imported, mode: ImportMode::Merge }).await.unwrap();
    let list = rules(&mut node).await;
    assert_eq!(list.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["hello", "bang"]);
    node.handle.send(CoreCommand::RuleUpdate { id: list[0].id, draft: rule("hello", RuleKind::Literal, "你好", "你好呀") }).await.unwrap();
    assert_eq!(rules(&mut node).await[0].replacement, "你好呀");
    node.handle.send(CoreCommand::RuleReorder(vec![list[1].id, list[0].id])).await.unwrap();
    assert_eq!(rules(&mut node).await[0].name, "bang");
    node.handle.send(CoreCommand::RuleReorder(vec![list[1].id, list[0].id])).await.unwrap();
    let list = rules(&mut node).await;
    node.handle.send(CoreCommand::RuleRemove(uuid::Uuid::new_v4())).await.unwrap();
    assert!(error(&mut node).await.contains("没有 id"));

    // A take: `你好，世界` → dictionary → `你好，World` → rules in order (`bang` then `hello`).
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    wait(&mut node, |e| matches!(e, CoreEvent::Dictation(s) if matches!(s.phase, DictationPhase::Listening { .. })).then_some(())).await;
    node.handle.send(CoreCommand::DictationStop).await.unwrap();
    let done = wait(&mut node, |e| match e {
        CoreEvent::Dictation(s) if s.phase.is_terminal() => Some(s.phase.clone()),
        _ => None,
    })
    .await;
    assert!(matches!(&done, DictationPhase::Done { text, raw_text, .. } if text == "你好呀，World!" && raw_text == FAKE_TRANSCRIPT), "{done:?}");
    assert_eq!(injector.injected(), vec!["你好呀，World!".to_owned()]);
    let history = wait(&mut node, |e| if let CoreEvent::History { recent: h, .. } = e { (!h.is_empty()).then(|| h.clone()) } else { None }).await;
    let world = dictionary_file(dir.path()).into_iter().find(|e| e.term == "World").unwrap();
    assert_eq!(
        history[0].vocabulary,
        Some(VocabularyHits {
            corrections: vec![VocabularyHit { id: world.id, count: 1 }],
            rules: vec![VocabularyHit { id: list[0].id, count: 1 }, VocabularyHit { id: list[1].id, count: 1 }],
        })
    );

    // Everything is on disk; a restarted core reads the same lists back (and the export matches).
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
    drop(node);
    let mut node = start(dir.path(), Arc::new(FakeInjector::paste()));
    let reloaded = dictionary(&mut node).await;
    assert_eq!(reloaded.iter().map(|e| e.term.as_str()).collect::<Vec<_>>(), ["Voltip", "World"]);
    let reloaded_rules = rules(&mut node).await;
    assert_eq!(reloaded_rules, list);
    assert_eq!(parse_rules_toml(&export_rules_toml(&reloaded_rules).unwrap()).unwrap().len(), 2);
    node.handle.send(CoreCommand::RulesImport { rules: Vec::new(), mode: ImportMode::Replace }).await.unwrap();
    assert!(rules(&mut node).await.is_empty());
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

fn dictionary_file(dir: &std::path::Path) -> Vec<DictionaryEntry> {
    #[derive(serde::Deserialize)]
    struct File {
        entries: Vec<DictionaryEntry>,
    }
    serde_json::from_slice::<File>(&std::fs::read(dir.join(DICTIONARY_FILE_NAME)).unwrap()).unwrap().entries
}

/// A corrupt `rules.json` / `dictionary.json` does not stop the core: it starts with empty lists,
/// the files are kept as `<file>.corrupt-<secs>`, and the UI hears about each after `Ready`.
#[tokio::test]
async fn regression_corrupt_vocabulary_files_are_set_aside_and_reported_after_ready() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(RULES_FILE_NAME), b"{ definitely not json").unwrap();
    std::fs::write(dir.path().join(DICTIONARY_FILE_NAME), br#"{"schema":7,"entries":[]}"#).unwrap();
    let mut node = start(dir.path(), Arc::new(FakeInjector::paste()));
    wait(&mut node, |e| matches!(e, CoreEvent::Ready { .. }).then_some(())).await;
    assert!(dictionary(&mut node).await.is_empty());
    assert!(rules(&mut node).await.is_empty());
    let first = error(&mut node).await;
    let second = error(&mut node).await;
    assert!(first.contains("dictionary.json 无法使用") && first.contains("schema 不是 1"), "{first}");
    assert!(second.contains("rules.json 无法使用") && second.contains(".corrupt-"), "{second}");
    let kept: Vec<String> =
        std::fs::read_dir(dir.path()).unwrap().filter_map(|e| e.ok()?.file_name().into_string().ok()).filter(|n| n.contains(".corrupt-")).collect();
    assert_eq!(kept.len(), 2, "{kept:?}");
    assert!(!dir.path().join(RULES_FILE_NAME).exists());
    // The core works on: a new rule is saved to a fresh file.
    node.handle.send(CoreCommand::RuleAdd(rule("a", RuleKind::Literal, "a", "b"))).await.unwrap();
    assert_eq!(rules(&mut node).await.len(), 1);
    assert!(dir.path().join(RULES_FILE_NAME).exists());
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}
