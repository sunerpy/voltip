//! macOS (docs/dictation.md §32). tao puts an always-on-top window at the floating level, the
//! level of every other application's floating window, so one of those that comes to the front
//! later covers the pill; and a window joins another application's full-screen Space only when
//! its collection behaviour says it may. [`raise`] is to put the pill above them; in this commit it
//! does nothing, which is what the shell does today.

use std::ffi::c_void;

/// Put the pill window at the front of the status level on every Space, without activating the
/// application. Main thread only (AppKit).
pub fn raise(ns_window: *mut c_void) {
    let _ = ns_window;
}
