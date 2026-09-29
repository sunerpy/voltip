//! The injectors and the decisions they make, kept apart from the OS calls in
//! [`crate::platform`] / [`crate::linux`] so everything here runs on a headless host.

use std::sync::Arc;
use std::time::Duration;

use crate::toolchain::{Chord, Delivery, PasteMethod};
use crate::{FallbackCode, InjectError, InjectNote, Injection, Injector};

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

/// Whether the clipboard should be put back to `previous` after a paste: only when there was a
/// previous text, the clipboard still holds `ours` (the user has not copied something new), and
/// restoring would change anything.
pub fn should_restore(previous: Option<&str>, current: Option<&str>, ours: &str) -> bool {
    match (previous, current) {
        (Some(previous), Some(current)) => current == ours && previous != ours,
        _ => false,
    }
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

/// Clipboard write, paste chord, delayed restore. See the crate docs for the platform caveats.
pub struct ClipboardPasteInjector {
    clipboard: Arc<dyn ClipboardPort>,
    keys: Arc<dyn KeystrokePort>,
    chord: Chord,
    options: PasteOptions,
}

impl ClipboardPasteInjector {
    /// [`crate::system_paste_injector`] with this platform's [`PasteOptions::for_os`] and
    /// `restore_delay` before the old clipboard text comes back.
    pub fn new(restore_delay: Duration) -> Self {
        crate::system_paste_injector(PasteOptions::for_os(std::env::consts::OS).with_restore_delay(restore_delay))
    }

    /// Explicit ports and options; the chord follows `options.method` for this platform.
    pub fn with_options(clipboard: Arc<dyn ClipboardPort>, keys: Arc<dyn KeystrokePort>, options: PasteOptions) -> Self {
        Self::with_ports(clipboard, keys, options.method.chord_for(std::env::consts::OS), options)
    }

    /// Explicit ports, chord and options, for tests and unusual hosts.
    pub fn with_ports(clipboard: Arc<dyn ClipboardPort>, keys: Arc<dyn KeystrokePort>, chord: Chord, options: PasteOptions) -> Self {
        Self { clipboard, keys, chord, options }
    }

    /// The chord this injector sends.
    pub fn chord(&self) -> Chord {
        self.chord
    }

    /// The timings and shape in force.
    pub fn options(&self) -> PasteOptions {
        self.options
    }

    fn schedule_restore(&self, previous: String, ours: String) {
        let clipboard = Arc::clone(&self.clipboard);
        let delay = self.options.paste_delay_after;
        let spawned = std::thread::Builder::new().name("voltip-clipboard-restore".into()).spawn(move || {
            // Deliberate fixed delay: the target application must have consumed the paste before
            // the clipboard changes under it. There is no event to wait for.
            std::thread::sleep(delay);
            restore_now(clipboard.as_ref(), &previous, &ours);
        });
        if let Err(err) = spawned {
            tracing::warn!(%err, "could not start the clipboard restore thread; previous text stays replaced");
        }
    }
}

/// Put `previous` back if the clipboard still holds `ours`; log, never fail.
fn restore_now(clipboard: &dyn ClipboardPort, previous: &str, ours: &str) {
    let current = clipboard.read_text().unwrap_or_default();
    if should_restore(Some(previous), current.as_deref(), ours) {
        if let Err(err) = clipboard.write_text(previous) {
            tracing::warn!(%err, "could not restore the previous clipboard text");
        }
    } else {
        tracing::debug!("clipboard changed since the paste; leaving it alone");
    }
}

impl Injector for ClipboardPasteInjector {
    fn inject(&self, text: &str) -> Result<Injection, InjectError> {
        if text.trim().is_empty() {
            return Err(InjectError::EmptyText);
        }
        let text = self.options.prepare(text);
        let chars = text.chars().count();
        let previous = match self.clipboard.read_text() {
            Ok(previous) => previous,
            Err(err) => {
                tracing::debug!(%err, "could not read the clipboard before injecting; nothing will be restored");
                None
            }
        };
        self.clipboard.write_text(&text)?;
        std::thread::sleep(self.options.paste_delay);
        let previous = previous.filter(|p| *p != text);
        match self.keys.deliver(self.chord, &text) {
            Ok(delivered) => {
                if let Some(previous) = previous {
                    self.schedule_restore(previous, text);
                }
                tracing::debug!(chars, chord = %self.chord, tool = %delivered.tool, delivery = ?delivered.delivery, "pasted");
                Ok(Injection::pasted(chars))
            }
            Err(DeliveryError::Unavailable(note)) => {
                // Nothing reached the application: the text stays on the clipboard for the user.
                tracing::warn!(reason = %note.detail, code = ?note.code, chord = %self.chord, "no way to paste; text left on the clipboard");
                Ok(Injection::clipboard(chars, Some(note)))
            }
            Err(DeliveryError::Failed(reason)) => {
                // Something was attempted and failed: undo our clipboard write right away.
                let restored = match &previous {
                    Some(previous) => {
                        restore_now(self.clipboard.as_ref(), previous, &text);
                        true
                    }
                    None => false,
                };
                tracing::warn!(%reason, chord = %self.chord, restored, "paste failed");
                Err(InjectError::Keystroke(if restored { reason } else { format!("{reason} (text left on the clipboard)") }))
            }
        }
    }

    fn describe(&self) -> &'static str {
        "clipboard+paste"
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
    use std::time::Instant;

    use super::*;
    use crate::Via;

    /// In-memory clipboard with optional failure injection and a write log.
    #[derive(Default)]
    pub struct MemoryClipboard {
        pub text: Mutex<Option<String>>,
        pub fail_read: Mutex<Option<InjectError>>,
        pub fail_write: Mutex<Option<InjectError>>,
        pub writes: Mutex<Vec<String>>,
    }

    impl MemoryClipboard {
        pub fn holding(text: &str) -> Arc<Self> {
            let cb = Self::default();
            *cb.text.lock().unwrap() = Some(text.to_string());
            Arc::new(cb)
        }

        pub fn current(&self) -> Option<String> {
            self.text.lock().unwrap().clone()
        }

        pub fn writes(&self) -> Vec<String> {
            self.writes.lock().unwrap().clone()
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
            if let Some(err) = self.fail_write.lock().unwrap().clone() {
                return Err(err);
            }
            *self.text.lock().unwrap() = Some(text.to_string());
            self.writes.lock().unwrap().push(text.to_string());
            Ok(())
        }

        /// Empties for real (like arboard) instead of writing `""`; recorded as `<clear>`.
        fn clear(&self) -> Result<(), InjectError> {
            if let Some(err) = self.fail_write.lock().unwrap().clone() {
                return Err(err);
            }
            *self.text.lock().unwrap() = None;
            self.writes.lock().unwrap().push(CLEARED.to_string());
            Ok(())
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
        assert!(should_restore(Some("old"), Some("ours"), "ours"));
        assert!(!should_restore(Some("old"), Some("user copied this"), "ours"), "user changed it");
        assert!(!should_restore(None, Some("ours"), "ours"), "nothing to restore");
        assert!(!should_restore(Some("old"), None, "ours"), "clipboard unreadable or empty");
        assert!(!should_restore(Some("ours"), Some("ours"), "ours"), "no-op");
        assert!(should_restore(Some(""), Some("ours"), "ours"), "empty string was a real previous text");
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

    #[test]
    fn injectors_are_send_sync_and_object_safe() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ClipboardPasteInjector>();
        assert_send_sync::<ClipboardOnlyInjector>();
        let boxed: Vec<Box<dyn Injector>> = vec![Box::new(ClipboardOnlyInjector::with_clipboard(Arc::new(MemoryClipboard::default())))];
        assert_eq!(boxed[0].describe(), "clipboard-only");
    }
}
