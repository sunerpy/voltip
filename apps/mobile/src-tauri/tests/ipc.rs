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
use voltip_core::ui::{UI_EVENT_NAME, UiState};
use voltip_core::{CoreConfig, Settings, SettingsStore, ThemeId};
use voltip_identity::MemorySecretStore;
use voltip_mobile_lib::{
    COMMANDS, DICTATION_UNAVAILABLE, FEEDBACK_UNAVAILABLE, KEYSTORE_SERVICE, MODELS_UNAVAILABLE, PROJECT_LINKS_UNAVAILABLE, PROVIDERS_UNAVAILABLE,
    SCENES_UNAVAILABLE, UPDATE_UNAVAILABLE, VOCABULARY_UNAVAILABLE, build_app, data_dir, platform_label, production_config, secret_store,
};
use voltip_pairing::PairingState;
use voltip_tauri_bridge::Bridge;

const STEP_TIMEOUT: Duration = Duration::from_secs(15);
const POLL: Duration = Duration::from_millis(20);
const DEVICE_NAME: &str = "Phone Test";
const NOT_FOUND: &str = "not found";

/// Offline core: relay disabled, LAN host on an ephemeral loopback port.
fn offline_config(dir: &Path) -> CoreConfig {
    SettingsStore::new(dir).save(&Settings { relay_enabled: false, ..Settings::default() }).unwrap();
    let mut cfg = CoreConfig::new(dir.to_path_buf());
    cfg.default_device_name = DEVICE_NAME.into();
    cfg.direct_bind = "127.0.0.1:0".parse().unwrap();
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
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().to_path_buf();
    let app = build_app(mock_builder(), move |_| offline_config(&data), Arc::new(MemorySecretStore::new()), voltip_core::dictation::fakes::ports())
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
        assert!(err["message"].as_str().unwrap().contains("relay"), "{err}");
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
        wait_event(rx, "error", |e| e["type"] == "error" && e["message"].as_str().is_some_and(|m| m.contains("unknown device")));
        for cmd in ["phone_take_stop", "phone_take_cancel"] {
            assert_eq!(invoke(webview, cmd, json!({})), Ok(Value::Null), "{cmd}");
            wait_event(rx, "error", |e| e["type"] == "error" && e["message"].as_str().is_some_and(|m| m.contains("没有进行中的录音")));
        }
        assert!(core_state(webview).phone_take.is_none());
    });
}

#[test]
fn dictation_is_refused_but_engines_secrets_and_history_work() {
    with_running_app(|_, webview, rx| {
        wait_state(webview, |s| s.identity.is_some());
        for cmd in ["dictation_start", "dictation_stop", "dictation_cancel"] {
            assert_eq!(invoke(webview, cmd, json!({})), Err(Value::String(DICTATION_UNAVAILABLE.into())), "{cmd}");
        }
        // No hotkey on the phone: a key edge is refused the same way; the activation mode is a
        // shared setting and persists through the core like the desktop's (docs/dictation.md §13).
        assert_eq!(invoke(webview, "hotkey_edge", json!({ "pressed": true, "atMs": 1, "source": "ui" })), Err(Value::String(DICTATION_UNAVAILABLE.into())));
        assert!(invoke(webview, "hotkey_edge", json!({})).is_err(), "pressed is required");
        // Voice edit (docs/dictation.md §19): the edit key's edge is refused the same way, its
        // chord is a shared setting the phone edits through the core like the desktop does.
        assert_eq!(invoke(webview, "hotkey_edge", json!({ "pressed": true, "purpose": "edit" })), Err(Value::String(DICTATION_UNAVAILABLE.into())));
        assert_eq!(invoke(webview, "settings_set_edit_hotkey", json!({ "hotkey": "ctrl+alt+shift+e" })), Ok(Value::Null));
        wait_state(webview, |s| s.settings.edit_hotkey.as_deref() == Some("Ctrl+Alt+Shift+E"));
        assert_eq!(invoke(webview, "settings_set_edit_hotkey", json!({ "hotkey": null })), Ok(Value::Null));
        wait_state(webview, |s| s.settings.edit_hotkey.is_none());
        // The lone-key trigger (§13.1) is a shared setting too; only the desktop watches the key.
        assert_eq!(invoke(webview, "settings_set_solo_key", json!({ "key": "mouse_back" })), Ok(Value::Null));
        wait_state(webview, |s| s.settings.solo_key == Some(voltip_core::SoloKey::MouseBack));
        assert_eq!(invoke(webview, "hotkey_edge", json!({ "pressed": false, "chorded": true })), Err(Value::String(DICTATION_UNAVAILABLE.into())));
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
        assert_eq!(wait_state(webview, |_| true).update, voltip_core::ui::UpdateStatus::Disabled);
        // No local models on a phone: the library verbs refuse and the state carries an empty list.
        for cmd in ["model_download", "model_cancel", "model_remove"] {
            assert_eq!(invoke(webview, cmd, json!({ "id": "sense-voice-small" })), Err(Value::String(MODELS_UNAVAILABLE.into())), "{cmd}");
        }
        assert!(wait_state(webview, |_| true).models.is_empty());
        // No dictation pipeline, so no personal dictionary or rules (docs/dictation.md §16.4): every
        // vocabulary verb and query refuses, and the state carries empty lists.
        let id = "0f3f1a1e-8d4b-4c8e-9f7a-1c2d3e4f5a6b";
        let rule = json!({ "name": "n", "pattern": "p" });
        for (cmd, args) in [
            ("dictionary_add", json!({ "entry": { "term": "x" }, "historyId": null })),
            ("dictionary_update", json!({ "id": id, "entry": { "term": "x" } })),
            ("dictionary_remove", json!({ "id": id })),
            ("dictionary_reorder", json!({ "ids": [id] })),
            ("rules_add", json!({ "rule": rule })),
            ("rules_update", json!({ "id": id, "rule": rule })),
            ("rules_remove", json!({ "id": id })),
            ("rules_reorder", json!({ "ids": [id] })),
            ("rules_import", json!({ "toml": "version = 1\n", "mode": "merge" })),
            ("rules_export", json!({})),
            ("vocabulary_preview", json!({ "text": "你好", "draft": null })),
        ] {
            assert_eq!(invoke(webview, cmd, args), Err(Value::String(VOCABULARY_UNAVAILABLE.into())), "{cmd}");
        }
        let st = wait_state(webview, |_| true);
        assert!(st.dictionary.is_empty() && st.rules.is_empty());
        // No probe and no pipeline: scenes and the context switch refuse too (docs/dictation.md §18.6).
        let scene = json!({ "name": "聊天", "match": { "apps": ["slack"] } });
        for (cmd, args) in [
            ("scenes_add", json!({ "scene": scene })),
            ("scenes_update", json!({ "id": id, "scene": scene })),
            ("scenes_remove", json!({ "id": id })),
            ("scenes_reorder", json!({ "ids": [id] })),
            ("settings_set_context_sharing", json!({ "appName": true, "windowTitle": false })),
            ("recent_apps", json!({})),
        ] {
            assert_eq!(invoke(webview, cmd, args), Err(Value::String(SCENES_UNAVAILABLE.into())), "{cmd}");
        }
        assert!(wait_state(webview, |_| true).scenes.is_empty());
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
        // The phone has no HTTP probe: the core answers `unsupported`, it does not hang.
        assert_eq!(invoke(webview, "provider_probe", json!({ "provider": "groq", "kind": "llm" })), Ok(Value::Null));
        wait_event(rx, "provider_probe", |e| e["type"] == "provider_probe" && e["reason"] == "unsupported");
        assert_eq!(invoke(webview, "provider_console_open", json!({ "provider": "groq" })), Err(Value::String(PROVIDERS_UNAVAILABLE.into())));
        assert_eq!(invoke(webview, "project_link_open", json!({ "link": "feedback" })), Err(Value::String(PROJECT_LINKS_UNAVAILABLE.into())));
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
        assert!(wait_state(webview, |_| true).history.is_empty());
    });
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
            "audio_meter_start",
            "audio_meter_stop",
            "overlay_state",
            "update_status",
            "rules_export",
            "vocabulary_preview",
            "recent_apps",
            "permissions_status",
            "permissions_request",
            "inject_preflight",
            "provider_console_open",
            "project_link_open",
            "feedback_diagnostics",
            "feedback_submit",
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
