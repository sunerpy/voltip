//! Whole-clipboard round trips on the real system clipboard of Windows and macOS
//! (docs/dictation.md §14.3): what another program copied — formatted text, an image, a file
//! list — is saved before a paste, the dictation's text replaces it marked for clipboard
//! histories, and afterwards everything the program copied is back, byte for byte. They change
//! the clipboard of the desktop session they run in, so they are `#[ignore]`d; CI runs them on
//! its desktops (windows-2025 in the hooks job, both Macs) and `scripts/windows-remote.sh gate`
//! on the Windows build host:
//! `cargo test -p voltip-inject --test real_clipboard -- --ignored --test-threads=1`.
//! Linux's are in `real_display.rs` (they need an X server).

#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(any(windows, target_os = "macos"))]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use voltip_inject::toolchain::{Chord, Delivery};
use voltip_inject::{
    AfterPaste, ClipSnapshot, ClipboardPasteInjector, ClipboardPort, Delivered, DeliveryError, Injection, Injector, KeystrokePort, PasteOptions, PieceOutcome,
    SystemClipboard, Via, paste_chord,
};

/// What the session's clipboard held before the test, put back when it ends (also when it fails):
/// the person whose desktop runs it keeps their clipboard.
struct Kept(Arc<SystemClipboard>, ClipSnapshot);

impl Kept {
    fn now() -> Self {
        let clipboard = Arc::new(SystemClipboard::new());
        let snapshot = clipboard.snapshot().unwrap_or_default();
        Self(clipboard, snapshot)
    }
}

impl Drop for Kept {
    fn drop(&mut self) {
        if let Err(e) = self.0.restore(&self.1) {
            eprintln!("could not put the clipboard back: {e}");
        }
    }
}

/// Answers every paste as delivered and runs `watch` at that moment (the clipboard as the target
/// application reads it).
struct WatchingKeys<F: Fn() + Send + Sync>(F);

impl<F: Fn() + Send + Sync> KeystrokePort for WatchingKeys<F> {
    fn deliver(&self, _: Chord, _: &str) -> Result<Delivered, DeliveryError> {
        (self.0)();
        Ok(Delivered { tool: "watching".into(), delivery: Delivery::Chord })
    }
}

/// One paste session over the real clipboard, `watch` running during the paste; returns once
/// the restore delay has passed.
fn paste_once(clipboard: &Arc<SystemClipboard>, watch: impl Fn() + Send + Sync + 'static) {
    let injector = Arc::new(ClipboardPasteInjector::with_ports(
        clipboard.clone(),
        Arc::new(WatchingKeys(watch)),
        paste_chord(),
        PasteOptions { paste_delay: Duration::from_millis(20), paste_delay_after: Duration::from_millis(50), ..PasteOptions::default() },
    ));
    let mut session = injector.begin_session(AfterPaste::Restore);
    assert!(matches!(session.piece("voltip 口述的文字", &|| true).unwrap(), PieceOutcome::Delivered(Injection { via: Via::Paste, .. })));
    session.finish();
}

/// Wait (bounded) until `done` holds for the clipboard's snapshot.
fn wait_for(clipboard: &SystemClipboard, what: &str, done: impl Fn(&ClipSnapshot) -> bool) -> ClipSnapshot {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let now = clipboard.snapshot().unwrap();
        if done(&now) {
            return now;
        }
        assert!(Instant::now() < deadline, "{what}: {now:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Windows: Unicode text, the HTML Format, a device-independent bitmap and a file list
/// (`CF_HDROP`), written as another program writes them; the dictation carries the clipboard
/// history markers while it is pasted; afterwards every format holds its bytes again and the
/// sequence number moved on.
#[cfg(windows)]
#[test]
#[ignore = "changes the clipboard of this desktop session"]
fn real_windows_formats_another_program_copied_come_back_after_a_paste() {
    use clipboard_win::raw;

    let _kept = Kept::now();
    let utf16 = |s: &str| s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect::<Vec<u8>>();
    let html_format = raw::register_format("HTML Format").unwrap().get();
    let fragment = "<b>加粗</b>的 voltip";
    let html = {
        let head = |start: usize, end: usize| {
            format!("Version:0.9\r\nStartHTML:{start:010}\r\nEndHTML:{end:010}\r\nStartFragment:{start:010}\r\nEndFragment:{end:010}\r\n")
        };
        let start = head(0, 0).len();
        let mut bytes = head(start, start + fragment.len()).into_bytes();
        bytes.extend(fragment.as_bytes());
        bytes
    };
    // BITMAPINFOHEADER of a 2×1 32-bit image, then its two pixels.
    let mut dib = Vec::new();
    for v in [40i32, 2, 1] {
        dib.extend(v.to_le_bytes());
    }
    dib.extend(1u16.to_le_bytes());
    dib.extend(32u16.to_le_bytes());
    for v in [0u32, 8, 0, 0, 0, 0] {
        dib.extend(v.to_le_bytes());
    }
    dib.extend([0, 0, 255, 255, 0, 255, 0, 255]);
    // DROPFILES (pFiles = 20, pt, fNC, fWide = 1), then the paths, double NUL-terminated.
    let mut drop = Vec::new();
    for v in [20u32, 0, 0, 0, 1] {
        drop.extend(v.to_le_bytes());
    }
    drop.extend(utf16(r"C:\Windows\win.ini"));
    drop.extend([0, 0]);
    let written: Vec<(u32, Vec<u8>)> = vec![(13, utf16("原来的文字")), (html_format, html), (8, dib), (15, drop)];
    {
        let _open = clipboard_win::Clipboard::new_attempts(10).unwrap();
        raw::empty().unwrap();
        for (format, bytes) in &written {
            raw::set_without_clear(*format, bytes).unwrap();
        }
    }
    let name = |format: u32| if format == html_format { "HTML Format".to_owned() } else { format!("#{format}") };
    let clipboard = Arc::new(SystemClipboard::new());
    let holds_the_copy = |s: &ClipSnapshot| written.iter().all(|(f, bytes)| s.format(&name(*f)).is_some_and(|b| b.starts_with(bytes)));
    let before = clipboard.snapshot().unwrap();
    assert!(holds_the_copy(&before), "every format was kept: {before:?}");
    let seq = clipboard.change_count().expect("Windows counts clipboard changes");
    let marked = Arc::new(Mutex::new(Vec::new()));
    let seen = marked.clone();
    paste_once(&clipboard, move || {
        let markers = ["CanIncludeInClipboardHistory", "CanUploadToCloudClipboard", "ExcludeClipboardContentFromMonitorProcessing", "Clipboard Viewer Ignore"];
        seen.lock().unwrap().push(markers.iter().all(|m| raw::register_format(m).is_some_and(|f| raw::is_format_avail(f.get()))));
    });
    assert_eq!(*marked.lock().unwrap(), [true], "the dictation carried every history marker while it was pasted");
    let after = wait_for(&clipboard, "the copy is not back", holds_the_copy);
    assert!(after.format("CanIncludeInClipboardHistory").is_some(), "the restore is kept out of the history too");
    assert!(clipboard.change_count().unwrap() > seq, "the clipboard changed and changed back");
}

/// macOS: two pasteboard items as the Finder writes copied files, and an item with HTML, plain
/// text and a PNG; the dictation is marked `org.nspasteboard.TransientType` while it is pasted;
/// afterwards every item and type holds its bytes again.
#[cfg(target_os = "macos")]
#[test]
#[ignore = "changes the pasteboard of this login session"]
fn real_macos_pasteboard_items_another_program_copied_come_back_after_a_paste() {
    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2_app_kit::{NSPasteboard, NSPasteboardItem, NSPasteboardWriting};
    use objc2_foundation::{NSArray, NSData, NSString};

    let _kept = Kept::now();
    let written: Vec<Vec<(&str, Vec<u8>)>> = vec![
        vec![
            ("public.html", "<b>加粗</b>的 voltip".as_bytes().to_vec()),
            ("public.utf8-plain-text", "加粗的 voltip".as_bytes().to_vec()),
            ("public.png", vec![0x89, b'P', b'N', b'G', 1, 2, 3]),
        ],
        vec![("public.file-url", b"file:///tmp/voltip-a.txt".to_vec())],
        vec![("public.file-url", b"file:///tmp/voltip-b.txt".to_vec())],
    ];
    {
        let pasteboard = NSPasteboard::generalPasteboard();
        pasteboard.clearContents();
        let items: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = written
            .iter()
            .map(|formats| {
                let item = NSPasteboardItem::new();
                for (kind, bytes) in formats {
                    assert!(item.setData_forType(&NSData::with_bytes(bytes), &NSString::from_str(kind)));
                }
                ProtocolObject::from_retained(item)
            })
            .collect();
        assert!(pasteboard.writeObjects(&NSArray::from_retained_slice(&items)));
    }
    let clipboard = Arc::new(SystemClipboard::new());
    let holds_the_copy = |s: &ClipSnapshot| {
        s.items.len() == written.len()
            && s.items.iter().zip(&written).all(|(item, formats)| formats.iter().all(|(kind, bytes)| item.iter().any(|f| f.name == *kind && f.bytes == *bytes)))
    };
    let before = clipboard.snapshot().unwrap();
    assert!(holds_the_copy(&before), "every item and type was kept: {before:?}");
    let count = clipboard.change_count().expect("macOS counts pasteboard changes");
    let marked = Arc::new(Mutex::new(Vec::new()));
    let seen = marked.clone();
    paste_once(&clipboard, move || {
        let types = NSPasteboard::generalPasteboard().types().map(|t| t.iter().map(|t| t.to_string()).collect::<Vec<_>>()).unwrap_or_default();
        seen.lock().unwrap().push(types.iter().any(|t| t == "org.nspasteboard.TransientType"));
    });
    assert_eq!(*marked.lock().unwrap(), [true], "the dictation was marked transient while it was pasted");
    let after = wait_for(&clipboard, "the copy is not back", holds_the_copy);
    assert!(after.items[0].iter().any(|f| f.name == "org.nspasteboard.TransientType"), "the restore is kept out of the managers too");
    assert!(clipboard.change_count().unwrap() > count, "the pasteboard changed and changed back");
}
