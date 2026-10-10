#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Link tests over real sockets.

use std::time::Duration;

use tokio::sync::mpsc;
use voltip_protocol::ProtocolVersion;
use voltip_protocol::relay::RelayFrame;
use voltip_relay::RelayConfig;
use voltip_relay::server::RelayHandle;
use voltip_transport::{ConnectionState, LinkConfig, LinkEvent, ReconnectPolicy, RelayEndpoint, RelayLink, TransportError};

async fn next_state(rx: &mut mpsc::Receiver<LinkEvent>, want: ConnectionState) {
    loop {
        let ev = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await.expect("event within 10 s").expect("channel open");
        if let LinkEvent::State(c) = ev
            && c.to == want
        {
            return;
        }
    }
}

async fn next_frame(rx: &mut mpsc::Receiver<LinkEvent>) -> RelayFrame {
    loop {
        let ev = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await.expect("event within 10 s").expect("channel open");
        if let LinkEvent::Frame(f) = ev {
            return f;
        }
    }
}

async fn relay() -> (RelayEndpoint, tokio::sync::oneshot::Sender<()>) {
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let handle = RelayHandle::new(RelayConfig::default());
    let (addr, _task) = handle
        .serve("127.0.0.1:0".parse().unwrap(), async move {
            let _ = stop_rx.await;
        })
        .await
        .unwrap();
    (RelayEndpoint::parse(&format!("ws://{addr}/ws")).unwrap(), stop_tx)
}

#[tokio::test]
async fn two_links_pair_through_a_relay() {
    let (endpoint, stop) = relay().await;
    let (a, mut ea) = RelayLink::spawn(LinkConfig::new(endpoint.clone()));
    let (b, mut eb) = RelayLink::spawn(LinkConfig::new(endpoint));
    assert!(
        matches!(a.send(RelayFrame::Bye { version: ProtocolVersion::CURRENT }).await.unwrap_err(), TransportError::NotConnected(_)),
        "send before connected fails"
    );
    next_state(&mut ea, ConnectionState::Connected).await;
    next_state(&mut eb, ConnectionState::Connected).await;
    assert_eq!(a.state(), ConnectionState::Connected);
    a.send(RelayFrame::CreateSession { version: ProtocolVersion::CURRENT, ttl_secs: None }).await.unwrap();
    let RelayFrame::SessionCreated { session_id, code, .. } = next_frame(&mut ea).await else { panic!() };
    b.send(RelayFrame::JoinByCode { version: ProtocolVersion::CURRENT, code }).await.unwrap();
    assert!(matches!(next_frame(&mut eb).await, RelayFrame::Joined { .. }));
    assert!(matches!(next_frame(&mut ea).await, RelayFrame::PeerJoined { .. }));
    a.send(RelayFrame::forward(session_id, vec![1, 2, 3])).await.unwrap();
    assert!(matches!(next_frame(&mut eb).await, RelayFrame::Forward { payload, .. } if payload == vec![1, 2, 3]));
    assert!(format!("{a:?}").contains("Connected"));
    b.close().await;
    assert!(matches!(next_frame(&mut ea).await, RelayFrame::PeerLeft { .. }));
    a.close().await;
    next_state(&mut ea, ConnectionState::Closed).await;
    a.close().await; // idempotent
    let _ = stop.send(());
}

#[tokio::test]
async fn link_reconnects_after_relay_restart_and_gives_up_when_told() {
    // Bind a port, run a relay, drop it, run another on the same port.
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let handle = RelayHandle::new(RelayConfig::default());
    let (addr, task) = handle
        .serve("127.0.0.1:0".parse().unwrap(), async move {
            let _ = stop_rx.await;
        })
        .await
        .unwrap();
    let endpoint = RelayEndpoint::parse(&format!("ws://{addr}/ws")).unwrap();
    let mut cfg = LinkConfig::new(endpoint.clone());
    cfg.reconnect = ReconnectPolicy { base: Duration::from_millis(50), max: Duration::from_millis(200), jitter: 0.0, max_attempts: Some(20) };
    let (link, mut ev) = RelayLink::spawn(cfg);
    next_state(&mut ev, ConnectionState::Connected).await;
    let _ = stop_tx.send(());
    let _ = task.await;
    next_state(&mut ev, ConnectionState::Reconnecting).await;
    // Bring a relay back on the same port.
    let (stop2_tx, stop2_rx) = tokio::sync::oneshot::channel::<()>();
    let handle2 = RelayHandle::new(RelayConfig::default());
    let (_addr2, _task2) = handle2
        .serve(addr, async move {
            let _ = stop2_rx.await;
        })
        .await
        .unwrap();
    next_state(&mut ev, ConnectionState::Connected).await;
    link.send(RelayFrame::CreateSession { version: ProtocolVersion::CURRENT, ttl_secs: None }).await.unwrap();
    assert!(matches!(next_frame(&mut ev).await, RelayFrame::SessionCreated { .. }));
    link.close().await;
    next_state(&mut ev, ConnectionState::Closed).await;
    let _ = stop2_tx.send(());

    // A link to nothing with a tiny attempt budget ends Closed.
    let dead = RelayEndpoint::parse("ws://127.0.0.1:1/ws").unwrap();
    let mut cfg = LinkConfig::new(dead);
    cfg.reconnect = ReconnectPolicy { base: Duration::from_millis(10), max: Duration::from_millis(10), jitter: 0.0, max_attempts: Some(2) };
    cfg.connect_timeout = Duration::from_secs(2);
    let (dead_link, mut dev) = RelayLink::spawn(cfg);
    next_state(&mut dev, ConnectionState::Closed).await;
    assert_eq!(dead_link.state(), ConnectionState::Closed);
    assert!(matches!(
        dead_link.send(RelayFrame::Bye { version: ProtocolVersion::CURRENT }).await.unwrap_err(),
        TransportError::NotConnected(ConnectionState::Closed)
    ));
    // Closing while reconnecting is honoured promptly.
    let dead = RelayEndpoint::parse("ws://127.0.0.1:1/ws").unwrap();
    let mut cfg = LinkConfig::new(dead);
    cfg.reconnect = ReconnectPolicy { base: Duration::from_secs(30), max: Duration::from_secs(30), jitter: 0.0, max_attempts: None };
    let (l, mut e) = RelayLink::spawn(cfg);
    next_state(&mut e, ConnectionState::Reconnecting).await;
    l.close().await;
    next_state(&mut e, ConnectionState::Closed).await;
}

/// A relay that answers `hello` and then stops reading altogether: a wedged relay, or the far end
/// of a path that broke without a word. It never sees our pings, so no pong ever comes back.
async fn silent_relay() -> RelayEndpoint {
    use futures_util::{SinkExt as _, StreamExt as _};
    use tokio_tungstenite::tungstenite::Message;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else { return };
                if let Some(Ok(Message::Text(t))) = ws.next().await
                    && t.contains(r#""type":"hello""#)
                {
                    let ack = RelayFrame::HelloAck {
                        version: ProtocolVersion::CURRENT,
                        relay_version: "silent".into(),
                        limits: voltip_protocol::relay::RelayLimits { code_attempts_per_connection: 5, code_attempts_per_session: 10, session_ttl_secs: 120 },
                    };
                    let _ = ws.send(Message::Text(ack.encode().unwrap().into())).await;
                }
                std::future::pending::<()>().await;
            });
        }
    });
    RelayEndpoint::parse(&format!("ws://{addr}/ws")).unwrap()
}

/// A relay that answers `hello` but never pongs must be detected by the heartbeat.
#[tokio::test]
async fn heartbeat_detects_a_silent_relay() {
    let mut cfg = LinkConfig::new(silent_relay().await);
    cfg.ping_interval = Duration::from_millis(100);
    cfg.pong_timeout = Duration::from_millis(200);
    cfg.reconnect = ReconnectPolicy { base: Duration::from_millis(50), max: Duration::from_millis(50), jitter: 0.0, max_attempts: Some(1) };
    let (link, mut ev) = RelayLink::spawn(cfg);
    next_state(&mut ev, ConnectionState::Connected).await;
    // The silent relay never pongs: the heartbeat must declare the socket dead.
    next_state(&mut ev, ConnectionState::Reconnecting).await;
    // A successful reconnect resets the attempt counter (by design), so a flapping relay is
    // retried forever; closing must still be honoured promptly.
    link.close().await;
    next_state(&mut ev, ConnectionState::Closed).await;
    assert_eq!(link.state(), ConnectionState::Closed);
}

/// The connectivity self-check's probe: a relay answers `hello`; a closed port is `Refused`, an
/// HTTP server that is not Voltip is `Failed`, and an address that swallows the connection is
/// `Timeout` within the deadline.
#[tokio::test]
async fn probe_tells_a_voltip_endpoint_from_a_closed_port_a_stranger_and_silence() {
    use voltip_transport::{ProbeFailure, probe};
    let (relay_ep, _stop) = relay().await;
    let took = probe(&LinkConfig::new(relay_ep)).await.unwrap();
    assert!(took < Duration::from_secs(5), "{took:?}");

    // A port that was just freed: nothing listens.
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    let closed = RelayEndpoint::parse(&format!("ws://{closed}/ws")).unwrap();
    assert_eq!(probe(&LinkConfig::new(closed)).await, Err(ProbeFailure::Refused));

    // Something that answers TCP but is not a WebSocket server.
    let stranger = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let stranger_addr = stranger.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = stranger.accept().await {
            use tokio::io::AsyncWriteExt as _;
            let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n").await;
        }
    });
    let stranger = RelayEndpoint::parse(&format!("ws://{stranger_addr}/ws")).unwrap();
    assert!(matches!(probe(&LinkConfig::new(stranger)).await, Err(ProbeFailure::Failed(_))));

    // A listener that accepts and then says nothing: the deadline ends the probe.
    let silent = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let silent_addr = silent.local_addr().unwrap();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = silent.accept().await {
            held.push(socket);
        }
    });
    let mut cfg = LinkConfig::new(RelayEndpoint::parse(&format!("ws://{silent_addr}/ws")).unwrap());
    cfg.connect_timeout = Duration::from_millis(300);
    assert_eq!(probe(&cfg).await, Err(ProbeFailure::Timeout));
}

/// `reconnect_now` while the link waits out a long backoff (docs/pairing.md 「重连」): it tries again
/// at once, and reaches the relay that has come back since, long before the backoff would end.
#[tokio::test]
async fn reconnect_now_cuts_a_backoff_short() {
    let addr = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    let mut cfg = LinkConfig::new(RelayEndpoint::parse(&format!("ws://{addr}/ws")).unwrap());
    cfg.reconnect = ReconnectPolicy { base: Duration::from_secs(600), max: Duration::from_secs(600), jitter: 0.0, max_attempts: None };
    let (link, mut ev) = RelayLink::spawn(cfg);
    next_state(&mut ev, ConnectionState::Reconnecting).await;
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let (_addr, _task) = RelayHandle::new(RelayConfig::default())
        .serve(addr, async move {
            let _ = stop_rx.await;
        })
        .await
        .unwrap();
    link.reconnect_now();
    next_state(&mut ev, ConnectionState::Connected).await;
    link.send(RelayFrame::CreateSession { version: ProtocolVersion::CURRENT, ttl_secs: None }).await.unwrap();
    assert!(matches!(next_frame(&mut ev).await, RelayFrame::SessionCreated { .. }));
    link.close().await;
    let _ = stop_tx.send(());
}

/// `reconnect_now` on a connected link whose socket died without a word (the path broke when the
/// network changed): the ping it sends at once goes unanswered, and the link reconnects after
/// `probe_timeout` instead of after the heartbeat's next ping and its pong timeout.
#[tokio::test]
async fn reconnect_now_drops_a_socket_that_no_longer_answers() {
    let mut cfg = LinkConfig::new(silent_relay().await);
    cfg.ping_interval = Duration::from_secs(600);
    cfg.pong_timeout = Duration::from_secs(600);
    cfg.probe_timeout = Duration::from_millis(200);
    let (link, mut ev) = RelayLink::spawn(cfg);
    next_state(&mut ev, ConnectionState::Connected).await;
    let asked = tokio::time::Instant::now();
    link.reconnect_now();
    next_state(&mut ev, ConnectionState::Reconnecting).await;
    assert!(asked.elapsed() < Duration::from_secs(60), "the probe, not the heartbeat: {:?}", asked.elapsed());
    link.close().await;
}

/// `reconnect_now` on a healthy link: the relay answers the ping and nothing else happens.
#[tokio::test]
async fn reconnect_now_leaves_a_working_link_alone() {
    let (endpoint, stop) = relay().await;
    let mut cfg = LinkConfig::new(endpoint);
    cfg.probe_timeout = Duration::from_millis(300);
    let (link, mut ev) = RelayLink::spawn(cfg);
    next_state(&mut ev, ConnectionState::Connected).await;
    for _ in 0..3 {
        link.reconnect_now();
    }
    // Well past the probe's deadline the link is still the same one.
    let lost = tokio::time::timeout(Duration::from_millis(1_500), next_state(&mut ev, ConnectionState::Reconnecting)).await;
    assert!(lost.is_err(), "a link whose relay answers stays up");
    link.send(RelayFrame::CreateSession { version: ProtocolVersion::CURRENT, ttl_secs: None }).await.unwrap();
    assert!(matches!(next_frame(&mut ev).await, RelayFrame::SessionCreated { .. }));
    link.close().await;
    let _ = stop.send(());
}
