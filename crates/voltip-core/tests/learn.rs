#![allow(clippy::unwrap_used, clippy::expect_used)]
//! 从选中文字学习 (docs/dictation.md §35) through the real core task, on the dictation fakes: the
//! shortcut among four distinct chords, a selection that corrects the take (stored with the entry,
//! offered on the notice, 加入词典 and 撤销 through the learning suggestions' batches), the notice's
//! queue and timer (behind a result card, cleared by a take, none with the pill off), and the
//! refusals (history off, a terminal in front, nothing selected, nothing alike).

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use voltip_core::dictation::fakes::{FakeAudio, FakeFocusReader, FakeInjector, FakeProbe, FakeTranscriber};
use voltip_core::dictation::{DictationPorts, ForegroundProbe, Transcriber, UnconfirmedWhy};
use voltip_core::{
    AppCore, CoreCommand, CoreConfig, CoreEvent, CoreHandle, DictationPhase, EdgeSource, HistoryEntry, LearnNotice, LearnOutcome, Modifier, OverlayPlacement,
    ResolvedEngines, SETTINGS_FILE_NAME, Settings, SettingsStore,
};
use voltip_identity::MemorySecretStore;

const STEP: Duration = Duration::from_secs(10);
/// The notice's stay in these tests.
const DWELL: Duration = Duration::from_millis(400);
/// What the fake recogniser hears; inserted as it is (no AI clean-up configured).
const HEARD: &str = "我在用 Quinn for max 做测试";
/// What the user corrected it to in the application.
const FIXED: &str = "我在用 Qwen 4 max 做测试";

struct Node {
    handle: CoreHandle,
    events: mpsc::Receiver<CoreEvent>,
    injector: Arc<FakeInjector>,
    _dir: tempfile::TempDir,
}

fn start(injector: FakeInjector, probe: Option<Arc<dyn ForegroundProbe>>, result_cards: bool) -> Node {
    let dir = tempfile::tempdir().unwrap();
    if !dir.path().join(SETTINGS_FILE_NAME).exists() {
        SettingsStore::new(dir.path())
            .save(&Settings { relay_enabled: false, engines: voltip_core::dictation::fakes::fake_engines(), ..Settings::default() })
            .unwrap();
    }
    let injector = Arc::new(injector);
    let transcriber = Arc::new(FakeTranscriber::ok(HEARD));
    let ports = DictationPorts {
        audio: Arc::new(FakeAudio::speech()),
        injector: injector.clone(),
        factory: Arc::new(move |_: &ResolvedEngines| (transcriber.clone() as Arc<dyn Transcriber>, None)),
        models: None,
        streaming: None,
        probe,
        service_probe: None,
        segmenter: None,
    };
    let mut cfg = CoreConfig::new(dir.path().to_path_buf());
    cfg.default_device_name = "Learn Test".into();
    cfg.result_cards = result_cards;
    cfg.learn_notice_dwell = DWELL;
    let (handle, events) = AppCore::start_with(cfg, Arc::new(MemorySecretStore::new()), ports).unwrap();
    Node { handle, events, injector, _dir: dir }
}

async fn wait<T>(node: &mut Node, mut pick: impl FnMut(&CoreEvent) -> Option<T>) -> T {
    loop {
        let ev = tokio::time::timeout(STEP, node.events.recv()).await.expect("event within 10 s").expect("core alive");
        if let Some(v) = pick(&ev) {
            return v;
        }
    }
}

async fn ready(node: &mut Node) {
    wait(node, |e| matches!(e, CoreEvent::Ready { .. }).then_some(())).await;
}

/// The next learn notice event.
async fn notice(node: &mut Node) -> Option<LearnNotice> {
    wait(node, |e| if let CoreEvent::LearnNotice(n) = e { Some(n.clone()) } else { None }).await
}

/// No learn notice event within `within` (other events are drained).
async fn no_notice(node: &mut Node, within: Duration, what: &str) {
    let deadline = Instant::now() + within;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return;
        }
        match tokio::time::timeout(left, node.events.recv()).await {
            Ok(Some(CoreEvent::LearnNotice(n))) => panic!("{what}: {n:?}"),
            Ok(Some(_)) => {}
            Ok(None) => panic!("core stopped"),
            Err(_) => return,
        }
    }
}

/// One take of [`HEARD`]: its session once it is over.
async fn dictate(node: &mut Node) -> u64 {
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    let session = wait(node, |e| match e {
        CoreEvent::Dictation(s) if matches!(s.phase, DictationPhase::Listening { .. }) => Some(s.session),
        _ => None,
    })
    .await;
    node.handle.send(CoreCommand::DictationStop).await.unwrap();
    wait(node, |e| matches!(e, CoreEvent::Dictation(s) if s.phase.is_terminal()).then_some(())).await;
    session
}

async fn idle(node: &mut Node) {
    wait(node, |e| matches!(e, CoreEvent::Dictation(s) if s.phase == DictationPhase::Idle).then_some(())).await;
}

async fn learn(node: &Node) {
    node.handle.send(CoreCommand::LearnSelection { source: EdgeSource::Hotkey }).await.unwrap();
}

async fn history_with(node: &mut Node, pred: impl Fn(&HistoryEntry) -> bool) -> HistoryEntry {
    wait(node, |e| match e {
        CoreEvent::History { recent, .. } => recent.iter().find(|e| pred(e)).cloned(),
        _ => None,
    })
    .await
}

/// docs/dictation.md §35: the 从选中文字学习 hotkey is unassigned by default, validated and
/// canonicalised like the others, never one of the other three chords (and they are never it).
#[tokio::test]
async fn the_learn_hotkey_is_one_of_four_distinct_chords() {
    let mut node = start(FakeInjector::paste(), None, false);
    let settings = wait(&mut node, |e| if let CoreEvent::Ready { settings, .. } = e { Some(settings.clone()) } else { None }).await;
    assert_eq!(settings.learn_hotkey, None, "unassigned by default");
    node.handle.send(CoreCommand::SetCopyLastHotkey(Some("Ctrl+Shift+V".into()))).await.unwrap();
    node.handle.send(CoreCommand::SetLearnHotkey(Some("alt + control + l".into()))).await.unwrap();
    let s = wait(&mut node, |e| if let CoreEvent::Settings(s) = e { s.learn_hotkey.clone().map(|_| s.clone()) } else { None }).await;
    assert_eq!(s.learn_hotkey.as_deref(), Some("Ctrl+Alt+L"), "canonical form");
    for (cmd, needle) in [
        (CoreCommand::SetLearnHotkey(Some("Ctrl+Alt+Space".into())), "已用作听写快捷键"),
        (CoreCommand::SetLearnHotkey(Some("Ctrl+Alt+E".into())), "已用作「编辑选中文本」的快捷键"),
        (CoreCommand::SetLearnHotkey(Some("Ctrl+Shift+V".into())), "已用作「复制上一条结果」的快捷键"),
        (CoreCommand::SetLearnHotkey(Some("L".into())), "至少需要一个修饰键"),
        (CoreCommand::SetHotkey("Ctrl+Alt+L".into()), "已用作「从选中文字学习」的快捷键"),
        (CoreCommand::SetEditHotkey(Some("Ctrl+Alt+L".into())), "已用作「从选中文字学习」的快捷键"),
        (CoreCommand::SetCopyLastHotkey(Some("Ctrl+Alt+L".into())), "已用作「从选中文字学习」的快捷键"),
    ] {
        node.handle.send(cmd).await.unwrap();
        let message = wait(&mut node, |e| if let CoreEvent::Error(m) = e { Some(m.clone()) } else { None }).await;
        assert!(message.contains(needle), "{message}");
    }
    node.handle.send(CoreCommand::SetLearnHotkey(None)).await.unwrap();
    let s = wait(&mut node, |e| if let CoreEvent::Settings(s) = e { s.learn_hotkey.is_none().then(|| s.clone()) } else { None }).await;
    assert_eq!(s.learn_hotkey, None);
}

/// docs/dictation.md §35: the selection corrects the take; the correction is kept with the entry,
/// the notice waits while the take's result is on the pill, offers the pair, puts it in the
/// dictionary and takes it back, and goes after its stay. The copy went out with the shortcut's
/// modifiers held.
#[tokio::test]
async fn a_selection_corrects_the_take_and_the_notice_offers_its_pair() {
    let mut node = start(FakeInjector::paste().with_selection(FIXED), None, false);
    ready(&mut node).await;
    node.handle.send(CoreCommand::SetLearnHotkey(Some("Ctrl+Alt+L".into()))).await.unwrap();
    dictate(&mut node).await;
    // The take's 已插入 is still on the pill: the notice waits. The correction is kept first.
    learn(&node).await;
    let entry = history_with(&mut node, |e| e.corrected.is_some()).await;
    assert_eq!(entry.corrected.as_ref().unwrap().text, FIXED);
    assert_eq!(entry.text, HEARD, "the inserted text stays");
    let waiting = notice(&mut node).await.expect("a notice");
    assert!(!waiting.shown, "behind the take's result: {waiting:?}");
    let LearnOutcome::Learned { pairs, safe } = &waiting.outcome else { panic!("{waiting:?}") };
    assert_eq!(pairs.iter().map(|p| (p.heard.as_str(), p.polished.as_str())).collect::<Vec<_>>(), [("Quinn for", "Qwen 4")]);
    assert!(safe);
    assert_eq!(pairs[0].history_id, entry.id);
    assert_eq!(node.injector.copies(), [vec![Modifier::Ctrl, Modifier::Alt]], "the shortcut's modifiers are released first");
    idle(&mut node).await;
    let shown = notice(&mut node).await.expect("shown once the pill is idle");
    assert!(shown.shown && shown.id == waiting.id);

    // The dictionary changes first, then the notice says so.
    node.handle.send(CoreCommand::LearnNoticeAccept { id: shown.id }).await.unwrap();
    let entries = wait(&mut node, |e| if let CoreEvent::Dictionary(d) = e { Some(d.clone()) } else { None }).await;
    assert_eq!(entries.iter().map(|e| (e.term.as_str(), e.heard_as.clone())).collect::<Vec<_>>(), [("Qwen 4", vec!["Quinn for".to_owned()])]);
    let accepted = notice(&mut node).await.expect("the notice stays");
    let added = accepted.accepted.clone().expect("加入词典 ran");
    assert_eq!((added.added, added.refused), (1, 0));

    node.handle.send(CoreCommand::LearnNoticeUndo { id: shown.id }).await.unwrap();
    let entries = wait(&mut node, |e| if let CoreEvent::Dictionary(d) = e { Some(d.clone()) } else { None }).await;
    assert!(entries.is_empty(), "{entries:?}");
    let undone = notice(&mut node).await.expect("the notice stays");
    assert_eq!(undone.undone, Some(voltip_core::vocabulary::BatchUndoOutcome::Undone));

    let started = Instant::now();
    assert_eq!(notice(&mut node).await, None, "gone after its stay");
    assert!(started.elapsed() >= DWELL - Duration::from_millis(50), "timed from the last button: {:?}", started.elapsed());
}

/// docs/dictation.md §35: behind a result card the notice waits without a timer and shows, timed
/// from then, once the card is closed; a take that starts clears it; with the pill off there is
/// none, and the correction is kept all the same.
#[tokio::test]
async fn the_notice_waits_for_a_result_card_goes_with_a_take_and_needs_the_pill() {
    let mut node = start(FakeInjector::err("denied").with_selection(FIXED), None, true);
    ready(&mut node).await;
    let session = dictate(&mut node).await;
    learn(&node).await;
    let waiting = notice(&mut node).await.expect("a notice");
    assert!(!waiting.shown);
    no_notice(&mut node, DWELL * 2, "no timer while the card is up").await;
    node.handle.send(CoreCommand::DictationDismiss { session }).await.unwrap();
    let shown = notice(&mut node).await.expect("shown after the card");
    assert!(shown.shown);
    let started = Instant::now();
    assert_eq!(notice(&mut node).await, None);
    assert!(started.elapsed() >= DWELL - Duration::from_millis(50), "timed from when it showed: {:?}", started.elapsed());

    learn(&node).await;
    let LearnNotice { shown: true, outcome: LearnOutcome::Learned { .. }, .. } = notice(&mut node).await.expect("a notice") else { panic!("not shown") };
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    assert_eq!(notice(&mut node).await, None, "a take clears it");
    node.handle.send(CoreCommand::DictationCancel).await.unwrap();
    idle(&mut node).await;

    node.handle.send(CoreCommand::SetOverlay(OverlayPlacement::Off)).await.unwrap();
    wait(&mut node, |e| matches!(e, CoreEvent::Settings(s) if s.overlay == OverlayPlacement::Off).then_some(())).await;
    learn(&node).await;
    no_notice(&mut node, Duration::from_millis(500), "no pill, no notice").await;
}

/// docs/dictation.md §35: what the notice says when there is nothing to learn from.
#[tokio::test]
async fn the_notice_says_why_nothing_was_learned() {
    let outcome = async |injector: FakeInjector, probe: Option<Arc<dyn ForegroundProbe>>, history: bool, take: bool| {
        let mut node = start(injector, probe, false);
        ready(&mut node).await;
        if !history {
            node.handle.send(CoreCommand::SetHistory(voltip_core::HistorySettings { enabled: false, keep: 500 })).await.unwrap();
            wait(&mut node, |e| matches!(e, CoreEvent::Settings(s) if !s.history.enabled).then_some(())).await;
        }
        if take {
            dictate(&mut node).await;
            idle(&mut node).await;
        }
        learn(&node).await;
        let outcome = notice(&mut node).await.expect("a notice").outcome;
        (outcome, node.injector.copies().len())
    };
    assert_eq!(outcome(FakeInjector::paste().with_selection(FIXED), None, false, false).await, (LearnOutcome::HistoryOff, 0));
    let terminal: Arc<dyn ForegroundProbe> = Arc::new(FakeProbe::app("windowsterminal", "Terminal", None));
    let in_terminal = FakeInjector::paste().with_selection(FIXED).with_terminals(|id| id == "windowsterminal");
    assert_eq!(outcome(in_terminal, Some(terminal), true, true).await, (LearnOutcome::Terminal, 0), "no key pressed in a terminal");
    assert_eq!(outcome(FakeInjector::paste(), None, true, true).await, (LearnOutcome::NoSelection, 1));
    assert_eq!(outcome(FakeInjector::paste().with_selection("今天天气不错"), None, true, true).await, (LearnOutcome::NoMatch { context: false }, 1));
    assert_eq!(outcome(FakeInjector::paste().with_selection(HEARD), None, true, true).await, (LearnOutcome::Unchanged, 1));
    let long = "字".repeat(8001);
    assert!(matches!(outcome(FakeInjector::paste().with_selection(&long), None, true, true).await, (LearnOutcome::TooLong { max: 8000 }, 1)));
}

/// Regression (goal review of PR #260, 2026-11-18): 「查看」 and 「打开设置」 open a page in the main
/// window and tell the core nothing, so the notice went at its first deadline although the user
/// had just pressed a button (D3: every button starts the 8 seconds again). Now they keep the
/// notice (`LearnNoticeKeep`), as 加入词典 and 撤销 do; another notice's id changes nothing.
#[tokio::test]
async fn regression_view_and_open_settings_start_the_notices_stay_again() {
    let mut node = start(FakeInjector::paste().with_selection(FIXED), None, false);
    ready(&mut node).await;
    dictate(&mut node).await;
    idle(&mut node).await;
    learn(&node).await;
    let shown = notice(&mut node).await.expect("a notice");
    assert!(shown.shown);
    // Pressed late in the stay: the notice is there a whole stay after it.
    tokio::time::sleep(DWELL * 3 / 4).await;
    node.handle.send(CoreCommand::LearnNoticeKeep { id: shown.id + 1 }).await.unwrap();
    node.handle.send(CoreCommand::LearnNoticeKeep { id: shown.id }).await.unwrap();
    let pressed = Instant::now();
    let kept = notice(&mut node).await.expect("the notice stays");
    assert_eq!(kept, shown, "nothing on it changes");
    assert_eq!(notice(&mut node).await, None, "gone after its stay");
    assert!(pressed.elapsed() >= DWELL - Duration::from_millis(50), "timed from the button: {:?}", pressed.elapsed());
}

/// Regression (goal review of PR #260, 2026-11-18): on macOS the voice edit's terminal table is
/// empty (Cmd+C is safe there), and the learn task asked that table, so a selection in Terminal or
/// iTerm2 was copied and compared. Learning asks the port's own question, which names macOS's
/// terminals: no key is pressed there.
#[tokio::test]
async fn regression_a_terminal_the_voice_edit_allows_is_still_not_learned_from() {
    let terminal: Arc<dyn ForegroundProbe> = Arc::new(FakeProbe::app("com.apple.terminal", "Terminal", None));
    // The voice edit's table (macOS: none) says no; learning's says yes.
    let reader = Arc::new(FakeFocusReader::around("我在用 ", "Qwen 4", " max 做测试"));
    let injector = FakeInjector::paste()
        .with_selection(FIXED)
        .with_terminals(|_| false)
        .with_program_output(|id| id == "com.apple.terminal")
        .with_focus_reader(reader.clone());
    let mut node = start(injector, Some(terminal), false);
    ready(&mut node).await;
    dictate(&mut node).await;
    idle(&mut node).await;
    learn(&node).await;
    let outcome = notice(&mut node).await.expect("a notice").outcome;
    assert_eq!(outcome, LearnOutcome::Terminal);
    assert!(node.injector.copies().is_empty(), "no key pressed in the terminal: {:?}", node.injector.copies());
    assert!(reader.calls().contexts.is_empty(), "nothing read in the terminal either");
}

/// The application in front, as the probe names it: the learning read needs a name, as the insert
/// check does (docs/dictation.md §28.2).
fn notepad() -> Option<Arc<dyn ForegroundProbe>> {
    Some(Arc::new(FakeProbe::app("notepad", "Notepad", None)))
}

/// What one 从选中文字学习 after one take came to, and what the reader was asked.
async fn learned_with(injector: FakeInjector, reader: &Arc<FakeFocusReader>) -> (LearnOutcome, Option<HistoryEntry>, Vec<(String, Option<String>)>) {
    let mut node = start(injector.with_focus_reader(reader.clone()), notepad(), false);
    ready(&mut node).await;
    dictate(&mut node).await;
    idle(&mut node).await;
    learn(&node).await;
    // The correction is kept before the notice comes: collect it on the way.
    let mut kept = None;
    let outcome = loop {
        match tokio::time::timeout(STEP, node.events.recv()).await.expect("event within 10 s").expect("core alive") {
            CoreEvent::History { recent, .. } => kept = recent.iter().find(|e| e.corrected.is_some()).cloned().or(kept),
            CoreEvent::LearnNotice(Some(n)) => break n.outcome,
            _ => {}
        }
    };
    (outcome, kept, reader.calls().contexts)
}

fn pairs_of(outcome: &LearnOutcome) -> Vec<(String, String)> {
    match outcome {
        LearnOutcome::Learned { pairs, .. } => pairs.iter().map(|p| (p.heard.clone(), p.polished.clone())).collect(),
        other => panic!("not learned: {other:?}"),
    }
}

/// docs/dictation.md §35. Regression, 2026-11-25 (the user on windows-local): with only the
/// corrected words selected (「Qwen 4」 of 「…Qwen 4 Max…」), the pill said 「最近50次听写里没有找到这段文字」:
/// they share no word with the take. Now the paragraph around the selection is read through the
/// focused control (the copied selection as the hint) and the take is found.
#[tokio::test]
async fn regression_only_the_corrected_words_selected_are_learned_from_their_paragraph() {
    let reader = Arc::new(FakeFocusReader::around("我在用 ", "Qwen 4", " max 做测试"));
    let (outcome, entry, asked) = learned_with(FakeInjector::paste().with_selection("Qwen 4"), &reader).await;
    assert_eq!(pairs_of(&outcome), [("Quinn for".to_owned(), "Qwen 4".to_owned())]);
    assert_eq!(entry.expect("kept").corrected.unwrap().text, FIXED);
    assert_eq!(asked, [("notepad".to_owned(), Some("Qwen 4".to_owned()))], "the reader is asked with the copied selection");
}

/// Nothing selected, the caret in the corrected sentence: its paragraph is enough. Regression of
/// the same report: the pill said 「没有选中文字」.
#[tokio::test]
async fn regression_with_nothing_selected_the_paragraph_at_the_caret_is_learned() {
    let reader = Arc::new(FakeFocusReader::around(FIXED, "", ""));
    let (outcome, entry, asked) = learned_with(FakeInjector::paste(), &reader).await;
    assert_eq!(pairs_of(&outcome), [("Quinn for".to_owned(), "Qwen 4".to_owned())]);
    assert_eq!(entry.expect("kept").corrected.unwrap().text, FIXED);
    assert_eq!(asked, [("notepad".to_owned(), None)]);
}

/// docs/dictation.md §35: the 没有找到 notice says whether the text around the cursor was read; a
/// paragraph that does not hold the take falls back to the selection alone.
#[tokio::test]
async fn the_notice_says_whether_the_text_around_the_cursor_was_read() {
    // Unreadable: the selection alone, not found.
    let unreadable = Arc::new(FakeFocusReader::refusing(UnconfirmedWhy::Unreadable));
    let (outcome, _, _) = learned_with(FakeInjector::paste().with_selection("今天天气不错"), &unreadable).await;
    assert_eq!(outcome, LearnOutcome::NoMatch { context: false });
    // Read, and neither the paragraph nor the selection is a take.
    let read = Arc::new(FakeFocusReader::around("今天", "天气", "不错"));
    let (outcome, _, _) = learned_with(FakeInjector::paste().with_selection("天气"), &read).await;
    assert_eq!(outcome, LearnOutcome::NoMatch { context: true });
    // Read, but the paragraph holds no take while the copied selection does: the selection wins.
    let stale = Arc::new(FakeFocusReader::around("完全不同的", "内容", ""));
    let (outcome, entry, _) = learned_with(FakeInjector::paste().with_selection(FIXED), &stale).await;
    assert_eq!(pairs_of(&outcome), [("Quinn for".to_owned(), "Qwen 4".to_owned())]);
    assert_eq!(entry.expect("kept").corrected.unwrap().text, FIXED);
    // Nothing selected and nothing read: still 没有选中文字.
    let (outcome, _, _) = learned_with(FakeInjector::paste(), &unreadable).await;
    assert_eq!(outcome, LearnOutcome::NoSelection);
}
