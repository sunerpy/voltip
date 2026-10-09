//! Local (offline) ASR models (docs/dictation.md §10): what the UI sees of the model library
//! ([`ModelState`] / [`ModelInstallState`]) and the port through which the core drives it
//! ([`ModelManager`]). The core never touches a model file or a download itself: the desktop shell
//! plugs in `voltip_asr_local::ModelStore`; shells without local models (the phone) plug in
//! nothing and the library is empty.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

/// Catalogue id used when `EngineSettings.local_model` is `None` (`qwen3-asr-0.6b`, the「均衡」
/// tier of docs/dictation.md §10). The catalogue itself lives in `voltip-asr-local`; its test pins
/// this constant to its default entry.
pub const DEFAULT_LOCAL_MODEL_ID: &str = "qwen3-asr-0.6b";

/// `ModelState.capabilities` entry of a model that produces partial results while audio is still
/// arriving (docs/dictation.md §11); every other model carries [`CAPABILITY_OFFLINE`].
pub const CAPABILITY_STREAMING: &str = "streaming";
/// `ModelState.capabilities` entry of a whole-take model.
pub const CAPABILITY_OFFLINE: &str = "offline";
/// `ModelState.capabilities` entry of the voice activity detector (docs/dictation.md §12): an
/// auxiliary entry the settings dialog never shows as a card.
pub const CAPABILITY_VAD: &str = "vad";

/// Where one catalogue entry is on disk (`ModelState.state`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelInstallState {
    /// Nothing (or only a resumable `.part`) on disk.
    #[default]
    NotInstalled,
    /// A file is being fetched.
    Downloading {
        /// Bytes of `file` on disk so far.
        received: u64,
        /// Size of `file` from the catalogue.
        total: u64,
        /// File name within the model directory.
        file: String,
    },
    /// Every file is on disk; the sha256 check is running.
    Verifying,
    /// All files verified; `manifest.json` written.
    Installed {
        /// Model directory.
        path: String,
        /// Unix time in seconds when the manifest was written.
        installed_at: u64,
    },
    /// The last download or verification failed; `.part` files are kept for a retry.
    Failed {
        /// Human-readable reason.
        message: String,
    },
    /// A manual import (docs/dictation.md §10) found files missing from the model directory or
    /// files that are not the catalogue's: the card names them.
    ImportIncomplete {
        /// Catalogue files not in the directory, or not of their catalogue size.
        missing: Vec<String>,
        /// Files of the right size whose sha256 is not the catalogue's.
        mismatched: Vec<String>,
    },
}

/// Why a manual import did not install a model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelImportError {
    /// The directory does not hold every catalogue file as it should.
    Incomplete {
        /// Files absent, or not of their catalogue size.
        missing: Vec<String>,
        /// Files whose sha256 is not the catalogue's.
        mismatched: Vec<String>,
    },
    /// Anything else (unknown id, file system trouble).
    Failed(String),
}

impl ModelInstallState {
    /// The files are on disk and verified.
    pub fn is_installed(&self) -> bool {
        matches!(self, Self::Installed { .. })
    }
}

impl ModelState {
    /// Whether the model can produce partial results while recording (`capabilities` names
    /// [`CAPABILITY_STREAMING`]).
    pub fn is_streaming(&self) -> bool {
        self.capabilities.iter().any(|c| c == CAPABILITY_STREAMING)
    }

    /// Whether the entry is the voice activity detector (`capabilities` names [`CAPABILITY_VAD`]).
    pub fn is_vad(&self) -> bool {
        self.capabilities.iter().any(|c| c == CAPABILITY_VAD)
    }
}

/// One catalogue entry with its install state (`UiState.models[]`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelState {
    /// Catalogue id (`qwen3-asr-0.6b`, `sense-voice-small`, `zipformer-stream-zh-en`, …).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Recogniser family (`transcribe_cpp` | `sense_voice` | `paraformer` | `zipformer_streaming` | `silero_vad`).
    pub engine: String,
    /// Product tier the settings dialog groups by (`balanced` | `accurate` | `light` | `streaming`;
    /// `auxiliary` entries are dependencies and never cards).
    #[serde(default)]
    pub tier: String,
    /// What the model can do: [`CAPABILITY_OFFLINE`] and / or [`CAPABILITY_STREAMING`].
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// ISO language codes the model understands.
    pub languages: Vec<String>,
    /// Bytes on disk once installed.
    pub size_bytes: u64,
    /// One line for the model card.
    pub description: String,
    /// The catalogue's default choice.
    pub recommended: bool,
    /// Where the files come from (`owner/name` on Hugging Face); the About pane credits it.
    #[serde(default)]
    pub repo: String,
    /// The directory its files go into (`<models root>/<id>`): where a manual download puts them.
    #[serde(default)]
    pub dir: String,
    /// Its files, primary first, with the public addresses each can be downloaded from.
    #[serde(default)]
    pub files: Vec<ModelFileView>,
    /// Selected by the current `EngineSettings` in local mode.
    pub active: bool,
    /// Where it is on disk.
    pub state: ModelInstallState,
}

/// One file of a catalogue model, as the card lists it for a manual download.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelFileView {
    /// File name inside the model directory.
    pub name: String,
    /// Exact size in bytes.
    pub size_bytes: u64,
    /// Where to download it, in the order the app tries them: huggingface.co, hf-mirror.com. A
    /// build's own mirror is a host the interface never shows.
    pub urls: Vec<String>,
}

/// Cooperative cancellation for one download: `cancel()` flips the flag and wakes `cancelled()`.
#[derive(Clone, Debug, Default)]
pub struct CancelToken {
    flag: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl CancelToken {
    /// A fresh, uncancelled token.
    pub fn new() -> Self {
        Self::default()
    }

    /// Request cancellation (idempotent).
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    /// Whether cancellation was requested.
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// Resolves once cancellation is requested (immediately when it already was).
    pub async fn cancelled(&self) {
        while !self.is_cancelled() {
            let notified = self.notify.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

/// Receives install-state transitions while a download runs (`Downloading` many times, then
/// `Verifying`, then the final `Installed` / `Failed`).
pub type ProgressSink = Arc<dyn Fn(ModelInstallState) + Send + Sync>;

/// The model library as the core drives it. Implemented by `voltip_asr_local::ModelStore`.
#[async_trait]
pub trait ModelManager: Send + Sync {
    /// Every catalogue entry with its install state. `active` is left `false`: the core knows the
    /// settings.
    fn scan(&self) -> Vec<ModelState>;

    /// Fetch and verify `id`. Progress goes to `progress`; `cancel` stops it between chunks (the
    /// `.part` files stay for a resume). Returns the final state (`Installed`) or the reason.
    async fn download(&self, id: &str, progress: ProgressSink, cancel: CancelToken) -> Result<ModelInstallState, String>;

    /// Delete the model directory (files, `.part`s and manifest).
    fn remove(&self, id: &str) -> Result<(), String>;

    /// Install `id` from files a person downloaded into its directory (docs/dictation.md §10):
    /// check every catalogue file's size and sha256 there and write the manifest. `progress`
    /// receives `Verifying`.
    async fn import(&self, id: &str, progress: ProgressSink) -> Result<ModelInstallState, ModelImportError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_states_serialize_with_the_kind_tag() {
        for (state, wire) in [
            (ModelInstallState::NotInstalled, r#"{"kind":"not_installed"}"#),
            (
                ModelInstallState::Downloading { received: 1_048_576, total: 239_233_841, file: "model.int8.onnx".into() },
                r#"{"kind":"downloading","received":1048576,"total":239233841,"file":"model.int8.onnx"}"#,
            ),
            (ModelInstallState::Verifying, r#"{"kind":"verifying"}"#),
            (
                ModelInstallState::Installed { path: "/data/models/sense-voice-small".into(), installed_at: 1_758_700_000 },
                r#"{"kind":"installed","path":"/data/models/sense-voice-small","installed_at":1758700000}"#,
            ),
            (ModelInstallState::Failed { message: "sha256 mismatch".into() }, r#"{"kind":"failed","message":"sha256 mismatch"}"#),
        ] {
            assert_eq!(serde_json::to_string(&state).unwrap(), wire);
            assert_eq!(serde_json::from_str::<ModelInstallState>(wire).unwrap(), state);
            assert_eq!(state.is_installed(), matches!(state, ModelInstallState::Installed { .. }));
        }
        assert_eq!(ModelInstallState::default(), ModelInstallState::NotInstalled);
        let model = ModelState {
            id: DEFAULT_LOCAL_MODEL_ID.into(),
            name: "均衡".into(),
            engine: "transcribe_cpp".into(),
            tier: "balanced".into(),
            capabilities: vec![CAPABILITY_OFFLINE.into()],
            languages: vec!["zh".into(), "en".into()],
            size_bytes: 690_417_824,
            description: "d".into(),
            recommended: true,
            repo: "example/model".into(),
            dir: String::new(),
            files: Vec::new(),
            active: false,
            state: ModelInstallState::NotInstalled,
        };
        let json = serde_json::to_string(&model).unwrap();
        assert!(
            json.starts_with(
                r#"{"id":"qwen3-asr-0.6b","name":"均衡","engine":"transcribe_cpp","tier":"balanced","capabilities":["offline"],"languages":["zh","en"]"#
            ),
            "{json}"
        );
        assert_eq!(serde_json::from_str::<ModelState>(&json).unwrap(), model);
        assert!(!model.is_streaming() && !model.is_vad());
        assert!(ModelState { capabilities: vec![CAPABILITY_STREAMING.into()], ..model.clone() }.is_streaming());
        assert!(ModelState { capabilities: vec![CAPABILITY_VAD.into()], ..model.clone() }.is_vad());
        // A `models` list written before tiers and capabilities existed still parses.
        let legacy: ModelState = serde_json::from_str(
            r#"{"id":"sense-voice-small","name":"S","engine":"sense_voice","languages":[],"size_bytes":1,"description":"","recommended":false,"active":false,"state":{"kind":"not_installed"}}"#,
        )
        .unwrap();
        assert_eq!(legacy.tier, "");
        assert!(legacy.capabilities.is_empty() && !legacy.is_streaming());
        // …and one from before the manual download (§10.8): no folder, no files to list.
        assert!(legacy.dir.is_empty() && legacy.files.is_empty());
        assert_eq!(DEFAULT_LOCAL_MODEL_ID, "qwen3-asr-0.6b");
        assert_eq!((CAPABILITY_OFFLINE, CAPABILITY_STREAMING, CAPABILITY_VAD), ("offline", "streaming", "vad"));
    }

    #[tokio::test]
    async fn cancel_token_flips_once_and_wakes_waiters() {
        let token = CancelToken::new();
        assert!(!token.is_cancelled());
        let waiter = token.clone();
        let task = tokio::spawn(async move { waiter.cancelled().await });
        tokio::task::yield_now().await;
        token.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(5), task).await.unwrap().unwrap();
        assert!(token.is_cancelled());
        token.cancel();
        // Already cancelled: resolves at once.
        tokio::time::timeout(std::time::Duration::from_secs(1), token.cancelled()).await.unwrap();
        assert!(format!("{token:?}").contains("CancelToken"));
    }
}
