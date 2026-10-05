//! The steps a recognised take's text goes through (docs/dictation.md §16.3, §17, §21, §22):
//! recognition brought to the take's Chinese script, the dictionary, the clean-up and the rules.
//! The engine's pipeline and the local speech service (§23) both run them, so the same audio with
//! the same settings gives the same text whichever of the two received it.

use super::RefineFailure;
use super::long;
use super::ports::{DictationError, RefineHints, Refiner, Transcriber};
use crate::engines::ChineseScript;
use crate::script::normalized;
use crate::vocabulary::{Step, Vocabulary};

/// Why a requested clean-up did not run: no clean-up service is configured.
pub const REFINE_UNCONFIGURED: &str = "润色未配置：缺少 API 密钥";
/// Why a clean-up's text was not used: the service answered with nothing.
pub const REFINE_EMPTY: &str = "润色返回空文本，已使用原文";
/// Why a clean-up's text was not used, when no explanation came with the failure.
pub const REFINE_FAILED: &str = "润色失败，已使用原文";

/// What recognition gave: the text in the take's script (trimmed), the round trip and the model
/// the client reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recognised {
    /// The recogniser's text, trimmed and in the take's script.
    pub text: String,
    /// Round trip of the request, milliseconds.
    pub asr_ms: u64,
    /// The model that recognised it, when the client says.
    pub model: Option<String>,
}

/// Recognise `wav` with the take's `language` hint and the vocabulary's `glossary`, and bring the
/// text to `script` (§17) before anything else touches it.
pub async fn recognize(
    transcriber: &dyn Transcriber,
    wav: &[u8],
    language: Option<&str>,
    glossary: &[String],
    script: ChineseScript,
) -> Result<Recognised, DictationError> {
    let t = transcriber.transcribe(wav, language, glossary).await?;
    Ok(Recognised { text: normalized(script, t.text.trim()), asr_ms: t.latency_ms, model: t.model })
}

/// Log a vocabulary step that fell back to its input (docs/dictation.md §16.3); the take goes on.
pub fn note_fallback(step: &Step, what: &str) {
    if let Some(reason) = &step.error {
        tracing::warn!(step = what, %reason, "vocabulary step fell back to the unmodified text");
    }
}

/// The dictionary on the recognised text (§16.3).
pub fn correct(vocabulary: &Vocabulary, text: &str) -> Step {
    let step = vocabulary.correct(text);
    note_fallback(&step, "dictionary");
    step
}

/// The replacement rules on the text about to leave (§16.3).
pub fn apply_rules(vocabulary: &Vocabulary, text: &str) -> Step {
    let step = vocabulary.apply_rules(text);
    note_fallback(&step, "rules");
    step
}

/// Whether the clean-up runs on a text, decided before it starts (the engine announces the
/// refining stage only for [`RefinePlan::Run`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefinePlan {
    /// Ask the refiner.
    Run,
    /// Leave the text as it is; `failure` says why when a clean-up was asked for.
    Skip {
        /// [`RefineFailure::TooLong`] or [`RefineFailure::Unconfigured`]; `None` when none was
        /// asked for.
        failure: Option<RefineFailure>,
    },
}

/// The plan for `text`: a long take's text past [`long::REFINE_MAX_CHARS`] is never refined
/// (§22); a requested clean-up without a refiner says it is unconfigured. `refiner_present` is
/// only ever true for a take that asked for the clean-up.
pub fn plan_refine(refiner_present: bool, requested: bool, long_take: bool, text: &str) -> RefinePlan {
    let too_long = long_take && text.chars().count() > long::REFINE_MAX_CHARS;
    if too_long {
        return RefinePlan::Skip { failure: requested.then_some(RefineFailure::TooLong) };
    }
    if refiner_present {
        return RefinePlan::Run;
    }
    RefinePlan::Skip { failure: requested.then_some(RefineFailure::Unconfigured) }
}

/// What the clean-up did to a text.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct CleanUp {
    /// The refined text, or the input when nothing usable came back.
    pub text: String,
    /// The refiner's text is the one in `text`.
    pub refined: bool,
    /// Round trip of the request, when one answered.
    pub refine_ms: Option<u64>,
    /// Why the input was kept.
    pub refine_error: Option<String>,
    /// The kind of `refine_error` (docs/dictation.md §3.6).
    pub refine_failure: Option<RefineFailure>,
    /// The model that answered with the text in `text`.
    pub refine_model: Option<String>,
}

impl CleanUp {
    /// `text` kept as it is, for `failure` (none: no clean-up was asked for); `refine_error` is the
    /// sentence that says so.
    pub fn skipped(text: &str, failure: Option<RefineFailure>) -> Self {
        let reason = |failure: RefineFailure| match failure {
            RefineFailure::TooLong => long::REFINE_SKIPPED,
            RefineFailure::Empty => REFINE_EMPTY,
            RefineFailure::Unconfigured => REFINE_UNCONFIGURED,
            // A failed request carries the service's own explanation (`run_refine`).
            RefineFailure::RateLimited | RefineFailure::Quota | RefineFailure::Failed => REFINE_FAILED,
        };
        Self { text: text.to_owned(), refine_error: failure.map(|f| reason(f).to_owned()), refine_failure: failure, ..Self::default() }
    }
}

/// Ask `refiner` to clean `text` (already corrected by the dictionary) up with `hints`. A failure
/// or an empty answer keeps `text`: the clean-up never stops a take (§2).
pub async fn run_refine(refiner: &dyn Refiner, text: &str, hints: &RefineHints) -> CleanUp {
    match refiner.refine(text, hints).await {
        Ok(out) => {
            let cleaned = out.text.trim();
            if cleaned.is_empty() {
                CleanUp { refine_ms: Some(out.latency_ms), ..CleanUp::skipped(text, Some(RefineFailure::Empty)) }
            } else {
                CleanUp {
                    text: cleaned.to_owned(),
                    refined: true,
                    refine_ms: Some(out.latency_ms),
                    refine_error: None,
                    refine_failure: None,
                    refine_model: Some(out.model),
                }
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "refine failed; keeping the raw transcript");
            CleanUp { text: text.to_owned(), refine_error: Some(e.to_string()), refine_failure: Some(RefineFailure::of(&e)), ..CleanUp::default() }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::dictation::fakes::{FakeRefiner, FakeTranscriber};

    #[tokio::test]
    async fn recognition_brings_the_text_to_the_script_before_anything_else() {
        let transcriber = FakeTranscriber::ok(" 這個測試 ");
        let wav = crate::dictation::wav::encode_pcm16(&[1000; 1600], 16_000);
        let simplified = recognize(&transcriber, &wav, Some("zh"), &[], ChineseScript::Simplified).await.unwrap();
        assert_eq!(simplified.text, "这个测试");
        let as_is = recognize(&transcriber, &wav, None, &[], ChineseScript::AsIs).await.unwrap();
        assert_eq!(as_is.text, "這個測試");
    }

    #[test]
    fn the_plan_follows_the_request_the_refiner_and_the_length() {
        assert_eq!(plan_refine(true, true, false, "短"), RefinePlan::Run);
        assert_eq!(plan_refine(false, true, false, "短"), RefinePlan::Skip { failure: Some(RefineFailure::Unconfigured) });
        assert_eq!(plan_refine(false, false, false, "短"), RefinePlan::Skip { failure: None });
        let long_text = "字".repeat(long::REFINE_MAX_CHARS + 1);
        assert_eq!(plan_refine(true, true, true, &long_text), RefinePlan::Skip { failure: Some(RefineFailure::TooLong) });
        assert_eq!(plan_refine(false, true, true, &long_text), RefinePlan::Skip { failure: Some(RefineFailure::TooLong) });
        assert_eq!(plan_refine(true, true, false, &long_text), RefinePlan::Run, "only a long take has the limit");
    }

    #[tokio::test]
    async fn a_failed_or_empty_clean_up_keeps_the_text() {
        let hints = RefineHints::default();
        let ok = run_refine(&FakeRefiner::ok("整理好了。"), "整理好了", &hints).await;
        assert!(ok.refined && ok.text == "整理好了。" && ok.refine_model.is_some() && ok.refine_error.is_none());
        let empty = run_refine(&FakeRefiner::ok("  "), "原文", &hints).await;
        assert_eq!((empty.text.as_str(), empty.refined, empty.refine_error.as_deref()), ("原文", false, Some(REFINE_EMPTY)));
        let failed = run_refine(&FakeRefiner::err("down"), "原文", &hints).await;
        assert_eq!((failed.text.as_str(), failed.refined, failed.refine_ms), ("原文", false, None));
        assert!(failed.refine_error.is_some());
        // docs/dictation.md §3.6: each kept text says why by kind, the detail stays the sentence.
        assert_eq!((ok.refine_failure, empty.refine_failure, failed.refine_failure), (None, Some(RefineFailure::Empty), Some(RefineFailure::Failed)));
        let busy = run_refine(&FakeRefiner::rate_limited(), "原文", &hints).await;
        assert_eq!((busy.text.as_str(), busy.refined, busy.refine_failure), ("原文", false, Some(RefineFailure::RateLimited)));
        assert!(busy.refine_error.as_deref().is_some_and(|e| e.starts_with("请求过于频繁")), "{busy:?}");
        let quota = run_refine(&FakeRefiner::quota(), "原文", &hints).await;
        assert_eq!(quota.refine_failure, Some(RefineFailure::Quota));
        let skipped = CleanUp::skipped("原文", Some(RefineFailure::TooLong));
        assert_eq!((skipped.refine_error.as_deref(), skipped.refine_failure), (Some(long::REFINE_SKIPPED), Some(RefineFailure::TooLong)));
        let unconfigured = CleanUp::skipped("原文", Some(RefineFailure::Unconfigured));
        assert_eq!(unconfigured.refine_error.as_deref(), Some(REFINE_UNCONFIGURED));
        assert_eq!(CleanUp::skipped("原文", Some(RefineFailure::Failed)).refine_error.as_deref(), Some(REFINE_FAILED));
        assert_eq!(CleanUp::skipped("原文", None), CleanUp { text: "原文".into(), ..CleanUp::default() });
    }
}
