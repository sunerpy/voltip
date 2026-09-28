//! macOS backend of the lone-key trigger (docs/dictation.md §13.1): an active `CGEventTap` on the
//! session, on a thread that runs its own run loop.
//!
//! An active tap (it can drop events) needs the Accessibility permission, which the paste already
//! needs; a listen-only tap would need Input Monitoring on top. The tap drops the mouse trigger
//! (the back button no longer navigates) and passes every key on. A modifier's state comes from
//! the event's device-dependent flag (`NX_DEVICER*KEYMASK`, `kCGEventFlagMaskSecondaryFn`), not
//! from counting events. What Voltip posts itself carries enigo's `EVENT_SOURCE_USER_DATA` marker
//! and is ignored. When the system disables the tap (a slow callback, secure input), the run loop
//! stops and the tap is installed again.

use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender, SyncSender};
use std::thread::JoinHandle;

use core_foundation::runloop::CFRunLoop;
use core_graphics::event::{CGEvent, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType, CallbackResult, EventField};
use voltip_platform::solo_key::SoloKey;
use voltip_platform::solo_key::{SoloEdge, SoloInput, SoloTracker, macos_button, macos_modifier};

/// The `EVENT_SOURCE_USER_DATA` enigo stamps on everything it posts (`enigo::EVENT_MARKER`):
/// Voltip's own paste and copy chords.
const ENIGO_MARKER: i64 = 100;

/// The tap's thread and its run loop.
pub struct Backend {
    run_loop: CFRunLoop,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Backend {
    /// Stop the run loop, which removes the tap.
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.run_loop.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Install the tap for `key` on a new thread; the error says what the user has to allow.
pub fn start(key: SoloKey, tx: Sender<SoloEdge>, synthetic: bool) -> Result<Backend, String> {
    let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<CFRunLoop, String>>(1);
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let thread =
        std::thread::Builder::new().name("voltip-solo-key-mac".into()).spawn(move || run(key, &tx, synthetic, &flag, ready_tx)).map_err(|e| e.to_string())?;
    match ready_rx.recv_timeout(super::START_TIMEOUT) {
        Ok(Ok(run_loop)) => Ok(Backend { run_loop, stop, thread: Some(thread) }),
        Ok(Err(e)) => {
            let _ = thread.join();
            Err(e)
        }
        Err(_) => Err("安装事件监听超时".to_owned()),
    }
}

fn run(key: SoloKey, tx: &Sender<SoloEdge>, synthetic: bool, stop: &AtomicBool, ready: SyncSender<Result<CFRunLoop, String>>) {
    let mut ready = Some(ready);
    let tracker = RefCell::new(SoloTracker::new(key));
    let modifier = macos_modifier(key);
    let button = macos_button(key);
    loop {
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
        let installed = CGEventTap::with_enabled(
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
            || {
                if let Some(ready) = ready.take() {
                    let _ = ready.send(Ok(CFRunLoop::get_current()));
                    tracing::info!(key = %key, "lone-key event tap installed");
                }
                CFRunLoop::run_current();
            },
        );
        if installed.is_err() {
            if let Some(ready) = ready.take() {
                let _ = ready.send(Err("需要在「系统设置 › 隐私与安全性 › 辅助功能」里允许 Voltip".to_owned()));
            } else {
                tracing::warn!(key = %key, "the event tap could not be installed again");
            }
            return;
        }
        if stop.load(Ordering::Relaxed) || !disabled.get() {
            return;
        }
        tracing::warn!(key = %key, "the system disabled the lone-key event tap; installing it again");
    }
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

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use core_graphics::event::{CGEventFlags, CGMouseButton};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
    use core_graphics::geometry::CGPoint;

    use super::*;

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

    /// `kVK_ANSI_C` down and up.
    fn key_c() {
        for down in [true, false] {
            CGEvent::new_keyboard_event(source(), 8, down).expect("event").post(CGEventTapLocation::HID);
        }
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

    #[test]
    #[ignore = "needs a macOS login session where this test binary has the Accessibility permission"]
    fn real_tap_reports_a_lone_modifier_its_chord_and_a_side_button() {
        let (tx, rx) = mpsc::channel();
        let backend = start(SoloKey::RightCtrl, tx, true).expect("event tap (Accessibility)");
        right_ctrl(true);
        assert_eq!(next(&rx), Some(SoloEdge::Press));
        right_ctrl(false);
        assert_eq!(next(&rx), Some(SoloEdge::Release));
        // Right Control + C: pressed, then chorded; its release is not reported.
        right_ctrl(true);
        key_c();
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
}
