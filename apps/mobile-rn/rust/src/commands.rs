//! The phone's command surface (docs/mobile-rn.md §4): the same names and arguments as the Tauri
//! phone shell (`apps/mobile/src-tauri/src/lib.rs`), so `@voltip/shared`'s `TauriBackend` drives it
//! unchanged. A command that is a `UiCommand` takes the generic path: `{ "command": name, ...args }`
//! deserializes into the variant (the wire form `crates/voltip-tauri-bridge/tests/contract.rs`
//! checks for every variant). The queries and the phone's own commands are answered here, each as
//! the Tauri shell answers it.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use voltip_cloud::feedback;
use voltip_core::ui::{GuidePage, ProjectLink};
use voltip_core::{HistoryQuery, ProviderId};
use voltip_tauri_bridge::{Bridge, BridgeError, UiCommand};

use crate::shell::Shell;

/// Every command the app may invoke: the Tauri phone shell's `COMMANDS`, in its order (a test reads
/// that list and compares).
pub const COMMANDS: [&str; 110] = [
    "core_state",
    "pairing_start",
    "pairing_join_code",
    "pairing_join_ticket",
    "pairing_confirm",
    "pairing_reject",
    "pairing_cancel",
    "pairing_reset",
    "device_forget",
    "device_sync_set",
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
    "engines_quota_reset",
    "provider_probe",
    "provider_console_open",
    "project_link_open",
    "guide_open",
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
    "model_import",
    "model_folder_open",
    "model_link_open",
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
    "refine_notice_close",
    "recent_apps",
    "history_query",
    "history_entry",
    "history_stats",
    "history_hits",
    "mirror_history_query",
    "mirror_history_entry",
    "mirror_profile",
    "permissions_status",
    "permissions_request",
    "inject_preflight",
    "paste_text",
    "phone_share_text",
];

/// Why the phone refuses a key edge: it has no hotkey (and no voice edit); its takes start from the
/// button. The Tauri shell's text.
pub const HOTKEY_UNAVAILABLE: &str = "hotkey: 手机端没有快捷键";
/// Phones carry no local speech models (docs/dictation.md §10). The Tauri shell's text.
pub const MODELS_UNAVAILABLE: &str = "models: 手机端不支持本地模型";
/// This build has no update source (docs/mobile-rn.md §1): its status stays `disabled`.
pub const UPDATES_UNAVAILABLE: &str = "updater: 这个版本没有更新渠道";
/// The repository this build comes from (`Cargo.toml` `repository`): what 关于 opens.
pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
/// The argument `audio_meter_start`'s `onFrame` arrives as: the stream's id on the app side.
pub const CHANNEL_KEY: &str = "__voltipChannel";

type Args = Map<String, Value>;

/// Run `command`; the answer as JSON, or the error text.
pub async fn run(shell: &Shell, command: &str, args: Value) -> Result<Value, String> {
    if !COMMANDS.contains(&command) {
        return Err(format!("unknown command {command}"));
    }
    let args = match args {
        Value::Null => Args::new(),
        Value::Object(map) => map,
        other => return Err(format!("invalid args for {command}: expected an object, got {other}")),
    };
    let inner = &shell.inner;
    let bridge = &inner.bridge;
    match command {
        "core_state" => to_json(&bridge.state()),
        "phone_clipboard_read" => {
            let text = blocking(shell, |host| host.clipboard_read()).await?;
            Ok(json!({ "text": text.filter(|t| !t.is_empty()) }))
        }
        "settings_set_lan_discovery" => {
            let enabled: bool = arg(command, &args, "enabled")?;
            bridge.dispatch(UiCommand::SettingsSetLanDiscovery { enabled }).map_err(String::from)?;
            shell.hold_multicast(enabled);
            Ok(Value::Null)
        }
        // Phones register no OS hotkey; the recorder's suspend request is accepted and ignored so
        // the shared code needs no platform branch.
        "hotkey_capture" => Ok(Value::Null),
        // Nothing to pick: a take records from the system's default input, which AAudio routes.
        "audio_devices" => Ok(json!([])),
        // The phone records its own microphone; the computer's sound is a desktop source.
        "audio_outputs" => Ok(json!({ "system_audio": { "state": "unsupported" }, "devices": [] })),
        "audio_meter_start" => {
            let channel = args.get("onFrame").and_then(|v| v.get(CHANNEL_KEY)).and_then(Value::as_u64);
            let channel = channel.ok_or_else(|| format!("invalid args for {command}: onFrame must be a channel"))?;
            let host = inner.host.clone();
            let id = inner.meters.start(&inner.runtime, bridge.levels(), move |frame| match serde_json::to_string(&frame) {
                Ok(json) => {
                    host.channel(channel, &json);
                    true
                }
                Err(_) => false,
            });
            Ok(json!(id))
        }
        "audio_meter_stop" => {
            inner.meters.stop(arg(command, &args, "id")?);
            Ok(Value::Null)
        }
        // The phone has no pill window.
        "overlay_state" => Ok(json!("blank")),
        "hotkey_edge" => Err(HOTKEY_UNAVAILABLE.to_owned()),
        "provider_console_open" => {
            let provider: ProviderId = arg(command, &args, "provider")?;
            let url = provider.spec().console_url.ok_or_else(|| format!("{}: no key page", provider.as_str()))?;
            open(shell, url.to_owned()).await
        }
        "project_link_open" => {
            let link: ProjectLink = arg(command, &args, "link")?;
            open(shell, link.url(REPOSITORY)).await
        }
        "guide_open" => {
            let page: GuidePage = arg(command, &args, "page")?;
            let locale: String = arg(command, &args, "locale")?;
            open(shell, page.url(&locale)).await
        }
        "feedback_diagnostics" => {
            let locale: String = arg(command, &args, "locale")?;
            let state = bridge.state();
            to_json(&feedback::FeedbackInfo {
                configured: feedback::feedback_url().is_some(),
                diagnostics: feedback::diagnostics(&state, &locale, None, &state.app_version),
            })
        }
        "feedback_submit" => feedback_submit(shell, command, &args).await,
        "feedback_attachment_add" => {
            // The bytes arrive as base64 (the app's `invokeRaw`): the JSON bridge carries text.
            let data: String = arg(command, &args, "data")?;
            let name: String = arg(command, &args, "name")?;
            let mime: String = arg(command, &args, "type")?;
            let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data.as_bytes())
                .map_err(|_| feedback::AttachError::Type.as_str().to_owned())?;
            to_json(&inner.attachments.add(&name, &mime, bytes).map_err(|e| e.as_str().to_owned())?)
        }
        "feedback_attachment_remove" => {
            let id: String = arg(command, &args, "id")?;
            inner.attachments.remove(&id);
            Ok(Value::Null)
        }
        "feedback_attachments_clear" => {
            inner.attachments.clear();
            Ok(Value::Null)
        }
        "history_export" => history_export(shell, command, &args).await,
        "update_check" | "update_install" => Err(UPDATES_UNAVAILABLE.to_owned()),
        "update_status" => to_json(&bridge.state().update),
        "model_download" | "model_cancel" | "model_remove" | "model_import" | "model_folder_open" | "model_link_open" => Err(MODELS_UNAVAILABLE.to_owned()),
        "rules_export" => to_json(&bridge.rules_export().map_err(String::from)?),
        "vocabulary_preview" => {
            let text: String = arg(command, &args, "text")?;
            let draft: Option<voltip_core::PreviewDraft> = arg(command, &args, "draft")?;
            to_json(&bridge.vocabulary_preview(&text, draft.as_ref()).map_err(String::from)?)
        }
        "scenes_builtin" => {
            #[derive(Serialize)]
            struct BuiltinSceneTerms {
                id: voltip_core::BuiltinScene,
                terms: &'static [&'static str],
            }
            let packs: Vec<_> =
                voltip_core::BuiltinScene::ALL.into_iter().map(|id| BuiltinSceneTerms { id, terms: voltip_core::vocabulary::packs::terms(id) }).collect();
            to_json(&packs)
        }
        "presets_builtin" => to_json(&voltip_cloud::builtin_preset_texts()),
        "recent_apps" => read(shell, Bridge::recent_apps).await,
        "history_query" => {
            let query = history_query(command, &args)?;
            read(shell, move |b| b.history_query(&query)).await
        }
        "history_entry" => {
            let id: uuid::Uuid = arg(command, &args, "id")?;
            read(shell, move |b| b.history_entry(id)).await
        }
        "history_stats" => {
            let boundaries: Vec<u64> = arg(command, &args, "boundaries")?;
            read(shell, move |b| b.history_stats(&boundaries)).await
        }
        "history_hits" => read(shell, Bridge::history_hits).await,
        "mirror_history_query" => {
            let desktop: String = arg(command, &args, "desktop")?;
            let query = history_query(command, &args)?;
            read(shell, move |b| b.mirror_history_query(&desktop, &query)).await
        }
        "mirror_history_entry" => {
            let desktop: String = arg(command, &args, "desktop")?;
            let id: uuid::Uuid = arg(command, &args, "id")?;
            read(shell, move |b| b.mirror_history_entry(&desktop, id)).await
        }
        "mirror_profile" => {
            let desktop: String = arg(command, &args, "desktop")?;
            read(shell, move |b| b.mirror_profile(&desktop)).await
        }
        // The phone asks for its permissions through Android's runtime-permission flow
        // (docs/dictation.md §15.1): everything `not_applicable`, and a request is a no-op.
        "permissions_status" => to_json(&voltip_platform::PermissionReport::not_applicable(voltip_platform::HostOs::current())),
        "permissions_request" => {
            let _: voltip_platform::Permission = arg(command, &args, "permission")?;
            Ok(Value::Null)
        }
        // The phone injects nothing: `proceed`, unchecked.
        "inject_preflight" => to_json(&voltip_platform::InjectPreflight::not_applicable(voltip_platform::HostOs::current())),
        "paste_text" => paste_text(shell, command, &args).await,
        "phone_share_text" => {
            let text: String = arg(command, &args, "text")?;
            if !voltip_core::paste::valid_paste_text(&text) {
                return Err(format!("share: 文字为空或超过 {} 字", voltip_core::paste::MAX_PASTE_TEXT_CHARS));
            }
            blocking(shell, move |host| host.share_text(&text)).await?;
            Ok(Value::Null)
        }
        // Everything else is a `UiCommand`; `phone_take_start` and `dictation_start` arrive once
        // the app holds the microphone permission (`src/backend/transport.ts`).
        _ => {
            bridge.dispatch(to_command(command, args)?).map_err(String::from)?;
            Ok(Value::Null)
        }
    }
}

/// `invoke(name, args)` → the tagged form `UiCommand` deserializes, as a `#[tauri::command]` binds
/// its parameters.
pub fn to_command(command: &str, mut args: Args) -> Result<UiCommand, String> {
    args.insert("command".into(), Value::String(command.to_owned()));
    serde_json::from_value(Value::Object(args)).map_err(|e| format!("invalid args for {command}: {e}"))
}

/// Argument `key`; a missing one reads as `null` (an `Option` is then `None`).
fn arg<T: DeserializeOwned>(command: &str, args: &Args, key: &str) -> Result<T, String> {
    serde_json::from_value(args.get(key).cloned().unwrap_or(Value::Null)).map_err(|e| format!("invalid args `{key}` for command `{command}`: {e}"))
}

fn to_json<T: Serialize>(value: &T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

/// The history query's arguments, as `history_query` and `mirror_history_query` take them.
fn history_query(command: &str, args: &Args) -> Result<HistoryQuery, String> {
    Ok(HistoryQuery {
        since_ms: arg(command, args, "sinceMs")?,
        starred: arg::<Option<bool>>(command, args, "starred")?.unwrap_or(false),
        failed: arg::<Option<bool>>(command, args, "failed")?.unwrap_or(false),
        query: arg::<Option<String>>(command, args, "query")?.unwrap_or_default(),
        offset: arg::<Option<u32>>(command, args, "offset")?.unwrap_or(0),
        limit: arg(command, args, "limit")?,
    })
}

/// A history read on a blocking thread (it opens the database).
async fn read<T: Serialize + Send + 'static>(shell: &Shell, read: impl FnOnce(&Bridge) -> Result<T, BridgeError> + Send + 'static) -> Result<Value, String> {
    let bridge = shell.inner.bridge.clone();
    let value = shell.inner.runtime.spawn_blocking(move || read(&bridge)).await.map_err(|e| e.to_string())?.map_err(String::from)?;
    to_json(&value)
}

/// A host call on a blocking thread (it waits for Android's main thread).
async fn blocking<T: Send + 'static>(shell: &Shell, call: impl FnOnce(&dyn crate::host::Host) -> Result<T, String> + Send + 'static) -> Result<T, String> {
    let host = shell.inner.host.clone();
    shell.inner.runtime.spawn_blocking(move || call(host.as_ref())).await.map_err(|e| e.to_string())?
}

/// Open `url` in the phone's browser. The app names a provider, a page or a link, never a URL.
async fn open(shell: &Shell, url: String) -> Result<Value, String> {
    blocking(shell, move |host| host.open_url(&url)).await?;
    Ok(Value::Null)
}

/// Post the 反馈 page's report with the diagnostics it showed and the attachments it staged, as the
/// Tauri shell does: the error is the reason's wire name, never the endpoint, and the staged files
/// are forgotten once the report went out.
async fn feedback_submit(shell: &Shell, command: &str, args: &Args) -> Result<Value, String> {
    let kind: feedback::FeedbackKind = arg(command, args, "kind")?;
    let message: String = arg(command, args, "message")?;
    let contact: Option<String> = arg(command, args, "contact")?;
    let locale: String = arg(command, args, "locale")?;
    let ids: Vec<String> = arg::<Option<Vec<String>>>(command, args, "attachments")?.unwrap_or_default();
    let url = feedback::feedback_url().ok_or_else(|| feedback::SendError::NotConfigured.as_str().to_owned())?;
    let (message, contact) = feedback::check(&message, contact.as_deref()).map_err(|e| e.as_str().to_owned())?;
    let staged = &shell.inner.attachments;
    let files = staged.pick(&ids).map_err(|e| e.as_str().to_owned())?;
    let state = shell.inner.bridge.state();
    let diagnostics = feedback::diagnostics(&state, &locale, None, &state.app_version);
    let agent = format!("voltip-mobile/{}", state.app_version);
    let sent = feedback::send(url, feedback::feedback_token(), &agent, kind, &message, contact.as_deref(), &diagnostics, &files).await;
    if matches!(sent, Ok(_) | Err(feedback::SendError::Attachments)) {
        staged.forget(&ids);
    }
    to_json(&sent.map_err(|e| e.as_str().to_owned())?)
}

/// 分享字幕 / 分享文本 (docs/dictation.md §20.7, §22): the phone has no file dialog, so an export goes
/// to the share sheet as a file, named as the desktop's save dialog would offer it. Answers like
/// the Tauri shell: `shared`, or `failed` with the reason.
async fn history_export(shell: &Shell, command: &str, args: &Args) -> Result<Value, String> {
    use voltip_core::history::export;
    let id: uuid::Uuid = arg(command, args, "id")?;
    let format: export::ExportFormat = arg(command, args, "format")?;
    let file_name: String = arg(command, args, "fileName")?;
    let bridge = shell.inner.bridge.clone();
    let entry = shell.inner.runtime.spawn_blocking(move || bridge.history_entry(id)).await.map_err(|e| e.to_string())?.map_err(String::from)?;
    let failed = |code: &str, detail: &str| json!({ "kind": "failed", "code": code, "detail": detail });
    let Some(entry) = entry else { return Ok(failed("gone", "the entry is not in the history")) };
    let Some(content) = export::render(&entry, format) else { return Ok(failed("empty", "the entry has no segments")) };
    let mime = match format {
        export::ExportFormat::Srt => "application/x-subrip",
        export::ExportFormat::Txt => "text/plain",
    };
    let name = export::file_name(&file_name, format);
    Ok(match blocking(shell, move |host| host.share_file(&name, &content, mime)).await {
        Ok(()) => json!({ "kind": "shared" }),
        Err(e) => {
            tracing::warn!(error = %e, "sharing an export failed");
            failed("share", &e)
        }
    })
}

/// The history's paste button (`voltip_core::paste`): the phone has no window to paste into, so the
/// text goes onto its clipboard (`copied { clipboard_only }`); `failed { inject }` when the
/// clipboard refuses it.
async fn paste_text(shell: &Shell, command: &str, args: &Args) -> Result<Value, String> {
    use voltip_core::paste::{CopyReason, PasteFailure, PasteOutcome};
    let text: String = arg(command, args, "text")?;
    if !voltip_core::paste::valid_paste_text(&text) {
        return to_json(&PasteOutcome::Failed { reason: PasteFailure::Invalid });
    }
    to_json(&match blocking(shell, move |host| host.clipboard_write(&text)).await {
        Ok(()) => PasteOutcome::Copied { reason: CopyReason::ClipboardOnly },
        Err(e) => {
            tracing::warn!(error = %e, "copy to the phone clipboard failed");
            PasteOutcome::Failed { reason: PasteFailure::Inject }
        }
    })
}
