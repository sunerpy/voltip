//! Mixing the microphone with the computer's sound (docs/dictation.md §22, `mixed`).
//!
//! Both sides are resampled to 16 kHz mono first, each on its own audio thread. The computer's
//! sound waits in a lock-free ring; the microphone's stream is the clock: each microphone sample
//! takes the next queued sample of the other side (silence when none is queued — that side is
//! behind or has nothing to play). Before each microphone chunk, queued samples older than the
//! chunk plus [`MAX_LAG_SAMPLES`] are dropped — the computer's sound starts first and piles up
//! while the microphone opens, and a faster clock piles up more — so the two are never more than
//! 20 ms apart, from the first chunk on, whichever clock runs faster. A queue that fills up (the
//! microphone took nothing for a second) drops the newest of the computer's sound, so what it
//! still holds is older than that. A push that does not fit is dropped whole, and [`MixSender`]
//! reports it; the mixer then empties the queue before its next chunk, as it does a full queue.
//! The mixer reads the queue first and the report second, and reads the queue again when there is
//! a report. A push is published before a later drop is reported, so a first read that holds
//! anything pushed after a drop finds its report, and the second read holds everything pushed
//! before it. The sum leaves headroom ([`MIX_GAIN`] on each side) and goes through a soft limiter,
//! so a loud call over loud speech never wraps around.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Gain on each side before the sum: −3 dB, so two loud inputs rarely reach the limiter.
pub const MIX_GAIN: f32 = 0.707;
/// Where the limiter starts to bend: below it the sum passes unchanged.
pub const LIMIT_KNEE: f32 = 0.9;
/// The most the computer's sound may run ahead of the microphone before the excess is dropped:
/// 20 ms at 16 kHz.
pub const MAX_LAG_SAMPLES: usize = 320;
/// Samples the other side's scratch buffer holds per step; a longer microphone chunk is mixed in
/// several steps.
pub const MIX_STEP: usize = 1024;

/// The computer's sound on its way to a [`Mixer`], pushed from its own audio callback.
pub struct MixSender {
    queue: rtrb::Producer<f32>,
    /// Samples that found the queue full since the mixer last looked.
    overflowed: Arc<AtomicU64>,
}

impl std::fmt::Debug for MixSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MixSender").field("free", &self.queue.slots()).field("overflowed", &self.overflowed.load(Ordering::Relaxed)).finish()
    }
}

impl MixSender {
    /// Queue `samples` (16 kHz mono) whole, or drop and count them all when they do not fit.
    /// Never blocks.
    pub fn push(&mut self, samples: &[f32]) {
        self.push_with(samples, || {});
    }

    /// [`Self::push`], calling `between` once it has decided and before it publishes anything: a
    /// push that does not fit has reported the drop by then. The tests hold a push there.
    fn push_with(&mut self, samples: &[f32], between: impl FnOnce()) {
        // All or nothing. A part published after the report would be older than the dropped rest,
        // and a mixer that had already emptied the queue for the report would mix it.
        let fits = samples.len() <= self.queue.slots();
        if !fits {
            // Release: the samples published before are visible to a mixer that sees the report.
            self.overflowed.fetch_add(samples.len() as u64, Ordering::Release);
        }
        between();
        if fits {
            // Only the mixer frees slots, so the space found above is still free.
            let pushed = self.queue.push_entire_slice(samples);
            debug_assert!(pushed.is_ok());
        }
    }
}

/// A queue of `capacity` samples from the computer's sound to the mixer.
pub fn mix_queue(capacity: usize) -> (MixSender, Mixer) {
    let (queue, queued) = rtrb::RingBuffer::new(capacity);
    let overflowed = Arc::new(AtomicU64::new(0));
    (MixSender { queue, overflowed: overflowed.clone() }, Mixer::with_overflow(queued, overflowed))
}

/// Soft limiter: unchanged up to [`LIMIT_KNEE`], then bent smoothly towards full scale without
/// ever reaching past it.
pub fn limit(x: f32) -> f32 {
    let magnitude = x.abs();
    if magnitude <= LIMIT_KNEE || !x.is_finite() {
        return if x.is_finite() { x } else { 0.0 };
    }
    let over = (magnitude - LIMIT_KNEE) / (1.0 - LIMIT_KNEE);
    x.signum() * (LIMIT_KNEE + (1.0 - LIMIT_KNEE) * over.tanh())
}

/// Mixes the microphone (the clock) with the queued computer's sound.
pub struct Mixer {
    other: rtrb::Consumer<f32>,
    /// Set by the [`MixSender`] when the queue was full.
    overflowed: Arc<AtomicU64>,
    /// One step of the other side's samples; allocated once.
    scratch: Vec<f32>,
    /// Queued samples dropped because the other side ran ahead (for the log).
    dropped: u64,
    /// Microphone samples mixed with silence because nothing was queued (for the log).
    filled: u64,
}

impl std::fmt::Debug for Mixer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mixer").field("queued", &self.other.slots()).field("dropped", &self.dropped).field("filled", &self.filled).finish()
    }
}

impl Mixer {
    /// A mixer reading the other side from `other` (a queue that never reports an overflow; see
    /// [`mix_queue`] for one that does).
    pub fn new(other: rtrb::Consumer<f32>) -> Self {
        Self::with_overflow(other, Arc::new(AtomicU64::new(0)))
    }

    fn with_overflow(other: rtrb::Consumer<f32>, overflowed: Arc<AtomicU64>) -> Self {
        Self { other, overflowed, scratch: vec![0.0; MIX_STEP], dropped: 0, filled: 0 }
    }

    /// Mix one microphone chunk (16 kHz mono) into `out` (cleared first; `mic.len()` samples).
    /// Allocates only when `out` has less capacity than the chunk.
    pub fn mix(&mut self, mic: &[f32], out: &mut Vec<f32>) {
        self.mix_with(mic, out, || {});
    }

    /// [`Self::mix`], calling `between` after it has read the queue and before it reads the report:
    /// the tests push there.
    fn mix_with(&mut self, mic: &[f32], out: &mut Vec<f32>, between: impl FnOnce()) {
        out.clear();
        // The queue first: a push is published before a later drop is reported, so if this read
        // holds anything pushed after a drop, the report is there as well.
        let queued = self.other.slots();
        between();
        // A drop was reported, or the queue is full (its next push is dropped): the newest of the
        // computer's sound is gone, so all the queue holds is older than it. After a report the
        // queue is read again, which holds everything pushed before the drop.
        let excess = if self.overflowed.swap(0, Ordering::Acquire) > 0 {
            self.other.slots()
        } else if queued >= self.other.buffer().capacity() {
            queued
        } else {
            queued.saturating_sub(mic.len() + MAX_LAG_SAMPLES)
        };
        if excess > 0
            && let Ok(chunk) = self.other.read_chunk(excess)
        {
            chunk.commit_all();
            self.dropped += excess as u64;
        }
        for step in mic.chunks(MIX_STEP) {
            let (queued, _) = self.other.pop_partial_slice(&mut self.scratch[..step.len()]);
            let got = queued.len();
            self.filled += (step.len() - got) as u64;
            for (i, m) in step.iter().enumerate() {
                let o = if i < got { self.scratch[i] } else { 0.0 };
                out.push(limit(MIX_GAIN * m + MIX_GAIN * o));
            }
        }
    }

    /// Samples of the other side dropped so far because it ran ahead.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Microphone samples so far that had nothing of the other side to mix with.
    pub fn filled(&self) -> u64 {
        self.filled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn queue(capacity: usize) -> (rtrb::Producer<f32>, Mixer) {
        let (producer, consumer) = rtrb::RingBuffer::new(capacity);
        (producer, Mixer::new(consumer))
    }

    #[test]
    fn the_limiter_passes_the_normal_range_and_never_exceeds_full_scale() {
        for x in [-0.9, -0.5, 0.0, 0.3, 0.9] {
            assert_eq!(limit(x), x);
        }
        for x in [0.95f32, 1.2, 2.0, 10.0, 1e6] {
            let y = limit(x);
            assert!(y > LIMIT_KNEE && y <= 1.0, "never past full scale: {x} → {y}");
            assert_eq!(limit(-x), -y, "symmetric");
        }
        assert!(limit(1.2) < limit(2.0), "still monotonic above the knee");
        assert_eq!(limit(f32::NAN), 0.0);
        assert_eq!(limit(f32::INFINITY), 0.0);
    }

    /// The sum of both sides with −3 dB each; two full-scale inputs stay inside full scale.
    #[test]
    fn both_sides_are_summed_with_headroom() {
        let (mut other, mut mixer) = queue(4096);
        for s in [0.5f32, -0.5, 1.0, 0.0] {
            other.push(s).unwrap();
        }
        let mut out = Vec::new();
        mixer.mix(&[0.5, 0.5, 1.0, 0.0], &mut out);
        assert!((out[0] - 0.707).abs() < 1e-3, "{out:?}");
        assert!(out[1].abs() < 1e-6, "opposite phases cancel");
        assert!(out[2] > 0.9 && out[2] < 1.0, "full scale on both sides is limited, not wrapped: {}", out[2]);
        assert_eq!(out[3], 0.0);
        assert_eq!((mixer.dropped(), mixer.filled()), (0, 0));
    }

    /// Nothing queued (the other side is behind or quiet): the microphone goes through alone.
    #[test]
    fn silence_fills_in_when_the_other_side_is_behind() {
        let (mut other, mut mixer) = queue(4096);
        other.push(0.2).unwrap();
        let mut out = Vec::new();
        mixer.mix(&[0.4; 3], &mut out);
        assert_eq!(out.len(), 3);
        assert!((out[0] - MIX_GAIN * 0.6).abs() < 1e-6);
        assert!((out[1] - MIX_GAIN * 0.4).abs() < 1e-6 && (out[2] - out[1]).abs() < 1e-9);
        assert_eq!(mixer.filled(), 2);
    }

    /// The other side running ahead is cut back to 20 ms before every chunk: the two never drift
    /// apart by more.
    #[test]
    fn the_other_side_never_runs_more_than_20_ms_ahead() {
        let (mut other, mut mixer) = queue(16_000);
        for _ in 0..5000 {
            other.push(0.1).unwrap();
        }
        let mut out = Vec::new();
        mixer.mix(&[0.0; 160], &mut out);
        assert_eq!(out.len(), 160);
        assert_eq!(mixer.other.slots(), MAX_LAG_SAMPLES);
        assert_eq!(mixer.dropped(), (5000 - 160 - MAX_LAG_SAMPLES) as u64);
        // A long microphone chunk is mixed in steps and consumes the rest.
        let long = vec![0.0f32; MIX_STEP * 2 + 10];
        mixer.mix(&long, &mut out);
        assert_eq!(out.len(), long.len());
        assert_eq!(mixer.filled(), (long.len() - MAX_LAG_SAMPLES) as u64);
        assert!(format!("{mixer:?}").contains("dropped"));
    }

    /// Regression (goal review, 2026-09-30): the computer's sound starts before the microphone
    /// opens, and the first microphone chunk was mixed with the oldest of what had piled up by
    /// then — half a second before it here — the backlog going only afterwards. It goes first
    /// now: the chunk is mixed with sound at most 20 ms older than the newest queued.
    #[test]
    fn regression_the_first_chunk_is_mixed_with_recent_sound_not_the_oldest_queued() {
        let (mut other, mut mixer) = queue(16_000);
        // Half a second queued; each sample is its own index (÷ 10 000).
        for i in 0..8000u16 {
            other.push(f32::from(i) / 10_000.0).unwrap();
        }
        let mut out = Vec::new();
        mixer.mix(&[0.0; 160], &mut out);
        let index = |v: f32| v / MIX_GAIN * 10_000.0;
        let newest_kept = 8000.0 - 160.0 - MAX_LAG_SAMPLES as f32;
        assert!((index(out[0]) - newest_kept).abs() < 0.5, "mixed with sample {}", index(out[0]));
        assert!((index(out[159]) - (newest_kept + 159.0)).abs() < 0.5, "mixed with sample {}", index(out[159]));
        assert_eq!(mixer.dropped(), (8000 - 160 - MAX_LAG_SAMPLES) as u64);
    }

    /// Regression (goal review round 2, 2026-09-30): the microphone opened more than the queue's
    /// second after the computer's sound, the queue filled, and its newest samples were dropped
    /// while the oldest stayed; the first chunk was then mixed with sound from before those. A
    /// queue that overflowed is emptied before the next chunk, which mixes with what comes after.
    #[test]
    fn regression_after_the_queue_overflowed_no_stale_sound_is_mixed_in() {
        let (mut other, mut mixer) = mix_queue(1000);
        // 1500 samples before the microphone takes any: 0..1000 stay queued, and the push of
        // 1000..1500 finds the queue full and is dropped.
        let early: Vec<f32> = (0..1500u16).map(|i| f32::from(i) / 10_000.0).collect();
        other.push(&early[..1000]);
        other.push(&early[1000..]);
        assert!(format!("{other:?}").contains("overflowed: 500"));
        let mut out = Vec::new();
        mixer.mix(&[0.0; 160], &mut out);
        assert!(out.iter().all(|&s| s == 0.0), "stale sound mixed in: {:?}", &out[..4]);
        assert_eq!(mixer.dropped(), 1000);
        // What arrives after the overflow mixes in as usual.
        other.push(&[0.5; 160]);
        mixer.mix(&[0.0; 160], &mut out);
        assert!(out.iter().all(|&s| (s - MIX_GAIN * 0.5).abs() < 1e-6), "{:?}", &out[..4]);
        assert_eq!(mixer.dropped(), 1000);
    }

    /// Samples 0.0000, 0.0001, …: a mixed value tells which one it was.
    fn early(n: u16) -> Vec<f32> {
        (0..n).map(|i| f32::from(i) / 10_000.0).collect()
    }

    /// Runs `in_between` on this thread while a push of `samples` on another thread waits after it
    /// has decided and before it publishes anything (a push that does not fit has reported its drop
    /// by then).
    fn during_push(other: MixSender, samples: Vec<f32>, in_between: impl FnOnce()) {
        let (at_tx, at_rx) = std::sync::mpsc::channel();
        let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
        let pusher = std::thread::spawn(move || {
            let mut other = other;
            other.push_with(&samples, || {
                at_tx.send(()).expect("the test thread waits");
                go_rx.recv_timeout(Duration::from_secs(5)).expect("the test thread went on");
            });
        });
        at_rx.recv_timeout(Duration::from_secs(5)).expect("the push reached the point between its steps");
        in_between();
        go_tx.send(()).expect("the pusher waits");
        pusher.join().expect("the push finished");
    }

    /// Regression (goal review round 3, 2026-09-30): the two sides are pushed and mixed on their own
    /// audio threads, and a push used to publish what fit before it reported the rest as dropped. A
    /// microphone chunk mixed between those two steps found the queue full with no drop reported,
    /// and mixed in the stale samples at its end.
    #[test]
    fn regression_a_chunk_mixed_while_a_push_overflows_mixes_no_stale_sound() {
        let (mut other, mut mixer) = mix_queue(1000);
        other.push(&early(900));
        let mut out = Vec::new();
        during_push(other, vec![0.5; 200], || mixer.mix(&[0.0; 160], &mut out));
        assert!(out.iter().all(|&s| s == 0.0), "stale sound mixed in: {:?}", &out[..4]);
    }

    /// Regression (the same review): a chunk being mixed can take samples while an overflowing push
    /// is under way, so the next chunk finds the queue no longer full. The drop is reported before
    /// the push returns, and the report empties the queue.
    #[test]
    fn regression_samples_taken_during_an_overflowing_push_do_not_hide_its_drop() {
        let (mut other, mut mixer) = mix_queue(1000);
        other.push(&early(900));
        let mut out = Vec::new();
        during_push(other, vec![0.5; 200], || {
            for _ in 0..100 {
                mixer.other.pop().expect("queued");
            }
            mixer.mix(&[0.0; 160], &mut out);
        });
        assert!(out.iter().all(|&s| s == 0.0), "stale sound mixed in: {:?}", &out[..4]);
    }

    /// Regression (goal review round 4, 2026-09-30): a push that did not fit reported the drop and
    /// then published the part that fitted. A chunk mixed in between took the report and emptied
    /// the queue, and the next chunk mixed that part, which is older than the dropped rest. A push
    /// that does not fit is now dropped whole.
    #[test]
    fn regression_the_chunk_after_an_overflowing_push_mixes_none_of_it() {
        let (mut other, mut mixer) = mix_queue(1000);
        other.push(&early(900));
        let mut out = Vec::new();
        during_push(other, vec![0.5; 200], || mixer.mix(&[0.0; 160], &mut out));
        assert!(out.iter().all(|&s| s == 0.0), "stale sound mixed in: {:?}", &out[..4]);
        mixer.mix(&[0.0; 160], &mut out);
        assert!(out.iter().all(|&s| s == 0.0), "part of the dropped push mixed in: {:?}", &out[..4]);
    }

    /// Regression (the same review): after a report the mixer emptied only what its first read of
    /// the queue had counted, so samples pushed between its two reads, before the drop, were mixed.
    /// Here another thread pushes between the two reads: a push that fits, one that does not, and
    /// one that fits again. Nothing from before the drop may be mixed once the mixer has seen
    /// either the report or what came after it.
    #[test]
    fn regression_a_drop_between_the_mixers_two_reads_leaves_nothing_from_before_it() {
        let (mut other, mut mixer) = mix_queue(1000);
        other.push(&early(800));
        let mut out = Vec::new();
        mixer.mix_with(&[0.0; 160], &mut out, || {
            std::thread::scope(|s| {
                s.spawn(|| {
                    other.push(&[0.5; 100]);
                    other.push(&[0.25; 200]);
                    other.push(&[0.125; 50]);
                });
            });
        });
        assert!(format!("{other:?}").contains("free: 1000"), "sound from before the drop left queued: {other:?}");
        assert!(out.iter().all(|&s| s == 0.0), "sound from before the drop mixed in: {:?}", &out[..4]);
    }

    /// A push that finds the queue full publishes nothing, so a chunk mixed right after it may not
    /// see its report yet. The queue being full is enough.
    #[test]
    fn a_full_queue_is_emptied_before_a_drop_is_reported() {
        let (mut other, mut mixer) = mix_queue(1000);
        other.push(&early(1000));
        assert!(format!("{other:?}").contains("free: 0, overflowed: 0"), "full, nothing reported: {other:?}");
        let mut out = Vec::new();
        mixer.mix(&[0.0; 160], &mut out);
        assert!(out.iter().all(|&s| s == 0.0), "stale sound mixed in: {:?}", &out[..4]);
        assert_eq!(mixer.dropped(), 1000);
    }
}
