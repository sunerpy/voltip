//! What a [`crate::Recorder`] hands back: mono 16-bit PCM plus the arithmetic around it
//! (downmix, resampling, WAV framing, silence detection). Nothing here touches a device.

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};

use crate::AudioError;
use crate::dsp::{self, DBFS_FLOOR, SampleChunk};

/// Recordings quieter than this at their loudest are treated as "nothing was said".
pub const SILENCE_PEAK_DBFS: f32 = -60.0;
/// Recordings shorter than this are treated as an accidental tap on the hotkey.
pub const MIN_SPEECH_MS: u64 = 300;

/// Frames per resampler chunk. 1024 keeps the FFT block a few hundred frames wide, which is
/// what rubato recommends for speech-band quality at low delay.
const RESAMPLE_CHUNK_FRAMES: usize = 1024;

/// A finished capture: mono, 16-bit, at the rate the recorder was asked for.
#[derive(Clone, Debug, PartialEq)]
pub struct Recording {
    /// Mono PCM samples.
    pub samples: Vec<i16>,
    /// Sample rate of `samples` (the recorder's `target_rate_hz`).
    pub sample_rate_hz: u32,
    /// Length of `samples` in milliseconds (rounded down).
    pub duration_ms: u64,
    /// The recorder hit its `max_duration` and dropped everything after it.
    pub truncated: bool,
    /// Loudest sample in dBFS (`0.0` is full scale, `-90.0` is digital silence).
    pub peak_dbfs: f32,
}

impl Recording {
    /// Build from mono samples, deriving the duration and peak.
    pub fn from_samples(samples: Vec<i16>, sample_rate_hz: u32, truncated: bool) -> Self {
        let duration_ms = if sample_rate_hz == 0 { 0 } else { samples.len() as u64 * 1000 / u64::from(sample_rate_hz) };
        let peak = samples.iter().fold(0.0_f32, |acc, &s| acc.max(dsp::i16_to_f32(s).abs()));
        Self { samples, sample_rate_hz, duration_ms, truncated, peak_dbfs: dsp::to_dbfs(peak) }
    }

    /// Whether nothing worth uploading was captured: the loudest sample stays below
    /// [`SILENCE_PEAK_DBFS`] or the take is shorter than [`MIN_SPEECH_MS`].
    pub fn is_silent(&self) -> bool {
        self.duration_ms < MIN_SPEECH_MS || self.peak_dbfs < SILENCE_PEAK_DBFS
    }

    /// The recording as a RIFF/WAVE file: 16-bit PCM, one channel, 44-byte canonical header.
    pub fn to_wav(&self) -> Vec<u8> {
        encode_wav(&self.samples, self.sample_rate_hz)
    }
}

/// Serialise mono 16-bit PCM as a canonical 44-byte-header WAV.
pub fn encode_wav(samples: &[i16], sample_rate_hz: u32) -> Vec<u8> {
    const CHANNELS: u16 = 1;
    const BITS_PER_SAMPLE: u16 = 16;
    let block_align = CHANNELS * BITS_PER_SAMPLE / 8;
    let byte_rate = sample_rate_hz * u32::from(block_align);
    // A recording is bounded by `max_duration`, so the data size always fits the 32-bit field.
    let data_len = u32::try_from(samples.len() * usize::from(block_align)).unwrap_or(u32::MAX);
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16_u32.to_le_bytes()); // PCM fmt chunk size
    out.extend_from_slice(&1_u16.to_le_bytes()); // audio format: PCM
    out.extend_from_slice(&CHANNELS.to_le_bytes());
    out.extend_from_slice(&sample_rate_hz.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&BITS_PER_SAMPLE.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// Average the interleaved `channels` of `chunk` into one mono `-1.0..=1.0` float stream,
/// appending to `out`. A trailing partial frame (fewer than `channels` samples) is dropped.
pub fn downmix_chunk(chunk: SampleChunk<'_>, channels: u16, out: &mut Vec<f32>) {
    match chunk {
        SampleChunk::F32(s) => downmix_into(s, channels, |x| x, out),
        SampleChunk::I16(s) => downmix_into(s, channels, dsp::i16_to_f32, out),
        SampleChunk::U16(s) => downmix_into(s, channels, dsp::u16_to_f32, out),
        SampleChunk::I32(s) => downmix_into(s, channels, dsp::i32_to_f32, out),
    }
}

/// [`downmix_chunk`] for one concrete sample type.
pub fn downmix_into<T: Copy>(interleaved: &[T], channels: u16, to_f32: impl Fn(T) -> f32, out: &mut Vec<f32>) {
    let channels = usize::from(channels.max(1));
    let scale = 1.0 / channels as f32;
    for frame in interleaved.chunks_exact(channels) {
        out.push(frame.iter().map(|&s| to_f32(s)).sum::<f32>() * scale);
    }
}

/// Resample mono `input` from `from_hz` to `to_hz` with rubato's FFT resampler (delay trimmed,
/// output length `ceil(len * to / from)`). Equal rates return a copy.
pub fn resample_mono(input: &[f32], from_hz: u32, to_hz: u32) -> Result<Vec<f32>, AudioError> {
    if from_hz == to_hz {
        return Ok(input.to_vec());
    }
    if input.is_empty() {
        return Ok(Vec::new());
    }
    let mut resampler = Fft::<f32>::new(from_hz as usize, to_hz as usize, RESAMPLE_CHUNK_FRAMES, 1, FixedSync::Input)
        .map_err(|e| AudioError::Resample(format!("{from_hz} Hz -> {to_hz} Hz: {e}")))?;
    let adapter = InterleavedSlice::new(input, 1, input.len()).map_err(|e| AudioError::Resample(e.to_string()))?;
    let out = resampler.process_all(&adapter, input.len(), None).map_err(|e| AudioError::Resample(e.to_string()))?;
    Ok(out.take_data())
}

/// `-1.0..=1.0` float to `i16`, saturating.
pub fn f32_to_i16(s: f32) -> i16 {
    if s.is_nan() {
        return 0;
    }
    (f64::from(s) * 32_767.0).round().clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16
}

/// Peak of a mono float buffer in dBFS (`DBFS_FLOOR` when empty).
pub fn peak_dbfs(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return DBFS_FLOOR;
    }
    dsp::to_dbfs(dsp::peak(samples))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(frequency_hz: f64, rate: u32, seconds: f64, amplitude: f64) -> Vec<f32> {
        let n = (f64::from(rate) * seconds) as usize;
        (0..n).map(|i| (amplitude * (2.0 * std::f64::consts::PI * frequency_hz * i as f64 / f64::from(rate)).sin()) as f32).collect()
    }

    #[test]
    fn wav_header_is_canonical_pcm() {
        let samples: Vec<i16> = vec![0, 1, -1, i16::MAX, i16::MIN];
        let wav = encode_wav(&samples, 16_000);
        assert_eq!(wav.len(), 44 + 10);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes([wav[4], wav[5], wav[6], wav[7]]), 36 + 10);
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(u32::from_le_bytes([wav[16], wav[17], wav[18], wav[19]]), 16);
        assert_eq!(u16::from_le_bytes([wav[20], wav[21]]), 1, "PCM");
        assert_eq!(u16::from_le_bytes([wav[22], wav[23]]), 1, "mono");
        assert_eq!(u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]), 16_000);
        assert_eq!(u32::from_le_bytes([wav[28], wav[29], wav[30], wav[31]]), 32_000, "byte rate");
        assert_eq!(u16::from_le_bytes([wav[32], wav[33]]), 2, "block align");
        assert_eq!(u16::from_le_bytes([wav[34], wav[35]]), 16, "bits");
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]), 10);
        assert_eq!(&wav[44..48], &[0, 0, 1, 0]);
        assert_eq!(&wav[48..50], &(-1_i16).to_le_bytes());
        assert_eq!(&wav[50..54], &[0xFF, 0x7F, 0x00, 0x80]);
        let empty = encode_wav(&[], 8000);
        assert_eq!(empty.len(), 44);
        assert_eq!(u32::from_le_bytes([empty[4], empty[5], empty[6], empty[7]]), 36);
    }

    #[test]
    fn recording_derives_duration_peak_and_silence() {
        let loud = Recording::from_samples(vec![16_384; 16_000], 16_000, false);
        assert_eq!(loud.duration_ms, 1000);
        assert!((loud.peak_dbfs + 6.02).abs() < 0.01, "{}", loud.peak_dbfs);
        assert!(!loud.is_silent());
        assert!(!loud.truncated);
        assert_eq!(loud.to_wav().len(), 44 + 32_000);

        let quiet = Recording::from_samples(vec![20; 16_000], 16_000, false);
        assert!(quiet.peak_dbfs < SILENCE_PEAK_DBFS);
        assert!(quiet.is_silent(), "peak {} dBFS must count as silence", quiet.peak_dbfs);

        let short = Recording::from_samples(vec![16_384; 4000], 16_000, true);
        assert_eq!(short.duration_ms, 250);
        assert!(short.is_silent(), "250 ms is shorter than the 300 ms floor");
        assert!(short.truncated);

        let digital_silence = Recording::from_samples(vec![0; 16_000], 16_000, false);
        assert_eq!(digital_silence.peak_dbfs, DBFS_FLOOR);
        assert!(digital_silence.is_silent());

        let empty = Recording::from_samples(Vec::new(), 0, false);
        assert_eq!(empty.duration_ms, 0);
        assert!(empty.is_silent());
    }

    #[test]
    fn downmix_averages_channels_and_drops_partial_frames() {
        let mut out = Vec::new();
        downmix_chunk(SampleChunk::F32(&[1.0, 0.0, 0.5, 0.5, -1.0, 1.0, 0.25]), 2, &mut out);
        assert_eq!(out, vec![0.5, 0.5, 0.0]);
        downmix_chunk(SampleChunk::F32(&[0.3, 0.6]), 1, &mut out);
        assert_eq!(out.len(), 5);
        assert!((out[3] - 0.3).abs() < 1e-6);
        let mut zero_channels = Vec::new();
        downmix_chunk(SampleChunk::F32(&[0.7]), 0, &mut zero_channels);
        assert_eq!(zero_channels, vec![0.7]);
    }

    #[test]
    fn downmix_decodes_every_integer_format() {
        let mut out = Vec::new();
        downmix_chunk(SampleChunk::I16(&[16_384, -16_384, i16::MIN, i16::MIN]), 2, &mut out);
        assert_eq!(out, vec![0.0, -1.0]);
        downmix_chunk(SampleChunk::U16(&[32_768, 65_535, 0]), 1, &mut out);
        assert_eq!(out.len(), 5);
        assert_eq!(out[2], 0.0);
        assert!((out[3] - 0.99997).abs() < 1e-4);
        assert_eq!(out[4], -1.0);
        downmix_chunk(SampleChunk::I32(&[i32::MIN, 0]), 2, &mut out);
        assert_eq!(out[5], -0.5);
    }

    #[test]
    fn resample_48k_to_16k_keeps_length_and_level() {
        let input = sine(440.0, 48_000, 1.0, 0.5);
        let out = resample_mono(&input, 48_000, 16_000).unwrap();
        let expected = 16_000;
        let tolerance = expected / 100;
        assert!(out.len().abs_diff(expected) <= tolerance, "got {} samples, want {expected} ± {tolerance}", out.len());
        // Skip the edges: the FFT resampler's window ramps at the boundaries.
        let core = &out[800..out.len() - 800];
        let peak = dsp::peak(core);
        assert!((peak - 0.5).abs() < 0.02, "peak {peak}");
        let rms = dsp::rms(core);
        assert!((rms - 0.3536).abs() < 0.01, "rms {rms}");
    }

    #[test]
    fn resample_upsamples_passes_through_and_handles_empty() {
        let input = sine(1000.0, 16_000, 0.5, 0.25);
        let up = resample_mono(&input, 16_000, 44_100).unwrap();
        assert!(up.len().abs_diff(22_050) <= 220, "{}", up.len());
        let same = resample_mono(&input, 16_000, 16_000).unwrap();
        assert_eq!(same, input);
        assert!(resample_mono(&[], 48_000, 16_000).unwrap().is_empty());
        let odd = resample_mono(&input[..1001], 16_000, 16_000).unwrap();
        assert_eq!(odd.len(), 1001);
    }

    #[test]
    fn resample_rejects_zero_rates() {
        let err = resample_mono(&[0.0; 10], 0, 16_000).unwrap_err();
        assert!(matches!(&err, AudioError::Resample(msg) if msg.contains("0 Hz -> 16000 Hz")), "{err:?}");
        assert!(err.to_string().starts_with("resample:"));
        assert!(matches!(resample_mono(&[0.0; 10], 16_000, 0), Err(AudioError::Resample(_))));
    }

    #[test]
    fn float_to_i16_saturates() {
        assert_eq!(f32_to_i16(0.0), 0);
        assert_eq!(f32_to_i16(1.0), i16::MAX);
        assert_eq!(f32_to_i16(-1.0), -32_767);
        assert_eq!(f32_to_i16(2.0), i16::MAX);
        assert_eq!(f32_to_i16(-2.0), i16::MIN);
        assert_eq!(f32_to_i16(f32::NAN), 0);
        assert_eq!(f32_to_i16(0.5), 16_384);
    }

    #[test]
    fn peak_dbfs_of_floats() {
        assert_eq!(peak_dbfs(&[]), DBFS_FLOOR);
        assert!((peak_dbfs(&[0.1, -0.5, 0.2]) + 6.02).abs() < 0.01);
        assert!((peak_dbfs(&[1.0])).abs() < 1e-6);
    }
}
