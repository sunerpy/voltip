#![allow(clippy::unwrap_used, clippy::expect_used)]
//! 自动跟踪输入框里的修改 (docs/dictation.md §35.10) through the real core task, on the dictation
//! fakes: a take written to a field whose control is kept and read again ([`FakeFocusReader::following`]
//! and its feed), the user's change stored with the entry (`via: follow`) and taken back, never a
//! notice; and every way the follow ends or never starts: the switch, the history, the insert check,
//! a paste that did not arrive, a new take, 移除, a correction taught with 从选中文字学习, a delivery
//! that started before the switch went off. The phone's texts are in `e2e.rs`, which has the relay.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use voltip_core::dictation::fakes::{FakeAudio, FakeFocusReader, FakeInjector, FakeProbe, FakeTranscriber, FollowFeed};
use voltip_core::dictation::{DictationPorts, ForegroundProbe, Transcriber, UnconfirmedWhy};
use voltip_core::history::CorrectionVia;
use voltip_core::vocabulary::follow::FollowTimes;
use voltip_core::vocabulary::{MAX_SUGGESTIONS, suggestions};
use voltip_core::{
    AppCore, CoreCommand, CoreConfig, CoreEvent, CoreHandle, DictationPhase, EdgeSource, HistoryEntry, HistoryReader, ResolvedEngines, Settings, SettingsStore,
};
use voltip_identity::MemorySecretStore;

const STEP: Duration = Duration::from_secs(10);
/// What the fake recogniser hears; inserted as it is (no AI clean-up configured).
const HEARD: &str = "我在用 Quinn for max 做测试";
/// What the user corrects it to in the field.
const FIXED: &str = "我在用 Qwen 4 max 做测试";
/// What the field held before the take.
const BEFORE: &str = "笔记：";

/// The follow's times in these tests: a change stands 150 ms, the field is read every 20 ms.
const TIMES: FollowTimes = FollowTimes {
    stable: Duration::from_millis(150),
    gone_stable: Duration::from_millis(75),
    idle: Duration::from_secs(20),
    max: Duration::from_secs(30),
    every: Duration::from_millis(20),
    fast: Duration::from_millis(10),
    fast_for: Duration::from_millis(200),
};

/// The field right after the paste.
fn pasted() -> String {
    format!("{BEFORE}{HEARD}")
}

struct Node {
    handle: CoreHandle,
    events: mpsc::Receiver<CoreEvent>,
    reader: Arc<FakeFocusReader>,
    feed: FollowFeed,
    transcriber: Arc<FakeTranscriber>,
    /// The history's newest entries, from its last event.
    recent: Vec<HistoryEntry>,
    /// The newest entry's correction each time it changed: its text and how it was made.
    corrections: Vec<Option<(String, CorrectionVia)>>,
    /// Learn notices shown (the follow never shows one).
    notices: usize,
    _dir: tempfile::TempDir,
}

/// A core whose fields read as `after_paste` once a take is pasted into them, with the follow
/// switched on and `tune` applied to the saved settings; its injector pastes and reads through the
/// follow's reader.
fn start(tune: impl FnOnce(&mut Settings), transcriber: FakeTranscriber, injector: FakeInjector, after_paste: &str) -> Node {
    let dir = tempfile::tempdir().unwrap();
    let mut settings = Settings { relay_enabled: false, engines: voltip_core::dictation::fakes::fake_engines(), ..Settings::default() };
    settings.engines.follow_edits = true;
    tune(&mut settings);
    SettingsStore::new(dir.path()).save(&settings).unwrap();
    let (reader, feed) = FakeFocusReader::following(BEFORE, after_paste);
    let reader = Arc::new(reader);
    let injector = Arc::new(injector.with_focus_reader(reader.clone()));
    let transcriber = Arc::new(transcriber);
    let recogniser = transcriber.clone();
    let probe = Arc::new(FakeProbe::app("notepad", "Notepad", None));
    probe.set_window(Some(7));
    let ports = DictationPorts {
        audio: Arc::new(FakeAudio::speech()),
        injector,
        factory: Arc::new(move |_: &ResolvedEngines| (recogniser.clone() as Arc<dyn Transcriber>, None)),
        models: None,
        streaming: None,
        probe: Some(probe as Arc<dyn ForegroundProbe>),
        service_probe: None,
        segmenter: None,
    };
    let mut cfg = CoreConfig::new(dir.path().to_path_buf());
    cfg.default_device_name = "Follow Test".into();
    cfg.follow_times = TIMES;
    let (handle, events) = AppCore::start_with(cfg, Arc::new(MemorySecretStore::new()), ports).unwrap();
    Node { handle, events, reader, feed, transcriber, recent: Vec::new(), corrections: vec![None], notices: 0, _dir: dir }
}

/// A core as [`start`] with the follow on and a field that receives the paste.
fn following() -> Node {
    start(|_| {}, FakeTranscriber::ok(HEARD), FakeInjector::paste(), &pasted())
}

impl Node {
    fn fold(&mut self, ev: &CoreEvent) {
        match ev {
            CoreEvent::History { recent, .. } => {
                self.recent = recent.clone();
                let newest = recent.first().and_then(|e| e.corrected.as_ref()).map(|c| (c.text.clone(), c.via));
                if self.corrections.last() != Some(&newest) {
                    self.corrections.push(newest);
                }
            }
            CoreEvent::LearnNotice(Some(_)) => self.notices += 1,
            _ => {}
        }
    }

    async fn wait<T>(&mut self, mut pick: impl FnMut(&CoreEvent) -> Option<T>) -> T {
        loop {
            let ev = tokio::time::timeout(STEP, self.events.recv()).await.expect("event within 10 s").expect("core alive");
            self.fold(&ev);
            if let Some(v) = pick(&ev) {
                return v;
            }
        }
    }

    /// Until `cond` holds on what was folded and what the reader was asked; events are folded
    /// meanwhile.
    async fn until(&mut self, what: &str, cond: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + STEP;
        loop {
            while let Ok(ev) = self.events.try_recv() {
                self.fold(&ev);
            }
            if cond(self) {
                return;
            }
            assert!(Instant::now() < deadline, "{what}: not within {STEP:?}");
            if let Ok(Some(ev)) = tokio::time::timeout(Duration::from_millis(5), self.events.recv()).await {
                self.fold(&ev);
            }
        }
    }

    /// Events folded for `within`, while nothing is expected to happen.
    async fn quiet_for(&mut self, within: Duration) {
        let deadline = Instant::now() + within;
        while let Ok(Some(ev)) = tokio::time::timeout(deadline.saturating_duration_since(Instant::now()), self.events.recv()).await {
            self.fold(&ev);
        }
    }

    async fn ready(&mut self) {
        self.wait(|e| matches!(e, CoreEvent::Ready { .. }).then_some(())).await;
    }

    /// One take of [`HEARD`], to its terminal phase.
    async fn dictate(&mut self) {
        self.handle.send(CoreCommand::DictationStart).await.unwrap();
        self.wait(|e| matches!(e, CoreEvent::Dictation(s) if matches!(s.phase, DictationPhase::Listening { .. })).then_some(())).await;
        self.handle.send(CoreCommand::DictationStop).await.unwrap();
        self.wait(|e| matches!(e, CoreEvent::Dictation(s) if s.phase.is_terminal()).then_some(())).await;
    }

    /// One take, recorded, and its follow reading the field.
    async fn followed_take(&mut self) -> HistoryEntry {
        let before = self.recent.len();
        self.dictate().await;
        self.until("the take is in the history", |n| n.recent.len() > before).await;
        let reads = self.reader.calls().follow_reads;
        self.until("the field is read again", |n| n.reader.calls().follow_reads >= reads + 2).await;
        self.recent[0].clone()
    }

    /// The newest entry's correction, as last folded.
    fn corrected(&self) -> Option<(String, CorrectionVia)> {
        self.corrections.last().cloned().flatten()
    }
}

/// The switch off (the default): the paste is checked as before, no control is kept and the field
/// is never read again.
#[tokio::test]
async fn with_the_switch_off_no_control_is_kept_or_read() {
    let mut node = start(|s| s.engines.follow_edits = false, FakeTranscriber::ok(HEARD), FakeInjector::paste(), &pasted());
    node.ready().await;
    node.dictate().await;
    node.until("the take is in the history", |n| !n.recent.is_empty()).await;
    node.quiet_for(TIMES.stable).await;
    let calls = node.reader.calls();
    assert_eq!((calls.watched.len(), calls.kept, calls.follow_reads, calls.released), (1, 0, 0, 0), "{calls:?}");
    assert_eq!(calls.order, ["watch"]);
}

/// docs/dictation.md §35.10: the user changes the take in the field and stops; once the change has
/// stood, the entry carries it (`via: follow`), for the learning suggestions — no notice, nothing
/// added to the dictionary. Put back as it was, the follow's correction goes again.
#[tokio::test]
async fn a_change_made_in_the_field_is_stored_with_the_take_and_taken_back_with_it() {
    let mut node = following();
    node.ready().await;
    node.feed.field(&pasted(), "", "");
    let entry = node.followed_take().await;
    assert_eq!((entry.text.as_str(), entry.corrected.as_ref()), (HEARD, None));
    node.feed.field(&format!("{BEFORE}{FIXED}"), "", "");
    node.until("the change is stored", |n| n.corrected().is_some()).await;
    assert_eq!(node.corrected(), Some((FIXED.to_owned(), CorrectionVia::Follow)));
    assert_eq!(node.recent[0].id, entry.id, "on the take's own entry");
    // The learning suggestions read it from the history as the follow's.
    let texts = HistoryReader::new(node._dir.path()).recent_texts(50).unwrap();
    assert_eq!((texts[0].corrected.as_deref(), texts[0].corrected_via), (Some(FIXED), Some(CorrectionVia::Follow)));
    let found = suggestions(&[], &[], &texts, &[], MAX_SUGGESTIONS);
    let pairs: Vec<_> = found.pairs.iter().map(|p| (p.heard.as_str(), p.polished.as_str(), p.from_edits, p.from_follow)).collect();
    assert_eq!(pairs, [("Quinn for", "Qwen 4", 0, 1)]);
    assert_eq!(found.repeated, 1, "shown by default");
    node.feed.field(&pasted(), "", "");
    node.until("the follow's correction goes", |n| n.corrected().is_none()).await;
    assert_eq!(node.corrections, [None, Some((FIXED.to_owned(), CorrectionVia::Follow)), None]);
    assert_eq!(node.notices, 0, "the follow shows no notice");
    let calls = node.reader.calls();
    assert_eq!((calls.kept, calls.released), (1, 0), "still followed");
}

/// The take leaves the field (sent, deleted) after its change stood: the change is stored once, and
/// the control is released as the follow ends.
#[tokio::test]
async fn a_take_that_leaves_the_field_after_its_change_stood_is_stored_once_and_released() {
    let mut node = following();
    node.ready().await;
    node.feed.field(&pasted(), "", "");
    node.followed_take().await;
    let reads = node.reader.calls().follow_reads;
    node.feed.field(&format!("{BEFORE}{FIXED}"), "", "");
    // Ten readings at least 10 ms apart: the change stood longer than `gone_stable`.
    node.until("the change is read", |n| n.reader.calls().follow_reads >= reads + 10).await;
    node.feed.field(BEFORE, "", "");
    node.until("the follow ends", |n| n.reader.calls().released == 1).await;
    node.quiet_for(TIMES.stable).await;
    assert_eq!(node.corrections, [None, Some((FIXED.to_owned(), CorrectionVia::Follow))], "written once");
    let calls = node.reader.calls();
    assert_eq!((calls.kept, calls.released), (1, 1));
}

/// A new take ends the last one's follow and releases its control before the new paste is
/// checked; the new take is followed in turn.
#[tokio::test]
async fn a_new_take_ends_the_follow_before_its_paste_is_checked() {
    let mut node = following();
    node.ready().await;
    node.feed.field(&pasted(), "", "");
    node.followed_take().await;
    node.followed_take().await;
    assert_eq!(node.recent.len(), 2);
    assert_eq!(node.reader.calls().order, ["watch", "keep", "release", "watch", "keep"]);
}

/// A text pasted from the history (or a result card) goes into the field without a take: the
/// follow ends and releases its control before it goes in. The phone's texts are in `e2e.rs`.
#[tokio::test]
async fn a_history_paste_ends_the_follow() {
    use voltip_core::paste::{CopyReason, PasteTarget};
    let mut node = following();
    node.ready().await;
    node.feed.field(&pasted(), "", "");
    node.followed_take().await;
    node.handle.send(CoreCommand::PasteText { request_id: 1, text: "另一段文字".into(), target: PasteTarget::CopyOnly(CopyReason::NoProbe) }).await.unwrap();
    node.wait(|e| matches!(e, CoreEvent::PasteResult { request_id: 1, .. }).then_some(())).await;
    node.until("the control is released", |n| n.reader.calls().released == 1).await;
    let reads = node.reader.calls().follow_reads;
    node.feed.field(&format!("{BEFORE}{FIXED}"), "", "");
    node.quiet_for(TIMES.stable * 2).await;
    assert_eq!(node.reader.calls().follow_reads, reads, "the field is not read any more");
    assert_eq!(node.corrected(), None);
}

/// Regression (docs/dictation.md §35.10, plan review 2026-11-26): the switch turned off while a
/// take is recognised. Its delivery started with the switch on, and kept its paste's control, which
/// nobody took or released (`kept: 1, released: 0`). Now a delivery that started before the
/// engine was configured again keeps nothing, and the field is never read.
#[tokio::test]
async fn regression_a_take_recognised_while_the_switch_goes_off_keeps_no_control() {
    let mut node = start(|_| {}, FakeTranscriber::slow(HEARD, Duration::from_millis(500)), FakeInjector::paste(), &pasted());
    let settings = node.wait(|e| if let CoreEvent::Ready { settings, .. } = e { Some(settings.clone()) } else { None }).await;
    node.feed.field(&pasted(), "", "");
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    node.wait(|e| matches!(e, CoreEvent::Dictation(s) if matches!(s.phase, DictationPhase::Listening { .. })).then_some(())).await;
    node.handle.send(CoreCommand::DictationStop).await.unwrap();
    // The recording is with the recogniser: its delivery has started, with the switch on.
    node.until("the take is being recognised", |n| n.transcriber.calls() == 1).await;
    let mut off = settings.engines.clone();
    off.follow_edits = false;
    node.handle.send(CoreCommand::SetEngines(off)).await.unwrap();
    node.wait(|e| matches!(e, CoreEvent::Dictation(s) if s.phase.is_terminal()).then_some(())).await;
    node.until("the take is in the history", |n| !n.recent.is_empty()).await;
    node.quiet_for(TIMES.stable).await;
    let calls = node.reader.calls();
    assert_eq!(calls.watched.len(), 1, "the paste was checked: {calls:?}");
    assert_eq!((calls.kept, calls.released, calls.follow_reads), (0, 0, 0), "nothing kept, nothing read: {calls:?}");
    assert_eq!(node.corrected(), None);
}

/// Neither with the history off (the take has no entry to carry a change) nor with 检查文字是否写入
/// off (no reader looks at the field) is the field followed; a control kept for an unrecorded take
/// is released.
#[tokio::test]
async fn with_the_history_or_the_insert_check_off_the_field_is_not_followed() {
    let mut node = start(|s| s.history.enabled = false, FakeTranscriber::ok(HEARD), FakeInjector::paste(), &pasted());
    node.ready().await;
    node.feed.field(&pasted(), "", "");
    node.dictate().await;
    node.until("the control is released", |n| n.reader.calls().released == 1).await;
    node.quiet_for(TIMES.stable).await;
    let calls = node.reader.calls();
    assert_eq!((calls.kept, calls.follow_reads, calls.released), (1, 0, 1), "{calls:?}");

    let mut node = start(|s| s.engines.insert_check = false, FakeTranscriber::ok(HEARD), FakeInjector::paste(), &pasted());
    node.ready().await;
    node.feed.field(&pasted(), "", "");
    node.dictate().await;
    node.until("the take is in the history", |n| !n.recent.is_empty()).await;
    node.quiet_for(TIMES.stable).await;
    let calls = node.reader.calls();
    assert_eq!((calls.watched.len(), calls.kept, calls.follow_reads), (0, 0, 0), "{calls:?}");
}

/// A paste the field did not receive (`not_written`), or one the reader could not confirm, is not
/// followed: only a written paste's control is kept.
#[tokio::test]
async fn a_paste_that_was_not_seen_written_is_not_followed() {
    let mut node = start(|_| {}, FakeTranscriber::ok(HEARD), FakeInjector::paste(), BEFORE);
    node.ready().await;
    node.feed.field(&pasted(), "", "");
    node.dictate().await;
    node.until("the take is in the history", |n| !n.recent.is_empty()).await;
    node.quiet_for(TIMES.stable).await;
    let calls = node.reader.calls();
    assert_eq!((calls.watched.len(), calls.kept, calls.follow_reads), (1, 0, 0), "{calls:?}");

    let mut node = start(|_| {}, FakeTranscriber::ok(HEARD), FakeInjector::paste(), &pasted());
    node.ready().await;
    // The follow's first reading fails: the follow ends at once and releases the control.
    node.feed.push(Err(UnconfirmedWhy::Unreadable));
    node.dictate().await;
    node.until("the control is released", |n| n.reader.calls().released == 1).await;
    let calls = node.reader.calls();
    assert_eq!((calls.kept, calls.follow_reads, calls.released), (1, 1, 1), "{calls:?}");
    assert_eq!(node.corrected(), None);
}

/// 历史详情 › 移除 on the followed take: its correction goes, the follow ends with its control,
/// and a later change in the field is not written back.
#[tokio::test]
async fn after_its_correction_is_removed_the_take_is_no_longer_followed() {
    let mut node = following();
    node.ready().await;
    node.feed.field(&pasted(), "", "");
    let entry = node.followed_take().await;
    node.feed.field(&format!("{BEFORE}{FIXED}"), "", "");
    node.until("the change is stored", |n| n.corrected().is_some()).await;
    node.handle.send(CoreCommand::HistoryClearCorrected(entry.id)).await.unwrap();
    node.until("the correction goes", |n| n.corrected().is_none()).await;
    node.until("the control is released", |n| n.reader.calls().released == 1).await;
    node.feed.field(&format!("{BEFORE}我在用 Qwen 4 Max 做测试"), "", "");
    node.quiet_for(TIMES.stable * 2).await;
    assert_eq!(node.corrected(), None, "nothing written back");
}

/// A correction the user taught with 从选中文字学习 is theirs: the follow ends, and nothing it
/// reads afterwards replaces it.
#[tokio::test]
async fn a_correction_taught_by_selection_is_never_overwritten_by_the_follow() {
    const TAUGHT: &str = "我在用 Qwen 4 Max 做测试";
    let mut node = start(|_| {}, FakeTranscriber::ok(HEARD), FakeInjector::paste().with_selection(TAUGHT), &pasted());
    node.ready().await;
    node.feed.field(&pasted(), "", "");
    node.followed_take().await;
    node.feed.field(&format!("{BEFORE}{FIXED}"), "", "");
    node.until("the change is stored", |n| n.corrected().is_some()).await;
    node.handle.send(CoreCommand::LearnSelection { source: EdgeSource::Hotkey }).await.unwrap();
    node.until("the user's correction is kept", |n| n.corrected().is_some_and(|(_, via)| via == CorrectionVia::Selection)).await;
    node.until("the control is released", |n| n.reader.calls().released == 1).await;
    node.feed.field(&format!("{BEFORE}{FIXED}，好的"), "", "");
    node.quiet_for(TIMES.stable * 2).await;
    assert_eq!(node.corrected(), Some((TAUGHT.to_owned(), CorrectionVia::Selection)));
}
