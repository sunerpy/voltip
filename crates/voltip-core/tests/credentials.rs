#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Credentials of several fields through the real core task (docs/dictation.md §3.15): saving
//! merges the form's fields over the stored ones (a blank keeps, another set replaces whole), the
//! status names the stored set and never a value, and 测试连接 of a vendor that lists no models
//! checks the merged credential with the form's model, answering with the suggested models without
//! filling the selects' lists.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc;
use voltip_core::dictation::ServiceProbe;
use voltip_core::dictation::fakes::ports;
use voltip_core::{
    AppCore, CoreCommand, CoreConfig, CoreEvent, CoreHandle, EngineStatus, ProbeError, ProbeFailure, ProbeOutcome, ProbeReport, ProviderId, ServiceKind,
    Settings, SettingsStore,
};
use voltip_identity::{MemorySecretStore, SecretStore};

const STEP: Duration = Duration::from_secs(10);

/// One credential check as the probe saw it: provider, address, credential, model.
type Check = (ProviderId, String, Option<String>, String);

/// The vendor check stand-in: records every check and answers with `answer`.
struct FakeChecks {
    checks: Mutex<Vec<Check>>,
    lists: Mutex<usize>,
    answer: Mutex<Result<(), ProbeError>>,
}

#[async_trait]
impl ServiceProbe for FakeChecks {
    async fn list_models(&self, _base_url: &str, _key: Option<&str>) -> Result<Vec<String>, ProbeError> {
        *self.lists.lock().unwrap() += 1;
        Ok(Vec::new())
    }

    async fn check_credentials(&self, provider: ProviderId, base_url: &str, key: Option<&str>, model: &str) -> Result<(), ProbeError> {
        self.checks.lock().unwrap().push((provider, base_url.to_owned(), key.map(str::to_owned), model.to_owned()));
        self.answer.lock().unwrap().clone()
    }
}

struct Node {
    handle: CoreHandle,
    events: mpsc::Receiver<CoreEvent>,
    store: Arc<MemorySecretStore>,
    last: Option<EngineStatus>,
}

impl Node {
    async fn start(dir: &std::path::Path, probe: Arc<FakeChecks>) -> Self {
        SettingsStore::new(dir).save(&Settings { relay_enabled: false, ..Settings::default() }).unwrap();
        let mut config = CoreConfig::new(dir.to_path_buf());
        config.default_device_name = "Credentials Test".into();
        let store = Arc::new(MemorySecretStore::new());
        let probe: Arc<dyn ServiceProbe> = probe;
        let (handle, events) =
            AppCore::start_with(config, store.clone(), voltip_core::dictation::DictationPorts { service_probe: Some(probe), ..ports() }).unwrap();
        let mut node = Self { handle, events, store, last: None };
        node.wait(|e| matches!(e, CoreEvent::Ready { .. }).then_some(())).await;
        node
    }

    async fn send(&self, command: CoreCommand) {
        self.handle.send(command).await.unwrap();
    }

    async fn wait<T>(&mut self, mut pick: impl FnMut(&CoreEvent) -> Option<T>) -> T {
        loop {
            let ev = tokio::time::timeout(STEP, self.events.recv()).await.expect("event within 10 s").expect("core alive");
            if let CoreEvent::Engines(status) = &ev {
                self.last = Some(status.clone());
            }
            if let Some(v) = pick(&ev) {
                return v;
            }
        }
    }

    async fn engines_where(&mut self, pred: impl Fn(&EngineStatus) -> bool) -> EngineStatus {
        if let Some(status) = self.last.clone().filter(|s| pred(s)) {
            return status;
        }
        self.wait(|e| match e {
            CoreEvent::Engines(s) if pred(s) => Some(s.clone()),
            _ => None,
        })
        .await
    }

    async fn report(&mut self) -> ProbeReport {
        self.wait(|e| if let CoreEvent::ProviderProbe(r) = e { Some(r.clone()) } else { None }).await
    }

    async fn error(&mut self) -> String {
        self.wait(|e| if let CoreEvent::Error(message) = e { Some(message.clone()) } else { None }).await
    }

    fn stored(&self, entry: &str) -> Option<String> {
        self.store.get(entry).unwrap().map(|bytes| String::from_utf8(bytes.to_vec()).unwrap())
    }
}

fn set(provider: ProviderId, value: &str) -> CoreCommand {
    CoreCommand::SetProviderKey { provider, kind: ServiceKind::Asr, value: Some(value.into()) }
}

/// The stored set of `provider`'s recognition, as the status says it.
fn credential_set(status: &EngineStatus, provider: ProviderId) -> (bool, Option<String>) {
    let asr = status.providers.iter().find(|p| p.id == provider).and_then(|p| p.asr.as_ref()).expect("recognises");
    (asr.key.set, asr.key.credential_set.clone())
}

fn checks() -> Arc<FakeChecks> {
    Arc::new(FakeChecks { checks: Mutex::default(), lists: Mutex::new(0), answer: Mutex::new(Ok(())) })
}

#[tokio::test]
async fn saving_merges_the_fields_and_the_status_names_the_set() {
    let dir = tempfile::tempdir().unwrap();
    let mut node = Node::start(dir.path(), checks()).await;
    let tencent = "provider-key.tencent";

    // A first save needs every field: nothing is stored, the core says what is missing.
    node.send(set(ProviderId::Tencent, r#"{"app_id":"125"}"#)).await;
    let message = node.error().await;
    assert!(message.contains("secret_id") && message.contains("secret_key"), "{message}");
    assert_eq!(node.stored(tencent), None);

    node.send(set(ProviderId::Tencent, r#"{"app_id":"125","secret_id":"test-secret-id","secret_key":"secret-key-1"}"#)).await;
    let status = node.engines_where(|s| credential_set(s, ProviderId::Tencent).0).await;
    assert_eq!(credential_set(&status, ProviderId::Tencent), (true, Some("default".into())));
    assert_eq!(node.stored(tencent).as_deref(), Some(r#"{"set":"default","app_id":"125","secret_id":"test-secret-id","secret_key":"secret-key-1"}"#));
    let json = serde_json::to_string(&status).unwrap();
    assert!(!json.contains("secret-key-1") && !json.contains("test-secret-id"), "no value reaches the UI");

    // One field changed, the others left blank: kept.
    node.send(set(ProviderId::Tencent, r#"{"set":"default","app_id":"","secret_id":"","secret_key":"secret-key-2"}"#)).await;
    let want = r#"{"set":"default","app_id":"125","secret_id":"test-secret-id","secret_key":"secret-key-2"}"#;
    tokio::time::timeout(STEP, async {
        while node.stored(tencent).as_deref() != Some(want) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the merged credential is stored");

    // 豆包语音: the old console's pair, then the new console's key replaces it whole.
    node.send(set(ProviderId::VolcengineSpeech, r#"{"set":"app_token","app_id":"app","access_token":"tok"}"#)).await;
    node.engines_where(|s| credential_set(s, ProviderId::VolcengineSpeech).1.as_deref() == Some("app_token")).await;
    node.send(set(ProviderId::VolcengineSpeech, r#"{"set":"api_key"}"#)).await;
    assert!(node.error().await.contains("api_key"), "a new set needs its own fields");
    node.send(set(ProviderId::VolcengineSpeech, r#"{"set":"api_key","api_key":"volc-key"}"#)).await;
    node.engines_where(|s| credential_set(s, ProviderId::VolcengineSpeech).1.as_deref() == Some("api_key")).await;
    assert_eq!(node.stored("provider-key.volcengine-speech").as_deref(), Some(r#"{"set":"api_key","api_key":"volc-key"}"#));
}

#[tokio::test]
async fn a_vendor_without_a_model_list_is_checked_with_the_merged_credential() {
    let dir = tempfile::tempdir().unwrap();
    let probe = checks();
    let mut node = Node::start(dir.path(), probe.clone()).await;
    let probe_cmd = |key: Option<&str>, model: Option<&str>| CoreCommand::ProbeProvider {
        provider: ProviderId::Iflytek,
        kind: ServiceKind::Asr,
        base_url: None,
        key: key.map(str::to_owned),
        model: model.map(str::to_owned),
    };

    // Nothing stored and an incomplete form: missing, and nothing is asked.
    node.send(probe_cmd(Some(r#"{"app_id":"app"}"#), None)).await;
    let report = node.report().await;
    assert!(matches!(report.outcome, ProbeOutcome::Failed { reason: ProbeFailure::KeyMissing, .. }), "{report:?}");
    assert!(probe.checks.lock().unwrap().is_empty());

    node.send(set(ProviderId::Iflytek, r#"{"app_id":"app","api_key":"k","api_secret":"s"}"#)).await;
    node.engines_where(|s| credential_set(s, ProviderId::Iflytek).0).await;

    // The form changed one field: the check gets the merged credential and the default model,
    // and answers with the suggested models as they are.
    node.send(probe_cmd(Some(r#"{"set":"default","api_secret":"s2"}"#), None)).await;
    let report = node.report().await;
    assert!(matches!(&report.outcome, ProbeOutcome::Ok { models, .. } if models == &["iat".to_owned()]), "{report:?}");
    let (provider, url, key, model) = probe.checks.lock().unwrap().last().cloned().unwrap();
    assert_eq!((provider, url.as_str(), model.as_str()), (ProviderId::Iflytek, "https://iat-api.xfyun.cn", "iat"));
    assert_eq!(key.as_deref(), Some(r#"{"set":"default","app_id":"app","api_key":"k","api_secret":"s2"}"#));
    assert_eq!(
        node.stored("provider-key.iflytek").as_deref(),
        Some(r#"{"set":"default","app_id":"app","api_key":"k","api_secret":"s"}"#),
        "a test stores nothing"
    );

    // The form's model is the one checked; a refusal says what the service said.
    *probe.answer.lock().unwrap() = Err(ProbeError::new(ProbeFailure::Refused).with_detail("10313: appid 与 apikey 不匹配"));
    node.send(probe_cmd(None, Some("iat-custom"))).await;
    let report = node.report().await;
    assert!(matches!(&report.outcome, ProbeOutcome::Failed { reason: ProbeFailure::Refused, detail: Some(d), .. } if d.contains("10313")), "{report:?}");
    assert_eq!(probe.checks.lock().unwrap().last().map(|c| c.3.clone()).as_deref(), Some("iat-custom"));

    // Never a model list: neither the probe nor a refresh asks for one, and the selects keep the presets.
    node.send(CoreCommand::RefreshProviderModels { provider: ProviderId::Iflytek, kind: ServiceKind::Asr }).await;
    node.send(probe_cmd(None, None)).await;
    node.report().await;
    assert_eq!(*probe.lists.lock().unwrap(), 0);
    let status = node.last.clone().unwrap();
    let iflytek = status.providers.iter().find(|p| p.id == ProviderId::Iflytek).unwrap();
    assert!(!iflytek.lists_models);
    assert!(iflytek.asr.as_ref().unwrap().listed.is_empty());
    assert_eq!(iflytek.credential.first().map(|s| s.fields.iter().map(|f| f.id.as_str()).collect::<Vec<_>>()), Some(vec!["app_id", "api_key", "api_secret"]));
}
