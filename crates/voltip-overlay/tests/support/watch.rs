//! What happened around a click that did not bring a test window to the front, for the failure
//! message of [`super::TestWindow::activate`]: the messages the test's windows received, what the
//! system reported through WinEvents (the window in front, mouse capture, menus, move and size
//! loops, desktop switches, and windows shown, hidden, cloaked or focused), and two probes of a
//! window's thread: whether it handles a message posted to it, and where it is when a message sent
//! to it arrives.

use std::backtrace::Backtrace;
use std::collections::VecDeque;
use std::fmt::Write as _;
use std::sync::{Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CHILDID_SELF, DispatchMessageW, EVENT_OBJECT_CLOAKED, EVENT_OBJECT_FOCUS, EVENT_OBJECT_HIDE, EVENT_OBJECT_SHOW, EVENT_OBJECT_UNCLOAKED,
    EVENT_SYSTEM_CAPTUREEND, EVENT_SYSTEM_CAPTURESTART, EVENT_SYSTEM_DESKTOPSWITCH, EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MENUEND, EVENT_SYSTEM_MENUPOPUPEND,
    EVENT_SYSTEM_MENUPOPUPSTART, EVENT_SYSTEM_MENUSTART, EVENT_SYSTEM_MINIMIZEEND, EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MOVESIZEEND,
    EVENT_SYSTEM_MOVESIZESTART, EVENT_SYSTEM_SWITCHEND, EVENT_SYSTEM_SWITCHSTART, GA_ROOT, GetAncestor, GetMessageW, MSG, OBJID_WINDOW, PostMessageW,
    SMTO_BLOCK, SendMessageTimeoutW, TranslateMessage, WINEVENT_OUTOFCONTEXT, WM_ACTIVATE, WM_ACTIVATEAPP, WM_APP, WM_CAPTURECHANGED, WM_KILLFOCUS,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_NCACTIVATE, WM_NCHITTEST, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP, WM_NCMOUSEMOVE, WM_PAINT,
    WM_SETCURSOR, WM_SETFOCUS, WM_SHOWWINDOW, WM_WINDOWPOSCHANGED,
};

/// Posted to a test window by [`posted_probe`]: handled at once by a thread that pumps its queue.
pub(super) const PROBE_POSTED: u32 = WM_APP;
/// Sent to a test window by [`stack_probe`]: its window procedure records the stack it runs on.
pub(super) const PROBE_TRACE: u32 = WM_APP + 1;

/// A message one of the test's windows received.
struct Message {
    at: Instant,
    hwnd: isize,
    msg: u32,
    wparam: usize,
    lparam: isize,
    result: isize,
}

/// The messages the test's windows received last (at most [`KEPT`]).
static MESSAGES: Mutex<VecDeque<Message>> = Mutex::new(VecDeque::new());
const KEPT: usize = 8192;

/// A WinEvent, with the window it is about described when it came.
struct Event {
    at: Instant,
    event: u32,
    thread: u32,
    what: String,
}

/// The WinEvents reported last (at most [`KEPT`]).
static EVENTS: Mutex<VecDeque<Event>> = Mutex::new(VecDeque::new());
/// Whether the WinEvent hooks are in place (set once per process).
static WATCHING: OnceLock<bool> = OnceLock::new();

/// The stack [`PROBE_TRACE`] found its window's thread on.
static TRACE: Mutex<Option<Backtrace>> = Mutex::new(None);

fn keep<T>(log: &Mutex<VecDeque<T>>, entry: T) {
    if let Ok(mut log) = log.lock() {
        if log.len() >= KEPT {
            log.pop_front();
        }
        log.push_back(entry);
    }
}

/// Note a message a test window's procedure handled, if it tells what a click, an activation or
/// the window's message loop did.
pub(super) fn note(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, result: LRESULT) {
    let noted = matches!(
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
            | WM_NCLBUTTONUP
            | WM_CAPTURECHANGED
            | WM_SETCURSOR
            | WM_NCHITTEST
            | WM_MOUSEMOVE
            | WM_NCMOUSEMOVE
            | WM_PAINT
            | WM_SHOWWINDOW
            | WM_WINDOWPOSCHANGED
            | PROBE_POSTED
            | PROBE_TRACE
    );
    if noted {
        keep(&MESSAGES, Message { at: Instant::now(), hwnd: hwnd as isize, msg, wparam, lparam, result });
    }
}

/// Called by the window procedure on [`PROBE_TRACE`]: record the stack of the thread it runs on.
pub(super) fn trace_here() {
    if let Ok(mut trace) = TRACE.lock() {
        *trace = Some(Backtrace::force_capture());
    }
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
        WM_NCLBUTTONUP => "WM_NCLBUTTONUP",
        WM_CAPTURECHANGED => "WM_CAPTURECHANGED",
        WM_SETCURSOR => "WM_SETCURSOR",
        WM_NCHITTEST => "WM_NCHITTEST",
        WM_MOUSEMOVE => "WM_MOUSEMOVE",
        WM_NCMOUSEMOVE => "WM_NCMOUSEMOVE",
        WM_PAINT => "WM_PAINT",
        WM_SHOWWINDOW => "WM_SHOWWINDOW",
        WM_WINDOWPOSCHANGED => "WM_WINDOWPOSCHANGED",
        PROBE_POSTED => "the posted probe",
        PROBE_TRACE => "the sent probe",
        _ => "?",
    }
}

fn event_name(event: u32) -> String {
    match event {
        EVENT_SYSTEM_FOREGROUND => "FOREGROUND".to_owned(),
        EVENT_SYSTEM_MENUSTART => "MENUSTART".to_owned(),
        EVENT_SYSTEM_MENUEND => "MENUEND".to_owned(),
        EVENT_SYSTEM_MENUPOPUPSTART => "MENUPOPUPSTART".to_owned(),
        EVENT_SYSTEM_MENUPOPUPEND => "MENUPOPUPEND".to_owned(),
        EVENT_SYSTEM_CAPTURESTART => "CAPTURESTART".to_owned(),
        EVENT_SYSTEM_CAPTUREEND => "CAPTUREEND".to_owned(),
        EVENT_SYSTEM_MOVESIZESTART => "MOVESIZESTART".to_owned(),
        EVENT_SYSTEM_MOVESIZEEND => "MOVESIZEEND".to_owned(),
        EVENT_SYSTEM_SWITCHSTART => "SWITCHSTART".to_owned(),
        EVENT_SYSTEM_SWITCHEND => "SWITCHEND".to_owned(),
        EVENT_SYSTEM_MINIMIZESTART => "MINIMIZESTART".to_owned(),
        EVENT_SYSTEM_MINIMIZEEND => "MINIMIZEEND".to_owned(),
        EVENT_SYSTEM_DESKTOPSWITCH => "DESKTOPSWITCH".to_owned(),
        EVENT_OBJECT_SHOW => "SHOW".to_owned(),
        EVENT_OBJECT_HIDE => "HIDE".to_owned(),
        EVENT_OBJECT_FOCUS => "FOCUS".to_owned(),
        EVENT_OBJECT_CLOAKED => "CLOAKED".to_owned(),
        EVENT_OBJECT_UNCLOAKED => "UNCLOAKED".to_owned(),
        other => format!("event {other:#x}"),
    }
}

/// `at` relative to `click`, signed.
fn offset(at: Instant, click: Instant) -> String {
    if at >= click { format!("+{} ms", at.duration_since(click).as_millis()) } else { format!("-{} ms", click.duration_since(at).as_millis()) }
}

/// The messages the test's windows received from `since` on, in order, `this` named.
pub(super) fn messages_since(since: Instant, click: Instant, this: isize) -> String {
    let Ok(messages) = MESSAGES.lock() else { return "\n  (the log is poisoned)".to_owned() };
    let mut text = String::new();
    for message in messages.iter().filter(|message| message.at >= since) {
        let whose = if message.hwnd == this { "this window".to_owned() } else { format!("{:#x}", message.hwnd) };
        let _ = write!(
            text,
            "\n  {} {whose}: {} wparam {:#x} lparam {:#x} -> {:#x}",
            offset(message.at, click),
            message_name(message.msg),
            message.wparam,
            message.lparam,
            message.result
        );
    }
    if text.is_empty() { "\n  nothing".to_owned() } else { text }
}

/// Start recording WinEvents, once per process, on a thread that pumps their messages; whether the
/// hooks are in place.
pub(super) fn watch() -> bool {
    *WATCHING.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let ranges = [
                (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_DESKTOPSWITCH),
                (EVENT_OBJECT_SHOW, EVENT_OBJECT_HIDE),
                (EVENT_OBJECT_FOCUS, EVENT_OBJECT_FOCUS),
                (EVENT_OBJECT_CLOAKED, EVENT_OBJECT_UNCLOAKED),
            ];
            let hooked = ranges.iter().all(|&(first, last)| {
                // SAFETY: an out-of-context hook with a callback of the WINEVENTPROC signature, for
                // every process and thread, delivered to this thread's message loop below; it stays
                // until the process ends.
                let hook = unsafe { SetWinEventHook(first, last, std::ptr::null_mut(), Some(on_event), 0, 0, WINEVENT_OUTOFCONTEXT) };
                !hook.is_null()
            });
            let _ = tx.send(hooked);
            // SAFETY: a message loop on this thread; `msg` is a writable MSG.
            unsafe {
                let mut msg: MSG = std::mem::zeroed();
                while GetMessageW(&raw mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                    TranslateMessage(&raw const msg);
                    DispatchMessageW(&raw const msg);
                }
            }
        });
        rx.recv_timeout(super::WAIT).unwrap_or(false)
    })
}

unsafe extern "system" fn on_event(_hook: HWINEVENTHOOK, event: u32, hwnd: HWND, object: i32, child: i32, thread: u32, _time: u32) {
    let at = Instant::now();
    // A window's own show, hide or cloak, not one of its parts'; the focus of anything, by the
    // top-level window it is in.
    let whole_window = object == OBJID_WINDOW && child.cast_unsigned() == CHILDID_SELF;
    let root = if hwnd.is_null() {
        0
    } else {
        // SAFETY: plain query; a stale handle answers null.
        unsafe { GetAncestor(hwnd, GA_ROOT) as isize }
    };
    let top_level = root == 0 || root == hwnd as isize;
    let wanted = match event {
        EVENT_OBJECT_SHOW | EVENT_OBJECT_HIDE | EVENT_OBJECT_CLOAKED | EVENT_OBJECT_UNCLOAKED => whole_window && top_level,
        _ => true,
    };
    if !wanted {
        return;
    }
    let window = if root != 0 { root } else { hwnd as isize };
    keep(&EVENTS, Event { at, event, thread, what: super::describe(window) });
}

/// The WinEvents reported from `since` on, in order.
pub(super) fn events_since(since: Instant, click: Instant) -> String {
    let watching = WATCHING.get().copied().unwrap_or(false);
    let Ok(events) = EVENTS.lock() else { return "\n  (the log is poisoned)".to_owned() };
    let mut text = String::new();
    for event in events.iter().filter(|event| event.at >= since) {
        let _ = write!(text, "\n  {} {} {} (thread {})", offset(event.at, click), event_name(event.event), event.what, event.thread);
    }
    if !watching {
        text.insert_str(0, "\n  (the WinEvent hooks could not be set)");
    }
    if text.is_empty() { "\n  nothing".to_owned() } else { text }
}

/// Whether the thread of `hwnd` handles a message posted to its window now, within `within`.
pub(super) fn posted_probe(hwnd: isize, within: Duration) -> String {
    let posted = Instant::now();
    // SAFETY: a window handle of this test; a stale one makes the post fail.
    if unsafe { PostMessageW(hwnd as HWND, PROBE_POSTED, 0, 0) } == 0 {
        return format!("cannot be posted to ({})", std::io::Error::last_os_error());
    }
    let handled = super::wait_until(within, || {
        MESSAGES.lock().is_ok_and(|messages| messages.iter().any(|message| message.hwnd == hwnd && message.msg == PROBE_POSTED && message.at >= posted))
    });
    if handled {
        format!("handles a message posted to it now, after {} ms", posted.elapsed().as_millis())
    } else {
        format!("does not handle a message posted to it now within {} ms", within.as_millis())
    }
}

/// Where the thread of `hwnd` is when a message sent to its window arrives: its stack, from the
/// window procedure down to the thread's start (frames of the standard library's own left out).
pub(super) fn stack_probe(hwnd: isize) -> String {
    if let Ok(mut trace) = TRACE.lock() {
        *trace = None;
    }
    let mut result = 0usize;
    // SAFETY: a window handle of this test; `result` outlives the call.
    let answered = unsafe { SendMessageTimeoutW(hwnd as HWND, PROBE_TRACE, 0, 0, SMTO_BLOCK, 2000, &raw mut result) } != 0;
    let trace = TRACE.lock().ok().and_then(|mut trace| trace.take());
    let Some(trace) = trace else {
        return format!("\n  (no answer within 2 s{})", if answered { ", yet the call returned" } else { "" });
    };
    let text = trace.to_string();
    let mut lines = text.lines().skip_while(|line| !line.contains("window_proc")).filter(|line| !line.contains("/rustc/")).peekable();
    if lines.peek().is_none() {
        return format!("\n  {}", text.lines().take(60).collect::<Vec<_>>().join("\n  "));
    }
    let mut out = String::new();
    for line in lines.take(60) {
        let _ = write!(out, "\n  {line}");
    }
    out
}
