//! The seam between the meter and the sound system.
//!
//! [`Backend`] is what [`crate::Meter`] talks to. [`CpalBackend`] is the real one (WASAPI on
//! Windows, CoreAudio on macOS, ALSA on Linux); the `test-support` feature adds
//! [`crate::FakeBackend`], which plays a synthetic signal so the whole meter path runs on a host
//! without a microphone.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{ErrorKind, SampleFormat, StreamConfig};
use serde::{Deserialize, Serialize};

use crate::AudioError;
use crate::dsp::SampleChunk;

/// A capture device as the UI lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioDevice {
    /// Stable handle to pass back in [`crate::MeterConfig::device_id`]. It is cpal's
    /// [`DeviceId`](cpal::DeviceId) rendered as `host:identifier` (the WASAPI endpoint id, the
    /// CoreAudio device UID, the ALSA PCM name), which survives restarts and re-plugging.
    pub id: String,
    /// Human-readable name as the operating system reports it.
    pub name: String,
    /// Whether the host currently routes default input to this device.
    pub is_default: bool,
    /// Sample rate of the device's default input configuration, if it reported one.
    pub sample_rate_hz: Option<u32>,
    /// Channel count of the device's default input configuration, if it reported one.
    pub channels: Option<u16>,
}

/// A running input stream. Dropping the handle stops capture and releases the device.
pub trait StreamHandle: Send + Sync {}

/// Called from the audio thread with each captured chunk, the stream's sample rate and its
/// interleaved channel count. Must not allocate or block.
pub type SampleCallback = Box<dyn FnMut(SampleChunk<'_>, u32, u16) + Send>;

/// What the meter needs from a sound system.
pub trait Backend {
    /// Input devices the meter can open, default device first. An empty list is not an error.
    fn input_devices(&self) -> Result<Vec<AudioDevice>, AudioError>;
    /// Id of the host's current default input device, if it has one.
    fn default_input(&self) -> Option<String>;
    /// Start capturing from `id` (or from the default device when `None`) in the device's default
    /// input configuration, delivering samples to `on_samples`.
    fn open_input(&self, id: Option<&str>, on_samples: SampleCallback) -> Result<Box<dyn StreamHandle>, AudioError>;
}

/// The real backend: whatever [`cpal::default_host`] resolves to on this platform.
pub struct CpalBackend {
    host: cpal::Host,
}

impl CpalBackend {
    /// Bind to the platform's default host.
    pub fn new() -> Self {
        Self { host: cpal::default_host() }
    }

    fn find_input(&self, id: &str) -> Result<cpal::Device, AudioError> {
        let devices = self.host.input_devices().map_err(|e| map_cpal(&e, id))?;
        for device in devices {
            if device.id().is_ok_and(|did| did.to_string() == id) {
                return Ok(device);
            }
        }
        Err(AudioError::DeviceNotFound(id.to_string()))
    }
}

impl Default for CpalBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for CpalBackend {
    fn input_devices(&self) -> Result<Vec<AudioDevice>, AudioError> {
        let default_id = self.default_input();
        let devices = match self.host.input_devices() {
            Ok(devices) => devices,
            // A machine without a sound server or without any capture endpoint has no devices;
            // that is a state the UI shows, not a failure.
            Err(e) if matches!(e.kind(), ErrorKind::HostUnavailable | ErrorKind::DeviceNotAvailable) => return Ok(Vec::new()),
            Err(e) => return Err(AudioError::Backend(e.to_string())),
        };
        let mut out = Vec::new();
        for device in devices {
            // A device that vanished between enumeration and inspection is simply skipped.
            let Ok(id) = device.id() else { continue };
            let id = id.to_string();
            // `open_input` captures in the default input configuration, so a device without one
            // (ALSA lists rate-converter and mixer plugins as PCMs) cannot be metered; leave it out.
            let Ok(config) = device.default_input_config() else { continue };
            let name = device.description().map(|d| d.name().to_string()).unwrap_or_else(|_| device.to_string());
            let is_default = default_id.as_deref() == Some(id.as_str());
            out.push(AudioDevice { id, name, is_default, sample_rate_hz: Some(config.sample_rate()), channels: Some(config.channels()) });
        }
        sort_default_first(&mut out);
        Ok(out)
    }

    fn default_input(&self) -> Option<String> {
        self.host.default_input_device().and_then(|d| d.id().ok()).map(|id| id.to_string())
    }

    fn open_input(&self, id: Option<&str>, mut on_samples: SampleCallback) -> Result<Box<dyn StreamHandle>, AudioError> {
        let device = match id {
            Some(id) => self.find_input(id)?,
            None => self.host.default_input_device().ok_or(AudioError::NoDevice)?,
        };
        let label = id.map(str::to_string).unwrap_or_else(|| device.to_string());
        let supported = device.default_input_config().map_err(|e| map_cpal(&e, &label))?;
        let format = supported.sample_format();
        let config: StreamConfig = supported.config();
        let rate = config.sample_rate;
        let channels = config.channels;
        tracing::debug!(device = %label, rate, channels, ?format, "opening input stream");
        let on_error = move |e: cpal::Error| tracing::warn!(error = %e, kind = ?e.kind(), "input stream error");
        let built = match format {
            SampleFormat::F32 => {
                device.build_input_stream::<f32, _, _>(config, move |data, _| on_samples(SampleChunk::F32(data), rate, channels), on_error, None)
            }
            SampleFormat::I16 => {
                device.build_input_stream::<i16, _, _>(config, move |data, _| on_samples(SampleChunk::I16(data), rate, channels), on_error, None)
            }
            SampleFormat::U16 => {
                device.build_input_stream::<u16, _, _>(config, move |data, _| on_samples(SampleChunk::U16(data), rate, channels), on_error, None)
            }
            SampleFormat::I32 => {
                device.build_input_stream::<i32, _, _>(config, move |data, _| on_samples(SampleChunk::I32(data), rate, channels), on_error, None)
            }
            other => return Err(AudioError::UnsupportedFormat(format!("{other:?} on {label}"))),
        };
        let stream = built.map_err(|e| map_cpal(&e, &label))?;
        stream.play().map_err(|e| map_cpal(&e, &label))?;
        Ok(Box::new(CpalStream { _stream: stream }))
    }
}

struct CpalStream {
    _stream: cpal::Stream,
}

impl StreamHandle for CpalStream {}

/// Stable partition: the default device moves to the front, everything else keeps the host's
/// enumeration order.
pub(crate) fn sort_default_first(devices: &mut [AudioDevice]) {
    devices.sort_by_key(|d| !d.is_default);
}

/// Translate a cpal error about `device` into the crate's vocabulary.
pub(crate) fn map_cpal(error: &cpal::Error, device: &str) -> AudioError {
    match error.kind() {
        ErrorKind::DeviceNotAvailable => AudioError::DeviceNotFound(device.to_string()),
        ErrorKind::UnsupportedConfig | ErrorKind::UnsupportedOperation => AudioError::UnsupportedFormat(format!("{error} on {device}")),
        _ => AudioError::Backend(format!("{error} on {device}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(id: &str, is_default: bool) -> AudioDevice {
        AudioDevice { id: id.to_string(), name: id.to_uppercase(), is_default, sample_rate_hz: None, channels: None }
    }

    #[test]
    fn default_device_moves_first_and_order_is_otherwise_stable() {
        let mut devices = vec![dev("a", false), dev("b", false), dev("c", true), dev("d", false)];
        sort_default_first(&mut devices);
        let ids: Vec<&str> = devices.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, ["c", "a", "b", "d"]);
        let mut none_default = vec![dev("x", false), dev("y", false)];
        sort_default_first(&mut none_default);
        assert_eq!(none_default[0].id, "x");
    }

    #[test]
    fn cpal_errors_map_to_crate_errors() {
        let gone = cpal::Error::new(ErrorKind::DeviceNotAvailable);
        assert_eq!(map_cpal(&gone, "mic-1"), AudioError::DeviceNotFound("mic-1".into()));
        let unsupported = cpal::Error::with_message(ErrorKind::UnsupportedConfig, "no 7.1 capture");
        assert_eq!(map_cpal(&unsupported, "mic-1"), AudioError::UnsupportedFormat("no 7.1 capture on mic-1".into()));
        let op = cpal::Error::new(ErrorKind::UnsupportedOperation);
        assert!(matches!(map_cpal(&op, "m"), AudioError::UnsupportedFormat(_)));
        let busy = cpal::Error::with_message(ErrorKind::DeviceBusy, "in use");
        assert_eq!(map_cpal(&busy, "mic-1"), AudioError::Backend("in use on mic-1".into()));
    }

    #[test]
    fn audio_device_round_trips_through_serde() {
        let device = AudioDevice { id: "alsa:default".into(), name: "Default".into(), is_default: true, sample_rate_hz: Some(48_000), channels: Some(2) };
        let json = serde_json::to_string(&device).unwrap();
        assert!(json.contains("\"is_default\":true"), "{json}");
        assert_eq!(serde_json::from_str::<AudioDevice>(&json).unwrap(), device);
    }

    /// Runs against the real host. CI and this container usually have no capture device, so the
    /// contract is only: no panic, no error, and when there are devices the default is first.
    #[test]
    fn real_backend_enumerates_without_panicking() {
        let backend = CpalBackend::default();
        let devices = backend.input_devices().expect("enumeration must not fail without devices");
        let default = backend.default_input();
        eprintln!("real backend: default={default:?} devices={devices:#?}");
        if let Some(first) = devices.first() {
            assert!(!first.id.is_empty());
            assert!(!first.name.is_empty());
            if devices.iter().any(|d| d.is_default) {
                assert!(first.is_default, "default device must be listed first");
                assert_eq!(default.as_deref(), Some(first.id.as_str()));
            }
        }
        assert!(devices.iter().filter(|d| d.is_default).count() <= 1);
    }

    /// Opening the real default device is allowed to fail on a headless host (no card behind ALSA's
    /// `default`), but it must fail with a typed error, not a panic, and a success must be
    /// droppable without hanging. On a host without any sound card (CI container, 2026-09-25) ALSA
    /// still names `default`, and opening it anonymously *and* by id both report `DeviceNotFound`;
    /// the invariant is consistency between the two paths, not that a device exists.
    #[test]
    fn real_backend_open_default_is_ok_or_typed_error() {
        let backend = CpalBackend::new();
        let Some(default) = backend.default_input() else { return };
        let anonymous_missing = match backend.open_input(None, Box::new(|_, _, _| {})) {
            Ok(handle) => {
                drop(handle);
                false
            }
            Err(err) => {
                eprintln!("real backend: opening {default} failed as expected on a headless host: {err}");
                matches!(err, AudioError::DeviceNotFound(_))
            }
        };
        let by_id = backend.open_input(Some(&default), Box::new(|_, _, _| {}));
        match by_id {
            Ok(handle) => drop(handle),
            Err(AudioError::DeviceNotFound(id)) => {
                assert!(anonymous_missing, "the default id {id} must resolve when the anonymous default opens or fails for another reason");
            }
            Err(err) => eprintln!("real backend: opening {default} by id failed with a typed error: {err}"),
        }
    }

    #[test]
    fn real_backend_refuses_unknown_device_and_reports_missing_default() {
        let backend = CpalBackend::new();
        let missing = backend.open_input(Some("voltip-test:this-device-does-not-exist"), Box::new(|_, _, _| {})).map(drop);
        assert!(matches!(&missing, Err(AudioError::DeviceNotFound(id)) if id.contains("does-not-exist")), "{missing:?}");
        if backend.default_input().is_none() {
            let none = backend.open_input(None, Box::new(|_, _, _| {})).map(drop);
            assert!(matches!(none, Err(AudioError::NoDevice)), "{none:?}");
        }
    }
}
