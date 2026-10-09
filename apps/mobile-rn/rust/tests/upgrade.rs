#![allow(clippy::unwrap_used, clippy::expect_used)]
//! An update from the Tauri phone app, end to end on the build host. The core keeps the same files
//! in both apps; the Tauri app kept them at the root of the app's data directory. Here the core
//! writes them there, they move on the first start of this app (`legacy::adopt_tauri_data`, as
//! `VoltipShell::start` does), and the shell starts on them with the same settings, history and
//! device identity: the Keystore holds the identity in both apps under the same service id, which
//! the shared secret store stands in for.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use voltip_core::dictation::fakes;
use voltip_core::history::{HistoryEntry, HistoryStore};
use voltip_core::{Settings, SettingsStore};
use voltip_identity::MemorySecretStore;
use voltip_rn::Shell;
use voltip_rn::host::RecordingHost;

const STEP_TIMEOUT: Duration = Duration::from_secs(15);

struct Running {
    shell: Shell,
    runtime: Option<tokio::runtime::Runtime>,
}

impl Running {
    /// The shell on `data_dir` with `store`, offline.
    fn start(data_dir: &Path, store: Arc<MemorySecretStore>) -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().worker_threads(2).build().unwrap();
        let mut config = voltip_rn::shell::phone_config(data_dir.to_path_buf(), "0.0.50");
        config.discovery = None;
        config.direct_bind = "127.0.0.1:0".parse().unwrap();
        let host = Arc::new(RecordingHost::default());
        let shell = Shell::start(runtime.handle().clone(), config, store, fakes::ports(), host).unwrap();
        let running = Self { shell, runtime: Some(runtime) };
        running.wait(|s| s["identity"].is_object());
        running
    }

    fn invoke(&self, command: &str, args: Value) -> Value {
        self.shell.invoke_blocking(command, args).unwrap()
    }

    fn wait(&self, done: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + STEP_TIMEOUT;
        loop {
            let state = self.invoke("core_state", Value::Null);
            if done(&state) {
                return state;
            }
            assert!(Instant::now() < deadline, "condition not met within {STEP_TIMEOUT:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.shell.shutdown();
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(Duration::from_secs(2));
        }
    }
}

/// A take as the Tauri app recorded it.
fn entry() -> HistoryEntry {
    serde_json::from_value(json!({
        "id": "6f1c2a8e-4b1d-4c3e-9a2f-8d7e6c5b4a39",
        "at_ms": 1_790_000_000_000_u64,
        "raw_text": "明天下午三点开会",
        "text": "明天下午三点开会。",
        "refined": true,
        "asr_model": "builtin",
        "duration_ms": 2_400,
        "asr_ms": 600,
        "outcome": { "kind": "clipboard", "reason": "phone" },
    }))
    .unwrap()
}

#[test]
fn an_update_from_the_tauri_app_keeps_its_settings_history_and_identity() {
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(MemorySecretStore::new());
    // The Tauri app's phone: its own settings, a take in the history, and the identity it made.
    let settings = Settings { relay_enabled: false, auto_update: true, lan_discovery: false, ..Settings::default() };
    SettingsStore::new(root.path()).save(&settings).unwrap();
    HistoryStore::open(root.path()).push(entry(), 100).unwrap();
    let identity = {
        let tauri = Running::start(root.path(), store.clone());
        tauri.invoke("core_state", Value::Null)["identity"].clone()
    };

    let data_dir = root.path().join("files/voltip");
    std::fs::create_dir_all(&data_dir).unwrap();
    let app_root = voltip_rn::legacy::app_root_of(&data_dir).unwrap();
    let moved = voltip_rn::legacy::adopt_tauri_data(app_root, &data_dir);
    assert!(moved.iter().any(|m| m == "settings.json"), "{moved:?}");
    assert!(moved.iter().any(|m| m == "history.sqlite3"), "{moved:?}");

    let app = Running::start(&data_dir, store);
    let state = app.invoke("core_state", Value::Null);
    assert_eq!(state["identity"], identity, "the phone keeps its device identity");
    assert_eq!(state["settings"]["auto_update"], json!(true));
    assert_eq!(state["settings"]["lan_discovery"], json!(false));
    let page = app.invoke("history_query", json!({ "limit": 20 }));
    let texts: Vec<&str> = page["entries"].as_array().unwrap().iter().filter_map(|e| e["text"].as_str()).collect();
    assert_eq!(texts, ["明天下午三点开会。"], "{page}");
    // Nothing of the core stays at the root.
    assert!(!root.path().join("settings.json").exists());
    assert!(!root.path().join("history.sqlite3").exists());
}
