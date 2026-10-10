#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Real-socket tests: axum relay + tokio-tungstenite clients.

use std::time::Duration;

use futures_util::{SinkExt as _, StreamExt as _};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use voltip_protocol::relay::{RelayErrorCode, RelayFrame};
use voltip_protocol::{ProtocolVersion, SessionId};
use voltip_relay::RelayConfig;
use voltip_relay::server::RelayHandle;

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
    send(ws, RelayFrame::Hello { version: ProtocolVersion::CURRENT, client_version: "test".into() }).await;
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
