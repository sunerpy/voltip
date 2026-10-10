#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Activation (docs/dictation.md §13) through the real core task: `HotkeyEdge` commands in,
//! `Dictation` events out, on the dictation fakes. The pure machine is table-tested in
//! `voltip_core::dictation::activation`; this covers the runtime glue — the grace timer, the lock
//! flag on the wire, `SetActivation` persistence and validation, the extra-recording window and
//! the pending start after processing.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use voltip_core::dictation::fakes::{FAKE_TRANSCRIPT, FakeAudio, FakeInjector, FakeTranscriber};
use voltip_core::dictation::{DictationPorts, Transcriber};
use voltip_core::{
    Activation, AppCore, CoreCommand, CoreConfig, CoreEvent, CoreHandle, DictationPhase, EdgeSource, MAX_ACTIVATION_MS, ResolvedEngines, SETTINGS_FILE_NAME,
    Settings, SettingsStore, TakeKind, now_ms,
};
use voltip_identity::MemorySecretStore;

const STEP: Duration = Duration::from_secs(10);

struct Node {
    handle: CoreHandle,
    events: mpsc::Receiver<CoreEvent>,
    dir: tempfile::TempDir,
}

fn config(dir: &std::path::Path) -> CoreConfig {
    SettingsStore::new(dir).save(&Settings { relay_enabled: false, engines: voltip_core::dictation::fakes::fake_engines(), ..Settings::default() }).unwrap();
    let mut cfg = CoreConfig::new(dir.to_path_buf());
    cfg.default_device_name = "Activation Test".into();
    cfg
}

fn start(audio: Arc<FakeAudio>, transcriber: Arc<FakeTranscriber>) -> Node {
    let dir = tempfile::tempdir().unwrap();
    let ports = DictationPorts {
        audio,
        injector: Arc::new(FakeInjector::paste()),
        factory: Arc::new(move |_: &ResolvedEngines| (transcriber.clone() as Arc<dyn Transcriber>, None)),
        models: None,
        streaming: None,
        probe: None,
        service_probe: None,
        segmenter: None,
    };
    let (handle, events) = AppCore::start_with(config(dir.path()), Arc::new(MemorySecretStore::new()), ports).unwrap();
    Node { handle, events, dir }
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

/// No `Dictation` event satisfying `pred` arrives within `within` (other events are drained).
async fn assert_no_phase(node: &mut Node, within: Duration, mut pred: impl FnMut(&DictationPhase) -> bool, what: &str) {
    let deadline = Instant::now() + within;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return;
        }
        match tokio::time::timeout(left, node.events.recv()).await {
            Ok(Some(CoreEvent::Dictation(s))) if pred(&s.phase) => panic!("{what}: {:?}", s.phase),
            Ok(Some(_)) => {}
            Ok(None) => panic!("core stopped"),
            Err(_) => return,
        }
    }
}

/// Skip the 2.5 s terminal dwell: a cancel in a terminal state goes straight to `Idle`.
async fn dismiss(node: &mut Node) {
    node.handle.send(CoreCommand::DictationCancel).await.unwrap();
    wait_phase(node, |p| *p == DictationPhase::Idle).await;
}

/// Real keys are never this fast: the fake pipeline finishes a whole run inside a millisecond, so
/// two consecutive edges would land inside the 30 ms debounce / 50 ms grace windows. Every edge
/// waits this long first.
const EDGE_GAP: Duration = Duration::from_millis(60);

async fn edge(node: &Node, pressed: bool, source: EdgeSource) {
    tokio::time::sleep(EDGE_GAP).await;
    node.handle.send(CoreCommand::HotkeyEdge { pressed, at_ms: now_ms(), source, purpose: TakeKind::Dictation, chorded: false }).await.unwrap();
}

/// The shell's report that another key joined the held lone-key trigger (docs/dictation.md §13.1).
async fn chord(node: &Node) {
    tokio::time::sleep(EDGE_GAP).await;
    node.handle
        .send(CoreCommand::HotkeyEdge { pressed: false, at_ms: now_ms(), source: EdgeSource::Hotkey, purpose: TakeKind::Dictation, chorded: true })
        .await
        .unwrap();
}

async fn set_activation(node: &mut Node, activation: Activation, hold_threshold_ms: u32, extra_recording_ms: u32) {
    node.handle.send(CoreCommand::SetActivation { activation, hold_threshold_ms, extra_recording_ms }).await.unwrap();
    wait(node, |e| matches!(e, CoreEvent::Settings(s) if s.activation == activation && s.extra_recording_ms == extra_recording_ms).then_some(())).await;
}

#[tokio::test]
async fn hotkey_edges_drive_hold_toggle_and_lock_through_the_core() {
    let audio = Arc::new(FakeAudio::speech());
    let mut node = start(audio.clone(), Arc::new(FakeTranscriber::ok(FAKE_TRANSCRIPT)));
    let settings = wait(&mut node, |e| if let CoreEvent::Ready { settings, .. } = e { Some(settings.clone()) } else { None }).await;
    assert_eq!((settings.activation, settings.hold_threshold_ms, settings.extra_recording_ms), (Activation::Hold, 300, 0), "defaults");

    // hold: press → listening (not locked), release → stop after the 50 ms grace → done.
    edge(&node, true, EdgeSource::Hotkey).await;
    let listening = wait_phase(&mut node, |p| matches!(p, DictationPhase::Listening { .. })).await;
    assert!(matches!(listening, DictationPhase::Listening { locked: false, .. }), "{listening:?}");
    let released = Instant::now();
    edge(&node, false, EdgeSource::Hotkey).await;
    wait_phase(&mut node, |p| matches!(p, DictationPhase::Processing { .. })).await;
    assert!(released.elapsed() >= Duration::from_millis(45), "the stop waits out the release grace: {:?}", released.elapsed());
    let done = wait_phase(&mut node, DictationPhase::is_terminal).await;
    assert!(matches!(&done, DictationPhase::Done { text, .. } if text == FAKE_TRANSCRIPT), "{done:?}");
    dismiss(&mut node).await;

    // toggle: the release is ignored, the next press stops.
    set_activation(&mut node, Activation::Toggle, 300, 0).await;
    edge(&node, true, EdgeSource::Hotkey).await;
    wait_phase(&mut node, |p| matches!(p, DictationPhase::Listening { .. })).await;
    edge(&node, false, EdgeSource::Hotkey).await;
    assert_no_phase(&mut node, Duration::from_millis(250), |p| !matches!(p, DictationPhase::Listening { .. }), "a release in toggle mode must not stop").await;
    edge(&node, true, EdgeSource::Hotkey).await;
    wait_phase(&mut node, |p| matches!(p, DictationPhase::Processing { .. })).await;
    wait_phase(&mut node, DictationPhase::is_terminal).await;
    dismiss(&mut node).await;

    // hold_or_toggle: a short press locks (the flag is on the wire), the next press stops.
    set_activation(&mut node, Activation::HoldOrToggle, 300, 0).await;
    edge(&node, true, EdgeSource::Hotkey).await;
    wait_phase(&mut node, |p| matches!(p, DictationPhase::Listening { locked: false, .. })).await;
    edge(&node, false, EdgeSource::Hotkey).await;
    let locked = wait_phase(&mut node, |p| matches!(p, DictationPhase::Listening { locked: true, .. })).await;
    assert!(matches!(locked, DictationPhase::Listening { locked: true, .. }));
    assert_no_phase(&mut node, Duration::from_millis(200), |p| !matches!(p, DictationPhase::Listening { .. }), "locked keeps listening").await;
    edge(&node, true, EdgeSource::Hotkey).await;
    let processing = wait_phase(&mut node, |p| matches!(p, DictationPhase::Processing { .. })).await;
    assert!(matches!(processing, DictationPhase::Processing { .. }));
    wait_phase(&mut node, DictationPhase::is_terminal).await;
    dismiss(&mut node).await;

    // hold_or_toggle: a long press stops on release.
    edge(&node, true, EdgeSource::Hotkey).await;
    wait_phase(&mut node, |p| matches!(p, DictationPhase::Listening { .. })).await;
    tokio::time::sleep(Duration::from_millis(350)).await;
    edge(&node, false, EdgeSource::Hotkey).await;
    wait_phase(&mut node, |p| matches!(p, DictationPhase::Processing { .. })).await;
    wait_phase(&mut node, DictationPhase::is_terminal).await;
    dismiss(&mut node).await;

    // CLI edges alternate start / stop whatever the mode, and a UI edge is a key.
    edge(&node, true, EdgeSource::Cli).await;
    wait_phase(&mut node, |p| matches!(p, DictationPhase::Listening { .. })).await;
    edge(&node, true, EdgeSource::Cli).await;
    wait_phase(&mut node, |p| matches!(p, DictationPhase::Processing { .. })).await;
    wait_phase(&mut node, DictationPhase::is_terminal).await;
    dismiss(&mut node).await;
    edge(&node, true, EdgeSource::Ui).await;
    wait_phase(&mut node, |p| matches!(p, DictationPhase::Listening { ready: true, .. })).await;
    // A CLI release is the edge-path cancel.
    edge(&node, false, EdgeSource::Cli).await;
    assert!(matches!(wait_phase(&mut node, DictationPhase::is_terminal).await, DictationPhase::Cancelled { .. }));
    assert_eq!(audio.starts(), 6);

    // Validation and persistence.
    node.handle
        .send(CoreCommand::SetActivation { activation: Activation::Hold, hold_threshold_ms: MAX_ACTIVATION_MS + 1, extra_recording_ms: 0 })
        .await
        .unwrap();
    let err = wait(&mut node, |e| if let CoreEvent::Error(m) = e { Some(m.clone()) } else { None }).await;
    assert!(err.contains("activation"), "{err}");
    let saved = SettingsStore::new(node.dir.path()).load().unwrap();
    assert_eq!((saved.activation, saved.hold_threshold_ms), (Activation::HoldOrToggle, 300), "the refused change did not stick");
    let text = std::fs::read_to_string(node.dir.path().join(SETTINGS_FILE_NAME)).unwrap();
    assert!(text.contains(r#""activation": "hold_or_toggle""#), "{text}");
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

#[tokio::test]
async fn extra_recording_keeps_the_microphone_open_and_a_cancel_or_second_stop_cuts_it_short() {
    let audio = Arc::new(FakeAudio::speech());
    let mut node = start(audio.clone(), Arc::new(FakeTranscriber::ok(FAKE_TRANSCRIPT)));
    wait(&mut node, |e| matches!(e, CoreEvent::Ready { .. }).then_some(())).await;
    set_activation(&mut node, Activation::Hold, 300, 400).await;

    // The stop is honoured ~400 ms later; the UI button path takes the same window.
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    wait_phase(&mut node, |p| matches!(p, DictationPhase::Listening { .. })).await;
    let stopped = Instant::now();
    node.handle.send(CoreCommand::DictationStop).await.unwrap();
    assert_no_phase(&mut node, Duration::from_millis(250), |p| !matches!(p, DictationPhase::Listening { .. }), "still recording inside the window").await;
    wait_phase(&mut node, |p| matches!(p, DictationPhase::Processing { .. })).await;
    let elapsed = stopped.elapsed();
    assert!(elapsed >= Duration::from_millis(380) && elapsed < Duration::from_secs(3), "{elapsed:?}");
    wait_phase(&mut node, DictationPhase::is_terminal).await;
    dismiss(&mut node).await;
    assert_eq!(audio.stops(), 1);

    // A cancel inside the window discards the recording: no processing, no second stop.
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    wait_phase(&mut node, |p| matches!(p, DictationPhase::Listening { .. })).await;
    node.handle.send(CoreCommand::DictationStop).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    node.handle.send(CoreCommand::DictationCancel).await.unwrap();
    assert!(matches!(wait_phase(&mut node, DictationPhase::is_terminal).await, DictationPhase::Cancelled { .. }));
    assert_no_phase(&mut node, Duration::from_millis(600), |p| matches!(p, DictationPhase::Processing { .. }), "the deferred stop was cancelled").await;
    dismiss(&mut node).await;

    // A second stop inside the window closes the microphone at once.
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    wait_phase(&mut node, |p| matches!(p, DictationPhase::Listening { .. })).await;
    node.handle.send(CoreCommand::DictationStop).await.unwrap();
    let again = Instant::now();
    node.handle.send(CoreCommand::DictationStop).await.unwrap();
    wait_phase(&mut node, |p| matches!(p, DictationPhase::Processing { .. })).await;
    assert!(again.elapsed() < Duration::from_millis(300), "{:?}", again.elapsed());
    wait_phase(&mut node, DictationPhase::is_terminal).await;
    dismiss(&mut node).await;

    // Through the key: a press inside the window is pending (the window counts as processing) and
    // starts the next run once idle.
    set_activation(&mut node, Activation::Toggle, 300, 300).await;
    edge(&node, true, EdgeSource::Hotkey).await;
    wait_phase(&mut node, |p| matches!(p, DictationPhase::Listening { .. })).await;
    edge(&node, true, EdgeSource::Hotkey).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    edge(&node, true, EdgeSource::Hotkey).await;
    wait_phase(&mut node, |p| matches!(p, DictationPhase::Processing { .. })).await;
    let done = wait_phase(&mut node, DictationPhase::is_terminal).await;
    assert!(matches!(done, DictationPhase::Done { .. }), "{done:?}");
    let next = wait(&mut node, |e| match e {
        CoreEvent::Dictation(s) if matches!(s.phase, DictationPhase::Listening { .. }) => Some(s.session),
        _ => None,
    })
    .await;
    assert_eq!(next, 5, "the pending press started the next session");
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

#[tokio::test]
async fn a_press_while_processing_starts_the_next_run_and_a_failed_start_rolls_back() {
    let audio = Arc::new(FakeAudio::speech());
    let mut node = start(audio.clone(), Arc::new(FakeTranscriber::slow(FAKE_TRANSCRIPT, Duration::from_millis(600))));
    wait(&mut node, |e| matches!(e, CoreEvent::Ready { .. }).then_some(())).await;
    set_activation(&mut node, Activation::Toggle, 300, 0).await;
    edge(&node, true, EdgeSource::Hotkey).await;
    wait_phase(&mut node, |p| matches!(p, DictationPhase::Listening { .. })).await;
    edge(&node, true, EdgeSource::Hotkey).await;
    wait_phase(&mut node, |p| matches!(p, DictationPhase::Processing { .. })).await;
    // Pending, then cancelled by the same key, then pending again.
    edge(&node, true, EdgeSource::Hotkey).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    edge(&node, true, EdgeSource::Hotkey).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    edge(&node, true, EdgeSource::Hotkey).await;
    wait_phase(&mut node, DictationPhase::is_terminal).await;
    // `ready` follows the device open, so the start count is settled by then.
    let session = wait(&mut node, |e| match e {
        CoreEvent::Dictation(s) if matches!(s.phase, DictationPhase::Listening { ready: true, .. }) => Some(s.session),
        _ => None,
    })
    .await;
    assert_eq!(session, 2);
    assert_eq!(audio.starts(), 2);
    node.handle.send(CoreCommand::DictationCancel).await.unwrap();
    dismiss(&mut node).await;
    node.handle.send(CoreCommand::Shutdown).await.unwrap();

    // A start the engine refuses (no microphone) rolls the machine back: the release that follows
    // is a no-op and the next press asks again.
    let failing = Arc::new(FakeAudio::failing_start("no mic"));
    let mut node = start(failing.clone(), Arc::new(FakeTranscriber::ok(FAKE_TRANSCRIPT)));
    wait(&mut node, |e| matches!(e, CoreEvent::Ready { .. }).then_some(())).await;
    edge(&node, true, EdgeSource::Hotkey).await;
    wait_phase(&mut node, |p| matches!(p, DictationPhase::Listening { .. })).await;
    let failed = wait_phase(&mut node, DictationPhase::is_terminal).await;
    assert!(matches!(failed, DictationPhase::Failed { .. }), "{failed:?}");
    edge(&node, false, EdgeSource::Hotkey).await;
    assert_no_phase(&mut node, Duration::from_millis(150), |p| matches!(p, DictationPhase::Processing { .. }), "nothing to stop").await;
    edge(&node, true, EdgeSource::Hotkey).await;
    wait_phase(&mut node, |p| matches!(p, DictationPhase::Listening { .. })).await;
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// docs/dictation.md §13.1: Right Ctrl + C with Right Ctrl as the trigger. The press starts a take,
/// the chord cancels it before anything is recognised, and the next lone press dictates normally.
#[tokio::test]
async fn regression_a_chorded_trigger_cancels_its_take_through_the_core() {
    let audio = Arc::new(FakeAudio::speech());
    let transcriber = Arc::new(FakeTranscriber::ok(FAKE_TRANSCRIPT));
    let mut node = start(audio.clone(), transcriber.clone());
    wait(&mut node, |e| matches!(e, CoreEvent::Ready { .. }).then_some(())).await;

    edge(&node, true, EdgeSource::Hotkey).await;
    wait_phase(&mut node, |p| matches!(p, DictationPhase::Listening { .. })).await;
    chord(&node).await;
    let end = wait_phase(&mut node, DictationPhase::is_terminal).await;
    assert!(matches!(end, DictationPhase::Cancelled { .. }), "{end:?}");
    assert_eq!(transcriber.calls(), 0, "a chord is never recognised");
    dismiss(&mut node).await;

    edge(&node, true, EdgeSource::Hotkey).await;
    wait_phase(&mut node, |p| matches!(p, DictationPhase::Listening { .. })).await;
    edge(&node, false, EdgeSource::Hotkey).await;
    let done = wait_phase(&mut node, DictationPhase::is_terminal).await;
    assert!(matches!(&done, DictationPhase::Done { text, .. } if text == FAKE_TRANSCRIPT), "{done:?}");
    assert_eq!(transcriber.calls(), 1);
}
