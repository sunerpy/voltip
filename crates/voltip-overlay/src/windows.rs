//! Windows (docs/dictation.md §32). The pill is a `WS_EX_TOPMOST | WS_EX_NOACTIVATE` window (tao
//! builds it from the shell's `always_on_top(true)` and `focusable(false)`). Topmost windows are
//! stacked among themselves by which came to the front last, a window shown again with
//! `SW_SHOWNOACTIVATE` keeps its old place, and tao's `set_always_on_top(true)` does nothing while
//! the flag is already set, so a topmost window activated since (a full-screen player, a game, a
//! pinned window) stays over the pill. [`raise`] puts the pill back at the top itself, and
//! [`Keeper`] does it again while the pill shows: whenever another window comes to the front, and
//! whenever a window appears or the stacking order changes and a window now covers the pill
//! ([`crate::contest`] says when a window that comes straight back on top is looked at again
//! later instead).
//!
//! `HWND_TOPMOST` also makes a window topmost again when another program took that away (placed it
//! after an ordinary window or said `HWND_NOTOPMOST`). Nothing here activates the pill or changes
//! the window in front: the paste goes to that window.

#![allow(unsafe_code)]

use std::cell::{Cell, RefCell};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{HWND, RECT, S_OK};
use windows_sys::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CHILDID_SELF, DispatchMessageW, EVENT_OBJECT_REORDER, EVENT_OBJECT_SHOW, EVENT_SYSTEM_FOREGROUND, GW_HWNDPREV, GW_OWNER, GetClassNameW, GetMessageW,
    GetWindow, GetWindowRect, HWND_TOPMOST, IsIconic, IsWindowVisible, KillTimer, MSG, OBJID_WINDOW, PostThreadMessageW, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SetTimer, SetWindowPos, TranslateMessage, WINEVENT_OUTOFCONTEXT, WM_QUIT,
};

use crate::contest::{Answer, Contest};

/// How long the keeper's thread may take to install its hooks.
const START_TIMEOUT: Duration = Duration::from_secs(5);
/// How many windows above the pill the keeper looks at for one that covers it.
const MAX_ABOVE: usize = 1024;

/// Put `hwnd` at the top of the topmost windows without activating, moving or sizing it.
/// `SWP_ASYNCWINDOWPOS`: called from another thread than the window's own, the request is posted to
/// that thread and never waits for it. `false` when Windows refused (a window that is gone).
pub fn raise(hwnd: isize) -> bool {
    // SAFETY: plain Win32 call on a window handle; a stale handle makes it fail (0).
    unsafe { SetWindowPos(hwnd as HWND, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS) != 0 }
}

thread_local! {
    /// The window the keeper on this thread raises.
    static PILL: Cell<isize> = const { Cell::new(0) };
    /// When it raises the pill over a window that covers it ([`Contest`]).
    static CONTEST: RefCell<Contest> = RefCell::new(Contest::default());
    /// The thread timer set to look again at the pill ([`Answer::LookAgainIn`]); 0 when none is.
    static LOOK_AGAIN: Cell<usize> = const { Cell::new(0) };
}

/// The keeper's hook: another window came to the front, so it is now at the top of its band (the
/// topmost windows, if it is one); or a window appeared or the stacking order changed, which may
/// have put a window over the pill (one pinned on top, a topmost window shown without activation).
unsafe extern "system" fn on_event(_hook: HWINEVENTHOOK, event: u32, hwnd: HWND, object: i32, child: i32, _thread: u32, _time: u32) {
    let pill = PILL.get();
    if pill == 0 {
        return;
    }
    match event {
        EVENT_SYSTEM_FOREGROUND => {
            CONTEST.with_borrow_mut(Contest::front);
            // The taskbar in front is the user working in it (an auto-hidden one slides over the
            // pill's place): the pill goes back on top with the next window in front.
            if !class_is(hwnd, TASKBARS) {
                raise_logged(pill);
            }
        }
        EVENT_OBJECT_SHOW | EVENT_OBJECT_REORDER => {
            if event == EVENT_OBJECT_SHOW && (object != OBJID_WINDOW || child != CHILDID_SELF.cast_signed() || hwnd as isize == pill) {
                return;
            }
            raise_over_cover(pill);
        }
        _ => {}
    }
}

/// A window may cover the pill: raise the pill over it, or look again later ([`Contest`]).
fn raise_over_cover(pill: isize) {
    let Some(cover) = covering(pill) else { return };
    match CONTEST.with_borrow_mut(|contest| contest.raise_over(cover, Instant::now())) {
        Answer::Raise => raise_logged(pill),
        Answer::LookAgainIn(wait) => look_again_in(wait),
        Answer::Leave => {}
    }
}

/// Have the message loop call [`on_look_again`] after `wait`. A timer already set stays: it is
/// due no later, and looking again then answers anew.
fn look_again_in(wait: Duration) {
    if LOOK_AGAIN.get() != 0 {
        return;
    }
    let ms = u32::try_from(wait.as_millis()).unwrap_or(u32::MAX).saturating_add(1);
    // SAFETY: a thread timer (no window) with a callback of the TIMERPROC signature; this
    // thread's message loop delivers it, and `on_look_again` kills it.
    let timer = unsafe { SetTimer(std::ptr::null_mut(), 0, ms, Some(on_look_again)) };
    if timer == 0 {
        tracing::debug!(error = %std::io::Error::last_os_error(), "overlay keeper: SetTimer failed");
    }
    LOOK_AGAIN.set(timer);
}

/// The timer [`look_again_in`] set: once only, then the pill is looked at again.
unsafe extern "system" fn on_look_again(_hwnd: HWND, _message: u32, timer: usize, _time: u32) {
    // SAFETY: the thread timer this keeper set on this thread, killed once.
    unsafe { KillTimer(std::ptr::null_mut(), timer) };
    LOOK_AGAIN.set(0);
    let pill = PILL.get();
    if pill != 0 {
        raise_over_cover(pill);
    }
}

fn raise_logged(pill: isize) {
    if !raise(pill) {
        tracing::debug!("overlay keeper: the pill window refused to be raised");
    }
}

/// The shell's taskbars: never raised over.
const TASKBARS: &[&str] = &["Shell_TrayWnd", "Shell_SecondaryTrayWnd"];
/// Windows that open over the pill for a moment while the user works in another window (menus,
/// tooltips) or the taskbar: the pill is not raised over them.
const PASSING: &[&str] = &["#32768", "tooltips_class32", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"];

/// The first window above the pill that covers part of it: visible, on this virtual desktop
/// (not cloaked), not minimized, a window of its own (owned windows are a program's menus,
/// tooltips and dialogs over its own window) and not one of [`PASSING`].
fn covering(pill: isize) -> Option<isize> {
    let area = rect_of(pill as HWND)?;
    // SAFETY: plain Win32 queries on window handles; a stale handle answers null.
    let mut hwnd = unsafe { GetWindow(pill as HWND, GW_HWNDPREV) };
    for _ in 0..MAX_ABOVE {
        if hwnd.is_null() {
            return None;
        }
        if covers(hwnd, &area) {
            return Some(hwnd as isize);
        }
        // SAFETY: as above.
        hwnd = unsafe { GetWindow(hwnd, GW_HWNDPREV) };
    }
    None
}

fn covers(hwnd: HWND, area: &RECT) -> bool {
    // SAFETY: plain Win32 queries on a window handle.
    let shown = unsafe { IsWindowVisible(hwnd) != 0 && IsIconic(hwnd) == 0 && GetWindow(hwnd, GW_OWNER).is_null() };
    shown && rect_of(hwnd).is_some_and(|rect| overlap(&rect, area)) && !class_is(hwnd, PASSING) && !cloaked(hwnd)
}

fn rect_of(hwnd: HWND) -> Option<RECT> {
    let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    // SAFETY: a window handle and a RECT on the stack.
    (unsafe { GetWindowRect(hwnd, &raw mut rect) } != 0).then_some(rect)
}

fn overlap(a: &RECT, b: &RECT) -> bool {
    a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom
}

/// Hidden by the desktop window manager: on another virtual desktop, or a frame kept for later.
fn cloaked(hwnd: HWND) -> bool {
    let mut cloak = 0u32;
    // SAFETY: a window handle and a u32 of the size passed.
    unsafe { DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED.cast_unsigned(), (&raw mut cloak).cast(), 4) == S_OK && cloak != 0 }
}

fn class_is(hwnd: HWND, classes: &[&str]) -> bool {
    let mut class = [0u16; 64];
    // SAFETY: a window handle and a buffer of the length passed.
    let n = unsafe { GetClassNameW(hwnd, class.as_mut_ptr(), 64) };
    let Some(class) = usize::try_from(n).ok().and_then(|n| class.get(..n)) else { return false };
    classes.iter().any(|name| name.encode_utf16().eq(class.iter().copied()))
}

/// While it lives, puts the pill back on top whenever another window comes to the front
/// (`EVENT_SYSTEM_FOREGROUND`) and whenever a window appears (`EVENT_OBJECT_SHOW`) or the stacking
/// order changes (`EVENT_OBJECT_REORDER`) with a window over the pill: out-of-context WinEvent
/// hooks on a thread of its own that pumps messages. Nothing runs between those events but the
/// one-off timer that looks again at a window back over the pill at once; dropping it removes the
/// hooks and ends the thread.
#[derive(Debug)]
pub struct Keeper {
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl Keeper {
    /// Start keeping `hwnd` on top; the error names the call that failed.
    pub fn start(hwnd: isize) -> Result<Self, String> {
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread = std::thread::Builder::new().name("voltip-overlay-keeper".into()).spawn(move || run(hwnd, &ready_tx)).map_err(|e| e.to_string())?;
        match ready_rx.recv_timeout(START_TIMEOUT) {
            Ok(Ok(thread_id)) => Ok(Self { thread_id, thread: Some(thread) }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => Err("the keeper's thread did not start".to_owned()),
        }
    }
}

impl Drop for Keeper {
    /// Quit the thread's message loop, which removes the hooks.
    fn drop(&mut self) {
        // SAFETY: plain Win32 call with a thread id this module obtained; a thread that already
        // ended makes it fail harmlessly.
        unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The keeper's thread: the hooks, then the message loop that delivers their events.
fn run(hwnd: isize, ready: &mpsc::SyncSender<Result<u32, String>>) {
    PILL.set(hwnd);
    let mut hooks = Vec::with_capacity(2);
    for (first, last) in [(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND), (EVENT_OBJECT_SHOW, EVENT_OBJECT_REORDER)] {
        // SAFETY: an out-of-context hook with a callback of the WINEVENTPROC signature, for every
        // process and thread; it is delivered to this thread's message loop below and removed there.
        let hook = unsafe { SetWinEventHook(first, last, std::ptr::null_mut(), Some(on_event), 0, 0, WINEVENT_OUTOFCONTEXT) };
        if hook.is_null() {
            let error = format!("SetWinEventHook: {}", std::io::Error::last_os_error());
            for hook in hooks {
                // SAFETY: a handle SetWinEventHook returned on this thread, removed once.
                unsafe { UnhookWinEvent(hook) };
            }
            let _ = ready.send(Err(error));
            return;
        }
        hooks.push(hook);
    }
    // SAFETY: no arguments. The hook calls above made this thread's message queue, so a WM_QUIT
    // posted from now on is not lost.
    let _ = ready.send(Ok(unsafe { GetCurrentThreadId() }));
    // SAFETY: `msg` is a valid, writable MSG; a null window takes every message of this thread.
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    // SAFETY: as above; the loop ends on WM_QUIT (0) or an error (-1).
    while unsafe { GetMessageW(&raw mut msg, std::ptr::null_mut(), 0, 0) } > 0 {
        // SAFETY: a message GetMessageW filled in.
        unsafe {
            TranslateMessage(&raw const msg);
            DispatchMessageW(&raw const msg);
        }
    }
    for hook in hooks {
        // SAFETY: a handle SetWinEventHook returned on this thread, removed once.
        unsafe { UnhookWinEvent(hook) };
    }
    let timer = LOOK_AGAIN.replace(0);
    if timer != 0 {
        // SAFETY: the thread timer this keeper set on this thread and has not killed yet.
        unsafe { KillTimer(std::ptr::null_mut(), timer) };
    }
}
