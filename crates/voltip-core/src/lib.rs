//! Application core: the one place that knows how identity, pairing, transport and trust fit
//! together. UIs (Tauri desktop, Tauri mobile) drive it with [`CoreCommand`]s and render
//! [`CoreEvent`]s; nothing above this crate touches sockets or keys.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod channel;
pub mod connectivity;
pub mod dictation;
pub mod discovery;
pub mod engines;
pub mod history;
pub mod hotkey;
mod list_file;
pub mod models;
pub mod paste;
mod peer;
pub mod phone;
pub mod presets;
pub mod providers;
mod runtime;
pub mod scenes;
pub mod script;
mod settings;
pub mod ui;
mod view;
pub mod vocabulary;

pub use channel::{is_initiator, rendezvous_channel};
pub use dictation::activation::{Activation, ActivationConfig, ActivationMachine, Edge, EdgeSource, Intent, PhaseHint};
pub use dictation::{
    DictationError, DictationPhase, DictationPorts, DictationStatus, FailureCode, ForegroundApp, ForegroundProbe, LiveText, MAX_EDIT_SELECTION_CHARS,
    ProcessingStage, Segment, SelectionTiming, TakeKind,
};
pub use engines::{
    BuiltIn, ChineseScript, EngineIssue, EngineSettings, EngineStatus, InjectMode, LocalDevice, LocalModelRef, MAX_LOCAL_THREADS, OutputMode, ProviderSettings,
    ProviderStatus, RemoteService, ResolvedEngines, SecretSource, SecretState, ServiceStatus, UserSecrets,
};
pub use history::{EditRecord, EntryOrigin, HistoryEntry, HistoryStore, OriginKind, Outcome};
pub use hotkey::{DEFAULT_EDIT_HOTKEY, DEFAULT_HOTKEY, Hotkey, HotkeyError, Modifier, SoloKey};
pub use models::{
    CAPABILITY_OFFLINE, CAPABILITY_STREAMING, CAPABILITY_VAD, CancelToken, DEFAULT_LOCAL_MODEL_ID, ModelInstallState, ModelManager, ModelState, ProgressSink,
};
pub use presets::{BuiltinPreset, CustomPreset, PresetDraft, PresetError, PresetId, PresetRef, PresetTrial, PresetTryOutcome, TakePreset};
pub use providers::{KeyPolicy, PROVIDERS, ProbeError, ProbeFailure, ProbeOutcome, ProbeReport, ProviderId, ProviderSpec, ServiceKind, ServicePreset};
pub use runtime::{AppCore, CoreCommand, CoreConfig, CoreEvent, CoreHandle, EXTRA_RECORDING_POLL, MAX_ACTIVATION_MS, PRESET_TRY_UNCONFIGURED, now_ms};
pub use scenes::{AppRef, BuiltinScene, ContextSharing, Scene, SceneDraft, SceneError, SceneMatch, SceneOverrides, SceneRef, TakeContext};
pub use settings::{HistorySettings, Locale, OverlayPlacement, SETTINGS_FILE_NAME, Settings, SettingsStore, ThemeId};
pub use view::{DeviceConnection, DeviceView, RelaySource, RelayStatus};
pub use vocabulary::{
    DictionaryDraft, DictionaryEntry, EntrySource, ImportMode, PreviewDraft, ReplacementRule, RuleDraft, RuleKind, Vocabulary, VocabularyError, VocabularyHit,
    VocabularyHits, VocabularyPreview,
};

/// Errors surfaced to the UI as `CoreEvent::Error` or returned from command submission.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// Identity / secret store problems.
    #[error(transparent)]
    Identity(#[from] voltip_identity::IdentityError),
    /// Transport problems.
    #[error(transparent)]
    Transport(#[from] voltip_transport::TransportError),
    /// Pairing state machine misuse.
    #[error(transparent)]
    Pairing(#[from] voltip_pairing::PairingError),
    /// Protocol encode/decode.
    #[error(transparent)]
    Protocol(#[from] voltip_protocol::CodecError),
    /// Settings file problems.
    #[error("settings: {0}")]
    Settings(String),
    /// History file problems.
    #[error("history: {0}")]
    History(String),
    /// Dictation command refused or a pipeline step failed.
    #[error(transparent)]
    Dictation(#[from] DictationError),
    /// A dictionary / rule command, store or import was refused (docs/dictation.md §16).
    #[error(transparent)]
    Vocabulary(#[from] vocabulary::VocabularyError),
    /// A scene command or the scene store was refused (docs/dictation.md §18).
    #[error(transparent)]
    Scenes(#[from] scenes::SceneError),
    /// A preset command or the preset store was refused (docs/dictation.md §21).
    #[error(transparent)]
    Presets(#[from] presets::PresetError),
    /// A hotkey text the user typed or recorded is not a usable chord.
    #[error("hotkey: {0}")]
    Hotkey(#[from] hotkey::HotkeyError),
    /// The core task is gone.
    #[error("core is not running")]
    NotRunning,
    /// Command not valid right now (e.g. pairing while already pairing).
    #[error("{0}")]
    Invalid(String),
}
