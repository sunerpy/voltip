#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The UniFFI interface the app sees (docs/mobile-rn.md §3), on the build host: a Rust
//! [`PlatformHost`] where the phone has `VoltipHost.kt`, and the shell started with the dictation
//! fakes and the in-memory secret store. Commands answer JSON or the Tauri shell's error text, the
//! core's events reach the host, platform failures come back as the command's error, and a host
//! that fails to take an event does not stop the ones after it.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use voltip_core::{CoreConfig, Settings, SettingsStore};
use voltip_identity::MemorySecretStore;
use voltip_rn::{HostError, PlatformHost, ShellError, VoltipShell};

const STEP_TIMEOUT: Duration = Duration::from_secs(15);

/// What `VoltipHost.kt` would see: every event and request, a clipboard, and a switch that makes
/// the platform refuse.
#[derive(Default)]
struct FakePlatform {
    events: Mutex<Vec<String>>,
    calls: Mutex<Vec<String>>,
    clipboard: Mutex<Option<String>>,
    refuse: Mutex<Option<String>>,
    /// Fail this many `event` calls first (a JavaScript side that is not listening yet).
    drop_events: Mutex<usize>,
}

impl FakePlatform {
    fn outcome(&self, call: String) -> Result<(), HostError> {
        self.calls.lock().unwrap().push(call);
        match self.refuse.lock().unwrap().clone() {
            Some(reason) => Err(HostError::Failed { reason }),
            None => Ok(()),
        }
    }

    fn events(&self) -> Vec<Value> {
        self.events.lock().unwrap().iter().map(|e| serde_json::from_str(e).unwrap()).collect()
    }
}

impl PlatformHost for FakePlatform {
    fn event(&self, json: String) -> Result<(), HostError> {
        let mut drop = self.drop_events.lock().unwrap();
        if *drop > 0 {
            *drop -= 1;
            return Err(HostError::Failed { reason: "nobody listens".into() });
        }
        self.events.lock().unwrap().push(json);
        Ok(())
    }

    fn channel(&self, channel: u64, json: String) -> Result<(), HostError> {
        self.outcome(format!("channel {channel} {json}"))
    }

    fn clipboard_read(&self) -> Result<Option<String>, HostError> {
        self.outcome("clipboard_read".into())?;
        Ok(self.clipboard.lock().unwrap().clone())
    }

    fn clipboard_write(&self, text: String) -> Result<(), HostError> {
        self.outcome(format!("clipboard_write {text}"))?;
        *self.clipboard.lock().unwrap() = Some(text);
        Ok(())
    }

    fn share_text(&self, text: String) -> Result<(), HostError> {
        self.outcome(format!("share_text {text}"))
    }

    fn share_file(&self, name: String, text: String, mime: String) -> Result<(), HostError> {
        self.outcome(format!("share_file {name} {mime} {}", text.len()))
    }

    fn multicast(&self, held: bool) -> Result<(), HostError> {
        self.outcome(format!("multicast {held}"))
    }

    fn open_url(&self, url: String) -> Result<(), HostError> {
        self.outcome(format!("open_url {url}"))
    }
}

/// An offline phone: no relay, no mDNS, the LAN host on an ephemeral port.
fn offline_config(dir: &Path) -> CoreConfig {
    SettingsStore::new(dir).save(&Settings { relay_enabled: false, ..Settings::default() }).unwrap();
    let mut config = voltip_rn::shell::phone_config(dir.to_path_buf(), "0.0.44");
    config.discovery = None;
    config.direct_bind = "127.0.0.1:0".parse().unwrap();
    config
}

struct Started {
    shell: Arc<VoltipShell>,
    platform: Arc<FakePlatform>,
    runtime: Option<tokio::runtime::Runtime>,
    _dir: tempfile::TempDir,
}

impl Started {
    fn new(platform: FakePlatform) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().worker_threads(2).build().unwrap();
        let platform = Arc::new(platform);
        let host: Arc<dyn PlatformHost> = platform.clone();
        let shell = VoltipShell::start_in(
            runtime.handle().clone(),
            offline_config(dir.path()),
            Arc::new(MemorySecretStore::new()),
            |_| voltip_core::dictation::fakes::ports(),
            host,
        )
        .unwrap();
        let started = Self { shell, platform, runtime: Some(runtime), _dir: dir };
        started.wait(|| started.state()["identity"].is_object());
        started
    }

    /// `invoke` as the app's coroutine awaits it, driven here by a small executor of its own: the
    /// command itself runs on the shell's runtime.
    fn invoke(&self, command: &str, args: &str) -> Result<String, ShellError> {
        let shell = self.shell.clone();
        let (command, args) = (command.to_owned(), args.to_owned());
        let driver = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        driver.block_on(async move { tokio::time::timeout(STEP_TIMEOUT, shell.invoke(command, args)).await.expect("the shell answered in time") })
    }

    fn state(&self) -> Value {
        serde_json::from_str(&self.invoke("core_state", "").unwrap()).unwrap()
    }

    fn wait(&self, done: impl Fn() -> bool) {
        let deadline = Instant::now() + STEP_TIMEOUT;
        while !done() {
            assert!(Instant::now() < deadline, "condition not met within {STEP_TIMEOUT:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Started {
    fn drop(&mut self) {
        self.shell.shell().shutdown();
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(Duration::from_secs(2));
        }
    }
}

#[test]
fn commands_answer_json_or_the_tauri_shells_error_text() {
    let started = Started::new(FakePlatform::default());
    let state = started.state();
    assert_eq!(state["settings"]["relay_enabled"], false, "{state}");
    // Empty arguments and `null` both mean none.
    let devices: Value = serde_json::from_str(&started.invoke("audio_devices", "null").unwrap()).unwrap();
    assert_eq!(devices, serde_json::json!([]));
    let err = started.invoke("core_state_v2", "").unwrap_err();
    assert!(matches!(&err, ShellError::Command(text) if text == "unknown command core_state_v2"), "{err:?}");
    let err = started.invoke("device_rename", "{not json").unwrap_err();
    assert!(err.to_string().starts_with("invalid args json:"), "{err}");
    let err = started.invoke("device_rename", r#"["Studio"]"#).unwrap_err();
    assert!(err.to_string().starts_with("invalid args for device_rename"), "{err}");
}

/// The core's events reach the platform as JSON, the update status `disabled` among them.
#[test]
fn events_reach_the_platform_as_json() {
    let started = Started::new(FakePlatform::default());
    started.wait(|| started.platform.events().iter().any(|e| e["type"] == "update" && e["state"] == "disabled"));
}

/// A host that cannot take an event (the JavaScript side not listening yet) misses only that one:
/// the ones after it still arrive.
#[test]
fn a_dropped_event_stops_nothing() {
    let started = Started::new(FakePlatform { drop_events: Mutex::new(1), ..FakePlatform::default() });
    started.wait(|| *started.platform.drop_events.lock().unwrap() == 0);
    started.invoke("device_rename", r#"{"name":"书房的手机"}"#).unwrap();
    started.wait(|| started.platform.events().iter().any(|e| e.to_string().contains("书房的手机")));
}

/// The platform's capabilities go through the host; when it refuses, the command answers with the
/// platform's reason.
#[test]
fn platform_requests_and_their_refusals() {
    let started = Started::new(FakePlatform { clipboard: Mutex::new(Some("剪贴板里的字".into())), ..FakePlatform::default() });
    let read: Value = serde_json::from_str(&started.invoke("phone_clipboard_read", "").unwrap()).unwrap();
    assert_eq!(read, serde_json::json!({ "text": "剪贴板里的字" }));
    started.invoke("phone_share_text", r#"{"text":"今天下午三点开会"}"#).unwrap();
    assert!(started.platform.calls.lock().unwrap().contains(&"share_text 今天下午三点开会".to_owned()));
    *started.platform.refuse.lock().unwrap() = Some("no app takes the text".into());
    let err = started.invoke("phone_share_text", r#"{"text":"x"}"#).unwrap_err();
    assert!(err.to_string().contains("no app takes the text"), "{err}");
}
