//! Moving the model library (docs/dictation.md §10.9, issue #192): the command, its progress, and
//! where the library is, for the UI. The files move through the port
//! ([`crate::ModelManager::relocate`]); the core keeps `models-location.json` in step with them —
//! written by the port's commit before the library switches, its leftovers pruned once the old
//! copies are gone.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{CoreEvent, ModelProgress, Runtime};
use crate::CoreError;
use crate::models::{CancelToken, RelocateCommit, RelocateProgress, Relocated};
use crate::models_location::{self, ModelsLocationView, ModelsMove, ModelsMoveOutcome};

/// A move's progress reaches the UI at most this often…
const MOVE_PROGRESS_INTERVAL: Duration = Duration::from_millis(250);
/// …unless this many more bytes were copied.
const MOVE_PROGRESS_BYTES: u64 = 16 * 1024 * 1024;

/// The move running now.
pub(super) struct Relocation {
    cancel: CancelToken,
    /// Where the library was.
    from: PathBuf,
    progress: ModelsMove,
}

/// The runtime's moves: the one running, and how the last one ended.
#[derive(Default)]
pub(super) struct Moves {
    running: Option<Relocation>,
    last: Option<ModelsMoveOutcome>,
}

impl Moves {
    /// A move runs: nothing else may write into the library meanwhile.
    pub(super) fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// The core is stopping: the move undoes itself on its thread.
    pub(super) fn stop(&mut self) {
        if let Some(running) = self.running.take() {
            running.cancel.cancel();
        }
    }
}

/// Why a model command waits for a move.
pub(super) const MOVING: &str = "models: 正在移动模型，请稍后再试";

impl Runtime {
    /// `UiState.models_location`.
    pub(super) fn models_location_view(&self) -> ModelsLocationView {
        let Some(manager) = &self.models else { return ModelsLocationView::default() };
        let default = models_location::default_root(&self.config.data_dir);
        let dir = manager.root();
        ModelsLocationView {
            available: true,
            custom: dir != default,
            dir: dir.to_string_lossy().into_owned(),
            default_dir: default.to_string_lossy().into_owned(),
            moving: self.moves.running.as_ref().map(|r| r.progress.clone()),
            last: self.moves.last.clone(),
        }
    }

    pub(super) fn emit_models_location(&self) {
        self.emit(CoreEvent::ModelsLocation(self.models_location_view()));
    }

    /// `ModelsRelocate`: move the library on a task; `None` is the default folder. The location
    /// file names the new folder only once every model is complete there (the port's commit), and
    /// before anything reads them there.
    pub(super) fn models_relocate(&mut self, to: Option<PathBuf>) -> Result<(), CoreError> {
        let Some(manager) = self.models.clone() else { return Err(CoreError::Invalid("models: 本地模型不可用".into())) };
        if self.moves.is_running() {
            return Err(CoreError::Invalid("models: 正在移动模型".into()));
        }
        if !self.downloads.is_empty() {
            return Err(CoreError::Invalid("models: 正在下载模型，请等下载完成或取消后再移动".into()));
        }
        let data_dir = self.config.data_dir.clone();
        let default = models_location::default_root(&data_dir);
        let to = to.unwrap_or_else(|| default.clone());
        if !to.is_absolute() {
            return Err(CoreError::Invalid(format!("models: {} 不是完整的路径", to.display())));
        }
        let from = manager.root();
        self.moves.last = None;
        if to == from {
            self.emit_models_location();
            return Ok(());
        }
        let cancel = CancelToken::new();
        let tx = self.model_tx.clone();
        let progress_tx = tx.clone();
        let last = parking_lot::Mutex::new((Instant::now() - MOVE_PROGRESS_INTERVAL, 0u64));
        let progress: RelocateProgress = Arc::new(move |moved, total| {
            let mut last = last.lock();
            let now = Instant::now();
            if moved < total && now.duration_since(last.0) < MOVE_PROGRESS_INTERVAL && moved.saturating_sub(last.1) < MOVE_PROGRESS_BYTES {
                return;
            }
            *last = (now, moved);
            let _ = progress_tx.try_send(ModelProgress::Moving { moved, total });
        });
        let commit_dir = data_dir.clone();
        let commit: RelocateCommit = Arc::new(move |to: &Path, doomed: &[PathBuf]| {
            let mut location = models_location::read(&commit_dir);
            // A folder an earlier move left behind in the new place is the library now.
            location.leftovers.retain(|dir| !dir.starts_with(to));
            location.leftovers.extend(doomed.iter().cloned());
            location.dir = (to != models_location::default_root(&commit_dir)).then(|| to.to_path_buf());
            models_location::write(&commit_dir, &location).map_err(|e| e.to_string())
        });
        // An earlier move's old copies in the new place (moving straight back): try them again,
        // so that the move does not find them in the way.
        let stale: Vec<PathBuf> = models_location::read(&data_dir).leftovers.into_iter().filter(|dir| dir.starts_with(&to)).collect();
        let (task_to, task_cancel) = (to.clone(), cancel.clone());
        tokio::spawn(async move {
            if !stale.is_empty() {
                let _ = tokio::task::spawn_blocking(move || {
                    for dir in stale {
                        let _ = std::fs::remove_dir_all(dir);
                    }
                })
                .await;
            }
            let result = manager.relocate(&task_to, progress, task_cancel.clone(), commit).await;
            let _ = tx.send(ModelProgress::Moved { result, cancelled: task_cancel.is_cancelled() }).await;
        });
        tracing::info!(from = %from.display(), to = %to.display(), "moving the models");
        self.moves.running = Some(Relocation { cancel, from, progress: ModelsMove { to: to.to_string_lossy().into_owned(), moved_bytes: 0, total_bytes: 0 } });
        self.emit_models_location();
        Ok(())
    }

    /// `ModelsRelocateCancel`.
    pub(super) fn models_relocate_cancel(&mut self) -> Result<(), CoreError> {
        let Some(running) = &self.moves.running else { return Err(CoreError::Invalid("models: 没有在移动模型".into())) };
        running.cancel.cancel();
        tracing::info!("moving the models: cancel requested");
        Ok(())
    }

    pub(super) fn on_models_moving(&mut self, moved: u64, total: u64) {
        if let Some(running) = &mut self.moves.running {
            running.progress.moved_bytes = moved;
            running.progress.total_bytes = total;
            self.emit_models_location();
        }
    }

    pub(super) fn on_models_moved(&mut self, result: Result<Relocated, String>, cancelled: bool) {
        let Some(running) = self.moves.running.take() else { return };
        let outcome = match result {
            Ok(relocated) => {
                let data_dir = &self.config.data_dir;
                // The old copies that went leave the list; one still in use stays for the next start.
                let mut location = models_location::read(data_dir);
                let listed = location.leftovers.len();
                location.leftovers.retain(|dir| dir.exists());
                if location.leftovers.len() != listed
                    && let Err(e) = models_location::write(data_dir, &location)
                {
                    tracing::warn!(error = %e, "the models' location could not be updated");
                }
                // The default folder goes too once a move left it empty.
                let default = models_location::default_root(data_dir);
                if running.from == default {
                    let _ = std::fs::remove_dir(&default);
                }
                let dir = self.models.as_ref().map(|m| m.root()).unwrap_or_default();
                tracing::info!(dir = %dir.display(), deferred = relocated.leftovers.len(), "the models moved");
                ModelsMoveOutcome::Moved { dir: dir.to_string_lossy().into_owned(), deferred: u32::try_from(relocated.leftovers.len()).unwrap_or(u32::MAX) }
            }
            Err(e) if cancelled => {
                tracing::info!(reason = %e, "moving the models was cancelled");
                ModelsMoveOutcome::Cancelled
            }
            Err(message) => {
                tracing::warn!(error = %message, "moving the models failed");
                ModelsMoveOutcome::Failed { message }
            }
        };
        self.moves.last = Some(outcome);
        // Every model's folder may have changed: the cards, and the engines that read them.
        self.rescan_models();
        self.emit_models_location();
    }
}
