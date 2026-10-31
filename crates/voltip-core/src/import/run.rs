//! One import job (docs/dictation.md §31): the file decoded, cut into segments, each segment
//! recognised and, when asked, polished, and the history entry made of them.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::watch;
use uuid::Uuid;

use super::{ImportFailure, ImportState, MediaDecoder, MediaError, SEGMENT_BOUNDS};
use crate::dictation::long::{self, EnergySegmenter, RATE, SEGMENT_ATTEMPTS};
use crate::dictation::{AsrNote, OutputMode, RefineFailure, RefineHints, Segment, Segmenter, SegmenterFactory, TakeClients, TakeKind, Via, steps};
use crate::engines::ChineseScript;
use crate::history::{HistoryEntry, ImportedFile, Outcome as Delivered};
use crate::models::CancelToken;
use crate::presets::TakePreset;
use crate::usage::{AsrCall, UsageOrigin};
use crate::vocabulary::{Vocabulary, VocabularyHits};

/// What a job runs with.
pub struct Job {
    /// The file.
    pub path: PathBuf,
    /// The name it shows and its entry keeps.
    pub name: String,
    /// Where its decoded audio goes (under the imports directory).
    pub pcm: PathBuf,
    /// The decoder.
    pub decoder: Arc<dyn MediaDecoder>,
    /// The shell's segmenter (it cuts where the speech pauses), when it has one.
    pub segmenter: Option<Arc<dyn SegmenterFactory>>,
    /// The recogniser and the polish the import chose.
    pub clients: TakeClients,
    /// Language hint.
    pub language: Option<String>,
    /// The script the recognised text is brought to (docs/dictation.md §17).
    pub script: ChineseScript,
    /// The dictionary and the rules.
    pub vocabulary: Arc<Vocabulary>,
    /// Stops the job.
    pub cancel: CancelToken,
    /// True while a dictation runs: the job waits before its next request.
    pub busy: watch::Receiver<bool>,
}

/// What a job reports while it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Progress {
    /// It moved on.
    State(ImportState),
    /// It waits for a dictation to end (`true`), or goes on (`false`).
    Paused(bool),
    /// The file's length, once known.
    Duration(u64),
}

/// The reports' receiver.
pub type Report = Arc<dyn Fn(Progress) + Send + Sync>;

/// One recognised segment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recognized {
    /// Its text in the import's script; empty for a silent segment.
    pub text: String,
    /// The request's round trip.
    pub asr_ms: u64,
    /// The model that answered, when the client said.
    pub model: Option<String>,
    /// How the on-device model got this text when it was not the usual way (docs/dictation.md
    /// §29.5: the graphics card failed and the CPU did it).
    pub note: Option<AsrNote>,
}

/// What earlier tries of a job got done. A failed job keeps it, so 重试 goes on from the first
/// segment not recognised.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Partial {
    /// Samples in the decoded file, once decoded.
    pub samples: Option<u64>,
    /// Where its segments end, once cut.
    pub cuts: Option<Vec<u64>>,
    /// The segments recognised so far, in order.
    pub recognized: Vec<Recognized>,
}

/// How a job ended.
#[derive(Debug)]
pub enum Outcome {
    /// Its entry, and how many segments' polish failed (they kept their recognised text).
    Done {
        /// The entry to keep.
        entry: Box<HistoryEntry>,
        /// Segments whose polish failed.
        polish_failed: u32,
    },
    /// It stopped; `partial` is what is done.
    Failed {
        /// Why.
        failure: ImportFailure,
        /// The decoded file and the recognised segments are kept for 重试.
        resumable: bool,
        /// What is done.
        partial: Partial,
    },
    /// Cancelled: nothing to keep.
    Cancelled,
}

/// Milliseconds since the Unix epoch.
fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Run `job` from where `partial` left it: decode the file (unless an earlier try did), cut it
/// into segments of [`SEGMENT_BOUNDS`], recognise every segment not recognised yet, each in two
/// tries at most, then — when the import polishes — polish every segment with text, and make the
/// entry. Before every request the job waits while a dictation runs. A cancel ends it at once.
pub async fn run(job: Job, mut partial: Partial, report: Report) -> Outcome {
    let Job { path, name, pcm, decoder, segmenter, clients, language, script, vocabulary, cancel, mut busy } = job;
    let samples = match partial.samples {
        Some(samples) if pcm.exists() => samples,
        _ => {
            partial = Partial::default();
            report(Progress::State(ImportState::Decoding { pct: None }));
            match decode(decoder, path, pcm.clone(), cancel.clone(), report.clone()).await {
                Ok(samples) => samples,
                Err(MediaError::Cancelled) => return Outcome::Cancelled,
                Err(e) => return Outcome::Failed { failure: e.into(), resumable: false, partial: Partial::default() },
            }
        }
    };
    partial.samples = Some(samples);
    let duration_ms = samples * 1000 / RATE;
    report(Progress::Duration(duration_ms));
    if cancel.is_cancelled() {
        return Outcome::Cancelled;
    }
    let cuts = match partial.cuts.clone() {
        Some(cuts) => cuts,
        None => match cut(pcm.clone(), samples, segmenter).await {
            Ok(cuts) => cuts,
            Err(detail) => return Outcome::Failed { failure: ImportFailure::Storage { detail }, resumable: false, partial: Partial::default() },
        },
    };
    partial.cuts = Some(cuts.clone());
    let spans: Vec<(u64, u64)> = cuts.iter().scan(0, |start, &end| Some((std::mem::replace(start, end), end))).collect();
    let total = u32::try_from(spans.len()).unwrap_or(u32::MAX);
    let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    report(Progress::State(ImportState::Recognizing { done: count(partial.recognized.len()), total }));
    let call = AsrCall::take(Some(UsageOrigin::Import));
    for &(start, end) in spans.iter().skip(partial.recognized.len()) {
        if !wait_turn(&mut busy, &cancel, &report).await {
            return Outcome::Cancelled;
        }
        let wav = match read_segment(pcm.clone(), start, end).await {
            Ok(wav) => wav,
            Err(detail) => return Outcome::Failed { failure: ImportFailure::Storage { detail }, resumable: false, partial: Partial::default() },
        };
        let mut failed = None;
        let mut recognized = None;
        for _ in 0..SEGMENT_ATTEMPTS {
            let attempt = long::recognize_segment(clients.transcriber.as_ref(), &wav, language.as_deref(), vocabulary.glossary(), script, &call);
            let answer = tokio::select! {
                answer = attempt => answer,
                () = cancel.cancelled() => return Outcome::Cancelled,
            };
            match answer {
                Ok(t) => {
                    recognized = Some(Recognized { text: t.text, asr_ms: t.latency_ms, model: t.model, note: t.backend_note });
                    break;
                }
                Err(e) => {
                    tracing::warn!(start, end, error = %e, "a segment of an imported file was not recognised");
                    failed = Some(e);
                }
            }
        }
        match recognized {
            Some(r) => partial.recognized.push(r),
            None => {
                let detail = failed.map(|e| e.to_string()).unwrap_or_default();
                return Outcome::Failed { failure: ImportFailure::Recognition { detail }, resumable: true, partial };
            }
        }
        report(Progress::State(ImportState::Recognizing { done: count(partial.recognized.len()), total }));
    }
    if partial.recognized.iter().all(|r| r.text.trim().is_empty()) {
        return Outcome::Failed { failure: ImportFailure::NoSpeech, resumable: false, partial: Partial::default() };
    }

    // The dictionary on each segment (§16.3), the polish of each one when asked, then the rules.
    let mut hits = VocabularyHits::default();
    let corrected: Vec<String> = partial
        .recognized
        .iter()
        .map(|r| {
            let step = steps::correct(&vocabulary, &r.text);
            hits.merge(VocabularyHits { corrections: step.hits, rules: Vec::new() });
            step.text
        })
        .collect();
    let mut polished: Vec<Option<String>> = vec![None; corrected.len()];
    let mut polish = Polished::default();
    if let (Some(true), Some(refiner)) = (clients.refine, clients.refiner.as_ref()) {
        let hints = RefineHints {
            glossary: vocabulary.glossary().to_vec(),
            language: language.clone(),
            preset: TakePreset::default(),
            origin: Some(UsageOrigin::Import),
            ..RefineHints::default()
        };
        let with_text = corrected.iter().filter(|t| !t.trim().is_empty()).count();
        report(Progress::State(ImportState::Polishing { done: 0, total: count(with_text) }));
        let mut done = 0;
        for (i, text) in corrected.iter().enumerate().filter(|(_, t)| !t.trim().is_empty()) {
            if !wait_turn(&mut busy, &cancel, &report).await {
                return Outcome::Cancelled;
            }
            let clean = tokio::select! {
                clean = steps::run_refine(refiner.as_ref(), text, &hints) => clean,
                () = cancel.cancelled() => return Outcome::Cancelled,
            };
            polish.ms += clean.refine_ms.unwrap_or(0);
            if clean.refined {
                polished[i] = Some(clean.text);
                polish.model = clean.refine_model.or(polish.model);
            } else {
                polish.failed += 1;
                polish.failure = clean.refine_failure.or(polish.failure);
                polish.error = clean.refine_error.or(polish.error);
            }
            done += 1;
            report(Progress::State(ImportState::Polishing { done: count(done), total: count(with_text) }));
        }
    }
    let finals: Vec<String> = corrected
        .iter()
        .zip(&polished)
        .map(|(corrected, polished)| {
            let step = steps::apply_rules(&vocabulary, polished.as_deref().unwrap_or(corrected));
            hits.merge(VocabularyHits { corrections: Vec::new(), rules: step.hits });
            step.text
        })
        .collect();
    let ms = |sample: u64| sample * 1000 / RATE;
    let mut segments = Vec::new();
    let (mut raw_pieces, mut final_pieces) = (Vec::new(), Vec::new());
    for (((&(start, end), recognized), polished), text) in spans.iter().zip(&partial.recognized).zip(&polished).zip(&finals) {
        if recognized.text.trim().is_empty() {
            continue;
        }
        segments.push(Segment { text: recognized.text.clone(), refined: polished.as_ref().map(|_| text.clone()), start_ms: ms(start), end_ms: ms(end) });
        raw_pieces.push((start, end, Some(recognized.text.clone())));
        final_pieces.push((start, end, Some(text.clone())));
    }
    let refined = polished.iter().any(Option::is_some);
    let asr_model = partial.recognized.iter().rev().find(|r| !r.text.trim().is_empty()).and_then(|r| r.model.clone()).unwrap_or(clients.asr_model);
    // Like a take's clean-up that did not run: the kind, and the service's words for a refusal.
    let unpolished = !refined && polish.failed > 0;
    let refine_failure = polish.failure.filter(|_| unpolished);
    let refine_error =
        polish.error.filter(|_| unpolished && matches!(refine_failure, Some(RefineFailure::RateLimited | RefineFailure::Quota | RefineFailure::Failed)));
    let entry = HistoryEntry {
        id: Uuid::new_v4(),
        at_ms: now_ms(),
        raw_text: long::assemble(&raw_pieces, &[]),
        text: long::assemble(&final_pieces, &[]),
        refined,
        asr_model,
        // The weightiest note of any segment, as for a long take's (docs/dictation.md §29.5).
        asr_note: partial.recognized.iter().filter_map(|r| r.note).max(),
        refine_model: refined.then(|| polish.model.unwrap_or(clients.refine_model)),
        duration_ms,
        asr_ms: partial.recognized.iter().map(|r| r.asr_ms).sum(),
        refine_ms: refined.then_some(polish.ms),
        refine_failure,
        refine_error,
        // In the history, pasted nowhere: the interface shows 导入 for an entry with a `file`.
        outcome: Delivered::Inserted { via: Via::Clipboard, check: None },
        starred: false,
        mode: OutputMode::WholeTake,
        segments: Some(segments),
        live_error: None,
        vocabulary: hits.into_option(),
        kind: TakeKind::Dictation,
        edit: None,
        app: None,
        scene: None,
        preset: refined.then(|| TakePreset::default().to_ref()),
        origin: None,
        processed: None,
        file: Some(ImportedFile { name, duration_ms }),
    };
    Outcome::Done { entry: Box::new(entry), polish_failed: polish.failed }
}

/// What the polish of a job came to.
#[derive(Default)]
struct Polished {
    failed: u32,
    ms: u64,
    model: Option<String>,
    failure: Option<RefineFailure>,
    error: Option<String>,
}

/// Wait while a dictation runs (docs/dictation.md §31: dictation always goes first), saying so;
/// `false` when the job was cancelled meanwhile.
async fn wait_turn(busy: &mut watch::Receiver<bool>, cancel: &CancelToken, report: &Report) -> bool {
    if cancel.is_cancelled() {
        return false;
    }
    if !*busy.borrow_and_update() {
        return true;
    }
    report(Progress::Paused(true));
    tokio::select! {
        // A sender that is gone is no dictation either.
        _ = busy.wait_for(|running| !*running) => {}
        () = cancel.cancelled() => return false,
    }
    report(Progress::Paused(false));
    true
}

/// Decode `path` into `pcm` on a blocking thread, reporting each new percent.
async fn decode(decoder: Arc<dyn MediaDecoder>, path: PathBuf, pcm: PathBuf, cancel: CancelToken, report: Report) -> Result<u64, MediaError> {
    let task = tokio::task::spawn_blocking(move || {
        let last = Mutex::new(None::<u8>);
        let progress = |done: u64, total: Option<u64>| {
            let pct = total.filter(|&t| t > 0).map(|t| u8::try_from((done.saturating_mul(100) / t).min(100)).unwrap_or(100));
            let mut last = last.lock();
            if pct.is_some() && pct != *last {
                *last = pct;
                report(Progress::State(ImportState::Decoding { pct }));
            }
        };
        decoder.decode_to_pcm(&path, &pcm, &progress, &cancel).map(|info| info.samples)
    });
    task.await.map_err(|e| MediaError::Write(format!("the decoding task failed: {e}")))?
}

/// [`cut_points`] on a blocking thread.
async fn cut(path: PathBuf, samples: u64, factory: Option<Arc<dyn SegmenterFactory>>) -> Result<Vec<u64>, String> {
    tokio::task::spawn_blocking(move || cut_points(&path, samples, factory.as_deref())).await.map_err(|e| format!("the cutting task failed: {e}"))?
}

/// Where the segments of the `samples` samples of the recording file `path` end, the last at the
/// end: the shell's segmenter within [`SEGMENT_BOUNDS`], or the core's own with the same bounds
/// when there is none or it cannot run (its model missing). `voltip --transcribe-file` cuts a file
/// it decoded with this too.
pub fn cut_points(path: &Path, samples: u64, factory: Option<&dyn SegmenterFactory>) -> Result<Vec<u64>, String> {
    let mut segmenter: Box<dyn Segmenter> = match factory.map(|f| f.create_with(SEGMENT_BOUNDS)) {
        Some(Ok(segmenter)) => segmenter,
        Some(Err(why)) => {
            tracing::info!(%why, "cutting the file with the core's own cutter");
            Box::new(EnergySegmenter::bounded(SEGMENT_BOUNDS))
        }
        None => Box::new(EnergySegmenter::bounded(SEGMENT_BOUNDS)),
    };
    let mut ends = read_blocks(path, samples, |block| segmenter.push(block))?;
    ends.extend(segmenter.finish());
    Ok(ends)
}

/// Feed the samples of `path` to `each` in blocks, collecting what it returns.
fn read_blocks(path: &Path, samples: u64, mut each: impl FnMut(&[f32]) -> Vec<u64>) -> Result<Vec<u64>, String> {
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = vec![0u8; 8192];
    let mut floats = Vec::with_capacity(4096);
    let (mut out, mut read) = (Vec::new(), 0u64);
    while read < samples {
        let want = usize::try_from((samples - read).min(4096) * 2).unwrap_or(8192);
        file.read_exact(&mut bytes[..want]).map_err(|e| e.to_string())?;
        floats.clear();
        floats.extend(bytes[..want].as_chunks::<2>().0.iter().map(|b| f32::from(i16::from_le_bytes(*b)) / 32_768.0));
        out.extend(each(&floats));
        read += (want / 2) as u64;
    }
    Ok(out)
}

/// One segment of the recording file as a WAV, read on a blocking thread.
async fn read_segment(path: PathBuf, start: u64, end: u64) -> Result<Vec<u8>, String> {
    tokio::task::spawn_blocking(move || long::read_segment(&path, start, end).map_err(|e| e.to_string()))
        .await
        .map_err(|e| format!("the reading task failed: {e}"))?
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use async_trait::async_trait;
    use parking_lot::Mutex;

    use super::*;
    use crate::dictation::fakes::{FakeMediaDecoder, FakeTranscriber, FixedSegmenterFactory};
    use crate::dictation::{DictationError, RefineHints, Refined, Refiner, Transcriber, Via};
    use crate::history::{ImportedFile, Outcome as Delivered};
    use crate::import::{ImportFailure, MediaError};
    use crate::presets::TakePreset;
    use crate::usage::{UsageOrigin, UsagePurpose};
    use crate::vocabulary::{EntrySource, ReplacementRule, RuleKind};

    /// Polishes `text` as 「<text>（润色）」, failing the calls numbered in `fail` (1-based).
    struct Polish {
        fail: Vec<usize>,
        calls: AtomicUsize,
        hints: Mutex<Vec<RefineHints>>,
    }

    impl Polish {
        fn failing(fail: &[usize]) -> Arc<Self> {
            Arc::new(Self { fail: fail.to_vec(), calls: AtomicUsize::new(0), hints: Mutex::new(Vec::new()) })
        }
    }

    #[async_trait]
    impl Refiner for Polish {
        async fn refine(&self, text: &str, hints: &RefineHints) -> Result<Refined, DictationError> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            self.hints.lock().push(hints.clone());
            if self.fail.contains(&n) {
                return Err(DictationError::Refine(format!("polish {n} failed")));
            }
            Ok(Refined { text: format!("{text}（润色）"), latency_ms: 7, model: "polish-model".into() })
        }

        async fn edit(&self, _selection: &str, _instruction: &str, _hints: &RefineHints) -> Result<Refined, DictationError> {
            unreachable!("an import never edits")
        }
    }

    struct Setup {
        dir: tempfile::TempDir,
        decoder: Arc<FakeMediaDecoder>,
        transcriber: Arc<FakeTranscriber>,
        polish: Option<Arc<Polish>>,
        busy: watch::Sender<bool>,
        cancel: CancelToken,
        vocabulary: Arc<Vocabulary>,
    }

    impl Setup {
        fn new(seconds: u64, transcriber: FakeTranscriber) -> Self {
            Self {
                dir: tempfile::tempdir().unwrap(),
                decoder: Arc::new(FakeMediaDecoder::new(seconds)),
                transcriber: Arc::new(transcriber),
                polish: None,
                busy: watch::channel(false).0,
                cancel: CancelToken::new(),
                vocabulary: Arc::new(Vocabulary::default()),
            }
        }

        fn pcm(&self) -> PathBuf {
            self.dir.path().join("job.pcm")
        }

        /// The job, cut every 10 s, polished when `polish` is set.
        fn job(&self) -> Job {
            let polishing = self.polish.is_some();
            Job {
                path: Path::new("/media/会议录音.m4a").to_path_buf(),
                name: "会议录音.m4a".into(),
                pcm: self.pcm(),
                decoder: self.decoder.clone(),
                segmenter: Some(Arc::new(FixedSegmenterFactory(10))),
                clients: TakeClients {
                    transcriber: self.transcriber.clone() as Arc<dyn Transcriber>,
                    refiner: self.polish.clone().map(|p| p as Arc<dyn Refiner>),
                    refine: Some(polishing),
                    asr_model: "whisper-1".into(),
                    refine_model: "chat-model".into(),
                },
                language: Some("zh".into()),
                script: ChineseScript::Simplified,
                vocabulary: self.vocabulary.clone(),
                cancel: self.cancel.clone(),
                busy: self.busy.subscribe(),
            }
        }
    }

    /// Run `job` from `partial`, collecting its reports.
    async fn run_collecting(job: Job, partial: Partial) -> (Outcome, Vec<Progress>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let report: Report = {
            let seen = seen.clone();
            Arc::new(move |p| seen.lock().push(p))
        };
        let outcome = tokio::time::timeout(Duration::from_secs(20), run(job, partial, report)).await.expect("the job ends");
        let reports = seen.lock().clone();
        (outcome, reports)
    }

    fn states(reports: &[Progress]) -> Vec<ImportState> {
        reports.iter().filter_map(|p| if let Progress::State(s) = p { Some(s.clone()) } else { None }).collect()
    }

    fn done(outcome: Outcome) -> (HistoryEntry, u32) {
        match outcome {
            Outcome::Done { entry, polish_failed } => (*entry, polish_failed),
            other => panic!("not done: {other:?}"),
        }
    }

    /// docs/dictation.md §31: a 25 s file in three segments, recognised one after the other: one
    /// entry with the segments' times, the file's name and length, recognised by the chosen model
    /// for an import, without a polish.
    #[tokio::test]
    async fn a_file_becomes_one_entry_whose_segments_keep_their_times() {
        let setup = Setup::new(25, FakeTranscriber::numbered(0, &[], Duration::ZERO));
        let (outcome, reports) = run_collecting(setup.job(), Partial::default()).await;
        let (entry, polish_failed) = done(outcome);
        assert_eq!(polish_failed, 0);
        assert_eq!(entry.raw_text, "第1段。第2段。第3段。");
        assert_eq!(entry.text, "第1段。第2段。第3段。");
        let segments = entry.segments.clone().unwrap();
        assert_eq!(
            segments.iter().map(|s| (s.text.as_str(), s.start_ms, s.end_ms)).collect::<Vec<_>>(),
            [("第1段。", 0, 10_000), ("第2段。", 10_000, 20_000), ("第3段。", 20_000, 25_000)]
        );
        assert!(segments.iter().all(|s| s.refined.is_none()));
        assert_eq!(entry.file, Some(ImportedFile { name: "会议录音.m4a".into(), duration_ms: 25_000 }));
        assert_eq!(entry.duration_ms, 25_000);
        assert_eq!((entry.refined, entry.refine_model.as_deref(), entry.preset.as_ref()), (false, None, None));
        assert_eq!(entry.asr_model, "whisper-1", "the chosen model, when the client names none");
        assert_eq!(entry.outcome, Delivered::Inserted { via: Via::Clipboard, check: None }, "in the history, pasted nowhere");
        assert_eq!(entry.origin, None);
        let recognizing: Vec<ImportState> = states(&reports).into_iter().filter(|s| matches!(s, ImportState::Recognizing { .. })).collect();
        assert_eq!(recognizing, (0..=3).map(|done| ImportState::Recognizing { done, total: 3 }).collect::<Vec<_>>());
        assert!(matches!(states(&reports).first(), Some(ImportState::Decoding { .. })), "{reports:?}");
        assert!(reports.contains(&Progress::Duration(25_000)));
        // Every request was for an import's take, in its language.
        let calls = setup.transcriber.usage_calls();
        assert_eq!(calls.len(), 3);
        assert!(calls.iter().all(|c| c.purpose == UsagePurpose::Take && c.origin == Some(UsageOrigin::Import)), "{calls:?}");
        assert_eq!(setup.transcriber.languages(), vec![Some("zh".to_owned()); 3]);
        assert_eq!(setup.transcriber.durations_ms(), [10_000, 10_000, 5_000]);
        assert!(setup.pcm().exists(), "the decoded file is the runtime's to remove");
    }

    /// docs/dictation.md §29.5: a segment the on-device model recognised on the CPU after the
    /// graphics card failed marks the imported file's entry as a take's would be, so its technical
    /// details say so; an import that ran as usual carries no note.
    #[tokio::test]
    async fn a_segment_the_cpu_took_over_from_the_graphics_card_marks_the_entry() {
        let setup = Setup::new(25, FakeTranscriber::ok("一段话").with_note(AsrNote::GpuFailedRanOnCpu));
        let (entry, _) = done(run_collecting(setup.job(), Partial::default()).await.0);
        assert_eq!(entry.asr_note, Some(AsrNote::GpuFailedRanOnCpu));
        let setup = Setup::new(25, FakeTranscriber::ok("一段话"));
        let (entry, _) = done(run_collecting(setup.job(), Partial::default()).await.0);
        assert_eq!(entry.asr_note, None);
    }

    /// docs/dictation.md §31 (D6): the polish runs segment by segment with the 校对 preset, for an
    /// import; a segment whose polish fails keeps its recognised text and is counted.
    #[tokio::test]
    async fn the_polish_runs_segment_by_segment_and_a_failed_one_keeps_its_text() {
        let mut setup = Setup::new(25, FakeTranscriber::numbered(0, &[], Duration::ZERO));
        let polish = Polish::failing(&[2]);
        setup.polish = Some(polish.clone());
        let (outcome, reports) = run_collecting(setup.job(), Partial::default()).await;
        let (entry, polish_failed) = done(outcome);
        assert_eq!(polish_failed, 1);
        assert_eq!(entry.raw_text, "第1段。第2段。第3段。");
        assert_eq!(entry.text, "第1段。（润色）第2段。第3段。（润色）");
        let refined: Vec<Option<&str>> = entry.segments.as_ref().unwrap().iter().map(|s| s.refined.as_deref()).collect();
        assert_eq!(refined, [Some("第1段。（润色）"), None, Some("第3段。（润色）")]);
        assert!(entry.refined);
        assert_eq!(entry.refine_model.as_deref(), Some("polish-model"));
        assert_eq!(entry.preset, Some(TakePreset::default().to_ref()), "校对: it tidies, it does not rewrite");
        assert_eq!((entry.refine_failure, entry.refine_error.as_deref()), (None, None), "most of it was polished");
        let hints = polish.hints.lock().clone();
        assert_eq!(hints.len(), 3);
        assert!(hints.iter().all(|h| h.origin == Some(UsageOrigin::Import) && h.preset == TakePreset::default() && h.language.as_deref() == Some("zh")));
        let polishing: Vec<ImportState> = states(&reports).into_iter().filter(|s| matches!(s, ImportState::Polishing { .. })).collect();
        assert_eq!(polishing, (0..=3).map(|done| ImportState::Polishing { done, total: 3 }).collect::<Vec<_>>());

        // Every polish failing: the text is the recognised one, and the entry says why.
        let mut setup = Setup::new(25, FakeTranscriber::numbered(0, &[], Duration::ZERO));
        setup.polish = Some(Polish::failing(&[1, 2, 3]));
        let (entry, polish_failed) = done(run_collecting(setup.job(), Partial::default()).await.0);
        assert_eq!((polish_failed, entry.refined, entry.text.as_str()), (3, false, "第1段。第2段。第3段。"));
        assert_eq!(entry.refine_failure, Some(crate::dictation::RefineFailure::Failed));
        assert!(entry.refine_error.as_deref().is_some_and(|e| e.contains("polish 3 failed")), "{entry:?}");
    }

    /// A job told not to polish never asks the refiner it was handed.
    #[tokio::test]
    async fn without_a_polish_the_refiner_is_never_asked() {
        let setup = Setup::new(12, FakeTranscriber::numbered(0, &[], Duration::ZERO));
        let polish = Polish::failing(&[]);
        let mut job = setup.job();
        job.clients.refiner = Some(polish.clone());
        job.clients.refine = Some(false);
        let (entry, _) = done(run_collecting(job, Partial::default()).await.0);
        assert_eq!(polish.calls.load(Ordering::SeqCst), 0);
        assert!(!entry.refined);
    }

    /// A segment that fails twice stops the job, which keeps what it did: 重试 goes on from that
    /// segment, without decoding the file again.
    #[tokio::test]
    async fn a_segment_that_fails_twice_stops_the_job_and_a_retry_goes_on_from_it() {
        let setup = Setup::new(25, FakeTranscriber::numbered(0, &[2, 3], Duration::ZERO));
        let (outcome, _) = run_collecting(setup.job(), Partial::default()).await;
        let Outcome::Failed { failure, resumable, partial } = outcome else { panic!("{outcome:?}") };
        assert!(matches!(&failure, ImportFailure::Recognition { detail } if detail.contains("fake failure of call 3")), "{failure:?}");
        assert!(resumable);
        assert_eq!(partial.samples, Some(25 * 16_000));
        assert_eq!(partial.cuts.as_deref(), Some(&[160_000, 320_000, 400_000][..]));
        assert_eq!(partial.recognized.len(), 1, "the first segment is done");
        assert!(setup.pcm().exists(), "kept for the retry");
        assert_eq!(setup.transcriber.calls(), 3);

        let (outcome, reports) = run_collecting(setup.job(), partial).await;
        let (entry, _) = done(outcome);
        assert_eq!(entry.raw_text, "第1段。第4段。第5段。", "segments 2 and 3 are the 4th and 5th calls");
        assert_eq!(setup.transcriber.calls(), 5);
        assert_eq!(setup.decoder.decodes(), 1, "not decoded again");
        assert!(states(&reports).contains(&ImportState::Recognizing { done: 1, total: 3 }));
        assert!(!states(&reports).iter().any(|s| matches!(s, ImportState::Decoding { .. })), "{reports:?}");
    }

    /// docs/dictation.md §31: dictation always goes first. While one runs the job waits before its
    /// next request, says it waits, and goes on when it is over.
    #[tokio::test]
    async fn the_job_waits_for_a_dictation_before_its_next_segment() {
        let setup = Setup::new(12, FakeTranscriber::numbered(0, &[], Duration::ZERO));
        setup.busy.send_replace(true);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let report: Report = {
            let seen = seen.clone();
            Arc::new(move |p| seen.lock().push(p))
        };
        let task = tokio::spawn(run(setup.job(), Partial::default(), report));
        tokio::time::timeout(Duration::from_secs(10), async {
            while !seen.lock().contains(&Progress::Paused(true)) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the job says it waits");
        assert_eq!(setup.transcriber.calls(), 0, "nothing asked while the dictation runs");
        setup.busy.send_replace(false);
        let (entry, _) = done(tokio::time::timeout(Duration::from_secs(10), task).await.expect("the job ends").unwrap());
        assert_eq!(entry.raw_text, "第1段。第2段。");
        assert!(seen.lock().contains(&Progress::Paused(false)));
    }

    /// A file the decoder refuses is the job's failure, with nothing kept.
    #[tokio::test]
    async fn a_file_that_does_not_decode_fails_the_job() {
        let mut setup = Setup::new(10, FakeTranscriber::ok("x"));
        setup.decoder = Arc::new(FakeMediaDecoder::failing(MediaError::UnsupportedCodec("AC-3".into())));
        let (outcome, _) = run_collecting(setup.job(), Partial::default()).await;
        let Outcome::Failed { failure, resumable, partial } = outcome else { panic!("{outcome:?}") };
        assert_eq!((failure, resumable, partial), (ImportFailure::UnsupportedCodec { codec: "AC-3".into() }, false, Partial::default()));
        assert_eq!(setup.transcriber.calls(), 0);
    }

    /// Nothing recognised in the whole file is no entry.
    #[tokio::test]
    async fn a_file_where_nothing_is_said_is_no_speech() {
        let setup = Setup::new(12, FakeTranscriber::ok("  "));
        let (outcome, _) = run_collecting(setup.job(), Partial::default()).await;
        assert!(matches!(outcome, Outcome::Failed { failure: ImportFailure::NoSpeech, resumable: false, .. }), "{outcome:?}");
    }

    /// A cancel ends the job at once, whatever it waits for.
    #[tokio::test]
    async fn a_cancel_ends_the_job() {
        let mut setup = Setup::new(12, FakeTranscriber::ok("x"));
        setup.decoder = Arc::new(FakeMediaDecoder::held(12));
        let cancel = setup.cancel.clone();
        let (job, report): (Job, Report) = (setup.job(), Arc::new(|_| {}));
        let task = tokio::spawn(run(job, Partial::default(), report));
        cancel.cancel();
        let outcome = tokio::time::timeout(Duration::from_secs(10), task).await.expect("the job ends").unwrap();
        assert!(matches!(outcome, Outcome::Cancelled), "{outcome:?}");
        assert!(!setup.pcm().exists());
        // Waiting for a dictation, too.
        let setup = Setup::new(12, FakeTranscriber::ok("x"));
        setup.busy.send_replace(true);
        let task = tokio::spawn(run(setup.job(), Partial::default(), Arc::new(|_| {})));
        setup.cancel.cancel();
        let outcome = tokio::time::timeout(Duration::from_secs(10), task).await.expect("the job ends").unwrap();
        assert!(matches!(outcome, Outcome::Cancelled), "{outcome:?}");
        assert_eq!(setup.transcriber.calls(), 0);
    }

    /// The dictionary corrects each segment and the rules run on what it says, as in a dictation:
    /// the subtitles' recognised text stays as it was heard.
    #[tokio::test]
    async fn the_dictionary_and_the_rules_work_on_every_segment() {
        let mut setup = Setup::new(12, FakeTranscriber::numbered(0, &[], Duration::ZERO));
        let rule = ReplacementRule {
            id: uuid::Uuid::new_v4(),
            name: "段落".into(),
            kind: RuleKind::Literal,
            pattern: "段".into(),
            replacement: "节".into(),
            case_sensitive: true,
            enabled: true,
            created_at_ms: 1,
            updated_at_ms: 1,
            source: EntrySource::Manual,
            pending: false,
        };
        setup.vocabulary = Arc::new(Vocabulary::compile(&[], &[rule]));
        let (entry, _) = done(run_collecting(setup.job(), Partial::default()).await.0);
        assert_eq!(entry.text, "第1节。第2节。");
        assert_eq!(entry.raw_text, "第1段。第2段。");
        assert_eq!(entry.segments.unwrap()[0].text, "第1段。");
        assert_eq!(entry.vocabulary.map(|v| v.rules.iter().map(|h| h.count).sum::<u32>()), Some(2));
    }
}
