//! Pure level-meter arithmetic. Nothing in here touches a device, so every function is exercised
//! by unit tests with synthetic signals.
//!
//! * [`rms`] / [`peak`] / [`to_dbfs`] — the three primitives.
//! * [`PeakHold`] — the classic meter ballistics: hold the loudest peak for a while, then let it
//!   fall at a fixed rate in dB per second.
//! * [`FrameAccumulator`] — turns an interleaved sample stream of any supported format into a
//!   [`LevelFrame`] every `sample_rate / frames_per_second` sample frames, without allocating.

use crate::LevelFrame;

/// Silence and anything quieter than this is reported as `-90 dBFS`.
pub const DBFS_FLOOR: f32 = -90.0;
/// A sample whose magnitude reaches this is counted as clipping (integer sources saturate at
/// `1.0` exactly; float sources may exceed it).
pub const CLIP_THRESHOLD: f32 = 0.999;
/// Default fall rate of the held peak once its hold time has elapsed.
pub const DEFAULT_PEAK_DECAY_DB_PER_S: f32 = 20.0;

/// Root mean square of `samples`; `0.0` for an empty slice.
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
    (sum_sq / samples.len() as f64).sqrt() as f32
}

/// Largest absolute sample value; `0.0` for an empty slice.
pub fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0_f32, |acc, &s| acc.max(s.abs()))
}

/// Linear amplitude to dBFS: `1.0` is `0 dBFS`, anything at or below `10^(-4.5)` (and `NaN`,
/// zero, negatives) is clamped to [`DBFS_FLOOR`]. Values above `1.0` map to positive dB.
pub fn to_dbfs(linear: f32) -> f32 {
    if linear.is_nan() || linear <= 0.0 {
        return DBFS_FLOOR;
    }
    (20.0 * linear.log10()).max(DBFS_FLOOR)
}

/// Peak-hold ballistics in the dBFS domain, driven by an injected clock in milliseconds.
///
/// A new peak at or above the value currently shown replaces it and restarts the hold. Once
/// `hold_ms` has passed without a louder peak, the shown value falls linearly (in dB) at the
/// configured rate until it reaches [`DBFS_FLOOR`] or a louder peak arrives.
#[derive(Clone, Debug, PartialEq)]
pub struct PeakHold {
    hold_ms: u32,
    decay_db_per_ms: f32,
    held_dbfs: f32,
    held_at_ms: u64,
    primed: bool,
}

impl PeakHold {
    /// Hold for `hold_ms`, then decay at [`DEFAULT_PEAK_DECAY_DB_PER_S`].
    pub fn new(hold_ms: u32) -> Self {
        Self::with_decay(hold_ms, DEFAULT_PEAK_DECAY_DB_PER_S)
    }

    /// Hold for `hold_ms`, then decay at `decay_db_per_second` (non-positive means never decay).
    pub fn with_decay(hold_ms: u32, decay_db_per_second: f32) -> Self {
        Self { hold_ms, decay_db_per_ms: decay_db_per_second.max(0.0) / 1000.0, held_dbfs: DBFS_FLOOR, held_at_ms: 0, primed: false }
    }

    /// Feed the peak of the newest frame (in dBFS) at time `now_ms`; returns the value to show.
    pub fn update(&mut self, peak_dbfs: f32, now_ms: u64) -> f32 {
        let shown = self.current(now_ms);
        if !self.primed || peak_dbfs >= shown {
            self.held_dbfs = peak_dbfs;
            self.held_at_ms = now_ms;
            self.primed = true;
            return peak_dbfs;
        }
        shown
    }

    /// The value shown at `now_ms` without feeding a new peak.
    pub fn current(&self, now_ms: u64) -> f32 {
        if !self.primed {
            return DBFS_FLOOR;
        }
        let elapsed = now_ms.saturating_sub(self.held_at_ms);
        let past_hold = elapsed.saturating_sub(u64::from(self.hold_ms));
        if past_hold == 0 {
            return self.held_dbfs;
        }
        (self.held_dbfs - past_hold as f32 * self.decay_db_per_ms).max(DBFS_FLOOR)
    }
}

/// One interleaved chunk of captured samples in the device's native format.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SampleChunk<'a> {
    /// `-1.0..=1.0` floats (WASAPI shared mode, CoreAudio, ALSA `FLOAT_LE`).
    F32(&'a [f32]),
    /// Signed 16-bit (ALSA `S16_LE`, WASAPI exclusive mode).
    I16(&'a [i16]),
    /// Unsigned 16-bit with `32768` as silence.
    U16(&'a [u16]),
    /// Signed 32-bit (ALSA `S32_LE` on most USB and HDA capture devices).
    I32(&'a [i32]),
}

/// Accumulates interleaved samples and emits one [`LevelFrame`] per
/// `sample_rate * channels / frames_per_second` samples.
///
/// The RMS is over all channels of the frame; the peak is the loudest sample in it, passed
/// through [`PeakHold`] whose clock is derived from the frame cadence, so identical input always
/// yields identical frames. No heap allocation happens after construction, which is what makes it
/// safe to drive from the audio callback.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameAccumulator {
    sample_rate_hz: u32,
    channels: u16,
    frames_per_second: u16,
    samples_per_frame: usize,
    filled: usize,
    sum_sq: f64,
    peak: f32,
    clipping: bool,
    seq: u64,
    hold: PeakHold,
}

impl FrameAccumulator {
    /// `frames_per_second` of `0` is treated as `1`; a frame never has fewer than one sample.
    pub fn new(sample_rate_hz: u32, channels: u16, frames_per_second: u16, peak_hold_ms: u32) -> Self {
        let frames_per_second = frames_per_second.max(1);
        let channels = channels.max(1);
        let samples_per_frame = (sample_rate_hz as usize * channels as usize / frames_per_second as usize).max(1);
        Self {
            sample_rate_hz,
            channels,
            frames_per_second,
            samples_per_frame,
            filled: 0,
            sum_sq: 0.0,
            peak: 0.0,
            clipping: false,
            seq: 0,
            hold: PeakHold::new(peak_hold_ms),
        }
    }

    /// Interleaved samples (all channels) that make up one emitted frame.
    pub fn samples_per_frame(&self) -> usize {
        self.samples_per_frame
    }

    /// Frames emitted so far.
    pub fn frames_emitted(&self) -> u64 {
        self.seq
    }

    /// Push a chunk in whatever format the device delivers.
    pub fn push(&mut self, chunk: SampleChunk<'_>, emit: impl FnMut(LevelFrame)) {
        match chunk {
            SampleChunk::F32(s) => self.push_f32(s, emit),
            SampleChunk::I16(s) => self.push_i16(s, emit),
            SampleChunk::U16(s) => self.push_u16(s, emit),
            SampleChunk::I32(s) => self.push_i32(s, emit),
        }
    }

    /// Push `-1.0..=1.0` floats.
    pub fn push_f32(&mut self, samples: &[f32], emit: impl FnMut(LevelFrame)) {
        self.push_with(samples, |s| s, emit);
    }

    /// Push signed 16-bit samples (`i16::MIN` maps to `-1.0`).
    pub fn push_i16(&mut self, samples: &[i16], emit: impl FnMut(LevelFrame)) {
        self.push_with(samples, i16_to_f32, emit);
    }

    /// Push unsigned 16-bit samples (`32768` is silence).
    pub fn push_u16(&mut self, samples: &[u16], emit: impl FnMut(LevelFrame)) {
        self.push_with(samples, u16_to_f32, emit);
    }

    /// Push signed 32-bit samples (`i32::MIN` maps to `-1.0`).
    pub fn push_i32(&mut self, samples: &[i32], emit: impl FnMut(LevelFrame)) {
        self.push_with(samples, i32_to_f32, emit);
    }

    fn push_with<T: Copy>(&mut self, samples: &[T], to_f32: impl Fn(T) -> f32, mut emit: impl FnMut(LevelFrame)) {
        for &raw in samples {
            let s = to_f32(raw);
            let magnitude = s.abs();
            self.sum_sq += f64::from(s) * f64::from(s);
            self.peak = self.peak.max(magnitude);
            self.clipping |= magnitude >= CLIP_THRESHOLD;
            self.filled += 1;
            if self.filled == self.samples_per_frame {
                emit(self.finish_frame());
            }
        }
    }

    fn finish_frame(&mut self) -> LevelFrame {
        let rms_linear = (self.sum_sq / self.samples_per_frame as f64).sqrt() as f32;
        let now_ms = self.seq * 1000 / u64::from(self.frames_per_second);
        let frame = LevelFrame {
            rms_dbfs: to_dbfs(rms_linear),
            peak_dbfs: self.hold.update(to_dbfs(self.peak), now_ms),
            clipping: self.clipping,
            sample_rate_hz: self.sample_rate_hz,
            channels: self.channels,
            seq: self.seq,
        };
        self.seq += 1;
        self.filled = 0;
        self.sum_sq = 0.0;
        self.peak = 0.0;
        self.clipping = false;
        frame
    }
}

/// `i16` sample to `-1.0..=1.0`.
pub fn i16_to_f32(s: i16) -> f32 {
    f32::from(s) / 32_768.0
}

/// `u16` sample (`32768` = silence) to `-1.0..=1.0`.
pub fn u16_to_f32(s: u16) -> f32 {
    (f32::from(s) - 32_768.0) / 32_768.0
}

/// `i32` sample to `-1.0..=1.0`.
pub fn i32_to_f32(s: i32) -> f32 {
    (s as f32) / 2_147_483_648.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(amplitude: f32, freq_hz: f32, sample_rate: u32, n: usize) -> Vec<f32> {
        (0..n).map(|i| amplitude * (2.0 * std::f32::consts::PI * freq_hz * i as f32 / sample_rate as f32).sin()).collect()
    }

    fn close(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn rms_and_peak_of_known_signals() {
        assert_eq!(rms(&[]), 0.0);
        assert_eq!(peak(&[]), 0.0);
        assert_eq!(rms(&[0.0; 64]), 0.0);
        assert_eq!(rms(&[0.5; 64]), 0.5);
        assert_eq!(rms(&[-0.5, 0.5, -0.5, 0.5]), 0.5);
        assert_eq!(peak(&[-0.7, 0.2, 0.6]), 0.7);
        // One full cycle of a 1 kHz sine at 48 kHz: rms = A/√2, peak ≈ A.
        let s = sine(0.8, 1000.0, 48_000, 48);
        assert!(close(rms(&s), 0.8 / std::f32::consts::SQRT_2, 1e-4), "rms {}", rms(&s));
        assert!(close(peak(&s), 0.8, 1e-3), "peak {}", peak(&s));
    }

    #[test]
    fn dbfs_mapping() {
        assert_eq!(to_dbfs(1.0), 0.0);
        assert!(close(to_dbfs(0.5), -6.0206, 1e-3));
        assert!(close(to_dbfs(0.1), -20.0, 1e-4));
        assert_eq!(to_dbfs(0.0), DBFS_FLOOR);
        assert_eq!(to_dbfs(-0.5), DBFS_FLOOR);
        assert_eq!(to_dbfs(f32::NAN), DBFS_FLOOR);
        assert_eq!(to_dbfs(1e-9), DBFS_FLOOR);
        // Just above the floor is not clamped.
        assert!(to_dbfs(10f32.powf(-4.4)) > DBFS_FLOOR);
        // Above full scale maps to positive dB.
        assert!(close(to_dbfs(2.0), 6.0206, 1e-3));
    }

    #[test]
    fn sample_converters() {
        assert_eq!(i16_to_f32(0), 0.0);
        assert_eq!(i16_to_f32(i16::MIN), -1.0);
        assert!(close(i16_to_f32(i16::MAX), 1.0, 1e-4));
        assert_eq!(u16_to_f32(32_768), 0.0);
        assert_eq!(u16_to_f32(0), -1.0);
        assert!(close(u16_to_f32(u16::MAX), 1.0, 1e-4));
        assert_eq!(i32_to_f32(0), 0.0);
        assert_eq!(i32_to_f32(i32::MIN), -1.0);
        assert!(close(i32_to_f32(i32::MAX), 1.0, 1e-6));
    }

    #[test]
    fn peak_hold_holds_then_decays() {
        let mut hold = PeakHold::with_decay(800, 20.0);
        assert_eq!(hold.current(0), DBFS_FLOOR, "unprimed shows the floor");
        assert_eq!(hold.update(-6.0, 0), -6.0);
        // Quieter peaks within the hold window do not lower the shown value.
        assert_eq!(hold.update(-30.0, 400), -6.0);
        assert_eq!(hold.update(-30.0, 800), -6.0);
        // 100 ms past the hold: 20 dB/s → 2 dB down.
        assert!(close(hold.update(-30.0, 900), -8.0, 1e-4));
        assert!(close(hold.current(1300), -16.0, 1e-4));
        // Decay reaches the quieter signal, which then takes over and restarts the hold.
        assert_eq!(hold.update(-30.0, 2100), -30.0);
        assert_eq!(hold.update(-40.0, 2500), -30.0);
        // A louder peak replaces immediately.
        assert_eq!(hold.update(-3.0, 2600), -3.0);
        // Never below the floor.
        assert_eq!(hold.current(1_000_000), DBFS_FLOOR);
        // Clock going backwards is treated as "no time passed".
        assert_eq!(hold.current(0), -3.0);
    }

    #[test]
    fn peak_hold_default_and_no_decay() {
        let mut hold = PeakHold::new(100);
        hold.update(-10.0, 0);
        assert!(close(hold.current(1100), -30.0, 1e-4), "default 20 dB/s: 1 s past the hold is 20 dB down");

        let mut frozen = PeakHold::with_decay(0, 0.0);
        frozen.update(-12.0, 0);
        assert_eq!(frozen.current(60_000), -12.0);
        let mut negative_rate = PeakHold::with_decay(0, -5.0);
        negative_rate.update(-12.0, 0);
        assert_eq!(negative_rate.current(60_000), -12.0, "negative rates are clamped to no decay");
    }

    fn collect(acc: &mut FrameAccumulator, chunk: SampleChunk<'_>) -> Vec<LevelFrame> {
        let mut out = Vec::new();
        acc.push(chunk, |f| out.push(f));
        out
    }

    #[test]
    fn accumulator_cadence_16k_mono_and_48k_stereo() {
        let mut mono = FrameAccumulator::new(16_000, 1, 30, 800);
        assert_eq!(mono.samples_per_frame(), 533);
        let silence = vec![0.0_f32; 16_000];
        let frames = collect(&mut mono, SampleChunk::F32(&silence));
        assert_eq!(frames.len(), 30, "one second of 16 kHz mono at 30 fps");
        assert_eq!(mono.frames_emitted(), 30);
        assert!(frames.iter().enumerate().all(|(i, f)| f.seq == i as u64), "seq is monotonic from 0");
        assert!(frames.iter().all(|f| f.sample_rate_hz == 16_000 && f.channels == 1));
        assert!(frames.iter().all(|f| f.rms_dbfs == DBFS_FLOOR && f.peak_dbfs == DBFS_FLOOR && !f.clipping));

        let mut stereo = FrameAccumulator::new(48_000, 2, 30, 800);
        assert_eq!(stereo.samples_per_frame(), 3200);
        let quarter_second = vec![0.25_f32; 48_000 * 2 / 4];
        let frames = collect(&mut stereo, SampleChunk::F32(&quarter_second));
        assert_eq!(frames.len(), 7, "24 000 samples / 3200 = 7 full frames");
        assert!(frames.iter().all(|f| close(f.rms_dbfs, -12.04, 1e-2) && close(f.peak_dbfs, -12.04, 1e-2)));
        assert!(frames.iter().all(|f| f.sample_rate_hz == 48_000 && f.channels == 2));
        // The 1600 leftover samples roll into the next frame.
        let more = collect(&mut stereo, SampleChunk::F32(&quarter_second[..1600]));
        assert_eq!(more.len(), 1);
        assert_eq!(more[0].seq, 7);
        assert!(collect(&mut stereo, SampleChunk::F32(&[])).is_empty());
    }

    #[test]
    fn accumulator_frames_split_across_pushes_and_seq_continues() {
        let mut acc = FrameAccumulator::new(1000, 1, 10, 0);
        assert_eq!(acc.samples_per_frame(), 100);
        let loud = vec![0.5_f32; 60];
        assert!(collect(&mut acc, SampleChunk::F32(&loud)).is_empty());
        let frames = collect(&mut acc, SampleChunk::F32(&loud));
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].seq, 0);
        assert!(close(frames[0].rms_dbfs, -6.02, 1e-2));
        let mut all = Vec::new();
        for _ in 0..10 {
            all.extend(collect(&mut acc, SampleChunk::F32(&loud)));
        }
        let seqs: Vec<u64> = all.iter().map(|f| f.seq).collect();
        assert_eq!(seqs, (1..=6).collect::<Vec<_>>());
    }

    #[test]
    fn accumulator_i16_u16_i32_inputs() {
        let mut acc = FrameAccumulator::new(8000, 1, 10, 0);
        let half_i16 = vec![16_384_i16; 800];
        let f = collect(&mut acc, SampleChunk::I16(&half_i16));
        assert_eq!(f.len(), 1);
        assert!(close(f[0].rms_dbfs, -6.02, 1e-2), "{}", f[0].rms_dbfs);
        assert!(!f[0].clipping);

        let mut acc = FrameAccumulator::new(8000, 1, 10, 0);
        let silence_u16 = vec![32_768_u16; 800];
        let f = collect(&mut acc, SampleChunk::U16(&silence_u16));
        assert_eq!(f[0].rms_dbfs, DBFS_FLOOR);
        let full_u16 = vec![u16::MAX; 800];
        let f = collect(&mut acc, SampleChunk::U16(&full_u16));
        assert!(f[0].clipping);
        assert!(close(f[0].peak_dbfs, 0.0, 1e-3));

        let mut acc = FrameAccumulator::new(8000, 1, 10, 0);
        let quarter_i32 = vec![i32::MAX / 4; 800];
        let f = collect(&mut acc, SampleChunk::I32(&quarter_i32));
        assert!(close(f[0].rms_dbfs, -12.04, 1e-2), "{}", f[0].rms_dbfs);
        let min_i32 = vec![i32::MIN; 800];
        let f = collect(&mut acc, SampleChunk::I32(&min_i32));
        assert!(f[0].clipping);
        assert_eq!(f[0].peak_dbfs, 0.0);
    }

    #[test]
    fn clipping_flag_per_frame() {
        let mut acc = FrameAccumulator::new(100, 1, 1, 0);
        let mut quiet = vec![0.1_f32; 100];
        assert!(!collect(&mut acc, SampleChunk::F32(&quiet))[0].clipping);
        quiet[57] = -0.999;
        let f = collect(&mut acc, SampleChunk::F32(&quiet));
        assert!(f[0].clipping, "a single sample at the threshold flags the frame");
        assert!(close(f[0].peak_dbfs, -0.0087, 1e-3));
        // The flag does not stick to the next frame.
        quiet[57] = 0.1;
        let f = collect(&mut acc, SampleChunk::F32(&quiet));
        assert!(!f[0].clipping);
        // Peak hold is 0 ms here, so the peak falls at 20 dB/s: one frame later it is 20 dB down
        // from -0.0087 but the live -20 dB signal is louder, so the live value wins.
        assert!(close(f[0].peak_dbfs, -20.0, 1e-3), "{}", f[0].peak_dbfs);
        let mut i16_clip = FrameAccumulator::new(4, 1, 1, 0);
        let f = collect(&mut i16_clip, SampleChunk::I16(&[0, i16::MIN, 0, 0]));
        assert!(f[0].clipping);
    }

    #[test]
    fn peak_hold_inside_accumulator_uses_frame_clock() {
        // 10 fps → 100 ms per frame; hold 250 ms → frames 0..=2 hold, frame 3 has decayed 50 ms.
        let mut acc = FrameAccumulator::new(1000, 1, 10, 250);
        let burst = vec![1.0_f32; 100];
        let silence = vec![0.0_f32; 100];
        let first = collect(&mut acc, SampleChunk::F32(&burst));
        assert_eq!(first[0].peak_dbfs, 0.0);
        assert!(first[0].clipping);
        let f1 = collect(&mut acc, SampleChunk::F32(&silence));
        let f2 = collect(&mut acc, SampleChunk::F32(&silence));
        let f3 = collect(&mut acc, SampleChunk::F32(&silence));
        assert_eq!(f1[0].peak_dbfs, 0.0);
        assert_eq!(f2[0].peak_dbfs, 0.0);
        assert!(close(f3[0].peak_dbfs, -1.0, 1e-4), "{}", f3[0].peak_dbfs);
        assert_eq!(f3[0].rms_dbfs, DBFS_FLOOR);
        assert!(!f3[0].clipping);
    }

    #[test]
    fn accumulator_degenerate_parameters() {
        let acc = FrameAccumulator::new(0, 0, 0, 0);
        assert_eq!(acc.samples_per_frame(), 1);
        let mut acc = FrameAccumulator::new(48_000, 1, 0, 0);
        assert_eq!(acc.samples_per_frame(), 48_000, "fps 0 behaves as 1");
        let f = collect(&mut acc, SampleChunk::F32(&[0.5]));
        assert!(f.is_empty());
    }
}
