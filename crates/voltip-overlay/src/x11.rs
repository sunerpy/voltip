//! X11 (docs/dictation.md §32). The pill is a window the window manager keeps above the others
//! (tao sets `_NET_WM_STATE_ABOVE` for `always_on_top`), but mapping a window again keeps its old
//! place in the stacking order, and without a window manager (or under one that keeps no layers)
//! that order is all there is. [`Stacker::raise`] restacks the pill on top (`ConfigureWindow` with
//! `stack_mode = Above`), which the X server applies at once and a window manager applies within
//! its own layers. A focused full-screen window stays over the windows kept above (the EWMH
//! stacking order), and a pure Wayland session has no stacking a client can ask for.

use std::ffi::OsStr;
use std::sync::{Mutex, PoisonError};

use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::{ConfigureWindowAux, ConnectionExt as _, StackMode};
use x11rb::rust_connection::RustConnection;

/// Restacks the pill window on top through a connection of its own, opened at the first raise,
/// kept, and dropped on any error (the next raise reconnects).
#[derive(Debug, Default)]
pub struct Stacker {
    connection: Mutex<Option<RustConnection>>,
}

impl Stacker {
    /// Put `window` on top of the stacking order on `$DISPLAY`; without one (pure Wayland, no
    /// session) there is nothing to do.
    pub fn raise(&self, window: u32) -> Result<(), String> {
        self.raise_on(std::env::var_os("DISPLAY").as_deref(), window)
    }

    /// [`Stacker::raise`] on an explicit display name (tests pass the server they started).
    pub fn raise_on(&self, display: Option<&OsStr>, window: u32) -> Result<(), String> {
        let Some(name) = display.and_then(OsStr::to_str).filter(|d| !d.is_empty()) else { return Ok(()) };
        let mut connection = self.connection.lock().unwrap_or_else(PoisonError::into_inner);
        if connection.is_none() {
            *connection = Some(x11rb::connect(Some(name)).map_err(|e| format!("x11: {e}"))?.0);
        }
        let Some(conn) = connection.as_ref() else { return Ok(()) };
        let raised = conn
            .configure_window(window, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE))
            .map_err(|e| e.to_string())
            .and_then(|cookie| cookie.check().map_err(|e| e.to_string()))
            .and_then(|()| conn.flush().map_err(|e| e.to_string()));
        if let Err(e) = raised {
            *connection = None;
            return Err(format!("x11: {e}"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    /// Pure Wayland or no session: no display, nothing to raise and nothing to fail.
    #[test]
    fn without_a_display_there_is_nothing_to_raise() {
        let stacker = Stacker::default();
        assert_eq!(stacker.raise_on(None, 1), Ok(()));
        assert_eq!(stacker.raise_on(Some(OsStr::new("")), 1), Ok(()));
    }

    /// A display that cannot be reached (no server listens on :97) is an error each time; no
    /// connection is kept.
    #[test]
    fn an_unreachable_display_is_an_error_and_the_next_raise_connects_again() {
        let stacker = Stacker::default();
        for _ in 0..2 {
            let error = stacker.raise_on(Some(OsStr::new(":97")), 1).expect_err("no X server there");
            assert!(error.starts_with("x11: "), "{error}");
            assert!(stacker.connection.lock().unwrap_or_else(PoisonError::into_inner).is_none());
        }
    }
}
