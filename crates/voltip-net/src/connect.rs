//! A TCP stream for a WebSocket connection, direct or through the proxy: HTTP `CONNECT` or SOCKS5,
//! by hyper-util's connectors (the ones reqwest tunnels with). A direct connection to a name tries
//! its addresses the way RFC 8305 §5 does: one after another, the next as soon as the previous
//! fails or [`ATTEMPT_DELAY`] passes without an answer, the first to connect kept.

use std::future::{Future, poll_fn};
use std::net::SocketAddr;
use std::time::Duration;

use base64::Engine as _;
use http::HeaderValue;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::connect::proxy::{SocksV5, Tunnel};
use tokio::net::TcpStream;
use tower_service::Service;
use url::Host;

use crate::{Resolver, Route};

/// How long an address may leave a connection attempt unanswered before the next address of the
/// name is tried beside it (RFC 8305 §5, "Connection Attempt Delay"). An address that never
/// answers then costs this much instead of the whole deadline (issue #193: one of the relay's three
/// addresses could not be reached from mainland China, and a phone that tried it first spent all
/// of its 8 s there, attempt after attempt, without ever trying the other two).
pub const ATTEMPT_DELAY: Duration = Duration::from_millis(250);

/// Why no stream could be opened. The messages name neither the proxy's host nor its credentials
/// (they reach log lines, AGENTS.md).
#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    /// The URL has no host or port.
    #[error("the address has no host or port")]
    NoHost,
    /// Nothing within the deadline.
    #[error("no connection within {0:?}")]
    Timeout(Duration),
    /// The host's name did not resolve to an address.
    #[error("resolve: {0}")]
    Resolve(#[source] std::io::Error),
    /// The direct connection failed: the error of the last address tried.
    #[error("connect: {0}")]
    Direct(#[source] std::io::Error),
    /// The proxy refused or failed the tunnel (the reason as the connector gives it).
    #[error("proxy: {0}")]
    Proxy(String),
    /// The system's proxy is of a kind the connections cannot tunnel through (`https://`): its
    /// scheme.
    #[error("{0} proxies are not supported for realtime connections (http, socks5 and socks5h are)")]
    Unsupported(String),
}

/// Open the stream to `host:port`: direct when `route` is `None` ([`direct`]); a name resolves
/// through `resolver` when a test set one, else the system's resolver.
pub(crate) async fn open(route: Option<Route>, host: Host<&str>, port: u16, resolver: Option<Resolver>) -> Result<TcpStream, ConnectError> {
    let Some(route) = route else {
        let stream = direct(host, port, resolver).await?;
        let _ = stream.set_nodelay(true);
        return Ok(stream);
    };
    let failed = ConnectError::Proxy;
    let scheme = route.server.scheme().to_owned();
    let proxy_uri: http::Uri = format!("{scheme}://{}", authority(&route.server)).parse().map_err(|e: http::uri::InvalidUri| failed(e.to_string()))?;
    let target_host = match host {
        Host::Ipv6(addr) => format!("[{addr}]"),
        host => host.to_string(),
    };
    let target: http::Uri = format!("http://{target_host}:{port}").parse().map_err(|e: http::uri::InvalidUri| failed(e.to_string()))?;
    let mut inner = HttpConnector::new();
    // The proxy's own URI may be `socks5://…`: the connector only dials it.
    inner.enforce_http(false);
    inner.set_nodelay(true);
    let io = match scheme.as_str() {
        "http" => {
            let mut tunnel = Tunnel::new(proxy_uri, inner);
            if let Some((user, pass)) = &route.credentials {
                tunnel = tunnel.with_auth(basic(user, pass).map_err(failed)?);
            }
            poll_fn(|cx| tunnel.poll_ready(cx)).await.map_err(|e| failed(chain(&e)))?;
            tunnel.call(target).await.map_err(|e| failed(chain(&e)))?
        }
        "socks5" | "socks5h" => {
            // socks5: this device resolves the name; socks5h: the proxy does.
            let mut socks = SocksV5::new(proxy_uri, inner).local_dns(scheme == "socks5");
            if let Some((user, pass)) = &route.credentials {
                socks = socks.with_auth(user.clone(), pass.clone());
            }
            poll_fn(|cx| socks.poll_ready(cx)).await.map_err(|e| failed(chain(&e)))?;
            socks.call(target).await.map_err(|e| failed(chain(&e)))?
        }
        _ => return Err(ConnectError::Unsupported(scheme)),
    };
    Ok(io.into_inner())
}

/// The direct stream to `host:port`: an address as it is, a name's addresses [`race`]d in the
/// order RFC 8305 §4 gives them ([`interleave`]).
async fn direct(host: Host<&str>, port: u16, resolver: Option<Resolver>) -> Result<TcpStream, ConnectError> {
    let addrs = match host {
        Host::Ipv4(addr) => vec![SocketAddr::from((addr, port))],
        Host::Ipv6(addr) => vec![SocketAddr::from((addr, port))],
        Host::Domain(domain) => match resolver {
            Some(resolve) => resolve(domain, port),
            None => tokio::net::lookup_host((domain, port)).await.map(Iterator::collect),
        }
        .map_err(ConnectError::Resolve)?,
    };
    if addrs.is_empty() {
        return Err(ConnectError::Resolve(std::io::Error::new(std::io::ErrorKind::NotFound, "the name has no addresses")));
    }
    race(interleave(addrs), ATTEMPT_DELAY, TcpStream::connect).await.map_err(ConnectError::Direct)
}

/// `addrs` with the address families taking turns, the first address's family first (RFC 8305
/// §4): a family whose addresses all fail costs one attempt at a time, not all of them in a row.
pub(crate) fn interleave(addrs: Vec<SocketAddr>) -> Vec<SocketAddr> {
    let Some(first) = addrs.first().copied() else { return addrs };
    let (mut same, mut other): (std::collections::VecDeque<_>, std::collections::VecDeque<_>) =
        addrs.into_iter().partition(|addr| addr.is_ipv4() == first.is_ipv4());
    let mut ordered = Vec::with_capacity(same.len() + other.len());
    while let Some(addr) = same.pop_front() {
        ordered.push(addr);
        if let Some(addr) = other.pop_front() {
            ordered.push(addr);
        }
    }
    ordered.extend(other);
    ordered
}

/// Connect to one of `addrs` (RFC 8305 §5): the first attempt at once, the next as soon as one
/// fails or `delay` passes without any connecting, every attempt left running until one connects.
/// The first stream that connects, the others dropped; when every attempt failed, the last error.
/// The caller's deadline bounds the whole of it (an attempt the network drops never fails here).
pub(crate) async fn race<C, F>(addrs: Vec<SocketAddr>, delay: Duration, connect: C) -> std::io::Result<TcpStream>
where
    C: Fn(SocketAddr) -> F,
    F: Future<Output = std::io::Result<TcpStream>> + Send + 'static,
{
    let mut waiting = addrs.into_iter();
    // Dropping the set aborts the attempts still running: the race is over, or abandoned.
    let mut running = tokio::task::JoinSet::new();
    let mut last_error = None;
    loop {
        if running.is_empty() {
            let Some(addr) = waiting.next() else {
                return Err(last_error.unwrap_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no address to connect to")));
            };
            running.spawn(connect(addr));
        }
        let more = waiting.len() > 0;
        tokio::select! {
            Some(done) = running.join_next() => match done {
                Ok(Ok(stream)) => return Ok(stream),
                // A failure starts the next address at once.
                Ok(Err(error)) => {
                    last_error = Some(error);
                    if let Some(addr) = waiting.next() {
                        running.spawn(connect(addr));
                    }
                }
                Err(join) => last_error = Some(std::io::Error::other(join)),
            },
            () = tokio::time::sleep(delay), if more => {
                if let Some(addr) = waiting.next() {
                    running.spawn(connect(addr));
                }
            }
        }
    }
}

/// `host:port` of a server address, IPv6 in brackets.
fn authority(server: &url::Url) -> String {
    let port = server.port_or_known_default().unwrap_or_default();
    match server.host() {
        Some(Host::Ipv6(addr)) => format!("[{addr}]:{port}"),
        Some(host) => format!("{host}:{port}"),
        None => String::new(),
    }
}

/// `Proxy-Authorization: Basic …`, marked sensitive so no log shows it.
fn basic(user: &str, pass: &str) -> Result<HeaderValue, String> {
    let token = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}"));
    let mut value = HeaderValue::from_str(&format!("Basic {token}")).map_err(|e| e.to_string())?;
    value.set_sensitive(true);
    Ok(value)
}

/// An error and its sources, one line.
fn chain(error: &dyn std::error::Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(inner) = source {
        let text = inner.to_string();
        if !message.contains(&text) {
            message.push_str(": ");
            message.push_str(&text);
        }
        source = inner.source();
    }
    message
}
