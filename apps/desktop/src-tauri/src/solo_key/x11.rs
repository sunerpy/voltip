//! X11 / XWayland backend of the lone-key trigger (docs/dictation.md §13.1).
//!
//! XInput 2 raw events selected on the root window report every key and button whichever window
//! has the focus, and a grab by another client does not take them away (XI 2.1). They only
//! observe, so a modifier trigger reaches the focused application as usual. A mouse trigger is
//! also grabbed passively on the root window (`XGrabButton`, any modifiers): the server then hands
//! that button to Voltip alone, and a held back button no longer navigates the browser back.
//! Events from the XTEST devices (Voltip's own paste, xdotool) are not input and are ignored.
//! Under XWayland the server only sees input over X11 windows, like the chord's `XGrabKey`.
//!
//! The loop blocks in `wait_for_event`; stopping sends a client message to an unmapped window of
//! the same connection, which wakes it.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::thread::JoinHandle;

use voltip_core::SoloKey;
use voltip_platform::solo_key::{SoloEdge, SoloInput, SoloTracker, x11_button, x11_keycode};
use x11rb::connection::Connection as _;
use x11rb::protocol::Event;
use x11rb::protocol::xinput::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{
    AtomEnum, ButtonIndex, ClientMessageEvent, ConnectionExt as _, CreateWindowAux, EventMask, GrabMode, ModMask, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;

/// The running watch: a thread blocked on its own X connection.
pub struct Backend {
    conn: Arc<RustConnection>,
    wake: Window,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Backend {
    /// Wake the loop, let it end and close the connection, which releases the grab.
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let wake = ClientMessageEvent::new(32, self.wake, AtomEnum::NONE, [0u32; 5]);
        if let Err(e) = self.conn.send_event(false, self.wake, EventMask::NO_EVENT, wake).map(|_| self.conn.flush()) {
            tracing::warn!(error = %e, "could not wake the lone-key watch");
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Connect, select the raw events (and grab a mouse trigger), then run the loop on a thread of its
/// own. The error says why the watch could not start.
pub fn start(key: SoloKey, tx: Sender<SoloEdge>, synthetic: bool) -> Result<Backend, String> {
    let watch = Watch::open(key, synthetic)?;
    let conn = watch.conn.clone();
    let wake = watch.wake;
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let thread = std::thread::Builder::new()
        .name("voltip-solo-key-x11".into())
        .spawn(move || {
            if let Err(e) = watch.run(&flag, &tx) {
                tracing::warn!(key = %key, error = %e, "lone-key watch ended");
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(Backend { conn, wake, stop, thread: Some(thread) })
}

struct Watch {
    conn: Arc<RustConnection>,
    wake: Window,
    key: SoloKey,
    keycode: Option<u32>,
    button: Option<u32>,
    /// XTEST slave devices: synthetic input, never counted (unless `synthetic`).
    xtest: HashSet<u16>,
    synthetic: bool,
    tracker: SoloTracker,
}

impl Watch {
    fn open(key: SoloKey, synthetic: bool) -> Result<Self, String> {
        let (conn, screen) = x11rb::connect(None).map_err(|e| format!("连不上 X 服务器（{e}）"))?;
        let root = conn.setup().roots.get(screen).ok_or("X 服务器没有这个屏幕")?.root;
        let version = conn.xinput_xi_query_version(2, 2).map_err(|e| e.to_string())?.reply().map_err(|_| "X 服务器没有 XInput 2".to_owned())?;
        if version.major_version < 2 {
            return Err(format!("X 服务器的 XInput 是 {}.{}，需要 2.x", version.major_version, version.minor_version));
        }
        let devices = conn.xinput_xi_query_device(xinput::Device::ALL).map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?;
        let xtest = devices.infos.iter().filter(|d| d.name.windows(5).any(|w| w == b"XTEST")).map(|d| d.deviceid).collect();
        let mask = xinput::XIEventMask::RAW_KEY_PRESS
            | xinput::XIEventMask::RAW_KEY_RELEASE
            | xinput::XIEventMask::RAW_BUTTON_PRESS
            | xinput::XIEventMask::RAW_BUTTON_RELEASE;
        conn.xinput_xi_select_events(root, &[xinput::EventMask { deviceid: xinput::Device::ALL_MASTER.into(), mask: vec![mask] }])
            .map_err(|e| e.to_string())?
            .check()
            .map_err(|e| format!("XInput 2 拒绝了原始事件（{e:?}）"))?;
        let button = x11_button(key);
        if let Some(button) = button {
            conn.grab_button(
                false,
                root,
                EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
                x11rb::NONE,
                x11rb::NONE,
                ButtonIndex::from(button),
                ModMask::ANY,
            )
            .map_err(|e| e.to_string())?
            .check()
            .map_err(|_| "另一个程序已经占用了这个鼠标键".to_owned())?;
        }
        // Never mapped: only the target of the client message that stops the loop.
        let wake = conn.generate_id().map_err(|e| e.to_string())?;
        conn.create_window(0, wake, root, 0, 0, 1, 1, 0, WindowClass::INPUT_ONLY, x11rb::COPY_FROM_PARENT, &CreateWindowAux::new())
            .map_err(|e| e.to_string())?
            .check()
            .map_err(|e| format!("{e:?}"))?;
        conn.flush().map_err(|e| e.to_string())?;
        tracing::info!(key = %key, xinput = %format!("{}.{}", version.major_version, version.minor_version), xtest_devices = ?xtest, "lone-key watch on X11");
        Ok(Self {
            conn: Arc::new(conn),
            wake,
            key,
            keycode: x11_keycode(key).map(u32::from),
            button: button.map(u32::from),
            xtest,
            synthetic,
            tracker: SoloTracker::new(key),
        })
    }

    fn run(mut self, stop: &AtomicBool, tx: &Sender<SoloEdge>) -> Result<(), String> {
        loop {
            let event = self.conn.wait_for_event().map_err(|e| e.to_string())?;
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            if let Some(edge) = self.input(&event).and_then(|input| self.tracker.feed(input))
                && tx.send(edge).is_err()
            {
                return Ok(());
            }
        }
    }

    /// The tracker's view of one event, or nothing for events that do not matter.
    fn input(&self, event: &Event) -> Option<SoloInput> {
        let counted = |source: u16| self.synthetic || !self.xtest.contains(&source);
        match event {
            Event::XinputRawKeyPress(e) if counted(e.sourceid) => {
                Some(if Some(e.detail) == self.keycode { SoloInput::TriggerDown } else { SoloInput::OtherDown })
            }
            Event::XinputRawKeyRelease(e) if counted(e.sourceid) && Some(e.detail) == self.keycode => Some(SoloInput::TriggerUp),
            // Buttons 4–7 are the wheel: scrolling while a modifier is held is no chord.
            Event::XinputRawButtonPress(e) if counted(e.sourceid) => match e.detail {
                d if Some(d) == self.button => Some(SoloInput::TriggerDown),
                4..=7 => None,
                _ => Some(SoloInput::OtherDown),
            },
            Event::XinputRawButtonRelease(e) if counted(e.sourceid) && Some(e.detail) == self.button => Some(SoloInput::TriggerUp),
            // The grab's own events (the tracker drops the duplicates of the raw ones).
            Event::ButtonPress(e) if Some(u32::from(e.detail)) == self.button => Some(SoloInput::TriggerDown),
            Event::ButtonRelease(e) if Some(u32::from(e.detail)) == self.button => Some(SoloInput::TriggerUp),
            Event::Error(e) => {
                tracing::debug!(key = %self.key, error = ?e, "X error on the lone-key connection");
                None
            }
            _ => None,
        }
    }
}

/// Real X server tests (`#[ignore]`d: they need `DISPLAY` and xdotool). Run them under Xvfb:
/// `xvfb-run -a cargo test -p voltip-desktop --lib solo_key -- --ignored`.
#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    fn xdotool(args: &[&str]) {
        let status = std::process::Command::new("xdotool").args(args).status().expect("xdotool on PATH");
        assert!(status.success(), "xdotool {args:?}");
    }

    fn next(rx: &mpsc::Receiver<SoloEdge>) -> Option<SoloEdge> {
        rx.recv_timeout(Duration::from_secs(3)).ok()
    }

    #[test]
    #[ignore = "needs an X server (DISPLAY) and xdotool"]
    fn real_x_server_reports_a_lone_modifier_its_chord_and_a_grabbed_button() {
        // Xvfb resets when its last client leaves; this one stays for the whole test.
        let _keep = x11rb::connect(None).expect("DISPLAY");
        // Synthetic input only here: xdotool goes through XTEST, which the shell ignores.
        let (tx, rx) = mpsc::channel();
        let backend = start(SoloKey::RightCtrl, tx, true).expect("watch");
        xdotool(&["keydown", "Control_R"]);
        assert_eq!(next(&rx), Some(SoloEdge::Press));
        xdotool(&["keyup", "Control_R"]);
        assert_eq!(next(&rx), Some(SoloEdge::Release));
        xdotool(&["keydown", "Control_R", "key", "c", "keyup", "Control_R"]);
        assert_eq!(next(&rx), Some(SoloEdge::Press));
        assert_eq!(next(&rx), Some(SoloEdge::Chorded));
        assert_eq!(rx.recv_timeout(Duration::from_millis(300)).ok(), None, "the chord's release is not reported");
        backend.stop();

        let (tx, rx) = mpsc::channel();
        let backend = start(SoloKey::MouseBack, tx, true).expect("watch");
        xdotool(&["mousedown", "8"]);
        assert_eq!(next(&rx), Some(SoloEdge::Press));
        xdotool(&["key", "a"]);
        xdotool(&["mouseup", "8"]);
        assert_eq!(next(&rx), Some(SoloEdge::Release), "a key during a mouse trigger is no chord");
        backend.stop();

        // Without `synthetic` the same XTEST input is not a trigger at all.
        let (tx, rx) = mpsc::channel();
        let backend = start(SoloKey::RightCtrl, tx, false).expect("watch");
        xdotool(&["keydown", "Control_R", "keyup", "Control_R"]);
        assert_eq!(rx.recv_timeout(Duration::from_millis(500)).ok(), None, "XTEST is not input");
        backend.stop();
    }
}
