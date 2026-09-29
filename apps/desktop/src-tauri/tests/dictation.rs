#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The desktop's real dictation ports against their in-process stand-ins: the audio crate's
//! `FakeBackend`, wiremock for the two HTTP clients, recording doubles for `voltip-inject`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::json;
use voltip_asr::AsrConfig;
use voltip_asr_local::LocalTranscriber;
use voltip_audio::{FakeBackend, Signal};
use voltip_core::dictation::{
    AudioSource, CaptureOptions, DictationError, Injector, LevelFrame, LivePcm, RefineContext, RefineHints, Refiner, Transcriber, Via,
};
use voltip_core::engines::{BuiltIn, EngineSettings, ProviderId, ProviderSettings, ResolvedEngines, ServiceKind, UserSecrets};
use voltip_core::{InjectMode, dictation::wav};
use voltip_desktop_lib::audio::AudioHub;
use voltip_desktop_lib::dictation::{
    HttpRefiner, HttpTranscriber, NativeInjector, RecorderAudioSource, Unconfigured, build_clients, engine_factory, to_audio_frame, to_core_frame,
};
use voltip_refine::RefineConfig;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Both services on the custom endpoint (`None` URL = not configured yet).
fn resolved(asr_url: Option<&str>, refine_url: Option<&str>, asr_token: Option<&str>, refine_key: Option<&str>, inject: InjectMode) -> ResolvedEngines {
    let custom = ProviderSettings {
        asr_url: asr_url.map(str::to_owned),
        asr_model: Some("whisper".into()),
        llm_url: refine_url.map(str::to_owned),
        llm_model: Some("m".into()),
    };
    let settings = EngineSettings {
        asr_provider: ProviderId::Custom,
        llm_provider: ProviderId::Custom,
        providers: [(ProviderId::Custom, custom)].into(),
        language: Some("zh".into()),
        inject,
        ..EngineSettings::default()
    };
    let mut secrets = UserSecrets::default();
    secrets.set(ProviderId::Custom, ServiceKind::Asr, asr_token.map(str::to_owned));
    secrets.set(ProviderId::Custom, ServiceKind::Llm, refine_key.map(str::to_owned));
    ResolvedEngines::resolve(&settings, &secrets, &BuiltIn::EMPTY)
}

#[test]
fn recorder_audio_source_records_through_the_fake_backend() {
    let meter_backend: Arc<dyn voltip_audio::Backend + Send + Sync> = Arc::new(FakeBackend::new());
    let hub = Arc::new(AudioHub::with_opener(Box::new(move |device_id, sink| {
        let config = voltip_audio::MeterConfig { device_id, ..voltip_audio::MeterConfig::default() };
        voltip_audio::Meter::start_with(meter_backend.as_ref(), config, move |f| sink(f)).map_err(|e| e.to_string())
    })));
    // A webview meter subscriber registered before the capture: it must be fed by the recorder.
    let meter_frames = Arc::new(Mutex::new(0usize));
    let mf = meter_frames.clone();
    let _ = hub.subscribe(None, Arc::new(move |_| *mf.lock() += 1));
    let backend = Arc::new(FakeBackend::new().with_signal(Signal::Sine { frequency_hz: 440.0, amplitude: 0.5 }));
    let source = RecorderAudioSource::with_backend(backend.clone(), hub.clone());
    let frames = Arc::new(Mutex::new(Vec::<LevelFrame>::new()));
    let sink = frames.clone();
    let ready = Arc::new(AtomicBool::new(false));
    let flag = ready.clone();
    let mut capture =
        source.start(None, Box::new(move |f| sink.lock().push(f)), Box::new(move || flag.store(true, Ordering::SeqCst)), CaptureOptions::default()).unwrap();
    assert!(hub.following_capture(), "the hub hands the microphone to the recorder");
    assert!(capture.live_pcm().is_none(), "no tap unless asked");
    // Let the fake stream produce some audio (it paces itself on a thread).
    std::thread::sleep(Duration::from_millis(400));
    assert!(ready.load(Ordering::SeqCst), "the first samples marked the capture ready");
    let recording = capture.stop().unwrap();
    assert!(!hub.following_capture(), "the hub gets the microphone back when the capture stops");
    assert!(*meter_frames.lock() > 0, "meter subscribers received the recorder's frames");
    assert_eq!(recording.sample_rate_hz, 16_000, "resampled to the ASR rate");
    assert!(recording.duration_ms >= 100, "{}", recording.duration_ms);
    assert!(!wav::is_silent(&recording.wav));
    assert!(wav::pcm_data(&recording.wav).is_some());
    assert!(!frames.lock().is_empty(), "levels reached the core-side callback");
    let f = frames.lock()[0];
    assert_eq!(f.sample_rate_hz, 48_000);
    // A named device that is connected is honoured; one that is not records from the default
    // input instead, and the choice stays in the settings until it is back (2026-09-28).
    assert!(source.start(Some(voltip_audio::FAKE_USB_ID), Box::new(|_| {}), Box::new(|| {}), CaptureOptions::default()).is_ok());
    assert_eq!(backend.opened_with().last(), Some(&Some(voltip_audio::FAKE_USB_ID.to_owned())));
    assert!(source.start(Some("fake:missing"), Box::new(|_| {}), Box::new(|| {}), CaptureOptions::default()).is_ok());
    assert_eq!(backend.opened_with().last(), Some(&None), "an unplugged choice opens the default input");
    // No device at all.
    let none = RecorderAudioSource::with_backend(Arc::new(FakeBackend::new().without_devices()), hub.clone());
    assert!(matches!(none.start(None, Box::new(|_| {}), Box::new(|| {}), CaptureOptions::default()).err().unwrap(), DictationError::Audio(_)));
    assert!(!hub.following_capture(), "a failed open returns the microphone to the meter");
    // Frame conversion is lossless both ways.
    let core = to_core_frame(voltip_audio::LevelFrame { rms_dbfs: -12.5, peak_dbfs: -3.0, clipping: true, sample_rate_hz: 44_100, channels: 2, seq: 9 });
    assert_eq!(core, LevelFrame { rms_dbfs: -12.5, peak_dbfs: -3.0, clipping: true, sample_rate_hz: 44_100, channels: 2, seq: 9 });
    assert_eq!(to_core_frame(to_audio_frame(core)), core);
}

#[tokio::test]
async fn http_transcriber_maps_results_and_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/transcriptions"))
        .and(header("authorization", "Bearer tok"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "text": "  你好，世界 " })))
        .mount(&server)
        .await;
    let t = HttpTranscriber::new(AsrConfig::new(server.uri(), "Qwen/Qwen3-ASR-1.7B").with_token(Some("tok".into()))).unwrap();
    let wav = wav::encode_pcm16(&[1000; 1600], 16_000);
    let out = t.transcribe(&wav, Some("zh"), &["Voltip".to_owned(), "good idea".to_owned()]).await.unwrap();
    assert_eq!(out.text, "你好，世界");
    // The dictionary terms travel as the `prompt` field (docs/dictation.md §16.3); none, no field.
    t.transcribe(&wav, Some("zh"), &[]).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    let with = String::from_utf8_lossy(&requests[0].body).into_owned();
    assert!(with.contains("name=\"prompt\"\r\n\r\nVoltip, good idea\r\n"), "{with}");
    assert!(!String::from_utf8_lossy(&requests[1].body).contains("name=\"prompt\""));
    // Wrong token → 401 → `Asr(...)` with the client's explanation.
    let t = HttpTranscriber::new(AsrConfig::new(server.uri(), "Qwen/Qwen3-ASR-1.7B").with_token(Some("bad".into()))).unwrap();
    server.reset().await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(401)).mount(&server).await;
    let err = t.transcribe(&wav, None, &[]).await.unwrap_err();
    assert!(matches!(&err, DictationError::Asr(m) if m.contains("credentials")), "{err}");
    // An unusable configuration fails at construction.
    assert!(matches!(HttpTranscriber::new(AsrConfig::new(server.uri(), "")), Err(DictationError::Asr(_))));
}

#[tokio::test]
async fn http_refiner_maps_results_and_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer gsk"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "qwen/qwen3.8-27b",
            "choices": [{ "message": { "role": "assistant", "content": "你好，世界。" } }]
        })))
        .mount(&server)
        .await;
    let r = HttpRefiner::new(RefineConfig::new(format!("{}/v1", server.uri()), "qwen/qwen3.8-27b").with_api_key(Some("gsk".into()))).unwrap();
    let hints = RefineHints { glossary: vec!["世界".to_owned()], language: Some("zh".into()), ..RefineHints::default() };
    let out = r.refine("你好 世界", &hints).await.unwrap();
    assert_eq!(out.text, "你好，世界。");
    assert_eq!(out.model, "qwen/qwen3.8-27b");
    // The glossary is the system prompt's last block (docs/dictation.md §16.3), the language hint above it.
    let body: serde_json::Value = serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
    let system = body["messages"][0]["content"].as_str().unwrap();
    assert!(system.ends_with("\n- 世界") && system.contains("语言代码：zh"), "{body}");
    assert!(!system.contains("听写场景") && !system.contains("场景要求"), "no context, no scene blocks: {system}");
    // docs/dictation.md §18.5: the take's style and context reach the system message, the user
    // message stays the text; a title the core did not pass never appears.
    let hints = RefineHints {
        style: voltip_core::RefineStyle::Punctuation,
        context: RefineContext { app_name: Some("Slack".into()), window_title: None, instruction: Some("口语化，句末不加句号".into()) },
        ..RefineHints::default()
    };
    r.refine("好的呀", &hints).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&server.received_requests().await.unwrap()[1].body).unwrap();
    let system = body["messages"][0]["content"].as_str().unwrap();
    assert!(system.contains("只处理标点") && system.contains("\n当前应用：Slack") && !system.contains("窗口标题"), "{system}");
    assert!(system.ends_with("\n口语化，句末不加句号"), "{system}");
    assert_eq!(body["messages"][1]["content"], "好的呀");
    server.reset().await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(429)).mount(&server).await;
    let err = r.refine("x", &RefineHints::default()).await.unwrap_err();
    assert!(matches!(&err, DictationError::Refine(m) if m.contains("rate limited")), "{err}");
    assert!(matches!(HttpRefiner::new(RefineConfig::new(server.uri(), "")), Err(DictationError::Refine(_))));
    // The stand-in for a client that could not be built.
    let u = Unconfigured("未配置".into());
    assert_eq!(u.transcribe(b"", None, &[]).await.unwrap_err(), DictationError::Asr("未配置".into()));
    assert_eq!(u.refine("x", &RefineHints::default()).await.unwrap_err(), DictationError::Refine("未配置".into()));
}

/// Records what it was asked to deliver and answers with a fixed outcome.
struct Recording {
    outcome: Result<voltip_inject::Injection, voltip_inject::InjectError>,
    calls: AtomicUsize,
}

impl voltip_inject::Injector for Recording {
    fn inject(&self, _text: &str) -> Result<voltip_inject::Injection, voltip_inject::InjectError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.outcome.clone()
    }
    fn describe(&self) -> &'static str {
        "recording"
    }
}

#[test]
fn native_injector_switches_on_the_mode_the_factory_saw() {
    let mode = Arc::new(Mutex::new(InjectMode::Paste));
    let paste = Box::new(Recording { outcome: Ok(voltip_inject::Injection::pasted(3)), calls: AtomicUsize::new(0) });
    let note = voltip_inject::InjectNote::new(voltip_inject::FallbackCode::Other, "clipboard only");
    let clipboard = Box::new(Recording { outcome: Ok(voltip_inject::Injection::clipboard(3, Some(note))), calls: AtomicUsize::new(0) });
    let injector = NativeInjector::with(mode.clone(), paste, clipboard);
    assert_eq!(injector.inject("abc").unwrap().via, Via::Paste);
    let factory = engine_factory(mode.clone(), LocalTranscriber::new(std::env::temp_dir().join("voltip-test-models")), None);
    factory(&resolved(None, None, None, None, InjectMode::ClipboardOnly));
    assert_eq!(*mode.lock(), InjectMode::ClipboardOnly, "the factory records the mode");
    let out = injector.inject("abc").unwrap();
    assert_eq!(out.via, Via::Clipboard);
    assert_eq!(out.note, Some(voltip_core::dictation::InjectNote::other("clipboard only")));
    factory(&resolved(None, None, None, None, InjectMode::Paste));
    assert_eq!(injector.inject("abc").unwrap().via, Via::Paste);
    // Failures map to `Inject` with the crate's message.
    let failing = NativeInjector::with(
        Arc::new(Mutex::new(InjectMode::Paste)),
        Box::new(Recording { outcome: Err(voltip_inject::InjectError::Clipboard("busy".into())), calls: AtomicUsize::new(0) }),
        Box::new(Recording { outcome: Ok(voltip_inject::Injection::pasted(0)), calls: AtomicUsize::new(0) }),
    );
    assert_eq!(failing.inject("x").unwrap_err(), DictationError::Inject("clipboard: busy".into()));
}

#[tokio::test]
async fn build_clients_follows_the_resolved_configuration() {
    let local = LocalTranscriber::new(std::env::temp_dir().join("voltip-test-models"));
    // No URLs: the transcriber explains instead of failing to build; no clean-up endpoint: no refiner.
    let (t, r) = build_clients(&resolved(None, None, None, None, InjectMode::Paste), &local);
    assert!(r.is_none());
    let err = t.transcribe(b"RIFF", None, &[]).await.unwrap_err();
    assert!(matches!(&err, DictationError::Asr(m) if m.contains("未配置")), "{err}");
    // A URL and a key: real clients that talk to the configured hosts.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/transcriptions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "text": "ok" })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "choices": [{ "message": { "content": "ok。" } }] })))
        .mount(&server)
        .await;
    let (t, r) = build_clients(&resolved(Some(&server.uri()), Some(&format!("{}/v1", server.uri())), Some("tok"), Some("gsk"), InjectMode::Paste), &local);
    let wav = wav::encode_pcm16(&[1000; 1600], 16_000);
    assert_eq!(t.transcribe(&wav, None, &[]).await.unwrap().text, "ok");
    assert_eq!(r.unwrap().refine("ok", &RefineHints::default()).await.unwrap().text, "ok。");
    // A configuration the clients refuse is reported on use, not swallowed.
    let (t, r) = build_clients(&resolved(Some("not a url"), Some("also not a url"), None, Some("gsk"), InjectMode::Paste), &local);
    assert!(matches!(t.transcribe(&wav, None, &[]).await.unwrap_err(), DictationError::Asr(m) if m.contains("配置无效")));
    assert!(matches!(r.unwrap().refine("x", &RefineHints::default()).await.unwrap_err(), DictationError::Refine(m) if m.contains("配置无效")));
}

/// Local mode (docs/dictation.md §10): the factory hands out the shared `LocalTranscriber` pointed
/// at the selected model, the cloud URL / token play no part, and a model that is not on disk is
/// reported with the documented text instead of a network error.
#[tokio::test]
async fn build_clients_local_mode_selects_the_model_and_needs_no_endpoint() {
    let root = tempfile::tempdir().unwrap();
    let local = LocalTranscriber::new(root.path());
    let settings = EngineSettings {
        asr_provider: ProviderId::Local,
        local_model: Some("paraformer-zh".into()),
        providers: [(ProviderId::Custom, ProviderSettings { asr_url: Some("https://asr.example.test".into()), ..Default::default() })].into(),
        ..EngineSettings::default()
    };
    let mut secrets = UserSecrets::default();
    secrets.set(ProviderId::Custom, ServiceKind::Asr, Some("tok".into()));
    let engines = ResolvedEngines::resolve(&settings, &secrets, &BuiltIn::EMPTY);
    assert!(engines.is_local() && engines.asr_remote.is_none(), "no endpoint and no key on-device");
    let (t, _) = build_clients(&engines, &local);
    let wav = wav::encode_pcm16(&[1000; 1600], 16_000);
    let err = t.transcribe(&wav, Some("zh"), &[]).await.unwrap_err();
    assert_eq!(err, DictationError::Asr("本地模型未下载：轻量 · 中文".into()), "no library on disk: the message names the model");
    // The default model when the settings name none; the wiring's store scans the same root.
    let engines = ResolvedEngines::resolve(&EngineSettings { asr_provider: ProviderId::Local, ..EngineSettings::default() }, &secrets, &BuiltIn::EMPTY);
    let (t, _) = build_clients(&engines, &local);
    let err = t.transcribe(&wav, None, &[]).await.unwrap_err();
    assert!(matches!(&err, DictationError::Asr(m) if m.contains("本地模型未下载")), "{err}");
    let ports = voltip_desktop_lib::dictation::ports_with_backend(Arc::new(FakeBackend::new()), Arc::new(AudioHub::default()), root.path().to_path_buf());
    let models = ports.dictation.models.as_ref().expect("the desktop wires a model library").scan();
    assert_eq!(
        models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["qwen3-asr-0.6b", "qwen3-asr-1.7b", "sense-voice-small", "paraformer-zh", "zipformer-stream-zh-en"]
    );
    assert!(models.iter().all(|m| !m.state.is_installed()));
    assert_eq!(models.iter().filter(|m| m.is_streaming()).map(|m| m.id.as_str()).collect::<Vec<_>>(), ["zipformer-stream-zh-en"]);
    let (t, _) = (ports.dictation.factory)(&engines);
    assert!(matches!(t.transcribe(&wav, None, &[]).await.unwrap_err(), DictationError::Asr(m) if m.contains("均衡")));
    // The streaming port is wired over the same library: nothing installed → the documented refusal.
    let streaming = ports.dictation.streaming.as_ref().expect("the desktop wires the streaming preview");
    assert_eq!(streaming.open(None).err(), Some(DictationError::Asr("实时识别模型未下载：实时预览".into())));
    streaming.warm();
}

/// docs/dictation.md §10.7 through the desktop's own factory: the engine the core builds at start
/// warms the selected local model, so it is in memory before the first key press, and a settings
/// change to another model warms that one (one model in memory at a time).
#[tokio::test]
async fn the_engine_warms_the_selected_local_model_before_the_first_take() {
    use voltip_asr_local::{CATALOGUE, CATALOGUE_VERSION, Compute, MANIFEST_FILE, Manifest, ModelEntry, Recognizer, RecognizerLoader};
    struct Silent;
    impl Recognizer for Silent {
        fn transcribe(&mut self, _sample_rate: u32, _samples: &[f32]) -> Result<String, String> {
            Ok(String::new())
        }
    }
    struct Counting(Arc<Mutex<Vec<String>>>);
    impl RecognizerLoader for Counting {
        fn load(&self, entry: &ModelEntry, _dir: &std::path::Path, _language: Option<&str>, _compute: &Compute) -> Result<Box<dyn Recognizer>, String> {
            self.0.lock().push(entry.id.to_owned());
            Ok(Box::new(Silent))
        }
    }
    let root = tempfile::tempdir().unwrap();
    // Installed as the store leaves it: right-sized files and the manifest.
    for id in ["paraformer-zh", "sense-voice-small"] {
        let e = voltip_asr_local::catalogue::entry(id).unwrap();
        let dir = root.path().join(id);
        std::fs::create_dir_all(&dir).unwrap();
        for f in e.files() {
            std::fs::File::create(dir.join(f.name)).unwrap().set_len(f.size).unwrap();
        }
        let manifest = Manifest {
            id: id.into(),
            version: CATALOGUE_VERSION,
            downloaded_at: 1,
            files: e.files().iter().map(|f| (f.name.to_owned(), f.sha256.to_owned())).collect(),
        };
        std::fs::write(dir.join(MANIFEST_FILE), serde_json::to_vec(&manifest).unwrap()).unwrap();
    }
    let loads = Arc::new(Mutex::new(Vec::new()));
    let local = LocalTranscriber::with_loader(root.path(), CATALOGUE, Arc::new(Counting(loads.clone())));
    let mut ports = voltip_core::dictation::fakes::ports();
    ports.factory = engine_factory(Arc::new(Mutex::new(InjectMode::Paste)), local.clone(), None);
    let on = |id: &str| {
        ResolvedEngines::resolve(
            &EngineSettings { asr_provider: ProviderId::Local, local_model: Some(id.into()), ..EngineSettings::default() },
            &UserSecrets::default(),
            &BuiltIn::EMPTY,
        )
    };
    let wait_loaded = |id: &str| {
        let deadline = Instant::now() + Duration::from_secs(5);
        while local.loaded().as_deref() != Some(id) {
            assert!(Instant::now() < deadline, "{id} was not warmed within 5 s (in memory: {:?})", local.loaded());
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let (levels, _) = tokio::sync::broadcast::channel(8);
    let (mut engine, _internal) = voltip_core::dictation::DictationEngine::new(ports, &on("paraformer-zh"), levels);
    wait_loaded("paraformer-zh");
    engine.configure(&on("sense-voice-small"));
    wait_loaded("sense-voice-small");
    assert_eq!(*loads.lock(), ["paraformer-zh", "sense-voice-small"]);
}

/// The live tap (docs/dictation.md §11) through the desktop's audio source: with `live` the capture
/// hands out a `LivePcm` once, fed with 16 kHz mono audio from the recorder's tap, and it closes
/// with the capture. `on_ready` fires from the first chunk.
#[test]
fn recorder_audio_source_live_tap_streams_16k_and_closes_on_stop() {
    let hub = Arc::new(AudioHub::with_opener(Box::new(|_, _| Err("no audio device on this runtime".into()))));
    let source =
        RecorderAudioSource::with_backend(Arc::new(FakeBackend::new().with_signal(Signal::Sine { frequency_hz: 1000.0, amplitude: 0.5 })), hub.clone());
    let ready = Arc::new(AtomicBool::new(false));
    let flag = ready.clone();
    let mut capture = source.start(None, Box::new(|_| {}), Box::new(move || flag.store(true, Ordering::SeqCst)), CaptureOptions::LIVE).unwrap();
    let mut tap = capture.live_pcm().expect("a live tap was requested");
    assert!(capture.live_pcm().is_none(), "take-once");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut live = Vec::new();
    let mut buf = vec![0.0_f32; 4096];
    while live.len() < 8000 {
        assert!(Instant::now() < deadline, "the tap delivered only {} samples in 5 s", live.len());
        let n = tap.read(&mut buf);
        if n == 0 {
            std::thread::sleep(Duration::from_millis(5));
        }
        live.extend_from_slice(&buf[..n]);
    }
    assert!(ready.load(Ordering::SeqCst));
    assert!(!tap.overrun() && !tap.is_closed());
    let recording = capture.stop().unwrap();
    assert!(tap.is_closed(), "stop closes the tap");
    assert!(!hub.following_capture());
    assert_eq!(recording.sample_rate_hz, 16_000);
    // 1 kHz at −6 dBFS, 16 samples per period: the tap carries the same signal the take does.
    let core = &live[1600..];
    let rms = (core.iter().map(|&s| f64::from(s) * f64::from(s)).sum::<f64>() / core.len() as f64).sqrt();
    assert!((rms - 0.3536).abs() < 0.01, "rms {rms}");
    let crossings = core.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
    let expected = core.len() / 8;
    assert!(crossings.abs_diff(expected) <= expected / 50, "crossings {crossings} vs {expected}");
    // The core-facing wrapper over a detached consumer: closed at once, nothing to read.
    let (producer, consumer) = voltip_audio::live::live_tap(&voltip_audio::LiveTapConfig::default(), 48_000).unwrap();
    drop(producer);
    let mut wrapped = voltip_desktop_lib::dictation::LiveTap(consumer);
    assert!(wrapped.is_closed() && !wrapped.overrun());
    assert_eq!(wrapped.read(&mut buf), 0);
}

/// The production wiring is lazy: building it opens no device, clipboard or socket, so it can be
/// constructed headlessly; the factory it carries behaves like [`build_clients`].
#[tokio::test]
async fn production_ports_build_headlessly() {
    let ports = voltip_desktop_lib::dictation::production_ports(std::env::temp_dir().join("voltip-test-models"));
    assert!(ports.dictation.models.is_some(), "the desktop always carries the model library");
    let (t, r) = (ports.dictation.factory)(&resolved(None, None, None, None, InjectMode::ClipboardOnly));
    assert!(r.is_none());
    assert!(matches!(t.transcribe(b"RIFF", None, &[]).await.unwrap_err(), DictationError::Asr(m) if m.contains("未配置")));
    assert!(format!("{:?}", ports.dictation).contains("DictationPorts"));
    assert!(!ports.hub.is_running(), "no device is opened until a subscriber asks");
}

/// Regression (2026-09-25): the home meter and the dictation recorder used to fight
/// over the microphone, and the home meter froze after the first dictation. The hub now switches
/// the source underneath the subscribers and reopens the device meter when the capture ends.
#[test]
fn meter_subscribers_survive_a_dictation_capture_and_get_the_device_back() {
    let backend: Arc<dyn voltip_audio::Backend + Send + Sync> = Arc::new(FakeBackend::new().with_signal(Signal::Sine { frequency_hz: 440.0, amplitude: 0.5 }));
    let opener_backend = backend.clone();
    let hub = Arc::new(AudioHub::with_opener(Box::new(move |device_id, sink| {
        let config = voltip_audio::MeterConfig { device_id, ..voltip_audio::MeterConfig::default() };
        voltip_audio::Meter::start_with(opener_backend.as_ref(), config, move |f| sink(f)).map_err(|e| e.to_string())
    })));
    let got = Arc::new(Mutex::new(Vec::<u64>::new()));
    let sink = got.clone();
    let id = hub.subscribe(None, Arc::new(move |f| sink.lock().push(f.seq))).unwrap();
    assert!(hub.is_running() && !hub.following_capture());
    std::thread::sleep(Duration::from_millis(250));
    let before_capture = got.lock().len();
    assert!(before_capture > 0, "device meter delivers frames");

    let ports = voltip_desktop_lib::dictation::ports_with_backend(backend, hub.clone(), std::env::temp_dir().join("voltip-test-models"));
    let capture = ports.dictation.audio.start(None, Box::new(|_| {}), Box::new(|| {}), CaptureOptions::default()).unwrap();
    assert!(hub.following_capture());
    std::thread::sleep(Duration::from_millis(250));
    let during = got.lock().len();
    assert!(during > before_capture, "frames keep flowing from the recorder while it records");
    capture.stop().unwrap();
    assert!(hub.is_running() && !hub.following_capture(), "device meter reopened for the surviving subscriber");
    std::thread::sleep(Duration::from_millis(250));
    assert!(got.lock().len() > during, "the home meter is not frozen after a dictation");
    hub.unsubscribe(id);
    assert!(!hub.is_running());
}
