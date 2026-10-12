#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The proxy settings (docs/dictation.md §24) against a real relay and proxies that record what
//! they were asked: the plan in force from the start, the relay's connection through the proxy
//! with the keychain's password, a change checked, saved and put in force, and following the
//! system read again when asked.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use voltip_core::ui::NetworkStatus;
use voltip_core::{AppCore, CoreCommand, CoreConfig, CoreEvent, CoreHandle, PROXY_PASSWORD_ENTRY, ProxyMode, ProxySettings, Settings, SettingsStore};
use voltip_identity::{MemorySecretStore, SecretStore};
use voltip_net::{Network, ProxyPlan};
use voltip_relay::RelayConfig;
use voltip_relay::server::RelayHandle;
use voltip_transport::ConnectionState;

struct Node {
    handle: CoreHandle,
    events: mpsc::Receiver<CoreEvent>,
    network: Arc<Network>,
    store: Arc<MemorySecretStore>,
    dir: tempfile::TempDir,
}

/// A core with `settings` saved first and `network` as its proxy plan's home.
fn node(settings: &Settings, store: Arc<MemorySecretStore>, network: Arc<Network>) -> Node {
    let dir = tempfile::tempdir().unwrap();
    SettingsStore::new(dir.path()).save(settings).unwrap();
    let mut cfg = CoreConfig::new(dir.path().to_path_buf());
    cfg.tick = Duration::from_millis(50);
    cfg.network = Arc::clone(&network);
    let (handle, events) = AppCore::start(cfg, store.clone()).unwrap();
    Node { handle, events, network, store, dir }
}

async fn wait<T>(node: &mut Node, mut pick: impl FnMut(&CoreEvent) -> Option<T>) -> T {
    loop {
        let ev = tokio::time::timeout(Duration::from_secs(15), node.events.recv()).await.expect("event within 15 s").expect("core alive");
        if let Some(v) = pick(&ev) {
            return v;
        }
    }
}

async fn wait_network(node: &mut Node) -> NetworkStatus {
    wait(node, |e| if let CoreEvent::Network(status) = e { Some(status.clone()) } else { None }).await
}

async fn wait_error(node: &mut Node) -> String {
    wait(node, |e| if let CoreEvent::Error(text) = e { Some(text.clone()) } else { None }).await
}

async fn wait_relay(node: &mut Node, state: ConnectionState) {
    wait(node, |e| matches!(e, CoreEvent::Relay(r) if r.state == state).then_some(())).await;
}

/// A relay on this device, and its port.
async fn relay() -> (u16, tokio::sync::oneshot::Sender<()>) {
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let (addr, _task) = RelayHandle::new(RelayConfig::default())
        .serve("127.0.0.1:0".parse().unwrap(), async move {
            let _ = stop_rx.await;
        })
        .await
        .unwrap();
    (addr.port(), stop_tx)
}

/// What an HTTP proxy was asked: the request line and its `Proxy-Authorization`.
type Asked = Arc<Mutex<Vec<(String, Option<String>)>>>;

/// An HTTP proxy that opens a `CONNECT` tunnel to the address `routes` gives the target, and
/// answers 502 for any other.
async fn http_proxy(routes: HashMap<String, SocketAddr>) -> (SocketAddr, Asked) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let asked: Asked = Arc::default();
    let log = Arc::clone(&asked);
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let (log, routes) = (Arc::clone(&log), routes.clone());
            tokio::spawn(async move {
                let head = read_head(&mut socket).await;
                let line = head.lines().next().unwrap_or_default().to_owned();
                let auth =
                    head.lines().find_map(|l| l.strip_prefix("Proxy-Authorization: ").or_else(|| l.strip_prefix("proxy-authorization: "))).map(str::to_owned);
                log.lock().push((line.clone(), auth));
                let target = line.strip_prefix("CONNECT ").and_then(|rest| rest.split(' ').next()).and_then(|t| routes.get(t).copied());
                let Some(to) = target else {
                    let _ = socket.write_all(b"HTTP/1.1 502 Bad Gateway\r\ncontent-length: 0\r\n\r\n").await;
                    return;
                };
                let Ok(mut upstream) = TcpStream::connect(to).await else { return };
                if socket.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n").await.is_ok() {
                    let _ = tokio::io::copy_bidirectional(&mut socket, &mut upstream).await;
                }
            });
        }
    });
    (addr, asked)
}

async fn read_head(socket: &mut TcpStream) -> String {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") && head.len() < 16_384 {
        if socket.read(&mut byte).await.unwrap_or(0) == 0 {
            break;
        }
        head.push(byte[0]);
    }
    String::from_utf8_lossy(&head).into_owned()
}

fn basic(user: &str, pass: &str) -> String {
    format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}")))
}

fn manual(server: &str, username: Option<&str>) -> ProxySettings {
    ProxySettings { mode: ProxyMode::Manual, server: Some(server.to_owned()), username: username.map(str::to_owned), no_proxy: String::new() }
}

/// The relay's name, which only the proxies know: a connection that does not go through one fails.
const RELAY_HOST: &str = "relay.test";

fn relay_settings(relay_port: u16, proxy: ProxySettings) -> Settings {
    Settings { relay_url: Some(format!("ws://{RELAY_HOST}:{relay_port}/ws")), relay_enabled: true, proxy, ..Settings::default() }
}

/// The saved plan is in force from the start: the relay connects through the manual proxy, signed
/// in with the keychain's password, and the interface learns that a password is saved.
#[tokio::test]
async fn the_relay_connects_through_the_saved_proxy_with_the_keychain_password() {
    let (relay_port, _stop) = relay().await;
    let target = format!("{RELAY_HOST}:{relay_port}");
    let (proxy, asked) = http_proxy([(target.clone(), SocketAddr::from(([127, 0, 0, 1], relay_port)))].into()).await;
    let store = Arc::new(MemorySecretStore::new());
    store.set(PROXY_PASSWORD_ENTRY, b"s3cr:et").unwrap();
    let settings = relay_settings(relay_port, manual(&format!("http://{proxy}"), Some("alice")));
    let mut desk = node(&settings, store, Network::new());
    assert!(matches!(desk.network.plan(), ProxyPlan::Manual(p) if p.username() == Some("alice") && p.has_password()), "in force before the core runs");
    assert!(wait_network(&mut desk).await.proxy_password);
    wait_relay(&mut desk, ConnectionState::Connected).await;
    let asked = asked.lock().clone();
    assert_eq!(asked[0], (format!("CONNECT {target} HTTP/1.1"), Some(basic("alice", "s3cr:et"))));
    let saved = std::fs::read_to_string(desk.dir.path().join(voltip_core::SETTINGS_FILE_NAME)).unwrap();
    assert!(!saved.contains("s3cr"), "the password never reaches settings.json");
}

/// A change is checked first (nothing is saved when it is refused), then saved tidied, its
/// password kept in the keychain, and put in force: the engines' clients are rebuilt and the
/// relay reconnects through the new proxy.
#[tokio::test]
async fn a_proxy_change_is_checked_saved_and_put_in_force() {
    let (relay_port, _stop) = relay().await;
    let target = format!("{RELAY_HOST}:{relay_port}");
    let routes: HashMap<String, SocketAddr> = [(target.clone(), SocketAddr::from(([127, 0, 0, 1], relay_port)))].into();
    let (first, first_asked) = http_proxy(routes.clone()).await;
    let (second, second_asked) = http_proxy(routes).await;
    let mut desk = node(&relay_settings(relay_port, manual(&format!("http://{first}"), None)), Arc::new(MemorySecretStore::new()), Network::new());
    assert!(!wait_network(&mut desk).await.proxy_password);
    wait_relay(&mut desk, ConnectionState::Connected).await;
    assert_eq!(first_asked.lock().len(), 1);

    for (proxy, password, want) in [
        (manual("127.0.0.1", None), None, "proxy.server: 代理地址缺少端口，例如 127.0.0.1:7890"),
        (manual("https://127.0.0.1:8443", None), None, "proxy.server: 不支持 https 代理，可以使用 http、socks5、socks5h"),
        (manual("http://u:p@127.0.0.1:8080", None), None, "proxy.server: 地址中不要包含用户名或密码，请填写在用户名和密码中"),
        (
            ProxySettings { no_proxy: "ok.test, bad@entry".into(), ..manual("127.0.0.1:8080", None) },
            None,
            "proxy.no_proxy: 「bad@entry」不是有效的主机、域名、IP 地址或网段",
        ),
        (ProxySettings { mode: ProxyMode::Manual, ..ProxySettings::default() }, None, "proxy.server: 手动代理需要填写代理地址"),
        (manual("127.0.0.1:8080", None), Some("pw".to_owned()), "proxy.username: 填写密码时也需要填写用户名"),
    ] {
        desk.handle.send(CoreCommand::SetProxy { proxy, password }).await.unwrap();
        assert_eq!(wait_error(&mut desk).await, want);
    }
    let saved = SettingsStore::new(desk.dir.path()).load().unwrap();
    assert_eq!(saved.proxy.server.as_deref(), Some(format!("http://{first}").as_str()), "a refused change saves nothing");

    let typed = ProxySettings {
        mode: ProxyMode::Manual,
        server: Some(format!(" {second} ")),
        username: Some(" bob ".into()),
        no_proxy: "*.corp.test; 10.0.0.0/8".into(),
    };
    desk.handle.send(CoreCommand::SetProxy { proxy: typed, password: Some("pa ss".into()) }).await.unwrap();
    let tidy = ProxySettings {
        mode: ProxyMode::Manual,
        server: Some(format!("http://{second}")),
        username: Some("bob".into()),
        no_proxy: "corp.test,10.0.0.0/8".into(),
    };
    let settings = wait(&mut desk, |e| if let CoreEvent::Settings(s) = e { Some(s.clone()) } else { None }).await;
    assert_eq!(settings.proxy, tidy);
    assert!(wait_network(&mut desk).await.proxy_password);
    wait(&mut desk, |e| matches!(e, CoreEvent::Engines(_)).then_some(())).await;
    wait_relay(&mut desk, ConnectionState::Connected).await;
    assert_eq!(second_asked.lock()[0], (format!("CONNECT {target} HTTP/1.1"), Some(basic("bob", "pa ss"))));
    assert_eq!(desk.store.get(PROXY_PASSWORD_ENTRY).unwrap().map(|v| v.to_vec()), Some(b"pa ss".to_vec()));
    assert_eq!(SettingsStore::new(desk.dir.path()).load().unwrap().proxy, tidy);

    // `None` keeps the saved password; the mode changes alone, the manual fields stay for later.
    let off = ProxySettings { mode: ProxyMode::Off, ..tidy.clone() };
    desk.handle.send(CoreCommand::SetProxy { proxy: off.clone(), password: None }).await.unwrap();
    assert!(wait_network(&mut desk).await.proxy_password);
    assert_eq!(desk.network.plan(), ProxyPlan::Off);
    assert_eq!(SettingsStore::new(desk.dir.path()).load().unwrap().proxy, off);
    // An empty password removes it.
    desk.handle.send(CoreCommand::SetProxy { proxy: off, password: Some(String::new()) }).await.unwrap();
    assert!(!wait_network(&mut desk).await.proxy_password);
    assert!(desk.store.get(PROXY_PASSWORD_ENTRY).unwrap().is_none());
}

/// Following the system: the settings show its proxy, read again on `RefreshNetwork`; any change
/// of the system's settings rebuilds the engines' clients, the same reading does not, and with a
/// manual proxy in force the system's proxy is only shown.
#[tokio::test]
async fn following_the_system_is_read_again_when_asked() {
    let system: Arc<Mutex<Option<String>>> = Arc::default();
    let excluded: Arc<Mutex<String>> = Arc::default();
    let (reads, exclusions) = (Arc::clone(&system), Arc::clone(&excluded));
    let network = Network::with_system_settings(move || {
        let (proxy, no) = (reads.lock().clone(), exclusions.lock().clone());
        let builder = hyper_util::client::proxy::matcher::Matcher::builder().no(no.clone());
        let matcher = match proxy.clone() {
            Some(proxy) => builder.https(proxy).build(),
            None => builder.build(),
        };
        // What the environment variables would hold.
        (matcher, format!("HTTPS_PROXY={proxy:?} NO_PROXY={no:?}"))
    });
    let settings = Settings { relay_enabled: false, ..Settings::default() };
    let mut desk = node(&settings, Arc::new(MemorySecretStore::new()), network);
    assert_eq!(wait_network(&mut desk).await, NetworkStatus { proxy_password: false, system_proxy: None });

    *system.lock() = Some("http://user:pw@10.0.0.1:3128".into());
    desk.handle.send(CoreCommand::RefreshNetwork).await.unwrap();
    assert_eq!(wait_network(&mut desk).await.system_proxy.as_deref(), Some("http://10.0.0.1:3128"), "shown without its credentials");
    wait(&mut desk, |e| matches!(e, CoreEvent::Engines(_)).then_some(())).await;

    desk.handle.send(CoreCommand::RefreshNetwork).await.unwrap();
    desk.handle.send(CoreCommand::RefreshDevices).await.unwrap();
    let mut rebuilt = false;
    wait(&mut desk, |e| {
        rebuilt |= matches!(e, CoreEvent::Engines(_));
        matches!(e, CoreEvent::Devices(_)).then_some(())
    })
    .await;
    assert!(!rebuilt, "the same reading changes nothing");

    // Regression (goal gate, 2026-10-12): a change the shown proxy does not reflect — an exclusion
    // only, the credentials only, the credentials while the exclusions name every address the
    // settings could probe — reroutes all the same.
    for change in [0, 1, 2] {
        match change {
            0 => *excluded.lock() = "intranet.test".into(),
            1 => *system.lock() = Some("http://user:other@10.0.0.1:3128".into()),
            _ => {
                *excluded.lock() = "intranet.test,example.com".into();
                desk.handle.send(CoreCommand::RefreshNetwork).await.unwrap();
                wait(&mut desk, |e| matches!(e, CoreEvent::Engines(_)).then_some(())).await;
                *system.lock() = Some("http://user:third@10.0.0.1:3128".into());
            }
        }
        desk.handle.send(CoreCommand::RefreshNetwork).await.unwrap();
        let status = wait_network(&mut desk).await;
        assert_eq!(status.system_proxy.as_deref(), Some("http://10.0.0.1:3128"), "the shown proxy is the same");
        wait(&mut desk, |e| matches!(e, CoreEvent::Engines(_)).then_some(())).await;
    }

    desk.handle.send(CoreCommand::SetProxy { proxy: manual("127.0.0.1:7890", None), password: None }).await.unwrap();
    wait(&mut desk, |e| matches!(e, CoreEvent::Engines(_)).then_some(())).await;
    *system.lock() = None;
    desk.handle.send(CoreCommand::RefreshNetwork).await.unwrap();
    desk.handle.send(CoreCommand::RefreshDevices).await.unwrap();
    let mut rebuilt = false;
    let status = wait(&mut desk, |e| {
        rebuilt |= matches!(e, CoreEvent::Engines(_));
        if let CoreEvent::Network(status) = e { Some(status.clone()) } else { None }
    })
    .await;
    assert_eq!(status.system_proxy, None);
    wait(&mut desk, |e| {
        rebuilt |= matches!(e, CoreEvent::Engines(_));
        matches!(e, CoreEvent::Devices(_)).then_some(())
    })
    .await;
    assert!(!rebuilt, "a manual proxy does not follow the system");
}

/// A settings file edited by hand into a proxy that cannot be used: the app follows the system
/// and says why; the setting stays as it is, for the user to correct.
#[tokio::test]
async fn an_unusable_saved_proxy_follows_the_system_and_says_so() {
    let settings = Settings { relay_enabled: false, proxy: manual("not a proxy", None), ..Settings::default() };
    let mut desk = node(&settings, Arc::new(MemorySecretStore::new()), Network::new());
    assert_eq!(desk.network.plan(), ProxyPlan::System);
    assert_eq!(wait_error(&mut desk).await, "代理设置无法使用，已改为跟随系统：代理地址缺少端口，例如 127.0.0.1:7890");
    assert_eq!(SettingsStore::new(desk.dir.path()).load().unwrap().proxy.server.as_deref(), Some("not a proxy"));
}

/// A settings file from before the proxy setting follows the system.
#[test]
fn settings_from_before_the_proxy_follow_the_system() {
    let dir = tempfile::tempdir().unwrap();
    let mut json = serde_json::to_value(Settings::default()).unwrap();
    json.as_object_mut().unwrap().remove("proxy");
    std::fs::write(dir.path().join(voltip_core::SETTINGS_FILE_NAME), json.to_string()).unwrap();
    assert_eq!(SettingsStore::new(dir.path()).load().unwrap().proxy, ProxySettings::default());
    assert_eq!(ProxySettings::default().mode, ProxyMode::System);
}
