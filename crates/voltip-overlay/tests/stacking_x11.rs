//! The pill's place in the X11 stacking order (docs/dictation.md §32), against a real X server
//! without a window manager (Xvfb), where only the clients' own requests order the windows: the
//! pill, hidden after a take, is mapped again the way GTK shows it, and a window mapped in the
//! meantime must not stay over it once the shell raised it; while it shows, a window mapped over
//! it or restacked over it later must not stay there either while the shell's [`Keeper`] runs,
//! and without one it does. A menu or a tooltip (an override-redirect window) stays over the pill.
//! Checked by the stacking order and by the screen's pixels.
//!
//! It needs an X server, so it is ignored: CI runs it under Xvfb (`smoke-desktop`):
//! `xvfb-run -a cargo test -p voltip-overlay --test stacking_x11 -- --ignored --test-threads=1 --nocapture`.

#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::ffi::OsString;
use std::time::{Duration, Instant};

use voltip_overlay::contest::AGAIN_AFTER;
use voltip_overlay::x11::{Keeper, Stacker};
use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::{ConfigureWindowAux, ConnectionExt as _, CreateWindowAux, ImageFormat, MapState, StackMode, Window, WindowClass};
use x11rb::rust_connection::RustConnection;

/// The pill's colour (`0x00RRGGBB`): green.
const PILL: u32 = 0x0000_C000;
/// The covering window's colour: magenta.
const COVER: u32 = 0x00FF_00FF;
/// How long a change may take to show.
const WAIT: Duration = Duration::from_secs(5);

struct Server {
    display: OsString,
    conn: RustConnection,
    root: Window,
    depth: u8,
    visual: u32,
    width: i16,
    height: i16,
}

impl Server {
    fn open() -> Self {
        let display = std::env::var_os("DISPLAY").expect("DISPLAY: an X server (xvfb-run)");
        let (conn, screen) = x11rb::connect(display.to_str()).expect("the X server");
        let s = &conn.setup().roots[screen];
        let (root, depth, visual) = (s.root, s.root_depth, s.root_visual);
        let (width, height) = (i16::try_from(s.width_in_pixels).unwrap(), i16::try_from(s.height_in_pixels).unwrap());
        Self { display, conn, root, depth, visual, width, height }
    }

    /// A window of one colour, mapped.
    fn window(&self, x: i16, y: i16, width: u16, height: u16, color: u32) -> Window {
        self.window_with(x, y, width, height, CreateWindowAux::new().background_pixel(color))
    }

    /// The whole screen in the cover's colour, mapped on top.
    fn cover(&self) -> Window {
        self.window(0, 0, u16::try_from(self.width).unwrap(), u16::try_from(self.height).unwrap(), COVER)
    }

    /// A window made with `aux`, mapped.
    fn window_with(&self, x: i16, y: i16, width: u16, height: u16, aux: CreateWindowAux) -> Window {
        let id = self.conn.generate_id().unwrap();
        self.conn.create_window(self.depth, id, self.root, x, y, width, height, 0, WindowClass::INPUT_OUTPUT, self.visual, &aux).unwrap().check().unwrap();
        self.conn.map_window(id).unwrap().check().unwrap();
        id
    }

    /// Restack `window` on top, as a client brings its window to the front.
    fn raise(&self, window: Window) {
        self.conn.configure_window(window, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE)).unwrap().check().unwrap();
    }

    /// The pill shown as the shell shows it: mapped, then raised.
    fn pill(&self) -> Window {
        let (x, y) = self.pill_origin();
        let pill = self.window(x, y, 480, 64, PILL);
        Stacker::default().raise_on(Some(&self.display), pill).expect("the raise request");
        assert!(wait_until(|| self.top_at(x + 240, y + 32) == Some(pill)), "the pill did not come up on top");
        pill
    }

    /// The pill's centre.
    fn pill_centre(&self) -> (i16, i16) {
        let (x, y) = self.pill_origin();
        (x + 240, y + 32)
    }

    /// The pill is on top at its centre, by the stacking order and by the pixels.
    fn assert_pill_on_top(&self, pill: Window, what: &str) {
        let (cx, cy) = self.pill_centre();
        assert!(wait_until(|| self.top_at(cx, cy) == Some(pill)), "{what}: the pill is not on top: window {:?} is", self.top_at(cx, cy));
        assert!(wait_until(|| self.pixel(cx, cy) == PILL), "{what}: the pill is on top, but the screen shows {:#08x} at its centre", self.pixel(cx, cy));
    }

    /// The pill's place: centred, 24 px above the screen's bottom edge, 480 × 64.
    fn pill_origin(&self) -> (i16, i16) {
        ((self.width - 480) / 2, self.height - 64 - 24)
    }

    /// The mapped window on top at (`x`, `y`): the root's children run from the bottom up.
    fn top_at(&self, x: i16, y: i16) -> Option<Window> {
        let tree = self.conn.query_tree(self.root).unwrap().reply().unwrap();
        tree.children.into_iter().rev().find(|&w| {
            let viewable = self.conn.get_window_attributes(w).unwrap().reply().is_ok_and(|a| a.map_state == MapState::VIEWABLE);
            let geometry = self.conn.get_geometry(w).unwrap().reply();
            viewable
                && geometry.is_ok_and(|g| {
                    let (right, bottom) = (i32::from(g.x) + i32::from(g.width), i32::from(g.y) + i32::from(g.height));
                    g.x <= x && i32::from(x) < right && g.y <= y && i32::from(y) < bottom
                })
        })
    }

    /// The screen's colour at (`x`, `y`) (`0x00RRGGBB`).
    fn pixel(&self, x: i16, y: i16) -> u32 {
        let image = self.conn.get_image(ImageFormat::Z_PIXMAP, self.root, x, y, 1, 1, !0).unwrap().reply().unwrap();
        let bytes: [u8; 4] = image.data[..4].try_into().unwrap();
        u32::from_le_bytes(bytes) & 0x00ff_ffff
    }
}

fn wait_until(mut done: impl FnMut() -> bool) -> bool {
    let until = Instant::now() + WAIT;
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

#[test]
#[ignore = "needs an X server: CI runs it under Xvfb"]
fn regression_the_pill_mapped_again_comes_back_above_a_window_mapped_since() {
    let server = Server::open();
    let (x, y) = server.pill_origin();
    let pill = server.window(x, y, 480, 64, PILL);
    // The take ended: the pill is unmapped, and another window comes up over its place.
    server.conn.unmap_window(pill).unwrap().check().unwrap();
    let cover = server.window(0, 0, u16::try_from(server.width).unwrap(), u16::try_from(server.height).unwrap(), COVER);
    // The next take: GTK maps the pill again, which keeps its old place, then the shell raises it.
    server.conn.map_window(pill).unwrap().check().unwrap();
    Stacker::default().raise_on(Some(&server.display), pill).expect("the raise request");
    let (cx, cy) = (x + 240, y + 32);
    assert!(wait_until(|| server.top_at(cx, cy) == Some(pill)), "the pill is not on top: window {:?} is (the cover is {cover})", server.top_at(cx, cy));
    assert!(wait_until(|| server.pixel(cx, cy) == PILL), "the pill is on top, but the screen shows {:#08x} at its centre", server.pixel(cx, cy));
}

#[test]
#[ignore = "needs an X server: CI runs it under Xvfb"]
fn regression_the_pill_goes_back_above_windows_mapped_or_raised_over_it_while_it_shows() {
    let server = Server::open();
    let pill = server.pill();
    let keeper = Keeper::start_on(Some(&server.display), pill).expect("the keeper").expect("a display");
    // While the pill shows, another window comes up over its place.
    let cover = server.cover();
    server.assert_pill_on_top(pill, "a window mapped over the pill while it shows");
    // Later, that window is brought to the front again (a new event, not the window fighting back).
    std::thread::sleep(AGAIN_AFTER + Duration::from_millis(100));
    server.raise(cover);
    server.assert_pill_on_top(pill, "the same window raised over the pill again while it shows");
    drop(keeper);
}

/// What the regression above guards against: with nothing restacking the pill while it shows, a
/// window mapped over it and a window raised over it each end up on top.
#[test]
#[ignore = "needs an X server: CI runs it under Xvfb"]
fn without_a_keeper_windows_mapped_or_raised_over_the_pill_cover_it() {
    let server = Server::open();
    let pill = server.pill();
    let (cx, cy) = server.pill_centre();
    let cover = server.cover();
    assert!(wait_until(|| server.top_at(cx, cy) == Some(cover)), "a window mapped over the pill should cover it");
    server.raise(pill);
    server.assert_pill_on_top(pill, "raised again");
    server.raise(cover);
    assert!(wait_until(|| server.top_at(cx, cy) == Some(cover)), "a window raised over the pill should cover it");
}

/// A menu or a tooltip (an override-redirect window) opened over the pill while it shows is what
/// the user works in: the keeper leaves it there.
#[test]
#[ignore = "needs an X server: CI runs it under Xvfb"]
fn a_menu_over_the_pill_stays_over_it() {
    let server = Server::open();
    let pill = server.pill();
    let keeper = Keeper::start_on(Some(&server.display), pill).expect("the keeper").expect("a display");
    let (x, y) = server.pill_origin();
    let menu = server.window_with(x - 40, y - 40, 560, 144, CreateWindowAux::new().background_pixel(COVER).override_redirect(1));
    let (cx, cy) = server.pill_centre();
    assert!(wait_until(|| server.top_at(cx, cy) == Some(menu)), "the menu did not come up over the pill");
    // The keeper answers a window mapped within milliseconds: a second is ample.
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(server.top_at(cx, cy), Some(menu), "the pill was restacked over a menu");
    drop(keeper);
}
