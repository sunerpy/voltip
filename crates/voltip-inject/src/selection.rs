//! Reading the text selected in the foreground application (docs/dictation.md §19): save the
//! clipboard, clear it, press the copy chord, wait for the application to put the selection on the
//! clipboard, read it, put the previous clipboard text back. The decisions (chord per platform,
//! ordering, what counts as "nothing selected", when the restore is skipped) live here over the
//! same two ports as the paste ([`ClipboardPort`], [`KeystrokePort`]), so they run headless.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::InjectError;
use crate::injector::{ClipboardPort, KeystrokePort};
use crate::toolchain::{Chord, Key, Modifier};

/// How long the copy waits for the application to put the selection on the clipboard.
pub const COPY_TIMEOUT: Duration = Duration::from_millis(250);
/// How often the clipboard is read while waiting.
pub const COPY_POLL: Duration = Duration::from_millis(10);

/// Timing of one selection copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CopyOptions {
    /// Give up (nothing selected) after this long without a text on the clipboard.
    pub timeout: Duration,
    /// Read interval while waiting.
    pub poll: Duration,
}

impl Default for CopyOptions {
    fn default() -> Self {
        Self { timeout: COPY_TIMEOUT, poll: COPY_POLL }
    }
}

/// The copy chord for `os` as [`std::env::consts::OS`] names it: `Cmd+C` on macOS, `Ctrl+Insert`
/// elsewhere. Not `Ctrl+C` (docs/dictation.md §19.2): that is a terminal's interrupt, and the
/// application in front is not always known (pure Wayland gives no answer) or is an editor whose
/// terminal panel has the focus. `Ctrl+Insert` is the CUA copy that Win32 / WPF / WinUI edits,
/// Chromium and Electron, Firefox, Office, GTK, Qt and Tk widgets, VS Code, JetBrains and Windows
/// Terminal all take as copy, whatever the keyboard layout; a terminal that does not bind it gets
/// an escape sequence, never a signal.
pub fn copy_chord_for(os: &str) -> Chord {
    if os == "macos" {
        return Chord { control: false, meta: true, shift: false, key: Key::Char('c') };
    }
    Chord { control: true, meta: false, shift: false, key: Key::Insert }
}

/// [`copy_chord_for`] this platform.
pub fn copy_chord() -> Chord {
    copy_chord_for(std::env::consts::OS)
}

/// Something that can read the foreground application's selection.
pub trait SelectionSource: Send + Sync {
    /// The selected text, `None` when nothing is selected (the application put no text on the
    /// clipboard within the timeout). `held`: modifiers the user may still hold from the hotkey,
    /// released before the copy chord. `Err` when the copy could not be attempted or the clipboard
    /// is unusable; the previous clipboard text is put back whenever that is possible.
    fn copy_selection(&self, held: &[Modifier]) -> Result<Option<String>, InjectError>;
}

/// The clipboard-and-copy-chord [`SelectionSource`].
pub struct ClipboardSelection {
    clipboard: Arc<dyn ClipboardPort>,
    keys: Arc<dyn KeystrokePort>,
    chord: Chord,
    options: CopyOptions,
}

impl ClipboardSelection {
    /// Explicit ports, chord and timing (tests, unusual hosts); [`crate::system_selection`] is the
    /// shells' constructor.
    pub fn with_ports(clipboard: Arc<dyn ClipboardPort>, keys: Arc<dyn KeystrokePort>, chord: Chord, options: CopyOptions) -> Self {
        Self { clipboard, keys, chord, options }
    }

    /// The chord this copier presses.
    pub fn chord(&self) -> Chord {
        self.chord
    }

    /// The timing in force.
    pub fn options(&self) -> CopyOptions {
        self.options
    }

    /// Poll the clipboard until two reads in a row return the same non-empty text, or the timeout
    /// passes (then the last non-empty text read, if any). One read is not enough: arboard's X11
    /// read tries several targets one round trip at a time (`UTF8_STRING` … `STRING`), and when the
    /// application takes the clipboard over between two of them the read answers with the Latin-1
    /// `STRING` fallback — CJK as `?` (measured under Xvfb with a Tk entry, docs/dictation.md §19).
    fn wait_for_text(&self) -> Option<String> {
        let deadline = Instant::now() + self.options.timeout;
        let mut last: Option<String> = None;
        loop {
            match self.clipboard.read_text() {
                Ok(Some(text)) if !text.is_empty() => {
                    if last.as_deref() == Some(text.as_str()) {
                        return Some(text);
                    }
                    last = Some(text);
                }
                Ok(_) => {}
                Err(err) => tracing::debug!(%err, "clipboard read while waiting for the copy failed"),
            }
            let now = Instant::now();
            if now >= deadline {
                return last;
            }
            // A bounded poll: the application writes the clipboard in its own time, there is no
            // event to wait for across processes.
            std::thread::sleep(self.options.poll.min(deadline - now));
        }
    }

    /// Put `previous` back — only while the clipboard still holds what the copy produced
    /// (`copied`) or nothing: a text the user copied meanwhile is left alone.
    fn restore(&self, previous: Option<&str>, copied: Option<&str>) {
        let current = match self.clipboard.read_text() {
            Ok(current) => current,
            Err(err) => {
                tracing::debug!(%err, "clipboard unreadable after the copy; not restoring");
                return;
            }
        };
        let ours = match (current.as_deref(), copied) {
            (None | Some(""), _) => true,
            (Some(current), Some(copied)) => current == copied,
            (Some(_), None) => false,
        };
        if !ours {
            tracing::debug!("clipboard changed during the selection copy; leaving it alone");
            return;
        }
        let result = match previous {
            Some(previous) => self.clipboard.write_text(previous),
            None if current.is_some() => self.clipboard.clear(),
            None => Ok(()),
        };
        if let Err(err) = result {
            tracing::warn!(%err, "could not restore the clipboard after the selection copy");
        }
    }
}

impl SelectionSource for ClipboardSelection {
    fn copy_selection(&self, held: &[Modifier]) -> Result<Option<String>, InjectError> {
        let previous = match self.clipboard.read_text() {
            Ok(previous) => previous,
            Err(err) => {
                tracing::debug!(%err, "could not read the clipboard before the copy; nothing will be restored");
                None
            }
        };
        // Cleared first: a selection equal to the old clipboard text still shows up as a change.
        self.clipboard.clear()?;
        if let Err(err) = self.keys.press_chord(self.chord, held) {
            self.restore(previous.as_deref(), None);
            tracing::warn!(%err, chord = %self.chord, "the copy chord could not be pressed");
            return Err(InjectError::Keystroke(err.to_string()));
        }
        let copied = self.wait_for_text();
        self.restore(previous.as_deref(), copied.as_deref());
        // Only the length reaches the log, never the text.
        tracing::debug!(chord = %self.chord, chars = copied.as_deref().map(|t| t.chars().count()), "selection copy");
        Ok(copied)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::injector::tests::{CLEARED, MemoryClipboard};
    use crate::injector::{Delivered, DeliveryError};
    use crate::toolchain::{Delivery, TypingTool, YdotoolSyntax};

    /// The application under the cursor: when the copy chord arrives it puts `selection` on the
    /// clipboard (after `delay`, on its own thread, like a real application), and logs the chord.
    struct App {
        clipboard: Arc<MemoryClipboard>,
        selection: Option<String>,
        delay: Duration,
        pressed: Mutex<Vec<(Chord, Vec<Modifier>)>>,
        outcome: Result<(), DeliveryError>,
        /// When the application put its copy on the clipboard.
        wrote_at: Arc<Mutex<Option<Instant>>>,
    }

    impl App {
        fn with(clipboard: &Arc<MemoryClipboard>, selection: Option<&str>) -> Arc<Self> {
            Arc::new(Self {
                clipboard: clipboard.clone(),
                selection: selection.map(str::to_owned),
                delay: Duration::ZERO,
                pressed: Mutex::new(Vec::new()),
                outcome: Ok(()),
                wrote_at: Arc::default(),
            })
        }
    }

    impl KeystrokePort for App {
        fn deliver(&self, _: Chord, _: &str) -> Result<Delivered, DeliveryError> {
            unreachable!("the copy never pastes")
        }

        fn press_chord(&self, chord: Chord, held: &[Modifier]) -> Result<Delivered, DeliveryError> {
            self.pressed.lock().unwrap().push((chord, held.to_vec()));
            self.outcome.clone()?;
            if let Some(selection) = self.selection.clone() {
                let clipboard = self.clipboard.clone();
                let delay = self.delay;
                let wrote_at = self.wrote_at.clone();
                let writer = std::thread::spawn(move || {
                    std::thread::sleep(delay);
                    clipboard.write_text(&selection).unwrap();
                    *wrote_at.lock().unwrap() = Some(Instant::now());
                });
                if delay.is_zero() {
                    writer.join().unwrap();
                }
            }
            Ok(Delivered { tool: "fake".into(), delivery: Delivery::Chord })
        }
    }

    fn fast() -> CopyOptions {
        CopyOptions { timeout: Duration::from_millis(200), poll: Duration::from_millis(2) }
    }

    fn copier(clipboard: &Arc<MemoryClipboard>, app: Arc<App>) -> ClipboardSelection {
        ClipboardSelection::with_ports(clipboard.clone(), app, copy_chord_for("linux"), fast())
    }

    #[test]
    fn chords_defaults_and_accessors() {
        assert_eq!(copy_chord_for("linux").to_string(), "Ctrl+Insert");
        assert_eq!(copy_chord_for("windows").to_string(), "Ctrl+Insert");
        assert_eq!(copy_chord_for("macos").to_string(), "Cmd+C");
        assert_eq!(copy_chord(), copy_chord_for(std::env::consts::OS));
        assert_eq!(CopyOptions::default(), CopyOptions { timeout: Duration::from_millis(250), poll: Duration::from_millis(10) });
        let clipboard = Arc::new(MemoryClipboard::default());
        let c = copier(&clipboard, App::with(&clipboard, None));
        assert_eq!((c.chord(), c.options()), (copy_chord_for("linux"), fast()));
    }

    /// docs/dictation.md §19: the copy chord per Linux tool (the same command lines as the paste,
    /// with `Insert` — ydotool ≥ 1.0 wants KEY_INSERT = 110), and `kwtype` has nothing to offer a
    /// copy. Never `c` with Ctrl: that is a terminal's interrupt (2026-09-26).
    #[test]
    fn regression_copy_chord_command_lines_per_tool_press_ctrl_insert_not_ctrl_c() {
        let chord = copy_chord_for("linux");
        let args = |tool: TypingTool| tool.command(chord, "").map(|c| (c.program, c.args.join(" "), c.stdin)).unwrap();
        assert_eq!(args(TypingTool::Xdotool), ("xdotool".into(), "key --clearmodifiers ctrl+Insert".into(), None));
        assert_eq!(args(TypingTool::Wtype), ("wtype".into(), "-M ctrl -k Insert -m ctrl".into(), None));
        assert_eq!(args(TypingTool::Dotool), ("dotool".into(), String::new(), Some("key ctrl+Insert\n".into())));
        assert_eq!(args(TypingTool::Ydotool(YdotoolSyntax::KeyCodes)), ("ydotool".into(), "key 29:1 110:1 110:0 29:0".into(), None));
        assert_eq!(args(TypingTool::Ydotool(YdotoolSyntax::KeyNames)), ("ydotool".into(), "key ctrl+Insert".into(), None));
        assert_eq!(chord.evdev_codes(), vec![29, 110]);
        assert_eq!(TypingTool::Kwtype.delivery(), Delivery::Type, "a typing tool: the chain skips it for the copy");
        assert_eq!(copy_chord_for("macos").evdev_codes(), vec![125, 46]);
    }

    /// The order is the contract: read the previous text, clear, press, read until the copy shows,
    /// then put the previous text back (the clipboard ends where it started).
    #[test]
    fn copy_saves_clears_presses_reads_and_restores_in_order() {
        let clipboard = MemoryClipboard::holding("用户原来的剪贴板");
        let app = App::with(&clipboard, Some("选中的一段话"));
        let got = copier(&clipboard, app.clone()).copy_selection(&[Modifier::Alt]).unwrap();
        assert_eq!(got.as_deref(), Some("选中的一段话"));
        assert_eq!(clipboard.writes(), vec![CLEARED.to_string(), "选中的一段话".into(), "用户原来的剪贴板".into()], "clear → app copy → restore");
        assert_eq!(clipboard.current().as_deref(), Some("用户原来的剪贴板"));
        assert_eq!(app.pressed.lock().unwrap().as_slice(), &[(copy_chord_for("linux"), vec![Modifier::Alt])], "the held modifiers reach the keyboard port");
        // The selection equals the old clipboard text: the clear makes the copy visible anyway.
        let clipboard = MemoryClipboard::holding("same");
        let got = copier(&clipboard, App::with(&clipboard, Some("same"))).copy_selection(&[]).unwrap();
        assert_eq!(got.as_deref(), Some("same"));
        // An empty clipboard before: it is emptied again afterwards, the selection does not linger.
        let clipboard = Arc::new(MemoryClipboard::default());
        let got = copier(&clipboard, App::with(&clipboard, Some("x"))).copy_selection(&[]).unwrap();
        assert_eq!(got.as_deref(), Some("x"));
        assert_eq!(clipboard.current(), None);
        assert_eq!(clipboard.writes(), vec![CLEARED.to_string(), "x".into(), CLEARED.to_string()]);
    }

    /// A slow application (the copy lands after a few polls) is still read, and the copy returns
    /// as soon as the text shows. The bound is the one this test always had, 200 ms with the
    /// application's 40 ms in it, now timed from when the text showed: on main CI's Intel Mac
    /// (2026-09-30) the application's thread ran late enough that the text came near the timeout,
    /// and how late a thread is scheduled says nothing about the copy. So the timeout only has to
    /// outlast that thread.
    #[test]
    fn a_copy_that_lands_late_within_the_timeout_is_read() {
        let clipboard = MemoryClipboard::holding("before");
        let app = Arc::new(App { delay: Duration::from_millis(40), ..Arc::into_inner(App::with(&clipboard, Some("late"))).unwrap() });
        let wrote_at = app.wrote_at.clone();
        let options = CopyOptions { timeout: Duration::from_secs(5), ..fast() };
        let got = ClipboardSelection::with_ports(clipboard.clone(), app, copy_chord_for("linux"), options).copy_selection(&[]).unwrap();
        let returned = Instant::now();
        assert_eq!(got.as_deref(), Some("late"));
        let shown = wrote_at.lock().unwrap().expect("the application copied");
        assert!(returned - shown < Duration::from_millis(200 - 40), "returned {:?} after the text showed", returned - shown);
        assert_eq!(clipboard.current().as_deref(), Some("before"));
    }

    /// Measured under Xvfb with a Tk entry: the first read after the copy can straddle the
    /// application's takeover of the clipboard and come back as the Latin-1 `STRING` fallback
    /// (`voltip ???? selection`). A text is only trusted once two reads agree.
    #[test]
    fn regression_a_read_that_straddles_the_takeover_is_not_trusted() {
        struct Scripted {
            reads: Mutex<std::collections::VecDeque<Option<&'static str>>>,
            writes: Mutex<Vec<String>>,
        }
        impl ClipboardPort for Scripted {
            fn read_text(&self) -> Result<Option<String>, InjectError> {
                let mut reads = self.reads.lock().unwrap();
                let next = if reads.len() > 1 { reads.pop_front().flatten() } else { reads.front().copied().flatten() };
                Ok(next.map(str::to_owned))
            }
            fn write_text(&self, text: &str) -> Result<(), InjectError> {
                self.writes.lock().unwrap().push(text.to_owned());
                Ok(())
            }
        }
        struct Pressed;
        impl KeystrokePort for Pressed {
            fn deliver(&self, _: Chord, _: &str) -> Result<Delivered, DeliveryError> {
                unreachable!("the copy never pastes")
            }
            fn press_chord(&self, _: Chord, _: &[Modifier]) -> Result<Delivered, DeliveryError> {
                Ok(Delivered { tool: "fake".into(), delivery: Delivery::Chord })
            }
        }
        let good = "voltip 选中文本 selection";
        let clipboard = Arc::new(Scripted {
            // previous text, then (after the clear) empty, the mangled fallback, the real text twice.
            reads: Mutex::new([Some("before"), None, Some("voltip ???? selection"), Some(good), Some(good)].into_iter().collect()),
            writes: Mutex::new(Vec::new()),
        });
        let c = ClipboardSelection::with_ports(clipboard.clone(), Arc::new(Pressed), copy_chord_for("linux"), fast());
        assert_eq!(c.copy_selection(&[]).unwrap().as_deref(), Some(good));
        assert_eq!(clipboard.writes.lock().unwrap().as_slice(), ["", "before"].map(String::from), "clear (the default: an empty text), then the restore");
        // A text that keeps changing until the timeout: the last one read is the answer.
        struct Flicker(std::sync::atomic::AtomicUsize);
        impl ClipboardPort for Flicker {
            fn read_text(&self) -> Result<Option<String>, InjectError> {
                let n = self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok((n > 0).then(|| if n.is_multiple_of(2) { "a" } else { "b" }.to_owned()))
            }
            fn write_text(&self, _: &str) -> Result<(), InjectError> {
                Ok(())
            }
        }
        let c = ClipboardSelection::with_ports(Arc::new(Flicker(0.into())), Arc::new(Pressed), copy_chord_for("linux"), fast());
        assert!(matches!(c.copy_selection(&[]).unwrap().as_deref(), Some("a" | "b")));
    }

    /// Nothing selected: the clipboard stays empty until the timeout, the answer is `None`, and the
    /// previous text comes back.
    #[test]
    fn nothing_selected_is_none_and_the_clipboard_is_restored() {
        let clipboard = MemoryClipboard::holding("keep me");
        let started = Instant::now();
        let got = copier(&clipboard, App::with(&clipboard, None)).copy_selection(&[]).unwrap();
        assert_eq!(got, None);
        assert!(started.elapsed() >= Duration::from_millis(200), "waited the whole timeout");
        assert_eq!(clipboard.current().as_deref(), Some("keep me"));
        assert_eq!(clipboard.writes(), vec![CLEARED.to_string(), "keep me".into()]);
        // Nothing before, nothing selected: nothing to write back either.
        let clipboard = Arc::new(MemoryClipboard::default());
        assert_eq!(copier(&clipboard, App::with(&clipboard, None)).copy_selection(&[]).unwrap(), None);
        assert_eq!(clipboard.writes(), vec![CLEARED.to_string()]);
    }

    /// The user copies something between the copy and the restore: their copy stays; only our own
    /// copy (or an empty clipboard) is replaced by the previous text.
    #[test]
    fn a_copy_the_user_made_meanwhile_is_not_overwritten() {
        let clipboard = MemoryClipboard::holding("old");
        let c = copier(&clipboard, App::with(&clipboard, None));
        // Simulate the user copying after the wait but before the restore.
        *clipboard.text.lock().unwrap() = Some("fresh".into());
        c.restore(Some("old"), None);
        assert_eq!(clipboard.current().as_deref(), Some("fresh"), "a text that is not ours is left alone");
        c.restore(Some("old"), Some("fresh"));
        assert_eq!(clipboard.current().as_deref(), Some("old"), "our own copy is replaced by the previous text");
    }

    /// The chord could not be pressed (no tool, a tool failed): typed error, the clipboard is put
    /// back; the clipboard itself failing is its own error and nothing is pressed.
    #[test]
    fn keystroke_and_clipboard_failures_are_typed_and_restore() {
        for outcome in [
            DeliveryError::Unavailable(crate::InjectNote::new(crate::FallbackCode::NoTool, "no copy tool on Wayland · GNOME")),
            DeliveryError::Failed("wtype: exited 1".into()),
        ] {
            let clipboard = MemoryClipboard::holding("previous");
            let app = Arc::new(App { outcome: Err(outcome.clone()), ..Arc::into_inner(App::with(&clipboard, Some("never"))).unwrap() });
            let err = copier(&clipboard, app).copy_selection(&[]).unwrap_err();
            assert_eq!(err, InjectError::Keystroke(outcome.to_string()));
            assert_eq!(clipboard.current().as_deref(), Some("previous"));
        }
        let clipboard = MemoryClipboard::holding("previous");
        *clipboard.fail_write.lock().unwrap() = Some(InjectError::Clipboard("locked".into()));
        let app = App::with(&clipboard, Some("never"));
        assert_eq!(copier(&clipboard, app.clone()).copy_selection(&[]).unwrap_err(), InjectError::Clipboard("locked".into()));
        assert!(app.pressed.lock().unwrap().is_empty(), "no chord without a cleared clipboard");
        // An unreadable clipboard before the copy still copies (nothing to restore).
        let clipboard = Arc::new(MemoryClipboard::default());
        *clipboard.fail_read.lock().unwrap() = Some(InjectError::Clipboard("busy".into()));
        let c = copier(&clipboard, App::with(&clipboard, Some("sel")));
        assert_eq!(c.copy_selection(&[]).unwrap(), None, "reads keep failing: nothing is seen");
        *clipboard.fail_read.lock().unwrap() = None;
        assert_eq!(c.copy_selection(&[]).unwrap().as_deref(), Some("sel"));
        // A restore that cannot write is logged, not surfaced.
        let clipboard = MemoryClipboard::holding("previous");
        let c = copier(&clipboard, App::with(&clipboard, None));
        *clipboard.text.lock().unwrap() = None;
        *clipboard.fail_write.lock().unwrap() = Some(InjectError::Clipboard("gone".into()));
        c.restore(Some("previous"), None);
        assert_eq!(clipboard.current(), None);
    }

    /// The default `KeystrokePort::press_chord` refuses: a port that cannot press a bare chord
    /// cannot copy, and says so.
    #[test]
    fn a_port_without_press_chord_refuses_the_copy() {
        struct PasteOnly;
        impl KeystrokePort for PasteOnly {
            fn deliver(&self, _: Chord, _: &str) -> Result<Delivered, DeliveryError> {
                Ok(Delivered { tool: "paste-only".into(), delivery: Delivery::Chord })
            }
        }
        assert!(PasteOnly.deliver(copy_chord_for("linux"), "x").is_ok());
        let clipboard = MemoryClipboard::holding("previous");
        let c = ClipboardSelection::with_ports(clipboard.clone(), Arc::new(PasteOnly), copy_chord_for("linux"), fast());
        let err = c.copy_selection(&[]).unwrap_err();
        assert!(matches!(&err, InjectError::Keystroke(m) if m.contains("cannot press a chord")), "{err:?}");
        assert_eq!(clipboard.current().as_deref(), Some("previous"));
    }
}
