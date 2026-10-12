//! The protocol's side of the service (docs/dictation.md §25): `initialize` names the client to the
//! core, `tools/list` and `prompts/list` give the core's tools and prompt, `tools/call` and
//! `prompts/get` run them. A tool that ran and failed or was refused answers with a tool error
//! whose text the agent reads and can pass on (`isError`); only a tool that does not exist is a
//! protocol error.
//!
//! Only the protocol versions with an `initialize` handshake are offered (up to 2025-11-25): every
//! client then has a session and a name, which the session limit and the activity rely on. A
//! client that probes with `server/discover` falls back to `initialize`.

use std::borrow::Cow;
use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, GetPromptRequestParams, GetPromptResponse, GetPromptResult, Implementation,
    InitializeRequestParams, InitializeResult, ListPromptsResult, ListToolsResult, PaginatedRequestParams, Prompt, PromptArgument, PromptMessage,
    ProtocolVersion, Role, ServerCapabilities, ServerConfig, Tool, ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};
use voltip_core::mcp::{McpService, PromptSpec, ToolError, ToolSpec};

/// What `initialize` tells a client about the service.
const INSTRUCTIONS: &str = "Voltip is the user's voice input app. These tools read Voltip's personal dictionary, replacement rules and dictation history, and add dictionary entries and rules, as far as the user allowed in Voltip's settings. A refused call says what the user would have to allow; tell the user. Ask the user before writing anything. Start with the review_misrecognitions prompt to find what the speech recognition keeps getting wrong.";

/// One session's handler: the core's service, and nothing of its own.
#[derive(Clone)]
pub struct VoltipHandler {
    service: Arc<dyn McpService>,
}

impl std::fmt::Debug for VoltipHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VoltipHandler")
    }
}

impl VoltipHandler {
    /// A handler for `service`.
    pub fn new(service: Arc<dyn McpService>) -> Self {
        Self { service }
    }
}

/// A core tool as the protocol describes it.
pub fn tool(spec: &ToolSpec) -> Tool {
    let schema = match &spec.input_schema {
        serde_json::Value::Object(map) => map.clone(),
        _ => serde_json::Map::new(),
    };
    Tool::new(spec.name, spec.description, Arc::new(schema))
        .with_title(spec.title)
        .with_annotations(ToolAnnotations::with_title(spec.title).read_only(spec.read_only).destructive(false).idempotent(spec.idempotent).open_world(false))
}

/// A core prompt as the protocol describes it.
pub fn prompt(spec: &PromptSpec) -> Prompt {
    let arguments = spec.arguments.iter().map(|a| PromptArgument::new(a.name).with_description(a.description).with_required(a.required)).collect();
    Prompt::new(spec.name, Some(spec.description), Some(arguments)).with_title(spec.title)
}

/// The name the client gave at `initialize`.
fn client_of(context: &RequestContext<RoleServer>) -> String {
    context.peer.peer_info().map(|info| info.client_info.name.clone()).unwrap_or_default()
}

impl ServerHandler for VoltipHandler {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_prompts().enable_tools().build())
            .with_protocol_version(ProtocolVersion::V_2025_11_25)
            .with_server_info(Implementation::new("voltip", env!("CARGO_PKG_VERSION")).with_title("Voltip"))
            .with_instructions(INSTRUCTIONS)
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(ProtocolVersion::known_up_to(&ProtocolVersion::V_2025_11_25))
    }

    async fn initialize(&self, request: InitializeRequestParams, context: RequestContext<RoleServer>) -> Result<InitializeResult, ErrorData> {
        context.peer.set_peer_info(request.clone());
        self.service.connected(&request.client_info.name);
        self.negotiate_initialize(&request)
    }

    async fn list_tools(&self, _request: Option<PaginatedRequestParams>, _context: RequestContext<RoleServer>) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(self.service.tools().iter().map(tool).collect()))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.service.tools().iter().find(|t| t.name == name).map(tool)
    }

    async fn call_tool(&self, request: CallToolRequestParams, context: RequestContext<RoleServer>) -> Result<CallToolResponse, ErrorData> {
        let client = client_of(&context);
        match self.service.call(&client, &request.name, request.arguments.unwrap_or_default()).await {
            Ok(value) => Ok(CallToolResult::structured(value).into()),
            Err(ToolError::Unknown(message)) => Err(ErrorData::invalid_params(message, None)),
            Err(error) => Ok(CallToolResult::error(vec![ContentBlock::text(error.to_string())]).into()),
        }
    }

    async fn list_prompts(&self, _request: Option<PaginatedRequestParams>, _context: RequestContext<RoleServer>) -> Result<ListPromptsResult, ErrorData> {
        Ok(ListPromptsResult::with_all_items(self.service.prompts().iter().map(prompt).collect()))
    }

    async fn get_prompt(&self, request: GetPromptRequestParams, _context: RequestContext<RoleServer>) -> Result<GetPromptResponse, ErrorData> {
        let arguments = request.arguments.unwrap_or_default();
        match self.service.prompt(&request.name, &arguments) {
            Ok(text) => {
                let description = self.service.prompts().iter().find(|p| p.name == request.name).map(|p| p.description.to_owned()).unwrap_or_default();
                Ok(GetPromptResult::new(vec![PromptMessage::new_text(Role::User, text)]).with_description(description).into())
            }
            Err(error) => Err(ErrorData::invalid_params(error.to_string(), None)),
        }
    }
}
