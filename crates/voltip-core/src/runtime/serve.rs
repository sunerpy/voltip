//! The local speech service the app hosts (docs/dictation.md §23.6): switched on in the settings,
//! it listens on 127.0.0.1 through the shell's [`ServeHost`] and processes requests with the app's
//! own state — the clients of the dictation engine (so a local model is loaded once), its lists and
//! settings, pushed again whenever they change. No take of the service is written to the history.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use super::{CoreEvent, Runtime};
use crate::CoreError;
use crate::presets::PresetId;
use crate::serve::{
    DEFAULT_CONCURRENCY, Defaults, ListenConfig, MAX_MINUTES, PushedState, RunningServer, ServeHost, ServeState, Service, SpeechService, default_token_path,
    load_or_create_token, rotate_token, uploads_dir,
};
use crate::settings::{MIN_SERVE_PORT, ServeSettings};
use crate::ui::{ServePhase, ServeStatus};
use crate::vocabulary::Vocabulary;

/// Why a service command is refused where the shell cannot host the service (the phone).
pub const SERVE_UNAVAILABLE: &str = "serve: 本机服务仅在电脑上提供";

/// The service's part of the runtime.
#[derive(Default)]
pub(super) struct ServeRuntime {
    host: Option<Arc<dyn ServeHost>>,
    service: Option<(Arc<Service>, Arc<PushedState>)>,
    running: Option<Box<dyn RunningServer>>,
    port: Option<u16>,
    status: ServeStatus,
}

impl ServeRuntime {
    /// With the shell's host, if it has one.
    pub(super) fn new(host: Option<Arc<dyn ServeHost>>) -> Self {
        Self { status: ServeStatus { available: host.is_some(), ..ServeStatus::default() }, host, ..Self::default() }
    }

    /// `UiState.serve` now.
    pub(super) fn status(&self) -> ServeStatus {
        self.status.clone()
    }
}

/// The processing of the app's `voltip` requests: its service preset and scene.
fn defaults(settings: &ServeSettings) -> Defaults {
    Defaults { scene: settings.scene, preset: settings.preset, ..Defaults::default() }
}

impl Runtime {
    fn serve_state(&self) -> ServeState {
        ServeState {
            settings: self.settings.clone(),
            presets: Arc::new(self.presets.presets().to_vec()),
            scenes: Arc::new(self.scenes.scenes().to_vec()),
            vocabulary: Arc::new(Vocabulary::compile(self.dictionary.entries(), self.rules.rules())),
            engines: self.resolved_engines(),
            transcriber: self.dictation.transcriber(),
            refiner: self.dictation.refiner(),
            segmenter: self.dictation.segmenter(),
        }
    }

    /// Hand the running service the app's state as it is now (after a change of the settings,
    /// the engines or the lists).
    pub(super) fn push_serve_state(&self) {
        if let Some((service, state)) = &self.serve.service {
            state.set(self.serve_state());
            service.set_defaults(defaults(&self.settings.serve));
        }
    }

    fn set_serve_status(&mut self, phase: ServePhase, address: Option<String>, error: Option<String>) {
        self.serve.status = ServeStatus { available: self.serve.host.is_some(), phase, address, error };
        self.emit(CoreEvent::Serve(self.serve.status.clone()));
    }

    /// Start, stop or restart the listener as `Settings.serve` says.
    pub(super) async fn apply_serve(&mut self) {
        let Some(host) = self.serve.host.clone() else { return };
        let wanted = self.settings.serve.clone();
        if !wanted.enabled {
            self.stop_serve();
            if self.serve.status.phase != ServePhase::Off {
                self.set_serve_status(ServePhase::Off, None, None);
            }
            return;
        }
        if self.serve.running.is_some() && self.serve.port == Some(wanted.port) {
            self.push_serve_state();
            return;
        }
        self.stop_serve();
        let token = match load_or_create_token(&default_token_path(&self.config.data_dir)) {
            Ok((token, warning)) => {
                if let Some(warning) = warning {
                    tracing::warn!("{warning}");
                }
                token
            }
            Err(e) => {
                self.set_serve_status(ServePhase::Failed, None, Some(e));
                return;
            }
        };
        self.set_serve_status(ServePhase::Starting, None, None);
        let state = Arc::new(PushedState::new(self.serve_state()));
        let service = Arc::new(Service::new(state.clone(), defaults(&wanted)));
        let config = ListenConfig {
            addr: SocketAddr::from((Ipv4Addr::LOCALHOST, wanted.port)),
            token,
            concurrency: DEFAULT_CONCURRENCY,
            max_minutes: MAX_MINUTES,
            uploads_dir: uploads_dir(&self.config.data_dir),
        };
        match host.start(config, service.clone() as Arc<dyn SpeechService>).await {
            Ok(running) => {
                let address = format!("http://{}/v1", running.addr());
                tracing::info!(%address, "local speech service on");
                self.serve.port = Some(wanted.port);
                self.serve.running = Some(running);
                self.serve.service = Some((service, state));
                self.set_serve_status(ServePhase::Running, Some(address), None);
            }
            Err(e) => {
                tracing::warn!(error = %e, "the local speech service did not start");
                self.set_serve_status(ServePhase::Failed, None, Some(e));
            }
        }
    }

    /// Stop listening (the app quits, the service is switched off or moves to another port).
    pub(super) fn stop_serve(&mut self) {
        if let Some(running) = self.serve.running.take() {
            running.stop();
        }
        self.serve.service = None;
        self.serve.port = None;
    }

    /// `SetServe`: check, persist, apply.
    pub(super) async fn set_serve(&mut self, serve: ServeSettings) -> Result<(), CoreError> {
        if self.serve.host.is_none() {
            return Err(CoreError::Invalid(SERVE_UNAVAILABLE.to_owned()));
        }
        if serve.port < MIN_SERVE_PORT {
            return Err(CoreError::Invalid(format!("serve: 端口须在 {MIN_SERVE_PORT}–65535 之间")));
        }
        if let Some(id) = serve.scene
            && !self.scenes.scenes().iter().any(|s| s.id == id)
        {
            return Err(CoreError::Invalid(format!("serve: 没有 id 为 {id} 的场景")));
        }
        if let Some(PresetId::Custom(id)) = serve.preset
            && !self.presets.presets().iter().any(|p| p.id == id)
        {
            return Err(CoreError::Invalid(format!("serve: 没有 id 为 {id} 的预设")));
        }
        self.settings.serve = serve;
        self.save_settings()?;
        self.apply_serve().await;
        Ok(())
    }

    /// `ServeCopyToken`: the token goes to the clipboard, never to the webview.
    pub(super) async fn copy_serve_token(&mut self) -> Result<(), CoreError> {
        if self.serve.host.is_none() {
            return Err(CoreError::Invalid(SERVE_UNAVAILABLE.to_owned()));
        }
        let (token, _) = load_or_create_token(&default_token_path(&self.config.data_dir)).map_err(CoreError::Invalid)?;
        let injector = self.dictation.injector();
        tokio::task::spawn_blocking(move || injector.copy(&token))
            .await
            .map_err(|e| CoreError::Invalid(format!("serve: 复制任务失败：{e}")))?
            .map_err(|e| CoreError::Invalid(format!("serve: 令牌无法复制到剪贴板：{e}")))
    }

    /// `ServeRotateToken`: a new token, in force at once; clients need it from now on.
    pub(super) fn rotate_serve_token(&mut self) -> Result<(), CoreError> {
        if self.serve.host.is_none() {
            return Err(CoreError::Invalid(SERVE_UNAVAILABLE.to_owned()));
        }
        let token = rotate_token(&default_token_path(&self.config.data_dir)).map_err(CoreError::Invalid)?;
        if let Some(running) = &self.serve.running {
            running.set_token(token);
        }
        tracing::info!("the local speech service's token was replaced");
        Ok(())
    }
}
