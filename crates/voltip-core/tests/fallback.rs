#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Fallback models through the real core task (docs/dictation.md §3.5): the settings are checked,
//! a model that runs out of quota shows on the engines status with its retry time, 重新检查
//! (`ResetQuota`) and a new key for its provider forget it, and the next take starts from the
//! selected model again.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use voltip_core::dictation::fakes::{FakeAudio, FakeInjector, FakeTranscriber, factory_by_model, ports_with};
use voltip_core::{
    AppCore, CoreCommand, CoreConfig, CoreEvent, CoreHandle, DictationPhase, EngineSettings, EngineStatus, FallbackModel, FallbackSettings, ProviderId,
    ProviderSettings, ServiceKind, Settings, SettingsStore,
};
use voltip_identity::MemorySecretStore;

const STEP: Duration = Duration::from_secs(10);

struct Node {
    handle: CoreHandle,
    events: mpsc::Receiver<CoreEvent>,
    selected: Arc<FakeTranscriber>,
    fallback: Arc<FakeTranscriber>,
    /// The last engines status seen, whatever the wait was for: a quota change may be reported
    /// while a take's phases are awaited.
    engines: Option<EngineStatus>,
    /// The newest history entry seen, for the same reason.
    newest: Option<voltip_core::HistoryEntry>,
}

/// Recognition on Model Studio with one fallback model; no clean-up.
fn engines() -> EngineSettings {
    EngineSettings {
        asr_provider: ProviderId::Aliyun,
        refine_enabled: false,
        providers: [(ProviderId::Aliyun, ProviderSettings { asr_model: Some("qwen-audio-3.1-asr-flash".into()), ..Default::default() })].into(),
        asr_fallback: FallbackSettings { enabled: true, models: vec![FallbackModel { provider: ProviderId::Aliyun, model: "qwen3-asr-flash".into() }] },
        ..EngineSettings::default()
    }
}

fn start(dir: &std::path::Path, selected: FakeTranscriber, fallback: FakeTranscriber) -> Node {
    SettingsStore::new(dir).save(&Settings { relay_enabled: false, engines: engines(), ..Settings::default() }).unwrap();
    let mut config = CoreConfig::new(dir.to_path_buf());
    config.default_device_name = "Fallback Test".into();
    config.direct_enabled = false;
    let (selected, fallback) = (Arc::new(selected), Arc::new(fallback));
    let mut ports = ports_with(Arc::new(FakeAudio::speech()), selected.clone(), None, Arc::new(FakeInjector::paste()));
    ports.factory = factory_by_model(&[("qwen-audio-3.1-asr-flash", selected.clone()), ("qwen3-asr-flash", fallback.clone())], &[]);
    let store = MemorySecretStore::new();
    voltip_identity::SecretStore::set(&store, "provider-key.aliyun", b"sk-test").unwrap();
    let (handle, events) = AppCore::start_with(config, Arc::new(store), ports).unwrap();
    Node { handle, events, selected, fallback, engines: None, newest: None }
}

async fn wait<T>(node: &mut Node, mut pick: impl FnMut(&CoreEvent) -> Option<T>) -> T {
    loop {
        let ev = tokio::time::timeout(STEP, node.events.recv()).await.expect("event within 10 s").expect("core alive");
        match &ev {
            CoreEvent::Engines(status) => node.engines = Some(status.clone()),
            CoreEvent::History { recent, .. } if !recent.is_empty() => node.newest = recent.first().cloned(),
            _ => {}
        }
        if let Some(v) = pick(&ev) {
            return v;
        }
    }
}

/// The engines status once `pred` holds for it: the last one seen already, or a later one.
async fn engines_where(node: &mut Node, pred: impl Fn(&EngineStatus) -> bool) -> EngineStatus {
    if let Some(status) = node.engines.clone().filter(|s| pred(s)) {
        return status;
    }
    wait(node, |e| match e {
        CoreEvent::Engines(s) if pred(s) => Some(s.clone()),
        _ => None,
    })
    .await
}

async fn take(node: &mut Node) -> DictationPhase {
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    wait(node, |e| matches!(e, CoreEvent::Dictation(s) if matches!(s.phase, DictationPhase::Listening { ready: true, .. })).then_some(())).await;
    node.handle.send(CoreCommand::DictationStop).await.unwrap();
    let done = wait(node, |e| match e {
        CoreEvent::Dictation(s) if s.phase.is_terminal() => Some(s.phase.clone()),
        _ => None,
    })
    .await;
    wait(node, |e| matches!(e, CoreEvent::Dictation(s) if s.phase == DictationPhase::Idle).then_some(())).await;
    done
}

fn selected_out(status: &EngineStatus) -> bool {
    status.asr_fallback.selected_retry_at_ms.is_some()
}

#[tokio::test]
async fn a_used_up_model_shows_on_the_status_until_it_is_checked_again() {
    let dir = tempfile::tempdir().unwrap();
    let mut node = start(dir.path(), FakeTranscriber::quota(), FakeTranscriber::ok("候补识别的文字"));
    let first = engines_where(&mut node, |_| true).await;
    assert!(first.asr_fallback.enabled && first.asr_fallback.in_use && !selected_out(&first), "{:?}", first.asr_fallback);
    assert_eq!(first.asr_fallback.models.len(), 1);

    // The selected model refuses: the take is the fallback's, and the status says the selected
    // one ran out (the ledger's change re-sends it).
    let done = take(&mut node).await;
    assert!(matches!(&done, DictationPhase::Done { text, .. } if text == "候补识别的文字"), "{done:?}");
    let marked = engines_where(&mut node, selected_out).await;
    let now_ms = u64::try_from(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis()).unwrap();
    let retry = marked.asr_fallback.selected_retry_at_ms.unwrap();
    assert!(retry > now_ms + 23 * 3_600_000 && retry <= now_ms + 24 * 3_600_000, "a day from now: {retry} vs {now_ms}");
    // The take's entry was reported while its phases were awaited.
    assert_eq!(node.newest.as_ref().map(|e| e.asr_model.as_str()), Some("qwen3-asr-flash"));

    // The next take skips it.
    take(&mut node).await;
    assert_eq!((node.selected.calls(), node.fallback.calls()), (1, 2));

    // 重新检查: forgotten, and the next take asks the selected model again.
    node.handle.send(CoreCommand::ResetQuota(ServiceKind::Asr)).await.unwrap();
    engines_where(&mut node, |s| !selected_out(s)).await;
    take(&mut node).await;
    assert_eq!((node.selected.calls(), node.fallback.calls()), (2, 3));
    engines_where(&mut node, selected_out).await;

    // A new key for the provider forgets its models too: maybe another account.
    node.handle.send(CoreCommand::SetProviderKey { provider: ProviderId::Aliyun, kind: ServiceKind::Asr, value: Some("sk-other".into()) }).await.unwrap();
    engines_where(&mut node, |s| !selected_out(s)).await;
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// What `SetEngines` refuses in a fallback list, and that a valid list is kept.
#[tokio::test]
async fn fallback_lists_are_checked_before_they_are_saved() {
    let dir = tempfile::tempdir().unwrap();
    let mut node = start(dir.path(), FakeTranscriber::ok("x"), FakeTranscriber::ok("y"));
    engines_where(&mut node, |_| true).await;
    let with = |models: Vec<FallbackModel>| EngineSettings { asr_fallback: FallbackSettings { enabled: true, models }, ..engines() };
    let entry = |provider, model: &str| FallbackModel { provider, model: model.into() };
    let refused = [
        (with(vec![entry(ProviderId::Local, "qwen3-asr-0.6b")]), "不能作为候补模型"),
        (with(vec![entry(ProviderId::Deepseek, "deepseek-chat")]), "不能作为候补模型"),
        (with(vec![entry(ProviderId::Groq, "  ")]), "未填写模型"),
        (with(vec![entry(ProviderId::Groq, "whisper-large-v3"), entry(ProviderId::Groq, " whisper-large-v3 ")]), "已在列表中"),
        (with(vec![entry(ProviderId::Builtin, ""), entry(ProviderId::Builtin, "other")]), "已在列表中"),
        (with((0..9).map(|i| entry(ProviderId::Groq, &format!("m{i}"))).collect()), "最多 8 个候补模型"),
    ];
    for (settings, says) in refused {
        node.handle.send(CoreCommand::SetEngines(settings)).await.unwrap();
        let err = wait(&mut node, |e| if let CoreEvent::Error(m) = e { Some(m.clone()) } else { None }).await;
        assert!(err.contains("asr_fallback") && err.contains(says), "{says}: {err}");
    }
    let mut kept =
        with(vec![entry(ProviderId::Aliyun, "qwen-audio-3.1-asr-flash"), entry(ProviderId::Builtin, ""), entry(ProviderId::Groq, "whisper-large-v3")]);
    kept.llm_fallback = FallbackSettings { enabled: false, models: vec![entry(ProviderId::Ollama, "qwen3:8b")] };
    node.handle.send(CoreCommand::SetEngines(kept.clone())).await.unwrap();
    let saved = wait(&mut node, |e| if let CoreEvent::Settings(s) = e { Some(s.engines.clone()) } else { None }).await;
    assert_eq!(saved, kept, "the selected model in the list is kept: it is skipped, not refused");
    let status = engines_where(&mut node, |s| s.asr_fallback.models.len() == 3).await;
    assert_eq!(status.asr_fallback.models[0].skip, Some(voltip_core::FallbackSkip::SameAsSelected));
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}
