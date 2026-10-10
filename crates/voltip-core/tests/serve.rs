#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The local speech service the app hosts (docs/dictation.md §23.6), through the real core task
//! with a fake host: switched on and off by `SetServe`, its status in `Serve` events, the token on
//! the clipboard and replaced, the processing it runs with following the app's state, a port that
//! is taken, a restart that brings it back, and a shell without a host that refuses it.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::mpsc;
use voltip_core::dictation::fakes::{FAKE_TRANSCRIPT, FakeAudio, FakeInjector, FakeRefiner, FakeTranscriber};
use voltip_core::dictation::{DictationPorts, Refiner, Transcriber};
use voltip_core::presets::{BuiltinPreset, PresetDraft, PresetId};
use voltip_core::serve::{ListenConfig, PcmFile, RunningServer, ServeHost, ServeRequest, SpeechService};
use voltip_core::ui::{ServePhase, ServeStatus};
use voltip_core::{AppCore, CancelToken, CoreCommand, CoreConfig, CoreEvent, CoreHandle, SERVE_UNAVAILABLE, ServeSettings, Settings, SettingsStore};
use voltip_identity::MemorySecretStore;

const STEP: Duration = Duration::from_secs(10);

#[derive(Default)]
struct Started {
    configs: Vec<ListenConfig>,
    services: Vec<Arc<dyn SpeechService>>,
    tokens: Vec<String>,
    stops: usize,
}

/// A host that binds nothing: it records what it was asked and can refuse (a taken port).
#[derive(Debug, Default)]
struct FakeHost {
    log: Arc<Mutex<Started>>,
    refuse: Arc<Mutex<bool>>,
}

impl std::fmt::Debug for Started {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Started").field("configs", &self.configs.len()).finish_non_exhaustive()
    }
}

struct FakeRunning {
    addr: SocketAddr,
    log: Arc<Mutex<Started>>,
}

impl RunningServer for FakeRunning {
    fn addr(&self) -> SocketAddr {
        self.addr
    }

    fn set_token(&self, token: String) {
        self.log.lock().tokens.push(token);
    }

    fn stop(&self) {
        self.log.lock().stops += 1;
    }
}

#[async_trait]
impl ServeHost for FakeHost {
    async fn start(&self, config: ListenConfig, service: Arc<dyn SpeechService>) -> Result<Box<dyn RunningServer>, String> {
        if *self.refuse.lock() {
            return Err(format!("无法监听 {}：Address already in use", config.addr));
        }
        let addr = config.addr;
        let mut log = self.log.lock();
        log.configs.push(config);
        log.services.push(service);
        Ok(Box::new(FakeRunning { addr, log: self.log.clone() }))
    }
}

struct Node {
    handle: CoreHandle,
    events: mpsc::Receiver<CoreEvent>,
    injector: Arc<FakeInjector>,
}

fn start(dir: &Path, host: Option<Arc<FakeHost>>) -> Node {
    if !dir.join(voltip_core::SETTINGS_FILE_NAME).exists() {
        SettingsStore::new(dir)
            .save(&Settings { relay_enabled: false, engines: voltip_core::dictation::fakes::fake_engines(), ..Settings::default() })
            .unwrap();
    }
    let mut cfg = CoreConfig::new(dir.to_path_buf());
    cfg.default_device_name = "Serve Test".into();
    cfg.serve_host = host.map(|h| h as Arc<dyn ServeHost>);
    let injector = Arc::new(FakeInjector::paste());
    let ports = DictationPorts {
        audio: Arc::new(FakeAudio::speech()),
        injector: injector.clone(),
        factory: Arc::new(|_| {
            (
                Arc::new(FakeTranscriber::ok(FAKE_TRANSCRIPT)) as Arc<dyn Transcriber>,
                Some(Arc::new(FakeRefiner::ok("你好，世界。整理好了。")) as Arc<dyn Refiner>),
            )
        }),
        models: None,
        streaming: None,
        probe: None,
        service_probe: None,
        segmenter: None,
    };
    let (handle, events) = AppCore::start_with(cfg, Arc::new(MemorySecretStore::new()), ports).unwrap();
    Node { handle, events, injector }
}

async fn wait<T>(node: &mut Node, mut pick: impl FnMut(&CoreEvent) -> Option<T>) -> T {
    loop {
        let ev = tokio::time::timeout(STEP, node.events.recv()).await.expect("event within 10 s").expect("core alive");
        if let Some(v) = pick(&ev) {
            return v;
        }
    }
}

async fn serve_status(node: &mut Node) -> ServeStatus {
    wait(node, |e| if let CoreEvent::Serve(s) = e { Some(s.clone()) } else { None }).await
}

/// The next status that is not `starting` (it comes first whenever a listener is started).
async fn serve_settled(node: &mut Node) -> ServeStatus {
    loop {
        let status = serve_status(node).await;
        if status.phase != ServePhase::Starting {
            return status;
        }
    }
}

async fn error(node: &mut Node) -> String {
    wait(node, |e| if let CoreEvent::Error(m) = e { Some(m.clone()) } else { None }).await
}

fn on(port: u16) -> ServeSettings {
    ServeSettings { enabled: true, port, ..ServeSettings::default() }
}

fn tone(dir: &Path, seconds: f64) -> PcmFile {
    let samples: Vec<i16> = (0..(16_000.0 * seconds) as usize).map(|i| (6000.0 * (i as f64 * 0.17).sin()) as i16).collect();
    let path = dir.join("take.pcm");
    std::fs::write(&path, samples.iter().flat_map(|s| s.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
    PcmFile { path, samples: samples.len() as u64 }
}

#[tokio::test]
async fn switching_the_service_on_starts_a_listener_that_runs_with_the_apps_state() {
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(FakeHost::default());
    let mut node = start(dir.path(), Some(host.clone()));
    assert_eq!(serve_settled(&mut node).await, ServeStatus { available: true, phase: ServePhase::Off, address: None, error: None }, "off at first");
    node.handle.send(CoreCommand::SetServe(on(48123))).await.unwrap();
    assert_eq!(serve_status(&mut node).await.phase, ServePhase::Starting, "starting first");
    let status = serve_settled(&mut node).await;
    assert_eq!((status.phase, status.address.as_deref()), (ServePhase::Running, Some("http://127.0.0.1:48123/v1")));
    let (config, service) = {
        let log = host.log.lock();
        (log.configs[0].clone(), log.services[0].clone())
    };
    assert_eq!(config.addr.to_string(), "127.0.0.1:48123", "this computer only");
    let token = std::fs::read_to_string(dir.path().join("serve/token")).unwrap();
    assert_eq!(config.token, token.trim());
    assert_eq!(config.uploads_dir, dir.path().join("serve/uploads"));
    assert!(SettingsStore::new(dir.path()).load().unwrap().serve.enabled, "persisted");
    // The service runs the app's pipeline with the engine's clients.
    let audio = tone(dir.path(), 2.0);
    let out = service.transcribe(audio.clone(), ServeRequest::default(), CancelToken::new()).await.unwrap();
    assert_eq!((out.raw_text.as_str(), out.text.as_str(), out.refined), (FAKE_TRANSCRIPT, "你好，世界。整理好了。", true));
    assert_eq!(out.preset.map(|p| p.id), Some(PresetId::Builtin(BuiltinPreset::Proofread)));
    // A preset made in the app is there at once and can be the service's.
    node.handle.send(CoreCommand::PresetAdd(PresetDraft { name: "周报".into(), prompt: "整理成周报".into() })).await.unwrap();
    let id = wait(&mut node, |e| if let CoreEvent::Presets(p) = e { p.first().map(|p| p.id) } else { None }).await;
    assert!(service.models().iter().any(|m| m.id == format!("voltip:preset={id}")), "the new preset is listed");
    node.handle.send(CoreCommand::SetServe(ServeSettings { preset: Some(PresetId::Custom(id)), ..on(48123) })).await.unwrap();
    wait(&mut node, |e| if let CoreEvent::Settings(s) = e { (s.serve.preset == Some(PresetId::Custom(id))).then_some(()) } else { None }).await;
    let out = service.transcribe(audio, ServeRequest::default(), CancelToken::new()).await.unwrap();
    assert_eq!(out.preset.map(|p| p.name), Some("周报".to_owned()), "the service's preset follows the settings");
    assert_eq!(host.log.lock().configs.len(), 1, "the same port: no restart");
}

#[tokio::test]
async fn the_token_goes_to_the_clipboard_and_a_new_one_is_in_force_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(FakeHost::default());
    let mut node = start(dir.path(), Some(host.clone()));
    serve_settled(&mut node).await;
    node.handle.send(CoreCommand::SetServe(on(48124))).await.unwrap();
    assert_eq!(serve_settled(&mut node).await.phase, ServePhase::Running);
    node.handle.send(CoreCommand::ServeCopyToken).await.unwrap();
    node.handle.send(CoreCommand::ServeRotateToken).await.unwrap();
    // Commands run in order: once the rotation reached the host, the copy is done too.
    let started = std::time::Instant::now();
    while host.log.lock().tokens.is_empty() {
        assert!(started.elapsed() < STEP, "the new token never reached the listener");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let first = host.log.lock().configs[0].token.clone();
    assert_eq!(node.injector.clipboard_copies(), vec![first.clone()], "the token, on the clipboard only");
    let rotated = host.log.lock().tokens[0].clone();
    assert_ne!(rotated, first);
    assert_eq!(std::fs::read_to_string(dir.path().join("serve/token")).unwrap().trim(), rotated, "the file holds the new one");
}

#[tokio::test]
async fn a_taken_port_is_reported_and_switching_off_stops_the_listener() {
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(FakeHost::default());
    *host.refuse.lock() = true;
    let mut node = start(dir.path(), Some(host.clone()));
    serve_settled(&mut node).await;
    node.handle.send(CoreCommand::SetServe(on(48125))).await.unwrap();
    let failed = serve_settled(&mut node).await;
    assert_eq!(failed.phase, ServePhase::Failed);
    assert!(failed.error.unwrap().contains("Address already in use"));
    *host.refuse.lock() = false;
    node.handle.send(CoreCommand::SetServe(on(48126))).await.unwrap();
    assert_eq!(serve_settled(&mut node).await.phase, ServePhase::Running, "another port works");
    node.handle.send(CoreCommand::SetServe(on(48127))).await.unwrap();
    assert_eq!(serve_settled(&mut node).await.address.as_deref(), Some("http://127.0.0.1:48127/v1"), "a new port restarts it");
    assert_eq!(host.log.lock().stops, 1, "the old listener stopped");
    node.handle.send(CoreCommand::SetServe(ServeSettings { enabled: false, ..on(48127) })).await.unwrap();
    assert_eq!(serve_settled(&mut node).await.phase, ServePhase::Off);
    assert_eq!(host.log.lock().stops, 2);
    // Refusals change nothing.
    node.handle.send(CoreCommand::SetServe(on(80))).await.unwrap();
    assert!(error(&mut node).await.contains("端口须在 1024–65535 之间"));
    node.handle.send(CoreCommand::SetServe(ServeSettings { scene: Some(uuid::Uuid::new_v4()), ..on(48128) })).await.unwrap();
    assert!(error(&mut node).await.contains("场景"));
    node.handle.send(CoreCommand::SetServe(ServeSettings { preset: Some(PresetId::Custom(uuid::Uuid::new_v4())), ..on(48128) })).await.unwrap();
    assert!(error(&mut node).await.contains("预设"));
    assert!(!SettingsStore::new(dir.path()).load().unwrap().serve.enabled);
}

#[tokio::test]
async fn a_service_switched_on_starts_with_the_app() {
    let dir = tempfile::tempdir().unwrap();
    SettingsStore::new(dir.path())
        .save(&Settings { relay_enabled: false, engines: voltip_core::dictation::fakes::fake_engines(), serve: on(48129), ..Settings::default() })
        .unwrap();
    let host = Arc::new(FakeHost::default());
    let mut node = start(dir.path(), Some(host.clone()));
    let first = serve_settled(&mut node).await;
    let status = if first.phase == ServePhase::Running { first } else { serve_settled(&mut node).await };
    assert_eq!(status.address.as_deref(), Some("http://127.0.0.1:48129/v1"));
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
    let started = std::time::Instant::now();
    while host.log.lock().stops == 0 {
        assert!(started.elapsed() < STEP, "the listener was not stopped with the app");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn a_shell_without_a_host_refuses_the_service() {
    let dir = tempfile::tempdir().unwrap();
    let mut node = start(dir.path(), None);
    assert_eq!(serve_settled(&mut node).await, ServeStatus::default(), "not available");
    for command in [CoreCommand::SetServe(on(48130)), CoreCommand::ServeCopyToken, CoreCommand::ServeRotateToken] {
        node.handle.send(command).await.unwrap();
        assert_eq!(error(&mut node).await, SERVE_UNAVAILABLE);
    }
    assert!(!dir.path().join("serve").exists(), "no token was made");
}
