//! Win32 helpers of the Windows tests: windows of the test's own on threads that pump messages,
//! clicks, what is on top at a point of the screen, and the screen's pixels.

// Win32 FFI throughout; each test file uses a part of the helpers.
#![allow(unsafe_code, dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::fmt::Write as _;
use std::sync::{Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, S_OK, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows_sys::Win32::Graphics::Gdi::{CLR_INVALID, CreateSolidBrush, GetDC, GetPixel, ReleaseDC};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEINPUT, SendInput};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, EnumWindows, GA_ROOT, GUITHREADINFO, GWL_EXSTYLE, GetAncestor, GetClassNameW,
    GetForegroundWindow, GetGUIThreadInfo, GetMessageW, GetSystemMetrics, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId, HWND_NOTOPMOST, HWND_TOP,
    HWND_TOPMOST, IsHungAppWindow, IsIconic, IsWindowVisible, MSG, PostThreadMessageW, RegisterClassW, SM_CXSCREEN, SM_CYSCREEN, SMTO_ABORTIFHUNG, SMTO_BLOCK,
    SPI_GETWORKAREA, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SendMessageTimeoutW, SetCursorPos, SetWindowPos, ShowWindow,
    SystemParametersInfoW, TranslateMessage, WM_ACTIVATE, WM_ACTIVATEAPP, WM_CAPTURECHANGED, WM_KILLFOCUS, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE,
    WM_NCACTIVATE, WM_NCLBUTTONDOWN, WM_NULL, WM_QUIT, WM_SETCURSOR, WM_SETFOCUS, WNDCLASSW, WS_CLIPSIBLINGS, WS_EX_ACCEPTFILES, WS_EX_APPWINDOW,
    WS_EX_NOACTIVATE, WS_EX_TOPMOST, WS_EX_WINDOWEDGE, WS_OVERLAPPEDWINDOW, WS_POPUP, WindowFromPoint,
};

/// How long a window may take to come up, to the front or on top.
pub const WAIT: Duration = Duration::from_secs(5);

/// The pill's colour (`0x00BBGGRR`): green.
pub const PILL_COLOR: u32 = 0x0000_C000;
/// The colour of the windows that cover it: magenta.
pub const COVER_COLOR: u32 = 0x00FF_00FF;

/// The extended style tao gives Voltip's pill window (read off the 0.3.0 candidate on Windows
/// Server 2025, 2026-11-01: `0x08040118`): topmost, never activated, an app window (no owner),
/// and tao's window-edge and drop-target bits.
pub const PILL_EX_STYLE: u32 = WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_APPWINDOW | WS_EX_WINDOWEDGE | WS_EX_ACCEPTFILES;

/// The pill window's size and its distance from the work area's bottom edge (overlay.rs).
pub const PILL_SIZE: (i32, i32) = (480, 64);
const PILL_MARGIN: i32 = 24;

/// A message a window of the test received that tells what a click and an activation did to it,
/// for a failure message.
struct Received {
    at: Instant,
    hwnd: isize,
    msg: u32,
    wparam: usize,
    lparam: isize,
    result: isize,
}

/// What the test's windows received since the last [`TestWindow::activate`] began (at most
/// [`RECEIVED_KEPT`]).
static RECEIVED: Mutex<Vec<Received>> = Mutex::new(Vec::new());
const RECEIVED_KEPT: usize = 4096;

/// The test windows' window procedure: `DefWindowProcW`, with the messages of activation, focus,
/// capture and the click noted in [`RECEIVED`].
unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: the arguments Windows passed this window procedure.
    let result = unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    let click = msg == WM_SETCURSOR && (lparam as usize >> 16) & 0xffff == WM_LBUTTONDOWN as usize;
    let noted = click
        || matches!(
            msg,
            WM_ACTIVATE
                | WM_ACTIVATEAPP
                | WM_NCACTIVATE
                | WM_MOUSEACTIVATE
                | WM_SETFOCUS
                | WM_KILLFOCUS
                | WM_LBUTTONDOWN
                | WM_LBUTTONUP
                | WM_NCLBUTTONDOWN
                | WM_CAPTURECHANGED
        );
    if noted
        && let Ok(mut received) = RECEIVED.lock()
        && received.len() < RECEIVED_KEPT
    {
        received.push(Received { at: Instant::now(), hwnd: hwnd as isize, msg, wparam, lparam, result });
    }
    result
}

fn message_name(msg: u32) -> &'static str {
    match msg {
        WM_ACTIVATE => "WM_ACTIVATE",
        WM_ACTIVATEAPP => "WM_ACTIVATEAPP",
        WM_NCACTIVATE => "WM_NCACTIVATE",
        WM_MOUSEACTIVATE => "WM_MOUSEACTIVATE",
        WM_SETFOCUS => "WM_SETFOCUS",
        WM_KILLFOCUS => "WM_KILLFOCUS",
        WM_LBUTTONDOWN => "WM_LBUTTONDOWN",
        WM_LBUTTONUP => "WM_LBUTTONUP",
        WM_NCLBUTTONDOWN => "WM_NCLBUTTONDOWN",
        WM_CAPTURECHANGED => "WM_CAPTURECHANGED",
        WM_SETCURSOR => "WM_SETCURSOR(button down)",
        _ => "?",
    }
}

/// What the test's windows received since `since`, `this` first, for a failure message.
fn received_since(since: Instant, this: isize) -> String {
    let Ok(received) = RECEIVED.lock() else { return "\n  (the log is poisoned)".to_owned() };
    let mut text = String::new();
    for pass in [true, false] {
        for event in received.iter().filter(|event| event.at >= since && (event.hwnd == this) == pass) {
            let whose = if event.hwnd == this { "this window".to_owned() } else { format!("{:#x}", event.hwnd) };
            let _ = write!(
                text,
                "\n  +{} ms {whose}: {} wparam {:#x} lparam {:#x} -> {:#x}",
                event.at.duration_since(since).as_millis(),
                message_name(event.msg),
                event.wparam,
                event.lparam,
                event.result
            );
        }
    }
    if text.is_empty() { "\n  nothing".to_owned() } else { text }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// Physical pixels everywhere (the pill's rectangle, the cursor, the screen's pixels).
pub fn dpi_aware() {
    // SAFETY: plain call; a process whose awareness is already set gets an error, ignored.
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
}

/// The primary monitor's work area (the screen without the taskbar).
pub fn work_area() -> RECT {
    let mut area = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    // SAFETY: `area` is a RECT, which SPI_GETWORKAREA writes.
    unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, (&raw mut area).cast(), 0) };
    area
}

/// The primary monitor.
pub fn screen() -> RECT {
    // SAFETY: plain metric queries.
    unsafe { RECT { left: 0, top: 0, right: GetSystemMetrics(SM_CXSCREEN), bottom: GetSystemMetrics(SM_CYSCREEN) } }
}

/// Where Voltip puts the pill: centred, 24 px above the work area's bottom edge.
pub fn pill_rect() -> RECT {
    let area = work_area();
    let left = area.left + (area.right - area.left - PILL_SIZE.0) / 2;
    let top = area.bottom - PILL_SIZE.1 - PILL_MARGIN;
    RECT { left, top, right: left + PILL_SIZE.0, bottom: top + PILL_SIZE.1 }
}

pub fn center(rect: &RECT) -> POINT {
    POINT { x: (rect.left + rect.right) / 2, y: (rect.top + rect.bottom) / 2 }
}

/// How a test window is made.
#[derive(Clone, Copy)]
pub struct Spec {
    pub title: &'static str,
    pub ex_style: u32,
    pub style: u32,
    pub rect: RECT,
    pub color: u32,
    /// Shown (without activation) as soon as it exists.
    pub shown: bool,
    /// The window that owns it (a program's popup over its own window), or 0.
    pub owner: isize,
}

impl Spec {
    /// A window like Voltip's pill, hidden.
    pub fn pill() -> Self {
        Self {
            title: "voltip-overlay-pill",
            ex_style: PILL_EX_STYLE,
            style: WS_POPUP | WS_CLIPSIBLINGS,
            rect: pill_rect(),
            color: PILL_COLOR,
            shown: false,
            owner: 0,
        }
    }

    /// An ordinary application window over the whole work area.
    pub fn ordinary() -> Self {
        Self { title: "voltip-overlay-ordinary", ex_style: 0, style: WS_OVERLAPPEDWINDOW, rect: work_area(), color: COVER_COLOR, shown: true, owner: 0 }
    }

    /// A borderless window over the whole monitor, what a browser or a player in full screen is.
    pub fn full_screen() -> Self {
        Self { title: "voltip-overlay-full-screen", ex_style: 0, style: WS_POPUP, rect: screen(), color: COVER_COLOR, shown: true, owner: 0 }
    }

    /// The same window, also on top of the others (a game, a player's "always on top").
    pub fn topmost(self) -> Self {
        Self { ex_style: self.ex_style | WS_EX_TOPMOST, ..self }
    }

    /// A popup of `owner`'s over the pill's place, topmost (a program's dropdown or menu).
    pub fn popup_of(owner: &TestWindow) -> Self {
        let pill = pill_rect();
        let rect = RECT { left: pill.left - 40, top: pill.top - 40, right: pill.right + 40, bottom: pill.bottom + 40 };
        Self { title: "voltip-overlay-popup", ex_style: WS_EX_TOPMOST, style: WS_POPUP, rect, color: COVER_COLOR, shown: true, owner: owner.hwnd }
    }
}

/// A window of the test's own on a thread that pumps its messages; closed on drop.
pub struct TestWindow {
    pub hwnd: isize,
    thread: u32,
    join: Option<JoinHandle<()>>,
}

impl TestWindow {
    pub fn open(spec: Spec) -> Self {
        let (tx, rx) = mpsc::channel();
        let join = std::thread::spawn(move || {
            // SAFETY: plain Win32 window creation and a message loop on this thread; the strings
            // outlive the calls that read them.
            unsafe {
                let instance = GetModuleHandleW(std::ptr::null());
                let class = wide(&format!("VoltipOverlayTest{:06x}", spec.color));
                let mut wc: WNDCLASSW = std::mem::zeroed();
                wc.lpfnWndProc = Some(window_proc);
                wc.hInstance = instance;
                wc.hbrBackground = CreateSolidBrush(spec.color);
                wc.lpszClassName = class.as_ptr();
                // A class registered by an earlier test answers 0 here and stays usable.
                RegisterClassW(&raw const wc);
                let title = wide(spec.title);
                let RECT { left, top, right, bottom } = spec.rect;
                let hwnd = CreateWindowExW(
                    spec.ex_style,
                    class.as_ptr(),
                    title.as_ptr(),
                    spec.style,
                    left,
                    top,
                    right - left,
                    bottom - top,
                    spec.owner as HWND,
                    std::ptr::null_mut(),
                    instance,
                    std::ptr::null(),
                );
                if spec.shown {
                    ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                }
                tx.send((GetCurrentThreadId(), hwnd as isize)).unwrap();
                let mut msg: MSG = std::mem::zeroed();
                while GetMessageW(&raw mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                    TranslateMessage(&raw const msg);
                    DispatchMessageW(&raw const msg);
                }
                DestroyWindow(hwnd);
            }
        });
        let (thread, hwnd) = rx.recv_timeout(WAIT).expect("the test window");
        assert!(hwnd != 0, "CreateWindowExW failed for {}", spec.title);
        Self { hwnd, thread, join: Some(join) }
    }

    /// Show it the way tao shows Voltip's pill (`ShowWindow(SW_SHOWNOACTIVATE)`; the shell's
    /// `set_always_on_top(true)` after it changes nothing while the window already is).
    pub fn show_like_tao(&self) {
        // SAFETY: a live window handle of this test.
        unsafe { ShowWindow(self.hwnd as HWND, SW_SHOWNOACTIVATE) };
    }

    pub fn hide(&self) {
        // SAFETY: a live window handle of this test.
        unsafe { ShowWindow(self.hwnd as HWND, SW_HIDE) };
    }

    /// Pin it on top where it is, without activating it, as a player's "always on top" button or
    /// PowerToys' Always On Top does to the window in front, and wait until it is topmost.
    pub fn pin(&self) {
        // SAFETY: a live window handle of this test.
        unsafe { SetWindowPos(self.hwnd as HWND, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) };
        assert!(wait_until(WAIT, || is_topmost(self.hwnd)), "the window could not be pinned on top");
    }

    /// Take it out of the topmost windows again, without activating it, and wait until it is.
    pub fn unpin(&self) {
        // SAFETY: a live window handle of this test.
        unsafe { SetWindowPos(self.hwnd as HWND, HWND_NOTOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) };
        assert!(wait_until(WAIT, || !is_topmost(self.hwnd)), "the window could not be unpinned");
    }

    pub fn rect(&self) -> RECT {
        rect_of(self.hwnd)
    }

    /// A point of this window that the pill does not cover, for a click that activates it.
    pub fn grip(&self) -> POINT {
        let rect = self.rect();
        POINT { x: rect.left + 40, y: rect.top + 40 }
    }

    /// Activate it with a click, as the user would, and wait until it is the window in front.
    pub fn activate(&self, what: &str) {
        let at = self.grip();
        // Read just before the click: the window in front, and the one the click goes to.
        let before = foreground();
        let landed = root_at(at);
        let stack = stack_at(self.hwnd, at);
        if let Ok(mut received) = RECEIVED.lock() {
            received.clear();
        }
        let clicked = Instant::now();
        click(at);
        // The window in front after the click, each time it changed.
        let mut seen: Vec<(Duration, isize)> = Vec::new();
        let came = wait_until(WAIT, || {
            let now = foreground();
            if seen.last().is_none_or(|&(_, last)| last != now) {
                seen.push((clicked.elapsed(), now));
            }
            now == self.hwnd
        });
        if came {
            let took = clicked.elapsed();
            if took > Duration::from_millis(500) {
                println!("{what}: in front {} ms after the click (in front before it: {})", took.as_millis(), describe(before));
            }
            return;
        }
        let mut changes = String::new();
        for (when, hwnd) in &seen {
            let _ = write!(changes, "\n  +{} ms {}", when.as_millis(), describe_for(*hwnd, self.hwnd));
        }
        let received = received_since(clicked, self.hwnd);
        let position = |hwnd: isize| z_order().iter().position(|&each| each == hwnd).map_or_else(|| "-".to_owned(), |at| at.to_string());
        let orders = format!("this window {}, the window in front before {}, of {}", position(self.hwnd), position(before), z_order().len());
        let threads = format!("this window's thread: {}; the thread of the window in front before: {}", thread_state(self.hwnd), thread_state(before));
        let answers = format!("this window {}; the window in front before {}", answers(self.hwnd), answers(before));
        let front = describe_for(foreground(), self.hwnd);
        let front_thread = foreground_thread();
        // Whether a raise without activation would have put this window under the click point.
        // SAFETY: a live window handle of this test.
        unsafe { SetWindowPos(self.hwnd as HWND, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) };
        let raised = root_at(at);
        panic!(
            "{what} did not come to the front (in front: {}; the foreground thread: {}). The click at {},{} went to {} (in front before it: {}); the windows there, top first:{stack}\n in front after the click:{changes}\n received since the click:{received}\n Z order: {orders}\n {threads}\n {answers}\n raised without activation now, the click point goes to {}",
            front,
            front_thread,
            at.x,
            at.y,
            describe_for(landed, self.hwnd),
            describe_for(before, self.hwnd),
            describe_for(raised, self.hwnd),
        );
    }
}

impl Drop for TestWindow {
    fn drop(&mut self) {
        // SAFETY: the window thread's id; WM_QUIT ends its loop, which destroys the window.
        unsafe { PostThreadMessageW(self.thread, WM_QUIT, 0, 0) };
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// A left click at `at`.
pub fn click(at: POINT) {
    let mouse =
        |flags| INPUT { r#type: INPUT_MOUSE, Anonymous: INPUT_0 { mi: MOUSEINPUT { dx: 0, dy: 0, mouseData: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } } };
    let clicks = [mouse(MOUSEEVENTF_LEFTDOWN), mouse(MOUSEEVENTF_LEFTUP)];
    // SAFETY: an array of INPUTs that outlives the call.
    let sent = unsafe {
        SetCursorPos(at.x, at.y);
        SendInput(2, clicks.as_ptr(), i32::try_from(size_of::<INPUT>()).unwrap())
    };
    assert_eq!(sent, 2, "the click at {},{}", at.x, at.y);
}

pub fn foreground() -> isize {
    // SAFETY: plain query.
    unsafe { GetForegroundWindow() as isize }
}

/// What the foreground thread holds, for a failure message: its active, focus and capture windows
/// and its flags (in a menu, in a move or size loop), which say more when no window is in front.
pub fn foreground_thread() -> String {
    let mut info = GUITHREADINFO { cbSize: u32::try_from(size_of::<GUITHREADINFO>()).unwrap(), ..GUITHREADINFO::default() };
    // SAFETY: `info` is a GUITHREADINFO with its size set; thread 0 is the foreground thread.
    if unsafe { GetGUIThreadInfo(0, &raw mut info) } == 0 {
        return format!("unknown ({})", std::io::Error::last_os_error());
    }
    format!(
        "active {}, focus {}, capture {}, flags {:#x}",
        describe(info.hwndActive as isize),
        describe(info.hwndFocus as isize),
        describe(info.hwndCapture as isize),
        info.flags
    )
}

/// What the thread of `hwnd` holds (`GetGUIThreadInfo`), for a failure message.
pub fn thread_state(hwnd: isize) -> String {
    if hwnd == 0 {
        return "no window".to_owned();
    }
    // SAFETY: a window handle; a stale one answers thread 0.
    let thread = unsafe { GetWindowThreadProcessId(hwnd as HWND, std::ptr::null_mut()) };
    let mut info = GUITHREADINFO { cbSize: u32::try_from(size_of::<GUITHREADINFO>()).unwrap(), ..GUITHREADINFO::default() };
    // SAFETY: `info` is a GUITHREADINFO with its size set.
    if thread == 0 || unsafe { GetGUIThreadInfo(thread, &raw mut info) } == 0 {
        return format!("unknown ({})", std::io::Error::last_os_error());
    }
    format!(
        "active {}, focus {}, capture {}, flags {:#x}",
        describe(info.hwndActive as isize),
        describe(info.hwndFocus as isize),
        describe(info.hwndCapture as isize),
        info.flags
    )
}

/// Whether the thread of `hwnd` answers a message (within a second), and how soon, for a failure
/// message.
pub fn answers(hwnd: isize) -> String {
    if hwnd == 0 {
        return "is no window".to_owned();
    }
    // SAFETY: plain queries on a window handle; `result` outlives the call.
    let (hung, answered, took) = unsafe {
        let hung = IsHungAppWindow(hwnd as HWND) != 0;
        let started = Instant::now();
        let mut result = 0usize;
        let answered = SendMessageTimeoutW(hwnd as HWND, WM_NULL, 0, 0, SMTO_ABORTIFHUNG | SMTO_BLOCK, 1000, &raw mut result) != 0;
        (hung, answered, started.elapsed())
    };
    format!("{} in {} ms (hung: {hung})", if answered { "answers" } else { "does not answer" }, took.as_millis())
}

/// Wait until `done` holds, at most `within`; whether it did.
pub fn wait_until(within: Duration, mut done: impl FnMut() -> bool) -> bool {
    let until = Instant::now() + within;
    loop {
        if done() {
            return true;
        }
        if Instant::now() >= until {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn rect_of(hwnd: isize) -> RECT {
    let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    // SAFETY: a window handle and a RECT on the stack; a stale handle leaves it empty.
    unsafe { GetWindowRect(hwnd as HWND, &raw mut rect) };
    rect
}

pub fn ex_style(hwnd: isize) -> u32 {
    // SAFETY: a window handle; a stale one answers 0.
    unsafe { GetWindowLongW(hwnd as HWND, GWL_EXSTYLE).cast_unsigned() }
}

pub fn is_topmost(hwnd: isize) -> bool {
    ex_style(hwnd) & WS_EX_TOPMOST != 0
}

pub fn cloaked(hwnd: isize) -> bool {
    let mut cloak = 0u32;
    // SAFETY: a window handle and a u32 of the size passed.
    unsafe { DwmGetWindowAttribute(hwnd as HWND, DWMWA_CLOAKED.cast_unsigned(), (&raw mut cloak).cast(), 4) == S_OK && cloak != 0 }
}

/// Every top-level window, in Z order (the one on top first).
pub fn z_order() -> Vec<isize> {
    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> i32 {
        // SAFETY: `lparam` is the Vec handed to EnumWindows below, alive for the whole call.
        unsafe { (*(lparam as *mut Vec<isize>)).push(hwnd as isize) };
        1
    }
    let mut list: Vec<isize> = Vec::new();
    // SAFETY: the callback only pushes into `list`, which outlives the call.
    unsafe { EnumWindows(Some(collect), (&raw mut list) as LPARAM) };
    list
}

fn shows_at(hwnd: isize, at: POINT) -> bool {
    let rect = rect_of(hwnd);
    // SAFETY: plain queries on a window handle.
    let (visible, iconic) = unsafe { (IsWindowVisible(hwnd as HWND) != 0, IsIconic(hwnd as HWND) != 0) };
    visible && !iconic && !cloaked(hwnd) && rect.left <= at.x && at.x < rect.right && rect.top <= at.y && at.y < rect.bottom
}

/// The window the user sees at `at`: the first visible, uncloaked window in Z order there.
pub fn top_at(at: POINT) -> isize {
    z_order().into_iter().find(|&hwnd| shows_at(hwnd, at)).unwrap_or(0)
}

/// The top-level window a click at `at` goes to: the root of the window Windows finds there
/// (`WindowFromPoint`, which also skips hidden and click-through windows), or 0.
pub fn root_at(at: POINT) -> isize {
    // SAFETY: plain queries; a point over no window answers null.
    unsafe {
        let hwnd = WindowFromPoint(at);
        if hwnd.is_null() { 0 } else { GetAncestor(hwnd, GA_ROOT) as isize }
    }
}

/// The windows that show at `at` in Z order, the one on top first, down to `target`, for a failure
/// message (every one there when `target` is not).
pub fn stack_at(target: isize, at: POINT) -> String {
    let mut text = String::new();
    for hwnd in z_order() {
        if shows_at(hwnd, at) {
            let _ = write!(text, "\n  {}", describe_for(hwnd, target));
            if hwnd == target {
                break;
            }
        }
    }
    if text.is_empty() { "\n  no window".to_owned() } else { text }
}

/// The screen's colour at `at` as the desktop composes it (`0x00BBGGRR`).
pub fn pixel(at: POINT) -> u32 {
    // SAFETY: the screen's device context, released after the one read.
    unsafe {
        let dc = GetDC(std::ptr::null_mut());
        let color = GetPixel(dc, at.x, at.y);
        ReleaseDC(std::ptr::null_mut(), dc);
        color
    }
}

/// Whether two colours are the same but for rounding.
pub fn same_color(a: u32, b: u32) -> bool {
    a != CLR_INVALID && (0..3).all(|shift| ((a >> (shift * 8)) & 0xff).abs_diff((b >> (shift * 8)) & 0xff) <= 8)
}

/// One window, for a failure message: class, process, top-most, rectangle (never its title).
pub fn describe(hwnd: isize) -> String {
    if hwnd == 0 {
        return "no window".to_owned();
    }
    let mut class = [0u16; 128];
    let mut pid = 0u32;
    // SAFETY: a window handle, a buffer of the length passed, and an out-parameter on the stack.
    let n = unsafe {
        GetWindowThreadProcessId(hwnd as HWND, &raw mut pid);
        GetClassNameW(hwnd as HWND, class.as_mut_ptr(), 128)
    };
    let class = String::from_utf16_lossy(&class[..usize::try_from(n).unwrap_or(0).min(128)]);
    let rect = rect_of(hwnd);
    format!("{class} (pid {pid}, topmost {}, ex {:#010x}, {},{}-{},{})", is_topmost(hwnd), ex_style(hwnd), rect.left, rect.top, rect.right, rect.bottom)
}

/// [`describe`], with `this` named as the window being tested.
pub fn describe_for(hwnd: isize, this: isize) -> String {
    if hwnd != 0 && hwnd == this { format!("this window, {}", describe(hwnd)) } else { describe(hwnd) }
}

/// The windows above `target` in Z order that show at `at`, for a failure message.
pub fn above(target: isize, at: POINT) -> String {
    let mut text = String::new();
    for hwnd in z_order().into_iter().take_while(|&hwnd| hwnd != target) {
        if shows_at(hwnd, at) {
            let _ = write!(text, "\n  above: {}", describe(hwnd));
        }
    }
    if text.is_empty() { "\n  nothing above".to_owned() } else { text }
}

/// Wait until the pill is what the user sees at its centre, by the Z order and by the screen's
/// pixels; on a timeout, panic with who is in the way.
pub fn assert_pill_on_top(pill: isize, what: &str) {
    let at = center(&rect_of(pill));
    let on_top = wait_until(WAIT, || top_at(at) == pill);
    assert!(on_top, "{what}: the pill is not on top. On top: {}{}", describe(top_at(at)), above(pill, at));
    let painted = wait_until(WAIT, || same_color(pixel(at), PILL_COLOR));
    assert!(painted, "{what}: the pill is on top in the Z order, but the screen shows {:#08x} at its centre", pixel(at));
}
