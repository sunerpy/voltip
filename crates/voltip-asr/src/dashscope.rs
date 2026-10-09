//! Alibaba Cloud Model Studio (DashScope) recognition (docs/dictation.md §3.4). One workspace
//! address serves models that speak three protocols, none of them OpenAI's
//! `/audio/transcriptions` (which answers 404 there):
//!
//! * [`DashscopeMode::Chat`] — `qwen3-asr-flash…`: the OpenAI-compatible
//!   `POST /compatible-mode/v1/chat/completions`, the WAV as an `input_audio` data URI.
//! * [`DashscopeMode::Multimodal`] — `qwen-audio-…-asr-flash`, `fun-asr-flash…`: the native
//!   `POST /api/v1/services/aigc/multimodal-generation/generation`.
//! * [`DashscopeMode::Duplex`] — the realtime models: the WebSocket task protocol of
//!   [`crate::duplex`]; a whole take is sent at once and its sentences joined.
//!
//! The endpoints come from the base URL's origin, so the public address, a workspace's own one
//! and either of their paths (`/compatible-mode/v1`, `/api/v1`) all work. The user's dictionary
//! terms go out as instant hot words where the model takes them (the `qwen-audio` models).

use std::time::Instant;

use base64::Engine as _;
use reqwest::{Client, StatusCode};
use serde_json::{Map, Value, json};
use url::Url;

use crate::client::{Transcript, client_builder, map_reqwest, retry_after_ms, send, truncate_chars};
use crate::config::{AsrConfig, MAX_ERROR_BODY_CHARS};
use crate::duplex::{self, DuplexOptions};
use crate::error::{AsrError, error_fields, quota_error};

/// Which of Model Studio's recognition protocols the model speaks (the core decides from the
/// endpoint and the model: `voltip_core::providers::AsrProtocol`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DashscopeMode {
    /// `qwen3-asr-flash…` through the OpenAI-compatible chat completions.
    Chat,
    /// `qwen-audio-…-asr-flash`, `fun-asr-flash…` through the native multimodal generation.
    Multimodal,
    /// The realtime models through the WebSocket task protocol.
    Duplex,
}

/// Weight of a dictionary term as an instant hot word: 1–5, higher is stronger (50 would be a
/// "super" hot word, which Model Studio allows for 50 terms at most).
pub const HOTWORD_WEIGHT: u8 = 4;
/// Most instant hot words a request may carry (Model Studio's limit; the glossary has 200 at most).
pub const MAX_HOTWORDS: usize = 2000;
/// Largest data URI the HTTP models take (10 MB, base64 included): about four minutes of 16 kHz
/// 16-bit mono.
pub const MAX_DATA_URI_BYTES: usize = 10 * 1024 * 1024;

/// The language codes every Model Studio recogniser takes as a hint; any other code (`yue`, which
/// they recognise as Chinese anyway) is left to their detection.
const SHARED_HINTS: [&str; 4] = ["zh", "en", "ja", "ko"];

/// One Model Studio model behind one base URL. Cheap to clone; share it.
#[derive(Clone, Debug)]
pub struct DashscopeClient {
    config: AsrConfig,
    mode: DashscopeMode,
    origin: String,
    http: Client,
}

impl DashscopeClient {
    /// Validate `config` and build the HTTP client (the WebSocket opens per take).
    pub fn new(config: AsrConfig, mode: DashscopeMode) -> Result<Self, AsrError> {
        let origin = origin_of(&config.base_url)?;
        if config.model.trim().is_empty() {
            return Err(AsrError::InvalidConfig("model is empty".into()));
        }
        if config.timeout.is_zero() {
            return Err(AsrError::InvalidConfig("timeout must be greater than zero".into()));
        }
        let http = client_builder()
            .timeout(config.timeout)
            .user_agent(concat!("voltip-asr/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| AsrError::InvalidConfig(format!("http client: {e}")))?;
        Ok(Self { config, mode, origin, http })
    }

    /// The configuration this client was built from (token redacted in `Debug`).
    pub fn config(&self) -> &AsrConfig {
        &self.config
    }

    /// The protocol.
    pub fn mode(&self) -> DashscopeMode {
        self.mode
    }

    /// Where requests go: the HTTP endpoint, or the WebSocket of a realtime model.
    pub fn endpoint(&self) -> String {
        match self.mode {
            DashscopeMode::Chat => format!("{}/compatible-mode/v1/chat/completions", self.origin),
            DashscopeMode::Multimodal => format!("{}/api/v1/services/aigc/multimodal-generation/generation", self.origin),
            DashscopeMode::Duplex => duplex_url(&self.origin),
        }
    }

    /// Recognise a whole WAV take; `language` is a hint, `glossary` the user's dictionary terms.
    pub async fn transcribe(&self, wav: &[u8], language: Option<&str>, glossary: &[String]) -> Result<Transcript, AsrError> {
        match self.mode {
            DashscopeMode::Chat => self.chat(wav, language).await,
            DashscopeMode::Multimodal => self.multimodal(wav, language, glossary).await,
            DashscopeMode::Duplex => duplex::transcribe_whole(self, wav, language, glossary).await,
        }
    }

    /// The realtime task's model and parameters for this take (docs/dictation.md §3.4): 16-bit PCM
    /// at `sample_rate`, the language hint where the model takes one, the glossary as hot words for
    /// the `qwen-audio` models. A `live` session asks for growing partial results (the `…-message`
    /// model sends only finished sentences unless asked) and for heartbeats, so a pause in the
    /// middle of a take does not end the connection.
    pub fn duplex_options(&self, language: Option<&str>, glossary: &[String], sample_rate: u32, live: bool) -> DuplexOptions {
        let model = self.config.model.trim().to_owned();
        let family = Family::of(&model);
        let mut parameters = Map::new();
        parameters.insert("format".into(), json!("pcm"));
        parameters.insert("sample_rate".into(), json!(sample_rate));
        // The `…-message` model takes no language hint.
        if family != Family::QwenAudioMessage
            && let Some(hint) = hint(language, family)
        {
            parameters.insert("language_hints".into(), json!([hint]));
        }
        if family.takes_hotwords()
            && let Some(vocabulary) = hotwords(glossary)
        {
            parameters.insert("vocabulary".into(), vocabulary);
        }
        if live {
            if family == Family::QwenAudioMessage {
                parameters.insert("intermediate_result_enabled".into(), json!(true));
            }
            if family.takes_heartbeat(&model) {
                parameters.insert("heartbeat".into(), json!(true));
            }
        }
        DuplexOptions { url: duplex_url(&self.origin), token: self.config.token.clone(), model, parameters }
    }

    /// `qwen3-asr-flash`: one chat completion with the audio as the user's `input_audio` part.
    /// The glossary is not sent: the model's context goes in a system message whose form the
    /// compatible mode does not document.
    async fn chat(&self, wav: &[u8], language: Option<&str>) -> Result<Transcript, AsrError> {
        let started = Instant::now();
        let mut body = json!({
            "model": self.config.model.trim(),
            "messages": [{ "role": "user", "content": [{ "type": "input_audio", "input_audio": { "data": data_uri(wav)? } }] }],
            "stream": false,
        });
        if let Some(language) = language.map(primary_subtag).filter(|l| !l.is_empty()) {
            body["asr_options"] = json!({ "language": language });
        }
        let answer = self.post(&self.endpoint(), &body, false).await?;
        let text = answer
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .ok_or_else(|| AsrError::BadResponse(format!("no choices[0].message.content: {}", truncate_chars(&answer.to_string(), MAX_ERROR_BODY_CHARS))))?;
        Ok(self.transcript(text, started))
    }

    /// `qwen-audio-…-asr-flash` / `fun-asr-flash…`: the native multimodal generation, answered in
    /// one piece (no SSE).
    async fn multimodal(&self, wav: &[u8], language: Option<&str>, glossary: &[String]) -> Result<Transcript, AsrError> {
        let started = Instant::now();
        let format = wav_format(wav)?;
        let model = self.config.model.trim();
        let family = Family::of(model);
        let mut parameters = Map::new();
        parameters.insert("format".into(), json!("wav"));
        parameters.insert("sample_rate".into(), json!(format.sample_rate.to_string()));
        if let Some(hint) = hint(language, family) {
            parameters.insert("language_hints".into(), json!([hint]));
        }
        if family.takes_hotwords()
            && let Some(vocabulary) = hotwords(glossary)
        {
            parameters.insert("vocabulary".into(), vocabulary);
        }
        let body = json!({
            "model": model,
            "input": { "messages": [{ "role": "user", "content": [{ "type": "input_audio", "input_audio": { "data": data_uri(wav)? } }] }] },
            "parameters": parameters,
        });
        let answer = self.post(&self.endpoint(), &body, true).await?;
        let text = answer
            .pointer("/output/text")
            .or_else(|| answer.get("text"))
            .and_then(Value::as_str)
            .ok_or_else(|| AsrError::BadResponse(format!("no output.text: {}", truncate_chars(&answer.to_string(), MAX_ERROR_BODY_CHARS))))?;
        Ok(self.transcript(text, started))
    }

    async fn post(&self, url: &str, body: &Value, native: bool) -> Result<Value, AsrError> {
        let request = || {
            let mut request = self.http.post(url).json(body);
            if native {
                request = request.header("X-DashScope-SSE", "disable");
            }
            Ok(match &self.config.token {
                Some(token) => request.bearer_auth(token),
                None => request,
            })
        };
        tracing::debug!(mode = ?self.mode, model = %self.config.model, "transcribing");
        let response = send(request).await?;
        let status = response.status();
        let retry = retry_after_ms(response.headers());
        let text = response.text().await.map_err(map_reqwest)?;
        if !status.is_success() {
            return Err(service_error(status, retry, &text));
        }
        serde_json::from_str(&text).map_err(|e| AsrError::BadResponse(format!("{e}: {}", truncate_chars(&text, MAX_ERROR_BODY_CHARS))))
    }

    fn transcript(&self, text: &str, started: Instant) -> Transcript {
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let text = text.trim().to_owned();
        tracing::debug!(latency_ms, chars = text.chars().count(), "transcribed");
        Transcript { text, latency_ms, model: self.config.model.clone() }
    }
}

/// The model families whose request parameters differ.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    /// `qwen-audio-…` (the flash, streaming models): language hints, instant hot words.
    QwenAudio,
    /// `qwen-audio-…-message`: hot words, no language hint, partials only when asked.
    QwenAudioMessage,
    /// `fun-asr…`: one language hint, hot words only as a vocabulary id.
    FunAsr,
    /// `paraformer…`: language hints, hot words only as a vocabulary id.
    Paraformer,
    /// Anything else (`qwen3-asr-flash`).
    Other,
}

impl Family {
    fn of(model: &str) -> Self {
        let model = model.trim().to_ascii_lowercase();
        if model.starts_with("qwen-audio") {
            if model.contains("-message") { Self::QwenAudioMessage } else { Self::QwenAudio }
        } else if model.starts_with("fun-asr") {
            Self::FunAsr
        } else if model.starts_with("paraformer") {
            Self::Paraformer
        } else {
            Self::Other
        }
    }

    fn takes_hotwords(self) -> bool {
        matches!(self, Self::QwenAudio | Self::QwenAudioMessage)
    }

    /// The realtime models that document `heartbeat` (Paraformer: the v2 ones only).
    fn takes_heartbeat(self, model: &str) -> bool {
        match self {
            Self::QwenAudio | Self::QwenAudioMessage | Self::FunAsr => true,
            Self::Paraformer => model.to_ascii_lowercase().contains("-v2"),
            Self::Other => false,
        }
    }
}

/// The language hint `family` takes for `language`: the shared codes, plus Cantonese for
/// Paraformer, which lists it.
fn hint(language: Option<&str>, family: Family) -> Option<String> {
    let code = primary_subtag(language?);
    let takes = SHARED_HINTS.contains(&code.as_str()) || (family == Family::Paraformer && code == "yue");
    takes.then_some(code)
}

/// `zh-CN` → `zh`, trimmed and lower-cased.
fn primary_subtag(language: &str) -> String {
    language.trim().split(['-', '_']).next().unwrap_or_default().to_ascii_lowercase()
}

/// The glossary as Model Studio's instant hot words (`{"term": weight}`); `None` when empty.
fn hotwords(glossary: &[String]) -> Option<Value> {
    let mut words = Map::new();
    for term in glossary.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
        if words.len() == MAX_HOTWORDS {
            break;
        }
        words.insert(term.to_owned(), json!(HOTWORD_WEIGHT));
    }
    (!words.is_empty()).then_some(Value::Object(words))
}

/// The WAV as a `data:audio/wav;base64,…` URI, refused above [`MAX_DATA_URI_BYTES`].
fn data_uri(wav: &[u8]) -> Result<String, AsrError> {
    const PREFIX: &str = "data:audio/wav;base64,";
    let encoded_len = wav.len().div_ceil(3) * 4 + PREFIX.len();
    if encoded_len > MAX_DATA_URI_BYTES {
        return Err(AsrError::Audio(format!("the take is too long for this model ({encoded_len} bytes encoded, at most {MAX_DATA_URI_BYTES})")));
    }
    let mut uri = String::with_capacity(encoded_len);
    uri.push_str(PREFIX);
    base64::engine::general_purpose::STANDARD.encode_string(wav, &mut uri);
    Ok(uri)
}

/// The service root of `base_url` (`https://host[:port]`): every Model Studio endpoint hangs off
/// it, whatever path the settings hold.
pub fn origin_of(base_url: &str) -> Result<String, AsrError> {
    let raw = base_url.trim();
    if raw.is_empty() {
        return Err(AsrError::InvalidConfig("base_url is empty".into()));
    }
    let url = Url::parse(raw).map_err(|e| AsrError::InvalidConfig(format!("base_url {raw:?}: {e}")))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(AsrError::InvalidConfig(format!("base_url {raw:?}: needs http(s) and a host")));
    }
    Ok(url.origin().ascii_serialization())
}

/// Model Studio's OpenAI-compatible base for `base_url` (`https://host/compatible-mode/v1`): where
/// its model list and its chat models are.
pub fn compatible_base(base_url: &str) -> Result<String, AsrError> {
    Ok(format!("{}/compatible-mode/v1", origin_of(base_url)?))
}

/// The realtime WebSocket of `origin` (`wss://host/api-ws/v1/inference`; `ws://` for a plain
/// `http://` origin, as in the tests).
pub(crate) fn duplex_url(origin: &str) -> String {
    let ws = origin.strip_prefix("https://").map(|rest| format!("wss://{rest}")).or_else(|| origin.strip_prefix("http://").map(|rest| format!("ws://{rest}")));
    format!("{}/api-ws/v1/inference", ws.unwrap_or_else(|| origin.to_owned()))
}

/// A Model Studio error answer as an [`AsrError`]: the native body (`{"code", "message"}`) or the
/// compatible one (`{"error": {"code", "message"}}`). The free tier's stop is named; 401 is the
/// key; 5xx stays retryable.
pub(crate) fn service_error(status: StatusCode, retry_after_ms: Option<u64>, body: &str) -> AsrError {
    if let Some(quota) = quota_error(body) {
        return quota;
    }
    let (code, message) = error_fields(body);
    if status == StatusCode::UNAUTHORIZED || code == "InvalidApiKey" {
        return AsrError::Unauthorized;
    }
    if status == StatusCode::TOO_MANY_REQUESTS || code.starts_with("Throttling") {
        return AsrError::RateLimited { retry_after_ms };
    }
    if status.is_server_error() || code.is_empty() {
        if status == StatusCode::FORBIDDEN && code.is_empty() {
            return AsrError::Unauthorized;
        }
        let shown = if code.is_empty() { body.to_owned() } else { format!("{code}: {message}") };
        return AsrError::Server { status: status.as_u16(), body: truncate_chars(&shown, MAX_ERROR_BODY_CHARS) };
    }
    AsrError::Service { code, message: truncate_chars(&message, MAX_ERROR_BODY_CHARS) }
}

/// A WAV's sample format: where its PCM is and at what rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WavFormat {
    pub sample_rate: u32,
    pub data: (usize, usize),
}

/// Read a RIFF/WAVE header: 16-bit PCM mono (what the recorder writes) or refused.
pub(crate) fn wav_format(wav: &[u8]) -> Result<WavFormat, AsrError> {
    let bad = |why: &str| AsrError::Audio(format!("not a 16-bit mono PCM WAV: {why}"));
    if wav.len() < 12 || &wav[..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return Err(bad("no RIFF/WAVE header"));
    }
    let (mut at, mut fmt, mut data) = (12, None, None);
    while at + 8 <= wav.len() {
        let id = &wav[at..at + 4];
        let len = u32::from_le_bytes([wav[at + 4], wav[at + 5], wav[at + 6], wav[at + 7]]) as usize;
        let body = at + 8;
        let end = body.saturating_add(len).min(wav.len());
        match id {
            b"fmt " if end - body >= 16 => {
                let tag = u16::from_le_bytes([wav[body], wav[body + 1]]);
                let channels = u16::from_le_bytes([wav[body + 2], wav[body + 3]]);
                let rate = u32::from_le_bytes([wav[body + 4], wav[body + 5], wav[body + 6], wav[body + 7]]);
                let bits = u16::from_le_bytes([wav[body + 14], wav[body + 15]]);
                fmt = Some((tag, channels, rate, bits));
            }
            b"data" => data = Some((body, end)),
            _ => {}
        }
        at = body.saturating_add(len).saturating_add(len % 2);
    }
    let (tag, channels, sample_rate, bits) = fmt.ok_or_else(|| bad("no fmt chunk"))?;
    if !(tag == 1 || tag == 0xFFFE) || bits != 16 {
        return Err(bad(&format!("format {tag}, {bits} bits")));
    }
    if channels != 1 {
        return Err(bad(&format!("{channels} channels")));
    }
    if sample_rate == 0 {
        return Err(bad("sample rate 0"));
    }
    let data = data.ok_or_else(|| bad("no data chunk"))?;
    Ok(WavFormat { sample_rate, data })
}

/// Sentences joined as they read: nothing between Chinese, Japanese or Korean text (or its
/// punctuation), one space between two pieces of other text.
pub(crate) fn join_sentences<S: AsRef<str>>(sentences: &[S]) -> String {
    let mut out = String::new();
    for sentence in sentences.iter().map(|s| s.as_ref().trim()).filter(|s| !s.is_empty()) {
        let spaced = out.chars().last().is_some_and(|c| !is_cjk(c)) && sentence.chars().next().is_some_and(|c| !is_cjk(c));
        if spaced {
            out.push(' ');
        }
        out.push_str(sentence);
    }
    out
}

/// CJK ideographs, kana, Hangul and their full-width punctuation.
fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3000..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF | 0xAC00..=0xD7AF | 0x20000..=0x2FA1F)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::time::Duration;

    use serde_json::Value;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    /// `samples` of a quiet 16-bit mono saw at `rate` as a WAV.
    pub(crate) fn wav(samples: usize, rate: u32) -> Vec<u8> {
        let data = samples * 2;
        let mut out = Vec::with_capacity(44 + data);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data as u32).to_le_bytes());
        out.extend((0..samples).flat_map(|i| ((i % 100) as i16 * 50).to_le_bytes()));
        out
    }

    fn client(server: &MockServer, model: &str, mode: DashscopeMode) -> DashscopeClient {
        let config = AsrConfig::new(format!("{}/compatible-mode/v1", server.uri()), model).with_token(Some("sk-test".into()));
        DashscopeClient::new(config, mode).unwrap()
    }

    #[test]
    fn endpoints_hang_off_the_origin_whatever_the_path() {
        for base in [
            "https://ws-1.cn-beijing.maas.aliyuncs.com/compatible-mode/v1",
            "https://ws-1.cn-beijing.maas.aliyuncs.com",
            "https://ws-1.cn-beijing.maas.aliyuncs.com/api/v1/",
        ] {
            assert_eq!(origin_of(base).unwrap(), "https://ws-1.cn-beijing.maas.aliyuncs.com");
            assert_eq!(compatible_base(base).unwrap(), "https://ws-1.cn-beijing.maas.aliyuncs.com/compatible-mode/v1");
        }
        assert_eq!(duplex_url("https://dashscope.aliyuncs.com"), "wss://dashscope.aliyuncs.com/api-ws/v1/inference");
        assert_eq!(duplex_url("http://127.0.0.1:8080"), "ws://127.0.0.1:8080/api-ws/v1/inference");
        assert_eq!(duplex_url("odd"), "odd/api-ws/v1/inference");
        for bad in ["", "  ", "ftp://host", "not a url", "file:///x"] {
            assert!(matches!(origin_of(bad), Err(AsrError::InvalidConfig(_))), "{bad:?}");
        }
        let config = |model: &str| AsrConfig::new("https://dashscope.aliyuncs.com/compatible-mode/v1", model);
        let chat = DashscopeClient::new(config("qwen3-asr-flash"), DashscopeMode::Chat).unwrap();
        assert_eq!(chat.endpoint(), "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions");
        assert_eq!((chat.mode(), chat.config().model.as_str()), (DashscopeMode::Chat, "qwen3-asr-flash"));
        let native = DashscopeClient::new(config("qwen-audio-3.1-asr-flash"), DashscopeMode::Multimodal).unwrap();
        assert_eq!(native.endpoint(), "https://dashscope.aliyuncs.com/api/v1/services/aigc/multimodal-generation/generation");
        let realtime = DashscopeClient::new(config("qwen-audio-3.1-asr-flash-streaming"), DashscopeMode::Duplex).unwrap();
        assert_eq!(realtime.endpoint(), "wss://dashscope.aliyuncs.com/api-ws/v1/inference");
        assert!(matches!(DashscopeClient::new(config(" "), DashscopeMode::Chat), Err(AsrError::InvalidConfig(_))));
        assert!(matches!(DashscopeClient::new(config("m").with_timeout(Duration::ZERO), DashscopeMode::Chat), Err(AsrError::InvalidConfig(_))));
        assert!(matches!(DashscopeClient::new(AsrConfig::new("nope", "m"), DashscopeMode::Chat), Err(AsrError::InvalidConfig(_))));
        assert!(!format!("{:?}", DashscopeClient::new(config("m").with_token(Some("sk-secret-x".into())), DashscopeMode::Chat).unwrap()).contains("secret"));
    }

    /// docs/dictation.md §3.4: each family gets the parameters it documents — language hints where
    /// taken, the glossary as hot words for `qwen-audio`, partials and heartbeats for a live take.
    #[test]
    fn realtime_parameters_follow_the_model_family() {
        let config = |model: &str| AsrConfig::new("https://dashscope.aliyuncs.com", model).with_token(Some("sk-test".into()));
        let options = |model: &str, language: Option<&str>, live: bool| {
            DashscopeClient::new(config(model), DashscopeMode::Duplex).unwrap().duplex_options(
                language,
                &["Voltip".into(), " ".into(), "Teams".into()],
                16_000,
                live,
            )
        };
        let streaming = options("qwen-audio-3.1-asr-flash-streaming", Some("zh-CN"), true);
        assert_eq!(streaming.url, "wss://dashscope.aliyuncs.com/api-ws/v1/inference");
        assert_eq!(streaming.token.as_deref(), Some("sk-test"));
        assert_eq!(
            Value::Object(streaming.parameters.clone()),
            json!({ "format": "pcm", "sample_rate": 16000, "language_hints": ["zh"], "vocabulary": { "Voltip": 4, "Teams": 4 }, "heartbeat": true })
        );
        let whole = options("qwen-audio-3.1-asr-flash-streaming", Some("yue"), false);
        assert_eq!(
            Value::Object(whole.parameters),
            json!({ "format": "pcm", "sample_rate": 16000, "vocabulary": { "Voltip": 4, "Teams": 4 } }),
            "Cantonese is detected, not hinted"
        );
        let message = options("qwen-audio-3.1-asr-flash-message", Some("en"), true);
        assert_eq!(
            Value::Object(message.parameters),
            json!({ "format": "pcm", "sample_rate": 16000, "vocabulary": { "Voltip": 4, "Teams": 4 }, "intermediate_result_enabled": true, "heartbeat": true })
        );
        let fun = options("fun-asr-realtime", Some("en"), true);
        assert_eq!(Value::Object(fun.parameters), json!({ "format": "pcm", "sample_rate": 16000, "language_hints": ["en"], "heartbeat": true }));
        let paraformer = options("paraformer-realtime-v2", Some("yue"), true);
        assert_eq!(Value::Object(paraformer.parameters), json!({ "format": "pcm", "sample_rate": 16000, "language_hints": ["yue"], "heartbeat": true }));
        let old = options("paraformer-realtime-v1", None, true);
        assert_eq!(Value::Object(old.parameters), json!({ "format": "pcm", "sample_rate": 16000 }));
        let unknown = options("qwen3-something", Some("ja"), true);
        assert_eq!(Value::Object(unknown.parameters), json!({ "format": "pcm", "sample_rate": 16000, "language_hints": ["ja"] }));
        assert_eq!(hotwords(&[]), None);
        let many: Vec<String> = (0..MAX_HOTWORDS + 5).map(|i| format!("t{i}")).collect();
        assert_eq!(hotwords(&many).and_then(|v| v.as_object().map(Map::len)), Some(MAX_HOTWORDS));
    }

    #[tokio::test]
    async fn chat_sends_the_audio_as_input_audio_and_reads_the_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/compatible-mode/v1/chat/completions"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_partial_json(json!({ "model": "qwen3-asr-flash", "stream": false, "asr_options": { "language": "zh" } })))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "choices": [{ "message": { "role": "assistant", "content": " 欢迎使用阿里云。 " } }] })),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/compatible-mode/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "choices": [{ "message": { "content": "无提示" } }] })))
            .mount(&server)
            .await;
        let c = client(&server, "qwen3-asr-flash", DashscopeMode::Chat);
        let t = c.transcribe(&wav(1600, 16_000), Some("zh-CN"), &["Voltip".into()]).await.unwrap();
        assert_eq!((t.text.as_str(), t.model.as_str()), ("欢迎使用阿里云。", "qwen3-asr-flash"));
        let body: Value = serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
        let data = body.pointer("/messages/0/content/0/input_audio/data").and_then(Value::as_str).unwrap();
        assert!(data.starts_with("data:audio/wav;base64,UklGR"), "{}", &data[..40]);
        assert_eq!(body["messages"].as_array().map(Vec::len), Some(1), "no context message: {body}");
        // No hint: no asr_options at all.
        assert_eq!(c.transcribe(&wav(1600, 16_000), None, &[]).await.unwrap().text, "无提示");
        let body: Value = serde_json::from_slice(&server.received_requests().await.unwrap()[1].body).unwrap();
        assert!(body.get("asr_options").is_none(), "{body}");
        // An answer without the message is a bad response.
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "choices": [] }))).mount(&server).await;
        let err = client(&server, "qwen3-asr-flash", DashscopeMode::Chat).transcribe(&wav(10, 16_000), None, &[]).await.unwrap_err();
        assert!(matches!(&err, AsrError::BadResponse(m) if m.contains("choices[0]")), "{err:?}");
    }

    #[tokio::test]
    async fn multimodal_sends_the_native_body_with_hot_words_and_reads_output_text() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/services/aigc/multimodal-generation/generation"))
            .and(header("x-dashscope-sse", "disable"))
            .and(body_partial_json(json!({ "model": "qwen-audio-3.1-asr-flash", "parameters": { "format": "wav", "sample_rate": "16000", "language_hints": ["en"], "vocabulary": { "Good Idea": 4 } } })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "output": { "text": "我想创建一个Good Idea吧。", "sentence": { "sentence_id": 1, "text": "我想创建一个Good Idea吧。" } },
                "text": "ignored when output.text is there",
                "usage": { "duration": 5 },
                "request_id": "r-1"
            })))
            .mount(&server)
            .await;
        let c = client(&server, "qwen-audio-3.1-asr-flash", DashscopeMode::Multimodal);
        let t = c.transcribe(&wav(1600, 16_000), Some("en"), &["Good Idea".into()]).await.unwrap();
        assert_eq!(t.text, "我想创建一个Good Idea吧。");
        // The top-level `text` serves when `output.text` is missing; fun-asr gets no hot words.
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "text": "fine" }))).mount(&server).await;
        let fun = client(&server, "fun-asr-flash-2026-06-15", DashscopeMode::Multimodal);
        assert_eq!(fun.transcribe(&wav(1600, 8_000), None, &["Voltip".into()]).await.unwrap().text, "fine");
        let body: Value = serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
        assert_eq!(body["parameters"], json!({ "format": "wav", "sample_rate": "8000" }), "{body}");
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_string("{\"output\":{}}")).mount(&server).await;
        let err = client(&server, "qwen-audio-3.1-asr-flash", DashscopeMode::Multimodal).transcribe(&wav(10, 16_000), None, &[]).await.unwrap_err();
        assert!(matches!(&err, AsrError::BadResponse(m) if m.contains("output.text")), "{err:?}");
        let err = client(&server, "qwen-audio-3.1-asr-flash", DashscopeMode::Multimodal).transcribe(b"not audio", None, &[]).await.unwrap_err();
        assert!(matches!(err, AsrError::Audio(_)), "{err:?}");
    }

    /// Model Studio's error bodies as errors a person can act on: the key (401, `InvalidApiKey`),
    /// the free tier's stop (`AllocationQuota.FreeTierOnly`), throttling, the service's own codes,
    /// 5xx retryable.
    #[tokio::test]
    async fn service_errors_are_sorted() {
        let cases = [
            (401, json!({ "code": "InvalidApiKey", "message": "Invalid API-key provided." }), AsrError::Unauthorized),
            (
                403,
                json!({ "code": "AllocationQuota.FreeTierOnly", "message": "The free tier of the model has been exhausted." }),
                AsrError::QuotaExhausted { code: "AllocationQuota.FreeTierOnly".into(), message: "The free tier of the model has been exhausted.".into() },
            ),
            (
                403,
                json!({ "error": { "code": "AllocationQuota.FreeTierOnly", "message": "free tier exhausted", "type": "AllocationQuota.FreeTierOnly" } }),
                AsrError::QuotaExhausted { code: "AllocationQuota.FreeTierOnly".into(), message: "free tier exhausted".into() },
            ),
            (
                400,
                json!({ "error": { "code": "invalid_parameter_error", "message": "url error, please check url！", "type": "invalid_request_error" } }),
                AsrError::Service { code: "invalid_parameter_error".into(), message: "url error, please check url！".into() },
            ),
            (
                404,
                json!({ "code": "InvalidParameter", "message": "Model not exist.", "request_id": "r" }),
                AsrError::Service { code: "InvalidParameter".into(), message: "Model not exist.".into() },
            ),
            (429, json!({ "code": "Throttling.RateQuota", "message": "Requests rate limit exceeded" }), AsrError::RateLimited { retry_after_ms: None }),
            (400, json!({ "code": "Throttling", "message": "busy" }), AsrError::RateLimited { retry_after_ms: None }),
            (500, json!({ "code": "InternalError", "message": "oops" }), AsrError::Server { status: 500, body: "InternalError: oops".into() }),
            (403, json!({}), AsrError::Unauthorized),
            (
                403,
                json!({ "code": "Model.AccessDenied", "message": "no access" }),
                AsrError::Service { code: "Model.AccessDenied".into(), message: "no access".into() },
            ),
            (418, json!({ "code": 7, "message": null }), AsrError::Service { code: "7".into(), message: String::new() }),
        ];
        for (status, body, want) in cases {
            let server = MockServer::start().await;
            Mock::given(method("POST")).respond_with(ResponseTemplate::new(status).set_body_json(body.clone())).mount(&server).await;
            let err = client(&server, "qwen-audio-3.1-asr-flash", DashscopeMode::Multimodal).transcribe(&wav(10, 16_000), None, &[]).await.unwrap_err();
            assert_eq!(err, want, "{status} {body}");
        }
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(502).set_body_string("bad gateway")).mount(&server).await;
        let err = client(&server, "qwen3-asr-flash", DashscopeMode::Chat).transcribe(&wav(10, 16_000), None, &[]).await.unwrap_err();
        assert_eq!(err, AsrError::Server { status: 502, body: "bad gateway".into() });
        assert!(err.is_retryable());
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "2")).mount(&server).await;
        let err = client(&server, "qwen3-asr-flash", DashscopeMode::Chat).transcribe(&wav(10, 16_000), None, &[]).await.unwrap_err();
        assert_eq!(err, AsrError::RateLimited { retry_after_ms: Some(2000) });
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_string("not json")).mount(&server).await;
        let err = client(&server, "qwen3-asr-flash", DashscopeMode::Chat).transcribe(&wav(10, 16_000), None, &[]).await.unwrap_err();
        assert!(matches!(err, AsrError::BadResponse(_)), "{err:?}");
    }

    #[tokio::test]
    async fn a_take_too_long_for_the_http_models_is_refused_before_sending() {
        let server = MockServer::start().await;
        let c = client(&server, "qwen-audio-3.1-asr-flash", DashscopeMode::Multimodal);
        let long = wav(4 * 1024 * 1024, 16_000);
        let err = c.transcribe(&long, None, &[]).await.unwrap_err();
        assert!(matches!(&err, AsrError::Audio(m) if m.contains("too long")), "{err:?}");
        assert!(server.received_requests().await.unwrap().is_empty());
        assert!(data_uri(&wav(100, 16_000)).unwrap().starts_with("data:audio/wav;base64,"));
    }

    #[test]
    fn wav_headers_are_read_or_refused() {
        let ok = wav(160, 16_000);
        let f = wav_format(&ok).unwrap();
        assert_eq!(f.sample_rate, 16_000);
        assert_eq!(f.data, (44, 44 + 320));
        let mut stereo = ok.clone();
        stereo[22] = 2;
        assert!(matches!(wav_format(&stereo), Err(AsrError::Audio(m)) if m.contains("2 channels")));
        let mut eight = ok.clone();
        eight[34] = 8;
        assert!(matches!(wav_format(&eight), Err(AsrError::Audio(m)) if m.contains("8 bits")));
        let mut zero = ok.clone();
        zero[24..28].copy_from_slice(&0u32.to_le_bytes());
        assert!(wav_format(&zero).is_err());
        assert!(wav_format(b"RIFF\0\0\0\0WAVE").is_err(), "no chunks");
        assert!(wav_format(&ok[..36]).is_err(), "no data chunk");
        assert!(wav_format(b"nope").is_err());
        // A LIST chunk before `data` and an odd-sized chunk are skipped.
        let mut listed = ok[..36].to_vec();
        listed.extend_from_slice(b"LIST\x03\0\0\0abc\0");
        listed.extend_from_slice(&ok[36..]);
        assert_eq!(wav_format(&listed).unwrap().data.1 - wav_format(&listed).unwrap().data.0, 320);
    }

    #[test]
    fn sentences_join_like_they_read() {
        assert_eq!(join_sentences(&["我想创建一个 Good Idea 吧。", "比如说。"]), "我想创建一个 Good Idea 吧。比如说。");
        assert_eq!(join_sentences(&["Hello world.", "How are you?"]), "Hello world. How are you?");
        assert_eq!(join_sentences(&["你好。", "Hello."]), "你好。Hello.");
        assert_eq!(join_sentences(&["Hello.", "你好。"]), "Hello.你好。");
        assert_eq!(join_sentences(&[" ", "a", ""]), "a");
        assert_eq!(join_sentences::<&str>(&[]), "");
        assert_eq!(join_sentences(&["안녕하세요.", "네"]), "안녕하세요.네");
    }
}
