//! The local MCP service the app hosts (docs/dictation.md §25): switched on in the settings, it
//! listens on 127.0.0.1 through the shell's [`McpHost`], and every call comes to this task. Here a
//! call is checked against the settings of that moment, a read runs on a blocking thread with a
//! copy of the lists and the history's own read-only connection, and a write goes through the
//! same store methods (and the same events) as the interface's commands. Each call is noted in
//! the recent activity; a write can be undone from the settings.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::sync::mpsc;
use uuid::Uuid;

use super::{CoreEvent, Runtime, now_ms};
use crate::CoreError;
use crate::history::HistoryReader;
use crate::mcp::activity::{ActivityDraft, Undo};
use crate::mcp::reads::{self, Lists};
use crate::mcp::records::{EntryView, RuleView};
use crate::mcp::tools::McpRequest;
use crate::mcp::writes::{self, EntryAdd};
use crate::mcp::{
    ActivityLog, CoreMcp, MAX_CALLS_PER_SECOND, MAX_LISTED_CLIENTS, MAX_REQUEST_BYTES, MAX_SESSIONS, MCP_UNAVAILABLE, McpAction, McpCall, McpClient, McpDone,
    McpHost, McpInbound, McpListenConfig, McpService, McpSettings, McpStatus, RunningMcp, SetupClient, ToolError, access, address, setup_command, token_path,
};
use crate::serve::{load_or_create_token, rotate_token};
use crate::settings::MIN_SERVE_PORT;
use crate::ui::ServePhase;
use crate::vocabulary::EntrySource;

/// The service's part of the runtime.
pub(super) struct McpRuntime {
    host: Option<Arc<dyn McpHost>>,
    running: Option<Box<dyn RunningMcp>>,
    port: Option<u16>,
    phase: ServePhase,
    address: Option<String>,
    error: Option<String>,
    clients: Vec<McpClient>,
    log: ActivityLog,
    /// The history's own read-only connection: reads never wait for the core's writer.
    reader: Arc<HistoryReader>,
    /// The inbox of this task, handed to the service.
    tx: mpsc::Sender<McpInbound>,
}

impl McpRuntime {
    /// With the shell's host, if it has one; the second value is the notice for the UI when the
    /// activity file could not be used.
    pub(super) fn new(host: Option<Arc<dyn McpHost>>, data_dir: &std::path::Path, tx: mpsc::Sender<McpInbound>) -> (Self, Option<String>) {
        let (log, notice) = ActivityLog::open(data_dir);
        let runtime = Self {
            host,
            running: None,
            port: None,
            phase: ServePhase::Off,
            address: None,
            error: None,
            clients: Vec::new(),
            log,
            reader: Arc::new(HistoryReader::new(data_dir)),
            tx,
        };
        (runtime, notice)
    }

    /// `UiState.mcp` now.
    pub(super) fn status(&self) -> McpStatus {
        McpStatus {
            available: self.host.is_some(),
            phase: self.phase,
            address: self.address.clone(),
            error: self.error.clone(),
            clients: self.clients.clone(),
            activity: self.log.activities(),
        }
    }

    /// Note that `client` was heard from now; most recent first.
    fn seen(&mut self, client: &str, at_ms: u64) {
        self.clients.retain(|c| c.name != client);
        self.clients.insert(0, McpClient { name: client.to_owned(), last_seen_ms: at_ms });
        self.clients.truncate(MAX_LISTED_CLIENTS);
    }
}

fn mcp_err(message: impl Into<String>) -> CoreError {
    CoreError::Invalid(message.into())
}

fn tool_failed(e: impl std::fmt::Display) -> ToolError {
    ToolError::Failed(e.to_string())
}

impl Runtime {
    fn emit_mcp(&self) {
        self.emit(CoreEvent::Mcp(self.mcp.status()));
    }

    fn set_mcp_status(&mut self, phase: ServePhase, address: Option<String>, error: Option<String>) {
        self.mcp.phase = phase;
        self.mcp.address = address;
        self.mcp.error = error;
        self.emit_mcp();
    }

    /// Start, stop or restart the listener as `Settings.mcp` says.
    pub(super) async fn apply_mcp(&mut self) {
        let Some(host) = self.mcp.host.clone() else { return };
        let wanted = self.settings.mcp.clone();
        if !wanted.enabled {
            self.stop_mcp();
            if self.mcp.phase != ServePhase::Off {
                self.set_mcp_status(ServePhase::Off, None, None);
            }
            return;
        }
        if self.mcp.running.is_some() && self.mcp.port == Some(wanted.port) {
            return;
        }
        self.stop_mcp();
        let token = match load_or_create_token(&token_path(&self.config.data_dir)) {
            Ok((token, warning)) => {
                if let Some(warning) = warning {
                    tracing::warn!("{warning}");
                }
                token
            }
            Err(e) => {
                self.set_mcp_status(ServePhase::Failed, None, Some(e));
                return;
            }
        };
        self.set_mcp_status(ServePhase::Starting, None, None);
        let config = McpListenConfig {
            addr: SocketAddr::from((Ipv4Addr::LOCALHOST, wanted.port)),
            token,
            max_sessions: MAX_SESSIONS,
            calls_per_second: MAX_CALLS_PER_SECOND,
            max_request_bytes: MAX_REQUEST_BYTES,
        };
        let service: Arc<dyn McpService> = Arc::new(CoreMcp::new(self.mcp.tx.clone()));
        match host.start(config, service).await {
            Ok(running) => {
                let address = address(running.addr().port());
                tracing::info!(%address, "MCP service on");
                self.mcp.port = Some(wanted.port);
                self.mcp.running = Some(running);
                self.mcp.clients.clear();
                self.set_mcp_status(ServePhase::Running, Some(address), None);
            }
            Err(e) => {
                tracing::warn!(error = %e, "the MCP service did not start");
                self.set_mcp_status(ServePhase::Failed, None, Some(e));
            }
        }
    }

    /// Stop listening and end every session (the app quits, the service is switched off or moves).
    pub(super) fn stop_mcp(&mut self) {
        if let Some(running) = self.mcp.running.take() {
            running.stop();
        }
        self.mcp.port = None;
    }

    /// `SetMcp`: check, persist, apply. A change of what may be read or written applies to the
    /// next call; only a change of the port restarts the listener.
    pub(super) async fn set_mcp(&mut self, mcp: McpSettings) -> Result<(), CoreError> {
        if self.mcp.host.is_none() {
            return Err(mcp_err(MCP_UNAVAILABLE));
        }
        if mcp.port < MIN_SERVE_PORT {
            return Err(mcp_err(format!("mcp: 端口须在 {MIN_SERVE_PORT}–65535 之间")));
        }
        if mcp.port == self.settings.serve.port {
            return Err(mcp_err(format!("mcp: 端口 {} 已用于本机服务，请换一个端口", mcp.port)));
        }
        self.settings.mcp = mcp;
        self.save_settings()?;
        self.apply_mcp().await;
        Ok(())
    }

    fn mcp_token(&self) -> Result<String, CoreError> {
        if self.mcp.host.is_none() {
            return Err(mcp_err(MCP_UNAVAILABLE));
        }
        load_or_create_token(&token_path(&self.config.data_dir)).map(|(token, _)| token).map_err(|e| mcp_err(format!("mcp: {e}")))
    }

    async fn copy_to_clipboard(&self, text: String, what: &str) -> Result<(), CoreError> {
        let injector = self.dictation.injector();
        tokio::task::spawn_blocking(move || injector.copy(&text))
            .await
            .map_err(|e| mcp_err(format!("mcp: 复制任务失败：{e}")))?
            .map_err(|e| mcp_err(format!("mcp: {what}无法复制到剪贴板：{e}")))
    }

    /// `McpCopyToken`: the token goes to the clipboard, never to the webview.
    pub(super) async fn copy_mcp_token(&mut self) -> Result<(), CoreError> {
        let token = self.mcp_token()?;
        self.copy_to_clipboard(token, "令牌").await
    }

    /// `McpCopySetup`: the command that connects `client`, with the real token, to the clipboard.
    pub(super) async fn copy_mcp_setup(&mut self, client: SetupClient) -> Result<(), CoreError> {
        let token = self.mcp_token()?;
        let address = self.mcp.address.clone().unwrap_or_else(|| address(self.settings.mcp.port));
        self.copy_to_clipboard(setup_command(client, &address, &token, cfg!(windows)), "命令").await
    }

    /// `McpRotateToken`: a new token, in force at once; clients need it from now on.
    pub(super) fn rotate_mcp_token(&mut self) -> Result<(), CoreError> {
        if self.mcp.host.is_none() {
            return Err(mcp_err(MCP_UNAVAILABLE));
        }
        let token = rotate_token(&token_path(&self.config.data_dir)).map_err(|e| mcp_err(format!("mcp: {e}")))?;
        if let Some(running) = &self.mcp.running {
            running.set_token(token);
        }
        tracing::info!("the MCP service's token was replaced");
        Ok(())
    }

    /// `McpUndo`: put back what an AI tool's write changed, while the item is still as the write
    /// left it.
    pub(super) fn mcp_undo(&mut self, id: Uuid) -> Result<(), CoreError> {
        if self.mcp.host.is_none() {
            return Err(mcp_err(MCP_UNAVAILABLE));
        }
        let undo = self.mcp.log.undo_of(id).map_err(|e| mcp_err(e.to_string()))?.clone();
        match undo {
            Undo::Entry { before, after } => {
                match self.dictionary.entries().iter().find(|e| e.id == after.id) {
                    None => return Err(mcp_err("mcp: 无法撤销：这个词条已经删除")),
                    Some(current) if *current != after => return Err(mcp_err("mcp: 无法撤销：这个词条之后又被修改过")),
                    Some(_) => {}
                }
                let result = match before {
                    None => self.dictionary.remove(after.id),
                    Some(before) => self.dictionary.restore(before),
                };
                self.dictionary_changed(result)?;
            }
            Undo::Rule { before, after } => {
                match self.rules.rules().iter().find(|r| r.id == after.id) {
                    None => return Err(mcp_err("mcp: 无法撤销：这条规则已经删除")),
                    Some(current) if *current != after => return Err(mcp_err("mcp: 无法撤销：这条规则之后又被修改过")),
                    Some(_) => {}
                }
                let result = match before {
                    None => self.rules.remove(after.id),
                    Some(before) => self.rules.restore(before),
                };
                self.rules_changed(result)?;
            }
        }
        self.mcp.log.mark_undone(id);
        self.emit_mcp();
        Ok(())
    }

    /// What the service sent.
    pub(super) fn on_mcp(&mut self, inbound: McpInbound) {
        match inbound {
            McpInbound::Call(call) => self.on_mcp_call(call),
            McpInbound::Connected { client } => {
                tracing::info!(client = %client, "MCP client connected");
                self.mcp.seen(&client, now_ms());
                self.emit_mcp();
            }
            McpInbound::Done(done) => self.on_mcp_done(*done),
        }
    }

    fn on_mcp_call(&mut self, call: McpCall) {
        let McpCall { client, request, reply } = call;
        let tool = request.tool();
        if self.mcp.running.is_none() {
            let _ = reply.send(Err(ToolError::Unavailable(access::bilingual("Voltip 的 AI 工具接入已关闭。", "Voltip's AI tool access is off."))));
            return;
        }
        let settings = self.settings.mcp.clone();
        self.mcp.seen(&client, now_ms());
        if let Err(denied) = access::check(access::need(tool), &settings) {
            tracing::info!(tool = tool.name(), client = %client, "MCP call refused by the settings");
            self.emit_mcp();
            let _ = reply.send(Err(denied));
            return;
        }
        if tool.read_only() {
            let lists = Lists { dictionary: self.dictionary.entries().to_vec(), rules: self.rules.rules().to_vec() };
            let reader = self.mcp.reader.clone();
            let tx = self.mcp.tx.clone();
            let started = std::time::Instant::now();
            tokio::task::spawn_blocking(move || {
                let result = reads::run(&request, &settings, &lists, &reader, now_ms());
                tracing::info!(tool = tool.name(), client = %client, ok = result.is_ok(), ms = started.elapsed().as_millis() as u64, "MCP read");
                if tx.blocking_send(McpInbound::Done(Box::new(McpDone { client, result, reply }))).is_err() {
                    tracing::debug!("an MCP read finished after the core stopped");
                }
            });
            return;
        }
        let started = std::time::Instant::now();
        let result = self.mcp_write(&client, request, &settings);
        tracing::info!(tool = tool.name(), client = %client, ok = result.is_ok(), ms = started.elapsed().as_millis() as u64, "MCP write");
        let answer = result.map(|(value, draft, undo)| if let Some(draft) = draft { self.note_mcp(&client, draft, undo, &value) } else { value });
        self.emit_mcp();
        let _ = reply.send(answer);
    }

    fn on_mcp_done(&mut self, done: McpDone) {
        let McpDone { client, result, reply } = done;
        // Taking a permission back applies to a read that was still running, too.
        let answer = match result {
            Ok((value, draft)) => {
                let still = match draft.action {
                    McpAction::ReadSummary => access::check(access::Need::Read(crate::mcp::McpRead::Summary), &self.settings.mcp),
                    McpAction::SearchHistory | McpAction::ReadEntry => access::check(access::Need::Read(crate::mcp::McpRead::Full), &self.settings.mcp),
                    _ => Ok(()),
                };
                match (still, self.mcp.running.is_some()) {
                    (Err(denied), _) => Err(denied),
                    (Ok(()), false) => Err(ToolError::Unavailable(access::bilingual("Voltip 的 AI 工具接入已关闭。", "Voltip's AI tool access is off."))),
                    (Ok(()), true) => Ok(self.note_mcp(&client, draft, None, &value)),
                }
            }
            Err(e) => Err(e),
        };
        self.emit_mcp();
        let _ = reply.send(answer);
    }

    /// Note a call in the activity; a write's result gets the activity's id (what the user undoes).
    fn note_mcp(&mut self, client: &str, draft: ActivityDraft, undo: Option<Undo>, value: &Value) -> Value {
        let writes = draft.action.writes();
        let activity = self.mcp.log.record(draft.into_activity(client, now_ms()), undo);
        let mut value = value.clone();
        if writes && let Some(object) = value.as_object_mut() {
            object.insert("activity_id".into(), Value::String(activity.id.to_string()));
        }
        value
    }

    /// Run a writing tool for `client`: the result, what to note (`None`: nothing changed) and what
    /// undoing it puts back.
    fn mcp_write(&mut self, client: &str, request: McpRequest, settings: &McpSettings) -> Result<(Value, Option<ActivityDraft>, Option<Undo>), ToolError> {
        let now = now_ms();
        let source = EntrySource::Mcp { client: client.to_owned() };
        match request {
            McpRequest::DictionaryAdd { draft } => match writes::plan_entry_add(self.dictionary.entries(), &draft)? {
                EntryAdd::Create(draft) => {
                    let id = self.dictionary.add(&draft, source, now).map_err(tool_failed)?;
                    self.dictionary_changed(Ok(())).map_err(tool_failed)?;
                    let after = self.entry(id)?;
                    let value = json!({ "id": id, "result": "created", "entry": EntryView::new(&after, 0) });
                    let note = ActivityDraft::new(McpAction::AddEntry).subject(after.term.clone());
                    Ok((value, Some(note), Some(Undo::Entry { before: None, after })))
                }
                EntryAdd::Merge { id, draft, added } => {
                    let before = self.entry(id)?;
                    self.dictionary.update(id, &draft, now).map_err(tool_failed)?;
                    self.dictionary_changed(Ok(())).map_err(tool_failed)?;
                    let after = self.entry(id)?;
                    let value = json!({ "id": id, "result": "merged", "added": added, "entry": EntryView::new(&after, 0) });
                    let note = ActivityDraft::new(McpAction::MergeEntry).subject(after.term.clone()).count(added.len());
                    Ok((value, Some(note), Some(Undo::Entry { before: Some(before), after })))
                }
                EntryAdd::Unchanged { id } => {
                    let entry = self.entry(id)?;
                    Ok((json!({ "id": id, "result": "unchanged", "entry": EntryView::new(&entry, 0) }), None, None))
                }
            },
            McpRequest::DictionaryUpdate { id, patch } => {
                let before = self.entry(id)?;
                let draft = patch.apply(writes::draft_of(&before));
                if draft == writes::draft_of(&before) {
                    return Ok((json!({ "id": id, "result": "unchanged", "entry": EntryView::new(&before, 0) }), None, None));
                }
                self.dictionary.update(id, &draft, now).map_err(tool_failed)?;
                self.dictionary_changed(Ok(())).map_err(tool_failed)?;
                let after = self.entry(id)?;
                let value = json!({ "id": id, "result": "updated", "entry": EntryView::new(&after, 0) });
                let note = ActivityDraft::new(McpAction::UpdateEntry).subject(after.term.clone());
                Ok((value, Some(note), Some(Undo::Entry { before: Some(before), after })))
            }
            McpRequest::RulesAdd { draft } => {
                let waits = writes::new_rule_waits(settings);
                let id = self.rules.add_from(&draft, source, waits, now).map_err(tool_failed)?;
                self.rules_changed(Ok(())).map_err(tool_failed)?;
                let (position, after) = self.rule(id)?;
                let mut value = json!({ "id": id, "result": "created", "pending": after.pending, "rule": RuleView::new(&after, position, 0) });
                if waits
                    && draft.enabled
                    && let Some(object) = value.as_object_mut()
                {
                    object.insert("note".into(), Value::String(writes::waiting_note()));
                }
                let note = ActivityDraft::new(McpAction::AddRule).subject(after.name.clone()).pending(after.pending);
                Ok((value, Some(note), Some(Undo::Rule { before: None, after })))
            }
            McpRequest::RulesUpdate { id, patch } => {
                let (position, before) = self.rule(id)?;
                let Some((draft, pending)) = writes::plan_rule_update(settings, client, &before, &patch)? else {
                    return Ok((
                        json!({ "id": id, "result": "unchanged", "pending": before.pending, "rule": RuleView::new(&before, position, 0) }),
                        None,
                        None,
                    ));
                };
                self.rules.update_with(id, &draft, pending, now).map_err(tool_failed)?;
                self.rules_changed(Ok(())).map_err(tool_failed)?;
                let (position, after) = self.rule(id)?;
                let mut value = json!({ "id": id, "result": "updated", "pending": after.pending, "rule": RuleView::new(&after, position, 0) });
                if after.pending
                    && patch.enabled == Some(true)
                    && let Some(object) = value.as_object_mut()
                {
                    object.insert("note".into(), Value::String(writes::waiting_note()));
                }
                let note = ActivityDraft::new(McpAction::UpdateRule).subject(after.name.clone()).pending(after.pending);
                Ok((value, Some(note), Some(Undo::Rule { before: Some(before), after })))
            }
            McpRequest::DictionaryList { .. }
            | McpRequest::RulesList { .. }
            | McpRequest::Preview { .. }
            | McpRequest::Backtest { .. }
            | McpRequest::Candidates { .. }
            | McpRequest::HistorySearch { .. }
            | McpRequest::HistoryGet { .. } => Err(ToolError::Failed("not a write".into())),
        }
    }

    fn entry(&self, id: Uuid) -> Result<crate::vocabulary::DictionaryEntry, ToolError> {
        self.dictionary
            .entries()
            .iter()
            .find(|e| e.id == id)
            .cloned()
            .ok_or_else(|| ToolError::Failed(access::bilingual("没有这个词条（可能已删除）。", "No such dictionary entry (it may have been deleted).")))
    }

    fn rule(&self, id: Uuid) -> Result<(usize, crate::vocabulary::ReplacementRule), ToolError> {
        self.rules
            .rules()
            .iter()
            .enumerate()
            .find(|(_, r)| r.id == id)
            .map(|(i, r)| (i + 1, r.clone()))
            .ok_or_else(|| ToolError::Failed(access::bilingual("没有这条规则（可能已删除）。", "No such rule (it may have been deleted).")))
    }
}
