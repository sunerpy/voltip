//! The insert check's AT-SPI reader against a real GTK 3 window (docs/dictation.md §28): text that
//! lands in an editable entry is `written`, the same keys on a read-only entry leave it
//! `not_written`, the focus moving to another entry is `unconfirmed`, and with accessibility off
//! nothing is read. Ignored: it needs an X server, a D-Bus session with the accessibility bus and
//! GTK 3. Run it with `scripts/focus-check-linux.sh`, which sets them up (CI's `smoke-desktop`).

#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(target_os = "linux")]

use std::io::{BufRead as _, BufReader, Write as _};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use voltip_focus::{Expected, Reader, Settled, UnconfirmedWhy, linux};
use voltip_platform::HostOs;
use voltip_platform::insert_check::rules::InsertCheck;

/// What the keyboard "pastes": typed with xdotool, so it reaches the entry like keys would.
const TYPED: &str = "voltip check";
/// How long a paste is watched: well over `CONFIRM_WITHIN`, for a slow runner.
const WATCH: Duration = Duration::from_secs(3);
/// How long the helper window may take to answer.
const ANSWER: Duration = Duration::from_secs(20);
/// The application the keys go to, as the probe would name the helper (any id outside the table of
/// skipped applications).
const APP: &str = "python3";

/// The GTK 3 helper window (`tests/support/gtk_entries.py`).
struct Entries {
    child: Child,
    stdin: ChildStdin,
    lines: mpsc::Receiver<String>,
}

impl Entries {
    fn start() -> Self {
        let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/gtk_entries.py");
        let mut child = Command::new("/usr/bin/python3").arg(script).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().expect("python3 with GTK 3");
        let stdin = child.stdin.take().unwrap();
        let stdout: ChildStdout = child.stdout.take().unwrap();
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let mut entries = Self { child, stdin, lines };
        entries.expect("ready");
        entries
    }

    fn expect(&mut self, wanted: &str) {
        loop {
            let line = self.lines.recv_timeout(ANSWER).unwrap_or_else(|e| panic!("the helper window did not say {wanted:?}: {e}"));
            if line == wanted {
                return;
            }
        }
    }

    /// Give `name` the keyboard focus: the window gets the X input focus (no window manager runs
    /// under Xvfb), then the entry the GTK focus.
    fn focus(&mut self, name: &str) {
        xdotool(&["search", "--sync", "--name", "voltip-focus-check", "windowfocus", "--sync"]);
        writeln!(self.stdin, "{name}").unwrap();
        self.expect(&format!("focused {name}"));
    }
}

impl Drop for Entries {
    fn drop(&mut self) {
        let _ = writeln!(self.stdin, "quit");
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

fn xdotool(args: &[&str]) {
    let status = Command::new("xdotool").args(args).status().expect("xdotool");
    assert!(status.success(), "xdotool {args:?}: {status}");
}

fn reader() -> Reader {
    Reader::new(HostOs::Linux, linux::open)
}

/// Watch the focused entry, type [`TYPED`], and settle.
fn type_and_settle(reader: &Reader) -> Settled {
    let watch = reader.watch(APP, &Expected::of(TYPED)).expect("the focused entry is read");
    xdotool(&["type", "--delay", "5", TYPED]);
    watch.settle(WATCH)
}

fn set_accessibility(on: bool) {
    let value = if on { "<true>" } else { "<false>" };
    let status = Command::new("gdbus")
        .args(["call", "--session", "--dest", "org.a11y.Bus", "--object-path", "/org/a11y/bus", "--method", "org.freedesktop.DBus.Properties.Set"])
        .args(["org.a11y.Status", "IsEnabled", value])
        .stdout(Stdio::null())
        .status()
        .expect("gdbus");
    assert!(status.success(), "IsEnabled {on}: {status}");
}

#[test]
#[ignore = "needs Xvfb, the accessibility bus and GTK 3: scripts/focus-check-linux.sh"]
fn a_typed_text_lands_a_read_only_entry_stays_and_another_entry_takes_the_focus() {
    let mut window = Entries::start();
    let reader = reader();

    window.focus("editable");
    assert_eq!(type_and_settle(&reader), Settled::sure(InsertCheck::Written));

    window.focus("readonly");
    assert_eq!(type_and_settle(&reader), Settled::sure(InsertCheck::NotWritten));

    window.focus("editable");
    let watch = reader.watch(APP, &Expected::of(TYPED)).expect("the focused entry is read");
    window.focus("other");
    assert_eq!(watch.settle(WATCH), Settled::unconfirmed(UnconfirmedWhy::ControlChanged));
}

#[test]
#[ignore = "needs Xvfb, the accessibility bus and GTK 3: scripts/focus-check-linux.sh"]
fn with_accessibility_off_nothing_is_read() {
    let mut window = Entries::start();
    window.focus("editable");
    set_accessibility(false);
    // A new reader opens the bus again, and finds accessibility off.
    let refused = reader().watch(APP, &Expected::of(TYPED)).err();
    set_accessibility(true);
    assert_eq!(refused, Some(UnconfirmedWhy::Unreadable));
    assert!(reader().watch(APP, &Expected::of(TYPED)).is_ok(), "on again, the entry is read");
}
