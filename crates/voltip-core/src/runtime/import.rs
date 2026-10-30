//! Importing audio and video files (docs/dictation.md §31), the runtime's side: the queue in
//! memory, one job at a time on a task of its own ([`crate::import::run`]), the clients it runs
//! with ([`Runtime::import_clients`]: the chosen models only, never the built-in service, no
//! fallback chain), the entry it makes, and the files it decodes to under `<data_dir>/imports`,
//! emptied at every start and stop. A dictation always goes first: the jobs see it through
//! [`ImportRuntime::set_busy`] and wait at their next segment.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::{mpsc, watch};
use uuid::Uuid;

use super::{CoreEvent, Runtime};
use crate::CoreError;
use crate::audio_cache::RecognitionChoice;
use crate::dictation::TakeClients;
use crate::engines::{FallbackSettings, ProviderId, ResolvedEngines, ServiceKind};
use crate::import::run::{self, Outcome, Partial, Progress, Report};
use crate::import::{ImportError, ImportFailure, ImportJob, ImportRequest, ImportState, ImportsView, MediaDecoder, file_name};
use crate::models::CancelToken;
use crate::vocabulary::Vocabulary;

/// Progress reports a job may have queued before the core reads them; more are dropped (the
/// next state report carries the whole state again), the end never is.
const IMPORT_QUEUE: usize = 256;

/// What reaches the core from a job's task.
pub(super) enum ImportInbound {
    /// The job got this far.
    Progress {
        /// The job.
        id: Uuid,
        /// Where it is.
        progress: Progress,
    },
    /// The job's task ended.
    Finished {
        /// The job.
        id: Uuid,
        /// How.
        outcome: Outcome,
    },
}

/// One file in the queue.
struct QueuedJob {
    /// What the interface shows.
    view: ImportJob,
    /// The file and the models.
    request: ImportRequest,
    /// What an earlier try left for a retry.
    partial: Partial,
}

/// The job whose task runs.
struct RunningJob {
    id: Uuid,
    cancel: CancelToken,
    task: tokio::task::JoinHandle<()>,
}

/// The import part of the runtime.
pub(super) struct ImportRuntime {
    /// The shell's decoder; `None`: this app imports nothing (the phone).
    decoder: Option<Arc<dyn MediaDecoder>>,
    /// `<data_dir>/imports`.
    dir: PathBuf,
    jobs: Vec<QueuedJob>,
    /// At most one; it stays until its task ends, also when its job was cancelled.
    running: Option<RunningJob>,
    tx: mpsc::Sender<ImportInbound>,
    /// A dictation runs: the jobs wait at their next segment.
    busy: watch::Sender<bool>,
}

impl ImportRuntime {
    /// The queue of a core whose data lives in `data_dir`, decoding with `decoder`; the jobs report
    /// on the returned receiver. What an earlier run left in `<data_dir>/imports` goes: the queue
    /// lives in memory, so nothing could use it.
    pub(super) fn new(decoder: Option<Arc<dyn MediaDecoder>>, data_dir: &Path) -> (Self, mpsc::Receiver<ImportInbound>) {
        let (tx, rx) = mpsc::channel(IMPORT_QUEUE);
        let dir = crate::import::imports_dir(data_dir);
        remove_all(&dir);
        (Self { decoder, dir, jobs: Vec::new(), running: None, tx, busy: watch::Sender::new(false) }, rx)
    }

    /// `UiState.imports`.
    pub(super) fn view(&self) -> ImportsView {
        ImportsView {
            available: self.decoder.is_some(),
            extensions: self.decoder.as_ref().map(|d| d.extensions().iter().map(|e| (*e).to_owned()).collect()).unwrap_or_default(),
            jobs: self.jobs.iter().map(|j| j.view.clone()).collect(),
        }
    }

    /// A dictation started or ended (any phase but idle counts).
    pub(super) fn set_busy(&self, busy: bool) {
        self.busy.send_if_modified(|running| std::mem::replace(running, busy) != busy);
    }

    /// The core stops: the running job ends and the decoded files go (the original files stay).
    pub(super) fn stop(&mut self) {
        if let Some(running) = self.running.take() {
            running.cancel.cancel();
            running.task.abort();
        }
        self.jobs.clear();
        remove_all(&self.dir);
    }

    /// Where job `id`'s file is decoded to.
    fn pcm(&self, id: Uuid) -> PathBuf {
        self.dir.join(format!("{id}.pcm"))
    }

    fn job_mut(&mut self, id: Uuid) -> Option<&mut QueuedJob> {
        self.jobs.iter_mut().find(|j| j.view.id == id)
    }

    /// Remove job `id`'s decoded file (and a part of it), unless its task still runs: then it goes
    /// when the task ends ([`Runtime::on_import`]).
    fn forget_pcm(&self, id: Uuid) {
        if self.running.as_ref().is_none_or(|r| r.id != id) {
            remove_pcm(&self.pcm(id));
        }
    }
}

impl Runtime {
    /// Tell the interface the queue.
    pub(super) fn emit_imports(&self) {
        self.emit(CoreEvent::Imports(self.imports.view()));
    }

    /// `ImportAdd`: every file whose models may run joins the queue, in order. The requests are
    /// checked first, all of them: one refused, none is queued. A path that is no readable file is
    /// left out and named in the error; the others are queued all the same.
    pub(super) fn import_add(&mut self, requests: Vec<ImportRequest>) -> Result<(), CoreError> {
        if self.imports.decoder.is_none() {
            return Err(ImportError::Unavailable.into());
        }
        if !self.settings.history.enabled {
            return Err(ImportError::HistoryOff.into());
        }
        for request in &requests {
            self.import_engines(request)?;
        }
        let mut unreadable = Vec::new();
        for mut request in requests {
            request.language = request.language.as_deref().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned);
            match std::fs::metadata(&request.path) {
                Ok(meta) if meta.is_file() => {
                    let view = ImportJob {
                        id: Uuid::new_v4(),
                        name: file_name(&request.path),
                        size: meta.len(),
                        duration_ms: None,
                        paused: false,
                        state: ImportState::Queued,
                    };
                    tracing::info!(id = %view.id, bytes = view.size, "a file joins the import queue");
                    self.imports.jobs.push(QueuedJob { view, request, partial: Partial::default() });
                }
                _ => unreadable.push(file_name(&request.path)),
            }
        }
        self.emit_imports();
        self.start_import();
        match unreadable.is_empty() {
            true => Ok(()),
            false => Err(ImportError::NotAFile(unreadable.join("、")).into()),
        }
    }

    /// `ImportCancel` and `ImportRemove`: the job leaves the queue at once; a running one is
    /// stopped, and writes nothing. Its decoded file goes (a running job's when its task ends).
    pub(super) fn import_drop(&mut self, id: Uuid) -> Result<(), CoreError> {
        let Some(index) = self.imports.jobs.iter().position(|j| j.view.id == id) else { return Ok(()) };
        self.imports.jobs.remove(index);
        match &self.imports.running {
            Some(running) if running.id == id => {
                tracing::info!(%id, "the running import is cancelled");
                running.cancel.cancel();
            }
            _ => self.imports.forget_pcm(id),
        }
        self.emit_imports();
        Ok(())
    }

    /// `ImportRetry`: a failed job waits again, keeping its decoded file and the segments it
    /// recognised when it can go on from them, otherwise from the start.
    pub(super) fn import_retry(&mut self, id: Uuid) -> Result<(), CoreError> {
        let Some(job) = self.imports.job_mut(id) else { return Ok(()) };
        let ImportState::Failed { resumable, .. } = job.view.state else { return Ok(()) };
        if !resumable {
            job.partial = Partial::default();
        }
        job.view.state = ImportState::Queued;
        job.view.paused = false;
        if !resumable {
            self.imports.forget_pcm(id);
        }
        self.emit_imports();
        self.start_import();
        Ok(())
    }

    /// A job reported: its state, or its end, after which the next one starts.
    pub(super) fn on_import(&mut self, inbound: ImportInbound) {
        match inbound {
            ImportInbound::Progress { id, progress } => {
                // A job cancelled meanwhile reports nothing more.
                let Some(job) = self.imports.job_mut(id) else { return };
                match progress {
                    Progress::State(state) => {
                        job.view.state = state;
                        job.view.paused = false;
                    }
                    Progress::Paused(paused) => job.view.paused = paused,
                    Progress::Duration(ms) => job.view.duration_ms = Some(ms),
                }
                self.emit_imports();
            }
            ImportInbound::Finished { id, outcome } => {
                if self.imports.running.as_ref().is_some_and(|r| r.id == id) {
                    self.imports.running = None;
                }
                let pcm = self.imports.pcm(id);
                if self.imports.job_mut(id).is_none() {
                    // Cancelled or removed while it ran.
                    remove_pcm(&pcm);
                } else {
                    let (state, partial) = self.finish_import(id, outcome, &pcm);
                    if let Some(job) = self.imports.job_mut(id) {
                        job.partial = partial;
                        job.view.paused = false;
                        match state {
                            Some(state) => job.view.state = state,
                            None => self.imports.jobs.retain(|j| j.view.id != id),
                        }
                    }
                }
                self.emit_imports();
                self.start_import();
            }
        }
    }

    /// What a job's end makes of it: its state (`None`: it leaves the queue) and what a retry
    /// keeps. A done job's entry goes into the history; a failure that cannot go on drops its
    /// decoded file.
    fn finish_import(&mut self, id: Uuid, outcome: Outcome, pcm: &Path) -> (Option<ImportState>, Partial) {
        match outcome {
            Outcome::Done { entry, polish_failed } => {
                remove_pcm(pcm);
                let entry_id = entry.id;
                if !self.settings.history.enabled {
                    let failure = ImportFailure::Storage { detail: ImportError::HistoryOff.to_string() };
                    return (Some(ImportState::Failed { failure, resumable: false }), Partial::default());
                }
                match self.push_history(*entry) {
                    Ok(()) => {
                        tracing::info!(%id, entry = %entry_id, polish_failed, "an imported file is in the history");
                        (Some(ImportState::Done { entry: entry_id, polish_failed }), Partial::default())
                    }
                    Err(e) => {
                        tracing::warn!(%id, error = %e, "the imported file's entry could not be saved");
                        (Some(ImportState::Failed { failure: ImportFailure::Storage { detail: e.to_string() }, resumable: false }), Partial::default())
                    }
                }
            }
            Outcome::Failed { failure, resumable, partial } => {
                tracing::info!(%id, ?failure, resumable, "an import stopped");
                if resumable {
                    (Some(ImportState::Failed { failure, resumable }), partial)
                } else {
                    remove_pcm(pcm);
                    (Some(ImportState::Failed { failure, resumable }), Partial::default())
                }
            }
            Outcome::Cancelled => {
                remove_pcm(pcm);
                (None, Partial::default())
            }
        }
    }

    /// Start the first waiting job when none runs. A job whose models cannot run now (a key
    /// removed since, the history turned off) fails at once and the next one is tried.
    fn start_import(&mut self) {
        if self.imports.running.is_some() {
            return;
        }
        while let Some(index) = self.imports.jobs.iter().position(|j| j.view.state == ImportState::Queued) {
            let (id, request, partial) = {
                let job = &self.imports.jobs[index];
                (job.view.id, job.request.clone(), job.partial.clone())
            };
            match self.spawn_import(id, &request, partial) {
                Ok(running) => {
                    self.imports.running = Some(running);
                    return;
                }
                Err(failure) => {
                    let job = &mut self.imports.jobs[index];
                    let resumable = job.partial.samples.is_some();
                    job.view.state = ImportState::Failed { failure, resumable };
                    self.emit_imports();
                }
            }
        }
    }

    /// The task of job `id`, going on from `partial`.
    fn spawn_import(&self, id: Uuid, request: &ImportRequest, partial: Partial) -> Result<RunningJob, ImportFailure> {
        if !self.settings.history.enabled {
            return Err(ImportFailure::Storage { detail: ImportError::HistoryOff.to_string() });
        }
        let Some(decoder) = self.imports.decoder.clone() else {
            return Err(ImportFailure::Storage { detail: ImportError::Unavailable.to_string() });
        };
        let clients = self.import_clients(request).map_err(|e| ImportFailure::Recognition { detail: e.to_string() })?;
        std::fs::create_dir_all(&self.imports.dir).map_err(|e| ImportFailure::Storage { detail: e.to_string() })?;
        let cancel = CancelToken::new();
        let job = run::Job {
            path: request.path.clone(),
            name: file_name(&request.path),
            pcm: self.imports.pcm(id),
            decoder,
            segmenter: self.dictation.segmenter(),
            clients,
            language: request.language.clone(),
            script: self.settings.engines.chinese_script,
            vocabulary: Arc::new(Vocabulary::compile(self.dictionary.entries(), self.rules.rules())),
            cancel: cancel.clone(),
            busy: self.imports.busy.subscribe(),
        };
        let (progress_tx, done_tx) = (self.imports.tx.clone(), self.imports.tx.clone());
        let report: Report = Arc::new(move |progress| {
            if progress_tx.try_send(ImportInbound::Progress { id, progress }).is_err() {
                tracing::debug!(%id, "an import's progress was dropped: the core is behind");
            }
        });
        tracing::info!(%id, resumed = partial.samples.is_some(), "an import starts");
        let task = tokio::spawn(async move {
            let outcome = run::run(job, partial, report).await;
            let _ = done_tx.send(ImportInbound::Finished { id, outcome }).await;
        });
        Ok(RunningJob { id, cancel, task })
    }

    /// The clients an import of `request` runs with (docs/dictation.md §31), as a 重新识别 on
    /// other models builds them: the settings with the chosen models put in, checked and resolved,
    /// but neither saved nor applied to anything else. Never a fallback chain: both lists are off
    /// for the import; the clients are metered as usual, each call as an import's. The built-in
    /// service is refused, chosen or resolved. Without a polish the clean-up settings are left as
    /// they are (they start on the built-in service, which the import neither uses nor refuses)
    /// and the clients say the import does not clean up.
    pub(super) fn import_clients(&self, request: &ImportRequest) -> Result<TakeClients, ImportError> {
        let (resolved, polish) = self.import_engines(request)?;
        Ok(self.dictation.clients_for(&resolved, Some(polish)))
    }

    /// The checked and resolved configuration of an import of `request`, and whether it polishes.
    fn import_engines(&self, request: &ImportRequest) -> Result<(ResolvedEngines, bool), ImportError> {
        if matches!(request.asr, RecognitionChoice::Remote { provider: ProviderId::Builtin, .. })
            || request.refine.as_ref().is_some_and(|r| r.provider == ProviderId::Builtin)
        {
            return Err(ImportError::BuiltinNotAllowed);
        }
        let mut engines = self.settings.engines.clone();
        match &request.asr {
            RecognitionChoice::Remote { provider, model } => {
                engines.asr_provider = *provider;
                engines.providers.entry(*provider).or_default().asr_model = Some(model.clone());
            }
            RecognitionChoice::Local { id } => {
                engines.asr_provider = ProviderId::Local;
                engines.local_model = Some(id.clone());
            }
        }
        if let Some(polish) = &request.refine {
            engines.refine_enabled = true;
            engines.llm_provider = polish.provider;
            engines.providers.entry(polish.provider).or_default().llm_model = Some(polish.model.clone());
        }
        engines.language = request.language.clone();
        engines.asr_fallback = FallbackSettings::default();
        engines.llm_fallback = FallbackSettings::default();
        let engine_error = |e: CoreError| ImportError::Engines(e.to_string());
        self.check_engines(&engines).map_err(engine_error)?;
        let resolved = ResolvedEngines::resolve_with_models(&engines, &self.user_secrets, &self.built_in, &self.library);
        Self::check_recognition(&resolved).map_err(engine_error)?;
        let polish = request.refine.is_some();
        refuse_builtin(&resolved, polish)?;
        if polish && let Some(issue) = resolved.refine_issue {
            return Err(ImportError::Engines(issue.message(ServiceKind::Llm).to_owned()));
        }
        Ok((resolved, polish))
    }
}

/// The resolved models of an import may not be the built-in service's: its recognition, or its
/// clean-up when the import polishes.
fn refuse_builtin(resolved: &ResolvedEngines, polish: bool) -> Result<(), ImportError> {
    if resolved.asr_provider == ProviderId::Builtin || (polish && resolved.llm_provider == Some(ProviderId::Builtin)) {
        return Err(ImportError::BuiltinNotAllowed);
    }
    Ok(())
}

/// Remove a decoded file and what may be left of its writing.
fn remove_pcm(pcm: &Path) {
    for path in [pcm.to_path_buf(), pcm.with_extension("pcm.part")] {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!(path = %path.display(), error = %e, "a decoded file could not be removed"),
        }
    }
}

/// Remove `<data_dir>/imports` and everything in it.
fn remove_all(dir: &Path) {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!(dir = %dir.display(), error = %e, "the decoded files could not be removed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_cache::RecognitionChoice;
    use crate::dictation::fakes::{FakeMediaDecoder, FakeRefiner, FakeTranscriber, FixedSegmenterFactory, factory_by_model, ports};
    use crate::engines::{BuiltIn, EngineSettings, ProviderSettings};
    use crate::import::{ImportState, PolishModel};
    use crate::settings::{Settings, SettingsStore};
    use crate::{AppCore, CoreConfig, CoreEvent};
    use voltip_identity::SecretStore;

    /// The built-in service of these tests: recognition and clean-up.
    const BUILT_IN: BuiltIn = BuiltIn {
        asr_url: Some("https://<asr-host>"),
        asr_token: Some("t"),
        asr_model: "builtin/asr",
        refine_url: Some("https://<refine-host>/v1"),
        refine_api_key: Some("k"),
        refine_model: "builtin/llm",
        ..BuiltIn::EMPTY
    };

    /// Recognition on Groq's Whisper with the user's key; the clean-up `llm` (the built-in one by
    /// default, as the settings start).
    fn engines(llm: ProviderId) -> EngineSettings {
        EngineSettings {
            asr_provider: ProviderId::Groq,
            llm_provider: llm,
            refine_enabled: true,
            providers: [(
                ProviderId::Groq,
                ProviderSettings { asr_model: Some("whisper-large-v3".into()), llm_model: Some("llama-3.3-70b".into()), ..Default::default() },
            )]
            .into(),
            ..EngineSettings::default()
        }
    }

    struct Fakes {
        groq_asr: Arc<FakeTranscriber>,
        builtin_asr: Arc<FakeTranscriber>,
        groq_llm: Arc<FakeRefiner>,
        builtin_llm: Arc<FakeRefiner>,
    }

    fn fakes() -> Fakes {
        Fakes {
            groq_asr: Arc::new(FakeTranscriber::ok("导入的文字。").with_model("whisper-large-v3")),
            builtin_asr: Arc::new(FakeTranscriber::ok("内置识别。")),
            groq_llm: Arc::new(FakeRefiner::ok("润色后的文字。")),
            builtin_llm: Arc::new(FakeRefiner::ok("内置润色。")),
        }
    }

    fn config(dir: &Path, built_in: BuiltIn) -> CoreConfig {
        let mut config = CoreConfig::new(dir.to_path_buf());
        config.test_hooks.built_in = Some(built_in);
        config.media_decoder = Some(Arc::new(FakeMediaDecoder::new(25)));
        config
    }

    fn core_ports(fakes: &Fakes) -> crate::dictation::DictationPorts {
        let mut ports = ports();
        ports.factory = factory_by_model(
            &[("whisper-large-v3", fakes.groq_asr.clone()), ("builtin/asr", fakes.builtin_asr.clone())],
            &[("llama-3.3-70b", fakes.groq_llm.clone()), ("builtin/llm", fakes.builtin_llm.clone())],
        );
        ports.segmenter = Some(Arc::new(FixedSegmenterFactory(10)));
        ports
    }

    fn store() -> Arc<dyn SecretStore> {
        let store = voltip_identity::MemorySecretStore::new();
        store.set("provider-key.groq", b"gsk-test").unwrap();
        Arc::new(store)
    }

    /// A runtime on `dir` with `engines` saved, built but not running.
    fn runtime(dir: &Path, engines: EngineSettings, built_in: BuiltIn, fakes: &Fakes) -> Runtime {
        SettingsStore::new(dir).save(&Settings { relay_enabled: false, engines, ..Settings::default() }).unwrap();
        AppCore::build(config(dir, built_in), store(), core_ports(fakes)).unwrap().0
    }

    fn request(asr: RecognitionChoice, refine: Option<PolishModel>) -> ImportRequest {
        ImportRequest { path: PathBuf::from("/media/会议录音.mp4"), asr, refine, language: None }
    }

    fn remote(provider: ProviderId, model: &str) -> RecognitionChoice {
        RecognitionChoice::Remote { provider, model: model.into() }
    }

    fn refused(result: Result<TakeClients, ImportError>) -> ImportError {
        match result {
            Err(e) => e,
            Ok(clients) => panic!("not refused: {clients:?}"),
        }
    }

    /// docs/dictation.md §31: imports never use the built-in service. Chosen as the recognition or
    /// the polish it is refused, on a build that has it and on one without it (which would resolve
    /// it to the local model); and models that resolve to it are refused after the resolution.
    #[tokio::test]
    async fn regression_builtin_is_refused_for_imports() {
        let fakes = fakes();
        for built_in in [BUILT_IN, BuiltIn::EMPTY] {
            let dir = tempfile::tempdir().unwrap();
            let rt = runtime(dir.path(), engines(ProviderId::Groq), built_in, &fakes);
            let builtin_asr = request(remote(ProviderId::Builtin, ""), None);
            assert_eq!(refused(rt.import_clients(&builtin_asr)), ImportError::BuiltinNotAllowed, "{built_in:?}");
            let builtin_named = request(remote(ProviderId::Builtin, "builtin/asr"), None);
            assert_eq!(refused(rt.import_clients(&builtin_named)), ImportError::BuiltinNotAllowed);
            let builtin_polish =
                request(remote(ProviderId::Groq, "whisper-large-v3"), Some(PolishModel { provider: ProviderId::Builtin, model: "builtin/llm".into() }));
            assert_eq!(refused(rt.import_clients(&builtin_polish)), ImportError::BuiltinNotAllowed);
            // The same files with the user's own services are taken.
            let own = request(remote(ProviderId::Groq, "whisper-large-v3"), Some(PolishModel { provider: ProviderId::Groq, model: "llama-3.3-70b".into() }));
            assert_eq!(rt.import_clients(&own).unwrap().refine, Some(true));
        }

        // After the resolution: recognition or (when it polishes) a clean-up on the built-in
        // service is refused whatever was chosen.
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(dir.path(), engines(ProviderId::Groq), BUILT_IN, &fakes);
        let resolved = rt.resolved_engines();
        assert_eq!(refuse_builtin(&resolved, true), Ok(()));
        let mut on_builtin = resolved.clone();
        on_builtin.asr_provider = ProviderId::Builtin;
        assert_eq!(refuse_builtin(&on_builtin, false), Err(ImportError::BuiltinNotAllowed));
        let mut polish_on_builtin = resolved;
        polish_on_builtin.llm_provider = Some(ProviderId::Builtin);
        assert_eq!(refuse_builtin(&polish_on_builtin, true), Err(ImportError::BuiltinNotAllowed));
        assert_eq!(refuse_builtin(&polish_on_builtin, false), Ok(()), "without the polish its service is not used");
    }

    /// The core in the regression below, running, and the newest import's state.
    async fn imported(events: &mut mpsc::Receiver<CoreEvent>) -> ImportState {
        loop {
            let ev = tokio::time::timeout(std::time::Duration::from_secs(10), events.recv()).await.expect("event within 10 s").expect("core alive");
            if let CoreEvent::Imports(view) = &ev
                && let Some(job) = view.jobs.last()
                && matches!(job.state, ImportState::Done { .. } | ImportState::Failed { .. })
            {
                return job.state.clone();
            }
        }
    }

    /// docs/dictation.md §31: an import that does not polish leaves the clean-up settings alone —
    /// they start on the built-in service, which it may neither use nor refuse — and its clients
    /// say it does not clean up, whatever the settings or a scene say.
    #[tokio::test]
    async fn regression_an_import_without_polish_ignores_the_saved_builtin_llm() {
        let fakes = fakes();
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(dir.path(), engines(ProviderId::Builtin), BUILT_IN, &fakes);
        let without = request(remote(ProviderId::Groq, "whisper-large-v3"), None);
        let clients = rt.import_clients(&without).unwrap();
        assert_eq!(clients.refine, Some(false));
        assert_eq!(clients.asr_model, "whisper-large-v3");
        drop(rt);

        let dir = tempfile::tempdir().unwrap();
        SettingsStore::new(dir.path()).save(&Settings { relay_enabled: false, engines: engines(ProviderId::Builtin), ..Settings::default() }).unwrap();
        let file = dir.path().join("会议录音.mp4");
        std::fs::write(&file, b"not really a video").unwrap();
        let (handle, mut events) = AppCore::start_with(config(dir.path(), BUILT_IN), store(), core_ports(&fakes)).unwrap();
        handle.send(crate::CoreCommand::ImportAdd(vec![ImportRequest { path: file, ..without }])).await.unwrap();
        let state = imported(&mut events).await;
        assert!(matches!(state, ImportState::Done { polish_failed: 0, .. }), "{state:?}");
        assert_eq!((fakes.groq_asr.calls(), fakes.builtin_asr.calls()), (3, 0));
        assert_eq!((fakes.groq_llm.calls(), fakes.builtin_llm.calls()), (0, 0), "nothing was polished");
        handle.send(crate::CoreCommand::Shutdown).await.unwrap();
    }

    /// The local model chosen for an import must be installed; another provider than the one the
    /// settings recognise with needs its key.
    #[tokio::test]
    async fn an_import_needs_its_model_installed_and_its_key() {
        let fakes = fakes();
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(dir.path(), engines(ProviderId::Groq), BUILT_IN, &fakes);
        let local = request(RecognitionChoice::Local { id: "paraformer-zh".into() }, None);
        assert!(matches!(refused(rt.import_clients(&local)), ImportError::Engines(why) if why.contains("本地模型")), "no models on this core");
        let keyless = request(remote(ProviderId::Openai, "whisper-1"), None);
        assert!(matches!(refused(rt.import_clients(&keyless)), ImportError::Engines(_)));
        assert!(rt.import_clients(&request(remote(ProviderId::Groq, "whisper-large-v3"), None)).is_ok());
    }
}
