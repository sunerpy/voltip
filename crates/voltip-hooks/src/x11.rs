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
//!
//! Keeping the watch (docs/dictation.md §29): when the connection breaks (the server restarted, a
//! client killed ours), [`SoloEdge::Lost`] says so and the thread connects again after
//! [`retry_after`]: 1, 2, 4 … 60 s. The first connection that works reports
//! [`SoloEdge::Recovered`]. [`is_down`] reads key and button state over a connection of its own.

use std::collections::HashSet;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

use voltip_platform::held_key::{self, HeldKey, X11Key};
use voltip_platform::solo_key::SoloKey;
use voltip_platform::solo_key::{SoloEdge, SoloInput, SoloLoss, SoloTracker, x11_button, x11_keycode};
use x11rb::connection::Connection as _;
use x11rb::protocol::Event;
use x11rb::protocol::xinput::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{
    AtomEnum, ButtonIndex, ClientMessageEvent, ConnectionExt as _, CreateWindowAux, EventMask, GrabMode, ModMask, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;

use crate::watch::retry_after;

/// What the watch's thread and [`Backend`] share, under one lock: a stop and the publication of
/// a new connection never miss each other.
#[derive(Default)]
struct Shared {
    state: Mutex<SharedState>,
    /// Cuts the wait between two connection attempts short.
    wake: Condvar,
}

#[derive(Default)]
struct SharedState {
    stop: bool,
    /// The live connection and the window whose client message wakes its loop.
    live: Option<(Arc<RustConnection>, Window)>,
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, SharedState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn stopping(&self) -> bool {
        self.state().stop
    }

    /// Wait `wait`, or until asked to stop; whether to stop.
    fn wait(&self, wait: std::time::Duration) -> bool {
        let state = self.state();
        let (state, _) = self.wake.wait_timeout_while(state, wait, |state| !state.stop).unwrap_or_else(PoisonError::into_inner);
        state.stop
    }
}

/// The running watch: a thread blocked on its own X connection.
pub struct Backend {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl Backend {
    /// Wake the loop, let it end and close the connection, which releases the grab.
    pub fn stop(mut self) {
        let live = {
            let mut state = self.shared.state();
            state.stop = true;
            state.live.clone()
        };
        self.shared.wake.notify_all();
        if let Some((conn, wake)) = live {
            let event = ClientMessageEvent::new(32, wake, AtomEnum::NONE, [0u32; 5]);
            if let Err(e) = conn.send_event(false, wake, EventMask::NO_EVENT, event).map(|_| conn.flush()) {
                tracing::warn!(error = %e, "could not wake the lone-key watch");
            }
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }

    /// Nothing to do: the thread notices a lost connection itself.
    pub fn refresh(&self) {}
}

/// Connect, select the raw events (and grab a mouse trigger), then run the loop on a thread of its
/// own. The error says why the watch could not start.
pub fn start(key: SoloKey, tx: Sender<SoloEdge>, synthetic: bool) -> Result<Backend, String> {
    let watch = Watch::open(key, synthetic)?;
    let shared = Arc::new(Shared::default());
    shared.state().live = Some((watch.conn.clone(), watch.wake));
    let thread_shared = shared.clone();
    let thread = std::thread::Builder::new().name("voltip-solo-key-x11".into()).spawn(move || run(watch, &thread_shared, &tx)).map_err(|e| e.to_string())?;
    Ok(Backend { shared, thread: Some(thread) })
}

/// Watch until stopped; connect again whenever the connection breaks.
fn run(mut watch: Watch, shared: &Shared, tx: &Sender<SoloEdge>) {
    let (key, synthetic) = (watch.key, watch.synthetic);
    loop {
        match watch.run(shared, tx) {
            Ok(()) => return,
            Err(e) if shared.stopping() => {
                tracing::debug!(key = %key, error = %e, "lone-key watch ended while stopping");
                return;
            }
            Err(e) => tracing::warn!(key = %key, error = %e, "the lone-key watch lost its X connection; connecting again"),
        }
        shared.state().live = None;
        if tx.send(SoloEdge::Lost(SoloLoss::Display)).is_err() {
            return;
        }
        let mut attempt = 0;
        watch = loop {
            if shared.wait(retry_after(attempt)) {
                return;
            }
            attempt = attempt.saturating_add(1);
            match Watch::open(key, synthetic) {
                Ok(mut next) => {
                    // Held across the gap: its release is not a press of its own.
                    next.tracker = SoloTracker::resumed(key, is_down(&HeldKey::Solo(key)).unwrap_or(false));
                    let mut state = shared.state();
                    if state.stop {
                        return;
                    }
                    state.live = Some((next.conn.clone(), next.wake));
                    break next;
                }
                Err(e) => tracing::debug!(key = %key, attempt, error = %e, "the X server is still out of reach"),
            }
        };
        tracing::warn!(key = %key, attempts = attempt, "lone-key watch connected again");
        if tx.send(SoloEdge::Recovered).is_err() {
            return;
        }
    }
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

    /// Feed the tracker until asked to stop (`Ok`) or the connection breaks (`Err`).
    fn run(&mut self, shared: &Shared, tx: &Sender<SoloEdge>) -> Result<(), String> {
        loop {
            let event = self.conn.wait_for_event().map_err(|e| e.to_string())?;
            if shared.stopping() {
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

/// The connection the key-state queries use: opened on first use and dropped after an error (the
/// next query connects again). Never the watch's, which waits for events on its own thread and
/// comes and goes with the server.
static QUERIES: Mutex<Option<Queries>> = Mutex::new(None);

struct Queries {
    conn: RustConnection,
    root: Window,
    min_keycode: u8,
    max_keycode: u8,
}

impl Queries {
    fn open() -> Result<Self, String> {
        let (conn, screen) = x11rb::connect(None).map_err(|e| e.to_string())?;
        let setup = conn.setup();
        let root = setup.roots.get(screen).ok_or("no such screen")?.root;
        let (min_keycode, max_keycode) = (setup.min_keycode, setup.max_keycode);
        // XI 2 requests need the version announced first.
        conn.xinput_xi_query_version(2, 2).map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?;
        Ok(Self { conn, root, min_keycode, max_keycode })
    }

    fn read(&self, key: X11Key) -> Result<bool, String> {
        match key {
            X11Key::Keycode(code) => Ok(keymap_has(&self.keymap()?, code)),
            X11Key::Keysym(sym) => {
                let count = u8::try_from(u16::from(self.max_keycode).saturating_sub(u16::from(self.min_keycode)) + 1).unwrap_or(u8::MAX);
                let mapping = self.conn.get_keyboard_mapping(self.min_keycode, count).map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?;
                let codes = keycodes_for(self.min_keycode, mapping.keysyms_per_keycode, &mapping.keysyms, sym);
                let keymap = self.keymap()?;
                Ok(codes.into_iter().any(|code| keymap_has(&keymap, code)))
            }
            X11Key::Button(button) => {
                let pointer = self.conn.xinput_xi_get_client_pointer(x11rb::NONE).map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?.deviceid;
                let reply = self.conn.xinput_xi_query_pointer(self.root, pointer).map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?;
                Ok(buttons_have(&reply.buttons, button))
            }
        }
    }

    fn keymap(&self) -> Result<[u8; 32], String> {
        Ok(self.conn.query_keymap().map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?.keys)
    }
}

/// Whether `key` is physically down (docs/dictation.md §29): `QueryKeymap` for a key (a chord's by
/// the keycodes its keysym has on the current layout), the master pointer's XI 2 button state for
/// a button (the core `QueryPointer` has no buttons 8 and 9). `None` without an X server. Under
/// XWayland the server only knows the keys pressed over X11 windows, so the shell reads this on a
/// plain X11 session alone.
pub fn is_down(key: &HeldKey) -> Option<bool> {
    let target = held_key::x11_key(key)?;
    let mut queries = QUERIES.lock().unwrap_or_else(PoisonError::into_inner);
    if queries.is_none() {
        *queries = Queries::open().map_err(|e| tracing::debug!(error = %e, "no X connection for key-state queries")).ok();
    }
    match queries.as_ref()?.read(target) {
        Ok(down) => Some(down),
        Err(e) => {
            tracing::debug!(error = %e, "key-state query failed; connecting again next time");
            *queries = None;
            None
        }
    }
}

/// Whether `code` is down in a `QueryKeymap` bit vector.
fn keymap_has(keymap: &[u8; 32], code: u8) -> bool {
    keymap[usize::from(code / 8)] & (1 << (code % 8)) != 0
}

/// Whether `button` is down in an XI 2 button mask.
fn buttons_have(mask: &[u32], button: u8) -> bool {
    mask.get(usize::from(button / 32)).is_some_and(|word| word & (1 << (button % 32)) != 0)
}

/// The keycodes whose row of a `GetKeyboardMapping` reply (from `first`, `per` keysyms a row)
/// holds `sym`.
fn keycodes_for(first: u8, per: u8, keysyms: &[u32], sym: u32) -> Vec<u8> {
    if per == 0 {
        return Vec::new();
    }
    keysyms
        .chunks(usize::from(per))
        .enumerate()
        .filter(|(_, row)| row.contains(&sym))
        .filter_map(|(row, _)| u8::try_from(usize::from(first) + row).ok())
        .collect()
}

/// Real X server tests (`#[ignore]`d: they need `DISPLAY` and xdotool). Run them under Xvfb:
/// `xvfb-run -a cargo test -p voltip-hooks --lib -- --ignored --test-threads=1`.
#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn key_and_button_state_is_read_from_the_bits_the_server_sends() {
        let mut keymap = [0u8; 32];
        keymap[13] = 0b0000_0010; // keycode 105, Right Control under evdev
        assert!(keymap_has(&keymap, 105));
        assert!(!keymap_has(&keymap, 104));
        assert!(!keymap_has(&keymap, 255));
        // Button 8 (back) is bit 8 of the first word; button 33 would be in the second.
        assert!(buttons_have(&[1 << 8], 8));
        assert!(!buttons_have(&[1 << 8], 9));
        assert!(!buttons_have(&[1 << 8], 33), "a word the reply does not have is up");
        assert!(buttons_have(&[0, 1 << 1], 33));
        // `a A a A` on keycode 38, `space` on 65 (first keycode 8, 4 keysyms a row).
        let mut keysyms = vec![0u32; 4 * 60];
        keysyms[(38 - 8) * 4..(38 - 8) * 4 + 4].copy_from_slice(&[0x61, 0x41, 0x61, 0x41]);
        keysyms[(65 - 8) * 4] = 0x20;
        assert_eq!(keycodes_for(8, 4, &keysyms, 0x41), [38]);
        assert_eq!(keycodes_for(8, 4, &keysyms, 0x20), [65]);
        assert_eq!(keycodes_for(8, 4, &keysyms, 0xFFBE), Vec::<u8>::new());
        assert_eq!(keycodes_for(8, 0, &keysyms, 0x41), Vec::<u8>::new());
    }

    fn xdotool(args: &[&str]) {
        let status = std::process::Command::new("xdotool").args(args).status().expect("xdotool on PATH");
        assert!(status.success(), "xdotool {args:?}");
    }

    fn next(rx: &mpsc::Receiver<SoloEdge>) -> Option<SoloEdge> {
        rx.recv_timeout(Duration::from_secs(3)).ok()
    }

    /// Wait (at most 5 s) for `probe` to hold.
    fn wait_until(what: &str, mut probe: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !probe() {
            assert!(Instant::now() < deadline, "{what} within 5 s");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    impl Backend {
        /// The live connection's wake window: a resource that names the watch's client.
        fn wake_window(&self) -> Option<Window> {
            self.shared.state().live.as_ref().map(|(_, wake)| *wake)
        }
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

    /// docs/dictation.md §29: the X connection breaks (here another client kills it, as a
    /// restarting server would close it); the watch says so, connects again and reports edges.
    #[test]
    #[ignore = "needs an X server (DISPLAY) and xdotool"]
    fn regression_a_lost_x_connection_comes_back() {
        let (keep, _) = x11rb::connect(None).expect("DISPLAY");
        let (tx, rx) = mpsc::channel();
        let backend = start(SoloKey::RightCtrl, tx, true).expect("watch");
        let wake = backend.wake_window().expect("connected");
        keep.kill_client(wake).expect("KillClient").check().expect("KillClient");
        assert_eq!(next(&rx), Some(SoloEdge::Lost(SoloLoss::Display)));
        assert_eq!(next(&rx), Some(SoloEdge::Recovered), "connected again after a second");
        xdotool(&["keydown", "Control_R"]);
        assert_eq!(next(&rx), Some(SoloEdge::Press));
        xdotool(&["keyup", "Control_R"]);
        assert_eq!(next(&rx), Some(SoloEdge::Release));
        backend.stop();
    }

    /// docs/dictation.md §29: the physical state a hold-to-talk take is checked against: a solo
    /// modifier by keycode, a chord key by keysym, and the grabbed back button through XI 2.
    #[test]
    #[ignore = "needs an X server (DISPLAY) and xdotool"]
    fn real_held_keys_read_through_the_server() {
        let _keep = x11rb::connect(None).expect("DISPLAY");
        let ctrl = HeldKey::Solo(SoloKey::RightCtrl);
        xdotool(&["keydown", "Control_R"]);
        wait_until("Right Control reads as down", || is_down(&ctrl) == Some(true));
        xdotool(&["keyup", "Control_R"]);
        wait_until("Right Control reads as up", || is_down(&ctrl) == Some(false));
        let space = HeldKey::Chord("Space".to_owned());
        xdotool(&["keydown", "space"]);
        wait_until("Space reads as down", || is_down(&space) == Some(true));
        xdotool(&["keyup", "space"]);
        wait_until("Space reads as up", || is_down(&space) == Some(false));
        let (tx, rx) = mpsc::channel();
        let backend = start(SoloKey::MouseBack, tx, true).expect("watch");
        let back = HeldKey::Solo(SoloKey::MouseBack);
        xdotool(&["mousedown", "8"]);
        assert_eq!(next(&rx), Some(SoloEdge::Press));
        wait_until("the grabbed back button reads as down", || is_down(&back) == Some(true));
        xdotool(&["mouseup", "8"]);
        assert_eq!(next(&rx), Some(SoloEdge::Release));
        wait_until("the back button reads as up", || is_down(&back) == Some(false));
        backend.stop();
    }
}
