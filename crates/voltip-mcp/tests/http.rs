#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The MCP service end to end (docs/dictation.md §25): the real core task (on the in-memory
//! dictation fakes) hosting the service through [`HttpHost`] on a real socket, and rmcp's own
//! client talking to it over Streamable HTTP as Claude Code and Codex do — plus raw HTTP for what a
//! well-behaved client never sends: no token, a wrong one, a foreign `Host`, a web page's
//! `Origin`, a burst of calls, a fifth session.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rmcp::model::{CallToolRequestParams, CallToolResult, ClientCapabilities, ClientConfig, GetPromptRequestParams, Implementation};
use rmcp::service::RunningService;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::{RoleClient, ServiceExt as _};
use serde_json::{Map, Value, json};
use tokio::sync::mpsc;
use voltip_core::mcp::{McpAction, McpRead, McpSettings, McpWrite};
use voltip_core::ui::ServePhase;
use voltip_core::{AppCore, CoreCommand, CoreConfig, CoreEvent, CoreHandle, DictionaryEntry, EntrySource, ReplacementRule, Settings, SettingsStore};
use voltip_identity::MemorySecretStore;
use voltip_mcp::HttpHost;

const STEP: Duration = Duration::from_secs(10);

type Client = RunningService<RoleClient, ClientConfig>;

struct Node {
    handle: CoreHandle,
    events: mpsc::Receiver<CoreEvent>,
    /// `http://127.0.0.1:<port>/mcp`.
    url: String,
    port: u16,
    token: String,
    _dir: tempfile::TempDir,
}

impl Node {
    async fn wait<T>(&mut self, mut pick: impl FnMut(&CoreEvent) -> Option<T>) -> T {
        loop {
            let ev = tokio::time::timeout(STEP, self.events.recv()).await.expect("event within 10 s").expect("core alive");
            if let Some(v) = pick(&ev) {
                return v;
            }
        }
    }

    async fn set(&mut self, mcp: McpSettings) {
        let want = mcp.clone();
        self.handle.send(CoreCommand::SetMcp(mcp)).await.unwrap();
        self.wait(|e| if let CoreEvent::Settings(s) = e { (s.mcp == want).then_some(()) } else { None }).await;
    }

    fn settings(&self) -> McpSettings {
        McpSettings { enabled: true, port: self.port, ..McpSettings::default() }
    }

    async fn dictionary(&mut self) -> Vec<DictionaryEntry> {
        self.wait(|e| if let CoreEvent::Dictionary(d) = e { Some(d.clone()) } else { None }).await
    }

    async fn rules(&mut self) -> Vec<ReplacementRule> {
        self.wait(|e| if let CoreEvent::Rules(r) = e { Some(r.clone()) } else { None }).await
    }
}

/// A core with the service on a free port of 127.0.0.1 (a settings file may name port 0; the
/// interface never saves one), running.
async fn start() -> Node {
    let dir = tempfile::tempdir().unwrap();
    start_in(dir).await
}

async fn start_in(dir: tempfile::TempDir) -> Node {
    let path: &Path = dir.path();
    SettingsStore::new(path)
        .save(&Settings {
            relay_enabled: false,
            engines: voltip_core::dictation::fakes::fake_engines(),
            mcp: McpSettings { enabled: true, port: 0, ..McpSettings::default() },
            ..Settings::default()
        })
        .unwrap();
    let mut config = CoreConfig::new(path.to_path_buf());
    config.mcp_host = Some(Arc::new(HttpHost));
    let (handle, events) = AppCore::start(config, Arc::new(MemorySecretStore::new())).unwrap();
    let mut node = Node { handle, events, url: String::new(), port: 0, token: String::new(), _dir: dir };
    let address = node
        .wait(|e| match e {
            CoreEvent::Mcp(s) if s.phase == ServePhase::Running => s.address.clone(),
            CoreEvent::Mcp(s) if s.phase == ServePhase::Failed => panic!("the service did not start: {:?}", s.error),
            _ => None,
        })
        .await;
    node.port = address.trim_start_matches("http://127.0.0.1:").trim_end_matches("/mcp").parse().unwrap();
    node.url = address;
    node.token = std::fs::read_to_string(node._dir.path().join("mcp/token")).unwrap().trim().to_owned();
    node
}

/// A client as an AI tool connects: rmcp's Streamable HTTP client with the token (direct: the
/// machine's proxy settings never apply to this computer's own address).
async fn connect(node: &Node, name: &str) -> Result<Client, Box<rmcp::service::ClientInitializeError>> {
    let config = StreamableHttpClientTransportConfig::with_uri(node.url.as_str()).auth_header(node.token.clone());
    let transport = StreamableHttpClientTransport::with_client(http(), config);
    ClientConfig::new(ClientCapabilities::default(), Implementation::new(name, "1.0")).serve(transport).await.map_err(Box::new)
}

fn args(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        other => panic!("not an object: {other}"),
    }
}

async fn call(client: &Client, tool: &str, arguments: Value) -> CallToolResult {
    client.call_tool(CallToolRequestParams::new(tool.to_owned()).with_arguments(args(arguments))).await.unwrap()
}

/// The structured result of a call that worked.
async fn ok(client: &Client, tool: &str, arguments: Value) -> Value {
    let result = call(client, tool, arguments).await;
    assert_eq!(result.is_error, Some(false), "{tool}: {result:?}");
    result.structured_content.unwrap()
}

/// The text of a call that was refused or failed.
async fn refused(client: &Client, tool: &str, arguments: Value) -> String {
    let result = call(client, tool, arguments).await;
    assert_eq!(result.is_error, Some(true), "{tool}: {result:?}");
    result.content[0].as_text().unwrap().text.clone()
}

fn http() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().pool_max_idle_per_host(0).redirect(reqwest::redirect::Policy::none()).build().unwrap()
}

/// A JSON-RPC request as a client posts it.
fn post(node: &Node, body: &Value) -> reqwest::RequestBuilder {
    http().post(&node.url).header("accept", "application/json, text/event-stream").header("content-type", "application/json").body(body.to_string())
}

fn initialize_body() -> Value {
    json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "curl", "version": "1" } } })
}

#[tokio::test]
async fn a_real_client_lists_the_tools_and_the_prompt_and_calls_them() {
    let mut node = start().await;
    let client = connect(&node, "claude-code").await.unwrap();
    let info = client.peer_info().unwrap();
    assert_eq!((info.server_info.as_ref().map(|s| s.name.as_str()), info.protocol_version.as_str()), (Some("voltip"), "2025-11-25"));
    assert!(info.instructions.as_deref().unwrap().contains("review_misrecognitions"));
    let tools = client.list_all_tools().await.unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert_eq!(names.len(), 11);
    for (name, read_only) in
        [("dictionary_list", true), ("history_search", true), ("vocabulary_backtest", true), ("dictionary_add", false), ("rules_update", false)]
    {
        let tool = tools.iter().find(|t| t.name == name).unwrap_or_else(|| panic!("{name}"));
        let hints = tool.annotations.as_ref().unwrap();
        assert_eq!((hints.read_only_hint, hints.destructive_hint, hints.open_world_hint), (Some(read_only), Some(false), Some(false)), "{name}");
    }
    assert!(!names.iter().any(|n| n.contains("delete") || n.contains("remove")), "no deleting tool");
    let prompts = client.list_all_prompts().await.unwrap();
    assert_eq!(prompts.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["review_misrecognitions"]);
    let prompt = client.get_prompt(GetPromptRequestParams::new("review_misrecognitions").with_arguments(args(json!({ "days": "14" })))).await.unwrap();
    let text = serde_json::to_string(&prompt.messages).unwrap();
    assert!(text.contains("last 14 days") && text.contains("vocabulary_candidates"), "{text}");
    // A write: the entry is the client's, through the same store and event as the interface's.
    let added = ok(&client, "dictionary_add", json!({ "term": "Voltip", "heard_as": ["维他命"] })).await;
    assert_eq!(added["result"], "created");
    let list = node.dictionary().await;
    assert_eq!(list[0].source, EntrySource::Mcp { client: "claude-code".into() });
    let listed = ok(&client, "dictionary_list", json!({})).await;
    assert_eq!(listed["entries"][0]["source"], json!({ "kind": "mcp", "client": "claude-code" }));
    let preview = ok(&client, "vocabulary_preview", json!({ "text": "用维他命开会" })).await;
    assert_eq!(preview["corrected"], "用Voltip开会");
    // Arguments that do not fit, and a tool that is not there.
    assert!(refused(&client, "dictionary_list", json!({ "page": 2 })).await.contains("unknown field"));
    let unknown = client.call_tool(CallToolRequestParams::new("dictionary_delete")).await.unwrap_err();
    assert!(unknown.to_string().contains("dictionary_delete"), "{unknown}");
    // The client shows up in the status.
    let status = node.wait(|e| if let CoreEvent::Mcp(s) = e { (!s.activity.is_empty()).then(|| s.clone()) } else { None }).await;
    assert_eq!(status.clients[0].name, "claude-code");
    client.cancel().await.unwrap();
}

/// What a web page or another program without the token can do: nothing but `/healthz`, and not
/// even that from a web page. No response carries a CORS header.
#[tokio::test]
async fn requests_without_the_token_or_from_a_web_page_are_refused() {
    let node = start().await;
    let checks: Vec<(&str, reqwest::RequestBuilder, u16)> = vec![
        ("no token", post(&node, &initialize_body()), 401),
        ("a wrong token", post(&node, &initialize_body()).bearer_auth("not-the-token"), 401),
        ("the local speech service's kind of header", post(&node, &initialize_body()).header("authorization", format!("Basic {}", node.token)), 401),
        ("a foreign Host", post(&node, &initialize_body()).bearer_auth(&node.token).header("host", "evil.example"), 403),
        (
            "a rebinding Host with the right port",
            post(&node, &initialize_body()).bearer_auth(&node.token).header("host", format!("evil.example:{}", node.port)),
            403,
        ),
        ("a web page", post(&node, &initialize_body()).bearer_auth(&node.token).header("origin", "http://evil.example"), 403),
        ("a sandboxed page", post(&node, &initialize_body()).bearer_auth(&node.token).header("origin", "null"), 403),
        ("a page on this computer", post(&node, &initialize_body()).bearer_auth(&node.token).header("origin", format!("http://127.0.0.1:{}", node.port)), 403),
        (
            "a preflight",
            http().request(reqwest::Method::OPTIONS, &node.url).header("origin", "http://evil.example").header("access-control-request-method", "POST"),
            403,
        ),
        ("health from a page", http().get(format!("http://127.0.0.1:{}/healthz", node.port)).header("origin", "http://evil.example"), 403),
    ];
    for (what, request, status) in checks {
        let response = request.send().await.unwrap();
        assert_eq!(response.status().as_u16(), status, "{what}");
        assert!(!response.headers().keys().any(|k| k.as_str().starts_with("access-control-")), "{what}: no CORS header");
        if status == 401 {
            assert_eq!(response.headers()["www-authenticate"], "Bearer", "{what}");
        }
        let body: Value = response.json().await.unwrap();
        assert!(body["error"]["message"].as_str().unwrap().contains('\n'), "{what}: a refusal in both languages: {body}");
    }
    let health = http().get(format!("http://127.0.0.1:{}/healthz", node.port)).send().await.unwrap();
    assert_eq!(health.status(), 200, "the health check needs no token");
    assert_eq!(health.json::<Value>().await.unwrap(), json!({ "status": "ok" }));
    let localhost = post(&node, &initialize_body()).bearer_auth(&node.token).header("host", format!("localhost:{}", node.port)).send().await.unwrap();
    assert_eq!(localhost.status(), 200, "localhost:<port> names this computer too");
    assert!(!localhost.headers().keys().any(|k| k.as_str().starts_with("access-control-")));
}

/// docs/dictation.md §25: the read and write levels of the moment decide; taking one back applies
/// to the very next call of a session that is already open.
#[tokio::test]
async fn the_levels_of_the_moment_decide_each_call() {
    let mut node = start().await;
    let client = connect(&node, "codex").await.unwrap();
    let text = refused(&client, "history_search", json!({})).await;
    assert!(text.starts_with("当前只允许读取替换汇总；完整的历史记录需要在 Voltip 设置中授权。"), "{text}");
    assert!(text.contains("Voltip currently shares the correction summary only"), "{text}");
    let base = node.settings();
    node.set(McpSettings { read: McpRead::Full, ..base.clone() }).await;
    let page = ok(&client, "history_search", json!({})).await;
    assert_eq!((page["entries"].clone(), page["range"].clone()), (json!([]), json!("last_30_days")));
    node.set(McpSettings { read: McpRead::Rules, ..base.clone() }).await;
    assert!(refused(&client, "history_search", json!({})).await.contains("完整的历史记录"));
    assert!(refused(&client, "vocabulary_candidates", json!({})).await.contains("替换汇总"));
    assert!(ok(&client, "rules_list", json!({})).await["rules"].as_array().unwrap().is_empty());
    node.set(McpSettings { write: McpWrite::ReadOnly, ..base.clone() }).await;
    assert!(refused(&client, "dictionary_add", json!({ "term": "x" })).await.contains("只允许读取"));
    assert!(refused(&client, "rules_add", json!({ "name": "x", "pattern": "x" })).await.contains("只允许读取"));
    client.cancel().await.unwrap();
}

/// A rule the client creates waits for the user's approval and does not run; the user's undo in
/// the settings takes back what a client wrote.
#[tokio::test]
async fn a_clients_rule_waits_and_its_writes_can_be_undone() {
    let mut node = start().await;
    let client = connect(&node, "claude-code").await.unwrap();
    let created = ok(&client, "rules_add", json!({ "name": "飞书", "pattern": "非书", "replacement": "飞书" })).await;
    assert_eq!(created["pending"], true);
    let rules = node.rules().await;
    assert_eq!((rules[0].pending, rules[0].enabled), (true, false));
    let preview = ok(&client, "vocabulary_preview", json!({ "text": "发到非书群" })).await;
    assert_eq!(preview["output"], "发到非书群", "a waiting rule does not run");
    let listed = ok(&client, "rules_list", json!({})).await;
    assert_eq!((listed["rules"][0]["pending"].clone(), listed["rules"][0]["source"]["client"].clone()), (json!(true), json!("claude-code")));
    let status = node
        .wait(|e| match e {
            CoreEvent::Mcp(s) => s.activity.iter().find(|a| a.action == McpAction::AddRule).cloned(),
            _ => None,
        })
        .await;
    assert!(status.pending && status.undoable);
    assert_eq!(created["activity_id"], status.id.to_string());
    node.handle.send(CoreCommand::McpUndo(status.id)).await.unwrap();
    assert!(node.rules().await.is_empty(), "undone");
    client.cancel().await.unwrap();
}

/// At most four sessions; a session that closes frees its place. Each session makes at most ten
/// calls a second.
#[tokio::test]
async fn sessions_and_calls_are_limited() {
    let node = start().await;
    let mut clients = Vec::new();
    for i in 0..4 {
        clients.push(connect(&node, &format!("tool {i}")).await.unwrap());
    }
    let refused = post(&node, &initialize_body()).bearer_auth(&node.token).send().await.unwrap();
    assert_eq!(refused.status(), 503, "a fifth session");
    assert_eq!(refused.headers()["retry-after"], "1");
    assert!(refused.json::<Value>().await.unwrap()["error"]["message"].as_str().unwrap().contains("同时最多 4 个会话"));
    assert!(connect(&node, "tool 5").await.is_err());
    // Closing a session (rmcp's client deletes it) frees its place.
    clients.pop().unwrap().cancel().await.unwrap();
    let fifth = connect(&node, "tool 5").await.unwrap();
    // A burst of calls in one session: past ten in a second they are refused.
    let session = "burst-session";
    let burst = (0..20).map(|i| {
        post(&node, &json!({ "jsonrpc": "2.0", "id": i, "method": "tools/list" }))
            .bearer_auth(&node.token)
            .header("mcp-session-id", session)
            .header("mcp-protocol-version", "2025-06-18")
            .send()
    });
    let statuses: Vec<u16> = futures_statuses(burst).await;
    let limited = statuses.iter().filter(|s| **s == 429).count();
    assert!(limited >= 1, "{statuses:?}");
    assert!(statuses.iter().all(|s| *s == 429 || *s == 404), "an unknown session is the protocol's 404: {statuses:?}");
    fifth.cancel().await.unwrap();
    for client in clients {
        client.cancel().await.unwrap();
    }
}

async fn futures_statuses(requests: impl Iterator<Item = impl std::future::Future<Output = reqwest::Result<reqwest::Response>> + Send + 'static>) -> Vec<u16> {
    let tasks: Vec<_> = requests.map(tokio::spawn).collect();
    let mut out = Vec::new();
    for task in tasks {
        out.push(task.await.unwrap().unwrap().status().as_u16());
    }
    out
}

/// A new token is in force at once; switching the service off ends every session and frees the
/// port.
#[tokio::test]
async fn a_new_token_and_switching_off_take_effect_at_once() {
    let mut node = start().await;
    let client = connect(&node, "claude-code").await.unwrap();
    node.handle.send(CoreCommand::McpRotateToken).await.unwrap();
    let path = node._dir.path().join("mcp/token");
    let started = std::time::Instant::now();
    let rotated = loop {
        let now = std::fs::read_to_string(&path).unwrap().trim().to_owned();
        if now != node.token {
            break now;
        }
        assert!(started.elapsed() < STEP, "the token was not replaced");
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    // The command is applied before the file is renamed into place returns to the core; the old
    // token is refused from the next request on.
    let mut old = post(&node, &initialize_body()).bearer_auth(&node.token).send().await.unwrap().status().as_u16();
    while old != 401 {
        assert!(started.elapsed() < STEP, "the old token still works");
        tokio::time::sleep(Duration::from_millis(5)).await;
        old = post(&node, &initialize_body()).bearer_auth(&node.token).send().await.unwrap().status().as_u16();
    }
    assert_eq!(post(&node, &initialize_body()).bearer_auth(&rotated).send().await.unwrap().status(), 200);
    assert!(client.list_all_tools().await.is_err(), "the open session's client still sends the old token");
    let base = node.settings();
    node.handle.send(CoreCommand::SetMcp(McpSettings { enabled: false, ..base })).await.unwrap();
    node.wait(|e| matches!(e, CoreEvent::Mcp(s) if s.phase == ServePhase::Off).then_some(())).await;
    let started = std::time::Instant::now();
    while tokio::net::TcpStream::connect(("127.0.0.1", node.port)).await.is_ok() {
        assert!(started.elapsed() < STEP, "the port was not freed");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    drop(client);
}

/// Captures every log line of the process, for the test that checks they hold no text.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// docs/dictation.md §25: the logs name the tool, the client, the count and the time — never a
/// text the client sent or got, at the app's log levels with Voltip's own crates at the most
/// detailed one.
#[tokio::test]
async fn the_logs_hold_no_text() {
    let captured = Captured::default();
    let filter = tracing_subscriber::EnvFilter::new("info,voltip=trace,voltip_core=trace,voltip_mcp=trace");
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(captured.clone()).with_ansi(false).try_init().unwrap();
    let mut node = start().await;
    let client = connect(&node, "claude-code").await.unwrap();
    node.set(McpSettings { read: McpRead::Full, write: McpWrite::EnableRules, ..node.settings() }).await;
    const MARK: &str = "秘密标记Zq7";
    ok(&client, "dictionary_add", json!({ "term": format!("{MARK}词"), "heard_as": [format!("{MARK}错")] })).await;
    ok(&client, "rules_add", json!({ "name": format!("{MARK}规则"), "pattern": format!("{MARK}找"), "replacement": format!("{MARK}换") })).await;
    ok(&client, "vocabulary_preview", json!({ "text": format!("这里有{MARK}错和{MARK}找") })).await;
    ok(&client, "history_search", json!({ "query": MARK })).await;
    refused(&client, "dictionary_add", json!({ "term": format!("{MARK}词"), "heard_as": [format!("{MARK}规则")], "extra": MARK })).await;
    ok(&client, "dictionary_list", json!({})).await;
    client.cancel().await.unwrap();
    let logs = String::from_utf8(captured.0.lock().clone()).unwrap();
    assert!(logs.contains("MCP write") && logs.contains("dictionary_add"), "the calls are logged by name");
    assert!(!logs.contains(MARK), "a text reached the log:\n{logs}");
    assert!(!logs.contains(&node.token), "the token reached the log");
}
