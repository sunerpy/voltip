//! The local speech service (docs/dictation.md §23): an OpenAI-compatible transcription endpoint
//! on this computer, for other programs (Paseo's dictation, scripts) to use Voltip's recognition,
//! dictionary, presets and scenes. The same take gets the same text as a take of the app: the
//! pipeline's steps are shared ([`crate::dictation::steps`]).
//!
//! Two hosts run it: the headless `voltip-server` (it reads the app's files, [`FileSource`]) and the
//! desktop app when its service is switched on (it pushes its own state, [`PushedState`]). The HTTP
//! layer lives in the `voltip-serve` crate behind [`SpeechService`]; nothing here opens a socket.

mod host;
mod profile;
mod service;
mod token;

#[cfg(test)]
mod tests;

pub use host::{ListenConfig, RunningServer, ServeHost, SpeechService};
pub use profile::{
    Catalog, DefaultChoices, Defaults, MODEL_DEFAULT, MODEL_PREFIX, MODEL_RAW, ModelInfo, Profile, ProfileError, Recipe, find_preset, find_scene, model_list,
    parse_model, resolve_recipe,
};
pub use service::{
    EngineOverrides, FileSource, FileSourceConfig, MAX_MINUTES, PcmFile, PushedState, RATE, ServeError, ServeOutcome, ServeRequest, ServeState, Service,
    StateSource, not_ready,
};
pub use token::{
    SERVE_DIR, TOKEN_FILE, UPLOADS_DIR, create_private_dir, default_token_path, load_or_create_token, new_token, rotate_token, serve_dir, uploads_dir,
};

/// The port the service listens on unless told otherwise (not the LAN host's
/// [`crate::DEFAULT_LAN_PORT`]).
pub const DEFAULT_PORT: u16 = 47840;
/// Takes processed at the same time unless told otherwise.
pub const DEFAULT_CONCURRENCY: usize = 2;
/// Most takes processed at the same time.
pub const MAX_CONCURRENCY: usize = 8;
