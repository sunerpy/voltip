//! The core's [`McpService`]: a call's arguments are parsed here, then the call goes to the core
//! task, which checks it against the settings of that moment, runs it on its own lists (a read of
//! the history on a blocking thread) and answers. Nothing is answered from a copy of the lists or
//! the settings held here, so a change in the settings applies to the very next call.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Map, Value};
use tokio::sync::{mpsc, oneshot};

use super::activity::ActivityDraft;
use super::tools::{self, McpRequest, PromptSpec, ToolSpec};
use super::{McpService, ToolError, clean_client_name};

/// How long a call waits for the core's answer.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(60);

/// A call on its way to the core task.
#[derive(Debug)]
pub(crate) struct McpCall {
    /// The client's cleaned name.
    pub client: String,
    /// What it asks.
    pub request: McpRequest,
    /// Where the answer goes.
    pub reply: oneshot::Sender<Result<Value, ToolError>>,
}

/// A read the core ran on a blocking thread, back to be noted and answered.
#[derive(Debug)]
pub(crate) struct McpDone {
    /// The client's cleaned name.
    pub client: String,
    /// The result, and what to note in the activity.
    pub result: Result<(Value, ActivityDraft), ToolError>,
    /// Where the answer goes.
    pub reply: oneshot::Sender<Result<Value, ToolError>>,
}

/// What reaches the core task from the service.
#[derive(Debug)]
pub(crate) enum McpInbound {
    /// A call.
    Call(McpCall),
    /// A client finished `initialize`.
    Connected {
        /// Its cleaned name.
        client: String,
    },
    /// A read that ran off the core task finished.
    Done(Box<McpDone>),
}

/// The service the core hands its host.
#[derive(Clone, Debug)]
pub(crate) struct CoreMcp {
    tx: mpsc::Sender<McpInbound>,
}

impl CoreMcp {
    /// Calls go to `tx` (the core task's inbox).
    pub(crate) fn new(tx: mpsc::Sender<McpInbound>) -> Self {
        Self { tx }
    }
}

fn closing() -> ToolError {
    ToolError::Unavailable(super::access::bilingual("Voltip 的 AI 工具接入已关闭。", "Voltip's AI tool access is off."))
}

#[async_trait]
impl McpService for CoreMcp {
    fn tools(&self) -> Vec<ToolSpec> {
        tools::specs()
    }

    fn prompts(&self) -> Vec<PromptSpec> {
        tools::prompts()
    }

    fn prompt(&self, name: &str, arguments: &Map<String, Value>) -> Result<String, ToolError> {
        tools::prompt_text(name, arguments)
    }

    fn connected(&self, client: &str) {
        if self.tx.try_send(McpInbound::Connected { client: clean_client_name(client) }).is_err() {
            tracing::debug!("the core did not take an MCP connection notice");
        }
    }

    async fn call(&self, client: &str, tool: &str, arguments: Map<String, Value>) -> Result<Value, ToolError> {
        let request = tools::parse_call(tool, arguments)?;
        let (reply, answer) = oneshot::channel();
        self.tx.send(McpInbound::Call(McpCall { client: clean_client_name(client), request, reply })).await.map_err(|_| closing())?;
        match tokio::time::timeout(CALL_TIMEOUT, answer).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(closing()),
            Err(_) => {
                Err(ToolError::Unavailable(super::access::bilingual("Voltip 没有及时回答，请稍后重试。", "Voltip did not answer in time; try again later.")))
            }
        }
    }
}
