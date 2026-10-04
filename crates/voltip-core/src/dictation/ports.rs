//! The native capabilities the dictation pipeline needs, as traits (docs/dictation.md §1): the
//! microphone, speech-to-text, the optional clean-up, text delivery, — since §11 — the streaming
//! recogniser that previews text while the microphone is still open, and — since §18 — the probe
//! that names the application in front when a take starts.
//!
//! The core owns the state machine and depends on nothing but these traits; the desktop shell
//! plugs in cpal / rtrb / reqwest / sherpa-onnx / enigo implementations, tests plug in
//! [`crate::dictation::fakes`].

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::wav;
use crate::engines::OutputMode;
use crate::hotkey::Modifier;
use crate::presets::TakePreset;
use crate::scenes::{MAX_CONTEXT_NAME_CHARS, MAX_CONTEXT_TITLE_CHARS, clean_context_line, normalize_app_id};
use crate::settings::{RecordingSettings, RecordingSource};

/// One input-level reading (≈ 30 Hz while a capture runs). Same wire shape as the meter frame the
/// audio crate emits, so the webview's level meter can be fed from either source.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LevelFrame {
    /// RMS level in dBFS (`-90` is the silence floor).
    pub rms_dbfs: f32,
    /// Peak-hold level in dBFS.
    pub peak_dbfs: f32,
    /// At least one sample clipped in this frame.
    pub clipping: bool,
    /// Sample rate of the open stream.
    pub sample_rate_hz: u32,
    /// Channel count of the open stream.
    pub channels: u16,
    /// Frame counter, `0`-based per capture.
    pub seq: u64,
}

/// Why a pipeline step failed. The message variants carry the adapter's own explanation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DictationError {
    /// Capture device could not be opened or stopped.
    #[error("audio: {0}")]
    Audio(String),
    /// The recording was too short or silent; nothing was uploaded.
    #[error("没有听到声音")]
    NoSpeech,
    /// Speech recognition failed.
    #[error("asr: {0}")]
    Asr(String),
    /// Text refinement failed (never fatal for the pipeline: the raw text is injected).
    #[error("refine: {0}")]
    Refine(String),
    /// The service said the model's quota is used up (docs/dictation.md §3.5): a fallback model
    /// list moves on to its next model; with none left the take fails with `FailureCode::Quota`
    /// (a refinement still injects the raw text).
    #[error("额度已用完：{detail}")]
    QuotaExhausted {
        /// The service that ran out.
        service: crate::providers::ServiceKind,
        /// The adapter's explanation: the service's code and message.
        detail: String,
    },
    /// The text could not be handed to the foreground application.
    #[error("inject: {0}")]
    Inject(String),
    /// Voice edit (docs/dictation.md §19): the foreground application has no selection (the copy
    /// put no text on the clipboard).
    #[error("没有选中文本")]
    NoSelection,
    /// Voice edit: the selection has this many characters, more than [`MAX_EDIT_SELECTION_CHARS`].
    #[error("选中文本过长：{0} 字（上限 {MAX_EDIT_SELECTION_CHARS} 字）")]
    SelectionTooLong(usize),
    /// Voice edit: the selection could not be read (no copy tool, clipboard unusable).
    #[error("selection: {0}")]
    Selection(String),
    /// Voice edit is not possible here: no LLM configured, or a shell that cannot read a selection.
    #[error("edit: {0}")]
    EditUnavailable(String),
    /// Voice edit (docs/dictation.md §19.2): the foreground application is a terminal, whose
    /// selection (program output) cannot be replaced; refused before any key is pressed.
    #[error("终端里不支持语音编辑：终端里的选区不能被替换")]
    EditInTerminal,
    /// A start arrived while a recording or a pipeline run is in progress.
    #[error("已有听写正在进行")]
    Busy,
    /// A stop / cancel arrived with nothing running.
    #[error("当前没有进行中的听写")]
    Idle,
}

impl DictationError {
    /// The model's quota is used up ([`DictationError::QuotaExhausted`]): the one failure a
    /// fallback model list moves on for (docs/dictation.md §3.5).
    pub fn is_quota_exhausted(&self) -> bool {
        matches!(self, Self::QuotaExhausted { .. })
    }
}

/// Captured audio, ready for upload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recording {
    /// Complete WAV file (PCM 16-bit, mono).
    pub wav: Vec<u8>,
    /// Length of the recording.
    pub duration_ms: u64,
    /// Sample rate of the PCM data.
    pub sample_rate_hz: u32,
}

impl Recording {
    /// The part of the take from `ms` on, as its own recording (docs/dictation.md §12: when
    /// `live_inject` degrades after some sentences were pasted, only the audio after the last
    /// committed `Segment.end_ms` goes to the whole-take transcriber). `ms` past the end gives an
    /// empty recording; a WAV that does not parse is returned whole.
    pub fn slice_from_ms(&self, ms: u64) -> Recording {
        let Some(pcm) = wav::pcm_data(&self.wav) else { return self.clone() };
        let (frames, _) = pcm.as_chunks::<2>();
        let skip = usize::try_from(ms.saturating_mul(u64::from(self.sample_rate_hz)) / 1000).unwrap_or(usize::MAX).min(frames.len());
        let samples: Vec<i16> = frames[skip..].iter().map(|b| i16::from_le_bytes(*b)).collect();
        let duration_ms = if self.sample_rate_hz == 0 { 0 } else { samples.len() as u64 * 1000 / u64::from(self.sample_rate_hz) };
        Recording { wav: wav::encode_pcm16(&samples, self.sample_rate_hz), duration_ms, sample_rate_hz: self.sample_rate_hz }
    }
}

/// What a capture is asked for besides the microphone device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureOptions {
    /// Also feed a 16 kHz mono copy of the audio to [`Capture::live_pcm`] for the streaming
    /// recogniser (docs/dictation.md §11).
    pub live: bool,
    /// Longest take the capture records; the core stops the capture itself when it is reached,
    /// the recorder only has to reserve for it and never to truncate before (except a `long` one,
    /// which keeps at most [`MAX_RECORDING`] in memory).
    pub max_duration: Duration,
    /// The take may run past [`MAX_RECORDING`] (docs/dictation.md §22): the capture keeps at most
    /// [`MAX_RECORDING`] of audio in memory, and the whole take goes to the core's recording file
    /// through [`Capture::pcm_stream`].
    pub long: bool,
    /// Where the audio comes from: the microphone, the computer's sound, or both.
    pub source: RecordingSource,
    /// The output device the computer's sound comes from (`None` = the system default output).
    pub output_device: Option<String>,
    /// `mixed`: cancel the microphone's echo of the computer's sound (docs/dictation.md §22.6).
    pub echo_cancel: bool,
}

impl Default for CaptureOptions {
    /// The microphone, no live tap, the whole-take cap ([`MAX_RECORDING`]).
    fn default() -> Self {
        Self { live: false, max_duration: MAX_RECORDING, long: false, source: RecordingSource::Microphone, output_device: None, echo_cancel: false }
    }
}

impl CaptureOptions {
    /// The microphone with a live tap and the whole-take cap.
    pub const LIVE: Self =
        Self { live: true, max_duration: MAX_RECORDING, long: false, source: RecordingSource::Microphone, output_device: None, echo_cancel: false };

    /// A dictation take on this computer (docs/dictation.md §22): the settings' source, output
    /// device, length and echo cancellation (which only a `mixed` take has); `long` when the take
    /// may run past [`MAX_RECORDING`].
    pub fn dictation(recording: &RecordingSettings, live: bool) -> Self {
        let max_duration = recording.max_duration();
        Self {
            live,
            max_duration,
            long: max_duration > MAX_RECORDING,
            source: recording.source,
            output_device: recording.output_device.clone(),
            echo_cancel: recording.echo_cancel && recording.source == RecordingSource::Mixed,
        }
    }
}

/// Microphone capture.
pub trait AudioSource: Send + Sync {
    /// Start capturing from `device_id` (default input when `None`). `on_level` is called from the
    /// audio thread at ≈ 30 Hz; `on_ready` once, from the audio thread, when the first samples
    /// arrive (Bluetooth / USB microphones deliver them 100–500 ms after the stream opens — the
    /// core takes `Listening.started_at` from it). With `options.live`, the capture also feeds a
    /// 16 kHz mono copy of the audio to [`Capture::live_pcm`] for the streaming preview; the take
    /// is kept up to `options.max_duration`. The returned handle stops the capture and hands back
    /// the audio.
    fn start(
        &self,
        device_id: Option<&str>,
        on_level: Box<dyn Fn(LevelFrame) + Send>,
        on_ready: Box<dyn FnOnce() + Send>,
        options: CaptureOptions,
    ) -> Result<Box<dyn Capture>, DictationError>;
}

/// A running capture; consumed by [`Capture::stop`]. Dropping it without stopping discards the audio.
pub trait Capture: Send {
    /// Stop and return the recording.
    fn stop(self: Box<Self>) -> Result<Recording, DictationError>;

    /// The live 16 kHz mono tap requested with `live` (take-once: `None` on the second call, when
    /// `live` was `false`, or when the shell has no tap). Closes when the capture stops.
    fn live_pcm(&mut self) -> Option<Box<dyn LivePcm>> {
        None
    }

    /// The whole take at 16 kHz requested with `long` (docs/dictation.md §22; take-once, `None`
    /// on the second call, when `long` was `false`, or when the shell has none — its capture then
    /// keeps the whole take in memory). Closes when the capture stops.
    fn pcm_stream(&mut self) -> Option<Box<dyn PcmStream>> {
        None
    }
}

/// Sample rate of [`PcmStream`]: what the recognisers expect.
pub const PCM_SAMPLE_RATE_HZ: u32 = 16_000;

/// Cuts a long take into segments as it arrives (docs/dictation.md §22).
pub trait Segmenter: Send {
    /// Feed the next samples (mono, [`PCM_SAMPLE_RATE_HZ`]); returns where each segment these
    /// samples complete ends, as a sample position from the start of the take. A segment starts
    /// where the previous one ended, the first at 0.
    fn push(&mut self, samples: &[f32]) -> Vec<u64>;
    /// The take ended: where its last segment ends; `None` when nothing is left after the last cut.
    fn finish(&mut self) -> Option<u64>;
}

/// Makes a [`Segmenter`] for each long take (the desktop's cuts where the speech pauses,
/// docs/dictation.md §22).
pub trait SegmenterFactory: Send + Sync {
    /// A segmenter for one take, or why there is none now (its model is missing): the core cuts
    /// with its own fallback then.
    fn create(&self) -> Result<Box<dyn Segmenter>, String>;
}

/// The consumer end of a long take's stream (docs/dictation.md §22): the whole take as mono `f32`
/// at [`PCM_SAMPLE_RATE_HZ`], produced on the audio thread and read by the core's recording
/// thread. Never blocks.
pub trait PcmStream: Send {
    /// Copy up to `out.len()` samples, in order and never past a gap; returns how many.
    fn read(&mut self, out: &mut [f32]) -> usize;
    /// How many samples are missing at the current position (the shell's buffer overflowed
    /// because the reader fell behind), once; `None` while the next sample follows the last one.
    fn gap(&mut self) -> Option<u64>;
    /// The capture stopped: once `read` returns `0` and `gap` `None`, nothing more will come.
    fn is_closed(&self) -> bool;
}

/// The consumer end of the recorder's live tap (docs/dictation.md §11): mono `f32` at 16 kHz,
/// produced on the audio thread, read by the decode thread. Never blocks.
pub trait LivePcm: Send {
    /// Copy up to `out.len()` samples that arrived since the last read; returns how many.
    fn read(&mut self, out: &mut [f32]) -> usize;
    /// The producer had to drop samples because the ring was full (the decode thread fell behind);
    /// sticky. The preview is degraded from then on: its text may miss words.
    fn overrun(&self) -> bool;
    /// The capture stopped: nothing more will arrive once `read` returns `0`.
    fn is_closed(&self) -> bool;
}

/// Speech recognition result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transcript {
    /// Recognised text.
    pub text: String,
    /// Round-trip time of the request.
    pub latency_ms: u64,
    /// The model that recognised it, as the client that ran reports it (docs/dictation.md §3.5):
    /// what the history records, also when a fallback model or a configuration changed mid-take
    /// did the work. `None` when the client cannot say (the fakes).
    pub model: Option<String>,
}

/// Speech-to-text.
#[async_trait]
pub trait Transcriber: Send + Sync {
    /// Transcribe a WAV file; `language` is a hint (`zh`, `en`, …) or `None` for auto-detect.
    /// `glossary` is the user's enabled dictionary terms (docs/dictation.md §16.3), a recognition
    /// hint the engine may use — the cloud endpoint sends it as the OpenAI `prompt` field — or
    /// ignore (the local engines have no prompt input); empty means no hint.
    async fn transcribe(&self, wav: &[u8], language: Option<&str>, glossary: &[String]) -> Result<Transcript, DictationError>;

    /// Get ready for the next [`Transcriber::transcribe`] with `language` (docs/dictation.md §10.7):
    /// an engine with something big to load (a local model) starts loading it in the background, so
    /// the first take after start-up or a configuration change does not wait for it. Returns at
    /// once and is idempotent; a failure only logs (the take then loads again and reports it). The
    /// default does nothing: a remote client has nothing to load.
    fn warm(&self, _language: Option<&str>) {}

    /// The same service as a [`StreamingTranscriber`], when it recognises while the audio arrives
    /// (a realtime model, docs/dictation.md §11.9), hinted with `glossary` as a take is. The engine
    /// then streams the take to it and takes its sentences as the take's text. `None` (the
    /// default): the service only takes whole recordings.
    fn streaming(&self, _glossary: &[String]) -> Option<Arc<dyn StreamingTranscriber>> {
        None
    }
}

/// One sentence the streaming recogniser committed at an endpoint (docs/dictation.md §11). Times
/// are milliseconds since the stream started (the recording start, once the device delivered audio).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    /// The sentence, as the streaming model wrote it (punctuation included).
    pub text: String,
    /// Where it started.
    pub start_ms: u64,
    /// Where the endpoint was detected.
    pub end_ms: u64,
}

/// What [`StreamingSession::poll`] found after a [`StreamingSession::feed`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamEvent {
    /// The current (uncommitted) sentence changed; `current` is its whole text so far.
    Partial {
        /// The sentence being spoken, from its start to now.
        current: String,
    },
    /// An endpoint (trailing silence) closed a sentence; `text` is final for that sentence and the
    /// next `Partial` starts from empty.
    Endpoint {
        /// The committed sentence.
        text: String,
        /// Start in stream milliseconds.
        start_ms: u64,
        /// End in stream milliseconds.
        end_ms: u64,
    },
    /// Nothing new.
    Idle,
    /// The recogniser failed; the session is unusable from here and the preview degrades.
    Error(String),
}

/// What a finished streaming session hands back.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct StreamFinal {
    /// Every sentence committed at an endpoint, in order.
    pub committed: Vec<Segment>,
    /// The last, uncommitted sentence after flushing the recogniser (may be empty).
    pub tail: String,
    /// The model this text came from, set after the flush (docs/dictation.md §3.5): the flush may
    /// itself send the last requests. `None` when the session cannot say (the local streaming
    /// model, the fakes).
    pub model: Option<String>,
}

/// A streaming recogniser (docs/dictation.md §11): the source of the pill's live text while the
/// microphone is open. Only a preview — the final text always comes from [`Transcriber`] on the
/// whole take, so when this is unavailable or fails the pipeline behaves as if it did not exist.
pub trait StreamingTranscriber: Send + Sync {
    /// Open a session for one recording. Synchronous and possibly slow (the model loads on first
    /// use; measured 2.6 s for the Zipformer): the core calls it on the decode thread.
    fn open(&self, language: Option<&str>) -> Result<Box<dyn StreamingSession>, DictationError>;

    /// Load the model in the background so the first [`StreamingTranscriber::open`] is quick.
    /// Called when live preview becomes ready and on every hotkey press; must return at once and
    /// only log when the load fails.
    fn warm(&self);
}

/// One recording's streaming session, driven from the decode thread: [`StreamingSession::feed`]
/// then [`StreamingSession::poll`] until `Idle`, [`StreamingSession::finish`] when the capture ends.
pub trait StreamingSession: Send {
    /// Hand over the next 16 kHz mono samples and decode what is ready.
    fn feed(&mut self, pcm16k: &[f32]);
    /// The next event since the last poll.
    fn poll(&mut self) -> StreamEvent;
    /// Flush: signal end of input, decode the rest, return every committed sentence plus the tail.
    fn finish(self: Box<Self>) -> Result<StreamFinal, DictationError>;

    /// The model the text so far came from (docs/dictation.md §3.5): read when the session
    /// degrades, after sentences of it may have been used. `None` (the default) when the session
    /// cannot say.
    fn model(&self) -> Option<String> {
        None
    }
}

/// Refinement result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refined {
    /// Text with punctuation / typos / fillers fixed, meaning unchanged.
    pub text: String,
    /// Round-trip time of the request.
    pub latency_ms: u64,
    /// Model that produced it.
    pub model: String,
}

/// The take's context for the LLM (docs/dictation.md §18.5), already filtered by the privacy
/// switches: what is `None` is not sent. `Debug` shows only which parts are present.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct RefineContext {
    /// Display name of the application being dictated into (`context_sharing.app_name`).
    pub app_name: Option<String>,
    /// Its window title (`context_sharing.window_title`, off by default).
    pub window_title: Option<String>,
    /// The matched scene's extra instruction for the LLM.
    pub instruction: Option<String>,
}

impl std::fmt::Debug for RefineContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RefineContext")
            .field("app_name", &self.app_name.is_some())
            .field("window_title", &self.window_title.is_some())
            .field("instruction", &self.instruction.is_some())
            .finish()
    }
}

impl RefineContext {
    /// Nothing to say about the take.
    pub fn is_empty(&self) -> bool {
        self.app_name.is_none() && self.window_title.is_none() && self.instruction.is_none()
    }
}

/// Everything the refiner is told besides the text (docs/dictation.md §16.3, §18.5, §21): one
/// struct, so the glossary, the take's language and preset and its context travel the same way.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RefineHints {
    /// The user's enabled dictionary terms (spelling authority); empty = no glossary block.
    pub glossary: Vec<String>,
    /// The take's language hint (a scene's override or the engines' language); `None` = unknown.
    pub language: Option<String>,
    /// What the clean-up does: the scene's preset, else the engines' (校对 when a custom preset
    /// is gone).
    pub preset: TakePreset,
    /// Where the text is going and what the scene asks for.
    pub context: RefineContext,
}

/// LLM clean-up of the raw transcript, and the rewrite of a voice edit.
#[async_trait]
pub trait Refiner: Send + Sync {
    /// Refine `text` (already corrected by the dictionary) with `hints`: every glossary term kept
    /// exactly as spelled (docs/dictation.md §16.3), the take's language and preset, and its context
    /// (§18.5). Empty hints leave the prompt as it was before §16.
    async fn refine(&self, text: &str, hints: &RefineHints) -> Result<Refined, DictationError>;

    /// Rewrite `selection` according to the spoken `instruction` (docs/dictation.md §19; the
    /// instruction is already corrected by the dictionary). The same `hints` as a dictation: the
    /// glossary terms are the spelling authority and the application in front is reference
    /// context; the preset, the language hint and the scene instruction are dictation-only (the
    /// spoken instruction decides how the selection changes). The answer replaces the selection as
    /// it is: an empty or cut-off answer must be an error, never an empty `text`.
    async fn edit(&self, selection: &str, instruction: &str, hints: &RefineHints) -> Result<Refined, DictationError>;
}

/// The application that had the focus when a take started (docs/dictation.md §18.2). `Debug`
/// never prints the window title.
#[derive(Clone, PartialEq, Eq)]
pub struct ForegroundApp {
    /// Normalised id (`slack`, `code`, `com.microsoft.vscode`; see `scenes::normalize_app_id`).
    pub app_id: String,
    /// Display name (`slack`, `WINWORD`, `Code`, `Visual Studio Code`).
    pub name: String,
    /// Window title, where the platform tells (not on macOS).
    pub title: Option<String>,
    /// The focused window, where the platform tells: the HWND on Windows, the window id on X11, the
    /// front process id on macOS (which cannot tell two windows of one application apart). Only
    /// used to check that a paste from the history still goes where the user left off
    /// ([`crate::paste`]); never shown, stored, logged or printed by `Debug`.
    pub window: Option<u64>,
}

impl std::fmt::Debug for ForegroundApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForegroundApp").field("app_id", &self.app_id).field("name", &self.name).field("title", &self.title.is_some()).finish()
    }
}

impl ForegroundApp {
    /// The probe's answer as the core keeps it: the id normalised, the name and title one clean
    /// line each within the §18.1 limits (the id stands in for an empty name). `None` when no id
    /// is left.
    pub fn sanitized(self) -> Option<Self> {
        let app_id = normalize_app_id(&self.app_id);
        if app_id.is_empty() {
            return None;
        }
        let name = clean_context_line(&self.name, MAX_CONTEXT_NAME_CHARS).unwrap_or_else(|| app_id.clone());
        let title = self.title.as_deref().and_then(|t| clean_context_line(t, MAX_CONTEXT_TITLE_CHARS));
        Some(Self { app_id, name, title, window: self.window })
    }
}

/// Names the application in front (docs/dictation.md §18.2). The desktop shell plugs in the
/// platform's implementation (Win32 / X11 / AppKit); shells without one (the phone) plug in none.
pub trait ForegroundProbe: Send + Sync {
    /// The focused application right now. Blocking but meant to be quick: the core calls it on a
    /// blocking thread and waits at most [`PROBE_DEADLINE`]. `Ok(None)` when there is no
    /// application the shell can name (pure Wayland, the desktop itself, Voltip's own window).
    fn foreground(&self) -> Result<Option<ForegroundApp>, String>;
}

/// Asks an OpenAI-compatible service which models it serves (`GET {base}/models`): the engines
/// pane's 测试连接 (docs/dictation.md §3.3). The desktop shell implements it over HTTP; shells
/// without one (the phone, most tests) plug in none and the probe answers `unsupported`. Failures
/// carry no host: the built-in endpoint must not reach the UI through an error.
#[async_trait]
pub trait ServiceProbe: Send + Sync {
    /// The model ids `base_url` lists, with `key` as the bearer token.
    async fn list_models(&self, base_url: &str, key: Option<&str>) -> Result<Vec<String>, crate::providers::ProbeError>;
}

/// How the text reached the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Via {
    /// Pasted into the foreground application.
    Paste,
    /// Left in the clipboard (by choice, or because the paste did not go through).
    Clipboard,
}

/// Why a requested paste left the text on the clipboard (docs/dictation.md §4.2): the kind the
/// interface explains in a sentence (`Outcome::Clipboard.code`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardCode {
    /// The system does not let Voltip send keystrokes (macOS: Accessibility not granted).
    NoPermission,
    /// No paste tool for this session (Wayland without wtype, dotool or ydotool).
    NoTool,
    /// No connection to the display server.
    NoDisplay,
    /// A secure input field or the secure desktop has the keyboard.
    SecureInput,
    /// The window in front runs as administrator (Windows).
    ElevatedTarget,
    /// The text is longer than a paste should be (a long take over 5 000 characters,
    /// docs/dictation.md §22): it waits on the clipboard.
    TooLong,
    /// Anything else.
    Other,
}

/// A clipboard fallback as the injector reports it: its kind and the original message (the
/// history keeps both; the interface shows the message only under the technical details).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InjectNote {
    /// What the interface says.
    pub code: ClipboardCode,
    /// The message as the tool or the system gave it.
    pub detail: String,
}

impl InjectNote {
    /// A note of `code` with `detail`.
    pub fn new(code: ClipboardCode, detail: impl Into<String>) -> Self {
        Self { code, detail: detail.into() }
    }

    /// A note with no kind the interface can name.
    pub fn other(detail: impl Into<String>) -> Self {
        Self::new(ClipboardCode::Other, detail)
    }
}

/// Injection outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Injection {
    /// Route taken.
    pub via: Via,
    /// Why the text stayed in the clipboard when a paste was requested.
    pub note: Option<InjectNote>,
}

/// When a voice edit's copy chord can reach the foreground application (docs/dictation.md §19).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SelectionTiming {
    /// At the hotkey press: the hotkey does not grab the keyboard (Windows `RegisterHotKey`, macOS
    /// Carbon, a compositor shortcut running `--edit-toggle`).
    #[default]
    AtPress,
    /// Only once the hotkey's key is up: an X11 `XGrabKey` routes every key event — the copy chord
    /// included — to the grabbing client while the key is down (X11 and XWayland sessions).
    AfterKeyUp,
}

/// Puts text into the foreground application — and, for voice edit, reads what is selected there.
pub trait Injector: Send + Sync {
    /// Inject `text`. Blocking (clipboard + synthetic key events + a short restore delay); the
    /// core calls it from a blocking task.
    fn inject(&self, text: &str) -> Result<Injection, DictationError>;

    /// Put `text` on the clipboard and nothing else, whatever `EngineSettings.inject` says: a paste
    /// from the history whose window is gone ([`crate::paste`]). Blocking, like `inject`. The
    /// default belongs to shells without a clipboard of their own (the phone).
    fn copy(&self, text: &str) -> Result<(), DictationError> {
        let _ = text;
        Err(DictationError::Inject("this shell has no clipboard".to_owned()))
    }

    /// The foreground application's selection (docs/dictation.md §19): clipboard saved, copy
    /// chord pressed after releasing `held` (the edit hotkey's modifiers the user may still
    /// hold), clipboard restored. `Ok(None)` = nothing selected. Blocking, like `inject`. The
    /// default belongs to shells that cannot read a selection (the phone).
    fn copy_selection(&self, held: &[Modifier]) -> Result<Option<String>, DictationError> {
        let _ = held;
        Err(DictationError::EditUnavailable("this shell cannot read the selection".to_owned()))
    }

    /// When the copy can be sent ([`SelectionTiming`]); the default copies at the press.
    fn selection_timing(&self) -> SelectionTiming {
        SelectionTiming::AtPress
    }

    /// Whether `app_id` (as the foreground probe names it; docs/dictation.md §19.2) is a terminal
    /// on this host, whose selection a voice edit cannot replace. The core refuses the edit there
    /// before any key is pressed. The default — a shell without a copy chord — knows none.
    fn is_terminal_app(&self, app_id: &str) -> bool {
        let _ = app_id;
        false
    }
}

/// The part of a take kept in memory and recognised in one piece (docs/dictation.md §22): a
/// dictation take that stops before it is a short take, as before; a longer one is recognised in
/// segments from its recording file. Also the longest voice edit.
pub const MAX_RECORDING: Duration = Duration::from_secs(120);
/// Longest recording of a phone's take in the streaming output modes (docs/dictation.md §12,
/// §20): text leaves the recogniser as it is spoken, so the take can run 10 min.
pub const MAX_RECORDING_STREAMING: Duration = Duration::from_secs(600);
/// Longest recording of a phone's take (docs/dictation.md §20; a take on this computer follows
/// `Settings.recording.max_minutes`, §22): [`MAX_RECORDING`] for a whole take,
/// [`MAX_RECORDING_STREAMING`] for `streaming_final` / `live_inject`.
pub fn max_recording(mode: OutputMode) -> Duration {
    match mode {
        OutputMode::WholeTake => MAX_RECORDING,
        OutputMode::StreamingFinal | OutputMode::LiveInject => MAX_RECORDING_STREAMING,
    }
}
/// Recordings shorter than this are not uploaded.
pub const MIN_RECORDING: Duration = Duration::from_millis(300);
/// Longest selection a voice edit sends to the LLM (docs/dictation.md §19); a longer one is refused
/// before the request: the refine output is capped at 900 tokens, a longer rewrite would be cut off.
pub const MAX_EDIT_SELECTION_CHARS: usize = 2000;
/// How long a terminal state (`Done` / `Cancelled` / `Failed` without text) stays before `Idle`.
pub const DWELL: Duration = Duration::from_millis(2500);
/// How long `Failed` with recoverable text stays before `Idle` (the pill offers "copy").
pub const DWELL_WITH_TEXT: Duration = Duration::from_secs(6);
/// Minimum interval between two `Listening.live` updates from partial results (12.5 Hz).
pub const PARTIAL_THROTTLE: Duration = Duration::from_millis(80);
/// Samples per decode step of the live worker (100 ms at 16 kHz).
pub const LIVE_CHUNK_SAMPLES: usize = 1600;
/// Sample rate of the live tap.
pub const LIVE_SAMPLE_RATE_HZ: u32 = 16_000;
/// How long a take's start waits for the foreground probe (docs/dictation.md §18.4); no answer by
/// then means no scene. The device opens right after.
pub const PROBE_DEADLINE: Duration = Duration::from_millis(100);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_render_their_kind_and_via_is_snake_case() {
        assert_eq!(DictationError::NoSpeech.to_string(), "没有听到声音");
        assert_eq!(DictationError::Asr("401".into()).to_string(), "asr: 401");
        assert_eq!(DictationError::Audio("no device".into()).to_string(), "audio: no device");
        assert_eq!(DictationError::Refine("x".into()).to_string(), "refine: x");
        assert_eq!(DictationError::Inject("x".into()).to_string(), "inject: x");
        assert_eq!(DictationError::NoSelection.to_string(), "没有选中文本");
        assert_eq!(DictationError::SelectionTooLong(2400).to_string(), "选中文本过长：2400 字（上限 2000 字）");
        assert_eq!(DictationError::Selection("no copy tool".into()).to_string(), "selection: no copy tool");
        assert_eq!(DictationError::EditUnavailable("no key".into()).to_string(), "edit: no key");
        assert_eq!(SelectionTiming::default(), SelectionTiming::AtPress);
        assert!(DictationError::Busy.to_string().contains("正在进行"));
        assert!(DictationError::Idle.to_string().contains("没有进行中的听写"));
        assert_eq!(serde_json::to_string(&Via::Clipboard).unwrap(), r#""clipboard""#);
        assert_eq!(serde_json::from_str::<Via>(r#""paste""#).unwrap(), Via::Paste);
        assert!(MIN_RECORDING < DWELL && DWELL < DWELL_WITH_TEXT && DWELL_WITH_TEXT < MAX_RECORDING);
        let frame = LevelFrame { rms_dbfs: -20.0, peak_dbfs: -3.0, clipping: false, sample_rate_hz: 16_000, channels: 1, seq: 7 };
        let json = serde_json::to_string(&frame).unwrap();
        assert!(json.contains(r#""rms_dbfs":-20.0"#) && json.contains(r#""seq":7"#), "{json}");
        let segment = Segment { text: "你好。".into(), start_ms: 120, end_ms: 1480 };
        assert_eq!(serde_json::to_string(&segment).unwrap(), r#"{"text":"你好。","start_ms":120,"end_ms":1480}"#);
        assert_eq!(StreamFinal::default(), StreamFinal { committed: Vec::new(), tail: String::new(), model: None });
        assert_eq!(PARTIAL_THROTTLE, Duration::from_millis(80));
        assert_eq!(LIVE_CHUNK_SAMPLES as u32 * 10, LIVE_SAMPLE_RATE_HZ, "one chunk is 100 ms");
    }

    /// docs/dictation.md §19: an injector that knows nothing about selections (the phone's) refuses
    /// the copy with `edit_unavailable` and reports the press as its timing.
    #[test]
    fn an_injector_without_selection_support_refuses_the_copy() {
        struct PasteOnly;
        impl Injector for PasteOnly {
            fn inject(&self, _text: &str) -> Result<Injection, DictationError> {
                Ok(Injection { via: Via::Paste, note: None })
            }
        }
        assert_eq!(PasteOnly.inject("x").unwrap().via, Via::Paste);
        assert!(matches!(PasteOnly.copy_selection(&[Modifier::Alt]), Err(DictationError::EditUnavailable(m)) if m.contains("cannot read the selection")));
        assert_eq!(PasteOnly.selection_timing(), SelectionTiming::AtPress);
        assert!(!PasteOnly.is_terminal_app("windowsterminal"), "no copy chord, no terminal table");
        assert_eq!(DictationError::EditInTerminal.to_string(), "终端里不支持语音编辑：终端里的选区不能被替换");
        assert_eq!(MAX_EDIT_SELECTION_CHARS, 2000);
    }

    /// docs/dictation.md §18.2: the probe's answer is normalised and cleaned before the core keeps
    /// it; `Debug` of the app and of the refine context never shows the title or the texts.
    #[test]
    fn foreground_answers_are_sanitised_and_never_debug_print_the_title() {
        let raw = ForegroundApp { app_id: " Slack.EXE ".into(), name: " Slack\n".into(), title: Some("  #dev\u{7}chat  ".into()), window: Some(42) };
        let app = raw.sanitized().unwrap();
        assert_eq!(app, ForegroundApp { app_id: "slack".into(), name: "Slack".into(), title: Some("#dev chat".into()), window: Some(42) });
        // Neither the title nor the window id is printed.
        assert_eq!(format!("{app:?}"), r#"ForegroundApp { app_id: "slack", name: "Slack", title: true }"#);
        let nameless = ForegroundApp { app_id: "code".into(), name: "  ".into(), title: Some("\u{7}".into()), window: None }.sanitized().unwrap();
        assert_eq!((nameless.name.as_str(), nameless.title), ("code", None), "the id stands in for an empty name");
        assert_eq!(ForegroundApp { app_id: ".exe".into(), name: "x".into(), title: None, window: None }.sanitized(), None);
        let long = ForegroundApp { app_id: "a".into(), name: "名".repeat(100), title: Some("t".repeat(500)), window: None }.sanitized().unwrap();
        assert_eq!((long.name.chars().count(), long.title.map(|t| t.chars().count())), (MAX_CONTEXT_NAME_CHARS, Some(MAX_CONTEXT_TITLE_CHARS)));
        let context = RefineContext { app_name: Some("Slack".into()), window_title: Some("secret title".into()), instruction: None };
        assert_eq!(format!("{context:?}"), "RefineContext { app_name: true, window_title: true, instruction: false }");
        assert!(!format!("{:?}", RefineHints { context: context.clone(), ..RefineHints::default() }).contains("secret"));
        assert!(!context.is_empty() && RefineContext::default().is_empty());
        assert_eq!(PROBE_DEADLINE, Duration::from_millis(100));
    }

    /// A phone's take keeps the cap of its output mode (docs/dictation.md §12, §20): 120 s for a
    /// whole take, 10 min when the text streams out; the capture options carry it to the recorder.
    #[test]
    fn max_recording_is_per_output_mode_and_capture_options_default_to_the_whole_take() {
        assert_eq!(max_recording(OutputMode::WholeTake), MAX_RECORDING);
        assert_eq!(MAX_RECORDING, Duration::from_secs(120), "existing whole-take behaviour");
        assert_eq!(max_recording(OutputMode::StreamingFinal), MAX_RECORDING_STREAMING);
        assert_eq!(max_recording(OutputMode::LiveInject), Duration::from_secs(600));
        let microphone = CaptureOptions {
            live: false,
            max_duration: MAX_RECORDING,
            long: false,
            source: RecordingSource::Microphone,
            output_device: None,
            echo_cancel: false,
        };
        assert_eq!(CaptureOptions::default(), microphone);
        assert_eq!(CaptureOptions::LIVE, CaptureOptions { live: true, ..microphone });
        assert!(format!("{:?}", CaptureOptions::LIVE).contains("live: true"));
    }

    /// docs/dictation.md §22: a take on this computer records from the settings' source for their
    /// length, and is `long` (a recording file, segments) only when it may run past 120 s.
    #[test]
    fn a_dictation_take_follows_the_recording_settings() {
        let settings = RecordingSettings::default();
        let options = CaptureOptions::dictation(&settings, true);
        assert_eq!((options.max_duration, options.long, options.live), (Duration::from_secs(600), true, true), "10 minutes by default");
        assert_eq!(options.source, RecordingSource::Microphone);
        let short = |max_minutes| CaptureOptions::dictation(&RecordingSettings { max_minutes, ..RecordingSettings::default() }, false);
        assert_eq!((short(1).max_duration, short(1).long), (Duration::from_secs(60), false));
        assert_eq!((short(2).max_duration, short(2).long), (MAX_RECORDING, false), "2 minutes is a short take, as before");
        assert_eq!((short(120).max_duration, short(120).long), (Duration::from_secs(7200), true));
        let mixed = RecordingSettings { source: RecordingSource::Mixed, output_device: Some("wasapi:out".into()), max_minutes: 30, echo_cancel: true };
        let options = CaptureOptions::dictation(&mixed, false);
        assert_eq!((options.source, options.output_device.as_deref()), (RecordingSource::Mixed, Some("wasapi:out")));
        // docs/dictation.md §22.6: the echo is cancelled in a mixed take unless it is switched off,
        // and only there: a take of one source has nothing to cancel.
        assert!(options.echo_cancel);
        assert!(!CaptureOptions::dictation(&RecordingSettings { echo_cancel: false, ..mixed.clone() }, false).echo_cancel);
        for source in [RecordingSource::Microphone, RecordingSource::System] {
            assert!(!CaptureOptions::dictation(&RecordingSettings { source, ..mixed.clone() }, false).echo_cancel, "{source:?}");
        }
        assert!(RecordingSettings::default().echo_cancel, "on by default");
    }

    /// `Recording::slice_from_ms` keeps the audio after the cut (sample-exact), re-encodes a valid
    /// WAV and recomputes the length; out-of-range cuts and unparseable input are harmless.
    #[test]
    fn recording_slices_from_a_millisecond_offset() {
        let samples: Vec<i16> = (0..1600).map(|i| (i % 200) as i16 * 100).collect();
        let rec = Recording { wav: wav::encode_pcm16(&samples, 16_000), duration_ms: 100, sample_rate_hz: 16_000 };
        let tail = rec.slice_from_ms(40);
        assert_eq!(tail.sample_rate_hz, 16_000);
        assert_eq!(tail.duration_ms, 60);
        let (frames, _) = wav::pcm_data(&tail.wav).unwrap().as_chunks::<2>();
        assert_eq!(frames.len(), 960, "1600 − 40 ms × 16 samples/ms");
        assert_eq!(i16::from_le_bytes(frames[0]), samples[640], "the cut is sample-exact");
        assert_eq!(rec.slice_from_ms(0), rec, "a cut at zero is the whole take");
        let past = rec.slice_from_ms(5_000);
        assert_eq!(past.duration_ms, 0);
        assert_eq!(wav::pcm_data(&past.wav).unwrap().len(), 0);
        let junk = Recording { wav: b"not a wav".to_vec(), duration_ms: 7, sample_rate_hz: 16_000 };
        assert_eq!(junk.slice_from_ms(3), junk, "unparseable audio is returned whole");
        let zero_rate = Recording { wav: wav::encode_pcm16(&[1, 2, 3], 0), duration_ms: 0, sample_rate_hz: 0 };
        assert_eq!(zero_rate.slice_from_ms(1).duration_ms, 0);
    }
}
