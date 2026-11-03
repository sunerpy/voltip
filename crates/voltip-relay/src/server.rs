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
//!
//! A close the core asks for happens in two steps. Reading stops at once (a per-connection
//! [`CancellationToken`], which no full queue can hold up); the writer then sends what is queued,
//! the reason before the close, for up to [`CLOSE_GRACE`], and is aborted if it is still stuck on
//! a client that does not read. Either way the socket is gone soon after.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::State;
use axum::extract::connect_info::ConnectInfo;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::routing::get;
use futures_util::{Sink, SinkExt as _, StreamExt as _};
use ipnet::IpNet;
use parking_lot::Mutex;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use voltip_protocol::relay::RelayFrame;

use crate::policy::PolicyFiles;
use crate::stats::StatsReport;
use crate::{ConnId, Delivery, RelayConfig, RelayCore, RelayStats, forwarded};

/// How long a closing connection's writer may take to send what is queued.
pub const CLOSE_GRACE: Duration = Duration::from_secs(2);
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
    outbound: Mutex<std::collections::HashMap<ConnId, Slot>>,
    /// `RelayConfig::outbound_queue`.
    queue: usize,
    next_id: AtomicU64,
    /// Frames dropped on a full outbound queue.
    dropped: AtomicU64,
    /// Connection tasks still running (tests).
    live: AtomicUsize,
}

/// A connection's outbound queue and the token that stops its reading.
struct Slot {
    tx: mpsc::Sender<Outbound>,
    stop: CancellationToken,
}

enum Outbound {
    Frame(RelayFrame),
    Close,
}

impl RelayHandle {
    /// New handle around a fresh core.
    pub fn new(config: RelayConfig) -> Self {
        let trusted_proxies = config.trusted_proxies.clone();
        let queue = config.outbound_queue.max(1);
        Self {
            inner: Arc::new(Shared {
                core: Mutex::new(RelayCore::new(config)),
                policy: Mutex::new(None),
                trusted_proxies,
                outbound: Mutex::new(Default::default()),
                queue,
                next_id: AtomicU64::new(1),
                dropped: AtomicU64::new(0),
                live: AtomicUsize::new(0),
            }),
        }
    }

    /// Connection tasks still running, writer included (tests watch a closed one end).
    pub fn live_connections(&self) -> usize {
        self.inner.live.load(Ordering::Relaxed)
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
        for slot in outbound.values() {
            let _ = slot.tx.try_send(Outbound::Close);
            slot.stop.cancel();
        }
    }

    fn dispatch(&self, deliveries: Vec<Delivery>) {
        let outbound = self.inner.outbound.lock();
        for d in deliveries {
            let (conn, item, closing) = match d {
                Delivery::Send(c, f) => (c, Outbound::Frame(f), false),
                Delivery::Close(c) => (c, Outbound::Close, true),
            };
            let Some(slot) = outbound.get(&conn) else { continue };
            if slot.tx.try_send(item).is_err() {
                self.inner.dropped.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(?conn, "outbound queue full; dropping frame");
            }
            // The close itself cannot be lost to a full queue: reading stops here.
            if closing {
                slot.stop.cancel();
            }
        }
    }
}

async fn healthz(State(h): State<RelayHandle>) -> impl IntoResponse {
    let s = h.stats();
    let revision = h.inner.core.lock().policy().revision;
    axum::Json(serde_json::json!({
        "connections": s.connections,
        "sessions": s.sessions,
        "channels": s.channels,
        "forwarded": s.forwarded,
        "policy_revision": revision,
    }))
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

/// Counts a connection task for [`RelayHandle::live_connections`] until it is dropped.
struct LiveGuard(Arc<Shared>);

impl Drop for LiveGuard {
    fn drop(&mut self) {
        self.0.live.fetch_sub(1, Ordering::Relaxed);
    }
}

async fn handle_socket(socket: WebSocket, addr: SocketAddr, client: IpAddr, h: RelayHandle) {
    h.inner.live.fetch_add(1, Ordering::Relaxed);
    let _live = LiveGuard(h.inner.clone());
    let conn = ConnId(h.inner.next_id.fetch_add(1, Ordering::Relaxed));
    let (tx, rx) = mpsc::channel::<Outbound>(h.inner.queue);
    let stop = CancellationToken::new();
    h.inner.outbound.lock().insert(conn, Slot { tx, stop: stop.clone() });
    let (ping_interval, idle_timeout, refused) = {
        let mut core = h.inner.core.lock();
        let refused = core.on_connect(conn, client);
        (core.config().ping_interval, core.config().idle_timeout, refused)
    };
    tracing::debug!(?conn, %addr, %client, "connection opened");

    let (sink, mut stream) = socket.split();
    let writer = tokio::spawn(write_loop(sink, rx, ping_interval));
    h.dispatch(refused);

    let mut first = true;
    loop {
        // Any message counts as a sign of life, a pong included.
        let wait = if first { HELLO_TIMEOUT.min(idle_timeout) } else { idle_timeout };
        first = false;
        let next = tokio::select! {
            biased;
            () = stop.cancelled() => break,
            next = tokio::time::timeout(wait, stream.next()) => next,
        };
        let next = match next {
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
    // a close, typically) and then exit on its own, for up to the grace period.
    h.inner.outbound.lock().remove(&conn);
    h.dispatch(deliveries);
    drop(stream);
    if !finish_writer(writer, CLOSE_GRACE).await {
        tracing::info!(?conn, "client not reading; dropping the connection");
    }
    tracing::debug!(?conn, "connection closed");
}

/// Send what is queued, in order, and ping every `ping_interval`; end after a close, when the
/// queue's senders are gone, or when sending fails.
async fn write_loop<S>(mut sink: S, mut rx: mpsc::Receiver<Outbound>, ping_interval: Duration)
where
    S: Sink<Message> + Unpin,
{
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
}

/// Wait up to `grace` for the writer to finish; abort it if it has not (a send stuck on a client
/// that does not read), and wait for it to be gone either way so the socket is released. `true`
/// when it finished by itself.
async fn finish_writer(mut writer: JoinHandle<()>, grace: Duration) -> bool {
    if tokio::time::timeout(grace, &mut writer).await.is_ok() {
        return true;
    }
    writer.abort();
    let _ = writer.await;
    false
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use super::*;

    /// A sink that never takes anything: a client that stopped reading, its buffers full.
    struct Stuck;

    impl Sink<Message> for Stuck {
        type Error = std::io::Error;
        fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Pending
        }
        fn start_send(self: Pin<&mut Self>, _: Message) -> Result<(), Self::Error> {
            Ok(())
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Pending
        }
        fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Pending
        }
    }

    #[tokio::test]
    async fn a_writer_stuck_on_a_send_is_aborted_after_the_grace() {
        let (tx, rx) = mpsc::channel(4);
        tx.send(Outbound::Frame(RelayFrame::error(voltip_protocol::relay::RelayErrorCode::TicketRequired))).await.unwrap();
        tx.send(Outbound::Close).await.unwrap();
        let writer = tokio::spawn(write_loop(Stuck, rx, Duration::from_secs(3600)));
        let started = Instant::now();
        assert!(!finish_writer(writer, Duration::from_millis(100)).await, "aborted");
        assert!(started.elapsed() < Duration::from_secs(5));
        drop(tx);
    }

    #[tokio::test]
    async fn a_writer_sends_the_reason_then_the_close_and_ends_by_itself() {
        let (tx, rx) = mpsc::channel(4);
        let (out_tx, mut out_rx) = mpsc::channel::<Message>(8);
        tx.send(Outbound::Frame(RelayFrame::error(voltip_protocol::relay::RelayErrorCode::TicketRequired))).await.unwrap();
        tx.send(Outbound::Close).await.unwrap();
        let writer = tokio::spawn(write_loop(tokio_util::sync::PollSender::new(out_tx), rx, Duration::from_secs(3600)));
        assert!(finish_writer(writer, CLOSE_GRACE).await, "it ended without being aborted");
        let Some(Message::Text(text)) = out_rx.recv().await else { panic!("the reason") };
        assert!(text.contains("ticket_required"));
        assert!(matches!(out_rx.recv().await, Some(Message::Close(None))));
        drop(tx);
    }
}
