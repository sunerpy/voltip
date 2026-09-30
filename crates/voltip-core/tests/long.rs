#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Long takes through the real core task (docs/dictation.md §22), on the dictation fakes: a take
//! that may run past two minutes is recorded to a file under `recordings/`, the core's own
//! segmenter cuts it about every 30 s, the segments are recognised in order while the take goes
//! on, and the text is put together at the stop. What was not recognised is marked with its time,
//! the AI clean-up and the paste depend on the text's length, and the file is removed whatever the
//! outcome.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use voltip_core::dictation::fakes::{
    FAKE_STREAMING_MODEL_ID, FakeAudio, FakeInjector, FakeModels, FakeRefiner, FakeStreaming, FakeTranscriber, fake_engines, ports_with,
};
use voltip_core::dictation::long::{self, REFINE_SKIPPED, TOO_LONG_TO_PASTE};
use voltip_core::dictation::{ClipboardCode, SegmentProgress, Via};
use voltip_core::{
    AppCore, CoreCommand, CoreConfig, CoreEvent, CoreHandle, DictationPhase, DictationStatus, EngineSettings, FailureCode, HistoryEntry, Outcome, OutputMode,
    RecordingSettings, RecordingSource, Segment, Settings, SettingsStore,
};
use voltip_identity::MemorySecretStore;

/// How long one step may take: a debug build writes, cuts and reads back two hours of the fake
/// signal in one of them.
const STEP: Duration = Duration::from_secs(90);

/// 16-bit samples at 16 kHz: the size of `seconds` of a recording file.
fn file_bytes(seconds: u64) -> u64 {
    seconds * u64::from(voltip_core::dictation::PCM_SAMPLE_RATE_HZ) * 2
}

struct Node {
    handle: CoreHandle,
    events: mpsc::Receiver<CoreEvent>,
    transcriber: Arc<FakeTranscriber>,
    refiner: Arc<FakeRefiner>,
    injector: Arc<FakeInjector>,
    dir: tempfile::TempDir,
}

/// The core on `audio`, recording up to `max_minutes`, with a refiner that answers 「润色后的全文。」.
fn start(audio: FakeAudio, transcriber: FakeTranscriber, injector: FakeInjector, max_minutes: u16) -> Node {
    start_in(tempfile::tempdir().unwrap(), audio, transcriber, injector, max_minutes)
}

fn start_in(dir: tempfile::TempDir, audio: FakeAudio, transcriber: FakeTranscriber, injector: FakeInjector, max_minutes: u16) -> Node {
    start_with(dir, audio, transcriber, injector, max_minutes, None, FakeRefiner::ok("润色后的全文。"))
}

/// [`start`] in a streaming mode (docs/dictation.md §12), its model installed and `streaming` the
/// recogniser; no clean-up.
fn start_streaming(audio: FakeAudio, transcriber: FakeTranscriber, streaming: FakeStreaming, mode: OutputMode) -> Node {
    start_with(tempfile::tempdir().unwrap(), audio, transcriber, FakeInjector::paste(), 10, Some((streaming, mode)), FakeRefiner::ok("润色后的全文。"))
}

fn start_with(
    dir: tempfile::TempDir,
    audio: FakeAudio,
    transcriber: FakeTranscriber,
    injector: FakeInjector,
    max_minutes: u16,
    streaming: Option<(FakeStreaming, OutputMode)>,
    refiner: FakeRefiner,
) -> Node {
    let recording = RecordingSettings { max_minutes, ..RecordingSettings::default() };
    let engines = match &streaming {
        Some((_, mode)) => EngineSettings { output_mode: *mode, live_preview: true, refine_enabled: false, ..fake_engines() },
        None => fake_engines(),
    };
    SettingsStore::new(dir.path()).save(&Settings { relay_enabled: false, engines, recording, ..Settings::default() }).unwrap();
    let transcriber = Arc::new(transcriber);
    let refiner = Arc::new(refiner);
    let injector = Arc::new(injector);
    let mut ports = ports_with(Arc::new(audio), transcriber.clone(), Some(refiner.clone()), injector.clone());
    if let Some((streaming, _)) = streaming {
        ports.models = Some(Arc::new(FakeModels::new(1).with_installed(FAKE_STREAMING_MODEL_ID)));
        ports.streaming = Some(Arc::new(streaming));
    }
    let mut config = CoreConfig::new(dir.path().to_path_buf());
    config.default_device_name = "Long Test".into();
    config.direct_enabled = false;
    let (handle, events) = AppCore::start_with(config, Arc::new(MemorySecretStore::new()), ports).unwrap();
    Node { handle, events, transcriber, refiner, injector, dir }
}

async fn wait<T>(node: &mut Node, mut pick: impl FnMut(&CoreEvent) -> Option<T>) -> T {
    let deadline = Instant::now() + STEP;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let ev = tokio::time::timeout(left, node.events.recv()).await.expect("event in time").expect("core alive");
        if let Some(v) = pick(&ev) {
            return v;
        }
    }
}

/// Every dictation status until one `pred` holds for (returned last).
async fn statuses_until(node: &mut Node, mut pred: impl FnMut(&DictationStatus) -> bool) -> Vec<DictationStatus> {
    let mut seen = Vec::new();
    loop {
        let status = wait(node, |e| if let CoreEvent::Dictation(s) = e { Some(s.clone()) } else { None }).await;
        let done = pred(&status);
        seen.push(status);
        if done {
            return seen;
        }
    }
}

fn listening_with(status: &DictationStatus, segments: SegmentProgress) -> bool {
    matches!(status.phase, DictationPhase::Listening { .. }) && status.segments == Some(segments)
}

async fn ready(node: &mut Node) {
    wait(node, |e| matches!(e, CoreEvent::Ready { .. }).then_some(())).await;
}

/// Stop the take and return its terminal phase and every status on the way.
async fn stop(node: &mut Node) -> (DictationPhase, Vec<DictationStatus>) {
    node.handle.send(CoreCommand::DictationStop).await.unwrap();
    let seen = statuses_until(node, |s| s.phase.is_terminal()).await;
    (seen.last().unwrap().phase.clone(), seen)
}

async fn newest_entry(node: &mut Node) -> HistoryEntry {
    wait(node, |e| if let CoreEvent::History { recent, .. } = e { recent.first().cloned() } else { None }).await
}

/// The recording files in `dir` right now.
fn recordings(dir: &Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(long::recordings_dir(dir)).map(|entries| entries.flatten().map(|e| e.path()).collect()).unwrap_or_default()
}

/// Wait until the take's recording file holds `seconds`: the device is open and the fake's stream
/// has been read (a stop before that is a stop while the device opens — a take of nothing).
async fn wait_recorded(dir: &Path, seconds: u64) {
    let deadline = Instant::now() + STEP;
    loop {
        let files = recordings(dir);
        if files.len() == 1 && std::fs::metadata(&files[0]).is_ok_and(|m| m.len() == file_bytes(seconds)) {
            return;
        }
        assert!(Instant::now() < deadline, "no recording of {seconds} s: {files:?}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// The recording thread reports its end after the take's: wait for the file to go.
async fn wait_no_recordings(dir: &Path) {
    let deadline = Instant::now() + STEP;
    while !recordings(dir).is_empty() {
        assert!(Instant::now() < deadline, "the recording file stayed: {:?}", recordings(dir));
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// 「第1段。」 … 「第n段。」, as the numbered fake answers calls `from..=to`.
fn numbered(from: usize, to: usize) -> String {
    (from..=to).map(|n| format!("第{n}段。")).collect()
}

/// Ten minutes: the segments past the first two are recognised while the take goes on and the
/// status counts them; every one is read back from the file (none longer than 30 s); at the stop
/// the text is the segments in order, cleaned up (it is short), pasted, and kept with its segments
/// in the history; the recording file goes.
#[tokio::test]
async fn a_long_take_is_recognised_in_segments_while_it_records() {
    let mut node = start(FakeAudio::speech().long(600), FakeTranscriber::numbered(0, &[], Duration::ZERO), FakeInjector::paste(), 10);
    ready(&mut node).await;
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    // Cut at 29.9 s, 59.9 s … 599.9 s (the pause before every 30 s mark): 20 segments.
    let seen = statuses_until(&mut node, |s| listening_with(s, SegmentProgress { done: 20, total: 20 })).await;
    let listening = seen.last().unwrap();
    assert_eq!(listening.source, Some(RecordingSource::Microphone));
    let counts: Vec<SegmentProgress> = seen.iter().filter_map(|s| s.segments).collect();
    assert!(counts.iter().all(|c| c.total > 4), "nothing is recognised within the first two minutes: {counts:?}");
    assert_eq!(node.transcriber.calls(), 20);
    let durations = node.transcriber.durations_ms();
    assert_eq!(durations[0], 29_900);
    assert!(durations[1..].iter().all(|&d| d == 30_000), "{durations:?}");
    let files = recordings(node.dir.path());
    assert_eq!(files.len(), 1, "{files:?}");
    assert!(files[0].file_name().unwrap().to_string_lossy().starts_with("take-"));
    assert_eq!(std::fs::metadata(&files[0]).unwrap().len(), file_bytes(600));

    let (done, seen) = stop(&mut node).await;
    let expected = numbered(1, 20);
    match &done {
        DictationPhase::Done { text, raw_text, refined, duration_ms, via, segments, .. } => {
            assert_eq!(raw_text, &expected);
            assert_eq!(text, "润色后的全文。");
            assert!(refined);
            assert_eq!(*duration_ms, 600_000);
            assert_eq!(*via, Via::Paste);
            assert_eq!(segments.as_ref().map(Vec::len), Some(20));
        }
        other => panic!("{other:?}"),
    }
    assert!(
        seen.iter().any(|s| matches!(s.phase, DictationPhase::Processing { .. }) && s.segments == Some(SegmentProgress { done: 21, total: 21 })),
        "the last, silent piece is counted too: {seen:?}"
    );
    assert!(seen.last().unwrap().segments.is_none(), "the count ends with the take");
    assert_eq!(node.transcriber.calls(), 20, "the silent last 100 ms were not uploaded");
    assert_eq!(node.refiner.inputs()[0].0, expected, "the clean-up gets the whole text");
    assert_eq!(node.injector.injected(), vec!["润色后的全文。".to_owned()]);
    let entry = newest_entry(&mut node).await;
    assert_eq!(entry.duration_ms, 600_000);
    assert_eq!(entry.raw_text, expected);
    let segments = entry.segments.unwrap();
    assert_eq!(segments.len(), 20, "the silent piece is left out");
    assert_eq!(segments[0], Segment { text: "第1段。".into(), start_ms: 0, end_ms: 29_900 });
    assert_eq!(segments[19], Segment { text: "第20段。".into(), start_ms: 569_900, end_ms: 599_900 });
    wait_no_recordings(node.dir.path()).await;
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// A take of two minutes or less is a whole take as before (docs/dictation.md §22): its file is
/// dropped, the in-memory recording goes to the transcriber once, and no count is ever shown. One
/// second more and the text comes from the file's five segments.
#[tokio::test]
async fn the_two_minute_boundary_keeps_short_takes_as_they_were() {
    let mut node = start(FakeAudio::speech().long(120), FakeTranscriber::numbered(0, &[], Duration::ZERO), FakeInjector::paste(), 10);
    ready(&mut node).await;
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    let mut seen = statuses_until(&mut node, |s| matches!(s.phase, DictationPhase::Listening { .. })).await;
    wait_recorded(node.dir.path(), 120).await;
    let (done, rest) = stop(&mut node).await;
    seen.extend(rest);
    assert!(seen.iter().all(|s| s.segments.is_none()), "{seen:?}");
    assert!(matches!(&done, DictationPhase::Done { raw_text, segments: None, .. } if raw_text == "第1段。"), "{done:?}");
    assert_eq!(node.transcriber.durations_ms(), vec![1_500], "the fake's in-memory take, once");
    assert_eq!(newest_entry(&mut node).await.segments, None);
    wait_no_recordings(node.dir.path()).await;
    node.handle.send(CoreCommand::Shutdown).await.unwrap();

    let mut node = start(FakeAudio::speech().long(121), FakeTranscriber::numbered(0, &[], Duration::ZERO), FakeInjector::paste(), 10);
    ready(&mut node).await;
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    statuses_until(&mut node, |s| matches!(s.phase, DictationPhase::Listening { .. })).await;
    wait_recorded(node.dir.path(), 121).await;
    assert_eq!(node.transcriber.calls(), 0, "nothing is recognised within the first two minutes");
    let (done, _) = stop(&mut node).await;
    assert!(matches!(&done, DictationPhase::Done { raw_text, duration_ms: 121_000, .. } if *raw_text == numbered(1, 5)), "{done:?}");
    assert_eq!(node.transcriber.durations_ms(), vec![29_900, 30_000, 30_000, 30_000, 1_100]);
    let entry = newest_entry(&mut node).await;
    assert_eq!(entry.segments.map(|s| s.len()), Some(5));
    wait_no_recordings(node.dir.path()).await;
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// A segment that fails twice is marked with its time and the take goes on; so are the samples the
/// shell's buffer lost. A take nobody could recognise fails like a whole take and keeps nothing.
#[tokio::test]
async fn what_was_not_recognised_is_marked_and_the_rest_goes_on() {
    // Segment 2 fails on calls 2 and 3; 3 s go missing at 1:40, inside segment 4.
    let audio = FakeAudio::speech().long(200).with_gap(100, 3);
    let mut node = start(audio, FakeTranscriber::numbered(0, &[2, 3], Duration::ZERO), FakeInjector::paste(), 10);
    ready(&mut node).await;
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    statuses_until(&mut node, |s| listening_with(s, SegmentProgress { done: 6, total: 6 })).await;
    assert_eq!(node.transcriber.calls(), 7, "segment 2 was tried twice");
    let (done, _) = stop(&mut node).await;
    let expected = "第1段。[未识别 00:00:29–00:00:59] 第4段。第5段。[未识别 00:01:40–00:01:43] 第6段。第7段。第8段。";
    assert!(matches!(&done, DictationPhase::Done { raw_text, .. } if raw_text == expected), "{done:?}");
    let segments = newest_entry(&mut node).await.segments.unwrap();
    assert_eq!(segments.len(), 7);
    assert_eq!(segments[1], Segment { text: "[未识别 00:00:29–00:00:59]".into(), start_ms: 29_900, end_ms: 59_900 });
    assert_eq!(segments[3].text, "第5段。", "the lost samples are marked in the text only");
    wait_no_recordings(node.dir.path()).await;
    node.handle.send(CoreCommand::Shutdown).await.unwrap();

    let mut node = start(FakeAudio::speech().long(150), FakeTranscriber::err("503 service unavailable"), FakeInjector::paste(), 10);
    ready(&mut node).await;
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    statuses_until(&mut node, |s| listening_with(s, SegmentProgress { done: 5, total: 5 })).await;
    let (done, _) = stop(&mut node).await;
    assert!(matches!(&done, DictationPhase::Failed { code: FailureCode::Asr, message, text: None } if message.contains("503")), "{done:?}");
    assert_eq!(node.transcriber.calls(), 10, "each of the five segments with speech was tried twice");
    assert!(node.injector.injected().is_empty(), "placeholders alone are never pasted");
    wait_no_recordings(node.dir.path()).await;
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// Cancelling a long take stops its recognition and removes its file; a file an earlier run left
/// behind (it ended while recording) is removed when the core starts.
#[tokio::test]
async fn cancelled_and_leftover_recordings_are_removed() {
    let slow = FakeTranscriber::numbered(0, &[], Duration::from_millis(200));
    let mut node = start(FakeAudio::speech().long(300), slow, FakeInjector::paste(), 10);
    ready(&mut node).await;
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    statuses_until(&mut node, |s| matches!(s.phase, DictationPhase::Listening { .. }) && s.segments.is_some()).await;
    assert_eq!(recordings(node.dir.path()).len(), 1);
    node.handle.send(CoreCommand::DictationCancel).await.unwrap();
    let seen = statuses_until(&mut node, |s| s.phase.is_terminal()).await;
    assert_eq!(seen.last().unwrap().phase, DictationPhase::CANCELLED);
    wait_no_recordings(node.dir.path()).await;
    let calls = node.transcriber.calls();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(node.transcriber.calls(), calls, "no segment is recognised after the cancel");
    assert!(node.injector.injected().is_empty());
    node.handle.send(CoreCommand::Shutdown).await.unwrap();

    let dir = tempfile::tempdir().unwrap();
    let left = long::recordings_dir(dir.path());
    std::fs::create_dir_all(&left).unwrap();
    std::fs::write(left.join("take-1-1.pcm"), [0u8; 64]).unwrap();
    std::fs::write(left.join("notes.txt"), "not a recording").unwrap();
    let mut node = start_in(dir, FakeAudio::speech(), FakeTranscriber::ok("x"), FakeInjector::paste(), 10);
    ready(&mut node).await;
    assert_eq!(recordings(node.dir.path()), vec![left.join("notes.txt")], "only recordings are removed");
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// A stop while the segments are still with the recogniser: `Processing` counts them as they come
/// back, and the text keeps their order.
#[tokio::test]
async fn a_stop_while_segments_are_recognised_waits_for_all_of_them() {
    let slow = FakeTranscriber::numbered(0, &[], Duration::from_millis(80));
    let mut node = start(FakeAudio::speech().long(600), slow, FakeInjector::paste(), 10);
    ready(&mut node).await;
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    statuses_until(&mut node, |s| matches!(s.phase, DictationPhase::Listening { .. }) && s.segments.is_some_and(|c| c.done >= 1)).await;
    let (done, seen) = stop(&mut node).await;
    assert!(matches!(&done, DictationPhase::Done { raw_text, .. } if *raw_text == numbered(1, 20)), "{done:?}");
    let counts: Vec<SegmentProgress> = seen.iter().filter(|s| matches!(s.phase, DictationPhase::Processing { .. })).filter_map(|s| s.segments).collect();
    assert!(counts.len() >= 2 && counts.windows(2).all(|w| w[0].done <= w[1].done), "{counts:?}");
    assert!(counts.first().is_some_and(|c| c.done < c.total), "the stop came before the last segment: {counts:?}");
    assert_eq!(counts.last(), Some(&SegmentProgress { done: 21, total: 21 }));
    wait_no_recordings(node.dir.path()).await;
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// Two hours of the fake signal, as fast as it can be read (docs/dictation.md §22): 240 segments
/// of at most 30 s, read back one at a time — the take never sits in memory — and one text.
#[tokio::test]
async fn two_hours_make_240_segments() {
    let mut node = start(FakeAudio::speech().long(7_200), FakeTranscriber::numbered(0, &[], Duration::ZERO), FakeInjector::paste(), 120);
    ready(&mut node).await;
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    statuses_until(&mut node, |s| listening_with(s, SegmentProgress { done: 240, total: 240 })).await;
    let files = recordings(node.dir.path());
    assert_eq!(std::fs::metadata(&files[0]).unwrap().len(), file_bytes(7_200));
    let (done, _) = stop(&mut node).await;
    assert!(matches!(&done, DictationPhase::Done { raw_text, duration_ms: 7_200_000, .. } if *raw_text == numbered(1, 240)), "{done:?}");
    let durations = node.transcriber.durations_ms();
    assert_eq!(durations.len(), 240);
    assert!(durations.iter().all(|&d| d <= 30_000), "{:?}", durations.iter().max());
    assert_eq!(newest_entry(&mut node).await.segments.map(|s| s.len()), Some(240));
    wait_no_recordings(node.dir.path()).await;
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// Past 2000 characters the text is not cleaned up (the history's presets take it in parts); past
/// 5000 it is left on the clipboard instead of pasted, and the history says why.
#[tokio::test]
async fn long_texts_skip_the_clean_up_and_very_long_ones_the_paste() {
    // 20 segments of 「第n段」 + 100 字 + 「。」: about 2100 characters.
    let mut node = start(FakeAudio::speech().long(600), FakeTranscriber::numbered(100, &[], Duration::ZERO), FakeInjector::paste(), 10);
    ready(&mut node).await;
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    statuses_until(&mut node, |s| listening_with(s, SegmentProgress { done: 20, total: 20 })).await;
    let (done, _) = stop(&mut node).await;
    match &done {
        DictationPhase::Done { text, raw_text, refined, refine_error, via, chars, .. } => {
            assert!((2_001..5_000).contains(chars), "{chars}");
            assert_eq!(text, raw_text);
            assert!(!refined);
            assert_eq!(refine_error.as_deref(), Some(REFINE_SKIPPED));
            assert_eq!(*via, Via::Paste);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(node.refiner.calls(), 0);
    assert_eq!(node.injector.injected().len(), 1);
    node.handle.send(CoreCommand::Shutdown).await.unwrap();

    // 250 字 each: about 5100 characters.
    let mut node = start(FakeAudio::speech().long(600), FakeTranscriber::numbered(250, &[], Duration::ZERO), FakeInjector::paste(), 10);
    ready(&mut node).await;
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    statuses_until(&mut node, |s| listening_with(s, SegmentProgress { done: 20, total: 20 })).await;
    let (done, _) = stop(&mut node).await;
    let text = match &done {
        DictationPhase::Done { text, via: Via::Clipboard, chars, .. } if *chars > 5_000 => text.clone(),
        other => panic!("{other:?}"),
    };
    assert!(node.injector.injected().is_empty(), "never pasted");
    assert_eq!(node.injector.clipboard_copies(), vec![text]);
    let entry = newest_entry(&mut node).await;
    assert_eq!(entry.outcome, Outcome::Clipboard { reason: TOO_LONG_TO_PASTE.into(), code: Some(ClipboardCode::TooLong) });
    wait_no_recordings(node.dir.path()).await;
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// `streaming_final` whose stream cannot start falls back to the whole take (docs/dictation.md
/// §12); a long one takes it from the file, segment by segment, as a whole take would.
#[tokio::test]
async fn a_long_streaming_take_that_falls_back_takes_its_text_from_the_file() {
    let streaming = FakeStreaming::failing_open("模型未下载");
    let mut node = start_streaming(FakeAudio::speech().long(200), FakeTranscriber::numbered(0, &[], Duration::ZERO), streaming, OutputMode::StreamingFinal);
    ready(&mut node).await;
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    statuses_until(&mut node, |s| listening_with(s, SegmentProgress { done: 6, total: 6 })).await;
    let (done, _) = stop(&mut node).await;
    match &done {
        DictationPhase::Done { raw_text, mode, live_error, duration_ms, .. } => {
            assert_eq!(raw_text, &numbered(1, 7));
            assert_eq!(*mode, OutputMode::WholeTake);
            assert!(live_error.as_deref().is_some_and(|e| e.contains("模型未下载")), "{live_error:?}");
            assert_eq!(*duration_ms, 200_000);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(newest_entry(&mut node).await.segments.map(|s| s.len()), Some(7));
    wait_no_recordings(node.dir.path()).await;
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// `live_inject` failing after it pasted a sentence (docs/dictation.md §12): the rest of a long
/// take comes from the file, from where that sentence ended, and is pasted as the last piece;
/// nothing is pasted twice.
#[tokio::test]
async fn a_long_live_inject_take_that_falls_back_pastes_the_rest_from_the_file() {
    // The default script commits 「你好，世界。」 at 400 ms; the decoder fails on the sixth chunk.
    let streaming = FakeStreaming::erroring_after(5);
    let mut node = start_streaming(FakeAudio::speech().long(150), FakeTranscriber::numbered(0, &[], Duration::ZERO), streaming, OutputMode::LiveInject);
    ready(&mut node).await;
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    statuses_until(&mut node, |s| listening_with(s, SegmentProgress { done: 5, total: 5 })).await;
    assert_eq!(node.transcriber.durations_ms()[0], 29_500, "the first segment from 400 ms on");
    let (done, _) = stop(&mut node).await;
    let rest = numbered(1, 5);
    match &done {
        DictationPhase::Done { text, mode, segments, live_error, .. } => {
            assert_eq!(*text, format!("你好，世界。{rest}"));
            assert_eq!(*mode, OutputMode::LiveInject);
            assert_eq!(
                segments.as_deref(),
                Some(&[Segment { text: "你好，世界。".into(), start_ms: 0, end_ms: 400 }, Segment { text: rest.clone(), start_ms: 400, end_ms: 150_000 }][..])
            );
            assert_eq!(live_error.as_deref(), Some("fake decoder failed"));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(node.injector.injected(), vec!["你好，世界。".to_owned(), rest]);
    wait_no_recordings(node.dir.path()).await;
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// 用 AI 预设处理 on a long take's entry (docs/dictation.md §22): its 2100 characters in two parts
/// through 要点纪要 and a summary of their points, the progress counted, the result stored beside the
/// entry's own text; a cancel stores nothing, and a request without an AI service or for an entry
/// that is gone fails at once.
#[tokio::test]
async fn a_long_entry_is_processed_in_parts_and_the_result_kept_with_it() {
    use voltip_core::history::process::{PROCESS_ENTRY_GONE, ProcessState};
    use voltip_core::{BuiltinPreset, PresetId};
    let dir = tempfile::tempdir().unwrap();
    let refiner = FakeRefiner::slow("- 要点", Duration::from_millis(30));
    let mut node =
        start_with(dir, FakeAudio::speech().long(600), FakeTranscriber::numbered(100, &[], Duration::ZERO), FakeInjector::paste(), 10, None, refiner);
    ready(&mut node).await;
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    statuses_until(&mut node, |s| listening_with(s, SegmentProgress { done: 20, total: 20 })).await;
    stop(&mut node).await;
    let entry = newest_entry(&mut node).await;
    assert!(entry.processed.is_none() && entry.text.chars().count() > 2000);
    let calls_before = node.refiner.calls();
    let notes = PresetId::Builtin(BuiltinPreset::Notes);
    node.handle.send(CoreCommand::HistoryProcess { request_id: 7, id: entry.id, preset: notes }).await.unwrap();
    let (mut running, mut stored) = (Vec::new(), None);
    let processed = wait(&mut node, |e| match e {
        CoreEvent::HistoryProcess { request_id: 7, id, state } => {
            assert_eq!(*id, entry.id);
            match state {
                ProcessState::Running { done, total } => {
                    running.push((*done, *total));
                    None
                }
                ProcessState::Done { processed } => Some(processed.clone()),
                other => panic!("{other:?}"),
            }
        }
        // The history is sent again with the stored result, before the answer.
        CoreEvent::History { recent, .. } => {
            stored = recent.first().cloned();
            None
        }
        _ => None,
    })
    .await;
    assert_eq!(running.first(), Some(&(0, 3)), "two parts and the summary: {running:?}");
    assert_eq!(processed.text, "- 要点");
    assert_eq!(processed.preset.id, notes);
    assert_eq!(node.refiner.calls() - calls_before, 3);
    let stored = stored.expect("the history came again");
    assert_eq!((stored.text, stored.processed), (entry.text.clone(), Some(Box::new(processed))));

    // Cancelled while its first part is out: nothing is stored.
    node.handle.send(CoreCommand::HistoryProcess { request_id: 8, id: entry.id, preset: PresetId::Builtin(BuiltinPreset::Formal) }).await.unwrap();
    wait(&mut node, |e| matches!(e, CoreEvent::HistoryProcess { request_id: 8, state: ProcessState::Running { .. }, .. }).then_some(())).await;
    node.handle.send(CoreCommand::HistoryProcessCancel { request_id: 8 }).await.unwrap();
    wait(&mut node, |e| matches!(e, CoreEvent::HistoryProcess { request_id: 8, state: ProcessState::Cancelled, .. }).then_some(())).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    node.handle.send(CoreCommand::HistoryProcess { request_id: 9, id: uuid::Uuid::new_v4(), preset: notes }).await.unwrap();
    let reason = wait(&mut node, |e| match e {
        CoreEvent::HistoryProcess { request_id: 9, state: ProcessState::Failed { reason }, .. } => Some(reason.clone()),
        CoreEvent::HistoryProcess { request_id: 8, state, .. } => panic!("the cancelled request went on: {state:?}"),
        _ => None,
    })
    .await;
    assert_eq!(reason, PROCESS_ENTRY_GONE);
    node.handle.send(CoreCommand::HistoryProcessCancel { request_id: 99 }).await.unwrap();
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}
