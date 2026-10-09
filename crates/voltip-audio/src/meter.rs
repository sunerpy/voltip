//! The live input level meter: one open stream, one [`FrameAccumulator`], one sink.

use serde::{Deserialize, Serialize};

use crate::AudioError;
use crate::backend::{AudioDevice, Backend, CpalBackend, SampleCallback, StreamHandle};
use crate::dsp::FrameAccumulator;

/// Default [`MeterConfig::frames_per_second`].
pub const DEFAULT_FRAMES_PER_SECOND: u16 = 30;
/// Default [`MeterConfig::peak_hold_ms`].
pub const DEFAULT_PEAK_HOLD_MS: u32 = 800;

/// One reading of the meter, emitted `frames_per_second` times per second from the audio thread.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LevelFrame {
    /// RMS level of this frame over all channels, `-90.0..` dBFS (`-90` is the silence floor).
    pub rms_dbfs: f32,
    /// Peak-hold level: the loudest sample of the last [`MeterConfig::peak_hold_ms`], falling at
    /// 20 dB/s once the hold expires. `0.0` is full scale; float sources can exceed it.
    pub peak_dbfs: f32,
    /// At least one sample in this frame reached [`crate::dsp::CLIP_THRESHOLD`].
    pub clipping: bool,
    /// Sample rate of the open stream.
    pub sample_rate_hz: u32,
    /// Interleaved channel count of the open stream.
    pub channels: u16,
    /// Frame counter starting at `0`; increases by one per frame, so a gap means the UI missed one.
    pub seq: u64,
}

/// How to run a [`Meter`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MeterConfig {
    /// [`AudioDevice::id`] to capture from; `None` follows the host's default input device.
    pub device_id: Option<String>,
    /// Frames emitted per second (`0` is treated as `1`).
    pub frames_per_second: u16,
    /// How long the peak reading holds its maximum before it starts to fall.
    pub peak_hold_ms: u32,
}

impl Default for MeterConfig {
    fn default() -> Self {
        Self { device_id: None, frames_per_second: DEFAULT_FRAMES_PER_SECOND, peak_hold_ms: DEFAULT_PEAK_HOLD_MS }
    }
}

/// A running level meter. Dropping it stops capture and releases the device.
pub struct Meter {
    device: AudioDevice,
    stream: Option<Box<dyn StreamHandle>>,
}

impl Meter {
    /// Open the configured device through the platform backend and start delivering
    /// [`LevelFrame`]s to `sink` from the audio thread. `sink` must not block; hand the frame to
    /// a channel and return.
    pub fn start(config: MeterConfig, sink: impl Fn(LevelFrame) + Send + 'static) -> Result<Self, AudioError> {
        Self::start_with(&CpalBackend::new(), config, sink)
    }

    /// [`Meter::start`] against an explicit [`Backend`].
    pub fn start_with(backend: &dyn Backend, config: MeterConfig, sink: impl Fn(LevelFrame) + Send + 'static) -> Result<Self, AudioError> {
        let devices = backend.input_devices()?;
        let device = match &config.device_id {
            Some(id) => devices.iter().find(|d| &d.id == id).cloned().ok_or_else(|| AudioError::DeviceNotFound(id.clone()))?,
            None => devices.iter().find(|d| d.is_default).or_else(|| devices.first()).cloned().ok_or(AudioError::NoDevice)?,
        };
        let frames_per_second = config.frames_per_second;
        let peak_hold_ms = config.peak_hold_ms;
        // The stream's real rate and channel count arrive with the first chunk, so the
        // accumulator is built there. It holds no heap data, so this is not an allocation.
        let mut accumulator: Option<FrameAccumulator> = None;
        let on_samples: SampleCallback = Box::new(move |chunk, rate, channels| {
            let acc = accumulator.get_or_insert_with(|| FrameAccumulator::new(rate, channels, frames_per_second, peak_hold_ms));
            acc.push(chunk, &sink);
        });
        let stream = backend.open_input(config.device_id.as_deref(), on_samples)?;
        tracing::info!(device = %device.id, name = %device.name, frames_per_second, peak_hold_ms, "level meter started");
        Ok(Self { device, stream: Some(stream) })
    }

    /// The device the meter is capturing from.
    pub fn device(&self) -> &AudioDevice {
        &self.device
    }
}

impl Drop for Meter {
    fn drop(&mut self) {
        // Dropping the handle stops the stream and releases the device.
        drop(self.stream.take());
        tracing::info!(device = %self.device.id, "level meter stopped");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;
    use crate::dsp::DBFS_FLOOR;
    use crate::fake::{FakeBackend, FakeFormat, Signal};

    const RECV_TIMEOUT: Duration = Duration::from_secs(5);

    fn frames(rx: &mpsc::Receiver<LevelFrame>, n: usize) -> Vec<LevelFrame> {
        (0..n).map(|_| rx.recv_timeout(RECV_TIMEOUT).expect("frame within timeout")).collect()
    }

    fn close(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn config_defaults_and_serde() {
        let config = MeterConfig::default();
        assert_eq!(config, MeterConfig { device_id: None, frames_per_second: 30, peak_hold_ms: 800 });
        let parsed: MeterConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(parsed, config);
        let parsed: MeterConfig = serde_json::from_str(r#"{"device_id":"alsa:default","frames_per_second":10}"#).unwrap();
        assert_eq!(parsed, MeterConfig { device_id: Some("alsa:default".into()), frames_per_second: 10, peak_hold_ms: 800 });
        let frame = LevelFrame { rms_dbfs: -20.5, peak_dbfs: -3.0, clipping: true, sample_rate_hz: 48_000, channels: 2, seq: 7 };
        let json = serde_json::to_string(&frame).unwrap();
        assert_eq!(serde_json::from_str::<LevelFrame>(&json).unwrap(), frame);
    }

    #[test]
    fn meter_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Meter>();
    }

    #[test]
    fn sine_on_default_device_yields_expected_levels() {
        let backend = FakeBackend::new().with_signal(Signal::Sine { frequency_hz: 1000.0, amplitude: 0.5 });
        let (tx, rx) = mpsc::channel();
        let meter = Meter::start_with(&backend, MeterConfig::default(), move |f| {
            let _ = tx.send(f);
        })
        .unwrap();
        assert_eq!(meter.device().id, "fake:default");
        assert!(meter.device().is_default);
        assert_eq!(backend.opened_with(), vec![None]);

        let got = frames(&rx, 10);
        assert_eq!(got.iter().map(|f| f.seq).collect::<Vec<_>>(), (0..10).collect::<Vec<_>>());
        for f in &got {
            assert_eq!(f.sample_rate_hz, 48_000);
            assert_eq!(f.channels, 2);
            // A 3200-sample frame holds 33⅓ cycles, so the RMS wobbles a few hundredths of a dB.
            assert!(close(f.rms_dbfs, -9.03, 0.1), "rms {}", f.rms_dbfs);
            assert!(close(f.peak_dbfs, -6.02, 0.05), "peak {}", f.peak_dbfs);
            assert!(!f.clipping);
        }
        assert!(backend.is_running());
        drop(meter);
        assert!(!backend.is_running(), "drop must stop the fake stream");
    }

    #[test]
    fn silence_and_full_scale_clip_on_a_named_i16_device() {
        let backend = FakeBackend::new().with_format(FakeFormat::I16).with_rate(16_000, 1).with_signal(Signal::Silence);
        let (tx, rx) = mpsc::channel();
        let config = MeterConfig { device_id: Some("fake:usb-mic".into()), frames_per_second: 20, peak_hold_ms: 0 };
        let meter = Meter::start_with(&backend, config, move |f| {
            let _ = tx.send(f);
        })
        .unwrap();
        assert_eq!(meter.device().name, "Fake USB Microphone");
        assert!(!meter.device().is_default);
        assert_eq!(backend.opened_with(), vec![Some("fake:usb-mic".to_string())]);
        let got = frames(&rx, 3);
        for f in &got {
            assert_eq!(f.sample_rate_hz, 16_000);
            assert_eq!(f.channels, 1);
            assert_eq!(f.rms_dbfs, DBFS_FLOOR);
            assert_eq!(f.peak_dbfs, DBFS_FLOOR);
            assert!(!f.clipping);
        }
        drop(meter);

        let backend = FakeBackend::new().with_format(FakeFormat::U16).with_signal(Signal::Constant(1.0));
        let (tx, rx) = mpsc::channel();
        let _meter = Meter::start_with(&backend, MeterConfig::default(), move |f| {
            let _ = tx.send(f);
        })
        .unwrap();
        let got = frames(&rx, 2);
        assert!(got.iter().all(|f| f.clipping));
        assert!(got.iter().all(|f| close(f.peak_dbfs, 0.0, 1e-3)));

        let backend = FakeBackend::new().with_format(FakeFormat::I32).with_signal(Signal::Constant(-0.25));
        let (tx, rx) = mpsc::channel();
        let _meter = Meter::start_with(&backend, MeterConfig::default(), move |f| {
            let _ = tx.send(f);
        })
        .unwrap();
        let got = frames(&rx, 2);
        assert!(got.iter().all(|f| close(f.rms_dbfs, -12.04, 1e-2) && !f.clipping), "{got:?}");
    }

    #[test]
    fn errors_from_device_selection_and_backend() {
        let backend = FakeBackend::new();
        let unknown = MeterConfig { device_id: Some("fake:nope".into()), ..MeterConfig::default() };
        let err = Meter::start_with(&backend, unknown, |_| {}).map(drop).unwrap_err();
        assert_eq!(err, AudioError::DeviceNotFound("fake:nope".into()));
        assert!(backend.opened_with().is_empty(), "nothing is opened for an unknown id");

        let empty = FakeBackend::new().without_devices();
        let err = Meter::start_with(&empty, MeterConfig::default(), |_| {}).map(drop).unwrap_err();
        assert_eq!(err, AudioError::NoDevice);

        let failing = FakeBackend::new().failing_open(AudioError::Backend("device busy".into()));
        let err = Meter::start_with(&failing, MeterConfig::default(), |_| {}).map(drop).unwrap_err();
        assert_eq!(err, AudioError::Backend("device busy".into()));

        let broken = FakeBackend::new().failing_enumeration(AudioError::Backend("host down".into()));
        let err = Meter::start_with(&broken, MeterConfig::default(), |_| {}).map(drop).unwrap_err();
        assert_eq!(err, AudioError::Backend("host down".into()));
    }

    #[test]
    fn no_default_flag_falls_back_to_first_device() {
        let backend = FakeBackend::new().without_default();
        let (tx, rx) = mpsc::channel();
        let meter = Meter::start_with(&backend, MeterConfig::default(), move |f| {
            let _ = tx.send(f);
        })
        .unwrap();
        assert_eq!(meter.device().id, "fake:default");
        assert!(!meter.device().is_default);
        assert_eq!(frames(&rx, 1)[0].seq, 0);
    }
}
