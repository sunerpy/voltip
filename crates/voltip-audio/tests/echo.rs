#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Echo cancellation of the mixed recording on synthetic rooms (docs/dictation.md §22.6): a voice
//! (the user) and another (the computer's sound through the speakers, heard by the microphone
//! after a delay and the room's reverberation). The thresholds are the ones the feature shipped
//! on: at least 20 dB of echo removed while only the computer speaks, at most 3 dB of the voice
//! lost while only the user speaks, and the voice kept while both speak (at most 8 dB lost: AEC3
//! holds back during double talk; measured 2–6 dB, the most with the echo 6 dB louder than the
//! voice; SIMD paths differ between processors, so the bounds keep a margin).
//!
//! The timing test is `#[ignore]`d: it means something only in an optimised build, on the machine
//! being judged:
//!
//! ```text
//! cargo test --release -p voltip-audio --test echo -- --ignored --nocapture
//! ```
//!
//! (`scripts/windows-remote.sh gate echo` runs it, with the rest of this file, on the Windows host.)

mod common;

use common::{RATE, Rng, voice};
use voltip_audio::EchoCanceller;
use voltip_audio::echo::FRAME;

/// The echo path: `delay_ms`, a direct sound and four early reflections, then a diffuse tail of
/// sparse taps that dies away 60 dB in `rt60` seconds.
fn room(far: &[f32], delay_ms: usize, rt60: f32, seed: u64) -> Vec<f32> {
    let mut rng = Rng(seed);
    let mut taps: Vec<(usize, f32)> = vec![(0, 1.0), (37, 0.6), (83, -0.45), (151, 0.35), (229, -0.3)];
    let length = (rt60 * RATE as f32) as usize;
    for _ in 0..400 {
        let at = 240 + (rng.between(0.0, 1.0) * (length - 240) as f32) as usize;
        let level = 0.4 * (-6.9078 * at as f32 / length as f32).exp();
        taps.push((at, level * rng.next()));
    }
    let delay = delay_ms * RATE / 1000;
    let mut echo = vec![0.0f32; far.len()];
    for (at, gain) in taps {
        for (n, x) in far.iter().enumerate() {
            if let Some(slot) = echo.get_mut(n + delay + at) {
                *slot += gain * x;
            }
        }
    }
    echo
}

/// The microphone of one scene, its parts, and where each part of the scene is.
struct Scene {
    far: Vec<f32>,
    near: Vec<f32>,
    mic: Vec<f32>,
    /// The microphone without the user: the echo and the noise alone.
    mic_without_near: Vec<f32>,
}

const FAR_ONLY: std::ops::Range<usize> = 3 * RATE..8 * RATE;
const DOUBLE_TALK: std::ops::Range<usize> = 8 * RATE + RATE / 4..12 * RATE - RATE / 4;
const NEAR_ONLY: std::ops::Range<usize> = 12 * RATE + RATE / 4..16 * RATE - RATE / 4;

/// 16 s: the computer speaks for 0–12 s, the user for 8–16 s; the echo at `echo_db` against the
/// user's voice.
fn scene(delay_ms: usize, rt60: f32, echo_db: f32, seed: u64) -> Scene {
    let total = 16 * RATE;
    let mut far = voice(12.0, (95.0, 140.0), seed);
    far.resize(total, 0.0);
    let mut near = vec![0.0f32; 8 * RATE];
    near.extend(voice(8.0, (190.0, 260.0), seed ^ 0x5bd1_e995));
    near.truncate(total);
    let mut echo = room(&far, delay_ms, rt60, seed ^ 0x9e37_79b9);
    let echo_rms = (echo[FAR_ONLY].iter().map(|v| v * v).sum::<f32>() / FAR_ONLY.len() as f32).sqrt();
    let gain = 0.05 * 10f32.powf(echo_db / 20.0) / echo_rms;
    echo.iter_mut().for_each(|v| *v *= gain);
    let mut rng = Rng(seed ^ 0x2545_f491);
    let noise: Vec<f32> = (0..total).map(|_| 0.000_5 * rng.next()).collect();
    let mic = (0..total).map(|n| near[n] + echo[n] + noise[n]).collect();
    let mic_without_near = (0..total).map(|n| echo[n] + noise[n]).collect();
    Scene { far, near, mic, mic_without_near }
}

/// The microphone through a fresh canceller, in chunks of `chunk` (as an audio callback gives
/// them), with the output moved back by the canceller's frame so it lines up with the input.
fn cancel(mic: &[f32], far: &[f32], chunk: usize) -> Vec<f32> {
    let mut canceller = EchoCanceller::new();
    let mut out = Vec::with_capacity(mic.len());
    for (m, f) in mic.chunks(chunk).zip(far.chunks(chunk)) {
        canceller.process(m, f, &mut out);
    }
    out.drain(..FRAME);
    out.resize(mic.len(), 0.0);
    out
}

fn energy(x: &[f32]) -> f64 {
    x.iter().map(|v| f64::from(*v) * f64::from(*v)).sum()
}

fn db(ratio: f64) -> f64 {
    10.0 * ratio.log10()
}

struct Measured {
    /// Echo removed while only the computer speaks.
    erle: f64,
    /// The voice lost while only the user speaks.
    near_only_loss: f64,
    /// The voice lost while both speak: what differs between the take with the user and the take
    /// without is the voice as the canceller let it through.
    double_talk_loss: f64,
}

fn measure(s: &Scene, chunk: usize) -> Measured {
    let out = cancel(&s.mic, &s.far, chunk);
    let without = cancel(&s.mic_without_near, &s.far, chunk);
    let voice: Vec<f32> = DOUBLE_TALK.map(|n| out[n] - without[n]).collect();
    Measured {
        erle: db(energy(&s.mic[FAR_ONLY]) / energy(&out[FAR_ONLY])),
        near_only_loss: db(energy(&s.mic[NEAR_ONLY]) / energy(&out[NEAR_ONLY])),
        double_talk_loss: db(energy(&s.near[DOUBLE_TALK]) / energy(&voice)),
    }
}

#[test]
fn the_echo_goes_and_the_voice_stays_in_rooms_near_and_far() {
    // (echo delay, reverberation, echo against the voice, a callback's chunk)
    for (delay_ms, rt60, echo_db, chunk) in [(30, 0.25, -6.0, 160), (60, 0.3, 0.0, 441), (120, 0.4, 6.0, 1024)] {
        let m = measure(&scene(delay_ms, rt60, echo_db, 0x51ed_270b ^ delay_ms as u64), chunk);
        println!(
            "{delay_ms} ms, RT60 {rt60} s, echo {echo_db:+} dB: echo removed {:.1} dB, voice lost {:.2} dB alone and {:.2} dB in double talk",
            m.erle, m.near_only_loss, m.double_talk_loss
        );
        assert!(m.erle >= 20.0, "{delay_ms} ms: only {:.1} dB of echo removed", m.erle);
        assert!(m.near_only_loss <= 3.0, "{delay_ms} ms: {:.2} dB of the voice lost", m.near_only_loss);
        assert!(m.double_talk_loss <= 8.0, "{delay_ms} ms: {:.2} dB of the voice lost in double talk", m.double_talk_loss);
    }
}

/// Without the computer's sound there is nothing to cancel: the voice passes (through the
/// high-pass filter AEC3 keeps on) at its level.
#[test]
fn a_silent_computer_leaves_the_voice_alone() {
    let near = voice(6.0, (190.0, 260.0), 7);
    let out = cancel(&near, &vec![0.0; near.len()], 480);
    let steady = RATE..near.len();
    let loss = db(energy(&near[steady.clone()]) / energy(&out[steady]));
    assert!(loss.abs() <= 1.0, "{loss:.2} dB");
}

/// Each 10 ms frame costs well under its 10 ms: the mixing thread keeps up with room to spare.
#[test]
#[ignore = "timing; meaningful only in an optimised build on the machine being judged"]
fn a_frame_takes_well_under_a_millisecond() {
    let s = scene(60, 0.3, 0.0, 11);
    let mut canceller = EchoCanceller::new();
    let mut out = Vec::with_capacity(FRAME);
    let mut times = Vec::with_capacity(s.mic.len() / FRAME);
    for (m, f) in s.mic.chunks(FRAME).zip(s.far.chunks(FRAME)) {
        let start = std::time::Instant::now();
        out.clear();
        canceller.process(m, f, &mut out);
        times.push(start.elapsed());
    }
    times.sort();
    let mean = times.iter().sum::<std::time::Duration>() / times.len() as u32;
    let p99 = times[times.len() * 99 / 100];
    println!("per 10 ms frame: mean {mean:?}, p99 {p99:?}, max {:?}", times[times.len() - 1]);
    assert!(mean < std::time::Duration::from_millis(1), "mean {mean:?}");
    assert!(p99 < std::time::Duration::from_millis(2), "p99 {p99:?}");
}
