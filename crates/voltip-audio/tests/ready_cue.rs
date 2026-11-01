#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The ready cue on a real sound system (docs/dictation.md §30.3): the cue plays into a PulseAudio
//! null sink whose monitor is the microphone, so the device chain certainly records it, and the
//! take must not contain it. `#[ignore]`, Linux, with a private server (nothing is heard, no other
//! session's sound server is touched):
//!
//! ```text
//! scripts/measure/private-pulse.sh start /tmp/voltip-cue-pulse
//! PULSE_SERVER=unix:/tmp/voltip-cue-pulse/native cargo test -p voltip-audio --test ready_cue -- --ignored --nocapture
//! scripts/measure/private-pulse.sh stop /tmp/voltip-cue-pulse
//! ```
//!
//! Quiet noise plays into the sink throughout, so the tones' frequencies have a background to be
//! compared with; `parec` records the monitor beside the recorder as the control that the cue was
//! there to be removed. With `VOLTIP_CUE_CORPUS=<manifest.csv>` (scripts/measure/make-corpus.py
//! sentences) and `VOLTIP_CUE_OUT=<dir>`, `the_cue_leaves_the_first_words_alone` records every
//! sentence twice, with the cue and without it, the sentence starting 150 ms after the cue's window
//! (or as long after the first samples without it); `scripts/measure/cue-words.sh` runs it, has
//! both recognised and compares their first characters.

#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use voltip_audio::cue::{CUE_TONES_HZ, WindowClock};
use voltip_audio::{CpalBackend, Recorder, RecorderConfig};

const RATE: u32 = 16_000;
/// The background noise's RMS, dBFS.
const NOISE_DBFS: f32 = -40.0;
/// The tones may stand at most this far above the background in the take.
const ALLOWED_DB: f32 = 6.0;
/// In the monitor's own recording they must stand at least this far above it (the control).
const PRESENT_DB: f32 = 20.0;

/// Kills a helper process however the test ends.
struct Helper(Child);

impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A directory of this test's own, removed however the test ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("voltip-ready-cue-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn pulse_ready() -> bool {
    std::env::var("PULSE_SERVER").is_ok_and(|s| s.starts_with("unix:"))
        && Command::new("pactl").arg("info").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

/// A 16 kHz mono 16-bit WAV of white noise at [`NOISE_DBFS`] RMS, `seconds` long.
fn noise_wav(path: &Path, seconds: u32) {
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let amplitude = 10f32.powf(NOISE_DBFS / 20.0) * 3f32.sqrt();
    let samples: Vec<i16> = (0..RATE * seconds)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let uniform = (state >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0;
            (uniform * amplitude * 32_767.0) as i16
        })
        .collect();
    write_wav(path, &samples);
}

fn write_wav(path: &Path, samples: &[i16]) {
    let recording = voltip_audio::Recording::from_samples(samples.to_vec(), RATE, false);
    std::fs::write(path, recording.to_wav()).unwrap();
}

/// Play `wav` into the private server's default sink.
fn play(wav: &Path) -> Helper {
    Helper(Command::new("paplay").arg(wav).stdout(Stdio::null()).stderr(Stdio::null()).spawn().expect("paplay"))
}

/// Record the default sink's monitor as raw 16 kHz mono 16-bit into `out`.
fn monitor(out: &Path) -> Helper {
    let file = std::fs::File::create(out).unwrap();
    Helper(
        Command::new("parec")
            .args(["--device=@DEFAULT_MONITOR@", "--rate=16000", "--channels=1", "--format=s16le", "--raw"])
            .stdout(file)
            .stderr(Stdio::null())
            .spawn()
            .expect("parec"),
    )
}

/// Goertzel power of `frame` at `hz`.
fn goertzel(frame: &[f32], hz: f32) -> f32 {
    let w = 2.0 * std::f32::consts::PI * hz / RATE as f32;
    let coeff = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0_f32, 0.0_f32);
    for &x in frame {
        let s = x + coeff * s1 - s2;
        s2 = s1;
        s1 = s;
    }
    (s1 * s1 + s2 * s2 - coeff * s1 * s2) / (frame.len() * frame.len()) as f32
}

/// The tones' energy over 120 ms windows (six 20 ms frames, both tones), in dB, one per window.
fn tone_energy(samples: &[i16]) -> Vec<f32> {
    let floats: Vec<f32> = samples.iter().map(|&s| f32::from(s) / 32_768.0).collect();
    let frames: Vec<f32> = floats.as_chunks::<320>().0.iter().map(|frame| CUE_TONES_HZ.iter().map(|&hz| goertzel(frame, hz)).sum::<f32>()).collect();
    frames.windows(6).step_by(3).map(|w| 10.0 * (w.iter().sum::<f32>() / 6.0).max(1e-20).log10()).collect()
}

/// The median and the maximum of `values`.
fn median_and_max(values: &[f32]) -> (f32, f32) {
    let mut sorted = values.to_vec();
    sorted.sort_by(f32::total_cmp);
    (sorted[sorted.len() / 2], *sorted.last().unwrap())
}

fn pcm_file(path: &Path) -> Vec<i16> {
    std::fs::read(path).unwrap().as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect()
}

/// The cue is recorded by the device chain (the monitor hears it) and is not in the take: the
/// take's energy at the two tones stays within [`ALLOWED_DB`] of the background.
#[test]
#[ignore = "needs a private PulseAudio server with paplay and parec (see the file's head)"]
fn the_cue_is_heard_by_the_microphone_and_kept_out_of_the_take() {
    assert!(pulse_ready(), "PULSE_SERVER must name a running private PulseAudio server (scripts/measure/private-pulse.sh)");
    let dir = Scratch::new("take");
    let noise = dir.path().join("noise.wav");
    noise_wav(&noise, 6);
    let monitored = dir.path().join("monitor.raw");
    let _monitor = monitor(&monitored);
    let _noise = play(&noise);
    let backend = CpalBackend::new();
    let (ready_tx, ready_rx) = mpsc::channel();
    let config = RecorderConfig { ready_cue: true, ..RecorderConfig::default() };
    let recorder = Recorder::start_with_ready(
        &backend,
        config,
        |_| {},
        move || {
            let _ = ready_tx.send(Instant::now());
        },
    )
    .expect("the default input records the sink's monitor");
    let first = ready_rx.recv_timeout(Duration::from_secs(10)).expect("the microphone delivered its first samples");
    let window = wait_for(Duration::from_secs(5), || recorder.ready_cue_window()).expect("the cue's output started the tone");
    eprintln!("ready cue: window {:?} after the first samples, {:?} long, by {:?}", window.0.saturating_duration_since(first), window.1 - window.0, window.2);
    // Long enough for the noise to frame the cue on both sides.
    std::thread::sleep(Duration::from_millis(2_500));
    let take = recorder.stop().unwrap();
    drop(_monitor);
    let (background, peak) = median_and_max(&tone_energy(&take.samples));
    eprintln!("ready cue: take {} ms; tones {peak:.1} dB at most, background {background:.1} dB", take.duration_ms);
    assert!(peak <= background + ALLOWED_DB, "the cue is in the take: {peak:.1} dB against a background of {background:.1} dB");
    let heard = pcm_file(&monitored);
    let (monitor_background, monitor_peak) = median_and_max(&tone_energy(&heard));
    eprintln!(
        "ready cue: monitor {} ms; tones {monitor_peak:.1} dB at most, background {monitor_background:.1} dB",
        heard.len() as u64 * 1000 / u64::from(RATE)
    );
    assert!(monitor_peak >= monitor_background + PRESENT_DB, "the control: the monitor must have recorded the cue");
    assert!(window.2 == WindowClock::Capture || window.2 == WindowClock::Callback);
}

/// Poll `probe` until it answers or `limit` passes (a bounded condition wait).
fn wait_for<T>(limit: Duration, mut probe: impl FnMut() -> Option<T>) -> Option<T> {
    let started = Instant::now();
    loop {
        if let Some(value) = probe() {
            return Some(value);
        }
        if started.elapsed() > limit {
            return None;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// One sentence of the corpus manifest: its name and WAV.
fn sentences(manifest: &Path) -> Vec<(String, PathBuf)> {
    let text = std::fs::read_to_string(manifest).unwrap();
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().unwrap().split(',').collect();
    let column = |name: &str| header.iter().position(|h| *h == name).unwrap();
    let (name, wav) = (column("name"), column("wav"));
    lines.filter(|l| !l.trim().is_empty()).map(|l| l.split(',').collect::<Vec<_>>()).map(|c| (c[name].to_owned(), PathBuf::from(c[wav]))).collect()
}

/// The length of a 16 kHz mono WAV.
fn wav_length(path: &Path) -> Duration {
    let bytes = std::fs::metadata(path).unwrap().len().saturating_sub(44);
    Duration::from_millis(bytes / 2 * 1000 / u64::from(RATE))
}

/// Each corpus sentence recorded with the cue and then without it, starting 150 ms after the cue's
/// window, and as long after the first samples without the cue, into `VOLTIP_CUE_OUT`.
#[test]
#[ignore = "needs a private PulseAudio server, VOLTIP_CUE_CORPUS and VOLTIP_CUE_OUT (see the file's head)"]
fn the_cue_leaves_the_first_words_alone() {
    assert!(pulse_ready(), "PULSE_SERVER must name a running private PulseAudio server");
    let manifest = PathBuf::from(std::env::var("VOLTIP_CUE_CORPUS").expect("VOLTIP_CUE_CORPUS=<manifest.csv>"));
    let out = PathBuf::from(std::env::var("VOLTIP_CUE_OUT").expect("VOLTIP_CUE_OUT=<dir>"));
    std::fs::create_dir_all(&out).unwrap();
    let backend = CpalBackend::new();
    for (name, wav) in sentences(&manifest) {
        // How long after the first samples the cue run's sentence started; the run without the cue
        // starts as long after its own first samples.
        let mut offset = Duration::ZERO;
        for cue in [true, false] {
            let (ready_tx, ready_rx) = mpsc::channel();
            let config = RecorderConfig { ready_cue: cue, ..RecorderConfig::default() };
            let recorder = Recorder::start_with_ready(
                &backend,
                config,
                |_| {},
                move || {
                    let _ = ready_tx.send(Instant::now());
                },
            )
            .unwrap();
            let first = ready_rx.recv_timeout(Duration::from_secs(10)).expect("first samples");
            let start = if cue {
                let window = wait_for(Duration::from_secs(5), || recorder.ready_cue_window()).expect("the tone started");
                let start = window.1 + Duration::from_millis(150);
                offset = start.saturating_duration_since(first);
                start
            } else {
                first + offset
            };
            std::thread::sleep(start.saturating_duration_since(Instant::now()));
            let player = play(&wav);
            std::thread::sleep(wav_length(&wav) + Duration::from_millis(600));
            drop(player);
            let take = recorder.stop().unwrap();
            let file = out.join(format!("{name}-{}.wav", if cue { "cue" } else { "plain" }));
            std::fs::write(&file, take.to_wav()).unwrap();
            eprintln!("{} {} ms", file.display(), take.duration_ms);
        }
    }
}
