//! The dictation state machine (docs/dictation.md §2, §11, §12).
//!
//! Single-threaded by construction: every transition happens on the core task, either from a
//! command (`start` / `stop` / `cancel`) or from an [`Internal`] notification that a spawned
//! task — the device open / close (blocking), the live decode worker, the pipeline, the live
//! injections, the auto-stop timer, the dwell timer — sends back tagged with the session it belongs
//! to. A notification for an older session is dropped, which is all the cancellation the pipeline
//! needs.
//!
//! The live preview (§11) is a second, optional path next to the recording: when a
//! [`StreamingTranscriber`] is plugged in and `live_preview_ready`, the capture's 16 kHz tap is
//! decoded on a blocking thread ([`run_live`]) and its partials land in `Listening.live`; on stop
//! the text moves to `Processing.preview`.
//!
//! Every recogniser's text is first brought to the configured Chinese script (§17): whole takes,
//! the live worker's partials / sentences / flush, and the `live_inject` remainder.
//!
//! The vocabulary (§16) wraps the text on its way: dictionary corrections right after the
//! recogniser (and for every `live_inject` sentence), the glossary for the recogniser and the
//! refiner, the replacement rules right before injection. Every run takes the snapshot current at
//! its `start`, and a vocabulary problem only ever falls back to the unmodified text.
//!
//! Scenes (§18): with a [`ForegroundProbe`] plugged in, `start` asks it (on a blocking thread, at
//! most [`PROBE_DEADLINE`]) which application is in front before the device opens; the first
//! enabled scene matching it overrides the take's output mode, refine switch and style, language
//! and Chinese script, and gives the refiner an instruction. The take's context (app + scene)
//! rides on `DictationStatus.context` and into the history. No answer, no match: the globals.
//! A voice edit (§19) is a take of its own kind ([`TakeKind::Edit`]): the microphone records the
//! spoken instruction while the injector copies the foreground application's selection; once both
//! are here the instruction is recognised (script, dictionary — no rules), the refiner rewrites the
//! selection by it, and the injector pastes the result over the selection. Edit takes ignore the
//! output mode (always a whole take) and refuse to start without a refiner.
//!
//! The output mode (§12) decides where the final text comes from. `whole_take` (the default):
//! every failure on the live path degrades the preview and nothing else, the text comes from the
//! whole take. `streaming_final`: the flushed stream (committed sentences + tail) *is* the text and
//! the whole-take transcriber is skipped. `live_inject`: every committed sentence is pasted at once,
//! the flushed tail last. Both streaming modes fall back to the whole take — kept in full — when
//! the stream fails before the first sentence, and `live_inject` transcribes only the audio after
//! the last pasted sentence when it fails later, never pasting anything twice.

use std::collections::VecDeque;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use uuid::Uuid;

use super::ports::{
    AudioSource, Capture, CaptureOptions, DWELL, DWELL_WITH_TEXT, DictationError, ForegroundApp, ForegroundProbe, Injection, Injector, LIVE_CHUNK_SAMPLES,
    LevelFrame, LivePcm, MAX_EDIT_SELECTION_CHARS, MIN_RECORDING, PARTIAL_THROTTLE, PROBE_DEADLINE, Recording, RefineContext, RefineHints, Refiner, Segment,
    SelectionTiming, ServiceProbe, StreamEvent, StreamFinal, StreamingTranscriber, Transcriber, Transcript, Via, max_recording,
};
use super::wav;
use super::{DictationPhase, DictationStatus, FailureCode, LiveText, OutputMode, ProcessingStage, TakeKind, inject_separator, join_text};
use crate::engines::{ChineseScript, ResolvedEngines};
use crate::history::{EditRecord, HistoryEntry, Outcome};
use crate::hotkey::Modifier;
use crate::models::ModelManager;
use crate::scenes::{AppRef, ContextSharing, LANGUAGE_AUTO, Scene, TakeContext, match_scene};
use crate::script::normalized;
use crate::vocabulary::{Step, Vocabulary, VocabularyHits};

/// Builds the network clients for a resolved configuration. Called at startup and again whenever
/// the engine settings or a secret change.
pub type EngineFactory = Arc<dyn Fn(&ResolvedEngines) -> (Arc<dyn Transcriber>, Option<Arc<dyn Refiner>>) + Send + Sync>;

/// Everything the shell plugs into the core.
#[derive(Clone)]
pub struct DictationPorts {
    /// Microphone.
    pub audio: Arc<dyn AudioSource>,
    /// Text delivery.
    pub injector: Arc<dyn Injector>,
    /// ASR / refine client factory.
    pub factory: EngineFactory,
    /// Local model library (docs/dictation.md §10); `None` on shells without local models, which
    /// then report an empty library and refuse the model commands.
    pub models: Option<Arc<dyn ModelManager>>,
    /// Streaming recogniser for the live preview (docs/dictation.md §11); `None` on shells without
    /// one (the phone). Used only while `EngineSettings.live_preview` is on and the streaming model
    /// is installed.
    pub streaming: Option<Arc<dyn StreamingTranscriber>>,
    /// Names the application in front when a take starts (docs/dictation.md §18.2); `None` on
    /// shells without one (the phone, the fakes): takes then run without a scene, as before.
    pub probe: Option<Arc<dyn ForegroundProbe>>,
    /// Lists a provider's models for the engines pane's 测试连接 (docs/dictation.md §3.3); `None`
    /// on shells without HTTP (the phone, the fakes): the probe then answers `unsupported`.
    pub service_probe: Option<Arc<dyn ServiceProbe>>,
}

impl std::fmt::Debug for DictationPorts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DictationPorts").finish_non_exhaustive()
    }
}

/// Notifications from spawned work back to the state machine.
pub enum Internal {
    /// The foreground probe answered, failed or ran out of time (docs/dictation.md §18.4): the
    /// take's scene is decided and the device opens next.
    Context {
        /// Session the probe was for.
        session: u64,
        /// The application in front, or `None` (no answer in time, an error, nothing nameable).
        app: Option<ForegroundApp>,
    },
    /// The device open finished (on a blocking thread).
    CaptureStarted {
        /// Session the open was for.
        session: u64,
        /// The running capture, or why it did not start.
        result: Result<Box<dyn Capture>, DictationError>,
    },
    /// The device close + PCM conversion finished (on a blocking thread).
    Stopped {
        /// Session.
        session: u64,
        /// The recording, or why the stop failed.
        result: Result<Recording, DictationError>,
    },
    /// The device delivered its first samples (from the audio thread).
    CaptureReady {
        /// Session.
        session: u64,
    },
    /// The streaming recogniser's current sentence changed (throttled to [`PARTIAL_THROTTLE`]).
    Partial {
        /// Session.
        session: u64,
        /// The current sentence so far.
        current: String,
    },
    /// The streaming recogniser committed a sentence at an endpoint.
    Segment {
        /// Session.
        session: u64,
        /// The sentence.
        segment: Segment,
    },
    /// The live preview stopped following the audio (open failed, tap overrun, decoder error,
    /// panic); the recording is unaffected. In a streaming output mode this is the signal to fall
    /// back to the whole take (docs/dictation.md §12).
    StreamDegraded {
        /// Session.
        session: u64,
        /// Why.
        reason: String,
    },
    /// The live worker finished (the capture closed): the flushed committed sentences and tail.
    StreamFinished {
        /// Session.
        session: u64,
        /// The flushed result, or why the flush failed.
        result: Result<StreamFinal, DictationError>,
    },
    /// `live_inject` (docs/dictation.md §12): one sentence's injection came back.
    LiveInjected {
        /// Session.
        session: u64,
        /// Which piece (in queue order).
        idx: usize,
        /// The outcome.
        result: Result<Injection, DictationError>,
    },
    /// `live_inject` degraded after some sentences were pasted: the whole-take transcriber's answer
    /// for the audio after the last of them.
    Remainder {
        /// Session.
        session: u64,
        /// The transcript, or why it failed.
        result: Result<Transcript, DictationError>,
    },
    /// Voice edit (docs/dictation.md §19): the selection copy finished (on a blocking thread).
    SelectionCopied {
        /// Session.
        session: u64,
        /// The selected text (`None`: nothing selected), or why it could not be read.
        result: Result<Option<String>, DictationError>,
    },
    /// The recording reached its maximum length ([`max_recording`]).
    AutoStop {
        /// Session the timer was armed for.
        session: u64,
    },
    /// The pipeline moved to its next step.
    Stage {
        /// Session.
        session: u64,
        /// Step.
        stage: ProcessingStage,
    },
    /// The pipeline finished (ASR failure, or a result with the injection outcome).
    Finished {
        /// Session.
        session: u64,
        /// Result.
        result: Result<PipelineOutcome, DictationError>,
    },
    /// A terminal state has been shown long enough.
    DwellOver {
        /// Session.
        session: u64,
    },
}

impl std::fmt::Debug for Internal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Context { session, app } => {
                f.debug_struct("Context").field("session", session).field("app", &app.as_ref().map(|a| a.app_id.as_str())).finish()
            }
            Self::CaptureStarted { session, result } => f.debug_struct("CaptureStarted").field("session", session).field("ok", &result.is_ok()).finish(),
            Self::Stopped { session, result } => {
                f.debug_struct("Stopped").field("session", session).field("result", &result.as_ref().map(|r| r.duration_ms)).finish()
            }
            Self::CaptureReady { session } => f.debug_struct("CaptureReady").field("session", session).finish(),
            // Preview text never reaches the log, like transcripts.
            Self::Partial { session, current } => f.debug_struct("Partial").field("session", session).field("chars", &current.chars().count()).finish(),
            Self::Segment { session, segment } => f.debug_struct("Segment").field("session", session).field("end_ms", &segment.end_ms).finish(),
            Self::StreamDegraded { session, reason } => f.debug_struct("StreamDegraded").field("session", session).field("reason", reason).finish(),
            Self::StreamFinished { session, result } => {
                f.debug_struct("StreamFinished").field("session", session).field("segments", &result.as_ref().map(|r| r.committed.len())).finish()
            }
            Self::LiveInjected { session, idx, result } => {
                f.debug_struct("LiveInjected").field("session", session).field("idx", idx).field("result", &result.as_ref().map(|i| i.via)).finish()
            }
            Self::Remainder { session, result } => {
                f.debug_struct("Remainder").field("session", session).field("result", &result.as_ref().map(|t| t.text.chars().count())).finish()
            }
            // The selection never reaches the log either: its length only.
            Self::SelectionCopied { session, result } => f
                .debug_struct("SelectionCopied")
                .field("session", session)
                .field("chars", &result.as_ref().map(|s| s.as_ref().map(|t| t.chars().count())))
                .finish(),
            Self::AutoStop { session } => f.debug_struct("AutoStop").field("session", session).finish(),
            Self::Stage { session, stage } => f.debug_struct("Stage").field("session", session).field("stage", stage).finish(),
            Self::Finished { session, result } => f.debug_struct("Finished").field("session", session).field("result", result).finish(),
            Self::DwellOver { session } => f.debug_struct("DwellOver").field("session", session).finish(),
        }
    }
}

/// What the pipeline produced once ASR succeeded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineOutcome {
    /// ASR text.
    pub raw_text: String,
    /// Text handed to the injector.
    pub text: String,
    /// The refiner ran and its output was used.
    pub refined: bool,
    /// ASR latency.
    pub asr_ms: u64,
    /// Refine latency.
    pub refine_ms: Option<u64>,
    /// Refine skip / failure reason.
    pub refine_error: Option<String>,
    /// Model the refiner reported.
    pub refine_model: Option<String>,
    /// Which dictionary entries and rules fired (docs/dictation.md §16.3).
    pub vocabulary: VocabularyHits,
    /// Injection outcome.
    pub injection: Result<Injection, DictationError>,
    /// A voice edit's instruction and selection (docs/dictation.md §19); `None` for dictation.
    /// Boxed: two strings would make every `Internal` notification this much larger.
    pub edit: Option<Box<EditRecord>>,
}

/// What the runtime has to do after a transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Publish the new status.
    Status(DictationStatus),
    /// Append to the history.
    Record(HistoryEntry),
}

/// What the user asked for while the device was still opening.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Then {
    Stop,
    Cancel,
}

/// Where the microphone is. `Capture` is `Send` but not `Sync`, hence the mutex around it in the
/// engine (never contended: only the core task touches it).
enum Mic {
    Closed,
    /// `AudioSource::start` is running on a blocking thread.
    Opening {
        then: Option<Then>,
    },
    Open(Box<dyn Capture>),
    /// `Capture::stop` is running on a blocking thread.
    Stopping,
}

/// One text handed to the injector in `live_inject`: a committed sentence with its separator, the
/// flushed tail, the transcribed remainder, or the accumulated rest after a clipboard fallback.
#[derive(Clone, Debug)]
struct Piece {
    /// Queue position, `0`-based per run.
    idx: usize,
    /// What the injector gets (sentence + separator).
    text: String,
    /// Characters of the sentence itself (`Cancelled.injected_chars` counts these).
    chars: usize,
}

/// `live_inject` bookkeeping (docs/dictation.md §12): sentences go to the injector one at a time,
/// in order; a clipboard fallback or an injection error switches to accumulating the rest for one
/// final write.
#[derive(Default)]
struct LiveInject {
    /// Every sentence in order, as the recogniser wrote it: committed ones, then the tail or the
    /// remainder.
    segments: Vec<Segment>,
    /// The sentences as recognised, joined like a preview (`Done.raw_text`).
    raw_text: String,
    /// The sentences as pasted (after the dictionary and the rules), joined (`Done.text`).
    text: String,
    /// What the dictionary and the rules did to the sentences so far (docs/dictation.md §16.3).
    hits: VocabularyHits,
    /// Pieces waiting for the injector.
    queue: VecDeque<Piece>,
    /// The piece the injector has now.
    in_flight: Option<Piece>,
    next_idx: usize,
    /// Sentences pasted so far (`LiveText.injected`).
    pasted: usize,
    /// Characters pasted so far (`Cancelled.injected_chars`).
    pasted_chars: usize,
    /// Once a paste fell back to the clipboard (or failed): the text not pasted yet, written in
    /// one go at the end; `Some` from the fallback on.
    rest: Option<String>,
    /// The note the fallback came with (the history's clipboard reason).
    note: Option<String>,
    /// The final write of `rest` is with the injector.
    writing_rest: bool,
    /// The last sentence (tail / remainder) has been queued: when the queue drains the run is done.
    closing: bool,
    /// End of the last committed sentence, where the remainder starts.
    last_end_ms: u64,
    /// Committed sentences the worker reported (`Segment`), pasted or not: the flush repeats them
    /// and this index keeps them from being queued twice.
    seen: usize,
}

/// Where a voice edit's selection is (docs/dictation.md §19).
enum SelectionState {
    /// Not copied yet: the copy waits for the hotkey's key to come up ([`SelectionTiming::AfterKeyUp`]).
    Pending,
    /// The injector is copying on a blocking thread.
    Copying,
    /// The text to rewrite.
    Ready(String),
}

/// A voice edit take's state (docs/dictation.md §19).
struct EditTake {
    /// The edit hotkey's modifiers the user may still hold when the copy chord goes out.
    held: Vec<Modifier>,
    selection: SelectionState,
    /// The foreground probe answered (or there is none): the terminal guard (§19.2) has had its
    /// say, so the copy chord may go out.
    app_known: bool,
    /// A copy was asked for before the probe answered; it goes out with the answer.
    copy_wanted: bool,
}

/// The message of an edit take refused for want of an LLM.
pub const EDIT_NEEDS_REFINE: &str = "语音编辑需要 AI 润色服务：请先配置润色的 API 密钥";

/// One run's output-mode state (docs/dictation.md §12), reset by `start`.
struct Take {
    /// The mode resolved at `start`.
    requested: OutputMode,
    /// The mode the run is on now: `requested`, or `WholeTake` after the stream failed before its
    /// first sentence.
    mode: OutputMode,
    /// Why the streaming path was abandoned or cut short (`Done.live_error`); only recorded when a
    /// streaming mode was requested.
    live_error: Option<String>,
    /// The live worker will report (`StreamFinished` / `StreamDegraded`) — it was spawned and has
    /// not reported yet.
    worker_pending: bool,
    /// The recording, once `Stopped` arrived (the streaming modes hold it until the flush).
    recording: Option<Recording>,
    /// The flushed stream, once `StreamFinished` arrived (held until the recording).
    flushed: Option<StreamFinal>,
    /// When `stop` was called (`asr_ms` of the streaming modes = finalisation time).
    finalize_started: Option<Instant>,
    /// `asr_ms` once the streaming text is complete.
    asr_ms: u64,
    inject: LiveInject,
    /// The dictionary / rules snapshot this run uses from start to finish (docs/dictation.md §16.3).
    vocabulary: Arc<Vocabulary>,
    /// The scene list this run matches against (docs/dictation.md §18), taken at `start`.
    scenes: Arc<Vec<Scene>>,
    /// What of the context may go to the refiner, taken at `start`.
    sharing: ContextSharing,
    /// The application in front when the run started, once the probe answered.
    app: Option<ForegroundApp>,
    /// The scene the run matched; its overrides apply to this run only.
    scene: Option<Scene>,
    /// The voice edit's selection (docs/dictation.md §19); `None` for a dictation take.
    edit: Option<EditTake>,
    /// Where this take's audio comes from instead of the microphone: a paired phone
    /// (docs/dictation.md §20, [`super::remote::RemoteFeed`]).
    source: Option<Arc<dyn AudioSource>>,
}

impl Take {
    fn new(requested: OutputMode, vocabulary: Arc<Vocabulary>, scenes: Arc<Vec<Scene>>, sharing: ContextSharing) -> Self {
        Self {
            requested,
            mode: requested,
            live_error: None,
            worker_pending: false,
            recording: None,
            flushed: None,
            finalize_started: None,
            asr_ms: 0,
            inject: LiveInject::default(),
            vocabulary,
            scenes,
            sharing,
            app: None,
            scene: None,
            edit: None,
            source: None,
        }
    }

    /// Milliseconds since `stop`.
    fn finalize_ms(&self) -> u64 {
        self.finalize_started.map_or(0, |t| u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX))
    }
}

/// The state machine plus the ports it drives.
pub struct DictationEngine {
    audio: Arc<dyn AudioSource>,
    transcriber: Arc<dyn Transcriber>,
    refiner: Option<Arc<dyn Refiner>>,
    injector: Arc<dyn Injector>,
    factory: EngineFactory,
    streaming: Option<Arc<dyn StreamingTranscriber>>,
    /// `ResolvedEngines::live_preview_ready()` of the last configuration: whether the next start
    /// opens a live tap and a streaming session.
    live_ready: bool,
    /// `EngineSettings.output_mode` of the last configuration; resolved per run at `start`.
    output_mode: OutputMode,
    levels: broadcast::Sender<LevelFrame>,
    internal: mpsc::Sender<Internal>,
    status: DictationStatus,
    mic: Mutex<Mic>,
    /// The one armed timer: auto-stop while listening, dwell in a terminal state.
    timer: Option<JoinHandle<()>>,
    /// The pipeline task while `Processing`; aborted on cancel so a cancelled run never injects.
    pipeline: Option<JoinHandle<()>>,
    language: Option<String>,
    /// `EngineSettings.chinese_script` of the last configuration (docs/dictation.md §17).
    chinese_script: ChineseScript,
    refine_enabled: bool,
    asr_model: String,
    refine_model: String,
    recording_ms: u64,
    take: Take,
    /// The compiled dictionary and rules the next run takes (replaced by the runtime on every change).
    vocabulary: Arc<Vocabulary>,
    /// The foreground probe (docs/dictation.md §18.2), when the shell has one.
    probe: Option<Arc<dyn ForegroundProbe>>,
    /// The scenes the next run matches against (replaced by the runtime on every change).
    scenes: Arc<Vec<Scene>>,
    /// `Settings.context_sharing` for the next run.
    context_sharing: ContextSharing,
    /// `Settings.microphone`: the device the microphone port opens (`None` = the default).
    microphone: Option<String>,
}

/// Why a scene's streaming output mode could not be honoured (docs/dictation.md §18.4); the take
/// runs as a whole take and `Done.live_error` says so.
const SCENE_MODE_NOT_READY: &str = "场景要求边说边识别，但实时预览未就绪（已关闭或实时识别模型未下载），本次按整段输出处理";
/// The same, on a shell without a streaming recogniser.
const SCENE_MODE_NO_STREAMING: &str = "场景要求边说边识别，但此设备不支持实时识别，本次按整段输出处理";

impl std::fmt::Debug for DictationEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mic = match &*self.mic.lock() {
            Mic::Closed => "closed",
            Mic::Opening { .. } => "opening",
            Mic::Open(_) => "open",
            Mic::Stopping => "stopping",
        };
        f.debug_struct("DictationEngine")
            .field("status", &self.status)
            .field("mic", &mic)
            .field("live", &self.live_enabled())
            .field("mode", &self.take.mode)
            .field("scene", &self.take.scene.as_ref().map(|s| s.id))
            .finish_non_exhaustive()
    }
}

/// Capacity of the internal notification queue.
const INTERNAL_QUEUE: usize = 32;

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)).unwrap_or(0)
}

/// Stop a capture nobody wants any more (device release), off the core task.
fn release(capture: Box<dyn Capture>) {
    tokio::task::spawn_blocking(move || {
        if let Err(e) = capture.stop() {
            tracing::debug!(error = %e, "discarded capture did not stop cleanly");
        }
    });
}

impl DictationEngine {
    /// The injector the takes deliver through; a phone's text uses it too (docs/dictation.md §20.6).
    pub fn injector(&self) -> Arc<dyn Injector> {
        self.injector.clone()
    }

    /// Build the engine; `levels` receives every level frame while a capture runs. The returned
    /// receiver must be polled by the owner and fed back through [`DictationEngine::on_internal`].
    pub fn new(ports: DictationPorts, engines: &ResolvedEngines, levels: broadcast::Sender<LevelFrame>) -> (Self, mpsc::Receiver<Internal>) {
        let (tx, rx) = mpsc::channel(INTERNAL_QUEUE);
        let (transcriber, refiner) = (ports.factory)(engines);
        let engine = Self {
            audio: ports.audio,
            transcriber,
            refiner,
            injector: ports.injector,
            factory: ports.factory,
            streaming: ports.streaming,
            live_ready: engines.live_preview_ready(),
            output_mode: engines.output_mode,
            levels,
            internal: tx,
            status: DictationStatus::default(),
            mic: Mutex::new(Mic::Closed),
            timer: None,
            pipeline: None,
            language: engines.language.clone(),
            chinese_script: engines.chinese_script,
            refine_enabled: engines.refine_enabled,
            asr_model: engines.asr_model.clone(),
            refine_model: engines.refine_model.clone(),
            recording_ms: 0,
            take: Take::new(OutputMode::WholeTake, Arc::new(Vocabulary::empty()), Arc::new(Vec::new()), ContextSharing::default()),
            vocabulary: Arc::new(Vocabulary::empty()),
            probe: ports.probe,
            scenes: Arc::new(Vec::new()),
            context_sharing: ContextSharing::default(),
            microphone: None,
        };
        engine.warm_streaming();
        engine.warm_transcriber();
        (engine, rx)
    }

    /// The compiled dictionary and rules for the runs that start from now on (docs/dictation.md
    /// §16); a run already going keeps the snapshot it started with.
    pub fn set_vocabulary(&mut self, vocabulary: Arc<Vocabulary>) {
        self.vocabulary = vocabulary;
    }

    /// The scenes the runs that start from now on match against (docs/dictation.md §18); a run
    /// already going keeps the list it started with.
    pub fn set_scenes(&mut self, scenes: Arc<Vec<Scene>>) {
        self.scenes = scenes;
    }

    /// `Settings.context_sharing` for the runs that start from now on (docs/dictation.md §18.5).
    pub fn set_context_sharing(&mut self, sharing: ContextSharing) {
        self.context_sharing = sharing;
    }

    /// `Settings.microphone` for the takes that start from now on: the device id handed to the
    /// microphone port (`None` = the system default).
    pub fn set_microphone(&mut self, device: Option<String>) {
        self.microphone = device;
    }

    /// Rebuild the clients for a changed configuration. A pipeline already running keeps the
    /// clients it started with; a live preview already running keeps its session; a run already
    /// started keeps its output mode.
    pub fn configure(&mut self, engines: &ResolvedEngines) {
        let (transcriber, refiner) = (self.factory)(engines);
        self.transcriber = transcriber;
        self.refiner = refiner;
        self.language = engines.language.clone();
        self.chinese_script = engines.chinese_script;
        self.refine_enabled = engines.refine_enabled;
        self.asr_model = engines.asr_model.clone();
        self.refine_model = engines.refine_model.clone();
        self.live_ready = engines.live_preview_ready();
        self.output_mode = engines.output_mode;
        self.warm_streaming();
        self.warm_transcriber();
    }

    /// Whether the next start previews: a streaming port is plugged in and the configuration says
    /// live preview is on with its model installed.
    pub fn live_enabled(&self) -> bool {
        self.live_ready && self.streaming.is_some()
    }

    /// The output mode the next start runs with (docs/dictation.md §12): the configured one, or
    /// `whole_take` when a streaming mode was asked for without the live preview being usable.
    /// The second value says why it fell back.
    pub fn effective_output_mode(&self) -> (OutputMode, Option<&'static str>) {
        self.resolve_output_mode(self.output_mode)
    }

    /// The mode a run asking for `mode` gets: the streaming modes need the live preview, the rest
    /// is `whole_take`, with the reason (docs/dictation.md §12; the same rule for a scene, §18.4).
    fn resolve_output_mode(&self, mode: OutputMode) -> (OutputMode, Option<&'static str>) {
        match mode {
            OutputMode::WholeTake => (OutputMode::WholeTake, None),
            mode if self.live_enabled() => (mode, None),
            _ if !self.live_ready => (OutputMode::WholeTake, Some("live preview is not ready (switched off or streaming model not installed)")),
            _ => (OutputMode::WholeTake, Some("this shell has no streaming recogniser")),
        }
    }

    /// The mode the current (or last) run is on.
    pub fn current_mode(&self) -> OutputMode {
        self.take.mode
    }

    /// Preload the streaming model (2.6 s measured) so the first partial is not late. No-op when
    /// live preview is not ready; the port itself makes it idempotent.
    fn warm_streaming(&self) {
        if let (true, Some(streaming)) = (self.live_ready, &self.streaming) {
            streaming.warm();
        }
    }

    /// Preload the recogniser the next take uses (docs/dictation.md §10.7: a local model takes
    /// 0.6–2 s to load on a CPU, over 10 s the first time on a GPU), at construction and after every
    /// configuration change. Not at a key press: a scene may ask for another language, and the take
    /// loads what it needs itself.
    fn warm_transcriber(&self) {
        self.transcriber.warm(self.language.as_deref());
    }

    /// Current status.
    pub fn status(&self) -> &DictationStatus {
        &self.status
    }

    /// `DictationStart`: open the microphone. `Listening` is reported at once (the pill must
    /// follow the key press); the device opens on a blocking thread and a failure turns the
    /// session into `Failed`. Interrupts a terminal dwell; refused while a recording or a pipeline
    /// run is in progress.
    pub fn start(&mut self) -> Result<Vec<Effect>, DictationError> {
        self.begin(TakeKind::Dictation, Vec::new(), None)
    }

    /// A dictation take whose audio comes from `source` instead of the microphone — a paired
    /// phone's stream (docs/dictation.md §20). Everything else is an ordinary take: the scene of
    /// the application in front, the output mode, the pipeline, the delivery, the history.
    pub fn start_from(&mut self, source: Arc<dyn AudioSource>) -> Result<Vec<Effect>, DictationError> {
        self.begin(TakeKind::Dictation, Vec::new(), Some(source))
    }

    /// The microphone port (the phone streams its takes from it, docs/dictation.md §20).
    pub fn microphone(&self) -> Arc<dyn AudioSource> {
        self.audio.clone()
    }

    /// The level stream `CoreHandle::levels` subscribes to: a capture the runtime opens itself (a
    /// phone's take, docs/dictation.md §20) reports its frames here too.
    pub fn levels(&self) -> broadcast::Sender<LevelFrame> {
        self.levels.clone()
    }

    /// A voice edit take (docs/dictation.md §19): like [`DictationEngine::start`] — the microphone
    /// opens for the spoken instruction — for a take that rewrites the foreground application's
    /// selection. Without a refiner (no LLM configured) the take fails at once with
    /// `edit_unavailable` and the microphone stays closed. `held`: the edit hotkey's modifiers the
    /// user may still hold when the copy chord goes out. The copy itself is
    /// [`DictationEngine::capture_selection`] — the runtime calls it at the press or after the
    /// key-up ([`DictationEngine::selection_timing`]); a stop that finds it not done yet copies
    /// first. The output mode does not apply: an edit take is always a whole take.
    pub fn start_edit(&mut self, held: Vec<Modifier>) -> Result<Vec<Effect>, DictationError> {
        self.begin(TakeKind::Edit, held, None)
    }

    /// When the injector can send the copy chord (docs/dictation.md §19).
    pub fn selection_timing(&self) -> SelectionTiming {
        self.injector.selection_timing()
    }

    /// Copy the selection of the current edit take now (docs/dictation.md §19); the result arrives
    /// as [`Internal::SelectionCopied`]. A no-op unless the take is an edit take that is still
    /// listening or processing and has not copied yet — the runtime may call it at the press,
    /// after the key-up and before a stop without keeping count.
    pub fn capture_selection(&mut self) {
        if !matches!(self.status.phase, DictationPhase::Listening { .. } | DictationPhase::Processing { .. }) {
            return;
        }
        let Some(edit) = self.take.edit.as_mut() else { return };
        if !matches!(edit.selection, SelectionState::Pending) {
            return;
        }
        if !edit.app_known {
            // §19.2: no key is pressed before the probe said which application is in front (at most
            // `PROBE_DEADLINE`) — a terminal is refused without one.
            edit.copy_wanted = true;
            return;
        }
        edit.selection = SelectionState::Copying;
        let held = edit.held.clone();
        let (session, injector, tx) = (self.status.session, self.injector.clone(), self.internal.clone());
        tracing::debug!(session, ?held, "copying the selection");
        tokio::spawn(async move {
            let result = match tokio::task::spawn_blocking(move || injector.copy_selection(&held)).await {
                Ok(result) => result,
                Err(e) => Err(DictationError::Selection(format!("copy task failed: {e}"))),
            };
            let _ = tx.send(Internal::SelectionCopied { session, result }).await;
        });
    }

    fn begin(&mut self, kind: TakeKind, held: Vec<Modifier>, source: Option<Arc<dyn AudioSource>>) -> Result<Vec<Effect>, DictationError> {
        match self.status.phase {
            DictationPhase::Listening { .. } | DictationPhase::Processing { .. } => return Err(DictationError::Busy),
            _ => {}
        }
        self.disarm();
        self.abort_pipeline();
        self.status.session += 1;
        self.status.kind = kind;
        let session = self.status.session;
        let mode = match kind {
            TakeKind::Dictation => {
                let (mode, reason) = self.effective_output_mode();
                match reason {
                    Some(reason) => tracing::info!(session, requested = self.output_mode.as_str(), effective = mode.as_str(), reason, "output mode fell back"),
                    None => tracing::info!(session, mode = mode.as_str(), "output mode"),
                }
                mode
            }
            TakeKind::Edit => {
                tracing::info!(session, "edit take: a whole take whatever the output mode");
                OutputMode::WholeTake
            }
        };
        self.take = Take::new(mode, self.vocabulary.clone(), self.scenes.clone(), self.context_sharing);
        self.take.source = source;
        self.status.context = None;
        if kind == TakeKind::Edit {
            if self.refiner.is_none() {
                tracing::warn!(session, "edit take refused: no LLM is configured");
                if let Mic::Open(capture) = std::mem::replace(&mut *self.mic.lock(), Mic::Closed) {
                    release(capture);
                }
                return Ok(self.fail(&DictationError::EditUnavailable(EDIT_NEEDS_REFINE.to_owned()), None));
            }
            self.take.edit = Some(EditTake { held, selection: SelectionState::Pending, app_known: self.probe.is_none(), copy_wanted: false });
        }
        // A previous session's capture still open (its stop never came back) is released now.
        if let Mic::Open(capture) = std::mem::replace(&mut *self.mic.lock(), Mic::Opening { then: None }) {
            release(capture);
        }
        self.warm_streaming();
        let listening = self.set_phase(DictationPhase::Listening { started_at: now_ms(), ready: false, live: None, locked: false });
        // With a probe the device opens once it answered (`Internal::Context`), so the scene's
        // overrides are known before the capture and the live worker start (docs/dictation.md §18.4).
        match self.probe.clone() {
            Some(probe) => self.spawn_probe(probe),
            None => self.open_capture(),
        }
        Ok(vec![listening])
    }

    /// Ask the probe which application is in front, on a blocking thread and for at most
    /// [`PROBE_DEADLINE`]; [`Internal::Context`] follows in every case (no answer = `None`).
    fn spawn_probe(&self, probe: Arc<dyn ForegroundProbe>) {
        let session = self.status.session;
        let tx = self.internal.clone();
        tokio::spawn(async move {
            let app = match tokio::time::timeout(PROBE_DEADLINE, tokio::task::spawn_blocking(move || probe.foreground())).await {
                Ok(Ok(Ok(app))) => app,
                Ok(Ok(Err(e))) => {
                    tracing::debug!(session, error = %e, "foreground probe failed; no scene");
                    None
                }
                Ok(Err(e)) => {
                    tracing::warn!(session, error = %e, "foreground probe task failed; no scene");
                    None
                }
                Err(_) => {
                    tracing::info!(session, deadline_ms = PROBE_DEADLINE.as_millis(), "foreground probe did not answer in time; no scene");
                    None
                }
            };
            let _ = tx.send(Internal::Context { session, app }).await;
        });
    }

    /// Open the device for the current session on a blocking thread ([`Internal::CaptureStarted`]
    /// follows), with the cap of the take's output mode, and arm the auto-stop while listening.
    fn open_capture(&mut self) {
        let session = self.status.session;
        let mode = self.take.mode;
        let levels = self.levels.clone();
        let on_level: Box<dyn Fn(LevelFrame) + Send> = Box::new(move |frame| {
            let _ = levels.send(frame);
        });
        // From the audio thread: never block. A full queue only loses the `ready` mark.
        let ready_tx = self.internal.clone();
        let on_ready: Box<dyn FnOnce() + Send> = Box::new(move || {
            let _ = ready_tx.try_send(Internal::CaptureReady { session });
        });
        let options = CaptureOptions { live: self.live_enabled(), max_duration: max_recording(mode) };
        // A phone's take streams from its own source; the microphone port records from the
        // device the settings name (the system default without one).
        let (audio, device) = match self.take.source.clone() {
            Some(source) => (source, None),
            None => (self.audio.clone(), self.microphone.clone()),
        };
        let tx = self.internal.clone();
        tokio::spawn(async move {
            let result = match tokio::task::spawn_blocking(move || audio.start(device.as_deref(), on_level, on_ready, options)).await {
                Ok(result) => result,
                Err(e) => Err(DictationError::Audio(format!("capture task failed: {e}"))),
            };
            let _ = tx.send(Internal::CaptureStarted { session, result }).await;
        });
        if matches!(self.status.phase, DictationPhase::Listening { .. }) {
            self.arm(max_recording(mode), |session| Internal::AutoStop { session });
        }
    }

    /// The probe's answer (docs/dictation.md §18.4). Still listening: match the scenes and apply
    /// the scene to this take. Then the device opens — unless the take was cancelled meanwhile.
    fn on_context(&mut self, session: u64, app: Option<ForegroundApp>) -> Vec<Effect> {
        if session != self.status.session {
            return Vec::new();
        }
        let then = match &*self.mic.lock() {
            Mic::Opening { then } => *then,
            // Not waiting for a probe (a duplicate answer): nothing to do.
            _ => return Vec::new(),
        };
        if then == Some(Then::Cancel) || self.status.phase.is_terminal() {
            // Cancelled while the probe ran: the device never has to open.
            *self.mic.lock() = Mic::Closed;
            return Vec::new();
        }
        if let Some(edit) = self.take.edit.as_mut() {
            // §19.2: a terminal's selection is program output, nothing a rewrite can replace. In a
            // terminal the take ends here — no key pressed, the device never opened — whether it is
            // still listening or a quick stop came first.
            let terminal = app.clone().and_then(ForegroundApp::sanitized).filter(|a| self.injector.is_terminal_app(&a.app_id));
            if let Some(terminal) = terminal {
                tracing::warn!(session, app = %terminal.app_id, "edit take refused: a terminal is in front");
                *self.mic.lock() = Mic::Closed;
                return self.fail(&DictationError::EditInTerminal, None);
            }
            edit.app_known = true;
        }
        // A stop that came before the answer (a tap quicker than the probe) runs without a scene.
        let effects = if matches!(self.status.phase, DictationPhase::Listening { .. }) { self.apply_context(app) } else { Vec::new() };
        self.open_capture();
        if self.take.edit.as_ref().is_some_and(|e| e.copy_wanted) {
            // The copy asked for while the probe ran goes out now.
            self.capture_selection();
        }
        effects
    }

    /// Match `app` against the take's scenes and apply the first match to this take: the output
    /// mode now (it decides the capture), the rest where it is used. The status gains the context.
    /// A voice edit (docs/dictation.md §19) keeps the app — reference context for the rewrite and
    /// the history — but matches no scene: the scene overrides are dictation settings, the spoken
    /// instruction decides how the selection changes.
    fn apply_context(&mut self, app: Option<ForegroundApp>) -> Vec<Effect> {
        let session = self.status.session;
        let Some(app) = app.and_then(ForegroundApp::sanitized) else {
            tracing::debug!(session, "no foreground application; the take runs with the global settings");
            return Vec::new();
        };
        let scene = match self.status.kind {
            TakeKind::Dictation => match_scene(&self.take.scenes, &app).cloned(),
            TakeKind::Edit => None,
        };
        tracing::info!(session, app = %app.app_id, scene = ?scene.as_ref().map(|s| s.id), "take context");
        if let Some(requested) = scene.as_ref().and_then(|s| s.overrides.output_mode) {
            let (mode, reason) = self.resolve_output_mode(requested);
            self.take.requested = requested;
            self.take.mode = mode;
            // The globals' fallback is visible on the settings page; a scene's is not: say it in `live_error`.
            self.take.live_error = reason.map(|_| if self.live_ready { SCENE_MODE_NO_STREAMING } else { SCENE_MODE_NOT_READY }.to_owned());
            tracing::info!(session, requested = requested.as_str(), mode = mode.as_str(), "scene output mode");
        }
        self.status.context = Some(TakeContext { app: AppRef { id: app.app_id.clone(), name: app.name.clone() }, scene: scene.as_ref().map(Scene::to_ref) });
        self.take.app = Some(app);
        self.take.scene = scene;
        vec![Effect::Status(self.status.clone())]
    }

    /// The take's language hint: the scene's (`auto` = none), else the engines'.
    fn take_language(&self) -> Option<String> {
        match self.take.scene.as_ref().and_then(|s| s.overrides.language.as_deref()) {
            Some(LANGUAGE_AUTO) => None,
            Some(code) => Some(code.to_owned()),
            None => self.language.clone(),
        }
    }

    /// The take's Chinese script: the scene's, else the engines'.
    fn take_script(&self) -> ChineseScript {
        self.take.scene.as_ref().and_then(|s| s.overrides.chinese_script).unwrap_or(self.chinese_script)
    }

    /// Whether the take refines: the scene's switch, else the engines'.
    fn take_refine_enabled(&self) -> bool {
        self.take.scene.as_ref().and_then(|s| s.overrides.refine_enabled).unwrap_or(self.refine_enabled)
    }

    /// What the refiner is told besides the text (docs/dictation.md §18.5): the glossary, the
    /// take's language and style, and the context the privacy switches allow.
    fn refine_hints(&self) -> RefineHints {
        let overrides = self.take.scene.as_ref().map(|s| &s.overrides);
        let (sharing, app) = (self.take.sharing, self.take.app.as_ref());
        RefineHints {
            glossary: self.take.vocabulary.glossary().to_vec(),
            language: self.take_language(),
            style: overrides.and_then(|o| o.refine_style).unwrap_or_default(),
            context: RefineContext {
                app_name: app.filter(|_| sharing.app_name).map(|a| a.name.clone()),
                window_title: app.filter(|_| sharing.window_title).and_then(|a| a.title.clone()),
                instruction: overrides.and_then(|o| o.prompt.clone()),
            },
        }
    }

    /// `DictationStop`: close the microphone and run the pipeline. Ignored (no-op) while
    /// processing or in a terminal state — the hotkey release always follows a press, and the
    /// press may already have failed.
    pub fn stop(&mut self) -> Result<Vec<Effect>, DictationError> {
        let preview = match &self.status.phase {
            // The preview text follows the run into `Processing` (docs/dictation.md §11).
            DictationPhase::Listening { live, .. } => live.as_ref().map(LiveText::preview).filter(|p| !p.is_empty()),
            DictationPhase::Idle => return Err(DictationError::Idle),
            _ => return Ok(Vec::new()),
        };
        // A voice edit still waiting for its copy (the key is up by now): copy before the device
        // closes (docs/dictation.md §19).
        self.capture_selection();
        self.disarm();
        let mut mic = self.mic.lock();
        match std::mem::replace(&mut *mic, Mic::Stopping) {
            Mic::Open(capture) => {
                drop(mic);
                self.begin_stop(capture);
            }
            Mic::Opening { .. } => {
                // The device is still opening: stop as soon as it is there.
                *mic = Mic::Opening { then: Some(Then::Stop) };
                drop(mic);
            }
            Mic::Closed | Mic::Stopping => {
                *mic = Mic::Closed;
                drop(mic);
                return Ok(self.fail(&DictationError::Audio("capture handle is gone".to_owned()), None));
            }
        }
        self.take.finalize_started = Some(Instant::now());
        // The streaming modes wait for the flush first (§12); the whole take goes to the transcriber.
        let stage = if self.take.mode.is_streaming() { ProcessingStage::Finalizing } else { ProcessingStage::Transcribing };
        let started_at = now_ms();
        Ok(vec![self.set_phase(DictationPhase::Processing { stage, started_at, stage_started_at: started_at, preview })])
    }

    /// Start the live decode worker for this session on a blocking thread (docs/dictation.md §11).
    /// It ends by itself when the tap closes (the capture stopped or was released).
    fn spawn_live(&mut self, session: u64, pcm: Box<dyn LivePcm>, streaming: Arc<dyn StreamingTranscriber>) {
        let job = LiveJob { session, pcm, streaming, language: self.take_language(), script: self.take_script(), tx: self.internal.clone() };
        self.take.worker_pending = true;
        tokio::task::spawn_blocking(move || run_live(job));
    }

    /// Close the device and convert the audio on a blocking thread; [`Internal::Stopped`] follows.
    fn begin_stop(&self, capture: Box<dyn Capture>) {
        let session = self.status.session;
        let tx = self.internal.clone();
        tokio::spawn(async move {
            let result = match tokio::task::spawn_blocking(move || capture.stop()).await {
                Ok(result) => result,
                Err(e) => Err(DictationError::Audio(format!("capture stop task failed: {e}"))),
            };
            let _ = tx.send(Internal::Stopped { session, result }).await;
        });
    }

    /// `DictationCancel`: discard the recording or the pending result. In a terminal state this
    /// dismisses it straight to `Idle`. `live_inject` does not take back what it already pasted;
    /// `Cancelled.injected_chars` says how much that was (docs/dictation.md §12).
    pub fn cancel(&mut self) -> Result<Vec<Effect>, DictationError> {
        let cancelled = DictationPhase::Cancelled { injected_chars: self.take.inject.pasted_chars };
        match self.status.phase {
            DictationPhase::Idle => Err(DictationError::Idle),
            DictationPhase::Listening { .. } => {
                let mut mic = self.mic.lock();
                match std::mem::replace(&mut *mic, Mic::Closed) {
                    Mic::Open(capture) => release(capture),
                    Mic::Opening { .. } => *mic = Mic::Opening { then: Some(Then::Cancel) },
                    Mic::Closed | Mic::Stopping => {}
                }
                drop(mic);
                Ok(self.terminal(cancelled, DWELL))
            }
            DictationPhase::Processing { .. } => {
                // A stop still finalising releases the device by itself; its result is dropped.
                self.abort_pipeline();
                Ok(self.terminal(cancelled, DWELL))
            }
            DictationPhase::Done { .. } | DictationPhase::Failed { .. } | DictationPhase::Cancelled { .. } => {
                self.disarm();
                Ok(vec![self.set_phase(DictationPhase::Idle)])
            }
        }
    }

    /// Fold a notification from spawned work. Stale sessions are ignored.
    pub fn on_internal(&mut self, event: Internal) -> Vec<Effect> {
        match event {
            Internal::Context { session, app } => self.on_context(session, app),
            Internal::CaptureStarted { session, result } => self.on_capture_started(session, result),
            Internal::Stopped { session, result } => self.on_stopped(session, result),
            Internal::AutoStop { session } => {
                if session == self.status.session && matches!(self.status.phase, DictationPhase::Listening { .. }) {
                    tracing::info!("recording reached the maximum length; stopping");
                    return self.stop().unwrap_or_default();
                }
                Vec::new()
            }
            Internal::Stage { session, stage } => {
                if session == self.status.session {
                    return self.stage(stage);
                }
                Vec::new()
            }
            Internal::CaptureReady { session } => match &self.status.phase {
                DictationPhase::Listening { ready: false, live, locked, .. } if session == self.status.session => {
                    let (live, locked) = (live.clone(), *locked);
                    vec![self.set_phase(DictationPhase::Listening { started_at: now_ms(), ready: true, live, locked })]
                }
                _ => Vec::new(),
            },
            Internal::Partial { session, current } => self.on_live(session, |live| {
                if live.current == current {
                    return false;
                }
                live.current = current;
                true
            }),
            Internal::Segment { session, segment } => self.on_segment(session, segment),
            Internal::StreamDegraded { session, reason } => self.on_stream_degraded(session, reason),
            Internal::StreamFinished { session, result } => self.on_stream_finished(session, result),
            Internal::LiveInjected { session, idx, result } => self.on_live_injected(session, idx, result),
            Internal::Remainder { session, result } => self.on_remainder(session, result),
            Internal::SelectionCopied { session, result } => self.on_selection_copied(session, result),
            Internal::Finished { session, result } => {
                if session != self.status.session || !matches!(self.status.phase, DictationPhase::Processing { .. }) {
                    tracing::debug!(session, "pipeline result for a finished session dropped");
                    return Vec::new();
                }
                self.pipeline = None;
                match result {
                    Ok(outcome) => self.finish(outcome),
                    Err(e) => self.fail(&e, None),
                }
            }
            Internal::DwellOver { session } => {
                if session == self.status.session && self.status.phase.is_terminal() {
                    return vec![self.set_phase(DictationPhase::Idle)];
                }
                Vec::new()
            }
        }
    }

    /// Move `Processing` to `stage` (nothing outside `Processing`); the stage clock restarts.
    fn stage(&mut self, stage: ProcessingStage) -> Vec<Effect> {
        match &self.status.phase {
            DictationPhase::Processing { stage: current, started_at, preview, .. } if *current != stage => {
                let (started_at, preview) = (*started_at, preview.clone());
                vec![self.set_phase(DictationPhase::Processing { stage, started_at, stage_started_at: now_ms(), preview })]
            }
            _ => Vec::new(),
        }
    }

    /// Fold a live-preview notification into `Listening.live` (created on the first one). `update`
    /// returns whether anything changed; a degraded preview ignores further partials. Nothing
    /// happens outside `Listening` or for another session.
    fn on_live(&mut self, session: u64, update: impl FnOnce(&mut LiveText) -> bool) -> Vec<Effect> {
        match &self.status.phase {
            DictationPhase::Listening { started_at, ready, live, locked } if session == self.status.session => {
                let (started_at, ready, locked) = (*started_at, *ready, *locked);
                let mut live = live.clone().unwrap_or_default();
                if live.degraded.is_some() {
                    return Vec::new();
                }
                if !update(&mut live) {
                    return Vec::new();
                }
                vec![self.set_phase(DictationPhase::Listening { started_at, ready, live: Some(live), locked })]
            }
            _ => Vec::new(),
        }
    }

    /// A committed sentence: it joins the preview while listening; in `live_inject` it also goes
    /// to the injector at once (also after the stop, while the worker drains the tap — the flush
    /// repeats it and the index keeps it from being pasted twice).
    fn on_segment(&mut self, session: u64, segment: Segment) -> Vec<Effect> {
        if session != self.status.session {
            return Vec::new();
        }
        if self.take.mode == OutputMode::LiveInject
            && !self.take.inject.closing
            && matches!(self.status.phase, DictationPhase::Listening { .. } | DictationPhase::Processing { .. })
        {
            self.take.inject.seen += 1;
            self.queue_sentence(segment.clone());
        }
        let pasted = self.take.inject.pasted;
        self.on_live(session, |live| {
            live.committed.push(segment);
            live.current.clear();
            live.injected = pasted;
            true
        })
    }

    /// The live worker gave up (docs/dictation.md §11 / §12). Whole take: the preview is marked
    /// degraded, nothing else. Streaming modes: before the first sentence the run becomes a whole
    /// take; `live_inject` after a sentence keeps what it pasted and later transcribes only the
    /// remainder. `Done.live_error` carries the reason either way.
    fn on_stream_degraded(&mut self, session: u64, reason: String) -> Vec<Effect> {
        if session != self.status.session {
            return Vec::new();
        }
        tracing::warn!(session, reason = %reason, mode = self.take.mode.as_str(), "live preview degraded; the recording continues without it");
        self.take.worker_pending = false;
        let mut effects = self.on_live(session, |live| {
            live.degraded = Some(reason.clone());
            true
        });
        effects.extend(self.degrade(reason));
        effects
    }

    /// Apply the §12 fallback for the current mode after the stream failed (`reason`).
    fn degrade(&mut self, reason: String) -> Vec<Effect> {
        match self.take.mode {
            OutputMode::WholeTake => Vec::new(),
            OutputMode::StreamingFinal => {
                self.take.live_error = Some(reason);
                self.take.mode = OutputMode::WholeTake;
                self.take.flushed = None;
                // Already stopped and the recording is here: the whole take goes to the transcriber now.
                match self.take.recording.take() {
                    Some(recording) if matches!(self.status.phase, DictationPhase::Processing { .. }) => self.run_whole_take(recording),
                    other => {
                        self.take.recording = other;
                        Vec::new()
                    }
                }
            }
            OutputMode::LiveInject => {
                self.take.live_error = Some(reason);
                if self.take.inject.segments.is_empty() {
                    // Nothing pasted yet: a plain whole take from here.
                    self.take.mode = OutputMode::WholeTake;
                    return match self.take.recording.take() {
                        Some(recording) if matches!(self.status.phase, DictationPhase::Processing { .. }) => self.run_whole_take(recording),
                        other => {
                            self.take.recording = other;
                            Vec::new()
                        }
                    };
                }
                self.try_close_live_inject()
            }
        }
    }

    /// The live worker flushed. Whole take: while the run is still `Processing`, its complete text
    /// (committed sentences + tail) replaces the preview taken at stop time, which may have missed
    /// the last partial; a failed flush keeps the earlier preview. Streaming modes: the flush is
    /// the text (or, failing, the reason to fall back).
    fn on_stream_finished(&mut self, session: u64, result: Result<StreamFinal, DictationError>) -> Vec<Effect> {
        if session != self.status.session {
            return Vec::new();
        }
        self.take.worker_pending = false;
        let fin = match result {
            Ok(fin) => fin,
            Err(e) => {
                tracing::warn!(session, error = %e, mode = self.take.mode.as_str(), "live preview flush failed");
                return self.degrade(format!("flush: {e}"));
            }
        };
        let final_text = LiveText { committed: fin.committed.clone(), current: fin.tail.clone(), ..LiveText::default() }.preview();
        let mut effects = match &self.status.phase {
            DictationPhase::Processing { stage, started_at, stage_started_at, preview }
                if !final_text.is_empty() && preview.as_deref() != Some(final_text.as_str()) =>
            {
                let (stage, started_at, stage_started_at) = (*stage, *started_at, *stage_started_at);
                vec![self.set_phase(DictationPhase::Processing { stage, started_at, stage_started_at, preview: Some(final_text) })]
            }
            _ => Vec::new(),
        };
        match self.take.mode {
            OutputMode::WholeTake => {}
            OutputMode::StreamingFinal => {
                self.take.flushed = Some(fin);
                effects.extend(self.try_finalize_streaming());
            }
            OutputMode::LiveInject => {
                self.take.flushed = Some(fin);
                effects.extend(self.try_close_live_inject());
            }
        }
        effects
    }

    /// `streaming_final`: once both the recording and the flush are here, the flushed text goes
    /// straight to refine / inject; an empty flush falls back to the whole take.
    fn try_finalize_streaming(&mut self) -> Vec<Effect> {
        if !matches!(self.status.phase, DictationPhase::Processing { .. }) || self.take.recording.is_none() || self.take.flushed.is_none() {
            return Vec::new();
        }
        let Some(fin) = self.take.flushed.take() else { return Vec::new() };
        let raw_text = LiveText { committed: fin.committed.clone(), current: fin.tail.clone(), ..LiveText::default() }.preview();
        if raw_text.is_empty() {
            return self.degrade("实时识别未得到文本".to_owned());
        }
        let mut segments = fin.committed;
        let tail = fin.tail.trim();
        if !tail.is_empty() {
            let start_ms = segments.last().map_or(0, |s| s.end_ms);
            segments.push(Segment { text: tail.to_owned(), start_ms, end_ms: self.recording_ms.max(start_ms) });
        }
        self.take.inject.segments = segments;
        self.take.asr_ms = self.take.finalize_ms();
        self.take.recording = None;
        let refine = self.take_refine_enabled();
        let job = PipelineJob {
            session: self.status.session,
            input: PipelineInput::Text { raw_text, asr_ms: self.take.asr_ms },
            language: self.take_language(),
            transcriber: self.transcriber.clone(),
            refiner: if refine { self.refiner.clone() } else { None },
            refine_requested: refine,
            hints: self.refine_hints(),
            injector: self.injector.clone(),
            vocabulary: self.take.vocabulary.clone(),
            script: self.take_script(),
            tx: self.internal.clone(),
        };
        self.abort_pipeline();
        self.pipeline = Some(tokio::spawn(run_pipeline(job)));
        Vec::new()
    }

    /// `live_inject`: once the recording is here and the worker has reported, queue the last piece
    /// — the flushed tail, or (after a degradation) the transcribed remainder — and close when the
    /// injector has caught up.
    fn try_close_live_inject(&mut self) -> Vec<Effect> {
        if !matches!(self.status.phase, DictationPhase::Processing { .. }) || self.take.inject.closing || self.take.recording.is_none() {
            return Vec::new();
        }
        if self.take.worker_pending {
            return Vec::new();
        }
        let duration_ms = self.recording_ms;
        if let Some(fin) = self.take.flushed.take() {
            // Sentences the flush knows and we never saw as `Segment` (should not happen; cheap to honour).
            let seen = self.take.inject.seen;
            for segment in fin.committed.into_iter().skip(seen) {
                self.take.inject.seen += 1;
                self.queue_sentence(segment);
            }
            let tail = fin.tail.trim();
            if !tail.is_empty() {
                let start_ms = self.take.inject.last_end_ms;
                self.queue_sentence(Segment { text: tail.to_owned(), start_ms, end_ms: duration_ms.max(start_ms) });
            }
            if self.take.inject.segments.is_empty() {
                // The stream produced no text at all: the whole take is the safety net.
                return self.degrade("实时识别未得到文本".to_owned());
            }
            return self.close_live_inject();
        }
        // Degraded after at least one sentence: transcribe what came after the last one.
        let Some(recording) = self.take.recording.as_ref() else { return Vec::new() };
        let remainder = recording.slice_from_ms(self.take.inject.last_end_ms);
        if remainder.duration_ms < u64::try_from(MIN_RECORDING.as_millis()).unwrap_or(u64::MAX) || wav::is_silent(&remainder.wav) {
            tracing::info!(remainder_ms = remainder.duration_ms, "nothing after the last pasted sentence; closing");
            return self.close_live_inject();
        }
        let effects = self.stage(ProcessingStage::Transcribing);
        let (session, language, transcriber, tx) = (self.status.session, self.take_language(), self.transcriber.clone(), self.internal.clone());
        let (vocabulary, script) = (self.take.vocabulary.clone(), self.take_script());
        self.abort_pipeline();
        self.pipeline = Some(tokio::spawn(async move {
            let result = transcriber
                .transcribe(&remainder.wav, language.as_deref(), vocabulary.glossary())
                .await
                .map(|t| Transcript { text: normalized(script, &t.text), ..t });
            let _ = tx.send(Internal::Remainder { session, result }).await;
        }));
        effects
    }

    /// The remainder's transcript (`live_inject` after a degradation): queued as the last piece.
    fn on_remainder(&mut self, session: u64, result: Result<Transcript, DictationError>) -> Vec<Effect> {
        if session != self.status.session || !matches!(self.status.phase, DictationPhase::Processing { .. }) || self.take.mode != OutputMode::LiveInject {
            return Vec::new();
        }
        self.pipeline = None;
        match result {
            Ok(t) if !t.text.trim().is_empty() => {
                let start_ms = self.take.inject.last_end_ms;
                self.queue_sentence(Segment { text: t.text.trim().to_owned(), start_ms, end_ms: self.recording_ms.max(start_ms) });
            }
            Ok(_) => tracing::info!(session, "the remainder transcribed to nothing"),
            Err(e) => {
                tracing::warn!(session, error = %e, "the remainder's transcription failed; closing with what was pasted");
                let reason = self.take.live_error.take().unwrap_or_default();
                self.take.live_error = Some(format!("{reason}; 补齐失败：{e}"));
            }
        }
        self.close_live_inject()
    }

    /// The last piece is queued: `Inserting` until the injector has taken everything, then `Done`.
    fn close_live_inject(&mut self) -> Vec<Effect> {
        self.take.inject.closing = true;
        self.take.asr_ms = self.take.finalize_ms();
        let mut effects = self.stage(ProcessingStage::Inserting);
        effects.extend(self.drain_or_finish_live_inject());
        effects
    }

    /// Add one sentence to the run's text and hand it to the injector (or queue it behind the one
    /// in flight; or accumulate it once a paste fell back to the clipboard). The sentence goes
    /// through the dictionary and the rules first (docs/dictation.md §16.3); one they empty is
    /// recorded but not pasted.
    fn queue_sentence(&mut self, segment: Segment) {
        let raw = segment.text.trim().to_owned();
        if raw.is_empty() {
            return;
        }
        let vocabulary = self.take.vocabulary.clone();
        let (processed, hits) = process_final_text(&vocabulary, &raw);
        let li = &mut self.take.inject;
        li.hits.merge(hits);
        join_text(&mut li.raw_text, &raw);
        li.last_end_ms = li.last_end_ms.max(segment.end_ms);
        li.segments.push(segment);
        let sentence = processed.trim();
        if sentence.is_empty() {
            tracing::info!(session = self.status.session, "the vocabulary emptied a sentence; nothing to paste");
            return;
        }
        let piece = Piece { idx: li.next_idx, text: format!("{sentence}{}", inject_separator(sentence)), chars: sentence.chars().count() };
        li.next_idx += 1;
        join_text(&mut li.text, sentence);
        match &mut li.rest {
            Some(rest) => rest.push_str(&piece.text),
            None if li.in_flight.is_none() => self.dispatch(piece),
            None => li.queue.push_back(piece),
        }
    }

    /// Hand `piece` to the injector on a blocking thread; [`Internal::LiveInjected`] follows.
    fn dispatch(&mut self, piece: Piece) {
        let (session, idx, text) = (self.status.session, piece.idx, piece.text.clone());
        self.take.inject.in_flight = Some(piece);
        let (injector, tx) = (self.injector.clone(), self.internal.clone());
        tokio::spawn(async move {
            let result = match tokio::task::spawn_blocking(move || injector.inject(&text)).await {
                Ok(result) => result,
                Err(e) => Err(DictationError::Inject(format!("injector task failed: {e}"))),
            };
            let _ = tx.send(Internal::LiveInjected { session, idx, result }).await;
        });
    }

    /// One injection came back: count it, or switch to accumulating the rest; then the next one.
    fn on_live_injected(&mut self, session: u64, idx: usize, result: Result<Injection, DictationError>) -> Vec<Effect> {
        if session != self.status.session || self.take.mode != OutputMode::LiveInject || self.status.phase.is_terminal() {
            tracing::debug!(session, idx, "live injection for a finished run dropped");
            return Vec::new();
        }
        let li = &mut self.take.inject;
        let Some(piece) = li.in_flight.take_if(|p| p.idx == idx) else {
            tracing::debug!(session, idx, "live injection does not match the piece in flight");
            return Vec::new();
        };
        if li.writing_rest {
            // The final write of everything not pasted: the run's outcome.
            li.writing_rest = false;
            let text = li.rest.take().unwrap_or_default();
            return self.finish_live_inject(result.map(|i| (i, text.clone())).map_err(|e| (e, text)));
        }
        match result {
            Ok(Injection { via: Via::Paste, .. }) => {
                li.pasted += 1;
                li.pasted_chars += piece.chars;
            }
            Ok(Injection { via: Via::Clipboard, note }) => {
                tracing::info!(session, idx, ?note, "paste fell back to the clipboard; accumulating the rest");
                li.note = note;
                let mut rest = piece.text;
                rest.extend(li.queue.drain(..).map(|p| p.text));
                li.rest = Some(rest);
            }
            Err(e) => {
                tracing::warn!(session, idx, error = %e, "live injection failed; accumulating the rest");
                li.note = Some(e.to_string());
                let mut rest = piece.text;
                rest.extend(li.queue.drain(..).map(|p| p.text));
                li.rest = Some(rest);
            }
        }
        let pasted = li.pasted;
        let mut effects = match &self.status.phase {
            DictationPhase::Listening { started_at, ready, live: Some(live), locked } if live.injected != pasted => {
                let (started_at, ready, locked, mut live) = (*started_at, *ready, *locked, live.clone());
                live.injected = pasted;
                vec![self.set_phase(DictationPhase::Listening { started_at, ready, live: Some(live), locked })]
            }
            _ => Vec::new(),
        };
        let li = &mut self.take.inject;
        if li.rest.is_none()
            && let Some(next) = li.queue.pop_front()
        {
            self.dispatch(next);
        }
        effects.extend(self.drain_or_finish_live_inject());
        effects
    }

    /// Closing and the injector is idle: write the accumulated rest, or finish.
    fn drain_or_finish_live_inject(&mut self) -> Vec<Effect> {
        let li = &mut self.take.inject;
        if !li.closing || li.in_flight.is_some() || !li.queue.is_empty() || li.writing_rest {
            return Vec::new();
        }
        match li.rest.clone() {
            Some(rest) if !rest.trim().is_empty() => {
                li.writing_rest = true;
                let piece = Piece { idx: li.next_idx, text: rest, chars: 0 };
                li.next_idx += 1;
                self.dispatch(piece);
                Vec::new()
            }
            _ => self.finish_live_inject(Ok((Injection { via: Via::Paste, note: None }, String::new()))),
        }
    }

    /// `Done` (or `Failed` when the final write failed) for a `live_inject` run. `last` is the
    /// outcome of the final delivery with the text it carried (the accumulated rest, or nothing).
    fn finish_live_inject(&mut self, last: Result<(Injection, String), (DictationError, String)>) -> Vec<Effect> {
        let text = self.take.inject.text.clone();
        let raw_text = self.take.inject.raw_text.clone();
        let vocabulary = std::mem::take(&mut self.take.inject.hits);
        let fallback_note = self.take.inject.note.take();
        let (injection, undelivered) = match last {
            Ok((Injection { via: Via::Paste, note }, _)) => (Ok(Injection { via: Via::Paste, note }), None),
            Ok((Injection { via: Via::Clipboard, note }, _)) => (Ok(Injection { via: Via::Clipboard, note: note.or(fallback_note) }), None),
            Err((e, rest)) => (Err(e), Some(rest)),
        };
        let outcome = PipelineOutcome {
            raw_text,
            text,
            refined: false,
            asr_ms: self.take.asr_ms,
            refine_ms: None,
            refine_error: None,
            refine_model: None,
            vocabulary,
            injection,
            edit: None,
        };
        self.finish_with(outcome, undelivered)
    }

    /// The voice edit's copy came back (docs/dictation.md §19): nothing selected, a selection past
    /// [`MAX_EDIT_SELECTION_CHARS`] or a failed copy end the take before anything is uploaded (the
    /// microphone is released); a selection is kept until the recording is here too.
    fn on_selection_copied(&mut self, session: u64, result: Result<Option<String>, DictationError>) -> Vec<Effect> {
        let running = matches!(self.status.phase, DictationPhase::Listening { .. } | DictationPhase::Processing { .. });
        if session != self.status.session || !running || self.take.edit.is_none() {
            tracing::debug!(session, "selection for a finished take dropped");
            return Vec::new();
        }
        let text = match result {
            Ok(Some(text)) if !text.trim().is_empty() => text,
            Ok(_) => {
                tracing::info!(session, "nothing is selected; the edit take ends");
                return self.abort_take(&DictationError::NoSelection);
            }
            Err(e) => {
                tracing::warn!(session, error = %e, "the selection could not be read");
                return self.abort_take(&e);
            }
        };
        let chars = text.chars().count();
        if chars > MAX_EDIT_SELECTION_CHARS {
            tracing::info!(session, chars, "the selection is too long for an edit");
            return self.abort_take(&DictationError::SelectionTooLong(chars));
        }
        tracing::info!(session, chars, "selection copied");
        if let Some(edit) = self.take.edit.as_mut() {
            edit.selection = SelectionState::Ready(text);
        }
        self.try_run_edit()
    }

    /// End the current take with `error` before it produced anything: the microphone is released
    /// (a stop already in flight drops its recording when it lands), nothing was uploaded.
    fn abort_take(&mut self, error: &DictationError) -> Vec<Effect> {
        if matches!(self.status.phase, DictationPhase::Listening { .. }) {
            let mut mic = self.mic.lock();
            match std::mem::replace(&mut *mic, Mic::Closed) {
                Mic::Open(capture) => release(capture),
                Mic::Opening { .. } => *mic = Mic::Opening { then: Some(Then::Cancel) },
                Mic::Closed | Mic::Stopping => {}
            }
        }
        self.abort_pipeline();
        self.fail(error, None)
    }

    /// A voice edit whose recording and selection are both here (in either order): the instruction
    /// goes through recognition, the script and the dictionary, the refiner rewrites the selection,
    /// the injector pastes the result ([`run_edit`], docs/dictation.md §19).
    fn try_run_edit(&mut self) -> Vec<Effect> {
        if !matches!(self.status.phase, DictationPhase::Processing { .. }) || self.take.recording.is_none() {
            return Vec::new();
        }
        let Some(EditTake { selection: SelectionState::Ready(selection), .. }) = &self.take.edit else { return Vec::new() };
        let selection = selection.clone();
        let Some(refiner) = self.refiner.clone() else {
            // The key was removed while the take ran.
            return self.abort_take(&DictationError::EditUnavailable(EDIT_NEEDS_REFINE.to_owned()));
        };
        let Some(recording) = self.take.recording.take() else { return Vec::new() };
        let effects = self.stage(ProcessingStage::Transcribing);
        let job = EditJob {
            session: self.status.session,
            wav: recording.wav,
            selection,
            language: self.take_language(),
            transcriber: self.transcriber.clone(),
            refiner,
            hints: self.refine_hints(),
            injector: self.injector.clone(),
            vocabulary: self.take.vocabulary.clone(),
            script: self.take_script(),
            tx: self.internal.clone(),
        };
        self.abort_pipeline();
        self.pipeline = Some(tokio::spawn(run_edit(job)));
        effects
    }

    fn on_capture_started(&mut self, session: u64, result: Result<Box<dyn Capture>, DictationError>) -> Vec<Effect> {
        if session != self.status.session {
            // Opened for a session that is over: release it, nothing to report.
            if let Ok(capture) = result {
                release(capture);
            }
            return Vec::new();
        }
        let mut mic = self.mic.lock();
        let then = match &*mic {
            Mic::Opening { then } => *then,
            // Not opening any more (cancelled and restarted quickly, or a stale duplicate).
            _ => {
                drop(mic);
                if let Ok(capture) = result {
                    release(capture);
                }
                return Vec::new();
            }
        };
        match result {
            Err(e) => {
                *mic = Mic::Closed;
                drop(mic);
                tracing::warn!(error = %e, "capture did not start");
                match self.status.phase {
                    // Listening (with or without a stop already requested): the run failed.
                    DictationPhase::Listening { .. } | DictationPhase::Processing { .. } => self.fail(&e, None),
                    _ => Vec::new(),
                }
            }
            Ok(mut capture) => match then {
                None => {
                    let (mut worker, mut no_tap) = (None, false);
                    if self.live_enabled() {
                        match (capture.live_pcm(), self.streaming.clone()) {
                            (Some(pcm), Some(streaming)) => worker = Some((pcm, streaming)),
                            (None, _) => no_tap = true,
                            (_, None) => {}
                        }
                    }
                    *mic = Mic::Open(capture);
                    drop(mic);
                    if let Some((pcm, streaming)) = worker {
                        self.spawn_live(session, pcm, streaming);
                    }
                    if no_tap {
                        tracing::debug!(session, "capture has no live tap; no preview this run");
                        return self.degrade("capture has no live tap".to_owned());
                    }
                    Vec::new()
                }
                Some(Then::Stop) => {
                    *mic = Mic::Stopping;
                    drop(mic);
                    self.begin_stop(capture);
                    // No worker was ever started for this take: nothing will flush.
                    self.degrade("the take ended while the device was opening".to_owned())
                }
                Some(Then::Cancel) => {
                    *mic = Mic::Closed;
                    drop(mic);
                    release(capture);
                    Vec::new()
                }
            },
        }
    }

    fn on_stopped(&mut self, session: u64, result: Result<Recording, DictationError>) -> Vec<Effect> {
        if session != self.status.session || !matches!(self.status.phase, DictationPhase::Processing { .. }) {
            tracing::debug!(session, "recording for a finished session dropped");
            return Vec::new();
        }
        *self.mic.lock() = Mic::Closed;
        let recording = match result {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "capture did not stop cleanly");
                return self.fail(&e, None);
            }
        };
        // `live_inject` with sentences already pasted cannot be an empty take.
        let pasted_something = self.take.mode == OutputMode::LiveInject && !self.take.inject.segments.is_empty();
        if !pasted_something && (recording.duration_ms < u64::try_from(MIN_RECORDING.as_millis()).unwrap_or(u64::MAX) || wav::is_silent(&recording.wav)) {
            tracing::info!(duration_ms = recording.duration_ms, "recording empty or silent; not uploaded");
            return self.fail(&DictationError::NoSpeech, None);
        }
        self.recording_ms = recording.duration_ms;
        if self.status.kind == TakeKind::Edit {
            // The instruction waits for the selection when the copy is still out (§19).
            self.take.recording = Some(recording);
            return self.try_run_edit();
        }
        match self.take.mode {
            OutputMode::WholeTake => self.run_whole_take(recording),
            OutputMode::StreamingFinal => {
                self.take.recording = Some(recording);
                self.try_finalize_streaming()
            }
            OutputMode::LiveInject => {
                self.take.recording = Some(recording);
                self.try_close_live_inject()
            }
        }
    }

    /// The whole take goes to the transcriber (docs/dictation.md §2).
    fn run_whole_take(&mut self, recording: Recording) -> Vec<Effect> {
        let effects = self.stage(ProcessingStage::Transcribing);
        let refine = self.take_refine_enabled();
        let job = PipelineJob {
            session: self.status.session,
            input: PipelineInput::Wav(recording.wav),
            language: self.take_language(),
            transcriber: self.transcriber.clone(),
            refiner: if refine { self.refiner.clone() } else { None },
            refine_requested: refine,
            hints: self.refine_hints(),
            injector: self.injector.clone(),
            vocabulary: self.take.vocabulary.clone(),
            script: self.take_script(),
            tx: self.internal.clone(),
        };
        self.abort_pipeline();
        self.pipeline = Some(tokio::spawn(run_pipeline(job)));
        effects
    }

    fn finish(&mut self, outcome: PipelineOutcome) -> Vec<Effect> {
        let text = outcome.text.clone();
        self.finish_with(outcome, Some(text))
    }

    /// Enter `Done` (or `Failed` with `undelivered` when the injection failed) and record the run.
    fn finish_with(&mut self, outcome: PipelineOutcome, undelivered: Option<String>) -> Vec<Effect> {
        let PipelineOutcome { raw_text, text, refined, asr_ms, refine_ms, refine_error, refine_model, vocabulary, injection, edit } = outcome;
        let mode = self.take.mode;
        let segments = if mode.is_streaming() { Some(std::mem::take(&mut self.take.inject.segments)) } else { None };
        let live_error = if self.take.requested.is_streaming() { self.take.live_error.clone() } else { None };
        let mut entry = HistoryEntry {
            id: Uuid::new_v4(),
            at_ms: now_ms(),
            raw_text: raw_text.clone(),
            text: text.clone(),
            refined,
            asr_model: self.asr_model.clone(),
            refine_model: if refined { Some(refine_model.filter(|m| !m.is_empty()).unwrap_or_else(|| self.refine_model.clone())) } else { None },
            duration_ms: self.recording_ms,
            asr_ms,
            refine_ms,
            outcome: Outcome::Failed { reason: String::new() },
            starred: false,
            mode,
            segments: segments.clone(),
            live_error: live_error.clone(),
            vocabulary: vocabulary.into_option(),
            kind: self.status.kind,
            edit: edit.map(|e| *e),
            app: self.take.app.as_ref().map(|a| AppRef { id: a.app_id.clone(), name: a.name.clone() }),
            scene: self.take.scene.as_ref().map(Scene::to_ref),
            origin: None,
        };
        match injection {
            Ok(Injection { via, note }) => {
                entry.outcome = match (via, note) {
                    (Via::Paste, _) => Outcome::Inserted { via: Via::Paste },
                    (Via::Clipboard, Some(reason)) => Outcome::Clipboard { reason },
                    (Via::Clipboard, None) => Outcome::Inserted { via: Via::Clipboard },
                };
                let phase = DictationPhase::Done {
                    chars: text.chars().count(),
                    text,
                    raw_text,
                    via,
                    refined,
                    duration_ms: self.recording_ms,
                    asr_ms,
                    refine_ms,
                    refine_error,
                    mode,
                    segments,
                    live_error,
                };
                let mut effects = self.terminal(phase, DWELL);
                effects.push(Effect::Record(entry));
                effects
            }
            Err(e) => {
                entry.outcome = Outcome::Failed { reason: e.to_string() };
                let mut effects = self.fail(&e, undelivered);
                effects.push(Effect::Record(entry));
                effects
            }
        }
    }

    /// Enter `Failed` with the error's stage code; the dwell is longer when there is text the user
    /// may still want to copy.
    fn fail(&mut self, error: &DictationError, text: Option<String>) -> Vec<Effect> {
        let dwell = if text.is_some() { DWELL_WITH_TEXT } else { DWELL };
        self.terminal(DictationPhase::Failed { code: FailureCode::from(error), message: error.to_string(), text }, dwell)
    }

    /// Enter a terminal phase and arm its dwell timer.
    fn terminal(&mut self, phase: DictationPhase, dwell: Duration) -> Vec<Effect> {
        self.disarm();
        self.arm(dwell, |session| Internal::DwellOver { session });
        vec![self.set_phase(phase)]
    }

    fn set_phase(&mut self, phase: DictationPhase) -> Effect {
        // Phase names only: transcripts never reach the log.
        let name = match &phase {
            DictationPhase::Idle => "idle",
            DictationPhase::Listening { .. } => "listening",
            DictationPhase::Processing { .. } => "processing",
            DictationPhase::Done { .. } => "done",
            DictationPhase::Failed { .. } => "failed",
            DictationPhase::Cancelled { .. } => "cancelled",
        };
        tracing::info!(session = self.status.session, phase = name, kind = self.status.kind.as_str(), "dictation phase");
        if phase == DictationPhase::Idle {
            // The take is over: its context goes with it (the next start probes afresh).
            self.status.context = None;
        }
        self.status.phase = phase;
        Effect::Status(self.status.clone())
    }

    /// Replace the armed timer with one that fires `make(session)` after `after`. The deadline is
    /// fixed here, not when the task first runs, so it measures from the transition itself.
    fn arm(&mut self, after: Duration, make: fn(u64) -> Internal) {
        self.disarm();
        let session = self.status.session;
        let tx = self.internal.clone();
        let deadline = tokio::time::Instant::now() + after;
        self.timer = Some(tokio::spawn(async move {
            tokio::time::sleep_until(deadline).await;
            let _ = tx.send(make(session)).await;
        }));
    }

    fn disarm(&mut self) {
        if let Some(t) = self.timer.take() {
            t.abort();
        }
    }

    fn abort_pipeline(&mut self) {
        if let Some(p) = self.pipeline.take() {
            p.abort();
        }
    }
}

impl Drop for DictationEngine {
    fn drop(&mut self) {
        self.disarm();
        self.abort_pipeline();
        if let Mic::Open(capture) = std::mem::replace(&mut *self.mic.lock(), Mic::Closed)
            && let Err(e) = capture.stop()
        {
            tracing::debug!(error = %e, "capture stop on shutdown failed");
        }
    }
}

struct LiveJob {
    session: u64,
    pcm: Box<dyn LivePcm>,
    streaming: Arc<dyn StreamingTranscriber>,
    language: Option<String>,
    /// The script partials, sentences and the flush are normalised to (docs/dictation.md §17).
    script: ChineseScript,
    tx: mpsc::Sender<Internal>,
}

/// The live decode loop (docs/dictation.md §11), on a blocking thread: open the streaming
/// session, then read the tap in [`LIVE_CHUNK_SAMPLES`] steps, `feed`, `poll` until `Idle`,
/// forwarding partials at most every [`PARTIAL_THROTTLE`] and every endpoint at once. When the
/// tap closes the session is flushed and [`Internal::StreamFinished`] carries the result. Every
/// failure — open, tap overrun, decoder error, a panic in the recogniser — becomes one
/// [`Internal::StreamDegraded`] and ends the worker; the recording never notices.
fn run_live(job: LiveJob) {
    let LiveJob { session, mut pcm, streaming, language, script, tx } = job;
    let degrade = |reason: String| {
        let _ = tx.blocking_send(Internal::StreamDegraded { session, reason });
    };
    let mut stream = match std::panic::catch_unwind(AssertUnwindSafe(|| streaming.open(language.as_deref()))) {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return degrade(format!("open: {e}")),
        Err(_) => return degrade("open: recogniser panicked".to_owned()),
    };
    let mut buf = vec![0.0_f32; LIVE_CHUNK_SAMPLES];
    let mut pending: Vec<f32> = Vec::with_capacity(LIVE_CHUNK_SAMPLES * 2);
    let mut throttle = PartialThrottle::default();
    let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| -> Result<(), String> {
        loop {
            let n = pcm.read(&mut buf);
            pending.extend_from_slice(&buf[..n]);
            if pcm.overrun() {
                return Err("live tap overrun: the decoder fell behind the microphone".to_owned());
            }
            let closed = pcm.is_closed() && n == 0;
            if pending.len() >= LIVE_CHUNK_SAMPLES || (closed && !pending.is_empty()) {
                stream.feed(&pending);
                pending.clear();
                loop {
                    match stream.poll() {
                        StreamEvent::Idle => break,
                        StreamEvent::Error(e) => return Err(e),
                        StreamEvent::Endpoint { text, start_ms, end_ms } => {
                            throttle.reset();
                            let segment = Segment { text: normalized(script, &text), start_ms, end_ms };
                            let _ = tx.blocking_send(Internal::Segment { session, segment });
                        }
                        StreamEvent::Partial { current } => {
                            // The throttle compares the recogniser's text; only what goes out is converted.
                            if throttle.admit(&current, Instant::now()) {
                                let _ = tx.blocking_send(Internal::Partial { session, current: normalized(script, &current) });
                            }
                        }
                    }
                }
            } else if closed {
                return Ok(());
            } else if n == 0 {
                // Nothing buffered yet: a short back-off, not a wait for anything in particular.
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }));
    match outcome {
        Ok(Ok(())) => {}
        Ok(Err(reason)) => return degrade(reason),
        Err(_) => return degrade("decoder panicked".to_owned()),
    }
    let result = match std::panic::catch_unwind(AssertUnwindSafe(move || stream.finish())) {
        Ok(r) => r,
        Err(_) => Err(DictationError::Asr("flush: recogniser panicked".to_owned())),
    };
    let result = result.map(|fin| StreamFinal {
        committed: fin.committed.into_iter().map(|s| Segment { text: normalized(script, &s.text), ..s }).collect(),
        tail: normalized(script, &fin.tail),
    });
    let _ = tx.blocking_send(Internal::StreamFinished { session, result });
}

/// Rate limit for partial results: one every [`PARTIAL_THROTTLE`] at most, only when the text
/// changed; an endpoint resets it so the next sentence's first word shows at once.
#[derive(Debug, Default)]
struct PartialThrottle {
    last_at: Option<Instant>,
    last: String,
}

impl PartialThrottle {
    /// Whether `current` (seen at `now`) should be forwarded.
    fn admit(&mut self, current: &str, now: Instant) -> bool {
        if current == self.last || self.last_at.is_some_and(|t| now.duration_since(t) < PARTIAL_THROTTLE) {
            return false;
        }
        self.last.clear();
        self.last.push_str(current);
        self.last_at = Some(now);
        true
    }

    fn reset(&mut self) {
        self.last_at = None;
        self.last.clear();
    }
}

/// Where the pipeline's raw text comes from.
enum PipelineInput {
    /// The whole take: transcribe it.
    Wav(Vec<u8>),
    /// `streaming_final`: the flushed stream is the text; `asr_ms` is the finalisation time.
    Text { raw_text: String, asr_ms: u64 },
}

struct PipelineJob {
    session: u64,
    input: PipelineInput,
    language: Option<String>,
    transcriber: Arc<dyn Transcriber>,
    refiner: Option<Arc<dyn Refiner>>,
    refine_requested: bool,
    /// What the refiner is told besides the text (glossary, language, style, context; §18.5).
    hints: RefineHints,
    injector: Arc<dyn Injector>,
    vocabulary: Arc<Vocabulary>,
    /// The script the transcript is normalised to before anything else (docs/dictation.md §17).
    script: ChineseScript,
    tx: mpsc::Sender<Internal>,
}

/// Log a vocabulary step that fell back to its input (docs/dictation.md §16.3); the take goes on.
fn note_fallback(step: &Step, what: &str) {
    if let Some(reason) = &step.error {
        tracing::warn!(step = what, %reason, "vocabulary step fell back to the unmodified text");
    }
}

/// The dictionary then the rules on a text nothing else will touch (a `live_inject` sentence).
fn process_final_text(vocabulary: &Vocabulary, text: &str) -> (String, VocabularyHits) {
    let corrected = vocabulary.correct(text);
    note_fallback(&corrected, "dictionary");
    let ruled = vocabulary.apply_rules(&corrected.text);
    note_fallback(&ruled, "rules");
    (ruled.text, VocabularyHits { corrections: corrected.hits, rules: ruled.hits })
}

/// (ASR →) dictionary → (refine) → rules → inject, off the core task (docs/dictation.md §16.3).
/// Every step reports back with the session id.
async fn run_pipeline(job: PipelineJob) {
    let PipelineJob { session, input, language, transcriber, refiner, refine_requested, hints, injector, vocabulary, script, tx } = job;
    let glossary = vocabulary.glossary();
    let (raw_text, asr_ms) = match input {
        // The recogniser's text in the chosen script first (docs/dictation.md §17); a streaming
        // final text (`Text`) was normalised by the live worker already.
        PipelineInput::Wav(wav) => match transcriber.transcribe(&wav, language.as_deref(), glossary).await {
            Ok(t) => (normalized(script, t.text.trim()), t.latency_ms),
            Err(e) => {
                let _ = tx.send(Internal::Finished { session, result: Err(e) }).await;
                return;
            }
        },
        PipelineInput::Text { raw_text, asr_ms } => (raw_text.trim().to_owned(), asr_ms),
    };
    if raw_text.is_empty() {
        let _ = tx.send(Internal::Finished { session, result: Err(DictationError::NoSpeech) }).await;
        return;
    }
    let corrected = vocabulary.correct(&raw_text);
    note_fallback(&corrected, "dictionary");
    let mut text = corrected.text.clone();
    let (mut refined, mut refine_ms, mut refine_error, mut refine_model) = (false, None, None, None);
    if let Some(refiner) = refiner {
        let _ = tx.send(Internal::Stage { session, stage: ProcessingStage::Refining }).await;
        match refiner.refine(&corrected.text, &hints).await {
            Ok(out) => {
                let cleaned = out.text.trim();
                if cleaned.is_empty() {
                    refine_error = Some("润色返回空文本，已使用原文".to_owned());
                } else {
                    text = cleaned.to_owned();
                    refined = true;
                    refine_model = Some(out.model);
                }
                refine_ms = Some(out.latency_ms);
            }
            Err(e) => {
                tracing::warn!(error = %e, "refine failed; injecting the raw transcript");
                refine_error = Some(e.to_string());
            }
        }
    } else if refine_requested {
        refine_error = Some("润色未配置：缺少 API 密钥".to_owned());
    }
    let ruled = vocabulary.apply_rules(&text);
    note_fallback(&ruled, "rules");
    let text = ruled.text;
    let hits = VocabularyHits { corrections: corrected.hits, rules: ruled.hits };
    if text.trim().is_empty() {
        // The dictionary / rules emptied the text (a filler-only take): nothing to insert.
        tracing::info!(session, "the vocabulary emptied the text; nothing to insert");
        let _ = tx.send(Internal::Finished { session, result: Err(DictationError::NoSpeech) }).await;
        return;
    }
    let _ = tx.send(Internal::Stage { session, stage: ProcessingStage::Inserting }).await;
    let to_inject = text.clone();
    let injection = match tokio::task::spawn_blocking(move || injector.inject(&to_inject)).await {
        Ok(result) => result,
        Err(e) => Err(DictationError::Inject(format!("injector task failed: {e}"))),
    };
    let outcome = PipelineOutcome { raw_text, text, refined, asr_ms, refine_ms, refine_error, refine_model, vocabulary: hits, injection, edit: None };
    let _ = tx.send(Internal::Finished { session, result: Ok(outcome) }).await;
}

struct EditJob {
    session: u64,
    /// The spoken instruction.
    wav: Vec<u8>,
    /// The text to rewrite, as copied.
    selection: String,
    language: Option<String>,
    transcriber: Arc<dyn Transcriber>,
    refiner: Arc<dyn Refiner>,
    /// The take's hints (glossary, the app in front as the privacy switches allow; no scene).
    hints: RefineHints,
    injector: Arc<dyn Injector>,
    vocabulary: Arc<Vocabulary>,
    script: ChineseScript,
    tx: mpsc::Sender<Internal>,
}

/// The voice edit pipeline (docs/dictation.md §19), off the core task: recognise the instruction
/// (glossary as the hint), bring it to the configured script (§17), correct it with the dictionary
/// (§16) — then `Refiner::edit(selection, instruction, hints)` and paste the answer over the
/// selection. The replacement rules do not run: they are for dictated text, not for the LLM's
/// rewrite. Any failure before the paste leaves the selection untouched.
async fn run_edit(job: EditJob) {
    let EditJob { session, wav, selection, language, transcriber, refiner, hints, injector, vocabulary, script, tx } = job;
    let finish = |result: Result<PipelineOutcome, DictationError>| {
        let tx = tx.clone();
        async move {
            let _ = tx.send(Internal::Finished { session, result }).await;
        }
    };
    let glossary = vocabulary.glossary();
    let (raw_text, asr_ms) = match transcriber.transcribe(&wav, language.as_deref(), glossary).await {
        Ok(t) => (normalized(script, t.text.trim()), t.latency_ms),
        Err(e) => return finish(Err(e)).await,
    };
    let corrected = vocabulary.correct(&raw_text);
    note_fallback(&corrected, "dictionary");
    let instruction = corrected.text.trim().to_owned();
    if instruction.is_empty() {
        return finish(Err(DictationError::NoSpeech)).await;
    }
    let _ = tx.send(Internal::Stage { session, stage: ProcessingStage::Refining }).await;
    let out = match refiner.edit(&selection, &instruction, &hints).await {
        Ok(out) => out,
        Err(e) => {
            tracing::warn!(session, error = %e, "the edit failed; the selection is left as it was");
            return finish(Err(e)).await;
        }
    };
    if out.text.trim().is_empty() {
        return finish(Err(DictationError::Refine("改写结果为空，选中文本未改动".to_owned()))).await;
    }
    let _ = tx.send(Internal::Stage { session, stage: ProcessingStage::Inserting }).await;
    let text = out.text;
    let to_inject = text.clone();
    let injection = match tokio::task::spawn_blocking(move || injector.inject(&to_inject)).await {
        Ok(result) => result,
        Err(e) => Err(DictationError::Inject(format!("injector task failed: {e}"))),
    };
    let outcome = PipelineOutcome {
        raw_text,
        text,
        refined: true,
        asr_ms,
        refine_ms: Some(out.latency_ms),
        refine_error: None,
        refine_model: Some(out.model),
        // The dictionary corrected the instruction; the rules never run on an edit.
        vocabulary: VocabularyHits { corrections: corrected.hits, rules: Vec::new() },
        injection,
        edit: Some(Box::new(EditRecord { instruction, selection })),
    };
    finish(Ok(outcome)).await;
}

#[cfg(test)]
mod tests {
    use super::super::ports::{MAX_RECORDING, MAX_RECORDING_STREAMING};
    use super::*;
    use crate::dictation::fakes::{
        FAKE_LATENCY_MS, FAKE_REFINE_MODEL, FAKE_STREAMING_MODEL_ID, FAKE_TRANSCRIPT, FakeAudio, FakeInjector, FakeModels, FakeProbe, FakeRefiner,
        FakeStreaming, FakeTranscriber, ports_with,
    };
    use crate::engines::{BuiltIn, EngineSettings, UserSecrets};
    use crate::history::EditRecord;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Rig {
        engine: DictationEngine,
        rx: mpsc::Receiver<Internal>,
        audio: Arc<FakeAudio>,
        transcriber: Arc<FakeTranscriber>,
        refiner: Option<Arc<FakeRefiner>>,
        injector: Arc<FakeInjector>,
        levels: broadcast::Receiver<LevelFrame>,
    }

    /// A build with the built-in recognition service (its model is what history records); the
    /// fakes stand in for the clients, so no request is ever made.
    const TEST_BUILT_IN: BuiltIn = BuiltIn { asr_url: Some("https://asr.test"), ..BuiltIn::EMPTY };

    fn resolved(refine_enabled: bool, language: Option<&str>) -> ResolvedEngines {
        let settings = EngineSettings { refine_enabled, language: language.map(str::to_owned), ..EngineSettings::default() };
        ResolvedEngines::resolve(&settings, &UserSecrets::default(), &TEST_BUILT_IN)
    }

    /// A configuration whose library has the streaming model installed: live preview is ready
    /// (unless `live_preview` is switched off).
    fn resolved_live(live_preview: bool) -> ResolvedEngines {
        let settings = EngineSettings { refine_enabled: false, live_preview, ..EngineSettings::default() };
        let library = FakeModels::new(1).with_installed(FAKE_STREAMING_MODEL_ID).scan();
        ResolvedEngines::resolve_with_models(&settings, &UserSecrets::default(), &TEST_BUILT_IN, &library)
    }

    fn rig(audio: FakeAudio, transcriber: FakeTranscriber, refiner: Option<FakeRefiner>, injector: FakeInjector, refine_enabled: bool) -> Rig {
        let audio = Arc::new(audio);
        let transcriber = Arc::new(transcriber);
        let refiner = refiner.map(Arc::new);
        let injector = Arc::new(injector);
        let (levels_tx, levels) = broadcast::channel(64);
        let (engine, rx) =
            DictationEngine::new(ports_with(audio.clone(), transcriber.clone(), refiner.clone(), injector.clone()), &resolved(refine_enabled, None), levels_tx);
        Rig { engine, rx, audio, transcriber, refiner, injector, levels }
    }

    /// A rig with a streaming recogniser plugged in and live preview configured on (or off).
    fn rig_live(audio: FakeAudio, streaming: Arc<FakeStreaming>, live_preview: bool) -> Rig {
        rig_live_with(audio, FakeTranscriber::ok(FAKE_TRANSCRIPT), streaming, live_preview)
    }

    fn rig_live_with(audio: FakeAudio, transcriber: FakeTranscriber, streaming: Arc<FakeStreaming>, live_preview: bool) -> Rig {
        let audio = Arc::new(audio);
        let transcriber = Arc::new(transcriber);
        let injector = Arc::new(FakeInjector::paste());
        let (levels_tx, levels) = broadcast::channel(64);
        let ports = DictationPorts { streaming: Some(streaming), ..ports_with(audio.clone(), transcriber.clone(), None, injector.clone()) };
        let (engine, rx) = DictationEngine::new(ports, &resolved_live(live_preview), levels_tx);
        Rig { engine, rx, audio, transcriber, refiner: None, injector, levels }
    }

    fn happy() -> Rig {
        rig(FakeAudio::speech(), FakeTranscriber::ok(FAKE_TRANSCRIPT), Some(FakeRefiner::ok("你好，世界。")), FakeInjector::paste(), true)
    }

    fn phase(effects: &[Effect]) -> &DictationPhase {
        match phase_opt(effects) {
            Some(p) => p,
            None => panic!("no status effect in {effects:?}"),
        }
    }

    fn phase_opt(effects: &[Effect]) -> Option<&DictationPhase> {
        effects.iter().rev().find_map(|e| if let Effect::Status(s) = e { Some(&s.phase) } else { None })
    }

    fn record(effects: &[Effect]) -> Option<&HistoryEntry> {
        effects.iter().find_map(|e| if let Effect::Record(r) = e { Some(r) } else { None })
    }

    /// Poll `cond` for up to two real seconds, yielding to the runtime in between.
    async fn wait_until(cond: impl Fn() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !cond() && std::time::Instant::now() < deadline {
            tokio::task::yield_now().await;
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    impl Rig {
        /// Fold the next internal notification.
        async fn next(&mut self) -> Vec<Effect> {
            let ev = tokio::time::timeout(Duration::from_secs(30), self.rx.recv()).await.expect("internal event within 30 s (virtual)").expect("engine alive");
            self.engine.on_internal(ev)
        }

        /// `start()` and fold the two open notifications — `CaptureReady` (the fake device delivers
        /// at once: `Listening.ready` flips) then `CaptureStarted` (silent): the microphone is open
        /// afterwards.
        async fn start_open(&mut self) -> Vec<Effect> {
            let fx = self.engine.start().unwrap();
            assert!(matches!(phase(&fx), DictationPhase::Listening { ready: false, live: None, .. }), "{fx:?}");
            let ready = self.next().await;
            assert!(matches!(phase(&ready), DictationPhase::Listening { ready: true, .. }), "the device's first samples mark it ready: {ready:?}");
            assert!(self.next().await.is_empty(), "a successful open reports no new status");
            fx
        }

        /// Fold notifications until `pred` holds for a reported phase (or a terminal phase arrives);
        /// returns every phase seen, in order.
        async fn phases_until(&mut self, mut pred: impl FnMut(&DictationPhase) -> bool) -> Vec<DictationPhase> {
            let mut seen = Vec::new();
            loop {
                let fx = self.next().await;
                for e in fx {
                    if let Effect::Status(st) = e {
                        let done = pred(&st.phase) || st.phase.is_terminal();
                        seen.push(st.phase);
                        if done {
                            return seen;
                        }
                    }
                }
            }
        }

        /// Fold notifications until a terminal phase (or Idle) is reached; returns every effect.
        async fn run_to_terminal(&mut self) -> Vec<Effect> {
            let mut all = Vec::new();
            loop {
                let fx = self.next().await;
                let done = matches!(phase_opt(&fx), Some(p) if p.is_terminal() || *p == DictationPhase::Idle);
                all.extend(fx);
                if done {
                    return all;
                }
            }
        }

        async fn settle(&mut self) {
            for _ in 0..8 {
                tokio::task::yield_now().await;
            }
        }

        fn nothing_pending(&mut self) -> bool {
            matches!(self.rx.try_recv(), Err(mpsc::error::TryRecvError::Empty))
        }

        /// Background device releases run on the blocking pool (real threads): wait for them.
        async fn wait_stops(&self, n: usize) {
            wait_until(|| self.audio.stops() == n).await;
            assert_eq!(self.audio.stops(), n);
        }

        /// Drain and drop whatever is queued (device releases from discarded captures).
        async fn drain(&mut self) {
            self.settle().await;
            while let Ok(ev) = self.rx.try_recv() {
                self.engine.on_internal(ev);
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn stage_change_restarts_the_stage_clock() {
        // User feedback 2026-09-29: the pill's timer stood at 0.0 s while transcribing and
        // polishing. The first step starts with the run; every new step restarts the step clock.
        let mut r = happy();
        r.engine.start().unwrap();
        r.next().await;
        let fx = r.engine.stop().unwrap();
        let DictationPhase::Processing { started_at, stage_started_at, .. } = phase(&fx).clone() else { panic!("{fx:?}") };
        assert_eq!(stage_started_at, started_at, "the first step starts with the run");
        // Let the first step have begun a second ago, then move on.
        if let DictationPhase::Processing { started_at, stage_started_at, .. } = &mut r.engine.status.phase {
            *started_at -= 1000;
            *stage_started_at -= 1000;
        }
        let fx = r.engine.stage(ProcessingStage::Refining);
        let DictationPhase::Processing { stage, started_at: run, stage_started_at: step, .. } = phase(&fx).clone() else { panic!("{fx:?}") };
        assert_eq!(stage, ProcessingStage::Refining);
        assert_eq!(run, started_at - 1000, "the run keeps its start");
        assert!(step >= started_at, "the new step's clock restarted: {step} < {started_at}");
        assert!(r.engine.stage(ProcessingStage::Refining).is_empty(), "the same step changes nothing");
    }

    #[tokio::test(start_paused = true)]
    async fn happy_path_records_refined_text_and_dwells_then_idles() {
        let mut r = happy();
        assert_eq!(r.engine.status(), &DictationStatus::default());
        assert!(format!("{:?}", r.engine).contains("mic: \"closed\""));
        let fx = r.engine.start().unwrap();
        assert!(matches!(phase(&fx), DictationPhase::Listening { started_at, ready: false, live: None, .. } if *started_at > 0));
        assert_eq!(r.engine.status().session, 1);
        assert!(format!("{:?}", r.engine).contains("opening"));
        let fx = r.next().await;
        assert!(matches!(phase(&fx), DictationPhase::Listening { ready: true, live: None, .. }), "ready: {fx:?}");
        assert!(r.next().await.is_empty(), "device opened; still Listening");
        assert!(format!("{:?}", r.engine).contains("mic: \"open\""));
        assert!(format!("{:?}", r.engine).contains("live: false"));
        assert_eq!(r.audio.starts(), 1);
        let mut frames = 0;
        while r.levels.try_recv().is_ok() {
            frames += 1;
        }
        assert_eq!(frames, 3, "level frames reach the broadcast");

        let fx = r.engine.stop().unwrap();
        assert!(matches!(phase(&fx), DictationPhase::Processing { stage: ProcessingStage::Transcribing, preview: None, .. }));
        assert!(format!("{:?}", r.engine).contains("stopping"));
        assert!(r.next().await.is_empty(), "the recording came back and the pipeline started");
        assert_eq!(r.audio.stops(), 1);
        let fx = r.next().await;
        assert!(matches!(phase(&fx), DictationPhase::Processing { stage: ProcessingStage::Refining, .. }));
        let fx = r.next().await;
        assert!(matches!(phase(&fx), DictationPhase::Processing { stage: ProcessingStage::Inserting, .. }));
        let fx = r.next().await;
        match phase(&fx) {
            DictationPhase::Done { text, raw_text, chars, via, refined, duration_ms, asr_ms, refine_ms, refine_error, mode, segments, live_error } => {
                assert_eq!(text, "你好，世界。");
                assert_eq!(raw_text, FAKE_TRANSCRIPT);
                assert_eq!(*chars, 6);
                assert_eq!(*via, Via::Paste);
                assert!(*refined);
                assert_eq!(*duration_ms, 1500);
                assert_eq!(*asr_ms, FAKE_LATENCY_MS);
                assert_eq!(*refine_ms, Some(FAKE_LATENCY_MS));
                assert!(refine_error.is_none());
                assert_eq!(*mode, OutputMode::WholeTake);
                assert!(segments.is_none() && live_error.is_none(), "a whole take carries no stream data");
            }
            other => panic!("{other:?}"),
        }
        let entry = record(&fx).expect("Done appends to history");
        assert_eq!(entry.mode, OutputMode::WholeTake);
        assert!(entry.segments.is_none() && entry.live_error.is_none());
        assert_eq!(entry.text, "你好，世界。");
        assert_eq!(entry.raw_text, FAKE_TRANSCRIPT);
        assert_eq!(entry.outcome, Outcome::Inserted { via: Via::Paste });
        assert_eq!(entry.refine_model.as_deref(), Some(FAKE_REFINE_MODEL));
        assert_eq!(entry.asr_model, crate::engines::DEFAULT_ASR_MODEL);
        assert!(!entry.starred && entry.refined && entry.at_ms > 0);
        assert_eq!(r.injector.injected(), vec!["你好，世界。".to_owned()]);
        assert_eq!(r.transcriber.languages(), vec![None]);

        // Dwell: nothing before 2.5 s, Idle right after.
        tokio::time::advance(DWELL - Duration::from_millis(1)).await;
        r.settle().await;
        assert!(r.nothing_pending());
        tokio::time::advance(Duration::from_millis(1)).await;
        let fx = r.next().await;
        assert_eq!(phase(&fx), &DictationPhase::Idle);
        assert_eq!(r.engine.status().session, 1, "session is only bumped by start");
    }

    #[tokio::test(start_paused = true)]
    async fn silence_short_and_empty_transcripts_fail_without_uploading() {
        for audio in [FakeAudio::silence(), FakeAudio::short()] {
            let mut r = rig(audio, FakeTranscriber::ok("x"), None, FakeInjector::paste(), false);
            r.start_open().await;
            let fx = r.engine.stop().unwrap();
            assert!(matches!(phase(&fx), DictationPhase::Processing { .. }));
            let fx = r.run_to_terminal().await;
            assert_eq!(phase(&fx), &DictationPhase::Failed { code: FailureCode::NoSpeech, message: "没有听到声音".into(), text: None });
            assert!(record(&fx).is_none());
            assert_eq!(r.transcriber.calls(), 0, "silence is not uploaded");
            tokio::time::advance(DWELL).await;
            assert_eq!(phase(&r.next().await), &DictationPhase::Idle);
        }
        let mut r = rig(FakeAudio::speech(), FakeTranscriber::ok("   "), None, FakeInjector::paste(), false);
        r.start_open().await;
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert_eq!(phase(&fx), &DictationPhase::Failed { code: FailureCode::NoSpeech, message: "没有听到声音".into(), text: None });
        assert_eq!(r.transcriber.calls(), 1);
        assert!(r.injector.injected().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn asr_failure_is_failed_without_text_or_history() {
        let mut r = rig(FakeAudio::speech(), FakeTranscriber::err("401 unauthorized"), Some(FakeRefiner::ok("never")), FakeInjector::paste(), true);
        r.start_open().await;
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert_eq!(phase(&fx), &DictationPhase::Failed { code: FailureCode::Asr, message: "asr: 401 unauthorized".into(), text: None });
        assert!(record(&fx).is_none());
        assert_eq!(r.refiner.as_ref().unwrap().calls(), 0);
        assert!(r.injector.injected().is_empty());
        tokio::time::advance(DWELL).await;
        assert_eq!(phase(&r.next().await), &DictationPhase::Idle);
    }

    #[tokio::test(start_paused = true)]
    async fn refine_failure_does_not_block_and_is_reported() {
        let mut r = rig(FakeAudio::speech(), FakeTranscriber::ok("原文"), Some(FakeRefiner::err("429 rate limited")), FakeInjector::paste(), true);
        r.start_open().await;
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        match phase(&fx) {
            DictationPhase::Done { text, raw_text, refined, refine_ms, refine_error, .. } => {
                assert_eq!(text, "原文");
                assert_eq!(raw_text, "原文");
                assert!(!refined);
                assert_eq!(*refine_ms, None);
                assert_eq!(refine_error.as_deref(), Some("refine: 429 rate limited"));
            }
            other => panic!("{other:?}"),
        }
        let entry = record(&fx).unwrap();
        assert!(!entry.refined && entry.refine_model.is_none() && entry.refine_ms.is_none());
        assert_eq!(r.injector.injected(), vec!["原文".to_owned()]);
    }

    #[tokio::test(start_paused = true)]
    async fn refine_is_skipped_when_disabled_missing_or_empty() {
        let mut r = rig(FakeAudio::speech(), FakeTranscriber::ok("a"), Some(FakeRefiner::ok("b")), FakeInjector::paste(), false);
        r.start_open().await;
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { text, refined: false, refine_error: None, .. } if text == "a"));
        assert_eq!(r.refiner.as_ref().unwrap().calls(), 0);
        let mut r = rig(FakeAudio::speech(), FakeTranscriber::ok("a"), None, FakeInjector::paste(), true);
        r.start_open().await;
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { refined: false, refine_error: Some(e), .. } if e.contains("未配置")));
        let mut r = rig(FakeAudio::speech(), FakeTranscriber::ok("a"), Some(FakeRefiner::ok("  ")), FakeInjector::paste(), true);
        r.start_open().await;
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert!(
            matches!(phase(&fx), DictationPhase::Done { text, refined: false, refine_ms: Some(_), refine_error: Some(e), .. } if text == "a" && e.contains("空文本"))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn clipboard_outcomes_are_recorded_by_reason() {
        let mut r = rig(FakeAudio::speech(), FakeTranscriber::ok("a"), None, FakeInjector::clipboard(Some("no focused window")), false);
        r.start_open().await;
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { via: Via::Clipboard, .. }));
        assert_eq!(record(&fx).unwrap().outcome, Outcome::Clipboard { reason: "no focused window".into() });
        let mut r = rig(FakeAudio::speech(), FakeTranscriber::ok("a"), None, FakeInjector::clipboard(None), false);
        r.start_open().await;
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert_eq!(record(&fx).unwrap().outcome, Outcome::Inserted { via: Via::Clipboard });
    }

    #[tokio::test(start_paused = true)]
    async fn inject_failure_keeps_the_text_records_it_and_dwells_longer() {
        let mut r = rig(FakeAudio::speech(), FakeTranscriber::ok("重要的话"), None, FakeInjector::err("denied"), false);
        r.start_open().await;
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert_eq!(phase(&fx), &DictationPhase::Failed { code: FailureCode::Inject, message: "inject: denied".into(), text: Some("重要的话".into()) });
        assert_eq!(record(&fx).unwrap().outcome, Outcome::Failed { reason: "inject: denied".into() });
        tokio::time::advance(DWELL).await;
        r.settle().await;
        assert!(r.nothing_pending(), "text is still on screen after the short dwell");
        tokio::time::advance(DWELL_WITH_TEXT - DWELL).await;
        assert_eq!(phase(&r.next().await), &DictationPhase::Idle);
    }

    #[tokio::test(start_paused = true)]
    async fn command_guards_busy_idle_and_noop_stop() {
        let mut r = happy();
        assert_eq!(r.engine.stop().unwrap_err(), DictationError::Idle);
        assert_eq!(r.engine.cancel().unwrap_err(), DictationError::Idle);
        r.start_open().await;
        assert_eq!(r.engine.start().unwrap_err(), DictationError::Busy);
        assert_eq!(r.audio.starts(), 1);
        r.engine.stop().unwrap();
        assert_eq!(r.engine.start().unwrap_err(), DictationError::Busy);
        assert!(r.engine.stop().unwrap().is_empty(), "stop while processing is a no-op");
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { .. }));
        assert!(r.engine.stop().unwrap().is_empty(), "stop in a terminal state is a no-op");
        let fx = r.engine.cancel().unwrap();
        assert_eq!(phase(&fx), &DictationPhase::Idle);
        tokio::time::advance(DWELL_WITH_TEXT).await;
        r.settle().await;
        assert!(r.nothing_pending());
    }

    #[tokio::test(start_paused = true)]
    async fn cancel_while_listening_releases_the_device_and_uploads_nothing() {
        let mut r = happy();
        r.start_open().await;
        let fx = r.engine.cancel().unwrap();
        assert_eq!(phase(&fx), &DictationPhase::CANCELLED);
        r.wait_stops(1).await;
        assert_eq!(r.transcriber.calls(), 0);
        assert!(record(&fx).is_none());
        tokio::time::advance(DWELL).await;
        assert_eq!(phase(&r.next().await), &DictationPhase::Idle);
        let mut r = rig(FakeAudio::failing_stop("xrun"), FakeTranscriber::ok("a"), None, FakeInjector::paste(), false);
        r.start_open().await;
        assert_eq!(phase(&r.engine.cancel().unwrap()), &DictationPhase::CANCELLED);
        r.wait_stops(1).await;
    }

    #[tokio::test(start_paused = true)]
    async fn stop_or_cancel_while_the_device_is_still_opening() {
        // Quick tap: release before the device is open → stop once it is, then the (short) run fails.
        let mut r = rig(FakeAudio::short(), FakeTranscriber::ok("a"), None, FakeInjector::paste(), false);
        r.engine.start().unwrap();
        let fx = r.engine.stop().unwrap();
        assert!(matches!(phase(&fx), DictationPhase::Processing { .. }));
        assert!(r.next().await.is_empty(), "CaptureReady once already Processing changes nothing");
        assert!(r.next().await.is_empty(), "CaptureStarted with a pending stop");
        let fx = r.run_to_terminal().await;
        assert_eq!(phase(&fx), &DictationPhase::Failed { code: FailureCode::NoSpeech, message: "没有听到声音".into(), text: None });
        assert_eq!((r.audio.starts(), r.audio.stops()), (1, 1));
        // Cancel before the device is open → released when it arrives.
        let mut r = happy();
        r.engine.start().unwrap();
        assert_eq!(phase(&r.engine.cancel().unwrap()), &DictationPhase::CANCELLED);
        assert!(r.next().await.is_empty(), "CaptureReady after a cancel changes nothing");
        assert!(r.next().await.is_empty());
        r.wait_stops(1).await;
        assert_eq!(r.audio.starts(), 1);
        assert_eq!(r.transcriber.calls(), 0);
        // Open fails after a stop was already requested: the run fails, no upload.
        let mut r = rig(FakeAudio::failing_start("busy"), FakeTranscriber::ok("a"), None, FakeInjector::paste(), false);
        r.engine.start().unwrap();
        r.engine.stop().unwrap();
        let fx = r.next().await;
        assert_eq!(phase(&fx), &DictationPhase::Failed { code: FailureCode::Audio, message: "audio: busy".into(), text: None });
        assert_eq!(r.transcriber.calls(), 0);
        // Open fails after a cancel: already Cancelled, nothing more to say.
        let mut r = rig(FakeAudio::failing_start("busy"), FakeTranscriber::ok("a"), None, FakeInjector::paste(), false);
        r.engine.start().unwrap();
        r.engine.cancel().unwrap();
        assert!(r.next().await.is_empty());
        assert_eq!(r.engine.status().phase, DictationPhase::CANCELLED);
    }

    #[tokio::test(start_paused = true)]
    async fn cancel_while_processing_aborts_the_pipeline_and_never_injects() {
        let mut r = rig(FakeAudio::speech(), FakeTranscriber::slow("late", Duration::from_secs(5)), None, FakeInjector::paste(), false);
        r.start_open().await;
        r.engine.stop().unwrap();
        assert!(r.next().await.is_empty(), "Stopped → pipeline spawned");
        r.settle().await;
        assert_eq!(r.transcriber.calls(), 1, "the upload is in flight");
        let fx = r.engine.cancel().unwrap();
        assert_eq!(phase(&fx), &DictationPhase::CANCELLED);
        tokio::time::advance(Duration::from_secs(10)).await;
        r.settle().await;
        let fx = r.next().await;
        assert_eq!(phase(&fx), &DictationPhase::Idle);
        assert!(r.nothing_pending());
        assert!(r.injector.injected().is_empty(), "a cancelled run must not paste");
        let stale = Internal::Finished {
            session: 1,
            result: Ok(PipelineOutcome {
                raw_text: "x".into(),
                text: "x".into(),
                refined: false,
                asr_ms: 1,
                refine_ms: None,
                refine_error: None,
                refine_model: None,
                vocabulary: VocabularyHits::default(),
                injection: Ok(Injection { via: Via::Paste, note: None }),
                edit: None,
            }),
        };
        assert!(format!("{stale:?}").contains("Finished"));
        assert!(format!("{:?}", Internal::LiveInjected { session: 1, idx: 0, result: Ok(Injection { via: Via::Paste, note: None }) }).contains("idx: 0"));
        assert!(format!("{:?}", Internal::Remainder { session: 1, result: Ok(Transcript { text: "秘密".into(), latency_ms: 1 }) }).contains("Ok(2)"));
        assert!(r.engine.on_internal(Internal::LiveInjected { session: 1, idx: 0, result: Ok(Injection { via: Via::Paste, note: None }) }).is_empty());
        assert!(r.engine.on_internal(Internal::Remainder { session: 1, result: Err(DictationError::Asr("late".into())) }).is_empty());
        assert!(r.engine.on_internal(stale).is_empty());
        assert!(r.engine.on_internal(Internal::Stage { session: 1, stage: ProcessingStage::Refining }).is_empty());
        assert!(r.engine.on_internal(Internal::DwellOver { session: 1 }).is_empty());
        assert!(r.engine.on_internal(Internal::AutoStop { session: 1 }).is_empty());
        assert!(r.engine.on_internal(Internal::Stopped { session: 1, result: Err(DictationError::Audio("late".into())) }).is_empty());
        assert_eq!(r.engine.status().phase, DictationPhase::Idle);
    }

    #[tokio::test(start_paused = true)]
    async fn cancel_while_the_recording_is_finalising_drops_it() {
        let mut r = happy();
        r.start_open().await;
        r.engine.stop().unwrap();
        // Cancel before `Stopped` arrives: the recording is dropped, nothing is uploaded.
        assert_eq!(phase(&r.engine.cancel().unwrap()), &DictationPhase::CANCELLED);
        assert!(r.next().await.is_empty(), "Stopped for a cancelled run");
        r.wait_stops(1).await;
        assert_eq!(r.transcriber.calls(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn late_result_after_a_new_start_belongs_to_the_old_session() {
        let mut r = rig(FakeAudio::speech(), FakeTranscriber::slow("late", Duration::from_secs(5)), None, FakeInjector::paste(), false);
        r.start_open().await;
        r.engine.stop().unwrap();
        assert!(r.next().await.is_empty());
        r.engine.cancel().unwrap();
        let fx = r.engine.start().unwrap();
        assert!(matches!(phase(&fx), DictationPhase::Listening { .. }));
        assert_eq!(r.engine.status().session, 2);
        assert!(matches!(phase(&r.next().await), DictationPhase::Listening { ready: true, .. }), "session 2 ready");
        assert!(r.next().await.is_empty(), "session 2 opened");
        tokio::time::advance(Duration::from_secs(10)).await;
        r.settle().await;
        assert!(r.nothing_pending(), "neither the aborted pipeline nor the old dwell reports");
        assert!(matches!(r.engine.status().phase, DictationPhase::Listening { .. }));
        let stale = Internal::Finished { session: 1, result: Err(DictationError::Asr("old".into())) };
        assert!(r.engine.on_internal(stale).is_empty());
        assert!(matches!(r.engine.status().phase, DictationPhase::Listening { .. }));
        // A capture that opened for an older session is released, not adopted.
        let stray = FakeAudio::speech();
        let capture = stray.start(None, Box::new(|_| {}), Box::new(|| {}), CaptureOptions::default()).unwrap();
        assert!(r.engine.on_internal(Internal::CaptureStarted { session: 1, result: Ok(capture) }).is_empty());
        wait_until(|| stray.stops() == 1).await;
        assert_eq!(stray.stops(), 1);
        // …and so is one that arrives when the mic is no longer opening (duplicate).
        let capture = stray.start(None, Box::new(|_| {}), Box::new(|| {}), CaptureOptions::default()).unwrap();
        assert!(r.engine.on_internal(Internal::CaptureStarted { session: 2, result: Ok(capture) }).is_empty());
        wait_until(|| stray.stops() == 2).await;
        assert_eq!(stray.stops(), 2);
        assert!(matches!(r.engine.status().phase, DictationPhase::Listening { .. }));
    }

    #[tokio::test(start_paused = true)]
    async fn recording_auto_stops_at_the_maximum_length() {
        let mut r = happy();
        r.start_open().await;
        tokio::time::advance(MAX_RECORDING - Duration::from_millis(1)).await;
        r.settle().await;
        assert!(r.nothing_pending());
        tokio::time::advance(Duration::from_millis(1)).await;
        let fx = r.next().await;
        assert!(matches!(phase(&fx), DictationPhase::Processing { stage: ProcessingStage::Transcribing, .. }));
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { .. }));
        assert_eq!(r.audio.stops(), 1);
        assert_eq!(r.transcriber.calls(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_new_start_interrupts_the_dwell() {
        let mut r = happy();
        r.start_open().await;
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { .. }));
        tokio::time::advance(Duration::from_secs(1)).await;
        let fx = r.engine.start().unwrap();
        assert!(matches!(phase(&fx), DictationPhase::Listening { .. }));
        assert_eq!(r.engine.status().session, 2);
        assert!(matches!(phase(&r.next().await), DictationPhase::Listening { ready: true, .. }));
        assert!(r.next().await.is_empty());
        tokio::time::advance(DWELL).await;
        r.settle().await;
        assert!(r.nothing_pending(), "the old dwell timer was disarmed");
        assert!(matches!(r.engine.status().phase, DictationPhase::Listening { .. }));
        // Starting while a previous capture is still open (its stop never returned) releases it.
        let mut r = happy();
        r.start_open().await;
        assert_eq!(phase(&r.engine.cancel().unwrap()), &DictationPhase::CANCELLED);
        r.drain().await;
        r.wait_stops(1).await;
        r.engine.start().unwrap();
        assert!(matches!(phase(&r.next().await), DictationPhase::Listening { ready: true, .. }));
        assert!(r.next().await.is_empty());
        assert_eq!(r.audio.starts(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn capture_failures_become_failed_and_the_release_is_harmless() {
        let mut r = rig(FakeAudio::failing_start("no input device"), FakeTranscriber::ok("a"), None, FakeInjector::paste(), false);
        let fx = r.engine.start().unwrap();
        assert!(matches!(phase(&fx), DictationPhase::Listening { .. }), "optimistic: the pill follows the key");
        let fx = r.next().await;
        assert_eq!(phase(&fx), &DictationPhase::Failed { code: FailureCode::Audio, message: "audio: no input device".into(), text: None });
        assert_eq!(r.engine.status().session, 1);
        assert!(r.engine.stop().unwrap().is_empty(), "hotkey release after a failed press is silent");
        tokio::time::advance(DWELL).await;
        assert_eq!(phase(&r.next().await), &DictationPhase::Idle);
        let mut r = rig(FakeAudio::failing_stop("stream xrun"), FakeTranscriber::ok("a"), None, FakeInjector::paste(), false);
        r.start_open().await;
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert_eq!(phase(&fx), &DictationPhase::Failed { code: FailureCode::Audio, message: "audio: stream xrun".into(), text: None });
        assert_eq!(r.transcriber.calls(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn configure_rebuilds_clients_and_passes_the_language() {
        let calls = Arc::new(AtomicUsize::new(0));
        let transcriber = Arc::new(FakeTranscriber::ok("a"));
        let (levels_tx, levels) = broadcast::channel(8);
        let factory_calls = calls.clone();
        let t = transcriber.clone();
        let ports = DictationPorts {
            audio: Arc::new(FakeAudio::speech()),
            injector: Arc::new(FakeInjector::paste()),
            factory: Arc::new(move |engines: &ResolvedEngines| {
                factory_calls.fetch_add(1, Ordering::SeqCst);
                let refiner: Option<Arc<dyn Refiner>> = engines.refine_enabled.then(|| Arc::new(FakeRefiner::ok("b")) as Arc<dyn Refiner>);
                (t.clone() as Arc<dyn Transcriber>, refiner)
            }),
            models: None,
            streaming: None,
            probe: None,
            service_probe: None,
        };
        let (mut engine, mut rx) = DictationEngine::new(ports, &resolved(false, None), levels_tx);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "clients are built once at construction");
        assert_eq!(transcriber.warms(), vec![None], "the recogniser is warmed at construction (§10.7)");
        engine.configure(&resolved(true, Some("zh")));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(transcriber.warms(), vec![None, Some("zh".to_owned())], "and again, with the new language, after a change");
        drop(levels);
        engine.start().unwrap();
        assert!(!engine.on_internal(rx.recv().await.unwrap()).is_empty(), "ready");
        assert!(engine.on_internal(rx.recv().await.unwrap()).is_empty());
        engine.stop().unwrap();
        let last = loop {
            let ev = rx.recv().await.unwrap();
            let fx = engine.on_internal(ev);
            if phase_opt(&fx).is_some_and(DictationPhase::is_terminal) {
                break fx;
            }
        };
        assert!(matches!(phase(&last), DictationPhase::Done { text, refined: true, .. } if text == "b"), "{last:?}");
        assert_eq!(transcriber.languages(), vec![Some("zh".to_owned())]);
        assert_eq!(transcriber.warms().len(), 2, "a take does not warm: it loads what it needs itself");
        assert_eq!(record(&last).unwrap().refine_model.as_deref(), Some(FAKE_REFINE_MODEL));
        engine.cancel().unwrap();
        assert!(matches!(engine.status().phase, DictationPhase::Idle));
        // Dropping the engine with an open capture stops it.
        engine.start().unwrap();
        assert!(!engine.on_internal(rx.recv().await.unwrap()).is_empty(), "ready");
        assert!(engine.on_internal(rx.recv().await.unwrap()).is_empty());
        drop(engine);
    }

    /// `CaptureReady` (docs/dictation.md §11): the device's first samples flip `Listening.ready`
    /// and re-take `started_at`; a device that never delivers leaves `ready = false`; a late or
    /// stale `CaptureReady` changes nothing.
    #[tokio::test(start_paused = true)]
    async fn capture_ready_marks_listening_once_and_ignores_stale_or_late_marks() {
        let mut r = happy();
        let fx = r.engine.start().unwrap();
        let DictationPhase::Listening { started_at: at_start, ready: false, live: None, .. } = phase(&fx).clone() else { panic!("{fx:?}") };
        std::thread::sleep(Duration::from_millis(3));
        let fx = r.next().await;
        let DictationPhase::Listening { started_at, ready: true, live: None, .. } = phase(&fx).clone() else { panic!("{fx:?}") };
        assert!(started_at > at_start, "started_at is re-taken when the audio arrives ({started_at} > {at_start})");
        assert!(r.next().await.is_empty());
        assert!(r.engine.on_internal(Internal::CaptureReady { session: 1 }).is_empty(), "already ready: nothing to report");
        assert!(r.engine.on_internal(Internal::CaptureReady { session: 7 }).is_empty(), "stale session");
        r.engine.stop().unwrap();
        assert!(r.engine.on_internal(Internal::CaptureReady { session: 1 }).is_empty(), "late mark while processing");
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { .. }));
        // A device that opens but never delivers: `ready` stays false until the stop.
        let mut r = rig(FakeAudio::speech().never_ready(), FakeTranscriber::ok("a"), None, FakeInjector::paste(), false);
        let fx = r.engine.start().unwrap();
        assert!(matches!(phase(&fx), DictationPhase::Listening { ready: false, .. }));
        assert!(r.next().await.is_empty(), "CaptureStarted only");
        assert!(matches!(r.engine.status().phase, DictationPhase::Listening { ready: false, .. }));
        assert!(format!("{:?}", Internal::CaptureReady { session: 1 }).contains("CaptureReady"));
        r.engine.cancel().unwrap();
        r.wait_stops(1).await;
    }

    /// The live preview (docs/dictation.md §11) end to end on the fakes: the tap is requested,
    /// the streaming session opened and warmed, partials land in `Listening.live` (throttled: the
    /// scripted words arrive within one throttle window, so not every one is shown), an endpoint
    /// commits a segment and clears `current`, stop carries `committed + current` into
    /// `Processing.preview`, the flushed result refines it, and the final text still comes from
    /// the whole-take transcriber.
    #[tokio::test(start_paused = true)]
    async fn live_preview_streams_partials_and_carries_the_preview_into_processing() {
        let streaming = Arc::new(FakeStreaming::script());
        // A slow (virtual-time) transcriber keeps the pipeline open until the flush has landed.
        let mut r = rig_live_with(FakeAudio::speech(), FakeTranscriber::slow(FAKE_TRANSCRIPT, Duration::from_secs(5)), streaming.clone(), true);
        assert!(r.engine.live_enabled());
        assert_eq!(streaming.warms(), 1, "warmed at construction because live preview is ready");
        assert!(format!("{:?}", r.engine).contains("live: true"));
        r.start_open().await;
        assert_eq!(streaming.warms(), 2, "warmed again on the hotkey press");
        assert_eq!(r.audio.live_requests(), 1, "the capture was asked for a tap");
        // Fold live notifications until the second sentence shows.
        let phases = r.phases_until(|p| matches!(p, DictationPhase::Listening { live: Some(l), .. } if l.current == "今天")).await;
        let lives: Vec<LiveText> =
            phases.iter().filter_map(|p| if let DictationPhase::Listening { live: Some(l), .. } = p { Some(l.clone()) } else { None }).collect();
        assert!(!lives.is_empty(), "{phases:?}");
        assert_eq!(lives[0].current, "你好", "the first word shows at once: {lives:?}");
        assert!(lives.iter().all(|l| l.degraded.is_none()));
        let committed: Vec<&LiveText> = lives.iter().filter(|l| !l.committed.is_empty()).collect();
        assert_eq!(committed[0].committed, vec![Segment { text: "你好，世界。".into(), start_ms: 0, end_ms: 400 }]);
        assert_eq!(committed[0].current, "", "an endpoint clears the current sentence");
        assert!(
            lives.len() < FAKE_LIVE_WORDS_PARTIALS,
            "at least one partial was throttled: {} updates for {} partials",
            lives.len(),
            FAKE_LIVE_WORDS_PARTIALS
        );
        assert!(phases.iter().all(|p| matches!(p, DictationPhase::Listening { ready: true, .. })));
        assert_eq!(streaming.opens(), 1);
        // Stop: the preview travels into Processing; the worker flushes and refines it.
        let fx = r.engine.stop().unwrap();
        assert!(
            matches!(phase(&fx), DictationPhase::Processing { stage: ProcessingStage::Transcribing, preview: Some(p), .. } if p == "你好，世界。今天"),
            "{fx:?}"
        );
        let phases = r.phases_until(|p| matches!(p, DictationPhase::Processing { preview: Some(p), .. } if p == "你好，世界。今天天气")).await;
        assert!(
            phases.iter().any(|p| matches!(p, DictationPhase::Processing { preview: Some(p), .. } if p == "你好，世界。今天天气")),
            "the flushed tail refines the preview: {phases:?}"
        );
        assert_eq!(streaming.finishes(), 1);
        tokio::time::advance(Duration::from_secs(5)).await;
        let fx = r.run_to_terminal().await;
        match phase(&fx) {
            DictationPhase::Done { text, raw_text, .. } => {
                assert_eq!(text, FAKE_TRANSCRIPT, "the final text is the whole take's, not the preview");
                assert_eq!(raw_text, FAKE_TRANSCRIPT);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(r.injector.injected(), vec![FAKE_TRANSCRIPT.to_owned()]);
        assert_eq!(r.transcriber.calls(), 1);
        // The stages keep the preview.
        let entry = record(&fx).unwrap();
        assert_eq!(entry.text, FAKE_TRANSCRIPT);
    }

    /// Partials the script produces in the default session (before and after the endpoint).
    const FAKE_LIVE_WORDS_PARTIALS: usize = 5;

    /// The throttle admits one partial per 80 ms and only when the text changed; an endpoint resets
    /// it so the next sentence's first word is not held back.
    #[test]
    fn partial_throttle_admits_one_per_window_and_resets_at_endpoints() {
        let mut t = PartialThrottle::default();
        let t0 = Instant::now();
        assert!(t.admit("你", t0));
        assert!(!t.admit("你好", t0 + Duration::from_millis(40)), "inside the window");
        assert!(!t.admit("你", t0 + Duration::from_millis(200)), "unchanged text is never re-sent");
        assert!(t.admit("你好，", t0 + PARTIAL_THROTTLE), "exactly one window later");
        assert!(!t.admit("你好，世", t0 + PARTIAL_THROTTLE + Duration::from_millis(79)));
        t.reset();
        assert!(t.admit("今", t0 + PARTIAL_THROTTLE + Duration::from_millis(80)), "an endpoint resets the window");
        assert!(t.admit("", t0 + Duration::from_secs(10)), "clearing the sentence is a change");
        t.reset();
        assert!(!t.admit("", t0 + Duration::from_secs(20)), "an empty partial right after a reset is not news");
        assert!(format!("{t:?}").contains("PartialThrottle"));
    }

    /// Every failure on the live path only degrades the preview (docs/dictation.md §11): the
    /// streaming model failing to open, the decoder erroring mid-sentence, a tap overrun. The
    /// recording, the pipeline and the final text are untouched.
    #[tokio::test(start_paused = true)]
    async fn streaming_failures_degrade_the_preview_and_leave_the_pipeline_alone() {
        // Open fails: one degraded `live`, no partials, the run completes normally.
        let streaming = Arc::new(FakeStreaming::failing_open("模型未下载"));
        let mut r = rig_live(FakeAudio::speech(), streaming.clone(), true);
        r.start_open().await;
        let phases = r.phases_until(|p| matches!(p, DictationPhase::Listening { live: Some(l), .. } if l.degraded.is_some())).await;
        let DictationPhase::Listening { live: Some(live), .. } = phases.last().unwrap() else { panic!("{phases:?}") };
        assert_eq!(live.degraded.as_deref(), Some("open: asr: 模型未下载"));
        assert!(live.committed.is_empty() && live.current.is_empty());
        assert_eq!(streaming.opens(), 1);
        assert!(r.engine.on_internal(Internal::Partial { session: 1, current: "late".into() }).is_empty(), "a degraded preview ignores partials");
        let fx = r.engine.stop().unwrap();
        assert!(matches!(phase(&fx), DictationPhase::Processing { preview: None, .. }), "nothing to preview: {fx:?}");
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { text, .. } if text == FAKE_TRANSCRIPT));
        assert_eq!(streaming.finishes(), 0);

        // Decoder error after the first word: the word stays, `degraded` is set, stop previews it.
        let streaming = Arc::new(FakeStreaming::erroring_after(1));
        let mut r = rig_live(FakeAudio::speech(), streaming.clone(), true);
        r.start_open().await;
        let phases = r.phases_until(|p| matches!(p, DictationPhase::Listening { live: Some(l), .. } if l.degraded.is_some())).await;
        let DictationPhase::Listening { live: Some(live), .. } = phases.last().unwrap() else { panic!("{phases:?}") };
        assert_eq!(live.current, "你好");
        assert_eq!(live.degraded.as_deref(), Some("fake decoder failed"));
        let fx = r.engine.stop().unwrap();
        assert!(matches!(phase(&fx), DictationPhase::Processing { preview: Some(p), .. } if p == "你好"), "{fx:?}");
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { text, .. } if text == FAKE_TRANSCRIPT));
        assert_eq!(streaming.finishes(), 0, "an errored session is not flushed");

        // Tap overrun: reported as degraded before any word.
        let streaming = Arc::new(FakeStreaming::script());
        let mut r = rig_live(FakeAudio::speech().overrunning(), streaming.clone(), true);
        r.start_open().await;
        let phases = r.phases_until(|p| matches!(p, DictationPhase::Listening { live: Some(l), .. } if l.degraded.is_some())).await;
        let DictationPhase::Listening { live: Some(live), .. } = phases.last().unwrap() else { panic!("{phases:?}") };
        assert!(live.degraded.as_deref().unwrap().contains("overrun"), "{live:?}");
        r.engine.cancel().unwrap();
        r.wait_stops(1).await;

        // A failed flush keeps the stop-time preview; a stale flush is dropped.
        let mut r = happy();
        r.start_open().await;
        r.engine.stop().unwrap();
        assert!(r.engine.on_internal(Internal::StreamFinished { session: 1, result: Err(DictationError::Asr("flush".into())) }).is_empty());
        assert!(r.engine.on_internal(Internal::StreamFinished { session: 9, result: Ok(StreamFinal::default()) }).is_empty());
        let fin = StreamFinal { committed: vec![Segment { text: "迟到的".into(), start_ms: 0, end_ms: 1 }], tail: String::new() };
        let fx = r.engine.on_internal(Internal::StreamFinished { session: 1, result: Ok(fin.clone()) });
        assert!(matches!(phase(&fx), DictationPhase::Processing { preview: Some(p), .. } if p == "迟到的"), "a good flush fills an empty preview: {fx:?}");
        assert!(r.engine.on_internal(Internal::StreamFinished { session: 1, result: Ok(fin) }).is_empty(), "same text: no update");
        assert!(r.engine.on_internal(Internal::StreamDegraded { session: 1, reason: "late".into() }).is_empty(), "degraded while processing: ignored");
        assert!(r.engine.on_internal(Internal::Segment { session: 1, segment: Segment { text: "x".into(), start_ms: 0, end_ms: 1 } }).is_empty());
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { .. }));
        assert!(format!("{:?}", Internal::StreamDegraded { session: 1, reason: "r".into() }).contains("StreamDegraded"));
        assert!(format!("{:?}", Internal::Partial { session: 1, current: "秘密".into() }).contains("chars: 2"));
        assert!(!format!("{:?}", Internal::Partial { session: 1, current: "秘密".into() }).contains("秘密"), "preview text stays out of the log");
        assert!(format!("{:?}", Internal::Segment { session: 1, segment: Segment { text: "s".into(), start_ms: 0, end_ms: 5 } }).contains("end_ms: 5"));
        assert!(format!("{:?}", Internal::StreamFinished { session: 1, result: Ok(StreamFinal::default()) }).contains("segments: Ok(0)"));
    }

    /// Live preview is gated twice: by the configuration (`live_preview_ready`) and by the port
    /// being there. Either missing → no tap, no session, no `live`; switching the setting on via
    /// `configure` enables the next run.
    #[tokio::test(start_paused = true)]
    async fn live_preview_is_off_without_readiness_or_a_streaming_port() {
        let streaming = Arc::new(FakeStreaming::script());
        let mut r = rig_live(FakeAudio::speech(), streaming.clone(), false);
        assert!(!r.engine.live_enabled());
        assert_eq!(streaming.warms(), 0, "not warmed while off");
        r.start_open().await;
        assert_eq!(r.audio.live_requests(), 0);
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { .. }));
        assert_eq!(streaming.opens(), 0);
        // Switch it on: the next run previews.
        r.engine.configure(&resolved_live(true));
        assert!(r.engine.live_enabled());
        assert_eq!(streaming.warms(), 1);
        tokio::time::advance(DWELL).await;
        assert_eq!(phase(&r.next().await), &DictationPhase::Idle);
        r.start_open().await;
        assert_eq!(r.audio.live_requests(), 1);
        r.phases_until(|p| matches!(p, DictationPhase::Listening { live: Some(_), .. })).await;
        assert_eq!(streaming.opens(), 1);
        r.engine.cancel().unwrap();
        r.wait_stops(2).await;
        // Ready by configuration but no port (the phone): nothing happens either.
        let mut r = happy();
        r.engine.configure(&resolved_live(true));
        assert!(!r.engine.live_enabled());
        r.start_open().await;
        assert_eq!(r.audio.live_requests(), 0);
        r.engine.cancel().unwrap();
        r.wait_stops(1).await;
    }

    /// Cancelling while previewing releases the device, which closes the tap; the worker's late
    /// flush belongs to a finished session and is dropped.
    #[tokio::test(start_paused = true)]
    async fn cancel_while_previewing_closes_the_tap_and_drops_the_late_flush() {
        let streaming = Arc::new(FakeStreaming::script());
        let mut r = rig_live(FakeAudio::speech(), streaming.clone(), true);
        r.start_open().await;
        r.phases_until(|p| matches!(p, DictationPhase::Listening { live: Some(_), .. })).await;
        assert_eq!(phase(&r.engine.cancel().unwrap()), &DictationPhase::CANCELLED);
        r.wait_stops(1).await;
        wait_until(|| streaming.finishes() == 1).await;
        assert_eq!(streaming.finishes(), 1, "the worker flushed when the tap closed");
        r.drain().await;
        assert_eq!(r.engine.status().phase, DictationPhase::CANCELLED, "the late flush changed nothing");
        assert_eq!(r.transcriber.calls(), 0);
    }

    // ---------------- output modes (docs/dictation.md §12) ----------------

    /// A configuration with the streaming model installed and `output_mode` set.
    fn resolved_mode(mode: OutputMode, live_preview: bool, refine_enabled: bool) -> ResolvedEngines {
        let settings = EngineSettings { refine_enabled, live_preview, output_mode: mode, ..EngineSettings::default() };
        let library = FakeModels::new(1).with_installed(FAKE_STREAMING_MODEL_ID).scan();
        ResolvedEngines::resolve_with_models(&settings, &UserSecrets::default(), &TEST_BUILT_IN, &library)
    }

    /// A rig on `mode` with every port explicit; live preview on and its model installed.
    fn rig_mode(
        audio: FakeAudio,
        transcriber: FakeTranscriber,
        refiner: Option<FakeRefiner>,
        injector: FakeInjector,
        streaming: Arc<FakeStreaming>,
        mode: OutputMode,
    ) -> Rig {
        let audio = Arc::new(audio);
        let transcriber = Arc::new(transcriber);
        let refiner = refiner.map(Arc::new);
        let injector = Arc::new(injector);
        let (levels_tx, levels) = broadcast::channel(64);
        let ports = DictationPorts { streaming: Some(streaming), ..ports_with(audio.clone(), transcriber.clone(), refiner.clone(), injector.clone()) };
        let (engine, rx) = DictationEngine::new(ports, &resolved_mode(mode, true, refiner.is_some()), levels_tx);
        Rig { engine, rx, audio, transcriber, refiner, injector, levels }
    }

    /// Two English sentences and a tail: `Hello world.` (0–200 ms), `How are you.` (200–400 ms), `Fine`.
    fn english_script() -> Arc<FakeStreaming> {
        Arc::new(FakeStreaming::words(["Hello", "Hello world.", "How", "How are you.", "Fine"].map(String::from).to_vec(), 2))
    }

    fn statuses(effects: &[Effect]) -> Vec<&DictationPhase> {
        effects.iter().filter_map(|e| if let Effect::Status(s) = e { Some(&s.phase) } else { None }).collect()
    }

    /// `streaming_final` (docs/dictation.md §12): the flushed stream is the text — committed
    /// sentences joined with the tail by the preview rules — the whole-take transcriber is never
    /// called, `Processing` goes `finalizing → refining → inserting`, refinement still applies,
    /// `asr_ms` is the finalisation time and `Done` / the history carry the mode and the segments.
    #[tokio::test(start_paused = true)]
    async fn streaming_final_skips_the_transcriber_and_joins_committed_and_tail() {
        let streaming = Arc::new(FakeStreaming::script());
        let mut r = rig_mode(
            FakeAudio::speech(),
            FakeTranscriber::ok("整段识别的结果"),
            Some(FakeRefiner::ok("润色后的终稿")),
            FakeInjector::paste(),
            streaming.clone(),
            OutputMode::StreamingFinal,
        );
        assert_eq!(r.engine.effective_output_mode(), (OutputMode::StreamingFinal, None));
        r.start_open().await;
        assert_eq!(r.engine.current_mode(), OutputMode::StreamingFinal);
        assert_eq!(r.audio.max_durations(), vec![MAX_RECORDING_STREAMING], "a streaming take may run ten minutes");
        assert!(format!("{:?}", r.engine).contains("mode: StreamingFinal"));
        r.phases_until(|p| matches!(p, DictationPhase::Listening { live: Some(l), .. } if l.current == "今天")).await;
        let fx = r.engine.stop().unwrap();
        assert!(
            matches!(phase(&fx), DictationPhase::Processing { stage: ProcessingStage::Finalizing, preview: Some(p), .. } if p.starts_with("你好，世界。今天")),
            "the stop-time preview may miss the last (throttled) partial: {fx:?}"
        );
        let fx = r.run_to_terminal().await;
        let stages: Vec<ProcessingStage> =
            statuses(&fx).iter().filter_map(|p| if let DictationPhase::Processing { stage, .. } = p { Some(*stage) } else { None }).collect();
        assert!(!stages.contains(&ProcessingStage::Transcribing), "no transcribing stage: {fx:?}");
        assert!(stages.ends_with(&[ProcessingStage::Refining, ProcessingStage::Inserting]), "{stages:?}");
        assert!(
            statuses(&fx).iter().any(|p| matches!(p, DictationPhase::Processing { preview: Some(p), .. } if p == "你好，世界。今天天气")),
            "the flush refines the preview before the text is final: {fx:?}"
        );
        match phase(&fx) {
            DictationPhase::Done { text, raw_text, refined, mode, segments, live_error, asr_ms, duration_ms, via, refine_ms, .. } => {
                assert_eq!(raw_text, "你好，世界。今天天气", "committed + tail, joined like the preview");
                assert_eq!(text, "润色后的终稿");
                assert!(*refined);
                assert_eq!(*mode, OutputMode::StreamingFinal);
                assert_eq!(
                    segments.as_deref(),
                    Some(
                        &[Segment { text: "你好，世界。".into(), start_ms: 0, end_ms: 400 }, Segment { text: "今天天气".into(), start_ms: 400, end_ms: 1500 }]
                            [..]
                    )
                );
                assert!(live_error.is_none());
                assert_eq!(*duration_ms, 1500);
                assert!(*asr_ms < 10_000, "asr_ms is the (real-time) finalisation duration: {asr_ms}");
                assert_eq!(*via, Via::Paste);
                assert_eq!(*refine_ms, Some(FAKE_LATENCY_MS));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(r.transcriber.calls(), 0, "the whole-take transcriber is skipped");
        assert_eq!(r.refiner.as_ref().unwrap().calls(), 1);
        assert_eq!(r.injector.injected(), vec!["润色后的终稿".to_owned()]);
        assert_eq!(streaming.finishes(), 1);
        let entry = record(&fx).unwrap();
        assert_eq!(entry.mode, OutputMode::StreamingFinal);
        assert_eq!(entry.raw_text, "你好，世界。今天天气");
        assert_eq!(entry.segments.as_ref().map(Vec::len), Some(2));
        assert_eq!(entry.outcome, Outcome::Inserted { via: Via::Paste });
        // Refinement off: the joined text is injected as is.
        let mut r =
            rig_mode(FakeAudio::speech(), FakeTranscriber::ok("x"), None, FakeInjector::paste(), Arc::new(FakeStreaming::script()), OutputMode::StreamingFinal);
        r.start_open().await;
        r.phases_until(|p| matches!(p, DictationPhase::Listening { live: Some(l), .. } if l.current == "今天")).await;
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert!(
            matches!(phase(&fx), DictationPhase::Done { text, refined: false, mode: OutputMode::StreamingFinal, .. } if text == "你好，世界。今天天气"),
            "{fx:?}"
        );
        assert_eq!(r.transcriber.calls(), 0);
    }

    /// `live_inject` (docs/dictation.md §12): every committed sentence goes to the injector with its
    /// separator as soon as the endpoint fires, one at a time — the second sentence waits in the
    /// queue while the first is with the injector — `Listening.live.injected` counts the pasted
    /// ones, the flushed tail is the last piece (`finalizing → inserting`), no refinement, and the
    /// whole-take transcriber is never called.
    #[tokio::test(start_paused = true)]
    async fn live_inject_injects_segments_in_order_and_queues_while_one_is_in_flight() {
        let streaming = english_script();
        let mut r = rig_mode(
            FakeAudio::speech(),
            FakeTranscriber::ok("never"),
            Some(FakeRefiner::ok("never")),
            FakeInjector::paste().gated(),
            streaming.clone(),
            OutputMode::LiveInject,
        );
        r.start_open().await;
        assert_eq!(r.audio.max_durations(), vec![MAX_RECORDING_STREAMING]);
        let phases = r.phases_until(|p| matches!(p, DictationPhase::Listening { live: Some(l), .. } if l.committed.len() == 2)).await;
        let DictationPhase::Listening { live: Some(live), .. } = phases.last().unwrap() else { panic!("{phases:?}") };
        assert_eq!(live.committed.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(), ["Hello world.", "How are you."]);
        assert_eq!(live.injected, 0, "nothing came back from the injector yet");
        wait_until(|| r.injector.waiting() == 1).await;
        r.settle().await;
        assert_eq!(r.injector.waiting(), 1, "the second sentence is queued, not handed over while the first is in flight");
        assert!(r.injector.injected().is_empty(), "the gated injector recorded nothing yet");
        // Let the first one through: it is counted, the second one is dispatched.
        r.injector.release(1);
        let phases = r.phases_until(|p| matches!(p, DictationPhase::Listening { live: Some(l), .. } if l.injected == 1)).await;
        assert!(matches!(phases.last().unwrap(), DictationPhase::Listening { live: Some(l), .. } if l.injected == 1 && l.committed.len() == 2), "{phases:?}");
        assert_eq!(r.injector.injected(), vec!["Hello world. ".to_owned()], "Latin sentence + one space");
        wait_until(|| r.injector.waiting() == 1).await;
        assert_eq!(r.injector.waiting(), 1, "the queued sentence follows at once");
        let fx = r.engine.stop().unwrap();
        assert!(
            matches!(phase(&fx), DictationPhase::Processing { stage: ProcessingStage::Finalizing, preview: Some(p), .. } if p == "Hello world. How are you. Fine"),
            "{fx:?}"
        );
        // Permits for the second sentence and the tail, whenever they arrive.
        r.injector.release(2);
        let fx = r.run_to_terminal().await;
        let stages: Vec<ProcessingStage> =
            statuses(&fx).iter().filter_map(|p| if let DictationPhase::Processing { stage, .. } = p { Some(*stage) } else { None }).collect();
        assert_eq!(stages, vec![ProcessingStage::Inserting], "finalizing → inserting (the tail), never transcribing / refining: {fx:?}");
        match phase(&fx) {
            DictationPhase::Done { text, raw_text, chars, via, refined, mode, segments, live_error, refine_error, duration_ms, .. } => {
                assert_eq!(text, "Hello world. How are you. Fine");
                assert_eq!(raw_text, text);
                assert_eq!(*chars, text.chars().count());
                assert_eq!(*via, Via::Paste);
                assert!(!*refined && refine_error.is_none(), "no refinement in live_inject");
                assert_eq!(*mode, OutputMode::LiveInject);
                assert_eq!(
                    segments.as_deref(),
                    Some(
                        &[
                            Segment { text: "Hello world.".into(), start_ms: 0, end_ms: 200 },
                            Segment { text: "How are you.".into(), start_ms: 200, end_ms: 400 },
                            Segment { text: "Fine".into(), start_ms: 400, end_ms: 1500 },
                        ][..]
                    )
                );
                assert!(live_error.is_none());
                assert_eq!(*duration_ms, 1500);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(r.injector.injected(), ["Hello world. ", "How are you. ", "Fine "].map(String::from), "in order, each with its separator");
        assert_eq!(r.transcriber.calls(), 0);
        assert_eq!(r.refiner.as_ref().unwrap().calls(), 0);
        assert_eq!(streaming.finishes(), 1);
        let entry = record(&fx).unwrap();
        assert_eq!((entry.mode, entry.refined, entry.segments.as_ref().map(Vec::len)), (OutputMode::LiveInject, false, Some(3)));
        assert_eq!(entry.outcome, Outcome::Inserted { via: Via::Paste });
        // CJK sentences take no separator.
        let mut r =
            rig_mode(FakeAudio::speech(), FakeTranscriber::ok("x"), None, FakeInjector::paste(), Arc::new(FakeStreaming::script()), OutputMode::LiveInject);
        r.start_open().await;
        r.phases_until(|p| matches!(p, DictationPhase::Listening { live: Some(l), .. } if l.injected == 1)).await;
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { text, mode: OutputMode::LiveInject, .. } if text == "你好，世界。今天天气"), "{fx:?}");
        assert_eq!(r.injector.injected(), ["你好，世界。", "今天天气"].map(String::from));
    }

    /// `live_inject` when a paste falls back to the clipboard (docs/dictation.md §12): the pasting
    /// stops there, every later sentence (and the tail) is accumulated and written to the clipboard
    /// in one go at the end, and `Done` / the history say so. When that final write does paste
    /// after all, the run is `Done` via paste.
    #[tokio::test(start_paused = true)]
    async fn live_inject_clipboard_fallback_accumulates_the_rest() {
        let mut r = rig_mode(
            FakeAudio::speech(),
            FakeTranscriber::ok("never"),
            None,
            FakeInjector::clipboard(Some("no focused window")),
            english_script(),
            OutputMode::LiveInject,
        );
        r.start_open().await;
        r.phases_until(|p| matches!(p, DictationPhase::Listening { live: Some(l), .. } if l.committed.len() == 2)).await;
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        match phase(&fx) {
            DictationPhase::Done { text, via, mode, segments, .. } => {
                assert_eq!(text, "Hello world. How are you. Fine");
                assert_eq!(*via, Via::Clipboard);
                assert_eq!(*mode, OutputMode::LiveInject);
                assert_eq!(segments.as_ref().map(Vec::len), Some(3));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            r.injector.injected(),
            ["Hello world. ", "Hello world. How are you. Fine "].map(String::from),
            "the first attempt, then everything not pasted in one clipboard write"
        );
        assert_eq!(record(&fx).unwrap().outcome, Outcome::Clipboard { reason: "no focused window".into() });
        assert_eq!(r.transcriber.calls(), 0);
        // Only the first paste fails over; the final write pastes: `Done` via paste, all text delivered.
        let mut r = rig_mode(
            FakeAudio::speech(),
            FakeTranscriber::ok("never"),
            None,
            FakeInjector::paste().clipboard_once(Some("busy")),
            english_script(),
            OutputMode::LiveInject,
        );
        r.start_open().await;
        r.phases_until(|p| matches!(p, DictationPhase::Listening { live: Some(l), .. } if l.committed.len() == 2)).await;
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { via: Via::Paste, text, .. } if text == "Hello world. How are you. Fine"), "{fx:?}");
        assert_eq!(r.injector.injected(), ["Hello world. ", "Hello world. How are you. Fine "].map(String::from));
        assert_eq!(record(&fx).unwrap().outcome, Outcome::Inserted { via: Via::Paste });
        // An injection error works the same way; when the final write fails too the run is `Failed`
        // with the undelivered text, and what was pasted before stays pasted.
        let mut r =
            rig_mode(FakeAudio::speech(), FakeTranscriber::ok("never"), None, FakeInjector::err("denied").gated(), english_script(), OutputMode::LiveInject);
        r.start_open().await;
        r.phases_until(|p| matches!(p, DictationPhase::Listening { live: Some(l), .. } if l.committed.len() == 2)).await;
        r.engine.stop().unwrap();
        r.injector.release(2);
        let fx = r.run_to_terminal().await;
        assert_eq!(
            phase(&fx),
            &DictationPhase::Failed { code: FailureCode::Inject, message: "inject: denied".into(), text: Some("Hello world. How are you. Fine ".into()) }
        );
        assert_eq!(r.injector.injected(), ["Hello world. ", "Hello world. How are you. Fine "].map(String::from));
        assert_eq!(record(&fx).unwrap().outcome, Outcome::Failed { reason: "inject: denied".into() });
        assert_eq!(record(&fx).unwrap().mode, OutputMode::LiveInject);
    }

    /// Cancelling a `live_inject` run does not take back what was pasted (docs/dictation.md §12):
    /// `Cancelled.injected_chars` counts the characters of the sentences that reached the
    /// application; a sentence still queued is never injected and nothing is transcribed.
    #[tokio::test(start_paused = true)]
    async fn live_inject_cancel_keeps_injected_text_and_reports_injected_chars() {
        let mut r = rig_mode(FakeAudio::speech(), FakeTranscriber::ok("never"), None, FakeInjector::paste().gated(), english_script(), OutputMode::LiveInject);
        r.start_open().await;
        r.phases_until(|p| matches!(p, DictationPhase::Listening { live: Some(l), .. } if l.committed.len() == 2)).await;
        wait_until(|| r.injector.waiting() == 1).await;
        r.injector.release(1);
        r.phases_until(|p| matches!(p, DictationPhase::Listening { live: Some(l), .. } if l.injected == 1)).await;
        wait_until(|| r.injector.waiting() == 1).await;
        let fx = r.engine.cancel().unwrap();
        assert_eq!(phase(&fx), &DictationPhase::Cancelled { injected_chars: "Hello world.".chars().count() });
        assert!(record(&fx).is_none(), "a cancelled run is not history");
        // The second sentence, still with the (gated) injector, comes back to a finished run.
        r.injector.release(1);
        r.wait_stops(1).await;
        wait_until(|| r.injector.injected().len() == 2).await;
        r.drain().await;
        assert_eq!(r.engine.status().phase, DictationPhase::Cancelled { injected_chars: 12 }, "the late result changes nothing");
        assert_eq!(r.injector.injected(), ["Hello world. ", "How are you. "].map(String::from), "the piece already handed over cannot be recalled");
        assert_eq!(r.transcriber.calls(), 0);
        tokio::time::advance(DWELL).await;
        assert_eq!(phase(&r.next().await), &DictationPhase::Idle);
        // Cancelling while `Processing` (the tail pending) reports the same count; other modes report 0.
        let mut r = rig_mode(FakeAudio::speech(), FakeTranscriber::ok("never"), None, FakeInjector::paste(), english_script(), OutputMode::LiveInject);
        r.start_open().await;
        r.phases_until(|p| matches!(p, DictationPhase::Listening { live: Some(l), .. } if l.injected == 2)).await;
        r.engine.stop().unwrap();
        assert_eq!(phase(&r.engine.cancel().unwrap()), &DictationPhase::Cancelled { injected_chars: 24 });
        r.wait_stops(1).await;
        let mut r = happy();
        r.start_open().await;
        assert_eq!(phase(&r.engine.cancel().unwrap()), &DictationPhase::CANCELLED);
        r.wait_stops(1).await;
    }

    /// Degradation before the first sentence (docs/dictation.md §12): the run becomes a plain whole
    /// take — same transcriber, same refinement, same injected text as `whole_take` — in both
    /// streaming modes, with `Done.mode = whole_take` and the reason in `Done.live_error`. A flush
    /// that fails after the stop falls back the same way.
    #[tokio::test(start_paused = true)]
    async fn regression_degrading_before_the_first_segment_equals_whole_take() {
        // The reference: the same ports in whole_take mode.
        let mut reference = happy();
        reference.start_open().await;
        reference.engine.stop().unwrap();
        let fx = reference.run_to_terminal().await;
        let DictationPhase::Done { text: ref_text, raw_text: ref_raw, refined: ref_refined, via: ref_via, .. } = phase(&fx).clone() else { panic!("{fx:?}") };
        for mode in [OutputMode::StreamingFinal, OutputMode::LiveInject] {
            for (streaming, reason) in
                [(FakeStreaming::failing_open("模型未下载"), "open: asr: 模型未下载"), (FakeStreaming::erroring_after(1), "fake decoder failed")]
            {
                let mut r = rig_mode(
                    FakeAudio::speech(),
                    FakeTranscriber::ok(FAKE_TRANSCRIPT),
                    Some(FakeRefiner::ok("你好，世界。")),
                    FakeInjector::paste(),
                    Arc::new(streaming),
                    mode,
                );
                r.start_open().await;
                assert_eq!(r.engine.current_mode(), mode);
                let phases = r.phases_until(|p| matches!(p, DictationPhase::Listening { live: Some(l), .. } if l.degraded.is_some())).await;
                assert!(matches!(phases.last().unwrap(), DictationPhase::Listening { live: Some(l), .. } if l.committed.is_empty()), "{phases:?}");
                assert_eq!(r.engine.current_mode(), OutputMode::WholeTake, "{mode:?}: fell back before the first sentence");
                let fx = r.engine.stop().unwrap();
                assert!(
                    matches!(phase(&fx), DictationPhase::Processing { stage: ProcessingStage::Transcribing, .. }),
                    "{mode:?}: a whole take from here: {fx:?}"
                );
                let fx = r.run_to_terminal().await;
                match phase(&fx) {
                    DictationPhase::Done { text, raw_text, refined, via, mode: done_mode, segments, live_error, .. } => {
                        assert_eq!((text, raw_text, refined, via), (&ref_text, &ref_raw, &ref_refined, &ref_via), "{mode:?} / {reason}: same as whole_take");
                        assert_eq!(*done_mode, OutputMode::WholeTake);
                        assert!(segments.is_none());
                        assert_eq!(live_error.as_deref(), Some(reason), "{mode:?}");
                    }
                    other => panic!("{mode:?}: {other:?}"),
                }
                assert_eq!(r.transcriber.calls(), 1, "{mode:?}: the whole take was transcribed once");
                assert_eq!(r.injector.injected(), vec![ref_text.clone()], "{mode:?}: injected once, the whole-take text");
                let entry = record(&fx).unwrap();
                assert_eq!((entry.mode, entry.live_error.as_deref()), (OutputMode::WholeTake, Some(reason)));
            }
        }
        // The flush fails after the stop (`streaming_final`): the recording is still there and goes
        // to the transcriber; the worker's own late flush changes nothing.
        let mut r = rig_mode(
            FakeAudio::speech(),
            FakeTranscriber::ok(FAKE_TRANSCRIPT),
            None,
            FakeInjector::paste(),
            Arc::new(FakeStreaming::script()),
            OutputMode::StreamingFinal,
        );
        r.start_open().await;
        r.phases_until(|p| matches!(p, DictationPhase::Listening { live: Some(l), .. } if l.current == "今天")).await;
        let fx = r.engine.stop().unwrap();
        assert!(matches!(phase(&fx), DictationPhase::Processing { stage: ProcessingStage::Finalizing, .. }));
        assert!(r.engine.on_internal(Internal::StreamFinished { session: 1, result: Err(DictationError::Asr("flush".into())) }).is_empty());
        assert_eq!(r.engine.current_mode(), OutputMode::WholeTake);
        let fx = r.run_to_terminal().await;
        assert!(statuses(&fx).iter().any(|p| matches!(p, DictationPhase::Processing { stage: ProcessingStage::Transcribing, .. })), "{fx:?}");
        assert!(
            matches!(phase(&fx), DictationPhase::Done { text, mode: OutputMode::WholeTake, live_error: Some(e), .. } if text == FAKE_TRANSCRIPT && e == "flush: asr: flush"),
            "{fx:?}"
        );
        assert_eq!(r.transcriber.calls(), 1);
        // A whole take never reports a live error, degraded preview or not.
        let mut r = rig_live(FakeAudio::speech(), Arc::new(FakeStreaming::failing_open("x")), true);
        r.start_open().await;
        r.phases_until(|p| matches!(p, DictationPhase::Listening { live: Some(l), .. } if l.degraded.is_some())).await;
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { mode: OutputMode::WholeTake, live_error: None, segments: None, .. }), "{fx:?}");
    }

    /// `live_inject` degrading after a sentence was pasted (docs/dictation.md §12): the pasted text
    /// stays, only the audio after the last committed `Segment.end_ms` goes to the whole-take
    /// transcriber, its text is pasted as the last piece, nothing is pasted twice, and `Done`
    /// carries both the mode and the reason. A failing remainder closes with what was pasted.
    #[tokio::test(start_paused = true)]
    async fn regression_degrading_after_a_segment_transcribes_only_the_remainder_and_never_reinjects() {
        // The default script commits `你好，世界。` at 400 ms; the decoder fails on the sixth chunk.
        let mut r = rig_mode(
            FakeAudio::speech(),
            FakeTranscriber::ok("剩下的话"),
            Some(FakeRefiner::ok("never")),
            FakeInjector::paste(),
            Arc::new(FakeStreaming::erroring_after(5)),
            OutputMode::LiveInject,
        );
        r.start_open().await;
        let phases = r.phases_until(|p| matches!(p, DictationPhase::Listening { live: Some(l), .. } if l.degraded.is_some())).await;
        let DictationPhase::Listening { live: Some(live), .. } = phases.last().unwrap() else { panic!("{phases:?}") };
        assert_eq!(live.committed, vec![Segment { text: "你好，世界。".into(), start_ms: 0, end_ms: 400 }]);
        assert_eq!(live.degraded.as_deref(), Some("fake decoder failed"));
        assert_eq!(r.engine.current_mode(), OutputMode::LiveInject, "after a sentence the mode is kept");
        wait_until(|| r.injector.injected().len() == 1).await;
        let fx = r.engine.stop().unwrap();
        assert!(matches!(phase(&fx), DictationPhase::Processing { stage: ProcessingStage::Finalizing, .. }), "{fx:?}");
        let fx = r.run_to_terminal().await;
        let stages: Vec<ProcessingStage> =
            statuses(&fx).iter().filter_map(|p| if let DictationPhase::Processing { stage, .. } = p { Some(*stage) } else { None }).collect();
        assert_eq!(stages, vec![ProcessingStage::Transcribing, ProcessingStage::Inserting], "{fx:?}");
        match phase(&fx) {
            DictationPhase::Done { text, via, refined, mode, segments, live_error, .. } => {
                assert_eq!(text, "你好，世界。剩下的话");
                assert_eq!(*via, Via::Paste);
                assert!(!*refined);
                assert_eq!(*mode, OutputMode::LiveInject);
                assert_eq!(
                    segments.as_deref(),
                    Some(
                        &[Segment { text: "你好，世界。".into(), start_ms: 0, end_ms: 400 }, Segment { text: "剩下的话".into(), start_ms: 400, end_ms: 1500 }]
                            [..]
                    )
                );
                assert_eq!(live_error.as_deref(), Some("fake decoder failed"));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(r.transcriber.calls(), 1);
        assert_eq!(r.transcriber.durations_ms(), vec![1100], "only the audio after 400 ms was transcribed");
        assert_eq!(r.injector.injected(), ["你好，世界。", "剩下的话"].map(String::from), "the first sentence is not pasted again");
        assert_eq!(r.refiner.as_ref().unwrap().calls(), 0);
        let entry = record(&fx).unwrap();
        assert_eq!(
            (entry.mode, entry.live_error.as_deref(), entry.segments.as_ref().map(Vec::len)),
            (OutputMode::LiveInject, Some("fake decoder failed"), Some(2))
        );
        // The remainder's transcription fails: the run closes with what was pasted and says why.
        let mut r = rig_mode(
            FakeAudio::speech(),
            FakeTranscriber::err("500"),
            None,
            FakeInjector::paste(),
            Arc::new(FakeStreaming::erroring_after(5)),
            OutputMode::LiveInject,
        );
        r.start_open().await;
        r.phases_until(|p| matches!(p, DictationPhase::Listening { live: Some(l), .. } if l.degraded.is_some())).await;
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert!(
            matches!(phase(&fx), DictationPhase::Done { text, mode: OutputMode::LiveInject, live_error: Some(e), .. } if text == "你好，世界。" && e == "fake decoder failed; 补齐失败：asr: 500"),
            "{fx:?}"
        );
        assert_eq!(r.injector.injected(), vec!["你好，世界。".to_owned()]);
        // A silent remainder is not transcribed at all: the pasted text is the whole result.
        let mut samples: Vec<i16> =
            wav::pcm_data(&crate::dictation::fakes::speech_recording(400).wav).unwrap().as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
        samples.resize(1500 * 16, 0);
        let recording = Recording { wav: wav::encode_pcm16(&samples, 16_000), duration_ms: 1500, sample_rate_hz: 16_000 };
        let mut r = rig_mode(
            FakeAudio::recording(recording),
            FakeTranscriber::ok("never"),
            None,
            FakeInjector::paste(),
            Arc::new(FakeStreaming::erroring_after(5)),
            OutputMode::LiveInject,
        );
        r.start_open().await;
        r.phases_until(|p| matches!(p, DictationPhase::Listening { live: Some(l), .. } if l.degraded.is_some())).await;
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { text, mode: OutputMode::LiveInject, .. } if text == "你好，世界。"), "{fx:?}");
        assert_eq!(r.transcriber.calls(), 0, "silence after the last sentence is not uploaded");
    }

    /// A streaming mode is only effective with the live preview ready (docs/dictation.md §12):
    /// with the preview switched off, the streaming model missing, or no streaming port at all the
    /// run is a whole take (120 s cap, transcriber called, `Done.mode = whole_take`, no live error);
    /// `configure` to a ready configuration makes the next run streaming.
    #[tokio::test(start_paused = true)]
    async fn streaming_modes_fall_back_to_whole_take_when_live_preview_is_not_ready() {
        for mode in [OutputMode::StreamingFinal, OutputMode::LiveInject] {
            // Preview switched off.
            let streaming = Arc::new(FakeStreaming::script());
            let audio = Arc::new(FakeAudio::speech());
            let transcriber = Arc::new(FakeTranscriber::ok(FAKE_TRANSCRIPT));
            let injector = Arc::new(FakeInjector::paste());
            let (levels_tx, levels) = broadcast::channel(64);
            let ports = DictationPorts { streaming: Some(streaming.clone()), ..ports_with(audio.clone(), transcriber.clone(), None, injector.clone()) };
            let (engine, rx) = DictationEngine::new(ports, &resolved_mode(mode, false, false), levels_tx);
            let mut r = Rig { engine, rx, audio, transcriber, refiner: None, injector, levels };
            assert_eq!(
                r.engine.effective_output_mode(),
                (OutputMode::WholeTake, Some("live preview is not ready (switched off or streaming model not installed)"))
            );
            r.start_open().await;
            assert_eq!(r.engine.current_mode(), OutputMode::WholeTake);
            assert_eq!(r.audio.max_durations(), vec![MAX_RECORDING], "a whole take keeps the 120 s cap");
            assert_eq!(r.audio.live_requests(), 0);
            let fx = r.engine.stop().unwrap();
            assert!(matches!(phase(&fx), DictationPhase::Processing { stage: ProcessingStage::Transcribing, .. }));
            let fx = r.run_to_terminal().await;
            assert!(
                matches!(phase(&fx), DictationPhase::Done { text, mode: OutputMode::WholeTake, live_error: None, segments: None, .. } if text == FAKE_TRANSCRIPT),
                "{fx:?}"
            );
            assert_eq!(r.transcriber.calls(), 1);
            assert_eq!(streaming.opens(), 0);
            // Switched on and installed: the next run streams.
            r.engine.configure(&resolved_mode(mode, true, false));
            assert_eq!(r.engine.effective_output_mode(), (mode, None));
            tokio::time::advance(DWELL).await;
            assert_eq!(phase(&r.next().await), &DictationPhase::Idle);
            r.start_open().await;
            assert_eq!(r.engine.current_mode(), mode);
            assert_eq!(r.audio.max_durations(), vec![MAX_RECORDING, MAX_RECORDING_STREAMING]);
            r.phases_until(|p| matches!(p, DictationPhase::Listening { live: Some(_), .. })).await;
            r.engine.cancel().unwrap();
            r.wait_stops(2).await;
            r.drain().await;
            // Ready by configuration but no streaming port (the phone): whole take too.
            let mut r = rig(FakeAudio::speech(), FakeTranscriber::ok(FAKE_TRANSCRIPT), None, FakeInjector::paste(), false);
            r.engine.configure(&resolved_mode(mode, true, false));
            assert_eq!(r.engine.effective_output_mode(), (OutputMode::WholeTake, Some("this shell has no streaming recogniser")));
            r.start_open().await;
            assert_eq!(r.audio.max_durations(), vec![MAX_RECORDING]);
            r.engine.stop().unwrap();
            let fx = r.run_to_terminal().await;
            assert!(matches!(phase(&fx), DictationPhase::Done { mode: OutputMode::WholeTake, live_error: None, .. }), "{fx:?}");
            assert_eq!(r.transcriber.calls(), 1);
        }
        assert_eq!(happy().engine.effective_output_mode(), (OutputMode::WholeTake, None));
    }

    // ---------------- vocabulary (docs/dictation.md §16) and script (§17) ----------------

    fn dict_entry(term: &str, heard: &[&str]) -> crate::vocabulary::DictionaryEntry {
        crate::vocabulary::DictionaryEntry {
            id: Uuid::new_v4(),
            term: term.into(),
            heard_as: heard.iter().map(|h| (*h).to_owned()).collect(),
            enabled: true,
            source: crate::vocabulary::EntrySource::Manual,
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    fn vocab_rule(name: &str, kind: crate::vocabulary::RuleKind, pattern: &str, replacement: &str) -> crate::vocabulary::ReplacementRule {
        crate::vocabulary::ReplacementRule {
            id: Uuid::new_v4(),
            name: name.into(),
            kind,
            pattern: pattern.into(),
            replacement: replacement.into(),
            case_sensitive: true,
            enabled: true,
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    fn with_script(script: ChineseScript, refine_enabled: bool) -> ResolvedEngines {
        let settings = EngineSettings { refine_enabled, chinese_script: script, ..EngineSettings::default() };
        ResolvedEngines::resolve(&settings, &UserSecrets::default(), &TEST_BUILT_IN)
    }

    /// docs/dictation.md §16.3, whole take: the recogniser gets the glossary, the dictionary corrects
    /// the transcript before the refiner (which also gets the glossary), the rules rewrite the
    /// refined text, `raw_text` stays the recogniser's, and the history records what fired.
    #[tokio::test(start_paused = true)]
    async fn vocabulary_runs_dictionary_before_refine_and_rules_after_it() {
        let mut r = rig(
            FakeAudio::speech(),
            FakeTranscriber::ok("我想创建一个谷歌IDR吧"),
            Some(FakeRefiner::ok("我想创建一个 good idea 吧。")),
            FakeInjector::paste(),
            true,
        );
        let good = dict_entry("good idea", &["谷歌IDR"]);
        let rule = vocab_rule("吧", crate::vocabulary::RuleKind::Regex, r"\s*吧。$", "。");
        r.engine.set_vocabulary(Arc::new(Vocabulary::compile(&[good.clone(), dict_entry("Teams", &[])], std::slice::from_ref(&rule))));
        r.start_open().await;
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        match phase(&fx) {
            DictationPhase::Done { text, raw_text, refined, .. } => {
                assert_eq!(text, "我想创建一个 good idea。", "the rule rewrote the refined text");
                assert_eq!(raw_text, "我想创建一个谷歌IDR吧", "raw_text stays what the recogniser said");
                assert!(*refined);
            }
            other => panic!("{other:?}"),
        }
        let glossary = vec!["good idea".to_owned(), "Teams".to_owned()];
        assert_eq!(r.transcriber.glossaries(), vec![glossary.clone()], "the recogniser got the glossary");
        assert_eq!(r.refiner.as_ref().unwrap().inputs(), vec![("我想创建一个good idea吧".to_owned(), glossary)], "the refiner got the corrected text");
        assert_eq!(r.injector.injected(), vec!["我想创建一个 good idea。".to_owned()]);
        let entry = record(&fx).unwrap();
        assert_eq!(
            entry.vocabulary,
            Some(VocabularyHits {
                corrections: vec![crate::vocabulary::VocabularyHit { id: good.id, count: 1 }],
                rules: vec![crate::vocabulary::VocabularyHit { id: rule.id, count: 1 }]
            })
        );
        // Without a refiner the rules work on the corrected text; nothing fired, nothing recorded.
        let mut r = rig(FakeAudio::speech(), FakeTranscriber::ok("谷歌IDR"), None, FakeInjector::paste(), false);
        r.engine.set_vocabulary(Arc::new(Vocabulary::compile(
            std::slice::from_ref(&good),
            &[vocab_rule("gi", crate::vocabulary::RuleKind::Literal, "good idea", "Good Idea")],
        )));
        r.start_open().await;
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { text, .. } if text == "Good Idea"), "{fx:?}");
        let mut r = happy();
        r.start_open().await;
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert_eq!(record(&fx).unwrap().vocabulary, None, "an empty vocabulary records nothing");
        assert_eq!(r.transcriber.glossaries(), vec![Vec::<String>::new()]);
    }

    /// A vocabulary step that fails never costs the take (docs/dictation.md §16.3): a rule that
    /// would blow the text up is skipped with the text as it was; a rule set that empties the text
    /// ends the take as `no_speech`, injecting nothing.
    #[tokio::test(start_paused = true)]
    async fn vocabulary_failures_fall_back_and_an_emptied_text_is_no_speech() {
        let explode = vocab_rule("explode", crate::vocabulary::RuleKind::Literal, "好", &"好".repeat(crate::vocabulary::MAX_REPLACEMENT_CHARS));
        let mut r = rig(FakeAudio::speech(), FakeTranscriber::ok(&"你好".repeat(200)), None, FakeInjector::paste(), false);
        r.engine.set_vocabulary(Arc::new(Vocabulary::compile(&[], &[explode])));
        r.start_open().await;
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { text, .. } if *text == "你好".repeat(200)), "the unmodified text went out");
        assert_eq!(record(&fx).unwrap().vocabulary, None);
        let wipe = vocab_rule("wipe", crate::vocabulary::RuleKind::Regex, "(?s).+", "");
        let mut r = rig(FakeAudio::speech(), FakeTranscriber::ok("嗯"), None, FakeInjector::paste(), false);
        r.engine.set_vocabulary(Arc::new(Vocabulary::compile(&[], &[wipe])));
        r.start_open().await;
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert_eq!(phase(&fx), &DictationPhase::Failed { code: FailureCode::NoSpeech, message: "没有听到声音".into(), text: None });
        assert!(r.injector.injected().is_empty() && record(&fx).is_none());
    }

    /// A run keeps the vocabulary it started with (docs/dictation.md §16.3): a change made while it
    /// records applies from the next run on.
    #[tokio::test(start_paused = true)]
    async fn a_run_keeps_the_vocabulary_snapshot_it_started_with() {
        let mut r = rig(FakeAudio::speech(), FakeTranscriber::ok("沃提普"), None, FakeInjector::paste(), false);
        r.engine.set_vocabulary(Arc::new(Vocabulary::compile(&[dict_entry("Voltip", &["沃提普"])], &[])));
        r.start_open().await;
        r.engine.set_vocabulary(Arc::new(Vocabulary::empty()));
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { text, .. } if text == "Voltip"), "{fx:?}");
        tokio::time::advance(DWELL).await;
        assert_eq!(phase(&r.next().await), &DictationPhase::Idle);
        r.start_open().await;
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { text, .. } if text == "沃提普"), "{fx:?}");
    }

    /// `live_inject` (docs/dictation.md §16.3): every sentence is corrected and rewritten before it
    /// is pasted; the recogniser's sentences stay in `segments` and `raw_text`, the pasted ones make
    /// `text`, and the hits of all sentences add up in the history.
    #[tokio::test(start_paused = true)]
    async fn live_inject_corrects_and_rewrites_each_sentence_before_pasting_it() {
        let world = dict_entry("World", &["world"]);
        let fine = vocab_rule("fine", crate::vocabulary::RuleKind::Literal, "Fine", "Fine!");
        let mut r = rig_mode(FakeAudio::speech(), FakeTranscriber::ok("never"), None, FakeInjector::paste(), english_script(), OutputMode::LiveInject);
        r.engine.set_vocabulary(Arc::new(Vocabulary::compile(std::slice::from_ref(&world), std::slice::from_ref(&fine))));
        r.start_open().await;
        r.phases_until(|p| matches!(p, DictationPhase::Listening { live: Some(l), .. } if l.injected == 2)).await;
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        match phase(&fx) {
            DictationPhase::Done { text, raw_text, segments, mode, .. } => {
                assert_eq!(text, "Hello World. How are you. Fine!");
                assert_eq!(raw_text, "Hello world. How are you. Fine");
                assert_eq!(*mode, OutputMode::LiveInject);
                assert_eq!(segments.as_ref().unwrap().iter().map(|s| s.text.as_str()).collect::<Vec<_>>(), ["Hello world.", "How are you.", "Fine"]);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(r.injector.injected(), ["Hello World. ", "How are you. ", "Fine! "].map(String::from), "each sentence pasted already corrected");
        let entry = record(&fx).unwrap();
        assert_eq!(
            entry.vocabulary,
            Some(VocabularyHits {
                corrections: vec![crate::vocabulary::VocabularyHit { id: world.id, count: 1 }],
                rules: vec![crate::vocabulary::VocabularyHit { id: fine.id, count: 1 }]
            })
        );
        // A sentence the rules empty is recorded but not pasted.
        let drop_how = vocab_rule("drop", crate::vocabulary::RuleKind::Literal, "How are you.", "");
        let mut r = rig_mode(FakeAudio::speech(), FakeTranscriber::ok("never"), None, FakeInjector::paste(), english_script(), OutputMode::LiveInject);
        r.engine.set_vocabulary(Arc::new(Vocabulary::compile(&[], &[drop_how])));
        r.start_open().await;
        r.phases_until(|p| matches!(p, DictationPhase::Listening { live: Some(l), .. } if l.committed.len() == 2)).await;
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { text, segments: Some(s), .. } if text == "Hello world. Fine" && s.len() == 3), "{fx:?}");
        assert_eq!(r.injector.injected(), ["Hello world. ", "Fine "].map(String::from));
    }

    /// `streaming_final` (docs/dictation.md §16.3): the joined stream text is corrected before the
    /// refiner and the rules run after it, like a whole take.
    #[tokio::test(start_paused = true)]
    async fn streaming_final_runs_the_vocabulary_on_the_joined_text() {
        let weather = dict_entry("天氣預報", &["天气"]);
        let mut r = rig_mode(
            FakeAudio::speech(),
            FakeTranscriber::ok("never"),
            Some(FakeRefiner::ok("润色后")),
            FakeInjector::paste(),
            Arc::new(FakeStreaming::script()),
            OutputMode::StreamingFinal,
        );
        r.engine.configure(&resolved_mode(OutputMode::StreamingFinal, true, true));
        r.engine.set_vocabulary(Arc::new(Vocabulary::compile(&[weather], &[vocab_rule("r", crate::vocabulary::RuleKind::Literal, "润色后", "终稿")])));
        r.start_open().await;
        r.phases_until(|p| matches!(p, DictationPhase::Listening { live: Some(l), .. } if l.current == "今天")).await;
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { text, raw_text, .. } if text == "终稿" && raw_text == "你好，世界。今天天气"), "{fx:?}");
        let inputs = r.refiner.as_ref().unwrap().inputs();
        assert_eq!(inputs[0].0, "你好，世界。今天天氣預報", "the refiner got the corrected text");
        assert_eq!(inputs[0].1, vec!["天氣預報".to_owned()]);
    }

    /// docs/dictation.md §17: the default local model answers the public zh.wav sample in
    /// Traditional; with the default `simplified` the text that reaches the application (and the
    /// history, raw text included) is Simplified. `as_is` keeps it, `traditional` converts the other
    /// way, and the dictionary matches the chosen script.
    #[tokio::test(start_paused = true)]
    async fn regression_a_traditional_transcript_is_injected_in_simplified_by_default() {
        const TRADITIONAL: &str = "開放時間：早上九點至下午五點。";
        const SIMPLIFIED: &str = "开放时间：早上九点至下午五点。";
        let mut r = rig(FakeAudio::speech(), FakeTranscriber::ok(TRADITIONAL), None, FakeInjector::paste(), false);
        assert_eq!(r.engine.chinese_script, ChineseScript::Simplified, "the default");
        r.start_open().await;
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { text, raw_text, .. } if text == SIMPLIFIED && raw_text == SIMPLIFIED), "{fx:?}");
        assert_eq!(r.injector.injected(), vec![SIMPLIFIED.to_owned()]);
        for (script, heard, expect) in [(ChineseScript::AsIs, TRADITIONAL, TRADITIONAL), (ChineseScript::Traditional, SIMPLIFIED, TRADITIONAL)] {
            let mut r = rig(FakeAudio::speech(), FakeTranscriber::ok(heard), None, FakeInjector::paste(), false);
            r.engine.configure(&with_script(script, false));
            r.start_open().await;
            r.engine.stop().unwrap();
            let fx = r.run_to_terminal().await;
            assert!(matches!(phase(&fx), DictationPhase::Done { text, .. } if text == expect), "{script:?}: {fx:?}");
        }
        // The dictionary is written in Simplified and still matches the Traditional recognition.
        let mut r = rig(FakeAudio::speech(), FakeTranscriber::ok(TRADITIONAL), None, FakeInjector::paste(), false);
        r.engine.set_vocabulary(Arc::new(Vocabulary::compile(&[dict_entry("营业时间", &["开放时间"])], &[])));
        r.start_open().await;
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { text, .. } if text == "营业时间：早上九点至下午五点。"), "{fx:?}");
    }

    /// docs/dictation.md §17 on the live path: partials, committed sentences and the flush are
    /// converted on the decode thread, so the pill, `live_inject` and `streaming_final` all see the
    /// chosen script.
    #[tokio::test(start_paused = true)]
    async fn streaming_partials_sentences_and_the_flush_come_out_in_the_chosen_script() {
        let traditional = Arc::new(FakeStreaming::words(["開放", "開放時間。", "早上", "早上九點"].map(String::from).to_vec(), 2));
        let mut r = rig_mode(FakeAudio::speech(), FakeTranscriber::ok("never"), None, FakeInjector::paste(), traditional, OutputMode::LiveInject);
        r.start_open().await;
        let phases = r.phases_until(|p| matches!(p, DictationPhase::Listening { live: Some(l), .. } if l.committed.len() == 2)).await;
        let lives: Vec<LiveText> =
            phases.iter().filter_map(|p| if let DictationPhase::Listening { live: Some(l), .. } = p { Some(l.clone()) } else { None }).collect();
        assert_eq!(lives[0].current, "开放", "the first partial is converted");
        let last = lives.last().unwrap();
        assert_eq!(last.committed.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(), ["开放时间。", "早上九点"]);
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { text, .. } if text == "开放时间。早上九点"), "{fx:?}");
        assert_eq!(r.injector.injected(), ["开放时间。", "早上九点"].map(String::from));
    }

    // ---------------- voice edit (docs/dictation.md §19) ----------------

    const SELECTION: &str = "大家好，会议改到周四十点哈";
    const INSTRUCTION: &str = "改的更正式";
    const REWRITE: &str = "各位同事：会议改至周四上午十点。";

    /// A rig whose recogniser hears the instruction, with `injector` playing the foreground
    /// application and `refiner` the LLM; refinement is switched **off** (an edit does not need it).
    fn edit_rig(injector: FakeInjector, refiner: Option<FakeRefiner>) -> Rig {
        rig(FakeAudio::speech(), FakeTranscriber::ok(INSTRUCTION), refiner, injector, false)
    }

    impl Rig {
        fn mic_open(&self) -> bool {
            matches!(*self.engine.mic.lock(), Mic::Open(_))
        }

        /// Fold whatever arrives until `cond` holds (bounded by two real seconds): the late device
        /// notifications of a take that already ended (a refused edit releases the device only
        /// once its open came back).
        async fn fold_until(&mut self, cond: impl Fn(&Self) -> bool) {
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while !cond(self) && std::time::Instant::now() < deadline {
                self.settle().await;
                while let Ok(ev) = self.rx.try_recv() {
                    self.engine.on_internal(ev);
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            assert!(cond(self), "condition not reached within two seconds");
        }

        /// The copy is not pending and not in flight any more (or the take has no edit state).
        fn selection_back(&self) -> bool {
            !matches!(self.engine.take.edit.as_ref().map(|e| &e.selection), Some(SelectionState::Pending | SelectionState::Copying))
        }

        /// The edit key's press: `start_edit`, the copy at once (what the runtime does for
        /// `AtPress`), then fold until the microphone is open and the copy is back (the three
        /// notifications arrive in any order) — or the take ended.
        async fn start_edit_open(&mut self, held: Vec<Modifier>) -> Vec<Effect> {
            let fx = self.engine.start_edit(held).unwrap();
            assert!(matches!(phase(&fx), DictationPhase::Listening { .. }), "{fx:?}");
            assert_eq!(self.engine.status().kind, TakeKind::Edit);
            self.engine.capture_selection();
            let mut all = fx;
            while !(self.mic_open() && self.selection_back()) {
                let more = self.next().await;
                let ended = phase_opt(&more).is_some_and(DictationPhase::is_terminal);
                all.extend(more);
                if ended {
                    break;
                }
            }
            all
        }
    }

    /// docs/dictation.md §19 end to end on the fakes: the edit key opens the microphone and copies
    /// the selection with the hotkey's modifiers; the instruction goes through the recogniser (with
    /// the glossary), the script and the dictionary; `Refiner::edit` gets the selection and the
    /// corrected instruction; the injector pastes the rewrite exactly as returned (the replacement
    /// rules never run on it), even with refinement switched off; the history records the kind,
    /// the instruction and the selection.
    #[tokio::test(start_paused = true)]
    async fn edit_take_rewrites_the_selection_by_the_spoken_instruction_and_pastes_the_result() {
        let mut r = edit_rig(FakeInjector::paste().with_selection(SELECTION), Some(FakeRefiner::ok(REWRITE)));
        let fix = dict_entry("改得", &["改的"]);
        let rule = vocab_rule("周四", crate::vocabulary::RuleKind::Literal, "周四", "星期四");
        r.engine.set_vocabulary(Arc::new(Vocabulary::compile(&[fix.clone(), dict_entry("Voltip", &[])], std::slice::from_ref(&rule))));
        let fx = r.start_edit_open(vec![Modifier::Ctrl, Modifier::Alt]).await;
        assert!(statuses(&fx).iter().all(|p| matches!(p, DictationPhase::Listening { .. })), "{fx:?}");
        assert_eq!(r.injector.copies(), vec![vec![Modifier::Ctrl, Modifier::Alt]], "one copy, with the edit chord's modifiers");
        assert_eq!(r.engine.status().kind, TakeKind::Edit);
        // A second capture is a no-op: the runtime may call it at the press, the key-up and the stop.
        r.engine.capture_selection();
        let fx = r.engine.stop().unwrap();
        assert!(matches!(phase(&fx), DictationPhase::Processing { stage: ProcessingStage::Transcribing, .. }), "{fx:?}");
        let fx = r.run_to_terminal().await;
        let stages: Vec<ProcessingStage> =
            statuses(&fx).iter().filter_map(|p| if let DictationPhase::Processing { stage, .. } = p { Some(*stage) } else { None }).collect();
        assert_eq!(stages, [ProcessingStage::Refining, ProcessingStage::Inserting], "{fx:?}");
        match phase(&fx) {
            DictationPhase::Done { text, raw_text, via, refined, refine_ms, mode, chars, .. } => {
                assert_eq!(text, REWRITE, "the rule for 周四 did not run on the rewrite");
                assert_eq!(raw_text, INSTRUCTION, "raw_text is what the recogniser heard");
                assert_eq!((*via, *refined, *refine_ms, *mode), (Via::Paste, true, Some(FAKE_LATENCY_MS), OutputMode::WholeTake));
                assert_eq!(*chars, REWRITE.chars().count());
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(r.engine.status().kind, TakeKind::Edit, "the status says which kind of take this was");
        assert_eq!(r.copies_and_edits(), (1, vec![(SELECTION.to_owned(), "改得更正式".to_owned(), vec!["改得".to_owned(), "Voltip".to_owned()])]));
        assert_eq!(r.transcriber.glossaries(), vec![vec!["改得".to_owned(), "Voltip".to_owned()]], "the recogniser got the glossary");
        assert_eq!(r.injector.injected(), vec![REWRITE.to_owned()]);
        assert_eq!(r.refiner.as_ref().unwrap().calls(), 0, "no refine pass on top of the edit");
        let entry = record(&fx).expect("an edit is recorded");
        assert_eq!(entry.kind, TakeKind::Edit);
        assert_eq!(entry.edit, Some(EditRecord { instruction: "改得更正式".into(), selection: SELECTION.into() }));
        assert_eq!((entry.text.as_str(), entry.raw_text.as_str()), (REWRITE, INSTRUCTION));
        assert!(entry.refined && entry.refine_model.as_deref() == Some(FAKE_REFINE_MODEL));
        assert_eq!(entry.outcome, Outcome::Inserted { via: Via::Paste });
        assert_eq!(
            entry.vocabulary,
            Some(VocabularyHits { corrections: vec![crate::vocabulary::VocabularyHit { id: fix.id, count: 1 }], rules: Vec::new() }),
            "the dictionary corrected the instruction; no rule fired"
        );
        // The next dictation take is a dictation again.
        tokio::time::advance(DWELL).await;
        assert_eq!(phase(&r.next().await), &DictationPhase::Idle);
        r.start_open().await;
        assert_eq!(r.engine.status().kind, TakeKind::Dictation);
        assert!(format!("{:?}", Internal::SelectionCopied { session: 1, result: Ok(Some("秘密".into())) }).contains("chars: Ok(Some(2))"));
        assert!(
            !format!("{:?}", Internal::SelectionCopied { session: 1, result: Ok(Some("秘密".into())) }).contains("秘密"),
            "the selection stays out of the log"
        );
    }

    impl Rig {
        fn copies_and_edits(&self) -> (usize, Vec<(String, String, Vec<String>)>) {
            (self.injector.copies().len(), self.refiner.as_ref().map(|r| r.edits()).unwrap_or_default())
        }
    }

    /// Nothing selected: the take ends as `no_selection` while still listening, before anything is
    /// recognised or sent — the microphone is released, the recogniser and the LLM never run,
    /// nothing is pasted, nothing is recorded.
    #[tokio::test(start_paused = true)]
    async fn regression_no_selection_refuses_the_edit_before_the_recording_is_uploaded() {
        for selection in [None, Some("  \n ")] {
            let injector = match selection {
                Some(blank) => FakeInjector::paste().with_selection(blank),
                None => FakeInjector::paste(),
            };
            let mut r = edit_rig(injector, Some(FakeRefiner::ok(REWRITE)));
            let fx = r.start_edit_open(Vec::new()).await;
            assert_eq!(phase(&fx), &DictationPhase::Failed { code: FailureCode::NoSelection, message: "没有选中文本".into(), text: None }, "{selection:?}");
            r.fold_until(|r| r.audio.stops() == 1).await;
            assert_eq!(r.transcriber.calls(), 0, "the instruction is never uploaded");
            assert_eq!(r.copies_and_edits(), (1, Vec::new()));
            assert!(r.injector.injected().is_empty() && record(&fx).is_none());
            assert!(r.engine.stop().unwrap().is_empty(), "the release after the refusal is silent");
            tokio::time::advance(DWELL).await;
            r.drain().await;
            assert_eq!(r.engine.status().phase, DictationPhase::Idle);
        }
    }

    /// No LLM configured: the edit key fails at once (`edit_unavailable`) without opening the
    /// microphone or copying anything.
    #[tokio::test(start_paused = true)]
    async fn an_edit_without_an_llm_fails_at_once_and_never_opens_the_microphone() {
        let mut r = edit_rig(FakeInjector::paste().with_selection(SELECTION), None);
        let fx = r.engine.start_edit(vec![Modifier::Alt]).unwrap();
        assert_eq!(phase(&fx), &DictationPhase::Failed { code: FailureCode::EditUnavailable, message: format!("edit: {EDIT_NEEDS_REFINE}"), text: None });
        assert_eq!((r.engine.status().kind, r.engine.status().session), (TakeKind::Edit, 1));
        r.engine.capture_selection();
        r.settle().await;
        assert!(r.nothing_pending());
        assert_eq!((r.audio.starts(), r.injector.copies().len()), (0, 0));
        tokio::time::advance(DWELL).await;
        assert_eq!(phase(&r.next().await), &DictationPhase::Idle);
        // Busy guard: an edit cannot start over a running take either.
        let mut r = edit_rig(FakeInjector::paste().with_selection(SELECTION), Some(FakeRefiner::ok(REWRITE)));
        r.start_edit_open(Vec::new()).await;
        assert_eq!(r.engine.start_edit(Vec::new()).unwrap_err(), DictationError::Busy);
        assert_eq!(r.engine.start().unwrap_err(), DictationError::Busy);
    }

    /// A selection past the limit, or a copy that failed, end the take with their own codes.
    #[tokio::test(start_paused = true)]
    async fn selection_problems_end_the_take_with_their_own_code() {
        let long = "字".repeat(MAX_EDIT_SELECTION_CHARS + 1);
        let mut r = edit_rig(FakeInjector::paste().with_selection(&long), Some(FakeRefiner::ok(REWRITE)));
        let fx = r.start_edit_open(Vec::new()).await;
        assert_eq!(
            phase(&fx),
            &DictationPhase::Failed {
                code: FailureCode::SelectionTooLong,
                message: format!("选中文本过长：{} 字（上限 2000 字）", MAX_EDIT_SELECTION_CHARS + 1),
                text: None
            }
        );
        r.fold_until(|r| r.audio.stops() == 1).await;
        // Exactly at the limit is fine.
        let mut r = edit_rig(FakeInjector::paste().with_selection(&"字".repeat(MAX_EDIT_SELECTION_CHARS)), Some(FakeRefiner::ok(REWRITE)));
        let fx = r.start_edit_open(Vec::new()).await;
        assert!(matches!(phase(&fx), DictationPhase::Listening { .. }), "{fx:?}");
        let error = DictationError::Selection("no copy tool on Wayland · GNOME".into());
        let mut r = edit_rig(FakeInjector::paste().with_selection_error(error.clone()), Some(FakeRefiner::ok(REWRITE)));
        let fx = r.start_edit_open(Vec::new()).await;
        assert_eq!(phase(&fx), &DictationPhase::Failed { code: FailureCode::Selection, message: error.to_string(), text: None });
        assert_eq!(r.transcriber.calls(), 0);
    }

    /// `AfterKeyUp` (an X11 session): nothing is copied at the press; a stop that finds the copy not
    /// done yet copies first, and the pipeline waits for it — however long the copy takes.
    #[tokio::test(start_paused = true)]
    async fn after_key_up_the_stop_copies_first_and_the_pipeline_waits_for_the_copy() {
        let mut r = edit_rig(FakeInjector::paste().with_selection(SELECTION).after_key_up().copy_gated(), Some(FakeRefiner::ok(REWRITE)));
        assert_eq!(r.engine.selection_timing(), SelectionTiming::AfterKeyUp);
        let fx = r.engine.start_edit(vec![Modifier::Alt]).unwrap();
        assert!(matches!(phase(&fx), DictationPhase::Listening { .. }));
        // The runtime does not call capture_selection for AfterKeyUp until the key is up.
        assert!(!r.engine.on_internal(r.rx.recv().await.unwrap()).is_empty(), "ready");
        assert!(r.engine.on_internal(r.rx.recv().await.unwrap()).is_empty(), "the device opened");
        assert!(r.injector.copies().is_empty() && r.injector.copies_waiting() == 0, "no copy at the press");
        let fx = r.engine.stop().unwrap();
        assert!(matches!(phase(&fx), DictationPhase::Processing { .. }));
        wait_until(|| r.injector.copies_waiting() == 1).await;
        assert!(r.next().await.is_empty(), "the recording is here, the pipeline waits for the selection");
        r.settle().await;
        assert_eq!(r.transcriber.calls(), 0, "nothing is recognised before the selection is known");
        r.injector.release_copies(1);
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { text, .. } if text == REWRITE), "{fx:?}");
        assert_eq!(r.injector.copies(), vec![vec![Modifier::Alt]]);
    }

    /// Cancel at any point leaves the selection alone: while the copy is in flight (its late answer
    /// is dropped), and while the LLM is working (the pipeline is aborted, nothing is pasted).
    #[tokio::test(start_paused = true)]
    async fn cancel_during_an_edit_never_pastes() {
        let mut r = edit_rig(FakeInjector::paste().with_selection(SELECTION).copy_gated(), Some(FakeRefiner::ok(REWRITE)));
        r.engine.start_edit(Vec::new()).unwrap();
        r.engine.capture_selection();
        wait_until(|| r.injector.copies_waiting() == 1).await;
        assert_eq!(phase(&r.engine.cancel().unwrap()), &DictationPhase::CANCELLED);
        r.injector.release_copies(1);
        r.drain().await;
        wait_until(|| r.injector.copies().len() == 1).await;
        r.drain().await;
        assert_eq!(r.engine.status().phase, DictationPhase::CANCELLED, "the late copy changes nothing");
        assert!(r.injector.injected().is_empty() && r.transcriber.calls() == 0);
        // While the LLM is rewriting.
        let mut r = edit_rig(FakeInjector::paste().with_selection(SELECTION), Some(FakeRefiner::slow(REWRITE, Duration::from_secs(5))));
        r.start_edit_open(Vec::new()).await;
        r.engine.stop().unwrap();
        r.phases_until(|p| matches!(p, DictationPhase::Processing { stage: ProcessingStage::Refining, .. })).await;
        assert_eq!(phase(&r.engine.cancel().unwrap()), &DictationPhase::CANCELLED);
        tokio::time::advance(Duration::from_secs(10)).await;
        r.drain().await;
        assert!(r.injector.injected().is_empty(), "a cancelled edit never pastes");
        assert_eq!(r.copies_and_edits().1.len(), 1, "the request had gone out");
    }

    /// Everything that goes wrong after the copy leaves the selection untouched: an LLM error, an
    /// empty rewrite, a silent instruction; a failed paste keeps the rewrite for copying and records it.
    #[tokio::test(start_paused = true)]
    async fn edit_failures_leave_the_selection_alone() {
        let cases: Vec<(FakeAudio, FakeRefiner, FakeInjector, DictationPhase)> = vec![
            (
                FakeAudio::speech(),
                FakeRefiner::err("refine answer was cut off at the output limit"),
                FakeInjector::paste(),
                DictationPhase::Failed { code: FailureCode::Refine, message: "refine: refine answer was cut off at the output limit".into(), text: None },
            ),
            (
                FakeAudio::speech(),
                FakeRefiner::ok("  \n"),
                FakeInjector::paste(),
                DictationPhase::Failed { code: FailureCode::Refine, message: "refine: 改写结果为空，选中文本未改动".into(), text: None },
            ),
            (
                FakeAudio::silence(),
                FakeRefiner::ok(REWRITE),
                FakeInjector::paste(),
                DictationPhase::Failed { code: FailureCode::NoSpeech, message: "没有听到声音".into(), text: None },
            ),
            (
                FakeAudio::speech(),
                FakeRefiner::ok(REWRITE),
                FakeInjector::err("denied"),
                DictationPhase::Failed { code: FailureCode::Inject, message: "inject: denied".into(), text: Some(REWRITE.into()) },
            ),
        ];
        for (audio, refiner, injector, expected) in cases {
            let mut r = rig(audio, FakeTranscriber::ok(INSTRUCTION), Some(refiner), injector.with_selection(SELECTION), false);
            r.start_edit_open(Vec::new()).await;
            r.engine.stop().unwrap();
            let fx = r.run_to_terminal().await;
            assert_eq!(phase(&fx), &expected);
            let pasted = r.injector.injected();
            match &expected {
                DictationPhase::Failed { code: FailureCode::Inject, .. } => {
                    assert_eq!(pasted, vec![REWRITE.to_owned()], "the paste was attempted");
                    assert!(matches!(record(&fx), Some(HistoryEntry { kind: TakeKind::Edit, outcome: Outcome::Failed { .. }, .. })));
                }
                _ => {
                    assert!(pasted.is_empty(), "{expected:?}: nothing pasted");
                    assert!(record(&fx).is_none(), "{expected:?}: nothing recorded");
                }
            }
        }
        // An empty instruction after the dictionary (a filler the dictionary cannot touch is still
        // text, so only a transcript that is blank reaches here).
        let mut r =
            rig(FakeAudio::speech(), FakeTranscriber::ok("   "), Some(FakeRefiner::ok(REWRITE)), FakeInjector::paste().with_selection(SELECTION), false);
        r.start_edit_open(Vec::new()).await;
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Failed { code: FailureCode::NoSpeech, .. }), "{fx:?}");
        assert!(r.refiner.as_ref().unwrap().edits().is_empty());
    }

    /// An edit take is a whole take whatever the output mode, but the live preview still shows the
    /// spoken instruction while listening (docs/dictation.md §19).
    #[tokio::test(start_paused = true)]
    async fn edit_takes_ignore_the_output_mode_and_preview_the_instruction() {
        let audio = Arc::new(FakeAudio::speech());
        let transcriber = Arc::new(FakeTranscriber::ok(INSTRUCTION));
        let refiner = Arc::new(FakeRefiner::ok(REWRITE));
        let injector = Arc::new(FakeInjector::paste().with_selection(SELECTION));
        let (levels_tx, levels) = broadcast::channel(64);
        let streaming = Arc::new(FakeStreaming::script());
        let ports =
            DictationPorts { streaming: Some(streaming.clone()), ..ports_with(audio.clone(), transcriber.clone(), Some(refiner.clone()), injector.clone()) };
        let (engine, rx) = DictationEngine::new(ports, &resolved_mode(OutputMode::LiveInject, true, false), levels_tx);
        let mut r = Rig { engine, rx, audio, transcriber, refiner: Some(refiner), injector, levels };
        assert_eq!(r.engine.effective_output_mode().0, OutputMode::LiveInject, "dictation would inject live");
        r.start_edit_open(Vec::new()).await;
        assert_eq!(r.engine.current_mode(), OutputMode::WholeTake);
        assert_eq!(r.audio.max_durations(), vec![MAX_RECORDING], "a whole take's cap");
        r.phases_until(|p| matches!(p, DictationPhase::Listening { live: Some(l), .. } if !l.committed.is_empty())).await;
        assert!(r.injector.injected().is_empty(), "nothing is pasted while listening");
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { text, mode: OutputMode::WholeTake, segments: None, .. } if text == REWRITE), "{fx:?}");
        assert_eq!(r.injector.injected(), vec![REWRITE.to_owned()]);
    }

    /// A streaming take runs up to ten minutes (docs/dictation.md §12), even when it fell back to
    /// the whole take after starting; a stop that arrives while the device is still opening leaves
    /// no live worker behind, so a streaming mode falls back at once instead of waiting for a flush
    /// that never comes.
    #[tokio::test(start_paused = true)]
    async fn streaming_takes_auto_stop_at_ten_minutes_and_a_workerless_take_falls_back() {
        let mut r = rig_mode(
            FakeAudio::speech(),
            FakeTranscriber::ok(FAKE_TRANSCRIPT),
            None,
            FakeInjector::paste(),
            Arc::new(FakeStreaming::failing_open("x")),
            OutputMode::StreamingFinal,
        );
        r.start_open().await;
        r.phases_until(|p| matches!(p, DictationPhase::Listening { live: Some(l), .. } if l.degraded.is_some())).await;
        tokio::time::advance(MAX_RECORDING).await;
        r.settle().await;
        assert!(r.nothing_pending(), "no auto-stop at the whole-take cap");
        tokio::time::advance(MAX_RECORDING_STREAMING - MAX_RECORDING).await;
        let fx = r.next().await;
        assert!(matches!(phase(&fx), DictationPhase::Processing { .. }), "{fx:?}");
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { mode: OutputMode::WholeTake, .. }));
        // Quick tap in a streaming mode: stop before the device opened.
        for mode in [OutputMode::StreamingFinal, OutputMode::LiveInject] {
            let streaming = Arc::new(FakeStreaming::script());
            let mut r = rig_mode(FakeAudio::speech(), FakeTranscriber::ok(FAKE_TRANSCRIPT), None, FakeInjector::paste(), streaming.clone(), mode);
            r.engine.start().unwrap();
            let fx = r.engine.stop().unwrap();
            assert!(matches!(phase(&fx), DictationPhase::Processing { stage: ProcessingStage::Finalizing, .. }), "{fx:?}");
            let fx = r.run_to_terminal().await;
            assert!(
                matches!(phase(&fx), DictationPhase::Done { text, mode: OutputMode::WholeTake, live_error: Some(e), .. } if text == FAKE_TRANSCRIPT && e.contains("device was opening")),
                "{mode:?}: {fx:?}"
            );
            assert_eq!(streaming.opens(), 0, "{mode:?}: no session was ever opened");
            assert_eq!(r.transcriber.calls(), 1);
        }
    }

    // ---------------- scenes and context (docs/dictation.md §18) ----------------

    fn scene_with(name: &str, apps: &[&str], keywords: &[&str], overrides: crate::scenes::SceneOverrides) -> Scene {
        let draft = crate::scenes::SceneDraft {
            name: name.into(),
            enabled: true,
            matching: crate::scenes::SceneMatch {
                apps: apps.iter().map(|a| (*a).to_owned()).collect(),
                title_contains: keywords.iter().map(|k| (*k).to_owned()).collect(),
            },
            overrides,
        };
        let d = crate::scenes::validate_scene_draft(&draft).unwrap();
        Scene { id: Uuid::new_v4(), name: d.name, enabled: d.enabled, matching: d.matching, overrides: d.overrides, created_at_ms: 1, updated_at_ms: 1 }
    }

    /// A rig with `probe` plugged in (and optionally a streaming recogniser), on `engines`.
    fn rig_probed(
        probe: Arc<FakeProbe>,
        transcriber: FakeTranscriber,
        refiner: Option<FakeRefiner>,
        streaming: Option<Arc<FakeStreaming>>,
        engines: &ResolvedEngines,
    ) -> Rig {
        rig_probed_with(probe, transcriber, refiner, streaming, engines, FakeInjector::paste())
    }

    /// [`rig_probed`] over an explicit injector (a voice edit's foreground selection, §19).
    fn rig_probed_with(
        probe: Arc<FakeProbe>,
        transcriber: FakeTranscriber,
        refiner: Option<FakeRefiner>,
        streaming: Option<Arc<FakeStreaming>>,
        engines: &ResolvedEngines,
        injector: FakeInjector,
    ) -> Rig {
        let audio = Arc::new(FakeAudio::speech());
        let transcriber = Arc::new(transcriber);
        let refiner = refiner.map(Arc::new);
        let injector = Arc::new(injector);
        let (levels_tx, levels) = broadcast::channel(64);
        let streaming = streaming.map(|s| s as Arc<dyn StreamingTranscriber>);
        let ports = DictationPorts { streaming, probe: Some(probe), ..ports_with(audio.clone(), transcriber.clone(), refiner.clone(), injector.clone()) };
        let (engine, rx) = DictationEngine::new(ports, engines, levels_tx);
        Rig { engine, rx, audio, transcriber, refiner, injector, levels }
    }

    fn status_of(effects: &[Effect]) -> &DictationStatus {
        match effects.iter().rev().find_map(|e| if let Effect::Status(s) = e { Some(s) } else { None }) {
            Some(s) => s,
            None => panic!("no status effect in {effects:?}"),
        }
    }

    fn app_ref(id: &str, name: &str) -> AppRef {
        AppRef { id: id.into(), name: name.into() }
    }

    impl Rig {
        /// `start()` with a probe: `Listening` at once without a context, then the probe's answer
        /// (returned: a status with the context when it named an app, nothing otherwise), then the
        /// device (`ready`, then the silent open).
        async fn start_probed(&mut self) -> Vec<Effect> {
            let opened = self.audio.starts();
            let fx = self.engine.start().unwrap();
            assert!(matches!(phase(&fx), DictationPhase::Listening { ready: false, .. }), "{fx:?}");
            assert!(status_of(&fx).context.is_none(), "no context before the probe answered");
            assert_eq!(self.audio.starts(), opened, "the device waits for the probe");
            let context = self.next().await;
            let ready = self.next().await;
            assert!(matches!(phase(&ready), DictationPhase::Listening { ready: true, .. }), "{ready:?}");
            assert!(self.next().await.is_empty(), "a successful open reports no new status");
            context
        }
    }

    /// docs/dictation.md §19.2: a terminal's selection is program output, nothing a rewrite can
    /// replace. The copy waits for the probe; when the probe names a terminal of the host's table
    /// the take fails as `edit_in_terminal` with no copy chord sent,
    /// the microphone never opened and nothing recorded — `windowsterminal` under the Windows
    /// table, `gnome-terminal-server` under the Linux one. The same take in an editor copies right
    /// after the answer, and a probe that names nothing lets the copy through as before.
    #[tokio::test(start_paused = true)]
    async fn regression_an_edit_in_a_terminal_is_refused_before_any_key_or_microphone() {
        use voltip_platform::{HostOs, foreground::is_terminal};
        let table = |os: HostOs| move |id: &str| is_terminal(os, id);
        let rig = |os: HostOs, probe: &Arc<FakeProbe>| {
            rig_probed_with(
                probe.clone(),
                FakeTranscriber::ok(INSTRUCTION),
                Some(FakeRefiner::ok(REWRITE)),
                None,
                &resolved(true, None),
                FakeInjector::paste().with_selection(SELECTION).with_terminals(table(os)),
            )
        };
        for (os, id, name) in [(HostOs::Windows, "WindowsTerminal.exe", "WindowsTerminal"), (HostOs::Linux, "gnome-terminal-server", "Gnome-terminal")] {
            let probe = Arc::new(FakeProbe::app(id, name, Some("~/src — cargo build")));
            let mut r = rig(os, &probe);
            let fx = r.engine.start_edit(vec![Modifier::Ctrl, Modifier::Alt]).unwrap();
            assert!(matches!(phase(&fx), DictationPhase::Listening { ready: false, .. }), "{fx:?}");
            // The runtime asks for the copy at the press; it waits for the probe.
            r.engine.capture_selection();
            assert!(r.injector.copies().is_empty(), "{id}: no copy chord before the probe answered");
            let fx = r.next().await;
            match phase(&fx) {
                DictationPhase::Failed { code: FailureCode::EditInTerminal, message, text: None } => {
                    assert_eq!(message, "终端里不支持语音编辑：终端里的选区不能被替换");
                }
                other => panic!("{id}: {other:?}"),
            }
            assert_eq!(r.engine.status().kind, TakeKind::Edit);
            assert!(record(&fx).is_none(), "{id}: nothing recorded");
            // Nothing follows: no capture, no copy, no late notification.
            r.settle().await;
            assert!(r.rx.try_recv().is_err(), "{id}: the take is over");
            assert!(r.injector.copies().is_empty(), "{id}: no copy chord was sent");
            assert_eq!(r.audio.starts(), 0, "{id}: the microphone never opened");
            assert!(r.refiner.as_ref().unwrap().edits().is_empty() && r.transcriber.calls() == 0);
            assert_eq!(probe.calls(), 1);
            // A stop that came before the answer is refused the same way.
            tokio::time::advance(DWELL).await;
            assert_eq!(phase(&r.next().await), &DictationPhase::Idle);
            r.engine.start_edit(Vec::new()).unwrap();
            r.engine.capture_selection();
            r.engine.stop().unwrap();
            let fx = r.next().await;
            assert!(matches!(phase(&fx), DictationPhase::Failed { code: FailureCode::EditInTerminal, .. }), "{id}: {fx:?}");
            r.settle().await;
            assert!(r.injector.copies().is_empty() && r.audio.starts() == 0, "{id}: still no key and no microphone");
        }
        // An editor: the copy goes out once the probe answered, then the take runs as usual.
        let probe = Arc::new(FakeProbe::app("Code.exe", "Code", None));
        let mut r = rig(HostOs::Windows, &probe);
        let fx = r.start_edit_open(vec![Modifier::Ctrl, Modifier::Alt]).await;
        assert!(statuses(&fx).iter().all(|p| matches!(p, DictationPhase::Listening { .. })), "{fx:?}");
        assert_eq!(r.injector.copies(), vec![vec![Modifier::Ctrl, Modifier::Alt]]);
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { text, .. } if text == REWRITE), "{fx:?}");
        // No answer (pure Wayland, the desktop): the guard cannot see a terminal; the copy goes out.
        let probe = Arc::new(FakeProbe::nothing());
        let mut r = rig(HostOs::Linux, &probe);
        r.start_edit_open(Vec::new()).await;
        assert_eq!(r.injector.copies().len(), 1);
    }

    /// docs/dictation.md §19 with §18: an edit take probes the app in front like a dictation — the
    /// app is reference context for the rewrite (the title only when allowed) and goes into the
    /// status and the history — but matches no scene: a matching scene's output mode, style,
    /// language and instruction stay out of the edit.
    #[tokio::test(start_paused = true)]
    async fn an_edit_take_tells_the_refiner_the_app_but_applies_no_scene() {
        let probe = Arc::new(FakeProbe::app("slack", "Slack", Some("#dev")));
        let mut r = rig_probed_with(
            probe.clone(),
            FakeTranscriber::ok(INSTRUCTION),
            Some(FakeRefiner::ok(REWRITE)),
            Some(Arc::new(FakeStreaming::script())),
            &resolved_mode(OutputMode::WholeTake, true, true),
            FakeInjector::paste().with_selection(SELECTION),
        );
        let chat = scene_with(
            "聊天",
            &["slack"],
            &[],
            crate::scenes::SceneOverrides {
                output_mode: Some(OutputMode::StreamingFinal),
                refine_style: Some(crate::engines::RefineStyle::Formal),
                language: Some("en".into()),
                prompt: Some("口语化".into()),
                ..Default::default()
            },
        );
        r.engine.set_scenes(Arc::new(vec![chat]));
        let fx = r.start_edit_open(vec![Modifier::Ctrl, Modifier::Alt]).await;
        assert!(statuses(&fx).iter().all(|p| matches!(p, DictationPhase::Listening { .. })), "{fx:?}");
        let context = TakeContext { app: app_ref("slack", "Slack"), scene: None };
        assert_eq!(r.engine.status().context.as_ref(), Some(&context), "the app, no scene");
        assert_eq!(r.engine.current_mode(), OutputMode::WholeTake, "the scene's streaming mode does not apply");
        assert_eq!(r.audio.max_durations(), vec![MAX_RECORDING]);
        assert_eq!(probe.calls(), 1);
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { text, mode: OutputMode::WholeTake, .. } if text == REWRITE), "{fx:?}");
        let entry = record(&fx).expect("history");
        assert_eq!((entry.kind, entry.app.clone(), entry.scene.clone()), (TakeKind::Edit, Some(app_ref("slack", "Slack")), None));
        assert_eq!(r.transcriber.languages(), vec![None], "the instruction is recognised with the global language hint");
        let refiner = r.refiner.clone().unwrap();
        let hints = refiner.edit_hints();
        assert_eq!(hints.len(), 1);
        assert_eq!((hints[0].style, hints[0].language.as_deref()), (crate::engines::RefineStyle::Default, None));
        assert_eq!(
            hints[0].context,
            RefineContext { app_name: Some("Slack".into()), window_title: None, instruction: None },
            "the title stays home by default"
        );
        assert_eq!(refiner.calls(), 0, "no refine pass");
    }

    /// docs/dictation.md §18.4: the probe names the app before the device opens; the first
    /// matching scene switches this take's output mode (and with it the recording cap), its refine
    /// style, language and instruction; the status and the history carry the context. The next take
    /// in an app no scene names runs with the globals — still telling the refiner which app it is.
    #[tokio::test(start_paused = true)]
    async fn a_matching_scene_overrides_one_take_and_the_next_take_uses_the_globals() {
        let probe = Arc::new(FakeProbe::app("Code.exe", "Code", Some("main.rs — voltip")));
        let mut r = rig_probed(
            probe.clone(),
            FakeTranscriber::ok(FAKE_TRANSCRIPT),
            Some(FakeRefiner::ok("润色后")),
            Some(Arc::new(FakeStreaming::script())),
            &resolved_mode(OutputMode::WholeTake, true, true),
        );
        let code = scene_with(
            "代码",
            &["code"],
            &[],
            crate::scenes::SceneOverrides {
                output_mode: Some(OutputMode::StreamingFinal),
                refine_style: Some(crate::engines::RefineStyle::Formal),
                language: Some("en".into()),
                prompt: Some("保留代码标识符原样".into()),
                ..Default::default()
            },
        );
        let chat = scene_with("聊天", &["slack"], &[], crate::scenes::SceneOverrides { refine_enabled: Some(false), ..Default::default() });
        r.engine.set_scenes(Arc::new(vec![chat, code.clone()]));
        let fx = r.start_probed().await;
        let context = TakeContext { app: app_ref("code", "Code"), scene: Some(code.to_ref()) };
        assert_eq!(status_of(&fx).context.as_ref(), Some(&context), "the listening status gains the context");
        assert!(matches!(phase(&fx), DictationPhase::Listening { .. }));
        assert_eq!(r.engine.current_mode(), OutputMode::StreamingFinal);
        assert_eq!(r.audio.max_durations(), vec![MAX_RECORDING_STREAMING], "the scene's mode decided the recording cap");
        assert_eq!(probe.calls(), 1);
        let seen = r.phases_until(|p| matches!(p, DictationPhase::Listening { live: Some(l), .. } if !l.committed.is_empty())).await;
        assert!(!seen.is_empty());
        assert_eq!(r.engine.status().context.as_ref(), Some(&context), "the context stays while listening");
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { mode: OutputMode::StreamingFinal, refined: true, live_error: None, .. }), "{fx:?}");
        assert_eq!(status_of(&fx).context.as_ref(), Some(&context), "done still names the scene");
        let entry = record(&fx).expect("history");
        assert_eq!((entry.app.clone(), entry.scene.clone()), (Some(app_ref("code", "Code")), Some(code.to_ref())));
        assert_eq!(r.transcriber.calls(), 0, "streaming_final never runs the whole-take pass");
        let refiner = r.refiner.clone().unwrap();
        let hints = refiner.hints();
        assert_eq!(hints.len(), 1);
        assert_eq!((hints[0].style, hints[0].language.as_deref()), (crate::engines::RefineStyle::Formal, Some("en")));
        assert_eq!(
            hints[0].context,
            RefineContext { app_name: Some("Code".into()), window_title: None, instruction: Some("保留代码标识符原样".into()) },
            "the window title stays home by default"
        );
        tokio::time::advance(DWELL).await;
        let fx = r.next().await;
        assert_eq!(phase(&fx), &DictationPhase::Idle);
        assert!(status_of(&fx).context.is_none(), "idle clears the context");
        // The next take: an app no scene names — the globals, the app still told to the refiner.
        probe.set_app("WINWORD", "WINWORD", Some("报告.docx"));
        let fx = r.start_probed().await;
        assert_eq!(status_of(&fx).context, Some(TakeContext { app: app_ref("winword", "WINWORD"), scene: None }));
        assert_eq!(r.engine.current_mode(), OutputMode::WholeTake);
        assert_eq!(r.audio.max_durations(), vec![MAX_RECORDING_STREAMING, MAX_RECORDING]);
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { mode: OutputMode::WholeTake, refined: true, .. }), "{fx:?}");
        let entry = record(&fx).unwrap();
        assert_eq!((entry.app.clone(), entry.scene.clone()), (Some(app_ref("winword", "WINWORD")), None));
        assert_eq!(r.transcriber.languages(), vec![None], "the global language hint (none)");
        let hints = refiner.hints();
        assert_eq!((hints[1].style, hints[1].language.as_deref()), (crate::engines::RefineStyle::Default, None));
        assert_eq!(hints[1].context, RefineContext { app_name: Some("WINWORD".into()), window_title: None, instruction: None });
    }

    /// §18.4: a scene's language reaches the recogniser (`auto` = no hint) and its script the
    /// normaliser; outside the scene the engines' language and script apply.
    #[tokio::test(start_paused = true)]
    async fn scene_language_and_script_reach_the_recogniser_and_the_normaliser() {
        let probe = Arc::new(FakeProbe::app("notes", "Notes", None));
        let mut r = rig_probed(probe.clone(), FakeTranscriber::ok("開放時間：早上九點。"), None, None, &resolved(false, Some("zh")));
        r.engine.set_scenes(Arc::new(vec![
            scene_with(
                "粤语笔记",
                &["notes"],
                &[],
                crate::scenes::SceneOverrides { language: Some("yue".into()), chinese_script: Some(ChineseScript::AsIs), ..Default::default() },
            ),
            scene_with("自动", &["terminal"], &[], crate::scenes::SceneOverrides { language: Some("auto".into()), ..Default::default() }),
        ]));
        for (app, raw) in [("notes", "開放時間：早上九點。"), ("terminal", "开放时间：早上九点。"), ("mail", "开放时间：早上九点。")]
        {
            probe.set_app(app, app, None);
            r.start_probed().await;
            r.engine.stop().unwrap();
            let fx = r.run_to_terminal().await;
            assert!(matches!(phase(&fx), DictationPhase::Done { raw_text, .. } if raw_text == raw), "{app}: {fx:?}");
            tokio::time::advance(DWELL).await;
            assert_eq!(phase(&r.next().await), &DictationPhase::Idle);
        }
        assert_eq!(r.transcriber.languages(), vec![Some("yue".to_owned()), None, Some("zh".to_owned())]);
    }

    /// §18.4: no answer means no scene — the probe failing, finding nothing, panicking or not
    /// answering within `PROBE_DEADLINE`; the take runs with the globals and nothing names an app.
    #[tokio::test(start_paused = true)]
    async fn a_probe_that_fails_or_times_out_means_no_scene() {
        let every = crate::scenes::SceneOverrides { refine_enabled: Some(false), ..Default::default() };
        for probe in [FakeProbe::failing("cannot open display"), FakeProbe::nothing(), FakeProbe::panicking()] {
            let probe = Arc::new(probe);
            let mut r = rig_probed(probe.clone(), FakeTranscriber::ok(FAKE_TRANSCRIPT), Some(FakeRefiner::ok("润色")), None, &resolved(true, None));
            r.engine.set_scenes(Arc::new(vec![scene_with("全部", &["slack"], &[], every.clone())]));
            let fx = r.start_probed().await;
            assert!(fx.is_empty(), "no context, no new status: {fx:?}");
            r.engine.stop().unwrap();
            let fx = r.run_to_terminal().await;
            assert!(matches!(phase(&fx), DictationPhase::Done { refined: true, .. }), "the globals refine: {fx:?}");
            assert!(status_of(&fx).context.is_none());
            let entry = record(&fx).unwrap();
            assert_eq!((entry.app.clone(), entry.scene.clone()), (None, None));
            assert!(r.refiner.as_ref().unwrap().hints()[0].context.is_empty(), "nothing about an app reaches the refiner");
            assert_eq!(probe.calls(), 1);
        }
        // A hung window system: the device opens once the deadline passed, without a scene.
        let probe = Arc::new(FakeProbe::hanging());
        let mut r = rig_probed(probe.clone(), FakeTranscriber::ok(FAKE_TRANSCRIPT), None, None, &resolved(false, None));
        r.engine.set_scenes(Arc::new(vec![scene_with("全部", &["slack"], &[], every)]));
        r.engine.start().unwrap();
        wait_until(|| probe.calls() == 1).await;
        r.settle().await;
        assert!(r.nothing_pending(), "nothing before the deadline");
        assert_eq!(r.audio.starts(), 0);
        tokio::time::advance(PROBE_DEADLINE).await;
        assert!(r.next().await.is_empty(), "the deadline answers `no app`");
        assert!(matches!(phase(&r.next().await), DictationPhase::Listening { ready: true, .. }));
        assert!(r.next().await.is_empty());
        assert_eq!(r.audio.starts(), 1);
        probe.release();
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { .. }), "{fx:?}");
        assert_eq!(record(&fx).unwrap().app, None);
        assert!(format!("{:?}", Internal::Context { session: 1, app: None }).contains("Context"));
    }

    /// §18.4: a take cancelled before the probe answered never opens the device; one stopped before
    /// it answered (a tap quicker than the probe) runs without a scene.
    #[tokio::test(start_paused = true)]
    async fn cancel_or_stop_before_the_probe_answered() {
        let probe = Arc::new(FakeProbe::app("slack", "Slack", None));
        let quiet = crate::scenes::SceneOverrides { refine_enabled: Some(false), ..Default::default() };
        let mut r = rig_probed(probe.clone(), FakeTranscriber::ok(FAKE_TRANSCRIPT), Some(FakeRefiner::ok("润色")), None, &resolved(true, None));
        r.engine.set_scenes(Arc::new(vec![scene_with("聊天", &["slack"], &[], quiet)]));
        r.engine.start().unwrap();
        assert_eq!(phase(&r.engine.cancel().unwrap()), &DictationPhase::CANCELLED);
        assert!(r.next().await.is_empty(), "the late answer changes nothing");
        r.settle().await;
        assert!(r.nothing_pending());
        assert_eq!(r.audio.starts(), 0, "the device never opened");
        assert!(format!("{:?}", r.engine).contains("mic: \"closed\""));
        // Stop first, answer second: the take runs, without the scene (it would have switched refine off).
        r.engine.start().unwrap();
        let fx = r.engine.stop().unwrap();
        assert!(matches!(phase(&fx), DictationPhase::Processing { .. }));
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { refined: true, .. }), "{fx:?}");
        let entry = record(&fx).unwrap();
        assert_eq!((entry.app.clone(), entry.scene.clone()), (None, None));
        assert_eq!(r.audio.starts(), 1);
    }

    /// §18.4: a scene asking for a streaming mode while the live preview is not ready runs the take
    /// as a whole take (the whole-take cap), and says why in `live_error` and the history.
    #[tokio::test(start_paused = true)]
    async fn a_streaming_scene_without_the_live_preview_runs_a_whole_take_and_says_why() {
        let probe = Arc::new(FakeProbe::app("slack", "Slack", None));
        let streaming = Arc::new(FakeStreaming::script());
        let mut r = rig_probed(probe, FakeTranscriber::ok(FAKE_TRANSCRIPT), None, Some(streaming.clone()), &resolved_mode(OutputMode::WholeTake, false, false));
        r.engine.set_scenes(Arc::new(vec![scene_with(
            "实时",
            &["slack"],
            &[],
            crate::scenes::SceneOverrides { output_mode: Some(OutputMode::LiveInject), ..Default::default() },
        )]));
        r.start_probed().await;
        assert_eq!(r.engine.current_mode(), OutputMode::WholeTake);
        assert_eq!(r.audio.max_durations(), vec![MAX_RECORDING]);
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert!(
            matches!(phase(&fx), DictationPhase::Done { mode: OutputMode::WholeTake, live_error: Some(e), text, .. } if e == SCENE_MODE_NOT_READY && text == FAKE_TRANSCRIPT),
            "{fx:?}"
        );
        assert_eq!(record(&fx).unwrap().live_error.as_deref(), Some(SCENE_MODE_NOT_READY));
        assert_eq!(streaming.opens(), 0);
        assert_eq!(r.transcriber.calls(), 1);
        // Ready by configuration but no streaming port: the other reason.
        let probe = Arc::new(FakeProbe::app("slack", "Slack", None));
        let mut r = rig_probed(probe, FakeTranscriber::ok(FAKE_TRANSCRIPT), None, None, &resolved_mode(OutputMode::WholeTake, true, false));
        r.engine.set_scenes(Arc::new(vec![scene_with(
            "实时",
            &["slack"],
            &[],
            crate::scenes::SceneOverrides { output_mode: Some(OutputMode::StreamingFinal), ..Default::default() },
        )]));
        r.start_probed().await;
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { live_error: Some(e), .. } if e == SCENE_MODE_NO_STREAMING), "{fx:?}");
    }

    /// §18.5: the switches decide what of the context reaches the refiner (the title only when
    /// allowed); a scene's refine switch works both ways, and a take that does not refine sends
    /// nothing at all.
    #[tokio::test(start_paused = true)]
    async fn the_privacy_switches_and_the_scene_refine_switch_decide_what_is_sent() {
        let probe = Arc::new(FakeProbe::app("slack", "Slack", Some("#dev · Voltip")));
        let mut r = rig_probed(probe.clone(), FakeTranscriber::ok(FAKE_TRANSCRIPT), Some(FakeRefiner::ok("润色")), None, &resolved(true, None));
        let chat = scene_with("聊天", &["slack"], &[], crate::scenes::SceneOverrides { prompt: Some("口语化".into()), ..Default::default() });
        let silent =
            scene_with("不润色", &["code"], &[], crate::scenes::SceneOverrides { refine_enabled: Some(false), prompt: Some("x".into()), ..Default::default() });
        r.engine.set_scenes(Arc::new(vec![chat, silent]));
        r.engine.set_context_sharing(ContextSharing { app_name: false, window_title: true });
        let refiner = r.refiner.clone().unwrap();
        r.start_probed().await;
        r.engine.stop().unwrap();
        r.run_to_terminal().await;
        assert_eq!(
            refiner.hints()[0].context,
            RefineContext { app_name: None, window_title: Some("#dev · Voltip".into()), instruction: Some("口语化".into()) }
        );
        // A scene that switches refining off: no request, so nothing about the take leaves.
        tokio::time::advance(DWELL).await;
        r.next().await;
        probe.set_app("code", "Code", Some("secret.rs"));
        r.start_probed().await;
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { refined: false, refine_error: None, .. }), "{fx:?}");
        assert_eq!(refiner.calls(), 1, "no second refine request");
        // Globals off, the scene on: the scene refines (with the switches as they were at start).
        let probe = Arc::new(FakeProbe::app("slack", "Slack", Some("#dev")));
        let mut r = rig_probed(probe, FakeTranscriber::ok(FAKE_TRANSCRIPT), Some(FakeRefiner::ok("润色")), None, &resolved(false, None));
        r.engine.set_scenes(Arc::new(vec![scene_with(
            "聊天",
            &["slack"],
            &[],
            crate::scenes::SceneOverrides { refine_enabled: Some(true), ..Default::default() },
        )]));
        r.start_probed().await;
        r.engine.set_context_sharing(ContextSharing { app_name: false, window_title: true });
        r.engine.stop().unwrap();
        let fx = r.run_to_terminal().await;
        assert!(matches!(phase(&fx), DictationPhase::Done { refined: true, .. }), "{fx:?}");
        let hints = r.refiner.as_ref().unwrap().hints();
        assert_eq!(hints[0].context, RefineContext { app_name: Some("Slack".into()), window_title: None, instruction: None }, "the take's snapshot");
    }
}
