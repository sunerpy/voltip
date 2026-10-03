//! The service providers the engines can talk to (docs/dictation.md §3): the service compiled into
//! the build, the on-device models, a few OpenAI-compatible vendors and a custom endpoint. Each
//! provider offers speech recognition, text clean-up (LLM) or both; the catalogue carries the
//! public base URL and a short list of suggested models per service. A provider's key is shared by
//! its services, except for the custom provider, whose two endpoints may be two different servers.
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
    /// SiliconFlow (硅基流动).
    Siliconflow,
    /// Alibaba Cloud Model Studio (阿里云百炼, the DashScope API): its own recognition protocols
    /// ([`AsrProtocol`]), clean-up through its OpenAI-compatible mode.
    Aliyun,
    /// DeepSeek (clean-up only).
    Deepseek,
    /// Ollama on this machine (clean-up only, no key).
    Ollama,
    /// Any OpenAI-compatible endpoint the user enters (a Model Studio address speaks Model
    /// Studio's protocols, [`AsrProtocol::of`]).
    Custom,
}

impl ProviderId {
    /// Every provider in display order.
    pub const ALL: [Self; 9] =
        [Self::Builtin, Self::Local, Self::Openai, Self::Groq, Self::Siliconflow, Self::Aliyun, Self::Deepseek, Self::Ollama, Self::Custom];

    /// Wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::Local => "local",
            Self::Openai => "openai",
            Self::Groq => "groq",
            Self::Siliconflow => "siliconflow",
            Self::Aliyun => "aliyun",
            Self::Deepseek => "deepseek",
            Self::Ollama => "ollama",
            Self::Custom => "custom",
        }
    }

    /// The catalogue entry.
    pub fn spec(self) -> &'static ProviderSpec {
        PROVIDERS.iter().find(|p| p.id == self).unwrap_or(&PROVIDERS[0])
    }
}

/// The two services a provider may offer.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceKind {
    /// Speech recognition (`POST {base}/audio/transcriptions`, or the endpoint's own protocol:
    /// [`AsrProtocol`]).
    Asr,
    /// Text clean-up and voice edit (`POST {base}/chat/completions`).
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

/// A service a vendor offers: its public base URL and the models suggested first (the first one is
/// the default). The user may type any other model id or fetch the provider's list.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ServicePreset {
    /// Public base URL (OpenAI-compatible, including `/v1` where the vendor uses it).
    pub base_url: &'static str,
    /// Suggested model ids, default first.
    pub models: &'static [&'static str],
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
}

impl ProviderSpec {
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

const NO_PRESET: ServicePreset = ServicePreset { base_url: "", models: &[] };

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
    /// the background (`…-filetrans`, `fun-asr`, `fun-asr-mtl`, `paraformer-v2`), and
    /// `qwen3-asr-flash-realtime`, whose realtime protocol this client does not speak.
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
        if model.contains("filetrans") || model.starts_with("qwen3-asr-flash-realtime") {
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

/// The catalogue, in display order. Model ids checked against the vendors' documentation on
/// 2026-09-27; lists go stale, so the UI also offers the provider's own `GET /models`.
pub const PROVIDERS: &[ProviderSpec] = &[
    ProviderSpec { id: ProviderId::Builtin, asr: Some(NO_PRESET), llm: Some(NO_PRESET), key: KeyPolicy::Builtin, on_device: false, console_url: None },
    ProviderSpec { id: ProviderId::Local, asr: Some(NO_PRESET), llm: None, key: KeyPolicy::None, on_device: true, console_url: None },
    ProviderSpec {
        id: ProviderId::Openai,
        asr: Some(ServicePreset { base_url: "https://api.openai.com/v1", models: &["gpt-transcribe", "gpt-4o-mini-transcribe", "whisper-1"] }),
        llm: Some(ServicePreset { base_url: "https://api.openai.com/v1", models: &["gpt-6-luna", "gpt-6-sol"] }),
        key: KeyPolicy::Required,
        on_device: false,
        console_url: Some("https://platform.openai.com/api-keys"),
    },
    ProviderSpec {
        id: ProviderId::Groq,
        asr: Some(ServicePreset { base_url: "https://api.groq.com/openai/v1", models: &["whisper-large-v3-turbo", "whisper-large-v3"] }),
        llm: Some(ServicePreset { base_url: "https://api.groq.com/openai/v1", models: &["qwen/qwen3.8-27b", "openai/gpt-oss-20b", "llama-3.3-70b-versatile"] }),
        key: KeyPolicy::Required,
        on_device: false,
        console_url: Some("https://console.groq.com/keys"),
    },
    ProviderSpec {
        id: ProviderId::Siliconflow,
        asr: Some(ServicePreset { base_url: "https://api.siliconflow.cn/v1", models: &["FunAudioLLM/SenseVoiceSmall", "TeleAI/TeleSpeechASR"] }),
        llm: Some(ServicePreset { base_url: "https://api.siliconflow.cn/v1", models: &["Qwen/Qwen3-8B", "deepseek-ai/DeepSeek-V3"] }),
        key: KeyPolicy::Required,
        on_device: false,
        console_url: Some("https://cloud.siliconflow.cn/account/ak"),
    },
    // Model Studio (checked 2026-10-04): the realtime `…-streaming` model first, since it recognises
    // while the user speaks (§3.4); `…-asr-flash` takes a whole file over HTTP. A workspace's own
    // address (`https://<workspace>.cn-beijing.maas.aliyuncs.com/compatible-mode/v1`) replaces the
    // public one in the URL field.
    ProviderSpec {
        id: ProviderId::Aliyun,
        asr: Some(ServicePreset {
            base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
            models: &[
                "qwen-audio-3.1-asr-flash-streaming",
                "qwen-audio-3.1-asr-flash",
                "qwen-audio-3.1-asr-flash-message",
                "qwen3-asr-flash",
                "fun-asr-realtime",
            ],
        }),
        llm: Some(ServicePreset { base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1", models: &["qwen3.8-flash", "qwen3.8-max", "qwen3.7-flash"] }),
        key: KeyPolicy::Required,
        on_device: false,
        console_url: Some("https://bailian.console.aliyun.com/cn-beijing/model/settings/api-key"),
    },
    ProviderSpec {
        id: ProviderId::Deepseek,
        asr: None,
        llm: Some(ServicePreset { base_url: "https://api.deepseek.com", models: &["deepseek-flash", "deepseek-v4-pro"] }),
        key: KeyPolicy::Required,
        on_device: false,
        console_url: Some("https://platform.deepseek.com/api_keys"),
    },
    ProviderSpec {
        id: ProviderId::Ollama,
        asr: None,
        llm: Some(ServicePreset { base_url: "http://127.0.0.1:11434/v1", models: &[] }),
        key: KeyPolicy::None,
        on_device: true,
        console_url: None,
    },
    ProviderSpec { id: ProviderId::Custom, asr: Some(NO_PRESET), llm: Some(NO_PRESET), key: KeyPolicy::Optional, on_device: false, console_url: None },
];

/// The secret-store entry holding the user's key for `provider`'s `kind` service, or `None` when
/// the provider takes no user key (built-in, on-device, Ollama). A vendor's services share one key;
/// the custom provider keeps one per service.
pub fn key_entry(provider: ProviderId, kind: ServiceKind) -> Option<&'static str> {
    match (provider, kind) {
        (ProviderId::Builtin | ProviderId::Local | ProviderId::Ollama, _) => None,
        (ProviderId::Openai, _) => Some("provider-key.openai"),
        (ProviderId::Groq, _) => Some("provider-key.groq"),
        (ProviderId::Siliconflow, _) => Some("provider-key.siliconflow"),
        (ProviderId::Aliyun, _) => Some("provider-key.aliyun"),
        (ProviderId::Deepseek, _) => Some("provider-key.deepseek"),
        (ProviderId::Custom, ServiceKind::Asr) => Some("provider-key.custom-asr"),
        (ProviderId::Custom, ServiceKind::Llm) => Some("provider-key.custom-llm"),
    }
}

/// Every secret-store entry a user key may live in (loaded at startup).
pub fn key_entries() -> Vec<&'static str> {
    let mut out: Vec<&'static str> =
        ProviderId::ALL.into_iter().flat_map(|p| [ServiceKind::Asr, ServiceKind::Llm].map(|k| key_entry(p, k))).flatten().collect();
    out.sort_unstable();
    out.dedup();
    out
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ProbeError {
    /// Why.
    pub reason: ProbeFailure,
    /// The HTTP status of [`ProbeFailure::HttpStatus`].
    pub status: Option<u16>,
}

impl ProbeError {
    /// A failure without an HTTP status.
    pub const fn new(reason: ProbeFailure) -> Self {
        Self { reason, status: None }
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
                        assert!(!preset.models.is_empty(), "{:?} {kind:?} has a default model", spec.id);
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
        assert_eq!(entries.len(), 7, "{entries:?}");
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
    fn probe_reports_are_flat_and_tagged() {
        let ok = ProbeReport { provider: ProviderId::Groq, kind: ServiceKind::Llm, outcome: ProbeOutcome::Ok { models: vec!["m".into()], latency_ms: 12 } };
        assert_eq!(serde_json::to_string(&ok).unwrap(), r#"{"provider":"groq","kind":"llm","result":"ok","models":["m"],"latency_ms":12}"#);
        let failed = ProbeReport {
            provider: ProviderId::Builtin,
            kind: ServiceKind::Asr,
            outcome: ProbeOutcome::Failed { reason: ProbeFailure::HttpStatus, status: Some(502) },
        };
        let json = serde_json::to_string(&failed).unwrap();
        assert_eq!(json, r#"{"provider":"builtin","kind":"asr","result":"failed","reason":"http_status","status":502}"#);
        assert_eq!(serde_json::from_str::<ProbeReport>(&json).unwrap(), failed);
    }
}
