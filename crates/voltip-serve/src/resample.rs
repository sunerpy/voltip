//! Mono resampling as the upload is decoded: rubato's asynchronous sinc with a persistent state,
//! fed block by block, so a two-hour upload is never held in memory. The same resampler as
//! `voltip_audio::StreamResampler` (that crate cannot be used here: it links cpal, and with it ALSA,
//! which a headless server does not have).

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Async, FixedAsync, Indexing, Resampler, SincInterpolationParameters, SincInterpolationType, WindowFunction};

/// Input frames per resampler step.
const STEP_FRAMES: usize = 1024;
/// Length of the sinc filter (as the app's live tap).
const SINC_LEN: usize = 256;

/// A block-by-block mono resampler; equal rates pass the samples through.
pub struct StreamResampler {
    inner: Option<Async<f32>>,
    pending: Vec<f32>,
    out: Vec<f32>,
    /// Output frames still to drop: the resampler's start-up delay.
    trim: usize,
    step: usize,
    fed: usize,
    emitted: usize,
}

impl StreamResampler {
    /// From `from_hz` to `to_hz`.
    pub fn new(from_hz: u32, to_hz: u32) -> Result<Self, String> {
        if from_hz == 0 || to_hz == 0 {
            return Err(format!("{from_hz} Hz -> {to_hz} Hz: rates must be greater than 0"));
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
        let inner = Async::<f32>::new_sinc(f64::from(to_hz) / f64::from(from_hz), 1.0, &params, STEP_FRAMES, 1, FixedAsync::Input)
            .map_err(|e| format!("{from_hz} Hz -> {to_hz} Hz: {e}"))?;
        let step = inner.input_frames_next();
        let out = vec![0.0; inner.output_frames_max()];
        let trim = inner.output_delay();
        Ok(Self { inner: Some(inner), pending: Vec::with_capacity(step * 4), out, trim, step, fed: 0, emitted: 0 })
    }

    /// Resample `block` and hand what is ready to `sink`.
    pub fn push(&mut self, block: &[f32], sink: &mut impl FnMut(&[f32]) -> Result<(), String>) -> Result<(), String> {
        let Some(inner) = &mut self.inner else { return sink(block) };
        self.pending.extend_from_slice(block);
        self.fed += block.len();
        let mut consumed = 0;
        while self.pending.len() - consumed >= self.step {
            let input = InterleavedSlice::new(&self.pending[consumed..consumed + self.step], 1, self.step).map_err(|e| e.to_string())?;
            let frames = self.out.len();
            let mut output = InterleavedSlice::new_mut(&mut self.out, 1, frames).map_err(|e| e.to_string())?;
            let (used, produced) = inner.process_into_buffer(&input, &mut output, None).map_err(|e| e.to_string())?;
            consumed += used;
            emit(&mut self.trim, &mut self.emitted, &self.out, produced, usize::MAX, sink)?;
        }
        self.pending.drain(..consumed);
        Ok(())
    }

    /// The tail: the output covers the whole input (`ceil(frames * ratio)` frames in all).
    pub fn flush(&mut self, sink: &mut impl FnMut(&[f32]) -> Result<(), String>) -> Result<(), String> {
        let Some(inner) = &mut self.inner else { return Ok(()) };
        let expected = (inner.resample_ratio() * self.fed as f64).ceil() as usize;
        let mut first = true;
        while self.emitted < expected {
            let partial = if first { self.pending.len() } else { 0 };
            let input = InterleavedSlice::new(&self.pending, 1, partial).map_err(|e| e.to_string())?;
            let indexing = Indexing { partial_len: Some(partial), ..Indexing::default() };
            let frames = self.out.len();
            let mut output = InterleavedSlice::new_mut(&mut self.out, 1, frames).map_err(|e| e.to_string())?;
            let (_, produced) = inner.process_into_buffer(&input, &mut output, Some(&indexing)).map_err(|e| e.to_string())?;
            emit(&mut self.trim, &mut self.emitted, &self.out, produced, expected, sink)?;
            first = false;
        }
        self.pending.clear();
        Ok(())
    }
}

/// Hand `produced` frames of `out` to `sink`, minus the start-up delay still to drop, never past
/// `limit` frames in all.
fn emit(
    trim: &mut usize,
    emitted: &mut usize,
    out: &[f32],
    produced: usize,
    limit: usize,
    sink: &mut impl FnMut(&[f32]) -> Result<(), String>,
) -> Result<(), String> {
    let skip = produced.min(*trim);
    *trim -= skip;
    let end = produced.min(skip.saturating_add(limit.saturating_sub(*emitted)));
    if end > skip {
        *emitted += end - skip;
        sink(&out[skip..end])?;
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn run(from: u32, to: u32, input: &[f32], block: usize) -> Vec<f32> {
        let mut r = StreamResampler::new(from, to).unwrap();
        let mut out = Vec::new();
        let mut sink = |s: &[f32]| {
            out.extend_from_slice(s);
            Ok(())
        };
        for chunk in input.chunks(block) {
            r.push(chunk, &mut sink).unwrap();
        }
        r.flush(&mut sink).unwrap();
        out
    }

    #[test]
    fn the_output_covers_the_input_at_the_new_rate_and_keeps_the_level() {
        let input: Vec<f32> = (0..48_000).map(|i| 0.5 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48_000.0).sin()).collect();
        for block in [100, 4096, 48_000] {
            let out = run(48_000, 16_000, &input, block);
            assert_eq!(out.len(), 16_000, "block {block}");
            let peak = out[1000..15_000].iter().fold(0.0f32, |m, s| m.max(s.abs()));
            assert!((peak - 0.5).abs() < 0.02, "peak {peak}");
        }
        assert_eq!(run(16_000, 16_000, &input[..1000], 7), input[..1000].to_vec(), "equal rates pass through");
        assert_eq!(run(8_000, 16_000, &input[..8000], 512).len(), 16_000);
        assert!(StreamResampler::new(0, 16_000).is_err());
    }
}
