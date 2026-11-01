//! macOS (docs/dictation.md §32). tao puts an always-on-top window at the floating level, the
//! level of every other application's floating window (a pinned note, a player kept on top), so
//! one of those that comes to the front later covers the pill; and a window joins another
//! application's full-screen Space only when its collection behaviour says it may. [`raise`] moves
//! the pill to the status window level (above floating windows and the Dock, below menus), lets
//! it join every Space including full-screen ones, keeps it out of Mission Control and the window
//! cycle, and orders it to the front without activating Voltip.

#![allow(unsafe_code)]

use std::ffi::c_void;

use objc2_app_kit::{NSStatusWindowLevel, NSWindow, NSWindowCollectionBehavior};

/// The collection behaviour the pill needs: on every Space, on full-screen Spaces, and left
/// where it is by Mission Control and the window cycle.
pub fn pill_behaviour(current: NSWindowCollectionBehavior) -> NSWindowCollectionBehavior {
    // A window cannot be both a full-screen window and an auxiliary one of another.
    let without = current - NSWindowCollectionBehavior::FullScreenPrimary - NSWindowCollectionBehavior::MoveToActiveSpace;
    without
        | NSWindowCollectionBehavior::CanJoinAllSpaces
        | NSWindowCollectionBehavior::FullScreenAuxiliary
        | NSWindowCollectionBehavior::Stationary
        | NSWindowCollectionBehavior::IgnoresCycle
}

/// Put the pill window at the front of the status level on every Space, without activating the
/// application. Main thread only (AppKit); `ns_window` is the pill's `NSWindow` (tauri's
/// `ns_window()`), a null pointer does nothing.
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
    let behaviour = pill_behaviour(window.collectionBehavior());
    if window.collectionBehavior() != behaviour {
        window.setCollectionBehavior(behaviour);
    }
    window.orderFrontRegardless();
}
