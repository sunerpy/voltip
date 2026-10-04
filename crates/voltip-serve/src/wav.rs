//! Decoding an uploaded WAV as it arrives (docs/dictation.md §23.4): the bytes come over a channel
//! from the request body, hound reads them on a blocking thread, and the samples leave as the
//! app's recording format — 16 kHz mono 16-bit — into a file. The raw upload is never stored.
//! Every bound is checked while reading: the header must sit in the first 64 KiB, the format must
//! be one this decoder reads, the declared and the decoded length must stay within the limit.

use std::io::{BufWriter, Read, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use axum::body::Bytes;
use tokio::sync::mpsc;
use voltip_core::serve::RATE;

use crate::resample::StreamResampler;

/// The header (everything before the samples) must fit in this many bytes.
pub const HEADER_WINDOW: u64 = 64 * 1024;
/// Most channels an upload may have.
pub const MAX_CHANNELS: u16 = 8;
/// Lowest and highest sample rates an upload may have.
pub const RATES: std::ops::RangeInclusive<u32> = 8_000..=192_000;

/// Why an upload could not be decoded.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    /// Not a WAV this decoder reads (415).
    #[error("{0}")]
    Unsupported(String),
    /// Longer than the limit (413).
    #[error("{0}")]
    TooLong(String),
    /// A WAV whose data is broken or cut off (400).
    #[error("{0}")]
    Broken(String),
    /// The output file could not be written (500).
    #[error("{0}")]
    Io(String),
}

/// A `Read` over the chunks the request body delivers; the end of the channel is the end of the
/// file. While the header is read, more than [`HEADER_WINDOW`] bytes is an error.
struct ChannelReader {
    rx: mpsc::Receiver<Bytes>,
    current: Bytes,
    read: Arc<AtomicU64>,
    in_header: Arc<AtomicBool>,
}

impl Read for ChannelReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        while self.current.is_empty() {
            match self.rx.blocking_recv() {
                Some(chunk) => self.current = chunk,
                None => return Ok(0),
            }
        }
        let n = out.len().min(self.current.len());
        out[..n].copy_from_slice(&self.current[..n]);
        let _ = self.current.split_to(n);
        let total = self.read.fetch_add(n as u64, Ordering::SeqCst) + n as u64;
        if self.in_header.load(Ordering::SeqCst) && total > HEADER_WINDOW {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "header past 64 KiB"));
        }
        Ok(n)
    }
}

/// What the decoder wrote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoded {
    /// 16 kHz samples in the file.
    pub samples: u64,
    /// The header's format, for the log: channels, rate, bits.
    pub format: (u16, u32, u16),
}

/// The sending end the request body feeds, and the decoder running on a blocking thread.
pub struct Decoder {
    /// Chunks of the file field go here; dropping it ends the file.
    pub tx: mpsc::Sender<Bytes>,
    /// Bytes the decoder has consumed so far.
    pub consumed: Arc<AtomicU64>,
    /// The decoder's result.
    pub task: tokio::task::JoinHandle<Result<Decoded, DecodeError>>,
}

/// Start decoding into `out` (created, mode 0600 on Unix), allowing at most `max_samples` samples
/// at 16 kHz.
pub fn start(out: PathBuf, max_samples: u64) -> Decoder {
    let (tx, rx) = mpsc::channel::<Bytes>(16);
    let consumed = Arc::new(AtomicU64::new(0));
    let reader = ChannelReader { rx, current: Bytes::new(), read: consumed.clone(), in_header: Arc::new(AtomicBool::new(true)) };
    let task = tokio::task::spawn_blocking(move || decode(reader, &out, max_samples));
    Decoder { tx, consumed, task }
}

fn open_output(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)
}

fn minutes(samples: u64) -> String {
    format!("{:.1} 分钟", samples as f64 / RATE as f64 / 60.0)
}

fn decode(reader: ChannelReader, out: &Path, max_samples: u64) -> Result<Decoded, DecodeError> {
    let in_header = reader.in_header.clone();
    let reader = hound::WavReader::new(reader).map_err(|e| DecodeError::Unsupported(format!("不是可用的 WAV 文件（{e}）；只接受 WAV")))?;
    in_header.store(false, Ordering::SeqCst);
    let spec = reader.spec();
    let format = (spec.channels, spec.sample_rate, spec.bits_per_sample);
    if spec.channels == 0 || spec.channels > MAX_CHANNELS {
        return Err(DecodeError::Unsupported(format!("不支持 {} 个声道（最多 {MAX_CHANNELS} 个）", spec.channels)));
    }
    if !RATES.contains(&spec.sample_rate) {
        return Err(DecodeError::Unsupported(format!("不支持 {} Hz 的采样率（8–192 kHz）", spec.sample_rate)));
    }
    let float = match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Int, 8 | 16 | 24 | 32) => false,
        (hound::SampleFormat::Float, 32) => true,
        (kind, bits) => return Err(DecodeError::Unsupported(format!("不支持 {bits} 位 {kind:?} 采样"))),
    };
    let declared = u64::from(reader.duration()) * RATE / u64::from(spec.sample_rate);
    if declared > max_samples {
        return Err(DecodeError::TooLong(format!("音频长 {}，超过上限 {}", minutes(declared), minutes(max_samples))));
    }
    let file = open_output(out).map_err(|e| DecodeError::Io(format!("临时文件无法创建：{e}")))?;
    let mut writer = BufWriter::new(file);
    let mut resampler = StreamResampler::new(spec.sample_rate, RATE as u32).map_err(DecodeError::Unsupported)?;
    let channels = usize::from(spec.channels);
    let scale = if float { 1.0 } else { 1.0 / (1u64 << (spec.bits_per_sample - 1)) as f32 };
    let mut written = 0u64;
    {
        let mut sink = |block: &[f32]| -> Result<(), String> {
            written += block.len() as u64;
            if written > max_samples {
                return Err(format!("音频超过上限 {}", minutes(max_samples)));
            }
            let mut bytes = Vec::with_capacity(block.len() * 2);
            for &s in block {
                let v = (f64::from(s.clamp(-1.0, 1.0)) * 32_767.0).round() as i16;
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            writer.write_all(&bytes).map_err(|e| format!("临时文件无法写入：{e}"))
        };
        let mut frame = Vec::with_capacity(channels);
        let mut block = Vec::with_capacity(4096);
        let mut reader = reader;
        let feed = |sample: f32, block: &mut Vec<f32>, frame: &mut Vec<f32>| {
            frame.push(sample);
            if frame.len() == channels {
                block.push(frame.iter().sum::<f32>() / channels as f32);
                frame.clear();
            }
        };
        let fail = |e: String| if e.starts_with("音频超过上限") { DecodeError::TooLong(e) } else { DecodeError::Io(e) };
        if float {
            for sample in reader.samples::<f32>() {
                feed(sample.map_err(|e| DecodeError::Broken(format!("音频数据不完整（{e}）")))?, &mut block, &mut frame);
                if block.len() >= 4096 {
                    resampler.push(&block, &mut sink).map_err(fail)?;
                    block.clear();
                }
            }
        } else {
            for sample in reader.samples::<i32>() {
                let sample = sample.map_err(|e| DecodeError::Broken(format!("音频数据不完整（{e}）")))?;
                feed(sample as f32 * scale, &mut block, &mut frame);
                if block.len() >= 4096 {
                    resampler.push(&block, &mut sink).map_err(fail)?;
                    block.clear();
                }
            }
        }
        resampler.push(&block, &mut sink).map_err(fail)?;
        resampler.flush(&mut sink).map_err(fail)?;
    }
    writer.flush().map_err(|e| DecodeError::Io(format!("临时文件无法写入：{e}")))?;
    Ok(Decoded { samples: written, format })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn wav(spec: hound::WavSpec, frames: usize, value: f32) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = hound::WavWriter::new(&mut buf, spec).unwrap();
            for i in 0..frames * usize::from(spec.channels) {
                let v = if i % 7 == 0 { value } else { value / 2.0 };
                match spec.sample_format {
                    hound::SampleFormat::Float => w.write_sample(v).unwrap(),
                    hound::SampleFormat::Int => w.write_sample((v * ((1i64 << (spec.bits_per_sample - 1)) - 1) as f32) as i32).unwrap(),
                }
            }
            w.finalize().unwrap();
        }
        buf.into_inner()
    }

    async fn run(bytes: Vec<u8>, max_samples: u64, chunk: usize) -> (Result<Decoded, DecodeError>, Vec<i16>) {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("take.pcm");
        let decoder = start(out.clone(), max_samples);
        for piece in bytes.chunks(chunk) {
            if decoder.tx.send(Bytes::copy_from_slice(piece)).await.is_err() {
                break;
            }
        }
        drop(decoder.tx);
        let result = decoder.task.await.unwrap();
        let pcm = std::fs::read(&out).map(|b| b.as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c)).collect()).unwrap_or_default();
        (result, pcm)
    }

    fn spec(channels: u16, rate: u32, bits: u16, float: bool) -> hound::WavSpec {
        hound::WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: bits,
            sample_format: if float { hound::SampleFormat::Float } else { hound::SampleFormat::Int },
        }
    }

    #[tokio::test]
    async fn every_supported_format_becomes_16_khz_mono() {
        for (channels, rate, bits, float) in
            [(1, 24_000, 16, false), (2, 48_000, 16, false), (1, 8_000, 8, false), (2, 44_100, 24, false), (1, 16_000, 32, true), (6, 32_000, 32, false)]
        {
            let frames = rate as usize;
            let (result, pcm) = run(wav(spec(channels, rate, bits, float), frames, 0.6), RATE * 10, 3000).await;
            let decoded = result.unwrap();
            assert_eq!(decoded.samples, RATE, "{channels} ch {rate} Hz {bits} bit: one second");
            assert_eq!(pcm.len() as u64, RATE);
            let peak = pcm[2000..14_000].iter().map(|s| i32::from(*s).abs()).max().unwrap();
            assert!(peak > 9_000, "{channels} ch {rate} Hz {bits} bit: level kept ({peak})");
        }
    }

    #[tokio::test]
    async fn the_limits_hold_while_reading() {
        // Declared longer than allowed: refused from the header alone.
        let (result, pcm) = run(wav(spec(1, 16_000, 16, false), 16_000 * 3, 0.5), RATE * 2, 4096).await;
        assert!(matches!(result, Err(DecodeError::TooLong(_))), "{result:?}");
        assert!(pcm.is_empty(), "nothing written");
        // Not a WAV, and a header past 64 KiB.
        let (result, _) = run(b"ID3\x04 not a wav at all".to_vec(), RATE, 8).await;
        assert!(matches!(result, Err(DecodeError::Unsupported(_))), "{result:?}");
        let mut padded = b"RIFF\xff\xff\xff\x7fWAVE".to_vec();
        padded.extend_from_slice(b"LIST");
        padded.extend_from_slice(&(100_000u32).to_le_bytes());
        padded.extend(std::iter::repeat_n(0u8, 100_000));
        let (result, _) = run(padded, RATE, 4096).await;
        assert!(matches!(result, Err(DecodeError::Unsupported(_))), "{result:?}");
        // Unsupported formats.
        for bad in [spec(9, 16_000, 16, false), spec(1, 4_000, 16, false)] {
            let (result, _) = run(wav(bad, 100, 0.5), RATE, 4096).await;
            assert!(matches!(result, Err(DecodeError::Unsupported(_))), "{bad:?}: {result:?}");
        }
        // Cut off in the middle of the data.
        let full = wav(spec(1, 16_000, 16, false), 16_000, 0.5);
        let (result, _) = run(full[..full.len() / 2].to_vec(), RATE * 2, 4096).await;
        assert!(matches!(result, Err(DecodeError::Broken(_))), "{result:?}");
    }
}
