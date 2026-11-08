//! Pastes on GNOME through the remote-desktop portal (docs/dictation.md §14.9, issue #235):
//! Voltip asks `org.freedesktop.portal.RemoteDesktop` for keyboard-only remote interaction with a
//! persistent grant once, restores the session from the saved restore token for every later
//! paste, and keeps each session open only for its one chord. A fake portal and a fake EIS keyboard
//! on a private D-Bus session bus stand in for xdg-desktop-portal and Mutter
//! (tests/support/portal.rs).
//!
//! Linux only. Needs `dbus-daemon` and the XKB data, which CI's Ubuntu runners have.

#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::io::Read as _;
use std::os::unix::fs::PermissionsExt as _;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use support::portal::{FakePortal, KEYBOARD, PrivateBus};
use voltip_inject::KeystrokePort as _;

/// Set in the child process a test starts.
const CHILD: &str = "VOLTIP_PORTAL_TEST_CHILD";
/// How many pastes the child makes.
const PASTES: usize = 3;
/// How long the child may take.
const CHILD_PATIENCE: Duration = Duration::from_secs(120);
/// Ctrl+V on a US layout: evdev `KEY_LEFTCTRL` and `KEY_V`, down and up.
const CTRL_V: [(u32, bool); 4] = [(29, true), (47, true), (47, false), (29, false)];
/// What a coverage or debugging run needs in the child too.
const PASSED_THROUGH: &[&str] = &["LLVM_PROFILE_FILE", "CARGO_LLVM_COV", "CARGO_LLVM_COV_TARGET_DIR", "RUST_BACKTRACE", "RUST_LOG"];

/// Run `test` of this binary again in a child process: a GNOME Wayland session without XWayland
/// and without any paste tool on its `PATH`, whose session bus is `bus`. Returns the child's
/// `PASTE …` lines.
fn run_child(test: &str, bus: &PrivateBus) -> Vec<String> {
    let dir = tempfile::tempdir().unwrap();
    let no_tools = dir.path().join("bin");
    let runtime = dir.path().join("run");
    std::fs::create_dir(&no_tools).unwrap();
    std::fs::create_dir(&runtime).unwrap();
    std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args([test, "--exact", "--nocapture", "--test-threads=1"]).env_clear();
    for name in PASSED_THROUGH {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command
        .env(CHILD, "1")
        .env("PATH", &no_tools)
        .env("HOME", dir.path())
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("DBUS_SESSION_BUS_ADDRESS", &bus.address)
        .env("XDG_SESSION_TYPE", "wayland")
        .env("XDG_CURRENT_DESKTOP", "GNOME")
        .env("WAYLAND_DISPLAY", "voltip-test-no-compositor")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let mut child = command.spawn().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut out = String::new();
        let _ = stdout.read_to_string(&mut out);
        out
    });
    let deadline = Instant::now() + CHILD_PATIENCE;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("the child process did not finish within {CHILD_PATIENCE:?}");
        }
        // A bounded poll of the child's exit; the deadline above ends it.
        std::thread::sleep(Duration::from_millis(20));
    };
    let out = reader.join().unwrap();
    assert!(status.success(), "the child process failed ({status}):\n{out}");
    out.lines().filter_map(|line| line.split_once("PASTE ").map(|(_, rest)| rest.to_owned())).collect()
}

/// The child's part: [`PASTES`] pastes through the chain this session builds, as the shell's
/// injector makes them.
fn child_pastes() {
    let keys = voltip_inject::linux::system_toolchain_keys();
    let chord = voltip_inject::paste_chord_for("linux");
    for _ in 0..PASTES {
        match keys.deliver(chord, "dictated text") {
            Ok(delivered) => println!("PASTE delivered by {}", delivered.tool),
            Err(e) => println!("PASTE not delivered: {e}"),
        }
    }
}

/// issue #235: on GNOME every paste went out through XTEST, and Xwayland asked the portal for a
/// fresh grant, never remembered, whenever the last one had lapsed (ten minutes after Voltip's X11
/// connection closed, or after a restart of Voltip or Xwayland). Now Voltip asks the portal itself
/// for keyboard-only remote interaction with a persistent grant: one authorization, then every
/// paste restores the session from the token the portal handed out last, in a session of its own
/// that is closed right after the chord. Before the fix, Voltip never talked to the portal: on a
/// GNOME Wayland session without a uinput tool no paste was delivered.
#[test]
fn regression_gnome_pastes_ask_for_remote_interaction_once() {
    if std::env::var_os(CHILD).is_some() {
        child_pastes();
        return;
    }
    let bus = PrivateBus::start();
    let portal = FakePortal::start(&bus);
    let pastes = run_child("regression_gnome_pastes_ask_for_remote_interaction_once", &bus);
    let record = portal.record();
    assert_eq!(record.dialogs, 1, "one authorization for {PASTES} pastes; the pastes: {pastes:?}; the portal saw {record:?}");
    assert_eq!(pastes, vec!["delivered by remote-desktop-portal".to_string(); PASTES], "every paste goes through the portal: {record:?}");
    assert_eq!(record.restored, PASTES - 1, "the later pastes restore the grant: {record:?}");
    assert_eq!((record.sessions, record.closed_sessions), (PASTES, PASTES), "a session per paste, each closed after it: {record:?}");
    assert_eq!(record.keys, vec![CTRL_V.to_vec(); PASTES], "Ctrl+V once per paste: {record:?}");
    assert!(record.selects.iter().all(|s| s.types == KEYBOARD && s.persist_mode == 2), "keyboard only, remembered until revoked: {:?}", record.selects);
    let offered: Vec<Option<String>> = record.selects.iter().map(|s| s.restore_token.clone()).collect();
    let mut expected = vec![None];
    expected.extend(record.issued.iter().take(PASTES - 1).cloned().map(Some));
    assert_eq!(offered, expected, "each paste offers the token the portal handed out last");
}
