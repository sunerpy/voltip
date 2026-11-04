//! iFlytek (docs/dictation.md §3.15): dictation (`iat`, `/v2/iat`, HMAC-SHA256 signed, 60 seconds
//! a session; a live take longer than that continues on a new session) and realtime transcription
//! (`rtasr`, `/v1/ws`, signed with `signa`, no length limit).
//!
//! Both take 16 kHz 16-bit PCM in 40 ms frames of 1280 bytes, at the pace the audio plays: a
//! recorded take goes out 1:1. Neither takes a glossary; dictation's hot words are the app's list
//! in the iFlytek console.
//!
//! Dictation signs its URL with the host and the path it connects to (a proxy's path prefix is
//! part of the request line), then sends the app id and the parameters in the first frame,
//! base64 audio in JSON frames and the end frame `{"data":{"status":2}}`. Its results are revised
//! by the dynamic correction (`wpgs`: a result appends to the earlier ones or replaces a range of
//! them) until the session's last one (`data.status` 2); a session's text is one sentence. The
//! service ends a session at 60 seconds, so a session takes 55 at most: a live take goes on on a
//! new session, a recorded one is cut in its pauses ([`split::quiet_cuts`]), and the sentences'
//! times count from the start of the take across the sessions.
//!
//! Realtime transcription signs its URL with `signa` (the HMAC-SHA1 of the MD5 of the app id and
//! the time), answers `started`, takes binary audio and the binary end marker `{"end": true}`, and
//! sends the sentence being spoken (`type` 1) and the finished ones (`type` 0, with their times)
//! until it closes the connection after the last.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::task::Poll;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, KeyInit as _, Mac as _};
use md5::{Digest as _, Md5};
use serde::Deserialize;
use serde_json::{Value, json};
use sha1::Sha1;
use sha2::Sha256;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{self, Message, Utf8Bytes};
use url::Url;

use crate::client::{Transcript, truncate_chars};
use crate::config::{AsrConfig, MAX_ERROR_BODY_CHARS};
use crate::dashscope::{join_sentences, wav_format};
use crate::error::AsrError;
use crate::realtime::{self, BYTES_PER_MS, Pace, Socket};
use crate::vendor::{self, RealtimeEvent, RealtimeEvents, RealtimeInput, VendorAsr};
use crate::{credentials, split};

/// Bytes of one audio frame: 40 ms of 16 kHz 16-bit mono, what both services ask for.
const FRAME_BYTES: usize = 1280;
/// A recorded take goes out as fast as it plays: both services may misrecognise audio sent faster.
const WHOLE_TAKE_PACE: Pace = Pace::Realtime { speedup: 1 };
/// The audio one dictation session takes before the take goes on on a new one: the service ends
/// a session at 60 seconds (docs/dictation.md §3.15).
const SESSION_MS: u64 = 55_000;
/// `vad_eos`, the silence after which dictation takes the speech as over, at its maximum (10 s), so
/// that a pause does not end the session.
const VAD_EOS_MS: u64 = 10_000;
/// What dictation is told the audio is.
const IAT_FORMAT: &str = "audio/L16;rate=16000";
/// Dictation's end frame: no more audio in this session.
const IAT_END: &str = r#"{"data":{"status":2}}"#;
/// Realtime transcription's end marker, a binary frame.
const RTASR_END: &[u8] = br#"{"end": true}"#;
/// How long a failed send looks for the answer the service sent before the connection went.
const LAST_WORDS: Duration = Duration::from_secs(1);

/// Dictation's credential: the app's AppID, APIKey and APISecret.
#[derive(Deserialize)]
struct IatCredential {
    app_id: String,
    api_key: String,
    api_secret: String,
}

/// iFlytek dictation (语音听写): one session per 55 seconds of a take.
pub(crate) struct IatAsr {
    config: AsrConfig,
    /// The domain (`business.domain`): `iat`, everyday speech.
    model: String,
    /// `wss://<host>[/prefix]/v2/iat`, without the signature.
    endpoint: Url,
    app_id: String,
    api_key: String,
    api_secret: String,
    /// The audio a session takes ([`SESSION_MS`]; shorter in the tests).
    session_ms: u64,
}

impl fmt::Debug for IatAsr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IatAsr").field("endpoint", &self.endpoint.as_str()).field("model", &self.model).finish_non_exhaustive()
    }
}

impl IatAsr {
    /// The client of `config`: the stored base URL, the domain as the model, the credential JSON
    /// with `app_id`, `api_key` and `api_secret`.
    pub(crate) fn new(config: AsrConfig) -> Result<Self, AsrError> {
        let endpoint = vendor::endpoint(&config.base_url, true, "/v2/iat")?;
        let credential: IatCredential = credentials::parse(config.token.as_deref())?;
        let app_id = credentials::require(&credential.app_id, "app_id")?;
        let api_key = credentials::require(&credential.api_key, "api_key")?;
        let api_secret = credentials::require(&credential.api_secret, "api_secret")?;
        let model = config.model.trim().to_owned();
        if model.is_empty() {
            return Err(AsrError::InvalidConfig("model is empty".into()));
        }
        Ok(Self { config, model, endpoint, app_id, api_key, api_secret, session_ms: SESSION_MS })
    }

    /// The endpoint signed for `date` (RFC 1123, GMT): the host and the request line are those of
    /// the address it connects to, a proxy's path prefix included.
    fn signed_url(&self, date: &str) -> Url {
        let host = authority(&self.endpoint);
        let authorization = iat_authorization(&self.api_key, &self.api_secret, &host, date, self.endpoint.path());
        let mut url = self.endpoint.clone();
        url.set_query(Some(&format!("authorization={}&date={}&host={}", component(&authorization), component(date), component(&host))));
        url
    }

    /// A new session's connection, signed now (the service allows 300 seconds of clock skew).
    async fn open(&self) -> Result<Socket, AsrError> {
        let date = chrono::Utc::now().format("%a, %d %b %Y %H:%M:%S GMT").to_string();
        let (socket, _) = realtime::connect(realtime::request(&self.signed_url(&date))?, &self.config.network).await?;
        Ok(socket)
    }

    /// `business` of the first frame: Mandarin with the dynamic correction (Mandarin's only), or
    /// English for an English take; the domain is the model.
    fn business(&self, language: Option<&str>) -> Value {
        let english = language.is_some_and(|l| primary_subtag(l) == "en");
        let mut business = json!({
            "language": if english { "en_us" } else { "zh_cn" },
            "domain": self.model,
            "accent": "mandarin",
            "vad_eos": VAD_EOS_MS,
        });
        if !english && let Some(fields) = business.as_object_mut() {
            fields.insert("dwa".into(), json!("wpgs"));
        }
        business
    }

    /// Send the take's whole frames of audio, and with `flush` what is left of it (a shorter last
    /// frame), to the session that takes audio: the newest, unless it was ended or has carried its
    /// [`IatAsr::session_ms`] of audio, when it is ended and a new session takes the audio on.
    async fn send_audio(&self, take: &mut IatTake, business: &Value, flush: bool) -> Result<(), AsrError> {
        let limit = usize::try_from(self.session_ms).unwrap_or(usize::MAX).saturating_mul(BYTES_PER_MS);
        loop {
            let size = match take.pending.len() {
                n if n >= FRAME_BYTES => FRAME_BYTES,
                n if flush && n > 0 => n,
                _ => return Ok(()),
            };
            // The last bit of a take, shorter than a frame, goes to its session even when that one
            // is full: a new session for it would cost a call for nothing, and 60 seconds is the
            // limit. A whole frame over the limit starts the next session.
            let open = take.sessions.back().is_some_and(|s| !s.ended && (s.sent < limit || size < FRAME_BYTES));
            if !open {
                if let Some(full) = take.sessions.back_mut()
                    && !full.ended
                {
                    full.end().await;
                }
                let offset_ms = ms_of(take.sent);
                tracing::debug!(offset_ms, "the dictation goes on on a new session");
                take.sessions.push_back(IatSession::new(self.open().await?, offset_ms));
            }
            let Some(session) = take.sessions.back_mut() else { return Err(realtime::closed_early()) };
            let audio = &take.pending[..size];
            let frame = if session.sent == 0 {
                json!({ "common": { "app_id": self.app_id }, "business": business, "data": iat_audio(0, audio) })
            } else {
                json!({ "data": iat_audio(1, audio) })
            };
            match session.socket.send(Message::text(frame.to_string())).await {
                Ok(()) => {
                    session.sent += size;
                    take.sent += size;
                    take.pending.drain(..size);
                }
                // The service may have ended the session as the frame went out: what it said last
                // is still to be read. The frame waits for the next session.
                Err(error) if session.sent > 0 => {
                    tracing::debug!(%error, "a dictation session stopped taking audio");
                    session.ended = true;
                    return Ok(());
                }
                Err(error) => return Err(said_before(&mut session.socket, error, |text| parse_iat(text).err()).await),
            }
        }
    }
}

#[async_trait]
impl VendorAsr for IatAsr {
    fn model(&self) -> &str {
        &self.model
    }

    /// The hot words are the app's, in the iFlytek console.
    fn sends_glossary(&self, _glossary: &[String]) -> bool {
        false
    }

    /// A recorded take cut into pieces of 55 seconds at most, in its pauses: one session each, at
    /// the pace the audio plays, each within its own deadline (docs/dictation.md §3.15).
    async fn transcribe(&self, wav: &[u8], language: Option<&str>, glossary: &[String]) -> Result<Transcript, AsrError> {
        let started = Instant::now();
        let pcm = pcm_of(wav)?;
        let mut texts = Vec::new();
        for piece in split::quiet_cuts(pcm, self.session_ms) {
            let piece = realtime::transcribe_whole(self, &pcm[piece], language, glossary, FRAME_BYTES, WHOLE_TAKE_PACE, self.config.timeout).await?;
            texts.push(piece.text);
        }
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        Ok(Transcript { text: join_sentences(&texts), latency_ms, model: self.model.clone() })
    }

    /// The handshake checks the key and the secret only: the app id goes in the first frame. So
    /// the check is a whole session: the first frame with 40 ms of silence, the end frame, and the
    /// service's last result without an error code.
    async fn probe(&self) -> Result<(), AsrError> {
        let check = async {
            let mut socket = self.open().await?;
            let first = json!({ "common": { "app_id": self.app_id }, "business": self.business(None), "data": iat_audio(0, &[0; FRAME_BYTES]) });
            for frame in [first.to_string(), IAT_END.to_owned()] {
                if let Err(error) = socket.send(Message::text(frame)).await {
                    return Err(said_before(&mut socket, error, |text| parse_iat(text).err()).await);
                }
            }
            loop {
                match socket.next().await {
                    Some(Ok(Message::Text(text))) => {
                        if parse_iat(text.as_str())?.status == 2 {
                            let _ = socket.close(normal_close()).await;
                            return Ok(());
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => return Err(realtime::closed_early()),
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

    /// The take's sessions: the first connects at once (a refused credential fails the take
    /// early), the next ones when audio comes for them. A session's results are its sentence so
    /// far; its last one, the sentence, timed from the audio sent before it to the audio it took.
    /// After [`RealtimeInput::Finish`] the sessions say their last results for as long as the
    /// caller waits: an input closed then, as before, closes them. However the take ends, its
    /// sessions' connections close with a normal close (1000), as the service asks.
    async fn realtime(
        &self,
        language: Option<&str>,
        _glossary: &[String],
        mut input: mpsc::UnboundedReceiver<RealtimeInput>,
        events: RealtimeEvents,
    ) -> Result<(), AsrError> {
        let mut take = IatTake { sessions: VecDeque::new(), pending: Vec::new(), sent: 0 };
        take.sessions.push_back(IatSession::new(self.open().await?, 0));
        let result = self.run(&mut take, &self.business(language), &mut input, &events).await;
        take.close().await;
        result
    }
}

impl IatAsr {
    /// A take's sessions until everything was said ([`RealtimeEvent::Finished`]), the take was
    /// given up (its input closed) or one failed.
    async fn run(
        &self,
        take: &mut IatTake,
        business: &Value,
        input: &mut mpsc::UnboundedReceiver<RealtimeInput>,
        events: &RealtimeEvents,
    ) -> Result<(), AsrError> {
        let mut finished = false;
        loop {
            take.commit(events);
            if finished && take.sessions.is_empty() {
                let _ = events.send(RealtimeEvent::Finished);
                return Ok(());
            }
            tokio::select! {
                // What the service already said goes first: a session it ended takes no more audio.
                biased;
                (index, message) = next_message(&mut take.sessions) => take.on_message(index, message, events).await?,
                item = input.recv() => match item {
                    Some(RealtimeInput::Audio(pcm)) if !finished => {
                        take.pending.extend_from_slice(&pcm);
                        self.send_audio(take, business, false).await?;
                    }
                    Some(RealtimeInput::Finish) if !finished => {
                        finished = true;
                        self.send_audio(take, business, true).await?;
                        take.end_live().await;
                    }
                    Some(_) => {}
                    None => return Ok(()),
                },
            }
        }
    }
}

/// A live take's dictation sessions, oldest first: the newest may take audio, the others were
/// ended and are still saying their last results. A session's sentence is told once every
/// session before it told its own, so the sentences keep the take's order.
struct IatTake {
    sessions: VecDeque<IatSession>,
    /// Audio not sent yet: less than a frame, or a frame waiting for a new session.
    pending: Vec<u8>,
    /// Bytes of audio sent in all the sessions.
    sent: usize,
}

impl IatTake {
    /// Tell the sentences of the sessions that finished, in order, and the sentence so far of the
    /// one that comes next.
    fn commit(&mut self, events: &RealtimeEvents) {
        while self.sessions.front().is_some_and(|s| s.done) {
            if let Some(session) = self.sessions.pop_front()
                && let Some(sentence) = session.sentence()
            {
                let _ = events.send(sentence);
            }
            if let Some(next) = self.sessions.front()
                && !next.done
            {
                let text = next.text();
                if !text.is_empty() {
                    let _ = events.send(RealtimeEvent::Partial { text });
                }
            }
        }
    }

    /// What session `index` said: a result, its last result, an error, or the connection's end.
    async fn on_message(&mut self, index: usize, message: Option<Result<Message, tungstenite::Error>>, events: &RealtimeEvents) -> Result<(), AsrError> {
        let Some(session) = self.sessions.get_mut(index) else { return Ok(()) };
        match message {
            Some(Ok(Message::Text(text))) => {
                let reply = parse_iat(text.as_str())?;
                if let Some(result) = reply.result {
                    session.apply(result);
                }
                if reply.status == 2 {
                    // Its sentence is complete; the next audio goes to a new session.
                    session.done = true;
                    session.ended = true;
                    let _ = session.socket.close(normal_close()).await;
                } else if index == 0 {
                    let text = session.text();
                    if !text.is_empty() {
                        let _ = events.send(RealtimeEvent::Partial { text });
                    }
                }
                Ok(())
            }
            Some(Ok(Message::Close(_))) | None => Err(realtime::closed_early()),
            Some(Ok(_)) => Ok(()),
            Some(Err(error)) => Err(realtime::map_ws(error)),
        }
    }

    /// No more audio: the session taking it is ended. One that never had any (a take without
    /// audio) has nothing to say and is dropped.
    async fn end_live(&mut self) {
        if let Some(session) = self.sessions.back_mut()
            && !session.ended
        {
            if session.sent == 0 {
                session.ended = true;
                session.done = true;
                let _ = session.socket.close(normal_close()).await;
            } else {
                session.end().await;
            }
        }
    }

    /// The take is over (given up or failed, or done, when none is left): every connection
    /// still open closes, and nothing more is sent.
    async fn close(&mut self) {
        for session in self.sessions.iter_mut().filter(|s| !s.done) {
            let _ = session.socket.close(normal_close()).await;
        }
    }
}

/// One dictation session: a connection of 55 seconds of audio at most.
struct IatSession {
    socket: Socket,
    /// Milliseconds of the take's audio sent before it: its sentence starts there.
    offset_ms: u64,
    /// Bytes of audio it took.
    sent: usize,
    /// It takes no more audio: its end frame went out, a send to it failed, or it finished.
    ended: bool,
    /// Its last result arrived.
    done: bool,
    /// Its results, as the dynamic correction left them.
    pieces: Pieces,
}

impl IatSession {
    fn new(socket: Socket, offset_ms: u64) -> Self {
        Self { socket, offset_ms, sent: 0, ended: false, done: false, pieces: Pieces::default() }
    }

    /// Its sentence so far.
    fn text(&self) -> String {
        self.pieces.text()
    }

    fn apply(&mut self, result: IatResult) {
        self.pieces.apply(result);
    }

    /// Its sentence, from the audio sent before it to the end of the audio it took.
    fn sentence(&self) -> Option<RealtimeEvent> {
        let text = self.text();
        let text = text.trim();
        (!text.is_empty()).then(|| RealtimeEvent::Sentence { text: text.to_owned(), begin_ms: self.offset_ms, end_ms: self.offset_ms + ms_of(self.sent) })
    }

    /// Send the end frame; a session whose end frame does not go out says how it ended through
    /// its connection.
    async fn end(&mut self) {
        self.ended = true;
        if let Err(error) = self.socket.send(Message::text(IAT_END)).await {
            tracing::debug!(%error, "a dictation session's end frame did not go out");
        }
    }
}

/// A session's results by `sn`: its text is theirs in order.
#[derive(Debug, Default)]
struct Pieces(BTreeMap<u64, String>);

impl Pieces {
    /// A result: it replaces the results of its range (`pgs: rpl`) or adds to them (`apd`).
    fn apply(&mut self, result: IatResult) {
        if let Some((from, to)) = result.replaces {
            self.0.retain(|sn, _| !(from..=to).contains(sn));
        }
        self.0.insert(result.sn, result.text);
    }

    fn text(&self) -> String {
        self.0.values().map(String::as_str).collect()
    }
}

/// The next message of any session that has not finished, with its index; pending while none
/// can say anything.
fn next_message(sessions: &mut VecDeque<IatSession>) -> impl Future<Output = (usize, Option<Result<Message, tungstenite::Error>>)> + Send + '_ {
    std::future::poll_fn(move |cx| {
        for (index, session) in sessions.iter_mut().enumerate() {
            if session.done {
                continue;
            }
            if let Poll::Ready(message) = session.socket.poll_next_unpin(cx) {
                return Poll::Ready((index, message));
            }
        }
        Poll::Pending
    })
}

/// The close frame iFlytek asks a client to end with: 1000, a normal close.
fn normal_close() -> Option<CloseFrame> {
    Some(CloseFrame { code: CloseCode::Normal, reason: Utf8Bytes::from_static("") })
}

/// `data` of an audio frame: its status (0 the session's first, 1 the others) and the audio.
fn iat_audio(status: u8, audio: &[u8]) -> Value {
    json!({ "status": status, "format": IAT_FORMAT, "encoding": "raw", "audio": STANDARD.encode(audio) })
}

/// One answer of the dictation service.
#[derive(Debug, PartialEq, Eq)]
struct IatReply {
    /// `data.status`: 2 on the session's last result.
    status: u64,
    /// `data.result`, when it has one.
    result: Option<IatResult>,
}

/// A dictation result: its words, and the earlier results it replaces.
#[derive(Debug, PartialEq, Eq)]
struct IatResult {
    sn: u64,
    /// `rg` of a result with `pgs: rpl`: the `sn`s it replaces, both included.
    replaces: Option<(u64, u64)>,
    text: String,
}

/// A dictation answer; one with an error code is the error it names.
fn parse_iat(text: &str) -> Result<IatReply, AsrError> {
    let value: Value = serde_json::from_str(text).map_err(|e| bad_answer("dictation", &e, text))?;
    let code = scalar(value.get("code"));
    if !code.is_empty() && code != "0" {
        return Err(iat_error(&code, &scalar(value.get("message"))));
    }
    let data = value.get("data");
    let status = number(data.and_then(|d| d.get("status"))).unwrap_or(0);
    let result = data.and_then(|d| d.get("result")).filter(|r| r.is_object()).map(|r| {
        let range = r.get("rg").and_then(Value::as_array).and_then(|rg| Some((number(rg.first())?, number(rg.get(1))?)));
        IatResult {
            sn: number(r.get("sn")).unwrap_or(0),
            replaces: if r.get("pgs").and_then(Value::as_str) == Some("rpl") { range } else { None },
            text: words(r.get("ws")),
        }
    });
    Ok(IatReply { status, result })
}

/// A dictation error code as the error it is (the 错误码 table of the dictation API).
fn iat_error(code: &str, message: &str) -> AsrError {
    let said = || voltip_reply::clean(&format!("{code}: {message}"));
    match code {
        // The app id is wrong or missing (10313 also when it is not the key's app).
        "10005" | "10313" => AsrError::Unauthorized { message: said() },
        // The package's calls are used up, not licensed or expired; the day's calls are used up;
        // the licence expired.
        "11200" | "11201" | "11203" => AsrError::QuotaExhausted { code: code.to_owned(), message: voltip_reply::clean(message), retry_at_ms: None },
        // Too many requests in a second.
        "11202" => AsrError::RateLimited { retry_after_ms: None, message: said() },
        // The service's network failed.
        "10222" => AsrError::Network(said()),
        _ => AsrError::Service { code: code.to_owned(), message: voltip_reply::clean(message) },
    }
}

/// `authorization` of a dictation URL: the HMAC-SHA256 of `host`, `date` and the request line
/// with the secret, in the signature line with the key, base64.
fn iat_authorization(api_key: &str, api_secret: &str, host: &str, date: &str, path: &str) -> String {
    let signature = STANDARD.encode(hmac_sha256(api_secret, &format!("host: {host}\ndate: {date}\nGET {path} HTTP/1.1")));
    STANDARD.encode(format!(r#"api_key="{api_key}", algorithm="hmac-sha256", headers="host date request-line", signature="{signature}""#))
}

fn hmac_sha256(key: &str, message: &str) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes()).unwrap_or_else(|_| unreachable!("HMAC takes a key of any length"));
    mac.update(message.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

/// Realtime transcription's credential: the app's AppID and the service's own APIKey.
#[derive(Deserialize)]
struct RtasrCredential {
    app_id: String,
    api_key: String,
}

/// iFlytek realtime transcription (实时语音转写): one session for a take of any length.
pub(crate) struct RtasrAsr {
    config: AsrConfig,
    /// `rtasr`: the receipt's model; the service has one.
    model: String,
    /// `wss://<host>[/prefix]/v1/ws`, without the signature.
    endpoint: Url,
    app_id: String,
    api_key: String,
}

impl fmt::Debug for RtasrAsr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RtasrAsr").field("endpoint", &self.endpoint.as_str()).field("model", &self.model).finish_non_exhaustive()
    }
}

impl RtasrAsr {
    /// The client of `config`: the stored base URL, the credential JSON with `app_id` and
    /// `api_key`.
    pub(crate) fn new(config: AsrConfig) -> Result<Self, AsrError> {
        let endpoint = vendor::endpoint(&config.base_url, true, "/v1/ws")?;
        let credential: RtasrCredential = credentials::parse(config.token.as_deref())?;
        let app_id = credentials::require(&credential.app_id, "app_id")?;
        let api_key = credentials::require(&credential.api_key, "api_key")?;
        let model = config.model.trim().to_owned();
        if model.is_empty() {
            return Err(AsrError::InvalidConfig("model is empty".into()));
        }
        Ok(Self { config, model, endpoint, app_id, api_key })
    }

    /// The endpoint signed for `ts` (seconds since the epoch). Neither `lang` nor `pd` goes with
    /// it: the service's default is Mandarin with English mixed in, for every domain.
    fn signed_url(&self, ts: u64) -> Url {
        let mut url = self.endpoint.clone();
        url.set_query(Some(&format!("appid={}&ts={ts}&signa={}", component(&self.app_id), component(&signa(&self.app_id, &self.api_key, ts)))));
        url
    }

    /// A connection the service started its session on: it checks the signature and the licence
    /// after the handshake and answers `started` or the error.
    async fn open(&self) -> Result<Socket, AsrError> {
        let ts = u64::try_from(chrono::Utc::now().timestamp()).unwrap_or(0);
        let (mut socket, _) = realtime::connect(realtime::request(&self.signed_url(ts))?, &self.config.network).await?;
        let started = async {
            loop {
                match socket.next().await {
                    Some(Ok(Message::Text(text))) => {
                        if parse_rtasr(text.as_str())? == RtasrReply::Started {
                            return Ok(());
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => return Err(realtime::closed_early()),
                    Some(Ok(_)) => {}
                    Some(Err(error)) => return Err(realtime::map_ws(error)),
                }
            }
        };
        tokio::time::timeout(realtime::CONNECT_TIMEOUT, started).await.map_err(|_| AsrError::Timeout)??;
        Ok(socket)
    }
}

#[async_trait]
impl VendorAsr for RtasrAsr {
    fn model(&self) -> &str {
        &self.model
    }

    /// The service takes no hot words.
    fn sends_glossary(&self, _glossary: &[String]) -> bool {
        false
    }

    /// A recorded take on one session, at the pace the audio plays (docs/dictation.md §3.15).
    async fn transcribe(&self, wav: &[u8], language: Option<&str>, glossary: &[String]) -> Result<Transcript, AsrError> {
        let pcm = pcm_of(wav)?;
        if pcm.is_empty() {
            return Ok(Transcript { text: String::new(), latency_ms: 0, model: self.model.clone() });
        }
        realtime::transcribe_whole(self, pcm, language, glossary, FRAME_BYTES, WHOLE_TAKE_PACE, self.config.timeout).await
    }

    /// The service says `started` once the signature and the licence hold; the session then ends
    /// without audio.
    async fn probe(&self) -> Result<(), AsrError> {
        let check = async {
            let mut socket = self.open().await?;
            let _ = socket.send(Message::binary(RTASR_END)).await;
            let _ = socket.close(normal_close()).await;
            Ok(())
        };
        tokio::time::timeout(self.config.timeout, check).await.map_err(|_| AsrError::Timeout)?
    }

    fn streams(&self) -> bool {
        true
    }

    /// One session: binary audio in 1280-byte frames, the end marker on
    /// [`RealtimeInput::Finish`], and the service's sentences until it closes the connection. After
    /// `Finish` the session lasts as long as the caller waits: an input closed then, as before,
    /// closes it. However it ends, the connection closes with a normal close.
    async fn realtime(
        &self,
        _language: Option<&str>,
        _glossary: &[String],
        mut input: mpsc::UnboundedReceiver<RealtimeInput>,
        events: RealtimeEvents,
    ) -> Result<(), AsrError> {
        let mut socket = self.open().await?;
        let result = rtasr_run(&mut socket, &mut input, &events).await;
        let _ = socket.close(normal_close()).await;
        result
    }
}

/// A realtime transcription session until everything was said ([`RealtimeEvent::Finished`]), the
/// take was given up (its input closed) or it failed.
async fn rtasr_run(socket: &mut Socket, input: &mut mpsc::UnboundedReceiver<RealtimeInput>, events: &RealtimeEvents) -> Result<(), AsrError> {
    let (mut pending, mut sent, mut last_end_ms, mut finished) = (Vec::<u8>::new(), 0usize, 0u64, false);
    loop {
        tokio::select! {
            // What the service already said goes first: an error ends the session.
            biased;
            message = socket.next() => match message {
                Some(Ok(Message::Text(text))) => match parse_rtasr(text.as_str())? {
                    RtasrReply::Partial(text) => {
                        let _ = events.send(RealtimeEvent::Partial { text });
                    }
                    RtasrReply::Sentence { text, begin_ms, end_ms } => {
                        // Without the service's times, the sentence runs from the last one's end to
                        // the audio sent so far.
                        let begin_ms = begin_ms.unwrap_or(last_end_ms);
                        let end_ms = end_ms.filter(|&end| end > begin_ms).unwrap_or_else(|| ms_of(sent).max(begin_ms));
                        last_end_ms = end_ms;
                        let _ = events.send(RealtimeEvent::Sentence { text, begin_ms, end_ms });
                    }
                    RtasrReply::Started | RtasrReply::Nothing => {}
                },
                // Everything was said: the service closes the connection after its last result. A
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
                    sent += send_binary(socket, &mut pending, false).await?;
                }
                Some(RealtimeInput::Finish) if !finished => {
                    finished = true;
                    sent += send_binary(socket, &mut pending, true).await?;
                    if let Err(error) = socket.send(Message::binary(RTASR_END)).await {
                        return Err(said_before(socket, error, |text| parse_rtasr(text).err()).await);
                    }
                }
                Some(_) => {}
                None => return Ok(()),
            },
        }
    }
}

/// Send `pending`'s whole frames as binary messages, and with `flush` the rest as the last one;
/// the bytes sent.
async fn send_binary(socket: &mut Socket, pending: &mut Vec<u8>, flush: bool) -> Result<usize, AsrError> {
    let mut at = 0;
    while pending.len() - at >= FRAME_BYTES || (flush && pending.len() > at) {
        let end = (at + FRAME_BYTES).min(pending.len());
        if let Err(error) = socket.send(Message::binary(pending[at..end].to_vec())).await {
            return Err(said_before(socket, error, |text| parse_rtasr(text).err()).await);
        }
        at = end;
    }
    pending.drain(..at);
    Ok(at)
}

/// One message of realtime transcription.
#[derive(Debug, PartialEq, Eq)]
enum RtasrReply {
    /// The session started.
    Started,
    /// The sentence being spoken, whole so far.
    Partial(String),
    /// A finished sentence, with its times when the service gave them.
    Sentence { text: String, begin_ms: Option<u64>, end_ms: Option<u64> },
    /// Nothing to tell: an empty result, one of a kind not asked for.
    Nothing,
}

/// A realtime transcription message; `action: error` (or another code than `0`) is the error it
/// names. A result's `data` is JSON in a string: `cn.st` with the words (`rt[].ws[].cw[].w`), the
/// `type` (0 final, 1 the sentence so far) and the times (`bg`, `ed`, in milliseconds of the
/// session, as text or numbers).
fn parse_rtasr(text: &str) -> Result<RtasrReply, AsrError> {
    let value: Value = serde_json::from_str(text).map_err(|e| bad_answer("realtime transcription", &e, text))?;
    let action = value.get("action").and_then(Value::as_str).unwrap_or_default();
    let code = scalar(value.get("code"));
    if action == "error" || (!code.is_empty() && code != "0") {
        return Err(rtasr_error(&code, &scalar(value.get("desc"))));
    }
    match action {
        "started" => Ok(RtasrReply::Started),
        "result" => {
            let data = value.get("data").and_then(Value::as_str).unwrap_or_default();
            if data.trim().is_empty() {
                return Ok(RtasrReply::Nothing);
            }
            let data: Value = serde_json::from_str(data).map_err(|e| bad_answer("realtime transcription", &e, text))?;
            // A translation (`biz: trans`) is not asked for.
            let Some(st) = data.pointer("/cn/st").filter(|_| data.get("biz").is_none()) else { return Ok(RtasrReply::Nothing) };
            let words: String = st.get("rt").and_then(Value::as_array).into_iter().flatten().map(|rt| words(rt.get("ws"))).collect();
            let words = words.trim().to_owned();
            if words.is_empty() {
                return Ok(RtasrReply::Nothing);
            }
            if scalar(st.get("type")) == "0" {
                Ok(RtasrReply::Sentence { text: words, begin_ms: number(st.get("bg")), end_ms: number(st.get("ed")) })
            } else {
                Ok(RtasrReply::Partial(words))
            }
        }
        _ => Ok(RtasrReply::Nothing),
    }
}

/// A realtime transcription error code as the error it is (the 错误码 table and the 常见问题 of
/// the realtime transcription API).
fn rtasr_error(code: &str, desc: &str) -> AsrError {
    let said = || voltip_reply::clean(&format!("{code}: {desc}"));
    // The service names a signature it could not verify in the description, whatever the code.
    let signature = desc.contains("signa") || desc.contains("authorization");
    match code {
        // Not allowed: the app id, the key, the time or the address.
        "10105" => AsrError::Unauthorized { message: said() },
        "10110" if signature => AsrError::Unauthorized { message: said() },
        // No licence: the hours are used up, the licence expired or every licensed connection is
        // in use.
        "10110" => AsrError::QuotaExhausted { code: code.to_owned(), message: voltip_reply::clean(desc), retry_at_ms: None },
        // More connections than licensed.
        "10800" => AsrError::RateLimited { retry_after_ms: None, message: said() },
        // The service's WebSocket failed.
        "10202" | "10204" | "10205" => AsrError::Network(said()),
        _ => AsrError::Service { code: code.to_owned(), message: voltip_reply::clean(desc) },
    }
}

/// `signa`: the HMAC-SHA1 of the hex MD5 of the app id and `ts`, with the key, base64.
fn signa(app_id: &str, api_key: &str, ts: u64) -> String {
    let md5: String = Md5::digest(format!("{app_id}{ts}").as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    let mut mac = Hmac::<Sha1>::new_from_slice(api_key.as_bytes()).unwrap_or_else(|_| unreachable!("HMAC takes a key of any length"));
    mac.update(md5.as_bytes());
    STANDARD.encode(mac.finalize().into_bytes())
}

/// A send failed because the connection is going: the error the service answered before it went
/// explains it better (`read` picks it out of a text message), else the send's own error.
async fn said_before(socket: &mut Socket, failed: tungstenite::Error, read: fn(&str) -> Option<AsrError>) -> AsrError {
    let reason = async {
        while let Some(Ok(message)) = socket.next().await {
            if let Message::Text(text) = message
                && let Some(error) = read(text.as_str())
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

/// The words of a result's `ws`: each word's first candidate, as the service spaced them.
fn words(ws: Option<&Value>) -> String {
    ws.and_then(Value::as_array).into_iter().flatten().filter_map(|w| w.pointer("/cw/0/w").and_then(Value::as_str)).collect()
}

/// A JSON scalar as text: `"10110"` and `10110` alike; empty for anything else.
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

fn bad_answer(service: &str, error: &serde_json::Error, text: &str) -> AsrError {
    AsrError::BadResponse(format!("{service} answer: {error}: {}", truncate_chars(text, MAX_ERROR_BODY_CHARS)))
}

/// `host[:port]` of `url`, as its handshake's `Host` header names it.
fn authority(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default();
    url.port().map_or_else(|| host.to_owned(), |port| format!("{host}:{port}"))
}

/// `text` as a URL query component: everything but the unreserved characters percent-encoded
/// (`Wed%2C%2010%20Jul…`, as the services' examples write it).
fn component(text: &str) -> String {
    text.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') { char::from(b).to_string() } else { format!("%{b:02X}") })
        .collect()
}

/// `zh-CN` → `zh`, trimmed and lower-cased.
fn primary_subtag(language: &str) -> String {
    language.trim().split(['-', '_']).next().unwrap_or_default().to_ascii_lowercase()
}

/// Milliseconds of `bytes` of the PCM.
fn ms_of(bytes: usize) -> u64 {
    u64::try_from(bytes / BYTES_PER_MS).unwrap_or(u64::MAX)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::{Arc, Mutex};

    use tokio::net::{TcpListener, TcpStream};
    use tokio_tungstenite::WebSocketStream;
    use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
    use tokio_tungstenite::tungstenite::http::StatusCode;

    use super::*;
    use crate::dashscope::tests::wav;

    /// Test credentials: no service knows them.
    const IAT_TOKEN: &str = r#"{"set":"default","app_id":"app-test","api_key":"key-test","api_secret":"secret-test"}"#;
    const RTASR_TOKEN: &str = r#"{"set":"default","app_id":"app-test","api_key":"rtasr-key-test"}"#;
    /// The word each scripted dictation session answers every frame with: one per session.
    const SESSION_WORDS: [&str; 4] = ["一", "二", "三", "四"];

    /// What a scripted service saw.
    #[derive(Default)]
    pub(crate) struct Seen {
        /// Each connection's request target (path and query) and `Host` header.
        pub(crate) handshakes: Vec<(String, String)>,
        /// Each connection's messages.
        pub(crate) frames: Vec<Vec<Message>>,
        /// The connections the client closed.
        pub(crate) closed: Vec<usize>,
        /// The code each of them closed with (`None` for a close frame without one).
        pub(crate) close_codes: Vec<Option<u16>>,
    }

    impl Seen {
        /// Connection `index` was closed by the client with `frame`.
        pub(crate) fn close(&mut self, index: usize, frame: Option<&CloseFrame>) {
            self.closed.push(index);
            self.close_codes.push(frame.map(|f| u16::from(f.code)));
        }

        fn open(&mut self) -> usize {
            self.handshakes.push(Default::default());
            self.frames.push(Vec::new());
            self.frames.len() - 1
        }

        /// The text frames of connection `index`, as JSON.
        pub(crate) fn json(&self, index: usize) -> Vec<Value> {
            self.frames[index].iter().filter_map(|m| if let Message::Text(t) = m { Some(serde_json::from_str(t.as_str()).unwrap()) } else { None }).collect()
        }

        /// The binary frames of connection `index`.
        pub(crate) fn binary(&self, index: usize) -> Vec<Vec<u8>> {
            self.frames[index].iter().filter_map(|m| if let Message::Binary(b) = m { Some(b.to_vec()) } else { None }).collect()
        }
    }

    /// A local endpoint whose connections `session` scripts; its `http://` base and what it saw.
    pub(crate) async fn serve<F, Fut>(session: F) -> (String, Arc<Mutex<Seen>>)
    where
        F: Fn(TcpStream, usize, Arc<Mutex<Seen>>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Seen::default()));
        let shared = seen.clone();
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                let index = shared.lock().unwrap().open();
                tokio::spawn(session(tcp, index, shared.clone()));
            }
        });
        (base, seen)
    }

    /// The WebSocket handshake of connection `index`, recorded; refused with `refuse`'s status and
    /// body when given.
    // tungstenite's handshake callback returns its `ErrorResponse` by value.
    #[allow(clippy::result_large_err)]
    pub(crate) async fn accept(
        tcp: TcpStream,
        index: usize,
        refuse: Option<(u16, &'static str)>,
        seen: &Arc<Mutex<Seen>>,
    ) -> Option<WebSocketStream<TcpStream>> {
        let record = seen.clone();
        let callback = move |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
            let host = request.headers().get("host").and_then(|v| v.to_str().ok()).unwrap_or_default().to_owned();
            record.lock().unwrap().handshakes[index] = (request.uri().to_string(), host);
            match refuse {
                Some((status, body)) => {
                    let mut refusal = ErrorResponse::new(Some(body.to_owned()));
                    *refusal.status_mut() = StatusCode::from_u16(status).unwrap();
                    Err(refusal)
                }
                None => Ok(response),
            }
        };
        tokio_tungstenite::accept_hdr_async(tcp, callback).await.ok()
    }

    /// Read on until the client goes, noting that it closed: a service that said its last word.
    pub(crate) async fn read_on(socket: &mut WebSocketStream<TcpStream>, index: usize, seen: &Arc<Mutex<Seen>>) {
        while let Some(Ok(message)) = socket.next().await {
            if let Message::Close(frame) = message {
                seen.lock().unwrap().close(index, frame.as_ref());
                return;
            }
            seen.lock().unwrap().frames[index].push(message);
        }
    }

    /// Wait for what `check` reads to hold (the service's side of a connection runs on its own).
    pub(crate) async fn eventually(what: &str, check: impl Fn() -> bool) {
        let start = std::time::Instant::now();
        while !check() {
            assert!(start.elapsed() < Duration::from_secs(10), "timed out waiting for {what}");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// `bytes` of PCM that differ from frame to frame.
    pub(crate) fn pcm(bytes: usize) -> Vec<u8> {
        (0..bytes).map(|i| (i % 251) as u8).collect()
    }

    /// Run a live session over `inputs`, its input open until it ends: its result and events.
    pub(crate) async fn live(client: &dyn VendorAsr, language: Option<&str>, inputs: Vec<RealtimeInput>) -> (Result<(), AsrError>, Vec<RealtimeEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        for input in inputs {
            tx.send(input).unwrap();
        }
        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        let result = tokio::time::timeout(Duration::from_secs(20), client.realtime(language, &[], rx, events_tx)).await.expect("the session ended");
        drop(tx);
        let mut events = Vec::new();
        while let Ok(event) = events_rx.try_recv() {
            events.push(event);
        }
        (result, events)
    }

    pub(crate) fn sentences(events: &[RealtimeEvent]) -> Vec<(String, u64, u64)> {
        events
            .iter()
            .filter_map(|e| match e {
                RealtimeEvent::Sentence { text, begin_ms, end_ms } => Some((text.clone(), *begin_ms, *end_ms)),
                _ => None,
            })
            .collect()
    }

    pub(crate) fn partials(events: &[RealtimeEvent]) -> Vec<String> {
        events
            .iter()
            .filter_map(|e| match e {
                RealtimeEvent::Partial { text } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    // ---- Dictation (iat) --------------------------------------------------------------------

    /// What a scripted dictation service does.
    #[derive(Clone, Copy)]
    enum Iat {
        /// Answer each audio frame with the session's word and the end frame with the last
        /// result (`。`); a first frame of another app than `app-test` with 10313, as the service.
        Dictate,
        /// Answer the first three frames and the end frame with the dynamic correction.
        Corrects,
        /// End the first session by itself after this many audio frames, as a long pause does.
        EndsAfter(usize),
        /// Refuse the handshake with this status and body.
        Refuses(u16, &'static str),
        /// Answer the first frame with this code and message, then read on.
        Fails(i64, &'static str),
        /// Drop the connection at the first frame.
        Drops,
        /// Say nothing.
        Silent,
    }

    fn iat_result(sn: u64, status: u64, replaces: Option<[u64; 2]>, words: &[&str]) -> Message {
        let ws: Vec<Value> = words.iter().map(|w| json!({ "bg": 0, "cw": [{ "sc": 0, "w": w }] })).collect();
        let mut result = json!({ "sn": sn, "ls": status == 2, "bg": 0, "ed": 0, "pgs": "apd", "ws": ws });
        if let Some(rg) = replaces {
            result["pgs"] = json!("rpl");
            result["rg"] = json!(rg);
        }
        Message::text(json!({ "code": 0, "message": "success", "sid": "iat-test", "data": { "result": result, "status": status } }).to_string())
    }

    fn iat_failure(code: i64, message: &str) -> Message {
        Message::text(json!({ "code": code, "message": message, "sid": "iat-test" }).to_string())
    }

    async fn serve_iat(script: Iat) -> (String, Arc<Mutex<Seen>>) {
        serve(move |tcp, index, seen| iat_session(tcp, index, script, seen)).await
    }

    async fn iat_session(tcp: TcpStream, index: usize, script: Iat, seen: Arc<Mutex<Seen>>) {
        let refuse = if let Iat::Refuses(status, body) = script { Some((status, body)) } else { None };
        let Some(mut socket) = accept(tcp, index, refuse, &seen).await else { return };
        let word = SESSION_WORDS[index % SESSION_WORDS.len()];
        let (mut sn, mut frames, mut over) = (0u64, 0usize, false);
        while let Some(Ok(message)) = socket.next().await {
            if let Message::Close(frame) = message {
                seen.lock().unwrap().close(index, frame.as_ref());
                return;
            }
            seen.lock().unwrap().frames[index].push(message.clone());
            let Message::Text(text) = message else { continue };
            let frame: Value = serde_json::from_str(text.as_str()).unwrap();
            let status = frame.pointer("/data/status").and_then(Value::as_u64).unwrap();
            match script {
                Iat::Refuses(..) | Iat::Drops => return,
                Iat::Silent => {}
                Iat::Fails(code, message) => {
                    socket.send(iat_failure(code, message)).await.unwrap();
                    read_on(&mut socket, index, &seen).await;
                    return;
                }
                Iat::Dictate | Iat::EndsAfter(_) => {
                    if status == 0 && frame.pointer("/common/app_id") != Some(&json!("app-test")) {
                        socket.send(iat_failure(10313, "invalid appid")).await.unwrap();
                        read_on(&mut socket, index, &seen).await;
                        return;
                    }
                    if over {
                        // Audio after the session ended, as the service answers it.
                        socket.send(iat_failure(10101, "engine session has ended")).await.unwrap();
                        continue;
                    }
                    sn += 1;
                    if status == 2 {
                        socket.send(iat_result(sn, 2, None, &["。"])).await.unwrap();
                        over = true;
                        continue;
                    }
                    frames += 1;
                    socket.send(iat_result(sn, u64::from(frames > 1), None, &[word])).await.unwrap();
                    if matches!(script, Iat::EndsAfter(n) if index == 0 && frames == n) {
                        sn += 1;
                        socket.send(iat_result(sn, 2, None, &["。"])).await.unwrap();
                        over = true;
                    }
                }
                Iat::Corrects => {
                    sn += 1;
                    let reply = match (status, sn) {
                        (2, _) => iat_result(sn, 2, None, &["好的。"]),
                        (_, 1) => iat_result(1, 0, None, &["侧室"]),
                        (_, 2) => iat_result(2, 1, Some([1, 1]), &["测试", "一下"]),
                        (_, 3) => iat_result(3, 1, None, &["，"]),
                        _ => continue,
                    };
                    socket.send(reply).await.unwrap();
                }
            }
        }
    }

    fn iat_at(base: &str) -> IatAsr {
        IatAsr::new(AsrConfig::new(base, " iat ").with_token(Some(IAT_TOKEN.into()))).unwrap()
    }

    #[test]
    fn iat_signs_like_the_documented_example() {
        // The worked example of the API's 鉴权方法 (its key and secret are the page's placeholders).
        let (key, secret, date) = ("keyxxxxxxxx8ee279348519exxxxxxxx", "secretxxxxxxxx2df7900c09xxxxxxxx", "Wed, 10 Jul 2019 07:35:43 GMT");
        let documented = "YXBpX2tleT0ia2V5eHh4eHh4eHg4ZWUyNzkzNDg1MTlleHh4eHh4eHgiLCBhbGdvcml0aG09ImhtYWMtc2hhMjU2IiwgaGVhZGVycz0iaG9zdCBkYXRlIHJlcXVlc3QtbGluZSIsIHNpZ25hdHVyZT0iSHAzVHk0WmtTQm1MOGpLeU9McFFpdjlTcjVudm1lWUVIN1dzTC9aTzJKZz0i";
        assert_eq!(iat_authorization(key, secret, "iat-api.xfyun.cn", date, "/v2/iat"), documented);
        let decoded = String::from_utf8(STANDARD.decode(documented).unwrap()).unwrap();
        assert!(decoded.ends_with(r#"signature="Hp3Ty4ZkSBmL8jKyOLpQiv9Sr5nvmeYEH7WsL/ZO2Jg=""#), "{decoded}");
        let token = json!({ "app_id": "app", "api_key": key, "api_secret": secret }).to_string();
        let client = IatAsr::new(AsrConfig::new("https://iat-api.xfyun.cn", "iat").with_token(Some(token))).unwrap();
        assert_eq!(
            client.signed_url(date).as_str(),
            format!("wss://iat-api.xfyun.cn/v2/iat?authorization={documented}&date=Wed%2C%2010%20Jul%202019%2007%3A35%3A43%20GMT&host=iat-api.xfyun.cn")
        );
        assert!(!format!("{client:?}").contains(secret) && !format!("{client:?}").contains(key));
    }

    /// The signature names the host and the request line the connection uses: a proxy's address
    /// with a path prefix signs that prefix. Checked against a signature computed here.
    #[tokio::test]
    async fn iat_signs_the_host_and_path_it_connects_to() {
        let (base, seen) = serve_iat(Iat::Dictate).await;
        iat_at(&format!("{base}/xf/")).probe().await.unwrap();
        let (target, host_header) = seen.lock().unwrap().handshakes[0].clone();
        let url = Url::parse(&format!("ws://{host_header}{target}")).unwrap();
        assert_eq!(url.path(), "/xf/v2/iat");
        let query: std::collections::HashMap<String, String> = url.query_pairs().into_owned().collect();
        assert_eq!(query.len(), 3, "{query:?}");
        assert_eq!(query["host"], host_header, "the Host the handshake sent");
        let date = chrono::DateTime::parse_from_rfc2822(&query["date"]).unwrap();
        assert!((chrono::Utc::now().timestamp() - date.timestamp()).abs() < 60, "signed now: {}", query["date"]);
        assert!(query["date"].ends_with(" GMT"));
        let mut mac = Hmac::<Sha256>::new_from_slice(b"secret-test").unwrap();
        mac.update(format!("host: {}\ndate: {}\nGET /xf/v2/iat HTTP/1.1", query["host"], query["date"]).as_bytes());
        let signature = STANDARD.encode(mac.finalize().into_bytes());
        let origin = String::from_utf8(STANDARD.decode(&query["authorization"]).unwrap()).unwrap();
        assert_eq!(origin, format!(r#"api_key="key-test", algorithm="hmac-sha256", headers="host date request-line", signature="{signature}""#));
    }

    /// The first frame carries the app id and the parameters; the audio follows in 40 ms frames
    /// and a shorter last one, then the bare end frame. The session's results are its sentence so
    /// far; its last one is the sentence, timed by the audio it took.
    #[tokio::test]
    async fn iat_sends_its_parameters_first_and_frames_the_audio() {
        let (base, seen) = serve_iat(Iat::Dictate).await;
        let audio = pcm(3000);
        let (result, events) = live(&iat_at(&base), Some("zh-CN"), vec![RealtimeInput::Audio(audio.clone()), RealtimeInput::Finish]).await;
        result.unwrap();
        assert_eq!(
            events,
            vec![
                RealtimeEvent::Partial { text: "一".into() },
                RealtimeEvent::Partial { text: "一一".into() },
                RealtimeEvent::Partial { text: "一一一".into() },
                RealtimeEvent::Sentence { text: "一一一。".into(), begin_ms: 0, end_ms: 93 },
                RealtimeEvent::Finished,
            ]
        );
        let frames = seen.lock().unwrap().json(0);
        assert_eq!(frames.len(), 4, "{frames:?}");
        assert_eq!(frames[0]["common"], json!({ "app_id": "app-test" }));
        assert_eq!(frames[0]["business"], json!({ "language": "zh_cn", "domain": "iat", "accent": "mandarin", "dwa": "wpgs", "vad_eos": 10000 }));
        for (frame, (status, range)) in frames.iter().zip([(0, 0..1280), (1, 1280..2560), (1, 2560..3000)]) {
            assert_eq!(frame["data"]["status"], json!(status));
            assert_eq!(frame["data"]["format"], json!("audio/L16;rate=16000"));
            assert_eq!(frame["data"]["encoding"], json!("raw"));
            assert_eq!(STANDARD.decode(frame["data"]["audio"].as_str().unwrap()).unwrap(), audio[range]);
        }
        assert!(frames[1].get("common").is_none() && frames[1].get("business").is_none(), "the parameters go once");
        assert_eq!(frames[3], json!({ "data": { "status": 2 } }));
        eventually("the client closing", || seen.lock().unwrap().closed == [0]).await;
        assert_eq!(seen.lock().unwrap().close_codes, [Some(1000)], "the normal close the service asks for");

        // English: no dynamic correction, which is Mandarin's only.
        let (base, seen) = serve_iat(Iat::Dictate).await;
        let (result, _) = live(&iat_at(&base), Some("en-US"), vec![RealtimeInput::Audio(pcm(1280)), RealtimeInput::Finish]).await;
        result.unwrap();
        assert_eq!(seen.lock().unwrap().json(0)[0]["business"], json!({ "language": "en_us", "domain": "iat", "accent": "mandarin", "vad_eos": 10000 }));
    }

    #[test]
    fn iat_rebuilds_its_text_from_the_dynamic_correction() {
        // The documented answer (返回参数示例（动态修正dwa=wpgs）).
        let documented = r#"{"code":0,"message":"success","sid":"iatxxxxxxxxxxxxx","data":{"result":{"bg":0,"ed":0,"ls":false,"pgs":"rpl","rg":[1,1],"sn":2,"ws":[{"bg":0,"cw":[{"sc":0,"w":"测试"}]},{"bg":0,"cw":[{"sc":0,"w":"一下"}]}]},"status":1}}"#;
        let reply = parse_iat(documented).unwrap();
        assert_eq!(reply, IatReply { status: 1, result: Some(IatResult { sn: 2, replaces: Some((1, 1)), text: "测试一下".into() }) });
        let mut pieces = Pieces::default();
        pieces.apply(IatResult { sn: 1, replaces: None, text: "侧室".into() });
        assert_eq!(pieces.text(), "侧室");
        pieces.apply(reply.result.unwrap());
        assert_eq!(pieces.text(), "测试一下");
        pieces.apply(IatResult { sn: 3, replaces: None, text: "，好".into() });
        pieces.apply(IatResult { sn: 4, replaces: Some((3, 3)), text: "，好的".into() });
        pieces.apply(IatResult { sn: 5, replaces: Some((2, 4)), text: "测试一下，好的。".into() });
        assert_eq!(pieces.text(), "测试一下，好的。");
        // `apd` and a result without `pgs` append; `rg` counts only with `rpl`.
        let appended = parse_iat(r#"{"code":0,"data":{"status":2,"result":{"sn":6,"pgs":"apd","rg":[1,5],"ws":[{"cw":[{"w":"嗯"},{"w":"恩"}]}]}}}"#).unwrap();
        assert_eq!(appended, IatReply { status: 2, result: Some(IatResult { sn: 6, replaces: None, text: "嗯".into() }) });
        assert_eq!(parse_iat(r#"{"code":0,"data":{"status":2}}"#).unwrap(), IatReply { status: 2, result: None });
        assert!(matches!(parse_iat("not json"), Err(AsrError::BadResponse(_))));
    }

    #[tokio::test]
    async fn iat_corrections_reach_the_partials_and_the_sentence() {
        let (base, _) = serve_iat(Iat::Corrects).await;
        let (result, events) = live(&iat_at(&base), None, vec![RealtimeInput::Audio(pcm(3 * FRAME_BYTES)), RealtimeInput::Finish]).await;
        result.unwrap();
        assert_eq!(partials(&events), ["侧室", "测试一下", "测试一下，"]);
        assert_eq!(sentences(&events), [("测试一下，好的。".to_owned(), 0, 120)]);
        assert_eq!(events.last(), Some(&RealtimeEvent::Finished));
    }

    /// A session takes 55 seconds (one here): the next audio ends it and goes to a new session,
    /// freshly signed, whose sentence's times go on from the audio sent before it. A session's
    /// sentence so far shows only once the sessions before it told theirs.
    #[tokio::test]
    async fn iat_goes_on_on_a_new_session_when_one_is_full() {
        let (base, seen) = serve_iat(Iat::Dictate).await;
        let mut client = iat_at(&base);
        client.session_ms = 1_000;
        let audio = pcm(80_000);
        let inputs: Vec<RealtimeInput> = audio.chunks(1280).map(|c| RealtimeInput::Audio(c.to_vec())).chain([RealtimeInput::Finish]).collect();
        let (result, events) = live(&client, None, inputs).await;
        result.unwrap();
        assert_eq!(
            sentences(&events),
            [(format!("{}。", "一".repeat(25)), 0, 1000), (format!("{}。", "二".repeat(25)), 1000, 2000), (format!("{}。", "三".repeat(13)), 2000, 2500)]
        );
        assert_eq!(events.last(), Some(&RealtimeEvent::Finished));
        // The sentence so far is always the one that comes next.
        let mut next = sentences(&events).into_iter().map(|(text, ..)| text);
        let mut expected = next.next();
        for event in &events {
            match event {
                RealtimeEvent::Partial { text } => assert!(expected.as_deref().is_some_and(|s| s.starts_with(text.as_str())), "{text} before {expected:?}"),
                RealtimeEvent::Sentence { .. } => expected = next.next(),
                RealtimeEvent::Finished => {}
            }
        }
        let seen = seen.lock().unwrap();
        assert_eq!(seen.frames.len(), 3);
        let mut sent = Vec::new();
        for (index, frames) in [26, 26, 14].into_iter().enumerate() {
            let json = seen.json(index);
            assert_eq!(json.len(), frames, "session {index}");
            assert_eq!(json[0]["data"]["status"], json!(0), "each session starts with the parameters");
            assert_eq!(json[0]["common"]["app_id"], json!("app-test"));
            assert!(json[1..].iter().all(|f| f.get("common").is_none()));
            assert_eq!(json.last(), Some(&json!({ "data": { "status": 2 } })), "each session is ended");
            sent.extend(json[..frames - 1].iter().flat_map(|f| STANDARD.decode(f["data"]["audio"].as_str().unwrap()).unwrap()));
            assert!(seen.handshakes[index].0.starts_with("/v2/iat?authorization="), "each session is signed");
        }
        assert_eq!(sent, audio, "every byte once, in order");
    }

    /// The last bit of a take, less than a frame, stays on a full session: a new one would cost a
    /// call for 20 ms.
    #[tokio::test]
    async fn iat_the_last_bit_of_a_take_stays_on_a_full_session() {
        let (base, seen) = serve_iat(Iat::Dictate).await;
        let mut client = iat_at(&base);
        client.session_ms = 1_000;
        let (result, events) = live(&client, None, vec![RealtimeInput::Audio(pcm(32_000 + 640)), RealtimeInput::Finish]).await;
        result.unwrap();
        assert_eq!(sentences(&events), [(format!("{}。", "一".repeat(26)), 0, 1020)]);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.frames.len(), 1, "one session");
        assert_eq!(seen.json(0).len(), 27, "25 frames, the last bit, the end frame");
    }

    /// A session the service ended by itself (a long pause) takes no more audio: the next goes to
    /// a new session, timed on from the first.
    #[tokio::test]
    async fn iat_starts_a_new_session_after_one_the_service_ended() {
        let (base, seen) = serve_iat(Iat::EndsAfter(10)).await;
        let client = iat_at(&base);
        let (tx, rx) = mpsc::unbounded_channel();
        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        let run = client.realtime(None, &[], rx, events_tx);
        let drive = async {
            for _ in 0..10 {
                tx.send(RealtimeInput::Audio(pcm(1280))).unwrap();
            }
            let mut first = Vec::new();
            while !matches!(first.last(), Some(RealtimeEvent::Sentence { .. })) {
                first.push(tokio::time::timeout(Duration::from_secs(10), events_rx.recv()).await.unwrap().unwrap());
            }
            for _ in 0..5 {
                tx.send(RealtimeInput::Audio(pcm(1280))).unwrap();
            }
            tx.send(RealtimeInput::Finish).unwrap();
            first
        };
        let (result, mut events) = tokio::join!(run, drive);
        result.unwrap();
        while let Ok(event) = events_rx.try_recv() {
            events.push(event);
        }
        assert_eq!(sentences(&events), [(format!("{}。", "一".repeat(10)), 0, 400), (format!("{}。", "二".repeat(5)), 400, 600)]);
        assert_eq!(events.last(), Some(&RealtimeEvent::Finished));
        eventually("the first session closed", || seen.lock().unwrap().closed.contains(&0)).await;
        let seen = seen.lock().unwrap();
        assert_eq!(seen.json(0).len(), 10, "no end frame for a session the service ended");
        assert_eq!(seen.json(1).len(), 6);
    }

    #[test]
    fn iat_codes_sort_into_the_error_kinds() {
        let quota = |code: &str, message: &str| AsrError::QuotaExhausted { code: code.into(), message: message.into(), retry_at_ms: None };
        let service = |code: &str, message: &str| AsrError::Service { code: code.into(), message: message.into() };
        let cases = [
            ("10005", "licc failed", AsrError::Unauthorized { message: "10005: licc failed".into() }),
            ("10313", "invalid appid", AsrError::Unauthorized { message: "10313: invalid appid".into() }),
            ("11200", "auth no license", quota("11200", "auth no license")),
            ("11201", "auth no enough license", quota("11201", "auth no enough license")),
            ("11203", "license expired", quota("11203", "license expired")),
            ("11202", "too many requests", AsrError::RateLimited { retry_after_ms: None, message: "11202: too many requests".into() }),
            ("10222", "network error", AsrError::Network("10222: network error".into())),
            ("10165", "invalid handle", service("10165", "invalid handle")),
            ("10114", "session timeout", service("10114", "session timeout")),
            ("10110", "no license", service("10110", "no license")),
            ("10700", "engine error", service("10700", "engine error")),
        ];
        for (code, message, want) in cases {
            assert_eq!(iat_error(code, message), want, "{code}");
            let answer = json!({ "code": code.parse::<i64>().unwrap(), "message": message, "sid": "iat-test" }).to_string();
            assert_eq!(parse_iat(&answer), Err(want.clone()), "{code} as the service sends it");
        }
    }

    /// The service's codes, its refused handshakes and a dropped connection end a take with their
    /// kinds; nothing names the host.
    #[tokio::test]
    async fn iat_failures_end_the_take_with_their_kind() {
        let take = || vec![RealtimeInput::Audio(pcm(2 * FRAME_BYTES)), RealtimeInput::Finish];
        for (script, want) in [
            (Iat::Fails(11201, "day limit"), AsrError::QuotaExhausted { code: "11201".into(), message: "day limit".into(), retry_at_ms: None }),
            (Iat::Fails(10165, "invalid handle"), AsrError::Service { code: "10165".into(), message: "invalid handle".into() }),
            (Iat::Refuses(401, r#"{"message":"HMAC signature does not match"}"#), AsrError::Unauthorized { message: "HMAC signature does not match".into() }),
            (
                Iat::Refuses(403, r#"{"message":"HMAC signature cannot be verified, a valid date or x-date header is required for HMAC Authentication"}"#),
                AsrError::Unauthorized {
                    message: "HMAC signature cannot be verified, a valid date or x-date header is required for HMAC Authentication".into(),
                },
            ),
        ] {
            let (base, _) = serve_iat(script).await;
            assert_eq!(live(&iat_at(&base), None, take()).await.0, Err(want));
        }
        let (base, _) = serve_iat(Iat::Drops).await;
        let (result, events) = live(&iat_at(&base), None, take()).await;
        assert!(matches!(&result, Err(AsrError::Network(_))), "{result:?}");
        assert!(!events.contains(&RealtimeEvent::Finished));
        let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = closed.local_addr().unwrap().port();
        drop(closed);
        let (result, _) = live(&iat_at(&format!("http://127.0.0.1:{port}")), None, take()).await;
        assert!(matches!(&result, Err(AsrError::Network(m)) if !m.contains("127.0.0.1")), "{result:?}");
    }

    /// 测试连接 is a whole session: the first frame with 40 ms of silence and the end frame, and
    /// success only on the last result. A wrong app id, which the handshake does not check, fails.
    #[tokio::test]
    async fn iat_probe_waits_for_the_last_result() {
        let (base, seen) = serve_iat(Iat::Dictate).await;
        iat_at(&base).probe().await.unwrap();
        let frames = seen.lock().unwrap().json(0);
        assert_eq!(frames.len(), 2, "{frames:?}");
        assert_eq!(frames[0]["common"]["app_id"], json!("app-test"));
        assert_eq!(frames[0]["business"]["language"], json!("zh_cn"));
        assert_eq!(frames[0]["data"]["status"], json!(0));
        assert_eq!(STANDARD.decode(frames[0]["data"]["audio"].as_str().unwrap()).unwrap(), vec![0u8; 1280]);
        assert_eq!(frames[1], json!({ "data": { "status": 2 } }));

        let (base, _) = serve_iat(Iat::Dictate).await;
        let token = r#"{"app_id":"other-app","api_key":"key-test","api_secret":"secret-test"}"#;
        let wrong = IatAsr::new(AsrConfig::new(&base, "iat").with_token(Some(token.into()))).unwrap();
        assert_eq!(wrong.probe().await, Err(AsrError::Unauthorized { message: "10313: invalid appid".into() }));
        let (base, _) = serve_iat(Iat::Fails(11200, "licc failed")).await;
        assert_eq!(iat_at(&base).probe().await, Err(AsrError::QuotaExhausted { code: "11200".into(), message: "licc failed".into(), retry_at_ms: None }));
        let (base, _) = serve_iat(Iat::Refuses(401, r#"{"message":"Unauthorized"}"#)).await;
        assert_eq!(iat_at(&base).probe().await, Err(AsrError::Unauthorized { message: "Unauthorized".into() }));
        let (base, _) = serve_iat(Iat::Silent).await;
        let silent = IatAsr::new(AsrConfig::new(&base, "iat").with_token(Some(IAT_TOKEN.into())).with_timeout(Duration::from_millis(300))).unwrap();
        assert_eq!(silent.probe().await, Err(AsrError::Timeout), "a handshake alone is not success");
    }

    /// A take cancelled (its input closed before `Finish`) closes the connection without an end
    /// frame and without `Finished`.
    #[tokio::test]
    async fn iat_a_cancelled_take_closes_and_sends_nothing_more() {
        let (base, seen) = serve_iat(Iat::Dictate).await;
        let client = iat_at(&base);
        let (tx, rx) = mpsc::unbounded_channel();
        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        let watch = seen.clone();
        let drive = async move {
            tx.send(RealtimeInput::Audio(pcm(2 * FRAME_BYTES))).unwrap();
            eventually("two frames", || watch.lock().unwrap().frames.first().is_some_and(|f| f.len() == 2)).await;
            drop(tx);
        };
        let (result, ()) = tokio::join!(client.realtime(None, &[], rx, events_tx), drive);
        result.unwrap();
        eventually("the connection closed", || seen.lock().unwrap().closed == [0]).await;
        assert_eq!(seen.lock().unwrap().close_codes, [Some(1000)]);
        assert_eq!(seen.lock().unwrap().json(0).len(), 2, "no end frame");
        while let Ok(event) = events_rx.try_recv() {
            assert_ne!(event, RealtimeEvent::Finished);
        }
    }

    /// A recorded take is cut into pieces of a session each (one second here), sent at the pace
    /// the audio plays; their texts are joined.
    #[tokio::test]
    async fn iat_a_whole_take_is_cut_into_sessions_at_the_audio_pace() {
        let (base, seen) = serve_iat(Iat::Dictate).await;
        let mut client = iat_at(&base);
        client.session_ms = 1_000;
        let take = wav(40_000, 16_000);
        let started = std::time::Instant::now();
        let transcript = client.transcribe(&take, Some("zh"), &["术语".into()]).await.unwrap();
        let took = started.elapsed();
        assert_eq!(transcript.model, "iat");
        let seen = seen.lock().unwrap();
        let sessions = seen.frames.len();
        assert!(sessions >= 3, "{sessions} sessions for 2.5 s of 1 s each");
        let (mut audio, mut texts, mut frames) = (Vec::new(), Vec::new(), 0);
        for index in 0..sessions {
            let json = seen.json(index);
            let piece: Vec<u8> = json[..json.len() - 1].iter().flat_map(|f| STANDARD.decode(f["data"]["audio"].as_str().unwrap()).unwrap()).collect();
            assert!(piece.len() <= 32_000, "session {index} took {} bytes", piece.len());
            frames += json.len() - 1;
            texts.push(format!("{}。", SESSION_WORDS[index % 4].repeat(json.len() - 1)));
            audio.extend(piece);
        }
        assert_eq!(audio, take[44..], "the whole take, in order");
        assert_eq!(transcript.text, join_sentences(&texts));
        // Each session's frames go out 40 ms apart from its first.
        let paced = Duration::from_millis(40) * u32::try_from(frames - sessions).unwrap();
        assert!(took >= paced, "{took:?} for {frames} frames in {sessions} sessions");
    }

    /// A recorded take's deadline is the client's timeout plus the time the audio takes to play.
    #[tokio::test]
    async fn iat_a_whole_take_times_out_after_its_deadline() {
        let (base, _) = serve_iat(Iat::Silent).await;
        let client = IatAsr::new(AsrConfig::new(&base, "iat").with_token(Some(IAT_TOKEN.into())).with_timeout(Duration::from_millis(200))).unwrap();
        let started = std::time::Instant::now();
        assert_eq!(client.transcribe(&wav(4_800, 16_000), None, &[]).await, Err(AsrError::Timeout));
        let took = started.elapsed();
        assert!(took >= Duration::from_millis(500) && took < Duration::from_secs(5), "{took:?}");
    }

    #[test]
    fn iat_needs_its_three_fields_and_takes_no_glossary() {
        let with = |token: &str| IatAsr::new(AsrConfig::new("https://iat-api.xfyun.cn", "iat").with_token(Some(token.into())));
        assert_eq!(
            with(r#"{"app_id":"a","api_key":"k"}"#).unwrap_err(),
            AsrError::InvalidConfig("the credential is incomplete: missing field `api_secret`".into())
        );
        assert_eq!(
            with(r#"{"app_id":" ","api_key":"k","api_secret":"s"}"#).unwrap_err(),
            AsrError::InvalidConfig("the credential is incomplete: app_id is empty".into())
        );
        assert!(matches!(IatAsr::new(AsrConfig::new("https://iat-api.xfyun.cn", "iat")), Err(AsrError::InvalidConfig(m)) if m.starts_with("the credential")));
        assert!(
            matches!(IatAsr::new(AsrConfig::new("ftp://x", "iat").with_token(Some(IAT_TOKEN.into()))), Err(AsrError::InvalidConfig(m)) if m.starts_with("base_url"))
        );
        let client = with(IAT_TOKEN).unwrap();
        assert!(!client.sends_glossary(&["术语".into()]) && client.streams());
        assert_eq!(client.model(), "iat");
        assert!(matches!(futures_util::FutureExt::now_or_never(client.transcribe(&wav(100, 8_000), None, &[])), Some(Err(AsrError::Audio(_)))));
    }

    // ---- Realtime transcription (rtasr) -----------------------------------------------------

    /// What a scripted realtime transcription service does.
    #[derive(Clone, Copy)]
    enum Rtasr {
        /// Check `signa` (key `rtasr-key-test`) and start; answer each audio frame with the
        /// sentence so far and the end marker with the finished sentence, then close.
        Transcribes,
        /// Answer the handshake with this error (code, description) instead of `started`.
        Refuses(&'static str, &'static str),
        /// Start, then answer the first audio frame with this error.
        Fails(&'static str, &'static str),
        /// Refuse the WebSocket handshake with this status and body.
        RefusesHandshake(u16, &'static str),
        /// Start, then drop the connection at the first audio frame.
        Drops,
        /// Start, then say nothing.
        Silent,
        /// Say nothing, not even `started`.
        Mute,
    }

    fn rtasr_message(action: &str, code: &str, data: &str, desc: &str) -> Message {
        Message::text(json!({ "action": action, "code": code, "data": data, "desc": desc, "sid": "rta-test" }).to_string())
    }

    fn rtasr_result(kind: &str, bg: u64, ed: u64, words: &[&str]) -> Message {
        let ws: Vec<Value> = words.iter().map(|w| json!({ "cw": [{ "w": w, "wp": "n" }], "wb": 0, "we": 0 })).collect();
        let data = json!({ "cn": { "st": { "bg": bg.to_string(), "ed": ed.to_string(), "rt": [{ "ws": ws }], "type": kind } }, "seg_id": 0 });
        rtasr_message("result", "0", &data.to_string(), "success")
    }

    /// `signa` as the service computes it, written apart from the client's.
    fn expected_signa(app_id: &str, ts: &str) -> String {
        let digest = Md5::digest(format!("{app_id}{ts}").as_bytes());
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        let mut mac = Hmac::<Sha1>::new_from_slice(b"rtasr-key-test").unwrap();
        mac.update(hex.as_bytes());
        STANDARD.encode(mac.finalize().into_bytes())
    }

    async fn serve_rtasr(script: Rtasr) -> (String, Arc<Mutex<Seen>>) {
        serve(move |tcp, index, seen| rtasr_session(tcp, index, script, seen)).await
    }

    async fn rtasr_session(tcp: TcpStream, index: usize, script: Rtasr, seen: Arc<Mutex<Seen>>) {
        let refuse = if let Rtasr::RefusesHandshake(status, body) = script { Some((status, body)) } else { None };
        let Some(mut socket) = accept(tcp, index, refuse, &seen).await else { return };
        let target = seen.lock().unwrap().handshakes[index].0.clone();
        let query: std::collections::HashMap<String, String> = Url::parse(&format!("ws://h{target}")).unwrap().query_pairs().into_owned().collect();
        match script {
            Rtasr::Refuses(code, desc) => {
                socket.send(rtasr_message("error", code, "", desc)).await.unwrap();
                return read_on(&mut socket, index, &seen).await;
            }
            Rtasr::Mute => return read_on(&mut socket, index, &seen).await,
            _ if query.get("signa") != Some(&expected_signa(&query["appid"], &query["ts"])) => {
                socket.send(rtasr_message("error", "10110", "", "invalid authorization|illegal signa")).await.unwrap();
                return read_on(&mut socket, index, &seen).await;
            }
            _ => socket.send(rtasr_message("started", "0", "", "success")).await.unwrap(),
        }
        let (mut frames, mut audio) = (0usize, 0usize);
        while let Some(Ok(message)) = socket.next().await {
            if let Message::Close(frame) = message {
                seen.lock().unwrap().close(index, frame.as_ref());
                return;
            }
            seen.lock().unwrap().frames[index].push(message.clone());
            let Message::Binary(bytes) = message else { continue };
            if bytes.as_ref() == RTASR_END {
                if !matches!(script, Rtasr::Transcribes) {
                    continue;
                }
                let end = u64::try_from(audio / BYTES_PER_MS).unwrap();
                socket.send(rtasr_result("0", 10, end, &["你好", "。"])).await.unwrap();
                let _ = socket.close(None).await;
                return read_on(&mut socket, index, &seen).await;
            }
            frames += 1;
            audio += bytes.len();
            match script {
                Rtasr::Drops => return,
                Rtasr::Fails(code, desc) => {
                    socket.send(rtasr_message("error", code, "", desc)).await.unwrap();
                    return read_on(&mut socket, index, &seen).await;
                }
                Rtasr::Transcribes => socket.send(rtasr_result("1", 0, 0, &vec!["你"; frames])).await.unwrap(),
                _ => {}
            }
        }
    }

    fn rtasr_at(base: &str) -> RtasrAsr {
        RtasrAsr::new(AsrConfig::new(base, "rtasr").with_token(Some(RTASR_TOKEN.into()))).unwrap()
    }

    #[test]
    fn rtasr_signs_like_the_documented_example() {
        // The worked example of the API's signa生成, and its request example's URL.
        assert_eq!(signa("595f23df", "d9f4aa7ea6d94faca62cd88a28fd5234", 1_512_041_814), "IrrzsJeOFk1NGfJHW6SkHUoN9CU=");
        let token = r#"{"app_id":"595f23df","api_key":"d9f4aa7ea6d94faca62cd88a28fd5234"}"#;
        let client = RtasrAsr::new(AsrConfig::new("https://rtasr.xfyun.cn", "rtasr").with_token(Some(token.into()))).unwrap();
        assert_eq!(client.signed_url(1_512_041_814).as_str(), "wss://rtasr.xfyun.cn/v1/ws?appid=595f23df&ts=1512041814&signa=IrrzsJeOFk1NGfJHW6SkHUoN9CU%3D");
        assert!(!format!("{client:?}").contains("d9f4aa7e"));
    }

    /// The handshake is signed now with the app id and the key; the audio goes in 1280-byte binary
    /// frames and a shorter last one, then the binary end marker. The sentence so far and the
    /// finished sentence with the service's times come back; the service's close ends the take.
    #[tokio::test]
    async fn rtasr_streams_its_sentences_until_the_service_closes() {
        let (base, seen) = serve_rtasr(Rtasr::Transcribes).await;
        let audio = pcm(3000);
        let (result, events) = live(&rtasr_at(&base), Some("zh"), vec![RealtimeInput::Audio(audio.clone()), RealtimeInput::Finish]).await;
        result.unwrap();
        assert_eq!(
            events,
            vec![
                RealtimeEvent::Partial { text: "你".into() },
                RealtimeEvent::Partial { text: "你你".into() },
                RealtimeEvent::Partial { text: "你你你".into() },
                RealtimeEvent::Sentence { text: "你好。".into(), begin_ms: 10, end_ms: 93 },
                RealtimeEvent::Finished,
            ]
        );
        let seen = seen.lock().unwrap();
        assert_eq!(seen.binary(0), [audio[..1280].to_vec(), audio[1280..2560].to_vec(), audio[2560..].to_vec(), RTASR_END.to_vec()]);
        let url = Url::parse(&format!("ws://h{}", seen.handshakes[0].0)).unwrap();
        assert_eq!(url.path(), "/v1/ws");
        let query: Vec<(String, String)> = url.query_pairs().into_owned().collect();
        assert_eq!(query.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["appid", "ts", "signa"], "no lang, no pd");
        assert_eq!(query[0].1, "app-test");
        let ts: i64 = query[1].1.parse().unwrap();
        assert!((chrono::Utc::now().timestamp() - ts).abs() < 60);
    }

    #[test]
    fn rtasr_messages_parse_or_are_skipped() {
        // The documented result: an intermediate result with its times as text.
        let documented = r#"{"action":"result","code":"0","data":"{\"cn\":{\"st\":{\"bg\":\"820\",\"ed\":\"0\",\"rt\":[{\"ws\":[{\"cw\":[{\"w\":\"啊\",\"wp\":\"n\"}],\"wb\":0,\"we\":0},{\"cw\":[{\"w\":\"喂\",\"wp\":\"n\"}],\"wb\":0,\"we\":0},{\"cw\":[{\"w\":\"！\",\"wp\":\"p\"}],\"wb\":0,\"we\":0},{\"cw\":[{\"w\":\"你好\",\"wp\":\"n\"}],\"wb\":0,\"we\":0}]}],\"type\":\"1\"}},\"seg_id\":5}\n","desc":"success","sid":"rta0000000e@ch312c0e3f6bcc9f0900"}"#;
        assert_eq!(parse_rtasr(documented).unwrap(), RtasrReply::Partial("啊喂！你好".into()));
        let finished = r#"{"action":"result","code":"0","data":"{\"cn\":{\"st\":{\"bg\":1200,\"ed\":3400,\"rt\":[{\"ws\":[{\"cw\":[{\"w\":\"好\"}]}]}],\"type\":\"0\"}}}"}"#;
        assert_eq!(parse_rtasr(finished).unwrap(), RtasrReply::Sentence { text: "好".into(), begin_ms: Some(1200), end_ms: Some(3400) });
        assert_eq!(
            parse_rtasr(r#"{"action":"started","code":"0","data":"","desc":"success","sid":"rta0000000a@ch312c0e3f63609f0900"}"#).unwrap(),
            RtasrReply::Started
        );
        assert_eq!(parse_rtasr(r#"{"action":"result","code":"0","data":""}"#).unwrap(), RtasrReply::Nothing);
        let translated = r#"{"action":"result","code":"0","data":"{\"biz\":\"trans\",\"dst\":\"x\",\"src\":\"床\",\"type\":0,\"bg\":0,\"ed\":4770}"}"#;
        assert_eq!(parse_rtasr(translated).unwrap(), RtasrReply::Nothing);
        assert!(matches!(parse_rtasr("{"), Err(AsrError::BadResponse(_))));
        // The documented failure: a signature it could not verify.
        assert_eq!(
            parse_rtasr(r#"{"action":"error","code":"10110","data":"","desc":"invalid authorization|illegal signa","sid":"rta0000000b@ch312c0e3f65f09f0900"}"#),
            Err(AsrError::Unauthorized { message: "10110: invalid authorization|illegal signa".into() })
        );
    }

    #[test]
    fn rtasr_codes_sort_into_the_error_kinds() {
        let service = |code: &str, desc: &str| AsrError::Service { code: code.into(), message: desc.into() };
        let cases = [
            (
                "10105",
                "illegal access|illegal client_ip: 192.0.2.1",
                AsrError::Unauthorized { message: "10105: illegal access|illegal client_ip: 192.0.2.1".into() },
            ),
            ("10110", "invalid authorization|illegal signa", AsrError::Unauthorized { message: "10110: invalid authorization|illegal signa".into() }),
            ("10110", "no license", AsrError::QuotaExhausted { code: "10110".into(), message: "no license".into(), retry_at_ms: None }),
            ("10800", "over max connect limit", AsrError::RateLimited { retry_after_ms: None, message: "10800: over max connect limit".into() }),
            ("10202", "websocket connect error", AsrError::Network("10202: websocket connect error".into())),
            ("10204", "websocket write error", AsrError::Network("10204: websocket write error".into())),
            ("10205", "websocket read error", AsrError::Network("10205: websocket read error".into())),
            ("10106", "invalid parameter", service("10106", "invalid parameter")),
            ("10107", "illegal parameter", service("10107", "illegal parameter")),
            ("10700", "engine error", service("10700", "engine error")),
            ("16003", "basic component error", service("16003", "basic component error")),
            ("37005", "no audio data", service("37005", "no audio data")),
        ];
        for (code, desc, want) in cases {
            assert_eq!(rtasr_error(code, desc), want, "{code} {desc}");
        }
    }

    #[tokio::test]
    async fn rtasr_failures_end_the_take_with_their_kind() {
        let take = || vec![RealtimeInput::Audio(pcm(FRAME_BYTES)), RealtimeInput::Finish];
        let (base, _) = serve_rtasr(Rtasr::Transcribes).await;
        let wrong = RtasrAsr::new(AsrConfig::new(&base, "rtasr").with_token(Some(r#"{"app_id":"app-test","api_key":"wrong"}"#.into()))).unwrap();
        assert_eq!(live(&wrong, None, take()).await.0, Err(AsrError::Unauthorized { message: "10110: invalid authorization|illegal signa".into() }));
        let (base, _) = serve_rtasr(Rtasr::Fails("10800", "over max connect limit")).await;
        assert_eq!(
            live(&rtasr_at(&base), None, take()).await.0,
            Err(AsrError::RateLimited { retry_after_ms: None, message: "10800: over max connect limit".into() })
        );
        let (base, _) = serve_rtasr(Rtasr::RefusesHandshake(403, r#"{"message":"Your IP address is not allowed"}"#)).await;
        assert_eq!(live(&rtasr_at(&base), None, take()).await.0, Err(AsrError::Unauthorized { message: "Your IP address is not allowed".into() }));
        let (base, _) = serve_rtasr(Rtasr::Drops).await;
        let (result, events) = live(&rtasr_at(&base), None, take()).await;
        assert!(matches!(&result, Err(AsrError::Network(_))), "{result:?}");
        assert!(!events.contains(&RealtimeEvent::Finished));
    }

    /// 测试连接: `started` is the success, after which the session ends without audio; an error
    /// in its place fails it, and so does a service that never says it.
    #[tokio::test]
    async fn rtasr_probe_needs_started() {
        let (base, seen) = serve_rtasr(Rtasr::Transcribes).await;
        rtasr_at(&base).probe().await.unwrap();
        eventually("the session ended", || seen.lock().unwrap().closed == [0]).await;
        assert_eq!(seen.lock().unwrap().binary(0), [RTASR_END.to_vec()]);
        let (base, _) = serve_rtasr(Rtasr::Transcribes).await;
        let wrong = RtasrAsr::new(AsrConfig::new(&base, "rtasr").with_token(Some(r#"{"app_id":"app-test","api_key":"wrong"}"#.into()))).unwrap();
        assert!(matches!(wrong.probe().await, Err(AsrError::Unauthorized { .. })));
        let (base, _) = serve_rtasr(Rtasr::Refuses("10110", "no license")).await;
        assert_eq!(rtasr_at(&base).probe().await, Err(AsrError::QuotaExhausted { code: "10110".into(), message: "no license".into(), retry_at_ms: None }));
        let (base, _) = serve_rtasr(Rtasr::RefusesHandshake(401, r#"{"message":"Unauthorized"}"#)).await;
        assert!(matches!(rtasr_at(&base).probe().await, Err(AsrError::Unauthorized { .. })));
        let (base, _) = serve_rtasr(Rtasr::Mute).await;
        let mute = RtasrAsr::new(AsrConfig::new(&base, "rtasr").with_token(Some(RTASR_TOKEN.into())).with_timeout(Duration::from_millis(300))).unwrap();
        assert_eq!(mute.probe().await, Err(AsrError::Timeout), "a handshake alone is not success");
    }

    #[tokio::test]
    async fn rtasr_a_cancelled_take_closes_and_sends_nothing_more() {
        let (base, seen) = serve_rtasr(Rtasr::Transcribes).await;
        let client = rtasr_at(&base);
        let (tx, rx) = mpsc::unbounded_channel();
        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        let watch = seen.clone();
        let drive = async move {
            tx.send(RealtimeInput::Audio(pcm(2 * FRAME_BYTES))).unwrap();
            eventually("two frames", || watch.lock().unwrap().frames.first().is_some_and(|f| f.len() == 2)).await;
            drop(tx);
        };
        let (result, ()) = tokio::join!(client.realtime(None, &[], rx, events_tx), drive);
        result.unwrap();
        eventually("the connection closed", || seen.lock().unwrap().closed == [0]).await;
        assert_eq!(seen.lock().unwrap().binary(0).len(), 2, "no end marker");
        while let Ok(event) = events_rx.try_recv() {
            assert_ne!(event, RealtimeEvent::Finished);
        }
    }

    /// A recorded take goes out at the pace it plays, on one session of any length.
    #[tokio::test]
    async fn rtasr_a_whole_take_goes_at_the_audio_pace() {
        let (base, seen) = serve_rtasr(Rtasr::Transcribes).await;
        let started = std::time::Instant::now();
        let transcript = rtasr_at(&base).transcribe(&wav(16_000, 16_000), None, &["术语".into()]).await.unwrap();
        assert!(started.elapsed() >= Duration::from_millis(960), "25 frames 40 ms apart: {:?}", started.elapsed());
        assert_eq!((transcript.text.as_str(), transcript.model.as_str()), ("你好。", "rtasr"));
        let frames = seen.lock().unwrap().binary(0);
        assert_eq!(frames.len(), 26);
        assert!(frames[..25].iter().all(|f| f.len() == 1280));
    }

    #[tokio::test]
    async fn rtasr_a_whole_take_times_out_after_its_deadline() {
        let (base, _) = serve_rtasr(Rtasr::Silent).await;
        let client = RtasrAsr::new(AsrConfig::new(&base, "rtasr").with_token(Some(RTASR_TOKEN.into())).with_timeout(Duration::from_millis(200))).unwrap();
        let started = std::time::Instant::now();
        assert_eq!(client.transcribe(&wav(4_800, 16_000), None, &[]).await, Err(AsrError::Timeout));
        let took = started.elapsed();
        assert!(took >= Duration::from_millis(500) && took < Duration::from_secs(5), "{took:?}");
        // No audio, no request.
        assert_eq!(client.transcribe(&wav(0, 16_000), None, &[]).await.map(|t| t.text), Ok(String::new()));
    }

    #[test]
    fn rtasr_needs_its_two_fields() {
        let with = |token: &str| RtasrAsr::new(AsrConfig::new("https://rtasr.xfyun.cn", "rtasr").with_token(Some(token.into())));
        assert_eq!(with(r#"{"app_id":"a"}"#).unwrap_err(), AsrError::InvalidConfig("the credential is incomplete: missing field `api_key`".into()));
        assert_eq!(with(r#"{"app_id":"a","api_key":""}"#).unwrap_err(), AsrError::InvalidConfig("the credential is incomplete: api_key is empty".into()));
        let client = with(RTASR_TOKEN).unwrap();
        assert!(!client.sends_glossary(&["术语".into()]) && client.streams());
    }

    /// A send that fails reports what the service said before the connection went, else the
    /// send's own failure.
    #[tokio::test]
    async fn a_failed_send_reports_what_the_service_said_first() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = Url::parse(&format!("ws://{}/v2/iat", listener.local_addr().unwrap())).unwrap();
        tokio::spawn(async move {
            for answer in [Some(iat_failure(11201, "day limit")), None] {
                let (tcp, _) = listener.accept().await.unwrap();
                tokio::spawn(async move {
                    let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
                    if let Some(answer) = answer {
                        socket.send(answer).await.unwrap();
                    }
                    let _ = socket.close(None).await;
                    while let Some(Ok(_)) = socket.next().await {}
                });
            }
        });
        let network = voltip_net::NetworkRef::default();
        let read = |text: &str| parse_iat(text).err();
        let (mut socket, _) = realtime::connect(realtime::request(&url).unwrap(), &network).await.unwrap();
        let said = said_before(&mut socket, tungstenite::Error::ConnectionClosed, read).await;
        assert_eq!(said, AsrError::QuotaExhausted { code: "11201".into(), message: "day limit".into(), retry_at_ms: None });
        let (mut socket, _) = realtime::connect(realtime::request(&url).unwrap(), &network).await.unwrap();
        let said = said_before(&mut socket, tungstenite::Error::ConnectionClosed, read).await;
        assert!(matches!(said, AsrError::Network(_)), "{said:?}");
    }

    #[test]
    fn query_components_are_percent_encoded() {
        assert_eq!(component("Wed, 10 Jul 2019 07:35:43 GMT"), "Wed%2C%2010%20Jul%202019%2007%3A35%3A43%20GMT");
        assert_eq!(component("a+b/c=d~e_f-g.h"), "a%2Bb%2Fc%3Dd~e_f-g.h");
        assert_eq!(authority(&Url::parse("wss://h.example:8443/v2/iat").unwrap()), "h.example:8443");
        assert_eq!(authority(&Url::parse("wss://h.example:443/v2/iat").unwrap()), "h.example");
    }
}
