//! GLM-ASR (docs/dictation.md §3.15): `POST {base}/audio/transcriptions`, multipart with the WAV,
//! the model and the glossary as `hotwords`. A request takes at most 30 seconds of audio, so a
//! longer take goes in pieces cut in its pauses ([`crate::split::quiet_cuts`]), each piece with the
//! text so far as its `prompt`, the context GLM-ASR documents for long recordings.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use reqwest::multipart::{Form, Part};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use tokio::sync::mpsc;
use url::Url;

use crate::client::{Transcript, client_builder, http_error, map_reqwest, retry_after_ms, send, truncate_chars};
use crate::config::{AsrConfig, MAX_ERROR_BODY_CHARS};
use crate::dashscope::{join_sentences, wav_format};
use crate::error::{AsrError, error_fields};
use crate::realtime::SAMPLE_RATE;
use crate::vendor::{RealtimeEvents, RealtimeInput, VendorAsr, endpoint};

/// The longest piece one request carries: GLM-ASR takes 30 seconds, with a margin.
pub const MAX_PIECE_MS: u64 = 28_000;
/// Most hot words a request carries (the API's limit).
pub const MAX_HOTWORDS: usize = 100;
/// Most characters of earlier text sent as a piece's context (the API suggests under 8000).
const MAX_PROMPT_CHARS: usize = 2_000;

/// The GLM-ASR client.
pub(crate) struct ZhipuAsr {
    config: AsrConfig,
    key: String,
    transcriptions: Url,
    models: Url,
    http: Client,
}

impl ZhipuAsr {
    /// The client of `config`: the stored base (`…/api/paas/v4`), the model, the API key.
    pub(crate) fn new(config: AsrConfig) -> Result<Self, AsrError> {
        let key = config
            .token
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .ok_or_else(|| AsrError::InvalidConfig("the credential is missing".into()))?
            .to_owned();
        let transcriptions = endpoint(&config.base_url, false, "/audio/transcriptions")?;
        let models = endpoint(&config.base_url, false, "/models")?;
        let http = config
            .network
            .apply(client_builder())
            .timeout(config.timeout)
            .user_agent(concat!("voltip-asr/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| AsrError::InvalidConfig(format!("http client: {e}")))?;
        Ok(Self { config, key, transcriptions, models, http })
    }

    /// One request: a piece as its WAV, the hot words, the text before it as context.
    async fn piece(&self, wav: Vec<u8>, hotwords: &[String], prompt: Option<&str>) -> Result<String, AsrError> {
        let request = || {
            let file = Part::bytes(wav.clone()).file_name("audio.wav").mime_str("audio/wav").map_err(|e| AsrError::InvalidConfig(e.to_string()))?;
            let mut form = Form::new().part("file", file).text("model", self.model().to_owned()).text("stream", "false");
            // An array field: one part per term, as the SDKs send it.
            for word in hotwords {
                form = form.text("hotwords", word.clone());
            }
            if let Some(prompt) = prompt {
                form = form.text("prompt", prompt.to_owned());
            }
            Ok(self.http.post(self.transcriptions.as_str()).bearer_auth(&self.key).multipart(form))
        };
        let response = send(request).await?;
        let status = response.status();
        let retry = retry_after_ms(response.headers());
        let body = response.text().await.map_err(map_reqwest)?;
        if !status.is_success() {
            return Err(zhipu_error(status, retry, &body));
        }
        let parsed: Answer = serde_json::from_str(&body).map_err(|e| AsrError::BadResponse(format!("{e}: {}", truncate_chars(&body, MAX_ERROR_BODY_CHARS))))?;
        Ok(parsed.text.trim().to_owned())
    }
}

#[derive(Deserialize)]
struct Answer {
    #[serde(default)]
    text: String,
}

#[async_trait]
impl VendorAsr for ZhipuAsr {
    fn model(&self) -> &str {
        self.config.model.trim()
    }

    fn sends_glossary(&self, glossary: &[String]) -> bool {
        !hotwords(glossary).is_empty()
    }

    async fn transcribe(&self, wav: &[u8], _language: Option<&str>, glossary: &[String]) -> Result<Transcript, AsrError> {
        let started = Instant::now();
        let format = wav_format(wav)?;
        if format.sample_rate != SAMPLE_RATE {
            return Err(AsrError::Audio(format!("{} Hz audio; GLM-ASR takes it at {SAMPLE_RATE} Hz here", format.sample_rate)));
        }
        let pcm = &wav[format.data.0..format.data.1];
        let hotwords = hotwords(glossary);
        let mut pieces: Vec<String> = Vec::new();
        for range in crate::split::quiet_cuts(pcm, MAX_PIECE_MS) {
            let before = join_sentences(&pieces);
            let prompt = tail_chars(&before, MAX_PROMPT_CHARS);
            let text = self.piece(crate::split::wav(&pcm[range], SAMPLE_RATE), &hotwords, prompt).await?;
            pieces.push(text);
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
        Err(zhipu_error(status, retry, &response.text().await.unwrap_or_default()))
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
        Err(AsrError::InvalidConfig("GLM-ASR does not recognise while the take is spoken".into()))
    }
}

/// The glossary as `hotwords`: trimmed, no blanks, the first [`MAX_HOTWORDS`].
fn hotwords(glossary: &[String]) -> Vec<String> {
    glossary.iter().map(|t| t.trim()).filter(|t| !t.is_empty()).take(MAX_HOTWORDS).map(str::to_owned).collect()
}

/// The last `max` characters of `text`, `None` when it is empty.
fn tail_chars(text: &str, max: usize) -> Option<&str> {
    let count = text.chars().count();
    if count == 0 {
        return None;
    }
    let skip = count.saturating_sub(max);
    text.char_indices().nth(skip).map(|(at, _)| &text[at..])
}

/// A refusal of GLM's API (`{"error":{"code","message"}}`): its balance and daily limit are used-up
/// quotas (`1113` 欠费, `1304` 日调用上限), its concurrency and rate limits (`1302`, `1303`, `1305`)
/// rate limits; the rest sorts by status (docs/dictation.md §3.15).
fn zhipu_error(status: StatusCode, retry_after_ms: Option<u64>, body: &str) -> AsrError {
    let (code, message) = error_fields(body);
    let said = || voltip_reply::clean(&if message.is_empty() { code.clone() } else { format!("{code}: {message}") });
    match code.as_str() {
        "1113" | "1304" => AsrError::QuotaExhausted { message: voltip_reply::clean(&message), code, retry_at_ms: None },
        "1302" | "1303" | "1305" => AsrError::RateLimited { retry_after_ms, message: said() },
        _ => http_error(status, retry_after_ms, body),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn client(server: &MockServer) -> ZhipuAsr {
        let config = AsrConfig::new(format!("{}/api/paas/v4", server.uri()), "glm-asr-2512").with_token(Some("zk-test".into()));
        ZhipuAsr::new(config).unwrap()
    }

    /// `seconds` of speech-like noise with a pause every 10 s, as a 16 kHz WAV.
    fn take(seconds: usize) -> Vec<u8> {
        let rate = 16_000;
        let pcm: Vec<u8> = (0..seconds * rate)
            .flat_map(|i| {
                let pause = i % (10 * rate) >= 10 * rate - rate / 2;
                let x: i16 = if pause { 0 } else { ((i * 7919 % 2000) as i16) - 1000 };
                x.to_le_bytes()
            })
            .collect();
        crate::split::wav(&pcm, 16_000)
    }

    /// The parts of a multipart body, by name, in order (the values of text parts).
    fn parts(body: &[u8]) -> Vec<(String, String)> {
        let text = String::from_utf8_lossy(body);
        text.split("Content-Disposition: form-data; name=\"")
            .skip(1)
            .map(|part| {
                let name = part.split('"').next().unwrap_or_default().to_owned();
                let value = part.split("\r\n\r\n").nth(1).unwrap_or_default().split("\r\n--").next().unwrap_or_default().to_owned();
                (name, value)
            })
            .collect()
    }

    #[tokio::test]
    async fn a_short_take_is_one_request_with_its_hot_words() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/paas/v4/audio/transcriptions"))
            .and(header("authorization", "Bearer zk-test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "t", "model": "glm-asr-2512", "text": " 你好，Voltip。 " })))
            .mount(&server)
            .await;
        let c = client(&server);
        let terms = vec![" Voltip ".to_owned(), String::new(), "术语".to_owned()];
        assert!(c.sends_glossary(&terms) && !c.sends_glossary(&["  ".into()]));
        let t = c.transcribe(&take(5), Some("zh"), &terms).await.unwrap();
        assert_eq!((t.text.as_str(), t.model.as_str()), ("你好，Voltip。", "glm-asr-2512"));
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        let parts = parts(&requests[0].body);
        let names: Vec<&str> = parts.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["file", "model", "stream", "hotwords", "hotwords"], "no prompt for the first piece");
        let hot: Vec<&str> = parts.iter().filter(|(n, _)| n == "hotwords").map(|(_, v)| v.as_str()).collect();
        assert_eq!(hot, ["Voltip", "术语"]);
        assert_eq!(hotwords(&(0..150).map(|i| format!("t{i}")).collect::<Vec<_>>()).len(), MAX_HOTWORDS);
    }

    /// docs/dictation.md §3.15: a take longer than 30 s goes in pieces cut in its pauses, each with
    /// the text before it as context, the texts joined.
    #[tokio::test]
    async fn a_long_take_goes_in_pieces_with_the_text_so_far_as_context() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/paas/v4/audio/transcriptions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "text": "一段。" })))
            .mount(&server)
            .await;
        let t = client(&server).transcribe(&take(65), None, &[]).await.unwrap();
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 3, "65 s in pieces of at most 28 s");
        assert_eq!(t.text, "一段。一段。一段。");
        for (n, request) in requests.iter().enumerate() {
            let parts = parts(&request.body);
            let prompt = parts.iter().find(|(name, _)| name == "prompt").map(|(_, v)| v.clone());
            assert_eq!(prompt, (n > 0).then(|| "一段。".repeat(n)), "piece {n}");
            let file = request.body.windows(4).position(|w| w == b"RIFF").unwrap();
            let format = wav_format(&request.body[file..]).unwrap();
            let seconds = (format.data.1 - format.data.0) / 32_000;
            assert!(seconds <= 28, "piece {n}: {seconds} s");
        }
        assert_eq!(tail_chars("一二三四", 2), Some("三四"));
        assert_eq!(tail_chars("", 2), None);
    }

    #[tokio::test]
    async fn refusals_keep_their_kind() {
        let cases = [
            (401, json!({ "error": { "code": "1002", "message": "令牌无效" } }), "unauthorized"),
            (429, json!({ "error": { "code": "1113", "message": "余额不足或无可用资源包,请充值。" } }), "quota"),
            (429, json!({ "error": { "code": "1302", "message": "并发数过高" } }), "rate"),
            (400, json!({ "error": { "code": "1214", "message": "音频时长超过限制" } }), "server"),
        ];
        for (status, body, kind) in cases {
            let server = MockServer::start().await;
            Mock::given(method("POST")).respond_with(ResponseTemplate::new(status).set_body_json(body)).mount(&server).await;
            let err = client(&server).transcribe(&take(2), None, &[]).await.unwrap_err();
            let ok = match kind {
                "unauthorized" => matches!(err, AsrError::Unauthorized { .. }),
                "quota" => matches!(&err, AsrError::QuotaExhausted { code, .. } if code == "1113"),
                "rate" => matches!(err, AsrError::RateLimited { .. }),
                _ => matches!(err, AsrError::Server { status: 400, .. }),
            };
            assert!(ok, "{kind}: {err:?}");
        }
        assert!(
            matches!(ZhipuAsr::new(AsrConfig::new("https://open.bigmodel.cn/api/paas/v4", "glm-asr-2512")), Err(AsrError::InvalidConfig(m)) if m.starts_with("the credential"))
        );
    }

    #[tokio::test]
    async fn the_probe_asks_for_the_model_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/paas/v4/models"))
            .and(header("authorization", "Bearer zk-test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [] })))
            .mount(&server)
            .await;
        assert_eq!(client(&server).probe().await, Ok(()));
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({ "error": { "code": "1002", "message": "令牌无效" } })))
            .mount(&server)
            .await;
        assert!(matches!(client(&server).probe().await, Err(AsrError::Unauthorized { .. })));
    }
}
