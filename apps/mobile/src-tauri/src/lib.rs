//! Voltip mobile shell: the same command surface as the desktop, with the Android Keystore as
//! the secret store, the barcode-scanner plugin for QR pairing, and the phone's microphone for the
//! takes it streams to a paired desktop (docs/dictation.md §20, [`microphone`]) — or, with no
//! paired desktop online, recognises itself through the built-in cloud services, the result
//! landing on the phone's clipboard and in its own history (§20.7, [`phone_ports`]).
//!
//! Everything except [`run`] is generic over the Tauri runtime so `tests/ipc.rs` drives the real
//! command layer on `tauri::test::MockRuntime` (desktop host, no keystore, no window).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod clipboard;
pub mod meter;
pub mod microphone;
pub mod multicast;
pub mod share;

use std::sync::Arc;

use tauri::{AppHandle, Emitter as _, Manager as _, Runtime};
use voltip_core::ui::{ProjectLink, UI_EVENT_NAME, UiState, UpdateStatus};
use voltip_core::{
    Activation, AppRef, CoreConfig, DictionaryDraft, EdgeSource, EngineSettings, ImportMode, Locale, OverlayPlacement, PresetDraft, PreviewDraft, ProviderId,
    RuleDraft, SceneDraft, ServiceKind, TakeKind, ThemeId, VocabularyPreview,
};
use voltip_identity::SecretStore;
use voltip_tauri_bridge::{Bridge, BridgeError, UiCommand};

/// Keystore service id.
pub const KEYSTORE_SERVICE: &str = "dev.voltip.mobile";

/// Every command the webview may invoke, in registration order. Must equal the desktop shell's
/// list, `packages/shared/src/schema.ts` (`CommandArgs`) and `fixtures/ipc/commands.json`.
pub const COMMANDS: [&str; 100] = [
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
    "settings_set_pinned_scene",
    "recent_apps",
    "history_query",
    "history_entry",
    "history_stats",
    "history_hits",
    "permissions_status",
    "permissions_request",
    "inject_preflight",
    "paste_text",
    "phone_share_text",
];

/// App data directory.
pub fn data_dir<R: Runtime>(app: &AppHandle<R>) -> std::path::PathBuf {
    app.path().app_data_dir().unwrap_or_else(|_| std::env::temp_dir().join("voltip-mobile"))
}

/// Core configuration for a real device: platform data dir, `<platform> 手机` as the first name.
pub fn production_config<R: Runtime>(app: &AppHandle<R>) -> CoreConfig {
    let mut config = CoreConfig::new(data_dir(app));
    config.default_device_name = format!("{} 手机", platform_label());
    // The app version (tauri.conf.json → package.json); the Cargo version is static.
    config.client_version = format!("voltip/{}", app.package_info().version);
    config.app_version = app.package_info().version.to_string();
    // The phone is the microphone; a desktop records its takes, never the other way round.
    config.accepts_phone_takes = false;
    // The user picks a take's scene on the talk card (no foreground probe on a phone).
    config.manual_scenes = true;
    // LAN discovery (docs/pairing.md 「局域网发现」): Android drops multicast without the lock,
    // held while the switch is on.
    let discovering = voltip_core::SettingsStore::new(&config.data_dir).load().map_or(true, |s| s.lan_discovery);
    multicast::hold_in_background(app, discovering);
    config.discovery = match voltip_core::discovery::MdnsDiscovery::new() {
        Ok(mdns) => Some(mdns),
        Err(e) => {
            tracing::warn!(error = %e, "LAN discovery unavailable");
            None
        }
    };
    config
}

/// Platform secret store. Android: the Keystore, and nothing less secure; desktop hosts (dev
/// runs, CI): the OS keychain. Nothing is touched until the first read.
pub fn secret_store() -> Arc<dyn SecretStore> {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        match voltip_identity::AndroidKeystoreSecretStore::new(KEYSTORE_SERVICE, "voltip") {
            Ok(store) => Arc::new(store),
            Err(e) => {
                tracing::error!(error = %e, "Android Keystore unavailable; refusing to fall back to an insecure store");
                std::process::exit(1);
            }
        }
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let user = std::env::var("USER").unwrap_or_else(|_| "default".into());
        Arc::new(voltip_identity::KeyringSecretStore::new(KEYSTORE_SERVICE, user))
    }
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

/// Stream a take to the paired desktop `public_key` (docs/dictation.md §20); on Android the
/// microphone permission is asked for first.
#[tauri::command]
async fn phone_take_start<R: Runtime>(app: AppHandle<R>, public_key: String) -> Result<(), String> {
    microphone::ensure_permission(&app).await?;
    let bridge = app.state::<Bridge>();
    Ok(bridge.dispatch(UiCommand::PhoneTakeStart { public_key })?)
}

#[tauri::command]
fn phone_take_stop(bridge: tauri::State<'_, Bridge>) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::PhoneTakeStop)?)
}

#[tauri::command]
fn phone_take_cancel(bridge: tauri::State<'_, Bridge>) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::PhoneTakeCancel)?)
}

/// Send text for the paired desktop `public_key` to insert at its cursor (docs/dictation.md §20.6).
#[tauri::command]
fn phone_text_send(bridge: tauri::State<'_, Bridge>, public_key: String, body: String, source: voltip_core::phone::PhoneTextSource) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::PhoneTextSend { public_key, body, source })?)
}

/// LAN discovery (docs/pairing.md 「局域网发现」): announce this device and browse for the others;
/// the multicast lock follows the switch.
#[tauri::command]
fn settings_set_lan_discovery<R: Runtime>(app: AppHandle<R>, bridge: tauri::State<'_, Bridge>, enabled: bool) -> Result<(), String> {
    bridge.dispatch(UiCommand::SettingsSetLanDiscovery { enabled })?;
    multicast::hold_in_background(&app, enabled);
    Ok(())
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

/// Forget the list of sent texts.
#[tauri::command]
fn sent_texts_clear(bridge: tauri::State<'_, Bridge>) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::SentTextsClear)?)
}

/// The phone's clipboard text, `null` when it holds none (`PhoneClipboardPlugin.kt` on Android).
#[tauri::command]
async fn phone_clipboard_read<R: Runtime>(app: AppHandle<R>) -> Result<serde_json::Value, String> {
    let text = clipboard::read_text(&app).await?;
    Ok(serde_json::json!({ "text": text }))
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

/// The voice-edit hotkey is a shared setting (docs/dictation.md §19): the phone edits it like the
/// desktop does; only the desktop registers it.
#[tauri::command]
fn settings_set_edit_hotkey(bridge: tauri::State<'_, Bridge>, hotkey: Option<String>) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::SettingsSetEditHotkey { hotkey })?)
}

/// The lone-key trigger (docs/dictation.md §13.1) is a shared setting: the phone stores it like
/// the desktop does; only the desktop's input hook watches the key.
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

/// Phones register no OS hotkey; the recorder's suspend request is accepted and ignored so the
/// shared webview code needs no platform branch.
#[tauri::command]
fn hotkey_capture(active: bool) -> Result<(), String> {
    tracing::debug!(active, "hotkey_capture ignored on mobile");
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

/// Nothing to pick: a take records from the system's default input, which AAudio routes (the
/// headset or Bluetooth microphone when one is connected), so the list is empty.
#[tauri::command]
fn audio_devices() -> Result<Vec<serde_json::Value>, String> {
    Ok(Vec::new())
}

/// The phone records its own microphone (docs/dictation.md §22): the computer's sound is a desktop
/// source.
#[tauri::command]
fn audio_outputs() -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({ "system_audio": { "state": "unsupported" }, "devices": [] }))
}

/// Stream the level of the phone's own takes to `on_frame` ([`meter::Meters`]): frames arrive
/// while a take records and the meter opens no microphone of its own. `device_id` has no effect
/// (see [`audio_devices`]).
#[tauri::command]
fn audio_meter_start(
    bridge: tauri::State<'_, Bridge>,
    meters: tauri::State<'_, meter::Meters>,
    device_id: Option<String>,
    on_frame: tauri::ipc::Channel<voltip_core::dictation::LevelFrame>,
) -> Result<u64, String> {
    if device_id.is_some() {
        tracing::debug!(?device_id, "audio_meter_start: the phone records from its default input");
    }
    Ok(meters.start(bridge.levels(), move |frame| on_frame.send(frame).is_ok()))
}

/// End one meter subscription.
#[tauri::command]
fn audio_meter_stop(meters: tauri::State<'_, meter::Meters>, id: u64) -> Result<(), String> {
    meters.stop(id);
    Ok(())
}

/// The phone has no pill window; the query answers `blank` so shared code needs no branch.
#[tauri::command]
fn overlay_state() -> String {
    "blank".to_owned()
}

/// A take the phone recognises itself (docs/dictation.md §20.7): 「按住说话」 with no paired
/// desktop online. On Android the microphone permission is asked for first, as for a phone take.
#[tauri::command]
async fn dictation_start<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    microphone::ensure_permission(&app).await?;
    let bridge = app.state::<Bridge>();
    Ok(bridge.dispatch(UiCommand::DictationStart)?)
}

/// Close the microphone and run ASR → refine → the phone's clipboard.
#[tauri::command]
fn dictation_stop(bridge: tauri::State<'_, Bridge>) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::DictationStop)?)
}

/// Discard the recording or the pending result (the finger slid off the button).
#[tauri::command]
fn dictation_cancel(bridge: tauri::State<'_, Bridge>) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::DictationCancel)?)
}

/// Why the phone refuses a key edge: it has no hotkey (and no voice edit); its takes start from
/// the button. Honest error, not a silent no-op.
pub const HOTKEY_UNAVAILABLE: &str = "hotkey: 手机端没有快捷键";

/// No hotkey on the phone: a key edge (dictation or voice edit) is refused.
#[tauri::command]
fn hotkey_edge(pressed: bool, at_ms: Option<u64>, source: Option<EdgeSource>, purpose: Option<TakeKind>, chorded: Option<bool>) -> Result<(), String> {
    tracing::debug!(pressed, ?at_ms, ?source, ?purpose, ?chorded, "hotkey_edge refused on mobile");
    Err(HOTKEY_UNAVAILABLE.to_owned())
}

/// The activation mode is a shared setting (docs/dictation.md §13): the phone edits it like the
/// desktop does; only the desktop's key acts on it.
#[tauri::command]
fn settings_set_activation(bridge: tauri::State<'_, Bridge>, activation: Activation, hold_threshold_ms: u32, extra_recording_ms: u32) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::SettingsSetActivation { activation, hold_threshold_ms, extra_recording_ms })?)
}

/// Engine settings are shared state: the phone edits them like the desktop does.
#[tauri::command]
fn settings_set_engines(bridge: tauri::State<'_, Bridge>, engines: EngineSettings) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::SettingsSetEngines { engines })?)
}

#[tauri::command]
fn provider_key_set(bridge: tauri::State<'_, Bridge>, provider: ProviderId, kind: ServiceKind, value: Option<String>) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::ProviderKeySet { provider, kind, value })?)
}

/// 测试连接 (docs/dictation.md §3.3): the core lists the provider's models through the phone's
/// HTTP probe (`voltip_cloud::HttpServiceProbe`) and answers with a `provider_probe` event.
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

/// Open the vendor's API-key page in the phone's browser. Only catalogue URLs can be opened: the
/// webview names a provider, never a URL (as on the desktop).
#[tauri::command]
fn provider_console_open<R: Runtime>(app: AppHandle<R>, provider: ProviderId) -> Result<(), String> {
    let url = provider.spec().console_url.ok_or_else(|| format!("{}: no key page", provider.as_str()))?;
    open_in_browser(&app, url)
}

/// The repository this build comes from (`Cargo.toml` `repository`): what 关于 opens.
pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

/// Open a project page (the repository, its releases) in the phone's browser; the webview names
/// the page and the shell builds the URL.
#[tauri::command]
fn project_link_open<R: Runtime>(app: AppHandle<R>, link: ProjectLink) -> Result<(), String> {
    open_in_browser(&app, &link.url(REPOSITORY))
}

/// `url` in the browser: Android's `ACTION_VIEW` through the opener plugin. The desktop-hosted
/// builds of this crate (tests) have no browser to hand it to.
fn open_in_browser<R: Runtime>(app: &AppHandle<R>, url: &str) -> Result<(), String> {
    #[cfg(mobile)]
    {
        use tauri_plugin_opener::OpenerExt as _;
        app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
    }
    #[cfg(not(mobile))]
    {
        let _ = (app, url);
        Err(BROWSER_UNAVAILABLE.into())
    }
}

/// Why a desktop-hosted build of the phone shell (tests) opens nothing.
pub const BROWSER_UNAVAILABLE: &str = "opener: 这个平台没有手机浏览器";

/// Feedback is sent from the computer (its 反馈 dialog, docs/feedback.md).
pub const FEEDBACK_UNAVAILABLE: &str = "feedback: 请在电脑上反馈";

#[tauri::command]
fn feedback_diagnostics(_locale: String) -> Result<(), String> {
    Err(FEEDBACK_UNAVAILABLE.into())
}

#[tauri::command]
fn feedback_submit(_kind: String, _message: String, _contact: Option<String>, _locale: String, _attachments: Option<Vec<String>>) -> Result<(), String> {
    Err(FEEDBACK_UNAVAILABLE.into())
}

/// Same answer as `feedback_submit`: the phone stages no attachments.
#[tauri::command]
fn feedback_attachment_add() -> Result<(), String> {
    Err(FEEDBACK_UNAVAILABLE.into())
}

/// Same answer as `feedback_submit`.
#[tauri::command]
fn feedback_attachment_remove(_id: String) -> Result<(), String> {
    Err(FEEDBACK_UNAVAILABLE.into())
}

/// Same answer as `feedback_submit`.
#[tauri::command]
fn feedback_attachments_clear() -> Result<(), String> {
    Err(FEEDBACK_UNAVAILABLE.into())
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

/// 用 AI 预设处理 (docs/dictation.md §22), as on the desktop; the phone's page does not offer it.
#[tauri::command]
fn history_process(bridge: tauri::State<'_, Bridge>, request_id: u64, id: String, preset: String) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::HistoryProcess { request_id, id, preset })?)
}

/// Stop a `history_process`.
#[tauri::command]
fn history_process_cancel(bridge: tauri::State<'_, Bridge>, request_id: u64) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::HistoryProcessCancel { request_id })?)
}

/// Exports are a desktop feature (docs/dictation.md §22): the phone writes no files.
#[tauri::command]
fn history_export(id: String, format: String, file_name: String) -> Result<serde_json::Value, String> {
    let _ = (id, format, file_name);
    Ok(serde_json::json!({ "kind": "failed", "code": "write", "detail": "exports are not available on the phone" }))
}

/// UI language is shared state: the phone edits it like the desktop does.
#[tauri::command]
fn settings_set_locale(bridge: tauri::State<'_, Bridge>, locale: Locale) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::SettingsSetLocale { locale })?)
}

/// The toggle persists on the phone too (it is one settings file per device, so this only matters
/// for parity of the settings page); the phone itself never runs an updater — the store does.
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

/// Why the phone refuses the update verbs: app stores deliver phone updates, not the app itself.
pub const UPDATE_UNAVAILABLE: &str = "updater: 手机端由应用商店更新";

/// Phones carry no local speech models (docs/dictation.md §10): the library verbs refuse honestly
/// and `UiState.models` stays empty.
pub const MODELS_UNAVAILABLE: &str = "models: 手机端不支持本地模型";

#[tauri::command]
fn model_download(_id: String) -> Result<(), String> {
    Err(MODELS_UNAVAILABLE.to_owned())
}

#[tauri::command]
fn model_cancel(_id: String) -> Result<(), String> {
    Err(MODELS_UNAVAILABLE.to_owned())
}

#[tauri::command]
fn model_remove(_id: String) -> Result<(), String> {
    Err(MODELS_UNAVAILABLE.to_owned())
}

// The phone's own dictionary, rules and scenes (user decision 2026-10-01: the phone has every
// setting but the local models), handed to the core like the desktop's (docs/dictation.md §16, §18).
// A phone has no foreground probe: the user picks a take's scene (`settings_set_pinned_scene`).

#[tauri::command]
fn dictionary_add(bridge: tauri::State<'_, Bridge>, entry: DictionaryDraft, history_id: Option<String>) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::DictionaryAdd { entry, history_id })?)
}

#[tauri::command]
fn dictionary_update(bridge: tauri::State<'_, Bridge>, id: String, entry: DictionaryDraft) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::DictionaryUpdate { id, entry })?)
}

#[tauri::command]
fn dictionary_remove(bridge: tauri::State<'_, Bridge>, id: String) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::DictionaryRemove { id })?)
}

#[tauri::command]
fn dictionary_reorder(bridge: tauri::State<'_, Bridge>, ids: Vec<String>) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::DictionaryReorder { ids })?)
}

#[tauri::command]
fn rules_add(bridge: tauri::State<'_, Bridge>, rule: RuleDraft) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::RulesAdd { rule })?)
}

#[tauri::command]
fn rules_update(bridge: tauri::State<'_, Bridge>, id: String, rule: RuleDraft) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::RulesUpdate { id, rule })?)
}

#[tauri::command]
fn rules_remove(bridge: tauri::State<'_, Bridge>, id: String) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::RulesRemove { id })?)
}

#[tauri::command]
fn rules_reorder(bridge: tauri::State<'_, Bridge>, ids: Vec<String>) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::RulesReorder { ids })?)
}

/// Import rules from pasted TOML (`replace` | `merge`, docs/dictation.md §16.5).
#[tauri::command]
fn rules_import(bridge: tauri::State<'_, Bridge>, toml: String, mode: ImportMode) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::RulesImport { toml, mode })?)
}

/// Query: the rules as TOML text (the phone hands it to the share sheet).
#[tauri::command]
fn rules_export(bridge: tauri::State<'_, Bridge>) -> Result<String, String> {
    Ok(bridge.rules_export()?)
}

/// Query: `text` through the dictionary and the rules with the pipeline's own code.
#[tauri::command]
fn vocabulary_preview(bridge: tauri::State<'_, Bridge>, text: String, draft: Option<PreviewDraft>) -> Result<VocabularyPreview, String> {
    Ok(bridge.vocabulary_preview(&text, draft.as_ref())?)
}

#[tauri::command]
fn scenes_add(bridge: tauri::State<'_, Bridge>, scene: SceneDraft) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::ScenesAdd { scene })?)
}

#[tauri::command]
fn scenes_update(bridge: tauri::State<'_, Bridge>, id: String, scene: SceneDraft) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::ScenesUpdate { id, scene })?)
}

#[tauri::command]
fn scenes_remove(bridge: tauri::State<'_, Bridge>, id: String) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::ScenesRemove { id })?)
}

#[tauri::command]
fn scenes_reorder(bridge: tauri::State<'_, Bridge>, ids: Vec<String>) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::ScenesReorder { ids })?)
}

/// 恢复默认 on a built-in scene (docs/dictation.md §18.10).
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

/// What of a take's context may go to the LLM (§18.5); the phone names no application, but the
/// setting is shared state like the desktop's.
#[tauri::command]
fn settings_set_context_sharing(bridge: tauri::State<'_, Bridge>, app_name: bool, window_title: bool) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::SettingsSetContextSharing { app_name, window_title })?)
}

/// The scene the phone's takes run with (the talk card's choice); `None` = no scene.
#[tauri::command]
fn settings_set_pinned_scene(bridge: tauri::State<'_, Bridge>, id: Option<String>) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::SettingsSetPinnedScene { id })?)
}

/// Query: the applications the history saw, newest first: none on a phone, which names no
/// application (the scene editor's picker on the desktop, §18.6).
#[tauri::command]
async fn recent_apps(bridge: tauri::State<'_, Bridge>) -> Result<Vec<AppRef>, String> {
    history_read(&bridge, Bridge::recent_apps).await
}

/// A history read on a blocking thread (it opens the database).
async fn history_read<T: Send + 'static>(
    bridge: &Bridge,
    read: impl FnOnce(&Bridge) -> Result<T, voltip_tauri_bridge::BridgeError> + Send + 'static,
) -> Result<T, String> {
    let bridge = bridge.clone();
    Ok(tauri::async_runtime::spawn_blocking(move || read(&bridge)).await.map_err(|e| e.to_string())??)
}

/// The phone's own history (docs/dictation.md §20.7): the takes it recognised itself — a take
/// streamed to a desktop is that desktop's. Read as on the desktop (§4.4): filtered, searched and
/// paged in the database.
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

/// The dictations between each two of the local midnights in `boundaries`, and over the whole
/// history (docs/dictation.md §4.5).
#[tauri::command]
async fn history_stats(bridge: tauri::State<'_, Bridge>, boundaries: Vec<u64>) -> Result<voltip_core::HistoryStats, String> {
    history_read(&bridge, move |b| b.history_stats(&boundaries)).await
}

/// How often each dictionary entry and rule fired in the history: none on the phone, which has
/// no vocabulary (docs/dictation.md §16.4); the same read as the desktop's.
#[tauri::command]
async fn history_hits(bridge: tauri::State<'_, Bridge>) -> Result<voltip_core::HistoryHits, String> {
    history_read(&bridge, Bridge::history_hits).await
}

/// The phone's own AI presets (docs/dictation.md §21, §20.7): the phone cleans up what it
/// recognises itself, so its presets are edited here like the desktop's.
#[tauri::command]
fn presets_add(bridge: tauri::State<'_, Bridge>, preset: PresetDraft) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::PresetsAdd { preset })?)
}

#[tauri::command]
fn presets_update(bridge: tauri::State<'_, Bridge>, id: String, preset: PresetDraft) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::PresetsUpdate { id, preset })?)
}

#[tauri::command]
fn presets_remove(bridge: tauri::State<'_, Bridge>, id: String) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::PresetsRemove { id })?)
}

/// 试一试: the answer arrives as a `preset_try` event carrying `id`.
#[tauri::command]
fn presets_try(bridge: tauri::State<'_, Bridge>, id: u64, preset: Option<String>, prompt: Option<String>, text: String) -> Result<(), String> {
    Ok(bridge.dispatch(UiCommand::PresetsTry { id, preset, prompt, text })?)
}

/// Query: every built-in preset's text, what 复制为自定义 starts from.
#[tauri::command]
fn presets_builtin() -> Vec<voltip_cloud::BuiltinPresetText> {
    voltip_cloud::builtin_preset_texts()
}

#[tauri::command]
fn update_check() -> Result<(), String> {
    Err(UPDATE_UNAVAILABLE.to_owned())
}

#[tauri::command]
fn update_install() -> Result<(), String> {
    Err(UPDATE_UNAVAILABLE.to_owned())
}

/// No updater on the phone: always `disabled`, so the shared settings page hides the section.
#[tauri::command]
fn update_status() -> UpdateStatus {
    UpdateStatus::Disabled
}

/// The phone asks for its permissions through the Android runtime-permission flow, not through
/// this query (docs/dictation.md §15.1): everything `not_applicable`, so the shared onboarding
/// code sees "nothing to grant here".
#[tauri::command]
fn permissions_status() -> voltip_platform::PermissionReport {
    voltip_platform::PermissionReport::not_applicable(voltip_platform::HostOs::current())
}

/// Accepted no-op (see `permissions_status`).
#[tauri::command]
fn permissions_request(permission: voltip_platform::Permission) -> Result<(), String> {
    tracing::debug!(permission = permission.as_str(), "permissions_request ignored on mobile");
    Ok(())
}

/// The phone injects nothing: `proceed`, unchecked.
#[tauri::command]
fn inject_preflight() -> voltip_platform::InjectPreflight {
    voltip_platform::InjectPreflight::not_applicable(voltip_platform::HostOs::current())
}

/// The history's paste button (`voltip_core::paste`): the phone has no window to paste into, so
/// the text goes onto its clipboard (`copied { clipboard_only }`, as with `EngineSettings.inject =
/// clipboard_only` on the desktop); `failed { inject }` when the clipboard refuses it.
#[tauri::command]
async fn paste_text<R: Runtime>(app: AppHandle<R>, text: String) -> Result<voltip_core::paste::PasteOutcome, String> {
    use voltip_core::paste::{CopyReason, PasteFailure, PasteOutcome};
    if !voltip_core::paste::valid_paste_text(&text) {
        return Ok(PasteOutcome::Failed { reason: PasteFailure::Invalid });
    }
    let written = tauri::async_runtime::spawn_blocking(move || clipboard::write_text(&app, &text)).await.map_err(|e| e.to_string())?;
    Ok(match written {
        Ok(()) => PasteOutcome::Copied { reason: CopyReason::ClipboardOnly },
        Err(e) => {
            tracing::warn!(error = %e, "copy to the phone clipboard failed");
            PasteOutcome::Failed { reason: PasteFailure::Inject }
        }
    })
}

/// 「分享」 (docs/dictation.md §20.7): `text` through the system share sheet (`SharePlugin.kt`).
#[tauri::command]
async fn phone_share_text<R: Runtime>(app: AppHandle<R>, text: String) -> Result<(), String> {
    share::share_text(&app, text).await
}

/// The phone's dictation ports: its microphone (the takes it streams, docs/dictation.md §20, and
/// the ones it recognises itself, §20.7), the cloud clients of the resolved engines — the built-in
/// services unless the settings name others; the phone has no local models — and its clipboard
/// for the result, and the HTTP provider probe (测试连接). No live preview, no foreground probe,
/// no VAD: the core's fallbacks apply.
pub fn phone_ports<R: Runtime>(app: &AppHandle<R>) -> voltip_core::dictation::DictationPorts {
    voltip_core::dictation::DictationPorts {
        audio: Arc::new(microphone::PhoneMicrophone::cpal()),
        injector: Arc::new(clipboard::PhoneClipboardInjector(app.clone())),
        factory: Arc::new(|engines: &voltip_core::ResolvedEngines| (voltip_cloud::remote_transcriber(engines), voltip_cloud::refiner(engines))),
        models: None,
        streaming: None,
        probe: None,
        service_probe: Some(Arc::new(voltip_cloud::HttpServiceProbe)),
        segmenter: None,
    }
}

/// Start the core, forward every [`voltip_core::ui::UiEvent`] onto the webview event bus and
/// manage the [`Bridge`] as Tauri state so the commands above can reach it.
pub fn attach_bridge<R: Runtime>(
    app: &AppHandle<R>,
    config: CoreConfig,
    store: Arc<dyn SecretStore>,
    ports: voltip_core::dictation::DictationPorts,
) -> Result<(), BridgeError> {
    // Tauri runs `setup` on the UI thread, outside any Tokio context; the core spawns its tasks
    // with `tokio::spawn`, so it has to be started from Tauri's own runtime.
    // Subscribed before the core starts so the first `state` event reaches the webview bus.
    let (bridge, mut events) = tauri::async_runtime::block_on(async { Bridge::start_subscribed(config, store, ports) })?;
    // No updater on the phone: `core_state().update` says so from the first frame.
    bridge.publish(voltip_core::ui::UiEvent::Update(UpdateStatus::Disabled));
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            match events.recv().await {
                Ok(ev) => {
                    if let Err(e) = handle.emit(UI_EVENT_NAME, &ev) {
                        tracing::warn!(error = %e, "emit failed");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => tracing::warn!(skipped = n, "webview lagged"),
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    app.manage(bridge);
    Ok(())
}

/// Register the plugins, the command handlers and the setup hook that attaches the bridge.
/// `config` and `ports` run inside `setup`: the platform data dir and the clipboard injector need
/// a live [`AppHandle`]. [`run`] feeds it `tauri::Builder::default()` and [`phone_ports`]; tests
/// feed it `tauri::test::mock_builder()` and the in-memory fakes.
pub fn build_app<R: Runtime>(
    builder: tauri::Builder<R>,
    config: impl FnOnce(&AppHandle<R>) -> CoreConfig + Send + 'static,
    store: Arc<dyn SecretStore>,
    ports: impl FnOnce(&AppHandle<R>) -> voltip_core::dictation::DictationPorts + Send + 'static,
) -> tauri::Builder<R> {
    #[cfg(mobile)]
    let builder = builder.plugin(tauri_plugin_barcode_scanner::init()).plugin(tauri_plugin_opener::init());
    builder
        .plugin(microphone::init())
        .plugin(clipboard::init())
        .plugin(share::init())
        .plugin(multicast::init())
        .manage(meter::Meters::default())
        .setup(move |app| {
            let config = config(app.handle());
            let ports = ports(app.handle());
            Ok(attach_bridge(app.handle(), config, store, ports).map_err(|e| std::io::Error::other(e.to_string()))?)
        })
        .invoke_handler(tauri::generate_handler![
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
            presets_add,
            presets_update,
            presets_remove,
            presets_try,
            presets_builtin,
            scenes_add,
            scenes_update,
            scenes_remove,
            scenes_reorder,
            scenes_restore,
            scenes_builtin,
            settings_set_context_sharing,
            settings_set_pinned_scene,
            recent_apps,
            history_query,
            history_entry,
            history_stats,
            history_hits,
            permissions_status,
            permissions_request,
            inject_preflight,
            paste_text,
            phone_share_text
        ])
}

/// Build and run with the platform data dir and secret store.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,voltip=debug"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();

    let outcome = build_app(tauri::Builder::default(), production_config, secret_store(), phone_ports).run(tauri::generate_context!());
    if let Err(e) = outcome {
        tracing::error!(error = %e, "tauri exited with error");
        std::process::exit(1);
    }
}

/// Human-readable platform for the default device name.
pub fn platform_label() -> &'static str {
    if cfg!(target_os = "android") {
        "Android"
    } else if cfg!(target_os = "ios") {
        "iOS"
    } else {
        "Voltip"
    }
}
