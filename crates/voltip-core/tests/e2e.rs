#![allow(clippy::unwrap_used, clippy::expect_used)]
//! End-to-end: two cores, a real relay, real sockets. Pairing by code and by ticket, E2EE
//! messaging, presence, reconnect (a path that breaks without a word included), forget, and the
//! identity-changed regression.

#[path = "support/net_cut.rs"]
mod net_cut;

use std::sync::Arc;
use std::time::Duration;

use net_cut::NetCut;
use tokio::sync::mpsc;
use voltip_core::{AppCore, CoreCommand, CoreConfig, CoreEvent, CoreHandle, DeviceConnection, Settings, SettingsStore};
use voltip_identity::MemorySecretStore;
use voltip_pairing::{PairingState, Snapshot};
use voltip_relay::RelayConfig;
use voltip_relay::server::RelayHandle;
use voltip_transport::ConnectionState;

struct Node {
    handle: CoreHandle,
    events: mpsc::Receiver<CoreEvent>,
    _dir: tempfile::TempDir,
    store: Arc<MemorySecretStore>,
    dir_path: std::path::PathBuf,
}

async fn relay() -> (String, tokio::sync::oneshot::Sender<()>, RelayHandle) {
    let (url, _addr, stop, handle) = relay_with(RelayConfig::default()).await;
    (url, stop, handle)
}

/// A relay with `config`, and the address it listens on.
async fn relay_with(config: RelayConfig) -> (String, std::net::SocketAddr, tokio::sync::oneshot::Sender<()>, RelayHandle) {
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let handle = RelayHandle::new(config);
    let (addr, _task) = handle
        .clone()
        .serve("127.0.0.1:0".parse().unwrap(), async move {
            let _ = stop_rx.await;
        })
        .await
        .unwrap();
    (format!("ws://{addr}/ws"), addr, stop_tx, handle)
}

fn node_with(dir: tempfile::TempDir, store: Arc<MemorySecretStore>, name: &str, relay_url: Option<&str>) -> Node {
    let settings = Settings { relay_url: relay_url.map(str::to_owned), relay_enabled: relay_url.is_some(), ..Settings::default() };
    node_tuned(dir, store, name, &settings, |_| {})
}

/// A node with the test timings, `settings` saved first, and `tune` applied last.
fn node_tuned(dir: tempfile::TempDir, store: Arc<MemorySecretStore>, name: &str, settings: &Settings, tune: impl FnOnce(&mut CoreConfig)) -> Node {
    trace_init();
    let dir_path = dir.path().to_path_buf();
    SettingsStore::new(&dir_path).save(settings).unwrap();
    let mut cfg = CoreConfig::new(dir_path.clone());
    cfg.default_device_name = name.into();
    cfg.tick = Duration::from_millis(50);
    cfg.peer_handshake_timeout = Duration::from_millis(600);
    tune(&mut cfg);
    let (handle, events) = AppCore::start(cfg, store.clone()).unwrap();
    Node { handle, events, _dir: dir, store, dir_path }
}

async fn pair_by_code(desk: &mut Node, phone: &mut Node) -> (voltip_identity::TrustedDevice, voltip_identity::TrustedDevice) {
    desk.handle.send(CoreCommand::StartPairing).await.unwrap();
    let code = wait_pairing(desk, PairingState::WaitingForPeer).await.code.unwrap();
    phone.handle.send(CoreCommand::JoinWithCode(code)).await.unwrap();
    wait_pairing(desk, PairingState::AwaitingVerification).await;
    wait_pairing(phone, PairingState::AwaitingVerification).await;
    desk.handle.send(CoreCommand::ConfirmPairing).await.unwrap();
    phone.handle.send(CoreCommand::ConfirmPairing).await.unwrap();
    let td = wait(desk, |e| if let CoreEvent::Trusted(d) = e { Some(d.clone()) } else { None }).await;
    let tp = wait(phone, |e| if let CoreEvent::Trusted(d) = e { Some(d.clone()) } else { None }).await;
    (td, tp)
}

/// `E2E_TRACE=1` prints core events and `voltip=debug` tracing to stderr.
fn trace_init() {
    if std::env::var_os("E2E_TRACE").is_some() {
        let _ = tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::new("voltip=debug")).with_test_writer().try_init();
    }
}

fn node(name: &str, relay_url: Option<&str>) -> Node {
    node_with(tempfile::tempdir().unwrap(), Arc::new(MemorySecretStore::new()), name, relay_url)
}

async fn wait<T>(node: &mut Node, mut pick: impl FnMut(&CoreEvent) -> Option<T>) -> T {
    loop {
        let ev = tokio::time::timeout(Duration::from_secs(15), node.events.recv()).await.expect("event within 15 s").expect("core alive");
        if std::env::var_os("E2E_TRACE").is_some() {
            eprintln!("[event] {ev:?}");
        }
        if let Some(v) = pick(&ev) {
            return v;
        }
    }
}

async fn wait_pairing(node: &mut Node, state: PairingState) -> Snapshot {
    wait(node, |e| match e {
        CoreEvent::Pairing(s) if s.state == state => Some(s.clone()),
        _ => None,
    })
    .await
}

async fn wait_online(node: &mut Node) {
    wait(node, |e| match e {
        CoreEvent::Devices(list) if list.iter().any(|d| matches!(d.connection, DeviceConnection::Online)) => Some(()),
        _ => None,
    })
    .await;
}

async fn wait_offline(node: &mut Node) {
    wait(node, |e| match e {
        CoreEvent::Devices(list) if !list.is_empty() && list.iter().all(|d| matches!(d.connection, DeviceConnection::Offline)) => Some(()),
        _ => None,
    })
    .await;
}

/// Presence, reconnect and forget, all through the relay.
#[tokio::test]
async fn pair_by_code_message_presence_reconnect_forget() {
    let (url, _stop, relay) = relay().await;
    let mut desk = node_with(tempfile::tempdir().unwrap(), Arc::new(MemorySecretStore::new()), "Surface-Laptop", Some(&url));
    let mut phone = node_with(tempfile::tempdir().unwrap(), Arc::new(MemorySecretStore::new()), "Pixel 10", Some(&url));
    let ready =
        wait(&mut desk, |e| if let CoreEvent::Ready { identity, secret_backend, .. } = e { Some((identity.clone(), *secret_backend)) } else { None }).await;
    assert_eq!(ready.0.name, "Surface-Laptop");
    assert_eq!(ready.1, "memory");
    wait(&mut desk, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    wait(&mut phone, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;

    desk.handle.send(CoreCommand::StartPairing).await.unwrap();
    let waiting = wait_pairing(&mut desk, PairingState::WaitingForPeer).await;
    let code = waiting.code.clone().unwrap();
    assert!(waiting.ticket_uri.as_deref().unwrap().starts_with("voltip://pair?"));
    assert!(waiting.remaining_secs.unwrap() > 100);
    // A second StartPairing while one is running is refused.
    desk.handle.send(CoreCommand::StartPairing).await.unwrap();
    wait(&mut desk, |e| matches!(e, CoreEvent::Error(m) if m.contains("已有配对正在进行")).then_some(())).await;

    phone.handle.send(CoreCommand::JoinWithCode(code)).await.unwrap();
    let vd = wait_pairing(&mut desk, PairingState::AwaitingVerification).await;
    let vp = wait_pairing(&mut phone, PairingState::AwaitingVerification).await;
    assert_eq!(vd.safety_code, vp.safety_code);
    assert!(vd.safety_code.is_some());

    desk.handle.send(CoreCommand::ConfirmPairing).await.unwrap();
    phone.handle.send(CoreCommand::ConfirmPairing).await.unwrap();
    let td = wait(&mut desk, |e| if let CoreEvent::Trusted(d) = e { Some(d.clone()) } else { None }).await;
    let tp = wait(&mut phone, |e| if let CoreEvent::Trusted(d) = e { Some(d.clone()) } else { None }).await;
    assert_eq!(td.name, "Pixel 10");
    assert_eq!(tp.name, "Surface-Laptop");
    wait_pairing(&mut desk, PairingState::Trusted).await;
    wait_pairing(&mut phone, PairingState::Trusted).await;
    // Live channel comes up via the rendezvous channel on both sides.
    wait_online(&mut desk).await;
    wait_online(&mut phone).await;
    // Small settle so both handshakes are through before we send.
    tokio::time::sleep(Duration::from_millis(200)).await;
    desk.handle.send(CoreCommand::SendText { to: td.public_key, body: "把 fetchUser 改成 async".into() }).await.unwrap();
    let body = wait(&mut phone, |e| {
        if let CoreEvent::Message { body, from } = e {
            assert_eq!(*from, tp.public_key);
            Some(body.clone())
        } else {
            None
        }
    })
    .await;
    assert_eq!(body, "把 fetchUser 改成 async");
    phone.handle.send(CoreCommand::SendText { to: tp.public_key, body: "ok".into() }).await.unwrap();
    assert_eq!(wait(&mut desk, |e| if let CoreEvent::Message { body, .. } = e { Some(body.clone()) } else { None }).await, "ok");
    // The relay only ever forwarded ciphertext: its counters moved, but nothing else it holds.
    assert!(relay.stats().forwarded >= 5);

    desk.handle.send(CoreCommand::ResetPairing).await.unwrap();
    wait_pairing(&mut desk, PairingState::Idle).await;

    // Phone disables its relay -> desktop sees it offline; re-enabling brings it back.
    phone.handle.send(CoreCommand::SetRelay { url: Some(url.clone()), enabled: false }).await.unwrap();
    wait_offline(&mut desk).await;
    desk.handle.send(CoreCommand::SendText { to: td.public_key, body: "lost".into() }).await.unwrap();
    wait(&mut desk, |e| matches!(e, CoreEvent::Error(m) if m.contains("不在线")).then_some(())).await;
    phone.handle.send(CoreCommand::SetRelay { url: Some(url.clone()), enabled: true }).await.unwrap();
    wait_online(&mut desk).await;
    wait_online(&mut phone).await;

    // Forget on the desktop: its list empties, and the phone, told over the channel before it
    // closed, forgets the desktop too (docs/pairing.md: unpairing is mutual) and says who did it.
    desk.handle.send(CoreCommand::ForgetDevice(td.public_key)).await.unwrap();
    wait(&mut desk, |e| matches!(e, CoreEvent::Devices(l) if l.is_empty()).then_some(())).await;
    let by = wait(&mut phone, |e| if let CoreEvent::Unpaired(d) = e { Some(d.clone()) } else { None }).await;
    assert_eq!((by.public_key, by.name.as_str()), (tp.public_key, "Surface-Laptop"));
    phone.handle.send(CoreCommand::RefreshDevices).await.unwrap();
    wait(&mut phone, |e| matches!(e, CoreEvent::Devices(l) if l.is_empty()).then_some(())).await;
    // Rename + theme round-trip.
    desk.handle.send(CoreCommand::RenameDevice("Studio".into())).await.unwrap();
    wait(&mut desk, |e| matches!(e, CoreEvent::Identity(i) if i.name == "Studio").then_some(())).await;
    desk.handle.send(CoreCommand::SetTheme { theme: voltip_core::ThemeId::Dark, follow_system: true }).await.unwrap();
    wait(&mut desk, |e| matches!(e, CoreEvent::Settings(s) if s.theme == voltip_core::ThemeId::Dark && s.follow_system_theme).then_some(())).await;
    // Locale and auto-update persist and echo as `Settings`, leaving the other fields alone.
    desk.handle.send(CoreCommand::SetLocale(voltip_core::Locale::En)).await.unwrap();
    wait(&mut desk, |e| matches!(e, CoreEvent::Settings(s) if s.locale == voltip_core::Locale::En && s.theme == voltip_core::ThemeId::Dark).then_some(()))
        .await;
    desk.handle.send(CoreCommand::SetAutoUpdate(true)).await.unwrap();
    wait(&mut desk, |e| matches!(e, CoreEvent::Settings(s) if s.auto_update && s.locale == voltip_core::Locale::En).then_some(())).await;
    // The pill's placement (Settings › 外观) is a core setting the desktop shell reads.
    desk.handle.send(CoreCommand::SetOverlay(voltip_core::OverlayPlacement::Top)).await.unwrap();
    wait(&mut desk, |e| matches!(e, CoreEvent::Settings(s) if s.overlay == voltip_core::OverlayPlacement::Top && s.auto_update).then_some(())).await;
    let saved = voltip_core::SettingsStore::new(&desk.dir_path).load().unwrap();
    assert!(saved.auto_update && saved.locale == voltip_core::Locale::En, "persisted to settings.json");
    assert_eq!(saved.overlay, voltip_core::OverlayPlacement::Top);
    desk.handle.send(CoreCommand::SetRelay { url: Some("ftp://bad".into()), enabled: true }).await.unwrap();
    wait(&mut desk, |e| matches!(e, CoreEvent::Error(m) if m.contains("endpoint")).then_some(())).await;
    desk.handle.send(CoreCommand::Shutdown).await.unwrap();
    phone.handle.send(CoreCommand::Shutdown).await.unwrap();
}

#[tokio::test]
async fn pair_by_ticket_then_reject_and_replay() {
    let (url, _stop, _relay) = relay().await;
    let mut desk = node("Mac Studio", Some(&url));
    let mut phone = node("iPhone", Some(&url));
    wait(&mut desk, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    wait(&mut phone, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    desk.handle.send(CoreCommand::StartPairing).await.unwrap();
    let waiting = wait_pairing(&mut desk, PairingState::WaitingForPeer).await;
    let uri = waiting.ticket_uri.unwrap();
    phone.handle.send(CoreCommand::JoinWithTicket(uri.clone())).await.unwrap();
    wait_pairing(&mut desk, PairingState::AwaitingVerification).await;
    wait_pairing(&mut phone, PairingState::AwaitingVerification).await;
    // Phone user says the words do not match.
    phone.handle.send(CoreCommand::RejectPairing).await.unwrap();
    wait_pairing(&mut phone, PairingState::Rejected).await;
    wait_pairing(&mut desk, PairingState::Rejected).await;
    phone.handle.send(CoreCommand::RefreshDevices).await.unwrap();
    wait(&mut phone, |e| matches!(e, CoreEvent::Devices(l) if l.is_empty()).then_some(())).await;
    // Replaying the same ticket is refused before any network I/O.
    phone.handle.send(CoreCommand::ResetPairing).await.unwrap();
    wait_pairing(&mut phone, PairingState::Idle).await;
    phone.handle.send(CoreCommand::JoinWithTicket(uri)).await.unwrap();
    let snap = wait(&mut phone, |e| {
        if let CoreEvent::Pairing(s) = e { if matches!(s.state, PairingState::Failed { .. }) { Some(s.clone()) } else { None } } else { None }
    })
    .await;
    assert_eq!(snap.state, PairingState::Failed { reason: voltip_pairing::FailureReason::Replay });
    // Garbage inputs are reported, not fatal.
    phone.handle.send(CoreCommand::ResetPairing).await.unwrap();
    phone.handle.send(CoreCommand::JoinWithTicket("https://evil.example".into())).await.unwrap();
    wait(&mut phone, |e| matches!(e, CoreEvent::Error(m) if m.contains("scheme")).then_some(())).await;
    phone.handle.send(CoreCommand::JoinWithCode("12".into())).await.unwrap();
    wait(&mut phone, |e| matches!(e, CoreEvent::Error(m) if m.contains("code")).then_some(())).await;
    phone.handle.send(CoreCommand::ConfirmPairing).await.unwrap();
    wait(&mut phone, |e| matches!(e, CoreEvent::Error(m) if m.contains("没有进行中的配对")).then_some(())).await;
    // Cancel from the desktop side.
    desk.handle.send(CoreCommand::ResetPairing).await.unwrap();
    desk.handle.send(CoreCommand::StartPairing).await.unwrap();
    wait_pairing(&mut desk, PairingState::WaitingForPeer).await;
    desk.handle.send(CoreCommand::CancelPairing).await.unwrap();
    wait(&mut desk, |e| matches!(e, CoreEvent::Pairing(s) if matches!(s.state, PairingState::Failed { .. })).then_some(())).await;
}

#[tokio::test]
async fn regression_relay_compromise_impostor_on_channel_is_flagged_not_trusted() {
    use futures_util::{SinkExt as _, StreamExt as _};
    use tokio_tungstenite::tungstenite::Message;
    use voltip_crypto::{Handshake, HandshakeStep, Role, StaticKeypair};
    use voltip_protocol::ProtocolVersion;
    use voltip_protocol::relay::RelayFrame;

    let (url, _stop, _relay) = relay().await;
    let mut desk = node("Desk", Some(&url));
    let mut phone = node("Phone", Some(&url));
    let desk_id = wait(&mut desk, |e| if let CoreEvent::Ready { identity, .. } = e { Some(identity.clone()) } else { None }).await;
    wait(&mut desk, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    wait(&mut phone, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    desk.handle.send(CoreCommand::StartPairing).await.unwrap();
    let code = wait_pairing(&mut desk, PairingState::WaitingForPeer).await.code.unwrap();
    phone.handle.send(CoreCommand::JoinWithCode(code)).await.unwrap();
    wait_pairing(&mut desk, PairingState::AwaitingVerification).await;
    wait_pairing(&mut phone, PairingState::AwaitingVerification).await;
    desk.handle.send(CoreCommand::ConfirmPairing).await.unwrap();
    phone.handle.send(CoreCommand::ConfirmPairing).await.unwrap();
    let phone_record = wait(&mut desk, |e| if let CoreEvent::Trusted(d) = e { Some(d.clone()) } else { None }).await;
    wait_online(&mut desk).await;
    // The phone goes away for good.
    phone.handle.send(CoreCommand::Shutdown).await.unwrap();
    wait_offline(&mut desk).await;

    // An attacker who controls the relay knows the channel label (it is routing metadata) and
    // attaches with its own identity. The desktop picks its Noise role from the *trusted* key
    // (it cannot know who is really there), so the impostor takes the complementary role.
    let impostor = StaticKeypair::generate().unwrap();
    let desk_initiates = voltip_core::is_initiator(&desk_id.public_key, &phone_record.public_key);
    let channel = voltip_core::rendezvous_channel(&desk_id.public_key, &phone_record.public_key);
    let (mut ws, _) = tokio_tungstenite::connect_async(url.clone()).await.unwrap();
    ws.send(Message::Text(RelayFrame::Hello { version: ProtocolVersion::CURRENT, client_version: "evil".into() }.encode().unwrap().into())).await.unwrap();
    let _ack = ws.next().await;
    ws.send(Message::Text(RelayFrame::Attach { version: ProtocolVersion::CURRENT, channel }.encode().unwrap().into())).await.unwrap();
    let mut hs = Handshake::new(if desk_initiates { Role::Responder } else { Role::Initiator }, &impostor, None).unwrap();
    let mut session_id = None;
    let mut done = false;
    while !done {
        let msg = tokio::time::timeout(Duration::from_secs(10), ws.next()).await.unwrap().unwrap().unwrap();
        let Message::Text(t) = msg else { continue };
        let frame = RelayFrame::decode(&t).unwrap();
        match frame {
            RelayFrame::Attached { session_id: sid, .. } => {
                session_id = Some(sid);
                if let HandshakeStep::Send(out) = hs.next_step().unwrap() {
                    ws.send(Message::Text(RelayFrame::forward(sid, out).encode().unwrap().into())).await.unwrap();
                }
            }
            RelayFrame::Forward { payload, .. } => {
                hs.receive(&payload).unwrap();
                if let HandshakeStep::Send(out) = hs.next_step().unwrap() {
                    ws.send(Message::Text(RelayFrame::forward(session_id.unwrap(), out).encode().unwrap().into())).await.unwrap();
                }
                if hs.is_finished() {
                    done = true;
                }
            }
            _ => {}
        }
    }
    let flagged = wait(&mut desk, |e| {
        if let CoreEvent::IdentityChanged { previous, presented_fingerprint } = e { Some((previous.clone(), presented_fingerprint.clone())) } else { None }
    })
    .await;
    assert_eq!(flagged.0.public_key, phone_record.public_key);
    assert_eq!(flagged.1, impostor.public.fingerprint());
    let list = wait(&mut desk, |e| if let CoreEvent::Devices(l) = e { Some(l.clone()) } else { None }).await;
    assert!(matches!(list[0].connection, DeviceConnection::IdentityChanged { .. }));
    // The impostor never gets a message: SendText is refused.
    desk.handle.send(CoreCommand::SendText { to: phone_record.public_key, body: "secret".into() }).await.unwrap();
    wait(&mut desk, |e| matches!(e, CoreEvent::Error(m) if m.contains("不在线")).then_some(())).await;
}

/// Without a relay there is no pairing (docs/pairing.md 「只走中继」): starting one or joining with
/// a code says why at once.
#[tokio::test]
async fn pairing_without_a_relay_is_refused_with_the_reason() {
    let mut desk = node("Desk", None);
    let mut phone = node("Phone", None);
    let r = wait(&mut desk, |e| if let CoreEvent::Relay(r) = e { Some(r.clone()) } else { None }).await;
    assert_eq!(r.state, ConnectionState::Disconnected);
    assert!(r.endpoint.is_none());
    desk.handle.send(CoreCommand::StartPairing).await.unwrap();
    wait(&mut desk, |e| matches!(e, CoreEvent::Error(m) if m.contains("配对需要中继")).then_some(())).await;
    phone.handle.send(CoreCommand::JoinWithCode("483921".into())).await.unwrap();
    wait(&mut phone, |e| matches!(e, CoreEvent::Error(m) if m.contains("配对需要中继")).then_some(())).await;
    desk.handle.send(CoreCommand::Shutdown).await.unwrap();
    phone.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// Trusted devices survive a restart of the core (same data directory, same secret store), and
/// the restarted side meets the other again on their channel, whichever of the two restarts.
#[tokio::test]
async fn restarted_cores_keep_their_devices_and_meet_again_over_the_relay() {
    let (url, _stop, _relay) = relay().await;
    let mut desk = node("Desk", Some(&url));
    let mut phone = node("Phone", Some(&url));
    for n in [&mut desk, &mut phone] {
        wait(n, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    }
    let (phone_on_desk, desk_on_phone) = pair_by_code(&mut desk, &mut phone).await;
    wait_online(&mut desk).await;
    wait_online(&mut phone).await;
    desk.handle.send(CoreCommand::Shutdown).await.unwrap();
    wait_offline(&mut phone).await;
    let mut desk = node_with(desk._dir, desk.store.clone(), "ignored", Some(&url));
    let list = wait(&mut desk, |e| if let CoreEvent::Devices(l) = e { Some(l.clone()) } else { None }).await;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].device.name, "Phone");
    let file = std::fs::read_to_string(desk.dir_path.join("trusted-devices.json")).unwrap();
    assert!(file.contains("Phone") && !file.contains("direct_hints") && !file.contains("last_connection"), "{file}");
    let online = async {
        wait_online(&mut desk).await;
        wait_online(&mut phone).await;
    };
    tokio::time::timeout(Duration::from_secs(30), online).await.expect("the restarted computer is back");
    tokio::time::sleep(Duration::from_millis(200)).await;
    desk.handle.send(CoreCommand::SendText { to: phone_on_desk.public_key, body: "still here".into() }).await.unwrap();
    assert_eq!(wait(&mut phone, |e| if let CoreEvent::Message { body, .. } = e { Some(body.clone()) } else { None }).await, "still here");
    // Now the phone restarts.
    phone.handle.send(CoreCommand::Shutdown).await.unwrap();
    wait_offline(&mut desk).await;
    let mut phone = node_with(phone._dir, phone.store.clone(), "ignored", Some(&url));
    let online = async {
        wait_online(&mut phone).await;
        wait_online(&mut desk).await;
    };
    tokio::time::timeout(Duration::from_secs(30), online).await.expect("the restarted phone is back");
    tokio::time::sleep(Duration::from_millis(200)).await;
    phone.handle.send(CoreCommand::SendText { to: desk_on_phone.public_key, body: "phone is back".into() }).await.unwrap();
    assert_eq!(wait(&mut desk, |e| if let CoreEvent::Message { body, .. } = e { Some(body.clone()) } else { None }).await, "phone is back");
    desk.handle.send(CoreCommand::Shutdown).await.unwrap();
    phone.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// The next pairing snapshot other than `Idle` within `within`, if any.
async fn next_pairing_within(node: &mut Node, within: Duration) -> Option<Snapshot> {
    tokio::time::timeout(within, async {
        loop {
            match node.events.recv().await {
                Some(CoreEvent::Pairing(s)) if s.state != PairingState::Idle => return s,
                Some(_) => {}
                None => panic!("core gone"),
            }
        }
    })
    .await
    .ok()
}

/// docs/pairing.md 「常开配对」: with always-on pairing a desktop keeps a session waiting (from the
/// switch, and on its own at start), renews it before it lapses (it never shows expired), opens
/// the next one shortly after a phone paired, and closes the waiting one when switched off. On at
/// start before the relay is there, it opens a session once the relay is. A phone cannot turn it
/// on.
#[tokio::test]
async fn always_on_pairing_keeps_a_session_waiting_until_switched_off() {
    use voltip_protocol::ticket::PairingTicket;
    let (url, _stop, _relay) = relay().await;
    let settings = Settings { relay_url: Some(url.clone()), relay_enabled: true, ..Settings::default() };
    // A 12 s session is renewed with 10 s left, about 2 s after it opens.
    let short = |cfg: &mut CoreConfig| cfg.pairing_timeouts.session_ttl = Duration::from_secs(12);
    let mut desk = node_tuned(tempfile::tempdir().unwrap(), Arc::new(MemorySecretStore::new()), "Desk", &settings, short);
    let mut phone = node_tuned(tempfile::tempdir().unwrap(), Arc::new(MemorySecretStore::new()), "Phone", &settings, |cfg| cfg.accepts_phone_takes = false);
    wait(&mut desk, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    wait(&mut phone, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    phone.handle.send(CoreCommand::SetPairingAlwaysOn(true)).await.unwrap();
    wait(&mut phone, |e| matches!(e, CoreEvent::Error(m) if m.contains("只在电脑上")).then_some(())).await;

    desk.handle.send(CoreCommand::SetPairingAlwaysOn(true)).await.unwrap();
    wait(&mut desk, |e| matches!(e, CoreEvent::Settings(s) if s.pairing_always_on).then_some(())).await;
    assert!(std::fs::read_to_string(desk.dir_path.join("settings.json")).unwrap().contains(r#""pairing_always_on": true"#));
    let first = wait_pairing(&mut desk, PairingState::WaitingForPeer).await;
    let renewed = wait(&mut desk, |e| match e {
        CoreEvent::Pairing(s) if s.state == PairingState::Expired => panic!("an always-on session must not lapse"),
        CoreEvent::Pairing(s) if s.state == PairingState::WaitingForPeer && s.session_id != first.session_id => Some(s.clone()),
        _ => None,
    })
    .await;
    // A phone pairs; the next session opens shortly after.
    phone.handle.send(CoreCommand::JoinWithCode(renewed.code.clone().unwrap())).await.unwrap();
    wait_pairing(&mut desk, PairingState::AwaitingVerification).await;
    wait_pairing(&mut phone, PairingState::AwaitingVerification).await;
    desk.handle.send(CoreCommand::ConfirmPairing).await.unwrap();
    phone.handle.send(CoreCommand::ConfirmPairing).await.unwrap();
    wait_pairing(&mut desk, PairingState::Trusted).await;
    let next = wait_pairing(&mut desk, PairingState::WaitingForPeer).await;
    assert_ne!(next.session_id, renewed.session_id);
    // Off: the waiting session closes and no other opens.
    desk.handle.send(CoreCommand::SetPairingAlwaysOn(false)).await.unwrap();
    wait_pairing(&mut desk, PairingState::Idle).await;
    assert!(next_pairing_within(&mut desk, Duration::from_secs(3)).await.is_none(), "nothing opens once it is off");
    desk.handle.send(CoreCommand::Shutdown).await.unwrap();
    phone.handle.send(CoreCommand::Shutdown).await.unwrap();

    // On at start, before the relay is there: nothing to open a session on until it is.
    let spare = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = spare.local_addr().unwrap().port();
    drop(spare);
    let later = Settings { relay_url: Some(format!("ws://127.0.0.1:{port}/ws")), relay_enabled: true, pairing_always_on: true, ..Settings::default() };
    let fast = |cfg: &mut CoreConfig| cfg.reconnect.max = Duration::from_millis(500);
    let mut desk = node_tuned(tempfile::tempdir().unwrap(), Arc::new(MemorySecretStore::new()), "Desk", &later, fast);
    assert!(next_pairing_within(&mut desk, Duration::from_secs(1)).await.is_none(), "no session without a relay");
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let (_addr, _task) = RelayHandle::new(RelayConfig::default())
        .serve(format!("127.0.0.1:{port}").parse().unwrap(), async move {
            let _ = stop_rx.await;
        })
        .await
        .unwrap();
    let relayed = wait_pairing(&mut desk, PairingState::WaitingForPeer).await;
    assert!(PairingTicket::from_uri(relayed.ticket_uri.as_deref().unwrap()).unwrap().relay_hint.is_some(), "opened on the relay");
    desk.handle.send(CoreCommand::Shutdown).await.unwrap();
    drop(stop_tx);
}

/// Regression: 再配一台, 重新开始 and Ctrl R send `pairing_start` straight from a finished session
/// (trusted, rejected), and the phone may join over a finished one; the core used to refuse with
/// "pairing: 已有配对正在进行，请先取消".
#[tokio::test]
async fn regression_a_new_pairing_starts_straight_from_a_finished_one() {
    let (url, _stop, _relay) = relay().await;
    let mut desk = node("Desk", Some(&url));
    let mut phone = node("Phone", Some(&url));
    wait(&mut desk, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    wait(&mut phone, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    pair_by_code(&mut desk, &mut phone).await;
    // 再配一台 on the trusted screen.
    desk.handle.send(CoreCommand::StartPairing).await.unwrap();
    let code = wait_pairing(&mut desk, PairingState::WaitingForPeer).await.code.unwrap();
    // The phone still shows its trusted screen and joins over it.
    phone.handle.send(CoreCommand::JoinWithCode(code)).await.unwrap();
    wait_pairing(&mut desk, PairingState::AwaitingVerification).await;
    wait_pairing(&mut phone, PairingState::AwaitingVerification).await;
    desk.handle.send(CoreCommand::RejectPairing).await.unwrap();
    wait_pairing(&mut desk, PairingState::Rejected).await;
    // 重新开始 on the rejected screen.
    desk.handle.send(CoreCommand::StartPairing).await.unwrap();
    wait_pairing(&mut desk, PairingState::WaitingForPeer).await;
    desk.handle.send(CoreCommand::Shutdown).await.unwrap();
    phone.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// Threat model §"Relay 读取用户数据": intercept every frame the relay carries and assert that
/// neither the plaintext message nor either static public key ever appears in it.
#[tokio::test]
async fn regression_relay_never_sees_plaintext_or_static_keys() {
    use futures_util::{SinkExt as _, StreamExt as _};
    use tokio_tungstenite::tungstenite::Message;

    // A transparent tap between the phone and the relay: records every text frame both ways.
    let (relay_url, _stop, _relay) = relay().await;
    let tap = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tap_addr = tap.local_addr().unwrap();
    let seen: Arc<parking_lot::Mutex<Vec<String>>> = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let seen2 = seen.clone();
    let upstream = relay_url.clone();
    tokio::spawn(async move {
        while let Ok((client, _)) = tap.accept().await {
            let seen = seen2.clone();
            let upstream = upstream.clone();
            tokio::spawn(async move {
                let Ok(client_ws) = tokio_tungstenite::accept_async(client).await else { return };
                let Ok((server_ws, _)) = tokio_tungstenite::connect_async(upstream).await else { return };
                let (mut c_tx, mut c_rx) = client_ws.split();
                let (mut s_tx, mut s_rx) = server_ws.split();
                let seen_a = seen.clone();
                let a = tokio::spawn(async move {
                    while let Some(Ok(m)) = c_rx.next().await {
                        if let Message::Text(t) = &m {
                            seen_a.lock().push(t.to_string());
                        }
                        if s_tx.send(m).await.is_err() {
                            break;
                        }
                    }
                });
                let b = tokio::spawn(async move {
                    while let Some(Ok(m)) = s_rx.next().await {
                        if let Message::Text(t) = &m {
                            seen.lock().push(t.to_string());
                        }
                        if c_tx.send(m).await.is_err() {
                            break;
                        }
                    }
                });
                let _ = tokio::join!(a, b);
            });
        }
    });
    let tap_url = format!("ws://{tap_addr}/ws");

    let mut desk = node("Desk", Some(&relay_url));
    let mut phone = node("Phone", Some(&tap_url));
    let desk_id = wait(&mut desk, |e| if let CoreEvent::Ready { identity, .. } = e { Some(identity.clone()) } else { None }).await;
    let phone_id = wait(&mut phone, |e| if let CoreEvent::Ready { identity, .. } = e { Some(identity.clone()) } else { None }).await;
    wait(&mut desk, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    wait(&mut phone, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    desk.handle.send(CoreCommand::StartPairing).await.unwrap();
    let code = wait_pairing(&mut desk, PairingState::WaitingForPeer).await.code.unwrap();
    phone.handle.send(CoreCommand::JoinWithCode(code)).await.unwrap();
    wait_pairing(&mut desk, PairingState::AwaitingVerification).await;
    wait_pairing(&mut phone, PairingState::AwaitingVerification).await;
    desk.handle.send(CoreCommand::ConfirmPairing).await.unwrap();
    phone.handle.send(CoreCommand::ConfirmPairing).await.unwrap();
    let td = wait(&mut desk, |e| if let CoreEvent::Trusted(d) = e { Some(d.clone()) } else { None }).await;
    wait_online(&mut desk).await;
    wait_online(&mut phone).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let secret = "the quick brown fox 语音输入 secret-7f3a";
    desk.handle.send(CoreCommand::SendText { to: td.public_key, body: secret.into() }).await.unwrap();
    let got = wait(&mut phone, |e| if let CoreEvent::Message { body, .. } = e { Some(body.clone()) } else { None }).await;
    assert_eq!(got, secret);

    let frames = seen.lock().clone();
    assert!(frames.iter().any(|f| f.contains(r#""type":"forward""#)), "tap saw no forward frames");
    assert!(frames.iter().any(|f| f.contains(r#""type":"join_by_code""#)), "tap saw no join frame");
    for f in &frames {
        assert!(!f.contains("secret-7f3a") && !f.contains("brown fox"), "plaintext leaked: {f}");
        assert!(!f.contains(&desk_id.public_key.to_hex()), "desk static key leaked: {f}");
        assert!(!f.contains(&phone_id.public_key.to_hex()), "phone static key leaked: {f}");
        assert!(!f.contains("Desk\"") && !f.contains("Phone\""), "device name leaked: {f}");
    }
    // Base64 payloads could hide the key bytes: decode them and check the raw bytes too.
    use base64::Engine as _;
    for f in frames.iter().filter(|f| f.contains(r#""type":"forward""#)) {
        let v: serde_json::Value = serde_json::from_str(f).unwrap();
        let bytes = base64::engine::general_purpose::STANDARD.decode(v["payload"].as_str().unwrap()).unwrap();
        assert!(!bytes.windows(32).any(|w| w == desk_id.public_key.as_bytes()), "desk key bytes in ciphertext");
        assert!(!bytes.windows(32).any(|w| w == phone_id.public_key.as_bytes()), "phone key bytes in ciphertext");
        assert!(!bytes.windows(11).any(|w| w == b"secret-7f3a"));
    }
}

/// A party on the channel that never answers the handshake must not leave the device stuck in
/// "connecting": after the deadline it is offline again, and a later genuine peer still works.
#[tokio::test]
async fn regression_stalled_peer_handshake_times_out() {
    use futures_util::{SinkExt as _, StreamExt as _};
    use tokio_tungstenite::tungstenite::Message;
    use voltip_protocol::ProtocolVersion;
    use voltip_protocol::relay::RelayFrame;

    let (url, _stop, _relay) = relay().await;
    let mut desk = node("Desk", Some(&url));
    let mut phone = node("Phone", Some(&url));
    let desk_id = wait(&mut desk, |e| if let CoreEvent::Ready { identity, .. } = e { Some(identity.clone()) } else { None }).await;
    wait(&mut desk, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    wait(&mut phone, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    desk.handle.send(CoreCommand::StartPairing).await.unwrap();
    let code = wait_pairing(&mut desk, PairingState::WaitingForPeer).await.code.unwrap();
    phone.handle.send(CoreCommand::JoinWithCode(code)).await.unwrap();
    wait_pairing(&mut desk, PairingState::AwaitingVerification).await;
    wait_pairing(&mut phone, PairingState::AwaitingVerification).await;
    desk.handle.send(CoreCommand::ConfirmPairing).await.unwrap();
    phone.handle.send(CoreCommand::ConfirmPairing).await.unwrap();
    let phone_record = wait(&mut desk, |e| if let CoreEvent::Trusted(d) = e { Some(d.clone()) } else { None }).await;
    wait_online(&mut desk).await;
    phone.handle.send(CoreCommand::Shutdown).await.unwrap();
    wait_offline(&mut desk).await;

    // A silent party attaches to the channel and never sends a byte.
    let channel = voltip_core::rendezvous_channel(&desk_id.public_key, &phone_record.public_key);
    let (mut ws, _) = tokio_tungstenite::connect_async(url.clone()).await.unwrap();
    ws.send(Message::Text(RelayFrame::Hello { version: ProtocolVersion::CURRENT, client_version: "silent".into() }.encode().unwrap().into())).await.unwrap();
    let _ack = ws.next().await;
    ws.send(Message::Text(RelayFrame::Attach { version: ProtocolVersion::CURRENT, channel }.encode().unwrap().into())).await.unwrap();
    // If the desk initiates we see "connecting"; either way it must settle back to offline.
    let started = std::time::Instant::now();
    loop {
        let list = wait(&mut desk, |e| if let CoreEvent::Devices(l) = e { Some(l.clone()) } else { None }).await;
        if matches!(list[0].connection, DeviceConnection::Offline) && started.elapsed() > Duration::from_millis(300) {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "never settled: {list:?}");
    }
    drop(ws);
    desk.handle.send(CoreCommand::RefreshDevices).await.unwrap();
    let list = wait(&mut desk, |e| if let CoreEvent::Devices(l) = e { Some(l.clone()) } else { None }).await;
    assert!(matches!(list[0].connection, DeviceConnection::Offline), "{list:?}");
}

/// A relay-only node whose test hooks `hooks` sets.
fn relay_node(name: &str, url: &str, hooks: impl FnOnce(&mut voltip_core::TestHooks)) -> Node {
    let settings = Settings { relay_url: Some(url.to_owned()), relay_enabled: true, ..Settings::default() };
    node_tuned(tempfile::tempdir().unwrap(), Arc::new(MemorySecretStore::new()), name, &settings, |cfg| hooks(&mut cfg.test_hooks))
}

async fn wait_not_online(node: &mut Node) {
    wait(node, |e| match e {
        CoreEvent::Devices(list) if !list.is_empty() && !list.iter().any(|d| matches!(d.connection, DeviceConnection::Online)) => Some(()),
        _ => None,
    })
    .await;
}

/// Pair `desk` and `phone`; the one that starts the peer handshakes (the smaller key) first, and
/// each side's record of the other.
async fn pair_ordered<'a>(
    desk: &'a mut Node,
    phone: &'a mut Node,
) -> ((&'a mut Node, voltip_identity::TrustedDevice), (&'a mut Node, voltip_identity::TrustedDevice)) {
    let (phone_record, desk_record) = pair_by_code(desk, phone).await;
    if desk_record.public_key.0 < phone_record.public_key.0 {
        ((desk, phone_record), (phone, desk_record))
    } else {
        ((phone, desk_record), (desk, phone_record))
    }
}

/// Until the initiator has heard the responder on its newest channel. The initiator shows the
/// device online as soon as its channel is up, but tears the channel down when it has not heard the
/// responder by the handshake deadline, and then drops what comes on it (docs/dictation.md §20.8).
/// On a slow machine (CI's coverage run, 2026-10-05) the responder's first message on the new
/// channel came after the deadline and the test's next text went with it. A probe the initiator
/// opens is heard, so the channel stays; "ready" then comes after any probe still on the way.
async fn heard_by_initiator(initiator: &mut Node, responder: &mut Node, initiator_on_responder: &voltip_identity::TrustedDevice) {
    let to = initiator_on_responder.public_key;
    let probed = async {
        loop {
            responder.handle.send(CoreCommand::SendText { to, body: "probe".into() }).await.unwrap();
            let got = wait(initiator, |e| matches!(e, CoreEvent::Message { body, .. } if body == "probe").then_some(()));
            if tokio::time::timeout(Duration::from_secs(2), got).await.is_ok() {
                return;
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(30), probed).await.expect("a probe from the responder gets through");
    responder.handle.send(CoreCommand::SendText { to, body: "ready".into() }).await.unwrap();
    wait(initiator, |e| match e {
        CoreEvent::Message { body, .. } if body == "ready" => Some(()),
        CoreEvent::Message { body, .. } if body == "probe" => None,
        CoreEvent::Message { body, .. } => panic!("unexpected text {body:?}"),
        _ => None,
    })
    .await;
}

/// Text from each side reaches the other.
async fn texts_both_ways(a: &mut Node, a_peer: &voltip_identity::TrustedDevice, b: &mut Node, b_peer: &voltip_identity::TrustedDevice) {
    async fn text(from: &mut Node, to: &mut Node, peer: &voltip_identity::TrustedDevice, body: &str) {
        from.handle.send(CoreCommand::SendText { to: peer.public_key, body: body.into() }).await.unwrap();
        let got = wait(to, |e| if let CoreEvent::Message { body, .. } = e { Some(body.clone()) } else { None }).await;
        assert_eq!(got, body);
    }
    text(a, b, a_peer, "一").await;
    text(b, a, b_peer, "二").await;
}

/// regression (2026-10-02, the Windows build host): the responder gave up waiting for the
/// handshake's last message just before it came, while the initiator already had its channel.
/// Each side then dropped all the other sent until a connection was lost (a phone's records never
/// reached the computer). The initiator, not heard from on its new channel, starts over
/// (docs/dictation.md §20.8).
#[tokio::test]
async fn regression_a_responder_that_missed_the_last_handshake_message_is_asked_again() {
    let (url, _stop, _relay) = relay().await;
    // The handshake's third message (64 bytes, which no sealed frame is) is lost once, at the
    // side that responds.
    let lose_third = |hooks: &mut voltip_core::TestHooks| {
        let lost = Arc::new(std::sync::atomic::AtomicBool::new(false));
        hooks.drop_peer_payload = Some(Arc::new(move |payload: &[u8]| payload.len() == 64 && !lost.swap(true, std::sync::atomic::Ordering::SeqCst)));
    };
    let mut desk = relay_node("Desk", &url, lose_third);
    let mut phone = relay_node("Phone", &url, lose_third);
    for n in [&mut desk, &mut phone] {
        wait(n, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    }
    let ((initiator, initiator_peer), (responder, responder_peer)) = pair_ordered(&mut desk, &mut phone).await;
    // The responder has a channel only from a second handshake on.
    tokio::time::timeout(Duration::from_secs(30), wait_online(responder)).await.expect("the responder is asked again");
    heard_by_initiator(initiator, responder, &responder_peer).await;
    texts_both_ways(initiator, &initiator_peer, responder, &responder_peer).await;
}

/// regression (2026-10-02): an initiator that heard nothing on its new channel starts over, and a
/// responder whose channel is up answers the new handshake instead of dropping it as a frame it
/// cannot open (docs/dictation.md §20.8).
#[tokio::test]
async fn regression_a_responder_with_a_channel_answers_an_initiator_that_starts_over() {
    use voltip_protocol::app::AppMessage;
    let (url, _stop, _relay) = relay().await;
    // The first device info each side sends is lost: the initiator hears nothing on its channel.
    let quiet = |hooks: &mut voltip_core::TestHooks| {
        let lost = Arc::new(std::sync::atomic::AtomicBool::new(false));
        hooks.drop_app =
            Some(Arc::new(move |msg: &AppMessage| matches!(msg, AppMessage::DeviceInfoUpdate { .. }) && !lost.swap(true, std::sync::atomic::Ordering::SeqCst)));
    };
    let mut desk = relay_node("Desk", &url, quiet);
    let mut phone = relay_node("Phone", &url, quiet);
    for n in [&mut desk, &mut phone] {
        wait(n, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    }
    let ((initiator, initiator_peer), (responder, responder_peer)) = pair_ordered(&mut desk, &mut phone).await;
    // Up, given up on, and up again: the responder answered the second handshake.
    let restarted = async {
        wait_online(initiator).await;
        wait_not_online(initiator).await;
        wait_online(initiator).await;
    };
    tokio::time::timeout(Duration::from_secs(30), restarted).await.expect("the second handshake is answered");
    heard_by_initiator(initiator, responder, &responder_peer).await;
    texts_both_ways(initiator, &initiator_peer, responder, &responder_peer).await;
}

/// Forget the other device on `desk` and wait until both sides have let go of each other.
async fn forget_both_ways(desk: &mut Node, phone: &mut Node, phone_on_desk: &voltip_identity::TrustedDevice) {
    desk.handle.send(CoreCommand::ForgetDevice(phone_on_desk.public_key)).await.unwrap();
    wait(desk, |e| matches!(e, CoreEvent::Devices(l) if l.is_empty()).then_some(())).await;
    wait(phone, |e| matches!(e, CoreEvent::Unpaired(_)).then_some(())).await;
}

/// regression (2026-10-02, a user report): forgetting a device left this device on their
/// rendezvous channel, because the relay keeps a connection on a channel until the connection
/// drops. Paired again at once, each side asked to attach to the channel it was still on, the
/// relay refused (`SessionAlreadyActive`), and neither came online until its relay connection was
/// replaced. The channel a forgotten device leaves behind is taken up again (docs/pairing.md).
#[tokio::test]
async fn regression_a_device_forgotten_and_paired_again_at_once_comes_online_over_the_relay() {
    let (url, _stop, _relay) = relay().await;
    let mut desk = relay_node("Desk", &url, |_| {});
    let mut phone = relay_node("Phone", &url, |_| {});
    for n in [&mut desk, &mut phone] {
        wait(n, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    }
    let (phone_on_desk, _) = pair_by_code(&mut desk, &mut phone).await;
    wait_online(&mut desk).await;
    wait_online(&mut phone).await;
    forget_both_ways(&mut desk, &mut phone, &phone_on_desk).await;
    // At once, on the relay connections both already have.
    let (phone_on_desk, desk_on_phone) = pair_by_code(&mut desk, &mut phone).await;
    let online = async {
        wait_online(&mut desk).await;
        wait_online(&mut phone).await;
    };
    tokio::time::timeout(Duration::from_secs(30), online).await.expect("both come online without a new relay connection");
    texts_both_ways(&mut desk, &phone_on_desk, &mut phone, &desk_on_phone).await;
}

/// One-sided: the device that answers handshakes forgets the other while that one is offline, so
/// the other, never told, still trusts it, attaches to their channel when it is back and opens a
/// handshake there that nobody answers. Paired again, the forgetting side takes up the channel and
/// answers that handshake; the other, already on the channel, attaches no second time.
#[tokio::test]
async fn regression_a_device_forgotten_while_offline_and_paired_again_comes_online() {
    let (url, _stop, _relay) = relay().await;
    // A handshake nobody answers stays open long after this test would give up: only an answer to
    // the one already sent brings the two online.
    let settings = Settings { relay_url: Some(url.clone()), relay_enabled: true, ..Settings::default() };
    let patient = |cfg: &mut CoreConfig| cfg.peer_handshake_timeout = Duration::from_secs(120);
    let mut desk = node_tuned(tempfile::tempdir().unwrap(), Arc::new(MemorySecretStore::new()), "Desk", &settings, patient);
    let mut phone = node_tuned(tempfile::tempdir().unwrap(), Arc::new(MemorySecretStore::new()), "Phone", &settings, patient);
    for n in [&mut desk, &mut phone] {
        wait(n, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    }
    let ((initiator, _), (responder, initiator_on_responder)) = pair_ordered(&mut desk, &mut phone).await;
    wait_online(initiator).await;
    wait_online(responder).await;
    initiator.handle.send(CoreCommand::SetRelay { url: Some(url.clone()), enabled: false }).await.unwrap();
    wait_offline(responder).await;
    responder.handle.send(CoreCommand::ForgetDevice(initiator_on_responder.public_key)).await.unwrap();
    wait(responder, |e| matches!(e, CoreEvent::Devices(l) if l.is_empty()).then_some(())).await;
    initiator.handle.send(CoreCommand::SetRelay { url: Some(url.clone()), enabled: true }).await.unwrap();
    wait(initiator, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    let (initiator_on_responder, responder_on_initiator) = pair_by_code(responder, initiator).await;
    let online = async {
        wait_online(responder).await;
        wait_online(initiator).await;
    };
    tokio::time::timeout(Duration::from_secs(30), online).await.expect("both come online on the channel both kept");
    texts_both_ways(initiator, &responder_on_initiator, responder, &initiator_on_responder).await;
}

/// A desktop whose recogniser is ready: the custom endpoint (the fake factory answers for it).
fn desktop_ready(name: &str, relay_url: &str) -> Node {
    let mut settings = Settings { relay_url: Some(relay_url.to_owned()), relay_enabled: true, ..Settings::default() };
    settings.engines.asr_provider = voltip_core::ProviderId::Custom;
    settings.engines.providers.insert(
        voltip_core::ProviderId::Custom,
        voltip_core::ProviderSettings { asr_url: Some("http://127.0.0.1:9/v1".into()), asr_model: Some("fake".into()), ..Default::default() },
    );
    node_tuned(tempfile::tempdir().unwrap(), Arc::new(MemorySecretStore::new()), name, &settings, |_| {})
}

fn phone_take(e: &CoreEvent) -> Option<voltip_core::phone::PhoneTakeState> {
    match e {
        CoreEvent::PhoneTake(Some(view)) => Some(view.state.clone()),
        _ => None,
    }
}

/// The state of phone take `take` (a late status of an earlier take does not count).
fn phone_take_of(take: u32) -> impl FnMut(&CoreEvent) -> Option<voltip_core::phone::PhoneTakeState> {
    move |e| match e {
        CoreEvent::PhoneTake(Some(view)) if view.take == take => Some(view.state.clone()),
        _ => None,
    }
}

/// docs/dictation.md §20: the phone is the desktop's microphone. The phone streams a take through
/// the relay; the desktop records it (its status names the phone), recognises and delivers it, and
/// the phone hears back every state down to the delivered text. A cancel on the phone discards the
/// take on both ends, and a second take while the desktop is busy is refused as busy.
#[tokio::test]
async fn a_phone_streams_a_take_the_desktop_delivers() {
    use voltip_core::dictation::DictationPhase;
    use voltip_core::dictation::fakes::FAKE_TRANSCRIPT;
    use voltip_core::phone::{PhoneTakeFailure, PhoneTakeState};
    let (url, _stop, _relay) = relay().await;
    let mut desk = desktop_ready("Studio", &url);
    let mut phone = node_with(tempfile::tempdir().unwrap(), Arc::new(MemorySecretStore::new()), "Pixel 8", Some(&url));
    wait(&mut desk, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    wait(&mut phone, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    let (_, desktop) = pair_by_code(&mut desk, &mut phone).await;
    wait_online(&mut desk).await;
    wait_online(&mut phone).await;

    // The phone's own level stream follows its take: the phone shows its meter from it.
    let mut phone_levels = phone.handle.levels();
    phone.handle.send(CoreCommand::PhoneTakeStart { to: desktop.public_key }).await.unwrap();
    assert_eq!(wait(&mut phone, phone_take).await, PhoneTakeState::Starting);
    let frame = tokio::time::timeout(Duration::from_secs(5), phone_levels.recv()).await.expect("no level frame from the phone's take").unwrap();
    assert!(frame.rms_dbfs < 0.0 && frame.sample_rate_hz > 0, "{frame:?}");
    let remote = wait(&mut desk, |e| match e {
        CoreEvent::Dictation(s) if matches!(s.phase, DictationPhase::Listening { .. }) => Some(s.remote.clone()),
        _ => None,
    })
    .await;
    assert_eq!(remote.as_deref(), Some("Pixel 8"), "the desktop's take names the phone");
    // docs/dictation.md §20.1: the desktop's first status says it decodes Opus, so the phone
    // switches from PCM for the rest of the take (and its view says so).
    wait(&mut phone, |e| match e {
        CoreEvent::PhoneTake(Some(view)) if view.state == PhoneTakeState::Listening && view.opus => Some(()),
        _ => None,
    })
    .await;
    // The desktop's take is ready once the phone's audio arrived (the ready mark rides on it).
    wait(&mut desk, |e| match e {
        CoreEvent::Dictation(s) if matches!(s.phase, DictationPhase::Listening { ready: true, .. }) => Some(()),
        _ => None,
    })
    .await;
    phone.handle.send(CoreCommand::PhoneTakeStop).await.unwrap();
    let (text, duration_ms) = wait(&mut desk, |e| match e {
        CoreEvent::Dictation(s) => match &s.phase {
            DictationPhase::Done { text, duration_ms, .. } => Some((text.clone(), *duration_ms)),
            _ => None,
        },
        _ => None,
    })
    .await;
    assert_eq!(text, FAKE_TRANSCRIPT);
    // PCM first, Opus after: the decoded packets join the same feed, nothing is lost or doubled.
    assert!(duration_ms >= 1000, "the streamed 1.5 s of audio is the take: {duration_ms} ms");
    let done = wait(&mut phone, |e| phone_take(e).filter(PhoneTakeState::is_final)).await;
    assert_eq!(done, PhoneTakeState::Done { text: FAKE_TRANSCRIPT.into(), pasted: true });

    // Cancel from the phone: both ends drop the take.
    wait(&mut desk, |e| matches!(e, CoreEvent::Dictation(s) if s.phase == DictationPhase::Idle).then_some(())).await;
    phone.handle.send(CoreCommand::PhoneTakeStart { to: desktop.public_key }).await.unwrap();
    wait(&mut phone, |e| (phone_take(e) == Some(PhoneTakeState::Listening)).then_some(())).await;
    phone.handle.send(CoreCommand::PhoneTakeCancel).await.unwrap();
    assert_eq!(wait(&mut phone, |e| phone_take(e).filter(PhoneTakeState::is_final)).await, PhoneTakeState::Cancelled);
    wait(&mut desk, |e| matches!(e, CoreEvent::Dictation(s) if matches!(s.phase, DictationPhase::Cancelled { .. })).then_some(())).await;

    // The desktop busy with its own take: the phone is told so.
    wait(&mut desk, |e| matches!(e, CoreEvent::Dictation(s) if s.phase == DictationPhase::Idle).then_some(())).await;
    desk.handle.send(CoreCommand::DictationStart).await.unwrap();
    wait(&mut desk, |e| matches!(e, CoreEvent::Dictation(s) if matches!(s.phase, DictationPhase::Listening { .. })).then_some(())).await;
    phone.handle.send(CoreCommand::PhoneTakeStart { to: desktop.public_key }).await.unwrap();
    let take = wait(&mut phone, |e| match e {
        CoreEvent::PhoneTake(Some(view)) if view.state == PhoneTakeState::Starting => Some(view.take),
        _ => None,
    })
    .await;
    let mut this = phone_take_of(take);
    let busy = wait(&mut phone, |e| this(e).filter(PhoneTakeState::is_final)).await;
    assert!(matches!(busy, PhoneTakeState::Failed { code: PhoneTakeFailure::Busy, .. }), "{busy:?}");

    phone.handle.send(CoreCommand::Shutdown).await.unwrap();
    desk.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// Regression (user report 2026-10-03, and the user's decision the same day): a take the phone
/// streamed to a computer was in the computer's history only, so the phone's 记录 counted no
/// dictations however many it had sent. The phone now keeps its own record of a take a computer
/// delivered: the text the computer reported, the length of the audio sent and the computer it
/// went to. It counts in the phone's statistics; a take that did not deliver leaves none.
#[tokio::test]
async fn regression_a_take_the_phone_streamed_is_in_the_phones_own_history_and_counts() {
    use voltip_core::dictation::DictationPhase;
    use voltip_core::dictation::fakes::FAKE_TRANSCRIPT;
    use voltip_core::history::{HistoryReader, OriginKind};
    use voltip_core::phone::PhoneTakeState;
    let (url, _stop, _relay) = relay().await;
    let mut desk = desktop_ready("Studio", &url);
    let mut phone = node_with(tempfile::tempdir().unwrap(), Arc::new(MemorySecretStore::new()), "Pixel 8", Some(&url));
    wait(&mut desk, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    wait(&mut phone, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    let (_, desktop) = pair_by_code(&mut desk, &mut phone).await;
    wait_online(&mut desk).await;
    wait_online(&mut phone).await;

    phone.handle.send(CoreCommand::PhoneTakeStart { to: desktop.public_key }).await.unwrap();
    wait(&mut desk, |e| match e {
        CoreEvent::Dictation(s) if matches!(s.phase, DictationPhase::Listening { ready: true, .. }) => Some(()),
        _ => None,
    })
    .await;
    phone.handle.send(CoreCommand::PhoneTakeStop).await.unwrap();
    let done = wait(&mut phone, |e| phone_take(e).filter(PhoneTakeState::is_final)).await;
    assert_eq!(done, PhoneTakeState::Done { text: FAKE_TRANSCRIPT.into(), pasted: true });
    // The record is written after the take's end is shown, and announced like any other.
    let recent = wait(&mut phone, |e| match e {
        CoreEvent::History { recent, total: 1 } => Some(recent.clone()),
        _ => None,
    })
    .await;
    let entry = &recent[0];
    assert_eq!((entry.text.as_str(), entry.raw_text.as_str()), (FAKE_TRANSCRIPT, FAKE_TRANSCRIPT));
    assert_eq!(entry.origin.as_ref().map(|o| (o.device.as_str(), o.kind)), Some(("Studio", OriginKind::Sent)));
    assert!(entry.duration_ms >= 1000, "the length of the audio the phone sent: {} ms", entry.duration_ms);
    let reader = HistoryReader::new(&phone.dir_path);
    assert_eq!(reader.stats(&[0, 4_102_444_800_000]).unwrap().total.count, 1, "the phone's statistics count it");

    // A take the phone cancelled delivered nothing: no record.
    wait(&mut desk, |e| matches!(e, CoreEvent::Dictation(s) if s.phase == DictationPhase::Idle).then_some(())).await;
    phone.handle.send(CoreCommand::PhoneTakeStart { to: desktop.public_key }).await.unwrap();
    wait(&mut phone, |e| (phone_take(e) == Some(PhoneTakeState::Listening)).then_some(())).await;
    phone.handle.send(CoreCommand::PhoneTakeCancel).await.unwrap();
    assert_eq!(wait(&mut phone, |e| phone_take(e).filter(PhoneTakeState::is_final)).await, PhoneTakeState::Cancelled);
    assert_eq!(reader.stats(&[0, 4_102_444_800_000]).unwrap().total.count, 1);

    phone.handle.send(CoreCommand::Shutdown).await.unwrap();
    desk.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// The phone's list of sent texts, from the latest `SentTexts` event.
fn sent_texts(e: &CoreEvent) -> Option<Vec<voltip_core::phone::SentText>> {
    match e {
        CoreEvent::SentTexts(texts) => Some(texts.clone()),
        _ => None,
    }
}

/// docs/dictation.md §20.6: the phone sends text for the desktop to insert. It goes in at once
/// when the desktop is idle, waits (`queued`) while the desktop records, lands in the desktop's
/// history as the phone's, and the phone's list follows every answer. A phone take's history entry
/// is the phone's too.
#[tokio::test]
async fn a_phone_sends_text_the_desktop_inserts_now_or_after_its_take() {
    use voltip_core::dictation::DictationPhase;
    use voltip_core::phone::{PhoneTextSource, SentTextState};
    use voltip_core::{OriginKind, Outcome};
    let (url, _stop, _relay) = relay().await;
    let mut desk = desktop_ready("Studio", &url);
    let mut phone = node_with(tempfile::tempdir().unwrap(), Arc::new(MemorySecretStore::new()), "Pixel 8", Some(&url));
    wait(&mut desk, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    wait(&mut phone, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    let (_, desktop) = pair_by_code(&mut desk, &mut phone).await;
    wait_online(&mut desk).await;
    wait_online(&mut phone).await;

    // Idle desktop: inserted at once, pasted, recorded as the phone's typed text.
    phone.handle.send(CoreCommand::PhoneTextSend { to: desktop.public_key, body: "会议改到三点".into(), source: PhoneTextSource::Typed }).await.unwrap();
    let delivered = wait(&mut phone, |e| sent_texts(e).filter(|t| t.first().is_some_and(|t| t.state.is_final()))).await;
    assert_eq!(delivered[0].state, SentTextState::Delivered { pasted: true });
    assert_eq!((delivered[0].body.as_str(), delivered[0].device_name.as_str()), ("会议改到三点", "Studio"));
    let history = wait(&mut desk, |e| match e {
        CoreEvent::History { recent: entries, .. } if entries.iter().any(|h| h.text == "会议改到三点") => Some(entries.clone()),
        _ => None,
    })
    .await;
    let entry = history.iter().find(|h| h.text == "会议改到三点").unwrap();
    let origin = entry.origin.clone().unwrap();
    assert_eq!((origin.device.as_str(), origin.kind), ("Pixel 8", OriginKind::Typed));
    assert!(matches!(entry.outcome, Outcome::Inserted { .. }), "{:?}", entry.outcome);

    // The desktop records: the clipboard text waits, then goes in once the take is over.
    desk.handle.send(CoreCommand::DictationStart).await.unwrap();
    wait(&mut desk, |e| matches!(e, CoreEvent::Dictation(s) if matches!(s.phase, DictationPhase::Listening { .. })).then_some(())).await;
    phone.handle.send(CoreCommand::PhoneTextSend { to: desktop.public_key, body: "剪贴板里的地址".into(), source: PhoneTextSource::Clipboard }).await.unwrap();
    wait(&mut phone, |e| sent_texts(e).filter(|t| t.first().is_some_and(|t| t.state == SentTextState::Queued))).await;
    desk.handle.send(CoreCommand::DictationCancel).await.unwrap();
    let after = wait(&mut phone, |e| sent_texts(e).filter(|t| t.first().is_some_and(|t| t.state.is_final()))).await;
    assert_eq!(after[0].state, SentTextState::Delivered { pasted: true });
    assert_eq!(after.len(), 2, "newest first, both kept");
    wait(&mut desk, |e| match e {
        CoreEvent::History { recent: entries, .. } => {
            entries.iter().find(|h| h.text == "剪贴板里的地址").and_then(|h| h.origin.clone()).filter(|o| o.kind == OriginKind::Clipboard)
        }
        _ => None,
    })
    .await;

    // Refused before anything is sent: nothing, or more than the limit.
    for body in ["   ".to_owned(), "字".repeat(voltip_core::phone::MAX_PHONE_TEXT_CHARS + 1)] {
        phone.handle.send(CoreCommand::PhoneTextSend { to: desktop.public_key, body, source: PhoneTextSource::Typed }).await.unwrap();
        let message = wait(&mut phone, |e| if let CoreEvent::Error(m) = e { Some(m.clone()) } else { None }).await;
        assert!(message.contains("phone text"), "{message}");
    }

    // A phone take lands in the history as the phone's.
    wait(&mut desk, |e| matches!(e, CoreEvent::Dictation(s) if s.phase == DictationPhase::Idle).then_some(())).await;
    phone.handle.send(CoreCommand::PhoneTakeStart { to: desktop.public_key }).await.unwrap();
    wait(&mut phone, |e| (phone_take(e) == Some(voltip_core::phone::PhoneTakeState::Listening)).then_some(())).await;
    wait(&mut desk, |e| matches!(e, CoreEvent::Dictation(s) if matches!(s.phase, DictationPhase::Listening { ready: true, .. })).then_some(())).await;
    phone.handle.send(CoreCommand::PhoneTakeStop).await.unwrap();
    wait(&mut desk, |e| match e {
        CoreEvent::History { recent: entries, .. } => {
            entries.first().and_then(|h| h.origin.clone()).filter(|o| o.kind == OriginKind::Take && o.device == "Pixel 8")
        }
        _ => None,
    })
    .await;

    // The list is the phone's, and it can forget it.
    phone.handle.send(CoreCommand::SentTextsClear).await.unwrap();
    wait(&mut phone, |e| sent_texts(e).filter(Vec::is_empty)).await;
    // Regression: the next text after a clear is still inserted. Its id used to restart at 1,
    // which the desktop had already seen, so it was dropped as a duplicate without an answer.
    phone.handle.send(CoreCommand::PhoneTextSend { to: desktop.public_key, body: "清空后再发一条".into(), source: PhoneTextSource::Typed }).await.unwrap();
    let again = wait(&mut phone, |e| sent_texts(e).filter(|t| t.first().is_some_and(|t| t.state.is_final()))).await;
    assert_eq!(again[0].state, SentTextState::Delivered { pasted: true });
    wait(&mut desk, |e| match e {
        CoreEvent::History { recent: entries, .. } => entries.iter().any(|h| h.text == "清空后再发一条").then_some(()),
        _ => None,
    })
    .await;

    phone.handle.send(CoreCommand::Shutdown).await.unwrap();
    desk.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// Opt-in check against a **deployed** relay: `VOLTIP_LIVE_RELAY_URL=wss://host/ws cargo test -p
/// voltip-core --test e2e live_relay`. Two cores pair by code through that relay (TLS handshake,
/// WebSocket upgrade through the reverse proxy, rendezvous, E2EE message both ways). Skipped when
/// the variable is unset so the offline suite stays hermetic.
#[tokio::test]
async fn live_relay_round_trip_when_configured() {
    let Some(url) = std::env::var("VOLTIP_LIVE_RELAY_URL").ok().filter(|u| !u.trim().is_empty()) else {
        eprintln!("live_relay_round_trip_when_configured: VOLTIP_LIVE_RELAY_URL unset, skipped");
        return;
    };
    let mut desk = node_with(tempfile::tempdir().unwrap(), Arc::new(MemorySecretStore::new()), "Live-Desk", Some(&url));
    let mut phone = node_with(tempfile::tempdir().unwrap(), Arc::new(MemorySecretStore::new()), "Live-Phone", Some(&url));
    wait(&mut desk, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    wait(&mut phone, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    let (td, tp) = pair_by_code(&mut desk, &mut phone).await;
    assert_eq!(td.name, "Live-Phone");
    assert_eq!(tp.name, "Live-Desk");
    wait_online(&mut desk).await;
    wait_online(&mut phone).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    desk.handle.send(CoreCommand::SendText { to: td.public_key, body: "live relay ping".into() }).await.unwrap();
    let body = wait(&mut phone, |e| if let CoreEvent::Message { body, .. } = e { Some(body.clone()) } else { None }).await;
    assert_eq!(body, "live relay ping");
    phone.handle.send(CoreCommand::SendText { to: tp.public_key, body: "pong".into() }).await.unwrap();
    assert_eq!(wait(&mut desk, |e| if let CoreEvent::Message { body, .. } = e { Some(body.clone()) } else { None }).await, "pong");
    desk.handle.send(CoreCommand::Shutdown).await.unwrap();
    phone.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// The next finished connectivity report.
async fn connectivity_report(node: &mut Node) -> voltip_core::connectivity::ConnectivityReport {
    use voltip_core::connectivity::ConnectivityStatus;
    wait(node, |e| match e {
        CoreEvent::Connectivity(ConnectivityStatus { running: false, report: Some(r) }) => Some(r.clone()),
        _ => None,
    })
    .await
}

/// The connectivity self-check (docs/pairing.md): the phone probes the relay (it answers `hello`),
/// pings the computer over their channel, and reports both; a second check while one runs is
/// refused. Once the computer is gone, it is offline with no round trip.
#[tokio::test]
async fn the_connectivity_check_reports_the_relay_and_each_device() {
    use voltip_core::connectivity::{ConnectivityStatus, ProbeResult};
    let (url, _stop, _relay) = relay().await;
    let mut desk = node("Desk", Some(&url));
    let mut phone = node("Phone", Some(&url));
    wait(&mut desk, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    wait(&mut phone, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    let (_, desktop) = pair_by_code(&mut desk, &mut phone).await;
    wait_online(&mut phone).await;

    phone.handle.send(CoreCommand::CheckConnectivity).await.unwrap();
    phone.handle.send(CoreCommand::CheckConnectivity).await.unwrap();
    wait(&mut phone, |e| matches!(e, CoreEvent::Connectivity(ConnectivityStatus { running: true, .. })).then_some(())).await;
    wait(&mut phone, |e| matches!(e, CoreEvent::Error(m) if m.contains("自检正在进行")).then_some(())).await;
    let first = connectivity_report(&mut phone).await;
    assert!(first.relay.configured && matches!(first.relay.result, Some(ProbeResult::Ok { .. })), "{:?}", first.relay);
    assert_eq!(first.peers.len(), 1);
    let peer = &first.peers[0];
    assert_eq!((peer.public_key.as_str(), peer.name.as_str()), (desktop.public_key.to_hex().as_str(), "Desk"));
    assert!(peer.online && peer.rtt_ms.is_some(), "{peer:?}");

    // The computer shuts down: there is no channel any more.
    desk.handle.send(CoreCommand::Shutdown).await.unwrap();
    wait_offline(&mut phone).await;
    phone.handle.send(CoreCommand::CheckConnectivity).await.unwrap();
    let second = connectivity_report(&mut phone).await;
    let peer = &second.peers[0];
    assert!(!peer.online && peer.rtt_ms.is_none(), "{peer:?}");
    assert!(matches!(second.relay.result, Some(ProbeResult::Ok { .. })), "{:?}", second.relay);
    phone.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// A relay that notices silent connections quickly, a proxy whose path can break in front of it
/// for the phone, and the phone's settings: a short heartbeat, fast reconnects, and quick attach
/// retries (docs/pairing.md 「重连」).
async fn breakable(phone_tune: impl FnOnce(&mut CoreConfig)) -> (Node, Node, NetCut, tokio::sync::oneshot::Sender<()>) {
    let config = RelayConfig { ping_interval: Duration::from_millis(200), idle_timeout: Duration::from_millis(1_500), ..RelayConfig::default() };
    let (url, addr, stop, _relay) = relay_with(config).await;
    let cut = NetCut::start(addr).await;
    let desk = node("Desk", Some(&url));
    let settings = Settings { relay_url: Some(cut.url()), relay_enabled: true, ..Settings::default() };
    let phone = node_tuned(tempfile::tempdir().unwrap(), Arc::new(MemorySecretStore::new()), "Phone", &settings, |cfg| {
        cfg.reconnect = voltip_transport::ReconnectPolicy { base: Duration::from_millis(50), max: Duration::from_millis(300), jitter: 0.0, max_attempts: None };
        cfg.attach_retry = Duration::from_millis(200);
        phone_tune(cfg);
    });
    (desk, phone, cut, stop)
}

/// Wait up to 30 s for both nodes to show their device online, then until texts get through.
async fn both_back(desk: &mut Node, phone: &mut Node, phone_on_desk: &voltip_identity::TrustedDevice, desk_on_phone: &voltip_identity::TrustedDevice) {
    let online = async {
        wait_online(desk).await;
        wait_online(phone).await;
    };
    tokio::time::timeout(Duration::from_secs(30), online).await.expect("both are back on their own");
    let (initiator, responder, responder_peer) =
        if phone_on_desk.public_key.0 < desk_on_phone.public_key.0 { (phone, desk, phone_on_desk) } else { (desk, phone, desk_on_phone) };
    heard_by_initiator(initiator, responder, responder_peer).await;
}

/// Regression (2026-10-10, 「配对后断开」): the phone's path to the relay breaks without a word, as
/// when it changes networks. Its heartbeat notices and it connects again, but the relay still holds
/// its old connection on their channel and refuses the attach (`channel_full`); the refusal used
/// to be dropped, and the two devices stayed apart until an app restarted. Now the relay closes the
/// silent connection and the phone asks again until it is let in (docs/pairing.md 「重连」).
#[tokio::test]
async fn regression_a_path_that_breaks_without_a_word_heals_on_its_own() {
    let (mut desk, mut phone, cut, _stop) = breakable(|cfg| {
        cfg.relay_ping_interval = Duration::from_millis(300);
        cfg.relay_pong_timeout = Duration::from_millis(600);
    })
    .await;
    for n in [&mut desk, &mut phone] {
        wait(n, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    }
    let (phone_on_desk, desk_on_phone) = pair_by_code(&mut desk, &mut phone).await;
    both_back(&mut desk, &mut phone, &phone_on_desk, &desk_on_phone).await;
    cut.cut();
    // The phone's heartbeat finds the socket dead and it connects again.
    wait(&mut phone, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Reconnecting).then_some(())).await;
    // The computer hears the phone leave once the relay gave up on the silent connection…
    tokio::time::timeout(Duration::from_secs(30), wait_not_online(&mut desk)).await.expect("the relay let the silent connection go");
    // …and the phone, refused while it was still there, is let in.
    both_back(&mut desk, &mut phone, &phone_on_desk, &desk_on_phone).await;
    texts_both_ways(&mut desk, &phone_on_desk, &mut phone, &desk_on_phone).await;
}

/// docs/pairing.md 「重连」: told that the network changed (`ReconnectRelay`, what the phone app sends
/// when Android reports a new network or the app comes to the front), the phone checks its relay
/// socket at once and connects again, long before its heartbeat would have noticed anything.
#[tokio::test]
async fn reconnect_relay_checks_the_socket_at_once_after_the_network_changed() {
    let (mut desk, mut phone, cut, _stop) = breakable(|cfg| {
        cfg.relay_ping_interval = Duration::from_secs(600);
        cfg.relay_pong_timeout = Duration::from_secs(600);
        cfg.relay_probe_timeout = Duration::from_millis(300);
    })
    .await;
    for n in [&mut desk, &mut phone] {
        wait(n, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    }
    let (phone_on_desk, desk_on_phone) = pair_by_code(&mut desk, &mut phone).await;
    both_back(&mut desk, &mut phone, &phone_on_desk, &desk_on_phone).await;
    cut.cut();
    phone.handle.send(CoreCommand::ReconnectRelay).await.unwrap();
    let noticed = wait(&mut phone, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Reconnecting).then_some(()));
    tokio::time::timeout(Duration::from_secs(10), noticed).await.expect("the probe found the socket dead");
    both_back(&mut desk, &mut phone, &phone_on_desk, &desk_on_phone).await;
    texts_both_ways(&mut desk, &phone_on_desk, &mut phone, &desk_on_phone).await;
}
