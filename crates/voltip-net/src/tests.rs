//! The plan, the reqwest proxy and the tunnels, against proxies that record what they were asked.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use hyper_util::client::proxy::matcher::Matcher;
use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::*;

fn url(text: &str) -> Url {
    Url::parse(text).unwrap()
}

fn manual(server: &str, user: Option<&str>, pass: Option<&str>, no_proxy: &str) -> ProxyPlan {
    ProxyPlan::Manual(ManualProxy::new(server, user, pass, no_proxy).unwrap())
}

/// The plan decides for each URL; this device's own addresses always go direct.
#[test]
fn the_plan_routes_each_url() {
    let network = Network::with_plan(manual("127.0.0.1:7890", Some("alice"), Some("s3cr:et@"), "corp.test, 10.0.0.0/8, 192.168.1.5"));
    let via = network.proxy_for(&url("https://api.example.com/v1/x")).unwrap();
    assert_eq!(via.as_str(), "http://alice:s3cr%3Aet%40@127.0.0.1:7890/", "credentials percent-encoded for reqwest");
    for direct in [
        "https://corp.test/",
        "https://a.b.corp.test:8443/",
        "http://10.1.2.3/",
        "http://192.168.1.5:11434/",
        "http://localhost:11434/",
        "http://app.localhost/",
        "http://127.0.0.1:47840/v1",
        "http://127.8.9.10/",
        "http://[::1]:47840/",
        "http://0.0.0.0/",
    ] {
        assert_eq!(network.proxy_for(&url(direct)), None, "{direct}");
    }
    assert!(network.proxy_for(&url("http://notcorp.test/")).is_some(), "a domain matches itself and its subdomains, nothing else");
    assert!(network.proxy_for(&url("http://192.168.1.6/")).is_some());
    assert!(network.proxy_for(&url("wss://ws.example.com/ws")).is_some(), "WebSocket URLs count as HTTP(S)");
    assert_eq!(network.proxy_for(&url("file:///tmp/x")), None);
    network.set(ProxyPlan::Off);
    assert_eq!(network.proxy_for(&url("https://api.example.com/")), None);
    network.set(manual("socks5h://gw.example.com:1080", None, None, "*"));
    assert_eq!(network.proxy_for(&url("https://api.example.com/")), None, "`*` sends everything direct");
    network.set(manual("socks5h://gw.example.com:1080", None, None, ""));
    assert_eq!(network.proxy_for(&url("https://api.example.com/")).unwrap().as_str(), "socks5h://gw.example.com:1080");
    assert_eq!(network.plan().describe(), "manual socks5h", "the log names no host");
}

/// Following the system: the system's matcher (environment, OS settings), or on Android the
/// network's proxy the shell handed in, with its exclusions.
#[test]
fn following_the_system_reads_it_for_each_request() {
    let seen = Arc::new(Mutex::new(0usize));
    let reads = Arc::clone(&seen);
    let network = Network::with_system_matcher(move || {
        *reads.lock() += 1;
        Matcher::builder().https("http://bob:p%40ss@10.0.0.1:3128".to_owned()).no("intranet.test".to_owned()).build()
    });
    assert_eq!(network.plan(), ProxyPlan::System, "the default");
    let via = network.proxy_for(&url("https://api.example.com/")).unwrap();
    assert_eq!((via.host_str(), via.port(), via.username(), via.password()), (Some("10.0.0.1"), Some(3128), "bob", Some("p%40ss")));
    assert_eq!(network.proxy_for(&url("http://api.example.com/")), None, "only HTTPS has a proxy here");
    assert_eq!(network.proxy_for(&url("https://intranet.test/")), None);
    assert_eq!(*seen.lock(), 3, "read again for each request");
    assert_eq!(network.system_proxy_for(&url("https://api.example.com/")).unwrap().as_str(), "http://10.0.0.1:3128/", "without credentials");
    assert_eq!(network.system_https_proxy().unwrap().as_str(), "http://10.0.0.1:3128/");
    network.set(ProxyPlan::Off);
    assert!(network.system_proxy_for(&url("https://api.example.com/")).is_some(), "what the system says, whatever the plan");

    network.set(ProxyPlan::System);
    network.set_platform_proxy(Some(PlatformProxy::new("10.0.2.2", 8080, "*.corp.test").unwrap()));
    assert_eq!(network.proxy_for(&url("http://api.example.com/")).unwrap().as_str(), "http://10.0.2.2:8080/", "the platform's proxy takes both schemes");
    assert_eq!(network.proxy_for(&url("https://x.corp.test/")), None);
    assert_eq!(network.platform_proxy().map(|p| p.server().to_string()), Some("http://10.0.2.2:8080/".into()));
    network.set_platform_proxy(None);
    assert!(network.platform_proxy().is_none());
}

/// The HTTPS proxy, the HTTP proxy and the exclusions a test's system settings hold.
type SystemSettings = (Option<&'static str>, Option<&'static str>, &'static str);

/// A network on `system`'s settings, its raw form what the environment variables would hold.
fn network_on(system: &Arc<Mutex<SystemSettings>>) -> Arc<Network> {
    let reads = Arc::clone(system);
    Network::with_system_settings(move || {
        let (https, http, no) = *reads.lock();
        let mut builder = Matcher::builder().no(no.to_owned());
        if let Some(https) = https {
            builder = builder.https(https.to_owned());
        }
        if let Some(http) = http {
            builder = builder.http(http.to_owned());
        }
        (builder.build(), format!("HTTPS_PROXY={https:?} HTTP_PROXY={http:?} NO_PROXY={no:?}"))
    })
}

/// The system fingerprint changes with anything that could change a route: either proxy, its
/// credentials, an exclusion; on Android the network's proxy or its exclusions. The same settings
/// read again give the same one.
#[test]
fn the_system_fingerprint_follows_every_part_of_the_system_settings() {
    let system: Arc<Mutex<SystemSettings>> = Arc::new(Mutex::new((Some("http://10.0.0.1:3128"), None, "")));
    let network = network_on(&system);
    let first = network.system_fingerprint();
    assert_eq!(network.system_fingerprint(), first, "the same settings, the same fingerprint");
    let mut seen = vec![first];
    for change in [
        (Some("http://10.0.0.1:3128"), None, "intranet.test"),
        (Some("http://10.0.0.1:3128"), Some("http://10.0.0.2:3128"), "intranet.test"),
        (Some("http://bob:pw@10.0.0.1:3128"), Some("http://10.0.0.2:3128"), "intranet.test"),
        (Some("http://bob:other@10.0.0.1:3128"), Some("http://10.0.0.2:3128"), "intranet.test"),
        (None, None, ""),
    ] {
        *system.lock() = change;
        let now = network.system_fingerprint();
        assert!(!seen.contains(&now), "{change:?} changes the fingerprint");
        seen.push(now);
    }
    network.set_platform_proxy(Some(PlatformProxy::new("10.0.2.2", 8080, "corp.test").unwrap()));
    let platform = network.system_fingerprint();
    network.set_platform_proxy(Some(PlatformProxy::new("10.0.2.2", 8080, "corp.test, other.test").unwrap()));
    assert_ne!(network.system_fingerprint(), platform, "an exclusion of the network's proxy counts");
    network.set_platform_proxy(Some(PlatformProxy::new("10.0.2.2", 8080, "corp.test").unwrap()));
    assert_eq!(network.system_fingerprint(), platform);
}

/// Regression (goal gate round 2, 2026-10-12): the fingerprint once took the credentials from the
/// routes of two fixed addresses, so an exclusion naming them hid a change of the password.
#[test]
fn regression_a_password_change_counts_whatever_the_exclusions_name() {
    for excluded in ["example.com", "*"] {
        let system: Arc<Mutex<SystemSettings>> = Arc::new(Mutex::new((Some("http://bob:pw@10.0.0.1:3128"), Some("http://bob:pw@10.0.0.1:3128"), excluded)));
        let network = network_on(&system);
        let before = network.system_fingerprint();
        *system.lock() = (Some("http://bob:other@10.0.0.1:3128"), Some("http://bob:other@10.0.0.1:3128"), excluded);
        assert_ne!(network.system_fingerprint(), before, "excluding {excluded}");
    }
}

/// Regression (goal gate rounds 3 and 4, 2026-10-12): credentials that come in the proxy's own
/// address, as Windows' `ProxyServer` can carry them, count whatever the exclusions name: the raw
/// settings hold the address as the reader read it.
#[test]
fn regression_credentials_in_the_proxy_address_count_whatever_the_exclusions_name() {
    for excluded in ["", "example.com", "proxy-probe.voltip.invalid,198.51.100.1,2001:db8::1", "*"] {
        let password: Arc<Mutex<&'static str>> = Arc::new(Mutex::new("pw"));
        let reads = Arc::clone(&password);
        let network = Network::with_system_settings(move || {
            let proxy = format!("http://bob:{}@10.0.0.1:3128", *reads.lock());
            let matcher = Matcher::builder().http(proxy.clone()).https(proxy.clone()).no(excluded.to_owned()).build();
            // What Internet Options would hold.
            (matcher, format!("ProxyEnable=Some(1) ProxyServer={proxy:?} ProxyOverride={excluded:?}"))
        });
        let before = network.system_fingerprint();
        assert_eq!(network.system_fingerprint(), before);
        *password.lock() = "other";
        assert_ne!(network.system_fingerprint(), before, "excluding {excluded:?}");
    }
}

/// The production reader's raw settings name every variable hyper-util reads, and on Windows the
/// three Internet Options values.
#[test]
fn the_raw_system_settings_name_what_the_reader_reads() {
    let raw = system_settings();
    for name in PROXY_ENVIRONMENT {
        assert!(raw.contains(&format!("{name}=")), "{name}");
    }
    if cfg!(target_os = "windows") {
        let read = ["ProxyEnable=", "ProxyServer=", "ProxyOverride="].iter().all(|value| raw.contains(value));
        assert!(read || raw.contains("Internet Settings: none"), "{raw}");
    }
}

/// What an HTTP proxy was asked: the request line and its `Proxy-Authorization`.
type Asked = Arc<Mutex<Vec<(String, Option<String>)>>>;

/// An HTTP proxy: answers a plain request itself (`via proxy`), and opens a `CONNECT` tunnel to
/// the address `routes` gives the target.
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
                if let Some(target) = line.strip_prefix("CONNECT ").and_then(|rest| rest.split(' ').next()) {
                    let Some(to) = routes.get(target) else {
                        let _ = socket.write_all(b"HTTP/1.1 502 Bad Gateway\r\ncontent-length: 0\r\n\r\n").await;
                        return;
                    };
                    let mut upstream = TcpStream::connect(to).await.unwrap();
                    socket.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n").await.unwrap();
                    let _ = tokio::io::copy_bidirectional(&mut socket, &mut upstream).await;
                } else {
                    let _ = socket.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 9\r\nconnection: close\r\n\r\nvia proxy").await;
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

/// What a SOCKS5 proxy was asked: the target as sent (a name, or an address) and the credentials.
type SocksAsked = Arc<Mutex<Vec<(String, Option<(String, String)>)>>>;

/// A SOCKS5 proxy (RFC 1928, user name and password per RFC 1929 when `credentials`): `CONNECT`
/// only, to the address `routes` gives the target.
async fn socks_proxy(credentials: bool, routes: HashMap<String, SocketAddr>) -> (SocketAddr, SocksAsked) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let asked: SocksAsked = Arc::default();
    let log = Arc::clone(&asked);
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            let (log, routes) = (Arc::clone(&log), routes.clone());
            tokio::spawn(async move {
                let mut head = [0u8; 2];
                s.read_exact(&mut head).await.unwrap();
                let mut methods = vec![0u8; usize::from(head[1])];
                s.read_exact(&mut methods).await.unwrap();
                let wanted = if credentials { 2 } else { 0 };
                assert!(methods.contains(&wanted), "offered {methods:?}");
                s.write_all(&[5, wanted]).await.unwrap();
                let mut signed_in = None;
                if credentials {
                    let mut version = [0u8; 2];
                    s.read_exact(&mut version).await.unwrap();
                    let mut user = vec![0u8; usize::from(version[1])];
                    s.read_exact(&mut user).await.unwrap();
                    let mut len = [0u8; 1];
                    s.read_exact(&mut len).await.unwrap();
                    let mut pass = vec![0u8; usize::from(len[0])];
                    s.read_exact(&mut pass).await.unwrap();
                    signed_in = Some((String::from_utf8(user).unwrap(), String::from_utf8(pass).unwrap()));
                    s.write_all(&[1, 0]).await.unwrap();
                }
                let mut request = [0u8; 4];
                s.read_exact(&mut request).await.unwrap();
                let target = match request[3] {
                    1 => {
                        let mut ip = [0u8; 4];
                        s.read_exact(&mut ip).await.unwrap();
                        std::net::Ipv4Addr::from(ip).to_string()
                    }
                    3 => {
                        let mut len = [0u8; 1];
                        s.read_exact(&mut len).await.unwrap();
                        let mut name = vec![0u8; usize::from(len[0])];
                        s.read_exact(&mut name).await.unwrap();
                        String::from_utf8(name).unwrap()
                    }
                    other => panic!("address type {other}"),
                };
                let mut port = [0u8; 2];
                s.read_exact(&mut port).await.unwrap();
                let target = format!("{target}:{}", u16::from_be_bytes(port));
                log.lock().push((target.clone(), signed_in));
                let to = routes[&target];
                let mut upstream = TcpStream::connect(to).await.unwrap();
                s.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await.unwrap();
                let _ = tokio::io::copy_bidirectional(&mut s, &mut upstream).await;
            });
        }
    });
    (addr, asked)
}

/// Echoes what it reads.
async fn echo() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (mut read, mut write) = socket.split();
                let _ = tokio::io::copy(&mut read, &mut write).await;
            });
        }
    });
    addr
}

async fn round_trip(mut stream: TcpStream) -> String {
    stream.write_all(b"ping").await.unwrap();
    let mut answer = [0u8; 4];
    stream.read_exact(&mut answer).await.unwrap();
    String::from_utf8_lossy(&answer).into_owned()
}

const TIMEOUT: Duration = Duration::from_secs(10);

/// A WebSocket connection's stream through an HTTP proxy: `CONNECT host:port` with the user's
/// `Proxy-Authorization`, then the tunnel carries the bytes.
#[tokio::test]
async fn a_stream_tunnels_through_an_http_proxy_with_its_credentials() {
    let target = echo().await;
    let (proxy, asked) = http_proxy([("relay.test:443".to_owned(), target)].into()).await;
    let network = Network::with_plan(manual(&format!("http://{proxy}"), Some("alice"), Some("s3cr:et@"), ""));
    let stream = network.connect(&url("wss://relay.test/ws"), TIMEOUT).await.unwrap();
    assert_eq!(round_trip(stream).await, "ping");
    let asked = asked.lock().clone();
    assert_eq!(asked[0].0, "CONNECT relay.test:443 HTTP/1.1");
    let token = base64::engine::general_purpose::STANDARD.encode("alice:s3cr:et@");
    assert_eq!(asked[0].1.as_deref(), Some(format!("Basic {token}").as_str()));
}

/// SOCKS5 with remote names (socks5h) sends the name; SOCKS5 sends an address as it is; the user
/// name and password go in the RFC 1929 exchange.
#[tokio::test]
async fn a_stream_tunnels_through_socks5_with_its_credentials() {
    let target = echo().await;
    let (proxy, asked) = socks_proxy(true, [("relay.test:8443".to_owned(), target), ("192.0.2.10:443".to_owned(), target)].into()).await;
    let network = Network::with_plan(manual(&format!("socks5h://{proxy}"), Some("alice"), Some("pw"), ""));
    assert_eq!(round_trip(network.connect(&url("wss://relay.test:8443/"), TIMEOUT).await.unwrap()).await, "ping");
    network.set(manual(&format!("socks5://{proxy}"), Some("alice"), Some("pw"), ""));
    assert_eq!(round_trip(network.connect(&url("wss://192.0.2.10/"), TIMEOUT).await.unwrap()).await, "ping");
    let asked = asked.lock().clone();
    assert_eq!(
        asked,
        [
            ("relay.test:8443".to_owned(), Some(("alice".to_owned(), "pw".to_owned()))),
            ("192.0.2.10:443".to_owned(), Some(("alice".to_owned(), "pw".to_owned())))
        ]
    );
    let (open, asked) = socks_proxy(false, [("relay.test:443".to_owned(), target)].into()).await;
    network.set(manual(&format!("socks5h://{open}"), None, None, ""));
    assert_eq!(round_trip(network.connect(&url("wss://relay.test/"), TIMEOUT).await.unwrap()).await, "ping");
    assert_eq!(asked.lock()[0], ("relay.test:443".to_owned(), None), "no credentials offered when none are set");
}

/// A local host is dialled directly whatever the plan; a refused tunnel says so, naming neither the
/// proxy nor its credentials (the text reaches log lines).
#[tokio::test]
async fn a_local_stream_goes_direct_and_a_refused_tunnel_names_no_host() {
    let target = echo().await;
    let (proxy, asked) = http_proxy(HashMap::new()).await;
    let network = Network::with_plan(manual(&format!("http://{proxy}"), Some("alice"), Some("secret"), ""));
    let direct = network.connect(&url(&format!("ws://127.0.0.1:{}/", target.port())), TIMEOUT).await.unwrap();
    assert_eq!(round_trip(direct).await, "ping");
    assert!(asked.lock().is_empty(), "the proxy saw nothing");
    let err = network.connect(&url("wss://elsewhere.test/"), TIMEOUT).await.unwrap_err();
    let text = err.to_string();
    assert!(matches!(err, ConnectError::Proxy(_)), "{text}");
    assert!(!text.contains(&proxy.ip().to_string()) && !text.contains("secret") && !text.contains("alice"), "{text}");
    let https = Network::with_system_matcher(|| Matcher::builder().https("https://10.9.9.9:443".to_owned()).build());
    let err = https.connect(&url("wss://elsewhere.test/"), TIMEOUT).await.unwrap_err();
    assert!(matches!(&err, ConnectError::Unsupported(scheme) if scheme == "https"), "{err}");
    assert!(!err.to_string().contains("10.9.9.9"), "{err}");
}

/// A reqwest client asks the plan on every request: through the proxy (with its credentials),
/// direct once the plan says off, through it again after; this device's own server never via it.
#[tokio::test]
async fn a_client_follows_the_plan_from_one_request_to_the_next() {
    let (proxy, asked) = http_proxy(HashMap::new()).await;
    let network = Network::with_plan(manual(&format!("http://{proxy}"), Some("alice"), Some("s3cr:et@"), ""));
    let client = network.apply(reqwest::Client::builder().timeout(TIMEOUT)).build().unwrap();
    let body = client.get("http://api.test/v1/models").send().await.unwrap().text().await.unwrap();
    assert_eq!(body, "via proxy");
    {
        let asked = asked.lock();
        assert_eq!(asked[0].0, "GET http://api.test/v1/models HTTP/1.1");
        let token = base64::engine::general_purpose::STANDARD.encode("alice:s3cr:et@");
        assert_eq!(asked[0].1.as_deref(), Some(format!("Basic {token}").as_str()));
    }
    network.set(ProxyPlan::Off);
    assert!(client.get("http://api.test/v1/models").send().await.is_err(), "direct: the name does not resolve");
    assert_eq!(asked.lock().len(), 1);
    network.set(manual(&format!("http://{proxy}"), None, None, "api.test"));
    assert!(client.get("http://api.test/v1/models").send().await.is_err(), "no-proxy hosts go direct");
    assert_eq!(asked.lock().len(), 1);
    network.set(manual(&format!("http://{proxy}"), None, None, ""));
    assert_eq!(client.get("http://api.test/").send().await.unwrap().text().await.unwrap(), "via proxy");
    assert_eq!(asked.lock()[1].1, None, "no credentials set, none sent");
    let local = local_http().await;
    assert_eq!(client.get(format!("http://127.0.0.1:{}/", local.port())).send().await.unwrap().text().await.unwrap(), "local");
    assert_eq!(client.get(format!("http://localhost:{}/", local.port())).send().await.unwrap().text().await.unwrap(), "local");
    assert_eq!(asked.lock().len(), 2, "this device's own servers never reach the proxy");
}

/// An HTTP server on this device that answers `local`.
async fn local_http() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let _ = read_head(&mut socket).await;
                let _ = socket.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 5\r\nconnection: close\r\n\r\nlocal").await;
            });
        }
    });
    addr
}

/// An address whose connection attempts go unanswered, as a network that blackholes an address
/// leaves them: a listener with no room left in its queue (backlog 0, one connection already
/// waiting) drops the SYN of every further attempt on Linux and macOS. Windows answers with a
/// reset instead, which the code under test takes as one more failed attempt. Keep the returned
/// pair alive for as long as the address must stay silent.
async fn silent_address() -> (SocketAddr, (TcpListener, TcpStream)) {
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let addr = socket.local_addr().unwrap();
    let listener = socket.listen(0).unwrap();
    let waiting = TcpStream::connect(addr).await.unwrap();
    (addr, (listener, waiting))
}

/// An address nothing listens on: bound without `listen`, so no other test's server takes it.
fn refusing_address() -> (SocketAddr, tokio::net::TcpSocket) {
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    (socket.local_addr().unwrap(), socket)
}

/// Regression (issue #193): a phone in mainland China could not reach the relay, whose name had
/// three addresses, one of them unreachable from there. The direct connection tried the addresses
/// one after another inside a single deadline, and when the silent one came first it spent all of
/// it there, attempt after attempt. Now the next address goes out after [`ATTEMPT_DELAY`] and the
/// stream connects to it.
#[tokio::test]
async fn regression_a_name_whose_first_address_never_answers_still_connects() {
    let (silent, _hold) = silent_address().await;
    let target = echo().await;
    let network = Network::with_plan(ProxyPlan::Off);
    network.set_resolver(move |name, _| {
        assert_eq!(name, "relay.test");
        Ok(vec![silent, target])
    });
    let stream = network.connect(&url("wss://relay.test/ws"), Duration::from_secs(5)).await.unwrap();
    assert_eq!(stream.peer_addr().unwrap(), target);
    assert_eq!(round_trip(stream).await, "ping");
}

/// An attempt that fails starts the next address at once, without waiting out the delay; every
/// address failing gives the last one's error; a name without addresses is an error of its own.
#[tokio::test]
async fn a_failed_address_hands_over_at_once_and_the_last_error_is_kept() {
    let target = echo().await;
    let (refused, _bound) = refusing_address();
    let (also_refused, _also) = refusing_address();
    // A delay the test would never outlast: only a failure can start the second attempt.
    let patient = Duration::from_secs(600);
    let stream = tokio::time::timeout(TIMEOUT, connect::race(vec![refused, target], patient, TcpStream::connect))
        .await
        .expect("a failed attempt waited for the delay")
        .unwrap();
    assert_eq!(stream.peer_addr().unwrap(), target);
    let error = tokio::time::timeout(TIMEOUT, connect::race(vec![refused, also_refused], patient, TcpStream::connect)).await.unwrap().unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::ConnectionRefused, "{error}");
    let none = connect::race(Vec::new(), patient, TcpStream::connect).await.unwrap_err();
    assert_eq!(none.kind(), std::io::ErrorKind::NotFound);

    let network = Network::with_plan(ProxyPlan::Off);
    network.set_resolver(|_, _| Err(std::io::Error::other("no such name")));
    let err = network.connect(&url("wss://relay.test/ws"), TIMEOUT).await.unwrap_err();
    assert!(matches!(&err, ConnectError::Resolve(_)), "{err}");
    network.set_resolver(|_, _| Ok(Vec::new()));
    assert!(matches!(network.connect(&url("wss://relay.test/ws"), TIMEOUT).await, Err(ConnectError::Resolve(_))));
}

/// Every address keeps its attempt running while the next goes out: the slow first one still wins
/// when it answers before the others.
#[tokio::test]
async fn an_attempt_keeps_running_after_the_next_address_starts() {
    let target = echo().await;
    let (silent, _hold) = silent_address().await;
    let first = target;
    let attempts = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&attempts);
    let stream = tokio::time::timeout(
        TIMEOUT,
        connect::race(vec![first, silent], Duration::from_millis(20), move |addr| {
            seen.lock().push(addr);
            async move {
                if addr == first {
                    // Slower than the delay: the silent address has been tried meanwhile.
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
                TcpStream::connect(addr).await
            }
        }),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(stream.peer_addr().unwrap(), target);
    assert_eq!(*attempts.lock(), [first, silent]);
}

/// The address families take turns, the first address's family first (RFC 8305 §4).
#[test]
fn the_address_families_take_turns() {
    let v4 = |n: u8| SocketAddr::from(([192, 0, 2, n], 443));
    let v6 = |n: u16| SocketAddr::from(([0x2001, 0xdb8, 0, 0, 0, 0, 0, n], 443));
    assert_eq!(connect::interleave(vec![v6(1), v6(2), v6(3), v4(1), v4(2)]), [v6(1), v4(1), v6(2), v4(2), v6(3)]);
    assert_eq!(connect::interleave(vec![v4(1), v4(2), v6(1), v6(2), v6(3)]), [v4(1), v6(1), v4(2), v6(2), v6(3)]);
    assert_eq!(connect::interleave(vec![v4(3), v4(1)]), [v4(3), v4(1)], "one family keeps its order");
    assert!(connect::interleave(Vec::new()).is_empty());
}
