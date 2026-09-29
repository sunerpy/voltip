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
use voltip_core::dictation::LevelFrame;
use voltip_core::paste::{PasteFailure, PasteOutcome, PasteTarget};
use voltip_core::scenes::{MAX_RECENT_APPS, recent_apps, validate_scene_draft};
use voltip_core::ui::{UiEvent, UiState};
use voltip_core::vocabulary::{export_rules_toml, parse_rules_toml, preview, validate_dictionary_draft, validate_rule_draft};
use voltip_core::{
    Activation, AppCore, AppRef, ContextSharing, CoreCommand, CoreConfig, CoreError, CoreEvent, CoreHandle, DictationPorts, DictionaryDraft, EdgeSource,
    EngineSettings, EntrySource, ImportMode, Locale, OverlayPlacement, PreviewDraft, ProviderId, RuleDraft, SceneDraft, ServiceKind, TakeKind, ThemeId,
    VocabularyPreview,
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
    /// Announce this device on the LAN and browse for the others (docs/pairing.md 「局域网发现」).
    SettingsSetLanDiscovery {
        /// On / off.
        enabled: bool,
    },
    /// Keep a pairing open until turned off (docs/pairing.md 「常开配对」; desktop only).
    SettingsSetPairingAlwaysOn {
        /// On / off.
        enabled: bool,
    },
    /// Join the pairing a nearby device waits for (its LAN tag from `nearby`).
    PairingJoinNearby {
        /// `NearbyDevice.fingerprint`.
        fingerprint: String,
    },
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
    /// Replace a scene's name, flag, match and overrides.
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
    /// What of a take's context may go to the LLM (docs/dictation.md §18.5); persisted, the core
    /// re-emits `settings`.
    SettingsSetContextSharing {
        /// Send the application's name.
        app_name: bool,
        /// Send the window title.
        window_title: bool,
    },
}

impl UiCommand {
    /// Translate to a core command. Fails on malformed keys.
    pub fn into_core(self) -> Result<CoreCommand, BridgeError> {
        Ok(match self {
            Self::PairingStart => CoreCommand::StartPairing,
            Self::PairingJoinCode { code } => CoreCommand::JoinWithCode(code),
            Self::PairingJoinTicket { uri } => CoreCommand::JoinWithTicket(uri),
            Self::PairingConfirm => CoreCommand::ConfirmPairing,
            Self::PairingReject => CoreCommand::RejectPairing,
            Self::PairingCancel => CoreCommand::CancelPairing,
            Self::PairingReset => CoreCommand::ResetPairing,
            Self::DeviceForget { public_key } => CoreCommand::ForgetDevice(parse_key(&public_key)?),
            Self::DeviceRename { name } => CoreCommand::RenameDevice(name),
            Self::SendText { public_key, body } => CoreCommand::SendText { to: parse_key(&public_key)?, body },
            Self::PhoneTakeStart { public_key } => CoreCommand::PhoneTakeStart { to: parse_key(&public_key)? },
            Self::PhoneTakeStop => CoreCommand::PhoneTakeStop,
            Self::PhoneTakeCancel => CoreCommand::PhoneTakeCancel,
            Self::PhoneTextSend { public_key, body, source } => CoreCommand::PhoneTextSend { to: parse_key(&public_key)?, body, source },
            Self::SentTextsClear => CoreCommand::SentTextsClear,
            Self::SettingsSetLanDiscovery { enabled } => CoreCommand::SetLanDiscovery(enabled),
            Self::SettingsSetPairingAlwaysOn { enabled } => CoreCommand::SetPairingAlwaysOn(enabled),
            Self::PairingJoinNearby { fingerprint } => CoreCommand::PairingJoinNearby(fingerprint),
            Self::SettingsSetRelay { url, enabled } => CoreCommand::SetRelay { url, enabled },
            Self::SettingsSetTheme { theme, follow_system } => CoreCommand::SetTheme { theme, follow_system },
            Self::SettingsSetHotkey { hotkey } => CoreCommand::SetHotkey(hotkey),
            Self::SettingsSetEditHotkey { hotkey } => CoreCommand::SetEditHotkey(hotkey),
            Self::SettingsSetSoloKey { key } => CoreCommand::SetSoloKey(key),
            Self::SettingsSetMicrophone { device } => CoreCommand::SetMicrophone(device),
            Self::SettingsSetLocale { locale } => CoreCommand::SetLocale(locale),
            Self::SettingsSetAutoUpdate { enabled } => CoreCommand::SetAutoUpdate(enabled),
            Self::SettingsSetHistory { enabled, keep } => CoreCommand::SetHistory(voltip_core::HistorySettings { enabled, keep }),
            Self::SettingsSetOverlay { placement } => CoreCommand::SetOverlay(placement),
            Self::DevicesRefresh => CoreCommand::RefreshDevices,
            Self::ConnectivityCheck => CoreCommand::CheckConnectivity,
            Self::DictationStart => CoreCommand::DictationStart,
            Self::DictationStop => CoreCommand::DictationStop,
            Self::DictationCancel => CoreCommand::DictationCancel,
            Self::HotkeyEdge { pressed, at_ms, source, purpose, chorded } => CoreCommand::HotkeyEdge { pressed, at_ms, source, purpose, chorded },
            Self::SettingsSetActivation { activation, hold_threshold_ms, extra_recording_ms } => {
                CoreCommand::SetActivation { activation, hold_threshold_ms, extra_recording_ms }
            }
            Self::SettingsSetEngines { engines } => CoreCommand::SetEngines(engines),
            Self::ProviderKeySet { provider, kind, value } => CoreCommand::SetProviderKey { provider, kind, value },
            Self::ProviderProbe { provider, kind, base_url, key } => CoreCommand::ProbeProvider { provider, kind, base_url, key },
            Self::HistoryDelete { id } => CoreCommand::HistoryDelete(parse_id(&id)?),
            Self::HistoryClear => CoreCommand::HistoryClear,
            Self::HistoryStar { id, starred } => CoreCommand::HistoryStar(parse_id(&id)?, starred),
            Self::ModelDownload { id } => CoreCommand::ModelDownload(id),
            Self::ModelCancel { id } => CoreCommand::ModelCancel(id),
            Self::ModelRemove { id } => CoreCommand::ModelRemove(id),
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
            Self::RulesAdd { rule } => CoreCommand::RuleAdd(validate_rule_draft(&rule).map_err(bad)?),
            Self::RulesUpdate { id, rule } => CoreCommand::RuleUpdate { id: parse_id(&id)?, draft: validate_rule_draft(&rule).map_err(bad)? },
            Self::RulesRemove { id } => CoreCommand::RuleRemove(parse_id(&id)?),
            Self::RulesReorder { ids } => CoreCommand::RuleReorder(parse_ids(&ids)?),
            Self::RulesImport { toml, mode } => CoreCommand::RulesImport { rules: parse_rules_toml(&toml).map_err(bad)?, mode },
            // Same split as the vocabulary: a draft wrong on its own is the command's error; list-level
            // refusals (duplicate name, cap, unknown id) arrive as `error` events.
            Self::ScenesAdd { scene } => CoreCommand::SceneAdd(validate_scene_draft(&scene).map_err(bad_scene)?),
            Self::ScenesUpdate { id, scene } => CoreCommand::SceneUpdate { id: parse_id(&id)?, draft: validate_scene_draft(&scene).map_err(bad_scene)? },
            Self::ScenesRemove { id } => CoreCommand::SceneRemove(parse_id(&id)?),
            Self::ScenesReorder { ids } => CoreCommand::SceneReorder(parse_ids(&ids)?),
            Self::SettingsSetContextSharing { app_name, window_title } => CoreCommand::SetContextSharing(ContextSharing { app_name, window_title }),
        })
    }
}

fn bad(e: voltip_core::VocabularyError) -> BridgeError {
    BridgeError::BadArgument(e.to_string())
}

fn bad_scene(e: voltip_core::SceneError) -> BridgeError {
    BridgeError::BadArgument(e.to_string())
}

fn parse_ids(texts: &[String]) -> Result<Vec<Uuid>, BridgeError> {
    texts.iter().map(|t| parse_id(t)).collect()
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
}

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
        let (handle, core_events) = AppCore::start_with(config, secret_store, ports)?;
        let bridge = Self { handle, state: state.clone(), events: tx.clone(), paste_ids: Arc::new(AtomicU64::new(0)) };
        tokio::spawn(pump(core_events, state, tx));
        Ok((bridge, first))
    }

    /// Current state for a freshly mounted webview.
    pub fn state(&self) -> UiState {
        self.state.lock().clone()
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
        let core_cmd = cmd.into_core()?;
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

    /// `rules_export` (docs/dictation.md §16.5): the cached rules as TOML text.
    pub fn rules_export(&self) -> Result<String, BridgeError> {
        let rules = self.state.lock().rules.clone();
        export_rules_toml(&rules).map_err(bad)
    }

    /// `recent_apps` (docs/dictation.md §18.6): the applications the cached history saw, newest
    /// first, one per id, at most [`MAX_RECENT_APPS`] — what the scene editor offers to pick from.
    pub fn recent_apps(&self) -> Vec<AppRef> {
        recent_apps(&self.state.lock().history, MAX_RECENT_APPS)
    }

    /// Stop the core.
    pub fn shutdown(&self) {
        let _ = self.handle.try_send(CoreCommand::Shutdown);
    }
}

async fn pump(mut core_events: mpsc::Receiver<CoreEvent>, state: Arc<Mutex<UiState>>, tx: broadcast::Sender<UiEvent>) {
    while let Some(ev) = core_events.recv().await {
        let ui = state.lock().apply(ev);
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
        let err: String = c.into_core().unwrap_err().into();
        assert!(err.starts_with("scenes: ") && err.contains("至少要有一个应用"), "{err}");
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
        let c: UiCommand = serde_json::from_str(r#"{"command":"scenes_update","id":"nope","scene":{"name":"y","match":{"apps":["code"]}}}"#).unwrap();
        assert!(String::from(c.into_core().unwrap_err()).contains("UUID"));
        let c: UiCommand = serde_json::from_str(&format!(r#"{{"command":"scenes_remove","id":"{hid}"}}"#)).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::SceneRemove(_)));
        let c: UiCommand = serde_json::from_str(&format!(r#"{{"command":"scenes_reorder","ids":["{hid}"]}}"#)).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::SceneReorder(ids) if ids.len() == 1));
        let c: UiCommand = serde_json::from_str(r#"{"command":"settings_set_context_sharing","appName":false,"windowTitle":true}"#).unwrap();
        assert!(matches!(c.into_core().unwrap(), CoreCommand::SetContextSharing(ContextSharing { app_name: false, window_title: true })));
        assert!(serde_json::from_str::<UiCommand>(r#"{"command":"settings_set_context_sharing","appName":true}"#).is_err(), "both switches are required");
        for (json, expect) in [
            (r#"{"command":"dictation_start"}"#, "DictationStart"),
            (r#"{"command":"dictation_stop"}"#, "DictationStop"),
            (r#"{"command":"dictation_cancel"}"#, "DictationCancel"),
            (r#"{"command":"history_clear"}"#, "HistoryClear"),
            (r#"{"command":"pairing_start"}"#, "StartPairing"),
            (r#"{"command":"pairing_confirm"}"#, "ConfirmPairing"),
            (r#"{"command":"pairing_reject"}"#, "RejectPairing"),
            (r#"{"command":"pairing_cancel"}"#, "CancelPairing"),
            (r#"{"command":"pairing_reset"}"#, "ResetPairing"),
            (r#"{"command":"devices_refresh"}"#, "RefreshDevices"),
            (r#"{"command":"connectivity_check"}"#, "CheckConnectivity"),
            (r#"{"command":"device_rename","name":"X"}"#, "RenameDevice"),
            (r#"{"command":"pairing_join_ticket","uri":"voltip://pair?v=1&t=AA"}"#, "JoinWithTicket"),
        ] {
            let c: UiCommand = serde_json::from_str(json).unwrap();
            let core = c.into_core().unwrap();
            assert!(format!("{core:?}").starts_with(expect), "{json} -> {core:?}");
        }
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

    /// docs/dictation.md §18.6: `recent_apps` answers from the cached history — a take on the fakes
    /// with a probe records its app, and the query names it.
    #[tokio::test]
    async fn recent_apps_come_from_the_cached_history() {
        let dir = tempfile::tempdir().unwrap();
        voltip_core::SettingsStore::new(dir.path())
            .save(&voltip_core::Settings { relay_enabled: false, engines: voltip_core::dictation::fakes::fake_engines(), ..Default::default() })
            .unwrap();
        let probe = Arc::new(voltip_core::dictation::fakes::FakeProbe::app("Code.exe", "Code", None));
        let ports = DictationPorts { probe: Some(probe), ..voltip_core::dictation::fakes::ports() };
        let bridge = Bridge::start_with(CoreConfig::new(dir.path().to_path_buf()), Arc::new(MemorySecretStore::new()), ports).unwrap();
        let mut rx = bridge.events();
        assert!(bridge.recent_apps().is_empty());
        bridge.dispatch(UiCommand::DictationStart).unwrap();
        for _ in 0..40 {
            if matches!(bridge.state().dictation.phase, voltip_core::DictationPhase::Listening { ready: true, .. }) {
                break;
            }
            let _ = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await;
        }
        bridge.dispatch(UiCommand::DictationStop).unwrap();
        for _ in 0..60 {
            if !bridge.state().history.is_empty() {
                break;
            }
            let _ = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await;
        }
        assert_eq!(bridge.recent_apps(), vec![AppRef { id: "code".into(), name: "Code".into() }]);
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
            if matches!(ev, UiEvent::History { ref entries } if !entries.is_empty()) {
                done = true;
                break;
            }
        }
        assert!(done, "history received the finished dictation");
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(bridge.state().history.len(), 1);
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
}
