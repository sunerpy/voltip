#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The desktop updater against the real `tauri-plugin-updater` on the mock runtime, with a local
//! manifest server standing in for the release host: check → up to date / available, download →
//! the package is refused because it is not signed with the configured key, automatic mode at
//! startup and on toggle, one run at a time. Installing needs a genuinely signed package and an
//! installed bundle to replace, so it stays out of reach here (and of the mock runtime).

use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder, mock_context, noop_assets};
use tauri::webview::InvokeRequest;
use tauri::{AppHandle, Listener as _, Manager as _, WebviewWindow};
use voltip_core::dictation::fakes;
use voltip_core::ui::{UI_EVENT_NAME, UiState, UpdateStatus};
use voltip_core::{CoreConfig, Settings, SettingsStore};
use voltip_desktop_lib::update::{BUSY, MARKER_FILE_NAME, UpdateSlot, UpdaterConfig};
use voltip_desktop_lib::{ShellOptions, build_app};
use voltip_identity::MemorySecretStore;
use voltip_tauri_bridge::Bridge;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const STEP_TIMEOUT: Duration = Duration::from_secs(20);
const POLL: Duration = Duration::from_millis(20);
const MANIFEST_PATH: &str = "/latest.json";
const PACKAGE_PATH: &str = "/voltip-9.9.9.pkg";
const NEW_VERSION: &str = "9.9.9";
const NOTES: &str = "修复听写热键冲突";
const PUB_DATE: &str = "2026-09-25T08:00:00Z";
/// A structurally valid minisign public key (the base64 of a `.pub` file) that signed nothing.
const PUBKEY: &str =
    "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDA4MDcwNjA1MDQwMzAyMDEKUldRQkFnTUVCUVlIQ0JFUkVSRVJFUkVSRVJFUkVSRVJFUkVSRVJFUkVSRVJFUkVSRVJFUkVSRVIK";
/// A structurally valid minisign signature (the base64 of a `.sig` file) that does not verify.
const SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVRQkFnTUVCUVlIQ0NJaUlpSWlJaUlpSWlJaUlpSWlJaUlpSWlJaUlpSWlJaUlpSWlJaUlpSWlJaUlpSWlJaUlpSWlJaUlpSWlJaUlpSWlJaUlpSWlJaUlpSWlJaUlpSWlJPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzU4NzAwMDAwCWZpbGU6dm9sdGlwLTkuOS45LnBrZwl2ZXJzaW9uOjkuOS45Ck16TXpNek16TXpNek16TXpNek16TXpNek16TXpNek16TXpNek16TXpNek16TXpNek16TXpNek16TXpNek16TXpNek16TXpNek16TXpNek16TXpNek13PT0K";

type Body = Box<dyn FnOnce(&AppHandle<MockRuntime>, &WebviewWindow<MockRuntime>, &mpsc::Receiver<String>) + Send>;

fn request(cmd: &str, args: Value) -> InvokeRequest {
    InvokeRequest {
        cmd: cmd.into(),
        callback: CallbackFn(0),
        error: CallbackFn(1),
        url: if cfg!(any(windows, target_os = "android")) { "http://tauri.localhost" } else { "tauri://localhost" }.parse().unwrap(),
        body: InvokeBody::Json(args),
        headers: Default::default(),
        invoke_key: INVOKE_KEY.to_owned(),
    }
}

fn invoke(webview: &WebviewWindow<MockRuntime>, cmd: &str, args: Value) -> Result<Value, Value> {
    get_ipc_response(webview, request(cmd, args)).map(|body| body.deserialize::<Value>().unwrap())
}

fn core_state(webview: &WebviewWindow<MockRuntime>) -> UiState {
    serde_json::from_value(invoke(webview, "core_state", json!({})).unwrap()).unwrap()
}

fn update_status(webview: &WebviewWindow<MockRuntime>) -> UpdateStatus {
    serde_json::from_value(invoke(webview, "update_status", json!({})).unwrap()).unwrap()
}

fn wait_for<T>(mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + STEP_TIMEOUT;
    loop {
        if let Some(v) = probe() {
            return v;
        }
        assert!(Instant::now() < deadline, "condition not met within {STEP_TIMEOUT:?}");
        std::thread::sleep(POLL);
    }
}

fn wait_state(webview: &WebviewWindow<MockRuntime>, mut pred: impl FnMut(&UiState) -> bool) -> UiState {
    wait_for(|| {
        let st = core_state(webview);
        pred(&st).then_some(st)
    })
}

/// Next `update` event whose `state` is `state`. Other event types are skipped; an `update` with
/// a state outside `allow_between` fails the test, because the sequence is part of the contract.
fn wait_update(rx: &mpsc::Receiver<String>, state: &str, allow_between: &[&str]) -> Value {
    let deadline = Instant::now() + STEP_TIMEOUT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let raw = rx.recv_timeout(left).unwrap_or_else(|e| panic!("no `update/{state}` event within {STEP_TIMEOUT:?}: {e}"));
        let json: Value = serde_json::from_str(&raw).unwrap();
        // `idle` is the frame `attach_bridge` publishes before anything ran.
        if json["type"] != "update" || json["state"] == "idle" {
            continue;
        }
        if json["state"] == state {
            return json;
        }
        assert!(allow_between.iter().any(|s| json["state"] == *s), "expected update/{state}, got {json}");
    }
}

/// Manifest in the static `latest.json` layout the Tauri CLI writes, every desktop target pointing
/// at the same (unsigned) package on the mock server.
fn manifest(server: &MockServer) -> ResponseTemplate {
    let platform = json!({ "url": format!("{}{PACKAGE_PATH}", server.uri()), "signature": SIGNATURE });
    ResponseTemplate::new(200).set_body_json(json!({
        "version": NEW_VERSION,
        "notes": NOTES,
        "pub_date": PUB_DATE,
        "platforms": {
            "linux-x86_64": platform, "linux-aarch64": platform,
            "windows-x86_64": platform, "windows-aarch64": platform,
            "darwin-x86_64": platform, "darwin-aarch64": platform,
        }
    }))
}

/// "Nothing newer": the manifest endpoint answers 204.
fn no_update(_: &MockServer) -> ResponseTemplate {
    ResponseTemplate::new(204)
}

/// Build the shell on the mock runtime with the real updater plugin pointed at a local manifest
/// server, run the event loop on this thread and drive it from `body` on a helper thread.
/// `prepare` runs against the data dir before the app starts and returns the settings to save.
fn with_updater_app(respond: fn(&MockServer) -> ResponseTemplate, prepare: impl FnOnce(&Path) -> Settings, body: Body) {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let server = rt.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path(MANIFEST_PATH)).respond_with(respond(&server)).mount(&server).await;
        Mock::given(method("GET")).and(path(PACKAGE_PATH)).respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0x56; 4096])).mount(&server).await;
        server
    });
    let dir = tempfile::tempdir().unwrap();
    let settings = prepare(dir.path());
    SettingsStore::new(dir.path()).save(&Settings { relay_enabled: false, ..settings }).unwrap();
    let mut core = CoreConfig::new(dir.path().to_path_buf());
    core.default_device_name = "Update Test".into();
    core.direct_bind = "127.0.0.1:0".parse().unwrap();
    let config = UpdaterConfig {
        endpoint: format!("{}{MANIFEST_PATH}", server.uri()).parse().unwrap(),
        pubkey: PUBKEY.into(),
        auto_check_delay: Duration::from_millis(200),
    };
    // What `run()` does in production: the plugin block comes from the build, not from a file.
    let mut context = mock_context(noop_assets());
    context.config_mut().plugins.0.insert("updater".to_owned(), config.plugin_config());
    let app = build_app(
        mock_builder().plugin(tauri_plugin_updater::Builder::new().build()).manage(config),
        core,
        Arc::new(MemorySecretStore::new()),
        ShellOptions::HEADLESS,
        voltip_desktop_lib::dictation::ShellPorts::headless(fakes::ports()),
    )
    .build(context)
    .unwrap();
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
    let (tx, rx) = mpsc::channel::<String>();
    app.listen_any(UI_EVENT_NAME, move |ev| {
        let _ = tx.send(ev.payload().to_owned());
    });
    let handle = app.handle().clone();
    let driver = std::thread::spawn(move || {
        let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
            wait_for(|| handle.try_state::<Bridge>().map(|_| ()));
            body(&handle, &webview, &rx);
        }));
        webview.close().unwrap();
        outcome
    });
    app.run(|_, _| {});
    if let Err(panic) = driver.join().unwrap() {
        std::panic::resume_unwind(panic);
    }
    drop(rt);
}

/// `update_check` asks the manifest and reports `checking` → `up_to_date` with the running version;
/// `update_install` with nothing pending checks first and lands there too.
#[test]
fn check_against_a_204_manifest_reports_up_to_date() {
    with_updater_app(
        no_update,
        |_| Settings::default(),
        Box::new(|app, webview, rx| {
            let st = wait_state(webview, |s| s.identity.is_some());
            assert_eq!(st.update, UpdateStatus::Idle, "configured but not checked yet");
            assert_eq!(update_status(webview), UpdateStatus::Idle);
            assert!(app.state::<Arc<UpdateSlot>>().enabled());
            assert_eq!(invoke(webview, "update_check", json!({})), Ok(Value::Null));
            wait_update(rx, "checking", &[]);
            let ev = wait_update(rx, "up_to_date", &["checking"]);
            assert_eq!(ev["version"], app.package_info().version.to_string());
            assert!(ev["checked_at"].as_u64().unwrap() > 1_700_000_000);
            let st = wait_state(webview, |s| matches!(s.update, UpdateStatus::UpToDate { .. }));
            assert_eq!(update_status(webview), st.update, "the query and the cached state agree");
            assert_eq!(invoke(webview, "update_install", json!({})), Ok(Value::Null));
            wait_update(rx, "checking", &[]);
            wait_update(rx, "up_to_date", &["checking"]);
        }),
    );
}

/// A newer version in the manifest: `available` carries the version, the running version, the
/// notes and the date; `update_install` reuses it (no second `checking`), downloads it
/// (`downloading` from 0) and refuses the package because the signature is not the configured
/// key's, so the run ends in `failed`, nothing is installed and nothing is remembered as `Ready`.
/// The slot is free afterwards and the next explicit check starts from the manifest again.
#[test]
fn available_update_is_downloaded_and_refused_when_the_signature_does_not_verify() {
    with_updater_app(
        manifest,
        |_| Settings::default(),
        Box::new(|app, webview, rx| {
            wait_state(webview, |s| s.identity.is_some());
            assert_eq!(invoke(webview, "update_check", json!({})), Ok(Value::Null));
            wait_update(rx, "checking", &[]);
            let ev = wait_update(rx, "available", &["checking"]);
            assert_eq!(ev["version"], NEW_VERSION);
            assert_eq!(ev["current"], app.package_info().version.to_string());
            assert_eq!(ev["notes"], NOTES);
            assert_eq!(ev["date"], PUB_DATE);
            assert!(matches!(update_status(webview), UpdateStatus::Available { ref version, .. } if version == NEW_VERSION));
            assert_eq!(invoke(webview, "update_install", json!({})), Ok(Value::Null));
            let first = wait_update(rx, "downloading", &[]);
            assert_eq!(first["version"], NEW_VERSION);
            assert_eq!(first["received"], 0);
            let failed = wait_update(rx, "failed", &["downloading"]);
            let message = failed["message"].as_str().unwrap();
            assert!(!message.is_empty(), "{failed}");
            assert!(matches!(update_status(webview), UpdateStatus::Failed { .. }));
            assert_eq!(app.state::<Arc<UpdateSlot>>().read_marker(), None, "a refused package is never remembered as Ready");
            assert_eq!(invoke(webview, "update_check", json!({})), Ok(Value::Null));
            wait_update(rx, "checking", &[]);
            wait_update(rx, "available", &["checking"]);
        }),
    );
}

/// Auto mode at startup (saved `auto_update: true`): the check runs after the (shortened) delay
/// without any command, and with the found version remembered as `Ready` from an "earlier run" the
/// intent is to install — which the unsigned package turns into `failed` before anything happens.
#[test]
fn auto_update_on_at_startup_checks_after_the_delay_and_honours_the_marker() {
    with_updater_app(
        manifest,
        |dir| {
            std::fs::write(dir.join(MARKER_FILE_NAME), format!(r#"{{"version":"{NEW_VERSION}"}}"#)).unwrap();
            Settings { auto_update: true, ..Settings::default() }
        },
        Box::new(|app, webview, rx| {
            let st = wait_state(webview, |s| s.identity.is_some());
            assert!(st.settings.auto_update);
            assert_eq!(app.state::<Arc<UpdateSlot>>().read_marker(), Some(NEW_VERSION.into()));
            wait_update(rx, "checking", &[]);
            wait_update(rx, "available", &["checking"]);
            wait_update(rx, "downloading", &["available"]);
            wait_update(rx, "failed", &["downloading"]);
            assert!(matches!(update_status(webview), UpdateStatus::Failed { .. }));
        }),
    );
}

/// Turning the toggle on while running triggers an immediate check that downloads only; a manual
/// check while that run is in flight is refused as busy. Turning it off checks nothing.
#[test]
fn toggling_auto_update_on_checks_immediately_and_runs_one_at_a_time() {
    with_updater_app(
        manifest,
        |_| Settings::default(),
        Box::new(|_, webview, rx| {
            wait_state(webview, |s| s.identity.is_some());
            assert_eq!(invoke(webview, "settings_set_auto_update", json!({ "enabled": true })), Ok(Value::Null));
            wait_state(webview, |s| s.settings.auto_update);
            wait_update(rx, "checking", &[]);
            // The slot is claimed by the automatic run until it finishes.
            let busy = match invoke(webview, "update_check", json!({})) {
                Err(Value::String(e)) if e == BUSY => true,
                Ok(Value::Null) => false,
                other => panic!("{other:?}"),
            };
            wait_update(rx, "available", &["checking"]);
            wait_update(rx, "downloading", &["available"]);
            wait_update(rx, "failed", &["downloading"]);
            if !busy {
                // The automatic run had already finished when the manual check landed: it ran after.
                wait_update(rx, "checking", &[]);
                wait_update(rx, "available", &["checking"]);
            }
            assert_eq!(invoke(webview, "settings_set_auto_update", json!({ "enabled": false })), Ok(Value::Null));
            wait_state(webview, |s| !s.settings.auto_update);
            std::thread::sleep(Duration::from_millis(300));
            while let Ok(raw) = rx.try_recv() {
                let json: Value = serde_json::from_str(&raw).unwrap();
                assert!(json["type"] != "update" || json["state"] != "checking", "turning the toggle off must not check: {json}");
            }
        }),
    );
}
