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
//!
//! With [`RelayHandle::watch_policy`] the policy files are read at once and every
//! [`POLICY_CHECK`] after (docs/pairing.md 「最低版本」); `GET /stats` answers the admin console's
//! counts. The listener is on loopback behind nginx, which forwards `/ws` only.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::State;
use axum::extract::connect_info::ConnectInfo;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::routing::get;
use futures_util::{SinkExt as _, StreamExt as _};
use ipnet::IpNet;
use parking_lot::Mutex;
use tokio::sync::mpsc;
use voltip_protocol::relay::RelayFrame;

use crate::policy::PolicyFiles;
use crate::stats::StatsReport;
use crate::{ConnId, Delivery, RelayConfig, RelayCore, RelayStats, forwarded};

/// Outbound queue depth per connection before we consider the client stuck.
const OUTBOUND_QUEUE: usize = 64;
/// Housekeeping cadence.
const TICK: Duration = Duration::from_secs(1);
/// How often the policy files are looked at.
pub const POLICY_CHECK: Duration = Duration::from_secs(30);
/// Longest we wait for a client's first frame.
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
/// Where a reverse proxy names the client (read only from `RelayConfig::trusted_proxies`).
const X_FORWARDED_FOR: &str = "x-forwarded-for";

/// Shared state behind the axum handlers.
#[derive(Clone)]
pub struct RelayHandle {
    inner: Arc<Shared>,
}

struct Shared {
    core: Mutex<RelayCore>,
    /// The policy files, once [`RelayHandle::watch_policy`] named them.
    policy: Mutex<Option<PolicyFiles>>,
    /// `RelayConfig::trusted_proxies`, read on every upgrade without taking the core's lock.
    trusted_proxies: Vec<IpNet>,
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
        let trusted_proxies = config.trusted_proxies.clone();
        Self {
            inner: Arc::new(Shared {
                core: Mutex::new(RelayCore::new(config)),
                policy: Mutex::new(None),
                trusted_proxies,
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

    /// The admin console's counts (`GET /stats`).
    pub fn stats_report(&self) -> StatsReport {
        self.inner.core.lock().stats_report()
    }

    /// Follow the policy in `files`: read now, and every [`POLICY_CHECK`] while serving.
    pub fn watch_policy(&self, files: PolicyFiles) {
        *self.inner.policy.lock() = Some(files);
        self.reload_policy();
    }

    /// Look at the policy files now; a changed policy is put in force (the connections it turns
    /// away are told and closed).
    pub fn reload_policy(&self) {
        let changed = self.inner.policy.lock().as_mut().and_then(PolicyFiles::poll);
        if let Some(policy) = changed {
            let deliveries = self.inner.core.lock().set_policy(policy);
            self.dispatch(deliveries);
        }
    }

    /// The axum router: `GET /ws` upgrades, `GET /healthz` reports stats, `GET /stats` the
    /// admin console's counts.
    pub fn router(&self) -> Router {
        Router::new().route("/ws", get(ws_upgrade)).route("/healthz", get(healthz)).route("/stats", get(stats)).with_state(self.clone())
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
            let mut policy_due = Instant::now() + POLICY_CHECK;
            loop {
                interval.tick().await;
                let deliveries = handle.inner.core.lock().tick(Instant::now());
                handle.dispatch(deliveries);
                if Instant::now() >= policy_due {
                    policy_due = Instant::now() + POLICY_CHECK;
                    handle.reload_policy();
                }
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
        tracing::info!(%addr, trusted_proxies = ?self.inner.trusted_proxies, "relay listening");
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

async fn stats(State(h): State<RelayHandle>) -> impl IntoResponse {
    axum::Json(h.stats_report())
}

async fn ws_upgrade(ws: WebSocketUpgrade, ConnectInfo(addr): ConnectInfo<SocketAddr>, headers: HeaderMap, State(h): State<RelayHandle>) -> impl IntoResponse {
    // The per-IP limits count against the client: behind a trusted proxy, the one it forwards.
    // Bytes that are not text stay in place as an entry that is not an address.
    let forwarded_for = headers.get_all(X_FORWARDED_FOR).iter().map(|v| String::from_utf8_lossy(v.as_bytes()));
    let client = forwarded::client_ip(addr.ip(), forwarded_for, &h.inner.trusted_proxies);
    ws.max_message_size(128 * 1024).on_upgrade(move |socket| handle_socket(socket, addr, client, h))
}

async fn handle_socket(socket: WebSocket, addr: SocketAddr, client: IpAddr, h: RelayHandle) {
    let conn = ConnId(h.inner.next_id.fetch_add(1, Ordering::Relaxed));
    let (tx, mut rx) = mpsc::channel::<Outbound>(OUTBOUND_QUEUE);
    h.inner.outbound.lock().insert(conn, tx);
    let (ping_interval, idle_timeout) = {
        let mut core = h.inner.core.lock();
        core.on_connect(conn, client);
        (core.config().ping_interval, core.config().idle_timeout)
    };
    tracing::debug!(?conn, %addr, %client, "connection opened");

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
