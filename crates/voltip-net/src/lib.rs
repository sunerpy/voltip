//! The proxy every connection Voltip makes goes through (docs/dictation.md §24): recognition, AI
//! polish, the providers' 测试连接, local model downloads, feedback, update checks and downloads,
//! and the WebSocket connections (Model Studio's realtime recognition, the phone relay).
//!
//! * [`ProxyPlan`] — what the settings ask for: the system's proxy (the default), none, or one
//!   the user set ([`ManualProxy`]: `http://`, `socks5://` or `socks5h://`, a user name and
//!   password, the hosts that go direct).
//! * [`Network`] — the plan in force, shared by every client of a process. [`Network::apply`]
//!   gives a reqwest client a proxy that asks the plan on every request, so a change applies from
//!   the next request on without rebuilding the client; [`Network::connect`] opens a TCP stream to
//!   a host, through the proxy when the plan says so, for a WebSocket handshake to run on.
//! * The system's proxy is the environment (`HTTPS_PROXY`, `HTTP_PROXY`, `ALL_PROXY`, `NO_PROXY`),
//!   then macOS's network settings or Windows' Internet Options, read for each request
//!   (hyper-util's reader). On Android nothing here can read it: the shell hands in the network's
//!   proxy ([`Network::set_platform_proxy`]).
//! * This device's own addresses (`localhost`, `127.0.0.0/8`, `::1`) always go direct, whatever
//!   the plan: a local Ollama or Voltip's own speech service is never sent to a proxy.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod connect;
mod plan;

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use hyper_util::client::proxy::matcher::{Intercept, Matcher};
use parking_lot::RwLock;
use tokio::net::TcpStream;
use url::{Host, Url};

pub use connect::ConnectError;
pub use plan::{
    MAX_NO_PROXY_BYTES, MAX_PASSWORD_BYTES, MAX_USERNAME_BYTES, ManualProxy, PROXY_SCHEMES, PlatformProxy, ProxyError, ProxyPlan, parse_no_proxy, parse_server,
};

/// Reads the system's proxy settings: hyper-util's reader unless a test hands in its own.
type SystemSource = Box<dyn Fn() -> Matcher + Send + Sync>;
/// The raw settings the system's matcher is made from, credentials included, for
/// [`Network::system_fingerprint`]: the matcher's Debug leaves the credentials out.
type SystemRaw = Box<dyn Fn() -> String + Send + Sync>;

/// The environment variables hyper-util's reader takes the proxies, their credentials and the
/// exclusions from (`REQUEST_METHOD` makes it ignore `HTTP_PROXY`). macOS's network settings and
/// Windows' Internet Options give it addresses and exclusions only, which the matcher shows.
const PROXY_ENVIRONMENT: [&str; 9] =
    ["ALL_PROXY", "all_proxy", "HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy", "NO_PROXY", "no_proxy", "REQUEST_METHOD"];

fn environment_settings() -> String {
    PROXY_ENVIRONMENT.iter().map(|name| format!("{name}={:?}\n", std::env::var_os(name))).collect()
}

/// The proxy plan in force for a process, shared by its clients (`Arc<Network>`).
pub struct Network {
    state: RwLock<State>,
    system: SystemSource,
    system_raw: SystemRaw,
}

#[derive(Default)]
struct State {
    plan: ProxyPlan,
    /// The manual proxy's no-proxy list as a matcher, built when the plan is set.
    manual: Option<Matcher>,
    /// The platform network's proxy (Android) and its exclusions as a matcher.
    platform: Option<(PlatformProxy, Matcher)>,
}

/// Where one connection goes: the proxy, and how to sign in to it.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Route {
    /// `scheme://host:port`, without credentials.
    pub(crate) server: Url,
    /// User name and password, as they are (not percent-encoded).
    pub(crate) credentials: Option<(String, String)>,
}

impl Route {
    /// The address with the credentials in it (percent-encoded), as reqwest takes a proxy.
    fn url(&self) -> Url {
        let mut url = self.server.clone();
        if let Some((user, pass)) = &self.credentials {
            // The server has a host, so it can carry credentials.
            let _ = url.set_username(user);
            let _ = url.set_password((!pass.is_empty()).then_some(pass.as_str()));
        }
        url
    }
}

impl Default for Network {
    /// The system's proxy.
    fn default() -> Self {
        Self::with_system(Box::new(Matcher::from_system), Box::new(environment_settings))
    }
}

impl std::fmt::Debug for Network {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state.read();
        f.debug_struct("Network").field("plan", &state.plan).field("platform", &state.platform.as_ref().map(|(p, _)| p.server.as_str().to_owned())).finish()
    }
}

impl Network {
    /// A network that follows the system's proxy.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// A network with `plan` in force.
    pub fn with_plan(plan: ProxyPlan) -> Arc<Self> {
        let network = Self::default();
        network.set(plan);
        Arc::new(network)
    }

    /// Tests: a network whose system proxy settings are `system` instead of this machine's.
    #[doc(hidden)]
    pub fn with_system_matcher(system: impl Fn() -> Matcher + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self::with_system(Box::new(system), Box::new(String::new)))
    }

    /// Tests: [`Network::with_system_matcher`] with the raw settings the matcher is made from
    /// (what the environment variables would hold), as the fingerprint reads them.
    #[doc(hidden)]
    pub fn with_system_settings(system: impl Fn() -> (Matcher, String) + Send + Sync + 'static) -> Arc<Self> {
        let system = Arc::new(system);
        let raw = Arc::clone(&system);
        Arc::new(Self::with_system(Box::new(move || system().0), Box::new(move || raw().1)))
    }

    fn with_system(system: SystemSource, system_raw: SystemRaw) -> Self {
        Self { state: RwLock::new(State::default()), system, system_raw }
    }

    /// Put `plan` in force: the next request and the next connection follow it.
    pub fn set(&self, plan: ProxyPlan) {
        let manual = match &plan {
            ProxyPlan::Manual(proxy) => Some(matcher(proxy.server(), proxy.no_proxy())),
            ProxyPlan::System | ProxyPlan::Off => None,
        };
        tracing::info!(proxy = %describe(&plan), "proxy plan");
        let mut state = self.state.write();
        state.plan = plan;
        state.manual = manual;
    }

    /// The plan in force.
    pub fn plan(&self) -> ProxyPlan {
        self.state.read().plan.clone()
    }

    /// The proxy of the platform's network (Android's `LinkProperties.httpProxy`; `None`: it has
    /// none), which following the system means there. The shell hands it in at start and whenever
    /// the network changes.
    pub fn set_platform_proxy(&self, proxy: Option<PlatformProxy>) {
        let entry = proxy.map(|proxy| {
            let matcher = matcher(&proxy.server, &proxy.no_proxy);
            (proxy, matcher)
        });
        self.state.write().platform = entry;
    }

    /// The platform network's proxy the shell handed in.
    pub fn platform_proxy(&self) -> Option<PlatformProxy> {
        self.state.read().platform.as_ref().map(|(proxy, _)| proxy.clone())
    }

    /// Where a request to `url` goes now: `None` = direct, else the proxy's address with its
    /// credentials in it. `ws://` and `wss://` count as `http://` and `https://`.
    pub fn proxy_for(&self, url: &Url) -> Option<Url> {
        self.route_for(url).map(|route| route.url())
    }

    /// What following the system means for `url` now, whatever the plan, without credentials:
    /// for the settings to show next to 「跟随系统」. `None`: direct.
    pub fn system_proxy_for(&self, url: &Url) -> Option<Url> {
        let uri = target_uri(url)?;
        let state = self.state.read();
        self.system_route(&state, &uri).map(|route| route.server)
    }

    /// [`Network::system_proxy_for`] an `https://` host no exclusion list names (a name under the
    /// reserved `.invalid`, RFC 2606): the proxy the settings show next to 「跟随系统」.
    pub fn system_https_proxy(&self) -> Option<Url> {
        self.system_proxy_for(&Url::parse("https://proxy-probe.voltip.invalid/").ok()?)
    }

    /// A digest of everything following the system means now: the system's proxies for HTTP and
    /// HTTPS with their credentials and its exclusions, or the platform network's proxy and its
    /// exclusions. Equal digests: no connection would take another route. For telling whether
    /// the system's settings changed; never shown or logged. It reads the settings themselves, not
    /// the route of some address, so an exclusion that happens to name that address hides nothing.
    pub fn system_fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        let state = self.state.read();
        match &state.platform {
            Some((proxy, _)) => {
                "platform".hash(&mut hasher);
                proxy.server.as_str().hash(&mut hasher);
                proxy.no_proxy.hash(&mut hasher);
            }
            None => {
                // Both proxies and every exclusion rule, from wherever the reader took them; the
                // credentials, which the matcher's Debug leaves out, from the raw settings.
                format!("{:?}", (self.system)()).hash(&mut hasher);
                (self.system_raw)().hash(&mut hasher);
            }
        }
        hasher.finish()
    }

    pub(crate) fn route_for(&self, url: &Url) -> Option<Route> {
        let uri = target_uri(url)?;
        let state = self.state.read();
        match &state.plan {
            ProxyPlan::Off => None,
            ProxyPlan::Manual(proxy) => {
                state.manual.as_ref()?.intercept(&uri)?;
                let credentials = proxy.username().map(|user| (user.to_owned(), proxy.password().unwrap_or_default().to_owned()));
                Some(Route { server: proxy.server().clone(), credentials })
            }
            ProxyPlan::System => self.system_route(&state, &uri),
        }
    }

    fn system_route(&self, state: &State, uri: &http::Uri) -> Option<Route> {
        match &state.platform {
            Some((proxy, matcher)) => matcher.intercept(uri).map(|_| Route { server: proxy.server.clone(), credentials: None }),
            None => (self.system)().intercept(uri).and_then(|intercept| route_of(&intercept)),
        }
    }

    /// `builder` with a proxy that asks this network on every request ([`Network::proxy_for`]).
    /// It replaces reqwest's own reading of the environment.
    pub fn apply(self: &Arc<Self>, builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
        let network = Arc::clone(self);
        builder.proxy(reqwest::Proxy::custom(move |url| network.proxy_for(url)))
    }

    /// A TCP stream to the host and port of `url` (`ws://`, `wss://` and the like), direct or
    /// through the proxy [`Network::proxy_for`] picks: HTTP `CONNECT`, or SOCKS5. A WebSocket
    /// handshake runs on it, with TLS first for `wss`. `timeout` bounds the whole of it.
    pub async fn connect(&self, url: &Url, timeout: Duration) -> Result<TcpStream, ConnectError> {
        let host = url.host().ok_or(ConnectError::NoHost)?;
        let port = url.port_or_known_default().ok_or(ConnectError::NoHost)?;
        let route = self.route_for(url);
        tokio::time::timeout(timeout, connect::open(route, host, port)).await.map_err(|_| ConnectError::Timeout(timeout))?
    }
}

/// A network as a client's configuration holds it: equal when it is the same network. The default
/// is one network per process that follows the system, for a configuration nobody handed one
/// (tools, tests); the apps hand theirs in.
#[derive(Clone)]
pub struct NetworkRef(Arc<Network>);

impl NetworkRef {
    /// Hold `network`.
    pub fn new(network: &Arc<Network>) -> Self {
        Self(Arc::clone(network))
    }

    /// The network.
    pub fn get(&self) -> &Arc<Network> {
        &self.0
    }
}

impl Default for NetworkRef {
    fn default() -> Self {
        static SYSTEM: std::sync::OnceLock<Arc<Network>> = std::sync::OnceLock::new();
        Self(Arc::clone(SYSTEM.get_or_init(Network::new)))
    }
}

impl PartialEq for NetworkRef {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for NetworkRef {}

impl std::fmt::Debug for NetworkRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0.plan().describe())
    }
}

impl std::ops::Deref for NetworkRef {
    type Target = Arc<Network>;

    fn deref(&self) -> &Arc<Network> {
        &self.0
    }
}

impl ProxyPlan {
    /// The plan for the log: the kind of proxy and whether it signs in, never its address (no host
    /// goes into a log line, AGENTS.md) nor its password.
    pub fn describe(&self) -> String {
        describe(self)
    }
}

fn describe(plan: &ProxyPlan) -> String {
    match plan {
        ProxyPlan::System => "system".into(),
        ProxyPlan::Off => "off".into(),
        ProxyPlan::Manual(proxy) => {
            let signs_in = if proxy.username().is_some() { " (signs in)" } else { "" };
            format!("manual {}{signs_in}", proxy.server().scheme())
        }
    }
}

/// A matcher sending everything to `server` but the hosts of `no_proxy`.
fn matcher(server: &Url, no_proxy: &str) -> Matcher {
    Matcher::builder().all(server.as_str().to_owned()).no(no_proxy.to_owned()).build()
}

/// The request target as hyper-util's matcher reads it (`scheme://host:port`), or `None` for a
/// local host, which goes direct whatever the plan, and for a URL that is no HTTP request or
/// WebSocket connection.
fn target_uri(url: &Url) -> Option<http::Uri> {
    let host = url.host()?;
    if is_local(&host) {
        return None;
    }
    let scheme = match url.scheme() {
        "http" | "ws" => "http",
        "https" | "wss" => "https",
        _ => return None,
    };
    let port = url.port_or_known_default()?;
    let authority = match host {
        Host::Ipv6(addr) => format!("[{addr}]:{port}"),
        host => format!("{host}:{port}"),
    };
    format!("{scheme}://{authority}").parse().ok()
}

/// This device: `localhost` (and `*.localhost`), the loopback and unspecified addresses.
fn is_local(host: &Host<&str>) -> bool {
    match host {
        Host::Domain(domain) => {
            let domain = domain.trim_end_matches('.').to_ascii_lowercase();
            domain == "localhost" || domain.ends_with(".localhost")
        }
        Host::Ipv4(addr) => addr.is_loopback() || addr.is_unspecified(),
        Host::Ipv6(addr) => addr.is_loopback() || addr.is_unspecified() || addr.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback()),
    }
}

/// A system proxy as hyper-util's matcher answers it: its address, and the credentials it carried
/// (a Basic header for an HTTP proxy, the raw pair for SOCKS).
fn route_of(intercept: &Intercept) -> Option<Route> {
    let uri = intercept.uri();
    let server = Url::parse(&format!("{}://{}", uri.scheme_str()?, uri.authority()?.as_str())).ok()?;
    let credentials = match (intercept.raw_auth(), intercept.basic_auth()) {
        (Some((user, pass)), _) => Some((user.to_owned(), pass.to_owned())),
        (None, Some(header)) => {
            let encoded = header.to_str().ok()?.strip_prefix("Basic ")?;
            let decoded = String::from_utf8(base64::engine::general_purpose::STANDARD.decode(encoded).ok()?).ok()?;
            let (user, pass) = decoded.split_once(':').unwrap_or((decoded.as_str(), ""));
            Some((user.to_owned(), pass.to_owned()))
        }
        (None, None) => None,
    };
    Some(Route { server, credentials })
}

#[cfg(test)]
mod tests;
