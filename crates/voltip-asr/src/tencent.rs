//! Tencent Cloud ASR (docs/dictation.md §3.15): live takes over the signed realtime WebSocket
//! (`/asr/v2/<appid>`, 实时语音识别), recorded takes through the flash file API
//! (`/asr/flash/v1/<appid>`, 录音文件识别极速版); the model is the `engine_model_type` there and the
//! `engine_type` here. The credential is the AppId, SecretId and SecretKey: the SecretKey signs
//! each request (HMAC-SHA1 over the host, the path and the parameters in dictionary order, as
//! base64), and only the SecretId and the signature travel.
//!
//! `Hy-ASR-3.0-preview`, the model of the 2.0 plan, is not one of the flash API's engines and takes
//! at most five minutes a session: its recorded takes go over the realtime WebSocket at the pace
//! they play, cut at quiet points into pieces under five minutes, one session each.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use base64::Engine as _;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt as _, StreamExt as _};
use hmac::{Hmac, KeyInit as _, Mac as _};
use reqwest::StatusCode;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::Deserialize;
use serde_json::Value;
use sha1::Sha1;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::client::Request;
use url::Url;

use crate::client::{self, Transcript, client_builder, map_reqwest, retry_after_ms, truncate_chars};
use crate::config::{AsrConfig, MAX_ERROR_BODY_CHARS};
use crate::credentials;
use crate::dashscope::{join_sentences, service_error, wav_format};
use crate::error::AsrError;
use crate::realtime::{self, BYTES_PER_MS, Pace, Socket};
use crate::split;
use crate::vendor::{self, RealtimeEvent, RealtimeEvents, RealtimeInput, VendorAsr};

/// One audio frame: 200 ms of 16 kHz 16-bit mono, what the doc recommends sending every 200 ms
/// (建议每 200ms 发送 200ms 时长的数据包, 6400 bytes at 16 kHz).
const FRAME_BYTES: usize = 200 * BYTES_PER_MS;
/// The text message that ends a session's audio.
const END: &str = r#"{"type":"end"}"#;
/// The weight of a glossary term in `hotword_list` (1–11; 11 makes a super hot word).
const HOTWORD_WEIGHT: u8 = 10;
/// Hot words a request carries at most.
const MAX_HOTWORDS: usize = 128;
/// The longest hot word, in characters.
const MAX_HOTWORD_CHARS: usize = 30;
/// The longest hot word, in Chinese characters.
const MAX_HOTWORD_HAN: usize = 10;
/// The longest piece of a recorded take sent over the WebSocket: Hy-ASR takes five minutes.
const PIECE_MS: u64 = 290_000;
/// How long a realtime signature is valid (`expired` − `timestamp`).
const SIGNATURE_LIFETIME_SECS: u64 = 86_400;
/// A recorded take goes over the WebSocket at the pace it plays: faster than 1:1 the engine may
/// fail, and more than 3 s of audio a second it refuses (`4000`).
const WEBSOCKET_PACE: Pace = Pace::Realtime { speedup: 1 };
/// How long a failed send waits for the message that explains it.
const EXPLAIN_WAIT: Duration = Duration::from_secs(1);

type Outgoing = SplitSink<Socket, Message>;
type Incoming = SplitStream<Socket>;

/// Tencent Cloud's recognition client: one engine (the model), one AppId and key pair.
pub(crate) struct TencentAsr {
    config: AsrConfig,
    /// The engine, trimmed.
    model: String,
    secret_id: String,
    secret_key: String,
    /// `…/asr/v2/<appid>` as `wss://`.
    live_url: Url,
    /// `…/asr/flash/v1/<appid>`.
    flash_url: Url,
    http: reqwest::Client,
    /// The longest piece of a recorded take over the WebSocket ([`PIECE_MS`]; tests shorten it).
    piece_ms: u64,
}

/// The stored credential (docs/dictation.md §3.15): `{"set":"default","app_id":…,
/// "secret_id":…,"secret_key":…}`.
#[derive(Deserialize)]
struct Fields {
    app_id: String,
    secret_id: String,
    secret_key: String,
}

/// Which of the two APIs answered: their error codes differ in two places.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Api {
    Realtime,
    Flash,
}

impl TencentAsr {
    /// The client of `config`: its base URL, its engine as the model, its credential as the JSON
    /// the core stored.
    pub(crate) fn new(config: AsrConfig) -> Result<Self, AsrError> {
        // The address is checked before the credential: each failure names what to fix.
        vendor::endpoint(&config.base_url, false, "/")?;
        let fields: Fields = credentials::parse(config.token.as_deref())?;
        let app_id = credentials::require(&fields.app_id, "app_id")?;
        if !app_id.bytes().all(|b| b.is_ascii_digit()) {
            return Err(AsrError::InvalidConfig("the credential is incomplete: app_id is not a number".into()));
        }
        let secret_id = credentials::require(&fields.secret_id, "secret_id")?;
        let secret_key = credentials::require(&fields.secret_key, "secret_key")?;
        let live_url = vendor::endpoint(&config.base_url, true, &format!("/asr/v2/{app_id}"))?;
        let flash_url = vendor::endpoint(&config.base_url, false, &format!("/asr/flash/v1/{app_id}"))?;
        let model = config.model.trim().to_owned();
        if model.is_empty() {
            return Err(AsrError::InvalidConfig("model is empty".into()));
        }
        if config.timeout.is_zero() {
            return Err(AsrError::InvalidConfig("timeout must be greater than zero".into()));
        }
        let http = config
            .network
            .apply(client_builder())
            .timeout(config.timeout)
            .user_agent(concat!("voltip-asr/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| AsrError::InvalidConfig(format!("http client: {e}")))?;
        Ok(Self { config, model, secret_id, secret_key, live_url, flash_url, http, piece_ms: PIECE_MS })
    }

    /// The handshake of a realtime session (the doc's 握手阶段): PCM with voice detection, the
    /// glossary as `hotword_list`, a fresh `voice_id` and nonce, signed.
    fn live_request(&self, glossary: &[String]) -> Result<Request, AsrError> {
        let now = unix_now();
        let mut params = BTreeMap::new();
        params.insert("engine_model_type", self.model.clone());
        params.insert("expired", (now + SIGNATURE_LIFETIME_SECS).to_string());
        if let Some(list) = hotword_list(glossary) {
            params.insert("hotword_list", list);
        }
        params.insert("needvad", "1".to_owned());
        params.insert("nonce", nonce().to_string());
        params.insert("secretid", self.secret_id.clone());
        params.insert("timestamp", now.to_string());
        params.insert("voice_format", "1".to_owned());
        params.insert("voice_id", uuid::Uuid::new_v4().to_string());
        let signature = sign(&self.secret_key, &signing_text("", &self.live_url, &params));
        let mut url = self.live_url.clone();
        url.set_query(Some(&format!("{}&signature={}", query(&params), encode(&signature))));
        realtime::request(&url)
    }

    /// Connect a realtime session: the service checks the signature and answers first, `code` 0
    /// or why not (and closes).
    async fn open(&self, glossary: &[String]) -> Result<(Outgoing, Incoming), AsrError> {
        let (socket, _) = realtime::connect(self.live_request(glossary)?, &self.config.network).await?;
        let (outgoing, mut incoming) = socket.split();
        tokio::time::timeout(realtime::CONNECT_TIMEOUT, next_message(&mut incoming)).await.map_err(|_| AsrError::Timeout)??;
        tracing::debug!(path = self.live_url.path(), model = %self.model, "Tencent Cloud realtime session started");
        Ok((outgoing, incoming))
    }

    /// A recorded take through the flash file API: the WAV as the body, the parameters in the URL,
    /// the signature in `Authorization` (the doc's 签名生成说明).
    async fn flash(&self, wav: &[u8], glossary: &[String]) -> Result<String, AsrError> {
        let mut params = BTreeMap::new();
        params.insert("engine_type", self.model.clone());
        if let Some(list) = hotword_list(glossary) {
            params.insert("hotword_list", list);
        }
        params.insert("secretid", self.secret_id.clone());
        params.insert("timestamp", unix_now().to_string());
        params.insert("voice_format", "wav".to_owned());
        let authorization = sign(&self.secret_key, &signing_text("POST", &self.flash_url, &params));
        let mut url = self.flash_url.clone();
        url.set_query(Some(&query(&params)));
        let request = || {
            Ok(self.http.post(url.clone()).header(AUTHORIZATION, authorization.as_str()).header(CONTENT_TYPE, "application/octet-stream").body(wav.to_vec()))
        };
        tracing::debug!(path = self.flash_url.path(), model = %self.model, bytes = wav.len(), "Tencent Cloud flash recognition");
        let response = client::send(request).await?;
        let status = response.status();
        let retry = retry_after_ms(response.headers());
        let body = response.text().await.map_err(map_reqwest)?;
        flash_text(status, retry, &body)
    }
}

#[async_trait]
impl VendorAsr for TencentAsr {
    fn model(&self) -> &str {
        &self.model
    }

    fn sends_glossary(&self, glossary: &[String]) -> bool {
        hotword_list(glossary).is_some()
    }

    /// A recorded take: one flash request, or for Hy-ASR realtime sessions over its pieces.
    async fn transcribe(&self, wav: &[u8], language: Option<&str>, glossary: &[String]) -> Result<Transcript, AsrError> {
        let started = std::time::Instant::now();
        let pcm = pcm_of(wav)?;
        let text = if pcm.is_empty() {
            String::new()
        } else if takes_flash(&self.model) {
            self.flash(wav, glossary).await?
        } else {
            let mut texts = Vec::new();
            for piece in split::quiet_cuts(pcm, self.piece_ms) {
                let piece = realtime::transcribe_whole(self, &pcm[piece], language, glossary, FRAME_BYTES, WEBSOCKET_PACE, self.config.timeout).await?;
                texts.push(piece.text);
            }
            join_sentences(&texts)
        };
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        tracing::debug!(latency_ms, chars = text.chars().count(), "Tencent Cloud transcribed a whole take");
        Ok(Transcript { text, latency_ms, model: self.model.clone() })
    }

    /// The probe of docs/dictation.md §3.15: the realtime handshake; only its answer with `code` 0
    /// is success (the service sends it once the signature checks out). The session then ends.
    async fn probe(&self) -> Result<(), AsrError> {
        let (mut outgoing, _incoming) = self.open(&[]).await?;
        let _ = outgoing.send(Message::text(END)).await;
        let _ = outgoing.close().await;
        Ok(())
    }

    fn streams(&self) -> bool {
        true
    }

    async fn realtime(
        &self,
        _language: Option<&str>,
        glossary: &[String],
        mut input: mpsc::UnboundedReceiver<RealtimeInput>,
        events: RealtimeEvents,
    ) -> Result<(), AsrError> {
        let (mut outgoing, mut incoming) = self.open(glossary).await?;
        let mut slices = Slices::default();
        let mut pending: Vec<u8> = Vec::new();
        // Audio sent, in bytes: the times of a sentence that comes without them.
        let mut sent = 0usize;
        // Once the end marker is out, the service has the client's timeout to finish.
        let mut finish_by: Option<Instant> = None;
        loop {
            tokio::select! {
                item = input.recv(), if finish_by.is_none() => match item {
                    Some(RealtimeInput::Audio(pcm)) => {
                        pending.extend_from_slice(&pcm);
                        while pending.len() >= FRAME_BYTES {
                            let frame: Vec<u8> = pending.drain(..FRAME_BYTES).collect();
                            send(&mut outgoing, &mut incoming, Message::binary(frame)).await?;
                            sent += FRAME_BYTES;
                        }
                    }
                    Some(RealtimeInput::Finish) => {
                        if !pending.is_empty() {
                            sent += pending.len();
                            send(&mut outgoing, &mut incoming, Message::binary(std::mem::take(&mut pending))).await?;
                        }
                        send(&mut outgoing, &mut incoming, Message::text(END)).await?;
                        finish_by = Some(Instant::now() + self.config.timeout);
                    }
                    // A cancelled take: the connection closes, nothing more goes out.
                    None => {
                        let _ = outgoing.close().await;
                        return Ok(());
                    }
                },
                message = next_message(&mut incoming) => {
                    let message = message?;
                    slices.read(&message, played_ms(sent), &events);
                    if message.get("final").and_then(Value::as_i64) == Some(1) {
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

/// What a realtime session's results said so far: each result is the sentence being spoken
/// (`slice_type` 0 or 1) or a finished one (2), with its place in the session (`index`).
#[derive(Debug, Default)]
struct Slices {
    /// Where the last sentence ended.
    end_ms: u64,
    /// The partial last passed on.
    partial: String,
    /// The `index` of the last sentence passed on.
    index: Option<u64>,
}

impl Slices {
    /// Pass on what `message` adds. `sent_ms` is the audio sent so far, the end of a sentence the
    /// service gave no time.
    fn read(&mut self, message: &Value, sent_ms: u64, events: &RealtimeEvents) {
        let Some(result) = message.get("result") else { return };
        let text = result.get("voice_text_str").and_then(Value::as_str).unwrap_or_default().trim();
        let number = |key: &str| result.get(key).and_then(Value::as_u64);
        if number("slice_type") == Some(2) {
            let index = number("index");
            if let (Some(index), Some(done)) = (index, self.index)
                && index <= done
            {
                return;
            }
            self.index = index.or(self.index);
            self.partial.clear();
            let begin_ms = number("start_time").unwrap_or(self.end_ms);
            let end_ms = number("end_time").unwrap_or(sent_ms).max(begin_ms);
            self.end_ms = end_ms;
            if !text.is_empty() {
                let _ = events.send(RealtimeEvent::Sentence { text: text.to_owned(), begin_ms, end_ms });
            }
        } else if !text.is_empty() && text != self.partial {
            text.clone_into(&mut self.partial);
            let _ = events.send(RealtimeEvent::Partial { text: text.to_owned() });
        }
    }
}

/// Send `message`; a send that fails is explained by what the service said before it closed.
async fn send(outgoing: &mut Outgoing, incoming: &mut Incoming, message: Message) -> Result<(), AsrError> {
    let Err(error) = outgoing.send(message).await else { return Ok(()) };
    let error = realtime::map_ws(error);
    Err(match tokio::time::timeout(EXPLAIN_WAIT, next_message(incoming)).await {
        Ok(Err(said)) if !matches!(said, AsrError::Network(_)) => said,
        _ => error,
    })
}

/// The service's next message. One whose `code` is not 0 is its error (the connection then
/// closes); so are a message that is not JSON and a connection that ends.
async fn next_message(incoming: &mut Incoming) -> Result<Value, AsrError> {
    loop {
        let text = match incoming.next().await {
            Some(Ok(Message::Text(text))) => text,
            Some(Ok(Message::Close(_))) | None => return Err(realtime::closed_early()),
            // Pings are answered by tungstenite itself; the service sends no binary.
            Some(Ok(_)) => continue,
            Some(Err(e)) => return Err(realtime::map_ws(e)),
        };
        let value: Value = serde_json::from_str(text.as_str())
            .map_err(|e| AsrError::BadResponse(format!("Tencent Cloud message: {e}: {}", truncate_chars(text.as_str(), MAX_ERROR_BODY_CHARS))))?;
        return match value.get("code").and_then(Value::as_i64) {
            Some(0) | None => Ok(value),
            Some(code) => Err(tencent_error(code, value.get("message").and_then(Value::as_str).unwrap_or_default(), Api::Realtime)),
        };
    }
}

/// The text of a flash answer: every channel's `flash_result[].text`. A `code` other than 0 is
/// the service's error, whatever the HTTP status; an answer without either is sorted by status.
fn flash_text(status: StatusCode, retry_after_ms: Option<u64>, body: &str) -> Result<String, AsrError> {
    let answer: Option<Value> = serde_json::from_str(body).ok();
    if let Some(code) = answer.as_ref().and_then(|a| a.get("code")).and_then(Value::as_i64)
        && code != 0
    {
        let message = answer.as_ref().and_then(|a| a.get("message")).and_then(Value::as_str).unwrap_or_default();
        return Err(tencent_error(code, message, Api::Flash));
    }
    if !status.is_success() {
        return Err(service_error(status, retry_after_ms, body));
    }
    let results = answer
        .as_ref()
        .and_then(|a| a.get("flash_result"))
        .and_then(Value::as_array)
        .ok_or_else(|| AsrError::BadResponse(format!("no flash_result: {}", truncate_chars(body, MAX_ERROR_BODY_CHARS))))?;
    Ok(join_sentences(&results.iter().filter_map(|r| r.get("text").and_then(Value::as_str)).collect::<Vec<_>>()))
}

/// A Tencent Cloud error code as an error a person can act on (the 错误码 tables of both docs):
/// the signature (`4002`), a used-up resource package or an account in arrears (`4004`, `4005`),
/// the concurrency and audio-rate limits (`4006`, `4000`), audio the service cannot read or take
/// (`4007`, `4011`), a timeout (`4008`, the flash API's `5003`), a lost connection (`4009`), a
/// failing server (`5xxx`), and the service's own code for the rest (`4001` parameters, `4003`
/// service not enabled, `4010`, `4012` empty audio, `6001` calls from abroad).
fn tencent_error(code: i64, message: &str, api: Api) -> AsrError {
    let said = voltip_reply::clean(message);
    let with_code = || if said.is_empty() { code.to_string() } else { format!("{code}: {said}") };
    match code {
        4002 => AsrError::Unauthorized { message: with_code() },
        4004 | 4005 => AsrError::QuotaExhausted { code: code.to_string(), message: said, retry_at_ms: None },
        4000 | 4006 => AsrError::RateLimited { retry_after_ms: None, message: with_code() },
        4007 | 4011 => AsrError::Audio(with_code()),
        4008 => AsrError::Timeout,
        4009 => AsrError::Network(with_code()),
        5003 if api == Api::Flash => AsrError::Timeout,
        // The flash API's 5002 is a recognition that failed, the realtime one's a busy machine.
        5002 if api == Api::Flash => AsrError::Server { status: 500, body: with_code() },
        5000..=5002 => AsrError::Server { status: 503, body: with_code() },
        5003..=5999 => AsrError::Server { status: 500, body: with_code() },
        _ => AsrError::Service { code: code.to_string(), message: said },
    }
}

/// The glossary as `hotword_list` (docs/dictation.md §3.15): `词|10` joined by commas, at most
/// [`MAX_HOTWORDS`]. A term with a space (or another whitespace or control character), `|` or `,`
/// is skipped, as is one longer than [`MAX_HOTWORD_CHARS`] characters or [`MAX_HOTWORD_HAN`]
/// Chinese ones; repeats go once.
fn hotword_list(glossary: &[String]) -> Option<String> {
    let mut words: Vec<&str> = Vec::new();
    for term in glossary.iter().map(|t| t.trim()) {
        if words.len() == MAX_HOTWORDS {
            break;
        }
        let fits = !term.is_empty()
            && !term.chars().any(|c| c.is_whitespace() || c.is_control() || c == '|' || c == ',')
            && term.chars().count() <= MAX_HOTWORD_CHARS
            && term.chars().filter(|&c| is_han(c)).count() <= MAX_HOTWORD_HAN;
        if fits && !words.contains(&term) {
            words.push(term);
        }
    }
    (!words.is_empty()).then(|| words.iter().map(|word| format!("{word}|{HOTWORD_WEIGHT}")).collect::<Vec<_>>().join(","))
}

/// A Chinese character (the CJK ideograph blocks).
fn is_han(c: char) -> bool {
    matches!(c as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FA1F)
}

/// What the service signs (the docs' 签名生成): `method`, the host (and a port the URL names), the
/// path, `?`, and the parameters in dictionary order as `key=value` with their values as they are.
fn signing_text(method: &str, url: &Url, params: &BTreeMap<&str, String>) -> String {
    let host = url.host_str().unwrap_or_default();
    let authority = url.port().map_or_else(|| host.to_owned(), |port| format!("{host}:{port}"));
    let raw = params.iter().map(|(key, value)| format!("{key}={value}")).collect::<Vec<_>>().join("&");
    format!("{method}{authority}{}?{raw}", url.path())
}

/// The base64 of `text`'s HMAC-SHA1 under `secret_key`.
fn sign(secret_key: &str, text: &str) -> String {
    let mut mac = Hmac::<Sha1>::new_from_slice(secret_key.as_bytes()).unwrap_or_else(|_| unreachable!("HMAC takes a key of any length"));
    mac.update(text.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

/// The query of `params` as the request carries it: each value percent-encoded as Tencent's SDKs
/// encode it. The service decodes the values and checks the signature over them as they were.
fn query(params: &BTreeMap<&str, String>) -> String {
    params.iter().map(|(key, value)| format!("{key}={}", encode(value))).collect::<Vec<_>>().join("&")
}

/// `value` with every byte but RFC 3986's unreserved characters percent-encoded (`+`, `/` and `=`
/// of a signature included, which the doc requires).
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// Whether the flash API offers `model` as an engine: every model but the Hy-ASR ones of the 2.0
/// plan.
fn takes_flash(model: &str) -> bool {
    !model.to_ascii_lowercase().starts_with("hy-asr")
}

/// The PCM of a 16 kHz 16-bit mono WAV; another rate is refused.
fn pcm_of(wav: &[u8]) -> Result<&[u8], AsrError> {
    let format = wav_format(wav)?;
    if format.sample_rate != realtime::SAMPLE_RATE {
        return Err(AsrError::Audio(format!("the take is {} Hz; Tencent Cloud's 16k engines take 16 kHz", format.sample_rate)));
    }
    Ok(&wav[format.data.0..format.data.1])
}

/// A random positive nonce of at most nine digits (the doc allows ten).
fn nonce() -> u64 {
    u64::from(uuid::Uuid::new_v4().as_fields().0 % 999_999_999) + 1
}

/// Seconds since the Unix epoch.
fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Milliseconds of `bytes` of the session's PCM.
fn played_ms(bytes: usize) -> u64 {
    u64::try_from(bytes / BYTES_PER_MS).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Instant as Clock;

    use serde_json::json;
    use tokio::net::{TcpListener, TcpStream};
    use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request as Handshake, Response as Answer};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::dashscope::tests::wav;

    const APP_ID: &str = "1250000000";
    const SECRET_ID: &str = "test-secret-id";
    const SECRET_KEY: &str = "test-secret-key";

    fn credential(secret_key: &str) -> String {
        json!({ "set": "default", "app_id": APP_ID, "secret_id": SECRET_ID, "secret_key": secret_key }).to_string()
    }

    fn client(base: &str, model: &str) -> TencentAsr {
        TencentAsr::new(AsrConfig::new(base, model).with_token(Some(credential(SECRET_KEY)))).unwrap()
    }

    fn client_with_timeout(base: &str, model: &str, timeout: Duration) -> TencentAsr {
        TencentAsr::new(AsrConfig::new(base, model).with_token(Some(credential(SECRET_KEY))).with_timeout(timeout)).unwrap()
    }

    /// What the service signs, rebuilt from a request apart from the client's code: the query's
    /// values decoded, `signature` left out, the rest in dictionary order after method, host and
    /// path.
    fn rebuilt(method: &str, host: &str, path: &str, query: &str) -> String {
        let mut pairs: Vec<(String, String)> = url::form_urlencoded::parse(query.as_bytes()).into_owned().filter(|(k, _)| k != "signature").collect();
        pairs.sort();
        format!("{method}{host}{path}?{}", pairs.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&"))
    }

    fn hmac_base64(key: &str, text: &str) -> String {
        let mut mac = Hmac::<Sha1>::new_from_slice(key.as_bytes()).unwrap();
        mac.update(text.as_bytes());
        base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
    }

    fn decoded(query: &str) -> BTreeMap<String, String> {
        url::form_urlencoded::parse(query.as_bytes()).into_owned().collect()
    }

    /// What the scripted service does.
    #[derive(Clone)]
    struct Script {
        /// Refuse the handshake with this status and body.
        refuse: Option<(u16, &'static str)>,
        /// The first message once the signature checks out; `null` closes without one.
        handshake: Value,
        /// Messages sent after the client's binary frame of this index.
        after: Vec<(usize, Vec<Value>)>,
        /// Messages sent after the end marker; `{n}` in them is the connection's number, from 1.
        after_end: Vec<Value>,
        /// Drop the connection after the binary frame of this index.
        drop_after: Option<usize>,
    }

    impl Default for Script {
        fn default() -> Self {
            Self {
                refuse: None,
                handshake: json!({ "code": 0, "message": "success", "voice_id": "RnKu9FODFHK5FPpsrN" }),
                after: Vec::new(),
                after_end: Vec::new(),
                drop_after: None,
            }
        }
    }

    /// One connection the scripted service took.
    #[derive(Debug, Default)]
    struct Connection {
        uri: String,
        /// The signature checked out, as the service checks it.
        signed: bool,
        audio: Vec<(usize, Clock)>,
        texts: Vec<String>,
        closed: bool,
    }

    impl Connection {
        fn params(&self) -> BTreeMap<String, String> {
            decoded(self.uri.split_once('?').map_or("", |(_, q)| q))
        }

        fn sizes(&self) -> Vec<usize> {
            self.audio.iter().map(|(n, _)| *n).collect()
        }
    }

    type Seen = Arc<Mutex<Vec<Connection>>>;

    /// A local realtime endpoint following `script`: its `http` base, what it saw, and a message
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
        let refuse = script.refuse;
        let callback = move |request: &Handshake, response: Answer| -> Result<Answer, ErrorResponse> {
            let host = request.headers().get("host").and_then(|v| v.to_str().ok()).unwrap_or_default();
            let query = request.uri().query().unwrap_or_default();
            let signature = decoded(query).remove("signature").unwrap_or_default();
            let mut seen = record.lock().unwrap();
            seen[index].uri = request.uri().to_string();
            seen[index].signed = signature == hmac_base64(SECRET_KEY, &rebuilt("", host, request.uri().path(), query));
            match refuse {
                Some((status, body)) => {
                    let mut refusal = ErrorResponse::new(Some(body.to_owned()));
                    *refusal.status_mut() = tokio_tungstenite::tungstenite::http::StatusCode::from_u16(status).unwrap();
                    Err(refusal)
                }
                None => Ok(response),
            }
        };
        let Ok(socket) = tokio_tungstenite::accept_hdr_async(tcp, callback).await else { return };
        let (mut sink, mut stream) = socket.split();
        let signed = seen.lock().unwrap()[index].signed;
        let first = if signed { script.handshake.clone() } else { json!({ "code": 4002, "message": "鉴权失败", "voice_id": "v" }) };
        if first.is_null() {
            return;
        }
        let _ = sink.send(Message::text(first.to_string())).await;
        if first["code"] != 0 {
            let _ = sink.close().await;
            return;
        }
        let mut count = 0usize;
        while let Some(Ok(message)) = stream.next().await {
            match message {
                Message::Binary(bytes) => {
                    seen.lock().unwrap()[index].audio.push((bytes.len(), Clock::now()));
                    for (after, messages) in &script.after {
                        if *after == count {
                            for message in messages {
                                // A JSON string goes as its raw text: a message that is not JSON.
                                let text = message.as_str().map_or_else(|| message.to_string(), str::to_owned);
                                let _ = sink.send(Message::text(text)).await;
                            }
                        }
                    }
                    if script.drop_after == Some(count) {
                        return;
                    }
                    count += 1;
                }
                Message::Text(text) => {
                    seen.lock().unwrap()[index].texts.push(text.as_str().to_owned());
                    if text.as_str() == END {
                        for message in &script.after_end {
                            let message = message.to_string().replace("{n}", &(index + 1).to_string());
                            let _ = sink.send(Message::text(message)).await;
                        }
                    }
                }
                Message::Close(_) => {
                    seen.lock().unwrap()[index].closed = true;
                    break;
                }
                _ => {}
            }
        }
    }

    /// The scripted service finished a connection.
    async fn wait(ended: &mut mpsc::UnboundedReceiver<()>) {
        tokio::time::timeout(Duration::from_secs(10), ended.recv()).await.unwrap().unwrap();
    }

    /// Run a live session over `audio`, ended with `Finish` or, without it, cancelled.
    async fn live(client: &TencentAsr, glossary: &[String], audio: &[Vec<u8>], finish: bool) -> (Result<(), AsrError>, Vec<RealtimeEvent>) {
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

    /// A result message: `slice_type`, `index`, the text, its times.
    fn result(slice: u64, index: Option<u64>, text: &str, times: Option<(u64, u64)>) -> Value {
        let mut result = json!({ "slice_type": slice, "voice_text_str": text, "word_size": 0, "word_list": [] });
        if let Some(index) = index {
            result["index"] = json!(index);
        }
        if let Some((start, end)) = times {
            result["start_time"] = json!(start);
            result["end_time"] = json!(end);
        }
        json!({ "code": 0, "message": "success", "voice_id": "RnKu9FODFHK5FPpsrN", "message_id": "RnKu9FODFHK5FPpsrN_11_0", "result": result })
    }

    fn last() -> Value {
        json!({ "code": 0, "message": "success", "voice_id": "RnKu9FODFHK5FPpsrN", "message_id": "RnKu9FODFHK5FPpsrN_241", "final": 1 })
    }

    fn sentence(text: &str, begin_ms: u64, end_ms: u64) -> RealtimeEvent {
        RealtimeEvent::Sentence { text: text.into(), begin_ms, end_ms }
    }

    fn partial(text: &str) -> RealtimeEvent {
        RealtimeEvent::Partial { text: text.into() }
    }

    /// The realtime doc's worked example (签名生成, steps 1–3): its signing text, signature and
    /// URL-encoded signature, with the doc's own (masked) values.
    #[test]
    fn the_signature_follows_the_documented_example() {
        let url = Url::parse("wss://asr.cloud.tencent.com/asr/v2/125922***").unwrap();
        let params: BTreeMap<&str, String> = [
            ("engine_model_type", "16k_zh"),
            ("expired", "1673494772"),
            ("needvad", "1"),
            ("nonce", "1673408372"),
            ("secretid", "*****Qq1zhZMN8dv0******"),
            ("timestamp", "1673408372"),
            ("voice_format", "1"),
            ("voice_id", "c64385ee-3e5c-4fc5-bbfd-7c71addb35b0"),
        ]
        .into_iter()
        .map(|(k, v)| (k, v.to_owned()))
        .collect();
        let text = signing_text("", &url, &params);
        assert_eq!(
            text,
            "asr.cloud.tencent.com/asr/v2/125922***?engine_model_type=16k_zh&expired=1673494772&needvad=1&nonce=1673408372&secretid=*****Qq1zhZMN8dv0******&timestamp=1673408372&voice_format=1&voice_id=c64385ee-3e5c-4fc5-bbfd-7c71addb35b0"
        );
        let signature = sign("*****SkqpeHgqmSz*****", &text);
        assert_eq!(signature, "G8jDQBRg1JfeBi/YnTjyjekxfDA=");
        assert_eq!(encode(&signature), "G8jDQBRg1JfeBi%2FYnTjyjekxfDA%3D");
    }

    /// The flash API's signing text (`POST`, host, path, sorted raw values) and its signature,
    /// against a vector computed apart (Python's `hmac`); the values travel percent-encoded as
    /// Tencent's SDKs encode them; a port the URL names is signed.
    #[test]
    fn the_flash_signature_matches_an_independent_vector() {
        let url = Url::parse("https://asr.cloud.tencent.com/asr/flash/v1/1250000000").unwrap();
        let params: BTreeMap<&str, String> = [
            ("voice_format", "wav"),
            ("engine_type", "16k_zh_en"),
            ("secretid", "example-secret-id"),
            ("timestamp", "1700000000"),
            ("hotword_list", "腾讯云|10,Voltip|10"),
        ]
        .into_iter()
        .map(|(k, v)| (k, v.to_owned()))
        .collect();
        let text = signing_text("POST", &url, &params);
        assert_eq!(
            text,
            "POSTasr.cloud.tencent.com/asr/flash/v1/1250000000?engine_type=16k_zh_en&hotword_list=腾讯云|10,Voltip|10&secretid=example-secret-id&timestamp=1700000000&voice_format=wav"
        );
        assert_eq!(sign("example-secret-key", &text), "3mzXbmAc53LNBOhEWf1Oqm9OYv8=");
        assert_eq!(
            query(&params),
            "engine_type=16k_zh_en&hotword_list=%E8%85%BE%E8%AE%AF%E4%BA%91%7C10%2CVoltip%7C10&secretid=example-secret-id&timestamp=1700000000&voice_format=wav"
        );
        assert_eq!(encode("a b+c~*-_.Z9"), "a%20b%2Bc~%2A-_.Z9");
        let local = Url::parse("http://127.0.0.1:8080/x/asr/flash/v1/1").unwrap();
        assert_eq!(signing_text("POST", &local, &BTreeMap::new()), "POST127.0.0.1:8080/x/asr/flash/v1/1?");
    }

    /// A live session: signed as the service checks it, with the parameters of the doc's 握手阶段
    /// and the cleaned glossary; audio reframed into 200 ms frames, the rest before the end
    /// marker; partials, each finished sentence once with its times, `Finished` at `final`.
    #[tokio::test]
    async fn a_live_session_is_signed_as_the_service_checks_it_and_passes_on_its_results() {
        let script = Script {
            after: vec![
                (0, vec![result(0, Some(0), "腾讯", Some((0, 400)))]),
                (
                    1,
                    vec![
                        result(1, Some(0), "腾讯云", Some((0, 800))),
                        result(1, Some(0), "腾讯云", Some((0, 800))),
                        result(2, Some(0), "腾讯云语音识别。", Some((0, 1240))),
                    ],
                ),
            ],
            after_end: vec![result(2, Some(1), "欢迎使用。", Some((1300, 2000))), last()],
            ..Script::default()
        };
        let (base, seen, mut ended) = serve(script).await;
        let long = "x".repeat(31);
        let glossary: Vec<String> =
            ["腾讯云", "Voltip", "腾讯云", "has space", "a|b", "a,b", "一二三四五六七八九十一", long.as_str(), ""].iter().map(|t| (*t).to_owned()).collect();
        let (outcome, said) = live(&client(&base, "16k_zh_en"), &glossary, &[vec![1; 1000], vec![2; 9000], vec![3; 3000]], true).await;
        assert_eq!(outcome, Ok(()));
        assert_eq!(
            said,
            vec![partial("腾讯"), partial("腾讯云"), sentence("腾讯云语音识别。", 0, 1240), sentence("欢迎使用。", 1300, 2000), RealtimeEvent::Finished]
        );
        wait(&mut ended).await;
        let seen = seen.lock().unwrap();
        let connection = &seen[0];
        assert!(connection.signed, "the service's own check of the signature: {}", connection.uri);
        assert_eq!(connection.uri.split('?').next(), Some("/asr/v2/1250000000"));
        let params = connection.params();
        assert_eq!(
            params.keys().map(String::as_str).collect::<Vec<_>>(),
            ["engine_model_type", "expired", "hotword_list", "needvad", "nonce", "secretid", "signature", "timestamp", "voice_format", "voice_id"]
        );
        assert_eq!(params["engine_model_type"], "16k_zh_en");
        assert_eq!((params["voice_format"].as_str(), params["needvad"].as_str()), ("1", "1"), "PCM, voice detection");
        assert_eq!(params["secretid"], SECRET_ID);
        assert_eq!(params["hotword_list"], "腾讯云|10,Voltip|10");
        let (timestamp, expired): (u64, u64) = (params["timestamp"].parse().unwrap(), params["expired"].parse().unwrap());
        assert_eq!(expired - timestamp, 86_400);
        assert!(unix_now().abs_diff(timestamp) < 60);
        let nonce: u64 = params["nonce"].parse().unwrap();
        assert!((1..=999_999_999).contains(&nonce), "{nonce}");
        assert!(uuid::Uuid::parse_str(&params["voice_id"]).is_ok());
        let raw = connection.uri.split_once('?').unwrap().1;
        assert!(raw.contains("hotword_list=%E8%85%BE%E8%AE%AF%E4%BA%91%7C10%2CVoltip%7C10"), "{raw}");
        let order: Vec<&str> = raw.split('&').map(|pair| pair.split('=').next().unwrap()).collect();
        assert_eq!(order.last(), Some(&"signature"), "the signature last, after the sorted parameters");
        assert!(order[..order.len() - 1].is_sorted());
        assert_eq!(connection.sizes(), [6400, 6400, 200], "200 ms frames, the rest before the end");
        assert_eq!(connection.texts, [END]);
        assert!(connection.closed);
    }

    /// A finished sentence is passed on once (by its `index`); without times it runs from the
    /// previous end to the audio sent; empty results and messages without one say nothing.
    #[test]
    fn slices_are_passed_on_once_with_their_times() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut drain = || std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
        let mut slices = Slices::default();
        slices.read(&result(0, Some(0), "", None), 0, &tx);
        slices.read(&result(1, Some(0), "一", None), 100, &tx);
        slices.read(&result(1, Some(0), "一", None), 200, &tx);
        slices.read(&result(2, Some(0), "一。", None), 600, &tx);
        assert_eq!(drain(), vec![partial("一"), sentence("一。", 0, 600)]);
        slices.read(&result(2, Some(0), "一。", Some((0, 600))), 700, &tx);
        assert_eq!(drain(), vec![], "the same sentence again");
        slices.read(&result(2, Some(1), "二。", Some((650, 900))), 1000, &tx);
        slices.read(&result(2, None, "三。", None), 1200, &tx);
        slices.read(&result(2, Some(3), "", Some((1300, 1400))), 1500, &tx);
        slices.read(&result(2, Some(4), "四。", None), 1600, &tx);
        slices.read(&last(), 1600, &tx);
        assert_eq!(drain(), vec![sentence("二。", 650, 900), sentence("三。", 900, 1200), sentence("四。", 1400, 1600)]);
        // A partial after a sentence is news again.
        slices.read(&result(1, Some(5), "一", None), 1700, &tx);
        assert_eq!(drain(), vec![partial("一")]);
    }

    /// The 错误码 tables of both docs.
    #[test]
    fn error_codes_are_sorted_as_the_docs_describe() {
        use Api::{Flash, Realtime};
        let quota = |code: &str, message: &str| AsrError::QuotaExhausted { code: code.into(), message: message.into(), retry_at_ms: None };
        let service = |code: &str, message: &str| AsrError::Service { code: code.into(), message: message.into() };
        let limited = |message: &str| AsrError::RateLimited { retry_after_ms: None, message: message.into() };
        let server = |status: u16, body: &str| AsrError::Server { status, body: body.into() };
        let cases = [
            (4000, Realtime, "音频数据发送过多", limited("4000: 音频数据发送过多")),
            (4001, Realtime, "参数不合法", service("4001", "参数不合法")),
            (4002, Realtime, "鉴权失败", AsrError::Unauthorized { message: "4002: 鉴权失败".into() }),
            (4002, Flash, "", AsrError::Unauthorized { message: "4002".into() }),
            (4003, Flash, "AppID 服务未开通", service("4003", "AppID 服务未开通")),
            (4004, Realtime, "资源包耗尽", quota("4004", "资源包耗尽")),
            (4005, Flash, "账户欠费停止服务", quota("4005", "账户欠费停止服务")),
            (4006, Realtime, "账号当前调用并发超限", limited("4006: 账号当前调用并发超限")),
            (4007, Realtime, "音频解码失败", AsrError::Audio("4007: 音频解码失败".into())),
            (4008, Realtime, "客户端超过15秒未发送音频数据", AsrError::Timeout),
            (4008, Flash, "客户端数据上传超时", AsrError::Timeout),
            (4009, Flash, "客户端连接断开", AsrError::Network("4009: 客户端连接断开".into())),
            (4010, Realtime, "客户端上传未知文本消息", service("4010", "客户端上传未知文本消息")),
            (4011, Flash, "音频数据太大", AsrError::Audio("4011: 音频数据太大".into())),
            (4012, Flash, "音频数据为空", service("4012", "音频数据为空")),
            (5000, Realtime, "机器负载过高", server(503, "5000: 机器负载过高")),
            (5002, Realtime, "机器负载过高", server(503, "5002: 机器负载过高")),
            (5002, Flash, "音频识别失败", server(500, "5002: 音频识别失败")),
            (5003, Flash, "音频识别超时", AsrError::Timeout),
            (5003, Realtime, "x", server(500, "5003: x")),
            (6001, Realtime, "境外调用请前往腾讯云国际站开通服务", service("6001", "境外调用请前往腾讯云国际站开通服务")),
        ];
        for (code, api, message, want) in cases {
            assert_eq!(tencent_error(code, message, api), want, "{code} {api:?}");
        }
    }

    /// The service's error code ends a session with the error it names, at the handshake or later;
    /// a dropped connection is a network error.
    #[tokio::test]
    async fn errors_and_dropped_connections_end_a_session_with_their_kind() {
        let script = Script {
            after: vec![(0, vec![json!({ "code": 4004, "message": "资源包耗尽，请开通后付费或者购买资源包。" })])], ..Script::default()
        };
        let (base, _, _) = serve(script).await;
        let (outcome, said) = live(&client(&base, "16k_zh_en"), &[], &[vec![0; 6400], vec![0; 6400]], true).await;
        assert_eq!(
            outcome,
            Err(AsrError::QuotaExhausted {
                code: "4004".into(), message: "资源包耗尽，请开通后付费或者购买资源包。".into(), retry_at_ms: None
            })
        );
        assert!(said.is_empty());
        let (base, _, _) = serve(Script { handshake: json!({ "code": 4006, "message": "账号当前调用并发超限" }), ..Script::default() }).await;
        let (outcome, _) = live(&client(&base, "16k_zh_en"), &[], &[vec![0; 6400]], true).await;
        assert_eq!(outcome, Err(AsrError::RateLimited { retry_after_ms: None, message: "4006: 账号当前调用并发超限".into() }));
        let (base, _, _) = serve(Script { drop_after: Some(0), ..Script::default() }).await;
        let (outcome, said) = live(&client(&base, "16k_zh_en"), &[], &[vec![0; 6400], vec![0; 6400]], true).await;
        assert!(matches!(outcome, Err(AsrError::Network(_))), "{outcome:?}");
        assert!(said.is_empty());
        // A message that is not JSON.
        let (base, _, _) = serve(Script { after: vec![(0, vec![json!("not json")])], ..Script::default() }).await;
        let (outcome, _) = live(&client(&base, "16k_zh_en"), &[], &[vec![0; 6400]], true).await;
        assert!(matches!(outcome, Err(AsrError::BadResponse(_))), "{outcome:?}");
    }

    /// docs/dictation.md §3.15: the probe connects and succeeds only on the handshake's `code` 0,
    /// then ends the session; a wrong key, a service not enabled, arrears, a refused handshake
    /// and a service that closes without answering are the failures they name.
    #[tokio::test]
    async fn the_probe_succeeds_on_the_handshake_s_code_0_then_ends_the_session() {
        let (base, seen, mut ended) = serve(Script::default()).await;
        assert_eq!(client(&base, "16k_zh_en").probe().await, Ok(()));
        wait(&mut ended).await;
        {
            let seen = seen.lock().unwrap();
            assert!(seen[0].signed);
            assert_eq!(seen[0].texts, [END], "the session ends at once");
            assert!(seen[0].audio.is_empty() && seen[0].closed);
            assert!(!seen[0].params().contains_key("hotword_list"));
        }
        let wrong = TencentAsr::new(AsrConfig::new(&base, "16k_zh_en").with_token(Some(credential("wrong-key")))).unwrap();
        assert_eq!(wrong.probe().await, Err(AsrError::Unauthorized { message: "4002: 鉴权失败".into() }));
        let answers = [
            (
                json!({ "code": 4003, "message": "AppID 服务未开通，请在控制台开通服务。" }),
                AsrError::Service { code: "4003".into(), message: "AppID 服务未开通，请在控制台开通服务。".into() },
            ),
            (
                json!({ "code": 4005, "message": "账户欠费停止服务，请及时充值。" }),
                AsrError::QuotaExhausted { code: "4005".into(), message: "账户欠费停止服务，请及时充值。".into(), retry_at_ms: None },
            ),
        ];
        for (handshake, want) in answers {
            let (base, _, _) = serve(Script { handshake, ..Script::default() }).await;
            assert_eq!(client(&base, "16k_zh_en").probe().await, Err(want));
        }
        let (base, _, _) = serve(Script { refuse: Some((403, "forbidden")), ..Script::default() }).await;
        assert_eq!(client(&base, "16k_zh_en").probe().await, Err(AsrError::Unauthorized { message: "forbidden".into() }));
        let (base, _, _) = serve(Script { handshake: Value::Null, ..Script::default() }).await;
        assert!(matches!(client(&base, "16k_zh_en").probe().await, Err(AsrError::Network(_))));
    }

    /// A cancelled take: the input closes before `Finish`; the connection closes at once without
    /// the end marker, and the session says nothing more.
    #[tokio::test]
    async fn a_cancelled_session_closes_without_the_end_marker() {
        let (base, seen, mut ended) = serve(Script::default()).await;
        let (outcome, said) = live(&client(&base, "16k_zh_en"), &[], &[vec![0; 7000]], false).await;
        assert_eq!((outcome, said), (Ok(()), vec![]));
        wait(&mut ended).await;
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].sizes(), [6400], "nothing after the cancel");
        assert!(seen[0].texts.is_empty());
        assert!(seen[0].closed);
    }

    /// A recorded take on a flash engine: one POST of the WAV as it is, `voice_format=wav`, the
    /// glossary as `hotword_list`, signed in `Authorization` as the service checks it; the text of
    /// `flash_result`.
    #[tokio::test]
    async fn a_whole_take_goes_to_the_flash_api_signed_with_its_wav() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/asr/flash/v1/1250000000"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "request_id": "6098aecab9c686fbfd35adb0", "code": 0, "message": "", "audio_duration": 2386,
                "flash_result": [{ "text": "腾讯云智能语音欢迎您。", "channel_id": 0, "sentence_list": [] }]
            })))
            .mount(&server)
            .await;
        let take = wav(16_000, 16_000);
        let t = client(&server.uri(), "16k_zh_en").transcribe(&take, Some("zh"), &["腾讯云".into(), "Voltip".into()]).await.unwrap();
        assert_eq!((t.text.as_str(), t.model.as_str()), ("腾讯云智能语音欢迎您。", "16k_zh_en"));
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.body, take, "the WAV as it is");
        assert_eq!(request.headers.get("content-type").unwrap(), "application/octet-stream");
        let query = request.url.query().unwrap();
        let params = decoded(query);
        assert_eq!(params.keys().map(String::as_str).collect::<Vec<_>>(), ["engine_type", "hotword_list", "secretid", "timestamp", "voice_format"]);
        assert_eq!((params["engine_type"].as_str(), params["voice_format"].as_str(), params["secretid"].as_str()), ("16k_zh_en", "wav", SECRET_ID));
        assert_eq!(params["hotword_list"], "腾讯云|10,Voltip|10");
        assert!(query.contains("hotword_list=%E8%85%BE%E8%AE%AF%E4%BA%91%7C10%2CVoltip%7C10"), "{query}");
        // The host the request was addressed to, port included (wiremock's own URL names localhost).
        let host = request.headers.get("host").unwrap().to_str().unwrap();
        assert!(host.starts_with("127.0.0.1:"), "{host}");
        let authorization = request.headers.get("authorization").unwrap().to_str().unwrap();
        assert_eq!(authorization, hmac_base64(SECRET_KEY, &rebuilt("POST", host, request.url.path(), query)));
    }

    /// A flash answer: its `code` first, whatever the HTTP status; then the status; then
    /// `flash_result`.
    #[tokio::test]
    async fn flash_answers_are_read_or_sorted() {
        let ok = StatusCode::OK;
        assert_eq!(flash_text(ok, None, r#"{"code":0,"flash_result":[{"text":"你好。"},{"text":"Hello."}]}"#), Ok("你好。Hello.".into()));
        assert_eq!(flash_text(ok, None, r#"{"code":0,"flash_result":[]}"#), Ok(String::new()), "nothing heard");
        assert_eq!(flash_text(ok, None, r#"{"code":4002,"message":"鉴权失败"}"#), Err(AsrError::Unauthorized { message: "4002: 鉴权失败".into() }));
        assert_eq!(
            flash_text(StatusCode::FORBIDDEN, None, r#"{"code":4004,"message":"资源包耗尽"}"#),
            Err(AsrError::QuotaExhausted { code: "4004".into(), message: "资源包耗尽".into(), retry_at_ms: None })
        );
        assert_eq!(flash_text(ok, None, r#"{"code":5003,"message":"音频识别超时"}"#), Err(AsrError::Timeout));
        assert_eq!(flash_text(StatusCode::BAD_GATEWAY, None, "bad gateway"), Err(AsrError::Server { status: 502, body: "bad gateway".into() }));
        assert_eq!(flash_text(StatusCode::UNAUTHORIZED, None, ""), Err(AsrError::Unauthorized { message: String::new() }));
        assert_eq!(
            flash_text(StatusCode::TOO_MANY_REQUESTS, Some(2000), ""),
            Err(AsrError::RateLimited { retry_after_ms: Some(2000), message: String::new() })
        );
        assert!(matches!(flash_text(ok, None, "not json"), Err(AsrError::BadResponse(_))));
        assert!(matches!(flash_text(ok, None, r#"{"code":0,"message":""}"#), Err(AsrError::BadResponse(m)) if m.contains("flash_result")));
        // Through the client, a refusal keeps its kind.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "code": 4006, "message": "账号当前调用并发超限", "request_id": "r" })))
            .mount(&server)
            .await;
        let err = client(&server.uri(), "16k_zh").transcribe(&wav(1600, 16_000), None, &[]).await.unwrap_err();
        assert_eq!(err, AsrError::RateLimited { retry_after_ms: None, message: "4006: 账号当前调用并发超限".into() });
    }

    /// Hy-ASR is no flash engine: its recorded take goes over the realtime WebSocket, cut into
    /// pieces (shortened here), one session each in order, at the pace it plays; the pieces'
    /// sentences joined.
    #[tokio::test]
    async fn hy_asr_takes_go_over_the_websocket_in_pieces_at_the_pace_they_play() {
        let script = Script { after_end: vec![result(2, Some(0), "第{n}段。", Some((0, 300))), last()], ..Script::default() };
        let (base, seen, mut ended) = serve(script).await;
        let mut client = client(&base, "Hy-ASR-3.0-preview");
        client.piece_ms = 400;
        let take = wav(16_000, 16_000);
        let pieces = split::quiet_cuts(&take[44..], 400);
        assert_eq!(pieces.iter().map(ExactSizeIterator::len).collect::<Vec<_>>(), [9600, 9600, 12800]);
        let started = Clock::now();
        let t = client.transcribe(&take, None, &["腾讯云".into()]).await.unwrap();
        let took = started.elapsed();
        assert_eq!((t.text.as_str(), t.model.as_str()), ("第1段。第2段。第3段。", "Hy-ASR-3.0-preview"));
        for _ in 0..3 {
            wait(&mut ended).await;
        }
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 3, "one session a piece");
        for (connection, piece) in seen.iter().zip(&pieces) {
            assert!(connection.signed);
            assert_eq!(connection.sizes().iter().sum::<usize>(), piece.len());
            assert!(connection.sizes().iter().all(|&n| n <= 6400));
            assert_eq!(connection.texts, [END]);
            let params = connection.params();
            assert_eq!((params["engine_model_type"].as_str(), params["hotword_list"].as_str()), ("Hy-ASR-3.0-preview", "腾讯云|10"));
        }
        // 1:1: each piece's second frame waits 200 ms after its first.
        assert!(took >= Duration::from_millis(600), "{took:?}");
    }

    /// The limit itself: a long take is cut into pieces of at most 290 s, Hy-ASR's five minutes
    /// with room to spare.
    #[test]
    fn a_long_hy_asr_take_is_cut_into_pieces_under_five_minutes() {
        let pcm = vec![0u8; 11 * 60 * 1000 * BYTES_PER_MS];
        let pieces = split::quiet_cuts(&pcm, PIECE_MS);
        assert!(pieces.len() >= 3, "{}", pieces.len());
        assert!(pieces.iter().all(|p| p.len() <= 290_000 * BYTES_PER_MS));
        assert_eq!((pieces.first().map(|p| p.start), pieces.last().map(|p| p.end)), (Some(0), Some(pcm.len())));
        assert!(takes_flash("16k_zh_en") && takes_flash("16k_zh") && !takes_flash("Hy-ASR-3.0-preview") && !takes_flash("hy-asr-4"));
    }

    /// Real time, not a paused clock: the servers are real sockets. A service that never answers
    /// times a flash take out, a Hy-ASR take, and a live session once its end marker is out.
    #[tokio::test]
    async fn a_silent_service_times_out() {
        let short = Duration::from_millis(300);
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(5))).mount(&server).await;
        let err = client_with_timeout(&server.uri(), "16k_zh_en", short).transcribe(&wav(1600, 16_000), None, &[]).await.unwrap_err();
        assert_eq!(err, AsrError::Timeout);
        let (base, _, _) = serve(Script::default()).await;
        let err = client_with_timeout(&base, "Hy-ASR-3.0-preview", short).transcribe(&wav(1600, 16_000), None, &[]).await.unwrap_err();
        assert_eq!(err, AsrError::Timeout);
        let (outcome, _) = live(&client_with_timeout(&base, "16k_zh_en", short), &[], &[vec![0; 3200]], true).await;
        assert_eq!(outcome, Err(AsrError::Timeout));
    }

    /// docs/dictation.md §3.15: `hotword_list` is `词|10` joined by commas, at most 128; terms
    /// with a space, `|` or `,`, longer than 30 characters or 10 Chinese ones are skipped, repeats
    /// go once.
    #[test]
    fn the_glossary_goes_out_as_hotword_list_within_its_limits() {
        let list = |terms: &[&str]| hotword_list(&terms.iter().map(|t| (*t).to_owned()).collect::<Vec<_>>());
        assert_eq!(list(&[]), None);
        assert_eq!(list(&[" ", "has space", "a|b", "a,b", "tab\tx"]), None);
        let thirty = "x".repeat(30);
        assert_eq!(
            list(&["腾讯云", " Voltip ", "腾讯云", "一二三四五六七八九十", "一二三四五六七八九十一", &thirty, &"y".repeat(31), "C++"]),
            Some(format!("腾讯云|10,Voltip|10,一二三四五六七八九十|10,{thirty}|10,C++|10"))
        );
        let many: Vec<String> = (0..200).map(|i| format!("t{i}")).collect();
        let sent = hotword_list(&many).unwrap();
        assert_eq!((sent.split(',').count(), sent.split(',').next_back()), (128, Some("t127|10")));
        let client = client("https://asr.cloud.tencent.com", "16k_zh_en");
        assert!(client.sends_glossary(&["腾讯云".into()]));
        assert!(!client.sends_glossary(&[]) && !client.sends_glossary(&["has space".into()]));
    }

    /// A client fails on what it cannot use, naming the field, never its value: the address
    /// first, then the credential; another sample rate never connects; a proxy address is the
    /// host and path the requests are signed with.
    #[tokio::test]
    async fn configurations_and_audio_it_cannot_use_are_refused() {
        let error = |base: &str, token: Option<String>| {
            TencentAsr::new(AsrConfig::new(base, "16k_zh_en").with_token(token)).err().map(|e| e.to_string()).unwrap_or_default()
        };
        assert!(error("ftp://asr.cloud.tencent.com", None).contains("base_url"));
        assert!(error("https://asr.cloud.tencent.com", None).contains("the credential is missing"));
        let incomplete = [
            (json!({ "set": "default", "app_id": APP_ID, "secret_id": "secret-value-1" }), "missing field `secret_key`"),
            (json!({ "set": "default", "app_id": APP_ID, "secret_id": " ", "secret_key": "secret-value-1" }), "secret_id is empty"),
            (json!({ "set": "default", "app_id": "12a", "secret_id": "secret-value-1", "secret_key": "secret-value-2" }), "app_id is not a number"),
            (json!({ "set": "default", "app_id": 1_250_000_000, "secret_id": "secret-value-1", "secret_key": "k" }), "not an object of text fields"),
        ];
        for (token, says) in incomplete {
            let message = error("https://asr.cloud.tencent.com", Some(token.to_string()));
            assert!(message.contains("the credential is incomplete") && message.contains(says), "{token}: {message}");
            assert!(!message.contains("secret-value"), "{message}");
        }
        assert!(
            TencentAsr::new(AsrConfig::new("https://asr.cloud.tencent.com", " ").with_token(Some(credential(SECRET_KEY))))
                .err()
                .is_some_and(|e| e.to_string().contains("model is empty"))
        );
        let proxied = TencentAsr::new(AsrConfig::new("https://proxy.example.test/tencent/", " 16k_zh ").with_token(Some(credential(SECRET_KEY)))).unwrap();
        assert_eq!(proxied.model(), "16k_zh");
        assert_eq!(proxied.live_url.as_str(), "wss://proxy.example.test/tencent/asr/v2/1250000000");
        assert_eq!(proxied.flash_url.as_str(), "https://proxy.example.test/tencent/asr/flash/v1/1250000000");
        assert!(signing_text("", &proxied.live_url, &BTreeMap::new()).starts_with("proxy.example.test/tencent/asr/v2/1250000000?"));
        assert!(proxied.streams());
        // 8 kHz is refused before any request (nobody listens at port 9).
        let err = client("http://127.0.0.1:9", "16k_zh_en").transcribe(&wav(800, 8_000), None, &[]).await.unwrap_err();
        assert!(matches!(&err, AsrError::Audio(m) if m.contains("8000 Hz")), "{err:?}");
        assert_eq!(client("http://127.0.0.1:9", "16k_zh_en").transcribe(&wav(0, 16_000), None, &[]).await.unwrap().text, "", "an empty take sends nothing");
    }
}
