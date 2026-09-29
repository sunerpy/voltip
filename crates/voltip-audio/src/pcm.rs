//! The long take's stream (docs/dictation.md §22): the whole take as mono 16 kHz `f32`, produced
//! on the audio thread and read by the core's recording thread, which writes it to the take's
//! recording file and cuts it into segments. A take that may run past two minutes keeps no more
//! than that in memory; the rest reaches the core through this stream.
//!
//! Audio-thread discipline, as for the live tap ([`crate::live`]): no allocation and no lock on
//! this path; a lock-free [`rtrb`] ring (60 s by default) sits between the callback and a thread
//! that only writes a file. Should the ring fill up anyway (the disk stalled for a minute), the
//! producer drops samples, but not silently: it records how many and where, and the consumer
//! reports the gap at exactly that point ([`PcmConsumer::gap`]). The file keeps its timeline and
//! the text can say which span went unrecognised.

use serde::{Deserialize, Serialize};

use crate::AudioError;
use crate::live::StreamResampler;

/// Default [`PcmStreamConfig::target_rate_hz`]: what the recognisers expect.
pub const DEFAULT_PCM_RATE_HZ: u32 = 16_000;
/// Default [`PcmStreamConfig::buffer_ms`]: a minute of slack between the audio thread and the
/// thread writing the file (960 000 samples, 3.84 MB).
pub const DEFAULT_PCM_BUFFER_MS: u32 = 60_000;
/// Gap reports the stream can hold before the reader picks them up. Each one is a separate time
/// the ring overflowed; this many unread is already a stalled machine.
const GAP_REPORTS: usize = 256;

/// How the long take's stream runs (`RecorderConfig::pcm_stream`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PcmStreamConfig {
    /// Sample rate of the stream.
    pub target_rate_hz: u32,
    /// Ring capacity in milliseconds; a reader further behind than this loses samples, and the
    /// stream reports where.
    pub buffer_ms: u32,
}

impl Default for PcmStreamConfig {
    fn default() -> Self {
        Self { target_rate_hz: DEFAULT_PCM_RATE_HZ, buffer_ms: DEFAULT_PCM_BUFFER_MS }
    }
}

impl PcmStreamConfig {
    /// Ring capacity in samples.
    pub fn capacity(&self) -> usize {
        usize::try_from(u64::from(self.target_rate_hz) * u64::from(self.buffer_ms) / 1000).unwrap_or(usize::MAX).max(1)
    }
}

/// `len` samples went missing right after the first `at` samples of the stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Gap {
    at: u64,
    len: u64,
}

/// Producer end, owned by the audio callback.
pub struct PcmProducer {
    ring: rtrb::Producer<f32>,
    gaps: rtrb::Producer<Gap>,
    /// Device rate → target rate; `None` when the input is at the target rate already (the mixer's
    /// output).
    resampler: Option<StreamResampler>,
    /// Samples written to the ring so far: the stream position of the next one.
    written: u64,
    /// Samples dropped at `written` and not reported yet.
    dropped: u64,
}

impl std::fmt::Debug for PcmProducer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PcmProducer").field("free", &self.ring.slots()).field("written", &self.written).field("dropped", &self.dropped).finish()
    }
}

impl PcmProducer {
    /// Swap the resampler (the device turned out to run at another rate than advertised).
    pub fn replace_resampler(&mut self, resampler: StreamResampler) {
        self.resampler = Some(resampler);
    }

    /// Resample one mono chunk at the device rate and push it. Without a resampler the chunk is
    /// taken as it is (it is at the target rate already).
    pub fn push(&mut self, mono: &[f32]) {
        let Some(mut resampler) = self.resampler.take() else {
            self.write(mono);
            return;
        };
        let result = resampler.push(mono, &mut |frames: &[f32]| self.write(frames));
        if result.is_err() {
            // Cannot happen after construction succeeded, but the audio thread must never panic:
            // the chunk counts as dropped.
            self.dropped += mono.len() as u64;
        }
        self.resampler = Some(resampler);
    }

    fn write(&mut self, samples: &[f32]) {
        if samples.is_empty() {
            return;
        }
        if self.dropped > 0 {
            // A drop is reported before the samples that follow it; until the report is out
            // (the report ring is full: nobody reads) nothing may follow it, or the reader would
            // place the gap at the wrong point.
            if self.ring.slots() == 0 || self.gaps.push(Gap { at: self.written, len: self.dropped }).is_err() {
                self.dropped += samples.len() as u64;
                return;
            }
            self.dropped = 0;
        }
        let (pushed, rest) = self.ring.push_partial_slice(samples);
        self.written += pushed.len() as u64;
        self.dropped += rest.len() as u64;
    }
}

/// Consumer end: what the recorder hands to the core's recording thread.
pub struct PcmConsumer {
    ring: rtrb::Consumer<f32>,
    gaps: rtrb::Consumer<Gap>,
    /// Samples read so far: the stream position of the next one.
    read: u64,
    sample_rate_hz: u32,
}

impl std::fmt::Debug for PcmConsumer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PcmConsumer").field("available", &self.ring.slots()).field("read", &self.read).field("closed", &self.is_closed()).finish()
    }
}

impl PcmConsumer {
    /// Copy up to `out.len()` samples out of the ring, in order, never past a gap (see
    /// [`PcmConsumer::gap`]); returns how many. Never blocks.
    pub fn read(&mut self, out: &mut [f32]) -> usize {
        let until_gap = self.gaps.peek().map_or(usize::MAX, |g| usize::try_from(g.at.saturating_sub(self.read)).unwrap_or(usize::MAX));
        let n = out.len().min(until_gap);
        let (popped, _) = self.ring.pop_partial_slice(&mut out[..n]);
        self.read += popped.len() as u64;
        popped.len()
    }

    /// How many samples are missing at the current position (they were dropped because the ring
    /// was full), once; `None` while the next sample follows the last one read.
    pub fn gap(&mut self) -> Option<u64> {
        let at = self.gaps.peek().ok()?.at;
        if at > self.read {
            return None;
        }
        self.gaps.pop().ok().map(|g| g.len)
    }

    /// The recorder stopped (the producer is gone): once [`PcmConsumer::read`] returns `0` and
    /// [`PcmConsumer::gap`] `None`, nothing more will come.
    pub fn is_closed(&self) -> bool {
        self.ring.is_abandoned()
    }

    /// Sample rate of the stream.
    pub fn sample_rate_hz(&self) -> u32 {
        self.sample_rate_hz
    }

    /// Samples waiting in the ring.
    pub fn available(&self) -> usize {
        self.ring.slots()
    }
}

/// Build the two ends of a stream fed at `input_rate_hz` (`None`: at the target rate already).
pub fn pcm_stream(config: &PcmStreamConfig, input_rate_hz: Option<u32>) -> Result<(PcmProducer, PcmConsumer), AudioError> {
    if config.target_rate_hz == 0 {
        return Err(AudioError::Resample("target rate must be greater than 0 Hz".into()));
    }
    let resampler = match input_rate_hz {
        Some(rate) if rate != config.target_rate_hz => Some(StreamResampler::new(rate, config.target_rate_hz)?),
        _ => None,
    };
    let (ring, ring_out) = rtrb::RingBuffer::new(config.capacity());
    let (gaps, gaps_out) = rtrb::RingBuffer::new(GAP_REPORTS);
    Ok((
        PcmProducer { ring, gaps, resampler, written: 0, dropped: 0 },
        PcmConsumer { ring: ring_out, gaps: gaps_out, read: 0, sample_rate_hz: config.target_rate_hz },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny(capacity_samples: u32) -> PcmStreamConfig {
        // 1000 Hz: one sample per millisecond, so `buffer_ms` is the capacity in samples.
        PcmStreamConfig { target_rate_hz: 1000, buffer_ms: capacity_samples }
    }

    /// Everything the reader gets, with each gap as its length in `NaN`s, until the stream ends.
    fn drain(consumer: &mut PcmConsumer) -> Vec<f32> {
        let mut out = Vec::new();
        let mut buf = [0.0f32; 7];
        loop {
            if let Some(len) = consumer.gap() {
                out.extend(std::iter::repeat_n(f32::NAN, usize::try_from(len).unwrap()));
                continue;
            }
            let n = consumer.read(&mut buf);
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
        }
        out
    }

    #[test]
    fn defaults_hold_a_minute_at_16_khz() {
        let config = PcmStreamConfig::default();
        assert_eq!((config.target_rate_hz, config.buffer_ms, config.capacity()), (16_000, 60_000, 960_000));
        assert_eq!(serde_json::from_str::<PcmStreamConfig>("{}").unwrap(), config);
        assert!(matches!(pcm_stream(&PcmStreamConfig { target_rate_hz: 0, buffer_ms: 1 }, None), Err(AudioError::Resample(_))));
    }

    /// In order, whole, and closed once the producer is gone.
    #[test]
    fn samples_arrive_in_order_and_the_stream_closes_with_the_producer() {
        let (mut producer, mut consumer) = pcm_stream(&tiny(100), None).unwrap();
        let input: Vec<f32> = (0..60).map(|i| i as f32).collect();
        producer.push(&input[..25]);
        producer.push(&input[25..]);
        assert_eq!(consumer.available(), 60);
        assert!(!consumer.is_closed());
        assert_eq!(drain(&mut consumer), input);
        drop(producer);
        assert!(consumer.is_closed());
        assert_eq!(consumer.sample_rate_hz(), 1000);
    }

    /// A full ring drops the rest of a push, and the reader learns how many samples are missing and
    /// exactly where: the timeline stays whole.
    #[test]
    fn a_full_ring_reports_the_dropped_span_at_its_place() {
        let (mut producer, mut consumer) = pcm_stream(&tiny(10), None).unwrap();
        producer.push(&[1.0; 8]);
        producer.push(&[2.0; 5]); // 2 fit, 3 are dropped
        producer.push(&[3.0; 4]); // nothing fits: 4 more dropped at the same point
        let mut buf = [0.0f32; 20];
        assert_eq!(consumer.read(&mut buf), 10, "the ring's content");
        assert_eq!(&buf[..10], &[1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 2.0, 2.0]);
        assert_eq!(consumer.gap(), None, "the drop is reported with the next sample that fits");
        producer.push(&[4.0; 3]);
        assert_eq!(consumer.gap(), Some(7), "3 + 4 samples missing after the first 10");
        assert_eq!(consumer.gap(), None, "once");
        assert_eq!(consumer.read(&mut buf), 3);
        assert_eq!(&buf[..3], &[4.0, 4.0, 4.0]);
    }

    /// The reader never reads across a gap: samples after it wait until the gap was taken.
    #[test]
    fn reads_stop_at_a_gap_until_it_is_taken() {
        let (mut producer, mut consumer) = pcm_stream(&tiny(4), None).unwrap();
        producer.push(&[1.0; 6]); // 4 in, 2 dropped
        let mut buf = [0.0f32; 3];
        assert_eq!(consumer.read(&mut buf), 3);
        producer.push(&[2.0; 2]); // room again: the drop is reported, then both samples go in
        assert_eq!(consumer.read(&mut buf), 1, "only up to the gap");
        assert_eq!(consumer.read(&mut buf), 0, "the gap comes first");
        assert_eq!(consumer.gap(), Some(2));
        assert_eq!(consumer.read(&mut buf), 2);
        assert_eq!(&buf[..2], &[2.0, 2.0]);
        drop(producer);
        assert_eq!(drain(&mut consumer), Vec::<f32>::new());
        assert!(consumer.is_closed());
    }

    /// End to end with the drain helper: the gap sits between the samples on either side of it.
    #[test]
    fn the_drained_stream_keeps_the_gap_in_place() {
        let (mut producer, mut consumer) = pcm_stream(&tiny(3), None).unwrap();
        producer.push(&[1.0, 2.0, 3.0, 4.0]); // the 4.0 is dropped
        let mut buf = [0.0f32; 3];
        assert_eq!(consumer.read(&mut buf), 3);
        producer.push(&[5.0]);
        drop(producer);
        let rest = drain(&mut consumer);
        assert!(rest[0].is_nan() && rest.len() == 2 && rest[1] == 5.0, "{rest:?}");
    }

    /// A device-rate input is resampled to the target rate on the way in.
    #[test]
    fn a_device_rate_input_is_resampled_to_16_khz() {
        let config = PcmStreamConfig::default();
        let (mut producer, mut consumer) = pcm_stream(&config, Some(48_000)).unwrap();
        let second: Vec<f32> = (0..48_000).map(|i| (i as f32 * 0.01).sin() * 0.5).collect();
        for chunk in second.chunks(480) {
            producer.push(chunk);
        }
        let got = drain(&mut consumer);
        // The resampler holds back its start-up delay until more input arrives: about a second out.
        assert!((15_500..=16_000).contains(&got.len()), "{}", got.len());
        assert!(got.iter().all(|s| s.is_finite() && s.abs() <= 0.6));
        let mut replaced = pcm_stream(&config, None).unwrap().0;
        replaced.replace_resampler(StreamResampler::new(44_100, 16_000).unwrap());
        replaced.push(&[0.0; 4410]);
        assert!(replaced.written > 0);
    }
}
