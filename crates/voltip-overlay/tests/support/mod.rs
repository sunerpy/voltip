//! Win32 helpers of the Windows tests: windows of the test's own on threads that pump messages,
//! clicks, what is on top at a point of the screen, and the screen's pixels.

// Win32 FFI throughout; each test file uses a part of the helpers.
#![allow(unsafe_code, dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::fmt::Write as _;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, S_OK, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows_sys::Win32::Graphics::Gdi::{CLR_INVALID, ClientToScreen, CreateSolidBrush, GetDC, GetPixel, ReleaseDC, ScreenToClient};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEINPUT, SendInput};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, EnumWindows, GA_ROOT, GUITHREADINFO, GWL_EXSTYLE, GetAncestor, GetClassNameW,
    GetClientRect, GetCursorPos, GetForegroundWindow, GetGUIThreadInfo, GetMessageW, GetSystemMetrics, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId,
    HWND_NOTOPMOST, HWND_TOP, HWND_TOPMOST, IsHungAppWindow, IsIconic, IsWindowVisible, MSG, PostThreadMessageW, RegisterClassW, SM_CXSCREEN, SM_CYSCREEN,
    SMTO_ABORTIFHUNG, SMTO_BLOCK, SPI_GETWORKAREA, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SendMessageTimeoutW, SetCursorPos,
    SetWindowPos, ShowWindow, SystemParametersInfoW, TranslateMessage, WM_NULL, WM_QUIT, WNDCLASSW, WS_CLIPSIBLINGS, WS_EX_ACCEPTFILES, WS_EX_APPWINDOW,
    WS_EX_NOACTIVATE, WS_EX_TOPMOST, WS_EX_WINDOWEDGE, WS_OVERLAPPEDWINDOW, WS_POPUP, WindowFromPoint,
};

mod watch;

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

/// The test windows' window procedure: `DefWindowProcW`, with what a click, an activation and the
/// window's message loop did noted for a failure message ([`watch`]).
unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == watch::PROBE_TRACE {
        watch::trace_here();
        watch::note(hwnd, msg, wparam, lparam, 1);
        return 1;
    }
    // SAFETY: the arguments Windows passed this window procedure.
    let result = unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    watch::note(hwnd, msg, wparam, lparam, result);
    result
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

    /// A point where a click goes to this window ([`root_at`]): its grip; else, with another window
    /// over the grip, the grip once this one is raised to the top of its band without being
    /// activated; else the first point of its client area that nothing covers (a topmost window
    /// over an ordinary one). `None` when there is none.
    fn reachable(&self, what: &str) -> Option<POINT> {
        let grip = self.grip();
        let over = root_at(grip);
        if over == self.hwnd {
            return Some(grip);
        }
        // SAFETY: a live window handle of this test.
        unsafe { SetWindowPos(self.hwnd as HWND, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) };
        let still = root_at(grip);
        if still == self.hwnd {
            println!("{what}: raised without activation over {} at its grip {},{}", describe(over), grip.x, grip.y);
            return Some(grip);
        }
        let open = self.open_point();
        if let Some(at) = open {
            println!("{what}: {} stays over its grip {},{} once it is raised; the click goes to {},{}", describe(still), grip.x, grip.y, at.x, at.y);
        }
        open
    }

    /// The first point of its client area, on a 16 px grid 8 px inside the edges and the top row
    /// first, where a click goes to this window.
    fn open_point(&self) -> Option<POINT> {
        let client = client_rect(self.hwnd);
        (client.top + 8..client.bottom - 8)
            .step_by(16)
            .flat_map(|y| (client.left + 8..client.right - 8).step_by(16).map(move |x| POINT { x, y }))
            .find(|&at| root_at(at) == self.hwnd)
    }

    /// Move the pointer to `at`, as a user does before a click, and wait until this window receives
    /// that move, at `at` itself: the system then routes the pointer's input at that point to it.
    /// Each 100 ms it has not, the pointer moves to `at` again (by way of the pixel right of it when
    /// it is there already, where a move to `at` reports nothing), so that there is a new move to
    /// route. It returns with the pointer at `at`: how long it took and how many moves to `at`, or
    /// `None` when none had reached it by `until`.
    fn hover(&self, at: POINT, until: Instant) -> Option<(Duration, u32)> {
        let started = Instant::now();
        let aside = POINT { x: at.x + 1, y: at.y };
        let client = [client_point(self.hwnd, at)];
        let mut moves = 0;
        loop {
            let moved = Instant::now();
            // SAFETY: plain calls.
            unsafe {
                if same_point(cursor(), at) {
                    SetCursorPos(aside.x, aside.y);
                }
                SetCursorPos(at.x, at.y);
            }
            moves += 1;
            let left = until.saturating_duration_since(Instant::now()).min(Duration::from_millis(100));
            if wait_until(left, || watch::moved_over(self.hwnd, moved, &client, &[at])) {
                return Some((started.elapsed(), moves));
            }
            if Instant::now() >= until {
                return None;
            }
        }
    }

    /// Activate it with a click, as the user would, and wait until it is the window in front. The
    /// click goes to a point where this window is what a click reaches ([`TestWindow::reachable`]),
    /// once the pointer's move to that point has reached it ([`TestWindow::hover`]), and only when,
    /// read right before the click, the pointer is still there and a click there still goes to this
    /// window; a point that no longer is is chosen and hovered again, until [`WAIT`] has passed.
    /// Seen on windows-2025 (CI, 2026-11-07/08): a click that went elsewhere left the window under
    /// it where it was, with the runner's Windows Terminal or no window at all in front.
    pub fn activate(&self, what: &str) {
        watch::watch();
        let before = foreground();
        // The checks before the click share one deadline: the one for coming to the front starts
        // with the click.
        let started = Instant::now();
        let until = started + WAIT;
        let (at, moment) = loop {
            let Some(at) = self.reachable(what) else {
                let grip = self.grip();
                panic!(
                    "{what}: a click reaches it nowhere (in front: {}); the windows at its grip {},{}, top first:{}",
                    describe(before),
                    grip.x,
                    grip.y,
                    stack_at(self.hwnd, grip)
                );
            };
            let chosen = Moment::now(self.hwnd, at);
            let Some((hovered, moves)) = self.hover(at, until) else {
                panic!(
                    "{what}: a move of the pointer to {},{} did not reach it within {} ms (in front: {}; the foreground thread: {}); a click there would go to {}{}",
                    at.x,
                    at.y,
                    WAIT.as_millis(),
                    describe_for(foreground(), self.hwnd),
                    foreground_thread(),
                    describe_for(root_at(at), self.hwnd),
                    self.account(&chosen, &[])
                );
            };
            // A move that never reached it is what a click right after it shows meets (the
            // evidence for the wait: CI's logs list these).
            if moves > 1 {
                println!("{what}: the pointer's move to {},{} reached it after {} ms, at move {moves}", at.x, at.y, hovered.as_millis());
            }
            // Read just before the click: the window in front, the Z order, what the foreground
            // thread holds and, last, the window a click at `at` goes to.
            let moment = Moment::now(self.hwnd, at);
            let pointer = cursor();
            if moment.landed == self.hwnd && same_point(pointer, at) {
                break (at, moment);
            }
            let changed = format!(
                "right before the click at {},{}, a click there goes to {} and the pointer is at {},{}",
                at.x,
                at.y,
                describe_for(moment.landed, self.hwnd),
                pointer.x,
                pointer.y
            );
            assert!(Instant::now() < until, "{what}: {changed}, {} ms after the first check{}", started.elapsed().as_millis(), self.account(&moment, &[]));
            println!("{what}: {changed}; the point is chosen again");
        };
        press();
        // The window in front after the click, each time it changed.
        let mut seen: Vec<(Duration, isize)> = Vec::new();
        let came = wait_until(WAIT, || {
            let now = foreground();
            if seen.last().is_none_or(|&(_, last)| last != now) {
                seen.push((moment.when.elapsed(), now));
            }
            now == self.hwnd
        });
        if came {
            let took = moment.when.elapsed();
            if took > Duration::from_millis(500) {
                println!("{what}: in front {} ms after the click (in front before it: {})", took.as_millis(), describe(moment.front));
            }
            return;
        }
        panic!(
            "{what} did not come to the front (in front: {}; the foreground thread: {}). The click at {},{} went to {} (in front before it: {}){}",
            describe_for(foreground(), self.hwnd),
            foreground_thread(),
            at.x,
            at.y,
            describe_for(moment.landed, self.hwnd),
            describe_for(moment.front, self.hwnd),
            self.account(&moment, &seen)
        );
    }

    /// For a failure message: the windows at the moment's point, the Z order and what the threads
    /// held then and now, whether this window's thread pumps its messages and where it is, and the
    /// messages and WinEvents from a second before the moment on. `seen` is the window in front
    /// after it, each time it changed.
    fn account(&self, moment: &Moment, seen: &[(Duration, isize)]) -> String {
        let at = moment.at;
        let mut changes = String::new();
        for (when, hwnd) in seen {
            let _ = write!(changes, "\n  +{} ms {}", when.as_millis(), describe_for(*hwnd, self.hwnd));
        }
        if changes.is_empty() {
            changes.push_str("\n  (not watched)");
        }
        let positions = |order: &[isize]| {
            let position = |hwnd: isize| order.iter().position(|&each| each == hwnd).map_or_else(|| "-".to_owned(), |at| at.to_string());
            format!("this window {}, the window in front before {}, of {}", position(self.hwnd), position(moment.front), order.len())
        };
        let order_now = positions(&z_order());
        let threads = format!("this window's thread: {}; the thread of the window in front before: {}", thread_state(self.hwnd), thread_state(moment.front));
        let answers = format!("this window {}; the window in front before {}", answers(self.hwnd), answers(moment.front));
        let posted = watch::posted_probe(self.hwnd, Duration::from_secs(1));
        let trace = watch::stack_probe(self.hwnd);
        let cursor = cursor();
        let since = moment.when.checked_sub(Duration::from_secs(1)).unwrap_or(moment.when);
        let received = watch::messages_since(since, moment.when, self.hwnd);
        let events = watch::events_since(since, moment.when);
        format!(
            "; the windows at {},{} then, top first:{}\n then: Z order {}; the foreground thread: {}\n in front after:{changes}\n now: Z order {order_now}; the cursor at {},{}\n {threads}\n {answers}\n this window's thread {posted}\n this window's thread when a message sent to it arrives:{trace}\n received from 1 s before on:{received}\n WinEvents from 1 s before on:{events}",
            at.x,
            at.y,
            moment.stack,
            positions(&moment.order),
            moment.front_thread,
            cursor.x,
            cursor.y,
        )
    }
}

/// What the screen and the foreground thread were like at a point, right before a click there,
/// for a failure message.
struct Moment {
    at: POINT,
    when: Instant,
    /// The window in front.
    front: isize,
    /// The window a click at `at` goes to ([`root_at`]).
    landed: isize,
    /// The windows that show at `at`, top first, down to the target ([`stack_at`]).
    stack: String,
    /// Every top-level window, in Z order.
    order: Vec<isize>,
    /// What the foreground thread held ([`foreground_thread`]).
    front_thread: String,
}

impl Moment {
    /// Read now; the window a click at `at` goes to last, closest to a click after it.
    fn now(target: isize, at: POINT) -> Self {
        let front = foreground();
        let stack = stack_at(target, at);
        let order = z_order();
        let front_thread = foreground_thread();
        let landed = root_at(at);
        Self { at, when: Instant::now(), front, landed, stack, order, front_thread }
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

/// A left click where the pointer is, without moving it.
pub fn press() {
    let mouse =
        |flags| INPUT { r#type: INPUT_MOUSE, Anonymous: INPUT_0 { mi: MOUSEINPUT { dx: 0, dy: 0, mouseData: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } } };
    let clicks = [mouse(MOUSEEVENTF_LEFTDOWN), mouse(MOUSEEVENTF_LEFTUP)];
    // SAFETY: an array of INPUTs that outlives the call.
    let sent = unsafe { SendInput(2, clicks.as_ptr(), i32::try_from(size_of::<INPUT>()).unwrap()) };
    let at = cursor();
    assert_eq!(sent, 2, "the click at {},{}", at.x, at.y);
}

/// Whether two points are the same (`POINT` has no `PartialEq`).
pub fn same_point(a: POINT, b: POINT) -> bool {
    a.x == b.x && a.y == b.y
}

/// Where the cursor is.
pub fn cursor() -> POINT {
    let mut at = POINT { x: 0, y: 0 };
    // SAFETY: a POINT on the stack.
    unsafe { GetCursorPos(&raw mut at) };
    at
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

/// The window's client area in screen coordinates.
pub fn client_rect(hwnd: isize) -> RECT {
    let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    let mut origin = POINT { x: 0, y: 0 };
    // SAFETY: a window handle, a RECT and a POINT on the stack; a stale handle leaves them empty.
    unsafe {
        GetClientRect(hwnd as HWND, &raw mut rect);
        ClientToScreen(hwnd as HWND, &raw mut origin);
    }
    RECT { left: origin.x, top: origin.y, right: origin.x + rect.right, bottom: origin.y + rect.bottom }
}

/// A point of the screen in the window's client coordinates.
pub fn client_point(hwnd: isize, at: POINT) -> POINT {
    let mut point = at;
    // SAFETY: a window handle and a POINT on the stack; a stale handle leaves it as it was.
    unsafe { ScreenToClient(hwnd as HWND, &raw mut point) };
    point
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

/// One window, for a failure message: class, handle, process, top-most, rectangle (never its title).
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
    format!(
        "{class} {hwnd:#x} (pid {pid}, topmost {}, ex {:#010x}, {},{}-{},{})",
        is_topmost(hwnd),
        ex_style(hwnd),
        rect.left,
        rect.top,
        rect.right,
        rect.bottom
    )
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
