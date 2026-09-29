#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Voice edit (docs/dictation.md §19) through the real core task: `SetEditHotkey` validation and
//! persistence, `HotkeyEdge { purpose: edit }` driving its own activation machine, the copy timing
//! (`AtPress`, `AfterKeyUp` with the auto-repeat filter, CLI), the refusals, and a whole edit take
//! from the key to the history entry — on the dictation fakes.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use voltip_core::dictation::fakes::{FakeAudio, FakeInjector, FakeProbe, FakeRefiner, FakeTranscriber};
use voltip_core::dictation::{DictationPorts, ForegroundProbe, Refiner, Transcriber};
use voltip_core::{
    Activation, AppCore, CoreCommand, CoreConfig, CoreEvent, CoreHandle, DictationPhase, DictationStatus, EdgeSource, EditRecord, FailureCode, HistoryEntry,
    Modifier, ResolvedEngines, SETTINGS_FILE_NAME, Settings, SettingsStore, TakeKind, now_ms,
};
use voltip_identity::MemorySecretStore;

const STEP: Duration = Duration::from_secs(10);
/// See `tests/activation.rs`: edges of a real key are never closer than the debounce / grace.
const EDGE_GAP: Duration = Duration::from_millis(60);
const SELECTION: &str = "大家好，会议改到周四十点哈";
const INSTRUCTION: &str = "改得更正式";
const REWRITE: &str = "各位同事：会议改至周四上午十点。";

struct Node {
    handle: CoreHandle,
    events: mpsc::Receiver<CoreEvent>,
    audio: Arc<FakeAudio>,
    injector: Arc<FakeInjector>,
    refiner: Arc<FakeRefiner>,
    _dir: Option<tempfile::TempDir>,
}

fn config(dir: &std::path::Path) -> CoreConfig {
    let mut cfg = CoreConfig::new(dir.to_path_buf());
    cfg.default_device_name = "Edit Test".into();
    cfg.direct_enabled = false;
    cfg
}

/// A core whose recogniser hears [`INSTRUCTION`], whose LLM (when `llm`) answers [`REWRITE`], and
/// whose injector plays the foreground application (`injector`).
fn start_in(dir: &std::path::Path, injector: FakeInjector, llm: bool) -> Node {
    start_with_probe(dir, injector, llm, None)
}

/// [`start_in`] with a foreground probe (docs/dictation.md §18.2).
fn start_with_probe(dir: &std::path::Path, injector: FakeInjector, llm: bool, probe: Option<Arc<dyn ForegroundProbe>>) -> Node {
    if !dir.join(SETTINGS_FILE_NAME).exists() {
        SettingsStore::new(dir)
            .save(&Settings { relay_enabled: false, engines: voltip_core::dictation::fakes::fake_engines(), ..Settings::default() })
            .unwrap();
    }
    let audio = Arc::new(FakeAudio::speech());
    let injector = Arc::new(injector);
    let refiner = Arc::new(FakeRefiner::ok(REWRITE));
    let (factory_refiner, transcriber) = (refiner.clone(), Arc::new(FakeTranscriber::ok(INSTRUCTION)));
    let ports = DictationPorts {
        audio: audio.clone(),
        injector: injector.clone(),
        factory: Arc::new(move |_: &ResolvedEngines| {
            let refiner = llm.then(|| factory_refiner.clone() as Arc<dyn Refiner>);
            (transcriber.clone() as Arc<dyn Transcriber>, refiner)
        }),
        models: None,
        streaming: None,
        probe,
        service_probe: None,
    };
    let (handle, events) = AppCore::start_with(config(dir), Arc::new(MemorySecretStore::new()), ports).unwrap();
    Node { handle, events, audio, injector, refiner, _dir: None }
}

fn start(injector: FakeInjector, llm: bool) -> Node {
    let dir = tempfile::tempdir().unwrap();
    let node = start_in(dir.path(), injector, llm);
    Node { _dir: Some(dir), ..node }
}

async fn wait<T>(node: &mut Node, mut pick: impl FnMut(&CoreEvent) -> Option<T>) -> T {
    loop {
        let ev = tokio::time::timeout(STEP, node.events.recv()).await.expect("event within 10 s").expect("core alive");
        if let Some(v) = pick(&ev) {
            return v;
        }
    }
}

async fn wait_status(node: &mut Node, mut pred: impl FnMut(&DictationStatus) -> bool) -> DictationStatus {
    wait(node, |e| match e {
        CoreEvent::Dictation(s) if pred(s) => Some(s.clone()),
        _ => None,
    })
    .await
}

/// No `Dictation` event arrives within `within` (other events are drained).
async fn assert_quiet(node: &mut Node, within: Duration, what: &str) {
    let deadline = Instant::now() + within;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return;
        }
        match tokio::time::timeout(left, node.events.recv()).await {
            Ok(Some(CoreEvent::Dictation(s))) => panic!("{what}: {s:?}"),
            Ok(Some(_)) => {}
            Ok(None) => panic!("core stopped"),
            Err(_) => return,
        }
    }
}

/// Poll a fake's counter (bounded): the copy runs on a blocking thread of the core.
async fn wait_for(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + STEP;
    while !cond() {
        assert!(Instant::now() < deadline, "{what} within {STEP:?}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

async fn send_edge(node: &Node, pressed: bool, source: EdgeSource, purpose: TakeKind) {
    node.handle.send(CoreCommand::HotkeyEdge { pressed, at_ms: now_ms(), source, purpose, chorded: false }).await.unwrap();
}

async fn edge(node: &Node, pressed: bool, purpose: TakeKind) {
    tokio::time::sleep(EDGE_GAP).await;
    send_edge(node, pressed, EdgeSource::Hotkey, purpose).await;
}

async fn ready(node: &mut Node) -> Settings {
    wait(node, |e| if let CoreEvent::Ready { settings, .. } = e { Some(settings.clone()) } else { None }).await
}

async fn history(node: &mut Node) -> Vec<HistoryEntry> {
    wait(node, |e| match e {
        CoreEvent::History(entries) if !entries.is_empty() => Some(entries.clone()),
        _ => None,
    })
    .await
}

/// `SetEditHotkey` is validated like the dictation hotkey, canonicalised, never the dictation
/// chord (and the dictation hotkey never the edit chord), switched off with `None`, and persisted —
/// a switched-off hotkey stays off across a restart.
#[tokio::test]
async fn edit_hotkey_is_validated_persisted_and_never_the_dictation_chord() {
    let dir = tempfile::tempdir().unwrap();
    let mut node = start_in(dir.path(), FakeInjector::paste(), true);
    let settings = ready(&mut node).await;
    assert_eq!(settings.edit_hotkey.as_deref(), Some("Ctrl+Alt+E"), "the default");

    node.handle.send(CoreCommand::SetEditHotkey(Some("alt + control + shift + e".into()))).await.unwrap();
    let s = wait(&mut node, |e| if let CoreEvent::Settings(s) = e { Some(s.clone()) } else { None }).await;
    assert_eq!(s.edit_hotkey.as_deref(), Some("Ctrl+Alt+Shift+E"), "canonical form");

    for (cmd, needle) in [
        (CoreCommand::SetEditHotkey(Some("Space+Alt+Ctrl".into())), "已用作听写快捷键"),
        (CoreCommand::SetEditHotkey(Some("E".into())), "至少需要一个修饰键"),
        (CoreCommand::SetHotkey("Ctrl+Shift+Alt+E".into()), "已用作「编辑选中文本」的快捷键"),
    ] {
        node.handle.send(cmd).await.unwrap();
        let message = wait(&mut node, |e| if let CoreEvent::Error(m) = e { Some(m.clone()) } else { None }).await;
        assert!(message.contains(needle), "{message}");
    }

    node.handle.send(CoreCommand::SetEditHotkey(None)).await.unwrap();
    let s = wait(&mut node, |e| if let CoreEvent::Settings(s) = e { Some(s.clone()) } else { None }).await;
    assert_eq!(s.edit_hotkey, None);
    assert_eq!(s.hotkey, "Ctrl+Alt+Space", "the refused dictation change did not land");
    let text = std::fs::read_to_string(dir.path().join(SETTINGS_FILE_NAME)).unwrap();
    assert!(text.contains(r#""edit_hotkey": null"#), "{text}");
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
    drop(node);
    let mut node = start_in(dir.path(), FakeInjector::paste(), true);
    assert_eq!(ready(&mut node).await.edit_hotkey, None, "still off after a restart");
}

/// The edit key (hold, the default) runs a whole edit take through the core: the status says
/// `edit`, the copy carries the edit chord's modifiers, the LLM gets the selection and the
/// instruction, the rewrite is pasted and the history records the edit.
#[tokio::test]
async fn the_edit_key_runs_an_edit_take_through_the_core_and_records_it() {
    let mut node = start(FakeInjector::paste().with_selection(SELECTION), true);
    ready(&mut node).await;
    edge(&node, true, TakeKind::Edit).await;
    let listening = wait_status(&mut node, |s| matches!(s.phase, DictationPhase::Listening { .. })).await;
    assert_eq!(listening.kind, TakeKind::Edit);
    wait_for("the copy at the press", || node.injector.copies().len() == 1).await;
    assert_eq!(node.injector.copies(), vec![vec![Modifier::Ctrl, Modifier::Alt]], "the held modifiers of Ctrl+Alt+E");
    edge(&node, false, TakeKind::Edit).await;
    let done = wait_status(&mut node, |s| s.phase.is_terminal()).await;
    assert!(matches!(&done.phase, DictationPhase::Done { text, .. } if text == REWRITE), "{done:?}");
    assert_eq!(done.kind, TakeKind::Edit);
    let entries = history(&mut node).await;
    assert_eq!(entries[0].kind, TakeKind::Edit);
    assert_eq!(entries[0].edit, Some(EditRecord { instruction: INSTRUCTION.into(), selection: SELECTION.into() }));
    assert_eq!(node.injector.injected(), vec![REWRITE.to_owned()]);
    assert_eq!(node.refiner.edits(), vec![(SELECTION.to_owned(), INSTRUCTION.to_owned(), Vec::new())]);
    assert_eq!(node.refiner.calls(), 0, "no refine pass");
}

/// One take at a time (docs/dictation.md §19): while a dictation runs, the edit key's edges are
/// dropped — no stop, no pending start — and vice versa.
#[tokio::test]
async fn edges_of_the_other_key_are_dropped_while_a_take_runs() {
    let mut node = start(FakeInjector::paste().with_selection(SELECTION), true);
    ready(&mut node).await;
    edge(&node, true, TakeKind::Dictation).await;
    // The device's `ready` mark is the last status of the press; nothing may follow it.
    let listening = wait_status(&mut node, |s| matches!(s.phase, DictationPhase::Listening { ready: true, .. })).await;
    assert_eq!(listening.kind, TakeKind::Dictation);
    edge(&node, true, TakeKind::Edit).await;
    edge(&node, false, TakeKind::Edit).await;
    assert_quiet(&mut node, Duration::from_millis(200), "the edit key must not touch a dictation").await;
    edge(&node, false, TakeKind::Dictation).await;
    let done = wait_status(&mut node, |s| s.phase.is_terminal()).await;
    assert!(matches!(done.phase, DictationPhase::Done { .. }) && done.kind == TakeKind::Dictation, "{done:?}");
    assert!(node.injector.copies().is_empty(), "no copy for a dictation");
    node.handle.send(CoreCommand::DictationCancel).await.unwrap();
    wait_status(&mut node, |s| s.phase == DictationPhase::Idle).await;
    // And the other way round.
    edge(&node, true, TakeKind::Edit).await;
    wait_status(&mut node, |s| matches!(s.phase, DictationPhase::Listening { ready: true, .. }) && s.kind == TakeKind::Edit).await;
    edge(&node, true, TakeKind::Dictation).await;
    edge(&node, false, TakeKind::Dictation).await;
    assert_quiet(&mut node, Duration::from_millis(200), "the dictation key must not touch an edit").await;
    edge(&node, false, TakeKind::Edit).await;
    let done = wait_status(&mut node, |s| s.phase.is_terminal()).await;
    assert_eq!(done.kind, TakeKind::Edit);
}

/// `AfterKeyUp` (an X11 session, docs/dictation.md §19): nothing is copied while the key is down;
/// the copy goes out one release grace after the key comes up; a press inside that grace (X11
/// auto-repeat) cancels the pending copy until the real release.
#[tokio::test]
async fn regression_after_key_up_the_copy_waits_for_the_release_and_auto_repeat_disarms_it() {
    let mut node = start(FakeInjector::paste().with_selection(SELECTION).after_key_up(), true);
    ready(&mut node).await;
    node.handle.send(CoreCommand::SetActivation { activation: Activation::Toggle, hold_threshold_ms: 300, extra_recording_ms: 0 }).await.unwrap();
    wait(&mut node, |e| matches!(e, CoreEvent::Settings(s) if s.activation == Activation::Toggle).then_some(())).await;
    edge(&node, true, TakeKind::Edit).await;
    wait_status(&mut node, |s| matches!(s.phase, DictationPhase::Listening { .. }) && s.kind == TakeKind::Edit).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(node.injector.copies().is_empty(), "no copy while the key is down");
    // Release + press 10 ms apart: X11 auto-repeat, the key is still held.
    edge(&node, false, TakeKind::Edit).await;
    tokio::time::sleep(Duration::from_millis(10)).await;
    send_edge(&node, true, EdgeSource::Hotkey, TakeKind::Edit).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(node.injector.copies().is_empty(), "the auto-repeat press disarmed the copy");
    // The real release: the copy follows within the grace.
    let released = Instant::now();
    edge(&node, false, TakeKind::Edit).await;
    wait_for("the copy after the key-up", || node.injector.copies().len() == 1).await;
    assert!(released.elapsed() >= Duration::from_millis(50), "the copy waited out the release grace");
    edge(&node, true, TakeKind::Edit).await;
    let done = wait_status(&mut node, |s| s.phase.is_terminal()).await;
    assert!(matches!(&done.phase, DictationPhase::Done { text, .. } if text == REWRITE), "{done:?}");
    assert_eq!(node.injector.copies().len(), 1, "one copy per take");
}

/// `voltip --edit-toggle` (a CLI edge, no key to wait for) copies at once even where the key
/// would have to come up first, and a second one stops the take.
#[tokio::test]
async fn a_cli_edit_edge_copies_at_once_and_toggles_the_take() {
    let mut node = start(FakeInjector::paste().with_selection(SELECTION).after_key_up(), true);
    ready(&mut node).await;
    send_edge(&node, true, EdgeSource::Cli, TakeKind::Edit).await;
    wait_status(&mut node, |s| matches!(s.phase, DictationPhase::Listening { .. }) && s.kind == TakeKind::Edit).await;
    wait_for("the copy of a CLI edge", || node.injector.copies().len() == 1).await;
    assert_eq!(node.injector.copies(), vec![Vec::<Modifier>::new()], "no key, no modifiers to release");
    tokio::time::sleep(EDGE_GAP).await;
    send_edge(&node, true, EdgeSource::Cli, TakeKind::Edit).await;
    let done = wait_status(&mut node, |s| s.phase.is_terminal()).await;
    assert!(matches!(&done.phase, DictationPhase::Done { text, .. } if text == REWRITE), "{done:?}");
}

/// Without an LLM (no refine key) the edit key fails at once, the microphone never opens, and the
/// release that follows does nothing; nothing selected ends the take as `no_selection`.
#[tokio::test]
async fn refusals_reach_the_ui_as_failed_phases() {
    let mut node = start(FakeInjector::paste().with_selection(SELECTION), false);
    ready(&mut node).await;
    edge(&node, true, TakeKind::Edit).await;
    let failed = wait_status(&mut node, |s| s.phase.is_terminal()).await;
    assert!(matches!(failed.phase, DictationPhase::Failed { code: FailureCode::EditUnavailable, .. }), "{failed:?}");
    edge(&node, false, TakeKind::Edit).await;
    assert_quiet(&mut node, Duration::from_millis(200), "the release after a refused press").await;
    assert_eq!(node.audio.starts(), 0, "the microphone never opened");

    let mut node = start(FakeInjector::paste(), true);
    ready(&mut node).await;
    edge(&node, true, TakeKind::Edit).await;
    let failed = wait_status(&mut node, |s| s.phase.is_terminal()).await;
    assert!(matches!(failed.phase, DictationPhase::Failed { code: FailureCode::NoSelection, .. }), "{failed:?}");
    wait_for("the device release", || node.audio.stops() == 1).await;
    assert!(node.injector.injected().is_empty() && node.refiner.edits().is_empty());
}

/// docs/dictation.md §19.2 through the real core task: the edit key pressed with a terminal in
/// front (the Windows table, `WindowsTerminal.exe`) is refused as `edit_in_terminal` — the runtime's
/// copy at the press waits for the probe, so no copy chord goes out, the microphone never opens
/// and no history is written; the release that follows does nothing.
#[tokio::test]
async fn regression_the_edit_key_in_a_terminal_sends_no_copy_chord_and_opens_no_microphone() {
    let dir = tempfile::tempdir().unwrap();
    let injector =
        FakeInjector::paste().with_selection(SELECTION).with_terminals(|id| voltip_platform::foreground::is_terminal(voltip_platform::HostOs::Windows, id));
    let probe = Arc::new(FakeProbe::app("WindowsTerminal.exe", "WindowsTerminal", Some("PowerShell")));
    let mut node = start_with_probe(dir.path(), injector, true, Some(probe.clone()));
    ready(&mut node).await;
    edge(&node, true, TakeKind::Edit).await;
    let failed = wait_status(&mut node, |s| s.phase.is_terminal()).await;
    assert!(matches!(failed.phase, DictationPhase::Failed { code: FailureCode::EditInTerminal, .. }), "{failed:?}");
    assert_eq!(failed.kind, TakeKind::Edit);
    edge(&node, false, TakeKind::Edit).await;
    assert_quiet(&mut node, Duration::from_millis(200), "the release after a refused press").await;
    assert!(node.injector.copies().is_empty(), "no copy chord was sent");
    assert_eq!(node.audio.starts(), 0, "the microphone never opened");
    assert!(node.injector.injected().is_empty() && node.refiner.edits().is_empty());
    assert_eq!(probe.calls(), 1);
    // The next press in an editor goes through: the copy follows the probe's answer.
    probe.set_app("Code.exe", "Code", None);
    edge(&node, true, TakeKind::Edit).await;
    wait_status(&mut node, |s| matches!(s.phase, DictationPhase::Listening { ready: true, .. })).await;
    wait_for("the copy after the probe", || node.injector.copies().len() == 1).await;
    edge(&node, false, TakeKind::Edit).await;
    let done = wait_status(&mut node, |s| s.phase.is_terminal()).await;
    assert!(matches!(&done.phase, DictationPhase::Done { text, .. } if text == REWRITE), "{done:?}");
    let entries = history(&mut node).await;
    assert_eq!(entries.len(), 1, "only the editor take is recorded");
    assert_eq!(entries[0].app.as_ref().map(|a| a.id.as_str()), Some("code"));
}
