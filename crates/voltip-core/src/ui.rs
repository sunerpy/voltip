//! UI-facing projection of [`CoreEvent`]: JSON-friendly (hex keys, tagged enums) and a cached
//! [`UiState`] so a freshly mounted webview can ask for "everything right now".

use serde::{Deserialize, Serialize};
use voltip_identity::{DeviceIdentityPublic, TrustedDevice};
use voltip_pairing::{PairingState, Snapshot};

use crate::phone::{PhoneTakeView, SentText};
use crate::presets::{CustomPreset, PresetTryOutcome};
use crate::{
    CoreEvent, DeviceView, DictationStatus, DictionaryEntry, EngineStatus, HistoryEntry, ModelState, ProbeReport, RefineFailure, RelayStatus, ReplacementRule,
    Scene, Settings,
};

/// Event name used on the Tauri event bus.
pub const UI_EVENT_NAME: &str = "voltip://event";

/// What the webview receives.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UiEvent {
    /// Full state (sent once after `Ready`, and on request).
    State(Box<UiState>),
    /// Identity changed.
    Identity(DeviceIdentityPublic),
    /// Settings changed.
    Settings(Settings),
    /// Relay status.
    Relay(RelayStatus),
    /// Pairing screen.
    Pairing(Snapshot),
    /// Device list.
    Devices {
        /// Full replacement of the list.
        devices: Vec<DeviceView>,
    },
    /// New trusted device.
    Trusted(TrustedDevice),
    /// A trusted device unpaired this one (docs/pairing.md); it has left the device list too.
    Unpaired(TrustedDevice),
    /// Identity mismatch on a trusted device.
    IdentityChanged {
        /// Trusted record.
        previous: TrustedDevice,
        /// Presented key fingerprint.
        presented_fingerprint: String,
    },
    /// Text from a peer.
    Message {
        /// Sender's public key, lower-case hex.
        from: String,
        /// Body.
        body: String,
    },
    /// Non-fatal error.
    Error {
        /// Message.
        message: String,
    },
    /// Global hotkey registration / press state. Produced by the desktop shell (which owns the OS
    /// registration), folded into the same state as core events so the UI has one source of truth.
    Hotkey(HotkeyStatus),
    /// Dictation state machine moved.
    Dictation(DictationStatus),
    /// The newest history entries and how many there are (docs/dictation.md §4.4).
    History {
        /// The newest [`crate::history::RECENT_ENTRIES`], newest first.
        recent: Vec<HistoryEntry>,
        /// Entries in the history.
        total: u32,
    },
    /// Resolved engine configuration (providers, models, user-entered hosts, key presence; never a
    /// key and never the built-in host).
    Engines(EngineStatus),
    /// The answer to one `provider_probe` (not cached: the pane that asked shows it).
    ProviderProbe(ProbeReport),
    /// Local model library, full replacement (download progress is folded into the list).
    Models {
        /// Every catalogue entry with its install state.
        models: Vec<ModelState>,
    },
    /// Personal dictionary, full replacement (docs/dictation.md §16).
    Dictionary {
        /// Entries in order.
        entries: Vec<DictionaryEntry>,
    },
    /// Replacement rules, full replacement, in execution order (docs/dictation.md §16).
    Rules {
        /// Rules in order.
        rules: Vec<ReplacementRule>,
    },
    /// Scenes, full replacement, in matching order (docs/dictation.md §18).
    Scenes {
        /// Scenes in order.
        scenes: Vec<Scene>,
    },
    /// Custom presets, full replacement (docs/dictation.md §21).
    Presets {
        /// In the order they were made.
        presets: Vec<CustomPreset>,
    },
    /// The answer to one `presets_try`, tagged with the id the request carried (not cached).
    PresetTry {
        /// The request's id.
        id: u64,
        /// The text, or why there is none.
        outcome: PresetTryOutcome,
    },
    /// Where a 用 AI 预设处理 request is (docs/dictation.md §22; not cached): progress, and the
    /// processed text once it is stored with the entry.
    HistoryProcess {
        /// The request's id.
        request_id: u64,
        /// The entry.
        id: uuid::Uuid,
        /// Running, done, failed or cancelled.
        state: crate::history::process::ProcessState,
    },
    /// Updater progress. Produced by the desktop shell (which owns the updater plugin) and folded
    /// into the same state as core events, like [`UiEvent::Hotkey`].
    Update(UpdateStatus),
    /// The machine as the local engines see it (docs/dictation.md §10.6). Produced by the desktop
    /// shell once at start, folded like [`UiEvent::Hotkey`].
    Hardware(HardwareStatus),
    /// The phone's take streamed to a desktop (docs/dictation.md §20).
    PhoneTake {
        /// `None` before the first take.
        take: Option<PhoneTakeView>,
    },
    /// The phone's texts sent to a desktop (docs/dictation.md §20.6), newest first.
    SentTexts {
        /// The whole list.
        texts: Vec<SentText>,
    },
    /// What the LAN browse sees (docs/pairing.md 「局域网发现」).
    Nearby {
        /// The whole list.
        devices: Vec<crate::discovery::NearbyDevice>,
    },
    /// Phone: its copies of its computers changed (docs/dictation.md §20.8).
    Mirrors {
        /// The whole list.
        mirrors: Vec<crate::sync::MirrorView>,
    },
    /// Phone: its records too large to upload.
    PhoneOutbox {
        /// Their ids.
        too_large: Vec<uuid::Uuid>,
    },
    /// The connectivity self-check started or finished.
    Connectivity(crate::connectivity::ConnectivityStatus),
    /// The local speech service started, stopped or failed to start (docs/dictation.md §23.6).
    Serve(ServeStatus),
    /// The built-in clean-up service's notice came or went (docs/dictation.md §3.6).
    RefineNotice {
        /// The notice; `None` once it is gone.
        notice: Option<RefineNotice>,
    },
    /// The answer to a paste from the history (`crate::paste`): the desktop shell waits for it; the
    /// webview reads the `paste_text` command's answer instead and ignores the event.
    PasteResult {
        /// The command's id.
        request_id: u64,
        /// What became of the text.
        outcome: crate::paste::PasteOutcome,
    },
}

/// Where the in-app updater is (`UiState.update`, `packages/shared/src/schema.ts`
/// `updateStatusSchema`). The core only defines and folds the type; the desktop shell drives it.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum UpdateStatus {
    /// Nothing checked yet.
    #[default]
    Idle,
    /// Asking the update endpoint.
    Checking,
    /// The running version is the newest.
    UpToDate {
        /// Running version.
        version: String,
        /// Unix time in seconds of this check.
        checked_at: u64,
    },
    /// A newer version is published.
    Available {
        /// Published version.
        version: String,
        /// Running version.
        current: String,
        /// Release notes from the manifest, when any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        notes: Option<String>,
        /// Publish date from the manifest (RFC 3339), when any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        date: Option<String>,
    },
    /// The package is being downloaded.
    Downloading {
        /// Version being downloaded.
        version: String,
        /// Bytes received so far.
        received: u64,
        /// Total size when the server announced one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        total: Option<u64>,
    },
    /// Downloaded and verified; installs on the install command or on the next start (auto mode).
    Ready {
        /// Version waiting to be installed.
        version: String,
    },
    /// The installer is running; the app is about to restart.
    Installing {
        /// Version being installed.
        version: String,
    },
    /// The last check / download / install failed.
    Failed {
        /// Human-readable reason.
        message: String,
    },
    /// This install is updated by the app store it came from (Google Play on Android): the phone
    /// only points there.
    Store {
        /// Running version.
        version: String,
    },
    /// This build has no update endpoint / public key baked in (or the shell has no updater).
    Disabled,
}

/// A project page the shell opens in the browser (`project_link_open`): the webview names the
/// page, never a URL, and the shell builds it from its own `Cargo.toml` `repository`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectLink {
    /// The source repository.
    Source,
    /// Where to report a problem or ask for a feature: the repository's new-issue page.
    Feedback,
    /// The published releases, with their notes and downloads (the update dialog).
    Releases,
}

impl ProjectLink {
    /// The page under `repository` (`https://github.com/<owner>/<repo>`, no trailing slash needed).
    pub fn url(self, repository: &str) -> String {
        let base = repository.trim().trim_end_matches('/');
        match self {
            Self::Source => base.to_owned(),
            Self::Feedback => format!("{base}/issues/new/choose"),
            Self::Releases => format!("{base}/releases"),
        }
    }
}

/// The user guide's site (docs/site, published from sunerpy/firlab): English at the root,
/// Chinese under `/zh/`.
pub const GUIDE_SITE: &str = "https://voltip.firlab.app";

/// A page of the user guide the shell opens in the browser (`guide_open`): the webview names the
/// page and its language, never a URL.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuidePage {
    /// 本机服务 (docs/site/recognition/service.md; docs/dictation.md §23.6).
    Service,
}

impl GuidePage {
    /// The page in the language of `locale` (a BCP 47 tag such as `zh-CN` or `en`): Chinese for
    /// `zh…`, English otherwise.
    pub fn url(self, locale: &str) -> String {
        let path = match self {
            Self::Service => "recognition/service",
        };
        let zh = locale.trim().to_ascii_lowercase().starts_with("zh");
        format!("{GUIDE_SITE}/{}{path}", if zh { "zh/" } else { "" })
    }
}

/// A GPU the local engines can run on (docs/dictation.md §10.6).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct GpuDevice {
    /// Backend device name (`Metal`, `Vulkan0`): what `EngineSettings.local_gpu` stores.
    pub name: String,
    /// What the driver calls it.
    pub description: String,
    /// Backend family (`metal`, `vulkan`, …).
    pub kind: String,
    /// Device memory in MiB (`0` when not reported).
    #[serde(default)]
    pub memory_mb: u64,
    /// Integrated (shares system memory).
    #[serde(default)]
    pub integrated: bool,
}

/// Whether the local speech service runs (docs/dictation.md §23.6).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServePhase {
    /// Switched off (or not available here).
    #[default]
    Off,
    /// Switched on, the listener being started.
    Starting,
    /// Listening on `address`.
    Running,
    /// Switched on but not listening: `error` says why (the port is taken).
    Failed,
}

/// `UiState.serve`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServeStatus {
    /// This app can host the service (the desktop).
    pub available: bool,
    /// Off, running or failed.
    pub phase: ServePhase,
    /// What a client sets as its base URL (`http://127.0.0.1:<port>/v1`) while running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// Why the service is not running although switched on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The machine as the local engines see it (docs/dictation.md §10.6). Shell-owned: empty until the
/// desktop shell reports, always empty on the phone.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct HardwareStatus {
    /// Logical CPUs (`0` until reported).
    #[serde(default)]
    pub cpu_threads: u32,
    /// GPUs this build has a backend for (empty in a CPU-only build).
    #[serde(default)]
    pub gpus: Vec<GpuDevice>,
}

/// What the shell knows about the global hotkey.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct HotkeyStatus {
    /// Chord actually registered with the OS (display form), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registered: Option<String>,
    /// Why the last registration failed (conflict with another app, unsupported chord, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The chord is currently held down.
    pub pressed: bool,
    /// The settings page is recording a new chord; the OS registration is suspended meanwhile.
    #[serde(default)]
    pub capturing: bool,
    /// Registration backend name for the settings page (`global-shortcut · Windows`).
    pub backend: String,
    /// The voice-edit chord actually registered (docs/dictation.md §19), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit_registered: Option<String>,
    /// Why the voice-edit chord is not registered (conflict, pure Wayland, same as the dictation chord).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit_error: Option<String>,
    /// What the hotkey can do in the session the shell runs in (all `false` until it reports).
    #[serde(default)]
    pub capabilities: HotkeyCapabilities,
    /// The lone-key trigger the shell's input hook watches (docs/dictation.md §13.1), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solo_registered: Option<crate::SoloKey>,
    /// Why the chosen lone key is not watched (no hook on this session, a permission, a key this
    /// platform has not got).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solo_error: Option<String>,
    /// The lone key is currently held down on its own.
    #[serde(default)]
    pub solo_pressed: bool,
}

/// What the global hotkey can do in the session the desktop runs in (docs/dictation.md §13, §14),
/// as the shell measures it; the settings page shows it for this machine.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct HotkeyCapabilities {
    /// The shell can register a global chord here (not on a pure Wayland session).
    pub global: bool,
    /// The chord fires whichever window has the focus (under XWayland only while an X11 window does).
    pub everywhere: bool,
    /// Presses and releases both arrive, so press-and-hold activation works (a compositor
    /// shortcut only runs a command on the press).
    pub hold: bool,
    /// What a desktop or compositor shortcut runs to start and stop a take (`… --toggle`).
    pub toggle_command: String,
    /// The same for voice edit (`… --edit-toggle`).
    pub edit_toggle_command: String,
    /// The lone keys the shell can watch here (docs/dictation.md §13.1); empty where no input hook
    /// exists (a pure Wayland session, the phone).
    #[serde(default)]
    pub solo_keys: Vec<crate::SoloKey>,
}

/// Snapshot of everything the UI renders.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct UiState {
    /// This device.
    pub identity: Option<DeviceIdentityPublic>,
    /// Settings.
    pub settings: Settings,
    /// Secret store backend name.
    pub secret_backend: String,
    /// The app version (`""` until the core is ready).
    #[serde(default)]
    pub app_version: String,
    /// Relay status.
    pub relay: RelayStatus,
    /// Pairing screen.
    pub pairing: Snapshot,
    /// Device list.
    pub devices: Vec<DeviceView>,
    /// Global hotkey registration state (shell-owned; empty until the shell reports).
    #[serde(default)]
    pub hotkey: HotkeyStatus,
    /// Dictation state machine.
    #[serde(default)]
    pub dictation: DictationStatus,
    /// The newest history entries, newest first ([`crate::history::RECENT_ENTRIES`] at most); the
    /// rest is read through the history queries (docs/dictation.md §4.4).
    #[serde(default)]
    pub history_recent: Vec<HistoryEntry>,
    /// Entries in the history.
    #[serde(default)]
    pub history_total: u32,
    /// Resolved ASR / refine / inject configuration.
    #[serde(default)]
    pub engines: EngineStatus,
    /// In-app updater state (shell-owned; `Idle` until the shell reports).
    #[serde(default)]
    pub update: UpdateStatus,
    /// Local model library (empty on shells without local models).
    #[serde(default)]
    pub models: Vec<ModelState>,
    /// Personal dictionary (docs/dictation.md §16).
    #[serde(default)]
    pub dictionary: Vec<DictionaryEntry>,
    /// Replacement rules in execution order (docs/dictation.md §16).
    #[serde(default)]
    pub rules: Vec<ReplacementRule>,
    /// Scenes in matching order (docs/dictation.md §18).
    #[serde(default)]
    pub scenes: Vec<Scene>,
    /// Custom presets (docs/dictation.md §21); the built-in ones are the interface's own.
    #[serde(default)]
    pub presets: Vec<CustomPreset>,
    /// The phone's current or last take streamed to a desktop (docs/dictation.md §20); always
    /// `None` on the desktop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone_take: Option<PhoneTakeView>,
    /// The texts this phone sent to a desktop (docs/dictation.md §20.6), newest first; always empty
    /// on the desktop.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sent_texts: Vec<SentText>,
    /// Devices the LAN browse sees (docs/pairing.md 「局域网发现」).
    #[serde(default)]
    pub nearby: Vec<crate::discovery::NearbyDevice>,
    /// The machine as the local engines see it (shell-owned; empty until reported).
    #[serde(default)]
    pub hardware: HardwareStatus,
    /// The connectivity self-check: running, and the last report.
    #[serde(default)]
    pub connectivity: crate::connectivity::ConnectivityStatus,
    /// Phone: its copies of its computers' histories and settings (docs/dictation.md §20.8);
    /// always empty on the desktop.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mirrors: Vec<crate::sync::MirrorView>,
    /// Phone: its own records too large to upload to a computer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub phone_outbox_too_large: Vec<uuid::Uuid>,
    /// The local speech service the app hosts (docs/dictation.md §23.6); `available` is false
    /// where the shell cannot host it (the phone).
    #[serde(default)]
    pub serve: ServeStatus,
    /// The built-in clean-up service turned a take down for want of capacity (docs/dictation.md
    /// §3.6); `None` when there is nothing to say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refine_notice: Option<RefineNotice>,
}

/// The built-in clean-up service turned a take's clean-up down for want of capacity
/// (docs/dictation.md §3.6): the raw text went in, and the interface suggests a provider of the
/// user's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefineNotice {
    /// What the service said: [`RefineFailure::RateLimited`] or [`RefineFailure::Quota`].
    pub failure: RefineFailure,
    /// When the take ended, Unix milliseconds.
    pub at_ms: u64,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            identity: None,
            settings: Settings::default(),
            secret_backend: String::new(),
            app_version: String::new(),
            relay: RelayStatus { endpoint: None, source: crate::RelaySource::None, state: voltip_transport::ConnectionState::Disconnected, attempts: 0 },
            pairing: idle_snapshot(),
            devices: Vec::new(),
            hotkey: HotkeyStatus::default(),
            dictation: DictationStatus::default(),
            history_recent: Vec::new(),
            history_total: 0,
            engines: EngineStatus::default(),
            update: UpdateStatus::default(),
            models: Vec::new(),
            dictionary: Vec::new(),
            rules: Vec::new(),
            scenes: Vec::new(),
            presets: Vec::new(),
            phone_take: None,
            sent_texts: Vec::new(),
            nearby: Vec::new(),
            hardware: HardwareStatus::default(),
            connectivity: crate::connectivity::ConnectivityStatus::default(),
            mirrors: Vec::new(),
            phone_outbox_too_large: Vec::new(),
            serve: ServeStatus::default(),
            refine_notice: None,
        }
    }
}

fn idle_snapshot() -> Snapshot {
    Snapshot {
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
    }
}

impl UiState {
    /// Fold a core event into the cache and return the UI event to broadcast.
    pub fn apply(&mut self, event: CoreEvent) -> UiEvent {
        match event {
            CoreEvent::Ready { identity, settings, secret_backend, app_version } => {
                self.identity = Some(identity);
                self.settings = settings;
                self.secret_backend = secret_backend.to_owned();
                self.app_version = app_version;
                UiEvent::State(Box::new(self.clone()))
            }
            CoreEvent::Identity(i) => {
                self.identity = Some(i.clone());
                UiEvent::Identity(i)
            }
            CoreEvent::Settings(s) => {
                self.settings = s.clone();
                UiEvent::Settings(s)
            }
            CoreEvent::Mirrors(mirrors) => {
                self.mirrors = mirrors.clone();
                UiEvent::Mirrors { mirrors }
            }
            CoreEvent::PhoneOutbox { too_large } => {
                self.phone_outbox_too_large = too_large.clone();
                UiEvent::PhoneOutbox { too_large }
            }
            CoreEvent::Relay(r) => {
                self.relay = r.clone();
                UiEvent::Relay(r)
            }
            CoreEvent::Pairing(p) => {
                self.pairing = p.clone();
                UiEvent::Pairing(p)
            }
            CoreEvent::Devices(d) => {
                self.devices = d.clone();
                UiEvent::Devices { devices: d }
            }
            CoreEvent::Trusted(t) => UiEvent::Trusted(t),
            CoreEvent::Unpaired(t) => UiEvent::Unpaired(t),
            CoreEvent::IdentityChanged { previous, presented_fingerprint } => UiEvent::IdentityChanged { previous, presented_fingerprint },
            CoreEvent::Message { from, body } => UiEvent::Message { from: from.to_hex(), body },
            CoreEvent::Dictation(status) => {
                self.dictation = status.clone();
                UiEvent::Dictation(status)
            }
            CoreEvent::History { recent, total } => {
                self.history_recent = recent.clone();
                self.history_total = total;
                UiEvent::History { recent, total }
            }
            CoreEvent::Engines(status) => {
                self.engines = status.clone();
                UiEvent::Engines(status)
            }
            CoreEvent::Models(models) => {
                self.models = models.clone();
                UiEvent::Models { models }
            }
            CoreEvent::Dictionary(entries) => {
                self.dictionary = entries.clone();
                UiEvent::Dictionary { entries }
            }
            CoreEvent::Rules(rules) => {
                self.rules = rules.clone();
                UiEvent::Rules { rules }
            }
            CoreEvent::Scenes(scenes) => {
                self.scenes = scenes.clone();
                UiEvent::Scenes { scenes }
            }
            CoreEvent::Presets(presets) => {
                self.presets = presets.clone();
                UiEvent::Presets { presets }
            }
            CoreEvent::PresetTry { id, outcome } => UiEvent::PresetTry { id, outcome },
            CoreEvent::HistoryProcess { request_id, id, state } => UiEvent::HistoryProcess { request_id, id, state },
            CoreEvent::ProviderProbe(report) => UiEvent::ProviderProbe(report),
            CoreEvent::Connectivity(status) => {
                self.connectivity = status.clone();
                UiEvent::Connectivity(status)
            }
            CoreEvent::RefineNotice(notice) => {
                self.refine_notice = notice;
                UiEvent::RefineNotice { notice }
            }
            CoreEvent::Serve(status) => {
                self.serve = status.clone();
                UiEvent::Serve(status)
            }
            CoreEvent::PhoneTake(take) => {
                self.phone_take = take.clone();
                UiEvent::PhoneTake { take }
            }
            CoreEvent::SentTexts(texts) => {
                self.sent_texts = texts.clone();
                UiEvent::SentTexts { texts }
            }
            CoreEvent::Nearby(devices) => {
                self.nearby = devices.clone();
                UiEvent::Nearby { devices }
            }
            CoreEvent::PasteResult { request_id, outcome } => UiEvent::PasteResult { request_id, outcome },
            CoreEvent::Error(message) => UiEvent::Error { message },
        }
    }

    /// Fold an event that did not come from the core (the shell's hotkey or updater status) into
    /// the cache. Returns the event unchanged so it can be broadcast on the same channel.
    pub fn apply_shell(&mut self, event: UiEvent) -> UiEvent {
        match &event {
            UiEvent::Hotkey(status) => self.hotkey = status.clone(),
            UiEvent::Update(status) => self.update = status.clone(),
            UiEvent::Hardware(status) => self.hardware = status.clone(),
            _ => {}
        }
        event
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use voltip_crypto::PublicKey;
    use voltip_protocol::{DeviceId, Platform};

    #[test]
    fn project_links_are_pages_of_the_repository() {
        let repo = "https://github.com/example/voltip";
        assert_eq!(ProjectLink::Source.url(repo), repo);
        assert_eq!(ProjectLink::Feedback.url(&format!("{repo}/")), format!("{repo}/issues/new/choose"));
        assert_eq!(ProjectLink::Releases.url(repo), format!("{repo}/releases"));
        assert_eq!(GuidePage::Service.url("zh-CN"), "https://voltip.firlab.app/zh/recognition/service");
        assert_eq!(GuidePage::Service.url("en"), "https://voltip.firlab.app/recognition/service");
        assert_eq!(GuidePage::Service.url("ZH-tw"), "https://voltip.firlab.app/zh/recognition/service");
        assert_eq!(serde_json::to_string(&GuidePage::Service).unwrap(), r#""service""#);
        assert_eq!(serde_json::to_string(&ProjectLink::Feedback).unwrap(), r#""feedback""#);
        assert_eq!(serde_json::from_str::<ProjectLink>(r#""source""#).unwrap(), ProjectLink::Source);
    }

    #[test]
    fn state_folds_events_and_serializes_with_tags() {
        let mut st = UiState::default();
        assert!(st.identity.is_none());
        let id = DeviceIdentityPublic {
            device_id: DeviceId::random(),
            name: "Desk".into(),
            platform: Platform::Windows,
            public_key: PublicKey([1; 32]),
            fingerprint: "x".into(),
        };
        let ev = st.apply(CoreEvent::Ready { identity: id.clone(), settings: Settings::default(), secret_backend: "memory", app_version: "1.2.3".into() });
        assert!(matches!(ev, UiEvent::State(ref s) if s.identity.as_ref() == Some(&id) && s.secret_backend == "memory"));
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.starts_with(r#"{"type":"state","identity":"#), "{json}");
        let back: UiEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ev);
        let ev = st.apply(CoreEvent::Message { from: PublicKey([0xab; 32]), body: "hi".into() });
        assert_eq!(serde_json::to_string(&ev).unwrap(), format!(r#"{{"type":"message","from":"{}","body":"hi"}}"#, "ab".repeat(32)));
        let ev = st.apply(CoreEvent::Error("boom".into()));
        assert_eq!(serde_json::to_string(&ev).unwrap(), r#"{"type":"error","message":"boom"}"#);
        st.apply(CoreEvent::Relay(RelayStatus {
            endpoint: Some("wss://x".into()),
            source: crate::RelaySource::User,
            state: voltip_transport::ConnectionState::Connected,
            attempts: 0,
        }));
        assert_eq!(st.relay.state, voltip_transport::ConnectionState::Connected);
        let renamed = DeviceIdentityPublic { name: "Studio".into(), ..id };
        st.apply(CoreEvent::Identity(renamed.clone()));
        assert_eq!(st.identity.as_ref().unwrap().name, "Studio");
        let s2 = Settings { theme: crate::ThemeId::Warm, ..Settings::default() };
        st.apply(CoreEvent::Settings(s2.clone()));
        assert_eq!(st.settings, s2);
        let snap = idle_snapshot();
        st.apply(CoreEvent::Pairing(snap.clone()));
        assert_eq!(st.pairing, snap);
        st.apply(CoreEvent::Devices(vec![]));
        let rec = TrustedDevice {
            device_id: DeviceId::random(),
            name: "P".into(),
            platform: Platform::Android,
            public_key: PublicKey([2; 32]),
            fingerprint: "f".into(),
            trusted_at: 1,
            last_seen: None,
            last_connection: None,
            direct_hints: vec!["192.168.1.24:47831".into()],
            sync: true,
            sync_gen: 0,
        };
        assert!(matches!(st.apply(CoreEvent::Trusted(rec.clone())), UiEvent::Trusted(_)));
        let ev = st.apply(CoreEvent::IdentityChanged { previous: rec, presented_fingerprint: "AA".into() });
        assert!(serde_json::to_string(&ev).unwrap().contains(r#""type":"identity_changed""#));
        assert_eq!(UI_EVENT_NAME, "voltip://event");
    }

    #[test]
    fn dictation_history_and_engines_fold_and_default_when_absent() {
        let mut st = UiState::default();
        assert_eq!(st.dictation, DictationStatus::default());
        let status = DictationStatus::dictation(crate::DictationPhase::Listening { started_at: 1, ready: true, live: None, locked: false }, 1);
        let ev = st.apply(CoreEvent::Dictation(status.clone()));
        assert_eq!(st.dictation, status);
        let json = serde_json::to_string(&ev).unwrap();
        assert_eq!(json, r#"{"type":"dictation","phase":{"phase":"listening","started_at":1,"ready":true,"locked":false},"session":1,"kind":"dictation"}"#);
        assert_eq!(serde_json::from_str::<UiEvent>(&json).unwrap(), ev);
        let entry = HistoryEntry {
            id: uuid::Uuid::nil(),
            at_ms: 1,
            raw_text: "a".into(),
            text: "a。".into(),
            refined: true,
            asr_model: "m".into(),
            refine_model: Some("r".into()),
            duration_ms: 2,
            asr_ms: 3,
            refine_ms: Some(4),
            refine_failure: None,
            outcome: crate::Outcome::Inserted { via: crate::dictation::Via::Paste },
            starred: false,
            mode: crate::OutputMode::WholeTake,
            segments: None,
            live_error: None,
            vocabulary: None,
            kind: crate::TakeKind::Dictation,
            edit: None,
            app: None,
            scene: None,
            preset: None,
            origin: None,
            processed: None,
        };
        let ev = st.apply(CoreEvent::History { recent: vec![entry.clone()], total: 7 });
        assert_eq!((st.history_recent.clone(), st.history_total), (vec![entry], 7));
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.starts_with(r#"{"type":"history","recent":[{"id":"00000000-"#) && json.ends_with(r#""total":7}"#), "{json}");
        let engines = EngineStatus { asr_host: "asr.example.test".into(), ..EngineStatus::default() };
        let ev = st.apply(CoreEvent::Engines(engines.clone()));
        assert_eq!(st.engines, engines);
        assert!(
            serde_json::to_string(&ev)
                .unwrap()
                .starts_with(r#"{"type":"engines","asr_provider":"builtin","asr_ready":false,"asr_model":"","asr_host":"asr.example.test""#)
        );
        // The model library folds as a list (struct variant, like `devices`).
        assert!(st.models.is_empty());
        let model = ModelState {
            id: "sense-voice-small".into(),
            name: "SenseVoice Small".into(),
            engine: "sense_voice".into(),
            tier: "light".into(),
            capabilities: vec!["offline".into()],
            languages: vec!["zh".into()],
            size_bytes: 1,
            description: "d".into(),
            recommended: true,
            repo: "example/model".into(),
            dir: String::new(),
            files: Vec::new(),
            active: true,
            state: crate::ModelInstallState::Verifying,
        };
        let ev = st.apply(CoreEvent::Models(vec![model.clone()]));
        assert_eq!(st.models, vec![model]);
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.starts_with(r#"{"type":"models","models":[{"id":"sense-voice-small""#), "{json}");
        assert!(json.contains(r#""state":{"kind":"verifying"}"#), "{json}");
        assert_eq!(serde_json::from_str::<UiEvent>(&json).unwrap(), ev);
        // A state serialized before the dictation fields existed still parses.
        let legacy = r#"{"identity":null,"settings":{"schema":1,"theme":"light","follow_system_theme":false,"relay_enabled":true},"secret_backend":"","relay":{"state":"disconnected","attempts":0},"pairing":{"state":{"state":"idle"},"local_confirmed":false,"peer_confirmed":false},"devices":[]}"#;
        let back: UiState = serde_json::from_str(legacy).unwrap();
        assert_eq!(back, UiState::default());
    }

    /// docs/dictation.md §16.4: the dictionary and the rules fold as lists (struct variants, like
    /// `models`), and a state serialized before them parses with empty lists.
    #[test]
    fn dictionary_and_rules_fold_as_lists() {
        let mut st = UiState::default();
        let id = uuid::Uuid::nil();
        let entry = DictionaryEntry {
            id,
            term: "Voltip".into(),
            heard_as: vec!["沃提普".into()],
            enabled: true,
            source: crate::EntrySource::Manual,
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        let ev = st.apply(CoreEvent::Dictionary(vec![entry.clone()]));
        assert_eq!(st.dictionary, vec![entry]);
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.starts_with(r#"{"type":"dictionary","entries":[{"id":"00000000-"#), "{json}");
        assert_eq!(serde_json::from_str::<UiEvent>(&json).unwrap(), ev);
        let rule = ReplacementRule {
            id,
            name: "n".into(),
            kind: crate::RuleKind::Literal,
            pattern: "a".into(),
            replacement: "b".into(),
            case_sensitive: true,
            enabled: true,
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        let ev = st.apply(CoreEvent::Rules(vec![rule.clone()]));
        assert_eq!(st.rules, vec![rule]);
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.starts_with(r#"{"type":"rules","rules":[{"id":"00000000-"#), "{json}");
        assert_eq!(serde_json::from_str::<UiEvent>(&json).unwrap(), ev);
        let legacy = r#"{"identity":null,"settings":{"schema":1,"theme":"light","follow_system_theme":false,"relay_enabled":true},"secret_backend":"","relay":{"state":"disconnected","attempts":0},"pairing":{"state":{"state":"idle"},"local_confirmed":false,"peer_confirmed":false},"devices":[]}"#;
        let back: UiState = serde_json::from_str(legacy).unwrap();
        assert!(back.dictionary.is_empty() && back.rules.is_empty());
    }

    /// docs/dictation.md §18.6: the scenes fold as a list (a struct variant), and a state serialized
    /// before them parses with an empty list.
    #[test]
    fn scenes_fold_as_a_list() {
        let mut st = UiState::default();
        let scene = crate::Scene {
            id: uuid::Uuid::nil(),
            name: "聊天".into(),
            enabled: true,
            matching: crate::SceneMatch { apps: vec!["slack".into()], title_contains: Vec::new() },
            overrides: crate::SceneOverrides::default(),
            created_at_ms: 1,
            updated_at_ms: 1,
            builtin: None,
        };
        let ev = st.apply(CoreEvent::Scenes(vec![scene.clone()]));
        assert_eq!(st.scenes, vec![scene]);
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.starts_with(r#"{"type":"scenes","scenes":[{"id":"00000000-"#), "{json}");
        assert_eq!(serde_json::from_str::<UiEvent>(&json).unwrap(), ev);
        let legacy = r#"{"identity":null,"settings":{"schema":1,"theme":"light","follow_system_theme":false,"relay_enabled":true},"secret_backend":"","relay":{"state":"disconnected","attempts":0},"pairing":{"state":{"state":"idle"},"local_confirmed":false,"peer_confirmed":false},"devices":[]}"#;
        assert!(serde_json::from_str::<UiState>(legacy).unwrap().scenes.is_empty());
    }

    /// Shell-produced events fold into the cache exactly like core events: the hotkey status and
    /// the updater status both land in `UiState`, everything else passes through untouched.
    #[test]
    fn shell_events_fold_hotkey_and_update_and_serialize_with_tags() {
        let mut st = UiState::default();
        assert_eq!(st.update, UpdateStatus::Idle);
        let hk = HotkeyStatus { registered: Some("Ctrl+Alt+Space".into()), backend: "x".into(), ..HotkeyStatus::default() };
        assert_eq!(st.apply_shell(UiEvent::Hotkey(hk.clone())), UiEvent::Hotkey(hk.clone()));
        assert_eq!(st.hotkey, hk);
        let available = UpdateStatus::Available { version: "2.1.0".into(), current: "2.0.0".into(), notes: None, date: Some("2026-09-25T00:00:00Z".into()) };
        let ev = st.apply_shell(UiEvent::Update(available.clone()));
        assert_eq!(st.update, available);
        let json = serde_json::to_string(&ev).unwrap();
        assert_eq!(json, r#"{"type":"update","state":"available","version":"2.1.0","current":"2.0.0","date":"2026-09-25T00:00:00Z"}"#, "None is omitted");
        assert_eq!(serde_json::from_str::<UiEvent>(&json).unwrap(), ev);
        // An unrelated event passes through and changes nothing.
        let before = st.clone();
        assert_eq!(st.apply_shell(UiEvent::Error { message: "x".into() }), UiEvent::Error { message: "x".into() });
        assert_eq!(st, before);
        // Every variant carries its `state` tag; the shell publishes these in this order in auto mode.
        for (status, wire) in [
            (UpdateStatus::Idle, r#"{"state":"idle"}"#),
            (UpdateStatus::Checking, r#"{"state":"checking"}"#),
            (UpdateStatus::UpToDate { version: "2.0.0".into(), checked_at: 7 }, r#"{"state":"up_to_date","version":"2.0.0","checked_at":7}"#),
            (UpdateStatus::Downloading { version: "2.1.0".into(), received: 10, total: None }, r#"{"state":"downloading","version":"2.1.0","received":10}"#),
            (UpdateStatus::Ready { version: "2.1.0".into() }, r#"{"state":"ready","version":"2.1.0"}"#),
            (UpdateStatus::Installing { version: "2.1.0".into() }, r#"{"state":"installing","version":"2.1.0"}"#),
            (UpdateStatus::Failed { message: "offline".into() }, r#"{"state":"failed","message":"offline"}"#),
            (UpdateStatus::Disabled, r#"{"state":"disabled"}"#),
        ] {
            assert_eq!(serde_json::to_string(&status).unwrap(), wire);
            assert_eq!(serde_json::from_str::<UpdateStatus>(wire).unwrap(), status);
            st.apply_shell(UiEvent::Update(status.clone()));
            assert_eq!(st.update, status);
        }
        // A state serialized before the updater existed parses with `Idle`.
        let legacy = r#"{"identity":null,"settings":{"schema":1,"theme":"light","follow_system_theme":false,"relay_enabled":true},"secret_backend":"","relay":{"state":"disconnected","attempts":0},"pairing":{"state":{"state":"idle"},"local_confirmed":false,"peer_confirmed":false},"devices":[]}"#;
        assert_eq!(serde_json::from_str::<UiState>(legacy).unwrap().update, UpdateStatus::Idle);
    }
}
