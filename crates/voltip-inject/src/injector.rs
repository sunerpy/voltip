//! The injectors and the decisions they make, kept apart from the OS calls in
//! [`crate::platform`] / [`crate::linux`] so everything here runs on a headless host.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::clip::ClipSnapshot;
use crate::toolchain::{Chord, Delivery, PasteMethod};
use crate::{AfterPaste, FallbackCode, InjectError, InjectNote, InjectSession, Injection, Injector, PieceOutcome};

/// Linux default pause between writing the clipboard and pressing the paste chord, so the target
/// application observes the new content (`paste_delay_ms`, docs/dictation.md §14).
pub const PASTE_DELAY: Duration = Duration::from_millis(60);
/// Linux default time after the paste before the previous clipboard text is put back
/// (`paste_delay_after_ms`). Short on purpose: `live_inject` (§12) pastes segment after segment,
/// and a restore still pending when the next segment saves the clipboard would put an earlier
/// segment back instead of the user's text.
pub const PASTE_DELAY_AFTER: Duration = Duration::from_millis(120);
/// Windows / macOS settle delay, unchanged since the first release (B6 owns their tuning).
pub const PASTE_SETTLE: Duration = Duration::from_millis(40);
/// Windows / macOS restore delay: 600 ms is what was tested there; the shell can adjust it.
pub const DEFAULT_RESTORE_DELAY: Duration = Duration::from_millis(600);

/// Timing and shape of a clipboard paste. Constructor parameters for now (docs/dictation.md §14
/// lists them as the settings of a later increment).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PasteOptions {
    /// Which chord pastes.
    pub method: PasteMethod,
    /// Wait between the clipboard write and the chord.
    pub paste_delay: Duration,
    /// Wait between the chord and the clipboard restore.
    pub paste_delay_after: Duration,
    /// Add one space after the text so the next dictation does not glue onto it.
    pub append_trailing_space: bool,
}

impl Default for PasteOptions {
    fn default() -> Self {
        Self { method: PasteMethod::CtrlV, paste_delay: PASTE_DELAY, paste_delay_after: PASTE_DELAY_AFTER, append_trailing_space: false }
    }
}

impl PasteOptions {
    /// Defaults for `os` as [`std::env::consts::OS`] names it: Linux gets the tool-chain timings
    /// ([`PASTE_DELAY`] / [`PASTE_DELAY_AFTER`], [`PasteOptions::default`]); Windows and macOS keep
    /// the settle / restore delays they shipped with ([`PASTE_SETTLE`] / [`DEFAULT_RESTORE_DELAY`]).
    pub fn for_os(os: &str) -> Self {
        if os == "linux" { Self::default() } else { Self { paste_delay: PASTE_SETTLE, paste_delay_after: DEFAULT_RESTORE_DELAY, ..Self::default() } }
    }

    /// Same chord and delays, different restore delay.
    pub fn with_restore_delay(self, paste_delay_after: Duration) -> Self {
        Self { paste_delay_after, ..self }
    }

    /// The text as it is put on the clipboard: `text`, plus a trailing space when asked for.
    pub fn prepare(&self, text: &str) -> String {
        if self.append_trailing_space && !text.ends_with(char::is_whitespace) { format!("{text} ") } else { text.to_string() }
    }
}

/// The paste chord for the platform this binary runs on ([`PasteMethod::CtrlV`]).
pub fn paste_chord() -> Chord {
    paste_chord_for(std::env::consts::OS)
}

/// The `Ctrl+V` / `Cmd+V` chord for `os` as [`std::env::consts::OS`] names it.
pub fn paste_chord_for(os: &str) -> Chord {
    PasteMethod::CtrlV.chord_for(os)
}

/// Whether `snapshot` comes back after a paste of `ours`: when it holds anything, and anything
/// besides `ours` itself (putting the same text back would change nothing). An empty text the user
/// had copied is something.
pub fn puts_back(snapshot: &ClipSnapshot, ours: &str) -> bool {
    !snapshot.is_empty() && !snapshot.holds_only(ours)
}

/// Whether a display server is reachable, judged from the environment: on `linux` either
/// `DISPLAY` or `WAYLAND_DISPLAY` must be non-empty; other platforms always pass.
pub fn check_display(env: impl Fn(&str) -> Option<String>, os: &str) -> Result<(), InjectError> {
    if os != "linux" {
        return Ok(());
    }
    let set = |name: &str| env(name).is_some_and(|v| !v.trim().is_empty());
    if set("DISPLAY") || set("WAYLAND_DISPLAY") { Ok(()) } else { Err(InjectError::NoDisplay("neither DISPLAY nor WAYLAND_DISPLAY is set".into())) }
}

/// The clipboard as the injectors see it. [`crate::SystemClipboard`] is the real one.
pub trait ClipboardPort: Send + Sync {
    /// Current text, `None` when the clipboard is empty or holds something that is not text.
    fn read_text(&self) -> Result<Option<String>, InjectError>;
    /// Replace the clipboard content with `text`.
    fn write_text(&self, text: &str) -> Result<(), InjectError>;
    /// Empty the clipboard (the selection copy clears it first so any copy shows up as a change,
    /// docs/dictation.md §19). The default writes an empty text, which every reader treats as
    /// "nothing copied"; [`crate::SystemClipboard`] clears for real.
    fn clear(&self) -> Result<(), InjectError> {
        self.write_text("")
    }

    /// Everything the clipboard holds now, to put back after an injection (docs/dictation.md
    /// §14.3). The default keeps the text alone ([`ClipSnapshot::of_text`]); the platform ports keep
    /// every format they can read.
    fn snapshot(&self) -> Result<ClipSnapshot, InjectError> {
        Ok(ClipSnapshot::of_text(self.read_text()?.as_deref()))
    }

    /// Put `snapshot` back: what it holds replaces the clipboard; an empty one empties it. The
    /// default writes the snapshot's text; the platform ports write every format, marked so
    /// that clipboard histories do not list it a second time.
    fn restore(&self, snapshot: &ClipSnapshot) -> Result<(), InjectError> {
        match snapshot.text() {
            Some(text) => self.write_text(text),
            None => self.clear(),
        }
    }

    /// A number that changes whenever anything writes the clipboard (Windows'
    /// `GetClipboardSequenceNumber`, macOS' `changeCount`): it tells "still ours" from "the user
    /// copied the same text again". `None` where the platform has none; the text is compared then.
    fn change_count(&self) -> Option<u64> {
        None
    }

    /// Write `text` for a paste that is put back afterwards, marked for clipboard histories and
    /// managers to skip (Windows clipboard history and cloud clipboard, Ditto, macOS managers,
    /// Klipper). `Ok(true)` when a marker went with it, so text that stays after all is written again
    /// plainly; the default writes plainly and answers `false`.
    fn write_transient(&self, text: &str) -> Result<bool, InjectError> {
        self.write_text(text).map(|()| false)
    }
}

/// What a [`KeystrokePort`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delivered {
    /// Tool name for the log (`enigo`, `wtype`, …).
    pub tool: String,
    /// Chord pressed or text typed.
    pub delivery: Delivery,
}

/// Why a [`KeystrokePort`] did not deliver. The two cases end differently: nothing was attempted
/// → the text stays on the clipboard for the user ([`crate::Via::Clipboard`]); an attempt failed
/// → the previous clipboard comes back and the injection is an error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeliveryError {
    /// No tool / connection / permission to try with (nothing reached the application).
    Unavailable(InjectNote),
    /// A tool ran and reported failure, or the connection broke mid-way.
    Failed(String),
}

impl std::fmt::Display for DeliveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(note) => f.write_str(&note.detail),
            Self::Failed(r) => f.write_str(r),
        }
    }
}

/// Synthetic keyboard input as the injectors see it: [`crate::SystemKeys`] (enigo) on every
/// platform, [`crate::linux::ToolchainKeys`] on Linux.
pub trait KeystrokePort: Send + Sync {
    /// Put `text` (already on the clipboard) into the focused application: press `chord`, or type
    /// `text` with a tool that cannot press chords.
    fn deliver(&self, chord: Chord, text: &str) -> Result<Delivered, DeliveryError>;

    /// Press `chord` on its own — no text involved, so tools that can only type are skipped — after
    /// letting go of `held`, the hotkey modifiers the user may still be holding (the selection
    /// copy, docs/dictation.md §19). The default refuses: a port without it cannot copy.
    fn press_chord(&self, chord: Chord, held: &[crate::Modifier]) -> Result<Delivered, DeliveryError> {
        let _ = (chord, held);
        Err(DeliveryError::Unavailable(InjectNote::new(FallbackCode::Other, "this keyboard port cannot press a chord on its own")))
    }
}

/// Clipboard write, paste chord, delayed restore, in sessions (docs/dictation.md §14.3). See the
/// crate docs for the platform caveats.
pub struct ClipboardPasteInjector {
    clipboard: Arc<dyn ClipboardPort>,
    keys: Arc<dyn KeystrokePort>,
    chord: Chord,
    options: PasteOptions,
    /// The last session's restore while it waits for its delay; the next session takes it over.
    pending: Arc<Mutex<Option<Pending>>>,
    generations: Arc<AtomicU64>,
}

impl ClipboardPasteInjector {
    /// [`crate::system_paste_injector`] with this platform's [`PasteOptions::for_os`] and
    /// `restore_delay` before the old clipboard content comes back.
    pub fn new(restore_delay: Duration) -> Self {
        crate::system_paste_injector(PasteOptions::for_os(std::env::consts::OS).with_restore_delay(restore_delay))
    }

    /// Explicit ports and options; the chord follows `options.method` for this platform.
    pub fn with_options(clipboard: Arc<dyn ClipboardPort>, keys: Arc<dyn KeystrokePort>, options: PasteOptions) -> Self {
        Self::with_ports(clipboard, keys, options.method.chord_for(std::env::consts::OS), options)
    }

    /// Explicit ports, chord and options, for tests and unusual hosts.
    pub fn with_ports(clipboard: Arc<dyn ClipboardPort>, keys: Arc<dyn KeystrokePort>, chord: Chord, options: PasteOptions) -> Self {
        Self { clipboard, keys, chord, options, pending: Arc::default(), generations: Arc::default() }
    }

    /// The chord this injector sends.
    pub fn chord(&self) -> Chord {
        self.chord
    }

    /// The timings and shape in force.
    pub fn options(&self) -> PasteOptions {
        self.options
    }

    fn session(&self, after: AfterPaste) -> PasteSession {
        PasteSession {
            clipboard: Arc::clone(&self.clipboard),
            keys: Arc::clone(&self.keys),
            chord: self.chord,
            options: self.options,
            pending: Arc::clone(&self.pending),
            generations: Arc::clone(&self.generations),
            after,
            snapshot: None,
            ours: None,
            last_paste: None,
            left_for_user: false,
            ended: false,
        }
    }
}

/// What a session wrote last: the text, and the clipboard's change count right after where it has
/// one ([`ClipboardPort::change_count`]).
#[derive(Clone)]
struct Ours {
    text: String,
    seq: Option<u64>,
}

/// A finished session's restore, waiting for the target to read the last paste.
struct Pending {
    snapshot: ClipSnapshot,
    ours: Ours,
    generation: u64,
}

/// The clipboard still holds `ours`: the same change count where the clipboard counts its
/// changes, the same text otherwise.
fn still_ours(clipboard: &dyn ClipboardPort, ours: &Ours) -> bool {
    match (ours.seq, clipboard.change_count()) {
        (Some(then), Some(now)) => then == now,
        _ => matches!(clipboard.read_text(), Ok(Some(text)) if text == ours.text),
    }
}

/// Put `snapshot` back if the clipboard still holds `ours`; log, never fail.
fn restore_if_ours(clipboard: &dyn ClipboardPort, snapshot: &ClipSnapshot, ours: &Ours) {
    if !still_ours(clipboard, ours) {
        tracing::debug!("clipboard changed since the paste; leaving it alone");
        return;
    }
    match clipboard.restore(snapshot) {
        Ok(()) => tracing::debug!(formats = ?snapshot.format_names(), bytes = snapshot.bytes(), "clipboard put back"),
        Err(err) => tracing::warn!(%err, "could not put the previous clipboard content back"),
    }
}

/// One [`ClipboardPasteInjector`] session (docs/dictation.md §14.3). The clipboard is saved before
/// the first piece — or taken over from an earlier session whose restore is still waiting — and
/// saved again when the user copies something between two pieces; it comes back once, after the
/// last paste, while it still holds the session's text.
struct PasteSession {
    clipboard: Arc<dyn ClipboardPort>,
    keys: Arc<dyn KeystrokePort>,
    chord: Chord,
    options: PasteOptions,
    pending: Arc<Mutex<Option<Pending>>>,
    generations: Arc<AtomicU64>,
    after: AfterPaste,
    /// What comes back; `None` before the first piece. An empty snapshot puts nothing back.
    snapshot: Option<ClipSnapshot>,
    /// What the session wrote last, while it is on the clipboard.
    ours: Option<Ours>,
    /// When the last piece was pasted: the restore waits for the target to have read it.
    last_paste: Option<Instant>,
    /// A paste could not be attempted: the text stays for the user to paste, nothing comes back.
    left_for_user: bool,
    ended: bool,
}

impl PasteSession {
    fn read_snapshot(&self) -> ClipSnapshot {
        match self.clipboard.snapshot() {
            Ok(snapshot) => {
                if !snapshot.complete {
                    tracing::info!(formats = ?snapshot.format_names(), bytes = snapshot.bytes(), "not every clipboard format could be kept; the others will not come back");
                }
                snapshot
            }
            Err(err) => {
                tracing::debug!(%err, "could not read the clipboard before injecting; nothing will be put back");
                ClipSnapshot::empty()
            }
        }
    }

    /// Before a piece: the first takes over a waiting restore whose text is still on the clipboard
    /// (its snapshot is the user's clipboard), or saves the clipboard; a later one saves it again
    /// when it no longer holds the session's text (the user copied something meanwhile).
    fn save(&mut self) {
        if self.snapshot.is_none() {
            let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(waiting) = pending.take()
                && still_ours(self.clipboard.as_ref(), &waiting.ours)
            {
                tracing::debug!("a restore was still waiting; this session puts that clipboard back instead");
                self.snapshot = Some(waiting.snapshot);
                return;
            }
            drop(pending);
            self.snapshot = Some(self.read_snapshot());
            return;
        }
        if self.ours.as_ref().is_none_or(|ours| !still_ours(self.clipboard.as_ref(), ours)) {
            tracing::debug!("the clipboard changed between two pieces; that is what comes back now");
            self.snapshot = Some(self.read_snapshot());
            self.left_for_user = false;
        }
    }

    /// The snapshot, when putting it back would change something after a paste of `text`.
    fn to_put_back(&self, text: &str) -> Option<&ClipSnapshot> {
        self.snapshot.as_ref().filter(|s| puts_back(s, text))
    }

    /// Put the clipboard back now (the piece was called off or failed). Whether there was
    /// anything to put back.
    fn put_back_now(&mut self) -> bool {
        let Some(ours) = self.ours.take() else { return false };
        let Some(snapshot) = self.to_put_back(&ours.text) else { return false };
        restore_if_ours(self.clipboard.as_ref(), snapshot, &ours);
        true
    }

    /// The end of the session: put the clipboard back once the last paste has had its delay.
    fn end(&mut self, aborted: bool) {
        if std::mem::replace(&mut self.ended, true) {
            return;
        }
        let Some(ours) = self.ours.take() else { return };
        if self.after == AfterPaste::Keep || (self.left_for_user && !aborted) {
            return;
        }
        let Some(snapshot) = self.to_put_back(&ours.text).cloned() else { return };
        let wait = self.last_paste.map_or(Duration::ZERO, |at| self.options.paste_delay_after.saturating_sub(at.elapsed()));
        if wait.is_zero() {
            restore_if_ours(self.clipboard.as_ref(), &snapshot, &ours);
            return;
        }
        let generation = self.generations.fetch_add(1, Ordering::SeqCst) + 1;
        *self.pending.lock().unwrap_or_else(PoisonError::into_inner) = Some(Pending { snapshot, ours, generation });
        let (clipboard, pending) = (Arc::clone(&self.clipboard), Arc::clone(&self.pending));
        let spawned = std::thread::Builder::new().name("voltip-clipboard-restore".into()).spawn(move || {
            // Deliberate fixed delay: the target application must have consumed the paste before
            // the clipboard changes under it. There is no event to wait for.
            std::thread::sleep(wait);
            // Checked and put back under the lock: a session starting now either takes the
            // restore over first or finds the clipboard already back.
            let mut slot = pending.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(waiting) = slot.take_if(|w| w.generation == generation) {
                restore_if_ours(clipboard.as_ref(), &waiting.snapshot, &waiting.ours);
            }
        });
        if let Err(err) = spawned {
            tracing::warn!(%err, "could not start the clipboard restore thread; the next paste puts the clipboard back");
        }
    }
}

impl InjectSession for PasteSession {
    fn piece(&mut self, text: &str, may_deliver: &dyn Fn() -> bool) -> Result<PieceOutcome, InjectError> {
        if self.ended {
            return Ok(PieceOutcome::Cancelled);
        }
        if text.trim().is_empty() {
            return Err(InjectError::EmptyText);
        }
        let text = self.options.prepare(text);
        let chars = text.chars().count();
        self.save();
        // Marked for clipboard histories only when something comes back over it.
        let transient = self.after == AfterPaste::Restore && self.to_put_back(&text).is_some();
        let marked = if transient {
            self.clipboard.write_transient(&text)?
        } else {
            self.clipboard.write_text(&text)?;
            false
        };
        self.ours = Some(Ours { text: text.clone(), seq: self.clipboard.change_count() });
        std::thread::sleep(self.options.paste_delay);
        if !may_deliver() {
            self.put_back_now();
            tracing::debug!(chars, "piece called off before the paste");
            return Ok(PieceOutcome::Cancelled);
        }
        match self.keys.deliver(self.chord, &text) {
            Ok(delivered) => {
                self.last_paste = Some(Instant::now());
                tracing::debug!(chars, chord = %self.chord, tool = %delivered.tool, delivery = ?delivered.delivery, "pasted");
                Ok(PieceOutcome::Delivered(Injection::pasted(chars)))
            }
            Err(DeliveryError::Unavailable(note)) => {
                // Nothing reached the application: the text stays on the clipboard for the user,
                // as an ordinary copy.
                tracing::warn!(reason = %note.detail, code = ?note.code, chord = %self.chord, "no way to paste; text left on the clipboard");
                if marked && self.ours.as_ref().is_some_and(|ours| still_ours(self.clipboard.as_ref(), ours)) {
                    match self.clipboard.write_text(&text) {
                        Ok(()) => self.ours = Some(Ours { text: text.clone(), seq: self.clipboard.change_count() }),
                        Err(err) => tracing::warn!(%err, "could not write the text again without the history marker"),
                    }
                }
                self.left_for_user = true;
                Ok(PieceOutcome::Delivered(Injection::clipboard(chars, Some(note))))
            }
            Err(DeliveryError::Failed(reason)) => {
                // Something was attempted and failed: the clipboard comes back right away.
                let restored = self.put_back_now();
                tracing::warn!(%reason, chord = %self.chord, restored, "paste failed");
                Err(InjectError::Keystroke(if restored { reason } else { format!("{reason} (text left on the clipboard)") }))
            }
        }
    }

    fn finish(&mut self) {
        self.end(false);
    }

    fn abort(&mut self) {
        self.end(true);
    }
}

/// A session dropped without its end ends as finished.
impl Drop for PasteSession {
    fn drop(&mut self) {
        self.end(false);
    }
}

impl Injector for ClipboardPasteInjector {
    /// One piece in a session of its own, the clipboard put back afterwards.
    fn inject(&self, text: &str) -> Result<Injection, InjectError> {
        let mut session = self.session(AfterPaste::Restore);
        let outcome = session.piece(text, &|| true);
        session.finish();
        match outcome? {
            PieceOutcome::Delivered(injection) => Ok(injection),
            PieceOutcome::Cancelled => Err(InjectError::Keystroke("the paste was called off".into())),
        }
    }

    fn describe(&self) -> &'static str {
        "clipboard+paste"
    }

    fn begin_session(self: Arc<Self>, after: AfterPaste) -> Box<dyn InjectSession> {
        Box::new(self.session(after))
    }
}

/// Puts the text on the clipboard and stops there.
pub struct ClipboardOnlyInjector {
    clipboard: Arc<dyn ClipboardPort>,
}

impl ClipboardOnlyInjector {
    /// The system clipboard.
    pub fn new() -> Self {
        Self::with_clipboard(Arc::new(crate::SystemClipboard::new()))
    }

    /// An explicit clipboard port.
    pub fn with_clipboard(clipboard: Arc<dyn ClipboardPort>) -> Self {
        Self { clipboard }
    }
}

impl Default for ClipboardOnlyInjector {
    fn default() -> Self {
        Self::new()
    }
}

impl Injector for ClipboardOnlyInjector {
    fn inject(&self, text: &str) -> Result<Injection, InjectError> {
        if text.trim().is_empty() {
            return Err(InjectError::EmptyText);
        }
        self.clipboard.write_text(text)?;
        let chars = text.chars().count();
        tracing::debug!(chars, "copied to clipboard");
        Ok(Injection::clipboard(chars, None))
    }

    fn describe(&self) -> &'static str {
        "clipboard-only"
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::Instant;

    use super::*;
    use crate::clip::{ClipFormat, TEXT_FORMAT};
    use crate::{AfterPaste, PieceOutcome, Via};

    /// In-memory clipboard with optional failure injection and a write log. Besides its text it
    /// holds other formats (an image, a file list, HTML: `formats`), counts its changes when asked
    /// to ([`MemoryClipboard::counted`], like Windows and macOS), and remembers whether the current
    /// text came with a history-exclusion marker ([`MemoryClipboard::marking`] makes
    /// `write_transient` add one).
    #[derive(Default)]
    pub struct MemoryClipboard {
        pub text: Mutex<Option<String>>,
        /// The formats besides the text.
        pub formats: Mutex<Vec<ClipFormat>>,
        pub fail_read: Mutex<Option<InjectError>>,
        pub fail_write: Mutex<Option<InjectError>>,
        pub writes: Mutex<Vec<String>>,
        /// Changes so far; offered as `change_count` when `counts`.
        pub seq: AtomicU64,
        pub counts: bool,
        /// `write_transient` adds a marker.
        pub marks: bool,
        /// The current text carries a marker.
        pub marked: AtomicBool,
    }

    impl MemoryClipboard {
        pub fn holding(text: &str) -> Arc<Self> {
            let cb = Self::default();
            *cb.text.lock().unwrap() = Some(text.to_string());
            Arc::new(cb)
        }

        /// Holding `formats` (and `text` when given) — what an application copied.
        pub fn holding_formats(text: Option<&str>, formats: Vec<ClipFormat>) -> Self {
            let cb = Self::default();
            *cb.text.lock().unwrap() = text.map(str::to_owned);
            *cb.formats.lock().unwrap() = formats;
            cb
        }

        /// Counting its changes.
        pub fn counted(self) -> Self {
            Self { counts: true, ..self }
        }

        /// Marking transient writes.
        pub fn marking(self) -> Self {
            Self { marks: true, ..self }
        }

        pub fn current(&self) -> Option<String> {
            self.text.lock().unwrap().clone()
        }

        pub fn writes(&self) -> Vec<String> {
            self.writes.lock().unwrap().clone()
        }

        /// What it holds, as a snapshot sees it.
        pub fn content(&self) -> ClipSnapshot {
            let text = self.current();
            let mut formats = self.formats.lock().unwrap().clone();
            if let Some(text) = text {
                formats.push(ClipFormat::new(TEXT_FORMAT, text.into_bytes()));
            }
            ClipSnapshot::single(formats)
        }

        pub fn is_marked(&self) -> bool {
            self.marked.load(Ordering::SeqCst)
        }

        /// Another application copies `text`: a change, not one of ours (not in `writes`).
        pub fn user_copies(&self, text: &str) {
            *self.text.lock().unwrap() = Some(text.to_owned());
            self.formats.lock().unwrap().clear();
            self.marked.store(false, Ordering::SeqCst);
            self.seq.fetch_add(1, Ordering::SeqCst);
        }

        fn replace(&self, text: Option<&str>, formats: Vec<ClipFormat>, marked: bool, log: String) -> Result<(), InjectError> {
            if let Some(err) = self.fail_write.lock().unwrap().clone() {
                return Err(err);
            }
            *self.text.lock().unwrap() = text.map(str::to_owned);
            *self.formats.lock().unwrap() = formats;
            self.marked.store(marked, Ordering::SeqCst);
            self.seq.fetch_add(1, Ordering::SeqCst);
            self.writes.lock().unwrap().push(log);
            Ok(())
        }
    }

    impl ClipboardPort for MemoryClipboard {
        fn read_text(&self) -> Result<Option<String>, InjectError> {
            if let Some(err) = self.fail_read.lock().unwrap().clone() {
                return Err(err);
            }
            Ok(self.current())
        }

        fn write_text(&self, text: &str) -> Result<(), InjectError> {
            self.replace(Some(text), Vec::new(), false, text.to_string())
        }

        /// Empties for real (like arboard) instead of writing `""`; recorded as `<clear>`.
        fn clear(&self) -> Result<(), InjectError> {
            self.replace(None, Vec::new(), false, CLEARED.to_string())
        }

        fn snapshot(&self) -> Result<ClipSnapshot, InjectError> {
            if let Some(err) = self.fail_read.lock().unwrap().clone() {
                return Err(err);
            }
            Ok(self.content())
        }

        /// Recorded as the snapshot's text, `<clear>` for an empty one, `<restore …>` with the
        /// format names for one without text.
        fn restore(&self, snapshot: &ClipSnapshot) -> Result<(), InjectError> {
            let text = snapshot.text();
            let others: Vec<ClipFormat> = snapshot.items.iter().flatten().filter(|f| f.name != TEXT_FORMAT).cloned().collect();
            let log = match text {
                _ if snapshot.is_empty() => CLEARED.to_string(),
                Some(text) => text.to_string(),
                None => format!("<restore {}>", snapshot.format_names().join(",")),
            };
            self.replace(text, others, false, log)
        }

        fn change_count(&self) -> Option<u64> {
            self.counts.then(|| self.seq.load(Ordering::SeqCst))
        }

        fn write_transient(&self, text: &str) -> Result<bool, InjectError> {
            self.replace(Some(text), Vec::new(), self.marks, text.to_string()).map(|()| self.marks)
        }
    }

    /// What [`MemoryClipboard::writes`] records for a `clear`.
    pub const CLEARED: &str = "<clear>";

    /// Records every delivery; `outcome` is what it answers.
    pub struct RecordingKeys {
        pub pressed: Mutex<Vec<(Chord, String)>>,
        pub outcome: Result<Delivered, DeliveryError>,
    }

    impl Default for RecordingKeys {
        fn default() -> Self {
            Self { pressed: Mutex::new(Vec::new()), outcome: Ok(Delivered { tool: "fake".into(), delivery: Delivery::Chord }) }
        }
    }

    impl RecordingKeys {
        pub fn failing(outcome: DeliveryError) -> Arc<Self> {
            Arc::new(Self { pressed: Mutex::new(Vec::new()), outcome: Err(outcome) })
        }
    }

    impl KeystrokePort for RecordingKeys {
        fn deliver(&self, chord: Chord, text: &str) -> Result<Delivered, DeliveryError> {
            self.pressed.lock().unwrap().push((chord, text.to_string()));
            self.outcome.clone()
        }
    }

    const FAST: Duration = Duration::from_millis(1);
    const RESTORE: Duration = Duration::from_millis(30);

    fn fast() -> PasteOptions {
        PasteOptions { paste_delay: FAST, paste_delay_after: RESTORE, ..PasteOptions::default() }
    }

    fn wait_until(mut done: impl FnMut() -> bool) {
        let start = Instant::now();
        while !done() {
            assert!(start.elapsed() < Duration::from_secs(5), "condition not met in time");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn make_injector(clipboard: Arc<MemoryClipboard>, keys: Arc<RecordingKeys>) -> ClipboardPasteInjector {
        ClipboardPasteInjector::with_ports(clipboard, keys, paste_chord_for("linux"), fast())
    }

    fn make_injector_with(clipboard: Arc<MemoryClipboard>) -> ClipboardPasteInjector {
        make_injector(clipboard, Arc::new(RecordingKeys::default()))
    }

    #[test]
    fn chords_per_platform() {
        assert_eq!(paste_chord_for("macos").to_string(), "Cmd+V");
        assert_eq!(paste_chord_for("windows").to_string(), "Ctrl+V");
        assert_eq!(paste_chord_for("linux").to_string(), "Ctrl+V");
        assert_eq!(paste_chord_for("freebsd").to_string(), "Ctrl+V");
        assert_eq!(paste_chord(), paste_chord_for(std::env::consts::OS));
        assert!(paste_chord_for("macos").meta && !paste_chord_for("macos").control);
        assert!(paste_chord_for("linux").control && !paste_chord_for("linux").meta);
    }

    #[test]
    fn options_defaults_and_prepare() {
        let o = PasteOptions::default();
        assert_eq!((o.method, o.paste_delay, o.paste_delay_after, o.append_trailing_space), (PasteMethod::CtrlV, PASTE_DELAY, PASTE_DELAY_AFTER, false));
        assert_eq!(PASTE_DELAY, Duration::from_millis(60));
        assert_eq!(PASTE_DELAY_AFTER, Duration::from_millis(120));
        assert_eq!(o.with_restore_delay(DEFAULT_RESTORE_DELAY).paste_delay_after, Duration::from_millis(600));
        assert_eq!(PasteOptions::for_os("linux"), o);
        for os in ["windows", "macos"] {
            let legacy = PasteOptions::for_os(os);
            assert_eq!(
                (legacy.paste_delay, legacy.paste_delay_after),
                (Duration::from_millis(40), Duration::from_millis(600)),
                "{os} keeps its shipped timings"
            );
            assert_eq!((legacy.method, legacy.append_trailing_space), (PasteMethod::CtrlV, false));
        }
        assert_eq!(o.prepare("你好"), "你好");
        let spaced = PasteOptions { append_trailing_space: true, ..o };
        assert_eq!(spaced.prepare("你好"), "你好 ");
        assert_eq!(spaced.prepare("done. "), "done. ", "no second space");
        assert_eq!(spaced.prepare("line\n"), "line\n", "a trailing newline counts as whitespace");
        let injector = ClipboardPasteInjector::with_options(
            Arc::new(MemoryClipboard::default()),
            Arc::new(RecordingKeys::default()),
            PasteOptions { method: PasteMethod::ShiftInsert, ..fast() },
        );
        // `with_options` takes the chord for the OS the test runs on: Shift+Insert on Linux and
        // Windows, Cmd+V on macOS, which has no Insert key (regression: macos.yml, 2026-09-27).
        let expected = if cfg!(target_os = "macos") { "Cmd+V" } else { "Shift+Insert" };
        assert_eq!(injector.chord().to_string(), expected);
        assert_eq!(injector.chord(), PasteMethod::ShiftInsert.chord_for(std::env::consts::OS));
        assert_eq!(injector.options().method, PasteMethod::ShiftInsert);
    }

    #[test]
    fn restore_decision_table() {
        let of = |t: &str| ClipSnapshot::of_text(Some(t));
        assert!(puts_back(&of("old"), "ours"));
        assert!(!puts_back(&ClipSnapshot::empty(), "ours"), "nothing to restore");
        assert!(!puts_back(&of("ours"), "ours"), "no-op");
        assert!(puts_back(&of(""), "ours"), "empty string was a real previous text");
        assert!(puts_back(&ClipSnapshot::single(image()), "ours"), "an image is something");
        // Ours while the clipboard holds our text — or, where it counts its changes, while nothing
        // wrote it since, whatever the text.
        let ours = |text: &str, seq| Ours { text: text.into(), seq };
        let plain = MemoryClipboard::holding("ours");
        assert!(still_ours(plain.as_ref(), &ours("ours", None)));
        plain.user_copies("user copied this");
        assert!(!still_ours(plain.as_ref(), &ours("ours", None)), "user changed it");
        *plain.text.lock().unwrap() = None;
        assert!(!still_ours(plain.as_ref(), &ours("ours", None)), "clipboard empty");
        *plain.fail_read.lock().unwrap() = Some(InjectError::Clipboard("busy".into()));
        assert!(!still_ours(plain.as_ref(), &ours("ours", None)), "clipboard unreadable");
        let counted = MemoryClipboard::holding_formats(Some("ours"), Vec::new()).counted();
        let seq = counted.change_count();
        assert!(still_ours(&counted, &ours("ours", seq)));
        counted.user_copies("ours");
        assert!(!still_ours(&counted, &ours("ours", seq)), "the same text copied again is the user's");
    }

    #[test]
    fn display_check_table() {
        let env = |vars: &[(&str, &str)]| {
            let vars: Vec<(String, String)> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            move |name: &str| vars.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
        };
        assert_eq!(check_display(env(&[]), "windows"), Ok(()));
        assert_eq!(check_display(env(&[]), "macos"), Ok(()));
        assert!(matches!(check_display(env(&[]), "linux"), Err(InjectError::NoDisplay(_))));
        assert!(matches!(check_display(env(&[("DISPLAY", "  ")]), "linux"), Err(InjectError::NoDisplay(_))));
        assert_eq!(check_display(env(&[("DISPLAY", ":0")]), "linux"), Ok(()));
        assert_eq!(check_display(env(&[("WAYLAND_DISPLAY", "wayland-0")]), "linux"), Ok(()));
        assert_eq!(check_display(env(&[("DISPLAY", ""), ("WAYLAND_DISPLAY", "wayland-1")]), "linux"), Ok(()));
    }

    #[test]
    fn paste_saves_writes_presses_and_restores() {
        let clipboard = MemoryClipboard::holding("previous text");
        let keys = Arc::new(RecordingKeys::default());
        let injector = make_injector(Arc::clone(&clipboard), Arc::clone(&keys));
        assert_eq!(injector.describe(), "clipboard+paste");
        assert_eq!(injector.chord().to_string(), "Ctrl+V");

        let injection = injector.inject("你好，世界").unwrap();
        assert_eq!(injection, Injection { via: Via::Paste, chars: 5, note: None });
        assert_eq!(keys.pressed.lock().unwrap().as_slice(), &[(paste_chord_for("linux"), "你好，世界".to_string())]);
        assert_eq!(clipboard.current().as_deref(), Some("你好，世界"), "ours is on the clipboard right after the paste");

        wait_until(|| clipboard.current().as_deref() == Some("previous text"));
        assert_eq!(clipboard.writes(), vec!["你好，世界".to_string(), "previous text".to_string()]);
    }

    #[test]
    fn trailing_space_is_pasted_and_counted() {
        let clipboard = MemoryClipboard::holding("prev");
        let keys = Arc::new(RecordingKeys::default());
        let injector = ClipboardPasteInjector::with_ports(
            clipboard.clone() as Arc<dyn ClipboardPort>,
            keys.clone() as Arc<dyn KeystrokePort>,
            paste_chord_for("linux"),
            PasteOptions { append_trailing_space: true, ..fast() },
        );
        let injection = injector.inject("word").unwrap();
        assert_eq!(injection.chars, 5);
        assert_eq!(keys.pressed.lock().unwrap()[0].1, "word ");
        wait_until(|| clipboard.current().as_deref() == Some("prev"));
    }

    #[test]
    fn restore_skipped_when_user_copied_something_else() {
        let clipboard = MemoryClipboard::holding("previous");
        let injector = make_injector_with(Arc::clone(&clipboard));
        injector.inject("ours").unwrap();
        // The user copies something before the restore fires.
        *clipboard.text.lock().unwrap() = Some("user copy".into());
        std::thread::sleep(RESTORE * 4);
        assert_eq!(clipboard.current().as_deref(), Some("user copy"));
        assert_eq!(clipboard.writes(), vec!["ours".to_string()]);
    }

    #[test]
    fn no_previous_text_means_no_restore() {
        let clipboard = Arc::new(MemoryClipboard::default());
        let injector = make_injector_with(Arc::clone(&clipboard));
        assert_eq!(injector.inject("fresh").unwrap().via, Via::Paste);
        std::thread::sleep(RESTORE * 4);
        assert_eq!(clipboard.current().as_deref(), Some("fresh"));
        assert_eq!(clipboard.writes().len(), 1);

        // Same text already on the clipboard: nothing to restore either.
        let clipboard = MemoryClipboard::holding("same");
        let injector = make_injector_with(Arc::clone(&clipboard));
        injector.inject("same").unwrap();
        std::thread::sleep(RESTORE * 4);
        assert_eq!(clipboard.writes(), vec!["same".to_string()]);
    }

    #[test]
    fn unreadable_clipboard_still_pastes_without_restore() {
        let clipboard = Arc::new(MemoryClipboard::default());
        *clipboard.fail_read.lock().unwrap() = Some(InjectError::Clipboard("occupied".into()));
        let injector = make_injector_with(Arc::clone(&clipboard));
        let injection = injector.inject("text").unwrap();
        assert_eq!(injection.via, Via::Paste);
        assert_eq!(clipboard.current().as_deref(), Some("text"));
    }

    /// Nothing could be tried (no tool, no connection): the text stays on the clipboard for the
    /// user, the previous text is not restored, and the note says why.
    #[test]
    fn unavailable_keystroke_falls_back_to_clipboard_with_note() {
        let clipboard = MemoryClipboard::holding("previous");
        let keys = RecordingKeys::failing(DeliveryError::Unavailable(InjectNote::new(FallbackCode::NoDisplay, "no connection could be established")));
        let injector = make_injector(Arc::clone(&clipboard), keys);
        let injection = injector.inject("fallback").unwrap();
        assert_eq!(injection.via, Via::Clipboard);
        assert_eq!(injection.chars, 8);
        assert_eq!(injection.note, Some(InjectNote::new(FallbackCode::NoDisplay, "no connection could be established")), "the code rides along");
        std::thread::sleep(RESTORE * 4);
        assert_eq!(clipboard.current().as_deref(), Some("fallback"), "text stays for the user to paste");
        assert_eq!(clipboard.writes(), vec!["fallback".to_string()]);
    }

    /// Failure-restore order (docs/dictation.md §14): save → write → attempt → fail → restore now,
    /// and the result is an error, not a silent clipboard fallback.
    #[test]
    fn failed_keystroke_restores_previous_clipboard_and_errors() {
        let clipboard = MemoryClipboard::holding("previous");
        let keys = RecordingKeys::failing(DeliveryError::Failed("wtype exited with exit status: 1".into()));
        let injector = make_injector(Arc::clone(&clipboard), Arc::clone(&keys));
        let err = injector.inject("ours").unwrap_err();
        assert_eq!(err, InjectError::Keystroke("wtype exited with exit status: 1".into()));
        assert_eq!(keys.pressed.lock().unwrap().len(), 1, "the tool was tried once");
        assert_eq!(clipboard.writes(), vec!["ours".to_string(), "previous".to_string()], "write, then restore — no delay");
        assert_eq!(clipboard.current().as_deref(), Some("previous"));

        // The user copied something between our write and the failure: leave their copy alone.
        let clipboard = MemoryClipboard::holding("previous");
        struct CopyingKeys(Arc<MemoryClipboard>);
        impl KeystrokePort for CopyingKeys {
            fn deliver(&self, _: Chord, _: &str) -> Result<Delivered, DeliveryError> {
                *self.0.text.lock().unwrap() = Some("user copy".into());
                Err(DeliveryError::Failed("late".into()))
            }
        }
        let injector = ClipboardPasteInjector::with_ports(
            clipboard.clone() as Arc<dyn ClipboardPort>,
            Arc::new(CopyingKeys(Arc::clone(&clipboard))),
            paste_chord_for("linux"),
            fast(),
        );
        assert!(matches!(injector.inject("ours"), Err(InjectError::Keystroke(_))));
        assert_eq!(clipboard.current().as_deref(), Some("user copy"));

        // No previous text: nothing to restore, the message says the text is still there.
        let clipboard = Arc::new(MemoryClipboard::default());
        let keys = RecordingKeys::failing(DeliveryError::Failed("ydotool: connect: No such file".into()));
        let injector = make_injector(Arc::clone(&clipboard), keys);
        let err = injector.inject("ours").unwrap_err();
        assert_eq!(err.to_string(), "keystroke: ydotool: connect: No such file (text left on the clipboard)");
        assert_eq!(clipboard.current().as_deref(), Some("ours"));

        // A restore that itself fails is logged, not surfaced twice.
        let clipboard = MemoryClipboard::holding("previous");
        struct BreakingKeys(Arc<MemoryClipboard>);
        impl KeystrokePort for BreakingKeys {
            fn deliver(&self, _: Chord, _: &str) -> Result<Delivered, DeliveryError> {
                *self.0.fail_write.lock().unwrap() = Some(InjectError::Clipboard("gone".into()));
                Err(DeliveryError::Failed("broken".into()))
            }
        }
        let injector = ClipboardPasteInjector::with_ports(
            clipboard.clone() as Arc<dyn ClipboardPort>,
            Arc::new(BreakingKeys(Arc::clone(&clipboard))),
            paste_chord_for("linux"),
            fast(),
        );
        assert_eq!(injector.inject("ours").unwrap_err(), InjectError::Keystroke("broken".into()));
        assert_eq!(clipboard.current().as_deref(), Some("ours"));
        assert_eq!(DeliveryError::Failed("x".into()).to_string(), "x");
        assert_eq!(DeliveryError::Unavailable(InjectNote::new(FallbackCode::Other, "y")).to_string(), "y");
    }

    #[test]
    fn clipboard_write_failure_and_empty_text_are_errors() {
        let clipboard = Arc::new(MemoryClipboard::default());
        *clipboard.fail_write.lock().unwrap() = Some(InjectError::NoDisplay("DISPLAY unset".into()));
        let keys = Arc::new(RecordingKeys::default());
        let injector = make_injector(Arc::clone(&clipboard), Arc::clone(&keys));
        assert_eq!(injector.inject("x").unwrap_err(), InjectError::NoDisplay("DISPLAY unset".into()));
        assert!(keys.pressed.lock().unwrap().is_empty(), "no keystroke without a clipboard write");
        *clipboard.fail_write.lock().unwrap() = Some(InjectError::Clipboard("denied".into()));
        assert_eq!(injector.inject("x").unwrap_err(), InjectError::Clipboard("denied".into()));
        assert_eq!(injector.inject("").unwrap_err(), InjectError::EmptyText);
        assert_eq!(injector.inject(" \n\t").unwrap_err(), InjectError::EmptyText);
    }

    #[test]
    fn restore_write_failure_is_logged_not_fatal() {
        let clipboard = MemoryClipboard::holding("previous");
        let injector = make_injector_with(Arc::clone(&clipboard));
        injector.inject("ours").unwrap();
        *clipboard.fail_write.lock().unwrap() = Some(InjectError::Clipboard("gone".into()));
        std::thread::sleep(RESTORE * 4);
        assert_eq!(clipboard.current().as_deref(), Some("ours"));
    }

    #[test]
    fn clipboard_only_never_types() {
        let clipboard = MemoryClipboard::holding("previous");
        let port: Arc<dyn ClipboardPort> = Arc::clone(&clipboard) as Arc<dyn ClipboardPort>;
        let injector = ClipboardOnlyInjector::with_clipboard(port);
        assert_eq!(injector.describe(), "clipboard-only");
        let injection = injector.inject("copy me").unwrap();
        assert_eq!(injection, Injection { via: Via::Clipboard, chars: 7, note: None });
        std::thread::sleep(RESTORE * 2);
        assert_eq!(clipboard.current().as_deref(), Some("copy me"), "nothing is restored");
        assert_eq!(injector.inject("   ").unwrap_err(), InjectError::EmptyText);
        *clipboard.fail_write.lock().unwrap() = Some(InjectError::Clipboard("locked".into()));
        assert_eq!(injector.inject("x").unwrap_err(), InjectError::Clipboard("locked".into()));
    }

    /// A PNG and its alt text, a Finder / Explorer file list, formatted text: what users copy
    /// besides plain text.
    fn image() -> Vec<ClipFormat> {
        vec![ClipFormat::new("image/png", [0x89, b'P', b'N', b'G', 1, 2, 3])]
    }

    fn files() -> Vec<ClipFormat> {
        vec![ClipFormat::new("text/uri-list", "file:///home/u/report.pdf\r\n")]
    }

    fn html() -> Vec<ClipFormat> {
        vec![ClipFormat::new("text/html", "<b>加粗</b>的一段")]
    }

    fn session_injector(clipboard: &Arc<MemoryClipboard>, keys: Arc<RecordingKeys>, restore: Duration) -> Arc<ClipboardPasteInjector> {
        Arc::new(ClipboardPasteInjector::with_ports(
            clipboard.clone(),
            keys,
            paste_chord_for("linux"),
            PasteOptions { paste_delay: FAST, paste_delay_after: restore, ..PasteOptions::default() },
        ))
    }

    /// Regression (R01, 2026-10-28 review): only the text was saved, so an image, a file list or
    /// the HTML of formatted text the user had copied was replaced by the dictation and never came
    /// back. The whole clipboard comes back now.
    #[test]
    fn regression_an_image_file_list_or_html_on_the_clipboard_comes_back_after_the_paste() {
        for (text, formats) in [(None, image()), (None, files()), (Some("加粗的一段"), html())] {
            let clipboard = Arc::new(MemoryClipboard::holding_formats(text, formats.clone()));
            let before = clipboard.content();
            let injector = make_injector_with(Arc::clone(&clipboard));
            assert_eq!(injector.inject("口述的文字").unwrap().via, Via::Paste);
            wait_until(|| clipboard.content() == before);
            assert_eq!(clipboard.writes().len(), 2, "our text, then the restore: {:?}", clipboard.writes());
            assert_eq!(clipboard.formats.lock().unwrap().as_slice(), formats.as_slice());
        }
    }

    /// Regression (R01, `live_inject` and quick takes): a paste that came before the previous one's
    /// restore saved the previous dictation as "the user's clipboard" and put that back. A session
    /// that starts while an earlier one's restore is pending takes the earlier one's snapshot over:
    /// the user's own clipboard comes back, once.
    #[test]
    fn regression_two_pastes_within_the_restore_delay_put_back_the_users_clipboard_once() {
        for counted in [false, true] {
            let clipboard = Arc::new(if counted {
                MemoryClipboard::holding_formats(Some("用户的剪贴板"), Vec::new()).counted()
            } else {
                MemoryClipboard::holding_formats(Some("用户的剪贴板"), Vec::new())
            });
            let injector = session_injector(&clipboard, Arc::new(RecordingKeys::default()), Duration::from_millis(400));
            injector.inject("第一次").unwrap();
            injector.inject("第二次").unwrap();
            wait_until(|| clipboard.current().as_deref() == Some("用户的剪贴板"));
            // Long enough for both restores to have run, had there been two.
            std::thread::sleep(Duration::from_millis(1200));
            assert_eq!(clipboard.current().as_deref(), Some("用户的剪贴板"), "counted={counted}");
            let writes = clipboard.writes();
            let second = writes.iter().position(|w| w == "第二次").unwrap();
            assert!(!writes[second..].iter().any(|w| w == "第一次"), "the first dictation never comes back: {writes:?}");
        }
    }

    /// Regression (R01): with a change count the clipboard tells our text from the same text the
    /// user copied again; the user's copy is not overwritten by the restore.
    #[test]
    fn regression_the_same_text_copied_again_by_the_user_is_kept() {
        let clipboard = Arc::new(MemoryClipboard::holding_formats(Some("原来的"), Vec::new()).counted());
        let injector = session_injector(&clipboard, Arc::new(RecordingKeys::default()), RESTORE);
        injector.inject("一样的字").unwrap();
        clipboard.user_copies("一样的字");
        std::thread::sleep(RESTORE * 4);
        assert_eq!(clipboard.current().as_deref(), Some("一样的字"));
        assert_eq!(clipboard.writes(), vec!["一样的字".to_string()], "nothing was restored over the user's copy");
    }

    /// Regression (R01, `live_inject`): every sentence scheduled its own restore, which put an
    /// earlier sentence back instead of the user's text. A session puts the clipboard back once,
    /// after its last paste.
    #[test]
    fn regression_a_session_restores_once_after_its_last_piece() {
        let clipboard = Arc::new(MemoryClipboard::holding_formats(Some("用户的"), image()));
        let keys = Arc::new(RecordingKeys::default());
        let injector = session_injector(&clipboard, Arc::clone(&keys), RESTORE);
        let before = clipboard.content();
        let mut session = injector.clone().begin_session(AfterPaste::Restore);
        for piece in ["一句。", "二句。", "三句。"] {
            assert!(matches!(session.piece(piece, &|| true).unwrap(), PieceOutcome::Delivered(Injection { via: Via::Paste, .. })));
            std::thread::sleep(RESTORE * 2);
            assert_eq!(clipboard.current().as_deref(), Some(piece), "nothing is put back between pieces");
        }
        session.finish();
        wait_until(|| clipboard.content() == before);
        assert_eq!(clipboard.writes(), ["一句。", "二句。", "三句。", "用户的"].map(String::from));
        assert_eq!(keys.pressed.lock().unwrap().len(), 3);
    }

    /// What the user copies in the middle of a session is what the session puts back at its end.
    #[test]
    fn a_copy_the_user_makes_during_a_session_is_what_comes_back() {
        let clipboard = Arc::new(MemoryClipboard::holding_formats(Some("原来的"), Vec::new()));
        let injector = session_injector(&clipboard, Arc::new(RecordingKeys::default()), RESTORE);
        let mut session = injector.clone().begin_session(AfterPaste::Restore);
        session.piece("一。", &|| true).unwrap();
        clipboard.user_copies("新复制的");
        session.piece("二。", &|| true).unwrap();
        session.finish();
        wait_until(|| clipboard.current().as_deref() == Some("新复制的"));
        std::thread::sleep(RESTORE * 4);
        assert_eq!(clipboard.current().as_deref(), Some("新复制的"));
        assert!(!clipboard.writes().contains(&"原来的".to_string()), "{:?}", clipboard.writes());
    }

    /// Regression (R01): a paste that failed put back the text only; the image the user had
    /// copied was gone. The whole snapshot comes back at once.
    #[test]
    fn regression_a_failed_paste_puts_the_whole_clipboard_back_at_once() {
        let clipboard = Arc::new(MemoryClipboard::holding_formats(Some("图片说明"), image()));
        let before = clipboard.content();
        let keys = RecordingKeys::failing(DeliveryError::Failed("wtype exited with exit status: 1".into()));
        let injector = make_injector(Arc::clone(&clipboard), keys);
        assert!(matches!(injector.inject("口述"), Err(InjectError::Keystroke(m)) if m == "wtype exited with exit status: 1"));
        assert_eq!(clipboard.content(), before, "back before inject returns");
    }

    /// A piece `may_deliver` turns down (the take was cancelled, the window changed) is not
    /// delivered and the clipboard is put back right away; it was asked with our text already on
    /// the clipboard, right before the paste.
    #[test]
    fn a_piece_that_may_not_be_delivered_is_not_and_the_clipboard_comes_back() {
        let clipboard = Arc::new(MemoryClipboard::holding_formats(Some("原来的"), image()));
        let before = clipboard.content();
        let keys = Arc::new(RecordingKeys::default());
        let injector = session_injector(&clipboard, Arc::clone(&keys), RESTORE);
        let mut session = injector.clone().begin_session(AfterPaste::Restore);
        let seen = Mutex::new(None);
        let outcome = session
            .piece("取消的", &|| {
                *seen.lock().unwrap() = clipboard.current();
                false
            })
            .unwrap();
        assert_eq!(outcome, PieceOutcome::Cancelled);
        assert_eq!(seen.lock().unwrap().as_deref(), Some("取消的"), "asked once our text was on the clipboard");
        assert!(keys.pressed.lock().unwrap().is_empty(), "nothing was pressed");
        assert_eq!(clipboard.content(), before, "the clipboard is back at once");
        session.finish();
        std::thread::sleep(RESTORE * 4);
        assert_eq!(clipboard.writes(), ["取消的", "原来的"].map(String::from), "write, restore; nothing at the finish");
    }

    /// An ended session delivers nothing more and leaves the clipboard alone.
    #[test]
    fn an_ended_session_delivers_nothing() {
        let clipboard = MemoryClipboard::holding("原来的");
        let keys = Arc::new(RecordingKeys::default());
        let injector = session_injector(&clipboard, Arc::clone(&keys), RESTORE);
        for end in [true, false] {
            let mut session = injector.clone().begin_session(AfterPaste::Restore);
            if end {
                session.finish()
            } else {
                session.abort()
            }
            assert_eq!(session.piece("迟到的", &|| true).unwrap(), PieceOutcome::Cancelled);
        }
        assert!(keys.pressed.lock().unwrap().is_empty());
        assert!(clipboard.writes().is_empty());
    }

    /// `Keep` (the `clipboard_after` setting): the text is written as an ordinary copy and stays,
    /// and an earlier session's pending restore does not land on it.
    #[test]
    fn keep_leaves_the_text_as_an_ordinary_copy() {
        let clipboard = Arc::new(MemoryClipboard::holding_formats(Some("原来的"), Vec::new()).marking());
        let injector = session_injector(&clipboard, Arc::new(RecordingKeys::default()), Duration::from_millis(300));
        injector.inject("先粘贴的").unwrap();
        let mut session = injector.clone().begin_session(AfterPaste::Keep);
        session.piece("保留的", &|| true).unwrap();
        assert!(!clipboard.is_marked(), "kept text carries no history marker");
        session.finish();
        std::thread::sleep(Duration::from_millis(900));
        assert_eq!(clipboard.current().as_deref(), Some("保留的"));
    }

    /// The text of a paste that is put back afterwards carries the history marker; when the paste
    /// is unavailable and the text stays for the user, it is written again as an ordinary copy and
    /// the session puts nothing back.
    #[test]
    fn a_restored_paste_is_marked_and_text_left_for_the_user_is_not() {
        let clipboard = Arc::new(MemoryClipboard::holding_formats(Some("原来的"), Vec::new()).marking());
        struct Watching(Arc<MemoryClipboard>, Mutex<Vec<bool>>);
        impl KeystrokePort for Watching {
            fn deliver(&self, _: Chord, _: &str) -> Result<Delivered, DeliveryError> {
                self.1.lock().unwrap().push(self.0.is_marked());
                Ok(Delivered { tool: "fake".into(), delivery: Delivery::Chord })
            }
        }
        let keys = Arc::new(Watching(Arc::clone(&clipboard), Mutex::new(Vec::new())));
        let injector = ClipboardPasteInjector::with_ports(clipboard.clone(), keys.clone(), paste_chord_for("linux"), fast());
        injector.inject("口述").unwrap();
        assert_eq!(keys.1.lock().unwrap().as_slice(), &[true], "marked while the target reads it");
        wait_until(|| clipboard.current().as_deref() == Some("原来的"));
        // Unavailable: the text stays, as an ordinary copy.
        let clipboard = Arc::new(MemoryClipboard::holding_formats(Some("原来的"), Vec::new()).marking());
        let keys = RecordingKeys::failing(DeliveryError::Unavailable(InjectNote::new(FallbackCode::NoTool, "no paste tool")));
        let injector = make_injector(Arc::clone(&clipboard), keys);
        assert_eq!(injector.inject("留下的").unwrap().via, Via::Clipboard);
        std::thread::sleep(RESTORE * 4);
        assert_eq!(clipboard.current().as_deref(), Some("留下的"));
        assert!(!clipboard.is_marked(), "written again without the marker");
        assert_eq!(clipboard.writes(), ["留下的", "留下的"].map(String::from));
    }

    /// Nothing on the clipboard before: nothing to put back, so no marker either.
    #[test]
    fn an_empty_clipboard_gets_an_ordinary_copy_that_stays() {
        let clipboard = Arc::new(MemoryClipboard::default().marking());
        let injector = make_injector_with(Arc::clone(&clipboard));
        injector.inject("新的").unwrap();
        std::thread::sleep(RESTORE * 4);
        assert_eq!((clipboard.current().as_deref(), clipboard.is_marked()), (Some("新的"), false));
        assert_eq!(clipboard.writes(), vec!["新的".to_string()]);
    }

    /// `abort` right after a paste still gives the target its restore delay; long after the last
    /// paste it puts the clipboard back at once.
    #[test]
    fn abort_waits_for_the_last_paste_to_be_read() {
        let restore = Duration::from_millis(300);
        let clipboard = MemoryClipboard::holding("原来的");
        let injector = session_injector(&clipboard, Arc::new(RecordingKeys::default()), restore);
        let mut session = injector.clone().begin_session(AfterPaste::Restore);
        session.piece("刚粘贴", &|| true).unwrap();
        session.abort();
        assert_eq!(clipboard.current().as_deref(), Some("刚粘贴"), "the target may not have read it yet");
        wait_until(|| clipboard.current().as_deref() == Some("原来的"));
        let mut session = injector.clone().begin_session(AfterPaste::Restore);
        session.piece("早就粘贴了", &|| true).unwrap();
        std::thread::sleep(restore + Duration::from_millis(100));
        session.abort();
        assert_eq!(clipboard.current().as_deref(), Some("原来的"), "back before abort returns");
    }

    #[test]
    fn injectors_are_send_sync_and_object_safe() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ClipboardPasteInjector>();
        assert_send_sync::<ClipboardOnlyInjector>();
        let boxed: Vec<Box<dyn Injector>> = vec![Box::new(ClipboardOnlyInjector::with_clipboard(Arc::new(MemoryClipboard::default())))];
        assert_eq!(boxed[0].describe(), "clipboard-only");
    }
}
