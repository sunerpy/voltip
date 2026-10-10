//! Thin axum WebSocket adapter around [`RelayCore`].
//!
//! One task per connection reads frames and hands them to the shared core; deliveries are
//! pushed to per-connection channels. A housekeeping task ticks the core every second.
//!
//! Every connection is pinged (`RelayConfig::ping_interval`) and closed once nothing has come from
//! it for `RelayConfig::idle_timeout` (docs/pairing.md 「中继侧的连接检测」): a device that went
//! away without closing its socket (another network, a frozen app, a broken path) is noticed,
//! its peers are told it is offline, and its place on the rendezvous channels is free for the
//! connection it opens next.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::State;
use axum::extract::connect_info::ConnectInfo;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use futures_util::{SinkExt as _, StreamExt as _};
use parking_lot::Mutex;
use tokio::sync::mpsc;
use voltip_protocol::relay::RelayFrame;

use crate::{ConnId, Delivery, RelayConfig, RelayCore, RelayStats};

/// Outbound queue depth per connection before we consider the client stuck.
const OUTBOUND_QUEUE: usize = 64;
/// Housekeeping cadence.
const TICK: Duration = Duration::from_secs(1);
/// Longest we wait for a client's first frame.
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);

/// Shared state behind the axum handlers.
#[derive(Clone)]
pub struct RelayHandle {
    inner: Arc<Shared>,
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

impl RelayHandle {
    /// New handle around a fresh core.
    pub fn new(config: RelayConfig) -> Self {
        Self {
            inner: Arc::new(Shared {
                core: Mutex::new(RelayCore::new(config)),
                outbound: Mutex::new(Default::default()),
                next_id: AtomicU64::new(1),
                dropped: AtomicU64::new(0),
            }),
        }
    }

    /// Counters.
    pub fn stats(&self) -> RelayStats {
        RelayStats { dropped: self.inner.dropped.load(Ordering::Relaxed), ..self.inner.core.lock().stats() }
    }

    /// The axum router: `GET /ws` upgrades, `GET /healthz` reports stats.
    pub fn router(&self) -> Router {
        Router::new().route("/ws", get(ws_upgrade)).route("/healthz", get(healthz)).with_state(self.clone())
    }

    /// Bind and serve until `shutdown` resolves. Returns the bound address once listening.
    pub async fn serve(
        self,
        bind: SocketAddr,
        shutdown: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> std::io::Result<(SocketAddr, tokio::task::JoinHandle<std::io::Result<()>>)> {
        let listener = tokio::net::TcpListener::bind(bind).await?;
        let addr = listener.local_addr()?;
        let handle = self.clone();
        let ticker = tokio::spawn(async move {
            let mut interval = tokio::time::interval(TICK);
            loop {
                interval.tick().await;
                let deliveries = handle.inner.core.lock().tick(Instant::now());
                handle.dispatch(deliveries);
            }
        });
        let app = self.router();
        let closer = self.clone();
        let shutdown = async move {
            shutdown.await;
            // Graceful shutdown alone leaves upgraded WebSockets open; tell every client to go.
            closer.close_all();
        };
        let task = tokio::spawn(async move {
            let result = axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).with_graceful_shutdown(shutdown).await;
            ticker.abort();
            result
        });
        tracing::info!(%addr, "relay listening");
        Ok((addr, task))
    }

    /// Send a WebSocket close to every live connection (shutdown).
    pub fn close_all(&self) {
        let outbound = self.inner.outbound.lock();
        for tx in outbound.values() {
            let _ = tx.try_send(Outbound::Close);
        }
    }

    fn dispatch(&self, deliveries: Vec<Delivery>) {
        let outbound = self.inner.outbound.lock();
        for d in deliveries {
            let (conn, item) = match d {
                Delivery::Send(c, f) => (c, Outbound::Frame(f)),
                Delivery::Close(c) => (c, Outbound::Close),
            };
            if let Some(tx) = outbound.get(&conn)
                && tx.try_send(item).is_err()
            {
                self.inner.dropped.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(?conn, "outbound queue full; dropping frame");
            }
        }
    }
}

async fn healthz(State(h): State<RelayHandle>) -> impl IntoResponse {
    let s = h.stats();
    axum::Json(serde_json::json!({ "connections": s.connections, "sessions": s.sessions, "channels": s.channels, "forwarded": s.forwarded }))
}

async fn ws_upgrade(ws: WebSocketUpgrade, ConnectInfo(addr): ConnectInfo<SocketAddr>, State(h): State<RelayHandle>) -> impl IntoResponse {
    ws.max_message_size(128 * 1024).on_upgrade(move |socket| handle_socket(socket, addr, h))
}

async fn handle_socket(socket: WebSocket, addr: SocketAddr, h: RelayHandle) {
    let conn = ConnId(h.inner.next_id.fetch_add(1, Ordering::Relaxed));
    let (tx, mut rx) = mpsc::channel::<Outbound>(OUTBOUND_QUEUE);
    h.inner.outbound.lock().insert(conn, tx);
    let (ping_interval, idle_timeout) = {
        let mut core = h.inner.core.lock();
        core.on_connect(conn, addr.ip());
        (core.config().ping_interval, core.config().idle_timeout)
    };
    tracing::debug!(?conn, %addr, "connection opened");

    let (mut sink, mut stream) = socket.split();
    let writer = tokio::spawn(async move {
        let mut ping = tokio::time::interval(ping_interval);
        ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ping.tick().await; // the first tick is immediate
        loop {
            tokio::select! {
                item = rx.recv() => match item {
                    Some(Outbound::Frame(f)) => {
                        let Ok(text) = f.encode() else { continue };
                        if sink.send(Message::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                    Some(Outbound::Close) => {
                        let _ = sink.send(Message::Close(None)).await;
                        break;
                    }
                    None => break,
                },
                _ = ping.tick() => {
                    if sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    let mut first = true;
    loop {
        // Any message counts as a sign of life, a pong included.
        let wait = if first { HELLO_TIMEOUT.min(idle_timeout) } else { idle_timeout };
        first = false;
        let next = match tokio::time::timeout(wait, stream.next()).await {
            Ok(next) => next,
            Err(_) => {
                tracing::info!(?conn, %addr, silent_for = ?wait, "connection silent; closing it");
                break;
            }
        };
        let Some(Ok(msg)) = next else { break };
        let deliveries = match msg {
            Message::Text(text) => h.inner.core.lock().on_text(conn, &text, Instant::now()),
            Message::Binary(_) => vec![Delivery::Send(conn, RelayFrame::error(voltip_protocol::relay::RelayErrorCode::Malformed))],
            Message::Close(_) => break,
            Message::Ping(_) | Message::Pong(_) => Vec::new(),
        };
        let closes_self = deliveries.contains(&Delivery::Close(conn));
        h.dispatch(deliveries);
        if closes_self {
            break;
        }
    }

    let deliveries = h.inner.core.lock().on_disconnect(conn, Instant::now());
    // Dropping our sender lets the writer drain whatever is queued (an error frame followed by
    // a close, typically) and then exit on its own; never abort it mid-flush.
    h.inner.outbound.lock().remove(&conn);
    h.dispatch(deliveries);
    let _ = tokio::time::timeout(Duration::from_secs(2), writer).await;
    tracing::debug!(?conn, "connection closed");
}
