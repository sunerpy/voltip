#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Real-socket tests: axum relay + tokio-tungstenite clients.

use std::time::Duration;

use futures_util::{SinkExt as _, StreamExt as _};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use voltip_protocol::relay::{RelayErrorCode, RelayFrame};
use voltip_protocol::{ProtocolVersion, SessionId};
use voltip_relay::forwarded::parse_trusted_proxy;
use voltip_relay::server::RelayHandle;
use voltip_relay::{PolicyFiles, RateWindow, RelayConfig};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn start() -> (RelayHandle, String, tokio::sync::oneshot::Sender<()>) {
    start_with(RelayConfig::default()).await
}

async fn start_with(config: RelayConfig) -> (RelayHandle, String, tokio::sync::oneshot::Sender<()>) {
    let handle = RelayHandle::new(config);
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let (addr, _task) = handle
        .clone()
        .serve("127.0.0.1:0".parse().unwrap(), async move {
            let _ = stop_rx.await;
        })
        .await
        .unwrap();
    (handle, format!("ws://{addr}/ws"), stop_tx)
}

async fn connect(url: &str) -> Ws {
    let (ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    ws
}

/// A connection that says it comes through a proxy for `forwarded_for`; its TCP peer is 127.0.0.1
/// like every other one here.
async fn connect_forwarded(url: &str, forwarded_for: &str) -> Ws {
    let mut request = url.into_client_request().unwrap();
    request.headers_mut().insert("x-forwarded-for", forwarded_for.parse().unwrap());
    let (ws, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    ws
}

async fn send(ws: &mut Ws, f: RelayFrame) {
    ws.send(Message::Text(f.encode().unwrap().into())).await.unwrap();
}

async fn recv(ws: &mut Ws) -> RelayFrame {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), ws.next()).await.expect("frame within 5 s").expect("stream open").expect("ws ok");
        match msg {
            Message::Text(t) => return RelayFrame::decode(&t).unwrap(),
            Message::Ping(_) | Message::Pong(_) => continue,
            other => panic!("unexpected message {other:?}"),
        }
    }
}

async fn hello(ws: &mut Ws) {
    send(ws, RelayFrame::hello("test")).await;
    assert!(matches!(recv(ws).await, RelayFrame::HelloAck { .. }));
}

#[tokio::test]
async fn pairing_session_over_real_sockets() {
    let (handle, url, stop) = start().await;
    let mut a = connect(&url).await;
    let mut b = connect(&url).await;
    hello(&mut a).await;
    hello(&mut b).await;
    send(&mut a, RelayFrame::CreateSession { version: ProtocolVersion::CURRENT, ttl_secs: Some(60) }).await;
    let RelayFrame::SessionCreated { session_id, code, .. } = recv(&mut a).await else { panic!() };
    send(&mut b, RelayFrame::JoinByCode { version: ProtocolVersion::CURRENT, code }).await;
    assert!(matches!(recv(&mut b).await, RelayFrame::Joined { session_id: s, .. } if s == session_id));
    assert!(matches!(recv(&mut a).await, RelayFrame::PeerJoined { session_id: s, .. } if s == session_id));
    // Opaque bytes travel untouched in both directions.
    send(&mut a, RelayFrame::forward(session_id, vec![0xde, 0xad])).await;
    assert!(matches!(recv(&mut b).await, RelayFrame::Forward { payload, .. } if payload == vec![0xde, 0xad]));
    send(&mut b, RelayFrame::forward(session_id, vec![0xbe, 0xef])).await;
    assert!(matches!(recv(&mut a).await, RelayFrame::Forward { payload, .. } if payload == vec![0xbe, 0xef]));
    // Health endpoint reflects the state.
    let stats = handle.stats();
    assert_eq!(stats.connections, 2);
    assert_eq!(stats.forwarded, 2);
    // B disconnects; A is told.
    b.close(None).await.unwrap();
    assert!(matches!(recv(&mut a).await, RelayFrame::PeerLeft { .. }));
    let _ = stop.send(());
}

#[tokio::test]
async fn hello_required_unsupported_version_and_binary_frames() {
    let (_handle, url, stop) = start().await;
    let mut a = connect(&url).await;
    send(&mut a, RelayFrame::CreateSession { version: ProtocolVersion::CURRENT, ttl_secs: None }).await;
    assert!(matches!(recv(&mut a).await, RelayFrame::Error { code: RelayErrorCode::HelloRequired, .. }));
    // Server closes afterwards.
    let closed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match a.next().await {
                Some(Ok(Message::Close(_))) | None => break true,
                Some(Ok(_)) => continue,
                Some(Err(_)) => break true,
            }
        }
    })
    .await
    .unwrap();
    assert!(closed);

    let mut b = connect(&url).await;
    b.send(Message::Text(r#"{"type":"hello","version":42,"client_version":"x"}"#.into())).await.unwrap();
    assert!(matches!(recv(&mut b).await, RelayFrame::Error { code: RelayErrorCode::UnsupportedVersion, .. }));

    let mut c = connect(&url).await;
    hello(&mut c).await;
    c.send(Message::Binary(vec![1, 2, 3].into())).await.unwrap();
    assert!(matches!(recv(&mut c).await, RelayFrame::Error { code: RelayErrorCode::Malformed, .. }));
    let _ = stop.send(());
}

#[tokio::test]
async fn channel_attach_and_presence_over_sockets() {
    let (_handle, url, stop) = start().await;
    let mut a = connect(&url).await;
    let mut b = connect(&url).await;
    hello(&mut a).await;
    hello(&mut b).await;
    let ch = "ab".repeat(32);
    send(&mut a, RelayFrame::Attach { version: ProtocolVersion::CURRENT, channel: ch.clone() }).await;
    let RelayFrame::Attached { session_id, peer_online: false, .. } = recv(&mut a).await else { panic!() };
    send(&mut b, RelayFrame::Attach { version: ProtocolVersion::CURRENT, channel: ch }).await;
    assert!(matches!(recv(&mut b).await, RelayFrame::Attached { peer_online: true, session_id: s, .. } if s == session_id));
    assert!(matches!(recv(&mut a).await, RelayFrame::PeerPresence { online: true, .. }));
    send(&mut b, RelayFrame::forward(session_id, vec![1])).await;
    assert!(matches!(recv(&mut a).await, RelayFrame::Forward { .. }));
    // Not-joined error for a random session id.
    send(&mut b, RelayFrame::forward(SessionId::random(), vec![1])).await;
    assert!(matches!(recv(&mut b).await, RelayFrame::Error { code: RelayErrorCode::NotJoined, .. }));
    drop(b);
    assert!(matches!(recv(&mut a).await, RelayFrame::PeerPresence { online: false, .. }));
    let _ = stop.send(());
}

#[tokio::test]
async fn healthz_reports_json() {
    let (handle, url, stop) = start().await;
    let base = url.trim_end_matches("/ws").replace("ws://", "http://");
    let mut a = connect(&url).await;
    hello(&mut a).await;
    let router = handle.router();
    let req = axum::http::Request::builder().uri("/healthz").body(axum::body::Body::empty()).unwrap();
    let resp = tower::ServiceExt::oneshot(router, req).await.unwrap();
    assert_eq!(resp.status(), 200);
    let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["connections"], 1);
    assert!(base.starts_with("http://127.0.0.1:"));
    let _ = stop.send(());
}

/// The server pings: a client that only answers pings (it sends nothing of its own) stays
/// connected well past the idle limit.
#[tokio::test]
async fn a_client_that_only_answers_pings_stays_connected() {
    let config = RelayConfig { ping_interval: Duration::from_millis(100), idle_timeout: Duration::from_millis(800), ..RelayConfig::default() };
    let (handle, url, stop) = start_with(config).await;
    let mut a = connect(&url).await;
    hello(&mut a).await;
    let mut pings = 0;
    let quiet = tokio::time::Instant::now() + Duration::from_millis(2_500);
    while let Ok(msg) = tokio::time::timeout_at(quiet, a.next()).await {
        match msg {
            // tungstenite answers each ping itself.
            Some(Ok(Message::Ping(_))) => pings += 1,
            other => panic!("the relay closed a connection that answers its pings: {other:?}"),
        }
    }
    assert!(pings >= 5, "{pings} pings in 2.5 s");
    assert_eq!(handle.stats().connections, 1);
    let _ = stop.send(());
}

/// Regression (2026-10-10, 「配对后断开」): a device that went away without closing its socket (it
/// changed networks, its app was frozen) kept its place on the rendezvous channel for as long as
/// the proxies in front kept the dead connection, an hour behind the ALB; the connection it opened
/// next was refused with `channel_full` and the two devices stayed apart. The relay now closes a
/// connection it hears nothing from, pongs included, and tells the other party.
#[tokio::test]
async fn regression_a_connection_gone_silent_gives_up_its_place_on_the_channel() {
    let config = RelayConfig { ping_interval: Duration::from_millis(200), idle_timeout: Duration::from_millis(1_500), ..RelayConfig::default() };
    let (_handle, url, stop) = start_with(config).await;
    let ch = "ab".repeat(32);
    let mut desk = connect(&url).await;
    hello(&mut desk).await;
    send(&mut desk, RelayFrame::Attach { version: ProtocolVersion::CURRENT, channel: ch.clone() }).await;
    assert!(matches!(recv(&mut desk).await, RelayFrame::Attached { peer_online: false, .. }));
    let mut gone = connect(&url).await;
    hello(&mut gone).await;
    send(&mut gone, RelayFrame::Attach { version: ProtocolVersion::CURRENT, channel: ch.clone() }).await;
    assert!(matches!(recv(&mut gone).await, RelayFrame::Attached { peer_online: true, .. }));
    assert!(matches!(recv(&mut desk).await, RelayFrame::PeerPresence { online: true, .. }));
    // `gone` is never read again, so it answers no ping: the socket is open and silent.
    let mut back = connect(&url).await;
    hello(&mut back).await;
    send(&mut back, RelayFrame::Attach { version: ProtocolVersion::CURRENT, channel: ch.clone() }).await;
    assert!(
        matches!(recv(&mut back).await, RelayFrame::Error { code: RelayErrorCode::ChannelFull, channel: Some(c), .. } if c == ch),
        "the old connection still holds its place"
    );
    // `desk` answers the pings (it is read) and hears the silent one leave.
    let left = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let RelayFrame::PeerPresence { online: false, .. } = recv(&mut desk).await {
                break;
            }
        }
    })
    .await;
    assert!(left.is_ok(), "the silent connection was closed and its peer told");
    send(&mut back, RelayFrame::Attach { version: ProtocolVersion::CURRENT, channel: ch.clone() }).await;
    assert!(matches!(recv(&mut back).await, RelayFrame::Attached { peer_online: true, channel: Some(c), .. } if c == ch));
    assert!(matches!(recv(&mut desk).await, RelayFrame::PeerPresence { online: true, .. }));
    drop(gone);
    let _ = stop.send(());
}

/// Opens a pairing session on a new connection and returns the relay's answer.
async fn create_session(ws: &mut Ws) -> RelayFrame {
    hello(ws).await;
    send(ws, RelayFrame::CreateSession { version: ProtocolVersion::CURRENT, ttl_secs: Some(60) }).await;
    recv(ws).await
}

fn one_session_a_minute() -> RelayConfig {
    RelayConfig { create_rate: RateWindow::new(1, Duration::from_secs(60)), ..RelayConfig::default() }
}

/// Regression (2026-10-12): the relay runs behind nginx on the same host, so every connection came
/// from 127.0.0.1 and the per-IP limits were one limit for all users together: one client could use
/// up everyone's pairing sessions and `attach`es for the minute. Behind a trusted proxy the client
/// is the address the proxy forwards.
#[tokio::test]
async fn regression_clients_behind_a_trusted_proxy_have_their_own_limits() {
    let config = RelayConfig { trusted_proxies: vec![parse_trusted_proxy("127.0.0.1").unwrap()], ..one_session_a_minute() };
    let (_handle, url, stop) = start_with(config).await;
    let mut first = connect_forwarded(&url, "203.0.113.1").await;
    assert!(matches!(create_session(&mut first).await, RelayFrame::SessionCreated { .. }));
    let mut other = connect_forwarded(&url, "203.0.113.2").await;
    assert!(matches!(create_session(&mut other).await, RelayFrame::SessionCreated { .. }), "another client has a limit of its own");
    // The first client again, with an address of its own choosing in front of the proxy's entry.
    let mut again = connect_forwarded(&url, "198.51.100.7, 203.0.113.1").await;
    assert!(matches!(create_session(&mut again).await, RelayFrame::Error { code: RelayErrorCode::RateLimited, .. }));
    let _ = stop.send(());
}

#[tokio::test]
async fn a_forwarded_address_counts_only_from_a_trusted_proxy() {
    let (_handle, url, stop) = start_with(one_session_a_minute()).await;
    let mut first = connect_forwarded(&url, "203.0.113.1").await;
    assert!(matches!(create_session(&mut first).await, RelayFrame::SessionCreated { .. }));
    // No proxy is trusted, so the header is the client's own claim and changes nothing.
    let mut other = connect_forwarded(&url, "203.0.113.2").await;
    assert!(matches!(create_session(&mut other).await, RelayFrame::Error { code: RelayErrorCode::RateLimited, .. }));
    let _ = stop.send(());
}

/// Wait until `ws` is closed by the relay.
async fn closed(ws: &mut Ws) -> bool {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break true,
                Some(Ok(_)) => continue,
            }
        }
    })
    .await
    .unwrap_or(false)
}

/// docs/pairing.md 「最低版本」 over real sockets: the policy file is read when the relay starts
/// watching it, a client below its minimum is refused at `hello`, and a raised minimum read again
/// turns a connected client away; `/stats` counts both.
#[tokio::test]
async fn the_policy_file_turns_old_clients_away_and_stats_count_them() {
    let dir = tempfile::tempdir().unwrap();
    let policy = dir.path().join("relay-policy.json");
    std::fs::write(&policy, r#"{"schema":1,"min_version":"0.1.10"}"#).unwrap();
    let (handle, url, stop) = start().await;
    handle.watch_policy(PolicyFiles::new(&policy, None));

    let mut old = connect(&url).await;
    send(&mut old, RelayFrame::hello("voltip/0.1.9")).await;
    assert_eq!(recv(&mut old).await, RelayFrame::client_too_old("0.1.10"));
    assert!(closed(&mut old).await);

    let mut current = connect(&url).await;
    send(&mut current, RelayFrame::hello("voltip/0.1.10")).await;
    assert!(matches!(recv(&mut current).await, RelayFrame::HelloAck { .. }));
    std::fs::write(&policy, r#"{"schema":1,"min_version":"0.2.0"}"#).unwrap();
    // A newer modification time whatever the file system's resolution.
    let file = std::fs::File::options().write(true).open(&policy).unwrap();
    file.set_modified(std::time::SystemTime::now() + Duration::from_secs(5)).unwrap();
    handle.reload_policy();
    assert_eq!(recv(&mut current).await, RelayFrame::client_too_old("0.2.0"));
    assert!(closed(&mut current).await);

    let req = axum::http::Request::builder().uri("/stats").body(axum::body::Body::empty()).unwrap();
    let resp = tower::ServiceExt::oneshot(handle.router(), req).await.unwrap();
    assert_eq!(resp.status(), 200);
    let v: serde_json::Value = serde_json::from_slice(&axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap()).unwrap();
    assert_eq!(v["schema"], 1);
    assert_eq!(v["hours"][0]["too_old"], 2);
    assert_eq!(v["hours"][0]["hellos"], 2);
    let _ = stop.send(());
}
