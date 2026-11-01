//! A [`Backend`] that plays a synthetic signal from a thread, so the meter can be tested on a host
//! without a capture device. Compiled for this crate's tests and, behind `test-support`, for
//! the shells' tests.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::AudioError;
use crate::backend::{AudioDevice, Backend, ChunkTime, SampleCallback, StreamHandle, SystemAudio, TimedSampleCallback};
use crate::cue::{CueShared, CueWindow, CueWriter};
use crate::dsp::SampleChunk;

/// Id of the fake default device.
pub const FAKE_DEFAULT_ID: &str = "fake:default";
/// Id of the fake secondary device.
pub const FAKE_USB_ID: &str = "fake:usb-mic";
/// Id of the fake output device (docs/dictation.md §22: the computer's sound).
pub const FAKE_SPEAKERS_ID: &str = "fake:speakers";

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
    /// The computer's sound (docs/dictation.md §22): its output devices, whether it can be
    /// recorded, what it plays and how.
    outputs: Vec<AudioDevice>,
    system: SystemAudio,
    output_signal: Signal,
    output_rate_hz: u32,
    output_channels: u16,
    opened_outputs: Mutex<Vec<Option<String>>>,
    /// The microphone never delivers a sample ([`FakeBackend::without_samples`]).
    no_samples: bool,
    /// What the timed input reports as its latency ([`FakeBackend::with_input_latency`]).
    input_latency: Option<Duration>,
    /// The ready cue's output (docs/dictation.md §30.3).
    cue: CueOutput,
}

thread_local! {
    /// Set on a generator thread while it runs an input callback: an output opened from inside one
    /// reads it, one opened on another thread at the same moment does not.
    static IN_INPUT_CALLBACK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The fake ready-cue output: what it was asked for, what it played, and how it plays.
struct CueOutput {
    /// There is no output device to play the cue on.
    missing: bool,
    /// The latency each output callback reports.
    latency: Option<Duration>,
    /// Frames per output callback, and the pause after each (the pace of the playback).
    frames: usize,
    pace: Duration,
    /// Held: the output thread starts writing once [`FakeBackend::release_cue_output`] is called.
    gate: Option<Arc<(Mutex<bool>, Condvar)>>,
    /// Every output opened: whether an input callback was running at that moment.
    opened: Mutex<Vec<bool>>,
    /// The mono samples written so far, by every cue output.
    played: Arc<Mutex<Vec<f32>>>,
    /// The shared state the last cue output was given.
    shared: Mutex<Option<Arc<CueShared>>>,
}

impl Default for CueOutput {
    fn default() -> Self {
        Self {
            missing: false,
            latency: Some(Duration::from_millis(20)),
            frames: 480,
            pace: Duration::from_millis(1),
            gate: None,
            opened: Mutex::new(Vec::new()),
            played: Arc::default(),
            shared: Mutex::new(None),
        }
    }
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
            outputs: vec![AudioDevice {
                id: FAKE_SPEAKERS_ID.into(),
                name: "Fake Speakers".into(),
                is_default: true,
                sample_rate_hz: Some(44_100),
                channels: Some(2),
            }],
            system: SystemAudio::Available,
            output_signal: Signal::Sine { frequency_hz: 440.0, amplitude: 0.4 },
            output_rate_hz: 44_100,
            output_channels: 2,
            opened_outputs: Mutex::new(Vec::new()),
            no_samples: false,
            input_latency: Some(Duration::from_millis(5)),
            cue: CueOutput::default(),
        }
    }

    /// The microphone opens but never delivers a sample.
    pub fn without_samples(mut self) -> Self {
        self.no_samples = true;
        self
    }

    /// The latency the timed input reports with every chunk (`None`: the host does not say).
    pub fn with_input_latency(mut self, latency: Option<Duration>) -> Self {
        self.input_latency = latency;
        self
    }

    /// No output device for the ready cue.
    pub fn without_cue_output(mut self) -> Self {
        self.cue.missing = true;
        self
    }

    /// The ready cue's output writes `frames` frames per callback, `pace` apart, reporting
    /// `latency` (44.1 kHz stereo, the fake speakers' configuration).
    pub fn with_cue_output(mut self, frames: usize, pace: Duration, latency: Option<Duration>) -> Self {
        self.cue.frames = frames.max(1);
        self.cue.pace = pace;
        self.cue.latency = latency;
        self
    }

    /// The ready cue's output is built at once but writes nothing until
    /// [`FakeBackend::release_cue_output`]: an output that is ready later than the microphone.
    pub fn with_cue_output_held(mut self) -> Self {
        self.cue.gate = Some(Arc::new((Mutex::new(false), Condvar::new())));
        self
    }

    /// Let a held cue output start writing.
    pub fn release_cue_output(&self) {
        if let Some(gate) = &self.cue.gate {
            *gate.0.lock().unwrap_or_else(PoisonError::into_inner) = true;
            gate.1.notify_all();
        }
    }

    /// Every cue output opened, in order: whether it was opened from inside an input callback, on
    /// the callback's own thread.
    pub fn cue_outputs_opened(&self) -> Vec<bool> {
        self.cue.opened.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// The mono samples the cue outputs have written so far.
    pub fn cue_played(&self) -> Vec<f32> {
        self.cue.played.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// The window the last cue output published, if it has.
    pub fn cue_window(&self) -> Option<CueWindow> {
        self.cue.shared.lock().unwrap_or_else(PoisonError::into_inner).as_ref().and_then(|shared| shared.window())
    }

    /// The rate the cue output plays at.
    pub fn cue_rate(&self) -> u32 {
        self.output_rate_hz
    }

    /// What the fake output device plays (the computer's sound).
    pub fn with_output_signal(mut self, signal: Signal) -> Self {
        self.output_signal = signal;
        self
    }

    /// The computer's sound cannot be recorded here, for `why`: no output devices either.
    pub fn without_system_audio(mut self, why: SystemAudio) -> Self {
        self.system = why;
        self.outputs.clear();
        self
    }

    /// The `id` argument of every successful `open_output_capture`, in order.
    pub fn opened_outputs(&self) -> Vec<Option<String>> {
        self.opened_outputs.lock().unwrap_or_else(PoisonError::into_inner).clone()
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

    fn open_input(&self, id: Option<&str>, on_samples: SampleCallback) -> Result<Box<dyn StreamHandle>, AudioError> {
        if let Some(err) = &self.open_error {
            return Err(err.clone());
        }
        match id {
            Some(id) if !self.devices.iter().any(|d| d.id == id) => return Err(AudioError::DeviceNotFound(id.to_string())),
            None if self.devices.is_empty() => return Err(AudioError::NoDevice),
            _ => {}
        }
        self.opened.lock().unwrap_or_else(PoisonError::into_inner).push(id.map(str::to_string));
        Ok(Box::new(self.play((self.sample_rate_hz, self.channels), self.signal, on_samples)))
    }

    fn output_devices(&self) -> Result<Vec<AudioDevice>, AudioError> {
        Ok(self.outputs.clone())
    }

    fn default_output(&self) -> Option<String> {
        self.outputs.iter().find(|d| d.is_default).map(|d| d.id.clone())
    }

    fn system_audio(&self) -> SystemAudio {
        self.system.clone()
    }

    fn open_output_capture(&self, id: Option<&str>, on_samples: SampleCallback) -> Result<Box<dyn StreamHandle>, AudioError> {
        if !self.system.is_available() {
            return Err(AudioError::SystemAudioUnavailable(self.system.describe()));
        }
        match id {
            Some(id) if !self.outputs.iter().any(|d| d.id == id) => return Err(AudioError::DeviceNotFound(id.to_string())),
            None if self.outputs.is_empty() => return Err(AudioError::NoDevice),
            _ => {}
        }
        self.opened_outputs.lock().unwrap_or_else(PoisonError::into_inner).push(id.map(str::to_string));
        Ok(Box::new(self.play((self.output_rate_hz, self.output_channels), self.output_signal, on_samples)))
    }

    fn open_input_timed(&self, id: Option<&str>, mut on_samples: TimedSampleCallback) -> Result<Box<dyn StreamHandle>, AudioError> {
        let input_latency = self.input_latency;
        self.open_input(id, Box::new(move |chunk, rate, channels| on_samples(chunk, rate, channels, ChunkTime { at: Instant::now(), input_latency })))
    }

    fn open_cue_output(&self, shared: Arc<CueShared>) -> Result<Box<dyn StreamHandle>, AudioError> {
        self.cue.opened.lock().unwrap_or_else(PoisonError::into_inner).push(IN_INPUT_CALLBACK.get());
        if self.cue.missing {
            return Err(AudioError::NoDevice);
        }
        *self.cue.shared.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::clone(&shared));
        let stop = Arc::new(AtomicBool::new(false));
        let active = Arc::clone(&self.active);
        active.fetch_add(1, Ordering::SeqCst);
        let (frames, pace, latency, gate) = (self.cue.frames, self.cue.pace, self.cue.latency, self.cue.gate.clone());
        let channels = usize::from(self.output_channels.max(1));
        let played = Arc::clone(&self.cue.played);
        let mut writer = CueWriter::new(shared, self.output_rate_hz);
        let thread_stop = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            if let Some(gate) = gate {
                let mut open = gate.0.lock().unwrap_or_else(PoisonError::into_inner);
                while !*open && !thread_stop.load(Ordering::SeqCst) {
                    open = gate.1.wait_timeout(open, Duration::from_millis(5)).unwrap_or_else(PoisonError::into_inner).0;
                }
            }
            let mut buffer = vec![0.0_f32; frames * channels];
            while !thread_stop.load(Ordering::SeqCst) {
                writer.begin(Instant::now(), latency);
                writer.fill(&mut buffer, channels as u16);
                played.lock().unwrap_or_else(PoisonError::into_inner).extend(buffer.iter().step_by(channels));
                std::thread::sleep(pace);
            }
            active.fetch_sub(1, Ordering::SeqCst);
        });
        Ok(Box::new(FakeStream { stop, thread: Some(thread) }))
    }
}

impl FakeBackend {
    /// A thread delivering `signal` at `(rate, channels)` in the configured format and chunk size.
    fn play(&self, (rate, channels): (u32, u16), signal: Signal, mut on_samples: SampleCallback) -> FakeStream {
        let stop = Arc::new(AtomicBool::new(false));
        let active = Arc::clone(&self.active);
        active.fetch_add(1, Ordering::SeqCst);
        let format = self.format;
        let samples_per_chunk = self.chunk_frames * usize::from(channels.max(1));
        // Chunks are paced at a fixed short interval: a throttle so the test thread does not spin,
        // not a wait for anything.
        let pace = Duration::from_millis(1);
        let thread_stop = Arc::clone(&stop);
        let no_samples = self.no_samples;
        let thread = std::thread::spawn(move || {
            let mut index: u64 = 0;
            let mut f32_buf = vec![0.0_f32; samples_per_chunk];
            let mut i16_buf = vec![0_i16; samples_per_chunk];
            let mut u16_buf = vec![0_u16; samples_per_chunk];
            let mut i32_buf = vec![0_i32; samples_per_chunk];
            while !thread_stop.load(Ordering::SeqCst) {
                if no_samples {
                    std::thread::sleep(pace);
                    continue;
                }
                for frame in f32_buf.chunks_mut(usize::from(channels.max(1))) {
                    let v = signal.sample(index, rate);
                    frame.fill(v);
                    index += 1;
                }
                IN_INPUT_CALLBACK.set(true);
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
                IN_INPUT_CALLBACK.set(false);
                std::thread::sleep(pace);
            }
            active.fetch_sub(1, Ordering::SeqCst);
        });
        FakeStream { stop, thread: Some(thread) }
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

    /// regression (the load runs of 2026-11-01): `cue_outputs_opened` says whether the output was
    /// opened from inside an input callback, on the callback's own thread. The recorder opens it on
    /// the thread that starts the take while the microphone already delivers, and a callback that
    /// happened to be running elsewhere at that moment made the fake report "inside".
    #[test]
    fn regression_an_output_opened_beside_a_running_callback_is_not_opened_inside_it() {
        let wait = Duration::from_secs(10);
        let backend = Arc::new(FakeBackend::new());
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let inside = Arc::clone(&backend);
        let mut calls = 0;
        let input = backend
            .open_input(
                None,
                Box::new(move |_, _, _| {
                    calls += 1;
                    if calls == 1 {
                        let _ = entered_tx.send(());
                        let _ = release_rx.recv_timeout(wait);
                    } else if calls == 2 {
                        drop(inside.open_cue_output(Arc::new(CueShared::new())));
                    }
                }),
            )
            .unwrap();
        entered_rx.recv_timeout(wait).expect("the first callback runs");
        let beside = backend.open_cue_output(Arc::new(CueShared::new())).unwrap();
        release_tx.send(()).unwrap();
        let started = Instant::now();
        while backend.cue_outputs_opened().len() < 2 && started.elapsed() < wait {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(backend.cue_outputs_opened(), vec![false, true], "beside a running callback, then inside one");
        drop(beside);
        drop(input);
        assert!(!backend.is_running());
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

    /// docs/dictation.md §22: the fake speakers play their own signal at their own rate, and a
    /// machine that cannot record its output says why.
    #[test]
    fn the_output_capture_plays_the_output_signal_or_says_why_not() {
        let backend = FakeBackend::new().with_output_signal(Signal::Constant(0.25)).with_format(FakeFormat::F32);
        assert_eq!(backend.default_output().as_deref(), Some(FAKE_SPEAKERS_ID));
        assert_eq!(backend.output_devices().unwrap().len(), 1);
        let (tx, rx) = mpsc::channel::<(f32, u32, u16)>();
        let handle = backend
            .open_output_capture(
                None,
                Box::new(move |chunk, rate, channels| {
                    if let SampleChunk::F32(s) = chunk {
                        let _ = tx.send((s[0], rate, channels));
                    }
                }),
            )
            .unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), (0.25, 44_100, 2));
        drop(handle);
        assert_eq!(backend.opened_outputs(), vec![None]);
        assert!(matches!(backend.open_output_capture(Some("fake:none"), Box::new(|_, _, _| {})).map(drop), Err(AudioError::DeviceNotFound(_))));
        let old = FakeBackend::new().without_system_audio(SystemAudio::MacosTooOld { version: "14.5".into() });
        assert!(old.output_devices().unwrap().is_empty());
        let refused = old.open_output_capture(None, Box::new(|_, _, _| {})).map(drop).unwrap_err();
        assert!(matches!(refused, AudioError::SystemAudioUnavailable(ref why) if why.contains("14.5")), "{refused:?}");
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
