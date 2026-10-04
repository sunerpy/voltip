//! Voltip's local speech service over HTTP (docs/dictation.md §23): an OpenAI-compatible
//! `POST /v1/audio/transcriptions` plus `GET /v1/models` and `GET /healthz`, for other programs on
//! this computer. The processing is the core's ([`voltip_core::serve`], behind
//! [`SpeechService`]); this crate is the adapter: routes, the bearer token, admission, the upload
//! decoded as it arrives, the OpenAI response and error shapes.
//!
//! Two hosts use it: the headless `voltip-server` (apps/server) through [`ServeHandle::serve`],
//! and the desktop app through [`HttpHost`], the core's [`ServeHost`] port.

mod admission;
mod auth;
mod openai;
mod resample;
mod upload;
mod wav;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use async_trait::async_trait;
use axum::Router;
use axum::extract::multipart::MultipartRejection;
use axum::extract::{DefaultBodyLimit, Multipart, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use parking_lot::Mutex;
use tokio::sync::oneshot;
use voltip_core::CancelToken;
use voltip_core::serve::{ListenConfig, PcmFile, RunningServer, ServeHost, SpeechService};

pub use admission::Admission;
pub use auth::Token;
pub use openai::{ApiError, ResponseFormat};
pub use resample::StreamResampler;
pub use upload::{IDLE, MAX_FIELD_BYTES, MAX_PARTS, MAX_TRAILING_BYTES};
pub use wav::{HEADER_WINDOW, MAX_CHANNELS, RATES};

/// Everything a request needs.
struct App {
    service: Arc<dyn SpeechService>,
    admission: Admission,
    max_samples: u64,
    uploads: PathBuf,
    /// The cancel tokens of the takes being processed, so stopping the service cancels them.
    active: Mutex<HashMap<u64, CancelToken>>,
    next: AtomicU64,
}

/// Cancels a take's processing when the request handler goes away (the client disconnected) and
/// forgets it either way.
struct Active<'a> {
    app: &'a App,
    id: u64,
    cancel: CancelToken,
}

impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.app.active.lock().remove(&self.id);
    }
}

/// The service's routes and state.
#[derive(Clone)]
pub struct ServeHandle {
    app: Arc<App>,
    token: Token,
}

impl std::fmt::Debug for ServeHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServeHandle").field("uploads", &self.app.uploads).field("max_samples", &self.app.max_samples).finish_non_exhaustive()
    }
}

/// Remove what an earlier run left in `uploads` (it ended mid-request); create it (0700) when it
/// is not there. Returns how many files went.
pub fn clear_uploads(uploads: &Path) -> std::io::Result<usize> {
    voltip_core::serve::create_private_dir(uploads)?;
    let mut removed = 0;
    for entry in std::fs::read_dir(uploads)?.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "pcm") && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

impl ServeHandle {
    /// The service with `config`'s token, limits and uploads directory (cleared now).
    pub fn new(config: &ListenConfig, service: Arc<dyn SpeechService>) -> std::io::Result<Self> {
        let removed = clear_uploads(&config.uploads_dir)?;
        if removed > 0 {
            tracing::info!(removed, "removed the uploads an earlier run left behind");
        }
        let app = App {
            service,
            admission: Admission::new(config.concurrency),
            max_samples: openai::samples_for(config.max_minutes),
            uploads: config.uploads_dir.clone(),
            active: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
        };
        Ok(Self { app: Arc::new(app), token: Token::new(config.token.clone()) })
    }

    /// The token the requests must carry now (replace it with [`Token::set`]).
    pub fn token(&self) -> Token {
        self.token.clone()
    }

    /// Cancel every take being processed (the service stops).
    pub fn cancel_all(&self) {
        for cancel in self.app.active.lock().values() {
            cancel.cancel();
        }
    }

    /// Refuse new and waiting requests (503) from now on; the takes being processed go on.
    pub fn close_queue(&self) {
        self.app.admission.close();
    }

    /// Takes being processed now.
    pub fn active(&self) -> usize {
        self.app.active.lock().len()
    }

    /// The routes: `/healthz` open, everything else behind the token.
    pub fn router(&self) -> Router {
        let protected = Router::new()
            .route("/v1/audio/transcriptions", post(transcriptions).layer(DefaultBodyLimit::disable()))
            .route("/v1/models", get(models))
            .layer(axum::middleware::from_fn_with_state(self.token.clone(), auth::require))
            .with_state(self.app.clone());
        Router::new().route("/healthz", get(healthz)).with_state(self.app.clone()).merge(protected)
    }

    /// Bind `bind` and serve until `shutdown` resolves; returns the bound address and the task,
    /// which ends once the requests in progress finished.
    pub async fn serve(
        self,
        bind: SocketAddr,
        shutdown: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> std::io::Result<(SocketAddr, tokio::task::JoinHandle<std::io::Result<()>>)> {
        let listener = tokio::net::TcpListener::bind(bind).await?;
        let addr = listener.local_addr()?;
        let router = self.router();
        let task = tokio::spawn(async move { axum::serve(listener, router).with_graceful_shutdown(shutdown).await });
        tracing::info!(%addr, "local speech service listening");
        Ok((addr, task))
    }
}

async fn healthz(State(app): State<Arc<App>>) -> Response {
    match app.service.ready() {
        Ok(()) => axum::Json(serde_json::json!({ "status": "ok" })).into_response(),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, axum::Json(serde_json::json!({ "status": "unavailable" }))).into_response(),
    }
}

async fn models(State(app): State<Arc<App>>) -> Response {
    openai::models(&app.service.models())
}

async fn transcriptions(State(app): State<Arc<App>>, multipart: Result<Multipart, MultipartRejection>) -> Response {
    let started = Instant::now();
    let id = app.next.fetch_add(1, Ordering::SeqCst);
    // A body that is not multipart is answered in the OpenAI shape too, not with axum's plain text.
    let result = match multipart {
        Ok(multipart) => transcribe(&app, id, multipart).await,
        Err(rejection) => Err(ApiError::invalid("invalid_multipart", format!("请求体须为 multipart/form-data：{}", rejection.body_text()))),
    };
    match result {
        Ok(response) => response,
        Err(error) => {
            tracing::info!(request = id, status = error.status.as_u16(), code = error.code, ms = started.elapsed().as_millis() as u64, "transcription refused");
            error.into_response()
        }
    }
}

async fn transcribe(app: &Arc<App>, id: u64, multipart: Multipart) -> Result<Response, ApiError> {
    let started = Instant::now();
    // The permit first: a request waiting for one has not read a byte of its body.
    let permit = app.admission.admit().await?;
    // Known as active before its body is read, so that stopping the service (the app's switch,
    // `cancel_all`) also ends an upload in progress; one that slipped in after the queue closed
    // ends here.
    let cancel = CancelToken::new();
    app.active.lock().insert(id, cancel.clone());
    let active = Active { app: app.as_ref(), id, cancel: cancel.clone() };
    if app.admission.is_closed() {
        return Err(ApiError::unavailable("shutting_down", "服务正在停止"));
    }
    let upload = tokio::select! {
        upload = upload::read(multipart, &app.uploads, app.max_samples) => upload?,
        () = cancel.cancelled() => return Err(ApiError::unavailable("shutting_down", "服务正在停止")),
    };
    let (service, format, model) = (app.service.clone(), upload.format, upload.request.model.clone());
    let pcm = upload.pcm;
    let audio = PcmFile { path: pcm.path().to_path_buf(), samples: pcm.samples() };
    let request = upload.request;
    // The task holds the permit and the file until the processing ends, also when the client goes
    // away meanwhile (the guard above then cancels it, and it stops before the next segment).
    let task = tokio::spawn(async move {
        let _permit = permit;
        let _pcm = pcm;
        service.transcribe(audio, request, cancel).await
    });
    let result = task.await.map_err(|e| ApiError::internal(format!("处理任务失败：{e}")))?;
    drop(active);
    let outcome = result?;
    tracing::info!(
        request = id,
        model = model.as_deref().unwrap_or(""),
        audio_ms = outcome.duration_ms,
        segments = outcome.segments,
        asr_model = outcome.asr_model.as_deref().unwrap_or(""),
        refined = outcome.refined,
        ms = started.elapsed().as_millis() as u64,
        "transcription done"
    );
    Ok(openai::transcription(&outcome, format))
}

/// A listener the desktop app started ([`HttpHost`]).
struct Running {
    addr: SocketAddr,
    handle: ServeHandle,
    stop: Mutex<Option<oneshot::Sender<()>>>,
}

impl RunningServer for Running {
    fn addr(&self) -> SocketAddr {
        self.addr
    }

    fn set_token(&self, token: String) {
        self.handle.token().set(token);
    }

    fn stop(&self) {
        self.handle.close_queue();
        self.handle.cancel_all();
        if let Some(stop) = self.stop.lock().take() {
            let _ = stop.send(());
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The core's [`ServeHost`] over this crate: what the desktop app starts its service with.
#[derive(Clone, Copy, Debug, Default)]
pub struct HttpHost;

#[async_trait]
impl ServeHost for HttpHost {
    async fn start(&self, config: ListenConfig, service: Arc<dyn SpeechService>) -> Result<Box<dyn RunningServer>, String> {
        let handle = ServeHandle::new(&config, service).map_err(|e| format!("临时目录无法使用：{e}"))?;
        let (tx, rx) = oneshot::channel::<()>();
        let (addr, _task) = handle
            .clone()
            .serve(config.addr, async move {
                let _ = rx.await;
            })
            .await
            .map_err(|e| format!("无法监听 {}：{e}", config.addr))?;
        Ok(Box::new(Running { addr, handle, stop: Mutex::new(Some(tx)) }))
    }
}
