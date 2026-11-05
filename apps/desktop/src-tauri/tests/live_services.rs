#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Every realtime recognition service through the desktop's real wiring (docs/dictation.md §11.9,
//! §3.15): the settings select the service, the production ports build its client (`voltip-cloud`
//! over `voltip-asr`), the fake audio backend records a tone, and a stand-in of the service's own
//! protocol answers on 127.0.0.1. While the take is spoken the pill shows the service's sentence so
//! far and then its finished sentence; the take's text is the stream's, with no second recognition
//! of the recording; the history names the service and its model; the usage record counts the
//! take. A stream that ends without text, or whose connection drops, still ends the take with the
//! whole recording's text, and the history says why. No real service is contacted: a test build
//! has no built-in service, and every address is the stand-in's.
//!
//! Model Studio's protocols follow its address (`*.aliyuncs.com`, docs/dictation.md §3.4), which a
//! stand-in cannot have: its settings keep the public address, so the core decides for the take as
//! it does for a user, and the client for them is Model Studio's own at the stand-in's address,
//! with nothing else that could ask the service.

use std::future::Future;
use std::io::{Read as _, Write as _};
use std::panic::AssertUnwindSafe;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use futures_util::{SinkExt as _, StreamExt as _};
use serde_json::{Value, json};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder, mock_context, noop_assets};
use tauri::webview::InvokeRequest;
use tauri::{Listener as _, Manager as _, WebviewWindow};
use tokio::net::TcpStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::HeaderValue;
use voltip_cloud::dashscope::DashscopeTranscriber;
use voltip_core::dictation::{Transcriber, fakes};
use voltip_core::engines::{EngineSettings, ProviderSettings};
use voltip_core::providers::key_entry;
use voltip_core::ui::{UI_EVENT_NAME, UiEvent, UiState};
use voltip_core::{CoreConfig, DictationPhase, LiveSource, OutputMode, ProviderId, ServiceKind, Settings, SettingsStore};
use voltip_desktop_lib::{ShellOptions, build_app};
use voltip_identity::{MemorySecretStore, SecretStore as _};
use voltip_tauri_bridge::Bridge;

/// How long one step of a take may take.
const STEP: Duration = Duration::from_secs(60);
/// The sentence being spoken, as every stand-in first says it.
const PARTIAL: &str = "你好";
/// The first sentence, finished while the take is spoken.
const FIRST: &str = "你好，世界。";
/// The last sentence, the stand-in's answer to the end of the audio.
const LAST: &str = "今天天气不错。";
/// The take's text from the stream.
const TEXT: &str = "你好，世界。今天天气不错。";
/// The text a recorded take of the whole recording gets.
const WHOLE: &str = "整段识别的文字。";
/// Audio (bytes of 16 kHz 16-bit mono) after which a stand-in says the sentence being spoken:
/// 200 ms.
const PARTIAL_AFTER: usize = 6_400;
/// The audio after which it finishes the first sentence: 800 ms.
const SENTENCE_AFTER: usize = 25_600;
/// The 豆包语音 stand-in's log id.
const LOGID: &str = "20261106100000TESTLOGID";

type Socket = WebSocketStream<TcpStream>;

/// What a stand-in saw, all its connections together.
#[derive(Debug, Default)]
struct Saw {
    /// The path of each connection, in order.
    paths: Vec<String>,
    /// Bytes of audio it received.
    audio: usize,
}

type Shared = Arc<Mutex<Saw>>;

/// A realtime service as the settings select it.
struct Service {
    provider: ProviderId,
    model: &'static str,
    /// What follows the stand-in's origin in the service's base URL.
    base_path: &'static str,
    /// The stored credential, as the settings page saves it.
    credential: &'static str,
}

/// What a take showed and how it ended.
struct Take {
    /// Every phase the webview was told, in order.
    phases: Vec<DictationPhase>,
    /// The state once the take is in the history and the usage record.
    end: UiState,
}

fn request(cmd: &str, args: Value) -> InvokeRequest {
    InvokeRequest {
        cmd: cmd.into(),
        callback: CallbackFn(0),
        error: CallbackFn(1),
        url: if cfg!(any(windows, target_os = "android")) { "http://tauri.localhost" } else { "tauri://localhost" }.parse().unwrap(),
        body: InvokeBody::Json(args),
        headers: Default::default(),
        invoke_key: INVOKE_KEY.to_owned(),
    }
}

fn invoke(webview: &WebviewWindow<MockRuntime>, cmd: &str, args: Value) -> Result<Value, Value> {
    get_ipc_response(webview, request(cmd, args)).map(|body| body.deserialize::<Value>().unwrap())
}

fn core_state(webview: &WebviewWindow<MockRuntime>) -> UiState {
    serde_json::from_value(invoke(webview, "core_state", json!({})).unwrap()).unwrap()
}

/// The state once `ready` holds for it and what the stand-in saw.
fn wait_state(webview: &WebviewWindow<MockRuntime>, saw: &Shared, what: &str, mut ready: impl FnMut(&UiState, &Saw) -> bool) -> UiState {
    let deadline = Instant::now() + STEP;
    loop {
        let st = core_state(webview);
        if ready(&st, &saw.lock().unwrap()) {
            return st;
        }
        assert!(
            Instant::now() < deadline,
            "{what}: not reached; the take is {:?}, the engines {:?} / {:?} / {:?}, the stand-in saw {:?}",
            st.dictation.phase,
            st.engines.asr_provider,
            st.engines.live_source,
            st.engines.asr_issue,
            saw.lock().unwrap()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A stand-in on 127.0.0.1 whose connections `answer` serves (with the connection's index and
/// path): its `http://` origin and what it saw. Its handshakes name [`LOGID`] as their log id.
fn stand_in<F, Fut>(runtime: &tokio::runtime::Runtime, answer: F) -> (String, Shared)
where
    F: Fn(Socket, usize, String, Shared) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let listener = runtime.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let saw = Shared::default();
    let shared = Arc::clone(&saw);
    let answer = Arc::new(answer);
    runtime.spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let (answer, saw) = (Arc::clone(&answer), Arc::clone(&shared));
            tokio::spawn(async move {
                let path = Arc::new(Mutex::new(String::new()));
                let record = Arc::clone(&path);
                // tungstenite's handshake callback returns its `ErrorResponse` by value.
                #[allow(clippy::result_large_err)]
                let callback = move |request: &Request, mut response: Response| -> Result<Response, ErrorResponse> {
                    request.uri().path().clone_into(&mut record.lock().unwrap());
                    response.headers_mut().insert("x-tt-logid", HeaderValue::from_static(LOGID));
                    Ok(response)
                };
                let Ok(socket) = tokio_tungstenite::accept_hdr_async(tcp, callback).await else { return };
                let path = path.lock().unwrap().clone();
                let index = {
                    let mut saw = saw.lock().unwrap();
                    saw.paths.push(path.clone());
                    saw.paths.len() - 1
                };
                answer(socket, index, path, saw).await;
            });
        }
    });
    (origin, saw)
}

/// One take on `service`, answered by `answer`: started once the engines say the service streams
/// the preview, stopped once `ready` holds.
fn take<F, Fut>(service: &Service, answer: F, ready: impl Fn(&UiState, &Saw) -> bool + Send + 'static) -> (Take, Shared)
where
    F: Fn(Socket, usize, String, Shared) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let (origin, saw) = stand_in(&runtime, answer);
    let dir = tempfile::tempdir().unwrap();
    let provider = service.provider;
    // Model Studio keeps its public address (see the head of this file).
    let asr_url = (provider != ProviderId::Aliyun).then(|| format!("{origin}{}", service.base_path));
    let settings = ProviderSettings { asr_url, asr_model: Some(service.model.into()), ..Default::default() };
    let engines = EngineSettings { asr_provider: provider, providers: [(provider, settings)].into(), refine_enabled: false, ..EngineSettings::default() };
    SettingsStore::new(dir.path()).save(&Settings { relay_enabled: false, engines, ..Settings::default() }).unwrap();
    let mut config = CoreConfig::new(dir.path().to_path_buf());
    config.default_device_name = "Desk Test".into();
    let secrets = Arc::new(MemorySecretStore::new());
    secrets.set(key_entry(provider, ServiceKind::Asr).unwrap(), service.credential.as_bytes()).unwrap();
    let hub = Arc::new(voltip_desktop_lib::audio::AudioHub::with_opener(Box::new(|_, _| Err("no meter".into()))));
    let backend = Arc::new(voltip_audio::FakeBackend::new().with_signal(voltip_audio::Signal::Sine { frequency_hz: 440.0, amplitude: 0.3 }));
    let mut ports = voltip_desktop_lib::dictation::ports_with_backend(backend, hub, dir.path().join("models"), &voltip_net::Network::new());
    if provider == ProviderId::Aliyun {
        let config = voltip_asr::AsrConfig::new(format!("{origin}{}", service.base_path), service.model).with_token(Some(service.credential.into()));
        let realtime: Arc<dyn Transcriber> = Arc::new(DashscopeTranscriber::new(config, voltip_asr::DashscopeMode::Duplex).unwrap());
        ports.dictation.factory = Arc::new(move |_| (Arc::clone(&realtime), None));
        ports.dictation.service_probe = None;
    }
    ports.dictation.injector = Arc::new(fakes::FakeInjector::paste());
    // The foreground probe runs as it does on the desktop: the preview waits for its answer.
    ports.dictation.probe = Some(Arc::new(fakes::FakeProbe::app("code.exe", "Code", None)));
    let app = build_app(mock_builder(), config, secrets, ShellOptions::HEADLESS, ports).build(mock_context(noop_assets())).unwrap();
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
    let (tx, rx) = mpsc::channel::<String>();
    app.listen_any(UI_EVENT_NAME, move |event| {
        let _ = tx.send(event.payload().to_owned());
    });
    let handle = app.handle().clone();
    let seen = Arc::clone(&saw);
    let driver = std::thread::spawn(move || {
        let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let deadline = Instant::now() + STEP;
            while handle.try_state::<Bridge>().is_none() {
                assert!(Instant::now() < deadline, "the core did not start");
                std::thread::sleep(Duration::from_millis(20));
            }
            let st = wait_state(&webview, &seen, "the service's own stream", |s, _| {
                s.identity.is_some() && s.engines.asr_provider == provider && s.engines.live_source == Some(LiveSource::Stream)
            });
            assert!(!st.usage.builtin, "a test build has no built-in service to send audio to");
            assert_eq!(st.engines.effective_output_mode, OutputMode::StreamingFinal, "the stream's sentences are the take's text");
            assert_eq!(invoke(&webview, "dictation_start", json!({})), Ok(Value::Null));
            wait_state(&webview, &seen, "the moment to stop", ready);
            assert_eq!(invoke(&webview, "dictation_stop", json!({})), Ok(Value::Null));
            wait_state(&webview, &seen, "the end of the take", |s, _| s.dictation.phase.is_terminal());
            let end = wait_state(&webview, &seen, "the history and the usage record", |s, _| {
                !s.history_recent.is_empty() && s.usage.record.others_today.iter().any(|u| u.provider == provider && u.service == ServiceKind::Asr)
            });
            let phases = rx
                .try_iter()
                .filter_map(|payload| match serde_json::from_str::<UiEvent>(&payload) {
                    Ok(UiEvent::Dictation(status)) => Some(status.phase),
                    _ => None,
                })
                .collect();
            Take { phases, end }
        }));
        webview.close().unwrap();
        outcome
    });
    app.run(|_, _| {});
    let outcome = driver.join().unwrap();
    drop(runtime);
    match outcome {
        Ok(take) => (take, saw),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

/// Stop once the pill shows the first sentence and the stand-in has a second of audio, part of it
/// for the next sentence.
fn after_the_first_sentence(st: &UiState, saw: &Saw) -> bool {
    matches!(&st.dictation.phase, DictationPhase::Listening { live: Some(l), .. } if l.committed.first().is_some_and(|s| s.text == FIRST))
        && saw.audio >= SENTENCE_AFTER + PARTIAL_AFTER
}

/// The pill showed the service's sentence so far, then its finished sentence; the take's text is
/// the stream's; the history names the service and its model; the usage record counts the take.
fn assert_streamed(service: &Service, take: &Take) {
    let live: Vec<_> = take.phases.iter().filter_map(|p| if let DictationPhase::Listening { live: Some(l), .. } = p { Some(l) } else { None }).collect();
    assert!(live.iter().any(|l| l.committed.is_empty() && l.current == PARTIAL), "{:?}: the pill showed the sentence being spoken: {live:?}", service.provider);
    assert!(
        live.iter().any(|l| l.committed.first().is_some_and(|s| s.text == FIRST)),
        "{:?}: the pill showed the finished sentence: {live:?}",
        service.provider
    );
    assert!(live.iter().all(|l| l.degraded.is_none()), "{:?}: the preview never stopped: {live:?}", service.provider);
    match &take.end.dictation.phase {
        DictationPhase::Done { text, mode, live_error, .. } => {
            assert_eq!((text.as_str(), *mode, live_error), (TEXT, OutputMode::StreamingFinal, &None), "{:?}", service.provider);
        }
        other => panic!("{:?}: {other:?}", service.provider),
    }
    let entry = &take.end.history_recent[0];
    assert_eq!(
        (entry.asr_provider, entry.asr_model.as_str(), entry.mode, entry.live_error.as_deref(), entry.text.as_str()),
        (Some(service.provider), service.model, OutputMode::StreamingFinal, None, TEXT),
        "{:?}: the history names the service that streamed",
        service.provider
    );
    let used = take.end.usage.record.others_today.iter().find(|u| u.provider == service.provider && u.service == ServiceKind::Asr).unwrap();
    assert!(used.amount >= 1, "{:?}: the usage record counts the streamed audio: {used:?}", service.provider);
}

// ---- 豆包语音 ------------------------------------------------------------------------------

/// How the 豆包语音 stand-in answers a live session; a recorded take always gets [`WHOLE`].
#[derive(Clone, Copy, PartialEq, Eq)]
enum Volc {
    Speaks,
    /// It answers without any text.
    Silent,
    /// It drops the connection after the first sentence.
    Drops,
}

/// A client frame of the binary protocol: its message type, its flags and its payload.
fn volc_frame(bytes: &[u8]) -> (u8, u8, Vec<u8>) {
    let size = u32::from_be_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let mut payload = Vec::new();
    flate2::read::GzDecoder::new(&bytes[8..8 + size]).read_to_end(&mut payload).unwrap();
    (bytes[1] >> 4, bytes[1] & 0x0F, payload)
}

/// A full server response: header, the sequence number when the flags say one follows, size,
/// gzipped JSON.
fn volc_response(flags: u8, sequence: i32, payload: &Value) -> Message {
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(payload.to_string().as_bytes()).unwrap();
    let body = gz.finish().unwrap();
    let mut frame = vec![0x11, 0x90 | flags, 0x11, 0x00];
    if flags & 1 != 0 {
        frame.extend_from_slice(&sequence.to_be_bytes());
    }
    frame.extend_from_slice(&u32::try_from(body.len()).unwrap().to_be_bytes());
    frame.extend_from_slice(&body);
    Message::binary(frame)
}

fn utterance(text: &str, definite: bool, start: u64, end: u64) -> Value {
    json!({ "text": text, "definite": definite, "start_time": start, "end_time": end })
}

async fn volc(mut socket: Socket, script: Volc, path: String, saw: Shared) {
    let whole = path.ends_with("/bigmodel_nostream");
    let (mut audio, mut sequence, mut partial, mut sentence) = (0usize, 2i32, false, false);
    while let Some(Ok(message)) = socket.next().await {
        let Message::Binary(bytes) = message else { continue };
        let (kind, flags, payload) = volc_frame(&bytes);
        if kind == 0b0001 {
            // The session's first answer, as the service gives it: no text yet.
            let _ = socket.send(volc_response(0, 0, &json!({ "result": { "additions": { "log_id": LOGID } } }))).await;
            continue;
        }
        audio += payload.len();
        saw.lock().unwrap().audio += payload.len();
        let last = flags & 0b0010 != 0;
        if whole || script == Volc::Silent {
            if last {
                let text = if whole { WHOLE } else { "" };
                let _ = socket.send(volc_response(0b0011, -sequence, &json!({ "result": { "text": text } }))).await;
                let _ = socket.close(None).await;
                return;
            }
            continue;
        }
        if audio >= PARTIAL_AFTER && !partial {
            partial = true;
            let said = json!({ "result": { "text": PARTIAL, "utterances": [utterance(PARTIAL, false, 0, 200)] } });
            let _ = socket.send(volc_response(0b0001, sequence, &said)).await;
            sequence += 1;
        }
        if audio >= SENTENCE_AFTER && !sentence {
            sentence = true;
            let said = json!({ "result": { "text": FIRST, "utterances": [utterance(FIRST, true, 0, 800)] } });
            let _ = socket.send(volc_response(0b0001, sequence, &said)).await;
            sequence += 1;
            if script == Volc::Drops {
                // The connection goes without a close, as a network that fails takes it.
                return;
            }
        }
        if last {
            let said = json!({ "result": { "text": TEXT, "utterances": [utterance(FIRST, true, 0, 800), utterance(LAST, true, 900, 1400)] } });
            let _ = socket.send(volc_response(0b0011, -sequence, &said)).await;
            let _ = socket.close(None).await;
            return;
        }
    }
}

const VOLC: Service = Service {
    provider: ProviderId::VolcengineSpeech,
    model: "volc.seedasr.sauc.duration",
    base_path: "",
    credential: r#"{"set":"api_key","api_key":"volc-test-key"}"#,
};

/// docs/dictation.md §3.15: a 豆包语音 take streams to `bigmodel_async`, and its sentences are the
/// take's text.
#[test]
fn volcengine_streams_into_the_pill() {
    let (take, saw) = take(&VOLC, |socket, _, path, saw| volc(socket, Volc::Speaks, path, saw), after_the_first_sentence);
    assert_streamed(&VOLC, &take);
    assert_eq!(saw.lock().unwrap().paths, ["/api/v3/sauc/bigmodel_async"], "one live session, no recorded take");
}

/// regression (user report 2026-11-05): a 豆包语音 stream that ended without any text fell back to
/// the whole recording, and the history said only 「实时识别未得到文本」. The take still gets the
/// whole recording's text, and the history now says what the service answered, its log id and
/// the audio it was sent.
#[test]
fn regression_a_volcengine_stream_without_text_says_what_the_service_answered() {
    let (take, saw) = take(&VOLC, |socket, _, path, saw| volc(socket, Volc::Silent, path, saw), |_, saw| saw.audio >= SENTENCE_AFTER + PARTIAL_AFTER);
    let DictationPhase::Done { text, mode, live_error, .. } = &take.end.dictation.phase else { panic!("{:?}", take.end.dictation.phase) };
    assert_eq!((text.as_str(), *mode), (WHOLE, OutputMode::WholeTake), "the whole recording's text");
    let reason = live_error.as_deref().unwrap_or_default();
    let prefix = format!("实时识别未得到文本（豆包语音返回了 2 次结果，均不含文字；日志 ID {LOGID}；已发送 ");
    assert!(reason.starts_with(&prefix) && reason.ends_with(" 秒音频，最大音量 -10 dB）"), "{reason}");
    let entry = &take.end.history_recent[0];
    assert_eq!((entry.asr_provider, entry.live_error.as_deref(), entry.text.as_str()), (Some(ProviderId::VolcengineSpeech), Some(reason), WHOLE));
    assert_eq!(saw.lock().unwrap().paths, ["/api/v3/sauc/bigmodel_async", "/api/v3/sauc/bigmodel_nostream"], "the stream, then the recorded take");
}

/// A 豆包语音 connection that drops in the middle of a take stops the preview, which the pill says,
/// and the take ends with the whole recording's text and the reason in the history: it neither
/// waits for the stream nor loses the take.
#[test]
fn a_volcengine_connection_that_drops_ends_the_take_with_the_whole_recording() {
    let stopped = |st: &UiState, _: &Saw| matches!(&st.dictation.phase, DictationPhase::Listening { live: Some(l), .. } if l.degraded.is_some());
    let (take, saw) = take(&VOLC, |socket, _, path, saw| volc(socket, Volc::Drops, path, saw), stopped);
    let DictationPhase::Done { text, mode, live_error, .. } = &take.end.dictation.phase else { panic!("{:?}", take.end.dictation.phase) };
    assert_eq!((text.as_str(), *mode), (WHOLE, OutputMode::WholeTake));
    assert!(live_error.as_deref().is_some_and(|e| !e.is_empty()), "the reason is kept: {live_error:?}");
    assert_eq!(take.end.history_recent[0].live_error, *live_error);
    assert_eq!(saw.lock().unwrap().paths, ["/api/v3/sauc/bigmodel_async", "/api/v3/sauc/bigmodel_nostream"]);
}

// ---- 腾讯云 --------------------------------------------------------------------------------

fn tencent_result(slice: u64, index: u64, text: &str, start: u64, end: u64) -> Message {
    let result = json!({ "slice_type": slice, "index": index, "start_time": start, "end_time": end, "voice_text_str": text, "word_size": 0, "word_list": [] });
    Message::text(json!({ "code": 0, "message": "success", "voice_id": "v1", "message_id": format!("v1_{index}_{slice}"), "result": result }).to_string())
}

async fn tencent(mut socket: Socket, saw: Shared) {
    let _ = socket.send(Message::text(json!({ "code": 0, "message": "success", "voice_id": "v1" }).to_string())).await;
    let (mut audio, mut partial, mut sentence) = (0usize, false, false);
    while let Some(Ok(message)) = socket.next().await {
        match message {
            Message::Binary(bytes) => {
                audio += bytes.len();
                saw.lock().unwrap().audio += bytes.len();
                if audio >= PARTIAL_AFTER && !partial {
                    partial = true;
                    let _ = socket.send(tencent_result(1, 0, PARTIAL, 0, 200)).await;
                }
                if audio >= SENTENCE_AFTER && !sentence {
                    sentence = true;
                    let _ = socket.send(tencent_result(2, 0, FIRST, 0, 800)).await;
                }
            }
            Message::Text(text) if text.as_str() == r#"{"type":"end"}"# => {
                let _ = socket.send(tencent_result(2, 1, LAST, 900, 1400)).await;
                let last = json!({ "code": 0, "message": "success", "voice_id": "v1", "message_id": "v1_end", "final": 1 });
                let _ = socket.send(Message::text(last.to_string())).await;
                let _ = socket.close(None).await;
                return;
            }
            _ => {}
        }
    }
}

#[test]
fn tencent_streams_into_the_pill() {
    let service = Service {
        provider: ProviderId::Tencent,
        model: "16k_zh_en",
        base_path: "",
        credential: r#"{"set":"default","app_id":"1250000000","secret_id":"test-secret-id","secret_key":"tencent-test-key"}"#,
    };
    let (take, saw) = take(&service, |socket, _, _, saw| tencent(socket, saw), after_the_first_sentence);
    assert_streamed(&service, &take);
    assert_eq!(saw.lock().unwrap().paths, ["/asr/v2/1250000000"]);
}

// ---- 讯飞 --------------------------------------------------------------------------------

fn iat_result(sn: u64, status: u64, word: &str) -> Message {
    let result = json!({ "sn": sn, "ls": status == 2, "bg": 0, "ed": 0, "pgs": "apd", "ws": [{ "bg": 0, "cw": [{ "sc": 0, "w": word }] }] });
    Message::text(json!({ "code": 0, "message": "success", "sid": "iat-test", "data": { "result": result, "status": status } }).to_string())
}

/// Dictation: the first session ends itself after the first sentence, as the service's pause
/// detection does; the next one says the last sentence at the end of the audio.
async fn iat(mut socket: Socket, index: usize, saw: Shared) {
    let (mut audio, mut sn, mut partial, mut over) = (0usize, 0u64, false, false);
    while let Some(Ok(message)) = socket.next().await {
        let Message::Text(text) = message else { continue };
        let frame: Value = serde_json::from_str(text.as_str()).unwrap();
        let status = frame.pointer("/data/status").and_then(Value::as_u64).unwrap_or(1);
        let bytes = frame.pointer("/data/audio").and_then(Value::as_str).map_or(0, |a| STANDARD.decode(a).unwrap().len());
        audio += bytes;
        saw.lock().unwrap().audio += bytes;
        if over {
            continue;
        }
        if index == 0 {
            if audio >= PARTIAL_AFTER && !partial {
                partial = true;
                sn += 1;
                let _ = socket.send(iat_result(sn, 1, PARTIAL)).await;
            }
            if audio >= SENTENCE_AFTER || status == 2 {
                sn += 1;
                let _ = socket.send(iat_result(sn, 2, "，世界。")).await;
                over = true;
            }
        } else if status == 2 {
            sn += 1;
            let _ = socket.send(iat_result(sn, 2, LAST)).await;
            over = true;
        }
    }
}

#[test]
fn iflytek_dictation_streams_into_the_pill() {
    let service = Service {
        provider: ProviderId::Iflytek,
        model: "iat",
        base_path: "",
        credential: r#"{"set":"default","app_id":"app-test","api_key":"iat-test-key","api_secret":"iat-test-secret"}"#,
    };
    let (take, saw) = take(&service, |socket, index, _, saw| iat(socket, index, saw), after_the_first_sentence);
    assert_streamed(&service, &take);
    assert_eq!(saw.lock().unwrap().paths, ["/v2/iat", "/v2/iat"], "a session a sentence");
}

fn rtasr_result(kind: &str, begin: u64, end: u64, word: &str) -> Message {
    let data = json!({ "cn": { "st": { "bg": begin.to_string(), "ed": end.to_string(), "rt": [{ "ws": [{ "cw": [{ "w": word, "wp": "n" }], "wb": 0, "we": 0 }] }], "type": kind } }, "seg_id": 0 });
    Message::text(json!({ "action": "result", "code": "0", "data": data.to_string(), "desc": "success", "sid": "rta-test" }).to_string())
}

async fn rtasr(mut socket: Socket, saw: Shared) {
    let started = json!({ "action": "started", "code": "0", "data": "", "desc": "success", "sid": "rta-test" });
    let _ = socket.send(Message::text(started.to_string())).await;
    let (mut audio, mut partial, mut sentence) = (0usize, false, false);
    while let Some(Ok(message)) = socket.next().await {
        let Message::Binary(bytes) = message else { continue };
        if bytes.as_ref() == br#"{"end": true}"# {
            let _ = socket.send(rtasr_result("0", 900, 1400, LAST)).await;
            let _ = socket.close(None).await;
            return;
        }
        audio += bytes.len();
        saw.lock().unwrap().audio += bytes.len();
        if audio >= PARTIAL_AFTER && !partial {
            partial = true;
            let _ = socket.send(rtasr_result("1", 0, 0, PARTIAL)).await;
        }
        if audio >= SENTENCE_AFTER && !sentence {
            sentence = true;
            let _ = socket.send(rtasr_result("0", 0, 800, FIRST)).await;
        }
    }
}

#[test]
fn iflytek_realtime_transcription_streams_into_the_pill() {
    let service = Service {
        provider: ProviderId::IflytekRtasr,
        model: "rtasr",
        base_path: "",
        credential: r#"{"set":"default","app_id":"app-test","api_key":"rtasr-key-test"}"#,
    };
    let (take, saw) = take(&service, |socket, _, _, saw| rtasr(socket, saw), after_the_first_sentence);
    assert_streamed(&service, &take);
    assert_eq!(saw.lock().unwrap().paths, ["/v1/ws"]);
}

// ---- 百度 --------------------------------------------------------------------------------

fn baidu_reply(kind: &str, text: &str, times: Option<(u64, u64)>) -> Message {
    let mut reply = json!({ "err_no": 0, "err_msg": "OK", "type": kind, "result": text, "log_id": "45677785", "sn": "sn-test" });
    if let Some((start, end)) = times {
        reply["start_time"] = json!(start);
        reply["end_time"] = json!(end);
        reply["product_id"] = json!(15372);
        reply["product_line"] = json!("open");
    }
    Message::text(reply.to_string())
}

async fn baidu(mut socket: Socket, saw: Shared) {
    let (mut audio, mut partial, mut sentence) = (0usize, false, false);
    while let Some(Ok(message)) = socket.next().await {
        match message {
            Message::Text(text) => match serde_json::from_str::<Value>(text.as_str()).unwrap()["type"].as_str() {
                Some("START") => drop(socket.send(Message::text(r#"{"type":"HEARTBEAT"}"#)).await),
                Some("FINISH") => {
                    let _ = socket.send(baidu_reply("FIN_TEXT", LAST, Some((900, 1400)))).await;
                    let _ = socket.close(None).await;
                    return;
                }
                _ => {}
            },
            Message::Binary(bytes) => {
                audio += bytes.len();
                saw.lock().unwrap().audio += bytes.len();
                if audio >= PARTIAL_AFTER && !partial {
                    partial = true;
                    let _ = socket.send(baidu_reply("MID_TEXT", PARTIAL, None)).await;
                }
                if audio >= SENTENCE_AFTER && !sentence {
                    sentence = true;
                    let _ = socket.send(baidu_reply("FIN_TEXT", FIRST, Some((0, 800)))).await;
                }
            }
            _ => {}
        }
    }
}

#[test]
fn baidu_streams_into_the_pill() {
    let service = Service {
        provider: ProviderId::Baidu,
        model: "15372",
        base_path: "",
        credential: r#"{"set":"default","app_id":"12345678","api_key":"baidu-key-test"}"#,
    };
    let (take, saw) = take(&service, |socket, _, _, saw| baidu(socket, saw), after_the_first_sentence);
    assert_streamed(&service, &take);
    assert_eq!(saw.lock().unwrap().paths, ["/realtime_asr"]);
}

// ---- 阶跃星辰 ------------------------------------------------------------------------------

fn stepfun_event(event: Value) -> Message {
    Message::text(event.to_string())
}

/// The doc's event order: `session.created`, `session.updated` after the configuration; each
/// sentence as deltas of its settled `text` and its `stash`, then `completed`; the sentence being
/// spoken at the end completed by the client's commit.
async fn stepfun(mut socket: Socket, saw: Shared) {
    let _ = socket.send(stepfun_event(json!({ "type": "session.created", "event_id": "e0" }))).await;
    let (mut audio, mut started, mut partial, mut sentence) = (0usize, false, false, false);
    while let Some(Ok(message)) = socket.next().await {
        let Message::Text(text) = message else { continue };
        let value: Value = serde_json::from_str(text.as_str()).unwrap();
        let mut say = Vec::new();
        match value["type"].as_str() {
            Some("session.update") => say.push(json!({ "type": "session.updated" })),
            Some("input_audio_buffer.append") => {
                let bytes = STANDARD.decode(value["audio"].as_str().unwrap()).unwrap().len();
                audio += bytes;
                saw.lock().unwrap().audio += bytes;
                if !std::mem::replace(&mut started, true) {
                    say.push(json!({ "type": "input_audio_buffer.speech_started", "audio_start_ms": 0, "item_id": "i1" }));
                }
                if audio >= PARTIAL_AFTER && !partial {
                    partial = true;
                    say.push(json!({ "type": "conversation.item.input_audio_transcription.delta", "item_id": "i1", "text": "", "stash": PARTIAL }));
                }
                if audio >= SENTENCE_AFTER && !sentence {
                    sentence = true;
                    say.extend([
                        json!({ "type": "input_audio_buffer.speech_stopped", "audio_start_ms": 800, "item_id": "i1" }),
                        json!({ "type": "input_audio_buffer.committed", "item_id": "i1" }),
                        json!({ "type": "conversation.item.input_audio_transcription.completed", "item_id": "i1", "transcript": FIRST, "words": [{ "word": "你", "start": 0.0, "end": 0.2 }, { "word": "界", "start": 0.6, "end": 0.8 }] }),
                        json!({ "type": "input_audio_buffer.speech_started", "audio_start_ms": 900, "item_id": "i2" }),
                        json!({ "type": "conversation.item.input_audio_transcription.delta", "item_id": "i2", "text": "今天", "stash": "天气" }),
                    ]);
                }
            }
            Some("input_audio_buffer.commit") => say.extend([
                json!({ "type": "input_audio_buffer.committed", "item_id": "i2" }),
                json!({ "type": "conversation.item.input_audio_transcription.completed", "item_id": "i2", "transcript": LAST, "words": [{ "word": "今", "start": 0.9, "end": 1.1 }, { "word": "错", "start": 1.2, "end": 1.4 }] }),
            ]),
            _ => {}
        }
        for event in say {
            let _ = socket.send(stepfun_event(event)).await;
        }
    }
}

#[test]
fn stepfun_streams_into_the_pill() {
    let service = Service { provider: ProviderId::Stepfun, model: "stepaudio-2.5-asr-stream", base_path: "/v1", credential: "sf-test" };
    let (take, saw) = take(&service, |socket, _, _, saw| stepfun(socket, saw), after_the_first_sentence);
    assert_streamed(&service, &take);
    assert_eq!(saw.lock().unwrap().paths, ["/v1/realtime/asr/stream"]);
}

// ---- 阿里云百炼 ----------------------------------------------------------------------------

fn dashscope_event(name: &str, payload: Value) -> Message {
    Message::text(json!({ "header": { "task_id": "t", "event": name }, "payload": payload }).to_string())
}

fn dashscope_sentence(id: u64, text: &str, end: bool, begin_ms: u64, end_ms: u64) -> Message {
    let sentence = json!({ "sentence_id": id, "text": text, "sentence_end": end, "begin_time": begin_ms, "end_time": end_ms });
    dashscope_event("result-generated", json!({ "output": { "sentence": sentence } }))
}

async fn dashscope(mut socket: Socket, saw: Shared) {
    let (mut audio, mut partial, mut sentence) = (0usize, false, false);
    while let Some(Ok(message)) = socket.next().await {
        match message {
            Message::Text(text) => match serde_json::from_str::<Value>(text.as_str()).unwrap().pointer("/header/action").and_then(Value::as_str) {
                Some("run-task") => drop(socket.send(dashscope_event("task-started", json!({}))).await),
                Some("finish-task") => {
                    let _ = socket.send(dashscope_sentence(2, LAST, true, 900, 1400)).await;
                    let _ = socket.send(dashscope_event("task-finished", json!({}))).await;
                }
                _ => {}
            },
            Message::Binary(bytes) => {
                audio += bytes.len();
                saw.lock().unwrap().audio += bytes.len();
                if audio >= PARTIAL_AFTER && !partial {
                    partial = true;
                    let _ = socket.send(dashscope_sentence(1, PARTIAL, false, 0, 200)).await;
                }
                if audio >= SENTENCE_AFTER && !sentence {
                    sentence = true;
                    let _ = socket.send(dashscope_sentence(1, FIRST, true, 0, 800)).await;
                }
            }
            _ => {}
        }
    }
}

#[test]
fn model_studio_realtime_models_stream_into_the_pill() {
    for model in ["qwen-audio-3.1-asr-flash-streaming", "fun-asr-realtime"] {
        let service = Service { provider: ProviderId::Aliyun, model, base_path: "/compatible-mode/v1", credential: "test-api-key" };
        let (take, saw) = take(&service, |socket, _, _, saw| dashscope(socket, saw), after_the_first_sentence);
        assert_streamed(&service, &take);
        assert_eq!(saw.lock().unwrap().paths.len(), 1, "{model}: one task");
    }
}
