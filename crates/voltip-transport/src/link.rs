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
    /// Tests only: the writer takes this shared before it writes a frame, so a test that holds it
    /// exclusively keeps the queued frames waiting (docs/dictation.md §20.8, frames of an ended
    /// session left in the queue).
    pub write_gate: Option<std::sync::Arc<tokio::sync::RwLock<()>>>,
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
            write_gate: None,
        }
    }
}

/// What the owner of a link observes.
#[derive(Debug)]
pub enum LinkEvent {
    /// Lifecycle transition.
    State(StateChange),
    /// A frame from the relay.
    Frame(RelayFrame),
    /// A non-fatal problem worth surfacing (connect failure text, decode failure).
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
            Outcome::Lost(reason) => {
                let _ = evt.try_send(LinkEvent::Warning(reason));
                let (change, attempts) = {
                    let mut m = state.lock();
                    let c = m.reconnecting();
                    (c, m.attempts())
                };
                emit_state(change, &evt);
                match config.reconnect.delay_for(attempts) {
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
    Lost(String),
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
    Failed(String),
}

/// Open a fresh connection to `config.endpoint`, exchange `hello` / `hello_ack` and close it:
/// whether a Voltip relay answers there, and how long that took. Nothing else is sent.
/// `config.connect_timeout` bounds the whole exchange.
pub async fn probe(config: &LinkConfig) -> Result<Duration, ProbeFailure> {
    let started = tokio::time::Instant::now();
    let url = config.endpoint.url().as_str().to_owned();
    let exchange = async {
        let (ws, _) = tokio_tungstenite::connect_async(url).await.map_err(|e| match &e {
            tokio_tungstenite::tungstenite::Error::Io(io) if io.kind() == std::io::ErrorKind::ConnectionRefused => ProbeFailure::Refused,
            _ => ProbeFailure::Failed(e.to_string()),
        })?;
        let (mut sink, mut stream) = ws.split();
        let hello = RelayFrame::Hello { version: ProtocolVersion::CURRENT, client_version: config.client_version.clone() };
        let text = hello.encode().map_err(|e| ProbeFailure::Failed(e.to_string()))?;
        sink.send(Message::Text(text.into())).await.map_err(|e| ProbeFailure::Failed(e.to_string()))?;
        loop {
            match stream.next().await {
                Some(Ok(Message::Text(t))) => match RelayFrame::decode(&t) {
                    Ok(RelayFrame::HelloAck { .. }) => break,
                    Ok(RelayFrame::Error { code, .. }) => return Err(ProbeFailure::Failed(format!("hello refused: {code:?}"))),
                    Ok(_) => return Err(ProbeFailure::Failed("unexpected frame before hello_ack".into())),
                    Err(e) => return Err(ProbeFailure::Failed(e.to_string())),
                },
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                Some(Ok(_)) => return Err(ProbeFailure::Failed("unexpected message before hello_ack".into())),
                Some(Err(e)) => return Err(ProbeFailure::Failed(e.to_string())),
                None => return Err(ProbeFailure::Failed("closed before hello_ack".into())),
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
    let url = config.endpoint.url().as_str().to_owned();
    let connected = tokio::time::timeout(config.connect_timeout, tokio_tungstenite::connect_async(url)).await;
    let (ws, _) = match connected {
        Ok(Ok(pair)) => pair,
        Ok(Err(e)) => return Outcome::Lost(format!("connect: {e}")),
        Err(_) => return Outcome::Lost(format!("connect timed out after {:?}", config.connect_timeout)),
    };
    let (mut sink, mut stream) = ws.split();
    let change = state.lock().authenticating();
    if let Some(c) = change {
        let _ = evt.try_send(LinkEvent::State(c));
    }
    let hello = RelayFrame::Hello { version: ProtocolVersion::CURRENT, client_version: config.client_version.clone() };
    let Ok(text) = hello.encode() else { return Outcome::Lost("encode hello".into()) };
    if let Err(e) = sink.send(Message::Text(text.into())).await {
        return Outcome::Lost(format!("send hello: {e}"));
    }
    let ack = tokio::time::timeout(config.hello_timeout, async {
        loop {
            match stream.next().await {
                Some(Ok(Message::Text(t))) => return RelayFrame::decode(&t).map_err(|e| e.to_string()),
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                Some(Ok(other)) => return Err(format!("unexpected frame before hello_ack: {other:?}")),
                Some(Err(e)) => return Err(e.to_string()),
                None => return Err("closed before hello_ack".into()),
            }
        }
    })
    .await;
    match ack {
        Ok(Ok(RelayFrame::HelloAck { .. })) => {}
        Ok(Ok(RelayFrame::Error { code, .. })) => return Outcome::Lost(format!("relay refused hello: {code:?}")),
        Ok(Ok(other)) => return Outcome::Lost(format!("expected hello_ack, got {}", other.encode().unwrap_or_default())),
        Ok(Err(e)) => return Outcome::Lost(e),
        Err(_) => return Outcome::Lost("hello_ack timed out".into()),
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
                        return Outcome::Lost(format!("send: {e}"));
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
                        return Outcome::Lost(format!("ping: {e}"));
                    }
                }
            },
            msg = stream.next() => match msg {
                Some(Ok(Message::Text(t))) => match RelayFrame::decode(&t) {
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
                Some(Ok(Message::Close(_))) | None => return Outcome::Lost("socket closed by peer".into()),
                Some(Ok(_)) => {}
                Some(Err(e)) => return Outcome::Lost(format!("read: {e}")),
            },
            _ = ping.tick() => {
                if pong_deadline.is_none() {
                    pings += 1;
                    pong_deadline = Some(tokio::time::Instant::now() + config.pong_timeout);
                    if let Err(e) = sink.send(Message::Ping(pings.to_be_bytes().to_vec().into())).await {
                        return Outcome::Lost(format!("ping: {e}"));
                    }
                }
            },
            _ = async { match pong_deadline { Some(d) => tokio::time::sleep_until(d).await, None => std::future::pending().await } } => {
                return Outcome::Lost("pong timeout".into());
            }
        }
    }
}
