//! Voltip desktop shell: Tauri commands → [`voltip_tauri_bridge::Bridge`] → `voltip-core`.
//!
//! The shell owns nothing but wiring: identity, pairing and transport live in `voltip-core`;
//! the private key lives in the OS keychain via `KeyringSecretStore`. Everything except
//! [`run`] is generic over the Tauri runtime so `tests/ipc.rs` drives the real command layer on
//! `tauri::test::MockRuntime` without a window.

// `unsafe` is forbidden everywhere except `platform/windows.rs` (Win32 FFI for the injection
// preflight and the microphone consent store), `solo_key/windows.rs` (the low-level input hooks of
// the lone-key trigger) and `exit.rs` (`_exit` on Linux): on those platforms the crate-level lint
// is `deny`, which each of those modules relaxes with `#![allow(unsafe_code)]` and a SAFETY
// comment on every block.
#![cfg_attr(not(any(target_os = "windows", target_os = "linux")), forbid(unsafe_code))]
#![cfg_attr(any(target_os = "windows", target_os = "linux"), deny(unsafe_code))]
#![warn(missing_docs)]

use std::sync::Arc;

pub mod audio;
pub mod cli;
pub mod dictation;
pub mod exit;
pub mod export;
pub mod feedback;
pub mod hotkey;
#[cfg(target_os = "macos")]
pub mod keychain_handoff;
pub mod overlay;
pub mod paste;
pub mod platform;
pub mod restart;
pub mod solo_key;
pub mod update;

use tauri::{Emitter as _, Manager as _, Runtime};
use voltip_core::ui::{ProjectLink, UI_EVENT_NAME, UiEvent, UiState, UpdateStatus};
use voltip_core::{
    Activation, AppRef, CoreConfig, DictionaryDraft, EdgeSource, EngineSettings, ImportMode, Locale, OverlayPlacement, PresetDraft, PreviewDraft, ProviderId,
    RuleDraft, SceneDraft, ServiceKind, TakeKind, ThemeId, VocabularyPreview,
};
use voltip_identity::{KeyringSecretStore, SecretStore};
use voltip_tauri_bridge::{Bridge, BridgeError, UiCommand};

/// Keychain service id.
pub const KEYCHAIN_SERVICE: &str = "dev.voltip.desktop";

/// Every command the webview may invoke, in registration order. The TypeScript side
/// (`packages/shared/src/schema.ts` `CommandArgs`) and the IPC fixtures
/// (`packages/shared/src/fixtures/ipc/commands.json`) must name exactly this set; `tests/ipc.rs`
/// checks all three against each other.
pub const COMMANDS: [&str; 98] = [
    "core_state",
    "pairing_start",
    "pairing_join_code",
    "pairing_join_ticket",
    "pairing_confirm",
    "pairing_reject",
    "pairing_cancel",
    "pairing_reset",
    "device_forget",
    "device_rename",
    "send_text",
    "phone_take_start",
    "phone_take_stop",
    "phone_take_cancel",
    "phone_text_send",
    "sent_texts_clear",
    "phone_clipboard_read",
    "settings_set_lan_discovery",
    "settings_set_pairing_always_on",
    "pairing_join_nearby",
    "settings_set_relay",
    "settings_set_theme",
    "settings_set_hotkey",
    "settings_set_edit_hotkey",
    "settings_set_solo_key",
    "settings_set_microphone",
    "settings_set_recording",
    "hotkey_capture",
    "devices_refresh",
    "connectivity_check",
    "audio_devices",
    "audio_outputs",
    "audio_meter_start",
    "audio_meter_stop",
    "overlay_state",
    "dictation_start",
    "dictation_stop",
    "dictation_cancel",
    "hotkey_edge",
    "settings_set_activation",
    "settings_set_engines",
    "provider_key_set",
    "provider_probe",
    "provider_console_open",
    "project_link_open",
    "feedback_diagnostics",
    "feedback_submit",
    "feedback_attachment_add",
    "feedback_attachment_remove",
    "feedback_attachments_clear",
    "history_delete",
    "history_clear",
    "history_star",
    "history_process",
    "history_process_cancel",
    "history_export",
    "settings_set_locale",
    "settings_set_auto_update",
    "settings_set_history",
    "settings_set_overlay",
    "update_check",
    "update_install",
    "update_status",
    "model_download",
    "model_cancel",
    "model_remove",
    "dictionary_add",
    "dictionary_update",
    "dictionary_remove",
    "dictionary_reorder",
    "rules_add",
    "rules_update",
    "rules_remove",
    "rules_reorder",
    "rules_import",
    "rules_export",
    "vocabulary_preview",
    "scenes_add",
    "scenes_update",
    "scenes_remove",
    "scenes_reorder",
    "scenes_restore",
    "scenes_builtin",
    "presets_add",
    "presets_update",
    "presets_remove",
    "presets_try",
    "presets_builtin",
    "settings_set_context_sharing",
    "recent_apps",
    "history_query",
    "history_entry",
    "history_stats",
    "history_hits",
    "permissions_status",
    "permissions_request",
    "inject_preflight",
    "paste_text",
];

/// The app version: `package.json`'s, which release-please bumps and `tauri.conf.json` names
/// (build.rs). Not `CARGO_PKG_VERSION`: the Cargo version is static (`0.0.0`).
pub const APP_VERSION: &str = env!("VOLTIP_APP_VERSION");

/// App data directory (`ProjectDirs` → per-OS conventional location).
pub fn data_dir() -> std::path::PathBuf {
    directories::ProjectDirs::from("dev", "voltip", "Voltip").map(|d| d.data_dir().to_path_buf()).unwrap_or_else(|| std::env::temp_dir().join("voltip"))
}

/// OS keychain store scoped to the current user (nothing is touched until the first read). A macOS
/// release keeps its items in ones it created itself, starting from what the build before it
/// handed over (`keychain_handoff`; docs/runbook.md 发布 · macOS 签名与钥匙串).
pub fn secret_store() -> Arc<dyn SecretStore> {
    let user = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_else(|_| "default".into());
    if dev_memory_store_requested(std::env::var(DEV_SECRET_STORE_ENV).ok().as_deref(), cfg!(debug_assertions)) {
        tracing::warn!("{DEV_SECRET_STORE_ENV}=memory: identity lives in memory for this run only (debug build smoke test)");
        return Arc::new(voltip_identity::MemorySecretStore::new());
    }
    #[cfg(target_os = "macos")]
    if voltip_identity::signed_with_a_certificate() {
        match (keychain_handoff::cdhash(), voltip_identity::SecurityKeychain::login()) {
            (Some(build), Ok(keychain)) => {
                return Arc::new(voltip_identity::PerBuildStore::new(keychain, KEYCHAIN_SERVICE, user, build, keychain_handoff::take_received()));
            }
            (build, keychain) => {
                tracing::warn!(cdhash = build.is_some(), keychain = keychain.is_ok(), "per-build keychain items unavailable; using the shared ones")
            }
        }
    }
    Arc::new(KeyringSecretStore::new(KEYCHAIN_SERVICE, user))
}

/// Environment variable that lets a **debug** build run without an OS keychain (headless smoke
/// tests under Xvfb, CI runners without Secret Service). Ignored by release builds, which always
/// use the platform store and refuse to fall back to anything weaker.
pub const DEV_SECRET_STORE_ENV: &str = "VOLTIP_DEV_SECRET_STORE";

/// `true` only when a debug build was explicitly asked for the in-memory store.
pub fn dev_memory_store_requested(value: Option<&str>, debug_build: bool) -> bool {
    debug_build && value == Some("memory")
}

#[tauri::command]
fn core_state(bridge: tauri::State<'_, Bridge>) -> UiState {
    bridge.state()
}

#[tauri::command]
fn pairing_start(bridge: tauri::State<'_, Bridge>) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::PairingStart)?)
}

#[tauri::command]
fn pairing_join_code(bridge: tauri::State<'_, Bridge>, code: String) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::PairingJoinCode { code })?)
}

#[tauri::command]
fn pairing_join_ticket(bridge: tauri::State<'_, Bridge>, uri: String) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::PairingJoinTicket { uri })?)
}

#[tauri::command]
fn pairing_confirm(bridge: tauri::State<'_, Bridge>) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::PairingConfirm)?)
}

#[tauri::command]
fn pairing_reject(bridge: tauri::State<'_, Bridge>) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::PairingReject)?)
}

#[tauri::command]
fn pairing_cancel(bridge: tauri::State<'_, Bridge>) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::PairingCancel)?)
}

#[tauri::command]
fn pairing_reset(bridge: tauri::State<'_, Bridge>) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::PairingReset)?)
}

#[tauri::command]
fn device_forget(bridge: tauri::State<'_, Bridge>, public_key: String) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::DeviceForget { public_key })?)
}

#[tauri::command]
fn device_rename(bridge: tauri::State<'_, Bridge>, name: String) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::DeviceRename { name })?)
}

#[tauri::command]
fn send_text(bridge: tauri::State<'_, Bridge>, public_key: String, body: String) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::SendText { public_key, body })?)
}

/// Why the desktop streams no take: it is the one that records a phone's takes
/// (docs/dictation.md §20), not a phone.
pub const PHONE_TAKE_UNAVAILABLE: &str = "phone_take: 电脑接收手机的录音，不向其他设备推送";

#[tauri::command]
fn phone_take_start(_public_key: String) -> Result<(), String> {
    Err(PHONE_TAKE_UNAVAILABLE.into())
}

#[tauri::command]
fn phone_take_stop() -> Result<(), String> {
    Err(PHONE_TAKE_UNAVAILABLE.into())
}

#[tauri::command]
fn phone_take_cancel() -> Result<(), String> {
    Err(PHONE_TAKE_UNAVAILABLE.into())
}

/// The desktop inserts a phone's text (docs/dictation.md §20.6); it has no phone side of its own.
pub const PHONE_TEXT_UNAVAILABLE: &str = "phone_text: 电脑接收手机发来的文字，不向其他设备发送";

#[tauri::command]
fn phone_text_send(_public_key: String, _body: String, _source: voltip_core::phone::PhoneTextSource) -> Result<(), String> {
    Err(PHONE_TEXT_UNAVAILABLE.into())
}

/// LAN discovery (docs/pairing.md 「局域网发现」): announce this device and browse for the others.
#[tauri::command]
fn settings_set_lan_discovery(bridge: tauri::State<'_, Bridge>, enabled: bool) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::SettingsSetLanDiscovery { enabled })?)
}

/// Always-on pairing (docs/pairing.md 「常开配对」): keep a session waiting for a phone until
/// turned off. The phone's core refuses it.
#[tauri::command]
fn settings_set_pairing_always_on(bridge: tauri::State<'_, Bridge>, enabled: bool) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::SettingsSetPairingAlwaysOn { enabled })?)
}

/// Join the pairing the nearby device `fingerprint` waits for (a tap under 「附近的电脑」).
#[tauri::command]
fn pairing_join_nearby(bridge: tauri::State<'_, Bridge>, fingerprint: String) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::PairingJoinNearby { fingerprint })?)
}

#[tauri::command]
fn sent_texts_clear() -> Result<(), String> {
    Err(PHONE_TEXT_UNAVAILABLE.into())
}

#[tauri::command]
fn phone_clipboard_read() -> Result<serde_json::Value, String> {
    Err(PHONE_TEXT_UNAVAILABLE.into())
}

#[tauri::command]
fn settings_set_relay(bridge: tauri::State<'_, Bridge>, url: Option<String>, enabled: bool) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::SettingsSetRelay { url, enabled })?)
}

#[tauri::command]
fn settings_set_theme(bridge: tauri::State<'_, Bridge>, theme: ThemeId, follow_system: bool) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::SettingsSetTheme { theme, follow_system })?)
}

#[tauri::command]
fn settings_set_hotkey(bridge: tauri::State<'_, Bridge>, hotkey: String) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::SettingsSetHotkey { hotkey })?)
}

/// The voice-edit hotkey (docs/dictation.md §19): a chord, or `null` to unregister it (the
/// `--edit-toggle` remote keeps working). The core validates it like the dictation hotkey,
/// refuses the dictation chord, persists and re-emits `settings`; the hotkey module re-registers.
#[tauri::command]
fn settings_set_edit_hotkey(bridge: tauri::State<'_, Bridge>, hotkey: Option<String>) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::SettingsSetEditHotkey { hotkey })?)
}

/// The lone-key trigger (docs/dictation.md §13.1): a key, or `null` to switch it off. The core
/// persists it; `hotkey::follow_settings` (un)installs the input hook when `settings` arrives.
#[tauri::command]
fn settings_set_solo_key(bridge: tauri::State<'_, Bridge>, key: Option<voltip_core::SoloKey>) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::SettingsSetSoloKey { key })?)
}

/// The microphone takes record from: a device id of `audio_devices`, or `null` for the system
/// default. The core validates and persists it.
#[tauri::command]
fn settings_set_microphone(bridge: tauri::State<'_, Bridge>, device: Option<String>) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::SettingsSetMicrophone { device })?)
}

/// `settings_set_recording { recording }` (docs/dictation.md §22): a dictation take's source, the
/// output device and the longest length. The core validates and persists it.
#[tauri::command]
fn settings_set_recording(bridge: tauri::State<'_, Bridge>, recording: voltip_core::RecordingSettings) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::SettingsSetRecording { recording })?)
}

/// The settings page is recording a chord (`active = true`): suspend the OS registration so the
/// keys reach the webview; `false` registers the saved chord again. Without the plugin (headless
/// tests) this only records the flag.
#[tauri::command]
fn hotkey_capture<R: Runtime>(
    app: tauri::AppHandle<R>,
    bridge: tauri::State<'_, Bridge>,
    registry: tauri::State<'_, Arc<hotkey::HotkeyRegistry>>,
    options: tauri::State<'_, ShellOptions>,
    active: bool,
) -> Result<(), String> {
    if options.global_hotkey {
        hotkey::set_capture(&app, &bridge, &registry, active);
    } else {
        registry.set_capturing_flag(active);
    }
    Ok(())
}

#[tauri::command]
fn devices_refresh(bridge: tauri::State<'_, Bridge>) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::DevicesRefresh)?)
}

/// Start the connectivity self-check; the report arrives as a `connectivity` event.
#[tauri::command]
fn connectivity_check(bridge: tauri::State<'_, Bridge>) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::ConnectivityCheck)?)
}

/// Open the microphone ("开始听写" / hotkey pressed).
#[tauri::command]
fn dictation_start(bridge: tauri::State<'_, Bridge>) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::DictationStart)?)
}

/// Close the microphone and run ASR → refine → inject ("停止" / hotkey released).
#[tauri::command]
fn dictation_stop(bridge: tauri::State<'_, Bridge>) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::DictationStop)?)
}

/// Discard the recording or the pending result.
#[tauri::command]
fn dictation_cancel(bridge: tauri::State<'_, Bridge>) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::DictationCancel)?)
}

/// A key transition for the core's activation machine (docs/dictation.md §13). The OS hotkey
/// handler dispatches the same command itself; this entry exists for the webview (a `ui` edge
/// from a test or a remote page) and stamps the core's clock when `at_ms` is absent. `purpose`
/// picks the key (`dictation`, the default, or `edit` — docs/dictation.md §19).
#[tauri::command]
fn hotkey_edge(
    bridge: tauri::State<'_, Bridge>,
    pressed: bool,
    at_ms: Option<u64>,
    source: Option<EdgeSource>,
    purpose: Option<TakeKind>,
    chorded: Option<bool>,
) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::HotkeyEdge {
        pressed,
        at_ms: at_ms.unwrap_or_else(voltip_core::now_ms),
        source: source.unwrap_or(EdgeSource::Ui),
        purpose: purpose.unwrap_or_default(),
        chorded: chorded.unwrap_or(false),
    })?)
}

/// How the hotkey drives a dictation (`hold` | `toggle` | `hold_or_toggle`) and its timings; the
/// core validates, persists and re-emits `settings`.
#[tauri::command]
fn settings_set_activation(bridge: tauri::State<'_, Bridge>, activation: Activation, hold_threshold_ms: u32, extra_recording_ms: u32) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::SettingsSetActivation { activation, hold_threshold_ms, extra_recording_ms })?)
}

/// Replace `Settings.engines`; the core validates, persists, rebuilds its clients and emits `engines`.
#[tauri::command]
fn settings_set_engines(bridge: tauri::State<'_, Bridge>, engines: EngineSettings) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::SettingsSetEngines { engines })?)
}

/// Store (`value`) or delete (`null`) the user's key for a provider's service. Only `set` /
/// `source` ever come back.
#[tauri::command]
fn provider_key_set(bridge: tauri::State<'_, Bridge>, provider: ProviderId, kind: ServiceKind, value: Option<String>) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::ProviderKeySet { provider, kind, value })?)
}

/// List a provider's models with the form's values; answered by a `provider_probe` event.
#[tauri::command]
fn provider_probe(
    bridge: tauri::State<'_, Bridge>,
    provider: ProviderId,
    kind: ServiceKind,
    base_url: Option<String>,
    key: Option<String>,
) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::ProviderProbe { provider, kind, base_url, key })?)
}

/// Open the vendor's API-key page in the browser. Only catalogue URLs can be opened: the
/// webview names a provider, never a URL.
#[tauri::command]
fn provider_console_open(provider: ProviderId) -> Result<(), String> {
    let url = provider.spec().console_url.ok_or_else(|| format!("{}: no key page", provider.as_str()))?;
    tauri_plugin_opener::open_url(url, None::<&str>).map_err(|e| e.to_string())
}

/// The repository this build comes from (`Cargo.toml` `repository`): what 反馈 and 关于 open.
pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

/// Open a project page (the repository, its new-issue page) in the browser. Like the key pages,
/// the webview names the page and the shell builds the URL.
#[tauri::command]
fn project_link_open(link: ProjectLink) -> Result<(), String> {
    tauri_plugin_opener::open_url(link.url(REPOSITORY), None::<&str>).map_err(|e| e.to_string())
}

/// What a 反馈 report would carry, and whether this build can send one (docs/feedback.md).
/// `locale` is the language the webview resolved.
#[tauri::command]
fn feedback_diagnostics(bridge: tauri::State<'_, Bridge>, locale: String) -> feedback::FeedbackInfo {
    let session = hotkey::linux_session().map(|s| s.kind.to_string());
    feedback::FeedbackInfo { configured: feedback::feedback_url().is_some(), diagnostics: feedback::diagnostics(&bridge.state(), &locale, session) }
}

/// Post the 反馈 page's report with the diagnostics it showed and the attachments it staged; the
/// error is the reason's wire name (`not_configured`, `invalid`, `rate_limited`, …), never the
/// endpoint. The staged files are forgotten once the report went out (`attachments`: it did, but
/// a file's bytes did not all follow).
#[tauri::command]
async fn feedback_submit(
    bridge: tauri::State<'_, Bridge>,
    staged: tauri::State<'_, feedback::Attachments>,
    kind: feedback::FeedbackKind,
    message: String,
    contact: Option<String>,
    locale: String,
    attachments: Option<Vec<String>>,
) -> Result<feedback::Receipt, String> {
    let url = feedback::feedback_url().ok_or_else(|| feedback::SendError::NotConfigured.as_str().to_owned())?;
    let (message, contact) = feedback::check(&message, contact.as_deref()).map_err(|e| e.as_str().to_owned())?;
    let ids = attachments.unwrap_or_default();
    let files = staged.pick(&ids).map_err(|e| e.as_str().to_owned())?;
    let session = hotkey::linux_session().map(|s| s.kind.to_string());
    let diagnostics = feedback::diagnostics(&bridge.state(), &locale, session);
    let sent = feedback::send(url, feedback::feedback_token(), kind, &message, contact.as_deref(), &diagnostics, &files).await;
    // `Attachments`: the report itself went out, so its files are done with too.
    if matches!(sent, Ok(_) | Err(feedback::SendError::Attachments)) {
        staged.forget(&ids);
    }
    sent.map_err(|e| e.as_str().to_owned())
}

/// Stage a screenshot or a screen recording for the next report (docs/feedback.md): the raw bytes
/// as the IPC body, the file name (`x-voltip-name`, percent-encoded) and the MIME type
/// (`x-voltip-type`) as headers. The error is the refusal's wire name (`attachment_type`,
/// `attachment_too_large`, …).
#[tauri::command]
fn feedback_attachment_add(request: tauri::ipc::Request<'_>, staged: tauri::State<'_, feedback::Attachments>) -> Result<feedback::StagedAttachment, String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err(feedback::AttachError::Type.as_str().to_owned());
    };
    let header = |name: &str| request.headers().get(name).and_then(|v| v.to_str().ok()).unwrap_or_default();
    let name = feedback::percent_decode(header("x-voltip-name"));
    staged.add(&name, header("x-voltip-type"), bytes.clone()).map_err(|e| e.as_str().to_owned())
}

/// Drop a staged attachment (the page's ×).
#[tauri::command]
fn feedback_attachment_remove(staged: tauri::State<'_, feedback::Attachments>, id: String) {
    staged.remove(&id);
}

/// Drop every staged attachment: the 反馈 page opens with none and takes its files along when
/// it is left, so a reloaded page cannot leave files behind that count against the limits.
#[tauri::command]
fn feedback_attachments_clear(staged: tauri::State<'_, feedback::Attachments>) {
    staged.clear();
}

#[tauri::command]
fn history_delete(bridge: tauri::State<'_, Bridge>, id: String) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::HistoryDelete { id })?)
}

#[tauri::command]
fn history_clear(bridge: tauri::State<'_, Bridge>) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::HistoryClear)?)
}

#[tauri::command]
fn history_star(bridge: tauri::State<'_, Bridge>, id: String, starred: bool) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::HistoryStar { id, starred })?)
}

/// 用 AI 预设处理 (docs/dictation.md §22): progress and the end arrive as `history_process` events
/// with `request_id`.
#[tauri::command]
fn history_process(bridge: tauri::State<'_, Bridge>, request_id: u64, id: String, preset: String) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::HistoryProcess { request_id, id, preset })?)
}

/// Stop a `history_process`; nothing is stored.
#[tauri::command]
fn history_process_cancel(bridge: tauri::State<'_, Bridge>, request_id: u64) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::HistoryProcessCancel { request_id })?)
}

/// 导出字幕 / 导出文本 (docs/dictation.md §22): the save dialog offers `file_name`, and the file is
/// written here.
#[tauri::command]
async fn history_export<R: Runtime>(
    app: tauri::AppHandle<R>,
    bridge: tauri::State<'_, Bridge>,
    id: uuid::Uuid,
    format: voltip_core::history::export::ExportFormat,
    file_name: String,
) -> Result<export::ExportOutcome, String> {
    let entry = history_read(&bridge, move |b| b.history_entry(id)).await?;
    Ok(match export::content(entry.as_ref(), format) {
        Ok(content) => export::save(&app, content, format, &file_name).await,
        Err(outcome) => outcome,
    })
}

/// UI language (`system` | `zh-cn` | `en`); persisted by the core, every window follows `settings`.
#[tauri::command]
fn settings_set_locale(bridge: tauri::State<'_, Bridge>, locale: Locale) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::SettingsSetLocale { locale })?)
}

/// Automatic update check on launch; persisted by the core, [`update::follow_settings`] reacts.
#[tauri::command]
fn settings_set_auto_update(bridge: tauri::State<'_, Bridge>, enabled: bool) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::SettingsSetAutoUpdate { enabled })?)
}

#[tauri::command]
fn settings_set_history(bridge: tauri::State<'_, Bridge>, enabled: bool, keep: u32) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::SettingsSetHistory { enabled, keep })?)
}

#[tauri::command]
fn settings_set_overlay(bridge: tauri::State<'_, Bridge>, placement: OverlayPlacement) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::SettingsSetOverlay { placement })?)
}

/// Ask the update manifest now; progress arrives as `update` events (`checking` → `up_to_date` /
/// `available` / `failed`). Refused when this build has no update source or a run is in flight.
#[tauri::command]
fn update_check<R: Runtime>(app: tauri::AppHandle<R>, bridge: tauri::State<'_, Bridge>, slot: tauri::State<'_, Arc<update::UpdateSlot>>) -> Result<(), String> {
    update::request(&app, &bridge, &slot, update::Intent::Check)
}

/// Download (if not yet), install and relaunch: `downloading` → `ready` → `installing`. Checks
/// first when nothing is pending. Same refusals as `update_check`.
#[tauri::command]
fn update_install<R: Runtime>(
    app: tauri::AppHandle<R>,
    bridge: tauri::State<'_, Bridge>,
    slot: tauri::State<'_, Arc<update::UpdateSlot>>,
) -> Result<(), String> {
    update::request(&app, &bridge, &slot, update::Intent::Install)
}

/// The updater's current status (query; the webview pulls it on mount, then follows `update` events).
#[tauri::command]
fn update_status(slot: tauri::State<'_, Arc<update::UpdateSlot>>) -> UpdateStatus {
    slot.status()
}

/// Fetch and verify a local model (docs/dictation.md §10); progress arrives as `models` events.
#[tauri::command]
fn model_download(bridge: tauri::State<'_, Bridge>, id: String) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::ModelDownload { id })?)
}

/// Stop a running model download; the partial files stay for a resume.
#[tauri::command]
fn model_cancel(bridge: tauri::State<'_, Bridge>, id: String) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::ModelCancel { id })?)
}

/// Delete a model directory.
#[tauri::command]
fn model_remove(bridge: tauri::State<'_, Bridge>, id: String) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::ModelRemove { id })?)
}

/// Append a personal dictionary entry (docs/dictation.md §16.4); `history_id` marks one added from
/// a history entry. A draft that is wrong on its own is refused here; list conflicts come back as
/// an `error` event.
#[tauri::command]
fn dictionary_add(bridge: tauri::State<'_, Bridge>, entry: DictionaryDraft, history_id: Option<String>) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::DictionaryAdd { entry, history_id })?)
}

/// Replace an entry's term, mis-hearings and flag.
#[tauri::command]
fn dictionary_update(bridge: tauri::State<'_, Bridge>, id: String, entry: DictionaryDraft) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::DictionaryUpdate { id, entry })?)
}

/// Delete a dictionary entry.
#[tauri::command]
fn dictionary_remove(bridge: tauri::State<'_, Bridge>, id: String) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::DictionaryRemove { id })?)
}

/// Reorder the dictionary (every id, new order; order = glossary priority).
#[tauri::command]
fn dictionary_reorder(bridge: tauri::State<'_, Bridge>, ids: Vec<String>) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::DictionaryReorder { ids })?)
}

/// Append a replacement rule; an invalid regex is refused here with the compiler's message.
#[tauri::command]
fn rules_add(bridge: tauri::State<'_, Bridge>, rule: RuleDraft) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::RulesAdd { rule })?)
}

/// Replace a rule (id and position stay).
#[tauri::command]
fn rules_update(bridge: tauri::State<'_, Bridge>, id: String, rule: RuleDraft) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::RulesUpdate { id, rule })?)
}

/// Delete a rule.
#[tauri::command]
fn rules_remove(bridge: tauri::State<'_, Bridge>, id: String) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::RulesRemove { id })?)
}

/// Reorder the rules (every id, new order; order = execution order).
#[tauri::command]
fn rules_reorder(bridge: tauri::State<'_, Bridge>, ids: Vec<String>) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::RulesReorder { ids })?)
}

/// Import rules from pasted TOML (`replace` | `merge`, docs/dictation.md §16.5): a file that does
/// not parse or validate is refused here, with its position, and nothing changes.
#[tauri::command]
fn rules_import(bridge: tauri::State<'_, Bridge>, toml: String, mode: ImportMode) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::RulesImport { toml, mode })?)
}

/// Query: the rules as TOML text (the export dialog shows it for copying).
#[tauri::command]
fn rules_export(bridge: tauri::State<'_, Bridge>) -> Result<String, String> {
    Ok(bridge.rules_export()?)
}

/// Query: `text` through the dictionary and the rules with the pipeline's own code, optionally with
/// an unsaved rule `draft` in place (docs/dictation.md §16.4).
#[tauri::command]
fn vocabulary_preview(bridge: tauri::State<'_, Bridge>, text: String, draft: Option<PreviewDraft>) -> Result<VocabularyPreview, String> {
    Ok(bridge.vocabulary_preview(&text, draft.as_ref())?)
}

/// Append a scene (docs/dictation.md §18.6). A draft that is wrong on its own (no app, a bad
/// language code, over a limit) is refused here; a clash with the list comes back as an `error` event.
#[tauri::command]
fn scenes_add(bridge: tauri::State<'_, Bridge>, scene: SceneDraft) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::ScenesAdd { scene })?)
}

/// Replace a scene's name, flag, match and overrides (id and position stay).
#[tauri::command]
fn scenes_update(bridge: tauri::State<'_, Bridge>, id: String, scene: SceneDraft) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::ScenesUpdate { id, scene })?)
}

/// Delete a scene.
#[tauri::command]
fn scenes_remove(bridge: tauri::State<'_, Bridge>, id: String) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::ScenesRemove { id })?)
}

/// Reorder the scenes (every id, new order; order = matching order).
#[tauri::command]
fn scenes_reorder(bridge: tauri::State<'_, Bridge>, ids: Vec<String>) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::ScenesReorder { ids })?)
}

/// 恢复默认 on a built-in scene (docs/dictation.md §18.10): its applications and overrides back to
/// the defaults; the core re-emits `scenes` (a scene of the user's is refused).
#[tauri::command]
fn scenes_restore(bridge: tauri::State<'_, Bridge>, id: String) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::ScenesRestore { id })?)
}

/// One built-in scene's term pack, what 查看术语 lists.
#[derive(serde::Serialize)]
struct BuiltinSceneTerms {
    id: voltip_core::BuiltinScene,
    terms: &'static [&'static str],
}

/// Query: every built-in scene's term pack (§18.10), in the order the scene list appends them.
#[tauri::command]
fn scenes_builtin() -> Vec<BuiltinSceneTerms> {
    voltip_core::BuiltinScene::ALL.into_iter().map(|id| BuiltinSceneTerms { id, terms: voltip_core::vocabulary::packs::terms(id) }).collect()
}

/// Append a custom preset (docs/dictation.md §21). A draft that is wrong on its own is refused
/// here; a clash with the list (a duplicate name, the cap) comes back as an `error` event.
#[tauri::command]
fn presets_add(bridge: tauri::State<'_, Bridge>, preset: PresetDraft) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::PresetsAdd { preset })?)
}

/// Replace a custom preset's name and instruction.
#[tauri::command]
fn presets_update(bridge: tauri::State<'_, Bridge>, id: String, preset: PresetDraft) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::PresetsUpdate { id, preset })?)
}

/// Delete a custom preset.
#[tauri::command]
fn presets_remove(bridge: tauri::State<'_, Bridge>, id: String) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::PresetsRemove { id })?)
}

/// 试一试: `text` through the current clean-up with a saved preset or the instruction being
/// edited; the answer arrives as a `preset_try` event carrying `id`.
#[tauri::command]
fn presets_try(bridge: tauri::State<'_, Bridge>, id: u64, preset: Option<String>, prompt: Option<String>, text: String) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::PresetsTry { id, preset, prompt, text })?)
}

/// Query: every built-in preset's text (task, rules, examples; the output contract is added to
/// every preset and is not part of it), in the order the interface lists them: what 复制为自定义
/// starts from.
#[tauri::command]
fn presets_builtin() -> Vec<dictation::BuiltinPresetText> {
    dictation::builtin_preset_texts()
}

/// What of a take's context may go to the LLM (§18.5); the core persists and re-emits `settings`.
#[tauri::command]
fn settings_set_context_sharing(bridge: tauri::State<'_, Bridge>, app_name: bool, window_title: bool) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::SettingsSetContextSharing { app_name, window_title })?)
}

/// Query: the applications the history saw, newest first (the scene editor's picker, §18.6).
/// Run a history read off the main thread: SQLite blocks (docs/dictation.md §4.4).
async fn history_read<T: Send + 'static>(
    bridge: &Bridge,
    read: impl FnOnce(&Bridge) -> Result<T, voltip_tauri_bridge::BridgeError> + Send + 'static,
) -> Result<T, String> {
    let bridge = bridge.clone();
    Ok(tauri::async_runtime::spawn_blocking(move || read(&bridge)).await.map_err(|e| e.to_string())??)
}

/// The applications the history saw, newest first (docs/dictation.md §18.6): what the scene
/// editor offers to pick from.
#[tauri::command]
async fn recent_apps(bridge: tauri::State<'_, Bridge>) -> Result<Vec<AppRef>, String> {
    history_read(&bridge, Bridge::recent_apps).await
}

/// A page of the history (docs/dictation.md §4.4), filtered, searched and paged in the database:
/// the entries at or after `since_ms` (a local midnight), starred ones, ones not inserted, ones
/// containing `query`; `limit` ≤ 200.
#[tauri::command]
async fn history_query(
    bridge: tauri::State<'_, Bridge>,
    since_ms: Option<u64>,
    starred: Option<bool>,
    failed: Option<bool>,
    query: Option<String>,
    offset: Option<u32>,
    limit: u32,
) -> Result<voltip_core::HistoryPage, String> {
    let query = voltip_core::HistoryQuery {
        since_ms,
        starred: starred.unwrap_or(false),
        failed: failed.unwrap_or(false),
        query: query.unwrap_or_default(),
        offset: offset.unwrap_or(0),
        limit,
    };
    history_read(&bridge, move |b| b.history_query(&query)).await
}

/// One history entry by id; `null` once it is gone.
#[tauri::command]
async fn history_entry(bridge: tauri::State<'_, Bridge>, id: uuid::Uuid) -> Result<Option<voltip_core::HistoryEntry>, String> {
    history_read(&bridge, move |b| b.history_entry(id)).await
}

/// The home page's statistics (docs/dictation.md §4.5): the dictations between each two of the
/// local midnights in `boundaries`, and over the whole history.
#[tauri::command]
async fn history_stats(bridge: tauri::State<'_, Bridge>, boundaries: Vec<u64>) -> Result<voltip_core::HistoryStats, String> {
    history_read(&bridge, move |b| b.history_stats(&boundaries)).await
}

/// How often each dictionary entry and rule fired in the history (docs/dictation.md §16.3).
#[tauri::command]
async fn history_hits(bridge: tauri::State<'_, Bridge>) -> Result<voltip_core::HistoryHits, String> {
    history_read(&bridge, Bridge::history_hits).await
}

/// Microphones the native audio backend can open (`voltip-audio`), default first.
#[tauri::command]
async fn audio_devices() -> Result<Vec<audio::Device>, String> {
    audio::devices().await
}

/// The output devices a take can record the computer's sound from, default first, and whether that
/// works here (docs/dictation.md §22).
#[tauri::command]
async fn audio_outputs() -> Result<audio::Outputs, String> {
    audio::outputs().await
}

/// Subscribe to the input level stream for `device_id` (default device when `None`); frames arrive
/// on `on_frame`. Returns the subscription id for `audio_meter_stop`. The hub owns the microphone:
/// while a dictation records, frames come from the recorder instead of a second device open.
#[tauri::command]
async fn audio_meter_start(
    hub: tauri::State<'_, Arc<audio::AudioHub>>,
    device_id: Option<String>,
    on_frame: tauri::ipc::Channel<audio::Frame>,
) -> Result<u64, String> {
    audio::start(hub.inner().clone(), device_id, on_frame).await
}

/// Remove one level subscriber; the device meter closes with the last one.
#[tauri::command]
fn audio_meter_stop(hub: tauri::State<'_, Arc<audio::AudioHub>>, id: u64) -> Result<(), String> {
    hub.unsubscribe(id);
    Ok(())
}

/// The state the pill window should show right now (pulled by the overlay webview on mount).
#[tauri::command]
fn overlay_state(slot: tauri::State<'_, overlay::OverlaySlot>) -> String {
    slot.current()
}

/// Query (docs/dictation.md §15.1): what the OS currently grants. The onboarding step and the home
/// page's notice poll it every second while on screen. Linux answers `not_applicable` for
/// everything. A newly granted Accessibility also brings a lone-key trigger that failed without
/// it back (`hotkey::after_permission_read`): granting needs no restart.
#[tauri::command]
async fn permissions_status<R: Runtime>(app: tauri::AppHandle<R>) -> platform::PermissionReport {
    let report = platform::permissions_status().await;
    if let (Some(bridge), Some(registry)) = (app.try_state::<Bridge>(), app.try_state::<Arc<hotkey::HotkeyRegistry>>()) {
        hotkey::after_permission_read(bridge.inner(), registry.inner(), report.accessibility);
    }
    report
}

/// Ask the OS for one permission (macOS: the system prompt / System Settings pane). A no-op that
/// still succeeds where nothing needs asking, so the webview has no platform branch.
#[tauri::command]
async fn permissions_request(permission: platform::Permission) -> Result<(), String> {
    platform::permissions_request(permission).await
}

/// Query (§15.3): would an injection into the current foreground window land? Windows compares
/// integrity levels and checks for the secure desktop; other hosts answer `proceed`, unchecked.
#[tauri::command]
fn inject_preflight() -> platform::InjectPreflight {
    platform::inject_preflight()
}

/// 「粘贴到上一个窗口」 on the home and history pages (`paste::paste_text`): refused while a take
/// runs; otherwise Voltip gets out of the way and the core pastes into the window that came to the
/// front, or copies. Answers what became of the text; the main window comes back unless it went
/// into the other window.
#[tauri::command]
async fn paste_text<R: Runtime>(app: tauri::AppHandle<R>, text: String) -> voltip_core::paste::PasteOutcome {
    paste::paste_text(&app, text).await
}

/// What the shell wires besides the core.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShellOptions {
    /// Register `Settings.hotkey` with the OS through the global-shortcut plugin (and the
    /// single-instance plugin that forwards `voltip --toggle` / `--cancel`). Needs a real window
    /// system; the mock runtime used by tests turns it off.
    pub global_hotkey: bool,
    /// `voltip --start-hidden`: leave the main window hidden (tray and hotkey only). The window is
    /// declared invisible in `tauri.conf.json` and shown from the setup hook otherwise.
    pub start_hidden: bool,
}

impl ShellOptions {
    /// Production wiring.
    pub const PRODUCTION: Self = Self { global_hotkey: true, start_hidden: false };
    /// Headless / mock-runtime wiring.
    pub const HEADLESS: Self = Self { global_hotkey: false, start_hidden: false };

    /// Production wiring with `--start-hidden`.
    pub const fn hidden(self, start_hidden: bool) -> Self {
        Self { start_hidden, ..self }
    }
}

/// Label of the main window in `tauri.conf.json`.
pub const MAIN_WINDOW: &str = "main";

/// Bring the main window to the front: a second launch without arguments, a tray click, and the
/// macOS Dock / `open -a` reopen all end here. Nothing to do when the window does not exist.
pub fn show_main_window<R: Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Apply a second invocation's arguments to this (running) instance: `--quit` exits, `--toggle` /
/// `--edit-toggle` / `--cancel` reach the core as a CLI edge of the dictation or the edit key / a
/// cancel; anything else brings the main window to the front.
pub fn on_second_instance<R: Runtime>(app: &tauri::AppHandle<R>, args: &[String]) {
    if cli::quit_from_args(args) {
        tracing::info!("quit requested by a second invocation");
        app.exit(0);
        return;
    }
    match cli::remote_from_args(args) {
        Some(remote) => match app.try_state::<Bridge>() {
            Some(bridge) => {
                if let Err(e) = bridge.dispatch(remote.command()) {
                    tracing::warn!(error = %e, ?remote, "remote control not accepted");
                } else {
                    tracing::info!(?remote, "remote control applied");
                }
            }
            None => tracing::warn!(?remote, "remote control before the core is up; dropped"),
        },
        None => show_main_window(app),
    }
}

/// Start the core with the shell's dictation `ports`, forward every [`voltip_core::ui::UiEvent`]
/// onto the webview event bus and manage the [`Bridge`] as Tauri state so the commands above can
/// reach it.
pub fn attach_bridge<R: Runtime>(
    app: &tauri::AppHandle<R>,
    config: CoreConfig,
    store: Arc<dyn SecretStore>,
    options: ShellOptions,
    ports: dictation::ShellPorts,
) -> Result<(), BridgeError> {
    let dictation::ShellPorts { dictation: ports, hub } = ports;
    // The paste button asks the same probe the core does (`paste::PasteProbe`).
    let paste_probe = paste::PasteProbe(ports.probe.clone());
    // The updater exists only in the production wiring and only when the build carries a source; a
    // config managed on the builder before `setup` wins (tests point it at a local manifest server
    // and register the plugin themselves).
    let updater_config = app
        .try_state::<update::UpdaterConfig>()
        .map(|c| c.inner().clone())
        .or_else(|| if options.global_hotkey { update::UpdaterConfig::from_build() } else { None });
    let updater = Arc::new(update::UpdateSlot::new(updater_config, &config.data_dir));
    app.manage(restart::Restart::new(store.clone()));
    // Tauri runs `setup` on the UI thread, outside any Tokio context; the core spawns its tasks
    // with `tokio::spawn`, so it has to be started from Tauri's own runtime.
    // Subscribed before the core starts: the first `state` event must reach the webview bus even
    // when the core is ready before this thread gets to subscribe (broadcast drops earlier events).
    let (bridge, mut events) = tauri::async_runtime::block_on(async { Bridge::start_subscribed(config, store, ports) })?;
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            match events.recv().await {
                Ok(ev) => {
                    if let Err(e) = handle.emit(UI_EVENT_NAME, &ev) {
                        tracing::warn!(error = %e, "emit failed");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => tracing::warn!(skipped = n, "webview lagged; events dropped"),
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    // What the local models can run on (docs/dictation.md §10.6), off the UI thread: the first
    // enumeration initialises the build's GPU backends.
    {
        let bridge = bridge.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let machine = voltip_asr_local::hardware();
            tracing::info!(cpu_threads = machine.cpu_threads, gpus = ?machine.gpus.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), "compute devices");
            bridge.publish(UiEvent::Hardware(dictation::hardware_status(&machine)));
        });
    }
    // The global hotkey follows `Settings.hotkey`: registered now, re-registered on every change,
    // and reported back into the same state the webview reads.
    let registry = Arc::new(hotkey::HotkeyRegistry::default());
    if options.global_hotkey {
        // The pill window is prewarmed hidden so the first press only has to show it.
        if let Err(e) = overlay::prewarm(app) {
            tracing::warn!(error = %e, "overlay prewarm failed; it will be created on first use");
        }
        overlay::follow_dictation(app.clone(), bridge.clone());
        hotkey::follow_settings(app.clone(), bridge.clone(), registry.clone());
        audio::follow_phone_takes(bridge.clone(), hub.clone());
        platform::install_tray(app, &bridge, options.start_hidden, updater.enabled());
    }
    // The main window is declared invisible so `--start-hidden` never flashes it; every other start
    // shows it now that the core is up.
    if !options.start_hidden
        && let Some(window) = app.get_webview_window(MAIN_WINDOW)
        && let Err(e) = window.show()
    {
        tracing::warn!(error = %e, "main window show failed");
    }
    // `core_state().update` and the stream agree from the first frame: `disabled` or `idle`.
    bridge.publish(UiEvent::Update(updater.status()));
    if updater.enabled() {
        update::follow_settings(app.clone(), bridge.clone(), updater.clone());
    }
    app.manage(updater);
    app.manage(registry);
    app.manage(bridge);
    app.manage(options);
    app.manage(hub);
    app.manage(overlay::OverlaySlot::default());
    app.manage(feedback::Attachments::default());
    app.manage(paste_probe);
    Ok(())
}

/// Register the plugins, the command handlers and the setup hook that attaches the bridge.
/// [`run`] feeds it `tauri::Builder::default()` and [`dictation::production_ports`]; tests feed it
/// `tauri::test::mock_builder()` and `voltip_core::dictation::fakes::ports()`.
pub fn build_app<R: Runtime>(
    builder: tauri::Builder<R>,
    config: CoreConfig,
    store: Arc<dyn SecretStore>,
    options: ShellOptions,
    ports: dictation::ShellPorts,
) -> tauri::Builder<R> {
    // Single instance first (the plugin's own requirement): a second `voltip` hands its arguments
    // to this process and exits, so `--toggle` / `--cancel` are remote controls.
    let builder =
        if options.global_hotkey { builder.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| on_second_instance(app, &args))) } else { builder };
    let builder = builder.plugin(tauri_plugin_opener::init()).plugin(tauri_plugin_dialog::init());
    let builder = if options.global_hotkey { builder.plugin(tauri_plugin_global_shortcut::Builder::new().build()) } else { builder };
    // The updater plugin reads `plugins.updater` from the context (`run` injects it from the build
    // environment); without a source it is not registered at all.
    let builder = if options.global_hotkey && update::UpdaterConfig::from_build().is_some() {
        builder.plugin(tauri_plugin_updater::Builder::new().build())
    } else {
        builder
    };
    // Closing the main window hides it where the tray or the Dock brings it back and quits
    // elsewhere (`platform::on_window_event`).
    let builder = builder.on_window_event(platform::on_window_event);
    builder.setup(move |app| Ok(attach_bridge(app.handle(), config, store, options, ports).map_err(|e| std::io::Error::other(e.to_string()))?)).invoke_handler(
        tauri::generate_handler![
            core_state,
            pairing_start,
            pairing_join_code,
            pairing_join_ticket,
            pairing_confirm,
            pairing_reject,
            pairing_cancel,
            pairing_reset,
            device_forget,
            device_rename,
            send_text,
            phone_take_start,
            phone_take_stop,
            phone_take_cancel,
            phone_text_send,
            sent_texts_clear,
            phone_clipboard_read,
            settings_set_lan_discovery,
            settings_set_pairing_always_on,
            pairing_join_nearby,
            settings_set_relay,
            settings_set_theme,
            settings_set_hotkey,
            settings_set_edit_hotkey,
            settings_set_solo_key,
            settings_set_microphone,
            settings_set_recording,
            hotkey_capture,
            devices_refresh,
            connectivity_check,
            audio_devices,
            audio_outputs,
            audio_meter_start,
            audio_meter_stop,
            overlay_state,
            dictation_start,
            dictation_stop,
            dictation_cancel,
            hotkey_edge,
            settings_set_activation,
            settings_set_engines,
            provider_key_set,
            provider_probe,
            provider_console_open,
            project_link_open,
            feedback_diagnostics,
            feedback_submit,
            feedback_attachment_add,
            feedback_attachment_remove,
            feedback_attachments_clear,
            history_delete,
            history_clear,
            history_star,
            history_process,
            history_process_cancel,
            history_export,
            settings_set_locale,
            settings_set_auto_update,
            settings_set_history,
            settings_set_overlay,
            update_check,
            update_install,
            update_status,
            model_download,
            model_cancel,
            model_remove,
            dictionary_add,
            dictionary_update,
            dictionary_remove,
            dictionary_reorder,
            rules_add,
            rules_update,
            rules_remove,
            rules_reorder,
            rules_import,
            rules_export,
            vocabulary_preview,
            scenes_add,
            scenes_update,
            scenes_remove,
            scenes_reorder,
            scenes_restore,
            scenes_builtin,
            presets_add,
            presets_update,
            presets_remove,
            presets_try,
            presets_builtin,
            settings_set_context_sharing,
            recent_apps,
            history_query,
            history_entry,
            history_stats,
            history_hits,
            permissions_status,
            permissions_request,
            inject_preflight,
            paste_text
        ],
    )
}

/// Build and run the Tauri application with the production data dir and keychain. Parses the
/// command line first (docs/dictation.md §13): the headless flags run and exit here without a
/// window, tray, hotkey or microphone; `--toggle` / `--cancel` reach a running instance through
/// the single-instance plugin (with no instance running they start the GUI and are dropped).
pub fn run() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,voltip=debug"));
    // Logs go to stderr in every mode: stdout carries only headless command output (`--json` must
    // stay parseable), and a writer on stdout would contend with the CLI's own writes. Colour only
    // on a terminal (and never with NO_COLOR): piped stderr is a log file or a script's capture.
    let ansi = std::io::IsTerminal::is_terminal(&std::io::stderr()) && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty());
    let _ = tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).with_ansi(ansi).try_init();
    // Before anything else: a macOS update that started this build hands the keychain over here.
    #[cfg(target_os = "macos")]
    keychain_handoff::receive_at_startup();

    let args = match cli::Cli::parse_args(std::env::args_os()) {
        Ok(cli) => cli,
        // `--help` / `--version` and usage errors: clap prints and picks the exit code.
        Err(e) => e.exit(),
    };
    let action = args.action();
    let quit = action == cli::Action::Quit;
    let start_hidden = match &action {
        cli::Action::Gui { start_hidden, remote } => {
            if let Some(remote) = remote {
                tracing::info!(?remote, "no running instance to control; starting the GUI");
            }
            *start_hidden
        }
        // Built like the GUI below so the single-instance plugin can hand `--quit` to a running
        // instance (and end this process); getting past `build` means there is none.
        cli::Action::Quit => true,
        headless => {
            tracing::debug!(?headless, "headless action");
            // Unlocked handles: each write takes the lock briefly. Holding `stdout().lock()` across
            // the call deadlocked `--transcribe-file` as soon as a worker thread logged, and holding
            // `stderr().lock()` would do the same now that the subscriber writes there.
            let code = cli::run_headless(headless, &data_dir(), &mut std::io::stdout(), &mut std::io::stderr());
            exit::exit_process(code.code());
        }
    };

    let mut context = tauri::generate_context!();
    if let Some(updater) = update::UpdaterConfig::from_build() {
        // The manifest URL and public key come from the build environment, never from tauri.conf.json.
        context.config_mut().plugins.0.insert("updater".to_owned(), updater.plugin_config());
    }
    let mut config = CoreConfig::new(data_dir());
    config.client_version = format!("voltip/{APP_VERSION}");
    config.app_version = APP_VERSION.to_owned();
    // LAN discovery (docs/pairing.md 「局域网发现」): phones find this desktop, and each other's
    // address after it changed, without a relay.
    config.discovery = match voltip_core::discovery::MdnsDiscovery::new() {
        Ok(mdns) => Some(mdns),
        Err(e) => {
            tracing::warn!(error = %e, "LAN discovery unavailable");
            None
        }
    };
    let ports = dictation::production_ports(config.models_root.clone());
    let mut app = match build_app(tauri::Builder::default(), config, secret_store(), ShellOptions::PRODUCTION.hidden(start_hidden), ports).build(context) {
        Ok(app) => app,
        Err(e) => {
            tracing::error!(error = %e, "tauri failed to build");
            std::process::exit(1);
        }
    };
    if quit {
        tracing::info!("--quit: no running instance");
        exit::exit_process(0);
    }
    // macOS: the activation policy goes in between `build` and `run` (docs/dictation.md §15.2).
    platform::before_run(&mut app, start_hidden);
    // Linux leaves through `exit::exit_process` once Tauri has cleaned up (a restart after an
    // update still goes through Tauri's own exit); elsewhere `run` exits as usual.
    #[cfg(target_os = "linux")]
    {
        let code = app.run_return(|app, event| platform::on_run_event(app, &event));
        exit::exit_process(code);
    }
    #[cfg(not(target_os = "linux"))]
    app.run(|app, event| platform::on_run_event(app, &event));
}
