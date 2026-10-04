//! The cloud services as the core's ports (docs/dictation.md §3): speech-to-text through
//! `voltip-asr` behind [`Transcriber`] (Alibaba Cloud Model Studio's models through
//! [`dashscope`], its realtime ones streaming as well), clean-up and the voice edit's rewrite
//! through `voltip-refine` behind [`Refiner`], the choice of client for a [`ResolvedEngines`], and
//! the provider probe behind [`ServiceProbe`]. The desktop adds its local models next to these
//! (`apps/desktop/src-tauri/src/dictation.rs`); the phone, which has none, uses them alone when it
//! recognises a take itself (§20.7). [`feedback`] is the in-app feedback client both shells use.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod dashscope;
pub mod feedback;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use voltip_asr::{AsrClient, AsrConfig, DashscopeMode};
use voltip_core::dictation::{DictationError, RefineHints, Refined, Refiner, ServiceProbe, Transcriber, Transcript};
use voltip_core::providers::is_dashscope;
use voltip_core::{AsrProtocol, BuiltinPreset, ProbeError, ProbeFailure, ProviderId, RemoteService, ResolvedEngines, ServiceKind, TakePreset};

use crate::dashscope::DashscopeTranscriber;
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
        Ok(Self { client: AsrClient::new(config).map_err(asr_error)? })
    }
}

#[async_trait]
impl Transcriber for HttpTranscriber {
    /// The glossary goes out as the OpenAI `prompt` field when there is one (docs/dictation.md §16.3).
    async fn transcribe(&self, wav: &[u8], language: Option<&str>, glossary: &[String]) -> Result<Transcript, DictationError> {
        let prompt = voltip_core::vocabulary::glossary_prompt(glossary);
        let t = self.client.transcribe_with_prompt(wav, language, prompt.as_deref()).await.map_err(asr_error)?;
        Ok(Transcript { text: t.text, latency_ms: t.latency_ms, model: Some(t.model) })
    }
}

/// Clean-up through [`voltip_refine::RefineClient`].
pub struct HttpRefiner {
    client: RefineClient,
}

impl HttpRefiner {
    /// Build the client (the take's language, style and context arrive with every request).
    pub fn new(config: RefineConfig) -> Result<Self, DictationError> {
        Ok(Self { client: RefineClient::new(config).map_err(refine_error)? })
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
        let r = self.client.refine_with(text, &prompt_hints(hints)).await.map_err(refine_error)?;
        Ok(Refined { text: r.text, latency_ms: r.latency_ms, model: r.model })
    }

    /// The voice edit's rewrite (docs/dictation.md §19) through the same client and the same hints
    /// (the edit prompt uses the glossary and the app block); a cut-off or empty answer is an
    /// error, so nothing is pasted.
    async fn edit(&self, selection: &str, instruction: &str, hints: &RefineHints) -> Result<Refined, DictationError> {
        let r = self.client.edit(selection, instruction, &prompt_hints(hints)).await.map_err(refine_error)?;
        Ok(Refined { text: r.text, latency_ms: r.latency_ms, model: r.model })
    }
}

/// The core's error for a failed recognition: a used-up quota keeps its kind, so a fallback model
/// list moves on to its next model (docs/dictation.md §3.5); anything else is `Asr` with the reason.
pub fn asr_error(error: voltip_asr::AsrError) -> DictationError {
    if matches!(error, voltip_asr::AsrError::QuotaExhausted { .. }) {
        return DictationError::QuotaExhausted { service: ServiceKind::Asr, detail: error.to_string() };
    }
    DictationError::Asr(error.to_string())
}

/// The core's error for a failed clean-up or edit: a used-up quota keeps its kind (docs/dictation.md
/// §3.5); anything else is `Refine` with the reason.
pub fn refine_error(error: voltip_refine::RefineError) -> DictationError {
    if matches!(error, voltip_refine::RefineError::QuotaExhausted { .. }) {
        return DictationError::QuotaExhausted { service: ServiceKind::Llm, detail: error.to_string() };
    }
    DictationError::Refine(error.to_string())
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

/// The HTTP client builder with the trust roots of this platform, for the shells' own requests
/// (feedback, the phone's update check). Elsewhere reqwest verifies with the system's store
/// (rustls-platform-verifier); on Android that verifier needs a JNI context the app never hands it
/// and panics on the first request, so there the requests trust Mozilla's root store, as
/// voltip-asr, voltip-refine and the relay connection do.
pub fn http_client_builder() -> reqwest::ClientBuilder {
    let builder = reqwest::Client::builder();
    #[cfg(target_os = "android")]
    let builder = builder.tls_certs_only(mozilla_roots());
    builder
}

/// Mozilla's root store (webpki-root-certs) as reqwest certificates.
#[cfg(any(target_os = "android", test))]
fn mozilla_roots() -> Vec<reqwest::Certificate> {
    webpki_root_certs::TLS_SERVER_ROOT_CERTS.iter().filter_map(|der| reqwest::Certificate::from_der(der.as_ref()).ok()).collect()
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
        Some(remote) => transcriber_for(AsrProtocol::of(&remote.url, &remote.model), remote),
    }
}

/// The client that speaks `protocol` to `remote` (docs/dictation.md §3.4): OpenAI's
/// `/audio/transcriptions`, or one of Model Studio's; a Model Studio model dictation cannot use
/// says so on use.
pub fn transcriber_for(protocol: AsrProtocol, remote: &RemoteService) -> Arc<dyn Transcriber> {
    let config = AsrConfig::new(&remote.url, &remote.model).with_token(remote.key.clone()).with_timeout(ASR_TIMEOUT);
    let built = match protocol {
        AsrProtocol::OpenaiTranscriptions => HttpTranscriber::new(config).map(|t| Arc::new(t) as Arc<dyn Transcriber>),
        AsrProtocol::DashscopeChat => DashscopeTranscriber::new(config, DashscopeMode::Chat).map(|t| Arc::new(t) as Arc<dyn Transcriber>),
        AsrProtocol::DashscopeMultimodal => DashscopeTranscriber::new(config, DashscopeMode::Multimodal).map(|t| Arc::new(t) as Arc<dyn Transcriber>),
        AsrProtocol::DashscopeDuplex => DashscopeTranscriber::new(config, DashscopeMode::Duplex).map(|t| Arc::new(t) as Arc<dyn Transcriber>),
        AsrProtocol::DashscopeUnsupported => return Arc::new(Unconfigured(unsupported_model(&remote.model))),
    };
    built.unwrap_or_else(|e| {
        tracing::warn!(error = %e, ?protocol, "ASR client not built");
        Arc::new(Unconfigured(format!("识别服务配置无效：{e}")))
    })
}

/// Why a Model Studio model cannot dictate, and what to pick instead.
fn unsupported_model(model: &str) -> String {
    let instead = "请改用 qwen-audio-3.1-asr-flash-streaming 等实时识别模型，或 qwen-audio-3.1-asr-flash";
    if model.trim().to_ascii_lowercase().starts_with("qwen3-asr-flash-realtime") {
        format!("识别服务配置无效：暂不支持 {model} 的实时接口；{instead}")
    } else {
        format!("识别服务配置无效：{model} 只能在后台转写录音文件，不能用于听写；{instead}")
    }
}

/// The OpenAI-compatible base of a Model Studio address that names no `compatible-mode` path (a
/// workspace's bare host, its `/api/v1`): where its chat models and its model list are. Any other
/// address is used as it is.
pub fn openai_compatible_base(base_url: &str) -> String {
    if is_dashscope(base_url)
        && !base_url.contains("/compatible-mode")
        && let Ok(base) = voltip_asr::compatible_base(base_url)
    {
        return base;
    }
    base_url.to_owned()
}

/// The clean-up client of a configuration; no ready clean-up provider means none (the core then
/// explains "润色未配置").
pub fn refiner(engines: &ResolvedEngines) -> Option<Arc<dyn Refiner>> {
    engines.refine.as_ref().map(|remote| {
        let config = refine_config(remote, engines.llm_provider == Some(ProviderId::Builtin));
        match HttpRefiner::new(config) {
            Ok(r) => Arc::new(r) as Arc<dyn Refiner>,
            Err(e) => {
                tracing::warn!(error = %e, "refine client not built");
                Arc::new(Unconfigured(format!("润色服务配置无效：{e}"))) as Arc<dyn Refiner>
            }
        }
    })
}

/// How the clean-up client reaches `remote`. The built-in service stays under its free tier's
/// output limit; a service the user configured may answer a long translation or notes in full
/// (docs/dictation.md §21). A Model Studio address gets its OpenAI-compatible base and
/// `enable_thinking: false` (§3.4): its Qwen3 and DeepSeek models think by default there.
pub fn refine_config(remote: &RemoteService, builtin: bool) -> RefineConfig {
    let cap = if builtin { voltip_refine::BUILTIN_OUTPUT_CAP } else { voltip_refine::USER_OUTPUT_CAP };
    RefineConfig::new(openai_compatible_base(&remote.url), &remote.model)
        .with_api_key(remote.key.clone())
        .with_timeout(REFINE_TIMEOUT)
        .with_output_cap(cap)
        .with_enable_thinking(is_dashscope(&remote.url).then_some(false))
}

/// The engines pane's 测试连接 over HTTP (docs/dictation.md §3.3): `GET {base}/models` with the
/// key, answered with ids or a host-free failure. Recognition and clean-up endpoints are
/// normalised the same way (`…/v1`), so one request serves both kinds; a Model Studio address
/// is asked at its OpenAI-compatible base (§3.4). Both shells use it.
#[derive(Clone, Copy, Debug, Default)]
pub struct HttpServiceProbe;

#[async_trait]
impl ServiceProbe for HttpServiceProbe {
    async fn list_models(&self, base_url: &str, key: Option<&str>) -> Result<Vec<String>, ProbeError> {
        voltip_refine::list_models(&openai_compatible_base(base_url), key, PROBE_TIMEOUT).await.map_err(|e| {
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

    /// Regression (2026-10-03, the goal gate on the phone's update check): on Android reqwest's
    /// platform verifier panics on the first request ("Expect rustls-platform-verifier to be
    /// initialized"). The shells' own requests take this builder, which trusts Mozilla's roots
    /// there; the store loads in full and makes a client.
    #[test]
    fn regression_the_shells_own_requests_trust_mozillas_roots_on_android() {
        let roots = mozilla_roots();
        assert_eq!(roots.len(), webpki_root_certs::TLS_SERVER_ROOT_CERTS.len(), "every root parses");
        assert!(roots.len() > 100, "{} roots", roots.len());
        assert!(reqwest::Client::builder().tls_certs_only(roots).build().is_ok());
        assert!(http_client_builder().build().is_ok());
    }

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

    /// docs/dictation.md §3.4 (goal 2026-10-03: every Model Studio model failed on
    /// `/audio/transcriptions`): the endpoint and the model pick the client — a realtime model's
    /// streams, the HTTP ones do not, a file-only model says what to use instead, and everything
    /// else keeps OpenAI's multipart.
    #[tokio::test]
    async fn the_protocol_picks_the_client() {
        let remote = |url: &str, model: &str| RemoteService { url: url.into(), model: model.into(), key: Some("sk-test".into()) };
        let studio = "https://dashscope.aliyuncs.com/compatible-mode/v1";
        let pick = |url: &str, model: &str| transcriber_for(AsrProtocol::of(url, model), &remote(url, model));
        assert!(pick(studio, "qwen-audio-3.1-asr-flash-streaming").streaming(&[]).is_some());
        assert!(pick(studio, "qwen-audio-3.1-asr-flash-message").streaming(&["Voltip".into()]).is_some());
        assert!(pick(studio, "qwen-audio-3.1-asr-flash").streaming(&[]).is_none());
        assert!(pick(studio, "qwen3-asr-flash").streaming(&[]).is_none());
        assert!(pick("https://api.openai.com/v1", "whisper-1").streaming(&[]).is_none());
        for (model, says) in
            [("qwen-audio-3.1-asr-flash-filetrans", "后台转写录音文件"), ("paraformer-v2", "后台转写录音文件"), ("qwen3-asr-flash-realtime", "暂不支持")]
        {
            let err = pick(studio, model).transcribe(b"RIFF", None, &[]).await.unwrap_err();
            assert!(
                matches!(&err, DictationError::Asr(m) if m.contains(says) && m.contains(model) && m.contains("qwen-audio-3.1-asr-flash")),
                "{model}: {err}"
            );
        }
        // A configuration the client refuses is reported on use, whatever the protocol.
        for protocol in [AsrProtocol::DashscopeChat, AsrProtocol::DashscopeMultimodal, AsrProtocol::DashscopeDuplex, AsrProtocol::OpenaiTranscriptions] {
            let err = transcriber_for(protocol, &remote("not a url", "m")).transcribe(b"RIFF", None, &[]).await.unwrap_err();
            assert!(matches!(&err, DictationError::Asr(m) if m.starts_with("识别服务配置无效")), "{protocol:?}: {err}");
        }
        // Through the resolution: the Model Studio vendor with a key.
        let mut secrets = UserSecrets::default();
        secrets.set(ProviderId::Aliyun, ServiceKind::Asr, Some("sk-test".into()));
        let settings = EngineSettings { asr_provider: ProviderId::Aliyun, ..EngineSettings::default() };
        let engines = ResolvedEngines::resolve(&settings, &secrets, &BuiltIn::EMPTY);
        assert!(remote_transcriber(&engines).streaming(&[]).is_some(), "the catalogue's default model streams");
    }

    /// docs/dictation.md §3.5: a used-up quota reaches the core as its own kind, which a fallback
    /// model list moves on for; every other failure keeps its service's kind; a transcript and a
    /// clean-up name the model that answered.
    #[tokio::test]
    async fn a_used_up_quota_keeps_its_kind_and_answers_name_their_model() {
        use voltip_asr::AsrError;
        use voltip_refine::RefineError;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let asr = asr_error(AsrError::QuotaExhausted { code: "AllocationQuota.FreeTierOnly".into(), message: "free tier exhausted".into() });
        assert!(matches!(&asr, DictationError::QuotaExhausted { service: ServiceKind::Asr, detail } if detail.contains("FreeTierOnly")), "{asr:?}");
        assert!(asr.is_quota_exhausted());
        assert_eq!(asr_error(AsrError::Unauthorized), DictationError::Asr("ASR rejected the credentials".into()));
        let llm = refine_error(RefineError::QuotaExhausted { code: "insufficient_quota".into(), message: "m".into() });
        assert!(matches!(&llm, DictationError::QuotaExhausted { service: ServiceKind::Llm, .. }), "{llm:?}");
        assert_eq!(refine_error(RefineError::Timeout), DictationError::Refine("refine request timed out".into()));

        let used_up = serde_json::json!({ "error": { "code": "insufficient_quota", "message": "You exceeded your current quota" } });
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/transcriptions"))
            .respond_with(ResponseTemplate::new(429).set_body_json(used_up.clone()))
            .mount(&server)
            .await;
        Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(ResponseTemplate::new(403).set_body_json(used_up)).mount(&server).await;
        let remote = RemoteService { url: server.uri(), model: "whisper-1".into(), key: Some("sk-test".into()) };
        let err = transcriber_for(AsrProtocol::OpenaiTranscriptions, &remote).transcribe(b"RIFF", None, &[]).await.unwrap_err();
        assert!(matches!(&err, DictationError::QuotaExhausted { service: ServiceKind::Asr, .. }), "{err:?}");
        let refiner = HttpRefiner::new(refine_config(&remote, false)).unwrap();
        let err = refiner.refine("x", &RefineHints::default()).await.unwrap_err();
        assert!(matches!(&err, DictationError::QuotaExhausted { service: ServiceKind::Llm, .. }), "{err:?}");

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/transcriptions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"text":"你好"}"#))
            .mount(&server)
            .await;
        let remote = RemoteService { url: server.uri(), model: "whisper-1".into(), key: None };
        let t = transcriber_for(AsrProtocol::OpenaiTranscriptions, &remote).transcribe(b"RIFF", None, &[]).await.unwrap();
        assert_eq!((t.text.as_str(), t.model.as_deref()), ("你好", Some("whisper-1")));
    }

    /// Regression (2026-10-04): a Model Studio clean-up runs without thinking, at the compatible
    /// base; every other service is asked as before.
    #[test]
    fn regression_a_model_studio_clean_up_does_not_think() {
        let remote = |url: &str| RemoteService { url: url.into(), model: "qwen3.8-flash".into(), key: Some("sk-test".into()) };
        let studio = refine_config(&remote("https://ws-1.cn-beijing.maas.aliyuncs.com"), false);
        assert_eq!(studio.enable_thinking, Some(false));
        assert_eq!(studio.base_url, "https://ws-1.cn-beijing.maas.aliyuncs.com/compatible-mode/v1");
        assert_eq!(studio.output_cap, voltip_refine::USER_OUTPUT_CAP);
        let other = refine_config(&remote("https://api.groq.com/openai/v1"), false);
        assert_eq!((other.enable_thinking, other.base_url.as_str()), (None, "https://api.groq.com/openai/v1"));
        assert_eq!(refine_config(&remote("https://llm.builtin.test/v1"), true).output_cap, voltip_refine::BUILTIN_OUTPUT_CAP);
    }

    #[test]
    fn a_model_studio_address_is_asked_at_its_compatible_base() {
        assert_eq!(openai_compatible_base("https://ws-1.cn-beijing.maas.aliyuncs.com"), "https://ws-1.cn-beijing.maas.aliyuncs.com/compatible-mode/v1");
        assert_eq!(openai_compatible_base("https://dashscope.aliyuncs.com/api/v1"), "https://dashscope.aliyuncs.com/compatible-mode/v1");
        for kept in ["https://dashscope.aliyuncs.com/compatible-mode/v1", "https://api.openai.com/v1", "http://127.0.0.1:8000", "not a url"] {
            assert_eq!(openai_compatible_base(kept), kept);
        }
    }

    #[test]
    fn every_builtin_preset_has_its_refine_counterpart() {
        for preset in BuiltinPreset::ALL {
            assert!(refine_preset(&TakePreset::Builtin(preset)).builtin_body().is_some(), "{preset:?}");
        }
    }
}
