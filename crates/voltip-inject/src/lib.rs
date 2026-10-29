//! Putting dictated text into whatever application has focus.
//!
//! * [`Injector`] — the one operation: `inject(text)`. Implementations report how the text
//!   arrived ([`Via::Paste`] typed a paste chord, [`Via::Clipboard`] left it on the clipboard for
//!   the user to paste) and why, if a step was skipped ([`Injection::note`]).
//! * [`ClipboardPasteInjector`] — the default: save the whole clipboard ([`ClipSnapshot`]: every
//!   format, see [`clip`]), write ours, wait [`PasteOptions::paste_delay`], deliver through a
//!   [`KeystrokePort`] (`Ctrl+V` / `Cmd+V` by default, see [`PasteMethod`]), then put the
//!   clipboard back [`PasteOptions::paste_delay_after`] after the last paste if it still holds
//!   ours ([`PasteOptions::for_os`]: 60 / 120 ms on Linux, 40 / 600 ms on Windows and macOS). A
//!   take's pieces share one [`InjectSession`] ([`Injector::begin_session`], docs/dictation.md
//!   §14.3), so the clipboard comes back once. Two failure shapes ([`DeliveryError`]): nothing
//!   could be tried → `Via::Clipboard` with the reason, text left for the user; a tool ran and
//!   failed → the previous clipboard is put back at once and the piece returns
//!   [`InjectError::Keystroke`].
//! * [`ClipboardOnlyInjector`] — for the `inject = clipboard_only` setting: never types.
//! * [`system_paste_injector`] — the shells' constructor: enigo on Windows / macOS, the Linux tool
//!   chain below on Linux.
//! * [`ClipboardSelection`] ([`SelectionSource`], [`system_selection`]) — the reverse direction for
//!   voice editing (docs/dictation.md §19): save the clipboard, clear it, press the copy chord
//!   ([`copy_chord`]: `Ctrl+Insert`, macOS `Cmd+C`; never `Ctrl+C`, a terminal's interrupt) after
//!   releasing the hotkey modifiers the user may still hold, wait up to [`COPY_TIMEOUT`] for the
//!   selection to land, read it, restore.
//! * [`FakeInjector`] (feature `test-support`, always on for this crate's tests) — records calls
//!   and returns a configured outcome, so the core and the shells test without a display.
//! * [`Session`], [`toolchain`], [`puts_back`], [`check_display`] — the decisions, as pure
//!   functions.
//!
//! # Platform notes
//!
//! * **Windows**: arboard reads and writes text through the Win32 clipboard, `clipboard-win`'s raw
//!   API keeps and restores its other formats; enigo uses `SendInput`. No setup needed.
//! * **macOS**: arboard and `objc2-app-kit` use `NSPasteboard`; enigo posts `CGEvent`s, which requires the app to
//!   be trusted under *Privacy & Security → Accessibility*. Without it `Enigo::new` fails (the
//!   prompt is never opened from here, `open_prompt_to_get_permissions = false`) and the result
//!   is `Via::Clipboard` with the reason in the note. The `v` of Cmd+V is sent as a virtual
//!   keycode (`voltip_platform::macos::cmd_v_keycode`, [`SystemKeys::with_mac_v_keycode`]) and
//!   held 100 ms: enigo's own layout lookup turns it into Cmd+A on layouts without a `v` key
//!   ([`platform::chord_steps`]).
//! * **Linux** (docs/dictation.md §14): the session is judged from the environment
//!   ([`Session::detect`]: X11 / XWayland / Wayland × KDE / GNOME / wlroots / other). The
//!   clipboard is arboard: wlr-data-control on a Wayland compositor that offers it, X11
//!   otherwise. The paste walks [`toolchain::candidates`] — X11: enigo (XTEST) →
//!   `xdotool --clearmodifiers`; Wayland: `wtype` → `dotool` → `ydotool` (grammar judged from
//!   `--help`) → `kwtype` (KDE) → enigo's `zwp_virtual_keyboard_v1`, plus the X11 pair on
//!   XWayland; GNOME skips the virtual-keyboard tools. Unavailable tools are skipped, a chord tool
//!   that fails hands over to the next one (they fail before sending anything), a typing tool
//!   (`kwtype`) or a timeout stops the chain ([`linux::classify`]); with nothing available the
//!   text stays on the clipboard (`Via::Clipboard`, the note names the skipped tools and what to
//!   install). External tools run with stdin/stdout closed, stderr captured, and a deadline. When neither
//!   `DISPLAY` nor `WAYLAND_DISPLAY` is set the clipboard itself is unreachable and `inject`
//!   returns [`InjectError::NoDisplay`]. Headless CI therefore runs the fakes and the pure
//!   functions; the one real test is `#[ignore]` and runs with
//!   `DISPLAY=:99 cargo test -p voltip-inject -- --ignored` under Xvfb.
//! * The clipboard is kept whole across an injection (docs/dictation.md §14.3): Windows' raw
//!   formats, every item of a macOS pasteboard, and on Linux what arboard reads (a file list, an
//!   image, HTML with its text, text; one of them comes back). Handle-only formats, data an owner
//!   does not render and more than [`MAX_SNAPSHOT_BYTES`] are not; the snapshot says so
//!   ([`ClipSnapshot::complete`]). What is written to be put back later, and every restore, is
//!   marked for clipboard histories (Windows' history and cloud clipboard,
//!   `org.nspasteboard.TransientType`, `x-kde-passwordManagerHint`).

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::sync::Arc;

pub mod clip;
#[cfg(any(test, feature = "test-support"))]
mod fake;
mod injector;
#[cfg(target_os = "linux")]
pub mod linux;
mod os_clip;
pub mod platform;
#[cfg(unix)]
pub mod process;
pub mod selection;
pub mod session;
pub mod toolchain;

pub use clip::{ClipFormat, ClipSnapshot, MAX_SNAPSHOT_BYTES, TEXT_FORMAT};
#[cfg(any(test, feature = "test-support"))]
pub use fake::FakeInjector;
pub use injector::{
    ClipboardOnlyInjector, ClipboardPasteInjector, ClipboardPort, DEFAULT_RESTORE_DELAY, Delivered, DeliveryError, KeystrokePort, PASTE_DELAY,
    PASTE_DELAY_AFTER, PASTE_SETTLE, PasteOptions, check_display, paste_chord, paste_chord_for, puts_back,
};
pub use platform::{EnigoBackend, EnigoError, SystemClipboard, SystemKeys, display_available, resolve_backend};
pub use selection::{COPY_POLL, COPY_TIMEOUT, ClipboardSelection, CopyOptions, SelectionSource, copy_chord, copy_chord_for};
pub use session::{Desktop, Session, SessionKind, X11Grab, remote_command, toggle_command};
pub use toolchain::{Chord, Key, Modifier, PasteMethod, TypingTool};

/// Something that can put text into the focused application.
pub trait Injector: Send + Sync + 'static {
    /// Deliver `text`. `Ok` means the text is at least on the clipboard; `Err` means it is not
    /// (or, for [`InjectError::Keystroke`], that a paste was attempted and failed and the previous
    /// clipboard text was put back).
    fn inject(&self, text: &str) -> Result<Injection, InjectError>;
    /// Short stable name for logs and the history (`"clipboard+paste"`, `"clipboard-only"`).
    fn describe(&self) -> &'static str;

    /// A session for the pieces of one take's text (docs/dictation.md §14.3), with the clipboard
    /// handled as `after` says. The default delivers each piece through [`Injector::inject`]
    /// after asking `may_deliver`, and has nothing to put back ([`ClipboardOnlyInjector`] leaves
    /// each piece on the clipboard).
    fn begin_session(self: Arc<Self>, after: AfterPaste) -> Box<dyn InjectSession> {
        let _ = after;
        Box::new(EachPiece(Some(self)))
    }
}

/// What the clipboard holds once a session's text is in (the `clipboard_after` setting,
/// docs/dictation.md §14.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AfterPaste {
    /// What the user had copied comes back after the paste (the default).
    #[default]
    Restore,
    /// The pasted text stays on the clipboard, as an ordinary copy.
    Keep,
}

/// What became of one piece of a session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PieceOutcome {
    /// The piece went out, or stayed on the clipboard for the user ([`Injection::via`]).
    Delivered(Injection),
    /// `may_deliver` said no right before the delivery: nothing reached the application, and the
    /// clipboard is back to what the session found.
    Cancelled,
}

/// The pieces of one take's text, delivered through one clipboard session (docs/dictation.md
/// §14.3): the clipboard is saved before the first piece and put back once, after the last.
/// A session that has ended ([`InjectSession::finish`], [`InjectSession::abort`]) delivers nothing
/// more: every later piece is [`PieceOutcome::Cancelled`] without touching the clipboard.
pub trait InjectSession: Send {
    /// Deliver `text`. `may_deliver` is asked right before the delivery — for a paste, once the
    /// text is on the clipboard and the target has had its settle delay — and `false` calls the
    /// piece off.
    fn piece(&mut self, text: &str, may_deliver: &dyn Fn() -> bool) -> Result<PieceOutcome, InjectError>;
    /// The take's text is all in: the clipboard comes back once the target has read the last paste.
    fn finish(&mut self);
    /// The take was abandoned: the clipboard comes back as soon as that is safe.
    fn abort(&mut self);
}

/// The default session: every piece through [`Injector::inject`], nothing to put back.
struct EachPiece<I: ?Sized>(Option<Arc<I>>);

impl<I: Injector + ?Sized> InjectSession for EachPiece<I> {
    fn piece(&mut self, text: &str, may_deliver: &dyn Fn() -> bool) -> Result<PieceOutcome, InjectError> {
        let Some(injector) = &self.0 else { return Ok(PieceOutcome::Cancelled) };
        if !may_deliver() {
            return Ok(PieceOutcome::Cancelled);
        }
        injector.inject(text).map(PieceOutcome::Delivered)
    }

    fn finish(&mut self) {
        self.0 = None;
    }

    fn abort(&mut self) {
        self.0 = None;
    }
}

/// The clipboard-and-paste injector for this process: enigo through [`SystemKeys`] on Windows and
/// macOS, the Linux tool chain ([`linux::ToolchainKeys`]) on Linux, with `options`.
pub fn system_paste_injector(options: PasteOptions) -> ClipboardPasteInjector {
    #[cfg(target_os = "linux")]
    {
        ClipboardPasteInjector::with_options(std::sync::Arc::new(SystemClipboard::new()), std::sync::Arc::new(linux::system_toolchain_keys()), options)
    }
    #[cfg(not(target_os = "linux"))]
    {
        ClipboardPasteInjector::with_options(std::sync::Arc::new(SystemClipboard::new()), std::sync::Arc::new(SystemKeys::native()), options)
    }
}

/// The selection copier for this process (docs/dictation.md §19): the same clipboard and keyboard
/// ports as [`system_paste_injector`], the platform's copy chord, `options`.
pub fn system_selection(options: CopyOptions) -> ClipboardSelection {
    #[cfg(target_os = "linux")]
    {
        ClipboardSelection::with_ports(std::sync::Arc::new(SystemClipboard::new()), std::sync::Arc::new(linux::system_toolchain_keys()), copy_chord(), options)
    }
    #[cfg(not(target_os = "linux"))]
    {
        ClipboardSelection::with_ports(std::sync::Arc::new(SystemClipboard::new()), std::sync::Arc::new(SystemKeys::native()), copy_chord(), options)
    }
}

/// One line naming the injection backend for logs and status strings: the session and the tool
/// the chain would pick right now (`Wayland · KDE → wtype`), or `enigo` off Linux.
pub fn backend_summary() -> String {
    #[cfg(target_os = "linux")]
    {
        let keys = linux::system_toolchain_keys();
        match keys.selection().tool {
            Some(tool) => format!("{} → {tool}", keys.session()),
            None => format!("{} → clipboard only", keys.session()),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        "enigo".to_string()
    }
}

/// How the text reached the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    /// A paste chord was sent (or a tool typed the text); the text should be in the focused input.
    Paste,
    /// The text is on the clipboard and the user pastes it themselves.
    Clipboard,
}

impl std::fmt::Display for Via {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Paste => "paste",
            Self::Clipboard => "clipboard",
        })
    }
}

/// Why a paste left the text on the clipboard, in the kinds the interface explains in words
/// (docs/dictation.md §4.2); the message itself goes under the technical details.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FallbackCode {
    /// The system does not let Voltip send keystrokes (macOS: Accessibility not granted).
    NoPermission,
    /// No paste tool for this session (Wayland without wtype, dotool or ydotool).
    NoTool,
    /// No connection to the display server.
    NoDisplay,
    /// A secure input field or the secure desktop has the keyboard.
    SecureInput,
    /// The window in front runs as administrator (Windows).
    ElevatedTarget,
    /// Anything else.
    Other,
}

/// A clipboard fallback: its kind and the original message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InjectNote {
    /// What the interface says.
    pub code: FallbackCode,
    /// The message as the tool or the system gave it (logs, technical details).
    pub detail: String,
}

impl InjectNote {
    /// A note of `code` with `detail`.
    pub fn new(code: FallbackCode, detail: impl Into<String>) -> Self {
        Self { code, detail: detail.into() }
    }
}

impl std::fmt::Display for InjectNote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.detail)
    }
}

/// Outcome of a successful [`Injector::inject`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Injection {
    /// How the text was delivered.
    pub via: Via,
    /// Characters delivered.
    pub chars: usize,
    /// Why a requested paste did not happen and the text stayed on the clipboard.
    pub note: Option<InjectNote>,
}

impl Injection {
    /// A paste that went through.
    pub fn pasted(chars: usize) -> Self {
        Self { via: Via::Paste, chars, note: None }
    }

    /// Text left on the clipboard, with the reason when there is one.
    pub fn clipboard(chars: usize, note: Option<InjectNote>) -> Self {
        Self { via: Via::Clipboard, chars, note }
    }
}

/// Why nothing was delivered.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum InjectError {
    /// The text was empty or whitespace only.
    #[error("nothing to insert: the text is empty")]
    EmptyText,
    /// No display server to talk to (Linux without `DISPLAY` / `WAYLAND_DISPLAY`).
    #[error("no display server: {0}")]
    NoDisplay(String),
    /// The clipboard could not be read or written.
    #[error("clipboard: {0}")]
    Clipboard(String),
    /// A paste was attempted and failed; the previous clipboard text was put back when there was
    /// one (the message says so when it was not).
    #[error("keystroke: {0}")]
    Keystroke(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn via_and_error_display() {
        assert_eq!(Via::Paste.to_string(), "paste");
        assert_eq!(Via::Clipboard.to_string(), "clipboard");
        assert_eq!(InjectError::EmptyText.to_string(), "nothing to insert: the text is empty");
        assert_eq!(InjectError::NoDisplay("DISPLAY unset".into()).to_string(), "no display server: DISPLAY unset");
        assert_eq!(InjectError::Clipboard("busy".into()).to_string(), "clipboard: busy");
        assert_eq!(InjectError::Keystroke("no permission".into()).to_string(), "keystroke: no permission");
    }

    #[test]
    fn injection_constructors() {
        assert_eq!(Injection::pasted(3), Injection { via: Via::Paste, chars: 3, note: None });
        let note = InjectNote::new(FallbackCode::NoTool, "x");
        assert_eq!(Injection::clipboard(2, Some(note.clone())), Injection { via: Via::Clipboard, chars: 2, note: Some(note.clone()) });
        assert_eq!(note.to_string(), "x", "the message, as the logs show it");
    }

    #[test]
    fn system_constructors_are_headless_safe() {
        // `backend_summary` may run `ydotool --help` when it is installed.
        #[cfg(unix)]
        let _spawn = crate::process::tests::spawn_lock();
        let injector = system_paste_injector(PasteOptions::default());
        assert_eq!(injector.describe(), "clipboard+paste");
        assert_eq!(injector.options(), PasteOptions::default());
        assert_eq!(injector.chord(), paste_chord());
        let selection = system_selection(CopyOptions::default());
        assert_eq!((selection.chord(), selection.options()), (copy_chord(), CopyOptions::default()));
        if display_available().is_err() {
            // No display: the clipboard cannot even be cleared, so nothing is pressed.
            assert!(matches!(selection.copy_selection(&[]), Err(InjectError::NoDisplay(_))));
        }
        let summary = backend_summary();
        assert!(!summary.is_empty());
        if cfg!(target_os = "linux") && display_available().is_err() {
            // No display: the session falls back to X11 · other and nothing is selectable.
            assert!(summary.starts_with("X11 · other → "), "{summary}");
        }
    }
}
