//! [`LocalTranscriber`]: the dictation pipeline's `Transcriber` over a local recogniser — a
//! transcribe.cpp GGUF session (`gguf.rs`) or a sherpa-onnx recogniser (`sherpa.rs`), picked by
//! the catalogue entry's engine ([`DefaultLoader`]).
//!
//! The recogniser is loaded ahead of the first take ([`LocalTranscriber::warm_up`], the port's
//! `warm`: at start-up and after every configuration change, docs/dictation.md §10.7), or at the
//! latest by the take, and kept in a small pool shared between the clones the engine factory hands
//! out (`pool.rs`), rebuilt only when the selected model, the language hint or the compute choice
//! changes. Loading and inference run on a blocking thread. A model that is not installed fails
//! before any file is touched with the documented「本地模型未下载」text. Takes shorter than
//! [`MIN_INPUT`] are zero-padded to it first: SenseVoice / Paraformer misbehave on very short input
//! (decided 2026-09-25); the 300 ms / silence → `NoSpeech` rule runs before this in the core. With
//! `vad_trim` on (docs/dictation.md §12) the take is first cut to its speech by the
//! [`VadTrimmer`] — fail-open: without the VAD model, or when it fails, the take goes in whole.
//!
//! When a recognition goes wrong (docs/dictation.md §29), the recogniser says how
//! ([`RecognizeError`], a [`LocalFailure`]): the instance that failed is not used again, a model
//! failure takes every instance of the model with it, so the next take loads afresh; only a run
//! the take's own cancel stopped gives its instance back. A device or memory failure on a GPU is
//! tried once more on the CPU, and a second one within ten minutes keeps the local models on the
//! CPU until Voltip restarts; the transcript says so ([`AsrNote`]). An input longer than the
//! model's `max_piece` is cut where it pauses ([`crate::segmenter::pieces`]), recognised piece by
//! piece on one instance and put together like a long take's segments. A take that is cancelled
//! sets its [`Abort`] and starts no further piece; the piece in progress runs to its end
//! (transcribe.cpp 0.4.1 looks at a cancel only before a Qwen3-ASR run, sherpa-onnx not at all), on
//! the one transcribe.cpp instance or, for sherpa-onnx, on an instance of its own while the next
//! take uses a second one, or loads the model or language it needs beside it.

use std::io::Cursor;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::Mutex;
pub use voltip_core::dictation::LocalFailure;
use voltip_core::dictation::{AsrNote, DictationError, Transcriber, Transcript};

use crate::catalogue::{CATALOGUE, Engine, ModelEntry};
use crate::compute::{Compute, LocalDevice};
use crate::gguf::GgufLoader;
use crate::pool::{GpuHealth, Grant, Key, LOCK_WAIT, Lease, Pool, cap};
use crate::root::ModelsRoot;
use crate::sherpa::SherpaLoader;
use crate::store;
use crate::vad::VadTrimmer;

/// Shortest input handed to a local recogniser; shorter takes are zero-padded to this length.
pub const MIN_INPUT: Duration = Duration::from_millis(1250);

/// What the take hears when every recogniser stayed busy for its lock wait (docs/dictation.md §29).
pub const BUSY: &str = "上一次识别还没结束";

/// Why a recogniser produced no text: what kind of failure, and its own words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecognizeError {
    /// What went wrong, as the transcriber and the fallback (docs/dictation.md §29) act on it.
    pub kind: LocalFailure,
    /// The engine's message.
    pub message: String,
    /// The take's own cancel stopped the run ([`RecognizeError::cancelled`]): no fault of the
    /// recogniser, which goes back to the pool. Any other failure drops it, a decoder stopped by
    /// its output budget or a repetition too, even while the take is being cancelled.
    pub by_cancel: bool,
}

impl RecognizeError {
    /// A failure of `kind`.
    pub fn new(kind: LocalFailure, message: impl Into<String>) -> Self {
        Self { kind, message: message.into(), by_cancel: false }
    }

    /// A failure the engine does not classify.
    pub fn other(message: impl Into<String>) -> Self {
        Self::new(LocalFailure::Other, message)
    }

    /// The run did not start, or stopped, because the take was cancelled ([`LocalFailure::Aborted`]).
    pub fn cancelled(message: impl Into<String>) -> Self {
        Self { by_cancel: true, ..Self::new(LocalFailure::Aborted, message) }
    }
}

impl std::fmt::Display for RecognizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<String> for RecognizeError {
    fn from(message: String) -> Self {
        Self::other(message)
    }
}

/// A take's request to stop its recognition (docs/dictation.md §29), set when the take is
/// cancelled. A recogniser that can stop between decode steps listens with [`Abort::on_abort`].
#[derive(Clone, Default)]
pub struct Abort(Arc<AbortState>);

#[derive(Default)]
struct AbortState {
    set: AtomicBool,
    hooks: Mutex<Vec<Box<dyn Fn() + Send>>>,
}

impl Abort {
    /// Not set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask the recognition to stop; every hook runs once.
    pub fn abort(&self) {
        let hooks = self.0.hooks.lock();
        if !self.0.set.swap(true, Ordering::SeqCst) {
            for hook in hooks.iter() {
                hook();
            }
        }
    }

    /// Whether the recognition was asked to stop.
    pub fn is_aborted(&self) -> bool {
        self.0.set.load(Ordering::SeqCst)
    }

    /// Run `hook` when the recognition is asked to stop (at once when it was already).
    pub fn on_abort(&self, hook: impl Fn() + Send + 'static) {
        let mut hooks = self.0.hooks.lock();
        if self.is_aborted() {
            hook();
        } else {
            hooks.push(Box::new(hook));
        }
    }
}

impl std::fmt::Debug for Abort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Abort").field(&self.is_aborted()).finish()
    }
}

/// Sets the take's [`Abort`] when the take's future is dropped before its recognition returned.
struct AbortOnDrop(Option<Abort>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some(abort) = self.0.take() {
            abort.abort();
        }
    }
}

/// A loaded recogniser: one model, one language setting.
pub trait Recognizer: Send {
    /// Recognise mono PCM samples in `-1.0..=1.0` at `sample_rate` Hz (resampled inside when
    /// not 16 kHz). Returns the text, untrimmed. `abort` is set when the take is cancelled: a
    /// recogniser that can stop between decode steps stops ([`LocalFailure::Aborted`]); one that
    /// cannot runs to its end. Neither engine starts a run that was cancelled before it began.
    fn transcribe(&mut self, sample_rate: u32, samples: &[f32], abort: &Abort) -> Result<String, RecognizeError>;

    /// The compute backend the recogniser actually runs on (`CPU`, `Metal`, `Vulkan0`, …).
    fn backend(&self) -> String {
        "CPU".to_owned()
    }
}

/// Builds a [`Recognizer`] from an installed model directory. The real one wraps sherpa-onnx;
/// tests plug in a fake so no model file is needed.
pub trait RecognizerLoader: Send + Sync {
    /// Load `entry` from `dir` with `language` (`None` = auto) where `compute` says
    /// (docs/dictation.md §10.6).
    fn load(&self, entry: &ModelEntry, dir: &Path, language: Option<&str>, compute: &Compute) -> Result<Box<dyn Recognizer>, String>;
}

/// The production loader: dispatches on the entry's engine — transcribe.cpp for the GGUF tiers,
/// sherpa-onnx for SenseVoice / Paraformer. The streaming model is not a whole-take recogniser.
#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultLoader;

impl RecognizerLoader for DefaultLoader {
    fn load(&self, entry: &ModelEntry, dir: &Path, language: Option<&str>, compute: &Compute) -> Result<Box<dyn Recognizer>, String> {
        match entry.engine {
            Engine::TranscribeCpp => GgufLoader.load(entry, dir, language, compute),
            Engine::SenseVoice | Engine::Paraformer => SherpaLoader.load(entry, dir, language, compute),
            Engine::ZipformerStreaming => Err(format!("{}: 实时预览模型不能识别整段录音", entry.id)),
            Engine::SileroVad => Err(format!("{}: 语音活动检测模型不能做整段识别", entry.id)),
        }
    }
}

/// What the clones of one transcriber share: the recognisers in memory, the GPU's record and the
/// warm-up tickets.
#[derive(Default)]
struct Shared {
    pool: Arc<Pool>,
    gpu: Mutex<GpuHealth>,
    /// Warm-ups asked for so far: one that finds a newer ticket when it may load gives way to it.
    warm_ups: AtomicU64,
}

/// Offline speech-to-text over the model library.
#[derive(Clone)]
pub struct LocalTranscriber {
    root: ModelsRoot,
    catalogue: &'static [ModelEntry],
    loader: Arc<dyn RecognizerLoader>,
    shared: Arc<Shared>,
    selected: String,
    /// Where the recogniser runs (docs/dictation.md §10.6); a change reloads it.
    compute: Compute,
    /// The VAD over the same library (docs/dictation.md §12); shared between the clones.
    vad: VadTrimmer,
    /// `EngineSettings.vad_trim`: cut the take to its speech before recognition.
    vad_trim: bool,
    /// How long a take waits for a busy recogniser ([`LOCK_WAIT`]).
    lock_wait: Duration,
}

impl std::fmt::Debug for LocalTranscriber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalTranscriber")
            .field("root", &self.root)
            .field("selected", &self.selected)
            .field("compute", &self.compute)
            .field("vad_trim", &self.vad_trim)
            .field("loaded", &self.shared.pool.model())
            .finish()
    }
}

/// Inference threads: the machine's parallelism, capped (more does not help these models).
pub fn default_threads() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 4)
}

impl LocalTranscriber {
    /// The real engines ([`DefaultLoader`]) over the real catalogue under `root`
    /// (`<app data dir>/models`); the default model is selected until [`LocalTranscriber::select`].
    pub fn new(root: impl Into<ModelsRoot>) -> Self {
        Self::with_loader(root, CATALOGUE, Arc::new(DefaultLoader))
    }

    /// Any catalogue and loader (tests); the VAD is the real one over the same root until
    /// [`LocalTranscriber::with_vad`].
    pub fn with_loader(root: impl Into<ModelsRoot>, catalogue: &'static [ModelEntry], loader: Arc<dyn RecognizerLoader>) -> Self {
        let root = root.into();
        Self {
            vad: VadTrimmer::new(root.clone()),
            root,
            catalogue,
            loader,
            shared: Arc::new(Shared::default()),
            selected: crate::catalogue::DEFAULT_MODEL_ID.to_owned(),
            compute: Compute::default(),
            vad_trim: false,
            lock_wait: LOCK_WAIT,
        }
    }

    /// A handle on the same recognisers that transcribes with `id` (the engine factory calls this
    /// on every settings change; the model is only reloaded when `id` differs).
    pub fn select(&self, id: &str) -> Self {
        Self { selected: id.to_owned(), ..self.clone() }
    }

    /// Inference threads (default: each engine's own, [`default_threads`] for sherpa-onnx).
    pub fn with_threads(self, threads: usize) -> Self {
        let compute = Compute { threads: Some(threads.max(1)), ..self.compute.clone() };
        Self { compute, ..self }
    }

    /// Where the recogniser runs: the device choice, the GPU and the threads
    /// (`EngineSettings.local_device / local_gpu / local_threads`); a change reloads the model.
    pub fn with_compute(self, compute: Compute) -> Self {
        Self { compute, ..self }
    }

    /// The compute choice the next load uses.
    pub fn compute(&self) -> &Compute {
        &self.compute
    }

    /// How long a take waits for a recogniser every instance of which is busy ([`LOCK_WAIT`]; the
    /// local speech service, which queues its own requests, waits longer).
    pub fn with_lock_wait(self, wait: Duration) -> Self {
        Self { lock_wait: wait, ..self }
    }

    /// Cut every take to its speech before recognition (docs/dictation.md §12 `vad_trim`); needs
    /// the `silero-vad` entry installed, otherwise the take goes in whole.
    pub fn with_vad_trim(self, on: bool) -> Self {
        Self { vad_trim: on, ..self }
    }

    /// Any VAD trimmer (tests plug in a fake detector).
    pub fn with_vad(self, vad: VadTrimmer) -> Self {
        Self { vad, ..self }
    }

    /// Whether takes are trimmed before recognition.
    pub fn vad_trim(&self) -> bool {
        self.vad_trim
    }

    /// The VAD trimmer over this library.
    pub fn vad(&self) -> &VadTrimmer {
        &self.vad
    }

    /// The selected catalogue id.
    pub fn selected(&self) -> &str {
        &self.selected
    }

    /// The library root.
    pub fn root(&self) -> PathBuf {
        self.root.get()
    }

    /// Id of the recogniser currently in memory, if any.
    pub fn loaded(&self) -> Option<String> {
        self.shared.pool.model()
    }

    /// The backend the recogniser in memory runs on (what `auto` / `gpu` turned into), while no
    /// take holds it.
    pub fn loaded_backend(&self) -> Option<String> {
        self.shared.pool.idle_backend()
    }

    /// Whether the GPU is off until Voltip restarts (two GPU failures within ten minutes).
    pub fn gpu_off(&self) -> bool {
        self.shared.gpu.lock().is_off()
    }

    /// Drop the recognisers (the memory) until the next transcription.
    pub fn unload(&self) {
        self.shared.pool.clear();
    }

    /// Load the selected model for `language` where [`Self::compute`] says, now, on a background
    /// thread (docs/dictation.md §10.7), so the next take does not wait for it. Nothing happens
    /// when the model is not installed or is already in memory for this language and compute. The
    /// thread waits while a take holds a recogniser of another model; a newer warm-up supersedes
    /// it meanwhile. A failure only logs: the next take loads again and reports it. Returns the
    /// thread when one was started.
    pub fn warm_up(&self, language: Option<&str>) -> Option<std::thread::JoinHandle<()>> {
        let entry = self.entry().ok()?;
        let dir = self.root.join(entry.id);
        if !store::is_installed(&dir, entry) {
            return None;
        }
        let language = language_for(entry.engine, language).map(str::to_owned);
        let compute = self.shared.gpu.lock().compute(&self.compute);
        let key = Key { id: entry.id.to_owned(), language, compute };
        if self.shared.pool.has_idle(&key) {
            return None;
        }
        let ticket = self.shared.warm_ups.fetch_add(1, Ordering::SeqCst) + 1;
        let me = self.clone();
        std::thread::Builder::new()
            .name("voltip-asr-warm".into())
            .spawn(move || {
                let superseded = || me.shared.warm_ups.load(Ordering::SeqCst) != ticket;
                if !me.shared.pool.reserve_load(&key, superseded) {
                    tracing::debug!(model = entry.id, "local model warm-up not needed or superseded");
                    return;
                }
                match load(me.loader.as_ref(), entry, &dir, &key) {
                    Ok(recognizer) => me.shared.pool.stock(key, recognizer),
                    Err(e) => {
                        me.shared.pool.load_failed(&key);
                        tracing::warn!(model = entry.id, error = %e, "local model warm-up failed; the next take loads it again");
                    }
                }
            })
            .map_err(|e| tracing::warn!(error = %e, "could not start the local model warm-up"))
            .ok()
    }

    fn entry(&self) -> Result<&'static ModelEntry, DictationError> {
        self.catalogue.iter().find(|e| e.id == self.selected).ok_or_else(|| DictationError::Asr(format!("本地模型不在目录中：{}", self.selected)))
    }
}

/// Load `entry` from `dir` for `key`, logging what it took.
fn load(loader: &dyn RecognizerLoader, entry: &ModelEntry, dir: &Path, key: &Key) -> Result<Box<dyn Recognizer>, String> {
    let started = Instant::now();
    let recognizer = loader.load(entry, dir, key.language.as_deref(), &key.compute)?;
    tracing::info!(
        model = entry.id,
        language = ?key.language,
        compute = ?key.compute,
        backend = %recognizer.backend(),
        load_ms = started.elapsed().as_millis() as u64,
        "local model loaded"
    );
    Ok(recognizer)
}

/// One take's recognition, on its blocking thread.
struct Job {
    shared: Arc<Shared>,
    loader: Arc<dyn RecognizerLoader>,
    entry: &'static ModelEntry,
    dir: PathBuf,
    language: Option<String>,
    compute: Compute,
    vad: Option<VadTrimmer>,
    /// The library's VAD, for the piece cuts.
    pieces_vad: VadTrimmer,
    lock_wait: Duration,
    abort: Abort,
}

/// An attempt that failed, and whether it ran on a GPU.
struct Failed {
    error: RecognizeError,
    on_gpu: bool,
}

impl Job {
    /// The text and what to tell about it, or the reason there is none.
    fn run(self, mut samples: Vec<f32>, sample_rate: u32) -> Result<(String, Option<AsrNote>), String> {
        if let Some(vad) = &self.vad {
            // Blocking (may load the VAD model): inside the blocking task. Fail-open.
            let trim = vad.trim(&samples, sample_rate);
            match &trim.skipped {
                Some(why) => tracing::debug!(why, "VAD trim skipped; the whole take goes in"),
                None => tracing::debug!(cut_start = trim.cut_start, cut_end = trim.cut_end, "take trimmed before recognition"),
            }
            samples = trim.samples;
        }
        pad_to_min(&mut samples, sample_rate);
        let pieces = crate::segmenter::pieces(&samples, sample_rate, self.entry.max_piece, &self.pieces_vad);
        if pieces.len() > 1 {
            tracing::info!(
                model = self.entry.id,
                pieces = pieces.len(),
                max_piece_s = self.entry.max_piece.as_secs(),
                "input longer than the model takes; cut where it pauses"
            );
        }
        let compute = self.shared.gpu.lock().compute(&self.compute);
        // The GPU is off until restart and this take would have used it.
        let off = (compute != self.compute).then_some(AsrNote::GpuOffUntilRestart);
        match self.attempt(&pieces, &samples, sample_rate, &compute) {
            Ok(text) => Ok((text, off)),
            Err(Failed { error, on_gpu: true }) if error.kind.is_device() => {
                let now_off = self.shared.gpu.lock().failed(Instant::now());
                tracing::warn!(model = self.entry.id, kind = error.kind.as_str(), error = %error, gpu_off = now_off, "the GPU failed; recognising on the CPU");
                let cpu = Compute { device: LocalDevice::Cpu, gpu: None, ..compute };
                match self.attempt(&pieces, &samples, sample_rate, &cpu) {
                    Ok(text) => Ok((text, Some(if now_off { AsrNote::GpuOffUntilRestart } else { AsrNote::GpuFailedRanOnCpu }))),
                    Err(again) => Err(format!("显卡出错（{}），改用 CPU 也失败（{}）", error.message, again.error.message)),
                }
            }
            Err(Failed { error, .. }) => Err(error.message),
        }
    }

    /// Recognise every piece on one instance loaded for `compute`; the pieces' texts put together.
    fn attempt(&self, pieces: &[Range<usize>], samples: &[f32], sample_rate: u32, compute: &Compute) -> Result<String, Failed> {
        let key = Key { id: self.entry.id.to_owned(), language: self.language.clone(), compute: compute.clone() };
        let mut lease = self.lease(&key)?;
        // sherpa-onnx (room for a second instance) cannot stop a decode: once the take is
        // cancelled, its instance holds back no other model or language while it runs on. A
        // transcribe.cpp instance stays the one of its model until its piece ends.
        if let Some(id) = lease.id().filter(|_| cap(self.entry.engine) > 1) {
            let pool = self.shared.pool.clone();
            self.abort.on_abort(move || pool.abandon(id));
        }
        let on_gpu = lease.backend() != "CPU";
        let mut texts = Vec::with_capacity(pieces.len());
        for piece in pieces {
            if self.abort.is_aborted() {
                return Err(Failed { error: RecognizeError::cancelled("cancelled"), on_gpu });
            }
            let Some(recognizer) = lease.recognizer() else { return Err(Failed { error: RecognizeError::other("recogniser vanished"), on_gpu }) };
            let started = Instant::now();
            match recognizer.transcribe(sample_rate, &samples[piece.clone()], &self.abort) {
                Ok(text) => {
                    tracing::info!(model = self.entry.id, samples = piece.len(), infer_ms = started.elapsed().as_millis() as u64, "local transcription done");
                    texts.push((piece.start as u64, piece.end as u64, Some(text)));
                }
                Err(error) => {
                    match error.kind {
                        // The model itself: none of its instances is used again.
                        LocalFailure::Model => {
                            lease.discard();
                            self.shared.pool.evict(self.entry.id);
                        }
                        // A run the take's own cancel stopped: no fault of the instance.
                        _ if error.by_cancel => {}
                        // Any other failure, even one that ends a cancelled take's run: the next
                        // take does not use this instance again.
                        LocalFailure::Device | LocalFailure::Memory | LocalFailure::InputTooLong | LocalFailure::Aborted | LocalFailure::Other => {
                            lease.discard()
                        }
                    }
                    tracing::warn!(model = self.entry.id, kind = error.kind.as_str(), on_gpu, error = %error, "local transcription failed");
                    return Err(Failed { error, on_gpu });
                }
            }
        }
        Ok(voltip_core::dictation::long::assemble(&texts, &[]))
    }

    /// An instance for `key`: an idle one, a new one when there is room, or the wait for one.
    fn lease(&self, key: &Key) -> Result<Lease, Failed> {
        let pool = &self.shared.pool;
        let deadline = Instant::now() + self.lock_wait;
        match pool.acquire(key, cap(self.entry.engine), deadline) {
            Some(Grant::Ready(loaded)) => Ok(Lease::new(pool.clone(), loaded)),
            Some(Grant::Load) => match load(self.loader.as_ref(), self.entry, &self.dir, key) {
                Ok(recognizer) => Ok(Lease::new(pool.clone(), pool.loaded(key.clone(), recognizer))),
                Err(e) => {
                    pool.load_failed(key);
                    Err(Failed { error: RecognizeError::new(LocalFailure::Model, e), on_gpu: false })
                }
            },
            None => {
                tracing::warn!(model = self.entry.id, wait_s = self.lock_wait.as_secs(), "every recogniser stayed busy");
                Err(Failed { error: RecognizeError::other(BUSY), on_gpu: false })
            }
        }
    }
}

#[async_trait]
impl Transcriber for LocalTranscriber {
    fn warm(&self, language: Option<&str>) {
        // The thread is detached: nothing waits for a warm-up.
        drop(self.warm_up(language));
    }

    /// The glossary is not used: none of the local engines takes a vocabulary prompt
    /// (docs/dictation.md §16.3 / §16.8 — transcribe.cpp's Qwen3-ASR has no prompt input, sherpa's
    /// SenseVoice / Paraformer have no hotwords); the dictionary corrects their text afterwards.
    async fn transcribe(&self, wav: &[u8], language: Option<&str>, _glossary: &[String]) -> Result<Transcript, DictationError> {
        let started = Instant::now();
        let entry = self.entry()?;
        let dir = self.root.join(entry.id);
        if !store::is_installed(&dir, entry) {
            return Err(DictationError::Asr(format!("本地模型未下载：{}", entry.name)));
        }
        let (samples, sample_rate) = decode_wav(wav).map_err(DictationError::Asr)?;
        if samples.is_empty() {
            return Err(DictationError::NoSpeech);
        }
        let job = Job {
            shared: self.shared.clone(),
            loader: self.loader.clone(),
            entry,
            dir,
            language: language_for(entry.engine, language).map(str::to_owned),
            compute: self.compute.clone(),
            vad: self.vad_trim.then(|| self.vad.clone()),
            pieces_vad: self.vad.clone(),
            lock_wait: self.lock_wait,
            abort: Abort::new(),
        };
        // A take cancelled meanwhile drops this future: the recognition is asked to stop.
        let mut guard = AbortOnDrop(Some(job.abort.clone()));
        let outcome = tokio::task::spawn_blocking(move || job.run(samples, sample_rate)).await;
        guard.0 = None;
        let (text, backend_note) =
            outcome.map_err(|e| DictationError::Asr(format!("本地识别任务失败：{e}")))?.map_err(|e| DictationError::Asr(format!("本地识别失败：{e}")))?;
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        // docs/dictation.md §3.5: the catalogue entry's name is the model the history records.
        Ok(Transcript { text: text.trim().to_owned(), latency_ms, model: Some(entry.name.to_owned()), backend_note })
    }
}

/// Zero-pad `samples` (at `sample_rate`) to at least [`MIN_INPUT`].
pub fn pad_to_min(samples: &mut Vec<f32>, sample_rate: u32) {
    let min = usize::try_from(MIN_INPUT.as_millis() * u128::from(sample_rate) / 1000).unwrap_or(usize::MAX);
    if samples.len() < min {
        samples.resize(min, 0.0);
    }
}

/// The language the recogniser is told: SenseVoice takes one of its five codes (anything else is
/// `auto`); Paraformer takes nothing; the Qwen3 GGUF models detect the language themselves and
/// refuse a hint (transcribe.cpp upstream); the streaming model has no language switch.
pub fn language_for(engine: Engine, hint: Option<&str>) -> Option<&'static str> {
    match engine {
        Engine::Paraformer | Engine::TranscribeCpp | Engine::ZipformerStreaming | Engine::SileroVad => None,
        Engine::SenseVoice => Some(match hint.map(|h| h.trim().to_ascii_lowercase()).as_deref() {
            Some("zh") | Some("zh-cn") | Some("zh-hans") => "zh",
            Some("en") => "en",
            Some("ja") => "ja",
            Some("ko") => "ko",
            Some("yue") | Some("zh-hk") | Some("zh-yue") => "yue",
            _ => "auto",
        }),
    }
}

/// Decode a WAV file to mono `f32` samples in `-1.0..=1.0` plus its sample rate. Multi-channel
/// audio is averaged; 8–32-bit integer and 32-bit float PCM are accepted.
pub fn decode_wav(wav: &[u8]) -> Result<(Vec<f32>, u32), String> {
    let mut reader = hound::WavReader::new(Cursor::new(wav)).map_err(|e| format!("wav: {e}"))?;
    let spec = reader.spec();
    let channels = usize::from(spec.channels.max(1));
    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>().map_err(|e| format!("wav: {e}"))?,
        hound::SampleFormat::Int => {
            let scale = 1.0 / f32::from(2u16).powi(i32::from(spec.bits_per_sample.clamp(1, 32)) - 1);
            reader.samples::<i32>().map(|s| s.map(|v| v as f32 * scale)).collect::<Result<_, _>>().map_err(|e| format!("wav: {e}"))?
        }
    };
    let mono = if channels == 1 { interleaved } else { interleaved.chunks(channels).map(|frame| frame.iter().sum::<f32>() / frame.len() as f32).collect() };
    Ok((mono, spec.sample_rate))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use voltip_core::dictation::wav;

    fn pcm16(samples: &[i16], rate: u32) -> Vec<u8> {
        wav::encode_pcm16(samples, rate)
    }

    #[test]
    fn decode_wav_handles_mono_stereo_float_and_garbage() {
        let (mono, rate) = decode_wav(&pcm16(&[0, 16_384, -32_768], 16_000)).unwrap();
        assert_eq!(rate, 16_000);
        assert_eq!(mono.len(), 3);
        assert!((mono[1] - 0.5).abs() < 1e-6 && (mono[2] + 1.0).abs() < 1e-6);
        // Stereo 48 kHz is averaged and keeps its rate (sherpa resamples).
        let spec = hound::WavSpec { channels: 2, sample_rate: 48_000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut buf = Cursor::new(Vec::new());
        {
            let mut w = hound::WavWriter::new(&mut buf, spec).unwrap();
            for s in [16_384i16, 0, -16_384, -16_384] {
                w.write_sample(s).unwrap();
            }
            w.finalize().unwrap();
        }
        let (mono, rate) = decode_wav(&buf.into_inner()).unwrap();
        assert_eq!(rate, 48_000);
        assert_eq!(mono.len(), 2);
        assert!((mono[0] - 0.25).abs() < 1e-6 && (mono[1] + 0.5).abs() < 1e-6);
        // 32-bit float passes through.
        let spec = hound::WavSpec { channels: 1, sample_rate: 16_000, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
        let mut buf = Cursor::new(Vec::new());
        {
            let mut w = hound::WavWriter::new(&mut buf, spec).unwrap();
            w.write_sample(0.75f32).unwrap();
            w.finalize().unwrap();
        }
        assert_eq!(decode_wav(&buf.into_inner()).unwrap().0, vec![0.75]);
        assert!(decode_wav(b"not a wav").unwrap_err().starts_with("wav:"));
        assert!(decode_wav(&[]).is_err());
        assert_eq!(language_for(Engine::Paraformer, Some("en")), None);
        assert_eq!(language_for(Engine::TranscribeCpp, Some("zh")), None, "Qwen3 takes no hint");
        assert_eq!(language_for(Engine::ZipformerStreaming, Some("zh")), None);
        assert_eq!(language_for(Engine::SileroVad, Some("zh")), None);
        let mut short = vec![0.5; 1600];
        pad_to_min(&mut short, 16_000);
        assert_eq!(short.len(), 20_000, "1.25 s at 16 kHz");
        assert!(short[1600..].iter().all(|&s| s == 0.0) && short[..1600].iter().all(|&s| s == 0.5));
        let mut long = vec![0.5; 30_000];
        pad_to_min(&mut long, 16_000);
        assert_eq!(long.len(), 30_000, "long takes are untouched");
        let mut at_48k = vec![0.1; 100];
        pad_to_min(&mut at_48k, 48_000);
        assert_eq!(at_48k.len(), 60_000);
        assert_eq!(MIN_INPUT, Duration::from_millis(1250));
        assert!(format!("{DefaultLoader:?}").contains("DefaultLoader"));
        let dir = tempfile::tempdir().unwrap();
        let Err(err) = DefaultLoader.load(crate::catalogue::streaming_entry(), dir.path(), None, &Compute::default()) else {
            panic!("a streaming model is not offline")
        };
        assert!(err.contains("实时预览模型不能识别整段录音"), "{err}");
        let Err(err) = DefaultLoader.load(crate::catalogue::vad_entry(), dir.path(), None, &Compute::default()) else { panic!("a VAD is not a recogniser") };
        assert!(err.contains("语音活动检测模型不能做整段识别"), "{err}");
        // The real families reach their loaders and fail on the missing files, naming the entry.
        for id in ["qwen3-asr-0.6b", "sense-voice-small", "paraformer-zh"] {
            let Err(err) = DefaultLoader.load(crate::catalogue::entry(id).unwrap(), dir.path(), None, &Compute::default()) else {
                panic!("{id} loaded from nothing")
            };
            assert!(err.contains(id), "{id}: {err}");
        }
        assert_eq!(language_for(Engine::SenseVoice, None), Some("auto"));
        assert_eq!(language_for(Engine::SenseVoice, Some("zh")), Some("zh"));
        assert_eq!(language_for(Engine::SenseVoice, Some("ZH-CN")), Some("zh"));
        assert_eq!(language_for(Engine::SenseVoice, Some("yue")), Some("yue"));
        assert_eq!(language_for(Engine::SenseVoice, Some("fr")), Some("auto"));
        assert!((1..=4).contains(&default_threads()));
    }

    /// How long a held fake call waits for its release before it gives up (the test then fails
    /// instead of hanging).
    const HOLD_LIMIT: Duration = Duration::from_secs(30);

    /// Wait (at most 5 s) until `done` says so, letting the test's runtime run its tasks.
    async fn until(done: impl Fn() -> bool, what: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(Instant::now() < deadline, "not within 5 s: {what}");
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }

    /// Echoes what it was loaded with; counts loads so the pool can be observed. Its recognisers
    /// follow one [`Script`].
    #[derive(Default)]
    struct FakeLoader {
        loads: AtomicUsize,
        fail: bool,
        computes: Mutex<Vec<Compute>>,
        script: Arc<Script>,
    }

    impl FakeLoader {
        fn new() -> Arc<Self> {
            Arc::new(Self::default())
        }

        fn failing() -> Arc<Self> {
            Arc::new(Self { fail: true, ..Self::default() })
        }

        fn loads(&self) -> usize {
            self.loads.load(Ordering::SeqCst)
        }
    }

    /// What every recogniser of a [`FakeLoader`] does (docs/dictation.md §29 tests): fail as
    /// scripted and hold calls until released. A held call runs to its end whatever its abort
    /// says, as both engines' runs do (transcribe.cpp's Qwen3-ASR looks at a cancel only before a
    /// run, sherpa-onnx not at all).
    #[derive(Default)]
    struct Script {
        /// The next calls fail with these, in order; `true` = only a call on a GPU.
        failures: Mutex<std::collections::VecDeque<(bool, LocalFailure)>>,
        /// Calls still to hold.
        hold: Mutex<usize>,
        released: Mutex<bool>,
        wake: parking_lot::Condvar,
        running: AtomicUsize,
        /// Each call's input length, in order.
        lengths: Mutex<Vec<usize>>,
        /// A call whose take was cancelled by the time it runs stops with the cancel, as
        /// transcribe.cpp's check before a Qwen3-ASR run does; otherwise it runs to its end.
        stop_on_cancel: std::sync::atomic::AtomicBool,
    }

    impl Script {
        fn fail(&self, gpu_only: bool, kind: LocalFailure) {
            self.failures.lock().push_back((gpu_only, kind));
        }

        fn hold(&self, calls: usize) {
            *self.released.lock() = false;
            *self.hold.lock() += calls;
        }

        fn release(&self) {
            *self.released.lock() = true;
            self.wake.notify_all();
        }

        fn running(&self) -> usize {
            self.running.load(Ordering::SeqCst)
        }

        /// Wait (at most 5 s) until `n` calls run.
        async fn until_running(&self, n: usize) {
            until(|| self.running() == n, &format!("{n} calls running")).await;
        }

        fn lengths(&self) -> Vec<usize> {
            self.lengths.lock().clone()
        }

        fn stop_on_cancel(&self, on: bool) {
            self.stop_on_cancel.store(on, Ordering::SeqCst);
        }
    }

    /// A take of `speech` on `t` whose call is held, cancelled while the call runs; returns once
    /// the call was released and has ended.
    async fn cancel_while_held(t: &LocalTranscriber, loader: &FakeLoader, speech: &[u8]) {
        loader.script.hold(1);
        let cancelled = tokio::spawn({
            let (t, speech) = (t.clone(), speech.to_vec());
            async move { t.transcribe(&speech, None, &[]).await }
        });
        loader.script.until_running(1).await;
        cancelled.abort();
        assert!(cancelled.await.unwrap_err().is_cancelled());
        loader.script.release();
        until(|| loader.script.running() == 0, "the cancelled call ended").await;
    }

    struct Echo {
        id: String,
        language: Option<String>,
        backend: String,
        script: Arc<Script>,
    }

    impl Recognizer for Echo {
        fn transcribe(&mut self, sample_rate: u32, samples: &[f32], abort: &Abort) -> Result<String, RecognizeError> {
            let script = &self.script;
            script.lengths.lock().push(samples.len());
            script.running.fetch_add(1, Ordering::SeqCst);
            let held = {
                let mut hold = script.hold.lock();
                let held = *hold > 0;
                *hold = hold.saturating_sub(1);
                held
            };
            let deadline = Instant::now() + HOLD_LIMIT;
            let mut released = script.released.lock();
            while held && !*released {
                assert!(Instant::now() < deadline, "a held call was not released within {HOLD_LIMIT:?}");
                script.wake.wait_for(&mut released, Duration::from_millis(2));
            }
            drop(released);
            script.running.fetch_sub(1, Ordering::SeqCst);
            if script.stop_on_cancel.load(Ordering::SeqCst) && abort.is_aborted() {
                return Err(RecognizeError::cancelled("cancelled"));
            }
            let on_gpu = self.backend != "CPU";
            let failure = {
                let mut failures = script.failures.lock();
                let next = failures.front().copied();
                next.filter(|(gpu_only, _)| on_gpu || !gpu_only).and_then(|_| failures.pop_front())
            };
            if let Some((_, kind)) = failure {
                return Err(RecognizeError::new(kind, format!("scripted {kind}")));
            }
            if samples.iter().all(|s| *s == 0.0) {
                return Err(RecognizeError::other("all zero"));
            }
            Ok(format!(" {}:{}:{sample_rate}:{} ", self.id, self.language.as_deref().unwrap_or("-"), samples.len()))
        }

        fn backend(&self) -> String {
            self.backend.clone()
        }
    }

    impl RecognizerLoader for FakeLoader {
        fn load(&self, entry: &ModelEntry, dir: &Path, language: Option<&str>, compute: &Compute) -> Result<Box<dyn Recognizer>, String> {
            self.loads.fetch_add(1, Ordering::SeqCst);
            assert!(dir.ends_with(entry.id));
            self.computes.lock().push(compute.clone());
            if self.fail {
                return Err("onnxruntime: bad model".into());
            }
            if matches!(entry.engine, Engine::ZipformerStreaming | Engine::SileroVad) {
                return DefaultLoader.load(entry, dir, language, compute);
            }
            let backend = if compute.device == LocalDevice::Gpu { "Vulkan0" } else { "CPU" };
            Ok(Box::new(Echo { id: entry.id.to_owned(), language: language.map(str::to_owned), backend: backend.to_owned(), script: self.script.clone() }))
        }
    }

    /// Pretend every recognition / streaming model is installed under `root` (manifest +
    /// right-sized files); the auxiliary VAD is left to `install_vad`.
    fn install_all(root: &Path) {
        for e in CATALOGUE.iter().filter(|e| e.tier.is_visible()) {
            let dir = root.join(e.id);
            std::fs::create_dir_all(&dir).unwrap();
            for f in e.files() {
                std::fs::File::create(dir.join(f.name)).unwrap().set_len(f.size).unwrap();
            }
            let manifest = store::Manifest {
                id: e.id.into(),
                version: store::CATALOGUE_VERSION,
                downloaded_at: 1,
                files: e.files().iter().map(|f| (f.name.to_owned(), f.sha256.to_owned())).collect(),
            };
            std::fs::write(dir.join(store::MANIFEST_FILE), serde_json::to_vec(&manifest).unwrap()).unwrap();
        }
    }

    /// docs/dictation.md §10.6: the device, GPU and thread settings reach the loader, and changing
    /// them reloads the recogniser (the same model on another device is another load).
    #[tokio::test]
    async fn the_compute_choice_reaches_the_loader_and_a_change_reloads() {
        use crate::compute::LocalDevice;
        let dir = tempfile::tempdir().unwrap();
        install_all(dir.path());
        let loader = FakeLoader::new();
        let base = LocalTranscriber::with_loader(dir.path(), CATALOGUE, loader.clone()).select("qwen3-asr-0.6b");
        let speech = pcm16(&[1000; 24_000], 16_000);
        base.transcribe(&speech, None, &[]).await.unwrap();
        let gpu = Compute { device: LocalDevice::Gpu, gpu: Some("Vulkan0".into()), threads: Some(8) };
        let on_gpu = base.clone().with_compute(gpu.clone());
        assert_eq!(on_gpu.compute(), &gpu);
        on_gpu.transcribe(&speech, None, &[]).await.unwrap();
        on_gpu.transcribe(&speech, None, &[]).await.unwrap();
        let cpu = on_gpu.clone().with_compute(Compute { device: LocalDevice::Cpu, ..gpu.clone() });
        cpu.transcribe(&speech, None, &[]).await.unwrap();
        assert_eq!(loader.loads.load(Ordering::SeqCst), 3, "one load per compute choice, none for a repeat");
        let seen = loader.computes.lock().clone();
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[0], Compute::default());
        assert_eq!(seen[1], gpu);
        assert_eq!(seen[2].device, LocalDevice::Cpu);
        // `with_threads` keeps the device choice.
        assert_eq!(on_gpu.clone().with_threads(3).compute(), &Compute { threads: Some(3), ..gpu });
    }

    /// docs/dictation.md §10.7: the warm-up loads the selected model in the background and the take
    /// reuses it; nothing happens for a model that is not installed or already loaded; a newer
    /// warm-up supersedes an older one waiting behind a take; a failed warm-up leaves the take to
    /// load again and report.
    #[tokio::test]
    async fn the_warm_up_loads_the_model_before_the_first_take() {
        let dir = tempfile::tempdir().unwrap();
        let loader = FakeLoader::new();
        let t = LocalTranscriber::with_loader(dir.path(), CATALOGUE, loader.clone()).select("sense-voice-small");
        assert!(t.warm_up(Some("zh")).is_none(), "not installed: nothing to warm");
        install_all(dir.path());
        t.warm_up(Some("zh")).unwrap().join().unwrap();
        assert_eq!(loader.loads.load(Ordering::SeqCst), 1);
        assert_eq!(t.loaded().as_deref(), Some("sense-voice-small"));
        assert!(t.warm_up(Some("zh")).is_none(), "already in memory for this language and compute");
        Transcriber::warm(&t, Some("zh"));
        let speech = pcm16(&[1000; 24_000], 16_000);
        assert_eq!(t.transcribe(&speech, Some("zh"), &[]).await.unwrap().text, "sense-voice-small:zh:16000:24000");
        assert_eq!(loader.loads.load(Ordering::SeqCst), 1, "the take found the warmed recogniser");
        // A settings change to another model: the warm-up swaps it in (one model in memory).
        let paraformer = t.select("paraformer-zh");
        paraformer.warm_up(Some("zh")).unwrap().join().unwrap();
        assert_eq!(loader.loads.load(Ordering::SeqCst), 2);
        assert_eq!(t.loaded().as_deref(), Some("paraformer-zh"));
        // Two changes while a take holds the recogniser in memory: only the newer warm-up loads.
        loader.script.hold(1);
        let holding = paraformer.clone();
        let take = tokio::spawn(async move { holding.transcribe(&pcm16(&[1000; 24_000], 16_000), Some("zh"), &[]).await });
        loader.script.until_running(1).await;
        let older = t.select("qwen3-asr-0.6b").warm_up(None).unwrap();
        let newer = t.select("sense-voice-small").warm_up(Some("en")).unwrap();
        loader.script.release();
        take.await.unwrap().unwrap();
        older.join().unwrap();
        newer.join().unwrap();
        assert_eq!(loader.loads.load(Ordering::SeqCst), 3, "the superseded warm-up did not load");
        assert_eq!(t.loaded().as_deref(), Some("sense-voice-small"));
        assert_eq!(t.transcribe(&speech, Some("en"), &[]).await.unwrap().text, "sense-voice-small:en:16000:24000");
        assert_eq!(loader.loads.load(Ordering::SeqCst), 3);
        // A loader that fails: the warm-up only logs; the take loads again and reports it.
        let broken = FakeLoader::failing();
        let failing = LocalTranscriber::with_loader(dir.path(), CATALOGUE, broken.clone()).select("paraformer-zh");
        failing.warm_up(None).unwrap().join().unwrap();
        assert_eq!(failing.loaded(), None);
        let err = failing.transcribe(&speech, None, &[]).await.unwrap_err();
        assert!(matches!(&err, DictationError::Asr(m) if m.contains("bad model")), "{err:?}");
        assert_eq!(broken.loads.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn transcriber_checks_installation_caches_the_recogniser_and_maps_errors() {
        let dir = tempfile::tempdir().unwrap();
        let loader = FakeLoader::new();
        let t = LocalTranscriber::with_loader(dir.path(), CATALOGUE, loader.clone()).with_threads(2).select("sense-voice-small");
        assert_eq!(LocalTranscriber::with_loader(dir.path(), CATALOGUE, loader.clone()).selected(), "qwen3-asr-0.6b", "the catalogue default");
        assert_eq!(t.selected(), "sense-voice-small");
        assert_eq!(t.root(), dir.path());
        let speech = pcm16(&[1000; 1600], 16_000);
        // Not installed: refused before anything loads, with the documented text.
        let err = t.transcribe(&speech, None, &[]).await.unwrap_err();
        assert_eq!(err, DictationError::Asr("本地模型未下载：轻量".into()));
        assert_eq!(loader.loads.load(Ordering::SeqCst), 0);
        install_all(dir.path());
        // 100 ms of audio is zero-padded to 1.25 s (20 000 samples) before the recogniser.
        let out = t.transcribe(&speech, Some("zh"), &[]).await.unwrap();
        assert_eq!(out.text, "sense-voice-small:zh:16000:20000", "trimmed, padded");
        assert_eq!(loader.loads.load(Ordering::SeqCst), 1);
        assert_eq!(t.loaded().as_deref(), Some("sense-voice-small"));
        let long = pcm16(&[1000; 24_000], 16_000);
        assert_eq!(t.transcribe(&long, Some("zh"), &[]).await.unwrap().text, "sense-voice-small:zh:16000:24000", "1.5 s is not padded");
        // Same model and language: no reload. A different hint: reload. Unknown hint = auto.
        t.transcribe(&speech, Some("zh"), &[]).await.unwrap();
        assert_eq!(loader.loads.load(Ordering::SeqCst), 1);
        assert_eq!(t.transcribe(&speech, Some("fr"), &[]).await.unwrap().text, "sense-voice-small:auto:16000:20000");
        assert_eq!(loader.loads.load(Ordering::SeqCst), 2);
        // `select` shares the cache: switching models reloads once, switching back reloads again.
        let pf = t.select("paraformer-zh");
        assert_eq!(pf.selected(), "paraformer-zh");
        assert_eq!(pf.transcribe(&speech, Some("zh"), &[]).await.unwrap().text, "paraformer-zh:-:16000:20000", "paraformer takes no language");
        assert_eq!(loader.loads.load(Ordering::SeqCst), 3);
        assert_eq!(t.loaded().as_deref(), Some("paraformer-zh"), "one recogniser in memory, shared");
        pf.transcribe(&speech, Some("en"), &[]).await.unwrap();
        assert_eq!(loader.loads.load(Ordering::SeqCst), 3, "the hint is irrelevant to paraformer");
        assert!(format!("{pf:?}").contains("paraformer-zh"));
        // The GGUF tiers take no hint either and go through the same cache.
        let qwen = t.select("qwen3-asr-0.6b");
        assert_eq!(qwen.transcribe(&speech, Some("zh"), &[]).await.unwrap().text, "qwen3-asr-0.6b:-:16000:20000");
        assert_eq!(loader.loads.load(Ordering::SeqCst), 4);
        qwen.transcribe(&speech, Some("en"), &[]).await.unwrap();
        assert_eq!(loader.loads.load(Ordering::SeqCst), 4);
        // The streaming entry is installed but not a whole-take model: the loader refuses.
        let zf = t.select("zipformer-stream-zh-en");
        assert!(matches!(zf.transcribe(&speech, None, &[]).await.unwrap_err(), DictationError::Asr(m) if m.contains("识别整段录音")));
        t.unload();
        assert_eq!(t.loaded(), None);
        // Other rates pass through (the recognisers resample); a recogniser error maps to `Asr`.
        assert_eq!(t.transcribe(&pcm16(&[1000; 480], 48_000), None, &[]).await.unwrap().text, "sense-voice-small:auto:48000:60000");
        let err = t.transcribe(&pcm16(&[0; 1600], 16_000), None, &[]).await.unwrap_err();
        assert_eq!(err, DictationError::Asr("本地识别失败：all zero".into()));
        // Bad audio and unknown ids.
        assert!(matches!(t.transcribe(b"RIFF", None, &[]).await.unwrap_err(), DictationError::Asr(m) if m.contains("wav")));
        assert_eq!(t.transcribe(&pcm16(&[], 16_000), None, &[]).await.unwrap_err(), DictationError::NoSpeech);
        let ghost = t.select("ghost");
        assert_eq!(ghost.transcribe(&speech, None, &[]).await.unwrap_err(), DictationError::Asr("本地模型不在目录中：ghost".into()));
        let vad = t.select("silero-vad");
        assert_eq!(
            vad.transcribe(&speech, None, &[]).await.unwrap_err(),
            DictationError::Asr("本地模型未下载：语音活动检测".into()),
            "not a recogniser, not installed either"
        );
        crate::vad::tests::install_vad(dir.path());
        assert!(matches!(vad.transcribe(&speech, None, &[]).await.unwrap_err(), DictationError::Asr(m) if m.contains("语音活动检测模型不能做整段识别")));
        // A loader failure is an `Asr` error too, and nothing is cached.
        let failing = LocalTranscriber::with_loader(dir.path(), CATALOGUE, FakeLoader::failing()).select("sense-voice-small");
        let err = failing.transcribe(&speech, None, &[]).await.unwrap_err();
        assert_eq!(err, DictationError::Asr("本地识别失败：onnxruntime: bad model".into()));
        assert_eq!(failing.loaded(), None);
    }

    /// `vad_trim` (docs/dictation.md §12) in front of the recogniser: a 3 s take of silence + tone
    /// + silence reaches the recogniser as 1.9 s (tone + 450 ms each side) when the VAD model is
    /// installed and the setting is on; off, or without the model, or with a failing detector, the
    /// take goes in whole.
    #[tokio::test]
    async fn vad_trim_cuts_the_take_before_recognition_and_fails_open() {
        use crate::vad::tests::{FakeVadLoader, install_vad, silence_tone_silence};
        let dir = tempfile::tempdir().unwrap();
        install_all(dir.path());
        let loader = FakeLoader::new();
        let take = pcm16(&silence_tone_silence(1000, 1000, 16_000).iter().map(|s| (s * 32_767.0) as i16).collect::<Vec<_>>(), 16_000);
        let vad = VadTrimmer::with_loader(dir.path(), crate::catalogue::vad_entry(), Arc::new(FakeVadLoader::energy()));
        let t = LocalTranscriber::with_loader(dir.path(), CATALOGUE, loader.clone()).select("sense-voice-small").with_vad(vad.clone());
        assert!(!t.vad_trim(), "off by default");
        let whole = t.transcribe(&take, Some("zh"), &[]).await.unwrap();
        assert_eq!(whole.text, "sense-voice-small:zh:16000:48000", "off: the whole 3 s");
        assert_eq!(whole.model.as_deref(), Some("轻量"), "the catalogue entry's name is the receipt (docs/dictation.md §3.5)");
        let trimming = t.clone().with_vad_trim(true);
        assert!(trimming.vad_trim() && trimming.vad().entry().id == "silero-vad");
        assert!(format!("{trimming:?}").contains("vad_trim: true"));
        // On, but the VAD model is not installed: fail-open, the whole take.
        assert_eq!(trimming.transcribe(&take, Some("zh"), &[]).await.unwrap().text, "sense-voice-small:zh:16000:48000");
        install_vad(dir.path());
        assert!(vad.is_installed());
        // 1.9 s (tone + 450 ms each side), give or take the detector's 512-sample window per side.
        let count = |text: &str| text.rsplit(':').next().unwrap().parse::<usize>().unwrap();
        let trimmed = count(&trimming.transcribe(&take, Some("zh"), &[]).await.unwrap().text);
        assert!((30_400..=30_400 + 1024).contains(&trimmed), "≈ 1.9 s reaches the recogniser: {trimmed}");
        assert!(vad.is_loaded(), "the detector stays loaded");
        // `select` keeps the trimmer and the setting.
        let other = trimming.select("paraformer-zh").transcribe(&take, None, &[]).await.unwrap().text;
        assert!(other.starts_with("paraformer-zh:-:16000:") && count(&other) == trimmed, "{other}");
        // A short take that trims below the minimum is still padded to 1.25 s.
        let short = pcm16(&silence_tone_silence(1000, 100, 16_000).iter().map(|s| (s * 32_767.0) as i16).collect::<Vec<_>>(), 16_000);
        assert_eq!(trimming.transcribe(&short, Some("zh"), &[]).await.unwrap().text, "sense-voice-small:zh:16000:20000");
        // A failing detector: the whole take, no error.
        let broken = VadTrimmer::with_loader(dir.path(), crate::catalogue::vad_entry(), Arc::new(FakeVadLoader { broken: true, ..FakeVadLoader::energy() }));
        let t = trimming.with_vad(broken);
        assert_eq!(t.transcribe(&take, Some("zh"), &[]).await.unwrap().text, "sense-voice-small:zh:16000:48000");
        // The real transcriber constructor carries a real trimmer over the same root.
        let real = LocalTranscriber::new(dir.path()).with_vad_trim(true);
        assert_eq!(real.vad().entry().id, "silero-vad");
        assert!(real.vad().is_installed());
    }

    fn gpu() -> Compute {
        Compute { device: LocalDevice::Gpu, gpu: Some("Vulkan0".into()), threads: None }
    }

    fn cpu_of(compute: &Compute) -> Compute {
        Compute { device: LocalDevice::Cpu, gpu: None, ..compute.clone() }
    }

    /// The text [`Echo`] returns for one input of `len` samples.
    fn echo(id: &str, language: &str, len: usize) -> String {
        format!("{id}:{language}:16000:{len}")
    }

    /// docs/dictation.md §29: a device or memory failure on the GPU is tried once more on the CPU
    /// and the history says so; a second one within ten minutes keeps the local models on the CPU
    /// until Voltip restarts. A failure on the CPU is not tried again.
    #[tokio::test]
    async fn a_gpu_failure_is_tried_again_on_the_cpu_and_a_second_keeps_the_cpu() {
        let dir = tempfile::tempdir().unwrap();
        install_all(dir.path());
        let loader = FakeLoader::new();
        let t = LocalTranscriber::with_loader(dir.path(), CATALOGUE, loader.clone()).select("qwen3-asr-0.6b").with_compute(gpu());
        let speech = pcm16(&[1000; 24_000], 16_000);
        let text = echo("qwen3-asr-0.6b", "-", 24_000);
        loader.script.fail(true, LocalFailure::Device);
        let out = t.transcribe(&speech, None, &[]).await.unwrap();
        assert_eq!((out.text.as_str(), out.backend_note), (text.as_str(), Some(AsrNote::GpuFailedRanOnCpu)));
        assert_eq!(*loader.computes.lock(), [gpu(), cpu_of(&gpu())], "the same audio, once more on the CPU");
        assert!(!t.gpu_off(), "one failure: the GPU stays");
        // The next take is on the GPU again, without a note.
        let out = t.transcribe(&speech, None, &[]).await.unwrap();
        assert_eq!(out.backend_note, None);
        assert_eq!(loader.computes.lock().last(), Some(&gpu()));
        assert_eq!(t.loaded_backend().as_deref(), Some("Vulkan0"));
        // A second failure within ten minutes: the CPU from now on.
        loader.script.fail(true, LocalFailure::Memory);
        let out = t.transcribe(&speech, None, &[]).await.unwrap();
        assert_eq!((out.text.as_str(), out.backend_note), (text.as_str(), Some(AsrNote::GpuOffUntilRestart)));
        assert!(t.gpu_off());
        let loads = loader.loads();
        for _ in 0..2 {
            let out = t.transcribe(&speech, None, &[]).await.unwrap();
            assert_eq!(out.backend_note, Some(AsrNote::GpuOffUntilRestart), "a take that would have used the GPU says why it did not");
        }
        assert_eq!(loader.loads(), loads, "the CPU instance is reused");
        assert_eq!(loader.computes.lock().last(), Some(&cpu_of(&gpu())));
        assert_eq!(t.loaded_backend().as_deref(), Some("CPU"));
        // A clone asked for the CPU anyway says nothing; the warm-up loads the CPU as well.
        let on_cpu = t.clone().with_compute(cpu_of(&gpu()));
        assert_eq!(on_cpu.transcribe(&speech, None, &[]).await.unwrap().backend_note, None);
        assert!(t.warm_up(None).is_none(), "the CPU instance is there already");
        // A failure on the CPU is reported as it is, and the instance is not kept.
        loader.script.fail(false, LocalFailure::Device);
        let err = on_cpu.transcribe(&speech, None, &[]).await.unwrap_err();
        assert_eq!(err, DictationError::Asr("本地识别失败：scripted device".into()));
        assert_eq!(t.loaded(), None);
        assert_eq!(loader.loads(), loads, "no second attempt");
    }

    /// docs/dictation.md §29: when the CPU fails too, the take says both.
    #[tokio::test]
    async fn a_failed_cpu_retry_reports_both_failures() {
        let dir = tempfile::tempdir().unwrap();
        install_all(dir.path());
        let loader = FakeLoader::new();
        let t = LocalTranscriber::with_loader(dir.path(), CATALOGUE, loader.clone()).select("sense-voice-small").with_compute(gpu());
        loader.script.fail(true, LocalFailure::Device);
        loader.script.fail(false, LocalFailure::Other);
        let err = t.transcribe(&pcm16(&[1000; 24_000], 16_000), None, &[]).await.unwrap_err();
        assert_eq!(err, DictationError::Asr("本地识别失败：显卡出错（scripted device），改用 CPU 也失败（scripted other）".into()));
        assert_eq!(loader.loads(), 2);
        // A failure that is not the device's is not tried on the CPU.
        loader.script.fail(true, LocalFailure::Other);
        let err = t.transcribe(&pcm16(&[1000; 24_000], 16_000), None, &[]).await.unwrap_err();
        assert_eq!(err, DictationError::Asr("本地识别失败：scripted other".into()));
        assert_eq!(*loader.computes.lock(), [gpu(), cpu_of(&gpu()), gpu()]);
    }

    /// docs/dictation.md §29: a model failure drops every instance of the model, so the next take
    /// loads it afresh; a failure the engine does not classify drops the instance that failed.
    #[tokio::test]
    async fn a_model_failure_drops_the_model_and_the_next_take_loads_it_again() {
        let dir = tempfile::tempdir().unwrap();
        install_all(dir.path());
        let loader = FakeLoader::new();
        let t = LocalTranscriber::with_loader(dir.path(), CATALOGUE, loader.clone()).select("sense-voice-small");
        let speech = pcm16(&[1000; 24_000], 16_000);
        t.transcribe(&speech, None, &[]).await.unwrap();
        loader.script.fail(false, LocalFailure::Other);
        assert!(t.transcribe(&speech, None, &[]).await.is_err());
        assert_eq!((t.loaded(), loader.loads()), (None, 1), "the instance that failed is dropped");
        // Two instances in memory: a take holds one while the next loads the other.
        loader.script.hold(1);
        let held = tokio::spawn({
            let (t, speech) = (t.clone(), speech.clone());
            async move { t.transcribe(&speech, None, &[]).await }
        });
        loader.script.until_running(1).await;
        t.transcribe(&speech, None, &[]).await.unwrap();
        loader.script.release();
        held.await.unwrap().unwrap();
        until(|| t.shared.pool.counts() == (2, 0, 0), "both instances idle").await;
        loader.script.fail(false, LocalFailure::Model);
        let err = t.transcribe(&speech, None, &[]).await.unwrap_err();
        assert_eq!(err, DictationError::Asr("本地识别失败：scripted model".into()));
        assert_eq!((t.loaded(), t.shared.pool.counts()), (None, (0, 0, 0)), "every instance of the model dropped");
        assert_eq!(t.transcribe(&speech, None, &[]).await.unwrap().text, echo("sense-voice-small", "auto", 24_000));
        assert_eq!(loader.loads(), 4, "loaded afresh");
    }

    /// Regression (R04, 2026-10-30): a recognition that failed with an input that was too long, a
    /// decoder that stopped early (output budget, repeating output) or an error the engine does not
    /// classify (every sherpa-onnx error) left its instance in the pool for the next take. Every
    /// failure now drops the instance that failed, and the next take loads afresh; another idle
    /// instance of the model stays. Only a run that the take's own cancel stopped gives its
    /// instance back: a failure of any other kind drops it, also when the take was cancelled
    /// meanwhile (goal review round 2, 2026-10-30).
    #[tokio::test]
    async fn regression_an_inference_error_drops_the_instance_that_failed() {
        let dir = tempfile::tempdir().unwrap();
        install_all(dir.path());
        let loader = FakeLoader::new();
        let speech = pcm16(&[1000; 24_000], 16_000);
        let qwen = LocalTranscriber::with_loader(dir.path(), CATALOGUE, loader.clone()).select("qwen3-asr-0.6b");
        qwen.transcribe(&speech, None, &[]).await.unwrap();
        for kind in [LocalFailure::InputTooLong, LocalFailure::Aborted, LocalFailure::Other] {
            let loads = loader.loads();
            loader.script.fail(false, kind);
            assert!(qwen.transcribe(&speech, None, &[]).await.is_err());
            assert_eq!(qwen.loaded(), None, "{kind}: the instance that failed is dropped");
            qwen.transcribe(&speech, None, &[]).await.unwrap();
            assert_eq!(loader.loads(), loads + 1, "{kind}: the next take loads afresh");
        }
        // A run the take's cancel stopped (transcribe.cpp's check before a run) gives it back.
        let loads = loader.loads();
        loader.script.stop_on_cancel(true);
        cancel_while_held(&qwen, &loader, &speech).await;
        until(|| qwen.shared.pool.counts() == (1, 0, 0), "the cancelled take's instance back").await;
        loader.script.stop_on_cancel(false);
        qwen.transcribe(&speech, None, &[]).await.unwrap();
        assert_eq!(loader.loads(), loads, "reused");
        // An output budget or a repetition that ends a cancelled take's run is a failure all the same.
        loader.script.fail(false, LocalFailure::Aborted);
        cancel_while_held(&qwen, &loader, &speech).await;
        until(|| qwen.shared.pool.counts() == (0, 0, 0), "the instance that failed dropped").await;
        qwen.transcribe(&speech, None, &[]).await.unwrap();
        assert_eq!(loader.loads(), loads + 1, "loaded afresh");
        // Only the instance that failed: the model's other one stays.
        let t = qwen.select("sense-voice-small");
        loader.script.hold(1);
        let held = tokio::spawn({
            let (t, speech) = (t.clone(), speech.clone());
            async move { t.transcribe(&speech, None, &[]).await }
        });
        loader.script.until_running(1).await;
        t.transcribe(&speech, None, &[]).await.unwrap();
        loader.script.release();
        held.await.unwrap().unwrap();
        until(|| t.shared.pool.counts() == (2, 0, 0), "both instances idle").await;
        loader.script.fail(false, LocalFailure::Other);
        assert!(t.transcribe(&speech, None, &[]).await.is_err());
        assert_eq!((t.shared.pool.counts(), t.loaded().as_deref()), ((1, 0, 0), Some("sense-voice-small")));
    }

    /// Regression (R04, 2026-10-30): a take cancelled while transcribe.cpp ran kept the one
    /// recogniser until its whole inference ended, and the next take waited behind it.
    /// transcribe.cpp (0.2.3 and 0.4.1) looks at a cancel only before a Qwen3-ASR run, so the run
    /// in progress still ends as it would have; but a take is recognised in pieces of at most
    /// `max_piece`, and a cancelled one starts no further piece. The next take gets the same
    /// instance as soon as the piece in progress is done, not after the rest of the cancelled take.
    #[tokio::test]
    async fn regression_a_cancelled_local_take_releases_the_model() {
        let dir = tempfile::tempdir().unwrap();
        install_all(dir.path());
        let loader = FakeLoader::new();
        let t = LocalTranscriber::with_loader(dir.path(), short_pieces(), loader.clone()).select("qwen3-asr-0.6b").with_lock_wait(Duration::from_secs(10));
        // 70 s: three pieces of at most 30 s.
        loader.script.hold(1);
        let cancelled = tokio::spawn({
            let t = t.clone();
            async move { t.transcribe(&pcm16(&[1000; 70 * 16_000], 16_000), None, &[]).await }
        });
        loader.script.until_running(1).await;
        // The core aborts the take's task while its first piece runs.
        cancelled.abort();
        assert!(cancelled.await.unwrap_err().is_cancelled());
        let next = tokio::spawn({
            let t = t.clone();
            async move { t.transcribe(&pcm16(&[1000; 24_000], 16_000), None, &[]).await }
        });
        // One instance (transcribe.cpp): the next take waits for the piece in progress.
        loader.script.release();
        assert_eq!(next.await.unwrap().unwrap().text, echo("qwen3-asr-0.6b", "-", 24_000));
        let lengths = loader.script.lengths();
        assert_eq!(lengths.len(), 2, "the cancelled take's first piece and the next take, nothing more: {lengths:?}");
        assert!(lengths[0] <= 30 * 16_000 && lengths[1] == 24_000, "{lengths:?}");
        assert_eq!(loader.loads(), 1, "the same instance, one in memory");
    }

    /// Regression (R04, 2026-10-30): sherpa-onnx cannot stop a decode, so a cancelled take held
    /// the recogniser to its end and the next take waited. The next take now runs on a second
    /// instance; with both busy a third gives up after its lock wait with「上一次识别还没结束」.
    /// No more than two instances are ever in memory, and they are reused once free.
    #[tokio::test]
    async fn regression_a_cancelled_sherpa_take_does_not_block_the_next() {
        let dir = tempfile::tempdir().unwrap();
        install_all(dir.path());
        let loader = FakeLoader::new();
        let t = LocalTranscriber::with_loader(dir.path(), CATALOGUE, loader.clone()).select("sense-voice-small").with_lock_wait(Duration::from_secs(10));
        let speech = pcm16(&[1000; 24_000], 16_000);
        let spawn = |t: &LocalTranscriber| {
            let (t, speech) = (t.clone(), speech.clone());
            tokio::spawn(async move { t.transcribe(&speech, Some("zh"), &[]).await })
        };
        loader.script.hold(1);
        let cancelled = spawn(&t);
        loader.script.until_running(1).await;
        cancelled.abort();
        assert!(cancelled.await.unwrap_err().is_cancelled());
        assert_eq!(loader.script.running(), 1, "the cancelled decode runs on");
        let started = Instant::now();
        assert_eq!(t.transcribe(&speech, Some("zh"), &[]).await.unwrap().text, echo("sense-voice-small", "zh", 24_000));
        assert!(started.elapsed() < Duration::from_secs(5), "the next take waited {:?}", started.elapsed());
        assert_eq!(loader.loads(), 2, "on a second instance");
        // Both instances busy: a third take gives up after its lock wait.
        loader.script.hold(1);
        let busy = spawn(&t);
        loader.script.until_running(2).await;
        let started = Instant::now();
        let err = t.clone().with_lock_wait(Duration::from_millis(300)).transcribe(&speech, Some("zh"), &[]).await.unwrap_err();
        assert_eq!(err, DictationError::Asr(format!("本地识别失败：{BUSY}")));
        assert!(started.elapsed() >= Duration::from_millis(300));
        assert_eq!(loader.loads(), 2, "never a third instance");
        // Free again: both come back and are reused.
        loader.script.release();
        busy.await.unwrap().unwrap();
        until(|| loader.script.running() == 0 && t.shared.pool.counts() == (2, 0, 0), "both instances back").await;
        for _ in 0..3 {
            t.transcribe(&speech, Some("zh"), &[]).await.unwrap();
        }
        assert_eq!(loader.loads(), 2);
    }

    /// Regression (R04, 2026-10-30): the second instance after a cancelled sherpa-onnx take only
    /// helped a next take of the same model, language and compute; one in another language, or
    /// with another model, waited for the cancelled decode (one model at a time). The instance
    /// finishing a cancelled take now holds back nothing: the next take loads what it needs beside
    /// it, and the cancelled one is dropped when it comes back to a pool that holds another model.
    /// A cancelled transcribe.cpp take still holds its one instance until its piece ends: the next
    /// take waits at most for that piece (the user's decision of 2026-10-30).
    #[tokio::test]
    async fn regression_a_cancelled_sherpa_take_holds_back_no_other_language_or_model() {
        let dir = tempfile::tempdir().unwrap();
        install_all(dir.path());
        let loader = FakeLoader::new();
        let t = LocalTranscriber::with_loader(dir.path(), CATALOGUE, loader.clone()).select("sense-voice-small").with_lock_wait(Duration::from_secs(10));
        let speech = pcm16(&[1000; 24_000], 16_000);
        let spawn = |t: &LocalTranscriber, language: Option<&'static str>| {
            let (t, speech) = (t.clone(), speech.clone());
            tokio::spawn(async move { t.transcribe(&speech, language, &[]).await })
        };
        loader.script.hold(1);
        let cancelled = spawn(&t, Some("zh"));
        loader.script.until_running(1).await;
        cancelled.abort();
        assert!(cancelled.await.unwrap_err().is_cancelled());
        // Another language, then another model, while the cancelled decode runs on.
        let started = Instant::now();
        assert_eq!(t.transcribe(&speech, Some("en"), &[]).await.unwrap().text, echo("sense-voice-small", "en", 24_000));
        let paraformer = t.select("paraformer-zh");
        assert_eq!(paraformer.transcribe(&speech, None, &[]).await.unwrap().text, echo("paraformer-zh", "-", 24_000));
        assert!(started.elapsed() < Duration::from_secs(5), "the next takes waited {:?}", started.elapsed());
        assert_eq!(loader.script.running(), 1, "the cancelled decode runs on");
        // It ends: dropped, because another model is in memory now.
        loader.script.release();
        until(|| loader.script.running() == 0 && t.shared.pool.counts() == (1, 0, 0), "the cancelled take's instance gone").await;
        assert_eq!((t.loaded().as_deref(), loader.loads()), (Some("paraformer-zh"), 3));
        // transcribe.cpp: the next take, whatever its model, waits for the piece in progress.
        loader.script.hold(1);
        let cancelled = spawn(&t.select("qwen3-asr-0.6b"), None);
        loader.script.until_running(1).await;
        cancelled.abort();
        assert!(cancelled.await.unwrap_err().is_cancelled());
        let err = t.clone().with_lock_wait(Duration::from_millis(300)).transcribe(&speech, Some("zh"), &[]).await.unwrap_err();
        assert_eq!(err, DictationError::Asr(format!("本地识别失败：{BUSY}")));
        loader.script.release();
        assert_eq!(t.transcribe(&speech, Some("zh"), &[]).await.unwrap().text, echo("sense-voice-small", "zh", 24_000), "once the piece is done");
    }

    /// A take cancelled between two pieces of a long input recognises no further piece.
    #[tokio::test]
    async fn a_cancelled_take_stops_between_pieces() {
        let dir = tempfile::tempdir().unwrap();
        install_all(dir.path());
        let loader = FakeLoader::new();
        let t = LocalTranscriber::with_loader(dir.path(), short_pieces(), loader.clone()).select("sense-voice-small");
        loader.script.hold(1);
        let cancelled = tokio::spawn({
            let t = t.clone();
            async move { t.transcribe(&pcm16(&[1000; 70 * 16_000], 16_000), None, &[]).await }
        });
        loader.script.until_running(1).await;
        cancelled.abort();
        assert!(cancelled.await.unwrap_err().is_cancelled());
        loader.script.release();
        until(|| t.shared.pool.counts() == (1, 0, 0), "the instance back").await;
        assert_eq!(loader.script.lengths().len(), 1, "the first piece only");
    }

    /// The real catalogue with SenseVoice and Qwen3-ASR 0.6B taking at most 30 s at a time and
    /// Paraformer 119 s, whatever the measured values are.
    fn short_pieces() -> &'static [ModelEntry] {
        let entries: Vec<ModelEntry> = CATALOGUE
            .iter()
            .map(|e| match e.id {
                "sense-voice-small" | "qwen3-asr-0.6b" => ModelEntry { max_piece: Duration::from_secs(30), ..*e },
                "paraformer-zh" => ModelEntry { max_piece: Duration::from_secs(119), ..*e },
                _ => *e,
            })
            .collect();
        Box::leak(entries.into_boxed_slice())
    }

    /// `seconds` of tone at 16 kHz with a 600 ms pause before every 8 s mark, as 16-bit PCM.
    fn talk(seconds: usize) -> Vec<i16> {
        let (period, pause) = (8 * 16_000, 600 * 16);
        (0..seconds * 16_000)
            .map(|i| if i % period >= period - pause { 0 } else { ((i as f32 / 16_000.0 * 440.0 * std::f32::consts::TAU).sin() * 16_000.0) as i16 })
            .collect()
    }

    /// docs/dictation.md §29: an input longer than the model's `max_piece` is cut where it pauses
    /// (with the VAD) or where it is quietest (without), every piece is recognised on the same
    /// instance, and the pieces' texts are put together in order; an input that fits goes in whole.
    #[tokio::test]
    async fn an_input_longer_than_the_model_takes_is_recognised_piece_by_piece() {
        use crate::vad::tests::{FakeVadLoader, install_vad};
        let dir = tempfile::tempdir().unwrap();
        install_all(dir.path());
        let loader = FakeLoader::new();
        let vad = VadTrimmer::with_loader(dir.path(), crate::catalogue::vad_entry(), Arc::new(FakeVadLoader::energy()));
        let t = LocalTranscriber::with_loader(dir.path(), short_pieces(), loader.clone()).select("sense-voice-small").with_vad(vad);
        let max = 30 * 16_000;
        let take = talk(70);
        let check = |lengths: &[usize], text: &str| {
            assert!(lengths.len() >= 3, "{lengths:?}");
            assert!(lengths.iter().all(|&l| l <= max), "a piece longer than 30 s: {lengths:?}");
            assert_eq!(lengths.iter().sum::<usize>(), take.len(), "every sample once");
            let pieces: Vec<String> = lengths.iter().map(|&l| echo("sense-voice-small", "zh", l)).collect();
            assert_eq!(text, pieces.join(" "));
        };
        // Without the VAD model: the cut where the last 5 s before 30 s is quietest.
        let out = t.transcribe(&pcm16(&take, 16_000), Some("zh"), &[]).await.unwrap();
        let forced = loader.script.lengths();
        check(&forced, &out.text);
        assert_eq!(loader.loads(), 1, "one instance for every piece");
        // With it: at the first pause once a piece is 15 s long (half of 30 s), inside the pause.
        install_vad(dir.path());
        loader.script.lengths.lock().clear();
        let out = t.transcribe(&pcm16(&take, 16_000), Some("zh"), &[]).await.unwrap();
        let at_pauses = loader.script.lengths();
        check(&at_pauses, &out.text);
        let mut at = 0;
        for length in &at_pauses[..at_pauses.len() - 1] {
            at += length;
            assert!(take[at - 1] == 0 && take[at] == 0, "the cut at {:.2} s is inside the sound", at as f64 / 16_000.0);
        }
        assert_ne!(forced, at_pauses);
        // An input that fits: whole, and so is every input of a model that takes 119 s.
        loader.script.lengths.lock().clear();
        t.transcribe(&pcm16(&take[..max], 16_000), Some("zh"), &[]).await.unwrap();
        t.select("paraformer-zh").transcribe(&pcm16(&take, 16_000), None, &[]).await.unwrap();
        assert_eq!(loader.script.lengths(), [max, take.len()]);
    }
}
