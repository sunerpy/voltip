//! Linux foreground application (docs/dictation.md §18.2): X11 and XWayland through `x11rb`. The
//! root window's `_NET_ACTIVE_WINDOW` names the focused client; its `WM_CLASS` gives the id, its
//! `_NET_WM_NAME` (or `WM_NAME`) the title, its `_NET_WM_PID` tells Voltip's own windows apart.
//!
//! Pure Wayland (no `DISPLAY`) has no standard way to ask: the probe answers `None` and the take
//! runs without a scene. Under XWayland only X11 windows are visible — a native Wayland window in
//! front reads as no active window (or, on some compositors, the last X11 window).
//!
//! The connection is opened lazily, kept, and dropped on any error (the next probe reconnects). A
//! probe that finds the previous one still stuck on the socket answers `None` at once instead of
//! queueing behind it: the core stops waiting after 100 ms anyway.

use std::ffi::OsStr;

use parking_lot::Mutex;
use voltip_core::ForegroundApp;
use voltip_platform::foreground::{decode_x11_text, from_wm_class};
use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt as _, Window};
use x11rb::rust_connection::RustConnection;

/// Longest title read, in 32-bit units of the property (4 KiB).
const TITLE_UNITS: u32 = 1024;
/// Longest `WM_CLASS` read, in 32-bit units (1 KiB).
const CLASS_UNITS: u32 = 256;

/// The atoms the probe reads, interned once per connection.
struct Atoms {
    net_active_window: Atom,
    net_wm_pid: Atom,
    net_wm_name: Atom,
    utf8_string: Atom,
}

/// An open display: the connection, its root window and the atoms.
struct Display {
    conn: RustConnection,
    root: Window,
    atoms: Atoms,
}

/// The X11 half of `PlatformProbe`.
#[derive(Default)]
pub struct X11Probe {
    display: Mutex<Option<Display>>,
}

fn x_err(e: impl std::fmt::Display) -> String {
    format!("x11: {e}")
}

impl X11Probe {
    /// The focused application on `$DISPLAY`, `None` without one (pure Wayland, no session).
    pub fn foreground(&self) -> Result<Option<ForegroundApp>, String> {
        self.foreground_on(std::env::var_os("DISPLAY").as_deref())
    }

    /// [`X11Probe::foreground`] on an explicit display name (tests pass `None` / a test server).
    pub fn foreground_on(&self, display: Option<&OsStr>) -> Result<Option<ForegroundApp>, String> {
        let Some(name) = display.and_then(OsStr::to_str).filter(|d| !d.is_empty()) else { return Ok(None) };
        // A previous probe still blocked on the server holds the lock: do not pile up behind it.
        let Some(mut guard) = self.display.try_lock() else {
            tracing::debug!("x11 probe busy (the previous one has not returned); no answer");
            return Ok(None);
        };
        if guard.is_none() {
            *guard = Some(Self::open(name)?);
        }
        let Some(display) = guard.as_ref() else { return Ok(None) };
        let answer = Self::query(display);
        if answer.is_err() {
            // A broken connection is dropped; the next probe reconnects.
            *guard = None;
        }
        answer
    }

    fn open(name: &str) -> Result<Display, String> {
        let (conn, screen) = x11rb::connect(Some(name)).map_err(x_err)?;
        let root = conn.setup().roots.get(screen).map(|s| s.root).ok_or_else(|| x_err(format!("no screen {screen}")))?;
        let intern = |atom: &[u8]| -> Result<Atom, String> { Ok(conn.intern_atom(false, atom).map_err(x_err)?.reply().map_err(x_err)?.atom) };
        let atoms = Atoms {
            net_active_window: intern(b"_NET_ACTIVE_WINDOW")?,
            net_wm_pid: intern(b"_NET_WM_PID")?,
            net_wm_name: intern(b"_NET_WM_NAME")?,
            utf8_string: intern(b"UTF8_STRING")?,
        };
        Ok(Display { conn, root, atoms })
    }

    fn property(
        display: &Display,
        window: Window,
        property: impl Into<Atom>,
        kind: impl Into<Atom>,
        units: u32,
    ) -> Result<x11rb::protocol::xproto::GetPropertyReply, String> {
        display.conn.get_property(false, window, property, kind, 0, units).map_err(x_err)?.reply().map_err(x_err)
    }

    fn query(display: &Display) -> Result<Option<ForegroundApp>, String> {
        let active = Self::property(display, display.root, display.atoms.net_active_window, AtomEnum::WINDOW, 1)?;
        let Some(window) = active.value32().and_then(|mut v| v.next()).filter(|w| *w != x11rb::NONE) else { return Ok(None) };
        let pid = Self::property(display, window, display.atoms.net_wm_pid, AtomEnum::CARDINAL, 1)?.value32().and_then(|mut v| v.next());
        if pid == Some(std::process::id()) {
            // Voltip's own window (the dictation came from its UI): not a target application.
            return Ok(None);
        }
        let class = Self::property(display, window, AtomEnum::WM_CLASS, AtomEnum::STRING, CLASS_UNITS)?;
        let Some(identity) = from_wm_class(&class.value) else { return Ok(None) };
        let net_name = Self::property(display, window, display.atoms.net_wm_name, display.atoms.utf8_string, TITLE_UNITS)?;
        let title_bytes =
            if net_name.value.is_empty() { Self::property(display, window, AtomEnum::WM_NAME, AtomEnum::ANY, TITLE_UNITS)?.value } else { net_name.value };
        let title = Some(decode_x11_text(&title_bytes)).filter(|t| !t.trim().is_empty());
        Ok(Some(ForegroundApp { app_id: identity.app_id, name: identity.name, title }))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use x11rb::protocol::xproto::{CreateWindowAux, PropMode, WindowClass};
    use x11rb::wrapper::ConnectionExt as _;

    /// Without a display (pure Wayland, no session) there is nothing to ask.
    #[test]
    fn no_display_means_no_answer() {
        let probe = X11Probe::default();
        assert_eq!(probe.foreground_on(None), Ok(None));
        assert_eq!(probe.foreground_on(Some(OsStr::new(""))), Ok(None));
        let err = probe.foreground_on(Some(OsStr::new(":987"))).unwrap_err();
        assert!(err.starts_with("x11: "), "{err}");
        assert!(probe.display.lock().is_none(), "a failed open keeps nothing");
    }

    /// The real probe against an X server (`xvfb-run -a cargo test -p voltip-desktop --lib
    /// platform::linux -- --ignored`): a window with `WM_CLASS` / `_NET_WM_NAME` made active on the
    /// root is named; our own pid on it, or no active window, is no answer.
    #[test]
    #[ignore = "needs an X server: xvfb-run -a cargo test -p voltip-desktop --lib platform::linux -- --ignored"]
    fn x11_probe_names_the_active_window() {
        let display = std::env::var_os("DISPLAY").expect("DISPLAY (run under xvfb-run)");
        let (conn, screen) = x11rb::connect(display.to_str()).unwrap();
        let screen = &conn.setup().roots[screen];
        let window = conn.generate_id().unwrap();
        conn.create_window(0, window, screen.root, 0, 0, 10, 10, 0, WindowClass::INPUT_OUTPUT, 0, &CreateWindowAux::new()).unwrap();
        let atom = |name: &[u8]| conn.intern_atom(false, name).unwrap().reply().unwrap().atom;
        let (active, pid, net_name, utf8) = (atom(b"_NET_ACTIVE_WINDOW"), atom(b"_NET_WM_PID"), atom(b"_NET_WM_NAME"), atom(b"UTF8_STRING"));
        conn.change_property8(PropMode::REPLACE, window, AtomEnum::WM_CLASS, AtomEnum::STRING, b"voltip-probe\0VoltipProbe\0").unwrap();
        conn.change_property8(PropMode::REPLACE, window, net_name, utf8, "报告.docx — 草稿".as_bytes()).unwrap();
        conn.change_property32(PropMode::REPLACE, window, pid, AtomEnum::CARDINAL, &[1]).unwrap();
        conn.change_property32(PropMode::REPLACE, screen.root, active, AtomEnum::WINDOW, &[window]).unwrap();
        conn.flush().unwrap();
        let probe = X11Probe::default();
        let app = probe.foreground().unwrap().expect("the active window is named");
        assert_eq!((app.app_id.as_str(), app.name.as_str(), app.title.as_deref()), ("voltipprobe", "VoltipProbe", Some("报告.docx — 草稿")));
        // WM_NAME stands in for a missing _NET_WM_NAME.
        conn.delete_property(window, net_name).unwrap();
        conn.change_property8(PropMode::REPLACE, window, AtomEnum::WM_NAME, AtomEnum::STRING, b"legacy title").unwrap();
        conn.flush().unwrap();
        assert_eq!(probe.foreground().unwrap().unwrap().title.as_deref(), Some("legacy title"));
        // Our own window is not a target.
        conn.change_property32(PropMode::REPLACE, window, pid, AtomEnum::CARDINAL, &[std::process::id()]).unwrap();
        conn.flush().unwrap();
        assert_eq!(probe.foreground(), Ok(None));
        // No active window.
        conn.change_property32(PropMode::REPLACE, screen.root, active, AtomEnum::WINDOW, &[x11rb::NONE]).unwrap();
        conn.flush().unwrap();
        assert_eq!(probe.foreground(), Ok(None));
        conn.destroy_window(window).unwrap();
        conn.flush().unwrap();
    }
}
