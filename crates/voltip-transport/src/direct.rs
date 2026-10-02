//! A LAN WebSocket listener that embeds `RelayCore`.
//!
//! Every device runs one of these for as long as the core runs ([`DirectHost::bind_lan_host`]):
//! the ticket's `direct_hints` point at it during pairing, and afterwards paired devices meet on
//! it through rendezvous channels exactly as they would on a public relay. Both the owner (via
//! loopback) and the peer connect as ordinary [`crate::RelayLink`]s, so the protocol code has a
//! single path for "relay" and "direct".

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use futures_util::{SinkExt as _, StreamExt as _};
use parking_lot::Mutex;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use voltip_protocol::relay::{RelayErrorCode, RelayFrame};
use voltip_relay::{ConnId, Delivery, RelayConfig, RelayCore};

use crate::{RelayEndpoint, TransportError};

/// Running LAN host.
pub struct DirectHost {
    addr: SocketAddr,
    shared: Arc<Shared>,
    accept_task: tokio::task::JoinHandle<()>,
    tick_task: tokio::task::JoinHandle<()>,
}

struct Shared {
    core: Mutex<RelayCore>,
    outbound: Mutex<std::collections::HashMap<ConnId, mpsc::Sender<Outbound>>>,
    next_id: AtomicU64,
    /// Frames dropped on a full outbound queue.
    dropped: AtomicU64,
}

enum Outbound {
    Frame(RelayFrame),
    Close,
}

impl std::fmt::Debug for DirectHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DirectHost").field("addr", &self.addr).finish()
    }
}

impl DirectHost {
    /// Bind a pairing-only host on `addr` (use port 0 for an ephemeral port) and start accepting.
    pub async fn bind(addr: SocketAddr) -> Result<Self, TransportError> {
        Self::bind_with(addr, RelayConfig::single_session()).await
    }

    /// Bind the device's persistent LAN host (pairing session + rendezvous channels). When the
    /// preferred port is taken, fall back to an ephemeral one: peers learn the actual endpoint
    /// from the ticket and from `device_info_update` messages, so a fixed port is only a
    /// convenience that keeps stored hints valid across restarts.
    pub async fn bind_lan_host(preferred: SocketAddr) -> Result<Self, TransportError> {
        match Self::bind_with(preferred, RelayConfig::lan_host()).await {
            Ok(host) => Ok(host),
            Err(TransportError::Io(e)) if e.kind() == std::io::ErrorKind::AddrInUse && preferred.port() != 0 => {
                tracing::warn!(%preferred, "preferred LAN port in use; falling back to an ephemeral port");
                Self::bind_with(SocketAddr::new(preferred.ip(), 0), RelayConfig::lan_host()).await
            }
            Err(e) => Err(e),
        }
    }

    async fn bind_with(addr: SocketAddr, config: RelayConfig) -> Result<Self, TransportError> {
        let listener = TcpListener::bind(addr).await?;
        let addr = listener.local_addr()?;
        let shared = Arc::new(Shared {
            core: Mutex::new(RelayCore::new(config)),
            outbound: Mutex::new(Default::default()),
            next_id: AtomicU64::new(1),
            dropped: AtomicU64::new(0),
        });
        let accept_shared = shared.clone();
        let accept_task = tokio::spawn(async move {
            loop {
                let Ok((stream, peer)) = listener.accept().await else { break };
                let s = accept_shared.clone();
                tokio::spawn(handle(stream, peer, s));
            }
        });
        let tick_shared = shared.clone();
        let tick_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                interval.tick().await;
                let deliveries = tick_shared.core.lock().tick(Instant::now());
                dispatch(&tick_shared, deliveries);
            }
        });
        tracing::info!(%addr, "direct host listening");
        Ok(Self { addr, shared, accept_task, tick_task })
    }

    /// Bound address.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Loopback endpoint the host's own process should connect to.
    pub fn loopback_endpoint(&self) -> Result<RelayEndpoint, TransportError> {
        RelayEndpoint::parse(&format!("ws://127.0.0.1:{}/ws", self.addr.port()))
    }

    /// `ip:port` strings for the ticket: the primary LAN IPv4 when one exists, otherwise the
    /// loopback address (only useful for two processes on one machine, e.g. tests and demos).
    pub fn lan_hints(&self) -> Vec<String> {
        let ip = if self.addr.ip().is_unspecified() { primary_lan_ip().unwrap_or(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)) } else { self.addr.ip() };
        vec![format!("{ip}:{}", self.addr.port())]
    }

    /// Relay-core counters.
    pub fn stats(&self) -> voltip_relay::RelayStats {
        voltip_relay::RelayStats { dropped: self.shared.dropped.load(Ordering::Relaxed), ..self.shared.core.lock().stats() }
    }

    /// Stop accepting and drop all connections. Returns once the accept loop has ended, so the
    /// listening socket is closed: an address announced to peers no longer answers.
    pub async fn shutdown(self) {
        self.accept_task.abort();
        self.tick_task.abort();
        let outbound: Vec<_> = self.shared.outbound.lock().drain().map(|(_, tx)| tx).collect();
        for tx in outbound {
            let _ = tx.try_send(Outbound::Close);
        }
        // An aborted task ends at its next await; its `JoinError` is the cancellation itself.
        let _ = self.accept_task.await;
        let _ = self.tick_task.await;
    }
}

/// Best-effort primary LAN IPv4: the source address the OS would use for an outbound datagram.
/// No packets are sent. Returns `None` on loopback-only hosts.
pub fn primary_lan_ip() -> Option<IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("10.255.255.255:9").ok()?;
    let ip = socket.local_addr().ok()?.ip();
    if ip.is_loopback() || ip.is_unspecified() {
        return None;
    }
    Some(ip)
}

fn dispatch(shared: &Shared, deliveries: Vec<Delivery>) {
    let outbound = shared.outbound.lock();
    for d in deliveries {
        let (conn, item) = match d {
            Delivery::Send(c, f) => (c, Outbound::Frame(f)),
            Delivery::Close(c) => (c, Outbound::Close),
        };
        if let Some(tx) = outbound.get(&conn)
            && tx.try_send(item).is_err()
        {
            shared.dropped.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(?conn, "outbound queue full; frame dropped");
        }
    }
}

async fn handle(stream: TcpStream, peer: SocketAddr, shared: Arc<Shared>) {
    let Ok(ws) = tokio_tungstenite::accept_async(stream).await else { return };
    let conn = ConnId(shared.next_id.fetch_add(1, Ordering::Relaxed));
    let (tx, mut rx) = mpsc::channel::<Outbound>(64);
    shared.outbound.lock().insert(conn, tx);
    shared.core.lock().on_connect(conn, peer.ip());
    let (mut sink, mut reader) = ws.split();
    let writer = tokio::spawn(async move {
        while let Some(item) = rx.recv().await {
            match item {
                Outbound::Frame(f) => {
                    let Ok(text) = f.encode() else { continue };
                    if sink.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                Outbound::Close => {
                    let _ = sink.send(Message::Close(None)).await;
                    break;
                }
            }
        }
    });
    let mut first = true;
    loop {
        let next = if first { tokio::time::timeout(Duration::from_secs(10), reader.next()).await.ok().flatten() } else { reader.next().await };
        first = false;
        let Some(Ok(msg)) = next else { break };
        let deliveries = match msg {
            Message::Text(t) => shared.core.lock().on_text(conn, &t, Instant::now()),
            Message::Binary(_) => vec![Delivery::Send(conn, RelayFrame::error(RelayErrorCode::Malformed))],
            Message::Close(_) => break,
            _ => Vec::new(),
        };
        let closes_self = deliveries.contains(&Delivery::Close(conn));
        dispatch(&shared, deliveries);
        if closes_self {
            break;
        }
    }
    let deliveries = shared.core.lock().on_disconnect(conn, Instant::now());
    shared.outbound.lock().remove(&conn);
    dispatch(&shared, deliveries);
    let _ = tokio::time::timeout(Duration::from_secs(2), writer).await;
}
