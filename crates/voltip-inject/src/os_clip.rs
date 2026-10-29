//! The system clipboard's whole content (docs/dictation.md §14.3) for [`crate::SystemClipboard`]:
//! Windows through the Win32 clipboard's raw formats, macOS through the pasteboard's items, Linux
//! through arboard's typed reads (text, HTML, PNG images, file lists). A write that is put back
//! later, and every restore, carries the platform's marker for clipboard histories: Windows'
//! clipboard history, cloud clipboard and monitors such as Ditto skip it, so do the macOS managers
//! (`org.nspasteboard.TransientType`) and Klipper (`x-kde-passwordManagerHint`). What a platform
//! cannot keep — Windows handle formats, data an owner does not render, more than
//! [`crate::MAX_SNAPSHOT_BYTES`] — marks the snapshot incomplete and is not put back.

/// Windows: the raw formats, and `GetClipboardSequenceNumber`.
#[cfg(windows)]
pub(crate) mod windows {
    use std::time::Duration;

    use clipboard_win::raw;

    use crate::InjectError;
    use crate::clip::{ClipFormat, ClipSnapshot, MAX_SNAPSHOT_BYTES};

    const CF_BITMAP: u32 = 2;
    const CF_METAFILEPICT: u32 = 3;
    const CF_DIB: u32 = 8;
    const CF_PALETTE: u32 = 9;
    const CF_UNICODETEXT: u32 = 13;
    const CF_ENHMETAFILE: u32 = 14;
    const CF_DIBV5: u32 = 17;
    const CF_OWNERDISPLAY: u32 = 0x80;
    const CF_DSPBITMAP: u32 = 0x82;
    const CF_DSPMETAFILEPICT: u32 = 0x83;
    const CF_DSPENHMETAFILE: u32 = 0x8E;
    /// `CF_PRIVATEFIRST ..= CF_GDIOBJLAST`: handles only their owner understands.
    const HANDLES: std::ops::RangeInclusive<u32> = 0x200..=0x3FF;
    /// The first registered format id.
    const REGISTERED: u32 = 0xC000;

    /// Registered formats that tell clipboard histories, the cloud clipboard and monitors to skip
    /// what is on the clipboard (the Microsoft documentation's DWORD 0; Ditto's own format).
    const MARKERS: [(&str, &[u8]); 4] = [
        ("ExcludeClipboardContentFromMonitorProcessing", &[0, 0, 0, 0]),
        ("CanIncludeInClipboardHistory", &[0, 0, 0, 0]),
        ("CanUploadToCloudClipboard", &[0, 0, 0, 0]),
        ("Clipboard Viewer Ignore", &[0, 0, 0, 0]),
    ];

    /// The clipboard, opened for this thread until the guard drops. Another program may hold it
    /// for a moment: a few tries, as arboard and the browsers do.
    fn open() -> Result<clipboard_win::Clipboard, InjectError> {
        let mut tries = 5;
        loop {
            match clipboard_win::Clipboard::new() {
                Ok(clipboard) => return Ok(clipboard),
                Err(e) if tries == 0 => return Err(InjectError::Clipboard(format!("the clipboard is busy: {e}"))),
                // A fixed short pause between tries: there is no event for "the clipboard is free".
                Err(_) => tries -= 1,
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Whether a snapshot leaves `format` out, and whether that loses anything: handle formats
    /// cannot be copied as bytes. `kept_bitmap`: a device-independent bitmap is kept, from which
    /// the system makes the handle bitmap and its palette again.
    fn left_out(format: u32, kept_bitmap: bool) -> Option<bool> {
        match format {
            CF_BITMAP | CF_PALETTE => Some(!kept_bitmap),
            CF_METAFILEPICT | CF_ENHMETAFILE | CF_DSPBITMAP | CF_DSPMETAFILEPICT | CF_DSPENHMETAFILE => Some(true),
            CF_OWNERDISPLAY => Some(true),
            f if HANDLES.contains(&f) => Some(true),
            _ => None,
        }
    }

    /// How a snapshot names `format`: a registered format by its name (its id is only stable
    /// for the session), a standard one by its number.
    fn name_of(format: u32) -> String {
        if format >= REGISTERED
            && let Some(name) = raw::format_name_big(format)
        {
            return name;
        }
        format!("#{format}")
    }

    fn id_of(name: &str) -> Option<u32> {
        match name.strip_prefix('#') {
            Some(number) => number.parse().ok(),
            None => raw::register_format(name).map(|id| id.get()),
        }
    }

    pub(crate) fn snapshot() -> Result<ClipSnapshot, InjectError> {
        let _open = open()?;
        let listed: Vec<u32> = raw::EnumFormats::new().collect();
        // The two device-independent bitmaps are made from each other: the first one listed is
        // what the owner put there, the other one comes again by itself.
        let first_dib = listed.iter().copied().find(|f| matches!(*f, CF_DIB | CF_DIBV5));
        let mut formats = Vec::new();
        let (mut complete, mut total) = (true, 0usize);
        for format in listed {
            if matches!(format, CF_DIB | CF_DIBV5) && Some(format) != first_dib {
                continue;
            }
            if let Some(loses) = left_out(format, first_dib.is_some()) {
                complete &= !loses;
                continue;
            }
            // Renders a delayed format, which can take its owner a while.
            let Some(size) = raw::size(format) else {
                complete = false;
                continue;
            };
            if total + size.get() > MAX_SNAPSHOT_BYTES {
                complete = false;
                continue;
            }
            let mut bytes = Vec::with_capacity(size.get());
            match raw::get_vec(format, &mut bytes) {
                Ok(_) => {
                    total += bytes.len();
                    formats.push(ClipFormat::new(name_of(format), bytes));
                }
                Err(e) => {
                    tracing::debug!(format, error = %e, "a clipboard format could not be read");
                    complete = false;
                }
            }
        }
        Ok(ClipSnapshot { complete, ..ClipSnapshot::single(formats) })
    }

    /// Empty the clipboard, put `formats` on it, and the history markers it does not carry yet.
    fn replace(formats: &[ClipFormat]) -> Result<(), InjectError> {
        let _open = open()?;
        raw::empty().map_err(|e| InjectError::Clipboard(format!("the clipboard could not be emptied: {e}")))?;
        for format in formats {
            let Some(id) = id_of(&format.name) else {
                tracing::debug!(format = %format.name, "a clipboard format could not be registered again");
                continue;
            };
            if let Err(e) = raw::set_without_clear(id, &format.bytes) {
                tracing::debug!(format = %format.name, error = %e, "a clipboard format could not be put back");
            }
        }
        for (name, data) in MARKERS {
            if formats.iter().any(|f| f.name == name) {
                continue;
            }
            match raw::register_format(name) {
                Some(id) => {
                    if let Err(e) = raw::set_without_clear(id.get(), data) {
                        tracing::debug!(marker = name, error = %e, "a clipboard history marker could not be set");
                    }
                }
                None => tracing::debug!(marker = name, "a clipboard history marker could not be registered"),
            }
        }
        Ok(())
    }

    pub(crate) fn restore(snapshot: &ClipSnapshot) -> Result<(), InjectError> {
        if snapshot.is_empty() {
            let _open = open()?;
            return raw::empty().map_err(|e| InjectError::Clipboard(format!("the clipboard could not be emptied: {e}")));
        }
        let formats: Vec<ClipFormat> = snapshot.items.iter().flatten().cloned().collect();
        replace(&formats)
    }

    pub(crate) fn write_transient(text: &str) -> Result<bool, InjectError> {
        let mut utf16: Vec<u8> = text.encode_utf16().chain(std::iter::once(0)).flat_map(u16::to_le_bytes).collect();
        utf16.shrink_to_fit();
        replace(&[ClipFormat::new(format!("#{CF_UNICODETEXT}"), utf16)]).map(|()| true)
    }

    pub(crate) fn change_count() -> Option<u64> {
        raw::seq_num().map(|n| u64::from(n.get()))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn handle_formats_are_left_out_and_named_formats_keep_their_name() {
            assert_eq!(left_out(CF_BITMAP, true), Some(false), "made again from the kept bitmap");
            assert_eq!(left_out(CF_BITMAP, false), Some(true));
            assert_eq!(left_out(CF_ENHMETAFILE, true), Some(true));
            assert_eq!(left_out(0x2A0, true), Some(true), "private handles");
            assert_eq!(left_out(CF_UNICODETEXT, false), None);
            assert_eq!(left_out(1, false), None, "CF_TEXT");
            assert_eq!(left_out(7, false), None, "CF_OEMTEXT");
            assert_eq!((name_of(CF_UNICODETEXT), id_of("#13")), ("#13".to_owned(), Some(13)));
            let html = raw::register_format("HTML Format").unwrap().get();
            assert_eq!(name_of(html), "HTML Format");
            assert_eq!(id_of("HTML Format"), Some(html));
        }
    }
}

/// macOS: the general pasteboard's items, and its change count. The bindings used are safe; the
/// pasteboard is looked up on every call (its handle is not `Send`), inside an autorelease pool.
#[cfg(target_os = "macos")]
pub(crate) mod macos {
    use objc2::rc::{Retained, autoreleasepool};
    use objc2::runtime::ProtocolObject;
    use objc2_app_kit::{NSPasteboard, NSPasteboardItem, NSPasteboardWriting};
    use objc2_foundation::{NSArray, NSData, NSString};

    use crate::InjectError;
    use crate::clip::{ClipFormat, ClipSnapshot, MAX_SNAPSHOT_BYTES};

    /// <http://nspasteboard.org>: the content is transient, clipboard managers do not record it.
    const TRANSIENT: &str = "org.nspasteboard.TransientType";
    const PLAIN_TEXT: &str = "public.utf8-plain-text";

    pub(crate) fn snapshot() -> ClipSnapshot {
        autoreleasepool(|_| {
            let pasteboard = NSPasteboard::generalPasteboard();
            let Some(items) = pasteboard.pasteboardItems() else { return ClipSnapshot::empty() };
            let (mut kept, mut complete, mut total) = (Vec::new(), true, 0usize);
            for item in items.iter() {
                let mut formats = Vec::new();
                for kind in item.types().iter() {
                    // Asks the owner for promised data, which can take it a while.
                    match item.dataForType(&kind) {
                        Some(data) if total + data.len() <= MAX_SNAPSHOT_BYTES => {
                            total += data.len();
                            formats.push(ClipFormat::new(kind.to_string(), data.to_vec()));
                        }
                        _ => complete = false,
                    }
                }
                if !formats.is_empty() {
                    kept.push(formats);
                }
            }
            ClipSnapshot { items: kept, complete }
        })
    }

    /// Clear the pasteboard and write `items`, the first one marked transient.
    fn write(items: &[Vec<ClipFormat>]) -> Result<(), InjectError> {
        autoreleasepool(|_| {
            let pasteboard = NSPasteboard::generalPasteboard();
            pasteboard.clearContents();
            if items.is_empty() {
                return Ok(());
            }
            let objects: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = items
                .iter()
                .enumerate()
                .map(|(n, formats)| {
                    let item = NSPasteboardItem::new();
                    for format in formats {
                        item.setData_forType(&NSData::with_bytes(&format.bytes), &NSString::from_str(&format.name));
                    }
                    if n == 0 && !formats.iter().any(|f| f.name == TRANSIENT) {
                        item.setData_forType(&NSData::with_bytes(&[]), &NSString::from_str(TRANSIENT));
                    }
                    ProtocolObject::from_retained(item)
                })
                .collect();
            if pasteboard.writeObjects(&NSArray::from_retained_slice(&objects)) {
                Ok(())
            } else {
                Err(InjectError::Clipboard("the pasteboard refused the write".into()))
            }
        })
    }

    pub(crate) fn restore(snapshot: &ClipSnapshot) -> Result<(), InjectError> {
        write(&snapshot.items)
    }

    pub(crate) fn write_transient(text: &str) -> Result<bool, InjectError> {
        write(&[vec![ClipFormat::new(PLAIN_TEXT, text.as_bytes())]]).map(|()| true)
    }

    pub(crate) fn change_count() -> Option<u64> {
        autoreleasepool(|_| u64::try_from(NSPasteboard::generalPasteboard().changeCount()).ok())
    }
}

/// Linux: arboard's typed reads and writes. Text, HTML, PNG images and file lists are what it can
/// read; other formats are not seen. One kind comes back — a file list, else the image, else the
/// HTML with its text, else the text — and a snapshot that held more than that is incomplete.
#[cfg(target_os = "linux")]
pub(crate) mod linux {
    use std::borrow::Cow;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    use std::path::PathBuf;

    use arboard::SetExtLinux;

    use crate::clip::{ClipFormat, ClipSnapshot, MAX_SNAPSHOT_BYTES, TEXT_FORMAT};

    /// The paths of a file list, NUL-separated (a path cannot hold a NUL).
    pub(crate) const FILES: &str = "voltip/file-list";
    /// An image as arboard decodes it: width and height (little-endian `u32`), then RGBA bytes.
    pub(crate) const RGBA: &str = "voltip/image-rgba";
    pub(crate) const HTML: &str = "text/html";

    /// A read that found nothing is no error. An owner that answers every request with its one
    /// format (`xclip -t image/png` does) comes back as an error of an unknown kind: nothing of
    /// that kind is there. Data that is there but could not be read is lost.
    fn optional<T>(read: Result<T, arboard::Error>, complete: &mut bool) -> Option<T> {
        match read {
            Ok(value) => Some(value),
            Err(arboard::Error::ContentNotAvailable) => None,
            Err(e @ (arboard::Error::ConversionFailure | arboard::Error::ClipboardOccupied)) => {
                tracing::debug!(error = %e, "a clipboard format could not be read");
                *complete = false;
                None
            }
            Err(e) => {
                tracing::debug!(error = %e, "a clipboard read found nothing of that kind");
                None
            }
        }
    }

    pub(crate) fn snapshot(clipboard: &mut arboard::Clipboard) -> Result<ClipSnapshot, arboard::Error> {
        let mut complete = true;
        let mut formats = Vec::new();
        if let Some(files) = optional(clipboard.get().file_list(), &mut complete).filter(|f| !f.is_empty()) {
            let bytes = files.iter().map(|p| p.as_os_str().as_bytes()).collect::<Vec<_>>().join(&0u8);
            formats.push(ClipFormat::new(FILES, bytes));
        }
        if let Some(image) = optional(clipboard.get_image(), &mut complete) {
            if image.bytes.len() > MAX_SNAPSHOT_BYTES {
                complete = false;
            } else {
                let mut bytes = Vec::with_capacity(8 + image.bytes.len());
                for side in [image.width, image.height] {
                    bytes.extend(u32::try_from(side).unwrap_or(u32::MAX).to_le_bytes());
                }
                bytes.extend_from_slice(&image.bytes);
                formats.push(ClipFormat::new(RGBA, bytes));
            }
        }
        if let Some(html) = optional(clipboard.get().html(), &mut complete) {
            formats.push(ClipFormat::new(HTML, html.into_bytes()));
        }
        if let Some(text) = optional(clipboard.get_text(), &mut complete) {
            formats.push(ClipFormat::new(TEXT_FORMAT, text.into_bytes()));
        }
        // One kind comes back (`restore`): what else was there is lost.
        let has = |kind: &str| formats.iter().any(|f| f.name == kind);
        let lost = if has(FILES) { has(RGBA) || has(HTML) || has(TEXT_FORMAT) } else { has(RGBA) && (has(HTML) || has(TEXT_FORMAT)) };
        Ok(ClipSnapshot { complete: complete && !lost, ..ClipSnapshot::single(formats) })
    }

    fn decode_image(bytes: &[u8]) -> Option<arboard::ImageData<'_>> {
        let (head, rgba) = bytes.split_at_checked(8)?;
        let side = |at: usize| head.get(at..at + 4).and_then(|b| <[u8; 4]>::try_from(b).ok()).map(u32::from_le_bytes).and_then(|v| usize::try_from(v).ok());
        let (width, height) = (side(0)?, side(4)?);
        (width.checked_mul(height)?.checked_mul(4)? == rgba.len()).then_some(arboard::ImageData { width, height, bytes: Cow::Borrowed(rgba) })
    }

    pub(crate) fn restore(clipboard: &mut arboard::Clipboard, snapshot: &ClipSnapshot) -> Result<(), arboard::Error> {
        if let Some(files) = snapshot.format(FILES) {
            let paths: Vec<PathBuf> = files.split(|b| *b == 0).map(|p| PathBuf::from(std::ffi::OsString::from_vec(p.to_vec()))).collect();
            return clipboard.set().exclude_from_history().file_list(&paths);
        }
        if let Some(image) = snapshot.format(RGBA).and_then(decode_image) {
            return clipboard.set().exclude_from_history().image(image);
        }
        let text = snapshot.text();
        match snapshot.format(HTML).and_then(|h| std::str::from_utf8(h).ok()) {
            Some(html) => clipboard.set().exclude_from_history().html(html, text),
            None => match text {
                Some(text) => clipboard.set().exclude_from_history().text(text),
                None => clipboard.clear(),
            },
        }
    }

    pub(crate) fn write_transient(clipboard: &mut arboard::Clipboard, text: &str) -> Result<(), arboard::Error> {
        clipboard.set().exclude_from_history().text(text)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn images_round_trip_through_their_header() {
            let mut bytes = Vec::new();
            bytes.extend(2u32.to_le_bytes());
            bytes.extend(1u32.to_le_bytes());
            bytes.extend([1, 2, 3, 4, 5, 6, 7, 8]);
            let image = decode_image(&bytes).unwrap();
            assert_eq!((image.width, image.height, image.bytes.as_ref()), (2, 1, &[1, 2, 3, 4, 5, 6, 7, 8][..]));
            assert!(decode_image(&bytes[..12]).is_none(), "short pixel data");
            assert!(decode_image(&[0; 4]).is_none());
        }
    }
}
