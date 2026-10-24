//! The service providers the engines can talk to (docs/dictation.md §3): the service compiled into
//! the build, the on-device models, OpenAI-compatible vendors and a custom endpoint. Each provider
//! offers speech recognition, text clean-up (LLM) or both; the catalogue carries the public base URL
//! of each service, the vendor's other sites ([`ProviderSite`]) and two or three suggested models
//! (user decision 2026-10-25: the rest comes from the provider's own list once a key is saved). A
//! provider's key is shared by its services, except for the custom provider, whose two endpoints may
//! be two different servers.
//! [`AsrProtocol`] says how a recognition request reaches an endpoint: most speak OpenAI's
//! `/audio/transcriptions`, Alibaba Cloud Model Studio speaks its own protocols (§3.4).

use serde::{Deserialize, Serialize};

/// A provider id (`settings.json`, IPC and the secret store use the snake_case name).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderId {
    /// The service compiled into this build (`VOLTIP_ASR_URL` / `VOLTIP_REFINE_URL`). Its host
    /// and credentials never reach the UI. Chosen by default; a build without it falls back to
    /// [`ProviderId::Local`] for recognition and to no clean-up.
    #[default]
    Builtin,
    /// On-device recognition (transcribe.cpp / sherpa-onnx models from the model library).
    Local,
    /// OpenAI.
    Openai,
    /// Groq.
    Groq,
    /// Google AI Studio (the Gemini API through its OpenAI-compatible endpoint; clean-up only).
    Google,
    /// SiliconFlow (硅基流动).
    Siliconflow,
    /// Alibaba Cloud Model Studio (阿里云百炼, the DashScope API): its own recognition protocols
    /// ([`AsrProtocol`]), clean-up through its OpenAI-compatible mode.
    Aliyun,
    /// DeepSeek (clean-up only).
    Deepseek,
    /// Kimi (Moonshot AI), on its Chinese or its international site (clean-up only).
    Kimi,
    /// Zhipu's GLM: bigmodel.cn, or Z.ai abroad (clean-up only).
    Glm,
    /// Volcengine Ark (火山方舟), the Doubao models (clean-up only).
    Volcengine,
    /// Tencent Hunyuan through Tencent Cloud's TokenHub (clean-up only).
    Hunyuan,
    /// Xiaomi MiMo (clean-up only).
    Mimo,
    /// OpenRouter, one key for many vendors' models (clean-up only).
    Openrouter,
    /// Ollama on this machine (clean-up only, no key).
    Ollama,
    /// Any OpenAI-compatible endpoint the user enters (a Model Studio address speaks Model
    /// Studio's protocols, [`AsrProtocol::of`]).
    Custom,
}

impl ProviderId {
    /// Every provider in display order.
    pub const ALL: [Self; 16] = [
        Self::Builtin,
        Self::Local,
        Self::Openai,
        Self::Groq,
        Self::Google,
        Self::Siliconflow,
        Self::Aliyun,
        Self::Deepseek,
        Self::Kimi,
        Self::Glm,
        Self::Volcengine,
        Self::Hunyuan,
        Self::Mimo,
        Self::Openrouter,
        Self::Ollama,
        Self::Custom,
    ];

    /// Wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::Local => "local",
            Self::Openai => "openai",
            Self::Groq => "groq",
            Self::Google => "google",
            Self::Siliconflow => "siliconflow",
            Self::Aliyun => "aliyun",
            Self::Deepseek => "deepseek",
            Self::Kimi => "kimi",
            Self::Glm => "glm",
            Self::Volcengine => "volcengine",
            Self::Hunyuan => "hunyuan",
            Self::Mimo => "mimo",
            Self::Openrouter => "openrouter",
            Self::Ollama => "ollama",
            Self::Custom => "custom",
        }
    }

    /// The catalogue entry.
    pub fn spec(self) -> &'static ProviderSpec {
        PROVIDERS.iter().find(|p| p.id == self).unwrap_or(&PROVIDERS[0])
    }

    /// The vendor page `provider_console_open` opens, or the shells' answer when there is none.
    /// Only catalogue URLs are opened: the webview names a provider, a page and a site, never a
    /// URL. A site the provider does not have, or one without a page of its own, opens the
    /// provider's own page.
    pub fn console_page(self, page: ConsolePage, site: Option<SiteId>) -> Result<&'static str, String> {
        self.spec().console(page, site).ok_or_else(|| match page {
            ConsolePage::Key => format!("{}: no key page", self.as_str()),
            ConsolePage::Quota => format!("{}: no quota page", self.as_str()),
        })
    }
}

/// Which OpenAI interface a clean-up service speaks (docs/dictation.md §3.7); the custom provider
/// alone lets the user choose, every other one speaks chat completions.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmApi {
    /// `POST {base}/chat/completions` (the default).
    #[default]
    ChatCompletions,
    /// `POST {base}/responses`.
    Responses,
}

/// A Responses request's `reasoning.effort` (docs/dictation.md §3.7).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    /// `minimal`.
    Minimal,
    /// `low`.
    Low,
    /// `medium`.
    Medium,
    /// `high`.
    High,
    /// `xhigh`.
    Xhigh,
}

impl ReasoningEffort {
    /// The wire value (`minimal` … `xhigh`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
        }
    }
}

/// The two services a provider may offer.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceKind {
    /// Speech recognition (`POST {base}/audio/transcriptions`, or the endpoint's own protocol:
    /// [`AsrProtocol`]).
    Asr,
    /// Text clean-up and voice edit (`POST {base}/chat/completions`, or `/responses`: [`LlmApi`]).
    Llm,
}

/// Where a provider's credential comes from.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyPolicy {
    /// Compiled into the build; the user never enters one.
    Builtin,
    /// The user must enter an API key.
    Required,
    /// The endpoint may or may not want one.
    Optional,
    /// No credential at all (on-device, Ollama).
    None,
}

/// A service a vendor offers: its public base URL, its other sites and the two or three models
/// suggested first (the first one is the default; docs/dictation.md §3.3). The user may type any
/// other model id or pick one from the provider's list.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ServicePreset {
    /// Public base URL (OpenAI-compatible, including `/v1` where the vendor uses it): the first
    /// site's when the vendor has several.
    pub base_url: &'static str,
    /// Suggested model ids, default first.
    pub models: &'static [&'static str],
    /// The vendor's sites, the default one first, when it has more than one (Kimi and GLM have a
    /// Chinese and an international one, user decision 2026-10-25: one card that switches between
    /// them). Each site has its own accounts and keys; empty for a vendor with one address.
    pub sites: &'static [ProviderSite],
}

/// Which of a vendor's sites (docs/dictation.md §3.2).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SiteId {
    /// The site in mainland China (moonshot.cn, bigmodel.cn).
    Cn,
    /// The international site (moonshot.ai, z.ai).
    Intl,
}

/// One site of a vendor: where its requests go and where its keys are made.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ProviderSite {
    /// Id.
    pub id: SiteId,
    /// The service's base URL on this site.
    pub base_url: &'static str,
    /// The page where a key for this site is created.
    pub console_url: &'static str,
}

/// One provider of the catalogue.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ProviderSpec {
    /// Id.
    pub id: ProviderId,
    /// Speech recognition, if offered. The built-in and on-device providers carry an empty preset:
    /// their endpoint and models come from the build and the model library.
    pub asr: Option<ServicePreset>,
    /// Text clean-up, if offered.
    pub llm: Option<ServicePreset>,
    /// Credential policy.
    pub key: KeyPolicy,
    /// Runs on this machine: no audio or text leaves it.
    pub on_device: bool,
    /// The vendor page where an API key is created (opened by the desktop shell on request).
    pub console_url: Option<&'static str>,
    /// The vendor page that shows the free quota left and the usage, for a vendor whose API does
    /// not tell (Model Studio, docs/dictation.md §3.5).
    pub quota_url: Option<&'static str>,
}

/// Which vendor page `provider_console_open` opens.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsolePage {
    /// Where an API key is created ([`ProviderSpec::console_url`]).
    #[default]
    Key,
    /// Where the free quota and the usage are shown ([`ProviderSpec::quota_url`]).
    Quota,
}

impl ProviderSpec {
    /// The vendor's `page`, if it has one; the key page of `site` when that site has its own.
    pub fn console(&self, page: ConsolePage, site: Option<SiteId>) -> Option<&'static str> {
        match page {
            ConsolePage::Key => site.and_then(|id| self.site(id)).map(|s| s.console_url).or(self.console_url),
            ConsolePage::Quota => self.quota_url,
        }
    }

    /// The site `id` of whichever service has it.
    pub fn site(&self, id: SiteId) -> Option<&'static ProviderSite> {
        [self.asr.as_ref(), self.llm.as_ref()].into_iter().flatten().flat_map(|preset| preset.sites.iter()).find(|s| s.id == id)
    }

    /// The preset of `kind`, if the provider offers it.
    pub fn service(&self, kind: ServiceKind) -> Option<&ServicePreset> {
        match kind {
            ServiceKind::Asr => self.asr.as_ref(),
            ServiceKind::Llm => self.llm.as_ref(),
        }
    }

    /// Whether the provider offers `kind` at all (the built-in one only when the build has it).
    pub fn offers(&self, kind: ServiceKind) -> bool {
        self.service(kind).is_some()
    }
}

const NO_PRESET: ServicePreset = ServicePreset { base_url: "", models: &[], sites: &[] };

/// How a recognition request reaches a service (docs/dictation.md §3.4), decided by the endpoint and
/// the model: an Alibaba Cloud Model Studio address serves models that speak three different
/// protocols, none of them OpenAI's `/audio/transcriptions` (which answers 404 there).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AsrProtocol {
    /// `POST {base}/audio/transcriptions`, multipart: OpenAI, Groq, SiliconFlow, vLLM, the built-in
    /// service, any other endpoint.
    OpenaiTranscriptions,
    /// Model Studio's `qwen3-asr-flash…`: `POST …/compatible-mode/v1/chat/completions` with the
    /// audio as an `input_audio` part.
    DashscopeChat,
    /// Model Studio's `qwen-audio-…-asr-flash` and `fun-asr-flash…`: the whole file to
    /// `POST /api/v1/services/aigc/multimodal-generation/generation`.
    DashscopeMultimodal,
    /// Model Studio's realtime models on its WebSocket task protocol (`/api-ws/v1/inference`,
    /// `run-task`): `…-asr-flash-streaming`, `…-asr-flash-message`, `fun-asr…realtime…`,
    /// `paraformer-realtime…`. They recognise while the audio arrives ([`AsrProtocol::streams`]);
    /// a whole file sent at once may take as long as it plays.
    DashscopeDuplex,
    /// A Model Studio model dictation cannot use: the ones that only transcribe files by URL, in
    /// the background (`…-filetrans`, `fun-asr`, `fun-asr-mtl`, `paraformer-v2`, `sensevoice-v1`),
    /// and `qwen3-asr-flash-realtime`, whose realtime protocol this client does not speak.
    DashscopeUnsupported,
}

impl AsrProtocol {
    /// The protocol `model` speaks at `url` (a base URL as the settings hold it). Any address under
    /// `aliyuncs.com` is Model Studio's (`dashscope.aliyuncs.com`, `dashscope-intl.aliyuncs.com`, a
    /// workspace's `<id>.cn-beijing.maas.aliyuncs.com`); the model id picks among its protocols,
    /// snapshots (`…-2026-02-10`) included. An unknown model there gets the OpenAI-compatible chat
    /// form, which Model Studio's newer recognition models answer.
    pub fn of(url: &str, model: &str) -> Self {
        if !is_dashscope(url) {
            return Self::OpenaiTranscriptions;
        }
        let model = model.trim().to_ascii_lowercase();
        let realtime = model.contains("realtime");
        if model.contains("filetrans") || model.starts_with("qwen3-asr-flash-realtime") || model.starts_with("sensevoice") {
            Self::DashscopeUnsupported
        } else if model.starts_with("qwen3-asr") {
            Self::DashscopeChat
        } else if model.starts_with("qwen-audio") && model.contains("-asr") {
            if realtime || model.contains("-streaming") || model.contains("-message") { Self::DashscopeDuplex } else { Self::DashscopeMultimodal }
        } else if model.starts_with("fun-asr") {
            if realtime {
                Self::DashscopeDuplex
            } else if model.starts_with("fun-asr-flash") {
                Self::DashscopeMultimodal
            } else {
                Self::DashscopeUnsupported
            }
        } else if model.starts_with("paraformer") {
            if realtime { Self::DashscopeDuplex } else { Self::DashscopeUnsupported }
        } else {
            Self::DashscopeChat
        }
    }

    /// The service recognises while the take is spoken, so the take's audio goes to it as it is
    /// recorded and its sentences are the take's text (docs/dictation.md §3.4, §11.9).
    pub fn streams(self) -> bool {
        self == Self::DashscopeDuplex
    }
}

/// Whether `url` is an Alibaba Cloud Model Studio address (a host under `aliyuncs.com`).
pub fn is_dashscope(url: &str) -> bool {
    url::Url::parse(url.trim()).ok().and_then(|u| u.host_str().map(str::to_ascii_lowercase)).is_some_and(|host| host.ends_with(".aliyuncs.com"))
}

/// The catalogue, in display order. Two or three suggested models per service (user decision
/// 2026-10-25), checked against the vendors' documentation on the date their comment gives; the
/// provider's own `GET /models` lists the rest once a key is saved (docs/dictation.md §3.3).
pub const PROVIDERS: &[ProviderSpec] = &[
    ProviderSpec {
        id: ProviderId::Builtin,
        asr: Some(NO_PRESET),
        llm: Some(NO_PRESET),
        key: KeyPolicy::Builtin,
        on_device: false,
        console_url: None,
        quota_url: None,
    },
    ProviderSpec { id: ProviderId::Local, asr: Some(NO_PRESET), llm: None, key: KeyPolicy::None, on_device: true, console_url: None, quota_url: None },
    ProviderSpec {
        id: ProviderId::Openai,
        asr: Some(ServicePreset { base_url: "https://api.openai.com/v1", models: &["gpt-transcribe", "gpt-4o-mini-transcribe", "whisper-1"], sites: &[] }),
        llm: Some(ServicePreset { base_url: "https://api.openai.com/v1", models: &["gpt-6-luna", "gpt-6-sol"], sites: &[] }),
        key: KeyPolicy::Required,
        on_device: false,
        console_url: Some("https://platform.openai.com/api-keys"),
        quota_url: None,
    },
    ProviderSpec {
        id: ProviderId::Groq,
        asr: Some(ServicePreset { base_url: "https://api.groq.com/openai/v1", models: &["whisper-large-v3-turbo", "whisper-large-v3"], sites: &[] }),
        llm: Some(ServicePreset {
            base_url: "https://api.groq.com/openai/v1",
            models: &["qwen/qwen3.8-27b", "openai/gpt-oss-120b", "openai/gpt-oss-20b"],
            sites: &[],
        }),
        key: KeyPolicy::Required,
        on_device: false,
        console_url: Some("https://console.groq.com/keys"),
        quota_url: None,
    },
    // Google AI Studio (checked 2026-10-08, again 2026-10-24): the Gemini models with a free tier,
    // Flash-Lite first. On 2026-10-24 `gemini-3.8-flash` turned clean-ups down with 503 "high
    // demand" for 40 minutes, as did `gemini-3.1-flash-lite`, while `gemini-3.5-flash-lite` answered
    // in 1–6 s; the 2.5 models answer 404 "no longer available to new users". Its `GET /models`
    // names them `models/gemini-…` (see `usable_models`).
    ProviderSpec {
        id: ProviderId::Google,
        asr: None,
        llm: Some(ServicePreset {
            base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
            models: &["gemini-3.5-flash-lite", "gemini-3.8-flash", "gemini-3.1-flash-lite"],
            sites: &[],
        }),
        key: KeyPolicy::Required,
        on_device: false,
        console_url: Some("https://aistudio.google.com/apikey"),
        quota_url: None,
    },
    ProviderSpec {
        id: ProviderId::Siliconflow,
        asr: Some(ServicePreset { base_url: "https://api.siliconflow.cn/v1", models: &["FunAudioLLM/SenseVoiceSmall", "TeleAI/TeleSpeechASR"], sites: &[] }),
        llm: Some(ServicePreset { base_url: "https://api.siliconflow.cn/v1", models: &["Qwen/Qwen3-8B", "deepseek-ai/DeepSeek-V3"], sites: &[] }),
        key: KeyPolicy::Required,
        on_device: false,
        console_url: Some("https://cloud.siliconflow.cn/account/ak"),
        quota_url: None,
    },
    // Model Studio (checked 2026-10-04): the realtime `…-streaming` model first, since it recognises
    // while the user speaks (§3.4); `…-asr-flash` takes a whole file over HTTP, `fun-asr-realtime`
    // is the other realtime engine. A workspace's own address
    // (`https://<workspace>.cn-beijing.maas.aliyuncs.com/compatible-mode/v1`) replaces the public one
    // in the URL field.
    ProviderSpec {
        id: ProviderId::Aliyun,
        asr: Some(ServicePreset {
            base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
            models: &["qwen-audio-3.1-asr-flash-streaming", "qwen-audio-3.1-asr-flash", "fun-asr-realtime"],
            sites: &[],
        }),
        llm: Some(ServicePreset {
            base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
            models: &["qwen3.8-flash", "qwen3.8-max", "qwen3.7-flash"],
            sites: &[],
        }),
        key: KeyPolicy::Required,
        on_device: false,
        console_url: Some("https://bailian.console.aliyun.com/cn-beijing/model/settings/api-key"),
        quota_url: Some("https://bailian.console.aliyun.com/cn-beijing/costing-balance/free-quota"),
    },
    ProviderSpec {
        id: ProviderId::Deepseek,
        asr: None,
        llm: Some(ServicePreset { base_url: "https://api.deepseek.com", models: &["deepseek-flash", "deepseek-v4-pro"], sites: &[] }),
        key: KeyPolicy::Required,
        on_device: false,
        console_url: Some("https://platform.deepseek.com/api_keys"),
        quota_url: None,
    },
    // Kimi (platform.kimi.com, checked 2026-10-25): the K3 models always reason and take
    // `reasoning_effort`; K2 closes to new accounts on 2026-10-31. The two sites keep separate
    // accounts and keys.
    ProviderSpec {
        id: ProviderId::Kimi,
        asr: None,
        llm: Some(ServicePreset {
            base_url: "https://api.moonshot.cn/v1",
            models: &["kimi-k3.1-pro-highspeed", "kimi-k3.1-pro", "kimi-k3"],
            sites: &[
                ProviderSite { id: SiteId::Cn, base_url: "https://api.moonshot.cn/v1", console_url: "https://platform.kimi.com/console/api-keys" },
                ProviderSite { id: SiteId::Intl, base_url: "https://api.moonshot.ai/v1", console_url: "https://platform.kimi.ai/console/api-keys" },
            ],
        }),
        key: KeyPolicy::Required,
        on_device: false,
        console_url: Some("https://platform.kimi.com/console/api-keys"),
        quota_url: None,
    },
    // GLM (docs.bigmodel.cn and docs.z.ai, checked 2026-10-25): `glm-5.3-flash` is the newest fast
    // model, `glm-4.7-flash` costs nothing on either site. The two sites keep separate accounts and
    // keys.
    ProviderSpec {
        id: ProviderId::Glm,
        asr: None,
        llm: Some(ServicePreset {
            base_url: "https://open.bigmodel.cn/api/paas/v4",
            models: &["glm-5.3-flash", "glm-4.7-flash", "glm-5.3"],
            sites: &[
                ProviderSite {
                    id: SiteId::Cn,
                    base_url: "https://open.bigmodel.cn/api/paas/v4",
                    console_url: "https://bigmodel.cn/usercenter/proj-mgmt/apikeys",
                },
                ProviderSite { id: SiteId::Intl, base_url: "https://api.z.ai/api/paas/v4", console_url: "https://z.ai/manage-apikey/apikey-list" },
            ],
        }),
        key: KeyPolicy::Required,
        on_device: false,
        console_url: Some("https://bigmodel.cn/usercenter/proj-mgmt/apikeys"),
        quota_url: None,
    },
    // Volcengine Ark (docs.volcengine.com/docs/ark/model-list, checked 2026-10-25): its model ids
    // carry their release date, and a model answers only once it is enabled in the console.
    ProviderSpec {
        id: ProviderId::Volcengine,
        asr: None,
        llm: Some(ServicePreset {
            base_url: "https://ark.cn-beijing.volces.com/api/v3",
            models: &["doubao-seed-2-1-lite-260915", "doubao-seed-2-1-turbo-260628", "doubao-seed-2-0-mini-260428"],
            sites: &[],
        }),
        key: KeyPolicy::Required,
        on_device: false,
        console_url: Some("https://ark.volcengine.com/region:cn-beijing/apikey"),
        quota_url: None,
    },
    // Tencent Hunyuan (cloud.tencent.com/document/product/1823, checked 2026-10-25): Tencent Cloud
    // retired the old Hunyuan platform's models on 2026-06-22 for TokenHub, which also serves other
    // vendors' models. A model answers only once it is enabled in the console (402 otherwise).
    ProviderSpec {
        id: ProviderId::Hunyuan,
        asr: None,
        llm: Some(ServicePreset { base_url: "https://tokenhub.tencentmaas.com/v1", models: &["hy3", "hy4-preview"], sites: &[] }),
        key: KeyPolicy::Required,
        on_device: false,
        console_url: Some("https://console.cloud.tencent.com/tokenhub/apikey"),
        quota_url: None,
    },
    // Xiaomi MiMo (mimo.mi.com/docs, checked 2026-10-25): the 2.5 models were retired on 2026-10-21.
    ProviderSpec {
        id: ProviderId::Mimo,
        asr: None,
        llm: Some(ServicePreset { base_url: "https://api.xiaomimimo.com/v1", models: &["mimo-v2.6-flash", "mimo-v2.6-pro"], sites: &[] }),
        key: KeyPolicy::Required,
        on_device: false,
        console_url: Some("https://platform.xiaomimimo.com/#/console/api-keys"),
        quota_url: None,
    },
    // OpenRouter (openrouter.ai/api/v1/models, checked 2026-10-25): three fast, inexpensive models
    // of different vendors; its model list needs no key.
    ProviderSpec {
        id: ProviderId::Openrouter,
        asr: None,
        llm: Some(ServicePreset {
            base_url: "https://openrouter.ai/api/v1",
            models: &["google/gemini-3.5-flash-lite", "deepseek/deepseek-v4-flash", "openai/gpt-6-luna"],
            sites: &[],
        }),
        key: KeyPolicy::Required,
        on_device: false,
        console_url: Some("https://openrouter.ai/settings/keys"),
        quota_url: None,
    },
    ProviderSpec {
        id: ProviderId::Ollama,
        asr: None,
        llm: Some(ServicePreset { base_url: "http://127.0.0.1:11434/v1", models: &[], sites: &[] }),
        key: KeyPolicy::None,
        on_device: true,
        console_url: None,
        quota_url: None,
    },
    ProviderSpec {
        id: ProviderId::Custom,
        asr: Some(NO_PRESET),
        llm: Some(NO_PRESET),
        key: KeyPolicy::Optional,
        on_device: false,
        console_url: None,
        quota_url: None,
    },
];

/// Words in the id of a model that turns speech into text.
const SPEECH_TO_TEXT: [&str; 5] = ["whisper", "transcribe", "asr", "sensevoice", "paraformer"];

/// Words in the id of a model that does not chat in text: speech in or out, embeddings and
/// reranking, moderation, images, video and text recognition in images, realtime sessions, the
/// models only the Responses interface serves, and the omni models, which answer only as a stream.
const NOT_CHAT: [&str; 36] = [
    "whisper",
    "transcribe",
    "asr",
    "sensevoice",
    "paraformer",
    "tts",
    "speech",
    "audio",
    "cosyvoice",
    "sambert",
    "embed",
    "bge-",
    "rerank",
    "moderation",
    "guard",
    "image",
    "dall-e",
    "flux",
    "stable-diffusion",
    "kolors",
    "wanx",
    "t2i",
    "t2v",
    "i2v",
    "video",
    "seedream",
    "seedance",
    "vidu",
    "cogview",
    "cogvideo",
    "ocr",
    "realtime",
    "codex",
    "computer-use",
    "deep-research",
    "omni",
];

/// What `provider`'s `GET /models` at `url` listed, as the model selects offer it for `kind`
/// (docs/dictation.md §3.3): sorted, without duplicates, only the models that can serve `kind`. A
/// vendor lists every model it hosts, so recognition keeps the speech-to-text models
/// ([`SPEECH_TO_TEXT`], and on Model Studio only those this client speaks to,
/// [`AsrProtocol::DashscopeUnsupported`]) and clean-up drops the ones that do not chat in text
/// ([`NOT_CHAT`]). Google AI Studio names its models `models/gemini-…` and lists embedding, image,
/// speech and video models beside the text ones: only the Gemini and Gemma text models are kept,
/// under the names its chat completions take. The custom endpoint's list passes whole: it serves
/// what its owner put there; so does the built-in service's, which runs only the build's models
/// (the runtime keeps those).
pub fn usable_models(provider: ProviderId, kind: ServiceKind, url: &str, ids: Vec<String>) -> Vec<String> {
    let has = |id: &str, words: &[&str]| {
        let id = id.to_ascii_lowercase();
        words.iter().any(|word| id.contains(word))
    };
    let mut out: Vec<String> = match (provider, kind) {
        (ProviderId::Custom | ProviderId::Builtin, _) => ids,
        (ProviderId::Google, ServiceKind::Llm) => google_text_models(ids),
        (_, ServiceKind::Asr) => {
            ids.into_iter().filter(|id| has(id, &SPEECH_TO_TEXT) && AsrProtocol::of(url, id) != AsrProtocol::DashscopeUnsupported).collect()
        }
        (_, ServiceKind::Llm) => ids.into_iter().filter(|id| !has(id, &NOT_CHAT)).collect(),
    };
    out.retain(|id| !id.trim().is_empty());
    out.sort_unstable();
    out.dedup();
    out
}

fn google_text_models(ids: Vec<String>) -> Vec<String> {
    const NOT_TEXT: [&str; 13] =
        ["embedding", "-tts", "-image", "-live", "native-audio", "transcribe", "robotics", "computer-use", "imagen", "veo", "lyria", "omni", "aqa"];
    ids.into_iter()
        .map(|id| match id.strip_prefix("models/") {
            Some(name) => name.to_owned(),
            None => id,
        })
        .filter(|id| (id.starts_with("gemini-") || id.starts_with("gemma-")) && !NOT_TEXT.iter().any(|word| id.contains(word)))
        .collect()
}

/// The secret-store entry holding the user's key for `provider`'s `kind` service, or `None` when
/// the provider takes no user key (built-in, on-device, Ollama). A vendor's services share one key;
/// the custom provider keeps one per service.
pub fn key_entry(provider: ProviderId, kind: ServiceKind) -> Option<&'static str> {
    match (provider, kind) {
        (ProviderId::Builtin | ProviderId::Local | ProviderId::Ollama, _) => None,
        (ProviderId::Openai, _) => Some("provider-key.openai"),
        (ProviderId::Groq, _) => Some("provider-key.groq"),
        (ProviderId::Google, _) => Some("provider-key.google"),
        (ProviderId::Siliconflow, _) => Some("provider-key.siliconflow"),
        (ProviderId::Aliyun, _) => Some("provider-key.aliyun"),
        (ProviderId::Deepseek, _) => Some("provider-key.deepseek"),
        (ProviderId::Kimi, _) => Some("provider-key.kimi"),
        (ProviderId::Glm, _) => Some("provider-key.glm"),
        (ProviderId::Volcengine, _) => Some("provider-key.volcengine"),
        (ProviderId::Hunyuan, _) => Some("provider-key.hunyuan"),
        (ProviderId::Mimo, _) => Some("provider-key.mimo"),
        (ProviderId::Openrouter, _) => Some("provider-key.openrouter"),
        (ProviderId::Custom, ServiceKind::Asr) => Some("provider-key.custom-asr"),
        (ProviderId::Custom, ServiceKind::Llm) => Some("provider-key.custom-llm"),
    }
}

/// The environment variable the local speech service reads a key from (docs/dictation.md §23.5):
/// `provider-key.custom-llm` → `VOLTIP_KEY_CUSTOM_LLM`. The app itself reads keys from the
/// keychain only.
pub fn key_env_var(entry: &str) -> String {
    let name = entry.strip_prefix("provider-key.").unwrap_or(entry);
    format!("VOLTIP_KEY_{}", name.to_ascii_uppercase().replace(['-', '.'], "_"))
}

/// Every secret-store entry a user key may live in (loaded at startup).
pub fn key_entries() -> Vec<&'static str> {
    let mut out: Vec<&'static str> =
        ProviderId::ALL.into_iter().flat_map(|p| [ServiceKind::Asr, ServiceKind::Llm].map(|k| key_entry(p, k))).flatten().collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// Read the user's engine secrets from the store (the app at start). A store that cannot be read
/// leaves the secret unset (and logs): the identity already loaded from the same store, so this is
/// rare.
pub fn load_user_secrets(store: &dyn voltip_identity::SecretStore) -> crate::engines::UserSecrets {
    read_user_secrets(store, false)
}

/// The user's engine secrets read with no side effect ([`voltip_identity::SecretStore::peek`]):
/// the local speech service shares the app's keychain items and never moves, writes or asks for
/// one (docs/dictation.md §23). What it cannot read without that stays unset.
pub fn peek_user_secrets(store: &dyn voltip_identity::SecretStore) -> crate::engines::UserSecrets {
    read_user_secrets(store, true)
}

fn read_user_secrets(store: &dyn voltip_identity::SecretStore, read_only: bool) -> crate::engines::UserSecrets {
    let mut secrets = crate::engines::UserSecrets::default();
    for entry in key_entries() {
        let read = if read_only { store.peek(entry) } else { store.get(entry) };
        match read {
            Ok(Some(bytes)) => secrets.set_entry(entry, String::from_utf8(bytes.to_vec()).ok()),
            Ok(None) => {}
            Err(e) => tracing::warn!(error = %e, secret = entry, backend = store.backend_name(), "secret store read failed; treating as unset"),
        }
    }
    secrets
}

/// Why a provider probe failed (`GET {base}/models`, the engines pane's 测试连接). Carries no host
/// and no key: the built-in service's endpoint must not reach the UI through an error either.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeFailure {
    /// The provider does not offer this service, or this shell cannot probe.
    Unsupported,
    /// The custom endpoint has no base URL, or the one entered does not parse as http(s).
    InvalidUrl,
    /// The provider needs a key and none was entered or stored.
    KeyMissing,
    /// 401 / 403: the key was refused.
    Unauthorized,
    /// DNS, TCP or TLS failed, or the connection dropped.
    Unreachable,
    /// No answer within the deadline.
    Timeout,
    /// Another non-2xx answer (`status`).
    HttpStatus,
    /// A 2xx answer that is not an OpenAI-style model list.
    BadResponse,
}

/// A failed probe as the shell's [`crate::dictation::ServiceProbe`] reports it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ProbeError {
    /// Why.
    pub reason: ProbeFailure,
    /// The HTTP status of [`ProbeFailure::HttpStatus`].
    pub status: Option<u16>,
    /// What the service said, or the connection's error, short and without credentials
    /// (docs/dictation.md §3.12); `None` when there is nothing to add to the reason.
    pub detail: Option<String>,
}

impl ProbeError {
    /// A failure without an HTTP status.
    pub const fn new(reason: ProbeFailure) -> Self {
        Self { reason, status: None, detail: None }
    }

    /// `self` with `detail` (nothing when it is blank).
    pub fn with_detail(self, detail: impl Into<String>) -> Self {
        let detail = detail.into();
        Self { detail: (!detail.trim().is_empty()).then_some(detail), ..self }
    }
}

/// What a probe found.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum ProbeOutcome {
    /// The service answered with its model list (possibly empty).
    Ok {
        /// Model ids, as the service lists them (sorted, de-duplicated).
        models: Vec<String>,
        /// Round-trip time.
        latency_ms: u64,
    },
    /// It did not.
    Failed {
        /// Why.
        reason: ProbeFailure,
        /// The HTTP status of [`ProbeFailure::HttpStatus`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<u16>,
        /// What the service said, or the connection's error ([`ProbeError::detail`]).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
}

/// `CoreEvent::ProviderProbe`: the answer to one `provider_probe` command.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ProbeReport {
    /// Provider probed.
    pub provider: ProviderId,
    /// Service probed.
    pub kind: ServiceKind,
    /// Result.
    #[serde(flatten)]
    pub outcome: ProbeOutcome,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both readers see the same keys; the read-only one goes through `peek` (docs/dictation.md §23).
    #[test]
    fn the_secret_readers_read_the_provider_keys() {
        let store = voltip_identity::MemorySecretStore::new();
        let entry = key_entry(ProviderId::Aliyun, ServiceKind::Asr).expect("aliyun has a key");
        voltip_identity::SecretStore::set(&store, entry, b"sk-test").unwrap();
        for secrets in [load_user_secrets(&store), peek_user_secrets(&store)] {
            assert_eq!(secrets.get(ProviderId::Aliyun, ServiceKind::Asr), Some("sk-test"));
            assert_eq!(secrets.get(ProviderId::Openai, ServiceKind::Asr), None);
        }
        store.set_unavailable(true);
        assert_eq!(peek_user_secrets(&store).get(ProviderId::Aliyun, ServiceKind::Asr), None, "an unreadable store leaves the key unset");
    }

    #[test]
    fn catalogue_covers_every_id_once_in_display_order() {
        let ids: Vec<ProviderId> = PROVIDERS.iter().map(|p| p.id).collect();
        assert_eq!(ids, ProviderId::ALL.to_vec());
        for id in ProviderId::ALL {
            assert_eq!(id.spec().id, id);
            assert_eq!(serde_json::to_string(&id).unwrap(), format!("\"{}\"", id.as_str()));
            assert_eq!(serde_json::from_str::<ProviderId>(&format!("\"{}\"", id.as_str())).unwrap(), id);
        }
        assert!(serde_json::from_str::<ProviderId>(r#""azure""#).is_err(), "unknown providers are refused");
    }

    #[test]
    fn vendors_carry_a_public_https_base_and_a_default_model_per_service() {
        for spec in PROVIDERS {
            for kind in [ServiceKind::Asr, ServiceKind::Llm] {
                let Some(preset) = spec.service(kind) else { continue };
                match spec.id {
                    ProviderId::Builtin | ProviderId::Local | ProviderId::Custom => {
                        assert_eq!(*preset, NO_PRESET, "{:?} {kind:?}: endpoint comes from the build / library / user", spec.id);
                    }
                    ProviderId::Ollama => {
                        assert!(preset.base_url.starts_with("http://127.0.0.1:"), "Ollama is on this machine");
                        assert!(preset.models.is_empty(), "Ollama's models are whatever the user pulled");
                    }
                    _ => {
                        assert!(preset.base_url.starts_with("https://"), "{:?} {kind:?}", spec.id);
                        assert!(url::Url::parse(preset.base_url).is_ok());
                        // User decision 2026-10-25: two or three suggestions, the provider lists the rest.
                        assert!((2..=3).contains(&preset.models.len()), "{:?} {kind:?} suggests 2–3 models: {:?}", spec.id, preset.models);
                        assert!(spec.console_url.is_some(), "{:?} has a key page", spec.id);
                    }
                }
            }
            assert!(spec.console_url.is_none_or(|u| u.starts_with("https://")));
        }
        assert!(!ProviderId::Local.spec().offers(ServiceKind::Llm), "no on-device LLM in the library");
        assert!(!ProviderId::Deepseek.spec().offers(ServiceKind::Asr));
        assert!(!ProviderId::Ollama.spec().offers(ServiceKind::Asr));
        assert!(ProviderId::Local.spec().on_device && ProviderId::Ollama.spec().on_device);
    }

    /// docs/dictation.md §3.2: a vendor with a Chinese and an international site is one card that
    /// switches between them; each site has its own address and key page, the first is the default.
    #[test]
    fn a_vendors_sites_start_with_its_default_and_have_their_own_key_pages() {
        for spec in PROVIDERS {
            for kind in [ServiceKind::Asr, ServiceKind::Llm] {
                let Some(preset) = spec.service(kind) else { continue };
                if preset.sites.is_empty() {
                    continue;
                }
                assert!(preset.sites.len() >= 2, "{:?}: one site is no choice", spec.id);
                assert_eq!(preset.sites[0].base_url, preset.base_url, "{:?}: the first site is the default", spec.id);
                assert_eq!(Some(preset.sites[0].console_url), spec.console_url, "{:?}", spec.id);
                let mut ids: Vec<SiteId> = preset.sites.iter().map(|s| s.id).collect();
                ids.dedup();
                assert_eq!(ids.len(), preset.sites.len(), "{:?}: sites are distinct", spec.id);
                for site in preset.sites {
                    assert!(site.base_url.starts_with("https://") && url::Url::parse(site.base_url).is_ok(), "{site:?}");
                    assert!(site.console_url.starts_with("https://"), "{site:?}");
                    assert_eq!(spec.console(ConsolePage::Key, Some(site.id)), Some(site.console_url));
                }
            }
        }
        let kimi = ProviderId::Kimi.spec();
        assert_eq!(kimi.site(SiteId::Intl).map(|s| s.base_url), Some("https://api.moonshot.ai/v1"));
        assert_eq!(ProviderId::Glm.spec().site(SiteId::Intl).map(|s| s.base_url), Some("https://api.z.ai/api/paas/v4"));
        assert_eq!(ProviderId::Kimi.console_page(ConsolePage::Key, Some(SiteId::Intl)), Ok("https://platform.kimi.ai/console/api-keys"));
        // A vendor with one site opens its own page whatever site is named.
        assert_eq!(ProviderId::Groq.console_page(ConsolePage::Key, Some(SiteId::Intl)), Ok("https://console.groq.com/keys"));
        assert!(ProviderId::Kimi.console_page(ConsolePage::Quota, Some(SiteId::Cn)).is_err());
        assert_eq!(serde_json::to_string(&SiteId::Intl).unwrap(), r#""intl""#);
    }

    #[test]
    fn vendor_keys_are_shared_and_custom_keys_are_per_service() {
        assert_eq!(key_entry(ProviderId::Groq, ServiceKind::Asr), key_entry(ProviderId::Groq, ServiceKind::Llm));
        assert_ne!(key_entry(ProviderId::Custom, ServiceKind::Asr), key_entry(ProviderId::Custom, ServiceKind::Llm));
        for p in [ProviderId::Builtin, ProviderId::Local, ProviderId::Ollama] {
            assert_eq!(key_entry(p, ServiceKind::Asr), None);
            assert_eq!(key_entry(p, ServiceKind::Llm), None);
        }
        assert_eq!(key_entry(ProviderId::Aliyun, ServiceKind::Asr), Some("provider-key.aliyun"));
        assert_eq!(key_entry(ProviderId::Aliyun, ServiceKind::Asr), key_entry(ProviderId::Aliyun, ServiceKind::Llm));
        let entries = key_entries();
        assert_eq!(entries.len(), 14, "{entries:?}");
        assert!(entries.iter().all(|e| e.starts_with("provider-key.")));
    }

    /// docs/dictation.md §3.4: a Model Studio address speaks Model Studio's protocols, picked by the
    /// model; every other address keeps OpenAI's `/audio/transcriptions`.
    #[test]
    fn the_endpoint_and_the_model_pick_the_recognition_protocol() {
        use AsrProtocol::*;
        let public = "https://dashscope.aliyuncs.com/compatible-mode/v1";
        let workspace = "https://ws-test.cn-beijing.maas.aliyuncs.com/compatible-mode/v1";
        let intl = "HTTPS://DashScope-Intl.AliyunCS.com/api/v1/";
        let table = [
            ("qwen-audio-3.1-asr-flash-streaming", DashscopeDuplex),
            ("qwen-audio-3.0-asr-flash-streaming", DashscopeDuplex),
            ("qwen-audio-3.1-asr-flash-message", DashscopeDuplex),
            (" Qwen-Audio-3.1-ASR-Flash-Streaming-2026-12-01 ", DashscopeDuplex),
            ("fun-asr-realtime", DashscopeDuplex),
            ("fun-asr-realtime-2026-02-28", DashscopeDuplex),
            ("fun-asr-flash-8k-realtime", DashscopeDuplex),
            ("paraformer-realtime-v2", DashscopeDuplex),
            ("paraformer-realtime-8k-v2", DashscopeDuplex),
            ("qwen-audio-3.1-asr-flash", DashscopeMultimodal),
            ("qwen-audio-3.0-asr-flash", DashscopeMultimodal),
            ("fun-asr-flash-2026-06-15", DashscopeMultimodal),
            ("qwen3-asr-flash", DashscopeChat),
            ("qwen3-asr-flash-2026-02-10", DashscopeChat),
            ("qwen3-asr-flash-realtime", DashscopeUnsupported),
            ("qwen3-asr-flash-realtime-2026-02-10", DashscopeUnsupported),
            ("qwen-audio-3.1-asr-flash-filetrans", DashscopeUnsupported),
            ("qwen3-asr-flash-filetrans", DashscopeUnsupported),
            ("fun-asr", DashscopeUnsupported),
            ("fun-asr-mtl", DashscopeUnsupported),
            ("paraformer-v2", DashscopeUnsupported),
            ("sensevoice-v1", DashscopeUnsupported),
            ("qwen3.8-omni-flash", DashscopeChat),
        ];
        for url in [public, workspace, intl] {
            for (model, want) in table {
                assert_eq!(AsrProtocol::of(url, model), want, "{url} {model}");
            }
        }
        for other in [
            "https://api.openai.com/v1",
            "http://127.0.0.1:8000/v1",
            "https://aliyuncs.com.example.test/v1",
            "https://example.test/aliyuncs.com",
            "not a url",
            "",
        ] {
            assert!(!is_dashscope(other), "{other}");
            assert_eq!(AsrProtocol::of(other, "qwen-audio-3.1-asr-flash-streaming"), OpenaiTranscriptions, "{other}");
        }
        assert!(is_dashscope(" https://dashscope.aliyuncs.com "));
        let streams: Vec<AsrProtocol> = table.iter().map(|(_, p)| *p).filter(|p| p.streams()).collect();
        assert!(streams.iter().all(|p| *p == DashscopeDuplex) && !streams.is_empty());
        for p in [OpenaiTranscriptions, DashscopeChat, DashscopeMultimodal, DashscopeUnsupported] {
            assert!(!p.streams(), "{p:?}");
        }
        // The catalogue's Model Studio models are all usable, the default one streams.
        let aliyun = ProviderId::Aliyun.spec().asr.expect("Model Studio recognises");
        assert!(aliyun.models.iter().all(|m| AsrProtocol::of(aliyun.base_url, m) != DashscopeUnsupported), "{aliyun:?}");
        assert!(AsrProtocol::of(aliyun.base_url, aliyun.models[0]).streams());
    }

    #[test]
    fn google_lists_its_text_models_under_the_names_chat_completions_take() {
        let listed = [
            "models/gemini-3.8-flash",
            "models/gemini-3.8-flash-tts",
            "models/gemini-embedding-001",
            "models/gemini-3.1-flash-image",
            "models/gemini-3.8-live",
            "models/gemma-4-27b-it",
            "models/veo-3.1-generate-preview",
            "models/gemini-2.5-flash-native-audio-preview-12-2025",
            "models/gemini-3.5-transcribe",
            "gemini-2.5-pro",
        ]
        .map(str::to_owned)
        .to_vec();
        let url = "https://generativelanguage.googleapis.com/v1beta/openai";
        assert_eq!(usable_models(ProviderId::Google, ServiceKind::Llm, url, listed.clone()), ["gemini-2.5-pro", "gemini-3.8-flash", "gemma-4-27b-it"]);
        // The custom endpoint's list is left as it came, sorted.
        let mut sorted = listed.clone();
        sorted.sort_unstable();
        assert_eq!(usable_models(ProviderId::Custom, ServiceKind::Llm, url, listed), sorted);
        let google = ProviderId::Google.spec();
        assert!(google.asr.is_none() && google.key == KeyPolicy::Required && google.console_url.is_some());
        assert_eq!(key_entry(ProviderId::Google, ServiceKind::Llm), Some("provider-key.google"));
        assert_eq!(key_env_var("provider-key.google"), "VOLTIP_KEY_GOOGLE");
    }

    /// docs/dictation.md §3.3: a vendor lists every model it hosts; recognition offers the
    /// speech-to-text ones this client can use, clean-up the ones that chat in text, so 测试连接's
    /// count is the select's.
    #[test]
    fn each_service_offers_the_listed_models_it_can_use() {
        let ids = |list: &[&str]| list.iter().map(|m| (*m).to_owned()).collect::<Vec<String>>();
        let openai = ids(&[
            "gpt-6-luna",
            "gpt-4o-mini-transcribe",
            "whisper-1",
            "tts-1",
            "gpt-4o-mini-tts",
            "text-embedding-3-small",
            "omni-moderation-latest",
            "dall-e-3",
            "gpt-image-1",
            "gpt-4o-realtime-preview",
            "gpt-4o-audio-preview",
            "gpt-5-codex",
            "gpt-6-luna",
            "",
        ]);
        let base = "https://api.openai.com/v1";
        assert_eq!(usable_models(ProviderId::Openai, ServiceKind::Asr, base, openai.clone()), ["gpt-4o-mini-transcribe", "whisper-1"]);
        assert_eq!(usable_models(ProviderId::Openai, ServiceKind::Llm, base, openai), ["gpt-6-luna"], "one entry per model, none empty");

        let siliconflow = ids(&[
            "FunAudioLLM/SenseVoiceSmall",
            "TeleAI/TeleSpeechASR",
            "FunAudioLLM/CosyVoice2-0.5B",
            "fishaudio/fish-speech-1.5",
            "BAAI/bge-m3",
            "BAAI/bge-reranker-v2-m3",
            "black-forest-labs/FLUX.1-schnell",
            "Kwai-Kolors/Kolors",
            "Wan-AI/Wan2.2-T2V-A14B",
            "Qwen/Qwen3-8B",
            "deepseek-ai/DeepSeek-V3",
        ]);
        let base = "https://api.siliconflow.cn/v1";
        assert_eq!(
            usable_models(ProviderId::Siliconflow, ServiceKind::Asr, base, siliconflow.clone()),
            ["FunAudioLLM/SenseVoiceSmall", "TeleAI/TeleSpeechASR"]
        );
        assert_eq!(usable_models(ProviderId::Siliconflow, ServiceKind::Llm, base, siliconflow), ["Qwen/Qwen3-8B", "deepseek-ai/DeepSeek-V3"]);

        let groq = ids(&["whisper-large-v3-turbo", "distil-whisper-large-v3-en", "openai/gpt-oss-120b", "meta-llama/llama-guard-4-12b", "playai-tts"]);
        let base = "https://api.groq.com/openai/v1";
        assert_eq!(usable_models(ProviderId::Groq, ServiceKind::Asr, base, groq.clone()), ["distil-whisper-large-v3-en", "whisper-large-v3-turbo"]);
        assert_eq!(usable_models(ProviderId::Groq, ServiceKind::Llm, base, groq), ["openai/gpt-oss-120b"]);

        // Model Studio: only the recognition models this client speaks to.
        let studio = ids(&[
            "qwen-audio-3.1-asr-flash-streaming",
            "qwen-audio-3.1-asr-flash",
            "qwen3-asr-flash",
            "qwen3-asr-flash-realtime",
            "qwen-audio-3.1-asr-flash-filetrans",
            "paraformer-realtime-v2",
            "paraformer-v2",
            "fun-asr",
            "sensevoice-v1",
            "qwen3.8-flash",
            "qwen3-omni-flash",
            "qwen-tts",
            "cosyvoice-v2",
            "text-embedding-v4",
            "gte-rerank-v2",
            "wanx2.1-t2i-turbo",
            "qwen-image",
        ]);
        let base = "https://dashscope.aliyuncs.com/compatible-mode/v1";
        assert_eq!(
            usable_models(ProviderId::Aliyun, ServiceKind::Asr, base, studio.clone()),
            ["paraformer-realtime-v2", "qwen-audio-3.1-asr-flash", "qwen-audio-3.1-asr-flash-streaming", "qwen3-asr-flash"]
        );
        assert_eq!(usable_models(ProviderId::Aliyun, ServiceKind::Llm, base, studio), ["qwen3.8-flash"]);

        // Ark, TokenHub and Zhipu list their image, video and text-in-image models beside the chat ones
        // (2026-10-25).
        let ark = ids(&[
            "doubao-seed-2-1-lite-260915",
            "doubao-seedream-5-0-260128",
            "doubao-seedance-2-5-260628",
            "doubao-embedding-vision-251215",
            "glm-5-3-flash-260828",
        ]);
        assert_eq!(
            usable_models(ProviderId::Volcengine, ServiceKind::Llm, "https://ark.cn-beijing.volces.com/api/v3", ark),
            ["doubao-seed-2-1-lite-260915", "glm-5-3-flash-260828"]
        );
        let tokenhub = ids(&["hy3", "hy4-preview", "hy-image-v3", "vidu-image-q2", "viduq2", "seedream-image-v5.0-pro"]);
        assert_eq!(usable_models(ProviderId::Hunyuan, ServiceKind::Llm, "https://tokenhub.tencentmaas.com/v1", tokenhub), ["hy3", "hy4-preview"]);
        let zhipu = ids(&["glm-5.3-flash", "cogview-4", "cogvideox-3", "glm-ocr", "glm-asr-2512", "embedding-3", "glm-realtime"]);
        assert_eq!(usable_models(ProviderId::Glm, ServiceKind::Llm, "https://open.bigmodel.cn/api/paas/v4", zhipu), ["glm-5.3-flash"]);
        // Ollama's pulled models, without the embedding ones.
        let ollama = ids(&["qwen3:8b", "nomic-embed-text:latest", "llava:7b"]);
        assert_eq!(usable_models(ProviderId::Ollama, ServiceKind::Llm, "http://127.0.0.1:11434/v1", ollama), ["llava:7b", "qwen3:8b"]);
        // The catalogue's own presets survive their own filter.
        for spec in PROVIDERS.iter().filter(|s| s.id != ProviderId::Custom) {
            for kind in [ServiceKind::Asr, ServiceKind::Llm] {
                let Some(preset) = spec.service(kind) else { continue };
                let mut want = ids(preset.models);
                want.sort_unstable();
                assert_eq!(usable_models(spec.id, kind, preset.base_url, ids(preset.models)), want, "{:?} {kind:?}", spec.id);
            }
        }
    }

    #[test]
    fn probe_reports_are_flat_and_tagged() {
        let ok = ProbeReport { provider: ProviderId::Groq, kind: ServiceKind::Llm, outcome: ProbeOutcome::Ok { models: vec!["m".into()], latency_ms: 12 } };
        assert_eq!(serde_json::to_string(&ok).unwrap(), r#"{"provider":"groq","kind":"llm","result":"ok","models":["m"],"latency_ms":12}"#);
        let failed = ProbeReport {
            provider: ProviderId::Builtin,
            kind: ServiceKind::Asr,
            outcome: ProbeOutcome::Failed { reason: ProbeFailure::HttpStatus, status: Some(502), detail: None },
        };
        let json = serde_json::to_string(&failed).unwrap();
        assert_eq!(json, r#"{"provider":"builtin","kind":"asr","result":"failed","reason":"http_status","status":502}"#);
        assert_eq!(serde_json::from_str::<ProbeReport>(&json).unwrap(), failed);
        // docs/dictation.md §3.12: what the service said goes with the reason.
        let said = ProbeReport {
            provider: ProviderId::Openai,
            kind: ServiceKind::Llm,
            outcome: ProbeOutcome::Failed { reason: ProbeFailure::Unauthorized, status: None, detail: Some("Incorrect API key provided".into()) },
        };
        let json = serde_json::to_string(&said).unwrap();
        assert_eq!(json, r#"{"provider":"openai","kind":"llm","result":"failed","reason":"unauthorized","detail":"Incorrect API key provided"}"#);
        assert_eq!(serde_json::from_str::<ProbeReport>(&json).unwrap(), said);
        assert_eq!(ProbeError::new(ProbeFailure::Timeout).with_detail("  ").detail, None, "a blank detail adds nothing");
    }
}
