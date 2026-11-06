//! Whether a Matroska or WebM file was cut off. Its top-level elements (the EBML header, then the
//! segment that holds everything else) say how long they are; Symphonia's reader takes a segment
//! that runs past the end of the file for a missing one, and the file for one of another format.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

/// Top-level elements are read at most this many times: a broken file cannot make the walk run
/// long.
const MAX_ELEMENTS: usize = 64;

/// A variable-length integer at the reader's position (RFC 8794 §4): its length in bytes, and its
/// value without the length marker, `None` for the reserved all-ones value (an unknown size).
/// `max` is the longest it may be (4 bytes for an element ID, 8 for a size).
fn vint(file: &mut File, max: u32) -> Option<(u64, Option<u64>)> {
    let mut first = [0u8; 1];
    file.read_exact(&mut first).ok()?;
    let len = first[0].leading_zeros() + 1;
    if len > max {
        return None;
    }
    let mut value = u64::from(first[0]) & (0xff >> len);
    let mut all_ones = value == 0xff >> len;
    for _ in 1..len {
        file.read_exact(&mut first).ok()?;
        value = value << 8 | u64::from(first[0]);
        all_ones &= first[0] == 0xff;
    }
    Some((u64::from(len), (!all_ones).then_some(value)))
}

/// Whether `file` ends inside one of its top-level elements, as a download or a copy that stopped
/// early leaves it. An element of unknown size runs to the end of the file by definition.
pub fn cut_off(file: &mut File) -> bool {
    let Ok(meta) = file.metadata() else { return false };
    let len = meta.len();
    let mut at = 0u64;
    for _ in 0..MAX_ELEMENTS {
        if at == len {
            return false;
        }
        if file.seek(SeekFrom::Start(at)).is_err() {
            return false;
        }
        // An ID or a size that the end of the file cuts in two: cut off; one no element has (a
        // length marker past its longest): broken, not cut.
        let Some((id_len, _)) = vint(file, 4) else { return file.stream_position().is_ok_and(|p| p >= len) };
        let Some((size_len, size)) = vint(file, 8) else { return file.stream_position().is_ok_and(|p| p >= len) };
        let Some(size) = size else { return false };
        match at.checked_add(id_len + size_len).and_then(|body| body.checked_add(size)) {
            Some(end) if end <= len => at = end,
            _ => return true,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An element with a one-byte size (its content under 127 bytes).
    fn element(id: &[u8], body: &[u8]) -> Vec<u8> {
        [id, &[0x80 | u8::try_from(body.len()).unwrap()], body].concat()
    }

    fn file(bytes: &[u8]) -> File {
        let mut f = tempfile::tempfile().unwrap();
        std::io::Write::write_all(&mut f, bytes).unwrap();
        f
    }

    const EBML: &[u8] = &[0x1a, 0x45, 0xdf, 0xa3];
    const SEGMENT: &[u8] = &[0x18, 0x53, 0x80, 0x67];

    /// Where the file ends says whether it was cut off: inside an element's ID, size or content,
    /// not after its last element or in one of unknown size.
    #[test]
    fn a_file_that_ends_inside_an_element_was_cut_off() {
        let whole = [element(EBML, b"\x42\x82\x88matroska"), element(SEGMENT, &[7u8; 100])].concat();
        assert!(!cut_off(&mut file(&whole)), "the whole file");
        assert!(cut_off(&mut file(&whole[..60])), "in the segment's content");
        assert!(cut_off(&mut file(&whole[..whole.len() - 102])), "in the segment's ID");
        assert!(cut_off(&mut file(&whole[..whole.len() - 101])), "between the segment's ID and its size");
        assert!(!cut_off(&mut file(&whole[..whole.len() - 105])), "after the header: nothing cut");
        // Eight bytes of size, all ones: unknown, as a live recording writes it.
        let unknown = [element(EBML, b"\x42\x82\x84webm"), SEGMENT.to_vec(), vec![0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff], vec![7u8; 40]].concat();
        assert!(!cut_off(&mut file(&unknown)), "a segment of unknown size runs to the end");
        // Two bytes of size (0x40 0x80 = 128) for 100 bytes of content.
        let wide = [element(EBML, b"\x42\x82\x84webm"), SEGMENT.to_vec(), vec![0x40, 0x80], vec![7u8; 100]].concat();
        assert!(cut_off(&mut file(&wide)), "a longer size past the end");
        assert!(!cut_off(&mut file(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x01])), "an ID no element has: broken, not cut");
    }
}
