//! The real app's pill on a real Windows desktop (docs/dictation.md §32): Voltip is started, a
//! window of this test is brought to the front the way a user would (a click), a take is started
//! with `voltip-desktop --toggle`, and the pill must be what the screen shows at its centre, by the
//! Z order and by the pixels, while the window in front stays in front. The windows are a topmost
//! one (a pinned window or a player), a topmost one over the whole monitor (a game or a player in
//! full screen), the same activated while the pill shows, the window in front pinned on top (then
//! unpinned and pinned again at once) and a topmost window coming up while the pill shows, and an ordinary
//! one and an ordinary full-screen one (a browser after F11).
//!
//! A runner has no microphone: the take fails at once and its pill shows for the 2.5 s dwell,
//! which every check fits into.
//!
//! It starts the app and clicks windows, so it is ignored: CI runs it on windows-2025
//! (`windows-native`) against the portable build, never on a computer someone is using:
//! `VOLTIP_DESKTOP_EXE=<voltip-desktop.exe> cargo test -p voltip-overlay --test app_windows -- --ignored --test-threads=1 --nocapture`.
//! The app's log goes to `VOLTIP_OVERLAY_APP_LOG`, or else `%TEMP%\voltip-overlay-app.log`.

#![cfg(windows)]
#![allow(unsafe_code, clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use support::{COVER_COLOR, Spec, TestWindow, above, center, describe, dpi_aware, foreground, pixel, rect_of, same_color, top_at, wait_until, work_area};
use voltip_overlay::contest::AGAIN_AFTER;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible};

/// How long the app may take to start and to make its pill window.
const START: Duration = Duration::from_secs(90);
/// How long the pill may take to appear after `--toggle`, or to go after `--cancel`.
const PILL: Duration = Duration::from_secs(20);
/// How long a check may take once the pill shows: well inside its 2.5 s dwell.
const CHECK: Duration = Duration::from_millis(1500);

/// The pill window of process `pid` (titled "Voltip Overlay"), shown or not; 0 when none.
fn find_pill(pid: u32) -> isize {
    struct Search {
        pid: u32,
        found: isize,
    }
    unsafe extern "system" fn visit(hwnd: HWND, lparam: isize) -> i32 {
        // SAFETY: `lparam` is the Search handed to EnumWindows below; the buffer has the length passed.
        unsafe {
            let search = &mut *(lparam as *mut Search);
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &raw mut pid);
            if pid != search.pid {
                return 1;
            }
            let mut title = [0u16; 64];
            let n = GetWindowTextW(hwnd, title.as_mut_ptr(), 64);
            if String::from_utf16_lossy(&title[..usize::try_from(n).unwrap_or(0).min(64)]) == "Voltip Overlay" {
                search.found = hwnd as isize;
                return 0;
            }
            1
        }
    }
    let mut search = Search { pid, found: 0 };
    // SAFETY: the callback only touches `search`, which outlives the call.
    unsafe { EnumWindows(Some(visit), (&raw mut search) as isize) };
    search.found
}

fn visible(hwnd: isize) -> bool {
    // SAFETY: plain query on a window handle.
    unsafe { IsWindowVisible(hwnd as HWND) != 0 }
}

/// The running app; `--quit` and reaped on drop.
struct App {
    exe: PathBuf,
    child: Child,
    pill: isize,
}

impl App {
    fn start(name: &str) -> Self {
        dpi_aware();
        let exe = PathBuf::from(std::env::var_os("VOLTIP_DESKTOP_EXE").expect("VOLTIP_DESKTOP_EXE: the voltip-desktop.exe to drive"));
        let log = std::env::var_os("VOLTIP_OVERLAY_APP_LOG").map_or_else(|| std::env::temp_dir().join("voltip-overlay-app.log"), PathBuf::from);
        let mut log = std::fs::OpenOptions::new().create(true).append(true).open(log).expect("the app's log file");
        let _ = writeln!(log, "=== {name}");
        let child = Command::new(&exe)
            .arg("--start-hidden")
            .env("RUST_LOG", "voltip=info")
            .env("NO_COLOR", "1")
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .expect("voltip-desktop.exe");
        let mut app = Self { exe, child, pill: 0 };
        let pid = app.child.id();
        assert!(
            wait_until(START, || {
                app.pill = find_pill(pid);
                app.pill != 0
            }),
            "the app made no pill window"
        );
        // One take with nothing in front, so that the pill's page has drawn once before any check.
        // The pill window comes up before the core does, and a `--toggle` that arrives before it
        // is dropped ("remote control before the core is up"): ask again until a take shows.
        let until = Instant::now() + START;
        loop {
            app.remote("--toggle");
            if wait_until(Duration::from_secs(3), || visible(app.pill)) {
                break;
            }
            assert!(Instant::now() < until, "no take showed a pill");
        }
        app.end();
        app
    }

    /// `voltip-desktop <flag>`: the running instance does it, the second one exits.
    fn remote(&self, flag: &str) {
        let status = Command::new(&self.exe).arg(flag).stdout(Stdio::null()).stderr(Stdio::null()).status().expect("the second instance");
        assert!(status.success(), "voltip-desktop {flag} exited with {status}");
    }

    /// Start a take and wait for its pill.
    fn take(&self) {
        self.remote("--toggle");
        assert!(wait_until(PILL, || visible(self.pill)), "the take showed no pill");
    }

    /// End the take (or its outcome) and wait for the pill to go.
    fn end(&self) {
        self.remote("--cancel");
        assert!(wait_until(PILL, || !visible(self.pill)), "the pill stayed after the take");
    }

    /// The pill is what the screen shows at its centre: on top in the Z order, and something other
    /// than the window under it drawn there.
    fn assert_on_top(&self, what: &str) {
        let at = center(&rect_of(self.pill));
        let started = Instant::now();
        let on_top = wait_until(CHECK, || top_at(at) == self.pill);
        assert!(on_top, "{what}: the pill is not on top. On top: {}{}", describe(top_at(at)), above(self.pill, at));
        let painted = wait_until(CHECK, || !same_color(pixel(at), COVER_COLOR));
        assert!(painted, "{what}: the pill is on top in the Z order, but the screen still shows the window under it at its centre");
        println!("{what}: the pill on top and drawn after {} ms ({})", started.elapsed().as_millis(), describe(self.pill));
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = Command::new(&self.exe).arg("--quit").stdout(Stdio::null()).stderr(Stdio::null()).status();
        let until = Instant::now() + Duration::from_secs(30);
        while Instant::now() < until {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A take in front of `cover`, which the user clicked first.
fn take_in_front_of(app: &App, cover: &TestWindow, what: &str) {
    cover.activate(what);
    app.take();
    app.assert_on_top(what);
    assert_eq!(foreground(), cover.hwnd, "{what}: the take took the window in front's place");
    app.end();
}

#[test]
#[ignore = "starts the app and clicks windows: CI's windows-2025 desktop only"]
fn regression_the_real_pill_shows_above_a_topmost_window_in_front() {
    let app = App::start("regression_the_real_pill_shows_above_a_topmost_window_in_front");
    let cover = TestWindow::open(Spec::ordinary().topmost());
    take_in_front_of(&app, &cover, "a topmost window in front");
}

#[test]
#[ignore = "starts the app and clicks windows: CI's windows-2025 desktop only"]
fn regression_the_real_pill_shows_above_a_topmost_full_screen_window_in_front() {
    let app = App::start("regression_the_real_pill_shows_above_a_topmost_full_screen_window_in_front");
    let cover = TestWindow::open(Spec::full_screen().topmost());
    take_in_front_of(&app, &cover, "a topmost full-screen window in front");
}

#[test]
#[ignore = "starts the app and clicks windows: CI's windows-2025 desktop only"]
fn regression_the_real_pill_goes_back_above_a_topmost_window_activated_while_it_shows() {
    let app = App::start("regression_the_real_pill_goes_back_above_a_topmost_window_activated_while_it_shows");
    let first = TestWindow::open(Spec::ordinary());
    first.activate("an ordinary window in front");
    // Made now and shown later, so that bringing it up fits in the pill's dwell.
    let cover = TestWindow::open(Spec { rect: work_area(), shown: false, ..Spec::full_screen().topmost() });
    app.take();
    app.assert_on_top("before the topmost window came to the front");
    cover.show_like_tao();
    cover.activate("the topmost window");
    app.assert_on_top("a topmost window activated while the pill shows");
    assert_eq!(foreground(), cover.hwnd, "the pill took the window in front's place");
    app.end();
}

#[test]
#[ignore = "starts the app and clicks windows: CI's windows-2025 desktop only"]
fn regression_the_real_pill_goes_back_above_windows_pinned_or_shown_while_it_shows() {
    let app = App::start("regression_the_real_pill_goes_back_above_windows_pinned_or_shown_while_it_shows");
    let front = TestWindow::open(Spec::ordinary());
    front.activate("an ordinary window in front");
    app.take();
    app.assert_on_top("before the window in front was pinned on top");
    // No window comes to the front: the one in front is pinned on top where it is.
    let first_pin = Instant::now();
    front.pin();
    app.assert_on_top("the window in front pinned on top while the pill shows");
    // Unpinned and pinned again at once, within `AGAIN_AFTER` of the raise over it: the keeper
    // looks again once that has passed (docs/dictation.md §32.2).
    front.unpin();
    front.pin();
    println!("pinned again {} ms after the first pin (AGAIN_AFTER is {} ms)", first_pin.elapsed().as_millis(), AGAIN_AFTER.as_millis());
    app.assert_on_top("the same window pinned on top again at once while the pill shows");
    assert_eq!(foreground(), front.hwnd, "the pill took the window in front's place");
    app.end();
    app.take();
    app.assert_on_top("before a topmost window came up");
    // A topmost window that comes up without being activated.
    let shown = TestWindow::open(Spec { rect: work_area(), ..Spec::full_screen().topmost() });
    app.assert_on_top("a topmost window shown while the pill shows");
    assert_eq!(foreground(), front.hwnd, "the pill took the window in front's place");
    app.end();
    drop(shown);
}

#[test]
#[ignore = "starts the app and clicks windows: CI's windows-2025 desktop only"]
fn the_real_pill_shows_above_ordinary_and_full_screen_windows_in_front() {
    let app = App::start("the_real_pill_shows_above_ordinary_and_full_screen_windows_in_front");
    let ordinary = TestWindow::open(Spec::ordinary());
    take_in_front_of(&app, &ordinary, "an ordinary window in front");
    drop(ordinary);
    let full_screen = TestWindow::open(Spec::full_screen());
    take_in_front_of(&app, &full_screen, "an ordinary full-screen window in front");
}
