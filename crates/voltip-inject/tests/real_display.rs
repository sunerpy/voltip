//! The tests that need a display server. Headless CI skips them; run them under Xvfb with
//! `DISPLAY=:99 cargo test -p voltip-inject --test real_display -- --ignored`
//! (or `xvfb-run -a cargo test -p voltip-inject --test real_display -- --ignored`).
//! The selection tests (docs/dictation.md §19) also need `python3` with `tkinter` (the X11 text
//! widget whose selection is copied) and `xdotool` (to hold keys the way a user holding the
//! hotkey would).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Mutex;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use voltip_inject::{
    ClipboardOnlyInjector, ClipboardPasteInjector, ClipboardPort, CopyOptions, Injection, Injector, Modifier, SelectionSource, SystemClipboard, Via,
    display_available, system_selection,
};

/// The clipboard and the keyboard focus are shared by every test in this file.
static DISPLAY_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    DISPLAY_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[test]
#[ignore = "needs a display server; run with DISPLAY set"]
fn real_clipboard_round_trip_paste_and_restore() {
    let _display = lock();
    display_available().expect("DISPLAY (or WAYLAND_DISPLAY) must be set for this test");
    let clipboard_only = ClipboardOnlyInjector::new();
    let copied = clipboard_only.inject("voltip smoke 你好").expect("clipboard write");
    assert_eq!(copied, Injection { via: Via::Clipboard, chars: 15, note: None });
    // A second handle in the same process sees what the first one owns.
    let reader = SystemClipboard::new();
    assert_eq!(reader.read_text().expect("read").as_deref(), Some("voltip smoke 你好"));

    let paste = ClipboardPasteInjector::new(Duration::from_millis(50));
    let pasted = paste.inject("voltip paste").expect("clipboard write");
    // With a bare Xvfb nothing has focus; the chord either goes through (XTEST) or enigo refuses.
    assert!(matches!(pasted.via, Via::Paste | Via::Clipboard), "{pasted:?}");
    eprintln!("real injector under this display: {pasted:?}");
    assert_eq!(reader.read_text().expect("read").as_deref(), Some("voltip paste"));

    if pasted.via == Via::Paste {
        // The previous text comes back once the restore delay has passed.
        let deadline = Instant::now() + Duration::from_secs(3);
        while reader.read_text().expect("read").as_deref() != Some("voltip smoke 你好") {
            assert!(Instant::now() < deadline, "clipboard was not restored: {:?}", reader.read_text());
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// What the Tk entry holds and selects.
const SELECTED: &str = "voltip 选中文本 selection";

/// A Tk entry with `SELECTED` selected and the keyboard focus (no window manager needed:
/// `focus_force` sets the X input focus itself). It prints `ready`, reports every `c` and `Insert` it
/// receives with its modifier state (`key insert state=4` is Ctrl alone, `12` Ctrl+Alt), and obeys
/// `deselect` / `select` / `quit` lines on stdin. Tk's X11 bindings copy the selection to CLIPBOARD
/// on Ctrl+C and on Ctrl+Insert (its default `<<Copy>>` virtual event).
const TK_ENTRY: &str = r#"
import queue, sys, threading, tkinter as tk
root = tk.Tk()
root.title("voltip-selection-target")
entry = tk.Entry(root, width=48)
entry.insert(0, sys.argv[1])
entry.pack()
def on_key(ev):
    if ev.keysym.lower() in ("c", "insert"):
        print(f"key {ev.keysym.lower()} state={ev.state}", flush=True)
entry.bind("<KeyPress>", on_key, add="+")
commands = queue.Queue()
def read_stdin():
    for line in sys.stdin:
        commands.put(line.strip())
threading.Thread(target=read_stdin, daemon=True).start()
def select():
    entry.focus_force()
    entry.select_range(0, "end")
def pump():
    while not commands.empty():
        cmd = commands.get()
        if cmd == "deselect":
            entry.selection_clear()
            print("deselected", flush=True)
        elif cmd == "select":
            select()
            print("selected", flush=True)
        elif cmd == "quit":
            root.destroy()
            return
    root.after(20, pump)
def ready():
    select()
    print("ready", flush=True)
root.after(100, ready)
root.after(20, pump)
root.mainloop()
"#;

/// A client that grabs `Ctrl+Alt+E` on the root window exactly like global-hotkey's X11 backend
/// (`XGrabKey`, async modes), printing `grabbed` once the grab is in place.
const X11_GRABBER: &str = r#"
import ctypes
x = ctypes.CDLL("libX11.so.6")
x.XOpenDisplay.restype = ctypes.c_void_p
x.XDefaultRootWindow.restype = ctypes.c_ulong
x.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
x.XKeysymToKeycode.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
x.XKeysymToKeycode.restype = ctypes.c_ubyte
x.XGrabKey.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_uint, ctypes.c_ulong, ctypes.c_int, ctypes.c_int, ctypes.c_int]
x.XNextEvent.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
x.XSync.argtypes = [ctypes.c_void_p, ctypes.c_int]
d = x.XOpenDisplay(None)
root = x.XDefaultRootWindow(d)
x.XGrabKey(d, x.XKeysymToKeycode(d, 0x65), (1 << 2) | (1 << 3), root, 0, 1, 1)
x.XSync(d, 0)
print("grabbed", flush=True)
event = (ctypes.c_char * 192)()
while True:
    x.XNextEvent(d, event)
"#;

/// A child process with its stdout lines forwarded to a channel.
struct Helper {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: mpsc::Receiver<String>,
}

impl Helper {
    fn python(script: &str, args: &[&str]) -> Self {
        let mut child = Command::new("python3")
            .arg("-c")
            .arg(script)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("python3 must be installed for this test");
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let stdin = child.stdin.take();
        Self { child, stdin, lines }
    }

    /// Wait (bounded) for a line equal to `want`; every line seen on the way is returned too.
    fn expect_line(&self, want: &str) -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut seen = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let line = self.lines.recv_timeout(left).unwrap_or_else(|e| panic!("no `{want}` line within 10 s ({e}); saw {seen:?}"));
            let done = line == want;
            seen.push(line);
            if done {
                return seen;
            }
        }
    }

    /// Lines that arrived so far, without waiting.
    fn drain(&self) -> Vec<String> {
        self.lines.try_iter().collect()
    }

    fn send(&mut self, command: &str) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{command}").unwrap();
        stdin.flush().unwrap();
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        if let Some(stdin) = self.stdin.as_mut() {
            let _ = writeln!(stdin, "quit");
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn xdotool(args: &[&str]) {
    let status = Command::new("xdotool").args(args).status().expect("xdotool must be installed for this test");
    assert!(status.success(), "xdotool {args:?}: {status}");
}

fn require_x11() {
    display_available().expect("DISPLAY must be set for this test");
    assert!(std::env::var("DISPLAY").is_ok_and(|d| !d.is_empty()), "an X11 display is needed (run under xvfb-run)");
}

/// docs/dictation.md §19 on a real X server: the production copier (enigo XTEST on X11) reads the
/// selection of a real text widget and puts the clipboard back; a modifier the user still holds
/// from the hotkey is released first (Tk sees Ctrl alone, state 4, not Ctrl+Alt); nothing selected
/// reads as `None` with the clipboard untouched.
#[test]
#[ignore = "needs an X11 display, python3 with tkinter and xdotool; run under xvfb-run"]
fn real_copy_chord_reads_the_selection_of_an_x11_text_widget() {
    let _display = lock();
    require_x11();
    let mut tk = Helper::python(TK_ENTRY, &[SELECTED]);
    tk.expect_line("ready");
    let clipboard = SystemClipboard::new();
    clipboard.write_text("SENTINEL").unwrap();
    let copier = system_selection(CopyOptions { timeout: Duration::from_millis(1000), ..CopyOptions::default() });

    let got = copier.copy_selection(&[]).unwrap();
    assert_eq!(got.as_deref(), Some(SELECTED));
    assert_eq!(clipboard.read_text().unwrap().as_deref(), Some("SENTINEL"), "the previous clipboard is back");
    // Ctrl+Insert, never Ctrl+C (a terminal's interrupt, docs/dictation.md §19.2).
    let keys = tk.expect_line("key insert state=4");
    assert_eq!(keys, ["key insert state=4"], "Tk received Ctrl+Insert: {keys:?}");

    // The user still holds Alt of Ctrl+Alt+E: it is released before the chord.
    xdotool(&["keydown", "alt"]);
    let got = copier.copy_selection(&[Modifier::Control, Modifier::Alt]);
    xdotool(&["keyup", "alt"]);
    assert_eq!(got.unwrap().as_deref(), Some(SELECTED));
    let keys = tk.expect_line("key insert state=4");
    assert!(!keys.iter().any(|k| k == "key insert state=12"), "Alt was released before the Insert: {keys:?}");
    assert_eq!(clipboard.read_text().unwrap().as_deref(), Some("SENTINEL"));

    // Nothing selected: nothing lands on the cleared clipboard, the previous text comes back.
    tk.send("deselect");
    tk.expect_line("deselected");
    let started = Instant::now();
    assert_eq!(copier.copy_selection(&[]).unwrap(), None);
    assert!(started.elapsed() >= Duration::from_millis(1000), "waited the whole timeout");
    assert_eq!(clipboard.read_text().unwrap().as_deref(), Some("SENTINEL"));
    eprintln!("tk saw: {:?}", tk.drain());
}

/// Why X11 copies only after the hotkey's key is up (docs/dictation.md §19 `AfterKeyUp`): while a
/// key grabbed with `XGrabKey` is down, the X server routes every key event — the synthesised
/// Ctrl+C included — to the grabbing client, not to the focused widget. Once the key is released
/// the same copy works, with the modifiers still held.
#[test]
#[ignore = "needs an X11 display, python3 with tkinter and xdotool; run under xvfb-run"]
fn regression_x11_hotkey_grab_swallows_the_copy_until_the_key_is_up() {
    let _display = lock();
    require_x11();
    let _grabber = {
        let grabber = Helper::python(X11_GRABBER, &[]);
        grabber.expect_line("grabbed");
        grabber
    };
    let tk = Helper::python(TK_ENTRY, &[SELECTED]);
    tk.expect_line("ready");
    let clipboard = SystemClipboard::new();
    clipboard.write_text("SENTINEL").unwrap();
    let copier = system_selection(CopyOptions { timeout: Duration::from_millis(600), ..CopyOptions::default() });

    // The hotkey is held: the passive grab is active.
    xdotool(&["keydown", "ctrl", "alt", "e"]);
    let during = copier.copy_selection(&[Modifier::Control, Modifier::Alt]);
    // The key comes up, the modifiers are still held.
    xdotool(&["keyup", "e"]);
    let after = copier.copy_selection(&[Modifier::Control, Modifier::Alt]);
    xdotool(&["keyup", "alt", "ctrl"]);
    assert_eq!(during.unwrap(), None, "during the grab the copy chord never reaches the widget");
    assert_eq!(after.unwrap().as_deref(), Some(SELECTED), "after the key is up the same copy works");
    assert_eq!(clipboard.read_text().unwrap().as_deref(), Some("SENTINEL"));
    eprintln!("tk saw: {:?}", tk.drain());
}
