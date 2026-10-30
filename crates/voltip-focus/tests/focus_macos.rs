//! The insert check's Accessibility reader against real macOS controls (docs/dictation.md §28): a
//! paste into the test's own window, whose text view can be selected but not edited, leaves it
//! `not_written`; a paste into a TextEdit document is `written`. AppKit wants the main thread, so
//! this test has its own `main` (`harness = false`): the main thread runs the application, the
//! checks run on another thread. It pastes the way Voltip does (the pasteboard and Cmd+V) into
//! windows it brings to the front, and needs the Accessibility permission for the test binary, so
//! it runs only when asked: CI's Apple silicon leg (`macos`) grants the permission and runs
//! `cargo test -p voltip-focus --test focus_macos -- --ignored`. Elsewhere, and without
//! `--ignored`, it does nothing.

#![allow(unsafe_code, clippy::unwrap_used, clippy::expect_used)]

#[cfg(target_os = "macos")]
mod mac {
    use std::path::PathBuf;
    use std::process::{Command, ExitCode};
    use std::time::{Duration, Instant};

    use objc2::rc::Retained;
    use objc2::{MainThreadMarker, MainThreadOnly as _};
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSResponder, NSTextView, NSView, NSWindow, NSWindowStyleMask, NSWorkspace,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
    use voltip_focus::{Expected, Reader, Settled, Watch, macos as ax};
    use voltip_inject::{Injector as _, PasteOptions};
    use voltip_platform::HostOs;
    use voltip_platform::insert_check::rules::InsertCheck;

    /// What is pasted: plain ASCII, so any control shows it as sent.
    const TYPED: &str = "voltip check";
    /// How long a paste is watched: well over `CONFIRM_WITHIN`, for a slow runner.
    const WATCH: Duration = Duration::from_secs(3);
    /// How long an application may take to come up or to the front.
    const WAIT: Duration = Duration::from_secs(30);
    /// The test's own window, as the probe would name it (any id outside the table of skipped
    /// applications).
    const OWN: &str = "focus_macos";

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> u8;
    }

    /// The frontmost application: its process id and bundle identifier.
    fn frontmost() -> Option<(i32, String)> {
        let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
        Some((app.processIdentifier(), app.bundleIdentifier().map(|id| id.to_string()).unwrap_or_default()))
    }

    /// Wait until `wanted` says the frontmost application is the right one.
    fn wait_front(what: &str, wanted: impl Fn(i32, &str) -> bool) {
        let until = Instant::now() + WAIT;
        loop {
            let front = frontmost();
            if front.as_ref().is_some_and(|(pid, id)| wanted(*pid, id)) {
                return;
            }
            assert!(Instant::now() < until, "{what} did not come to the front (in front: {front:?})");
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Read the focused control until it answers (a window just brought up may not have focused
    /// its text yet).
    fn wait_watch(reader: &Reader, app_id: &str) -> Watch {
        let until = Instant::now() + WAIT;
        loop {
            match reader.watch(app_id, &Expected::of(TYPED)) {
                Ok(watch) => return watch,
                Err(why) => assert!(Instant::now() < until, "the focused control was never read: {why:?}"),
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Paste [`TYPED`] the way Voltip does, into the application in front.
    fn paste() {
        voltip_inject::system_paste_injector(PasteOptions::for_os("macos")).inject(TYPED).expect("the paste went out");
    }

    /// An empty plain-text document for TextEdit, removed (and TextEdit ended) when the check ends.
    struct TextEdit {
        path: PathBuf,
    }

    impl TextEdit {
        fn open() -> Self {
            let path = std::env::temp_dir().join(format!("voltip-focus-check-{}.txt", std::process::id()));
            std::fs::write(&path, b"").expect("the document");
            let status = Command::new("open").arg("-a").arg("TextEdit").arg(&path).status().expect("open");
            assert!(status.success(), "open -a TextEdit: {status}");
            Self { path }
        }
    }

    impl Drop for TextEdit {
        fn drop(&mut self) {
            // Ended, not quit: a quit would ask whether to save the pasted text.
            let _ = Command::new("pkill").args(["-x", "TextEdit"]).status();
            let _ = std::fs::remove_file(&self.path);
        }
    }

    /// The checks, on their own thread while the main thread runs the application.
    fn checks() {
        // SAFETY: a plain query.
        assert!(unsafe { AXIsProcessTrusted() } != 0, "the test binary has no Accessibility permission");
        let reader = Reader::new(HostOs::Macos, ax::open);
        let own = std::process::id() as i32;

        wait_front("the test's window", |pid, _| pid == own);
        let watch = wait_watch(&reader, OWN);
        paste();
        assert_eq!(watch.settle(WATCH), Settled::sure(InsertCheck::NotWritten), "the text view that cannot be edited");
        println!("focus_macos: the text view that cannot be edited: not_written");

        let document = TextEdit::open();
        wait_front("TextEdit", |_, id| id.eq_ignore_ascii_case("com.apple.TextEdit"));
        let watch = wait_watch(&reader, "com.apple.textedit");
        paste();
        assert_eq!(watch.settle(WATCH), Settled::sure(InsertCheck::Written), "TextEdit");
        println!("focus_macos: TextEdit: written");
        drop(document);
    }

    /// The window: a text view showing 前文 that can be selected but not edited, focused.
    fn window(mtm: MainThreadMarker) -> (Retained<NSWindow>, Retained<NSTextView>) {
        let frame = NSRect::new(NSPoint::new(200.0, 200.0), NSSize::new(480.0, 160.0));
        // SAFETY: a new window on the main thread, kept alive by the caller.
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(NSWindow::alloc(mtm), frame, NSWindowStyleMask::Titled, NSBackingStoreType::Buffered, false)
        };
        // SAFETY: the window is owned by the returned `Retained`, not released by AppKit.
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&NSString::from_str("voltip-focus-check"));
        let text = NSTextView::initWithFrame(NSTextView::alloc(mtm), frame);
        text.setString(&NSString::from_str("前文"));
        text.setEditable(false);
        text.setSelectable(true);
        let view: &NSView = &text;
        window.setContentView(Some(view));
        window.makeKeyAndOrderFront(None);
        let responder: &NSResponder = &text;
        window.makeFirstResponder(Some(responder));
        (window, text)
    }

    pub fn main() -> ExitCode {
        if !std::env::args().any(|arg| arg == "--ignored") {
            println!("focus_macos: pastes into windows it brings to the front; run it with --ignored (CI's macos leg)");
            return ExitCode::SUCCESS;
        }
        let mtm = MainThreadMarker::new().expect("the main thread");
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        let (_window, _text) = window(mtm);
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
        std::thread::spawn(|| {
            let code = match std::panic::catch_unwind(checks) {
                Ok(()) => 0,
                Err(_) => 1,
            };
            std::process::exit(code);
        });
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
