//! Echo cancellation for the mixed recording (docs/dictation.md §22.6). With the speakers on, the
//! microphone also picks up the computer's sound, tens of milliseconds after it was played and
//! coloured by the room, so the mix would hold it twice. Before the two are summed, the
//! microphone goes through WebRTC's AEC3 (`sonora`, a pure-Rust port) with the computer's sound
//! as the reference; its high-pass filter stays on, as WebRTC enforces it for AEC3.
//!
//! AEC3 works on 10 ms frames ([`FRAME`]) and the microphone's chunks have any length, so
//! [`EchoCanceller`] keeps what does not fill a frame for the next chunk and hands out exactly as
//! many samples as it was given, [`FRAME`] samples late (the first frame is silence). Its own
//! processing adds about 8 ms more. AEC3 finds the echo's delay itself; the reference only has to
//! come no later than the echo, which the mixer's pairing ensures (`crate::mix`).
//!
//! `sonora` allocates while it processes a frame (about 35 small allocations per 10 ms, measured
//! 2026-10-01), so the recorder runs it on a thread of its own, never in an audio callback.

use std::collections::VecDeque;

use crate::recorder::MIX_RATE_HZ;

/// Samples per AEC3 frame: 10 ms at [`MIX_RATE_HZ`].
pub const FRAME: usize = (MIX_RATE_HZ / 100) as usize;

/// Removes the computer's sound from the microphone, frame by frame.
pub struct EchoCanceller {
    apm: sonora::AudioProcessing,
    /// The frame being filled: microphone and reference, `filled` samples so far.
    mic: [f32; FRAME],
    far: [f32; FRAME],
    filled: usize,
    /// Cancelled samples not handed out yet; [`FRAME`] of silence at first.
    ready: VecDeque<f32>,
    /// One cancelled frame, and where AEC3's render path writes its (unchanged) copy.
    frame_out: [f32; FRAME],
    render_out: [f32; FRAME],
}

impl std::fmt::Debug for EchoCanceller {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EchoCanceller").field("filled", &self.filled).field("ready", &self.ready.len()).finish_non_exhaustive()
    }
}

impl Default for EchoCanceller {
    fn default() -> Self {
        Self::new()
    }
}

impl EchoCanceller {
    /// AEC3 alone: no noise suppression and no gain control, which would change the voice the
    /// recogniser hears.
    pub fn new() -> Self {
        let config = sonora::Config { echo_canceller: Some(sonora::config::EchoCanceller::default()), ..Default::default() };
        let stream = sonora::StreamConfig::new(MIX_RATE_HZ, 1);
        let apm = sonora::AudioProcessing::builder().config(config).capture_config(stream).render_config(stream).build();
        // At most a frame plus one sample is waiting before a sample is handed out.
        let mut ready = VecDeque::with_capacity(2 * FRAME + 1);
        ready.extend(std::iter::repeat_n(0.0, FRAME));
        Self { apm, mic: [0.0; FRAME], far: [0.0; FRAME], filled: 0, ready, frame_out: [0.0; FRAME], render_out: [0.0; FRAME] }
    }

    /// Cancel the echo of `far` (the computer's sound) in `mic`, both 16 kHz mono and paired
    /// sample by sample as the mixer pairs them, and append `mic.len()` samples to `out`: the
    /// cancelled microphone, [`FRAME`] samples late. A shorter `far` counts as silence.
    pub fn process(&mut self, mic: &[f32], far: &[f32], out: &mut Vec<f32>) {
        for (i, m) in mic.iter().enumerate() {
            self.mic[self.filled] = *m;
            self.far[self.filled] = far.get(i).copied().unwrap_or(0.0);
            self.filled += 1;
            if self.filled == FRAME {
                self.run_frame();
                self.filled = 0;
            }
            // A frame of silence went in first and every full frame adds one, so a sample waits.
            out.push(self.ready.pop_front().unwrap_or(0.0));
        }
    }

    fn run_frame(&mut self) {
        // The reference first: AEC3 needs what was played before the capture it may echo in. With
        // the formats fixed here neither call fails; if one ever did, the microphone passes as is.
        let rendered = self.apm.process_render_f32(&[&self.far], &mut [&mut self.render_out]).is_ok();
        // The delay hint is required with AEC3 on; AEC3's own estimator finds the real echo delay.
        let _ = self.apm.set_stream_delay_ms(0);
        let captured = rendered && self.apm.process_capture_f32(&[&self.mic], &mut [&mut self.frame_out]).is_ok();
        if captured {
            self.ready.extend(self.frame_out.iter().copied());
        } else {
            self.ready.extend(self.mic.iter().copied());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Any chunking hands out as many samples as went in, the same samples whatever the
    /// chunking, and the first frame is the silence of the frame delay.
    #[test]
    fn every_chunk_size_gets_its_length_back_and_the_same_samples() {
        let mic: Vec<f32> = (0..8_000).map(|n| 0.2 * (n as f32 * 0.07).sin()).collect();
        let far: Vec<f32> = (0..8_000).map(|n| 0.2 * (n as f32 * 0.031).sin()).collect();
        let whole = {
            let mut c = EchoCanceller::new();
            let mut out = Vec::new();
            c.process(&mic, &far, &mut out);
            out
        };
        assert_eq!(whole.len(), mic.len());
        assert!(whole[..FRAME].iter().all(|s| *s == 0.0), "the frame delay is silence");
        assert!(whole[FRAME..].iter().any(|s| s.abs() > 1e-3), "then the microphone comes through");
        for chunk in [1, 7, 159, 160, 161, 480, 1024] {
            let mut c = EchoCanceller::new();
            let mut out = Vec::new();
            for (m, f) in mic.chunks(chunk).zip(far.chunks(chunk)) {
                let before = out.len();
                c.process(m, f, &mut out);
                assert_eq!(out.len() - before, m.len(), "chunks of {chunk}");
            }
            assert_eq!(out, whole, "chunks of {chunk}");
        }
    }

    #[test]
    fn a_missing_reference_is_silence() {
        let mic: Vec<f32> = (0..1_600).map(|n| 0.2 * (n as f32 * 0.07).sin()).collect();
        let (mut short, mut silent) = (EchoCanceller::new(), EchoCanceller::new());
        let (mut a, mut b) = (Vec::new(), Vec::new());
        short.process(&mic, &[], &mut a);
        silent.process(&mic, &vec![0.0; mic.len()], &mut b);
        assert_eq!(a, b);
    }
}
