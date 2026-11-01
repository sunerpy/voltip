//! 用最近一条录音试一下 (docs/dictation.md §30.5): the newest kept recording recognised — and
//! cleaned up when the settings say so — with the clients the next take would use, then its text,
//! who wrote it, how long it took and whether the glossary went out shown on the recognition
//! settings. Nothing goes to the clipboard or into the history, so the statistics (the time saved)
//! never see it; the built-in service still counts the requests against the day's quota.

use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::dictation::{DictationError, FailureCode, RefineFailure, RefineHints, Refiner, SwitchReason, Transcriber, steps};
use crate::engines::ChineseScript;
use crate::providers::ProviderId;
use crate::usage::{AsrCall, UsageOrigin};
use crate::vocabulary::Vocabulary;

/// Why a 试识别 is refused: the last one has not ended.
pub const TRIAL_BUSY: &str = "上一次试识别还没有结束";

/// `UiState.recognition_trial`: the last 试识别, or the one running.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum TrialView {
    /// The recording is being recognised.
    Running {
        /// Its id in the audio cache.
        recording: Uuid,
        /// When the trial started, Unix milliseconds.
        started_at: u64,
    },
    /// It was recognised.
    Done(Box<TrialResult>),
    /// It could not be.
    Failed {
        /// The recording it ran on, when there was one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        recording: Option<Uuid>,
        /// Why.
        failure: TrialFailure,
    },
}

/// What a 试识别 gave.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrialResult {
    /// The recording's id in the audio cache.
    pub recording: Uuid,
    /// When it was recorded, Unix milliseconds.
    pub recorded_at_ms: u64,
    /// Its length.
    pub duration_ms: u64,
    /// The text a take would deliver: the recognition's, corrected by the dictionary, cleaned up
    /// when the settings say so, through the rules.
    pub text: String,
    /// The recognition's text.
    pub raw_text: String,
    /// Whose model recognised it: a fallback model's too (docs/dictation.md §3.14).
    pub asr_provider: ProviderId,
    /// The model.
    pub asr_model: String,
    /// Why a fallback model recognised it; absent when the selected one did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asr_fallback: Option<SwitchReason>,
    /// The recognition's round trip.
    pub asr_ms: u64,
    /// Whether the request carried the glossary (§30.6).
    pub glossary_sent: bool,
    /// The clean-up's text was used.
    pub refined: bool,
    /// Whose model cleaned it up, when one did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refine_provider: Option<ProviderId>,
    /// That model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refine_model: Option<String>,
    /// The clean-up's round trip, when it ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refine_ms: Option<u64>,
    /// Why a clean-up that was asked for was not used (§3.6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refine_failure: Option<RefineFailure>,
    /// The take would run as 边说边输入 (`live_inject`, §12), which never cleans up: the clean-up the
    /// settings ask for did not run.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub live_inject: bool,
    /// From the start of the trial to its text.
    pub elapsed_ms: u64,
}

/// Why a 试识别 gave no text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TrialFailure {
    /// The audio cache holds no recording to try: it is off, empty, or holds long takes only.
    NoRecording,
    /// The recognition failed, as a take's would (`FailureCode`), with the service's words.
    Failed {
        /// The kind, as a take's failure says it.
        code: FailureCode,
        /// The explanation.
        message: String,
    },
}

/// One 试识别: the recording and what to recognise it with.
pub struct TrialJob {
    /// The recording's id.
    pub recording: Uuid,
    /// When it was recorded.
    pub recorded_at_ms: u64,
    /// Its length.
    pub duration_ms: u64,
    /// Its WAV.
    pub wav: Vec<u8>,
    /// The recogniser the next take would use (behind its fallback chain).
    pub transcriber: Arc<dyn Transcriber>,
    /// The clean-up the next take would use, when one is set up.
    pub refiner: Option<Arc<dyn Refiner>>,
    /// Whether the settings clean takes up.
    pub refine: bool,
    /// The take would run as 边说边输入, so the clean-up the settings ask for does not run (§12).
    pub live_inject: bool,
    /// The language hint.
    pub language: Option<String>,
    /// The script the text is brought to.
    pub script: ChineseScript,
    /// The dictionary and the rules.
    pub vocabulary: Arc<Vocabulary>,
    /// What the clean-up is told besides the text.
    pub hints: RefineHints,
    /// The provider and model the settings select, named when the client says none.
    pub asr_provider: ProviderId,
    /// The recognition model the settings select.
    pub asr_model: String,
    /// The clean-up provider the settings select.
    pub refine_provider: Option<ProviderId>,
}

/// Recognise the job's recording as a take would, stopping before the delivery.
pub async fn run(job: TrialJob) -> TrialView {
    let started = Instant::now();
    let call = AsrCall::take(Some(UsageOrigin::Retake));
    let recognised = match steps::recognize(job.transcriber.as_ref(), &job.wav, job.language.as_deref(), job.vocabulary.glossary(), job.script, &call).await {
        Ok(recognised) => recognised,
        Err(e) => {
            return TrialView::Failed { recording: Some(job.recording), failure: TrialFailure::Failed { code: FailureCode::from(&e), message: e.to_string() } };
        }
    };
    if recognised.text.is_empty() {
        let e = DictationError::NoSpeech;
        return TrialView::Failed { recording: Some(job.recording), failure: TrialFailure::Failed { code: FailureCode::from(&e), message: e.to_string() } };
    }
    let corrected = steps::correct(&job.vocabulary, &recognised.text);
    let cleaned = match (job.refine && !job.live_inject, &job.refiner) {
        (true, Some(refiner)) => steps::run_refine(refiner.as_ref(), &corrected.text, &job.hints).await,
        (true, None) => steps::CleanUp::skipped(&corrected.text, Some(RefineFailure::Unconfigured)),
        (false, _) => steps::CleanUp::skipped(&corrected.text, None),
    };
    let text = steps::apply_rules(&job.vocabulary, &cleaned.text).text;
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    TrialView::Done(Box::new(TrialResult {
        recording: job.recording,
        recorded_at_ms: job.recorded_at_ms,
        duration_ms: job.duration_ms,
        text,
        raw_text: recognised.text,
        asr_provider: recognised.provider.unwrap_or(job.asr_provider),
        asr_model: recognised.model.unwrap_or(job.asr_model),
        asr_fallback: recognised.fallback,
        asr_ms: recognised.asr_ms,
        glossary_sent: recognised.glossary_sent,
        refined: cleaned.refined,
        refine_provider: if cleaned.refined { cleaned.refine_provider.or(job.refine_provider) } else { None },
        refine_model: cleaned.refine_model.filter(|_| cleaned.refined),
        refine_ms: cleaned.refine_ms,
        refine_failure: cleaned.refine_failure,
        live_inject: job.refine && job.live_inject,
        elapsed_ms,
    }))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::dictation::fakes::{FAKE_REFINE_MODEL, FakeRefiner, FakeTranscriber};
    use crate::dictation::wav;
    use crate::vocabulary::{DictionaryEntry, EntrySource};

    fn job(transcriber: FakeTranscriber, refiner: Option<FakeRefiner>, refine: bool, vocabulary: Vocabulary) -> TrialJob {
        TrialJob {
            recording: Uuid::nil(),
            recorded_at_ms: 1_758_700_000_000,
            duration_ms: 3000,
            wav: wav::encode_pcm16(&[1000; 16_000], 16_000),
            transcriber: Arc::new(transcriber),
            refiner: refiner.map(|r| Arc::new(r) as Arc<dyn Refiner>),
            refine,
            live_inject: false,
            language: Some("zh".into()),
            script: ChineseScript::Simplified,
            vocabulary: Arc::new(vocabulary),
            hints: RefineHints::default(),
            asr_provider: ProviderId::Custom,
            asr_model: "whisper-1".into(),
            refine_provider: Some(ProviderId::Groq),
        }
    }

    fn voltip() -> Vocabulary {
        let entry = DictionaryEntry {
            id: Uuid::new_v4(),
            term: "Voltip".into(),
            heard_as: vec!["沃提普".into()],
            enabled: true,
            source: EntrySource::Manual,
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        Vocabulary::compile(&[entry], &[])
    }

    /// docs/dictation.md §30.5: the text a take would deliver, who wrote it and whether the glossary
    /// went out; the clean-up runs when the settings say so.
    #[tokio::test]
    async fn a_trial_recognises_and_cleans_up_as_a_take_would() {
        let view = run(job(FakeTranscriber::ok("沃提普好用"), Some(FakeRefiner::ok("Voltip 好用。")), true, voltip())).await;
        let TrialView::Done(result) = view else { panic!("{view:?}") };
        assert_eq!((result.raw_text.as_str(), result.text.as_str()), ("沃提普好用", "Voltip 好用。"));
        assert_eq!((result.asr_provider, result.asr_model.as_str()), (ProviderId::Custom, "whisper-1"), "the settings' names when the client says none");
        assert!(result.glossary_sent && result.refined);
        assert_eq!((result.refine_provider, result.refine_model.as_deref()), (Some(ProviderId::Groq), Some(FAKE_REFINE_MODEL)));
        assert_eq!(result.recording, Uuid::nil());
        // No clean-up asked for: the dictionary's text.
        let view = run(job(FakeTranscriber::ok("沃提普好用").ignoring_glossary(), Some(FakeRefiner::ok("x")), false, voltip())).await;
        let TrialView::Done(result) = view else { panic!("{view:?}") };
        assert_eq!(result.text, "Voltip好用");
        assert!(!result.glossary_sent && !result.refined && result.refine_failure.is_none());
        // Asked for without a service: the kind says so.
        let view = run(job(FakeTranscriber::ok("好"), None, true, Vocabulary::default())).await;
        assert!(matches!(&view, TrialView::Done(r) if r.refine_failure == Some(RefineFailure::Unconfigured)), "{view:?}");
    }

    /// A recognition that fails, or hears nothing, says why as a take's failure would.
    #[tokio::test]
    async fn a_failed_trial_says_why() {
        let view = run(job(FakeTranscriber::err("503 service unavailable"), None, false, Vocabulary::default())).await;
        assert!(
            matches!(&view, TrialView::Failed { recording: Some(_), failure: TrialFailure::Failed { code: FailureCode::Asr, message } } if message.contains("503")),
            "{view:?}"
        );
        let view = run(job(FakeTranscriber::ok("  "), None, false, Vocabulary::default())).await;
        assert!(matches!(&view, TrialView::Failed { failure: TrialFailure::Failed { code: FailureCode::NoSpeech, .. }, .. }), "{view:?}");
    }

    #[test]
    fn the_wire_names() {
        let running = TrialView::Running { recording: Uuid::nil(), started_at: 5 };
        assert_eq!(serde_json::to_value(&running).unwrap(), serde_json::json!({ "state": "running", "recording": Uuid::nil(), "started_at": 5 }));
        let none = TrialView::Failed { recording: None, failure: TrialFailure::NoRecording };
        assert_eq!(serde_json::to_value(&none).unwrap(), serde_json::json!({ "state": "failed", "failure": { "kind": "no_recording" } }));
    }
}
