//! Keeps the pill window (the desktop shell's `overlay.rs`) above the other windows without
//! taking the focus (docs/dictation.md §32). The shell builds the window hidden, always on top and
//! not focusable, and only shows and hides it afterwards; showing a window does not move it up the
//! stacking order, so a window that came to the front in the meantime stays over the pill and the
//! user thinks the take failed. One module per platform:
//!
//! - Windows (`windows.rs`): [`windows::raise`] puts the pill at the top of the topmost windows
//!   (`SetWindowPos(HWND_TOPMOST)` without activating it) on every show and phase, and
//!   [`windows::Keeper`] does it again while the pill shows, whenever another window comes to the
//!   front and whenever a window that covers it appears or is put on top (WinEvent hooks; no
//!   polling). When it raises the pill over a covering window is [`contest`]'s rule.
//! - macOS (`macos.rs`): the status window level and `orderFrontRegardless`; another application's
//!   full-screen Space takes only a non-activating panel, which the pill is not.
//! - Linux (`x11.rs`): the X11 stacking order; a Wayland compositor stacks windows itself.
//!
//! The `#[ignore]`d tests open real windows: CI runs them on a Windows Server desktop, a macOS
//! session and Xvfb, never on a computer someone is using.

pub mod contest;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;
#[cfg(target_os = "linux")]
pub mod x11;
