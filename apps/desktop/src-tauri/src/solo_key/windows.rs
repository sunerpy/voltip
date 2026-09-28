//! Windows backend of the lone-key trigger (docs/dictation.md §13.1): the low-level keyboard and
//! mouse hooks (`WH_KEYBOARD_LL`, `WH_MOUSE_LL`) on a thread of their own that pumps messages.
//!
//! - A modifier trigger is passed on to the focused application. When it goes down on its own, an
//!   unassigned key ([`WINDOWS_MASK_VK`]) is tapped, so Windows never sees a lone Alt (the menu
//!   bar), a lone Win (Start) or a lone Shift (the IME's Chinese / English switch) on release.
//! - A mouse trigger is swallowed (the hook returns 1): the back button no longer navigates.
//! - Input injected by anyone (`LLKHF_INJECTED` / `LLMHF_INJECTED`, which Voltip's own paste
//!   carries) is not counted; the mask key is injected too, so it never counts as a chord.
//!
//! The hook procedures are plain functions, so the state they need lives in one static: one hook
//! at a time, which [`super::SoloHook`]'s lifecycle guarantees (the old one is dropped first).
//! The procedures only lock it, feed the tracker and send on a channel: the system removes a
//! low-level hook whose procedure is slow.

#![allow(unsafe_code)]

use std::ptr::null_mut;
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

use voltip_core::SoloKey;
use voltip_platform::solo_key::{SoloEdge, SoloInput, SoloTracker, WINDOWS_MASK_VK, WindowsButton, windows_button, windows_vk};
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, LLKHF_INJECTED, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, PostThreadMessageW, SetWindowsHookExW,
    UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_QUIT, WM_RBUTTONDOWN,
    WM_SYSKEYDOWN, WM_SYSKEYUP, WM_XBUTTONDOWN, WM_XBUTTONUP, XBUTTON1, XBUTTON2,
};

/// What the hook procedures need.
struct HookState {
    vk: Option<u32>,
    button: Option<WindowsButton>,
    tracker: SoloTracker,
    tx: Sender<SoloEdge>,
    synthetic: bool,
}

static STATE: Mutex<Option<HookState>> = Mutex::new(None);

fn state() -> MutexGuard<'static, Option<HookState>> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The hooks' thread.
pub struct Backend {
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl Backend {
    /// Quit the thread's message loop, which removes both hooks.
    pub fn stop(mut self) {
        // SAFETY: plain Win32 call with a thread id this module obtained; a thread that already
        // left its loop makes it fail harmlessly.
        unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Install the hooks for `key` on a new thread; the error names the call that failed.
pub fn start(key: SoloKey, tx: Sender<SoloEdge>, synthetic: bool) -> Result<Backend, String> {
    let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<u32, String>>(1);
    let thread = std::thread::Builder::new().name("voltip-solo-key-win".into()).spawn(move || run(key, tx, synthetic, &ready_tx)).map_err(|e| e.to_string())?;
    match ready_rx.recv_timeout(super::START_TIMEOUT) {
        Ok(Ok(thread_id)) => Ok(Backend { thread_id, thread: Some(thread) }),
        Ok(Err(e)) => {
            let _ = thread.join();
            Err(e)
        }
        Err(_) => Err("安装输入钩子超时".to_owned()),
    }
}

fn run(key: SoloKey, tx: Sender<SoloEdge>, synthetic: bool, ready: &mpsc::SyncSender<Result<u32, String>>) {
    *state() = Some(HookState { vk: windows_vk(key).map(u32::from), button: windows_button(key), tracker: SoloTracker::new(key), tx, synthetic });
    // SAFETY: a null module name asks for the executable's own handle, which stays valid.
    let module = unsafe { GetModuleHandleW(std::ptr::null()) };
    // The keyboard hook only matters for a modifier (a mouse trigger never chords); the mouse
    // hook always does: a click during a modifier's hold is a chord (Ctrl + click).
    let keyboard = if key.is_mouse() {
        null_mut()
    } else {
        // SAFETY: `keyboard_proc` has the HOOKPROC signature and lives for the whole program.
        unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), module, 0) }
    };
    // SAFETY: as above, for `mouse_proc`.
    let mouse = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module, 0) };
    if mouse.is_null() || (!key.is_mouse() && keyboard.is_null()) {
        let error = std::io::Error::last_os_error();
        unhook(keyboard);
        unhook(mouse);
        *state() = None;
        let _ = ready.send(Err(format!("SetWindowsHookEx 失败（{error}）")));
        return;
    }
    // SAFETY: no arguments; always succeeds.
    let _ = ready.send(Ok(unsafe { GetCurrentThreadId() }));
    tracing::info!(key = %key, "lone-key hooks installed (WH_KEYBOARD_LL / WH_MOUSE_LL)");
    let mut msg = MSG::default();
    // SAFETY: `msg` is a valid, writable MSG; a null window takes every message of this thread.
    while unsafe { GetMessageW(&raw mut msg, null_mut(), 0, 0) } > 0 {}
    unhook(keyboard);
    unhook(mouse);
    *state() = None;
    tracing::info!(key = %key, "lone-key hooks removed");
}

fn unhook(hook: HHOOK) {
    if !hook.is_null() {
        // SAFETY: a handle SetWindowsHookExW returned on this thread, unhooked once.
        unsafe { UnhookWindowsHookEx(hook) };
    }
}

/// Tap [`WINDOWS_MASK_VK`] (down, up) so the held modifier is no longer "lone" to Windows.
fn send_mask_key() {
    let key = |flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: WINDOWS_MASK_VK, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    };
    let inputs = [key(0), key(KEYEVENTF_KEYUP)];
    // SAFETY: two initialised INPUT values and their exact size.
    let sent = unsafe { SendInput(2, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
    if sent != 2 {
        tracing::debug!(sent, "mask key not sent (secure desktop or UIPI)");
    }
}

/// Feed the tracker and send its edge; whether a modifier just went down on its own (the caller
/// taps the mask key once the state is unlocked).
fn report(hook: &mut HookState, input: SoloInput) -> bool {
    let Some(edge) = hook.tracker.feed(input) else { return false };
    let _ = hook.tx.send(edge);
    edge == SoloEdge::Press && hook.vk.is_some()
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: for HC_ACTION, lparam points at the KBDLLHOOKSTRUCT of this event.
        let info = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        let mut mask = false;
        // The mask key is ours: never a chord, not even when the tests count injected input.
        if let Some(hook) = state().as_mut()
            && (hook.synthetic || info.flags & LLKHF_INJECTED == 0)
            && info.vkCode != u32::from(WINDOWS_MASK_VK)
        {
            let own = Some(info.vkCode) == hook.vk;
            let input = match wparam as u32 {
                WM_KEYDOWN | WM_SYSKEYDOWN if own => Some(SoloInput::TriggerDown),
                WM_KEYUP | WM_SYSKEYUP if own => Some(SoloInput::TriggerUp),
                WM_KEYDOWN | WM_SYSKEYDOWN => Some(SoloInput::OtherDown),
                _ => None,
            };
            mask = input.is_some_and(|input| report(hook, input));
        }
        if mask {
            send_mask_key();
        }
    }
    // SAFETY: passes the event on unchanged, as every hook must.
    unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: for HC_ACTION, lparam points at the MSLLHOOKSTRUCT of this event.
        let info = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
        if let Some(hook) = state().as_mut()
            && (hook.synthetic || info.flags & LLMHF_INJECTED == 0)
        {
            let x = match (info.mouseData >> 16) as u16 {
                XBUTTON1 => Some(WindowsButton::X1),
                XBUTTON2 => Some(WindowsButton::X2),
                _ => None,
            };
            let (button, down) = match wparam as u32 {
                WM_MBUTTONDOWN => (Some(WindowsButton::Middle), true),
                WM_MBUTTONUP => (Some(WindowsButton::Middle), false),
                WM_XBUTTONDOWN => (x, true),
                WM_XBUTTONUP => (x, false),
                WM_LBUTTONDOWN | WM_RBUTTONDOWN => (None, true),
                _ => (None, false),
            };
            if button.is_some() && button == hook.button {
                report(hook, if down { SoloInput::TriggerDown } else { SoloInput::TriggerUp });
                // Swallowed: the application never sees the trigger button.
                return 1;
            }
            if down {
                // A click during a modifier's hold; a modifier's press never comes from here, so
                // there is no mask key to tap.
                report(hook, SoloInput::OtherDown);
            }
        }
    }
    // SAFETY: passes the event on unchanged.
    unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
}

/// Real hook tests (`#[ignore]`d: they need an interactive desktop session, where SendInput
/// reaches the low-level hooks). Run them on the Windows machine with
/// `cargo test -p voltip-desktop --lib solo_key -- --ignored`.
#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::time::Duration;

    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{INPUT_MOUSE, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT};

    use super::*;

    fn key(vk: u16, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { 0 }, time: 0, dwExtraInfo: 0 } },
        }
    }

    fn xbutton(which: u16, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: u32::from(which),
                    dwFlags: if up { MOUSEEVENTF_XUP } else { MOUSEEVENTF_XDOWN },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn send(inputs: &[INPUT]) {
        // SAFETY: initialised INPUT values and their exact size.
        let sent = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
        assert_eq!(sent as usize, inputs.len(), "SendInput needs an unlocked interactive desktop");
    }

    fn next(rx: &mpsc::Receiver<SoloEdge>) -> Option<SoloEdge> {
        rx.recv_timeout(Duration::from_secs(3)).ok()
    }

    #[test]
    #[ignore = "needs an interactive Windows desktop"]
    fn real_hooks_report_a_lone_modifier_its_chord_and_a_side_button() {
        let (tx, rx) = mpsc::channel();
        let backend = start(SoloKey::RightCtrl, tx, true).expect("hooks");
        send(&[key(0xA3, false)]);
        assert_eq!(next(&rx), Some(SoloEdge::Press));
        send(&[key(0xA3, true)]);
        assert_eq!(next(&rx), Some(SoloEdge::Release));
        // Right Ctrl + C: pressed, then chorded; the mask key tapped at the press is not a chord.
        send(&[key(0xA3, false), key(u16::from(b'C'), false), key(u16::from(b'C'), true), key(0xA3, true)]);
        assert_eq!(next(&rx), Some(SoloEdge::Press));
        assert_eq!(next(&rx), Some(SoloEdge::Chorded));
        assert_eq!(rx.recv_timeout(Duration::from_millis(300)).ok(), None);
        backend.stop();

        let (tx, rx) = mpsc::channel();
        let backend = start(SoloKey::MouseBack, tx, true).expect("hooks");
        send(&[xbutton(XBUTTON1, false), xbutton(XBUTTON1, true)]);
        assert_eq!(next(&rx), Some(SoloEdge::Press));
        assert_eq!(next(&rx), Some(SoloEdge::Release));
        backend.stop();

        // Injected input does not count without `synthetic`.
        let (tx, rx) = mpsc::channel();
        let backend = start(SoloKey::RightCtrl, tx, false).expect("hooks");
        send(&[key(0xA3, false), key(0xA3, true)]);
        assert_eq!(rx.recv_timeout(Duration::from_millis(500)).ok(), None);
        backend.stop();
    }
}
