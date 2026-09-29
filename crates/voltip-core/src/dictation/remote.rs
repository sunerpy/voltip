//! Audio from a paired phone (docs/dictation.md §20): the phone streams PCM16 mono at 16 kHz in
//! [`AppMessage::TakeAudio`](voltip_protocol::app::AppMessage::TakeAudio) chunks and the desktop
//! runs an ordinary take on it. [`RemoteFeed`] is the runtime's end — it appends each chunk in
//! order — and [`RemoteFeed::source`] is the [`AudioSource`] the engine opens for that take instead
//! of the microphone: level frames, the `ready` mark, the live tap for the streaming preview and
//! the whole-take recording all come from the streamed samples.
//!
//! The engine opens its capture only after the foreground probe answered (§18.4), so chunks may
//! arrive before [`AudioSource::start`]: they are kept, and replayed into the capture's callbacks
//! and live tap when it opens.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use voltip_protocol::app::TAKE_SAMPLE_RATE_HZ;

use super::ports::{AudioSource, Capture, CaptureOptions, DictationError, LevelFrame, LivePcm, Recording};
use super::wav;

/// Samples per level frame: ≈ 33 ms at 16 kHz, the ≈ 30 Hz cadence of a local capture.
const LEVEL_WINDOW: usize = 533;
/// Seconds of audio the live tap holds before it reports an overrun (the decode thread fell behind).
const LIVE_CAPACITY_SECS: usize = 10;
/// The level of silence (`LevelFrame.rms_dbfs` floor).
const SILENCE_DBFS: f32 = -90.0;

type LevelSink = Box<dyn Fn(LevelFrame) + Send>;
type ReadySink = Box<dyn FnOnce() + Send>;

/// The runtime's handle on one phone take's audio. Cheap to clone.
#[derive(Clone)]
pub struct RemoteFeed {
    shared: Arc<Mutex<Feed>>,
}

struct Feed {
    samples: Vec<i16>,
    /// `options.max_duration` of the open capture, in samples; nothing past it is kept.
    max_samples: usize,
    /// The next chunk expected; an older one (a duplicate from a second path) is dropped.
    next_seq: u32,
    on_level: Option<LevelSink>,
    on_ready: Option<ReadySink>,
    live: Option<Arc<Mutex<LiveBuffer>>>,
    level_seq: u64,
    /// The engine opened its capture on this feed.
    opened: bool,
    /// The capture stopped: later chunks are dropped.
    closed: bool,
    last_chunk: Instant,
}

struct LiveBuffer {
    queue: VecDeque<f32>,
    capacity: usize,
    overrun: bool,
    closed: bool,
}

impl Default for RemoteFeed {
    fn default() -> Self {
        Self::new()
    }
}

impl RemoteFeed {
    /// An empty feed for a take that is about to start.
    pub fn new() -> Self {
        Self {
            shared: Arc::new(Mutex::new(Feed {
                samples: Vec::new(),
                max_samples: sample_count(super::MAX_RECORDING),
                next_seq: 0,
                on_level: None,
                on_ready: None,
                live: None,
                level_seq: 0,
                opened: false,
                closed: false,
                last_chunk: Instant::now(),
            })),
        }
    }

    /// The [`AudioSource`] the engine opens for this take.
    pub fn source(&self) -> Arc<dyn AudioSource> {
        Arc::new(RemoteSource { feed: self.clone() })
    }

    /// Append chunk `seq` (PCM16 little-endian mono). `false` when it was dropped: a duplicate or
    /// older chunk, an odd byte count, or a capture that already stopped. A gap (a lost chunk) is
    /// accepted — the audio simply continues.
    pub fn push(&self, seq: u32, pcm: &[u8]) -> bool {
        let mut feed = self.shared.lock();
        if feed.closed || seq < feed.next_seq || !pcm.len().is_multiple_of(2) {
            return false;
        }
        if seq > feed.next_seq {
            tracing::debug!(expected = feed.next_seq, got = seq, "phone audio skipped chunks");
        }
        feed.next_seq = seq.saturating_add(1);
        feed.last_chunk = Instant::now();
        let room = feed.max_samples.saturating_sub(feed.samples.len());
        let (frames, _) = pcm.as_chunks::<2>();
        let chunk: Vec<i16> = frames.iter().take(room).map(|b| i16::from_le_bytes(*b)).collect();
        if chunk.is_empty() {
            return true;
        }
        feed.samples.extend_from_slice(&chunk);
        if feed.opened {
            feed.deliver(&chunk);
        }
        true
    }

    /// Audio received so far.
    pub fn received(&self) -> Duration {
        Duration::from_millis(samples_ms(self.shared.lock().samples.len()))
    }

    /// Time since the last chunk (or since the feed was made).
    pub fn quiet_for(&self) -> Duration {
        self.shared.lock().last_chunk.elapsed()
    }
}

impl Feed {
    /// Hand freshly kept samples to the open capture: level frames, `ready`, the live tap.
    fn deliver(&mut self, chunk: &[i16]) {
        if let Some(ready) = self.on_ready.take() {
            ready();
        }
        if let Some(on_level) = &self.on_level {
            for window in chunk.chunks(LEVEL_WINDOW) {
                on_level(level_of(window, self.level_seq));
                self.level_seq += 1;
            }
        }
        if let Some(live) = &self.live {
            live.lock().push(chunk);
        }
    }
}

impl LiveBuffer {
    fn push(&mut self, chunk: &[i16]) {
        for &s in chunk {
            if self.queue.len() >= self.capacity {
                self.overrun = true;
                return;
            }
            self.queue.push_back(f32::from(s) / 32_768.0);
        }
    }
}

struct RemoteSource {
    feed: RemoteFeed,
}

impl AudioSource for RemoteSource {
    /// Attach the capture to the feed; `device_id` means nothing here. Samples that arrived before
    /// (while the foreground probe ran) are replayed at once.
    fn start(&self, _device_id: Option<&str>, on_level: LevelSink, on_ready: ReadySink, options: CaptureOptions) -> Result<Box<dyn Capture>, DictationError> {
        let mut feed = self.feed.shared.lock();
        if feed.opened {
            return Err(DictationError::Audio("phone audio: this take's capture is already open".into()));
        }
        feed.opened = true;
        let max_samples = sample_count(options.max_duration);
        feed.max_samples = max_samples;
        feed.samples.truncate(max_samples);
        feed.on_level = Some(on_level);
        feed.on_ready = Some(on_ready);
        let live = options.live.then(|| {
            Arc::new(Mutex::new(LiveBuffer {
                queue: VecDeque::new(),
                capacity: LIVE_CAPACITY_SECS * TAKE_SAMPLE_RATE_HZ as usize,
                overrun: false,
                closed: false,
            }))
        });
        feed.live = live.clone();
        let early = std::mem::take(&mut feed.samples);
        if !early.is_empty() {
            feed.deliver(&early);
        }
        feed.samples = early;
        Ok(Box::new(RemoteCapture { feed: self.feed.clone(), live: live.map(|buffer| Box::new(RemoteLive(buffer)) as Box<dyn LivePcm>) }))
    }
}

struct RemoteCapture {
    feed: RemoteFeed,
    live: Option<Box<dyn LivePcm>>,
}

impl Capture for RemoteCapture {
    fn stop(self: Box<Self>) -> Result<Recording, DictationError> {
        let mut feed = self.feed.shared.lock();
        feed.closed = true;
        if let Some(live) = &feed.live {
            live.lock().closed = true;
        }
        let samples = std::mem::take(&mut feed.samples);
        Ok(Recording { wav: wav::encode_pcm16(&samples, TAKE_SAMPLE_RATE_HZ), duration_ms: samples_ms(samples.len()), sample_rate_hz: TAKE_SAMPLE_RATE_HZ })
    }

    fn live_pcm(&mut self) -> Option<Box<dyn LivePcm>> {
        self.live.take()
    }
}

impl Drop for RemoteCapture {
    /// A capture dropped without `stop` (a cancelled take) closes the feed as well.
    fn drop(&mut self) {
        let mut feed = self.feed.shared.lock();
        feed.closed = true;
        if let Some(live) = &feed.live {
            live.lock().closed = true;
        }
    }
}

struct RemoteLive(Arc<Mutex<LiveBuffer>>);

impl LivePcm for RemoteLive {
    fn read(&mut self, out: &mut [f32]) -> usize {
        let mut buffer = self.0.lock();
        let n = out.len().min(buffer.queue.len());
        for (slot, sample) in out.iter_mut().zip(buffer.queue.drain(..n)) {
            *slot = sample;
        }
        n
    }

    fn overrun(&self) -> bool {
        self.0.lock().overrun
    }

    fn is_closed(&self) -> bool {
        self.0.lock().closed
    }
}

fn sample_count(duration: Duration) -> usize {
    usize::try_from(duration.as_millis().saturating_mul(u128::from(TAKE_SAMPLE_RATE_HZ)) / 1000).unwrap_or(usize::MAX)
}

fn samples_ms(samples: usize) -> u64 {
    samples as u64 * 1000 / u64::from(TAKE_SAMPLE_RATE_HZ)
}

fn dbfs(amplitude: f32) -> f32 {
    if amplitude <= 0.0 { SILENCE_DBFS } else { (20.0 * amplitude.log10()).max(SILENCE_DBFS) }
}

/// RMS / peak of one window, as a local capture would report it.
fn level_of(window: &[i16], seq: u64) -> LevelFrame {
    let mut sum = 0.0f64;
    let mut peak = 0.0f32;
    let mut clipping = false;
    for &s in window {
        let v = f32::from(s) / 32_768.0;
        sum += f64::from(v * v);
        peak = peak.max(v.abs());
        clipping |= s == i16::MAX || s == i16::MIN;
    }
    let rms = if window.is_empty() { 0.0 } else { (sum / window.len() as f64).sqrt() as f32 };
    LevelFrame { rms_dbfs: dbfs(rms), peak_dbfs: dbfs(peak), clipping, sample_rate_hz: TAKE_SAMPLE_RATE_HZ, channels: 1, seq }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    use super::*;

    fn pcm(samples: &[i16]) -> Vec<u8> {
        samples.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    fn tone(n: usize, amplitude: i16) -> Vec<i16> {
        (0..n).map(|i| if i % 2 == 0 { amplitude } else { -amplitude }).collect()
    }

    /// docs/dictation.md §20: what the phone streams is the take — levels, `ready`, the live tap and
    /// the WAV all come from it, early chunks (before the capture opened) included, in order.
    #[test]
    fn a_feed_is_the_takes_microphone() {
        let feed = RemoteFeed::new();
        // The probe is still running: the first chunk arrives before the capture opens.
        assert!(feed.push(0, &pcm(&tone(1600, 8000))));
        let ready = Arc::new(AtomicBool::new(false));
        let frames = Arc::new(AtomicU64::new(0));
        let (r, f) = (ready.clone(), frames.clone());
        let mut capture = feed
            .source()
            .start(
                None,
                Box::new(move |frame| {
                    assert_eq!((frame.sample_rate_hz, frame.channels), (TAKE_SAMPLE_RATE_HZ, 1));
                    assert!(frame.rms_dbfs > -30.0, "{frame:?}");
                    f.fetch_add(1, Ordering::SeqCst);
                }),
                Box::new(move || r.store(true, Ordering::SeqCst)),
                CaptureOptions::LIVE,
            )
            .unwrap();
        assert!(ready.load(Ordering::SeqCst), "the early audio makes the capture ready at once");
        assert_eq!(frames.load(Ordering::SeqCst), 4, "1600 samples in ≈ 33 ms windows");
        let mut live = capture.live_pcm().unwrap();
        assert!(capture.live_pcm().is_none(), "take-once");
        assert!(feed.push(1, &pcm(&tone(800, 8000))));
        // A duplicate of chunk 1 (from a second path), an old chunk and an odd payload are dropped.
        assert!(!feed.push(1, &pcm(&tone(800, 8000))));
        assert!(!feed.push(0, &pcm(&tone(800, 8000))));
        assert!(!feed.push(2, &[0, 1, 2]));
        // A gap is accepted.
        assert!(feed.push(5, &pcm(&tone(1600, 8000))));
        assert_eq!(feed.received(), Duration::from_millis(250));
        let mut out = vec![0.0f32; 8000];
        assert_eq!(live.read(&mut out), 4000);
        assert!((out[0] - 8000.0 / 32_768.0).abs() < 1e-6);
        assert!(!live.is_closed() && !live.overrun());
        let recording = capture.stop().unwrap();
        assert!(live.is_closed());
        assert_eq!((recording.duration_ms, recording.sample_rate_hz), (250, TAKE_SAMPLE_RATE_HZ));
        assert_eq!(wav::pcm_data(&recording.wav).map(<[u8]>::len), Some(4000 * 2));
        assert!(!feed.push(6, &pcm(&tone(10, 1))), "nothing is kept after the stop");
    }

    #[test]
    fn a_feed_keeps_at_most_the_captures_cap_and_opens_once() {
        let feed = RemoteFeed::new();
        let source = feed.source();
        let options = CaptureOptions { live: false, max_duration: Duration::from_millis(100), ..CaptureOptions::default() };
        let capture = source.start(None, Box::new(|_| {}), Box::new(|| {}), options.clone()).unwrap();
        assert!(matches!(source.start(None, Box::new(|_| {}), Box::new(|| {}), options), Err(DictationError::Audio(_))));
        assert!(feed.push(0, &pcm(&tone(3200, 100))));
        assert_eq!(feed.received(), Duration::from_millis(100), "cut at max_duration");
        let recording = capture.stop().unwrap();
        assert_eq!(recording.duration_ms, 100);
        // A silent window reads as the floor, a full-scale one clips.
        assert_eq!(level_of(&[0; 16], 0).rms_dbfs, SILENCE_DBFS);
        assert!(level_of(&[i16::MAX, i16::MIN], 0).clipping);
        assert!(feed.quiet_for() < Duration::from_secs(5));
    }

    #[test]
    fn a_dropped_capture_closes_the_feed_and_its_tap() {
        let feed = RemoteFeed::new();
        let mut capture = feed.source().start(None, Box::new(|_| {}), Box::new(|| {}), CaptureOptions::LIVE).unwrap();
        let live = capture.live_pcm().unwrap();
        drop(capture);
        assert!(live.is_closed());
        assert!(!feed.push(0, &pcm(&tone(10, 1))));
    }
}
