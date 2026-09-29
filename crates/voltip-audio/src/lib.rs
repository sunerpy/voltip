//! Microphone enumeration, a live input level meter and the dictation recorder for the shells.
//!
//! * [`list_input_devices`] — capture devices as the settings page lists them, default first.
//! * [`Meter`] — opens one device through cpal (WASAPI on Windows, CoreAudio on macOS, ALSA on
//!   Linux) and pushes a [`LevelFrame`] to a sink `frames_per_second` times per second from the
//!   audio thread. The shell forwards frames over a channel to the UI; dropping the meter stops
//!   capture.
//! * [`Recorder`] — the same stream, but it also keeps the audio: any native format and channel
//!   count is downmixed to mono and resampled (rubato) to [`RecorderConfig::target_rate_hz`],
//!   capped at [`RecorderConfig::max_duration`]. `stop()` yields a [`Recording`] that knows how
//!   to serialise itself as WAV and whether it is worth uploading. With
//!   [`RecorderConfig::live_tap`] it also feeds a lock-free 16 kHz mono tap ([`live`]) for the
//!   streaming preview, and it reports the first delivered samples through `on_ready`.
//! * [`dsp`] — the arithmetic (RMS, peak, dBFS, peak hold, frame cadence), hardware-free.
//! * [`recording`] — downmix, resampling and WAV framing, hardware-free.
//! * [`Backend`] — the seam between the meter and the sound system. [`CpalBackend`] is the real
//!   one; [`FakeBackend`] (feature `test-support`, always on for this crate's tests) plays a
//!   synthetic signal so the whole path runs on a host without a microphone.
//!
//! The desktop shell records its takes with it; the mobile shell records the takes the phone
//! streams to a paired desktop (docs/dictation.md §20; cpal opens AAudio on Android).

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod backend;
pub mod dsp;
#[cfg(any(test, feature = "test-support"))]
mod fake;
pub mod live;
mod meter;
pub mod mix;
pub mod pcm;
mod recorder;
pub mod recording;

pub use backend::{AudioDevice, Backend, CpalBackend, SampleCallback, StreamHandle, SystemAudio};
pub use dsp::SampleChunk;
#[cfg(any(test, feature = "test-support"))]
pub use fake::{FAKE_DEFAULT_ID, FAKE_SPEAKERS_ID, FAKE_USB_ID, FakeBackend, FakeFormat, Signal};
pub use live::{DEFAULT_LIVE_BUFFER_MS, DEFAULT_LIVE_RATE_HZ, LiveConsumer, LiveTapConfig, StreamResampler};
pub use meter::{DEFAULT_FRAMES_PER_SECOND, DEFAULT_PEAK_HOLD_MS, LevelFrame, Meter, MeterConfig};
pub use pcm::{DEFAULT_PCM_BUFFER_MS, DEFAULT_PCM_RATE_HZ, PcmConsumer, PcmStreamConfig};
pub use recorder::{DEFAULT_MAX_DURATION, DEFAULT_TARGET_RATE_HZ, Recorder, RecorderConfig};
pub use recording::{MIN_SPEECH_MS, Recording, SILENCE_PEAK_DBFS};

/// Errors from the audio layer.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AudioError {
    /// The host has no input device at all.
    #[error("no audio input device available")]
    NoDevice,
    /// The requested [`AudioDevice::id`] is not (or no longer) present.
    #[error("audio input device not found: {0}")]
    DeviceNotFound(String),
    /// The sound system refused (device busy, permission denied, host gone, ...).
    #[error("audio backend: {0}")]
    Backend(String),
    /// The device's native sample format is one the meter does not decode.
    #[error("unsupported audio input format: {0}")]
    UnsupportedFormat(String),
    /// The recorder could not convert the device's rate to the requested one.
    #[error("resample: {0}")]
    Resample(String),
    /// The computer's sound cannot be recorded here (docs/dictation.md §22); the reason.
    #[error("system audio capture unavailable: {0}")]
    SystemAudioUnavailable(String),
}

/// Capture devices as the settings page lists them: the default device first, then the rest in
/// the host's enumeration order. A host without any capture device yields `Ok(vec![])`.
pub fn list_input_devices() -> Result<Vec<AudioDevice>, AudioError> {
    CpalBackend::new().input_devices()
}

/// Output devices whose sound can be recorded (docs/dictation.md §22), default first, and whether
/// the computer's sound can be recorded here at all.
pub fn list_output_devices() -> (SystemAudio, Result<Vec<AudioDevice>, AudioError>) {
    let backend = CpalBackend::new();
    (backend.system_audio(), backend.output_devices())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_messages_are_plain_english() {
        assert_eq!(AudioError::NoDevice.to_string(), "no audio input device available");
        assert_eq!(AudioError::DeviceNotFound("wasapi:{0.0.1.0}".into()).to_string(), "audio input device not found: wasapi:{0.0.1.0}");
        assert_eq!(AudioError::Backend("busy".into()).to_string(), "audio backend: busy");
        assert_eq!(AudioError::UnsupportedFormat("DsdU8".into()).to_string(), "unsupported audio input format: DsdU8");
        assert_eq!(AudioError::Resample("0 Hz".into()).to_string(), "resample: 0 Hz");
    }

    /// Real host: like the meter, the recorder may fail to open a device here, but only typed.
    #[test]
    fn recorder_start_on_real_host_is_ok_or_typed_error() {
        match Recorder::start(RecorderConfig::default(), |_| {}) {
            Ok(recorder) => {
                assert!(!recorder.device().id.is_empty());
                let recording = recorder.stop().expect("stop converts whatever was captured");
                assert_eq!(recording.sample_rate_hz, DEFAULT_TARGET_RATE_HZ);
            }
            Err(err) => eprintln!("real host: recorder did not start: {err}"),
        }
    }

    /// Real host: starting the meter may fail on a headless machine, but only with a typed error.
    #[test]
    fn meter_start_on_real_host_is_ok_or_typed_error() {
        match Meter::start(MeterConfig::default(), |_| {}) {
            Ok(meter) => {
                assert!(!meter.device().id.is_empty());
                drop(meter);
            }
            Err(err) => eprintln!("real host: meter did not start: {err}"),
        }
    }

    /// Real host, no assumptions about hardware: it must not fail or panic when there is nothing
    /// to list, and when there is, the default device leads.
    #[test]
    fn list_input_devices_works_without_hardware() {
        let devices = list_input_devices().expect("empty list, not an error");
        if devices.iter().any(|d| d.is_default) {
            assert!(devices[0].is_default);
        }
    }
}
