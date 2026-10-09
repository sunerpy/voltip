//! Model Studio's realtime recognition over its WebSocket task protocol (docs/dictation.md §3.4):
//! `wss://<host>/api-ws/v1/inference`, the key checked at the handshake, then
//!
//! 1. `run-task` (model, parameters) → `task-started`;
//! 2. binary 16-bit PCM frames, while `result-generated` events bring the sentence being spoken
//!    (`sentence_end: false`, revised as it grows) and every finished one (`sentence_end: true`);
//! 3. `finish-task` → the last sentences → `task-finished` (or `task-failed` at any point).
//!
//! [`connect`] opens a task and splits it into a [`DuplexSender`] and a [`DuplexReceiver`], so a
//! live take can feed audio and read results at the same time (`voltip-cloud`'s streaming
//! session); [`transcribe_whole`] sends a recorded take at once and joins its sentences. The
//! realtime models may take as long as the audio plays to answer a take sent whole, so a take is
//! better streamed while it is spoken (§11.9).

use std::time::{Duration, Instant};

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Map, Value, json};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::{self, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::client::{Transcript, truncate_chars};
use crate::config::MAX_ERROR_BODY_CHARS;
use crate::dashscope::{DashscopeClient, join_sentences, service_error, wav_format};
use crate::error::AsrError;
use crate::error::is_quota_exhausted;

/// Deadline for the handshake and `task-started`.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Bytes of one audio frame: 100 ms of 16 kHz 16-bit mono.
pub const FRAME_BYTES: usize = 3200;

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// One realtime task's request: where, with which key, which model and its parameters
/// ([`DashscopeClient::duplex_options`]). `Debug` hides the key.
#[derive(Clone, PartialEq)]
pub struct DuplexOptions {
    /// `wss://<host>/api-ws/v1/inference`.
    pub url: String,
    /// Bearer key for the handshake.
    pub token: Option<String>,
    /// Model id.
    pub model: String,
    /// `payload.parameters` of `run-task`.
    pub parameters: Map<String, Value>,
}

impl std::fmt::Debug for DuplexOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DuplexOptions")
            .field("model", &self.model)
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .field("parameters", &self.parameters.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

/// What the task said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DuplexEvent {
    /// The sentence being spoken, whole so far (it may still change).
    Partial {
        /// Its text.
        text: String,
    },
    /// A finished sentence.
    Sentence {
        /// Its text, punctuation included.
        text: String,
        /// Start in milliseconds of the audio sent.
        begin_ms: u64,
        /// End in milliseconds of the audio sent.
        end_ms: u64,
    },
    /// `task-finished`: everything was said.
    Finished,
}

/// The sending half of a task: audio frames, then `finish-task`.
pub struct DuplexSender {
    sink: SplitSink<Socket, Message>,
    task_id: String,
}

impl DuplexSender {
    /// Send 16-bit PCM (any length; the service takes frames of any size).
    pub async fn audio(&mut self, pcm: Vec<u8>) -> Result<(), AsrError> {
        self.sink.send(Message::binary(pcm)).await.map_err(map_ws)
    }

    /// No more audio: the service finishes the last sentence and ends the task.
    pub async fn finish(&mut self) -> Result<(), AsrError> {
        let finish = json!({ "header": { "action": "finish-task", "task_id": self.task_id, "streaming": "duplex" }, "payload": { "input": {} } });
        self.sink.send(Message::text(finish.to_string())).await.map_err(map_ws)
    }

    /// Close the connection (after `task-finished`, or to abandon the task).
    pub async fn close(mut self) {
        let _ = self.sink.close().await;
    }
}

/// The receiving half of a task.
pub struct DuplexReceiver {
    stream: SplitStream<Socket>,
}

impl DuplexReceiver {
    /// The next thing the task says. Heartbeats and empty sentence starts are skipped; a failed
    /// task, or a connection that ends before `task-finished`, is an error.
    pub async fn next(&mut self) -> Result<DuplexEvent, AsrError> {
        loop {
            let message = match self.stream.next().await {
                Some(Ok(message)) => message,
                Some(Err(e)) => return Err(map_ws(e)),
                None => return Err(closed_early()),
            };
            let text = match message {
                Message::Text(text) => text,
                Message::Close(_) => return Err(closed_early()),
                // Pings are answered by tungstenite itself; nothing else comes as binary.
                _ => continue,
            };
            if let Some(event) = parse_event(text.as_str())? {
                return Ok(event);
            }
        }
    }
}

fn closed_early() -> AsrError {
    AsrError::Network("the service closed the connection before the task finished".into())
}

/// Open a task: the handshake with the key, `run-task`, and its `task-started`, within
/// [`CONNECT_TIMEOUT`].
pub async fn connect(options: &DuplexOptions) -> Result<(DuplexSender, DuplexReceiver), AsrError> {
    tokio::time::timeout(CONNECT_TIMEOUT, open(options)).await.map_err(|_| AsrError::Timeout)?
}

async fn open(options: &DuplexOptions) -> Result<(DuplexSender, DuplexReceiver), AsrError> {
    let mut request = options.url.as_str().into_client_request().map_err(|e| AsrError::InvalidConfig(format!("realtime endpoint: {e}")))?;
    let headers = request.headers_mut();
    headers.insert("user-agent", HeaderValue::from_static(concat!("voltip-asr/", env!("CARGO_PKG_VERSION"))));
    if let Some(token) = &options.token {
        let value = HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| AsrError::InvalidConfig("the key is not a valid header value".into()))?;
        headers.insert("authorization", value);
    }
    let (socket, _) = tokio_tungstenite::connect_async(request).await.map_err(map_ws)?;
    let (mut sink, mut stream) = socket.split();
    let task_id = uuid::Uuid::new_v4().to_string();
    let run = json!({
        "header": { "action": "run-task", "task_id": task_id, "streaming": "duplex" },
        "payload": { "task_group": "audio", "task": "asr", "function": "recognition", "model": options.model, "parameters": options.parameters, "input": {} },
    });
    sink.send(Message::text(run.to_string())).await.map_err(map_ws)?;
    loop {
        let text = match stream.next().await {
            Some(Ok(Message::Text(text))) => text,
            Some(Ok(Message::Close(_))) | None => return Err(closed_early()),
            Some(Ok(_)) => continue,
            Some(Err(e)) => return Err(map_ws(e)),
        };
        let value: Value = serde_json::from_str(text.as_str()).map_err(|e| bad_event(&e, text.as_str()))?;
        match value.pointer("/header/event").and_then(Value::as_str) {
            Some("task-started") => break,
            Some("task-failed") => return Err(task_failed(&value)),
            _ => {}
        }
    }
    tracing::debug!(model = %options.model, "realtime task started");
    Ok((DuplexSender { sink, task_id }, DuplexReceiver { stream }))
}

/// One text frame as an event; `None` for what is not news (task-started repeated, heartbeats, an
/// empty sentence start, unknown events).
fn parse_event(text: &str) -> Result<Option<DuplexEvent>, AsrError> {
    let value: Value = serde_json::from_str(text).map_err(|e| bad_event(&e, text))?;
    match value.pointer("/header/event").and_then(Value::as_str) {
        Some("result-generated") => {
            let Some(sentence) = value.pointer("/payload/output/sentence") else { return Ok(None) };
            if sentence.get("heartbeat").and_then(Value::as_bool) == Some(true) {
                return Ok(None);
            }
            let text = sentence.get("text").and_then(Value::as_str).unwrap_or_default().to_owned();
            let time = |key: &str| sentence.get(key).and_then(Value::as_u64);
            if sentence.get("sentence_end").and_then(Value::as_bool) == Some(true) {
                let begin_ms = time("begin_time").unwrap_or(0);
                return Ok(Some(DuplexEvent::Sentence { text, begin_ms, end_ms: time("end_time").unwrap_or(begin_ms) }));
            }
            Ok((!text.is_empty()).then_some(DuplexEvent::Partial { text }))
        }
        Some("task-finished") => Ok(Some(DuplexEvent::Finished)),
        Some("task-failed") => Err(task_failed(&value)),
        _ => Ok(None),
    }
}

fn bad_event(e: &serde_json::Error, text: &str) -> AsrError {
    AsrError::BadResponse(format!("realtime event: {e}: {}", truncate_chars(text, MAX_ERROR_BODY_CHARS)))
}

/// `task-failed` as an error: the free tier's stop, throttling and the key are named.
fn task_failed(value: &Value) -> AsrError {
    let field = |key: &str| value.pointer(&format!("/header/{key}")).and_then(Value::as_str).unwrap_or_default().to_owned();
    let (code, message) = (field("error_code"), field("error_message"));
    if is_quota_exhausted(&code, &message) {
        AsrError::QuotaExhausted { code, message: truncate_chars(&message, MAX_ERROR_BODY_CHARS) }
    } else if code.starts_with("Throttling") {
        AsrError::RateLimited { retry_after_ms: None }
    } else if code == "InvalidApiKey" {
        AsrError::Unauthorized
    } else {
        AsrError::Service { code, message: truncate_chars(&message, MAX_ERROR_BODY_CHARS) }
    }
}

/// A WebSocket failure, host-free: the handshake's HTTP refusal goes through the same sorting as
/// the HTTP models' answers ([`service_error`]).
fn map_ws(error: tungstenite::Error) -> AsrError {
    use tungstenite::Error as E;
    match error {
        E::Http(response) => {
            let body = response.body().as_deref().map(String::from_utf8_lossy).unwrap_or_default().into_owned();
            service_error(response.status(), None, &body)
        }
        E::Url(e) => AsrError::InvalidConfig(format!("realtime endpoint: {e}")),
        E::ConnectionClosed | E::AlreadyClosed => closed_early(),
        E::Io(e) => AsrError::Network(e.to_string()),
        E::Tls(e) => AsrError::Network(format!("TLS: {e}")),
        other => AsrError::Network(other.to_string()),
    }
}

/// Recognise a recorded take on a realtime model: its PCM sent at once, then `finish-task`, the
/// finished sentences joined (docs/dictation.md §3.4). The deadline is the client's timeout plus
/// the take's length, since the service may answer at the pace the audio plays.
pub(crate) async fn transcribe_whole(client: &DashscopeClient, wav: &[u8], language: Option<&str>, glossary: &[String]) -> Result<Transcript, AsrError> {
    let started = Instant::now();
    let format = wav_format(wav)?;
    let pcm = &wav[format.data.0..format.data.1];
    let played = Duration::from_millis(u64::try_from(pcm.len()).unwrap_or(u64::MAX) / 2 * 1000 / u64::from(format.sample_rate));
    let options = client.duplex_options(language, glossary, format.sample_rate, false);
    let deadline = client.config().timeout.saturating_add(played);
    let run = async {
        let (mut sender, mut receiver) = connect(&options).await?;
        let send = async {
            for frame in pcm.chunks(FRAME_BYTES) {
                sender.audio(frame.to_vec()).await?;
            }
            sender.finish().await
        };
        let receive = async {
            let mut sentences = Vec::new();
            loop {
                match receiver.next().await? {
                    DuplexEvent::Sentence { text, .. } => sentences.push(text),
                    DuplexEvent::Partial { .. } => {}
                    DuplexEvent::Finished => return Ok::<_, AsrError>(sentences),
                }
            }
        };
        let (sent, received) = tokio::join!(send, receive);
        // What the service said explains a failure better than the send that it cut off.
        let sentences = received?;
        sent?;
        sender.close().await;
        Ok::<_, AsrError>(sentences)
    };
    let sentences = tokio::time::timeout(deadline, run).await.map_err(|_| AsrError::Timeout)??;
    let text = join_sentences(&sentences);
    let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    tracing::debug!(latency_ms, sentences = sentences.len(), chars = text.chars().count(), "realtime model transcribed a whole take");
    Ok(Transcript { text, latency_ms, model: client.config().model.clone() })
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Arc;

    use std::sync::Mutex;
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
    use tokio_tungstenite::tungstenite::http::StatusCode;

    use super::*;
    use crate::config::AsrConfig;
    use crate::dashscope::DashscopeMode;

    /// What a scripted task does.
    #[derive(Clone)]
    pub(crate) enum Script {
        /// Start, answer every 3200 bytes with a growing partial, finish with `sentences`.
        Answer { partials: bool, sentences: Vec<&'static str> },
        /// Refuse the handshake with this status and body.
        Refuse(u16, &'static str),
        /// Fail the task with this code at `run-task`.
        FailAtStart(&'static str, &'static str),
        /// Start, then close the connection after the first audio frame.
        Drop,
        /// Start, then say nothing.
        Silent,
    }

    /// What the scripted server saw.
    #[derive(Default)]
    pub(crate) struct Seen {
        pub authorization: Option<String>,
        pub run_task: Option<Value>,
        pub audio_bytes: usize,
        pub finished: bool,
    }

    /// A local realtime endpoint following `script`; returns its `ws://` URL and what it saw.
    pub(crate) async fn serve(script: Script) -> (String, Arc<Mutex<Seen>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/api-ws/v1/inference", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Seen::default()));
        let shared = seen.clone();
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                let (script, seen) = (script.clone(), shared.clone());
                tokio::spawn(async move { session(tcp, script, seen).await });
            }
        });
        (url, seen)
    }

    // tungstenite's handshake callback returns its `ErrorResponse` by value.
    #[allow(clippy::result_large_err)]
    async fn session(tcp: TcpStream, script: Script, seen: Arc<Mutex<Seen>>) {
        let refuse = match &script {
            Script::Refuse(status, body) => Some((*status, *body)),
            _ => None,
        };
        let check = seen.clone();
        let callback = move |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
            check.lock().unwrap().authorization = request.headers().get("authorization").and_then(|v| v.to_str().ok()).map(str::to_owned);
            match refuse {
                Some((status, body)) => {
                    let mut refusal = ErrorResponse::new(Some(body.to_owned()));
                    *refusal.status_mut() = StatusCode::from_u16(status).unwrap();
                    Err(refusal)
                }
                None => Ok(response),
            }
        };
        let Ok(socket) = tokio_tungstenite::accept_hdr_async(tcp, callback).await else { return };
        let (mut sink, mut stream) = socket.split();
        let event = |name: &str, payload: Value| {
            Message::text(json!({ "header": { "task_id": "t", "event": name, "attributes": {} }, "payload": payload }).to_string())
        };
        let mut said = 0usize;
        while let Some(Ok(message)) = stream.next().await {
            match message {
                Message::Text(text) => {
                    let value: Value = serde_json::from_str(text.as_str()).unwrap();
                    match value.pointer("/header/action").and_then(Value::as_str) {
                        Some("run-task") => {
                            seen.lock().unwrap().run_task = Some(value.clone());
                            if let Script::FailAtStart(code, message) = &script {
                                let failed = json!({ "header": { "task_id": "t", "event": "task-failed", "error_code": code, "error_message": message }, "payload": {} });
                                let _ = sink.send(Message::text(failed.to_string())).await;
                                return;
                            }
                            let _ = sink.send(event("task-started", json!({}))).await;
                            // A heartbeat and an empty sentence start are not news.
                            let _ = sink
                                .send(event("result-generated", json!({ "output": { "sentence": { "sentence_id": 0, "heartbeat": true, "text": "" } } })))
                                .await;
                            let _ = sink
                                .send(event(
                                    "result-generated",
                                    json!({ "output": { "sentence": { "sentence_id": 1, "sentence_begin": true, "sentence_end": false, "text": "" } } }),
                                ))
                                .await;
                        }
                        Some("finish-task") => {
                            seen.lock().unwrap().finished = true;
                            if let Script::Answer { sentences, .. } = &script {
                                let mut at = 0;
                                for (i, s) in sentences.iter().enumerate() {
                                    let sentence = json!({ "sentence_id": i + 1, "begin_time": at, "end_time": at + 1000, "text": s, "sentence_end": true });
                                    let _ = sink.send(event("result-generated", json!({ "output": { "sentence": sentence } }))).await;
                                    at += 1000;
                                }
                                let _ = sink.send(event("task-finished", json!({ "output": {}, "usage": { "duration": 1 } }))).await;
                            }
                        }
                        _ => {}
                    }
                }
                Message::Binary(bytes) => {
                    let total = {
                        let mut seen = seen.lock().unwrap();
                        seen.audio_bytes += bytes.len();
                        seen.audio_bytes
                    };
                    match &script {
                        Script::Drop => return,
                        Script::Answer { partials: true, .. } => {
                            while said < total / FRAME_BYTES {
                                said += 1;
                                let partial = json!({ "sentence_id": 1, "sentence_end": false, "text": "字".repeat(said) });
                                let _ = sink.send(event("result-generated", json!({ "output": { "sentence": partial } }))).await;
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    pub(crate) fn options(url: &str, token: Option<&str>) -> DuplexOptions {
        let mut parameters = Map::new();
        parameters.insert("format".into(), json!("pcm"));
        DuplexOptions { url: url.to_owned(), token: token.map(str::to_owned), model: "qwen-audio-3.1-asr-flash-streaming".into(), parameters }
    }

    fn client_at(url: &str, model: &str) -> DashscopeClient {
        // The client derives the WebSocket from the origin: give it the server's http origin.
        let origin = url.replace("ws://", "http://").replace("/api-ws/v1/inference", "");
        DashscopeClient::new(AsrConfig::new(format!("{origin}/compatible-mode/v1"), model).with_token(Some("sk-test".into())), DashscopeMode::Duplex).unwrap()
    }

    #[tokio::test]
    async fn a_task_streams_partials_and_sentences_and_finishes() {
        let (url, seen) = serve(Script::Answer { partials: true, sentences: vec!["你好，世界。", "今天天气不错。"] }).await;
        let (mut tx, mut rx) = connect(&options(&url, Some("sk-test"))).await.unwrap();
        {
            let seen = seen.lock().unwrap();
            assert_eq!(seen.authorization.as_deref(), Some("Bearer sk-test"));
            let run = seen.run_task.as_ref().unwrap();
            assert_eq!(run.pointer("/header/streaming"), Some(&json!("duplex")));
            assert_eq!(run.pointer("/payload/model"), Some(&json!("qwen-audio-3.1-asr-flash-streaming")));
            assert_eq!(run.pointer("/payload/function"), Some(&json!("recognition")));
            assert_eq!(run.pointer("/payload/parameters/format"), Some(&json!("pcm")));
        }
        tx.audio(vec![0; FRAME_BYTES]).await.unwrap();
        assert_eq!(rx.next().await.unwrap(), DuplexEvent::Partial { text: "字".into() });
        tx.audio(vec![0; FRAME_BYTES]).await.unwrap();
        assert_eq!(rx.next().await.unwrap(), DuplexEvent::Partial { text: "字字".into() });
        tx.finish().await.unwrap();
        assert_eq!(rx.next().await.unwrap(), DuplexEvent::Sentence { text: "你好，世界。".into(), begin_ms: 0, end_ms: 1000 });
        assert_eq!(rx.next().await.unwrap(), DuplexEvent::Sentence { text: "今天天气不错。".into(), begin_ms: 1000, end_ms: 2000 });
        assert_eq!(rx.next().await.unwrap(), DuplexEvent::Finished);
        tx.close().await;
        assert!(seen.lock().unwrap().finished);
        assert_eq!(seen.lock().unwrap().audio_bytes, 2 * FRAME_BYTES);
        assert!(!format!("{:?}", options(&url, Some("sk-secret-y"))).contains("secret"));
    }

    #[tokio::test]
    async fn a_whole_take_is_sent_at_once_and_its_sentences_joined() {
        let (url, seen) = serve(Script::Answer { partials: false, sentences: vec!["我想创建一个 Good Idea 吧。", "比如说。"] }).await;
        let client = client_at(&url, "qwen-audio-3.1-asr-flash-streaming");
        let wav = crate::dashscope::tests::wav(16_000, 16_000);
        let t = client.transcribe(&wav, Some("zh"), &["Good Idea".into()]).await.unwrap();
        assert_eq!(t.text, "我想创建一个 Good Idea 吧。比如说。");
        assert_eq!(t.model, "qwen-audio-3.1-asr-flash-streaming");
        let seen = seen.lock().unwrap();
        assert_eq!(seen.audio_bytes, 32_000, "the WAV's PCM, without its header");
        assert!(seen.finished);
        let parameters = seen.run_task.as_ref().and_then(|r| r.pointer("/payload/parameters")).cloned().unwrap();
        assert_eq!(
            parameters,
            json!({ "format": "pcm", "sample_rate": 16000, "language_hints": ["zh"], "vocabulary": { "Good Idea": 4 } }),
            "no live-only parameters"
        );
        assert_eq!(seen.authorization.as_deref(), Some("Bearer sk-test"));
    }

    /// The handshake's refusals and a task that fails are sorted like the HTTP answers: the key,
    /// the free tier's stop, the service's own code.
    #[tokio::test]
    async fn refusals_and_failures_are_sorted() {
        let (url, _) = serve(Script::Refuse(401, r#"{"code":"InvalidApiKey","message":"Invalid API-key provided."}"#)).await;
        assert_eq!(connect(&options(&url, Some("bad"))).await.err(), Some(AsrError::Unauthorized));
        let (url, _) = serve(Script::Refuse(403, r#"{"code":"AllocationQuota.FreeTierOnly","message":"free tier exhausted"}"#)).await;
        assert_eq!(
            connect(&options(&url, Some("k"))).await.err(),
            Some(AsrError::QuotaExhausted { code: "AllocationQuota.FreeTierOnly".into(), message: "free tier exhausted".into() })
        );
        let (url, _) = serve(Script::FailAtStart("ModelNotFound", "Model not found (qwen3-asr-flash-realtime)!")).await;
        assert_eq!(
            connect(&options(&url, Some("k"))).await.err(),
            Some(AsrError::Service { code: "ModelNotFound".into(), message: "Model not found (qwen3-asr-flash-realtime)!".into() })
        );
        let (url, _) = serve(Script::FailAtStart("Throttling.RateQuota", "slow down")).await;
        assert_eq!(connect(&options(&url, Some("k"))).await.err(), Some(AsrError::RateLimited { retry_after_ms: None }));
        let (url, _) = serve(Script::FailAtStart("AllocationQuota.FreeTierOnly", "x")).await;
        assert_eq!(
            connect(&options(&url, Some("k"))).await.err(),
            Some(AsrError::QuotaExhausted { code: "AllocationQuota.FreeTierOnly".into(), message: "x".into() })
        );
        let (url, _) = serve(Script::FailAtStart("InvalidApiKey", "no")).await;
        assert_eq!(connect(&options(&url, None)).await.err(), Some(AsrError::Unauthorized));
        // A whole take on a task that fails reports the task's reason.
        let (url, _) = serve(Script::FailAtStart("InvalidParameter", "sample_rate must be 16000")).await;
        let err = client_at(&url, "qwen-audio-3.1-asr-flash-message").transcribe(&crate::dashscope::tests::wav(1600, 8_000), None, &[]).await.unwrap_err();
        assert_eq!(err, AsrError::Service { code: "InvalidParameter".into(), message: "sample_rate must be 16000".into() });
        // A connection that drops mid-take.
        let (url, _) = serve(Script::Drop).await;
        let err = client_at(&url, "fun-asr-realtime").transcribe(&crate::dashscope::tests::wav(16_000, 16_000), None, &[]).await.unwrap_err();
        assert!(matches!(&err, AsrError::Network(_)), "{err:?}");
        // Nobody listening.
        let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = closed.local_addr().unwrap().port();
        drop(closed);
        let err = connect(&options(&format!("ws://127.0.0.1:{port}/api-ws/v1/inference"), None)).await.err().unwrap();
        assert!(matches!(&err, AsrError::Network(m) if !m.contains("127.0.0.1")), "{err:?}");
        assert!(matches!(connect(&options("not a url", None)).await.err(), Some(AsrError::InvalidConfig(_))));
        assert!(matches!(connect(&options("ws://127.0.0.1:1/x", Some("bad\nkey"))).await.err(), Some(AsrError::InvalidConfig(_))));
    }

    /// Real time, not a paused clock: the server is a real socket, and a paused clock would move
    /// on while the handshake waits for it.
    #[tokio::test]
    async fn a_silent_service_times_out() {
        let (url, _) = serve(Script::Silent).await;
        let client = DashscopeClient::new(
            AsrConfig::new(url.replace("ws://", "http://").replace("/api-ws/v1/inference", ""), "qwen-audio-3.1-asr-flash-streaming")
                .with_timeout(Duration::from_millis(500)),
            DashscopeMode::Duplex,
        )
        .unwrap();
        let err = client.transcribe(&crate::dashscope::tests::wav(1600, 16_000), None, &[]).await.unwrap_err();
        assert_eq!(err, AsrError::Timeout);
    }

    #[test]
    fn events_parse_or_are_skipped() {
        let event = |name: &str, sentence: Value| json!({ "header": { "event": name }, "payload": { "output": { "sentence": sentence } } }).to_string();
        assert_eq!(
            parse_event(&event("result-generated", json!({ "text": "你", "sentence_end": false }))).unwrap(),
            Some(DuplexEvent::Partial { text: "你".into() })
        );
        assert_eq!(
            parse_event(&event("result-generated", json!({ "text": "你好。", "sentence_end": true, "begin_time": 120, "end_time": null }))).unwrap(),
            Some(DuplexEvent::Sentence { text: "你好。".into(), begin_ms: 120, end_ms: 120 })
        );
        assert_eq!(parse_event(&event("result-generated", json!({ "text": "", "heartbeat": true }))).unwrap(), None);
        assert_eq!(parse_event(&event("result-generated", json!({ "text": "" }))).unwrap(), None);
        assert_eq!(parse_event(r#"{"header":{"event":"result-generated"},"payload":{}}"#).unwrap(), None);
        assert_eq!(parse_event(r#"{"header":{"event":"task-started"}}"#).unwrap(), None);
        assert_eq!(parse_event(r#"{"header":{"event":"task-finished"}}"#).unwrap(), Some(DuplexEvent::Finished));
        assert!(matches!(parse_event("not json"), Err(AsrError::BadResponse(_))));
        assert!(matches!(
            parse_event(r#"{"header":{"event":"task-failed","error_code":"CLIENT_ERROR","error_message":"request timeout after 23 seconds."}}"#),
            Err(AsrError::Service { code, .. }) if code == "CLIENT_ERROR"
        ));
    }
}
