//! Keeps the pill window (the desktop shell's `overlay.rs`) above the other windows without
//! taking the focus (docs/dictation.md §32). The shell builds the window hidden, always on top and
//! not focusable, and only shows and hides it afterwards; showing a window does not move it up the
//! stacking order, so a window that came to the front in the meantime stays over the pill and the
//! user thinks the take failed. One module per platform:
//!
//! - Windows (`windows.rs`): [`windows::raise`] puts the pill at the top of the topmost windows
//!   (`SetWindowPos(HWND_TOPMOST)` without activating it) on every show and phase, and
//!   [`windows::Keeper`] does it again whenever another window comes to the front while the pill
//!   shows (a foreground WinEvent hook; no polling).
//! - macOS (`macos.rs`): the status window level, full-screen Spaces and `orderFrontRegardless`.
//! - Linux (`x11.rs`): the X11 stacking order; a Wayland compositor stacks windows itself.
//!
//! The `#[ignore]`d tests open real windows: CI runs them on a Windows Server desktop, a macOS
//! session and Xvfb, never on a computer someone is using.

#[cfg(target_os = "windows")]
pub mod windows;
