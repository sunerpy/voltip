#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Importing audio and video files through the real core task (docs/dictation.md §31): the queue
//! and the states it shows, the entry an import makes, a file that cannot be decoded, cancel,
//! retry, a dictation going first, the decoded files, and the clients: the chosen models only,
//! never the built-in service and never a fallback chain, metered as imports.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::mpsc;
use uuid::Uuid;
use voltip_core::audio_cache::RecognitionChoice;
use voltip_core::dictation::fakes::{
    FAKE_QUOTA_DETAIL, FakeAudio, FakeInjector, FakeMediaDecoder, FakeModels, FakeRefiner, FakeTranscriber, FixedSegmenterFactory, factory_by_model, ports_with,
};
use voltip_core::import::{ImportFailure, ImportJob, ImportRequest, ImportState, ImportsView, MediaError, PolishModel};
use voltip_core::usage::UsageOrigin;
use voltip_core::{
    AppCore, BuiltIn, CoreCommand, CoreConfig, CoreEvent, CoreHandle, DictationPhase, EngineSettings, FallbackModel, FallbackSettings, HistoryEntry,
    HistorySettings, ProviderId, ProviderSettings, RefineFailure, ResolvedEngines, Settings, SettingsStore,
};
use voltip_identity::{MemorySecretStore, SecretStore};

const STEP: Duration = Duration::from_secs(10);

/// What the settings recognise with (Model Studio) and clean up with (Groq).
const SAVED_ASR: &str = "qwen-audio-3.1-asr-flash";
const SAVED_LLM: &str = "llama-3.3-70b";
/// Models an import may choose instead: another of Model Studio's, Groq's Whisper, a clean-up on
/// Model Studio, and the fake library's light local model (by its name).
const OTHER_ASR: &str = "qwen3-asr-flash";
const GROQ_ASR: &str = "whisper-large-v3";
const OTHER_LLM: &str = "qwen-plus";
const LOCAL_ID: &str = "paraformer-zh";
const LOCAL_NAME: &str = "Paraformer 中文";

/// The built-in service of these tests (the test build has none): recognition and clean-up.
const BUILT_IN: BuiltIn = BuiltIn {
    asr_url: Some("https://<asr-host>"),
    asr_token: Some("t"),
    asr_model: "builtin/asr",
    refine_url: Some("https://<refine-host>/v1"),
    refine_api_key: Some("k"),
    refine_model: "builtin/llm",
    ..BuiltIn::EMPTY
};

/// The settings: Model Studio recognises, Groq cleans up, and both fall back to the built-in
/// service when their model runs out.
fn saved() -> EngineSettings {
    let builtin = || FallbackSettings { enabled: true, models: vec![FallbackModel { provider: ProviderId::Builtin, model: String::new() }] };
    EngineSettings {
        asr_provider: ProviderId::Aliyun,
        llm_provider: ProviderId::Groq,
        refine_enabled: true,
        providers: [
            (ProviderId::Aliyun, ProviderSettings { asr_model: Some(SAVED_ASR.into()), ..Default::default() }),
            (ProviderId::Groq, ProviderSettings { llm_model: Some(SAVED_LLM.into()), ..Default::default() }),
        ]
        .into(),
        asr_fallback: builtin(),
        llm_fallback: builtin(),
        ..EngineSettings::default()
    }
}

/// Every model's fake.
struct Fakes {
    saved_asr: Arc<FakeTranscriber>,
    other_asr: Arc<FakeTranscriber>,
    groq_asr: Arc<FakeTranscriber>,
    local_asr: Arc<FakeTranscriber>,
    builtin_asr: Arc<FakeTranscriber>,
    saved_llm: Arc<FakeRefiner>,
    other_llm: Arc<FakeRefiner>,
    builtin_llm: Arc<FakeRefiner>,
}

impl Default for Fakes {
    fn default() -> Self {
        Self {
            saved_asr: Arc::new(FakeTranscriber::ok("听写的文字。")),
            other_asr: Arc::new(FakeTranscriber::numbered(0, &[], Duration::ZERO)),
            groq_asr: Arc::new(FakeTranscriber::numbered(0, &[], Duration::ZERO)),
            local_asr: Arc::new(FakeTranscriber::numbered(0, &[], Duration::ZERO)),
            builtin_asr: Arc::new(FakeTranscriber::ok("内置识别。")),
            saved_llm: Arc::new(FakeRefiner::ok("润色。")),
            other_llm: Arc::new(FakeRefiner::ok("润色后的一段。")),
            builtin_llm: Arc::new(FakeRefiner::ok("内置润色。")),
        }
    }
}

struct Node {
    handle: CoreHandle,
    events: mpsc::Receiver<CoreEvent>,
    fakes: Fakes,
    decoder: Arc<FakeMediaDecoder>,
    /// Every configuration the client factory was handed, in order.
    views: Arc<Mutex<Vec<ResolvedEngines>>>,
    /// The queue as last told, once told.
    imports: ImportsView,
    told: bool,
    /// The newest entries as last told.
    recent: Vec<HistoryEntry>,
    /// The states each job showed, in order, repeats left out.
    states: HashMap<Uuid, Vec<ImportState>>,
    /// The jobs seen waiting for a dictation.
    paused: Vec<Uuid>,
    errors: Vec<String>,
    phase: DictationPhase,
}

/// How a core of these tests is made.
struct Setup {
    decoder: FakeMediaDecoder,
    built_in: BuiltIn,
    /// The shell's decoder is plugged in (`false`: the phone).
    imports: bool,
    /// The fake library with the light local model installed.
    models: bool,
    fakes: Fakes,
}

impl Default for Setup {
    fn default() -> Self {
        Self { decoder: FakeMediaDecoder::new(25), built_in: BUILT_IN, imports: true, models: false, fakes: Fakes::default() }
    }
}

fn start(dir: &Path, setup: Setup) -> Node {
    SettingsStore::new(dir).save(&Settings { relay_enabled: false, engines: saved(), ..Settings::default() }).unwrap();
    let Setup { decoder, built_in, imports, models, fakes } = setup;
    let decoder = Arc::new(decoder);
    let mut config = CoreConfig::new(dir.to_path_buf());
    config.default_device_name = "Import Test".into();
    config.test_hooks.built_in = Some(built_in);
    if imports {
        config.media_decoder = Some(decoder.clone());
    }
    let mut ports = ports_with(Arc::new(FakeAudio::speech()), fakes.saved_asr.clone(), None, Arc::new(FakeInjector::paste()));
    let inner = factory_by_model(
        &[
            (SAVED_ASR, fakes.saved_asr.clone()),
            (OTHER_ASR, fakes.other_asr.clone()),
            (GROQ_ASR, fakes.groq_asr.clone()),
            (LOCAL_NAME, fakes.local_asr.clone()),
            ("builtin/asr", fakes.builtin_asr.clone()),
        ],
        &[(SAVED_LLM, fakes.saved_llm.clone()), (OTHER_LLM, fakes.other_llm.clone()), ("builtin/llm", fakes.builtin_llm.clone())],
    );
    let views: Arc<Mutex<Vec<ResolvedEngines>>> = Arc::default();
    let seen = views.clone();
    ports.factory = Arc::new(move |engines: &ResolvedEngines| {
        seen.lock().unwrap().push(engines.clone());
        inner(engines)
    });
    ports.segmenter = Some(Arc::new(FixedSegmenterFactory(10)));
    if models {
        ports.models = Some(Arc::new(FakeModels::new(1).with_installed(LOCAL_ID)));
    }
    let store = MemorySecretStore::new();
    store.set("provider-key.aliyun", b"sk-test").unwrap();
    store.set("provider-key.groq", b"gsk-test").unwrap();
    let (handle, events) = AppCore::start_with(config, Arc::new(store), ports).unwrap();
    Node {
        handle,
        events,
        fakes,
        decoder,
        views,
        imports: ImportsView::default(),
        told: false,
        recent: Vec::new(),
        states: HashMap::new(),
        paused: Vec::new(),
        errors: Vec::new(),
        phase: DictationPhase::Idle,
    }
}

/// The next event, with what it changed folded into `node`.
async fn next(node: &mut Node) -> CoreEvent {
    let ev = tokio::time::timeout(STEP, node.events.recv()).await.expect("event within 10 s").expect("core alive");
    match &ev {
        CoreEvent::Imports(view) => {
            for job in &view.jobs {
                let seen = node.states.entry(job.id).or_default();
                if seen.last() != Some(&job.state) {
                    seen.push(job.state.clone());
                }
                if job.paused && !node.paused.contains(&job.id) {
                    node.paused.push(job.id);
                }
            }
            node.imports = view.clone();
            node.told = true;
        }
        CoreEvent::History { recent, .. } => node.recent = recent.clone(),
        CoreEvent::Error(e) => node.errors.push(e.clone()),
        CoreEvent::Dictation(status) => node.phase = status.phase.clone(),
        _ => {}
    }
    ev
}

/// The queue once `pred` holds for it: as last told, or later.
async fn queue_until(node: &mut Node, what: &str, pred: impl Fn(&ImportsView) -> bool) -> ImportsView {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while !pred(&node.imports) {
        assert!(tokio::time::Instant::now() < deadline, "the queue never showed {what}: {:?}", node.imports);
        next(node).await;
    }
    node.imports.clone()
}

/// The job `id` once it is done or failed.
async fn finished(node: &mut Node, id: Uuid) -> ImportJob {
    let view = queue_until(node, "the job finished", |v| {
        v.jobs.iter().any(|j| j.id == id && matches!(j.state, ImportState::Done { .. } | ImportState::Failed { .. }))
    })
    .await;
    view.jobs.into_iter().find(|j| j.id == id).unwrap()
}

/// The error the core reports next (or reported since the last look).
async fn error(node: &mut Node) -> String {
    while node.errors.is_empty() {
        next(node).await;
    }
    node.errors.remove(0)
}

/// The core is up and has told the queue.
async fn ready(node: &mut Node) {
    while !matches!(next(node).await, CoreEvent::Ready { .. }) {}
    while !node.told {
        next(node).await;
    }
}

/// A file to import, with bytes no decoder could mistake for anything.
fn media(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, b"\x00\x00\x00\x18ftypmp42 not really a video").unwrap();
    path
}

fn request(path: &Path, asr: RecognitionChoice, refine: Option<PolishModel>) -> ImportRequest {
    ImportRequest { path: path.to_path_buf(), asr, refine, language: Some("zh".into()) }
}

fn remote(provider: ProviderId, model: &str) -> RecognitionChoice {
    RecognitionChoice::Remote { provider, model: model.into() }
}

/// The jobs just queued by the last `ImportAdd`, in order.
async fn added(node: &mut Node, requests: Vec<ImportRequest>) -> Vec<Uuid> {
    let before: Vec<Uuid> = node.imports.jobs.iter().map(|j| j.id).collect();
    let n = requests.len();
    node.handle.send(CoreCommand::ImportAdd(requests)).await.unwrap();
    let view = queue_until(node, "the new jobs", |v| v.jobs.iter().filter(|j| !before.contains(&j.id)).count() == n).await;
    view.jobs.iter().map(|j| j.id).filter(|id| !before.contains(id)).collect()
}

/// The decoded files waiting in `<data_dir>/imports`.
fn decoded_files(dir: &Path) -> usize {
    std::fs::read_dir(dir.join("imports")).map(|d| d.count()).unwrap_or(0)
}

/// `(provider, origin)` of every recognition the usage record holds.
fn usage_rows(dir: &Path) -> Vec<(String, String)> {
    fn collect(value: &serde_json::Value, rows: &mut Vec<(String, String)>) {
        match value {
            serde_json::Value::Object(map) => {
                if let (Some(provider), Some(origin), Some("asr")) = (map.get("provider"), map.get("origin"), map.get("service").and_then(|s| s.as_str())) {
                    rows.push((provider.as_str().unwrap_or_default().to_owned(), origin["kind"].as_str().unwrap_or_default().to_owned()));
                }
                map.values().for_each(|v| collect(v, rows));
            }
            serde_json::Value::Array(items) => items.iter().for_each(|v| collect(v, rows)),
            _ => {}
        }
    }
    let Ok(bytes) = std::fs::read(dir.join("usage.json")) else { return Vec::new() };
    let mut rows = Vec::new();
    collect(&serde_json::from_slice(&bytes).unwrap(), &mut rows);
    rows
}

/// docs/dictation.md §31: a file goes through the queue — waiting, decoding, its segments
/// recognised — and becomes one history entry whose segments keep their times; the original file
/// is only read, and the decoded one goes once the entry is in.
#[tokio::test]
async fn an_imported_file_becomes_one_history_entry_and_the_original_stays() {
    let dir = tempfile::tempdir().unwrap();
    let mut node = start(dir.path(), Setup::default());
    ready(&mut node).await;
    assert!(node.imports.available && node.imports.jobs.is_empty(), "{:?}", node.imports);
    let file = media(dir.path(), "会议录音.mp4");
    let original = std::fs::read(&file).unwrap();

    let [id] = added(&mut node, vec![request(&file, remote(ProviderId::Groq, GROQ_ASR), None)]).await[..] else { panic!("one job") };
    let job = finished(&mut node, id).await;
    let ImportState::Done { entry, polish_failed: 0 } = job.state else { panic!("{job:?}") };
    assert_eq!((job.name.as_str(), job.size, job.duration_ms, job.paused), ("会议录音.mp4", original.len() as u64, Some(25_000), false));
    let states = &node.states[&id];
    let at = |pred: &dyn Fn(&ImportState) -> bool| states.iter().position(pred).unwrap_or_else(|| panic!("{states:?}"));
    assert_eq!(states[0], ImportState::Queued, "{states:?}");
    assert!(at(&|s| matches!(s, ImportState::Decoding { .. })) < at(&|s| matches!(s, ImportState::Recognizing { done: 0, total: 3 })), "{states:?}");
    assert!(at(&|s| matches!(s, ImportState::Recognizing { done: 3, total: 3 })) < at(&|s| matches!(s, ImportState::Done { .. })), "{states:?}");
    assert!(!states.iter().any(|s| matches!(s, ImportState::Polishing { .. })), "no polish asked for");

    while node.recent.first().map(|e| e.id) != Some(entry) {
        next(&mut node).await;
    }
    let e = node.recent[0].clone();
    assert_eq!(e.text, "第1段。第2段。第3段。");
    assert_eq!(e.raw_text, e.text);
    assert_eq!(e.file.as_ref().map(|f| (f.name.as_str(), f.duration_ms)), Some(("会议录音.mp4", 25_000)));
    let times: Vec<(u64, u64)> = e.segments.as_ref().unwrap().iter().map(|s| (s.start_ms, s.end_ms)).collect();
    assert_eq!(times, [(0, 10_000), (10_000, 20_000), (20_000, 25_000)]);
    assert_eq!((e.asr_model.as_str(), e.refined, e.duration_ms), (GROQ_ASR, false, 25_000));
    assert!(e.origin.is_none() && e.edit.is_none(), "{e:?}");

    // On the chosen model, as an import, with the language chosen; nothing else was asked.
    let calls = node.fakes.groq_asr.usage_calls();
    assert_eq!(calls.len(), 3);
    assert!(calls.iter().all(|c| c.origin == Some(UsageOrigin::Import)), "{calls:?}");
    assert_eq!(node.fakes.groq_asr.languages(), vec![Some("zh".to_owned()); 3]);
    assert_eq!((node.fakes.saved_asr.calls(), node.fakes.builtin_asr.calls(), node.fakes.saved_llm.calls(), node.fakes.builtin_llm.calls()), (0, 0, 0, 0));
    assert_eq!(node.decoder.decodes(), 1);
    assert_eq!(std::fs::read(&file).unwrap(), original, "the original file is only read");
    assert_eq!(decoded_files(dir.path()), 0, "the decoded file went with the entry");

    // 移除: off the queue; the entry stays.
    node.handle.send(CoreCommand::ImportRemove(id)).await.unwrap();
    queue_until(&mut node, "an empty queue", |v| v.jobs.is_empty()).await;
    assert!(file.exists());
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// docs/dictation.md §31: an import runs on its models alone. With both fallback lists on and
/// holding the built-in service, a recognition model that keeps answering 额度用完 fails the job,
/// and a clean-up model that does keeps each segment's text: the built-in clients are never
/// asked, and every configuration the factory got for the import has no fallback model.
#[tokio::test]
async fn regression_an_import_never_reaches_a_saved_builtin_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let fakes = Fakes { other_asr: Arc::new(FakeTranscriber::quota()), other_llm: Arc::new(FakeRefiner::quota()), ..Fakes::default() };
    let mut node = start(dir.path(), Setup { fakes, ..Setup::default() });
    ready(&mut node).await;
    let file = media(dir.path(), "季度会议.mkv");
    let no_fallback = |views: &[ResolvedEngines]| {
        assert!(!views.is_empty(), "the import's clients were built");
        for view in views {
            assert_eq!((view.asr_fallback.targets().count(), view.refine_fallback.targets().count()), (0, 0), "{view:?}");
            assert_ne!(view.asr_provider, ProviderId::Builtin);
        }
    };

    let built = node.views.lock().unwrap().len();
    let [recognition] = added(&mut node, vec![request(&file, remote(ProviderId::Aliyun, OTHER_ASR), None)]).await[..] else { panic!() };
    let job = finished(&mut node, recognition).await;
    let ImportState::Failed { failure: ImportFailure::Recognition { detail }, resumable: true } = &job.state else { panic!("{job:?}") };
    assert!(detail.contains(FAKE_QUOTA_DETAIL), "{detail}");
    assert_eq!((node.fakes.other_asr.calls(), node.fakes.builtin_asr.calls()), (2, 0), "two tries of the first segment, none on the built-in service");
    no_fallback(&node.views.lock().unwrap()[built..]);

    let built = node.views.lock().unwrap().len();
    let polish = Some(PolishModel { provider: ProviderId::Aliyun, model: OTHER_LLM.into() });
    let [polished] = added(&mut node, vec![request(&file, remote(ProviderId::Groq, GROQ_ASR), polish)]).await[..] else { panic!() };
    let job = finished(&mut node, polished).await;
    let ImportState::Done { entry, polish_failed: 3 } = job.state else { panic!("{job:?}") };
    assert_eq!((node.fakes.other_llm.calls(), node.fakes.builtin_llm.calls(), node.fakes.saved_llm.calls()), (3, 0, 0));
    let views = node.views.lock().unwrap()[built..].to_vec();
    no_fallback(&views);
    assert!(views.iter().all(|v| v.llm_provider == Some(ProviderId::Aliyun) && v.refine_model == OTHER_LLM), "the chosen clean-up: {views:?}");
    while node.recent.first().map(|e| e.id) != Some(entry) {
        next(&mut node).await;
    }
    let e = &node.recent[0];
    assert_eq!((e.text.as_str(), e.refined, e.refine_failure), ("第1段。第2段。第3段。", false, Some(RefineFailure::Quota)));
    assert!(e.segments.as_ref().unwrap().iter().all(|s| s.refined.is_none()), "each segment kept its text");
    assert_eq!((node.fakes.saved_asr.calls(), node.fakes.builtin_asr.calls()), (0, 0));
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// docs/dictation.md §31: the polish runs segment by segment on the chosen service, and the
/// subtitles can use it.
#[tokio::test]
async fn a_polished_import_keeps_each_segments_polished_text() {
    let dir = tempfile::tempdir().unwrap();
    let mut node = start(dir.path(), Setup::default());
    ready(&mut node).await;
    let file = media(dir.path(), "访谈.m4a");
    let polish = Some(PolishModel { provider: ProviderId::Aliyun, model: OTHER_LLM.into() });
    let [id] = added(&mut node, vec![request(&file, remote(ProviderId::Groq, GROQ_ASR), polish)]).await[..] else { panic!() };
    let job = finished(&mut node, id).await;
    let ImportState::Done { entry, polish_failed: 0 } = job.state else { panic!("{job:?}") };
    let states = &node.states[&id];
    assert!(states.contains(&ImportState::Polishing { done: 0, total: 3 }) && states.contains(&ImportState::Polishing { done: 3, total: 3 }), "{states:?}");
    while node.recent.first().map(|e| e.id) != Some(entry) {
        next(&mut node).await;
    }
    let e = &node.recent[0];
    assert_eq!(e.text, "润色后的一段。润色后的一段。润色后的一段。");
    assert_eq!(e.raw_text, "第1段。第2段。第3段。");
    assert!(e.refined && e.refine_model.is_some(), "{e:?}");
    let segments = e.segments.as_ref().unwrap();
    assert!(segments.iter().all(|s| s.refined.as_deref() == Some("润色后的一段。")), "{segments:?}");
    let hints = node.fakes.other_llm.hints();
    assert_eq!(hints.len(), 3);
    assert!(hints.iter().all(|h| h.origin == Some(UsageOrigin::Import) && h.language.as_deref() == Some("zh")), "{hints:?}");
    assert_eq!((node.fakes.saved_llm.calls(), node.fakes.builtin_llm.calls()), (0, 0));
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// docs/dictation.md §31: a job whose segment fails twice stops, the next one runs, and 重试 goes
/// on from that segment with the file already decoded.
#[tokio::test]
async fn a_failed_import_does_not_hold_up_the_next_and_its_retry_goes_on_from_its_segment() {
    let dir = tempfile::tempdir().unwrap();
    let fakes = Fakes { other_asr: Arc::new(FakeTranscriber::numbered(0, &[2, 3], Duration::ZERO)), ..Fakes::default() };
    let mut node = start(dir.path(), Setup { fakes, ..Setup::default() });
    ready(&mut node).await;
    let (first, second) = (media(dir.path(), "第一个.mp3"), media(dir.path(), "第二个.mp3"));
    let ids =
        added(&mut node, vec![request(&first, remote(ProviderId::Aliyun, OTHER_ASR), None), request(&second, remote(ProviderId::Groq, GROQ_ASR), None)]).await;
    let [a, b] = ids[..] else { panic!("{ids:?}") };
    assert_eq!(node.imports.jobs.iter().map(|j| j.id).collect::<Vec<_>>(), [a, b], "in the order they were added");

    let failed = finished(&mut node, a).await;
    assert!(matches!(&failed.state, ImportState::Failed { failure: ImportFailure::Recognition { .. }, resumable: true }), "{failed:?}");
    let done = finished(&mut node, b).await;
    assert!(matches!(done.state, ImportState::Done { .. }), "{done:?}");
    assert_eq!(node.decoder.decodes(), 2);
    assert_eq!(decoded_files(dir.path()), 1, "the failed job keeps its decoded file for the retry");

    node.handle.send(CoreCommand::ImportRetry(a)).await.unwrap();
    queue_until(&mut node, "the retry", |v| v.jobs.iter().any(|j| j.id == a && !matches!(j.state, ImportState::Failed { .. }))).await;
    let tried = node.states[&a].len();
    let retried = finished(&mut node, a).await;
    let ImportState::Done { entry, .. } = retried.state else { panic!("{retried:?}") };
    let after = &node.states[&a][tried - 1..];
    assert!(!after.iter().any(|s| matches!(s, ImportState::Decoding { .. })), "not decoded again: {after:?}");
    assert!(after.contains(&ImportState::Recognizing { done: 1, total: 3 }), "{after:?}");
    assert_eq!(node.decoder.decodes(), 2);
    while !node.recent.iter().any(|e| e.id == entry) {
        next(&mut node).await;
    }
    let e = node.recent.iter().find(|e| e.id == entry).unwrap();
    assert_eq!(e.text, "第1段。第4段。第5段。", "segment 1 from the first try, 2 and 3 from the retry");
    assert_eq!(node.fakes.other_asr.calls(), 5);
    assert_eq!(decoded_files(dir.path()), 0);
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// docs/dictation.md §31: a file the decoder cannot read fails its job with the reason; a retry
/// starts it again.
#[tokio::test]
async fn a_file_that_cannot_be_decoded_fails_with_its_reason() {
    let dir = tempfile::tempdir().unwrap();
    let mut node = start(dir.path(), Setup { decoder: FakeMediaDecoder::failing(MediaError::UnsupportedCodec("AC-3".into())), ..Setup::default() });
    ready(&mut node).await;
    let file = media(dir.path(), "电影.mkv");
    let [id] = added(&mut node, vec![request(&file, remote(ProviderId::Groq, GROQ_ASR), None)]).await[..] else { panic!() };
    let job = finished(&mut node, id).await;
    assert_eq!(job.state, ImportState::Failed { failure: ImportFailure::UnsupportedCodec { codec: "AC-3".into() }, resumable: false });
    assert_eq!(node.fakes.groq_asr.calls(), 0);
    let tried = node.states[&id].len();
    node.handle.send(CoreCommand::ImportRetry(id)).await.unwrap();
    while node.states[&id].len() < tried + 2 || !matches!(node.states[&id].last(), Some(ImportState::Failed { .. })) {
        next(&mut node).await;
    }
    assert_eq!(node.decoder.decodes(), 2, "started again");
    assert_eq!(node.states[&id][tried], ImportState::Queued);
    assert!(node.recent.is_empty() && decoded_files(dir.path()) == 0);
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// docs/dictation.md §31: 取消 takes a waiting or a running job off the queue at once; nothing
/// goes into the history, its decoded file goes, and the original stays.
#[tokio::test]
async fn a_cancel_stops_the_import_and_leaves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut node = start(dir.path(), Setup { decoder: FakeMediaDecoder::held(25), ..Setup::default() });
    ready(&mut node).await;
    let file = media(dir.path(), "讲座.mov");
    let original = std::fs::read(&file).unwrap();
    let ids = added(&mut node, vec![request(&file, remote(ProviderId::Groq, GROQ_ASR), None), request(&file, remote(ProviderId::Groq, GROQ_ASR), None)]).await;
    let [running, waiting] = ids[..] else { panic!("{ids:?}") };
    queue_until(&mut node, "the first decoding", |v| v.jobs.iter().any(|j| j.id == running && matches!(j.state, ImportState::Decoding { .. }))).await;
    assert_eq!(node.imports.jobs.iter().find(|j| j.id == waiting).map(|j| &j.state), Some(&ImportState::Queued));

    node.handle.send(CoreCommand::ImportCancel(waiting)).await.unwrap();
    queue_until(&mut node, "the waiting job gone", |v| v.jobs.iter().map(|j| j.id).collect::<Vec<_>>() == [running]).await;
    node.handle.send(CoreCommand::ImportCancel(running)).await.unwrap();
    queue_until(&mut node, "an empty queue", |v| v.jobs.is_empty()).await;

    // The next file runs once the cancelled one has stopped.
    node.decoder.release();
    let [next_job] = added(&mut node, vec![request(&file, remote(ProviderId::Groq, GROQ_ASR), None)]).await[..] else { panic!() };
    let job = finished(&mut node, next_job).await;
    let ImportState::Done { entry, .. } = job.state else { panic!("{job:?}") };
    while node.recent.first().map(|e| e.id) != Some(entry) {
        next(&mut node).await;
    }
    assert_eq!(node.recent.len(), 1, "the cancelled jobs wrote nothing: {:?}", node.recent);
    assert_eq!(node.fakes.groq_asr.calls(), 3, "only the last job was recognised");
    assert_eq!(decoded_files(dir.path()), 0);
    assert_eq!(std::fs::read(&file).unwrap(), original);
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// docs/dictation.md §31: a dictation always goes first. A job waits at its next segment while
/// a take runs, says so, and goes on after it; the take's recognition is metered as this
/// device's, the import's as an import.
#[tokio::test]
async fn an_import_waits_while_a_dictation_runs() {
    let dir = tempfile::tempdir().unwrap();
    let mut node = start(dir.path(), Setup::default());
    ready(&mut node).await;
    node.handle.send(CoreCommand::DictationStart).await.unwrap();
    while !matches!(node.phase, DictationPhase::Listening { ready: true, .. }) {
        next(&mut node).await;
    }
    let file = media(dir.path(), "播客.mp3");
    let [id] = added(&mut node, vec![request(&file, remote(ProviderId::Groq, GROQ_ASR), None)]).await[..] else { panic!() };
    queue_until(&mut node, "the job waiting", |v| v.jobs.iter().any(|j| j.id == id && j.paused)).await;
    assert_eq!(node.fakes.groq_asr.calls(), 0, "no segment while the take runs");
    assert_eq!(node.decoder.decodes(), 1, "the file is decoded meanwhile");

    node.handle.send(CoreCommand::DictationStop).await.unwrap();
    let job = finished(&mut node, id).await;
    assert!(matches!(job.state, ImportState::Done { .. }) && !job.paused, "{job:?}");
    assert!(node.paused.contains(&id));
    assert_eq!(node.fakes.saved_asr.calls(), 1, "the take was recognised");
    assert!(node.fakes.saved_asr.usage_calls().iter().all(|c| c.origin.is_none() || c.origin == Some(UsageOrigin::ThisDevice)));
    assert!(node.fakes.groq_asr.usage_calls().iter().all(|c| c.origin == Some(UsageOrigin::Import)));

    // The usage record has the take as this device's and the file as an import.
    let expected =
        |rows: &[(String, String)]| rows.contains(&("aliyun".to_owned(), "this_device".to_owned())) && rows.contains(&("groq".to_owned(), "import".to_owned()));
    while !expected(&usage_rows(dir.path())) {
        next(&mut node).await;
    }
    assert!(!usage_rows(dir.path()).iter().any(|(p, o)| (p == "groq" && o != "import") || (p == "aliyun" && o == "import")), "{:?}", usage_rows(dir.path()));
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// docs/dictation.md §31: what an import is refused for, before anything is queued: the built-in
/// service, the history off, a path that is no file, and an app that imports nothing (the phone).
#[tokio::test]
async fn imports_are_refused_with_the_built_in_service_the_history_off_and_on_the_phone() {
    let dir = tempfile::tempdir().unwrap();
    let mut node = start(dir.path(), Setup::default());
    ready(&mut node).await;
    let file = media(dir.path(), "录音.wav");
    node.handle.send(CoreCommand::ImportAdd(vec![request(&file, remote(ProviderId::Builtin, ""), None)])).await.unwrap();
    assert_eq!(error(&mut node).await, voltip_core::import::ImportError::BuiltinNotAllowed.to_string());
    let builtin_polish = Some(PolishModel { provider: ProviderId::Builtin, model: String::new() });
    node.handle.send(CoreCommand::ImportAdd(vec![request(&file, remote(ProviderId::Groq, GROQ_ASR), builtin_polish)])).await.unwrap();
    assert_eq!(error(&mut node).await, voltip_core::import::ImportError::BuiltinNotAllowed.to_string());

    node.handle.send(CoreCommand::ImportAdd(vec![request(dir.path(), remote(ProviderId::Groq, GROQ_ASR), None)])).await.unwrap();
    let why = error(&mut node).await;
    assert!(why.starts_with("无法读取文件") && why.contains(&*dir.path().file_name().unwrap().to_string_lossy()), "{why}");

    node.handle.send(CoreCommand::SetHistory(HistorySettings { enabled: false, ..HistorySettings::default() })).await.unwrap();
    while !matches!(next(&mut node).await, CoreEvent::Settings(s) if !s.history.enabled) {}
    node.handle.send(CoreCommand::ImportAdd(vec![request(&file, remote(ProviderId::Groq, GROQ_ASR), None)])).await.unwrap();
    assert_eq!(error(&mut node).await, voltip_core::import::ImportError::HistoryOff.to_string());
    assert!(node.imports.jobs.is_empty(), "nothing was queued: {:?}", node.imports);
    assert_eq!((node.decoder.decodes(), node.fakes.groq_asr.calls(), node.fakes.builtin_asr.calls()), (0, 0, 0));
    node.handle.send(CoreCommand::Shutdown).await.unwrap();

    let phone = tempfile::tempdir().unwrap();
    let mut node = start(phone.path(), Setup { imports: false, ..Setup::default() });
    ready(&mut node).await;
    assert!(!node.imports.available);
    node.handle.send(CoreCommand::ImportAdd(vec![request(&file, remote(ProviderId::Groq, GROQ_ASR), None)])).await.unwrap();
    assert_eq!(error(&mut node).await, voltip_core::import::ImportError::Unavailable.to_string());
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// docs/dictation.md §31: an installed local model, or a provider the settings do not recognise
/// with, can import.
#[tokio::test]
async fn an_import_runs_on_an_installed_local_model() {
    let dir = tempfile::tempdir().unwrap();
    let mut node = start(dir.path(), Setup { models: true, ..Setup::default() });
    ready(&mut node).await;
    let file = media(dir.path(), "课堂.webm");
    let [id] = added(&mut node, vec![request(&file, RecognitionChoice::Local { id: LOCAL_ID.into() }, None)]).await[..] else { panic!() };
    let job = finished(&mut node, id).await;
    let ImportState::Done { entry, .. } = job.state else { panic!("{job:?}") };
    while node.recent.first().map(|e| e.id) != Some(entry) {
        next(&mut node).await;
    }
    assert_eq!(node.recent[0].asr_model, LOCAL_NAME);
    assert_eq!((node.fakes.local_asr.calls(), node.fakes.saved_asr.calls(), node.fakes.builtin_asr.calls()), (3, 0, 0));

    // A model the library does not have installed is refused.
    node.handle.send(CoreCommand::ImportAdd(vec![request(&file, RecognitionChoice::Local { id: "sensevoice".into() }, None)])).await.unwrap();
    let why = error(&mut node).await;
    assert!(why.contains("sensevoice"), "{why}");
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
}

/// docs/dictation.md §31: the decoded files wait in `<data_dir>/imports` only while the app runs:
/// what an earlier run left goes at the start, and a stop removes them.
#[tokio::test]
async fn the_decoded_files_go_at_start_and_at_stop() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("imports")).unwrap();
    std::fs::write(dir.path().join("imports").join("left-behind.pcm"), [0u8; 64]).unwrap();
    let mut node = start(dir.path(), Setup { decoder: FakeMediaDecoder::held(25), ..Setup::default() });
    ready(&mut node).await;
    assert_eq!(decoded_files(dir.path()), 0, "an earlier run's file went at the start");
    let file = media(dir.path(), "长视频.mp4");
    let [id] = added(&mut node, vec![request(&file, remote(ProviderId::Groq, GROQ_ASR), None)]).await[..] else { panic!() };
    queue_until(&mut node, "the decoding", |v| v.jobs.iter().any(|j| j.id == id && matches!(j.state, ImportState::Decoding { .. }))).await;
    node.handle.send(CoreCommand::Shutdown).await.unwrap();
    // The core stops: its events end.
    while tokio::time::timeout(STEP, node.events.recv()).await.expect("the core stops within 10 s").is_some() {}
    assert_eq!(decoded_files(dir.path()), 0);
    assert!(file.exists());
}
