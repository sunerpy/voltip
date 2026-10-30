#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The audio cache (docs/dictation.md §26) through the real core task: a take's recording kept on
//! disk and linked to its entry, 重新识别 onto the clipboard, the settings and 清理录音, and the
//! recordings following the history — fakes for the microphone, the recogniser and the injector,
//! a temp dir for the data.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use uuid::Uuid;
use voltip_core::audio_cache::{AUDIO_CACHE_DIR, AudioCacheView, CACHE_UNAVAILABLE, KeptStatus, PolishChoice, RECORDING_GONE, RecognitionChoice, RetakeModels};
use voltip_core::dictation::fakes::{FakeAudio, FakeInjector, FakeRefiner, FakeTranscriber, factory_by_model};
use voltip_core::dictation::{DictationPorts, Refiner, Transcriber, Via};
use voltip_core::{
    AppCore, AudioCacheSettings, CoreCommand, CoreConfig, CoreEvent, CoreHandle, DictationPhase, EngineSettings, HistoryEntry, HistorySettings, Outcome,
    ProviderId, ProviderSettings, ResolvedEngines, Settings, SettingsStore,
};
use voltip_identity::MemorySecretStore;

const STEP: Duration = Duration::from_secs(10);

struct Node {
    handle: CoreHandle,
    events: mpsc::Receiver<CoreEvent>,
}

/// Recognition at a custom endpoint the fake stands in for; no clean-up.
fn engines() -> EngineSettings {
    EngineSettings {
        asr_provider: ProviderId::Custom,
        refine_enabled: false,
        providers: [(
            ProviderId::Custom,
            ProviderSettings { asr_url: Some("https://asr.example.test".into()), asr_model: Some("whisper".into()), ..Default::default() },
        )]
        .into(),
        ..EngineSettings::default()
    }
}

fn start_with(dir: &Path, settings: Settings, transcriber: Arc<FakeTranscriber>, injector: Arc<FakeInjector>) -> Node {
    SettingsStore::new(dir).save(&Settings { relay_enabled: false, engines: engines(), ..settings }).unwrap();
    let ports = DictationPorts {
        audio: Arc::new(FakeAudio::speech()),
        injector,
        factory: Arc::new(move |_: &ResolvedEngines| (transcriber.clone() as Arc<dyn Transcriber>, None::<Arc<dyn Refiner>>)),
        models: None,
        streaming: None,
        probe: None,
        service_probe: None,
        segmenter: None,
    };
    let (handle, events) = AppCore::start_with(CoreConfig::new(dir.to_path_buf()), Arc::new(MemorySecretStore::new()), ports).unwrap();
    Node { handle, events }
}

fn start(dir: &Path, transcriber: Arc<FakeTranscriber>, injector: Arc<FakeInjector>) -> Node {
    start_with(dir, Settings::default(), transcriber, injector)
}

async fn wait<T>(node: &mut Node, mut pick: impl FnMut(&CoreEvent) -> Option<T>) -> T {
    loop {
        let ev = tokio::time::timeout(STEP, node.events.recv()).await.expect("event within 10 s").expect("core alive");
        if let Some(v) = pick(&ev) {
            return v;
        }
    }
}

async fn ready(node: &mut Node) -> AudioCacheView {
    wait(node, |e| matches!(e, CoreEvent::Ready { .. }).then_some(())).await;
    wait(node, |e| if let CoreEvent::AudioCache(v) = e { Some(v.clone()) } else { None }).await
}

/// The cache as it is once `done` says so.
async fn cache_until(node: &mut Node, mut done: impl FnMut(&AudioCacheView) -> bool) -> AudioCacheView {
    wait(node, |e| if let CoreEvent::AudioCache(v) = e { done(v).then(|| v.clone()) } else { None }).await
}

async fn history_until(node: &mut Node, mut done: impl FnMut(&[HistoryEntry]) -> bool) -> Vec<HistoryEntry> {
    wait(node, |e| if let CoreEvent::History { recent, .. } = e { done(recent).then(|| recent.clone()) } else { None }).await
}

async fn error(node: &mut Node) -> String {
    wait(node, |e| if let CoreEvent::Error(m) = e { Some(m.clone()) } else { None }).await
}

/// One take, start to its end.
async fn take(node: &mut Node) -> DictationPhase {
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    wait(node, |e| matches!(e, CoreEvent::Dictation(s) if matches!(s.phase, DictationPhase::Listening { .. })).then_some(())).await;
    node.handle.send(CoreCommand::DictationStop).await.unwrap();
    wait(node, |e| match e {
        CoreEvent::Dictation(s) if s.phase.is_terminal() => Some(s.phase.clone()),
        _ => None,
    })
    .await
}

fn wav(dir: &Path, id: Uuid) -> std::path::PathBuf {
    dir.join(AUDIO_CACHE_DIR).join(format!("{id}.wav"))
}

async fn shutdown(mut node: Node) {
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
    // The channel closes once the task has ended.
    while tokio::time::timeout(STEP, node.events.recv()).await.expect("the core ends within 10 s").is_some() {}
}

#[tokio::test]
async fn a_take_keeps_its_recording_which_goes_with_its_entry() {
    let dir = tempfile::tempdir().unwrap();
    let mut node = start(dir.path(), Arc::new(FakeTranscriber::ok("你好世界")), Arc::new(FakeInjector::paste()));
    let initial = ready(&mut node).await;
    assert!(initial.available && initial.recordings.is_empty() && initial.bytes == 0, "{initial:?}");
    assert!(matches!(take(&mut node).await, DictationPhase::Done { .. }));
    let entry = history_until(&mut node, |h| !h.is_empty()).await[0].id;
    let cache = cache_until(&mut node, |v| v.recordings.first().is_some_and(|r| r.status.entry().is_some())).await;
    let [kept] = cache.recordings.as_slice() else { panic!("{cache:?}") };
    assert_eq!(kept.status, KeptStatus::Recognized { entry });
    assert_eq!(kept.duration_ms, 1500);
    let file = wav(dir.path(), kept.id);
    let bytes = std::fs::read(&file).expect("the WAV is on disk");
    assert!(bytes.starts_with(b"RIFF"));
    assert_eq!(cache.bytes, bytes.len() as u64);
    assert_eq!(kept.bytes, cache.bytes);

    // It survives a restart, still linked.
    shutdown(node).await;
    let mut node = start(dir.path(), Arc::new(FakeTranscriber::ok("你好世界")), Arc::new(FakeInjector::paste()));
    let again = ready(&mut node).await;
    assert_eq!(again.recordings.len(), 1);
    assert_eq!(again.recordings[0].status, KeptStatus::Recognized { entry });

    node.handle.send(CoreCommand::HistoryDelete(entry)).await.unwrap();
    let cache = cache_until(&mut node, |v| v.recordings.is_empty()).await;
    assert_eq!(cache.bytes, 0);
    assert!(!file.exists(), "the WAV is deleted with its entry");
    shutdown(node).await;
}

#[tokio::test]
async fn a_failed_take_is_recognised_again_onto_the_clipboard() {
    let dir = tempfile::tempdir().unwrap();
    // The first request fails, the next one answers.
    let transcriber = Arc::new(FakeTranscriber::numbered(0, &[1], Duration::ZERO));
    let injector = Arc::new(FakeInjector::paste());
    let mut node = start(dir.path(), transcriber.clone(), injector.clone());
    ready(&mut node).await;
    assert!(matches!(take(&mut node).await, DictationPhase::Failed { text: None, .. }));
    let cache = cache_until(&mut node, |v| v.recordings.first().is_some_and(|r| matches!(r.status, KeptStatus::Failed { .. }))).await;
    let id = cache.recordings[0].id;

    node.handle.send(CoreCommand::AudioRetake { id, models: RetakeModels::default() }).await.unwrap();
    let mut retake = None;
    let done = wait(&mut node, |e| match e {
        CoreEvent::Dictation(s) => {
            if matches!(s.phase, DictationPhase::Processing { .. }) {
                retake = Some((s.recording, s.retake));
            }
            s.phase.is_terminal().then(|| s.phase.clone())
        }
        _ => None,
    })
    .await;
    assert_eq!(retake, Some((Some(id), true)), "the processing status names the recording and the retake");
    assert!(matches!(&done, DictationPhase::Done { text, via: Via::Clipboard, .. } if text == "第2段。"), "{done:?}");
    assert!(injector.injected().is_empty(), "never pasted");
    assert_eq!(injector.clipboard_copies(), vec!["第2段。".to_owned()]);
    assert_eq!(transcriber.durations_ms(), vec![1500, 1500], "the same recording, sent again");
    let history = history_until(&mut node, |h| !h.is_empty()).await;
    assert_eq!(history[0].outcome, Outcome::Inserted { via: Via::Clipboard, check: None });
    let cache = cache_until(&mut node, |v| v.recordings.first().is_some_and(|r| r.status.entry().is_some())).await;
    assert_eq!(cache.recordings[0].status, KeptStatus::Recognized { entry: history[0].id });

    // A recording the cache no longer holds says so.
    node.handle.send(CoreCommand::AudioCacheDelete(id)).await.unwrap();
    cache_until(&mut node, |v| v.recordings.is_empty()).await;
    node.handle.send(CoreCommand::AudioRetake { id, models: RetakeModels::default() }).await.unwrap();
    assert_eq!(error(&mut node).await, RECORDING_GONE);
    shutdown(node).await;
}

/// 重新识别 on other models (docs/dictation.md §26.4): a take failed on the settings' recogniser;
/// its recording is recognised on another model and cleaned up by another, for that take only —
/// the settings keep theirs and the next take is theirs again. A choice that cannot run is refused
/// before a take starts.
#[tokio::test]
async fn a_recording_is_recognised_again_on_other_models_for_that_take_only() {
    let dir = tempfile::tempdir().unwrap();
    let failing = Arc::new(FakeTranscriber::err("503 service unavailable"));
    let other = Arc::new(FakeTranscriber::ok("换个模型识别出的文字"));
    let polish = Arc::new(FakeRefiner::ok("换个模型润色后的文字。").with_model("polish-b"));
    let injector = Arc::new(FakeInjector::paste());
    let custom = ProviderSettings {
        asr_url: Some("https://asr.example.test".into()),
        asr_model: Some("whisper".into()),
        llm_url: Some("https://llm.example.test/v1".into()),
        llm_model: Some("polish-a".into()),
        ..Default::default()
    };
    let engines = EngineSettings {
        asr_provider: ProviderId::Custom,
        llm_provider: ProviderId::Custom,
        refine_enabled: false,
        providers: [(ProviderId::Custom, custom)].into(),
        ..EngineSettings::default()
    };
    SettingsStore::new(dir.path()).save(&Settings { relay_enabled: false, engines: engines.clone(), ..Settings::default() }).unwrap();
    let ports = DictationPorts {
        audio: Arc::new(FakeAudio::speech()),
        injector: injector.clone(),
        factory: factory_by_model(&[("whisper", failing.clone()), ("whisper-2", other.clone())], &[("polish-b", polish.clone())]),
        models: None,
        streaming: None,
        probe: None,
        service_probe: None,
        segmenter: None,
    };
    let (handle, events) = AppCore::start_with(CoreConfig::new(dir.path().to_path_buf()), Arc::new(MemorySecretStore::new()), ports).unwrap();
    let mut node = Node { handle, events };
    ready(&mut node).await;
    assert!(matches!(take(&mut node).await, DictationPhase::Failed { text: None, .. }));
    let cache = cache_until(&mut node, |v| v.recordings.first().is_some_and(|r| matches!(r.status, KeptStatus::Failed { .. }))).await;
    let id = cache.recordings[0].id;

    // A model this computer cannot run is refused, and nothing is asked.
    let groq = RetakeModels { asr: Some(RecognitionChoice::Remote { provider: ProviderId::Groq, model: "whisper-large-v3".into() }), refine: None };
    node.handle.send(CoreCommand::AudioRetake { id, models: groq }).await.unwrap();
    assert!(!error(&mut node).await.is_empty());
    let local = RetakeModels { asr: Some(RecognitionChoice::Local { id: "sense-voice-small".into() }), refine: None };
    node.handle.send(CoreCommand::AudioRetake { id, models: local }).await.unwrap();
    assert!(error(&mut node).await.contains("local_model"));

    let models = RetakeModels {
        asr: Some(RecognitionChoice::Remote { provider: ProviderId::Custom, model: "whisper-2".into() }),
        refine: Some(PolishChoice::Remote { provider: ProviderId::Custom, model: "polish-b".into() }),
    };
    node.handle.send(CoreCommand::AudioRetake { id, models }).await.unwrap();
    let done = wait(&mut node, |e| match e {
        CoreEvent::Dictation(s) if s.phase.is_terminal() => Some(s.phase.clone()),
        _ => None,
    })
    .await;
    assert!(
        matches!(&done, DictationPhase::Done { text, raw_text, refined: true, via: Via::Clipboard, .. } if text == "换个模型润色后的文字。" && raw_text == "换个模型识别出的文字"),
        "{done:?}"
    );
    assert_eq!((failing.calls(), other.calls(), polish.calls()), (1, 1, 1));
    assert_eq!(injector.clipboard_copies(), vec!["换个模型润色后的文字。".to_owned()]);
    let entry = history_until(&mut node, |h| !h.is_empty()).await.remove(0);
    assert_eq!((entry.asr_model.as_str(), entry.refine_model.as_deref()), ("whisper-2", Some("polish-b")));
    let cache = cache_until(&mut node, |v| v.recordings.first().is_some_and(|r| r.status.entry().is_some())).await;
    assert_eq!(cache.recordings[0].status, KeptStatus::Recognized { entry: entry.id });
    assert_eq!(SettingsStore::new(dir.path()).load().unwrap().engines, engines, "the settings keep their models");

    // The next take is the settings' again.
    assert!(matches!(take(&mut node).await, DictationPhase::Failed { .. }));
    assert_eq!((failing.calls(), other.calls()), (2, 1));
    shutdown(node).await;
}

#[tokio::test]
async fn the_settings_and_clearing_decide_what_stays() {
    let dir = tempfile::tempdir().unwrap();
    let mut node = start(dir.path(), Arc::new(FakeTranscriber::err("down")), Arc::new(FakeInjector::paste()));
    ready(&mut node).await;
    for _ in 0..3 {
        take(&mut node).await;
    }
    let cache = cache_until(&mut node, |v| v.recordings.len() == 3 && v.recordings.iter().all(|r| matches!(r.status, KeptStatus::Failed { .. }))).await;
    let newest = cache.recordings[0].id;

    // Out of range is refused.
    node.handle.send(CoreCommand::SetAudioCache(AudioCacheSettings { enabled: true, keep: 0, max_mb: 200 })).await.unwrap();
    assert!(error(&mut node).await.contains("keep"));
    // Fewer to keep drops the oldest.
    node.handle.send(CoreCommand::SetAudioCache(AudioCacheSettings { enabled: true, keep: 1, max_mb: 200 })).await.unwrap();
    let cache = cache_until(&mut node, |v| v.recordings.len() == 1).await;
    assert_eq!(cache.recordings[0].id, newest);
    // Off deletes them, and the next take keeps nothing.
    node.handle.send(CoreCommand::SetAudioCache(AudioCacheSettings { enabled: false, keep: 1, max_mb: 200 })).await.unwrap();
    cache_until(&mut node, |v| v.recordings.is_empty()).await;
    take(&mut node).await;
    node.handle.send(CoreCommand::SetAudioCache(AudioCacheSettings { enabled: true, keep: 20, max_mb: 200 })).await.unwrap();
    let settings = wait(&mut node, |e| if let CoreEvent::Settings(s) = e { s.audio_cache.enabled.then(|| s.clone()) } else { None }).await;
    assert_eq!(settings.audio_cache, AudioCacheSettings { enabled: true, keep: 20, max_mb: 200 });
    let saved = SettingsStore::new(dir.path()).load().unwrap();
    assert_eq!(saved.audio_cache, settings.audio_cache, "persisted");
    assert!(std::fs::read_dir(dir.path().join(AUDIO_CACHE_DIR)).map_or(true, |d| d.count() == 0), "no file left behind");

    // 清理录音 deletes every recording; clearing the history too.
    take(&mut node).await;
    take(&mut node).await;
    cache_until(&mut node, |v| v.recordings.len() == 2).await;
    node.handle.send(CoreCommand::AudioCacheClear).await.unwrap();
    cache_until(&mut node, |v| v.recordings.is_empty()).await;
    take(&mut node).await;
    cache_until(&mut node, |v| v.recordings.len() == 1).await;
    node.handle.send(CoreCommand::HistoryClear).await.unwrap();
    cache_until(&mut node, |v| v.recordings.is_empty()).await;
    shutdown(node).await;
}

#[tokio::test]
async fn without_the_history_no_recording_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let settings = Settings { history: HistorySettings { enabled: false, keep: 200 }, ..Settings::default() };
    let mut node = start_with(dir.path(), settings, Arc::new(FakeTranscriber::err("down")), Arc::new(FakeInjector::paste()));
    ready(&mut node).await;
    take(&mut node).await;
    // Turning the history on again keeps the next take's recording: the first left none.
    node.handle.send(CoreCommand::SetHistory(HistorySettings { enabled: true, keep: 200 })).await.unwrap();
    wait(&mut node, |e| if let CoreEvent::Settings(s) = e { s.history.enabled.then_some(()) } else { None }).await;
    take(&mut node).await;
    let cache = cache_until(&mut node, |v| !v.recordings.is_empty()).await;
    assert_eq!(cache.recordings.len(), 1, "{cache:?}");
    shutdown(node).await;
}

#[tokio::test]
async fn a_core_without_the_cache_keeps_nothing_and_refuses_a_retake() {
    let dir = tempfile::tempdir().unwrap();
    SettingsStore::new(dir.path()).save(&Settings { relay_enabled: false, engines: engines(), ..Settings::default() }).unwrap();
    let mut config = CoreConfig::new(dir.path().to_path_buf());
    config.keeps_audio = false;
    let transcriber = Arc::new(FakeTranscriber::err("down"));
    let ports = DictationPorts {
        audio: Arc::new(FakeAudio::speech()),
        injector: Arc::new(FakeInjector::paste()),
        factory: Arc::new(move |_: &ResolvedEngines| (transcriber.clone() as Arc<dyn Transcriber>, None::<Arc<dyn Refiner>>)),
        models: None,
        streaming: None,
        probe: None,
        service_probe: None,
        segmenter: None,
    };
    let (handle, events) = AppCore::start_with(config, Arc::new(MemorySecretStore::new()), ports).unwrap();
    let mut node = Node { handle, events };
    let view = ready(&mut node).await;
    assert!(!view.available);
    take(&mut node).await;
    node.handle.send(CoreCommand::AudioRetake { id: Uuid::new_v4(), models: RetakeModels::default() }).await.unwrap();
    assert_eq!(error(&mut node).await, CACHE_UNAVAILABLE);
    assert!(!dir.path().join(AUDIO_CACHE_DIR).exists(), "no directory made");
    shutdown(node).await;
}
