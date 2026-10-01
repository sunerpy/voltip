#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Command-layer tests for the mobile shell on Tauri's mock runtime (desktop host build): the real
//! `#[tauri::command]`s, setup hook and event forwarding, without a window, a keystore or a keychain.

use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder, mock_context, noop_assets};
use tauri::webview::InvokeRequest;
use tauri::{AppHandle, Listener as _, Manager as _, WebviewWindow};
use voltip_core::dictation::DictationPorts;
use voltip_core::ui::{UI_EVENT_NAME, UiState};
use voltip_core::{CoreConfig, Settings, SettingsStore, ThemeId};
use voltip_identity::MemorySecretStore;
use voltip_mobile_lib::{
    BROWSER_UNAVAILABLE, COMMANDS, FEEDBACK_UNAVAILABLE, HOTKEY_UNAVAILABLE, KEYSTORE_SERVICE, MODELS_UNAVAILABLE, UPDATE_UNAVAILABLE, build_app, data_dir,
    platform_label, production_config, secret_store,
};
use voltip_pairing::PairingState;
use voltip_tauri_bridge::Bridge;

const STEP_TIMEOUT: Duration = Duration::from_secs(15);
const POLL: Duration = Duration::from_millis(20);
const DEVICE_NAME: &str = "Phone Test";
const NOT_FOUND: &str = "not found";

/// Offline core with `settings` written first: LAN host on an ephemeral loopback port, a phone as
/// in `production_config` (the callers turn the relay off in `settings`).
fn offline_config(dir: &Path, settings: Settings) -> CoreConfig {
    SettingsStore::new(dir).save(&settings).unwrap();
    let mut cfg = CoreConfig::new(dir.to_path_buf());
    cfg.default_device_name = DEVICE_NAME.into();
    cfg.direct_bind = "127.0.0.1:0".parse().unwrap();
    cfg.accepts_phone_takes = false;
    cfg.manual_scenes = true;
    cfg
}

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

/// Invoke a command exactly as `@tauri-apps/api` would: JSON body, command name, IPC key.
fn invoke(webview: &WebviewWindow<MockRuntime>, cmd: &str, args: Value) -> Result<Value, Value> {
    get_ipc_response(webview, request(cmd, args)).map(|body| body.deserialize::<Value>().unwrap())
}

fn core_state(webview: &WebviewWindow<MockRuntime>) -> UiState {
    serde_json::from_value(invoke(webview, "core_state", json!({})).unwrap()).unwrap()
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

/// Wait for a forwarded `voltip://event` whose JSON satisfies `pred`; `what` names it on failure.
fn wait_event(rx: &mpsc::Receiver<String>, what: &str, mut pred: impl FnMut(&Value) -> bool) -> Value {
    let deadline = Instant::now() + STEP_TIMEOUT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let raw = rx.recv_timeout(left).unwrap_or_else(|e| panic!("no `{what}` event on the webview bus within {STEP_TIMEOUT:?}: {e}"));
        let json: Value = serde_json::from_str(&raw).expect("events are JSON");
        if pred(&json) {
            return json;
        }
    }
}

/// Build the app on the mock runtime, run its event loop on this thread (which is where Tauri
/// executes the `setup` hook) and drive it from `body` on a helper thread.
fn with_running_app(body: impl FnOnce(&AppHandle<MockRuntime>, &WebviewWindow<MockRuntime>, &mpsc::Receiver<String>) + Send + 'static) {
    with_app(Settings { relay_enabled: false, ..Settings::default() }, |_| voltip_core::dictation::fakes::ports(), body);
}

/// [`with_running_app`] with `settings` and the dictation `ports`.
fn with_app(
    settings: Settings,
    ports: impl FnOnce(&AppHandle<MockRuntime>) -> DictationPorts + Send + 'static,
    body: impl FnOnce(&AppHandle<MockRuntime>, &WebviewWindow<MockRuntime>, &mpsc::Receiver<String>) + Send + 'static,
) {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().to_path_buf();
    let app = build_app(mock_builder(), move |_| offline_config(&data, settings), Arc::new(MemorySecretStore::new()), ports)
        .build(mock_context(noop_assets()))
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
        // Closing the only window ends the mock event loop, so `app.run` below returns.
        webview.close().unwrap();
        outcome
    });
    app.run(|_, _| {});
    if let Err(panic) = driver.join().unwrap() {
        std::panic::resume_unwind(panic);
    }
}

#[test]
fn core_state_reports_identity_and_the_setup_hook_forwards_events() {
    with_running_app(|_, webview, rx| {
        let st = wait_state(webview, |s| s.identity.is_some());
        assert_eq!(st.identity.unwrap().name, DEVICE_NAME);
        assert_eq!(st.secret_backend, "memory");
        assert!(!st.settings.relay_enabled);
        assert_eq!(st.pairing.state, PairingState::Idle);
        let ev = wait_event(rx, "state", |e| e["type"] == "state");
        assert_eq!(ev["identity"]["name"], DEVICE_NAME);
        assert!(ev["identity"]["public_key"].as_str().unwrap().len() == 64, "keys are hex on the wire");
    });
}

#[test]
fn pairing_start_leaves_idle_and_cancel_reset_returns_to_it() {
    with_running_app(|_, webview, rx| {
        wait_state(webview, |s| s.identity.is_some());
        assert_eq!(invoke(webview, "pairing_start", json!({})), Ok(Value::Null));
        let st = wait_state(webview, |s| s.pairing.state != PairingState::Idle);
        assert!(matches!(st.pairing.state, PairingState::CreatingSession | PairingState::WaitingForPeer), "{:?}", st.pairing.state);
        let waiting = wait_state(webview, |s| s.pairing.state == PairingState::WaitingForPeer);
        assert!(waiting.pairing.code.is_some(), "LAN pairing still shows a code");
        assert!(waiting.pairing.ticket_uri.as_deref().unwrap_or_default().starts_with("voltip://pair?"));
        wait_event(rx, "pairing/waiting_for_peer", |e| e["type"] == "pairing" && e["state"]["state"] == "waiting_for_peer");
        assert_eq!(invoke(webview, "pairing_cancel", json!({})), Ok(Value::Null));
        wait_state(webview, |s| s.pairing.state.is_terminal());
        assert_eq!(invoke(webview, "pairing_reset", json!({})), Ok(Value::Null));
        wait_state(webview, |s| s.pairing.state == PairingState::Idle);
        // Without a relay a bare code cannot be joined: the command is accepted, the core reports.
        assert_eq!(invoke(webview, "pairing_join_code", json!({ "code": "483 921" })), Ok(Value::Null));
        let err = wait_event(rx, "error", |e| e["type"] == "error");
        assert!(err["message"].as_str().unwrap().contains("中继"), "{err}");
        // A ticket is accepted by the IPC layer too; a malformed one is reported by the core.
        assert_eq!(invoke(webview, "pairing_join_ticket", json!({ "uri": "voltip://pair?v=1&t=AA" })), Ok(Value::Null));
        wait_event(rx, "error (bad ticket)", |e| e["type"] == "error" && e["message"] != err["message"]);
    });
}

#[test]
fn argument_validation_errors_come_back_as_strings() {
    with_running_app(|_, webview, _| {
        wait_state(webview, |s| s.identity.is_some());
        let err = invoke(webview, "device_forget", json!({ "publicKey": "not-hex" })).unwrap_err();
        assert!(err.as_str().unwrap().contains("64 hex"), "{err}");
        let err = invoke(webview, "send_text", json!({ "publicKey": "zz", "body": "hi" })).unwrap_err();
        assert!(err.as_str().unwrap().contains("64 hex"), "{err}");
        // A missing argument is rejected by Tauri before the command runs.
        let err = invoke(webview, "pairing_join_ticket", json!({})).unwrap_err();
        assert!(err.as_str().unwrap().contains("uri"), "{err}");
        // Unknown command names are refused, which is what the list test below relies on.
        let err = invoke(webview, "no_such_command", json!({})).unwrap_err();
        assert!(err.as_str().unwrap().contains(NOT_FOUND), "{err}");
    });
}

#[test]
fn settings_and_identity_commands_change_state() {
    with_running_app(|_, webview, rx| {
        wait_state(webview, |s| s.identity.is_some());
        assert_eq!(invoke(webview, "settings_set_theme", json!({ "theme": "dark", "followSystem": true })), Ok(Value::Null));
        let st = wait_state(webview, |s| s.settings.theme == ThemeId::Dark);
        assert!(st.settings.follow_system_theme);
        wait_event(rx, "settings", |e| e["type"] == "settings" && e["theme"] == "dark");
        assert_eq!(invoke(webview, "settings_set_relay", json!({ "url": null, "enabled": false })), Ok(Value::Null));
        assert_eq!(invoke(webview, "device_rename", json!({ "name": "Studio" })), Ok(Value::Null));
        wait_state(webview, |s| s.identity.as_ref().is_some_and(|i| i.name == "Studio"));
        assert_eq!(invoke(webview, "devices_refresh", json!({})), Ok(Value::Null));
        // The pairing verbs are accepted by the IPC layer even when nothing is pairing.
        for cmd in ["pairing_confirm", "pairing_reject"] {
            assert_eq!(invoke(webview, cmd, json!({})), Ok(Value::Null), "{cmd}");
        }
    });
}

/// docs/pairing.md 「常开配对」: only a desktop keeps a pairing open; the phone's core says so.
#[test]
fn always_on_pairing_is_refused_on_the_phone() {
    with_running_app(|_, webview, rx| {
        wait_state(webview, |s| s.identity.is_some());
        assert_eq!(invoke(webview, "settings_set_pairing_always_on", json!({ "enabled": true })), Ok(Value::Null));
        wait_event(rx, "error", |e| e["type"] == "error" && e["message"].as_str().is_some_and(|m| m.contains("常开配对只在电脑上可用")));
        assert!(!core_state(webview).settings.pairing_always_on);
    });
}

/// docs/pairing.md 「局域网发现」: the switch persists and comes back in `settings`; joining a
/// device the LAN browse has not seen is the core's error; both need their argument.
#[test]
fn lan_discovery_commands_reach_the_core() {
    with_running_app(|_, webview, rx| {
        wait_state(webview, |s| s.identity.is_some());
        assert!(core_state(webview).settings.lan_discovery, "on by default");
        assert!(invoke(webview, "settings_set_lan_discovery", json!({})).is_err(), "enabled is required");
        assert_eq!(invoke(webview, "settings_set_lan_discovery", json!({ "enabled": false })), Ok(Value::Null));
        wait_event(rx, "settings", |e| e["type"] == "settings" && e["lan_discovery"] == false);
        assert!(!wait_state(webview, |s| !s.settings.lan_discovery).settings.lan_discovery);
        assert!(invoke(webview, "pairing_join_nearby", json!({})).is_err(), "fingerprint is required");
        assert_eq!(invoke(webview, "pairing_join_nearby", json!({ "fingerprint": "0000000000000000" })), Ok(Value::Null));
        wait_event(rx, "error", |e| e["type"] == "error" && e["message"].as_str().is_some_and(|m| m.contains("附近没有找到此设备")));
    });
}

/// `UiEvent::Devices` must be a struct variant (`{ "type": "devices", "devices": [...] }`): serde
/// refuses to internally tag a newtype variant holding a list, `emit` fails, and the webview never
/// sees a device list. `packages/shared/src/schema.ts` already expects the wrapped form.
#[test]
fn regression_devices_event_reaches_the_webview_bus() {
    with_running_app(|_, webview, rx| {
        wait_state(webview, |s| s.identity.is_some());
        assert_eq!(invoke(webview, "devices_refresh", json!({})), Ok(Value::Null));
        let ev = wait_event(rx, "devices", |e| e["type"] == "devices");
        assert!(ev["devices"].is_array(), "{ev}");
    });
}

/// No local pipeline on the phone: the dictation verbs are refused with an honest reason, while
/// the shared engine / secret / history commands work through the bridge like on the desktop.
/// docs/dictation.md §20: the phone streams takes to a paired desktop through the core. A key that
/// is not a paired desktop, or a stop / cancel with no take running, is the core's error; a
/// malformed key is an argument error.
#[test]
fn phone_take_commands_reach_the_core() {
    with_running_app(|_, webview, rx| {
        wait_state(webview, |s| s.identity.is_some());
        assert!(invoke(webview, "phone_take_start", json!({ "publicKey": "zz" })).is_err());
        assert!(invoke(webview, "phone_take_start", json!({})).is_err(), "publicKey is required");
        assert_eq!(invoke(webview, "phone_take_start", json!({ "publicKey": "11".repeat(32) })), Ok(Value::Null));
        wait_event(rx, "error", |e| e["type"] == "error" && e["message"].as_str().is_some_and(|m| m.contains("未知设备")));
        for cmd in ["phone_take_stop", "phone_take_cancel"] {
            assert_eq!(invoke(webview, cmd, json!({})), Ok(Value::Null), "{cmd}");
            wait_event(rx, "error", |e| e["type"] == "error" && e["message"].as_str().is_some_and(|m| m.contains("没有进行中的录音")));
        }
        assert!(core_state(webview).phone_take.is_none());
    });
}

/// docs/dictation.md §20.6: the phone's texts go through the core (an unknown desktop is the
/// docs/dictation.md §20.7 (user request 2026-09-30): with no paired desktop online the phone
/// recognises a take itself. `dictation_start` / `dictation_stop` reach the core, the real cloud
/// clients the phone's factory builds (`voltip-cloud`) post the take to the recogniser and the
/// clean-up — local fake servers here, the built-in services in a release — and the result goes
/// to the phone's injector (its clipboard) and into its own history; `dictation_cancel` discards.
#[test]
fn a_take_on_the_phone_runs_through_the_cloud_clients_onto_its_clipboard() {
    use voltip_core::dictation::fakes::{FakeAudio, FakeInjector};
    use voltip_core::{EngineSettings, ProviderId, ProviderSettings};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/transcriptions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "text": "今天下午三点开会" })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "model": "clean-up",
                "choices": [{ "message": { "role": "assistant", "content": "今天下午三点开会。" } }]
            })))
            .mount(&server)
            .await;
        server
    });
    let engines = EngineSettings {
        asr_provider: ProviderId::Custom,
        llm_provider: ProviderId::Custom,
        providers: [(
            ProviderId::Custom,
            ProviderSettings {
                asr_url: Some(server.uri()),
                asr_model: Some("asr".into()),
                llm_url: Some(format!("{}/v1", server.uri())),
                llm_model: Some("clean-up".into()),
            },
        )]
        .into(),
        refine_enabled: true,
        ..EngineSettings::default()
    };
    let injector = Arc::new(FakeInjector::clipboard(None));
    let delivered = injector.clone();
    let ports = move |_: &AppHandle<MockRuntime>| DictationPorts {
        audio: Arc::new(FakeAudio::speech()),
        injector,
        factory: Arc::new(|engines: &voltip_core::ResolvedEngines| (voltip_cloud::remote_transcriber(engines), voltip_cloud::refiner(engines))),
        ..voltip_core::dictation::fakes::ports()
    };
    with_app(Settings { relay_enabled: false, engines, ..Settings::default() }, ports, move |_, webview, _| {
        wait_state(webview, |s| s.identity.is_some());
        assert_eq!(invoke(webview, "dictation_start", json!({})), Ok(Value::Null));
        wait_state(webview, |s| matches!(s.dictation.phase, voltip_core::DictationPhase::Listening { .. }));
        assert_eq!(invoke(webview, "dictation_stop", json!({})), Ok(Value::Null));
        let st = wait_state(webview, |s| matches!(s.dictation.phase, voltip_core::DictationPhase::Done { .. } | voltip_core::DictationPhase::Failed { .. }));
        let phase = serde_json::to_value(&st.dictation.phase).unwrap();
        assert_eq!(phase["phase"], "done", "{phase}");
        assert_eq!((phase["text"].as_str(), phase["raw_text"].as_str()), (Some("今天下午三点开会。"), Some("今天下午三点开会")), "{phase}");
        assert_eq!(phase["via"], "clipboard", "the phone's result is on its clipboard: {phase}");
        assert_eq!(delivered.injected(), vec!["今天下午三点开会。".to_owned()]);
        // The take is in the phone's own history, as the state and the queries show it.
        let st = wait_state(webview, |s| !s.history_recent.is_empty());
        assert_eq!(st.history_recent[0].text, "今天下午三点开会。");
        let page = invoke(webview, "history_query", json!({ "limit": 10 })).unwrap();
        assert_eq!((page["total"].as_u64(), page["entries"][0]["text"].as_str()), (Some(1), Some("今天下午三点开会。")), "{page}");
        let id = page["entries"][0]["id"].clone();
        assert_eq!(invoke(webview, "history_entry", json!({ "id": id })).unwrap()["refined"], true);
        // 分享文本 / 分享字幕 (§22, user decision 2026-10-01: the phone has the desktop's history): an
        // export goes to the share sheet, which this host build does not have; a whole take has no
        // subtitles, and a deleted entry has nothing to export.
        let shared = invoke(webview, "history_export", json!({ "id": id, "format": "txt", "fileName": "Voltip 2026-10-02 10.00" })).unwrap();
        assert_eq!((shared["kind"].as_str(), shared["code"].as_str()), (Some("failed"), Some("share")), "{shared}");
        assert!(shared["detail"].as_str().is_some_and(|d| d.starts_with("share: ")), "{shared}");
        let subtitles = invoke(webview, "history_export", json!({ "id": id, "format": "srt", "fileName": "x" })).unwrap();
        assert_eq!(subtitles["code"], "empty", "{subtitles}");
        let gone = invoke(webview, "history_export", json!({ "id": "9b1deb4d-3b7d-4bad-9bdd-2b0d7b3dcb6d", "format": "txt", "fileName": "x" })).unwrap();
        assert_eq!(gone["code"], "gone", "{gone}");
        // One request to each service.
        let requests = rt.block_on(server.received_requests()).unwrap();
        let paths: Vec<&str> = requests.iter().map(|r| r.url.path()).collect();
        assert_eq!(paths, ["/v1/audio/transcriptions", "/v1/chat/completions"]);
        // A take discarded before it is recognised reaches no service.
        wait_state(webview, |s| matches!(s.dictation.phase, voltip_core::DictationPhase::Idle));
        assert_eq!(invoke(webview, "dictation_start", json!({})), Ok(Value::Null));
        wait_state(webview, |s| matches!(s.dictation.phase, voltip_core::DictationPhase::Listening { .. }));
        assert_eq!(invoke(webview, "dictation_cancel", json!({})), Ok(Value::Null));
        wait_state(webview, |s| matches!(s.dictation.phase, voltip_core::DictationPhase::Cancelled { .. } | voltip_core::DictationPhase::Idle));
        assert_eq!(rt.block_on(server.received_requests()).unwrap().len(), 2);
        assert_eq!(delivered.injected().len(), 1);
    });
}

/// core's error), the list can be cleared, and a build without a phone clipboard says so.
#[test]
fn phone_text_commands_reach_the_core() {
    with_running_app(|_, webview, rx| {
        wait_state(webview, |s| s.identity.is_some());
        assert!(invoke(webview, "phone_text_send", json!({ "publicKey": "11".repeat(32), "body": "x", "source": "voice" })).is_err(), "unknown source");
        assert_eq!(invoke(webview, "phone_text_send", json!({ "publicKey": "11".repeat(32), "body": "会议改到三点", "source": "typed" })), Ok(Value::Null));
        wait_event(rx, "error", |e| e["type"] == "error" && e["message"].as_str().is_some_and(|m| m.contains("未知设备")));
        assert_eq!(invoke(webview, "sent_texts_clear", json!({})), Ok(Value::Null));
        wait_event(rx, "sent_texts", |e| e["type"] == "sent_texts" && e["texts"].as_array().is_some_and(Vec::is_empty));
        assert_eq!(invoke(webview, "phone_clipboard_read", json!({})), Err(Value::String(voltip_mobile_lib::clipboard::CLIPBOARD_UNAVAILABLE.into())));
    });
}

#[test]
fn hotkeys_are_refused_but_engines_secrets_and_history_work() {
    with_running_app(|_, webview, rx| {
        wait_state(webview, |s| s.identity.is_some());
        // No hotkey on the phone: a key edge is refused (its takes start from the button, see
        // `a_take_on_the_phone_runs_through_the_cloud_clients_onto_its_clipboard`); the activation
        // mode is a shared setting and persists through the core like the desktop's (§13).
        assert_eq!(invoke(webview, "hotkey_edge", json!({ "pressed": true, "atMs": 1, "source": "ui" })), Err(Value::String(HOTKEY_UNAVAILABLE.into())));
        assert!(invoke(webview, "hotkey_edge", json!({})).is_err(), "pressed is required");
        // Voice edit (docs/dictation.md §19): the edit key's edge is refused the same way, its
        // chord is a shared setting the phone edits through the core like the desktop does.
        assert_eq!(invoke(webview, "hotkey_edge", json!({ "pressed": true, "purpose": "edit" })), Err(Value::String(HOTKEY_UNAVAILABLE.into())));
        assert_eq!(invoke(webview, "settings_set_edit_hotkey", json!({ "hotkey": "ctrl+alt+shift+e" })), Ok(Value::Null));
        wait_state(webview, |s| s.settings.edit_hotkey.as_deref() == Some("Ctrl+Alt+Shift+E"));
        assert_eq!(invoke(webview, "settings_set_edit_hotkey", json!({ "hotkey": null })), Ok(Value::Null));
        wait_state(webview, |s| s.settings.edit_hotkey.is_none());
        // The lone-key trigger (§13.1) is a shared setting too; only the desktop watches the key.
        assert_eq!(invoke(webview, "settings_set_solo_key", json!({ "key": "mouse_back" })), Ok(Value::Null));
        wait_state(webview, |s| s.settings.solo_key == Some(voltip_core::SoloKey::MouseBack));
        assert_eq!(invoke(webview, "hotkey_edge", json!({ "pressed": false, "chorded": true })), Err(Value::String(HOTKEY_UNAVAILABLE.into())));
        assert_eq!(
            invoke(webview, "settings_set_activation", json!({ "activation": "toggle", "holdThresholdMs": 300, "extraRecordingMs": 0 })),
            Ok(Value::Null)
        );
        wait_state(webview, |s| s.settings.activation == voltip_core::Activation::Toggle);
        assert!(invoke(webview, "settings_set_activation", json!({ "activation": "press", "holdThresholdMs": 300, "extraRecordingMs": 0 })).is_err());
        // Phones update through the store: the updater is `disabled` everywhere the UI looks, the
        // update verbs refuse honestly, while the locale / auto-update settings persist like on the desktop.
        for cmd in ["update_check", "update_install"] {
            assert_eq!(invoke(webview, cmd, json!({})), Err(Value::String(UPDATE_UNAVAILABLE.into())), "{cmd}");
        }
        assert_eq!(invoke(webview, "update_status", json!({})), Ok(json!({ "state": "disabled" })));
        // docs/dictation.md §15: the phone gates nothing through these queries.
        let report = invoke(webview, "permissions_status", json!({})).unwrap();
        for key in ["microphone", "accessibility"] {
            assert_eq!(report[key], "not_applicable", "{key}");
        }
        assert_eq!(invoke(webview, "permissions_request", json!({ "permission": "microphone" })), Ok(Value::Null));
        assert!(invoke(webview, "permissions_request", json!({ "permission": "camera" })).is_err());
        let preflight = invoke(webview, "inject_preflight", json!({})).unwrap();
        assert_eq!((preflight["checked"].as_bool(), preflight["decision"].as_str()), (Some(false), Some("proceed")));
        // The history's paste button copies on the phone (user request 2026-09-30, docs/dictation.md
        // §20.7): this desktop-hosted build has no phone clipboard, so the copy fails honestly;
        // nothing to copy is `invalid` before any clipboard is asked. Sharing needs the Android
        // share sheet the same way.
        assert_eq!(invoke(webview, "paste_text", json!({ "text": "你好" })), Ok(json!({ "kind": "failed", "reason": "inject" })));
        assert_eq!(invoke(webview, "paste_text", json!({ "text": "  " })), Ok(json!({ "kind": "failed", "reason": "invalid" })));
        assert_eq!(invoke(webview, "phone_share_text", json!({ "text": "你好" })), Err(Value::String(voltip_mobile_lib::share::SHARE_UNAVAILABLE.into())));
        assert!(invoke(webview, "phone_share_text", json!({ "text": " " })).unwrap_err().as_str().unwrap().starts_with("share: 文字为空"));
        assert_eq!(wait_state(webview, |_| true).update, voltip_core::ui::UpdateStatus::Disabled);
        // No local models on a phone: the library verbs refuse and the state carries an empty list.
        for cmd in ["model_download", "model_cancel", "model_remove"] {
            assert_eq!(invoke(webview, cmd, json!({ "id": "sense-voice-small" })), Err(Value::String(MODELS_UNAVAILABLE.into())), "{cmd}");
        }
        assert!(wait_state(webview, |_| true).models.is_empty());
        // The phone's own dictionary, rules and scenes (user decision 2026-10-01: the phone has every
        // setting but the local models; every one of these was refused before).
        assert_eq!(invoke(webview, "dictionary_add", json!({ "entry": { "term": "Voltip", "heard_as": ["沃尔提普"] }, "historyId": null })), Ok(Value::Null));
        let term = wait_state(webview, |s| s.dictionary.len() == 1).dictionary[0].id.to_string();
        assert_eq!(invoke(webview, "rules_add", json!({ "rule": { "name": "句号", "pattern": "。。", "replacement": "。" } })), Ok(Value::Null));
        wait_state(webview, |s| s.rules.len() == 1);
        let preview = invoke(webview, "vocabulary_preview", json!({ "text": "沃尔提普。。", "draft": null })).unwrap();
        assert_eq!(preview["output"], "Voltip。");
        assert!(invoke(webview, "rules_export", json!({})).unwrap().as_str().is_some_and(|t| t.contains("句号")));
        assert_eq!(invoke(webview, "dictionary_remove", json!({ "id": term })), Ok(Value::Null));
        wait_state(webview, |s| s.dictionary.is_empty());
        // Scenes: the built-in ones without applications, the phone's own without one either, and
        // the one its takes run with (`pinned_scene`, the talk card's choice).
        let st = wait_state(webview, |s| !s.scenes.is_empty());
        assert!(st.scenes.iter().all(|s| s.builtin.is_some() && s.matching.apps.is_empty()), "{:?}", st.scenes);
        let builtins = invoke(webview, "scenes_builtin", json!({})).unwrap();
        assert_eq!(builtins.as_array().map(Vec::len), Some(st.scenes.len()));
        assert_eq!(invoke(webview, "scenes_add", json!({ "scene": { "name": "会议", "match": { "apps": [] } } })), Ok(Value::Null));
        let meeting = wait_state(webview, |s| s.scenes.iter().any(|x| x.name == "会议")).scenes.into_iter().find(|x| x.name == "会议").unwrap();
        assert!(meeting.matching.apps.is_empty());
        assert_eq!(invoke(webview, "settings_set_pinned_scene", json!({ "id": meeting.id.to_string() })), Ok(Value::Null));
        wait_state(webview, |s| s.settings.pinned_scene == Some(meeting.id));
        assert!(invoke(webview, "settings_set_pinned_scene", json!({ "id": "nope" })).unwrap_err().as_str().unwrap().contains("UUID"));
        assert_eq!(invoke(webview, "settings_set_pinned_scene", json!({ "id": null })), Ok(Value::Null));
        wait_state(webview, |s| s.settings.pinned_scene.is_none());
        assert_eq!(invoke(webview, "recent_apps", json!({})), Ok(json!([])), "a phone names no application");
        assert_eq!(invoke(webview, "settings_set_context_sharing", json!({ "appName": true, "windowTitle": false })), Ok(Value::Null));
        // The phone cleans up what it recognises itself, with its own presets (user decision
        // 2026-10-01: the phone has every setting but the local models; they were refused before).
        let builtin = invoke(webview, "presets_builtin", json!({})).unwrap();
        assert!(builtin.as_array().is_some_and(|list| !list.is_empty() && list.iter().all(|p| p["prompt"].as_str().is_some_and(|t| !t.is_empty()))));
        assert_eq!(invoke(webview, "presets_add", json!({ "preset": { "name": "周报", "prompt": "整理成周报" } })), Ok(Value::Null));
        let added = wait_state(webview, |s| s.presets.len() == 1).presets[0].id.to_string();
        assert_eq!(invoke(webview, "presets_update", json!({ "id": added, "preset": { "name": "日报", "prompt": "整理成日报" } })), Ok(Value::Null));
        wait_state(webview, |s| s.presets.first().is_some_and(|p| p.name == "日报"));
        assert_eq!(invoke(webview, "presets_try", json!({ "id": 7, "preset": null, "prompt": "整理成日报", "text": "你好" })), Ok(Value::Null));
        wait_event(rx, "preset_try", |e| e["type"] == "preset_try" && e["id"] == 7);
        assert_eq!(invoke(webview, "presets_remove", json!({ "id": added })), Ok(Value::Null));
        wait_state(webview, |s| s.presets.is_empty());
        assert_eq!(invoke(webview, "settings_set_locale", json!({ "locale": "en" })), Ok(Value::Null));
        wait_state(webview, |s| s.settings.locale == voltip_core::Locale::En);
        assert!(invoke(webview, "settings_set_locale", json!({ "locale": "fr" })).is_err());
        assert_eq!(invoke(webview, "settings_set_auto_update", json!({ "enabled": true })), Ok(Value::Null));
        wait_state(webview, |s| s.settings.auto_update);
        assert_eq!(invoke(webview, "provider_key_set", json!({ "provider": "groq", "kind": "llm", "value": "gsk_x" })), Ok(Value::Null));
        let groq_key =
            |s: &UiState| s.engines.providers.iter().find(|p| p.id == voltip_core::ProviderId::Groq).and_then(|p| p.llm.as_ref()).is_some_and(|l| l.key.set);
        let st = wait_state(webview, groq_key);
        assert!(!serde_json::to_string(&st).unwrap().contains("gsk_x"));
        // These ports are the fakes, without an HTTP probe: the core answers `unsupported`, it does
        // not hang (the phone's own probe: `provider_probe_lists_the_models_through_the_http_probe`).
        assert_eq!(invoke(webview, "provider_probe", json!({ "provider": "groq", "kind": "llm" })), Ok(Value::Null));
        wait_event(rx, "provider_probe", |e| e["type"] == "provider_probe" && e["reason"] == "unsupported");
        // Key pages and the repository open in the phone's browser (user decision 2026-10-01; they
        // were refused before). This desktop-hosted build has none and says so; a provider without
        // a key page is refused before any browser is asked.
        assert_eq!(invoke(webview, "provider_console_open", json!({ "provider": "groq" })), Err(Value::String(BROWSER_UNAVAILABLE.into())));
        assert!(invoke(webview, "provider_console_open", json!({ "provider": "local" })).unwrap_err().as_str().unwrap().contains("no key page"));
        assert_eq!(invoke(webview, "project_link_open", json!({ "link": "source" })), Err(Value::String(BROWSER_UNAVAILABLE.into())));
        // Feedback goes from the computer (docs/feedback.md).
        assert_eq!(invoke(webview, "feedback_diagnostics", json!({ "locale": "zh-CN" })), Err(Value::String(FEEDBACK_UNAVAILABLE.into())));
        let report = json!({ "kind": "bug", "message": "x", "contact": null, "locale": "zh-CN" });
        assert_eq!(invoke(webview, "feedback_submit", report), Err(Value::String(FEEDBACK_UNAVAILABLE.into())));
        let engines = json!({ "refine_enabled": false, "inject": "clipboard_only" });
        assert_eq!(invoke(webview, "settings_set_engines", json!({ "engines": engines })), Ok(Value::Null));
        wait_state(webview, |s| !s.engines.refine_enabled);
        wait_event(rx, "engines", |e| e["type"] == "engines" && e["refine_enabled"] == false);
        assert_eq!(invoke(webview, "history_clear", json!({})), Ok(Value::Null));
        wait_event(rx, "history", |e| e["type"] == "history");
        assert!(invoke(webview, "history_delete", json!({ "id": "nope" })).unwrap_err().as_str().unwrap().contains("UUID"));
        assert_eq!(invoke(webview, "history_star", json!({ "id": "9b1deb4d-3b7d-4bad-9bdd-2b0d7b3dcb6d", "starred": true })), Ok(Value::Null));
        assert!(wait_state(webview, |_| true).history_recent.is_empty());
        // The phone's own history (§20.7): empty after the clear, read with the desktop's checks.
        assert_eq!(invoke(webview, "history_query", json!({ "limit": 50 })), Ok(json!({ "entries": [], "matching": 0, "total": 0 })));
        assert!(invoke(webview, "history_query", json!({ "limit": 0 })).is_err());
        assert_eq!(invoke(webview, "history_entry", json!({ "id": "9b1deb4d-3b7d-4bad-9bdd-2b0d7b3dcb6d" })), Ok(Value::Null));
        let stats = invoke(webview, "history_stats", json!({ "boundaries": [0, 10, 20] })).unwrap();
        assert_eq!((stats["buckets"].as_array().map(Vec::len), stats["total"]["count"].as_u64()), (Some(2), Some(0)));
        assert!(invoke(webview, "history_stats", json!({ "boundaries": [5, 5] })).is_err());
        assert_eq!(invoke(webview, "history_hits", json!({})), Ok(json!({ "dictionary": {}, "rules": {} })));
    });
}

/// 测试连接 on the phone (user decision 2026-10-01: it configures its own providers): the phone's
/// HTTP probe lists a provider's models, with the key typed in the draft (docs/dictation.md §3.3).
#[test]
fn provider_probe_lists_the_models_through_the_http_probe() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [{ "id": "whisper-b" }, { "id": "whisper-a" }] })))
            .mount(&server)
            .await;
        server
    });
    let base = format!("{}/v1", server.uri());
    let ports =
        |_: &AppHandle<MockRuntime>| DictationPorts { service_probe: Some(Arc::new(voltip_cloud::HttpServiceProbe)), ..voltip_core::dictation::fakes::ports() };
    with_app(Settings { relay_enabled: false, ..Settings::default() }, ports, move |_, webview, rx| {
        wait_state(webview, |s| s.identity.is_some());
        assert_eq!(invoke(webview, "provider_probe", json!({ "provider": "custom", "kind": "asr", "baseUrl": base, "key": "k" })), Ok(Value::Null));
        let ev = wait_event(rx, "provider_probe", |e| e["type"] == "provider_probe" && e["result"] == "ok");
        assert_eq!(ev["models"], json!(["whisper-a", "whisper-b"]));
    });
    let seen = rt.block_on(server.received_requests()).unwrap();
    assert_eq!(seen[0].headers.get("authorization").unwrap(), "Bearer k", "the draft key is used for the request");
}

// ---------------- command list parity ----------------

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// Keys of the `CommandArgs` interface in `packages/shared/src/schema.ts`.
fn typescript_command_names() -> Vec<String> {
    let src = std::fs::read_to_string(repo_root().join("packages/shared/src/schema.ts")).unwrap();
    let start = src.find("interface CommandArgs {").expect("schema.ts declares CommandArgs");
    let body = &src[start..];
    let end = body.find("\n}").expect("CommandArgs closes");
    // Only the interface's own members: a formatter may break an args object over several lines.
    let mut depth = 0_i32;
    let mut names = Vec::new();
    for line in body[..end].lines().skip(1) {
        let line = line.trim();
        if depth == 0
            && let Some((name, _)) = line.split_once(':')
            && !name.is_empty()
            && name.chars().all(|c| c.is_ascii_lowercase() || c == '_')
        {
            names.push(name.to_owned());
        }
        let opened = i32::try_from(line.matches('{').count()).unwrap_or(0);
        let closed = i32::try_from(line.matches('}').count()).unwrap_or(0);
        depth += opened - closed;
    }
    names
}

/// String literals passed to `invoke(...)` in `packages/shared/src/tauri-backend.ts`.
fn typescript_literal_invokes() -> Vec<String> {
    let src = std::fs::read_to_string(repo_root().join("packages/shared/src/tauri-backend.ts")).unwrap();
    src.match_indices("invoke(\"").map(|(i, pat)| src[i + pat.len()..].split('"').next().unwrap().to_owned()).collect()
}

/// `name` fields of `packages/shared/src/fixtures/ipc/commands.json`.
fn fixture_command_names() -> Vec<String> {
    let text = std::fs::read_to_string(repo_root().join("packages/shared/src/fixtures/ipc/commands.json")).unwrap();
    let entries: Vec<Value> = serde_json::from_str(&text).unwrap();
    entries.iter().map(|e| e["name"].as_str().unwrap().to_owned()).collect()
}

fn sorted(mut names: Vec<String>) -> Vec<String> {
    names.sort();
    names.dedup();
    names
}

/// Commands the Rust side and the fixtures already carry while their `CommandArgs` entry in
/// `packages/shared/src/schema.ts` is still to land (increment B4 shipped Rust + fixtures first).
/// Two-sided like the desktop's: a name here that `schema.ts` now declares fails too.
const PENDING_TYPESCRIPT: &[&str] = &[];

#[test]
fn command_list_matches_the_handlers_the_typescript_contract_and_the_fixtures() {
    let rust = sorted(COMMANDS.iter().map(|s| (*s).to_owned()).collect());
    assert_eq!(rust.len(), COMMANDS.len(), "COMMANDS has duplicates");
    let typescript = sorted(typescript_command_names());
    for name in &typescript {
        assert!(rust.contains(name), "schema.ts CommandArgs declares {name}, which Rust COMMANDS lacks");
    }
    let not_in_typescript: Vec<String> = rust.iter().filter(|n| !typescript.contains(n)).cloned().collect();
    assert_eq!(
        not_in_typescript,
        sorted(PENDING_TYPESCRIPT.iter().map(|s| (*s).to_owned()).collect()),
        "schema.ts CommandArgs keys != Rust COMMANDS (beyond the declared PENDING_TYPESCRIPT set)"
    );
    for name in typescript_literal_invokes() {
        assert!(rust.contains(&name), "tauri-backend.ts invokes unknown command {name}");
    }
    // Queries and streams (`QUERY_COMMANDS` in schema.ts) are not `UiCommand`s and have no fixture entry.
    let mut fixture = fixture_command_names();
    fixture.extend(
        [
            "core_state",
            "audio_devices",
            "audio_outputs",
            "history_export",
            "audio_meter_start",
            "audio_meter_stop",
            "overlay_state",
            "update_status",
            "rules_export",
            "vocabulary_preview",
            "recent_apps",
            "history_query",
            "history_entry",
            "history_stats",
            "history_hits",
            "permissions_status",
            "permissions_request",
            "inject_preflight",
            "paste_text",
            "provider_console_open",
            "project_link_open",
            "feedback_diagnostics",
            "feedback_submit",
            "feedback_attachment_add",
            "feedback_attachment_remove",
            "feedback_attachments_clear",
            "phone_clipboard_read",
            "presets_builtin",
            "scenes_builtin",
        ]
        .map(String::from),
    );
    assert_eq!(sorted(fixture), rust, "fixtures/ipc/commands.json != Rust COMMANDS (minus the query commands)");

    with_running_app(|_, webview, _| {
        wait_state(webview, |s| s.identity.is_some());
        for cmd in COMMANDS {
            // Empty args: either Ok or an argument error, never "not found".
            if let Err(e) = invoke(webview, cmd, json!({})) {
                assert!(!e.as_str().unwrap_or_default().contains(NOT_FOUND), "{cmd} is listed but not registered: {e}");
            }
        }
        let err = invoke(webview, "core_state_v2", json!({})).unwrap_err();
        assert!(err.as_str().unwrap().contains(NOT_FOUND));
    });
}

#[test]
fn production_wiring_helpers_are_well_formed() {
    assert!(!secret_store().backend_name().is_empty());
    assert!(KEYSTORE_SERVICE.starts_with("dev.voltip."));
    assert_eq!(platform_label(), "Voltip", "desktop host build of the mobile shell");
    let app = mock_builder().build(mock_context(noop_assets())).unwrap();
    let dir = data_dir(app.handle());
    assert!(dir.is_absolute(), "{dir:?}");
    let config = production_config(app.handle());
    assert_eq!(config.data_dir, dir);
    assert_eq!(config.default_device_name, "Voltip 手机");
}
