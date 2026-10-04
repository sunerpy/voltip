//! The local speech service over HTTP (docs/dictation.md §23.4) with a fake behind it: the token,
//! the request Paseo sends, the response formats, the error mapping, the limits, admission and
//! cancellation, and a real socket.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use parking_lot::Mutex;
use tower::ServiceExt as _;
use voltip_core::CancelToken;
use voltip_core::serve::{ListenConfig, ModelInfo, PcmFile, ServeError, ServeHost as _, ServeOutcome, ServeRequest, SpeechService};
use voltip_serve::{HttpHost, ServeHandle};

const TOKEN: &str = "test-token";
/// Longest a fake holds a call before giving up, so a failed assertion never hangs the job.
const HOLD_LIMIT: Duration = Duration::from_secs(30);

/// What the fake answers and what it saw.
#[derive(Default)]
struct Fake {
    answer: Mutex<Option<Result<ServeOutcome, ServeError>>>,
    seen: Mutex<Vec<(ServeRequest, u64, Vec<i16>)>>,
    ready: Mutex<Option<String>>,
    /// While set, calls wait (up to [`HOLD_LIMIT`]) until it is cleared.
    hold: AtomicBool,
    holding: AtomicUsize,
    saw_cancel: AtomicBool,
}

impl Fake {
    fn answering(outcome: ServeOutcome) -> Arc<Self> {
        Arc::new(Self { answer: Mutex::new(Some(Ok(outcome))), ..Self::default() })
    }

    fn failing(error: ServeError) -> Arc<Self> {
        Arc::new(Self { answer: Mutex::new(Some(Err(error))), ..Self::default() })
    }
}

#[async_trait]
impl SpeechService for Fake {
    fn models(&self) -> Vec<ModelInfo> {
        vec![ModelInfo { id: "voltip".into(), name: "默认处理方式".into() }, ModelInfo { id: "voltip:raw".into(), name: "不使用 AI 润色".into() }]
    }

    fn ready(&self) -> Result<(), String> {
        self.ready.lock().clone().map_or(Ok(()), Err)
    }

    async fn transcribe(&self, audio: PcmFile, request: ServeRequest, cancel: CancelToken) -> Result<ServeOutcome, ServeError> {
        let pcm: Vec<i16> = std::fs::read(&audio.path).unwrap().as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
        self.seen.lock().push((request, audio.samples, pcm));
        self.holding.fetch_add(1, Ordering::SeqCst);
        let started = std::time::Instant::now();
        while self.hold.load(Ordering::SeqCst) && started.elapsed() < HOLD_LIMIT {
            if cancel.is_cancelled() {
                self.saw_cancel.store(true, Ordering::SeqCst);
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        self.holding.fetch_sub(1, Ordering::SeqCst);
        self.answer.lock().clone().unwrap_or_else(|| Ok(ServeOutcome::default()))
    }
}

fn outcome(text: &str) -> ServeOutcome {
    ServeOutcome { text: text.into(), raw_text: format!("raw {text}"), duration_ms: 2000, segments: 1, language: Some("zh".into()), ..ServeOutcome::default() }
}

fn config(dir: &Path, concurrency: usize, max_minutes: u16) -> ListenConfig {
    ListenConfig { addr: "127.0.0.1:0".parse().unwrap(), token: TOKEN.into(), concurrency, max_minutes, uploads_dir: dir.join("uploads") }
}

fn handle(fake: &Arc<Fake>, dir: &Path) -> ServeHandle {
    ServeHandle::new(&config(dir, 2, 120), fake.clone() as Arc<dyn SpeechService>).unwrap()
}

fn wav(rate: u32, channels: u16, seconds: f64) -> Vec<u8> {
    let spec = hound::WavSpec { channels, sample_rate: rate, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut w = hound::WavWriter::new(&mut buf, spec).unwrap();
        for i in 0..(f64::from(rate) * seconds) as usize * usize::from(channels) {
            w.write_sample((8000.0 * (i as f64 * 0.05).sin()) as i16).unwrap();
        }
        w.finalize().unwrap();
    }
    buf.into_inner()
}

/// A WAV header (16 kHz mono 16-bit) that declares `data_bytes` of samples, without them.
fn header_declaring(data_bytes: u32) -> Vec<u8> {
    let mut h = Vec::new();
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes());
    h.extend_from_slice(&16_000u32.to_le_bytes());
    h.extend_from_slice(&32_000u32.to_le_bytes());
    h.extend_from_slice(&2u16.to_le_bytes());
    h.extend_from_slice(&16u16.to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&data_bytes.to_le_bytes());
    h
}

/// One multipart part: its name, its bytes, whether it is the file.
type Part = (&'static str, Vec<u8>, bool);

const BOUNDARY: &str = "voltip-test-boundary";

/// A multipart body: `(name, bytes, is_file)` parts in order.
fn multipart(parts: &[Part]) -> Vec<u8> {
    let mut body = Vec::new();
    for (name, bytes, file) in parts {
        body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        if *file {
            body.extend_from_slice(
                format!("Content-Disposition: form-data; name=\"{name}\"; filename=\"audio.wav\"\r\nContent-Type: application/octet-stream\r\n\r\n").as_bytes(),
            );
        } else {
            body.extend_from_slice(format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes());
        }
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    body
}

/// The request Paseo's `openai` STT sends: a 24 kHz mono WAV, model, language, its fixed prompt, json.
fn paseo_parts(model: &str) -> Vec<Part> {
    vec![
        ("file", wav(24_000, 1, 2.0), true),
        ("language", b"zh".to_vec(), false),
        ("model", model.as_bytes().to_vec(), false),
        ("prompt", b"Transcribe only what the speaker says. Do not add words.".to_vec(), false),
        ("response_format", b"json".to_vec(), false),
    ]
}

fn post(parts: &[Part], token: Option<&str>) -> Request<Body> {
    let mut request = Request::post("/v1/audio/transcriptions").header(header::CONTENT_TYPE, format!("multipart/form-data; boundary={BOUNDARY}"));
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    request.body(Body::from(multipart(parts))).unwrap()
}

async fn call(handle: &ServeHandle, request: Request<Body>) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let response = handle.router().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap().to_vec();
    (status, headers, body)
}

fn json(body: &[u8]) -> serde_json::Value {
    serde_json::from_slice(body).unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(body)))
}

#[tokio::test]
async fn the_token_guards_everything_but_the_health_check() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Fake::answering(outcome("你好"));
    let h = handle(&fake, dir.path());
    for token in [None, Some("wrong")] {
        let (status, headers, body) = call(&h, post(&paseo_parts("voltip"), token)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(headers[header::WWW_AUTHENTICATE], "Bearer");
        assert_eq!(json(&body)["error"]["code"], "invalid_api_key");
    }
    let models = Request::get("/v1/models").body(Body::empty()).unwrap();
    assert_eq!(call(&h, models).await.0, StatusCode::UNAUTHORIZED);
    assert!(fake.seen.lock().is_empty(), "nothing reached the service");
    let (status, _, body) = call(&h, Request::get("/healthz").body(Body::empty()).unwrap()).await;
    assert_eq!((status, json(&body)["status"].as_str()), (StatusCode::OK, Some("ok")));
    *fake.ready.lock() = Some("本地模型未下载".into());
    let (status, _, body) = call(&h, Request::get("/healthz").body(Body::empty()).unwrap()).await;
    assert_eq!((status, json(&body)["status"].as_str()), (StatusCode::SERVICE_UNAVAILABLE, Some("unavailable")), "no details");
    // A new token works at once, the old one no longer.
    h.token().set("rotated".into());
    assert_eq!(call(&h, post(&paseo_parts("voltip"), Some(TOKEN))).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(call(&h, post(&paseo_parts("voltip"), Some("rotated"))).await.0, StatusCode::OK);
}

#[tokio::test]
async fn the_request_paseo_sends_is_decoded_to_16_khz_and_answered_with_its_text() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Fake::answering(outcome("你好，世界。"));
    let h = handle(&fake, dir.path());
    let (status, _, body) = call(&h, post(&paseo_parts("voltip:scene=coding"), Some(TOKEN))).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(json(&body), serde_json::json!({ "text": "你好，世界。" }), "json is the text alone");
    let seen = fake.seen.lock();
    let (request, samples, pcm) = &seen[0];
    assert_eq!((request.model.as_deref(), request.language.as_deref()), (Some("voltip:scene=coding"), Some("zh")));
    assert_eq!(*samples, 32_000, "two seconds at 16 kHz");
    assert_eq!(pcm.len(), 32_000);
    assert!(pcm.iter().map(|s| i32::from(*s).abs()).max().unwrap() > 6000, "the level survived");
    drop(seen);
    let leftovers = std::fs::read_dir(dir.path().join("uploads")).unwrap().count();
    assert_eq!(leftovers, 0, "the decoded file is removed after the request");
}

#[tokio::test]
async fn the_response_formats() {
    let dir = tempfile::tempdir().unwrap();
    let h = handle(&Fake::answering(outcome("你好")), dir.path());
    let mut parts = paseo_parts("voltip");
    parts[4].1 = b"text".to_vec();
    let (status, headers, body) = call(&h, post(&parts, Some(TOKEN))).await;
    assert_eq!((status, headers[header::CONTENT_TYPE].to_str().unwrap(), body.as_slice()), (StatusCode::OK, "text/plain; charset=utf-8", "你好".as_bytes()));
    parts[4].1 = b"verbose_json".to_vec();
    let verbose = json(&call(&h, post(&parts, Some(TOKEN))).await.2);
    assert_eq!((verbose["task"].as_str(), verbose["language"].as_str(), verbose["duration"].as_f64()), (Some("transcribe"), Some("zh"), Some(2.0)));
    assert_eq!(verbose["segments"], serde_json::json!([]));
    assert_eq!(verbose["voltip"]["raw_text"], "raw 你好");
    for bad in ["srt", "vtt", "xml"] {
        parts[4].1 = bad.as_bytes().to_vec();
        let (status, _, body) = call(&h, post(&parts, Some(TOKEN))).await;
        assert_eq!((status, json(&body)["error"]["code"].as_str()), (StatusCode::BAD_REQUEST, Some("unsupported_response_format")), "{bad}");
    }
    let list = json(&call(&h, Request::get("/v1/models").header(header::AUTHORIZATION, format!("Bearer {TOKEN}")).body(Body::empty()).unwrap()).await.2);
    assert_eq!(list["object"], "list");
    assert_eq!((list["data"][1]["id"].as_str(), list["data"][1]["object"].as_str()), (Some("voltip:raw"), Some("model")));
}

#[tokio::test]
async fn service_errors_become_openai_errors() {
    for (error, status, code) in [
        (ServeError::Invalid("没有名为「x」的场景".into()), StatusCode::BAD_REQUEST, "invalid_model"),
        (ServeError::NotReady("本地模型未下载".into()), StatusCode::SERVICE_UNAVAILABLE, "not_ready"),
        (ServeError::Quota("额度已用完".into()), StatusCode::TOO_MANY_REQUESTS, "insufficient_quota"),
        (ServeError::Upstream("asr: 503".into()), StatusCode::BAD_GATEWAY, "upstream_error"),
        (ServeError::Internal("disk".into()), StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let h = handle(&Fake::failing(error.clone()), dir.path());
        let (got, headers, body) = call(&h, post(&paseo_parts("voltip"), Some(TOKEN))).await;
        let body = json(&body);
        assert_eq!((got, body["error"]["code"].as_str(), body["error"]["message"].as_str()), (status, Some(code), Some(error.to_string().as_str())));
        assert_eq!(headers.contains_key(header::RETRY_AFTER), status == StatusCode::SERVICE_UNAVAILABLE);
    }
}

#[tokio::test]
async fn malformed_and_oversized_requests_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Fake::answering(outcome("x"));
    let h = ServeHandle::new(&config(dir.path(), 2, 1), fake.clone() as Arc<dyn SpeechService>).unwrap();
    let cases: Vec<(Vec<Part>, StatusCode, &str)> = vec![
        (vec![("file", b"ID3\x04 an mp3".to_vec(), true)], StatusCode::UNSUPPORTED_MEDIA_TYPE, "unsupported_audio_format"),
        (vec![("model", b"voltip".to_vec(), false)], StatusCode::BAD_REQUEST, "missing_file"),
        (vec![("file", wav(16_000, 1, 1.0), true), ("stream", b"true".to_vec(), false)], StatusCode::BAD_REQUEST, "stream_unsupported"),
        (vec![("file", wav(16_000, 1, 1.0), true), ("file", wav(16_000, 1, 1.0), true)], StatusCode::BAD_REQUEST, "duplicate_file"),
        (vec![("prompt", vec![b'a'; 5000], false), ("file", wav(16_000, 1, 1.0), true)], StatusCode::BAD_REQUEST, "field_too_long"),
        // Two minutes declared where one is allowed: refused from the header, the samples never sent.
        (vec![("file", header_declaring(2 * 60 * 32_000), true)], StatusCode::PAYLOAD_TOO_LARGE, "audio_too_long"),
        // One second of samples followed by 2 MiB of something else.
        (vec![("file", [wav(16_000, 1, 1.0), vec![0u8; 2 * 1024 * 1024]].concat(), true)], StatusCode::PAYLOAD_TOO_LARGE, "audio_too_long"),
        (vec![("file", wav(16_000, 1, 1.0)[..20_000].to_vec(), true)], StatusCode::BAD_REQUEST, "invalid_audio"),
    ];
    for (parts, status, code) in cases {
        let (got, _, body) = call(&h, post(&parts, Some(TOKEN))).await;
        assert_eq!((got, json(&body)["error"]["code"].as_str()), (status, Some(code)), "{code}");
    }
    let many: Vec<Part> = (0..20).map(|_| ("temperature", b"0".to_vec(), false)).collect();
    let (got, _, body) = call(&h, post(&many, Some(TOKEN))).await;
    assert_eq!((got, json(&body)["error"]["code"].as_str()), (StatusCode::BAD_REQUEST, Some("too_many_fields")));
    assert!(fake.seen.lock().is_empty(), "no refused request reached the service");
    assert_eq!(std::fs::read_dir(dir.path().join("uploads")).unwrap().count(), 0, "no file left behind");
}

/// Waits (bounded) until `cond` holds.
async fn until(cond: impl Fn() -> bool) {
    let started = std::time::Instant::now();
    while !cond() {
        assert!(started.elapsed() < HOLD_LIMIT, "condition never held");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_waiting_request_reads_nothing_until_it_has_a_permit_and_a_full_queue_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Fake::answering(outcome("x"));
    fake.hold.store(true, Ordering::SeqCst);
    let h = ServeHandle::new(&config(dir.path(), 1, 120), fake.clone() as Arc<dyn SpeechService>).unwrap();
    let spawn = |h: &ServeHandle| {
        let router = h.router();
        tokio::spawn(async move { router.oneshot(post(&paseo_parts("voltip"), Some(TOKEN))).await.unwrap().status() })
    };
    let first = spawn(&h);
    until(|| fake.holding.load(Ordering::SeqCst) == 1).await;
    let (second, third) = (spawn(&h), spawn(&h));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(fake.seen.lock().len(), 1, "the queued requests did not reach the service");
    assert_eq!(std::fs::read_dir(dir.path().join("uploads")).unwrap().count(), 1, "only the running take has a file: the queued bodies were not read");
    let (refused, _, body) = call(&h, post(&paseo_parts("voltip"), Some(TOKEN))).await;
    assert_eq!((refused, json(&body)["error"]["code"].as_str()), (StatusCode::SERVICE_UNAVAILABLE, Some("server_busy")), "a queue of two is full");
    fake.hold.store(false, Ordering::SeqCst);
    for task in [first, second, third] {
        assert_eq!(task.await.unwrap(), StatusCode::OK);
    }
    assert_eq!(fake.seen.lock().len(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_client_that_goes_away_cancels_its_take_which_keeps_its_permit_until_it_ends() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Fake::answering(outcome("x"));
    fake.hold.store(true, Ordering::SeqCst);
    let h = ServeHandle::new(&config(dir.path(), 1, 120), fake.clone() as Arc<dyn SpeechService>).unwrap();
    let router = h.router();
    let client = tokio::spawn(async move { router.oneshot(post(&paseo_parts("voltip"), Some(TOKEN))).await });
    until(|| fake.holding.load(Ordering::SeqCst) == 1).await;
    client.abort();
    let _ = client.await;
    until(|| fake.saw_cancel.load(Ordering::SeqCst)).await;
    assert_eq!(fake.holding.load(Ordering::SeqCst), 1, "the take is still running");
    // Its permit is still taken: the next request waits instead of starting.
    let router = h.router();
    let next = tokio::spawn(async move { router.oneshot(post(&paseo_parts("voltip"), Some(TOKEN))).await.unwrap().status() });
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(fake.seen.lock().len(), 1, "no second take while the first one runs");
    fake.hold.store(false, Ordering::SeqCst);
    assert_eq!(next.await.unwrap(), StatusCode::OK);
    until(|| std::fs::read_dir(dir.path().join("uploads")).unwrap().count() == 0).await;
    assert_eq!(h.active(), 0);
}

#[tokio::test]
async fn a_real_socket_serves_and_shuts_down_after_its_requests() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Fake::answering(outcome("你好"));
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let (addr, task) = handle(&fake, dir.path())
        .serve("127.0.0.1:0".parse().unwrap(), async move {
            let _ = stopped.await;
        })
        .await
        .unwrap();
    let form = reqwest::multipart::Form::new()
        .part("file", reqwest::multipart::Part::bytes(wav(24_000, 1, 1.0)).file_name("audio.wav").mime_str("audio/wav").unwrap())
        .text("model", "voltip")
        .text("language", "zh")
        .text("response_format", "json");
    let response = reqwest::Client::new().post(format!("http://{addr}/v1/audio/transcriptions")).bearer_auth(TOKEN).multipart(form).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.json::<serde_json::Value>().await.unwrap()["text"], "你好");
    stop.send(()).unwrap();
    tokio::time::timeout(HOLD_LIMIT, task).await.unwrap().unwrap().unwrap();
}

#[tokio::test]
async fn the_apps_host_starts_reports_a_taken_port_and_stops() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Fake::answering(outcome("x"));
    let running = HttpHost.start(config(dir.path(), 2, 120), fake.clone() as Arc<dyn SpeechService>).await.unwrap();
    let addr: SocketAddr = running.addr();
    assert!(addr.port() > 0);
    let health = reqwest::get(format!("http://{addr}/healthz")).await.unwrap();
    assert_eq!(health.status(), reqwest::StatusCode::OK);
    let taken = ListenConfig { addr, ..config(dir.path(), 2, 120) };
    let err = HttpHost.start(taken, fake.clone() as Arc<dyn SpeechService>).await.err().expect("the port is taken");
    assert!(err.contains("无法监听"), "{err}");
    running.set_token("new".into());
    let client = reqwest::Client::new();
    let models = |token: &'static str| client.get(format!("http://{addr}/v1/models")).bearer_auth(token).send();
    assert_eq!(models(TOKEN).await.unwrap().status(), reqwest::StatusCode::UNAUTHORIZED);
    assert_eq!(models("new").await.unwrap().status(), reqwest::StatusCode::OK);
    running.stop();
    let started = std::time::Instant::now();
    while reqwest::get(format!("http://{addr}/healthz")).await.is_ok() {
        assert!(started.elapsed() < HOLD_LIMIT, "still listening");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
