//! The local MCP service (docs/dictation.md §25): the AI tools on this computer (Claude Code,
//! Codex) read the history, the dictionary and the rules over the Model Context Protocol, and add
//! dictionary entries and rules, as far as the user allows it in the settings.
//!
//! Everything here is the service's logic; nothing opens a socket. The HTTP layer (Streamable
//! HTTP, the bearer token, `Host` and `Origin`, the limits) lives in the `voltip-mcp` crate behind
//! the [`McpService`] and [`McpHost`] ports, as `voltip-serve` does for the local speech service.
//! Every call goes through the running core, the one writer of the lists: a write is checked and
//! applied by the same store methods as the interface's commands, and shows up in the interface
//! through the same events.
//!
//! - [`McpSettings`]: on or off, the port, what may be read ([`McpRead`], [`HistoryRange`]) and
//!   what may be changed ([`McpWrite`]); off by default.
//! - [`access`]: which level each tool needs, and the refusal an agent can pass on to the user.
//! - [`tools`]: the tools and the prompt as the protocol lists them, and their arguments.
//! - [`activity`]: the recent activity the settings show (`<data dir>/mcp/activity.json`, names
//!   and counts only, never text) and what undoing a write puts back.
//! - [`candidates`]: `vocabulary_candidates`, the pairs the AI clean-up changed.
//! - [`reads`] and [`writes`]: what each tool does; [`records`]: what the reads return.

pub mod access;
pub mod activity;
pub mod candidates;
mod host;
pub mod reads;
pub mod records;
mod service;
pub mod setup;
pub mod tools;
pub mod writes;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use activity::{ActivityLog, McpAction, McpActivity};
pub use host::{McpHost, McpListenConfig, McpService, RunningMcp, ToolError};
pub(crate) use service::{CoreMcp, McpCall, McpDone, McpInbound};
pub use setup::{SetupClient, setup_command};
pub use tools::{PromptSpec, ToolSpec};

/// The port the service listens on unless the user picks another (the local speech service's is
/// 47840, docs/dictation.md §23).
pub const DEFAULT_PORT: u16 = 47841;
/// The path the protocol is served on.
pub const MCP_PATH: &str = "/mcp";
/// Longest client name kept (`clientInfo.name`, cleaned), in characters.
pub const MAX_CLIENT_CHARS: usize = 64;
/// What a client is called when it names nobody.
pub const UNNAMED_CLIENT: &str = "MCP";
/// Most sessions open at the same time.
pub const MAX_SESSIONS: usize = 4;
/// Most calls one session makes in a second.
pub const MAX_CALLS_PER_SECOND: u32 = 10;
/// Most history entries one call returns.
pub const MAX_HISTORY_PAGE: usize = 50;
/// Largest result of one call, in bytes of JSON.
pub const MAX_RESULT_BYTES: usize = 256 * 1024;
/// Largest request body the HTTP layer reads.
pub const MAX_REQUEST_BYTES: usize = 256 * 1024;
/// Directory of the service's files inside the data directory.
pub const MCP_DIR: &str = "mcp";
/// The token's file name (its own token: a client of the local speech service cannot read the
/// history with that one).
pub const TOKEN_FILE: &str = "token";
/// The recent activity's file name.
pub const ACTIVITY_FILE: &str = "activity.json";

/// `<data dir>/mcp`.
pub fn mcp_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(MCP_DIR)
}

/// `<data dir>/mcp/token`.
pub fn token_path(data_dir: &Path) -> PathBuf {
    mcp_dir(data_dir).join(TOKEN_FILE)
}

/// `http://127.0.0.1:<port>/mcp`: what a client is told to connect to.
pub fn address(port: u16) -> String {
    format!("http://127.0.0.1:{port}{MCP_PATH}")
}

/// What may be read (`Settings.mcp.read`); each level includes the ones before it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpRead {
    /// The dictionary and the rules with their hits; previews; backtests give counts only.
    Rules,
    /// Also the pairs the AI clean-up changed, how often, and short snippets around them; never a
    /// whole entry (the default).
    #[default]
    Summary,
    /// Also every entry of [`McpSettings::history_range`]: its texts, application, scene, models and
    /// how it ended.
    Full,
}

/// How far back [`McpRead::Full`] reaches (`Settings.mcp.history_range`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HistoryRange {
    /// The last seven days.
    #[serde(rename = "last_7_days")]
    Last7Days,
    /// The last thirty days (the default).
    #[default]
    #[serde(rename = "last_30_days")]
    Last30Days,
    /// Every entry.
    #[serde(rename = "all")]
    All,
}

impl HistoryRange {
    /// Wire name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Last7Days => "last_7_days",
            Self::Last30Days => "last_30_days",
            Self::All => "all",
        }
    }

    /// The earliest instant a client may read at `now_ms` (Unix milliseconds); `None` for every entry.
    pub fn start(self, now_ms: u64) -> Option<u64> {
        const DAY_MS: u64 = 24 * 60 * 60 * 1000;
        match self {
            Self::Last7Days => Some(now_ms.saturating_sub(7 * DAY_MS)),
            Self::Last30Days => Some(now_ms.saturating_sub(30 * DAY_MS)),
            Self::All => None,
        }
    }
}

/// What may be changed (`Settings.mcp.write`); each level includes the ones before it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpWrite {
    /// Nothing.
    ReadOnly,
    /// Add and change dictionary entries (switching them off too); create rules that stay off and
    /// wait for the user's approval, and change only the waiting rules the same tool created (the
    /// default).
    #[default]
    Dictionary,
    /// Also create rules that are on at once, and change any rule.
    EnableRules,
}

/// `Settings.mcp`: whether the app serves the AI tools on this computer, on which port, and what
/// they may read and change. Off by default, and in settings files written before it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct McpSettings {
    /// Listen (only on 127.0.0.1).
    pub enabled: bool,
    /// The port.
    pub port: u16,
    /// What may be read.
    pub read: McpRead,
    /// How far back the full history reaches (only used with [`McpRead::Full`]).
    pub history_range: HistoryRange,
    /// What may be changed.
    pub write: McpWrite,
}

impl Default for McpSettings {
    fn default() -> Self {
        Self { enabled: false, port: DEFAULT_PORT, read: McpRead::Summary, history_range: HistoryRange::Last30Days, write: McpWrite::Dictionary }
    }
}

/// A client's name as the activity and the lists show it: `clientInfo.name` trimmed, control
/// characters dropped, runs of white space made one space, at most [`MAX_CLIENT_CHARS`]
/// characters; [`UNNAMED_CLIENT`] when nothing is left.
pub fn clean_client_name(name: &str) -> String {
    let words: Vec<String> = name
        .split(char::is_whitespace)
        .map(|word| word.chars().filter(|c| !c.is_control() && !is_format_control(*c)).collect::<String>())
        .filter(|word| !word.is_empty())
        .collect();
    let cut: String = words.join(" ").chars().take(MAX_CLIENT_CHARS).collect();
    let cut = cut.trim_end();
    if cut.is_empty() { UNNAMED_CLIENT.to_owned() } else { cut.to_owned() }
}

/// Invisible characters that change how text around them is shown (bidirectional overrides,
/// zero-width joiners): a client name could otherwise pose as another one.
fn is_format_control(c: char) -> bool {
    matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2069}' | '\u{feff}')
}

/// A client that called the service since it started, and when it was last heard from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpClient {
    /// Its cleaned name.
    pub name: String,
    /// Unix milliseconds of its last request.
    pub last_seen_ms: u64,
}

/// Most clients `McpStatus.clients` lists (the most recent ones).
pub const MAX_LISTED_CLIENTS: usize = 8;

/// `UiState.mcp`: whether this app can host the service, what it is doing, who used it and the
/// recent activity.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpStatus {
    /// This app can host the service (the desktop).
    pub available: bool,
    /// Off, starting, running or failed (the local speech service's phases).
    pub phase: crate::ui::ServePhase,
    /// What a client connects to (`http://127.0.0.1:<port>/mcp`) while running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// Why the service is not running although switched on (the port is taken).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The clients heard from since the service started, most recent first.
    #[serde(default)]
    pub clients: Vec<McpClient>,
    /// The recent activity, newest first ([`activity::MAX_ACTIVITY`] at most).
    #[serde(default)]
    pub activity: Vec<McpActivity>,
}

/// Why an MCP command is refused where the shell cannot host the service (the phone).
pub const MCP_UNAVAILABLE: &str = "mcp: AI 工具接入仅在电脑上提供";
