//! The pill's stacking on a real macOS desktop (docs/dictation.md §32). The pill is a window of the
//! test's own made the way tao makes Voltip's (borderless, at the floating level, on every Space)
//! and shown the way tao shows it (`makeKeyAndOrderFront`). In front comes a window of another
//! process (this binary started again as the "cover"), and that application is activated: one at
//! the floating level, as a pinned note or a player kept on top is, and one in full screen, in a
//! Space of its own. Shown that way, a floating window of another application brought to the front
//! covers the pill; raised as the shell raises it ([`raise`]), the pill must be the window a click
//! at its centre would reach, before and after another such window comes to the front, and the
//! other application must stay active. Over an application in full screen, only a non-activating
//! panel comes up, not tao's window (docs/dictation.md §32.3).
//!
//! AppKit wants the main thread, so this test has its own `main` (`harness = false`): the main
//! thread runs the application, the checks run on another thread and hand AppKit work to the main
//! queue. It brings windows to the front, so it runs only when asked: CI's `macos` legs run
//! `cargo test -p voltip-overlay --test stacking_macos -- --ignored`. Elsewhere, and without
//! `--ignored`, it does nothing.

#![allow(unsafe_code, clippy::unwrap_used, clippy::expect_used)]

#[cfg(target_os = "macos")]
mod mac {
    use std::cell::RefCell;
    use std::ffi::c_void;
    use std::io::{BufRead as _, BufReader, Write as _};
    use std::process::{Child, Command, ExitCode, Stdio};
    use std::sync::{Mutex, mpsc};
    use std::time::{Duration, Instant};

    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::{MainThreadMarker, MainThreadOnly as _};
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSColor, NSFloatingWindowLevel, NSPanel, NSScreen, NSWindow,
        NSWindowCollectionBehavior, NSWindowStyleMask, NSWorkspace,
    };
    use objc2_foundation::{NSOperationQueue, NSPoint, NSRect, NSSize};
    use voltip_overlay::macos::raise;

    /// How long a window or an application may take to come up, to the front or on top.
    const WAIT: Duration = Duration::from_secs(30);
    /// How long the cover waits for full screen after asking, and how many times it asks.
    const FULL_SCREEN_TRY: Duration = Duration::from_secs(10);
    const FULL_SCREEN_TRIES: u32 = 4;
    /// The environment variable that turns this binary into the cover.
    const COVER: &str = "VOLTIP_OVERLAY_COVER";

    thread_local! {
        /// The test's windows, on the main thread (AppKit objects never leave it).
        static WINDOWS: RefCell<Vec<Retained<NSWindow>>> = const { RefCell::new(Vec::new()) };
    }

    /// Run `job` on the main thread and wait for its answer.
    fn on_main<T: Send + 'static>(job: impl FnOnce(MainThreadMarker) -> T + Send + 'static) -> T {
        let (tx, rx) = mpsc::channel();
        let job = Mutex::new(Some((job, tx)));
        let block = RcBlock::new(move || {
            if let Some((job, tx)) = job.lock().unwrap().take() {
                let _ = tx.send(job(MainThreadMarker::new().expect("the main queue runs on the main thread")));
            }
        });
        // SAFETY: the block only moves `Send` values into the main thread and runs there once.
        unsafe { NSOperationQueue::mainQueue().addOperationWithBlock(&block) };
        rx.recv_timeout(WAIT).expect("the main thread answered")
    }

    /// The main screen's visible frame (without the menu bar and the Dock), in screen points.
    fn visible_frame(mtm: MainThreadMarker) -> NSRect {
        NSScreen::mainScreen(mtm).expect("a main screen").visibleFrame()
    }

    /// Where Voltip puts the pill: centred, 24 points above the visible frame's bottom edge.
    fn pill_frame(mtm: MainThreadMarker) -> NSRect {
        let area = visible_frame(mtm);
        NSRect::new(NSPoint::new(area.origin.x + (area.size.width - 480.0) / 2.0, area.origin.y + 24.0), NSSize::new(480.0, 64.0))
    }

    fn window(mtm: MainThreadMarker, frame: NSRect, style: NSWindowStyleMask) -> Retained<NSWindow> {
        // SAFETY: a new window on the main thread, kept alive by the caller.
        let window = unsafe { NSWindow::initWithContentRect_styleMask_backing_defer(NSWindow::alloc(mtm), frame, style, NSBackingStoreType::Buffered, false) };
        // SAFETY: the window is owned by the returned `Retained`, not released by AppKit.
        unsafe { window.setReleasedWhenClosed(false) };
        window
    }

    /// A window like Voltip's pill, hidden (an earlier test's is closed first); its window number.
    /// `panel`: a non-activating panel (`NSPanel`) instead of tao's window, which the full-screen
    /// check compares.
    fn make_pill(panel: bool) -> isize {
        on_main(move |mtm| {
            WINDOWS.with_borrow_mut(|windows| windows.drain(..).for_each(|w| w.orderOut(None)));
            let frame = pill_frame(mtm);
            let pill = if panel {
                let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
                let panel = NSPanel::initWithContentRect_styleMask_backing_defer(NSPanel::alloc(mtm), frame, style, NSBackingStoreType::Buffered, false);
                // SAFETY: the panel is owned by the returned `Retained`, not released by AppKit.
                unsafe { panel.setReleasedWhenClosed(false) };
                panel.into_super()
            } else {
                window(mtm, frame, NSWindowStyleMask::Borderless)
            };
            pill.setBackgroundColor(Some(&NSColor::greenColor()));
            pill.setLevel(NSFloatingWindowLevel);
            pill.setCollectionBehavior(NSWindowCollectionBehavior::CanJoinAllSpaces);
            let number = pill.windowNumber();
            WINDOWS.with_borrow_mut(|windows| windows.push(pill));
            number
        })
    }

    /// Show the pill as tao shows it (`makeKeyAndOrderFront`).
    fn show_pill_like_tao() {
        on_main(|_| WINDOWS.with_borrow(|windows| windows[0].makeKeyAndOrderFront(None)));
    }

    /// Raise the pill as the shell does after showing it.
    fn raise_pill() {
        on_main(|_| WINDOWS.with_borrow(|windows| raise(Retained::as_ptr(&windows[0]).cast_mut().cast::<c_void>())));
    }

    fn close_pill() {
        on_main(|_| WINDOWS.with_borrow_mut(|windows| windows.drain(..).for_each(|w| w.orderOut(None))));
    }

    /// The test application's activation policy: regular (Voltip with a Dock icon) or accessory
    /// (Voltip started in the menu bar, docs/dictation.md §15.2).
    fn set_policy(policy: NSApplicationActivationPolicy) {
        let _ = on_main(move |mtm| NSApplication::sharedApplication(mtm).setActivationPolicy(policy));
    }

    /// The pill's centre, in screen points.
    fn pill_centre() -> NSPoint {
        on_main(|mtm| {
            let frame = pill_frame(mtm);
            NSPoint::new(frame.origin.x + frame.size.width / 2.0, frame.origin.y + frame.size.height / 2.0)
        })
    }

    /// The window a click at `at` would reach, of any application.
    fn window_at(at: NSPoint) -> isize {
        on_main(move |mtm| NSWindow::windowNumberAtPoint_belowWindowWithWindowNumber(at, 0, mtm))
    }

    fn frontmost_pid() -> Option<i32> {
        NSWorkspace::sharedWorkspace().frontmostApplication().map(|app| app.processIdentifier())
    }

    fn wait_until(done: impl FnMut() -> bool) -> bool {
        wait_until_within(WAIT, done)
    }

    fn wait_until_within(within: Duration, mut done: impl FnMut() -> bool) -> bool {
        let until = Instant::now() + within;
        loop {
            if done() {
                return true;
            }
            if Instant::now() >= until {
                return false;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Another application with one window, brought to the front: `floating` or `full-screen`.
    /// Ended on drop.
    struct Cover {
        child: Child,
        window: isize,
    }

    impl Cover {
        fn open(kind: &str) -> Self {
            let mut child = Command::new(std::env::current_exe().unwrap()).env(COVER, kind).stdout(Stdio::piped()).spawn().expect("the cover");
            let stdout = child.stdout.take().unwrap();
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    let _ = tx.send(line);
                }
            });
            let mut cover = Self { child, window: 0 };
            let ready = rx.recv_timeout(WAIT).expect("the cover's window");
            cover.window = ready.strip_prefix("window ").and_then(|n| n.parse().ok()).expect("the cover's window number");
            if kind == "full-screen" {
                let entering = FULL_SCREEN_TRY * FULL_SCREEN_TRIES + WAIT;
                assert_eq!(rx.recv_timeout(entering).as_deref(), Ok("full-screen"), "the cover did not go full screen");
            }
            let pid = cover.pid();
            assert!(wait_until(|| frontmost_pid() == Some(pid)), "the cover did not come to the front (in front: {:?})", frontmost_pid());
            cover
        }

        fn pid(&self) -> i32 {
            i32::try_from(self.child.id()).unwrap()
        }
    }

    impl Drop for Cover {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    /// What AppKit says of the pill: its level, collection behaviour, and whether it shows on the
    /// Space in front.
    fn pill_state() -> String {
        on_main(|_| {
            WINDOWS.with_borrow(|windows| {
                let pill = &windows[0];
                format!(
                    "level {}, behaviour {:#x}, visible {}, on the active Space {}",
                    pill.level(),
                    pill.collectionBehavior().0,
                    pill.isVisible(),
                    pill.isOnActiveSpace()
                )
            })
        })
    }

    /// The pill is the window a click at its centre reaches, and the cover's application is still
    /// the active one.
    fn assert_pill_on_top(pill: isize, cover: &Cover, what: &str) {
        let at = pill_centre();
        assert!(
            wait_until(|| window_at(at) == pill),
            "{what}: the pill is not on top (window {} is, the cover is {}; the pill: {})",
            window_at(at),
            cover.window,
            pill_state()
        );
        assert_eq!(frontmost_pid(), Some(cover.pid()), "{what}: showing the pill activated another application");
        println!("stacking_macos: {what}: the pill is on top");
    }

    fn regression_the_pill_stays_above_floating_windows_of_another_application_brought_to_the_front() {
        let pill = make_pill(false);
        show_pill_like_tao();
        let at = pill_centre();
        // What tao's floating level does: another application's floating window brought to the
        // front covers the pill (before 2026-11-01, the whole take).
        let first = Cover::open("floating");
        assert!(
            wait_until(|| window_at(at) == first.window),
            "a floating window of another application should cover a pill at tao's level (window {} is on top, the pill is {pill})",
            window_at(at)
        );
        println!("stacking_macos: at tao's floating level, the other application's floating window covers the pill");
        raise_pill();
        assert_pill_on_top(pill, &first, "raised over a floating window of another application");
        // While the pill shows, another floating window of another application comes to the front.
        let second = Cover::open("floating");
        assert_pill_on_top(pill, &second, "a floating window of another application brought to the front while the pill shows");
        close_pill();
    }

    /// docs/dictation.md §32.3: an application in full screen has a Space of its own. A pill raised
    /// as the shell raises it stays off that Space as tao's window, with the test application
    /// regular (Voltip with a Dock icon) or accessory (started in the menu bar); as a non-activating
    /// panel it is on top there, and the other application stays active.
    fn full_screen_spaces_take_a_non_activating_panel_and_not_tao_s_window() {
        for panel in [false, true] {
            let pill = make_pill(panel);
            let cover = Cover::open("full-screen");
            let at = pill_centre();
            // Its full-screen Space is in front once the cover's window is at the pill's place:
            // until the Space has slid in, the desktop and the pill on it still show (run
            // 41330006513, Intel).
            assert!(
                wait_until(|| window_at(at) == cover.window),
                "the cover's full-screen Space did not come to the front (window {} is at the pill's place)",
                window_at(at)
            );
            for (policy, name) in [(NSApplicationActivationPolicy::Regular, "regular"), (NSApplicationActivationPolicy::Accessory, "accessory")] {
                set_policy(policy);
                show_pill_like_tao();
                raise_pill();
                let what = format!("full screen, {} in a {name} application", if panel { "a non-activating panel" } else { "tao's window" });
                if panel {
                    assert_pill_on_top(pill, &cover, &what);
                } else {
                    // A window that never comes up: give it the time a raise takes, three times over.
                    let on_top = wait_until_within(Duration::from_secs(3), || window_at(at) == pill);
                    assert!(!on_top, "{what}: the pill is on top: macOS now puts tao's window on a full-screen Space (update docs/dictation.md §32.3)");
                    println!("stacking_macos: {what}: off the full-screen Space ({})", pill_state());
                }
            }
            set_policy(NSApplicationActivationPolicy::Regular);
            close_pill();
        }
    }

    /// The checks, on their own thread while the main thread runs the application.
    fn checks() -> bool {
        let tests: [(&str, fn()); 2] = [
            (
                "regression_the_pill_stays_above_floating_windows_of_another_application_brought_to_the_front",
                regression_the_pill_stays_above_floating_windows_of_another_application_brought_to_the_front,
            ),
            ("full_screen_spaces_take_a_non_activating_panel_and_not_tao_s_window", full_screen_spaces_take_a_non_activating_panel_and_not_tao_s_window),
        ];
        let mut passed = true;
        for (name, test) in tests {
            let ok = std::panic::catch_unwind(test).is_ok();
            println!("test {name} ... {}", if ok { "ok" } else { "FAILED" });
            passed &= ok;
        }
        passed
    }

    /// The cover: one magenta window in front of everything else of its level, at the floating
    /// level over the visible frame, or a titled window that goes full screen. It prints
    /// `window <number>` (and `full-screen` once it is) and runs until it is ended.
    fn cover(kind: &str) -> ExitCode {
        let mtm = MainThreadMarker::new().expect("the main thread");
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        let full_screen = kind == "full-screen";
        let window = if full_screen {
            let window =
                window(mtm, NSRect::new(NSPoint::new(100.0, 100.0), NSSize::new(640.0, 400.0)), NSWindowStyleMask::Titled | NSWindowStyleMask::Resizable);
            window.setCollectionBehavior(NSWindowCollectionBehavior::FullScreenPrimary);
            window
        } else {
            let window = window(mtm, visible_frame(mtm), NSWindowStyleMask::Borderless);
            window.setLevel(NSFloatingWindowLevel);
            window
        };
        window.setBackgroundColor(Some(&NSColor::magentaColor()));
        window.makeKeyAndOrderFront(None);
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
        println!("window {}", window.windowNumber());
        let _ = std::io::stdout().flush();
        if full_screen {
            // Once the application runs: toggle full screen, then say when it is. macOS drops the
            // request while another application's full-screen Space is still closing (the earlier
            // cover's, just ended: run 41330001090), so a window not in full screen after a while
            // asks again.
            std::thread::spawn(|| {
                let in_full_screen = || on_main(|_| WINDOWS.with_borrow(|windows| windows[0].styleMask().contains(NSWindowStyleMask::FullScreen)));
                let entered = (0..FULL_SCREEN_TRIES).any(|_| {
                    on_main(|_| WINDOWS.with_borrow(|windows| windows[0].toggleFullScreen(None)));
                    wait_until_within(FULL_SCREEN_TRY, in_full_screen)
                });
                println!("{}", if entered { "full-screen" } else { "not full screen" });
                let _ = std::io::stdout().flush();
            });
        }
        WINDOWS.with_borrow_mut(|windows| windows.push(window));
        app.run();
        ExitCode::SUCCESS
    }

    pub fn main() -> ExitCode {
        if let Ok(kind) = std::env::var(COVER) {
            return cover(&kind);
        }
        if !std::env::args().any(|arg| arg == "--ignored") {
            println!("stacking_macos: brings windows of another application to the front; run it with --ignored (CI's macos legs)");
            return ExitCode::SUCCESS;
        }
        let mtm = MainThreadMarker::new().expect("the main thread");
        let app = NSApplication::sharedApplication(mtm);
        // A regular application, as Voltip is unless it starts in the menu bar; it is never the
        // active one here, as Voltip is not while the user dictates into another application.
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        std::thread::spawn(|| std::process::exit(if checks() { 0 } else { 1 }));
        app.run();
        ExitCode::FAILURE
    }
}

fn main() -> std::process::ExitCode {
    #[cfg(target_os = "macos")]
    {
        mac::main()
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::process::ExitCode::SUCCESS
    }
}
