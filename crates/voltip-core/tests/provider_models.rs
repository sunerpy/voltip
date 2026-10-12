#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The models a provider lists, through the real core task (docs/dictation.md §3.3): with a saved
//! key, `RefreshProviderModels` and a 测试连接 with the saved values bring the provider's models,
//! filtered by service, to `ServiceStatus.listed`; a new key drops them, another endpoint hides
//! them, a refused key forgets them, and one refresh runs at a time.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{mpsc, watch};
use voltip_core::dictation::ServiceProbe;
use voltip_core::dictation::fakes::ports;
use voltip_core::{
    AppCore, CoreCommand, CoreConfig, CoreEvent, CoreHandle, EngineSettings, EngineStatus, ProbeError, ProbeFailure, ProbeOutcome, ProviderId,
    ProviderSettings, ServiceKind, Settings, SettingsStore,
};
use voltip_identity::MemorySecretStore;

const STEP: Duration = Duration::from_secs(10);
/// A held request gives up after this long, so a failed assertion cannot hang the run.
const HOLD_LIMIT: Duration = Duration::from_secs(30);
const GROQ: &str = "https://api.groq.com/openai/v1";

type Answer = Result<Vec<String>, ProbeError>;

/// `GET /models` stand-in: answers by endpoint and key, records every request, and holds the
/// requests while it is closed.
struct FakeLists {
    answers: Mutex<HashMap<(String, Option<String>), Answer>>,
    calls: Mutex<Vec<(String, Option<String>)>>,
    count: watch::Sender<usize>,
    open: watch::Sender<bool>,
}

impl FakeLists {
    fn new() -> Arc<Self> {
        Arc::new(Self { answers: Mutex::default(), calls: Mutex::default(), count: watch::Sender::new(0), open: watch::Sender::new(true) })
    }

    fn answer(&self, url: &str, key: &str, answer: Answer) {
        self.answers.lock().unwrap().insert((url.to_owned(), Some(key.to_owned())), answer);
    }

    fn hold(&self, held: bool) {
        self.open.send_replace(!held);
    }

    fn calls(&self) -> Vec<(String, Option<String>)> {
        self.calls.lock().unwrap().clone()
    }

    async fn wait_calls(&self, n: usize) {
        let mut count = self.count.subscribe();
        tokio::time::timeout(STEP, count.wait_for(|c| *c >= n)).await.expect("the request within 10 s").unwrap();
    }
}

#[async_trait]
impl ServiceProbe for FakeLists {
    async fn list_models(&self, base_url: &str, key: Option<&str>) -> Result<Vec<String>, ProbeError> {
        let asked = (base_url.to_owned(), key.map(str::to_owned));
        self.calls.lock().unwrap().push(asked.clone());
        self.count.send_modify(|c| *c += 1);
        let mut open = self.open.subscribe();
        let _ = tokio::time::timeout(HOLD_LIMIT, open.wait_for(|o| *o)).await;
        self.answers.lock().unwrap().get(&asked).cloned().unwrap_or(Err(ProbeError::new(ProbeFailure::Unreachable)))
    }
}

fn models(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|m| (*m).to_owned()).collect()
}

/// What Groq lists for each key: both services' models, a guard and a voice model.
fn groq_list(tag: &str) -> Vec<String> {
    models(&["whisper-large-v3", "playai-tts", "meta-llama/llama-guard-4-12b", "openai/gpt-oss-120b", &format!("qwen/{tag}")])
}

struct Node {
    handle: CoreHandle,
    events: mpsc::Receiver<CoreEvent>,
    /// Every engines status seen, in order.
    seen: Vec<EngineStatus>,
}

impl Node {
    async fn send(&self, command: CoreCommand) {
        self.handle.send(command).await.unwrap();
    }

    async fn wait<T>(&mut self, mut pick: impl FnMut(&CoreEvent) -> Option<T>) -> T {
        loop {
            let ev = tokio::time::timeout(STEP, self.events.recv()).await.expect("event within 10 s").expect("core alive");
            if let CoreEvent::Engines(status) = &ev {
                self.seen.push(status.clone());
            }
            if let Some(v) = pick(&ev) {
                return v;
            }
        }
    }

    /// The engines status once `pred` holds: the last one seen, or a later one.
    async fn engines_where(&mut self, pred: impl Fn(&EngineStatus) -> bool) -> EngineStatus {
        if let Some(status) = self.seen.last().filter(|s| pred(s)) {
            return status.clone();
        }
        self.wait(|e| match e {
            CoreEvent::Engines(s) if pred(s) => Some(s.clone()),
            _ => None,
        })
        .await
    }
}

fn listed(status: &EngineStatus, provider: ProviderId, kind: ServiceKind) -> Vec<String> {
    let p = status.providers.iter().find(|p| p.id == provider).expect("the provider is listed");
    match kind {
        ServiceKind::Asr => p.asr.as_ref(),
        ServiceKind::Llm => p.llm.as_ref(),
    }
    .expect("the provider offers the service")
    .listed
    .clone()
}

fn engines(groq_url: Option<&str>) -> EngineSettings {
    EngineSettings {
        llm_provider: ProviderId::Groq,
        providers: [(ProviderId::Groq, ProviderSettings { llm_url: groq_url.map(str::to_owned), ..Default::default() })].into(),
        ..EngineSettings::default()
    }
}

async fn start(dir: &std::path::Path, key: Option<&str>, lists: &Arc<FakeLists>) -> Node {
    SettingsStore::new(dir).save(&Settings { relay_enabled: false, engines: engines(None), ..Settings::default() }).unwrap();
    let mut config = CoreConfig::new(dir.to_path_buf());
    config.default_device_name = "Provider Models Test".into();
    let store = MemorySecretStore::new();
    if let Some(key) = key {
        voltip_identity::SecretStore::set(&store, "provider-key.groq", key.as_bytes()).unwrap();
    }
    let probe: Arc<dyn ServiceProbe> = lists.clone();
    let (handle, events) =
        AppCore::start_with(config, Arc::new(store), voltip_core::dictation::DictationPorts { service_probe: Some(probe), ..ports() }).unwrap();
    let mut node = Node { handle, events, seen: Vec::new() };
    node.wait(|e| matches!(e, CoreEvent::Ready { .. }).then_some(())).await;
    node.engines_where(|_| true).await;
    node
}

fn refresh(provider: ProviderId, kind: ServiceKind) -> CoreCommand {
    CoreCommand::RefreshProviderModels { provider, kind }
}

fn set_key(value: &str) -> CoreCommand {
    CoreCommand::SetProviderKey { provider: ProviderId::Groq, kind: ServiceKind::Llm, value: Some(value.into()) }
}

#[tokio::test]
async fn a_saved_key_brings_the_providers_models_to_the_selects() {
    let dir = tempfile::tempdir().unwrap();
    let lists = FakeLists::new();
    lists.answer(GROQ, "gsk_a", Ok(groq_list("qwen3.8-27b")));
    lists.answer(GROQ, "gsk_b", Ok(groq_list("qwen3.9-32b")));
    lists.answer("https://proxy.example.test/v1", "gsk_b", Ok(models(&["proxied-model"])));
    let mut node = start(dir.path(), None, &lists).await;

    // No key yet: nothing to ask, and the built-in and on-device providers never are.
    node.send(refresh(ProviderId::Groq, ServiceKind::Llm)).await;
    node.send(refresh(ProviderId::Local, ServiceKind::Asr)).await;
    node.send(refresh(ProviderId::Builtin, ServiceKind::Llm)).await;
    node.send(set_key("gsk_a")).await;
    node.engines_where(|s| s.refine_issue.is_none()).await;
    assert!(lists.calls().is_empty(), "{:?}", lists.calls());

    // Each service gets the models it can use, as the provider lists them.
    node.send(refresh(ProviderId::Groq, ServiceKind::Llm)).await;
    let status = node.engines_where(|s| !listed(s, ProviderId::Groq, ServiceKind::Llm).is_empty()).await;
    assert_eq!(listed(&status, ProviderId::Groq, ServiceKind::Llm), ["openai/gpt-oss-120b", "qwen/qwen3.8-27b"]);
    assert!(listed(&status, ProviderId::Groq, ServiceKind::Asr).is_empty(), "recognition was not asked");
    node.send(refresh(ProviderId::Groq, ServiceKind::Asr)).await;
    let status = node.engines_where(|s| !listed(s, ProviderId::Groq, ServiceKind::Asr).is_empty()).await;
    assert_eq!(listed(&status, ProviderId::Groq, ServiceKind::Asr), ["whisper-large-v3"]);
    assert_eq!(lists.calls(), [(GROQ.to_owned(), Some("gsk_a".to_owned())), (GROQ.to_owned(), Some("gsk_a".to_owned()))]);

    // Another key may be another account: both lists go.
    node.send(set_key("gsk_b")).await;
    node.engines_where(|s| listed(s, ProviderId::Groq, ServiceKind::Llm).is_empty() && listed(s, ProviderId::Groq, ServiceKind::Asr).is_empty()).await;
    node.send(refresh(ProviderId::Groq, ServiceKind::Llm)).await;
    node.engines_where(|s| listed(s, ProviderId::Groq, ServiceKind::Llm) == ["openai/gpt-oss-120b", "qwen/qwen3.9-32b"]).await;

    // A list belongs to the endpoint it came from: another one hides it until that one's arrives.
    node.send(CoreCommand::SetEngines(engines(Some("https://proxy.example.test/v1")))).await;
    node.engines_where(|s| listed(s, ProviderId::Groq, ServiceKind::Llm).is_empty()).await;
    node.send(refresh(ProviderId::Groq, ServiceKind::Llm)).await;
    node.engines_where(|s| listed(s, ProviderId::Groq, ServiceKind::Llm) == ["proxied-model"]).await;
    node.send(CoreCommand::SetEngines(engines(None))).await;
    node.engines_where(|s| s.providers.iter().any(|p| p.id == ProviderId::Groq && p.llm.as_ref().is_some_and(|l| l.base_url.as_deref() == Some(GROQ)))).await;
    let status = node.seen.last().unwrap().clone();
    assert!(listed(&status, ProviderId::Groq, ServiceKind::Llm).is_empty(), "the proxy's list is not the vendor's");
    node.send(refresh(ProviderId::Groq, ServiceKind::Llm)).await;
    node.engines_where(|s| listed(s, ProviderId::Groq, ServiceKind::Llm) == ["openai/gpt-oss-120b", "qwen/qwen3.9-32b"]).await;
}

#[tokio::test]
async fn a_test_with_the_saved_values_fills_the_list_and_a_draft_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let lists = FakeLists::new();
    lists.answer(GROQ, "gsk_a", Ok(groq_list("qwen3.8-27b")));
    lists.answer(GROQ, "gsk_draft", Ok(groq_list("draft-model")));
    let mut node = start(dir.path(), Some("gsk_a"), &lists).await;
    let saved = models(&["openai/gpt-oss-120b", "qwen/qwen3.8-27b"]);

    // 测试连接 answers with the count the select offers, and the select has them.
    node.send(CoreCommand::ProbeProvider { provider: ProviderId::Groq, kind: ServiceKind::Llm, base_url: None, key: None }).await;
    let report = node.wait(|e| if let CoreEvent::ProviderProbe(r) = e { Some(r.clone()) } else { None }).await;
    assert!(matches!(&report.outcome, ProbeOutcome::Ok { models, .. } if *models == saved), "{report:?}");
    node.engines_where(|s| listed(s, ProviderId::Groq, ServiceKind::Llm) == saved).await;

    // The same test with the saved address typed out counts as the saved values too; a key
    // typed in the form is only tried.
    lists.answer(GROQ, "gsk_a", Ok(groq_list("qwen3.9-32b")));
    node.send(CoreCommand::ProbeProvider { provider: ProviderId::Groq, kind: ServiceKind::Llm, base_url: Some(format!(" {GROQ} ")), key: None }).await;
    let newer = models(&["openai/gpt-oss-120b", "qwen/qwen3.9-32b"]);
    node.engines_where(|s| listed(s, ProviderId::Groq, ServiceKind::Llm) == newer).await;
    node.send(CoreCommand::ProbeProvider { provider: ProviderId::Groq, kind: ServiceKind::Llm, base_url: None, key: Some("gsk_draft".into()) }).await;
    let report = node.wait(|e| if let CoreEvent::ProviderProbe(r) = e { Some(r.clone()) } else { None }).await;
    assert!(matches!(&report.outcome, ProbeOutcome::Ok { models, .. } if models.contains(&"qwen/draft-model".to_owned())), "{report:?}");
    // A key refused on a refresh drops the list.
    lists.answer(GROQ, "gsk_a", Err(ProbeError::new(ProbeFailure::Unauthorized)));
    node.send(refresh(ProviderId::Groq, ServiceKind::Llm)).await;
    node.engines_where(|s| listed(s, ProviderId::Groq, ServiceKind::Llm).is_empty()).await;
    assert!(
        node.seen.iter().all(|s| !listed(s, ProviderId::Groq, ServiceKind::Llm).contains(&"qwen/draft-model".to_owned())),
        "the draft's list never reached the selects"
    );
}

#[tokio::test]
async fn one_refresh_runs_at_a_time_and_a_list_fetched_with_an_old_key_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let lists = FakeLists::new();
    lists.answer(GROQ, "gsk_a", Ok(groq_list("old-account")));
    lists.answer(GROQ, "gsk_b", Ok(groq_list("new-account")));
    let mut node = start(dir.path(), Some("gsk_a"), &lists).await;
    lists.hold(true);
    node.send(refresh(ProviderId::Groq, ServiceKind::Llm)).await;
    lists.wait_calls(1).await;
    // A second click while the first request runs asks nothing more.
    node.send(refresh(ProviderId::Groq, ServiceKind::Llm)).await;
    // A new key does not wait for the old request.
    node.send(set_key("gsk_b")).await;
    node.engines_where(|_| true).await;
    let before = node.seen.len();
    node.send(refresh(ProviderId::Groq, ServiceKind::Llm)).await;
    lists.wait_calls(2).await;
    lists.hold(false);
    node.engines_where(|s| listed(s, ProviderId::Groq, ServiceKind::Llm) == ["openai/gpt-oss-120b", "qwen/new-account"]).await;
    assert_eq!(lists.calls(), [(GROQ.to_owned(), Some("gsk_a".to_owned())), (GROQ.to_owned(), Some("gsk_b".to_owned()))]);
    assert!(
        node.seen[before..].iter().all(|s| !listed(s, ProviderId::Groq, ServiceKind::Llm).contains(&"qwen/old-account".to_owned())),
        "the old key's list was dropped"
    );
}
