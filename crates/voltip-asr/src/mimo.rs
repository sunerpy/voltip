//! MiMo-ASR (docs/dictation.md §3.15): `POST {base}/chat/completions` with the take as the user's
//! one `input_audio` part (a `data:audio/wav;base64,…` URI) and the language in `asr_options`; the
//! text is the answer's message. The encoded audio may be 10 MB at most, so a take longer than
//! three minutes goes in pieces cut in its pauses. MiMo takes no hot words: the dictionary corrects
//! its text afterwards.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine as _;
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use url::Url;

use crate::client::{Transcript, client_builder, http_error, map_reqwest, retry_after_ms, send, truncate_chars};
use crate::config::{AsrConfig, MAX_ERROR_BODY_CHARS};
use crate::dashscope::{join_sentences, wav_format};
use crate::error::AsrError;
use crate::realtime::SAMPLE_RATE;
use crate::vendor::{RealtimeEvents, RealtimeInput, VendorAsr, endpoint};

/// The longest piece one request carries: three minutes is 7.7 MB once encoded, within MiMo's
/// 10 MB.
pub const MAX_PIECE_MS: u64 = 180_000;

/// The MiMo-ASR client.
pub(crate) struct MimoAsr {
    config: AsrConfig,
    key: String,
    completions: Url,
    models: Url,
    http: Client,
}

impl MimoAsr {
    /// The client of `config`: the stored base (`…/v1`), the model, the API key.
    pub(crate) fn new(config: AsrConfig) -> Result<Self, AsrError> {
        let key = config
            .token
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .ok_or_else(|| AsrError::InvalidConfig("the credential is missing".into()))?
            .to_owned();
        let completions = endpoint(&config.base_url, false, "/chat/completions")?;
        let models = endpoint(&config.base_url, false, "/models")?;
        let http = config
            .network
            .apply(client_builder())
            .timeout(config.timeout)
            .user_agent(concat!("voltip-asr/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| AsrError::InvalidConfig(format!("http client: {e}")))?;
        Ok(Self { config, key, completions, models, http })
    }

    /// One request for one piece's WAV.
    async fn piece(&self, wav: &[u8], language: Option<&str>) -> Result<String, AsrError> {
        let mut uri = String::from("data:audio/wav;base64,");
        base64::engine::general_purpose::STANDARD.encode_string(wav, &mut uri);
        let mut body = json!({
            "model": self.model(),
            "messages": [{ "role": "user", "content": [{ "type": "input_audio", "input_audio": { "data": uri } }] }],
            "stream": false,
        });
        if let Some(language) = language {
            body["asr_options"] = json!({ "language": language });
        }
        let response = send(|| Ok(self.http.post(self.completions.as_str()).bearer_auth(&self.key).json(&body))).await?;
        let status = response.status();
        let retry = retry_after_ms(response.headers());
        let text = response.text().await.map_err(map_reqwest)?;
        if !status.is_success() {
            return Err(mimo_error(status, retry, &text));
        }
        let answer: Value = serde_json::from_str(&text).map_err(|e| AsrError::BadResponse(format!("{e}: {}", truncate_chars(&text, MAX_ERROR_BODY_CHARS))))?;
        answer
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .map(|t| t.trim().to_owned())
            .ok_or_else(|| AsrError::BadResponse(format!("no choices[0].message.content: {}", truncate_chars(&answer.to_string(), MAX_ERROR_BODY_CHARS))))
    }
}

#[async_trait]
impl VendorAsr for MimoAsr {
    fn model(&self) -> &str {
        self.config.model.trim()
    }

    fn sends_glossary(&self, _glossary: &[String]) -> bool {
        false
    }

    async fn transcribe(&self, wav: &[u8], language: Option<&str>, _glossary: &[String]) -> Result<Transcript, AsrError> {
        let started = Instant::now();
        let format = wav_format(wav)?;
        if format.sample_rate != SAMPLE_RATE {
            return Err(AsrError::Audio(format!("{} Hz audio; MiMo-ASR takes it at {SAMPLE_RATE} Hz here", format.sample_rate)));
        }
        let pcm = &wav[format.data.0..format.data.1];
        let language = language_hint(language);
        let mut pieces = Vec::new();
        for range in crate::split::quiet_cuts(pcm, MAX_PIECE_MS) {
            pieces.push(self.piece(&crate::split::wav(&pcm[range], SAMPLE_RATE), language).await?);
        }
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        Ok(Transcript { text: join_sentences(&pieces), latency_ms, model: self.model().to_owned() })
    }

    /// The key against the model list, as for the vendor's other models.
    async fn probe(&self) -> Result<(), AsrError> {
        let response = send(|| Ok(self.http.get(self.models.as_str()).bearer_auth(&self.key).timeout(Duration::from_secs(10)))).await?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        let retry = retry_after_ms(response.headers());
        Err(mimo_error(status, retry, &response.text().await.unwrap_or_default()))
    }

    fn streams(&self) -> bool {
        false
    }

    async fn realtime(
        &self,
        _language: Option<&str>,
        _glossary: &[String],
        _input: mpsc::UnboundedReceiver<RealtimeInput>,
        _events: RealtimeEvents,
    ) -> Result<(), AsrError> {
        Err(AsrError::InvalidConfig("MiMo-ASR does not recognise while the take is spoken".into()))
    }
}

/// The language MiMo is told: `zh` or `en` (its `asr_options.language`), otherwise its own detection.
fn language_hint(language: Option<&str>) -> Option<&'static str> {
    match language?.trim().split(['-', '_']).next()?.to_ascii_lowercase().as_str() {
        "zh" => Some("zh"),
        "en" => Some("en"),
        _ => None,
    }
}

/// A refusal: an account without balance answers 402, which is a used-up quota (docs/dictation.md
/// §3.5); the rest as every OpenAI-compatible service's.
fn mimo_error(status: StatusCode, retry_after_ms: Option<u64>, body: &str) -> AsrError {
    if status == StatusCode::PAYMENT_REQUIRED {
        let (code, message) = crate::error::error_fields(body);
        return AsrError::QuotaExhausted { code: if code.is_empty() { "402".into() } else { code }, message: voltip_reply::clean(&message), retry_at_ms: None };
    }
    http_error(status, retry_after_ms, body)
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn client(server: &MockServer) -> MimoAsr {
        MimoAsr::new(AsrConfig::new(format!("{}/v1", server.uri()), "mimo-v2.5-asr").with_token(Some("mk-test".into()))).unwrap()
    }

    fn take(seconds: usize) -> Vec<u8> {
        let pcm: Vec<u8> = (0..seconds * 16_000).flat_map(|i| (((i * 7919 % 2000) as i16) - 1000).to_le_bytes()).collect();
        crate::split::wav(&pcm, 16_000)
    }

    #[tokio::test]
    async fn a_take_goes_as_one_input_audio_part_and_the_message_is_its_text() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("authorization", "Bearer mk-test"))
            .and(body_partial_json(json!({ "model": "mimo-v2.5-asr", "stream": false, "asr_options": { "language": "zh" } })))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "choices": [{ "message": { "role": "assistant", "content": " 欢迎使用小米。 " } }] })),
            )
            .mount(&server)
            .await;
        let c = client(&server);
        assert!(!c.sends_glossary(&["术语".into()]), "MiMo takes no hot words");
        let t = c.transcribe(&take(3), Some("zh-CN"), &["术语".into()]).await.unwrap();
        assert_eq!((t.text.as_str(), t.model.as_str()), ("欢迎使用小米。", "mimo-v2.5-asr"));
        let body: Value = serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
        let data = body.pointer("/messages/0/content/0/input_audio/data").and_then(Value::as_str).unwrap();
        assert!(data.starts_with("data:audio/wav;base64,UklGR"), "{}", &data[..40]);
        assert_eq!(body["messages"].as_array().map(Vec::len), Some(1));
        assert_eq!(language_hint(Some("ja")), None);
        assert_eq!(language_hint(Some("EN_us")), Some("en"));
    }

    /// docs/dictation.md §3.15: a take over three minutes goes in pieces, each within the 10 MB part.
    #[tokio::test]
    async fn a_long_take_goes_in_pieces() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "choices": [{ "message": { "content": "片段" } }] })))
            .mount(&server)
            .await;
        let t = client(&server).transcribe(&take(400), None, &[]).await.unwrap();
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 3);
        assert_eq!(t.text, "片段片段片段");
        for request in &requests {
            assert!(request.body.len() < 10 * 1024 * 1024, "{} bytes", request.body.len());
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            assert!(body.get("asr_options").is_none(), "no hint, its own detection");
        }
    }

    #[tokio::test]
    async fn refusals_keep_their_kind() {
        for (status, kind) in [(401, "unauthorized"), (402, "quota"), (429, "rate"), (500, "server")] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(status).set_body_json(json!({ "error": { "code": "x", "message": "refused" } })))
                .mount(&server)
                .await;
            let err = client(&server).transcribe(&take(1), None, &[]).await.unwrap_err();
            let ok = match kind {
                "unauthorized" => matches!(err, AsrError::Unauthorized { .. }),
                "quota" => matches!(err, AsrError::QuotaExhausted { .. }),
                "rate" => matches!(err, AsrError::RateLimited { .. }),
                _ => matches!(err, AsrError::Server { status: 500, .. }),
            };
            assert!(ok, "{status}: {err:?}");
        }
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "choices": [] }))).mount(&server).await;
        assert!(matches!(client(&server).transcribe(&take(1), None, &[]).await, Err(AsrError::BadResponse(_))));
    }
}
