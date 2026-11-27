//! The insert check's readers of the focused control (docs/dictation.md §28). A platform module
//! answers three questions about the control that has the keyboard focus — which control it is,
//! whether it is the same as another one, and what it holds around its caret ([`FocusPlatform`]):
//! UI Automation on Windows ([`windows`]), the Accessibility API on macOS ([`macos`]), AT-SPI on
//! Linux ([`linux`]). [`Reader`] asks them on one worker thread of its own, bounds every answer by
//! [`FOCUS_READ_DEADLINE`], replaces a thread that stopped answering ([`REPLACE_AFTER`] deadlines
//! in a row; the stuck call is left to finish on the old thread), skips the applications whose
//! fields are not read (`voltip_platform::insert_check`), and settles a paste with the check's
//! rules ([`settle_by_polling`]); the desktop shell puts it behind the core's `FocusReader` port.
//!
//! A crate of its own, as `voltip-hooks` is, so CI runs the readers against real applications on
//! Windows and macOS desktops (`tests/focus_*.rs`) without building the app.
//!
//! What is read stays in memory for the length of one paste, or, while 自动跟踪输入框里的修改
//! follows the field a paste went to ([`Follow`], docs/dictation.md §35.10), until that ends: it
//! is never logged, stored or sent.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use parking_lot::Mutex;
use voltip_platform::HostOs;
pub use voltip_platform::insert_check::rules::{
    CONTEXT_CHARS, Expected, FOCUS_READ_DEADLINE, FocusContext, FocusText, POLL_EVERY, Sample, Settled, UnconfirmedWhy, settle_by_polling,
};
use voltip_platform::insert_check::rules::{InsertCheck, READ_BEFORE_CARET};

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;

/// Deadlines missed in a row after which the worker thread is replaced.
pub const REPLACE_AFTER: u32 = 3;

/// One platform's reading of the focused control, on the reader's worker thread.
pub trait FocusPlatform {
    /// A control as the platform names it (a UI Automation element, an `AXUIElement`, an AT-SPI
    /// object), kept on the worker thread between the readings of one paste.
    type Control;

    /// The control that has the keyboard focus now: `Err(Skipped)` for a password field,
    /// `Err(Unreadable)` when there is none or it cannot be asked.
    fn focused(&mut self) -> Result<Self::Control, UnconfirmedWhy>;

    /// `a` and `b` are the same control.
    fn same(&mut self, a: &Self::Control, b: &Self::Control) -> bool;

    /// What `control` holds around its caret; `None` when it has no text to read.
    fn text(&mut self, control: &Self::Control) -> Option<FocusText>;

    /// The paragraph of `control` around its selection, or its caret (docs/dictation.md §35,
    /// [`FocusContext`]); for a control that tells only its whole value, the one place of `hint`
    /// (the copied selection) in it. `None` when that cannot be read.
    fn context(&mut self, control: &Self::Control, hint: Option<&str>) -> Option<FocusContext> {
        let _ = (control, hint);
        None
    }
}

/// What the worker thread is asked.
enum Request {
    /// Read the focused control and keep it under the token: the paste's `before`. Tokens come
    /// from the reader, unique across its worker threads ([`Inner::tokens`]).
    Begin(u64, mpsc::SyncSender<Result<FocusText, UnconfirmedWhy>>),
    /// Read the focused control again and compare it with the one kept under the token.
    Sample(u64, mpsc::SyncSender<Sample>),
    /// Forget the control kept under the token.
    End(u64),
    /// Read the focused control's paragraph around its selection, for 从选中文字学习, with the
    /// copied selection as the hint.
    Context(Option<String>, mpsc::SyncSender<Result<FocusContext, UnconfirmedWhy>>),
    /// Read the paragraph of the control kept under the token, when it still has the focus
    /// (自动跟踪输入框里的修改, docs/dictation.md §35.10).
    ContextOf(u64, Option<String>, mpsc::SyncSender<Result<FocusContext, UnconfirmedWhy>>),
}

/// The worker thread's loop: open the platform on this thread (COM, the accessibility bus), then
/// answer until the reader drops its end of the channel.
fn serve<P: FocusPlatform>(open: fn() -> Result<P, String>, requests: &mpsc::Receiver<Request>) {
    let mut platform = match open() {
        Ok(platform) => platform,
        Err(e) => {
            // Dropping the channel answers what is asked: no worker, nothing read. The next paste
            // opens a new thread and tries again (an accessibility bus may come up later).
            tracing::debug!(error = %e, "the insert check cannot read the focused control here");
            return;
        }
    };
    let mut kept: HashMap<u64, P::Control> = HashMap::new();
    for request in requests {
        match request {
            Request::Begin(token, reply) => {
                let answer = platform.focused().and_then(|control| {
                    let text = platform.text(&control).ok_or(UnconfirmedWhy::Unreadable)?;
                    kept.insert(token, control);
                    Ok(text)
                });
                let _ = reply.send(answer);
            }
            Request::Sample(token, reply) => {
                let sample = match kept.get(&token) {
                    None => Sample::Unreadable,
                    Some(before) => match platform.focused() {
                        Err(_) => Sample::Unreadable,
                        Ok(now) if !platform.same(before, &now) => Sample::OtherControl,
                        Ok(now) => platform.text(&now).map_or(Sample::Unreadable, Sample::Text),
                    },
                };
                let _ = reply.send(sample);
            }
            Request::End(token) => {
                kept.remove(&token);
            }
            Request::Context(hint, reply) => {
                let answer = platform.focused().and_then(|control| platform.context(&control, hint.as_deref()).ok_or(UnconfirmedWhy::Unreadable));
                let _ = reply.send(answer);
            }
            Request::ContextOf(token, hint, reply) => {
                // As `Sample`: a password field, nothing focused and an unknown token read as
                // nothing; another control is `ControlChanged`.
                let answer = match kept.get(&token) {
                    None => Err(UnconfirmedWhy::Unreadable),
                    Some(control) => match platform.focused() {
                        Err(_) => Err(UnconfirmedWhy::Unreadable),
                        Ok(now) if !platform.same(control, &now) => Err(UnconfirmedWhy::ControlChanged),
                        Ok(_) => platform.context(control, hint.as_deref()).ok_or(UnconfirmedWhy::Unreadable),
                    },
                };
                let _ = reply.send(answer);
            }
        }
    }
}

/// Why a request got no answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Unanswered {
    /// Not within the deadline.
    TimedOut,
    /// No worker thread (it could not start, or it ended).
    NoWorker,
}

/// The worker thread as the reader holds it.
struct Worker {
    requests: mpsc::Sender<Request>,
    /// Deadlines missed in a row.
    late: u32,
}

struct Inner {
    os: HostOs,
    deadline: Duration,
    spawn: Box<dyn Fn() -> Option<mpsc::Sender<Request>> + Send + Sync>,
    worker: Mutex<Option<Worker>>,
    /// The current worker's channel, for [`Inner::end`]: a release must not wait behind the read
    /// that holds `worker` until its answer comes (up to the deadline).
    sender: Mutex<Option<mpsc::Sender<Request>>>,
    /// The last token handed out. One counter for every worker thread, so a control kept before a
    /// thread was replaced never shares its token with one kept on the new thread.
    tokens: AtomicU64,
}

impl Inner {
    /// Ask the worker thread (started on first use, or again after it was replaced) and wait for
    /// its answer for at most the deadline.
    fn ask<T>(&self, make: impl FnOnce(mpsc::SyncSender<T>) -> Request) -> Result<T, Unanswered> {
        let mut slot = self.worker.lock();
        if slot.is_none() {
            *slot = (self.spawn)().map(|requests| Worker { requests, late: 0 });
            *self.sender.lock() = slot.as_ref().map(|worker| worker.requests.clone());
        }
        let Some(worker) = slot.as_mut() else { return Err(Unanswered::NoWorker) };
        let (reply, answer) = mpsc::sync_channel(1);
        if worker.requests.send(make(reply)).is_err() {
            *slot = None;
            return Err(Unanswered::NoWorker);
        }
        match answer.recv_timeout(self.deadline) {
            Ok(value) => {
                worker.late = 0;
                Ok(value)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                worker.late += 1;
                if worker.late >= REPLACE_AFTER {
                    tracing::warn!(late = worker.late, "the focus reader stopped answering; its thread is replaced");
                    *slot = None;
                }
                Err(Unanswered::TimedOut)
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                // The thread ended (the platform could not be opened, or it panicked).
                *slot = None;
                Err(Unanswered::NoWorker)
            }
        }
    }

    fn begin(&self) -> Result<(u64, FocusText), UnconfirmedWhy> {
        let token = self.tokens.fetch_add(1, Ordering::Relaxed) + 1;
        match self.ask(|reply| Request::Begin(token, reply)) {
            Ok(answer) => answer.map(|text| (token, text)),
            Err(Unanswered::TimedOut) => Err(UnconfirmedWhy::Timeout),
            Err(Unanswered::NoWorker) => Err(UnconfirmedWhy::Unreadable),
        }
    }

    fn sample(&self, token: u64) -> Sample {
        match self.ask(|reply| Request::Sample(token, reply)) {
            Ok(sample) => sample,
            Err(Unanswered::TimedOut) => Sample::TimedOut,
            Err(Unanswered::NoWorker) => Sample::Unreadable,
        }
    }

    fn context(&self, hint: Option<String>) -> Result<FocusContext, UnconfirmedWhy> {
        match self.ask(|reply| Request::Context(hint, reply)) {
            Ok(answer) => answer,
            Err(Unanswered::TimedOut) => Err(UnconfirmedWhy::Timeout),
            Err(Unanswered::NoWorker) => Err(UnconfirmedWhy::Unreadable),
        }
    }

    fn context_of(&self, token: u64, hint: Option<String>) -> Result<FocusContext, UnconfirmedWhy> {
        match self.ask(|reply| Request::ContextOf(token, hint, reply)) {
            Ok(answer) => answer,
            Err(Unanswered::TimedOut) => Err(UnconfirmedWhy::Timeout),
            Err(Unanswered::NoWorker) => Err(UnconfirmedWhy::Unreadable),
        }
    }

    /// Forget the control kept under `token`, without waiting: the channel is unbounded and the
    /// worker ends it when it gets there. After a thread was replaced the token is unknown to the
    /// new one, and the release does nothing.
    fn end(&self, token: u64) {
        if let Some(requests) = self.sender.lock().as_ref() {
            let _ = requests.send(Request::End(token));
        }
    }
}

/// The insert check's reader over one platform's [`FocusPlatform`]. Cheap to clone: the
/// clones share the worker thread.
#[derive(Clone)]
pub struct Reader(Arc<Inner>);

impl Reader {
    /// A reader that opens the platform with `open` on a worker thread of its own, on `os`.
    pub fn new<P: FocusPlatform + 'static>(os: HostOs, open: fn() -> Result<P, String>) -> Self {
        Self::with_deadline(os, open, FOCUS_READ_DEADLINE)
    }

    /// [`Reader::new`] with another per-read deadline (tests).
    pub fn with_deadline<P: FocusPlatform + 'static>(os: HostOs, open: fn() -> Result<P, String>, deadline: Duration) -> Self {
        let spawn = Box::new(move || {
            let (requests, incoming) = mpsc::channel();
            let started = std::thread::Builder::new().name("voltip-focus".into()).spawn(move || serve(open, &incoming));
            match started {
                Ok(_) => Some(requests),
                Err(e) => {
                    tracing::warn!(error = %e, "the focus reader's thread did not start");
                    None
                }
            }
        });
        Self(Arc::new(Inner { os, deadline, spawn, worker: Mutex::new(None), sender: Mutex::new(None), tokens: AtomicU64::new(0) }))
    }
}

impl Reader {
    /// Read the focused control right before a paste of `expected` into `app_id`, the probe's
    /// normalised id of the application the paste goes to (a paste whose application nobody named
    /// is not read at all, docs/dictation.md §28.2). An error says why there is nothing to compare
    /// with afterwards: a skipped application or field, nothing readable, or a reader that did not
    /// answer in time. Blocking, bounded by the deadline.
    pub fn watch(&self, app_id: &str, expected: &Expected) -> Result<Watch, UnconfirmedWhy> {
        if voltip_platform::insert_check::skips(self.0.os, app_id) {
            return Err(UnconfirmedWhy::Skipped);
        }
        let (token, before) = self.0.begin()?;
        Ok(Watch { inner: self.0.clone(), token, before, expected: expected.clone(), kept: false })
    }

    /// Read the focused control of `app_id` for 从选中文字学习 (docs/dictation.md §35): its
    /// paragraph around the selection or the caret, at most [`CONTEXT_CHARS`] characters on each
    /// side, with `hint` (the copied selection) placing it in a control that tells no caret. The
    /// insert check's rules hold: a terminal, a remote desktop, a virtual machine and a password
    /// field are never read, and the read is bounded by the deadline. Only the cut context leaves
    /// the worker thread. Blocking.
    pub fn context(&self, app_id: &str, hint: Option<&str>) -> Result<FocusContext, UnconfirmedWhy> {
        if voltip_platform::insert_check::skips(self.0.os, app_id) {
            return Err(UnconfirmedWhy::Skipped);
        }
        self.0.context(hint.map(str::to_owned))
    }
}

/// One paste's reading: the control and its text right before the paste.
pub struct Watch {
    inner: Arc<Inner>,
    token: u64,
    before: FocusText,
    expected: Expected,
    /// The control passed to a [`Follow`]: dropping the watch keeps it.
    kept: bool,
}

impl Watch {
    /// Read the control right away and then every [`POLL_EVERY`] for at most `within`, and decide
    /// with the check's rules. Blocking: every read is bounded by the reader's deadline.
    pub fn settle(self, within: Duration) -> Settled {
        self.decide(within)
    }

    /// [`Watch::settle`], keeping the control for 自动跟踪输入框里的修改 (docs/dictation.md §35.10)
    /// when the paste is written there; any other answer releases it as `settle` does.
    pub fn settle_and_keep(mut self, within: Duration) -> (Settled, Option<Follow>) {
        let settled = self.decide(within);
        if settled.check != InsertCheck::Written {
            return (settled, None);
        }
        self.kept = true;
        (settled, Some(Follow { inner: self.inner.clone(), token: self.token }))
    }

    fn decide(&self, within: Duration) -> Settled {
        let (settled, _took) = settle_by_polling(&self.before, &self.expected, within, POLL_EVERY, || self.inner.sample(self.token));
        settled
    }

    /// One reading of the watched control now (the measurement probe polls with it).
    pub fn sample(&self) -> Sample {
        self.inner.sample(self.token)
    }

    /// The reading right before the paste.
    pub fn before(&self) -> &FocusText {
        &self.before
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        if !self.kept {
            self.inner.end(self.token);
        }
    }
}

/// The control a paste was written to, kept while 自动跟踪输入框里的修改 follows it
/// (docs/dictation.md §35.10); dropping it releases the control without waiting.
pub struct Follow {
    inner: Arc<Inner>,
    token: u64,
}

impl Follow {
    /// The kept control's paragraph around its selection or caret ([`FocusContext`]), when it still
    /// has the keyboard focus: `Err(ControlChanged)` when another control has it, `Err(Unreadable)`
    /// for nothing focused, a password field or a control that no longer answers, `Err(Timeout)`
    /// past the deadline. Blocking, bounded by the reader's deadline.
    pub fn context(&self, hint: Option<&str>) -> Result<FocusContext, UnconfirmedWhy> {
        self.inner.context_of(self.token, hint.map(str::to_owned))
    }
}

impl Drop for Follow {
    fn drop(&mut self) {
        self.inner.end(self.token);
    }
}

impl Reader {
    /// The reader of this build's platform: UI Automation on Windows, the Accessibility API on
    /// macOS, AT-SPI on Linux; none elsewhere. Nothing is opened until the first paste.
    pub fn system() -> Option<Self> {
        #[cfg(target_os = "windows")]
        {
            Some(Self::new(HostOs::Windows, windows::open))
        }
        #[cfg(target_os = "macos")]
        {
            Some(Self::new(HostOs::Macos, macos::open))
        }
        #[cfg(target_os = "linux")]
        {
            Some(Self::new(HostOs::Linux, linux::open))
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        {
            None
        }
    }
}

/// `units` (UTF-16 code units, as UI Automation and the Accessibility API count) as text.
pub fn utf16_text(units: &[u16]) -> String {
    String::from_utf16_lossy(units)
}

/// The text before `caret` in `text` (UTF-16 units), at most `READ_BEFORE_CARET` units of it; the
/// end of `text` when the caret is past it.
pub fn utf16_before(text: &[u16], caret: usize) -> String {
    let end = caret.min(text.len());
    let start = end.saturating_sub(READ_BEFORE_CARET);
    utf16_text(&text[start..end])
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    use voltip_platform::insert_check::rules::InsertCheck;

    use super::*;

    /// What the fake field shows: its text and caret, its control id; `hang` makes every read
    /// sleep that long.
    #[derive(Clone, Default)]
    struct Field {
        control: u32,
        text: String,
        password: bool,
        hang: Option<Duration>,
        none: bool,
        /// The selection, in characters; the caret at the end of the text when `None`.
        selection: Option<(usize, usize)>,
        /// A control that tells its whole value but no caret.
        value_only: bool,
    }

    static FIELD: Mutex<Option<Field>> = Mutex::new(None);
    static OPENED: AtomicUsize = AtomicUsize::new(0);
    static SERIAL: Mutex<()> = Mutex::new(());

    struct Fake;

    impl FocusPlatform for Fake {
        type Control = u32;
        fn focused(&mut self) -> Result<u32, UnconfirmedWhy> {
            let field = FIELD.lock().clone().unwrap_or_default();
            if let Some(hang) = field.hang {
                std::thread::sleep(hang);
            }
            if field.none {
                return Err(UnconfirmedWhy::Unreadable);
            }
            if field.password {
                return Err(UnconfirmedWhy::Skipped);
            }
            Ok(field.control)
        }
        fn same(&mut self, a: &u32, b: &u32) -> bool {
            a == b
        }
        fn text(&mut self, _control: &u32) -> Option<FocusText> {
            let field = FIELD.lock().clone().unwrap_or_default();
            let n = field.text.chars().count();
            Some(FocusText::read(Some(n), Some(n), Some((n, n)), None, Some(&field.text)))
        }
        fn context(&mut self, _control: &u32, hint: Option<&str>) -> Option<FocusContext> {
            let field = FIELD.lock().clone().unwrap_or_default();
            if field.value_only {
                return FocusContext::find(&field.text, hint?);
            }
            let n = field.text.chars().count();
            let (start, end) = field.selection.unwrap_or((n, n));
            FocusContext::around(&field.text, start, end)
        }
    }

    fn open() -> Result<Fake, String> {
        OPENED.fetch_add(1, Ordering::SeqCst);
        Ok(Fake)
    }

    fn broken() -> Result<Fake, String> {
        Err("no accessibility bus".into())
    }

    fn set(field: Field) {
        *FIELD.lock() = Some(field);
    }

    fn reader(deadline: Duration) -> Reader {
        Reader::with_deadline(HostOs::Windows, open, deadline)
    }

    /// Paste `text` into the field: it shows `after` from then on.
    fn paste(reader: &Reader, text: &str, after: impl FnOnce(&mut Field)) -> Settled {
        let watch = reader.watch("notepad", &Expected::of(text)).expect("the field is read");
        let mut field = FIELD.lock().clone().unwrap_or_default();
        after(&mut field);
        set(field);
        watch.settle(Duration::from_millis(200))
    }

    #[test]
    fn a_paste_is_written_not_written_or_unconfirmed_from_the_platform_readings() {
        let _serial = SERIAL.lock();
        let reader = reader(Duration::from_secs(2));
        set(Field { control: 1, text: "前文".into(), ..Field::default() });
        assert_eq!(paste(&reader, "你好", |f| f.text.push_str("你好")).check, InsertCheck::Written);
        assert_eq!(paste(&reader, "你好", |_| {}).check, InsertCheck::NotWritten, "the field is as it was");
        let other = paste(&reader, "你好", |f| f.control = 2);
        assert_eq!(other, Settled::unconfirmed(UnconfirmedWhy::ControlChanged));
        let gone = paste(&reader, "你好", |f| f.none = true);
        assert_eq!(gone, Settled::unconfirmed(UnconfirmedWhy::Unreadable));
    }

    #[test]
    fn terminals_remote_screens_and_password_fields_are_never_read() {
        let _serial = SERIAL.lock();
        let reader = reader(Duration::from_secs(2));
        set(Field { control: 1, text: "x".into(), ..Field::default() });
        for skipped in ["windowsterminal", "mstsc"] {
            let watch = reader.watch(skipped, &Expected::of("你好"));
            assert_eq!(watch.err(), Some(UnconfirmedWhy::Skipped), "{skipped}");
        }
        set(Field { control: 1, text: "x".into(), password: true, ..Field::default() });
        assert_eq!(reader.watch("notepad", &Expected::of("你好")).err(), Some(UnconfirmedWhy::Skipped));
        // Any other application is read.
        set(Field { control: 1, text: "x".into(), ..Field::default() });
        assert!(reader.watch("notepad", &Expected::of("你好")).is_ok());
    }

    /// docs/dictation.md §35: 从选中文字学习 reads the selection's paragraph around it, with the
    /// insert check's rules for what is never read and how long a read may take.
    #[test]
    fn a_learning_read_gives_the_paragraph_around_the_selection_or_the_caret() {
        let _serial = SERIAL.lock();
        let reader = reader(Duration::from_secs(2));
        let text = "上一段\n今天测试一下Qwen 4 Max的推理能力。";
        set(Field { control: 1, text: text.into(), selection: Some((10, 16)), ..Field::default() });
        let c = reader.context("notepad", None).expect("read");
        assert_eq!((c.before.as_str(), c.selected.as_str(), c.after.as_str()), ("今天测试一下", "Qwen 4", " Max的推理能力。"));
        // Nothing selected: the paragraph up to the caret.
        set(Field { control: 1, text: text.into(), ..Field::default() });
        let c = reader.context("notepad", None).expect("read");
        assert_eq!((c.before.as_str(), c.selected.as_str(), c.after.as_str()), ("今天测试一下Qwen 4 Max的推理能力。", "", ""));
    }

    #[test]
    fn a_learning_read_skips_terminals_remote_screens_and_password_fields() {
        let _serial = SERIAL.lock();
        let reader = reader(Duration::from_secs(2));
        set(Field { control: 1, text: "x".into(), ..Field::default() });
        for skipped in ["windowsterminal", "mstsc"] {
            assert_eq!(reader.context(skipped, None).err(), Some(UnconfirmedWhy::Skipped), "{skipped}");
        }
        set(Field { control: 1, text: "x".into(), password: true, ..Field::default() });
        assert_eq!(reader.context("notepad", None).err(), Some(UnconfirmedWhy::Skipped));
        set(Field { control: 1, text: "x".into(), none: true, ..Field::default() });
        assert_eq!(reader.context("notepad", None).err(), Some(UnconfirmedWhy::Unreadable));
        set(Field { control: 1, text: "x".into(), hang: Some(Duration::from_millis(400)), ..Field::default() });
        let slow = Reader::with_deadline(HostOs::Windows, open, Duration::from_millis(100));
        assert_eq!(slow.context("notepad", None).err(), Some(UnconfirmedWhy::Timeout));
    }

    /// A control that tells its whole value but no caret is placed by the copied selection, when it
    /// is there exactly once.
    #[test]
    fn a_value_only_control_is_placed_by_the_copied_selection() {
        let _serial = SERIAL.lock();
        let reader = reader(Duration::from_secs(2));
        set(Field { control: 1, text: "今天测试一下Qwen 4 Max的推理能力。".into(), value_only: true, ..Field::default() });
        let c = reader.context("msedge", Some("Qwen 4")).expect("placed");
        assert_eq!((c.before.as_str(), c.after.as_str()), ("今天测试一下", " Max的推理能力。"));
        assert_eq!(reader.context("msedge", None).err(), Some(UnconfirmedWhy::Unreadable), "nothing copied: no place");
        assert_eq!(reader.context("msedge", Some("GPT")).err(), Some(UnconfirmedWhy::Unreadable));
    }

    /// Wait until the fake platform has been opened `n` times in all.
    fn wait_opened(n: usize) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while OPENED.load(Ordering::SeqCst) < n {
            assert!(Instant::now() < deadline, "opened {} times, not {n}", OPENED.load(Ordering::SeqCst));
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_reader_that_stops_answering_times_out_and_its_thread_is_replaced_after_three_deadlines() {
        let _serial = SERIAL.lock();
        let deadline = Duration::from_millis(100);
        let reader = reader(deadline);
        set(Field { control: 1, text: "x".into(), hang: Some(Duration::from_secs(2)), ..Field::default() });
        let opened = OPENED.load(Ordering::SeqCst);
        for missed in 1..=REPLACE_AFTER {
            let started = Instant::now();
            assert_eq!(reader.watch("notepad", &Expected::of("你好")).err(), Some(UnconfirmedWhy::Timeout));
            // Time it from the call: the answer comes at the deadline, not when the stuck read ends.
            assert!(started.elapsed() < Duration::from_millis(1500), "bounded by the deadline: {:?}", started.elapsed());
            if missed < REPLACE_AFTER {
                wait_opened(opened + 1);
                assert_eq!(OPENED.load(Ordering::SeqCst), opened + 1, "still the first thread after {missed}");
            }
        }
        // The stuck thread is left behind; the next paste opens a new one, which answers.
        set(Field { control: 1, text: "x".into(), ..Field::default() });
        let until = Instant::now() + Duration::from_secs(10);
        let watch = loop {
            if let Ok(watch) = reader.watch("notepad", &Expected::of("你好")) {
                break watch;
            }
            assert!(Instant::now() < until, "a new thread answers");
        };
        drop(watch);
        assert!(OPENED.load(Ordering::SeqCst) >= opened + 2, "a new thread after the deadlines");
    }

    /// docs/dictation.md §35.10: a written paste's control is kept and read again while it has the
    /// focus; another control, a password field, nothing focused and a late answer end the reading.
    #[test]
    fn a_follow_reads_the_kept_control_while_it_has_the_focus() {
        let _serial = SERIAL.lock();
        let reader = reader(Duration::from_millis(300));
        set(Field { control: 1, text: "我们讨论一下".into(), ..Field::default() });
        let watch = reader.watch("notepad", &Expected::of("千问四Max")).expect("read");
        set(Field { control: 1, text: "我们讨论一下千问四Max".into(), ..Field::default() });
        let (settled, follow) = watch.settle_and_keep(Duration::from_millis(200));
        assert_eq!(settled.check, InsertCheck::Written);
        let follow = follow.expect("a written paste keeps its control");
        assert_eq!(follow.context(None).expect("read").before, "我们讨论一下千问四Max");
        // The user corrects the take in place: the next reading has it.
        set(Field { control: 1, text: "我们讨论一下Qwen 4 Max".into(), ..Field::default() });
        assert_eq!(follow.context(None).expect("read").before, "我们讨论一下Qwen 4 Max");
        set(Field { control: 2, text: "别的输入框".into(), ..Field::default() });
        assert_eq!(follow.context(None).err(), Some(UnconfirmedWhy::ControlChanged));
        set(Field { control: 1, text: "x".into(), password: true, ..Field::default() });
        assert_eq!(follow.context(None).err(), Some(UnconfirmedWhy::Unreadable));
        set(Field { control: 1, text: "x".into(), none: true, ..Field::default() });
        assert_eq!(follow.context(None).err(), Some(UnconfirmedWhy::Unreadable));
        set(Field { control: 1, text: "x".into(), hang: Some(Duration::from_millis(900)), ..Field::default() });
        assert_eq!(follow.context(None).err(), Some(UnconfirmedWhy::Timeout));
        set(Field::default());
    }

    #[test]
    fn only_a_written_paste_keeps_its_control_and_dropping_the_follow_releases_it() {
        let _serial = SERIAL.lock();
        let reader = reader(Duration::from_secs(2));
        set(Field { control: 1, text: "前文".into(), ..Field::default() });
        let unchanged = reader.watch("notepad", &Expected::of("你好")).expect("read");
        let (settled, follow) = unchanged.settle_and_keep(Duration::from_millis(100));
        assert_eq!((settled.check, follow.is_none()), (InsertCheck::NotWritten, true));
        let moved = reader.watch("notepad", &Expected::of("你好")).expect("read");
        set(Field { control: 2, text: "前文你好".into(), ..Field::default() });
        let (settled, follow) = moved.settle_and_keep(Duration::from_millis(100));
        assert_eq!((settled, follow.is_none()), (Settled::unconfirmed(UnconfirmedWhy::ControlChanged), true));
        set(Field { control: 1, text: "前文".into(), ..Field::default() });
        let written = reader.watch("notepad", &Expected::of("你好")).expect("read");
        set(Field { control: 1, text: "前文你好".into(), ..Field::default() });
        let follow = written.settle_and_keep(Duration::from_millis(200)).1.expect("kept");
        let (inner, token) = (follow.inner.clone(), follow.token);
        drop(follow);
        // The release goes first on the worker's channel, so the next read finds nothing kept.
        assert_eq!(inner.context_of(token, None).err(), Some(UnconfirmedWhy::Unreadable));
    }

    /// A control kept across a replaced worker thread: the old token names nothing on the new
    /// thread. Regression, 2026-11-26 (the plan of 跟踪输入框里的修改): each thread counted its
    /// tokens from 1, so the first paste on the new thread got the old paste's token, and dropping
    /// the old paste's watch ended the new paste's control.
    #[test]
    fn regression_a_watch_from_before_a_replaced_thread_does_not_end_the_new_threads_control() {
        let _serial = SERIAL.lock();
        let reader = reader(Duration::from_millis(100));
        set(Field { control: 1, text: "前文".into(), ..Field::default() });
        let old = reader.watch("notepad", &Expected::of("你好")).expect("read");
        set(Field { control: 1, text: "前文".into(), hang: Some(Duration::from_secs(2)), ..Field::default() });
        for _ in 0..REPLACE_AFTER {
            assert_eq!(reader.watch("notepad", &Expected::of("你好")).err(), Some(UnconfirmedWhy::Timeout));
        }
        set(Field { control: 1, text: "前文".into(), ..Field::default() });
        let until = Instant::now() + Duration::from_secs(10);
        let new = loop {
            if let Ok(watch) = reader.watch("notepad", &Expected::of("你好")) {
                break watch;
            }
            assert!(Instant::now() < until, "a new thread answers");
        };
        drop(old);
        assert!(matches!(new.sample(), Sample::Text(_)), "the new paste's control is still kept");
    }

    /// Releasing a kept control does not wait for another read the worker thread is busy with.
    /// Regression, 2026-11-26: `end` took the lock a read holds until its answer (up to the
    /// deadline), so dropping a watch on the core's task could hold it that long.
    #[test]
    fn regression_releasing_a_control_does_not_wait_for_a_read_in_progress() {
        let _serial = SERIAL.lock();
        let reader = reader(Duration::from_secs(5));
        set(Field { control: 1, text: "前文".into(), ..Field::default() });
        let watch = reader.watch("notepad", &Expected::of("你好")).expect("read");
        set(Field { control: 1, text: "前文".into(), hang: Some(Duration::from_millis(1500)), ..Field::default() });
        let busy = {
            let reader = reader.clone();
            std::thread::spawn(move || reader.context("notepad", None))
        };
        // The read is under way once the worker is inside the slow platform call.
        std::thread::sleep(Duration::from_millis(200));
        let started = Instant::now();
        drop(watch);
        let took = started.elapsed();
        assert!(took < Duration::from_millis(300), "the release returned at once, not after the read: {took:?}");
        let _ = busy.join();
        set(Field::default());
    }

    #[test]
    fn a_reading_that_misses_its_deadline_after_the_paste_is_a_timeout_and_never_not_written() {
        let _serial = SERIAL.lock();
        let reader = reader(Duration::from_millis(300));
        set(Field { control: 1, text: "前文".into(), ..Field::default() });
        let settled = paste(&reader, "你好", |f| f.hang = Some(Duration::from_millis(900)));
        assert_eq!(settled, Settled::unconfirmed(UnconfirmedWhy::Timeout));
        set(Field::default());
    }

    #[test]
    fn a_platform_that_cannot_be_opened_reads_nothing_and_is_tried_again_next_time() {
        let reader = Reader::with_deadline(HostOs::Linux, broken, Duration::from_secs(2));
        for _ in 0..2 {
            assert_eq!(reader.watch("notepad", &Expected::of("你好")).err(), Some(UnconfirmedWhy::Unreadable));
        }
    }

    #[test]
    fn utf16_helpers_cut_the_text_before_the_caret() {
        let text: Vec<u16> = "前文😀ab".encode_utf16().collect();
        assert_eq!(utf16_before(&text, text.len()), "前文😀ab");
        assert_eq!(utf16_before(&text, 2), "前文");
        assert_eq!(utf16_before(&text, 99), "前文😀ab", "a caret past the end reads to the end");
        let long: Vec<u16> = "字".repeat(300).encode_utf16().collect();
        assert_eq!(utf16_before(&long, 300).chars().count(), READ_BEFORE_CARET);
    }
}
