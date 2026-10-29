//! macOS backend of the lone-key trigger (docs/dictation.md §13.1): an active `CGEventTap` on the
//! session, on a thread that runs its own run loop.
//!
//! An active tap (it can drop events) needs the Accessibility permission, which the paste already
//! needs; a listen-only tap would need Input Monitoring on top. The tap drops the mouse trigger
//! (the back button no longer navigates) and passes every key on. A modifier's state comes from
//! the event's device-dependent flag (`NX_DEVICER*KEYMASK`, `kCGEventFlagMaskSecondaryFn`), not
//! from counting events. What Voltip posts itself carries enigo's `EVENT_SOURCE_USER_DATA` marker
//! and is ignored.
//!
//! Keeping the tap (docs/dictation.md §29): the system disables a tap whose callback was slow, and
//! a tap can end up disabled without its callback hearing of it. The thread checks every
//! [`WATCH_EVERY`], and at once when [`Backend::refresh`] asks (the shell does after waking and
//! when the session becomes active again); a disabled tap is installed again and reported as
//! [`SoloEdge::Recovered`]. When that fails (the Accessibility permission was withdrawn),
//! [`SoloEdge::Lost`] says so once and the next attempt follows after [`retry_after`]: 1, 2, 4 …
//! 60 s, until one works.
//!
//! `core-graphics` wraps neither `CGEventTapIsEnabled` nor the key-state queries, and its scoped
//! tap constructor hides the tap from the run loop that checks it: those calls are made here,
//! the one module besides the Windows hooks where this crate allows unsafe code.

#![allow(unsafe_code)]

use std::cell::{Cell, RefCell};
use std::sync::mpsc::{self, Sender, SyncSender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use core_foundation::base::TCFType as _;
use core_foundation::mach_port::CFMachPortRef;
use core_foundation::runloop::{CFRunLoop, CFRunLoopRunResult, kCFRunLoopCommonModes, kCFRunLoopDefaultMode};
use core_graphics::event::{CGEvent, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType, CallbackResult, EventField};
use voltip_platform::held_key::{self, HeldKey, MacosKey};
use voltip_platform::solo_key::SoloKey;
use voltip_platform::solo_key::{SoloEdge, SoloInput, SoloLoss, SoloTracker, macos_button, macos_modifier};

use crate::watch::retry_after;

/// The `EVENT_SOURCE_USER_DATA` enigo stamps on everything it posts (`enigo::EVENT_MARKER`):
/// Voltip's own paste and copy chords.
const ENIGO_MARKER: i64 = 100;

/// How often the thread checks that its tap is still enabled.
const WATCH_EVERY: Duration = Duration::from_secs(2);

/// What the user has to allow when the tap cannot be installed.
const ACCESSIBILITY: &str = "需要在「系统设置 › 隐私与安全性 › 辅助功能」里允许 Voltip";

/// `kCGEventSourceStateHIDSystemState`: the hardware's key and button state, before any tap.
const HID_SYSTEM_STATE: i32 = 1;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapIsEnabled(tap: CFMachPortRef) -> bool;
    #[cfg(test)]
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    safe fn CGEventSourceKeyState(state: i32, key: u16) -> bool;
    safe fn CGEventSourceButtonState(state: i32, button: u32) -> bool;
}

/// What [`Backend`] asks of the tap's thread.
#[derive(Default)]
struct Asks {
    stop: bool,
    refresh: bool,
    /// Disable the tap the way the system does (the tests).
    #[cfg(test)]
    disable: bool,
}

#[derive(Default)]
struct Control {
    asks: Mutex<Asks>,
    /// Cuts the wait between two attempts short.
    wake: Condvar,
}

impl Control {
    fn asks(&self) -> MutexGuard<'_, Asks> {
        self.asks.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn ask(&self, set: impl FnOnce(&mut Asks)) {
        set(&mut self.asks());
        self.wake.notify_all();
    }

    /// Wait `wait`, or until asked to stop or to check; whether to stop.
    fn wait(&self, wait: Duration) -> bool {
        let asks = self.asks();
        let (mut asks, _) = self.wake.wait_timeout_while(asks, wait, |asks| !asks.stop && !asks.refresh).unwrap_or_else(PoisonError::into_inner);
        asks.refresh = false;
        asks.stop
    }
}

/// The tap's thread and its run loop.
pub struct Backend {
    run_loop: CFRunLoop,
    control: Arc<Control>,
    thread: Option<JoinHandle<()>>,
}

impl Backend {
    /// Stop the run loop, which removes the tap.
    pub fn stop(mut self) {
        self.control.ask(|asks| asks.stop = true);
        self.run_loop.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }

    /// Check the tap now: a disabled one is installed again, and a pending attempt to bring one
    /// back is made at once. The shell asks after waking and when the session becomes active.
    pub fn refresh(&self) {
        self.control.ask(|asks| asks.refresh = true);
        self.run_loop.stop();
    }
}

/// Install the tap for `key` on a new thread; the error says what the user has to allow.
pub fn start(key: SoloKey, tx: Sender<SoloEdge>, synthetic: bool) -> Result<Backend, String> {
    let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<CFRunLoop, String>>(1);
    let control = Arc::new(Control::default());
    let shared = control.clone();
    let thread =
        std::thread::Builder::new().name("voltip-solo-key-mac".into()).spawn(move || run(key, &tx, synthetic, &shared, ready_tx)).map_err(|e| e.to_string())?;
    match ready_rx.recv_timeout(super::START_TIMEOUT) {
        Ok(Ok(run_loop)) => Ok(Backend { run_loop, control, thread: Some(thread) }),
        Ok(Err(e)) => {
            let _ = thread.join();
            Err(e)
        }
        Err(_) => Err("安装事件监听超时".to_owned()),
    }
}

/// How one tap's run ended.
enum Ended {
    Stopped,
    /// The tap is disabled or gone: install another.
    Disabled,
}

fn run(key: SoloKey, tx: &Sender<SoloEdge>, synthetic: bool, control: &Control, ready: SyncSender<Result<CFRunLoop, String>>) {
    let mut ready = Some(ready);
    let tracker = RefCell::new(SoloTracker::new(key));
    // Attempts since the tap last worked, and whether the user has been told it is gone.
    let mut attempt = 0;
    let mut lost = false;
    loop {
        let installed = || {
            if let Some(ready) = ready.take() {
                let _ = ready.send(Ok(CFRunLoop::get_current()));
                tracing::info!(key = %key, "lone-key event tap installed");
            } else {
                // Held across the gap: its release is not a press of its own.
                let held = is_down(&HeldKey::Solo(key)).unwrap_or(false);
                *tracker.borrow_mut() = SoloTracker::resumed(key, held);
                tracing::warn!(key = %key, held, "lone-key event tap put back");
                let _ = tx.send(SoloEdge::Recovered);
            }
        };
        match run_tap(key, synthetic, &tracker, tx, control, installed) {
            Ok(Ended::Stopped) => return,
            Ok(Ended::Disabled) => {
                attempt = 0;
                lost = false;
                tracing::warn!(key = %key, "the lone-key event tap was disabled; installing it again");
            }
            Err(()) => {
                if let Some(ready) = ready.take() {
                    let _ = ready.send(Err(ACCESSIBILITY.to_owned()));
                    return;
                }
                if !lost {
                    lost = true;
                    tracing::warn!(key = %key, "the lone-key event tap could not be installed again; retrying");
                    let _ = tx.send(SoloEdge::Lost(SoloLoss::Accessibility));
                }
                let wait = retry_after(attempt);
                attempt = attempt.saturating_add(1);
                if control.wait(wait) {
                    return;
                }
            }
        }
    }
}

/// Install a tap, call `installed`, and run the run loop until asked to stop or until the tap is
/// disabled; `Err` when the system refused the tap (no Accessibility permission).
fn run_tap(
    key: SoloKey,
    synthetic: bool,
    tracker: &RefCell<SoloTracker>,
    tx: &Sender<SoloEdge>,
    control: &Control,
    installed: impl FnOnce(),
) -> Result<Ended, ()> {
    let modifier = macos_modifier(key);
    let button = macos_button(key);
    let disabled = Cell::new(false);
    let events = vec![
        CGEventType::FlagsChanged,
        CGEventType::KeyDown,
        CGEventType::OtherMouseDown,
        CGEventType::OtherMouseUp,
        CGEventType::OtherMouseDragged,
        CGEventType::LeftMouseDown,
        CGEventType::RightMouseDown,
    ];
    // SAFETY: `tap` is dropped before this function returns, so before everything its callback
    // borrows, and it is only added to this thread's run loop: the callback runs on this thread,
    // while `tap` lives.
    let tap = unsafe {
        CGEventTap::new_unchecked(
            CGEventTapLocation::Session,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::Default,
            events,
            |_proxy, kind, event| {
                if matches!(kind, CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput) {
                    disabled.set(true);
                    CFRunLoop::get_current().stop();
                    return CallbackResult::Keep;
                }
                if !synthetic && event.get_integer_value_field(EventField::EVENT_SOURCE_USER_DATA) == ENIGO_MARKER {
                    return CallbackResult::Keep;
                }
                let (input, drop) = classify(kind, event, modifier, button);
                if let Some(input) = input
                    && let Some(edge) = tracker.borrow_mut().feed(input)
                {
                    let _ = tx.send(edge);
                }
                if drop { CallbackResult::Drop } else { CallbackResult::Keep }
            },
        )
    }?;
    let source = tap.mach_port().create_runloop_source(0)?;
    let run_loop = CFRunLoop::get_current();
    // SAFETY: reading an immutable CoreFoundation constant.
    let common = unsafe { kCFRunLoopCommonModes };
    run_loop.add_source(&source, common);
    tap.enable();
    installed();
    let ended = loop {
        // SAFETY: as above.
        let result = CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, WATCH_EVERY, false);
        {
            let mut asks = control.asks();
            if asks.stop {
                break Ended::Stopped;
            }
            // A refresh asks for the check below, nothing more.
            asks.refresh = false;
            #[cfg(test)]
            if std::mem::take(&mut asks.disable) {
                // SAFETY: the tap's own port, valid while `tap` lives.
                unsafe { CGEventTapEnable(tap.mach_port().as_concrete_TypeRef(), false) };
                continue;
            }
        }
        // SAFETY: as above.
        let enabled = unsafe { CGEventTapIsEnabled(tap.mach_port().as_concrete_TypeRef()) };
        // A run loop whose last source went away returns at once: the tap's port is gone.
        if disabled.get() || !enabled || result == CFRunLoopRunResult::Finished {
            break Ended::Disabled;
        }
    };
    run_loop.remove_source(&source, common);
    drop(tap);
    Ok(ended)
}

/// The tracker's view of one event, and whether the tap drops it (the mouse trigger only).
fn classify(kind: CGEventType, event: &CGEvent, modifier: Option<(u16, u64)>, button: Option<i64>) -> (Option<SoloInput>, bool) {
    let is_trigger_button = || button.is_some() && Some(event.get_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER)) == button;
    match kind {
        CGEventType::FlagsChanged => {
            let keycode = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE);
            match modifier {
                Some((code, flag)) if keycode == i64::from(code) => {
                    let down = event.get_flags().bits() & flag != 0;
                    (Some(if down { SoloInput::TriggerDown } else { SoloInput::TriggerUp }), false)
                }
                // Another modifier going down is no chord by itself; the key that follows is.
                _ => (None, false),
            }
        }
        CGEventType::KeyDown | CGEventType::LeftMouseDown | CGEventType::RightMouseDown => (Some(SoloInput::OtherDown), false),
        CGEventType::OtherMouseDown if is_trigger_button() => (Some(SoloInput::TriggerDown), true),
        CGEventType::OtherMouseDown => (Some(SoloInput::OtherDown), false),
        CGEventType::OtherMouseUp if is_trigger_button() => (Some(SoloInput::TriggerUp), true),
        CGEventType::OtherMouseDragged if is_trigger_button() => (None, true),
        _ => (None, false),
    }
}

/// Whether `key` is physically down (docs/dictation.md §29): the HID system's key and button
/// state, which no event tap changes (the tap that drops the mouse trigger included).
pub fn is_down(key: &HeldKey) -> Option<bool> {
    Some(match held_key::macos_key(key)? {
        MacosKey::Key(code) => CGEventSourceKeyState(HID_SYSTEM_STATE, code),
        MacosKey::Button(button) => CGEventSourceButtonState(HID_SYSTEM_STATE, button),
    })
}

/// Real event-tap tests (`#[ignore]`d: they need a login session where the test binary has the
/// Accessibility permission). CI runs them in the `macos` job; by hand:
/// `cargo test -p voltip-hooks --lib -- --ignored --test-threads=1 --nocapture`.
#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use core_graphics::event::{CGEventFlags, CGMouseButton};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
    use core_graphics::geometry::CGPoint;

    use super::*;

    /// One tap at a time: two would see each other's events.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn serial() -> MutexGuard<'static, ()> {
        SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn source() -> CGEventSource {
        CGEventSource::new(CGEventSourceStateID::HIDSystemState).expect("event source")
    }

    /// Right Control going down or up the way the window server reports a modifier: a
    /// `flagsChanged` event with the key code and, while it is down, its device-dependent flag.
    fn right_ctrl(down: bool) {
        let event = CGEvent::new_keyboard_event(source(), 0x3E, down).expect("event");
        event.set_type(CGEventType::FlagsChanged);
        event.set_flags(if down { CGEventFlags::CGEventFlagControl | CGEventFlags::from_bits_retain(0x2000) } else { CGEventFlags::CGEventFlagNull });
        event.post(CGEventTapLocation::HID);
    }

    /// `kVK_F13`: a key nothing on the runner reacts to (Control + C would interrupt the process
    /// in a focused terminal).
    const F13: u16 = 0x69;

    fn f13(down: bool) {
        CGEvent::new_keyboard_event(source(), F13, down).expect("event").post(CGEventTapLocation::HID);
    }

    fn other_key() {
        f13(true);
        f13(false);
    }

    /// The back button (button number 3) down or up.
    fn back_button(down: bool) {
        let kind = if down { CGEventType::OtherMouseDown } else { CGEventType::OtherMouseUp };
        let event = CGEvent::new_mouse_event(source(), kind, CGPoint::new(10.0, 10.0), CGMouseButton::Center).expect("event");
        event.set_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER, 3);
        event.post(CGEventTapLocation::HID);
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
        /// Disable the tap the way the system does, without a word to the thread.
        fn disable_tap(&self) {
            self.control.ask(|asks| asks.disable = true);
            self.run_loop.stop();
        }
    }

    #[test]
    #[ignore = "needs a macOS login session where this test binary has the Accessibility permission"]
    fn real_tap_reports_a_lone_modifier_its_chord_and_a_side_button() {
        let _serial = serial();
        let (tx, rx) = mpsc::channel();
        let backend = start(SoloKey::RightCtrl, tx, true).expect("event tap (Accessibility)");
        right_ctrl(true);
        assert_eq!(next(&rx), Some(SoloEdge::Press));
        right_ctrl(false);
        assert_eq!(next(&rx), Some(SoloEdge::Release));
        // Right Control + another key: pressed, then chorded; its release is not reported.
        right_ctrl(true);
        other_key();
        right_ctrl(false);
        assert_eq!(next(&rx), Some(SoloEdge::Press));
        assert_eq!(next(&rx), Some(SoloEdge::Chorded));
        assert_eq!(rx.recv_timeout(Duration::from_millis(300)).ok(), None);
        backend.stop();

        let (tx, rx) = mpsc::channel();
        let backend = start(SoloKey::MouseBack, tx, true).expect("event tap");
        back_button(true);
        assert_eq!(next(&rx), Some(SoloEdge::Press));
        back_button(false);
        assert_eq!(next(&rx), Some(SoloEdge::Release));
        backend.stop();
    }

    /// docs/dictation.md §29: a tap the system disables is installed again within a check
    /// interval, says so, and reports edges again; a refresh of a working tap changes nothing.
    #[test]
    #[ignore = "needs a macOS login session where this test binary has the Accessibility permission"]
    fn regression_a_tap_the_system_disables_is_put_back() {
        let _serial = serial();
        let (tx, rx) = mpsc::channel();
        let backend = start(SoloKey::RightCtrl, tx, true).expect("event tap (Accessibility)");
        right_ctrl(true);
        right_ctrl(false);
        assert_eq!((next(&rx), next(&rx)), (Some(SoloEdge::Press), Some(SoloEdge::Release)));
        backend.disable_tap();
        eprintln!("tap disabled");
        assert_eq!(rx.recv_timeout(WATCH_EVERY * 2 + Duration::from_secs(1)).ok(), Some(SoloEdge::Recovered), "the tap was put back");
        right_ctrl(true);
        right_ctrl(false);
        assert_eq!((next(&rx), next(&rx)), (Some(SoloEdge::Press), Some(SoloEdge::Release)), "the trigger works again");
        backend.refresh();
        assert_eq!(rx.recv_timeout(Duration::from_millis(500)).ok(), None, "an enabled tap is left alone");
        backend.stop();
    }

    /// docs/dictation.md §29: the physical state a hold-to-talk take is checked against, read
    /// from the HID system: a chord key, and the back button the running tap drops.
    #[test]
    #[ignore = "needs a macOS login session where this test binary has the Accessibility permission"]
    fn real_held_keys_read_through_the_hid_state() {
        let _serial = serial();
        let chord = HeldKey::Chord("F13".to_owned());
        f13(true);
        wait_until("F13 reads as down", || is_down(&chord) == Some(true));
        f13(false);
        wait_until("F13 reads as up", || is_down(&chord) == Some(false));
        let (tx, rx) = mpsc::channel();
        let backend = start(SoloKey::MouseBack, tx, true).expect("event tap");
        let back = HeldKey::Solo(SoloKey::MouseBack);
        back_button(true);
        assert_eq!(next(&rx), Some(SoloEdge::Press));
        wait_until("the dropped back button reads as down", || is_down(&back) == Some(true));
        back_button(false);
        assert_eq!(next(&rx), Some(SoloEdge::Release));
        wait_until("the back button reads as up", || is_down(&back) == Some(false));
        backend.stop();
        assert_eq!(is_down(&HeldKey::Chord("ScrollLock".to_owned())), None, "a Mac keyboard has no Scroll Lock");
    }
}
