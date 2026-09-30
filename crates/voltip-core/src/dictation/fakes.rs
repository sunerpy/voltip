//! In-memory implementations of the dictation ports. **Test / headless support only**: they are a
//! plain public module so the bridge and the shells can build a working core without a microphone
//! or a network, but nothing here belongs in a production wiring.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;

use super::engine::DictationPorts;
use super::ports::{
    AudioSource, Capture, CaptureOptions, DictationError, ForegroundApp, ForegroundProbe, InjectNote, Injection, Injector, LIVE_CHUNK_SAMPLES, LevelFrame,
    LivePcm, PCM_SAMPLE_RATE_HZ, PcmStream, Recording, RefineHints, Refined, Refiner, Segment, SelectionTiming, StreamEvent, StreamFinal, StreamingSession,
    StreamingTranscriber, Transcriber, Transcript, Via,
};
use super::wav;
use crate::hotkey::Modifier;

/// Sample rate of the synthetic recordings.
pub const SAMPLE_RATE_HZ: u32 = 16_000;
/// Text the default fake transcriber returns.
pub const FAKE_TRANSCRIPT: &str = "你好，世界";
/// Latency the fakes report.
pub const FAKE_LATENCY_MS: u64 = 42;
/// How long a held fake call (a gated injection or copy, a held recogniser session, a hanging
/// probe) waits to be released before it gives up with a panic.
const HOLD_LIMIT: Duration = Duration::from_secs(30);

/// Wait on `cv` while `held` is true of the guarded value, for at most `limit`, and panic past it.
/// A test that fails while a fake call is held never releases it, and dropping a runtime waits for
/// every blocking thread: without the limit that test hangs the whole run instead of failing.
fn wait_released<'a, T>(
    guard: std::sync::MutexGuard<'a, T>,
    cv: &std::sync::Condvar,
    limit: Duration,
    held: impl FnMut(&mut T) -> bool,
) -> std::sync::MutexGuard<'a, T> {
    let (guard, wait) = cv.wait_timeout_while(guard, limit, held).unwrap_or_else(std::sync::PoisonError::into_inner);
    if wait.timed_out() {
        drop(guard);
        panic!("a held fake call was not released within {limit:?}");
    }
    guard
}

/// A mono 16 kHz recording of a 440 Hz tone at −6 dBFS, `ms` long.
pub fn speech_recording(ms: u64) -> Recording {
    let n = usize::try_from(u64::from(SAMPLE_RATE_HZ) * ms / 1000).unwrap_or(0);
    let samples: Vec<i16> = (0..n)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE_HZ as f32;
            ((t * 440.0 * std::f32::consts::TAU).sin() * 16_000.0) as i16
        })
        .collect();
    Recording { wav: wav::encode_pcm16(&samples, SAMPLE_RATE_HZ), duration_ms: ms, sample_rate_hz: SAMPLE_RATE_HZ }
}

/// A recording of nothing, `ms` long.
pub fn silent_recording(ms: u64) -> Recording {
    let n = usize::try_from(u64::from(SAMPLE_RATE_HZ) * ms / 1000).unwrap_or(0);
    Recording { wav: wav::encode_pcm16(&vec![0; n], SAMPLE_RATE_HZ), duration_ms: ms, sample_rate_hz: SAMPLE_RATE_HZ }
}

#[derive(Clone)]
enum AudioMode {
    Record(Recording),
    FailStart(String),
    FailStop(String),
}

/// Microphone stand-in: hands out a fixed recording (or fails), counts starts and stops, emits a
/// few level frames and the `ready` mark on start, and — when asked for `live` — a 16 kHz tap that
/// replays the recording's samples ([`FakeLivePcm`]) and closes when the capture stops.
pub struct FakeAudio {
    mode: AudioMode,
    starts: AtomicUsize,
    stops: Arc<AtomicUsize>,
    live_requests: AtomicUsize,
    /// Skip the `on_ready` call (a device that never delivers audio).
    never_ready: bool,
    /// The live tap reports an overrun from its first read.
    tap_overrun: bool,
    /// `max_duration` of every successful start, in order.
    max_durations: Mutex<Vec<Duration>>,
    /// Every start's options, in order (docs/dictation.md §22: source, length, `long`).
    options: Mutex<Vec<CaptureOptions>>,
    /// The device id of every start, in order (`None` = the default input).
    devices: Mutex<Vec<Option<String>>>,
    /// A long take's stream (docs/dictation.md §22): its length in samples and where samples go
    /// missing (`at`, `len`); handed out when a start asks for `long`.
    long: Option<(u64, Vec<(u64, u64)>)>,
}

impl FakeAudio {
    /// 1.5 s of speech-like audio.
    pub fn speech() -> Self {
        Self::recording(speech_recording(1500))
    }

    /// 1.5 s of silence.
    pub fn silence() -> Self {
        Self::recording(silent_recording(1500))
    }

    /// A recording that is too short to upload.
    pub fn short() -> Self {
        Self::recording(speech_recording(100))
    }

    /// Any recording.
    pub fn recording(recording: Recording) -> Self {
        Self::with_mode(AudioMode::Record(recording))
    }

    /// `start` fails with `Audio(message)`.
    pub fn failing_start(message: &str) -> Self {
        Self::with_mode(AudioMode::FailStart(message.to_owned()))
    }

    /// `start` succeeds, `stop` fails with `Audio(message)`.
    pub fn failing_stop(message: &str) -> Self {
        Self::with_mode(AudioMode::FailStop(message.to_owned()))
    }

    /// A device that opens but never delivers a sample: `on_ready` is never called.
    pub fn never_ready(self) -> Self {
        Self { never_ready: true, ..self }
    }

    /// The live tap reports an overrun at once (the decoder "fell behind").
    pub fn overrunning(self) -> Self {
        Self { tap_overrun: true, ..self }
    }

    /// A start that asks for a long take's stream gets one of `seconds` ([`FakePcmStream`]).
    pub fn long(self, seconds: u64) -> Self {
        Self { long: Some((seconds * u64::from(PCM_SAMPLE_RATE_HZ), Vec::new())), ..self }
    }

    /// The long take's stream loses `len_s` seconds at `at_s` (the shell's buffer overflowed).
    pub fn with_gap(mut self, at_s: u64, len_s: u64) -> Self {
        let rate = u64::from(PCM_SAMPLE_RATE_HZ);
        if let Some((_, gaps)) = &mut self.long {
            gaps.push((at_s * rate, len_s * rate));
        }
        self
    }

    fn with_mode(mode: AudioMode) -> Self {
        Self {
            mode,
            starts: AtomicUsize::new(0),
            stops: Arc::new(AtomicUsize::new(0)),
            live_requests: AtomicUsize::new(0),
            never_ready: false,
            tap_overrun: false,
            max_durations: Mutex::new(Vec::new()),
            options: Mutex::new(Vec::new()),
            devices: Mutex::new(Vec::new()),
            long: None,
        }
    }

    /// Successful `start` calls so far.
    pub fn starts(&self) -> usize {
        self.starts.load(Ordering::SeqCst)
    }

    /// `stop` calls so far (including failing ones).
    pub fn stops(&self) -> usize {
        self.stops.load(Ordering::SeqCst)
    }

    /// Successful `start` calls that asked for a live tap.
    pub fn live_requests(&self) -> usize {
        self.live_requests.load(Ordering::SeqCst)
    }

    /// The recording cap every successful `start` was asked for, in order.
    pub fn max_durations(&self) -> Vec<Duration> {
        self.max_durations.lock().clone()
    }

    /// The options of every start, in order.
    pub fn options(&self) -> Vec<CaptureOptions> {
        self.options.lock().clone()
    }

    /// The device id every start asked for, in order (`None` = the default input).
    pub fn devices(&self) -> Vec<Option<String>> {
        self.devices.lock().clone()
    }
}

struct FakeCapture {
    result: Option<Result<Recording, DictationError>>,
    stops: Arc<AtomicUsize>,
    live: Option<Box<dyn LivePcm>>,
    pcm: Option<Box<dyn PcmStream>>,
    closed: Arc<AtomicBool>,
}

impl Capture for FakeCapture {
    fn stop(mut self: Box<Self>) -> Result<Recording, DictationError> {
        self.stops.fetch_add(1, Ordering::SeqCst);
        self.closed.store(true, Ordering::SeqCst);
        self.result.take().unwrap_or_else(|| Err(DictationError::Audio("fake capture stopped twice".into())))
    }

    fn live_pcm(&mut self) -> Option<Box<dyn LivePcm>> {
        self.live.take()
    }

    fn pcm_stream(&mut self) -> Option<Box<dyn PcmStream>> {
        self.pcm.take()
    }
}

/// A fake long take's stream (docs/dictation.md §22): a signal loud except a 300 ms pause before
/// every 30 s mark, generated as it is read — two hours never sit in memory — with samples missing
/// where the fake says; everything is there at once, and the stream ends when the capture stops.
pub struct FakePcmStream {
    total: u64,
    /// The stream's timeline position: samples delivered and gaps reported.
    pos: u64,
    gaps: std::collections::VecDeque<(u64, u64)>,
    closed: Arc<AtomicBool>,
}

impl FakePcmStream {
    /// The fake signal at sample `i`.
    pub fn sample(i: u64) -> f32 {
        let period = 30 * u64::from(PCM_SAMPLE_RATE_HZ);
        let pause = 3 * u64::from(PCM_SAMPLE_RATE_HZ) / 10;
        if i % period >= period - pause {
            0.0
        } else if i.is_multiple_of(2) {
            0.3
        } else {
            -0.3
        }
    }
}

impl PcmStream for FakePcmStream {
    fn read(&mut self, out: &mut [f32]) -> usize {
        let until = self.gaps.front().map_or(self.total, |g| g.0.min(self.total));
        let n = usize::try_from(until.saturating_sub(self.pos)).unwrap_or(usize::MAX).min(out.len());
        for (k, slot) in out[..n].iter_mut().enumerate() {
            *slot = Self::sample(self.pos + k as u64);
        }
        self.pos += n as u64;
        n
    }

    fn gap(&mut self) -> Option<u64> {
        let &(at, len) = self.gaps.front()?;
        if at != self.pos {
            return None;
        }
        self.gaps.pop_front();
        self.pos += len;
        Some(len)
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst) && self.pos >= self.total
    }
}

impl Drop for FakeCapture {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
    }
}

/// The fake capture's live tap: replays the recording as 16 kHz `f32`, [`LIVE_CHUNK_SAMPLES`] per
/// read, then returns `0` until the capture stops (or is dropped), which closes it.
pub struct FakeLivePcm {
    samples: Vec<f32>,
    offset: usize,
    closed: Arc<AtomicBool>,
    overrun: bool,
}

impl FakeLivePcm {
    /// A tap over `samples` that closes when `closed` flips.
    pub fn new(samples: Vec<f32>, closed: Arc<AtomicBool>) -> Self {
        Self { samples, offset: 0, closed, overrun: false }
    }

    /// Report an overrun from the first read on (the decoder "fell behind").
    pub fn with_overrun(self) -> Self {
        Self { overrun: true, ..self }
    }
}

impl LivePcm for FakeLivePcm {
    fn read(&mut self, out: &mut [f32]) -> usize {
        let n = out.len().min(LIVE_CHUNK_SAMPLES).min(self.samples.len() - self.offset);
        out[..n].copy_from_slice(&self.samples[self.offset..self.offset + n]);
        self.offset += n;
        n
    }

    fn overrun(&self) -> bool {
        self.overrun
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}

impl AudioSource for FakeAudio {
    fn start(
        &self,
        device_id: Option<&str>,
        on_level: Box<dyn Fn(LevelFrame) + Send>,
        on_ready: Box<dyn FnOnce() + Send>,
        options: CaptureOptions,
    ) -> Result<Box<dyn Capture>, DictationError> {
        self.devices.lock().push(device_id.map(str::to_owned));
        let result = match &self.mode {
            AudioMode::FailStart(m) => return Err(DictationError::Audio(m.clone())),
            AudioMode::FailStop(m) => Err(DictationError::Audio(m.clone())),
            AudioMode::Record(r) => Ok(r.clone()),
        };
        self.starts.fetch_add(1, Ordering::SeqCst);
        self.max_durations.lock().push(options.max_duration);
        self.options.lock().push(options.clone());
        let live = options.live;
        for seq in 0..3 {
            on_level(LevelFrame { rms_dbfs: -20.0 - seq as f32, peak_dbfs: -6.0, clipping: false, sample_rate_hz: SAMPLE_RATE_HZ, channels: 1, seq });
        }
        if !self.never_ready {
            on_ready();
        }
        let closed = Arc::new(AtomicBool::new(false));
        let live_tap: Option<Box<dyn LivePcm>> = match (&result, live) {
            (Ok(recording), true) => {
                self.live_requests.fetch_add(1, Ordering::SeqCst);
                let (frames, _) = wav::pcm_data(&recording.wav).unwrap_or_default().as_chunks::<2>();
                let samples = frames.iter().map(|b| f32::from(i16::from_le_bytes(*b)) / 32_768.0).collect();
                let tap = FakeLivePcm::new(samples, closed.clone());
                Some(Box::new(if self.tap_overrun { tap.with_overrun() } else { tap }))
            }
            _ => None,
        };
        let pcm: Option<Box<dyn PcmStream>> = match (&self.long, options.long) {
            (Some((total, gaps)), true) => {
                Some(Box::new(FakePcmStream { total: *total, pos: 0, gaps: gaps.iter().copied().collect(), closed: closed.clone() }))
            }
            _ => None,
        };
        Ok(Box::new(FakeCapture { result: Some(result), stops: self.stops.clone(), live: live_tap, pcm, closed }))
    }
}

#[derive(Clone)]
enum Reply {
    Ok(String),
    Err(DictationError),
    Slow(String, Duration),
    /// 「第n段」 (padded with `pad` 字) for call `n` (1-based), failing the calls in `fail`, each
    /// after `delay` (docs/dictation.md §22: segments in order).
    Numbered {
        pad: usize,
        fail: Vec<usize>,
        delay: Duration,
    },
}

/// ASR stand-in.
pub struct FakeTranscriber {
    reply: Reply,
    calls: AtomicUsize,
    languages: Mutex<Vec<Option<String>>>,
    /// Length of every WAV received, in milliseconds at 16 kHz mono (`0` when it did not parse).
    durations: Mutex<Vec<u64>>,
    /// The glossary of every call (docs/dictation.md §16.3).
    glossaries: Mutex<Vec<Vec<String>>>,
    /// The language of every warm-up (docs/dictation.md §10.7).
    warms: Mutex<Vec<Option<String>>>,
}

impl FakeTranscriber {
    /// Always returns `text`.
    pub fn ok(text: &str) -> Self {
        Self::with_reply(Reply::Ok(text.to_owned()))
    }

    /// Always fails with `Asr(message)`.
    pub fn err(message: &str) -> Self {
        Self::with_reply(Reply::Err(DictationError::Asr(message.to_owned())))
    }

    /// Returns `text` after `delay` (tokio time, so tests can pause it).
    pub fn slow(text: &str, delay: Duration) -> Self {
        Self::with_reply(Reply::Slow(text.to_owned(), delay))
    }

    /// Answers call `n` with 「第n段」 plus `pad` times 「字」 and a full stop, failing the calls
    /// numbered in `fail` (1-based), each after `delay` (tokio time).
    pub fn numbered(pad: usize, fail: &[usize], delay: Duration) -> Self {
        Self::with_reply(Reply::Numbered { pad, fail: fail.to_vec(), delay })
    }

    fn with_reply(reply: Reply) -> Self {
        Self {
            reply,
            calls: AtomicUsize::new(0),
            languages: Mutex::new(Vec::new()),
            durations: Mutex::new(Vec::new()),
            glossaries: Mutex::new(Vec::new()),
            warms: Mutex::new(Vec::new()),
        }
    }

    /// Calls so far.
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// Language hints received, in order.
    pub fn languages(&self) -> Vec<Option<String>> {
        self.languages.lock().clone()
    }

    /// Length in milliseconds (16 kHz mono PCM 16-bit) of every WAV received, in order.
    pub fn durations_ms(&self) -> Vec<u64> {
        self.durations.lock().clone()
    }

    /// The glossary every call carried, in order.
    pub fn glossaries(&self) -> Vec<Vec<String>> {
        self.glossaries.lock().clone()
    }

    /// The language hint of every warm-up, in order.
    pub fn warms(&self) -> Vec<Option<String>> {
        self.warms.lock().clone()
    }
}

#[async_trait]
impl Transcriber for FakeTranscriber {
    async fn transcribe(&self, wav: &[u8], language: Option<&str>, glossary: &[String]) -> Result<Transcript, DictationError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.languages.lock().push(language.map(str::to_owned));
        self.glossaries.lock().push(glossary.to_vec());
        self.durations.lock().push(wav::pcm_data(wav).map_or(0, |pcm| pcm.len() as u64 / 2 * 1000 / u64::from(SAMPLE_RATE_HZ)));
        if wav::pcm_data(wav).is_none() {
            return Err(DictationError::Asr("not a wav file".into()));
        }
        match &self.reply {
            Reply::Ok(text) => Ok(Transcript { text: text.clone(), latency_ms: FAKE_LATENCY_MS }),
            Reply::Err(e) => Err(e.clone()),
            Reply::Slow(text, delay) => {
                tokio::time::sleep(*delay).await;
                Ok(Transcript { text: text.clone(), latency_ms: FAKE_LATENCY_MS })
            }
            Reply::Numbered { pad, fail, delay } => {
                let n = self.calls.load(Ordering::SeqCst);
                if !delay.is_zero() {
                    tokio::time::sleep(*delay).await;
                }
                if fail.contains(&n) {
                    return Err(DictationError::Asr(format!("fake failure of call {n}")));
                }
                Ok(Transcript { text: format!("第{n}段{}。", "字".repeat(*pad)), latency_ms: FAKE_LATENCY_MS })
            }
        }
    }

    fn warm(&self, language: Option<&str>) {
        self.warms.lock().push(language.map(str::to_owned));
    }
}

/// Refiner stand-in: records what it was asked to refine (or to edit, docs/dictation.md §19) and
/// with which hints (glossary, language, style, context); both answer with the configured reply.
pub struct FakeRefiner {
    reply: Reply,
    calls: AtomicUsize,
    inputs: Mutex<Vec<(String, RefineHints)>>,
    edits: Mutex<Vec<(String, String, RefineHints)>>,
}

/// Model name the fake refiner reports.
pub const FAKE_REFINE_MODEL: &str = "fake/refiner";

impl FakeRefiner {
    fn with_reply(reply: Reply) -> Self {
        Self { reply, calls: AtomicUsize::new(0), inputs: Mutex::new(Vec::new()), edits: Mutex::new(Vec::new()) }
    }

    /// Always returns `text`.
    pub fn ok(text: &str) -> Self {
        Self::with_reply(Reply::Ok(text.to_owned()))
    }

    /// Always fails with `Refine(message)`.
    pub fn err(message: &str) -> Self {
        Self::with_reply(Reply::Err(DictationError::Refine(message.to_owned())))
    }

    /// Returns `text` after `delay`.
    pub fn slow(text: &str, delay: Duration) -> Self {
        Self::with_reply(Reply::Slow(text.to_owned(), delay))
    }

    /// Calls so far.
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// Every `(text, glossary)` it was given, in order.
    pub fn inputs(&self) -> Vec<(String, Vec<String>)> {
        self.inputs.lock().iter().map(|(text, hints)| (text.clone(), hints.glossary.clone())).collect()
    }

    /// Every hints struct it was given, in order (docs/dictation.md §18.5).
    pub fn hints(&self) -> Vec<RefineHints> {
        self.inputs.lock().iter().map(|(_, hints)| hints.clone()).collect()
    }

    /// Every `(selection, instruction, glossary)` an edit gave it, in order.
    pub fn edits(&self) -> Vec<(String, String, Vec<String>)> {
        self.edits.lock().iter().map(|(selection, instruction, hints)| (selection.clone(), instruction.clone(), hints.glossary.clone())).collect()
    }

    /// Every hints struct an edit was given, in order (docs/dictation.md §19).
    pub fn edit_hints(&self) -> Vec<RefineHints> {
        self.edits.lock().iter().map(|(_, _, hints)| hints.clone()).collect()
    }

    async fn answer(&self) -> Result<Refined, DictationError> {
        match &self.reply {
            Reply::Ok(text) => Ok(Refined { text: text.clone(), latency_ms: FAKE_LATENCY_MS, model: FAKE_REFINE_MODEL.into() }),
            Reply::Err(e) => Err(e.clone()),
            Reply::Slow(text, delay) => {
                tokio::time::sleep(*delay).await;
                Ok(Refined { text: text.clone(), latency_ms: FAKE_LATENCY_MS, model: FAKE_REFINE_MODEL.into() })
            }
            Reply::Numbered { .. } => {
                let n = self.calls.load(Ordering::SeqCst);
                Ok(Refined { text: format!("润色第{n}次。"), latency_ms: FAKE_LATENCY_MS, model: FAKE_REFINE_MODEL.into() })
            }
        }
    }
}

#[async_trait]
impl Refiner for FakeRefiner {
    async fn refine(&self, text: &str, hints: &RefineHints) -> Result<Refined, DictationError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inputs.lock().push((text.to_owned(), hints.clone()));
        self.answer().await
    }

    async fn edit(&self, selection: &str, instruction: &str, hints: &RefineHints) -> Result<Refined, DictationError> {
        self.edits.lock().push((selection.to_owned(), instruction.to_owned(), hints.clone()));
        self.answer().await
    }
}

/// What a [`FakeProbe`] answers.
#[derive(Clone, Debug)]
enum ProbeReply {
    App(ForegroundApp),
    Nothing,
    Err(String),
    Panic,
    /// Block until [`FakeProbe::release`], then answer nothing (a hung window system).
    Hang,
}

/// Foreground-probe stand-in (docs/dictation.md §18.2): answers with a fixed app (changeable
/// between takes), nothing, an error, a panic, or hangs until released; counts calls.
pub struct FakeProbe {
    reply: Mutex<ProbeReply>,
    calls: AtomicUsize,
    gate: Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
}

impl FakeProbe {
    fn with(reply: ProbeReply) -> Self {
        Self { reply: Mutex::new(reply), calls: AtomicUsize::new(0), gate: Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new())) }
    }

    /// Always `app_id` / `name` with `title`.
    pub fn app(app_id: &str, name: &str, title: Option<&str>) -> Self {
        Self::with(ProbeReply::App(ForegroundApp { app_id: app_id.to_owned(), name: name.to_owned(), title: title.map(str::to_owned), window: None }))
    }

    /// No nameable application (pure Wayland, the desktop, Voltip itself).
    pub fn nothing() -> Self {
        Self::with(ProbeReply::Nothing)
    }

    /// The window system refused (`Err(message)`).
    pub fn failing(message: &str) -> Self {
        Self::with(ProbeReply::Err(message.to_owned()))
    }

    /// The probe panics.
    pub fn panicking() -> Self {
        Self::with(ProbeReply::Panic)
    }

    /// Every call blocks until [`FakeProbe::release`], then answers nothing.
    pub fn hanging() -> Self {
        Self::with(ProbeReply::Hang)
    }

    /// Answer `app_id` / `name` / `title` from the next call on.
    pub fn set_app(&self, app_id: &str, name: &str, title: Option<&str>) {
        *self.reply.lock() = ProbeReply::App(ForegroundApp { app_id: app_id.to_owned(), name: name.to_owned(), title: title.map(str::to_owned), window: None });
    }

    /// The answered application's window from the next call on (nothing when no app is answered).
    pub fn set_window(&self, window: Option<u64>) {
        if let ProbeReply::App(app) = &mut *self.reply.lock() {
            app.window = window;
        }
    }

    /// Answer nothing from the next call on.
    pub fn set_nothing(&self) {
        *self.reply.lock() = ProbeReply::Nothing;
    }

    /// Let hanging calls (current and future) return.
    pub fn release(&self) {
        let (open, cv) = &*self.gate;
        *open.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        cv.notify_all();
    }

    /// Calls so far.
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl ForegroundProbe for FakeProbe {
    fn foreground(&self) -> Result<Option<ForegroundApp>, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let reply = self.reply.lock().clone();
        match reply {
            ProbeReply::App(app) => Ok(Some(app)),
            ProbeReply::Nothing => Ok(None),
            ProbeReply::Err(e) => Err(e),
            ProbeReply::Panic => panic!("fake probe panicked"),
            ProbeReply::Hang => {
                let (open, cv) = &*self.gate;
                let released = open.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                drop(wait_released(released, cv, HOLD_LIMIT, |released| !*released));
                Ok(None)
            }
        }
    }
}

#[derive(Clone)]
enum InjectReply {
    Paste,
    Clipboard(Option<InjectNote>),
    Err(String),
}

/// A blocking gate of the fake injector: a call waits until a permit is released, for at most
/// `limit` ([`HOLD_LIMIT`]).
struct Gate {
    permits: std::sync::Mutex<usize>,
    cv: std::sync::Condvar,
    waiting: AtomicUsize,
    limit: Duration,
}

impl Default for Gate {
    fn default() -> Self {
        Self { permits: std::sync::Mutex::new(0), cv: std::sync::Condvar::new(), waiting: AtomicUsize::new(0), limit: HOLD_LIMIT }
    }
}

impl Gate {
    /// Block until a permit is there, then take it.
    fn pass(&self) {
        let permits = self.permits.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        self.waiting.fetch_add(1, Ordering::SeqCst);
        let mut permits = wait_released(permits, &self.cv, self.limit, |permits| *permits == 0);
        *permits -= 1;
        self.waiting.fetch_sub(1, Ordering::SeqCst);
    }

    fn release(&self, n: usize) {
        *self.permits.lock().unwrap_or_else(std::sync::PoisonError::into_inner) += n;
        self.cv.notify_all();
    }
}

/// Injector stand-in: records every text it was given. Optionally gated ([`FakeInjector::gated`]):
/// every call then blocks until a permit is [`FakeInjector::release`]d, so a test can observe what
/// the core does while an injection is in flight. For voice edit (docs/dictation.md §19) it also
/// plays the foreground application's selection ([`FakeInjector::with_selection`]; nothing is
/// selected by default), records every copy with its held modifiers, can gate the copies
/// ([`FakeInjector::copy_gated`]) and report [`SelectionTiming::AfterKeyUp`].
pub struct FakeInjector {
    reply: InjectReply,
    /// Replies for the first calls, in order; `reply` after they run out.
    first: Mutex<std::collections::VecDeque<InjectReply>>,
    injected: Mutex<Vec<String>>,
    gate: Option<Arc<Gate>>,
    selection: Result<Option<String>, DictationError>,
    timing: SelectionTiming,
    copies: Mutex<Vec<Vec<Modifier>>>,
    copy_gate: Option<Arc<Gate>>,
    /// Texts put on the clipboard alone (`Injector::copy`).
    clipboard: Mutex<Vec<String>>,
    /// Which application ids are terminals (docs/dictation.md §19.2).
    terminals: Arc<dyn Fn(&str) -> bool + Send + Sync>,
}

impl FakeInjector {
    /// Reports a successful paste.
    pub fn paste() -> Self {
        Self::with_reply(InjectReply::Paste)
    }

    /// Reports the text left in the clipboard (`note` = why the paste did not happen, or `None`
    /// for clipboard-only mode).
    pub fn clipboard(note: Option<&str>) -> Self {
        Self::with_reply(InjectReply::Clipboard(note.map(InjectNote::other)))
    }

    /// Reports the text left in the clipboard for `note` (a coded fallback, docs/dictation.md §4.2).
    pub fn clipboard_with(note: InjectNote) -> Self {
        Self::with_reply(InjectReply::Clipboard(Some(note)))
    }

    /// Fails with `Inject(message)`.
    pub fn err(message: &str) -> Self {
        Self::with_reply(InjectReply::Err(message.to_owned()))
    }

    fn with_reply(reply: InjectReply) -> Self {
        Self {
            reply,
            first: Mutex::new(Default::default()),
            injected: Mutex::new(Vec::new()),
            gate: None,
            selection: Ok(None),
            timing: SelectionTiming::AtPress,
            copies: Mutex::new(Vec::new()),
            copy_gate: None,
            clipboard: Mutex::new(Vec::new()),
            terminals: Arc::new(|_: &str| false),
        }
    }

    /// The foreground application has `text` selected: every copy answers it.
    pub fn with_selection(self, text: &str) -> Self {
        Self { selection: Ok(Some(text.to_owned())), ..self }
    }

    /// Every copy fails with `error` (no copy tool, clipboard unusable).
    pub fn with_selection_error(self, error: DictationError) -> Self {
        Self { selection: Err(error), ..self }
    }

    /// The applications `terminals` says yes to are terminals (a host's table, docs/dictation.md
    /// §19.2); by default nothing is.
    pub fn with_terminals(self, terminals: impl Fn(&str) -> bool + Send + Sync + 'static) -> Self {
        Self { terminals: Arc::new(terminals), ..self }
    }

    /// Report [`SelectionTiming::AfterKeyUp`] (an X11 session).
    pub fn after_key_up(self) -> Self {
        Self { timing: SelectionTiming::AfterKeyUp, ..self }
    }

    /// Every copy blocks until [`FakeInjector::release_copies`] grants it a permit.
    pub fn copy_gated(self) -> Self {
        Self { copy_gate: Some(Arc::new(Gate::default())), ..self }
    }

    /// Let `n` gated copies through.
    pub fn release_copies(&self, n: usize) {
        if let Some(gate) = &self.copy_gate {
            gate.release(n);
        }
    }

    /// Gated copies blocked right now.
    pub fn copies_waiting(&self) -> usize {
        self.copy_gate.as_ref().map_or(0, |g| g.waiting.load(Ordering::SeqCst))
    }

    /// The held modifiers of every copy, in order (one entry per copy).
    pub fn copies(&self) -> Vec<Vec<Modifier>> {
        self.copies.lock().clone()
    }

    /// The first call is answered with the clipboard fallback (`note`), later ones as configured.
    pub fn clipboard_once(self, note: Option<&str>) -> Self {
        self.first.lock().push_back(InjectReply::Clipboard(note.map(InjectNote::other)));
        self
    }

    /// The first call fails with `Inject(message)`, later ones are answered as configured.
    pub fn err_once(self, message: &str) -> Self {
        self.first.lock().push_back(InjectReply::Err(message.to_owned()));
        self
    }

    /// Every `inject` and `copy` blocks until [`FakeInjector::release`] grants it a permit.
    pub fn gated(self) -> Self {
        Self { gate: Some(Arc::new(Gate::default())), ..self }
    }

    /// Let `n` gated calls through (now or when they arrive).
    pub fn release(&self, n: usize) {
        if let Some(gate) = &self.gate {
            gate.release(n);
        }
    }

    /// Gated calls blocked right now.
    pub fn waiting(&self) -> usize {
        self.gate.as_ref().map_or(0, |g| g.waiting.load(Ordering::SeqCst))
    }

    /// Texts handed over, in order (including failed attempts).
    pub fn injected(&self) -> Vec<String> {
        self.injected.lock().clone()
    }

    /// Texts put on the clipboard alone (`Injector::copy`), in order.
    pub fn clipboard_copies(&self) -> Vec<String> {
        self.clipboard.lock().clone()
    }
}

impl Injector for FakeInjector {
    fn inject(&self, text: &str) -> Result<Injection, DictationError> {
        if let Some(gate) = &self.gate {
            gate.pass();
        }
        self.injected.lock().push(text.to_owned());
        let reply = self.first.lock().pop_front().unwrap_or_else(|| self.reply.clone());
        match reply {
            InjectReply::Paste => Ok(Injection { via: Via::Paste, note: None }),
            InjectReply::Clipboard(note) => Ok(Injection { via: Via::Clipboard, note }),
            InjectReply::Err(m) => Err(DictationError::Inject(m)),
        }
    }

    fn copy(&self, text: &str) -> Result<(), DictationError> {
        if let Some(gate) = &self.gate {
            gate.pass();
        }
        self.clipboard.lock().push(text.to_owned());
        Ok(())
    }

    fn copy_selection(&self, held: &[Modifier]) -> Result<Option<String>, DictationError> {
        if let Some(gate) = &self.copy_gate {
            gate.pass();
        }
        self.copies.lock().push(held.to_vec());
        self.selection.clone()
    }

    fn selection_timing(&self) -> SelectionTiming {
        self.timing
    }

    fn is_terminal_app(&self, app_id: &str) -> bool {
        (self.terminals)(app_id)
    }
}

/// What a [`FakeStreaming`] session does on each feed.
#[derive(Clone, Debug)]
enum StreamScript {
    /// `open` fails with `Asr(message)`.
    FailOpen(String),
    /// Every feed of ≥ 100 ms grows the current sentence by one word of `words`; every `endpoint_every`
    /// words an endpoint commits the sentence; `error_after` feeds (if any) then raise `Error`.
    Words { words: Vec<String>, endpoint_every: usize, error_after: Option<usize> },
}

/// Streaming recogniser stand-in (docs/dictation.md §11): a scripted session whose partials grow
/// word by word per 100 ms of audio fed, with an endpoint every few words; counts opens and warms.
pub struct FakeStreaming {
    script: StreamScript,
    opens: AtomicUsize,
    warms: AtomicUsize,
    finishes: Arc<AtomicUsize>,
    /// The word a session waits before, and its gate ([`FakeStreaming::holding`]).
    hold: Option<(usize, Arc<Gate>)>,
}

/// The words the default [`FakeStreaming::script`] session emits, one per 100 ms chunk.
pub const FAKE_LIVE_WORDS: [&str; 6] = ["你好", "你好，", "你好，世界", "你好，世界。", "今天", "今天天气"];

impl FakeStreaming {
    /// The default script: [`FAKE_LIVE_WORDS`], an endpoint after the fourth word (`你好，世界。`
    /// is committed), then `今天` / `今天天气` as the next sentence.
    pub fn script() -> Self {
        Self::words(FAKE_LIVE_WORDS.iter().map(|w| (*w).to_owned()).collect(), 4)
    }

    /// A script of `words` (cumulative texts of the sentence) with an endpoint every `endpoint_every` words.
    pub fn words(words: Vec<String>, endpoint_every: usize) -> Self {
        Self {
            script: StreamScript::Words { words, endpoint_every: endpoint_every.max(1), error_after: None },
            opens: AtomicUsize::new(0),
            warms: AtomicUsize::new(0),
            finishes: Arc::new(AtomicUsize::new(0)),
            hold: None,
        }
    }

    /// Sessions wait before word `index` (from 0) until [`FakeStreaming::release_hold`]: the decode
    /// thread stops there, as a busy machine may stop it.
    pub fn holding(self, index: usize) -> Self {
        Self { hold: Some((index, Arc::new(Gate::default()))), ..self }
    }

    /// Let a session waiting before the held word go on (now or when it gets there).
    pub fn release_hold(&self) {
        if let Some((_, gate)) = &self.hold {
            gate.release(1);
        }
    }

    /// `open` fails (model not installed, load error).
    pub fn failing_open(message: &str) -> Self {
        Self { script: StreamScript::FailOpen(message.to_owned()), ..Self::script() }
    }

    /// The default script, but the session reports `Error` after `feeds` feeds.
    pub fn erroring_after(feeds: usize) -> Self {
        let mut fake = Self::script();
        if let StreamScript::Words { error_after, .. } = &mut fake.script {
            *error_after = Some(feeds);
        }
        fake
    }

    /// `open` calls so far.
    pub fn opens(&self) -> usize {
        self.opens.load(Ordering::SeqCst)
    }

    /// `warm` calls so far.
    pub fn warms(&self) -> usize {
        self.warms.load(Ordering::SeqCst)
    }

    /// `finish` calls so far (across sessions).
    pub fn finishes(&self) -> usize {
        self.finishes.load(Ordering::SeqCst)
    }
}

impl StreamingTranscriber for FakeStreaming {
    fn open(&self, _language: Option<&str>) -> Result<Box<dyn StreamingSession>, DictationError> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        match &self.script {
            StreamScript::FailOpen(m) => Err(DictationError::Asr(m.clone())),
            StreamScript::Words { words, endpoint_every, error_after } => Ok(Box::new(FakeSession {
                words: words.clone(),
                endpoint_every: *endpoint_every,
                error_after: *error_after,
                feeds: 0,
                fed_samples: 0,
                word: 0,
                sentence_start_ms: 0,
                committed: Vec::new(),
                pending: std::collections::VecDeque::new(),
                finishes: self.finishes.clone(),
                hold: self.hold.clone(),
            })),
        }
    }

    fn warm(&self) {
        self.warms.fetch_add(1, Ordering::SeqCst);
    }
}

struct FakeSession {
    words: Vec<String>,
    endpoint_every: usize,
    error_after: Option<usize>,
    feeds: usize,
    fed_samples: usize,
    word: usize,
    sentence_start_ms: u64,
    committed: Vec<Segment>,
    pending: std::collections::VecDeque<StreamEvent>,
    finishes: Arc<AtomicUsize>,
    hold: Option<(usize, Arc<Gate>)>,
}

impl FakeSession {
    fn fed_ms(&self) -> u64 {
        (self.fed_samples / 16) as u64
    }
}

impl StreamingSession for FakeSession {
    fn feed(&mut self, pcm16k: &[f32]) {
        self.feeds += 1;
        self.fed_samples += pcm16k.len();
        if self.error_after.is_some_and(|n| self.feeds > n) {
            self.pending.push_back(StreamEvent::Error("fake decoder failed".into()));
            return;
        }
        let Some(text) = self.words.get(self.word) else { return };
        if let Some((_, gate)) = self.hold.as_ref().filter(|(at, _)| *at == self.word) {
            gate.pass();
        }
        self.word += 1;
        if self.word.is_multiple_of(self.endpoint_every) {
            let segment = Segment { text: text.clone(), start_ms: self.sentence_start_ms, end_ms: self.fed_ms() };
            self.sentence_start_ms = self.fed_ms();
            self.committed.push(segment.clone());
            self.pending.push_back(StreamEvent::Endpoint { text: segment.text, start_ms: segment.start_ms, end_ms: segment.end_ms });
        } else {
            self.pending.push_back(StreamEvent::Partial { current: text.clone() });
        }
    }

    fn poll(&mut self) -> StreamEvent {
        self.pending.pop_front().unwrap_or(StreamEvent::Idle)
    }

    fn finish(self: Box<Self>) -> Result<StreamFinal, DictationError> {
        self.finishes.fetch_add(1, Ordering::SeqCst);
        let tail =
            if self.word.is_multiple_of(self.endpoint_every) { String::new() } else { self.words.get(self.word.wrapping_sub(1)).cloned().unwrap_or_default() };
        Ok(StreamFinal { committed: self.committed, tail })
    }
}

/// Engine settings for tests that run the core with these fakes: recognition on a custom endpoint
/// (a test build has no built-in service, whose absence would fall back to the on-device model the
/// fake shells do not have). Nothing is ever requested: the fake factory hands out the clients.
pub fn fake_engines() -> crate::engines::EngineSettings {
    use crate::engines::{EngineSettings, ProviderId, ProviderSettings};
    EngineSettings {
        asr_provider: ProviderId::Custom,
        providers: [(
            ProviderId::Custom,
            ProviderSettings { asr_url: Some("https://asr.fake.test".into()), asr_model: Some(crate::engines::DEFAULT_ASR_MODEL.into()), ..Default::default() },
        )]
        .into(),
        ..EngineSettings::default()
    }
}

/// A finished dictation of `text` as the history keeps it (a whole take, pasted, not cleaned up).
pub fn history_entry(text: &str) -> crate::HistoryEntry {
    crate::HistoryEntry {
        id: uuid::Uuid::new_v4(),
        at_ms: 1_758_700_000_000,
        raw_text: text.to_owned(),
        text: text.to_owned(),
        refined: false,
        asr_model: "fake/asr".to_owned(),
        refine_model: None,
        duration_ms: 1500,
        asr_ms: FAKE_LATENCY_MS,
        refine_ms: None,
        outcome: crate::Outcome::Inserted { via: Via::Paste },
        starred: false,
        mode: crate::OutputMode::WholeTake,
        segments: None,
        live_error: None,
        vocabulary: None,
        kind: crate::TakeKind::Dictation,
        edit: None,
        app: None,
        scene: None,
        preset: None,
        origin: None,
        processed: None,
    }
}

/// Ports that complete the happy path with no network: speech audio, [`FAKE_TRANSCRIPT`], no
/// refiner, a paste injector, no streaming recogniser.
pub fn ports() -> DictationPorts {
    ports_with(Arc::new(FakeAudio::speech()), Arc::new(FakeTranscriber::ok(FAKE_TRANSCRIPT)), None, Arc::new(FakeInjector::paste()))
}

/// Ports from explicit fakes; the factory ignores the configuration and hands out these clients.
pub fn ports_with(audio: Arc<FakeAudio>, transcriber: Arc<FakeTranscriber>, refiner: Option<Arc<FakeRefiner>>, injector: Arc<FakeInjector>) -> DictationPorts {
    let transcriber: Arc<dyn Transcriber> = transcriber;
    let refiner: Option<Arc<dyn Refiner>> = refiner.map(|r| r as Arc<dyn Refiner>);
    DictationPorts {
        audio,
        injector,
        factory: Arc::new(move |_| (transcriber.clone(), refiner.clone())),
        models: None,
        streaming: None,
        probe: None,
        service_probe: None,
        segmenter: None,
    }
}

/// [`ports`] plus a streaming recogniser and a library with the streaming model installed, so
/// live preview is ready out of the box (docs/dictation.md §11) and every `Listening` carries
/// partials from [`FakeStreaming::script`].
pub fn ports_live(streaming: Arc<FakeStreaming>) -> DictationPorts {
    DictationPorts { models: Some(Arc::new(FakeModels::new(1).with_installed(FAKE_STREAMING_MODEL_ID))), streaming: Some(streaming), ..ports() }
}

/// Id of the fake catalogue's streaming entry (the real catalogue's `zipformer-stream-zh-en`).
pub const FAKE_STREAMING_MODEL_ID: &str = "zipformer-stream-zh-en";

/// A model library standing in for `voltip_asr_local::ModelStore`: a fixed catalogue whose
/// downloads finish after `steps` progress reports (or fail / hang for a cancel), so the core's
/// download wiring can be exercised without a network or a model file.
pub struct FakeModels {
    entries: Mutex<Vec<crate::models::ModelState>>,
    /// `Downloading` reports before the fake verifies; `None` = never finishes (until cancelled).
    steps: Option<u32>,
    /// Fail the verification with this message instead of installing.
    fail_with: Option<String>,
    downloads: AtomicUsize,
    removals: AtomicUsize,
}

impl FakeModels {
    /// Three catalogue entries, nothing installed; downloads install after `steps` reports.
    pub fn new(steps: u32) -> Self {
        Self { entries: Mutex::new(Self::catalogue()), steps: Some(steps), fail_with: None, downloads: AtomicUsize::new(0), removals: AtomicUsize::new(0) }
    }

    /// Downloads fail verification with `message` (the `.part` would stay).
    pub fn failing(message: &str) -> Self {
        Self { fail_with: Some(message.to_owned()), ..Self::new(1) }
    }

    /// Downloads never finish on their own: they wait for the cancel token.
    pub fn hanging() -> Self {
        Self { steps: None, ..Self::new(0) }
    }

    /// Mark `id` as installed before the core starts.
    pub fn with_installed(self, id: &str) -> Self {
        for m in self.entries.lock().iter_mut() {
            if m.id == id {
                m.state = crate::models::ModelInstallState::Installed { path: format!("/fake/models/{id}"), installed_at: 1 };
            }
        }
        self
    }

    /// The fake catalogue: the default tier, one light tier and the streaming model of
    /// docs/dictation.md §10 / §11 (sizes are tiny).
    pub fn catalogue() -> Vec<crate::models::ModelState> {
        use crate::models::{CAPABILITY_OFFLINE, CAPABILITY_STREAMING, ModelInstallState, ModelState};
        vec![
            ModelState {
                id: crate::models::DEFAULT_LOCAL_MODEL_ID.into(),
                name: "均衡".into(),
                engine: "transcribe_cpp".into(),
                tier: "balanced".into(),
                capabilities: vec![CAPABILITY_OFFLINE.into()],
                languages: vec!["zh".into(), "en".into()],
                size_bytes: 4096,
                description: "fake".into(),
                recommended: true,
                repo: "example/model".into(),
                active: false,
                state: ModelInstallState::NotInstalled,
            },
            ModelState {
                id: "paraformer-zh".into(),
                name: "Paraformer 中文".into(),
                engine: "paraformer".into(),
                tier: "light".into(),
                capabilities: vec![CAPABILITY_OFFLINE.into()],
                languages: vec!["zh".into(), "en".into()],
                size_bytes: 2048,
                description: "fake".into(),
                recommended: false,
                repo: "example/model".into(),
                active: false,
                state: ModelInstallState::NotInstalled,
            },
            ModelState {
                id: FAKE_STREAMING_MODEL_ID.into(),
                name: "实时预览".into(),
                engine: "zipformer_streaming".into(),
                tier: "streaming".into(),
                capabilities: vec![CAPABILITY_STREAMING.into()],
                languages: vec!["zh".into(), "en".into()],
                size_bytes: 1024,
                description: "fake".into(),
                recommended: false,
                repo: "example/model".into(),
                active: false,
                state: ModelInstallState::NotInstalled,
            },
        ]
    }

    /// `download` calls so far.
    pub fn downloads(&self) -> usize {
        self.downloads.load(Ordering::SeqCst)
    }

    /// `remove` calls so far.
    pub fn removals(&self) -> usize {
        self.removals.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl crate::models::ModelManager for FakeModels {
    fn scan(&self) -> Vec<crate::models::ModelState> {
        self.entries.lock().clone()
    }

    async fn download(
        &self,
        id: &str,
        progress: crate::models::ProgressSink,
        cancel: crate::models::CancelToken,
    ) -> Result<crate::models::ModelInstallState, String> {
        use crate::models::ModelInstallState;
        self.downloads.fetch_add(1, Ordering::SeqCst);
        let Some(entry) = self.entries.lock().iter().find(|m| m.id == id).cloned() else { return Err(format!("unknown model {id}")) };
        let total = entry.size_bytes;
        let Some(steps) = self.steps else {
            progress(ModelInstallState::Downloading { received: 0, total, file: "model.int8.onnx".into() });
            cancel.cancelled().await;
            return Err("cancelled".into());
        };
        for step in 1..=steps {
            if cancel.is_cancelled() {
                return Err("cancelled".into());
            }
            progress(ModelInstallState::Downloading { received: total * u64::from(step) / u64::from(steps.max(1)), total, file: "model.int8.onnx".into() });
            tokio::task::yield_now().await;
        }
        progress(ModelInstallState::Verifying);
        if let Some(message) = &self.fail_with {
            return Err(message.clone());
        }
        let installed = ModelInstallState::Installed { path: format!("/fake/models/{id}"), installed_at: 1_758_700_000 };
        for m in self.entries.lock().iter_mut() {
            if m.id == id {
                m.state = installed.clone();
            }
        }
        Ok(installed)
    }

    fn remove(&self, id: &str) -> Result<(), String> {
        self.removals.fetch_add(1, Ordering::SeqCst);
        let mut entries = self.entries.lock();
        let Some(m) = entries.iter_mut().find(|m| m.id == id) else { return Err(format!("unknown model {id}")) };
        m.state = crate::models::ModelInstallState::NotInstalled;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fakes_behave_as_documented() {
        let audio = FakeAudio::speech();
        let frames = Arc::new(Mutex::new(Vec::new()));
        let sink = frames.clone();
        let ready = Arc::new(AtomicBool::new(false));
        let ready_flag = ready.clone();
        let mut capture = audio
            .start(None, Box::new(move |f| sink.lock().push(f)), Box::new(move || ready_flag.store(true, Ordering::SeqCst)), CaptureOptions::default())
            .unwrap();
        assert_eq!(frames.lock().len(), 3);
        assert!(ready.load(Ordering::SeqCst), "the fake device is ready at once");
        assert!(capture.live_pcm().is_none(), "no tap unless asked");
        let rec = capture.stop().unwrap();
        assert_eq!(rec.duration_ms, 1500);
        assert!(!wav::is_silent(&rec.wav));
        assert_eq!((audio.starts(), audio.stops(), audio.live_requests()), (1, 1, 0));
        assert_eq!(audio.max_durations(), vec![super::super::ports::MAX_RECORDING]);
        // A live tap replays the recording at 16 kHz in 100 ms reads and closes with the capture.
        let mut capture = audio.start(None, Box::new(|_| {}), Box::new(|| {}), CaptureOptions::LIVE).unwrap();
        let mut tap = capture.live_pcm().expect("live tap");
        assert!(capture.live_pcm().is_none(), "take-once");
        assert_eq!(audio.live_requests(), 1);
        let mut buf = vec![0.0; 4000];
        let mut total = 0;
        loop {
            let n = tap.read(&mut buf);
            if n == 0 {
                break;
            }
            assert!(n <= LIVE_CHUNK_SAMPLES);
            total += n;
        }
        assert_eq!(total, 24_000, "1.5 s at 16 kHz");
        assert!(!tap.is_closed() && !tap.overrun());
        capture.stop().unwrap();
        assert!(tap.is_closed());
        let closed = Arc::new(AtomicBool::new(false));
        let mut degraded = FakeLivePcm::new(vec![0.5; 10], closed.clone()).with_overrun();
        assert!(degraded.overrun() && !degraded.is_closed());
        assert_eq!(degraded.read(&mut buf[..4]), 4);
        drop(closed);
        let never = FakeAudio::speech().never_ready();
        let flag = Arc::new(AtomicBool::new(false));
        let f = flag.clone();
        let capture = never.start(None, Box::new(|_| {}), Box::new(move || f.store(true, Ordering::SeqCst)), CaptureOptions::default()).unwrap();
        drop(capture);
        assert!(!flag.load(Ordering::SeqCst));
        assert!(wav::is_silent(&FakeAudio::silence().start(None, Box::new(|_| {}), Box::new(|| {}), CaptureOptions::default()).unwrap().stop().unwrap().wav));
        assert_eq!(FakeAudio::short().start(None, Box::new(|_| {}), Box::new(|| {}), CaptureOptions::default()).unwrap().stop().unwrap().duration_ms, 100);
        let failing = FakeAudio::failing_start("no mic");
        assert_eq!(failing.start(None, Box::new(|_| {}), Box::new(|| {}), CaptureOptions::LIVE).err(), Some(DictationError::Audio("no mic".into())));
        assert_eq!(failing.starts(), 0);
        let failing = FakeAudio::failing_stop("xrun");
        let mut capture = failing.start(None, Box::new(|_| {}), Box::new(|| {}), CaptureOptions::LIVE).unwrap();
        assert!(capture.live_pcm().is_none(), "a capture whose stop will fail has no tap");
        assert_eq!(capture.stop().err(), Some(DictationError::Audio("xrun".into())));
        assert_eq!(failing.stops(), 1);

        let t = FakeTranscriber::ok("hi");
        assert_eq!(t.transcribe(&rec.wav, Some("zh"), &["Voltip".to_owned()]).await.unwrap().text, "hi");
        assert_eq!(t.transcribe(b"junk", None, &[]).await.unwrap_err(), DictationError::Asr("not a wav file".into()));
        assert_eq!(t.calls(), 2);
        assert_eq!(t.languages(), vec![Some("zh".to_owned()), None]);
        assert_eq!(t.glossaries(), vec![vec!["Voltip".to_owned()], Vec::new()], "the fake records the glossary");
        assert_eq!(t.durations_ms(), vec![1500, 0], "the fake measures what it was given");
        assert!(matches!(FakeTranscriber::err("500").transcribe(&rec.wav, None, &[]).await, Err(DictationError::Asr(m)) if m == "500"));
        let slow = FakeTranscriber::slow("late", Duration::from_millis(5));
        assert_eq!(slow.transcribe(&rec.wav, None, &[]).await.unwrap().latency_ms, FAKE_LATENCY_MS);

        let r = FakeRefiner::ok("你好，世界。");
        let hints = RefineHints { glossary: vec!["世界".to_owned()], language: Some("zh".into()), ..RefineHints::default() };
        assert_eq!(r.refine("你好 世界", &hints).await.unwrap().model, FAKE_REFINE_MODEL);
        assert_eq!(r.calls(), 1);
        assert_eq!(r.inputs(), vec![("你好 世界".to_owned(), vec!["世界".to_owned()])]);
        assert_eq!(r.hints(), vec![hints]);
        assert!(matches!(FakeRefiner::err("429").refine("x", &RefineHints::default()).await, Err(DictationError::Refine(m)) if m == "429"));
        assert_eq!(FakeRefiner::slow("s", Duration::from_millis(5)).refine("x", &RefineHints::default()).await.unwrap().text, "s");
        // The probe answers as configured, can change between takes, and hangs until released.
        let probe = FakeProbe::app("slack", "Slack", Some("#dev"));
        assert_eq!(probe.foreground().unwrap().map(|a| a.app_id), Some("slack".into()));
        probe.set_app("code", "Code", None);
        assert_eq!(probe.foreground().unwrap().map(|a| (a.name, a.title)), Some(("Code".into(), None)));
        probe.set_nothing();
        assert_eq!(probe.foreground(), Ok(None));
        assert_eq!(probe.calls(), 3);
        assert_eq!(FakeProbe::nothing().foreground(), Ok(None));
        assert_eq!(FakeProbe::failing("no display").foreground(), Err("no display".into()));
        assert!(std::panic::catch_unwind(|| FakeProbe::panicking().foreground()).is_err());
        let hanging = Arc::new(FakeProbe::hanging());
        let h = hanging.clone();
        let worker = std::thread::spawn(move || h.foreground());
        hanging.release();
        assert_eq!(worker.join().unwrap(), Ok(None));
        // docs/dictation.md §19: an edit records the selection, the instruction and the hints.
        let e = FakeRefiner::ok("改写后");
        let edit_hints = RefineHints { glossary: vec!["Voltip".to_owned()], ..RefineHints::default() };
        assert_eq!(e.edit("原文", "改得更正式", &edit_hints).await.unwrap().text, "改写后");
        assert_eq!(e.edits(), vec![("原文".to_owned(), "改得更正式".to_owned(), vec!["Voltip".to_owned()])]);
        assert_eq!(e.edit_hints(), vec![edit_hints]);
        assert_eq!(e.calls(), 0, "an edit is not a refine");

        let i = FakeInjector::paste();
        assert_eq!(i.inject("a").unwrap(), Injection { via: Via::Paste, note: None });
        assert_eq!(FakeInjector::clipboard(Some("no focus")).inject("b").unwrap().note, Some(InjectNote::other("no focus")));
        let coded = InjectNote::new(super::super::ports::ClipboardCode::NoPermission, "enigo: no permission");
        assert_eq!(FakeInjector::clipboard_with(coded.clone()).inject("c").unwrap().note, Some(coded));
        assert_eq!(FakeInjector::clipboard(None).inject("b").unwrap().via, Via::Clipboard);
        let failing = FakeInjector::err("denied");
        assert_eq!(failing.inject("c").unwrap_err(), DictationError::Inject("denied".into()));
        assert_eq!(failing.injected(), vec!["c".to_owned()]);
        assert_eq!(i.injected(), vec!["a".to_owned()]);
        // Per-call replies run out in order, then the configured one applies.
        let once = FakeInjector::paste().clipboard_once(Some("no focus")).err_once("denied");
        assert_eq!(once.inject("1").unwrap().via, Via::Clipboard);
        assert_eq!(once.inject("2").unwrap_err(), DictationError::Inject("denied".into()));
        assert_eq!(once.inject("3").unwrap().via, Via::Paste);
        // A gated injector blocks until released; `waiting` counts the blocked calls.
        let gated = Arc::new(FakeInjector::paste().gated());
        assert_eq!(gated.waiting(), 0);
        let worker = {
            let g = gated.clone();
            std::thread::spawn(move || g.inject("gated").map(|i| i.via))
        };
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while gated.waiting() == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(gated.waiting(), 1, "the call is blocked on the gate");
        assert!(gated.injected().is_empty(), "nothing recorded until the gate opens");
        gated.release(1);
        assert_eq!(worker.join().unwrap().unwrap(), Via::Paste);
        assert_eq!(gated.injected(), vec!["gated".to_owned()]);
        FakeInjector::paste().release(1);
        // docs/dictation.md §19: the selection the fake plays, its errors, timing and copy gate.
        let plain = FakeInjector::paste();
        assert_eq!(plain.copy_selection(&[]).unwrap(), None, "nothing selected by default");
        assert_eq!(plain.selection_timing(), SelectionTiming::AtPress);
        let selected = FakeInjector::paste().with_selection("选中").after_key_up();
        assert_eq!(selected.copy_selection(&[Modifier::Alt]).unwrap().as_deref(), Some("选中"));
        assert_eq!(selected.copies(), vec![vec![Modifier::Alt]]);
        assert_eq!(selected.selection_timing(), SelectionTiming::AfterKeyUp);
        let broken = FakeInjector::paste().with_selection_error(DictationError::Selection("no tool".into()));
        assert_eq!(broken.copy_selection(&[]).unwrap_err(), DictationError::Selection("no tool".into()));
        let copy_gated = Arc::new(FakeInjector::paste().with_selection("x").copy_gated());
        let worker = {
            let g = copy_gated.clone();
            std::thread::spawn(move || g.copy_selection(&[]))
        };
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while copy_gated.copies_waiting() == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(copy_gated.copies_waiting(), 1);
        assert!(copy_gated.copies().is_empty(), "not recorded until the gate opens");
        copy_gated.release_copies(1);
        assert_eq!(worker.join().unwrap().unwrap().as_deref(), Some("x"));
        assert_eq!(FakeInjector::paste().copies_waiting(), 0);
        FakeInjector::paste().release_copies(1);

        let p = ports();
        let (t, r) = (p.factory)(&crate::engines::ResolvedEngines::resolve(&Default::default(), &Default::default(), &crate::engines::BuiltIn::EMPTY));
        assert!(r.is_none());
        assert!(p.streaming.is_none() && p.models.is_none() && p.probe.is_none());
        assert_eq!(t.transcribe(&rec.wav, None, &[]).await.unwrap().text, FAKE_TRANSCRIPT);
        assert!(format!("{p:?}").contains("DictationPorts"));
        assert_eq!(speech_recording(0).wav.len(), 44);
        assert_eq!(silent_recording(10).wav.len(), 44 + 320);
        let live = ports_live(Arc::new(FakeStreaming::script()));
        assert!(live.streaming.is_some());
        let library = live.models.as_ref().unwrap().scan();
        assert_eq!(library.len(), 3);
        let streaming = library.iter().find(|m| m.id == FAKE_STREAMING_MODEL_ID).unwrap();
        assert!(streaming.is_streaming() && streaming.state.is_installed());
        assert!(library.iter().filter(|m| !m.is_streaming()).all(|m| m.capabilities == ["offline"] && !m.state.is_installed()));
    }

    /// The scripted streaming session: one word per 100 ms chunk, an endpoint after the fourth,
    /// the tail after flushing; open failures and mid-stream errors on request.
    #[test]
    fn fake_streaming_follows_its_script() {
        let fake = FakeStreaming::script();
        fake.warm();
        assert_eq!(fake.warms(), 1);
        let mut s = fake.open(Some("zh")).unwrap();
        assert_eq!(fake.opens(), 1);
        assert_eq!(s.poll(), StreamEvent::Idle, "nothing before the first feed");
        let chunk = vec![0.1_f32; LIVE_CHUNK_SAMPLES];
        let mut events = Vec::new();
        for _ in 0..5 {
            s.feed(&chunk);
            loop {
                match s.poll() {
                    StreamEvent::Idle => break,
                    e => events.push(e),
                }
            }
        }
        assert_eq!(
            events,
            vec![
                StreamEvent::Partial { current: "你好".into() },
                StreamEvent::Partial { current: "你好，".into() },
                StreamEvent::Partial { current: "你好，世界".into() },
                StreamEvent::Endpoint { text: "你好，世界。".into(), start_ms: 0, end_ms: 400 },
                StreamEvent::Partial { current: "今天".into() },
            ]
        );
        let fin = s.finish().unwrap();
        assert_eq!(fin.committed, vec![Segment { text: "你好，世界。".into(), start_ms: 0, end_ms: 400 }]);
        assert_eq!(fin.tail, "今天");
        assert_eq!(fake.finishes(), 1);
        // Past the script the sentence stops growing; a session flushed right after an endpoint has no tail.
        let mut s = fake.open(None).unwrap();
        for _ in 0..10 {
            s.feed(&chunk);
        }
        while s.poll() != StreamEvent::Idle {}
        assert_eq!(s.finish().unwrap().tail, "今天天气");
        let mut s = FakeStreaming::words(vec!["a".into(), "b".into()], 2).open(None).unwrap();
        s.feed(&chunk);
        s.feed(&chunk);
        assert_eq!(s.poll(), StreamEvent::Partial { current: "a".into() });
        assert!(matches!(s.poll(), StreamEvent::Endpoint { text, .. } if text == "b"));
        assert_eq!(s.finish().unwrap().tail, "");
        assert_eq!(FakeStreaming::failing_open("模型未下载").open(None).err(), Some(DictationError::Asr("模型未下载".into())));
        let erroring = FakeStreaming::erroring_after(1);
        let mut s = erroring.open(None).unwrap();
        s.feed(&chunk);
        assert_eq!(s.poll(), StreamEvent::Partial { current: "你好".into() });
        s.feed(&chunk);
        assert_eq!(s.poll(), StreamEvent::Error("fake decoder failed".into()));
    }

    #[test]
    fn a_held_session_waits_before_its_word_until_released() {
        let fake = FakeStreaming::words(vec!["a".into(), "b".into()], 5).holding(1);
        let chunk = vec![0.1_f32; LIVE_CHUNK_SAMPLES];
        let mut s = fake.open(None).unwrap();
        s.feed(&chunk);
        assert_eq!(s.poll(), StreamEvent::Partial { current: "a".into() });
        let gate = fake.hold.as_ref().map(|(_, gate)| gate.clone()).unwrap();
        let feeding = std::thread::spawn(move || {
            s.feed(&chunk);
            s
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while gate.waiting.load(Ordering::SeqCst) == 0 {
            assert!(std::time::Instant::now() < deadline, "the second feed waits before the held word");
            std::thread::sleep(Duration::from_millis(1));
        }
        fake.release_hold();
        let mut s = feeding.join().unwrap();
        assert_eq!(s.poll(), StreamEvent::Partial { current: "b".into() });
    }

    /// Regression (CI, 2026-09-30): a test failed while its gated injection was held and never
    /// released it; dropping the runtime waits for that blocking thread, so the run hung until
    /// the job's hour ran out. A held call now gives up with a panic after its limit.
    #[test]
    fn regression_a_held_call_nobody_releases_gives_up_instead_of_hanging() {
        let gate = Arc::new(Gate { limit: Duration::from_millis(50), ..Gate::default() });
        let (tx, rx) = std::sync::mpsc::channel();
        let held = gate.clone();
        std::thread::spawn(move || {
            let gave_up = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| held.pass())).is_err();
            let _ = tx.send(gave_up);
        });
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)), Ok(true), "the held call gave up");
        // A released call still passes.
        gate.release(1);
        gate.pass();
    }
}
