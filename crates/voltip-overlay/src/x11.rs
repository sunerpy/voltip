//! X11 (docs/dictation.md §32). Mapping a window again keeps its old place in the stacking order,
//! and without a window manager (or under one that keeps no layers) that order is all there is.
//! [`Stacker::raise`] is to restack the pill on top; in this commit it does nothing, which is what
//! the shell does today.

use std::ffi::OsStr;

/// Restacks the pill window on top of the X11 stacking order.
#[derive(Debug, Default)]
pub struct Stacker {
    _connection: (),
}

impl Stacker {
    /// Put `window` on top of the stacking order on `$DISPLAY`.
    pub fn raise(&self, window: u32) -> Result<(), String> {
        self.raise_on(std::env::var_os("DISPLAY").as_deref(), window)
    }

    /// [`Stacker::raise`] on an explicit display name (tests pass the server they started).
    pub fn raise_on(&self, display: Option<&OsStr>, window: u32) -> Result<(), String> {
        let _ = (display, window);
        Ok(())
    }
}
