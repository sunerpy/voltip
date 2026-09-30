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
//!   sink's monitor through the recorder and removes the sink again; nothing is heard.
//! - Windows and macOS (14.6 or later; macOS asks the terminal for the permission to record other
//!   apps' audio the first time): the tone plays through the default output with cpal, at a
//!   quarter of full scale for about three seconds, and the recorder records that output.

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

/// The recording of `source` while the tone plays; the tone starts before and ends after it.
fn record(backend: &CpalBackend, source: CaptureSource) -> voltip_audio::Recording {
    let config = RecorderConfig { source, ..RecorderConfig::default() };
    let recorder = Recorder::start_with(backend, config, |_| {}).expect("the computer's sound can be recorded here");
    // The recording's own length: the tone plays meanwhile.
    std::thread::sleep(RECORD);
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
    let recording = record(&backend, CaptureSource::System { output_id: Some(sink.id.clone()) });
    let _ = player.wait();
    let _ = std::fs::remove_file(&wav);
    let level = rms_dbfs(&recording.samples);
    println!("loopback on {}: {} ms at {level:.1} dBFS (tone at {:.1} dBFS RMS)", sink.name, recording.duration_ms, dsp::to_dbfs(AMPLITUDE / 2f32.sqrt()));
    assert!(recording.duration_ms >= 1_500, "{} ms", recording.duration_ms);
    assert!(level > -30.0, "the tone is in the recording: {level:.1} dBFS");
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
    let mut n = 0u64;
    let stream = device
        .build_output_stream(
            config.into(),
            move |data: &mut [f32], _| {
                for frame in data.chunks_mut(channels) {
                    let v = (n as f32 / rate * TONE_HZ * std::f32::consts::TAU).sin() * AMPLITUDE;
                    frame.fill(v);
                    n += 1;
                }
            },
            |e| eprintln!("output stream: {e}"),
            None,
        )
        .unwrap();
    stream.play().unwrap();
    std::thread::sleep(Duration::from_millis(400));
    let backend = CpalBackend::new();
    let recording = record(&backend, CaptureSource::System { output_id: None });
    drop(stream);
    let level = rms_dbfs(&recording.samples);
    println!("loopback on the default output: {} ms at {level:.1} dBFS", recording.duration_ms);
    assert!(recording.duration_ms >= 1_500, "{} ms", recording.duration_ms);
    assert!(level > -30.0, "the tone is in the recording: {level:.1} dBFS");
}
