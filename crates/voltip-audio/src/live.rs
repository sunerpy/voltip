//! The recorder's live tap (docs/dictation.md §11): a second, real-time copy of the capture as
//! mono 16 kHz `f32`, produced on the audio thread and consumed by the streaming recogniser's
//! decode thread.
//!
//! Audio-thread discipline: the callback allocates nothing and takes no lock on this path. The
//! resampler ([`StreamResampler`], rubato's asynchronous sinc with a persistent state, fed chunk
//! by chunk as cpal delivers them) writes into pre-allocated buffers; the frames go into an
//! [`rtrb`] ring (lock-free, single producer / single consumer). When the ring is full the
//! producer does *not* silently discard: it sets the sticky `overrun` flag that the consumer
//! reports, so the preview is marked degraded instead of quietly missing words. The whole-take
//! recording is untouched by any of this.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Async, FixedAsync, Indexing, Resampler, SincInterpolationParameters, SincInterpolationType, WindowFunction};
use serde::{Deserialize, Serialize};

use crate::AudioError;

/// Default [`LiveTapConfig::target_rate_hz`]: what the streaming recogniser expects.
pub const DEFAULT_LIVE_RATE_HZ: u32 = 16_000;
/// Default [`LiveTapConfig::buffer_ms`]: two seconds of slack between the audio thread and the
/// decoder (32 000 samples at 16 kHz).
pub const DEFAULT_LIVE_BUFFER_MS: u32 = 2000;

/// Input frames per resampler step. Small, so the tap's latency stays a few milliseconds; the
/// accumulator in front of it absorbs whatever chunk size the device uses.
const STREAM_CHUNK_FRAMES: usize = 256;
/// Windowed-sinc length: rubato's recommended starting point; ~2 M multiply-adds per second at
/// 16 kHz output, well under a percent of one core.
const SINC_LEN: usize = 256;
/// Device-rate frames reserved for the input accumulator (cpal chunks are a few thousand frames
/// at most).
const PENDING_RESERVE_FRAMES: usize = 8192;

/// How the live tap runs (`RecorderConfig::live_tap`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LiveTapConfig {
    /// Sample rate of the tapped audio.
    pub target_rate_hz: u32,
    /// Ring capacity in milliseconds of tapped audio; a decoder further behind than this loses
    /// samples and the tap reports an overrun.
    pub buffer_ms: u32,
}

impl Default for LiveTapConfig {
    fn default() -> Self {
        Self { target_rate_hz: DEFAULT_LIVE_RATE_HZ, buffer_ms: DEFAULT_LIVE_BUFFER_MS }
    }
}

impl LiveTapConfig {
    /// Ring capacity in samples.
    pub fn capacity(&self) -> usize {
        usize::try_from(u64::from(self.target_rate_hz) * u64::from(self.buffer_ms) / 1000).unwrap_or(usize::MAX).max(1)
    }
}

/// Chunk-by-chunk mono resampler with persistent state: feed the device's chunks in the order
/// they arrive and get the equivalent of resampling the whole take, without waiting for it.
/// Equal rates pass the samples through untouched.
pub struct StreamResampler {
    inner: Option<Async<f32>>,
    /// Device-rate frames waiting for a full resampler step.
    pending: Vec<f32>,
    /// Pre-allocated output of one step (`output_frames_max` long).
    out: Vec<f32>,
    /// Output frames still to drop: the resampler's start-up delay, so the output lines up with
    /// `recording::resample_mono`'s trimmed output.
    trim: usize,
    step: usize,
    /// Input frames pushed and output frames emitted since the last flush (for the flush length).
    fed: usize,
    emitted: usize,
}

impl std::fmt::Debug for StreamResampler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamResampler").field("pending", &self.pending.len()).field("trim", &self.trim).field("passthrough", &self.inner.is_none()).finish()
    }
}

impl StreamResampler {
    /// A resampler from `from_hz` to `to_hz` (mono).
    pub fn new(from_hz: u32, to_hz: u32) -> Result<Self, AudioError> {
        if from_hz == 0 || to_hz == 0 {
            return Err(AudioError::Resample(format!("{from_hz} Hz -> {to_hz} Hz: rates must be greater than 0")));
        }
        if from_hz == to_hz {
            return Ok(Self { inner: None, pending: Vec::new(), out: Vec::new(), trim: 0, step: 0, fed: 0, emitted: 0 });
        }
        let params = SincInterpolationParameters {
            sinc_len: SINC_LEN,
            f_cutoff: None,
            oversampling_factor: 128,
            interpolation: SincInterpolationType::Cubic,
            window: WindowFunction::BlackmanHarris2,
        };
        let inner = Async::<f32>::new_sinc(f64::from(to_hz) / f64::from(from_hz), 1.0, &params, STREAM_CHUNK_FRAMES, 1, FixedAsync::Input)
            .map_err(|e| AudioError::Resample(format!("{from_hz} Hz -> {to_hz} Hz: {e}")))?;
        let step = inner.input_frames_next();
        let out = vec![0.0; inner.output_frames_max()];
        let trim = inner.output_delay();
        // Room for the largest chunk a device is likely to deliver plus one step, so the audio
        // thread never grows this buffer in normal use.
        Ok(Self { inner: Some(inner), pending: Vec::with_capacity(PENDING_RESERVE_FRAMES + step), out, trim, step, fed: 0, emitted: 0 })
    }

    /// Resample `chunk` (device rate, mono) and append what is ready to `sink`. Frames that do not
    /// fill a step stay buffered for the next call. Only allocates when a chunk is larger than
    /// anything seen before.
    pub fn push(&mut self, chunk: &[f32], sink: &mut impl FnMut(&[f32])) -> Result<(), AudioError> {
        let Some(inner) = &mut self.inner else {
            sink(chunk);
            return Ok(());
        };
        self.pending.extend_from_slice(chunk);
        self.fed += chunk.len();
        let mut consumed = 0;
        while self.pending.len() - consumed >= self.step {
            let input = InterleavedSlice::new(&self.pending[consumed..consumed + self.step], 1, self.step).map_err(|e| AudioError::Resample(e.to_string()))?;
            let frames = self.out.len();
            let mut output = InterleavedSlice::new_mut(&mut self.out, 1, frames).map_err(|e| AudioError::Resample(e.to_string()))?;
            let (used, produced) = inner.process_into_buffer(&input, &mut output, None).map_err(|e| AudioError::Resample(e.to_string()))?;
            consumed += used;
            Self::emit(&mut self.trim, &mut self.emitted, &self.out, produced, usize::MAX, sink);
        }
        self.pending.drain(..consumed);
        Ok(())
    }

    /// Flush the buffered tail (padded with silence) so the output covers the whole input:
    /// exactly `ceil(frames_pushed * ratio)` frames in total, like rubato's `process_all`. The
    /// resampler is reset afterwards. For tests and end-of-take use, not the audio thread.
    pub fn flush(&mut self, sink: &mut impl FnMut(&[f32])) -> Result<(), AudioError> {
        let Some(inner) = &mut self.inner else { return Ok(()) };
        let expected = (inner.resample_ratio() * self.fed as f64).ceil() as usize;
        let mut first = true;
        while self.emitted < expected {
            let partial = if first { self.pending.len() } else { 0 };
            let input = InterleavedSlice::new(&self.pending, 1, partial).map_err(|e| AudioError::Resample(e.to_string()))?;
            let indexing = Indexing { partial_len: Some(partial), ..Indexing::default() };
            let frames = self.out.len();
            let mut output = InterleavedSlice::new_mut(&mut self.out, 1, frames).map_err(|e| AudioError::Resample(e.to_string()))?;
            let (_, produced) = inner.process_into_buffer(&input, &mut output, Some(&indexing)).map_err(|e| AudioError::Resample(e.to_string()))?;
            Self::emit(&mut self.trim, &mut self.emitted, &self.out, produced, expected, sink);
            first = false;
        }
        self.pending.clear();
        inner.reset();
        self.trim = inner.output_delay();
        self.fed = 0;
        self.emitted = 0;
        Ok(())
    }

    /// Hand `produced` frames of `out` to `sink`, minus the start-up delay still to trim and
    /// capped so the total emitted never exceeds `limit`.
    fn emit(trim: &mut usize, emitted: &mut usize, out: &[f32], produced: usize, limit: usize, sink: &mut impl FnMut(&[f32])) {
        let skip = produced.min(*trim);
        *trim -= skip;
        let end = produced.min(skip.saturating_add(limit.saturating_sub(*emitted)));
        if end > skip {
            *emitted += end - skip;
            sink(&out[skip..end]);
        }
    }
}

/// Consumer end of the live tap: what the recorder hands to the decode thread.
pub struct LiveConsumer {
    consumer: rtrb::Consumer<f32>,
    overrun: Arc<AtomicBool>,
    sample_rate_hz: u32,
}

impl std::fmt::Debug for LiveConsumer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveConsumer").field("available", &self.consumer.slots()).field("overrun", &self.overrun()).field("closed", &self.is_closed()).finish()
    }
}

impl LiveConsumer {
    /// Copy up to `out.len()` samples out of the ring; returns how many. Never blocks.
    pub fn read(&mut self, out: &mut [f32]) -> usize {
        let (popped, _) = self.consumer.pop_partial_slice(out);
        popped.len()
    }

    /// The producer had to drop samples (sticky).
    pub fn overrun(&self) -> bool {
        self.overrun.load(Ordering::Relaxed)
    }

    /// The recorder stopped (the producer is gone); drain with [`LiveConsumer::read`] until `0`.
    pub fn is_closed(&self) -> bool {
        self.consumer.is_abandoned()
    }

    /// Sample rate of the tapped audio.
    pub fn sample_rate_hz(&self) -> u32 {
        self.sample_rate_hz
    }

    /// Samples waiting in the ring.
    pub fn available(&self) -> usize {
        self.consumer.slots()
    }
}

/// Producer end, owned by the audio callback.
pub struct LiveProducer {
    producer: rtrb::Producer<f32>,
    overrun: Arc<AtomicBool>,
    resampler: StreamResampler,
}

impl std::fmt::Debug for LiveProducer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveProducer").field("free", &self.producer.slots()).field("resampler", &self.resampler).finish()
    }
}

impl LiveProducer {
    /// Swap the resampler (the device turned out to run at another rate than advertised). The ring
    /// and the overrun flag stay.
    pub fn replace_resampler(&mut self, resampler: StreamResampler) {
        self.resampler = resampler;
    }

    /// Resample one mono chunk at the device rate and push it. A full ring drops the remainder and
    /// raises the overrun flag; a resampler error is reported once as an overrun too (it cannot
    /// happen after construction succeeded, but the audio thread must never panic).
    pub fn push(&mut self, mono: &[f32]) {
        let producer = &mut self.producer;
        let overrun = &self.overrun;
        let result = self.resampler.push(mono, &mut |frames: &[f32]| {
            let (_, rest) = producer.push_partial_slice(frames);
            if !rest.is_empty() {
                overrun.store(true, Ordering::Relaxed);
            }
        });
        if result.is_err() {
            self.overrun.store(true, Ordering::Relaxed);
        }
    }
}

/// Build the two ends of a tap for a device running at `device_rate_hz`.
pub fn live_tap(config: &LiveTapConfig, device_rate_hz: u32) -> Result<(LiveProducer, LiveConsumer), AudioError> {
    let resampler = StreamResampler::new(device_rate_hz, config.target_rate_hz)?;
    let (producer, consumer) = rtrb::RingBuffer::new(config.capacity());
    let overrun = Arc::new(AtomicBool::new(false));
    Ok((LiveProducer { producer, overrun: overrun.clone(), resampler }, LiveConsumer { consumer, overrun, sample_rate_hz: config.target_rate_hz }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::resample_mono;

    fn sine(frequency_hz: f64, rate: u32, seconds: f64, amplitude: f64) -> Vec<f32> {
        let n = (f64::from(rate) * seconds) as usize;
        (0..n).map(|i| (amplitude * (2.0 * std::f64::consts::PI * frequency_hz * i as f64 / f64::from(rate)).sin()) as f32).collect()
    }

    fn rms_diff(a: &[f32], b: &[f32]) -> f32 {
        let n = a.len().min(b.len());
        assert!(n > 0);
        (a[..n].iter().zip(&b[..n]).map(|(x, y)| (x - y) * (x - y)).sum::<f32>() / n as f32).sqrt()
    }

    /// The gate of docs/dictation.md §11: feeding the take chunk by chunk (in the odd sizes a
    /// device may use) yields the same 16 kHz signal as resampling the whole take at once.
    #[test]
    fn chunked_resampling_matches_the_whole_take_within_1e_3_rms() {
        let input = sine(440.0, 48_000, 2.0, 0.5);
        // Whole take through the same resampler type (rubato trims the delay itself).
        let mut whole = StreamResampler::new(48_000, 16_000).unwrap();
        let mut whole_out = Vec::new();
        whole.push(&input, &mut |f| whole_out.extend_from_slice(f)).unwrap();
        whole.flush(&mut |f| whole_out.extend_from_slice(f)).unwrap();
        assert_eq!(whole_out.len(), 32_000, "exactly ceil(96000 / 3) frames");
        // Chunked: irregular chunk sizes like a real device (480, 441, 1024, 7 ...).
        let mut chunked = StreamResampler::new(48_000, 16_000).unwrap();
        let mut chunked_out = Vec::new();
        let sizes = [480usize, 441, 1024, 7, 2048, 100, 480];
        let mut offset = 0;
        let mut i = 0;
        while offset < input.len() {
            let n = sizes[i % sizes.len()].min(input.len() - offset);
            chunked.push(&input[offset..offset + n], &mut |f| chunked_out.extend_from_slice(f)).unwrap();
            offset += n;
            i += 1;
        }
        chunked.flush(&mut |f| chunked_out.extend_from_slice(f)).unwrap();
        assert_eq!(chunked_out.len(), whole_out.len(), "same number of frames");
        let diff = rms_diff(&whole_out, &chunked_out);
        assert!(diff < 1e-3, "chunked vs whole-take RMS difference {diff}");
        // And against the recorder's FFT resampler (a different algorithm, so a fraction of a
        // sample of group delay apart — not comparable sample by sample): same length, level and
        // peak in the steady state, so the preview hears what the whole take will.
        let fft = resample_mono(&input, 48_000, 16_000).unwrap();
        assert_eq!(fft.len(), chunked_out.len());
        let core = 2000..30_000;
        let (rms_fft, rms_sinc) = (crate::dsp::rms(&fft[core.clone()]), crate::dsp::rms(&chunked_out[core.clone()]));
        assert!((rms_fft - rms_sinc).abs() < 1e-3, "levels: fft {rms_fft} vs sinc {rms_sinc}");
        assert!((rms_sinc - 0.3536).abs() < 0.002, "rms {rms_sinc}");
        let peak = crate::dsp::peak(&chunked_out[core]);
        assert!((peak - 0.5).abs() < 0.01, "peak {peak}");
        assert!(format!("{chunked:?}").contains("StreamResampler"));
    }

    #[test]
    fn passthrough_upsampling_and_errors() {
        let mut same = StreamResampler::new(16_000, 16_000).unwrap();
        let mut out = Vec::new();
        same.push(&[0.1, 0.2, 0.3], &mut |f| out.extend_from_slice(f)).unwrap();
        same.flush(&mut |f| out.extend_from_slice(f)).unwrap();
        assert_eq!(out, vec![0.1, 0.2, 0.3]);
        assert!(format!("{same:?}").contains("passthrough: true"));
        let input = sine(1000.0, 8000, 0.5, 0.25);
        let mut up = StreamResampler::new(8000, 16_000).unwrap();
        let mut out = Vec::new();
        for chunk in input.chunks(160) {
            up.push(chunk, &mut |f| out.extend_from_slice(f)).unwrap();
        }
        up.flush(&mut |f| out.extend_from_slice(f)).unwrap();
        assert_eq!(out.len(), 8000);
        // Reusable after a flush.
        let mut again = Vec::new();
        up.push(&input[..800], &mut |f| again.extend_from_slice(f)).unwrap();
        up.flush(&mut |f| again.extend_from_slice(f)).unwrap();
        assert_eq!(again.len(), 1600);
        let peak = crate::dsp::peak(&out[1000..7000]);
        assert!((peak - 0.25).abs() < 0.01, "peak {peak}");
        assert!(matches!(StreamResampler::new(0, 16_000), Err(AudioError::Resample(_))));
        assert!(matches!(StreamResampler::new(48_000, 0), Err(AudioError::Resample(_))));
        assert!(matches!(live_tap(&LiveTapConfig { target_rate_hz: 0, ..LiveTapConfig::default() }, 48_000), Err(AudioError::Resample(_))));
    }

    #[test]
    fn tap_carries_frames_reports_overrun_and_closes_with_the_producer() {
        let config = LiveTapConfig::default();
        assert_eq!(config, LiveTapConfig { target_rate_hz: 16_000, buffer_ms: 2000 });
        assert_eq!(config.capacity(), 32_000);
        assert_eq!(serde_json::from_str::<LiveTapConfig>("{}").unwrap(), config);
        assert_eq!(LiveTapConfig { target_rate_hz: 16_000, buffer_ms: 0 }.capacity(), 1);
        let (mut producer, mut consumer) = live_tap(&LiveTapConfig { buffer_ms: 100, ..config }, 48_000).unwrap();
        assert_eq!(consumer.sample_rate_hz(), 16_000);
        assert!(!consumer.overrun() && !consumer.is_closed() && consumer.available() == 0);
        // 48 kHz → 16 kHz: 4800 device frames become ~1600 samples (minus the start-up delay).
        producer.push(&sine(440.0, 48_000, 0.1, 0.5));
        let mut buf = vec![0.0; 4000];
        let n = consumer.read(&mut buf);
        assert!((1400..=1600).contains(&n), "{n}");
        assert_eq!(consumer.read(&mut buf), 0, "drained");
        assert!(!consumer.overrun());
        assert!(format!("{producer:?}").contains("LiveProducer") && format!("{consumer:?}").contains("LiveConsumer"));
        // Nobody reads: the 100 ms ring fills, the surplus is dropped and the flag is raised.
        producer.push(&sine(440.0, 48_000, 0.5, 0.5));
        assert!(consumer.overrun(), "a full ring is reported, not swallowed");
        assert!(consumer.available() <= 1600);
        let n = consumer.read(&mut buf);
        assert!(n > 0 && n <= 1600);
        // Dropping the producer closes the tap; what is left can still be read.
        producer.push(&[0.0; 480]);
        drop(producer);
        assert!(consumer.is_closed());
        let mut total = 0;
        loop {
            let n = consumer.read(&mut buf);
            if n == 0 {
                break;
            }
            total += n;
        }
        assert!(total > 0);
        assert_eq!(consumer.read(&mut buf), 0);
    }
}
