//! The dictation recorder: an open stream (or two, `mixed`), level frames for the UI while it
//! runs, a mono 16-bit [`Recording`] at the requested rate when it stops, and — on request — a
//! live 16 kHz tap for the streaming preview (docs/dictation.md §11), the long take's 16 kHz
//! stream for the core's recording file (§22), and a one-shot `ready` mark when the device
//! delivers its first samples.
//!
//! Sources (§22): the microphone, what an output device plays (the computer's sound), or both
//! mixed at 16 kHz ([`crate::mix`]: the microphone is the clock). Whatever the source, one
//! [`Sink`] turns the signal into levels, the in-memory take, the tap and the stream. A `mixed`
//! take with echo cancellation (§22.6) feeds its sink from a mixing thread of its own: the
//! microphone's callback pairs the two sides and queues the pairs, the thread cancels the echo,
//! sums and feeds the sink, and `stop()` lets it finish what is queued first.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::AudioError;
use crate::backend::{AudioDevice, Backend, CpalBackend, SampleCallback, StreamHandle};
use crate::dsp::{FrameAccumulator, SampleChunk};
use crate::live::{LiveConsumer, LiveProducer, LiveTapConfig, StreamResampler, live_tap};
use crate::meter::{DEFAULT_FRAMES_PER_SECOND, DEFAULT_PEAK_HOLD_MS, LevelFrame};
use crate::mix::{CancelledMix, MIX_STEP, mix_queue};
use crate::pcm::{PcmConsumer, PcmProducer, PcmStreamConfig, pcm_stream};
use crate::recording::{Recording, downmix_chunk, f32_to_i16, resample_mono};

/// Default [`RecorderConfig::target_rate_hz`]: what speech models expect.
pub const DEFAULT_TARGET_RATE_HZ: u32 = 16_000;
/// Default [`RecorderConfig::max_duration`].
pub const DEFAULT_MAX_DURATION: Duration = Duration::from_secs(120);
/// The rate the `mixed` source is mixed at, and so the rate its [`Sink`] sees.
pub const MIX_RATE_HZ: u32 = 16_000;

/// Upper bound on the capture buffer reserved up front, in seconds of audio at the device rate.
/// Longer `max_duration`s let the buffer grow instead.
const RESERVE_SECONDS: u64 = 120;
/// Mono frames reserved for one chunk's scratch space (cpal chunks are a few thousand frames at
/// most); grows only for a chunk larger than any before.
const SCRATCH_FRAMES: usize = 8192;
/// How much of the computer's sound may wait for the microphone in `mixed` (one second at 16 kHz;
/// the mixer keeps it to 20 ms in normal running).
const MIX_QUEUE_SAMPLES: usize = 16_000;
/// How many paired samples may wait for the mixing thread of an echo-cancelled `mixed` take: two
/// seconds; the thread is woken for every microphone chunk and keeps up with a few milliseconds of
/// work per 100 ms.
const PAIR_QUEUE_SAMPLES: usize = 2 * MIX_RATE_HZ as usize;
/// How long the mixing thread sleeps when nothing wakes it.
const MIX_THREAD_IDLE: Duration = Duration::from_millis(20);

/// Where the recorder takes its audio from (docs/dictation.md §22).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CaptureSource {
    /// The input device [`RecorderConfig::device_id`] (the default).
    #[default]
    Microphone,
    /// What the output device `output_id` plays (`None`: the default output).
    System {
        /// [`AudioDevice::id`] of an output device, or `None`.
        output_id: Option<String>,
    },
    /// The microphone and what `output_id` plays, mixed at [`MIX_RATE_HZ`].
    Mixed {
        /// [`AudioDevice::id`] of an output device, or `None`.
        output_id: Option<String>,
        /// Cancel the microphone's echo of the computer's sound before the sum (docs/dictation.md
        /// §22.6); on unless told otherwise.
        #[serde(default = "cancel_echo")]
        echo_cancel: bool,
    },
}

fn cancel_echo() -> bool {
    true
}

/// How to run a [`Recorder`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecorderConfig {
    /// [`AudioDevice::id`] to capture from; `None` follows the host's default input device.
    pub device_id: Option<String>,
    /// Sample rate of the finished [`Recording`]; the device's native rate is converted to it.
    pub target_rate_hz: u32,
    /// Capture stops accepting samples into the [`Recording`] once this much audio has been kept
    /// (the tap and the long take's stream go on).
    pub max_duration: Duration,
    /// [`LevelFrame`]s emitted per second while recording (`0` is treated as `1`).
    pub frames_per_second: u16,
    /// Also feed a live mono tap at [`LiveTapConfig::target_rate_hz`] (docs/dictation.md §11),
    /// readable through [`Recorder::live_consumer`]; `None` = no tap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_tap: Option<LiveTapConfig>,
    /// The microphone, the computer's sound, or both (docs/dictation.md §22).
    pub source: CaptureSource,
    /// Also feed the whole take to [`Recorder::pcm_consumer`] (docs/dictation.md §22, a take that
    /// may run past `max_duration`); `None` = no stream.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pcm_stream: Option<PcmStreamConfig>,
}

impl Default for RecorderConfig {
    fn default() -> Self {
        Self {
            device_id: None,
            target_rate_hz: DEFAULT_TARGET_RATE_HZ,
            max_duration: DEFAULT_MAX_DURATION,
            frames_per_second: DEFAULT_FRAMES_PER_SECOND,
            live_tap: None,
            source: CaptureSource::Microphone,
            pcm_stream: None,
        }
    }
}

/// What the audio thread fills in and `stop()` drains.
#[derive(Debug, Default)]
struct Capture {
    /// Mono samples at the device's native rate (at [`MIX_RATE_HZ`] for `mixed`).
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
    /// The device being recorded: the microphone, or the output device for `system`.
    device: AudioDevice,
    /// The output device whose sound is recorded (`system` and `mixed`).
    output: Option<AudioDevice>,
    streams: Vec<Box<dyn StreamHandle>>,
    capture: Arc<Mutex<Capture>>,
    target_rate_hz: u32,
    /// The consumer end of the live tap until [`Recorder::live_consumer`] takes it. Behind a mutex
    /// only so the recorder stays `Sync` (rtrb's consumer is `Send`, not `Sync`); never contended.
    live: Mutex<Option<LiveConsumer>>,
    /// The consumer end of the long take's stream until [`Recorder::pcm_consumer`] takes it.
    pcm: Mutex<Option<PcmConsumer>>,
    /// An echo-cancelled `mixed` take's mixing thread; after `streams`, so a drop stops the
    /// callbacks before the thread finishes.
    mixing: Option<MixThread>,
}

/// The mixing thread of an echo-cancelled `mixed` take (docs/dictation.md §22.6): it owns the
/// take's [`Sink`] and feeds it the cancelled sum of the pairs the microphone's callback queues.
struct MixThread {
    handle: Option<std::thread::JoinHandle<()>>,
    done: Arc<AtomicBool>,
    /// Pairs dropped because the queue was full (the thread fell two seconds behind).
    lost: Arc<AtomicU64>,
}

impl MixThread {
    fn spawn(mut pairs: rtrb::Consumer<[f32; 2]>, mut sink: Sink) -> Result<Self, AudioError> {
        let done = Arc::new(AtomicBool::new(false));
        let finished = Arc::clone(&done);
        let handle = std::thread::Builder::new()
            .name("voltip-mix".into())
            .spawn(move || {
                let mut mix = CancelledMix::new();
                let (mut mic, mut other, mut out) = (Vec::with_capacity(MIX_STEP), Vec::with_capacity(MIX_STEP), Vec::with_capacity(MIX_STEP));
                loop {
                    // Read the flag before draining: the callbacks have stopped by the time it is
                    // set, so a drain after seeing it takes everything they queued.
                    let last = finished.load(Ordering::Acquire);
                    loop {
                        let n = pairs.slots().min(MIX_STEP);
                        let Ok(chunk) = pairs.read_chunk(n) else { break };
                        if n == 0 {
                            break;
                        }
                        let (first, second) = chunk.as_slices();
                        mic.clear();
                        other.clear();
                        for pair in first.iter().chain(second) {
                            mic.push(pair[0]);
                            other.push(pair[1]);
                        }
                        chunk.commit_all();
                        out.clear();
                        mix.mix(&mic, &other, &mut out);
                        sink.process(SampleChunk::F32(&out), MIX_RATE_HZ, 1);
                    }
                    if last {
                        break;
                    }
                    std::thread::park_timeout(MIX_THREAD_IDLE);
                }
            })
            .map_err(|e| AudioError::Backend(format!("mixing thread: {e}")))?;
        Ok(Self { handle: Some(handle), done, lost: Arc::new(AtomicU64::new(0)) })
    }

    /// The handle the microphone's callback wakes the thread with.
    fn waker(&self) -> Option<std::thread::Thread> {
        self.handle.as_ref().map(|h| h.thread().clone())
    }

    /// Let the thread mix what is queued, then wait for it.
    fn finish(&mut self) {
        self.done.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            handle.thread().unpark();
            if handle.join().is_err() {
                tracing::warn!("the mixing thread panicked; the take ends where it stopped");
            }
        }
        let lost = self.lost.load(Ordering::Relaxed);
        if lost > 0 {
            tracing::warn!(lost, "the mixing thread fell behind; samples of the take were dropped");
        }
    }
}

impl Drop for MixThread {
    fn drop(&mut self) {
        self.finish();
    }
}

/// Where each chunk of the recorded signal goes, owned by the audio callback that feeds it: the
/// level frames, the in-memory take (up to `max_duration`), the live tap and the long take's
/// stream (the whole chunk, even once the in-memory take is full).
struct Sink {
    on_level: Box<dyn Fn(LevelFrame) + Send>,
    on_ready: Option<Box<dyn FnOnce() + Send>>,
    accumulator: Option<FrameAccumulator>,
    frames_per_second: u16,
    capture: Arc<Mutex<Capture>>,
    max_duration: Duration,
    /// The tap and the rate its resampler was built for.
    tap: Option<(LiveProducer, u32, u32)>,
    /// The stream and the rate its resampler was built for.
    pcm: Option<(PcmProducer, u32, u32)>,
    /// The current chunk as mono.
    mono: Vec<f32>,
}

impl Sink {
    fn process(&mut self, chunk: SampleChunk<'_>, rate: u32, channels: u16) {
        if chunk_len(chunk) > 0
            && let Some(ready) = self.on_ready.take()
        {
            ready();
        }
        let acc = self.accumulator.get_or_insert_with(|| FrameAccumulator::new(rate, channels, self.frames_per_second, DEFAULT_PEAK_HOLD_MS));
        acc.push(chunk, &*self.on_level);
        let channels = channels.max(1);
        self.mono.clear();
        downmix_chunk(chunk, channels, &mut self.mono);
        let mut capture = self.capture.lock().unwrap_or_else(PoisonError::into_inner);
        if capture.rate == 0 {
            capture.rate = rate;
            capture.channels = channels;
            capture.max_frames = frames_for(self.max_duration, rate);
            // Rare: the device runs at a rate other than the advertised one; the resamplers in
            // front of the tap and the stream are rebuilt once, here in the first callback (the
            // rings stay: rtrb cannot swap them).
            if let Some((producer, built_for, target)) = &mut self.tap
                && *built_for != rate
            {
                match StreamResampler::new(rate, *target) {
                    Ok(resampler) => producer.replace_resampler(resampler),
                    Err(e) => tracing::warn!(error = %e, rate, "live tap cannot resample this device rate; tap disabled"),
                }
                *built_for = rate;
            }
            if let Some((producer, built_for, target)) = &mut self.pcm
                && *built_for != rate
            {
                match StreamResampler::new(rate, *target) {
                    Ok(resampler) => producer.replace_resampler(resampler),
                    Err(e) => tracing::warn!(error = %e, rate, "the long take's stream cannot resample this device rate"),
                }
                *built_for = rate;
            }
        }
        let room = capture.max_frames.saturating_sub(capture.mono.len());
        let take = self.mono.len().min(room);
        if take < self.mono.len() {
            capture.truncated = true;
        }
        let Capture { mono, .. } = &mut *capture;
        mono.extend_from_slice(&self.mono[..take]);
        drop(capture);
        if let Some((producer, ..)) = &mut self.tap {
            producer.push(&self.mono);
        }
        if let Some((producer, ..)) = &mut self.pcm {
            producer.push(&self.mono);
        }
    }
}

/// One side of `mixed` on its way to 16 kHz mono: downmix, resample.
struct ToMixRate {
    resampler: StreamResampler,
    built_for: u32,
    mono: Vec<f32>,
    out: Vec<f32>,
}

impl ToMixRate {
    fn new(advertised_rate: u32) -> Result<Self, AudioError> {
        Ok(Self {
            resampler: StreamResampler::new(advertised_rate, MIX_RATE_HZ)?,
            built_for: advertised_rate,
            mono: Vec::with_capacity(SCRATCH_FRAMES),
            out: Vec::with_capacity(SCRATCH_FRAMES),
        })
    }

    /// `chunk` at `rate` as 16 kHz mono in `self.out` (whatever the resampler has ready).
    fn convert(&mut self, chunk: SampleChunk<'_>, rate: u32, channels: u16) {
        if rate != self.built_for {
            match StreamResampler::new(rate, MIX_RATE_HZ) {
                Ok(resampler) => self.resampler = resampler,
                Err(e) => tracing::warn!(error = %e, rate, "cannot resample this device rate for mixing"),
            }
            self.built_for = rate;
        }
        self.mono.clear();
        downmix_chunk(chunk, channels.max(1), &mut self.mono);
        self.out.clear();
        let Self { resampler, mono, out, .. } = self;
        if resampler.push(mono, &mut |frames: &[f32]| out.extend_from_slice(frames)).is_err() {
            out.clear();
        }
    }
}

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
    /// the stream opens). Must not block. With `config.live_tap` / `config.pcm_stream`, the tap and
    /// the stream are built here too and handed out once by [`Recorder::live_consumer`] /
    /// [`Recorder::pcm_consumer`].
    pub fn start_with_ready(
        backend: &dyn Backend,
        config: RecorderConfig,
        on_level: impl Fn(LevelFrame) + Send + 'static,
        on_ready: impl FnOnce() + Send + 'static,
    ) -> Result<Self, AudioError> {
        if config.target_rate_hz == 0 {
            return Err(AudioError::Resample("target rate must be greater than 0 Hz".into()));
        }
        let output_id = match &config.source {
            CaptureSource::Microphone => None,
            CaptureSource::System { output_id } | CaptureSource::Mixed { output_id, .. } => Some(output_id.as_deref()),
        };
        let output = match output_id {
            None => None,
            Some(id) => {
                let system = backend.system_audio();
                if !system.is_available() {
                    return Err(AudioError::SystemAudioUnavailable(system.describe()));
                }
                let outputs = backend.output_devices()?;
                let device = match id {
                    Some(id) => outputs.iter().find(|d| d.id == id).cloned().ok_or_else(|| AudioError::DeviceNotFound(id.to_owned()))?,
                    None => outputs.iter().find(|d| d.is_default).or_else(|| outputs.first()).cloned().ok_or(AudioError::NoDevice)?,
                };
                Some(device)
            }
        };
        let microphone = match &config.source {
            CaptureSource::System { .. } => None,
            CaptureSource::Microphone | CaptureSource::Mixed { .. } => {
                let devices = backend.input_devices()?;
                Some(match &config.device_id {
                    Some(id) => devices.iter().find(|d| &d.id == id).cloned().ok_or_else(|| AudioError::DeviceNotFound(id.clone()))?,
                    None => devices.iter().find(|d| d.is_default).or_else(|| devices.first()).cloned().ok_or(AudioError::NoDevice)?,
                })
            }
        };
        let mixed = matches!(config.source, CaptureSource::Mixed { .. });
        let echo_cancel = matches!(config.source, CaptureSource::Mixed { echo_cancel: true, .. });
        // What the sink sees: the device's own rate, or the mixing rate.
        let (device, sink_rate) = match (&microphone, &output) {
            (Some(mic), _) if mixed => (mic.clone(), MIX_RATE_HZ),
            (Some(mic), _) => (mic.clone(), mic.sample_rate_hz.unwrap_or(48_000)),
            (None, Some(out)) => (out.clone(), out.sample_rate_hz.unwrap_or(48_000)),
            (None, None) => return Err(AudioError::NoDevice),
        };
        let max_duration = config.max_duration;
        // Reserve for the whole in-memory take before the stream opens, so the audio thread never
        // grows the buffer under normal use. The advertised rate is the best guess here; the real
        // one arrives with the first chunk and only triggers a reallocation if it is higher.
        let mut capture = Capture::default();
        capture.mono.reserve(frames_for(max_duration.min(Duration::from_secs(RESERVE_SECONDS)), sink_rate));
        let capture = Arc::new(Mutex::new(capture));
        // The tap's and the stream's resamplers are built before the stream opens (from the
        // advertised rate), so the audio thread allocates nothing; a device that then reports a
        // different rate gets them rebuilt once, in the first callback.
        let (tap, live) = match &config.live_tap {
            Some(tap_config) => {
                let (producer, consumer) = live_tap(tap_config, sink_rate)?;
                (Some((producer, sink_rate, tap_config.target_rate_hz)), Some(consumer))
            }
            None => (None, None),
        };
        let (pcm, pcm_out) = match &config.pcm_stream {
            Some(stream_config) => {
                let (producer, consumer) = pcm_stream(stream_config, Some(sink_rate))?;
                (Some((producer, sink_rate, stream_config.target_rate_hz)), Some(consumer))
            }
            None => (None, None),
        };
        let mut sink = Sink {
            on_level: Box::new(on_level),
            on_ready: Some(Box::new(on_ready)),
            accumulator: None,
            frames_per_second: config.frames_per_second,
            capture: Arc::clone(&capture),
            max_duration,
            tap,
            pcm,
            mono: Vec::with_capacity(SCRATCH_FRAMES),
        };
        let mut streams = Vec::with_capacity(2);
        let mut mixing = None;
        match (&microphone, &output) {
            (Some(mic), Some(out)) => {
                // `mixed`: the computer's sound waits in a queue at 16 kHz; the microphone's
                // callback takes from it, mixes and feeds the sink.
                let (mut queue, mut mixer) = mix_queue(MIX_QUEUE_SAMPLES);
                let mut other = ToMixRate::new(out.sample_rate_hz.unwrap_or(48_000))?;
                let on_output: SampleCallback = Box::new(move |chunk, rate, channels| {
                    other.convert(chunk, rate, channels);
                    // A full queue means the microphone stopped taking: those samples are lost,
                    // and the mixer drops what is left from before them.
                    queue.push(&other.out);
                });
                streams.push(backend.open_output_capture(Some(out.id.as_str()), on_output)?);
                let mut own = ToMixRate::new(mic.sample_rate_hz.unwrap_or(48_000))?;
                let on_microphone: SampleCallback = if echo_cancel {
                    // The canceller allocates: the callback only pairs and queues, the mixing
                    // thread does the rest.
                    let (mut pairs, queued) = rtrb::RingBuffer::new(PAIR_QUEUE_SAMPLES);
                    let thread = MixThread::spawn(queued, sink)?;
                    let (wake, lost) = (thread.waker(), Arc::clone(&thread.lost));
                    mixing = Some(thread);
                    Box::new(move |chunk, rate, channels| {
                        own.convert(chunk, rate, channels);
                        mixer.pair(&own.out, |mic, other| match pairs.write_chunk_uninit(mic.len()) {
                            Ok(slots) => {
                                slots.fill_from_iter(mic.iter().zip(other).map(|(m, o)| [*m, *o]));
                            }
                            Err(_) => {
                                lost.fetch_add(mic.len() as u64, Ordering::Relaxed);
                            }
                        });
                        if let Some(thread) = &wake {
                            thread.unpark();
                        }
                    })
                } else {
                    let mut mixed_out = Vec::with_capacity(SCRATCH_FRAMES);
                    Box::new(move |chunk, rate, channels| {
                        own.convert(chunk, rate, channels);
                        mixer.mix(&own.out, &mut mixed_out);
                        sink.process(SampleChunk::F32(&mixed_out), MIX_RATE_HZ, 1);
                    })
                };
                streams.push(backend.open_input(config.device_id.as_deref(), on_microphone)?);
            }
            (Some(_), None) => {
                streams.push(backend.open_input(config.device_id.as_deref(), Box::new(move |chunk, rate, channels| sink.process(chunk, rate, channels)))?);
            }
            (None, Some(out)) => {
                streams.push(backend.open_output_capture(Some(out.id.as_str()), Box::new(move |chunk, rate, channels| sink.process(chunk, rate, channels)))?);
            }
            (None, None) => return Err(AudioError::NoDevice),
        }
        tracing::info!(
            device = %device.id,
            name = %device.name,
            output = ?output.as_ref().map(|d| d.id.as_str()),
            source = ?config.source,
            target_rate_hz = config.target_rate_hz,
            ?max_duration,
            live = config.live_tap.is_some(),
            stream = config.pcm_stream.is_some(),
            echo_cancel,
            "recorder started"
        );
        Ok(Self { device, output, streams, capture, target_rate_hz: config.target_rate_hz, live: Mutex::new(live), pcm: Mutex::new(pcm_out), mixing })
    }

    /// The consumer end of the live tap requested with `RecorderConfig::live_tap`: `Some` exactly
    /// once, `None` afterwards and when no tap was requested. It closes when the recorder stops or
    /// is dropped.
    pub fn live_consumer(&self) -> Option<LiveConsumer> {
        self.live.lock().unwrap_or_else(PoisonError::into_inner).take()
    }

    /// The consumer end of the long take's stream requested with `RecorderConfig::pcm_stream`
    /// (docs/dictation.md §22): `Some` exactly once. It closes when the recorder stops or is
    /// dropped.
    pub fn pcm_consumer(&self) -> Option<PcmConsumer> {
        self.pcm.lock().unwrap_or_else(PoisonError::into_inner).take()
    }

    /// The device being recorded: the microphone, or the output device for `system`.
    pub fn device(&self) -> &AudioDevice {
        &self.device
    }

    /// The output device whose sound is recorded (`system` and `mixed`).
    pub fn output_device(&self) -> Option<&AudioDevice> {
        self.output.as_ref()
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
        // Dropping the handles stops the streams; a mixing thread then mixes what they queued and
        // ends. Nothing touches the buffer after these lines.
        self.streams.clear();
        if let Some(mut thread) = self.mixing.take() {
            thread.finish();
        }
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
        if !self.streams.is_empty() {
            self.streams.clear();
            tracing::info!(device = %self.device.id, "recorder dropped without stop; audio discarded");
        }
    }
}

/// Mono frames in `duration` at `rate`.
fn frames_for(duration: Duration, rate: u32) -> usize {
    usize::try_from(duration.as_millis() * u128::from(rate) / 1000).unwrap_or(usize::MAX)
}

fn chunk_len(chunk: SampleChunk<'_>) -> usize {
    match chunk {
        SampleChunk::F32(s) => s.len(),
        SampleChunk::I16(s) => s.len(),
        SampleChunk::U16(s) => s.len(),
        SampleChunk::I32(s) => s.len(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Instant;

    use super::*;
    use crate::SampleChunk;
    use crate::backend::SystemAudio;
    use crate::dsp::DBFS_FLOOR;
    use crate::fake::{FAKE_DEFAULT_ID, FAKE_SPEAKERS_ID, FakeBackend, FakeFormat, Signal};

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
            RecorderConfig {
                device_id: None,
                target_rate_hz: 16_000,
                max_duration: Duration::from_secs(120),
                frames_per_second: 30,
                live_tap: None,
                source: CaptureSource::Microphone,
                pcm_stream: None,
            }
        );
        assert!(!serde_json::to_string(&config).unwrap().contains("pcm_stream"), "None is omitted");
        let mixed: RecorderConfig = serde_json::from_str(r#"{"source":{"kind":"mixed","output_id":"fake:speakers"},"pcm_stream":{}}"#).unwrap();
        assert_eq!(
            mixed.source,
            CaptureSource::Mixed { output_id: Some("fake:speakers".into()), echo_cancel: true },
            "echo cancellation unless told otherwise"
        );
        let plain: CaptureSource = serde_json::from_str(r#"{"kind":"mixed","output_id":null,"echo_cancel":false}"#).unwrap();
        assert_eq!(plain, CaptureSource::Mixed { output_id: None, echo_cancel: false });
        assert_eq!(mixed.pcm_stream, Some(PcmStreamConfig::default()));
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

    /// docs/dictation.md §22: `system` records what the output device plays, at its own rate,
    /// and opens no microphone; a machine that cannot record its output says why before opening
    /// anything.
    #[test]
    fn the_system_source_records_the_output_device() {
        let backend = FakeBackend::new().with_output_signal(Signal::Sine { frequency_hz: 440.0, amplitude: 0.4 });
        let system = || RecorderConfig { source: CaptureSource::System { output_id: None }, ..RecorderConfig::default() };
        let recorder = Recorder::start_with(&backend, system(), |_| {}).unwrap();
        assert_eq!(recorder.device().id, FAKE_SPEAKERS_ID);
        assert_eq!(recorder.output_device().map(|d| d.id.as_str()), Some(FAKE_SPEAKERS_ID));
        assert!(backend.opened_with().is_empty(), "no microphone is opened");
        assert_eq!(backend.opened_outputs(), vec![Some(FAKE_SPEAKERS_ID.to_owned())]);
        wait_until(|| recorder.elapsed() >= Duration::from_millis(500));
        let recording = recorder.stop().unwrap();
        assert!(!recording.is_silent());
        // 0.4 of full scale, measured away from the take's edges (the resampler rings where the
        // stop cuts the sine).
        let core = &recording.samples[1600..recording.samples.len() - 1600];
        let rms = (core.iter().map(|&s| f64::from(s) * f64::from(s)).sum::<f64>() / core.len() as f64).sqrt() / 32_768.0;
        assert!(close(rms as f32, 0.2828, 0.01), "rms {rms}");
        assert!(!backend.is_running());
        let old = FakeBackend::new().without_system_audio(SystemAudio::MacosTooOld { version: "14.5".into() });
        let refused = Recorder::start_with(&old, system(), |_| {}).map(drop).unwrap_err();
        assert!(matches!(refused, AudioError::SystemAudioUnavailable(ref why) if why.contains("14.5")), "{refused:?}");
        assert!(old.opened_with().is_empty() && old.opened_outputs().is_empty());
        let missing = RecorderConfig { source: CaptureSource::System { output_id: Some("fake:none".into()) }, ..RecorderConfig::default() };
        assert_eq!(Recorder::start_with(&backend, missing, |_| {}).map(drop).unwrap_err(), AudioError::DeviceNotFound("fake:none".into()));
    }

    /// `mixed`: the microphone (the clock) and the output land in one 16 kHz take, summed with
    /// −3 dB each; the live tap follows the mix; both streams are released at the stop. Without
    /// echo cancellation: the sum itself (the cancelled one has its own test below).
    #[test]
    fn the_mixed_source_sums_the_microphone_and_the_output() {
        let backend = FakeBackend::new().with_signal(Signal::Constant(0.2)).with_output_signal(Signal::Constant(0.3));
        let config = RecorderConfig {
            source: CaptureSource::Mixed { output_id: Some(FAKE_SPEAKERS_ID.into()), echo_cancel: false },
            live_tap: Some(LiveTapConfig::default()),
            ..RecorderConfig::default()
        };
        let recorder = Recorder::start_with(&backend, config, |_| {}).unwrap();
        assert_eq!(recorder.device().id, FAKE_DEFAULT_ID, "the microphone is the clock");
        assert_eq!(recorder.output_device().map(|d| d.id.as_str()), Some(FAKE_SPEAKERS_ID));
        assert_eq!((backend.opened_with(), backend.opened_outputs()), (vec![None], vec![Some(FAKE_SPEAKERS_ID.to_owned())]));
        let mut tap = recorder.live_consumer().unwrap();
        wait_until(|| recorder.elapsed() >= Duration::from_millis(800));
        let recording = recorder.stop().unwrap();
        assert_eq!(recording.sample_rate_hz, 16_000);
        let (both, microphone) = (crate::mix::MIX_GAIN * 0.5, crate::mix::MIX_GAIN * 0.2);
        let level = |s: i16| f32::from(s) / 32_768.0;
        // Past the resamplers' start-up (100 ms).
        let steady = &recording.samples[1600..];
        assert!(steady.iter().any(|&s| (level(s) - both).abs() < 0.01), "the output is in the mix");
        assert!(steady.iter().all(|&s| level(s) < both + 0.02), "nothing past the sum");
        assert!(steady.iter().all(|&s| level(s) > microphone - 0.02), "the microphone is always in it");
        let mut buf = vec![0.0f32; 4096];
        assert!(tap.read(&mut buf) > 0, "the tap follows the mix");
        assert!(!backend.is_running(), "both streams are released");
    }

    /// `mixed` with echo cancellation (docs/dictation.md §22.6): the mixing thread feeds the take,
    /// the tap and the levels, the microphone comes through (a tone the computer is not playing
    /// is no echo), and `stop()` waits for the thread, so the take ends with everything the
    /// callbacks queued and nothing runs after it.
    #[test]
    fn the_echo_cancelled_mix_runs_on_its_own_thread_and_the_stop_waits_for_it() {
        let backend = FakeBackend::new().with_signal(Signal::Sine { frequency_hz: 440.0, amplitude: 0.2 }).with_output_signal(Signal::Silence);
        let config = RecorderConfig {
            source: CaptureSource::Mixed { output_id: Some(FAKE_SPEAKERS_ID.into()), echo_cancel: true },
            live_tap: Some(LiveTapConfig::default()),
            ..RecorderConfig::default()
        };
        let levels = Arc::new(Mutex::new(0usize));
        let seen = Arc::clone(&levels);
        let recorder = Recorder::start_with(&backend, config, move |_| *seen.lock().unwrap() += 1).unwrap();
        let mut tap = recorder.live_consumer().unwrap();
        wait_until(|| recorder.elapsed() >= Duration::from_millis(800));
        let recording = recorder.stop().unwrap();
        assert!(!backend.is_running(), "both streams are released");
        assert_eq!(recording.sample_rate_hz, 16_000);
        assert!(recording.samples.len() >= 12_800, "{} samples", recording.samples.len());
        // Past the resamplers' and the canceller's start-up: the tone at −3 dB, give or take.
        let steady = &recording.samples[3_200..];
        let rms = (steady.iter().map(|&s| (f32::from(s) / 32_768.0).powi(2)).sum::<f32>() / steady.len() as f32).sqrt();
        let expected = crate::mix::MIX_GAIN * 0.2 / std::f32::consts::SQRT_2;
        assert!((rms / expected - 1.0).abs() < 0.25, "the microphone comes through: rms {rms}, expected about {expected}");
        let mut buf = vec![0.0f32; 4096];
        assert!(tap.read(&mut buf) > 0, "the tap follows the mix");
        assert!(*levels.lock().unwrap() > 0, "levels are reported");
    }

    /// A long take (docs/dictation.md §22): the in-memory take stops at `max_duration`, the stream
    /// carries the whole signal on at 16 kHz and closes when the recorder stops.
    #[test]
    fn a_long_take_streams_past_the_in_memory_part() {
        let backend = FakeBackend::new();
        assert!(Recorder::start_with(&backend, RecorderConfig::default(), |_| {}).unwrap().pcm_consumer().is_none(), "no stream unless asked");
        let config = RecorderConfig { max_duration: Duration::from_millis(300), pcm_stream: Some(PcmStreamConfig::default()), ..RecorderConfig::default() };
        let recorder = Recorder::start_with(&backend, config, |_| {}).unwrap();
        let mut stream = recorder.pcm_consumer().unwrap();
        assert!(recorder.pcm_consumer().is_none(), "handed out once");
        let mut got = 0usize;
        let mut buf = vec![0.0f32; 4096];
        // Read while recording, as the core's recording thread does: a second of audio.
        wait_until(|| {
            got += stream.read(&mut buf);
            got >= 16_000
        });
        assert!(recorder.is_truncated(), "the in-memory take stopped at 300 ms");
        let recording = recorder.stop().unwrap();
        assert!(recording.truncated && recording.duration_ms <= 300, "{}", recording.duration_ms);
        loop {
            let n = stream.read(&mut buf);
            got += n;
            if n == 0 && stream.gap().is_none() {
                break;
            }
        }
        assert!(stream.is_closed());
        assert!(got >= 16_000);
        assert_eq!(stream.sample_rate_hz(), 16_000);
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
    }
}
