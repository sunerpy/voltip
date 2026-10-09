//! VAD trimming before a local whole-take transcription (docs/dictation.md §12 `vad_trim`).
//!
//! A take usually starts with the hotkey press and ends with its release: a few hundred
//! milliseconds of room noise on either side that the recogniser has to wade through. With
//! `vad_trim` on, the Silero VAD (sherpa-onnx `VoiceActivityDetector`, catalogue entry
//! `silero-vad`) finds where the speech is and the take is cut to the first speech start minus
//! [`PADDING`] and the last speech end plus [`PADDING`]. Only the ends are cut, never a pause in
//! the middle. The detector sits behind [`VadLoader`] / [`VoiceActivity`] (the real one in
//! `sherpa.rs`, a fake in the tests), is loaded on first use and kept.
//!
//! Fail-open by design: a missing model, a load or inference error, or a take with no speech at
//! all hands the original audio to the recogniser unchanged — trimming is an optimisation, never
//! a gate.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::catalogue::{ModelEntry, vad_entry};
use crate::gguf::resample_to_model_rate;
use crate::sherpa::SherpaVadLoader;
use crate::store;

/// Speech probability above which a window counts as speech.
pub const THRESHOLD: f32 = 0.3;
/// Shortest run of speech windows that counts as speech.
pub const MIN_SPEECH: Duration = Duration::from_millis(60);
/// Silence shorter than this does not end a speech segment.
pub const MIN_SILENCE: Duration = Duration::from_millis(450);
/// Audio kept before the first and after the last detected speech.
pub const PADDING: Duration = Duration::from_millis(450);
/// The rate the detector works at (the model is trained for it).
pub const VAD_SAMPLE_RATE_HZ: u32 = 16_000;
/// Samples per detector window at [`VAD_SAMPLE_RATE_HZ`] (Silero v4).
pub const WINDOW_SAMPLES: usize = 512;
/// Speech this long makes the detector end the span at a shorter pause: past it, sherpa-onnx
/// (1.13.8, `voice-activity-detector.cc`) waits for [`LONG_SPEECH_MIN_SILENCE`] of audio under a
/// 0.9 threshold instead of [`MIN_SILENCE`] under [`THRESHOLD`]. It does not cut inside speech.
pub const MAX_SPEECH: Duration = Duration::from_secs(20);
/// The pause that ends a span past [`MAX_SPEECH`] (sherpa-onnx's own value, not configurable).
pub const LONG_SPEECH_MIN_SILENCE: Duration = Duration::from_millis(100);

/// One stretch of speech, as sample offsets at [`VAD_SAMPLE_RATE_HZ`] into the audio fed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpeechSpan {
    /// First sample.
    pub start: usize,
    /// One past the last sample.
    pub end: usize,
}

/// A loaded detector: finds the speech in one take.
pub trait VoiceActivity: Send {
    /// Speech spans in `pcm16k` (mono, `-1.0..=1.0`, 16 kHz), in order; empty when none.
    fn spans(&mut self, pcm16k: &[f32]) -> Result<Vec<SpeechSpan>, String>;

    /// A stream instead of a take (docs/dictation.md §22, where a long take pauses): the next
    /// samples of it; returns the spans that ended by now, in order, as offsets from the stream's
    /// start. A span is reported once the silence after it is [`MIN_SILENCE`] long, or
    /// [`LONG_SPEECH_MIN_SILENCE`] once the span is [`MAX_SPEECH`] long. Use a detector of its
    /// own: [`VoiceActivity::spans`] starts over.
    fn feed(&mut self, pcm16k: &[f32]) -> Result<Vec<SpeechSpan>, String>;
}

/// Builds a [`VoiceActivity`] from an installed model directory. The real one wraps sherpa-onnx;
/// tests plug in a fake so no model file is needed.
pub trait VadLoader: Send + Sync {
    /// Load `entry` from `dir`.
    fn load(&self, entry: &ModelEntry, dir: &Path) -> Result<Box<dyn VoiceActivity>, String>;
}

/// What [`VadTrimmer::trim`] decided.
#[derive(Clone, Debug, PartialEq)]
pub struct Trim {
    /// The audio to recognise: the cut, or the original when trimming did not apply.
    pub samples: Vec<f32>,
    /// Samples dropped at the start (at the input rate).
    pub cut_start: usize,
    /// Samples dropped at the end (at the input rate).
    pub cut_end: usize,
    /// Why the original was kept, when it was (`None` when the cut applied).
    pub skipped: Option<String>,
}

impl Trim {
    /// The cut applied (something was actually removed or the ends were speech already).
    pub fn applied(&self) -> bool {
        self.skipped.is_none()
    }

    fn untouched(samples: &[f32], why: impl Into<String>) -> Self {
        Self { samples: samples.to_vec(), cut_start: 0, cut_end: 0, skipped: Some(why.into()) }
    }
}

/// The trimmer over the model library: knows where the VAD model lives, loads it once, cuts.
#[derive(Clone)]
pub struct VadTrimmer {
    root: PathBuf,
    entry: &'static ModelEntry,
    loader: Arc<dyn VadLoader>,
    cache: Arc<Mutex<Option<Box<dyn VoiceActivity>>>>,
}

impl std::fmt::Debug for VadTrimmer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VadTrimmer").field("root", &self.root).field("model", &self.entry.id).field("loaded", &self.is_loaded()).finish()
    }
}

impl VadTrimmer {
    /// sherpa-onnx over the real catalogue's VAD entry under `root` (`<app data dir>/models`).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self::with_loader(root, vad_entry(), Arc::new(SherpaVadLoader))
    }

    /// Any entry and loader (tests).
    pub fn with_loader(root: impl Into<PathBuf>, entry: &'static ModelEntry, loader: Arc<dyn VadLoader>) -> Self {
        Self { root: root.into(), entry, loader, cache: Arc::new(Mutex::new(None)) }
    }

    /// The catalogue entry this trims with.
    pub fn entry(&self) -> &'static ModelEntry {
        self.entry
    }

    /// Whether the model files are on disk and verified.
    pub fn is_installed(&self) -> bool {
        store::is_installed(&self.root.join(self.entry.id), self.entry)
    }

    /// Whether the detector is in memory.
    pub fn is_loaded(&self) -> bool {
        self.cache.lock().is_some()
    }

    /// Drop the detector (the memory) until the next trim.
    pub fn unload(&self) {
        *self.cache.lock() = None;
    }

    /// Cut `samples` (mono at `sample_rate`) to the speech it contains plus [`PADDING`] on each
    /// side. Blocking (loads the model on first use, runs inference). Never fails: whatever goes
    /// wrong, the original audio comes back with the reason in `Trim::skipped`.
    pub fn trim(&self, samples: &[f32], sample_rate: u32) -> Trim {
        if samples.is_empty() || sample_rate == 0 {
            return Trim::untouched(samples, "nothing to trim");
        }
        let dir = self.root.join(self.entry.id);
        if !store::is_installed(&dir, self.entry) {
            return Trim::untouched(samples, format!("VAD 模型未下载：{}", self.entry.name));
        }
        let pcm16k = match resample_to_model_rate(samples, sample_rate) {
            Ok(pcm) => pcm,
            Err(e) => return Trim::untouched(samples, format!("resample for VAD failed: {e}")),
        };
        let mut slot = self.cache.lock();
        if slot.is_none() {
            let started = Instant::now();
            match self.loader.load(self.entry, &dir) {
                Ok(vad) => {
                    tracing::info!(model = self.entry.id, load_ms = started.elapsed().as_millis() as u64, "VAD model loaded");
                    *slot = Some(vad);
                }
                Err(e) => return Trim::untouched(samples, format!("VAD 模型加载失败：{e}")),
            }
        }
        let Some(vad) = slot.as_mut() else { return Trim::untouched(samples, "VAD vanished") };
        let started = Instant::now();
        let spans = match vad.spans(&pcm16k) {
            Ok(spans) => spans,
            Err(e) => {
                // A detector that failed once is not trusted again until reloaded.
                *slot = None;
                return Trim::untouched(samples, format!("VAD failed: {e}"));
            }
        };
        drop(slot);
        let Some(range) = speech_range(&spans, pcm16k.len()) else { return Trim::untouched(samples, "VAD found no speech") };
        let trim = cut(samples, sample_rate, range);
        tracing::info!(
            model = self.entry.id,
            spans = spans.len(),
            cut_start_ms = trim.cut_start as u64 * 1000 / u64::from(sample_rate),
            cut_end_ms = trim.cut_end as u64 * 1000 / u64::from(sample_rate),
            vad_ms = started.elapsed().as_millis() as u64,
            "take trimmed to its speech"
        );
        trim
    }
}

/// From the first speech start minus [`PADDING`] to the last speech end plus [`PADDING`], as
/// sample offsets at [`VAD_SAMPLE_RATE_HZ`] clamped to `len`; `None` without speech.
pub fn speech_range(spans: &[SpeechSpan], len: usize) -> Option<(usize, usize)> {
    let first = spans.iter().map(|s| s.start).min()?;
    let last = spans.iter().map(|s| s.end).max()?;
    let pad = samples_at(PADDING, VAD_SAMPLE_RATE_HZ);
    Some((first.saturating_sub(pad).min(len), last.saturating_add(pad).min(len)))
}

/// Apply a 16 kHz `range` to `samples` at `sample_rate`.
fn cut(samples: &[f32], sample_rate: u32, (start16k, end16k): (usize, usize)) -> Trim {
    let scale = |n: usize| (n as u128 * u128::from(sample_rate) / u128::from(VAD_SAMPLE_RATE_HZ)) as usize;
    let start = scale(start16k).min(samples.len());
    let end = scale(end16k).clamp(start, samples.len());
    Trim { samples: samples[start..end].to_vec(), cut_start: start, cut_end: samples.len() - end, skipped: None }
}

/// `d` in samples at `rate`.
pub fn samples_at(d: Duration, rate: u32) -> usize {
    usize::try_from(d.as_millis() * u128::from(rate) / 1000).unwrap_or(usize::MAX)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// An energy detector: a window is speech when its RMS is above `-40 dBFS`; consecutive speech
    /// windows form a span. Good enough to find a tone between two silences. Fed as a stream, a
    /// span is reported once [`MIN_SILENCE`] of quiet windows followed it, or
    /// [`LONG_SPEECH_MIN_SILENCE`] once it is [`MAX_SPEECH`] long, as sherpa-onnx does; a sound
    /// without any pause is never reported.
    #[derive(Default)]
    pub struct EnergyVad {
        /// Samples fed so far (whole windows) and the part of a window still waiting.
        fed: usize,
        pending: Vec<f32>,
        /// The span being heard, and the quiet samples since its last speech window.
        open: Option<SpeechSpan>,
        quiet: usize,
    }

    impl EnergyVad {
        fn window(&mut self, window: &[f32], spans: &mut Vec<SpeechSpan>) {
            let (start, end) = (self.fed, self.fed + window.len());
            self.fed = end;
            if loud(window) {
                self.quiet = 0;
                let span = self.open.get_or_insert(SpeechSpan { start, end });
                span.end = end;
            } else if let Some(span) = self.open {
                self.quiet += window.len();
                let long = span.end - span.start >= samples_at(MAX_SPEECH, VAD_SAMPLE_RATE_HZ);
                let pause = if long { LONG_SPEECH_MIN_SILENCE } else { MIN_SILENCE };
                if self.quiet >= samples_at(pause, VAD_SAMPLE_RATE_HZ) {
                    spans.extend(self.open.take());
                }
            }
        }
    }

    fn loud(window: &[f32]) -> bool {
        (window.iter().map(|s| s * s).sum::<f32>() / window.len() as f32).sqrt() >= 0.01
    }

    impl VoiceActivity for EnergyVad {
        fn feed(&mut self, pcm16k: &[f32]) -> Result<Vec<SpeechSpan>, String> {
            let mut spans = Vec::new();
            self.pending.extend_from_slice(pcm16k);
            let whole = self.pending.len() / WINDOW_SAMPLES * WINDOW_SAMPLES;
            let windows: Vec<f32> = self.pending.drain(..whole).collect();
            for window in windows.chunks(WINDOW_SAMPLES) {
                self.window(window, &mut spans);
            }
            Ok(spans)
        }

        fn spans(&mut self, pcm16k: &[f32]) -> Result<Vec<SpeechSpan>, String> {
            let mut spans: Vec<SpeechSpan> = Vec::new();
            for (i, window) in pcm16k.chunks(WINDOW_SAMPLES).enumerate() {
                if !loud(window) {
                    continue;
                }
                let (start, end) = (i * WINDOW_SAMPLES, i * WINDOW_SAMPLES + window.len());
                match spans.last_mut() {
                    Some(last) if last.end == start => last.end = end,
                    _ => spans.push(SpeechSpan { start, end }),
                }
            }
            Ok(spans)
        }
    }

    /// A detector that fails on every call.
    pub struct BrokenVad;

    impl VoiceActivity for BrokenVad {
        fn spans(&mut self, _pcm16k: &[f32]) -> Result<Vec<SpeechSpan>, String> {
            Err("onnxruntime: bad input".into())
        }

        fn feed(&mut self, _pcm16k: &[f32]) -> Result<Vec<SpeechSpan>, String> {
            Err("onnxruntime: bad input".into())
        }
    }

    /// Hands out `EnergyVad` (or `BrokenVad`, or fails to load); counts loads.
    pub struct FakeVadLoader {
        pub loads: AtomicUsize,
        pub broken: bool,
        pub fail_load: bool,
    }

    impl FakeVadLoader {
        pub fn energy() -> Self {
            Self { loads: AtomicUsize::new(0), broken: false, fail_load: false }
        }
    }

    impl VadLoader for FakeVadLoader {
        fn load(&self, entry: &ModelEntry, dir: &Path) -> Result<Box<dyn VoiceActivity>, String> {
            self.loads.fetch_add(1, Ordering::SeqCst);
            assert!(dir.ends_with(entry.id));
            if self.fail_load {
                return Err("onnxruntime: bad model".into());
            }
            Ok(if self.broken { Box::new(BrokenVad) } else { Box::new(EnergyVad::default()) })
        }
    }

    /// Pretend the VAD entry is installed under `root` (manifest + right-sized file).
    pub fn install_vad(root: &Path) {
        let e = vad_entry();
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

    /// `silence_ms` of nothing, `tone_ms` of a 440 Hz tone at −6 dBFS, `silence_ms` of nothing.
    pub fn silence_tone_silence(silence_ms: u64, tone_ms: u64, rate: u32) -> Vec<f32> {
        let n = |ms: u64| (ms * u64::from(rate) / 1000) as usize;
        let mut out = vec![0.0_f32; n(silence_ms)];
        out.extend((0..n(tone_ms)).map(|i| (i as f32 / rate as f32 * 440.0 * std::f32::consts::TAU).sin() * 0.5));
        out.extend(std::iter::repeat_n(0.0_f32, n(silence_ms)));
        out
    }

    /// The documented parameters (docs/dictation.md §12).
    #[test]
    fn parameters_follow_the_contract() {
        assert_eq!(THRESHOLD, 0.3);
        assert_eq!(MIN_SPEECH, Duration::from_millis(60));
        assert_eq!(MIN_SILENCE, Duration::from_millis(450));
        assert_eq!(PADDING, Duration::from_millis(450));
        assert_eq!(VAD_SAMPLE_RATE_HZ, 16_000);
        assert_eq!(WINDOW_SAMPLES, 512);
        assert_eq!(MAX_SPEECH, Duration::from_secs(20));
        assert_eq!(LONG_SPEECH_MIN_SILENCE, Duration::from_millis(100));
        assert_eq!(samples_at(PADDING, 16_000), 7200);
        assert_eq!(samples_at(Duration::from_secs(1), 48_000), 48_000);
        assert_eq!(speech_range(&[], 100), None, "no speech, no range");
        assert_eq!(speech_range(&[SpeechSpan { start: 16_000, end: 32_000 }], 48_000), Some((8800, 39_200)), "450 ms of padding each side");
        assert_eq!(speech_range(&[SpeechSpan { start: 1000, end: 2000 }], 2500), Some((0, 2500)), "padding is clamped to the take");
        assert_eq!(
            speech_range(&[SpeechSpan { start: 30_000, end: 31_000 }, SpeechSpan { start: 16_000, end: 20_000 }], 48_000),
            Some((8800, 38_200)),
            "the first start and the last end, whatever the order"
        );
    }

    /// Silence + tone + silence: the cut keeps the tone plus 450 ms of context on each side, so a
    /// 3 s take becomes 1.9 s; the model loads once; other sample rates are resampled for the
    /// detector and cut at their own rate; a take that is speech end to end is left as is.
    #[test]
    fn trims_silence_around_a_tone_and_keeps_the_padding() {
        let dir = tempfile::tempdir().unwrap();
        let loader = Arc::new(FakeVadLoader::energy());
        let trimmer = VadTrimmer::with_loader(dir.path(), vad_entry(), loader.clone());
        assert_eq!(trimmer.entry().id, "silero-vad");
        assert!(!trimmer.is_installed() && !trimmer.is_loaded());
        let take = silence_tone_silence(1000, 1000, 16_000);
        assert_eq!(take.len(), 48_000);
        // Not installed: the original comes back, nothing loads.
        let t = trimmer.trim(&take, 16_000);
        assert_eq!(t.samples, take);
        assert_eq!(t.skipped.as_deref(), Some("VAD 模型未下载：语音活动检测"));
        assert!(!t.applied() && loader.loads.load(Ordering::SeqCst) == 0);
        install_vad(dir.path());
        assert!(trimmer.is_installed());
        let t = trimmer.trim(&take, 16_000);
        assert!(t.applied(), "{t:?}");
        // 1 s of tone + 2 × 450 ms = 1.9 s, give or take the detector's 512-sample window on each side.
        assert!((30_400..=30_400 + 2 * WINDOW_SAMPLES).contains(&t.samples.len()), "got {} ms", t.samples.len() / 16);
        assert!((8800 - WINDOW_SAMPLES..=8800).contains(&t.cut_start), "≈ 550 ms cut from the start: {}", t.cut_start);
        assert!((8800 - WINDOW_SAMPLES..=8800).contains(&t.cut_end), "≈ 550 ms cut from the end: {}", t.cut_end);
        assert!(t.samples[..7000].iter().all(|s| *s == 0.0), "the padding before the tone is the original silence");
        assert!(t.samples[7200..7200 + 2 * WINDOW_SAMPLES].iter().any(|s| s.abs() > 0.1), "the tone starts right after the padding");
        assert_eq!(loader.loads.load(Ordering::SeqCst), 1);
        assert!(trimmer.is_loaded());
        assert!(format!("{trimmer:?}").contains("loaded: true"));
        // Same detector on the next take: no reload.
        trimmer.trim(&take, 16_000);
        assert_eq!(loader.loads.load(Ordering::SeqCst), 1);
        // 48 kHz: detected at 16 kHz, cut at 48 kHz.
        let take48 = silence_tone_silence(1000, 1000, 48_000);
        let t = trimmer.trim(&take48, 48_000);
        assert!(t.applied());
        let ms = t.samples.len() as f64 / 48.0;
        assert!((1880.0..=1970.0).contains(&ms), "≈ 1.9 s at 48 kHz after resampling for the detector (one window of slack per side): {ms} ms");
        // Speech from the first to the last sample: nothing to cut, still "applied" (the ends are speech).
        let tone_only = silence_tone_silence(0, 1000, 16_000);
        let t = trimmer.trim(&tone_only, 16_000);
        assert!(t.applied());
        assert_eq!(t.samples, tone_only);
        assert_eq!((t.cut_start, t.cut_end), (0, 0));
        // Speech that starts within the padding keeps the whole start.
        let early = silence_tone_silence(200, 1000, 16_000);
        let t = trimmer.trim(&early, 16_000);
        assert_eq!(t.cut_start, 0, "200 ms of silence is inside the 450 ms padding");
        assert_eq!(t.cut_end, 0);
        trimmer.unload();
        assert!(!trimmer.is_loaded());
        assert_eq!(trimmer.trim(&[], 16_000).skipped.as_deref(), Some("nothing to trim"));
        assert_eq!(trimmer.trim(&take, 0).skipped.as_deref(), Some("nothing to trim"));
    }

    /// Fail-open (docs/dictation.md §12): a detector that fails, a model that does not load, and a
    /// take with no speech at all each hand the original audio back — and say why.
    #[test]
    fn a_failing_vad_or_no_speech_returns_the_original_audio() {
        let dir = tempfile::tempdir().unwrap();
        install_vad(dir.path());
        let take = silence_tone_silence(1000, 1000, 16_000);
        let broken = Arc::new(FakeVadLoader { broken: true, ..FakeVadLoader::energy() });
        let trimmer = VadTrimmer::with_loader(dir.path(), vad_entry(), broken.clone());
        let t = trimmer.trim(&take, 16_000);
        assert_eq!(t.samples, take, "the original audio goes to the recogniser");
        assert_eq!(t.skipped.as_deref(), Some("VAD failed: onnxruntime: bad input"));
        assert!(!trimmer.is_loaded(), "a failed detector is dropped for a fresh load next time");
        trimmer.trim(&take, 16_000);
        assert_eq!(broken.loads.load(Ordering::SeqCst), 2);
        let unloadable = Arc::new(FakeVadLoader { fail_load: true, ..FakeVadLoader::energy() });
        let trimmer = VadTrimmer::with_loader(dir.path(), vad_entry(), unloadable);
        let t = trimmer.trim(&take, 16_000);
        assert_eq!(t.samples, take);
        assert_eq!(t.skipped.as_deref(), Some("VAD 模型加载失败：onnxruntime: bad model"));
        let trimmer = VadTrimmer::with_loader(dir.path(), vad_entry(), Arc::new(FakeVadLoader::energy()));
        let silence = vec![0.0_f32; 32_000];
        let t = trimmer.trim(&silence, 16_000);
        assert_eq!(t.samples, silence, "no speech → untouched (the core's own silence rule decides)");
        assert_eq!(t.skipped.as_deref(), Some("VAD found no speech"));
    }
}
