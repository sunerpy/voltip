#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The React Native shell's command surface on the build host (docs/mobile-rn.md §7): the real
//! [`Shell`] on the in-memory dictation fakes and secret store, with a [`RecordingHost`] standing in
//! for the Kotlin side. Every command the app may send is answered as the Tauri phone shell answers
//! it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use voltip_core::dictation::DictationPorts;
use voltip_core::dictation::fakes::{self, FakeAudio, FakeInjector, FakeTranscriber};
use voltip_core::{CoreConfig, EngineSettings, ProviderId, ProviderSettings, Settings, SettingsStore};
use voltip_identity::MemorySecretStore;
use voltip_rn::Shell;
use voltip_rn::commands::{CHANNEL_KEY, COMMANDS, HOTKEY_UNAVAILABLE, MODELS_UNAVAILABLE};
use voltip_rn::host::{HostCall, RecordingHost};
use voltip_rn::update::{InstallSource, NOTHING_TO_INSTALL, PACKAGE, UpdateConfig, store_listing};

const STEP_TIMEOUT: Duration = Duration::from_secs(15);
const POLL: Duration = Duration::from_millis(20);
/// Commands of the shared contract that only the desktop registers (docs/dictation.md §23.6).
const DESKTOP_ONLY: &[&str] = &["settings_set_serve", "serve_copy_token", "serve_rotate_token"];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// An offline phone with `settings`: `phone_config` with no relay, no mDNS, and the LAN host on an
/// ephemeral port.
fn offline_config(dir: &Path, settings: Settings) -> CoreConfig {
    SettingsStore::new(dir).save(&Settings { relay_enabled: false, ..settings }).unwrap();
    let mut config = voltip_rn::shell::phone_config(dir.to_path_buf(), "0.0.44");
    config.discovery = None;
    config.direct_bind = "127.0.0.1:0".parse().unwrap();
    config
}

struct Running {
    shell: Shell,
    host: Arc<RecordingHost>,
    runtime: Option<tokio::runtime::Runtime>,
    _dir: tempfile::TempDir,
}

impl Running {
    fn start(host: RecordingHost) -> Self {
        Self::start_with(host, Settings::default(), |_| fakes::ports())
    }

    /// The shell with `settings`, on the ports `ports` builds for the host.
    fn start_with(host: RecordingHost, settings: Settings, ports: impl FnOnce(Arc<RecordingHost>) -> DictationPorts) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().worker_threads(2).build().unwrap();
        let host = Arc::new(host);
        let shell = Shell::start_with_updates(
            runtime.handle().clone(),
            offline_config(dir.path(), settings),
            Arc::new(MemorySecretStore::new()),
            ports(host.clone()),
            host.clone(),
            offline_updates(),
        )
        .unwrap();
        let running = Self { shell, host, runtime: Some(runtime), _dir: dir };
        running.wait(|s| s["identity"].is_object());
        running
    }

    fn invoke(&self, command: &str, args: Value) -> Result<Value, String> {
        self.shell.invoke_blocking(command, args)
    }

    fn state(&self) -> Value {
        self.invoke("core_state", Value::Null).unwrap()
    }

    /// Wait until `core_state` satisfies `done`.
    fn wait(&self, done: impl Fn(&Value) -> bool) -> Value {
        wait_for(|| Some(self.state()).filter(|s| done(s)))
    }

    /// Wait for the take's phase to be one of `phases`; the state then.
    fn wait_phase(&self, phases: &[&str]) -> Value {
        let deadline = Instant::now() + STEP_TIMEOUT;
        loop {
            let state = self.state();
            if state["dictation"]["phase"]["phase"].as_str().is_some_and(|p| phases.contains(&p)) {
                return state;
            }
            assert!(Instant::now() < deadline, "no {phases:?} within {STEP_TIMEOUT:?}: {}", state["dictation"]);
            std::thread::sleep(POLL);
        }
    }
}

/// An install from a release whose update source answers nothing: a closed local port, so no test
/// asks GitHub.
fn offline_updates() -> UpdateConfig {
    UpdateConfig {
        source: Some(InstallSource::Direct),
        latest_release: "http://127.0.0.1:9/releases/latest".into(),
        listing: store_listing(PACKAGE),
        auto_check_delay: Duration::ZERO,
    }
}

/// A recogniser the engine counts as ready (a custom endpoint, no clean-up); the fake recogniser of
/// [`phone_ports_recognising`] answers in its place.
fn ready_settings() -> Settings {
    let custom = ProviderSettings { asr_url: Some("http://127.0.0.1:9".into()), asr_model: Some("asr".into()), ..ProviderSettings::default() };
    let engines = EngineSettings {
        asr_provider: ProviderId::Custom,
        providers: [(ProviderId::Custom, custom)].into(),
        refine_enabled: false,
        ..EngineSettings::default()
    };
    Settings { engines, ..Settings::default() }
}

/// The phone's own ports (`phone_ports`: its clipboard through the host) with a fake microphone and
/// recogniser in place of the device's and the cloud's.
fn phone_ports_recognising(host: Arc<RecordingHost>, transcriber: FakeTranscriber) -> DictationPorts {
    let fake = fakes::ports_with(Arc::new(FakeAudio::speech()), Arc::new(transcriber), None, Arc::new(FakeInjector::paste()));
    DictationPorts { audio: fake.audio, factory: fake.factory, ..voltip_rn::shell::phone_ports(host) }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.shell.shutdown();
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(Duration::from_secs(2));
        }
    }
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

/// The `COMMANDS` block of `apps/mobile/src-tauri/src/lib.rs`, in order.
fn tauri_phone_commands() -> Vec<String> {
    let src = std::fs::read_to_string(repo_root().join("apps/mobile/src-tauri/src/lib.rs")).unwrap();
    let start = src.find("pub const COMMANDS").unwrap();
    let block = &src[start..];
    let block = &block[block.find("= [").unwrap() + 3..block.find("];").unwrap()];
    block.split(',').map(|s| s.trim().trim_matches('"').to_owned()).filter(|s| !s.is_empty()).collect()
}

fn fixture_commands() -> Vec<(String, Value)> {
    let text = std::fs::read_to_string(repo_root().join("packages/shared/src/fixtures/ipc/commands.json")).unwrap();
    let entries: Vec<Value> = serde_json::from_str(&text).unwrap();
    entries.into_iter().map(|e| (e["name"].as_str().unwrap().to_owned(), e["args"].clone())).collect()
}

#[test]
fn the_command_list_is_the_tauri_phone_shells() {
    assert_eq!(COMMANDS.to_vec(), tauri_phone_commands());
    let mut unique = COMMANDS.to_vec();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), COMMANDS.len(), "COMMANDS has duplicates");
}

/// Every command of the shared fixture reaches its handler with the arguments `TauriBackend`
/// sends: no "unknown command", no argument error. The desktop's own commands are unknown here.
#[test]
fn every_fixture_command_is_answered() {
    let running = Running::start(RecordingHost::default());
    for (name, args) in fixture_commands() {
        let answer = running.invoke(&name, args.clone());
        if DESKTOP_ONLY.contains(&name.as_str()) {
            assert_eq!(answer, Err(format!("unknown command {name}")));
            continue;
        }
        match name.as_str() {
            "update_check" => assert_eq!(answer, Ok(Value::Null), "{name}"),
            "update_install" => assert_eq!(answer, Err(NOTHING_TO_INSTALL.to_owned()), "{name}"),
            "hotkey_edge" => assert_eq!(answer, Err(HOTKEY_UNAVAILABLE.to_owned())),
            n if n.starts_with("model_") => assert_eq!(answer, Err(MODELS_UNAVAILABLE.to_owned()), "{name}"),
            _ => {
                if let Err(e) = &answer {
                    assert!(!e.starts_with("unknown command") && !e.starts_with("invalid args"), "{name}({args}): {e}");
                }
            }
        }
        if name == "settings_set_auto_update" {
            // Turned on, 自动检查更新 checks at once, and the closed port fails that check. Wait for
            // it to end: while it runs, the list's own update_check is refused as busy.
            wait_for(|| (running.invoke("update_status", Value::Null).unwrap()["state"] == "failed").then_some(()));
        }
    }
    assert!(running.host.calls().contains(&HostCall::ShareText("今天下午三点开会。".into())), "phone_share_text reached the share sheet");
}

#[test]
fn unknown_commands_and_malformed_arguments_are_refused() {
    let running = Running::start(RecordingHost::default());
    assert_eq!(running.invoke("core_state_v2", Value::Null), Err("unknown command core_state_v2".into()));
    let err = running.invoke("device_rename", json!(["Studio"])).unwrap_err();
    assert!(err.starts_with("invalid args for device_rename"), "{err}");
    let err = running.invoke("device_rename", json!({})).unwrap_err();
    assert!(err.starts_with("invalid args for device_rename"), "{err}");
    let err = running.invoke("history_query", json!({ "limit": "ten" })).unwrap_err();
    assert!(err.starts_with("invalid args `limit` for command `history_query`"), "{err}");
}

/// The state the app reads first: the phone's identity and the default name, every event handed to
/// the host as JSON, the update status `idle` (an install from a release, nothing checked yet), and
/// the multicast lock taken because LAN discovery is on by default.
#[test]
fn the_shell_starts_the_phone_core_and_forwards_its_events() {
    let running = Running::start(RecordingHost::default());
    let state = running.state();
    assert_eq!(state["identity"]["name"], "Voltip 手机");
    assert_eq!(state["update"]["state"], "idle");
    assert_eq!(running.invoke("update_status", Value::Null).unwrap(), json!({ "state": "idle" }));
    wait_for(|| running.host.events().iter().any(|e| e["type"] == "state").then_some(()));
    assert!(running.host.events().iter().any(|e| e["type"] == "update" && e["state"] == "idle"), "the shell's own status went out");
    wait_for(|| running.host.calls().contains(&HostCall::Multicast(true)).then_some(()));
    running.invoke("device_rename", json!({ "name": "Pixel" })).unwrap();
    running.wait(|s| s["identity"]["name"] == "Pixel");
    wait_for(|| running.host.events().iter().any(|e| e["type"] == "identity" || e["identity"]["name"] == "Pixel").then_some(()));
}

#[test]
fn lan_discovery_follows_the_switch_and_holds_the_lock() {
    let running = Running::start(RecordingHost::default());
    running.invoke("settings_set_lan_discovery", json!({ "enabled": false })).unwrap();
    wait_for(|| running.host.calls().contains(&HostCall::Multicast(false)).then_some(()));
    running.wait(|s| s["settings"]["lan_discovery"] == false);
}

/// 发送剪贴板, the history's copy button and 分享 go through the host; the clipboard's empty text is
/// no text, a refused clipboard is `failed { inject }`, an over-long paste is `invalid`.
#[test]
fn the_clipboard_and_the_share_sheet_go_through_the_host() {
    let running = Running::start(RecordingHost::default());
    assert_eq!(running.invoke("phone_clipboard_read", Value::Null).unwrap(), json!({ "text": null }));
    running.host.set_clipboard(Some(""));
    assert_eq!(running.invoke("phone_clipboard_read", Value::Null).unwrap(), json!({ "text": null }));
    running.host.set_clipboard(Some("会议改到三点"));
    assert_eq!(running.invoke("phone_clipboard_read", Value::Null).unwrap(), json!({ "text": "会议改到三点" }));
    assert_eq!(running.invoke("paste_text", json!({ "text": "你好" })).unwrap(), json!({ "kind": "copied", "reason": "clipboard_only" }));
    assert_eq!(running.host.clipboard().as_deref(), Some("你好"));
    assert_eq!(running.invoke("paste_text", json!({ "text": "" })).unwrap(), json!({ "kind": "failed", "reason": "invalid" }));
    running.invoke("phone_share_text", json!({ "text": "分享这句" })).unwrap();
    assert!(running.host.calls().contains(&HostCall::ShareText("分享这句".into())));
    assert!(running.invoke("phone_share_text", json!({ "text": "" })).unwrap_err().starts_with("share: "));

    let refusing = Running::start(RecordingHost::refusing("clipboard: busy"));
    assert_eq!(refusing.invoke("paste_text", json!({ "text": "你好" })).unwrap(), json!({ "kind": "failed", "reason": "inject" }));
    assert_eq!(refusing.invoke("phone_clipboard_read", Value::Null), Err("clipboard: busy".into()));
    assert_eq!(refusing.invoke("phone_share_text", json!({ "text": "分享这句" })), Err("clipboard: busy".into()));
}

/// The app names a provider, a page or a link; the shell builds the URL (as the desktop does).
#[test]
fn pages_open_from_urls_the_shell_builds() {
    let running = Running::start(RecordingHost::default());
    running.invoke("project_link_open", json!({ "link": "releases" })).unwrap();
    running.invoke("guide_open", json!({ "page": "service", "locale": "zh-CN" })).unwrap();
    running.invoke("provider_console_open", json!({ "provider": "groq" })).unwrap();
    let opened: Vec<String> = running
        .host
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            HostCall::OpenUrl(url) => Some(url),
            _ => None,
        })
        .collect();
    assert_eq!(opened.len(), 3, "{opened:?}");
    assert_eq!(opened[0], "https://github.com/sunerpy/voltip/releases");
    assert!(opened[1].starts_with("https://"), "{}", opened[1]);
    assert!(opened[2].starts_with("https://") && opened[2].contains("groq"), "{}", opened[2]);
    assert!(running.invoke("provider_console_open", json!({ "provider": "custom" })).unwrap_err().contains("no key page"));
}

/// A history export goes to the share sheet as a file, or says why it cannot.
#[test]
fn a_history_export_of_a_missing_entry_says_it_is_gone() {
    let running = Running::start(RecordingHost::default());
    let answer = running.invoke("history_export", json!({ "id": "9b1deb4d-3b7d-4bad-9bdd-2b0d7b3dcb6d", "format": "txt", "fileName": "a" })).unwrap();
    assert_eq!(answer["kind"], "failed");
    assert_eq!(answer["code"], "gone");
}

/// The level meter subscribes to the core's stream and names the app's channel; stopping twice is
/// fine, and `onFrame` must be a channel.
#[test]
fn the_level_meter_subscribes_by_channel() {
    let running = Running::start(RecordingHost::default());
    let id = running.invoke("audio_meter_start", json!({ "deviceId": null, "onFrame": { CHANNEL_KEY: 7 } })).unwrap();
    let id = id.as_u64().unwrap();
    assert!(id > 0);
    running.invoke("audio_meter_stop", json!({ "id": id })).unwrap();
    running.invoke("audio_meter_stop", json!({ "id": id })).unwrap();
    let err = running.invoke("audio_meter_start", json!({ "deviceId": null, "onFrame": {} })).unwrap_err();
    assert!(err.contains("onFrame must be a channel"), "{err}");
}

/// What the phone answers itself: no devices to pick, no system audio, no pill, no permissions to
/// grant, nothing to inject.
#[test]
fn the_fixed_answers_match_the_tauri_phone_shell() {
    let running = Running::start(RecordingHost::default());
    assert_eq!(running.invoke("audio_devices", Value::Null).unwrap(), json!([]));
    assert_eq!(running.invoke("audio_outputs", Value::Null).unwrap(), json!({ "system_audio": { "state": "unsupported" }, "devices": [] }));
    assert_eq!(running.invoke("overlay_state", Value::Null).unwrap(), json!("blank"));
    assert_eq!(running.invoke("hotkey_capture", json!({ "active": true })).unwrap(), Value::Null);
    assert!(running.invoke("permissions_status", Value::Null).unwrap().is_object());
    assert_eq!(running.invoke("permissions_request", json!({ "permission": "microphone" })).unwrap(), Value::Null);
    assert!(running.invoke("inject_preflight", Value::Null).unwrap().is_object());
    let packs = running.invoke("scenes_builtin", Value::Null).unwrap();
    assert!(packs.as_array().is_some_and(|p| !p.is_empty() && p[0]["terms"].is_array()), "{packs}");
    assert!(running.invoke("presets_builtin", Value::Null).unwrap().as_array().is_some_and(|p| !p.is_empty()));
    assert!(running.invoke("rules_export", Value::Null).unwrap().is_string());
    let preview = running.invoke("vocabulary_preview", json!({ "text": "你好", "draft": null })).unwrap();
    assert!(preview.is_object(), "{preview}");
    let page = running.invoke("history_query", json!({ "limit": 20 })).unwrap();
    assert_eq!(page["entries"], json!([]), "{page}");
    assert_eq!(running.invoke("history_entry", json!({ "id": "9b1deb4d-3b7d-4bad-9bdd-2b0d7b3dcb6d" })).unwrap(), Value::Null);
    assert!(running.invoke("history_stats", json!({ "boundaries": [0, 86_400_000] })).unwrap().is_object());
    assert!(running.invoke("history_hits", Value::Null).unwrap().is_object());
    assert_eq!(running.invoke("recent_apps", Value::Null).unwrap(), json!([]));
    let desktop = "1".repeat(64);
    assert!(running.invoke("mirror_history_query", json!({ "desktop": desktop, "limit": 20 })).unwrap().is_object());
    assert_eq!(running.invoke("mirror_profile", json!({ "desktop": desktop })).unwrap(), Value::Null);
}

/// 反馈: the diagnostics the page shows, and attachments staged from base64 (the app's `invokeRaw`).
#[test]
fn feedback_attachments_arrive_as_base64() {
    let running = Running::start(RecordingHost::default());
    let info = running.invoke("feedback_diagnostics", json!({ "locale": "zh-CN" })).unwrap();
    assert!(info["configured"].is_boolean() && info["diagnostics"].is_object(), "{info}");
    // A 1×1 PNG.
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
    let staged = running.invoke("feedback_attachment_add", json!({ "data": png, "name": "截图.png", "type": "image/png" })).unwrap();
    let id = staged["id"].as_str().unwrap().to_owned();
    assert_eq!(staged["name"], "截图.png");
    running.invoke("feedback_attachment_remove", json!({ "id": id })).unwrap();
    running.invoke("feedback_attachments_clear", Value::Null).unwrap();
    assert!(running.invoke("feedback_attachment_add", json!({ "data": "not base64!", "name": "a.png", "type": "image/png" })).is_err());
    assert!(running.invoke("feedback_attachment_add", json!({ "data": png, "name": "a.txt", "type": "text/plain" })).is_err());
}

/// docs/dictation.md §20.7 on this shell: with no computer the phone recognises a take itself. The
/// level meter forwards the take's frames to the app's channel, the result goes onto the clipboard
/// through the host, the take is in the phone's history, and 分享文本 hands the export to the
/// share sheet as a file (or says why it could not).
#[test]
fn a_take_on_the_phone_meters_lands_on_the_clipboard_and_shares_its_export() {
    let running =
        Running::start_with(RecordingHost::default(), ready_settings(), |host| phone_ports_recognising(host, FakeTranscriber::ok("今天下午三点开会")));
    let meter = running.invoke("audio_meter_start", json!({ "deviceId": null, "onFrame": { CHANNEL_KEY: 7 } })).unwrap();
    assert_eq!(running.invoke("dictation_start", json!({})), Ok(Value::Null));
    running.wait_phase(&["listening"]);
    let frame = wait_for(|| {
        running.host.calls().into_iter().find_map(|c| match c {
            HostCall::Channel(7, json) => Some(json),
            _ => None,
        })
    });
    let frame: Value = serde_json::from_str(&frame).unwrap();
    assert!(frame["rms_dbfs"].is_number() && frame["seq"].is_number(), "{frame}");
    running.invoke("audio_meter_stop", json!({ "id": meter })).unwrap();
    assert_eq!(running.invoke("dictation_stop", json!({})), Ok(Value::Null));
    let state = running.wait_phase(&["done", "failed"]);
    let phase = &state["dictation"]["phase"];
    assert_eq!((phase["phase"].as_str(), phase["via"].as_str()), (Some("done"), Some("clipboard")), "{phase}");
    let text = phase["text"].as_str().unwrap().to_owned();
    assert!(text.contains("今天下午三点开会"), "{phase}");
    assert_eq!(running.host.clipboard().as_deref(), Some(text.as_str()), "the result is on the phone's clipboard");
    assert!(running.host.calls().contains(&HostCall::ClipboardWrite(text.clone())));

    let page = running.invoke("history_query", json!({ "limit": 10 })).unwrap();
    assert_eq!((page["total"].as_u64(), page["entries"][0]["text"].as_str()), (Some(1), Some(text.as_str())), "{page}");
    let id = page["entries"][0]["id"].clone();
    let shared = running.invoke("history_export", json!({ "id": id, "format": "txt", "fileName": "Voltip 2026-10-07 10.00" })).unwrap();
    assert_eq!(shared, json!({ "kind": "shared" }));
    let file = running.host.calls().into_iter().find_map(|c| match c {
        HostCall::ShareFile { name, text, mime } => Some((name, text, mime)),
        _ => None,
    });
    let (name, content, mime) = file.expect("the export reached the share sheet");
    assert_eq!((name.as_str(), mime.as_str()), ("Voltip 2026-10-07 10.00.txt", "text/plain"));
    assert!(content.contains(&text), "{content}");
    // A whole take has no subtitles; a share sheet that fails is the answer's reason.
    let subtitles = running.invoke("history_export", json!({ "id": id, "format": "srt", "fileName": "x" })).unwrap();
    assert_eq!(subtitles["code"], "empty", "{subtitles}");
    running.host.refuse(Some("share: no app takes the file"));
    let refused = running.invoke("history_export", json!({ "id": id, "format": "txt", "fileName": "x" })).unwrap();
    assert_eq!(refused, json!({ "kind": "failed", "code": "share", "detail": "share: no app takes the file" }));
}

/// A take that ends without text reaches the app as its failed phase (the shell logs the code).
#[test]
fn a_failed_take_reaches_the_app_with_its_code() {
    let running = Running::start_with(RecordingHost::default(), ready_settings(), |host| {
        phone_ports_recognising(host, FakeTranscriber::err("fake: the service is down"))
    });
    assert_eq!(running.invoke("dictation_start", json!({})), Ok(Value::Null));
    running.wait_phase(&["listening"]);
    assert_eq!(running.invoke("dictation_stop", json!({})), Ok(Value::Null));
    let state = running.wait_phase(&["done", "failed"]);
    let phase = &state["dictation"]["phase"];
    assert_eq!(phase["phase"], "failed", "{phase}");
    let code = phase["code"].clone();
    assert!(code.is_string(), "{phase}");
    let seen = wait_for(|| running.host.events().into_iter().find(|e| e["type"] == "dictation" && e["phase"]["phase"] == "failed"));
    assert_eq!(seen["phase"]["code"], code, "{seen}");
    assert_eq!(running.host.clipboard(), None, "nothing reached the clipboard");
}

/// 反馈 in a build without the endpoint says so, as the Tauri shells do; a computer's history the
/// phone has no copy of has no entries; the shell's handle names itself.
#[test]
fn feedback_without_an_endpoint_and_an_unknown_mirror_entry() {
    let running = Running::start(RecordingHost::default());
    let report = json!({ "kind": "bug", "message": "按住说话没有反应", "contact": null, "locale": "zh-CN", "attachments": [] });
    assert_eq!(running.invoke("feedback_submit", report), Err("not_configured".to_owned()));
    let desktop = "1".repeat(64);
    let entry = running.invoke("mirror_history_entry", json!({ "desktop": desktop, "id": "9b1deb4d-3b7d-4bad-9bdd-2b0d7b3dcb6d" }));
    assert_eq!(entry, Ok(Value::Null));
    assert!(format!("{:?}", running.shell).starts_with("Shell"));
    assert!(running.shell.bridge().state().identity.is_some());
}
