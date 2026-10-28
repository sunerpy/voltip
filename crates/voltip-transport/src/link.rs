//! A WebSocket link to the relay: a heartbeat that notices a dead socket, automatic reconnect,
//! and [`RelayLink::reconnect_now`] for the moments a dead socket is likely (docs/pairing.md
//! 「重连」).

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt as _, StreamExt as _};
use parking_lot::Mutex;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use voltip_protocol::ProtocolVersion;
use voltip_protocol::relay::RelayFrame;

use crate::{ConnectionMachine, ConnectionState, ReconnectPolicy, RelayEndpoint, StateChange, TransportError};

/// Link tunables.
#[derive(Clone, Debug)]
pub struct LinkConfig {
    /// Where to connect.
    pub endpoint: RelayEndpoint,
    /// Sent in `hello`.
    pub client_version: String,
    /// TCP + WebSocket handshake deadline.
    pub connect_timeout: Duration,
    /// `hello` → `hello_ack` deadline.
    pub hello_timeout: Duration,
    /// Backoff between attempts.
    pub reconnect: ReconnectPolicy,
    /// WebSocket ping cadence (liveness).
    pub ping_interval: Duration,
    /// Time allowed for a pong before the socket is declared dead.
    pub pong_timeout: Duration,
    /// After [`RelayLink::reconnect_now`]: time allowed for the pong of the ping it sends at once.
    pub probe_timeout: Duration,
    /// The wait before trying again once the relay said this version is below its minimum
    /// ([`FailureKind::TooOld`], docs/pairing.md 「最低版本」): only an update helps, so the link does
    /// not knock every few seconds. [`RelayLink::reconnect_now`] still tries at once.
    pub too_old_retry: Duration,
    /// Tests only: the writer takes this shared before it writes a frame, so a test that holds it
    /// exclusively keeps the queued frames waiting (docs/dictation.md §20.8, frames of an ended
    /// session left in the queue).
    pub write_gate: Option<std::sync::Arc<tokio::sync::RwLock<()>>>,
    /// The proxy plan the connection follows (docs/dictation.md §24): each attempt asks it again.
    pub network: voltip_net::NetworkRef,
}

impl LinkConfig {
    /// Sensible defaults for `endpoint`.
    pub fn new(endpoint: RelayEndpoint) -> Self {
        Self {
            endpoint,
            client_version: format!("voltip/{}", env!("CARGO_PKG_VERSION")),
            connect_timeout: Duration::from_secs(8),
            hello_timeout: Duration::from_secs(5),
            reconnect: ReconnectPolicy::default(),
            // A dead socket is noticed within 25 s at most; `reconnect_now` checks at once.
            ping_interval: Duration::from_secs(15),
            pong_timeout: Duration::from_secs(10),
            probe_timeout: Duration::from_secs(5),
            too_old_retry: Duration::from_secs(30 * 60),
            write_gate: None,
            network: voltip_net::NetworkRef::default(),
        }
    }
}

/// The WebSocket to `config.endpoint`: a TCP stream through the proxy the plan picks (direct when
/// it picks none), then TLS and the handshake on it, as `connect_async` would run them.
async fn open_socket(config: &LinkConfig) -> Result<Socket, OpenError> {
    let url = config.endpoint.url();
    let stream = config.network.connect(url, config.connect_timeout).await.map_err(OpenError::Connect)?;
    let (ws, _) = tokio_tungstenite::client_async_tls_with_config(url.as_str(), stream, None, None).await.map_err(OpenError::Handshake)?;
    Ok(ws)
}

type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Why [`open_socket`] failed.
#[derive(Debug, thiserror::Error)]
enum OpenError {
    #[error("{0}")]
    Connect(voltip_net::ConnectError),
    #[error("{0}")]
    Handshake(tokio_tungstenite::tungstenite::Error),
}

/// What kind of failure a connection to the relay met: what the apps tell the user while it is not
/// connected (docs/pairing.md 「重连」; issue #193: the phone said only 「尚未连上中继」).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// Nothing answered in time: connecting, or the relay's `hello_ack`.
    Timeout,
    /// The relay's name did not resolve (often: no network at all).
    Dns,
    /// The address refused the connection.
    Refused,
    /// This device has no route to the relay.
    Unreachable,
    /// The TLS handshake failed or was cut short.
    Tls,
    /// The address answered, but not as a Voltip relay (a refused WebSocket upgrade).
    NotRelay,
    /// The proxy could not reach the relay.
    Proxy,
    /// The relay does not speak this app's protocol version.
    Version,
    /// This app's version is below the relay's minimum (`LinkFailure::min_version` names it): it
    /// may not pair or reconnect until it is updated (docs/pairing.md 「最低版本」).
    TooOld,
    /// The connection was up and went down.
    Lost,
    /// Anything else.
    #[default]
    Other,
}

/// A failed attempt or a lost connection: its kind, and the transport's own words. The words can
/// quote the endpoint, so whoever shows them decides whether they may (the build's own relay is a
/// build secret).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkFailure {
    /// What kind of failure.
    pub kind: FailureKind,
    /// The error as the transport has it.
    pub detail: String,
    /// The lowest version the relay serves, with [`FailureKind::TooOld`].
    pub min_version: Option<String>,
}

impl LinkFailure {
    fn new(kind: FailureKind, detail: impl Into<String>) -> Self {
        Self { kind, detail: detail.into(), min_version: None }
    }

    /// The relay's refusal `code` (`min_version` with `client_too_old`). The version reaches the UI,
    /// so only a plain `x.y.z` is kept: a relay the user typed in could send anything.
    fn refused(code: voltip_protocol::relay::RelayErrorCode, min_version: Option<String>, detail: String) -> Self {
        let kind = refusal_kind(code);
        let min_version = min_version.filter(|v| kind == FailureKind::TooOld && is_plain_version(v));
        Self { kind, detail, min_version }
    }

    /// Opening the socket failed (`connect: …`).
    fn opening(error: &OpenError) -> Self {
        Self::new(error.kind(), format!("connect: {error}"))
    }
}

impl std::fmt::Display for LinkFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.detail)
    }
}

impl OpenError {
    /// The kind of failure, from the error's own type: the proxy's, the connection's, then the
    /// TLS and WebSocket handshakes'. tokio-tungstenite hands rustls' errors over as I/O errors.
    fn kind(&self) -> FailureKind {
        use tokio_tungstenite::tungstenite::Error as Ws;
        use voltip_net::ConnectError;
        match self {
            Self::Connect(error) => match error {
                ConnectError::Timeout(_) => FailureKind::Timeout,
                ConnectError::Resolve(_) => FailureKind::Dns,
                ConnectError::Direct(io) => io_kind(io),
                ConnectError::Proxy(_) | ConnectError::Unsupported(_) => FailureKind::Proxy,
                ConnectError::NoHost => FailureKind::Other,
            },
            Self::Handshake(Ws::Io(io)) if io.kind() == std::io::ErrorKind::TimedOut => FailureKind::Timeout,
            Self::Handshake(Ws::Io(_) | Ws::Tls(_)) => FailureKind::Tls,
            Self::Handshake(Ws::Http(_) | Ws::HttpFormat(_) | Ws::Protocol(_)) => FailureKind::NotRelay,
            Self::Handshake(_) => FailureKind::Other,
        }
    }
}

/// The kind of a failed TCP connection.
fn io_kind(io: &std::io::Error) -> FailureKind {
    use std::io::ErrorKind;
    match io.kind() {
        ErrorKind::ConnectionRefused => FailureKind::Refused,
        ErrorKind::NetworkUnreachable | ErrorKind::HostUnreachable | ErrorKind::NetworkDown | ErrorKind::AddrNotAvailable => FailureKind::Unreachable,
        ErrorKind::TimedOut => FailureKind::Timeout,
        _ => FailureKind::Other,
    }
}

/// `x.y.z`, each part 1 to 9 digits.
fn is_plain_version(text: &str) -> bool {
    let parts: Vec<&str> = text.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| (1..=9).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit()))
}

/// The kind of the relay refusing `hello`.
fn refusal_kind(code: voltip_protocol::relay::RelayErrorCode) -> FailureKind {
    match code {
        voltip_protocol::relay::RelayErrorCode::UnsupportedVersion => FailureKind::Version,
        voltip_protocol::relay::RelayErrorCode::ClientTooOld => FailureKind::TooOld,
        _ => FailureKind::Other,
    }
}

/// What the owner of a link observes.
#[derive(Debug)]
pub enum LinkEvent {
    /// Lifecycle transition.
    State(StateChange),
    /// A frame from the relay.
    Frame(RelayFrame),
    /// An attempt failed or the connection went down; the link tries again (a `Reconnecting`
    /// transition follows).
    Failed(LinkFailure),
    /// A non-fatal problem worth surfacing (a frame that does not decode).
    Warning(String),
}

enum Cmd {
    Send(RelayFrame),
    Close,
    /// [`RelayLink::reconnect_now`].
    Now,
}

/// Handle to a running link task.
pub struct RelayLink {
    cmd: mpsc::Sender<Cmd>,
    state: Arc<Mutex<ConnectionMachine>>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl std::fmt::Debug for RelayLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayLink").field("state", &self.state()).finish()
    }
}

impl RelayLink {
    /// Start connecting in the background. Events arrive on the returned receiver.
    pub fn spawn(config: LinkConfig) -> (Self, mpsc::Receiver<LinkEvent>) {
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let (evt_tx, evt_rx) = mpsc::channel(256);
        let state = Arc::new(Mutex::new(ConnectionMachine::new()));
        let task = tokio::spawn(run(config, cmd_rx, evt_tx, state.clone()));
        (Self { cmd: cmd_tx, state, task: Mutex::new(Some(task)) }, evt_rx)
    }

    /// Current state.
    pub fn state(&self) -> ConnectionState {
        self.state.lock().state()
    }

    /// Queue a frame. Fails when the link is not connected (callers should wait for
    /// `LinkEvent::State { to: Connected }` and re-send higher-level state themselves).
    pub async fn send(&self, frame: RelayFrame) -> Result<(), TransportError> {
        let st = self.state();
        if !st.is_connected() {
            return Err(TransportError::NotConnected(st));
        }
        self.cmd.send(Cmd::Send(frame)).await.map_err(|_| TransportError::Closed)
    }

    /// The network changed, the device woke up or the app came back to the front (docs/pairing.md
    /// 「重连」): a socket from before is likely dead, and waiting for the heartbeat or the backoff
    /// would keep the devices apart for many seconds. A link waiting out its backoff tries again at
    /// once; a connected one pings at once and reconnects when no pong comes within
    /// `probe_timeout`. Never waits; a link that is closed ignores it.
    pub fn reconnect_now(&self) {
        let _ = self.cmd.try_send(Cmd::Now);
    }

    /// Free places in the queue of frames waiting to be written (docs/dictation.md §20.8: the
    /// parts of a large body go out only while room is left for other messages).
    pub fn free_slots(&self) -> usize {
        self.cmd.capacity()
    }

    /// Close for good (sends `bye` when connected). Idempotent.
    pub async fn close(&self) {
        let _ = self.cmd.send(Cmd::Close).await;
        let task = self.task.lock().take();
        if let Some(t) = task {
            let _ = tokio::time::timeout(Duration::from_secs(3), t).await;
        }
    }
}

async fn run(config: LinkConfig, mut cmd_rx: mpsc::Receiver<Cmd>, evt: mpsc::Sender<LinkEvent>, state: Arc<Mutex<ConnectionMachine>>) {
    let emit_state = |change: Option<StateChange>, evt: &mpsc::Sender<LinkEvent>| {
        if let Some(c) = change {
            tracing::debug!(from = ?c.from, to = ?c.to, "link state");
            let _ = evt.try_send(LinkEvent::State(c));
        }
    };
    loop {
        let change = state.lock().connecting();
        emit_state(change, &evt);
        if state.lock().state().is_closed() {
            return;
        }
        match connect_and_run(&config, &mut cmd_rx, &evt, &state).await {
            Outcome::Closed => {
                let change = state.lock().closed();
                emit_state(change, &evt);
                return;
            }
            Outcome::Lost(failure) => {
                let too_old = failure.kind == FailureKind::TooOld;
                let _ = evt.try_send(LinkEvent::Failed(failure));
                let (change, attempts) = {
                    let mut m = state.lock();
                    let c = m.reconnecting();
                    (c, m.attempts())
                };
                emit_state(change, &evt);
                // Below the relay's minimum only an update helps (docs/pairing.md 「最低版本」).
                let delay = match config.reconnect.delay_for(attempts) {
                    Some(_) if too_old => Some(config.too_old_retry),
                    delay => delay,
                };
                match delay {
                    Some(delay) => {
                        tracing::info!(attempt = attempts, ?delay, "reconnecting");
                        let until = tokio::time::Instant::now() + delay;
                        loop {
                            tokio::select! {
                                () = tokio::time::sleep_until(until) => break,
                                cmd = cmd_rx.recv() => match cmd {
                                    Some(Cmd::Close) | None => {
                                        let change = state.lock().closed();
                                        emit_state(change, &evt);
                                        return;
                                    }
                                    Some(Cmd::Now) => {
                                        tracing::info!(attempt = attempts, "reconnecting at once");
                                        break;
                                    }
                                    // Queued just before the link went down: the owner sends its
                                    // state again on `Connected`.
                                    Some(Cmd::Send(_)) => {}
                                },
                            }
                        }
                    }
                    None => {
                        let change = state.lock().closed();
                        emit_state(change, &evt);
                        return;
                    }
                }
            }
        }
    }
}

enum Outcome {
    Closed,
    Lost(LinkFailure),
}

/// Why a [`probe`] did not reach a Voltip relay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProbeFailure {
    /// Nothing answered within the deadline (a firewall that drops, a broken path, a host that is
    /// gone).
    Timeout,
    /// The address answered but refused the connection (nothing listens on that port, or a
    /// firewall that rejects).
    Refused,
    /// Anything else: DNS, TLS, an HTTP answer that is not a WebSocket, a refused `hello`.
    Failed(LinkFailure),
}

/// Open a fresh connection to `config.endpoint`, exchange `hello` / `hello_ack` and close it:
/// whether a Voltip relay answers there, and how long that took. Nothing else is sent.
/// `config.connect_timeout` bounds the whole exchange.
pub async fn probe(config: &LinkConfig) -> Result<Duration, ProbeFailure> {
    let started = tokio::time::Instant::now();
    let exchange = async {
        let ws = open_socket(config).await.map_err(|e| match e.kind() {
            FailureKind::Refused => ProbeFailure::Refused,
            FailureKind::Timeout => ProbeFailure::Timeout,
            _ => ProbeFailure::Failed(LinkFailure::opening(&e)),
        })?;
        let failed = |kind: FailureKind, detail: String| ProbeFailure::Failed(LinkFailure::new(kind, detail));
        let (mut sink, mut stream) = ws.split();
        let hello = RelayFrame::hello(config.client_version.clone());
        let text = hello.encode().map_err(|e| failed(FailureKind::Other, e.to_string()))?;
        sink.send(Message::Text(text.into())).await.map_err(|e| failed(FailureKind::Lost, e.to_string()))?;
        loop {
            match stream.next().await {
                Some(Ok(Message::Text(t))) => match RelayFrame::decode(&t) {
                    Ok(RelayFrame::HelloAck { .. }) => break,
                    Ok(RelayFrame::Error { code, min_version, .. }) => {
                        return Err(ProbeFailure::Failed(LinkFailure::refused(code, min_version, format!("hello refused: {code:?}"))));
                    }
                    Ok(_) => return Err(failed(FailureKind::NotRelay, "unexpected frame before hello_ack".into())),
                    Err(e) => return Err(failed(FailureKind::NotRelay, e.to_string())),
                },
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                Some(Ok(_)) => return Err(failed(FailureKind::NotRelay, "unexpected message before hello_ack".into())),
                Some(Err(e)) => return Err(failed(FailureKind::Lost, e.to_string())),
                None => return Err(failed(FailureKind::Lost, "closed before hello_ack".into())),
            }
        }
        let elapsed = started.elapsed();
        let _ = sink.send(Message::Close(None)).await;
        Ok(elapsed)
    };
    tokio::time::timeout(config.connect_timeout, exchange).await.unwrap_or(Err(ProbeFailure::Timeout))
}

async fn connect_and_run(
    config: &LinkConfig,
    cmd_rx: &mut mpsc::Receiver<Cmd>,
    evt: &mpsc::Sender<LinkEvent>,
    state: &Arc<Mutex<ConnectionMachine>>,
) -> Outcome {
    let connected = tokio::time::timeout(config.connect_timeout, open_socket(config)).await;
    let ws = match connected {
        Ok(Ok(ws)) => ws,
        Ok(Err(e)) => return Outcome::Lost(LinkFailure::opening(&e)),
        Err(_) => return Outcome::Lost(LinkFailure::new(FailureKind::Timeout, format!("connect timed out after {:?}", config.connect_timeout))),
    };
    let (mut sink, mut stream) = ws.split();
    let change = state.lock().authenticating();
    if let Some(c) = change {
        let _ = evt.try_send(LinkEvent::State(c));
    }
    let hello = RelayFrame::hello(config.client_version.clone());
    let Ok(text) = hello.encode() else { return Outcome::Lost(LinkFailure::new(FailureKind::Other, "encode hello")) };
    if let Err(e) = sink.send(Message::Text(text.into())).await {
        return Outcome::Lost(LinkFailure::new(FailureKind::Lost, format!("send hello: {e}")));
    }
    let ack = tokio::time::timeout(config.hello_timeout, async {
        loop {
            match stream.next().await {
                Some(Ok(Message::Text(t))) => return RelayFrame::decode(&t).map_err(|e| LinkFailure::new(FailureKind::NotRelay, e.to_string())),
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                Some(Ok(other)) => return Err(LinkFailure::new(FailureKind::NotRelay, format!("unexpected frame before hello_ack: {other:?}"))),
                Some(Err(e)) => return Err(LinkFailure::new(FailureKind::Lost, e.to_string())),
                None => return Err(LinkFailure::new(FailureKind::Lost, "closed before hello_ack")),
            }
        }
    })
    .await;
    match ack {
        Ok(Ok(RelayFrame::HelloAck { .. })) => {}
        Ok(Ok(RelayFrame::Error { code, min_version, .. })) => {
            return Outcome::Lost(LinkFailure::refused(code, min_version, format!("relay refused hello: {code:?}")));
        }
        Ok(Ok(other)) => {
            return Outcome::Lost(LinkFailure::new(FailureKind::NotRelay, format!("expected hello_ack, got {}", other.encode().unwrap_or_default())));
        }
        Ok(Err(e)) => return Outcome::Lost(e),
        Err(_) => return Outcome::Lost(LinkFailure::new(FailureKind::Timeout, "hello_ack timed out")),
    }
    let change = state.lock().connected();
    if let Some(c) = change {
        let _ = evt.try_send(LinkEvent::State(c));
    }

    let mut ping = tokio::time::interval(config.ping_interval);
    ping.tick().await; // first tick is immediate; skip it
    // The ping in flight (its payload) and when its pong is due. A pong that answers an older ping
    // does not count: after `reconnect_now` only the newest one says the socket works now.
    let mut pings: u64 = 0;
    let mut pong_deadline: Option<tokio::time::Instant> = None;
    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => match cmd {
                Some(Cmd::Send(frame)) => {
                    let Ok(text) = frame.encode() else { continue };
                    let _gate = match &config.write_gate {
                        Some(gate) => Some(gate.read().await),
                        None => None,
                    };
                    if let Err(e) = sink.send(Message::Text(text.into())).await {
                        return Outcome::Lost(LinkFailure::new(FailureKind::Lost, format!("send: {e}")));
                    }
                }
                Some(Cmd::Close) | None => {
                    if let Ok(text) = (RelayFrame::Bye { version: ProtocolVersion::CURRENT }).encode() {
                        let _ = sink.send(Message::Text(text.into())).await;
                    }
                    let _ = sink.send(Message::Close(None)).await;
                    return Outcome::Closed;
                }
                Some(Cmd::Now) => {
                    pings += 1;
                    let due = tokio::time::Instant::now() + config.probe_timeout;
                    pong_deadline = Some(pong_deadline.map_or(due, |d| d.min(due)));
                    tracing::debug!(timeout = ?config.probe_timeout, "checking the socket");
                    if let Err(e) = sink.send(Message::Ping(pings.to_be_bytes().to_vec().into())).await {
                        return Outcome::Lost(LinkFailure::new(FailureKind::Lost, format!("ping: {e}")));
                    }
                }
            },
            msg = stream.next() => match msg {
                Some(Ok(Message::Text(t))) => match RelayFrame::decode(&t) {
                    // The relay raised its minimum while this link was up: it closes the socket
                    // next, and the link waits as it does after a refused `hello`.
                    Ok(RelayFrame::Error { code: code @ voltip_protocol::relay::RelayErrorCode::ClientTooOld, min_version, .. }) => {
                        return Outcome::Lost(LinkFailure::refused(code, min_version, format!("relay refused the connection: {code:?}")));
                    }
                    Ok(frame) => {
                        if evt.send(LinkEvent::Frame(frame)).await.is_err() {
                            return Outcome::Closed;
                        }
                    }
                    Err(e) => { let _ = evt.try_send(LinkEvent::Warning(format!("bad frame: {e}"))); }
                },
                Some(Ok(Message::Pong(p))) => {
                    if p.as_ref() == pings.to_be_bytes() {
                        pong_deadline = None;
                    }
                }
                Some(Ok(Message::Ping(p))) => { let _ = sink.send(Message::Pong(p)).await; }
                Some(Ok(Message::Close(_))) | None => return Outcome::Lost(LinkFailure::new(FailureKind::Lost, "socket closed by peer")),
                Some(Ok(_)) => {}
                Some(Err(e)) => return Outcome::Lost(LinkFailure::new(FailureKind::Lost, format!("read: {e}"))),
            },
            _ = ping.tick() => {
                if pong_deadline.is_none() {
                    pings += 1;
                    pong_deadline = Some(tokio::time::Instant::now() + config.pong_timeout);
                    if let Err(e) = sink.send(Message::Ping(pings.to_be_bytes().to_vec().into())).await {
                        return Outcome::Lost(LinkFailure::new(FailureKind::Lost, format!("ping: {e}")));
                    }
                }
            },
            _ = async { match pong_deadline { Some(d) => tokio::time::sleep_until(d).await, None => std::future::pending().await } } => {
                return Outcome::Lost(LinkFailure::new(FailureKind::Lost, "pong timeout"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Error, ErrorKind};

    use tokio_tungstenite::tungstenite::Error as Ws;
    use voltip_net::ConnectError;
    use voltip_protocol::relay::RelayErrorCode;

    use super::*;

    /// Each way of failing to open the socket has its kind, from the error's type, never from
    /// its words.
    #[test]
    fn each_way_of_failing_to_open_has_its_kind() {
        let cases = [
            (OpenError::Connect(ConnectError::Timeout(Duration::from_secs(8))), FailureKind::Timeout),
            (OpenError::Connect(ConnectError::Resolve(Error::other("no such name"))), FailureKind::Dns),
            (OpenError::Connect(ConnectError::Direct(Error::from(ErrorKind::ConnectionRefused))), FailureKind::Refused),
            (OpenError::Connect(ConnectError::Direct(Error::from(ErrorKind::NetworkUnreachable))), FailureKind::Unreachable),
            (OpenError::Connect(ConnectError::Direct(Error::from(ErrorKind::HostUnreachable))), FailureKind::Unreachable),
            (OpenError::Connect(ConnectError::Direct(Error::from(ErrorKind::TimedOut))), FailureKind::Timeout),
            (OpenError::Connect(ConnectError::Direct(Error::from(ErrorKind::PermissionDenied))), FailureKind::Other),
            (OpenError::Connect(ConnectError::Proxy("tunnel refused".into())), FailureKind::Proxy),
            (OpenError::Connect(ConnectError::Unsupported("https".into())), FailureKind::Proxy),
            (OpenError::Connect(ConnectError::NoHost), FailureKind::Other),
            (OpenError::Handshake(Ws::Io(Error::new(ErrorKind::UnexpectedEof, "tls handshake eof"))), FailureKind::Tls),
            (OpenError::Handshake(Ws::Io(Error::new(ErrorKind::InvalidData, "invalid peer certificate"))), FailureKind::Tls),
            (OpenError::Handshake(Ws::Io(Error::from(ErrorKind::TimedOut))), FailureKind::Timeout),
            (
                OpenError::Handshake(Ws::Http(Box::new(tokio_tungstenite::tungstenite::http::Response::builder().status(404).body(None).unwrap()))),
                FailureKind::NotRelay,
            ),
            (OpenError::Handshake(Ws::ConnectionClosed), FailureKind::Other),
        ];
        for (error, kind) in cases {
            assert_eq!(error.kind(), kind, "{error}");
            let failure = LinkFailure::opening(&error);
            assert_eq!((failure.kind, failure.detail), (kind, format!("connect: {error}")));
        }
        assert_eq!(refusal_kind(RelayErrorCode::UnsupportedVersion), FailureKind::Version);
        assert_eq!(refusal_kind(RelayErrorCode::RateLimited), FailureKind::Other);
    }

    /// The kinds go on the wire under their own names (the apps word them).
    #[test]
    fn the_kinds_serialise_in_snake_case() {
        let names: Vec<String> = [
            FailureKind::Timeout,
            FailureKind::Dns,
            FailureKind::Refused,
            FailureKind::Unreachable,
            FailureKind::Tls,
            FailureKind::NotRelay,
            FailureKind::Proxy,
            FailureKind::Version,
            FailureKind::Lost,
            FailureKind::Other,
        ]
        .iter()
        .map(|kind| serde_json::to_string(kind).unwrap())
        .collect();
        assert_eq!(
            names,
            [
                r#""timeout""#,
                r#""dns""#,
                r#""refused""#,
                r#""unreachable""#,
                r#""tls""#,
                r#""not_relay""#,
                r#""proxy""#,
                r#""version""#,
                r#""lost""#,
                r#""other""#
            ]
        );
        assert_eq!(FailureKind::default(), FailureKind::Other);
    }
}
