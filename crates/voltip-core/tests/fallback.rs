#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Fallback models through the real core task (docs/dictation.md §3.5): the settings are checked,
//! a model that runs out of quota shows on the engines status with its retry time, 重新检查
//! (`ResetQuota`) and a new key for its provider forget it, and the next take starts from the
//! selected model again.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use voltip_core::dictation::fakes::{FakeAudio, FakeInjector, FakeModels, FakeTranscriber, factory_by_model, ports_with};
use voltip_core::dictation::{LocalFailure, SwitchReason};
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
        asr_fallback: FallbackSettings {
            enabled: true,
            on_outage: false,
            models: vec![FallbackModel { provider: ProviderId::Aliyun, model: "qwen3-asr-flash".into() }],
        },
        ..EngineSettings::default()
    }
}

fn start(dir: &std::path::Path, selected: FakeTranscriber, fallback: FakeTranscriber) -> Node {
    SettingsStore::new(dir).save(&Settings { relay_enabled: false, engines: engines(), ..Settings::default() }).unwrap();
    let mut config = CoreConfig::new(dir.to_path_buf());
    config.default_device_name = "Fallback Test".into();
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
    let with = |models: Vec<FallbackModel>| EngineSettings { asr_fallback: FallbackSettings { enabled: true, on_outage: false, models }, ..engines() };
    let entry = |provider, model: &str| FallbackModel { provider, model: model.into() };
    let refused = [
        // No model library here (a phone): nothing of this device can stand in (docs/dictation.md §3.14).
        (with(vec![entry(ProviderId::Local, "qwen3-asr-0.6b")]), "不能作为候补模型"),
        (with(vec![entry(ProviderId::Deepseek, "deepseek-chat")]), "不能作为候补模型"),
        (with(vec![entry(ProviderId::Groq, "  ")]), "未填写模型"),
        (with(vec![entry(ProviderId::Groq, "whisper-large-v3"), entry(ProviderId::Groq, " whisper-large-v3 ")]), "已在列表中"),
        // The built-in service's models are told apart like any other provider's (it may offer
        // several, user request 2026-10-08): the same one twice is refused.
        (with(vec![entry(ProviderId::Builtin, "openai/gpt-oss-20b"), entry(ProviderId::Builtin, " openai/gpt-oss-20b ")]), "已在列表中"),
        (with(vec![entry(ProviderId::Builtin, ""), entry(ProviderId::Builtin, " ")]), "已在列表中"),
        (with((0..9).map(|i| entry(ProviderId::Groq, &format!("m{i}"))).collect()), "最多 8 个候补模型"),
    ];
    for (settings, says) in refused {
        node.handle.send(CoreCommand::SetEngines(settings)).await.unwrap();
        let err = wait(&mut node, |e| if let CoreEvent::Error(m) = e { Some(m.clone()) } else { None }).await;
        assert!(err.contains("asr_fallback") && err.contains(says), "{says}: {err}");
    }
    let mut kept =
        with(vec![entry(ProviderId::Aliyun, "qwen-audio-3.1-asr-flash"), entry(ProviderId::Builtin, ""), entry(ProviderId::Groq, "whisper-large-v3")]);
    kept.llm_fallback = FallbackSettings { enabled: false, on_outage: false, models: vec![entry(ProviderId::Ollama, "qwen3:8b")] };
    node.handle.send(CoreCommand::SetEngines(kept.clone())).await.unwrap();
    let saved = wait(&mut node, |e| if let CoreEvent::Settings(s) = e { Some(s.engines.clone()) } else { None }).await;
    assert_eq!(saved, kept, "the selected model in the list is kept: it is skipped, not refused");
    let status = engines_where(&mut node, |s| s.asr_fallback.models.len() == 3).await;
    assert_eq!(status.asr_fallback.models[0].skip, Some(voltip_core::FallbackSkip::SameAsSelected));
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// Recognition on this device (the fake library's 均衡, installed with Paraformer) with a service
/// and a model of this device after it, 服务出错时也改用候补 as `on_outage`; no clean-up. `local`
/// is the selected model's client, `paraformer` and `cloud` the fallback models'.
struct LocalNode {
    node: Node,
    paraformer: Arc<FakeTranscriber>,
    cloud: Arc<FakeTranscriber>,
}

fn local_engines(on_outage: bool, models: Vec<FallbackModel>) -> EngineSettings {
    EngineSettings {
        asr_provider: ProviderId::Local,
        local_model: Some("qwen3-asr-0.6b".into()),
        refine_enabled: false,
        asr_fallback: FallbackSettings { enabled: true, on_outage, models },
        ..EngineSettings::default()
    }
}

fn start_local(dir: &std::path::Path, engines: EngineSettings, local: FakeTranscriber, paraformer: FakeTranscriber, cloud: FakeTranscriber) -> LocalNode {
    SettingsStore::new(dir).save(&Settings { relay_enabled: false, engines, ..Settings::default() }).unwrap();
    let mut config = CoreConfig::new(dir.to_path_buf());
    config.default_device_name = "Fallback Test".into();
    let (local, paraformer, cloud) = (Arc::new(local), Arc::new(paraformer), Arc::new(cloud));
    let mut ports = ports_with(Arc::new(FakeAudio::speech()), local.clone(), None, Arc::new(FakeInjector::paste()));
    ports.factory = factory_by_model(&[("均衡", local.clone()), ("Paraformer 中文", paraformer.clone()), ("qwen3-asr-flash", cloud.clone())], &[]);
    ports.models = Some(Arc::new(FakeModels::new(1).with_installed("qwen3-asr-0.6b").with_installed("paraformer-zh")));
    let store = MemorySecretStore::new();
    voltip_identity::SecretStore::set(&store, "provider-key.aliyun", b"sk-test").unwrap();
    let (handle, events) = AppCore::start_with(config, Arc::new(store), ports).unwrap();
    LocalNode { node: Node { handle, events, selected: local, fallback: cloud.clone(), engines: None, newest: None }, paraformer, cloud }
}

fn entry(provider: ProviderId, model: &str) -> FallbackModel {
    FallbackModel { provider, model: model.into() }
}

fn device_failure() -> voltip_core::DictationError {
    voltip_core::DictationError::LocalFailed {
        failure: LocalFailure::Device, detail: "本地识别失败：显卡出错（vk lost），改用 CPU 也失败（vk lost）".into()
    }
}

/// docs/dictation.md §3.14: recognition on this device never sends the audio away for a failure.
/// The selected model's device fails: the model of this device after it recognises the take, the
/// service in the list is never asked, and the history says who did it and why. With nothing of
/// this device in the list the take fails without asking the service; with the switch off it fails
/// as before.
#[tokio::test]
async fn regression_a_local_primary_never_uploads_on_failure() {
    let dir = tempfile::tempdir().unwrap();
    let models = vec![entry(ProviderId::Aliyun, "qwen3-asr-flash"), entry(ProviderId::Local, "paraformer-zh")];
    // Paraformer reports its catalogue name, as the local recogniser does.
    let paraformer = FakeTranscriber::ok("本机候补的文字").with_model("Paraformer 中文");
    let mut n =
        start_local(dir.path(), local_engines(true, models.clone()), FakeTranscriber::failing(device_failure()), paraformer, FakeTranscriber::ok("云端"));
    let status = engines_where(&mut n.node, |s| s.asr_fallback.in_use).await;
    assert_eq!(status.asr_fallback.models[0].skip, Some(voltip_core::FallbackSkip::NotForLocal), "{:?}", status.asr_fallback);
    let done = take(&mut n.node).await;
    assert!(matches!(&done, DictationPhase::Done { text, .. } if text == "本机候补的文字"), "{done:?}");
    assert_eq!((n.node.selected.calls(), n.paraformer.calls(), n.cloud.calls()), (1, 1, 0), "the service in the list is never asked");
    let recorded = n.node.newest.clone().expect("the take's entry");
    assert_eq!(
        (recorded.asr_provider, recorded.asr_model.as_str(), recorded.asr_fallback),
        (Some(ProviderId::Local), "Paraformer 中文", Some(SwitchReason::Outage))
    );
    // The next take goes straight to it while the selected model cools down.
    take(&mut n.node).await;
    assert_eq!((n.node.selected.calls(), n.paraformer.calls(), n.cloud.calls()), (1, 2, 0));
    n.node.handle.send(CoreCommand::Shutdown).await.unwrap();

    // Only a service after the selected model of this device: the take fails, nothing uploaded.
    let dir = tempfile::tempdir().unwrap();
    let cloud_only = vec![entry(ProviderId::Aliyun, "qwen3-asr-flash")];
    let mut n = start_local(
        dir.path(),
        local_engines(true, cloud_only),
        FakeTranscriber::failing(device_failure()),
        FakeTranscriber::ok("x"),
        FakeTranscriber::ok("云端"),
    );
    engines_where(&mut n.node, |_| true).await;
    let done = take(&mut n.node).await;
    assert!(matches!(&done, DictationPhase::Failed { code: voltip_core::FailureCode::Asr, .. }), "{done:?}");
    assert_eq!(n.cloud.calls(), 0, "never the service");
    n.node.handle.send(CoreCommand::Shutdown).await.unwrap();

    // The switch off: as before, the failure is the answer.
    let dir = tempfile::tempdir().unwrap();
    let mut n = start_local(
        dir.path(),
        local_engines(false, models),
        FakeTranscriber::failing(device_failure()),
        FakeTranscriber::ok("x"),
        FakeTranscriber::ok("云端"),
    );
    engines_where(&mut n.node, |_| true).await;
    let done = take(&mut n.node).await;
    assert!(matches!(&done, DictationPhase::Failed { code: voltip_core::FailureCode::Asr, .. }), "{done:?}");
    assert_eq!((n.paraformer.calls(), n.cloud.calls()), (0, 0));
    n.node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// docs/dictation.md §3.14: a model of this device may be listed for recognition once the library
/// has it — installed or still to download; not the streaming model, not an id it does not know,
/// never for the clean-up; and a list is kept with 服务出错时也改用候补 on.
#[tokio::test]
async fn models_of_this_device_are_checked_against_the_library() {
    let dir = tempfile::tempdir().unwrap();
    let mut n = start_local(dir.path(), local_engines(false, Vec::new()), FakeTranscriber::ok("x"), FakeTranscriber::ok("y"), FakeTranscriber::ok("z"));
    engines_where(&mut n.node, |_| true).await;
    let with = |models: Vec<FallbackModel>| local_engines(true, models);
    let refused = [
        (with(vec![entry(ProviderId::Local, "zipformer-stream-zh-en")]), "asr_fallback", "不能作为候补模型"),
        (with(vec![entry(ProviderId::Local, "ghost")]), "asr_fallback", "不能作为候补模型"),
        (with(vec![entry(ProviderId::Local, " ")]), "asr_fallback", "未填写模型"),
        (
            EngineSettings {
                llm_fallback: FallbackSettings { enabled: true, on_outage: true, models: vec![entry(ProviderId::Local, "qwen3-asr-0.6b")] },
                ..with(Vec::new())
            },
            "llm_fallback",
            "不能作为候补模型",
        ),
    ];
    for (settings, field, says) in refused {
        n.node.handle.send(CoreCommand::SetEngines(settings)).await.unwrap();
        let err = wait(&mut n.node, |e| if let CoreEvent::Error(m) = e { Some(m.clone()) } else { None }).await;
        assert!(err.contains(field) && err.contains(says), "{says}: {err}");
    }
    let kept = with(vec![entry(ProviderId::Local, "paraformer-zh"), entry(ProviderId::Aliyun, "qwen3-asr-flash"), entry(ProviderId::Local, "qwen3-asr-0.6b")]);
    n.node.handle.send(CoreCommand::SetEngines(kept.clone())).await.unwrap();
    let saved = wait(&mut n.node, |e| if let CoreEvent::Settings(s) = e { Some(s.engines.clone()) } else { None }).await;
    assert_eq!(saved, kept);
    let status = engines_where(&mut n.node, |s| s.asr_fallback.models.len() == 3).await;
    let skips: Vec<_> = status.asr_fallback.models.iter().map(|m| m.skip).collect();
    assert_eq!(skips, [None, Some(voltip_core::FallbackSkip::NotForLocal), Some(voltip_core::FallbackSkip::SameAsSelected)], "{:?}", status.asr_fallback);
    n.node.handle.send(CoreCommand::Shutdown).await.unwrap();
}
