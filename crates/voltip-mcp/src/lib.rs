//! Voltip's local MCP service over HTTP (docs/dictation.md §25): the Model Context Protocol's
//! Streamable HTTP transport on `http://127.0.0.1:<port>/mcp`, for the AI tools on this computer
//! (Claude Code, Codex). The protocol is the official Rust SDK's (`rmcp`); what the tools do is
//! the core's ([`voltip_core::mcp`], behind [`McpService`]). This crate is the adapter:
//!
//! - [`guard`]: every request must name this computer in `Host` and carry no `Origin` (a web page
//!   cannot reach the service through DNS rebinding, and gets no CORS headers either); every
//!   request but `/healthz` carries the bearer token; at most `max_sessions` sessions, each at
//!   most `calls_per_second` calls a second.
//! - [`handler`]: the protocol's tools and prompts mapped onto the core's.
//! - [`HttpHost`]: the core's [`McpHost`] port, what the desktop app starts the service with.
//!
//! Logs name the method, the status and the time; the core logs the tool, the client and the
//! count. No text of a request or a result is ever logged.

pub mod guard;
pub mod handler;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::routing::get;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tokio_util::sync::CancellationToken;
use voltip_core::mcp::{MCP_PATH, McpHost, McpListenConfig, McpService, RunningMcp};

pub use guard::{Guard, Token};
pub use handler::VoltipHandler;

/// The routes of one listener on `port`: `/mcp` behind the guard, `/healthz` behind its `Host`
/// and `Origin` checks only. `cancel` ends every session.
pub fn router(config: &McpListenConfig, port: u16, service: Arc<dyn McpService>, cancel: CancellationToken) -> (Router, Guard) {
    let sessions = Arc::new(LocalSessionManager::default());
    let allowed = guard::allowed_hosts(port);
    let rmcp_config = StreamableHttpServerConfig::default()
        .with_allowed_hosts(allowed.clone())
        .enforce_origin_validation()
        .with_max_request_body_bytes(config.max_request_bytes)
        .with_cancellation_token(cancel);
    let factory_service = service.clone();
    let mcp = StreamableHttpService::new(move || Ok(VoltipHandler::new(factory_service.clone())), sessions.clone(), rmcp_config);
    let guard = Guard::new(Token::new(config.token.clone()), allowed, sessions, config.max_sessions, config.calls_per_second);
    let router =
        Router::new().route_service(MCP_PATH, mcp).route("/healthz", get(healthz)).layer(axum::middleware::from_fn_with_state(guard.clone(), guard::check));
    (router, guard)
}

async fn healthz() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({ "status": "ok" }))
}

/// How long a port in use is tried again: a listener this app just stopped (the service switched
/// off and on, a port changed back) lets go of it a moment after `stop`.
const BIND_RETRY: Duration = Duration::from_secs(2);

/// Bind `addr`, trying again for [`BIND_RETRY`] while the port is in use.
async fn bind(addr: SocketAddr) -> std::io::Result<tokio::net::TcpListener> {
    let started = tokio::time::Instant::now();
    loop {
        match tokio::net::TcpListener::bind(addr).await {
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse && started.elapsed() < BIND_RETRY => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            other => return other,
        }
    }
}

/// A listener the desktop app started ([`HttpHost`]).
struct Running {
    addr: SocketAddr,
    guard: Guard,
    cancel: CancellationToken,
}

impl RunningMcp for Running {
    fn addr(&self) -> SocketAddr {
        self.addr
    }

    fn set_token(&self, token: String) {
        self.guard.token().set(token);
    }

    fn stop(&self) {
        self.cancel.cancel();
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// The core's [`McpHost`] over this crate: what the desktop app starts its MCP service with.
#[derive(Clone, Copy, Debug, Default)]
pub struct HttpHost;

#[async_trait]
impl McpHost for HttpHost {
    async fn start(&self, config: McpListenConfig, service: Arc<dyn McpService>) -> Result<Box<dyn RunningMcp>, String> {
        let listener = bind(config.addr).await.map_err(|e| format!("无法监听 {}：{e}", config.addr))?;
        let addr = listener.local_addr().map_err(|e| format!("无法监听 {}：{e}", config.addr))?;
        let cancel = CancellationToken::new();
        let (router, guard) = router(&config, addr.port(), service, cancel.clone());
        let shutdown = cancel.clone();
        tokio::spawn(async move {
            let served = axum::serve(listener, router).with_graceful_shutdown(async move { shutdown.cancelled().await }).await;
            if let Err(e) = served {
                tracing::warn!(error = %e, "the MCP service stopped with an error");
            }
            tracing::info!("MCP service stopped");
        });
        tracing::info!(%addr, "MCP service listening");
        Ok(Box::new(Running { addr, guard, cancel }))
    }
}
