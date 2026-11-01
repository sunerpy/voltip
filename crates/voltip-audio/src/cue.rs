//! The 「可以说话」 cue (docs/dictation.md §30.3): a 120 ms two-tone on the default output once the
//! microphone delivers its first samples, kept out of what is recognised.
//!
//! Everything here runs inside the two audio callbacks, so it only reads and writes atomics and
//! the buffers it is handed; the tone is synthesised before the output stream starts. The input
//! callback marks the first audio ([`CueShared::arm`]) and reports its latency; the output callback
//! that writes the first block of the tone works out when the tone sounds and publishes that span,
//! widened a little, as the window the input callback zeroes ([`zero_range`]) before the samples
//! reach the recording, the live tap and the long take's stream. Only differences within one
//! stream's own timestamps are used (cpal says different streams may count from different
//! origins), mapped onto this process's monotonic clock where each callback ran.

use std::ops::Range;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU8, Ordering};
use std::time::{Duration, Instant};

/// How long the tone sounds.
pub const CUE_DURATION: Duration = Duration::from_millis(120);
/// The two tones, one after the other, each half of [`CUE_DURATION`] (rising: ready).
pub const CUE_TONES_HZ: [f32; 2] = [880.0, 1320.0];
/// Peak amplitude of the tone (full scale is 1.0).
pub const CUE_AMPLITUDE: f32 = 0.2;
/// Each tone fades in and out over this long, so it starts and ends without a click.
const CUE_FADE: Duration = Duration::from_millis(5);
/// The window opens at most this long before the tone starts (`pre`, bounded by the latencies).
pub const CUE_PRE_MAX: Duration = Duration::from_millis(10);
/// The window stays open this long after the tone ends (its echo in the room).
pub const CUE_TAIL: Duration = Duration::from_millis(30);
/// The window when the host reports no usable input latency, from when the first block of the tone
/// plays, compared against the input callbacks' own times.
pub const CUE_FALLBACK_WINDOW: Duration = Duration::from_millis(250);
/// An input latency above this is not believed. ALSA's PulseAudio plugin reports 686 ms when a
/// stream starts, falling to its real 10 ms over the next 1.3 s while the samples keep arriving in
/// real time (Linux, PulseAudio 16, 2026-11-01): capture times from it would put the cue before
/// its window.
pub const CUE_MAX_INPUT_LATENCY: Duration = Duration::from_millis(100);

/// How the samples of a chunk are placed in time against a published window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowClock {
    /// Frame `i` was captured at the chunk's capture time plus `i` frames: both latencies known.
    Capture,
    /// The host reported no latency: a chunk counts as a whole, at the moment its callback ran.
    Callback,
}

/// The span to keep out of the take, in nanoseconds of [`CueShared`]'s clock, both ends included.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CueWindow {
    /// First instant to zero.
    pub start_ns: i64,
    /// Last instant to zero.
    pub end_ns: i64,
    /// What a chunk's time is.
    pub clock: WindowClock,
}

/// A usable latency from one stream's own two timestamps: `later − earlier`, `None` when the host
/// reports the same instant for both (or the later one first), which is what a host without a
/// latency report does.
pub fn latency_between(later: Duration, earlier: Duration) -> Option<Duration> {
    later.checked_sub(earlier).filter(|d| !d.is_zero())
}

/// The window for a tone whose first block is written by the output callback that ran at
/// `callback_ns`: with both latencies, the tone sounds from `callback + output latency` for
/// [`CUE_DURATION`], and the window is that span opened `pre = min(10 ms, output + input latency)`
/// earlier and [`CUE_TAIL`] later. A sample of that span reaches the input callback no earlier than
/// `start − pre + input latency`, which is never before this callback publishes it. Without the
/// input's latency, [`CUE_FALLBACK_WINDOW`] from when the tone starts to play (the callback, plus
/// the output latency when known), by the input callbacks' own times: a sample reaches its
/// callback no earlier than it was played.
pub fn window_for(callback_ns: i64, output_latency: Option<Duration>, input_latency: Option<Duration>) -> CueWindow {
    let start = callback_ns.saturating_add(output_latency.map_or(0, nanos));
    match (output_latency, input_latency) {
        (Some(output), Some(input)) => {
            let pre = CUE_PRE_MAX.min(output + input);
            CueWindow { start_ns: start.saturating_sub(nanos(pre)), end_ns: start.saturating_add(nanos(CUE_DURATION + CUE_TAIL)), clock: WindowClock::Capture }
        }
        _ => CueWindow { start_ns: start, end_ns: start.saturating_add(nanos(CUE_FALLBACK_WINDOW)), clock: WindowClock::Callback },
    }
}

/// The frames of a chunk of `frames` frames at `rate` that fall into `window`. `capture_ns` is when
/// its first frame was captured (`None`: unknown), `callback_ns` when its callback ran. A window by
/// capture time places frame `i` at `capture_ns + i / rate`; one by callback time takes the chunk
/// whole when the callback ran inside it. A capture-time window and a chunk without a capture time
/// (its latency went missing) fall back to the callback time too.
pub fn zero_range(window: CueWindow, capture_ns: Option<i64>, callback_ns: i64, frames: usize, rate: u32) -> Range<usize> {
    let whole = |at: i64| if (window.start_ns..=window.end_ns).contains(&at) { 0..frames } else { 0..0 };
    let (WindowClock::Capture, Some(first)) = (window.clock, capture_ns) else { return whole(callback_ns) };
    if rate == 0 || frames == 0 {
        return 0..0;
    }
    let rate = i128::from(rate);
    // Frame `i` is at `first + i * 1e9 / rate`: the first one at or after `start`, and the first
    // one after `end`.
    let from = |t: i64| i128::from(t) - i128::from(first);
    let first_in = {
        let d = from(window.start_ns);
        if d <= 0 { 0 } else { (d * rate + 999_999_999) / 1_000_000_000 }
    };
    let first_after = {
        let d = from(window.end_ns);
        if d < 0 { 0 } else { d * rate / 1_000_000_000 + 1 }
    };
    let clamp = |v: i128| usize::try_from(v.clamp(0, frames as i128)).unwrap_or(frames);
    let (a, b) = (clamp(first_in), clamp(first_after));
    if a < b { a..b } else { 0..0 }
}

/// `d` in nanoseconds, saturating.
fn nanos(d: Duration) -> i64 {
    i64::try_from(d.as_nanos()).unwrap_or(i64::MAX)
}

/// No latency known (an `AtomicI64` cannot hold `Option`).
const UNKNOWN: i64 = -1;
const MODE_NONE: u8 = 0;
const MODE_CAPTURE: u8 = 1;
const MODE_CALLBACK: u8 = 2;

/// What the two callbacks of one take share: atomics only.
#[derive(Debug)]
pub struct CueShared {
    /// The clock origin: every time below is nanoseconds since this instant (negative before it).
    base: Instant,
    /// The microphone delivered its first samples: the output may play the tone.
    armed: AtomicBool,
    /// The input stream's latest `callback − capture`, nanoseconds, or [`UNKNOWN`].
    input_latency_ns: AtomicI64,
    window_start_ns: AtomicI64,
    window_end_ns: AtomicI64,
    /// Written last, with release: the window is complete once this is not [`MODE_NONE`].
    window_mode: AtomicU8,
}

impl CueShared {
    /// A take's shared state, counting from now.
    pub fn new() -> Self {
        Self::with_base(Instant::now())
    }

    /// Counting from `base` (tests).
    pub fn with_base(base: Instant) -> Self {
        Self {
            base,
            armed: AtomicBool::new(false),
            input_latency_ns: AtomicI64::new(UNKNOWN),
            window_start_ns: AtomicI64::new(0),
            window_end_ns: AtomicI64::new(0),
            window_mode: AtomicU8::new(MODE_NONE),
        }
    }

    /// `at` on this take's clock.
    pub fn at(&self, at: Instant) -> i64 {
        match at.checked_duration_since(self.base) {
            Some(after) => nanos(after),
            None => self.base.checked_duration_since(at).map_or(0, |before| -nanos(before)),
        }
    }

    /// The input callback: the first samples are here.
    pub fn arm(&self) {
        self.armed.store(true, Ordering::Release);
    }

    /// Whether the first samples arrived.
    pub fn is_armed(&self) -> bool {
        self.armed.load(Ordering::Acquire)
    }

    /// The input callback: its stream's latest latency; one above [`CUE_MAX_INPUT_LATENCY`] counts
    /// as unknown.
    pub fn set_input_latency(&self, latency: Option<Duration>) {
        let believed = latency.filter(|l| *l <= CUE_MAX_INPUT_LATENCY);
        self.input_latency_ns.store(believed.map_or(UNKNOWN, nanos), Ordering::Relaxed);
    }

    fn input_latency(&self) -> Option<Duration> {
        let ns = self.input_latency_ns.load(Ordering::Relaxed);
        u64::try_from(ns).ok().map(Duration::from_nanos)
    }

    /// The output callback: publish `window` (once).
    pub fn publish(&self, window: CueWindow) {
        self.window_start_ns.store(window.start_ns, Ordering::Relaxed);
        self.window_end_ns.store(window.end_ns, Ordering::Relaxed);
        let mode = match window.clock {
            WindowClock::Capture => MODE_CAPTURE,
            WindowClock::Callback => MODE_CALLBACK,
        };
        self.window_mode.store(mode, Ordering::Release);
    }

    /// `ns` on this take's clock as an [`Instant`].
    pub fn instant(&self, ns: i64) -> Instant {
        match u64::try_from(ns) {
            Ok(after) => self.base + Duration::from_nanos(after),
            Err(_) => self.base.checked_sub(Duration::from_nanos(ns.unsigned_abs())).unwrap_or(self.base),
        }
    }

    /// The published window, if any.
    pub fn window(&self) -> Option<CueWindow> {
        let clock = match self.window_mode.load(Ordering::Acquire) {
            MODE_CAPTURE => WindowClock::Capture,
            MODE_CALLBACK => WindowClock::Callback,
            _ => return None,
        };
        Some(CueWindow { start_ns: self.window_start_ns.load(Ordering::Relaxed), end_ns: self.window_end_ns.load(Ordering::Relaxed), clock })
    }
}

impl Default for CueShared {
    fn default() -> Self {
        Self::new()
    }
}

/// The tone at `rate` Hz, mono: [`CUE_TONES_HZ`] one after the other, each faded in and out.
pub fn cue_tone(rate: u32) -> Vec<f32> {
    let rate_f = rate.max(1) as f32;
    let total = usize::try_from(CUE_DURATION.as_millis() * u128::from(rate) / 1000).unwrap_or(0);
    let half = total / 2;
    let fade = usize::try_from(CUE_FADE.as_millis() * u128::from(rate) / 1000).unwrap_or(0).max(1);
    (0..total)
        .map(|i| {
            let (tone, j, len) = if i < half { (CUE_TONES_HZ[0], i, half) } else { (CUE_TONES_HZ[1], i - half, total - half) };
            let edge = j.min(len.saturating_sub(1 + j));
            let gain = if edge < fade { 0.5 - 0.5 * (std::f32::consts::PI * edge as f32 / fade as f32).cos() } else { 1.0 };
            CUE_AMPLITUDE * gain * (2.0 * std::f32::consts::PI * tone * j as f32 / rate_f).sin()
        })
        .collect()
}

/// Where the output stream is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Silence until the microphone's first samples.
    Waiting,
    /// The tone, from this frame of it.
    Playing(usize),
    /// Silence until the take ends.
    Done,
}

/// The output callback's side: silence, the tone once the microphone is ready, silence again.
pub struct CueWriter {
    shared: std::sync::Arc<CueShared>,
    tone: Vec<f32>,
    phase: Phase,
}

impl CueWriter {
    /// For an output stream at `rate` Hz (the tone is synthesised here, before the stream runs).
    pub fn new(shared: std::sync::Arc<CueShared>, rate: u32) -> Self {
        Self { shared, tone: cue_tone(rate), phase: Phase::Waiting }
    }

    /// Start one output buffer: the callback ran at `now`, its first frame plays `latency` later
    /// (`None`: the host did not say). The buffer that starts the tone publishes the window.
    pub fn begin(&mut self, now: Instant, latency: Option<Duration>) {
        if self.phase == Phase::Waiting && self.shared.is_armed() {
            self.shared.publish(window_for(self.shared.at(now), latency, self.shared.input_latency()));
            self.phase = Phase::Playing(0);
        }
    }

    /// The next frame's value, the same on every channel.
    pub fn next_frame(&mut self) -> f32 {
        match self.phase {
            Phase::Playing(i) => {
                let v = self.tone.get(i).copied().unwrap_or(0.0);
                self.phase = if i + 1 >= self.tone.len() { Phase::Done } else { Phase::Playing(i + 1) };
                v
            }
            Phase::Waiting | Phase::Done => 0.0,
        }
    }

    /// Fill an interleaved buffer of `channels` channels, frame by frame.
    pub fn fill(&mut self, out: &mut [f32], channels: u16) {
        for frame in out.chunks_mut(usize::from(channels.max(1))) {
            frame.fill(self.next_frame());
        }
    }

    /// Whether the tone has been written to the end.
    pub fn is_done(&self) -> bool {
        self.phase == Phase::Done
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::sync::Arc;

    use super::*;

    const MS: i64 = 1_000_000;

    #[test]
    fn latencies_come_from_one_streams_own_timestamps() {
        assert_eq!(latency_between(Duration::from_millis(30), Duration::from_millis(10)), Some(Duration::from_millis(20)));
        assert_eq!(latency_between(Duration::from_millis(10), Duration::from_millis(10)), None, "the same instant: not reported");
        assert_eq!(latency_between(Duration::from_millis(5), Duration::from_millis(10)), None, "backwards: not usable");
    }

    #[test]
    fn the_window_covers_the_tone_from_both_latencies_or_falls_back() {
        let ms = Duration::from_millis;
        // Output 20 ms, input 10 ms: the tone sounds 20..140 ms after the callback; pre = 10 ms.
        let w = window_for(1_000 * MS, Some(ms(20)), Some(ms(10)));
        assert_eq!(w, CueWindow { start_ns: 1_010 * MS, end_ns: 1_170 * MS, clock: WindowClock::Capture });
        // Short latencies bound `pre`: 2 + 3 ms.
        let w = window_for(0, Some(ms(2)), Some(ms(3)));
        assert_eq!((w.start_ns, w.end_ns), (-3 * MS, 152 * MS));
        // No input latency: the fixed window from when the tone plays (the callback, plus the output
        // latency when known), by callback time.
        for (out, inp, start) in [(None, Some(ms(10)), 500), (None, None, 500), (Some(ms(20)), None, 520)] {
            assert_eq!(window_for(500 * MS, out, inp), CueWindow { start_ns: start * MS, end_ns: (start + 250) * MS, clock: WindowClock::Callback });
        }
    }

    /// The zeroed frames of a 10 ms chunk at 48 kHz (480 frames, 20 833 ns each) against the window
    /// 1 010..=1 170 ms.
    #[test]
    fn the_frames_to_zero_follow_their_capture_times() {
        let window = CueWindow { start_ns: 1_010 * MS, end_ns: 1_170 * MS, clock: WindowClock::Capture };
        let cases: [(i64, std::ops::Range<usize>); 7] = [
            // Wholly before, wholly inside, wholly after.
            (990 * MS, 0..0),
            (1_050 * MS, 0..480),
            (1_175 * MS, 0..0),
            // Starts 5 ms before the window: frames from 5 ms in (240) on.
            (1_005 * MS, 240..480),
            // Exactly at the start.
            (1_010 * MS, 0..480),
            // Ends inside the tail: 5 ms in (240 frames) are still inside, the end itself (frame 240
            // at exactly 1 170 ms) included.
            (1_165 * MS, 0..241),
            // The last frame lands exactly on the start.
            (1_010 * MS - 479 * 1_000_000_000 / 48_000, 479..480),
        ];
        for (capture, expected) in cases {
            assert_eq!(zero_range(window, Some(capture), capture + 10 * MS, 480, 48_000), expected, "chunk captured at {capture}");
        }
        // No capture time for this chunk: its callback decides, whole.
        assert_eq!(zero_range(window, None, 1_100 * MS, 480, 48_000), 0..480);
        assert_eq!(zero_range(window, None, 1_200 * MS, 480, 48_000), 0..0);
        assert_eq!(zero_range(window, Some(1_050 * MS), 0, 0, 48_000), 0..0, "an empty chunk");
    }

    #[test]
    fn a_fallback_window_takes_chunks_whole_by_their_callback_time() {
        let window = CueWindow { start_ns: 500 * MS, end_ns: 750 * MS, clock: WindowClock::Callback };
        assert_eq!(zero_range(window, Some(400 * MS), 499 * MS, 480, 48_000), 0..0);
        assert_eq!(zero_range(window, Some(400 * MS), 500 * MS, 480, 48_000), 0..480, "the capture time is not used");
        assert_eq!(zero_range(window, None, 750 * MS, 480, 48_000), 0..480);
        assert_eq!(zero_range(window, None, 751 * MS, 480, 48_000), 0..0);
    }

    #[test]
    fn the_tone_is_two_faded_pitches_of_120_ms() {
        let tone = cue_tone(48_000);
        assert_eq!(tone.len(), 5_760);
        assert!(tone.iter().all(|v| v.abs() <= CUE_AMPLITUDE + f32::EPSILON));
        assert!(tone[0].abs() < 1e-3 && tone[tone.len() - 1].abs() < 0.01, "faded in and out");
        let peak = |part: &[f32]| part.iter().fold(0.0_f32, |m, v| m.max(v.abs()));
        assert!(peak(&tone[..2_880]) > CUE_AMPLITUDE * 0.95 && peak(&tone[2_880..]) > CUE_AMPLITUDE * 0.95);
        // Zero crossings give the pitch: 880 Hz → ~105 in 60 ms, 1 320 Hz → ~158.
        let crossings = |part: &[f32]| part.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
        assert!((100..=110).contains(&crossings(&tone[..2_880])), "{}", crossings(&tone[..2_880]));
        assert!((152..=164).contains(&crossings(&tone[2_880..])), "{}", crossings(&tone[2_880..]));
        assert!(cue_tone(0).is_empty());
    }

    /// The writer stays silent until the first samples, publishes the window with the buffer that
    /// starts the tone, writes the tone once, then silence.
    #[test]
    fn the_writer_waits_plays_once_and_publishes_the_window() {
        let base = Instant::now();
        let shared = Arc::new(CueShared::with_base(base));
        let mut writer = CueWriter::new(Arc::clone(&shared), 48_000);
        let mut buffer = vec![1.0_f32; 960];
        writer.begin(base + Duration::from_millis(5), Some(Duration::from_millis(20)));
        writer.fill(&mut buffer, 2);
        assert!(buffer.iter().all(|v| *v == 0.0), "silence before the first samples");
        assert_eq!(shared.window(), None);

        shared.set_input_latency(Some(Duration::from_millis(10)));
        shared.arm();
        let mut played = Vec::new();
        for n in 0..20 {
            writer.begin(base + Duration::from_millis(15 + 10 * n), Some(Duration::from_millis(20)));
            writer.fill(&mut buffer, 2);
            played.extend(buffer.chunks(2).map(|frame| {
                assert_eq!(frame[0], frame[1], "every channel alike");
                frame[0]
            }));
        }
        assert!(writer.is_done());
        assert_eq!(shared.window(), Some(CueWindow { start_ns: 25 * MS, end_ns: 185 * MS, clock: WindowClock::Capture }));
        let tone = cue_tone(48_000);
        assert_eq!(&played[..tone.len()], tone.as_slice(), "the tone from the first frame of that buffer");
        assert!(played[tone.len()..].iter().all(|v| *v == 0.0), "then silence");
    }

    #[test]
    fn the_clock_counts_both_ways_from_its_base() {
        let base = Instant::now() + Duration::from_secs(1);
        let shared = CueShared::with_base(base);
        assert_eq!(shared.at(base + Duration::from_millis(3)), 3 * MS);
        assert_eq!(shared.at(base - Duration::from_millis(3)), -3 * MS);
        assert_eq!(shared.instant(3 * MS), base + Duration::from_millis(3));
        assert_eq!(shared.instant(-3 * MS), base - Duration::from_millis(3));
        shared.set_input_latency(None);
        assert_eq!(shared.input_latency(), None);
        shared.set_input_latency(Some(Duration::from_micros(1500)));
        assert_eq!(shared.input_latency(), Some(Duration::from_micros(1500)));
        // ALSA's PulseAudio plugin when a stream starts: not believed.
        shared.set_input_latency(Some(Duration::from_millis(686)));
        assert_eq!(shared.input_latency(), None);
        shared.set_input_latency(Some(CUE_MAX_INPUT_LATENCY));
        assert_eq!(shared.input_latency(), Some(CUE_MAX_INPUT_LATENCY));
    }
}
