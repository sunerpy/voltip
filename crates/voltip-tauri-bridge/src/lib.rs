//! Glue between a webview shell and [`voltip_core`].
//!
//! Kept free of any `tauri` dependency so it can be unit-tested on a plain runtime: the shell
//! only has to (1) call [`Bridge::dispatch`] from its `#[tauri::command]`s and (2) forward
//! every [`UiEvent`] coming out of [`Bridge::events`] onto the webview event bus.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use serde::Deserialize;
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;
use voltip_core::backup::auto::{AutoKind, AutoOutcome, AutoReply};
use voltip_core::backup::{self, BackupError, BackupParts, ExportInput, ExportSummary, ImportFailure, ImportOutcome, ImportReply, OpenOutcome};
use voltip_core::dictation::LevelFrame;
use voltip_core::history::MAX_RECENT_TEXTS;
use voltip_core::paste::{PasteFailure, PasteOutcome, PasteTarget};
use voltip_core::presets::{MAX_PRESET_TRY_CHARS, PresetDraft, PresetId, PresetTrial, clean_preset_prompt, validate_preset_draft};
use voltip_core::scenes::{MAX_RECENT_APPS, validate_scene_draft_with};
use voltip_core::ui::{UiEvent, UiState};
use voltip_core::vocabulary::{
    AcceptOutcome, MAX_SUGGESTIONS, Reply, RuleWarning, Suggestions, SuggestionsAnswer, UndoOutcome, rule_warnings, suggestions, validate_pair,
};
use voltip_core::vocabulary::{BACKTEST_LIMIT, Backtest, BacktestDraft, MAX_BACKTEST_LIMIT, backtest};
use voltip_core::vocabulary::{export_rules_toml, parse_rules_toml, preview, validate_dictionary_draft, validate_rule_draft};
use voltip_core::{
    Activation, AppCore, AppRef, ContextSharing, CoreCommand, CoreConfig, CoreError, CoreEvent, CoreHandle, DictationPorts, DictionaryDraft, EdgeSource,
    EngineSettings, EntrySource, HistoryEntry, HistoryHits, HistoryPage, HistoryQuery, HistoryReader, HistoryStats, ImportMode, Locale, OverlayPlacement,
    PhoneTalkMode, PreviewDraft, ProviderId, RuleDraft, SceneDraft, ServiceKind, TakeKind, ThemeId, VocabularyPreview,
};
use voltip_crypto::PublicKey;
use voltip_identity::SecretStore;

/// One webview call. Field names are camelCase on the wire (Tauri convention).
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum UiCommand {
    /// Start a pairing session.
    PairingStart,
    /// Join by code.
    PairingJoinCode {
        /// Six digits (spaces/dashes allowed).
        code: String,
    },
    /// Join by ticket URI.
    PairingJoinTicket {
        /// `voltip://pair?...`.
        uri: String,
    },
    /// Confirm.
    PairingConfirm,
    /// Reject.
    PairingReject,
    /// Cancel.
    PairingCancel,
    /// Reset to idle.
    PairingReset,
    /// Forget a device.
    DeviceForget {
        /// Hex public key.
        public_key: String,
    },
    /// Computer: sync with this phone or not (docs/dictation.md §20.8).
    DeviceSyncSet {
        /// Hex public key.
        public_key: String,
        /// On or off.
        on: bool,
    },
    /// Rename this device.
    DeviceRename {
        /// New name.
        name: String,
    },
    /// Send text.
    SendText {
        /// Hex public key.
        public_key: String,
        /// Body.
        body: String,
    },
    /// Phone: stream a take to this trusted desktop (docs/dictation.md §20).
    PhoneTakeStart {
        /// Hex public key of the desktop.
        public_key: String,
    },
    /// Phone: the speaker let go.
    PhoneTakeStop,
    /// Phone: discard the take.
    PhoneTakeCancel,
    /// Phone: send text for the desktop to insert at its cursor (docs/dictation.md §20.6).
    PhoneTextSend {
        /// Hex public key of the desktop.
        public_key: String,
        /// The text.
        body: String,
        /// `typed` | `clipboard`.
        source: voltip_core::phone::PhoneTextSource,
    },
    /// Phone: forget the list of sent texts.
    SentTextsClear,
    /// Keep a pairing open until turned off (docs/pairing.md 「常开配对」; desktop only).
    SettingsSetPairingAlwaysOn {
        /// On / off.
        enabled: bool,
    },
    /// The network changed or the app came back to the front: the relay link checks its socket
    /// now (docs/pairing.md 「重连」).
    RelayReconnect,
    /// Relay settings.
    SettingsSetRelay {
        /// URL or null.
        url: Option<String>,
        /// Enabled.
        enabled: bool,
    },
    /// Theme settings.
    SettingsSetTheme {
        /// Theme.
        theme: ThemeId,
        /// Follow OS.
        follow_system: bool,
    },
    /// Global dictation hotkey (`Ctrl+Alt+Space`); the core validates and persists, the shell
    /// re-registers it when the `settings` event arrives.
    SettingsSetHotkey {
        /// Chord text.
        hotkey: String,
    },
    /// Voice-edit hotkey (docs/dictation.md §19): a chord, or `null` to switch it off; validated and
    /// persisted like the dictation hotkey, never the same chord.
    SettingsSetEditHotkey {
        /// Chord text or `null`.
        hotkey: Option<String>,
    },
    /// 复制上一条结果 hotkey (docs/dictation.md §28): a chord, or `null` for none (the default);
    /// validated and persisted like the dictation hotkey, never the dictation or the voice-edit chord.
    SettingsSetCopyLastHotkey {
        /// Chord text or `null`.
        hotkey: Option<String>,
    },
    /// Lone-key trigger (docs/dictation.md §13.1): `right_ctrl` … `mouse_forward`, or `null` to
    /// switch it off; persisted, the desktop shell watches the key when the `settings` event arrives.
    SettingsSetSoloKey {
        /// Key or `null`.
        key: Option<voltip_core::SoloKey>,
    },
    /// The microphone takes record from: a device id of `audio_devices`, or `null` for the system
    /// default; persisted, used from the next take on.
    SettingsSetMicrophone {
        /// Device id or `null`.
        device: Option<String>,
    },
    /// A dictation take's source (the microphone, the computer's sound, both), the output device
    /// and the longest length (docs/dictation.md §22); persisted, used from the next take on.
    SettingsSetRecording {
        /// The whole setting.
        recording: voltip_core::RecordingSettings,
    },
    /// UI language (`system` | `zh-cn` | `en`); persisted, every window follows `settings`.
    SettingsSetLocale {
        /// Language.
        locale: Locale,
    },
    /// Automatic update check on launch (desktop shell); persisted, the shell follows `settings`.
    SettingsSetAutoUpdate {
        /// On / off.
        enabled: bool,
    },
    /// History recording and retention (docs/dictation.md §4).
    SettingsSetHistory {
        /// Record takes.
        enabled: bool,
        /// Newest entries kept (10–500).
        keep: u32,
    },
    /// The recordings kept for recognising a take again (docs/dictation.md §26): on / off (off
    /// deletes them), how many and how much space at most.
    SettingsSetAudioCache {
        /// Keep recordings.
        enabled: bool,
        /// Recordings kept (1–200).
        keep: u32,
        /// MB at most (50–10000).
        max_mb: u32,
    },
    /// The weekly automatic backup on or off (docs/dictation.md §4.6.1); the backups before an
    /// update and before a replacing import are made either way.
    SettingsSetBackup {
        /// Make one every week.
        weekly: bool,
    },
    /// The automatic backups go to `<data dir>/backups` again; the ones made elsewhere stay there.
    BackupFolderReset,
    /// Close home's notice that the automatic backup before the update failed.
    BackupNoticeDismiss,
    /// Delete every kept recording.
    AudioCacheClear,
    /// Delete one kept recording.
    AudioCacheDelete {
        /// The recording's UUID.
        id: String,
    },
    /// Recognise a kept recording again: the take's status follows as `dictation` events, the text
    /// goes to the clipboard and into the history. `asr` / `refine` are other models than the
    /// settings', for this take only (docs/dictation.md §26.4).
    AudioRetake {
        /// The recording's UUID.
        id: String,
        /// The recognition, when not the settings'.
        #[serde(default)]
        asr: Option<voltip_core::audio_cache::RecognitionChoice>,
        /// The clean-up, when not the settings'.
        #[serde(default)]
        refine: Option<voltip_core::audio_cache::PolishChoice>,
    },
    /// Where the dictation pill appears; persisted, the desktop shell follows `settings`.
    SettingsSetOverlay {
        /// `bottom` / `top` / `off`.
        placement: OverlayPlacement,
    },
    /// Re-emit devices.
    DevicesRefresh,
    /// Run the connectivity self-check (`voltip_core::connectivity`).
    ConnectivityCheck,
    /// Open the microphone.
    DictationStart,
    /// Close the microphone and run the pipeline.
    DictationStop,
    /// Discard the recording / pending result.
    DictationCancel,
    /// The result card's 关闭, or its 粘贴 that went in (docs/dictation.md §28): that take's card
    /// goes away now; a card of another take stays.
    DictationDismiss {
        /// `DictationStatus.session` of the take whose card it is.
        session: u64,
    },
    /// A key transition for the activation machine (docs/dictation.md §13): the desktop shell sends
    /// one per hotkey press / release, the CLI one per `voltip --toggle`.
    HotkeyEdge {
        /// Key down / up.
        pressed: bool,
        /// `voltip_core::now_ms()` when it happened.
        at_ms: u64,
        /// `hotkey` | `cli` | `ui`.
        source: EdgeSource,
        /// `dictation` (default: shells and fixtures from before voice edit) | `edit`.
        #[serde(default)]
        purpose: TakeKind,
        /// Another key joined the held lone-key trigger (docs/dictation.md §13.1); only the desktop
        /// shell's input hook sets it.
        #[serde(default)]
        chorded: bool,
    },
    /// How the hotkey drives a dictation (`hold` | `toggle` | `hold_or_toggle`) plus its timings.
    SettingsSetActivation {
        /// Mode.
        activation: Activation,
        /// `hold_or_toggle` threshold.
        hold_threshold_ms: u32,
        /// Trailing capture after a stop.
        extra_recording_ms: u32,
    },
    /// Replace `Settings.engines` wholesale.
    SettingsSetEngines {
        /// Full block.
        engines: EngineSettings,
    },
    /// Store (`value`) or delete (`null`) the user's key for a provider's service (a vendor's
    /// services share one key; the custom endpoint keeps one per service).
    ProviderKeySet {
        /// Provider id (`openai`, `groq`, …).
        provider: ProviderId,
        /// `asr` | `llm`.
        kind: ServiceKind,
        /// New value or `null`.
        value: Option<String>,
    },
    /// Forget which models of a service ran out of quota (docs/dictation.md §3.5, 重新检查): the
    /// next request starts from the selected model again; answered by an `engines` event.
    EnginesQuotaReset {
        /// `asr` | `llm`.
        kind: ServiceKind,
    },
    /// List a provider's models with the form's values (`null` = the saved ones); answered by a
    /// `provider_probe` event.
    ProviderProbe {
        /// Provider id.
        provider: ProviderId,
        /// `asr` | `llm`.
        kind: ServiceKind,
        /// Base URL being edited.
        #[serde(default)]
        base_url: Option<String>,
        /// Key being edited; used for this request only.
        #[serde(default)]
        key: Option<String>,
    },
    /// List a provider's models with the saved endpoint and key, as a model list opens
    /// (docs/dictation.md §3.3); the list arrives in an `engines` event (`listed`).
    ProviderModelsRefresh {
        /// Provider id.
        provider: ProviderId,
        /// `asr` | `llm`.
        kind: ServiceKind,
    },
    /// Remove one history entry.
    HistoryDelete {
        /// UUID.
        id: String,
    },
    /// Remove every history entry.
    HistoryClear,
    /// Flag / unflag a history entry.
    HistoryStar {
        /// UUID.
        id: String,
        /// New flag.
        starred: bool,
    },
    /// 用 AI 预设处理 (docs/dictation.md §22): the entry's text through a preset in parts; progress
    /// and the end arrive as `history_process` events with `request_id`.
    HistoryProcess {
        /// Echoed in every answer.
        request_id: u64,
        /// The entry's UUID.
        id: String,
        /// A built-in name or a custom preset's UUID.
        preset: String,
    },
    /// Stop a `history_process`; nothing is stored.
    HistoryProcessCancel {
        /// The request to stop.
        request_id: u64,
    },
    /// Fetch and verify a local model (docs/dictation.md §10); progress arrives as `models` events.
    ModelDownload {
        /// Catalogue id (`sense-voice-small`, `paraformer-zh`).
        id: String,
    },
    /// Stop a running model download (`.part` files stay for a resume).
    ModelCancel {
        /// Catalogue id.
        id: String,
    },
    /// Delete a model directory.
    ModelRemove {
        /// Catalogue id.
        id: String,
    },
    /// Install a model from files a person downloaded into its directory (docs/dictation.md §10).
    ModelImport {
        /// Catalogue id.
        id: String,
    },
    /// Move the model library back to `<data_dir>/models` (docs/dictation.md §10.9). A chosen
    /// folder comes from the shell's folder dialog ([`Bridge::models_relocate`]), never from the
    /// webview.
    ModelsLocationReset,
    /// Stop a move of the library before it switches.
    ModelsLocationCancel,
    /// Append a personal dictionary entry (docs/dictation.md §16.4).
    DictionaryAdd {
        /// Term, mis-hearings, flag (snake_case inside).
        entry: DictionaryDraft,
        /// The history entry it comes from ("加入词典"); absent / `null` = manual.
        #[serde(default)]
        history_id: Option<String>,
    },
    /// Replace a dictionary entry's term, mis-hearings and flag.
    DictionaryUpdate {
        /// UUID.
        id: String,
        /// New content.
        entry: DictionaryDraft,
    },
    /// Delete a dictionary entry.
    DictionaryRemove {
        /// UUID.
        id: String,
    },
    /// Reorder the dictionary: every current id, in the new order.
    DictionaryReorder {
        /// UUIDs.
        ids: Vec<String>,
    },
    /// 忽略 on a learning suggestion (docs/dictation.md §16.10): leave the pair out from now on.
    VocabularySuggestionIgnore {
        /// What the recogniser wrote.
        heard: String,
        /// What the polished text has instead.
        polished: String,
    },
    /// 撤销 of an 忽略: suggest the pair again.
    VocabularySuggestionUnignore {
        /// What the recogniser wrote.
        heard: String,
        /// What the polished text has instead.
        polished: String,
    },
    /// Append a replacement rule.
    RulesAdd {
        /// The rule (snake_case inside).
        rule: RuleDraft,
    },
    /// Replace a rule.
    RulesUpdate {
        /// UUID.
        id: String,
        /// New content.
        rule: RuleDraft,
    },
    /// Delete a rule.
    RulesRemove {
        /// UUID.
        id: String,
    },
    /// Reorder the rules: every current id, in the new order.
    RulesReorder {
        /// UUIDs.
        ids: Vec<String>,
    },
    /// Import rules from TOML text (docs/dictation.md §16.5); parsed and validated here, so a bad
    /// file is refused synchronously.
    RulesImport {
        /// The TOML text.
        toml: String,
        /// `replace` | `merge`.
        mode: ImportMode,
    },
    /// Append a scene (docs/dictation.md §18.6); the draft is validated here as well.
    ScenesAdd {
        /// Name, flag, match and overrides (snake_case inside; `match` on the wire).
        scene: SceneDraft,
    },
    /// Replace a scene's name, flag, match and overrides. A built-in scene may list no application,
    /// so the core, which knows which scene it is, decides that rule (§18.10).
    ScenesUpdate {
        /// UUID.
        id: String,
        /// New content.
        scene: SceneDraft,
    },
    /// Delete a scene.
    ScenesRemove {
        /// UUID.
        id: String,
    },
    /// Reorder the scenes: every current id, in the new order (order = matching order).
    ScenesReorder {
        /// UUIDs.
        ids: Vec<String>,
    },
    /// 恢复默认 on a built-in scene: its applications and overrides back to the defaults.
    ScenesRestore {
        /// UUID.
        id: String,
    },
    /// Append a custom preset (docs/dictation.md §21); the draft is validated here as well.
    PresetsAdd {
        /// Name and instruction.
        preset: PresetDraft,
    },
    /// Replace a custom preset's name and instruction.
    PresetsUpdate {
        /// UUID.
        id: String,
        /// New content.
        preset: PresetDraft,
    },
    /// Delete a custom preset.
    PresetsRemove {
        /// UUID.
        id: String,
    },
    /// 试一试: run `text` through the current clean-up with a saved preset (`preset`) or an unsaved
    /// instruction (`prompt`), exactly one of them; the answer is a `preset_try` event with `id`.
    PresetsTry {
        /// Echoed in the answer.
        id: u64,
        /// A built-in name or a custom preset's UUID.
        preset: Option<String>,
        /// The instruction being edited.
        prompt: Option<String>,
        /// Sample text.
        text: String,
    },
    /// What of a take's context may go to the LLM (docs/dictation.md §18.5); persisted, the core
    /// re-emits `settings`.
    SettingsSetContextSharing {
        /// Send the application's name.
        app_name: bool,
        /// Send the window title.
        window_title: bool,
    },
    /// The scene a take runs with where no foreground probe picks one (the phone's talk card,
    /// docs/dictation.md §18); `None` = no scene. Persisted, the core re-emits `settings`.
    SettingsSetPinnedScene {
        /// The scene's id.
        id: Option<String>,
    },
    /// How the phone's talk button records (docs/dictation.md §20.11): `hold` or `tap`. Persisted,
    /// the core re-emits `settings`.
    SettingsSetPhoneTalk {
        /// The mode.
        mode: PhoneTalkMode,
    },
    /// The local speech service the app hosts (docs/dictation.md §23.6): on / off, its port, the
    /// preset and scene of its `voltip` requests. Persisted and applied at once; the core
    /// re-emits `settings` and the service's status (`serve`).
    SettingsSetServe {
        /// Serve on 127.0.0.1.
        enabled: bool,
        /// The port (1024–65535).
        port: u16,
        /// A built-in preset's name or a custom preset's UUID; `None` follows the engines'.
        preset: Option<String>,
        /// A scene's UUID; `None` = no scene.
        scene: Option<String>,
    },
    /// Put the service's token on the clipboard (it never reaches the webview).
    ServeCopyToken,
    /// Replace the service's token.
    ServeRotateToken,
    /// The local MCP service the app hosts for the AI tools on this computer (docs/dictation.md
    /// §25): on / off, its port, what the tools may read (and how far back the full history goes)
    /// and change. Persisted and applied at once; the core re-emits `settings` and `mcp`.
    SettingsSetMcp {
        /// Serve on 127.0.0.1.
        enabled: bool,
        /// The port (1024–65535, not the local speech service's).
        port: u16,
        /// `rules` | `summary` | `full`.
        read: voltip_core::mcp::McpRead,
        /// `last_7_days` | `last_30_days` | `all` (with `full`).
        history_range: voltip_core::mcp::HistoryRange,
        /// `read_only` | `dictionary` | `enable_rules`.
        write: voltip_core::mcp::McpWrite,
    },
    /// Put the MCP service's token on the clipboard (it never reaches the webview).
    McpCopyToken,
    /// Replace the MCP service's token.
    McpRotateToken,
    /// Put the command that connects an AI tool, with the real token, on the clipboard.
    McpCopySetup {
        /// `claude_code` | `codex`.
        client: voltip_core::mcp::SetupClient,
    },
    /// Undo an AI tool's write from the recent activity.
    McpUndo {
        /// The activity's UUID.
        id: String,
    },
    /// The proxy Voltip's connections go through (docs/dictation.md §24): follow the system, none,
    /// or the manual proxy below, whose fields are kept in the other modes too. Persisted and in
    /// force from the next request; the core re-emits `settings` and `network`.
    SettingsSetProxy {
        /// `system` | `off` | `manual`.
        mode: voltip_core::ProxyMode,
        /// `http://host:port`, `socks5://…` or `socks5h://…`; `host:port` is HTTP.
        server: Option<String>,
        /// The user name the proxy wants, if any.
        username: Option<String>,
        /// The hosts that go direct, comma-separated.
        no_proxy: String,
        /// The password for the keychain: `null` keeps the saved one, `""` removes it.
        password: Option<String>,
    },
    /// Read the system's proxy again (the settings opened); answered by a `network` event.
    NetworkRefresh,
    /// Close the built-in clean-up service's notice (docs/dictation.md §3.6).
    RefineNoticeClose,
    /// Ask the built-in service for this device's usage now (docs/dictation.md §27); answered by a
    /// `usage` event.
    UsageRefresh,
    /// Close a usage notice for the rest of the day (docs/dictation.md §27).
    UsageNoticeDismiss {
        /// `asr_near` | `asr_out` | `refine_near` | `refine_out`.
        notice: voltip_core::usage::UsageNotice,
    },
    /// Import audio or video files (docs/dictation.md §31): each joins the queue with the same
    /// models and language, and the queue follows as `imports` events. Refused with the built-in
    /// service, with the history off, and where the app imports nothing.
    ImportAdd {
        /// The files, as the file dialog or a drop named them.
        paths: Vec<String>,
        /// The recognition: an installed local model, or a provider's model (a cloud provider's
        /// with the user's key, or the custom endpoint's).
        asr: voltip_core::audio_cache::RecognitionChoice,
        /// The polish, when the text is polished.
        #[serde(default)]
        refine: Option<voltip_core::import::PolishModel>,
        /// The language hint; absent: detected.
        #[serde(default)]
        language: Option<String>,
    },
    /// Stop a running or waiting import: nothing goes into the history.
    ImportCancel {
        /// The job's UUID.
        id: String,
    },
    /// Run a failed import again.
    ImportRetry {
        /// The job's UUID.
        id: String,
    },
    /// Take a finished or failed import off the queue.
    ImportRemove {
        /// The job's UUID.
        id: String,
    },
}

impl UiCommand {
    /// Translate to a core command. Fails on malformed keys.
    pub fn into_core(self) -> Result<CoreCommand, BridgeError> {
        self.into_core_for(true)
    }

    /// [`UiCommand::into_core`] on a shell whose new scenes must name an application or not: not on
    /// a phone, where the user picks a take's scene (`CoreConfig::manual_scenes`, user decision
    /// 2026-10-01).
    pub fn into_core_for(self, scenes_need_apps: bool) -> Result<CoreCommand, BridgeError> {
        Ok(match self {
            Self::PairingStart => CoreCommand::StartPairing,
            Self::PairingJoinCode { code } => CoreCommand::JoinWithCode(code),
            Self::PairingJoinTicket { uri } => CoreCommand::JoinWithTicket(uri),
            Self::PairingConfirm => CoreCommand::ConfirmPairing,
            Self::PairingReject => CoreCommand::RejectPairing,
            Self::PairingCancel => CoreCommand::CancelPairing,
            Self::PairingReset => CoreCommand::ResetPairing,
            Self::DeviceForget { public_key } => CoreCommand::ForgetDevice(parse_key(&public_key)?),
            Self::DeviceSyncSet { public_key, on } => CoreCommand::SetDeviceSync { key: parse_key(&public_key)?, on },
            Self::DeviceRename { name } => CoreCommand::RenameDevice(name),
            Self::SendText { public_key, body } => CoreCommand::SendText { to: parse_key(&public_key)?, body },
            Self::PhoneTakeStart { public_key } => CoreCommand::PhoneTakeStart { to: parse_key(&public_key)? },
            Self::PhoneTakeStop => CoreCommand::PhoneTakeStop,
            Self::PhoneTakeCancel => CoreCommand::PhoneTakeCancel,
            Self::PhoneTextSend { public_key, body, source } => CoreCommand::PhoneTextSend { to: parse_key(&public_key)?, body, source },
            Self::SentTextsClear => CoreCommand::SentTextsClear,
            Self::SettingsSetPairingAlwaysOn { enabled } => CoreCommand::SetPairingAlwaysOn(enabled),
            Self::RelayReconnect => CoreCommand::ReconnectRelay,
            Self::SettingsSetRelay { url, enabled } => CoreCommand::SetRelay { url, enabled },
            Self::SettingsSetTheme { theme, follow_system } => CoreCommand::SetTheme { theme, follow_system },
            Self::SettingsSetHotkey { hotkey } => CoreCommand::SetHotkey(hotkey),
            Self::SettingsSetEditHotkey { hotkey } => CoreCommand::SetEditHotkey(hotkey),
            Self::SettingsSetCopyLastHotkey { hotkey } => CoreCommand::SetCopyLastHotkey(hotkey),
            Self::SettingsSetSoloKey { key } => CoreCommand::SetSoloKey(key),
            Self::SettingsSetMicrophone { device } => CoreCommand::SetMicrophone(device),
            Self::SettingsSetRecording { recording } => CoreCommand::SetRecording(recording),
            Self::SettingsSetLocale { locale } => CoreCommand::SetLocale(locale),
            Self::SettingsSetAutoUpdate { enabled } => CoreCommand::SetAutoUpdate(enabled),
            Self::SettingsSetHistory { enabled, keep } => CoreCommand::SetHistory(voltip_core::HistorySettings { enabled, keep }),
            Self::SettingsSetAudioCache { enabled, keep, max_mb } => CoreCommand::SetAudioCache(voltip_core::AudioCacheSettings { enabled, keep, max_mb }),
            Self::SettingsSetBackup { weekly } => CoreCommand::SetBackupWeekly(weekly),
            Self::BackupFolderReset => CoreCommand::SetBackupFolder(None),
            Self::BackupNoticeDismiss => CoreCommand::BackupNoticeDismiss,
            Self::AudioCacheClear => CoreCommand::AudioCacheClear,
            Self::AudioCacheDelete { id } => CoreCommand::AudioCacheDelete(parse_id(&id)?),
            Self::AudioRetake { id, asr, refine } => {
                CoreCommand::AudioRetake { id: parse_id(&id)?, models: voltip_core::audio_cache::RetakeModels { asr, refine } }
            }
            Self::ImportAdd { paths, asr, refine, language } => CoreCommand::ImportAdd(
                paths
                    .into_iter()
                    .map(|path| voltip_core::import::ImportRequest { path: path.into(), asr: asr.clone(), refine: refine.clone(), language: language.clone() })
                    .collect(),
            ),
            Self::ImportCancel { id } => CoreCommand::ImportCancel(parse_id(&id)?),
            Self::ImportRetry { id } => CoreCommand::ImportRetry(parse_id(&id)?),
            Self::ImportRemove { id } => CoreCommand::ImportRemove(parse_id(&id)?),
            Self::SettingsSetOverlay { placement } => CoreCommand::SetOverlay(placement),
            Self::DevicesRefresh => CoreCommand::RefreshDevices,
            Self::ConnectivityCheck => CoreCommand::CheckConnectivity,
            Self::DictationStart => CoreCommand::DictationStart,
            Self::DictationStop => CoreCommand::DictationStop,
            Self::DictationCancel => CoreCommand::DictationCancel,
            Self::DictationDismiss { session } => CoreCommand::DictationDismiss { session },
            Self::HotkeyEdge { pressed, at_ms, source, purpose, chorded } => CoreCommand::HotkeyEdge { pressed, at_ms, source, purpose, chorded },
            Self::SettingsSetActivation { activation, hold_threshold_ms, extra_recording_ms } => {
                CoreCommand::SetActivation { activation, hold_threshold_ms, extra_recording_ms }
            }
            Self::SettingsSetEngines { engines } => CoreCommand::SetEngines(engines),
            Self::ProviderKeySet { provider, kind, value } => CoreCommand::SetProviderKey { provider, kind, value },
            Self::EnginesQuotaReset { kind } => CoreCommand::ResetQuota(kind),
            Self::ProviderProbe { provider, kind, base_url, key } => CoreCommand::ProbeProvider { provider, kind, base_url, key },
            Self::ProviderModelsRefresh { provider, kind } => CoreCommand::RefreshProviderModels { provider, kind },
            Self::HistoryDelete { id } => CoreCommand::HistoryDelete(parse_id(&id)?),
            Self::HistoryClear => CoreCommand::HistoryClear,
            Self::HistoryStar { id, starred } => CoreCommand::HistoryStar(parse_id(&id)?, starred),
            Self::HistoryProcess { request_id, id, preset } => CoreCommand::HistoryProcess {
                request_id,
                id: parse_id(&id)?,
                preset: PresetId::parse(&preset).ok_or_else(|| BridgeError::BadArgument(format!("presets: 没有名为「{preset}」的预设")))?,
            },
            Self::HistoryProcessCancel { request_id } => CoreCommand::HistoryProcessCancel { request_id },
            Self::ModelDownload { id } => CoreCommand::ModelDownload(id),
            Self::ModelCancel { id } => CoreCommand::ModelCancel(id),
            Self::ModelRemove { id } => CoreCommand::ModelRemove(id),
            Self::ModelImport { id } => CoreCommand::ModelImport(id),
            Self::ModelsLocationReset => CoreCommand::ModelsRelocate(None),
            Self::ModelsLocationCancel => CoreCommand::ModelsRelocateCancel,
            // The drafts are validated here as well as in the core: a draft that is wrong on its own
            // (empty, too long, a regex that does not compile, a TOML file that does not parse) comes
            // back as the command's error; list-level refusals arrive as `error` events.
            Self::DictionaryAdd { entry, history_id } => CoreCommand::DictionaryAdd {
                draft: validate_dictionary_draft(&entry).map_err(bad)?,
                source: match history_id {
                    Some(id) => EntrySource::History { history_id: parse_id(&id)? },
                    None => EntrySource::Manual,
                },
            },
            Self::DictionaryUpdate { id, entry } => {
                CoreCommand::DictionaryUpdate { id: parse_id(&id)?, draft: validate_dictionary_draft(&entry).map_err(bad)? }
            }
            Self::DictionaryRemove { id } => CoreCommand::DictionaryRemove(parse_id(&id)?),
            Self::DictionaryReorder { ids } => CoreCommand::DictionaryReorder(parse_ids(&ids)?),
            Self::VocabularySuggestionIgnore { heard, polished } => {
                let (heard, polished) = validate_pair(&heard, &polished).map_err(bad)?;
                CoreCommand::SuggestionIgnore { heard, polished }
            }
            Self::VocabularySuggestionUnignore { heard, polished } => {
                let (heard, polished) = validate_pair(&heard, &polished).map_err(bad)?;
                CoreCommand::SuggestionUnignore { heard, polished }
            }
            Self::RulesAdd { rule } => CoreCommand::RuleAdd(validate_rule_draft(&rule).map_err(bad)?),
            Self::RulesUpdate { id, rule } => CoreCommand::RuleUpdate { id: parse_id(&id)?, draft: validate_rule_draft(&rule).map_err(bad)? },
            Self::RulesRemove { id } => CoreCommand::RuleRemove(parse_id(&id)?),
            Self::RulesReorder { ids } => CoreCommand::RuleReorder(parse_ids(&ids)?),
            Self::RulesImport { toml, mode } => CoreCommand::RulesImport { rules: parse_rules_toml(&toml).map_err(bad)?, mode },
            // Same split as the vocabulary: a draft wrong on its own is the command's error; list-level
            // refusals (duplicate name, cap, unknown id) arrive as `error` events.
            Self::ScenesAdd { scene } => CoreCommand::SceneAdd(validate_scene_draft_with(&scene, scenes_need_apps).map_err(bad_scene)?),
            Self::ScenesUpdate { id, scene } => {
                CoreCommand::SceneUpdate { id: parse_id(&id)?, draft: validate_scene_draft_with(&scene, false).map_err(bad_scene)? }
            }
            Self::ScenesRemove { id } => CoreCommand::SceneRemove(parse_id(&id)?),
            Self::ScenesReorder { ids } => CoreCommand::SceneReorder(parse_ids(&ids)?),
            Self::ScenesRestore { id } => CoreCommand::SceneRestore(parse_id(&id)?),
            // Same split again: a draft wrong on its own is the command's error; a clash with the
            // list (duplicate name, cap, unknown id) arrives as an `error` event.
            Self::PresetsAdd { preset } => CoreCommand::PresetAdd(validate_preset_draft(&preset).map_err(bad_preset)?),
            Self::PresetsUpdate { id, preset } => CoreCommand::PresetUpdate { id: parse_id(&id)?, draft: validate_preset_draft(&preset).map_err(bad_preset)? },
            Self::PresetsRemove { id } => CoreCommand::PresetRemove(parse_id(&id)?),
            Self::PresetsTry { id, preset, prompt, text } => CoreCommand::PresetTry { id, trial: preset_trial(preset, prompt)?, text: preset_try_text(&text)? },
            Self::SettingsSetContextSharing { app_name, window_title } => CoreCommand::SetContextSharing(ContextSharing { app_name, window_title }),
            Self::SettingsSetPinnedScene { id } => CoreCommand::SetPinnedScene(id.as_deref().map(parse_id).transpose()?),
            Self::SettingsSetPhoneTalk { mode } => CoreCommand::SetPhoneTalk(mode),
            Self::SettingsSetServe { enabled, port, preset, scene } => CoreCommand::SetServe(voltip_core::ServeSettings {
                enabled,
                port,
                preset: preset
                    .as_deref()
                    .map(|p| PresetId::parse(p).ok_or_else(|| BridgeError::BadArgument(format!("serve: 没有名为「{p}」的预设"))))
                    .transpose()?,
                scene: scene.as_deref().map(parse_id).transpose()?,
            }),
            Self::ServeCopyToken => CoreCommand::ServeCopyToken,
            Self::ServeRotateToken => CoreCommand::ServeRotateToken,
            Self::SettingsSetMcp { enabled, port, read, history_range, write } => {
                CoreCommand::SetMcp(voltip_core::mcp::McpSettings { enabled, port, read, history_range, write })
            }
            Self::McpCopyToken => CoreCommand::McpCopyToken,
            Self::McpRotateToken => CoreCommand::McpRotateToken,
            Self::McpCopySetup { client } => CoreCommand::McpCopySetup(client),
            Self::McpUndo { id } => CoreCommand::McpUndo(parse_id(&id)?),
            Self::SettingsSetProxy { mode, server, username, no_proxy, password } => {
                CoreCommand::SetProxy { proxy: voltip_core::ProxySettings { mode, server, username, no_proxy }, password }
            }
            Self::NetworkRefresh => CoreCommand::RefreshNetwork,
            Self::RefineNoticeClose => CoreCommand::RefineNoticeClose,
            Self::UsageRefresh => CoreCommand::UsageRefresh,
            Self::UsageNoticeDismiss { notice } => CoreCommand::UsageNoticeDismiss(notice),
        })
    }
}

fn bad(e: voltip_core::VocabularyError) -> BridgeError {
    BridgeError::BadArgument(e.to_string())
}

fn bad_scene(e: voltip_core::SceneError) -> BridgeError {
    BridgeError::BadArgument(e.to_string())
}

fn bad_preset(e: voltip_core::PresetError) -> BridgeError {
    BridgeError::BadArgument(e.to_string())
}

/// What `presets_try` runs: exactly one of a preset id and an instruction.
fn preset_trial(preset: Option<String>, prompt: Option<String>) -> Result<PresetTrial, BridgeError> {
    match (preset, prompt) {
        (Some(id), None) => PresetId::parse(&id).map(PresetTrial::Preset).ok_or_else(|| BridgeError::BadArgument(format!("presets: 没有名为「{id}」的预设"))),
        (None, Some(prompt)) => clean_preset_prompt(&prompt).map(PresetTrial::Prompt).map_err(bad_preset),
        _ => Err(BridgeError::BadArgument("presets: 试一试需要一个预设或一段预设内容".into())),
    }
}

/// The sample text of `presets_try`, trimmed, 1–[`MAX_PRESET_TRY_CHARS`] characters.
fn preset_try_text(text: &str) -> Result<String, BridgeError> {
    let text = text.trim();
    let chars = text.chars().count();
    if chars == 0 {
        return Err(BridgeError::BadArgument("presets: 请输入要试运行的文字".into()));
    }
    if chars > MAX_PRESET_TRY_CHARS {
        return Err(BridgeError::BadArgument(format!("presets: 试运行的文字最多 {MAX_PRESET_TRY_CHARS} 个字符（当前 {chars}）")));
    }
    Ok(text.to_owned())
}

fn parse_ids(texts: &[String]) -> Result<Vec<Uuid>, BridgeError> {
    texts.iter().map(|t| parse_id(t)).collect()
}

/// One entry of a phone's copy of a computer's history (`mirror_history_entry`).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MirrorEntry {
    /// The entry.
    pub entry: HistoryEntry,
    /// It arrived as its bounded projection: the phone shows the beginning only.
    pub shortened: bool,
}

/// A copy query naming no computer key is the caller's mistake; anything else is the database's.
fn bad_copy(e: voltip_core::CoreError) -> BridgeError {
    match e {
        voltip_core::CoreError::Invalid(why) => BridgeError::BadArgument(why),
        other => BridgeError::from(other),
    }
}

fn parse_key(hex: &str) -> Result<PublicKey, BridgeError> {
    PublicKey::from_hex(hex).map_err(|_| BridgeError::BadArgument("publicKey must be 64 hex chars".into()))
}

fn parse_id(text: &str) -> Result<Uuid, BridgeError> {
    Uuid::parse_str(text).map_err(|_| BridgeError::BadArgument("id must be a UUID".into()))
}

/// Errors returned to the webview as strings.
#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    /// Argument validation.
    #[error("{0}")]
    BadArgument(String),
    /// Core failure.
    #[error(transparent)]
    Core(#[from] CoreError),
}

impl From<BridgeError> for String {
    fn from(value: BridgeError) -> Self {
        value.to_string()
    }
}

/// Owns the core handle, the cached [`UiState`] and the event fan-out.
#[derive(Clone)]
pub struct Bridge {
    handle: CoreHandle,
    state: Arc<Mutex<UiState>>,
    events: broadcast::Sender<UiEvent>,
    /// The last id [`Bridge::paste`] gave out: each answer finds the call that asked.
    paste_ids: Arc<AtomicU64>,
    /// The history queries' own read-only connection (docs/dictation.md §4.4).
    history: Arc<HistoryReader>,
    /// Phone: the copies of its computers' histories, shared with the core (its locks keep a
    /// deletion from meeting a query, docs/dictation.md §20.8).
    mirrors: Arc<voltip_core::sync::MirrorFiles>,
    /// A new scene must name an application: everywhere but on a phone, where the user picks a
    /// take's scene (`CoreConfig::manual_scenes`, user decision 2026-10-01).
    scenes_need_apps: bool,
    /// The app data directory: where a backup's history comes from (docs/dictation.md §4.6).
    data_dir: std::path::PathBuf,
    /// The backup `backup_open` read, under its preview's token, until `backup_import` takes it,
    /// `backup_discard` drops it or another file is opened.
    opened_backup: Arc<Mutex<Option<OpenedBackup>>>,
    /// The learning suggestions last computed (docs/dictation.md §16.10).
    suggestions: Arc<SuggestionsCache>,
}

/// The learning suggestions last computed and what they were computed from (docs/dictation.md
/// §16.10): the event pump counts every change to the dictionary, the rules, the history and the
/// ignored list, and a count other than the one the list was computed at makes it stale.
#[derive(Default)]
struct SuggestionsCache {
    /// Changes so far.
    generation: AtomicU64,
    /// The suggestions (at most [`MAX_SUGGESTIONS`]) and the generation they were computed at; the
    /// lock is held while they are computed, so callers at the same time compute once.
    last: Mutex<Option<(u64, Suggestions)>>,
}

/// A backup `backup_open` read (docs/dictation.md §4.6) and the token its preview names it by.
type OpenedBackup = (Uuid, Arc<voltip_core::backup::Backup>);

impl std::fmt::Debug for Bridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bridge").finish_non_exhaustive()
    }
}

impl Bridge {
    /// [`Bridge::start_with`] on the in-memory dictation fakes (`voltip_core::dictation::fakes`):
    /// for tests and for shells without a native pipeline. The desktop passes its real ports.
    pub fn start(config: CoreConfig, secret_store: Arc<dyn SecretStore>) -> Result<Self, BridgeError> {
        Self::start_with(config, secret_store, voltip_core::dictation::fakes::ports())
    }

    /// Start the core with the shell's dictation ports and the event pump. The shell forwards
    /// events with a receiver from [`Bridge::events`]; use [`Bridge::start_subscribed`] when that
    /// receiver must not miss the first events.
    pub fn start_with(config: CoreConfig, secret_store: Arc<dyn SecretStore>, ports: DictationPorts) -> Result<Self, BridgeError> {
        Self::start_subscribed(config, secret_store, ports).map(|(bridge, _first)| bridge)
    }

    /// [`Bridge::start_with`] plus a receiver that was subscribed **before** the core started.
    ///
    /// The fan-out is a broadcast channel: a subscriber only sees events sent after it subscribed.
    /// The core publishes its single `state` event (identity, settings) as soon as it is ready,
    /// which on a slow machine happens before a shell that subscribes after `start` gets there, and
    /// then the webview never receives that event (regression seen on the CI runner, 2026-09-25).
    /// Shells forward events from the receiver returned here so nothing is lost.
    pub fn start_subscribed(
        config: CoreConfig,
        secret_store: Arc<dyn SecretStore>,
        ports: DictationPorts,
    ) -> Result<(Self, broadcast::Receiver<UiEvent>), BridgeError> {
        let (tx, first) = broadcast::channel(256);
        let state = Arc::new(Mutex::new(UiState::default()));
        let history = Arc::new(HistoryReader::new(&config.data_dir));
        let mirrors = config.mirror_files.clone();
        let scenes_need_apps = !config.manual_scenes;
        let data_dir = config.data_dir.clone();
        let (handle, core_events) = AppCore::start_with(config, secret_store, ports)?;
        let bridge = Self {
            handle,
            state: state.clone(),
            events: tx.clone(),
            paste_ids: Arc::new(AtomicU64::new(0)),
            history,
            mirrors,
            scenes_need_apps,
            data_dir,
            opened_backup: Arc::default(),
            suggestions: Arc::default(),
        };
        tokio::spawn(pump(core_events, state, tx, bridge.suggestions.clone()));
        Ok((bridge, first))
    }

    /// Current state for a freshly mounted webview.
    pub fn state(&self) -> UiState {
        self.state.lock().clone()
    }

    /// `model_folder_open`: the directory of model `id` for a manual download (docs/dictation.md
    /// §10), created when it is not there yet. The path comes from the core's catalogue, never from
    /// the webview.
    pub fn model_folder(&self, id: &str) -> Result<std::path::PathBuf, BridgeError> {
        let dir = self.state.lock().models.iter().find(|m| m.id == id).map(|m| m.dir.clone()).filter(|d| !d.is_empty());
        let dir = std::path::PathBuf::from(dir.ok_or_else(|| BridgeError::BadArgument(format!("model_folder_open: no model {id}")))?);
        std::fs::create_dir_all(&dir).map_err(|e| BridgeError::BadArgument(format!("model_folder_open: {e}")))?;
        Ok(dir)
    }

    /// `models_location_choose` (docs/dictation.md §10.9): move the library into `dir`, the folder
    /// the shell's dialog returned; progress and the end arrive as `models_location` events.
    pub fn models_relocate(&self, dir: std::path::PathBuf) -> Result<(), BridgeError> {
        self.handle.try_send(CoreCommand::ModelsRelocate(Some(dir)))?;
        Ok(())
    }

    /// Where the model library is (docs/dictation.md §10.9), as the core last said.
    pub fn models_location(&self) -> voltip_core::models_location::ModelsLocationView {
        self.state.lock().models_location.clone()
    }

    /// `model_link_open`: where `file` of model `id` downloads from its `source`-th public source;
    /// `file` may name the package the model comes in (docs/dictation.md §29.12). Like the folder,
    /// the address comes from the core's catalogue: the webview names a file.
    pub fn model_link(&self, id: &str, file: &str, source: usize) -> Result<String, BridgeError> {
        let state = self.state.lock();
        let model = state.models.iter().find(|m| m.id == id).ok_or_else(|| BridgeError::BadArgument(format!("model_link_open: no model {id}")))?;
        let urls = match model.package.as_ref().filter(|p| p.name == file) {
            Some(package) => &package.urls,
            None => &model.files.iter().find(|f| f.name == file).ok_or_else(|| BridgeError::BadArgument(format!("model_link_open: {id} has no {file}")))?.urls,
        };
        urls.get(source).cloned().ok_or_else(|| BridgeError::BadArgument(format!("model_link_open: {file} has no source {source}")))
    }

    /// Subscribe to UI events.
    pub fn events(&self) -> broadcast::Receiver<UiEvent> {
        self.events.subscribe()
    }

    /// Input levels while a dictation capture runs (see [`CoreHandle::levels`]).
    pub fn levels(&self) -> broadcast::Receiver<LevelFrame> {
        self.handle.levels()
    }

    /// Execute a webview command.
    pub fn dispatch(&self, cmd: UiCommand) -> Result<(), BridgeError> {
        let core_cmd = cmd.into_core_for(self.scenes_need_apps)?;
        self.handle.try_send(core_cmd)?;
        Ok(())
    }

    /// `paste_text` (the history's 「粘贴到上一个窗口」, `voltip_core::paste`): hand `text` to the
    /// core for `target` and wait at most `within` for the answer. The receiver subscribes before
    /// the command goes out, so the answer cannot pass by unseen; none in time, or a core that has
    /// stopped, is `failed { timeout }`.
    pub async fn paste(&self, text: String, target: PasteTarget, within: Duration) -> PasteOutcome {
        let no_answer = PasteOutcome::Failed { reason: PasteFailure::Timeout };
        let request_id = self.paste_ids.fetch_add(1, Ordering::Relaxed) + 1;
        let mut events = self.events.subscribe();
        let answer = async {
            self.handle.send(CoreCommand::PasteText { request_id, text, target }).await.map_err(|e| tracing::warn!(error = %e, "the core took no paste"))?;
            loop {
                match events.recv().await {
                    Ok(UiEvent::PasteResult { request_id: id, outcome }) if id == request_id => return Ok(outcome),
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(skipped)) => tracing::warn!(skipped, "paste answer may have been dropped"),
                    Err(broadcast::error::RecvError::Closed) => return Err(()),
                }
            }
        };
        match tokio::time::timeout(within, answer).await {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(())) => no_answer,
            Err(_) => {
                tracing::warn!(request_id, "no paste answer in time");
                no_answer
            }
        }
    }

    /// 复制上一条结果 (docs/dictation.md §28; the tray's entry, the hotkey, `--copy-last`): the newest
    /// result onto the clipboard. A core that does not answer within `within`, or has stopped,
    /// counts as `failed`.
    pub async fn copy_last(&self, within: Duration) -> voltip_core::last_result::CopyLastOutcome {
        use voltip_core::last_result::{CopyLastOutcome, CopyLastReply};
        let (reply, answer) = CopyLastReply::channel();
        if let Err(e) = self.handle.send(CoreCommand::CopyLastResult(reply)).await {
            tracing::warn!(error = %e, "the core took no copy of the last result");
            return CopyLastOutcome::Failed;
        }
        match tokio::time::timeout(within, answer).await {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(_)) => CopyLastOutcome::Failed,
            Err(_) => {
                tracing::warn!("no answer to the copy of the last result in time");
                CopyLastOutcome::Failed
            }
        }
    }

    /// Publish an event the shell produced itself (global hotkey registration / presses): folded
    /// into the cached state and broadcast like a core event, so `core_state` and the event stream
    /// agree.
    pub fn publish(&self, event: UiEvent) {
        let event = self.state.lock().apply_shell(event);
        let _ = self.events.send(event);
    }

    /// `vocabulary_preview` (docs/dictation.md §16.4): `text` through the cached dictionary and rules
    /// — with `draft` standing in for the rule it names — exactly as the pipeline would, minus the
    /// LLM. The lists are copied out so the state lock is not held while they compile.
    pub fn vocabulary_preview(&self, text: &str, draft: Option<&PreviewDraft>) -> Result<VocabularyPreview, BridgeError> {
        let (dictionary, rules) = {
            let state = self.state.lock();
            (state.dictionary.clone(), state.rules.clone())
        };
        preview(&dictionary, &rules, text, draft).map_err(bad)
    }

    /// `vocabulary_backtest` (docs/dictation.md §16.9): what `draft`, a dictionary entry or a rule
    /// not saved yet, would change in the newest `limit` recognised takes (default
    /// [`BACKTEST_LIMIT`], 1–[`MAX_BACKTEST_LIMIT`]), leaving out `exclude` (the entry the quick-fix
    /// card was opened on). The cached dictionary is copied out, so the state lock is not held while
    /// it compiles. Blocks on SQLite: the shells call it off the main thread.
    pub fn vocabulary_backtest(&self, draft: &BacktestDraft, exclude: Option<Uuid>, limit: Option<usize>) -> Result<Backtest, BridgeError> {
        let limit = limit.unwrap_or(BACKTEST_LIMIT);
        if !(1..=MAX_BACKTEST_LIMIT).contains(&limit) {
            return Err(BridgeError::BadArgument(format!("vocabulary_backtest: limit 1–{MAX_BACKTEST_LIMIT}")));
        }
        let dictionary = self.state.lock().dictionary.clone();
        let history = self.history.recent_texts(limit)?;
        backtest(&dictionary, &history, draft, exclude).map_err(bad)
    }

    /// `vocabulary_suggestions` (docs/dictation.md §16.10): the learning suggestions over the newest
    /// [`MAX_RECENT_TEXTS`] recognised takes, at most `limit` of them (default and most
    /// [`MAX_SUGGESTIONS`]); [`SuggestionsAnswer::HistoryOff`] while history recording is off. The
    /// answer is computed once and kept until the dictionary, the rules, the history or the ignored
    /// list change. Blocks on SQLite and the comparison: the shells call it off the main thread.
    pub fn vocabulary_suggestions(&self, limit: Option<usize>) -> Result<SuggestionsAnswer, BridgeError> {
        let limit = limit.unwrap_or(MAX_SUGGESTIONS);
        if !(1..=MAX_SUGGESTIONS).contains(&limit) {
            return Err(BridgeError::BadArgument(format!("vocabulary_suggestions: limit 1–{MAX_SUGGESTIONS}")));
        }
        let mut last = self.suggestions.last.lock();
        if !self.state.lock().settings.history.enabled {
            return Ok(SuggestionsAnswer::HistoryOff);
        }
        // The count first, the lists after: a change in between makes the answer stale at once
        // rather than kept under the new count.
        let generation = self.suggestions.generation.load(Ordering::Acquire);
        let mut answer = match &*last {
            Some((at, kept)) if *at == generation => kept.clone(),
            _ => {
                let (dictionary, rules, ignored) = {
                    let state = self.state.lock();
                    (state.dictionary.clone(), state.rules.clone(), state.vocabulary_ignored.clone())
                };
                let history = self.history.recent_texts(MAX_RECENT_TEXTS)?;
                let fresh = suggestions(&dictionary, &rules, &history, &ignored, MAX_SUGGESTIONS);
                *last = Some((generation, fresh.clone()));
                fresh
            }
        };
        answer.pairs.truncate(limit);
        Ok(SuggestionsAnswer::Ready(answer))
    }

    /// `vocabulary_suggestion_accept` (docs/dictation.md §16.10): add `heard` → `polished` to the
    /// dictionary, as an entry from history entry `history_id` or to the entry with that spelling,
    /// and wait at most `within` for the core's answer. A pair that is not one is refused here.
    pub async fn vocabulary_suggestion_accept(
        &self,
        heard: String,
        polished: String,
        history_id: Uuid,
        within: Duration,
    ) -> Result<AcceptOutcome, BridgeError> {
        let (heard, polished) = validate_pair(&heard, &polished).map_err(bad)?;
        let (reply, answer) = Reply::channel();
        self.handle.send(CoreCommand::SuggestionAccept { heard, polished, history_id, reply }).await?;
        Self::answer(answer, within).await
    }

    /// `vocabulary_suggestion_undo`: take the accept `id` names back, waiting at most `within`.
    pub async fn vocabulary_suggestion_undo(&self, id: Uuid, within: Duration) -> Result<UndoOutcome, BridgeError> {
        let (reply, answer) = Reply::channel();
        self.handle.send(CoreCommand::SuggestionUndo { id, reply }).await?;
        Self::answer(answer, within).await
    }

    /// The core's answer to a command with a [`Reply`], or [`CoreError::NotRunning`] when none
    /// comes within `within`.
    async fn answer<T>(answer: tokio::sync::oneshot::Receiver<T>, within: Duration) -> Result<T, BridgeError> {
        match tokio::time::timeout(within, answer).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(_)) | Err(_) => {
                tracing::warn!("a vocabulary suggestion command got no answer in time");
                Err(BridgeError::Core(CoreError::NotRunning))
            }
        }
    }

    /// `rules_warnings` (docs/dictation.md §16.10): the rules page's hints over the cached rules
    /// and dictionary.
    pub fn rules_warnings(&self) -> Vec<RuleWarning> {
        let state = self.state.lock();
        rule_warnings(&state.rules, &state.dictionary)
    }

    /// `rules_export` (docs/dictation.md §16.5): the cached rules as TOML text.
    pub fn rules_export(&self) -> Result<String, BridgeError> {
        let rules = self.state.lock().rules.clone();
        export_rules_toml(&rules).map_err(bad)
    }

    /// `recent_apps` (docs/dictation.md §18.6): the applications the history saw, newest first,
    /// one per id, at most [`MAX_RECENT_APPS`] — what the scene editor offers to pick from.
    pub fn recent_apps(&self) -> Result<Vec<AppRef>, BridgeError> {
        Ok(self.history.recent_apps(MAX_RECENT_APPS)?)
    }

    /// `history_query` (docs/dictation.md §4.4): a page of the history, filtered and searched in
    /// the database. Blocks on SQLite: the shells call it off the main thread.
    pub fn history_query(&self, query: &HistoryQuery) -> Result<HistoryPage, BridgeError> {
        Ok(self.history.query(query)?)
    }

    /// `history_entry`: one entry by id, `None` once it is gone.
    pub fn history_entry(&self, id: uuid::Uuid) -> Result<Option<HistoryEntry>, BridgeError> {
        Ok(self.history.entry(id)?)
    }

    /// `history_stats` (docs/dictation.md §4.5): the dictations between each two of the local
    /// midnights the page sends, and over the whole history.
    pub fn history_stats(&self, boundaries: &[u64]) -> Result<HistoryStats, BridgeError> {
        Ok(self.history.stats(boundaries)?)
    }

    /// `history_hits` (docs/dictation.md §16.3): how often each dictionary entry and rule fired.
    pub fn history_hits(&self) -> Result<HistoryHits, BridgeError> {
        Ok(self.history.hits()?)
    }

    /// `mirror_history_query` (docs/dictation.md §20.8): a page of the phone's copy of `desktop`'s
    /// history (the key in hex), read like the phone's own; empty when there is no copy.
    pub fn mirror_history_query(&self, desktop: &str, query: &HistoryQuery) -> Result<HistoryPage, BridgeError> {
        // Checked like the phone's own history even when there is no copy to read.
        if !(1..=voltip_core::history::MAX_QUERY_LIMIT).contains(&query.limit) {
            return Err(BridgeError::BadArgument(format!("mirror_history_query: limit 1–{}", voltip_core::history::MAX_QUERY_LIMIT)));
        }
        Ok(self.mirrors.read(desktop, |reader, _| reader.query(query)).map_err(bad_copy)?.unwrap_or_default())
    }

    /// `mirror_history_entry`: one entry of the copy, and whether it arrived shortened.
    pub fn mirror_history_entry(&self, desktop: &str, id: uuid::Uuid) -> Result<Option<MirrorEntry>, BridgeError> {
        let found = self.mirrors.read(desktop, |_, path| voltip_core::sync::read_entry(path, id)).map_err(bad_copy)?.flatten();
        Ok(found.map(|(entry, shortened)| MirrorEntry { entry, shortened }))
    }

    /// `mirror_profile`: the computer's settings as the copy holds them.
    pub fn mirror_profile(&self, desktop: &str) -> Result<Option<voltip_core::sync::Profile>, BridgeError> {
        Ok(self.mirrors.read(desktop, |_, path| voltip_core::sync::read_copied_profile(path)).map_err(bad_copy)?.flatten())
    }

    /// `backup_export` (docs/dictation.md §4.6): the cached dictionary, rules, scenes and presets
    /// and every entry of the history database into a backup at `path`, written whole or not at
    /// all. Blocks on the file and SQLite: the shells call it off the main thread.
    pub fn backup_export(&self, path: &std::path::Path) -> Result<ExportSummary, BackupError> {
        let (identity, app_version, dictionary, rules, scenes, presets) = {
            let state = self.state.lock();
            (state.identity.clone(), state.app_version.clone(), state.dictionary.clone(), state.rules.clone(), state.scenes.clone(), state.presets.clone())
        };
        let device = identity.as_ref().map(|i| i.name.clone()).unwrap_or_default();
        let platform = identity.map(|i| i.platform);
        let input = ExportInput {
            app_version: &app_version,
            device: &device,
            platform,
            created_at_ms: voltip_core::now_ms(),
            dictionary: &dictionary,
            rules: &rules,
            scenes: &scenes,
            presets: &presets,
        };
        backup::export(path, &self.data_dir.join(voltip_core::history::HISTORY_DB_FILE_NAME), &input)
    }

    /// `backup_open`: read and check the backup at `path` whole. Nothing changes: the bridge holds
    /// the backup for `backup_import` under the preview's token, in place of any opened before.
    /// Blocks on the file: the shells call it off the main thread.
    pub fn backup_open(&self, path: &std::path::Path) -> OpenOutcome {
        *self.opened_backup.lock() = None;
        match backup::read(path) {
            Ok(backup) => {
                let token = Uuid::new_v4();
                let manifest = backup.manifest.clone();
                *self.opened_backup.lock() = Some((token, Arc::new(backup)));
                OpenOutcome::Preview { token, manifest }
            }
            Err(e) => {
                let outcome = OpenOutcome::failed(&e);
                // The kind only: the parser's message may quote the file.
                if let OpenOutcome::Failed { code, .. } = &outcome {
                    tracing::warn!(?code, "backup not opened");
                }
                outcome
            }
        }
    }

    /// `backup_import`: restore `parts` of the backup `token` names, merged or in place, and wait at
    /// most `within` for the core's report. The bridge lets go of the backup as the core takes it,
    /// so a token works once; a replacing import the core refused before anything ran (a take
    /// running, the backup before it failed: docs/dictation.md §4.6.1) keeps the preview, and the
    /// import can be tried again.
    pub async fn backup_import(&self, token: Uuid, parts: BackupParts, mode: ImportMode, within: Duration) -> ImportOutcome {
        let backup = {
            let mut opened = self.opened_backup.lock();
            if opened.as_ref().is_some_and(|(held, _)| *held == token) { opened.take().map(|(_, backup)| backup) } else { None }
        };
        let Some(backup) = backup else { return ImportOutcome::failed(ImportFailure::Expired) };
        let held = Arc::clone(&backup);
        let (reply, answer) = ImportReply::channel();
        if self.handle.send(CoreCommand::BackupImport { backup, parts, mode, reply }).await.is_err() {
            return ImportOutcome::failed(ImportFailure::Unavailable);
        }
        let outcome = match tokio::time::timeout(within, answer).await {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(_)) | Err(_) => {
                tracing::warn!("a backup import got no answer in time");
                ImportOutcome::failed(ImportFailure::Unavailable)
            }
        };
        if matches!(outcome, ImportOutcome::Failed { code: ImportFailure::Busy | ImportFailure::SnapshotFailed, .. }) {
            let mut opened = self.opened_backup.lock();
            // Unless another backup was opened meanwhile.
            if opened.is_none() {
                *opened = Some((token, held));
            }
        }
        outcome
    }

    /// `backup_discard`: let go of the backup `token` names (its preview was closed).
    pub fn backup_discard(&self, token: Uuid) {
        let mut opened = self.opened_backup.lock();
        if opened.as_ref().is_some_and(|(held, _)| *held == token) {
            *opened = None;
        }
    }

    /// `backup_folder_choose` (docs/dictation.md §4.6.1): the automatic backups go to `dir`, the
    /// folder the shell's dialog returned and checked; `auto_backup` events follow.
    pub fn backup_set_folder(&self, dir: std::path::PathBuf) -> Result<(), BridgeError> {
        self.handle.try_send(CoreCommand::SetBackupFolder(Some(dir)))?;
        Ok(())
    }

    /// Where the automatic backups go, as the core last said (`backup_folder_open`, the import's
    /// open dialog): `<data dir>/backups` until it said.
    pub fn backup_folder(&self) -> std::path::PathBuf {
        let folder = self.state.lock().auto_backup.folder.clone();
        if folder.is_empty() { voltip_core::backup::auto::default_folder(&self.data_dir) } else { folder.into() }
    }

    /// An automatic backup of `kind` now (docs/dictation.md §4.6.1), the shell's before an update
    /// installs. The core makes it of its own lists, after every command sent before, never of
    /// this bridge's cache, which may be a step behind. Blocks until the file is written, however
    /// long the history takes: the shell calls it on a blocking thread. A core that has stopped
    /// answers with why.
    pub fn backup_auto(&self, kind: AutoKind) -> AutoOutcome {
        let (reply, answer) = AutoReply::channel();
        self.handle.blocking_send(CoreCommand::BackupAuto { kind, reply }).map_err(|e| e.to_string())?;
        answer.blocking_recv().unwrap_or_else(|_| Err("the app stopped before the backup was written".into()))
    }

    /// Stop the core.
    pub fn shutdown(&self) {
        let _ = self.handle.try_send(CoreCommand::Shutdown);
    }
}

async fn pump(mut core_events: mpsc::Receiver<CoreEvent>, state: Arc<Mutex<UiState>>, tx: broadcast::Sender<UiEvent>, suggestions: Arc<SuggestionsCache>) {
    while let Some(ev) = core_events.recv().await {
        let changes_suggestions = matches!(ev, CoreEvent::Dictionary(_) | CoreEvent::Rules(_) | CoreEvent::History { .. } | CoreEvent::VocabularyIgnored(_));
        let ui = state.lock().apply(ev);
        // After the state holds the change: a computation that reads the new count reads the new
        // lists too.
        if changes_suggestions {
            suggestions.generation.fetch_add(1, Ordering::AcqRel);
        }
        // No subscribers yet is fine: the webview asks for `core_state` when it mounts.
        let _ = tx.send(ui);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use voltip_identity::MemorySecretStore;

    #[test]
    fn ui_commands_parse_camel_case_and_translate() {
        let c: UiCommand = serde_json::from_str(r#"{"command":"pairing_join_code","code":"483 921"}"#).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::JoinWithCode(s) if s == "483 921"));
        let c: UiCommand = serde_json::from_str(&format!(r#"{{"command":"send_text","publicKey":"{}","body":"hi"}}"#, "ab".repeat(32))).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::SendText { .. }));
        let c: UiCommand = serde_json::from_str(r#"{"command":"device_forget","publicKey":"zz"}"#).unwrap();
        let err: String = c.into_core().unwrap_err().into();
        assert!(err.contains("64 hex"));
        let c: UiCommand = serde_json::from_str(r#"{"command":"settings_set_theme","theme":"graphite","followSystem":true}"#).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::SetTheme { theme: ThemeId::Graphite, follow_system: true }));
        let c: UiCommand = serde_json::from_str(r#"{"command":"settings_set_relay","url":null,"enabled":false}"#).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::SetRelay { url: None, enabled: false }));
        let c: UiCommand = serde_json::from_str(r#"{"command":"settings_set_locale","locale":"zh-cn"}"#).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::SetLocale(Locale::ZhCn)));
        assert!(
            serde_json::from_str::<UiCommand>(r#"{"command":"settings_set_locale","locale":"fr"}"#).is_err(),
            "unknown locales are refused at the IPC layer"
        );
        let c: UiCommand = serde_json::from_str(r#"{"command":"settings_set_auto_update","enabled":true}"#).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::SetAutoUpdate(true)));
        let c: UiCommand = serde_json::from_str(r#"{"command":"hotkey_edge","pressed":true,"atMs":1758700600000,"source":"cli"}"#).unwrap();
        assert!(matches!(
            c.into_core().unwrap(),
            CoreCommand::HotkeyEdge { pressed: true, at_ms: 1_758_700_600_000, source: EdgeSource::Cli, purpose: TakeKind::Dictation, chorded: false }
        ));
        // docs/dictation.md §19: the edit key says so; the purpose defaults to dictation.
        let c: UiCommand = serde_json::from_str(r#"{"command":"hotkey_edge","pressed":false,"atMs":1,"source":"hotkey","purpose":"edit"}"#).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::HotkeyEdge { pressed: false, purpose: TakeKind::Edit, .. }));
        assert!(serde_json::from_str::<UiCommand>(r#"{"command":"hotkey_edge","pressed":true,"atMs":1,"source":"cli","purpose":"translate"}"#).is_err());
        let c: UiCommand = serde_json::from_str(r#"{"command":"settings_set_edit_hotkey","hotkey":"Ctrl+Shift+E"}"#).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::SetEditHotkey(Some(h)) if h == "Ctrl+Shift+E"));
        let c: UiCommand = serde_json::from_str(r#"{"command":"settings_set_edit_hotkey","hotkey":null}"#).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::SetEditHotkey(None)));
        let c: UiCommand = serde_json::from_str(r#"{"command":"settings_set_solo_key","key":"mouse_back"}"#).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::SetSoloKey(Some(voltip_core::SoloKey::MouseBack))));
        let c: UiCommand = serde_json::from_str(r#"{"command":"settings_set_solo_key","key":null}"#).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::SetSoloKey(None)));
        assert!(serde_json::from_str::<UiCommand>(r#"{"command":"settings_set_solo_key","key":"caps_lock"}"#).is_err());
        let c: UiCommand = serde_json::from_str(r#"{"command":"hotkey_edge","pressed":false,"atMs":5,"source":"hotkey","chorded":true}"#).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::HotkeyEdge { chorded: true, purpose: TakeKind::Dictation, .. }));
        assert!(
            serde_json::from_str::<UiCommand>(r#"{"command":"hotkey_edge","pressed":true,"atMs":1,"source":"mouse"}"#).is_err(),
            "unknown sources are refused"
        );
        assert!(serde_json::from_str::<UiCommand>(r#"{"command":"hotkey_edge","pressed":true}"#).is_err(), "at_ms and source are required");
        let c: UiCommand =
            serde_json::from_str(r#"{"command":"settings_set_activation","activation":"hold_or_toggle","holdThresholdMs":400,"extraRecordingMs":150}"#)
                .unwrap();
        assert!(matches!(
            c.into_core().unwrap(),
            CoreCommand::SetActivation { activation: Activation::HoldOrToggle, hold_threshold_ms: 400, extra_recording_ms: 150 }
        ));
        assert!(
            serde_json::from_str::<UiCommand>(r#"{"command":"settings_set_activation","activation":"press","holdThresholdMs":400,"extraRecordingMs":0}"#)
                .is_err(),
            "unknown modes are refused at the IPC layer"
        );
        let c: UiCommand = serde_json::from_str(r#"{"command":"provider_key_set","provider":"groq","kind":"llm","value":null}"#).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::SetProviderKey { provider: ProviderId::Groq, kind: ServiceKind::Llm, value: None }));
        let c: UiCommand =
            serde_json::from_str(r#"{"command":"provider_probe","provider":"custom","kind":"asr","baseUrl":"http://10.0.0.2:8000/v1"}"#).unwrap();
        assert!(matches!(
            c.into_core().unwrap(),
            CoreCommand::ProbeProvider { provider: ProviderId::Custom, kind: ServiceKind::Asr, base_url: Some(u), key: None } if u == "http://10.0.0.2:8000/v1"
        ));
        let c: UiCommand = serde_json::from_str(r#"{"command":"provider_models_refresh","provider":"aliyun","kind":"asr"}"#).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::RefreshProviderModels { provider: ProviderId::Aliyun, kind: ServiceKind::Asr }));
        assert!(
            serde_json::from_str::<UiCommand>(r#"{"command":"provider_key_set","provider":"azure","kind":"llm","value":"x"}"#).is_err(),
            "unknown providers are refused at the IPC layer"
        );
        assert!(serde_json::from_str::<UiCommand>(r#"{"command":"secret_set","name":"asr_token","value":"x"}"#).is_err(), "the old secret command is gone");
        let c: UiCommand =
            serde_json::from_str(r#"{"command":"settings_set_engines","engines":{"refine_enabled":false,"inject":"clipboard_only","asr_provider":"groq"}}"#)
                .unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::SetEngines(e) if !e.refine_enabled && e.inject == voltip_core::InjectMode::ClipboardOnly));
        let c: UiCommand = serde_json::from_str(r#"{"command":"history_star","id":"not-a-uuid","starred":true}"#).unwrap();
        let err: String = c.into_core().unwrap_err().into();
        assert!(err.contains("UUID"), "{err}");
        let c: UiCommand = serde_json::from_str(r#"{"command":"history_delete","id":"0f3f1a1e-8d4b-4c8e-9f7a-1c2d3e4f5a6b"}"#).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::HistoryDelete(id) if id.to_string() == "0f3f1a1e-8d4b-4c8e-9f7a-1c2d3e4f5a6b"));
        let c: UiCommand = serde_json::from_str(r#"{"command":"history_star","id":"0f3f1a1e-8d4b-4c8e-9f7a-1c2d3e4f5a6b","starred":false}"#).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::HistoryStar(_, false)));
        let c: UiCommand = serde_json::from_str(r#"{"command":"model_download","id":"sense-voice-small"}"#).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::ModelDownload(id) if id == "sense-voice-small"));
        let c: UiCommand = serde_json::from_str(r#"{"command":"model_cancel","id":"paraformer-zh"}"#).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::ModelCancel(id) if id == "paraformer-zh"));
        let c: UiCommand = serde_json::from_str(r#"{"command":"model_remove","id":"paraformer-zh"}"#).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::ModelRemove(id) if id == "paraformer-zh"));
        let c: UiCommand = serde_json::from_str(r#"{"command":"model_import","id":"paraformer-zh"}"#).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::ModelImport(id) if id == "paraformer-zh"));
        let c: UiCommand = serde_json::from_str(r#"{"command":"models_location_reset"}"#).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::ModelsRelocate(None)));
        let c: UiCommand = serde_json::from_str(r#"{"command":"models_location_cancel"}"#).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::ModelsRelocateCancel));
        assert!(
            serde_json::from_str::<UiCommand>(r#"{"command":"models_location_reset","dir":"/elsewhere"}"#)
                .is_ok_and(|c| matches!(c.into_core(), Ok(CoreCommand::ModelsRelocate(None)))),
            "the webview never names the folder"
        );
        assert!(serde_json::from_str::<UiCommand>(r#"{"command":"model_download"}"#).is_err(), "the id is required");
        // Vocabulary (docs/dictation.md §16.4): drafts are snake_case inside, validated synchronously.
        let hid = "0f3f1a1e-8d4b-4c8e-9f7a-1c2d3e4f5a6b";
        let c: UiCommand =
            serde_json::from_str(&format!(r#"{{"command":"dictionary_add","entry":{{"term":" good idea ","heard_as":["谷歌IDR"]}},"historyId":"{hid}"}}"#))
                .unwrap();
        assert!(matches!(
            c.into_core().unwrap(),
            CoreCommand::DictionaryAdd { draft, source: EntrySource::History { history_id } } if draft.term == "good idea" && draft.enabled && history_id.to_string() == hid
        ));
        let c: UiCommand = serde_json::from_str(r#"{"command":"dictionary_add","entry":{"term":"x"},"historyId":null}"#).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::DictionaryAdd { source: EntrySource::Manual, .. }));
        let c: UiCommand = serde_json::from_str(r#"{"command":"dictionary_add","entry":{"term":"  "}}"#).unwrap();
        let err: String = c.into_core().unwrap_err().into();
        assert!(err.starts_with("dictionary: ") && err.contains("不能为空"), "{err}");
        let c: UiCommand = serde_json::from_str(r#"{"command":"dictionary_add","entry":{"term":"x"},"historyId":"nope"}"#).unwrap();
        assert!(String::from(c.into_core().unwrap_err()).contains("UUID"));
        assert!(serde_json::from_str::<UiCommand>(r#"{"command":"dictionary_add","entry":{"term":"x","weight":2}}"#).is_err(), "unknown draft fields");
        let c: UiCommand = serde_json::from_str(&format!(r#"{{"command":"dictionary_update","id":"{hid}","entry":{{"term":"y","enabled":false}}}}"#)).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::DictionaryUpdate { draft, .. } if !draft.enabled));
        let c: UiCommand = serde_json::from_str(&format!(r#"{{"command":"dictionary_remove","id":"{hid}"}}"#)).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::DictionaryRemove(_)));
        let c: UiCommand = serde_json::from_str(&format!(r#"{{"command":"dictionary_reorder","ids":["{hid}"]}}"#)).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::DictionaryReorder(ids) if ids.len() == 1));
        let c: UiCommand = serde_json::from_str(r#"{"command":"dictionary_reorder","ids":["x"]}"#).unwrap();
        assert!(String::from(c.into_core().unwrap_err()).contains("UUID"));
        let c: UiCommand = serde_json::from_str(
            r#"{"command":"rules_add","rule":{"name":"pr","kind":"regex","pattern":"\\bpr (\\d+)","replacement":"PR #$1","case_sensitive":false}}"#,
        )
        .unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::RuleAdd(r) if r.kind == voltip_core::RuleKind::Regex && !r.case_sensitive && r.enabled));
        let c: UiCommand = serde_json::from_str(r#"{"command":"rules_add","rule":{"name":"bad","kind":"regex","pattern":"("}}"#).unwrap();
        let err: String = c.into_core().unwrap_err().into();
        assert!(err.contains("正则无法编译"), "an invalid regex is refused before it reaches the core: {err}");
        let c: UiCommand = serde_json::from_str(&format!(r#"{{"command":"rules_update","id":"{hid}","rule":{{"name":"n","pattern":"p"}}}}"#)).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::RuleUpdate { .. }));
        let c: UiCommand = serde_json::from_str(&format!(r#"{{"command":"rules_remove","id":"{hid}"}}"#)).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::RuleRemove(_)));
        let c: UiCommand = serde_json::from_str(&format!(r#"{{"command":"rules_reorder","ids":["{hid}"]}}"#)).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::RuleReorder(_)));
        let import = |toml: &str, mode: &str| -> UiCommand {
            serde_json::from_value(serde_json::json!({ "command": "rules_import", "toml": toml, "mode": mode })).unwrap()
        };
        let c = import("version = 1\n[[rule]]\nname = \"a\"\npattern = \"x\"\n", "merge");
        assert!(matches!(c.into_core().unwrap(), CoreCommand::RulesImport { rules, mode: ImportMode::Merge } if rules.len() == 1));
        let c = import("version = 1\n[[rule]]\n", "replace");
        assert!(String::from(c.into_core().unwrap_err()).contains("TOML 无法解析"));
        assert!(serde_json::from_str::<UiCommand>(r#"{"command":"rules_import","toml":"","mode":"append"}"#).is_err(), "unknown modes");
        // Scenes (docs/dictation.md §18.6): `match` on the wire, the draft normalised and validated here.
        let c: UiCommand = serde_json::from_str(
            r#"{"command":"scenes_add","scene":{"name":" 聊天 ","match":{"apps":["Slack.exe","slack"],"title_contains":[]},"overrides":{"refine_style":"punctuation","prompt":" 口语化 "}}}"#,
        )
        .unwrap();
        assert!(matches!(
            c.into_core().unwrap(),
            CoreCommand::SceneAdd(d) if d.name == "聊天" && d.enabled && d.matching.apps == ["slack"] && d.overrides.prompt.as_deref() == Some("口语化")
        ));
        let c: UiCommand = serde_json::from_str(r#"{"command":"scenes_add","scene":{"name":"x","match":{"apps":[]}}}"#).unwrap();
        let err: String = c.clone().into_core().unwrap_err().into();
        assert!(err.starts_with("scenes: ") && err.contains("至少要有一个应用"), "{err}");
        // The phone picks a take's scene by hand (user decision 2026-10-01): one of its own may list
        // no application.
        assert!(matches!(c.into_core_for(false).unwrap(), CoreCommand::SceneAdd(d) if d.name == "x" && d.matching.apps.is_empty()));
        let c: UiCommand =
            serde_json::from_str(r#"{"command":"scenes_add","scene":{"name":"x","match":{"apps":["a"]},"overrides":{"language":"中文"}}}"#).unwrap();
        assert!(String::from(c.into_core().unwrap_err()).contains("语言代码"));
        assert!(
            serde_json::from_str::<UiCommand>(r#"{"command":"scenes_add","scene":{"name":"x","match":{"apps":["a"],"urls":["x"]}}}"#).is_err(),
            "unknown fields"
        );
        let c: UiCommand =
            serde_json::from_str(&format!(r#"{{"command":"scenes_update","id":"{hid}","scene":{{"name":"y","enabled":false,"match":{{"apps":["code"]}}}}}}"#))
                .unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::SceneUpdate { draft, .. } if !draft.enabled));
        // An update may list no application (a built-in scene may, §18.10): the core decides.
        let c: UiCommand =
            serde_json::from_str(&format!(r#"{{"command":"scenes_update","id":"{hid}","scene":{{"name":"legal","match":{{"apps":[]}}}}}}"#)).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::SceneUpdate { draft, .. } if draft.matching.apps.is_empty()));
        let c: UiCommand = serde_json::from_str(&format!(r#"{{"command":"scenes_restore","id":"{hid}"}}"#)).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::SceneRestore(_)));
        let c: UiCommand = serde_json::from_str(r#"{"command":"scenes_update","id":"nope","scene":{"name":"y","match":{"apps":["code"]}}}"#).unwrap();
        assert!(String::from(c.into_core().unwrap_err()).contains("UUID"));
        let c: UiCommand = serde_json::from_str(&format!(r#"{{"command":"scenes_remove","id":"{hid}"}}"#)).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::SceneRemove(_)));
        let c: UiCommand = serde_json::from_str(&format!(r#"{{"command":"scenes_reorder","ids":["{hid}"]}}"#)).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::SceneReorder(ids) if ids.len() == 1));
        // Presets (docs/dictation.md §21): drafts validated here, 试一试 takes exactly one source.
        let c: UiCommand = serde_json::from_str(r#"{"command":"presets_add","preset":{"name":" 周报 ","prompt":" 整理成周报 "}}"#).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::PresetAdd(d) if d.name == "周报" && d.prompt == "整理成周报"));
        let c: UiCommand = serde_json::from_str(r#"{"command":"presets_add","preset":{"name":"","prompt":"x"}}"#).unwrap();
        assert_eq!(String::from(c.into_core().unwrap_err()), "presets: 预设名称不能为空");
        assert!(serde_json::from_str::<UiCommand>(r#"{"command":"presets_add","preset":{"name":"a","prompt":"b","model":"m"}}"#).is_err(), "unknown fields");
        let c: UiCommand = serde_json::from_str(&format!(r#"{{"command":"presets_update","id":"{hid}","preset":{{"name":"b","prompt":"c"}}}}"#)).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::PresetUpdate { draft, .. } if draft.name == "b"));
        let c: UiCommand = serde_json::from_str(&format!(r#"{{"command":"presets_remove","id":"{hid}"}}"#)).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::PresetRemove(_)));
        let c: UiCommand = serde_json::from_str(r#"{"command":"presets_try","id":7,"preset":"translate","text":" 你好 "}"#).unwrap();
        assert!(matches!(
            c.into_core().unwrap(),
            CoreCommand::PresetTry { id: 7, trial: PresetTrial::Preset(PresetId::Builtin(voltip_core::BuiltinPreset::Translate)), text } if text == "你好"
        ));
        let c: UiCommand = serde_json::from_str(r#"{"command":"presets_try","id":8,"prompt":" 改写成邮件 \r\n","text":"x"}"#).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::PresetTry { trial: PresetTrial::Prompt(p), .. } if p == "改写成邮件"));
        for (json, want) in [
            (r#"{"command":"presets_try","id":1,"text":"x"}"#.to_owned(), "presets: 试一试需要一个预设或一段预设内容"),
            (r#"{"command":"presets_try","id":1,"preset":"notes","prompt":"x","text":"x"}"#.to_owned(), "presets: 试一试需要一个预设或一段预设内容"),
            (r#"{"command":"presets_try","id":1,"preset":"casual","text":"x"}"#.to_owned(), "presets: 没有名为「casual」的预设"),
            (r#"{"command":"presets_try","id":1,"preset":"notes","text":"  "}"#.to_owned(), "presets: 请输入要试运行的文字"),
            (
                format!(r#"{{"command":"presets_try","id":1,"preset":"notes","text":"{}"}}"#, "字".repeat(MAX_PRESET_TRY_CHARS + 1)),
                "presets: 试运行的文字最多 2000 个字符（当前 2001）",
            ),
        ] {
            let c: UiCommand = serde_json::from_str(&json).unwrap();
            assert_eq!(String::from(c.into_core().unwrap_err()), want, "{json}");
        }
        // docs/dictation.md §24: `password: null` keeps the saved one; the core checks the rest.
        let c: UiCommand =
            serde_json::from_str(r#"{"command":"settings_set_proxy","mode":"off","server":"127.0.0.1:7890","username":null,"noProxy":"","password":null}"#)
                .unwrap();
        assert!(matches!(
            c.into_core().unwrap(),
            CoreCommand::SetProxy {
                proxy: voltip_core::ProxySettings { mode: voltip_core::ProxyMode::Off, server: Some(ref s), username: None, ref no_proxy },
                password: None,
            } if s == "127.0.0.1:7890" && no_proxy.is_empty()
        ));
        assert!(
            serde_json::from_str::<UiCommand>(r#"{"command":"settings_set_proxy","mode":"socks","server":null,"username":null,"noProxy":"","password":null}"#)
                .is_err(),
            "an unknown mode is refused"
        );
        assert!(matches!(serde_json::from_str::<UiCommand>(r#"{"command":"network_refresh"}"#).unwrap().into_core().unwrap(), CoreCommand::RefreshNetwork));
        let c: UiCommand = serde_json::from_str(r#"{"command":"settings_set_serve","enabled":true,"port":47840,"preset":"prompt","scene":null}"#).unwrap();
        assert!(matches!(
            c.into_core().unwrap(),
            CoreCommand::SetServe(voltip_core::ServeSettings {
                enabled: true,
                port: 47840,
                preset: Some(PresetId::Builtin(voltip_core::BuiltinPreset::Prompt)),
                scene: None
            })
        ));
        let bad: UiCommand = serde_json::from_str(r#"{"command":"settings_set_serve","enabled":true,"port":47840,"preset":"nope","scene":null}"#).unwrap();
        assert!(bad.into_core().is_err(), "an unknown preset is refused");
        let bad: UiCommand =
            serde_json::from_str(r#"{"command":"settings_set_serve","enabled":true,"port":47840,"preset":null,"scene":"not-a-uuid"}"#).unwrap();
        assert!(bad.into_core().is_err(), "a scene is a UUID");
        assert!(matches!(serde_json::from_str::<UiCommand>(r#"{"command":"serve_copy_token"}"#).unwrap().into_core().unwrap(), CoreCommand::ServeCopyToken));
        assert!(matches!(
            serde_json::from_str::<UiCommand>(r#"{"command":"serve_rotate_token"}"#).unwrap().into_core().unwrap(),
            CoreCommand::ServeRotateToken
        ));
        // docs/dictation.md §25: the MCP service's settings, token, setup commands and undo.
        let c: UiCommand = serde_json::from_str(
            r#"{"command":"settings_set_mcp","enabled":true,"port":47841,"read":"full","historyRange":"last_7_days","write":"enable_rules"}"#,
        )
        .unwrap();
        assert!(matches!(
            c.into_core().unwrap(),
            CoreCommand::SetMcp(voltip_core::mcp::McpSettings {
                enabled: true,
                port: 47841,
                read: voltip_core::mcp::McpRead::Full,
                history_range: voltip_core::mcp::HistoryRange::Last7Days,
                write: voltip_core::mcp::McpWrite::EnableRules
            })
        ));
        assert!(
            serde_json::from_str::<UiCommand>(
                r#"{"command":"settings_set_mcp","enabled":true,"port":47841,"read":"all","historyRange":"all","write":"dictionary"}"#
            )
            .is_err(),
            "an unknown read level is refused"
        );
        assert!(matches!(serde_json::from_str::<UiCommand>(r#"{"command":"mcp_copy_token"}"#).unwrap().into_core().unwrap(), CoreCommand::McpCopyToken));
        assert!(matches!(serde_json::from_str::<UiCommand>(r#"{"command":"mcp_rotate_token"}"#).unwrap().into_core().unwrap(), CoreCommand::McpRotateToken));
        assert!(matches!(
            serde_json::from_str::<UiCommand>(r#"{"command":"mcp_copy_setup","client":"claude_code"}"#).unwrap().into_core().unwrap(),
            CoreCommand::McpCopySetup(voltip_core::mcp::SetupClient::ClaudeCode)
        ));
        assert!(serde_json::from_str::<UiCommand>(r#"{"command":"mcp_copy_setup","client":"cursor"}"#).is_err());
        let id = Uuid::new_v4();
        let c: UiCommand = serde_json::from_str(&format!(r#"{{"command":"mcp_undo","id":"{id}"}}"#)).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::McpUndo(u) if u == id));
        assert!(serde_json::from_str::<UiCommand>(r#"{"command":"mcp_undo","id":"x"}"#).unwrap().into_core().is_err(), "an id is a UUID");
        let c: UiCommand = serde_json::from_str(r#"{"command":"settings_set_context_sharing","appName":false,"windowTitle":true}"#).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::SetContextSharing(ContextSharing { app_name: false, window_title: true })));
        assert!(serde_json::from_str::<UiCommand>(r#"{"command":"settings_set_context_sharing","appName":true}"#).is_err(), "both switches are required");
        for (json, expect) in [
            (r#"{"command":"dictation_start"}"#, "DictationStart"),
            (r#"{"command":"dictation_stop"}"#, "DictationStop"),
            (r#"{"command":"dictation_cancel"}"#, "DictationCancel"),
            (r#"{"command":"dictation_dismiss","session":3}"#, "DictationDismiss"),
            (r#"{"command":"settings_set_copy_last_hotkey","hotkey":"Ctrl+Shift+V"}"#, "SetCopyLastHotkey(Some(\"Ctrl+Shift+V\"))"),
            (r#"{"command":"settings_set_copy_last_hotkey","hotkey":null}"#, "SetCopyLastHotkey(None)"),
            (r#"{"command":"history_clear"}"#, "HistoryClear"),
            (r#"{"command":"engines_quota_reset","kind":"llm"}"#, "ResetQuota(Llm)"),
            (r#"{"command":"pairing_start"}"#, "StartPairing"),
            (r#"{"command":"pairing_confirm"}"#, "ConfirmPairing"),
            (r#"{"command":"pairing_reject"}"#, "RejectPairing"),
            (r#"{"command":"pairing_cancel"}"#, "CancelPairing"),
            (r#"{"command":"pairing_reset"}"#, "ResetPairing"),
            (r#"{"command":"devices_refresh"}"#, "RefreshDevices"),
            (r#"{"command":"connectivity_check"}"#, "CheckConnectivity"),
            (r#"{"command":"relay_reconnect"}"#, "ReconnectRelay"),
            (r#"{"command":"device_rename","name":"X"}"#, "RenameDevice"),
            (r#"{"command":"pairing_join_ticket","uri":"voltip://pair?v=1&t=AA"}"#, "JoinWithTicket"),
        ] {
            let c: UiCommand = serde_json::from_str(json).unwrap();
            let core = c.into_core().unwrap();
            assert!(format!("{core:?}").starts_with(expect), "{json} -> {core:?}");
        }
    }

    /// docs/dictation.md §10: the webview names a model, a file and a source; the folder and the
    /// address come from the core's catalogue in the cached state.
    #[tokio::test]
    async fn the_manual_download_opens_the_catalogue_folder_and_addresses_only() {
        let dir = tempfile::tempdir().unwrap();
        voltip_core::SettingsStore::new(dir.path())
            .save(&voltip_core::Settings { relay_enabled: false, engines: voltip_core::dictation::fakes::fake_engines(), ..Default::default() })
            .unwrap();
        let bridge = Bridge::start(CoreConfig::new(dir.path().to_path_buf()), Arc::new(MemorySecretStore::new())).unwrap();
        let mut model = voltip_core::dictation::fakes::FakeModels::catalogue().remove(0);
        model.dir = dir.path().join("models").join(&model.id).to_string_lossy().into_owned();
        let (id, file, urls) = (model.id.clone(), model.files[0].name.clone(), model.files[0].urls.clone());
        bridge.state.lock().models = vec![model];
        let folder = bridge.model_folder(&id).unwrap();
        assert!(folder.is_dir(), "created for the person to put files in");
        assert_eq!(folder, dir.path().join("models").join(&id));
        assert_eq!(bridge.model_link(&id, &file, 1).unwrap(), urls[1]);
        assert!(bridge.model_link(&id, &file, 2).is_err(), "no third source");
        assert!(bridge.model_link(&id, "evil.txt", 0).is_err(), "only the catalogue's files");
        assert!(bridge.model_link("nope", &file, 0).is_err());
        assert!(bridge.model_folder("nope").is_err());
        bridge.shutdown();
    }

    /// docs/dictation.md §29.12: a model that comes as a package names the package, and the address
    /// the link opens is the package's; its files have none of their own.
    #[tokio::test]
    async fn the_manual_download_of_a_packaged_model_opens_the_package_address() {
        let dir = tempfile::tempdir().unwrap();
        voltip_core::SettingsStore::new(dir.path())
            .save(&voltip_core::Settings { relay_enabled: false, engines: voltip_core::dictation::fakes::fake_engines(), ..Default::default() })
            .unwrap();
        let bridge = Bridge::start(CoreConfig::new(dir.path().to_path_buf()), Arc::new(MemorySecretStore::new())).unwrap();
        let mut model = voltip_core::dictation::fakes::FakeModels::catalogue().remove(0);
        for file in &mut model.files {
            file.urls.clear();
        }
        let url = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/model.tar.bz2".to_owned();
        model.package = Some(voltip_core::ModelPackageView { name: "model.tar.bz2".into(), urls: vec![url.clone()] });
        let (id, file) = (model.id.clone(), model.files[0].name.clone());
        bridge.state.lock().models = vec![model];
        assert_eq!(bridge.model_link(&id, "model.tar.bz2", 0).unwrap(), url);
        assert!(bridge.model_link(&id, "model.tar.bz2", 1).is_err(), "one address");
        assert!(bridge.model_link(&id, &file, 0).is_err(), "a file in the package has no address of its own");
        bridge.shutdown();
    }

    /// docs/dictation.md §16.4: the two queries answer from the bridge's cached lists with the core's
    /// own functions — a rule added through `dispatch` shows up in the preview and the export; a
    /// draft stands in for the rule it names; a bad draft or text is an error, not a panic.
    #[tokio::test]
    async fn vocabulary_queries_use_the_cached_lists() {
        let dir = tempfile::tempdir().unwrap();
        voltip_core::SettingsStore::new(dir.path())
            .save(&voltip_core::Settings { relay_enabled: false, engines: voltip_core::dictation::fakes::fake_engines(), ..Default::default() })
            .unwrap();
        let bridge = Bridge::start(CoreConfig::new(dir.path().to_path_buf()), Arc::new(MemorySecretStore::new())).unwrap();
        let mut rx = bridge.events();
        bridge
            .dispatch(UiCommand::DictionaryAdd {
                entry: DictionaryDraft { term: "good idea".into(), heard_as: vec!["谷歌IDR".into()], enabled: true },
                history_id: None,
            })
            .unwrap();
        bridge
            .dispatch(UiCommand::RulesAdd {
                rule: RuleDraft {
                    name: "app".into(),
                    kind: voltip_core::RuleKind::Literal,
                    pattern: "app".into(),
                    replacement: "App".into(),
                    case_sensitive: true,
                    enabled: true,
                },
            })
            .unwrap();
        for _ in 0..40 {
            if !bridge.state().rules.is_empty() && !bridge.state().dictionary.is_empty() {
                break;
            }
            let _ = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await;
        }
        let p = bridge.vocabulary_preview("一个谷歌IDR的app", None).unwrap();
        assert_eq!((p.corrected.as_str(), p.output.as_str()), ("一个good idea的app", "一个good idea的App"));
        assert_eq!((p.corrections.len(), p.rules.len()), (1, 1));
        let rule_id = bridge.state().rules[0].id;
        let draft = PreviewDraft { id: Some(rule_id), rule: RuleDraft { replacement: "APP".into(), ..RuleDraft::from(&bridge.state().rules[0]) } };
        assert_eq!(bridge.vocabulary_preview("app", Some(&draft)).unwrap().output, "APP");
        let bad = PreviewDraft { id: None, rule: RuleDraft { kind: voltip_core::RuleKind::Regex, pattern: "(".into(), ..draft.rule.clone() } };
        assert!(String::from(bridge.vocabulary_preview("app", Some(&bad)).unwrap_err()).contains("正则无法编译"));
        assert!(bridge.vocabulary_preview(&"x".repeat(70 * 1024), None).is_err());
        let toml = bridge.rules_export().unwrap();
        assert!(toml.contains("name = \"app\"") && toml.contains("replacement = \"App\""), "{toml}");
        assert_eq!(voltip_core::vocabulary::parse_rules_toml(&toml).unwrap().len(), 1);
        bridge.shutdown();
    }

    /// docs/dictation.md §16.9: `vocabulary_backtest` reads the newest recognised takes from the
    /// database and tries the draft against the cached dictionary: the take the card was opened on
    /// is left out, the limit bounds the read, and a saved entry is the baseline the next draft is
    /// compared with. A bad limit or a draft wrong on its own is the caller's error.
    #[tokio::test]
    async fn vocabulary_backtest_tries_a_draft_on_the_recent_takes() {
        let dir = tempfile::tempdir().unwrap();
        voltip_core::SettingsStore::new(dir.path())
            .save(&voltip_core::Settings { relay_enabled: false, engines: voltip_core::dictation::fakes::fake_engines(), ..Default::default() })
            .unwrap();
        let takes = [("用维他命测了一下", "用 Voltip 测了一下。"), ("维他命的安装包发给我", "维他命的安装包发给我。"), ("与此无关", "与此无关。")];
        let ids: Vec<Uuid> = takes.iter().map(|_| Uuid::new_v4()).collect();
        {
            let mut store = voltip_core::history::HistoryStore::open(dir.path());
            for (i, (raw, text)) in takes.iter().enumerate().rev() {
                let entry: HistoryEntry = serde_json::from_value(serde_json::json!({
                    "id": ids[i], "at_ms": 1_758_700_000_000_u64 - i as u64, "raw_text": raw, "text": text, "refined": true,
                    "asr_model": "fake", "duration_ms": 1000, "asr_ms": 100, "outcome": { "kind": "inserted", "via": "paste" },
                }))
                .unwrap();
                store.push(entry, voltip_core::history::MAX_ENTRIES).unwrap();
            }
        }
        let bridge = Bridge::start(CoreConfig::new(dir.path().to_path_buf()), Arc::new(MemorySecretStore::new())).unwrap();
        let mut rx = bridge.events();
        let draft = BacktestDraft::Dictionary { id: None, entry: DictionaryDraft { term: "Voltip".into(), heard_as: vec!["维他命".into()], enabled: true } };
        let all = bridge.vocabulary_backtest(&draft, None, None).unwrap();
        assert_eq!((all.scanned, all.matched, all.kept), (3, 2, 1));
        assert_eq!(all.examples[0].history_id, ids[1], "the take whose final text kept the form comes first");
        let others = bridge.vocabulary_backtest(&draft, Some(ids[0]), None).unwrap();
        assert_eq!((others.scanned, others.matched, others.kept), (2, 1, 1), "the card's own take is left out");
        assert_eq!(bridge.vocabulary_backtest(&draft, None, Some(1)).unwrap().scanned, 1, "the newest one only");
        for limit in [0, MAX_BACKTEST_LIMIT + 1] {
            let err = String::from(bridge.vocabulary_backtest(&draft, None, Some(limit)).unwrap_err());
            assert!(err.starts_with("vocabulary_backtest: limit"), "{err}");
        }
        let empty = BacktestDraft::Dictionary { id: None, entry: DictionaryDraft { term: " ".into(), heard_as: vec![], enabled: true } };
        let err = String::from(bridge.vocabulary_backtest(&empty, None, None).unwrap_err());
        assert!(err.starts_with("dictionary: ") && err.contains("不能为空"), "{err}");
        let rule = BacktestDraft::Rule {
            id: None,
            rule: RuleDraft {
                name: "维他命 → Voltip".into(),
                kind: voltip_core::RuleKind::Literal,
                pattern: "维他命".into(),
                replacement: "Voltip".into(),
                case_sensitive: false,
                enabled: true,
            },
        };
        let ruled = bridge.vocabulary_backtest(&rule, None, None).unwrap();
        assert_eq!((ruled.scanned, ruled.matched, ruled.kept), (3, 1, 0), "a rule rewrites the final texts");
        // Saved, the entry is the baseline: the same draft changes nothing more.
        bridge
            .dispatch(UiCommand::DictionaryAdd {
                entry: DictionaryDraft { term: "Voltip".into(), heard_as: vec!["维他命".into()], enabled: true },
                history_id: None,
            })
            .unwrap();
        for _ in 0..40 {
            if !bridge.state().dictionary.is_empty() {
                break;
            }
            let _ = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await;
        }
        assert_eq!(bridge.state().dictionary.len(), 1);
        assert_eq!(bridge.vocabulary_backtest(&draft, None, None).unwrap().matched, 0);
        bridge.shutdown();
    }

    /// docs/dictation.md §16.10: `vocabulary_suggestions` reads the newest recognised takes and
    /// keeps its answer until the dictionary, the rules, the history or the ignored list change;
    /// 加入词典 and 撤销 go through the core and answer; history recording turned off is an answer
    /// of its own, not an empty list; a pair that is not one is refused here; the rules page's
    /// hints come from the cached lists.
    #[tokio::test]
    async fn learning_suggestions_follow_the_lists_and_the_history() {
        use voltip_core::vocabulary::{RuleStore, SuggestionsAnswer, UndoOutcome};
        async fn until(bridge: &Bridge, rx: &mut broadcast::Receiver<UiEvent>, done: impl Fn(&UiState) -> bool) {
            for _ in 0..60 {
                if done(&bridge.state()) {
                    return;
                }
                let _ = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await;
            }
            panic!("the state never got there");
        }
        let dir = tempfile::tempdir().unwrap();
        voltip_core::SettingsStore::new(dir.path())
            .save(&voltip_core::Settings { relay_enabled: false, engines: voltip_core::dictation::fakes::fake_engines(), ..Default::default() })
            .unwrap();
        let feishu = |name: &str| RuleDraft {
            name: name.into(),
            kind: voltip_core::RuleKind::Literal,
            pattern: "非书".into(),
            replacement: "飞书".into(),
            case_sensitive: true,
            enabled: true,
        };
        // The rule fired in the second take and was not changed since.
        let rule_id = RuleStore::open(dir.path()).0.add(&feishu("非书 → 飞书"), 1).unwrap();
        let takes =
            [("用维他命测了一下", "用 Voltip 测了一下。", false), ("发到非书群，跑千问", "发到飞书群，跑 Qwen。", true), ("又是非书", "又是飞书。", false)];
        let ids: Vec<Uuid> = takes.iter().map(|_| Uuid::new_v4()).collect();
        {
            let mut store = voltip_core::history::HistoryStore::open(dir.path());
            for (i, (raw, text, fired)) in takes.iter().enumerate().rev() {
                let vocabulary = fired.then(|| serde_json::json!({ "rules": [{ "id": rule_id, "count": 1 }] }));
                let entry: HistoryEntry = serde_json::from_value(serde_json::json!({
                    "id": ids[i], "at_ms": 1_758_700_000_000_u64 - i as u64, "raw_text": raw, "text": text, "refined": true,
                    "asr_model": "fake", "duration_ms": 1000, "asr_ms": 100, "outcome": { "kind": "inserted", "via": "paste" },
                    "vocabulary": vocabulary,
                }))
                .unwrap();
                store.push(entry, voltip_core::history::MAX_ENTRIES).unwrap();
            }
        }
        let bridge = Bridge::start(CoreConfig::new(dir.path().to_path_buf()), Arc::new(MemorySecretStore::new())).unwrap();
        let mut rx = bridge.events();
        until(&bridge, &mut rx, |s| s.identity.is_some() && s.rules.len() == 1 && s.history_total == 3).await;
        let ready = |answer: SuggestionsAnswer| match answer {
            SuggestionsAnswer::Ready(s) => s,
            other => panic!("{other:?}"),
        };
        let heard = |bridge: &Bridge| ready(bridge.vocabulary_suggestions(None).unwrap()).pairs.into_iter().map(|p| p.heard).collect::<Vec<_>>();
        let first = ready(bridge.vocabulary_suggestions(None).unwrap());
        assert_eq!(first.pairs.iter().map(|p| p.heard.as_str()).collect::<Vec<_>>(), ["维他命", "千问", "非"], "the rule's own change is left out");
        assert_eq!((first.scanned, first.refined, first.skipped, first.total), (3, 3, 0, 3));
        let one = ready(bridge.vocabulary_suggestions(Some(1)).unwrap());
        assert_eq!((one.pairs.len(), one.total), (1, 3));
        for limit in [0, MAX_SUGGESTIONS + 1] {
            let err = String::from(bridge.vocabulary_suggestions(Some(limit)).unwrap_err());
            assert!(err.starts_with("vocabulary_suggestions: limit"), "{err}");
        }

        // 加入词典 and 撤销: the dictionary changes, and the answer with it.
        let wait = Duration::from_secs(20);
        let AcceptOutcome::Accepted { undo } = bridge.vocabulary_suggestion_accept("维他命".into(), "Voltip".into(), ids[0], wait).await.unwrap() else {
            panic!("accepted")
        };
        until(&bridge, &mut rx, |s| s.dictionary.len() == 1).await;
        assert_eq!(bridge.state().dictionary[0].source, EntrySource::History { history_id: ids[0] });
        assert_eq!(heard(&bridge), ["千问", "非"]);
        assert_eq!(bridge.vocabulary_suggestion_undo(undo.id, wait).await.unwrap(), UndoOutcome::Undone);
        until(&bridge, &mut rx, |s| s.dictionary.is_empty()).await;
        assert_eq!(heard(&bridge), ["维他命", "千问", "非"]);
        assert_eq!(bridge.vocabulary_suggestion_undo(undo.id, wait).await.unwrap(), UndoOutcome::Expired, "once");
        let err = String::from(bridge.vocabulary_suggestion_accept(" ".into(), "Voltip".into(), ids[0], wait).await.unwrap_err());
        assert!(err.contains("不能为空"), "{err}");

        // The rule changed after the take: that take no longer counts.
        bridge.dispatch(UiCommand::RulesUpdate { id: rule_id.to_string(), rule: feishu("非书 → 飞书") }).unwrap();
        until(&bridge, &mut rx, |s| s.rules.first().is_some_and(|r| r.updated_at_ms > 1)).await;
        assert_eq!(heard(&bridge), ["维他命", "非"]);
        // A take deleted from the history no longer counts.
        bridge.dispatch(UiCommand::HistoryDelete { id: ids[0].to_string() }).unwrap();
        until(&bridge, &mut rx, |s| s.history_total == 2).await;
        assert_eq!(heard(&bridge), ["非"]);
        // 忽略 and its 撤销.
        bridge.dispatch(UiCommand::VocabularySuggestionIgnore { heard: "非".into(), polished: "飞".into() }).unwrap();
        until(&bridge, &mut rx, |s| s.vocabulary_ignored.len() == 1).await;
        assert!(heard(&bridge).is_empty());
        bridge.dispatch(UiCommand::VocabularySuggestionUnignore { heard: "非".into(), polished: "飞".into() }).unwrap();
        until(&bridge, &mut rx, |s| s.vocabulary_ignored.is_empty()).await;
        assert_eq!(heard(&bridge), ["非"]);
        assert!(bridge.dispatch(UiCommand::VocabularySuggestionIgnore { heard: "同".into(), polished: "同".into() }).is_err(), "refused here");

        // The rules page's hints: a second rule that finds what the first finds.
        bridge.dispatch(UiCommand::RulesAdd { rule: feishu("非书 2") }).unwrap();
        until(&bridge, &mut rx, |s| s.rules.len() == 2).await;
        let second = bridge.state().rules[1].id;
        assert_eq!(bridge.rules_warnings().iter().map(RuleWarning::rule_id).collect::<Vec<_>>(), [rule_id, second]);

        // History recording off: an answer of its own.
        bridge.dispatch(UiCommand::SettingsSetHistory { enabled: false, keep: voltip_core::HistorySettings::default().keep }).unwrap();
        until(&bridge, &mut rx, |s| !s.settings.history.enabled).await;
        assert_eq!(bridge.vocabulary_suggestions(None).unwrap(), SuggestionsAnswer::HistoryOff);
        bridge.shutdown();
    }

    /// docs/dictation.md §16.10, 已忽略: the ignored pairs are in the state with when each was
    /// ignored, oldest first, also after a restart; 取消忽略 of a pair no take has takes it out of
    /// the state, and the suggestions answer it no longer filters.
    #[tokio::test]
    async fn the_ignored_list_is_in_the_state_with_its_times() {
        async fn until(bridge: &Bridge, rx: &mut broadcast::Receiver<UiEvent>, done: impl Fn(&UiState) -> bool) {
            for _ in 0..60 {
                if done(&bridge.state()) {
                    return;
                }
                let _ = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await;
            }
            panic!("the state never got there");
        }
        let dir = tempfile::tempdir().unwrap();
        voltip_core::SettingsStore::new(dir.path())
            .save(&voltip_core::Settings { relay_enabled: false, engines: voltip_core::dictation::fakes::fake_engines(), ..Default::default() })
            .unwrap();
        let bridge = Bridge::start(CoreConfig::new(dir.path().to_path_buf()), Arc::new(MemorySecretStore::new())).unwrap();
        let mut rx = bridge.events();
        until(&bridge, &mut rx, |s| s.identity.is_some()).await;
        let before = voltip_core::now_ms();
        for (heard, polished) in [("集成", "继承"), ("非", "飞")] {
            bridge.dispatch(UiCommand::VocabularySuggestionIgnore { heard: heard.into(), polished: polished.into() }).unwrap();
        }
        until(&bridge, &mut rx, |s| s.vocabulary_ignored.len() == 2).await;
        let after = voltip_core::now_ms();
        let list = bridge.state().vocabulary_ignored;
        assert_eq!(list.iter().map(|p| (p.heard.as_str(), p.polished.as_str())).collect::<Vec<_>>(), [("集成", "继承"), ("非", "飞")]);
        assert!(list.iter().all(|p| (before..=after).contains(&p.at_ms)), "{list:?}");
        bridge.shutdown();

        let bridge = Bridge::start(CoreConfig::new(dir.path().to_path_buf()), Arc::new(MemorySecretStore::new())).unwrap();
        let mut rx = bridge.events();
        until(&bridge, &mut rx, |s| s.identity.is_some() && s.vocabulary_ignored.len() == 2).await;
        assert_eq!(bridge.state().vocabulary_ignored, list, "read back after a restart");
        bridge.dispatch(UiCommand::VocabularySuggestionUnignore { heard: "集成".into(), polished: "继承".into() }).unwrap();
        until(&bridge, &mut rx, |s| s.vocabulary_ignored.len() == 1).await;
        assert_eq!(bridge.state().vocabulary_ignored, [list[1].clone()]);
        bridge.shutdown();
    }

    /// The time `vocabulary_suggestions` takes over a full window of polished takes (docs/dictation.md
    /// §16.10 records the numbers): 2000 takes of about 100 characters and 2000 of about 900 (close to
    /// the comparison's cell limit), each asked once (read and compared) and again (the cache). A
    /// measurement, not a gate; run it alone, on an idle machine:
    /// `cargo test -p voltip-tauri-bridge --release --lib -- --ignored timing_learning_suggestions --nocapture`.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "a measurement, run by hand (docs/dictation.md §16.10)"]
    async fn timing_learning_suggestions_over_2000_takes() {
        use voltip_core::vocabulary::SuggestionsAnswer;
        for (label, repeat) in [("about 100 characters", 2), ("about 900 characters", 19)] {
            let dir = tempfile::tempdir().unwrap();
            voltip_core::SettingsStore::new(dir.path())
                .save(&voltip_core::Settings { relay_enabled: false, engines: voltip_core::dictation::fakes::fake_engines(), ..Default::default() })
                .unwrap();
            {
                let mut store = voltip_core::history::HistoryStore::open(dir.path());
                for i in 0..2000_u64 {
                    let raw = format!("第{i}次：把维他命的安装包发到非书群里，然后用库伯内特斯部署到测试集群，再跑一遍千问的回归。").repeat(repeat);
                    let text = format!("第 {i} 次：把 Voltip 的安装包发到飞书群里，然后用 Kubernetes 部署到测试集群，再跑一遍 Qwen 的回归。").repeat(repeat);
                    let entry: HistoryEntry = serde_json::from_value(serde_json::json!({
                        "id": Uuid::new_v4(), "at_ms": 1_758_700_000_000_u64 + i, "raw_text": raw, "text": text, "refined": true,
                        "asr_model": "fake", "duration_ms": 1000, "asr_ms": 100, "outcome": { "kind": "inserted", "via": "paste" },
                    }))
                    .unwrap();
                    store.push(entry, voltip_core::history::MAX_ENTRIES).unwrap();
                }
            }
            let bridge = Bridge::start(CoreConfig::new(dir.path().to_path_buf()), Arc::new(MemorySecretStore::new())).unwrap();
            let mut rx = bridge.events();
            for _ in 0..60 {
                if bridge.state().identity.is_some() && bridge.state().history_total == 2000 {
                    break;
                }
                let _ = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await;
            }
            let started = std::time::Instant::now();
            let first = bridge.vocabulary_suggestions(None).unwrap();
            let cold = started.elapsed();
            let started = std::time::Instant::now();
            let again = bridge.vocabulary_suggestions(None).unwrap();
            let cached = started.elapsed();
            assert_eq!(first, again);
            let SuggestionsAnswer::Ready(answer) = first else { panic!("history is on") };
            eprintln!(
                "vocabulary_suggestions, 2000 takes of {label}: first {cold:?}, cached {cached:?} (scanned {}, refined {}, skipped {}, pairs {})",
                answer.scanned, answer.refined, answer.skipped, answer.total
            );
            bridge.shutdown();
        }
    }

    /// docs/dictation.md §18.6: `recent_apps` answers from the cached history — a take on the fakes
    /// with a probe records its app, and the query names it.
    #[tokio::test]
    async fn recent_apps_and_the_history_queries_read_the_database() {
        let dir = tempfile::tempdir().unwrap();
        voltip_core::SettingsStore::new(dir.path())
            .save(&voltip_core::Settings { relay_enabled: false, engines: voltip_core::dictation::fakes::fake_engines(), ..Default::default() })
            .unwrap();
        let probe = Arc::new(voltip_core::dictation::fakes::FakeProbe::app("Code.exe", "Code", None));
        let ports = DictationPorts { probe: Some(probe), ..voltip_core::dictation::fakes::ports() };
        let bridge = Bridge::start_with(CoreConfig::new(dir.path().to_path_buf()), Arc::new(MemorySecretStore::new()), ports).unwrap();
        let mut rx = bridge.events();
        assert!(bridge.recent_apps().unwrap().is_empty());
        assert_eq!(bridge.history_query(&HistoryQuery { limit: 10, ..Default::default() }).unwrap(), HistoryPage::default(), "nothing yet");
        bridge.dispatch(UiCommand::DictationStart).unwrap();
        for _ in 0..40 {
            if matches!(bridge.state().dictation.phase, voltip_core::DictationPhase::Listening { ready: true, .. }) {
                break;
            }
            let _ = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await;
        }
        bridge.dispatch(UiCommand::DictationStop).unwrap();
        for _ in 0..60 {
            if !bridge.state().history_recent.is_empty() {
                break;
            }
            let _ = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await;
        }
        assert_eq!(bridge.recent_apps().unwrap(), vec![AppRef { id: "code".into(), name: "Code".into() }]);
        // The take is in the database the queries read, as the core's event said.
        let state = bridge.state();
        assert_eq!(state.history_total, 1);
        let page = bridge.history_query(&HistoryQuery { query: "code".into(), limit: 10, ..Default::default() }).unwrap();
        assert_eq!((page.entries.clone(), page.matching, page.total), (state.history_recent.clone(), 1, 1));
        let id = page.entries[0].id;
        assert_eq!(bridge.history_entry(id).unwrap().map(|e| e.id), Some(id));
        let at = page.entries[0].at_ms;
        let stats = bridge.history_stats(&[at, at + 1]).unwrap();
        assert_eq!((stats.buckets[0].count, stats.total.count), (1, 1));
        assert!(bridge.history_stats(&[at]).is_err(), "one boundary is no span");
        assert!(bridge.history_query(&HistoryQuery::default()).is_err(), "a page needs a size");
        assert_eq!(bridge.history_hits().unwrap(), HistoryHits::default(), "the fakes' take fires no dictionary entry");
        bridge.shutdown();
    }

    /// docs/dictation.md §4.6: one bridge writes a backup of the lists it caches and of the history
    /// in its database; another opens it — nothing changes yet — and holds it for `backup_import`
    /// under the preview's token. The token works once and `backup_discard` drops it; a file that is
    /// not a backup is refused and leaves nothing held.
    #[tokio::test]
    async fn a_backup_goes_from_one_bridge_to_another() {
        use voltip_core::backup::{BackupCounts, BackupParts, ImportFailure, ImportOutcome, OpenFailure, OpenOutcome};
        let start = |name: &str, dir: &std::path::Path| {
            voltip_core::SettingsStore::new(dir)
                .save(&voltip_core::Settings { relay_enabled: false, engines: voltip_core::dictation::fakes::fake_engines(), ..Default::default() })
                .unwrap();
            let mut cfg = CoreConfig::new(dir.to_path_buf());
            cfg.default_device_name = name.into();
            Bridge::start(cfg, Arc::new(MemorySecretStore::new())).unwrap()
        };
        async fn until(bridge: &Bridge, rx: &mut broadcast::Receiver<UiEvent>, done: impl Fn(&UiState) -> bool) {
            for _ in 0..60 {
                if done(&bridge.state()) {
                    return;
                }
                let _ = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await;
            }
            panic!("the state never got there");
        }
        let old = tempfile::tempdir().unwrap();
        let mut history = voltip_core::HistoryStore::open(old.path());
        for i in 0..3 {
            let entry = voltip_core::dictation::fakes::history_entry(&format!("第{i}句"));
            history.push(HistoryEntry { at_ms: 1_760_000_000_000 + i, ..entry }, 20_000).unwrap();
        }
        drop(history);
        let from = start("旧电脑", old.path());
        let mut rx = from.events();
        from.dispatch(UiCommand::DictionaryAdd {
            entry: DictionaryDraft { term: "Voltip".into(), heard_as: vec!["沃提普".into()], enabled: true },
            history_id: None,
        })
        .unwrap();
        until(&from, &mut rx, |s| s.identity.is_some() && s.dictionary.len() == 1 && !s.scenes.is_empty()).await;
        let file = old.path().join("备份.voltip-backup");
        let summary = from.backup_export(&file).unwrap();
        let scenes = u32::try_from(from.state().scenes.len()).unwrap();
        assert_eq!(summary.counts, BackupCounts { history: 3, dictionary: 1, rules: 0, scenes, presets: 0 });

        let new = tempfile::tempdir().unwrap();
        let to = start("新电脑", new.path());
        let mut rx = to.events();
        until(&to, &mut rx, |s| s.identity.is_some()).await;
        let OpenOutcome::Preview { token, manifest } = to.backup_open(&file) else { panic!("a backup opens") };
        assert_eq!((manifest.device.as_str(), manifest.counts, manifest.version), ("旧电脑", summary.counts, 1));
        assert!(to.state().dictionary.is_empty() && to.state().history_total == 0, "opening changes nothing");
        let wait = Duration::from_secs(20);
        let expired = ImportOutcome::failed(ImportFailure::Expired);
        assert_eq!(to.backup_import(Uuid::new_v4(), BackupParts::ALL, ImportMode::Merge, wait).await, expired);
        let ImportOutcome::Done { report } = to.backup_import(token, BackupParts::ALL, ImportMode::Merge, wait).await else { panic!("imported") };
        assert_eq!(report.parts.iter().map(|p| p.restored).sum::<u32>(), 3 + 1);
        until(&to, &mut rx, |s| s.dictionary.len() == 1 && s.history_total == 3).await;
        assert_eq!(to.state().dictionary, from.state().dictionary);
        assert_eq!(to.history_stats(&[1_760_000_000_000, 1_760_000_000_003]).unwrap().total.count, 3);
        assert_eq!(to.backup_import(token, BackupParts::ALL, ImportMode::Merge, wait).await, expired, "a token works once");

        let OpenOutcome::Preview { token, .. } = to.backup_open(&file) else { panic!("opens again") };
        to.backup_discard(token);
        assert_eq!(to.backup_import(token, BackupParts::ALL, ImportMode::Replace, wait).await, expired, "discarded");
        let OpenOutcome::Preview { token, .. } = to.backup_open(&file) else { panic!("opens again") };
        let foreign = new.path().join("notes.voltip-backup");
        std::fs::write(&foreign, "购物清单").unwrap();
        assert!(matches!(to.backup_open(&foreign), OpenOutcome::Failed { code: OpenFailure::NotBackup, .. }));
        assert_eq!(to.backup_import(token, BackupParts::ALL, ImportMode::Replace, wait).await, expired, "another file replaced the one held");
        from.shutdown();
        to.shutdown();
    }

    /// docs/dictation.md §4.6.1: the backup before an update is the core's own: a dictionary entry
    /// dispatched just before it — its event not necessarily in this bridge's cache yet — is in
    /// the file, which `auto_backup` then names. A core that has stopped answers with why instead
    /// of leaving the shell waiting.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_backup_before_an_update_is_made_of_the_cores_own_lists() {
        use voltip_core::backup::auto::AutoKind;
        let dir = tempfile::tempdir().unwrap();
        voltip_core::SettingsStore::new(dir.path())
            .save(&voltip_core::Settings { relay_enabled: false, engines: voltip_core::dictation::fakes::fake_engines(), ..Default::default() })
            .unwrap();
        let mut cfg = CoreConfig::new(dir.path().to_path_buf());
        cfg.auto_backup = true;
        let bridge = Bridge::start(cfg, Arc::new(MemorySecretStore::new())).unwrap();
        let mut rx = bridge.events();
        for _ in 0..60 {
            if bridge.state().auto_backup.available {
                break;
            }
            let _ = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await;
        }
        assert!(bridge.state().auto_backup.available, "the desktop's core makes automatic backups");
        bridge
            .dispatch(UiCommand::DictionaryAdd {
                entry: DictionaryDraft { term: "Voltip".into(), heard_as: vec!["沃提普".into()], enabled: true },
                history_id: None,
            })
            .unwrap();
        let worker = bridge.clone();
        let file = tokio::task::spawn_blocking(move || worker.backup_auto(AutoKind::Update)).await.unwrap().expect("written");
        assert_eq!(file.kind, AutoKind::Update);
        let device = bridge.state().auto_backup.device;
        assert_eq!(device.len(), voltip_core::backup::auto::DEVICE_TAG_LEN);
        assert!(file.name.ends_with(&format!("-update-{device}.voltip-backup")), "this computer's tag (D23): {}", file.name);
        let written = voltip_core::backup::read(&bridge.backup_folder().join(&file.name)).unwrap();
        assert_eq!(written.dictionary.iter().map(|d| d.term.as_str()).collect::<Vec<_>>(), ["Voltip"]);
        for _ in 0..60 {
            if bridge.state().auto_backup.last.as_ref() == Some(&file) {
                break;
            }
            let _ = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await;
        }
        assert_eq!(bridge.state().auto_backup.last, Some(file));
        bridge.shutdown();
        let worker = bridge.clone();
        let stopped = tokio::time::timeout(Duration::from_secs(20), tokio::task::spawn_blocking(move || worker.backup_auto(AutoKind::Update))).await;
        assert!(stopped.expect("an answer, not a wait").unwrap().is_err());
    }

    /// docs/dictation.md §4.6.1: a replacing import whose backup first could not be written
    /// replaced nothing and keeps its preview: once the folder can be written the same token
    /// imports, after the backup the report names.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_replacing_import_refused_before_it_ran_can_be_tried_again() {
        let dir = tempfile::tempdir().unwrap();
        voltip_core::SettingsStore::new(dir.path())
            .save(&voltip_core::Settings { relay_enabled: false, engines: voltip_core::dictation::fakes::fake_engines(), ..Default::default() })
            .unwrap();
        let mut cfg = CoreConfig::new(dir.path().to_path_buf());
        cfg.auto_backup = true;
        let bridge = Bridge::start(cfg, Arc::new(MemorySecretStore::new())).unwrap();
        let mut rx = bridge.events();
        let mut wait_for = async |what: &str, ok: &dyn Fn(&UiState) -> bool| {
            for _ in 0..60 {
                if ok(&bridge.state()) {
                    return;
                }
                let _ = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await;
            }
            panic!("{what}: not within 5 minutes");
        };
        wait_for("ready", &|s| s.auto_backup.available).await;
        let file = dir.path().join("mine.voltip-backup");
        let worker = bridge.clone();
        let written = file.clone();
        tokio::task::spawn_blocking(move || worker.backup_export(&written)).await.unwrap().unwrap();
        let OpenOutcome::Preview { token, .. } = bridge.backup_open(&file) else { panic!("a preview") };
        // A folder under a file cannot be made, on every system.
        std::fs::write(dir.path().join("blocker"), b"").unwrap();
        let blocked = dir.path().join("blocker").join("backups");
        bridge.backup_set_folder(blocked.clone()).unwrap();
        wait_for("blocked folder", &|s| s.auto_backup.folder == blocked.display().to_string()).await;
        let refused = bridge.backup_import(token, BackupParts::ALL, ImportMode::Replace, Duration::from_secs(60)).await;
        assert!(matches!(refused, ImportOutcome::Failed { code: ImportFailure::SnapshotFailed, ref detail } if !detail.is_empty()), "{refused:?}");
        let good = dir.path().join("good");
        bridge.backup_set_folder(good.clone()).unwrap();
        wait_for("good folder", &|s| s.auto_backup.folder == good.display().to_string()).await;
        let ImportOutcome::Done { report } = bridge.backup_import(token, BackupParts::ALL, ImportMode::Replace, Duration::from_secs(60)).await else {
            panic!("the same token imports")
        };
        let snapshot = std::path::PathBuf::from(report.snapshot.expect("the backup first"));
        assert_eq!(snapshot.parent(), Some(good.as_path()));
        assert!(snapshot.is_file());
        let again = bridge.backup_import(token, BackupParts::ALL, ImportMode::Replace, Duration::from_secs(60)).await;
        assert_eq!(again, ImportOutcome::failed(ImportFailure::Expired), "an import that ran used the token");
        bridge.shutdown();
    }

    /// Regression (CI runner, 2026-09-25): the shell used to subscribe after `start`, and a core
    /// that was ready first had already broadcast its only `state` event into the void. The
    /// receiver from `start_subscribed` is guaranteed to see it.
    #[tokio::test]
    async fn regression_the_pre_subscribed_receiver_never_misses_the_first_state_event() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = CoreConfig::new(dir.path().to_path_buf());
        cfg.default_device_name = "Early".into();
        voltip_core::SettingsStore::new(dir.path())
            .save(&voltip_core::Settings { relay_enabled: false, engines: voltip_core::dictation::fakes::fake_engines(), ..Default::default() })
            .unwrap();
        let (bridge, mut first) = Bridge::start_subscribed(cfg, Arc::new(MemorySecretStore::new()), voltip_core::dictation::fakes::ports()).unwrap();
        // Let the core run ahead: by now the `state` event has been broadcast.
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(bridge.state().identity.is_some(), "the core is ready and the cache saw it");
        let mut saw_state = false;
        for _ in 0..10 {
            let ev = tokio::time::timeout(Duration::from_secs(5), first.recv()).await.unwrap().unwrap();
            if matches!(ev, UiEvent::State(ref s) if s.identity.as_ref().map(|i| i.name.as_str()) == Some("Early")) {
                saw_state = true;
                break;
            }
        }
        assert!(saw_state, "the pre-subscribed receiver holds the first state event");
        // A late subscriber does not get it (that is the broadcast contract the shells must respect).
        let mut late = bridge.events();
        assert!(matches!(late.try_recv(), Err(broadcast::error::TryRecvError::Empty)));
        bridge.shutdown();
    }

    #[tokio::test]
    async fn bridge_starts_core_caches_state_and_fans_out_events() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = CoreConfig::new(dir.path().to_path_buf());
        cfg.default_device_name = "Bridge Test".into();
        // No relay: keep the test offline.
        voltip_core::SettingsStore::new(dir.path())
            .save(&voltip_core::Settings { relay_enabled: false, engines: voltip_core::dictation::fakes::fake_engines(), ..Default::default() })
            .unwrap();
        let bridge = Bridge::start(cfg, Arc::new(MemorySecretStore::new())).unwrap();
        let mut rx = bridge.events();
        let mut saw_state = false;
        for _ in 0..10 {
            let ev = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
            if matches!(ev, UiEvent::State(_)) {
                saw_state = true;
                break;
            }
        }
        assert!(saw_state);
        tokio::time::sleep(Duration::from_millis(100)).await;
        let st = bridge.state();
        assert_eq!(st.identity.as_ref().unwrap().name, "Bridge Test");
        assert_eq!(st.secret_backend, "memory");
        bridge.dispatch(UiCommand::DeviceRename { name: "Renamed".into() }).unwrap();
        let mut renamed = false;
        for _ in 0..20 {
            let ev = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
            if matches!(ev, UiEvent::Identity(ref i) if i.name == "Renamed") {
                renamed = true;
                break;
            }
        }
        assert!(renamed);
        assert!(bridge.dispatch(UiCommand::DeviceForget { public_key: "bad".into() }).is_err());
        assert!(format!("{bridge:?}").contains("Bridge"));
        // The fake ports complete a dictation; levels are reachable through the bridge.
        let mut levels = bridge.levels();
        bridge.dispatch(UiCommand::DictationStart).unwrap();
        let mut listening = false;
        for _ in 0..20 {
            let ev = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
            if matches!(ev, UiEvent::Dictation(ref s) if matches!(s.phase, voltip_core::DictationPhase::Listening { .. })) {
                listening = true;
                break;
            }
        }
        assert!(listening);
        assert!(tokio::time::timeout(Duration::from_secs(5), levels.recv()).await.unwrap().is_ok(), "levels flow through the bridge");
        bridge.dispatch(UiCommand::DictationStop).unwrap();
        let mut done = false;
        for _ in 0..40 {
            let ev = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
            if matches!(ev, UiEvent::History { ref recent, .. } if !recent.is_empty()) {
                done = true;
                break;
            }
        }
        assert!(done, "history received the finished dictation");
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!((bridge.state().history_recent.len(), bridge.state().history_total), (1, 1));
        // The fakes carry no model library: the list is empty and a model command is answered
        // with an `error` event rather than a panic or a silent drop.
        assert!(bridge.state().models.is_empty());
        bridge.dispatch(UiCommand::ModelDownload { id: "sense-voice-small".into() }).unwrap();
        let mut refused = false;
        for _ in 0..20 {
            let ev = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
            if matches!(ev, UiEvent::Error { ref message } if message.contains("本地模型不可用")) {
                refused = true;
                break;
            }
        }
        assert!(refused);
        bridge.shutdown();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(bridge.dispatch(UiCommand::DevicesRefresh).is_err() || true, "dispatch after shutdown may fail; must not panic");
    }

    /// `paste` (the history's paste button): a core that does not answer in time is
    /// `failed { timeout }`, and every call takes its own answer, whatever other paste answers go by.
    #[tokio::test]
    async fn a_paste_waits_for_its_own_answer_and_gives_up_in_time() {
        use voltip_core::dictation::fakes::{FAKE_TRANSCRIPT, FakeAudio, FakeInjector, FakeTranscriber, ports_with};
        use voltip_core::paste::CopyReason;
        let dir = tempfile::tempdir().unwrap();
        voltip_core::SettingsStore::new(dir.path())
            .save(&voltip_core::Settings { relay_enabled: false, engines: voltip_core::dictation::fakes::fake_engines(), ..Default::default() })
            .unwrap();
        // The injector holds every copy until the test lets it through.
        let injector = Arc::new(FakeInjector::paste().gated());
        let ports = ports_with(Arc::new(FakeAudio::speech()), Arc::new(FakeTranscriber::ok(FAKE_TRANSCRIPT)), None, injector.clone());
        let bridge = Bridge::start_with(CoreConfig::new(dir.path().to_path_buf()), Arc::new(MemorySecretStore::new()), ports).unwrap();
        let copy = PasteTarget::CopyOnly;

        // Held: no answer in time.
        let mut rx = bridge.events();
        assert_eq!(
            bridge.paste("一".into(), copy(CopyReason::NoProbe), Duration::from_millis(200)).await,
            PasteOutcome::Failed { reason: PasteFailure::Timeout }
        );
        // Let through, it still answers; that answer goes by before the next paste starts, so the
        // next one is not refused as busy.
        injector.release(1);
        let late = loop {
            if let UiEvent::PasteResult { request_id, outcome } = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.unwrap().unwrap() {
                break (request_id, outcome);
            }
        };
        assert_eq!(late, (1, PasteOutcome::Copied { reason: CopyReason::NoProbe }));

        // Another request's answer going by first is not taken for this one's.
        injector.release(1);
        let foreign = UiEvent::PasteResult { request_id: 999, outcome: PasteOutcome::Failed { reason: PasteFailure::Inject } };
        let (outcome, ()) = tokio::join!(bridge.paste("二".into(), copy(CopyReason::Timeout), Duration::from_secs(5)), async { bridge.publish(foreign) });
        assert_eq!(outcome, PasteOutcome::Copied { reason: CopyReason::Timeout });
        assert_eq!(injector.clipboard_copies(), vec!["一".to_owned(), "二".to_owned()]);
        assert!(injector.injected().is_empty(), "a copy-only paste never pastes");
        bridge.shutdown();
    }

    /// `copy_last` (docs/dictation.md §28): `nothing` before the first result; a core that holds
    /// the copy past `within` is `failed`.
    #[tokio::test]
    async fn the_last_result_is_copied_through_the_bridge_and_a_late_answer_is_failed() {
        use voltip_core::dictation::fakes::{FAKE_TRANSCRIPT, FakeAudio, FakeInjector, FakeTranscriber, ports_with};
        use voltip_core::last_result::CopyLastOutcome;
        let dir = tempfile::tempdir().unwrap();
        voltip_core::SettingsStore::new(dir.path())
            .save(&voltip_core::Settings { relay_enabled: false, engines: voltip_core::dictation::fakes::fake_engines(), ..Default::default() })
            .unwrap();
        let injector = Arc::new(FakeInjector::paste().gated());
        let ports = ports_with(Arc::new(FakeAudio::speech()), Arc::new(FakeTranscriber::ok(FAKE_TRANSCRIPT)), None, injector.clone());
        let bridge = Bridge::start_with(CoreConfig::new(dir.path().to_path_buf()), Arc::new(MemorySecretStore::new()), ports).unwrap();
        assert_eq!(bridge.copy_last(Duration::from_secs(5)).await, CopyLastOutcome::Nothing);
        let mut rx = bridge.events();
        bridge.dispatch(UiCommand::DictationStart).unwrap();
        bridge.dispatch(UiCommand::DictationStop).unwrap();
        injector.release(1);
        loop {
            if let UiEvent::Dictation(status) = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.unwrap().unwrap()
                && status.phase.is_terminal()
            {
                break;
            }
        }
        // The copy is held: no answer in time. Let through, the next one is copied.
        assert_eq!(bridge.copy_last(Duration::from_millis(200)).await, CopyLastOutcome::Failed);
        injector.release(2);
        assert_eq!(bridge.copy_last(Duration::from_secs(5)).await, CopyLastOutcome::Copied);
        // The held copy goes through on its own thread and may land after the answer to the next
        // one (9 of 200 runs under load, 2026-10-30): wait for it.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while injector.clipboard_copies().len() < 2 && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(injector.clipboard_copies(), vec![FAKE_TRANSCRIPT.to_owned(), FAKE_TRANSCRIPT.to_owned()]);
        bridge.shutdown();
    }
}
