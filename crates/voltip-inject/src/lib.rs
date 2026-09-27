//! Putting dictated text into whatever application has focus.
//!
//! * [`Injector`] — the one operation: `inject(text)`. Implementations report how the text
//!   arrived ([`Via::Paste`] typed a paste chord, [`Via::Clipboard`] left it on the clipboard for
//!   the user to paste) and why, if a step was skipped ([`Injection::note`]).
//! * [`ClipboardPasteInjector`] — the default: save the clipboard text, write ours, wait
//!   [`PasteOptions::paste_delay`], deliver through a [`KeystrokePort`] (`Ctrl+V` / `Cmd+V` by
//!   default, see [`PasteMethod`]), then restore the previous text after
//!   [`PasteOptions::paste_delay_after`] if the clipboard still holds ours
//!   ([`PasteOptions::for_os`]: 60 / 120 ms on Linux, 40 / 600 ms on Windows and macOS). Two
//!   failure shapes ([`DeliveryError`]): nothing could be tried → `Via::Clipboard` with the
//!   reason, text left for the user; a tool ran and failed → the previous clipboard is restored
//!   at once and `inject` returns [`InjectError::Keystroke`].
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
//! * [`Session`], [`toolchain`], [`should_restore`], [`check_display`] — the decisions, as pure
//!   functions.
//!
//! # Platform notes
//!
//! * **Windows**: arboard uses the Win32 clipboard, enigo uses `SendInput`. No setup needed.
//! * **macOS**: arboard uses `NSPasteboard`; enigo posts `CGEvent`s, which requires the app to
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
//! * Only **text** is preserved across an injection: an image or file list on the clipboard is
//!   replaced and not restored.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

#[cfg(any(test, feature = "test-support"))]
mod fake;
mod injector;
#[cfg(target_os = "linux")]
pub mod linux;
pub mod platform;
#[cfg(unix)]
pub mod process;
pub mod selection;
pub mod session;
pub mod toolchain;

#[cfg(any(test, feature = "test-support"))]
pub use fake::FakeInjector;
pub use injector::{
    ClipboardOnlyInjector, ClipboardPasteInjector, ClipboardPort, DEFAULT_RESTORE_DELAY, Delivered, DeliveryError, KeystrokePort, PASTE_DELAY,
    PASTE_DELAY_AFTER, PASTE_SETTLE, PasteOptions, check_display, paste_chord, paste_chord_for, should_restore,
};
pub use platform::{EnigoBackend, EnigoError, SystemClipboard, SystemKeys, display_available, resolve_backend};
pub use selection::{COPY_POLL, COPY_TIMEOUT, ClipboardSelection, CopyOptions, SelectionSource, copy_chord, copy_chord_for};
pub use session::{Desktop, Session, SessionKind, X11Grab, remote_command, toggle_command};
pub use toolchain::{Chord, Key, Modifier, PasteMethod, TypingTool};

/// Something that can put text into the focused application.
pub trait Injector: Send + Sync {
    /// Deliver `text`. `Ok` means the text is at least on the clipboard; `Err` means it is not
    /// (or, for [`InjectError::Keystroke`], that a paste was attempted and failed and the previous
    /// clipboard text was put back).
    fn inject(&self, text: &str) -> Result<Injection, InjectError>;
    /// Short stable name for logs and the history (`"clipboard+paste"`, `"clipboard-only"`).
    fn describe(&self) -> &'static str;
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

/// Outcome of a successful [`Injector::inject`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Injection {
    /// How the text was delivered.
    pub via: Via,
    /// Characters delivered.
    pub chars: usize,
    /// Why a step was skipped (e.g. no paste tool and the text stayed on the clipboard).
    pub note: Option<String>,
}

impl Injection {
    /// A paste that went through.
    pub fn pasted(chars: usize) -> Self {
        Self { via: Via::Paste, chars, note: None }
    }

    /// Text left on the clipboard, with the reason when there is one.
    pub fn clipboard(chars: usize, note: Option<String>) -> Self {
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
        assert_eq!(Injection::clipboard(2, Some("x".into())), Injection { via: Via::Clipboard, chars: 2, note: Some("x".into()) });
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
