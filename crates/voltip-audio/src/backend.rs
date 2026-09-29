//! The seam between the meter and the sound system.
//!
//! [`Backend`] is what [`crate::Meter`] talks to. [`CpalBackend`] is the real one (WASAPI on
//! Windows, CoreAudio on macOS, ALSA on Linux); the `test-support` feature adds
//! [`crate::FakeBackend`], which plays a synthetic signal so the whole meter path runs on a host
//! without a microphone.
//!
//! The computer's sound (docs/dictation.md §22) is recorded from an output device: WASAPI's
//! loopback on Windows (with a silent stream playing on the device, so the loopback keeps
//! delivering while nothing else plays and the recording keeps its timeline), a CoreAudio process
//! tap on macOS 14.6 and later (cpal builds it for an input stream on an output device), and the
//! output's monitor source of the PulseAudio server on Linux (PipeWire's `pipewire-pulse`
//! included). The microphone stays on ALSA there: with cpal's PulseAudio host compiled in, the
//! platform default would move to it and change every saved device id.

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

/// Whether this machine can record what it plays (docs/dictation.md §22), and why not.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SystemAudio {
    /// Recording the computer's sound works here.
    Available,
    /// macOS before 14.6 has no process tap to record the output with.
    MacosTooOld {
        /// The running version (`14.5`).
        version: String,
    },
    /// Linux without a PulseAudio server (PipeWire's `pipewire-pulse` counts): there is no monitor
    /// source to record the output from.
    NoSoundServer,
    /// This platform cannot record its own output.
    Unsupported,
}

impl SystemAudio {
    /// Whether recording the computer's sound works here.
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }

    /// The reason in plain English (logs and errors; the interface words it itself).
    pub fn describe(&self) -> String {
        match self {
            Self::Available => "available".to_owned(),
            Self::MacosTooOld { version } => format!("macOS {version} cannot record the computer's sound (14.6 or later can)"),
            Self::NoSoundServer => "no PulseAudio server is running, so the output cannot be recorded".to_owned(),
            Self::Unsupported => "this platform cannot record its own output".to_owned(),
        }
    }
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

    /// Output devices whose sound can be recorded, default first; empty when
    /// [`Backend::system_audio`] is not available.
    fn output_devices(&self) -> Result<Vec<AudioDevice>, AudioError> {
        Ok(Vec::new())
    }

    /// Id of the host's current default output device, if it has one.
    fn default_output(&self) -> Option<String> {
        None
    }

    /// Whether the computer's sound can be recorded here.
    fn system_audio(&self) -> SystemAudio {
        SystemAudio::Unsupported
    }

    /// Start recording what `id` (or the default output when `None`) plays, delivering samples to
    /// `on_samples` like [`Backend::open_input`].
    fn open_output_capture(&self, id: Option<&str>, on_samples: SampleCallback) -> Result<Box<dyn StreamHandle>, AudioError> {
        let _ = (id, on_samples);
        Err(AudioError::SystemAudioUnavailable(self.system_audio().describe()))
    }
}

/// The real backend: the platform's host for the microphone, and the host the computer's sound is
/// recorded through (see the module documentation).
pub struct CpalBackend {
    host: cpal::Host,
    /// The output host, or why the computer's sound cannot be recorded here.
    output: Result<cpal::Host, SystemAudio>,
}

impl CpalBackend {
    /// Bind to the platform's hosts.
    pub fn new() -> Self {
        Self { host: microphone_host(), output: output_host() }
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
        let stream = build_input(&device, supported, &label, &mut on_samples)?;
        Ok(Box::new(CpalStreams { _streams: vec![stream] }))
    }

    fn output_devices(&self) -> Result<Vec<AudioDevice>, AudioError> {
        let Ok(host) = &self.output else { return Ok(Vec::new()) };
        let default_id = self.default_output();
        let devices = match host.output_devices() {
            Ok(devices) => devices,
            Err(e) if matches!(e.kind(), ErrorKind::HostUnavailable | ErrorKind::DeviceNotAvailable) => return Ok(Vec::new()),
            Err(e) => return Err(AudioError::Backend(e.to_string())),
        };
        let mut out = Vec::new();
        for device in devices {
            let Ok(id) = device.id() else { continue };
            let id = id.to_string();
            let Ok(config) = device.default_output_config() else { continue };
            let name = device.description().map(|d| d.name().to_string()).unwrap_or_else(|_| device.to_string());
            let is_default = default_id.as_deref() == Some(id.as_str());
            out.push(AudioDevice { id, name, is_default, sample_rate_hz: Some(config.sample_rate()), channels: Some(config.channels()) });
        }
        sort_default_first(&mut out);
        Ok(out)
    }

    fn default_output(&self) -> Option<String> {
        self.output.as_ref().ok()?.default_output_device().and_then(|d| d.id().ok()).map(|id| id.to_string())
    }

    fn system_audio(&self) -> SystemAudio {
        match &self.output {
            Ok(_) => SystemAudio::Available,
            Err(why) => why.clone(),
        }
    }

    fn open_output_capture(&self, id: Option<&str>, mut on_samples: SampleCallback) -> Result<Box<dyn StreamHandle>, AudioError> {
        let host = self.output.as_ref().map_err(|why| AudioError::SystemAudioUnavailable(why.describe()))?;
        let output = match id {
            Some(id) => find_output(host, id)?,
            None => host.default_output_device().ok_or(AudioError::NoDevice)?,
        };
        let label = id.map(str::to_string).unwrap_or_else(|| output.to_string());
        let mut streams = Vec::with_capacity(2);
        #[cfg(target_os = "linux")]
        {
            let monitor = monitor_of(host, &output, &label)?;
            let supported = monitor.default_input_config().map_err(|e| map_cpal(&e, &label))?;
            streams.push(build_input(&monitor, supported, &label, &mut on_samples)?);
        }
        #[cfg(not(target_os = "linux"))]
        {
            let supported = output.default_output_config().map_err(|e| map_cpal(&e, &label))?;
            streams.push(build_input(&output, supported, &label, &mut on_samples)?);
        }
        #[cfg(target_os = "windows")]
        streams.push(silent_output(&output, &label)?);
        tracing::info!(device = %label, streams = streams.len(), "recording the computer's sound");
        Ok(Box::new(CpalStreams { _streams: streams }))
    }
}

/// The running streams of one capture (the computer's sound on Windows is two: the loopback and
/// the silence keeping it going).
struct CpalStreams {
    _streams: Vec<cpal::Stream>,
}

impl StreamHandle for CpalStreams {}

/// Build and start an input stream on `device` in `supported`, delivering to `on_samples`.
fn build_input(
    device: &cpal::Device,
    supported: cpal::SupportedStreamConfig,
    label: &str,
    on_samples: &mut SampleCallback,
) -> Result<cpal::Stream, AudioError> {
    let format = supported.sample_format();
    let config: StreamConfig = supported.config();
    let rate = config.sample_rate;
    let channels = config.channels;
    tracing::debug!(device = %label, rate, channels, ?format, "opening input stream");
    let mut on_samples = std::mem::replace(on_samples, Box::new(|_, _, _| {}));
    let on_error = move |e: cpal::Error| tracing::warn!(error = %e, kind = ?e.kind(), "input stream error");
    let built = match format {
        SampleFormat::F32 => device.build_input_stream::<f32, _, _>(config, move |data, _| on_samples(SampleChunk::F32(data), rate, channels), on_error, None),
        SampleFormat::I16 => device.build_input_stream::<i16, _, _>(config, move |data, _| on_samples(SampleChunk::I16(data), rate, channels), on_error, None),
        SampleFormat::U16 => device.build_input_stream::<u16, _, _>(config, move |data, _| on_samples(SampleChunk::U16(data), rate, channels), on_error, None),
        SampleFormat::I32 => device.build_input_stream::<i32, _, _>(config, move |data, _| on_samples(SampleChunk::I32(data), rate, channels), on_error, None),
        other => return Err(AudioError::UnsupportedFormat(format!("{other:?} on {label}"))),
    };
    let stream = built.map_err(|e| map_cpal(&e, label))?;
    stream.play().map_err(|e| map_cpal(&e, label))?;
    Ok(stream)
}

/// The output device `id` among `host`'s.
fn find_output(host: &cpal::Host, id: &str) -> Result<cpal::Device, AudioError> {
    let devices = host.output_devices().map_err(|e| map_cpal(&e, id))?;
    for device in devices {
        if device.id().is_ok_and(|did| did.to_string() == id) {
            return Ok(device);
        }
    }
    Err(AudioError::DeviceNotFound(id.to_string()))
}

/// The microphone's host: the platform default, except on Linux, where that would be PulseAudio
/// (compiled in for the computer's sound) — the microphone stays on ALSA, so a saved device id
/// keeps naming the same device.
fn microphone_host() -> cpal::Host {
    #[cfg(target_os = "linux")]
    if let Ok(host) = cpal::host_from_id(cpal::HostId::Alsa) {
        return host;
    }
    cpal::default_host()
}

#[cfg(target_os = "linux")]
fn output_host() -> Result<cpal::Host, SystemAudio> {
    cpal::host_from_id(cpal::HostId::PulseAudio).map_err(|e| {
        tracing::info!(error = %e, "no PulseAudio server: the computer's sound cannot be recorded");
        SystemAudio::NoSoundServer
    })
}

#[cfg(target_os = "windows")]
fn output_host() -> Result<cpal::Host, SystemAudio> {
    Ok(cpal::default_host())
}

#[cfg(target_os = "macos")]
fn output_host() -> Result<cpal::Host, SystemAudio> {
    match macos_version() {
        Some(version) if version < (14, 6) => Err(SystemAudio::MacosTooOld { version: format!("{}.{}", version.0, version.1) }),
        // Unknown (`sw_vers` failed): let cpal try; an old system then fails at the open, typed.
        _ => Ok(cpal::default_host()),
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
fn output_host() -> Result<cpal::Host, SystemAudio> {
    Err(SystemAudio::Unsupported)
}

/// The running macOS version as `(major, minor)`, from `sw_vers` (read once).
#[cfg(target_os = "macos")]
fn macos_version() -> Option<(u32, u32)> {
    static VERSION: std::sync::OnceLock<Option<(u32, u32)>> = std::sync::OnceLock::new();
    *VERSION.get_or_init(|| {
        let out = std::process::Command::new("sw_vers").arg("-productVersion").output().ok()?;
        parse_version(&String::from_utf8_lossy(&out.stdout))
    })
}

/// `14.6.1` → `(14, 6)`; `15` → `(15, 0)`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_version(text: &str) -> Option<(u32, u32)> {
    let mut parts = text.trim().split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().map_or(Some(0), |m| m.parse().ok())?;
    Some((major, minor))
}

/// The monitor source of the PulseAudio sink `output`: `<sink>.monitor`, the name every sink's
/// monitor has.
#[cfg(target_os = "linux")]
fn monitor_of(host: &cpal::Host, output: &cpal::Device, label: &str) -> Result<cpal::Device, AudioError> {
    let sink = output.id().map_err(|e| map_cpal(&e, label))?;
    let monitor = format!("{}.monitor", sink.id());
    let sources = host.input_devices().map_err(|e| map_cpal(&e, label))?;
    for source in sources {
        if source.id().is_ok_and(|id| id.id() == monitor) {
            return Ok(source);
        }
    }
    Err(AudioError::DeviceNotFound(monitor))
}

/// A stream playing silence on `output` (Windows): WASAPI's loopback only delivers while the
/// device plays something, and the recording must keep its timeline through quiet stretches.
#[cfg(target_os = "windows")]
fn silent_output(output: &cpal::Device, label: &str) -> Result<cpal::Stream, AudioError> {
    let supported = output.default_output_config().map_err(|e| map_cpal(&e, label))?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.config();
    let on_error = move |e: cpal::Error| tracing::warn!(error = %e, kind = ?e.kind(), "silent output stream error");
    let built = match format {
        SampleFormat::F32 => output.build_output_stream::<f32, _, _>(config, |data: &mut [f32], _| data.fill(0.0), on_error, None),
        SampleFormat::I16 => output.build_output_stream::<i16, _, _>(config, |data: &mut [i16], _| data.fill(0), on_error, None),
        SampleFormat::U16 => output.build_output_stream::<u16, _, _>(config, |data: &mut [u16], _| data.fill(32_768), on_error, None),
        SampleFormat::I32 => output.build_output_stream::<i32, _, _>(config, |data: &mut [i32], _| data.fill(0), on_error, None),
        other => return Err(AudioError::UnsupportedFormat(format!("{other:?} on {label}"))),
    };
    let stream = built.map_err(|e| map_cpal(&e, label))?;
    stream.play().map_err(|e| map_cpal(&e, label))?;
    Ok(stream)
}

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

    /// docs/dictation.md §22: the availability travels to the interface as a tagged value with a
    /// reason it can word; the log gets it in English.
    #[test]
    fn system_audio_serialises_with_its_reason() {
        assert_eq!(serde_json::to_string(&SystemAudio::Available).unwrap(), r#"{"state":"available"}"#);
        let old = SystemAudio::MacosTooOld { version: "14.5".into() };
        assert_eq!(serde_json::to_string(&old).unwrap(), r#"{"state":"macos_too_old","version":"14.5"}"#);
        assert_eq!(serde_json::from_str::<SystemAudio>(r#"{"state":"no_sound_server"}"#).unwrap(), SystemAudio::NoSoundServer);
        assert!(SystemAudio::Available.is_available() && !SystemAudio::Unsupported.is_available());
        assert!(old.describe().contains("14.5") && old.describe().contains("14.6"));
        assert!(SystemAudio::NoSoundServer.describe().contains("PulseAudio"));
        assert!(!SystemAudio::Unsupported.describe().is_empty());
    }

    #[test]
    fn macos_versions_parse_to_major_and_minor() {
        assert_eq!(parse_version("14.6.1\n"), Some((14, 6)));
        assert_eq!(parse_version("15"), Some((15, 0)));
        assert_eq!(parse_version("26.0"), Some((26, 0)));
        assert!(parse_version("14.6") >= Some((14, 6)) && parse_version("14.5.2") < Some((14, 6)));
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("x.y"), None);
    }

    /// Real host: listing what can be recorded of the computer's sound never fails without a sound
    /// server; with one, the default output leads. Opening an output that does not exist is a
    /// typed error (not found, or no server at all).
    #[test]
    fn real_backend_lists_outputs_and_refuses_an_unknown_one() {
        let backend = CpalBackend::new();
        let system = backend.system_audio();
        let outputs = backend.output_devices().expect("enumeration must not fail without devices");
        eprintln!("real backend: system audio {system:?}, outputs {outputs:#?}");
        if !system.is_available() {
            assert!(outputs.is_empty());
        }
        if outputs.iter().any(|d| d.is_default) {
            assert!(outputs[0].is_default);
        }
        let missing = backend.open_output_capture(Some("voltip-test:no-such-output"), Box::new(|_, _, _| {})).map(drop);
        match (&system, &missing) {
            (SystemAudio::Available, Err(AudioError::DeviceNotFound(_) | AudioError::Backend(_))) => {}
            (SystemAudio::Available, other) => panic!("an unknown output must not open: {other:?}"),
            (_, Err(AudioError::SystemAudioUnavailable(why))) => assert_eq!(why, &system.describe()),
            (_, other) => panic!("{other:?}"),
        }
    }

    /// Linux: the microphone keeps ALSA's device ids even with PulseAudio compiled in.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_microphone_stays_on_alsa_on_linux() {
        assert_eq!(CpalBackend::new().host.id(), cpal::HostId::Alsa);
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
