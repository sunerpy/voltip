//! StepFun (docs/dictation.md §3.15). `stepaudio-2.5-asr` takes whole files at
//! `POST {base}/audio/transcriptions` (multipart; `response_format` is required, the glossary goes as
//! `hotwords`, a JSON array in a string). The `…-stream` models recognise while the take is spoken
//! over `wss://…/realtime/asr/stream`:
//!
//! 1. the service says `session.created`; the client sends `session.update` (16 kHz PCM, the model,
//!    the language, the glossary as the `prompt`, the service's own pause detection), and the audio
//!    waits until the service confirms it with `session.updated`;
//! 2. `input_audio_buffer.append` frames of base64 PCM; the pause detection cuts the take into
//!    items, and the service answers each with `…transcription.delta` (the item's text so far: its
//!    settled part `text`, then the tail it may still correct, `stash`) and, once the item is
//!    committed, `…transcription.completed`;
//! 3. at the end of the take the client sends a second of silence, so the pause detection closes the
//!    sentence being spoken, then commits what is left, and waits for the items still open.
//!
//! A recorded take for a `…-stream` model goes to `stepaudio-2.5-asr` with the same key, which
//! transcribes it in a second instead of as long as it plays; its receipt names that model.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use reqwest::multipart::{Form, Part};
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use url::Url;

use crate::client::{Transcript, client_builder, http_error, map_reqwest, retry_after_ms, send, truncate_chars};
use crate::config::{AsrConfig, MAX_ERROR_BODY_CHARS};
use crate::dashscope::{join_sentences, wav_format};
use crate::error::AsrError;
use crate::realtime::{self, BYTES_PER_MS, SAMPLE_RATE};
use crate::vendor::{RealtimeEvent, RealtimeEvents, RealtimeInput, VendorAsr, endpoint};

/// The whole-file model a `…-stream` model's recorded takes go to.
pub const WHOLE_FILE_MODEL: &str = "stepaudio-2.5-asr";
/// Most hot words a whole-file request carries.
pub const MAX_HOTWORDS: usize = 100;
/// Most characters of the realtime session's `prompt`.
pub const MAX_PROMPT_CHARS: usize = 1_000;
/// The longest piece one whole-file request carries: 45 minutes of WAV is 86 MB, within the
/// service's 100 MB.
pub const MAX_PIECE_MS: u64 = 45 * 60_000;
/// Most audio one `append` carries (one second).
const MAX_APPEND_BYTES: usize = 1_000 * BYTES_PER_MS;
/// The pause after which the service's detection closes a sentence.
const PAUSE_MS: u64 = 800;
/// The silence sent at the end of a take: longer than [`PAUSE_MS`], so the detection closes the
/// sentence being spoken by itself.
const CLOSING_SILENCE_MS: usize = 1_000;

/// How long the end of a take may wait.
#[derive(Clone, Copy, Debug)]
struct Ending {
    /// With every item closed, how long the service may stay quiet before the session counts as
    /// finished (a service that ignored the final commit).
    settle: Duration,
    /// The longest the end may take; past it the session fails, and the take is recognised whole
    /// (within the core's 30 seconds).
    deadline: Duration,
}

const ENDING: Ending = Ending { settle: Duration::from_secs(2), deadline: Duration::from_secs(15) };

/// The StepFun client, for a whole-file or a `…-stream` model.
pub(crate) struct StepfunAsr {
    config: AsrConfig,
    key: String,
    transcriptions: Url,
    stream: Url,
    models: Url,
    http: Client,
    /// How long a live session may take to open, from the connection to `session.updated`.
    opening: Duration,
    ending: Ending,
}

impl StepfunAsr {
    /// The client of `config`: the stored base (`…/v1`, either site), the model, the API key.
    pub(crate) fn new(config: AsrConfig) -> Result<Self, AsrError> {
        let key = config
            .token
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .ok_or_else(|| AsrError::InvalidConfig("the credential is missing".into()))?
            .to_owned();
        let transcriptions = endpoint(&config.base_url, false, "/audio/transcriptions")?;
        let stream = endpoint(&config.base_url, true, "/realtime/asr/stream")?;
        let models = endpoint(&config.base_url, false, "/models")?;
        let http = config
            .network
            .apply(client_builder())
            .timeout(config.timeout)
            .user_agent(concat!("voltip-asr/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| AsrError::InvalidConfig(format!("http client: {e}")))?;
        Ok(Self { config, key, transcriptions, stream, models, http, opening: realtime::CONNECT_TIMEOUT, ending: ENDING })
    }

    fn realtime_model(&self) -> bool {
        self.model().to_ascii_lowercase().ends_with("-stream")
    }

    /// The model a whole file goes to.
    fn whole_file_model(&self) -> &str {
        if self.realtime_model() { WHOLE_FILE_MODEL } else { self.model() }
    }

    /// One whole-file request.
    async fn file(&self, wav: Vec<u8>, model: &str, words: &[String]) -> Result<String, AsrError> {
        let request = || {
            let file = Part::bytes(wav.clone()).file_name("audio.wav").mime_str("audio/wav").map_err(|e| AsrError::InvalidConfig(e.to_string()))?;
            let mut form = Form::new().part("file", file).text("model", model.to_owned()).text("response_format", "json");
            if !words.is_empty() {
                form = form.text("hotwords", Value::from(words.to_vec()).to_string());
            }
            Ok(self.http.post(self.transcriptions.as_str()).bearer_auth(&self.key).multipart(form))
        };
        let response = send(request).await?;
        let status = response.status();
        let retry = retry_after_ms(response.headers());
        let body = response.text().await.map_err(map_reqwest)?;
        if !status.is_success() {
            return Err(stepfun_error(status, retry, &body));
        }
        let answer: Value = serde_json::from_str(&body).map_err(|e| AsrError::BadResponse(format!("{e}: {}", truncate_chars(&body, MAX_ERROR_BODY_CHARS))))?;
        let text = answer
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| AsrError::BadResponse(format!("no text: {}", truncate_chars(&body, MAX_ERROR_BODY_CHARS))))?;
        Ok(text.trim().to_owned())
    }

    /// The `session.update` of a live session.
    fn session_update(&self, language: Option<&str>, glossary: &[String]) -> Value {
        let mut transcription = json!({ "model": self.model(), "full_rerun_on_commit": true, "enable_itn": true });
        if let Some(language) = language_hint(language) {
            transcription["language"] = json!(language);
        }
        if let Some(prompt) = prompt(glossary) {
            transcription["prompt"] = json!(prompt);
        }
        json!({
            "event_id": event_id(),
            "type": "session.update",
            "session": { "audio": { "input": {
                "format": { "type": "pcm", "codec": "pcm_s16le", "rate": SAMPLE_RATE, "bits": 16, "channel": 1 },
                "transcription": transcription,
                "turn_detection": { "type": "server_vad", "silence_duration_ms": PAUSE_MS, "threshold": 0.6 },
            } } },
        })
    }
}

#[async_trait]
impl VendorAsr for StepfunAsr {
    fn model(&self) -> &str {
        self.config.model.trim()
    }

    /// The whole-file model takes hot words, the realtime one a prompt; either carries the terms.
    fn sends_glossary(&self, glossary: &[String]) -> bool {
        if self.realtime_model() { prompt(glossary).is_some() } else { !hotwords(glossary).is_empty() }
    }

    async fn transcribe(&self, wav: &[u8], _language: Option<&str>, glossary: &[String]) -> Result<Transcript, AsrError> {
        let started = Instant::now();
        let model = self.whole_file_model().to_owned();
        let words = hotwords(glossary);
        let pieces = pieces(wav, MAX_PIECE_MS);
        let mut texts = Vec::with_capacity(pieces.len());
        for piece in pieces {
            texts.push(self.file(piece, &model, &words).await?);
        }
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        Ok(Transcript { text: join_sentences(&texts), latency_ms, model })
    }

    /// The key against the model list, as for the vendor's other models.
    async fn probe(&self) -> Result<(), AsrError> {
        let response = send(|| Ok(self.http.get(self.models.as_str()).bearer_auth(&self.key).timeout(Duration::from_secs(10)))).await?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        let retry = retry_after_ms(response.headers());
        Err(stepfun_error(status, retry, &response.text().await.unwrap_or_default()))
    }

    fn streams(&self) -> bool {
        self.realtime_model()
    }

    async fn realtime(
        &self,
        language: Option<&str>,
        glossary: &[String],
        mut input: mpsc::UnboundedReceiver<RealtimeInput>,
        events: RealtimeEvents,
    ) -> Result<(), AsrError> {
        let mut request = realtime::request(&self.stream)?;
        let bearer =
            HeaderValue::from_str(&format!("Bearer {}", self.key)).map_err(|_| AsrError::InvalidConfig("the credential is not a valid header value".into()))?;
        request.headers_mut().insert("authorization", bearer);
        let (socket, _) = realtime::connect(request, &self.config.network).await?;
        let (mut sink, mut stream) = socket.split();
        // The session opens with `session.created`; the configuration follows it, and the audio
        // waits in `input` until the service confirms the configuration (the doc's order).
        let opened_by = tokio::time::Instant::now() + self.opening;
        wait_for(&mut stream, "session.created", opened_by).await?;
        sink.send(Message::text(self.session_update(language, glossary).to_string())).await.map_err(realtime::map_ws)?;
        wait_for(&mut stream, "session.updated", opened_by).await?;

        let mut session = Session::default();
        // After the take's end: the final commit's event id, when the service last spoke, the deadline.
        let mut end: Option<(String, tokio::time::Instant, tokio::time::Instant)> = None;
        loop {
            let quiet = async {
                match &end {
                    Some((_, last, _)) if session.idle() => tokio::time::sleep_until(*last + self.ending.settle).await,
                    _ => std::future::pending().await,
                }
            };
            let late = async {
                match &end {
                    Some((_, _, deadline)) => tokio::time::sleep_until(*deadline).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                item = input.recv() => match item {
                    Some(RealtimeInput::Audio(pcm)) if end.is_none() => {
                        for chunk in pcm.chunks(MAX_APPEND_BYTES) {
                            session.sent_ms += u64::try_from(chunk.len() / BYTES_PER_MS).unwrap_or(0);
                            sink.send(append(chunk)).await.map_err(realtime::map_ws)?;
                        }
                    }
                    Some(RealtimeInput::Finish) if end.is_none() => {
                        sink.send(append(&[0u8; CLOSING_SILENCE_MS * BYTES_PER_MS])).await.map_err(realtime::map_ws)?;
                        let commit = event_id();
                        sink.send(Message::text(json!({ "event_id": commit, "type": "input_audio_buffer.commit" }).to_string())).await.map_err(realtime::map_ws)?;
                        let now = tokio::time::Instant::now();
                        end = Some((commit, now, now + self.ending.deadline));
                    }
                    // Nothing more goes out after the end.
                    Some(_) => {}
                    // A cancelled take, or a caller that stopped waiting: close at once.
                    None => {
                        let _ = sink.close().await;
                        return Ok(());
                    }
                },
                event = next_event(&mut stream) => match event? {
                    Some(event) => {
                        if let Some((_, last, _)) = end.as_mut() {
                            *last = tokio::time::Instant::now();
                        }
                        if let Some(out) = session.on(&event, end.as_ref().map(|(commit, _, _)| commit.as_str()))? {
                            let _ = events.send(out);
                        }
                        if end.is_some() && session.done() {
                            break;
                        }
                    }
                    None if end.is_some() && session.idle() => break,
                    None => return Err(realtime::closed_early()),
                },
                () = quiet => break,
                () = late => {
                    let _ = sink.close().await;
                    return Err(AsrError::Timeout);
                }
            }
        }
        let _ = sink.close().await;
        let _ = events.send(RealtimeEvent::Finished);
        Ok(())
    }
}

/// A take within one request's size as it is; a longer one in pieces of at most `max_ms` cut in
/// its pauses.
fn pieces(wav: &[u8], max_ms: u64) -> Vec<Vec<u8>> {
    match wav_format(wav) {
        Ok(format) if format.sample_rate == SAMPLE_RATE && ((format.data.1 - format.data.0) / BYTES_PER_MS) as u64 > max_ms => {
            let pcm = &wav[format.data.0..format.data.1];
            crate::split::quiet_cuts(pcm, max_ms).into_iter().map(|range| crate::split::wav(&pcm[range], SAMPLE_RATE)).collect()
        }
        _ => vec![wav.to_vec()],
    }
}

/// An `input_audio_buffer.append` of `pcm`.
fn append(pcm: &[u8]) -> Message {
    Message::text(
        json!({ "event_id": event_id(), "type": "input_audio_buffer.append", "audio": base64::engine::general_purpose::STANDARD.encode(pcm) }).to_string(),
    )
}

/// What a live session knows about its items: where each started and stopped in the audio, which
/// are committed and not yet recognised, and the audio sent.
#[derive(Default)]
struct Session {
    sent_ms: u64,
    started: HashMap<String, u64>,
    stopped: HashMap<String, u64>,
    /// The item whose speech started and has not stopped (the sentence being spoken).
    speaking: Option<String>,
    /// Items the service committed (or created) and has not completed.
    open: Vec<String>,
    /// The final commit was answered: by an item of its own, or by its refusal (an empty buffer).
    commit_answered: bool,
    last_end_ms: u64,
}

impl Session {
    /// Nothing is being spoken or recognised.
    fn idle(&self) -> bool {
        self.open.is_empty() && self.speaking.is_none()
    }

    /// Everything the end of the take committed is recognised.
    fn done(&self) -> bool {
        self.commit_answered && self.open.is_empty()
    }

    /// The final commit was answered: what was being spoken is committed, or there was nothing.
    fn answered(&mut self) {
        self.commit_answered = true;
        self.speaking = None;
    }

    fn opened(&mut self, item: &str) {
        if !item.is_empty() && !self.open.iter().any(|i| i == item) {
            self.open.push(item.to_owned());
        }
    }

    /// One event of the service, as the take's event when it is one. `commit` is the final
    /// commit's event id once the take ended.
    fn on(&mut self, event: &Value, commit: Option<&str>) -> Result<Option<RealtimeEvent>, AsrError> {
        let item = event.get("item_id").and_then(Value::as_str).unwrap_or_default().to_owned();
        let ms = |key: &str| event.get(key).and_then(Value::as_u64);
        // The pause detection commits an item it heard stop; an item committed without a stop is the
        // final commit's.
        let commits_own = commit.is_some() && !self.stopped.contains_key(&item);
        match event_type(event) {
            "input_audio_buffer.speech_started" => {
                self.started.insert(item.clone(), ms("audio_start_ms").unwrap_or(self.last_end_ms));
                self.speaking = Some(item);
            }
            "input_audio_buffer.speech_stopped" => {
                let at = ms("audio_end_ms").or_else(|| ms("audio_start_ms")).unwrap_or(self.sent_ms);
                self.stopped.insert(item, at);
                self.speaking = None;
            }
            "input_audio_buffer.committed" => {
                self.opened(&item);
                if commits_own {
                    self.answered();
                } else if self.speaking.as_deref() == Some(item.as_str()) {
                    self.speaking = None;
                }
            }
            "conversation.item.created" => {
                let id = event.pointer("/item/id").and_then(Value::as_str).unwrap_or(&item).to_owned();
                self.opened(&id);
            }
            // The item's text so far is its settled part and the tail the service may still
            // correct, which is all there is when an item starts.
            "conversation.item.input_audio_transcription.delta" => {
                let part = |key: &str| event.get(key).and_then(Value::as_str).unwrap_or_default();
                let text = format!("{}{}", part("text"), part("stash")).trim().to_owned();
                if !text.is_empty() {
                    return Ok(Some(RealtimeEvent::Partial { text }));
                }
            }
            "conversation.item.input_audio_transcription.completed" => {
                self.open.retain(|i| *i != item);
                let text = event.get("transcript").and_then(Value::as_str).unwrap_or_default().trim().to_owned();
                let (begin_ms, end_ms) = self.times(&item, event);
                self.last_end_ms = end_ms;
                if !text.is_empty() {
                    return Ok(Some(RealtimeEvent::Sentence { text, begin_ms, end_ms }));
                }
            }
            // A spoken item that could not be recognised fails the session (the take is then
            // recognised whole); the final commit's own, of the closing silence, does not.
            "conversation.item.input_audio_transcription.failed" => {
                self.open.retain(|i| *i != item);
                if !commits_own {
                    return Err(service_event_error(event));
                }
                self.answered();
            }
            "error" => {
                let field = |key: &str| event.pointer(&format!("/error/{key}")).and_then(Value::as_str);
                // The final commit refused (an empty or too short buffer): nothing was left.
                let refused_commit = commit.is_some_and(|id| field("event_id") == Some(id) || field("type") == Some("invalid_request_error"));
                if !refused_commit {
                    return Err(service_event_error(event));
                }
                tracing::debug!(error = %service_event_error(event), "StepFun refused the final commit");
                self.answered();
            }
            _ => {}
        }
        Ok(None)
    }

    /// An item's place in the audio: where its speech started, and as long as its words last (or
    /// until its speech stopped, or until the end of the audio sent).
    fn times(&self, item: &str, event: &Value) -> (u64, u64) {
        let begin = self.started.get(item).copied().unwrap_or(self.last_end_ms);
        let words = event.get("words").and_then(Value::as_array);
        let seconds = |word: Option<&Value>, key: &str| word.and_then(|w| w.get(key)).and_then(Value::as_f64);
        let spoken = match (seconds(words.and_then(|w| w.first()), "start"), seconds(words.and_then(|w| w.last()), "end")) {
            (Some(start), Some(end)) if end >= start => Some(((end - start) * 1000.0).round() as u64),
            _ => None,
        };
        let end = spoken.map(|d| begin + d).or_else(|| self.stopped.get(item).copied()).unwrap_or(self.sent_ms).max(begin);
        (begin, end)
    }
}

/// Wait until the service sends an event of `kind`, by `deadline`; its `error` event, a closed
/// connection or the deadline fails the session.
async fn wait_for<S>(stream: &mut S, kind: &str, deadline: tokio::time::Instant) -> Result<(), AsrError>
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    tokio::time::timeout_at(deadline, async {
        loop {
            match next_event(stream).await? {
                Some(event) if event_type(&event) == kind => return Ok(()),
                Some(event) if event_type(&event) == "error" => return Err(service_event_error(&event)),
                Some(_) => {}
                None => return Err(realtime::closed_early()),
            }
        }
    })
    .await
    .map_err(|_| AsrError::Timeout)?
}

/// The next service event, `None` when the connection closed.
async fn next_event<S>(stream: &mut S) -> Result<Option<Value>, AsrError>
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        match stream.next().await {
            Some(Ok(Message::Text(text))) => {
                return serde_json::from_str(text.as_str())
                    .map(Some)
                    .map_err(|e| AsrError::BadResponse(format!("realtime event: {e}: {}", truncate_chars(text.as_str(), MAX_ERROR_BODY_CHARS))));
            }
            Some(Ok(Message::Close(_))) | None => return Ok(None),
            Some(Ok(_)) => {}
            Some(Err(e)) => return Err(realtime::map_ws(e)),
        }
    }
}

fn event_type(event: &Value) -> &str {
    event.get("type").and_then(Value::as_str).unwrap_or_default()
}

/// An `error` event: StepFun's internal errors are the server's, its idle and heartbeat timeouts a
/// dropped connection, the rest a refused request (docs/dictation.md §3.15).
fn service_event_error(event: &Value) -> AsrError {
    let field = |key: &str| event.pointer(&format!("/error/{key}")).and_then(Value::as_str).unwrap_or_default().to_owned();
    let (code, message, kind) = (field("code"), field("message"), field("type"));
    match (kind.as_str(), code.as_str()) {
        ("internal_error", _) | (_, "internal_error") => AsrError::Server { status: 500, body: voltip_reply::clean(&message) },
        (_, "max_idle_timeout" | "pong_timeout") => AsrError::Network(voltip_reply::clean(&format!("{code}: {message}"))),
        _ => AsrError::Service { code, message: voltip_reply::clean(&message) },
    }
}

/// A refusal of the HTTP API: an account without balance answers 402, a used-up quota; the rest as
/// every OpenAI-compatible service's.
fn stepfun_error(status: StatusCode, retry_after_ms: Option<u64>, body: &str) -> AsrError {
    if status == StatusCode::PAYMENT_REQUIRED {
        let (code, message) = crate::error::error_fields(body);
        return AsrError::QuotaExhausted { code: if code.is_empty() { "402".into() } else { code }, message: voltip_reply::clean(&message), retry_at_ms: None };
    }
    http_error(status, retry_after_ms, body)
}

/// The glossary as `hotwords`: trimmed, no blanks, the first [`MAX_HOTWORDS`].
fn hotwords(glossary: &[String]) -> Vec<String> {
    glossary.iter().map(|t| t.trim()).filter(|t| !t.is_empty()).take(MAX_HOTWORDS).map(str::to_owned).collect()
}

/// The glossary as the realtime `prompt`: `专业术语：A、B`, whole terms within [`MAX_PROMPT_CHARS`].
fn prompt(glossary: &[String]) -> Option<String> {
    const HEAD: &str = "专业术语：";
    let mut out = String::from(HEAD);
    let mut chars = HEAD.chars().count();
    let mut any = false;
    for term in glossary.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
        let needed = term.chars().count() + usize::from(any);
        if chars + needed > MAX_PROMPT_CHARS {
            break;
        }
        if any {
            out.push('、');
        }
        out.push_str(term);
        chars += needed;
        any = true;
    }
    any.then_some(out)
}

/// The realtime session's `language`: `zh` or `en`, otherwise the service's own detection.
fn language_hint(language: Option<&str>) -> Option<&'static str> {
    match language?.trim().split(['-', '_']).next()?.to_ascii_lowercase().as_str() {
        "zh" => Some("zh"),
        "en" => Some("en"),
        _ => None,
    }
}

fn event_id() -> String {
    format!("event_{}", uuid::Uuid::new_v4().simple())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tokio::net::TcpListener;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn config(base: &str, model: &str) -> AsrConfig {
        AsrConfig::new(base, model).with_token(Some("sf-test".into()))
    }

    /// A live client whose end of a take waits briefly.
    fn live(base: &str) -> StepfunAsr {
        let mut c = StepfunAsr::new(config(base, "stepaudio-2.5-asr-stream")).unwrap();
        c.ending = Ending { settle: Duration::from_millis(200), deadline: Duration::from_millis(1_500) };
        c
    }

    fn pcm(seconds: usize) -> Vec<u8> {
        (0..seconds * 16_000).flat_map(|i| (((i * 7919 % 2000) as i16) - 1000).to_le_bytes()).collect()
    }

    fn take(seconds: usize) -> Vec<u8> {
        crate::split::wav(&pcm(seconds), 16_000)
    }

    #[tokio::test]
    async fn a_whole_file_goes_to_transcriptions_with_its_hot_words() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/transcriptions"))
            .and(header("authorization", "Bearer sf-test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "text": " 测试录制音频 " })))
            .mount(&server)
            .await;
        let c = StepfunAsr::new(config(&format!("{}/v1", server.uri()), "stepaudio-2.5-asr")).unwrap();
        assert!(!c.streams() && c.sends_glossary(&["Voltip".into()]));
        let t = c.transcribe(&take(2), Some("zh"), &["Voltip".into(), " ".into(), "术语".into()]).await.unwrap();
        assert_eq!((t.text.as_str(), t.model.as_str()), ("测试录制音频", "stepaudio-2.5-asr"));
        let body = String::from_utf8_lossy(&server.received_requests().await.unwrap()[0].body).into_owned();
        assert!(body.contains("name=\"response_format\"\r\n\r\njson"), "response_format is required");
        assert!(body.contains("name=\"hotwords\"\r\n\r\n[\"Voltip\",\"术语\"]"), "hot words as a JSON array in a string");
        // A `…-stream` model's recorded take goes to the whole-file model, which the receipt names.
        let stream = StepfunAsr::new(config(&format!("{}/v1", server.uri()), "stepaudio-2.5-asr-stream")).unwrap();
        assert!(stream.streams());
        let t = stream.transcribe(&take(1), None, &[]).await.unwrap();
        assert_eq!(t.model, WHOLE_FILE_MODEL);
        let body = String::from_utf8_lossy(&server.received_requests().await.unwrap()[1].body).into_owned();
        assert!(body.contains("name=\"model\"\r\n\r\nstepaudio-2.5-asr\r\n") && !body.contains("hotwords"), "{}", &body[..200.min(body.len())]);
    }

    #[tokio::test]
    async fn http_refusals_keep_their_kind() {
        for (status, kind) in [(401, "unauthorized"), (402, "quota"), (429, "rate"), (500, "server")] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(status).set_body_json(json!({ "error": { "message": "no", "type": "x" } })))
                .mount(&server)
                .await;
            let err = StepfunAsr::new(config(&format!("{}/v1", server.uri()), "stepaudio-2.5-asr")).unwrap().transcribe(&take(1), None, &[]).await.unwrap_err();
            let ok = match kind {
                "unauthorized" => matches!(err, AsrError::Unauthorized { .. }),
                "quota" => matches!(err, AsrError::QuotaExhausted { .. }),
                "rate" => matches!(err, AsrError::RateLimited { .. }),
                _ => matches!(err, AsrError::Server { status: 500, .. }),
            };
            assert!(ok, "{status}: {err:?}");
        }
    }

    #[test]
    fn the_prompt_and_hot_words_keep_their_limits() {
        assert_eq!(prompt(&["A".into(), " ".into(), "B".into()]).as_deref(), Some("专业术语：A、B"));
        assert_eq!(prompt(&[]), None);
        let many: Vec<String> = (0..500).map(|i| format!("术语{i:03}")).collect();
        let p = prompt(&many).unwrap();
        assert!(p.chars().count() <= MAX_PROMPT_CHARS && p.ends_with(|c: char| c.is_ascii_digit()), "{}", p.chars().count());
        assert_eq!(hotwords(&many).len(), MAX_HOTWORDS);
    }

    /// What the scripted realtime endpoint saw.
    #[derive(Default)]
    struct Seen {
        authorization: Option<String>,
        update: Option<Value>,
        /// The bytes of each append, in order.
        appends: Vec<Vec<u8>>,
        committed: bool,
        closed: bool,
        /// The client sent something after `session.update` before the service confirmed it.
        early_audio: bool,
    }

    /// How the scripted endpoint answers the final commit (and the first audio).
    #[derive(Clone, Copy, PartialEq, Debug)]
    enum Script {
        /// The closing silence stops the item being spoken; the commit, of an empty buffer, is
        /// refused with its event id.
        StopThenRefuse,
        /// The commit commits the item being spoken and its text follows.
        CommitTheOpenItem,
        /// One sentence, then the service says nothing at all after the commit.
        Silent,
        /// The second sentence never stops and the service ignores the commit.
        SpeakingForever,
        /// The commit commits an item that is never recognised.
        NeverCompletes,
        /// An error at the first audio.
        FailAtOnce,
        /// The service fails after the end.
        FailAtTheEnd,
        /// One sentence spoken as the doc's deltas show it, its tail in `stash`; then silence.
        Stash,
        /// `session.updated` comes 300 ms after `session.update`; then one sentence and silence.
        UpdatesLate,
        /// The service never confirms the configuration.
        NeverUpdates,
    }

    // tungstenite's handshake callback returns its `ErrorResponse` by value.
    #[allow(clippy::result_large_err)]
    async fn serve(script: Script) -> (String, Arc<Mutex<Seen>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Seen::default()));
        let shared = seen.clone();
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                let seen = shared.clone();
                tokio::spawn(async move {
                    let record = |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                        seen.lock().unwrap().authorization = request.headers().get("authorization").and_then(|v| v.to_str().ok()).map(str::to_owned);
                        assert_eq!(request.uri().path(), "/v1/realtime/asr/stream");
                        Ok(response)
                    };
                    let Ok(socket) = tokio_tungstenite::accept_hdr_async(tcp, record).await else { return };
                    let (mut sink, mut stream) = socket.split();
                    let mut say = vec![json!({ "type": "session.created", "event_id": "e0" })];
                    let mut first_audio = true;
                    loop {
                        for event in say.drain(..) {
                            let _ = sink.send(Message::text(event.to_string())).await;
                        }
                        let Some(Ok(message)) = stream.next().await else { break };
                        let Message::Text(text) = message else {
                            if matches!(message, Message::Close(_)) {
                                break;
                            }
                            continue;
                        };
                        let value: Value = serde_json::from_str(text.as_str()).unwrap();
                        match event_type(&value) {
                            "session.update" => {
                                seen.lock().unwrap().update = Some(value.clone());
                                match script {
                                    Script::NeverUpdates => {}
                                    Script::UpdatesLate => {
                                        // Whatever the client sends before the confirmation comes too early.
                                        if tokio::time::timeout(Duration::from_millis(300), stream.next()).await.is_ok() {
                                            seen.lock().unwrap().early_audio = true;
                                        }
                                        say.push(json!({ "type": "session.updated" }));
                                    }
                                    _ => say.push(json!({ "type": "session.updated" })),
                                }
                            }
                            "input_audio_buffer.append" => {
                                let bytes = base64::engine::general_purpose::STANDARD.decode(value["audio"].as_str().unwrap()).unwrap();
                                let silence = bytes.iter().all(|b| *b == 0);
                                seen.lock().unwrap().appends.push(bytes);
                                if std::mem::take(&mut first_audio) {
                                    if script == Script::FailAtOnce {
                                        say.push(json!({ "type": "error", "error": { "type": "invalid_request_error", "code": "invalid_value", "message": "bad audio" } }));
                                        continue;
                                    }
                                    if script == Script::Stash {
                                        say.extend([
                                            json!({ "type": "input_audio_buffer.speech_started", "audio_start_ms": 0, "item_id": "i1" }),
                                            json!({ "type": "conversation.item.input_audio_transcription.delta", "item_id": "i1", "text": "", "stash": "今天" }),
                                            json!({ "type": "conversation.item.input_audio_transcription.delta", "item_id": "i1", "text": "今天天气", "stash": "不错" }),
                                            json!({ "type": "input_audio_buffer.speech_stopped", "audio_start_ms": 900, "item_id": "i1" }),
                                            json!({ "type": "input_audio_buffer.committed", "item_id": "i1" }),
                                            json!({ "type": "conversation.item.input_audio_transcription.completed", "item_id": "i1", "transcript": "今天天气不错。", "words": [{ "word": "今", "start": 0.1, "end": 0.3 }, { "word": "错", "start": 0.6, "end": 0.8 }] }),
                                        ]);
                                        continue;
                                    }
                                    say.extend([
                                        json!({ "type": "input_audio_buffer.speech_started", "audio_start_ms": 0, "item_id": "i1" }),
                                        json!({ "type": "conversation.item.input_audio_transcription.delta", "item_id": "i1", "text": "你好" }),
                                        json!({ "type": "input_audio_buffer.speech_stopped", "audio_start_ms": 900, "item_id": "i1" }),
                                        json!({ "type": "input_audio_buffer.committed", "item_id": "i1" }),
                                        json!({ "type": "conversation.item.created", "item": { "id": "i1", "status": "in_progress" } }),
                                        json!({ "type": "conversation.item.input_audio_transcription.completed", "item_id": "i1", "transcript": "你好。", "words": [{ "word": "你", "start": 0.1, "end": 0.3 }, { "word": "好", "start": 0.3, "end": 0.8 }] }),
                                    ]);
                                    if !matches!(script, Script::Silent | Script::UpdatesLate) {
                                        say.extend([
                                            json!({ "type": "input_audio_buffer.speech_started", "audio_start_ms": 1200, "item_id": "i2" }),
                                            json!({ "type": "conversation.item.input_audio_transcription.delta", "item_id": "i2", "text": "世" }),
                                        ]);
                                    }
                                } else if silence && script == Script::StopThenRefuse {
                                    say.extend([
                                        json!({ "type": "input_audio_buffer.speech_stopped", "audio_start_ms": 2100, "item_id": "i2" }),
                                        json!({ "type": "input_audio_buffer.committed", "item_id": "i2" }),
                                        json!({ "type": "conversation.item.input_audio_transcription.delta", "item_id": "i2", "text": "世界" }),
                                    ]);
                                }
                            }
                            "input_audio_buffer.commit" => {
                                seen.lock().unwrap().committed = true;
                                let id = value["event_id"].clone();
                                match script {
                                    Script::StopThenRefuse => say.extend([
                                        json!({ "type": "conversation.item.input_audio_transcription.completed", "item_id": "i2", "transcript": "世界。" }),
                                        json!({ "type": "error", "error": { "type": "invalid_request_error", "code": "invalid_value", "message": "buffer too small", "event_id": id } }),
                                    ]),
                                    Script::CommitTheOpenItem => say.extend([
                                        json!({ "type": "input_audio_buffer.committed", "item_id": "i2" }),
                                        json!({ "type": "conversation.item.input_audio_transcription.delta", "item_id": "i2", "text": "世界" }),
                                        json!({ "type": "conversation.item.input_audio_transcription.completed", "item_id": "i2", "transcript": "世界。" }),
                                    ]),
                                    Script::NeverCompletes => say.push(json!({ "type": "input_audio_buffer.committed", "item_id": "i2" })),
                                    Script::FailAtTheEnd => say.push(json!({ "type": "error", "error": { "type": "internal_error", "code": "internal_error", "message": "boom" } })),
                                    Script::Silent
                                    | Script::SpeakingForever
                                    | Script::FailAtOnce
                                    | Script::Stash
                                    | Script::UpdatesLate
                                    | Script::NeverUpdates => {}
                                }
                            }
                            _ => {}
                        }
                    }
                    seen.lock().unwrap().closed = true;
                });
            }
        });
        (base, seen)
    }

    async fn run(client: &StepfunAsr, audio: &[Vec<u8>], finish: bool) -> (Result<(), AsrError>, Vec<RealtimeEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let (events_tx, mut events) = mpsc::unbounded_channel();
        for pcm in audio {
            tx.send(RealtimeInput::Audio(pcm.clone())).unwrap();
        }
        if finish {
            tx.send(RealtimeInput::Finish).unwrap();
        } else {
            drop(tx);
        }
        let result =
            tokio::time::timeout(Duration::from_secs(20), client.realtime(Some("zh-CN"), &["Voltip".into()], rx, events_tx)).await.expect("the session ends");
        let mut out = Vec::new();
        while let Ok(event) = events.try_recv() {
            out.push(event);
        }
        (result, out)
    }

    fn spoken() -> Vec<RealtimeEvent> {
        vec![
            RealtimeEvent::Partial { text: "你好".into() },
            RealtimeEvent::Sentence { text: "你好。".into(), begin_ms: 0, end_ms: 700 },
            RealtimeEvent::Partial { text: "世".into() },
            RealtimeEvent::Partial { text: "世界".into() },
        ]
    }

    /// docs/dictation.md §3.15: the session's configuration, its items as partials and sentences in
    /// the take's time, and at the end the closing silence and the commit, whose refusal (nothing
    /// left in the buffer) ends the session once the item the silence closed is recognised.
    #[tokio::test]
    async fn a_live_session_streams_its_items_and_flushes_at_the_end() {
        let (base, seen) = serve(Script::StopThenRefuse).await;
        let c = live(&base);
        let (result, events) = run(&c, &[pcm(2), pcm(1)[..3_200].to_vec()], true).await;
        assert_eq!(result, Ok(()));
        let mut want = spoken();
        want.extend([RealtimeEvent::Sentence { text: "世界。".into(), begin_ms: 1200, end_ms: 2100 }, RealtimeEvent::Finished]);
        assert_eq!(events, want);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.authorization.as_deref(), Some("Bearer sf-test"));
        let sizes: Vec<usize> = seen.appends.iter().map(Vec::len).collect();
        assert_eq!(sizes, [32_000, 32_000, 3_200, 32_000], "one second per append, then a second of silence");
        assert!(seen.appends[3].iter().all(|b| *b == 0) && seen.committed);
        let input = &seen.update.as_ref().unwrap()["session"]["audio"]["input"];
        assert_eq!(input["format"], json!({ "type": "pcm", "codec": "pcm_s16le", "rate": 16000, "bits": 16, "channel": 1 }));
        assert_eq!(input["transcription"]["model"], "stepaudio-2.5-asr-stream");
        assert_eq!(input["transcription"]["language"], "zh");
        assert_eq!(input["transcription"]["prompt"], "专业术语：Voltip");
        assert_eq!(input["turn_detection"], json!({ "type": "server_vad", "silence_duration_ms": 800, "threshold": 0.6 }));
    }

    /// The end of a take in each case the service may answer it: the commit taking the item being
    /// spoken, or silence (the session settles); an item never recognised and an error at the end
    /// fail the session, so the take is recognised whole.
    #[tokio::test]
    async fn the_end_of_a_session_in_each_case() {
        let (base, _) = serve(Script::CommitTheOpenItem).await;
        let (result, events) = run(&live(&base), &[pcm(2)], true).await;
        let mut want = spoken()[..3].to_vec();
        want.extend([
            RealtimeEvent::Partial { text: "世界".into() },
            RealtimeEvent::Sentence { text: "世界。".into(), begin_ms: 1200, end_ms: 2000 },
            RealtimeEvent::Finished,
        ]);
        assert_eq!((result, events), (Ok(()), want), "the commit's own item ends at the audio sent");

        let (base, _) = serve(Script::Silent).await;
        let started = tokio::time::Instant::now();
        let (result, events) = run(&live(&base), &[pcm(1)], true).await;
        assert_eq!((result, events.last()), (Ok(()), Some(&RealtimeEvent::Finished)));
        assert!(started.elapsed() >= Duration::from_millis(200), "it settled: {:?}", started.elapsed());

        // A sentence still being spoken keeps the session from settling: it waits to its deadline.
        let (base, _) = serve(Script::SpeakingForever).await;
        let (result, _) = run(&live(&base), &[pcm(1)], true).await;
        assert_eq!(result, Err(AsrError::Timeout));

        let (base, _) = serve(Script::NeverCompletes).await;
        let (result, events) = run(&live(&base), &[pcm(1)], true).await;
        assert_eq!(result, Err(AsrError::Timeout));
        assert!(!events.contains(&RealtimeEvent::Finished));

        let (base, _) = serve(Script::FailAtTheEnd).await;
        let (result, _) = run(&live(&base), &[pcm(1)], true).await;
        assert!(matches!(result, Err(AsrError::Server { status: 500, .. })), "{result:?}");
    }

    /// An error while the take is spoken fails the session with its kind; a cancelled take closes
    /// the connection at once, and nothing more goes out.
    #[tokio::test]
    async fn an_error_fails_the_session_and_a_cancel_closes_it() {
        let (base, _) = serve(Script::FailAtOnce).await;
        let c = live(&base);
        let (tx, rx) = mpsc::unbounded_channel();
        tx.send(RealtimeInput::Audio(pcm(1))).unwrap();
        let (events_tx, _events) = mpsc::unbounded_channel();
        let err = tokio::time::timeout(Duration::from_secs(20), c.realtime(None, &[], rx, events_tx)).await.expect("ends").unwrap_err();
        assert!(matches!(&err, AsrError::Service { code, .. } if code == "invalid_value"), "{err:?}");
        drop(tx);

        let (base, seen) = serve(Script::StopThenRefuse).await;
        let (result, events) = run(&live(&base), &[], false).await;
        assert_eq!(result, Ok(()));
        assert!(!events.contains(&RealtimeEvent::Finished), "a cancelled take does not finish");
        tokio::time::timeout(Duration::from_secs(5), async {
            while !seen.lock().unwrap().closed {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the connection closes");
        let seen = seen.lock().unwrap();
        assert!(seen.appends.is_empty() && !seen.committed, "nothing more went out");
    }

    /// regression: StepFun's delta carries the item's settled `text` and the tail it may still
    /// correct (`stash`); the preview showed `text` alone, so nothing while an item started (all of
    /// it in the stash) and never its last words.
    #[tokio::test]
    async fn regression_the_preview_is_the_text_and_its_stash() {
        let (base, _) = serve(Script::Stash).await;
        let (result, events) = run(&live(&base), &[pcm(1)], true).await;
        assert_eq!(result, Ok(()));
        assert_eq!(
            events,
            vec![
                RealtimeEvent::Partial { text: "今天".into() },
                RealtimeEvent::Partial { text: "今天天气不错".into() },
                RealtimeEvent::Sentence { text: "今天天气不错。".into(), begin_ms: 0, end_ms: 700 },
                RealtimeEvent::Finished,
            ]
        );
    }

    /// regression: the audio went out right after `session.update`, before the service confirmed
    /// the configuration with `session.updated` (the doc's order). Now it waits; a service that
    /// never confirms fails the session within its opening time, so the take is recognised whole.
    #[tokio::test]
    async fn regression_audio_waits_for_the_configuration_to_be_confirmed() {
        let (base, seen) = serve(Script::UpdatesLate).await;
        let (result, events) = run(&live(&base), &[pcm(1)], true).await;
        assert_eq!(result, Ok(()));
        assert_eq!(events.first(), Some(&RealtimeEvent::Partial { text: "你好".into() }));
        assert!(!seen.lock().unwrap().early_audio, "nothing went out before session.updated");

        let (base, seen) = serve(Script::NeverUpdates).await;
        let mut c = live(&base);
        c.opening = Duration::from_millis(500);
        let (result, events) = run(&c, &[pcm(1)], true).await;
        assert_eq!((result, events), (Err(AsrError::Timeout), vec![]));
        assert!(seen.lock().unwrap().appends.is_empty(), "no audio went to a session that was not configured");
    }

    #[test]
    fn service_errors_are_sorted() {
        let error = |kind: &str, code: &str| json!({ "type": "error", "error": { "type": kind, "code": code, "message": "m" } });
        assert!(matches!(service_event_error(&error("internal_error", "internal_error")), AsrError::Server { status: 500, .. }));
        assert!(matches!(service_event_error(&error("invalid_request_error", "max_idle_timeout")), AsrError::Network(_)));
        assert!(matches!(service_event_error(&error("risk", "risk_blocked")), AsrError::Service { .. }));
        // Before the end of the take an error is the session's; after it, a refused commit is not.
        let mut session = Session::default();
        assert!(session.on(&error("invalid_request_error", "invalid_value"), None).is_err());
        assert_eq!(session.on(&error("invalid_request_error", "invalid_value"), Some("event_x")), Ok(None));
        assert!(session.done());
        assert!(Session::default().on(&error("risk", "risk_blocked"), Some("event_x")).is_err());
    }

    /// A take longer than one request's 100 MB goes in pieces cut in its pauses; a shorter one, or
    /// one that is not this PCM, as it is.
    #[test]
    fn a_very_long_take_goes_in_pieces() {
        assert!(MAX_PIECE_MS as usize * BYTES_PER_MS < 95 * 1024 * 1024);
        let wav = take(7);
        let cut = pieces(&wav, 3_000);
        let bytes: Vec<usize> = cut.iter().map(|w| wav_format(w).map(|f| f.data.1 - f.data.0).unwrap()).collect();
        assert!(bytes.len() >= 3 && bytes.iter().all(|b| *b <= 3 * 32_000), "{bytes:?}");
        assert_eq!(bytes.iter().sum::<usize>(), 7 * 32_000, "the pieces cover the take");
        assert_eq!(pieces(&wav, 7_000), std::slice::from_ref(&wav));
        assert_eq!(pieces(b"not a wav", 1), [b"not a wav".to_vec()]);
    }
}
