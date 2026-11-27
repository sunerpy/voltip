//! The insert check's Accessibility reader against real macOS controls (docs/dictation.md §28): a
//! paste into the test's own window, whose text view can be selected but not edited, leaves it
//! `not_written`; a paste into a TextEdit document is `written`. Then the copy that reads the
//! selection (§19, 从选中文字学习 §35): TextEdit's selection comes back with Cmd+C, and the
//! pasteboard holds what it held before. 自动跟踪输入框里的修改 (§35.10): the document's text view,
//! kept from the paste, is read again once its text is deleted, until another document has the
//! focus. AppKit wants the main thread, so
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
    use voltip_focus::{Expected, Follow, Reader, Settled, UnconfirmedWhy, Watch, macos as ax};
    use voltip_inject::{ClipboardPort as _, CopyOptions, Injector as _, Modifier, PasteOptions, SelectionSource as _, SystemClipboard, system_selection};
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

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventCreateKeyboardEvent(source: *const std::ffi::c_void, keycode: u16, down: bool) -> *mut std::ffi::c_void;
        fn CGEventSetFlags(event: *mut std::ffi::c_void, flags: u64);
        fn CGEventPost(tap: u32, event: *mut std::ffi::c_void);
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(cf: *const std::ffi::c_void);
    }

    /// Cmd+A in the application in front (`kVK_ANSI_A` with `kCGEventFlagMaskCommand`, posted at
    /// `kCGHIDEventTap`): TextEdit selects the whole document.
    fn select_all() {
        const KVK_ANSI_A: u16 = 0x00;
        const COMMAND: u64 = 0x0010_0000;
        const HID_EVENT_TAP: u32 = 0;
        for down in [true, false] {
            // SAFETY: a keyboard event from the default source, flagged, posted and released.
            unsafe {
                let event = CGEventCreateKeyboardEvent(std::ptr::null(), KVK_ANSI_A, down);
                assert!(!event.is_null(), "CGEventCreateKeyboardEvent");
                CGEventSetFlags(event, COMMAND);
                CGEventPost(HID_EVENT_TAP, event);
                CFRelease(event);
            }
        }
    }

    /// Shift+Option+Left in the application in front (`kVK_LeftArrow`): TextEdit selects the word
    /// before the caret.
    fn select_last_word() {
        const KVK_LEFT_ARROW: u16 = 0x7B;
        const SHIFT_OPTION: u64 = 0x0002_0000 | 0x0008_0000;
        const HID_EVENT_TAP: u32 = 0;
        for down in [true, false] {
            // SAFETY: a keyboard event from the default source, flagged, posted and released.
            unsafe {
                let event = CGEventCreateKeyboardEvent(std::ptr::null(), KVK_LEFT_ARROW, down);
                assert!(!event.is_null(), "CGEventCreateKeyboardEvent");
                CGEventSetFlags(event, SHIFT_OPTION);
                CGEventPost(HID_EVENT_TAP, event);
                CFRelease(event);
            }
        }
    }

    /// Backspace in the application in front (`kVK_Delete`): TextEdit deletes the selection.
    fn delete_backward() {
        const KVK_DELETE: u16 = 0x33;
        const HID_EVENT_TAP: u32 = 0;
        for down in [true, false] {
            // SAFETY: a keyboard event from the default source, posted and released.
            unsafe {
                let event = CGEventCreateKeyboardEvent(std::ptr::null(), KVK_DELETE, down);
                assert!(!event.is_null(), "CGEventCreateKeyboardEvent");
                CGEventSetFlags(event, 0);
                CGEventPost(HID_EVENT_TAP, event);
                CFRelease(event);
            }
        }
    }

    /// Read the kept text view until `wanted` holds (the keys reach it a little later).
    fn wait_follow(follow: &Follow, what: &str, wanted: impl Fn(&Result<voltip_focus::FocusContext, UnconfirmedWhy>) -> bool) {
        let until = Instant::now() + WAIT;
        loop {
            let read = follow.context(None);
            if wanted(&read) {
                return;
            }
            assert!(Instant::now() < until, "{what}: last read {read:?}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Read TextEdit for 从选中文字学习 until `wanted` holds (the keys reach it a little later).
    fn wait_context(reader: &Reader, what: &str, wanted: impl Fn(&voltip_focus::FocusContext) -> bool) -> voltip_focus::FocusContext {
        let until = Instant::now() + WAIT;
        loop {
            let read = reader.context("com.apple.textedit", None);
            if let Ok(context) = &read
                && wanted(context)
            {
                return context.clone();
            }
            assert!(Instant::now() < until, "{what}: last read {read:?}");
            std::thread::sleep(Duration::from_millis(50));
        }
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
            Self::open_named("")
        }

        /// A document of its own: `name` tells two documents of one check apart.
        fn open_named(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("voltip-focus-check-{}{name}.txt", std::process::id()));
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
        // docs/dictation.md §35.10: the written paste keeps its text view for the follow.
        let (settled, follow) = watch.settle_and_keep(WATCH);
        assert_eq!(settled, Settled::sure(InsertCheck::Written), "TextEdit");
        let follow = follow.expect("a written paste keeps its text view");
        println!("focus_macos: TextEdit: written, its text view kept");

        // docs/dictation.md §35: the learning read gives TextEdit's text around its selection.
        select_last_word();
        let around = wait_context(&reader, "the last word selected", |c| c.selected == "check");
        assert_eq!((around.before.as_str(), around.after.as_str()), ("voltip ", ""), "TextEdit's text around the selection");
        println!("focus_macos: TextEdit's text around the selection: read");
        assert_eq!(reader.context("com.apple.terminal", None).err(), Some(UnconfirmedWhy::Skipped), "a terminal is never read");

        // docs/dictation.md §35 (the copy of §19): 从选中文字学习 reads TextEdit's selection with
        // Cmd+C — the shortcut's modifiers do not mix in, enigo posts its own flags — and puts back
        // what the pasteboard held.
        select_all();
        let pasteboard = SystemClipboard::new();
        pasteboard.write_text("SENTINEL").expect("the pasteboard");
        let copier = system_selection(CopyOptions { timeout: Duration::from_secs(1), ..CopyOptions::default() });
        let copied = copier.copy_selection(&[Modifier::Control, Modifier::Alt]).expect("the copy went out");
        assert_eq!(copied.as_deref(), Some(TYPED), "TextEdit's selection");
        assert_eq!(pasteboard.read_text().expect("the pasteboard").as_deref(), Some("SENTINEL"), "the previous pasteboard is back");
        println!("focus_macos: TextEdit's selection: copied, the pasteboard put back");

        // docs/dictation.md §35.10: the kept text view is read again once its text is deleted, and
        // once another document has the focus the follow says so instead of reading it.
        wait_follow(&follow, "the paste", |r| r.as_ref().is_ok_and(|c| c.selected == TYPED));
        delete_backward();
        wait_follow(&follow, "the text deleted", |r| r.as_ref().is_ok_and(|c| c.before.is_empty() && c.selected.is_empty() && c.after.is_empty()));
        let other = TextEdit::open_named("-other");
        wait_follow(&follow, "the focus moved", |r| *r == Err(UnconfirmedWhy::ControlChanged));
        println!("focus_macos: TextEdit's kept text view: read after the edit, ControlChanged once another document has the focus");
        drop(follow);
        drop(other);
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
