//! Dictation: hold the hotkey → record → release → transcribe → (refine) → inject → history.
//!
//! The state machine (docs/dictation.md §2) lives in [`engine::DictationEngine`], owned by the
//! core runtime. It never touches a device or a socket itself: those come in through the traits in
//! [`ports`], real ones from the desktop shell, [`fakes`] in tests.

pub mod activation;
pub mod engine;
pub mod fakes;
pub mod ports;
pub mod remote;
pub mod wav;

use serde::{Deserialize, Serialize};

pub use crate::engines::OutputMode;
pub use crate::scenes::TakeContext;
pub use engine::{DictationEngine, DictationPorts, EngineFactory};
pub use ports::{
    AudioSource, Capture, CaptureOptions, DWELL, DWELL_WITH_TEXT, DictationError, ForegroundApp, ForegroundProbe, Injection, Injector, LIVE_CHUNK_SAMPLES,
    LIVE_SAMPLE_RATE_HZ, LevelFrame, LivePcm, MAX_EDIT_SELECTION_CHARS, MAX_RECORDING, MAX_RECORDING_STREAMING, MIN_RECORDING, PARTIAL_THROTTLE,
    PROBE_DEADLINE, Recording, RefineContext, RefineHints, Refined, Refiner, Segment, SelectionTiming, ServiceProbe, StreamEvent, StreamFinal,
    StreamingSession, StreamingTranscriber, Transcriber, Transcript, Via, max_recording,
};

/// What a take is for (docs/dictation.md §19): dictating text, or rewriting the text selected in
/// the foreground application by a spoken instruction. The hotkey edge says which one it starts
/// (`HotkeyEdge.purpose`), the status and the history say which one ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TakeKind {
    /// The dictation pipeline (every take before §19).
    #[default]
    Dictation,
    /// Voice edit of the selection.
    Edit,
}

impl TakeKind {
    /// Wire name (`dictation` | `edit`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dictation => "dictation",
            Self::Edit => "edit",
        }
    }
}

/// The live preview while listening (docs/dictation.md §11): sentences the streaming recogniser
/// committed at endpoints plus the one being spoken. The pill renders `committed` in the normal
/// colour and `current` dimmed; `degraded` tells it the preview stopped following the audio.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LiveText {
    /// Sentences closed by an endpoint, in order (with stream timestamps).
    #[serde(default)]
    pub committed: Vec<Segment>,
    /// The current, uncommitted sentence.
    #[serde(default)]
    pub current: String,
    /// Why the preview stopped (model failed to load, tap overrun, decoder error); `None` while it
    /// is following the audio. The final text is unaffected either way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub degraded: Option<String>,
    /// `live_inject` (docs/dictation.md §12): how many committed sentences have been pasted so
    /// far; `0` in the other modes.
    #[serde(default)]
    pub injected: usize,
}

impl LiveText {
    /// `committed` + `current` as one string — what `Processing.preview` shows until the final
    /// transcript replaces it. Latin-script neighbours get a space, CJK neighbours do not.
    pub fn preview(&self) -> String {
        let mut out = String::new();
        for piece in self.committed.iter().map(|s| s.text.as_str()).chain(std::iter::once(self.current.as_str())) {
            join_text(&mut out, piece);
        }
        out
    }
}

/// Append `piece` to `out`, inserting a space only when both boundary characters are non-CJK.
fn join_text(out: &mut String, piece: &str) {
    let piece = piece.trim();
    if piece.is_empty() {
        return;
    }
    if let (Some(last), Some(first)) = (out.chars().next_back(), piece.chars().next())
        && !last.is_whitespace()
        && !is_cjk(last)
        && !is_cjk(first)
    {
        out.push(' ');
    }
    out.push_str(piece);
}

/// CJK scripts and their punctuation (U+2E80 and up, excluding the Latin ranges), which join
/// without spaces.
fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x2E80..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFFEF | 0x20000..=0x3134F)
}

/// The separator `live_inject` appends after one sentence so the next one does not run into it
/// (docs/dictation.md §12): nothing after a CJK character or CJK punctuation (`你好。`), one space
/// otherwise (`Hello world.`). The next sentence's first character is not known at inject time,
/// hence a rule on the end of this one only.
pub fn inject_separator(text: &str) -> &'static str {
    match text.trim_end().chars().next_back() {
        None => "",
        Some(last) if is_cjk(last) => "",
        Some(_) => " ",
    }
}

/// Which step of the pipeline is running.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessingStage {
    /// Uploading the WAV and waiting for the transcript.
    Transcribing,
    /// `streaming_final` / `live_inject` (docs/dictation.md §12): waiting for the streaming
    /// recogniser to flush its last sentence (and for the device to close).
    Finalizing,
    /// Waiting for the LLM clean-up.
    Refining,
    /// Handing the text to the foreground application.
    Inserting,
}

/// Which stage of the pipeline stopped a run (`DictationPhase::Failed.code`). Machine-readable so
/// the webview can localise the failure; `message` stays the human-readable Chinese text for the
/// log and as a fallback.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FailureCode {
    /// The recording was too short or silent, or ASR returned an empty transcript.
    NoSpeech,
    /// The capture device could not be opened or stopped.
    Audio,
    /// Speech recognition failed (network, auth, server).
    Asr,
    /// Refinement failed and no raw text could be used either (not produced today: refine errors
    /// are non-fatal and land in `Done.refine_error`).
    Refine,
    /// The text could not reach the foreground application (it is in the clipboard when possible).
    Inject,
    /// Voice edit (docs/dictation.md §19): nothing was selected in the foreground application.
    NoSelection,
    /// Voice edit: the selection is longer than [`MAX_EDIT_SELECTION_CHARS`].
    SelectionTooLong,
    /// Voice edit: the selection could not be read (no copy tool, clipboard unusable).
    Selection,
    /// Voice edit: no LLM is configured (the refine service has no key), so no edit can run.
    EditUnavailable,
    /// Voice edit: the foreground application is a terminal, whose selection cannot be replaced.
    EditInTerminal,
    /// Anything else.
    #[default]
    Unknown,
}

impl From<&DictationError> for FailureCode {
    fn from(error: &DictationError) -> Self {
        match error {
            DictationError::NoSpeech => Self::NoSpeech,
            DictationError::Audio(_) => Self::Audio,
            DictationError::Asr(_) => Self::Asr,
            DictationError::Refine(_) => Self::Refine,
            DictationError::Inject(_) => Self::Inject,
            DictationError::NoSelection => Self::NoSelection,
            DictationError::SelectionTooLong(_) => Self::SelectionTooLong,
            DictationError::Selection(_) => Self::Selection,
            DictationError::EditUnavailable(_) => Self::EditUnavailable,
            DictationError::EditInTerminal => Self::EditInTerminal,
            DictationError::Busy | DictationError::Idle => Self::Unknown,
        }
    }
}

/// Where the dictation is right now (`packages/shared/src/schema.ts` `dictationPhaseSchema`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum DictationPhase {
    /// Nothing running.
    #[default]
    Idle,
    /// The microphone is open.
    Listening {
        /// Unix time in milliseconds when the capture started — re-taken when the device delivered
        /// its first samples (`ready`), so the pill's timer does not count the device's start-up.
        started_at: u64,
        /// The device has delivered audio (docs/dictation.md §11 `CaptureReady`).
        #[serde(default)]
        ready: bool,
        /// The streaming preview, once the first partial result arrived; `None` when live preview
        /// is off or nothing was recognised yet.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        live: Option<LiveText>,
        /// `hold_or_toggle` (docs/dictation.md §13): a short press locked the run, the next press
        /// stops it; the pill shows a lock. Set by the runtime's activation machine.
        #[serde(default)]
        locked: bool,
    },
    /// The recording is on its way through ASR / refine / inject.
    Processing {
        /// Current step.
        stage: ProcessingStage,
        /// Unix time in milliseconds when processing started.
        started_at: u64,
        /// The live preview's text (`committed` + `current`) carried over from `Listening`, shown
        /// in place of「转写中…」until the final transcript arrives.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        preview: Option<String>,
    },
    /// Text delivered.
    Done {
        /// What was injected (refined when refinement succeeded).
        text: String,
        /// The transcript as ASR returned it.
        raw_text: String,
        /// `text.chars().count()`.
        chars: usize,
        /// How the text reached the user.
        via: Via,
        /// The refiner ran and its output was used.
        refined: bool,
        /// Recording length.
        duration_ms: u64,
        /// ASR round trip.
        asr_ms: u64,
        /// Refine round trip when it ran.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        refine_ms: Option<u64>,
        /// Why refinement was skipped or failed (the raw text was injected).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        refine_error: Option<String>,
        /// Where the text came from (docs/dictation.md §12): the mode that produced it — a
        /// streaming mode that degraded reads `whole_take` here, with the reason in `live_error`.
        #[serde(default)]
        mode: OutputMode,
        /// The streaming recogniser's sentences (committed ones plus the flushed tail) when the
        /// text came from it; `None` for a whole take.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        segments: Option<Vec<Segment>>,
        /// Why the streaming path was abandoned or only partly used (a streaming mode was asked
        /// for): open / decode / flush failure, tap overrun, or the remainder's transcription.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        live_error: Option<String>,
    },
    /// The pipeline stopped short.
    Failed {
        /// Which stage failed (for localisation).
        code: FailureCode,
        /// Human-readable reason (Chinese; log / fallback text).
        message: String,
        /// Recognised text that could not be injected (it is in the clipboard when possible).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    /// Cancelled by the user (or the pipeline result was discarded).
    Cancelled {
        /// `live_inject` (docs/dictation.md §12): characters already pasted before the cancel —
        /// they are not taken back. `0` in the other modes.
        #[serde(default)]
        injected_chars: usize,
    },
}

impl DictationPhase {
    /// `Done`, `Failed` or `Cancelled`: the states that return to `Idle` on their own.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Done { .. } | Self::Failed { .. } | Self::Cancelled { .. })
    }

    /// `Cancelled` with nothing injected (every mode but `live_inject`).
    pub const CANCELLED: Self = Self::Cancelled { injected_chars: 0 };
}

/// Phase plus a session counter (incremented on every start) so late notifications can be told
/// apart from the current run — on both sides of the IPC boundary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DictationStatus {
    /// Current phase: `{ "phase": { "phase": "listening", "started_at": … }, "session": 3 }`.
    pub phase: DictationPhase,
    /// Session number of the run this status belongs to (`0` before the first start).
    pub session: u64,
    /// The run's context (docs/dictation.md §18.6): the application in front when it started and
    /// the matched scene. Set once the probe answered, cleared by the next start and by `Idle`;
    /// absent on shells without a probe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<TakeContext>,
    /// What the current (or last) take is for (docs/dictation.md §19); always on the wire, a
    /// status written before voice edit reads as `dictation`.
    #[serde(default)]
    pub kind: TakeKind,
    /// The paired phone the take's audio comes from (docs/dictation.md §20), by its name; absent
    /// for the local microphone. Stamped by the runtime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
}

impl DictationStatus {
    /// A dictation take's status without a context (the pre-§18 / §19 shape).
    pub fn dictation(phase: DictationPhase, session: u64) -> Self {
        Self { phase, session, context: None, kind: TakeKind::Dictation, remote: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_serializes_with_the_phase_tag() {
        let st = DictationStatus::dictation(DictationPhase::Listening { started_at: 5, ready: false, live: None, locked: false }, 3);
        let json = serde_json::to_string(&st).unwrap();
        assert_eq!(json, r#"{"phase":{"phase":"listening","started_at":5,"ready":false,"locked":false},"session":3,"kind":"dictation"}"#);
        let back: DictationStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, st);
        // docs/dictation.md §19: an edit take says so; a status written before it reads as dictation.
        let edit = DictationStatus { kind: TakeKind::Edit, ..st.clone() };
        assert!(serde_json::to_string(&edit).unwrap().ends_with(r#""session":3,"kind":"edit"}"#));
        let legacy: DictationStatus = serde_json::from_str(r#"{"phase":{"phase":"idle"},"session":2}"#).unwrap();
        assert_eq!(legacy.kind, TakeKind::Dictation);
        assert_eq!((TakeKind::Dictation.as_str(), TakeKind::Edit.as_str()), ("dictation", "edit"));
        assert_eq!(serde_json::from_str::<TakeKind>(r#""edit""#).unwrap(), TakeKind::Edit);
        // A `listening` written before §11 (no `ready`, no `live`) or before §13 (no `locked`) still parses.
        let legacy: DictationStatus = serde_json::from_str(r#"{"phase":{"phase":"listening","started_at":5},"session":3}"#).unwrap();
        assert_eq!(legacy, st);
        let live = LiveText {
            committed: vec![Segment { text: "你好。".into(), start_ms: 0, end_ms: 900 }, Segment { text: "hello world.".into(), start_ms: 900, end_ms: 2000 }],
            current: "今天".into(),
            degraded: None,
            injected: 0,
        };
        assert_eq!(live.preview(), "你好。hello world.今天");
        let listening = DictationPhase::Listening { started_at: 5, ready: true, live: Some(live.clone()), locked: false };
        let json = serde_json::to_string(&listening).unwrap();
        assert!(
            json.starts_with(r#"{"phase":"listening","started_at":5,"ready":true,"live":{"committed":[{"text":"你好。","start_ms":0,"end_ms":900}"#),
            "{json}"
        );
        assert!(json.ends_with(r#""locked":false}"#), "the lock flag is always on the wire: {json}");
        let locked: DictationPhase = serde_json::from_str(r#"{"phase":"listening","started_at":5,"ready":true,"locked":true}"#).unwrap();
        assert_eq!(locked, DictationPhase::Listening { started_at: 5, ready: true, live: None, locked: true });
        assert!(!json.contains("degraded"), "None is omitted: {json}");
        assert!(json.contains(r#""current":"今天","injected":0}"#), "the pasted count is always on the wire: {json}");
        assert_eq!(serde_json::from_str::<DictationPhase>(&json).unwrap(), listening);
        let legacy_live: LiveText = serde_json::from_str(r#"{"committed":[],"current":"a"}"#).unwrap();
        assert_eq!(legacy_live.injected, 0, "a live text written before §12 parses");
        let degraded = LiveText { degraded: Some("tap overrun".into()), ..live };
        assert!(serde_json::to_string(&degraded).unwrap().contains(r#""degraded":"tap overrun""#));
        assert_eq!(LiveText::default().preview(), "");
        assert_eq!(LiveText { current: "  a ".into(), ..LiveText::default() }.preview(), "a");
        let latin = LiveText { committed: vec![Segment { text: "Hello.".into(), start_ms: 0, end_ms: 1 }], current: "How are".into(), ..LiveText::default() };
        assert_eq!(latin.preview(), "Hello. How are", "Latin neighbours get a space");
        let mixed =
            LiveText { committed: vec![Segment { text: "你好，".into(), start_ms: 0, end_ms: 1 }], current: "Rust 很好".into(), ..LiveText::default() };
        assert_eq!(mixed.preview(), "你好，Rust 很好", "a CJK boundary joins without a space");
        let idle: DictationStatus = serde_json::from_str(r#"{"phase":{"phase":"idle"},"session":0,"kind":"dictation"}"#).unwrap();
        assert_eq!(idle, DictationStatus::default());
        assert!(!idle.phase.is_terminal());
        let done = DictationPhase::Done {
            text: "你好。".into(),
            raw_text: "你好".into(),
            chars: 3,
            via: Via::Paste,
            refined: true,
            duration_ms: 1200,
            asr_ms: 400,
            refine_ms: Some(300),
            refine_error: None,
            mode: OutputMode::WholeTake,
            segments: None,
            live_error: None,
        };
        assert!(done.is_terminal());
        let json = serde_json::to_string(&done).unwrap();
        assert!(json.starts_with(r#"{"phase":"done","text":"你好。""#), "{json}");
        assert!(!json.contains("refine_error"), "None is omitted on the wire: {json}");
        assert!(json.ends_with(r#""refine_ms":300,"mode":"whole_take"}"#), "segments / live_error are omitted when None: {json}");
        assert_eq!(serde_json::from_str::<DictationPhase>(&json).unwrap(), done);
        // A `done` written before §12 (no mode) reads as a whole take.
        let legacy: DictationPhase = serde_json::from_str(
            r#"{"phase":"done","text":"你好。","raw_text":"你好","chars":3,"via":"paste","refined":true,"duration_ms":1200,"asr_ms":400,"refine_ms":300}"#,
        )
        .unwrap();
        assert_eq!(legacy, done);
        let streamed = DictationPhase::Done {
            text: "你好。".into(),
            raw_text: "你好".into(),
            chars: 3,
            via: Via::Paste,
            refined: true,
            duration_ms: 1200,
            asr_ms: 400,
            refine_ms: Some(300),
            refine_error: None,
            mode: OutputMode::StreamingFinal,
            segments: Some(vec![Segment { text: "你好。".into(), start_ms: 0, end_ms: 900 }]),
            live_error: Some("flush: asr: x".into()),
        };
        let json = serde_json::to_string(&streamed).unwrap();
        assert!(json.contains(r#""mode":"streaming_final","segments":[{"text":"你好。","start_ms":0,"end_ms":900}],"live_error":"flush: asr: x""#), "{json}");
        assert_eq!(serde_json::from_str::<DictationPhase>(&json).unwrap(), streamed);
        let processing = DictationPhase::Processing { stage: ProcessingStage::Refining, started_at: 9, preview: None };
        assert_eq!(serde_json::to_string(&processing).unwrap(), r#"{"phase":"processing","stage":"refining","started_at":9}"#);
        let finalizing = DictationPhase::Processing { stage: ProcessingStage::Finalizing, started_at: 9, preview: Some("你好".into()) };
        assert_eq!(serde_json::to_string(&finalizing).unwrap(), r#"{"phase":"processing","stage":"finalizing","started_at":9,"preview":"你好"}"#);
        assert_eq!(serde_json::from_str::<ProcessingStage>(r#""finalizing""#).unwrap(), ProcessingStage::Finalizing);
        let legacy: DictationPhase = serde_json::from_str(r#"{"phase":"processing","stage":"refining","started_at":9}"#).unwrap();
        assert_eq!(legacy, processing);
        let previewing = DictationPhase::Processing { stage: ProcessingStage::Transcribing, started_at: 9, preview: Some("你好。今天".into()) };
        assert_eq!(serde_json::to_string(&previewing).unwrap(), r#"{"phase":"processing","stage":"transcribing","started_at":9,"preview":"你好。今天"}"#);
        assert_eq!(serde_json::from_str::<DictationPhase>(&serde_json::to_string(&previewing).unwrap()).unwrap(), previewing);
        let failed = DictationPhase::Failed { code: FailureCode::Asr, message: "x".into(), text: None };
        assert!(failed.is_terminal());
        assert_eq!(serde_json::to_string(&failed).unwrap(), r#"{"phase":"failed","code":"asr","message":"x"}"#);
        assert!(DictationPhase::CANCELLED.is_terminal());
        assert_eq!(serde_json::to_string(&DictationPhase::CANCELLED).unwrap(), r#"{"phase":"cancelled","injected_chars":0}"#);
        let legacy: DictationPhase = serde_json::from_str(r#"{"phase":"cancelled"}"#).unwrap();
        assert_eq!(legacy, DictationPhase::CANCELLED, "a cancelled written before §12 parses");
        let partial = DictationPhase::Cancelled { injected_chars: 7 };
        assert_eq!(serde_json::from_str::<DictationPhase>(&serde_json::to_string(&partial).unwrap()).unwrap(), partial);
    }

    /// docs/dictation.md §18.6: the context rides next to the phase once the probe answered; a
    /// status written before scenes (or on a shell without a probe) parses without it.
    #[test]
    fn the_take_context_is_optional_on_the_status() {
        use crate::scenes::{AppRef, SceneRef};
        let context =
            TakeContext { app: AppRef { id: "slack".into(), name: "Slack".into() }, scene: Some(SceneRef { id: uuid::Uuid::nil(), name: "聊天".into() }) };
        let st = DictationStatus {
            context: Some(context),
            ..DictationStatus::dictation(DictationPhase::Listening { started_at: 5, ready: true, live: None, locked: false }, 3)
        };
        let json = serde_json::to_string(&st).unwrap();
        assert!(
            json.ends_with(
                r#""session":3,"context":{"app":{"id":"slack","name":"Slack"},"scene":{"id":"00000000-0000-0000-0000-000000000000","name":"聊天"}},"kind":"dictation"}"#
            ),
            "{json}"
        );
        assert_eq!(serde_json::from_str::<DictationStatus>(&json).unwrap(), st);
        let plain = DictationStatus { context: None, ..st };
        assert!(!serde_json::to_string(&plain).unwrap().contains("context"), "None is omitted");
    }

    /// `live_inject` puts a separator after each sentence so the next one does not run into it:
    /// nothing after CJK text or punctuation, one space after Latin text (docs/dictation.md §12).
    #[test]
    fn inject_separator_follows_the_last_character() {
        assert_eq!(inject_separator("你好，世界。"), "");
        assert_eq!(inject_separator("今天天气"), "", "unpunctuated CJK joins without a space");
        assert_eq!(inject_separator("Hello world."), " ");
        assert_eq!(inject_separator("fetchUser"), " ");
        assert_eq!(inject_separator("Rust 很好"), "");
        assert_eq!(inject_separator("你好 Rust"), " ");
        assert_eq!(inject_separator("123"), " ");
        assert_eq!(inject_separator("  "), "");
        assert_eq!(inject_separator(""), "");
        assert_eq!(inject_separator("Hello. "), " ", "trailing whitespace is ignored when deciding");
    }

    /// Every error the ports can raise maps to a stable, snake_case code the webview localises.
    #[test]
    fn failure_codes_follow_the_error_kind_and_serialize_snake_case() {
        let cases = [
            (DictationError::NoSpeech, FailureCode::NoSpeech, "no_speech"),
            (DictationError::Audio("busy".into()), FailureCode::Audio, "audio"),
            (DictationError::Asr("401".into()), FailureCode::Asr, "asr"),
            (DictationError::Refine("429".into()), FailureCode::Refine, "refine"),
            (DictationError::Inject("denied".into()), FailureCode::Inject, "inject"),
            (DictationError::NoSelection, FailureCode::NoSelection, "no_selection"),
            (DictationError::SelectionTooLong(2400), FailureCode::SelectionTooLong, "selection_too_long"),
            (DictationError::Selection("no copy tool".into()), FailureCode::Selection, "selection"),
            (DictationError::EditUnavailable("no key".into()), FailureCode::EditUnavailable, "edit_unavailable"),
            (DictationError::EditInTerminal, FailureCode::EditInTerminal, "edit_in_terminal"),
            (DictationError::Busy, FailureCode::Unknown, "unknown"),
            (DictationError::Idle, FailureCode::Unknown, "unknown"),
        ];
        for (error, code, wire) in cases {
            assert_eq!(FailureCode::from(&error), code, "{error}");
            assert_eq!(serde_json::to_string(&code).unwrap(), format!("\"{wire}\""));
            assert_eq!(serde_json::from_str::<FailureCode>(&format!("\"{wire}\"")).unwrap(), code);
        }
        assert_eq!(FailureCode::default(), FailureCode::Unknown);
        let with_text: DictationPhase = serde_json::from_str(r#"{"phase":"failed","code":"inject","message":"inject: denied","text":"重要"}"#).unwrap();
        assert_eq!(with_text, DictationPhase::Failed { code: FailureCode::Inject, message: "inject: denied".into(), text: Some("重要".into()) });
    }
}
