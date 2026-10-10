#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Scenes and context (docs/dictation.md §18) through the real core task: the four scene commands
//! and the context-sharing switch in, `Scenes` / `Settings` / `Error` events out, `scenes.json` on
//! disk, a restart that reads it back, a quarantined file, and takes on the fakes whose probe
//! answer picks the scene — refine switched off by it, the refiner told the app, the history
//! recording app and scene.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use voltip_core::dictation::fakes::{FAKE_TRANSCRIPT, FakeAudio, FakeInjector, FakeProbe, FakeRefiner, FakeTranscriber};
use voltip_core::dictation::{DictationPorts, Refiner, Transcriber};
use voltip_core::scenes::{MAX_RECENT_APPS, SCENES_FILE_NAME};
use voltip_core::{
    AppCore, AppRef, ContextSharing, CoreCommand, CoreConfig, CoreEvent, CoreHandle, DictationPhase, DictationStatus, HistoryEntry, Scene, SceneDraft,
    SceneMatch, SceneOverrides, Settings, SettingsStore,
};
use voltip_identity::MemorySecretStore;

const STEP: Duration = Duration::from_secs(10);

struct Node {
    handle: CoreHandle,
    events: mpsc::Receiver<CoreEvent>,
}

/// A core whose list holds the user's scenes alone (these tests are about theirs; the built-in
/// ones, docs/dictation.md §18.10, have their own test below).
fn start(dir: &std::path::Path, probe: Arc<FakeProbe>, refiner: Arc<FakeRefiner>) -> Node {
    start_with(dir, probe, refiner, false)
}

fn start_with(dir: &std::path::Path, probe: Arc<FakeProbe>, refiner: Arc<FakeRefiner>, builtin_scenes: bool) -> Node {
    if !dir.join(voltip_core::SETTINGS_FILE_NAME).exists() {
        SettingsStore::new(dir)
            .save(&Settings { relay_enabled: false, engines: voltip_core::dictation::fakes::fake_engines(), ..Settings::default() })
            .unwrap();
    }
    let mut cfg = CoreConfig::new(dir.to_path_buf());
    cfg.default_device_name = "Scenes Test".into();
    cfg.builtin_scenes = builtin_scenes;
    let ports = DictationPorts {
        audio: Arc::new(FakeAudio::speech()),
        injector: Arc::new(FakeInjector::paste()),
        // Refine is on by default; the fake refiner stands in for the configured one.
        factory: Arc::new(move |_| (Arc::new(FakeTranscriber::ok(FAKE_TRANSCRIPT)) as Arc<dyn Transcriber>, Some(refiner.clone() as Arc<dyn Refiner>))),
        models: None,
        streaming: None,
        probe: Some(probe),
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

async fn scenes(node: &mut Node) -> Vec<Scene> {
    wait(node, |e| if let CoreEvent::Scenes(s) = e { Some(s.clone()) } else { None }).await
}

async fn error(node: &mut Node) -> String {
    wait(node, |e| if let CoreEvent::Error(m) = e { Some(m.clone()) } else { None }).await
}

fn draft(name: &str, apps: &[&str], overrides: SceneOverrides) -> SceneDraft {
    SceneDraft {
        name: name.into(),
        enabled: true,
        matching: SceneMatch { apps: apps.iter().map(|a| (*a).to_owned()).collect(), title_contains: Vec::new() },
        overrides,
    }
}

/// One take: start, wait for listening, stop, return the terminal status and the new history.
async fn take(node: &mut Node) -> (DictationStatus, Vec<HistoryEntry>) {
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    wait(node, |e| matches!(e, CoreEvent::Dictation(s) if matches!(s.phase, DictationPhase::Listening { ready: true, .. })).then_some(())).await;
    node.handle.send(CoreCommand::DictationStop).await.unwrap();
    let done = wait(node, |e| match e {
        CoreEvent::Dictation(s) if s.phase.is_terminal() => Some(s.clone()),
        _ => None,
    })
    .await;
    let history = wait(node, |e| if let CoreEvent::History { recent: h, .. } = e { (!h.is_empty()).then(|| h.clone()) } else { None }).await;
    // Dismiss the dwell so the next start is not busy.
    node.handle.send(CoreCommand::DictationCancel).await.unwrap();
    wait(node, |e| matches!(e, CoreEvent::Dictation(s) if s.phase == DictationPhase::Idle).then_some(())).await;
    (done, history)
}

/// Every command with its event, the refusals (as `error` events, the list unchanged), the
/// context switch persisted, takes the scene really changes, and a restart that reads it all back.
#[tokio::test]
async fn scenes_and_context_through_the_core() {
    let dir = tempfile::tempdir().unwrap();
    let probe = Arc::new(FakeProbe::app("Slack.exe", "Slack", Some("#dev · Voltip")));
    let refiner = Arc::new(FakeRefiner::ok("润色后的文本"));
    let mut node = start(dir.path(), probe.clone(), refiner.clone());
    assert!(scenes(&mut node).await.is_empty(), "Ready announces the (empty) list");

    node.handle
        .send(CoreCommand::SceneAdd(draft(
            "聊天",
            &["slack"],
            SceneOverrides { refine_enabled: Some(false), prompt: Some("口语化".into()), ..Default::default() },
        )))
        .await
        .unwrap();
    let list = scenes(&mut node).await;
    assert_eq!((list.len(), list[0].name.as_str(), list[0].matching.apps.clone()), (1, "聊天", vec!["slack".to_owned()]));
    node.handle.send(CoreCommand::SceneAdd(draft("聊天", &["wechat"], SceneOverrides::default()))).await.unwrap();
    assert!(error(&mut node).await.starts_with("scenes: 已有名为「聊天」"), "a duplicate name is refused");
    node.handle.send(CoreCommand::SceneAdd(draft("文档", &["winword"], SceneOverrides { prompt: Some("书面语".into()), ..Default::default() }))).await.unwrap();
    let list = scenes(&mut node).await;
    node.handle.send(CoreCommand::SceneReorder(vec![list[1].id, list[0].id])).await.unwrap();
    assert_eq!(scenes(&mut node).await[0].name, "文档");
    node.handle.send(CoreCommand::SceneReorder(vec![list[0].id])).await.unwrap();
    assert!(error(&mut node).await.contains("全部场景"));
    node.handle.send(CoreCommand::SceneRemove(uuid::Uuid::new_v4())).await.unwrap();
    assert!(error(&mut node).await.contains("没有 id"));

    // A take in Slack: the scene switches refine off, so nothing goes to the LLM.
    let (done, history) = take(&mut node).await;
    assert!(matches!(&done.phase, DictationPhase::Done { refined: false, text, .. } if text == FAKE_TRANSCRIPT), "{done:?}");
    assert_eq!(done.context.as_ref().and_then(|c| c.scene.as_ref()).map(|s| s.name.as_str()), Some("聊天"));
    assert_eq!(history[0].app, Some(AppRef { id: "slack".into(), name: "Slack".into() }));
    assert_eq!(history[0].scene.as_ref().map(|s| s.id), Some(list[0].id));
    assert_eq!(refiner.calls(), 0);

    // Edit the scene to refine with its instruction; share the title, not the name.
    let chat = list[0].clone();
    node.handle
        .send(CoreCommand::SceneUpdate {
            id: chat.id,
            draft: draft("聊天", &["slack"], SceneOverrides { prompt: Some("口语化".into()), ..Default::default() }),
        })
        .await
        .unwrap();
    assert!(scenes(&mut node).await.iter().any(|s| s.id == chat.id && s.overrides.refine_enabled.is_none()));
    node.handle.send(CoreCommand::SetContextSharing(ContextSharing { app_name: false, window_title: true })).await.unwrap();
    let settings = wait(&mut node, |e| if let CoreEvent::Settings(s) = e { Some(s.clone()) } else { None }).await;
    assert_eq!(settings.context_sharing, ContextSharing { app_name: false, window_title: true });
    let (done, _) = take(&mut node).await;
    assert!(matches!(&done.phase, DictationPhase::Done { refined: true, .. }), "{done:?}");
    let hints = refiner.hints();
    assert_eq!(hints.len(), 1);
    assert_eq!(
        (hints[0].context.app_name.as_deref(), hints[0].context.window_title.as_deref(), hints[0].context.instruction.as_deref()),
        (None, Some("#dev · Voltip"), Some("口语化"))
    );

    // Another app: no scene; the history database knows both apps, newest first (what the
    // bridge's reader answers the scene editor with).
    probe.set_app("code", "Code", None);
    let (done, _) = take(&mut node).await;
    assert_eq!(done.context.as_ref().map(|c| (c.app.id.as_str(), c.scene.is_none())), Some(("code", true)));
    let reader = voltip_core::HistoryReader::new(dir.path());
    let apps: Vec<String> = reader.recent_apps(MAX_RECENT_APPS).unwrap().into_iter().map(|a| a.id).collect();
    assert_eq!(apps, ["code", "slack"]);

    // Everything is on disk; a restarted core reads the scenes and the switch back.
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
    drop(node);
    let saved = std::fs::read_to_string(dir.path().join(SCENES_FILE_NAME)).unwrap();
    assert!(saved.contains("\"schema\": 1") && saved.contains("\"match\""), "{saved}");
    let mut node = start(dir.path(), probe.clone(), refiner.clone());
    let ready = wait(&mut node, |e| if let CoreEvent::Ready { settings, .. } = e { Some(settings.clone()) } else { None }).await;
    assert_eq!(ready.context_sharing, ContextSharing { app_name: false, window_title: true });
    let reloaded = scenes(&mut node).await;
    assert_eq!(reloaded.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["文档", "聊天"]);
    node.handle.send(CoreCommand::SceneRemove(reloaded[0].id)).await.unwrap();
    assert_eq!(scenes(&mut node).await.len(), 1);
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// A corrupt `scenes.json` does not stop the core: it starts with no scenes, the file is kept as
/// `scenes.json.corrupt-<secs>`, and the UI hears about it after `Ready`; takes run on the globals.
#[tokio::test]
async fn regression_a_corrupt_scenes_file_is_set_aside_and_reported_after_ready() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(SCENES_FILE_NAME), b"[not the schema]").unwrap();
    let mut node = start(dir.path(), Arc::new(FakeProbe::app("slack", "Slack", None)), Arc::new(FakeRefiner::ok("x")));
    wait(&mut node, |e| matches!(e, CoreEvent::Ready { .. }).then_some(())).await;
    assert!(scenes(&mut node).await.is_empty());
    let notice = error(&mut node).await;
    assert!(notice.contains("scenes.json 无法使用") && notice.contains(".corrupt-"), "{notice}");
    assert!(!dir.path().join(SCENES_FILE_NAME).exists());
    let (done, history) = take(&mut node).await;
    assert!(matches!(done.phase, DictationPhase::Done { refined: true, .. }), "the globals: {done:?}");
    assert_eq!(history[0].scene, None);
    assert_eq!(history[0].app.as_ref().map(|a| a.id.as_str()), Some("slack"));
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// §18.10 through the core: a desktop's list starts with the seven built-in scenes, switched off;
/// deleting one is refused with an `error` event, a take in one that is on names it in the status
/// and the history, 恢复默认 puts its defaults back, and a restart finds the same ids.
#[tokio::test]
async fn builtin_scenes_through_the_core() {
    use voltip_core::BuiltinScene;
    let dir = tempfile::tempdir().unwrap();
    let probe = Arc::new(FakeProbe::app("nothing", "Nothing", None));
    let refiner = Arc::new(FakeRefiner::ok("润色后的文本"));
    let mut node = start_with(dir.path(), probe.clone(), refiner.clone(), true);
    let list = scenes(&mut node).await;
    assert_eq!(list.iter().map(|s| s.builtin).collect::<Vec<_>>(), BuiltinScene::ALL.map(Some));
    assert!(list.iter().all(|s| !s.enabled));
    let coding = list[0].clone();
    let Some(app) = coding.matching.apps.first().cloned() else {
        // A host with no default applications for 编程开发 (not a desktop): nothing more to see.
        return;
    };
    node.handle.send(CoreCommand::SceneRemove(coding.id)).await.unwrap();
    assert_eq!(error(&mut node).await, "scenes: 内置场景不能删除，可以关闭");
    let mut on = SceneDraft::from(&coding);
    on.enabled = true;
    on.matching.apps = vec![app.clone()];
    node.handle.send(CoreCommand::SceneUpdate { id: coding.id, draft: on }).await.unwrap();
    let list = scenes(&mut node).await;
    assert_eq!(list[0].matching.apps, std::slice::from_ref(&app));
    probe.set_app(&app, "Editor", None);
    let (done, history) = take(&mut node).await;
    assert_eq!(done.context.and_then(|c| c.scene).map(|s| s.builtin), Some(Some(BuiltinScene::Coding)));
    assert_eq!(history[0].scene.as_ref().map(|s| (s.name.as_str(), s.builtin)), Some(("coding", Some(BuiltinScene::Coding))));
    node.handle.send(CoreCommand::SceneRestore(coding.id)).await.unwrap();
    let restored = scenes(&mut node).await;
    assert_eq!((restored[0].enabled, &restored[0].matching), (true, &coding.matching));
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
    drop(node);
    let mut node = start_with(dir.path(), probe, refiner, true);
    let again = scenes(&mut node).await;
    assert_eq!(again.iter().map(|s| s.id).collect::<Vec<_>>(), restored.iter().map(|s| s.id).collect::<Vec<_>>());
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}
