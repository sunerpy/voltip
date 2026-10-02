#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The local model library through the real core task (docs/dictation.md §10): `Models` on
//! `Ready`, `ModelDownload` → progress → `Installed` → `local_ready`, cancel, failure, removal,
//! the `active` flag following `SetEngines`, and the refusals (unknown id, installed, no library).

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use voltip_core::dictation::DictationPorts;
use voltip_core::dictation::fakes::{FakeAudio, FakeInjector, FakeModels, FakeTranscriber, ports_with};
use voltip_core::{
    AppCore, CoreCommand, CoreConfig, CoreEvent, CoreHandle, DEFAULT_LOCAL_MODEL_ID, DictationPhase, EngineSettings, EngineStatus, ModelInstallState,
    ModelState, ProviderId, ProviderSettings, ResolvedEngines, Settings, SettingsStore,
};
use voltip_identity::MemorySecretStore;

const STEP: Duration = Duration::from_secs(10);

struct Node {
    handle: CoreHandle,
    events: mpsc::Receiver<CoreEvent>,
}

fn config(dir: &std::path::Path, engines: EngineSettings) -> CoreConfig {
    SettingsStore::new(dir).save(&Settings { relay_enabled: false, engines, ..Settings::default() }).unwrap();
    let mut cfg = CoreConfig::new(dir.to_path_buf());
    cfg.default_device_name = "Models Test".into();
    cfg.direct_enabled = false;
    cfg
}

fn start(dir: &std::path::Path, models: Option<Arc<FakeModels>>) -> Node {
    start_with(dir, models, remote())
}

/// Recognition on a remote endpoint (the test build has no built-in service, whose absence would
/// fall back to the on-device model).
fn remote() -> EngineSettings {
    EngineSettings {
        asr_provider: ProviderId::Custom,
        providers: [(
            ProviderId::Custom,
            ProviderSettings { asr_url: Some("https://asr.example.test".into()), asr_model: Some("m".into()), ..Default::default() },
        )]
        .into(),
        ..EngineSettings::default()
    }
}

fn start_with(dir: &std::path::Path, models: Option<Arc<FakeModels>>, engines: EngineSettings) -> Node {
    let mut ports: DictationPorts = ports_with(Arc::new(FakeAudio::speech()), Arc::new(FakeTranscriber::ok("本地")), None, Arc::new(FakeInjector::paste()));
    ports.models = models.map(|m| m as Arc<dyn voltip_core::ModelManager>);
    let (handle, events) = AppCore::start_with(config(dir, engines), Arc::new(MemorySecretStore::new()), ports).unwrap();
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

async fn wait_models(node: &mut Node, mut pred: impl FnMut(&[ModelState]) -> bool) -> Vec<ModelState> {
    wait(node, |e| match e {
        CoreEvent::Models(m) if pred(m) => Some(m.clone()),
        _ => None,
    })
    .await
}

async fn wait_engines(node: &mut Node, mut pred: impl FnMut(&EngineStatus) -> bool) -> EngineStatus {
    wait(node, |e| match e {
        CoreEvent::Engines(s) if pred(s) => Some(s.clone()),
        _ => None,
    })
    .await
}

async fn wait_error(node: &mut Node, needle: &str) -> String {
    wait(node, |e| match e {
        CoreEvent::Error(m) if m.contains(needle) => Some(m.clone()),
        _ => None,
    })
    .await
}

fn state_of<'a>(models: &'a [ModelState], id: &str) -> &'a ModelInstallState {
    &models.iter().find(|m| m.id == id).unwrap_or_else(|| panic!("{id} in {models:?}")).state
}

fn local(id: Option<&str>) -> EngineSettings {
    EngineSettings { asr_provider: ProviderId::Local, local_model: id.map(str::to_owned), ..EngineSettings::default() }
}

#[tokio::test]
async fn download_progress_install_active_and_readiness_through_the_core() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeModels::new(4));
    let mut node = start(dir.path(), Some(fake.clone()));

    // Ready → Engines → Models: the catalogue, nothing installed, nothing active (remote ASR).
    wait(&mut node, |e| matches!(e, CoreEvent::Ready { .. }).then_some(())).await;
    let engines = wait_engines(&mut node, |_| true).await;
    assert_eq!(engines.asr_provider, ProviderId::Custom);
    assert!(!engines.local_ready);
    let models = wait_models(&mut node, |_| true).await;
    assert_eq!(models.len(), 3);
    assert!(models.iter().all(|m| m.state == ModelInstallState::NotInstalled && !m.active));
    assert_eq!(models[0].id, DEFAULT_LOCAL_MODEL_ID);
    assert!(models[0].recommended);
    assert_eq!(models[0].tier, "balanced");
    assert_eq!(models[0].capabilities, ["offline"]);
    assert!(models[2].is_streaming() && models[2].tier == "streaming");
    assert!(!engines.live_preview_ready, "live preview waits for the streaming model");

    // Switch to local mode before anything is installed: allowed, `active` marks the default
    // model, `local_ready` is false and a start is refused with the documented reason.
    node.handle.send(CoreCommand::SetEngines(local(None))).await.unwrap();
    let engines = wait_engines(&mut node, |s| s.asr_provider == ProviderId::Local).await;
    assert_eq!(engines.asr_host, "", "no host on-device");
    assert_eq!(engines.local_model.as_deref(), Some(DEFAULT_LOCAL_MODEL_ID));
    assert!(!engines.local_ready);
    assert_eq!(engines.asr_model, "均衡");
    let models = wait_models(&mut node, |m| m[0].active).await;
    assert!(!models[1].active);
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    let err = wait_error(&mut node, "本地模型未下载").await;
    assert!(err.contains("均衡"), "{err}");

    // Download: `Downloading` at once, progress, then `Installed`; the engines flip to ready.
    node.handle.send(CoreCommand::ModelDownload(DEFAULT_LOCAL_MODEL_ID.into())).await.unwrap();
    let models = wait_models(&mut node, |m| matches!(state_of(m, DEFAULT_LOCAL_MODEL_ID), ModelInstallState::Downloading { .. })).await;
    assert!(matches!(state_of(&models, DEFAULT_LOCAL_MODEL_ID), ModelInstallState::Downloading { total: 4096, .. }));
    // The rescan after the download re-resolves the engines first, then republishes the list.
    let engines = wait_engines(&mut node, |s| s.local_ready).await;
    assert!(engines.asr_ready && engines.asr_issue.is_none());
    let models = wait_models(&mut node, |m| state_of(m, DEFAULT_LOCAL_MODEL_ID).is_installed()).await;
    assert!(matches!(state_of(&models, DEFAULT_LOCAL_MODEL_ID), ModelInstallState::Installed { path, .. } if path.contains("qwen3-asr-0.6b")));
    assert!(models[0].active);
    assert_eq!(fake.downloads(), 1);
    assert!(!engines.live_preview_ready, "an offline model does not make live preview ready");

    // Installing the streaming model flips `live_preview_ready` (the setting defaults to on);
    // switching the setting off flips it back without touching the library.
    node.handle.send(CoreCommand::ModelDownload("zipformer-stream-zh-en".into())).await.unwrap();
    let engines = wait_engines(&mut node, |s| s.live_preview_ready).await;
    assert!(engines.local_ready, "unrelated to local readiness, which stays");
    let models = wait_models(&mut node, |m| state_of(m, "zipformer-stream-zh-en").is_installed()).await;
    assert!(!models[2].active, "the streaming model is never the active (final-text) model");
    node.handle.send(CoreCommand::SetEngines(EngineSettings { live_preview: false, ..local(None) })).await.unwrap();
    let engines = wait_engines(&mut node, |s| !s.live_preview_ready).await;
    assert!(engines.local_ready);
    node.handle.send(CoreCommand::SetEngines(local(None))).await.unwrap();
    wait_engines(&mut node, |s| s.live_preview_ready).await;

    // Refusals: downloading an installed model, an unknown id, cancelling nothing.
    node.handle.send(CoreCommand::ModelDownload(DEFAULT_LOCAL_MODEL_ID.into())).await.unwrap();
    wait_error(&mut node, "已安装").await;
    node.handle.send(CoreCommand::ModelDownload("ghost".into())).await.unwrap();
    wait_error(&mut node, "目录中没有 ghost").await;
    node.handle.send(CoreCommand::ModelCancel("paraformer-zh".into())).await.unwrap();
    wait_error(&mut node, "没有在下载").await;
    node.handle.send(CoreCommand::SetEngines(local(Some("ghost")))).await.unwrap();
    wait_error(&mut node, "local_model: 目录中没有 ghost").await;

    // A dictation now runs through the (fake) local transcriber.
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    wait(&mut node, |e| matches!(e, CoreEvent::Dictation(s) if matches!(s.phase, DictationPhase::Listening { .. })).then_some(())).await;
    node.handle.send(CoreCommand::DictationStop).await.unwrap();
    let done = wait(&mut node, |e| match e {
        CoreEvent::Dictation(s) if s.phase.is_terminal() => Some(s.phase.clone()),
        _ => None,
    })
    .await;
    assert!(matches!(&done, DictationPhase::Done { text, .. } if text == "本地"), "{done:?}");
    let history = wait(&mut node, |e| if let CoreEvent::History { recent: h, .. } = e { (!h.is_empty()).then(|| h.clone()) } else { None }).await;
    assert_eq!(history[0].asr_model, "均衡", "the history records the model's display name");

    // Selecting the other model: `active` moves, readiness drops (it is not installed).
    node.handle.send(CoreCommand::SetEngines(local(Some("paraformer-zh")))).await.unwrap();
    let engines = wait_engines(&mut node, |s| s.local_model.as_deref() == Some("paraformer-zh")).await;
    assert!(!engines.local_ready);
    assert_eq!(engines.asr_model, "Paraformer 中文");
    let models = wait_models(&mut node, |m| m[1].active).await;
    assert!(!models[0].active);

    // Remove the installed one: back to `NotInstalled`; removing while downloading is refused.
    node.handle.send(CoreCommand::ModelRemove(DEFAULT_LOCAL_MODEL_ID.into())).await.unwrap();
    let models = wait_models(&mut node, |m| state_of(m, DEFAULT_LOCAL_MODEL_ID) == &ModelInstallState::NotInstalled).await;
    assert_eq!(fake.removals(), 1);
    assert!(!models[0].active);

    // Back to the remote endpoint: nothing active, `local_model` gone from the status.
    node.handle.send(CoreCommand::SetEngines(remote())).await.unwrap();
    let engines = wait_engines(&mut node, |s| s.asr_provider == ProviderId::Custom).await;
    assert_eq!(engines.local_model, None);
    assert!(!engines.local_ready);
    wait_models(&mut node, |m| m.iter().all(|m| !m.active)).await;
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

#[tokio::test]
async fn cancel_and_failure_keep_the_library_honest() {
    let dir = tempfile::tempdir().unwrap();
    let hanging = Arc::new(FakeModels::hanging());
    let mut node = start(dir.path(), Some(hanging.clone()));
    wait_models(&mut node, |_| true).await;
    node.handle.send(CoreCommand::ModelDownload("paraformer-zh".into())).await.unwrap();
    wait_models(&mut node, |m| matches!(state_of(m, "paraformer-zh"), ModelInstallState::Downloading { .. })).await;
    // Removing a model mid-download is refused; cancelling returns it to `NotInstalled`.
    node.handle.send(CoreCommand::ModelRemove("paraformer-zh".into())).await.unwrap();
    wait_error(&mut node, "正在下载").await;
    node.handle.send(CoreCommand::ModelDownload("paraformer-zh".into())).await.unwrap();
    wait_error(&mut node, "正在下载").await;
    node.handle.send(CoreCommand::ModelCancel("paraformer-zh".into())).await.unwrap();
    wait_models(&mut node, |m| state_of(m, "paraformer-zh") == &ModelInstallState::NotInstalled).await;
    assert_eq!(hanging.downloads(), 1);
    node.handle.send(CoreCommand::Shutdown).await.unwrap();

    // A failed verification is reported with its reason and survives the rescan.
    let dir = tempfile::tempdir().unwrap();
    let failing = Arc::new(FakeModels::failing("sha256 mismatch: model.int8.onnx"));
    let mut node = start(dir.path(), Some(failing));
    wait_models(&mut node, |_| true).await;
    node.handle.send(CoreCommand::ModelDownload(DEFAULT_LOCAL_MODEL_ID.into())).await.unwrap();
    let models = wait_models(&mut node, |m| matches!(state_of(m, DEFAULT_LOCAL_MODEL_ID), ModelInstallState::Failed { .. })).await;
    assert_eq!(state_of(&models, DEFAULT_LOCAL_MODEL_ID), &ModelInstallState::Failed { message: "sha256 mismatch: model.int8.onnx".into() });
    // Another change (selecting it) re-emits the list; the failure is still there.
    node.handle.send(CoreCommand::SetEngines(local(None))).await.unwrap();
    let models = wait_models(&mut node, |m| m[0].active).await;
    assert!(matches!(state_of(&models, DEFAULT_LOCAL_MODEL_ID), ModelInstallState::Failed { .. }));
    // A retry is allowed from `Failed` (it fails again here) …
    node.handle.send(CoreCommand::ModelDownload(DEFAULT_LOCAL_MODEL_ID.into())).await.unwrap();
    wait_models(&mut node, |m| matches!(state_of(m, DEFAULT_LOCAL_MODEL_ID), ModelInstallState::Downloading { .. })).await;
    wait_models(&mut node, |m| matches!(state_of(m, DEFAULT_LOCAL_MODEL_ID), ModelInstallState::Failed { .. })).await;
    // … and removal clears it.
    node.handle.send(CoreCommand::ModelRemove(DEFAULT_LOCAL_MODEL_ID.into())).await.unwrap();
    wait_models(&mut node, |m| state_of(m, DEFAULT_LOCAL_MODEL_ID) == &ModelInstallState::NotInstalled).await;
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// Shells without a model library (the phone, the fakes): the list is empty, local mode cannot be
/// selected, and the three model commands are refused with one honest reason.
#[tokio::test]
async fn without_a_library_models_are_empty_and_the_commands_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut node = start(dir.path(), None);
    let models = wait_models(&mut node, |_| true).await;
    assert!(models.is_empty());
    node.handle.send(CoreCommand::SetEngines(local(None))).await.unwrap();
    wait_error(&mut node, "本地模型不可用").await;
    for cmd in [
        CoreCommand::ModelDownload(DEFAULT_LOCAL_MODEL_ID.into()),
        CoreCommand::ModelCancel(DEFAULT_LOCAL_MODEL_ID.into()),
        CoreCommand::ModelRemove(DEFAULT_LOCAL_MODEL_ID.into()),
    ] {
        node.handle.send(cmd).await.unwrap();
        wait_error(&mut node, "models: 本地模型不可用").await;
    }
    // A pre-installed model is ready from the first frame.
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut node = start_with(dir.path(), Some(Arc::new(FakeModels::new(1).with_installed(DEFAULT_LOCAL_MODEL_ID))), local(None));
    let engines = wait_engines(&mut node, |_| true).await;
    assert!(engines.local_ready, "{engines:?}");
    assert_eq!((engines.asr_provider, engines.asr_host.as_str()), (ProviderId::Local, ""));
    let models = wait_models(&mut node, |_| true).await;
    assert!(models[0].active && models[0].state.is_installed());
    let r = ResolvedEngines::resolve_with_models(&local(None), &Default::default(), &voltip_core::BuiltIn::EMPTY, &models);
    assert!(r.status().local_ready);
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// The manual import through the core (docs/dictation.md §10, user request 2026-10-02): a card
/// names its folder and files; an import that finds the folder incomplete keeps the names until
/// the person fixes it, and the next import installs and readies the engine.
#[tokio::test]
async fn a_manual_import_names_what_is_missing_until_the_files_are_right() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeModels::new(1).with_import_problems(&["tokens.txt"], &["model.int8.onnx"]));
    let mut node = start_with(dir.path(), Some(fake.clone()), local(None));
    let models = wait_models(&mut node, |_| true).await;
    let card = models.iter().find(|m| m.id == DEFAULT_LOCAL_MODEL_ID).unwrap();
    assert_eq!(card.dir, format!("/fake/models/{DEFAULT_LOCAL_MODEL_ID}"));
    assert_eq!(card.files.len(), 1);
    assert_eq!(card.files[0].urls.len(), 2, "huggingface.co and hf-mirror.com");

    node.handle.send(CoreCommand::ModelImport(DEFAULT_LOCAL_MODEL_ID.into())).await.unwrap();
    wait_models(&mut node, |m| *state_of(m, DEFAULT_LOCAL_MODEL_ID) == ModelInstallState::Verifying).await;
    let models = wait_models(&mut node, |m| matches!(state_of(m, DEFAULT_LOCAL_MODEL_ID), ModelInstallState::ImportIncomplete { .. })).await;
    assert_eq!(
        *state_of(&models, DEFAULT_LOCAL_MODEL_ID),
        ModelInstallState::ImportIncomplete { missing: vec!["tokens.txt".into()], mismatched: vec!["model.int8.onnx".into()] }
    );
    // A rescan (any settings change) keeps what the import found: the disk only says "not installed".
    node.handle.send(CoreCommand::SetEngines(local(Some("paraformer-zh")))).await.unwrap();
    let models = wait_models(&mut node, |m| m.iter().any(|x| x.id == "paraformer-zh" && x.active)).await;
    assert!(matches!(state_of(&models, DEFAULT_LOCAL_MODEL_ID), ModelInstallState::ImportIncomplete { .. }));
    node.handle.send(CoreCommand::SetEngines(local(None))).await.unwrap();

    fake.fix_import();
    node.handle.send(CoreCommand::ModelImport(DEFAULT_LOCAL_MODEL_ID.into())).await.unwrap();
    // The rescan reports the engines before the models: wait for both, in whatever order.
    let (mut installed, mut ready) = (false, false);
    wait(&mut node, |e| {
        match e {
            CoreEvent::Models(m) => installed |= state_of(m, DEFAULT_LOCAL_MODEL_ID).is_installed(),
            CoreEvent::Engines(s) => ready |= s.local_ready,
            _ => {}
        }
        (installed && ready).then_some(())
    })
    .await;
    assert_eq!(fake.imports(), 2);
    // An installed model is not imported again.
    node.handle.send(CoreCommand::ModelImport(DEFAULT_LOCAL_MODEL_ID.into())).await.unwrap();
    wait_error(&mut node, "已安装").await;
    node.handle.send(CoreCommand::ModelImport("nope".into())).await.unwrap();
    wait_error(&mut node, "目录中没有 nope").await;
    assert_eq!(fake.imports(), 2);
}
