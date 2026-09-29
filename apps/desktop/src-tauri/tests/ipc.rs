#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Command-layer tests on Tauri's mock runtime: the real `#[tauri::command]`s, the real setup
//! hook and the real event forwarding, without a window, a webview process or a keychain.

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
use voltip_core::dictation::fakes::fake_engines;
use voltip_core::dictation::{DictationPorts, fakes};
use voltip_core::ui::{UI_EVENT_NAME, UiState, UpdateStatus};
use voltip_core::{
    Activation, ContextSharing, CoreConfig, DictationPhase, EdgeSource, FailureCode, InjectMode, Locale, ModelInstallState, SecretSource, Settings,
    SettingsStore, TakeKind, ThemeId,
};
use voltip_desktop_lib::update::{NOT_CONFIGURED, UpdateSlot};
use voltip_desktop_lib::{COMMANDS, DEV_SECRET_STORE_ENV, KEYCHAIN_SERVICE, ShellOptions, build_app, data_dir, dev_memory_store_requested, secret_store};
use voltip_identity::MemorySecretStore;
use voltip_pairing::PairingState;
use voltip_tauri_bridge::Bridge;

/// Per-step wait. 15 s was enough for a plain `cargo test`, but under `cargo llvm-cov` with all 17
/// tests booting a core each in parallel the first `state` event once took longer (make verify,
/// 2026-09-25); the assertions are unchanged, only the patience is.
const STEP_TIMEOUT: Duration = Duration::from_secs(45);
const POLL: Duration = Duration::from_millis(20);
const DEVICE_NAME: &str = "Desk Test";
const NOT_FOUND: &str = "not found";

/// Offline core: relay disabled, LAN host on an ephemeral loopback port, recognition on the fakes'
/// custom endpoint (a test build has no built-in service).
fn offline_config(dir: &Path) -> CoreConfig {
    SettingsStore::new(dir).save(&Settings { relay_enabled: false, engines: fake_engines(), ..Settings::default() }).unwrap();
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
    with_running_app_on(fakes::ports(), body);
}

/// [`with_running_app`] on explicit dictation ports (a probe, a refiner).
fn with_running_app_on(
    ports: DictationPorts,
    body: impl FnOnce(&AppHandle<MockRuntime>, &WebviewWindow<MockRuntime>, &mpsc::Receiver<String>) + Send + 'static,
) {
    let dir = tempfile::tempdir().unwrap();
    let app = build_app(
        mock_builder(),
        offline_config(dir.path()),
        Arc::new(MemorySecretStore::new()),
        ShellOptions::HEADLESS,
        voltip_desktop_lib::dictation::ShellPorts::headless(ports),
    )
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

/// docs/pairing.md 「常开配对」: the switch persists, a session opens on its own (on the LAN host
/// here, there is no relay), and switching it off closes the waiting one.
#[test]
fn always_on_pairing_opens_a_session_and_closes_it_when_off() {
    with_running_app(|_, webview, rx| {
        wait_state(webview, |s| s.identity.is_some());
        assert!(invoke(webview, "settings_set_pairing_always_on", json!({})).is_err(), "enabled is required");
        assert_eq!(invoke(webview, "settings_set_pairing_always_on", json!({ "enabled": true })), Ok(Value::Null));
        wait_event(rx, "settings", |e| e["type"] == "settings" && e["pairing_always_on"] == true);
        let waiting = wait_state(webview, |s| s.pairing.state == PairingState::WaitingForPeer);
        assert!(waiting.settings.pairing_always_on && waiting.pairing.code.is_some());
        assert_eq!(invoke(webview, "settings_set_pairing_always_on", json!({ "enabled": false })), Ok(Value::Null));
        wait_state(webview, |s| s.pairing.state == PairingState::Idle && !s.settings.pairing_always_on);
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

/// 「需要支持完整的中英文语言切换」: the locale is one persisted setting every window reads. The
/// command takes the kebab-case wire form, the core echoes it as `settings`, an unknown locale is
/// refused before the core sees it, and the other settings are untouched.
#[test]
fn settings_set_locale_persists_and_refuses_unknown_locales() {
    with_running_app(|_, webview, rx| {
        let st = wait_state(webview, |s| s.identity.is_some());
        assert_eq!(st.settings.locale, Locale::System, "default follows the OS");
        assert_eq!(invoke(webview, "settings_set_locale", json!({ "locale": "en" })), Ok(Value::Null));
        wait_state(webview, |s| s.settings.locale == Locale::En);
        let ev = wait_event(rx, "settings (locale)", |e| e["type"] == "settings" && e["locale"] == "en");
        assert_eq!(ev["hotkey"], voltip_core::DEFAULT_HOTKEY, "other settings survive");
        assert_eq!(invoke(webview, "settings_set_locale", json!({ "locale": "zh-cn" })), Ok(Value::Null));
        wait_state(webview, |s| s.settings.locale == Locale::ZhCn);
        for bad in [json!({ "locale": "fr" }), json!({ "locale": "zh_cn" }), json!({ "locale": "ZH-CN" }), json!({})] {
            assert!(invoke(webview, "settings_set_locale", bad.clone()).is_err(), "{bad}");
        }
        assert_eq!(wait_state(webview, |_| true).settings.locale, Locale::ZhCn);
    });
}

/// 「加入自动更新开关…默认关闭」: the toggle is off by default, persists through the core and
/// echoes as `settings`. This test build has no update source (the mock runtime never registers
/// the plugin), so `update_status` answers `disabled`, `core_state().update` agrees from the first
/// frame, and the two update verbs return the not-configured error instead of pretending.
#[test]
fn auto_update_toggle_persists_and_the_updater_is_honest_about_being_unconfigured() {
    with_running_app(|app, webview, rx| {
        let st = wait_state(webview, |s| s.identity.is_some());
        assert!(!st.settings.auto_update, "opt-in");
        assert_eq!(st.update, UpdateStatus::Disabled);
        assert_eq!(invoke(webview, "update_status", json!({})), Ok(json!({ "state": "disabled" })));
        for cmd in ["update_check", "update_install"] {
            assert_eq!(invoke(webview, cmd, json!({})), Err(Value::String(NOT_CONFIGURED.into())), "{cmd}");
        }
        let slot = app.state::<Arc<UpdateSlot>>();
        assert!(!slot.enabled());
        assert_eq!(invoke(webview, "settings_set_auto_update", json!({ "enabled": true })), Ok(Value::Null));
        let st = wait_state(webview, |s| s.settings.auto_update);
        assert_eq!(st.settings.locale, Locale::System, "other settings survive");
        wait_event(rx, "settings (auto_update)", |e| e["type"] == "settings" && e["auto_update"] == true);
        // Turning it on without an update source changes nothing about the updater.
        assert_eq!(invoke(webview, "update_status", json!({})), Ok(json!({ "state": "disabled" })));
        assert_eq!(invoke(webview, "settings_set_auto_update", json!({ "enabled": false })), Ok(Value::Null));
        wait_state(webview, |s| !s.settings.auto_update);
        assert!(invoke(webview, "settings_set_auto_update", json!({})).is_err(), "missing argument is refused");
        assert!(invoke(webview, "settings_set_auto_update", json!({ "enabled": "yes" })).is_err());
    });
}

/// Windows report (2026-09-24): "配置绑定快捷键无效" — the recorder kept the chord in React state and
/// nothing reached the core. The IPC command must persist the normalised chord into
/// `Settings.hotkey` and echo it as a `settings` event, and a chord without a modifier or with
/// two keys must be refused with the core's error rather than silently accepted.
#[test]
fn regression_settings_set_hotkey_persists_and_validates() {
    with_running_app(|_, webview, rx| {
        wait_state(webview, |s| s.identity.is_some());
        assert_eq!(wait_state(webview, |s| s.identity.is_some()).settings.hotkey, voltip_core::DEFAULT_HOTKEY);
        assert_eq!(invoke(webview, "settings_set_hotkey", json!({ "hotkey": "control + shift + d" })), Ok(Value::Null));
        let st = wait_state(webview, |s| s.settings.hotkey == "Ctrl+Shift+D");
        assert_eq!(st.settings.hotkey, "Ctrl+Shift+D");
        wait_event(rx, "settings", |e| e["type"] == "settings" && e["hotkey"] == "Ctrl+Shift+D");
        // The core validates asynchronously: a refused chord comes back as an `error` event (the
        // webview toasts it) and the saved chord stays untouched.
        for bad in ["D", "Ctrl+", "Ctrl+A+B", "Shift+Alt"] {
            assert_eq!(invoke(webview, "settings_set_hotkey", json!({ "hotkey": bad })), Ok(Value::Null), "{bad}");
            let ev = wait_event(rx, "error", |e| e["type"] == "error");
            assert!(ev["message"].as_str().is_some_and(|m| !m.is_empty()), "{bad}: {ev}");
            assert_eq!(wait_state(webview, |_| true).settings.hotkey, "Ctrl+Shift+D", "{bad}");
        }
    });
}

/// The voice-edit hotkey (docs/dictation.md §19) through the command layer: the default is
/// `Ctrl+Alt+E`, a chord is canonicalised, persisted and echoed as `settings`, a chord without a
/// modifier or the dictation chord comes back as an `error` event and changes nothing, `null`
/// switches it off (and the file keeps the `null`), and a non-string is refused at the IPC layer.
#[test]
fn settings_set_edit_hotkey_persists_validates_and_switches_off() {
    with_running_app(|_, webview, rx| {
        let st = wait_state(webview, |s| s.identity.is_some());
        assert_eq!(st.settings.edit_hotkey.as_deref(), Some(voltip_core::DEFAULT_EDIT_HOTKEY));
        assert_eq!(voltip_core::DEFAULT_EDIT_HOTKEY, "Ctrl+Alt+E");
        assert_eq!(invoke(webview, "settings_set_edit_hotkey", json!({ "hotkey": "control + alt + shift + e" })), Ok(Value::Null));
        wait_state(webview, |s| s.settings.edit_hotkey.as_deref() == Some("Ctrl+Alt+Shift+E"));
        wait_event(rx, "settings", |e| e["type"] == "settings" && e["edit_hotkey"] == "Ctrl+Alt+Shift+E");
        for (bad, needle) in [("E", "至少需要一个修饰键"), ("Ctrl+Alt+Space", "已用作听写快捷键")] {
            assert_eq!(invoke(webview, "settings_set_edit_hotkey", json!({ "hotkey": bad })), Ok(Value::Null), "{bad}");
            let ev = wait_event(rx, "error", |e| e["type"] == "error");
            assert!(ev["message"].as_str().is_some_and(|m| m.contains(needle)), "{bad}: {ev}");
            assert_eq!(wait_state(webview, |_| true).settings.edit_hotkey.as_deref(), Some("Ctrl+Alt+Shift+E"), "{bad}");
        }
        // The dictation hotkey may not take the edit chord either.
        assert_eq!(invoke(webview, "settings_set_hotkey", json!({ "hotkey": "Ctrl+Shift+Alt+E" })), Ok(Value::Null));
        let ev = wait_event(rx, "error (dictation = edit)", |e| e["type"] == "error");
        assert!(ev["message"].as_str().is_some_and(|m| m.contains("已用作「编辑选中文本」的快捷键")), "{ev}");
        assert_eq!(invoke(webview, "settings_set_edit_hotkey", json!({ "hotkey": null })), Ok(Value::Null));
        let st = wait_state(webview, |s| s.settings.edit_hotkey.is_none());
        assert_eq!(st.settings.hotkey, voltip_core::DEFAULT_HOTKEY, "the refused dictation change did not land");
        wait_event(rx, "settings (off)", |e| e["type"] == "settings" && e["edit_hotkey"].is_null());
        assert!(invoke(webview, "settings_set_edit_hotkey", json!({ "hotkey": 5 })).is_err());
    });
}

/// The lone-key trigger (docs/dictation.md §13.1) through the command layer: off by default, a
/// key persists under its wire name and comes back as `settings`, `null` switches it off, an
/// unknown key is refused at the IPC layer. `hotkey_edge` takes the `chorded` flag: a chorded
/// edge after a press cancels that press's take.
#[test]
fn settings_set_solo_key_persists_and_a_chorded_edge_cancels_its_take() {
    with_running_app(|_, webview, rx| {
        let st = wait_state(webview, |s| s.identity.is_some());
        assert_eq!(st.settings.solo_key, None);
        assert_eq!(invoke(webview, "settings_set_solo_key", json!({ "key": "right_ctrl" })), Ok(Value::Null));
        wait_state(webview, |s| s.settings.solo_key == Some(voltip_core::SoloKey::RightCtrl));
        wait_event(rx, "settings", |e| e["type"] == "settings" && e["solo_key"] == "right_ctrl");
        assert!(invoke(webview, "settings_set_solo_key", json!({ "key": "caps_lock" })).is_err());
        assert_eq!(invoke(webview, "settings_set_solo_key", json!({ "key": null })), Ok(Value::Null));
        wait_event(rx, "settings (off)", |e| e["type"] == "settings" && e["solo_key"].is_null());

        assert_eq!(invoke(webview, "hotkey_edge", json!({ "pressed": true, "source": "hotkey" })), Ok(Value::Null));
        wait_event(rx, "listening", |e| e["type"] == "dictation" && e["phase"]["phase"] == "listening");
        assert_eq!(invoke(webview, "hotkey_edge", json!({ "pressed": false, "source": "hotkey", "chorded": true })), Ok(Value::Null));
        wait_event(rx, "cancelled", |e| e["type"] == "dictation" && e["phase"]["phase"] == "cancelled");
    });
}

/// The settings page suspends the OS registration while it records (otherwise the bound chord is
/// swallowed by `RegisterHotKey` and never reaches the webview). Headless shells only track the
/// flag, but the command must exist, accept both edges and reject a missing argument.
#[test]
fn regression_hotkey_capture_toggles_the_recorder_flag() {
    with_running_app(|app, webview, _| {
        wait_state(webview, |s| s.identity.is_some());
        let registry = app.state::<Arc<voltip_desktop_lib::hotkey::HotkeyRegistry>>();
        assert!(!registry.capturing());
        assert_eq!(invoke(webview, "hotkey_capture", json!({ "active": true })), Ok(Value::Null));
        assert!(registry.capturing());
        assert_eq!(invoke(webview, "hotkey_capture", json!({ "active": false })), Ok(Value::Null));
        assert!(!registry.capturing());
        assert!(invoke(webview, "hotkey_capture", json!({})).is_err());
    });
}

/// docs/dictation.md §10.6: the shell reports what the local models can run on — the CPU's
/// threads and the GPUs this build drives — into the same state the engines pane reads.
#[test]
fn the_shell_reports_the_machine_the_local_models_run_on() {
    with_running_app(|_, webview, _| {
        let state = wait_state(webview, |s| s.hardware.cpu_threads > 0);
        let machine = voltip_asr_local::hardware();
        assert_eq!(state.hardware.cpu_threads as usize, machine.cpu_threads);
        assert_eq!(state.hardware.gpus.iter().map(|g| g.name.clone()).collect::<Vec<_>>(), machine.gpus.iter().map(|g| g.name.clone()).collect::<Vec<_>>());
    });
}

/// docs/dictation.md §20: the desktop records a phone's takes; it streams none itself.
#[test]
fn phone_take_commands_are_refused_on_the_desktop() {
    with_running_app(|_, webview, _| {
        wait_state(webview, |s| s.identity.is_some());
        for (cmd, args) in [("phone_take_start", json!({ "publicKey": "11".repeat(32) })), ("phone_take_stop", json!({})), ("phone_take_cancel", json!({}))] {
            assert_eq!(invoke(webview, cmd, args), Err(Value::String(voltip_desktop_lib::PHONE_TAKE_UNAVAILABLE.into())), "{cmd}");
        }
        assert!(core_state(webview).phone_take.is_none());
        // docs/dictation.md §20.6: the desktop inserts phones' texts; it sends none itself.
        for (cmd, args) in [
            ("phone_text_send", json!({ "publicKey": "11".repeat(32), "body": "x", "source": "typed" })),
            ("sent_texts_clear", json!({})),
            ("phone_clipboard_read", json!({})),
        ] {
            assert_eq!(invoke(webview, cmd, args), Err(Value::String(voltip_desktop_lib::PHONE_TEXT_UNAVAILABLE.into())), "{cmd}");
        }
        assert!(core_state(webview).sent_texts.is_empty());
    });
}

/// Settings › 外观 (2026-09-27): the pill's placement is a core setting the shell reads when it
/// shows the window; the webview only names `bottom` / `top` / `off`.
#[test]
fn settings_set_overlay_persists_the_pill_placement() {
    with_running_app(|_, webview, _| {
        wait_state(webview, |s| s.identity.is_some());
        assert_eq!(core_state(webview).settings.overlay, voltip_core::OverlayPlacement::Bottom);
        assert_eq!(invoke(webview, "settings_set_overlay", json!({ "placement": "off" })), Ok(Value::Null));
        wait_state(webview, |s| s.settings.overlay == voltip_core::OverlayPlacement::Off);
        assert!(invoke(webview, "settings_set_overlay", json!({ "placement": "left" })).is_err());
        assert_eq!(core_state(webview).settings.overlay, voltip_core::OverlayPlacement::Off);
    });
}

/// The prewarmed pill window runs on the `live` route and renders `UiState.dictation` itself; the
/// query exists for a window that finished loading after an explicit fixed-state show.
#[test]
fn overlay_state_query_answers_live_for_the_prewarmed_pill() {
    with_running_app(|app, webview, _| {
        wait_state(webview, |s| s.identity.is_some());
        assert_eq!(invoke(webview, "overlay_state", json!({})), Ok(Value::String("live".into())));
        assert_eq!(app.state::<voltip_desktop_lib::overlay::OverlaySlot>().current(), voltip_desktop_lib::overlay::LIVE_STATE);
        assert_eq!(voltip_desktop_lib::overlay::route_for(voltip_desktop_lib::overlay::LIVE_STATE), "index.html#/overlay?state=live");
    });
}

/// Windows report (2026-09-24): "windows平台显示为 linux x11". The shell must name the platform it was
/// compiled for; the webview shows this string verbatim.
#[test]
fn regression_hotkey_backend_names_the_compiled_platform() {
    let name = voltip_desktop_lib::hotkey::backend_name();
    let expected = if cfg!(target_os = "windows") {
        "Windows · RegisterHotKey"
    } else if cfg!(target_os = "macos") {
        "macOS · Carbon"
    } else {
        "Linux · X11"
    };
    if cfg!(target_os = "linux") {
        // Linux names the session it runs in (docs/dictation.md §14.4): X11 / XWayland / Wayland; a
        // headless run without a session keeps the X11 name the plugin uses.
        assert!(name.starts_with("global-shortcut · Linux · "), "{name}");
        assert_eq!(voltip_desktop_lib::hotkey::backend_name_for("linux", None), format!("global-shortcut · {expected}"));
    } else {
        assert_eq!(name, format!("global-shortcut · {expected}"));
    }
    // Exactly one platform is named; a Windows build can never say X11.
    assert_eq!(["Windows", "macOS", "Linux"].iter().filter(|p| name.contains(*p)).count(), 1, "{name}");
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

/// The dictation pipeline from the webview's side, on the fake ports: `dictation_start` opens the
/// (fake) microphone, `dictation_stop` runs ASR → inject and the finished text lands in the
/// history; every phase reaches the event bus as a `dictation` event.
#[test]
fn dictation_start_stop_runs_the_pipeline_and_records_history() {
    with_running_app(|_, webview, rx| {
        let st = wait_state(webview, |s| s.identity.is_some());
        assert_eq!(st.dictation.phase, DictationPhase::Idle);
        assert!(st.history.is_empty());
        // Stop with nothing running is refused by the core, as an `error` event, not a panic.
        assert_eq!(invoke(webview, "dictation_stop", json!({})), Ok(Value::Null));
        wait_event(rx, "error (idle stop)", |e| e["type"] == "error" && e["message"].as_str().is_some_and(|m| m.contains("没有进行中的听写")));
        assert_eq!(invoke(webview, "dictation_start", json!({})), Ok(Value::Null));
        let st = wait_state(webview, |s| matches!(s.dictation.phase, DictationPhase::Listening { .. }));
        assert_eq!(st.dictation.session, 1);
        wait_event(rx, "dictation/listening", |e| e["type"] == "dictation" && e["phase"]["phase"] == "listening");
        assert_eq!(invoke(webview, "dictation_stop", json!({})), Ok(Value::Null));
        let st = wait_state(webview, |s| s.dictation.phase.is_terminal());
        assert!(matches!(&st.dictation.phase, DictationPhase::Done { text, .. } if text == fakes::FAKE_TRANSCRIPT), "{:?}", st.dictation.phase);
        let ev = wait_event(rx, "dictation/done", |e| e["type"] == "dictation" && e["phase"]["phase"] == "done");
        assert_eq!(ev["phase"]["via"], "paste");
        assert_eq!(ev["session"], 1);
        let st = wait_state(webview, |s| s.history.len() == 1);
        assert_eq!(st.history[0].text, fakes::FAKE_TRANSCRIPT);
        let ev = wait_event(rx, "history", |e| e["type"] == "history" && e["entries"].as_array().is_some_and(|a| a.len() == 1));
        assert_eq!(ev["entries"][0]["outcome"]["kind"], "inserted");
        // Cancel in the terminal dwell dismisses the pill state straight away.
        assert_eq!(invoke(webview, "dictation_cancel", json!({})), Ok(Value::Null));
        wait_state(webview, |s| s.dictation.phase == DictationPhase::Idle);
    });
}

/// A provider key is stored and reported as set-by-user; its value never appears in the state,
/// the events or the settings file. `null` deletes it; a provider that takes no key refuses one.
#[test]
fn provider_key_set_flips_presence_without_exposing_the_value() {
    fn groq_key(s: &UiState) -> Option<voltip_core::SecretState> {
        s.engines.providers.iter().find(|p| p.id == voltip_core::ProviderId::Groq).and_then(|p| p.asr.as_ref()).map(|a| a.key)
    }
    with_running_app(|_, webview, rx| {
        let st = wait_state(webview, |s| s.identity.is_some() && !s.engines.providers.is_empty());
        assert_eq!(groq_key(&st).map(|k| k.set), Some(false), "no user key yet");
        assert_eq!(invoke(webview, "provider_key_set", json!({ "provider": "groq", "kind": "asr", "value": "tok_secret_value_123" })), Ok(Value::Null));
        let st = wait_state(webview, |s| groq_key(s).is_some_and(|k| k.source == SecretSource::User));
        let llm = st.engines.providers.iter().find(|p| p.id == voltip_core::ProviderId::Groq).and_then(|p| p.llm.as_ref()).unwrap();
        assert!(llm.key.set, "a vendor's two services share the key");
        let ev = wait_event(rx, "engines", |e| e["type"] == "engines" && e.to_string().contains(r#""source":"user""#));
        assert!(!ev.to_string().contains("tok_secret_value_123"));
        assert!(!serde_json::to_string(&st).unwrap().contains("tok_secret_value_123"));
        // Unknown providers are rejected by the IPC layer, key-less ones by the core.
        assert!(invoke(webview, "provider_key_set", json!({ "provider": "ssh", "kind": "asr", "value": "x" })).is_err());
        assert_eq!(invoke(webview, "provider_key_set", json!({ "provider": "local", "kind": "asr", "value": "x" })), Ok(Value::Null));
        wait_event(rx, "error (key-less provider)", |e| e["type"] == "error" && e["message"].as_str().is_some_and(|m| m.contains("local")));
        assert_eq!(invoke(webview, "provider_key_set", json!({ "provider": "groq", "kind": "llm", "value": null })), Ok(Value::Null));
        wait_state(webview, |s| groq_key(s).is_some_and(|k| !k.set));
    });
}

/// 测试连接 before anything is sent: a vendor without a key and a custom endpoint without a URL are
/// answered at once; a URL that answers is listed (docs/dictation.md §3.3).
#[test]
fn provider_probe_answers_with_a_reason_or_the_model_list() {
    let ports = DictationPorts { service_probe: Some(Arc::new(voltip_desktop_lib::dictation::HttpServiceProbe)), ..fakes::ports() };
    with_running_app_on(ports, |_, webview, rx| {
        wait_state(webview, |s| s.identity.is_some());
        assert_eq!(invoke(webview, "provider_probe", json!({ "provider": "openai", "kind": "asr" })), Ok(Value::Null));
        wait_event(rx, "probe (no key)", |e| e["type"] == "provider_probe" && e["provider"] == "openai" && e["reason"] == "key_missing");
        assert_eq!(invoke(webview, "provider_probe", json!({ "provider": "custom", "kind": "llm" })), Ok(Value::Null));
        wait_event(rx, "probe (no url)", |e| e["type"] == "provider_probe" && e["provider"] == "custom" && e["reason"] == "invalid_url");
        assert_eq!(invoke(webview, "provider_probe", json!({ "provider": "local", "kind": "asr" })), Ok(Value::Null));
        wait_event(rx, "probe (on-device)", |e| e["type"] == "provider_probe" && e["provider"] == "local" && e["reason"] == "unsupported");
        // A listening server: the model ids come back sorted.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let server = rt.block_on(wiremock::MockServer::start());
        rt.block_on(
            wiremock::Mock::given(wiremock::matchers::method("GET"))
                .and(wiremock::matchers::path("/v1/models"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({ "data": [{ "id": "b-model" }, { "id": "a-model" }] })))
                .mount(&server),
        );
        let base = format!("{}/v1", server.uri());
        assert_eq!(invoke(webview, "provider_probe", json!({ "provider": "custom", "kind": "asr", "baseUrl": base, "key": "k" })), Ok(Value::Null));
        let ev = wait_event(rx, "probe (ok)", |e| e["type"] == "provider_probe" && e["result"] == "ok");
        assert_eq!(ev["models"], json!(["a-model", "b-model"]));
        let seen = rt.block_on(server.received_requests()).unwrap();
        assert_eq!(seen[0].headers.get("authorization").unwrap(), "Bearer k", "the draft key is used for the request");
        // A probe saves nothing: the settings are the seeded ones and the draft key was not stored.
        let st = wait_state(webview, |_| true);
        assert_eq!(st.settings.engines, fake_engines());
        let custom = st.engines.providers.iter().find(|p| p.id == voltip_core::ProviderId::Custom).and_then(|p| p.asr.clone()).unwrap();
        assert!(!custom.key.set, "the draft key is not stored");
    });
}

/// Engine settings round-trip through `settings` + `engines`; history verbs mutate the list.
#[test]
fn engine_settings_and_history_commands_change_state() {
    with_running_app(|_, webview, rx| {
        wait_state(webview, |s| s.identity.is_some());
        // The `engines` object is a serde struct: snake_case keys inside (as `engineSettingsSchema`).
        let engines = json!({
            "asr_provider": "custom",
            "providers": { "custom": { "asr_url": "https://asr.example.test", "asr_model": "whisper" } },
            "language": "zh",
            "refine_enabled": false,
            "inject": "clipboard_only"
        });
        assert_eq!(invoke(webview, "settings_set_engines", json!({ "engines": engines })), Ok(Value::Null));
        let st = wait_state(webview, |s| s.engines.asr_host == "asr.example.test");
        assert_eq!(st.settings.engines.inject, InjectMode::ClipboardOnly);
        assert_eq!(st.settings.engines.language.as_deref(), Some("zh"));
        assert!(!st.engines.refine_enabled);
        wait_event(rx, "engines", |e| e["type"] == "engines" && e["asr_host"] == "asr.example.test");
        // A bad URL is reported by the core.
        let bad = json!({ "asr_provider": "custom", "providers": { "custom": { "asr_url": "ftp://nope" } } });
        assert_eq!(invoke(webview, "settings_set_engines", json!({ "engines": bad })), Ok(Value::Null));
        wait_event(rx, "error (bad asr_url)", |e| e["type"] == "error" && e["message"].as_str().is_some_and(|m| m.contains("custom.asr_url")));
        // Missing / malformed arguments are refused before the core sees them.
        assert!(invoke(webview, "settings_set_engines", json!({})).is_err());
        assert!(invoke(webview, "history_star", json!({ "id": "not-a-uuid", "starred": true })).unwrap_err().as_str().unwrap().contains("UUID"));
        assert!(invoke(webview, "history_delete", json!({ "id": "nope" })).is_err());

        // Produce one entry, then star / delete / clear it.
        assert_eq!(invoke(webview, "dictation_start", json!({})), Ok(Value::Null));
        wait_state(webview, |s| matches!(s.dictation.phase, DictationPhase::Listening { .. }));
        assert_eq!(invoke(webview, "dictation_stop", json!({})), Ok(Value::Null));
        let st = wait_state(webview, |s| s.history.len() == 1);
        let id = st.history[0].id.to_string();
        assert_eq!(invoke(webview, "history_star", json!({ "id": id, "starred": true })), Ok(Value::Null));
        wait_state(webview, |s| s.history.first().is_some_and(|h| h.starred));
        assert_eq!(invoke(webview, "history_delete", json!({ "id": id })), Ok(Value::Null));
        wait_state(webview, |s| s.history.is_empty());
        assert_eq!(invoke(webview, "dictation_start", json!({})), Ok(Value::Null));
        wait_state(webview, |s| matches!(s.dictation.phase, DictationPhase::Listening { .. }));
        assert_eq!(invoke(webview, "dictation_stop", json!({})), Ok(Value::Null));
        wait_state(webview, |s| s.history.len() == 1);
        assert_eq!(invoke(webview, "history_clear", json!({})), Ok(Value::Null));
        wait_state(webview, |s| s.history.is_empty());
        wait_event(rx, "history (cleared)", |e| e["type"] == "history" && e["entries"].as_array().is_some_and(Vec::is_empty));
    });
}

/// The personal dictionary and the replacement rules (docs/dictation.md §16.4) through the command
/// layer: a draft that is wrong on its own is refused as the command's error, a list conflict comes
/// back as an `error` event, the lists ride on `dictionary` / `rules` events and `core_state`, the
/// two queries answer with the core's own semantics, and a take on the fakes is rewritten and
/// records what fired.
#[test]
fn vocabulary_commands_and_queries_run_through_the_command_layer() {
    with_running_app(|_, webview, rx| {
        wait_state(webview, |s| s.identity.is_some());
        let err = invoke(webview, "dictionary_add", json!({ "entry": { "term": " " } })).unwrap_err();
        assert!(err.as_str().unwrap().starts_with("dictionary: ") && err.as_str().unwrap().contains("不能为空"), "{err}");
        assert!(invoke(webview, "dictionary_add", json!({ "entry": { "term": "x", "weight": 2 } })).is_err(), "unknown draft fields");
        assert_eq!(invoke(webview, "dictionary_add", json!({ "entry": { "term": "World", "heard_as": ["世界"] }, "historyId": null })), Ok(Value::Null));
        let st = wait_state(webview, |s| s.dictionary.len() == 1);
        assert_eq!(st.dictionary[0].source, voltip_core::EntrySource::Manual);
        wait_event(rx, "dictionary", |e| e["type"] == "dictionary" && e["entries"][0]["term"] == "World");
        assert_eq!(invoke(webview, "dictionary_add", json!({ "entry": { "term": "世界" } })), Ok(Value::Null));
        wait_event(rx, "error (conflict)", |e| e["type"] == "error" && e["message"].as_str().is_some_and(|m| m.starts_with("dictionary:")));
        assert_eq!(core_state(webview).dictionary.len(), 1, "the conflicting entry was not added");

        let err = invoke(webview, "rules_add", json!({ "rule": { "name": "bad", "kind": "regex", "pattern": "(" } })).unwrap_err();
        assert!(err.as_str().unwrap().contains("正则无法编译"), "{err}");
        assert_eq!(invoke(webview, "rules_add", json!({ "rule": { "name": "hello", "pattern": "你好", "replacement": "您好" } })), Ok(Value::Null));
        let st = wait_state(webview, |s| s.rules.len() == 1);
        wait_event(rx, "rules", |e| e["type"] == "rules" && e["rules"][0]["name"] == "hello");
        let preview = invoke(webview, "vocabulary_preview", json!({ "text": "你好，世界" })).unwrap();
        assert_eq!((preview["corrected"].as_str(), preview["output"].as_str()), (Some("你好，World"), Some("您好，World")), "{preview}");
        assert_eq!(preview["rules"][0]["id"], st.rules[0].id.to_string());
        let draft = json!({ "id": st.rules[0].id, "rule": { "name": "hello", "pattern": "你好", "replacement": "你好呀" } });
        assert_eq!(invoke(webview, "vocabulary_preview", json!({ "text": "你好", "draft": draft })).unwrap()["output"], "你好呀");
        let bad = json!({ "id": null, "rule": { "name": "x", "kind": "regex", "pattern": "(" } });
        assert!(invoke(webview, "vocabulary_preview", json!({ "text": "你好", "draft": bad })).unwrap_err().as_str().unwrap().contains("正则无法编译"));
        let toml = invoke(webview, "rules_export", json!({})).unwrap();
        assert!(toml.as_str().unwrap().contains("name = \"hello\""), "{toml}");
        let err = invoke(webview, "rules_import", json!({ "toml": "version = 1\n[[rule]]\n", "mode": "merge" })).unwrap_err();
        assert!(err.as_str().unwrap().contains("TOML 无法解析"), "{err}");
        let import = "version = 1\n[[rule]]\nname = \"bang\"\nkind = \"regex\"\npattern = \"World$\"\nreplacement = \"World!\"\n";
        assert_eq!(invoke(webview, "rules_import", json!({ "toml": import, "mode": "merge" })), Ok(Value::Null));
        let st = wait_state(webview, |s| s.rules.len() == 2);
        let ids: Vec<String> = st.rules.iter().rev().map(|r| r.id.to_string()).collect();
        assert_eq!(invoke(webview, "rules_reorder", json!({ "ids": ids })), Ok(Value::Null));
        let st = wait_state(webview, |s| s.rules.first().is_some_and(|r| r.name == "bang"));
        let hello = st.rules[1].id.to_string();
        assert_eq!(
            invoke(webview, "rules_update", json!({ "id": hello, "rule": { "name": "hello", "pattern": "你好", "replacement": "您好", "enabled": true } })),
            Ok(Value::Null)
        );
        let world = st.dictionary[0].id.to_string();
        assert_eq!(
            invoke(webview, "dictionary_update", json!({ "id": world, "entry": { "term": "World", "heard_as": ["世界"], "enabled": true } })),
            Ok(Value::Null)
        );
        assert_eq!(invoke(webview, "dictionary_reorder", json!({ "ids": [world] })), Ok(Value::Null));
        assert!(invoke(webview, "dictionary_remove", json!({ "id": "nope" })).unwrap_err().as_str().unwrap().contains("UUID"));

        // A take on the fakes (`你好，世界`): dictionary, then `bang`, then `hello`.
        assert_eq!(invoke(webview, "dictation_start", json!({})), Ok(Value::Null));
        wait_state(webview, |s| matches!(s.dictation.phase, DictationPhase::Listening { .. }));
        assert_eq!(invoke(webview, "dictation_stop", json!({})), Ok(Value::Null));
        let st = wait_state(webview, |s| s.history.len() == 1);
        assert_eq!(st.history[0].text, "您好，World!");
        assert_eq!(st.history[0].raw_text, fakes::FAKE_TRANSCRIPT);
        let hits = st.history[0].vocabulary.clone().expect("the history records what fired");
        assert_eq!((hits.corrections.len(), hits.rules.len()), (1, 2));

        assert_eq!(invoke(webview, "rules_remove", json!({ "id": st.rules[0].id })), Ok(Value::Null));
        wait_state(webview, |s| s.rules.len() == 1);
        assert_eq!(invoke(webview, "dictionary_remove", json!({ "id": st.dictionary[0].id })), Ok(Value::Null));
        wait_state(webview, |s| s.dictionary.is_empty());
    });
}

/// Scenes and context (docs/dictation.md §18) through the command layer: a draft wrong on its own
/// is the command's error, a name clash an `error` event; the list rides on `scenes` events and
/// `core_state`; the context switch persists; a take on the fakes with a probe carries the scene on
/// the `dictation` events and into the history, and `recent_apps` names the app afterwards.
#[test]
fn scene_commands_context_sharing_and_recent_apps_run_through_the_command_layer() {
    let probe = Arc::new(fakes::FakeProbe::app("Code.exe", "Code", Some("main.rs")));
    let ports = DictationPorts { probe: Some(probe), ..fakes::ports() };
    with_running_app_on(ports, |_, webview, rx| {
        wait_state(webview, |s| s.identity.is_some());
        let err = invoke(webview, "scenes_add", json!({ "scene": { "name": "代码", "match": { "apps": [] } } })).unwrap_err();
        assert!(err.as_str().unwrap().starts_with("scenes: ") && err.as_str().unwrap().contains("至少要有一个应用"), "{err}");
        assert!(invoke(webview, "scenes_add", json!({ "scene": { "name": "x", "match": { "apps": ["a"], "urls": [] } } })).is_err(), "unknown fields");
        let code = json!({ "name": "代码", "match": { "apps": ["Code.exe"], "title_contains": [] }, "overrides": { "refine_enabled": false, "prompt": "保留标识符" } });
        assert_eq!(invoke(webview, "scenes_add", json!({ "scene": code })), Ok(Value::Null));
        let st = wait_state(webview, |s| s.scenes.len() == 1);
        assert_eq!(st.scenes[0].matching.apps, ["code"], "stored normalised");
        wait_event(rx, "scenes", |e| e["type"] == "scenes" && e["scenes"][0]["name"] == "代码" && e["scenes"][0]["match"]["apps"][0] == "code");
        assert_eq!(invoke(webview, "scenes_add", json!({ "scene": { "name": "代码", "match": { "apps": ["slack"] } } })), Ok(Value::Null));
        wait_event(rx, "error (duplicate name)", |e| e["type"] == "error" && e["message"].as_str().is_some_and(|m| m.starts_with("scenes: 已有名为")));
        assert_eq!(invoke(webview, "scenes_add", json!({ "scene": { "name": "聊天", "match": { "apps": ["slack"] } } })), Ok(Value::Null));
        let st = wait_state(webview, |s| s.scenes.len() == 2);
        let ids: Vec<String> = st.scenes.iter().rev().map(|s| s.id.to_string()).collect();
        assert_eq!(invoke(webview, "scenes_reorder", json!({ "ids": ids })), Ok(Value::Null));
        wait_state(webview, |s| s.scenes.first().is_some_and(|x| x.name == "聊天"));
        let chat = st.scenes[1].id.to_string();
        assert_eq!(
            invoke(webview, "scenes_update", json!({ "id": chat, "scene": { "name": "聊天", "enabled": false, "match": { "apps": ["slack"] } } })),
            Ok(Value::Null)
        );
        wait_state(webview, |s| s.scenes.iter().any(|x| x.name == "聊天" && !x.enabled));
        assert!(invoke(webview, "scenes_remove", json!({ "id": "nope" })).unwrap_err().as_str().unwrap().contains("UUID"));
        assert_eq!(invoke(webview, "settings_set_context_sharing", json!({ "appName": false, "windowTitle": true })), Ok(Value::Null));
        wait_state(webview, |s| s.settings.context_sharing == ContextSharing { app_name: false, window_title: true });
        wait_event(rx, "settings", |e| e["type"] == "settings" && e["context_sharing"]["window_title"] == true);
        assert!(invoke(webview, "settings_set_context_sharing", json!({ "appName": true })).is_err(), "both switches are required");

        // A take in Code: the scene applies, the events carry it, the history records it.
        assert_eq!(invoke(webview, "recent_apps", json!({})).unwrap(), json!([]));
        assert_eq!(invoke(webview, "dictation_start", json!({})), Ok(Value::Null));
        wait_event(rx, "dictation with the context", |e| {
            e["type"] == "dictation" && e["context"]["scene"]["name"] == "代码" && e["context"]["app"]["id"] == "code"
        });
        wait_state(webview, |s| matches!(s.dictation.phase, DictationPhase::Listening { ready: true, .. }));
        assert_eq!(invoke(webview, "dictation_stop", json!({})), Ok(Value::Null));
        let st = wait_state(webview, |s| s.history.len() == 1);
        assert_eq!(st.history[0].scene.as_ref().map(|s| s.name.as_str()), Some("代码"));
        assert!(
            matches!(st.dictation.phase, DictationPhase::Done { refined: false, refine_error: None, .. }),
            "the scene switched refining off: {:?}",
            st.dictation.phase
        );
        assert_eq!(invoke(webview, "recent_apps", json!({})).unwrap(), json!([{ "id": "code", "name": "Code" }]));
        let removed = st.scenes[0].id.to_string();
        assert_eq!(invoke(webview, "scenes_remove", json!({ "id": removed })), Ok(Value::Null));
        wait_state(webview, |s| s.scenes.len() == 1);
    });
}

/// In-app feedback (docs/feedback.md) through the command layer: the diagnostics name the
/// version, the platform and the provider kinds, never a host; a report is checked before anything
/// is sent, and a build without an endpoint refuses it as `not_configured`.
#[test]
fn feedback_commands_show_what_goes_along_and_refuse_without_an_endpoint() {
    with_running_app(|_, webview, rx| {
        wait_state(webview, |s| s.identity.is_some());
        wait_event(rx, "engines", |e| e["type"] == "engines");
        let info = invoke(webview, "feedback_diagnostics", json!({ "locale": "zh-CN" })).unwrap();
        assert_eq!(info["configured"], voltip_desktop_lib::feedback::feedback_url().is_some());
        assert_eq!(info["diagnostics"]["app_version"], voltip_desktop_lib::APP_VERSION);
        assert_eq!(info["diagnostics"]["os"], std::env::consts::OS);
        assert_eq!(info["diagnostics"]["locale"], "zh-CN");
        assert_eq!(info["diagnostics"]["asr_provider"], "custom");
        assert!(!info.to_string().contains("127.0.0.1"), "{info}");
        assert!(invoke(webview, "feedback_diagnostics", json!({})).is_err(), "the locale is required");
        let report = |message: &str| json!({ "kind": "idea", "message": message, "contact": null, "locale": "en" });
        if voltip_desktop_lib::feedback::feedback_url().is_none() {
            assert_eq!(invoke(webview, "feedback_submit", report("hi")), Err(Value::String("not_configured".into())));
        } else {
            assert_eq!(invoke(webview, "feedback_submit", report("   ")), Err(Value::String("invalid".into())));
        }
        assert!(invoke(webview, "feedback_submit", json!({ "kind": "praise", "message": "x", "locale": "en" })).is_err());
    });
}

/// The 反馈 page's screenshots and recordings (docs/feedback.md) through the command layer: the
/// bytes are the raw IPC body with the name (percent-encoded) and the type in headers, the shell's
/// limits refuse by wire name, remove and clear drop what was staged, and a report naming a file
/// that is not staged goes nowhere.
#[test]
fn feedback_attachments_are_staged_from_a_raw_body_and_refused_by_name() {
    with_running_app(|_, webview, _| {
        wait_state(webview, |s| s.identity.is_some());
        let add = |name: &str, mime: &str, bytes: Vec<u8>| {
            let mut req = request("feedback_attachment_add", json!({}));
            req.body = InvokeBody::Raw(bytes);
            req.headers.insert("x-voltip-name", name.parse().unwrap());
            req.headers.insert("x-voltip-type", mime.parse().unwrap());
            get_ipc_response(webview, req).map(|body| body.deserialize::<Value>().unwrap())
        };
        let refused = |reason: &str| Err(Value::String(reason.into()));
        let shot = add("C%3A%5Cshots%5C%E6%88%AA%E5%9B%BE.png", "image/png", vec![1, 2, 3]).unwrap();
        assert_eq!((&shot["name"], &shot["type"], &shot["size"]), (&json!("截图.png"), &json!("image/png"), &json!(3)));
        assert_eq!(add("notes.txt", "text/plain", vec![1]), refused("attachment_type"));
        assert_eq!(add("big.png", "image/png", vec![0; 5 * 1024 * 1024 + 1]), refused("attachment_too_large"));
        assert_eq!(add("empty.mp4", "video/mp4", Vec::new()), refused("attachment_too_large"));
        assert_eq!(add("%20", "image/png", vec![1]), refused("attachment_name"));
        // A JSON body carries no file.
        assert_eq!(invoke(webview, "feedback_attachment_add", json!({})), refused("attachment_type"));
        assert_eq!(invoke(webview, "feedback_attachment_remove", json!({ "id": shot["id"] })), Ok(Value::Null));
        for i in 0..3 {
            add(&format!("{i}.png"), "image/png", vec![1]).unwrap();
        }
        assert_eq!(add("3.png", "image/png", vec![1]), refused("attachment_too_many"), "the removed one no longer counts");
        assert_eq!(invoke(webview, "feedback_attachments_clear", json!({})), Ok(Value::Null));
        add("again.png", "image/png", vec![1]).unwrap();
        let report = json!({ "kind": "bug", "message": "hi", "contact": null, "locale": "en", "attachments": ["not-staged"] });
        let expected = if voltip_desktop_lib::feedback::feedback_url().is_none() { "not_configured" } else { "invalid" };
        assert_eq!(invoke(webview, "feedback_submit", report), refused(expected));
    });
}

/// Local models (docs/dictation.md §10) through the command layer. The headless app is built on
/// the core's fakes, which carry no model library: the list is empty, local mode is refused, and the
/// three model verbs come back as honest `error` events (never "not found", never a panic). The
/// wiring with a real library is `with_model_library` below.
#[test]
fn model_commands_are_registered_and_refused_without_a_library() {
    with_running_app(|_, webview, rx| {
        wait_state(webview, |s| s.identity.is_some());
        // The core announces itself (`ready`, with the identity) before the engine status, so read
        // the state only once that `engines` event is in: it was flaky under coverage.
        wait_event(rx, "engines", |e| e["type"] == "engines");
        let st = wait_state(webview, |_| true);
        assert!(st.models.is_empty());
        assert_eq!(st.engines.asr_provider, voltip_core::ProviderId::Custom);
        assert!(!st.engines.local_ready);
        for cmd in ["model_download", "model_cancel", "model_remove"] {
            assert_eq!(invoke(webview, cmd, json!({ "id": "sense-voice-small" })), Ok(Value::Null), "{cmd}");
            let ev = wait_event(rx, "error (no library)", |e| e["type"] == "error");
            assert!(ev["message"].as_str().unwrap().contains("本地模型不可用"), "{cmd}: {ev}");
            assert!(invoke(webview, cmd, json!({})).is_err(), "{cmd}: the id is required");
        }
        let local = json!({ "asr_provider": "local", "local_model": "sense-voice-small", "refine_enabled": true, "inject": "paste" });
        assert_eq!(invoke(webview, "settings_set_engines", json!({ "engines": local })), Ok(Value::Null));
        wait_event(rx, "error (local without library)", |e| e["type"] == "error" && e["message"].as_str().is_some_and(|m| m.contains("本地模型不可用")));
        assert_eq!(wait_state(webview, |_| true).engines.asr_provider, voltip_core::ProviderId::Custom, "the refused settings did not stick");
    });
}

/// The same command layer over the desktop's real wiring (`ports_with_backend` with a temp models
/// root): the catalogue is listed, selecting a local model persists and reports `local_ready = false`
/// with no host, `dictation_start` is refused with「本地模型未下载」, `model_remove` of an
/// absent model is fine, and `model_cancel` of nothing is an error event. No network: nothing is
/// downloaded here.
#[test]
fn with_model_library_the_catalogue_is_listed_and_local_mode_reports_readiness() {
    let dir = tempfile::tempdir().unwrap();
    let models_root = dir.path().join("models");
    let hub = Arc::new(voltip_desktop_lib::audio::AudioHub::with_opener(Box::new(|_, _| Err("no audio device on this runtime".into()))));
    let ports = voltip_desktop_lib::dictation::ports_with_backend(Arc::new(voltip_audio::FakeBackend::new()), hub, models_root.clone());
    let app = build_app(mock_builder(), offline_config(dir.path()), Arc::new(MemorySecretStore::new()), ShellOptions::HEADLESS, ports)
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
            let st = wait_state(&webview, |s| s.identity.is_some() && !s.models.is_empty());
            assert_eq!(
                st.models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
                ["qwen3-asr-0.6b", "qwen3-asr-1.7b", "sense-voice-small", "paraformer-zh", "zipformer-stream-zh-en"]
            );
            assert!(st.models.iter().all(|m| m.state == ModelInstallState::NotInstalled && !m.active));
            assert!(st.models[0].recommended);
            assert_eq!(st.models[0].tier, "balanced");
            assert!(st.models[4].is_streaming());
            assert!(!st.engines.live_preview_ready, "the streaming model is not installed");
            assert!(st.settings.engines.live_preview, "the setting defaults to on");
            let ev = wait_event(&rx, "models", |e| e["type"] == "models");
            assert_eq!(ev["models"][0]["state"], json!({ "kind": "not_installed" }));
            assert_eq!(ev["models"][0]["capabilities"], json!(["offline"]));
            assert_eq!(ev["models"][4]["capabilities"], json!(["streaming"]));
            let local = json!({ "asr_provider": "local", "local_model": "paraformer-zh", "refine_enabled": false, "inject": "paste" });
            assert_eq!(invoke(&webview, "settings_set_engines", json!({ "engines": local })), Ok(Value::Null));
            let st = wait_state(&webview, |s| s.engines.asr_provider == voltip_core::ProviderId::Local);
            assert_eq!(st.engines.asr_host, "");
            assert_eq!(st.engines.local_model.as_deref(), Some("paraformer-zh"));
            assert!(!st.engines.local_ready);
            assert_eq!(st.engines.asr_model, "轻量 · 中文");
            assert_eq!(st.engines.asr_issue, Some(voltip_core::EngineIssue::ModelNotInstalled));
            let ev = wait_event(&rx, "engines (local)", |e| e["type"] == "engines" && e["asr_provider"] == "local");
            assert_eq!(ev["local_ready"], false);
            let st = wait_state(&webview, |s| s.models.iter().any(|m| m.active));
            assert!(st.models[3].active && !st.models[0].active);
            assert_eq!(invoke(&webview, "dictation_start", json!({})), Ok(Value::Null));
            wait_event(&rx, "error (model not downloaded)", |e| e["type"] == "error" && e["message"].as_str().is_some_and(|m| m.contains("本地模型未下载")));
            assert_eq!(wait_state(&webview, |_| true).dictation.phase, DictationPhase::Idle);
            assert_eq!(invoke(&webview, "model_cancel", json!({ "id": "paraformer-zh" })), Ok(Value::Null));
            wait_event(&rx, "error (nothing to cancel)", |e| e["type"] == "error" && e["message"].as_str().is_some_and(|m| m.contains("没有在下载")));
            assert_eq!(invoke(&webview, "model_remove", json!({ "id": "paraformer-zh" })), Ok(Value::Null));
            wait_event(&rx, "models (after remove)", |e| e["type"] == "models");
            assert_eq!(invoke(&webview, "model_download", json!({ "id": "ghost" })), Ok(Value::Null));
            wait_event(&rx, "error (unknown id)", |e| e["type"] == "error" && e["message"].as_str().is_some_and(|m| m.contains("目录中没有 ghost")));
            assert!(!models_root.join("paraformer-zh").exists(), "nothing was downloaded");
            // Back to the remote endpoint: the selection is gone from the status, nothing active.
            assert_eq!(invoke(&webview, "settings_set_engines", json!({ "engines": fake_engines() })), Ok(Value::Null));
            let st = wait_state(&webview, |s| s.engines.asr_provider == voltip_core::ProviderId::Custom);
            assert_eq!(st.engines.local_model, None);
            wait_state(&webview, |s| s.models.iter().all(|m| !m.active));
        }));
        webview.close().unwrap();
        outcome
    });
    app.run(|_, _| {});
    if let Err(panic) = driver.join().unwrap() {
        std::panic::resume_unwind(panic);
    }
}

/// The live preview (docs/dictation.md §11) through the command layer on the mock runtime: the
/// core's fakes plus a scripted streaming session and a library whose streaming model is
/// installed. `engines.live_preview_ready` is true, `dictation_start` produces `listening` events
/// whose `phase.live` carries partials and a committed segment, `dictation_stop` produces a
/// `processing` event with `preview`, and the final text is still the whole-take transcript.
#[test]
fn live_preview_events_carry_partials_and_the_processing_preview() {
    let dir = tempfile::tempdir().unwrap();
    let streaming = Arc::new(fakes::FakeStreaming::script());
    let ports = voltip_desktop_lib::dictation::ShellPorts::headless(fakes::ports_live(streaming.clone()));
    let app = build_app(mock_builder(), offline_config(dir.path()), Arc::new(MemorySecretStore::new()), ShellOptions::HEADLESS, ports)
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
            let st = wait_state(&webview, |s| s.identity.is_some() && !s.models.is_empty());
            assert!(st.engines.live_preview_ready, "streaming model installed + setting on");
            assert!(st.models.iter().any(|m| m.is_streaming() && m.state.is_installed()));
            assert_eq!(invoke(&webview, "dictation_start", json!({})), Ok(Value::Null));
            let ev = wait_event(&rx, "dictation/listening", |e| e["type"] == "dictation" && e["phase"]["phase"] == "listening");
            assert_eq!(ev["phase"]["ready"], false, "the first listening event precedes the device's audio");
            assert!(ev["phase"].get("live").is_none(), "no preview yet: {ev}");
            let ev = wait_event(&rx, "dictation/listening+live", |e| {
                e["type"] == "dictation" && e["phase"]["phase"] == "listening" && e["phase"].get("live").is_some()
            });
            assert_eq!(ev["phase"]["ready"], true);
            assert_eq!(ev["phase"]["live"]["current"], "你好", "the first word shows at once: {ev}");
            assert_eq!(ev["phase"]["live"]["committed"], json!([]));
            assert!(ev["phase"]["live"].get("degraded").is_none());
            let ev = wait_event(&rx, "dictation/listening+segment", |e| {
                e["type"] == "dictation" && e["phase"]["phase"] == "listening" && e["phase"]["live"]["committed"].as_array().is_some_and(|c| !c.is_empty())
            });
            assert_eq!(ev["phase"]["live"]["committed"][0]["text"], "你好，世界。");
            assert_eq!(ev["phase"]["live"]["committed"][0]["start_ms"], 0);
            assert_eq!(ev["phase"]["live"]["committed"][0]["end_ms"], 400);
            let st = wait_state(&webview, |s| matches!(&s.dictation.phase, DictationPhase::Listening { live: Some(l), .. } if l.current == "今天"));
            assert!(matches!(&st.dictation.phase, DictationPhase::Listening { ready: true, .. }));
            assert_eq!(invoke(&webview, "dictation_stop", json!({})), Ok(Value::Null));
            let ev = wait_event(&rx, "dictation/processing", |e| e["type"] == "dictation" && e["phase"]["phase"] == "processing");
            assert_eq!(ev["phase"]["stage"], "transcribing");
            assert_eq!(ev["phase"]["preview"], "你好，世界。今天", "committed + current carried into processing: {ev}");
            let st = wait_state(&webview, |s| s.dictation.phase.is_terminal());
            assert!(matches!(&st.dictation.phase, DictationPhase::Done { text, .. } if text == fakes::FAKE_TRANSCRIPT), "{:?}", st.dictation.phase);
            assert_eq!(streaming.opens(), 1);
            assert!(streaming.warms() >= 1);
            // Switching the setting off: readiness drops, the next run has no `live`.
            let mut off = serde_json::to_value(fake_engines()).unwrap();
            off["live_preview"] = false.into();
            off["refine_enabled"] = false.into();
            assert_eq!(invoke(&webview, "settings_set_engines", json!({ "engines": off })), Ok(Value::Null));
            let st = wait_state(&webview, |s| !s.engines.live_preview_ready);
            assert!(!st.settings.engines.live_preview);
            assert_eq!(invoke(&webview, "dictation_cancel", json!({})), Ok(Value::Null));
            wait_state(&webview, |s| s.dictation.phase == DictationPhase::Idle);
            assert_eq!(invoke(&webview, "dictation_start", json!({})), Ok(Value::Null));
            wait_event(&rx, "dictation/listening (off)", |e| e["type"] == "dictation" && e["phase"]["phase"] == "listening" && e["phase"]["ready"] == true);
            assert_eq!(invoke(&webview, "dictation_stop", json!({})), Ok(Value::Null));
            let ev = wait_event(&rx, "dictation/processing (off)", |e| e["type"] == "dictation" && e["phase"]["phase"] == "processing");
            assert!(ev["phase"].get("preview").is_none(), "no preview without live preview: {ev}");
            wait_state(&webview, |s| s.dictation.phase.is_terminal());
            assert_eq!(streaming.opens(), 1, "no second session");
        }));
        webview.close().unwrap();
        outcome
    });
    app.run(|_, _| {});
    if let Err(panic) = driver.join().unwrap() {
        std::panic::resume_unwind(panic);
    }
}

/// The pill follows the dictation phase, not the key: every phase maps to one of the pill's states and
/// `Idle` hides it. Every hotkey edge is one `HotkeyEdge` for the core's activation machine
/// (docs/dictation.md §13), stamped with the core's clock.
#[test]
fn overlay_pill_and_hotkey_edges_follow_the_dictation_contract() {
    use voltip_desktop_lib::overlay::pill_for;
    assert_eq!(pill_for(&DictationPhase::Idle), None);
    assert_eq!(pill_for(&DictationPhase::Listening { started_at: 1, ready: true, live: None, locked: false }), Some("listening"));
    assert_eq!(pill_for(&DictationPhase::Listening { started_at: 1, ready: true, live: None, locked: true }), Some("listening"), "locked is still listening");
    assert_eq!(pill_for(&DictationPhase::Processing { stage: voltip_core::ProcessingStage::Refining, started_at: 1, preview: None }), Some("processing"));
    assert_eq!(pill_for(&DictationPhase::CANCELLED), Some("cancelled"));
    assert_eq!(pill_for(&DictationPhase::Failed { code: FailureCode::Unknown, message: "x".into(), text: None }), Some("error"));
    let done = DictationPhase::Done {
        text: "a".into(),
        raw_text: "a".into(),
        chars: 1,
        via: voltip_core::dictation::Via::Paste,
        refined: false,
        duration_ms: 1,
        asr_ms: 1,
        refine_ms: None,
        refine_error: None,
        mode: voltip_core::OutputMode::WholeTake,
        segments: None,
        live_error: None,
    };
    assert_eq!(pill_for(&done), Some("inserted"));
    let before = voltip_core::now_ms();
    for (pressed, purpose) in [(true, TakeKind::Dictation), (false, TakeKind::Edit)] {
        match voltip_desktop_lib::hotkey::edge_command(pressed, EdgeSource::Hotkey, purpose) {
            voltip_tauri_bridge::UiCommand::HotkeyEdge { pressed: p, at_ms, source: EdgeSource::Hotkey, purpose: q, chorded: false } => {
                assert_eq!((p, q), (pressed, purpose));
                assert!(at_ms >= before && at_ms <= voltip_core::now_ms(), "stamped with the core's clock: {at_ms}");
            }
            other => panic!("{other:?}"),
        }
    }
    assert!(matches!(
        voltip_desktop_lib::cli::Remote::Toggle.command(),
        voltip_tauri_bridge::UiCommand::HotkeyEdge { pressed: true, source: EdgeSource::Cli, .. }
    ));
    assert!(matches!(voltip_desktop_lib::cli::Remote::Cancel.command(), voltip_tauri_bridge::UiCommand::DictationCancel));
}

/// Activation through the command layer (docs/dictation.md §13): `hotkey_edge` carries the key,
/// `settings_set_activation` the mode. Hold: press → `listening`, release → `processing` → `done`.
/// Toggle: the release is ignored, the next press stops. Hold-or-toggle: a short press locks and
/// `phase.locked` is on the bus, the next press stops. Bad arguments are refused at the IPC layer
/// and an out-of-range timing by the core.
#[test]
fn hotkey_edges_run_hold_toggle_and_lock_flows_through_the_command_layer() {
    with_running_app(|_, webview, rx| {
        let st = wait_state(webview, |s| s.identity.is_some());
        assert_eq!((st.settings.activation, st.settings.hold_threshold_ms, st.settings.extra_recording_ms), (Activation::Hold, 300, 0));
        let edge = |pressed: bool, source: &str| json!({ "pressed": pressed, "atMs": voltip_core::now_ms(), "source": source });

        // hold
        assert_eq!(invoke(webview, "hotkey_edge", edge(true, "hotkey")), Ok(Value::Null));
        let st = wait_state(webview, |s| matches!(s.dictation.phase, DictationPhase::Listening { .. }));
        assert!(matches!(st.dictation.phase, DictationPhase::Listening { locked: false, .. }));
        let ev = wait_event(rx, "dictation/listening", |e| e["type"] == "dictation" && e["phase"]["phase"] == "listening");
        assert_eq!(ev["phase"]["locked"], false, "the lock flag is on the wire: {ev}");
        assert_eq!(invoke(webview, "hotkey_edge", edge(false, "hotkey")), Ok(Value::Null));
        wait_event(rx, "dictation/processing", |e| e["type"] == "dictation" && e["phase"]["phase"] == "processing");
        let st = wait_state(webview, |s| s.dictation.phase.is_terminal());
        assert!(matches!(&st.dictation.phase, DictationPhase::Done { text, .. } if text == fakes::FAKE_TRANSCRIPT), "{:?}", st.dictation.phase);
        assert_eq!(invoke(webview, "dictation_cancel", json!({})), Ok(Value::Null));
        wait_state(webview, |s| s.dictation.phase == DictationPhase::Idle);

        // toggle
        assert_eq!(
            invoke(webview, "settings_set_activation", json!({ "activation": "toggle", "holdThresholdMs": 300, "extraRecordingMs": 0 })),
            Ok(Value::Null)
        );
        wait_state(webview, |s| s.settings.activation == Activation::Toggle);
        wait_event(rx, "settings (toggle)", |e| e["type"] == "settings" && e["activation"] == "toggle");
        assert_eq!(invoke(webview, "hotkey_edge", edge(true, "hotkey")), Ok(Value::Null));
        wait_state(webview, |s| matches!(s.dictation.phase, DictationPhase::Listening { .. }));
        assert_eq!(invoke(webview, "hotkey_edge", edge(false, "hotkey")), Ok(Value::Null));
        std::thread::sleep(Duration::from_millis(200));
        assert!(matches!(core_state(webview).dictation.phase, DictationPhase::Listening { .. }), "a release does not stop a toggle run");
        assert_eq!(invoke(webview, "hotkey_edge", edge(true, "hotkey")), Ok(Value::Null));
        wait_state(webview, |s| s.dictation.phase.is_terminal());
        assert_eq!(invoke(webview, "dictation_cancel", json!({})), Ok(Value::Null));
        wait_state(webview, |s| s.dictation.phase == DictationPhase::Idle);

        // hold_or_toggle: tap → locked, next press → stop; `at_ms` defaults to now, `source` to `ui`.
        // The threshold is the maximum so a slow test runner cannot turn the tap into a long press.
        assert_eq!(
            invoke(webview, "settings_set_activation", json!({ "activation": "hold_or_toggle", "holdThresholdMs": 5000, "extraRecordingMs": 0 })),
            Ok(Value::Null)
        );
        let st = wait_state(webview, |s| s.settings.activation == Activation::HoldOrToggle);
        assert_eq!(st.settings.hold_threshold_ms, 5000);
        assert_eq!(invoke(webview, "hotkey_edge", json!({ "pressed": true })), Ok(Value::Null));
        wait_state(webview, |s| matches!(s.dictation.phase, DictationPhase::Listening { .. }));
        assert_eq!(invoke(webview, "hotkey_edge", json!({ "pressed": false })), Ok(Value::Null));
        let st = wait_state(webview, |s| matches!(s.dictation.phase, DictationPhase::Listening { locked: true, .. }));
        assert_eq!(st.dictation.session, 3);
        let ev =
            wait_event(rx, "dictation/listening+locked", |e| e["type"] == "dictation" && e["phase"]["phase"] == "listening" && e["phase"]["locked"] == true);
        assert_eq!(ev["session"], 3);
        assert_eq!(invoke(webview, "hotkey_edge", edge(true, "cli")), Ok(Value::Null), "a CLI press stops the locked run");
        wait_state(webview, |s| s.dictation.phase.is_terminal());
        wait_state(webview, |s| s.history.len() == 3);

        // Refusals: IPC-level for shapes, core-level for ranges (and the saved mode is untouched).
        for bad in [json!({}), json!({ "pressed": "yes" }), json!({ "pressed": true, "source": "mouse" })] {
            assert!(invoke(webview, "hotkey_edge", bad.clone()).is_err(), "{bad}");
        }
        for bad in [json!({}), json!({ "activation": "press", "holdThresholdMs": 300, "extraRecordingMs": 0 }), json!({ "activation": "hold" })] {
            assert!(invoke(webview, "settings_set_activation", bad.clone()).is_err(), "{bad}");
        }
        assert_eq!(
            invoke(webview, "settings_set_activation", json!({ "activation": "hold", "holdThresholdMs": 300, "extraRecordingMs": 5001 })),
            Ok(Value::Null)
        );
        let ev = wait_event(rx, "error (range)", |e| e["type"] == "error");
        assert!(ev["message"].as_str().unwrap().contains("activation"), "{ev}");
        assert_eq!(core_state(webview).settings.activation, Activation::HoldOrToggle);
    });
}

/// A voice edit (docs/dictation.md §19) through the command layer, on fakes whose foreground
/// application has a selection and whose LLM rewrites it: `hotkey_edge` with `purpose: edit`
/// starts an edit take (`kind: edit` on the bus, the selection copied at the press), the release
/// runs ASR → edit → paste, and the history entry carries the instruction and the original. A
/// second `voltip --edit-toggle` pair runs another one; an unknown purpose is an IPC error and an
/// edge without one is still a dictation.
#[test]
fn an_edit_edge_rewrites_the_selection_through_the_command_layer() {
    const SELECTION: &str = "大家好，会议改到周四十点哈";
    const INSTRUCTION: &str = "改得更正式";
    const REWRITE: &str = "各位同事：会议改至周四上午十点。";
    let injector = Arc::new(fakes::FakeInjector::paste().with_selection(SELECTION));
    let refiner = Arc::new(fakes::FakeRefiner::ok(REWRITE));
    let ports =
        fakes::ports_with(Arc::new(fakes::FakeAudio::speech()), Arc::new(fakes::FakeTranscriber::ok(INSTRUCTION)), Some(refiner.clone()), injector.clone());
    with_running_app_on(ports, move |app, webview, rx| {
        wait_state(webview, |s| s.identity.is_some());
        let edge = |pressed: bool| json!({ "pressed": pressed, "source": "hotkey", "purpose": "edit" });
        assert_eq!(invoke(webview, "hotkey_edge", edge(true)), Ok(Value::Null));
        let ev = wait_event(rx, "dictation/listening (edit)", |e| e["type"] == "dictation" && e["phase"]["phase"] == "listening");
        assert_eq!(ev["kind"], "edit", "{ev}");
        wait_state(webview, |s| matches!(s.dictation.phase, DictationPhase::Listening { ready: true, .. }) && s.dictation.kind == TakeKind::Edit);
        assert_eq!(invoke(webview, "hotkey_edge", edge(false)), Ok(Value::Null));
        let st = wait_state(webview, |s| s.dictation.phase.is_terminal());
        assert!(matches!(&st.dictation.phase, DictationPhase::Done { text, .. } if text == REWRITE), "{:?}", st.dictation.phase);
        assert_eq!(st.dictation.kind, TakeKind::Edit);
        let ev = wait_event(rx, "history (edit)", |e| e["type"] == "history" && e["entries"].as_array().is_some_and(|a| a.len() == 1));
        assert_eq!(ev["entries"][0]["kind"], "edit");
        assert_eq!(ev["entries"][0]["edit"], json!({ "instruction": INSTRUCTION, "selection": SELECTION }));
        assert_eq!(ev["entries"][0]["text"], REWRITE);
        assert_eq!(injector.copies(), vec![vec![voltip_core::Modifier::Ctrl, voltip_core::Modifier::Alt]], "Ctrl+Alt+E's held modifiers");
        assert_eq!(injector.injected(), vec![REWRITE.to_owned()]);
        assert_eq!(refiner.edits(), vec![(SELECTION.to_owned(), INSTRUCTION.to_owned(), Vec::new())]);
        assert_eq!(invoke(webview, "dictation_cancel", json!({})), Ok(Value::Null));
        wait_state(webview, |s| s.dictation.phase == DictationPhase::Idle);

        // `voltip --edit-toggle` twice: a CLI edge copies at once (no key, no modifiers) and toggles.
        let argv = |a: &[&str]| std::iter::once("voltip").chain(a.iter().copied()).map(str::to_owned).collect::<Vec<_>>();
        voltip_desktop_lib::on_second_instance(app, &argv(&["--edit-toggle"]));
        wait_state(webview, |s| matches!(s.dictation.phase, DictationPhase::Listening { ready: true, .. }) && s.dictation.session == 2);
        assert_eq!(core_state(webview).dictation.kind, TakeKind::Edit);
        voltip_desktop_lib::on_second_instance(app, &argv(&["--edit-toggle"]));
        let st = wait_state(webview, |s| s.dictation.phase.is_terminal() && s.history.len() == 2);
        assert!(matches!(&st.dictation.phase, DictationPhase::Done { text, .. } if text == REWRITE), "{:?}", st.dictation.phase);
        assert_eq!(injector.copies().last(), Some(&Vec::new()));
        assert_eq!(invoke(webview, "dictation_cancel", json!({})), Ok(Value::Null));
        wait_state(webview, |s| s.dictation.phase == DictationPhase::Idle);

        // Shapes: an unknown purpose is refused before the core; no purpose is a dictation.
        assert!(invoke(webview, "hotkey_edge", json!({ "pressed": true, "purpose": "rewrite" })).is_err());
        assert_eq!(invoke(webview, "hotkey_edge", json!({ "pressed": true })), Ok(Value::Null));
        let st = wait_state(webview, |s| matches!(s.dictation.phase, DictationPhase::Listening { .. }) && s.dictation.session == 3);
        assert_eq!(st.dictation.kind, TakeKind::Dictation);
        assert_eq!(invoke(webview, "dictation_cancel", json!({})), Ok(Value::Null));
        wait_state(webview, |s| s.dictation.phase.is_terminal());
        assert_eq!(injector.copies().len(), 2, "no copy for a dictation");
    });
}

/// What the running instance does with a second `voltip` invocation's arguments (the
/// single-instance plugin hands them over): `--toggle` is a CLI key edge (start; stop), `--cancel`
/// discards, anything else only brings the main window forward.
#[test]
fn a_second_instance_forwards_toggle_and_cancel_to_the_running_core() {
    with_running_app(|app, webview, _| {
        wait_state(webview, |s| s.identity.is_some());
        let argv = |a: &[&str]| std::iter::once("voltip").chain(a.iter().copied()).map(str::to_owned).collect::<Vec<_>>();
        voltip_desktop_lib::on_second_instance(app, &argv(&["--toggle"]));
        let st = wait_state(webview, |s| matches!(s.dictation.phase, DictationPhase::Listening { .. }));
        assert_eq!(st.dictation.session, 1);
        voltip_desktop_lib::on_second_instance(app, &argv(&["--cancel"]));
        assert!(matches!(wait_state(webview, |s| s.dictation.phase.is_terminal()).dictation.phase, DictationPhase::Cancelled { .. }));
        voltip_desktop_lib::on_second_instance(app, &argv(&["--toggle"]));
        wait_state(webview, |s| matches!(s.dictation.phase, DictationPhase::Listening { .. }) && s.dictation.session == 2);
        voltip_desktop_lib::on_second_instance(app, &argv(&["--toggle"]));
        let st = wait_state(webview, |s| s.dictation.phase.is_terminal());
        assert!(matches!(st.dictation.phase, DictationPhase::Done { .. }), "{:?}", st.dictation.phase);
        // A plain relaunch (or garbage) touches the window only; the core sees nothing.
        voltip_desktop_lib::on_second_instance(app, &argv(&[]));
        voltip_desktop_lib::on_second_instance(app, &argv(&["--bogus"]));
        std::thread::sleep(Duration::from_millis(100));
        assert!(core_state(webview).dictation.phase.is_terminal(), "still in the dwell: nothing was dispatched");
        assert_eq!(voltip_desktop_lib::MAIN_WINDOW, "main");
        assert_eq!(ShellOptions::PRODUCTION.hidden(true), ShellOptions { global_hotkey: true, start_hidden: true });
        const { assert!(!ShellOptions::HEADLESS.start_hidden && !ShellOptions::PRODUCTION.start_hidden) };
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
/// The check below is two-sided: a name here that `schema.ts` now declares fails too, so the list
/// empties itself as the TypeScript side catches up.
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
    // Queries and streams (`QUERY_COMMANDS` in schema.ts) are not `UiCommand`s and have no fixture
    // entry: `core_state` returns the state, the audio commands enumerate / stream through a Channel.
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
            "feedback_attachment_add",
            "feedback_attachment_remove",
            "feedback_attachments_clear",
            "phone_clipboard_read",
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

/// docs/dictation.md §15: the three platform queries answer on every host. On the Linux test host
/// nothing is gated (`not_applicable` throughout, `nothing_to_grant`), a request is an accepted
/// no-op, and the injection preflight is `proceed` with `checked: false`; the wire is the
/// `voltip_platform` snake_case contract the TypeScript schema mirrors.
#[test]
fn platform_queries_answer_not_applicable_on_a_host_without_gates() {
    with_running_app(|_, webview, _| {
        wait_state(webview, |s| s.identity.is_some());
        let report = invoke(webview, "permissions_status", json!({})).unwrap();
        let expected_platform = if cfg!(target_os = "linux") { "linux" } else { "other" };
        if cfg!(not(any(target_os = "macos", target_os = "windows"))) {
            assert_eq!(report, json!({ "platform": expected_platform, "microphone": "not_applicable", "accessibility": "not_applicable" }));
            let parsed: voltip_desktop_lib::platform::PermissionReport = serde_json::from_value(report).unwrap();
            assert!(parsed.nothing_to_grant());
            assert!(voltip_platform::onboarding_gate(&parsed).is_empty());
        } else {
            for key in ["microphone", "accessibility"] {
                assert!(report[key].is_string(), "{key} missing in {report}");
            }
        }
        // Regression (public release, 2026-09-27): no Input Monitoring — no trigger needs it.
        assert!(invoke(webview, "permissions_request", json!({ "permission": "input_monitoring" })).is_err());
        for kind in ["microphone", "accessibility"] {
            // Accepted everywhere; only macOS (system prompts) and Windows (the Settings page for the
            // microphone) act on it, so the test host must not be one of them for this loop.
            if cfg!(not(any(target_os = "macos", target_os = "windows"))) {
                assert_eq!(invoke(webview, "permissions_request", json!({ "permission": kind })).unwrap(), Value::Null);
            }
        }
        let err = invoke(webview, "permissions_request", json!({ "permission": "camera" })).unwrap_err();
        assert!(err.as_str().unwrap().contains("permission"), "{err}");
        let preflight = invoke(webview, "inject_preflight", json!({})).unwrap();
        if cfg!(not(target_os = "windows")) {
            assert_eq!(preflight["checked"], false);
            assert_eq!(preflight["decision"], "proceed");
            assert_eq!(preflight["target_process"], Value::Null);
        } else {
            assert_eq!(preflight["checked"], true);
            assert!(["proceed", "elevated_target", "secure_desktop", "unknown"].contains(&preflight["decision"].as_str().unwrap()));
        }
        assert_eq!(preflight["platform"], expected_platform_for_preflight());
    });
}

fn expected_platform_for_preflight() -> &'static str {
    if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "other"
    }
}

#[test]
fn production_wiring_helpers_are_well_formed() {
    assert!(!secret_store().backend_name().is_empty());
    // The in-memory store is a debug-build, opt-in affordance only; release builds never take it.
    assert!(dev_memory_store_requested(Some("memory"), true));
    assert!(!dev_memory_store_requested(Some("memory"), false));
    assert!(!dev_memory_store_requested(Some("keyring"), true));
    assert!(!dev_memory_store_requested(None, true));
    assert_eq!(DEV_SECRET_STORE_ENV, "VOLTIP_DEV_SECRET_STORE");
    const { assert!(ShellOptions::PRODUCTION.global_hotkey && !ShellOptions::HEADLESS.global_hotkey) };
    assert!(voltip_desktop_lib::hotkey::backend_name().starts_with("global-shortcut · "));
    assert_eq!(voltip_desktop_lib::overlay::route_for("listening"), "index.html#/overlay?state=listening");
    assert_eq!(voltip_desktop_lib::overlay::route_for(voltip_desktop_lib::overlay::BLANK_STATE), "index.html#/overlay?state=blank");
    // The opaque pill is a debug smoke-test knob only; release builds always draw it transparent.
    assert!(voltip_desktop_lib::overlay::dev_opaque_overlay_requested(Some("1"), true));
    assert!(!voltip_desktop_lib::overlay::dev_opaque_overlay_requested(Some("1"), false));
    assert!(!voltip_desktop_lib::overlay::dev_opaque_overlay_requested(Some("yes"), true));
    assert!(!voltip_desktop_lib::overlay::dev_opaque_overlay_requested(None, true));
    assert!(KEYCHAIN_SERVICE.starts_with("dev.voltip."));
    let dir = data_dir();
    assert!(dir.is_absolute(), "{dir:?}");
    // The update source is a build-time affair: the variable names are the documented ones and a
    // build without them has no updater.
    assert_eq!(voltip_desktop_lib::update::UPDATE_URL_ENV, "VOLTIP_UPDATE_URL");
    assert_eq!(voltip_desktop_lib::update::UPDATE_PUBKEY_ENV, "VOLTIP_UPDATE_PUBKEY");
    assert!(voltip_desktop_lib::update::UpdaterConfig::from_values(None, Some("key")).is_none());
    assert!(voltip_desktop_lib::update::AUTO_CHECK_DELAY >= Duration::from_secs(5), "the window settles before the automatic check");
}
