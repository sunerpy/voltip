//! The model library's command-line actions (docs/dictation.md §13): `--list-models`,
//! `--download-model` and `--list-compute`, shared by the desktop binary and the headless
//! `voltip-server`. Each writes to the streams it is given and returns its [`ExitCode`].

use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};

use voltip_core::{CancelToken, ModelInstallState, ProgressSink};

use crate::{ModelStore, StoreError};

/// Exit code of a headless action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitCode {
    /// Done.
    Ok = 0,
    /// The model is missing or recognition failed (nothing on stdout).
    Failed = 1,
    /// The input could not be read (nothing on stdout).
    BadInput = 2,
}

impl ExitCode {
    /// Process exit status.
    pub fn code(self) -> i32 {
        self as i32
    }
}

/// `--download-model <id>` over an injected store (tests point it at a local server with a tiny
/// catalogue). A progress line goes to `err` each time a file crosses another 10 %; success prints
/// `id<TAB>installed<TAB>dir` to `out`. An unknown id is [`ExitCode::BadInput`], every other
/// failure (all sources down, sha256 mismatch, disk) [`ExitCode::Failed`] with the reason on `err`;
/// `.part` files stay for the next run to resume. The sink runs on the download task, so it only
/// formats and forwards; every write to `out` / `err` happens on the calling thread.
pub fn download_model(store: &ModelStore, id: &str, out: &mut dyn Write, err: &mut dyn Write) -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            let _ = writeln!(err, "voltip: runtime: {e}");
            return ExitCode::Failed;
        }
    };
    let (lines, mut incoming) = tokio::sync::mpsc::unbounded_channel::<String>();
    let last: Mutex<(String, u64)> = Mutex::new((String::new(), u64::MAX));
    let progress: ProgressSink = Arc::new(move |state| {
        let ModelInstallState::Downloading { received, total, file } = state else { return };
        let decile = received.saturating_mul(10).checked_div(total).map_or(10, |d| d.min(10));
        let Ok(mut last) = last.lock() else { return };
        if last.0 != file || last.1 != decile {
            let _ = lines.send(format!("voltip: {file} {received}/{total} ({}%)", decile * 10));
            *last = (file, decile);
        }
    });
    let result = runtime.block_on(async {
        let download = store.download(id, progress, CancelToken::new());
        tokio::pin!(download);
        loop {
            tokio::select! {
                result = &mut download => {
                    while let Ok(line) = incoming.try_recv() {
                        let _ = writeln!(err, "{line}");
                    }
                    break result;
                }
                Some(line) = incoming.recv() => {
                    let _ = writeln!(err, "{line}");
                }
            }
        }
    });
    match result {
        Ok(ModelInstallState::Installed { path, .. }) => {
            let _ = writeln!(out, "{id}\tinstalled\t{path}");
            ExitCode::Ok
        }
        Ok(other) => {
            let _ = writeln!(err, "voltip: {id}: unexpected final state {other:?}");
            ExitCode::Failed
        }
        Err(e @ StoreError::UnknownModel(_)) => {
            let _ = writeln!(err, "voltip: {e}");
            ExitCode::BadInput
        }
        Err(e) => {
            let _ = writeln!(err, "voltip: {id}: {e}");
            ExitCode::Failed
        }
    }
}

/// `--list-models`: one line per catalogue entry, `id<TAB>state<TAB>name`.
pub fn list_models(models_root: &Path, out: &mut dyn Write) -> ExitCode {
    for m in ModelStore::new(models_root.to_path_buf()).scan() {
        let state = match &m.state {
            voltip_core::ModelInstallState::Installed { .. } => "installed",
            voltip_core::ModelInstallState::NotInstalled => "not_installed",
            voltip_core::ModelInstallState::Downloading { .. } => "downloading",
            voltip_core::ModelInstallState::Verifying => "verifying",
            voltip_core::ModelInstallState::Failed { .. } => "failed",
            voltip_core::ModelInstallState::ImportIncomplete { .. } => "import_incomplete",
        };
        let _ = writeln!(out, "{}\t{state}\t{}", m.id, m.name);
    }
    ExitCode::Ok
}

/// `--list-compute`: the CPU's threads, then every GPU this build can run the models on.
pub fn list_compute(out: &mut dyn Write) -> ExitCode {
    let machine = crate::hardware();
    let _ = writeln!(out, "cpu\t{}", machine.cpu_threads);
    for g in machine.gpus {
        let _ = writeln!(out, "gpu\t{}\t{}\t{}\t{}", g.name, g.description, g.kind, g.memory_total / (1024 * 1024));
    }
    ExitCode::Ok
}
