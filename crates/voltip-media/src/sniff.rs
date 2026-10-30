//! What the decoder cannot ask Symphonia: the container's name, from the file's first bytes, and
//! the name of an audio codec it has no decoder for, which its readers drop or leave unnamed.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

/// The container `head` (a file's first bytes) starts as: `wav`, `aiff`, `flac`, `ogg`, `mp4`,
/// `mov`, `mkv`, `webm`, `mp3`, `adts` (a bare AAC stream), `caf`; `None` when it is none of them.
pub fn container(head: &[u8]) -> Option<&'static str> {
    let at = |offset: usize, magic: &[u8]| head.get(offset..offset + magic.len()) == Some(magic);
    if at(0, b"RIFF") && at(8, b"WAVE") {
        return Some("wav");
    }
    if at(0, b"FORM") && (at(8, b"AIFF") || at(8, b"AIFC")) {
        return Some("aiff");
    }
    if at(0, b"fLaC") {
        return Some("flac");
    }
    if at(0, b"OggS") {
        return Some("ogg");
    }
    if at(0, b"caff") {
        return Some("caf");
    }
    if at(4, b"ftyp") {
        return Some(if at(8, b"qt  ") { "mov" } else { "mp4" });
    }
    // QuickTime files from before `ftyp`: a `moov`, `mdat`, `wide` or `free` box first.
    if [b"moov", b"mdat", b"wide", b"free"].iter().any(|b| at(4, &b[..])) {
        return Some("mov");
    }
    if at(0, &[0x1A, 0x45, 0xDF, 0xA3]) {
        // The EBML header names its document type within its first bytes.
        return Some(if contains(&head[..head.len().min(64)], b"webm") { "webm" } else { "mkv" });
    }
    if at(0, b"ID3") || mpeg_audio_frame(head) {
        return Some("mp3");
    }
    if let [0xFF, b, ..] = head
        && b & 0xF6 == 0xF0
    {
        return Some("adts");
    }
    None
}

/// An MPEG audio frame header at the start: the 11-bit sync, a defined version and layer.
fn mpeg_audio_frame(head: &[u8]) -> bool {
    match head {
        [a, b, ..] => *a == 0xFF && b & 0xE0 == 0xE0 && b & 0x18 != 0x08 && b & 0x06 != 0,
        _ => false,
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// How far into each end of the file [`unsupported_codec`] looks: a Matroska file names its
/// tracks' codecs near its start, an MP4 file in its `moov` box at either end.
const SCAN: u64 = 4 * 1024 * 1024;

/// Audio codecs the import has no decoder for, by the marks their containers write: Matroska's
/// codec ids and MP4's sample-entry types (docs/dictation.md §31). Longer marks first: `A_EAC3`
/// before `A_AC3`.
const UNSUPPORTED: [(&[u8], &str); 23] = [
    (b"A_EAC3", "E-AC-3"),
    (b"A_AC3", "AC-3"),
    (b"A_DTS", "DTS"),
    (b"A_TRUEHD", "TrueHD"),
    (b"A_MLP", "MLP"),
    (b"A_TTA1", "TTA"),
    (b"A_WAVPACK4", "WavPack"),
    (b"A_REAL/", "RealAudio"),
    (b"A_MS/ACM", "ACM"),
    (b"A_QUICKTIME", "QuickTime audio"),
    (b"A_ATRAC/", "ATRAC"),
    (b"ec-3", "E-AC-3"),
    (b"ac-3", "AC-3"),
    (b"ac-4", "AC-4"),
    (b"dtsc", "DTS"),
    (b"dtsh", "DTS"),
    (b"dtsl", "DTS"),
    (b"dtse", "DTS"),
    (b"mlpa", "TrueHD"),
    (b"samr", "AMR"),
    (b"sawb", "AMR-WB"),
    (b"mha1", "MPEG-H"),
    // An ALAC track the MP4 reader could not set up (QuickTime's layout of its configuration).
    (b"alac", "ALAC"),
];

/// The name of an audio codec without a decoder that `file` carries, found by its container's
/// mark in the first and last [`SCAN`] bytes; `None` when none is there. Asked only once the
/// readers found no audio they can decode, so a mark in the samples cannot hide a codec that works.
pub fn unsupported_codec(file: &mut File) -> Option<&'static str> {
    let len = file.metadata().ok()?.len();
    let mut found = None;
    let mut look = |from: u64, size: u64| -> Option<()> {
        file.seek(SeekFrom::Start(from)).ok()?;
        let mut buf = Vec::new();
        file.by_ref().take(size).read_to_end(&mut buf).ok()?;
        found = found.or_else(|| UNSUPPORTED.iter().find(|(mark, _)| contains(&buf, mark)).map(|(_, name)| *name));
        Some(())
    };
    look(0, SCAN.min(len))?;
    if len > SCAN {
        let from = len.saturating_sub(SCAN).max(SCAN);
        look(from, len - from)?;
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn containers_are_named_by_their_first_bytes() {
        let mut wav = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        assert_eq!(container(&wav), Some("wav"));
        wav[8] = b'X';
        assert_eq!(container(&wav), None);
        assert_eq!(container(b"FORM\0\0\0\0AIFF"), Some("aiff"));
        assert_eq!(container(b"fLaC\0\0\0\x22"), Some("flac"));
        assert_eq!(container(b"OggS\0\x02"), Some("ogg"));
        assert_eq!(container(b"caff\0\x01"), Some("caf"));
        assert_eq!(container(b"\0\0\0\x20ftypM4A \0\0\0\0"), Some("mp4"));
        assert_eq!(container(b"\0\0\0\x14ftypqt  \0\0\0\0"), Some("mov"));
        assert_eq!(container(b"\0\0\0\x08wide\0\0\0\0mdat"), Some("mov"));
        assert_eq!(container(b"\x1a\x45\xdf\xa3\x9f\x42\x86\x81\x01\x42\x82\x84webm"), Some("webm"));
        assert_eq!(container(b"\x1a\x45\xdf\xa3\xa3\x42\x86\x81\x01\x42\x82\x88matroska"), Some("mkv"));
        assert_eq!(container(b"ID3\x04\0"), Some("mp3"));
        assert_eq!(container(&[0xFF, 0xFB, 0x90, 0x64]), Some("mp3"), "MPEG-1 layer III");
        assert_eq!(container(&[0xFF, 0xE8, 0x90]), None, "a reserved version");
        assert_eq!(container(&[0xFF, 0xF1, 0x50, 0x80]), Some("adts"), "MPEG-4 AAC, no CRC");
        assert_eq!(container(b"plain text"), None);
        assert_eq!(container(b""), None);
    }

    #[test]
    fn an_unsupported_codec_is_named_from_either_end_of_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.mkv");
        std::fs::write(&path, [b"\x1a\x45\xdf\xa3 ... A_EAC3 ...".as_slice(), &[0u8; 64]].concat()).unwrap();
        assert_eq!(unsupported_codec(&mut File::open(&path).unwrap()), Some("E-AC-3"), "not taken for A_AC3");
        // An MP4 whose `moov` (with its `ac-3` entry) is past the first part that is scanned.
        let mut tail = vec![0u8; (SCAN + 1024) as usize];
        tail.extend_from_slice(b"stsd\0\0\0\0\0\0\0\x01\0\0\0\x2bac-3");
        std::fs::write(&path, &tail).unwrap();
        assert_eq!(unsupported_codec(&mut File::open(&path).unwrap()), Some("AC-3"));
        std::fs::write(&path, b"nothing to name").unwrap();
        assert_eq!(unsupported_codec(&mut File::open(&path).unwrap()), None);
    }
}
