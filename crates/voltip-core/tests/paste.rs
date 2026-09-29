#![allow(clippy::unwrap_used, clippy::expect_used)]
//! A paste from the history (`voltip_core::paste`) through the real core task, on the dictation
//! fakes: it goes to the window the shell found while that window is still in front, to the
//! clipboard when another one came up, never while a take runs, and never into the history. The
//! answer carries the command's `request_id`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use voltip_core::dictation::fakes::{FakeAudio, FakeInjector, FakeProbe, FakeRefiner, FakeTranscriber};
use voltip_core::dictation::{DictationPorts, ForegroundApp, Refiner, Transcriber};
use voltip_core::paste::{CopyReason, PasteFailure, PasteOutcome, PasteTarget};
use voltip_core::{AppCore, CoreCommand, CoreConfig, CoreEvent, CoreHandle, DictationPhase, ResolvedEngines, Settings, SettingsStore};
use voltip_identity::MemorySecretStore;

const STEP: Duration = Duration::from_secs(10);

struct Node {
    handle: CoreHandle,
    events: mpsc::Receiver<CoreEvent>,
    injector: Arc<FakeInjector>,
    probe: Arc<FakeProbe>,
    _dir: tempfile::TempDir,
}

fn start() -> Node {
    let dir = tempfile::tempdir().unwrap();
    SettingsStore::new(dir.path())
        .save(&Settings { relay_enabled: false, engines: voltip_core::dictation::fakes::fake_engines(), ..Settings::default() })
        .unwrap();
    let injector = Arc::new(FakeInjector::paste());
    let probe = Arc::new(FakeProbe::app("notepad", "Notepad", None));
    probe.set_window(Some(7));
    let transcriber = Arc::new(FakeTranscriber::ok("你好"));
    let refiner = Arc::new(FakeRefiner::ok("你好。"));
    let ports = DictationPorts {
        audio: Arc::new(FakeAudio::speech()),
        injector: injector.clone(),
        factory: Arc::new(move |_: &ResolvedEngines| (transcriber.clone() as Arc<dyn Transcriber>, Some(refiner.clone() as Arc<dyn Refiner>))),
        models: None,
        streaming: None,
        probe: Some(probe.clone()),
        service_probe: None,
    };
    let mut config = CoreConfig::new(dir.path().to_path_buf());
    config.default_device_name = "Paste Test".into();
    config.direct_enabled = false;
    let (handle, events) = AppCore::start_with(config, Arc::new(MemorySecretStore::new()), ports).unwrap();
    Node { handle, events, injector, probe, _dir: dir }
}

async fn wait<T>(node: &mut Node, mut pick: impl FnMut(&CoreEvent) -> Option<T>) -> T {
    let deadline = Instant::now() + STEP;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let ev = tokio::time::timeout(left, node.events.recv()).await.expect("event in time").expect("core alive");
        if let Some(v) = pick(&ev) {
            return v;
        }
    }
}

/// Send the paste and wait for its answer. The history starts empty, so a history event with an
/// entry on the way is a failure: a paste from the history writes none.
async fn paste(node: &mut Node, request_id: u64, text: &str, target: PasteTarget) -> PasteOutcome {
    node.handle.send(CoreCommand::PasteText { request_id, text: text.into(), target }).await.unwrap();
    wait(node, |e| match e {
        CoreEvent::PasteResult { request_id: id, outcome } if *id == request_id => Some(*outcome),
        CoreEvent::History(entries) if !entries.is_empty() => panic!("a paste wrote the history: {entries:?}"),
        _ => None,
    })
    .await
}

fn notepad(window: Option<u64>) -> PasteTarget {
    PasteTarget::App(ForegroundApp { app_id: "notepad".into(), name: "Notepad".into(), title: None, window })
}

#[tokio::test]
async fn a_history_paste_goes_to_the_window_the_shell_found_and_nowhere_else() {
    let mut node = start();
    wait(&mut node, |e| matches!(e, CoreEvent::Ready { .. }).then_some(())).await;

    // The window is still in front: pasted, under the same request id.
    assert_eq!(paste(&mut node, 1, "第一段", notepad(Some(7))).await, PasteOutcome::Pasted);
    assert_eq!(node.injector.injected(), vec!["第一段".to_owned()]);

    // Another window of the same application came up: the clipboard only.
    node.probe.set_window(Some(8));
    assert_eq!(paste(&mut node, 2, "第二段", notepad(Some(7))).await, PasteOutcome::Copied { reason: CopyReason::TargetChanged });
    assert_eq!(node.injector.injected(), vec!["第一段".to_owned()]);
    assert_eq!(node.injector.clipboard_copies(), vec!["第二段".to_owned()]);

    // The shell found nothing to aim at: copied for its reason, the probe not asked again.
    let calls = node.probe.calls();
    assert_eq!(paste(&mut node, 3, "第三段", PasteTarget::CopyOnly(CopyReason::NoProbe)).await, PasteOutcome::Copied { reason: CopyReason::NoProbe });
    assert_eq!(node.probe.calls(), calls);

    // Empty or too long: refused.
    assert_eq!(paste(&mut node, 4, "   ", notepad(Some(8))).await, PasteOutcome::Failed { reason: PasteFailure::Invalid });

    // Nothing of this went into the history file either.
    let history = std::fs::read_to_string(node._dir.path().join(voltip_core::history::HISTORY_FILE_NAME)).unwrap_or_default();
    assert!(!history.contains("第一段") && !history.contains("第二段"), "{history}");
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

#[tokio::test]
async fn a_history_paste_waits_for_no_take() {
    let mut node = start();
    wait(&mut node, |e| matches!(e, CoreEvent::Ready { .. }).then_some(())).await;
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    wait(&mut node, |e| matches!(e, CoreEvent::Dictation(s) if matches!(s.phase, DictationPhase::Listening { .. })).then_some(())).await;
    assert_eq!(paste(&mut node, 9, "不该粘贴", notepad(Some(7))).await, PasteOutcome::Failed { reason: PasteFailure::Busy });
    assert!(node.injector.injected().is_empty() && node.injector.clipboard_copies().is_empty());
    node.handle.send(CoreCommand::DictationCancel).await.unwrap();
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}
