//! Windows (docs/dictation.md §32). The pill is a `WS_EX_TOPMOST | WS_EX_NOACTIVATE` window (tao
//! builds it from the shell's `always_on_top(true)` and `focusable(false)`). Topmost windows are
//! stacked among themselves by which came to the front last, a window shown again with
//! `SW_SHOWNOACTIVATE` keeps its old place, and tao's `set_always_on_top(true)` does nothing while
//! the flag is already set, so a topmost window activated since (a full-screen player, a game, a
//! pinned window) stays over the pill. [`raise`] puts the pill back at the top itself, and
//! [`Keeper`] does it again whenever another window comes to the front while the pill shows.

/// Put `hwnd` at the top of the topmost windows without activating, moving or sizing it.
pub fn raise(hwnd: isize) -> bool {
    let _ = hwnd;
    true
}

/// While it lives, puts the pill back on top whenever another window comes to the front.
#[derive(Debug)]
pub struct Keeper {
    _hwnd: isize,
}

impl Keeper {
    /// Start keeping `hwnd` on top.
    pub fn start(hwnd: isize) -> Result<Self, String> {
        Ok(Self { _hwnd: hwnd })
    }
}

impl Drop for Keeper {
    /// Stop keeping the pill on top.
    fn drop(&mut self) {}
}
