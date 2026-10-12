//! A TCP stream for a WebSocket connection, direct or through the proxy: HTTP `CONNECT` or SOCKS5,
//! by hyper-util's connectors (the ones reqwest tunnels with).

use std::future::poll_fn;
use std::time::Duration;

use base64::Engine as _;
use http::HeaderValue;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::connect::proxy::{SocksV5, Tunnel};
use tokio::net::TcpStream;
use tower_service::Service;
use url::Host;

use crate::Route;

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
    /// The direct connection failed.
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

/// Open the stream to `host:port`: direct when `route` is `None`.
pub(crate) async fn open(route: Option<Route>, host: Host<&str>, port: u16) -> Result<TcpStream, ConnectError> {
    let Some(route) = route else {
        let stream = match host {
            Host::Domain(domain) => TcpStream::connect((domain, port)).await,
            Host::Ipv4(addr) => TcpStream::connect((addr, port)).await,
            Host::Ipv6(addr) => TcpStream::connect((addr, port)).await,
        }
        .map_err(ConnectError::Direct)?;
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
