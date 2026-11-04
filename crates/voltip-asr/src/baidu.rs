//! Baidu AI Cloud realtime recognition (docs/dictation.md §3.15): `/realtime_asr`, a START frame
//! with the AppID and API Key, binary audio, FINISH; recorded takes go the same way. The model is
//! the `dev_pid`.
//!
//! The URL carries only a request id (`sn`, a UUID); the credential goes in the START frame, with
//! the `dev_pid` and the audio's format. The audio follows in binary frames of 160 ms (each 20 to
//! 200 ms but the last). The service answers with the sentence being spoken (`MID_TEXT`) and each
//! finished one (`FIN_TEXT`, with its start and end in milliseconds of the audio), sends a
//! `HEARTBEAT` every 5 seconds, and closes the connection once it has answered FINISH: that close
//! is the end of the request. An error comes as a `FIN_TEXT` with an `err_no`: `-3005` (audio
//! that holds no speech) and `-3003` concern one sentence and the others go on; `-3004` (the
//! credential) and the others end the request. A cancelled take sends CANCEL, after which the
//! service closes at once. A recorded take is sent as fast as the connection takes it: the
//! service allows that when audio is sent again after a dropped connection.

use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{self, Message};
use url::Url;

use crate::client::{Transcript, truncate_chars};
use crate::config::{AsrConfig, MAX_ERROR_BODY_CHARS};
use crate::credentials;
use crate::dashscope::wav_format;
use crate::error::AsrError;
use crate::realtime::{self, BYTES_PER_MS, Pace, Socket};
use crate::vendor::{self, RealtimeEvent, RealtimeEvents, RealtimeInput, VendorAsr};

/// Bytes of one audio frame: 160 ms of 16 kHz 16-bit mono, the size the service suggests.
const FRAME_BYTES: usize = 5120;
/// A recorded take goes out as fast as the connection takes it (docs/dictation.md §3.15).
const WHOLE_TAKE_PACE: Pace = Pace::Burst;
/// The Chinese dialects model, which wants a `user` in the START frame.
const DIALECTS: u64 = 15376;
/// The device id (`cuid`) and user the START frame names: the service counts them, nothing else.
const CUID: &str = "voltip";
/// Errors that concern one sentence: invalid audio (no speech in it) and a backend error.
const SENTENCE_ERRORS: [i64; 2] = [-3005, -3003];
/// The end of the audio: the service answers the last sentence and closes the connection.
const FINISH: &str = r#"{"type":"FINISH"}"#;
/// A cancelled take: the service drops the request and closes the connection at once.
const CANCEL: &str = r#"{"type":"CANCEL"}"#;
/// How long a failed send looks for the answer the service sent before the connection went.
const LAST_WORDS: Duration = Duration::from_secs(1);

/// The credential: the app's AppID and API Key, both of which go in the START frame.
#[derive(Deserialize)]
struct BaiduCredential {
    app_id: String,
    api_key: String,
}

/// Baidu realtime recognition: one request per take.
pub(crate) struct BaiduAsr {
    config: AsrConfig,
    /// The `dev_pid` as the settings hold it: the receipt's model.
    model: String,
    /// `wss://<host>[/prefix]/realtime_asr`, without the request id.
    endpoint: Url,
    /// `appid`: a number in the START frame.
    app_id: u64,
    api_key: String,
    /// The model: 15372 Mandarin with full punctuation, 1537 Mandarin, 15376 Chinese dialects,
    /// or any other the user names.
    dev_pid: u64,
}

impl fmt::Debug for BaiduAsr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BaiduAsr").field("endpoint", &self.endpoint.as_str()).field("model", &self.model).finish_non_exhaustive()
    }
}

impl BaiduAsr {
    /// The client of `config`: the stored base URL, the `dev_pid` as the model, the credential
    /// JSON with `app_id` and `api_key`. Both the app id and the model are numbers in the START
    /// frame, so anything else is refused here.
    pub(crate) fn new(config: AsrConfig) -> Result<Self, AsrError> {
        let endpoint = vendor::endpoint(&config.base_url, true, "/realtime_asr")?;
        let credential: BaiduCredential = credentials::parse(config.token.as_deref())?;
        let app_id = credentials::require(&credential.app_id, "app_id")?
            .parse::<u64>()
            .map_err(|_| AsrError::InvalidConfig("the credential is incomplete: app_id is not a number".into()))?;
        let api_key = credentials::require(&credential.api_key, "api_key")?;
        let model = config.model.trim().to_owned();
        let dev_pid = model.parse::<u64>().map_err(|_| AsrError::InvalidConfig(format!("model {model:?} is not a dev_pid")))?;
        Ok(Self { config, model, endpoint, app_id, api_key, dev_pid })
    }

    /// The START frame: the credential, the model and the audio's format.
    fn start_frame(&self) -> String {
        let mut data = json!({
            "appid": self.app_id,
            "appkey": self.api_key,
            "dev_pid": self.dev_pid,
            "cuid": CUID,
            "format": "pcm",
            "sample": realtime::SAMPLE_RATE,
        });
        if self.dev_pid == DIALECTS
            && let Some(fields) = data.as_object_mut()
        {
            fields.insert("user".into(), json!(CUID));
        }
        json!({ "type": "START", "data": data }).to_string()
    }

    /// A request: the connection with a new request id, and its START frame.
    async fn open(&self) -> Result<Socket, AsrError> {
        let mut url = self.endpoint.clone();
        url.query_pairs_mut().append_pair("sn", &uuid::Uuid::new_v4().to_string());
        let (mut socket, _) = realtime::connect(realtime::request(&url)?, &self.config.network).await?;
        if let Err(error) = socket.send(Message::text(self.start_frame())).await {
            return Err(said_before(&mut socket, error).await);
        }
        Ok(socket)
    }
}

#[async_trait]
impl VendorAsr for BaiduAsr {
    fn model(&self) -> &str {
        &self.model
    }

    /// The service takes no hot words from the request.
    fn sends_glossary(&self, _glossary: &[String]) -> bool {
        false
    }

    /// A recorded take on one request, as fast as the connection takes it (docs/dictation.md
    /// §3.15).
    async fn transcribe(&self, wav: &[u8], language: Option<&str>, glossary: &[String]) -> Result<Transcript, AsrError> {
        let pcm = pcm_of(wav)?;
        if pcm.is_empty() {
            return Ok(Transcript { text: String::new(), latency_ms: 0, model: self.model.clone() });
        }
        realtime::transcribe_whole(self, pcm, language, glossary, FRAME_BYTES, WHOLE_TAKE_PACE, self.config.timeout).await
    }

    /// START, one frame of silence and FINISH: the service closes the connection once it has
    /// answered, which is success unless it refused the credential or the request on the way. The
    /// silence's "invalid audio" (`-3005`) concerns its sentence only.
    async fn probe(&self) -> Result<(), AsrError> {
        let check = async {
            let mut socket = self.open().await?;
            for message in [Message::binary(vec![0; FRAME_BYTES]), Message::text(FINISH)] {
                if let Err(error) = socket.send(message).await {
                    return Err(said_before(&mut socket, error).await);
                }
            }
            loop {
                match socket.next().await {
                    Some(Ok(Message::Text(text))) => {
                        parse_baidu(text.as_str())?;
                    }
                    Some(Ok(Message::Close(_))) | None => return Ok(()),
                    Some(Ok(_)) => {}
                    Some(Err(error)) => return Err(realtime::map_ws(error)),
                }
            }
        };
        tokio::time::timeout(self.config.timeout, check).await.map_err(|_| AsrError::Timeout)?
    }

    fn streams(&self) -> bool {
        true
    }

    /// One request: binary audio in 5120-byte frames, FINISH on [`RealtimeInput::Finish`], and the
    /// service's sentences until it closes the connection. A take given up (its input closed)
    /// sends CANCEL; after FINISH the request lasts as long as the caller waits. However the
    /// request ends, the connection closes.
    async fn realtime(
        &self,
        _language: Option<&str>,
        _glossary: &[String],
        mut input: mpsc::UnboundedReceiver<RealtimeInput>,
        events: RealtimeEvents,
    ) -> Result<(), AsrError> {
        let mut socket = self.open().await?;
        let result = run(&mut socket, &mut input, &events).await;
        let _ = socket.close(None).await;
        result
    }
}

/// A request until everything was said ([`RealtimeEvent::Finished`]), the take was given up (its
/// input closed: CANCEL) or it failed.
async fn run(socket: &mut Socket, input: &mut mpsc::UnboundedReceiver<RealtimeInput>, events: &RealtimeEvents) -> Result<(), AsrError> {
    let (mut pending, mut sent, mut last_end_ms, mut finished) = (Vec::<u8>::new(), 0usize, 0u64, false);
    loop {
        tokio::select! {
            // What the service already said goes first: a refusal ends the request.
            biased;
            message = socket.next() => match message {
                Some(Ok(Message::Text(text))) => match parse_baidu(text.as_str())? {
                    BaiduReply::Partial(text) => {
                        let _ = events.send(RealtimeEvent::Partial { text });
                    }
                    BaiduReply::Sentence { text, begin_ms, end_ms } => {
                        // Without the service's times, the sentence runs from the last one's end to
                        // the audio sent so far.
                        let begin_ms = begin_ms.unwrap_or(last_end_ms);
                        let end_ms = end_ms.filter(|&end| end > begin_ms).unwrap_or_else(|| ms_of(sent).max(begin_ms));
                        last_end_ms = end_ms;
                        let _ = events.send(RealtimeEvent::Sentence { text, begin_ms, end_ms });
                    }
                    BaiduReply::Nothing => {}
                },
                // The request ends when the service closes the connection after FINISH. A
                // connection that just drops is not that close, whenever it drops.
                Some(Ok(Message::Close(_))) | None if finished => {
                    let _ = events.send(RealtimeEvent::Finished);
                    return Ok(());
                }
                Some(Ok(Message::Close(_))) | None => return Err(realtime::closed_early()),
                Some(Ok(_)) => {}
                Some(Err(error)) => return Err(realtime::map_ws(error)),
            },
            item = input.recv() => match item {
                Some(RealtimeInput::Audio(pcm)) if !finished => {
                    pending.extend_from_slice(&pcm);
                    sent += send_audio(socket, &mut pending, false).await?;
                }
                Some(RealtimeInput::Finish) if !finished => {
                    finished = true;
                    sent += send_audio(socket, &mut pending, true).await?;
                    if let Err(error) = socket.send(Message::text(FINISH)).await {
                        return Err(said_before(socket, error).await);
                    }
                }
                Some(_) => {}
                None => {
                    let _ = socket.send(Message::text(CANCEL)).await;
                    return Ok(());
                }
            },
        }
    }
}

/// Send `pending`'s whole frames, and with `flush` the rest as the last (shorter) one; the bytes
/// sent.
async fn send_audio(socket: &mut Socket, pending: &mut Vec<u8>, flush: bool) -> Result<usize, AsrError> {
    let mut at = 0;
    while pending.len() - at >= FRAME_BYTES || (flush && pending.len() > at) {
        let end = (at + FRAME_BYTES).min(pending.len());
        if let Err(error) = socket.send(Message::binary(pending[at..end].to_vec())).await {
            return Err(said_before(socket, error).await);
        }
        at = end;
    }
    pending.drain(..at);
    Ok(at)
}

/// One message of the service.
#[derive(Debug, PartialEq, Eq)]
enum BaiduReply {
    /// The sentence being spoken, whole so far (`MID_TEXT`).
    Partial(String),
    /// A finished sentence (`FIN_TEXT`), with its times when the service gave them.
    Sentence { text: String, begin_ms: Option<u64>, end_ms: Option<u64> },
    /// Nothing to tell: a heartbeat, an empty result, an error that concerns one sentence.
    Nothing,
}

/// A message of the service; an `err_no` that concerns the request is the error it names.
fn parse_baidu(text: &str) -> Result<BaiduReply, AsrError> {
    let value: Value = serde_json::from_str(text)
        .map_err(|e| AsrError::BadResponse(format!("realtime recognition answer: {e}: {}", truncate_chars(text, MAX_ERROR_BODY_CHARS))))?;
    let err_no = scalar(value.get("err_no")).parse::<i64>().unwrap_or(0);
    if err_no != 0 {
        let err_msg = scalar(value.get("err_msg"));
        if SENTENCE_ERRORS.contains(&err_no) {
            tracing::debug!(err_no, "the realtime recognition skipped a sentence");
            return Ok(BaiduReply::Nothing);
        }
        return Err(baidu_error(err_no, &err_msg));
    }
    let result = value.get("result").and_then(Value::as_str).map(str::trim).unwrap_or_default();
    Ok(match value.get("type").and_then(Value::as_str) {
        Some("MID_TEXT") if !result.is_empty() => BaiduReply::Partial(result.to_owned()),
        Some("FIN_TEXT") if !result.is_empty() => {
            BaiduReply::Sentence { text: result.to_owned(), begin_ms: number(value.get("start_time")), end_ms: number(value.get("end_time")) }
        }
        _ => BaiduReply::Nothing,
    })
}

/// A request-level `err_no` as the error it is.
fn baidu_error(err_no: i64, err_msg: &str) -> AsrError {
    let said = || voltip_reply::clean(&format!("{err_no}: {err_msg}"));
    match err_no {
        // The AppID and API Key were refused; the service closes the connection.
        -3004 => AsrError::Unauthorized { message: said() },
        // The service could not read the audio from the connection.
        -3001 => AsrError::Network(said()),
        _ => AsrError::Service { code: err_no.to_string(), message: voltip_reply::clean(err_msg) },
    }
}

/// A send failed because the connection is going: the error the service answered before it went
/// explains it better (a refused credential), else the send's own error.
async fn said_before(socket: &mut Socket, failed: tungstenite::Error) -> AsrError {
    let reason = async {
        while let Some(Ok(message)) = socket.next().await {
            if let Message::Text(text) = message
                && let Err(error) = parse_baidu(text.as_str())
            {
                return Some(error);
            }
        }
        None
    };
    match tokio::time::timeout(LAST_WORDS, reason).await {
        Ok(Some(error)) => error,
        _ => realtime::map_ws(failed),
    }
}

/// The 16 kHz PCM of a recorded take (whole samples only); another rate is refused.
fn pcm_of(wav: &[u8]) -> Result<&[u8], AsrError> {
    let format = wav_format(wav)?;
    if format.sample_rate != realtime::SAMPLE_RATE {
        return Err(AsrError::Audio(format!("{} Hz audio: the service takes 16 kHz", format.sample_rate)));
    }
    let pcm = &wav[format.data.0..format.data.1];
    Ok(&pcm[..pcm.len() - pcm.len() % 2])
}

/// A JSON scalar as text: `"-3004"` and `-3004` alike; empty for anything else.
fn scalar(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.trim().to_owned(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// A JSON number, or a number in a string.
fn number(value: Option<&Value>) -> Option<u64> {
    match value? {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Milliseconds of `bytes` of the PCM.
fn ms_of(bytes: usize) -> u64 {
    u64::try_from(bytes / BYTES_PER_MS).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tokio::net::{TcpListener, TcpStream};

    use super::*;
    use crate::dashscope::tests::wav;
    use crate::iflytek::tests::{Seen, accept, eventually, live, partials, pcm, read_on, sentences, serve};

    /// A test credential: no service knows it.
    const TOKEN: &str = r#"{"set":"default","app_id":"12345678","api_key":"baidu-key-test"}"#;
    /// The sentence a scripted service hears.
    const SENTENCE: &str = "北京天气怎么样";

    /// What a scripted service does.
    #[derive(Clone, Copy)]
    enum Baidu {
        /// Send a heartbeat at START; answer each audio frame with the sentence so far
        /// (`MID_TEXT`), the first also with a sentence's `-3005`; answer FINISH with the
        /// sentence (`FIN_TEXT`, from 20 ms to the audio's end), then close.
        Recognises,
        /// Answer START with this error, as the service refuses a credential, then read on.
        Refuses(i64, &'static str),
        /// Answer the first audio frame with this error, then read on.
        Fails(i64, &'static str),
        /// Refuse the WebSocket handshake with this status and body.
        RefusesHandshake(u16, &'static str),
        /// Drop the connection at the first audio frame.
        Drops,
        /// Say nothing.
        Silent,
    }

    fn reply(kind: &str, result: &str, times: Option<(u64, u64)>) -> Message {
        let mut value = json!({ "err_no": 0, "err_msg": "OK", "type": kind, "result": result, "log_id": "45677785", "sn": "test_ws_1" });
        if let Some((start, end)) = times {
            value["start_time"] = json!(start);
            value["end_time"] = json!(end);
            value["product_id"] = json!(15372);
            value["product_line"] = json!("open");
        }
        Message::text(value.to_string())
    }

    fn failure(err_no: i64, err_msg: &str) -> Message {
        Message::text(json!({ "err_msg": err_msg, "err_no": err_no, "log_id": "106549729", "result": "", "sn": "test_ws_1", "type": "FIN_TEXT" }).to_string())
    }

    async fn serve_baidu(script: Baidu) -> (String, Arc<Mutex<Seen>>) {
        serve(move |tcp, index, seen| session(tcp, index, script, seen)).await
    }

    async fn session(tcp: TcpStream, index: usize, script: Baidu, seen: Arc<Mutex<Seen>>) {
        let refuse = if let Baidu::RefusesHandshake(status, body) = script { Some((status, body)) } else { None };
        let Some(mut socket) = accept(tcp, index, refuse, &seen).await else { return };
        let (mut frames, mut audio) = (0usize, 0usize);
        while let Some(Ok(message)) = socket.next().await {
            if let Message::Close(frame) = message {
                seen.lock().unwrap().close(index, frame.as_ref());
                return;
            }
            seen.lock().unwrap().frames[index].push(message.clone());
            match (message, script) {
                (Message::Text(text), _) => {
                    let kind = serde_json::from_str::<Value>(text.as_str()).unwrap()["type"].as_str().unwrap().to_owned();
                    match (kind.as_str(), script) {
                        ("START", Baidu::Refuses(err_no, err_msg)) => {
                            socket.send(failure(err_no, err_msg)).await.unwrap();
                            return read_on(&mut socket, index, &seen).await;
                        }
                        ("START", Baidu::Recognises) => socket.send(Message::text(r#"{"type":"HEARTBEAT"}"#)).await.unwrap(),
                        ("FINISH", Baidu::Recognises) => {
                            let end = u64::try_from(audio / BYTES_PER_MS).unwrap();
                            socket.send(reply("FIN_TEXT", SENTENCE, Some((20, end)))).await.unwrap();
                            let _ = socket.close(None).await;
                            return read_on(&mut socket, index, &seen).await;
                        }
                        _ => {}
                    }
                }
                (Message::Binary(bytes), Baidu::Recognises) => {
                    frames += 1;
                    audio += bytes.len();
                    let so_far: String = SENTENCE.chars().take(frames).collect();
                    socket.send(reply("MID_TEXT", &so_far, None)).await.unwrap();
                    if frames == 1 {
                        // A sentence of invalid audio concerns that sentence only.
                        socket.send(failure(-3005, "asr invalid audio")).await.unwrap();
                    }
                }
                (Message::Binary(_), Baidu::Fails(err_no, err_msg)) => {
                    socket.send(failure(err_no, err_msg)).await.unwrap();
                    return read_on(&mut socket, index, &seen).await;
                }
                (Message::Binary(_), Baidu::Drops) => return,
                _ => {}
            }
        }
    }

    fn baidu_at(base: &str, model: &str) -> BaiduAsr {
        BaiduAsr::new(AsrConfig::new(base, model).with_token(Some(TOKEN.into()))).unwrap()
    }

    #[test]
    fn baidu_needs_a_numeric_app_id_and_dev_pid() {
        let with = |model: &str, token: &str| BaiduAsr::new(AsrConfig::new("https://vop.baidu.com", model).with_token(Some(token.into())));
        assert_eq!(
            with("15372", r#"{"app_id":"app-secret-ish","api_key":"k"}"#).unwrap_err(),
            AsrError::InvalidConfig("the credential is incomplete: app_id is not a number".into())
        );
        assert_eq!(with("fast", TOKEN).unwrap_err(), AsrError::InvalidConfig(r#"model "fast" is not a dev_pid"#.into()));
        assert_eq!(with("15372", r#"{"app_id":"1"}"#).unwrap_err(), AsrError::InvalidConfig("the credential is incomplete: missing field `api_key`".into()));
        assert_eq!(
            with("15372", r#"{"app_id":"1","api_key":" "}"#).unwrap_err(),
            AsrError::InvalidConfig("the credential is incomplete: api_key is empty".into())
        );
        assert!(matches!(BaiduAsr::new(AsrConfig::new("https://vop.baidu.com", "15372")), Err(AsrError::InvalidConfig(m)) if m.starts_with("the credential")));
        assert!(
            matches!(BaiduAsr::new(AsrConfig::new("wss://vop.baidu.com", "15372").with_token(Some(TOKEN.into()))), Err(AsrError::InvalidConfig(m)) if m.starts_with("base_url"))
        );
        let client = with(" 1737 ", TOKEN).unwrap();
        assert_eq!((client.model(), client.dev_pid, client.app_id), ("1737", 1737, 12_345_678));
        assert!(!client.sends_glossary(&["术语".into()]) && client.streams());
        assert!(!format!("{client:?}").contains("baidu-key-test"));
        assert!(matches!(futures_util::FutureExt::now_or_never(client.transcribe(&wav(100, 8_000), None, &[])), Some(Err(AsrError::Audio(_)))));
    }

    /// START carries the credential and the model as numbers; the audio goes in 160 ms binary
    /// frames and a shorter last one, then FINISH. The sentence so far and the finished one, with
    /// the service's times, come back; a heartbeat and a sentence's `-3005` are skipped; the
    /// service's close ends the take.
    #[tokio::test]
    async fn baidu_starts_with_the_credential_and_frames_the_audio() {
        let (base, seen) = serve_baidu(Baidu::Recognises).await;
        let audio = pcm(12_000);
        let (result, events) = live(&baidu_at(&base, "15372"), Some("zh"), vec![RealtimeInput::Audio(audio.clone()), RealtimeInput::Finish]).await;
        result.unwrap();
        assert_eq!(
            events,
            vec![
                RealtimeEvent::Partial { text: "北".into() },
                RealtimeEvent::Partial { text: "北京".into() },
                RealtimeEvent::Partial { text: "北京天".into() },
                RealtimeEvent::Sentence { text: SENTENCE.into(), begin_ms: 20, end_ms: 375 },
                RealtimeEvent::Finished,
            ]
        );
        let seen = seen.lock().unwrap();
        let texts = seen.json(0);
        assert_eq!(
            texts,
            [
                json!({ "type": "START", "data": { "appid": 12_345_678, "appkey": "baidu-key-test", "dev_pid": 15372, "cuid": "voltip", "format": "pcm", "sample": 16000 } }),
                json!({ "type": "FINISH" })
            ]
        );
        assert_eq!(seen.binary(0), [audio[..5120].to_vec(), audio[5120..10_240].to_vec(), audio[10_240..].to_vec()]);
        assert!(matches!(seen.frames[0].last(), Some(Message::Text(_))), "FINISH after the audio");
        let url = Url::parse(&format!("ws://h{}", seen.handshakes[0].0)).unwrap();
        assert_eq!(url.path(), "/realtime_asr");
        let query: Vec<(String, String)> = url.query_pairs().into_owned().collect();
        assert_eq!(query.len(), 1);
        assert_eq!(query[0].0, "sn");
        assert!(uuid::Uuid::parse_str(&query[0].1).is_ok(), "a request id: {}", query[0].1);
    }

    /// The dialects model wants a user in START.
    #[tokio::test]
    async fn baidu_names_a_user_for_the_dialects_model() {
        let (base, seen) = serve_baidu(Baidu::Recognises).await;
        let (result, _) = live(&baidu_at(&base, "15376"), None, vec![RealtimeInput::Audio(pcm(5120)), RealtimeInput::Finish]).await;
        result.unwrap();
        let start = seen.lock().unwrap().json(0)[0].clone();
        assert_eq!(start["data"]["dev_pid"], json!(15376));
        assert_eq!(start["data"]["user"], json!("voltip"));
    }

    #[test]
    fn baidu_messages_parse_or_are_skipped() {
        // The documented answers (返回说明), as JSON.
        let partial =
            r#"{"err_no":0,"err_msg":"OK","type":"MID_TEXT","result":"北京天气怎","log_id":"45677785","sn":"399427ce-e999-11e9-94c8-fa163e4e6064_ws_2"}"#;
        assert_eq!(parse_baidu(partial).unwrap(), BaiduReply::Partial("北京天气怎".into()));
        let finished = r#"{"err_no":0,"err_msg":"OK","log_id":"45677785","sn":"399427ce-e999-11e9-94c8-fa163e4e6064_ws_2","type":"FIN_TEXT","result":"北京天气怎么样","start_time":53220,"end_time":73340,"product_id":1537,"product_line":"open"}"#;
        assert_eq!(parse_baidu(finished).unwrap(), BaiduReply::Sentence { text: "北京天气怎么样".into(), begin_ms: Some(53_220), end_ms: Some(73_340) });
        assert_eq!(parse_baidu(r#"{"type":"HEARTBEAT"}"#).unwrap(), BaiduReply::Nothing);
        assert_eq!(parse_baidu(r#"{"err_no":0,"type":"FIN_TEXT","result":" "}"#).unwrap(), BaiduReply::Nothing);
        let refused = r#"{"err_msg":"asr authentication failed","err_no":-3004,"log_id":"106549729","result":"","sn":"0d25b5e1-ffa6-11e9-8968-f496349f3917","type":"FIN_TEXT"}"#;
        assert_eq!(parse_baidu(refused), Err(AsrError::Unauthorized { message: "-3004: asr authentication failed".into() }));
        for sentence_only in [-3005, -3003] {
            let text = json!({ "err_no": sentence_only, "err_msg": "x", "type": "FIN_TEXT", "result": "" }).to_string();
            assert_eq!(parse_baidu(&text).unwrap(), BaiduReply::Nothing, "{sentence_only}");
        }
        assert_eq!(parse_baidu(r#"{"err_no":-3001,"err_msg":"read data failed","type":"FIN_TEXT"}"#), Err(AsrError::Network("-3001: read data failed".into())));
        assert_eq!(
            parse_baidu(r#"{"err_no":"-3002","err_msg":"param error","type":"FIN_TEXT"}"#),
            Err(AsrError::Service { code: "-3002".into(), message: "param error".into() })
        );
        assert!(matches!(parse_baidu("{"), Err(AsrError::BadResponse(_))));
    }

    #[tokio::test]
    async fn baidu_failures_end_the_take_with_their_kind() {
        let take = || vec![RealtimeInput::Audio(pcm(2 * FRAME_BYTES)), RealtimeInput::Finish];
        for (script, want) in [
            (Baidu::Refuses(-3004, "asr authentication failed"), AsrError::Unauthorized { message: "-3004: asr authentication failed".into() }),
            (Baidu::Fails(-3001, "read data failed"), AsrError::Network("-3001: read data failed".into())),
            (Baidu::Fails(-3002, "param error"), AsrError::Service { code: "-3002".into(), message: "param error".into() }),
            (Baidu::RefusesHandshake(403, r#"{"error":"forbidden"}"#), AsrError::Unauthorized { message: "forbidden".into() }),
        ] {
            let (base, _) = serve_baidu(script).await;
            let (result, events) = live(&baidu_at(&base, "15372"), None, take()).await;
            assert_eq!(result, Err(want));
            assert!(!events.contains(&RealtimeEvent::Finished));
        }
        let (base, _) = serve_baidu(Baidu::Drops).await;
        let (result, events) = live(&baidu_at(&base, "15372"), None, take()).await;
        assert!(matches!(&result, Err(AsrError::Network(_))), "{result:?}");
        assert!(!events.contains(&RealtimeEvent::Finished));
    }

    /// 测试连接: START, one frame of silence and FINISH; the service's close is the success, its
    /// silence's `-3005` is not a failure, a refusal on the way is.
    #[tokio::test]
    async fn baidu_probe_succeeds_when_the_service_closes_after_finish() {
        let (base, seen) = serve_baidu(Baidu::Recognises).await;
        baidu_at(&base, "15372").probe().await.unwrap();
        {
            let seen = seen.lock().unwrap();
            assert_eq!(seen.frames[0].len(), 3, "START, one frame, FINISH");
            assert_eq!(seen.binary(0), [vec![0u8; 5120]]);
            assert_eq!(seen.json(0)[1], json!({ "type": "FINISH" }));
        }
        let (base, _) = serve_baidu(Baidu::Refuses(-3004, "asr authentication failed")).await;
        assert_eq!(baidu_at(&base, "15372").probe().await, Err(AsrError::Unauthorized { message: "-3004: asr authentication failed".into() }));
        let (base, _) = serve_baidu(Baidu::Fails(-3001, "read data failed")).await;
        assert_eq!(baidu_at(&base, "15372").probe().await, Err(AsrError::Network("-3001: read data failed".into())));
        let (base, _) = serve_baidu(Baidu::RefusesHandshake(401, r#"{"message":"Unauthorized"}"#)).await;
        assert!(matches!(baidu_at(&base, "15372").probe().await, Err(AsrError::Unauthorized { .. })));
        let (base, _) = serve_baidu(Baidu::Silent).await;
        let silent = BaiduAsr::new(AsrConfig::new(&base, "15372").with_token(Some(TOKEN.into())).with_timeout(Duration::from_millis(300))).unwrap();
        assert_eq!(silent.probe().await, Err(AsrError::Timeout), "a handshake alone is not success");
    }

    /// A take cancelled (its input closed before `Finish`) sends CANCEL instead of FINISH and closes,
    /// without `Finished`.
    #[tokio::test]
    async fn baidu_a_cancelled_take_sends_cancel_and_closes() {
        let (base, seen) = serve_baidu(Baidu::Recognises).await;
        let client = baidu_at(&base, "15372");
        let (tx, rx) = mpsc::unbounded_channel();
        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        let watch = seen.clone();
        let drive = async move {
            tx.send(RealtimeInput::Audio(pcm(2 * FRAME_BYTES))).unwrap();
            eventually("START and two frames", || watch.lock().unwrap().frames.first().is_some_and(|f| f.len() == 3)).await;
            drop(tx);
        };
        let (result, ()) = tokio::join!(client.realtime(None, &[], rx, events_tx), drive);
        result.unwrap();
        eventually("the connection closed", || seen.lock().unwrap().closed == [0]).await;
        let seen = seen.lock().unwrap();
        assert_eq!(seen.json(0).last(), Some(&json!({ "type": "CANCEL" })));
        assert!(!seen.json(0).contains(&json!({ "type": "FINISH" })));
        assert_eq!(seen.binary(0).len(), 2);
        while let Ok(event) = events_rx.try_recv() {
            assert_ne!(event, RealtimeEvent::Finished);
        }
    }

    /// A recorded take goes out as fast as the connection takes it: ten seconds of audio in well
    /// under ten seconds, in 160 ms frames and a shorter last one.
    #[tokio::test]
    async fn baidu_a_whole_take_goes_in_a_burst() {
        let (base, seen) = serve_baidu(Baidu::Recognises).await;
        let started = std::time::Instant::now();
        let transcript = baidu_at(&base, "15372").transcribe(&wav(160_000, 16_000), None, &["术语".into()]).await.unwrap();
        assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
        assert_eq!((transcript.text.as_str(), transcript.model.as_str()), (SENTENCE, "15372"));
        let frames = seen.lock().unwrap().binary(0);
        assert_eq!(frames.len(), 63);
        assert!(frames[..62].iter().all(|f| f.len() == 5120));
        assert_eq!(frames[62].len(), 2560);
    }

    /// A burst's deadline is the client's timeout alone: no time for the audio to play.
    #[tokio::test]
    async fn baidu_a_whole_take_times_out_after_its_deadline() {
        let (base, _) = serve_baidu(Baidu::Silent).await;
        let client = BaiduAsr::new(AsrConfig::new(&base, "15372").with_token(Some(TOKEN.into())).with_timeout(Duration::from_millis(200))).unwrap();
        let started = std::time::Instant::now();
        assert_eq!(client.transcribe(&wav(48_000, 16_000), None, &[]).await, Err(AsrError::Timeout));
        let took = started.elapsed();
        assert!(took >= Duration::from_millis(200) && took < Duration::from_secs(3), "{took:?}");
        // No audio, no request.
        assert_eq!(client.transcribe(&wav(0, 16_000), None, &[]).await.map(|t| t.text), Ok(String::new()));
    }

    /// Times the service leaves out run from the last sentence's end to the audio sent so far.
    #[tokio::test]
    async fn baidu_sentences_without_times_take_the_audio_sent() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let mut audio = 0usize;
            while let Some(Ok(message)) = socket.next().await {
                match message {
                    Message::Binary(bytes) => audio += bytes.len(),
                    Message::Text(text) if text.as_str().contains("FINISH") => {
                        assert_eq!(audio, 6400);
                        socket.send(reply("FIN_TEXT", "第一句。", None)).await.unwrap();
                        socket.send(reply("FIN_TEXT", "第二句。", None)).await.unwrap();
                        let _ = socket.close(None).await;
                    }
                    _ => {}
                }
            }
        });
        let (result, events) = live(&baidu_at(&base, "15372"), None, vec![RealtimeInput::Audio(pcm(6400)), RealtimeInput::Finish]).await;
        result.unwrap();
        assert_eq!(sentences(&events), [("第一句。".to_owned(), 0, 200), ("第二句。".to_owned(), 200, 200)]);
        assert!(partials(&events).is_empty());
    }
}
