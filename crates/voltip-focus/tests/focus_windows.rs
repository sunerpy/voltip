//! The insert check's UI Automation reader against real Windows controls (docs/dictation.md §28):
//! a paste into Notepad is `written`; the same paste into a read-only EDIT of this test's own
//! window leaves it `not_written`; the focus moving to the window's other EDIT is `unconfirmed`.
//! It pastes the way Voltip does (the clipboard and Ctrl+V) into windows it brings to the front,
//! so it is ignored: CI runs it on the windows-2025 desktop (`hooks-windows`), never on a
//! computer someone is using:
//! `cargo test -p voltip-focus --test focus_windows -- --ignored --test-threads=1 --nocapture`.

// The test's own window is Win32 FFI (windows-sys); UI Automation's focus calls are COM.
#![allow(unsafe_code, clippy::unwrap_used, clippy::expect_used)]
#![cfg(windows)]

use std::process::{Child, Command};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use voltip_focus::{Expected, Reader, Settled, UnconfirmedWhy, Watch, windows as uia};
use voltip_inject::{Injector as _, PasteOptions};
use voltip_platform::HostOs;
use voltip_platform::insert_check::rules::InsertCheck;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
use windows::Win32::UI::Accessibility::{CUIAutomation8, IUIAutomation};
use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEINPUT, SendInput};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, ES_AUTOHSCROLL, ES_READONLY, FindWindowW, GetForegroundWindow, GetMessageW,
    GetWindowRect, GetWindowThreadProcessId, HWND_TOPMOST, MSG, PostThreadMessageW, RegisterClassW, SWP_NOMOVE, SWP_NOSIZE, SetCursorPos, SetWindowPos,
    TranslateMessage, WM_QUIT, WNDCLASSW, WS_BORDER, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

/// What is pasted: plain ASCII, so any control shows it as sent.
const TYPED: &str = "voltip check";
/// How long a paste is watched: well over `CONFIRM_WITHIN`, for a slow runner.
const WATCH: Duration = Duration::from_secs(3);
/// How long a window may take to come up or to the front.
const WAIT: Duration = Duration::from_secs(20);
/// The test's own window, as the probe would name it (any id outside the table of skipped
/// applications).
const OWN: &str = "focus_windows";
/// The test window's title.
const TITLE: &str = "voltip-focus-check";

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// UI Automation on the test thread, for the focus moves (`SetFocus` brings a window to the front
/// where a background process may not call `SetForegroundWindow` itself).
struct Focuser(IUIAutomation);

impl Focuser {
    fn new() -> Self {
        // SAFETY: COM for this thread; an apartment already set up answers S_FALSE.
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        // SAFETY: COM is initialised on this thread.
        Self(unsafe { CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER) }.expect("UI Automation"))
    }

    /// Give another application's `control` the keyboard focus, and wait until `top` is the window
    /// in front.
    fn focus(&self, control: isize, top: isize, what: &str) {
        // SAFETY: a live window handle; UI Automation's element and SetFocus.
        unsafe { self.0.ElementFromHandle(HWND(control as *mut _)).and_then(|element| element.SetFocus()) }.unwrap_or_else(|e| panic!("focus {what}: {e}"));
        wait_front(top, what);
    }
}

/// Wait until `top` is the window in front.
fn wait_front(top: isize, what: &str) {
    let until = Instant::now() + WAIT;
    // SAFETY: plain query.
    while unsafe { GetForegroundWindow() } as isize != top {
        assert!(Instant::now() < until, "{what} did not come to the front");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Click the middle of this test's own `control`, its window `top` raised above the others first:
/// a click brings a window to the front and gives the control the focus, where UI Automation's
/// SetFocus on a window of the calling process times out (CI 2026-10-29).
fn click(control: isize, top: isize, what: &str) {
    let mouse =
        |flags| INPUT { r#type: INPUT_MOUSE, Anonymous: INPUT_0 { mi: MOUSEINPUT { dx: 0, dy: 0, mouseData: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } } };
    let clicks = [mouse(MOUSEEVENTF_LEFTDOWN), mouse(MOUSEEVENTF_LEFTUP)];
    // SAFETY: live window handles of this process, a RECT on the stack, an array of INPUTs that
    // outlives the call.
    unsafe {
        SetWindowPos(top as *mut _, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
        let mut rect: RECT = std::mem::zeroed();
        assert!(GetWindowRect(control as *mut _, &raw mut rect) != 0, "the rectangle of {what}");
        SetCursorPos((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2);
        let sent = SendInput(2, clicks.as_ptr(), std::mem::size_of::<INPUT>() as i32);
        assert_eq!(sent, 2, "the click on {what}");
    }
    wait_front(top, what);
}

/// This test's window: a read-only EDIT showing 前文 and an editable one, on a thread of their own
/// with a message loop (UI Automation reads them through window messages).
struct OwnWindow {
    thread: u32,
    top: isize,
    readonly: isize,
    other: isize,
    join: Option<JoinHandle<()>>,
}

impl OwnWindow {
    fn open() -> Self {
        let (tx, rx) = mpsc::channel();
        let join = std::thread::spawn(move || {
            // SAFETY: plain Win32 window creation and a message loop on this thread; the strings
            // outlive the calls that read them.
            unsafe {
                let instance = GetModuleHandleW(std::ptr::null());
                let class = wide("VoltipFocusCheck");
                let mut wc: WNDCLASSW = std::mem::zeroed();
                wc.lpfnWndProc = Some(DefWindowProcW);
                wc.hInstance = instance;
                wc.lpszClassName = class.as_ptr();
                RegisterClassW(&raw const wc);
                let title = wide(TITLE);
                let top = CreateWindowExW(
                    0,
                    class.as_ptr(),
                    title.as_ptr(),
                    WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                    200,
                    200,
                    640,
                    180,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    instance,
                    std::ptr::null(),
                );
                let edit = wide("EDIT");
                let text = wide("前文");
                let style = WS_CHILD | WS_VISIBLE | WS_BORDER | ES_AUTOHSCROLL as u32;
                let readonly = CreateWindowExW(
                    0,
                    edit.as_ptr(),
                    text.as_ptr(),
                    style | ES_READONLY as u32,
                    10,
                    10,
                    600,
                    30,
                    top,
                    std::ptr::null_mut(),
                    instance,
                    std::ptr::null(),
                );
                let other = CreateWindowExW(0, edit.as_ptr(), text.as_ptr(), style, 10, 60, 600, 30, top, std::ptr::null_mut(), instance, std::ptr::null());
                tx.send((GetCurrentThreadId(), top as isize, readonly as isize, other as isize)).unwrap();
                let mut msg: MSG = std::mem::zeroed();
                while GetMessageW(&raw mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                    TranslateMessage(&raw const msg);
                    DispatchMessageW(&raw const msg);
                }
                DestroyWindow(top);
            }
        });
        let (thread, top, readonly, other) = rx.recv_timeout(WAIT).expect("the test window");
        assert!(top != 0 && readonly != 0 && other != 0, "CreateWindowExW failed");
        Self { thread, top, readonly, other, join: Some(join) }
    }
}

impl Drop for OwnWindow {
    fn drop(&mut self) {
        // SAFETY: the window thread's id; WM_QUIT ends its loop.
        unsafe { PostThreadMessageW(self.thread, WM_QUIT, 0, 0) };
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// Notepad, ended (unsaved) when the test ends: the process that owns its window (the one started
/// may hand over to another), then the one started.
struct Notepad {
    started: Child,
    window: isize,
}

impl Notepad {
    fn open() -> Self {
        // Dropped on every path, the window found or not: `Drop` ends and reaps the process.
        let mut notepad = Self { started: Command::new("notepad.exe").spawn().expect("notepad.exe"), window: 0 };
        let class = wide("Notepad");
        let until = Instant::now() + WAIT;
        while notepad.window == 0 {
            assert!(Instant::now() < until, "no Notepad window");
            std::thread::sleep(Duration::from_millis(100));
            // SAFETY: a class-name lookup.
            notepad.window = unsafe { FindWindowW(class.as_ptr(), std::ptr::null()) } as isize;
        }
        notepad
    }
}

impl Drop for Notepad {
    fn drop(&mut self) {
        let mut pid = 0u32;
        if self.window != 0 {
            // SAFETY: a window handle and an out-parameter on the stack.
            unsafe { GetWindowThreadProcessId(self.window as *mut _, &raw mut pid) };
        }
        if pid != 0 {
            let _ = Command::new("taskkill").args(["/F", "/PID", &pid.to_string()]).status();
        }
        let _ = self.started.kill();
        let _ = self.started.wait();
    }
}

/// Read the focused control until it answers (a window just brought up may not have focused its
/// field yet).
fn wait_watch(reader: &Reader, app_id: &str) -> Watch {
    let until = Instant::now() + WAIT;
    loop {
        match reader.watch(app_id, &Expected::of(TYPED)) {
            Ok(watch) => return watch,
            Err(why) => assert!(Instant::now() < until, "the focused control was never read: {why:?}"),
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Paste [`TYPED`] the way Voltip does, into the window that must be in front.
fn paste_into(top: isize) {
    // SAFETY: plain query.
    assert_eq!(unsafe { GetForegroundWindow() } as isize, top, "the paste would go elsewhere");
    voltip_inject::system_paste_injector(PasteOptions::for_os("windows")).inject(TYPED).expect("the paste went out");
}

#[test]
#[ignore = "pastes into windows it brings to the front: CI's windows-2025 desktop only"]
fn a_paste_lands_in_notepad_a_read_only_edit_stays_and_another_edit_takes_the_focus() {
    let focuser = Focuser::new();
    let reader = Reader::new(HostOs::Windows, uia::open);

    let notepad = Notepad::open();
    focuser.focus(notepad.window, notepad.window, "Notepad");
    let watch = wait_watch(&reader, "notepad");
    paste_into(notepad.window);
    assert_eq!(watch.settle(WATCH), Settled::sure(InsertCheck::Written), "Notepad");
    drop(notepad);

    let own = OwnWindow::open();
    click(own.readonly, own.top, "the read-only EDIT");
    let watch = wait_watch(&reader, OWN);
    paste_into(own.top);
    assert_eq!(watch.settle(WATCH), Settled::sure(InsertCheck::NotWritten), "the read-only EDIT");

    let watch = wait_watch(&reader, OWN);
    click(own.other, own.top, "the other EDIT");
    assert_eq!(watch.settle(WATCH), Settled::unconfirmed(UnconfirmedWhy::ControlChanged), "the focus moved");
}
