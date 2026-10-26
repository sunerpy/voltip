//! Ogg pages (RFC 3533) of one Opus stream (RFC 7845), written and read back for the audio
//! cache's long recordings (docs/dictation.md §26.2). Every page carries a CRC-32 of its bytes, so
//! a damaged chunk is found when it is read, not when its audio comes out wrong.
//!
//! The writer is the layout `voltip-asr`'s upload codec writes (`ogg_opus.rs`); the reader takes
//! exactly that layout back: one logical stream, every packet whole on its page.

/// What a decoder drops at the start, in 48 kHz samples: the encoder's lookahead, 2.5 ms of CELT
/// overlap and 4 ms of delay compensation (libopus `OPUS_GET_LOOKAHEAD`).
pub(crate) const PRE_SKIP: u16 = 312;
/// Granule positions count 48 kHz samples, whatever the input rate (RFC 7845 §4).
pub(crate) const GRANULE_RATE: u32 = 48_000;
/// The stream's serial number; a file carries one stream, so any number does.
const SERIAL: u32 = 0x566f_6c74;
/// The vendor string in `OpusTags`.
const VENDOR: &[u8] = b"voltip";
/// Page header flags.
pub(crate) const FIRST_PAGE: u8 = 0x02;
pub(crate) const LAST_PAGE: u8 = 0x04;
/// The fixed part of a page header, before its lacing table.
const HEADER_BYTES: usize = 27;

/// Pages of one logical stream, each packet whole on its page.
#[derive(Default)]
pub(crate) struct OggWriter {
    pub(crate) bytes: Vec<u8>,
    sequence: u32,
}

impl OggWriter {
    /// One page with `packets`, ending at `granule`, with the header-type `flags`. A page holds at
    /// most 255 lacing values: the caller keeps its packets within that.
    pub(crate) fn page(&mut self, packets: &[Vec<u8>], granule: u64, flags: u8) {
        let start = self.bytes.len();
        self.bytes.extend_from_slice(b"OggS");
        self.bytes.extend_from_slice(&[0, flags]);
        self.bytes.extend_from_slice(&granule.to_le_bytes());
        self.bytes.extend_from_slice(&SERIAL.to_le_bytes());
        self.bytes.extend_from_slice(&self.sequence.to_le_bytes());
        let crc_at = self.bytes.len();
        self.bytes.extend_from_slice(&[0; 4]);
        let lacing: Vec<u8> = packets.iter().flat_map(|p| std::iter::repeat_n(255, p.len() / 255).chain(std::iter::once((p.len() % 255) as u8))).collect();
        debug_assert!(lacing.len() <= 255, "a page holds 255 lacing values");
        self.bytes.push(lacing.len() as u8);
        self.bytes.extend_from_slice(&lacing);
        for packet in packets {
            self.bytes.extend_from_slice(packet);
        }
        let crc = crc32(&self.bytes[start..]);
        self.bytes[crc_at..crc_at + 4].copy_from_slice(&crc.to_le_bytes());
        self.sequence += 1;
    }
}

/// RFC 7845 §5.1: version 1, one channel, the pre-skip, the input rate, no gain, mapping family 0.
pub(crate) fn opus_head(rate: u32) -> Vec<u8> {
    let mut head = Vec::with_capacity(19);
    head.extend_from_slice(b"OpusHead");
    head.extend_from_slice(&[1, 1]);
    head.extend_from_slice(&PRE_SKIP.to_le_bytes());
    head.extend_from_slice(&rate.to_le_bytes());
    head.extend_from_slice(&0i16.to_le_bytes());
    head.push(0);
    head
}

/// RFC 7845 §5.2: the vendor string and no comments.
pub(crate) fn opus_tags() -> Vec<u8> {
    let mut tags = Vec::with_capacity(16 + VENDOR.len());
    tags.extend_from_slice(b"OpusTags");
    tags.extend_from_slice(&(VENDOR.len() as u32).to_le_bytes());
    tags.extend_from_slice(VENDOR);
    tags.extend_from_slice(&0u32.to_le_bytes());
    tags
}

/// One page read back.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Page {
    pub(crate) flags: u8,
    pub(crate) granule: u64,
    pub(crate) packets: Vec<Vec<u8>>,
}

/// Why bytes are not the pages this module writes.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum OggError {
    /// A page does not start where the previous one ended, or its header is cut short.
    Truncated,
    /// A page's CRC-32 does not match its bytes.
    Checksum,
    /// A page continues a packet from the one before, which this layout never does.
    Continued,
}

impl std::fmt::Display for OggError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Truncated => "an Ogg page is cut short",
            Self::Checksum => "an Ogg page's checksum does not match",
            Self::Continued => "an Ogg page continues a packet",
        })
    }
}

/// Every page of `bytes`, in order, each checked against its CRC-32.
pub(crate) fn read_pages(bytes: &[u8]) -> Result<Vec<Page>, OggError> {
    let mut pages = Vec::new();
    let mut pos = 0;
    while pos < bytes.len() {
        let header = bytes.get(pos..pos + HEADER_BYTES).ok_or(OggError::Truncated)?;
        if &header[0..4] != b"OggS" || header[4] != 0 {
            return Err(OggError::Truncated);
        }
        let flags = header[5];
        if flags & 0x01 != 0 {
            return Err(OggError::Continued);
        }
        let granule = u64::from_le_bytes(header[6..14].try_into().map_err(|_| OggError::Truncated)?);
        let stored = u32::from_le_bytes(header[22..26].try_into().map_err(|_| OggError::Truncated)?);
        let segments = usize::from(header[26]);
        let lacing = bytes.get(pos + HEADER_BYTES..pos + HEADER_BYTES + segments).ok_or(OggError::Truncated)?;
        let body_len: usize = lacing.iter().map(|&l| usize::from(l)).sum();
        let end = pos + HEADER_BYTES + segments + body_len;
        let page = bytes.get(pos..end).ok_or(OggError::Truncated)?;
        let mut zeroed = page.to_vec();
        zeroed[22..26].fill(0);
        if crc32(&zeroed) != stored {
            return Err(OggError::Checksum);
        }
        let mut packets = Vec::new();
        let mut packet = Vec::new();
        let mut at = pos + HEADER_BYTES + segments;
        for &len in lacing {
            packet.extend_from_slice(&bytes[at..at + usize::from(len)]);
            at += usize::from(len);
            if len < 255 {
                packets.push(std::mem::take(&mut packet));
            }
        }
        if !packet.is_empty() {
            // A packet that runs on into the next page: not this layout.
            return Err(OggError::Continued);
        }
        pages.push(Page { flags, granule, packets });
        pos = end;
    }
    Ok(pages)
}

/// Ogg's CRC-32: polynomial 0x04c11db7, not reflected, starting at 0, no final XOR.
pub(crate) fn crc32(data: &[u8]) -> u32 {
    data.iter().fold(0, |crc, &byte| (crc << 8) ^ CRC_TABLE[usize::from((crc >> 24) as u8 ^ byte)])
}

static CRC_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut r = (i as u32) << 24;
        let mut bit = 0;
        while bit < 8 {
            r = if r & 0x8000_0000 != 0 { (r << 1) ^ 0x04c1_1db7 } else { r << 1 };
            bit += 1;
        }
        table[i] = r;
        i += 1;
    }
    table
};

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<u8> {
        let mut ogg = OggWriter::default();
        ogg.page(&[opus_head(16_000)], 0, FIRST_PAGE);
        ogg.page(&[opus_tags()], 0, 0);
        ogg.page(&[vec![1, 2, 3], vec![9; 300], Vec::new()], 960, 0);
        ogg.page(&[vec![7; 60]], 1920, LAST_PAGE);
        ogg.bytes
    }

    #[test]
    fn pages_read_back_as_written() {
        let pages = read_pages(&sample()).unwrap();
        assert_eq!(pages.len(), 4);
        assert_eq!(pages[0], Page { flags: FIRST_PAGE, granule: 0, packets: vec![opus_head(16_000)] });
        assert_eq!(&pages[1].packets[0][..8], b"OpusTags");
        assert_eq!(pages[2].packets, vec![vec![1, 2, 3], vec![9; 300], Vec::new()], "a 300-byte packet spans two lacing values");
        assert_eq!((pages[3].flags, pages[3].granule), (LAST_PAGE, 1920));
        assert!(read_pages(&[]).unwrap().is_empty());
    }

    #[test]
    fn a_damaged_or_cut_page_is_refused() {
        let good = sample();
        let mut flipped = good.clone();
        let at = flipped.len() - 10;
        flipped[at] ^= 0x40;
        assert_eq!(read_pages(&flipped), Err(OggError::Checksum));
        assert_eq!(read_pages(&good[..good.len() - 3]), Err(OggError::Truncated));
        assert_eq!(read_pages(&good[..20]), Err(OggError::Truncated));
        let mut garbage = good;
        garbage[0] = b'X';
        assert_eq!(read_pages(&garbage), Err(OggError::Truncated));
        assert_eq!(OggError::Checksum.to_string(), "an Ogg page's checksum does not match");
    }

    /// The CRC is Ogg's, so other tools read these files: the check value of the polynomial over
    /// "123456789" is 0x89a1897f.
    #[test]
    fn the_checksum_is_oggs() {
        assert_eq!(crc32(b"123456789"), 0x89a1_897f);
    }
}
