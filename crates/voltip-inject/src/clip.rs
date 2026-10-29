//! What the clipboard held before an injection, kept whole (docs/dictation.md §14.3): every format
//! the platform offers, as bytes, so an image, a file list or formatted text the user had copied
//! comes back after the paste — not only its text. The platform ports fill and restore snapshots
//! ([`crate::ClipboardPort::snapshot`] / [`crate::ClipboardPort::restore`]); the default port keeps
//! the text alone.

/// The name the text of a text-only snapshot is kept under (the default
/// [`crate::ClipboardPort::snapshot`]); the platform ports name their formats as the platform does.
pub const TEXT_FORMAT: &str = "text/plain;charset=utf-8";

/// Most bytes one snapshot keeps. Formats past it are left out and the snapshot is marked
/// incomplete: the injection goes ahead, and only what was kept comes back. 64 MiB holds the
/// bitmap of a 4K screenshot (3840 × 2160 × 4 bytes) with room to spare.
pub const MAX_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;

/// One format of a clipboard item: its name as the platform names it, and its bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct ClipFormat {
    /// `text/html`, `public.png`, `#8` (a Windows standard format), `HTML Format`, …
    pub name: String,
    /// The data, exactly as the clipboard handed it over.
    pub bytes: Vec<u8>,
}

impl ClipFormat {
    /// `name` holding `bytes`.
    pub fn new(name: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        Self { name: name.into(), bytes: bytes.into() }
    }
}

/// The name and the size only: a clipboard's content never reaches a log.
impl std::fmt::Debug for ClipFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({} bytes)", self.name, self.bytes.len())
    }
}

/// What a clipboard held: its items — one on Windows and Linux; a macOS pasteboard can hold several
/// (files copied in the Finder) — each a list of formats.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipSnapshot {
    /// The items, in the clipboard's order; an empty list is an empty clipboard.
    pub items: Vec<Vec<ClipFormat>>,
    /// Every format the clipboard offered was kept. `false` when one was left out: a private or
    /// handle-only format, data the owner would not render, or more than [`MAX_SNAPSHOT_BYTES`].
    pub complete: bool,
}

impl Default for ClipSnapshot {
    fn default() -> Self {
        Self::empty()
    }
}

impl ClipSnapshot {
    /// An empty clipboard.
    pub fn empty() -> Self {
        Self { items: Vec::new(), complete: true }
    }

    /// A clipboard holding `text` alone under [`TEXT_FORMAT`]; empty for `None`.
    pub fn of_text(text: Option<&str>) -> Self {
        Self::single(text.map(|t| ClipFormat::new(TEXT_FORMAT, t.as_bytes())).into_iter().collect())
    }

    /// One item of `formats`; empty when there are none.
    pub fn single(formats: Vec<ClipFormat>) -> Self {
        Self { items: if formats.is_empty() { Vec::new() } else { vec![formats] }, complete: true }
    }

    /// A clipboard that could not be read: nothing is known about what it held, so nothing is
    /// put back over it — not even emptiness.
    pub fn unread() -> Self {
        Self { items: Vec::new(), complete: false }
    }

    /// Nothing to put back.
    pub fn is_empty(&self) -> bool {
        self.items.iter().all(Vec::is_empty)
    }

    /// The clipboard was read and held nothing at all: a cancel empties it again.
    pub fn read_empty(&self) -> bool {
        self.is_empty() && self.complete
    }

    /// Bytes kept, over every item and format.
    pub fn bytes(&self) -> usize {
        self.items.iter().flatten().map(|f| f.bytes.len()).sum()
    }

    /// The bytes of the first format named `name`.
    pub fn format(&self, name: &str) -> Option<&[u8]> {
        self.items.iter().flatten().find(|f| f.name == name).map(|f| f.bytes.as_slice())
    }

    /// The text kept under [`TEXT_FORMAT`].
    pub fn text(&self) -> Option<&str> {
        self.format(TEXT_FORMAT).and_then(|b| std::str::from_utf8(b).ok())
    }

    /// The snapshot is `text` and nothing else: putting it back after a paste of `text` would
    /// change nothing.
    pub fn holds_only(&self, text: &str) -> bool {
        matches!(self.items.as_slice(), [item] if matches!(item.as_slice(), [f] if f.name == TEXT_FORMAT && f.bytes == text.as_bytes()))
    }

    /// The format names, item by item, for logs.
    pub fn format_names(&self) -> Vec<&str> {
        self.items.iter().flatten().map(|f| f.name.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_of_text_and_formats() {
        let empty = ClipSnapshot::empty();
        assert!(empty.is_empty() && empty.complete);
        assert_eq!(ClipSnapshot::default(), empty);
        assert_eq!(ClipSnapshot::of_text(None), empty);
        assert_eq!(ClipSnapshot::single(Vec::new()), empty);
        let text = ClipSnapshot::of_text(Some("你好"));
        assert_eq!((text.text(), text.bytes()), (Some("你好"), 6));
        assert!(text.holds_only("你好") && !text.holds_only("你"));
        let image = ClipSnapshot::single(vec![ClipFormat::new("image/png", [0x89, b'P', b'N', b'G']), ClipFormat::new(TEXT_FORMAT, "alt")]);
        assert_eq!(image.format("image/png"), Some(&[0x89, b'P', b'N', b'G'][..]));
        assert_eq!(image.text(), Some("alt"));
        assert!(!image.holds_only("alt"), "an image comes back even when its text equals ours");
        assert_eq!(image.format_names(), ["image/png", TEXT_FORMAT]);
        assert_eq!(format!("{:?}", image.items[0][0]), "image/png (4 bytes)", "the bytes never reach a log");
        let two = ClipSnapshot { items: vec![vec![ClipFormat::new("public.file-url", "a")], vec![ClipFormat::new("public.file-url", "b")]], complete: false };
        assert_eq!((two.bytes(), two.format("public.file-url")), (2, Some(&b"a"[..])));
        assert!(!two.is_empty());
        assert_eq!(MAX_SNAPSHOT_BYTES, 64 << 20);
    }
}
