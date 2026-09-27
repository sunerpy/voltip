//! Just enough WAV to decide whether a recording is worth uploading (and for the fakes to build
//! one): PCM 16-bit little-endian, any channel count.

/// Peak amplitude (linear, `0.0..=1.0`) below which a recording counts as silence (−60 dBFS).
pub const SILENCE_PEAK: f32 = 0.001;

/// Encode mono PCM 16-bit samples as a complete WAV file.
pub fn encode_pcm16(samples: &[i16], sample_rate_hz: u32) -> Vec<u8> {
    let data_len = u32::try_from(samples.len() * 2).unwrap_or(u32::MAX);
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate_hz.to_le_bytes());
    out.extend_from_slice(&(sample_rate_hz * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// The PCM payload of a WAV file (the `data` chunk), or `None` when the file is not RIFF/WAVE or
/// has no `data` chunk.
pub fn pcm_data(wav: &[u8]) -> Option<&[u8]> {
    if wav.len() < 12 || &wav[0..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return None;
    }
    let mut pos = 12;
    while pos + 8 <= wav.len() {
        let id = &wav[pos..pos + 4];
        let len = u32::from_le_bytes([wav[pos + 4], wav[pos + 5], wav[pos + 6], wav[pos + 7]]) as usize;
        let start = pos + 8;
        if id == b"data" {
            return Some(&wav[start..wav.len().min(start + len)]);
        }
        // Chunks are word-aligned.
        pos = start + len + (len & 1);
    }
    None
}

/// Peak amplitude of the PCM 16-bit payload, linear `0.0..=1.0`. Unparseable input is `0.0`.
pub fn peak(wav: &[u8]) -> f32 {
    let Some(data) = pcm_data(wav) else { return 0.0 };
    let max = data.as_chunks::<2>().0.iter().map(|b| i32::from(i16::from_le_bytes(*b)).unsigned_abs()).max().unwrap_or(0);
    max as f32 / 32_768.0
}

/// A recording nobody spoke into: no parseable audio, or a peak below [`SILENCE_PEAK`].
pub fn is_silent(wav: &[u8]) -> bool {
    peak(wav) < SILENCE_PEAK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_then_inspect() {
        let wav = encode_pcm16(&[0, 1000, -2000, 300], 16_000);
        assert_eq!(wav.len(), 44 + 8);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(pcm_data(&wav).unwrap().len(), 8);
        assert!((peak(&wav) - 2000.0 / 32_768.0).abs() < 1e-6);
        assert!(!is_silent(&wav));
        assert!(is_silent(&encode_pcm16(&[0; 1600], 16_000)));
        assert!(is_silent(&encode_pcm16(&[], 16_000)));
        assert!(is_silent(&encode_pcm16(&[20, -20], 16_000)), "below −60 dBFS");
    }

    #[test]
    fn garbage_is_silent_and_odd_chunks_are_skipped() {
        assert!(pcm_data(b"").is_none());
        assert!(pcm_data(b"RIFF\0\0\0\0WAVX").is_none());
        assert!(is_silent(b"not a wav at all"));
        // A LIST chunk of odd length before `data` must be padded to an even boundary.
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF\0\0\0\0WAVE");
        wav.extend_from_slice(b"LIST");
        wav.extend_from_slice(&3u32.to_le_bytes());
        wav.extend_from_slice(&[1, 2, 3, 0]);
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&2u32.to_le_bytes());
        wav.extend_from_slice(&16_000i16.to_le_bytes());
        assert_eq!(pcm_data(&wav).unwrap(), &16_000i16.to_le_bytes());
        assert!(!is_silent(&wav));
        // A truncated data chunk yields what is there.
        let mut wav = encode_pcm16(&[5000; 4], 8_000);
        wav.truncate(46);
        assert_eq!(pcm_data(&wav).unwrap().len(), 2);
        // A file that ends before the data chunk has no data.
        assert!(pcm_data(&encode_pcm16(&[1], 8_000)[..20]).is_none());
    }
}
