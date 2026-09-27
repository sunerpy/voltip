#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The model library against a local HTTP server (docs/dictation.md §10 门禁): resume with
//! `Range`, sha256 rejection that keeps the `.part`, cancel, source fall-back, manifest, removal,
//! the free-space check.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use sha2::{Digest as _, Sha256};
use voltip_asr_local::{
    CATALOGUE_VERSION, Capability, DISK_HEADROOM, Engine, FreeSpace, MANIFEST_FILE, Manifest, ModelEntry, ModelFile, ModelStore, SlowSourcePolicy, Source,
    StoreError, Tier, bytes_to_fetch,
};
use voltip_core::{CancelToken, ModelInstallState, ModelManager, ProgressSink};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const REPO: &str = "test/tiny-model";
const AUX_REPO: &str = "test/tiny-vad";
const MODEL_SIZE: usize = 3 * 1024 * 1024 + 517;
const TOKENS_SIZE: usize = 4096;
const AUX_SIZE: usize = 2048;

/// Deterministic pseudo-random bytes (an LCG), so every test sees the same "model".
fn bytes(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed;
    (0..len)
        .map(|_| {
            x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (x >> 33) as u8
        })
        .collect()
}

fn model_bytes() -> Vec<u8> {
    bytes(MODEL_SIZE, 7)
}

fn tokens_bytes() -> Vec<u8> {
    bytes(TOKENS_SIZE, 11)
}

fn aux_bytes() -> Vec<u8> {
    bytes(AUX_SIZE, 13)
}

fn sha(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

/// A two-entry catalogue — a recognition model and an auxiliary VAD (docs/dictation.md §12) — whose
/// hashes are those of the fake bytes above (leaked: the store wants `'static`, and a test process
/// is short-lived).
fn catalogue() -> &'static [ModelEntry] {
    let files: &'static [ModelFile] = Box::leak(
        vec![
            ModelFile { name: "model.int8.onnx", size: MODEL_SIZE as u64, sha256: Box::leak(sha(&model_bytes()).into_boxed_str()) },
            ModelFile { name: "tokens.txt", size: TOKENS_SIZE as u64, sha256: Box::leak(sha(&tokens_bytes()).into_boxed_str()) },
        ]
        .into_boxed_slice(),
    );
    let entry = ModelEntry {
        id: "tiny",
        name: "Tiny",
        engine: Engine::SenseVoice,
        tier: Tier::Light,
        capabilities: &[Capability::Offline],
        languages: &["zh"],
        description: "test",
        recommended: true,
        repo: REPO,
        files,
    };
    let aux_files: &'static [ModelFile] =
        Box::leak(vec![ModelFile { name: "silero_vad.onnx", size: AUX_SIZE as u64, sha256: Box::leak(sha(&aux_bytes()).into_boxed_str()) }].into_boxed_slice());
    let aux = ModelEntry {
        id: "tiny-vad",
        name: "Tiny VAD",
        engine: Engine::SileroVad,
        tier: Tier::Auxiliary,
        capabilities: &[Capability::Vad],
        languages: &["zh"],
        description: "test",
        recommended: false,
        repo: AUX_REPO,
        files: aux_files,
    };
    Box::leak(vec![entry, aux].into_boxed_slice())
}

fn store(root: &Path, sources: Vec<Source>) -> ModelStore {
    ModelStore::with_sources(root, catalogue(), sources)
}

fn sink() -> (ProgressSink, Arc<Mutex<Vec<ModelInstallState>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = seen.clone();
    (Arc::new(move |state| s.lock().push(state)), seen)
}

async fn serve_full(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path(format!("/{REPO}/model.int8.onnx")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(model_bytes()))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/{REPO}/tokens.txt")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(tokens_bytes()))
        .mount(server)
        .await;
}

async fn serve_aux(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path(format!("/{AUX_REPO}/silero_vad.onnx")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(aux_bytes()))
        .mount(server)
        .await;
}

fn assert_installed(root: &Path, state: &ModelInstallState) {
    let dir = root.join("tiny");
    assert!(matches!(state, ModelInstallState::Installed { path, installed_at } if Path::new(path) == dir && *installed_at > 1_700_000_000), "{state:?}");
    assert_eq!(std::fs::read(dir.join("model.int8.onnx")).unwrap(), model_bytes());
    assert_eq!(std::fs::read(dir.join("tokens.txt")).unwrap(), tokens_bytes());
    assert!(!dir.join("model.int8.onnx.part").exists() && !dir.join("tokens.txt.part").exists(), "no .part left behind");
    assert!(!dir.join(format!("{MANIFEST_FILE}.tmp")).exists(), "the manifest took its final name");
    let manifest: Manifest = serde_json::from_slice(&std::fs::read(dir.join(MANIFEST_FILE)).unwrap()).unwrap();
    assert_eq!(manifest.id, "tiny");
    assert_eq!(manifest.version, CATALOGUE_VERSION);
    assert_eq!(manifest.files.len(), 2);
    assert_eq!(manifest.files["model.int8.onnx"], sha(&model_bytes()));
}

#[tokio::test]
async fn fresh_download_verifies_writes_the_manifest_and_reports_progress() {
    let server = MockServer::start().await;
    serve_full(&server).await;
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path(), vec![Source::Base(server.uri())]);
    assert!(store.scan().iter().all(|m| m.state == ModelInstallState::NotInstalled));
    assert_eq!(store.scan().len(), 1, "the auxiliary entry is not a card");
    let (progress, seen) = sink();
    let state = store.download("tiny", progress, CancelToken::new()).await.unwrap();
    assert_installed(dir.path(), &state);
    assert_eq!(store.install_state("tiny-vad").unwrap(), ModelInstallState::NotInstalled, "auxiliary entries are not wanted by default");
    let seen = seen.lock().clone();
    // Starts at 0, ends at the full size for each file, verifies twice, never reports `Installed`
    // (that is the return value) and the counter never goes down within a file.
    assert_eq!(seen.first(), Some(&ModelInstallState::Downloading { received: 0, total: MODEL_SIZE as u64, file: "model.int8.onnx".into() }));
    assert!(
        seen.iter().any(|s| *s == ModelInstallState::Downloading { received: MODEL_SIZE as u64, total: MODEL_SIZE as u64, file: "model.int8.onnx".into() })
    );
    assert!(seen.iter().any(|s| *s == ModelInstallState::Downloading { received: TOKENS_SIZE as u64, total: TOKENS_SIZE as u64, file: "tokens.txt".into() }));
    assert_eq!(seen.iter().filter(|s| **s == ModelInstallState::Verifying).count(), 2);
    assert!(seen.iter().all(|s| !s.is_installed()));
    let mut last = 0;
    for s in seen.iter().take_while(|s| matches!(s, ModelInstallState::Downloading { file, .. } if file == "model.int8.onnx")) {
        if let ModelInstallState::Downloading { received, .. } = s {
            assert!(*received >= last);
            last = *received;
        }
    }
    // The scan agrees; a second download is a no-op that still returns `Installed`.
    assert!(store.scan()[0].state.is_installed());
    assert_eq!(store.install_state("tiny").unwrap(), state);
    server.reset().await; // nothing may be fetched again
    let (progress, seen) = sink();
    let again = store.download("tiny", progress, CancelToken::new()).await.unwrap();
    assert!(again.is_installed());
    assert!(seen.lock().is_empty(), "complete files are not re-fetched or re-hashed");
    // Through the trait the core uses.
    let manager: Arc<dyn ModelManager> = Arc::new(store.clone());
    assert!(manager.scan()[0].state.is_installed());
    manager.remove("tiny").unwrap();
    assert!(!dir.path().join("tiny").exists());
    assert_eq!(manager.scan()[0].state, ModelInstallState::NotInstalled);
    assert_eq!(manager.download("ghost", Arc::new(|_| {}), CancelToken::new()).await, Err("目录中没有 ghost".to_owned()));
}

#[tokio::test]
async fn partial_file_resumes_with_a_range_request() {
    let server = MockServer::start().await;
    let head = 1_048_576usize;
    let model = model_bytes();
    // Only the tail is served, and only for the right `Range`; a full request would 404.
    Mock::given(method("GET"))
        .and(path(format!("/{REPO}/model.int8.onnx")))
        .and(header("range", format!("bytes={head}-")))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header("content-range", format!("bytes {head}-{}/{MODEL_SIZE}", MODEL_SIZE - 1))
                .set_body_bytes(model[head..].to_vec()),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/{REPO}/tokens.txt")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(tokens_bytes()))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let model_dir = dir.path().join("tiny");
    std::fs::create_dir_all(&model_dir).unwrap();
    std::fs::write(model_dir.join("model.int8.onnx.part"), &model[..head]).unwrap();
    let store = store(dir.path(), vec![Source::Base(server.uri())]);
    let (progress, seen) = sink();
    let state = store.download("tiny", progress, CancelToken::new()).await.unwrap();
    assert_installed(dir.path(), &state);
    assert_eq!(
        seen.lock().first(),
        Some(&ModelInstallState::Downloading { received: head as u64, total: MODEL_SIZE as u64, file: "model.int8.onnx".into() }),
        "progress starts where the .part ended"
    );
}

/// Regression (design review 2026-09-27, model management): a download that does not fit on the
/// disk fails before its first byte and says how much it needs; what is already on disk counts,
/// the headroom is part of the need, an installed model needs nothing, and a disk that cannot say
/// how much is free does not block the download.
#[tokio::test]
async fn a_download_that_does_not_fit_on_the_disk_fails_before_its_first_byte() {
    let server = MockServer::start().await;
    serve_full(&server).await;
    let dir = tempfile::tempdir().unwrap();
    let entry = &catalogue()[0];
    let model_dir = dir.path().join("tiny");
    let missing = (MODEL_SIZE + TOKENS_SIZE) as u64;
    assert_eq!(bytes_to_fetch(entry, &model_dir), missing, "nothing on disk: every byte is missing");
    let short = store(dir.path(), vec![Source::Base(server.uri())]).with_free_space(FreeSpace::fixed(missing + DISK_HEADROOM - 1));
    let (progress, seen) = sink();
    let err = short.download("tiny", progress, CancelToken::new()).await.unwrap_err();
    assert_eq!(err, StoreError::NoSpace { needed: missing + DISK_HEADROOM, available: missing + DISK_HEADROOM - 1 });
    assert_eq!(err.to_string(), "磁盘空间不足：此模型还需 104 MB 空闲空间，当前可用 103 MB");
    assert!(seen.lock().is_empty(), "no progress: nothing was fetched");
    assert!(server.received_requests().await.unwrap().is_empty(), "no request went out");
    assert_eq!(short.install_state("tiny").unwrap(), ModelInstallState::NotInstalled);
    // A `.part` already on disk is space the download does not need again.
    let head = 1_048_576usize;
    std::fs::write(model_dir.join("model.int8.onnx.part"), &model_bytes()[..head]).unwrap();
    assert_eq!(bytes_to_fetch(entry, &model_dir), missing - head as u64);
    let fits = store(dir.path(), vec![Source::Base(server.uri())]).with_free_space(FreeSpace::fixed(missing - head as u64 + DISK_HEADROOM));
    let state = fits.download("tiny", Arc::new(|_| {}), CancelToken::new()).await.unwrap();
    assert_installed(dir.path(), &state);
    assert_eq!(bytes_to_fetch(entry, &model_dir), 0);
    // Installed: a full disk does not refuse the (no-op) download.
    let full = store(dir.path(), vec![Source::Base(server.uri())]).with_free_space(FreeSpace::fixed(0));
    assert!(full.download("tiny", Arc::new(|_| {}), CancelToken::new()).await.unwrap().is_installed());
    // A `.part` longer than the file starts over: all of that file is needed again.
    full.remove("tiny").unwrap();
    std::fs::create_dir_all(&model_dir).unwrap();
    std::fs::write(model_dir.join("tokens.txt.part"), vec![0u8; TOKENS_SIZE + 1]).unwrap();
    assert_eq!(bytes_to_fetch(entry, &model_dir), missing);
    // Free space unknown: the download goes ahead (the write fails on its own when the disk is full).
    let unknown = store(dir.path(), vec![Source::Base(server.uri())]).with_free_space(FreeSpace::from_fn(|_| Err(std::io::Error::other("statvfs refused"))));
    assert_installed(dir.path(), &unknown.download("tiny", Arc::new(|_| {}), CancelToken::new()).await.unwrap());
    // The real probe answers for a real directory.
    assert!(FreeSpace::system().at(dir.path()).unwrap() > 0);
}

#[tokio::test]
async fn wrong_sha256_fails_and_keeps_the_part_and_the_retry_starts_over() {
    let server = MockServer::start().await;
    let mut corrupt = model_bytes();
    corrupt[1000] ^= 0xff;
    Mock::given(method("GET"))
        .and(path(format!("/{REPO}/model.int8.onnx")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(corrupt))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/{REPO}/tokens.txt")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(tokens_bytes()))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path(), vec![Source::Base(server.uri())]);
    let (progress, seen) = sink();
    let err = store.download("tiny", progress, CancelToken::new()).await.unwrap_err();
    assert_eq!(err, StoreError::Checksum { file: "model.int8.onnx".into() });
    assert_eq!(String::from(err), "sha256 mismatch: model.int8.onnx");
    let part = dir.path().join("tiny/model.int8.onnx.part");
    assert_eq!(std::fs::metadata(&part).unwrap().len(), MODEL_SIZE as u64, ".part kept");
    assert!(!dir.path().join("tiny/model.int8.onnx").exists());
    assert!(!dir.path().join("tiny").join(MANIFEST_FILE).exists());
    assert_eq!(store.scan()[0].state, ModelInstallState::NotInstalled);
    assert_eq!(seen.lock().last(), Some(&ModelInstallState::Verifying));
    // Retry against a healthy source: the kept .part is re-checked, found wrong, and replaced.
    server.reset().await;
    serve_full(&server).await;
    let (progress, seen) = sink();
    let state = store.download("tiny", progress, CancelToken::new()).await.unwrap();
    assert_installed(dir.path(), &state);
    let seen = seen.lock().clone();
    assert_eq!(seen.first(), Some(&ModelInstallState::Verifying), "the full-size .part is verified first");
    assert_eq!(
        seen.get(1),
        Some(&ModelInstallState::Downloading { received: 0, total: MODEL_SIZE as u64, file: "model.int8.onnx".into() }),
        "then re-downloaded from zero"
    );
}

#[tokio::test]
async fn cancel_stops_the_download_and_keeps_the_part() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/{REPO}/model.int8.onnx")))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(30)).set_body_bytes(model_bytes()))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let model_dir = dir.path().join("tiny");
    std::fs::create_dir_all(&model_dir).unwrap();
    let part = model_dir.join("model.int8.onnx.part");
    std::fs::write(&part, &model_bytes()[..12_345]).unwrap();
    let store = store(dir.path(), vec![Source::Base(server.uri())]);
    let cancel = CancelToken::new();
    let (progress, seen) = sink();
    let canceller = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        canceller.cancel();
    });
    let started = std::time::Instant::now();
    let err = store.download("tiny", progress, cancel.clone()).await.unwrap_err();
    assert_eq!(err, StoreError::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(10), "cancel is prompt, not the server's delay");
    assert_eq!(std::fs::metadata(&part).unwrap().len(), 12_345, ".part kept for a resume");
    assert_eq!(seen.lock().as_slice(), &[ModelInstallState::Downloading { received: 12_345, total: MODEL_SIZE as u64, file: "model.int8.onnx".into() }]);
    assert!(cancel.is_cancelled());
    // A token cancelled before the call refuses at once.
    let (progress, seen) = sink();
    assert_eq!(store.download("tiny", progress, cancel).await.unwrap_err(), StoreError::Cancelled);
    assert!(seen.lock().is_empty());
}

#[tokio::test]
async fn sources_fall_back_in_order_and_a_short_body_resumes_on_the_next_source() {
    // Source A: 500 for the model, full tokens. Source B: only the first half of the model (200 with
    // a short body). Source C: honours the range for the rest. Everything ends up complete.
    let a = MockServer::start().await;
    let b = MockServer::start().await;
    let c = MockServer::start().await;
    let model = model_bytes();
    let half = MODEL_SIZE / 2;
    Mock::given(method("GET")).and(path(format!("/{REPO}/model.int8.onnx"))).respond_with(ResponseTemplate::new(500)).expect(1).mount(&a).await;
    Mock::given(method("GET"))
        .and(path(format!("/{REPO}/tokens.txt")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(tokens_bytes()))
        .expect(1)
        .mount(&a)
        .await;
    // `HfResolve` builds the `/resolve/main/` path, so B serves that shape.
    Mock::given(method("GET"))
        .and(path(format!("/{REPO}/resolve/main/model.int8.onnx")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(model[..half].to_vec()))
        .expect(1)
        .mount(&b)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/{REPO}/model.int8.onnx")))
        .and(header("range", format!("bytes={half}-")))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header("content-range", format!("bytes {half}-{}/{MODEL_SIZE}", MODEL_SIZE - 1))
                .set_body_bytes(model[half..].to_vec()),
        )
        .expect(1)
        .mount(&c)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path(), vec![Source::Base(a.uri()), Source::HfResolve(b.uri()), Source::Base(c.uri())]);
    let (progress, _) = sink();
    let state = store.download("tiny", progress, CancelToken::new()).await.unwrap();
    assert_installed(dir.path(), &state);
    assert_eq!(b.received_requests().await.unwrap().len(), 1, "B was asked once, on the resolve path");
}

#[tokio::test]
async fn every_source_failing_is_a_download_error_and_a_server_that_ignores_range_restarts() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(404)).mount(&server).await;
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path(), vec![Source::Base(server.uri()), Source::HfResolve(server.uri())]);
    let (progress, _) = sink();
    let err = store.download("tiny", progress, CancelToken::new()).await.unwrap_err();
    assert!(matches!(&err, StoreError::Download(m) if m.contains("404")), "{err}");
    assert!(err.to_string().starts_with("下载失败："));
    // A server that answers 200 to a ranged request: the .part is discarded and refetched whole.
    server.reset().await;
    serve_full(&server).await;
    let model_dir = dir.path().join("tiny");
    std::fs::write(model_dir.join("model.int8.onnx.part"), b"garbage that would never verify").unwrap();
    let (progress, seen) = sink();
    let state = store.download("tiny", progress, CancelToken::new()).await.unwrap();
    assert_installed(dir.path(), &state);
    let seen = seen.lock().clone();
    assert!(matches!(seen.first(), Some(ModelInstallState::Downloading { received: 31, .. })), "{:?}", seen.first());
    assert!(seen.iter().any(|s| matches!(s, ModelInstallState::Downloading { received: 0, file, .. } if file == "model.int8.onnx")), "restarted from zero");
    // A .part longer than the catalogue size is also thrown away.
    store.remove("tiny").unwrap();
    std::fs::create_dir_all(&model_dir).unwrap();
    std::fs::write(model_dir.join("model.int8.onnx.part"), vec![0u8; MODEL_SIZE + 1]).unwrap();
    let (progress, _) = sink();
    assert_installed(dir.path(), &store.download("tiny", progress, CancelToken::new()).await.unwrap());
}

/// Auxiliary entries (docs/dictation.md §12): with the flag on, downloading a recognition model
/// also installs the VAD; a VAD that cannot be fetched never fails the model; the flag turning on
/// later installs the missing VAD in the background; nothing happens twice.
#[tokio::test]
async fn auxiliary_entries_ride_along_when_wanted_and_never_fail_the_model() {
    let server = MockServer::start().await;
    serve_full(&server).await;
    let dir = tempfile::tempdir().unwrap();
    let wanted = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let store = store(dir.path(), vec![Source::Base(server.uri())]).with_auxiliary(wanted.clone());
    assert!(store.auxiliary_wanted());
    // The VAD is not served yet: the model installs, the VAD does not, no error.
    let (progress, seen) = sink();
    let state = store.download("tiny", progress, CancelToken::new()).await.unwrap();
    assert_installed(dir.path(), &state);
    assert_eq!(store.install_state("tiny-vad").unwrap(), ModelInstallState::NotInstalled);
    assert!(seen.lock().iter().any(|s| matches!(s, ModelInstallState::Downloading { file, .. } if file == "silero_vad.onnx")), "the attempt was reported");
    // Now served: the flag turning on (or staying on) fetches the missing VAD in the background.
    serve_aux(&server).await;
    assert!(store.spawn_auxiliary_download(), "a download was started");
    assert!(!store.spawn_auxiliary_download(), "not twice while one runs");
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !store.install_state("tiny-vad").unwrap().is_installed() && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(store.install_state("tiny-vad").unwrap().is_installed(), "the VAD arrived in the background");
    assert_eq!(std::fs::read(dir.path().join("tiny-vad").join("silero_vad.onnx")).unwrap(), aux_bytes());
    while store.spawn_auxiliary_download() {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!store.spawn_auxiliary_download(), "nothing missing: nothing started");
    let all = store.scan_all();
    assert_eq!(all.len(), 2);
    assert!(all.iter().all(|m| m.state.is_installed()));
    assert_eq!(store.scan().len(), 1, "still not a card");
    // Fresh library, flag on and the VAD served: one download installs both.
    let dir2 = tempfile::tempdir().unwrap();
    let store2 = ModelStore::with_sources(dir2.path(), catalogue(), vec![Source::Base(server.uri())]).with_auxiliary(wanted.clone());
    let (progress, _) = sink();
    store2.download("tiny", progress, CancelToken::new()).await.unwrap();
    assert!(store2.install_state("tiny-vad").unwrap().is_installed(), "the VAD rides along with the first model");
    // Flag off: neither the ride-along nor the background download happens.
    wanted.store(false, std::sync::atomic::Ordering::SeqCst);
    let dir3 = tempfile::tempdir().unwrap();
    let store3 = ModelStore::with_sources(dir3.path(), catalogue(), vec![Source::Base(server.uri())]).with_auxiliary(wanted);
    let (progress, _) = sink();
    store3.download("tiny", progress, CancelToken::new()).await.unwrap();
    assert_eq!(store3.install_state("tiny-vad").unwrap(), ModelInstallState::NotInstalled);
    assert!(!store3.spawn_auxiliary_download());
    // Installing the auxiliary entries directly, and removing one, work like any entry.
    let (progress, _) = sink();
    assert_eq!(store3.install_auxiliary(progress, CancelToken::new()).await.unwrap(), 1);
    assert!(store3.install_state("tiny-vad").unwrap().is_installed());
    store3.remove("tiny-vad").unwrap();
    assert_eq!(store3.install_state("tiny-vad").unwrap(), ModelInstallState::NotInstalled);
}

// --- slow sources (SlowSourcePolicy) ---------------------------------------------------------------

/// A minimal HTTP/1.1 file server that trickles every body out in `chunk`-byte writes `pause`
/// apart and honours `Range: bytes=N-`. Returns its base URL and the `(path, range)` of each request.
async fn trickle_server(files: Vec<(String, Vec<u8>)>, chunk: usize, pause: Duration) -> (String, Arc<Mutex<Vec<(String, Option<u64>)>>>) {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let files = Arc::new(files);
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { return };
            let files = files.clone();
            let log = log.clone();
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut buf = [0u8; 1024];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    let Ok(n) = socket.read(&mut buf).await else { return };
                    if n == 0 {
                        return;
                    }
                    request.extend_from_slice(&buf[..n]);
                }
                let text = String::from_utf8_lossy(&request).into_owned();
                let path = text.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let range = text
                    .lines()
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix("range: bytes=").map(str::to_owned))
                    .and_then(|r| r.trim_end_matches('-').trim().parse::<u64>().ok());
                log.lock().push((path.clone(), range));
                let Some((_, body)) = files.iter().find(|(p, _)| *p == path) else {
                    let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                    return;
                };
                let start = range.unwrap_or(0) as usize;
                let head = match range {
                    Some(from) => format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {from}-{}/{}\r\nConnection: close\r\n\r\n",
                        body.len() - start,
                        body.len() - 1,
                        body.len()
                    ),
                    None => format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()),
                };
                if socket.write_all(head.as_bytes()).await.is_err() {
                    return;
                }
                for piece in body[start..].chunks(chunk) {
                    if socket.write_all(piece).await.is_err() {
                        return; // the client moved on
                    }
                    tokio::time::sleep(pause).await;
                }
            });
        }
    });
    (base, seen)
}

fn tiny_files() -> Vec<(String, Vec<u8>)> {
    vec![(format!("/{REPO}/model.int8.onnx"), model_bytes()), (format!("/{REPO}/tokens.txt"), tokens_bytes())]
}

/// A mirror far below the floor (here ~80 KB/s against a 4 MB/s floor) is left after one window;
/// the next source resumes the `.part` where the slow one stopped instead of starting over.
#[tokio::test]
async fn a_source_below_the_speed_floor_hands_over_to_the_next_which_resumes_the_part() {
    let (slow, slow_seen) = trickle_server(tiny_files(), 4096, Duration::from_millis(50)).await;
    let (fast, fast_seen) = trickle_server(tiny_files(), 256 * 1024, Duration::from_millis(1)).await;
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path(), vec![Source::Base(slow), Source::Base(fast)])
        .with_slow_source_policy(SlowSourcePolicy { window: Duration::from_millis(300), min_bytes_per_sec: 4 * 1024 * 1024 });
    let (progress, _) = sink();
    let state = tokio::time::timeout(Duration::from_secs(30), store.download("tiny", progress, CancelToken::new())).await.expect("no hang").unwrap();
    assert_installed(dir.path(), &state);
    let model = format!("/{REPO}/model.int8.onnx");
    assert!(slow_seen.lock().iter().any(|(p, r)| *p == model && r.is_none()), "the slow mirror was tried first: {:?}", slow_seen.lock());
    let resumed = fast_seen.lock().iter().find(|(p, _)| *p == model).and_then(|(_, r)| *r);
    assert!(resumed.is_some_and(|from| from > 0 && (from as usize) < MODEL_SIZE), "the next source resumed with a Range: {:?}", fast_seen.lock());
}

/// A slow source that is the only one that works still finishes: after the first pass it gets a
/// patient second pass (no floor), resuming from the bytes it already delivered.
#[tokio::test]
async fn a_slow_source_that_is_the_only_working_one_still_finishes_on_the_second_pass() {
    let (slow, slow_seen) = trickle_server(tiny_files(), 64 * 1024, Duration::from_millis(20)).await;
    let dead = MockServer::start().await; // 404 for everything
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path(), vec![Source::Base(slow), Source::Base(dead.uri())])
        .with_slow_source_policy(SlowSourcePolicy { window: Duration::from_millis(200), min_bytes_per_sec: 64 * 1024 * 1024 });
    let (progress, _) = sink();
    let state = tokio::time::timeout(Duration::from_secs(30), store.download("tiny", progress, CancelToken::new())).await.expect("no hang").unwrap();
    assert_installed(dir.path(), &state);
    let model = format!("/{REPO}/model.int8.onnx");
    let requests: Vec<Option<u64>> = slow_seen.lock().iter().filter(|(p, _)| *p == model).map(|(_, r)| *r).collect();
    assert!(requests.len() >= 2 && requests[0].is_none() && requests[1].is_some_and(|from| from > 0), "second pass resumed on the slow source: {requests:?}");
    assert_eq!(StoreError::TooSlow { kib_per_sec: 12 }.to_string(), "下载源太慢：12 KB/s");
    assert_eq!(SlowSourcePolicy::default(), SlowSourcePolicy { window: Duration::from_secs(15), min_bytes_per_sec: 256 * 1024 });
}
