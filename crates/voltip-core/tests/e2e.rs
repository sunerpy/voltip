#![allow(clippy::unwrap_used, clippy::expect_used)]
//! End-to-end: two cores, a real relay, real sockets. Pairing by code and by ticket, E2EE
//! messaging, presence, reconnect, forget, and the identity-changed regression.

use std::sync::Arc;
use std::time::Duration;

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
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let handle = RelayHandle::new(RelayConfig::default());
    let (addr, _task) = handle
        .clone()
        .serve("127.0.0.1:0".parse().unwrap(), async move {
            let _ = stop_rx.await;
        })
        .await
        .unwrap();
    (format!("ws://{addr}/ws"), stop_tx, handle)
}

fn node_with(dir: tempfile::TempDir, store: Arc<MemorySecretStore>, name: &str, relay_url: Option<&str>) -> Node {
    node_opts(dir, store, name, relay_url, true)
}

/// `direct_enabled = false` simulates a device whose LAN is unreachable: no LAN host, no dialing.
fn node_opts(dir: tempfile::TempDir, store: Arc<MemorySecretStore>, name: &str, relay_url: Option<&str>, direct_enabled: bool) -> Node {
    let settings = Settings { relay_url: relay_url.map(str::to_owned), relay_enabled: relay_url.is_some(), ..Settings::default() };
    node_settings(dir, store, name, &settings, direct_enabled)
}

fn node_settings(dir: tempfile::TempDir, store: Arc<MemorySecretStore>, name: &str, settings: &Settings, direct_enabled: bool) -> Node {
    node_config(dir, store, name, settings, direct_enabled, None)
}

/// A node on the in-memory LAN `lan` (docs/pairing.md 「局域网发现」): LAN host on, no relay.
fn node_on_lan(dir: tempfile::TempDir, store: Arc<MemorySecretStore>, name: &str, lan: &voltip_core::discovery::fake::LocalLan) -> Node {
    let settings = Settings { relay_url: None, relay_enabled: false, ..Settings::default() };
    node_config(dir, store, name, &settings, true, Some(lan.join()))
}

fn node_config(
    dir: tempfile::TempDir,
    store: Arc<MemorySecretStore>,
    name: &str,
    settings: &Settings,
    direct_enabled: bool,
    discovery: Option<Arc<dyn voltip_core::discovery::Discovery>>,
) -> Node {
    trace_init();
    let dir_path = dir.path().to_path_buf();
    SettingsStore::new(&dir_path).save(settings).unwrap();
    let mut cfg = CoreConfig::new(dir_path.clone());
    cfg.default_device_name = name.into();
    cfg.tick = Duration::from_millis(50);
    cfg.direct_bind = "127.0.0.1:0".parse().unwrap();
    cfg.direct_enabled = direct_enabled;
    cfg.direct_retry = Duration::from_millis(100);
    cfg.direct_retry_max = Duration::from_millis(400);
    cfg.direct_connect_timeout = Duration::from_secs(2);
    cfg.peer_handshake_timeout = Duration::from_millis(600);
    cfg.discovery = discovery;
    let (handle, events) = AppCore::start(cfg, store.clone()).unwrap();
    Node { handle, events, _dir: dir, store, dir_path }
}

async fn wait_online_via(node: &mut Node, via: voltip_identity::ConnectionKind) {
    wait(node, |e| match e {
        CoreEvent::Devices(list) if list.iter().any(|d| d.connection == DeviceConnection::Online { via }) => Some(()),
        _ => None,
    })
    .await;
}

/// Wait until the device list shows a persisted LAN endpoint for the (single) trusted peer.
async fn wait_hints_learned(node: &mut Node) {
    node.handle.send(CoreCommand::RefreshDevices).await.unwrap();
    wait(node, |e| match e {
        CoreEvent::Devices(list) if list.iter().any(|d| !d.device.direct_hints.is_empty()) => Some(()),
        _ => None,
    })
    .await;
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
        CoreEvent::Devices(list) if list.iter().any(|d| matches!(d.connection, DeviceConnection::Online { .. })) => Some(()),
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

/// Relay-only scenario (devices on different networks): presence, reconnect and forget all
/// happen through the relay, so both nodes run with the LAN side disabled.
#[tokio::test]
async fn pair_by_code_message_presence_reconnect_forget() {
    let (url, _stop, relay) = relay().await;
    let mut desk = node_opts(tempfile::tempdir().unwrap(), Arc::new(MemorySecretStore::new()), "Surface-Laptop", Some(&url), false);
    let mut phone = node_opts(tempfile::tempdir().unwrap(), Arc::new(MemorySecretStore::new()), "Pixel 10", Some(&url), false);
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
    wait(&mut desk, |e| matches!(e, CoreEvent::Error(m) if m.contains("already")).then_some(())).await;

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
    wait(&mut desk, |e| matches!(e, CoreEvent::Error(m) if m.contains("not online")).then_some(())).await;
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
    wait(&mut phone, |e| matches!(e, CoreEvent::Error(m) if m.contains("no pairing")).then_some(())).await;
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
    wait(&mut desk, |e| matches!(e, CoreEvent::Error(m) if m.contains("not online")).then_some(())).await;
}

#[tokio::test]
async fn pair_over_lan_without_any_relay() {
    let mut desk = node("Desk", None);
    let mut phone = node("Phone", None);
    let r = wait(&mut desk, |e| if let CoreEvent::Relay(r) = e { Some(r.clone()) } else { None }).await;
    assert_eq!(r.state, ConnectionState::Disconnected);
    assert!(r.endpoint.is_none());
    // A code cannot be used without a relay.
    phone.handle.send(CoreCommand::JoinWithCode("483921".into())).await.unwrap();
    wait(&mut phone, |e| matches!(e, CoreEvent::Error(m) if m.contains("relay")).then_some(())).await;
    desk.handle.send(CoreCommand::StartPairing).await.unwrap();
    let waiting = wait_pairing(&mut desk, PairingState::WaitingForPeer).await;
    let ticket = voltip_protocol::ticket::PairingTicket::from_uri(waiting.ticket_uri.as_deref().unwrap()).unwrap();
    assert!(ticket.relay_hint.is_none());
    assert_eq!(ticket.direct_hints.len(), 1);
    phone.handle.send(CoreCommand::JoinWithTicket(waiting.ticket_uri.clone().unwrap())).await.unwrap();
    let vd = wait_pairing(&mut desk, PairingState::AwaitingVerification).await;
    let vp = wait_pairing(&mut phone, PairingState::AwaitingVerification).await;
    assert_eq!(vd.safety_code, vp.safety_code);
    desk.handle.send(CoreCommand::ConfirmPairing).await.unwrap();
    phone.handle.send(CoreCommand::ConfirmPairing).await.unwrap();
    wait_pairing(&mut desk, PairingState::Trusted).await;
    wait_pairing(&mut phone, PairingState::Trusted).await;
    // The live channel is re-established on the desktop's LAN host; nothing else is involved.
    wait_online_via(&mut desk, voltip_identity::ConnectionKind::Direct).await;
    wait_online_via(&mut phone, voltip_identity::ConnectionKind::Direct).await;
    // Each side announces its LAN endpoint inside the channel; the desktop learns the phone's.
    wait_hints_learned(&mut desk).await;
    desk.handle.send(CoreCommand::RefreshDevices).await.unwrap();
    let list = wait(&mut desk, |e| if let CoreEvent::Devices(l) = e { Some(l.clone()) } else { None }).await;
    assert!(matches!(list[0].connection, DeviceConnection::Online { via: voltip_identity::ConnectionKind::Direct }), "{list:?}");
    // Trusted devices survive a restart of the core (same data dir + same secret store).
    desk.handle.send(CoreCommand::Shutdown).await.unwrap();
    let dir = desk._dir;
    let store = desk.store.clone();
    let mut desk2 = node_with(dir, store, "ignored", None);
    let list = wait(&mut desk2, |e| if let CoreEvent::Devices(l) = e { Some(l.clone()) } else { None }).await;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].device.name, "Phone");
    let file = std::fs::read_to_string(desk2.dir_path.join("trusted-devices.json")).unwrap();
    assert!(file.contains("Phone"));
    // The phone's LAN endpoint was learned over the encrypted channel and persisted, so the
    // restarted desktop dials it: both sides are online again with no relay anywhere.
    assert!(file.contains("direct_hints"), "{file}");
    wait_online_via(&mut desk2, voltip_identity::ConnectionKind::Direct).await;
    wait_online_via(&mut phone, voltip_identity::ConnectionKind::Direct).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    desk2.handle.send(CoreCommand::SendText { to: list[0].device.public_key, body: "still here".into() }).await.unwrap();
    assert_eq!(wait(&mut phone, |e| if let CoreEvent::Message { body, .. } = e { Some(body.clone()) } else { None }).await, "still here");
    // Now the phone restarts on a new ephemeral port: its stored hint for the desktop was refreshed
    // during the last session, so this time it is the phone that dials.
    phone.handle.send(CoreCommand::Shutdown).await.unwrap();
    wait_offline(&mut desk2).await;
    let dir = phone._dir;
    let store = phone.store.clone();
    let mut phone2 = node_with(dir, store, "ignored", None);
    wait_online_via(&mut phone2, voltip_identity::ConnectionKind::Direct).await;
    wait_online_via(&mut desk2, voltip_identity::ConnectionKind::Direct).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    phone2.handle.send(CoreCommand::RefreshDevices).await.unwrap();
    let desk_key = wait(&mut phone2, |e| if let CoreEvent::Devices(l) = e { l.first().map(|d| d.device.public_key) } else { None }).await;
    phone2.handle.send(CoreCommand::SendText { to: desk_key, body: "phone is back".into() }).await.unwrap();
    assert_eq!(wait(&mut desk2, |e| if let CoreEvent::Message { body, .. } = e { Some(body.clone()) } else { None }).await, "phone is back");
}

/// docs/pairing.md 「局域网发现」: with no relay and nothing typed, a phone finds the desktop that
/// waits for a pairing on the LAN, joins it with one tap (the safety codes still match), and once
/// both restart on new ports, where no stored address answers any more, they find each other again.
#[tokio::test]
async fn regression_lan_discovery_pairs_by_tap_and_finds_a_trusted_device_on_its_new_port() {
    use voltip_core::discovery::NearbyDevice;
    let lan = voltip_core::discovery::fake::LocalLan::default();
    let mut desk = node_on_lan(tempfile::tempdir().unwrap(), Arc::new(MemorySecretStore::new()), "Studio", &lan);
    let mut phone = node_on_lan(tempfile::tempdir().unwrap(), Arc::new(MemorySecretStore::new()), "Pixel 8", &lan);
    let nearby = |pick: fn(&NearbyDevice) -> bool| {
        move |e: &CoreEvent| match e {
            CoreEvent::Nearby(list) => list.iter().find(|d| pick(d)).cloned(),
            _ => None,
        }
    };
    // Seen before any pairing: not pairing, not trusted.
    let seen = wait(&mut phone, nearby(|d| d.name == "Studio")).await;
    assert!(!seen.pairing && !seen.trusted, "{seen:?}");
    desk.handle.send(CoreCommand::StartPairing).await.unwrap();
    wait_pairing(&mut desk, PairingState::WaitingForPeer).await;
    let studio = wait(&mut phone, nearby(|d| d.name == "Studio" && d.pairing)).await;
    phone.handle.send(CoreCommand::PairingJoinNearby(studio.fingerprint.clone())).await.unwrap();
    let vd = wait_pairing(&mut desk, PairingState::AwaitingVerification).await;
    let vp = wait_pairing(&mut phone, PairingState::AwaitingVerification).await;
    assert_eq!(vd.safety_code, vp.safety_code, "a tap pairs like a scan: both screens show the same code");
    desk.handle.send(CoreCommand::ConfirmPairing).await.unwrap();
    phone.handle.send(CoreCommand::ConfirmPairing).await.unwrap();
    wait_pairing(&mut desk, PairingState::Trusted).await;
    wait_pairing(&mut phone, PairingState::Trusted).await;
    wait_online_via(&mut desk, voltip_identity::ConnectionKind::Direct).await;
    wait_online_via(&mut phone, voltip_identity::ConnectionKind::Direct).await;
    // The desktop stops offering the pairing; the phone lists it as its own now.
    phone.handle.send(CoreCommand::RefreshDevices).await.unwrap();
    wait(&mut phone, nearby(|d| d.name == "Studio" && d.trusted && !d.pairing)).await;
    // An unknown or no longer pairing device cannot be joined.
    phone.handle.send(CoreCommand::PairingJoinNearby("0000000000000000".into())).await.unwrap();
    wait(&mut phone, |e| matches!(e, CoreEvent::Error(m) if m.contains("附近没有")).then_some(())).await;

    // Both restart: new ephemeral ports, so the addresses they told each other are stale.
    desk.handle.send(CoreCommand::Shutdown).await.unwrap();
    phone.handle.send(CoreCommand::Shutdown).await.unwrap();
    let (desk_dir, desk_store) = (desk._dir, desk.store.clone());
    let (phone_dir, phone_store) = (phone._dir, phone.store.clone());
    let mut desk = node_on_lan(desk_dir, desk_store, "ignored", &lan);
    let mut phone = node_on_lan(phone_dir, phone_store, "ignored", &lan);
    wait_online_via(&mut desk, voltip_identity::ConnectionKind::Direct).await;
    wait_online_via(&mut phone, voltip_identity::ConnectionKind::Direct).await;
    // Discovery can be switched off: nothing is announced or listed any more.
    phone.handle.send(CoreCommand::SetLanDiscovery(false)).await.unwrap();
    wait(&mut phone, |e| matches!(e, CoreEvent::Settings(s) if !s.lan_discovery).then_some(())).await;
    wait(&mut phone, |e| matches!(e, CoreEvent::Nearby(list) if list.is_empty()).then_some(())).await;
    desk.handle.send(CoreCommand::Shutdown).await.unwrap();
    phone.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// Regression (docs/pairing.md 「局域网发现」): with a relay on both sides, as shipped, the pairing
/// desktop still shows under 「附近的电脑」 and a tap pairs it (on the relay, where its session
/// waits, as after a scan). The announced ticket leaves the relay out, so the record fits one TXT
/// string whatever the relay's URL, and the relay's address is not broadcast to the LAN.
#[tokio::test]
async fn regression_a_relay_connected_desktop_offers_its_pairing_on_the_lan_without_its_relay() {
    use voltip_core::discovery::{MAX_TXT_VALUE_BYTES, txt};
    use voltip_protocol::ticket::PairingTicket;
    let (url, _stop, relay) = relay().await;
    // A long relay URL (the relay routes on the path): with it, the ticket would not fit.
    let url = format!("{url}?pad={}", "p".repeat(64));
    let lan = voltip_core::discovery::fake::LocalLan::default();
    let settings = Settings { relay_url: Some(url), relay_enabled: true, ..Settings::default() };
    let mut desk = node_config(tempfile::tempdir().unwrap(), Arc::new(MemorySecretStore::new()), "Studio", &settings, true, Some(lan.join()));
    let mut phone = node_config(tempfile::tempdir().unwrap(), Arc::new(MemorySecretStore::new()), "Pixel 8", &settings, true, Some(lan.join()));
    wait(&mut desk, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    wait(&mut phone, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    desk.handle.send(CoreCommand::StartPairing).await.unwrap();
    let shown = wait_pairing(&mut desk, PairingState::WaitingForPeer).await;
    assert!(PairingTicket::from_uri(shown.ticket_uri.as_deref().unwrap()).unwrap().relay_hint.is_some(), "the QR code keeps the relay");
    let studio = wait(&mut phone, |e| match e {
        CoreEvent::Nearby(list) => list.iter().find(|d| d.name == "Studio" && d.pairing).cloned(),
        _ => None,
    })
    .await;
    let record = lan.announced().into_iter().find(|a| a.name == "Studio").unwrap();
    let ticket = PairingTicket::from_uri(record.ticket.as_deref().unwrap()).unwrap();
    assert!(ticket.relay_hint.is_none() && ticket.direct_hints.is_empty() && record.on_relay, "{ticket:?}");
    assert!(txt(&record).iter().any(|(k, v)| k == "t" && v.len() <= MAX_TXT_VALUE_BYTES));
    phone.handle.send(CoreCommand::PairingJoinNearby(studio.fingerprint)).await.unwrap();
    let vd = wait_pairing(&mut desk, PairingState::AwaitingVerification).await;
    let vp = wait_pairing(&mut phone, PairingState::AwaitingVerification).await;
    assert_eq!(vd.safety_code, vp.safety_code);
    desk.handle.send(CoreCommand::ConfirmPairing).await.unwrap();
    phone.handle.send(CoreCommand::ConfirmPairing).await.unwrap();
    wait_pairing(&mut desk, PairingState::Trusted).await;
    wait_pairing(&mut phone, PairingState::Trusted).await;
    // Then straight over the LAN, at the address the phone saw.
    wait_online_via(&mut phone, voltip_identity::ConnectionKind::Direct).await;
    drop(relay);
    desk.handle.send(CoreCommand::Shutdown).await.unwrap();
    phone.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// Requirement: paired devices talk directly when they can and fall back to the relay when they
/// cannot; the relay is never required once devices are paired.
#[tokio::test]
async fn regression_paired_devices_prefer_direct_and_fall_back_to_relay() {
    use voltip_identity::ConnectionKind;
    let (url, _stop, relay) = relay().await;
    let mut desk = node("Desk", Some(&url));
    let mut phone = node("Phone", Some(&url));
    wait(&mut desk, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    wait(&mut phone, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    let (td, tp) = pair_by_code(&mut desk, &mut phone).await;
    // Paired over the relay (a 6-digit code needs one), yet both end up connected directly: each
    // side announced its LAN endpoint inside the encrypted channel and the other dialled it.
    wait_online_via(&mut desk, ConnectionKind::Direct).await;
    wait_online_via(&mut phone, ConnectionKind::Direct).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let forwarded_before = relay.stats().forwarded;
    desk.handle.send(CoreCommand::SendText { to: td.public_key, body: "over the lan".into() }).await.unwrap();
    assert_eq!(wait(&mut phone, |e| if let CoreEvent::Message { body, .. } = e { Some(body.clone()) } else { None }).await, "over the lan");
    phone.handle.send(CoreCommand::SendText { to: tp.public_key, body: "back over the lan".into() }).await.unwrap();
    assert_eq!(wait(&mut desk, |e| if let CoreEvent::Message { body, .. } = e { Some(body.clone()) } else { None }).await, "back over the lan");
    assert_eq!(relay.stats().forwarded, forwarded_before, "traffic between directly connected devices must not touch the relay");
    // Hints are persisted on both sides.
    wait_hints_learned(&mut desk).await;
    wait_hints_learned(&mut phone).await;
    assert!(std::fs::read_to_string(desk.dir_path.join("trusted-devices.json")).unwrap().contains("direct_hints"));
    assert!(std::fs::read_to_string(phone.dir_path.join("trusted-devices.json")).unwrap().contains("direct_hints"));

    // The phone moves to a network where its LAN host is unreachable (simulated: direct disabled).
    phone.handle.send(CoreCommand::Shutdown).await.unwrap();
    wait_offline(&mut desk).await;
    let dir = phone._dir;
    let store = phone.store.clone();
    let mut phone2 = node_opts(dir, store, "ignored", Some(&url), false);
    wait_online_via(&mut desk, ConnectionKind::Relay).await;
    wait_online_via(&mut phone2, ConnectionKind::Relay).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let forwarded_before = relay.stats().forwarded;
    desk.handle.send(CoreCommand::SendText { to: td.public_key, body: "via relay now".into() }).await.unwrap();
    assert_eq!(wait(&mut phone2, |e| if let CoreEvent::Message { body, .. } = e { Some(body.clone()) } else { None }).await, "via relay now");
    assert!(relay.stats().forwarded > forwarded_before, "fallback traffic goes through the relay");
    // The desktop stops dialling a LAN endpoint the phone no longer advertises.
    assert!(!std::fs::read_to_string(desk.dir_path.join("trusted-devices.json")).unwrap().contains("direct_hints"));
    desk.handle.send(CoreCommand::Shutdown).await.unwrap();
    phone2.handle.send(CoreCommand::Shutdown).await.unwrap();
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

/// A desktop whose recogniser is ready: the custom endpoint (the fake factory answers for it).
fn desktop_ready(name: &str, relay_url: &str) -> Node {
    let mut settings = Settings { relay_url: Some(relay_url.to_owned()), relay_enabled: true, ..Settings::default() };
    settings.engines.asr_provider = voltip_core::ProviderId::Custom;
    settings.engines.providers.insert(
        voltip_core::ProviderId::Custom,
        voltip_core::ProviderSettings { asr_url: Some("http://127.0.0.1:9/v1".into()), asr_model: Some("fake".into()), ..Default::default() },
    );
    node_settings(tempfile::tempdir().unwrap(), Arc::new(MemorySecretStore::new()), name, &settings, false)
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
    let mut phone = node_opts(tempfile::tempdir().unwrap(), Arc::new(MemorySecretStore::new()), "Pixel 8", Some(&url), false);
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
    let mut phone = node_opts(tempfile::tempdir().unwrap(), Arc::new(MemorySecretStore::new()), "Pixel 8", Some(&url), false);
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
        CoreEvent::History(entries) if entries.iter().any(|h| h.text == "会议改到三点") => Some(entries.clone()),
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
        CoreEvent::History(entries) => {
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
        CoreEvent::History(entries) => entries.first().and_then(|h| h.origin.clone()).filter(|o| o.kind == OriginKind::Take && o.device == "Pixel 8"),
        _ => None,
    })
    .await;

    // The list is the phone's, and it can forget it.
    phone.handle.send(CoreCommand::SentTextsClear).await.unwrap();
    wait(&mut phone, |e| sent_texts(e).filter(Vec::is_empty)).await;

    phone.handle.send(CoreCommand::Shutdown).await.unwrap();
    desk.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// Opt-in check against a **deployed** relay: `VOLTIP_LIVE_RELAY_URL=wss://host/ws cargo test -p
/// voltip-core --test e2e live_relay`. Two cores with the LAN side disabled pair by code through
/// that relay (TLS handshake, WebSocket upgrade through the reverse proxy, rendezvous, E2EE
/// message both ways). Skipped when the variable is unset so the offline suite stays hermetic.
#[tokio::test]
async fn live_relay_round_trip_when_configured() {
    let Some(url) = std::env::var("VOLTIP_LIVE_RELAY_URL").ok().filter(|u| !u.trim().is_empty()) else {
        eprintln!("live_relay_round_trip_when_configured: VOLTIP_LIVE_RELAY_URL unset, skipped");
        return;
    };
    let mut desk = node_opts(tempfile::tempdir().unwrap(), Arc::new(MemorySecretStore::new()), "Live-Desk", Some(&url), false);
    let mut phone = node_opts(tempfile::tempdir().unwrap(), Arc::new(MemorySecretStore::new()), "Live-Phone", Some(&url), false);
    wait(&mut desk, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    wait(&mut phone, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    let (td, tp) = pair_by_code(&mut desk, &mut phone).await;
    assert_eq!(td.name, "Live-Phone");
    assert_eq!(tp.name, "Live-Desk");
    wait_online_via(&mut desk, voltip_identity::ConnectionKind::Relay).await;
    wait_online_via(&mut phone, voltip_identity::ConnectionKind::Relay).await;
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

/// The connectivity self-check (docs/pairing.md): the phone probes the relay and the computer's
/// LAN address (both answer `hello`), pings the computer over the live channel, and reports all
/// of it; a second check while one runs is refused. Once the computer is gone, its address no
/// longer answers and it has no round trip.
#[tokio::test]
async fn the_connectivity_check_reports_the_relay_the_lan_and_each_device() {
    use voltip_core::connectivity::{ConnectivityStatus, ProbeResult};
    let (url, _stop, _relay) = relay().await;
    let mut desk = node("Desk", Some(&url));
    let mut phone = node("Phone", Some(&url));
    wait(&mut desk, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    wait(&mut phone, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
    let (_, desktop) = pair_by_code(&mut desk, &mut phone).await;
    wait_online(&mut phone).await;
    wait_hints_learned(&mut phone).await;

    phone.handle.send(CoreCommand::CheckConnectivity).await.unwrap();
    phone.handle.send(CoreCommand::CheckConnectivity).await.unwrap();
    wait(&mut phone, |e| matches!(e, CoreEvent::Connectivity(ConnectivityStatus { running: true, .. })).then_some(())).await;
    wait(&mut phone, |e| matches!(e, CoreEvent::Error(m) if m.contains("自检正在进行")).then_some(())).await;
    let first = connectivity_report(&mut phone).await;
    assert!(first.lan.listening && !first.lan.addresses.is_empty(), "{:?}", first.lan);
    assert!(first.relay.configured && matches!(first.relay.result, Some(ProbeResult::Ok { .. })), "{:?}", first.relay);
    assert_eq!(first.peers.len(), 1);
    let peer = &first.peers[0];
    assert_eq!((peer.public_key.as_str(), peer.name.as_str()), (desktop.public_key.to_hex().as_str(), "Desk"));
    assert!(peer.via.is_some() && peer.rtt_ms.is_some(), "{peer:?}");
    assert!(!peer.addresses.is_empty() && peer.addresses.iter().all(|a| matches!(a.result, ProbeResult::Ok { .. })), "{:?}", peer.addresses);

    // The computer shuts down: its LAN host stops answering and there is no live channel.
    desk.handle.send(CoreCommand::Shutdown).await.unwrap();
    wait_offline(&mut phone).await;
    phone.handle.send(CoreCommand::CheckConnectivity).await.unwrap();
    let second = connectivity_report(&mut phone).await;
    let peer = &second.peers[0];
    assert!(peer.via.is_none() && peer.rtt_ms.is_none(), "{peer:?}");
    assert!(peer.addresses.iter().all(|a| !matches!(a.result, ProbeResult::Ok { .. })), "{:?}", peer.addresses);
    phone.handle.send(CoreCommand::Shutdown).await.unwrap();
}
