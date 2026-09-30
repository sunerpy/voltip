//! Where a long take is cut (docs/dictation.md §22): the desktop's [`SegmenterFactory`].
//!
//! The Silero VAD — the auxiliary model `vad_trim` uses — follows the take as it is recorded. Once
//! a segment is [`MIN_SEGMENT`] long it ends at the next pause the detector reports; one that finds
//! no pause is cut at [`MAX_SEGMENT`], in the quietest 200 ms of its last 5 s (the core's
//! [`EnergySegmenter`]). The detector is loaded for each take on its recording thread, never on
//! the core's task. Without the model the factory answers why and fetches it in the background;
//! that take is cut by the core every 30 s, the next ones where the speech pauses.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use voltip_core::dictation::long::{EnergySegmenter, RATE};
use voltip_core::dictation::{Segmenter, SegmenterFactory};

use crate::catalogue::{ModelEntry, vad_entry};
use crate::sherpa::SherpaVadLoader;
use crate::store::{self, ModelStore};
use crate::vad::{VAD_SAMPLE_RATE_HZ, VadLoader, VoiceActivity, samples_at};

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
    root: PathBuf,
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
    pub fn new(root: impl Into<PathBuf>, store: Option<ModelStore>) -> Self {
        Self::with_loader(root, vad_entry(), Arc::new(SherpaVadLoader), store)
    }

    /// Any entry and loader (tests).
    pub fn with_loader(root: impl Into<PathBuf>, entry: &'static ModelEntry, loader: Arc<dyn VadLoader>, store: Option<ModelStore>) -> Self {
        Self { root: root.into(), entry, loader, store }
    }
}

impl SegmenterFactory for VadSegmenterFactory {
    fn create(&self) -> Result<Box<dyn Segmenter>, String> {
        let dir = self.root.join(self.entry.id);
        if !store::is_installed(&dir, self.entry) {
            if self.store.as_ref().is_some_and(ModelStore::spawn_auxiliary_fetch) {
                tracing::info!(model = self.entry.id, "a long take cuts at pauses with the VAD; downloading it in the background");
            }
            return Err(format!("VAD 模型未下载：{}", self.entry.name));
        }
        Ok(Box::new(VadSegmenter::new(Detector::Pending { loader: self.loader.clone(), entry: self.entry, dir })))
    }
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
    /// The cut of a segment that found no pause.
    forced: EnergySegmenter,
    /// Where the current segment starts, and the samples pushed so far.
    start: u64,
    pos: u64,
}

impl VadSegmenter {
    fn new(detector: Detector) -> Self {
        Self { detector, forced: EnergySegmenter::cutting_after(MAX_SEGMENT), start: 0, pos: 0 }
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
            if pause >= self.start + MIN_SEGMENT {
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
}
