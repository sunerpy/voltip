//! The HTTP client.

use std::time::{Duration, Instant};

use reqwest::header::{HeaderMap, RETRY_AFTER};
use reqwest::multipart::{Form, Part};
use reqwest::{Client, StatusCode};
use serde::Deserialize;

use crate::config::{AsrConfig, MAX_ERROR_BODY_CHARS, normalize_base_url};
use crate::error::{AsrError, quota_error};

/// The HTTP client builder with the trust roots of this platform. Elsewhere reqwest verifies with
/// the system's own store (rustls-platform-verifier); on Android that verifier needs a JNI context
/// the app never hands it and panics on the first request, so the requests there trust Mozilla's
/// root store, as the relay connection does (`voltip-transport`). voltip-refine does the same.
pub(crate) fn client_builder() -> reqwest::ClientBuilder {
    let builder = Client::builder();
    #[cfg(target_os = "android")]
    let builder = builder.tls_certs_only(mozilla_roots());
    builder
}

/// The pauses before a request goes again after its connection failed: DNS, TCP or the TLS
/// handshake, so nothing reached the service and any request may be sent again. A timeout is not
/// tried again: the time the caller allowed is spent, and the service may be working on it.
/// 2026-10-09: computers and phones in China lose the odd handshake to the built-in service in
/// Seoul (`client error (Connect): tls handshake eof`); one more try a moment later gets through.
pub(crate) const CONNECT_RETRY_PAUSES: [Duration; 2] = [Duration::from_millis(300), Duration::from_millis(1000)];

/// Send the request `build` makes, and a new one after each of [`CONNECT_RETRY_PAUSES`] while the
/// connection fails; the answer, or the last failure (with the number of attempts).
pub(crate) async fn send(build: impl Fn() -> Result<reqwest::RequestBuilder, AsrError>) -> Result<reqwest::Response, AsrError> {
    let mut pauses = CONNECT_RETRY_PAUSES.iter();
    let mut attempts = 1usize;
    loop {
        let error = match build()?.send().await {
            Ok(response) => return Ok(response),
            Err(error) => error,
        };
        let pause = if error.is_connect() && !error.is_timeout() { pauses.next() } else { None };
        match (pause, map_reqwest(error)) {
            (Some(pause), failure) => {
                tracing::info!(attempt = attempts, error = %failure, ?pause, "could not connect; trying again");
                tokio::time::sleep(*pause).await;
                attempts += 1;
            }
            (None, AsrError::Network(message)) if attempts > 1 => return Err(AsrError::Network(format!("{message} ({attempts} attempts)"))),
            (None, failure) => return Err(failure),
        }
    }
}

/// Mozilla's root store (webpki-root-certs) as reqwest certificates.
#[cfg(any(target_os = "android", test))]
fn mozilla_roots() -> Vec<reqwest::Certificate> {
    webpki_root_certs::TLS_SERVER_ROOT_CERTS.iter().filter_map(|der| reqwest::Certificate::from_der(der.as_ref()).ok()).collect()
}

/// The service's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transcript {
    /// Recognised text, trimmed. May be empty when the service heard nothing.
    pub text: String,
    /// Wall time from sending the request to having the body parsed.
    pub latency_ms: u64,
    /// The model that was asked for.
    pub model: String,
}

/// Shape of a successful `/v1/audio/transcriptions` body (OpenAI and vLLM). Other fields
/// (`language`, `duration`, `segments`, `usage`) are ignored.
#[derive(Deserialize)]
struct TranscriptionResponse {
    text: String,
}

/// One connection pool to one transcription service. Cheap to clone; share it.
#[derive(Clone, Debug)]
pub struct AsrClient {
    config: AsrConfig,
    endpoint: String,
    http: Client,
}

impl AsrClient {
    /// Validate `config`, normalise its base URL and build the HTTP client.
    pub fn new(config: AsrConfig) -> Result<Self, AsrError> {
        let base = normalize_base_url(&config.base_url)?;
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
        Ok(Self::with_http(config, &base, http))
    }

    /// The client of `config` (base URL `base`, normalised) on the HTTP client `http`.
    fn with_http(config: AsrConfig, base: &str, http: Client) -> Self {
        let endpoint = format!("{base}/audio/transcriptions");
        Self { config, endpoint, http }
    }

    /// The configuration this client was built from (token still redacted in `Debug`).
    pub fn config(&self) -> &AsrConfig {
        &self.config
    }

    /// Full URL the WAV is posted to.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Upload `wav` (a complete RIFF/WAVE file) as multipart `file` with `model`,
    /// `response_format=json` and, when given, `language` (ISO-639-1 such as `zh`, `en`).
    pub async fn transcribe(&self, wav: &[u8], language: Option<&str>) -> Result<Transcript, AsrError> {
        self.transcribe_with_prompt(wav, language, None).await
    }

    /// [`AsrClient::transcribe`] plus the optional OpenAI `prompt` field (docs/dictation.md §16.3:
    /// the user's dictionary terms), sent only when non-blank. vLLM's Qwen3-ASR puts it into the
    /// system turn as recognition context; Whisper-style endpoints read it as preceding text.
    pub async fn transcribe_with_prompt(&self, wav: &[u8], language: Option<&str>, prompt: Option<&str>) -> Result<Transcript, AsrError> {
        let started = Instant::now();
        let language = language.map(str::trim).filter(|l| !l.is_empty());
        let prompt = prompt.map(str::trim).filter(|p| !p.is_empty());
        // A multipart form is sent once: each attempt builds its own.
        let request = || {
            let file = Part::bytes(wav.to_vec()).file_name("audio.wav").mime_str("audio/wav").map_err(|e| AsrError::InvalidConfig(e.to_string()))?;
            let mut form = Form::new().part("file", file).text("model", self.config.model.clone()).text("response_format", "json");
            if let Some(language) = language {
                form = form.text("language", language.to_string());
            }
            if let Some(prompt) = prompt {
                form = form.text("prompt", prompt.to_string());
            }
            let request = self.http.post(&self.endpoint).multipart(form);
            Ok(match &self.config.token {
                Some(token) => request.bearer_auth(token),
                None => request,
            })
        };
        // The prompt is the user's vocabulary: its length goes to the log, never its words.
        tracing::debug!(path = log_path(&self.endpoint), model = %self.config.model, bytes = wav.len(), ?language, prompt_chars = prompt.map_or(0, |p| p.chars().count()), "transcribing");
        let response = send(request).await?;
        let status = response.status();
        if !status.is_success() {
            let retry_after_ms = retry_after_ms(response.headers());
            // The body of a refusal is only read to sort it: one that cannot be read sorts by status.
            let body = response.text().await.unwrap_or_default();
            return Err(http_error(status, retry_after_ms, &body));
        }
        let body = response.text().await.map_err(map_reqwest)?;
        let parsed: TranscriptionResponse =
            serde_json::from_str(&body).map_err(|e| AsrError::BadResponse(format!("{e}: {}", truncate_chars(&body, MAX_ERROR_BODY_CHARS))))?;
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let text = parsed.text.trim().to_string();
        tracing::debug!(latency_ms, chars = text.chars().count(), "transcribed");
        Ok(Transcript { text, latency_ms, model: self.config.model.clone() })
    }
}

/// A non-2xx answer: a used-up quota first, whatever the status (docs/dictation.md §3.5; some
/// services send OpenAI's `insufficient_quota` with 403, others with 429), then by status.
fn http_error(status: StatusCode, retry_after_ms: Option<u64>, body: &str) -> AsrError {
    if let Some(quota) = quota_error(body) {
        return quota;
    }
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return AsrError::Unauthorized;
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        return AsrError::RateLimited { retry_after_ms };
    }
    AsrError::Server { status: status.as_u16(), body: truncate_chars(body, MAX_ERROR_BODY_CHARS) }
}

/// A reqwest failure, split into timeout and everything else.
pub(crate) fn map_reqwest(error: reqwest::Error) -> AsrError {
    if error.is_timeout() {
        return AsrError::Timeout;
    }
    // No URL in the message: it reaches the UI and the history, and the built-in service's host
    // must not (docs/dictation.md §3). reqwest's Display already chains the source.
    let error = error.without_url();
    let mut message = error.to_string();
    let mut source = std::error::Error::source(&error);
    while let Some(inner) = source {
        let text = inner.to_string();
        if !message.contains(&text) {
            message.push_str(": ");
            message.push_str(&text);
        }
        source = inner.source();
    }
    AsrError::Network(message)
}

/// `Retry-After` as milliseconds when it is a delay in seconds (HTTP-dates are not parsed).
pub(crate) fn retry_after_ms(headers: &HeaderMap) -> Option<u64> {
    headers.get(RETRY_AFTER)?.to_str().ok()?.trim().parse::<u64>().ok()?.checked_mul(1000)
}

/// The first `max` characters of `text`.
pub(crate) fn truncate_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// The path of `endpoint` without scheme and host (`https://h.example/v1/x` → `/v1/x`), for log
/// lines: a debug log users paste into a bug report must not carry the built-in service's host.
fn log_path(endpoint: &str) -> &str {
    let rest = endpoint.split_once("://").map_or(endpoint, |(_, rest)| rest);
    rest.find('/').map_or("/", |i| &rest[i..])
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use reqwest::header::HeaderValue;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    /// Regression (2026-10-01, the phone recognising its own takes): on Android reqwest's platform
    /// verifier needs a JNI context the app never hands it and panics on the first request
    /// ("Expect rustls-platform-verifier to be initialized"). The client there trusts Mozilla's
    /// root store instead; the store loads in full and makes a client.
    #[test]
    fn regression_android_trusts_mozillas_roots_not_an_uninitialised_platform_verifier() {
        let roots = mozilla_roots();
        assert!(roots.len() > 100, "{} roots", roots.len());
        assert_eq!(roots.len(), webpki_root_certs::TLS_SERVER_ROOT_CERTS.len(), "every root parses");
        assert!(Client::builder().tls_certs_only(roots).build().is_ok());
        assert!(client_builder().build().is_ok());
    }

    /// Regression (2026-09-27): the debug log carried the whole endpoint URL, host included.
    #[test]
    fn regression_the_request_log_names_the_path_not_the_host() {
        assert_eq!(log_path("https://asr.example.test/v1/audio/transcriptions"), "/v1/audio/transcriptions");
        assert_eq!(log_path("http://127.0.0.1:8001/v1/chat/completions"), "/v1/chat/completions");
        assert_eq!(log_path("https://host.example.test"), "/");
        assert_eq!(log_path("/v1/models"), "/v1/models");
    }

    const WAV: &[u8] = b"RIFF\x24\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\x3e\0\0\0\x7d\0\0\x02\0\x10\0data\0\0\0\0";

    /// A listener that takes every connection, reads the client's hello and closes its side, so a
    /// TLS client sees the peer leave during the handshake (`tls handshake eof`; closing before
    /// reading would reset the connection instead); the number of connections it took.
    async fn closing_listener() -> (std::net::SocketAddr, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let taken = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = taken.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let mut hello = [0u8; 4096];
                let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut hello).await;
                let _ = tokio::io::AsyncWriteExt::shutdown(&mut socket).await;
            }
        });
        (addr, taken)
    }

    /// Regression (2026-10-09, a phone's take recognised on the computer): `ASR network error:
    /// error sending request: client error (Connect): tls handshake eof` ended the take at once.
    /// A handshake the network cuts short is now tried twice more before the take fails, and the
    /// failure says how many attempts it made. The listener counts each connection before it
    /// closes it, so the count is final once the last attempt has failed.
    #[tokio::test]
    async fn regression_a_handshake_cut_short_is_tried_again_before_the_take_fails() {
        let (addr, taken) = closing_listener().await;
        let client = AsrClient::new(AsrConfig::new(format!("https://{addr}"), "m").with_timeout(Duration::from_secs(30))).unwrap();
        match client.transcribe(WAV, None).await.unwrap_err() {
            AsrError::Network(message) => {
                assert!(message.contains("tls handshake eof"), "{message}");
                assert!(message.ends_with("(3 attempts)"), "{message}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(taken.load(std::sync::atomic::Ordering::SeqCst), 1 + CONNECT_RETRY_PAUSES.len());
    }

    /// A resolver that sends the first connection to `first` and every later one to `then`.
    struct Moving {
        calls: std::sync::atomic::AtomicUsize,
        first: std::net::SocketAddr,
        then: std::net::SocketAddr,
    }

    impl reqwest::dns::Resolve for Moving {
        fn resolve(&self, _name: reqwest::dns::Name) -> reqwest::dns::Resolving {
            let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let addr = if call == 0 { self.first } else { self.then };
            Box::pin(async move { Ok(Box::new(std::iter::once(addr)) as reqwest::dns::Addrs) })
        }
    }

    /// A connection refused once (nobody listens where the first address points) is tried again,
    /// and the second connection reaches the service: the take goes through, with one request.
    #[tokio::test]
    async fn a_connection_refused_once_is_tried_again_and_the_take_goes_through() {
        let server = MockServer::start().await;
        mount(&server, ResponseTemplate::new(200).set_body_json(json!({ "text": "好" }))).await;
        // Bound and dropped: nobody listens on this port any more.
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        let resolver = std::sync::Arc::new(Moving { calls: std::sync::atomic::AtomicUsize::new(0), first: closed, then: *server.address() });
        // No proxy: an HTTP(S)_PROXY of the machine running the test would take `asr.test` away.
        let http = client_builder().no_proxy().dns_resolver(resolver.clone()).build().unwrap();
        let client = AsrClient::with_http(AsrConfig::new("http://asr.test", "m"), "http://asr.test/v1", http);
        assert_eq!(client.transcribe(WAV, None).await.unwrap().text, "好");
        assert_eq!(resolver.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    /// A timeout is not tried again: the request may have reached the service, and the time the
    /// caller allowed is spent. The listener accepts nothing, so every connection the client
    /// opened is still in its queue when the call returns; it holds exactly one.
    #[tokio::test]
    async fn a_timeout_is_not_tried_again() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let client = AsrClient::new(AsrConfig::new(format!("https://{addr}"), "m").with_timeout(Duration::from_millis(300))).unwrap();
        assert_eq!(client.transcribe(WAV, None).await.unwrap_err(), AsrError::Timeout);
        listener.set_nonblocking(true).unwrap();
        let queued = std::iter::from_fn(|| listener.accept().ok()).count();
        assert_eq!(queued, 1);
    }

    fn client(server: &MockServer, token: Option<&str>) -> AsrClient {
        AsrClient::new(AsrConfig::new(server.uri(), "Qwen/Qwen3-ASR-1.7B").with_token(token.map(str::to_string))).unwrap()
    }

    async fn mount(server: &MockServer, response: ResponseTemplate) {
        Mock::given(method("POST")).and(path("/v1/audio/transcriptions")).respond_with(response).mount(server).await;
    }

    #[tokio::test]
    async fn success_sends_multipart_with_token_and_language() {
        let server = MockServer::start().await;
        mount(&server, ResponseTemplate::new(200).set_body_json(json!({ "text": "  你好，世界 ", "language": "zh", "duration": 1.2 }))).await;
        let client = client(&server, Some("app-token"));
        assert_eq!(client.endpoint(), format!("{}/v1/audio/transcriptions", server.uri()));
        assert_eq!(client.config().model, "Qwen/Qwen3-ASR-1.7B");

        let transcript = client.transcribe(WAV, Some(" zh ")).await.unwrap();
        assert_eq!(transcript.text, "你好，世界");
        assert_eq!(transcript.model, "Qwen/Qwen3-ASR-1.7B");
        assert!(transcript.latency_ms < 10_000);

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.headers.get("authorization").unwrap(), "Bearer app-token");
        let content_type = request.headers.get("content-type").unwrap().to_str().unwrap();
        assert!(content_type.starts_with("multipart/form-data; boundary="), "{content_type}");
        assert!(request.headers.get("user-agent").unwrap().to_str().unwrap().starts_with("voltip-asr/"));
        let body = String::from_utf8_lossy(&request.body);
        assert!(body.contains("name=\"file\"; filename=\"audio.wav\""), "{body}");
        assert!(body.contains("Content-Type: audio/wav"), "{body}");
        assert!(body.contains("RIFF"), "{body}");
        assert!(body.contains("name=\"model\"\r\n\r\nQwen/Qwen3-ASR-1.7B"), "{body}");
        assert!(body.contains("name=\"response_format\"\r\n\r\njson"), "{body}");
        assert!(body.contains("name=\"language\"\r\n\r\nzh\r\n"), "{body}");
    }

    /// docs/dictation.md §16.3: the dictionary terms travel as the multipart `prompt` field, and only
    /// when there are any — no field for `None` or a blank prompt.
    #[tokio::test]
    async fn the_prompt_field_is_sent_only_when_non_empty() {
        let server = MockServer::start().await;
        mount(&server, ResponseTemplate::new(200).set_body_json(json!({ "text": "我想创建一个 good idea" }))).await;
        let client = client(&server, None);
        let transcript = client.transcribe_with_prompt(WAV, Some("zh"), Some(" good idea, Teams ")).await.unwrap();
        assert_eq!(transcript.text, "我想创建一个 good idea");
        client.transcribe_with_prompt(WAV, None, Some("   ")).await.unwrap();
        client.transcribe_with_prompt(WAV, None, None).await.unwrap();
        client.transcribe(WAV, None).await.unwrap();
        let requests = server.received_requests().await.unwrap();
        let bodies: Vec<String> = requests.iter().map(|r| String::from_utf8_lossy(&r.body).into_owned()).collect();
        assert!(bodies[0].contains("name=\"prompt\"\r\n\r\ngood idea, Teams\r\n"), "{}", bodies[0]);
        assert!(bodies[0].contains("name=\"language\"\r\n\r\nzh\r\n"), "the other fields stay: {}", bodies[0]);
        for body in &bodies[1..] {
            assert!(!body.contains("name=\"prompt\""), "{body}");
        }
    }

    #[tokio::test]
    async fn no_token_no_language_and_extra_fields_ignored() {
        let server = MockServer::start().await;
        mount(&server, ResponseTemplate::new(200).set_body_string(r#"{"text":"hello","segments":[{"id":0}],"usage":{"seconds":3}}"#)).await;
        let client = client(&server, None);
        let transcript = client.transcribe(WAV, None).await.unwrap();
        assert_eq!(transcript.text, "hello");
        let blank = client.transcribe(WAV, Some("   ")).await.unwrap();
        assert_eq!(blank.text, "hello");
        for request in server.received_requests().await.unwrap() {
            assert!(request.headers.get("authorization").is_none());
            let body = String::from_utf8_lossy(&request.body);
            assert!(!body.contains("name=\"language\""), "{body}");
        }
    }

    #[tokio::test]
    async fn unauthorized_on_401_and_403() {
        for status in [401_u16, 403] {
            let server = MockServer::start().await;
            mount(&server, ResponseTemplate::new(status).set_body_string("nope")).await;
            let err = client(&server, Some("bad")).transcribe(WAV, None).await.unwrap_err();
            assert_eq!(err, AsrError::Unauthorized, "status {status}");
            assert!(!err.is_retryable());
        }
    }

    /// docs/dictation.md §3.5: a used-up quota is told by the body, whatever the status, so a
    /// fallback model list can move on; a refusal without it keeps its status's meaning.
    #[tokio::test]
    async fn a_used_up_quota_is_read_from_the_body_of_any_refusal() {
        let insufficient = r#"{"error":{"message":"You exceeded your current quota, please check your plan and billing details.","type":"insufficient_quota","param":null,"code":"insufficient_quota"}}"#;
        let free_tier = r#"{"error":{"code":"AllocationQuota.FreeTierOnly","message":"The free tier of the model has been exhausted."}}"#;
        let cases = [
            (429_u16, insufficient, "insufficient_quota"),
            (403, insufficient, "insufficient_quota"),
            (403, free_tier, "AllocationQuota.FreeTierOnly"),
            (400, free_tier, "AllocationQuota.FreeTierOnly"),
        ];
        for (status, body, code) in cases {
            let server = MockServer::start().await;
            mount(&server, ResponseTemplate::new(status).set_body_string(body)).await;
            let err = client(&server, Some("k")).transcribe(WAV, None).await.unwrap_err();
            assert!(matches!(&err, AsrError::QuotaExhausted { code: c, .. } if c == code), "{status} {body}: {err:?}");
            assert!(!err.is_retryable());
        }
        let server = MockServer::start().await;
        mount(&server, ResponseTemplate::new(429).set_body_string(r#"{"error":{"code":"rate_limit_exceeded","message":"Rate limit reached"}}"#)).await;
        assert_eq!(client(&server, None).transcribe(WAV, None).await.unwrap_err(), AsrError::RateLimited { retry_after_ms: None });
        let server = MockServer::start().await;
        mount(&server, ResponseTemplate::new(403).set_body_string(r#"{"error":{"code":"invalid_api_key","message":"Incorrect API key provided"}}"#)).await;
        assert_eq!(client(&server, None).transcribe(WAV, None).await.unwrap_err(), AsrError::Unauthorized);
    }

    #[tokio::test]
    async fn rate_limited_reads_retry_after_seconds() {
        let server = MockServer::start().await;
        mount(&server, ResponseTemplate::new(429).insert_header("Retry-After", "3")).await;
        let err = client(&server, None).transcribe(WAV, None).await.unwrap_err();
        assert_eq!(err, AsrError::RateLimited { retry_after_ms: Some(3000) });
        assert!(err.is_retryable());

        let server = MockServer::start().await;
        mount(&server, ResponseTemplate::new(429).insert_header("Retry-After", "Wed, 21 Oct 2015 07:28:00 GMT")).await;
        let err = client(&server, None).transcribe(WAV, None).await.unwrap_err();
        assert_eq!(err, AsrError::RateLimited { retry_after_ms: None });

        let server = MockServer::start().await;
        mount(&server, ResponseTemplate::new(429)).await;
        assert_eq!(client(&server, None).transcribe(WAV, None).await.unwrap_err(), AsrError::RateLimited { retry_after_ms: None });
    }

    #[tokio::test]
    async fn server_errors_keep_a_short_body() {
        let server = MockServer::start().await;
        let long = "x".repeat(1000);
        mount(&server, ResponseTemplate::new(500).set_body_string(long)).await;
        let err = client(&server, None).transcribe(WAV, None).await.unwrap_err();
        match &err {
            AsrError::Server { status, body } => {
                assert_eq!(*status, 500);
                assert_eq!(body.chars().count(), MAX_ERROR_BODY_CHARS);
            }
            other => panic!("{other:?}"),
        }
        assert!(err.is_retryable());

        let server = MockServer::start().await;
        mount(&server, ResponseTemplate::new(400).set_body_json(json!({ "error": "unsupported audio" }))).await;
        let err = client(&server, None).transcribe(WAV, None).await.unwrap_err();
        assert_eq!(err, AsrError::Server { status: 400, body: r#"{"error":"unsupported audio"}"#.into() });
        assert!(!err.is_retryable());
    }

    #[tokio::test]
    async fn malformed_bodies_are_bad_responses() {
        for body in ["not json at all", r#"{"result":"x"}"#, r#"{"text":42}"#, ""] {
            let server = MockServer::start().await;
            mount(&server, ResponseTemplate::new(200).set_body_string(body)).await;
            let err = client(&server, None).transcribe(WAV, None).await.unwrap_err();
            assert!(matches!(&err, AsrError::BadResponse(msg) if msg.contains(body)), "{body:?} -> {err:?}");
            assert!(!err.is_retryable());
        }
    }

    #[tokio::test]
    async fn slow_server_times_out() {
        let server = MockServer::start().await;
        mount(&server, ResponseTemplate::new(200).set_body_json(json!({ "text": "late" })).set_delay(Duration::from_secs(5))).await;
        let config = AsrConfig::new(server.uri(), "m").with_timeout(Duration::from_millis(200));
        let err = AsrClient::new(config).unwrap().transcribe(WAV, None).await.unwrap_err();
        assert_eq!(err, AsrError::Timeout);
        assert!(err.is_retryable());
    }

    #[tokio::test]
    async fn refused_connection_is_a_network_error() {
        // Bind then drop a listener so the port is known to be closed.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let client = AsrClient::new(AsrConfig::new(format!("http://127.0.0.1:{port}"), "m")).unwrap();
        let err = client.transcribe(WAV, None).await.unwrap_err();
        assert!(matches!(&err, AsrError::Network(msg) if !msg.is_empty()), "{err:?}");
        // Regression (public release, 2026-09-27): no URL in the message (it reaches the UI).
        assert!(!err.to_string().contains("127.0.0.1") && !err.to_string().contains(&port.to_string()), "{err}");
        assert!(err.is_retryable());
    }

    #[test]
    fn constructor_rejects_bad_config() {
        let bad_url = AsrClient::new(AsrConfig::new("host-without-scheme", "m")).map(drop).unwrap_err();
        assert!(matches!(bad_url, AsrError::InvalidConfig(_)));
        let empty_model = AsrClient::new(AsrConfig::new("https://host", "  ")).map(drop).unwrap_err();
        assert_eq!(empty_model, AsrError::InvalidConfig("model is empty".into()));
        let zero = AsrClient::new(AsrConfig::new("https://host", "m").with_timeout(Duration::ZERO)).map(drop).unwrap_err();
        assert_eq!(zero, AsrError::InvalidConfig("timeout must be greater than zero".into()));
    }

    #[test]
    fn client_debug_redacts_token() {
        let client = AsrClient::new(AsrConfig::new("https://host", "m").with_token(Some("sk-top-secret".into()))).unwrap();
        let debug = format!("{client:?}");
        assert!(!debug.contains("top-secret"), "{debug}");
        assert!(debug.contains("<redacted>"), "{debug}");
        assert!(debug.contains("https://host/v1/audio/transcriptions"), "{debug}");
        let cloned = client.clone();
        assert_eq!(cloned.endpoint(), client.endpoint());
    }

    #[test]
    fn helpers() {
        let mut headers = HeaderMap::new();
        assert_eq!(retry_after_ms(&headers), None);
        headers.insert(RETRY_AFTER, HeaderValue::from_static(" 12 "));
        assert_eq!(retry_after_ms(&headers), Some(12_000));
        headers.insert(RETRY_AFTER, HeaderValue::from_static("soon"));
        assert_eq!(retry_after_ms(&headers), None);
        headers.insert(RETRY_AFTER, HeaderValue::from_static("99999999999999999999"));
        assert_eq!(retry_after_ms(&headers), None);
        headers.insert(RETRY_AFTER, HeaderValue::from_static("18446744073709551"));
        assert_eq!(retry_after_ms(&headers), Some(18_446_744_073_709_551_000));
        headers.insert(RETRY_AFTER, HeaderValue::from_static("18446744073709552"));
        assert_eq!(retry_after_ms(&headers), None, "overflow");
        headers.insert(RETRY_AFTER, HeaderValue::from_bytes(b"\xff").unwrap());
        assert_eq!(retry_after_ms(&headers), None, "non-utf8");
        assert_eq!(truncate_chars("héllo", 2), "hé");
        assert_eq!(truncate_chars("ab", 5), "ab");
    }
}
