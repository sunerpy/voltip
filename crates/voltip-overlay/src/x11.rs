//! X11 (docs/dictation.md §32). The pill is a window the window manager keeps above the others
//! (tao sets `_NET_WM_STATE_ABOVE` for `always_on_top`), but mapping a window again keeps its old
//! place in the stacking order, and without a window manager (or under one that keeps no layers)
//! that order is all there is. [`Stacker::raise`] restacks the pill on top (`ConfigureWindow` with
//! `stack_mode = Above`), which the X server applies at once and a window manager applies within
//! its own layers, and [`Keeper`] does it again while the pill shows whenever another window is
//! mapped or restacked over it. A focused full-screen window stays over the windows kept above
//! (the EWMH stacking order), some window managers ignore a restack an application in the
//! background asks for, and a pure Wayland session has no stacking a client can ask for.

use std::ffi::OsStr;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Instant;

use x11rb::connection::Connection as _;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    ChangeWindowAttributesAux, ClientMessageEvent, ConfigureWindowAux, ConnectionExt as _, CreateWindowAux, EventMask, MapState, StackMode, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;

use crate::contest::{Answer, Contest};

/// What a client message to the keeper's own window asks (its first value): end the thread, or
/// look again at the pill.
const STOP: u32 = 0;
const LOOK_AGAIN: u32 = 1;

/// The display a request goes to: `$DISPLAY`, or none (pure Wayland, no session).
fn display_name(display: Option<&OsStr>) -> Option<&str> {
    display.and_then(OsStr::to_str).filter(|d| !d.is_empty())
}

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
        let Some(name) = display_name(display) else { return Ok(()) };
        let mut connection = self.connection.lock().unwrap_or_else(PoisonError::into_inner);
        if connection.is_none() {
            *connection = Some(x11rb::connect(Some(name)).map_err(|e| format!("x11: {e}"))?.0);
        }
        let Some(conn) = connection.as_ref() else { return Ok(()) };
        if let Err(e) = restack(conn, window) {
            *connection = None;
            return Err(format!("x11: {e}"));
        }
        Ok(())
    }
}

/// `ConfigureWindow(stack_mode = Above)` for `window`, checked and flushed.
fn restack(conn: &RustConnection, window: Window) -> Result<(), String> {
    conn.configure_window(window, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE))
        .map_err(|e| e.to_string())
        .and_then(|cookie| cookie.check().map_err(|e| e.to_string()))
        .and_then(|()| conn.flush().map_err(|e| e.to_string()))
}

/// While it lives, restacks the pill on top whenever another window is mapped or restacked over
/// it: a connection of its own with `SubstructureNotify` on the root window and a thread that waits
/// for those events (nothing runs between them), with [`crate::contest`]'s rules for a window that
/// comes straight back on top (an alarm thread has the keeper look again later). Override-redirect
/// windows (menus, tooltips) are left over the pill. Dropping it ends the threads and the
/// connection.
pub struct Keeper {
    connection: Arc<RustConnection>,
    /// An unmapped window of the keeper's own: a client message to it ends the thread or has it
    /// look again ([`STOP`], [`LOOK_AGAIN`]).
    wake: Window,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Keeper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Keeper").field("wake", &self.wake).finish_non_exhaustive()
    }
}

impl Keeper {
    /// Start keeping `window` on top on `$DISPLAY`; `None` without one.
    pub fn start(window: u32) -> Result<Option<Self>, String> {
        Self::start_on(std::env::var_os("DISPLAY").as_deref(), window)
    }

    /// [`Keeper::start`] on an explicit display name (tests pass the server they started).
    pub fn start_on(display: Option<&OsStr>, window: u32) -> Result<Option<Self>, String> {
        let Some(name) = display_name(display) else { return Ok(None) };
        let x11 = |e: &dyn std::fmt::Display| format!("x11: {e}");
        let (conn, screen) = x11rb::connect(Some(name)).map_err(|e| x11(&e))?;
        let root = conn.setup().roots.get(screen).ok_or_else(|| x11(&"no such screen"))?.root;
        let wake = conn.generate_id().map_err(|e| x11(&e))?;
        conn.create_window(0, wake, root, 0, 0, 1, 1, 0, WindowClass::INPUT_ONLY, 0, &CreateWindowAux::new())
            .map_err(|e| x11(&e))?
            .check()
            .map_err(|e| x11(&e))?;
        conn.change_window_attributes(root, &ChangeWindowAttributesAux::new().event_mask(EventMask::SUBSTRUCTURE_NOTIFY))
            .map_err(|e| x11(&e))?
            .check()
            .map_err(|e| x11(&e))?;
        conn.flush().map_err(|e| x11(&e))?;
        let connection = Arc::new(conn);
        let watched = Arc::clone(&connection);
        let thread = std::thread::Builder::new()
            .name("voltip-overlay-x11-keeper".into())
            .spawn(move || watch(&watched, root, window, wake))
            .map_err(|e| e.to_string())?;
        Ok(Some(Self { connection, wake, thread: Some(thread) }))
    }
}

impl Drop for Keeper {
    /// Send the keeper's own window a client message, which ends the thread.
    fn drop(&mut self) {
        let _ = send_to(&self.connection, self.wake, STOP);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// A client message to the keeper's own window, flushed.
fn send_to(conn: &RustConnection, wake: Window, what: u32) -> Result<(), String> {
    let event = ClientMessageEvent::new(32, wake, x11rb::NONE, [what, 0, 0, 0, 0]);
    conn.send_event(false, wake, EventMask::NO_EVENT, event).map_err(|e| e.to_string())?;
    conn.flush().map_err(|e| e.to_string())
}

/// The keeper's thread: wait for the root window's substructure events and restack the pill when
/// a window that is not an override-redirect one covers it.
fn watch(conn: &Arc<RustConnection>, root: Window, pill: Window, wake: Window) {
    let (later, times) = mpsc::channel();
    let alarm = {
        let conn = Arc::clone(conn);
        std::thread::Builder::new().name("voltip-overlay-x11-alarm".into()).spawn(move || alarm(&conn, wake, &times))
    };
    let mut contest = Contest::default();
    loop {
        let event = match conn.wait_for_event() {
            Ok(event) => event,
            Err(e) => {
                tracing::debug!(error = %e, "overlay keeper: the X connection ended");
                break;
            }
        };
        let window = match event {
            Event::ClientMessage(message) if message.window == wake && message.data.as_data32()[0] == LOOK_AGAIN => None,
            Event::ClientMessage(message) if message.window == wake => break,
            Event::MapNotify(e) if !e.override_redirect => Some(e.window),
            Event::ConfigureNotify(e) if !e.override_redirect => Some(e.window),
            Event::CirculateNotify(e) => Some(e.window),
            _ => continue,
        };
        let Some(top) = top_level(conn, root, pill) else { continue };
        if window == Some(top) {
            continue;
        }
        let Some(cover) = covering(conn, root, top) else { continue };
        match contest.raise_over(isize::try_from(cover).unwrap_or(isize::MAX), Instant::now()) {
            Answer::Raise => {
                if let Err(e) = restack(conn, pill) {
                    tracing::debug!(error = %e, "overlay keeper: restack failed");
                }
            }
            Answer::LookAgainIn(wait) => {
                let _ = later.send(Instant::now() + wait);
            }
            Answer::Leave => {}
        }
    }
    drop(later);
    if let Ok(alarm) = alarm {
        let _ = alarm.join();
    }
}

/// The keeper's alarm: once the earliest time it was given has come, it has the keeper look
/// again ([`LOOK_AGAIN`]). It ends when the keeper's thread does.
fn alarm(conn: &RustConnection, wake: Window, times: &mpsc::Receiver<Instant>) {
    let mut next: Option<Instant> = None;
    loop {
        let received = match next {
            None => times.recv().map_err(|_| RecvTimeoutError::Disconnected),
            Some(at) => times.recv_timeout(at.saturating_duration_since(Instant::now())),
        };
        match received {
            Ok(at) => next = Some(next.map_or(at, |next| next.min(at))),
            Err(RecvTimeoutError::Timeout) => {
                next = None;
                if let Err(e) = send_to(conn, wake, LOOK_AGAIN) {
                    tracing::debug!(error = %e, "overlay keeper: the X connection ended");
                    return;
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// The pill's window among the root's children: the pill itself, or the frame a reparenting window
/// manager put it in.
fn top_level(conn: &RustConnection, root: Window, pill: Window) -> Option<Window> {
    let mut window = pill;
    for _ in 0..16 {
        let parent = conn.query_tree(window).ok()?.reply().ok()?.parent;
        if parent == root {
            return Some(window);
        }
        window = parent;
    }
    None
}

/// The lowest window above `top` that covers part of it: viewable, not override-redirect, an
/// input-output window, overlapping. The root's children run from the bottom up.
fn covering(conn: &RustConnection, root: Window, top: Window) -> Option<Window> {
    let children = conn.query_tree(root).ok()?.reply().ok()?.children;
    let at = children.iter().position(|&w| w == top)?;
    let area = conn.get_geometry(top).ok()?.reply().ok()?;
    children[at + 1..].iter().copied().find(|&w| {
        let shown = conn
            .get_window_attributes(w)
            .ok()
            .and_then(|c| c.reply().ok())
            .is_some_and(|a| a.map_state == MapState::VIEWABLE && !a.override_redirect && a.class == WindowClass::INPUT_OUTPUT);
        shown
            && conn.get_geometry(w).ok().and_then(|c| c.reply().ok()).is_some_and(|g| {
                let (ax, ay, bx, by) = (i32::from(area.x), i32::from(area.y), i32::from(g.x), i32::from(g.y));
                ax < bx + i32::from(g.width) && bx < ax + i32::from(area.width) && ay < by + i32::from(g.height) && by < ay + i32::from(area.height)
            })
    })
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
    /// connection is kept. The keeper cannot start there either, and without a display it has
    /// nothing to keep.
    #[test]
    fn an_unreachable_display_is_an_error_and_the_next_raise_connects_again() {
        let stacker = Stacker::default();
        for _ in 0..2 {
            let error = stacker.raise_on(Some(OsStr::new(":97")), 1).expect_err("no X server there");
            assert!(error.starts_with("x11: "), "{error}");
            assert!(stacker.connection.lock().unwrap_or_else(PoisonError::into_inner).is_none());
        }
        assert!(Keeper::start_on(Some(OsStr::new(":97")), 1).is_err());
        assert!(Keeper::start_on(None, 1).expect("no display").is_none());
    }
}
