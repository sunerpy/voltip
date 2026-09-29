//! The core task: owns all state, consumes commands and link events, emits UI events.
//!
//! Connectivity model (direct first, relay as fallback): every device runs a small LAN host for as long as
//! the core runs. Paired devices meet on rendezvous channels — on a peer's LAN host when one is
//! reachable (direct), on the public relay otherwise (fallback) — and traffic always takes the
//! best secure path that exists. LAN endpoints are learned from the pairing ticket and refreshed
//! over the encrypted channel, then persisted with the trusted record so a restart without any
//! relay still finds the other device.

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;
use voltip_crypto::{PublicKey, Role};
use voltip_identity::{ConnectionKind, DeviceIdentity, DeviceIdentityPublic, IdentityCheck, IdentityManager, SecretStore, TrustedDevice, TrustedDeviceStore};
use voltip_pairing::{Action, Event, Initiator, JoinMethod, NonceLedger, Now, PairingState, Reachability, Responder, Snapshot, Timeouts};
use voltip_protocol::app::AppMessage;
use voltip_protocol::relay::{RelayErrorCode, RelayFrame};
use voltip_protocol::ticket::PairingTicket;
use voltip_protocol::{PairCode, ProtocolVersion, SessionId};
use voltip_transport::{ConnectionState, DirectHost, LinkConfig, LinkEvent, ReconnectPolicy, RelayEndpoint, RelayLink};

use crate::dictation::activation::{Activation, ActivationConfig, ActivationMachine, Edge, EdgeSource, Intent, PhaseHint};
use crate::dictation::engine::{Effect, Internal};
use crate::dictation::{DictationEngine, DictationError, DictationPhase, DictationPorts, DictationStatus, LevelFrame, RefineHints, SelectionTiming, TakeKind};
use crate::engines::{BuiltIn, EngineSettings, EngineStatus, MAX_LOCAL_THREADS, ProviderId, ResolvedEngines, ServiceKind, UserSecrets};
use crate::history::{HistoryEntry, HistoryStore};
use crate::models::{CancelToken, DEFAULT_LOCAL_MODEL_ID, ModelInstallState, ModelManager, ModelState};
use crate::peer::{LinkId, PeerPath, PeerPhase, PeerState};
use crate::presets::{CustomPreset, PresetDraft, PresetStore, PresetTrial, PresetTryOutcome};
use crate::providers::{ProbeError, ProbeFailure, ProbeOutcome, ProbeReport, key_entries, key_entry};
use crate::scenes::{ContextSharing, Scene, SceneDraft, SceneStore};
use crate::settings::{Locale, Settings, SettingsStore, ThemeId};
use crate::view::{DeviceConnection, DeviceView, RelaySource, RelayStatus};
use crate::vocabulary::{DictionaryDraft, DictionaryEntry, DictionaryStore, EntrySource, ImportMode, ReplacementRule, RuleDraft, RuleStore, Vocabulary};
use crate::{CoreError, is_initiator, rendezvous_channel};

mod always_on;
mod check;
mod nearby;
mod take_codec;
mod takes;
mod texts;

/// Default TCP port of the LAN host. A fixed port keeps stored LAN hints valid across restarts;
/// when it is taken the host falls back to an ephemeral port and peers learn the new one.
pub const DEFAULT_LAN_PORT: u16 = 47831;

/// Directory under the app data dir that holds the local model library.
pub const MODELS_DIR_NAME: &str = "models";

/// Download progress reaches the UI at most this often per model …
pub const MODEL_PROGRESS_INTERVAL: Duration = Duration::from_millis(250);
/// … unless this many bytes arrived since the last report.
pub const MODEL_PROGRESS_BYTES: u64 = 1024 * 1024;

/// The extra-recording window (`Settings.extra_recording_ms`) is waited out in slices this long,
/// so a cancel or a second stop takes effect within one slice.
pub const EXTRA_RECORDING_POLL: Duration = Duration::from_millis(25);
/// Upper bound for `Settings.hold_threshold_ms` and `Settings.extra_recording_ms` (a typo must not
/// make the key or the stop unusable).
pub const MAX_ACTIVATION_MS: u32 = 5_000;

/// Milliseconds since the Unix epoch: the clock of `CoreCommand::HotkeyEdge { at_ms }`. Shells
/// stamp their edges with it so the runtime's grace timer and the machine agree on "now".
pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)).unwrap_or(0)
}

/// Startup configuration.
#[derive(Clone, Debug)]
pub struct CoreConfig {
    /// Where `settings.json` and `trusted-devices.json` live.
    pub data_dir: PathBuf,
    /// Name given to a brand-new identity.
    pub default_device_name: String,
    /// Sent in relay `hello`.
    pub client_version: String,
    /// The app version the About pane shows (the shells pass their package version).
    pub app_version: String,
    /// Pairing deadlines.
    pub pairing_timeouts: Timeouts,
    /// Relay reconnect policy.
    pub reconnect: ReconnectPolicy,
    /// Tick cadence (drives pairing timeouts and LAN retries).
    pub tick: Duration,
    /// Master switch for the LAN host and for dialling peers' LAN hosts.
    pub direct_enabled: bool,
    /// Preferred bind address of the LAN host (port 0 = ephemeral).
    pub direct_bind: SocketAddr,
    /// Initial backoff between attempts to reach one peer's LAN host.
    pub direct_retry: Duration,
    /// Backoff ceiling for LAN attempts.
    pub direct_retry_max: Duration,
    /// TCP + WebSocket deadline for one LAN attempt.
    pub direct_connect_timeout: Duration,
    /// A trusted-peer re-handshake that has not finished within this long is abandoned
    /// (the peer shows as offline again instead of "connecting" forever).
    pub peer_handshake_timeout: Duration,
    /// Where local ASR models are installed (`<data_dir>/models`, docs/dictation.md §10). The shell
    /// builds its `ModelStore` from this so the core and the store agree on one place.
    pub models_root: PathBuf,
    /// Run takes whose audio a trusted phone streams (docs/dictation.md §20): the desktop yes,
    /// the phone no (it answers `unavailable`).
    pub accepts_phone_takes: bool,
    /// LAN discovery (docs/pairing.md 「局域网发现」): the shells pass [`crate::discovery::MdnsDiscovery`],
    /// the tests an in-memory LAN; `None` announces and browses nothing.
    pub discovery: Option<Arc<dyn crate::discovery::Discovery>>,
}

impl CoreConfig {
    /// Defaults for `data_dir`.
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            models_root: data_dir.join(MODELS_DIR_NAME),
            data_dir,
            default_device_name: default_device_name(),
            client_version: format!("voltip/{}", env!("CARGO_PKG_VERSION")),
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            pairing_timeouts: Timeouts::default(),
            reconnect: ReconnectPolicy::default(),
            tick: Duration::from_secs(1),
            direct_enabled: true,
            direct_bind: SocketAddr::from(([0, 0, 0, 0], DEFAULT_LAN_PORT)),
            direct_retry: Duration::from_secs(5),
            direct_retry_max: Duration::from_secs(60),
            direct_connect_timeout: Duration::from_secs(3),
            peer_handshake_timeout: Duration::from_secs(15),
            accepts_phone_takes: true,
            discovery: None,
        }
    }
}

fn default_device_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("{} Device", voltip_protocol::Platform::current().label()))
}

/// Commands from the UI.
#[derive(Debug, Clone)]
pub enum CoreCommand {
    /// Desktop: create a pairing session (relay if configured, else LAN host).
    StartPairing,
    /// Phone: join by six digits (relay only).
    JoinWithCode(String),
    /// Phone: join by scanned `voltip://pair?...` URI.
    JoinWithTicket(String),
    /// User compared safety codes and accepted.
    ConfirmPairing,
    /// User rejected.
    RejectPairing,
    /// Abort the current pairing.
    CancelPairing,
    /// Return the pairing screen to idle after a terminal state.
    ResetPairing,
    /// Remove a trusted device.
    ForgetDevice(PublicKey),
    /// Rename this device.
    RenameDevice(String),
    /// Change relay configuration (takes effect immediately).
    SetRelay {
        /// Explicit URL or `None` for the build default.
        url: Option<String>,
        /// Master switch.
        enabled: bool,
    },
    /// Change the global dictation hotkey (validated; persisted; shells re-register it on `Settings`).
    SetHotkey(String),
    /// Change (`Some`) or switch off (`None`) the voice-edit hotkey (docs/dictation.md §19);
    /// validated like the dictation hotkey and refused when it is the same chord.
    SetEditHotkey(Option<String>),
    /// Choose (`Some`) or switch off (`None`) the lone-key trigger (docs/dictation.md §13.1);
    /// persisted, the desktop shell (un)installs its input hook on `Settings`.
    SetSoloKey(Option<crate::SoloKey>),
    /// The microphone takes record from (`Some` device id) or the system default (`None`);
    /// persisted, used from the next take on.
    SetMicrophone(Option<String>),
    /// Change theme.
    SetTheme {
        /// Theme.
        theme: ThemeId,
        /// Follow OS.
        follow_system: bool,
    },
    /// Change the UI language (persisted; every window follows the `Settings` event).
    SetLocale(Locale),
    /// Turn the desktop shell's automatic update check on or off (persisted; the shell follows the
    /// `Settings` event).
    SetAutoUpdate(bool),
    /// History recording and retention (docs/dictation.md §4); a smaller `keep` trims at once.
    SetHistory(crate::settings::HistorySettings),
    /// Where the dictation pill appears (persisted; the desktop shell follows the `Settings` event).
    SetOverlay(crate::settings::OverlayPlacement),
    /// Phone: stream a take to the trusted desktop `to`, which records, recognises and delivers
    /// it (docs/dictation.md §20); answered by `PhoneTake` events.
    PhoneTakeStart {
        /// The desktop.
        to: PublicKey,
    },
    /// Phone: the speaker let go; the rest of the audio goes out and the desktop delivers.
    PhoneTakeStop,
    /// Phone: discard the take on both ends.
    PhoneTakeCancel,
    /// Phone: send `body` to the online trusted desktop `to`, to be inserted at its cursor
    /// (docs/dictation.md §20.6); listed in `UiState.sent_texts` until the desktop answers.
    PhoneTextSend {
        /// The desktop.
        to: PublicKey,
        /// 1..=`MAX_PHONE_TEXT_CHARS` characters, not only whitespace.
        body: String,
        /// Typed or the clipboard.
        source: crate::phone::PhoneTextSource,
    },
    /// Phone: forget the list of sent texts.
    SentTextsClear,
    /// Desktop: paste a result from the history into the window the user came from
    /// ([`crate::paste`]); [`CoreEvent::PasteResult`] answers with the same `request_id`. Refused
    /// while a take runs; never recorded in the history.
    PasteText {
        /// Picked by the shell to find the answer.
        request_id: u64,
        /// The text.
        text: String,
        /// The window the shell found, or why there is none.
        target: crate::paste::PasteTarget,
    },
    /// Announce this device on the LAN and browse for the others (persisted,
    /// `Settings.lan_discovery`).
    SetLanDiscovery(bool),
    /// Keep a pairing open until turned off (persisted, `Settings.pairing_always_on`; desktop only).
    SetPairingAlwaysOn(bool),
    /// Join the pairing the nearby device `fingerprint` (its LAN tag) waits for.
    PairingJoinNearby(String),
    /// Send text to an online trusted device.
    SendText {
        /// Recipient.
        to: PublicKey,
        /// Body.
        body: String,
    },
    /// Re-emit the device list.
    RefreshDevices,
    /// Probe the relay and every paired device's LAN addresses, ping the online ones, and report
    /// ([`crate::connectivity`]).
    CheckConnectivity,
    /// Open the microphone (hotkey pressed / "开始听写").
    DictationStart,
    /// Close the microphone and run ASR → refine → inject (hotkey released / "停止").
    DictationStop,
    /// Discard the recording or the pending result.
    DictationCancel,
    /// A hotkey / CLI key transition (docs/dictation.md §13): the activation machine turns it into
    /// start / stop / cancel / lock according to `Settings.activation`.
    HotkeyEdge {
        /// Key down (`true`) or up.
        pressed: bool,
        /// When, in [`now_ms`] milliseconds (only differences matter).
        at_ms: u64,
        /// Hotkey, CLI or UI.
        source: EdgeSource,
        /// Which key: the dictation hotkey or the voice-edit hotkey (docs/dictation.md §19).
        purpose: TakeKind,
        /// Another key or button joined the held lone-key trigger (docs/dictation.md §13.1): the
        /// press began a shortcut, so the take it started is cancelled ([`ActivationMachine::chorded`]).
        /// `pressed` is not read then.
        chorded: bool,
    },
    /// Change how the hotkey drives a dictation (persisted; the machine follows at once).
    SetActivation {
        /// `hold` | `toggle` | `hold_or_toggle`.
        activation: Activation,
        /// `hold_or_toggle` threshold, ≤ [`MAX_ACTIVATION_MS`].
        hold_threshold_ms: u32,
        /// Trailing capture after a stop, ≤ [`MAX_ACTIVATION_MS`].
        extra_recording_ms: u32,
    },
    /// Replace `Settings.engines`; clients are rebuilt and `Engines` re-emitted.
    SetEngines(EngineSettings),
    /// Store (`Some`) or delete (`None`) the user's key for a provider's service (a vendor's
    /// services share one key). The value never comes back out.
    SetProviderKey {
        /// Provider.
        provider: ProviderId,
        /// Service the key was entered for.
        kind: ServiceKind,
        /// New value; `None` / empty deletes.
        value: Option<String>,
    },
    /// Ask a provider's service for its model list (docs/dictation.md §3.3) with the values in
    /// the form (`None` = the saved ones); answered by `CoreEvent::ProviderProbe`.
    ProbeProvider {
        /// Provider.
        provider: ProviderId,
        /// Service.
        kind: ServiceKind,
        /// Base URL being edited.
        base_url: Option<String>,
        /// Key being edited (never stored by the probe).
        key: Option<String>,
    },
    /// Remove one history entry.
    HistoryDelete(Uuid),
    /// Remove every history entry.
    HistoryClear,
    /// Flag / unflag a history entry.
    HistoryStar(Uuid, bool),
    /// Fetch and verify a local model (catalogue id); progress arrives as `Models` events.
    ModelDownload(String),
    /// Stop a running download (`.part` files stay for a resume).
    ModelCancel(String),
    /// Delete an installed (or partially downloaded) model directory.
    ModelRemove(String),
    /// Append a personal dictionary entry (docs/dictation.md §16).
    DictionaryAdd {
        /// Term, mis-hearings, flag.
        draft: DictionaryDraft,
        /// Manual, or the history entry it came from.
        source: EntrySource,
    },
    /// Replace an entry's term, mis-hearings and flag.
    DictionaryUpdate {
        /// Which entry.
        id: Uuid,
        /// New content.
        draft: DictionaryDraft,
    },
    /// Delete a dictionary entry.
    DictionaryRemove(Uuid),
    /// Reorder the dictionary (a permutation of the current ids; order = glossary priority).
    DictionaryReorder(Vec<Uuid>),
    /// Append a replacement rule.
    RuleAdd(RuleDraft),
    /// Replace a rule (id and position stay).
    RuleUpdate {
        /// Which rule.
        id: Uuid,
        /// New content.
        draft: RuleDraft,
    },
    /// Delete a rule.
    RuleRemove(Uuid),
    /// Reorder the rules (a permutation of the current ids; order = execution order).
    RuleReorder(Vec<Uuid>),
    /// Replace or merge the rule list with already parsed TOML rules (docs/dictation.md §16.5).
    RulesImport {
        /// The file's rules, in order.
        rules: Vec<RuleDraft>,
        /// Replace or merge.
        mode: ImportMode,
    },
    /// Append a scene (docs/dictation.md §18).
    SceneAdd(SceneDraft),
    /// Replace a scene's name, flag, match and overrides (id and position stay).
    SceneUpdate {
        /// Which scene.
        id: Uuid,
        /// New content.
        draft: SceneDraft,
    },
    /// Delete a scene.
    SceneRemove(Uuid),
    /// Reorder the scenes (a permutation of the current ids; order = matching order).
    SceneReorder(Vec<Uuid>),
    /// Append a custom preset (docs/dictation.md §21).
    PresetAdd(PresetDraft),
    /// Replace a custom preset's name and instruction.
    PresetUpdate {
        /// Which preset.
        id: Uuid,
        /// New content.
        draft: PresetDraft,
    },
    /// Delete a custom preset (settings and scenes that name it refine with 校对 from then on).
    PresetRemove(Uuid),
    /// 试一试: run `text` through the current clean-up with a preset, save nothing, answer with
    /// [`CoreEvent::PresetTry`] carrying `id`.
    PresetTry {
        /// The request's id, echoed in the answer.
        id: u64,
        /// What to run.
        trial: PresetTrial,
        /// Sample text.
        text: String,
    },
    /// Which parts of a take's context may go to the LLM (persisted; the next take follows).
    SetContextSharing(ContextSharing),
    /// Stop the core.
    Shutdown,
}

/// 试一试 without an AI service (docs/dictation.md §21).
pub const PRESET_TRY_UNCONFIGURED: &str = "尚未配置 AI 润色服务，无法试运行预设";

/// Everything the actor loop listens to.
struct Inbox {
    cmd_rx: mpsc::Receiver<CoreCommand>,
    link_rx: mpsc::Receiver<(LinkId, LinkEvent)>,
    dict_rx: mpsc::Receiver<Internal>,
    model_rx: mpsc::Receiver<ModelProgress>,
    act_rx: mpsc::Receiver<ActivationTimer>,
    phone_rx: mpsc::Receiver<takes::PhoneEvent>,
    check_rx: mpsc::Receiver<check::Probed>,
    disc_rx: mpsc::Receiver<crate::discovery::DiscoveryEvent>,
}

/// Events to the UI.
#[derive(Debug, Clone)]
pub enum CoreEvent {
    /// Core is up.
    Ready {
        /// This device.
        identity: DeviceIdentityPublic,
        /// Settings.
        settings: Settings,
        /// Secret store backend (`keychain`, `credential-manager`, `memory`, ...).
        secret_backend: &'static str,
        /// The app version (`CoreConfig.app_version`).
        app_version: String,
    },
    /// Identity changed (rename).
    Identity(DeviceIdentityPublic),
    /// Settings changed.
    Settings(Settings),
    /// Relay link status.
    Relay(RelayStatus),
    /// Pairing screen state.
    Pairing(Snapshot),
    /// Device list (full replacement).
    Devices(Vec<DeviceView>),
    /// A pairing completed.
    Trusted(TrustedDevice),
    /// A trusted device forgot this one and said so over the channel: its record is gone here too.
    Unpaired(TrustedDevice),
    /// A trusted device came back with a different key.
    IdentityChanged {
        /// Trusted record.
        previous: TrustedDevice,
        /// Fingerprint of the presented key.
        presented_fingerprint: String,
    },
    /// Text from a trusted device.
    Message {
        /// Sender.
        from: PublicKey,
        /// Body.
        body: String,
    },
    /// Dictation state machine moved.
    Dictation(DictationStatus),
    /// History list (full replacement); on `Ready` and after every change.
    History(Vec<HistoryEntry>),
    /// Resolved engine configuration (providers, models, user-entered hosts, key presence); on
    /// `Ready` and after changes.
    Engines(EngineStatus),
    /// The answer to one `ProbeProvider`.
    ProviderProbe(ProbeReport),
    /// Local model library (full replacement, download progress folded in); on `Ready` and after
    /// every change.
    Models(Vec<ModelState>),
    /// Personal dictionary (full replacement); on `Ready` and after every change.
    Dictionary(Vec<DictionaryEntry>),
    /// Replacement rules in execution order (full replacement); on `Ready` and after every change.
    Rules(Vec<ReplacementRule>),
    /// Scenes in matching order (full replacement); on `Ready` and after every change.
    Scenes(Vec<Scene>),
    /// Custom presets, full replacement (docs/dictation.md §21).
    Presets(Vec<CustomPreset>),
    /// The answer to one [`CoreCommand::PresetTry`].
    PresetTry {
        /// The request's id.
        id: u64,
        /// The text, or why there is none.
        outcome: PresetTryOutcome,
    },
    /// The phone's take streamed to a desktop (docs/dictation.md §20); `None` before the first.
    PhoneTake(Option<crate::phone::PhoneTakeView>),
    /// The phone's list of texts sent to a desktop (docs/dictation.md §20.6), newest first; on
    /// `Ready` and after every change.
    SentTexts(Vec<crate::phone::SentText>),
    /// The answer to [`CoreCommand::PasteText`].
    PasteResult {
        /// The command's id.
        request_id: u64,
        /// What became of the text.
        outcome: crate::paste::PasteOutcome,
    },
    /// What the LAN browse sees (docs/pairing.md 「局域网发现」), whole; after every change.
    Nearby(Vec<crate::discovery::NearbyDevice>),
    /// The connectivity self-check started or finished ([`crate::connectivity`]).
    Connectivity(crate::connectivity::ConnectivityStatus),
    /// Non-fatal error for the UI.
    Error(String),
}

/// Handle to submit commands.
#[derive(Clone, Debug)]
pub struct CoreHandle {
    cmd: mpsc::Sender<CoreCommand>,
    levels: broadcast::Sender<LevelFrame>,
}

/// Level frames buffered for a slow subscriber before it starts lagging (≈ 1 s at 30 Hz).
const LEVELS_QUEUE: usize = 32;

impl CoreHandle {
    /// Submit a command.
    pub async fn send(&self, cmd: CoreCommand) -> Result<(), CoreError> {
        self.cmd.send(cmd).await.map_err(|_| CoreError::NotRunning)
    }

    /// Submit without awaiting (from sync Tauri commands).
    pub fn try_send(&self, cmd: CoreCommand) -> Result<(), CoreError> {
        self.cmd.try_send(cmd).map_err(|_| CoreError::NotRunning)
    }

    /// Input levels while a dictation capture runs (nothing arrives otherwise). The shell feeds
    /// the webview's meter from this instead of opening the device a second time.
    pub fn levels(&self) -> broadcast::Receiver<LevelFrame> {
        self.levels.subscribe()
    }
}

/// The core.
pub struct AppCore;

impl AppCore {
    /// [`AppCore::start_with`] using the in-memory dictation fakes: for tests and shells without a
    /// native pipeline (the phone), never for the desktop.
    pub fn start(config: CoreConfig, secret_store: Arc<dyn SecretStore>) -> Result<(CoreHandle, mpsc::Receiver<CoreEvent>), CoreError> {
        Self::start_with(config, secret_store, crate::dictation::fakes::ports())
    }

    /// Load identity + trust + settings + history, resolve the engines, connect the relay (if any)
    /// and start the task.
    pub fn start_with(
        config: CoreConfig,
        secret_store: Arc<dyn SecretStore>,
        ports: DictationPorts,
    ) -> Result<(CoreHandle, mpsc::Receiver<CoreEvent>), CoreError> {
        let secrets = secret_store.clone();
        let manager = IdentityManager::new(secret_store);
        let identity = manager.load_or_create(&config.default_device_name)?;
        let trusted = TrustedDeviceStore::open(&config.data_dir)?;
        let settings_store = SettingsStore::new(&config.data_dir);
        let (settings, quarantined) = settings_store.load_or_quarantine()?;
        if let Some(reason) = quarantined {
            tracing::warn!(%reason, "settings.json could not be read; starting with the defaults");
        }
        let history = HistoryStore::open(&config.data_dir);
        let sent_texts = crate::phone::SentTexts::open(&config.data_dir);
        let (dictionary, dictionary_notice) = DictionaryStore::open(&config.data_dir);
        let (rules, rules_notice) = RuleStore::open(&config.data_dir);
        let (scenes, scenes_notice) = SceneStore::open(&config.data_dir);
        let (presets, presets_notice) = PresetStore::open(&config.data_dir);
        let built_in = BuiltIn::from_build();
        let user_secrets = load_user_secrets(secrets.as_ref());
        let models = ports.models.clone();
        let service_probe = ports.service_probe.clone();
        let library = models.as_ref().map(|m| m.scan()).unwrap_or_default();
        let resolved = ResolvedEngines::resolve_with_models(&settings.engines, &user_secrets, &built_in, &library);
        let (levels_tx, _) = broadcast::channel(LEVELS_QUEUE);
        let (mut dictation, dict_rx) = DictationEngine::new(ports, &resolved, levels_tx.clone());
        dictation.set_vocabulary(Arc::new(Vocabulary::compile(dictionary.entries(), rules.rules())));
        dictation.set_scenes(Arc::new(scenes.scenes().to_vec()));
        dictation.set_presets(Arc::new(presets.presets().to_vec()));
        dictation.set_context_sharing(settings.context_sharing);
        dictation.set_microphone(settings.microphone.clone());
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let (evt_tx, evt_rx) = mpsc::channel(512);
        let (link_tx, link_rx) = mpsc::channel(1024);
        let (model_tx, model_rx) = mpsc::channel(64);
        let (act_tx, act_rx) = mpsc::channel(16);
        let (phone_tx, phone_rx) = mpsc::channel(64);
        let (check_tx, check_rx) = mpsc::channel(4);
        let (disc_tx, disc_rx) = mpsc::channel(64);
        let activation = ActivationState::new(ActivationConfig::from(&settings));
        let mut rt = Runtime {
            config,
            manager,
            identity,
            trusted,
            settings_store,
            settings,
            secrets,
            user_secrets,
            built_in,
            service_probe,
            dictation,
            history,
            dictionary,
            rules,
            scenes,
            presets,
            startup_notices: [dictionary_notice, rules_notice, scenes_notice, presets_notice].into_iter().flatten().collect(),
            models,
            library,
            downloads: HashMap::new(),
            model_tx,
            activation,
            act_tx,
            evt: evt_tx,
            link_tx,
            relay: None,
            relay_status: RelayStatus { endpoint: None, source: RelaySource::None, state: ConnectionState::Disconnected, attempts: 0 },
            host: None,
            dials: HashMap::new(),
            next_dial: 1,
            pairing: Pairing::None,
            pairing_link: None,
            pairing_session: None,
            pairing_peer_hints: Vec::new(),
            pending_start: None,
            ledger: NonceLedger::default(),
            peers: HashMap::new(),
            session_to_peer: HashMap::new(),
            pending_attach: HashMap::new(),
            remote_take: None,
            phone_take: None,
            next_phone_take: 0,
            phone_tx,
            take_outbox: Vec::new(),
            texts: texts::TextInbox::default(),
            sent_texts,
            check: None,
            next_check: 0,
            last_check: None,
            check_tx,
            disc_tx,
            lan: nearby::Lan::default(),
            always_on_at: None,
        };
        rt.connect_relay()?;
        let inbox = Inbox { cmd_rx, link_rx, dict_rx, model_rx, act_rx, phone_rx, check_rx, disc_rx };
        tokio::spawn(async move { rt.run(inbox).await });
        Ok((CoreHandle { cmd: cmd_tx, levels: levels_tx }, evt_rx))
    }
}

/// Read the user's engine secrets from the store. A store that cannot be read leaves the secret
/// unset (and logs): the identity already loaded from the same store, so this is rare.
fn load_user_secrets(store: &dyn SecretStore) -> UserSecrets {
    let mut secrets = UserSecrets::default();
    for entry in key_entries() {
        match store.get(entry) {
            Ok(Some(bytes)) => secrets.set_entry(entry, String::from_utf8(bytes.to_vec()).ok()),
            Ok(None) => {}
            Err(e) => tracing::warn!(error = %e, secret = entry, "secret store read failed; treating as unset"),
        }
    }
    secrets
}

/// An endpoint the user entered must be an http(s) URL with a host.
fn check_http_url(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| e.to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("must be http(s)".into());
    }
    if parsed.host_str().is_none_or(str::is_empty) {
        return Err("missing host".into());
    }
    Ok(())
}

fn kind_name(kind: ServiceKind) -> &'static str {
    match kind {
        ServiceKind::Asr => "asr",
        ServiceKind::Llm => "llm",
    }
}

enum Pairing {
    None,
    Initiator(Box<Initiator>),
    Responder(Box<Responder>),
}

/// A pairing that waits for its LAN link to come up before the state machine starts.
enum PendingStart {
    /// `StartPairing` issued while the loopback link to our own LAN host was still connecting.
    Initiator,
    /// A ticket join whose outgoing LAN connection is still connecting.
    Responder(JoinMethod),
}

/// This device's LAN host and the loopback link through which the core itself is attached to it.
struct Host {
    host: DirectHost,
    link: RelayLink,
}

/// An outgoing LAN connection.
struct Dial {
    link: RelayLink,
    /// The trusted peer whose host this is; `None` while it only carries a pairing joined by ticket.
    peer: Option<PublicKey>,
}

/// A running model download.
struct Download {
    cancel: CancelToken,
    task: tokio::task::JoinHandle<()>,
}

/// What a download task reports back to the core task.
enum ModelProgress {
    /// An intermediate state (`Downloading` / `Verifying`), already throttled.
    State { id: String, state: ModelInstallState },
    /// The task is over: installed, failed or cancelled.
    Finished { id: String, result: Result<ModelInstallState, String>, cancelled: bool },
}

impl From<&Settings> for ActivationConfig {
    fn from(settings: &Settings) -> Self {
        Self { mode: settings.activation, hold_threshold_ms: settings.hold_threshold_ms, ..Self::default() }
    }
}

/// What the activation timers report back to the core task.
enum ActivationTimer {
    /// The release grace window a machine asked for has passed: poll them.
    Grace,
    /// The extra-recording window after a stop is over: close the microphone now.
    ExtraOver { session: u64 },
    /// The edit key has been up for the release grace (docs/dictation.md §19 `AfterKeyUp`): the
    /// X11 grab is over, the copy chord can reach the application.
    CaptureKeyUp { session: u64 },
}

/// A stop waiting out `Settings.extra_recording_ms` (docs/dictation.md §13).
struct DeferredStop {
    session: u64,
    cancel: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}

/// The activation machines — one per hotkey, the dictation key and the voice-edit key (docs/
/// dictation.md §19), sharing one configuration — and the timers they need, owned by the runtime.
/// The engine knows nothing about activation: intents map onto its start / stop / cancel, and
/// `Listening.locked` is stamped onto the statuses on their way out.
struct ActivationState {
    /// The dictation hotkey's machine.
    dictate: ActivationMachine,
    /// The voice-edit hotkey's machine.
    edit: ActivationMachine,
    /// `Listening.locked` as the UI sees it (`Intent::Lock`); cleared when the phase leaves `Listening`.
    locked: bool,
    /// The one grace timer (the earliest `ActivationMachine::deadline_ms` of the two).
    grace: Option<tokio::task::JoinHandle<()>>,
    deferred_stop: Option<DeferredStop>,
    /// `AfterKeyUp` (§19): the copy of an edit take waits out the release grace after the key-up.
    capture: Option<tokio::task::JoinHandle<()>>,
}

impl ActivationState {
    fn new(config: ActivationConfig) -> Self {
        Self { dictate: ActivationMachine::new(config), edit: ActivationMachine::new(config), locked: false, grace: None, deferred_stop: None, capture: None }
    }

    fn machine(&mut self, purpose: TakeKind) -> &mut ActivationMachine {
        match purpose {
            TakeKind::Dictation => &mut self.dictate,
            TakeKind::Edit => &mut self.edit,
        }
    }

    fn disarm_capture(&mut self) {
        if let Some(t) = self.capture.take() {
            t.abort();
        }
    }

    /// Drop a deferred stop (cancelled, superseded or fired); `true` when there was one.
    fn take_deferred(&mut self) -> bool {
        match self.deferred_stop.take() {
            Some(d) => {
                d.cancel.store(true, Ordering::SeqCst);
                d.task.abort();
                true
            }
            None => false,
        }
    }
}

struct Runtime {
    config: CoreConfig,
    manager: IdentityManager,
    identity: DeviceIdentity,
    trusted: TrustedDeviceStore,
    settings_store: SettingsStore,
    settings: Settings,
    /// The same store the identity lives in; also holds the user's engine secrets.
    secrets: Arc<dyn SecretStore>,
    user_secrets: UserSecrets,
    built_in: BuiltIn,
    /// Lists a provider's models (`ProbeProvider`); `None` on shells without HTTP.
    service_probe: Option<Arc<dyn crate::dictation::ServiceProbe>>,
    dictation: DictationEngine,
    history: HistoryStore,
    /// `dictionary.json` (docs/dictation.md §16).
    dictionary: DictionaryStore,
    /// `rules.json` (docs/dictation.md §16).
    rules: RuleStore,
    /// `scenes.json` (docs/dictation.md §18).
    scenes: SceneStore,
    presets: PresetStore,
    /// Why a vocabulary or scene file could not be used at startup; told to the UI after `Ready`.
    startup_notices: Vec<String>,
    /// The model library port (`None`: no local models on this shell).
    models: Option<Arc<dyn ModelManager>>,
    /// Last scan of the library, with in-flight download states patched in.
    library: Vec<ModelState>,
    /// Running downloads by catalogue id.
    downloads: HashMap<String, Download>,
    /// Download tasks report here.
    model_tx: mpsc::Sender<ModelProgress>,
    /// Hotkey activation (docs/dictation.md §13).
    activation: ActivationState,
    /// Activation timers report here.
    act_tx: mpsc::Sender<ActivationTimer>,
    evt: mpsc::Sender<CoreEvent>,
    /// Every link forwards its events here, tagged with its id.
    link_tx: mpsc::Sender<(LinkId, LinkEvent)>,
    relay: Option<(RelayLink, RelayEndpoint)>,
    relay_status: RelayStatus,
    host: Option<Host>,
    dials: HashMap<u64, Dial>,
    next_dial: u64,
    pairing: Pairing,
    /// Link the current pairing runs on.
    pairing_link: Option<LinkId>,
    /// Last relay session id seen for the current pairing (survives terminal states).
    pairing_session: Option<SessionId>,
    /// LAN endpoints from the ticket we joined with; stored on the trusted record once trusted.
    pairing_peer_hints: Vec<String>,
    pending_start: Option<PendingStart>,
    ledger: NonceLedger,
    peers: HashMap<PublicKey, PeerState>,
    session_to_peer: HashMap<(LinkId, SessionId), PublicKey>,
    pending_attach: HashMap<LinkId, VecDeque<PublicKey>>,
    /// Desktop: the take a paired phone streams (docs/dictation.md §20).
    remote_take: Option<takes::RemoteTake>,
    /// Phone: the take this phone streams to a desktop.
    phone_take: Option<takes::PhoneTake>,
    /// Phone: the take id of the last `PhoneTakeStart`.
    next_phone_take: u32,
    /// Phone: the microphone open and the pump report here.
    phone_tx: mpsc::Sender<takes::PhoneEvent>,
    /// Take and text messages to seal and send once the current event is handled.
    take_outbox: Vec<(PublicKey, AppMessage)>,
    /// Desktop: phones' texts waiting for the injector (docs/dictation.md §20.6).
    texts: texts::TextInbox,
    /// Phone: the texts it sent, newest first.
    sent_texts: crate::phone::SentTexts,
    /// The connectivity self-check in flight.
    check: Option<check::Pending>,
    /// The id of the last check started.
    next_check: u64,
    /// The last finished check (sent again with the next start).
    last_check: Option<crate::connectivity::ConnectivityReport>,
    /// The probe task reports here.
    check_tx: mpsc::Sender<check::Probed>,
    /// The LAN browse reports here (docs/pairing.md 「局域网发现」).
    disc_tx: mpsc::Sender<crate::discovery::DiscoveryEvent>,
    /// What the LAN browse sees, and what this device announces.
    lan: nearby::Lan,
    /// Always-on pairing (docs/pairing.md 「常开配对」): when the next session opens.
    always_on_at: Option<Instant>,
}

impl Runtime {
    fn emit(&self, event: CoreEvent) {
        if self.evt.try_send(event).is_err() {
            tracing::warn!("core event queue full; UI is not draining events");
        }
    }

    fn now() -> Now {
        Now::system()
    }

    // ---------------- links ----------------

    /// Start a link and forward its events into the runtime queue under `id`.
    fn spawn_link(&self, id: LinkId, cfg: LinkConfig) -> RelayLink {
        let (link, mut rx) = RelayLink::spawn(cfg);
        let tx = self.link_tx.clone();
        tokio::spawn(async move {
            while let Some(ev) = rx.recv().await {
                if tx.send((id, ev)).await.is_err() {
                    break;
                }
            }
        });
        link
    }

    fn link(&self, id: LinkId) -> Option<&RelayLink> {
        match id {
            LinkId::Relay => self.relay.as_ref().map(|(l, _)| l),
            LinkId::Host => self.host.as_ref().map(|h| &h.link),
            LinkId::Dial(n) => self.dials.get(&n).map(|d| &d.link),
        }
    }

    async fn send_on(&self, id: LinkId, frame: RelayFrame) -> Result<(), CoreError> {
        let Some(link) = self.link(id) else { return Err(CoreError::Invalid("连接已断开".into())) };
        Ok(link.send(frame).await?)
    }

    fn link_connected(&self, id: LinkId) -> bool {
        self.link(id).is_some_and(|l| l.state().is_connected())
    }

    fn relay_connected(&self) -> bool {
        self.link_connected(LinkId::Relay)
    }

    fn connect_relay(&mut self) -> Result<(), CoreError> {
        if let Some((link, _)) = self.relay.take() {
            tokio::spawn(async move { link.close().await });
        }
        self.drop_link_state(LinkId::Relay);
        if !self.settings.relay_enabled {
            self.relay_status = RelayStatus { endpoint: None, source: RelaySource::None, state: ConnectionState::Disconnected, attempts: 0 };
            self.emit(CoreEvent::Relay(self.relay_status.clone()));
            return Ok(());
        }
        let endpoint = RelayEndpoint::resolve(self.settings.relay_url.as_deref())?;
        let Some(endpoint) = endpoint else {
            self.relay_status = RelayStatus { endpoint: None, source: RelaySource::None, state: ConnectionState::Disconnected, attempts: 0 };
            self.emit(CoreEvent::Relay(self.relay_status.clone()));
            return Ok(());
        };
        // Only a relay the user typed in is named; the build's own relay stays anonymous.
        let user_relay = self.settings.relay_url.as_deref().map(str::trim).is_some_and(|u| !u.is_empty());
        let (shown, source) = if user_relay { (Some(endpoint.to_string()), RelaySource::User) } else { (None, RelaySource::Builtin) };
        let mut cfg = LinkConfig::new(endpoint.clone());
        cfg.client_version = self.config.client_version.clone();
        cfg.reconnect = self.config.reconnect;
        let link = self.spawn_link(LinkId::Relay, cfg);
        self.relay_status = RelayStatus { endpoint: shown, source, state: ConnectionState::Connecting, attempts: 0 };
        self.emit(CoreEvent::Relay(self.relay_status.clone()));
        self.relay = Some((link, endpoint));
        Ok(())
    }

    /// Bind the LAN host and attach the core to it over loopback.
    async fn start_host(&mut self) {
        if !self.config.direct_enabled {
            return;
        }
        let host = match DirectHost::bind_lan_host(self.config.direct_bind).await {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(error = %e, "LAN host unavailable; direct connections disabled");
                self.emit(CoreEvent::Error(format!("LAN host unavailable: {e}")));
                return;
            }
        };
        let endpoint = match host.loopback_endpoint() {
            Ok(e) => e,
            Err(e) => {
                self.emit(CoreEvent::Error(format!("LAN host unavailable: {e}")));
                return;
            }
        };
        let mut cfg = LinkConfig::new(endpoint);
        cfg.client_version = self.config.client_version.clone();
        cfg.reconnect = self.config.reconnect;
        let link = self.spawn_link(LinkId::Host, cfg);
        self.host = Some(Host { host, link });
    }

    fn lan_hints(&self) -> Vec<String> {
        self.host.as_ref().map(|h| h.host.lan_hints()).unwrap_or_default()
    }

    /// Open an outgoing LAN connection to `hint` (`ip:port`).
    fn dial(&mut self, hint: &str, peer: Option<PublicKey>) -> Result<LinkId, CoreError> {
        let endpoint = RelayEndpoint::parse(&format!("ws://{hint}/ws"))?;
        let mut cfg = LinkConfig::new(endpoint);
        cfg.client_version = self.config.client_version.clone();
        cfg.reconnect = ReconnectPolicy::never();
        cfg.connect_timeout = self.config.direct_connect_timeout;
        let n = self.next_dial;
        self.next_dial += 1;
        let id = LinkId::Dial(n);
        let link = self.spawn_link(id, cfg);
        self.dials.insert(n, Dial { link, peer });
        Ok(id)
    }

    /// Forget every rendezvous session that lived on `id` (the link is gone or being replaced).
    fn drop_link_state(&mut self, id: LinkId) {
        for st in self.peers.values_mut() {
            st.drop_link(id);
        }
        self.session_to_peer.retain(|(l, _), _| *l != id);
        self.pending_attach.remove(&id);
    }

    async fn close_dial(&mut self, n: u64) {
        self.drop_link_state(LinkId::Dial(n));
        if let Some(d) = self.dials.remove(&n) {
            d.link.close().await;
        }
    }

    // ---------------- main loop ----------------

    async fn run(mut self, inbox: Inbox) {
        let Inbox { mut cmd_rx, mut link_rx, mut dict_rx, mut model_rx, mut act_rx, mut phone_rx, mut check_rx, mut disc_rx } = inbox;
        self.emit(CoreEvent::Ready {
            identity: self.identity.public(),
            settings: self.settings.clone(),
            secret_backend: self.manager.backend_name(),
            app_version: self.config.app_version.clone(),
        });
        self.emit(CoreEvent::Engines(self.resolved_engines().status()));
        self.emit_models();
        self.emit_history();
        self.emit(CoreEvent::Dictionary(self.dictionary.entries().to_vec()));
        self.emit(CoreEvent::Rules(self.rules.rules().to_vec()));
        self.emit(CoreEvent::Scenes(self.scenes.scenes().to_vec()));
        self.emit(CoreEvent::Presets(self.presets.presets().to_vec()));
        self.emit_sent_texts();
        for notice in std::mem::take(&mut self.startup_notices) {
            self.emit(CoreEvent::Error(notice));
        }
        self.emit_devices();
        self.start_host().await;
        self.start_discovery();
        let mut ticker = tokio::time::interval(self.config.tick);
        loop {
            tokio::select! {
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(CoreCommand::Shutdown) | None => break,
                        Some(cmd) => self.handle_command(cmd).await,
                    }
                }
                Some((id, ev)) = link_rx.recv() => self.handle_link_event(id, ev).await,
                Some(ev) = dict_rx.recv() => {
                    let effects = self.dictation.on_internal(ev);
                    self.apply_dictation(effects);
                }
                Some(progress) = model_rx.recv() => self.on_model_progress(progress),
                Some(timer) = act_rx.recv() => self.on_activation_timer(timer),
                Some(event) = phone_rx.recv() => self.on_phone_event(event),
                Some(probed) = check_rx.recv() => self.on_probed(probed),
                Some(seen) = disc_rx.recv() => self.on_discovery(seen),
                _ = ticker.tick() => self.tick().await,
            }
            // A rename or a pairing that started or ended changes what the LAN hears; a device
            // trusted or forgotten changes the nearby list.
            self.refresh_announcement();
            if self.lan_running() {
                self.emit_nearby();
            }
            // Take messages queued by the handler above (docs/dictation.md §20).
            if !self.take_outbox.is_empty() {
                self.flush_takes().await;
            }
        }
        for (_, d) in self.downloads.drain() {
            d.cancel.cancel();
            d.task.abort();
        }
        self.activation.take_deferred();
        self.activation.disarm_capture();
        if let Some(t) = self.activation.grace.take() {
            t.abort();
        }
        self.stop_discovery();
        // The LAN host stops first: once a peer sees this device go offline (its relay or direct
        // link closed), the LAN address it was told about must not answer any more.
        if let Some(h) = self.host.take() {
            h.host.shutdown().await;
            h.link.close().await;
        }
        if let Some((l, _)) = self.relay.take() {
            l.close().await;
        }
        for (_, d) in self.dials.drain() {
            d.link.close().await;
        }
    }

    async fn handle_command(&mut self, cmd: CoreCommand) {
        let result = match cmd {
            CoreCommand::StartPairing => self.start_pairing().await,
            CoreCommand::JoinWithCode(code) => self.join(JoinSpec::Code(code)).await,
            CoreCommand::JoinWithTicket(uri) => self.join(JoinSpec::Ticket(uri)).await,
            CoreCommand::ConfirmPairing => self.step_pairing(Event::UserConfirm).await,
            CoreCommand::RejectPairing => self.step_pairing(Event::UserReject).await,
            CoreCommand::CancelPairing => self.step_pairing(Event::Cancel).await,
            CoreCommand::ResetPairing => self.reset_pairing().await,
            CoreCommand::ForgetDevice(key) => self.forget(key, true).await,
            CoreCommand::CheckConnectivity => self.check_connectivity().await,
            CoreCommand::RenameDevice(name) => self.rename(name).await,
            CoreCommand::SetRelay { url, enabled } => self.set_relay(url, enabled),
            CoreCommand::SetHotkey(text) => self.set_hotkey(&text),
            CoreCommand::SetEditHotkey(text) => self.set_edit_hotkey(text.as_deref()),
            CoreCommand::SetSoloKey(key) => self.set_solo_key(key),
            CoreCommand::SetMicrophone(device) => self.set_microphone(device),
            CoreCommand::SetTheme { theme, follow_system } => {
                self.settings.theme = theme;
                self.settings.follow_system_theme = follow_system;
                self.save_settings()
            }
            CoreCommand::SetLocale(locale) => {
                self.settings.locale = locale;
                self.save_settings()
            }
            CoreCommand::SetAutoUpdate(enabled) => {
                self.settings.auto_update = enabled;
                self.save_settings()
            }
            CoreCommand::SetHistory(history) => self.set_history(history),
            CoreCommand::SetOverlay(placement) => {
                self.settings.overlay = placement;
                self.save_settings()
            }
            CoreCommand::SendText { to, body } => self.send_text(to, body).await,
            CoreCommand::PhoneTakeStart { to } => self.phone_take_start(to),
            CoreCommand::PhoneTextSend { to, body, source } => self.phone_text_send(to, body, source),
            CoreCommand::SentTextsClear => {
                self.sent_texts_clear();
                Ok(())
            }
            CoreCommand::PasteText { request_id, text, target } => {
                self.paste_text(request_id, text, target);
                Ok(())
            }
            CoreCommand::SetLanDiscovery(enabled) => self.set_lan_discovery(enabled),
            CoreCommand::SetPairingAlwaysOn(enabled) => self.set_pairing_always_on(enabled).await,
            CoreCommand::PairingJoinNearby(fingerprint) => self.join_nearby(&fingerprint).await,
            CoreCommand::PhoneTakeStop => self.phone_take_stop(),
            CoreCommand::PhoneTakeCancel => self.phone_take_cancel(),
            CoreCommand::RefreshDevices => {
                self.emit_devices();
                self.resend_nearby();
                Ok(())
            }
            CoreCommand::DictationStart => self.dictation_start(),
            CoreCommand::DictationStop => self.dictation_stop(),
            CoreCommand::DictationCancel => self.dictation_cancel(),
            CoreCommand::HotkeyEdge { pressed, at_ms, source, purpose, chorded } => self.hotkey_edge(Edge { pressed, at_ms, source }, purpose, chorded),
            CoreCommand::SetActivation { activation, hold_threshold_ms, extra_recording_ms } => {
                self.set_activation(activation, hold_threshold_ms, extra_recording_ms)
            }
            CoreCommand::SetEngines(engines) => self.set_engines(engines),
            CoreCommand::SetProviderKey { provider, kind, value } => self.set_provider_key(provider, kind, value),
            CoreCommand::ProbeProvider { provider, kind, base_url, key } => self.probe_provider(provider, kind, base_url, key),
            CoreCommand::HistoryDelete(id) => self.history.delete(id).map(|_| self.emit_history()),
            CoreCommand::HistoryClear => self.history.clear().map(|()| self.emit_history()),
            CoreCommand::HistoryStar(id, starred) => self.history.star(id, starred).map(|_| self.emit_history()),
            CoreCommand::ModelDownload(id) => self.model_download(&id),
            CoreCommand::ModelCancel(id) => self.model_cancel(&id),
            CoreCommand::ModelRemove(id) => self.model_remove(&id),
            CoreCommand::DictionaryAdd { draft, source } => {
                let result = self.dictionary.add(&draft, source, now_ms()).map(drop);
                self.dictionary_changed(result)
            }
            CoreCommand::DictionaryUpdate { id, draft } => {
                let result = self.dictionary.update(id, &draft, now_ms());
                self.dictionary_changed(result)
            }
            CoreCommand::DictionaryRemove(id) => {
                let result = self.dictionary.remove(id);
                self.dictionary_changed(result)
            }
            CoreCommand::DictionaryReorder(ids) => {
                let result = self.dictionary.reorder(&ids);
                self.dictionary_changed(result)
            }
            CoreCommand::RuleAdd(draft) => {
                let result = self.rules.add(&draft, now_ms()).map(drop);
                self.rules_changed(result)
            }
            CoreCommand::RuleUpdate { id, draft } => {
                let result = self.rules.update(id, &draft, now_ms());
                self.rules_changed(result)
            }
            CoreCommand::RuleRemove(id) => {
                let result = self.rules.remove(id);
                self.rules_changed(result)
            }
            CoreCommand::RuleReorder(ids) => {
                let result = self.rules.reorder(&ids);
                self.rules_changed(result)
            }
            CoreCommand::RulesImport { rules, mode } => {
                let result = self.rules.import(&rules, mode, now_ms());
                self.rules_changed(result)
            }
            CoreCommand::SceneAdd(draft) => {
                let result = self.scenes.add(&draft, now_ms()).map(drop);
                self.scenes_changed(result)
            }
            CoreCommand::SceneUpdate { id, draft } => {
                let result = self.scenes.update(id, &draft, now_ms());
                self.scenes_changed(result)
            }
            CoreCommand::SceneRemove(id) => {
                let result = self.scenes.remove(id);
                self.scenes_changed(result)
            }
            CoreCommand::SceneReorder(ids) => {
                let result = self.scenes.reorder(&ids);
                self.scenes_changed(result)
            }
            CoreCommand::PresetAdd(draft) => {
                let result = self.presets.add(&draft, now_ms()).map(drop);
                self.presets_changed(result)
            }
            CoreCommand::PresetUpdate { id, draft } => {
                let result = self.presets.update(id, &draft, now_ms());
                self.presets_changed(result)
            }
            CoreCommand::PresetRemove(id) => {
                let result = self.presets.remove(id);
                self.presets_changed(result)
            }
            CoreCommand::PresetTry { id, trial, text } => {
                self.try_preset(id, &trial, text);
                Ok(())
            }
            CoreCommand::SetContextSharing(sharing) => {
                self.settings.context_sharing = sharing;
                self.dictation.set_context_sharing(sharing);
                self.save_settings()
            }
            CoreCommand::Shutdown => Ok(()),
        };
        if let Err(e) = result {
            tracing::warn!(error = %e, "command failed");
            self.emit(CoreEvent::Error(e.to_string()));
        }
    }

    // ---------------- dictation / engines / history ----------------

    fn resolved_engines(&self) -> ResolvedEngines {
        ResolvedEngines::resolve_with_models(&self.settings.engines, &self.user_secrets, &self.built_in, &self.library)
    }

    /// Rebuild the clients from the current settings + secrets and tell the UI.
    fn reconfigure_engines(&mut self) {
        let resolved = self.resolved_engines();
        self.dictation.configure(&resolved);
        self.emit(CoreEvent::Engines(resolved.status()));
    }

    /// The catalogue id the settings select in local mode.
    fn selected_local_model(engines: &EngineSettings) -> String {
        engines.local_model.as_deref().map(str::trim).filter(|s| !s.is_empty()).unwrap_or(DEFAULT_LOCAL_MODEL_ID).to_owned()
    }

    fn set_engines(&mut self, engines: EngineSettings) -> Result<(), CoreError> {
        if !engines.asr_provider.spec().offers(ServiceKind::Asr) {
            return Err(CoreError::Invalid(format!("asr_provider: {} 不提供语音识别", engines.asr_provider.as_str())));
        }
        if !engines.llm_provider.spec().offers(ServiceKind::Llm) {
            return Err(CoreError::Invalid(format!("llm_provider: {} 不提供 AI 润色", engines.llm_provider.as_str())));
        }
        for (provider, choice) in &engines.providers {
            for kind in [ServiceKind::Asr, ServiceKind::Llm] {
                if let Some(url) = choice.url(kind) {
                    check_http_url(url).map_err(|e| CoreError::Invalid(format!("{}.{}_url: {e}", provider.as_str(), kind_name(kind))))?;
                }
            }
        }
        if let Some(threads) = engines.local_threads
            && !(1..=MAX_LOCAL_THREADS).contains(&threads)
        {
            return Err(CoreError::Invalid(format!("local_threads: 1–{MAX_LOCAL_THREADS}")));
        }
        if engines.asr_provider == ProviderId::Local {
            // The model has to exist in the catalogue; it need not be installed yet (the UI then
            // shows "模型未下载" and the download button).
            let id = Self::selected_local_model(&engines);
            if self.models.is_none() {
                return Err(CoreError::Invalid("local_model: 本地模型不可用".into()));
            }
            if !self.library.iter().any(|m| m.id == id) {
                return Err(CoreError::Invalid(format!("local_model: 目录中没有 {id}")));
            }
        }
        self.settings.engines = engines;
        self.settings_store.save(&self.settings)?;
        self.emit(CoreEvent::Settings(self.settings.clone()));
        self.reconfigure_engines();
        // `active` follows the selection.
        self.emit_models();
        Ok(())
    }

    /// `DictationStart`, refused up front in local mode when the selected model is not installed
    /// (the home button is disabled for the same reason; the hotkey still gets an honest answer).
    fn dictation_start(&mut self) -> Result<(), CoreError> {
        self.check_recogniser()?;
        self.dictation.start().map(|fx| self.apply_dictation(fx)).map_err(CoreError::from)
    }

    /// The local-model refusal of [`Runtime::dictation_start`], shared by the edit take (its
    /// instruction is recognised by the same engine).
    fn check_recogniser(&self) -> Result<(), CoreError> {
        let resolved = self.resolved_engines();
        match resolved.asr_issue {
            None => Ok(()),
            Some(crate::engines::EngineIssue::ModelNotInstalled) => {
                let name = resolved.local_model.map(|m| m.name).unwrap_or_default();
                Err(CoreError::Dictation(DictationError::Asr(format!("本地模型未下载：{name}"))))
            }
            Some(issue) => Err(CoreError::Dictation(DictationError::Asr(issue.message(ServiceKind::Asr).to_owned()))),
        }
    }

    /// A voice edit take (docs/dictation.md §19) started by the edit key (or `voltip
    /// --edit-toggle`): the microphone opens for the instruction and the selection is copied — at
    /// once where the platform allows it and for a keyless (CLI) edge, otherwise after the key is up
    /// ([`Runtime::edit_key_edge`]). A key edge passes the edit hotkey's modifiers along: the user
    /// may still hold them when the copy chord goes out.
    fn edit_start(&mut self, source: EdgeSource) -> Result<(), CoreError> {
        self.check_recogniser()?;
        let held = match source {
            EdgeSource::Cli => Vec::new(),
            EdgeSource::Hotkey | EdgeSource::Ui => {
                self.settings.edit_hotkey.as_deref().and_then(|h| crate::Hotkey::parse(h).ok()).map(|h| h.modifiers).unwrap_or_default()
            }
        };
        let fx = self.dictation.start_edit(held)?;
        self.apply_dictation(fx);
        if source == EdgeSource::Cli || self.dictation.selection_timing() == SelectionTiming::AtPress {
            self.dictation.capture_selection();
        }
        Ok(())
    }

    // ---------------- activation (docs/dictation.md §13) ----------------

    /// `DictationStop`: with `extra_recording_ms > 0` the microphone stays open that much longer
    /// (trailing syllables) and closes from [`Runtime::on_activation_timer`]; a second stop inside
    /// the window closes it at once. Otherwise the engine stops now.
    fn dictation_stop(&mut self) -> Result<(), CoreError> {
        if self.activation.take_deferred() {
            return self.stop_now();
        }
        let extra = u64::from(self.settings.extra_recording_ms);
        if extra == 0 || !matches!(self.dictation.status().phase, DictationPhase::Listening { .. }) {
            return self.stop_now();
        }
        let session = self.dictation.status().session;
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let tx = self.act_tx.clone();
        let deadline = Instant::now() + Duration::from_millis(extra);
        let task = tokio::spawn(async move {
            // Polled in short slices so a cancel lands within one slice, not at the deadline.
            loop {
                if flag.load(Ordering::SeqCst) {
                    return;
                }
                let now = Instant::now();
                if now >= deadline {
                    break;
                }
                tokio::time::sleep((deadline - now).min(EXTRA_RECORDING_POLL)).await;
            }
            let _ = tx.send(ActivationTimer::ExtraOver { session }).await;
        });
        self.activation.deferred_stop = Some(DeferredStop { session, cancel, task });
        tracing::info!(session, extra_ms = extra, "stop deferred for the extra recording window");
        Ok(())
    }

    fn stop_now(&mut self) -> Result<(), CoreError> {
        self.dictation.stop().map(|fx| self.apply_dictation(fx)).map_err(CoreError::from)
    }

    /// `DictationCancel`: a stop waiting out the extra window is dropped with the recording.
    fn dictation_cancel(&mut self) -> Result<(), CoreError> {
        self.activation.take_deferred();
        self.dictation.cancel().map(|fx| self.apply_dictation(fx)).map_err(CoreError::from)
    }

    /// The engine's phase as the machine sees it: the extra-recording window counts as processing
    /// (the run is over for the key; a press now is pending, not a stop of a stop).
    fn phase_hint(&self) -> PhaseHint {
        if self.activation.deferred_stop.is_some() {
            return PhaseHint::Processing;
        }
        PhaseHint::from(&self.dictation.status().phase)
    }

    /// The phase as `purpose`'s machine sees it (docs/dictation.md §19): the engine's phase while the
    /// take is of that kind or nothing runs; `None` while a take of the other kind is listening or
    /// processing — its key's edges are dropped then, and its machine is kept inactive.
    fn phase_hint_for(&self, purpose: TakeKind) -> Option<PhaseHint> {
        let hint = self.phase_hint();
        (hint == PhaseHint::Idle || self.dictation.status().kind == purpose).then_some(hint)
    }

    fn hotkey_edge(&mut self, edge: Edge, purpose: TakeKind, chorded: bool) -> Result<(), CoreError> {
        let Some(phase) = self.phase_hint_for(purpose) else {
            tracing::info!(purpose = purpose.as_str(), pressed = edge.pressed, "hotkey edge dropped: a take of the other kind is running");
            return Ok(());
        };
        if chorded {
            let intents = self.activation.machine(purpose).chorded(phase);
            tracing::info!(purpose = purpose.as_str(), ?intents, "the trigger key became a chord");
            return self.apply_intents(intents, purpose, edge.source);
        }
        let intents = self.activation.machine(purpose).feed(edge, phase);
        self.arm_grace();
        let result = self.apply_intents(intents, purpose, edge.source);
        if purpose == TakeKind::Edit {
            self.edit_key_edge(edge);
        }
        result
    }

    /// The copy side of an edit key edge (docs/dictation.md §19): with `AfterKeyUp` the copy waits
    /// until the key is up — a release arms a `release_grace_ms` timer, a press inside the window
    /// (auto-repeat) disarms it. Nothing to do at the press (or for a CLI edge) otherwise: the take
    /// copied when it started.
    fn edit_key_edge(&mut self, edge: Edge) {
        let status = self.dictation.status();
        let running = matches!(status.phase, DictationPhase::Listening { .. } | DictationPhase::Processing { .. });
        if status.kind != TakeKind::Edit || !running {
            self.activation.disarm_capture();
            return;
        }
        if edge.source == EdgeSource::Cli || self.dictation.selection_timing() == SelectionTiming::AtPress {
            return;
        }
        self.activation.disarm_capture();
        if edge.pressed {
            return;
        }
        let session = status.session;
        let wait = Duration::from_millis(u64::from(self.activation.edit.config().release_grace_ms) + 1);
        let tx = self.act_tx.clone();
        self.activation.capture = Some(tokio::spawn(async move {
            tokio::time::sleep(wait).await;
            let _ = tx.send(ActivationTimer::CaptureKeyUp { session }).await;
        }));
    }

    fn set_activation(&mut self, activation: Activation, hold_threshold_ms: u32, extra_recording_ms: u32) -> Result<(), CoreError> {
        if hold_threshold_ms > MAX_ACTIVATION_MS || extra_recording_ms > MAX_ACTIVATION_MS {
            return Err(CoreError::Invalid(format!("activation: 时长不能超过 {MAX_ACTIVATION_MS}")));
        }
        self.settings.activation = activation;
        self.settings.hold_threshold_ms = hold_threshold_ms;
        self.settings.extra_recording_ms = extra_recording_ms;
        self.save_settings()?;
        let config = ActivationConfig::from(&self.settings);
        self.activation.dictate.set_config(config);
        self.activation.edit.set_config(config);
        if self.activation.locked {
            self.activation.locked = false;
            self.emit_dictation_status();
        }
        Ok(())
    }

    /// Run `purpose`'s machine intents against the engine: a start is a dictation or an edit take;
    /// a failed start rolls that machine back. `source` is where the edge came from (an edit take
    /// started by a key copies with the key's modifiers released, §19).
    fn apply_intents(&mut self, intents: Vec<Intent>, purpose: TakeKind, source: EdgeSource) -> Result<(), CoreError> {
        for intent in intents {
            match intent {
                Intent::Start => {
                    let started = match purpose {
                        TakeKind::Dictation => self.dictation_start(),
                        TakeKind::Edit => self.edit_start(source),
                    };
                    if let Err(e) = started {
                        self.activation.machine(purpose).on_start_failed();
                        return Err(e);
                    }
                }
                Intent::Stop => self.dictation_stop()?,
                Intent::Cancel => self.dictation_cancel()?,
                Intent::Lock => {
                    if !self.activation.locked {
                        self.activation.locked = true;
                        tracing::info!(session = self.dictation.status().session, "dictation locked");
                        self.emit_dictation_status();
                    }
                }
                Intent::Ignore => {}
            }
        }
        Ok(())
    }

    /// Re-emit the engine's current status (with the lock flag stamped on).
    fn emit_dictation_status(&mut self) {
        let status = self.dictation.status().clone();
        self.apply_dictation(vec![Effect::Status(status)]);
    }

    /// (Re)arm the grace timer for the earliest deadline of the two machines, if any.
    fn arm_grace(&mut self) {
        if let Some(t) = self.activation.grace.take() {
            t.abort();
        }
        let Some(deadline) = [self.activation.dictate.deadline_ms(), self.activation.edit.deadline_ms()].into_iter().flatten().min() else { return };
        // `+ 1`: the poll must see `now >= deadline` on its own clock read.
        let wait = Duration::from_millis(deadline.saturating_sub(now_ms()) + 1);
        let tx = self.act_tx.clone();
        self.activation.grace = Some(tokio::spawn(async move {
            tokio::time::sleep(wait).await;
            let _ = tx.send(ActivationTimer::Grace).await;
        }));
    }

    fn on_activation_timer(&mut self, timer: ActivationTimer) {
        let result = match timer {
            ActivationTimer::Grace => {
                self.activation.grace = None;
                let mut result = Ok(());
                for purpose in [TakeKind::Dictation, TakeKind::Edit] {
                    // A machine whose kind is not running sees `Processing`: its release resolves to nothing.
                    let phase = self.phase_hint_for(purpose).unwrap_or(PhaseHint::Processing);
                    let intents = self.activation.machine(purpose).poll(now_ms(), phase);
                    if let Err(e) = self.apply_intents(intents, purpose, EdgeSource::Hotkey) {
                        result = Err(e);
                    }
                }
                self.arm_grace();
                result
            }
            ActivationTimer::CaptureKeyUp { session } => {
                self.activation.capture = None;
                if self.dictation.status().session == session {
                    self.dictation.capture_selection();
                }
                Ok(())
            }
            ActivationTimer::ExtraOver { session } => {
                let current = self.activation.deferred_stop.as_ref().map(|d| d.session);
                if current != Some(session) {
                    return;
                }
                self.activation.take_deferred();
                self.stop_now().and_then(|()| {
                    // The run may have ended by itself inside the window (auto-stop, device error):
                    // the stop was then a no-op and a press remembered meanwhile must still fire.
                    let mut result = Ok(());
                    for purpose in [TakeKind::Dictation, TakeKind::Edit] {
                        let phase = self.phase_hint_for(purpose).unwrap_or(PhaseHint::Processing);
                        let intents = self.activation.machine(purpose).on_phase(phase);
                        if let Err(e) = self.apply_intents(intents, purpose, EdgeSource::Hotkey) {
                            result = Err(e);
                        }
                    }
                    result
                })
            }
        };
        if let Err(e) = result {
            tracing::warn!(error = %e, "activation intent failed");
            self.emit(CoreEvent::Error(e.to_string()));
        }
    }

    // ---------------- local model library ----------------

    /// The library as the UI sees it: `active` marks the model the settings select in local mode.
    fn models_view(&self) -> Vec<ModelState> {
        let active = self.resolved_engines().is_local().then(|| Self::selected_local_model(&self.settings.engines));
        self.library
            .iter()
            .cloned()
            .map(|mut m| {
                m.active = active.as_deref() == Some(m.id.as_str());
                m
            })
            .collect()
    }

    fn emit_models(&self) {
        self.emit(CoreEvent::Models(self.models_view()));
    }

    /// Re-read the library from disk and tell the UI. Running downloads keep their progress state
    /// and a failure keeps its reason (the disk only says "not installed" for both); the engines are
    /// re-resolved too, since `local_ready` may have changed.
    fn rescan_models(&mut self) {
        let Some(manager) = &self.models else { return };
        let mut library = manager.scan();
        for m in &mut library {
            let Some(current) = self.library.iter().find(|c| c.id == m.id) else { continue };
            let in_flight = self.downloads.contains_key(&m.id);
            let failed = matches!(current.state, ModelInstallState::Failed { .. }) && m.state == ModelInstallState::NotInstalled;
            if in_flight || failed {
                m.state = current.state.clone();
            }
        }
        self.library = library;
        self.reconfigure_engines();
        self.emit_models();
    }

    fn set_model_state(&mut self, id: &str, state: ModelInstallState) {
        if let Some(m) = self.library.iter_mut().find(|m| m.id == id) {
            m.state = state;
        }
        self.emit_models();
    }

    fn model_entry(&self, id: &str) -> Result<&ModelState, CoreError> {
        if self.models.is_none() {
            return Err(CoreError::Invalid("models: 本地模型不可用".into()));
        }
        self.library.iter().find(|m| m.id == id).ok_or_else(|| CoreError::Invalid(format!("models: 目录中没有 {id}")))
    }

    fn model_download(&mut self, id: &str) -> Result<(), CoreError> {
        let entry = self.model_entry(id)?;
        if entry.state.is_installed() {
            return Err(CoreError::Invalid(format!("models: {id} 已安装")));
        }
        if self.downloads.contains_key(id) {
            return Err(CoreError::Invalid(format!("models: {id} 正在下载")));
        }
        let Some(manager) = self.models.clone() else { return Err(CoreError::Invalid("models: 本地模型不可用".into())) };
        let cancel = CancelToken::new();
        let total = entry.size_bytes;
        let tx = self.model_tx.clone();
        let task_id = id.to_owned();
        let task_cancel = cancel.clone();
        let task = tokio::spawn(async move {
            let id = task_id;
            // Progress throttle: a `Downloading` report goes through when ≥ 250 ms or ≥ 1 MiB have
            // passed since the last one; state changes (`Verifying`) always go through.
            let last = Arc::new(parking_lot::Mutex::new((Instant::now() - MODEL_PROGRESS_INTERVAL, 0u64)));
            let sink_tx = tx.clone();
            let sink_id = id.clone();
            let progress: crate::models::ProgressSink = Arc::new(move |state: ModelInstallState| {
                if let ModelInstallState::Downloading { received, .. } = &state {
                    let mut last = last.lock();
                    let now = Instant::now();
                    if now.duration_since(last.0) < MODEL_PROGRESS_INTERVAL && received.saturating_sub(last.1) < MODEL_PROGRESS_BYTES {
                        return;
                    }
                    *last = (now, *received);
                }
                let _ = sink_tx.try_send(ModelProgress::State { id: sink_id.clone(), state });
            });
            let result = manager.download(&id, progress, task_cancel.clone()).await;
            let _ = tx.send(ModelProgress::Finished { id, result, cancelled: task_cancel.is_cancelled() }).await;
        });
        self.downloads.insert(id.to_owned(), Download { cancel, task });
        tracing::info!(model = id, "model download started");
        self.set_model_state(id, ModelInstallState::Downloading { received: 0, total, file: String::new() });
        Ok(())
    }

    fn model_cancel(&mut self, id: &str) -> Result<(), CoreError> {
        self.model_entry(id)?;
        let Some(download) = self.downloads.get(id) else { return Err(CoreError::Invalid(format!("models: {id} 没有在下载"))) };
        download.cancel.cancel();
        tracing::info!(model = id, "model download cancel requested");
        Ok(())
    }

    fn model_remove(&mut self, id: &str) -> Result<(), CoreError> {
        self.model_entry(id)?;
        if self.downloads.contains_key(id) {
            return Err(CoreError::Invalid(format!("models: {id} 正在下载，先取消")));
        }
        let Some(manager) = &self.models else { return Err(CoreError::Invalid("models: 本地模型不可用".into())) };
        manager.remove(id).map_err(|e| CoreError::Invalid(format!("models: {e}")))?;
        tracing::info!(model = id, "model removed");
        // A remembered failure goes with the directory.
        if let Some(m) = self.library.iter_mut().find(|m| m.id == id) {
            m.state = ModelInstallState::NotInstalled;
        }
        self.rescan_models();
        Ok(())
    }

    fn on_model_progress(&mut self, progress: ModelProgress) {
        match progress {
            ModelProgress::State { id, state } => {
                if self.downloads.contains_key(&id) {
                    self.set_model_state(&id, state);
                }
            }
            ModelProgress::Finished { id, result, cancelled } => {
                if let Some(d) = self.downloads.remove(&id) {
                    d.task.abort();
                }
                // The disk is the truth after a finished download (`Installed` comes from the
                // manifest the store just wrote); only a failure carries information the disk
                // does not have, its reason.
                let state = match result {
                    Ok(state) => {
                        tracing::info!(model = %id, "model installed");
                        state
                    }
                    Err(e) if cancelled => {
                        tracing::info!(model = %id, reason = %e, "model download cancelled");
                        ModelInstallState::NotInstalled
                    }
                    Err(e) => {
                        tracing::warn!(model = %id, error = %e, "model download failed");
                        ModelInstallState::Failed { message: e }
                    }
                };
                if let Some(m) = self.library.iter_mut().find(|m| m.id == id) {
                    m.state = state;
                }
                self.rescan_models();
            }
        }
    }

    fn set_provider_key(&mut self, provider: ProviderId, kind: ServiceKind, value: Option<String>) -> Result<(), CoreError> {
        let Some(entry) = key_entry(provider, kind) else {
            return Err(CoreError::Invalid(format!("{}: 此服务商不需要密钥", provider.as_str())));
        };
        let value = value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
        match &value {
            Some(v) => self.secrets.set(entry, v.as_bytes())?,
            None => self.secrets.delete(entry)?,
        }
        self.user_secrets.set_entry(entry, value);
        tracing::info!(secret = entry, "provider key updated");
        self.reconfigure_engines();
        Ok(())
    }

    /// `ProbeProvider`: resolve the endpoint and key the form would use (the draft over the saved
    /// values over the preset; the built-in service with its own key only), then list the models on
    /// a task and answer with `ProviderProbe`. Problems found before any request (no URL, no key,
    /// no such service, no probe on this shell) are answered at once.
    fn probe_provider(&mut self, provider: ProviderId, kind: ServiceKind, base_url: Option<String>, key: Option<String>) -> Result<(), CoreError> {
        let refused = |reason| ProbeReport { provider, kind, outcome: ProbeOutcome::Failed { reason, status: None } };
        let (url, key) = match self.probe_target(provider, kind, base_url, key) {
            Ok(target) => target,
            Err(reason) => {
                self.emit(CoreEvent::ProviderProbe(refused(reason)));
                return Ok(());
            }
        };
        let Some(probe) = self.service_probe.clone() else {
            self.emit(CoreEvent::ProviderProbe(refused(ProbeFailure::Unsupported)));
            return Ok(());
        };
        let evt = self.evt.clone();
        tokio::spawn(async move {
            let started = Instant::now();
            let outcome = match probe.list_models(&url, key.as_deref()).await {
                Ok(mut models) => {
                    models.sort_unstable();
                    models.dedup();
                    ProbeOutcome::Ok { models, latency_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX) }
                }
                Err(ProbeError { reason, status }) => ProbeOutcome::Failed { reason, status },
            };
            if evt.send(CoreEvent::ProviderProbe(ProbeReport { provider, kind, outcome })).await.is_err() {
                tracing::debug!("provider probe answered after the UI went away");
            }
        });
        Ok(())
    }

    /// The `(base URL, key)` a probe of `provider`'s `kind` service sends to.
    fn probe_target(
        &self,
        provider: ProviderId,
        kind: ServiceKind,
        base_url: Option<String>,
        key: Option<String>,
    ) -> Result<(String, Option<String>), ProbeFailure> {
        let spec = provider.spec();
        let Some(preset) = spec.service(kind).filter(|_| provider != ProviderId::Local) else { return Err(ProbeFailure::Unsupported) };
        if provider == ProviderId::Builtin {
            let (url, key) = match kind {
                ServiceKind::Asr => (self.built_in.asr_url, self.built_in.asr_token),
                ServiceKind::Llm => (self.built_in.refine_url, self.built_in.refine_api_key),
            };
            return url.map(|u| (u.to_owned(), key.map(str::to_owned))).ok_or(ProbeFailure::Unsupported);
        }
        let draft = |v: Option<String>| v.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
        let saved = self.settings.engines.provider(provider);
        let url = draft(base_url)
            .or_else(|| saved.url(kind).map(str::to_owned))
            .or_else(|| (!preset.base_url.is_empty()).then(|| preset.base_url.to_owned()))
            .ok_or(ProbeFailure::InvalidUrl)?;
        check_http_url(&url).map_err(|_| ProbeFailure::InvalidUrl)?;
        let key = draft(key).or_else(|| self.user_secrets.get(provider, kind).map(str::to_owned));
        if spec.key == crate::providers::KeyPolicy::Required && key.is_none() {
            return Err(ProbeFailure::KeyMissing);
        }
        Ok((url, key))
    }

    fn emit_history(&self) {
        self.emit(CoreEvent::History(self.history.entries().to_vec()));
    }

    // ---------------- dictionary and rules (docs/dictation.md §16) ----------------

    /// The engine's next runs take the new lists; the running one keeps its snapshot.
    fn recompile_vocabulary(&mut self) {
        let vocabulary = Vocabulary::compile(self.dictionary.entries(), self.rules.rules());
        tracing::debug!(?vocabulary, "vocabulary recompiled");
        self.dictation.set_vocabulary(Arc::new(vocabulary));
    }

    /// After a dictionary command: on success tell the UI and recompile; a refusal changed nothing.
    fn dictionary_changed(&mut self, result: Result<(), crate::vocabulary::VocabularyError>) -> Result<(), CoreError> {
        result?;
        self.emit(CoreEvent::Dictionary(self.dictionary.entries().to_vec()));
        self.recompile_vocabulary();
        Ok(())
    }

    /// After a rule command: on success tell the UI and recompile; a refusal changed nothing.
    fn rules_changed(&mut self, result: Result<(), crate::vocabulary::VocabularyError>) -> Result<(), CoreError> {
        result?;
        self.emit(CoreEvent::Rules(self.rules.rules().to_vec()));
        self.recompile_vocabulary();
        Ok(())
    }

    // ---------------- scenes (docs/dictation.md §18) ----------------

    /// After a scene command: on success tell the UI and hand the engine the new list (the running
    /// take keeps its snapshot); a refusal changed nothing.
    fn scenes_changed(&mut self, result: Result<(), crate::scenes::SceneError>) -> Result<(), CoreError> {
        result?;
        let scenes = self.scenes.scenes().to_vec();
        self.dictation.set_scenes(Arc::new(scenes.clone()));
        self.emit(CoreEvent::Scenes(scenes));
        Ok(())
    }

    // ---------------- presets (docs/dictation.md §21) ----------------

    /// After a preset command: on success tell the UI and hand the engine the new list (the running
    /// take keeps its snapshot); a refusal changed nothing.
    fn presets_changed(&mut self, result: Result<(), crate::presets::PresetError>) -> Result<(), CoreError> {
        result?;
        let presets = self.presets.presets().to_vec();
        self.dictation.set_presets(Arc::new(presets.clone()));
        self.emit(CoreEvent::Presets(presets));
        Ok(())
    }

    /// 试一试: `text` through the current clean-up with the trial's preset and the engines'
    /// language, on a task; the answer is [`CoreEvent::PresetTry`] with `id`. Nothing is saved, and
    /// no history entry is written.
    fn try_preset(&mut self, id: u64, trial: &PresetTrial, text: String) {
        let Some(refiner) = self.dictation.refiner() else {
            self.emit(CoreEvent::PresetTry { id, outcome: PresetTryOutcome::Failed { reason: PRESET_TRY_UNCONFIGURED.to_owned() } });
            return;
        };
        let hints = RefineHints { preset: trial.resolve(self.presets.presets()), language: self.resolved_engines().language.clone(), ..RefineHints::default() };
        let evt = self.evt.clone();
        tokio::spawn(async move {
            let outcome = match refiner.refine(&text, &hints).await {
                Ok(out) => PresetTryOutcome::Ok { text: out.text, latency_ms: out.latency_ms, model: out.model },
                Err(e) => PresetTryOutcome::Failed { reason: e.to_string() },
            };
            if evt.send(CoreEvent::PresetTry { id, outcome }).await.is_err() {
                tracing::debug!("preset trial answered after the UI went away");
            }
        });
    }

    fn apply_dictation(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Status(mut status) => {
                    self.follow_remote_take(&mut status);
                    // The lock lives in the runtime; a phase that is not `Listening` ends it.
                    match &mut status.phase {
                        DictationPhase::Listening { locked, .. } => *locked = self.activation.locked,
                        _ => self.activation.locked = false,
                    }
                    let hint = if self.activation.deferred_stop.is_some() { PhaseHint::Processing } else { PhaseHint::from(&status.phase) };
                    let kind = status.kind;
                    self.emit(CoreEvent::Dictation(status));
                    // A phone's text waits for the take to end (docs/dictation.md §20.6).
                    if hint == PhaseHint::Idle {
                        self.deliver_next_text();
                    }
                    // A press remembered during processing starts once the phase is idle again. The
                    // machine of the kind that is not running sees `Processing` (docs/dictation.md §19).
                    for purpose in [TakeKind::Dictation, TakeKind::Edit] {
                        let seen = if hint == PhaseHint::Idle || kind == purpose { hint } else { PhaseHint::Processing };
                        let intents = self.activation.machine(purpose).on_phase(seen);
                        if let Err(e) = self.apply_intents(intents, purpose, EdgeSource::Hotkey) {
                            tracing::warn!(error = %e, "pending activation start failed");
                            self.emit(CoreEvent::Error(e.to_string()));
                        }
                    }
                }
                Effect::Record(mut entry) => {
                    // A phone's take is recorded as the phone's (docs/dictation.md §20.6).
                    if let Some(name) = self.remote_take_name() {
                        entry.origin = Some(crate::history::EntryOrigin { device: name, kind: crate::history::OriginKind::Take });
                    }
                    self.record_history(entry);
                }
            }
        }
    }

    /// Keep `entry` in the history, unless history is off (docs/dictation.md §4).
    fn record_history(&mut self, entry: crate::history::HistoryEntry) {
        if !self.settings.history.enabled {
            return;
        }
        match self.history.push(entry, self.settings.history.keep as usize) {
            Ok(()) => self.emit_history(),
            Err(e) => {
                tracing::warn!(error = %e, "history append failed");
                self.emit(CoreEvent::Error(e.to_string()));
            }
        }
    }

    // ---------------- pairing ----------------

    /// The pairing this device runs, if any.
    fn pairing_snapshot(&self) -> Option<Snapshot> {
        let now = Self::now();
        match &self.pairing {
            Pairing::None => None,
            Pairing::Initiator(i) => Some(i.snapshot(now)),
            Pairing::Responder(r) => Some(r.snapshot(now)),
        }
    }

    /// The state of the pairing this device runs, if any.
    fn pairing_state(&self) -> Option<PairingState> {
        self.pairing_snapshot().map(|s| s.state)
    }

    /// A finished pairing (trusted, expired, rejected, failed) is cleared before a new one starts:
    /// 再配一台, 重新开始 and a code typed over a failed one come straight from its end screen.
    async fn clear_finished_pairing(&mut self) -> Result<(), CoreError> {
        if self.pairing_state().is_some_and(PairingState::is_terminal) {
            self.reset_pairing().await?;
        }
        Ok(())
    }

    async fn start_pairing(&mut self) -> Result<(), CoreError> {
        self.clear_finished_pairing().await?;
        if !matches!(self.pairing, Pairing::None) {
            return Err(CoreError::Invalid("pairing: 已有配对正在进行，请先取消".into()));
        }
        if self.relay_connected() {
            self.pairing_link = Some(LinkId::Relay);
            return self.begin_initiator().await;
        }
        if self.host.is_none() {
            return Err(CoreError::Invalid("pairing: 未连接中继，局域网服务也未开启".into()));
        }
        self.pairing_link = Some(LinkId::Host);
        if self.link_connected(LinkId::Host) {
            self.begin_initiator().await
        } else {
            // The loopback link to our own host is still coming up (typically right after start).
            self.pending_start = Some(PendingStart::Initiator);
            Ok(())
        }
    }

    async fn begin_initiator(&mut self) -> Result<(), CoreError> {
        let relay_hint = match self.pairing_link {
            Some(LinkId::Relay) => self.relay.as_ref().map(|(_, e)| e.url().clone()),
            _ => None,
        };
        // The ticket always carries the LAN endpoint too, so a phone that scans it can come
        // straight over the local network later even when the pairing itself ran on the relay.
        let reach = Reachability { relay_hint, direct_hints: self.lan_hints() };
        let mut init = Initiator::new(self.identity.clone(), self.config.pairing_timeouts, reach);
        let actions = init.step(Event::Start, Self::now())?;
        self.pairing = Pairing::Initiator(Box::new(init));
        self.apply_actions(actions).await;
        Ok(())
    }

    async fn join(&mut self, spec: JoinSpec) -> Result<(), CoreError> {
        self.clear_finished_pairing().await?;
        if !matches!(self.pairing, Pairing::None) {
            return Err(CoreError::Invalid("pairing: 已有配对正在进行，请先取消".into()));
        }
        let method = match spec {
            JoinSpec::Code(text) => JoinMethod::Code(PairCode::parse_user_input(&text)?),
            JoinSpec::Ticket(uri) => JoinMethod::Ticket(PairingTicket::from_uri(&uri)?),
        };
        let ticket_hints = match &method {
            JoinMethod::Ticket(t) => t.direct_hints.clone(),
            JoinMethod::Code(_) => Vec::new(),
        };
        // A ticket without a relay hint names a session on the initiator's LAN host (it had no
        // relay): only a direct connection reaches it, whatever relay this device has.
        let lan_only = matches!(&method, JoinMethod::Ticket(t) if t.relay_hint.is_none() && !t.direct_hints.is_empty());
        if self.relay_connected() && !lan_only {
            self.pairing_link = Some(LinkId::Relay);
            self.pairing_peer_hints = ticket_hints;
            return self.begin_responder(method).await;
        }
        // No relay: a ticket may carry LAN hints; a code alone cannot be used.
        if matches!(method, JoinMethod::Code(_)) {
            return Err(CoreError::Invalid("pairing: 使用验证码配对需要连接中继，请改用扫码".into()));
        }
        let Some(hint) = ticket_hints.first().cloned() else {
            return Err(CoreError::Invalid("pairing: 此配对信息需要经过中继，但未配置中继".into()));
        };
        let link = self.dial(&hint, None)?;
        self.pairing_link = Some(link);
        self.pairing_peer_hints = ticket_hints;
        self.pending_start = Some(PendingStart::Responder(method));
        Ok(())
    }

    async fn begin_responder(&mut self, method: JoinMethod) -> Result<(), CoreError> {
        let now = Self::now();
        match Responder::new(self.identity.clone(), self.config.pairing_timeouts, method, &mut self.ledger, now) {
            Ok(mut resp) => {
                let actions = resp.step(Event::Start, now)?;
                self.pairing = Pairing::Responder(Box::new(resp));
                self.apply_actions(actions).await;
                Ok(())
            }
            Err(state) => {
                // Expired / replayed ticket: show the terminal state without touching the network.
                self.pairing_link = None;
                self.pairing_peer_hints.clear();
                let snap = Snapshot {
                    state,
                    session_id: None,
                    code: None,
                    ticket_uri: None,
                    expires_at: None,
                    remaining_secs: None,
                    safety_code: None,
                    peer: None,
                    local_confirmed: false,
                    peer_confirmed: false,
                };
                self.emit(CoreEvent::Pairing(snap));
                Ok(())
            }
        }
    }

    async fn step_pairing(&mut self, event: Event) -> Result<(), CoreError> {
        let now = Self::now();
        let actions = match &mut self.pairing {
            Pairing::None => return Err(CoreError::Invalid("pairing: 当前没有进行中的配对".into())),
            Pairing::Initiator(i) => i.step(event, now)?,
            Pairing::Responder(r) => r.step(event, now)?,
        };
        self.apply_actions(actions).await;
        Ok(())
    }

    /// Release the pairing session on whichever link carried it (if any). Idempotent.
    async fn leave_pairing_session(&mut self) -> Result<(), CoreError> {
        if let Some(link) = self.pairing_link
            && let Some(sid) = self.pairing_session_id()
            && !self.session_to_peer.contains_key(&(link, sid))
            && self.link_connected(link)
        {
            self.send_on(link, RelayFrame::Leave { version: ProtocolVersion::CURRENT, session_id: sid }).await?;
            self.pairing_session = None;
        }
        Ok(())
    }

    async fn reset_pairing(&mut self) -> Result<(), CoreError> {
        let _ = self.leave_pairing_session().await;
        self.pairing = Pairing::None;
        self.pairing_session = None;
        self.pending_start = None;
        self.pairing_peer_hints.clear();
        // A connection opened only to reach a ticket's LAN host is not needed any more.
        if let Some(LinkId::Dial(n)) = self.pairing_link.take()
            && self.dials.get(&n).is_some_and(|d| d.peer.is_none())
        {
            self.close_dial(n).await;
        }
        let snap = Snapshot {
            state: PairingState::Idle,
            session_id: None,
            code: None,
            ticket_uri: None,
            expires_at: None,
            remaining_secs: None,
            safety_code: None,
            peer: None,
            local_confirmed: false,
            peer_confirmed: false,
        };
        self.emit(CoreEvent::Pairing(snap));
        Ok(())
    }

    fn pairing_session_id(&self) -> Option<SessionId> {
        let now = Self::now();
        let live = match &self.pairing {
            Pairing::None => None,
            Pairing::Initiator(i) => i.snapshot(now).session_id,
            Pairing::Responder(r) => r.snapshot(now).session_id,
        };
        live.or(self.pairing_session)
    }

    async fn pairing_link_send(&self, frame: RelayFrame) -> Result<(), CoreError> {
        let Some(link) = self.pairing_link else { return Err(CoreError::Invalid("pairing: 配对连接已断开".into())) };
        self.send_on(link, frame).await
    }

    async fn apply_actions(&mut self, actions: Vec<Action>) {
        for action in actions {
            let result = match action {
                Action::SendRelay(frame) => self.pairing_link_send(frame).await,
                Action::SendPeer(bytes) => match self.pairing_session_id() {
                    Some(sid) => self.pairing_link_send(RelayFrame::forward(sid, bytes)).await,
                    None => Err(CoreError::Invalid("pairing: 当前没有进行中的配对".into())),
                },
                Action::Emit(snapshot) => {
                    if snapshot.session_id.is_some() {
                        self.pairing_session = snapshot.session_id;
                    }
                    self.emit(CoreEvent::Pairing(*snapshot));
                    Ok(())
                }
                Action::Trusted(established) => self.on_trusted(*established).await,
                Action::Close => self.leave_pairing_session().await,
            };
            if let Err(e) = result {
                tracing::warn!(error = %e, "pairing action failed");
                self.emit(CoreEvent::Error(e.to_string()));
            }
        }
    }

    async fn on_trusted(&mut self, est: voltip_pairing::Established) -> Result<(), CoreError> {
        let key = est.remote_static;
        let record = self.trusted.trust(&est.peer, key, Self::now().unix_secs)?;
        // The ticket told us where the initiator's LAN host is: remember it for the next time.
        let hints = std::mem::take(&mut self.pairing_peer_hints);
        if !hints.is_empty() {
            self.trusted.update_hints(&key, &hints)?;
        }
        tracing::info!(peer = %est.peer.name, "device trusted");
        self.emit(CoreEvent::Trusted(record));
        let backoff = self.config.direct_retry;
        self.peers.entry(key).or_insert_with(|| PeerState::new(backoff));
        // Long-lived presence and traffic move to the rendezvous channel on the same link; the
        // one-shot pairing session is released so the host's slot is free for the next pairing.
        if let Some(link) = self.pairing_link {
            if let LinkId::Dial(n) = link
                && let Some(d) = self.dials.get_mut(&n)
            {
                d.peer = Some(key);
            }
            self.attach(link, key).await;
            if self.link_connected(link) {
                self.send_on(link, RelayFrame::Leave { version: ProtocolVersion::CURRENT, session_id: est.session_id }).await?;
            }
        }
        // Be reachable on our own LAN host as well, so the peer can dial us later.
        if self.pairing_link != Some(LinkId::Host) && self.link_connected(LinkId::Host) {
            self.attach(LinkId::Host, key).await;
        }
        self.emit_devices();
        Ok(())
    }

    // ---------------- devices ----------------

    fn emit_devices(&self) {
        let views = self
            .trusted
            .list()
            .into_iter()
            .map(|d| {
                let connection = match self.peers.get(&d.public_key) {
                    Some(st) => summarize(&st.paths),
                    None => DeviceConnection::Offline,
                };
                DeviceView { device: d, connection }
            })
            .collect();
        self.emit(CoreEvent::Devices(views));
    }

    /// Drop `key`'s record, sessions and dials. With `notify` the peer is told first
    /// (`AppMessage::Unpair`) when it is online, so it forgets this device too; offline it keeps
    /// its record until the next time it tries to connect.
    async fn forget(&mut self, key: PublicKey, notify: bool) -> Result<(), CoreError> {
        if notify
            && self.trusted.get_by_key(&key).is_some()
            && let Err(e) = self.send_app(key, &AppMessage::unpair()).await
        {
            tracing::debug!(peer = %key.fingerprint(), error = %e, "unpair notice not sent");
        }
        self.trusted.forget(&key)?;
        if let Some(st) = self.peers.remove(&key) {
            for p in &st.paths {
                if let Some(sid) = p.session_id {
                    self.session_to_peer.remove(&(p.link, sid));
                }
            }
        }
        let dials: Vec<u64> = self.dials.iter().filter(|(_, d)| d.peer == Some(key)).map(|(n, _)| *n).collect();
        for n in dials {
            self.close_dial(n).await;
        }
        self.emit_devices();
        Ok(())
    }

    async fn rename(&mut self, name: String) -> Result<(), CoreError> {
        self.manager.rename(&mut self.identity, &name)?;
        self.emit(CoreEvent::Identity(self.identity.public()));
        let keys: Vec<PublicKey> = self.peers.keys().copied().collect();
        for key in keys {
            self.announce_self(key).await;
        }
        Ok(())
    }

    fn set_hotkey(&mut self, text: &str) -> Result<(), CoreError> {
        let hotkey = crate::Hotkey::parse(text)?;
        if let Some(edit) = self.settings.edit_hotkey.as_deref()
            && crate::Hotkey::same_chord(&hotkey.display(), edit)
        {
            return Err(CoreError::Invalid(format!("hotkey: {} 已用作「编辑选中文本」的快捷键", hotkey.display())));
        }
        self.settings.hotkey = hotkey.display();
        self.save_settings()
    }

    /// `SetEditHotkey` (docs/dictation.md §19): `None` switches voice edit off; a chord is validated
    /// like the dictation hotkey and refused when it is the dictation chord.
    fn set_edit_hotkey(&mut self, text: Option<&str>) -> Result<(), CoreError> {
        let chord = match text {
            None => None,
            Some(text) => {
                let hotkey = crate::Hotkey::parse(text)?;
                if crate::Hotkey::same_chord(&hotkey.display(), &self.settings.hotkey) {
                    return Err(CoreError::Invalid(format!("edit_hotkey: {} 已用作听写快捷键", hotkey.display())));
                }
                Some(hotkey.display())
            }
        };
        self.settings.edit_hotkey = chord;
        self.save_settings()
    }

    /// `SetSoloKey` (docs/dictation.md §13.1): every key is valid data; whether this machine can
    /// watch it is the shell's report (`HotkeyStatus.solo_error`).
    fn set_solo_key(&mut self, key: Option<crate::SoloKey>) -> Result<(), CoreError> {
        self.settings.solo_key = key;
        self.save_settings()
    }

    /// `SetMicrophone`: any non-empty id up to [`crate::settings::MAX_MICROPHONE_ID_BYTES`]
    /// (whether it is connected is the recorder's business at the next take; a missing device
    /// falls back to the default there). The engine records from it from the next take on.
    fn set_microphone(&mut self, device: Option<String>) -> Result<(), CoreError> {
        if let Some(id) = &device
            && (id.trim().is_empty() || id.len() > crate::settings::MAX_MICROPHONE_ID_BYTES)
        {
            return Err(CoreError::Invalid(format!("microphone: 麦克风标识须为 1–{} 字节，留空则使用系统默认输入", crate::settings::MAX_MICROPHONE_ID_BYTES)));
        }
        self.settings.microphone = device;
        self.dictation.set_microphone(self.settings.microphone.clone());
        self.save_settings()
    }

    /// Persist `self.settings` and tell the UI.
    fn save_settings(&mut self) -> Result<(), CoreError> {
        self.settings_store.save(&self.settings)?;
        self.emit(CoreEvent::Settings(self.settings.clone()));
        Ok(())
    }

    fn set_history(&mut self, history: crate::settings::HistorySettings) -> Result<(), CoreError> {
        let keep = history.keep as usize;
        if !(crate::history::MIN_KEEP..=crate::history::MAX_ENTRIES).contains(&keep) {
            return Err(CoreError::Invalid(format!("history.keep: {}–{}", crate::history::MIN_KEEP, crate::history::MAX_ENTRIES)));
        }
        self.settings.history = history;
        self.save_settings()?;
        if self.history.retain_newest(keep)? {
            self.emit_history();
        }
        Ok(())
    }

    fn set_relay(&mut self, url: Option<String>, enabled: bool) -> Result<(), CoreError> {
        if let Some(u) = &url {
            RelayEndpoint::parse(u)?;
        }
        self.settings.relay_url = url;
        self.settings.relay_enabled = enabled;
        self.settings_store.save(&self.settings)?;
        self.emit(CoreEvent::Settings(self.settings.clone()));
        self.connect_relay()?;
        self.emit_devices();
        Ok(())
    }

    async fn send_text(&mut self, to: PublicKey, body: String) -> Result<(), CoreError> {
        self.send_app(to, &AppMessage::text(body)).await
    }

    /// Tell `key` who we are and where our LAN host listens, over the best secure path.
    async fn announce_self(&mut self, key: PublicKey) {
        let msg = AppMessage::DeviceInfoUpdate { version: ProtocolVersion::CURRENT, device: self.identity.info(), direct_hints: self.lan_hints() };
        tracing::debug!(peer = %key.fingerprint(), hints = ?self.lan_hints(), "announcing device info");
        let Some(st) = self.peers.get_mut(&key) else { return };
        let Some(path) = st.best_secure_path() else {
            tracing::debug!(peer = %key.fingerprint(), "no secure path to announce on");
            return;
        };
        let (PeerPhase::Secure(sc), Some(sid), link) = (&mut path.phase, path.session_id, path.link) else { return };
        match sc.seal(&msg) {
            Ok(bytes) => {
                if let Err(e) = self.send_on(link, RelayFrame::forward(sid, bytes)).await {
                    tracing::warn!(error = %e, "device info announce failed");
                }
            }
            Err(e) => tracing::warn!(error = %e, "device info seal failed"),
        }
    }

    // ---------------- link events ----------------

    async fn handle_link_event(&mut self, id: LinkId, ev: LinkEvent) {
        // Events from a link we already dropped (late close, stale forwarder) are noise.
        if self.link(id).is_none() {
            return;
        }
        match ev {
            LinkEvent::Warning(w) => tracing::warn!(warning = %w, link = ?id, "link"),
            LinkEvent::State(change) => self.on_state(id, change.to).await,
            LinkEvent::Frame(frame) => self.on_frame(id, frame).await,
        }
    }

    async fn on_state(&mut self, id: LinkId, to: ConnectionState) {
        match id {
            LinkId::Relay => {
                let attempts = match to {
                    ConnectionState::Reconnecting => self.relay_status.attempts + 1,
                    ConnectionState::Connected => 0,
                    _ => self.relay_status.attempts,
                };
                self.relay_status.state = to;
                self.relay_status.attempts = attempts;
                self.emit(CoreEvent::Relay(self.relay_status.clone()));
                if to.is_connected() {
                    self.attach_all(LinkId::Relay).await;
                } else {
                    // Every relay-borne session is gone; LAN paths are untouched.
                    self.drop_link_state(LinkId::Relay);
                    self.fail_pairing_on(LinkId::Relay).await;
                    self.emit_devices();
                }
            }
            LinkId::Host => {
                if to.is_connected() {
                    self.attach_all(LinkId::Host).await;
                    if matches!(self.pending_start, Some(PendingStart::Initiator)) {
                        self.pending_start = None;
                        if let Err(e) = self.begin_initiator().await {
                            self.emit(CoreEvent::Error(e.to_string()));
                        }
                    }
                } else {
                    if matches!(self.pending_start, Some(PendingStart::Initiator)) && (to.is_closed() || to == ConnectionState::Reconnecting) {
                        self.pending_start = None;
                        self.pairing_link = None;
                        self.emit(CoreEvent::Error("the LAN host is not reachable; cannot start pairing".into()));
                    }
                    self.drop_link_state(LinkId::Host);
                    self.fail_pairing_on(LinkId::Host).await;
                    self.emit_devices();
                }
            }
            LinkId::Dial(n) => {
                let peer = self.dials.get(&n).and_then(|d| d.peer);
                if to.is_connected() {
                    match peer {
                        Some(key) => self.attach(id, key).await,
                        None => {
                            if let Some(PendingStart::Responder(method)) = self.pending_start.take()
                                && let Err(e) = self.begin_responder(method).await
                            {
                                self.emit(CoreEvent::Error(e.to_string()));
                            }
                        }
                    }
                } else if to.is_closed() || to == ConnectionState::Reconnecting {
                    if peer.is_none() && self.pending_start.take().is_some() {
                        self.emit(CoreEvent::Error("could not reach the other device on the local network".into()));
                    }
                    self.fail_pairing_on(id).await;
                    self.close_dial(n).await;
                    if let Some(key) = peer
                        && let Some(st) = self.peers.get_mut(&key)
                    {
                        // Next attempt after the current backoff; the backoff itself grows in `maintain_direct`.
                        st.next_dial_at = Some(Instant::now() + st.dial_backoff);
                    }
                    self.emit_devices();
                }
            }
        }
    }

    /// A pairing that ran on `link` cannot continue once the link is gone.
    async fn fail_pairing_on(&mut self, link: LinkId) {
        if self.pairing_link == Some(link) && !matches!(self.pairing, Pairing::None) {
            let _ = self.step_pairing(Event::Relay(RelayFrame::error(RelayErrorCode::SessionExpired))).await;
        }
    }

    /// Attach to the rendezvous channel with `key` on `link`.
    async fn attach(&mut self, link: LinkId, key: PublicKey) {
        let channel = rendezvous_channel(&self.identity.keypair.public, &key);
        let backoff = self.config.direct_retry;
        self.peers.entry(key).or_insert_with(|| PeerState::new(backoff)).path_or_insert(link);
        self.pending_attach.entry(link).or_default().push_back(key);
        if let Err(e) = self.send_on(link, RelayFrame::Attach { version: ProtocolVersion::CURRENT, channel }).await {
            tracing::warn!(error = %e, link = ?link, "attach failed");
        }
    }

    async fn attach_all(&mut self, link: LinkId) {
        for d in self.trusted.list() {
            self.attach(link, d.public_key).await;
        }
    }

    async fn on_frame(&mut self, id: LinkId, frame: RelayFrame) {
        match frame {
            RelayFrame::Attached { session_id, peer_online, .. } => {
                let Some(key) = self.pending_attach.get_mut(&id).and_then(VecDeque::pop_front) else { return };
                self.session_to_peer.insert((id, session_id), key);
                if let Some(p) = self.peers.get_mut(&key).and_then(|st| st.path(id)) {
                    p.session_id = Some(session_id);
                }
                if peer_online {
                    self.start_peer_handshake(key, id).await;
                }
                self.emit_devices();
            }
            RelayFrame::PeerPresence { session_id, online, .. } => {
                let Some(key) = self.session_to_peer.get(&(id, session_id)).copied() else { return };
                if online {
                    self.start_peer_handshake(key, id).await;
                } else if let Some(p) = self.peers.get_mut(&key).and_then(|st| st.path(id)) {
                    p.phase = PeerPhase::Idle;
                    p.handshake_started = None;
                }
                self.emit_devices();
            }
            RelayFrame::Forward { session_id, payload, .. } => {
                if self.pairing_link == Some(id)
                    && self.pairing_session_id() == Some(session_id)
                    && !matches!(self.pairing, Pairing::None)
                    && !self.session_to_peer.contains_key(&(id, session_id))
                {
                    if let Err(e) = self.step_pairing(Event::Peer(payload)).await {
                        self.emit(CoreEvent::Error(e.to_string()));
                    }
                    return;
                }
                if let Some(key) = self.session_to_peer.get(&(id, session_id)).copied() {
                    self.on_peer_bytes(key, id, session_id, payload).await;
                }
            }
            RelayFrame::SessionCreated { .. }
            | RelayFrame::Joined { .. }
            | RelayFrame::PeerJoined { .. }
            | RelayFrame::PeerLeft { .. }
            | RelayFrame::Error { .. } => {
                if matches!(self.pairing, Pairing::None) || self.pairing_link != Some(id) {
                    if let RelayFrame::Error { code, .. } = frame {
                        tracing::debug!(?code, link = ?id, "relay error outside pairing");
                    }
                    return;
                }
                if let Err(e) = self.step_pairing(Event::Relay(frame)).await {
                    self.emit(CoreEvent::Error(e.to_string()));
                }
            }
            RelayFrame::Hello { .. }
            | RelayFrame::HelloAck { .. }
            | RelayFrame::CreateSession { .. }
            | RelayFrame::JoinByCode { .. }
            | RelayFrame::JoinBySession { .. }
            | RelayFrame::Attach { .. }
            | RelayFrame::Leave { .. }
            | RelayFrame::Bye { .. } => {}
        }
    }

    async fn start_peer_handshake(&mut self, key: PublicKey, link: LinkId) {
        let local = self.identity.keypair.clone();
        let Some(p) = self.peers.get_mut(&key).and_then(|st| st.path(link)) else { return };
        let role = if is_initiator(&local.public, &key) { Role::Initiator } else { Role::Responder };
        match p.begin(&local, role) {
            Ok(Some(first)) => {
                if let Some(sid) = p.session_id
                    && let Err(e) = self.send_on(link, RelayFrame::forward(sid, first)).await
                {
                    tracing::warn!(error = %e, "peer handshake send failed");
                }
            }
            Ok(None) => {}
            Err(e) => tracing::warn!(error = %e, "peer handshake start failed"),
        }
    }

    async fn on_peer_bytes(&mut self, key: PublicKey, link: LinkId, session_id: SessionId, payload: Vec<u8>) {
        let local = self.identity.keypair.clone();
        let Some(p) = self.peers.get_mut(&key).and_then(|st| st.path(link)) else { return };
        if matches!(p.phase, PeerPhase::Idle) {
            // The peer initiated; we are the responder. Fall through to feed message 1.
            if p.begin(&local, Role::Responder).is_err() {
                return;
            }
        }
        match &mut p.phase {
            PeerPhase::Idle => {}
            PeerPhase::Handshaking(_) => {
                let (reply, finished) = match p.handshake_input(&payload) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!(error = %e, "peer handshake failed");
                        p.phase = PeerPhase::Idle;
                        return;
                    }
                };
                if let Some(bytes) = reply
                    && let Err(e) = self.send_on(link, RelayFrame::forward(session_id, bytes)).await
                {
                    tracing::warn!(error = %e, "peer handshake reply failed");
                }
                if finished {
                    self.finish_peer_handshake(key, link).await;
                }
            }
            PeerPhase::Secure(sc) => match sc.open(&payload) {
                Ok(AppMessage::Ping { seq, .. }) => {
                    if let Ok(bytes) = sc.seal(&AppMessage::pong(seq)) {
                        let _ = self.send_on(link, RelayFrame::forward(session_id, bytes)).await;
                    }
                }
                Ok(AppMessage::Text { body, .. }) => self.emit(CoreEvent::Message { from: key, body }),
                Ok(AppMessage::DeviceInfoUpdate { device, direct_hints, .. }) => self.on_device_info(key, &device, &direct_hints),
                // The phone as microphone (docs/dictation.md §20).
                Ok(AppMessage::TakeStart { take, .. }) => self.on_take_start(key, take),
                Ok(AppMessage::TakeAudio { take, seq, pcm, .. }) => self.on_take_audio(key, take, seq, &pcm),
                Ok(AppMessage::TakeOpus { take, seq, packets, .. }) => self.on_take_opus(key, take, seq, &packets),
                Ok(AppMessage::TakeStop { take, .. }) => self.on_take_stop(key, take),
                Ok(AppMessage::TakeCancel { take, .. }) => self.on_take_cancel(key, take),
                Ok(AppMessage::TakeStatus { take, state, opus, .. }) => self.on_take_status(key, take, state, opus),
                // Text from the phone (docs/dictation.md §20.6).
                Ok(AppMessage::PhoneText { id, body, source, .. }) => self.on_phone_text(key, id, body, source),
                Ok(AppMessage::PhoneTextStatus { id, state, .. }) => self.on_phone_text_status(key, id, state),
                Ok(AppMessage::Unpair { .. }) => self.on_unpaired(key).await,
                Ok(AppMessage::Pong { seq, .. }) => self.on_pong(key, seq),
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "undecryptable peer message"),
            },
            PeerPhase::IdentityChanged { .. } => {}
        }
    }

    /// A trusted peer (authenticated by the channel it came in on) forgot this device: forget it
    /// too, without telling it back, and say who it was.
    async fn on_unpaired(&mut self, key: PublicKey) {
        let Some(record) = self.trusted.get_by_key(&key) else { return };
        tracing::info!(peer = %key.fingerprint(), "peer unpaired this device; forgetting it");
        match self.forget(key, false).await {
            Ok(()) => self.emit(CoreEvent::Unpaired(record)),
            Err(e) => tracing::warn!(peer = %key.fingerprint(), error = %e, "could not forget the peer that unpaired"),
        }
    }

    /// A trusted peer (authenticated by the channel it came in on) told us its name and where
    /// its LAN host listens.
    fn on_device_info(&mut self, key: PublicKey, device: &voltip_protocol::DeviceInfo, hints: &[String]) {
        let renamed = self.trusted.update_info(&key, device).unwrap_or(false);
        let moved = match self.trusted.update_hints(&key, hints) {
            Ok(changed) => changed,
            Err(e) => {
                tracing::warn!(error = %e, "peer sent unusable LAN hints");
                false
            }
        };
        tracing::debug!(peer = %key.fingerprint(), ?hints, renamed, moved, "device info from peer");
        if moved && let Some(st) = self.peers.get_mut(&key) {
            let initial = self.config.direct_retry;
            st.reset_dial_backoff(initial);
        }
        if renamed || moved {
            self.emit_devices();
        }
    }

    async fn finish_peer_handshake(&mut self, key: PublicKey, link: LinkId) {
        let Some(p) = self.peers.get_mut(&key).and_then(|st| st.path(link)) else { return };
        let remote = match p.finish() {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "peer handshake finish failed");
                p.phase = PeerPhase::Idle;
                return;
            }
        };
        let claimed = self.trusted.get_by_key(&key).map(|d| d.device_id);
        match self.trusted.check(&remote, claimed.unwrap_or(voltip_protocol::DeviceId(uuid_nil()))) {
            IdentityCheck::Trusted(_) if remote == key => {
                let via = if link.is_direct() { ConnectionKind::Direct } else { ConnectionKind::Relay };
                let _ = self.trusted.mark_seen(&key, Self::now().unix_secs, via);
                if via == ConnectionKind::Direct
                    && let Some(st) = self.peers.get_mut(&key)
                {
                    let initial = self.config.direct_retry;
                    st.reset_dial_backoff(initial);
                }
                tracing::info!(peer = %key.fingerprint(), ?via, "secure channel established");
                self.announce_self(key).await;
            }
            _ => {
                // The key on the other end is not the one we trust for this channel.
                let previous = self.trusted.get_by_key(&key);
                p.phase = PeerPhase::IdentityChanged { presented: remote };
                if let Some(previous) = previous {
                    tracing::warn!(peer = %previous.name, "identity changed — refusing to trust silently");
                    self.emit(CoreEvent::IdentityChanged { previous, presented_fingerprint: remote.fingerprint() });
                }
            }
        }
        self.emit_devices();
    }

    // ---------------- periodic work ----------------

    async fn tick(&mut self) {
        let now = Instant::now();
        let limit = self.config.peer_handshake_timeout;
        let mut stalled = false;
        for p in self.peers.values_mut().flat_map(|st| st.paths.iter_mut()) {
            if p.expire_stalled_handshake(now, limit) {
                stalled = true;
            }
        }
        if stalled {
            tracing::warn!("peer handshake stalled; back to offline");
            self.emit_devices();
        }
        self.maintain_direct(now);
        self.check_takes();
        self.check_texts();
        self.check_deadline();
        if !matches!(self.pairing, Pairing::None) {
            match self.step_pairing(Event::Tick).await {
                Ok(()) => {}
                Err(CoreError::Pairing(voltip_pairing::PairingError::InvalidTransition { .. })) => {}
                Err(e) => self.emit(CoreEvent::Error(e.to_string())),
            }
        }
        self.keep_pairing_open().await;
    }

    /// Direct first: for every trusted device with known LAN endpoints and no live LAN path,
    /// open one outgoing connection at a time, backing off between failures.
    fn maintain_direct(&mut self, now: Instant) {
        if !self.config.direct_enabled || self.host.is_none() {
            return;
        }
        for d in self.trusted.list() {
            // Where the LAN browse saw it first (docs/pairing.md 「局域网发现」), then what it told us.
            let mut hints = self.lan.hints.get(&d.public_key).cloned().unwrap_or_default();
            for h in &d.direct_hints {
                if !hints.contains(h) {
                    hints.push(h.clone());
                }
            }
            if hints.is_empty() {
                continue;
            }
            let backoff = self.config.direct_retry;
            let st = self.peers.entry(d.public_key).or_insert_with(|| PeerState::new(backoff));
            if st.has_active_direct_path() || self.dials.values().any(|dl| dl.peer == Some(d.public_key)) {
                continue;
            }
            if st.next_dial_at.is_some_and(|t| now < t) {
                continue;
            }
            let hint = hints[st.hint_cursor % hints.len()].clone();
            st.hint_cursor = st.hint_cursor.wrapping_add(1);
            st.next_dial_at = Some(now + st.dial_backoff);
            st.dial_backoff = (st.dial_backoff * 2).min(self.config.direct_retry_max);
            if let Err(e) = self.dial(&hint, Some(d.public_key)) {
                tracing::debug!(error = %e, hint, "dial failed to start");
            }
        }
    }
}

/// Collapse a device's paths into what the UI shows. An identity mismatch is never hidden
/// behind a healthy path; otherwise a LAN path beats the relay.
fn summarize(paths: &[PeerPath]) -> DeviceConnection {
    if let Some(presented) = paths.iter().find_map(|p| if let PeerPhase::IdentityChanged { presented } = &p.phase { Some(*presented) } else { None }) {
        return DeviceConnection::IdentityChanged { presented_fingerprint: presented.fingerprint() };
    }
    if paths.iter().any(|p| p.is_secure() && p.link.is_direct()) {
        return DeviceConnection::Online { via: ConnectionKind::Direct };
    }
    if paths.iter().any(PeerPath::is_secure) {
        return DeviceConnection::Online { via: ConnectionKind::Relay };
    }
    if paths.iter().any(|p| matches!(p.phase, PeerPhase::Handshaking(_))) {
        return DeviceConnection::Connecting;
    }
    DeviceConnection::Offline
}

fn uuid_nil() -> uuid::Uuid {
    uuid::Uuid::nil()
}

enum JoinSpec {
    Code(String),
    Ticket(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_never_hides_an_identity_mismatch_and_prefers_direct() {
        assert_eq!(summarize(&[]), DeviceConnection::Offline);
        let idle = PeerPath::new(LinkId::Relay);
        assert_eq!(summarize(&[idle]), DeviceConnection::Offline);
        let mut flagged = PeerPath::new(LinkId::Relay);
        flagged.phase = PeerPhase::IdentityChanged { presented: PublicKey([3; 32]) };
        let mut direct = PeerPath::new(LinkId::Host);
        direct.phase =
            PeerPhase::Handshaking(Box::new(voltip_crypto::Handshake::new(Role::Initiator, &voltip_crypto::StaticKeypair::generate().unwrap(), None).unwrap()));
        assert_eq!(summarize(&[direct]), DeviceConnection::Connecting);
        let mut direct = PeerPath::new(LinkId::Host);
        direct.phase =
            PeerPhase::Handshaking(Box::new(voltip_crypto::Handshake::new(Role::Initiator, &voltip_crypto::StaticKeypair::generate().unwrap(), None).unwrap()));
        assert!(matches!(summarize(&[direct, flagged]), DeviceConnection::IdentityChanged { .. }));
    }

    #[test]
    fn config_defaults_pin_the_lan_port_and_direct_first() {
        let cfg = CoreConfig::new(PathBuf::from("/data"));
        assert_eq!(cfg.models_root, PathBuf::from("/data").join(MODELS_DIR_NAME));
        assert_eq!(MODELS_DIR_NAME, "models");
        assert!(MODEL_PROGRESS_INTERVAL >= Duration::from_millis(250) && MODEL_PROGRESS_BYTES >= 1024 * 1024);
        let cfg = CoreConfig::new(std::env::temp_dir());
        assert!(cfg.direct_enabled);
        assert_eq!(cfg.direct_bind.port(), DEFAULT_LAN_PORT);
        assert!(cfg.direct_retry < cfg.direct_retry_max);
    }
}
