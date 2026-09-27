//! A [`Backend`] that plays a synthetic signal from a thread, so the meter can be tested on a host
//! without a capture device. Compiled for this crate's tests and, behind `test-support`, for
//! the shells' tests.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::AudioError;
use crate::backend::{AudioDevice, Backend, SampleCallback, StreamHandle};
use crate::dsp::SampleChunk;

/// Id of the fake default device.
pub const FAKE_DEFAULT_ID: &str = "fake:default";
/// Id of the fake secondary device.
pub const FAKE_USB_ID: &str = "fake:usb-mic";

/// Sample format the fake stream delivers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FakeFormat {
    /// `-1.0..=1.0` floats.
    F32,
    /// Signed 16-bit.
    I16,
    /// Unsigned 16-bit, `32768` = silence.
    U16,
    /// Signed 32-bit.
    I32,
}

/// What the fake microphone "hears".
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Signal {
    /// Digital silence.
    Silence,
    /// A sine of the given frequency and peak amplitude on every channel.
    Sine {
        /// Frequency in Hz.
        frequency_hz: f64,
        /// Peak amplitude, `1.0` is full scale.
        amplitude: f64,
    },
    /// Every sample has this value (DC); `1.0` produces clipping.
    Constant(f32),
}

impl Signal {
    fn sample(self, index: u64, sample_rate_hz: u32) -> f32 {
        match self {
            Self::Silence => 0.0,
            Self::Sine { frequency_hz, amplitude } => {
                let t = index as f64 / f64::from(sample_rate_hz);
                (amplitude * (2.0 * std::f64::consts::PI * frequency_hz * t).sin()) as f32
            }
            Self::Constant(v) => v,
        }
    }
}

/// Test double for the sound system. Builder-style configuration, then hand `&backend` to
/// [`crate::Meter::start_with`].
pub struct FakeBackend {
    devices: Vec<AudioDevice>,
    default_id: Option<String>,
    sample_rate_hz: u32,
    channels: u16,
    format: FakeFormat,
    signal: Signal,
    chunk_frames: usize,
    open_error: Option<AudioError>,
    enumeration_error: Option<AudioError>,
    opened: Mutex<Vec<Option<String>>>,
    active: Arc<AtomicUsize>,
}

impl Default for FakeBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeBackend {
    /// Two devices (`fake:default` at 48 kHz stereo, marked default, and `fake:usb-mic` at
    /// 16 kHz mono), streaming a 1 kHz sine at half amplitude as `f32`.
    pub fn new() -> Self {
        let devices = vec![
            AudioDevice {
                id: FAKE_DEFAULT_ID.into(),
                name: "Fake Default Microphone".into(),
                is_default: true,
                sample_rate_hz: Some(48_000),
                channels: Some(2),
            },
            AudioDevice { id: FAKE_USB_ID.into(), name: "Fake USB Microphone".into(), is_default: false, sample_rate_hz: Some(16_000), channels: Some(1) },
        ];
        Self {
            devices,
            default_id: Some(FAKE_DEFAULT_ID.into()),
            sample_rate_hz: 48_000,
            channels: 2,
            format: FakeFormat::F32,
            signal: Signal::Sine { frequency_hz: 1000.0, amplitude: 0.5 },
            chunk_frames: 480,
            open_error: None,
            enumeration_error: None,
            opened: Mutex::new(Vec::new()),
            active: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Replace the device list (the default flag is taken from the entries).
    pub fn with_devices(mut self, devices: Vec<AudioDevice>) -> Self {
        self.default_id = devices.iter().find(|d| d.is_default).map(|d| d.id.clone());
        self.devices = devices;
        self
    }

    /// Enumerate nothing: a machine without a microphone.
    pub fn without_devices(self) -> Self {
        self.with_devices(Vec::new())
    }

    /// Keep the devices but let none of them be the default.
    pub fn without_default(mut self) -> Self {
        for d in &mut self.devices {
            d.is_default = false;
        }
        self.default_id = None;
        self
    }

    /// Signal to stream.
    pub fn with_signal(mut self, signal: Signal) -> Self {
        self.signal = signal;
        self
    }

    /// Sample format to deliver.
    pub fn with_format(mut self, format: FakeFormat) -> Self {
        self.format = format;
        self
    }

    /// Stream rate and channel count reported with every chunk.
    pub fn with_rate(mut self, sample_rate_hz: u32, channels: u16) -> Self {
        self.sample_rate_hz = sample_rate_hz;
        self.channels = channels;
        self
    }

    /// Sample frames per callback.
    pub fn with_chunk_frames(mut self, frames: usize) -> Self {
        self.chunk_frames = frames.max(1);
        self
    }

    /// Make every `open_input` fail with `error`.
    pub fn failing_open(mut self, error: AudioError) -> Self {
        self.open_error = Some(error);
        self
    }

    /// Make `input_devices` fail with `error`.
    pub fn failing_enumeration(mut self, error: AudioError) -> Self {
        self.enumeration_error = Some(error);
        self
    }

    /// The `id` argument of every successful `open_input`, in order.
    pub fn opened_with(&self) -> Vec<Option<String>> {
        self.opened.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Whether any stream opened through this backend is still producing samples.
    pub fn is_running(&self) -> bool {
        self.active.load(Ordering::SeqCst) > 0
    }
}

impl Backend for FakeBackend {
    fn input_devices(&self) -> Result<Vec<AudioDevice>, AudioError> {
        if let Some(err) = &self.enumeration_error {
            return Err(err.clone());
        }
        let mut devices = self.devices.clone();
        crate::backend::sort_default_first(&mut devices);
        Ok(devices)
    }

    fn default_input(&self) -> Option<String> {
        self.default_id.clone()
    }

    fn open_input(&self, id: Option<&str>, mut on_samples: SampleCallback) -> Result<Box<dyn StreamHandle>, AudioError> {
        if let Some(err) = &self.open_error {
            return Err(err.clone());
        }
        match id {
            Some(id) if !self.devices.iter().any(|d| d.id == id) => return Err(AudioError::DeviceNotFound(id.to_string())),
            None if self.devices.is_empty() => return Err(AudioError::NoDevice),
            _ => {}
        }
        self.opened.lock().unwrap_or_else(PoisonError::into_inner).push(id.map(str::to_string));

        let stop = Arc::new(AtomicBool::new(false));
        let active = Arc::clone(&self.active);
        active.fetch_add(1, Ordering::SeqCst);
        let (rate, channels, format, signal) = (self.sample_rate_hz, self.channels, self.format, self.signal);
        let samples_per_chunk = self.chunk_frames * usize::from(channels.max(1));
        // Chunks are paced at a fixed short interval: a throttle so the test thread does not spin,
        // not a wait for anything.
        let pace = Duration::from_millis(1);
        let thread_stop = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            let mut index: u64 = 0;
            let mut f32_buf = vec![0.0_f32; samples_per_chunk];
            let mut i16_buf = vec![0_i16; samples_per_chunk];
            let mut u16_buf = vec![0_u16; samples_per_chunk];
            let mut i32_buf = vec![0_i32; samples_per_chunk];
            while !thread_stop.load(Ordering::SeqCst) {
                for frame in f32_buf.chunks_mut(usize::from(channels.max(1))) {
                    let v = signal.sample(index, rate);
                    frame.fill(v);
                    index += 1;
                }
                match format {
                    FakeFormat::F32 => on_samples(SampleChunk::F32(&f32_buf), rate, channels),
                    FakeFormat::I16 => {
                        for (dst, &src) in i16_buf.iter_mut().zip(&f32_buf) {
                            *dst = (f64::from(src) * 32_767.0).round() as i16;
                        }
                        on_samples(SampleChunk::I16(&i16_buf), rate, channels);
                    }
                    FakeFormat::U16 => {
                        for (dst, &src) in u16_buf.iter_mut().zip(&f32_buf) {
                            *dst = ((f64::from(src) + 1.0) * 32_768.0).clamp(0.0, 65_535.0) as u16;
                        }
                        on_samples(SampleChunk::U16(&u16_buf), rate, channels);
                    }
                    FakeFormat::I32 => {
                        for (dst, &src) in i32_buf.iter_mut().zip(&f32_buf) {
                            *dst = (f64::from(src) * 2_147_483_647.0) as i32;
                        }
                        on_samples(SampleChunk::I32(&i32_buf), rate, channels);
                    }
                }
                std::thread::sleep(pace);
            }
            active.fetch_sub(1, Ordering::SeqCst);
        });
        Ok(Box::new(FakeStream { stop, thread: Some(thread) }))
    }
}

struct FakeStream {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl StreamHandle for FakeStream {}

impl Drop for FakeStream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            // A panic inside the generator thread has already failed the test that caused it.
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    #[test]
    fn enumeration_puts_default_first_and_honours_overrides() {
        let backend = FakeBackend::default();
        let devices = backend.input_devices().unwrap();
        assert_eq!(devices.len(), 2);
        assert!(devices[0].is_default);
        assert_eq!(backend.default_input().as_deref(), Some(FAKE_DEFAULT_ID));

        let custom = FakeBackend::new().with_devices(vec![
            AudioDevice { id: "x:a".into(), name: "A".into(), is_default: false, sample_rate_hz: None, channels: None },
            AudioDevice { id: "x:b".into(), name: "B".into(), is_default: true, sample_rate_hz: None, channels: None },
        ]);
        assert_eq!(custom.default_input().as_deref(), Some("x:b"));
        assert_eq!(custom.input_devices().unwrap()[0].id, "x:b");

        let none = FakeBackend::new().without_default();
        assert_eq!(none.default_input(), None);
        assert!(none.input_devices().unwrap().iter().all(|d| !d.is_default));

        assert!(FakeBackend::new().without_devices().input_devices().unwrap().is_empty());
        let broken = FakeBackend::new().failing_enumeration(AudioError::Backend("boom".into()));
        assert_eq!(broken.input_devices().unwrap_err(), AudioError::Backend("boom".into()));
    }

    #[test]
    fn open_errors() {
        let backend = FakeBackend::new();
        let err = backend.open_input(Some("fake:missing"), Box::new(|_, _, _| {})).map(drop).unwrap_err();
        assert_eq!(err, AudioError::DeviceNotFound("fake:missing".into()));
        let empty = FakeBackend::new().without_devices();
        assert_eq!(empty.open_input(None, Box::new(|_, _, _| {})).map(drop).unwrap_err(), AudioError::NoDevice);
        let failing = FakeBackend::new().failing_open(AudioError::UnsupportedFormat("dsd".into()));
        assert_eq!(failing.open_input(None, Box::new(|_, _, _| {})).map(drop).unwrap_err(), AudioError::UnsupportedFormat("dsd".into()));
        assert!(backend.opened_with().is_empty());
        assert!(!backend.is_running());
    }

    #[test]
    fn stream_delivers_signal_in_every_format_and_stops_on_drop() {
        for format in [FakeFormat::F32, FakeFormat::I16, FakeFormat::U16, FakeFormat::I32] {
            let backend = FakeBackend::new()
                .with_format(format)
                .with_rate(8000, 1)
                .with_chunk_frames(80)
                .with_signal(Signal::Sine { frequency_hz: 1000.0, amplitude: 0.5 });
            let (tx, rx) = mpsc::channel::<(f32, f32, u32, u16, usize)>();
            let handle = backend
                .open_input(
                    None,
                    Box::new(move |chunk, rate, channels| {
                        let (min, max, len) = match chunk {
                            SampleChunk::F32(s) => (s.iter().copied().fold(f32::MAX, f32::min), s.iter().copied().fold(f32::MIN, f32::max), s.len()),
                            SampleChunk::I16(s) => (f32::from(*s.iter().min().unwrap()) / 32_768.0, f32::from(*s.iter().max().unwrap()) / 32_768.0, s.len()),
                            SampleChunk::U16(s) => (
                                (f32::from(*s.iter().min().unwrap()) - 32_768.0) / 32_768.0,
                                (f32::from(*s.iter().max().unwrap()) - 32_768.0) / 32_768.0,
                                s.len(),
                            ),
                            SampleChunk::I32(s) => {
                                (*s.iter().min().unwrap() as f32 / 2_147_483_648.0, *s.iter().max().unwrap() as f32 / 2_147_483_648.0, s.len())
                            }
                        };
                        let _ = tx.send((min, max, rate, channels, len));
                    }),
                )
                .unwrap();
            let (min, max, rate, channels, len) = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!((min + 0.5).abs() < 1e-3, "{format:?} min {min}");
            assert!((max - 0.5).abs() < 1e-3, "{format:?} max {max}");
            assert_eq!((rate, channels, len), (8000, 1, 80));
            assert!(backend.is_running());
            drop(handle);
            assert!(!backend.is_running(), "{format:?}: drop joins the generator thread");
        }
    }

    #[test]
    fn signals() {
        assert_eq!(Signal::Silence.sample(123, 48_000), 0.0);
        assert_eq!(Signal::Constant(0.25).sample(0, 48_000), 0.25);
        let sine = Signal::Sine { frequency_hz: 1000.0, amplitude: 1.0 };
        assert!(sine.sample(0, 48_000).abs() < 1e-6);
        assert!((sine.sample(12, 48_000) - 1.0).abs() < 1e-6);
        assert!((sine.sample(36, 48_000) + 1.0).abs() < 1e-6);
        let constant = FakeBackend::new().with_signal(Signal::Constant(1.0)).with_chunk_frames(0);
        assert_eq!(constant.chunk_frames, 1);
    }
}
