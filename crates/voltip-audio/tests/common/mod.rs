//! Signals shared by the echo tests: something with the rhythm and the spectrum of speech,
//! generated, so no recording has to live in the repository.
#![allow(dead_code)]

pub const RATE: usize = 16_000;

/// xorshift64: the same signals on every machine.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        ((self.0 >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
    }

    pub fn between(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * (self.next() * 0.5 + 0.5)
    }
}

/// A two-pole resonator (a formant): centre `hz`, bandwidth `bw` Hz.
struct Resonator {
    a1: f32,
    a2: f32,
    gain: f32,
    y1: f32,
    y2: f32,
}

impl Resonator {
    fn new(hz: f32, bw: f32) -> Self {
        let r = (-std::f32::consts::PI * bw / RATE as f32).exp();
        let theta = std::f32::consts::TAU * hz / RATE as f32;
        Self { a1: 2.0 * r * theta.cos(), a2: -r * r, gain: 1.0 - r, y1: 0.0, y2: 0.0 }
    }

    fn step(&mut self, x: f32) -> f32 {
        let y = self.gain * x + self.a1 * self.y1 + self.a2 * self.y2;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// Something with the rhythm and the spectrum of speech: syllables of 120–320 ms with pauses,
/// a glottal pulse train at a drifting pitch through three formants that move from syllable to
/// syllable, and now and then a fricative of filtered noise. `pitch` is the voice's range.
pub fn voice(seconds: f32, pitch: (f32, f32), seed: u64) -> Vec<f32> {
    let mut rng = Rng(seed);
    let total = (seconds * RATE as f32) as usize;
    let mut out = Vec::with_capacity(total);
    let mut phase = 0.0f32;
    while out.len() < total {
        let syllable = (rng.between(0.12, 0.32) * RATE as f32) as usize;
        let pause = (rng.between(0.04, 0.35) * RATE as f32) as usize;
        let f0 = rng.between(pitch.0, pitch.1);
        let glide = rng.between(-0.3, 0.3);
        let mut formants = [
            Resonator::new(rng.between(300.0, 800.0), 90.0),
            Resonator::new(rng.between(900.0, 2_200.0), 120.0),
            Resonator::new(rng.between(2_300.0, 3_200.0), 160.0),
        ];
        let fricative = rng.next() > 0.6;
        let mut hiss = Resonator::new(rng.between(3_800.0, 5_500.0), 900.0);
        for i in 0..syllable {
            let t = i as f32 / syllable as f32;
            let envelope = (std::f32::consts::PI * t).sin().powf(0.6);
            let hz = f0 * (1.0 + glide * t);
            phase += hz / RATE as f32;
            let pulse = if phase >= 1.0 {
                phase -= 1.0;
                1.0
            } else {
                0.0
            };
            let mut voiced = 0.0;
            for (k, f) in formants.iter_mut().enumerate() {
                voiced += f.step(pulse) * [1.0, 0.6, 0.3][k];
            }
            let noise = if fricative && t < 0.25 { hiss.step(rng.next()) * 0.4 } else { 0.0 };
            out.push(envelope * (voiced * 6.0 + noise));
        }
        out.extend(std::iter::repeat_n(0.0, pause));
    }
    out.truncate(total);
    normalise(&mut out, 0.05);
    out
}

/// Scale `x` so that its RMS where it is not silent is `rms`.
pub fn normalise(x: &mut [f32], rms: f32) {
    let loud: Vec<f32> = x.iter().copied().filter(|v| v.abs() > 1e-5).collect();
    let now = (loud.iter().map(|v| v * v).sum::<f32>() / loud.len().max(1) as f32).sqrt();
    if now > 0.0 {
        x.iter_mut().for_each(|v| *v *= rms / now);
    }
}
