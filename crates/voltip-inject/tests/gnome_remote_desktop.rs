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

use std::sync::Arc;

use support::portal::{Answer, FakePortal, Fault, KEYBOARD, Layout, POINTER, PrivateBus};
use voltip_inject::linux::PortalPort;
use voltip_inject::platform::{KeyStep, chord_steps, copy_steps};
use voltip_inject::portal::{Limits, PortalError, RemoteDesktop};
use voltip_inject::{Chord, DeliveryError, Key, KeystrokePort as _, MemoryRestoreTokens, Modifier, RestoreTokens as _};

/// Set in the child process a test starts.
const CHILD: &str = "VOLTIP_PORTAL_TEST_CHILD";
/// How many pastes the child makes.
const PASTES: usize = 3;
/// How long the child may take.
const CHILD_PATIENCE: Duration = Duration::from_secs(120);
/// Ctrl+V on a US layout: evdev `KEY_LEFTCTRL` and `KEY_V`, down and up.
const CTRL_V: [(u32, bool); 4] = [(29, true), (47, true), (47, false), (29, false)];
/// What a coverage or debugging run needs in the child too.
const PASSED_THROUGH: &[&str] = &["LLVM_PROFILE_FILE", "CARGO_LLVM_COV", "CARGO_LLVM_COV_TARGET_DIR", "RUST_BACKTRACE", "RUST_LOG", "REIS_DEBUG"];

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
    assert!(record.protocol_errors.is_empty(), "{:?}", record.protocol_errors);
    assert_eq!(record.open_sessions(), 0);
}

/// Ctrl+V as the chain presses it.
fn ctrl_v() -> Vec<KeyStep> {
    chord_steps(voltip_inject::paste_chord_for("linux"), "linux", 0)
}

/// A portal client over `tokens`, the store a restart of Voltip finds again (the keychain).
fn client(bus: &PrivateBus, tokens: &Arc<MemoryRestoreTokens>) -> RemoteDesktop {
    RemoteDesktop::at(bus.address.clone(), tokens.clone(), Limits::default())
}

/// The restore token each `SelectDevices` offered, in order.
fn offered(portal: &FakePortal) -> Vec<Option<String>> {
    portal.record().selects.into_iter().map(|s| s.restore_token).collect()
}

/// A restart of Voltip: a new client over the same token store restores the grant.
#[test]
fn a_restart_restores_the_saved_grant() {
    let bus = PrivateBus::start();
    let portal = FakePortal::start(&bus);
    let tokens = Arc::new(MemoryRestoreTokens::new());
    client(&bus, &tokens).press(&ctrl_v()).unwrap();
    client(&bus, &tokens).press(&ctrl_v()).unwrap();
    let record = portal.record();
    assert_eq!((record.dialogs, record.restored), (1, 1), "{record:?}");
    assert_eq!(tokens.load(), record.issued.last().cloned(), "the token handed out last is the one saved");
    assert_eq!(record.keys, vec![CTRL_V.to_vec(); 2]);
    assert_eq!(record.open_sessions(), 0);
    assert!(record.protocol_errors.is_empty(), "{:?}", record.protocol_errors);
}

/// A portal that hands out a new token for every start: each one replaces the saved one.
#[test]
fn a_new_token_replaces_the_saved_one() {
    let bus = PrivateBus::start();
    let portal = FakePortal::start(&bus);
    portal.rotate_tokens();
    let tokens = Arc::new(MemoryRestoreTokens::new());
    let desktop = client(&bus, &tokens);
    for _ in 0..3 {
        desktop.press(&ctrl_v()).unwrap();
    }
    let record = portal.record();
    assert_eq!((record.dialogs, record.restored), (1, 2), "{record:?}");
    let issued = record.issued.clone();
    assert_eq!(issued.len(), 3);
    assert!(issued[0] != issued[1] && issued[1] != issued[2], "{issued:?}");
    assert_eq!(offered(&portal), vec![None, Some(issued[0].clone()), Some(issued[1].clone())]);
    assert_eq!(tokens.load().as_deref(), Some(issued[2].as_str()));
}

/// The user removed the permission: the next paste asks once, the ones after it are restored.
#[test]
fn a_revoked_grant_asks_once_more() {
    let bus = PrivateBus::start();
    let portal = FakePortal::start(&bus);
    let tokens = Arc::new(MemoryRestoreTokens::new());
    let desktop = client(&bus, &tokens);
    desktop.press(&ctrl_v()).unwrap();
    portal.revoke();
    desktop.press(&ctrl_v()).unwrap();
    desktop.press(&ctrl_v()).unwrap();
    let record = portal.record();
    assert_eq!((record.dialogs, record.restored), (2, 1), "{record:?}");
    assert_eq!(record.keys, vec![CTRL_V.to_vec(); 3]);
    assert_eq!(tokens.load(), record.issued.last().cloned());
}

/// "Cancel": nothing is pressed, no token is kept, and the next paste asks again.
#[test]
fn a_refusal_asks_again_at_the_next_paste() {
    let bus = PrivateBus::start();
    let portal = FakePortal::start(&bus);
    let tokens = Arc::new(MemoryRestoreTokens::new());
    let desktop = client(&bus, &tokens);
    portal.answer(&[Answer::Deny]);
    assert_eq!(desktop.press(&ctrl_v()), Err(PortalError::Declined("the user cancelled".into())));
    assert_eq!(tokens.load(), None);
    let record = portal.record();
    assert_eq!((record.dialogs, record.eis_connections, record.open_sessions()), (1, 0, 0), "{record:?}");
    desktop.press(&ctrl_v()).unwrap();
    let record = portal.record();
    assert_eq!(record.dialogs, 2, "{record:?}");
    assert_eq!(record.keys, vec![CTRL_V.to_vec()]);

    // A refusal after a revoke forgets the token the portal no longer knows.
    portal.revoke();
    portal.answer(&[Answer::Deny]);
    assert!(matches!(desktop.press(&ctrl_v()), Err(PortalError::Declined(_))));
    assert_eq!(tokens.load(), None);
    desktop.press(&ctrl_v()).unwrap();
    assert_eq!(offered(&portal).last(), Some(&None), "the forgotten token is not offered again");
    assert_eq!(portal.record().open_sessions(), 0);
}

/// Nobody answers the dialog: the request is closed at the limit, which closes the dialog.
#[test]
fn an_unanswered_dialog_is_closed_at_the_limit() {
    let bus = PrivateBus::start();
    let portal = FakePortal::start(&bus);
    portal.answer(&[Answer::Ignore]);
    let tokens = Arc::new(MemoryRestoreTokens::new());
    let limits = Limits { answer: Duration::from_millis(300), ..Limits::default() };
    let err = RemoteDesktop::at(bus.address.clone(), tokens.clone(), limits).press(&ctrl_v()).unwrap_err();
    assert!(matches!(&err, PortalError::Declined(reason) if reason.starts_with("no answer within")), "{err:?}");
    let record = portal.record();
    assert_eq!((record.closed_requests, record.open_sessions(), record.eis_connections), (1, 0, 0), "{record:?}");
    assert_eq!(tokens.load(), None);
}

/// "Share" without "Remember This Selection": this paste goes out, the next one asks again.
#[test]
fn a_grant_for_one_paste_keeps_no_token() {
    let bus = PrivateBus::start();
    let portal = FakePortal::start(&bus);
    portal.answer(&[Answer::AllowOnce]);
    let tokens = Arc::new(MemoryRestoreTokens::new());
    let desktop = client(&bus, &tokens);
    desktop.press(&ctrl_v()).unwrap();
    assert_eq!(tokens.load(), None);
    desktop.press(&ctrl_v()).unwrap();
    desktop.press(&ctrl_v()).unwrap();
    let record = portal.record();
    assert_eq!((record.dialogs, record.restored), (2, 1), "{record:?}");
    assert_eq!(record.keys.len(), 3);
}

/// The request ended some other way (response 2): the portal closed the session, nothing is
/// pressed, and it is not the user's refusal.
#[test]
fn an_ended_request_is_a_failure_not_a_refusal() {
    let bus = PrivateBus::start();
    let portal = FakePortal::start(&bus);
    portal.answer(&[Answer::End]);
    let tokens = Arc::new(MemoryRestoreTokens::new());
    assert_eq!(client(&bus, &tokens).press(&ctrl_v()), Err(PortalError::Broken("Start answered 2".into())));
    let record = portal.record();
    assert_eq!((record.ended_sessions, record.open_sessions(), record.eis_connections), (1, 0, 0), "{record:?}");
}

/// docs/dictation.md §14.9 (goal gate 2026-11-08): the portal offers the interface, but this paste
/// cannot use it — `CreateSession` or `SelectDevices` answers 2, also after a saved token was
/// dropped and the session asked for again without it. That is a failure of this paste, not a
/// desktop without the interface: broken, so the chain stops with the text on the clipboard
/// instead of handing the paste to XTEST, which would make Xwayland ask the same portal again
/// (issue #235). The next paste tries the portal again.
#[test]
fn regression_a_portal_failing_after_its_check_is_broken_not_unavailable() {
    let bus = PrivateBus::start();
    let portal = FakePortal::start(&bus);
    let tokens = Arc::new(MemoryRestoreTokens::new());
    let desktop = client(&bus, &tokens);
    portal.fault(Fault::EndSession);
    assert_eq!(desktop.press(&ctrl_v()), Err(PortalError::Broken("CreateSession answered 2".into())));
    portal.fault(Fault::RefuseSelect);
    assert_eq!(desktop.press(&ctrl_v()), Err(PortalError::Broken("SelectDevices answered 2".into())));
    let saved = "00000000-0000-4000-8000-0000000000bb";
    tokens.save(saved);
    assert_eq!(desktop.press(&ctrl_v()), Err(PortalError::Broken("SelectDevices answered 2".into())), "the retry without the token fails too");
    assert_eq!(tokens.load(), None, "the refused token is dropped");
    assert_eq!(offered(&portal), vec![None, Some(saved.to_string()), None]);
    portal.fault(Fault::None);
    desktop.press(&ctrl_v()).unwrap();
    let record = portal.record();
    assert_eq!((record.sessions, record.open_sessions(), record.dialogs, record.eis_connections), (5, 0, 1, 1), "{record:?}");
    assert_eq!(record.keys, vec![CTRL_V.to_vec()]);
}

/// docs/dictation.md §14.9: a portal that fails when its properties are read may well be there, so
/// the chain tries it instead of skipping to XTEST; the paste then fails as broken. Only an answer
/// that there is no such portal (no service, no RemoteDesktop interface, version 1, no keyboard)
/// keeps it out of the chain.
#[test]
fn a_portal_that_cannot_be_read_is_tried_not_skipped() {
    let bus = PrivateBus::start();
    let portal = FakePortal::start(&bus);
    portal.fault(Fault::FailProperties);
    let desktop = client(&bus, &Arc::new(MemoryRestoreTokens::new()));
    let err = desktop.available().unwrap_err();
    assert!(matches!(&err, PortalError::Broken(reason) if reason.contains("org.freedesktop.DBus.Error.Failed")), "{err:?}");
    assert!(PortalPort::available(&desktop), "whether it is there is unknown: the chain tries it");
    assert!(matches!(desktop.press(&ctrl_v()), Err(PortalError::Broken(_))));
    assert_eq!(portal.record().sessions, 0, "nothing is asked of a portal that cannot be read");
    portal.fault(Fault::None);
    assert_eq!(desktop.available(), Ok(()));
    desktop.press(&ctrl_v()).unwrap();

    // xdg-desktop-portal without a remote-desktop backend: not there.
    let bus = PrivateBus::start();
    let _portal = FakePortal::without_remote_desktop(&bus);
    let desktop = client(&bus, &Arc::new(MemoryRestoreTokens::new()));
    assert!(matches!(desktop.available(), Err(PortalError::Unavailable(_))), "{:?}", desktop.available());
    assert!(!PortalPort::available(&desktop));
    assert!(matches!(desktop.press(&ctrl_v()), Err(PortalError::Unavailable(_))));
}

/// The child's part of [`regression_a_paste_the_portal_cannot_take_stops_with_its_reason`] and
/// [`regression_a_portal_gone_after_the_probe_stops_with_its_reason`]: one paste through the chain
/// this session builds, and how it ended.
fn child_one_paste() {
    let keys = voltip_inject::linux::system_toolchain_keys();
    match keys.deliver(voltip_inject::paste_chord_for("linux"), "dictated text") {
        Ok(delivered) => println!("PASTE delivered by {}", delivered.tool),
        Err(DeliveryError::Unavailable(note)) => println!("PASTE clipboard {:?}: {}", note.code, note.detail),
        Err(e) => println!("PASTE failed: {e}"),
    }
}

/// docs/dictation.md §14.9 (goal gate 2026-11-08): on GNOME, a paste the portal cannot take ends
/// with the text on the clipboard and its own reason, `remote_unavailable` (the 远程交互不可用
/// card), through the chain the shell builds. Before, it read as no portal at all: the chain went
/// on, to XTEST on XWayland or to `no_tool` without a card on pure Wayland.
#[test]
fn regression_a_paste_the_portal_cannot_take_stops_with_its_reason() {
    if std::env::var_os(CHILD).is_some() {
        child_one_paste();
        return;
    }
    let bus = PrivateBus::start();
    let portal = FakePortal::start(&bus);
    portal.fault(Fault::EndSession);
    let pastes = run_child("regression_a_paste_the_portal_cannot_take_stops_with_its_reason", &bus);
    assert_eq!(pastes, vec!["clipboard RemoteUnavailable: remote-desktop-portal: the portal's keyboard failed: CreateSession answered 2".to_string()]);
    let record = portal.record();
    assert_eq!((record.sessions, record.open_sessions(), record.dialogs), (1, 0, 0), "{record:?}");
}

/// docs/dictation.md §14.9 (goal gate round 2, 2026-11-09): once the chain has picked the portal,
/// a portal gone by the press (the press's own check hears that the interface is unknown) ends the
/// paste like any other failure of the portal: the text stays on the clipboard with
/// `remote_unavailable` (the 远程交互不可用 card). Before, it read as a desktop without the
/// interface: the chain went on, to XTEST on XWayland or to `no_tool` without a card on pure
/// Wayland. Only the chain's probe keeps a portal that is not there out of the chain.
#[test]
fn regression_a_portal_gone_after_the_probe_stops_with_its_reason() {
    if std::env::var_os(CHILD).is_some() {
        child_one_paste();
        return;
    }
    // The portal's own answer: the probe finds it, the press does not, nothing is asked for.
    let bus = PrivateBus::start();
    let portal = FakePortal::start(&bus);
    portal.fault(Fault::GoneAfterProbe);
    let desktop = client(&bus, &Arc::new(MemoryRestoreTokens::new()));
    assert_eq!(desktop.available(), Ok(()));
    let err = desktop.press(&ctrl_v()).unwrap_err();
    assert!(matches!(&err, PortalError::Unavailable(reason) if reason.contains("org.freedesktop.DBus.Error.UnknownInterface")), "{err:?}");
    let record = portal.record();
    assert_eq!((record.checks, record.sessions), (2, 0), "{record:?}");

    // Through the chain the shell builds, on pure Wayland: the probe passes, the press stops.
    let bus = PrivateBus::start();
    let portal = FakePortal::start(&bus);
    portal.fault(Fault::GoneAfterProbe);
    let pastes = run_child("regression_a_portal_gone_after_the_probe_stops_with_its_reason", &bus);
    let prefix = "clipboard RemoteUnavailable: remote-desktop-portal: no remote-desktop portal: org.freedesktop.portal.RemoteDesktop: ";
    assert!(matches!(pastes.as_slice(), [paste] if paste.starts_with(prefix) && paste.contains("UnknownInterface")), "{pastes:?}");
    let record = portal.record();
    assert_eq!((record.checks, record.sessions, record.dialogs), (2, 0, 0), "{record:?}");
}

/// A portal without `ConnectToEIS` (version 1) or without a keyboard, and no portal at all: the
/// next tool of the chain tries, and nothing is requested.
#[test]
fn portals_without_eis_or_a_keyboard_are_unavailable() {
    for (version, types) in [(1, KEYBOARD | POINTER), (2, POINTER)] {
        let bus = PrivateBus::start();
        let portal = FakePortal::with(&bus, version, types);
        let desktop = client(&bus, &Arc::new(MemoryRestoreTokens::new()));
        assert!(matches!(desktop.available(), Err(PortalError::Unavailable(_))), "version {version}, types {types}");
        assert!(matches!(desktop.press(&ctrl_v()), Err(PortalError::Unavailable(_))));
        assert_eq!(portal.record().sessions, 0);
    }
    let bus = PrivateBus::start();
    let desktop = client(&bus, &Arc::new(MemoryRestoreTokens::new()));
    let err = desktop.available().unwrap_err();
    assert!(matches!(&err, PortalError::Unavailable(reason) if reason.contains("org.freedesktop.portal.Desktop")), "{err:?}");
    assert!(client(&bus, &Arc::new(MemoryRestoreTokens::new())).available().is_err());
    let portal = FakePortal::start(&bus);
    assert_eq!(desktop.available(), Ok(()));
    assert_eq!(portal.record().sessions, 0, "availability asks for nothing");
}

/// A saved token the portal refuses outright (not a UUID): dropped, and the session asked for
/// once more without it.
#[test]
fn a_malformed_saved_token_is_dropped() {
    let bus = PrivateBus::start();
    let portal = FakePortal::start(&bus);
    let tokens = Arc::new(MemoryRestoreTokens::new());
    tokens.save("not-a-token");
    client(&bus, &tokens).press(&ctrl_v()).unwrap();
    let record = portal.record();
    assert_eq!(offered(&portal), vec![Some("not-a-token".to_string()), None]);
    assert_eq!((record.dialogs, record.sessions, record.open_sessions()), (1, 2, 0), "{record:?}");
    assert_eq!(tokens.load(), record.issued.last().cloned());
}

/// A grant an earlier run left in the permission store needs no dialog.
#[test]
fn a_grant_left_by_an_earlier_run_needs_no_dialog() {
    let bus = PrivateBus::start();
    let portal = FakePortal::start(&bus);
    let token = "00000000-0000-4000-8000-0000000000aa";
    portal.grant(token);
    let tokens = Arc::new(MemoryRestoreTokens::new());
    tokens.save(token);
    client(&bus, &tokens).press(&ctrl_v()).unwrap();
    let record = portal.record();
    assert_eq!((record.dialogs, record.restored), (0, 1), "{record:?}");
    assert_eq!(tokens.load().as_deref(), Some(token), "xdg-desktop-portal hands the same token back");
}

/// The keys come from the keymap the compositor announced: Dvorak's `v`, Ctrl+V on a Russian
/// layout, Control on Caps Lock, and the active layout of two.
#[test]
fn keys_follow_the_announced_layout() {
    let cases = [
        (Layout::names("us", "dvorak", None, 0), vec![(29, true), (52, true), (52, false), (29, false)]),
        (Layout::names("ru,us", ",", None, 0), CTRL_V.to_vec()),
        (Layout::names("us", "", Some("ctrl:swapcaps"), 0), vec![(58, true), (47, true), (47, false), (58, false)]),
        (Layout::names("us,us", "dvorak,", None, 0), vec![(29, true), (52, true), (52, false), (29, false)]),
        (Layout::names("us,us", "dvorak,", None, 1), CTRL_V.to_vec()),
    ];
    for (layout, expected) in cases {
        let bus = PrivateBus::start();
        let portal = FakePortal::start(&bus);
        portal.set_layout(layout.clone());
        client(&bus, &Arc::new(MemoryRestoreTokens::new())).press(&ctrl_v()).unwrap();
        assert_eq!(portal.record().keys, vec![expected], "{layout:?}");
    }
}

/// Ctrl+Shift+V, Shift+Insert, and the voice-editing copy (Ctrl+Insert) with the hotkey's
/// modifiers still held: the virtual keyboard does not release keys it never pressed.
#[test]
fn other_chords_and_the_copy() {
    let bus = PrivateBus::start();
    let portal = FakePortal::start(&bus);
    let desktop = client(&bus, &Arc::new(MemoryRestoreTokens::new()));
    let ctrl_shift_v = Chord { control: true, meta: false, shift: true, key: Key::Char('v') };
    let shift_insert = Chord { control: false, meta: false, shift: true, key: Key::Insert };
    desktop.press(&chord_steps(ctrl_shift_v, "linux", 0)).unwrap();
    desktop.press(&chord_steps(shift_insert, "linux", 0)).unwrap();
    desktop.press(&copy_steps(voltip_inject::copy_chord_for("linux"), "linux", 0, &[Modifier::Control, Modifier::Alt])).unwrap();
    assert_eq!(
        portal.record().keys,
        vec![
            vec![(29, true), (42, true), (47, true), (47, false), (42, false), (29, false)],
            vec![(42, true), (110, true), (110, false), (42, false)],
            vec![(29, true), (110, true), (110, false), (29, false)],
        ]
    );
    assert_eq!(portal.record().dialogs, 1);
}

/// Two chords at once (a paste and a copy): one waits for the other, and one grant serves both.
#[test]
fn concurrent_chords_share_one_grant() {
    let bus = PrivateBus::start();
    let portal = FakePortal::start(&bus);
    let tokens = Arc::new(MemoryRestoreTokens::new());
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let desktop = client(&bus, &tokens);
            std::thread::spawn(move || desktop.press(&ctrl_v()))
        })
        .collect();
    for thread in threads {
        thread.join().unwrap().unwrap();
    }
    let record = portal.record();
    assert_eq!((record.dialogs, record.restored, record.open_sessions()), (1, 1, 0), "{record:?}");
}

/// The keyboard fails after the grant. Before any key: broken, the grant is kept for the next
/// paste. During the chord: lost, and the chain must not paste again.
#[test]
fn keyboards_that_fail() {
    let bus = PrivateBus::start();
    let portal = FakePortal::start(&bus);
    let tokens = Arc::new(MemoryRestoreTokens::new());
    let desktop = client(&bus, &tokens);
    portal.fault(Fault::RefuseEis);
    assert!(matches!(desktop.press(&ctrl_v()), Err(PortalError::Broken(reason)) if reason.starts_with("ConnectToEIS")));
    portal.fault(Fault::CloseAtBind);
    assert_eq!(desktop.press(&ctrl_v()), Err(PortalError::Broken("the compositor closed the keyboard".into())));
    portal.fault(Fault::CloseAfterKeys(2));
    assert!(matches!(desktop.press(&ctrl_v()), Err(PortalError::Lost(_))));
    portal.fault(Fault::None);
    desktop.press(&ctrl_v()).unwrap();
    let record = portal.record();
    assert_eq!((record.dialogs, record.restored, record.open_sessions()), (1, 3, 0), "{record:?}");
    assert_eq!(record.keys.last(), Some(&CTRL_V.to_vec()));
}
