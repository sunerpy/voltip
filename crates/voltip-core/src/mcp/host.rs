//! The ports between the MCP service and what serves it over HTTP (docs/dictation.md §25):
//! [`McpService`] is what the HTTP layer calls (the core implements it and answers every call in
//! its task; tests plug in their own), [`McpHost`] is how the app starts and stops a listener for
//! it without the core knowing about sockets — the shape of `serve::host`.

use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Map, Value};

use super::tools::{PromptSpec, ToolSpec};

/// Why a call was not answered with a result. The text is what the agent reads: it names the
/// problem plainly, and a refusal says what the user would have to allow.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ToolError {
    /// No tool or prompt has that name (a protocol error).
    #[error("{0}")]
    Unknown(String),
    /// The arguments do not fit the tool's schema.
    #[error("{0}")]
    InvalidArguments(String),
    /// The settings do not allow it.
    #[error("{0}")]
    Denied(String),
    /// The tool ran and the change or the read failed (a draft the dictionary refuses, an entry
    /// that is gone).
    #[error("{0}")]
    Failed(String),
    /// The service is stopping or the app is closing.
    #[error("{0}")]
    Unavailable(String),
}

/// What the HTTP layer asks of the service.
#[async_trait]
pub trait McpService: Send + Sync {
    /// Every tool, whatever the settings allow (a call outside them is refused with a reason, so
    /// a change of the settings never changes the list).
    fn tools(&self) -> Vec<ToolSpec>;
    /// Every prompt.
    fn prompts(&self) -> Vec<PromptSpec>;
    /// The text of prompt `name` with `arguments`.
    fn prompt(&self, name: &str, arguments: &Map<String, Value>) -> Result<String, ToolError>;
    /// A client finished `initialize` under `client` (its cleaned name).
    fn connected(&self, client: &str);
    /// Run tool `tool` for `client` with `arguments`; the result is the tool's structured content.
    async fn call(&self, client: &str, tool: &str, arguments: Map<String, Value>) -> Result<Value, ToolError>;
}

/// How a listener is set up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpListenConfig {
    /// Where it listens (127.0.0.1 and the port; port 0 picks a free one, for tests).
    pub addr: SocketAddr,
    /// The bearer token every request but `/healthz` must carry.
    pub token: String,
    /// Most sessions open at the same time.
    pub max_sessions: usize,
    /// Most calls one session makes in a second.
    pub calls_per_second: u32,
    /// Largest request body.
    pub max_request_bytes: usize,
}

/// A listener that is running.
pub trait RunningMcp: Send + Sync {
    /// The address it is bound to.
    fn addr(&self) -> SocketAddr;
    /// Requests from now on must carry `token`.
    fn set_token(&self, token: String);
    /// Stop listening and end every session.
    fn stop(&self);
}

/// Starts listeners (the desktop shell implements it with the HTTP layer; the phone has none).
#[async_trait]
pub trait McpHost: Send + Sync + std::fmt::Debug {
    /// Bind and serve `service` with `config`; the reason (the port is taken) when it cannot.
    async fn start(&self, config: McpListenConfig, service: Arc<dyn McpService>) -> Result<Box<dyn RunningMcp>, String>;
}
