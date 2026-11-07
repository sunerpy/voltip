//! What the realtime protocols share (docs/dictation.md §3.15): the WebSocket through the proxy,
//! the longest packet of audio a live session sends, a recorded take fed to a realtime session at
//! the pace its service asks for, the deadline that pace needs, and the take's sentences joined
//! into its text.

use std::future::Future;
use std::time::{Duration, Instant};

use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::client::{Request, Response};
use tokio_tungstenite::tungstenite::{self};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::client::Transcript;
use crate::dashscope::{join_sentences, service_error};
use crate::error::AsrError;
use crate::vendor::{RealtimeEvent, RealtimeEvents, RealtimeInput, VendorAsr};

/// Deadline of the TCP connection and the WebSocket handshake.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// The rate of the PCM every realtime session takes: 16 kHz, 16-bit mono.
pub const SAMPLE_RATE: u32 = 16_000;
/// Bytes of one millisecond of that PCM.
pub const BYTES_PER_MS: usize = 32;
/// The longest packet of audio a live session sends: 100 ms (docs/dictation.md §3.15, 2026-11-08).
/// A packet that waits until it is full holds the first words of a sentence back until the rest of
/// it is recorded, so with shorter packets they reach the service, and the preview, sooner (豆包语音
/// showed its first word about 0.55 s after speech began with 200 ms packets, 0.48 s with 100 ms).
/// Recorded takes and the probes keep the packets their services suggest.
pub const LIVE_PACKET_BYTES: usize = 100 * BYTES_PER_MS;

/// A realtime connection.
pub type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Open a WebSocket for `request` (its URL, the vendor's headers) through `network`'s proxy plan,
/// within [`CONNECT_TIMEOUT`]. A refused handshake is sorted like an HTTP answer: 401/403 the
/// credential, 429 a rate limit, the rest by what the body says.
pub async fn connect(request: Request, network: &voltip_net::NetworkRef) -> Result<(Socket, Response), AsrError> {
    tokio::time::timeout(CONNECT_TIMEOUT, open(request, network)).await.map_err(|_| AsrError::Timeout)?
}

async fn open(request: Request, network: &voltip_net::NetworkRef) -> Result<(Socket, Response), AsrError> {
    let url = url::Url::parse(&request.uri().to_string()).map_err(|e| AsrError::InvalidConfig(format!("realtime endpoint: {e}")))?;
    let stream = network.connect(&url, CONNECT_TIMEOUT).await.map_err(|e| AsrError::Network(format!("realtime connection: {e}")))?;
    tokio_tungstenite::client_async_tls_with_config(request, stream, None, None).await.map_err(map_ws)
}

/// The handshake request of `url` with the user agent every client sends.
pub fn request(url: &url::Url) -> Result<Request, AsrError> {
    let mut request = url.as_str().into_client_request().map_err(|e| AsrError::InvalidConfig(format!("realtime endpoint: {e}")))?;
    request.headers_mut().insert("user-agent", tungstenite::http::HeaderValue::from_static(concat!("voltip-asr/", env!("CARGO_PKG_VERSION"))));
    Ok(request)
}

/// A WebSocket failure, without the host: the handshake's HTTP refusal is sorted as the HTTP
/// clients sort an answer.
pub fn map_ws(error: tungstenite::Error) -> AsrError {
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

/// The service closed the connection before the session ended.
pub fn closed_early() -> AsrError {
    AsrError::Network("the service closed the connection before the session finished".into())
}

/// How fast a recorded take goes to a realtime session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pace {
    /// As fast as the connection takes it (a service that allows it, Baidu).
    Burst,
    /// `speedup` times the speed the audio plays at (1 for iFlytek, 2 for 豆包语音's 流式输入).
    Realtime {
        /// How many times faster than it plays.
        speedup: u32,
    },
}

impl Pace {
    /// How long sending `played` of audio takes at this pace.
    pub fn sending(self, played: Duration) -> Duration {
        match self {
            Self::Burst => Duration::ZERO,
            Self::Realtime { speedup } => played / speedup.max(1),
        }
    }
}

/// `pcm` (16-bit) in the fewest packets of at most [`LIVE_PACKET_BYTES`], as equal as whole
/// samples allow: for the services that take the audio as it comes (阿里云百炼, 阶跃星辰). None waits
/// for more audio, and none is a sliver: the core feeds 100 ms or more at a time, so a longer feed
/// goes as parts of at least 50 ms; a take's last bit may be shorter, as it was.
pub fn live_packets(pcm: &[u8]) -> impl Iterator<Item = &[u8]> {
    // Samples, a trailing odd byte counting as one, so every edge falls between whole samples.
    let samples = pcm.len().div_ceil(2);
    let count = samples.div_ceil(LIVE_PACKET_BYTES / 2);
    let edge = move |i: usize| if i == count { pcm.len() } else { 2 * (i * samples / count) };
    (0..count).map(move |i| &pcm[edge(i)..edge(i + 1)])
}

/// How long `pcm` plays (16 kHz, 16-bit mono).
pub fn played(pcm: &[u8]) -> Duration {
    Duration::from_millis(u64::try_from(pcm.len() / BYTES_PER_MS).unwrap_or(u64::MAX))
}

/// The deadline of a whole take of `played` audio sent at `pace`: the client's `timeout` (the time
/// the service may take to answer) plus the time the sending itself takes.
pub fn whole_take_deadline(timeout: Duration, played: Duration, pace: Pace) -> Duration {
    timeout.saturating_add(pace.sending(played))
}

/// Feed `pcm` to a session in frames of `frame_bytes` at `pace`, then [`RealtimeInput::Finish`].
/// Frames go out on a fixed schedule from the first one, so a slow send does not add up. Stops
/// early when the session ended (its input closed).
pub async fn feed(input: &mpsc::UnboundedSender<RealtimeInput>, pcm: &[u8], frame_bytes: usize, pace: Pace) {
    let frame_bytes = frame_bytes.max(2);
    let frame = played(&pcm[..frame_bytes.min(pcm.len())]);
    let start = tokio::time::Instant::now();
    for (n, chunk) in pcm.chunks(frame_bytes).enumerate() {
        if let Pace::Realtime { speedup } = pace
            && n > 0
        {
            let due = frame * u32::try_from(n).unwrap_or(u32::MAX) / speedup.max(1);
            tokio::time::sleep_until(start + due).await;
        }
        if input.send(RealtimeInput::Audio(chunk.to_vec())).is_err() {
            return;
        }
    }
    let _ = input.send(RealtimeInput::Finish);
}

/// Recognise a recorded take on `client`'s realtime sessions: its PCM fed in frames of
/// `frame_bytes` at `pace`, the finished sentences joined as they read (nothing between Chinese
/// text, a space between others). Bounded by [`whole_take_deadline`] of `timeout`.
pub async fn transcribe_whole(
    client: &dyn VendorAsr,
    pcm: &[u8],
    language: Option<&str>,
    glossary: &[String],
    frame_bytes: usize,
    pace: Pace,
    timeout: Duration,
) -> Result<Transcript, AsrError> {
    whole_take(client.model(), |input, events| client.realtime(language, glossary, input, events), pcm, frame_bytes, pace, timeout).await
}

/// [`transcribe_whole`] on sessions `session` opens, for `model`: a client whose recorded takes
/// go in other packets than its live sessions' ([`LIVE_PACKET_BYTES`]) passes a session of its
/// own with their size.
pub async fn whole_take<S, F>(model: &str, session: S, pcm: &[u8], frame_bytes: usize, pace: Pace, timeout: Duration) -> Result<Transcript, AsrError>
where
    S: FnOnce(mpsc::UnboundedReceiver<RealtimeInput>, RealtimeEvents) -> F,
    F: Future<Output = Result<(), AsrError>>,
{
    let started = Instant::now();
    let deadline = whole_take_deadline(timeout, played(pcm), pace);
    let (input_tx, input) = mpsc::unbounded_channel();
    let (events_tx, mut events) = mpsc::unbounded_channel();
    let run = async {
        let session = session(input, events_tx);
        let sending = feed(&input_tx, pcm, frame_bytes, pace);
        let collect = async {
            let mut sentences = Vec::new();
            while let Some(event) = events.recv().await {
                match event {
                    RealtimeEvent::Sentence { text, .. } => sentences.push(text),
                    RealtimeEvent::Partial { .. } | RealtimeEvent::Note { .. } => {}
                    RealtimeEvent::Finished => break,
                }
            }
            sentences
        };
        let (result, (), sentences) = tokio::join!(session, sending, collect);
        result.map(|()| sentences)
    };
    let sentences = tokio::time::timeout(deadline, run).await.map_err(|_| AsrError::Timeout)??;
    let text = join_sentences(&sentences);
    let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(Transcript { text, latency_ms, model: model.to_owned() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_deadline_covers_the_sending() {
        let minute = Duration::from_secs(60);
        let timeout = Duration::from_secs(90);
        assert_eq!(whole_take_deadline(timeout, minute, Pace::Burst), timeout);
        assert_eq!(whole_take_deadline(timeout, minute, Pace::Realtime { speedup: 1 }), Duration::from_secs(150));
        assert_eq!(whole_take_deadline(timeout, minute, Pace::Realtime { speedup: 2 }), Duration::from_secs(120));
        assert_eq!(whole_take_deadline(timeout, minute, Pace::Realtime { speedup: 0 }), Duration::from_secs(150), "0 reads as 1");
        assert_eq!(played(&[0u8; 32_000]), Duration::from_secs(1));
    }

    /// Frames go out on schedule: at 1:1 a second of 40 ms frames takes a second, at 2:1 half.
    #[tokio::test(start_paused = true)]
    async fn feeding_keeps_the_pace() {
        for (pace, want) in [(Pace::Realtime { speedup: 1 }, 960), (Pace::Realtime { speedup: 2 }, 480), (Pace::Burst, 0)] {
            let (tx, mut rx) = mpsc::unbounded_channel();
            let start = tokio::time::Instant::now();
            feed(&tx, &[0u8; 32_000], 1280, pace).await;
            let took = start.elapsed().as_millis();
            assert_eq!(took, want, "{pace:?}");
            let mut frames = 0;
            while let Ok(input) = rx.try_recv() {
                match input {
                    RealtimeInput::Audio(bytes) => {
                        assert_eq!(bytes.len(), 1280);
                        frames += 1;
                    }
                    RealtimeInput::Finish => break,
                }
            }
            assert_eq!(frames, 25, "{pace:?}");
        }
    }

    /// docs/dictation.md §3.15 (2026-11-08): a live feed in the fewest packets of at most 100 ms,
    /// as equal as whole samples allow, nothing dropped or reordered: 100 ms goes as it is, the
    /// core's usual 106 ms as two of 53 ms, its longest (just under 200 ms) as two, a second as ten
    /// of 100 ms; nothing makes no packet, and an odd byte stays with the last.
    #[test]
    fn live_packets_hold_at_most_100_ms_each() {
        let sizes = |bytes: usize| -> Vec<usize> {
            let pcm: Vec<u8> = (0..bytes).map(|i| (i % 251) as u8).collect();
            let packets: Vec<&[u8]> = live_packets(&pcm).collect();
            assert_eq!(packets.concat(), pcm, "{bytes}: everything, in order");
            assert!(packets.iter().all(|p| p.len() <= LIVE_PACKET_BYTES), "{bytes}: {packets:?}");
            packets.iter().map(|p| p.len()).collect()
        };
        assert_eq!(LIVE_PACKET_BYTES, 3200);
        assert_eq!(sizes(0), Vec::<usize>::new());
        assert_eq!(sizes(640), [640]);
        assert_eq!(sizes(3200), [3200]);
        assert_eq!(sizes(3202), [1600, 1602]);
        assert_eq!(sizes(3400), [1700, 1700]);
        assert_eq!(sizes(6398), [3198, 3200]);
        assert_eq!(sizes(6400), [3200, 3200]);
        assert_eq!(sizes(6402), [2134, 2134, 2134]);
        assert_eq!(sizes(32_000), [3200; 10]);
        assert_eq!(sizes(3201), [1600, 1601]);
        // Every edge between whole samples.
        for bytes in (0..20_000).step_by(2) {
            let pcm = vec![0u8; bytes];
            assert!(live_packets(&pcm).all(|p| p.len() % 2 == 0), "{bytes}");
        }
    }

    /// A session that ended stops the feeding.
    #[tokio::test(start_paused = true)]
    async fn feeding_stops_when_the_session_ended() {
        let (tx, rx) = mpsc::unbounded_channel();
        drop(rx);
        let start = tokio::time::Instant::now();
        feed(&tx, &[0u8; 320_000], 1280, Pace::Realtime { speedup: 1 }).await;
        assert!(start.elapsed() < Duration::from_millis(100));
    }
}
