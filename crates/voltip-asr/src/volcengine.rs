//! 豆包语音, Volcengine's speech service (docs/dictation.md §3.15): Seed-ASR over the binary
//! WebSocket protocol of its 大模型流式语音识别 API. Live takes go to `bigmodel_async`
//! (双向流式优化版) with 二遍识别 (`enable_nonstream`): the utterances it marks `definite` are the
//! finished sentences. Recorded takes and 测试连接 go to `bigmodel_nostream` (流式输入), whose last
//! answer holds the take's text. The credential is the new console's API key (`X-Api-Key`) or the
//! old one's App ID and Access Token; the model is the `X-Api-Resource-Id`.
//!
//! Every message is one binary frame: a 4-byte header (protocol version and header size, message
//! type and its flags, serialization and compression), the payload's size (4 bytes, big-endian)
//! and the payload. The client sends one full client request (the JSON parameters), then
//! audio-only requests of PCM (100 ms each in a live session, 200 ms for a recorded take and the
//! probe), the last one flagged `0b0010`. The service answers with full server responses,
//! acknowledgements it reads past, or an error frame (a code and a message). A response's flags say
//! what follows the header, as the official demos read them: a sequence number (`0b0001`), an event
//! number (`0b0100`), and whether it is the last (`0b0010`).
//! A response may carry an error of its own in its JSON (`code`, `message`). The client gzips its
//! payloads; the service answers with the compression each frame's header names.

use std::io::{Read as _, Write as _};
use std::time::Duration;

use async_trait::async_trait;
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt as _, StreamExt as _};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::client::Request;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use url::Url;

use crate::client::{Transcript, truncate_chars};
use crate::config::{AsrConfig, MAX_ERROR_BODY_CHARS};
use crate::credentials;
use crate::dashscope::{join_sentences, wav_format};
use crate::error::AsrError;
use crate::realtime::{self, BYTES_PER_MS, Pace, Socket};
use crate::vendor::{self, RealtimeEvent, RealtimeEvents, RealtimeInput, VendorAsr};

/// Where live takes go: 双向流式优化版, which answers whenever the result changes.
const LIVE_PATH: &str = "/api/v3/sauc/bigmodel_async";
/// Where recorded takes and the probe go: 流式输入, which answers the last packet with the text.
const WHOLE_PATH: &str = "/api/v3/sauc/bigmodel_nostream";
/// One audio packet of a recorded take and of the probe: 200 ms of 16 kHz 16-bit mono, the size the
/// doc finds best for its 双向流式 mode (单包为200ms大小时性能最优). A live session sends 100 ms packets
/// ([`realtime::LIVE_PACKET_BYTES`]), the shortest of the 100–200 ms the doc asks for, so the first
/// words of a sentence wait less for the rest of their packet (docs/dictation.md §3.15).
const FRAME_BYTES: usize = 200 * BYTES_PER_MS;
/// How long one packet plays.
const FRAME_DURATION: Duration = Duration::from_millis(200);
/// The last packet of a live session when no audio is left over: 100 ms of silence, so the packet
/// that ends the session always carries audio.
const TAIL_BYTES: usize = 100 * BYTES_PER_MS;
/// A recorded take goes at twice the speed it plays, 200 ms every 100 ms: within the 100–200 ms
/// between packets the doc asks for (docs/dictation.md §3.15).
const WHOLE_PACE: Pace = Pace::Realtime { speedup: 2 };
/// The hot words a live session takes: 100 tokens in all, counted as characters to stay under it.
const LIVE_HOTWORD_CHARS: usize = 100;
/// The hot words `bigmodel_nostream` takes.
const WHOLE_HOTWORDS: usize = 5000;
/// How long a failed send waits for the error frame that explains it.
const EXPLAIN_WAIT: Duration = Duration::from_secs(1);
/// The largest payload read out of a frame once decompressed: a result is a few kilobytes.
const MAX_PAYLOAD_BYTES: u64 = 8 * 1024 * 1024;

// The binary protocol (the doc's header 字段描述).
/// Protocol version 1, a header of one 4-byte unit.
const VERSION_AND_HEADER_SIZE: u8 = 0b0001_0001;
/// Message type: the parameters, the session's first frame.
const FULL_CLIENT_REQUEST: u8 = 0b0001;
/// Message type: audio.
const AUDIO_ONLY_REQUEST: u8 = 0b0010;
/// Message type: a result.
const FULL_SERVER_RESPONSE: u8 = 0b1001;
/// Message type: an acknowledgement, which says nothing about the take.
const SERVER_ACK: u8 = 0b1011;
/// Message type: the service's error frame.
const ERROR_RESPONSE: u8 = 0b1111;
/// Flags: no sequence number follows the header.
const NO_SEQUENCE: u8 = 0b0000;
/// Flags: a sequence number follows the header.
const SEQUENCE: u8 = 0b0001;
/// Flags: the last packet (the client's last audio, the service's last response).
const LAST_PACKET: u8 = 0b0010;
/// Flags: an event number follows the sequence number.
const EVENT: u8 = 0b0100;
/// Serialization: none (audio).
const RAW: u8 = 0b0000;
/// Serialization: JSON.
const JSON: u8 = 0b0001;
/// Compression: gzip.
const GZIP: u8 = 0b0001;

type Outgoing = SplitSink<Socket, Message>;
type Incoming = SplitStream<Socket>;

/// 豆包语音's recognition client: one resource (the model), one credential, a connection per
/// session.
pub(crate) struct VolcengineAsr {
    config: AsrConfig,
    /// The resource the account bought (`X-Api-Resource-Id`), trimmed.
    model: String,
    resource: HeaderValue,
    /// The credential's headers, marked sensitive.
    credential: Vec<(&'static str, HeaderValue)>,
    live_url: Url,
    whole_url: Url,
}

impl VolcengineAsr {
    /// The client of `config`: its base URL, its resource as the model, its credential as the JSON
    /// the core stored (`{"set":"api_key","api_key":…}` or `{"set":"app_token","app_id":…,
    /// "access_token":…}`).
    pub(crate) fn new(config: AsrConfig) -> Result<Self, AsrError> {
        let live_url = vendor::endpoint(&config.base_url, true, LIVE_PATH)?;
        let whole_url = vendor::endpoint(&config.base_url, true, WHOLE_PATH)?;
        let credential = credential_headers(config.token.as_deref())?;
        let model = config.model.trim().to_owned();
        if model.is_empty() {
            return Err(AsrError::InvalidConfig("model is empty".into()));
        }
        let resource = HeaderValue::from_str(&model).map_err(|_| AsrError::InvalidConfig("model is not a valid resource id".into()))?;
        if config.timeout.is_zero() {
            return Err(AsrError::InvalidConfig("timeout must be greater than zero".into()));
        }
        Ok(Self { config, model, resource, credential, live_url, whole_url })
    }

    /// The full client request's JSON (the doc's 参数): 16 kHz 16-bit mono PCM, punctuation and
    /// number formatting, the utterances with their times; a `live` session asks for 二遍识别,
    /// a recorded take names its language (`bigmodel_nostream` takes one, 二遍识别 does not); the
    /// glossary goes as hot words.
    fn parameters(&self, live: bool, language: Option<&str>, glossary: &[String]) -> Value {
        let mut audio = json!({ "format": "pcm", "codec": "raw", "rate": realtime::SAMPLE_RATE, "bits": 16, "channel": 1 });
        if !live && let Some(code) = language_code(language) {
            audio["language"] = json!(code);
        }
        let mut request = json!({ "model_name": "bigmodel", "enable_itn": true, "enable_punc": true, "show_utterances": true, "result_type": "full" });
        if live {
            request["enable_nonstream"] = json!(true);
        }
        if let Some(context) = hotwords(glossary, live) {
            request["corpus"] = json!({ "context": context });
        }
        json!({ "user": { "uid": "voltip" }, "audio": audio, "request": request })
    }

    /// The handshake of `url`: the credential, the resource, and a fresh id for the connection
    /// (the doc's 鉴权 table names `X-Api-Request-Id`, its example `X-Api-Connect-Id`; both carry
    /// it).
    fn handshake(&self, url: &Url) -> Result<Request, AsrError> {
        let mut request = realtime::request(url)?;
        let headers = request.headers_mut();
        for (name, value) in &self.credential {
            headers.insert(*name, value.clone());
        }
        headers.insert("x-api-resource-id", self.resource.clone());
        let id = HeaderValue::from_str(&uuid::Uuid::new_v4().to_string()).map_err(|_| AsrError::InvalidConfig("connection id".into()))?;
        headers.insert("x-api-connect-id", id.clone());
        headers.insert("x-api-request-id", id);
        Ok(request)
    }

    /// Connect to `url` and send the full client request with `parameters`; the session's log id
    /// when the handshake named one.
    async fn open(&self, url: &Url, parameters: &Value) -> Result<(Outgoing, Incoming, Option<String>), AsrError> {
        let (socket, response) = realtime::connect(self.handshake(url)?, &self.config.network).await.map_err(refused)?;
        // The service's log id identifies the session in a report to Volcengine; it is no secret.
        let logid = response.headers().get("x-tt-logid").and_then(|v| v.to_str().ok()).map(str::trim).filter(|id| !id.is_empty()).map(str::to_owned);
        tracing::debug!(path = url.path(), model = %self.model, logid = logid.as_deref().unwrap_or_default(), "豆包语音 connected");
        let (mut outgoing, mut incoming) = socket.split();
        let json = serde_json::to_vec(parameters).map_err(|e| AsrError::InvalidConfig(format!("request parameters: {e}")))?;
        send(&mut outgoing, &mut incoming, client_frame(FULL_CLIENT_REQUEST, NO_SEQUENCE, JSON, &json)?).await?;
        Ok((outgoing, incoming, logid))
    }

    /// One `bigmodel_nostream` session over `pcm`: its packets at [`WHOLE_PACE`], the last one
    /// flagged, while reading the answers; the last answer's payload. An empty `pcm` goes as one
    /// packet of silence.
    async fn whole(&self, pcm: &[u8], parameters: &Value) -> Result<Value, AsrError> {
        let (mut outgoing, mut incoming, _) = self.open(&self.whole_url, parameters).await?;
        let silence = [0u8; FRAME_BYTES];
        let frames: Vec<&[u8]> = if pcm.is_empty() { vec![&silence[..]] } else { pcm.chunks(FRAME_BYTES).collect() };
        // Packets go out on a fixed schedule from the first one, so a slow send does not add up.
        let start = Instant::now();
        let mut next = 0;
        loop {
            tokio::select! {
                () = tokio::time::sleep_until(start + due(next)), if next < frames.len() => {
                    let flags = if next + 1 == frames.len() { LAST_PACKET } else { NO_SEQUENCE };
                    send(&mut outgoing, &mut incoming, client_frame(AUDIO_ONLY_REQUEST, flags, RAW, frames[next])?).await?;
                    next += 1;
                }
                response = next_response(&mut incoming) => {
                    let (last, payload) = response?;
                    if last {
                        let _ = outgoing.close().await;
                        return Ok(payload);
                    }
                }
            }
        }
    }
}

#[async_trait]
impl VendorAsr for VolcengineAsr {
    fn model(&self) -> &str {
        &self.model
    }

    fn sends_glossary(&self, glossary: &[String]) -> bool {
        !terms(glossary).is_empty()
    }

    /// A recorded take over `bigmodel_nostream` at twice the speed it plays; its text is the last
    /// answer's. Bounded by the client's timeout plus the time the sending takes.
    async fn transcribe(&self, wav: &[u8], language: Option<&str>, glossary: &[String]) -> Result<Transcript, AsrError> {
        let started = std::time::Instant::now();
        let pcm = pcm_of(wav)?;
        let text = if pcm.is_empty() {
            String::new()
        } else {
            let deadline = realtime::whole_take_deadline(self.config.timeout, realtime::played(pcm), WHOLE_PACE);
            let parameters = self.parameters(false, language, glossary);
            let payload = tokio::time::timeout(deadline, self.whole(pcm, &parameters)).await.map_err(|_| AsrError::Timeout)??;
            whole_text(&payload)
        };
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        tracing::debug!(latency_ms, chars = text.chars().count(), "豆包语音 transcribed a whole take");
        Ok(Transcript { text, latency_ms, model: self.model.clone() })
    }

    /// The probe of docs/dictation.md §3.15: the full client request and one last packet of
    /// 200 ms of silence; only the service's last response (`0b0011`) is success.
    async fn probe(&self) -> Result<(), AsrError> {
        let parameters = self.parameters(false, None, &[]);
        let silence = [0u8; FRAME_BYTES];
        tokio::time::timeout(self.config.timeout, self.whole(&silence, &parameters)).await.map_err(|_| AsrError::Timeout)??;
        Ok(())
    }

    fn streams(&self) -> bool {
        true
    }

    async fn realtime(
        &self,
        language: Option<&str>,
        glossary: &[String],
        mut input: mpsc::UnboundedReceiver<RealtimeInput>,
        events: RealtimeEvents,
    ) -> Result<(), AsrError> {
        let parameters = self.parameters(true, language, glossary);
        let (mut outgoing, mut incoming, logid) = self.open(&self.live_url, &parameters).await?;
        let mut results = Utterances::default();
        // The responses the service sent: what a session without text tells its history.
        let mut replies = 0usize;
        let mut pending: Vec<u8> = Vec::new();
        // Audio sent, in bytes: the times of an utterance that comes without them.
        let mut sent = 0usize;
        // Once the last packet is out, the service has the client's timeout to answer it.
        let mut finish_by: Option<Instant> = None;
        loop {
            tokio::select! {
                item = input.recv(), if finish_by.is_none() => match item {
                    Some(RealtimeInput::Audio(pcm)) => {
                        pending.extend_from_slice(&pcm);
                        while pending.len() >= realtime::LIVE_PACKET_BYTES {
                            let frame: Vec<u8> = pending.drain(..realtime::LIVE_PACKET_BYTES).collect();
                            send(&mut outgoing, &mut incoming, client_frame(AUDIO_ONLY_REQUEST, NO_SEQUENCE, RAW, &frame)?).await?;
                            sent += frame.len();
                        }
                    }
                    Some(RealtimeInput::Finish) => {
                        sent += pending.len();
                        let last = if pending.is_empty() { vec![0; TAIL_BYTES] } else { std::mem::take(&mut pending) };
                        send(&mut outgoing, &mut incoming, client_frame(AUDIO_ONLY_REQUEST, LAST_PACKET, RAW, &last)?).await?;
                        finish_by = Some(Instant::now() + self.config.timeout);
                    }
                    // A cancelled take: the connection closes, nothing more goes out.
                    None => {
                        let _ = outgoing.close().await;
                        return Ok(());
                    }
                },
                response = next_response(&mut incoming) => {
                    let (last, payload) = response?;
                    replies += 1;
                    results.read(&payload, last, played_ms(sent), &events);
                    if last {
                        tracing::info!(replies, sent_ms = played_ms(sent), said = results.said, logid = logid.as_deref().unwrap_or_default(), "豆包语音 session ended");
                        if !results.said {
                            let _ = events.send(RealtimeEvent::Note { text: silent_note(replies, logid.as_deref()) });
                        }
                        let _ = events.send(RealtimeEvent::Finished);
                        let _ = outgoing.close().await;
                        return Ok(());
                    }
                }
                () = tokio::time::sleep_until(finish_by.unwrap_or_else(Instant::now)), if finish_by.is_some() => return Err(AsrError::Timeout),
            }
        }
    }
}

/// What a live session's responses said so far: each response lists every utterance of the
/// session in order (`result_type: full`), so the n-th definite one is the same sentence in every
/// response that lists it.
#[derive(Debug, Default)]
struct Utterances {
    /// The definite utterances already passed on.
    committed: usize,
    /// Where the last sentence ended.
    end_ms: u64,
    /// The partial last passed on.
    partial: String,
    /// Whether any text was passed on, as a sentence or as the sentence being spoken.
    said: bool,
}

impl Utterances {
    /// Pass on what `payload` adds: each new definite utterance as a sentence with its times, the
    /// rest as the sentence being spoken. The `last` response finishes every utterance it lists.
    /// `sent_ms` is the audio sent so far, the end of an utterance the service gave no time.
    fn read(&mut self, payload: &Value, last: bool, sent_ms: u64, events: &RealtimeEvents) {
        let result = result_of(payload);
        let listed = result.and_then(|r| r.get("utterances")).and_then(Value::as_array).filter(|u| !u.is_empty());
        let Some(utterances) = listed else {
            // A response without utterances: its text is everything said so far.
            let text = result.and_then(|r| r.get("text")).and_then(Value::as_str).unwrap_or_default();
            if self.committed == 0 {
                if last {
                    self.sentence(text, None, None, sent_ms, events);
                } else {
                    self.partial(text, events);
                }
            }
            return;
        };
        let mut definite = 0;
        let mut speaking = Vec::new();
        for utterance in utterances {
            let text = utterance.get("text").and_then(Value::as_str).unwrap_or_default();
            let time = |key: &str| utterance.get(key).and_then(Value::as_u64);
            if utterance.get("definite").and_then(Value::as_bool) == Some(true) {
                definite += 1;
                if definite > self.committed {
                    self.committed = definite;
                    self.sentence(text, time("start_time"), time("end_time"), sent_ms, events);
                }
            } else if last {
                self.sentence(text, time("start_time"), time("end_time"), sent_ms, events);
            } else {
                speaking.push(text);
            }
        }
        if !last {
            self.partial(&join_sentences(&speaking), events);
        }
    }

    fn sentence(&mut self, text: &str, start: Option<u64>, end: Option<u64>, sent_ms: u64, events: &RealtimeEvents) {
        self.partial.clear();
        let begin_ms = start.unwrap_or(self.end_ms);
        let end_ms = end.unwrap_or(sent_ms).max(begin_ms);
        self.end_ms = end_ms;
        let text = text.trim();
        if !text.is_empty() {
            self.said = true;
            let _ = events.send(RealtimeEvent::Sentence { text: text.to_owned(), begin_ms, end_ms });
        }
    }

    fn partial(&mut self, text: &str, events: &RealtimeEvents) {
        let text = text.trim();
        if text != self.partial {
            text.clone_into(&mut self.partial);
            if !text.is_empty() {
                self.said = true;
                let _ = events.send(RealtimeEvent::Partial { text: text.to_owned() });
            }
        }
    }
}

/// What a live session that passed on no text says about it (docs/dictation.md §3.15): how often the
/// service replied, and the log id that finds the session in a report to Volcengine.
fn silent_note(replies: usize, logid: Option<&str>) -> String {
    match logid {
        Some(logid) => format!("豆包语音返回了 {replies} 次结果，均不含文字；日志 ID {logid}"),
        None => format!("豆包语音返回了 {replies} 次结果，均不含文字"),
    }
}

/// A response's `result`: the object, or the first of a list (the doc types it a list).
fn result_of(payload: &Value) -> Option<&Value> {
    match payload.get("result")? {
        Value::Array(list) => list.first(),
        result => Some(result),
    }
}

/// The text of a recorded take's last answer: `result.text`, else its utterances joined.
fn whole_text(payload: &Value) -> String {
    let result = result_of(payload);
    if let Some(text) = result.and_then(|r| r.get("text")).and_then(Value::as_str) {
        return text.trim().to_owned();
    }
    let utterances = result.and_then(|r| r.get("utterances")).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    join_sentences(&utterances.iter().filter_map(|u| u.get("text").and_then(Value::as_str)).collect::<Vec<_>>())
}

/// Send `frame`; a send that fails is explained by what the service said before it closed.
async fn send(outgoing: &mut Outgoing, incoming: &mut Incoming, frame: Vec<u8>) -> Result<(), AsrError> {
    let Err(error) = outgoing.send(Message::binary(frame)).await else { return Ok(()) };
    let error = realtime::map_ws(error);
    Err(match tokio::time::timeout(EXPLAIN_WAIT, next_response(incoming)).await {
        Ok(Err(said)) if !matches!(said, AsrError::Network(_)) => said,
        _ => error,
    })
}

/// The service's next response: whether it is the last, and its JSON. Acknowledgements are read
/// past. An error frame, an error inside a response, a frame that cannot be read, or a connection
/// that ends is an error.
async fn next_response(incoming: &mut Incoming) -> Result<(bool, Value), AsrError> {
    loop {
        let bytes = match incoming.next().await {
            Some(Ok(Message::Binary(bytes))) => bytes,
            Some(Ok(Message::Close(_))) | None => return Err(realtime::closed_early()),
            // Pings are answered by tungstenite itself; the protocol sends no text.
            Some(Ok(_)) => continue,
            Some(Err(e)) => return Err(realtime::map_ws(e)),
        };
        match server_frame(&bytes)? {
            ServerFrame::Ack => {}
            ServerFrame::Response { last, payload } => {
                return match in_band_error(&payload) {
                    Some(error) => Err(error),
                    None => Ok((last, payload)),
                };
            }
            ServerFrame::Error { code, message } => return Err(frame_error(u64::from(code), &message)),
        }
    }
}

/// The error a full server response carries in its JSON (`code` and `message`, as the older API
/// and some gateways answer): any code but a success (`1000`, `20000000`), the service's
/// "processing" and "queued" (`20000001`, `20000002`) or a take without speech (`1013`,
/// `20000003`).
fn in_band_error(payload: &Value) -> Option<AsrError> {
    let code = match payload.get("code")? {
        Value::Number(number) => number.as_u64()?,
        Value::String(text) => text.trim().parse().ok()?,
        _ => return None,
    };
    if matches!(code, 0 | 1000 | 1013 | 20_000_000..=20_000_003) {
        return None;
    }
    let message = payload.get("message").and_then(Value::as_str).unwrap_or_default();
    Some(frame_error(code, message))
}

/// One client frame: the header, the gzipped payload's size, the gzipped payload.
fn client_frame(message_type: u8, flags: u8, serialization: u8, payload: &[u8]) -> Result<Vec<u8>, AsrError> {
    let body = gzip(payload)?;
    let size = u32::try_from(body.len()).map_err(|_| AsrError::Audio("a packet larger than 4 GiB".into()))?;
    let mut frame = Vec::with_capacity(8 + body.len());
    frame.extend_from_slice(&[VERSION_AND_HEADER_SIZE, message_type << 4 | flags, serialization << 4 | GZIP, 0]);
    frame.extend_from_slice(&size.to_be_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

fn gzip(bytes: &[u8]) -> Result<Vec<u8>, AsrError> {
    let failed = |e: std::io::Error| AsrError::Audio(format!("could not compress a packet: {e}"));
    let mut encoder = GzEncoder::new(Vec::with_capacity(bytes.len() / 2 + 32), Compression::fast());
    encoder.write_all(bytes).map_err(failed)?;
    encoder.finish().map_err(failed)
}

fn gunzip(bytes: &[u8]) -> Result<Vec<u8>, AsrError> {
    let mut out = Vec::new();
    GzDecoder::new(bytes).take(MAX_PAYLOAD_BYTES).read_to_end(&mut out).map_err(|e| AsrError::BadResponse(format!("豆包语音 frame: gzip: {e}")))?;
    Ok(out)
}

/// What the service sent in one binary frame.
#[derive(Clone, Debug, PartialEq)]
enum ServerFrame {
    /// A full server response: whether it is the last (`0b0010`), and its JSON (`null` when it
    /// carries none).
    Response { last: bool, payload: Value },
    /// An acknowledgement.
    Ack,
    /// An error frame: its code and message.
    Error { code: u32, message: String },
}

/// Read one frame of the service (the doc's full server response and error message from server).
fn server_frame(bytes: &[u8]) -> Result<ServerFrame, AsrError> {
    let bad = |why: &str| AsrError::BadResponse(format!("豆包语音 frame: {why}"));
    let header = bytes.get(..4).ok_or_else(|| bad("shorter than its header"))?;
    let mut at = usize::from(header[0] & 0x0F) * 4;
    if at < 4 {
        return Err(bad("a header size of 0"));
    }
    let (message_type, flags, compression) = (header[1] >> 4, header[1] & 0x0F, header[2] & 0x0F);
    if message_type == SERVER_ACK {
        return Ok(ServerFrame::Ack);
    }
    let mut word = || {
        let value = bytes.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
        at += 4;
        value.ok_or_else(|| bad("cut short"))
    };
    // The sequence number, then the event number, when the flags say they follow.
    if flags & SEQUENCE != 0 {
        word()?;
    }
    if flags & EVENT != 0 {
        word()?;
    }
    match message_type {
        FULL_SERVER_RESPONSE => {
            let size = usize::try_from(word()?).unwrap_or(usize::MAX);
            let payload = bytes.get(at..at.saturating_add(size)).ok_or_else(|| bad("its payload is cut short"))?;
            let payload = if compression == GZIP { gunzip(payload)? } else { payload.to_vec() };
            let payload = if payload.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&payload).map_err(|e| {
                    AsrError::BadResponse(format!("豆包语音 response: {e}: {}", truncate_chars(&String::from_utf8_lossy(&payload), MAX_ERROR_BODY_CHARS)))
                })?
            };
            Ok(ServerFrame::Response { last: flags & LAST_PACKET != 0, payload })
        }
        ERROR_RESPONSE => {
            let code = word()?;
            let size = usize::try_from(word()?).unwrap_or(usize::MAX);
            // A message cut short still says something.
            let raw = bytes.get(at..).unwrap_or_default();
            let raw = &raw[..size.min(raw.len())];
            let raw = if compression == GZIP { gunzip(raw).unwrap_or_else(|_| raw.to_vec()) } else { raw.to_vec() };
            let text = String::from_utf8_lossy(&raw);
            // The doc's example sends a JSON error object; take its message when it has one.
            let fields = voltip_reply::error_fields(&text);
            let message = if fields.message.is_empty() { text.trim().to_owned() } else { fields.message };
            Ok(ServerFrame::Error { code, message })
        }
        other => Err(bad(&format!("unexpected message type {other:#06b}"))),
    }
}

/// An error frame as an error a person can act on (the doc's 错误码 table): a wait for audio that
/// ran out (`45000081`), audio the service could not read (`45000151`), a busy server
/// (`55000031`) or another internal error (`550xxxxx`), and the service's own code for the rest
/// (`45000001` parameters, `45000002` empty audio).
fn frame_error(code: u64, message: &str) -> AsrError {
    let said = voltip_reply::clean(message);
    let with_code = || if said.is_empty() { code.to_string() } else { format!("{code}: {said}") };
    if says_arrears(message) {
        return AsrError::QuotaExhausted { code: code.to_string(), message: said, retry_at_ms: None };
    }
    match code {
        45_000_081 => AsrError::Timeout,
        45_000_151 => AsrError::Audio(with_code()),
        55_000_031 => AsrError::Server { status: 503, body: with_code() },
        55_000_000..=55_099_999 => AsrError::Server { status: 500, body: with_code() },
        _ => AsrError::Service { code: code.to_string(), message: said },
    }
}

/// A refused handshake as [`realtime::map_ws`] sorts it (401/403 the credential, 429 a rate
/// limit, 5xx the server). The doc lists no code for an account in arrears or out of balance, so a
/// refusal that says so in words is a used-up quota.
fn refused(error: AsrError) -> AsrError {
    let (code, said) = match &error {
        AsrError::Unauthorized { message } | AsrError::RateLimited { message, .. } => ("arrears".to_owned(), message.clone()),
        AsrError::Server { status, body } => (status.to_string(), body.clone()),
        AsrError::Service { code, message } => (code.clone(), message.clone()),
        _ => return error,
    };
    if says_arrears(&said) { AsrError::QuotaExhausted { code, message: said, retry_at_ms: None } } else { error }
}

/// Whether the service's words say the account is in arrears or its balance is used up.
fn says_arrears(text: &str) -> bool {
    let lower = text.to_lowercase();
    ["欠费", "余额不足", "arrear", "insufficient balance"].iter().any(|word| lower.contains(word))
}

/// The glossary's terms as hot words: trimmed, blank and repeated ones dropped, and none longer
/// than a live session takes in all.
fn terms(glossary: &[String]) -> Vec<&str> {
    let mut terms: Vec<&str> = Vec::new();
    for term in glossary.iter().map(|t| t.trim()) {
        if !term.is_empty() && term.chars().count() <= LIVE_HOTWORD_CHARS && !terms.contains(&term) {
            terms.push(term);
        }
    }
    terms
}

/// The request's `corpus.context` (docs/dictation.md §3.15): `{"hotwords":[{"word":…}]}` as a
/// JSON string. A live session takes [`LIVE_HOTWORD_CHARS`] of them in all, the terms that fit in
/// order; `bigmodel_nostream` the first [`WHOLE_HOTWORDS`].
fn hotwords(glossary: &[String], live: bool) -> Option<String> {
    let mut room = LIVE_HOTWORD_CHARS;
    let mut words = Vec::new();
    for term in terms(glossary) {
        if live {
            let length = term.chars().count();
            if length > room {
                continue;
            }
            room -= length;
        } else if words.len() == WHOLE_HOTWORDS {
            break;
        }
        words.push(json!({ "word": term }));
    }
    (!words.is_empty()).then(|| json!({ "hotwords": words }).to_string())
}

/// The `audio.language` of a recorded take for its language hint, from the doc's list. Chinese,
/// English and Cantonese go without one: the default model recognises them, the dialects too.
fn language_code(language: Option<&str>) -> Option<&'static str> {
    let primary = language?.trim().split(['-', '_']).next()?.to_ascii_lowercase();
    Some(match primary.as_str() {
        "ja" => "ja-JP",
        "ko" => "ko-KR",
        "id" => "id-ID",
        "es" => "es-MX",
        "pt" => "pt-BR",
        "de" => "de-DE",
        "fr" => "fr-FR",
        "fil" | "tl" => "fil-PH",
        "ms" => "ms-MY",
        "th" => "th-TH",
        "ar" => "ar-SA",
        "it" => "it-IT",
        "bn" => "bn-BD",
        "el" => "el-GR",
        "nl" => "nl-NL",
        "ru" => "ru-RU",
        "tr" => "tr-TR",
        "vi" => "vi-VN",
        "pl" => "pl-PL",
        "ro" => "ro-RO",
        "ne" => "ne-NP",
        "uk" => "uk-UA",
        _ => return None,
    })
}

/// The stored credential's fields: its set says which are used.
#[derive(Deserialize)]
struct Fields {
    #[serde(default)]
    api_key: String,
    #[serde(default)]
    app_id: String,
    #[serde(default)]
    access_token: String,
}

/// The headers that carry the credential in `token` (the doc's 鉴权 tables): the new console's
/// key, or the old one's App ID and Access Token. Marked sensitive, so no `Debug` prints them.
fn credential_headers(token: Option<&str>) -> Result<Vec<(&'static str, HeaderValue)>, AsrError> {
    let fields: Fields = credentials::parse(token)?;
    let app_token = |fields: &Fields| -> Result<Vec<(&'static str, String)>, AsrError> {
        Ok(vec![
            ("x-api-app-key", credentials::require(&fields.app_id, "app_id")?),
            ("x-api-access-key", credentials::require(&fields.access_token, "access_token")?),
        ])
    };
    let pairs = match credentials::set_of(token).as_deref() {
        Some("api_key") => vec![("x-api-key", credentials::require(&fields.api_key, "api_key")?)],
        Some("app_token") => app_token(&fields)?,
        Some(_) => return Err(AsrError::InvalidConfig("the credential is incomplete: its set is neither api_key nor app_token".into())),
        // An entry written without its set: its fields say which.
        None if !fields.api_key.trim().is_empty() => vec![("x-api-key", fields.api_key.trim().to_owned())],
        None => app_token(&fields)?,
    };
    pairs
        .into_iter()
        .map(|(name, value)| {
            let mut value =
                HeaderValue::from_str(&value).map_err(|_| AsrError::InvalidConfig("the credential has a character a request header cannot carry".into()))?;
            value.set_sensitive(true);
            Ok((name, value))
        })
        .collect()
}

/// The PCM of a 16 kHz 16-bit mono WAV; another rate is refused.
fn pcm_of(wav: &[u8]) -> Result<&[u8], AsrError> {
    let format = wav_format(wav)?;
    if format.sample_rate != realtime::SAMPLE_RATE {
        return Err(AsrError::Audio(format!("the take is {} Hz; 豆包语音 takes 16 kHz", format.sample_rate)));
    }
    Ok(&wav[format.data.0..format.data.1])
}

/// When the `n`-th packet of a recorded take goes out, counted from the first.
fn due(n: usize) -> Duration {
    WHOLE_PACE.sending(FRAME_DURATION.saturating_mul(u32::try_from(n).unwrap_or(u32::MAX)))
}

/// Milliseconds of `bytes` of the session's PCM.
fn played_ms(bytes: usize) -> u64 {
    u64::try_from(bytes / BYTES_PER_MS).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Instant as Clock;

    use tokio::net::{TcpListener, TcpStream};
    use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request as Handshake, Response as Answer};
    use tokio_tungstenite::tungstenite::http::StatusCode;

    use super::*;
    use crate::dashscope::tests::wav;

    /// Flags of the service's last response: the last, a negative sequence number following.
    const LAST_RESULT: u8 = 0b0011;

    const API_KEY: &str = r#"{"set":"api_key","api_key":"volc-test-key"}"#;
    const APP_TOKEN: &str = r#"{"set":"app_token","app_id":"1234567890","access_token":"volc-test-token"}"#;
    const MODEL: &str = "volc.seedasr.sauc.duration";

    /// A client frame as the scripted service read it, without the client's decoder.
    #[derive(Clone, Debug)]
    struct Received {
        message_type: u8,
        flags: u8,
        serialization: u8,
        compression: u8,
        /// Version 1, a 4-byte header, and a size that agrees with the frame: no sequence number.
        well_formed: bool,
        payload: Vec<u8>,
        at: Clock,
    }

    /// One connection the scripted service took.
    #[derive(Debug, Default)]
    struct Connection {
        path: String,
        headers: Vec<(String, String)>,
        frames: Vec<Received>,
        closed: bool,
    }

    impl Connection {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
        }

        fn audio(&self) -> Vec<&Received> {
            self.frames.iter().filter(|f| f.message_type == AUDIO_ONLY_REQUEST).collect()
        }

        fn parameters(&self) -> Value {
            serde_json::from_slice(&self.frames[0].payload).unwrap()
        }
    }

    /// What the scripted service does.
    #[derive(Clone, Default)]
    struct Script {
        /// Refuse the handshake with this status and body.
        refuse: Option<(u16, &'static str)>,
        /// Frames sent after the client's frame of this index (0 is the full client request).
        after: Vec<(usize, Vec<Vec<u8>>)>,
        /// Frames sent after the client's last packet.
        after_last: Vec<Vec<u8>>,
        /// Drop the connection after the client's frame of this index.
        drop_after: Option<usize>,
        /// The `X-Tt-Logid` the handshake answers with.
        logid: Option<&'static str>,
    }

    type Seen = Arc<Mutex<Vec<Connection>>>;

    /// A local 豆包语音 endpoint following `script`: its `http` base, what it saw, and a message
    /// for every connection that ended.
    async fn serve(script: Script) -> (String, Seen, mpsc::UnboundedReceiver<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen: Seen = Arc::default();
        let (ended_tx, ended) = mpsc::unbounded_channel();
        let shared = Arc::clone(&seen);
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                let (script, seen, ended) = (script.clone(), Arc::clone(&shared), ended_tx.clone());
                tokio::spawn(async move {
                    session(tcp, script, seen).await;
                    let _ = ended.send(());
                });
            }
        });
        (base, seen, ended)
    }

    // tungstenite's handshake callback returns its `ErrorResponse` by value.
    #[allow(clippy::result_large_err)]
    async fn session(tcp: TcpStream, script: Script, seen: Seen) {
        let index = {
            let mut seen = seen.lock().unwrap();
            seen.push(Connection::default());
            seen.len() - 1
        };
        let record = Arc::clone(&seen);
        let (refuse, logid) = (script.refuse, script.logid);
        let callback = move |request: &Handshake, mut response: Answer| -> Result<Answer, ErrorResponse> {
            let mut seen = record.lock().unwrap();
            seen[index].path = request.uri().path().to_owned();
            seen[index].headers = request.headers().iter().map(|(n, v)| (n.as_str().to_owned(), v.to_str().unwrap_or_default().to_owned())).collect();
            match refuse {
                Some((status, body)) => {
                    let mut refusal = ErrorResponse::new(Some(body.to_owned()));
                    *refusal.status_mut() = StatusCode::from_u16(status).unwrap();
                    Err(refusal)
                }
                None => {
                    if let Some(logid) = logid {
                        response.headers_mut().insert("x-tt-logid", HeaderValue::from_static(logid));
                    }
                    Ok(response)
                }
            }
        };
        let Ok(socket) = tokio_tungstenite::accept_hdr_async(tcp, callback).await else { return };
        let (mut sink, mut stream) = socket.split();
        let mut count = 0usize;
        while let Some(Ok(message)) = stream.next().await {
            match message {
                Message::Binary(bytes) => {
                    let frame = parse(&bytes);
                    let last = frame.message_type == AUDIO_ONLY_REQUEST && frame.flags == LAST_PACKET;
                    seen.lock().unwrap()[index].frames.push(frame);
                    for (after, frames) in &script.after {
                        if *after == count {
                            for frame in frames {
                                let _ = sink.send(Message::binary(frame.clone())).await;
                            }
                        }
                    }
                    if last {
                        for frame in &script.after_last {
                            let _ = sink.send(Message::binary(frame.clone())).await;
                        }
                    }
                    if script.drop_after == Some(count) {
                        return;
                    }
                    count += 1;
                }
                Message::Close(_) => {
                    seen.lock().unwrap()[index].closed = true;
                    break;
                }
                _ => {}
            }
        }
    }

    /// Read a client frame by hand: header, size, payload.
    fn parse(bytes: &[u8]) -> Received {
        let size = bytes.get(4..8).map_or(usize::MAX, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize);
        let body = bytes.get(8..).unwrap_or_default();
        let compression = bytes[2] & 0x0F;
        Received {
            message_type: bytes[1] >> 4,
            flags: bytes[1] & 0x0F,
            serialization: bytes[2] >> 4,
            compression,
            well_formed: bytes[0] == 0x11 && size == body.len(),
            payload: if compression == 1 { ungzip(body) } else { body.to_vec() },
            at: Clock::now(),
        }
    }

    fn gz(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    fn ungzip(bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        GzDecoder::new(bytes).read_to_end(&mut out).unwrap();
        out
    }

    /// A full server response as the service sends one: header, sequence number (when its flags
    /// say one follows), size, JSON.
    fn response(flags: u8, sequence: i32, payload: &Value, gzipped: bool) -> Vec<u8> {
        let json = payload.to_string().into_bytes();
        let body = if gzipped { gz(&json) } else { json };
        let mut frame = vec![0x11, 0x90 | flags, 0x10 | u8::from(gzipped), 0x00];
        if flags & SEQUENCE != 0 {
            frame.extend_from_slice(&sequence.to_be_bytes());
        }
        frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
        frame.extend_from_slice(&body);
        frame
    }

    /// An error frame as the doc's example shows it: JSON, uncompressed.
    fn error_frame(code: u32, message: &str) -> Vec<u8> {
        let mut frame = vec![0x11, 0xF0, 0x10, 0x00];
        frame.extend_from_slice(&code.to_be_bytes());
        frame.extend_from_slice(&(message.len() as u32).to_be_bytes());
        frame.extend_from_slice(message.as_bytes());
        frame
    }

    fn client(base: &str, token: &str) -> VolcengineAsr {
        VolcengineAsr::new(AsrConfig::new(base, MODEL).with_token(Some(token.into()))).unwrap()
    }

    fn client_with_timeout(base: &str, timeout: Duration) -> VolcengineAsr {
        VolcengineAsr::new(AsrConfig::new(base, MODEL).with_token(Some(API_KEY.into())).with_timeout(timeout)).unwrap()
    }

    /// The scripted service finished a connection.
    async fn wait(ended: &mut mpsc::UnboundedReceiver<()>) {
        tokio::time::timeout(Duration::from_secs(10), ended.recv()).await.unwrap().unwrap();
    }

    /// Run a live session over `audio`, ended with `Finish` or, without it, cancelled.
    async fn live(client: &VolcengineAsr, glossary: &[String], audio: &[Vec<u8>], finish: bool) -> (Result<(), AsrError>, Vec<RealtimeEvent>) {
        let (input_tx, input) = mpsc::unbounded_channel();
        for chunk in audio {
            input_tx.send(RealtimeInput::Audio(chunk.clone())).unwrap();
        }
        if finish {
            input_tx.send(RealtimeInput::Finish).unwrap();
        }
        drop(input_tx);
        let (events_tx, mut events) = mpsc::unbounded_channel();
        let result = client.realtime(Some("zh"), glossary, input, events_tx).await;
        let mut said = Vec::new();
        while let Ok(event) = events.try_recv() {
            said.push(event);
        }
        (result, said)
    }

    fn sentence(text: &str, begin_ms: u64, end_ms: u64) -> RealtimeEvent {
        RealtimeEvent::Sentence { text: text.into(), begin_ms, end_ms }
    }

    fn partial(text: &str) -> RealtimeEvent {
        RealtimeEvent::Partial { text: text.into() }
    }

    /// The doc's 协议详情: the 4-byte header (version 1, header size 1; message type and flags;
    /// serialization and compression; reserved), the big-endian size, the payload.
    #[test]
    fn frames_are_written_and_read_byte_for_byte() {
        let request = client_frame(FULL_CLIENT_REQUEST, NO_SEQUENCE, JSON, br#"{"a":1}"#).unwrap();
        assert_eq!(request[..4], [0x11, 0x10, 0x11, 0x00], "full client request, no sequence, JSON, gzip");
        assert_eq!(u32::from_be_bytes(request[4..8].try_into().unwrap()) as usize, request.len() - 8);
        assert_eq!(ungzip(&request[8..]), br#"{"a":1}"#);
        let audio = client_frame(AUDIO_ONLY_REQUEST, NO_SEQUENCE, RAW, &[1, 2, 3, 4]).unwrap();
        assert_eq!(audio[..4], [0x11, 0x20, 0x01, 0x00], "audio only, no sequence, raw, gzip");
        assert_eq!(ungzip(&audio[8..]), [1, 2, 3, 4]);
        let last = client_frame(AUDIO_ONLY_REQUEST, LAST_PACKET, RAW, &[5, 6]).unwrap();
        assert_eq!(last[..4], [0x11, 0x22, 0x01, 0x00], "the last packet: 0b0010");
        assert_eq!(ungzip(&last[8..]), [5, 6]);

        let json = br#"{"result":{"text":"hi"}}"#;
        // A response with its sequence number (0b0001), uncompressed.
        let mut plain = vec![0x11, 0x91, 0x10, 0x00, 0x00, 0x00, 0x00, 0x01];
        plain.extend_from_slice(&(json.len() as u32).to_be_bytes());
        plain.extend_from_slice(json);
        assert_eq!(server_frame(&plain).unwrap(), ServerFrame::Response { last: false, payload: json!({ "result": { "text": "hi" } }) });
        // The last result (0b0011): a negative sequence number, gzipped.
        let packed = gz(json);
        let mut last = vec![0x11, 0x93, 0x11, 0x00];
        last.extend_from_slice(&(-3i32).to_be_bytes());
        last.extend_from_slice(&(packed.len() as u32).to_be_bytes());
        last.extend_from_slice(&packed);
        assert_eq!(server_frame(&last).unwrap(), ServerFrame::Response { last: true, payload: json!({ "result": { "text": "hi" } }) });
        // No sequence number (0b0000), no payload.
        assert_eq!(server_frame(&[0x11, 0x90, 0x10, 0x00, 0, 0, 0, 0]).unwrap(), ServerFrame::Response { last: false, payload: Value::Null });
        // The error frame: code, size, the JSON error object (or plain text, or gzipped text).
        assert_eq!(
            server_frame(&error_frame(45_000_001, r#"{"error":"payload unmarshal: no request object before data"}"#)).unwrap(),
            ServerFrame::Error { code: 45_000_001, message: "payload unmarshal: no request object before data".into() }
        );
        assert_eq!(server_frame(&error_frame(55_000_031, " server busy ")).unwrap(), ServerFrame::Error { code: 55_000_031, message: "server busy".into() });
        let packed = gz("空音频".as_bytes());
        let mut zipped = vec![0x11, 0xF0, 0x11, 0x00];
        zipped.extend_from_slice(&45_000_002u32.to_be_bytes());
        zipped.extend_from_slice(&(packed.len() as u32).to_be_bytes());
        zipped.extend_from_slice(&packed);
        assert_eq!(server_frame(&zipped).unwrap(), ServerFrame::Error { code: 45_000_002, message: "空音频".into() });
        // What cannot be read.
        let bad: [&[u8]; 5] = [
            &[0x11, 0x91],
            &[0x11, 0x91, 0x10, 0x00, 0, 0, 0, 1, 0, 0, 0, 9, b'{'],
            &[0x10, 0x90, 0x10, 0x00, 0, 0, 0, 0],
            &[0x11, 0x20, 0x10, 0x00, 0, 0, 0, 0],
            &[0x11, 0x90, 0x10, 0x00, 0, 0, 0, 2, b'x', b'y'],
        ];
        for frame in bad {
            assert!(matches!(server_frame(frame), Err(AsrError::BadResponse(_))), "{frame:?}");
        }
    }

    /// The doc's 鉴权: the new console's key alone, or the old one's App ID and Access Token; the
    /// resource; a fresh id for every connection.
    #[tokio::test]
    async fn the_handshake_carries_the_credential_the_resource_and_a_fresh_id() {
        let script = Script { after_last: vec![response(LAST_RESULT, -2, &json!({ "result": { "text": "" } }), true)], ..Script::default() };
        let (base, seen, mut ended) = serve(script).await;
        client(&base, API_KEY).probe().await.unwrap();
        client(&base, API_KEY).probe().await.unwrap();
        client(&base, APP_TOKEN).probe().await.unwrap();
        for _ in 0..3 {
            wait(&mut ended).await;
        }
        let seen = seen.lock().unwrap();
        let (first, second, old) = (&seen[0], &seen[1], &seen[2]);
        assert_eq!(first.path, WHOLE_PATH);
        assert_eq!(first.header("x-api-key"), Some("volc-test-key"));
        assert_eq!((first.header("x-api-app-key"), first.header("x-api-access-key")), (None, None));
        assert_eq!(first.header("x-api-resource-id"), Some(MODEL));
        let id = first.header("x-api-connect-id").unwrap();
        assert!(uuid::Uuid::parse_str(id).is_ok(), "{id}");
        assert_eq!(first.header("x-api-request-id"), Some(id));
        assert_ne!(second.header("x-api-connect-id"), Some(id), "a fresh id for every connection");
        assert_eq!(old.header("x-api-app-key"), Some("1234567890"));
        assert_eq!(old.header("x-api-access-key"), Some("volc-test-token"));
        assert_eq!(old.header("x-api-key"), None);
        assert_eq!(old.header("x-api-resource-id"), Some(MODEL));
    }

    /// docs/dictation.md §3.15: the probe sends the parameters and one last packet of 200 ms of
    /// silence, and only the last result is success; an error frame or a refused handshake is the
    /// failure it names.
    #[tokio::test]
    async fn the_probe_sends_one_last_packet_of_silence_and_needs_the_last_result() {
        let script = Script {
            after: vec![(0, vec![response(SEQUENCE, 1, &json!({}), true)])],
            after_last: vec![response(LAST_RESULT, -2, &json!({ "audio_info": { "duration": 200 }, "result": { "text": "" } }), true)],
            ..Script::default()
        };
        let (base, seen, mut ended) = serve(script).await;
        assert_eq!(client(&base, API_KEY).probe().await, Ok(()));
        wait(&mut ended).await;
        {
            let seen = seen.lock().unwrap();
            let frames = &seen[0].frames;
            assert_eq!(frames.len(), 2, "the parameters and one packet: {frames:?}");
            assert!(frames.iter().all(|f| f.well_formed));
            assert_eq!((frames[0].message_type, frames[0].flags, frames[0].serialization, frames[0].compression), (1, 0, 1, 1));
            let parameters = seen[0].parameters();
            assert_eq!(parameters["request"]["model_name"], "bigmodel");
            assert!(parameters["request"].get("enable_nonstream").is_none() && parameters["request"].get("corpus").is_none(), "{parameters}");
            assert_eq!((frames[1].message_type, frames[1].flags, frames[1].serialization, frames[1].compression), (2, 2, 0, 1), "the last packet");
            assert_eq!(frames[1].payload, vec![0; 6400], "200 ms of silence");
            assert!(seen[0].closed);
        }
        // An answer to the last packet that is not the last result, then a closed connection.
        let script = Script { after_last: vec![response(SEQUENCE, 2, &json!({ "result": { "text": "" } }), true)], drop_after: Some(1), ..Script::default() };
        let (base, _, _) = serve(script).await;
        assert!(matches!(client(&base, API_KEY).probe().await, Err(AsrError::Network(_))));
        // An error frame, the doc's empty audio.
        let script = Script { after_last: vec![error_frame(45_000_002, r#"{"error":"empty audio"}"#)], ..Script::default() };
        let (base, _, _) = serve(script).await;
        assert_eq!(client(&base, API_KEY).probe().await, Err(AsrError::Service { code: "45000002".into(), message: "empty audio".into() }));
        // Refused handshakes: a credential without a grant, a resource the account has not bought,
        // a rate limit, arrears, a failing server.
        let refusals = [
            (401, r#"{"error":"load grant: requested grant not found"}"#, AsrError::Unauthorized { message: "load grant: requested grant not found".into() }),
            (
                403,
                r#"{"error":"[resource_id=volc.seedasr.sauc.duration] requested resource not granted"}"#,
                AsrError::Unauthorized { message: "[resource_id=volc.seedasr.sauc.duration] requested resource not granted".into() },
            ),
            (429, r#"{"error":"too many requests"}"#, AsrError::RateLimited { retry_after_ms: None, message: "too many requests".into() }),
            (403, r#"{"error":"账户已欠费"}"#, AsrError::QuotaExhausted { code: "arrears".into(), message: "账户已欠费".into(), retry_at_ms: None }),
            (500, "oops", AsrError::Server { status: 500, body: "oops".into() }),
        ];
        for (status, body, want) in refusals {
            let (base, _, _) = serve(Script { refuse: Some((status, body)), ..Script::default() }).await;
            assert_eq!(client(&base, API_KEY).probe().await, Err(want), "{status} {body}");
        }
        // Nobody listening.
        let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", closed.local_addr().unwrap());
        drop(closed);
        let err = client(&base, API_KEY).probe().await.unwrap_err();
        assert!(matches!(&err, AsrError::Network(m) if !m.contains("127.0.0.1")), "{err:?}");
    }

    /// A live session: audio of any length reframed into 100 ms packets, the rest in the last
    /// one; the parameters of 二遍识别; partials while the utterance is not definite, each
    /// definite one once as a sentence with its times, `Finished` after the last result.
    #[tokio::test]
    async fn a_live_session_frames_its_audio_and_passes_on_partials_and_sentences() {
        let speaking =
            |text: &str| json!({ "result": { "text": text, "utterances": [{ "text": text, "definite": false, "start_time": 0, "end_time": 300 }] } });
        let script = Script {
            after: vec![
                (1, vec![response(SEQUENCE, 2, &speaking("你好"), true), response(SEQUENCE, 3, &speaking("你好"), false)]),
                (
                    2,
                    vec![response(
                        SEQUENCE,
                        4,
                        &json!({ "result": { "text": "你好。今天", "utterances": [
                            { "text": "你好。", "definite": true, "start_time": 0, "end_time": 380 },
                            { "text": "今天", "definite": false, "start_time": 500, "end_time": 700 }
                        ] } }),
                        true,
                    )],
                ),
            ],
            after_last: vec![response(
                LAST_RESULT,
                -5,
                &json!({ "result": { "text": "你好。今天天气不错。", "utterances": [
                    { "text": "你好。", "definite": true, "start_time": 0, "end_time": 380 },
                    { "text": "今天天气不错。", "definite": true, "start_time": 500, "end_time": 1200 }
                ] } }),
                true,
            )],
            ..Script::default()
        };
        let (base, seen, mut ended) = serve(script).await;
        let client = client(&base, API_KEY);
        let (result, said) = live(&client, &["Voltip".into(), " Voltip ".into()], &[vec![1; 1000], vec![2; 9000], vec![3; 3000]], true).await;
        assert_eq!(result, Ok(()));
        assert_eq!(said, vec![partial("你好"), sentence("你好。", 0, 380), partial("今天"), sentence("今天天气不错。", 500, 1200), RealtimeEvent::Finished]);
        wait(&mut ended).await;
        let seen = seen.lock().unwrap();
        let connection = &seen[0];
        assert_eq!(connection.path, LIVE_PATH);
        let audio = connection.audio();
        assert_eq!(audio.iter().map(|f| (f.payload.len(), f.flags)).collect::<Vec<_>>(), vec![(3200, 0), (3200, 0), (3200, 0), (3200, 0), (200, 2)]);
        assert!(connection.frames.iter().all(|f| f.well_formed));
        let mut first = vec![1; 1000];
        first.extend(vec![2; 2200]);
        let mut fourth = vec![2; 400];
        fourth.extend(vec![3; 2800]);
        assert_eq!(
            (&audio[0].payload, &audio[1].payload, &audio[2].payload, &audio[3].payload, &audio[4].payload),
            (&first, &vec![2; 3200], &vec![2; 3200], &fourth, &vec![3; 200]),
            "the audio in order, the rest last"
        );
        let parameters = connection.parameters();
        assert_eq!(parameters["audio"], json!({ "format": "pcm", "codec": "raw", "rate": 16000, "bits": 16, "channel": 1 }), "二遍识别 takes no language");
        assert_eq!(
            parameters["request"],
            json!({
                "model_name": "bigmodel", "enable_itn": true, "enable_punc": true, "show_utterances": true, "result_type": "full",
                "enable_nonstream": true, "corpus": { "context": r#"{"hotwords":[{"word":"Voltip"}]}"# }
            })
        );
        assert!(connection.closed);
    }

    /// A session whose audio filled its packets exactly ends with a packet of 100 ms of
    /// silence: the last packet always carries audio.
    #[tokio::test]
    async fn a_session_without_leftover_audio_ends_with_a_short_silence() {
        let script = Script { after_last: vec![response(LAST_RESULT, -3, &json!({ "result": { "text": "" } }), false)], ..Script::default() };
        let (base, seen, mut ended) = serve(script).await;
        let (result, said) = live(&client(&base, API_KEY), &[], &[vec![7; 6400]], true).await;
        assert_eq!((result, said), (Ok(()), vec![RealtimeEvent::Note { text: "豆包语音返回了 1 次结果，均不含文字".into() }, RealtimeEvent::Finished]));
        wait(&mut ended).await;
        let seen = seen.lock().unwrap();
        let audio = seen[0].audio();
        assert_eq!(audio.iter().map(|f| (f.payload.len(), f.flags)).collect::<Vec<_>>(), vec![(3200, 0), (3200, 0), (3200, 2)]);
        assert_eq!(audio[2].payload, vec![0; 3200]);
        assert!(seen[0].parameters()["request"].get("corpus").is_none(), "no glossary, no corpus");
    }

    /// docs/dictation.md §3.15 (2026-11-08): a live session sends a packet of 100 ms, whatever sizes
    /// the audio comes in. About a second fed as the core feeds it, a little over 100 ms at a time,
    /// goes out in 3200-byte packets and the rest in the last one. Recorded takes and the probe keep
    /// their 200 ms packets (the tests of `whole` and `probe`).
    #[tokio::test]
    async fn a_live_session_sends_100_ms_packets_whatever_the_feed() {
        let script = Script { after_last: vec![response(LAST_RESULT, -2, &json!({ "result": { "text": "" } }), false)], ..Script::default() };
        let (base, seen, mut ended) = serve(script).await;
        let feeds: Vec<Vec<u8>> = (1..=10u8).map(|n| vec![n; 3400]).collect();
        let (result, _) = live(&client(&base, API_KEY), &[], &feeds, true).await;
        assert_eq!(result, Ok(()));
        wait(&mut ended).await;
        let seen = seen.lock().unwrap();
        let audio = seen[0].audio();
        let mut want = vec![(3200, 0); 10];
        want.push((2000, LAST_PACKET));
        assert_eq!(audio.iter().map(|f| (f.payload.len(), f.flags)).collect::<Vec<_>>(), want, "100 ms packets, the rest in the last");
        assert_eq!(audio.iter().flat_map(|f| f.payload.clone()).collect::<Vec<_>>(), feeds.concat(), "the audio in order");
    }

    /// A recorded take goes to `bigmodel_nostream` in 200 ms packets at most twice as fast as it
    /// plays (packet n not before n × 100 ms), the last one flagged; its text is the last
    /// answer's, its language hint named.
    #[tokio::test]
    async fn a_whole_take_goes_to_nostream_in_packets_at_twice_the_speed_it_plays() {
        let script = Script {
            after: vec![(0, vec![response(SEQUENCE, 1, &json!({}), true)])],
            after_last: vec![response(
                LAST_RESULT,
                -6,
                &json!({ "audio_info": { "duration": 1000 }, "result": { "text": " 这是字节跳动，今日头条母公司。 ", "utterances": [] } }),
                true,
            )],
            ..Script::default()
        };
        let (base, seen, mut ended) = serve(script).await;
        let take = wav(16_000, 16_000);
        let started = Clock::now();
        let t = client(&base, APP_TOKEN).transcribe(&take, Some("ja-JP"), &["字节跳动".into()]).await.unwrap();
        assert_eq!((t.text.as_str(), t.model.as_str()), ("这是字节跳动，今日头条母公司。", MODEL));
        wait(&mut ended).await;
        let seen = seen.lock().unwrap();
        let connection = &seen[0];
        assert_eq!(connection.path, WHOLE_PATH);
        let audio = connection.audio();
        assert_eq!(audio.iter().map(|f| (f.payload.len(), f.flags)).collect::<Vec<_>>(), vec![(6400, 0), (6400, 0), (6400, 0), (6400, 0), (6400, 2)]);
        assert_eq!(audio.iter().flat_map(|f| f.payload.clone()).collect::<Vec<_>>(), take[44..], "the WAV's PCM, without its header");
        for (n, frame) in audio.iter().enumerate() {
            assert!(frame.at.duration_since(started) >= Duration::from_millis(100) * n as u32, "packet {n} came early");
        }
        let parameters = connection.parameters();
        assert_eq!(parameters["audio"]["language"], "ja-JP");
        assert!(parameters["request"].get("enable_nonstream").is_none());
        assert_eq!(parameters["request"]["corpus"]["context"], r#"{"hotwords":[{"word":"字节跳动"}]}"#);
        assert_eq!((due(0), due(1), due(5)), (Duration::ZERO, Duration::from_millis(100), Duration::from_millis(500)), "2:1");
    }

    /// An error frame ends a session with the error it names, a dropped connection with a
    /// network error.
    #[tokio::test]
    async fn errors_and_dropped_connections_end_a_session_with_their_kind() {
        let script = Script { after: vec![(1, vec![error_frame(55_000_031, r#"{"error":"服务器繁忙"}"#)])], ..Script::default() };
        let (base, _, _) = serve(script).await;
        let (result, said) = live(&client(&base, API_KEY), &[], &[vec![0; 6400], vec![0; 6400]], true).await;
        assert_eq!(result, Err(AsrError::Server { status: 503, body: "55000031: 服务器繁忙".into() }));
        assert!(!said.contains(&RealtimeEvent::Finished));
        let (base, _, _) = serve(Script { drop_after: Some(1), ..Script::default() }).await;
        let (result, said) = live(&client(&base, API_KEY), &[], &[vec![0; 6400], vec![0; 6400]], true).await;
        assert!(matches!(result, Err(AsrError::Network(_))), "{result:?}");
        assert!(said.is_empty());
        // A whole take reports the service's reason.
        let script = Script { after_last: vec![error_frame(45_000_151, r#"{"error":"invalid audio format"}"#)], ..Script::default() };
        let (base, _, _) = serve(script).await;
        let err = client(&base, API_KEY).transcribe(&wav(1600, 16_000), None, &[]).await.unwrap_err();
        assert_eq!(err, AsrError::Audio("45000151: invalid audio format".into()));
    }

    /// The doc's 错误码 table.
    #[test]
    fn error_codes_are_sorted() {
        assert_eq!(frame_error(45_000_001, "invalid params"), AsrError::Service { code: "45000001".into(), message: "invalid params".into() });
        assert_eq!(frame_error(45_000_002, ""), AsrError::Service { code: "45000002".into(), message: String::new() });
        assert_eq!(frame_error(45_000_081, "wait timeout"), AsrError::Timeout);
        assert_eq!(frame_error(45_000_151, "bad format"), AsrError::Audio("45000151: bad format".into()));
        assert_eq!(frame_error(55_000_031, ""), AsrError::Server { status: 503, body: "55000031".into() });
        assert_eq!(frame_error(55_000_123, "internal"), AsrError::Server { status: 500, body: "55000123: internal".into() });
        assert_eq!(frame_error(45_000_999, "余额不足"), AsrError::QuotaExhausted { code: "45000999".into(), message: "余额不足".into(), retry_at_ms: None });
        assert!(frame_error(55_000_031, "").is_retryable());
        assert_eq!(refused(AsrError::Timeout), AsrError::Timeout);
        assert_eq!(
            refused(AsrError::Server { status: 503, body: "account in arrears".into() }),
            AsrError::QuotaExhausted { code: "503".into(), message: "account in arrears".into(), retry_at_ms: None }
        );
        assert_eq!(refused(AsrError::Unauthorized { message: "no".into() }), AsrError::Unauthorized { message: "no".into() });
    }

    /// A cancelled take: the input closes before `Finish`; the connection closes at once without
    /// the last packet, and the session says nothing more.
    #[tokio::test]
    async fn a_cancelled_session_closes_without_its_last_packet() {
        let (base, seen, mut ended) = serve(Script::default()).await;
        let (result, said) = live(&client(&base, API_KEY), &[], &[vec![0; 7000]], false).await;
        assert_eq!((result, said), (Ok(()), vec![]));
        wait(&mut ended).await;
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].audio().iter().map(|f| (f.payload.len(), f.flags)).collect::<Vec<_>>(), vec![(3200, 0), (3200, 0)], "nothing after the cancel");
        assert!(seen[0].closed);
    }

    /// Real time, not a paused clock: the server is a real socket. A service that never answers
    /// times a whole take out, and a live session once its last packet is out.
    #[tokio::test]
    async fn a_silent_service_times_out() {
        let (base, _, _) = serve(Script::default()).await;
        let err = client_with_timeout(&base, Duration::from_millis(300)).transcribe(&wav(1600, 16_000), None, &[]).await.unwrap_err();
        assert_eq!(err, AsrError::Timeout);
        let (result, _) = live(&client_with_timeout(&base, Duration::from_millis(300)), &[], &[vec![0; 3200]], true).await;
        assert_eq!(result, Err(AsrError::Timeout));
    }

    /// docs/dictation.md §3.15: the glossary as hot words, the JSON string of
    /// `{"hotwords":[{"word":…}]}`; a live session takes 100 characters of them in all, a
    /// recorded take 5000 words; blank and repeated terms go nowhere.
    #[test]
    fn the_glossary_goes_out_as_hot_words_within_their_limits() {
        let words = |context: Option<String>| -> Vec<String> {
            context.map_or_else(Vec::new, |c| {
                let value: Value = serde_json::from_str(&c).unwrap();
                value["hotwords"].as_array().unwrap().iter().map(|w| w["word"].as_str().unwrap().to_owned()).collect()
            })
        };
        assert_eq!(hotwords(&[], true), None);
        assert_eq!(hotwords(&["  ".into()], false), None);
        assert_eq!(words(hotwords(&[" Voltip ".into(), "Voltip".into(), "豆包".into()], true)), ["Voltip", "豆包"]);
        // 30 terms of 5 characters: 20 fit a live session's 100.
        let many: Vec<String> = (0..30).map(|i| format!("术语{i:03}")).collect();
        assert_eq!(words(hotwords(&many, true)).len(), 20);
        assert_eq!(words(hotwords(&many, false)).len(), 30);
        // The terms that fit go, in order.
        let sized = ["长".repeat(80), "中".repeat(30), "短".repeat(20)];
        assert_eq!(words(hotwords(&sized, true)), [sized[0].clone(), sized[2].clone()]);
        // A recorded take: the first 5000.
        let lots: Vec<String> = (0..6000).map(|i| format!("w{i}")).collect();
        let sent = words(hotwords(&lots, false));
        assert_eq!((sent.len(), sent[4999].as_str()), (5000, "w4999"));
        // A term no session could take is no term.
        let long = vec!["字".repeat(101)];
        let client = VolcengineAsr::new(AsrConfig::new("https://openspeech.bytedance.com", MODEL).with_token(Some(API_KEY.into()))).unwrap();
        assert!(!client.sends_glossary(&long) && hotwords(&long, false).is_none());
        assert!(client.sends_glossary(&["Voltip".into()]));
        assert!(!client.sends_glossary(&[]) && !client.sends_glossary(&[" ".into()]));
        // Agreeing with what the requests carry.
        assert_eq!(client.parameters(true, None, &["Voltip".into()])["request"]["corpus"]["context"], r#"{"hotwords":[{"word":"Voltip"}]}"#);
        assert!(client.parameters(false, None, &long)["request"].get("corpus").is_none());
    }

    /// What a live session's responses add: each definite utterance once, in order, with its
    /// times or, without them, the previous end and the audio sent; the last result finishes the
    /// utterance still being spoken; a response without utterances is its text.
    #[test]
    fn utterances_are_passed_on_once_with_their_times() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut drain = || std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
        let mut utterances = Utterances::default();
        let utterance = |text: &str, definite: bool| json!({ "text": text, "definite": definite });
        utterances.read(&json!({ "result": { "utterances": [utterance("一", true), utterance("二", false)] } }), false, 500, &tx);
        assert_eq!(drain(), vec![sentence("一", 0, 500), partial("二")], "no times: from the start to the audio sent");
        utterances.read(&json!({ "result": { "utterances": [utterance("一", true), utterance("二", false)] } }), false, 700, &tx);
        assert_eq!(drain(), vec![], "nothing new");
        utterances.read(&json!({ "result": { "utterances": [utterance("一", true), utterance("二。", true), utterance("", false)] } }), false, 900, &tx);
        assert_eq!(drain(), vec![sentence("二。", 500, 900)], "from the last end");
        utterances.read(&json!({ "result": { "utterances": [utterance("一", true), utterance("二。", true), utterance("三", false)] } }), true, 1000, &tx);
        assert_eq!(drain(), vec![sentence("三", 900, 1000)], "the last result finishes the tail");
        let mut bare = Utterances::default();
        bare.read(&json!({ "result": { "text": "你好" } }), false, 300, &tx);
        bare.read(&json!({ "result": { "text": "你好世界" } }), true, 600, &tx);
        bare.read(&json!(null), false, 600, &tx);
        assert_eq!(drain(), vec![partial("你好"), sentence("你好世界", 0, 600)]);
        assert_eq!(whole_text(&json!({ "result": { "utterances": [{ "text": "Hello." }, { "text": "World." }] } })), "Hello. World.");
        assert_eq!(whole_text(&json!({})), "");
    }

    /// The language hints the doc lists go to a recorded take; Chinese, English and Cantonese are
    /// the default model's.
    #[test]
    fn language_hints_name_the_doc_s_codes() {
        assert_eq!(language_code(Some("ja")), Some("ja-JP"));
        assert_eq!(language_code(Some("ko-KR")), Some("ko-KR"));
        assert_eq!(language_code(Some(" DE ")), Some("de-DE"));
        assert_eq!(language_code(Some("pt_BR")), Some("pt-BR"));
        for none in [Some("zh"), Some("zh-CN"), Some("en"), Some("yue"), Some("xx"), Some(""), None] {
            assert_eq!(language_code(none), None, "{none:?}");
        }
    }

    /// A client fails on what it cannot use, naming the field, never its value: the address
    /// first, then the credential, the model; another sample rate never connects.
    #[tokio::test]
    async fn configurations_and_audio_it_cannot_use_are_refused() {
        let config = |base: &str, token: Option<&str>| AsrConfig::new(base, MODEL).with_token(token.map(str::to_owned));
        let error = |config: AsrConfig| VolcengineAsr::new(config).err().map(|e| e.to_string()).unwrap_or_default();
        assert!(error(config("ftp://openspeech.bytedance.com", None)).contains("base_url"));
        assert!(error(config("https://openspeech.bytedance.com", None)).contains("the credential is missing"));
        for (token, field) in [
            (r#"{"set":"api_key","api_key":" "}"#, "api_key"),
            (r#"{"set":"app_token","app_id":"1","access_token":""}"#, "access_token"),
            (r#"{"set":"app_token","access_token":"secret-value-1"}"#, "app_id"),
            (r#"{"access_token":"secret-value-1"}"#, "app_id"),
            (r#"{"set":"other","api_key":"secret-value-1"}"#, "set"),
        ] {
            let message = error(config("https://openspeech.bytedance.com", Some(token)));
            assert!(message.contains("the credential is incomplete") && message.contains(field), "{token}: {message}");
            assert!(!message.contains("secret-value"), "{message}");
        }
        assert!(error(config("https://openspeech.bytedance.com", Some(r#"{"set":"api_key","api_key":"a\nb"}"#))).contains("the credential has a character"));
        assert!(error(config("https://openspeech.bytedance.com", Some("not json"))).contains("the credential is incomplete"));
        assert!(error(config("https://openspeech.bytedance.com", Some(API_KEY)).with_timeout(Duration::ZERO)).contains("timeout"));
        // Without its set, the fields say which.
        let bare = VolcengineAsr::new(config("https://openspeech.bytedance.com/", Some(r#"{"api_key":"k"}"#))).unwrap();
        assert_eq!(bare.credential.iter().map(|(n, _)| *n).collect::<Vec<_>>(), ["x-api-key"]);
        assert!(!format!("{:?}", bare.credential).contains("\"k\""), "sensitive headers");
        let trimmed =
            VolcengineAsr::new(AsrConfig::new("https://proxy.example.test/volc/", " volc.bigasr.sauc.duration ").with_token(Some(APP_TOKEN.into()))).unwrap();
        assert_eq!(trimmed.model(), "volc.bigasr.sauc.duration");
        assert_eq!(trimmed.live_url.as_str(), "wss://proxy.example.test/volc/api/v3/sauc/bigmodel_async");
        assert_eq!(trimmed.whole_url.as_str(), "wss://proxy.example.test/volc/api/v3/sauc/bigmodel_nostream");
        assert!(trimmed.streams());
        // 8 kHz is refused before any connection (nobody listens at port 9).
        let err = client("http://127.0.0.1:9", API_KEY).transcribe(&wav(800, 8_000), None, &[]).await.unwrap_err();
        assert!(matches!(&err, AsrError::Audio(m) if m.contains("8000 Hz")), "{err:?}");
        assert_eq!(client("http://127.0.0.1:9", API_KEY).transcribe(&wav(0, 16_000), None, &[]).await.unwrap().text, "", "an empty take sends nothing");
    }

    /// A full server response with an event number after its sequence number (`0b0100`), as the
    /// official demos read one: header, sequence, event, size, payload.
    fn response_with_event(flags: u8, sequence: i32, event: u32, payload: &Value) -> Vec<u8> {
        let body = gz(payload.to_string().as_bytes());
        let mut frame = vec![0x11, 0x90 | flags | 0b0100, 0x11, 0x00];
        frame.extend_from_slice(&sequence.to_be_bytes());
        frame.extend_from_slice(&event.to_be_bytes());
        frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
        frame.extend_from_slice(&body);
        frame
    }

    /// The last result of a session: one finished sentence.
    fn last_result(text: &str) -> Vec<u8> {
        response(
            LAST_RESULT,
            -9,
            &json!({ "result": { "text": text, "utterances": [{ "text": text, "definite": true, "start_time": 0, "end_time": 300 }] } }),
            true,
        )
    }

    /// regression: the official demos read an event number after the sequence number when the
    /// flags carry `0b0100`; such a response was read as its size and failed the session.
    #[tokio::test]
    async fn regression_a_response_with_an_event_number_is_read() {
        let speaking = json!({ "result": { "text": "你好", "utterances": [{ "text": "你好", "definite": false }] } });
        let script =
            Script {
                after: vec![(1, vec![response_with_event(SEQUENCE, 2, 451, &speaking)])], after_last: vec![last_result("你好。")], ..Script::default()
            };
        let (base, _, _) = serve(script).await;
        let (result, said) = live(&client(&base, API_KEY), &[], &[vec![1; 6400], vec![2; 6400]], true).await;
        assert_eq!((result, said), (Ok(()), vec![partial("你好"), sentence("你好。", 0, 300), RealtimeEvent::Finished]));
    }

    /// regression: the doc gives `result` the type list; a result listed as an array was read as
    /// no text at all, and the take got no preview.
    #[tokio::test]
    async fn regression_a_result_listed_as_an_array_is_read() {
        let listed = |text: &str, definite: bool| json!({ "result": [{ "text": text, "utterances": [{ "text": text, "definite": definite, "start_time": 0, "end_time": 300 }] }] });
        let script = Script {
            after: vec![(1, vec![response(SEQUENCE, 2, &listed("你好", false), true)])],
            after_last: vec![response(LAST_RESULT, -3, &listed("你好。", true), true)],
            ..Script::default()
        };
        let (base, _, _) = serve(script).await;
        let (result, said) = live(&client(&base, API_KEY), &[], &[vec![1; 6400], vec![2; 6400]], true).await;
        assert_eq!((result, said), (Ok(()), vec![partial("你好"), sentence("你好。", 0, 300), RealtimeEvent::Finished]));
    }

    /// regression: a response whose `utterances` are empty while `result.text` holds the text (the
    /// other clients take the text) gave no preview and an empty last result.
    #[tokio::test]
    async fn regression_text_without_utterances_is_what_is_being_said() {
        let bare = |text: &str| json!({ "result": { "text": text, "utterances": [] } });
        let script = Script {
            after: vec![(1, vec![response(SEQUENCE, 2, &bare("你好"), true)])],
            after_last: vec![response(LAST_RESULT, -3, &bare("你好世界"), true)],
            ..Script::default()
        };
        let (base, _, _) = serve(script).await;
        let (result, said) = live(&client(&base, API_KEY), &[], &[vec![1; 6400], vec![2; 6400]], true).await;
        assert_eq!(result, Ok(()));
        assert_eq!(said, vec![partial("你好"), sentence("你好世界", 0, 400), RealtimeEvent::Finished]);
    }

    /// regression: a full server response that carries the service's error in its JSON (`code`
    /// and `message`) was read as a result without text; the session ended well with nothing, and
    /// the history said only that the live preview got no text.
    #[tokio::test]
    async fn regression_an_error_inside_a_response_ends_the_session_with_it() {
        let script = Script {
            after: vec![(1, vec![response(SEQUENCE, 2, &json!({ "code": 45_000_001, "message": "invalid hotwords" }), true)])],
            after_last: vec![last_result("你好。")],
            ..Script::default()
        };
        let (base, _, _) = serve(script).await;
        let (result, said) = live(&client(&base, API_KEY), &[], &[vec![1; 6400], vec![2; 6400]], true).await;
        assert_eq!(result, Err(AsrError::Service { code: "45000001".into(), message: "invalid hotwords".into() }));
        assert!(!said.contains(&RealtimeEvent::Finished));
        // The service's success codes and its "no speech" are no errors.
        for code in [0, 1000, 1013, 20_000_000, 20_000_003] {
            let script = Script {
                after: vec![(1, vec![response(SEQUENCE, 2, &json!({ "code": code, "message": "ok", "result": { "text": "" } }), true)])],
                after_last: vec![last_result("好。")],
                ..Script::default()
            };
            let (base, _, _) = serve(script).await;
            let (result, said) = live(&client(&base, API_KEY), &[], &[vec![1; 6400], vec![2; 6400]], true).await;
            assert_eq!((result, said), (Ok(()), vec![sentence("好。", 0, 300), RealtimeEvent::Finished]), "code {code}");
        }
    }

    /// regression: an acknowledgement frame (message type `0b1011`, which other clients skip)
    /// failed the session as an unexpected message type.
    #[tokio::test]
    async fn regression_an_acknowledgement_frame_is_skipped() {
        let mut ack = vec![0x11, 0xB1, 0x00, 0x00];
        ack.extend_from_slice(&2i32.to_be_bytes());
        ack.extend_from_slice(&0u32.to_be_bytes());
        let script = Script { after: vec![(1, vec![ack])], after_last: vec![last_result("你好。")], ..Script::default() };
        let (base, _, _) = serve(script).await;
        let (result, said) = live(&client(&base, API_KEY), &[], &[vec![1; 6400], vec![2; 6400]], true).await;
        assert_eq!((result, said), (Ok(()), vec![sentence("你好。", 0, 300), RealtimeEvent::Finished]));
    }

    /// regression: the official demos take the `0b0010` bit as the last result, with or without a
    /// sequence number; a last result flagged `0b0010` alone left the session waiting until it
    /// timed out.
    #[tokio::test]
    async fn regression_the_last_flag_alone_ends_the_session() {
        let payload = json!({ "result": { "text": "你好。", "utterances": [{ "text": "你好。", "definite": true, "start_time": 0, "end_time": 300 }] } });
        let body = gz(payload.to_string().as_bytes());
        let mut last = vec![0x11, 0x92, 0x11, 0x00];
        last.extend_from_slice(&(body.len() as u32).to_be_bytes());
        last.extend_from_slice(&body);
        let (base, _, _) = serve(Script { after_last: vec![last], ..Script::default() }).await;
        let (result, said) = live(&client_with_timeout(&base, Duration::from_secs(5)), &[], &[vec![1; 6400]], true).await;
        assert_eq!((result, said), (Ok(()), vec![sentence("你好。", 0, 300), RealtimeEvent::Finished]));
    }

    /// A session that ends without any text says what the service answered — how often it replied
    /// and the log id of its handshake — so the take's history can tell a silent service from a
    /// silent take (docs/dictation.md §11.9).
    #[tokio::test]
    async fn regression_a_session_without_text_says_what_the_service_answered() {
        let script = Script {
            logid: Some("20261105140442ABCDEF"),
            after: vec![
                (0, vec![response(NO_SEQUENCE, 0, &json!({ "result": { "additions": { "log_id": "x" } } }), true)]),
                (1, vec![response(SEQUENCE, 2, &json!({ "audio_info": { "duration": 200 } }), true)]),
            ],
            after_last: vec![response(LAST_RESULT, -3, &json!({ "audio_info": { "duration": 400 }, "result": { "text": "" } }), true)],
            ..Script::default()
        };
        let (base, _, _) = serve(script).await;
        let (result, said) = live(&client(&base, API_KEY), &[], &[vec![1; 6400], vec![2; 6400]], true).await;
        assert_eq!(result, Ok(()));
        assert_eq!(
            said,
            vec![RealtimeEvent::Note { text: "豆包语音返回了 3 次结果，均不含文字；日志 ID 20261105140442ABCDEF".into() }, RealtimeEvent::Finished]
        );
        // Without a log id, the replies alone.
        let script = Script { after_last: vec![response(LAST_RESULT, -2, &json!({ "result": { "text": "" } }), true)], ..Script::default() };
        let (base, _, _) = serve(script).await;
        let (_, said) = live(&client(&base, API_KEY), &[], &[vec![1; 6400]], true).await;
        assert_eq!(said, vec![RealtimeEvent::Note { text: "豆包语音返回了 1 次结果，均不含文字".into() }, RealtimeEvent::Finished]);
    }
}
