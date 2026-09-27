//! [`LocalTranscriber`]: the dictation pipeline's `Transcriber` over a local recogniser — a
//! transcribe.cpp GGUF session (`gguf.rs`) or a sherpa-onnx recogniser (`sherpa.rs`), picked by
//! the catalogue entry's engine ([`DefaultLoader`]).
//!
//! The recogniser is loaded ahead of the first take ([`LocalTranscriber::warm_up`], the port's
//! `warm`: at start-up and after every configuration change, docs/dictation.md §10.7), or at the
//! latest by the take, and kept (`Mutex<Option<Loaded>>`, shared between the clones the engine
//! factory hands out), rebuilt only when the selected model, the language hint or the compute
//! choice changes. Loading and inference run on a blocking thread. Every failure maps to
//! [`DictationError::Asr`] (`FailureCode::Asr` on the pill); a model that is not installed fails
//! before any file is touched with the documented「本地模型未下载」text. Takes shorter than
//! [`MIN_INPUT`] are zero-padded to it first: SenseVoice / Paraformer misbehave on very short input
//! (decided 2026-09-25); the 300 ms / silence → `NoSpeech` rule runs before this in the core. With
//! `vad_trim` on (docs/dictation.md §12) the take is first cut to its speech by the
//! [`VadTrimmer`] — fail-open: without the VAD model, or when it fails, the take goes in whole.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::Mutex;
use voltip_core::dictation::{DictationError, Transcriber, Transcript};

use crate::catalogue::{CATALOGUE, Engine, ModelEntry};
use crate::compute::Compute;
use crate::gguf::GgufLoader;
use crate::sherpa::SherpaLoader;
use crate::store;
use crate::vad::VadTrimmer;

/// Shortest input handed to a local recogniser; shorter takes are zero-padded to this length.
pub const MIN_INPUT: Duration = Duration::from_millis(1250);

/// A loaded recogniser: one model, one language setting.
pub trait Recognizer: Send {
    /// Recognise mono PCM samples in `-1.0..=1.0` at `sample_rate` Hz (resampled inside when
    /// not 16 kHz). Returns the text, untrimmed.
    fn transcribe(&mut self, sample_rate: u32, samples: &[f32]) -> Result<String, String>;

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
            Engine::ZipformerStreaming => Err(format!("{}: 流式预览模型不能做整段识别", entry.id)),
            Engine::SileroVad => Err(format!("{}: 语音活动检测模型不能做整段识别", entry.id)),
        }
    }
}

struct Loaded {
    id: String,
    language: Option<String>,
    compute: Compute,
    recognizer: Box<dyn Recognizer>,
}

impl Loaded {
    fn is(&self, id: &str, language: &Option<String>, compute: &Compute) -> bool {
        self.id == id && self.language == *language && self.compute == *compute
    }
}

/// Make `slot` hold `entry` loaded for `language` on `compute`, loading it unless it already does.
/// The old recogniser is dropped first, so two models are never in memory together. Blocking.
fn load_into<'a>(
    slot: &'a mut Option<Loaded>,
    loader: &dyn RecognizerLoader,
    entry: &ModelEntry,
    dir: &Path,
    language: Option<String>,
    compute: &Compute,
) -> Result<&'a mut Loaded, String> {
    if !slot.as_ref().is_some_and(|l| l.is(entry.id, &language, compute)) {
        *slot = None;
        let started = Instant::now();
        let recognizer = loader.load(entry, dir, language.as_deref(), compute)?;
        tracing::info!(
            model = entry.id,
            ?language,
            ?compute,
            backend = %recognizer.backend(),
            load_ms = started.elapsed().as_millis() as u64,
            "local model loaded"
        );
        *slot = Some(Loaded { id: entry.id.to_owned(), language, compute: compute.clone(), recognizer });
    }
    slot.as_mut().ok_or_else(|| "recogniser vanished".to_owned())
}

/// Offline speech-to-text over the model library.
#[derive(Clone)]
pub struct LocalTranscriber {
    root: PathBuf,
    catalogue: &'static [ModelEntry],
    loader: Arc<dyn RecognizerLoader>,
    cache: Arc<Mutex<Option<Loaded>>>,
    selected: String,
    /// Where the recogniser runs (docs/dictation.md §10.6); a change reloads it.
    compute: Compute,
    /// The VAD over the same library (docs/dictation.md §12); shared between the clones.
    vad: VadTrimmer,
    /// `EngineSettings.vad_trim`: cut the take to its speech before recognition.
    vad_trim: bool,
    /// Warm-ups asked for so far (shared between the clones): one that finds a newer ticket when
    /// it gets the recogniser gives way to it.
    warm_ups: Arc<AtomicU64>,
}

impl std::fmt::Debug for LocalTranscriber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalTranscriber")
            .field("root", &self.root)
            .field("selected", &self.selected)
            .field("compute", &self.compute)
            .field("vad_trim", &self.vad_trim)
            // `None` while a load or a take holds the recogniser.
            .field("loaded", &self.cache.try_lock().map(|slot| slot.as_ref().map(|l| l.id.clone())))
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
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self::with_loader(root, CATALOGUE, Arc::new(DefaultLoader))
    }

    /// Any catalogue and loader (tests); the VAD is the real one over the same root until
    /// [`LocalTranscriber::with_vad`].
    pub fn with_loader(root: impl Into<PathBuf>, catalogue: &'static [ModelEntry], loader: Arc<dyn RecognizerLoader>) -> Self {
        let root = root.into();
        Self {
            vad: VadTrimmer::new(root.clone()),
            root,
            catalogue,
            loader,
            cache: Arc::new(Mutex::new(None)),
            selected: crate::catalogue::DEFAULT_MODEL_ID.to_owned(),
            compute: Compute::default(),
            vad_trim: false,
            warm_ups: Arc::new(AtomicU64::new(0)),
        }
    }

    /// A handle on the same loaded recogniser that transcribes with `id` (the engine factory calls
    /// this on every settings change; the model is only reloaded when `id` differs).
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
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Id of the recogniser currently in memory, if any.
    pub fn loaded(&self) -> Option<String> {
        self.cache.lock().as_ref().map(|l| l.id.clone())
    }

    /// The backend the recogniser in memory runs on (what `auto` / `gpu` turned into).
    pub fn loaded_backend(&self) -> Option<String> {
        self.cache.lock().as_ref().map(|l| l.recognizer.backend())
    }

    /// Drop the recogniser (the memory) until the next transcription.
    pub fn unload(&self) {
        *self.cache.lock() = None;
    }

    /// Load the selected model for `language` where [`Self::compute`] says, now, on a background
    /// thread (docs/dictation.md §10.7), so the next take does not wait for it. Nothing happens
    /// when the model is not installed or is already in memory for this language and compute. The
    /// thread waits while a take or another load holds the recogniser; a newer warm-up supersedes
    /// it meanwhile. A failure only logs: the next take loads again and reports it. Returns the
    /// thread when one was started.
    pub fn warm_up(&self, language: Option<&str>) -> Option<std::thread::JoinHandle<()>> {
        let entry = self.entry().ok()?;
        let dir = self.root.join(entry.id);
        if !store::is_installed(&dir, entry) {
            return None;
        }
        let language = language_for(entry.engine, language).map(str::to_owned);
        if self.cache.try_lock().is_some_and(|slot| slot.as_ref().is_some_and(|l| l.is(entry.id, &language, &self.compute))) {
            return None;
        }
        let ticket = self.warm_ups.fetch_add(1, Ordering::SeqCst) + 1;
        let me = self.clone();
        std::thread::Builder::new()
            .name("voltip-asr-warm".into())
            .spawn(move || {
                let mut slot = me.cache.lock();
                if me.warm_ups.load(Ordering::SeqCst) != ticket {
                    tracing::debug!(model = entry.id, "local model warm-up superseded by a newer one");
                    return;
                }
                if let Err(e) = load_into(&mut slot, me.loader.as_ref(), entry, &dir, language, &me.compute) {
                    tracing::warn!(model = entry.id, error = %e, "local model warm-up failed; the next take loads it again");
                }
            })
            .map_err(|e| tracing::warn!(error = %e, "could not start the local model warm-up"))
            .ok()
    }

    fn entry(&self) -> Result<&'static ModelEntry, DictationError> {
        self.catalogue.iter().find(|e| e.id == self.selected).ok_or_else(|| DictationError::Asr(format!("本地模型不在目录中：{}", self.selected)))
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
        let (mut samples, sample_rate) = decode_wav(wav).map_err(DictationError::Asr)?;
        if samples.is_empty() {
            return Err(DictationError::NoSpeech);
        }
        let language = language_for(entry.engine, language).map(str::to_owned);
        let cache = self.cache.clone();
        let loader = self.loader.clone();
        let compute = self.compute.clone();
        let id = entry.id.to_owned();
        let vad = self.vad_trim.then(|| self.vad.clone());
        let text = tokio::task::spawn_blocking(move || -> Result<String, String> {
            if let Some(vad) = vad {
                // Blocking (may load the VAD model): inside the blocking task. Fail-open.
                let trim = vad.trim(&samples, sample_rate);
                match &trim.skipped {
                    Some(why) => tracing::debug!(why, "VAD trim skipped; the whole take goes in"),
                    None => tracing::debug!(cut_start = trim.cut_start, cut_end = trim.cut_end, "take trimmed before recognition"),
                }
                samples = trim.samples;
            }
            pad_to_min(&mut samples, sample_rate);
            let mut slot = cache.lock();
            let loaded = load_into(&mut slot, loader.as_ref(), entry, &dir, language, &compute)?;
            let infer_started = Instant::now();
            let text = loaded.recognizer.transcribe(sample_rate, &samples)?;
            tracing::info!(model = %id, samples = samples.len(), infer_ms = infer_started.elapsed().as_millis() as u64, "local transcription done");
            Ok(text)
        })
        .await
        .map_err(|e| DictationError::Asr(format!("本地识别任务失败：{e}")))?
        .map_err(|e| DictationError::Asr(format!("本地识别失败：{e}")))?;
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        Ok(Transcript { text: text.trim().to_owned(), latency_ms })
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
        assert!(err.contains("流式预览模型不能做整段识别"), "{err}");
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

    /// Echoes what it was loaded with; counts loads so the cache can be observed.
    struct FakeLoader {
        loads: AtomicUsize,
        fail: bool,
        computes: Mutex<Vec<Compute>>,
    }

    struct Echo {
        id: String,
        language: Option<String>,
    }

    impl Recognizer for Echo {
        fn transcribe(&mut self, sample_rate: u32, samples: &[f32]) -> Result<String, String> {
            if samples.iter().all(|s| *s == 0.0) {
                return Err("all zero".into());
            }
            Ok(format!(" {}:{}:{sample_rate}:{} ", self.id, self.language.as_deref().unwrap_or("-"), samples.len()))
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
            Ok(Box::new(Echo { id: entry.id.to_owned(), language: language.map(str::to_owned) }))
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
        let loader = Arc::new(FakeLoader { loads: AtomicUsize::new(0), fail: false, computes: Mutex::new(Vec::new()) });
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
        let loader = Arc::new(FakeLoader { loads: AtomicUsize::new(0), fail: false, computes: Mutex::new(Vec::new()) });
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
        // Two changes while a take holds the recogniser: only the newer warm-up loads.
        let take = t.cache.lock();
        let older = t.select("qwen3-asr-0.6b").warm_up(None).unwrap();
        let newer = t.select("sense-voice-small").warm_up(Some("en")).unwrap();
        drop(take);
        older.join().unwrap();
        newer.join().unwrap();
        assert_eq!(loader.loads.load(Ordering::SeqCst), 3, "the superseded warm-up did not load");
        assert_eq!(t.loaded().as_deref(), Some("sense-voice-small"));
        assert_eq!(t.transcribe(&speech, Some("en"), &[]).await.unwrap().text, "sense-voice-small:en:16000:24000");
        assert_eq!(loader.loads.load(Ordering::SeqCst), 3);
        // A loader that fails: the warm-up only logs; the take loads again and reports it.
        let broken = Arc::new(FakeLoader { loads: AtomicUsize::new(0), fail: true, computes: Mutex::new(Vec::new()) });
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
        let loader = Arc::new(FakeLoader { loads: AtomicUsize::new(0), fail: false, computes: Mutex::new(Vec::new()) });
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
        assert!(matches!(zf.transcribe(&speech, None, &[]).await.unwrap_err(), DictationError::Asr(m) if m.contains("整段识别")));
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
        let failing = LocalTranscriber::with_loader(
            dir.path(),
            CATALOGUE,
            Arc::new(FakeLoader { loads: AtomicUsize::new(0), fail: true, computes: Mutex::new(Vec::new()) }),
        )
        .select("sense-voice-small");
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
        let loader = Arc::new(FakeLoader { loads: AtomicUsize::new(0), fail: false, computes: Mutex::new(Vec::new()) });
        let take = pcm16(&silence_tone_silence(1000, 1000, 16_000).iter().map(|s| (s * 32_767.0) as i16).collect::<Vec<_>>(), 16_000);
        let vad = VadTrimmer::with_loader(dir.path(), crate::catalogue::vad_entry(), Arc::new(FakeVadLoader::energy()));
        let t = LocalTranscriber::with_loader(dir.path(), CATALOGUE, loader.clone()).select("sense-voice-small").with_vad(vad.clone());
        assert!(!t.vad_trim(), "off by default");
        assert_eq!(t.transcribe(&take, Some("zh"), &[]).await.unwrap().text, "sense-voice-small:zh:16000:48000", "off: the whole 3 s");
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
}
