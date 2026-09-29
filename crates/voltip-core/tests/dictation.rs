#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The dictation pipeline through the real core task: commands in, `CoreEvent`s out, fakes for
//! the microphone / ASR / refine / inject, a `MemorySecretStore` for the secrets and a temp dir
//! for `settings.json` + `history.sqlite3`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::sync::mpsc;
use voltip_core::dictation::fakes::{FAKE_TRANSCRIPT, FakeAudio, FakeInjector, FakeRefiner, FakeTranscriber};
use voltip_core::dictation::{DictationPorts, Refiner, Transcriber};
use voltip_core::{
    AppCore, CoreCommand, CoreConfig, CoreEvent, CoreHandle, DictationPhase, EngineSettings, EngineStatus, InjectMode, Outcome, ProviderId, ProviderSettings,
    RecordingSettings, RecordingSource, ResolvedEngines, SecretSource, ServiceKind, Settings, SettingsStore,
};
use voltip_identity::{MemorySecretStore, SecretStore as _};

const STEP: Duration = Duration::from_secs(10);

struct Node {
    handle: CoreHandle,
    events: mpsc::Receiver<CoreEvent>,
}

/// The test build has no built-in service, so recognition points at a custom endpoint (the fake
/// factory stands in for it) and the clean-up at Groq, which still needs its key.
fn remote_engines() -> EngineSettings {
    EngineSettings {
        asr_provider: ProviderId::Custom,
        llm_provider: ProviderId::Groq,
        providers: [(
            ProviderId::Custom,
            ProviderSettings { asr_url: Some("https://asr.example.test".into()), asr_model: Some("whisper".into()), ..Default::default() },
        )]
        .into(),
        ..EngineSettings::default()
    }
}

fn groq(status: &EngineStatus) -> voltip_core::ServiceStatus {
    status.providers.iter().find(|p| p.id == ProviderId::Groq).and_then(|p| p.llm.clone()).expect("groq offers clean-up")
}

fn config(dir: &std::path::Path) -> CoreConfig {
    SettingsStore::new(dir).save(&Settings { relay_enabled: false, engines: remote_engines(), ..Settings::default() }).unwrap();
    let mut cfg = CoreConfig::new(dir.to_path_buf());
    cfg.default_device_name = "Dictation Test".into();
    cfg.direct_enabled = false;
    cfg
}

fn ports(factory_calls: Arc<AtomicUsize>, injector: Arc<FakeInjector>) -> DictationPorts {
    DictationPorts {
        audio: Arc::new(FakeAudio::speech()),
        injector,
        factory: Arc::new(move |engines: &ResolvedEngines| {
            factory_calls.fetch_add(1, Ordering::SeqCst);
            // Refine only when the clean-up provider is ready, like the real shell does.
            let refiner: Option<Arc<dyn Refiner>> = engines.refine.as_ref().map(|_| Arc::new(FakeRefiner::ok("你好，世界。")) as Arc<dyn Refiner>);
            (Arc::new(FakeTranscriber::ok(FAKE_TRANSCRIPT)) as Arc<dyn Transcriber>, refiner)
        }),
        models: None,
        streaming: None,
        probe: None,
        service_probe: None,
        segmenter: None,
    }
}

fn start(dir: &std::path::Path, store: Arc<MemorySecretStore>, factory_calls: Arc<AtomicUsize>, injector: Arc<FakeInjector>) -> Node {
    let (handle, events) = AppCore::start_with(config(dir), store, ports(factory_calls, injector)).unwrap();
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

async fn wait_phase(node: &mut Node, mut pred: impl FnMut(&DictationPhase) -> bool) -> DictationPhase {
    wait(node, |e| match e {
        CoreEvent::Dictation(s) if pred(&s.phase) => Some(s.phase.clone()),
        _ => None,
    })
    .await
}

#[tokio::test]
async fn pipeline_history_secrets_and_engines_through_the_core() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(MemorySecretStore::new());
    let factory_calls = Arc::new(AtomicUsize::new(0));
    let injector = Arc::new(FakeInjector::paste());
    let mut node = start(dir.path(), store.clone(), factory_calls.clone(), injector.clone());

    // Ready is followed by the engines and the (empty) history.
    wait(&mut node, |e| matches!(e, CoreEvent::Ready { .. }).then_some(())).await;
    let engines = wait(&mut node, |e| if let CoreEvent::Engines(s) = e { Some(s.clone()) } else { None }).await;
    assert_eq!(groq(&engines).key.source, SecretSource::None);
    assert_eq!(engines.refine_issue, Some(voltip_core::EngineIssue::KeyMissing));
    assert!(engines.refine_enabled && engines.asr_ready);
    assert_eq!(engines.asr_host, "asr.example.test");
    let history = wait(&mut node, |e| if let CoreEvent::History { recent: h, .. } = e { Some(h.clone()) } else { None }).await;
    assert!(history.is_empty());
    assert_eq!(factory_calls.load(Ordering::SeqCst), 1);

    // Stop with nothing running is reported, not ignored.
    node.handle.send(CoreCommand::DictationStop).await.unwrap();
    let err = wait(&mut node, |e| if let CoreEvent::Error(m) = e { Some(m.clone()) } else { None }).await;
    assert!(err.contains("没有进行中的听写"), "{err}");

    // Start → Listening (levels flow), Stop → Processing → Done (raw text: no refine key yet).
    let mut levels = node.handle.levels();
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    assert!(matches!(wait_phase(&mut node, |p| matches!(p, DictationPhase::Listening { .. })).await, DictationPhase::Listening { .. }));
    // `Listening` is reported before the (blocking) device open finishes; the frames follow.
    tokio::time::timeout(STEP, levels.recv()).await.expect("level frame within 10 s").expect("the fake capture reported levels through the handle");
    node.handle.send(CoreCommand::DictationStop).await.unwrap();
    let done = wait_phase(&mut node, DictationPhase::is_terminal).await;
    match &done {
        DictationPhase::Done { text, refined, refine_error, .. } => {
            assert_eq!(text, FAKE_TRANSCRIPT);
            assert!(!refined);
            assert!(refine_error.as_deref().is_some_and(|e| e.contains("未配置")), "{refine_error:?}");
        }
        other => panic!("{other:?}"),
    }
    let history = wait(&mut node, |e| if let CoreEvent::History { recent: h, .. } = e { (!h.is_empty()).then(|| h.clone()) } else { None }).await;
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].outcome, Outcome::Inserted { via: voltip_core::dictation::Via::Paste });
    assert_eq!(injector.injected(), vec![FAKE_TRANSCRIPT.to_owned()]);
    let id = history[0].id;

    // Star / unstar / delete round-trip and re-emit the list each time.
    node.handle.send(CoreCommand::HistoryStar(id, true)).await.unwrap();
    let h = wait(&mut node, |e| if let CoreEvent::History { recent: h, .. } = e { Some(h.clone()) } else { None }).await;
    assert!(h[0].starred);

    // A provider key: stored, reported as set-by-user, never echoed, and the clients are rebuilt.
    let before = factory_calls.load(Ordering::SeqCst);
    node.handle.send(CoreCommand::SetProviderKey { provider: ProviderId::Groq, kind: ServiceKind::Llm, value: Some("gsk_super_secret".into()) }).await.unwrap();
    let engines = wait(&mut node, |e| if let CoreEvent::Engines(s) = e { Some(s.clone()) } else { None }).await;
    assert!(groq(&engines).key.set && engines.refine_ready);
    assert_eq!(groq(&engines).key.source, SecretSource::User);
    assert!(!serde_json::to_string(&engines).unwrap().contains("gsk_super_secret"));
    assert_eq!(factory_calls.load(Ordering::SeqCst), before + 1);
    assert_eq!(store.get("provider-key.groq").unwrap().unwrap().as_slice(), b"gsk_super_secret");
    // A provider without user keys refuses one.
    node.handle.send(CoreCommand::SetProviderKey { provider: ProviderId::Builtin, kind: ServiceKind::Asr, value: Some("x".into()) }).await.unwrap();
    let err = wait(&mut node, |e| if let CoreEvent::Error(m) = e { Some(m.clone()) } else { None }).await;
    assert!(err.contains("builtin"), "{err}");

    // Now the refiner exists: the next run is refined.
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    wait_phase(&mut node, |p| matches!(p, DictationPhase::Listening { .. })).await;
    node.handle.send(CoreCommand::DictationStop).await.unwrap();
    let done = wait_phase(&mut node, DictationPhase::is_terminal).await;
    assert!(matches!(&done, DictationPhase::Done { text, refined: true, .. } if text == "你好，世界。"), "{done:?}");
    let h = wait(&mut node, |e| if let CoreEvent::History { recent: h, .. } = e { (h.len() == 2).then(|| h.clone()) } else { None }).await;
    assert_eq!(h[0].text, "你好，世界。", "newest first");

    // Engine settings: a bad URL, a provider without the service or a thread count out of range is
    // refused; a good block persists and re-resolves.
    let custom_url = |url: &str| EngineSettings {
        providers: [(ProviderId::Custom, ProviderSettings { asr_url: Some(url.into()), ..Default::default() })].into(),
        ..remote_engines()
    };
    node.handle.send(CoreCommand::SetEngines(custom_url("ftp://nope"))).await.unwrap();
    let err = wait(&mut node, |e| if let CoreEvent::Error(m) = e { Some(m.clone()) } else { None }).await;
    assert!(err.contains("custom.asr_url"), "{err}");
    node.handle.send(CoreCommand::SetEngines(custom_url("http://not a url"))).await.unwrap();
    wait(&mut node, |e| if let CoreEvent::Error(m) = e { m.contains("custom.asr_url").then_some(()) } else { None }).await;
    node.handle.send(CoreCommand::SetEngines(EngineSettings { asr_provider: ProviderId::Deepseek, ..remote_engines() })).await.unwrap();
    wait(&mut node, |e| if let CoreEvent::Error(m) = e { m.contains("asr_provider").then_some(()) } else { None }).await;
    node.handle.send(CoreCommand::SetEngines(EngineSettings { llm_provider: ProviderId::Local, ..remote_engines() })).await.unwrap();
    wait(&mut node, |e| if let CoreEvent::Error(m) = e { m.contains("llm_provider").then_some(()) } else { None }).await;
    node.handle.send(CoreCommand::SetEngines(EngineSettings { local_threads: Some(0), ..remote_engines() })).await.unwrap();
    wait(&mut node, |e| if let CoreEvent::Error(m) = e { m.contains("local_threads").then_some(()) } else { None }).await;
    let good = EngineSettings {
        providers: [(
            ProviderId::Custom,
            ProviderSettings { asr_url: Some("https://asr2.example.test".into()), asr_model: Some("m".into()), ..Default::default() },
        )]
        .into(),
        language: Some("zh".into()),
        inject: InjectMode::ClipboardOnly,
        refine_enabled: false,
        ..remote_engines()
    };
    node.handle.send(CoreCommand::SetEngines(good.clone())).await.unwrap();
    let settings = wait(&mut node, |e| if let CoreEvent::Settings(s) = e { Some(s.clone()) } else { None }).await;
    assert_eq!(settings.engines, good);
    let engines = wait(&mut node, |e| if let CoreEvent::Engines(s) = e { Some(s.clone()) } else { None }).await;
    assert_eq!(engines.asr_host, "asr2.example.test");
    assert_eq!(engines.language.as_deref(), Some("zh"));
    assert!(!engines.refine_enabled);
    assert_eq!(engines.inject, InjectMode::ClipboardOnly);
    assert_eq!(SettingsStore::new(dir.path()).load().unwrap().engines, good);

    // Delete one, clear the rest.
    node.handle.send(CoreCommand::HistoryDelete(id)).await.unwrap();
    let h = wait(&mut node, |e| if let CoreEvent::History { recent: h, .. } = e { Some(h.clone()) } else { None }).await;
    assert_eq!(h.len(), 1);
    assert_ne!(h[0].id, id);
    node.handle.send(CoreCommand::HistoryClear).await.unwrap();
    let h = wait(&mut node, |e| if let CoreEvent::History { recent: h, .. } = e { Some(h.clone()) } else { None }).await;
    assert!(h.is_empty());

    // Deleting a key goes back to "none".
    node.handle.send(CoreCommand::SetProviderKey { provider: ProviderId::Groq, kind: ServiceKind::Asr, value: None }).await.unwrap();
    let engines = wait(&mut node, |e| if let CoreEvent::Engines(s) = e { Some(s.clone()) } else { None }).await;
    assert!(!groq(&engines).key.set, "a vendor's services share one key");
    assert!(store.get("provider-key.groq").unwrap().is_none());

    // Cancel during a run, then shut down.
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    wait_phase(&mut node, |p| matches!(p, DictationPhase::Listening { .. })).await;
    node.handle.send(CoreCommand::DictationCancel).await.unwrap();
    assert_eq!(wait_phase(&mut node, DictationPhase::is_terminal).await, DictationPhase::CANCELLED);
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

#[tokio::test]
async fn history_and_secret_survive_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(MemorySecretStore::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let injector = Arc::new(FakeInjector::clipboard(Some("paste blocked")));
    let mut node = start(dir.path(), store.clone(), calls.clone(), injector);
    wait(&mut node, |e| matches!(e, CoreEvent::Ready { .. }).then_some(())).await;
    node.handle.send(CoreCommand::SetProviderKey { provider: ProviderId::Custom, kind: ServiceKind::Asr, value: Some("  tok  ".into()) }).await.unwrap();
    let custom_asr = |s: &EngineStatus| s.providers.iter().find(|p| p.id == ProviderId::Custom).and_then(|p| p.asr.clone()).unwrap();
    let engines = wait(&mut node, |e| if let CoreEvent::Engines(s) = e { custom_asr(s).key.set.then(|| s.clone()) } else { None }).await;
    assert_eq!(custom_asr(&engines).key.source, SecretSource::User);
    let custom_llm = engines.providers.iter().find(|p| p.id == ProviderId::Custom).and_then(|p| p.llm.clone()).unwrap();
    assert!(!custom_llm.key.set, "the custom endpoint keeps one key per service");
    assert_eq!(store.get("provider-key.custom-asr").unwrap().unwrap().as_slice(), b"tok", "trimmed before storing");
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    wait_phase(&mut node, |p| matches!(p, DictationPhase::Listening { .. })).await;
    node.handle.send(CoreCommand::DictationStop).await.unwrap();
    let done = wait_phase(&mut node, DictationPhase::is_terminal).await;
    assert!(matches!(done, DictationPhase::Done { via: voltip_core::dictation::Via::Clipboard, .. }), "{done:?}");
    wait(&mut node, |e| if let CoreEvent::History { recent: h, .. } = e { (!h.is_empty()).then_some(()) } else { None }).await;
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
    // Let the task drain.
    while tokio::time::timeout(Duration::from_millis(200), node.events.recv()).await.ok().flatten().is_some() {}

    let mut again = start(dir.path(), store.clone(), calls, Arc::new(FakeInjector::paste()));
    wait(&mut again, |e| matches!(e, CoreEvent::Ready { .. }).then_some(())).await;
    let engines = wait(&mut again, |e| if let CoreEvent::Engines(s) = e { Some(s.clone()) } else { None }).await;
    assert_eq!(custom_asr(&engines).key.source, SecretSource::User, "key reloaded from the store");
    let history = wait(&mut again, |e| if let CoreEvent::History { recent: h, .. } = e { Some(h.clone()) } else { None }).await;
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].outcome, Outcome::Clipboard { reason: "paste blocked".into(), code: Some(voltip_core::dictation::ClipboardCode::Other) });
    again.handle.send(CoreCommand::Shutdown).await.unwrap();
}

#[tokio::test]
async fn start_without_ports_uses_the_fakes() {
    let dir = tempfile::tempdir().unwrap();
    let (handle, mut events) = AppCore::start(config(dir.path()), Arc::new(MemorySecretStore::new())).unwrap();
    let mut node = Node { handle, events: mpsc::channel(1).1 };
    std::mem::swap(&mut node.events, &mut events);
    wait(&mut node, |e| matches!(e, CoreEvent::Ready { .. }).then_some(())).await;
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    wait_phase(&mut node, |p| matches!(p, DictationPhase::Listening { .. })).await;
    node.handle.send(CoreCommand::DictationStop).await.unwrap();
    assert!(matches!(wait_phase(&mut node, DictationPhase::is_terminal).await, DictationPhase::Done { .. }));
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// One take start to finish.
async fn take(node: &mut Node) {
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    wait_phase(node, |p| matches!(p, DictationPhase::Listening { .. })).await;
    node.handle.send(CoreCommand::DictationStop).await.unwrap();
    wait_phase(node, DictationPhase::is_terminal).await;
}

/// docs/dictation.md §4: history off records nothing new, a smaller `keep` trims at once, and a
/// `keep` out of range is refused without changing anything.
#[tokio::test]
async fn history_can_be_switched_off_and_trimmed() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(MemorySecretStore::new());
    let mut node = start(dir.path(), store, Arc::new(AtomicUsize::new(0)), Arc::new(FakeInjector::paste()));
    wait(&mut node, |e| matches!(e, CoreEvent::Ready { .. }).then_some(())).await;
    for _ in 0..3 {
        take(&mut node).await;
    }
    let h = wait(&mut node, |e| if let CoreEvent::History { recent: h, .. } = e { (h.len() == 3).then(|| h.clone()) } else { None }).await;
    assert_eq!(h.len(), 3);
    // Out of range: refused, nothing changes.
    node.handle.send(CoreCommand::SetHistory(voltip_core::HistorySettings { enabled: true, keep: 1 })).await.unwrap();
    let err = wait(&mut node, |e| if let CoreEvent::Error(m) = e { Some(m.clone()) } else { None }).await;
    assert!(err.contains("history.keep"), "{err}");
    // Off: the next take is not recorded.
    node.handle.send(CoreCommand::SetHistory(voltip_core::HistorySettings { enabled: false, keep: 500 })).await.unwrap();
    let settings = wait(&mut node, |e| if let CoreEvent::Settings(s) = e { Some(s.clone()) } else { None }).await;
    assert!(!settings.history.enabled);
    take(&mut node).await;
    let stored = voltip_core::HistoryStore::open(dir.path());
    assert_eq!(stored.total(), 3, "history off keeps nothing new");
    // Keep 10 with 3 entries: nothing to trim; the setting persists.
    node.handle.send(CoreCommand::SetHistory(voltip_core::HistorySettings { enabled: true, keep: 10 })).await.unwrap();
    wait(&mut node, |e| if let CoreEvent::Settings(s) = e { (s.history.keep == 10).then_some(()) } else { None }).await;
    assert_eq!(SettingsStore::new(dir.path()).load().unwrap().history, voltip_core::HistorySettings { enabled: true, keep: 10 });
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// docs/dictation.md §22: 设置 › 录音来源 and 最长录音时长 go to the microphone port from the next
/// take on, are persisted, and a length or an output device id outside the choices is refused
/// with the setting left as it was.
#[tokio::test]
async fn the_recording_settings_reach_the_capture_and_persist() {
    let dir = tempfile::tempdir().unwrap();
    let audio = Arc::new(FakeAudio::speech());
    let mut ports = ports(Arc::new(AtomicUsize::new(0)), Arc::new(FakeInjector::paste()));
    ports.audio = audio.clone();
    let (handle, events) = AppCore::start_with(config(dir.path()), Arc::new(MemorySecretStore::new()), ports).unwrap();
    let mut node = Node { handle, events };
    let recording = RecordingSettings { source: RecordingSource::Mixed, output_device: Some("fake:speakers".into()), max_minutes: 30 };
    node.handle.send(CoreCommand::SetRecording(recording.clone())).await.unwrap();
    wait(&mut node, |e| matches!(e, CoreEvent::Settings(s) if s.recording == recording).then_some(())).await;
    assert_eq!(SettingsStore::new(dir.path()).load().unwrap().recording, recording);
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    wait_phase(&mut node, |p| matches!(p, DictationPhase::Listening { .. })).await;
    node.handle.send(CoreCommand::DictationStop).await.unwrap();
    wait_phase(&mut node, |p| matches!(p, DictationPhase::Done { .. } | DictationPhase::Failed { .. })).await;
    let options = audio.options();
    assert_eq!((options[0].source, options[0].output_device.as_deref()), (RecordingSource::Mixed, Some("fake:speakers")));
    assert_eq!((options[0].max_duration, options[0].long), (Duration::from_secs(1800), true));
    for bad in [
        RecordingSettings { max_minutes: 15, ..recording.clone() },
        RecordingSettings { max_minutes: 0, ..recording.clone() },
        RecordingSettings { output_device: Some(" ".into()), ..recording.clone() },
        RecordingSettings { output_device: Some("x".repeat(1025)), ..recording.clone() },
    ] {
        node.handle.send(CoreCommand::SetRecording(bad)).await.unwrap();
        let refused = wait(&mut node, |e| match e {
            CoreEvent::Error(m) => Some(m.clone()),
            _ => None,
        })
        .await;
        assert!(refused.starts_with("recording."), "{refused}");
    }
    assert_eq!(SettingsStore::new(dir.path()).load().unwrap().recording, recording, "a refused value changes nothing");
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// Regression (user feedback 2026-09-28): 设置 › 麦克风 chooses the device takes record from. The
/// choice is validated, persisted, and handed to the microphone port from the next take on;
/// `None` goes back to the system default.
#[tokio::test]
async fn the_chosen_microphone_reaches_the_recorder_and_persists() {
    let dir = tempfile::tempdir().unwrap();
    let audio = Arc::new(FakeAudio::speech());
    let mut ports = ports(Arc::new(AtomicUsize::new(0)), Arc::new(FakeInjector::paste()));
    ports.audio = audio.clone();
    let (handle, events) = AppCore::start_with(config(dir.path()), Arc::new(MemorySecretStore::new()), ports).unwrap();
    let mut node = Node { handle, events };
    let take = async |node: &mut Node| {
        node.handle.send(CoreCommand::DictationStart).await.unwrap();
        wait_phase(node, |p| matches!(p, DictationPhase::Listening { .. })).await;
        node.handle.send(CoreCommand::DictationStop).await.unwrap();
        wait_phase(node, |p| matches!(p, DictationPhase::Done { .. } | DictationPhase::Failed { .. })).await;
        wait_phase(node, |p| matches!(p, DictationPhase::Idle)).await;
    };
    take(&mut node).await;
    node.handle.send(CoreCommand::SetMicrophone(Some("fake:usb-mic".into()))).await.unwrap();
    let settings = wait(&mut node, |e| match e {
        CoreEvent::Settings(s) => Some(s.clone()),
        _ => None,
    })
    .await;
    assert_eq!(settings.microphone.as_deref(), Some("fake:usb-mic"));
    assert_eq!(SettingsStore::new(dir.path()).load().unwrap().microphone.as_deref(), Some("fake:usb-mic"));
    take(&mut node).await;
    // Nothing that is not a device id: an empty id is refused and the choice stays.
    node.handle.send(CoreCommand::SetMicrophone(Some("  ".into()))).await.unwrap();
    let refused = wait(&mut node, |e| match e {
        CoreEvent::Error(m) => Some(m.clone()),
        _ => None,
    })
    .await;
    assert!(refused.contains("microphone"), "{refused}");
    node.handle.send(CoreCommand::SetMicrophone(None)).await.unwrap();
    wait(&mut node, |e| matches!(e, CoreEvent::Settings(s) if s.microphone.is_none()).then_some(())).await;
    take(&mut node).await;
    assert_eq!(audio.devices(), vec![None, Some("fake:usb-mic".to_owned()), None]);
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}
