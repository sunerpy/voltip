//! macOS (docs/dictation.md §32). tao puts an always-on-top window at the floating level, the
//! level of every other application's floating window (a pinned note, a player kept on top), so
//! one of those brought to the front while the pill shows covers it. [`raise`] moves the pill to
//! the status window level (above floating windows and the Dock, below menus) and orders it to
//! the front without activating Voltip.
//!
//! An application in full screen has a Space of its own, and macOS does not put tao's window
//! there, whatever its collection behaviour (`FullScreenAuxiliary` included) and whether Voltip
//! has a Dock icon or runs in the menu bar; a non-activating panel (`NSPanel`) it does. Making the
//! pill one is a change to how the window is built, not a raise (docs/dictation.md §32.3).

#![allow(unsafe_code)]

use std::ffi::c_void;

use objc2_app_kit::{NSStatusWindowLevel, NSWindow};

/// Put the pill window at the front of the status level, without activating the application.
/// Main thread only (AppKit); `ns_window` is the pill's `NSWindow` (tauri's `ns_window()`), a null
/// pointer does nothing.
pub fn raise(ns_window: *mut c_void) {
    if ns_window.is_null() {
        return;
    }
    // SAFETY: the caller hands over the pill's NSWindow, which lives as long as the pill window,
    // on the main thread.
    let window = unsafe { &*ns_window.cast::<NSWindow>() };
    if window.level() < NSStatusWindowLevel {
        window.setLevel(NSStatusWindowLevel);
    }
    window.orderFrontRegardless();
}
