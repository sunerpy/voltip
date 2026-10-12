//! A take as Ogg Opus (RFC 7845) for a service that decodes it (docs/dictation.md §3.9): the same
//! speech in about a twelfth of the WAV's bytes. A WAV at 16 kHz is 32 KB a second; between China
//! and the built-in service's host a connection that lost packets early moves some 45 KB/s, so a
//! 36 s take spent 25 s uploading (2026-10-12). As Opus it is about 100 KB.
//!
//! The encoder is the phone take's (`opus-rs`, a pure-Rust port of libopus, docs/dictation.md
//! §20.1), VoIP in 20 ms frames, at 24 kbit/s VBR. Measured 2026-10-12 on seven speech clips
//! through the built-in Qwen3-ASR, three runs each: 0.5 % of the characters differ from the
//! WAV's text, as at 32, 40 or 75 kbit/s; at encoder complexity 5 or 0 it is 1.3 %, so the
//! default (9) stays. Encoding takes about 12 ms a second of audio on one core of the build host.

use opus_rs::{Application, OpusEncoder};

/// Target bitrate: the phone take's.
const BITRATE_BPS: usize = 24_000;
/// The space a 20 ms packet is given: the bitrate's share and the TOC byte. In VBR `opus-rs`
/// (0.1.34) takes the SILK layer's bitrate from the space it is given, not from `bitrate_bps`
/// (`8 * (n_bytes - 1) * rate / frame`), so the space is the budget: in a 1276-byte buffer speech
/// came out at some 75 kbit/s. Within it the encoder still spends less on quiet frames.
const PACKET_BYTES: usize = BITRATE_BPS / 8 / 50 + 1;
/// One lacing value a packet, so a page's lacing table never fills before its packet count.
const _: () = assert!(PACKET_BYTES < 255);
/// The input rates Opus encodes.
const OPUS_RATES: [u32; 5] = [8_000, 12_000, 16_000, 24_000, 48_000];
/// Granule positions count 48 kHz samples, whatever the input rate (RFC 7845 §4).
const GRANULE_RATE: u32 = 48_000;
/// What a decoder drops at the start, in 48 kHz samples: the encoder's lookahead, 2.5 ms of CELT
/// overlap and 4 ms of delay compensation (libopus `OPUS_GET_LOOKAHEAD`).
const PRE_SKIP: u16 = 312;
/// Packets on one Ogg page: a second of 20 ms frames (the last page takes up to two more).
const PACKETS_PER_PAGE: usize = 50;
/// The stream's serial number; a file carries one stream, so any number does.
const SERIAL: u32 = 0x566f_6c74;
/// The vendor string in `OpusTags`.
const VENDOR: &[u8] = b"voltip";

/// Ogg page flags.
const FIRST_PAGE: u8 = 0x02;
const LAST_PAGE: u8 = 0x04;

/// `wav` as an Ogg Opus file; `None` when it is not 16-bit PCM mono at a rate Opus encodes (8, 12,
/// 16, 24 or 48 kHz), holds no samples, or the encoder refuses a frame. The caller then sends the
/// WAV: every take Voltip makes is 16 kHz mono, so in practice that is a file from somewhere else.
pub(crate) fn from_wav(wav: &[u8]) -> Option<Vec<u8>> {
    let (rate, samples) = pcm16_mono(wav)?;
    if samples.is_empty() {
        return None;
    }
    let mut encoder = OpusEncoder::new(i32::try_from(rate).ok()?, 1, Application::Voip).ok()?;
    encoder.bitrate_bps = BITRATE_BPS as i32;
    encoder.use_cbr = false;
    let frame = rate as usize / 50;
    let scale = GRANULE_RATE / rate;
    // A page's granule position counts the samples its packets and the earlier ones decode to,
    // the pre-skip included (RFC 7845 §4).
    let granule = |input_samples: usize| input_samples as u64 * u64::from(scale);
    // The decoder drops the lookahead at the start, so frames of silence after the take carry its
    // last samples out of the encoder; the last page's granule position then marks where the take
    // ends, and the decoder trims the rest.
    let lookahead = usize::from(PRE_SKIP).div_ceil(scale as usize);
    let frames = (samples.len() + lookahead).div_ceil(frame);
    let end = u64::from(PRE_SKIP) + granule(samples.len());

    let mut ogg = OggWriter::default();
    ogg.page(&[opus_head(rate)], 0, FIRST_PAGE);
    ogg.page(&[opus_tags()], 0, 0);
    let mut input = vec![0i16; frame];
    let mut packet = [0u8; PACKET_BYTES];
    let mut page: Vec<Vec<u8>> = Vec::with_capacity(PACKETS_PER_PAGE + 2);
    for index in 0..frames {
        let start = (index * frame).min(samples.len());
        let chunk = &samples[start..samples.len().min(start + frame)];
        input[..chunk.len()].copy_from_slice(chunk);
        input[chunk.len()..].fill(0);
        let len = encoder.encode_i16(&input, frame, &mut packet).ok()?;
        page.push(packet[..len].to_vec());
        let encoded = (index + 1) * frame;
        if index + 1 == frames {
            ogg.page(&page, end, LAST_PAGE);
        } else if page.len() >= PACKETS_PER_PAGE && encoded < samples.len() {
            // A page ends only inside the take: the frame with its last samples and the padding
            // after it go on the last page, the one whose granule position trims them.
            ogg.page(&page, granule(encoded), 0);
            page.clear();
        }
    }
    Some(ogg.bytes)
}

/// The 16-bit mono samples of `wav` and their rate, when Opus takes them as they are: format 1
/// (PCM), or `WAVE_FORMAT_EXTENSIBLE` whose sub-format is PCM.
fn pcm16_mono(wav: &[u8]) -> Option<(u32, Vec<i16>)> {
    if wav.len() < 12 || &wav[0..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return None;
    }
    let mut format = None;
    let mut pos = 12;
    while pos + 8 <= wav.len() {
        let len = u32::from_le_bytes([wav[pos + 4], wav[pos + 5], wav[pos + 6], wav[pos + 7]]) as usize;
        let start = pos + 8;
        let body = &wav[start..wav.len().min(start.saturating_add(len))];
        match &wav[pos..pos + 4] {
            b"fmt " if body.len() >= 16 => {
                let u16_at = |at: usize| u16::from_le_bytes([body[at], body[at + 1]]);
                let pcm = match u16_at(0) {
                    1 => true,
                    0xfffe => body.len() >= 26 && u16_at(24) == 1,
                    _ => false,
                };
                let rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
                format = Some((pcm && u16_at(2) == 1 && u16_at(14) == 16, rate));
            }
            b"data" => {
                let (usable, rate) = format?;
                if !usable || !OPUS_RATES.contains(&rate) {
                    return None;
                }
                return Some((rate, body.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect()));
            }
            _ => {}
        }
        // Chunks are word-aligned.
        pos = start.saturating_add(len).saturating_add(len & 1);
    }
    None
}

/// RFC 7845 §5.1: version 1, one channel, the pre-skip, the input rate, no gain, mapping family 0.
fn opus_head(rate: u32) -> Vec<u8> {
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
fn opus_tags() -> Vec<u8> {
    let mut tags = Vec::with_capacity(16 + VENDOR.len());
    tags.extend_from_slice(b"OpusTags");
    tags.extend_from_slice(&(VENDOR.len() as u32).to_le_bytes());
    tags.extend_from_slice(VENDOR);
    tags.extend_from_slice(&0u32.to_le_bytes());
    tags
}

/// Ogg pages of one logical stream (RFC 3533), each packet whole on its page.
#[derive(Default)]
struct OggWriter {
    bytes: Vec<u8>,
    sequence: u32,
}

impl OggWriter {
    /// One page with `packets`, ending at `granule`, with the header-type `flags`.
    fn page(&mut self, packets: &[Vec<u8>], granule: u64, flags: u8) {
        let start = self.bytes.len();
        self.bytes.extend_from_slice(b"OggS");
        self.bytes.extend_from_slice(&[0, flags]);
        self.bytes.extend_from_slice(&granule.to_le_bytes());
        self.bytes.extend_from_slice(&SERIAL.to_le_bytes());
        self.bytes.extend_from_slice(&self.sequence.to_le_bytes());
        let crc_at = self.bytes.len();
        self.bytes.extend_from_slice(&[0; 4]);
        let lacing: Vec<u8> = packets.iter().flat_map(|p| std::iter::repeat_n(255, p.len() / 255).chain(std::iter::once((p.len() % 255) as u8))).collect();
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

/// Ogg's CRC-32: polynomial 0x04c11db7, not reflected, starting at 0, no final XOR.
fn crc32(data: &[u8]) -> u32 {
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
    use opus_rs::OpusDecoder;

    use super::*;

    /// A WAV as the core writes it (`voltip_core::dictation::wav::encode_pcm16`).
    fn wav(samples: &[i16], rate: u32) -> Vec<u8> {
        let data = (samples.len() * 2) as u32;
        let mut out = Vec::with_capacity(44 + samples.len() * 2);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&[1, 0, 1, 0]);
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 2).to_le_bytes());
        out.extend_from_slice(&[2, 0, 16, 0]);
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data.to_le_bytes());
        for s in samples {
            out.extend_from_slice(&s.to_le_bytes());
        }
        out
    }

    /// `n` samples of speech-like test audio at `rate`: a voice whose pitch glides between 80 and
    /// 200 Hz, in syllables of a quarter second, with a pause every fifth second.
    fn speech(n: usize, rate: u32) -> Vec<i16> {
        let mut phase = 0.0f64;
        (0..n)
            .map(|i| {
                let t = i as f64 / f64::from(rate);
                phase += 2.0 * std::f64::consts::PI * (140.0 + 60.0 * (0.7 * t).sin()) / f64::from(rate);
                let envelope = if (t as u64) % 5 == 4 { 0.0 } else { (std::f64::consts::PI * (t * 4.0).fract()).sin() };
                (8000.0 * envelope * (phase.sin() + 0.5 * (2.0 * phase).sin())) as i16
            })
            .collect()
    }

    /// A page as read back: its header fields and packets.
    #[derive(Debug)]
    struct Page {
        flags: u8,
        granule: u64,
        serial: u32,
        sequence: u32,
        packets: Vec<Vec<u8>>,
    }

    /// Every page of `ogg`, checking the capture pattern, the version and each CRC.
    fn pages(ogg: &[u8]) -> Vec<Page> {
        let mut pages = Vec::new();
        let mut pos = 0;
        while pos < ogg.len() {
            assert_eq!(&ogg[pos..pos + 4], b"OggS", "capture pattern at {pos}");
            assert_eq!(ogg[pos + 4], 0, "stream structure version");
            let n = usize::from(ogg[pos + 26]);
            let lacing = &ogg[pos + 27..pos + 27 + n];
            let body_len: usize = lacing.iter().map(|&l| usize::from(l)).sum();
            let end = pos + 27 + n + body_len;
            let mut zeroed = ogg[pos..end].to_vec();
            zeroed[22..26].fill(0);
            assert_eq!(crc32(&zeroed), u32::from_le_bytes(ogg[pos + 22..pos + 26].try_into().unwrap()), "CRC of the page at {pos}");
            let mut packets = Vec::new();
            let mut packet = Vec::new();
            let mut at = pos + 27 + n;
            for &l in lacing {
                packet.extend_from_slice(&ogg[at..at + usize::from(l)]);
                at += usize::from(l);
                if l < 255 {
                    packets.push(std::mem::take(&mut packet));
                }
            }
            assert!(packet.is_empty(), "every packet ends on its page");
            pages.push(Page {
                flags: ogg[pos + 5],
                granule: u64::from_le_bytes(ogg[pos + 6..pos + 14].try_into().unwrap()),
                serial: u32::from_le_bytes(ogg[pos + 14..pos + 18].try_into().unwrap()),
                sequence: u32::from_le_bytes(ogg[pos + 18..pos + 22].try_into().unwrap()),
                packets,
            });
            pos = end;
        }
        pages
    }

    /// RFC 7845's layout: `OpusHead` alone on the first page, `OpusTags` alone on the second, then
    /// the audio; one stream, pages numbered in order, the last one flagged, granule positions
    /// rising to the take's end. Every page's CRC checks out.
    #[test]
    fn the_file_is_an_ogg_opus_stream_as_rfc_7845_lays_it_out() {
        let samples = speech(53_920, 16_000);
        let ogg = from_wav(&wav(&samples, 16_000)).unwrap();
        let pages = pages(&ogg);
        assert!(pages.len() >= 6, "{} pages", pages.len());
        assert!(pages.iter().all(|p| p.serial == SERIAL));
        assert!(pages.iter().enumerate().all(|(i, p)| p.sequence as usize == i), "sequence numbers");
        assert_eq!((pages[0].flags, pages[0].granule, pages[0].packets.len()), (FIRST_PAGE, 0, 1));
        let head = &pages[0].packets[0];
        assert_eq!(&head[..8], b"OpusHead");
        assert_eq!((head[8], head[9]), (1, 1), "version 1, one channel");
        assert_eq!(u16::from_le_bytes([head[10], head[11]]), PRE_SKIP);
        assert_eq!(u32::from_le_bytes(head[12..16].try_into().unwrap()), 16_000, "the input rate");
        assert_eq!((&head[16..18], head[18], head.len()), (&[0u8, 0][..], 0, 19), "no gain, mapping family 0");
        assert_eq!((pages[1].flags, pages[1].granule, pages[1].packets.len()), (0, 0, 1));
        let tags = &pages[1].packets[0];
        assert_eq!(&tags[..8], b"OpusTags");
        assert_eq!(&tags[12..12 + VENDOR.len()], VENDOR);
        let audio = &pages[2..];
        assert!(audio[..audio.len() - 1].iter().all(|p| p.flags == 0), "only the last page is flagged");
        assert_eq!(audio.last().unwrap().flags, LAST_PAGE);
        assert!(audio.windows(2).all(|w| w[0].granule < w[1].granule), "granule positions rise");
        assert!(audio.iter().all(|p| p.packets.len() <= PACKETS_PER_PAGE));
        assert_eq!(audio.last().unwrap().granule, u64::from(PRE_SKIP) + samples.len() as u64 * 3, "the end is the take's end");
        // 20 ms a packet, with the lookahead carried out by one more frame of silence.
        let packets: usize = audio.iter().map(|p| p.packets.len()).sum();
        assert_eq!(packets, (samples.len() + 104).div_ceil(320));
    }

    /// Pages end only inside the take, so granule positions rise to the last page's, which is the
    /// take's end; the frame with the take's last samples and the padding after it share the last
    /// page. Lengths around a frame (320 samples) and a page (50 frames).
    #[test]
    fn granule_positions_rise_to_the_end_at_every_length_around_a_page() {
        for len in [1, 319, 320, 321, 15_895, 15_896, 15_990, 16_000, 16_001, 31_950, 32_000] {
            let ogg = from_wav(&wav(&speech(len, 16_000), 16_000)).unwrap();
            let pages = pages(&ogg);
            let audio = &pages[2..];
            let granules: Vec<u64> = audio.iter().map(|p| p.granule).collect();
            assert!(granules.windows(2).all(|w| w[0] < w[1]), "{len}: {granules:?}");
            let end = u64::from(PRE_SKIP) + len as u64 * 3;
            assert_eq!(*granules.last().unwrap(), end, "{len}");
            let mut packets = 0;
            for page in &audio[..audio.len() - 1] {
                packets += page.packets.len();
                assert_eq!(page.granule, packets as u64 * 960, "{len}: an inner page counts its whole frames");
                assert!(page.granule < len as u64 * 3, "{len}: an inner page ends inside the take");
            }
            let total: usize = audio.iter().map(|p| p.packets.len()).sum();
            assert_eq!(total, (len + 104).div_ceil(320), "{len}");
            assert!(total as u64 * 960 >= end, "{len}: the packets decode past the end");
        }
    }

    /// Regression (2026-10-12): `opus-rs` takes the bitrate from the space it is given, and with a
    /// whole 1276-byte buffer a minute of speech came out at some 75 kbit/s, a third of the WAV.
    /// Within the packet budget it stays at 24 kbit/s, under a tenth of the WAV, and decodes back
    /// to the same length and much the same signal once the encoder's lookahead is skipped.
    #[test]
    fn regression_a_take_stays_at_the_budgets_bitrate_and_decodes_back_to_it() {
        let samples = speech(60 * 16_000, 16_000);
        let wav = wav(&samples, 16_000);
        let ogg = from_wav(&wav).unwrap();
        let pages = pages(&ogg);
        let audio_bytes: usize = pages[2..].iter().flat_map(|p| &p.packets).map(Vec::len).sum();
        let kbps = audio_bytes as f64 * 8.0 / 60.0 / 1000.0;
        assert!(kbps <= 24.4, "{kbps:.1} kbit/s");
        assert!(ogg.len() * 10 < wav.len(), "{} bytes of Opus for {} of WAV", ogg.len(), wav.len());
        let mut decoder = OpusDecoder::new(16_000, 1).unwrap();
        let mut frame = vec![0f32; 320];
        let mut decoded = Vec::new();
        for packet in pages[2..].iter().flat_map(|p| &p.packets) {
            let n = decoder.decode(packet, 320, &mut frame).unwrap();
            decoded.extend(frame[..n].iter().map(|s| f64::from(*s) * 32_768.0));
        }
        let skip = usize::from(PRE_SKIP) / 3;
        assert!(decoded.len() >= samples.len() + skip, "{} decoded for {} + {skip}", decoded.len(), samples.len());
        // SILK keeps the sound, not the waveform sample for sample: compare the loudness of every
        // 20 ms, syllables and pauses, and the level of the whole minute.
        let input: Vec<f64> = samples.iter().map(|s| f64::from(*s)).collect();
        let output = &decoded[skip..skip + samples.len()];
        let loudness = |signal: &[f64]| signal.chunks(320).map(|c| (c.iter().map(|s| s * s).sum::<f64>() / c.len() as f64).sqrt()).collect::<Vec<f64>>();
        let (a, b) = (loudness(&input), loudness(output));
        let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
        let (ma, mb) = (mean(&a), mean(&b));
        let covariance: f64 = a.iter().zip(&b).map(|(x, y)| (x - ma) * (y - mb)).sum();
        let spread = |v: &[f64], m: f64| v.iter().map(|x| (x - m).powi(2)).sum::<f64>().sqrt();
        let contour = covariance / (spread(&a, ma) * spread(&b, mb));
        let energy = |v: &[f64]| v.iter().map(|s| s * s).sum::<f64>();
        let level_db = 10.0 * (energy(output) / energy(&input)).log10();
        assert!(contour > 0.95, "loudness contour correlation {contour:.3}");
        assert!(level_db.abs() < 1.0, "level {level_db:.2} dB");
    }

    /// Every rate Opus encodes at; anything Opus cannot take as it is stays a WAV (`None`).
    #[test]
    fn only_a_pcm16_mono_wav_at_an_opus_rate_is_encoded() {
        for rate in OPUS_RATES {
            let ogg = from_wav(&wav(&speech(rate as usize / 2, rate), rate)).unwrap_or_else(|| panic!("{rate} Hz"));
            let head = &pages(&ogg)[0].packets[0];
            assert_eq!(u32::from_le_bytes(head[12..16].try_into().unwrap()), rate);
        }
        assert!(from_wav(&wav(&speech(8_000, 16_000), 44_100)).is_none(), "44.1 kHz");
        assert!(from_wav(&wav(&[], 16_000)).is_none(), "no samples");
        let mut stereo = wav(&speech(8_000, 16_000), 16_000);
        stereo[22] = 2;
        assert!(from_wav(&stereo).is_none(), "two channels");
        let mut float = wav(&speech(8_000, 16_000), 16_000);
        float[20] = 3;
        assert!(from_wav(&float).is_none(), "IEEE float");
        let mut eight_bit = wav(&speech(8_000, 16_000), 16_000);
        eight_bit[34] = 8;
        assert!(from_wav(&eight_bit).is_none(), "8-bit");
        for garbage in [&b""[..], b"RIFF", b"RIFF\0\0\0\0WAVE", b"not a wav at all", &[0u8; 64]] {
            assert!(from_wav(garbage).is_none(), "{garbage:?}");
        }
        // A data chunk before any format chunk says nothing about its samples.
        let mut no_format = b"RIFF\x0c\0\0\0WAVEdata\x04\0\0\0".to_vec();
        no_format.extend_from_slice(&[1, 0, 2, 0]);
        assert!(from_wav(&no_format).is_none());
    }

    /// `WAVE_FORMAT_EXTENSIBLE` with a PCM sub-format is PCM; another chunk before `data` is skipped
    /// with its padding byte, and a `data` length past the end of the file is cut to the file.
    #[test]
    fn extensible_pcm_extra_chunks_and_a_long_data_length_are_read() {
        let samples = speech(6_400, 16_000);
        let mut file = b"RIFF\0\0\0\0WAVEfmt \x28\0\0\0".to_vec();
        file.extend_from_slice(&[0xfe, 0xff, 1, 0]);
        file.extend_from_slice(&16_000u32.to_le_bytes());
        file.extend_from_slice(&32_000u32.to_le_bytes());
        file.extend_from_slice(&[2, 0, 16, 0, 22, 0, 16, 0, 4, 0, 0, 0]);
        file.extend_from_slice(&[1, 0, 0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71]);
        file.extend_from_slice(b"LIST\x03\0\0\0abc\0");
        file.extend_from_slice(b"data\xff\xff\xff\x7f");
        for s in &samples {
            file.extend_from_slice(&s.to_le_bytes());
        }
        let (rate, read) = pcm16_mono(&file).unwrap();
        assert_eq!((rate, read.len()), (16_000, samples.len()));
        assert!(from_wav(&file).is_some());
        let mut float_sub = file.clone();
        float_sub[44] = 3;
        assert!(pcm16_mono(&float_sub).is_none(), "an IEEE float sub-format");
    }

    /// The table is Ogg's CRC: POSIX `cksum`'s CRC-32 without its final XOR, whose check value for
    /// "123456789" is 0x765e7680, so Ogg's is 0x89a1897f.
    #[test]
    fn the_crc_is_oggs() {
        assert_eq!(crc32(b"123456789"), 0x89a1_897f);
        assert_eq!(crc32(b""), 0);
    }
}
