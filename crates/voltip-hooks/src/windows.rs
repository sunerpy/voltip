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
//! Keeping the hooks (docs/dictation.md §29): Windows removes a low-level hook whose procedure was
//! too slow (`LowLevelHooksTimeout`) without telling its owner. The thread therefore also owns a
//! hidden tool window that receives Raw Input (`RIDEV_INPUTSINK`, so in the background too) of
//! the mice and, for a modifier trigger, the keyboards, and the power and session notifications.
//! Presses Raw Input reports that the matching hook never called back for ([`HookWatch`], one per
//! hook) put both hooks back, and so do waking from sleep and unlocking the session. Raw Input of
//! a device class has one target window per process, and the last registration wins: this one
//! replaces tao's, whose `DeviceEvent`s Voltip never uses, and nothing registers it again unless
//! the app calls `set_device_event_filter`, which `make lint` keeps out of the tree.
//!
//! An event a low-level hook keeps (the swallowed mouse trigger) reaches neither Raw Input nor the
//! async key state, so the mouse hook records the trigger button's state itself ([`is_down`]).
//!
//! The hook procedures are plain functions, so the state they need lives in one static: one hook
//! at a time: a caller stops the running [`Backend`] before it starts the next.
//! The procedures only lock it, feed the tracker and send on a channel: the system removes a
//! low-level hook whose procedure is slow.

#![allow(unsafe_code)]

use std::cell::Cell;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Instant;

use voltip_platform::held_key::{self, HeldKey};
use voltip_platform::solo_key::SoloKey;
use voltip_platform::solo_key::{SoloEdge, SoloInput, SoloLoss, SoloTracker, WINDOWS_MASK_VK, WindowsButton, windows_button, windows_vk};
use windows_sys::Win32::Foundation::{ERROR_CLASS_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Power::{HPOWERNOTIFY, RegisterSuspendResumeNotification, UnregisterSuspendResumeNotification};
use windows_sys::Win32::System::RemoteDesktop::{NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification, WTSUnRegisterSessionNotification};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput};
use windows_sys::Win32::UI::Input::{
    GetRawInputData, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RID_INPUT, RIDEV_INPUTSINK, RIDEV_REMOVE, RIM_TYPEKEYBOARD, RIM_TYPEMOUSE,
    RegisterRawInputDevices,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DEVICE_NOTIFY_WINDOW_HANDLE, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, HC_ACTION, HHOOK,
    KBDLLHOOKSTRUCT, LLKHF_INJECTED, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, PostThreadMessageW, RI_KEY_BREAK, RI_MOUSE_BUTTON_4_DOWN, RI_MOUSE_BUTTON_4_UP,
    RI_MOUSE_BUTTON_5_DOWN, RI_MOUSE_BUTTON_5_UP, RI_MOUSE_LEFT_BUTTON_DOWN, RI_MOUSE_MIDDLE_BUTTON_DOWN, RI_MOUSE_MIDDLE_BUTTON_UP,
    RI_MOUSE_RIGHT_BUTTON_DOWN, RegisterClassW, SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_INPUT, WM_KEYDOWN, WM_KEYUP,
    WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_QUIT, WM_RBUTTONDOWN, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_XBUTTONDOWN, WM_XBUTTONUP, WNDCLASSW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP, XBUTTON1, XBUTTON2,
};

use crate::watch::{self, HookWatch, Recovery, recovery_for_message};

// The constants the host-independent rules in `watch.rs` spell out themselves.
const _: () = assert!(watch::WM_POWERBROADCAST == windows_sys::Win32::UI::WindowsAndMessaging::WM_POWERBROADCAST);
const _: () = assert!(watch::WM_WTSSESSION_CHANGE == windows_sys::Win32::UI::WindowsAndMessaging::WM_WTSSESSION_CHANGE);
const _: () = assert!(watch::PBT_APMRESUMEAUTOMATIC as u32 == windows_sys::Win32::UI::WindowsAndMessaging::PBT_APMRESUMEAUTOMATIC);
const _: () = assert!(watch::PBT_APMRESUMESUSPEND as u32 == windows_sys::Win32::UI::WindowsAndMessaging::PBT_APMRESUMESUSPEND);
const _: () = assert!(watch::WTS_SESSION_UNLOCK as u32 == windows_sys::Win32::UI::WindowsAndMessaging::WTS_SESSION_UNLOCK);

/// A thread message that removes the hooks without telling the watch, as the system does (the
/// tests' stand-in for `LowLevelHooksTimeout`).
#[cfg(test)]
const WM_FORCE_UNHOOK: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 1;

/// What the hook procedures need.
struct HookState {
    key: SoloKey,
    vk: Option<u32>,
    button: Option<WindowsButton>,
    tracker: SoloTracker,
    tx: Sender<SoloEdge>,
    synthetic: bool,
    /// Raw Input against the keyboard hook (a modifier trigger's) and against the mouse hook.
    keys: HookWatch,
    mouse: HookWatch,
}

static STATE: Mutex<Option<HookState>> = Mutex::new(None);

/// The mouse trigger of the running hook ([`button_id`], 0 = none) and whether it is down: the
/// mouse hook, which swallows it, records its edges; Raw Input, a release that gets past a hook
/// the system removed.
static BUTTON: AtomicU8 = AtomicU8::new(0);
static BUTTON_DOWN: AtomicBool = AtomicBool::new(false);
/// How many times the hook thread obeyed [`WM_FORCE_UNHOOK`] (the tests wait for it).
#[cfg(test)]
static FORCED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

thread_local! {
    /// The hook thread's two hooks (null when not installed).
    static HOOKS: Cell<(HHOOK, HHOOK)> = const { Cell::new((null_mut(), null_mut())) };
}

fn state() -> MutexGuard<'static, Option<HookState>> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The hooks' thread.
pub struct Backend {
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
    /// The thread's hidden window, as a number (a window handle is not `Send`).
    #[cfg_attr(not(test), allow(dead_code))]
    window: usize,
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

    /// Nothing to do: the hook thread hears waking and unlocking itself (docs/dictation.md §29).
    pub fn refresh(&self) {}
}

/// Install the hooks for `key` on a new thread; the error names the call that failed.
pub fn start(key: SoloKey, tx: Sender<SoloEdge>, synthetic: bool) -> Result<Backend, String> {
    let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(u32, usize), String>>(1);
    let thread = std::thread::Builder::new().name("voltip-solo-key-win".into()).spawn(move || run(key, tx, synthetic, &ready_tx)).map_err(|e| e.to_string())?;
    match ready_rx.recv_timeout(super::START_TIMEOUT) {
        Ok(Ok((thread_id, window))) => Ok(Backend { thread_id, thread: Some(thread), window }),
        Ok(Err(e)) => {
            let _ = thread.join();
            Err(e)
        }
        Err(_) => Err("安装输入钩子超时".to_owned()),
    }
}

fn run(key: SoloKey, tx: Sender<SoloEdge>, synthetic: bool, ready: &mpsc::SyncSender<Result<(u32, usize), String>>) {
    *state() = Some(HookState {
        key,
        vk: windows_vk(key).map(u32::from),
        button: windows_button(key),
        tracker: SoloTracker::new(key),
        tx,
        synthetic,
        keys: HookWatch::new(synthetic),
        mouse: HookWatch::new(synthetic),
    });
    BUTTON_DOWN.store(false, Ordering::SeqCst);
    BUTTON.store(button_id(windows_button(key)), Ordering::SeqCst);
    if let Err(e) = install(key) {
        BUTTON.store(0, Ordering::SeqCst);
        *state() = None;
        let _ = ready.send(Err(e));
        return;
    }
    // Without the window the hooks still work; only their removal goes unnoticed.
    let window = Watcher::open(key)
        .map_err(|e| tracing::warn!(key = %key, error = %e, "the lone-key hook's watch window is missing; a removed hook is not put back"))
        .ok();
    // SAFETY: no arguments; always succeeds.
    let _ = ready.send(Ok((unsafe { GetCurrentThreadId() }, window.as_ref().map_or(0, |w| w.hwnd as usize))));
    tracing::info!(key = %key, watched = window.is_some(), "lone-key hooks installed (WH_KEYBOARD_LL / WH_MOUSE_LL)");
    let mut msg = MSG::default();
    // SAFETY: `msg` is a valid, writable MSG; a null window takes every message of this thread.
    while unsafe { GetMessageW(&raw mut msg, null_mut(), 0, 0) } > 0 {
        #[cfg(test)]
        if msg.hwnd.is_null() && msg.message == WM_FORCE_UNHOOK {
            uninstall();
            FORCED.fetch_add(1, Ordering::SeqCst);
            continue;
        }
        // SAFETY: a message GetMessageW filled in, handed to its window's procedure.
        unsafe { DispatchMessageW(&raw const msg) };
    }
    uninstall();
    drop(window);
    BUTTON.store(0, Ordering::SeqCst);
    *state() = None;
    tracing::info!(key = %key, "lone-key hooks removed");
}

/// Set the hooks `key` needs on this thread.
fn install(key: SoloKey) -> Result<(), String> {
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
        return Err(format!("SetWindowsHookEx 失败（{error}）"));
    }
    HOOKS.with(|hooks| hooks.set((keyboard, mouse)));
    Ok(())
}

/// Remove this thread's hooks (a hook the system already removed fails to unhook, harmlessly).
fn uninstall() {
    let (keyboard, mouse) = HOOKS.with(|hooks| hooks.replace((null_mut(), null_mut())));
    unhook(keyboard);
    unhook(mouse);
}

fn unhook(hook: HHOOK) {
    if !hook.is_null() {
        // SAFETY: a handle SetWindowsHookExW returned on this thread, unhooked once.
        unsafe { UnhookWindowsHookEx(hook) };
    }
}

/// Put the hooks back (docs/dictation.md §29) and report it: a trigger held across the gap is let
/// go without an edge ([`SoloTracker::resumed`]). When Windows refuses, [`SoloEdge::Lost`] says so;
/// the watch keeps counting and the next verdict, wake or unlock tries again.
fn recover(recovery: Recovery) {
    let Some(key) = state().as_ref().map(|hook| hook.key) else { return };
    uninstall();
    let installed = install(key);
    let down = is_down(&HeldKey::Solo(key)).unwrap_or(false);
    let mut guard = state();
    let Some(hook) = guard.as_mut() else { return };
    hook.keys.on_reinstall();
    hook.mouse.on_reinstall();
    match installed {
        Ok(()) => {
            hook.tracker = SoloTracker::resumed(key, down);
            tracing::warn!(key = %key, why = recovery.as_str(), held = down, "lone-key hooks put back");
            let _ = hook.tx.send(SoloEdge::Recovered);
        }
        Err(e) => {
            tracing::warn!(key = %key, why = recovery.as_str(), error = %e, "lone-key hooks could not be put back");
            let _ = hook.tx.send(SoloEdge::Lost(SoloLoss::Hook));
        }
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
        if let Some(hook) = state().as_mut() {
            // The hook is there: whatever Raw Input reported before this says nothing.
            hook.keys.on_hook();
            // The mask key is ours: never a chord, not even when the tests count injected input.
            if (hook.synthetic || info.flags & LLKHF_INJECTED == 0) && info.vkCode != u32::from(WINDOWS_MASK_VK) {
                let own = Some(info.vkCode) == hook.vk;
                let input = match wparam as u32 {
                    WM_KEYDOWN | WM_SYSKEYDOWN if own => Some(SoloInput::TriggerDown),
                    WM_KEYUP | WM_SYSKEYUP if own => Some(SoloInput::TriggerUp),
                    WM_KEYDOWN | WM_SYSKEYDOWN => Some(SoloInput::OtherDown),
                    _ => None,
                };
                mask = input.is_some_and(|input| report(hook, input));
            }
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
        if let Some(hook) = state().as_mut() {
            // Every move calls back: a live mouse hook is never judged removed.
            hook.mouse.on_hook();
            if hook.synthetic || info.flags & LLMHF_INJECTED == 0 {
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
                    BUTTON_DOWN.store(down, Ordering::SeqCst);
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
    }
    // SAFETY: passes the event on unchanged.
    unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
}

/// The hook thread's hidden window: Raw Input of the devices the hooks watch, the power and the
/// session notifications. Dropping it unregisters all three.
struct Watcher {
    hwnd: HWND,
    power: HPOWERNOTIFY,
    session: bool,
    raw: Vec<RAWINPUTDEVICE>,
}

/// Window class of [`Watcher`], NUL-terminated UTF-16.
const CLASS_NAME: &str = "VoltipSoloKeyWatch";
const CLASS: [u16; CLASS_NAME.len() + 1] = utf16(CLASS_NAME);

/// An ASCII `s` as UTF-16, zero-padded to `N`.
const fn utf16<const N: usize>(s: &str) -> [u16; N] {
    let bytes = s.as_bytes();
    let mut out = [0u16; N];
    let mut i = 0;
    while i < bytes.len() {
        out[i] = bytes[i] as u16;
        i += 1;
    }
    out
}

impl Watcher {
    fn open(key: SoloKey) -> Result<Self, String> {
        // SAFETY: a null module name asks for the executable's own handle.
        let module = unsafe { GetModuleHandleW(std::ptr::null()) };
        let class = WNDCLASSW { lpfnWndProc: Some(window_proc), hInstance: module, lpszClassName: CLASS.as_ptr(), ..WNDCLASSW::default() };
        // SAFETY: a fully initialised class whose name and procedure live for the whole program.
        if unsafe { RegisterClassW(&raw const class) } == 0 && unsafe { GetLastError() } != ERROR_CLASS_ALREADY_EXISTS {
            return Err(format!("RegisterClass 失败（{}）", std::io::Error::last_os_error()));
        }
        // Never shown, never activated, no taskbar button. Not a message-only window: those get no
        // Raw Input in the background (`RIDEV_INPUTSINK` needs a real window).
        // SAFETY: the class registered above; every pointer argument is null or that class name.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                CLASS.as_ptr(),
                CLASS.as_ptr(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                module,
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            return Err(format!("CreateWindowEx 失败（{}）", std::io::Error::last_os_error()));
        }
        let mut watcher = Self { hwnd, power: 0, session: false, raw: Vec::new() };
        // Generic desktop: mice (usage 2) for the mouse hook, keyboards (usage 6) for a modifier's
        // keyboard hook (a mouse trigger has none).
        let device = |usage| RAWINPUTDEVICE { usUsagePage: 0x01, usUsage: usage, dwFlags: RIDEV_INPUTSINK, hwndTarget: hwnd };
        let devices: Vec<RAWINPUTDEVICE> = if key.is_mouse() { vec![device(0x02)] } else { vec![device(0x02), device(0x06)] };
        // SAFETY: initialised RAWINPUTDEVICE values, their count and their exact size.
        if unsafe { RegisterRawInputDevices(devices.as_ptr(), devices.len() as u32, std::mem::size_of::<RAWINPUTDEVICE>() as u32) } != 0 {
            watcher.raw = devices;
        } else {
            tracing::warn!(error = %std::io::Error::last_os_error(), "RegisterRawInputDevices failed; a removed lone-key hook is not noticed");
        }
        // SAFETY: a window this thread owns; the handle is unregistered on drop.
        watcher.power = unsafe { RegisterSuspendResumeNotification(hwnd as HANDLE, DEVICE_NOTIFY_WINDOW_HANDLE) };
        // SAFETY: as above.
        watcher.session = unsafe { WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) } != 0;
        Ok(watcher)
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        if !self.raw.is_empty() {
            let removal: Vec<RAWINPUTDEVICE> =
                self.raw.iter().map(|device| RAWINPUTDEVICE { dwFlags: RIDEV_REMOVE, hwndTarget: null_mut(), ..*device }).collect();
            // SAFETY: the registrations made in `open`, removed once.
            unsafe { RegisterRawInputDevices(removal.as_ptr(), removal.len() as u32, std::mem::size_of::<RAWINPUTDEVICE>() as u32) };
        }
        if self.power != 0 {
            // SAFETY: the handle RegisterSuspendResumeNotification returned, unregistered once.
            unsafe { UnregisterSuspendResumeNotification(self.power) };
        }
        if self.session {
            // SAFETY: the window registered in `open`.
            unsafe { WTSUnRegisterSessionNotification(self.hwnd) };
        }
        // SAFETY: a window this thread created, destroyed once.
        unsafe { DestroyWindow(self.hwnd) };
    }
}

/// [`BUTTON`]'s number for a trigger button.
const fn button_id(button: Option<WindowsButton>) -> u8 {
    match button {
        None => 0,
        Some(WindowsButton::Middle) => 1,
        Some(WindowsButton::X1) => 2,
        Some(WindowsButton::X2) => 3,
    }
}

/// One Raw Input report of a key or button going down.
struct RawPress {
    /// A mouse button, not a key: the mouse hook's watch counts it.
    mouse: bool,
    /// A device sent it (injected input has no device handle).
    from_device: bool,
}

/// Read the report behind `WM_INPUT`; `None` when it is no press. A trigger button's release that
/// got past the hooks clears [`BUTTON_DOWN`] on the way.
fn raw_press(handle: HRAWINPUT) -> Option<RawPress> {
    let mut input = RAWINPUT::default();
    let mut size = std::mem::size_of::<RAWINPUT>() as u32;
    // SAFETY: a writable RAWINPUT-sized buffer and its size; keyboards and mice fit in it.
    let read = unsafe { GetRawInputData(handle, RID_INPUT, (&raw mut input).cast(), &raw mut size, std::mem::size_of::<RAWINPUTHEADER>() as u32) };
    if read == u32::MAX || read == 0 {
        return None;
    }
    let from_device = !input.header.hDevice.is_null();
    match input.header.dwType {
        RIM_TYPEKEYBOARD => {
            // SAFETY: the header says this report is a keyboard's.
            let keyboard = unsafe { input.data.keyboard };
            (u32::from(keyboard.Flags) & RI_KEY_BREAK == 0).then_some(RawPress { mouse: false, from_device })
        }
        RIM_TYPEMOUSE => {
            // SAFETY: the header says this report is a mouse's.
            let flags = u32::from(unsafe { input.data.mouse.Anonymous.Anonymous.usButtonFlags });
            let (down_flag, up_flag) = match BUTTON.load(Ordering::SeqCst) {
                1 => (RI_MOUSE_MIDDLE_BUTTON_DOWN, RI_MOUSE_MIDDLE_BUTTON_UP),
                2 => (RI_MOUSE_BUTTON_4_DOWN, RI_MOUSE_BUTTON_4_UP),
                3 => (RI_MOUSE_BUTTON_5_DOWN, RI_MOUSE_BUTTON_5_UP),
                _ => (0, 0),
            };
            // Seen here, the trigger button got past the hooks: there are none.
            if flags & down_flag != 0 {
                BUTTON_DOWN.store(true, Ordering::SeqCst);
            } else if flags & up_flag != 0 {
                BUTTON_DOWN.store(false, Ordering::SeqCst);
            }
            let any_down =
                RI_MOUSE_LEFT_BUTTON_DOWN | RI_MOUSE_RIGHT_BUTTON_DOWN | RI_MOUSE_MIDDLE_BUTTON_DOWN | RI_MOUSE_BUTTON_4_DOWN | RI_MOUSE_BUTTON_5_DOWN;
            (flags & any_down != 0).then_some(RawPress { mouse: true, from_device })
        }
        _ => None,
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let recovery = if message == WM_INPUT {
        raw_press(lparam as HRAWINPUT).and_then(|press| {
            let mut guard = state();
            let hook = guard.as_mut()?;
            let watch = if press.mouse { &mut hook.mouse } else { &mut hook.keys };
            watch.on_raw(Instant::now(), press.from_device)
        })
    } else {
        recovery_for_message(message, wparam)
    };
    if let Some(recovery) = recovery {
        recover(recovery);
    }
    // SAFETY: every message also takes the default path (WM_INPUT's clean-up included).
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

/// Whether `key` is physically down (docs/dictation.md §29): `GetAsyncKeyState`, except for the
/// mouse trigger of the running hook, which only the hook and Raw Input see ([`BUTTON_DOWN`]).
pub fn is_down(key: &HeldKey) -> Option<bool> {
    if let HeldKey::Solo(solo) = key {
        let id = button_id(windows_button(*solo));
        if id != 0 && BUTTON.load(Ordering::SeqCst) == id {
            return Some(BUTTON_DOWN.load(Ordering::SeqCst));
        }
    }
    let vk = held_key::windows_vk(key)?;
    // SAFETY: plain Win32 call with a virtual-key code.
    Some(unsafe { GetAsyncKeyState(i32::from(vk)) } as u16 & 0x8000 != 0)
}

/// Real hook tests (`#[ignore]`d: they need an interactive desktop session, where SendInput
/// reaches the low-level hooks). CI runs them in the `hooks-windows` job; by hand:
/// `cargo test -p voltip-hooks --lib -- --ignored --test-threads=1 --nocapture`.
#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::time::Duration;

    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{INPUT_MOUSE, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT};
    use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;

    use super::*;

    /// One hook at a time ([`STATE`]), whatever the test runner's thread count.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn serial() -> MutexGuard<'static, ()> {
        SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
    }

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

    /// VK_RCONTROL, the trigger of most tests.
    const RIGHT_CTRL: u16 = 0xA3;

    /// VK_F13: a key nothing on the runner's desktop reacts to. Ctrl + C would reach the focused
    /// console, which may be the one running this test.
    const CHORD_VK: u16 = 0x7C;

    fn next(rx: &mpsc::Receiver<SoloEdge>) -> Option<SoloEdge> {
        rx.recv_timeout(Duration::from_secs(3)).ok()
    }

    /// Nothing more within 300 ms.
    fn quiet(rx: &mpsc::Receiver<SoloEdge>) -> Option<SoloEdge> {
        rx.recv_timeout(Duration::from_millis(300)).ok()
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
        /// Remove the hooks the way the system does: silently, the watch none the wiser. Returns
        /// once the hook thread has done it.
        fn force_unhook(&self) {
            let before = FORCED.load(Ordering::SeqCst);
            // SAFETY: plain Win32 call with this backend's thread id.
            assert_ne!(unsafe { PostThreadMessageW(self.thread_id, WM_FORCE_UNHOOK, 0, 0) }, 0, "PostThreadMessage");
            wait_until("the hooks removed", || FORCED.load(Ordering::SeqCst) > before);
        }

        /// Hand the hook thread's window `message` as the system would.
        fn notify(&self, message: u32, wparam: usize) {
            assert_ne!(self.window, 0, "the watch window");
            // SAFETY: the backend's own window, alive while the backend is.
            assert_ne!(unsafe { PostMessageW(self.window as HWND, message, wparam, 0) }, 0, "PostMessage");
        }
    }

    #[test]
    #[ignore = "needs an interactive Windows desktop"]
    fn real_hooks_report_a_lone_modifier_its_chord_and_a_side_button() {
        let _serial = serial();
        let (tx, rx) = mpsc::channel();
        let backend = start(SoloKey::RightCtrl, tx, true).expect("hooks");
        eprintln!("hooks installed; Right Ctrl alone");
        send(&[key(RIGHT_CTRL, false)]);
        assert_eq!(next(&rx), Some(SoloEdge::Press));
        send(&[key(RIGHT_CTRL, true)]);
        assert_eq!(next(&rx), Some(SoloEdge::Release));
        eprintln!("Right Ctrl + F13");
        // Right Ctrl + another key: pressed, then chorded; the mask key tapped at the press is not
        // a chord.
        send(&[key(RIGHT_CTRL, false), key(CHORD_VK, false), key(CHORD_VK, true), key(RIGHT_CTRL, true)]);
        assert_eq!(next(&rx), Some(SoloEdge::Press));
        assert_eq!(next(&rx), Some(SoloEdge::Chorded));
        assert_eq!(quiet(&rx), None);
        backend.stop();

        eprintln!("the back button");
        let (tx, rx) = mpsc::channel();
        let backend = start(SoloKey::MouseBack, tx, true).expect("hooks");
        send(&[xbutton(XBUTTON1, false), xbutton(XBUTTON1, true)]);
        assert_eq!(next(&rx), Some(SoloEdge::Press));
        assert_eq!(next(&rx), Some(SoloEdge::Release));
        backend.stop();

        eprintln!("injected input without synthetic");
        // Injected input does not count without `synthetic`.
        let (tx, rx) = mpsc::channel();
        let backend = start(SoloKey::RightCtrl, tx, false).expect("hooks");
        send(&[key(RIGHT_CTRL, false), key(RIGHT_CTRL, true)]);
        assert_eq!(rx.recv_timeout(Duration::from_millis(500)).ok(), None);
        backend.stop();
    }

    /// docs/dictation.md §29: the system removes a slow hook without a word. Raw Input still sees
    /// the presses; three that no hook callback saw, over a second, put the hooks back, and the
    /// trigger works again.
    #[test]
    #[ignore = "needs an interactive Windows desktop"]
    fn regression_a_hook_windows_removed_silently_is_put_back() {
        let _serial = serial();
        let (tx, rx) = mpsc::channel();
        let backend = start(SoloKey::RightCtrl, tx, true).expect("hooks");
        send(&[key(RIGHT_CTRL, false), key(RIGHT_CTRL, true)]);
        assert_eq!((next(&rx), next(&rx)), (Some(SoloEdge::Press), Some(SoloEdge::Release)));
        backend.force_unhook();
        eprintln!("hooks removed; the trigger reports nothing");
        send(&[key(RIGHT_CTRL, false), key(RIGHT_CTRL, true)]);
        assert_eq!(quiet(&rx), None, "a removed hook reports nothing");
        // Two more presses the hook does not see, the last a second after the first (the Right
        // Ctrl above): the verdict needs that time to pass, and the sleeps are that time.
        std::thread::sleep(Duration::from_millis(600));
        send(&[key(CHORD_VK, false), key(CHORD_VK, true)]);
        std::thread::sleep(Duration::from_millis(600));
        send(&[key(CHORD_VK, false), key(CHORD_VK, true)]);
        assert_eq!(next(&rx), Some(SoloEdge::Recovered), "the watch put the hooks back");
        send(&[key(RIGHT_CTRL, false), key(RIGHT_CTRL, true)]);
        assert_eq!((next(&rx), next(&rx)), (Some(SoloEdge::Press), Some(SoloEdge::Release)), "the trigger works again");
        // The same for the mouse hook of a mouse trigger.
        backend.stop();
        let (tx, rx) = mpsc::channel();
        let backend = start(SoloKey::MouseBack, tx, true).expect("hooks");
        backend.force_unhook();
        eprintln!("mouse hook removed");
        send(&[xbutton(XBUTTON1, false), xbutton(XBUTTON1, true)]);
        assert_eq!(quiet(&rx), None, "a removed hook reports nothing");
        std::thread::sleep(Duration::from_millis(600));
        send(&[xbutton(XBUTTON1, false), xbutton(XBUTTON1, true)]);
        std::thread::sleep(Duration::from_millis(600));
        send(&[xbutton(XBUTTON1, false), xbutton(XBUTTON1, true)]);
        assert_eq!(next(&rx), Some(SoloEdge::Recovered), "the watch put the hooks back");
        send(&[xbutton(XBUTTON1, false), xbutton(XBUTTON1, true)]);
        assert_eq!((next(&rx), next(&rx)), (Some(SoloEdge::Press), Some(SoloEdge::Release)), "the button works again");
        backend.stop();
    }

    /// docs/dictation.md §29: waking from sleep and unlocking the session put the hooks back
    /// (here the window gets the messages the system would send), and a trigger held across it
    /// starts nothing until it has gone up.
    #[test]
    #[ignore = "needs an interactive Windows desktop"]
    fn regression_waking_and_unlocking_put_the_hook_back() {
        let _serial = serial();
        let (tx, rx) = mpsc::channel();
        let backend = start(SoloKey::RightCtrl, tx, true).expect("hooks");
        backend.notify(watch::WM_POWERBROADCAST, watch::PBT_APMRESUMEAUTOMATIC);
        assert_eq!(next(&rx), Some(SoloEdge::Recovered));
        backend.notify(watch::WM_WTSSESSION_CHANGE, watch::WTS_SESSION_UNLOCK);
        assert_eq!(next(&rx), Some(SoloEdge::Recovered));
        // Going to sleep is nothing.
        backend.notify(watch::WM_POWERBROADCAST, 0x0004);
        assert_eq!(quiet(&rx), None);
        // Held when the hooks come back: its auto-repeat and its release report nothing.
        send(&[key(RIGHT_CTRL, false)]);
        assert_eq!(next(&rx), Some(SoloEdge::Press));
        backend.notify(watch::WM_POWERBROADCAST, watch::PBT_APMRESUMESUSPEND);
        assert_eq!(next(&rx), Some(SoloEdge::Recovered));
        send(&[key(RIGHT_CTRL, false), key(RIGHT_CTRL, true)]);
        assert_eq!(quiet(&rx), None, "the hold across the recovery is let go");
        send(&[key(RIGHT_CTRL, false), key(RIGHT_CTRL, true)]);
        assert_eq!((next(&rx), next(&rx)), (Some(SoloEdge::Press), Some(SoloEdge::Release)));
        backend.stop();
    }

    /// docs/dictation.md §29: the physical state a hold-to-talk take is checked against. The side
    /// button the hook swallows reaches neither Raw Input nor the async key state, so the hook's
    /// own record says it is held; once the hook is gone, Raw Input sees the button again. A chord
    /// key reads through the async key state.
    #[test]
    #[ignore = "needs an interactive Windows desktop"]
    fn real_a_swallowed_side_button_and_a_chord_key_read_as_held() {
        let _serial = serial();
        let (tx, rx) = mpsc::channel();
        let backend = start(SoloKey::MouseBack, tx, true).expect("hooks");
        let back = HeldKey::Solo(SoloKey::MouseBack);
        assert_eq!(is_down(&back), Some(false));
        send(&[xbutton(XBUTTON1, false)]);
        assert_eq!(next(&rx), Some(SoloEdge::Press));
        wait_until("the swallowed button reads as down", || is_down(&back) == Some(true));
        send(&[xbutton(XBUTTON1, true)]);
        assert_eq!(next(&rx), Some(SoloEdge::Release));
        wait_until("the button reads as up", || is_down(&back) == Some(false));
        backend.force_unhook();
        send(&[xbutton(XBUTTON1, false)]);
        wait_until("Raw Input sees the button down past the removed hook", || is_down(&back) == Some(true));
        send(&[xbutton(XBUTTON1, true)]);
        wait_until("…and up", || is_down(&back) == Some(false));
        backend.stop();
        let f13 = HeldKey::Chord("F13".to_owned());
        send(&[key(CHORD_VK, false)]);
        wait_until("F13 reads as down", || is_down(&f13) == Some(true));
        send(&[key(CHORD_VK, true)]);
        wait_until("F13 reads as up", || is_down(&f13) == Some(false));
        assert_eq!(is_down(&HeldKey::Chord("Numpad1".to_owned())), None, "a key the table does not know is not read");
    }
}
