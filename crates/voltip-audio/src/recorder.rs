//! The dictation recorder: one open stream, level frames for the UI while it runs, a mono
//! 16-bit [`Recording`] at the requested rate when it stops, and — on request — a live 16 kHz
//! tap for the streaming preview (docs/dictation.md §11) plus a one-shot `ready` mark when the
//! device delivers its first samples.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::AudioError;
use crate::backend::{AudioDevice, Backend, CpalBackend, SampleCallback, StreamHandle};
use crate::dsp::FrameAccumulator;
use crate::live::{LiveConsumer, LiveProducer, LiveTapConfig, live_tap};
use crate::meter::{DEFAULT_FRAMES_PER_SECOND, DEFAULT_PEAK_HOLD_MS, LevelFrame};
use crate::recording::{Recording, downmix_chunk, f32_to_i16, resample_mono};

/// Default [`RecorderConfig::target_rate_hz`]: what speech models expect.
pub const DEFAULT_TARGET_RATE_HZ: u32 = 16_000;
/// Default [`RecorderConfig::max_duration`].
pub const DEFAULT_MAX_DURATION: Duration = Duration::from_secs(120);

/// Upper bound on the capture buffer reserved up front, in seconds of audio at the device rate.
/// Longer `max_duration`s let the buffer grow instead.
const RESERVE_SECONDS: u64 = 120;

/// How to run a [`Recorder`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecorderConfig {
    /// [`AudioDevice::id`] to capture from; `None` follows the host's default input device.
    pub device_id: Option<String>,
    /// Sample rate of the finished [`Recording`]; the device's native rate is converted to it.
    pub target_rate_hz: u32,
    /// Capture stops accepting samples once this much audio has been kept.
    pub max_duration: Duration,
    /// [`LevelFrame`]s emitted per second while recording (`0` is treated as `1`).
    pub frames_per_second: u16,
    /// Also feed a live mono tap at [`LiveTapConfig::target_rate_hz`] (docs/dictation.md §11),
    /// readable through [`Recorder::live_consumer`]; `None` = no tap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_tap: Option<LiveTapConfig>,
}

impl Default for RecorderConfig {
    fn default() -> Self {
        Self {
            device_id: None,
            target_rate_hz: DEFAULT_TARGET_RATE_HZ,
            max_duration: DEFAULT_MAX_DURATION,
            frames_per_second: DEFAULT_FRAMES_PER_SECOND,
            live_tap: None,
        }
    }
}

/// What the audio thread fills in and `stop()` drains.
#[derive(Debug, Default)]
struct Capture {
    /// Mono samples at the device's native rate.
    mono: Vec<f32>,
    /// Native rate, `0` until the first chunk arrives.
    rate: u32,
    channels: u16,
    /// Most mono frames to keep; derived from `max_duration` once the rate is known.
    max_frames: usize,
    truncated: bool,
}

/// A running recording. `stop()` hands back the audio; dropping it discards the audio and
/// releases the device.
///
/// `Send + Sync`: the shell can keep it in a mutex and stop it from any thread. The audio thread
/// only ever touches a short-lived lock around the sample buffer.
pub struct Recorder {
    device: AudioDevice,
    stream: Option<Box<dyn StreamHandle>>,
    capture: Arc<Mutex<Capture>>,
    target_rate_hz: u32,
    /// The consumer end of the live tap until [`Recorder::live_consumer`] takes it. Behind a mutex
    /// only so the recorder stays `Sync` (rtrb's consumer is `Send`, not `Sync`); never contended.
    live: Mutex<Option<LiveConsumer>>,
}

/// The tap producer and its scratch space, owned by the audio callback.
struct Tap {
    producer: LiveProducer,
    /// The mono frames of the current chunk, copied out of the capture buffer so the resampler
    /// runs outside the lock. Reserved up front; grows only for a chunk larger than any before.
    scratch: Vec<f32>,
}

/// Mono frames reserved for one chunk's tap scratch (cpal chunks are a few thousand frames at most).
const TAP_SCRATCH_FRAMES: usize = 8192;

impl Recorder {
    /// Open the configured device through the platform backend and start recording, pushing a
    /// [`LevelFrame`] to `on_level` from the audio thread `frames_per_second` times per second.
    /// `on_level` must not block; hand the frame to a channel and return.
    pub fn start(config: RecorderConfig, on_level: impl Fn(LevelFrame) + Send + 'static) -> Result<Self, AudioError> {
        Self::start_with(&CpalBackend::new(), config, on_level)
    }

    /// [`Recorder::start`] against an explicit [`Backend`].
    pub fn start_with(backend: &dyn Backend, config: RecorderConfig, on_level: impl Fn(LevelFrame) + Send + 'static) -> Result<Self, AudioError> {
        Self::start_with_ready(backend, config, on_level, || {})
    }

    /// [`Recorder::start_with`] plus a one-shot `on_ready`, called from the audio thread with the
    /// first chunk that carries samples (Bluetooth / USB devices take 100–500 ms to deliver after
    /// the stream opens). Must not block. With `config.live_tap`, the tap is built here too and
    /// handed out once by [`Recorder::live_consumer`].
    pub fn start_with_ready(
        backend: &dyn Backend,
        config: RecorderConfig,
        on_level: impl Fn(LevelFrame) + Send + 'static,
        on_ready: impl FnOnce() + Send + 'static,
    ) -> Result<Self, AudioError> {
        if config.target_rate_hz == 0 {
            return Err(AudioError::Resample("target rate must be greater than 0 Hz".into()));
        }
        let devices = backend.input_devices()?;
        let device = match &config.device_id {
            Some(id) => devices.iter().find(|d| &d.id == id).cloned().ok_or_else(|| AudioError::DeviceNotFound(id.clone()))?,
            None => devices.iter().find(|d| d.is_default).or_else(|| devices.first()).cloned().ok_or(AudioError::NoDevice)?,
        };
        let frames_per_second = config.frames_per_second;
        let max_duration = config.max_duration;
        // Reserve for the whole take before the stream opens, so the audio thread never grows
        // the buffer under normal use. The device's advertised rate is the best guess here; the
        // real one arrives with the first chunk and only triggers a reallocation if it is higher.
        let mut capture = Capture::default();
        capture.mono.reserve(frames_for(max_duration.min(Duration::from_secs(RESERVE_SECONDS)), device.sample_rate_hz.unwrap_or(48_000)));
        let capture = Arc::new(Mutex::new(capture));
        let sink = Arc::clone(&capture);
        // The tap's resampler needs the device's real rate, which only the first chunk knows for
        // sure; the ring and the resampler are still built before the stream opens (from the
        // advertised rate) so the audio thread allocates nothing. A device that then reports a
        // different rate gets its tap rebuilt once, in that first callback.
        let advertised_rate = device.sample_rate_hz.unwrap_or(48_000);
        let (mut tap, live) = match &config.live_tap {
            Some(tap_config) => {
                let (producer, consumer) = live_tap(tap_config, advertised_rate)?;
                (Some((Tap { producer, scratch: Vec::with_capacity(TAP_SCRATCH_FRAMES) }, tap_config.clone(), advertised_rate)), Some(consumer))
            }
            None => (None, None),
        };
        let mut on_ready: Option<Box<dyn FnOnce() + Send>> = Some(Box::new(on_ready));
        let mut accumulator: Option<FrameAccumulator> = None;
        let on_samples: SampleCallback = Box::new(move |chunk, rate, channels| {
            if chunk_len(chunk) > 0
                && let Some(ready) = on_ready.take()
            {
                ready();
            }
            let acc = accumulator.get_or_insert_with(|| FrameAccumulator::new(rate, channels, frames_per_second, DEFAULT_PEAK_HOLD_MS));
            acc.push(chunk, &on_level);
            let mut capture = sink.lock().unwrap_or_else(PoisonError::into_inner);
            if capture.rate == 0 {
                capture.rate = rate;
                capture.channels = channels;
                capture.max_frames = frames_for(max_duration, rate);
                if let Some((t, tap_config, built_for)) = &mut tap
                    && *built_for != rate
                {
                    // Rare: the device runs at a rate other than the advertised one. Rebuilding
                    // the producer here keeps the same consumer only if we could swap the ring,
                    // which rtrb does not allow; so the tap is rebuilt as a fresh resampler in
                    // front of the same producer instead.
                    match crate::live::StreamResampler::new(rate, tap_config.target_rate_hz) {
                        Ok(resampler) => t.producer.replace_resampler(resampler),
                        Err(e) => tracing::warn!(error = %e, rate, "live tap cannot resample this device rate; tap disabled"),
                    }
                    *built_for = rate;
                }
            }
            let channels = channels.max(1);
            let frames = chunk_len(chunk) / usize::from(channels);
            let room = capture.max_frames.saturating_sub(capture.mono.len());
            let take = frames.min(room);
            if take < frames {
                capture.truncated = true;
            }
            if take > 0 {
                let before = capture.mono.len();
                let Capture { mono, .. } = &mut *capture;
                downmix_chunk(truncate_chunk(chunk, take * usize::from(channels)), channels, mono);
                if let Some((t, ..)) = &mut tap {
                    t.scratch.clear();
                    t.scratch.extend_from_slice(&mono[before..]);
                }
            }
            drop(capture);
            if let Some((t, ..)) = &mut tap
                && !t.scratch.is_empty()
            {
                t.producer.push(&t.scratch);
            }
        });
        let stream = backend.open_input(config.device_id.as_deref(), on_samples)?;
        tracing::info!(device = %device.id, name = %device.name, target_rate_hz = config.target_rate_hz, ?max_duration, live = config.live_tap.is_some(), "recorder started");
        Ok(Self { device, stream: Some(stream), capture, target_rate_hz: config.target_rate_hz, live: Mutex::new(live) })
    }

    /// The consumer end of the live tap requested with `RecorderConfig::live_tap`: `Some` exactly
    /// once, `None` afterwards and when no tap was requested. It closes when the recorder stops or
    /// is dropped.
    pub fn live_consumer(&self) -> Option<LiveConsumer> {
        self.live.lock().unwrap_or_else(PoisonError::into_inner).take()
    }

    /// The device being recorded.
    pub fn device(&self) -> &AudioDevice {
        &self.device
    }

    /// Audio kept so far, by the device clock (not wall time): `0` until the first chunk arrives,
    /// and frozen at `max_duration` once the recorder is truncating.
    pub fn elapsed(&self) -> Duration {
        let capture = self.capture.lock().unwrap_or_else(PoisonError::into_inner);
        if capture.rate == 0 {
            return Duration::ZERO;
        }
        Duration::from_micros(capture.mono.len() as u64 * 1_000_000 / u64::from(capture.rate))
    }

    /// Whether `max_duration` has been reached and further samples are being dropped.
    pub fn is_truncated(&self) -> bool {
        self.capture.lock().unwrap_or_else(PoisonError::into_inner).truncated
    }

    /// Stop capturing, release the device and convert what was kept to mono 16-bit PCM at the
    /// configured `target_rate_hz`. A recorder that never received a sample yields an empty
    /// recording.
    pub fn stop(mut self) -> Result<Recording, AudioError> {
        // Dropping the handle stops the stream, so nothing touches the buffer after this line.
        drop(self.stream.take());
        let capture = std::mem::take(&mut *self.capture.lock().unwrap_or_else(PoisonError::into_inner));
        if capture.rate == 0 {
            return Ok(Recording::from_samples(Vec::new(), self.target_rate_hz, capture.truncated));
        }
        let resampled = resample_mono(&capture.mono, capture.rate, self.target_rate_hz)?;
        let samples: Vec<i16> = resampled.iter().copied().map(f32_to_i16).collect();
        let recording = Recording::from_samples(samples, self.target_rate_hz, capture.truncated);
        tracing::info!(
            device = %self.device.id,
            native_rate_hz = capture.rate,
            channels = capture.channels,
            duration_ms = recording.duration_ms,
            truncated = recording.truncated,
            peak_dbfs = recording.peak_dbfs,
            "recording finished"
        );
        Ok(recording)
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if self.stream.take().is_some() {
            tracing::info!(device = %self.device.id, "recorder dropped without stop; audio discarded");
        }
    }
}

/// Mono frames in `duration` at `rate`.
fn frames_for(duration: Duration, rate: u32) -> usize {
    usize::try_from(duration.as_millis() * u128::from(rate) / 1000).unwrap_or(usize::MAX)
}

fn chunk_len(chunk: crate::SampleChunk<'_>) -> usize {
    match chunk {
        crate::SampleChunk::F32(s) => s.len(),
        crate::SampleChunk::I16(s) => s.len(),
        crate::SampleChunk::U16(s) => s.len(),
        crate::SampleChunk::I32(s) => s.len(),
    }
}

fn truncate_chunk(chunk: crate::SampleChunk<'_>, len: usize) -> crate::SampleChunk<'_> {
    match chunk {
        crate::SampleChunk::F32(s) => crate::SampleChunk::F32(&s[..len.min(s.len())]),
        crate::SampleChunk::I16(s) => crate::SampleChunk::I16(&s[..len.min(s.len())]),
        crate::SampleChunk::U16(s) => crate::SampleChunk::U16(&s[..len.min(s.len())]),
        crate::SampleChunk::I32(s) => crate::SampleChunk::I32(&s[..len.min(s.len())]),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Instant;

    use super::*;
    use crate::SampleChunk;
    use crate::dsp::DBFS_FLOOR;
    use crate::fake::{FakeBackend, FakeFormat, Signal};

    const WAIT: Duration = Duration::from_secs(10);

    /// Poll `done` until it holds or `WAIT` passes (the fake backend runs faster than real time,
    /// so this is a bounded condition wait, not a fixed sleep).
    fn wait_until(mut done: impl FnMut() -> bool) {
        let start = Instant::now();
        while !done() {
            assert!(start.elapsed() < WAIT, "condition not met within {WAIT:?}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn close(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn config_defaults_and_serde() {
        let config = RecorderConfig::default();
        assert_eq!(
            config,
            RecorderConfig { device_id: None, target_rate_hz: 16_000, max_duration: Duration::from_secs(120), frames_per_second: 30, live_tap: None }
        );
        assert!(!serde_json::to_string(&config).unwrap().contains("live_tap"), "None is omitted");
        let live: RecorderConfig = serde_json::from_str(r#"{"live_tap":{}}"#).unwrap();
        assert_eq!(live.live_tap, Some(LiveTapConfig::default()));
        let parsed: RecorderConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(parsed, config);
        let parsed: RecorderConfig = serde_json::from_str(r#"{"device_id":"alsa:default","target_rate_hz":8000}"#).unwrap();
        assert_eq!(parsed.device_id.as_deref(), Some("alsa:default"));
        assert_eq!(parsed.target_rate_hz, 8000);
        assert_eq!(parsed.max_duration, Duration::from_secs(120));
    }

    #[test]
    fn recorder_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Recorder>();
    }

    #[test]
    fn stereo_48k_sine_becomes_mono_16k_with_levels() {
        let backend = FakeBackend::new().with_signal(Signal::Sine { frequency_hz: 1000.0, amplitude: 0.5 });
        let (tx, rx) = mpsc::channel();
        let recorder = Recorder::start_with(&backend, RecorderConfig::default(), move |f| {
            let _ = tx.send(f);
        })
        .unwrap();
        assert_eq!(recorder.device().id, "fake:default");
        assert!(recorder.device().is_default);
        assert_eq!(backend.opened_with(), vec![None]);
        assert!(!recorder.is_truncated());

        wait_until(|| recorder.elapsed() >= Duration::from_secs(1));
        let recording = recorder.stop().unwrap();
        assert!(!backend.is_running(), "stop releases the device");

        assert_eq!(recording.sample_rate_hz, 16_000);
        assert!(!recording.truncated);
        assert!(!recording.is_silent());
        // At least one second at 16 kHz, and the sample count matches the reported duration.
        assert!(recording.samples.len() >= 16_000, "{}", recording.samples.len());
        assert_eq!(recording.duration_ms, recording.samples.len() as u64 * 1000 / 16_000);
        // 1 kHz at half amplitude survives the downmix (identical channels) and the resampler.
        assert!(close(recording.peak_dbfs, -6.02, 0.1), "peak {}", recording.peak_dbfs);
        let core = &recording.samples[1600..recording.samples.len() - 1600];
        let rms = (core.iter().map(|&s| f64::from(s) * f64::from(s)).sum::<f64>() / core.len() as f64).sqrt() / 32_768.0;
        assert!(close(rms as f32, 0.3536, 0.01), "rms {rms}");
        // Roughly 16 samples per millisecond: 1 kHz has 16-sample periods, so a zero crossing
        // pattern repeats every 16 samples.
        let crossings = core.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count();
        let expected = core.len() / 8;
        assert!(crossings.abs_diff(expected) <= expected / 50, "crossings {crossings} vs {expected}");

        let wav = recording.to_wav();
        assert_eq!(wav.len(), 44 + recording.samples.len() * 2);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]), 16_000);

        let frames: Vec<LevelFrame> = rx.try_iter().collect();
        assert!(frames.len() >= 30, "one second of audio yields at least 30 level frames, got {}", frames.len());
        assert_eq!(frames.iter().map(|f| f.seq).collect::<Vec<_>>(), (0..frames.len() as u64).collect::<Vec<_>>());
        for f in &frames {
            assert_eq!((f.sample_rate_hz, f.channels), (48_000, 2));
            assert!(close(f.rms_dbfs, -9.03, 0.1), "rms {}", f.rms_dbfs);
            assert!(!f.clipping);
        }
    }

    #[test]
    fn integer_formats_and_native_rate_paths() {
        // i16 at the target rate: no resampling, sample values survive exactly.
        let backend = FakeBackend::new().with_format(FakeFormat::I16).with_rate(16_000, 1).with_signal(Signal::Constant(0.25));
        let config = RecorderConfig { device_id: Some("fake:usb-mic".into()), frames_per_second: 20, ..RecorderConfig::default() };
        let recorder = Recorder::start_with(&backend, config, |_| {}).unwrap();
        assert_eq!(recorder.device().name, "Fake USB Microphone");
        wait_until(|| recorder.elapsed() >= Duration::from_millis(400));
        let recording = recorder.stop().unwrap();
        assert!(recording.duration_ms >= 400);
        assert!(recording.samples.iter().all(|&s| s == 8192), "{:?}", &recording.samples[..4]);
        assert!(close(recording.peak_dbfs, -12.04, 0.01));
        assert!(!recording.is_silent());

        // u16 at 8 kHz: upsampled to 16 kHz.
        let backend = FakeBackend::new().with_format(FakeFormat::U16).with_rate(8000, 1).with_signal(Signal::Sine { frequency_hz: 500.0, amplitude: 0.5 });
        let recorder = Recorder::start_with(&backend, RecorderConfig::default(), |_| {}).unwrap();
        wait_until(|| recorder.elapsed() >= Duration::from_millis(500));
        let recording = recorder.stop().unwrap();
        // The fake delivers whole 480-frame chunks, so the 2x upsample is an exact multiple of 960.
        assert!(recording.samples.len() >= 8000, "{}", recording.samples.len());
        assert_eq!(recording.samples.len() % 960, 0, "{}", recording.samples.len());
        assert_eq!(recording.duration_ms, recording.samples.len() as u64 / 16);
        assert!(close(recording.peak_dbfs, -6.02, 0.15), "peak {}", recording.peak_dbfs);

        // i32 stereo at 44.1 kHz: downsampled with a non-integer ratio.
        let backend = FakeBackend::new().with_format(FakeFormat::I32).with_rate(44_100, 2).with_signal(Signal::Constant(-0.5));
        let recorder = Recorder::start_with(&backend, RecorderConfig::default(), |_| {}).unwrap();
        wait_until(|| recorder.elapsed() >= Duration::from_millis(300));
        let recording = recorder.stop().unwrap();
        assert!(recording.duration_ms >= 300);
        let middle = recording.samples[recording.samples.len() / 2];
        assert!((i32::from(middle) + 16_384).abs() <= 2, "{middle}");
    }

    /// The live tap (docs/dictation.md §11): `on_ready` fires with the first samples, the consumer
    /// is handed out once, it carries the same audio as the take at 16 kHz mono, and it closes when
    /// the recorder stops. No tap requested → no consumer, and `on_ready` still fires.
    #[test]
    fn live_tap_follows_the_take_and_closes_on_stop() {
        let backend = FakeBackend::new().with_signal(Signal::Sine { frequency_hz: 1000.0, amplitude: 0.5 });
        let ready = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = ready.clone();
        let config = RecorderConfig { live_tap: Some(LiveTapConfig::default()), ..RecorderConfig::default() };
        let recorder = Recorder::start_with_ready(&backend, config, |_| {}, move || flag.store(true, std::sync::atomic::Ordering::SeqCst)).unwrap();
        let mut tap = recorder.live_consumer().expect("the tap was requested");
        assert!(recorder.live_consumer().is_none(), "take-once");
        assert_eq!(tap.sample_rate_hz(), 16_000);
        wait_until(|| recorder.elapsed() >= Duration::from_millis(500));
        assert!(ready.load(std::sync::atomic::Ordering::SeqCst), "ready fired with the first chunk");
        let mut live = Vec::new();
        let mut buf = vec![0.0_f32; 4096];
        loop {
            let n = tap.read(&mut buf);
            if n == 0 {
                break;
            }
            live.extend_from_slice(&buf[..n]);
        }
        assert!(live.len() >= 7000, "≥ 0.5 s of 16 kHz audio reached the tap, got {}", live.len());
        assert!(!tap.overrun() && !tap.is_closed());
        let recording = recorder.stop().unwrap();
        assert!(tap.is_closed(), "stop drops the producer");
        // Drain the rest: the tap carried (almost) the whole take.
        loop {
            let n = tap.read(&mut buf);
            if n == 0 {
                break;
            }
            live.extend_from_slice(&buf[..n]);
        }
        let take = recording.samples.len();
        assert!(live.len() <= take && take - live.len() <= 600, "tap {} vs take {take}: at most the resampler tail is missing", live.len());
        // Same signal: 1 kHz at −6 dBFS in the steady state.
        let core = &live[1600..live.len() - 400];
        let rms = crate::dsp::rms(core);
        assert!(close(rms, 0.3536, 0.01), "rms {rms}");
        let crossings = core.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
        let expected = core.len() / 8;
        assert!(crossings.abs_diff(expected) <= expected / 50, "crossings {crossings} vs {expected}");
        // Without a tap: no consumer, ready still fires.
        let ready = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = ready.clone();
        let recorder =
            Recorder::start_with_ready(&backend, RecorderConfig::default(), |_| {}, move || flag.store(true, std::sync::atomic::Ordering::SeqCst)).unwrap();
        assert!(recorder.live_consumer().is_none());
        wait_until(|| recorder.elapsed() > Duration::ZERO);
        assert!(ready.load(std::sync::atomic::Ordering::SeqCst));
        drop(recorder);
        // A device that never delivers never reports ready.
        let ready = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = ready.clone();
        let recorder =
            Recorder::start_with_ready(&MuteBackend, RecorderConfig::default(), |_| {}, move || flag.store(true, std::sync::atomic::Ordering::SeqCst)).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        assert!(!ready.load(std::sync::atomic::Ordering::SeqCst));
        drop(recorder);
    }

    #[test]
    fn silence_is_detected() {
        let backend = FakeBackend::new().with_signal(Signal::Silence);
        let recorder = Recorder::start_with(&backend, RecorderConfig::default(), |_| {}).unwrap();
        wait_until(|| recorder.elapsed() >= Duration::from_millis(500));
        let recording = recorder.stop().unwrap();
        assert_eq!(recording.peak_dbfs, DBFS_FLOOR);
        assert!(recording.is_silent());
        assert!(recording.samples.iter().all(|&s| s == 0));
    }

    #[test]
    fn max_duration_truncates_and_freezes_elapsed() {
        let backend = FakeBackend::new();
        let config = RecorderConfig { max_duration: Duration::from_millis(200), ..RecorderConfig::default() };
        let recorder = Recorder::start_with(&backend, config, |_| {}).unwrap();
        wait_until(|| recorder.is_truncated());
        assert_eq!(recorder.elapsed(), Duration::from_millis(200));
        // Give the generator time to push more chunks; none of them may be kept.
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(recorder.elapsed(), Duration::from_millis(200));
        assert!(backend.is_running(), "the stream stays open; only samples are dropped");
        let recording = recorder.stop().unwrap();
        assert!(recording.truncated);
        assert_eq!(recording.duration_ms, 200);
        assert_eq!(recording.samples.len(), 3200);
        assert!(recording.is_silent(), "200 ms is under the speech floor");
    }

    #[test]
    fn drop_without_stop_releases_device() {
        let backend = FakeBackend::new();
        let recorder = Recorder::start_with(&backend, RecorderConfig::default(), |_| {}).unwrap();
        wait_until(|| recorder.elapsed() > Duration::ZERO);
        assert!(backend.is_running());
        drop(recorder);
        assert!(!backend.is_running());
    }

    #[test]
    fn errors_from_config_selection_and_backend() {
        let backend = FakeBackend::new();
        let zero = RecorderConfig { target_rate_hz: 0, ..RecorderConfig::default() };
        let err = Recorder::start_with(&backend, zero, |_| {}).map(drop).unwrap_err();
        assert!(matches!(err, AudioError::Resample(_)), "{err:?}");
        assert!(backend.opened_with().is_empty());

        let unknown = RecorderConfig { device_id: Some("fake:nope".into()), ..RecorderConfig::default() };
        let err = Recorder::start_with(&backend, unknown, |_| {}).map(drop).unwrap_err();
        assert_eq!(err, AudioError::DeviceNotFound("fake:nope".into()));

        let empty = FakeBackend::new().without_devices();
        assert_eq!(Recorder::start_with(&empty, RecorderConfig::default(), |_| {}).map(drop).unwrap_err(), AudioError::NoDevice);

        let failing = FakeBackend::new().failing_open(AudioError::Backend("device busy".into()));
        assert_eq!(Recorder::start_with(&failing, RecorderConfig::default(), |_| {}).map(drop).unwrap_err(), AudioError::Backend("device busy".into()));

        let broken = FakeBackend::new().failing_enumeration(AudioError::Backend("host down".into()));
        assert_eq!(Recorder::start_with(&broken, RecorderConfig::default(), |_| {}).map(drop).unwrap_err(), AudioError::Backend("host down".into()));

        let no_default = FakeBackend::new().without_default();
        let recorder = Recorder::start_with(&no_default, RecorderConfig::default(), |_| {}).unwrap();
        assert_eq!(recorder.device().id, "fake:default");
    }

    /// A backend whose stream never delivers a sample.
    struct MuteBackend;

    struct MuteStream;

    impl StreamHandle for MuteStream {}

    impl Backend for MuteBackend {
        fn input_devices(&self) -> Result<Vec<AudioDevice>, AudioError> {
            Ok(vec![AudioDevice { id: "mute:0".into(), name: "Mute".into(), is_default: true, sample_rate_hz: None, channels: None }])
        }

        fn default_input(&self) -> Option<String> {
            Some("mute:0".into())
        }

        fn open_input(&self, _id: Option<&str>, _on_samples: SampleCallback) -> Result<Box<dyn StreamHandle>, AudioError> {
            Ok(Box::new(MuteStream))
        }
    }

    #[test]
    fn stop_before_any_sample_yields_empty_recording() {
        let recorder = Recorder::start_with(&MuteBackend, RecorderConfig { target_rate_hz: 8000, ..RecorderConfig::default() }, |_| {}).unwrap();
        assert_eq!(recorder.elapsed(), Duration::ZERO);
        assert!(!recorder.is_truncated());
        let recording = recorder.stop().unwrap();
        assert_eq!(recording, Recording { samples: Vec::new(), sample_rate_hz: 8000, duration_ms: 0, truncated: false, peak_dbfs: DBFS_FLOOR });
        assert!(recording.is_silent());
        assert_eq!(recording.to_wav().len(), 44);
    }

    #[test]
    fn chunk_helpers() {
        assert_eq!(frames_for(Duration::from_millis(1500), 48_000), 72_000);
        assert_eq!(frames_for(Duration::ZERO, 48_000), 0);
        let f = [0.0_f32; 6];
        let i = [0_i16; 6];
        let u = [0_u16; 6];
        let w = [0_i32; 6];
        assert_eq!(chunk_len(SampleChunk::F32(&f)), 6);
        assert_eq!(chunk_len(SampleChunk::I16(&i)), 6);
        assert_eq!(chunk_len(SampleChunk::U16(&u)), 6);
        assert_eq!(chunk_len(SampleChunk::I32(&w)), 6);
        assert_eq!(chunk_len(truncate_chunk(SampleChunk::F32(&f), 4)), 4);
        assert_eq!(chunk_len(truncate_chunk(SampleChunk::I16(&i), 4)), 4);
        assert_eq!(chunk_len(truncate_chunk(SampleChunk::U16(&u), 9)), 6);
        assert_eq!(chunk_len(truncate_chunk(SampleChunk::I32(&w), 0)), 0);
    }
}
