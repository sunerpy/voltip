//! The ports between the local speech service and what serves it over HTTP (docs/dictation.md
//! §23): [`SpeechService`] is what the HTTP layer calls (the core's [`Service`] implements it,
//! tests plug in their own), [`ServeHost`] is how the app starts and stops a listener for it
//! without the core knowing about sockets.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

use super::profile::ModelInfo;
use super::service::{PcmFile, ServeError, ServeOutcome, ServeRequest, Service};
use crate::models::CancelToken;

/// What the HTTP layer asks of the service.
#[async_trait]
pub trait SpeechService: Send + Sync {
    /// Every processing a request can select (`GET /v1/models`).
    fn models(&self) -> Vec<ModelInfo>;
    /// Whether recognition can run now; the reason when it cannot (`/healthz`, 503).
    fn ready(&self) -> Result<(), String>;
    /// Process one take; `cancel` fires when the client went away.
    async fn transcribe(&self, audio: PcmFile, request: ServeRequest, cancel: CancelToken) -> Result<ServeOutcome, ServeError>;
}

#[async_trait]
impl SpeechService for Service {
    fn models(&self) -> Vec<ModelInfo> {
        Service::models(self)
    }

    fn ready(&self) -> Result<(), String> {
        Service::ready(self)
    }

    async fn transcribe(&self, audio: PcmFile, request: ServeRequest, cancel: CancelToken) -> Result<ServeOutcome, ServeError> {
        Service::transcribe(self, &audio, &request, &cancel).await
    }
}

/// How a listener is set up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListenConfig {
    /// Where it listens.
    pub addr: SocketAddr,
    /// The bearer token every request but `/healthz` must carry.
    pub token: String,
    /// Takes processed at the same time; twice as many wait.
    pub concurrency: usize,
    /// Longest audio of one request, in minutes.
    pub max_minutes: u16,
    /// Where uploads are decoded to (cleared when the listener starts).
    pub uploads_dir: PathBuf,
}

/// A listener that is running.
pub trait RunningServer: Send + Sync {
    /// The address it is bound to.
    fn addr(&self) -> SocketAddr;
    /// Requests from now on must carry `token`.
    fn set_token(&self, token: String);
    /// Stop listening; requests in progress are cancelled.
    fn stop(&self);
}

/// Starts listeners (the desktop shell implements it with the HTTP layer; the phone has none).
#[async_trait]
pub trait ServeHost: Send + Sync {
    /// Bind and serve `service` with `config`; the reason (the port is taken) when it cannot.
    async fn start(&self, config: ListenConfig, service: Arc<dyn SpeechService>) -> Result<Box<dyn RunningServer>, String>;
}
