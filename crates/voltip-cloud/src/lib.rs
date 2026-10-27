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
use voltip_asr::{AsrClient, AsrConfig, AudioUpload, DashscopeMode};
use voltip_core::dictation::{DictationError, RefineHints, Refined, Refiner, ServiceProbe, Transcriber, Transcript};
use voltip_core::providers::is_dashscope;
use voltip_core::usage::{AUDIO_MS_HEADER, AsrCall, Metering, PURPOSE_HEADER, QuotaReport, UsageProbe};
use voltip_core::{AsrProtocol, BuiltinPreset, ProbeError, ProbeFailure, ProviderId, RemoteService, ResolvedEngines, ServiceKind, TakePreset};

use crate::dashscope::DashscopeTranscriber;
use voltip_net::Network;
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
        self.transcribe_as(wav, language, glossary, &AsrCall::default()).await
    }

    /// A metered client (the built-in service, docs/dictation.md §27) says what the request is for
    /// and how long the take is; any other sends neither.
    async fn transcribe_as(&self, wav: &[u8], language: Option<&str>, glossary: &[String], call: &AsrCall) -> Result<Transcript, DictationError> {
        let prompt = voltip_core::vocabulary::glossary_prompt(glossary);
        let headers = match self.client.config().metering {
            Some(_) => vec![
                (PURPOSE_HEADER, call.purpose.as_str().to_owned()),
                (AUDIO_MS_HEADER, voltip_core::dictation::wav::duration_ms(wav).unwrap_or(0).to_string()),
            ],
            None => Vec::new(),
        };
        let t = self.client.transcribe_with_headers(wav, language, prompt.as_deref(), &headers).await.map_err(asr_error)?;
        Ok(Transcript { text: t.text, latency_ms: t.latency_ms, model: Some(t.model) })
    }
}

/// The recognition client's metering for the built-in service (docs/dictation.md §27).
pub fn asr_metering(metering: &Metering) -> voltip_asr::Metering {
    let reports = metering.clone();
    voltip_asr::Metering { headers: metering.headers(), on_report: Arc::new(move |report| reports.report(report)) }
}

/// The clean-up client's metering for the built-in service (docs/dictation.md §27).
pub fn refine_metering(metering: &Metering) -> voltip_refine::Metering {
    let reports = metering.clone();
    voltip_refine::Metering { headers: metering.headers(), on_report: Arc::new(move |report| reports.report(report)) }
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
    if let voltip_asr::AsrError::QuotaExhausted { retry_at_ms, .. } = &error {
        return DictationError::QuotaExhausted { service: ServiceKind::Asr, retry_at_ms: *retry_at_ms, detail: error.to_string() };
    }
    DictationError::Asr(error.to_string())
}

/// The core's error for a failed clean-up or edit: a used-up quota (docs/dictation.md §3.5), a
/// rate limit and an overloaded service (§3.6) keep their kind; anything else is `Refine` with the
/// reason.
pub fn refine_error(error: voltip_refine::RefineError) -> DictationError {
    match error {
        voltip_refine::RefineError::QuotaExhausted { retry_at_ms, .. } => {
            DictationError::QuotaExhausted { service: ServiceKind::Llm, retry_at_ms, detail: error.to_string() }
        }
        voltip_refine::RefineError::RateLimited { .. } => DictationError::RateLimited { service: ServiceKind::Llm, detail: error.to_string() },
        voltip_refine::RefineError::Overloaded { .. } => DictationError::Overloaded { service: ServiceKind::Llm, detail: error.to_string() },
        _ => DictationError::Refine(error.to_string()),
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

/// The HTTP client builder with the trust roots of this platform, for the shells' own requests
/// (feedback, the phone's update check). Elsewhere reqwest verifies with the system's store
/// (rustls-platform-verifier); on Android that verifier needs a JNI context the app never hands it
/// and panics on the first request, so there the requests trust Mozilla's root store, as
/// voltip-asr, voltip-refine and the relay connection do.
/// The requests follow `network`'s proxy plan (docs/dictation.md §24), and the connect timeout is
/// shared among a name's addresses ([`voltip_net::CONNECT_TIMEOUT`], issue #193).
pub fn http_client_builder(network: &Arc<Network>) -> reqwest::ClientBuilder {
    let builder = network.apply(reqwest::Client::builder().connect_timeout(voltip_net::CONNECT_TIMEOUT));
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
pub fn remote_transcriber(engines: &ResolvedEngines, network: &Arc<Network>) -> Arc<dyn Transcriber> {
    match &engines.asr_remote {
        None => Arc::new(Unconfigured(match engines.asr_issue {
            Some(issue) => format!("识别服务未配置：{}", issue.message(ServiceKind::Asr)),
            None => "识别服务未配置".to_owned(),
        })),
        Some(remote) => {
            // Only the built-in service is told who asks (docs/dictation.md §27).
            let metering = engines.metering.as_ref().filter(|_| engines.asr_provider == ProviderId::Builtin).map(asr_metering);
            transcriber_with(AsrProtocol::of(&remote.url, &remote.model), remote, asr_upload(engines.asr_provider), metering, network)
        }
    }
}

/// What a take is uploaded as to `provider`'s recogniser (docs/dictation.md §3.9): Ogg Opus to the
/// built-in service, whose vLLM decodes it (libsndfile), a tenth of the WAV on the slow cross-border
/// links most of its users reach it over; the WAV to every other service, the one format all of
/// them take.
pub fn asr_upload(provider: ProviderId) -> AudioUpload {
    if provider == ProviderId::Builtin { AudioUpload::OggOpus } else { AudioUpload::Wav }
}

/// The client that speaks `protocol` to `remote` (docs/dictation.md §3.4): OpenAI's
/// `/audio/transcriptions`, or one of Model Studio's; a Model Studio model dictation cannot use
/// says so on use. Takes go up as WAV.
pub fn transcriber_for(protocol: AsrProtocol, remote: &RemoteService, network: &Arc<Network>) -> Arc<dyn Transcriber> {
    transcriber_with(protocol, remote, AudioUpload::Wav, None, network)
}

/// [`transcriber_for`] uploading takes as `upload` (Model Studio's protocols always send the WAV),
/// metered with `metering` (the built-in service).
fn transcriber_with(
    protocol: AsrProtocol,
    remote: &RemoteService,
    upload: AudioUpload,
    metering: Option<voltip_asr::Metering>,
    network: &Arc<Network>,
) -> Arc<dyn Transcriber> {
    let config = AsrConfig::new(&remote.url, &remote.model)
        .with_token(remote.key.clone())
        .with_timeout(ASR_TIMEOUT)
        .with_upload(upload)
        .with_metering(metering)
        .with_network(network);
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
pub fn refiner(engines: &ResolvedEngines, network: &Arc<Network>) -> Option<Arc<dyn Refiner>> {
    engines.refine.as_ref().map(|remote| {
        let builtin = engines.llm_provider == Some(ProviderId::Builtin);
        // Only the built-in service is told who asks (docs/dictation.md §27).
        let metering = engines.metering.as_ref().filter(|_| builtin).map(refine_metering);
        let config = refine_config(remote, builtin, network).with_metering(metering);
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
pub fn refine_config(remote: &RemoteService, builtin: bool, network: &Arc<Network>) -> RefineConfig {
    let cap = if builtin { voltip_refine::BUILTIN_OUTPUT_CAP } else { voltip_refine::USER_OUTPUT_CAP };
    RefineConfig::new(openai_compatible_base(&remote.url), &remote.model)
        .with_api_key(remote.key.clone())
        .with_timeout(REFINE_TIMEOUT)
        .with_output_cap(cap)
        .with_enable_thinking(is_dashscope(&remote.url).then_some(false))
        .with_api(refine_api(remote.api))
        .with_reasoning_effort(remote.reasoning.map(reasoning_effort))
        .with_network(network)
}

/// The core's interface choice as the refine crate's (docs/dictation.md §3.7).
fn refine_api(api: voltip_core::LlmApi) -> voltip_refine::RefineApi {
    match api {
        voltip_core::LlmApi::ChatCompletions => voltip_refine::RefineApi::ChatCompletions,
        voltip_core::LlmApi::Responses => voltip_refine::RefineApi::Responses,
    }
}

fn reasoning_effort(effort: voltip_core::ReasoningEffort) -> voltip_refine::ReasoningEffort {
    use voltip_core::ReasoningEffort as E;
    match effort {
        E::Minimal => voltip_refine::ReasoningEffort::Minimal,
        E::Low => voltip_refine::ReasoningEffort::Low,
        E::Medium => voltip_refine::ReasoningEffort::Medium,
        E::High => voltip_refine::ReasoningEffort::High,
        E::Xhigh => voltip_refine::ReasoningEffort::Xhigh,
    }
}

/// Deadline of one question about the built-in service's quota.
pub const USAGE_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// The built-in service's quota for this device over HTTP (docs/dictation.md §27):
/// `GET {base}/voltip/quota`, `base` normalised to `…/v1` as the clients normalise it. What the
/// service said, or the connection's error, is the failure; the host is never in it.
#[derive(Clone, Debug)]
pub struct HttpUsageProbe {
    network: Arc<Network>,
}

impl HttpUsageProbe {
    /// A probe whose requests follow `network`'s proxy plan (docs/dictation.md §24).
    pub fn new(network: &Arc<Network>) -> Self {
        Self { network: Arc::clone(network) }
    }
}

#[async_trait]
impl UsageProbe for HttpUsageProbe {
    async fn quota(&self, base_url: &str, token: Option<&str>, headers: &[(String, String)]) -> Result<QuotaReport, String> {
        let base = voltip_asr::normalize_base_url(base_url).map_err(|e| e.to_string())?;
        let client = http_client_builder(&self.network).timeout(USAGE_PROBE_TIMEOUT).build().map_err(|e| e.to_string())?;
        let mut request = client.get(format!("{base}/voltip/quota"));
        for (name, value) in headers {
            request = request.header(name.as_str(), value.as_str());
        }
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.map_err(|e| e.without_url().to_string())?;
        let status = response.status();
        let body = response.text().await.map_err(|e| e.without_url().to_string())?;
        if !status.is_success() {
            return Err(format!("{status}: {}", voltip_reply::summary(&body)));
        }
        QuotaReport::parse_answer(&body)
    }
}

/// The engines pane's 测试连接 over HTTP (docs/dictation.md §3.3): `GET {base}/models` with the
/// key, answered with ids or a host-free failure. Recognition and clean-up endpoints are
/// normalised the same way (`…/v1`), so one request serves both kinds; a Model Studio address
/// is asked at its OpenAI-compatible base (§3.4). Both shells use it.
#[derive(Clone, Debug)]
pub struct HttpServiceProbe {
    network: Arc<Network>,
}

impl HttpServiceProbe {
    /// A probe whose requests follow `network`'s proxy plan (docs/dictation.md §24).
    pub fn new(network: &Arc<Network>) -> Self {
        Self { network: Arc::clone(network) }
    }
}

#[async_trait]
impl ServiceProbe for HttpServiceProbe {
    async fn list_models(&self, base_url: &str, key: Option<&str>) -> Result<Vec<String>, ProbeError> {
        voltip_refine::list_models(&openai_compatible_base(base_url), key, PROBE_TIMEOUT, &self.network).await.map_err(|e| {
            use voltip_refine::RefineError as E;
            // What the service said, or the connection's error, goes with the reason (§3.12).
            let status = |status: u16, said: String| ProbeError { reason: ProbeFailure::HttpStatus, status: Some(status), detail: None }.with_detail(said);
            match e {
                E::InvalidConfig(_) => ProbeError::new(ProbeFailure::InvalidUrl),
                E::Unauthorized { message } => ProbeError::new(ProbeFailure::Unauthorized).with_detail(message),
                E::RateLimited { message, .. } => status(429, message),
                E::Overloaded { message } => status(503, message),
                E::Server { status: code, body } => status(code, body),
                E::Network(message) => ProbeError::new(ProbeFailure::Unreachable).with_detail(message),
                E::Timeout => ProbeError::new(ProbeFailure::Timeout),
                other => ProbeError::new(ProbeFailure::BadResponse).with_detail(other.to_string()),
            }
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use voltip_core::{BuiltIn, EngineSettings, ProviderSettings, UserSecrets};

    /// The network of a test: following the system, whose proxy settings never apply to the
    /// local mock servers.
    fn net() -> Arc<Network> {
        Network::new()
    }

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
        assert!(http_client_builder(&net()).build().is_ok());
    }

    /// An address whose connection attempts go unanswered, as a network that blackholes it leaves
    /// them: a listener with no room left in its queue (backlog 0, one connection already waiting)
    /// drops the SYN of every further attempt on Linux and macOS; Windows resets them, one more
    /// failed attempt to the client. Keep the returned pair alive while the address must be silent.
    async fn silent_address() -> (std::net::SocketAddr, (tokio::net::TcpListener, tokio::net::TcpStream)) {
        let socket = tokio::net::TcpSocket::new_v4().unwrap();
        socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let addr = socket.local_addr().unwrap();
        let listener = socket.listen(0).unwrap();
        let waiting = tokio::net::TcpStream::connect(addr).await.unwrap();
        (addr, (listener, waiting))
    }

    /// A resolver that answers every name with the same addresses, in order.
    struct Fixed(Vec<std::net::SocketAddr>);

    impl reqwest::dns::Resolve for Fixed {
        fn resolve(&self, _name: reqwest::dns::Name) -> reqwest::dns::Resolving {
            let addrs = self.0.clone();
            Box::pin(async move { Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs) })
        }
    }

    /// The silent address first, then `server` four times: hyper gives each address a fifth of
    /// the connect timeout, so the silent one costs 2 s.
    fn silent_then(silent: std::net::SocketAddr, server: std::net::SocketAddr) -> std::sync::Arc<Fixed> {
        std::sync::Arc::new(Fixed(std::iter::once(silent).chain(std::iter::repeat_n(server, 4)).collect()))
    }

    /// Regression (issue #193): a name with an address that never answers held a request until
    /// the system gave up on it; the shells' requests share the connect timeout among the
    /// addresses, so the next one answers.
    #[tokio::test]
    async fn regression_an_address_that_never_answers_gives_way_to_the_next() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET")).respond_with(wiremock::ResponseTemplate::new(200).set_body_string("ok")).mount(&server).await;
        let (silent, _hold) = silent_address().await;
        let client = http_client_builder(&net()).no_proxy().dns_resolver(silent_then(silent, *server.address())).build().unwrap();
        let answer = tokio::time::timeout(std::time::Duration::from_secs(30), client.get("http://updates.test/latest.json").send())
            .await
            .expect("the silent address held the request");
        assert_eq!(answer.unwrap().text().await.unwrap(), "ok");
    }

    fn custom(asr_url: Option<&str>, llm_url: Option<&str>) -> ResolvedEngines {
        let provider = ProviderSettings {
            asr_url: asr_url.map(str::to_owned),
            asr_model: Some("whisper".into()),
            llm_url: llm_url.map(str::to_owned),
            llm_model: Some("m".into()),
            ..ProviderSettings::default()
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
        let err = remote_transcriber(&engines, &net()).transcribe(b"RIFF", None, &[]).await.unwrap_err();
        assert!(matches!(&err, DictationError::Asr(m) if m.starts_with("识别服务未配置")), "{err}");
        assert!(refiner(&engines, &net()).is_none(), "no clean-up provider, no refiner");
    }

    #[tokio::test]
    async fn a_url_the_client_refuses_is_reported_on_use() {
        let engines = custom(Some("not a url"), Some("ftp://nope"));
        let err = remote_transcriber(&engines, &net()).transcribe(b"RIFF", None, &[]).await.unwrap_err();
        assert!(matches!(&err, DictationError::Asr(m) if m.starts_with("识别服务配置无效")), "{err}");
        let err = refiner(&engines, &net()).expect("configured").refine("x", &RefineHints::default()).await.unwrap_err();
        assert!(matches!(&err, DictationError::Refine(m) if m.starts_with("润色服务配置无效")), "{err}");
    }

    /// docs/dictation.md §3.4 (goal 2026-10-03: every Model Studio model failed on
    /// `/audio/transcriptions`): the endpoint and the model pick the client — a realtime model's
    /// streams, the HTTP ones do not, a file-only model says what to use instead, and everything
    /// else keeps OpenAI's multipart.
    #[tokio::test]
    async fn the_protocol_picks_the_client() {
        let remote = |url: &str, model: &str| RemoteService { url: url.into(), model: model.into(), key: Some("sk-test".into()), ..RemoteService::default() };
        let studio = "https://dashscope.aliyuncs.com/compatible-mode/v1";
        let pick = |url: &str, model: &str| transcriber_for(AsrProtocol::of(url, model), &remote(url, model), &net());
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
            let err = transcriber_for(protocol, &remote("not a url", "m"), &net()).transcribe(b"RIFF", None, &[]).await.unwrap_err();
            assert!(matches!(&err, DictationError::Asr(m) if m.starts_with("识别服务配置无效")), "{protocol:?}: {err}");
        }
        // Through the resolution: the Model Studio vendor with a key.
        let mut secrets = UserSecrets::default();
        secrets.set(ProviderId::Aliyun, ServiceKind::Asr, Some("sk-test".into()));
        let settings = EngineSettings { asr_provider: ProviderId::Aliyun, ..EngineSettings::default() };
        let engines = ResolvedEngines::resolve(&settings, &secrets, &BuiltIn::EMPTY);
        assert!(remote_transcriber(&engines, &net()).streaming(&[]).is_some(), "the catalogue's default model streams");
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
        let asr = asr_error(AsrError::QuotaExhausted { code: "AllocationQuota.FreeTierOnly".into(), message: "free tier exhausted".into(), retry_at_ms: None });
        assert!(
            matches!(&asr, DictationError::QuotaExhausted { service: ServiceKind::Asr, detail, retry_at_ms: None } if detail.contains("FreeTierOnly")),
            "{asr:?}"
        );
        assert!(asr.is_quota_exhausted());
        assert_eq!(
            asr_error(AsrError::Unauthorized { message: "Invalid API Key (invalid_api_key)".into() }),
            DictationError::Asr("ASR rejected the credentials: Invalid API Key (invalid_api_key)".into()),
            "the service's words reach the take's failure (§3.12)"
        );
        let llm = refine_error(RefineError::QuotaExhausted { code: "insufficient_quota".into(), message: "m".into(), retry_at_ms: Some(9) });
        assert!(matches!(&llm, DictationError::QuotaExhausted { service: ServiceKind::Llm, retry_at_ms: Some(9), .. }), "{llm:?}");
        assert_eq!(refine_error(RefineError::Timeout), DictationError::Refine("refine request timed out".into()));
        // docs/dictation.md §3.6: a clean-up's 429 keeps its kind (the interface says the service is
        // busy), and it is not a used-up quota: no fallback model is tried for it.
        let busy = refine_error(RefineError::RateLimited { retry_after_ms: Some(355_000), message: String::new() });
        assert_eq!(busy, DictationError::RateLimited { service: ServiceKind::Llm, detail: "refine service rate limited (retry after 355000 ms)".into() });
        assert!(!busy.is_quota_exhausted());
        // 2026-10-24: so does a 503 (Gemini's "high demand"), as an overloaded service.
        let overloaded = refine_error(RefineError::Overloaded { message: "high demand".into() });
        assert_eq!(overloaded, DictationError::Overloaded { service: ServiceKind::Llm, detail: "refine service overloaded (HTTP 503): high demand".into() });
        assert!(!overloaded.is_quota_exhausted());
        assert_eq!(
            asr_error(AsrError::RateLimited { retry_after_ms: None, message: String::new() }),
            DictationError::Asr("ASR rate limited".into()),
            "recognition's is unchanged"
        );

        let used_up = serde_json::json!({ "error": { "code": "insufficient_quota", "message": "You exceeded your current quota" } });
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/transcriptions"))
            .respond_with(ResponseTemplate::new(429).set_body_json(used_up.clone()))
            .mount(&server)
            .await;
        Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(ResponseTemplate::new(403).set_body_json(used_up)).mount(&server).await;
        let remote = RemoteService { url: server.uri(), model: "whisper-1".into(), key: Some("sk-test".into()), ..RemoteService::default() };
        let err = transcriber_for(AsrProtocol::OpenaiTranscriptions, &remote, &net()).transcribe(b"RIFF", None, &[]).await.unwrap_err();
        assert!(matches!(&err, DictationError::QuotaExhausted { service: ServiceKind::Asr, .. }), "{err:?}");
        let refiner = HttpRefiner::new(refine_config(&remote, false, &net())).unwrap();
        let err = refiner.refine("x", &RefineHints::default()).await.unwrap_err();
        assert!(matches!(&err, DictationError::QuotaExhausted { service: ServiceKind::Llm, .. }), "{err:?}");

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/transcriptions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"text":"你好"}"#))
            .mount(&server)
            .await;
        let remote = RemoteService { url: server.uri(), model: "whisper-1".into(), key: None, ..RemoteService::default() };
        let t = transcriber_for(AsrProtocol::OpenaiTranscriptions, &remote, &net()).transcribe(b"RIFF", None, &[]).await.unwrap();
        assert_eq!((t.text.as_str(), t.model.as_deref()), ("你好", Some("whisper-1")));
    }

    /// Regression (2026-10-12, Paseo's dictation over voltip-server): a 36 s take to the built-in
    /// service was 1.15 MB of WAV and spent 25 s uploading on a lossy link from China, past
    /// Paseo's 25 s wait for the text. docs/dictation.md §3.9: the built-in service's takes go up
    /// as Ogg Opus, a tenth of the WAV; every other service gets the WAV.
    #[tokio::test]
    async fn regression_the_built_in_service_gets_ogg_opus_and_every_other_service_the_wav() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/transcriptions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"text":"你好"}"#))
            .mount(&server)
            .await;
        let tone: Vec<i16> = (0..36 * 16_000).map(|i| (6000.0 * (2.0 * std::f64::consts::PI * 180.0 * f64::from(i) / 16_000.0).sin()) as i16).collect();
        let take = voltip_core::dictation::wav::encode_pcm16(&tone, 16_000);
        let url: &'static str = Box::leak(server.uri().into_boxed_str());
        let built_in = BuiltIn { asr_url: Some(url), asr_token: Some("app-token"), ..BuiltIn::EMPTY };
        let engines = ResolvedEngines::resolve(&EngineSettings::default(), &UserSecrets::default(), &built_in);
        assert_eq!(engines.asr_provider, ProviderId::Builtin);
        assert_eq!(remote_transcriber(&engines, &net()).transcribe(&take, Some("zh"), &[]).await.unwrap().text, "你好");
        remote_transcriber(&custom(Some(url), None), &net()).transcribe(&take, Some("zh"), &[]).await.unwrap();

        let requests = server.received_requests().await.unwrap();
        let built_in_body = String::from_utf8_lossy(&requests[0].body);
        assert!(built_in_body.contains("filename=\"audio.ogg\"\r\nContent-Type: audio/ogg") && built_in_body.contains("OggS"), "{built_in_body}");
        assert!(requests[0].body.len() * 10 < take.len(), "{} bytes for a {}-byte WAV", requests[0].body.len(), take.len());
        assert_eq!(requests[0].headers.get("authorization").unwrap(), "Bearer app-token");
        let custom_body = String::from_utf8_lossy(&requests[1].body);
        assert!(
            custom_body.contains("filename=\"audio.wav\"\r\nContent-Type: audio/wav") && requests[1].body.len() > take.len(),
            "the custom service gets the WAV"
        );
        assert_eq!(asr_upload(ProviderId::Builtin), AudioUpload::OggOpus);
        // A fallback model's client is built for its own provider (`ResolvedEngines::with_candidate`).
        for provider in ProviderId::ALL.into_iter().filter(|p| *p != ProviderId::Builtin) {
            assert_eq!(asr_upload(provider), AudioUpload::Wav, "{provider:?}");
        }
    }

    /// A metering whose notes the test reads.
    fn test_metering() -> (Metering, Arc<std::sync::Mutex<Vec<voltip_core::usage::UsageNote>>>) {
        let notes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = notes.clone();
        let metering = Metering::new("6f1c2d3e-0000-4000-8000-000000000001", "voltip/0.1.10 (linux)", voltip_core::usage::UsageOrigin::ThisDevice, move |n| {
            sink.lock().unwrap().push(n);
        });
        (metering, notes)
    }

    /// docs/dictation.md §27: only the built-in service is told which device asks and what for, and
    /// its answers' reports reach the metering; a service the user configured gets none of it, even
    /// when the engines carry a metering.
    #[tokio::test]
    async fn only_the_built_in_service_is_told_who_asks() {
        use voltip_core::usage::{CLIENT_HEADER, DEVICE_HEADER, UsageNote};
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let report = "mode=count; basis=device; day=2026-10-27; reset=1793116800; asr=95/-; refine=3/-";
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/transcriptions"))
            .respond_with(ResponseTemplate::new(200).insert_header("x-voltip-quota", report).set_body_string(r#"{"text":"你好"}"#))
            .mount(&server)
            .await;
        let answer = serde_json::json!({ "model": "m", "choices": [{ "message": { "role": "assistant", "content": "好的。" }, "finish_reason": "stop" }] });
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).insert_header("x-voltip-quota", report).set_body_json(answer))
            .mount(&server)
            .await;
        let url: &'static str = Box::leak(server.uri().into_boxed_str());
        let (metering, notes) = test_metering();
        let built_in = BuiltIn { asr_url: Some(url), asr_token: Some("app-token"), refine_url: Some(url), refine_api_key: Some("app-key"), ..BuiltIn::EMPTY };
        let mut engines = ResolvedEngines::resolve(&EngineSettings::default(), &UserSecrets::default(), &built_in);
        assert_eq!((engines.asr_provider, engines.llm_provider), (ProviderId::Builtin, Some(ProviderId::Builtin)));
        engines.metering = Some(metering.clone());
        let take = voltip_core::dictation::wav::encode_pcm16(&[100; 24_000], 16_000);
        let transcriber = remote_transcriber(&engines, &net());
        transcriber.transcribe_as(&take, Some("zh"), &[], &AsrCall::take(None)).await.unwrap();
        transcriber.transcribe_as(&take, Some("zh"), &[], &AsrCall::preview(None)).await.unwrap();
        refiner(&engines, &net()).unwrap().refine("原文", &RefineHints::default()).await.unwrap();
        refiner(&engines, &net()).unwrap().edit("原文", "改短", &RefineHints::default()).await.unwrap();
        let mut user = custom(Some(url), Some(url));
        user.metering = Some(metering);
        remote_transcriber(&user, &net()).transcribe_as(&take, Some("zh"), &[], &AsrCall::take(None)).await.unwrap();
        refiner(&user, &net()).unwrap().refine("原文", &RefineHints::default()).await.unwrap();

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 6);
        let header = |i: usize, name: &str| requests[i].headers.get(name).map(|v| v.to_str().unwrap().to_owned());
        for i in 0..4 {
            assert_eq!(header(i, DEVICE_HEADER).as_deref(), Some("6f1c2d3e-0000-4000-8000-000000000001"), "request {i}");
            assert_eq!(header(i, CLIENT_HEADER).as_deref(), Some("voltip/0.1.10 (linux)"), "request {i}");
        }
        assert_eq!((header(0, PURPOSE_HEADER), header(0, AUDIO_MS_HEADER)), (Some("take".into()), Some("1500".into())));
        assert_eq!((header(1, PURPOSE_HEADER), header(1, AUDIO_MS_HEADER)), (Some("preview".into()), Some("1500".into())));
        assert_eq!((header(2, PURPOSE_HEADER), header(3, PURPOSE_HEADER)), (Some("refine".into()), Some("edit".into())));
        assert_eq!(header(3, AUDIO_MS_HEADER), None, "a clean-up carries no audio length");
        for i in 4..6 {
            for name in [DEVICE_HEADER, CLIENT_HEADER, PURPOSE_HEADER, AUDIO_MS_HEADER] {
                assert_eq!(header(i, name), None, "request {i} told the user's service {name}");
            }
        }
        let reports = notes.lock().unwrap().iter().filter(|n| matches!(n, UsageNote::Quota(q) if q.asr.used_seconds == 95 && q.refine.used == 3)).count();
        assert_eq!(reports, 4, "every built-in answer's report, none of the user's service: {:?}", notes.lock().unwrap());
    }

    /// docs/dictation.md §27: the quota is asked at `…/v1/voltip/quota` with the service's token and
    /// the metering's headers; a refusal says what the service said, never its host.
    #[tokio::test]
    async fn the_usage_probe_asks_the_built_in_service_for_the_day() {
        use voltip_core::usage::{DEVICE_HEADER, QuotaMode};
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "schema": 1, "mode": "limit", "basis": "device", "day": "2026-10-27", "reset_at": 1_793_116_800_u64,
            "asr": { "used_seconds": 600, "limit_seconds": 3600 }, "refine": { "used": 7, "limit": 100 }
        });
        Mock::given(method("GET"))
            .and(path("/v1/voltip/quota"))
            .and(header("authorization", "Bearer app-token"))
            .and(header(DEVICE_HEADER, "6f1c2d3e-0000-4000-8000-000000000001"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        let (metering, _) = test_metering();
        let probe = HttpUsageProbe::new(&net());
        let report = probe.quota(&server.uri(), Some("app-token"), &metering.headers()).await.unwrap();
        assert_eq!((report.mode, report.asr.limit_seconds, report.refine.used, report.reset_at), (QuotaMode::Limit, Some(3600), 7, 1_793_116_800));

        let refusing = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({ "error": { "message": "bad token" } })))
            .mount(&refusing)
            .await;
        let err = probe.quota(&format!("{}/v1", refusing.uri()), None, &[]).await.unwrap_err();
        assert!(err.contains("401") && err.contains("bad token") && !err.contains("127.0.0.1"), "{err}");
        let err = probe.quota("not a url", None, &[]).await.unwrap_err();
        assert!(!err.is_empty());
    }

    /// Regression (2026-10-04): a Model Studio clean-up runs without thinking, at the compatible
    /// base; every other service is asked as before.
    /// docs/dictation.md §3.7: the custom card's interface and effort reach the client.
    #[test]
    fn the_custom_interface_and_effort_reach_the_client() {
        let responses = RemoteService {
            url: "http://127.0.0.1:8787/v1".into(),
            model: "claude-opus-5-5".into(),
            key: Some("sk-test".into()),
            api: voltip_core::LlmApi::Responses,
            reasoning: Some(voltip_core::ReasoningEffort::Xhigh),
        };
        let config = refine_config(&responses, false, &net());
        assert_eq!((config.api, config.reasoning_effort), (voltip_refine::RefineApi::Responses, Some(voltip_refine::ReasoningEffort::Xhigh)));
        assert_eq!(config.enable_thinking, None);
        for (core, wire) in [
            (voltip_core::ReasoningEffort::Minimal, "minimal"),
            (voltip_core::ReasoningEffort::Low, "low"),
            (voltip_core::ReasoningEffort::Medium, "medium"),
            (voltip_core::ReasoningEffort::High, "high"),
        ] {
            assert_eq!(reasoning_effort(core).as_str(), wire);
        }
        let vendor = RemoteService { url: "https://api.openai.com/v1".into(), model: "gpt-5".into(), key: Some("sk-test".into()), ..RemoteService::default() };
        assert_eq!(refine_config(&vendor, false, &net()).api, voltip_refine::RefineApi::ChatCompletions);
    }

    #[test]
    fn regression_a_model_studio_clean_up_does_not_think() {
        let remote = |url: &str| RemoteService { url: url.into(), model: "qwen3.8-flash".into(), key: Some("sk-test".into()), ..RemoteService::default() };
        let studio = refine_config(&remote("https://ws-1.cn-beijing.maas.aliyuncs.com"), false, &net());
        assert_eq!(studio.enable_thinking, Some(false));
        assert_eq!(studio.base_url, "https://ws-1.cn-beijing.maas.aliyuncs.com/compatible-mode/v1");
        assert_eq!(studio.output_cap, voltip_refine::USER_OUTPUT_CAP);
        let other = refine_config(&remote("https://api.groq.com/openai/v1"), false, &net());
        assert_eq!((other.enable_thinking, other.base_url.as_str()), (None, "https://api.groq.com/openai/v1"));
        assert_eq!(refine_config(&remote("https://llm.builtin.test/v1"), true, &net()).output_cap, voltip_refine::BUILTIN_OUTPUT_CAP);
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
