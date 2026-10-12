//! The commands that connect an AI tool to the service (docs/dictation.md §25), as 复制命令 puts
//! them on the clipboard with the real token. The settings show the same commands with a
//! placeholder (`packages/shared/src/mcp.ts` builds them the same way; the IPC fixture
//! `mcp-setup.json` keeps the two in step). Checked against the tools' own documentation:
//! `claude mcp add --transport http <name> <url> --header` (code.claude.com/docs/en/mcp) and
//! `codex mcp add <name> --url <url> --bearer-token-env-var <var>` (openai/codex
//! `codex-rs/cli/src/mcp_cmd.rs`).

use serde::{Deserialize, Serialize};

/// The name the tools register the service under.
pub const SERVER_NAME: &str = "voltip";
/// The environment variable Codex reads the token from.
pub const TOKEN_ENV_VAR: &str = "VOLTIP_MCP_TOKEN";

/// The AI tools 接入 shows a command for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupClient {
    /// Claude Code: the token goes in a header the command stores.
    ClaudeCode,
    /// Codex: the token comes from an environment variable.
    Codex,
}

/// The command that connects `client` to `address` with `token`. On Windows the environment
/// variable is set the PowerShell way.
pub fn setup_command(client: SetupClient, address: &str, token: &str, windows: bool) -> String {
    match client {
        SetupClient::ClaudeCode => format!("claude mcp add --transport http {SERVER_NAME} {address} --header \"Authorization: Bearer {token}\""),
        SetupClient::Codex => {
            let set = if windows { format!("$env:{TOKEN_ENV_VAR} = \"{token}\"") } else { format!("export {TOKEN_ENV_VAR}={token}") };
            format!("{set}\ncodex mcp add {SERVER_NAME} --url {address} --bearer-token-env-var {TOKEN_ENV_VAR}")
        }
    }
}
