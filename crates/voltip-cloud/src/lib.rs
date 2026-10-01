//! The cloud services as the core's ports (docs/dictation.md §3): speech-to-text through
//! `voltip-asr` behind [`Transcriber`], clean-up and the voice edit's rewrite through
//! `voltip-refine` behind [`Refiner`], the choice of client for a [`ResolvedEngines`], and the
//! provider probe behind [`ServiceProbe`]. The desktop adds its local models next to these
//! (`apps/desktop/src-tauri/src/dictation.rs`); the phone, which has none, uses them alone when it
//! recognises a take itself (§20.7).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use voltip_asr::{AsrClient, AsrConfig};
use voltip_core::dictation::{DictationError, RefineHints, Refined, Refiner, ServiceProbe, Transcriber, Transcript};
use voltip_core::{BuiltinPreset, ProbeError, ProbeFailure, ProviderId, ResolvedEngines, ServiceKind, TakePreset};
use voltip_refine::{PromptContext, PromptHints, RefineClient, RefineConfig};

/// HTTP request deadline for one transcription (long recordings on a slow link).
pub const ASR_TIMEOUT: Duration = Duration::from_secs(90);
/// HTTP request deadline for one refinement.
pub const REFINE_TIMEOUT: Duration = Duration::from_secs(30);
/// Deadline of one provider probe (the engines pane's 测试连接).
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Speech-to-text through [`voltip_asr::AsrClient`].
pub struct HttpTranscriber {
    client: AsrClient,
}

impl HttpTranscriber {
    /// Build the client; fails on an unusable configuration (bad URL, empty model).
    pub fn new(config: AsrConfig) -> Result<Self, DictationError> {
        Ok(Self { client: AsrClient::new(config).map_err(|e| DictationError::Asr(e.to_string()))? })
    }
}

#[async_trait]
impl Transcriber for HttpTranscriber {
    /// The glossary goes out as the OpenAI `prompt` field when there is one (docs/dictation.md §16.3).
    async fn transcribe(&self, wav: &[u8], language: Option<&str>, glossary: &[String]) -> Result<Transcript, DictationError> {
        let prompt = voltip_core::vocabulary::glossary_prompt(glossary);
        let t = self.client.transcribe_with_prompt(wav, language, prompt.as_deref()).await.map_err(|e| DictationError::Asr(e.to_string()))?;
        Ok(Transcript { text: t.text, latency_ms: t.latency_ms })
    }
}

/// Clean-up through [`voltip_refine::RefineClient`].
pub struct HttpRefiner {
    client: RefineClient,
}

impl HttpRefiner {
    /// Build the client (the take's language, style and context arrive with every request).
    pub fn new(config: RefineConfig) -> Result<Self, DictationError> {
        Ok(Self { client: RefineClient::new(config).map_err(|e| DictationError::Refine(e.to_string()))? })
    }
}

/// The built-in preset's own text (task, rules, examples; the output contract is added to every
/// preset): what 复制为自定义 starts from.
pub fn builtin_preset_body(preset: BuiltinPreset) -> &'static str {
    refine_preset(&TakePreset::Builtin(preset)).builtin_body().unwrap_or_default()
}

/// One built-in preset's own text as `presets_builtin` answers it.
#[derive(Clone, Debug, serde::Serialize)]
pub struct BuiltinPresetText {
    /// Which preset.
    pub id: BuiltinPreset,
    /// Its task, rules and examples ([`builtin_preset_body`]).
    pub prompt: &'static str,
}

/// Every built-in preset's text, in the order the interface lists them (`presets_builtin`; the
/// preview serves the same list from `packages/shared/src/fixtures/ipc/presets-builtin.json`).
pub fn builtin_preset_texts() -> Vec<BuiltinPresetText> {
    BuiltinPreset::ALL.into_iter().map(|id| BuiltinPresetText { id, prompt: builtin_preset_body(id) }).collect()
}

/// The take's preset as the refine crate names it (docs/dictation.md §21).
pub fn refine_preset(preset: &TakePreset) -> voltip_refine::Preset<'_> {
    use voltip_refine::Preset;
    match preset {
        TakePreset::Builtin(builtin) => match builtin {
            BuiltinPreset::Proofread => Preset::Proofread,
            BuiltinPreset::Prompt => Preset::Prompt,
            BuiltinPreset::Intent => Preset::Intent,
            BuiltinPreset::Chat => Preset::Chat,
            BuiltinPreset::Translate => Preset::Translate,
            BuiltinPreset::Notes => Preset::Notes,
            BuiltinPreset::Punctuation => Preset::Punctuation,
            BuiltinPreset::Formal => Preset::Formal,
        },
        TakePreset::Custom { prompt, .. } => Preset::Custom(prompt),
    }
}

/// The core's hints as the refine crate's prompt input, one-to-one (the core already filtered the
/// context by the privacy switches, docs/dictation.md §18.5).
pub fn prompt_hints(hints: &RefineHints) -> PromptHints<'_> {
    PromptHints {
        preset: refine_preset(&hints.preset),
        language: hints.language.as_deref(),
        glossary: &hints.glossary,
        context: PromptContext {
            app_name: hints.context.app_name.as_deref(),
            window_title: hints.context.window_title.as_deref(),
            instruction: hints.context.instruction.as_deref(),
        },
    }
}

#[async_trait]
impl Refiner for HttpRefiner {
    /// The glossary joins the system prompt as the user-dictionary block (docs/dictation.md §16.3),
    /// the take's context as the scene blocks (§18.5); the take's language and style shape the rest.
    async fn refine(&self, text: &str, hints: &RefineHints) -> Result<Refined, DictationError> {
        let r = self.client.refine_with(text, &prompt_hints(hints)).await.map_err(|e| DictationError::Refine(e.to_string()))?;
        Ok(Refined { text: r.text, latency_ms: r.latency_ms, model: r.model })
    }

    /// The voice edit's rewrite (docs/dictation.md §19) through the same client and the same hints
    /// (the edit prompt uses the glossary and the app block); a cut-off or empty answer is an
    /// error, so nothing is pasted.
    async fn edit(&self, selection: &str, instruction: &str, hints: &RefineHints) -> Result<Refined, DictationError> {
        let r = self.client.edit(selection, instruction, &prompt_hints(hints)).await.map_err(|e| DictationError::Refine(e.to_string()))?;
        Ok(Refined { text: r.text, latency_ms: r.latency_ms, model: r.model })
    }
}

/// Stands in for a client that could not be built: every call fails with the reason, so the
/// pill and the history say what is wrong instead of hanging.
pub struct Unconfigured(pub String);

#[async_trait]
impl Transcriber for Unconfigured {
    async fn transcribe(&self, _wav: &[u8], _language: Option<&str>, _glossary: &[String]) -> Result<Transcript, DictationError> {
        Err(DictationError::Asr(self.0.clone()))
    }
}

#[async_trait]
impl Refiner for Unconfigured {
    async fn refine(&self, _text: &str, _hints: &RefineHints) -> Result<Refined, DictationError> {
        Err(DictationError::Refine(self.0.clone()))
    }

    async fn edit(&self, _selection: &str, _instruction: &str, _hints: &RefineHints) -> Result<Refined, DictationError> {
        Err(DictationError::Refine(self.0.clone()))
    }
}

/// The recogniser of a configuration that recognises in the cloud. A provider that is not ready,
/// or whose configuration the client refuses, becomes an [`Unconfigured`] client that reports the
/// problem on use.
pub fn remote_transcriber(engines: &ResolvedEngines) -> Arc<dyn Transcriber> {
    match &engines.asr_remote {
        None => Arc::new(Unconfigured(match engines.asr_issue {
            Some(issue) => format!("识别服务未配置：{}", issue.message(ServiceKind::Asr)),
            None => "识别服务未配置".to_owned(),
        })),
        Some(remote) => match HttpTranscriber::new(AsrConfig::new(&remote.url, &remote.model).with_token(remote.key.clone()).with_timeout(ASR_TIMEOUT)) {
            Ok(t) => Arc::new(t),
            Err(e) => {
                tracing::warn!(error = %e, "ASR client not built");
                Arc::new(Unconfigured(format!("识别服务配置无效：{e}")))
            }
        },
    }
}

/// The clean-up client of a configuration; no ready clean-up provider means none (the core then
/// explains "润色未配置").
pub fn refiner(engines: &ResolvedEngines) -> Option<Arc<dyn Refiner>> {
    engines.refine.as_ref().map(|remote| {
        // The built-in service stays under its free tier's output limit; a service the user
        // configured may answer a long translation or notes in full (docs/dictation.md §21).
        let cap = if engines.llm_provider == Some(ProviderId::Builtin) { voltip_refine::BUILTIN_OUTPUT_CAP } else { voltip_refine::USER_OUTPUT_CAP };
        let config = RefineConfig::new(&remote.url, &remote.model).with_api_key(remote.key.clone()).with_timeout(REFINE_TIMEOUT).with_output_cap(cap);
        match HttpRefiner::new(config) {
            Ok(r) => Arc::new(r) as Arc<dyn Refiner>,
            Err(e) => {
                tracing::warn!(error = %e, "refine client not built");
                Arc::new(Unconfigured(format!("润色服务配置无效：{e}"))) as Arc<dyn Refiner>
            }
        }
    })
}

/// The engines pane's 测试连接 over HTTP (docs/dictation.md §3.3): `GET {base}/models` with the
/// key, answered with ids or a host-free failure. Recognition and clean-up endpoints are
/// normalised the same way (`…/v1`), so one request serves both kinds. Both shells use it.
#[derive(Clone, Copy, Debug, Default)]
pub struct HttpServiceProbe;

#[async_trait]
impl ServiceProbe for HttpServiceProbe {
    async fn list_models(&self, base_url: &str, key: Option<&str>) -> Result<Vec<String>, ProbeError> {
        voltip_refine::list_models(base_url, key, PROBE_TIMEOUT).await.map_err(|e| {
            use voltip_refine::RefineError as E;
            match e {
                E::InvalidConfig(_) => ProbeError::new(ProbeFailure::InvalidUrl),
                E::Unauthorized => ProbeError::new(ProbeFailure::Unauthorized),
                E::RateLimited { .. } => ProbeError { reason: ProbeFailure::HttpStatus, status: Some(429) },
                E::Server { status, .. } => ProbeError { reason: ProbeFailure::HttpStatus, status: Some(status) },
                E::Network(_) => ProbeError::new(ProbeFailure::Unreachable),
                E::Timeout => ProbeError::new(ProbeFailure::Timeout),
                _ => ProbeError::new(ProbeFailure::BadResponse),
            }
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use voltip_core::{BuiltIn, EngineSettings, ProviderSettings, UserSecrets};

    fn custom(asr_url: Option<&str>, llm_url: Option<&str>) -> ResolvedEngines {
        let provider = ProviderSettings {
            asr_url: asr_url.map(str::to_owned),
            asr_model: Some("whisper".into()),
            llm_url: llm_url.map(str::to_owned),
            llm_model: Some("m".into()),
        };
        let settings = EngineSettings {
            asr_provider: ProviderId::Custom,
            llm_provider: ProviderId::Custom,
            providers: [(ProviderId::Custom, provider)].into(),
            ..EngineSettings::default()
        };
        ResolvedEngines::resolve(&settings, &UserSecrets::default(), &BuiltIn::EMPTY)
    }

    #[tokio::test]
    async fn an_unset_service_answers_with_the_reason_instead_of_a_request() {
        let engines = custom(None, None);
        let err = remote_transcriber(&engines).transcribe(b"RIFF", None, &[]).await.unwrap_err();
        assert!(matches!(&err, DictationError::Asr(m) if m.starts_with("识别服务未配置")), "{err}");
        assert!(refiner(&engines).is_none(), "no clean-up provider, no refiner");
    }

    #[tokio::test]
    async fn a_url_the_client_refuses_is_reported_on_use() {
        let engines = custom(Some("not a url"), Some("ftp://nope"));
        let err = remote_transcriber(&engines).transcribe(b"RIFF", None, &[]).await.unwrap_err();
        assert!(matches!(&err, DictationError::Asr(m) if m.starts_with("识别服务配置无效")), "{err}");
        let err = refiner(&engines).expect("configured").refine("x", &RefineHints::default()).await.unwrap_err();
        assert!(matches!(&err, DictationError::Refine(m) if m.starts_with("润色服务配置无效")), "{err}");
    }

    #[test]
    fn every_builtin_preset_has_its_refine_counterpart() {
        for preset in BuiltinPreset::ALL {
            assert!(refine_preset(&TakePreset::Builtin(preset)).builtin_body().is_some(), "{preset:?}");
        }
    }
}
