//! Where a long take is cut (docs/dictation.md §22): the desktop's [`SegmenterFactory`].
//!
//! The Silero VAD — the auxiliary model `vad_trim` uses — follows the take as it is recorded. Once
//! a segment is [`MIN_SEGMENT`] long it ends at the next pause the detector reports; one that finds
//! no pause is cut at [`MAX_SEGMENT`], in the quietest 200 ms of its last 5 s (the core's
//! [`EnergySegmenter`]). The detector is loaded for each take on its recording thread, never on
//! the core's task. Without the model the factory answers why and fetches it in the background;
//! that take is cut by the core every 30 s, the next ones where the speech pauses. An imported file
//! asks for shorter segments ([`SegmenterFactory::create_with`], docs/dictation.md §31): the same
//! cuts within its bounds.
//!
//! The same rules cut an input longer than a model's `max_piece` into pieces inside the local
//! transcriber ([`pieces`], docs/dictation.md §29), with `max_piece` for the longest segment.

use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use voltip_core::dictation::long::{EnergySegmenter, RATE};
use voltip_core::dictation::{SegmentBounds, Segmenter, SegmenterFactory};

use crate::catalogue::{ModelEntry, vad_entry};
use crate::root::ModelsRoot;
use crate::sherpa::SherpaVadLoader;
use crate::store::{self, ModelStore};
use crate::vad::{VAD_SAMPLE_RATE_HZ, VadLoader, VadTrimmer, VoiceActivity, samples_at};

/// A segment ends at the first pause once it is this long (20 s)…
pub const MIN_SEGMENT: u64 = 20 * RATE;
/// …and is cut wherever it is quietest once it reaches this length (45 s).
pub const MAX_SEGMENT: u64 = 45 * RATE;
/// How far into the pause after a span the cut goes: the detector ends a span only after at least
/// [`LONG_SPEECH_MIN_SILENCE`](crate::vad::LONG_SPEECH_MIN_SILENCE) of silence (the shortest pause
/// it ends an over-long utterance at), so 50 ms is always inside it.
pub const PAUSE_MARGIN: Duration = Duration::from_millis(50);

/// Makes a [`VadSegmenter`] for each long take.
#[derive(Clone)]
pub struct VadSegmenterFactory {
    root: ModelsRoot,
    entry: &'static ModelEntry,
    loader: Arc<dyn VadLoader>,
    store: Option<ModelStore>,
}

impl std::fmt::Debug for VadSegmenterFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VadSegmenterFactory").field("root", &self.root).field("model", &self.entry.id).finish_non_exhaustive()
    }
}

impl VadSegmenterFactory {
    /// sherpa-onnx over the real catalogue's VAD entry under `root` (`<app data dir>/models`);
    /// `store` fetches the model when a take finds it missing.
    pub fn new(root: impl Into<ModelsRoot>, store: Option<ModelStore>) -> Self {
        Self::with_loader(root, vad_entry(), Arc::new(SherpaVadLoader), store)
    }

    /// Any entry and loader (tests).
    pub fn with_loader(root: impl Into<ModelsRoot>, entry: &'static ModelEntry, loader: Arc<dyn VadLoader>, store: Option<ModelStore>) -> Self {
        Self { root: root.into(), entry, loader, store }
    }
}

/// A long take's bounds: [`MIN_SEGMENT`] to [`MAX_SEGMENT`].
const LONG_TAKE: SegmentBounds = SegmentBounds { min: MIN_SEGMENT, max: MAX_SEGMENT };

impl SegmenterFactory for VadSegmenterFactory {
    fn create(&self) -> Result<Box<dyn Segmenter>, String> {
        self.create_with(LONG_TAKE)
    }

    /// The same detector with other bounds: an imported file's shorter segments (docs/dictation.md
    /// §31) end at the first pause past `bounds.min`.
    fn create_with(&self, bounds: SegmentBounds) -> Result<Box<dyn Segmenter>, String> {
        let dir = self.root.join(self.entry.id);
        if !store::is_installed(&dir, self.entry) {
            if self.store.as_ref().is_some_and(ModelStore::spawn_auxiliary_fetch) {
                tracing::info!(model = self.entry.id, "a long take cuts at pauses with the VAD; downloading it in the background");
            }
            return Err(format!("VAD 模型未下载：{}", self.entry.name));
        }
        Ok(Box::new(VadSegmenter::with_bounds(Detector::Pending { loader: self.loader.clone(), entry: self.entry, dir }, bounds)))
    }
}

/// Where `samples` (mono at `rate`) is cut so that no piece is longer than `max_piece`
/// (docs/dictation.md §29): the whole input when it fits (or when `max_piece` is zero, no limit);
/// otherwise a piece ends at the first pause the VAD of `vad`'s library finds once it is 20 s long
/// (half of `max_piece` when that is shorter), and where it is quietest once it reaches
/// `max_piece`. Without the VAD model only the quietest-moment cuts are made. Blocking (loads the
/// detector).
pub fn pieces(samples: &[f32], rate: u32, max_piece: Duration, vad: &VadTrimmer) -> Vec<Range<usize>> {
    let max = samples_at(max_piece, rate);
    if max == 0 || samples.len() <= max || rate == 0 {
        return std::iter::once(0..samples.len()).collect();
    }
    // The cuts are found at the detector's rate and taken back to the input's.
    let pcm16k = match crate::gguf::resample_to_model_rate(samples, rate) {
        Ok(pcm) => pcm,
        Err(e) => {
            tracing::warn!(error = %e, "could not resample for the piece cuts; cutting at fixed lengths");
            return (0..samples.len()).step_by(max).map(|start| start..(start + max).min(samples.len())).collect();
        }
    };
    let max16k = samples_at(max_piece, VAD_SAMPLE_RATE_HZ) as u64;
    let detector = if vad.is_installed() { Detector::Pending { loader: vad.loader(), entry: vad.entry(), dir: vad.dir() } } else { Detector::Off };
    let mut segmenter = VadSegmenter::with_bounds(detector, SegmentBounds { min: MIN_SEGMENT.min(max16k / 2), max: max16k });
    let mut cuts: Vec<u64> = Vec::new();
    for chunk in pcm16k.chunks(4096) {
        cuts.extend(segmenter.push(chunk));
    }
    let scale = |at: u64| usize::try_from(u128::from(at) * u128::from(rate) / u128::from(VAD_SAMPLE_RATE_HZ)).unwrap_or(usize::MAX).min(samples.len());
    let mut bounds: Vec<usize> = std::iter::once(0).chain(cuts.into_iter().map(scale)).chain(std::iter::once(samples.len())).collect();
    bounds.dedup();
    bounds.windows(2).filter(|w| w[0] < w[1]).map(|w| w[0]..w[1]).collect()
}

/// The take's detector: loaded on the first samples.
enum Detector {
    Pending {
        loader: Arc<dyn VadLoader>,
        entry: &'static ModelEntry,
        dir: PathBuf,
    },
    Ready(Box<dyn VoiceActivity>),
    /// It did not load, or it failed: the forced cuts alone from then on.
    Off,
}

/// Cuts one long take where the speech pauses; see the module documentation.
pub struct VadSegmenter {
    detector: Detector,
    /// A pause ends a segment once it is this long.
    min: u64,
    /// The cut of a segment that found no pause.
    forced: EnergySegmenter,
    /// Where the current segment starts, and the samples pushed so far.
    start: u64,
    pos: u64,
}

impl VadSegmenter {
    #[cfg(test)]
    fn new(detector: Detector) -> Self {
        Self::with_bounds(detector, LONG_TAKE)
    }

    /// Segments of `bounds`: a pause ends one past `bounds.min`, and one without a pause is cut by
    /// `bounds.max` (for a long take's bounds the same as `EnergySegmenter::cutting_after(45 s)`).
    fn with_bounds(detector: Detector, bounds: SegmentBounds) -> Self {
        Self { detector, min: bounds.min, forced: EnergySegmenter::bounded(bounds), start: 0, pos: 0 }
    }

    /// The pauses the detector reports in `samples`, as positions a cut may go at.
    fn pauses(&mut self, samples: &[f32]) -> Vec<u64> {
        if let Detector::Pending { loader, entry, dir } = &self.detector {
            self.detector = match loader.load(entry, dir) {
                Ok(vad) => Detector::Ready(vad),
                Err(e) => {
                    tracing::warn!(error = %e, "the VAD did not load; this long take is cut every 45 s");
                    Detector::Off
                }
            };
        }
        let Detector::Ready(vad) = &mut self.detector else { return Vec::new() };
        match vad.feed(samples) {
            Ok(spans) => {
                let margin = samples_at(PAUSE_MARGIN, VAD_SAMPLE_RATE_HZ) as u64;
                spans.iter().map(|span| (span.end as u64 + margin).min(self.pos)).collect()
            }
            Err(e) => {
                tracing::warn!(error = %e, "the VAD failed; the rest of this long take is cut every 45 s");
                self.detector = Detector::Off;
                Vec::new()
            }
        }
    }
}

impl Segmenter for VadSegmenter {
    fn push(&mut self, samples: &[f32]) -> Vec<u64> {
        let mut cuts = Vec::new();
        for cut in self.forced.push(samples) {
            cuts.push(cut);
            self.start = cut;
        }
        self.pos += samples.len() as u64;
        for pause in self.pauses(samples) {
            if pause >= self.start + self.min {
                cuts.push(pause);
                self.start = pause;
                self.forced.cut_at(pause);
            }
        }
        cuts
    }

    fn finish(&mut self) -> Option<u64> {
        (self.pos > self.start).then_some(self.pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vad::MIN_SILENCE;
    use crate::vad::tests::{BrokenVad, EnergyVad, FakeVadLoader, install_vad};
    use std::path::Path;
    use std::sync::atomic::Ordering;

    /// `seconds` of speech-like sound (a 440 Hz tone at −6 dBFS) with a pause of `pause_ms` after
    /// every `every` seconds.
    fn talk(seconds: u64, every: u64, pause_ms: u64) -> Vec<f32> {
        let period = (every * RATE) as usize;
        let pause = (pause_ms * RATE / 1000) as usize;
        (0..(seconds * RATE) as usize)
            .map(|i| if i % period >= period - pause { 0.0 } else { (i as f32 / RATE as f32 * 440.0 * std::f32::consts::TAU).sin() * 0.5 })
            .collect()
    }

    fn cut_all(segmenter: &mut dyn Segmenter, signal: &[f32]) -> Vec<u64> {
        let mut cuts = Vec::new();
        for chunk in signal.chunks(4096) {
            cuts.extend(segmenter.push(chunk));
        }
        cuts
    }

    fn factory(root: &Path, loader: Arc<FakeVadLoader>) -> VadSegmenterFactory {
        VadSegmenterFactory::with_loader(root, vad_entry(), loader, None)
    }

    /// A pause every 8 s: a segment ends at the first one past 20 s — 24 s, then every 24 s —
    /// just inside the pause.
    #[test]
    fn a_segment_ends_at_the_first_pause_after_20_seconds() {
        let dir = tempfile::tempdir().unwrap();
        install_vad(dir.path());
        let loader = Arc::new(FakeVadLoader::energy());
        let mut segmenter = factory(dir.path(), loader.clone()).create().unwrap();
        assert_eq!(loader.loads.load(Ordering::SeqCst), 0, "loaded on the first samples, not by the factory");
        let cuts = cut_all(segmenter.as_mut(), &talk(100, 8, 600));
        assert_eq!(loader.loads.load(Ordering::SeqCst), 1);
        assert_eq!(cuts.len(), 4, "{cuts:?}");
        let margin = samples_at(PAUSE_MARGIN, VAD_SAMPLE_RATE_HZ) as u64;
        for (i, cut) in cuts.iter().enumerate() {
            // The pause before every third 8 s mark starts 600 ms before it; the fake detector
            // ends the span at the window it went quiet in (512-sample windows).
            let pause_start = (i as u64 + 1) * 24 * RATE - 600 * RATE / 1000;
            assert!((pause_start..pause_start + 512 + margin).contains(cut), "cut {i} at {} s", *cut as f64 / RATE as f64);
        }
        assert_eq!(segmenter.finish(), Some(100 * RATE));
    }

    /// No pause at all: 45 s segments, cut where the last 5 s is quietest. The detector reports no
    /// span for a sound that never pauses, so the cuts are the forced ones.
    #[test]
    fn without_a_pause_the_segment_is_cut_at_45_seconds() {
        let dir = tempfile::tempdir().unwrap();
        install_vad(dir.path());
        let mut segmenter = factory(dir.path(), Arc::new(FakeVadLoader::energy())).create().unwrap();
        let cuts = cut_all(segmenter.as_mut(), &talk(100, 1_000, 0));
        assert_eq!(cuts.len(), 2, "{cuts:?}");
        assert!((40 * RATE..=45 * RATE).contains(&cuts[0]), "{cuts:?}");
        assert!((cuts[0] + 40 * RATE..=cuts[0] + 45 * RATE).contains(&cuts[1]), "{cuts:?}");
        assert_eq!(segmenter.finish(), Some(100 * RATE));
        // A detector that never reports cuts the same way.
        let mut forced = VadSegmenter::new(Detector::Off);
        assert_eq!(cut_all(&mut forced, &talk(100, 1_000, 0)), cuts);
        assert_eq!(forced.finish(), Some(100 * RATE));
    }

    /// Regression (goal review, 2026-09-30): the fake detector split a span of 20 s of speech at
    /// exactly 20 s, and this test's predecessor asserted such cuts inside the sound, although
    /// sherpa-onnx only waits for a shorter pause then. Speech with a 150 ms breath every 7 s
    /// (too short to end a span before 20 s) is cut at the first breath past 20 s, inside it.
    #[test]
    fn regression_speech_past_20_seconds_is_cut_at_its_next_short_pause_not_inside_it() {
        let mut segmenter = VadSegmenter::new(Detector::Ready(Box::new(EnergyVad::default())));
        let signal = talk(60, 7, 150);
        let cuts = cut_all(&mut segmenter, &signal);
        assert_eq!(cuts.len(), 2, "{cuts:?}");
        for (cut, breath_ends_at) in cuts.iter().zip([21, 42]) {
            let at = usize::try_from(*cut).unwrap();
            assert!(signal[at - 1] == 0.0 && signal[at] == 0.0, "cut at {} s is inside the sound", *cut as f64 / RATE as f64);
            assert!((breath_ends_at * RATE - 150 * RATE / 1000..breath_ends_at * RATE).contains(cut), "cut at {} s", *cut as f64 / RATE as f64);
        }
        assert_eq!(segmenter.finish(), Some(60 * RATE));
    }

    /// docs/dictation.md §31: an imported file's 3–15 s segments. A pause every 4 s ends each one
    /// at the first pause past 3 s; sound without a pause is cut by 15 s.
    #[test]
    fn bounds_from_an_import_replace_the_long_takes() {
        let dir = tempfile::tempdir().unwrap();
        install_vad(dir.path());
        let bounds = SegmentBounds { min: 3 * RATE, max: 15 * RATE };
        let lengths = |cuts: &[u64]| -> Vec<u64> { cuts.iter().scan(0, |start, &cut| Some(cut - std::mem::replace(start, cut))).collect() };
        let mut segmenter = factory(dir.path(), Arc::new(FakeVadLoader::energy())).create_with(bounds).unwrap();
        let cuts = cut_all(segmenter.as_mut(), &talk(60, 4, 600));
        assert!(cuts.len() >= 12, "a cut at nearly every pause: {cuts:?}");
        assert!(lengths(&cuts).iter().all(|l| (bounds.min..=bounds.max).contains(l)), "{:?}", lengths(&cuts));
        assert_eq!(segmenter.finish(), Some(60 * RATE));

        let mut segmenter = factory(dir.path(), Arc::new(FakeVadLoader::energy())).create_with(bounds).unwrap();
        let cuts = cut_all(segmenter.as_mut(), &talk(60, 1_000, 0));
        assert!(cuts.len() >= 4, "{cuts:?}");
        assert!(lengths(&cuts).iter().all(|l| (10 * RATE..=bounds.max).contains(l)), "the forced cut: {:?}", lengths(&cuts));

        let missing = tempfile::tempdir().unwrap();
        let err = factory(missing.path(), Arc::new(FakeVadLoader::energy())).create_with(bounds).err().unwrap();
        assert!(err.contains("未下载"), "{err}");
    }

    /// A pause before 20 s is no cut.
    #[test]
    fn early_pauses_do_not_cut() {
        let mut segmenter = VadSegmenter::new(Detector::Ready(Box::new(EnergyVad::default())));
        // Pauses before 5, 10, 15 and 20 s, then sound until 50 s: those pauses are too early, and
        // the first cut comes only once the sound has run on.
        let mut signal = talk(20, 5, 700);
        signal.extend(talk(30, 1_000, 0));
        let cuts = cut_all(&mut segmenter, &signal);
        assert!(cuts.iter().all(|&c| c >= MIN_SEGMENT), "{cuts:?}");
        assert!(!cuts.is_empty());
        let mut empty = VadSegmenter::new(Detector::Off);
        assert_eq!(empty.finish(), None);
        assert_eq!(MIN_SILENCE, Duration::from_millis(450), "the pause the detector waits for");
    }

    /// Missing model: the factory says why (the core falls back); a detector that does not load or
    /// fails turns into the forced cuts alone.
    #[test]
    fn a_missing_or_broken_detector_leaves_the_forced_cuts() {
        let dir = tempfile::tempdir().unwrap();
        let err = factory(dir.path(), Arc::new(FakeVadLoader::energy())).create().err().unwrap();
        assert!(err.contains("未下载"), "{err}");
        install_vad(dir.path());
        let unloadable = Arc::new(FakeVadLoader { fail_load: true, ..FakeVadLoader::energy() });
        let mut segmenter = factory(dir.path(), unloadable).create().unwrap();
        assert_eq!(cut_all(segmenter.as_mut(), &talk(50, 8, 600)).len(), 1, "the forced cut only");
        let broken = Arc::new(FakeVadLoader { broken: true, ..FakeVadLoader::energy() });
        let mut segmenter = factory(dir.path(), broken).create().unwrap();
        assert_eq!(cut_all(segmenter.as_mut(), &talk(50, 8, 600)).len(), 1);
        let mut failing = VadSegmenter::new(Detector::Ready(Box::new(BrokenVad)));
        assert!(failing.push(&[0.1; 512]).is_empty());
        assert!(matches!(failing.detector, Detector::Off));
        assert!(format!("{:?}", factory(dir.path(), Arc::new(FakeVadLoader::energy()))).contains("silero-vad"));
    }

    /// docs/dictation.md §29: an input longer than a model's `max_piece` is cut into pieces no
    /// longer than it, which together cover the input once: at the first pause once a piece is
    /// half of `max_piece` long (20 s at most), inside the pause; where it is quietest without the
    /// VAD model. An input that fits, or a model without a limit, is one piece.
    #[test]
    fn an_input_longer_than_max_piece_is_cut_into_pieces_that_fit() {
        let dir = tempfile::tempdir().unwrap();
        let vad = VadTrimmer::with_loader(dir.path(), vad_entry(), Arc::new(FakeVadLoader::energy()));
        let max = Duration::from_secs(30);
        let signal = talk(70, 8, 600);
        let check = |pieces: &[Range<usize>], len: usize, rate: usize| {
            assert!(pieces.len() >= 3, "{pieces:?}");
            assert_eq!((pieces[0].start, pieces[pieces.len() - 1].end), (0, len));
            assert!(pieces.windows(2).all(|w| w[0].end == w[1].start), "{pieces:?}");
            assert!(pieces.iter().all(|p| p.len() <= 30 * rate), "{pieces:?}");
        };
        let rate = RATE as usize;
        let whole = |len: usize| std::iter::once(0..len).collect::<Vec<_>>();
        assert_eq!(pieces(&signal[..30 * rate], RATE as u32, max, &vad), whole(30 * rate), "fits");
        assert_eq!(pieces(&signal, RATE as u32, Duration::ZERO, &vad), whole(signal.len()), "no limit");
        let forced = pieces(&signal, RATE as u32, max, &vad);
        check(&forced, signal.len(), rate);
        install_vad(dir.path());
        let at_pauses = pieces(&signal, RATE as u32, max, &vad);
        check(&at_pauses, signal.len(), rate);
        assert_ne!(forced, at_pauses);
        for piece in &at_pauses[..at_pauses.len() - 1] {
            assert!(piece.len() >= 15 * rate, "{at_pauses:?}");
            assert!(signal[piece.end - 1] == 0.0 && signal[piece.end] == 0.0, "the cut at {:.2} s is inside the sound", piece.end as f64 / RATE as f64);
        }
        // Another rate: the cuts are found at 16 kHz and taken back.
        let at_48k: Vec<f32> = signal.iter().flat_map(|&s| [s, s, s]).collect();
        check(&pieces(&at_48k, 48_000, max, &vad), at_48k.len(), 48_000);
    }
}
