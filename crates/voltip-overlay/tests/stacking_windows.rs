//! The pill's place in the Z order on a real Windows desktop (docs/dictation.md §32). The pill is a
//! window of the test's own made the way tao makes Voltip's (topmost, never activated, hidden),
//! shown the way tao shows it (`ShowWindow(SW_SHOWNOACTIVATE)`) and then raised as the shell
//! raises it ([`raise`]); the windows in front of it are activated with a click, as the user
//! would. The pill must be what the screen shows at its centre, by the Z order and by the pixels,
//! and the window the user activated must stay in front.
//!
//! Reported 2026-11-01 on the 0.3.0 candidate: with many windows open, after clicking one of them,
//! or with a program in full screen, the pill was nowhere to be seen during the whole take and the
//! user thought it had failed. Showing a hidden window keeps its old place among the topmost
//! windows, tao's `set_always_on_top(true)` does nothing while the window already is, and nothing
//! raised the pill while it showed.
//!
//! It opens windows and clicks them, so it is ignored: CI runs it on the windows-2025 desktop
//! (`hooks-windows`), never on a computer someone is using:
//! `cargo test -p voltip-overlay --test stacking_windows -- --ignored --test-threads=1 --nocapture`.

#![cfg(windows)]
#![allow(unsafe_code, clippy::unwrap_used, clippy::expect_used)]

mod support;

use support::{Spec, TestWindow, assert_pill_on_top, center, dpi_aware, foreground, is_topmost, top_at, wait_until, work_area};
use voltip_overlay::windows::{Keeper, raise};
use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::UI::WindowsAndMessaging::{HWND_NOTOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos};

/// Show the pill as the shell does: tao's show, then the raise.
fn show(pill: &TestWindow) {
    pill.show_like_tao();
    assert!(raise(pill.hwnd), "SetWindowPos refused to raise the pill");
}

/// The pill came up with the window in front still in front.
fn assert_focus_kept(cover: &TestWindow) {
    assert_eq!(foreground(), cover.hwnd, "the pill took the window in front's place");
}

#[test]
#[ignore = "opens windows and clicks them: CI's windows-2025 desktop only"]
fn regression_the_pill_shows_above_a_topmost_window_activated_while_it_was_hidden() {
    dpi_aware();
    let pill = TestWindow::open(Spec::pill());
    let cover = TestWindow::open(Spec::ordinary().topmost());
    cover.activate("the topmost window");
    show(&pill);
    assert_pill_on_top(pill.hwnd, "a topmost window activated while the pill was hidden");
    assert_focus_kept(&cover);
}

#[test]
#[ignore = "opens windows and clicks them: CI's windows-2025 desktop only"]
fn regression_the_pill_shows_above_a_topmost_full_screen_window() {
    dpi_aware();
    let pill = TestWindow::open(Spec::pill());
    let cover = TestWindow::open(Spec::full_screen().topmost());
    cover.activate("the topmost full-screen window");
    show(&pill);
    assert_pill_on_top(pill.hwnd, "a full-screen window on top of the others");
    assert_focus_kept(&cover);
}

#[test]
#[ignore = "opens windows and clicks them: CI's windows-2025 desktop only"]
fn regression_the_pill_goes_back_above_a_topmost_window_activated_while_it_shows() {
    dpi_aware();
    let pill = TestWindow::open(Spec::pill());
    let first = TestWindow::open(Spec::ordinary());
    first.activate("the window in front");
    show(&pill);
    let keeper = Keeper::start(pill.hwnd).expect("the keeper");
    assert_pill_on_top(pill.hwnd, "before the other window came to the front");
    // A pinned or full-screen player brought to the front during the take.
    let cover = TestWindow::open(Spec { rect: work_area(), ..Spec::full_screen().topmost() });
    cover.activate("the topmost window");
    assert_pill_on_top(pill.hwnd, "a topmost window activated while the pill shows");
    assert_focus_kept(&cover);
    drop(keeper);
}

#[test]
#[ignore = "opens windows and clicks them: CI's windows-2025 desktop only"]
fn regression_a_pill_put_among_the_ordinary_windows_is_made_topmost_again() {
    dpi_aware();
    let pill = TestWindow::open(Spec::pill());
    // What another program can do to it: place it after an ordinary window, or say HWND_NOTOPMOST.
    // SAFETY: a live window handle of this test.
    unsafe { SetWindowPos(pill.hwnd as _, HWND_NOTOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) };
    assert!(wait_until(support::WAIT, || !is_topmost(pill.hwnd)), "the pill could not be taken out of the topmost windows");
    let cover = TestWindow::open(Spec::ordinary());
    cover.activate("the ordinary window");
    show(&pill);
    assert_pill_on_top(pill.hwnd, "an ordinary window over a pill that had lost its topmost place");
    assert!(is_topmost(pill.hwnd), "the pill is on top but not among the topmost windows");
    assert_focus_kept(&cover);
}

#[test]
#[ignore = "opens windows and clicks them: CI's windows-2025 desktop only"]
fn the_pill_shows_above_ordinary_windows_activated_over_it() {
    dpi_aware();
    let pill = TestWindow::open(Spec::pill());
    // Many windows over the pill's place, each 50 px right of the last so that a click can reach
    // any of them, and the one clicked first over the lower half of the screen.
    let area = work_area();
    let others: Vec<TestWindow> = (0..6).map(|i| TestWindow::open(Spec { rect: RECT { left: area.left + 50 * i, ..area }, ..Spec::ordinary() })).collect();
    let cover = TestWindow::open(Spec { rect: RECT { top: (area.top + area.bottom) / 2, ..area }, ..Spec::ordinary() });
    cover.activate("the ordinary window");
    show(&pill);
    assert_pill_on_top(pill.hwnd, "an ordinary window activated over the pill's place");
    assert_focus_kept(&cover);
    // A take under way, another ordinary window clicked: it stays under the pill.
    let keeper = Keeper::start(pill.hwnd).expect("the keeper");
    let last = others.last().expect("the other windows");
    last.activate("another ordinary window");
    assert_pill_on_top(pill.hwnd, "an ordinary window activated while the pill shows");
    assert_focus_kept(last);
    drop(keeper);
}

#[test]
#[ignore = "opens windows and clicks them: CI's windows-2025 desktop only"]
fn the_pill_shows_above_an_ordinary_full_screen_window() {
    dpi_aware();
    let pill = TestWindow::open(Spec::pill());
    // A borderless window over the whole monitor in front: Windows treats it as full screen
    // (the taskbar steps back), a browser after F11 or a video player.
    let cover = TestWindow::open(Spec::full_screen());
    cover.activate("the full-screen window");
    show(&pill);
    assert_pill_on_top(pill.hwnd, "an ordinary full-screen window");
    assert_focus_kept(&cover);
}

#[test]
#[ignore = "opens windows and clicks them: CI's windows-2025 desktop only"]
fn a_dropped_keeper_raises_nothing_more() {
    dpi_aware();
    let pill = TestWindow::open(Spec::pill());
    let first = TestWindow::open(Spec::ordinary());
    first.activate("the window in front");
    show(&pill);
    drop(Keeper::start(pill.hwnd).expect("the keeper"));
    // With the pill hidden and its keeper gone, a topmost window activated stays over its place.
    pill.hide();
    let cover = TestWindow::open(Spec::ordinary().topmost());
    cover.activate("the topmost window");
    pill.show_like_tao();
    let at = center(&pill.rect());
    assert!(wait_until(support::WAIT, || top_at(at) == cover.hwnd), "the topmost window should be on top while nothing raised the pill");
}
