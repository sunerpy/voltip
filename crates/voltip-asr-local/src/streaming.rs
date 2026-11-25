//! [`LocalStreamingTranscriber`]: the dictation pipeline's `StreamingTranscriber`
//! (docs/dictation.md §11) over the catalogue's streaming model (`zipformer-stream-zh-en`, a
//! sherpa-onnx `OnlineRecognizer`).
//!
//! The recogniser is loaded once and kept (`Mutex<Option<…>>`, shared between clones), either by
//! `warm()` on a background thread or by the first `open()`; every session is one `OnlineStream`
//! over that recogniser. The binding lives in `sherpa.rs` behind [`StreamingLoader`] /
//! [`StreamingRecognizer`], so the unit tests run on a fake without a model file. Endpoint rule
//! (dictation rhythm, decided 2026-09-25, rule 2 lengthened 2026-11-23): rule1 2.0 s of silence
//! before any word, rule2 1.2 s of silence after words, rule3 20 s of speech; after an endpoint the
//! sentence is committed and the stream reset. Measured: load 2.6 s, 4 ms average / 44 ms maximum per 100 ms chunk on two threads.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use parking_lot::Mutex;
use voltip_core::dictation::{DictationError, StreamEvent, StreamFinal, StreamingSession, StreamingTranscriber, SwitchReason};
use voltip_core::providers::ProviderId;

use crate::catalogue::{CATALOGUE, ModelEntry};
use crate::root::ModelsRoot;
use crate::sherpa::SherpaStreamingLoader;
use crate::store;

/// Decode threads for the streaming recogniser (real-time budget is 100 ms per 100 ms chunk;
/// two threads leave the whole-take recogniser its own cores).
pub const STREAMING_THREADS: usize = 2;

/// A loaded streaming recogniser: opens sessions (one per recording).
pub trait StreamingRecognizer: Send + Sync {
    /// Open a session over this recogniser.
    fn open(self: Arc<Self>) -> Box<dyn StreamingSession>;
}

/// Builds a [`StreamingRecognizer`] from an installed model directory. The real one wraps
/// sherpa-onnx's `OnlineRecognizer`; tests plug in a fake.
pub trait StreamingLoader: Send + Sync {
    /// Load `entry` from `dir` on `threads` threads.
    fn load(&self, entry: &ModelEntry, dir: &Path, threads: usize) -> Result<Arc<dyn StreamingRecognizer>, String>;
}

/// The live-preview recogniser over the model library.
#[derive(Clone)]
pub struct LocalStreamingTranscriber {
    root: ModelsRoot,
    entry: &'static ModelEntry,
    loader: Arc<dyn StreamingLoader>,
    cache: Arc<Mutex<Option<Arc<dyn StreamingRecognizer>>>>,
    /// A `warm()` load is in flight.
    warming: Arc<AtomicBool>,
    threads: usize,
}

impl std::fmt::Debug for LocalStreamingTranscriber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalStreamingTranscriber")
            .field("root", &self.root)
            .field("model", &self.entry.id)
            .field("threads", &self.threads)
            .field("loaded", &self.is_loaded())
            .field("warming", &self.warming.load(Ordering::SeqCst))
            .finish()
    }
}

impl LocalStreamingTranscriber {
    /// sherpa-onnx over the real catalogue's streaming entry under `root` (`<app data dir>/models`).
    pub fn new(root: impl Into<ModelsRoot>) -> Self {
        Self::with_loader(root, crate::catalogue::streaming_entry(), Arc::new(SherpaStreamingLoader))
    }

    /// Any entry and loader (tests). `entry` should be one of `CATALOGUE`'s streaming entries.
    pub fn with_loader(root: impl Into<ModelsRoot>, entry: &'static ModelEntry, loader: Arc<dyn StreamingLoader>) -> Self {
        debug_assert!(CATALOGUE.iter().any(|e| e.id == entry.id) || cfg!(test));
        Self { root: root.into(), entry, loader, cache: Arc::new(Mutex::new(None)), warming: Arc::new(AtomicBool::new(false)), threads: STREAMING_THREADS }
    }

    /// Decode threads (default [`STREAMING_THREADS`]).
    pub fn with_threads(self, threads: usize) -> Self {
        Self { threads: threads.max(1), ..self }
    }

    /// The catalogue entry this previews with.
    pub fn entry(&self) -> &'static ModelEntry {
        self.entry
    }

    /// The library root.
    pub fn root(&self) -> PathBuf {
        self.root.get()
    }

    /// Whether the model files are on disk and verified.
    pub fn is_installed(&self) -> bool {
        store::is_installed(&self.root.join(self.entry.id), self.entry)
    }

    /// Whether the recogniser is in memory.
    pub fn is_loaded(&self) -> bool {
        self.cache.lock().is_some()
    }

    /// Drop the recogniser (the memory) until the next `open` / `warm`.
    pub fn unload(&self) {
        *self.cache.lock() = None;
    }

    /// The loaded recogniser, loading it now when needed. Blocking (2.6 s measured for a cold load).
    fn recognizer(&self) -> Result<Arc<dyn StreamingRecognizer>, DictationError> {
        if let Some(r) = self.cache.lock().clone() {
            return Ok(r);
        }
        let dir = self.root.join(self.entry.id);
        if !store::is_installed(&dir, self.entry) {
            return Err(DictationError::Asr(format!("实时识别模型未下载：{}", self.entry.name)));
        }
        let started = Instant::now();
        let loaded = self.loader.load(self.entry, &dir, self.threads).map_err(|e| DictationError::Asr(format!("实时识别模型加载失败：{e}")))?;
        tracing::info!(model = self.entry.id, threads = self.threads, load_ms = started.elapsed().as_millis() as u64, "streaming model loaded");
        let mut slot = self.cache.lock();
        // Another thread may have loaded meanwhile (`warm` racing `open`): keep the first.
        Ok(slot.get_or_insert(loaded).clone())
    }
}

/// One session of the library's streaming model, which says what wrote its text (docs/dictation.md
/// §3.5): the model's catalogue name, as a whole take of a local model records it, and this
/// computer. Without it a streamed take recorded the configured model, which never heard it.
struct LocalSession {
    inner: Box<dyn StreamingSession>,
    model: &'static str,
}

impl StreamingSession for LocalSession {
    fn feed(&mut self, pcm16k: &[f32]) {
        self.inner.feed(pcm16k);
    }

    fn poll(&mut self) -> StreamEvent {
        self.inner.poll()
    }

    fn finish(self: Box<Self>) -> Result<StreamFinal, DictationError> {
        let model = self.model;
        let fin = self.inner.finish()?;
        Ok(StreamFinal {
            model: fin.model.clone().or_else(|| Some(model.to_owned())),
            provider: Some(ProviderId::Local),
            fallback: None,
            glossary_sent: fin.glossary_sent.or(Some(false)),
            ..fin
        })
    }

    fn model(&self) -> Option<String> {
        self.inner.model().or_else(|| Some(self.model.to_owned()))
    }

    fn served(&self) -> (Option<ProviderId>, Option<SwitchReason>) {
        (Some(ProviderId::Local), None)
    }

    /// An on-device model takes no glossary (docs/dictation.md §16.3, §30.6).
    fn glossary_sent(&self) -> Option<bool> {
        self.inner.glossary_sent().or(Some(false))
    }
}

impl StreamingTranscriber for LocalStreamingTranscriber {
    fn open(&self, language: Option<&str>) -> Result<Box<dyn StreamingSession>, DictationError> {
        // The Zipformer zh-en transducer has no language switch; the hint is only logged.
        tracing::debug!(model = self.entry.id, ?language, "streaming session opened");
        Ok(Box::new(LocalSession { inner: self.recognizer()?.open(), model: self.entry.name }))
    }

    fn warm(&self) {
        if self.is_loaded() || !self.is_installed() || self.warming.swap(true, Ordering::SeqCst) {
            return;
        }
        let me = self.clone();
        // A plain thread: the loader is synchronous and may take seconds; nothing awaits it.
        std::thread::Builder::new()
            .name("voltip-stream-warm".into())
            .spawn(move || {
                if let Err(e) = me.recognizer() {
                    tracing::warn!(error = %e, "streaming model warm-up failed; the first preview will retry");
                }
                me.warming.store(false, Ordering::SeqCst);
            })
            .map(drop)
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "could not spawn the streaming warm-up thread");
                self.warming.store(false, Ordering::SeqCst);
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    struct FakeLoader {
        loads: AtomicUsize,
        fail: bool,
    }

    struct FakeRecognizer {
        id: String,
    }

    impl StreamingRecognizer for FakeRecognizer {
        fn open(self: Arc<Self>) -> Box<dyn StreamingSession> {
            Box::new(FakeSession { id: self.id.clone(), fed: 0 })
        }
    }

    struct FakeSession {
        id: String,
        fed: usize,
    }

    impl StreamingSession for FakeSession {
        fn feed(&mut self, pcm16k: &[f32]) {
            self.fed += pcm16k.len();
        }
        fn poll(&mut self) -> StreamEvent {
            if self.fed > 0 { StreamEvent::Partial { current: format!("{}:{}", self.id, self.fed) } } else { StreamEvent::Idle }
        }
        fn finish(self: Box<Self>) -> Result<StreamFinal, DictationError> {
            Ok(StreamFinal {
                committed: Vec::new(),
                tail: format!("{}:{}", self.id, self.fed),
                model: None,
                provider: None,
                fallback: None,
                glossary_sent: None,
                note: None,
            })
        }
    }

    impl StreamingLoader for FakeLoader {
        fn load(&self, entry: &ModelEntry, dir: &Path, threads: usize) -> Result<Arc<dyn StreamingRecognizer>, String> {
            self.loads.fetch_add(1, Ordering::SeqCst);
            assert!(dir.ends_with(entry.id));
            assert!(threads >= 1);
            if self.fail {
                return Err("onnxruntime: bad model".into());
            }
            Ok(Arc::new(FakeRecognizer { id: entry.id.to_owned() }))
        }
    }

    fn install(root: &Path, e: &ModelEntry) {
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

    fn wait_until(cond: impl Fn() -> bool) {
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while !cond() && Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(cond(), "condition not met within 5 s");
    }

    #[test]
    fn streaming_transcriber_checks_installation_loads_once_warms_and_maps_errors() {
        let dir = tempfile::tempdir().unwrap();
        let loader = Arc::new(FakeLoader { loads: AtomicUsize::new(0), fail: false });
        let entry = crate::catalogue::streaming_entry();
        let t = LocalStreamingTranscriber::with_loader(dir.path(), entry, loader.clone()).with_threads(2);
        assert_eq!(t.entry().id, "zipformer-stream-zh-en");
        assert_eq!(t.root(), dir.path());
        assert!(!t.is_installed() && !t.is_loaded());
        // Not installed: `open` refuses with the documented text, `warm` does nothing.
        assert_eq!(t.open(None).err(), Some(DictationError::Asr("实时识别模型未下载：实时预览".into())));
        t.warm();
        assert_eq!(loader.loads.load(Ordering::SeqCst), 0);
        install(dir.path(), entry);
        assert!(t.is_installed());
        // `warm` loads on a background thread, exactly once; `open` then reuses it.
        t.warm();
        t.warm();
        wait_until(|| t.is_loaded() && !t.warming.load(Ordering::SeqCst));
        assert_eq!(loader.loads.load(Ordering::SeqCst), 1);
        t.warm();
        assert_eq!(loader.loads.load(Ordering::SeqCst), 1, "already loaded: no second load");
        let mut s = t.open(Some("zh")).unwrap();
        assert_eq!(loader.loads.load(Ordering::SeqCst), 1);
        assert_eq!(s.poll(), StreamEvent::Idle);
        s.feed(&[0.0; 1600]);
        assert_eq!(s.poll(), StreamEvent::Partial { current: "zipformer-stream-zh-en:1600".into() });
        assert_eq!(s.finish().unwrap().tail, "zipformer-stream-zh-en:1600");
        // Clones share the recogniser; `unload` drops it and the next `open` reloads.
        let clone = t.clone();
        assert!(clone.is_loaded());
        t.unload();
        assert!(!clone.is_loaded());
        clone.open(None).unwrap();
        assert_eq!(loader.loads.load(Ordering::SeqCst), 2);
        assert!(format!("{t:?}").contains("zipformer-stream-zh-en"));
        // A loader failure maps to `Asr`, nothing is cached, and `warm` only logs.
        let failing = LocalStreamingTranscriber::with_loader(dir.path(), entry, Arc::new(FakeLoader { loads: AtomicUsize::new(0), fail: true }));
        assert_eq!(failing.open(None).err(), Some(DictationError::Asr("实时识别模型加载失败：onnxruntime: bad model".into())));
        assert!(!failing.is_loaded());
        failing.warm();
        wait_until(|| !failing.warming.load(Ordering::SeqCst));
        assert!(!failing.is_loaded());
        assert_eq!(STREAMING_THREADS, 2);
    }

    /// A take whose text the local streaming model wrote names that model and this computer
    /// (docs/dictation.md §3.5). Regression, 2026-11-23 on windows-local: 边说边识别 takes whose
    /// text came from zipformer-stream-zh-en showed Qwen/Qwen3-ASR-1.7B · 内置服务 in the history,
    /// a model that never heard them: the session reported no receipt, and the take fell back to
    /// the configured model.
    #[test]
    fn regression_a_streamed_take_of_the_local_model_names_it_and_this_computer() {
        let dir = tempfile::tempdir().unwrap();
        let entry = crate::catalogue::streaming_entry();
        install(dir.path(), entry);
        let t = LocalStreamingTranscriber::with_loader(dir.path(), entry, Arc::new(FakeLoader { loads: AtomicUsize::new(0), fail: false }));
        let mut s = t.open(None).unwrap();
        // Read before the end too: a session that degrades is asked what wrote its text so far.
        assert_eq!(s.model().as_deref(), Some("实时预览"));
        assert_eq!(s.served(), (Some(ProviderId::Local), None));
        assert_eq!(s.glossary_sent(), Some(false), "an on-device model takes no glossary");
        s.feed(&[0.0; 1600]);
        assert_eq!(s.poll(), StreamEvent::Partial { current: "zipformer-stream-zh-en:1600".into() });
        let fin = s.finish().unwrap();
        assert_eq!(fin.tail, "zipformer-stream-zh-en:1600");
        assert_eq!((fin.model.as_deref(), fin.provider, fin.fallback, fin.glossary_sent), (Some("实时预览"), Some(ProviderId::Local), None, Some(false)));
    }
}
