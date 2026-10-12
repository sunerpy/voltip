#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The local MCP service the app hosts (docs/dictation.md §25), through the real core task with a
//! fake host that hands the test the service: switched on and off by `SetMcp`, its own port and
//! token, every read gated by the read level of that moment (taking it back applies to the next
//! call), writes through the same stores and events as the interface's commands, rules that wait
//! for approval and run nowhere until the user turns them on, the recent activity with its undo,
//! the token and the setup commands on the clipboard, and a shell without a host that refuses it.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::{Map, Value, json};
use tokio::sync::mpsc;
use uuid::Uuid;
use voltip_core::dictation::fakes::{FAKE_TRANSCRIPT, FakeAudio, FakeInjector, FakeRefiner, FakeTranscriber};
use voltip_core::dictation::{DictationPhase, DictationPorts, Refiner, Transcriber};
use voltip_core::mcp::{
    HistoryRange, MCP_UNAVAILABLE, McpAction, McpHost, McpListenConfig, McpRead, McpService, McpSettings, McpStatus, McpWrite, RunningMcp, SetupClient,
    ToolError,
};
use voltip_core::ui::ServePhase;
use voltip_core::{
    AppCore, CoreCommand, CoreConfig, CoreEvent, CoreHandle, DictionaryEntry, EntrySource, HistoryEntry, HistoryStore, ReplacementRule, RuleDraft, RuleKind,
    Settings, SettingsStore,
};
use voltip_identity::MemorySecretStore;

const STEP: Duration = Duration::from_secs(10);
const DAY_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Default)]
struct Started {
    configs: Vec<McpListenConfig>,
    services: Vec<Arc<dyn McpService>>,
    tokens: Vec<String>,
    stops: usize,
}

impl std::fmt::Debug for Started {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Started").field("configs", &self.configs.len()).finish_non_exhaustive()
    }
}

/// A host that binds nothing: it records what it was asked, hands out the service, and can
/// refuse (a taken port).
#[derive(Debug, Default)]
struct FakeHost {
    log: Arc<Mutex<Started>>,
    refuse: Arc<Mutex<bool>>,
}

struct FakeRunning {
    addr: SocketAddr,
    log: Arc<Mutex<Started>>,
}

impl RunningMcp for FakeRunning {
    fn addr(&self) -> SocketAddr {
        self.addr
    }

    fn set_token(&self, token: String) {
        self.log.lock().tokens.push(token);
    }

    fn stop(&self) {
        self.log.lock().stops += 1;
    }
}

#[async_trait]
impl McpHost for FakeHost {
    async fn start(&self, config: McpListenConfig, service: Arc<dyn McpService>) -> Result<Box<dyn RunningMcp>, String> {
        if *self.refuse.lock() {
            return Err(format!("无法监听 {}：Address already in use", config.addr));
        }
        let addr = config.addr;
        let mut log = self.log.lock();
        log.configs.push(config);
        log.services.push(service);
        Ok(Box::new(FakeRunning { addr, log: self.log.clone() }))
    }
}

impl FakeHost {
    fn service(&self) -> Arc<dyn McpService> {
        self.log.lock().services.last().cloned().expect("a listener was started")
    }
}

struct Node {
    handle: CoreHandle,
    events: mpsc::Receiver<CoreEvent>,
    injector: Arc<FakeInjector>,
}

fn start(dir: &Path, host: Option<Arc<FakeHost>>) -> Node {
    if !dir.join(voltip_core::SETTINGS_FILE_NAME).exists() {
        SettingsStore::new(dir)
            .save(&Settings { relay_enabled: false, engines: voltip_core::dictation::fakes::fake_engines(), ..Settings::default() })
            .unwrap();
    }
    let mut cfg = CoreConfig::new(dir.to_path_buf());
    cfg.default_device_name = "MCP Test".into();
    cfg.mcp_host = host.map(|h| h as Arc<dyn McpHost>);
    let injector = Arc::new(FakeInjector::paste());
    let ports = DictationPorts {
        audio: Arc::new(FakeAudio::speech()),
        injector: injector.clone(),
        factory: Arc::new(|_| {
            (
                Arc::new(FakeTranscriber::ok(FAKE_TRANSCRIPT)) as Arc<dyn Transcriber>,
                Some(Arc::new(FakeRefiner::ok("你好，世界。整理好了。")) as Arc<dyn Refiner>),
            )
        }),
        models: None,
        streaming: None,
        probe: None,
        service_probe: None,
        segmenter: None,
    };
    let (handle, events) = AppCore::start_with(cfg, Arc::new(MemorySecretStore::new()), ports).unwrap();
    Node { handle, events, injector }
}

async fn wait<T>(node: &mut Node, mut pick: impl FnMut(&CoreEvent) -> Option<T>) -> T {
    loop {
        let ev = tokio::time::timeout(STEP, node.events.recv()).await.expect("event within 10 s").expect("core alive");
        if let Some(v) = pick(&ev) {
            return v;
        }
    }
}

/// The next status whose phase is not `starting`.
async fn mcp_settled(node: &mut Node) -> McpStatus {
    wait(node, |e| match e {
        CoreEvent::Mcp(s) if s.phase != ServePhase::Starting => Some(s.clone()),
        _ => None,
    })
    .await
}

/// The next status with a phase other than `phase`.
async fn mcp_left(node: &mut Node, phase: ServePhase) -> McpStatus {
    wait(node, |e| match e {
        CoreEvent::Mcp(s) if s.phase != phase && s.phase != ServePhase::Starting => Some(s.clone()),
        _ => None,
    })
    .await
}

async fn error(node: &mut Node) -> String {
    wait(node, |e| if let CoreEvent::Error(m) = e { Some(m.clone()) } else { None }).await
}

fn on(port: u16) -> McpSettings {
    McpSettings { enabled: true, port, ..McpSettings::default() }
}

fn args(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        other => panic!("not an object: {other}"),
    }
}

async fn call(service: &Arc<dyn McpService>, client: &str, tool: &str, arguments: Value) -> Result<Value, ToolError> {
    service.call(client, tool, args(arguments)).await
}

fn denied(result: Result<Value, ToolError>) -> String {
    match result {
        Err(ToolError::Denied(text)) => text,
        other => panic!("not refused: {other:?}"),
    }
}

/// An entry of the history as a take writes it.
fn history_entry(raw: &str, text: &str, at_ms: u64) -> HistoryEntry {
    serde_json::from_value(json!({
        "id": Uuid::new_v4(), "at_ms": at_ms, "raw_text": raw, "text": text, "refined": true, "asr_model": "m",
        "duration_ms": 1000, "asr_ms": 1, "outcome": { "kind": "inserted", "via": "paste" },
        "app": { "id": "lark", "name": "飞书" }
    }))
    .unwrap()
}

/// A history with two recent takes and one ten days old.
fn seed_history(dir: &Path) -> (HistoryEntry, HistoryEntry, HistoryEntry) {
    let now = voltip_core::now_ms();
    let old = history_entry("十天前用维他命", "十天前用 Voltip。", now - 10 * DAY_MS);
    let recent = history_entry("发到非书群", "发到飞书群。", now - DAY_MS);
    let newest = history_entry("今天用维他命测了一下", "今天用 Voltip 测了一下。", now - 60_000);
    let mut store = HistoryStore::open(dir);
    for e in [&old, &recent, &newest] {
        store.push(e.clone(), 100).unwrap();
    }
    (old, recent, newest)
}

/// Start with the service on (`settings`), wait for it to run, and return the node, the host and
/// the service.
async fn running(dir: &Path, settings: McpSettings) -> (Node, Arc<FakeHost>, Arc<dyn McpService>) {
    SettingsStore::new(dir)
        .save(&Settings { relay_enabled: false, engines: voltip_core::dictation::fakes::fake_engines(), mcp: settings, ..Settings::default() })
        .unwrap();
    let host = Arc::new(FakeHost::default());
    let mut node = start(dir, Some(host.clone()));
    loop {
        if mcp_settled(&mut node).await.phase == ServePhase::Running {
            break;
        }
    }
    let service = host.service();
    (node, host, service)
}

async fn dictionary(node: &mut Node) -> Vec<DictionaryEntry> {
    wait(node, |e| if let CoreEvent::Dictionary(d) = e { Some(d.clone()) } else { None }).await
}

async fn rules(node: &mut Node) -> Vec<ReplacementRule> {
    wait(node, |e| if let CoreEvent::Rules(r) = e { Some(r.clone()) } else { None }).await
}

/// The status once its activity has `n` lines.
async fn activity(node: &mut Node, n: usize) -> McpStatus {
    wait(node, |e| match e {
        CoreEvent::Mcp(s) if s.activity.len() == n => Some(s.clone()),
        _ => None,
    })
    .await
}

#[tokio::test]
async fn switching_the_service_on_listens_on_its_own_port_with_its_own_token() {
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(FakeHost::default());
    let mut node = start(dir.path(), Some(host.clone()));
    let first = mcp_settled(&mut node).await;
    assert_eq!((first.available, first.phase, first.address.clone()), (true, ServePhase::Off, None), "off at first");
    node.handle.send(CoreCommand::SetMcp(on(48201))).await.unwrap();
    let status = mcp_settled(&mut node).await;
    assert_eq!((status.phase, status.address.as_deref()), (ServePhase::Running, Some("http://127.0.0.1:48201/mcp")));
    let config = host.log.lock().configs[0].clone();
    assert_eq!(config.addr.to_string(), "127.0.0.1:48201", "this computer only");
    assert_eq!((config.max_sessions, config.calls_per_second, config.max_request_bytes), (4, 10, 256 * 1024));
    let token = std::fs::read_to_string(dir.path().join("mcp/token")).unwrap();
    assert_eq!(config.token, token.trim());
    assert!(!dir.path().join("serve/token").exists(), "not the local speech service's token");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(std::fs::metadata(dir.path().join("mcp/token")).unwrap().permissions().mode() & 0o777, 0o600);
    }
    assert!(SettingsStore::new(dir.path()).load().unwrap().mcp.enabled, "persisted");
    // What may be read or written changes without a restart; the port restarts it.
    node.handle.send(CoreCommand::SetMcp(McpSettings { read: McpRead::Rules, ..on(48201) })).await.unwrap();
    wait(&mut node, |e| if let CoreEvent::Settings(s) = e { (s.mcp.read == McpRead::Rules).then_some(()) } else { None }).await;
    assert_eq!(host.log.lock().configs.len(), 1, "no restart");
    node.handle.send(CoreCommand::SetMcp(on(48202))).await.unwrap();
    let moved = wait(&mut node, |e| match e {
        CoreEvent::Mcp(s) if s.address.as_deref() == Some("http://127.0.0.1:48202/mcp") => Some(s.clone()),
        _ => None,
    })
    .await;
    assert_eq!(moved.phase, ServePhase::Running);
    assert_eq!(host.log.lock().stops, 1, "the old listener stopped");
    node.handle.send(CoreCommand::SetMcp(McpSettings { enabled: false, ..on(48202) })).await.unwrap();
    assert_eq!(mcp_left(&mut node, ServePhase::Running).await.phase, ServePhase::Off);
    assert_eq!(host.log.lock().stops, 2);
    // Refusals change nothing.
    node.handle.send(CoreCommand::SetMcp(on(80))).await.unwrap();
    assert!(error(&mut node).await.contains("端口须在 1024–65535 之间"));
    node.handle.send(CoreCommand::SetMcp(on(voltip_core::serve::DEFAULT_PORT))).await.unwrap();
    assert!(error(&mut node).await.contains("已用于本机服务"));
    assert!(!SettingsStore::new(dir.path()).load().unwrap().mcp.enabled);
}

#[tokio::test]
async fn a_taken_port_is_reported_and_the_service_starts_with_the_app() {
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(FakeHost::default());
    *host.refuse.lock() = true;
    let mut node = start(dir.path(), Some(host.clone()));
    mcp_settled(&mut node).await;
    node.handle.send(CoreCommand::SetMcp(on(48203))).await.unwrap();
    let failed = mcp_left(&mut node, ServePhase::Off).await;
    assert_eq!(failed.phase, ServePhase::Failed);
    assert!(failed.error.unwrap().contains("Address already in use"));
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
    drop(node);
    *host.refuse.lock() = false;
    let mut node = start(dir.path(), Some(host.clone()));
    let status = mcp_left(&mut node, ServePhase::Off).await;
    assert_eq!((status.phase, status.address.as_deref()), (ServePhase::Running, Some("http://127.0.0.1:48203/mcp")), "on again with the app");
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
    let started = std::time::Instant::now();
    while host.log.lock().stops == 0 {
        assert!(started.elapsed() < STEP, "the listener was not stopped with the app");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn a_shell_without_a_host_refuses_the_mcp_commands() {
    let dir = tempfile::tempdir().unwrap();
    let mut node = start(dir.path(), None);
    assert_eq!(mcp_settled(&mut node).await, McpStatus::default(), "not available");
    for command in [
        CoreCommand::SetMcp(on(48204)),
        CoreCommand::McpCopyToken,
        CoreCommand::McpRotateToken,
        CoreCommand::McpCopySetup(SetupClient::Codex),
        CoreCommand::McpUndo(Uuid::new_v4()),
    ] {
        node.handle.send(command).await.unwrap();
        assert_eq!(error(&mut node).await, MCP_UNAVAILABLE);
    }
    assert!(!dir.path().join("mcp").exists(), "no token was made");
}

#[tokio::test]
async fn the_token_and_the_setup_commands_go_to_the_clipboard_and_a_new_token_is_in_force_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let (node, host, _) = running(dir.path(), on(48205)).await;
    node.handle.send(CoreCommand::McpCopyToken).await.unwrap();
    node.handle.send(CoreCommand::McpCopySetup(SetupClient::ClaudeCode)).await.unwrap();
    node.handle.send(CoreCommand::McpCopySetup(SetupClient::Codex)).await.unwrap();
    node.handle.send(CoreCommand::McpRotateToken).await.unwrap();
    // Commands run in order: once the rotation reached the host, the copies are done too.
    let started = std::time::Instant::now();
    while host.log.lock().tokens.is_empty() {
        assert!(started.elapsed() < STEP, "the new token never reached the listener");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let token = host.log.lock().configs[0].token.clone();
    let copies = node.injector.clipboard_copies();
    assert_eq!(copies.len(), 3);
    assert_eq!(copies[0], token, "the token, on the clipboard only");
    assert_eq!(copies[1], format!("claude mcp add --transport http voltip http://127.0.0.1:48205/mcp --header \"Authorization: Bearer {token}\""));
    assert!(
        copies[2].contains(&token) && copies[2].contains("codex mcp add voltip --url http://127.0.0.1:48205/mcp --bearer-token-env-var VOLTIP_MCP_TOKEN"),
        "{}",
        copies[2]
    );
    let rotated = host.log.lock().tokens[0].clone();
    assert_ne!(rotated, token);
    assert_eq!(std::fs::read_to_string(dir.path().join("mcp/token")).unwrap().trim(), rotated, "the file holds the new one");
}

/// docs/dictation.md §25 (§6.2 of the design): every read needs its level at the moment of the
/// call; the full history reaches back only as far as allowed; taking a level back applies to the
/// very next call.
#[tokio::test]
async fn reads_follow_the_read_level_of_the_moment() {
    let dir = tempfile::tempdir().unwrap();
    let (old, recent, newest) = seed_history(dir.path());
    let (mut node, _, service) = running(dir.path(), on(48206)).await;
    // The default: the dictionary and the rules, and the correction summary.
    let listed = call(&service, "Claude Code", "dictionary_list", json!({})).await.unwrap();
    assert_eq!((listed["total"].clone(), listed["entries"].clone()), (json!(0), json!([])));
    let summary = call(&service, "Claude Code", "vocabulary_candidates", json!({})).await.unwrap();
    let pairs = summary["pairs"].as_array().unwrap();
    assert!(pairs.iter().any(|p| p["heard"] == "维他命" && p["polished"] == "Voltip" && p["count"] == 2), "{summary}");
    assert!(pairs.iter().all(|p| p["examples"].as_array().unwrap().iter().all(|e| e["snippet"].as_str().unwrap().chars().count() <= 12 + 64 + 12 + 2)));
    let refused = denied(call(&service, "Claude Code", "history_search", json!({})).await);
    assert!(refused.starts_with("当前只允许读取替换汇总；完整的历史记录需要在 Voltip 设置中授权。"), "{refused}");
    // The full history of the last seven days.
    node.handle.send(CoreCommand::SetMcp(McpSettings { read: McpRead::Full, history_range: HistoryRange::Last7Days, ..on(48206) })).await.unwrap();
    wait(&mut node, |e| if let CoreEvent::Settings(s) = e { (s.mcp.read == McpRead::Full).then_some(()) } else { None }).await;
    let page = call(&service, "Claude Code", "history_search", json!({})).await.unwrap();
    let ids: Vec<&str> = page["entries"].as_array().unwrap().iter().map(|e| e["id"].as_str().unwrap()).collect();
    assert_eq!(ids, [newest.id.to_string(), recent.id.to_string()], "newest first, nothing older than seven days");
    assert_eq!((page["range"].clone(), page["next_cursor"].clone()), (json!("last_7_days"), Value::Null));
    assert_eq!(page["entries"][0]["raw_text"], "今天用维他命测了一下");
    assert_eq!(page["entries"][0]["app"]["name"], "飞书");
    let first = call(&service, "Claude Code", "history_search", json!({ "limit": 1 })).await.unwrap();
    let cursor = first["next_cursor"].as_str().unwrap().to_owned();
    let second = call(&service, "Claude Code", "history_search", json!({ "limit": 1, "cursor": cursor })).await.unwrap();
    assert_eq!(second["entries"][0]["id"], recent.id.to_string(), "the cursor goes on where the page ended");
    let found = call(&service, "Claude Code", "history_search", json!({ "query": "飞书群" })).await.unwrap();
    assert_eq!(found["entries"].as_array().unwrap().len(), 1);
    let by_app = call(&service, "Claude Code", "history_search", json!({ "app": "lark", "since": voltip_core::now_ms() - 2 * DAY_MS })).await.unwrap();
    assert_eq!(by_app["entries"].as_array().unwrap().len(), 2);
    let one = call(&service, "Claude Code", "history_get", json!({ "id": newest.id })).await.unwrap();
    assert_eq!(one["text"], "今天用 Voltip 测了一下。");
    let refused = denied(call(&service, "Claude Code", "history_get", json!({ "id": old.id })).await);
    assert!(refused.contains("最近 7 天"), "{refused}");
    // Taking it back: the next call is refused.
    node.handle.send(CoreCommand::SetMcp(McpSettings { read: McpRead::Rules, ..on(48206) })).await.unwrap();
    wait(&mut node, |e| if let CoreEvent::Settings(s) = e { (s.mcp.read == McpRead::Rules).then_some(()) } else { None }).await;
    assert!(denied(call(&service, "Claude Code", "history_get", json!({ "id": newest.id })).await).contains("完整的历史记录"));
    assert!(denied(call(&service, "Claude Code", "vocabulary_candidates", json!({})).await).starts_with("当前只允许读取词典和规则；替换汇总"));
    // A backtest still counts, without examples.
    let tested = call(
        &service,
        "Claude Code",
        "vocabulary_backtest",
        json!({ "draft": { "kind": "dictionary", "entry": { "term": "Voltip", "heard_as": ["维他命"] } } }),
    )
    .await
    .unwrap();
    assert_eq!((tested["scanned"].clone(), tested["matched"].clone(), tested["examples"].clone()), (json!(3), json!(2), json!([])));
    assert!(tested["examples_withheld"].as_str().unwrap().contains("替换汇总"));
    // The activity names the reads and counts, never a text.
    let status = wait(&mut node, |e| match e {
        CoreEvent::Mcp(s) if s.activity.first().is_some_and(|a| a.action == McpAction::Backtest) => Some(s.clone()),
        _ => None,
    })
    .await;
    let actions: Vec<McpAction> = status.activity.iter().map(|a| a.action).collect();
    assert!(actions.contains(&McpAction::SearchHistory) && actions.contains(&McpAction::ReadEntry) && actions.contains(&McpAction::ReadSummary), "{actions:?}");
    let search = status.activity.iter().find(|a| a.action == McpAction::SearchHistory).unwrap();
    assert_eq!(search.range, Some(HistoryRange::Last7Days));
    assert_eq!(status.clients[0].name, "Claude Code");
    let file = std::fs::read_to_string(dir.path().join("mcp/activity.json")).unwrap();
    for text in ["维他命测了", "飞书群", "十天前"] {
        assert!(!file.contains(text), "the activity holds no text of the history: {text}");
    }
}

/// Writes go through the same stores as the interface's commands, show up through the same events,
/// are noted as the tool's, and can be undone while nobody changed them since.
#[tokio::test]
async fn writes_change_the_lists_as_the_interface_would_and_can_be_undone() {
    let dir = tempfile::tempdir().unwrap();
    let (mut node, _, service) = running(dir.path(), on(48207)).await;
    let added = call(&service, "Claude Code", "dictionary_add", json!({ "term": "Voltip", "heard_as": ["维他命"] })).await.unwrap();
    assert_eq!(added["result"], "created");
    let list = dictionary(&mut node).await;
    assert_eq!((list[0].term.as_str(), &list[0].source), ("Voltip", &EntrySource::Mcp { client: "Claude Code".into() }));
    let again = call(&service, "Claude Code", "dictionary_add", json!({ "term": "voltip", "heard_as": ["维他命"] })).await.unwrap();
    assert_eq!(again["result"], "unchanged", "idempotent");
    let merged = call(&service, "Codex", "dictionary_add", json!({ "term": "Voltip", "heard_as": ["沃提普"] })).await.unwrap();
    assert_eq!((merged["result"].clone(), merged["added"].clone()), (json!("merged"), json!(["沃提普"])));
    assert_eq!(dictionary(&mut node).await[0].heard_as, ["维他命", "沃提普"]);
    // The dictionary's own checks apply.
    let clash = call(&service, "Codex", "dictionary_add", json!({ "term": "维生素", "heard_as": ["维他命"] })).await.unwrap_err();
    assert!(matches!(&clash, ToolError::Failed(m) if m.contains("已是「Voltip」的误识别写法")), "{clash:?}");
    let status = activity(&mut node, 2).await;
    let (merge, add) = (&status.activity[0], &status.activity[1]);
    assert_eq!((merge.action, merge.client.as_str(), merge.count, merge.undoable), (McpAction::MergeEntry, "Codex", Some(1), true));
    assert_eq!((add.action, add.subject.as_deref()), (McpAction::AddEntry, Some("Voltip")));
    assert_eq!(merged["activity_id"], merge.id.to_string(), "the result names what the user can undo");
    // Undo the merge: the forms before it come back.
    node.handle.send(CoreCommand::McpUndo(merge.id)).await.unwrap();
    assert_eq!(dictionary(&mut node).await[0].heard_as, ["维他命"]);
    node.handle.send(CoreCommand::McpUndo(merge.id)).await.unwrap();
    assert!(error(&mut node).await.contains("已经撤销"));
    // A change the user made since is never thrown away by an undo.
    let id = list[0].id;
    node.handle
        .send(CoreCommand::DictionaryUpdate {
            id,
            draft: voltip_core::DictionaryDraft { term: "Voltip".into(), heard_as: vec!["维他命".into()], enabled: false },
        })
        .await
        .unwrap();
    dictionary(&mut node).await;
    node.handle.send(CoreCommand::McpUndo(add.id)).await.unwrap();
    assert!(error(&mut node).await.contains("之后又被修改过"));
    let updated = call(&service, "Codex", "dictionary_update", json!({ "id": id, "enabled": true })).await.unwrap();
    assert_eq!(updated["entry"]["enabled"], true);
    assert!(dictionary(&mut node).await[0].enabled);
    let status = wait(&mut node, |e| match e {
        CoreEvent::Mcp(s) if s.activity.first().is_some_and(|a| a.action == McpAction::UpdateEntry) => Some(s.clone()),
        _ => None,
    })
    .await;
    node.handle.send(CoreCommand::McpUndo(status.activity[0].id)).await.unwrap();
    assert!(!dictionary(&mut node).await[0].enabled, "back to the user's off");
    // Read-only: no write at all.
    node.handle.send(CoreCommand::SetMcp(McpSettings { write: McpWrite::ReadOnly, ..on(48207) })).await.unwrap();
    wait(&mut node, |e| if let CoreEvent::Settings(s) = e { (s.mcp.write == McpWrite::ReadOnly).then_some(()) } else { None }).await;
    assert!(denied(call(&service, "Codex", "dictionary_add", json!({ "term": "x" })).await).contains("只允许读取"));
}

async fn take_text(node: &mut Node) -> String {
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    wait(node, |e| matches!(e, CoreEvent::Dictation(s) if matches!(s.phase, DictationPhase::Listening { ready: true, .. })).then_some(())).await;
    node.handle.send(CoreCommand::DictationStop).await.unwrap();
    wait(node, |e| match e {
        CoreEvent::Dictation(s) => match &s.phase {
            DictationPhase::Done { text, .. } => Some(text.clone()),
            _ => None,
        },
        _ => None,
    })
    .await
}

/// A rule an AI tool creates waits for the user's approval and runs nowhere until the user turns
/// it on; below 「也可以直接启用规则」 a tool changes only its own waiting rules.
#[tokio::test]
async fn a_tools_rule_waits_for_approval_and_runs_once_approved() {
    let dir = tempfile::tempdir().unwrap();
    let (mut node, _, service) = running(dir.path(), on(48208)).await;
    assert_eq!(take_text(&mut node).await, "你好，世界。整理好了。");
    let created = call(&service, "Claude Code", "rules_add", json!({ "name": "世界 → World", "pattern": "世界", "replacement": "World" })).await.unwrap();
    assert_eq!((created["pending"].clone(), created["rule"]["enabled"].clone()), (json!(true), json!(false)));
    assert!(created["note"].as_str().unwrap().contains("确认"));
    let id: Uuid = created["id"].as_str().unwrap().parse().unwrap();
    let list = rules(&mut node).await;
    assert_eq!((list[0].pending, list[0].enabled, &list[0].source), (true, false, &EntrySource::Mcp { client: "Claude Code".into() }));
    assert_eq!(take_text(&mut node).await, "你好，世界。整理好了。", "a waiting rule does not run");
    let preview = call(&service, "Claude Code", "vocabulary_preview", json!({ "text": "你好，世界" })).await.unwrap();
    assert_eq!(preview["output"], "你好，世界", "nor in a preview");
    // Its own tool may change it, and it keeps waiting; another tool may not touch it.
    let changed = call(&service, "Claude Code", "rules_update", json!({ "id": id, "replacement": "World!", "enabled": true })).await.unwrap();
    assert_eq!((changed["pending"].clone(), changed["rule"]["replacement"].clone()), (json!(true), json!("World!")));
    assert!(denied(call(&service, "Codex", "rules_update", json!({ "id": id, "replacement": "x" })).await).contains("也可以直接启用规则"));
    rules(&mut node).await;
    // The user approves it on the rules page: it runs from the next take on.
    let draft = RuleDraft {
        name: "世界 → World".into(),
        kind: RuleKind::Literal,
        pattern: "世界".into(),
        replacement: "World!".into(),
        case_sensitive: true,
        enabled: true,
    };
    node.handle.send(CoreCommand::RuleUpdate { id, draft }).await.unwrap();
    let list = rules(&mut node).await;
    assert_eq!((list[0].pending, list[0].enabled), (false, true));
    assert_eq!(take_text(&mut node).await, "你好，World!。整理好了。");
    // 「也可以直接启用规则」: rules are on at once, and any rule may change.
    node.handle.send(CoreCommand::SetMcp(McpSettings { write: McpWrite::EnableRules, ..on(48208) })).await.unwrap();
    wait(&mut node, |e| if let CoreEvent::Settings(s) = e { (s.mcp.write == McpWrite::EnableRules).then_some(()) } else { None }).await;
    let direct = call(&service, "Codex", "rules_add", json!({ "name": "整理 → 收拾", "pattern": "整理", "replacement": "收拾" })).await.unwrap();
    assert_eq!(direct["pending"], false);
    rules(&mut node).await;
    assert_eq!(take_text(&mut node).await, "你好，World!。收拾好了。");
    let off = call(&service, "Codex", "rules_update", json!({ "id": id, "enabled": false })).await.unwrap();
    assert_eq!(off["rule"]["enabled"], false);
    // Undo the direct rule: it is gone.
    let status = wait(&mut node, |e| match e {
        CoreEvent::Mcp(s) if s.activity.iter().any(|a| a.action == McpAction::AddRule && a.client == "Codex") => Some(s.clone()),
        _ => None,
    })
    .await;
    let add = status.activity.iter().find(|a| a.action == McpAction::AddRule && a.client == "Codex").unwrap();
    assert!(!add.pending);
    node.handle.send(CoreCommand::McpUndo(add.id)).await.unwrap();
    let list = wait(&mut node, |e| if let CoreEvent::Rules(r) = e { (r.len() == 1).then(|| r.clone()) } else { None }).await;
    assert_eq!(list[0].id, id);
}

/// The activity is kept across a restart, and an undo still works after it.
#[tokio::test]
async fn the_activity_survives_a_restart_with_its_undo() {
    let dir = tempfile::tempdir().unwrap();
    let (mut node, _, service) = running(dir.path(), on(48209)).await;
    call(&service, "Claude Code", "dictionary_add", json!({ "term": "飞书", "heard_as": ["非书"] })).await.unwrap();
    let before = activity(&mut node, 1).await;
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
    drop(node);
    let mut node = start(dir.path(), Some(Arc::new(FakeHost::default())));
    let status = wait(&mut node, |e| if let CoreEvent::Mcp(s) = e { (!s.activity.is_empty()).then(|| s.clone()) } else { None }).await;
    assert_eq!(status.activity, before.activity);
    node.handle.send(CoreCommand::McpUndo(status.activity[0].id)).await.unwrap();
    assert!(dictionary(&mut node).await.is_empty(), "the undo removed the entry");
}
