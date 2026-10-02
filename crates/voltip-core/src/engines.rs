//! Which recognition and clean-up services the pipeline talks to (docs/dictation.md §3): the
//! user's [`EngineSettings`] — one provider per service plus per-provider model and endpoint
//! choices — on top of the [`BuiltIn`] service compiled into the build, and the user's provider
//! keys from the [`voltip_identity::SecretStore`]. The UI only ever sees [`EngineStatus`]: provider
//! ids, models, the hosts of endpoints the user entered and whether a key is set — never a key and
//! never the built-in service's host.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::models::{DEFAULT_LOCAL_MODEL_ID, ModelState};
use crate::presets::PresetId;
pub use crate::providers::{KeyPolicy, ProviderId, ServiceKind};
use crate::providers::{PROVIDERS, key_entry};

/// The built-in recognition model's name when the build names none (`VOLTIP_ASR_MODEL`).
pub const DEFAULT_ASR_MODEL: &str = "Qwen/Qwen3-ASR-1.7B";
/// The built-in clean-up model's name when the build names none (`VOLTIP_REFINE_MODEL`).
pub const DEFAULT_REFINE_MODEL: &str = "qwen/qwen3.8-27b";

/// How finished text reaches the foreground application.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InjectMode {
    /// Clipboard + synthetic paste chord, clipboard restored afterwards.
    #[default]
    Paste,
    /// Only put the text in the clipboard.
    ClipboardOnly,
}

/// Where the live preview comes from (docs/dictation.md §11.8).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveSource {
    /// The built-in service, which decodes the sentence again as it grows
    /// (`dictation::redecode`): its own model, nothing to download.
    Cloud,
    /// The library's streaming model, on this device.
    Local,
}

/// Where the final text comes from and when it is delivered (docs/dictation.md §12).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputMode {
    /// Record, then transcribe the whole take, refine, inject (§2; the default).
    #[default]
    WholeTake,
    /// The streaming recogniser's committed sentences + tail are the final text; the whole-take
    /// transcriber is skipped. Needs `live_preview_ready`, otherwise falls back to `WholeTake`.
    StreamingFinal,
    /// Every sentence the streaming recogniser commits is injected at once; no refinement. Needs
    /// `live_preview_ready`, otherwise falls back to `WholeTake`.
    LiveInject,
}

impl OutputMode {
    /// Wire name (`whole_take` | `streaming_final` | `live_inject`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WholeTake => "whole_take",
            Self::StreamingFinal => "streaming_final",
            Self::LiveInject => "live_inject",
        }
    }

    /// Whether the final text comes from the streaming recogniser (needs the live preview).
    pub fn is_streaming(self) -> bool {
        !matches!(self, Self::WholeTake)
    }
}

/// Which Chinese script the recogniser's text is normalised to (docs/dictation.md §17), before
/// the dictionary corrections so the dictionary matches the chosen script.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChineseScript {
    /// Traditional characters become Simplified (the default: a zh-CN user never gets Traditional
    /// text injected, whatever the model answers).
    #[default]
    Simplified,
    /// Simplified characters become Traditional.
    Traditional,
    /// The recogniser's text is left as it came.
    AsIs,
}

impl ChineseScript {
    /// Wire name (`simplified` | `traditional` | `as_is`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Simplified => "simplified",
            Self::Traditional => "traditional",
            Self::AsIs => "as_is",
        }
    }
}

/// Where on-device models run (docs/dictation.md §10.4).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalDevice {
    /// The fastest device this build can drive; the CPU when no GPU backend initialises.
    #[default]
    Auto,
    /// The CPU only.
    Cpu,
    /// A GPU (`EngineSettings.local_gpu`, or the first one); the CPU when it does not initialise.
    Gpu,
}

/// Most inference threads a setting may ask for.
pub const MAX_LOCAL_THREADS: u16 = 256;

/// One provider's choices (`EngineSettings.providers`); every `None` means the catalogue preset.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderSettings {
    /// Recognition model id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asr_model: Option<String>,
    /// Recognition base URL (required for the custom provider).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asr_url: Option<String>,
    /// Clean-up model id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_model: Option<String>,
    /// Clean-up base URL (required for the custom provider).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_url: Option<String>,
}

impl ProviderSettings {
    /// The model chosen for `kind`, trimmed; `None` when blank.
    pub fn model(&self, kind: ServiceKind) -> Option<&str> {
        let value = match kind {
            ServiceKind::Asr => &self.asr_model,
            ServiceKind::Llm => &self.llm_model,
        };
        trimmed(value.as_deref())
    }

    /// The base URL entered for `kind`, trimmed; `None` when blank.
    pub fn url(&self, kind: ServiceKind) -> Option<&str> {
        let value = match kind {
            ServiceKind::Asr => &self.asr_url,
            ServiceKind::Llm => &self.llm_url,
        };
        trimmed(value.as_deref())
    }
}

fn trimmed(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|s| !s.is_empty())
}

/// `Settings.engines`.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct EngineSettings {
    /// Who recognises speech. `builtin` (the default) falls back to `local` in a build without the
    /// built-in service.
    pub asr_provider: ProviderId,
    /// Who cleans up the text and runs voice edits. `builtin` (the default) means none in a build
    /// without the built-in service.
    pub llm_provider: ProviderId,
    /// Run the clean-up after recognition (a scene may override it per take).
    pub refine_enabled: bool,
    /// What the clean-up does (docs/dictation.md §21; a scene may override it per take). A custom
    /// preset that no longer exists refines with 校对.
    pub refine_preset: PresetId,
    /// Per-provider model and endpoint choices; providers without an entry use their presets.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub providers: BTreeMap<ProviderId, ProviderSettings>,
    /// Local model catalogue id; `None` = the catalogue default (`qwen3-asr-0.6b`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_model: Option<String>,
    /// Where local models run.
    pub local_device: LocalDevice,
    /// The GPU `local_device = gpu` asks for (a device name from `UiState.hardware`); `None` = the
    /// first GPU.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_gpu: Option<String>,
    /// Inference threads for local models; `None` = decided per engine.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_threads: Option<u16>,
    /// Language hint (`zh`, `en`, …); `None` = auto-detect.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Show partial results on the pill while recording (docs/dictation.md §11). Takes effect only
    /// when the streaming model is installed; independent of the recognition provider.
    pub live_preview: bool,
    /// Output mode (docs/dictation.md §12); the two streaming modes only take effect when
    /// `live_preview_ready`, otherwise a run behaves as `whole_take`.
    pub output_mode: OutputMode,
    /// Trim leading / trailing silence with the Silero VAD before a local whole-take
    /// transcription (docs/dictation.md §12; fail-open, needs the `silero-vad` catalogue entry).
    pub vad_trim: bool,
    /// Script the recogniser's Chinese is normalised to (docs/dictation.md §17); every engine.
    pub chinese_script: ChineseScript,
    /// Injection route.
    pub inject: InjectMode,
}

impl Default for EngineSettings {
    fn default() -> Self {
        Self {
            asr_provider: ProviderId::Builtin,
            llm_provider: ProviderId::Builtin,
            refine_enabled: true,
            refine_preset: PresetId::default(),
            providers: BTreeMap::new(),
            local_model: None,
            local_device: LocalDevice::Auto,
            local_gpu: None,
            local_threads: None,
            language: None,
            live_preview: true,
            output_mode: OutputMode::WholeTake,
            vad_trim: false,
            chinese_script: ChineseScript::Simplified,
            inject: InjectMode::Paste,
        }
    }
}

impl EngineSettings {
    /// `providers[provider]`, or the presets.
    pub fn provider(&self, provider: ProviderId) -> ProviderSettings {
        self.providers.get(&provider).cloned().unwrap_or_default()
    }
}

/// The service compiled into the binary from `VOLTIP_*` build-time environment variables (empty
/// values count as unset). Production hosts and tokens live in `.env.build` (git-ignored) and CI
/// secrets, never in source; `Debug` shows only which parts are present.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct BuiltIn {
    /// `VOLTIP_ASR_URL`; `None` = no built-in recognition.
    pub asr_url: Option<&'static str>,
    /// `VOLTIP_ASR_TOKEN`.
    pub asr_token: Option<&'static str>,
    /// `VOLTIP_ASR_MODEL` or [`DEFAULT_ASR_MODEL`].
    pub asr_model: &'static str,
    /// `VOLTIP_REFINE_URL`; `None` = no built-in clean-up.
    pub refine_url: Option<&'static str>,
    /// `VOLTIP_REFINE_API_KEY`.
    pub refine_api_key: Option<&'static str>,
    /// `VOLTIP_REFINE_MODEL` or [`DEFAULT_REFINE_MODEL`].
    pub refine_model: &'static str,
    /// The built-in recognition previews while recording: the sentence is decoded again as it
    /// grows (docs/dictation.md §11.8). On in every build that carries the built-in recognition.
    pub asr_live_preview: bool,
}

const fn present(value: Option<&'static str>) -> Option<&'static str> {
    match value {
        Some(v) if !v.is_empty() => Some(v),
        _ => None,
    }
}

impl BuiltIn {
    /// What this build was compiled with.
    pub const fn from_build() -> Self {
        Self {
            asr_url: present(option_env!("VOLTIP_ASR_URL")),
            asr_token: present(option_env!("VOLTIP_ASR_TOKEN")),
            asr_model: match present(option_env!("VOLTIP_ASR_MODEL")) {
                Some(v) => v,
                None => DEFAULT_ASR_MODEL,
            },
            refine_url: present(option_env!("VOLTIP_REFINE_URL")),
            refine_api_key: present(option_env!("VOLTIP_REFINE_API_KEY")),
            refine_model: match present(option_env!("VOLTIP_REFINE_MODEL")) {
                Some(v) => v,
                None => DEFAULT_REFINE_MODEL,
            },
            asr_live_preview: present(option_env!("VOLTIP_ASR_URL")).is_some(),
        }
    }

    /// No built-in service at all (tests, and builds without `.env.build`).
    pub const EMPTY: Self = Self {
        asr_url: None,
        asr_token: None,
        asr_model: DEFAULT_ASR_MODEL,
        refine_url: None,
        refine_api_key: None,
        refine_model: DEFAULT_REFINE_MODEL,
        asr_live_preview: false,
    };

    /// Whether this build carries the built-in `kind` service.
    pub fn offers(&self, kind: ServiceKind) -> bool {
        match kind {
            ServiceKind::Asr => self.asr_url.is_some(),
            ServiceKind::Llm => self.refine_url.is_some(),
        }
    }

    fn service(&self, kind: ServiceKind) -> Option<RemoteService> {
        let (url, key, model) = match kind {
            ServiceKind::Asr => (self.asr_url, self.asr_token, self.asr_model),
            ServiceKind::Llm => (self.refine_url, self.refine_api_key, self.refine_model),
        };
        url.map(|url| RemoteService { url: url.to_owned(), model: model.to_owned(), key: key.map(str::to_owned) })
    }
}

impl std::fmt::Debug for BuiltIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuiltIn")
            .field("asr", &self.asr_url.is_some())
            .field("asr_token", &self.asr_token.is_some())
            .field("asr_model", &self.asr_model)
            .field("refine", &self.refine_url.is_some())
            .field("refine_api_key", &self.refine_api_key.is_some())
            .field("refine_model", &self.refine_model)
            .field("asr_live_preview", &self.asr_live_preview)
            .finish()
    }
}

/// Who supplied the key in effect.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretSource {
    /// Compiled into the build (built-in service only).
    Builtin,
    /// Entered by the user.
    User,
    /// Nothing available.
    #[default]
    None,
}

/// What the UI may know about a key.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct SecretState {
    /// A value is available.
    pub set: bool,
    /// Where it comes from.
    pub source: SecretSource,
}

/// The user's provider keys, by secret-store entry ([`crate::providers::key_entry`]), held in
/// memory after loading from the store. `Debug` shows which entries are set only.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct UserSecrets {
    keys: BTreeMap<&'static str, String>,
}

impl UserSecrets {
    /// The user's key for `provider`'s `kind` service, if any.
    pub fn get(&self, provider: ProviderId, kind: ServiceKind) -> Option<&str> {
        key_entry(provider, kind).and_then(|entry| self.keys.get(entry)).map(String::as_str)
    }

    /// Set (`Some`) or clear (`None`) the key of `provider`'s `kind` service (shared by a vendor's
    /// services). Empty / whitespace-only values clear. `false` when the provider takes no key.
    pub fn set(&mut self, provider: ProviderId, kind: ServiceKind, value: Option<String>) -> bool {
        let Some(entry) = key_entry(provider, kind) else { return false };
        self.set_entry(entry, value);
        true
    }

    /// Set or clear one secret-store entry (startup loading).
    pub fn set_entry(&mut self, entry: &'static str, value: Option<String>) {
        match value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()) {
            Some(v) => {
                self.keys.insert(entry, v);
            }
            None => {
                self.keys.remove(entry);
            }
        }
    }
}

impl std::fmt::Debug for UserSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserSecrets").field("set", &self.keys.keys().collect::<Vec<_>>()).finish()
    }
}

/// The local model the settings point at, as far as the core can tell from the model library.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LocalModelRef {
    /// Catalogue id.
    pub id: String,
    /// Display name (the id when the library does not know the entry).
    pub name: String,
    /// Every file is on disk and verified.
    pub installed: bool,
}

/// A remote service in effect: where requests go, the model, the key. Holds the real key;
/// `Debug` shows its presence only.
#[derive(Clone, PartialEq, Eq)]
pub struct RemoteService {
    /// Base URL (the client appends `/audio/transcriptions` or `/chat/completions`).
    pub url: String,
    /// Model id.
    pub model: String,
    /// Bearer key.
    pub key: Option<String>,
}

impl std::fmt::Debug for RemoteService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteService").field("model", &self.model).field("key", &self.key.is_some()).finish_non_exhaustive()
    }
}

/// Why a service cannot run; the UI explains each one where it is fixed.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineIssue {
    /// The provider does not offer this service in this build.
    Unavailable,
    /// The provider needs an API key and none is stored.
    KeyMissing,
    /// The custom endpoint has no base URL.
    UrlMissing,
    /// No model is chosen (custom endpoint, Ollama).
    ModelMissing,
    /// The selected local model is not downloaded.
    ModelNotInstalled,
    /// Clean-up has no provider (the default in a build without the built-in service).
    NoProvider,
}

impl EngineIssue {
    /// The sentence the pipeline and the CLI report (Chinese, like the core's other messages; the
    /// UI renders its own text from the code).
    pub fn message(self, kind: ServiceKind) -> &'static str {
        match (self, kind) {
            (Self::Unavailable, ServiceKind::Asr) => "此服务商不提供语音识别",
            (Self::Unavailable, ServiceKind::Llm) => "此服务商不提供 AI 润色",
            (Self::KeyMissing, _) => "服务商缺少 API 密钥（在「语音模型」或「AI 模型」页填写）",
            (Self::UrlMissing, _) => "自定义接口缺少接口地址（在「语音模型」或「AI 模型」页填写）",
            (Self::ModelMissing, _) => "未选择模型（在「语音模型」或「AI 模型」页选择）",
            (Self::ModelNotInstalled, _) => "本地模型未下载",
            (Self::NoProvider, _) => "未选择 AI 润色服务商",
        }
    }
}

/// Settings ⊕ user keys ⊕ the built-in service: what the shell builds its clients from. Holds real
/// keys; never serialized, `Debug` redacts.
#[derive(Clone, PartialEq, Eq)]
pub struct ResolvedEngines {
    /// The recognition provider in effect (`builtin` falls back to `local` without the built-in).
    pub asr_provider: ProviderId,
    /// The selected local model (`Some` iff `asr_provider == local`).
    pub local_model: Option<LocalModelRef>,
    /// The remote recognition service (`Some` iff a remote provider is ready).
    pub asr_remote: Option<RemoteService>,
    /// Why recognition cannot run now; `None` = ready.
    pub asr_issue: Option<EngineIssue>,
    /// The recognition model history records (the local model's display name on-device).
    pub asr_model: String,
    /// Language hint.
    pub language: Option<String>,
    /// `EngineSettings.refine_enabled` (a scene may override it per take).
    pub refine_enabled: bool,
    /// `EngineSettings.refine_preset` (a scene may override it per take).
    pub refine_preset: PresetId,
    /// The clean-up provider chosen (`None`: none in this build).
    pub llm_provider: Option<ProviderId>,
    /// The clean-up service, whenever its provider is ready (also with `refine_enabled` off: voice
    /// edits and scenes that switch the clean-up on use it).
    pub refine: Option<RemoteService>,
    /// Why the clean-up cannot run; `None` = ready.
    pub refine_issue: Option<EngineIssue>,
    /// The clean-up model (`""` when none is chosen).
    pub refine_model: String,
    /// `EngineSettings.live_preview`.
    pub live_preview: bool,
    /// `EngineSettings.output_mode` as configured; see [`ResolvedEngines::effective_output_mode`].
    pub output_mode: OutputMode,
    /// `EngineSettings.vad_trim`.
    pub vad_trim: bool,
    /// `EngineSettings.chinese_script`.
    pub chinese_script: ChineseScript,
    /// The library's streaming model (`capabilities` contains `streaming`), whatever the provider;
    /// `None` when the library has none (or there is no library).
    pub streaming_model: Option<LocalModelRef>,
    /// The built-in recognition is in use and previews by decoding again
    /// ([`BuiltIn::asr_live_preview`]).
    pub cloud_preview: bool,
    /// `EngineSettings.local_device`.
    pub local_device: LocalDevice,
    /// `EngineSettings.local_gpu`.
    pub local_gpu: Option<String>,
    /// `EngineSettings.local_threads`.
    pub local_threads: Option<u16>,
    /// Injection route.
    pub inject: InjectMode,
    providers: Vec<ProviderStatus>,
}

impl ResolvedEngines {
    /// Resolve without a model library in sight: a local model is named but not installed.
    pub fn resolve(settings: &EngineSettings, secrets: &UserSecrets, built_in: &BuiltIn) -> Self {
        Self::resolve_with_models(settings, secrets, built_in, &[])
    }

    /// Resolve with the model library's current view, so the local model's display name and
    /// `installed` flag are real. Built-in keys only ever travel with the built-in endpoints and a
    /// user key only with its own provider: the two never mix.
    pub fn resolve_with_models(settings: &EngineSettings, secrets: &UserSecrets, built_in: &BuiltIn, models: &[ModelState]) -> Self {
        let asr_provider = match settings.asr_provider {
            ProviderId::Builtin if !built_in.offers(ServiceKind::Asr) => ProviderId::Local,
            provider => provider,
        };
        let local = local_model_ref(settings, models);
        let (local_model, asr_remote, asr_issue, asr_model) = if asr_provider == ProviderId::Local {
            let issue = (!local.installed).then_some(EngineIssue::ModelNotInstalled);
            let name = local.name.clone();
            (Some(local.clone()), None, issue, name)
        } else {
            match service_target(asr_provider, ServiceKind::Asr, settings, secrets, built_in) {
                Ok(remote) => {
                    let model = remote.model.clone();
                    (None, Some(remote), None, model)
                }
                Err((issue, model)) => (None, None, Some(issue), model),
            }
        };
        let llm_provider = match settings.llm_provider {
            ProviderId::Builtin if !built_in.offers(ServiceKind::Llm) => None,
            provider => Some(provider),
        };
        let (refine, refine_issue, refine_model) = match llm_provider {
            None => (None, Some(EngineIssue::NoProvider), String::new()),
            Some(provider) => match service_target(provider, ServiceKind::Llm, settings, secrets, built_in) {
                Ok(remote) => {
                    let model = remote.model.clone();
                    (Some(remote), None, model)
                }
                Err((issue, model)) => (None, Some(issue), model),
            },
        };
        let streaming_model =
            models.iter().find(|m| m.is_streaming()).map(|m| LocalModelRef { id: m.id.clone(), name: m.name.clone(), installed: m.state.is_installed() });
        let providers = provider_statuses(settings, secrets, built_in, &local, asr_provider, llm_provider);
        let cloud_preview = asr_provider == ProviderId::Builtin && asr_remote.is_some() && built_in.asr_live_preview;
        Self {
            asr_provider,
            local_model,
            asr_remote,
            asr_issue,
            asr_model,
            language: trimmed(settings.language.as_deref()).map(str::to_owned),
            refine_enabled: settings.refine_enabled,
            refine_preset: settings.refine_preset,
            llm_provider,
            refine,
            refine_issue,
            refine_model,
            live_preview: settings.live_preview,
            output_mode: settings.output_mode,
            vad_trim: settings.vad_trim,
            chinese_script: settings.chinese_script,
            streaming_model,
            cloud_preview,
            local_device: settings.local_device,
            local_gpu: trimmed(settings.local_gpu.as_deref()).map(str::to_owned),
            local_threads: settings.local_threads,
            inject: settings.inject,
            providers,
        }
    }

    /// Recognition is on-device.
    pub fn is_local(&self) -> bool {
        self.asr_provider == ProviderId::Local
    }

    /// Where the live preview comes from when it is on (docs/dictation.md §11.8): the built-in
    /// service when it recognises (user request 2026-09-30: Qwen3-ASR previews itself); otherwise
    /// the library's streaming model when it is installed. Other cloud services do not preview: each
    /// preview is another request, which their owners pay for.
    pub fn live_source(&self) -> Option<LiveSource> {
        if !self.live_preview {
            return None;
        }
        if self.cloud_preview {
            return Some(LiveSource::Cloud);
        }
        self.streaming_model.as_ref().is_some_and(|m| m.installed).then_some(LiveSource::Local)
    }

    /// The live preview has a source: the engine opens a streaming session next to every recording
    /// (docs/dictation.md §11).
    pub fn live_preview_ready(&self) -> bool {
        self.live_source().is_some()
    }

    /// The output mode a `DictationStart` would run with now (docs/dictation.md §12): the two
    /// streaming modes need [`ResolvedEngines::live_preview_ready`], otherwise `WholeTake`.
    pub fn effective_output_mode(&self) -> OutputMode {
        if self.output_mode.is_streaming() && !self.live_preview_ready() { OutputMode::WholeTake } else { self.output_mode }
    }

    /// The UI projection: provider ids, models, user-entered hosts and key presence only.
    pub fn status(&self) -> EngineStatus {
        let user_host = |provider: ProviderId, remote: &Option<RemoteService>| match (provider, remote) {
            (ProviderId::Builtin | ProviderId::Local, _) | (_, None) => String::new(),
            (_, Some(r)) => host_of(&r.url),
        };
        EngineStatus {
            asr_provider: self.asr_provider,
            asr_ready: self.asr_issue.is_none(),
            asr_issue: self.asr_issue,
            asr_model: self.asr_model.clone(),
            asr_host: user_host(self.asr_provider, &self.asr_remote),
            local_model: self.local_model.as_ref().map(|m| m.id.clone()),
            local_ready: self.local_model.as_ref().is_some_and(|m| m.installed),
            live_preview_ready: self.live_preview_ready(),
            live_source: self.live_source(),
            effective_output_mode: self.effective_output_mode(),
            language: self.language.clone(),
            refine_enabled: self.refine_enabled,
            llm_provider: self.llm_provider,
            refine_ready: self.refine_issue.is_none(),
            refine_issue: self.refine_issue,
            refine_model: self.refine_model.clone(),
            refine_host: self.llm_provider.map(|p| user_host(p, &self.refine)).unwrap_or_default(),
            inject: self.inject,
            providers: self.providers.clone(),
        }
    }
}

impl std::fmt::Debug for ResolvedEngines {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedEngines")
            .field("asr_provider", &self.asr_provider)
            .field("asr_issue", &self.asr_issue)
            .field("asr_model", &self.asr_model)
            .field("llm_provider", &self.llm_provider)
            .field("refine_issue", &self.refine_issue)
            .field("refine_model", &self.refine_model)
            .finish_non_exhaustive()
    }
}

fn local_model_ref(settings: &EngineSettings, models: &[ModelState]) -> LocalModelRef {
    let id = trimmed(settings.local_model.as_deref()).unwrap_or(DEFAULT_LOCAL_MODEL_ID).to_owned();
    let known = models.iter().find(|m| m.id == id);
    LocalModelRef { name: known.map_or_else(|| id.clone(), |m| m.name.clone()), installed: known.is_some_and(|m| m.state.is_installed()), id }
}

/// The endpoint, model and key `provider` uses for `kind`, or why it cannot run (with the model
/// that would be used, for display). Never called for the on-device provider.
fn service_target(
    provider: ProviderId,
    kind: ServiceKind,
    settings: &EngineSettings,
    secrets: &UserSecrets,
    built_in: &BuiltIn,
) -> Result<RemoteService, (EngineIssue, String)> {
    let spec = provider.spec();
    let Some(preset) = spec.service(kind).filter(|_| provider != ProviderId::Local) else {
        return Err((EngineIssue::Unavailable, String::new()));
    };
    if provider == ProviderId::Builtin {
        return built_in.service(kind).ok_or((EngineIssue::Unavailable, String::new()));
    }
    let choice = settings.providers.get(&provider);
    let url = choice.and_then(|c| c.url(kind)).map(str::to_owned).or_else(|| (!preset.base_url.is_empty()).then(|| preset.base_url.to_owned()));
    let model = choice.and_then(|c| c.model(kind)).map(str::to_owned).or_else(|| preset.models.first().map(|m| (*m).to_owned()));
    let key = secrets.get(provider, kind).map(str::to_owned);
    let shown = model.clone().unwrap_or_default();
    let Some(url) = url else { return Err((EngineIssue::UrlMissing, shown)) };
    if spec.key == KeyPolicy::Required && key.is_none() {
        return Err((EngineIssue::KeyMissing, shown));
    }
    let Some(model) = model else { return Err((EngineIssue::ModelMissing, shown)) };
    Ok(RemoteService { url, model, key })
}

fn provider_statuses(
    settings: &EngineSettings,
    secrets: &UserSecrets,
    built_in: &BuiltIn,
    local: &LocalModelRef,
    asr_provider: ProviderId,
    llm_provider: Option<ProviderId>,
) -> Vec<ProviderStatus> {
    let service = |provider: ProviderId, kind: ServiceKind| -> Option<ServiceStatus> {
        let spec = provider.spec();
        let preset = spec.service(kind)?;
        let active = match kind {
            ServiceKind::Asr => asr_provider == provider,
            ServiceKind::Llm => llm_provider == Some(provider),
        };
        match provider {
            ProviderId::Builtin => {
                let remote = built_in.service(kind)?;
                let key = SecretState { set: remote.key.is_some(), source: if remote.key.is_some() { SecretSource::Builtin } else { SecretSource::None } };
                Some(ServiceStatus {
                    model: remote.model.clone(),
                    presets: vec![remote.model],
                    base_url: None,
                    default_base_url: None,
                    key,
                    issue: None,
                    active,
                })
            }
            ProviderId::Local => Some(ServiceStatus {
                model: local.id.clone(),
                presets: Vec::new(),
                base_url: None,
                default_base_url: None,
                key: SecretState::default(),
                issue: (!local.installed).then_some(EngineIssue::ModelNotInstalled),
                active,
            }),
            _ => {
                let target = service_target(provider, kind, settings, secrets, built_in);
                let choice = settings.provider(provider);
                let user_key = secrets.get(provider, kind).is_some();
                let (model, issue) = match &target {
                    Ok(remote) => (remote.model.clone(), None),
                    Err((issue, model)) => (model.clone(), Some(*issue)),
                };
                let default_base_url = (!preset.base_url.is_empty()).then(|| preset.base_url.to_owned());
                Some(ServiceStatus {
                    model,
                    presets: preset.models.iter().map(|m| (*m).to_owned()).collect(),
                    base_url: choice.url(kind).map(str::to_owned).or_else(|| default_base_url.clone()),
                    default_base_url,
                    key: SecretState { set: user_key, source: if user_key { SecretSource::User } else { SecretSource::None } },
                    issue,
                    active,
                })
            }
        }
    };
    PROVIDERS
        .iter()
        .filter_map(|spec| {
            let asr = service(spec.id, ServiceKind::Asr);
            let llm = service(spec.id, ServiceKind::Llm);
            (asr.is_some() || llm.is_some()).then(|| ProviderStatus {
                id: spec.id,
                key: spec.key,
                on_device: spec.on_device,
                console: spec.console_url.is_some(),
                asr,
                llm,
            })
        })
        .collect()
}

/// One provider as the engines pane shows it (`EngineStatus.providers`).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ProviderStatus {
    /// Id.
    pub id: ProviderId,
    /// Credential policy.
    pub key: KeyPolicy,
    /// Runs on this machine.
    pub on_device: bool,
    /// The shell can open the vendor's key page (`provider_console_open`; the phone too since
    /// 2026-10-01).
    pub console: bool,
    /// Recognition, when the provider offers it in this build.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asr: Option<ServiceStatus>,
    /// Clean-up, when the provider offers it in this build.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm: Option<ServiceStatus>,
}

/// One service of a provider.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ServiceStatus {
    /// Model in effect (`""` when none is chosen); the catalogue id for the on-device provider.
    pub model: String,
    /// Suggested models (the built-in service's own; empty on-device, where the model library
    /// lists them, and for the custom endpoint and Ollama).
    pub presets: Vec<String>,
    /// The endpoint requests go to. `None` for the built-in and on-device providers: the built-in
    /// host is never shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// The vendor's public endpoint (the placeholder of the URL field); `None` for the custom one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_base_url: Option<String>,
    /// Key presence and source.
    pub key: SecretState,
    /// Why it cannot run; `None` = ready.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue: Option<EngineIssue>,
    /// The provider in use for this service.
    pub active: bool,
}

/// `UiState.engines`: the resolved configuration without any key, and without the built-in
/// service's host.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct EngineStatus {
    /// The recognition provider in effect.
    pub asr_provider: ProviderId,
    /// Recognition can run; the home button is disabled until it can.
    pub asr_ready: bool,
    /// Why it cannot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asr_issue: Option<EngineIssue>,
    /// Recognition model (the local model's display name on-device).
    pub asr_model: String,
    /// Host of the recognition endpoint the user chose; `""` for the built-in service (never shown)
    /// and on-device.
    pub asr_host: String,
    /// Selected local model's catalogue id (on-device only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_model: Option<String>,
    /// On-device and the model's files are on disk and verified.
    pub local_ready: bool,
    /// `live_preview` is on and has a source ([`EngineStatus::live_source`]), so the pill shows
    /// partial text while recording.
    pub live_preview_ready: bool,
    /// Where the live preview comes from; `None` when it is off or has no source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_source: Option<LiveSource>,
    /// The output mode the next `DictationStart` really runs with (docs/dictation.md §12).
    pub effective_output_mode: OutputMode,
    /// Language hint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Whether takes are cleaned up.
    pub refine_enabled: bool,
    /// The clean-up provider chosen (`None` in a build without the built-in one, until the user
    /// picks one).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_provider: Option<ProviderId>,
    /// The clean-up can run (whatever `refine_enabled` says).
    pub refine_ready: bool,
    /// Why it cannot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refine_issue: Option<EngineIssue>,
    /// Clean-up model.
    pub refine_model: String,
    /// Host of the clean-up endpoint the user chose; `""` for the built-in service.
    pub refine_host: String,
    /// Injection route.
    pub inject: InjectMode,
    /// Every provider this build offers, in display order.
    pub providers: Vec<ProviderStatus>,
}

impl Default for EngineStatus {
    /// Mirrors `emptyEngineStatus()` in `schema.ts`: nothing resolved yet.
    fn default() -> Self {
        Self {
            asr_provider: ProviderId::Builtin,
            asr_ready: false,
            asr_issue: None,
            asr_model: String::new(),
            asr_host: String::new(),
            local_model: None,
            local_ready: false,
            live_preview_ready: false,
            live_source: None,
            effective_output_mode: OutputMode::WholeTake,
            language: None,
            refine_enabled: true,
            llm_provider: None,
            refine_ready: false,
            refine_issue: None,
            refine_model: String::new(),
            refine_host: String::new(),
            inject: InjectMode::Paste,
            providers: Vec::new(),
        }
    }
}

/// Host part of a URL (`https://api.example.com/v1` → `api.example.com`); the input itself,
/// trimmed of scheme and path, when it does not parse.
pub fn host_of(url: &str) -> String {
    if let Ok(parsed) = url::Url::parse(url)
        && let Some(host) = parsed.host_str()
    {
        return host.to_owned();
    }
    let no_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    no_scheme.split(['/', '?', '#']).next().unwrap_or_default().trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUILT: BuiltIn = BuiltIn {
        asr_url: Some("https://asr.builtin.test"),
        asr_token: Some("built-asr-token"),
        asr_model: "Qwen/Qwen3-ASR-1.7B",
        refine_url: Some("https://llm.builtin.test/v1"),
        refine_api_key: Some("built-refine-key"),
        refine_model: "qwen/qwen3.8-27b",
        asr_live_preview: false,
    };

    fn with_provider(provider: ProviderId, choice: ProviderSettings) -> BTreeMap<ProviderId, ProviderSettings> {
        BTreeMap::from([(provider, choice)])
    }

    #[test]
    fn defaults_and_serde_shapes() {
        let s = EngineSettings::default();
        assert_eq!((s.asr_provider, s.llm_provider), (ProviderId::Builtin, ProviderId::Builtin));
        assert!(s.refine_enabled && s.live_preview && !s.vad_trim);
        assert_eq!(s.inject, InjectMode::Paste);
        assert_eq!(
            serde_json::to_string(&s).unwrap(),
            r#"{"asr_provider":"builtin","llm_provider":"builtin","refine_enabled":true,"refine_preset":"proofread","local_device":"auto","live_preview":true,"output_mode":"whole_take","vad_trim":false,"chinese_script":"simplified","inject":"paste"}"#
        );
        let parsed: EngineSettings = serde_json::from_str(
            r#"{"asr_provider":"groq","providers":{"groq":{"asr_model":"whisper-large-v3"},"custom":{"llm_url":"http://10.0.0.2:8000/v1"}},"local_threads":6,"local_device":"gpu","local_gpu":"Vulkan0"}"#,
        )
        .unwrap();
        assert_eq!(parsed.asr_provider, ProviderId::Groq);
        assert_eq!(parsed.llm_provider, ProviderId::Builtin, "missing keys take the defaults");
        assert_eq!(parsed.provider(ProviderId::Groq).model(ServiceKind::Asr), Some("whisper-large-v3"));
        assert_eq!(parsed.provider(ProviderId::Custom).url(ServiceKind::Llm), Some("http://10.0.0.2:8000/v1"));
        assert_eq!(parsed.refine_preset, PresetId::default(), "settings from before presets refine with 校对");
        let custom = uuid::Uuid::new_v4();
        let chosen: EngineSettings = serde_json::from_str(&format!(r#"{{"refine_preset":"{custom}"}}"#)).unwrap();
        assert_eq!(chosen.refine_preset, PresetId::Custom(custom));
        assert!(serde_json::from_str::<EngineSettings>(r#"{"refine_preset":"casual"}"#).is_err(), "unknown presets are refused");
        assert_eq!(parsed.provider(ProviderId::Openai), ProviderSettings::default());
        assert_eq!((parsed.local_device, parsed.local_gpu.as_deref(), parsed.local_threads), (LocalDevice::Gpu, Some("Vulkan0"), Some(6)));
        let round = serde_json::to_string(&parsed).unwrap();
        assert!(round.contains(r#""providers":{"groq":{"asr_model":"whisper-large-v3"},"custom":{"llm_url":"http://10.0.0.2:8000/v1"}}"#), "{round}");
        assert!(serde_json::from_str::<EngineSettings>(r#"{"asr_provider":"edge"}"#).is_err(), "unknown providers are refused");
        assert!(serde_json::from_str::<EngineSettings>(r#"{"local_device":"npu"}"#).is_err(), "unknown devices are refused");
        let empty = EngineStatus::default();
        let json = serde_json::to_string(&empty).unwrap();
        assert!(!json.contains("language") && !json.contains("local_model") && !json.contains("llm_provider"), "{json}");
        assert_eq!(serde_json::from_str::<EngineStatus>(&json).unwrap(), empty);
        assert_eq!(serde_json::from_str::<EngineStatus>("{}").unwrap(), empty, "every field has a default");
        assert_eq!(serde_json::to_string(&SecretState { set: true, source: SecretSource::Builtin }).unwrap(), r#"{"set":true,"source":"builtin"}"#);
        assert_eq!(serde_json::to_string(&EngineIssue::KeyMissing).unwrap(), r#""key_missing""#);
    }

    #[test]
    fn built_in_reads_empty_as_unset_and_debug_names_no_host_or_secret() {
        assert_eq!(present(Some("")), None);
        assert_eq!(present(Some("x")), Some("x"));
        let b = BuiltIn::from_build();
        assert!(!b.asr_model.is_empty() && !b.refine_model.is_empty());
        let dbg = format!("{BUILT:?}");
        assert!(dbg.contains("asr: true") && dbg.contains("asr_token: true"), "{dbg}");
        assert!(!dbg.contains("builtin.test"), "built-in host leaked: {dbg}");
        assert!(!dbg.contains("built-asr-token") && !dbg.contains("built-refine-key"), "secret value leaked: {dbg}");
        assert!(!BuiltIn::EMPTY.offers(ServiceKind::Asr) && !BuiltIn::EMPTY.offers(ServiceKind::Llm));
        assert!(BUILT.offers(ServiceKind::Asr) && BUILT.offers(ServiceKind::Llm));
    }

    /// The default configuration of a build with the built-in service: both services run on it, its
    /// keys travel with it, and the UI learns neither the host nor the keys.
    #[test]
    fn builtin_service_runs_with_its_own_keys_and_never_shows_its_host() {
        let r = ResolvedEngines::resolve(&EngineSettings::default(), &UserSecrets::default(), &BUILT);
        assert_eq!(r.asr_provider, ProviderId::Builtin);
        let asr = r.asr_remote.clone().unwrap();
        assert_eq!((asr.url.as_str(), asr.model.as_str(), asr.key.as_deref()), ("https://asr.builtin.test", "Qwen/Qwen3-ASR-1.7B", Some("built-asr-token")));
        let refine = r.refine.clone().unwrap();
        assert_eq!((refine.url.as_str(), refine.key.as_deref()), ("https://llm.builtin.test/v1", Some("built-refine-key")));
        let st = r.status();
        assert!(st.asr_ready && st.refine_ready);
        assert_eq!((st.asr_host.as_str(), st.refine_host.as_str()), ("", ""), "the built-in host is never reported");
        assert_eq!(st.asr_model, "Qwen/Qwen3-ASR-1.7B");
        let json = serde_json::to_string(&st).unwrap();
        for secret in ["builtin.test", "built-asr-token", "built-refine-key"] {
            assert!(!json.contains(secret), "{secret} reached the UI: {json}");
            assert!(!format!("{r:?}").contains(secret), "{secret} in Debug");
        }
        let builtin = st.providers.iter().find(|p| p.id == ProviderId::Builtin).unwrap();
        let asr = builtin.asr.as_ref().unwrap();
        assert!(asr.active && asr.base_url.is_none() && asr.issue.is_none());
        assert_eq!(asr.key, SecretState { set: true, source: SecretSource::Builtin });
        assert_eq!(asr.presets, ["Qwen/Qwen3-ASR-1.7B"]);
    }

    /// Regression (2026-09-25, carried over to providers): a key never travels to a
    /// host it was not meant for — built-in keys only to the built-in endpoints, a vendor key only
    /// to that vendor, the custom endpoint's key only to it.
    #[test]
    fn regression_keys_never_leave_their_provider() {
        let custom = EngineSettings {
            asr_provider: ProviderId::Custom,
            llm_provider: ProviderId::Custom,
            providers: with_provider(
                ProviderId::Custom,
                ProviderSettings {
                    asr_url: Some("https://my-asr.test".into()),
                    asr_model: Some("whisper".into()),
                    llm_url: Some("https://my-llm.test/v1".into()),
                    llm_model: Some("m".into()),
                },
            ),
            ..EngineSettings::default()
        };
        let r = ResolvedEngines::resolve(&custom, &UserSecrets::default(), &BUILT);
        assert_eq!(r.asr_remote.as_ref().map(|s| s.key.clone()), Some(None), "the built-in token must not follow a custom URL");
        assert_eq!(r.refine.as_ref().map(|s| s.key.clone()), Some(None));
        assert!(r.asr_issue.is_none() && r.refine_issue.is_none(), "the custom endpoint's key is optional");

        let mut secrets = UserSecrets::default();
        assert!(secrets.set(ProviderId::Groq, ServiceKind::Llm, Some(" gsk_mine ".into())));
        assert!(secrets.set(ProviderId::Custom, ServiceKind::Asr, Some("asr-own".into())));
        let r = ResolvedEngines::resolve(&custom, &secrets, &BUILT);
        assert_eq!(r.asr_remote.unwrap().key.as_deref(), Some("asr-own"));
        assert_eq!(r.refine.unwrap().key, None, "the custom ASR key is not the custom LLM key");
        let groq = EngineSettings { asr_provider: ProviderId::Groq, llm_provider: ProviderId::Groq, ..EngineSettings::default() };
        let r = ResolvedEngines::resolve(&groq, &secrets, &BUILT);
        assert_eq!(r.asr_remote.as_ref().unwrap().key.as_deref(), Some("gsk_mine"), "a vendor key serves both of its services");
        assert_eq!(r.refine.as_ref().unwrap().key.as_deref(), Some("gsk_mine"));
        assert_eq!(r.asr_remote.unwrap().url, "https://api.groq.com/openai/v1");
        let builtin = ResolvedEngines::resolve(&EngineSettings::default(), &secrets, &BUILT);
        assert_eq!(builtin.asr_remote.unwrap().key.as_deref(), Some("built-asr-token"), "user keys never reach the built-in service");
        assert!(!secrets.set(ProviderId::Builtin, ServiceKind::Asr, Some("x".into())), "the built-in service takes no user key");
        assert!(!secrets.set(ProviderId::Ollama, ServiceKind::Llm, Some("x".into())));
        assert!(!format!("{secrets:?}").contains("gsk_mine"), "Debug redacts");
    }

    /// A build without the built-in service: recognition falls back to the on-device model and the
    /// clean-up has no provider until the user picks one; neither built-in card is listed.
    #[test]
    fn a_build_without_the_builtin_service_falls_back_to_local_and_no_cleanup() {
        let r = ResolvedEngines::resolve_with_models(&EngineSettings::default(), &UserSecrets::default(), &BuiltIn::EMPTY, &library(true));
        assert_eq!(r.asr_provider, ProviderId::Local);
        assert_eq!(r.local_model.as_ref().map(|m| m.id.as_str()), Some(DEFAULT_LOCAL_MODEL_ID));
        assert!(r.asr_issue.is_none() && r.asr_remote.is_none());
        assert_eq!((r.llm_provider, r.refine_issue), (None, Some(EngineIssue::NoProvider)));
        assert!(r.refine.is_none());
        let st = r.status();
        assert_eq!(st.asr_provider, ProviderId::Local);
        assert!(st.asr_ready && st.local_ready && !st.refine_ready);
        assert_eq!(st.asr_host, "");
        assert!(st.providers.iter().all(|p| p.id != ProviderId::Builtin), "no built-in card without the built-in service");
        let local = st.providers.iter().find(|p| p.id == ProviderId::Local).unwrap();
        assert!(local.asr.as_ref().unwrap().active && local.llm.is_none() && local.on_device);
        // Not installed: named, not ready.
        let r = ResolvedEngines::resolve_with_models(&EngineSettings::default(), &UserSecrets::default(), &BuiltIn::EMPTY, &library(false));
        assert_eq!(r.asr_issue, Some(EngineIssue::ModelNotInstalled));
        assert!(!r.status().asr_ready);
    }

    #[test]
    fn vendors_resolve_presets_overrides_and_their_issues() {
        let mut secrets = UserSecrets::default();
        let openai = EngineSettings { asr_provider: ProviderId::Openai, llm_provider: ProviderId::Deepseek, ..EngineSettings::default() };
        let r = ResolvedEngines::resolve(&openai, &secrets, &BUILT);
        assert_eq!((r.asr_issue, r.refine_issue), (Some(EngineIssue::KeyMissing), Some(EngineIssue::KeyMissing)));
        assert_eq!(r.asr_model, "gpt-transcribe", "the preset model is shown even while the key is missing");
        assert_eq!(r.refine_model, "deepseek-flash");
        let st = r.status();
        assert!(!st.asr_ready && !st.refine_ready);
        assert_eq!((st.asr_host.as_str(), st.refine_host.as_str()), ("", ""), "nothing resolved, nothing to show");
        secrets.set(ProviderId::Openai, ServiceKind::Asr, Some("sk-a".into()));
        secrets.set(ProviderId::Deepseek, ServiceKind::Llm, Some("sk-d".into()));
        let r = ResolvedEngines::resolve(&openai, &secrets, &BUILT);
        let st = r.status();
        assert!(st.asr_ready && st.refine_ready);
        assert_eq!((st.asr_host.as_str(), st.refine_host.as_str()), ("api.openai.com", "api.deepseek.com"));
        let vendor = st.providers.iter().find(|p| p.id == ProviderId::Openai).unwrap();
        let asr = vendor.asr.as_ref().unwrap();
        assert_eq!(asr.presets, ["gpt-transcribe", "gpt-4o-mini-transcribe", "whisper-1"]);
        assert_eq!(asr.base_url.as_deref(), Some("https://api.openai.com/v1"));
        assert_eq!(asr.key, SecretState { set: true, source: SecretSource::User });
        assert!(asr.active && vendor.llm.as_ref().is_some_and(|l| !l.active && l.key.set), "one key, both services");
        assert!(vendor.console);
        // A chosen model and a proxy base URL override the presets; blank ones fall back.
        let proxied = EngineSettings {
            providers: with_provider(
                ProviderId::Openai,
                ProviderSettings {
                    asr_model: Some(" whisper-1 ".into()),
                    asr_url: Some("https://proxy.example.test/v1".into()),
                    llm_model: Some("  ".into()),
                    llm_url: None,
                },
            ),
            ..openai.clone()
        };
        let r = ResolvedEngines::resolve(&proxied, &secrets, &BUILT);
        assert_eq!(r.asr_remote.as_ref().map(|s| (s.url.as_str(), s.model.as_str())), Some(("https://proxy.example.test/v1", "whisper-1")));
        assert_eq!(r.status().asr_host, "proxy.example.test");
        // Ollama: no key needed, but no model until the user picks one.
        let ollama = EngineSettings { llm_provider: ProviderId::Ollama, ..EngineSettings::default() };
        let r = ResolvedEngines::resolve(&ollama, &UserSecrets::default(), &BUILT);
        assert_eq!(r.refine_issue, Some(EngineIssue::ModelMissing));
        let picked = EngineSettings {
            providers: with_provider(ProviderId::Ollama, ProviderSettings { llm_model: Some("qwen3:8b".into()), ..Default::default() }),
            ..ollama
        };
        let r = ResolvedEngines::resolve(&picked, &UserSecrets::default(), &BUILT);
        assert_eq!(r.refine.as_ref().map(|s| (s.url.as_str(), s.key.is_none())), Some(("http://127.0.0.1:11434/v1", true)));
        assert_eq!(r.status().refine_host, "127.0.0.1");
        // The custom endpoint needs its URL; a provider without the service is unavailable.
        let custom = EngineSettings { asr_provider: ProviderId::Custom, llm_provider: ProviderId::Local, ..EngineSettings::default() };
        let r = ResolvedEngines::resolve(&custom, &UserSecrets::default(), &BUILT);
        assert_eq!((r.asr_issue, r.refine_issue), (Some(EngineIssue::UrlMissing), Some(EngineIssue::Unavailable)));
    }

    fn library(installed: bool) -> Vec<ModelState> {
        use crate::models::ModelInstallState;
        let state = if installed { ModelInstallState::Installed { path: "/m".into(), installed_at: 1 } } else { ModelInstallState::NotInstalled };
        vec![
            ModelState {
                id: DEFAULT_LOCAL_MODEL_ID.into(),
                name: "均衡".into(),
                engine: "transcribe_cpp".into(),
                tier: "balanced".into(),
                capabilities: vec!["offline".into()],
                languages: vec!["zh".into()],
                size_bytes: 1,
                description: String::new(),
                recommended: true,
                repo: "example/model".into(),
                dir: String::new(),
                files: Vec::new(),
                active: false,
                state: state.clone(),
            },
            ModelState {
                id: "paraformer-zh".into(),
                name: "Paraformer 中文".into(),
                engine: "paraformer".into(),
                tier: "light".into(),
                capabilities: vec!["offline".into()],
                languages: vec!["zh".into()],
                size_bytes: 1,
                description: String::new(),
                recommended: false,
                repo: "example/model".into(),
                dir: String::new(),
                files: Vec::new(),
                active: false,
                state: ModelInstallState::Failed { message: "x".into() },
            },
            ModelState {
                id: "zipformer-stream-zh-en".into(),
                name: "实时预览".into(),
                engine: "zipformer_streaming".into(),
                tier: "streaming".into(),
                capabilities: vec!["streaming".into()],
                languages: vec!["zh".into(), "en".into()],
                size_bytes: 1,
                description: String::new(),
                recommended: false,
                repo: "example/model".into(),
                dir: String::new(),
                files: Vec::new(),
                active: false,
                state,
            },
        ]
    }

    /// On-device (docs/dictation.md §10): no endpoint, no credential, the model's display name
    /// stands in for the model id, and readiness follows the library's install state.
    #[test]
    fn local_recognition_drops_the_endpoint_and_reports_readiness_from_the_library() {
        let mut secrets = UserSecrets::default();
        secrets.set(ProviderId::Custom, ServiceKind::Asr, Some("user-token".into()));
        let local = EngineSettings { asr_provider: ProviderId::Local, ..EngineSettings::default() };
        let r = ResolvedEngines::resolve_with_models(&local, &secrets, &BUILT, &library(true));
        assert!(r.is_local() && r.asr_remote.is_none(), "no endpoint on-device");
        assert_eq!(r.asr_model, "均衡");
        assert_eq!(r.local_model, Some(LocalModelRef { id: DEFAULT_LOCAL_MODEL_ID.into(), name: "均衡".into(), installed: true }));
        assert_eq!(r.refine.as_ref().and_then(|s| s.key.as_deref()), Some("built-refine-key"), "the clean-up is untouched");
        let st = r.status();
        assert_eq!((st.asr_provider, st.asr_host.as_str()), (ProviderId::Local, ""));
        assert_eq!(st.local_model.as_deref(), Some("qwen3-asr-0.6b"));
        assert!(st.local_ready && st.asr_ready);
        assert!(!format!("{r:?}").contains("user-token"));
        let other = EngineSettings { local_model: Some(" paraformer-zh ".into()), ..local.clone() };
        let r = ResolvedEngines::resolve_with_models(&other, &secrets, &BUILT, &library(true));
        assert_eq!(r.asr_model, "Paraformer 中文");
        assert_eq!(r.asr_issue, Some(EngineIssue::ModelNotInstalled), "a failed download is not ready");
        let unknown = EngineSettings { local_model: Some("ghost".into()), ..local };
        let r = ResolvedEngines::resolve(&unknown, &secrets, &BUILT);
        assert_eq!(r.local_model, Some(LocalModelRef { id: "ghost".into(), name: "ghost".into(), installed: false }));
        // A remote provider never reports a local model.
        let st = ResolvedEngines::resolve_with_models(&EngineSettings::default(), &secrets, &BUILT, &library(true)).status();
        assert_eq!(st.local_model, None);
        assert!(!st.local_ready);
        let local_card = st.providers.iter().find(|p| p.id == ProviderId::Local).unwrap().asr.clone().unwrap();
        assert!(!local_card.active && local_card.issue.is_none(), "the on-device card still knows its model is installed");
    }

    #[test]
    fn local_runtime_choices_travel_into_the_resolution() {
        let settings =
            EngineSettings { local_device: LocalDevice::Gpu, local_gpu: Some(" Vulkan1 ".into()), local_threads: Some(8), ..EngineSettings::default() };
        let r = ResolvedEngines::resolve(&settings, &UserSecrets::default(), &BUILT);
        assert_eq!((r.local_device, r.local_gpu.as_deref(), r.local_threads), (LocalDevice::Gpu, Some("Vulkan1"), Some(8)));
        for (device, wire) in [(LocalDevice::Auto, "auto"), (LocalDevice::Cpu, "cpu"), (LocalDevice::Gpu, "gpu")] {
            assert_eq!(serde_json::to_string(&device).unwrap(), format!("\"{wire}\""));
        }
    }

    /// Live preview (docs/dictation.md §11) is independent of the provider: ready when the setting
    /// is on and the library's streaming model is installed; off by setting, missing model or no
    /// library at all.
    /// docs/dictation.md §11.8 (user request 2026-09-30: Qwen3-ASR previews itself): the built-in
    /// recognition previews on its own when its build says so, even with the streaming model
    /// installed; other providers, and a built-in service that does not preview, need the model;
    /// live preview switched off has no source.
    #[test]
    fn the_built_in_recognition_previews_itself_and_the_rest_needs_the_streaming_model() {
        const PREVIEWING: BuiltIn = BuiltIn { asr_live_preview: true, ..BUILT };
        let (streaming, none) = (library(true), library(false));
        let source = |settings: &EngineSettings, built_in: &BuiltIn, models: &[ModelState]| {
            let e = ResolvedEngines::resolve_with_models(settings, &UserSecrets::default(), built_in, models);
            assert_eq!(e.live_preview_ready(), e.live_source().is_some());
            assert_eq!(e.status().live_source, e.live_source());
            e.live_source()
        };
        let builtin = EngineSettings::default();
        assert_eq!(source(&builtin, &PREVIEWING, &none), Some(LiveSource::Cloud));
        assert_eq!(source(&builtin, &PREVIEWING, &streaming), Some(LiveSource::Cloud), "the service's own model before the local one");
        assert_eq!(source(&builtin, &BUILT, &none), None, "a built-in service that does not preview");
        assert_eq!(source(&builtin, &BUILT, &streaming), Some(LiveSource::Local));
        let off = EngineSettings { live_preview: false, ..EngineSettings::default() };
        assert_eq!(source(&off, &PREVIEWING, &streaming), None);
        let custom = EngineSettings {
            asr_provider: ProviderId::Custom,
            providers: with_provider(ProviderId::Custom, ProviderSettings { asr_url: Some("https://asr.example.test".into()), ..Default::default() }),
            ..EngineSettings::default()
        };
        assert_eq!(source(&custom, &PREVIEWING, &none), None, "another service's previews would be the owner's to pay for");
        assert_eq!(source(&custom, &PREVIEWING, &streaming), Some(LiveSource::Local));
        let local = EngineSettings { asr_provider: ProviderId::Local, ..EngineSettings::default() };
        assert_eq!(source(&local, &PREVIEWING, &streaming), Some(LiveSource::Local));
        // A build without the built-in recognition falls back to local, which previews locally.
        assert_eq!(source(&builtin, &BuiltIn::EMPTY, &streaming), Some(LiveSource::Local));
        assert!(BuiltIn::EMPTY.asr_live_preview.eq(&false));
        assert_eq!(serde_json::to_string(&[LiveSource::Cloud, LiveSource::Local]).unwrap(), r#"["cloud","local"]"#);
    }

    #[test]
    fn live_preview_readiness_follows_the_setting_and_the_streaming_model_only() {
        let secrets = UserSecrets::default();
        let cloud = EngineSettings::default();
        let r = ResolvedEngines::resolve_with_models(&cloud, &secrets, &BUILT, &library(true));
        assert!(r.live_preview && r.live_preview_ready());
        assert_eq!(r.streaming_model, Some(LocalModelRef { id: "zipformer-stream-zh-en".into(), name: "实时预览".into(), installed: true }));
        assert!(r.status().live_preview_ready, "remote ASR + installed streaming model: the pill previews");
        let r = ResolvedEngines::resolve_with_models(&cloud, &secrets, &BUILT, &library(false));
        assert!(!r.live_preview_ready() && !r.status().live_preview_ready, "streaming model not installed");
        let off = EngineSettings { live_preview: false, ..EngineSettings::default() };
        let r = ResolvedEngines::resolve_with_models(&off, &secrets, &BUILT, &library(true));
        assert!(!r.live_preview_ready() && r.streaming_model.is_some(), "switched off; the model is still known");
        let r = ResolvedEngines::resolve(&cloud, &secrets, &BUILT);
        assert!(r.streaming_model.is_none() && !r.live_preview_ready(), "no library (the phone): nothing to preview with");
        let local = EngineSettings { asr_provider: ProviderId::Local, ..EngineSettings::default() };
        let r = ResolvedEngines::resolve_with_models(&local, &secrets, &BUILT, &library(true));
        assert!(r.live_preview_ready() && r.status().local_ready);
    }

    /// The output mode (docs/dictation.md §12) rides on the live preview: a streaming mode is
    /// effective only while `live_preview_ready`, otherwise the status says `whole_take`.
    #[test]
    fn streaming_output_modes_need_the_live_preview_and_serialize_snake_case() {
        let secrets = UserSecrets::default();
        for (mode, wire) in [(OutputMode::WholeTake, "whole_take"), (OutputMode::StreamingFinal, "streaming_final"), (OutputMode::LiveInject, "live_inject")] {
            assert_eq!(serde_json::to_string(&mode).unwrap(), format!("\"{wire}\""));
            assert_eq!(serde_json::from_str::<OutputMode>(&format!("\"{wire}\"")).unwrap(), mode);
            assert_eq!(mode.as_str(), wire);
            assert_eq!(mode.is_streaming(), mode != OutputMode::WholeTake);
            let settings = EngineSettings { output_mode: mode, vad_trim: true, ..EngineSettings::default() };
            let ready = ResolvedEngines::resolve_with_models(&settings, &secrets, &BUILT, &library(true));
            assert!(ready.live_preview_ready() && ready.vad_trim);
            assert_eq!(ready.effective_output_mode(), mode, "ready: the setting is effective");
            assert_eq!(ready.status().effective_output_mode, mode);
            let not_installed = ResolvedEngines::resolve_with_models(&settings, &secrets, &BUILT, &library(false));
            assert_eq!(not_installed.effective_output_mode(), OutputMode::WholeTake, "{wire}: no streaming model → whole take");
            let off = ResolvedEngines::resolve_with_models(&EngineSettings { live_preview: false, ..settings.clone() }, &secrets, &BUILT, &library(true));
            assert_eq!(off.effective_output_mode(), OutputMode::WholeTake, "{wire}: preview off → whole take");
        }
        let parsed: EngineSettings = serde_json::from_str(r#"{"output_mode":"live_inject","vad_trim":true}"#).unwrap();
        assert_eq!((parsed.output_mode, parsed.vad_trim), (OutputMode::LiveInject, true));
        assert!(serde_json::from_str::<EngineSettings>(r#"{"output_mode":"stream"}"#).is_err(), "unknown modes are refused");
    }

    /// docs/dictation.md §17: the script setting defaults to Simplified and travels into the
    /// resolved configuration.
    #[test]
    fn chinese_script_defaults_to_simplified_and_resolves() {
        assert_eq!(EngineSettings::default().chinese_script, ChineseScript::Simplified);
        for (script, wire) in [(ChineseScript::Simplified, "simplified"), (ChineseScript::Traditional, "traditional"), (ChineseScript::AsIs, "as_is")] {
            assert_eq!(serde_json::to_string(&script).unwrap(), format!("\"{wire}\""));
            assert_eq!(script.as_str(), wire);
            let settings = EngineSettings { chinese_script: script, ..EngineSettings::default() };
            assert_eq!(ResolvedEngines::resolve(&settings, &UserSecrets::default(), &BUILT).chinese_script, script);
        }
        assert!(serde_json::from_str::<EngineSettings>(r#"{"chinese_script":"hk"}"#).is_err(), "unknown scripts are refused");
    }

    #[test]
    fn host_of_handles_odd_input() {
        assert_eq!(host_of("https://voltip.example.test/v1/audio"), "voltip.example.test");
        assert_eq!(host_of("http://127.0.0.1:8000"), "127.0.0.1");
        assert_eq!(host_of("not a url"), "not a url");
        assert_eq!(host_of("weird://host.test/path?x"), "host.test");
        assert_eq!(host_of("host.only/path"), "host.only");
        assert_eq!(host_of(""), "");
    }
}
