#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Recording the computer's sound on a real sound system (docs/dictation.md §22). Each test plays a
//! tone and records it back, so each is `#[ignore]`:
//!
//! ```text
//! cargo test -p voltip-audio --test loopback -- --ignored --nocapture
//! ```
//!
//! - Linux: a PulseAudio server (PipeWire's `pipewire-pulse` counts) with `pactl` and `paplay`. The
//!   test adds a null sink `voltip_loopback_test`, plays the tone into it with `paplay`, records the
//!   sink's monitor through the recorder and removes the sink again; nothing is heard. The echo
//!   test (docs/dictation.md §22.6) also points the default input at a null sink's monitor for a
//!   few seconds, so the microphone hears exactly what the speakers play, and puts it back.
//! - Windows and macOS (14.6 or later; macOS asks the terminal for the permission to record other
//!   apps' audio the first time): the tone plays through the default output with cpal, at a
//!   quarter of full scale for about three seconds, and the recorder records that output. With
//!   `VOLTIP_LOOPBACK_PLAY=<16 kHz mono WAV>` it plays that recording instead, and
//!   `VOLTIP_LOOPBACK_OUT=<path>` keeps what was recorded as a WAV, for a recogniser to check
//!   (`scripts/windows-remote.sh gate loopback` stages both; see `crates/voltip-asr-local/tests/real.rs`).

mod common;

use std::time::Duration;

use voltip_audio::{CaptureSource, CpalBackend, Recorder, RecorderConfig, dsp};

/// The tone: A4 at a quarter of full scale.
const TONE_HZ: f32 = 440.0;
const AMPLITUDE: f32 = 0.25;
/// How long the recorder records while the tone plays.
const RECORD: Duration = Duration::from_secs(2);

/// RMS of `samples` in dBFS.
fn rms_dbfs(samples: &[i16]) -> f32 {
    let floats: Vec<f32> = samples.iter().map(|&s| dsp::i16_to_f32(s)).collect();
    dsp::to_dbfs(dsp::rms(&floats))
}

/// `length` of `source` while the sound plays; the sound starts before and ends after it.
#[cfg(target_os = "linux")]
fn record(backend: &CpalBackend, source: CaptureSource, length: Duration) -> voltip_audio::Recording {
    let config = RecorderConfig { source, ..RecorderConfig::default() };
    let recorder = Recorder::start_with(backend, config, |_| {}).expect("the computer's sound can be recorded here");
    // The recording's own length: the sound plays meanwhile.
    std::thread::sleep(length);
    recorder.stop().unwrap()
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "needs a PulseAudio or PipeWire sound server with pactl and paplay"]
fn the_computers_sound_is_recorded_from_a_sinks_monitor() {
    use std::process::Command;
    use voltip_audio::Backend as _;

    /// Removes the null sink however the test ends.
    struct Sink(String);
    impl Drop for Sink {
        fn drop(&mut self) {
            let _ = Command::new("pactl").args(["unload-module", &self.0]).status();
        }
    }
    let loaded = Command::new("pactl")
        .args(["load-module", "module-null-sink", "sink_name=voltip_loopback_test", "sink_properties=device.description=VoltipLoopbackTest"])
        .output()
        .expect("pactl");
    assert!(loaded.status.success(), "{}", String::from_utf8_lossy(&loaded.stderr));
    let _sink = Sink(String::from_utf8_lossy(&loaded.stdout).trim().to_owned());

    let backend = CpalBackend::new();
    assert!(backend.system_audio().is_available(), "{:?}", backend.system_audio());
    let outputs = backend.output_devices().unwrap();
    let sink = outputs.iter().find(|d| d.id.contains("voltip_loopback_test")).unwrap_or_else(|| panic!("the null sink is listed: {outputs:?}"));

    let rate = 48_000u32;
    let tone: Vec<i16> =
        (0..rate as usize * 4).map(|i| ((i as f32 / rate as f32 * TONE_HZ * std::f32::consts::TAU).sin() * AMPLITUDE * 32_767.0) as i16).collect();
    let wav = std::env::temp_dir().join(format!("voltip-loopback-{}.wav", std::process::id()));
    std::fs::write(&wav, voltip_audio::recording::encode_wav(&tone, rate)).unwrap();
    let mut player = Command::new("paplay").arg("--device=voltip_loopback_test").arg(&wav).spawn().expect("paplay");
    // paplay needs a moment to connect before the tone reaches the monitor.
    std::thread::sleep(Duration::from_millis(400));
    let recording = record(&backend, CaptureSource::System { output_id: Some(sink.id.clone()) }, RECORD);
    let _ = player.wait();
    let _ = std::fs::remove_file(&wav);
    let level = rms_dbfs(&recording.samples);
    println!("loopback on {}: {} ms at {level:.1} dBFS (tone at {:.1} dBFS RMS)", sink.name, recording.duration_ms, dsp::to_dbfs(AMPLITUDE / 2f32.sqrt()));
    assert!(recording.duration_ms >= 1_500, "{} ms", recording.duration_ms);
    assert!(level > -30.0, "the tone is in the recording: {level:.1} dBFS");
}

/// Echo cancellation on a real sound server (docs/dictation.md §22.6): the microphone hears
/// exactly what the speakers play (the default input points at the sink's monitor), a voice plays
/// into the sink, and three takes record it: the computer's sound alone, and mixed with the
/// microphone's copy, as captured and with the echo cancelled.
#[cfg(target_os = "linux")]
#[test]
#[ignore = "needs a PulseAudio or PipeWire sound server with pactl and paplay; points the default input at a null sink for a few seconds"]
fn the_microphones_echo_of_the_computers_sound_is_cancelled_on_a_real_sound_server() {
    use std::process::Command;
    use voltip_audio::Backend as _;

    /// Removes the null sink and puts the default input back, however the test ends.
    struct Setup {
        module: String,
        source: String,
    }
    impl Drop for Setup {
        fn drop(&mut self) {
            let _ = Command::new("pactl").args(["unload-module", &self.module]).status();
            // The default input as it was; a stand-in sink (`auto_null`) the server removed while
            // the test's sink was there may be back under another name, and the server then picks
            // the default itself.
            let _ = Command::new("pactl").args(["set-default-source", &self.source]).stderr(std::process::Stdio::null()).status();
        }
    }
    let pactl = |args: &[&str]| {
        let out = Command::new("pactl").args(args).output().expect("pactl");
        assert!(out.status.success(), "pactl {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    };
    let source = pactl(&["get-default-source"]);
    let module = pactl(&["load-module", "module-null-sink", "sink_name=voltip_echo_test", "sink_properties=device.description=VoltipEchoTest"]);
    let _setup = Setup { module, source };
    pactl(&["set-default-source", "voltip_echo_test.monitor"]);

    let backend = CpalBackend::new();
    let outputs = backend.output_devices().unwrap();
    let sink = outputs.iter().find(|d| d.id.contains("voltip_echo_test")).unwrap_or_else(|| panic!("the null sink is listed: {outputs:?}"));
    let speech: Vec<i16> = common::voice(8.0, (110.0, 160.0), 3).iter().map(|v| (v * 2.0 * 32_767.0) as i16).collect();
    let wav = std::env::temp_dir().join(format!("voltip-echo-{}.wav", std::process::id()));
    std::fs::write(&wav, voltip_audio::recording::encode_wav(&speech, 16_000)).unwrap();
    let take = |source: CaptureSource| {
        let mut player = Command::new("paplay").arg("--device=voltip_echo_test").arg(&wav).spawn().expect("paplay");
        // paplay needs a moment to connect before the voice reaches the monitor.
        std::thread::sleep(Duration::from_millis(400));
        let config = RecorderConfig { source, ..RecorderConfig::default() };
        let recorder = Recorder::start_with(&backend, config, |_| {}).expect("a mixed take can be recorded here");
        std::thread::sleep(Duration::from_secs(6));
        let recording = recorder.stop().unwrap();
        let _ = player.wait();
        // Past the canceller's convergence: the last second and a half.
        let tail = &recording.samples[recording.samples.len().saturating_sub(24_000)..];
        (recording.duration_ms, rms_dbfs(tail))
    };
    let mixed = |echo_cancel| CaptureSource::Mixed { output_id: Some(sink.id.clone()), echo_cancel };
    let (_, computer) = take(CaptureSource::System { output_id: Some(sink.id.clone()) });
    let (plain_ms, plain) = take(mixed(false));
    let (cancelled_ms, cancelled) = take(mixed(true));
    let _ = std::fs::remove_file(&wav);
    println!(
        "the computer's sound {computer:.1} dBFS; mixed with the microphone's copy {plain:.1} dBFS as captured, {cancelled:.1} dBFS with the echo cancelled"
    );
    // A mixed take is as long as the microphone's stream: through the ALSA plugin of PulseAudio
    // 16 on a headless machine it delivers about 60 % of real time (2026-10-01), whatever the
    // source.
    assert!(plain_ms >= 3_000 && cancelled_ms >= 3_000, "{plain_ms} ms, {cancelled_ms} ms");
    assert!(computer > -40.0, "the voice is in the take: {computer:.1} dBFS");
    // Each side goes into the mix at −3 dB. As captured, the microphone's copy brings the take
    // back to about the computer's own level (2026-10-01 on PulseAudio 16: 0.5 dB below it);
    // cancelled, only the computer's sound is left, 3 dB below (measured 3.4 dB). Separate takes
    // of the same voice differ by about half a decibel, hence the margins.
    assert!(plain - computer >= -1.5, "the microphone hears the speakers: {plain:.1} dBFS as captured, {computer:.1} dBFS alone");
    assert!(computer - cancelled >= 2.0, "the echo is cancelled: {cancelled:.1} dBFS cancelled, {computer:.1} dBFS alone");
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
#[ignore = "plays a tone through the default output and records it"]
fn the_computers_sound_is_recorded_from_the_default_output() {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let host = cpal::default_host();
    let device = host.default_output_device().expect("an output device");
    let config = device.default_output_config().unwrap();
    assert_eq!(config.sample_format(), cpal::SampleFormat::F32, "the test plays f32");
    let (rate, channels) = (config.sample_rate() as f32, usize::from(config.channels()));
    // What plays: a recording when one is given (16 kHz mono, linearly resampled), else the tone.
    let speech = std::env::var_os("VOLTIP_LOOPBACK_PLAY").map(|path| wav_samples(&std::fs::read(path).expect("VOLTIP_LOOPBACK_PLAY")));
    let length = speech.as_ref().map_or(RECORD, |s| Duration::from_millis(s.len() as u64 * 1000 / 16_000 + 800));
    let mut n = 0u64;
    let stream = device
        .build_output_stream(
            config.into(),
            move |data: &mut [f32], _| {
                for frame in data.chunks_mut(channels) {
                    let t = n as f32 / rate;
                    let v = match &speech {
                        Some(samples) => {
                            let at = t * 16_000.0;
                            let (i, frac) = (at as usize, at.fract());
                            let a = samples.get(i).copied().unwrap_or(0.0);
                            let b = samples.get(i + 1).copied().unwrap_or(0.0);
                            a + (b - a) * frac
                        }
                        None => (t * TONE_HZ * std::f32::consts::TAU).sin() * AMPLITUDE,
                    };
                    frame.fill(v);
                    n += 1;
                }
            },
            |e| eprintln!("output stream: {e}"),
            None,
        )
        .unwrap();
    let backend = CpalBackend::new();
    // The recorder opens first, so the start of a recording played is not lost.
    let config = RecorderConfig { source: CaptureSource::System { output_id: None }, ..RecorderConfig::default() };
    let recorder = Recorder::start_with(&backend, config, |_| {}).expect("the computer's sound can be recorded here");
    stream.play().unwrap();
    std::thread::sleep(length);
    let recording = recorder.stop().unwrap();
    drop(stream);
    let level = rms_dbfs(&recording.samples);
    println!("loopback on the default output: {} ms at {level:.1} dBFS", recording.duration_ms);
    if let Some(out) = std::env::var_os("VOLTIP_LOOPBACK_OUT") {
        std::fs::write(&out, recording.to_wav()).unwrap();
        println!("recording written to {}", std::path::Path::new(&out).display());
    }
    assert!(recording.duration_ms >= 1_500, "{} ms", recording.duration_ms);
    assert!(level > -40.0, "the sound is in the recording: {level:.1} dBFS");
}

/// The samples of a 16-bit mono WAV as `-1.0..=1.0` (the data after the canonical 44-byte header).
#[cfg(any(windows, target_os = "macos"))]
fn wav_samples(wav: &[u8]) -> Vec<f32> {
    let data = wav.windows(4).position(|w| w == b"data").map_or(44, |at| at + 8);
    wav[data..].as_chunks::<2>().0.iter().map(|b| dsp::i16_to_f32(i16::from_le_bytes(*b))).collect()
}
