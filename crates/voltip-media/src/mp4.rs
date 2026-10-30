//! An MP4 or QuickTime track's edit list: which of its decoded samples the file plays. An AAC
//! encoder starts with a frame or two of priming (1024 samples from FFmpeg, 2112 from Apple's) and
//! pads the last frame; the edit list says where the sound starts and how long it lasts, and
//! Symphonia's reader does not apply it. Without it every subtitle of an AAC file would start that
//! late (docs/dictation.md §31).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

/// What a track's edit list keeps, in its media's samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edit {
    /// Samples to drop at the start.
    pub skip: u64,
    /// Samples to keep after them; `None`: to the end.
    pub keep: Option<u64>,
}

/// Boxes are read at most this deep and this many per level: a broken file cannot make the walk
/// run long.
const MAX_BOXES: usize = 4096;

struct Header {
    kind: [u8; 4],
    /// Where the box's content starts and ends.
    body: u64,
    end: u64,
}

fn read_u32(file: &mut File) -> Option<u32> {
    let mut b = [0u8; 4];
    file.read_exact(&mut b).ok()?;
    Some(u32::from_be_bytes(b))
}

fn read_u64(file: &mut File) -> Option<u64> {
    let mut b = [0u8; 8];
    file.read_exact(&mut b).ok()?;
    Some(u64::from_be_bytes(b))
}

/// The box at `at`, within `limit`.
fn header(file: &mut File, at: u64, limit: u64) -> Option<Header> {
    if at.checked_add(8)? > limit {
        return None;
    }
    file.seek(SeekFrom::Start(at)).ok()?;
    let size = read_u32(file)?;
    let mut kind = [0u8; 4];
    file.read_exact(&mut kind).ok()?;
    let (size, body) = match size {
        0 => (limit - at, at + 8),
        1 => (read_u64(file)?, at + 16),
        n => (u64::from(n), at + 8),
    };
    let end = at.checked_add(size)?;
    (end <= limit && end >= body).then_some(Header { kind, body, end })
}

/// The boxes directly inside `from..to`.
fn children(file: &mut File, from: u64, to: u64) -> Vec<Header> {
    let mut out = Vec::new();
    let mut at = from;
    while out.len() < MAX_BOXES {
        let Some(h) = header(file, at, to) else { break };
        at = h.end;
        out.push(h);
    }
    out
}

fn find(boxes: &[Header], kind: &[u8; 4]) -> Option<(u64, u64)> {
    boxes.iter().find(|h| &h.kind == kind).map(|h| (h.body, h.end))
}

/// A full box's version, at the start of its content.
fn version(file: &mut File, body: u64) -> Option<u8> {
    file.seek(SeekFrom::Start(body)).ok()?;
    let mut b = [0u8; 4];
    file.read_exact(&mut b).ok()?;
    Some(b[0])
}

/// The timescale of an `mvhd` or `mdhd` box.
fn timescale(file: &mut File, body: u64) -> Option<u32> {
    let v = version(file, body)?;
    file.seek(SeekFrom::Start(body + if v == 1 { 20 } else { 12 })).ok()?;
    read_u32(file).filter(|&t| t > 0)
}

/// The edit list of the `track`-th `trak` of `file` (the order Symphonia numbers them in); `None`
/// when it has none or the file cannot be walked. An empty edit (a pause before the track starts)
/// is passed over: the import keeps the sound's own timeline.
pub fn edit(file: &mut File, track: usize) -> Option<Edit> {
    let len = file.metadata().ok()?.len();
    let top = children(file, 0, len);
    let (moov, moov_end) = find(&top, b"moov")?;
    let inside = children(file, moov, moov_end);
    let (mvhd, _) = find(&inside, b"mvhd")?;
    let movie_scale = timescale(file, mvhd)?;
    let trak = inside.iter().filter(|h| &h.kind == b"trak").nth(track)?;
    let parts = children(file, trak.body, trak.end);
    let (mdia, mdia_end) = find(&parts, b"mdia")?;
    let (mdhd, _) = find(&children(file, mdia, mdia_end), b"mdhd")?;
    let media_scale = timescale(file, mdhd)?;
    let (edts, edts_end) = find(&parts, b"edts")?;
    let (elst, _) = find(&children(file, edts, edts_end), b"elst")?;
    let v = version(file, elst)?;
    let entries = read_u32(file)?;
    for _ in 0..entries.min(64) {
        let (duration, media_time) =
            if v == 1 { (read_u64(file)?, read_u64(file)? as i64) } else { (u64::from(read_u32(file)?), i64::from(read_u32(file)? as i32)) };
        let _rate = read_u32(file)?;
        if media_time < 0 {
            continue;
        }
        // The duration is in the movie's timescale, the start in the media's.
        let keep = (duration > 0).then(|| (u128::from(duration) * u128::from(media_scale) / u128::from(movie_scale)) as u64);
        return Some(Edit { skip: media_time as u64, keep });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mp4_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = (u32::try_from(body.len() + 8).unwrap()).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }

    fn full(version: u8, rest: &[u8]) -> Vec<u8> {
        [&[version, 0, 0, 0][..], rest].concat()
    }

    /// `mvhd`/`mdhd` (version 0): times, then the timescale, then the duration.
    fn header_with_scale(kind: &[u8; 4], scale: u32) -> Vec<u8> {
        mp4_box(kind, &full(0, &[&[0u8; 8][..], &scale.to_be_bytes(), &[0u8; 4]].concat()))
    }

    fn elst(entries: &[(u32, i32)]) -> Vec<u8> {
        let mut body = full(0, &u32::try_from(entries.len()).unwrap().to_be_bytes());
        for (duration, start) in entries {
            body.extend_from_slice(&duration.to_be_bytes());
            body.extend_from_slice(&start.to_be_bytes());
            body.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        }
        mp4_box(b"elst", &body)
    }

    fn trak(edits: Option<&[(u32, i32)]>, media_scale: u32) -> Vec<u8> {
        let mut body = mp4_box(b"tkhd", &full(0, &[0u8; 80]));
        if let Some(edits) = edits {
            body.extend(mp4_box(b"edts", &elst(edits)));
        }
        body.extend(mp4_box(b"mdia", &header_with_scale(b"mdhd", media_scale)));
        mp4_box(b"trak", &body)
    }

    /// An anonymous file holding `boxes`, deleted when it is closed.
    fn file(boxes: &[Vec<u8>]) -> File {
        let mut f = tempfile::tempfile().unwrap();
        std::io::Write::write_all(&mut f, &boxes.concat()).unwrap();
        f
    }

    #[test]
    fn the_edit_list_of_the_numbered_track_is_read_in_its_media_samples() {
        let moov = mp4_box(b"moov", &[header_with_scale(b"mvhd", 1000), trak(None, 90_000), trak(Some(&[(-1i32 as u32, -1), (2_000, 1024)]), 48_000)].concat());
        let mut f = file(&[mp4_box(b"ftyp", b"M4A \0\0\0\0"), mp4_box(b"mdat", &[0u8; 100]), moov]);
        assert_eq!(edit(&mut f, 1), Some(Edit { skip: 1024, keep: Some(96_000) }), "the empty edit is passed over");
        assert_eq!(edit(&mut f, 0), None, "no edit list");
        assert_eq!(edit(&mut f, 2), None, "no such track");
    }

    #[test]
    fn a_broken_file_has_no_edit_list() {
        assert_eq!(edit(&mut file(&[b"\0\0\0\x08moov".to_vec()]), 0), None, "an empty moov");
        assert_eq!(edit(&mut file(&[b"\xff\xff\xff\xffmoov".to_vec()]), 0), None, "a box past the end");
        assert_eq!(edit(&mut file(&[b"not an mp4 at all".to_vec()]), 0), None);
    }
}
