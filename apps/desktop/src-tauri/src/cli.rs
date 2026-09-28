//! Command line of the desktop binary (docs/dictation.md §13).
//!
//! Two kinds of flags: remote controls for a running instance (`--toggle`, `--edit-toggle`,
//! `--cancel`), which the single-instance plugin forwards as `HotkeyEdge { source: cli }` (with
//! `purpose: edit` for the voice edit of docs/dictation.md §19) / `DictationCancel` and which
//! Wayland users bind to a compositor shortcut in place of a global hotkey; and headless
//! utilities (`--list-devices`, `--list-compute`, `--list-models`, `--download-model`,
//! `--transcribe-file`) that run
//! without a window, tray, hotkey or microphone and exit. `--start-hidden` starts the GUI without
//! the main window.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use clap::Parser;
use voltip_asr_local::{Compute, LocalDevice, LocalTranscriber, ModelStore, StoreError};
use voltip_core::dictation::Transcriber as _;
use voltip_core::{
    CancelToken, ChineseScript, CoreConfig, DEFAULT_LOCAL_MODEL_ID, DictationError, EdgeSource, ModelInstallState, ProgressSink, ProviderId, SettingsStore,
    TakeKind,
};
use voltip_tauri_bridge::UiCommand;

/// `voltip [FLAGS]`.
#[derive(Debug, Clone, PartialEq, Eq, Parser)]
#[command(name = "voltip", version = crate::APP_VERSION, about = "Voltip desktop: voice input with end-to-end encrypted device pairing.", disable_help_subcommand = true)]
pub struct Cli {
    /// Toggle dictation in the running instance (start; or stop and transcribe). Bind this to a
    /// compositor shortcut where no global hotkey exists (Wayland).
    #[arg(long, conflicts_with_all = ["cancel", "edit_toggle", "list_devices", "list_models", "transcribe_file"])]
    pub toggle: bool,
    /// Toggle a voice edit of the selected text in the running instance (docs/dictation.md §19):
    /// start (the selection is copied at once), or stop and rewrite. Bind it to a compositor
    /// shortcut where no global hotkey exists (Wayland), preferably on key release.
    #[arg(long, conflicts_with_all = ["cancel", "list_devices", "list_models", "transcribe_file"])]
    pub edit_toggle: bool,
    /// Discard the running instance's recording or pending result.
    #[arg(long, conflicts_with_all = ["list_devices", "list_models", "transcribe_file"])]
    pub cancel: bool,
    /// Start without showing the main window (tray and hotkey only).
    #[arg(long, conflicts_with_all = ["list_devices", "list_models", "transcribe_file"])]
    pub start_hidden: bool,
    /// Print the input devices (`id<TAB>name`, default first) and exit.
    #[arg(long, conflicts_with_all = ["list_models", "transcribe_file"])]
    pub list_devices: bool,
    /// Print what the local models can run on (`cpu<TAB>threads`, then one
    /// `gpu<TAB>name<TAB>description<TAB>kind<TAB>MiB` line per GPU this build drives) and exit.
    #[arg(long, conflicts_with_all = ["list_devices", "list_models", "transcribe_file", "download_model"])]
    pub list_compute: bool,
    /// Print the local model library (`id<TAB>state<TAB>name`) and exit.
    #[arg(long, conflicts_with_all = ["transcribe_file", "download_model"])]
    pub list_models: bool,
    /// Download (or resume) a catalogue model with the app's own downloader (build mirror, then
    /// huggingface.co, then hf-mirror.com; sha256-verified) and exit. Progress goes to stderr; on
    /// success stdout carries `id<TAB>installed<TAB>dir`.
    #[arg(long, value_name = "ID", conflicts_with_all = ["toggle", "cancel", "start_hidden", "list_devices", "transcribe_file"])]
    pub download_model: Option<String>,
    /// Transcribe a WAV file (16 kHz mono preferred; other rates are resampled) with a local model
    /// and exit. Headless: no window, tray, hotkey or microphone. Exit 0 with the text on stdout;
    /// non-zero with an empty stdout when the model is not installed or recognition fails.
    #[arg(long, value_name = "WAV16K")]
    pub transcribe_file: Option<PathBuf>,
    /// Catalogue id of the local model for --transcribe-file (default: the model the settings
    /// select in local mode, else the catalogue default).
    #[arg(long, value_name = "ID", requires = "transcribe_file")]
    pub model: Option<String>,
    /// With --transcribe-file: print `{ "text", "model", "latency_ms", "backend" }` instead of the
    /// bare text.
    #[arg(long, requires = "transcribe_file")]
    pub json: bool,
    /// With --transcribe-file: where the model runs (`auto`, `cpu`, `gpu`; default: the settings).
    #[arg(long, value_name = "DEVICE", value_parser = parse_device, requires = "transcribe_file")]
    pub device: Option<LocalDevice>,
    /// With --transcribe-file --device gpu: the GPU by name (see --list-compute; default: the
    /// settings, else the first GPU).
    #[arg(long, value_name = "NAME", requires = "transcribe_file")]
    pub gpu: Option<String>,
    /// With --transcribe-file: inference threads (default: the settings, else the engine's own).
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u16).range(1..=i64::from(voltip_core::MAX_LOCAL_THREADS)), requires = "transcribe_file")]
    pub threads: Option<u16>,
}

fn parse_device(text: &str) -> Result<LocalDevice, String> {
    match text {
        "auto" => Ok(LocalDevice::Auto),
        "cpu" => Ok(LocalDevice::Cpu),
        "gpu" => Ok(LocalDevice::Gpu),
        other => Err(format!("{other}: expected auto, cpu or gpu")),
    }
}

/// What the running instance is asked to do by a second `voltip` invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Remote {
    /// `--toggle` → `HotkeyEdge { pressed: true, source: cli }`.
    Toggle,
    /// `--edit-toggle` → `HotkeyEdge { pressed: true, source: cli, purpose: edit }`.
    EditToggle,
    /// `--cancel` → `DictationCancel`.
    Cancel,
}

impl Remote {
    /// The bridge command for this remote control.
    pub fn command(self) -> UiCommand {
        match self {
            Self::Toggle => {
                UiCommand::HotkeyEdge { pressed: true, at_ms: voltip_core::now_ms(), source: EdgeSource::Cli, purpose: TakeKind::Dictation, chorded: false }
            }
            Self::EditToggle => {
                UiCommand::HotkeyEdge { pressed: true, at_ms: voltip_core::now_ms(), source: EdgeSource::Cli, purpose: TakeKind::Edit, chorded: false }
            }
            Self::Cancel => UiCommand::DictationCancel,
        }
    }
}

/// What the process should do for a parsed command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Run the GUI; `remote` is applied to the running instance instead when one exists.
    Gui {
        /// `--start-hidden`.
        start_hidden: bool,
        /// `--toggle` / `--edit-toggle` / `--cancel`.
        remote: Option<Remote>,
    },
    /// `--list-devices`.
    ListDevices,
    /// `--list-compute`.
    ListCompute,
    /// `--list-models`.
    ListModels,
    /// `--download-model`.
    DownloadModel {
        /// Catalogue id.
        id: String,
    },
    /// `--transcribe-file`.
    TranscribeFile {
        /// The WAV.
        path: PathBuf,
        /// `--model`.
        model: Option<String>,
        /// `--json`.
        json: bool,
        /// `--device` / `--gpu` / `--threads`, each `None` when absent (the settings decide).
        device: Option<LocalDevice>,
        /// `--gpu`.
        gpu: Option<String>,
        /// `--threads`.
        threads: Option<u16>,
    },
}

impl Cli {
    /// Parse `argv` (including the program name); a usage error is returned, not printed.
    pub fn parse_args<I, T>(args: I) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        Self::try_parse_from(args)
    }

    /// The remote control carried by `argv`, if any (what the running instance applies when a
    /// second invocation reaches it through the single-instance plugin).
    pub fn remote(&self) -> Option<Remote> {
        if self.toggle {
            Some(Remote::Toggle)
        } else if self.edit_toggle {
            Some(Remote::EditToggle)
        } else if self.cancel {
            Some(Remote::Cancel)
        } else {
            None
        }
    }

    /// What to do.
    pub fn action(&self) -> Action {
        if let Some(path) = &self.transcribe_file {
            return Action::TranscribeFile {
                path: path.clone(),
                model: self.model.clone(),
                json: self.json,
                device: self.device,
                gpu: self.gpu.clone(),
                threads: self.threads,
            };
        }
        if self.list_devices {
            return Action::ListDevices;
        }
        if self.list_compute {
            return Action::ListCompute;
        }
        if self.list_models {
            return Action::ListModels;
        }
        if let Some(id) = &self.download_model {
            return Action::DownloadModel { id: id.clone() };
        }
        Action::Gui { start_hidden: self.start_hidden, remote: self.remote() }
    }
}

/// `--toggle` / `--edit-toggle` / `--cancel` from the arguments a second instance forwarded, or `None` when they
/// carry nothing to apply (a plain second launch just focuses the running instance).
pub fn remote_from_args(args: &[String]) -> Option<Remote> {
    Cli::parse_args(args).ok().and_then(|cli| cli.remote())
}

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

/// What `--transcribe-file --json` prints.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TranscribeOutput {
    /// Recognised text (trimmed).
    pub text: String,
    /// Catalogue id of the model that produced it.
    pub model: String,
    /// Wall time of the recognition (model load included on the first call).
    pub latency_ms: u64,
    /// The compute backend it ran on (`CPU`, `Metal`, `Vulkan0`, …).
    #[serde(default)]
    pub backend: String,
}

/// The local model `--transcribe-file` uses when `--model` is absent: the one the settings select
/// in local mode (the desktop's `settings.json` under `data_dir`), else the catalogue default.
pub fn default_model(data_dir: &Path) -> String {
    let settings = SettingsStore::new(data_dir).load().unwrap_or_default();
    let selected = settings.engines.local_model.as_deref().map(str::trim).filter(|s| !s.is_empty());
    match (settings.engines.asr_provider, selected) {
        (ProviderId::Local, Some(id)) => id.to_owned(),
        _ => DEFAULT_LOCAL_MODEL_ID.to_owned(),
    }
}

/// Recognise `path` with `transcriber` (already pointed at the model) and bring the text to
/// `script` like the pipeline does (docs/dictation.md §17). Errors are the core's
/// `DictationError`s, mapped to exit codes by [`transcribe_file`].
pub async fn transcribe_path(
    transcriber: &LocalTranscriber,
    path: &Path,
    language: Option<&str>,
    script: ChineseScript,
) -> Result<TranscribeOutput, DictationError> {
    let wav = std::fs::read(path).map_err(|e| DictationError::Audio(format!("{}: {e}", path.display())))?;
    let started = Instant::now();
    let transcript = transcriber.transcribe(&wav, language, &[]).await?;
    Ok(TranscribeOutput {
        text: voltip_core::script::normalized(script, &transcript.text),
        model: transcriber.selected().to_owned(),
        latency_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        backend: transcriber.loaded_backend().unwrap_or_default(),
    })
}

/// The whole `--transcribe-file` action over an injected transcriber: writes the text (or the
/// JSON) plus a newline to `out` on success and nothing otherwise; the reason goes to `err`.
/// `main` maps the returned code to the process exit status. Tests call this with a fake
/// recogniser; the binary passes `LocalTranscriber::new(models_root).select(model)`.
pub fn transcribe_file(
    transcriber: &LocalTranscriber,
    path: &Path,
    json: bool,
    language: Option<&str>,
    script: ChineseScript,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            let _ = writeln!(err, "voltip: runtime: {e}");
            return ExitCode::Failed;
        }
    };
    match runtime.block_on(transcribe_path(transcriber, path, language, script)) {
        Ok(output) => {
            let line = if json { serde_json::to_string(&output).unwrap_or_else(|_| output.text.clone()) } else { output.text.clone() };
            let _ = writeln!(out, "{line}");
            ExitCode::Ok
        }
        Err(DictationError::Audio(reason)) => {
            let _ = writeln!(err, "voltip: {reason}");
            ExitCode::BadInput
        }
        Err(e) => {
            let _ = writeln!(err, "voltip: {e}");
            ExitCode::Failed
        }
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
        };
        let _ = writeln!(out, "{}\t{state}\t{}", m.id, m.name);
    }
    ExitCode::Ok
}

/// `--list-devices`: one line per input device, `id<TAB>name`, the default first and marked.
pub fn list_devices(out: &mut dyn Write, err: &mut dyn Write) -> ExitCode {
    match voltip_audio::list_input_devices() {
        Ok(devices) => {
            for d in devices {
                let _ = writeln!(out, "{}\t{}{}", d.id, d.name, if d.is_default { "\t(default)" } else { "" });
            }
            ExitCode::Ok
        }
        Err(e) => {
            let _ = writeln!(err, "voltip: audio: {e}");
            ExitCode::Failed
        }
    }
}

/// `--list-compute`: the CPU's threads, then every GPU this build can run the models on.
pub fn list_compute(out: &mut dyn Write) -> ExitCode {
    let machine = voltip_asr_local::hardware();
    let _ = writeln!(out, "cpu\t{}", machine.cpu_threads);
    for g in machine.gpus {
        let _ = writeln!(out, "gpu\t{}\t{}\t{}\t{}", g.name, g.description, g.kind, g.memory_total / (1024 * 1024));
    }
    ExitCode::Ok
}

/// The compute choice of `--transcribe-file`: each flag given wins over the settings' value.
pub fn compute_for(settings: &voltip_core::EngineSettings, device: Option<LocalDevice>, gpu: Option<&str>, threads: Option<u16>) -> Compute {
    Compute {
        device: device.unwrap_or(settings.local_device),
        gpu: gpu.map(str::to_owned).or_else(|| settings.local_gpu.clone()),
        threads: threads.or(settings.local_threads).map(usize::from),
    }
}

/// Run a headless action and return its exit code. `data_dir` is the app data dir (`settings.json`
/// and the model library under `CoreConfig::models_root`).
pub fn run_headless(action: &Action, data_dir: &Path, out: &mut dyn Write, err: &mut dyn Write) -> ExitCode {
    let models_root = CoreConfig::new(data_dir.to_path_buf()).models_root;
    match action {
        Action::ListDevices => list_devices(out, err),
        Action::ListCompute => list_compute(out),
        Action::ListModels => list_models(&models_root, out),
        Action::DownloadModel { id } => download_model(&ModelStore::new(models_root), id, out, err),
        Action::TranscribeFile { path, model, json, device, gpu, threads } => {
            let settings = SettingsStore::new(data_dir).load().unwrap_or_default();
            let model = model.clone().unwrap_or_else(|| default_model(data_dir));
            let compute = compute_for(&settings.engines, *device, gpu.as_deref(), *threads);
            let transcriber = LocalTranscriber::new(models_root).select(&model).with_compute(compute);
            transcribe_file(&transcriber, path, *json, settings.engines.language.as_deref(), settings.engines.chinese_script, out, err)
        }
        Action::Gui { .. } => ExitCode::Ok,
    }
}
