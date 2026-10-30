#![allow(clippy::unwrap_used, clippy::expect_used)]
//! A model published as one tar.bz2 package (docs/dictation.md §29.12, the streaming model since
//! its Hugging Face repository was deleted), against servers on 127.0.0.1: the package comes
//! through a release redirect and only the catalogue's files are taken out of it, each checked;
//! a mirror that fails hands over to the package and Hugging Face is never asked; a damaged
//! package, a missing file and a file that is not the catalogue's each fail by name and leave
//! nothing behind; a cancel is prompt, and the next download resumes the package or unpacks the
//! one already whole; the free-space check counts the package; the manual import unpacks a package
//! put into the folder.
//!
//! `real_the_streaming_model_installs_from_its_release_package` is `#[ignore]`d: it needs a
//! downloaded copy of the real package (sherpa-onnx's release, 133,895,136 bytes):
//! `VOLTIP_STREAMING_PACKAGE=<file> cargo test --release -p voltip-asr-local --test package real_ -- --ignored --nocapture`.

use std::io::Write as _;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use sha2::{Digest as _, Sha256};
use voltip_asr_local::{
    Capability, DISK_HEADROOM, Engine, FreeSpace, ImportError, MANIFEST_FILE, Manifest, ModelEntry, ModelFile, ModelStore, Package, Source, StoreError, Tier,
};
use voltip_core::{CancelToken, ModelDownloadError, ModelInstallState, ModelManager, ModelPackageView, PackageProblem, ProgressSink};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ID: &str = "tiny-stream";
const REPO: &str = "test/tiny-stream";
/// The folder inside the package that holds the model's files.
const FOLDER: &str = "tiny-stream-2026-06-05";
const PACKAGE_PATH: &str = "k2-fsa/sherpa-onnx/releases/download/asr-models/tiny-stream-2026-06-05.tar.bz2";
const PACKAGE_NAME: &str = "tiny-stream-2026-06-05.tar.bz2";
/// Where the release redirect sends the client (GitHub sends it to a signed asset URL).
const ASSET_PATH: &str = "/assets/tiny-stream-2026-06-05.tar.bz2";
const SIZES: [(&str, usize, u64); 3] = [("encoder.int8.onnx", 300 * 1024 + 7, 3), ("decoder.onnx", 64 * 1024, 5), ("tokens.txt", 4096, 9)];

/// Deterministic pseudo-random bytes (an LCG). Not compressible, like the real model weights.
fn bytes(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed;
    (0..len)
        .map(|_| {
            x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (x >> 33) as u8
        })
        .collect()
}

fn content(name: &str) -> Vec<u8> {
    let (_, len, seed) = SIZES.iter().find(|(n, ..)| *n == name).unwrap();
    bytes(*len, *seed)
}

fn sha(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

/// One packaged entry whose hashes are those of [`content`] (leaked: the store wants `'static`).
fn catalogue() -> &'static [ModelEntry] {
    let files: Vec<ModelFile> =
        SIZES.iter().map(|(name, len, _)| ModelFile { name, size: *len as u64, sha256: Box::leak(sha(&content(name)).into_boxed_str()) }).collect();
    Box::leak(
        vec![ModelEntry {
            id: ID,
            name: "Tiny stream",
            engine: Engine::ZipformerStreaming,
            tier: Tier::Streaming,
            capabilities: &[Capability::Streaming],
            languages: &["zh", "en"],
            description: "test",
            recommended: false,
            repo: REPO,
            package: Some(Package { path: PACKAGE_PATH, dir: FOLDER }),
            files: Box::leak(files.into_boxed_slice()),
            max_piece: Duration::ZERO,
        }]
        .into_boxed_slice(),
    )
}

/// A tar.bz2 of `members` (path, bytes), regular files unless the path ends in `/`.
fn tar_bz2(members: &[(String, Vec<u8>)]) -> Vec<u8> {
    let encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
    let mut builder = tar::Builder::new(encoder);
    for (name, data) in members {
        let mut header = tar::Header::new_ustar();
        if name.ends_with('/') {
            header.set_entry_type(tar::EntryType::Directory);
            header.set_size(0);
            header.set_mode(0o755);
        } else {
            header.set_entry_type(tar::EntryType::Regular);
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
        }
        builder.append_data(&mut header, name, data.as_slice()).unwrap();
    }
    builder.into_inner().unwrap().finish().unwrap()
}

/// The release package as sherpa-onnx publishes one: the folder, the model's files in it in
/// another order than the catalogue's, a sample next to them, and a file of the same name in
/// another folder (never taken: only `<folder>/<name>` is the model's).
fn package_with(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut members = vec![(format!("{FOLDER}/"), Vec::new()), (format!("{FOLDER}/test_wavs/0.wav"), bytes(2000, 1))];
    members.extend(files.iter().rev().map(|(name, data)| (format!("{FOLDER}/{name}"), data.clone())));
    members.push(("other/tokens.txt".into(), b"not the model's".to_vec()));
    tar_bz2(&members)
}

fn package() -> Vec<u8> {
    package_with(&SIZES.map(|(name, ..)| (name, content(name))))
}

fn store(root: &Path, sources: Vec<Source>) -> ModelStore {
    ModelStore::with_sources(root, catalogue(), sources)
}

fn sink() -> (ProgressSink, Arc<Mutex<Vec<ModelInstallState>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = seen.clone();
    (Arc::new(move |state| s.lock().push(state)), seen)
}

/// The release path answers with a redirect to the asset, as GitHub does; the asset is `body`.
async fn serve_release(server: &MockServer, body: Vec<u8>) {
    Mock::given(method("GET"))
        .and(path(format!("/{PACKAGE_PATH}")))
        .respond_with(ResponseTemplate::new(302).insert_header("location", format!("{}{ASSET_PATH}", server.uri())))
        .mount(server)
        .await;
    Mock::given(method("GET")).and(path(ASSET_PATH)).respond_with(ResponseTemplate::new(200).set_body_bytes(body)).mount(server).await;
}

/// Only the model's files are in its folder: no package, no `.part`, nothing the package chose.
fn assert_installed(root: &Path, state: &ModelInstallState) {
    let dir = root.join(ID);
    assert!(matches!(state, ModelInstallState::Installed { path, .. } if Path::new(path) == dir), "{state:?}");
    for (name, ..) in SIZES {
        assert_eq!(std::fs::read(dir.join(name)).unwrap(), content(name), "{name}");
    }
    let manifest: Manifest = serde_json::from_slice(&std::fs::read(dir.join(MANIFEST_FILE)).unwrap()).unwrap();
    assert_eq!(manifest.files.len(), SIZES.len());
    let mut left: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    left.sort();
    let mut expected: Vec<String> = SIZES.iter().map(|(n, ..)| (*n).to_owned()).chain([MANIFEST_FILE.to_owned()]).collect();
    expected.sort();
    assert_eq!(left, expected, "only the model's files and its manifest are left");
}

/// What is in the model's folder after a failure (sorted names).
fn leftovers(root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root.join(ID)) else { return Vec::new() };
    let mut names: Vec<String> = entries.map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

#[tokio::test]
async fn a_model_published_as_a_package_installs_from_it_through_the_release_redirect() {
    let server = MockServer::start().await;
    let body = package();
    serve_release(&server, body.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path(), vec![Source::Release(server.uri())]);
    let (progress, seen) = sink();
    // Through the trait the core drives a download from the interface with.
    let manager: Arc<dyn ModelManager> = Arc::new(store.clone());
    let state = manager.download(ID, progress, CancelToken::new()).await.unwrap();
    assert_installed(dir.path(), &state);
    let seen = seen.lock().clone();
    // The package's progress, from the size the server said, then the check of every file.
    let total = body.len() as u64;
    assert_eq!(seen.first(), Some(&ModelInstallState::Downloading { received: 0, total, file: PACKAGE_NAME.into() }), "{:?}", seen.first());
    assert!(seen.iter().any(|s| *s == ModelInstallState::Downloading { received: total, total, file: PACKAGE_NAME.into() }));
    assert!(
        seen.iter()
            .filter(|s| matches!(s, ModelInstallState::Downloading { .. }))
            .all(|s| matches!(s, ModelInstallState::Downloading { file, .. } if file == PACKAGE_NAME))
    );
    assert!(seen.contains(&ModelInstallState::Verifying));
    // One request for the package, one after the redirect; nothing else.
    let requests: Vec<String> = server.received_requests().await.unwrap().iter().map(|r| r.url.path().to_owned()).collect();
    assert_eq!(requests, [format!("/{PACKAGE_PATH}"), ASSET_PATH.to_owned()]);
    // Installed: a second download fetches nothing.
    server.reset().await;
    assert!(store.download(ID, sink().0, CancelToken::new()).await.unwrap().is_installed());
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_failing_mirror_hands_over_to_the_package_and_hugging_face_is_not_asked() {
    let mirror = MockServer::start().await; // 404 for everything
    let hugging_face = MockServer::start().await;
    let release = MockServer::start().await;
    serve_release(&release, package()).await;
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path(), vec![Source::Base(mirror.uri()), Source::HfResolve(hugging_face.uri()), Source::Release(release.uri())]);
    let state = store.download(ID, sink().0, CancelToken::new()).await.unwrap();
    assert_installed(dir.path(), &state);
    let asked: Vec<String> = mirror.received_requests().await.unwrap().iter().map(|r| r.url.path().to_owned()).collect();
    assert_eq!(asked, [format!("/{REPO}/encoder.int8.onnx")], "the mirror was tried first, file by file; the package then brought every file");
    assert!(hugging_face.received_requests().await.unwrap().is_empty(), "Hugging Face no longer has the model");
    // The mirror serving the first file: the package brings only the rest, and the file the mirror
    // delivered is not replaced.
    let dir = tempfile::tempdir().unwrap();
    let mirror = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/{REPO}/encoder.int8.onnx")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(content("encoder.int8.onnx")))
        .mount(&mirror)
        .await;
    let release = MockServer::start().await;
    // A package whose encoder differs: taking the encoder out of it would fail the download.
    let mut other = content("encoder.int8.onnx");
    other[0] ^= 1;
    serve_release(&release, package_with(&[("encoder.int8.onnx", other), ("decoder.onnx", content("decoder.onnx")), ("tokens.txt", content("tokens.txt"))]))
        .await;
    let store = self::store(dir.path(), vec![Source::Base(mirror.uri()), Source::Release(release.uri())]);
    assert_installed(dir.path(), &store.download(ID, sink().0, CancelToken::new()).await.unwrap());
}

#[tokio::test]
async fn a_damaged_package_fails_as_broken_and_is_deleted() {
    let mut damaged = package();
    let middle = damaged.len() / 2;
    for b in &mut damaged[middle..middle + 64] {
        *b ^= 0x5a;
    }
    let cut = package()[..package().len() * 2 / 3].to_vec();
    for (body, what) in [(damaged, "damaged"), (cut, "cut short"), (b"<html>not a package</html>".to_vec(), "not bzip2")] {
        let server = MockServer::start().await;
        serve_release(&server, body).await;
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path(), vec![Source::Release(server.uri())]);
        let err = store.download(ID, sink().0, CancelToken::new()).await.unwrap_err();
        assert_eq!(err, StoreError::Package(PackageProblem::Broken), "{what}");
        assert_eq!(err.to_string(), "模型包无法使用：压缩包损坏或不完整");
        let left = leftovers(dir.path());
        assert!(left.iter().all(|n| SIZES.iter().any(|(f, ..)| f == n)), "{what}: no package and no .part left, only files that checked: {left:?}");
        assert_eq!(store.install_state(ID).unwrap(), ModelInstallState::NotInstalled);
        // The interface gets the problem itself, to say in its own language.
        let manager: &dyn ModelManager = &store;
        assert_eq!(
            manager.download(ID, sink().0, CancelToken::new()).await.unwrap_err(),
            ModelDownloadError { message: "模型包无法使用：压缩包损坏或不完整".into(), package: Some(PackageProblem::Broken) },
            "{what}"
        );
    }
}

#[tokio::test]
async fn a_package_without_a_file_of_the_model_names_it() {
    let server = MockServer::start().await;
    serve_release(&server, package_with(&[("encoder.int8.onnx", content("encoder.int8.onnx")), ("decoder.onnx", content("decoder.onnx"))])).await;
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path(), vec![Source::Release(server.uri())]);
    let err = store.download(ID, sink().0, CancelToken::new()).await.unwrap_err();
    assert_eq!(err, StoreError::Package(PackageProblem::Missing { files: vec!["tokens.txt".into()] }), "other/tokens.txt is not the model's");
    assert_eq!(err.to_string(), "模型包无法使用：包里没有 tokens.txt");
    assert_eq!(leftovers(dir.path()), ["decoder.onnx", "encoder.int8.onnx"], "the files that checked stay; no package, no .part");
    assert_eq!(store.install_state(ID).unwrap(), ModelInstallState::NotInstalled);
}

#[tokio::test]
async fn a_file_in_the_package_that_is_not_the_catalogue_s_fails_and_leaves_nothing_behind() {
    let mut changed = content("decoder.onnx");
    changed[100] ^= 1;
    let mut longer = content("decoder.onnx");
    longer.push(0);
    for (decoder, what) in [(changed, "another sha256"), (longer, "another size")] {
        let server = MockServer::start().await;
        let files = [("encoder.int8.onnx", content("encoder.int8.onnx")), ("decoder.onnx", decoder), ("tokens.txt", content("tokens.txt"))];
        serve_release(&server, package_with(&files)).await;
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path(), vec![Source::Release(server.uri())]);
        let err = store.download(ID, sink().0, CancelToken::new()).await.unwrap_err();
        assert_eq!(err, StoreError::Package(PackageProblem::Mismatched { files: vec!["decoder.onnx".into()] }), "{what}");
        assert_eq!(err.to_string(), "模型包无法使用：包里的 decoder.onnx 不是这个模型的文件");
        let left = leftovers(dir.path());
        assert!(!left.iter().any(|n| n == "decoder.onnx" || n.ends_with(".part") || n == PACKAGE_NAME), "{what}: {left:?}");
        assert_eq!(store.install_state(ID).unwrap(), ModelInstallState::NotInstalled);
    }
}

// --- cancel, resume, disk -------------------------------------------------------------------------

/// An HTTP/1.1 server on 127.0.0.1 that sends `body` at `path` in `chunk`-byte writes `pause`
/// apart, honours `Range: bytes=N-` (416 with `bytes */<size>` past the end) and logs each request's
/// range.
async fn trickle(path: String, body: Vec<u8>, chunk: usize, pause: Duration) -> (String, Arc<Mutex<Vec<Option<u64>>>>) {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let body = Arc::new(body);
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { return };
            let (body, log, path) = (body.clone(), log.clone(), path.clone());
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
                if text.split_whitespace().nth(1) != Some(path.as_str()) {
                    let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                    return;
                }
                let range = text
                    .lines()
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix("range: bytes=").map(str::to_owned))
                    .and_then(|r| r.trim_end_matches('-').trim().parse::<u64>().ok());
                log.lock().push(range);
                let start = range.unwrap_or(0) as usize;
                if start >= body.len() && range.is_some() {
                    let head = format!(
                        "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(head.as_bytes()).await;
                    return;
                }
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
                        return;
                    }
                    tokio::time::sleep(pause).await;
                }
            });
        }
    });
    (base, seen)
}

#[tokio::test]
async fn cancel_stops_a_package_download_promptly_and_the_next_download_resumes_it() {
    let body = package();
    let (slow, slow_seen) = trickle(format!("/{PACKAGE_PATH}"), body.clone(), 4096, Duration::from_millis(20)).await;
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path(), vec![Source::Release(slow)]);
    // Cancel as soon as some of the package is on disk.
    let cancel = CancelToken::new();
    let at = Arc::new(Mutex::new(None::<std::time::Instant>));
    let (canceller, mark) = (cancel.clone(), at.clone());
    let progress: ProgressSink = Arc::new(move |state| {
        if matches!(state, ModelInstallState::Downloading { received, .. } if received >= 16 * 1024) && !canceller.is_cancelled() {
            *mark.lock() = Some(std::time::Instant::now());
            canceller.cancel();
        }
    });
    let err = tokio::time::timeout(Duration::from_secs(60), store.download(ID, progress, cancel)).await.expect("no hang").unwrap_err();
    assert_eq!(err, StoreError::Cancelled);
    let cancelled_at = at.lock().expect("the download was cancelled while it ran");
    assert!(cancelled_at.elapsed() < Duration::from_secs(5), "cancel is prompt: {:?}", cancelled_at.elapsed());
    let part = dir.path().join(ID).join(format!("{PACKAGE_NAME}.part"));
    let kept = std::fs::metadata(&part).expect("the package's .part stays for a resume").len();
    assert!(kept >= 16 * 1024 && kept < body.len() as u64, "{kept}");
    assert_eq!(leftovers(dir.path()), [format!("{PACKAGE_NAME}.part")], "nothing unpacked yet");
    // The next download asks for the rest only.
    let (fast, fast_seen) = trickle(format!("/{PACKAGE_PATH}"), body.clone(), 64 * 1024, Duration::from_millis(1)).await;
    let store = self::store(dir.path(), vec![Source::Release(fast)]);
    let (progress, seen) = sink();
    let state = tokio::time::timeout(Duration::from_secs(60), store.download(ID, progress, CancelToken::new())).await.expect("no hang").unwrap();
    assert_installed(dir.path(), &state);
    assert_eq!(fast_seen.lock().as_slice(), &[Some(kept)], "resumed where the cancelled download stopped");
    assert_eq!(slow_seen.lock().as_slice(), &[None]);
    assert_eq!(
        seen.lock().first(),
        Some(&ModelInstallState::Downloading { received: kept, total: body.len() as u64, file: PACKAGE_NAME.into() }),
        "progress starts where the .part ended"
    );
}

#[tokio::test]
async fn a_cancel_while_unpacking_keeps_the_whole_package_and_the_next_download_does_not_fetch_it_again() {
    let server = MockServer::start().await;
    serve_release(&server, package()).await;
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path(), vec![Source::Release(server.uri())]);
    // The package is whole when the check starts: cancel right then.
    let cancel = CancelToken::new();
    let canceller = cancel.clone();
    let progress: ProgressSink = Arc::new(move |state| {
        if state == ModelInstallState::Verifying {
            canceller.cancel();
        }
    });
    assert_eq!(store.download(ID, progress, cancel).await.unwrap_err(), StoreError::Cancelled);
    assert_eq!(leftovers(dir.path()), [PACKAGE_NAME], "no half-written file; the whole package stays");
    server.reset().await; // nothing may be fetched again
    let state = store.download(ID, sink().0, CancelToken::new()).await.unwrap();
    assert_installed(dir.path(), &state);
    assert!(server.received_requests().await.unwrap().is_empty(), "unpacked from the package already on disk");
}

#[tokio::test]
async fn a_whole_package_from_an_earlier_try_is_not_downloaded_twice() {
    // The `.part` holds the whole package (the download was cut after its last byte): the server
    // answers the range past the end with 416, and the package is unpacked as it is.
    let body = package();
    let (server, seen) = trickle(format!("/{PACKAGE_PATH}"), body.clone(), 64 * 1024, Duration::from_millis(1)).await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(ID)).unwrap();
    std::fs::write(dir.path().join(ID).join(format!("{PACKAGE_NAME}.part")), &body).unwrap();
    let store = store(dir.path(), vec![Source::Release(server)]);
    assert_installed(dir.path(), &store.download(ID, sink().0, CancelToken::new()).await.unwrap());
    assert_eq!(seen.lock().as_slice(), &[Some(body.len() as u64)]);
}

#[tokio::test]
async fn a_package_that_does_not_fit_on_the_disk_fails_before_its_body() {
    let server = MockServer::start().await;
    let body = package();
    serve_release(&server, body.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let files: u64 = SIZES.iter().map(|(_, len, _)| *len as u64).sum();
    // Room for the files, not for the package next to them.
    let store = store(dir.path(), vec![Source::Release(server.uri())]).with_free_space(FreeSpace::fixed(files + DISK_HEADROOM));
    let err = store.download(ID, sink().0, CancelToken::new()).await.unwrap_err();
    assert_eq!(err, StoreError::NoSpace { needed: body.len() as u64 + files + DISK_HEADROOM, available: files + DISK_HEADROOM });
    assert!(leftovers(dir.path()).iter().all(|n| n == &format!("{PACKAGE_NAME}.part")), "{:?}", leftovers(dir.path()));
    assert_eq!(std::fs::metadata(dir.path().join(ID).join(format!("{PACKAGE_NAME}.part"))).map_or(0, |m| m.len()), 0, "no byte of it was written");
    // With room for both it installs.
    let store = self::store(dir.path(), vec![Source::Release(server.uri())]).with_free_space(FreeSpace::fixed(body.len() as u64 + files + DISK_HEADROOM));
    assert_installed(dir.path(), &store.download(ID, sink().0, CancelToken::new()).await.unwrap());
}

// --- manual download ------------------------------------------------------------------------------

#[tokio::test]
async fn the_card_offers_the_official_package_address_and_the_import_unpacks_a_package_put_into_the_folder() {
    let root = tempfile::tempdir().unwrap();
    // The build's own hosts are never shown: the card names the package at its official address.
    let store = store(
        root.path(),
        vec![
            Source::Base("https://mirror.example.test".into()),
            Source::HfResolve("https://huggingface.co".into()),
            Source::Release("http://127.0.0.1:9".into()),
        ],
    );
    let card = store.scan().into_iter().find(|m| m.id == ID).unwrap();
    assert_eq!(card.package, Some(ModelPackageView { name: PACKAGE_NAME.into(), urls: vec![format!("https://github.com/{PACKAGE_PATH}")] }));
    assert!(card.files.iter().all(|f| f.urls.is_empty()), "no file has an address of its own: {:?}", card.files);
    assert_eq!(card.files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), SIZES.map(|(n, ..)| n));
    // A damaged package put into the folder: the import says so and leaves the person's file.
    let dir = root.path().join(ID);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(PACKAGE_NAME), b"<html>a login page</html>").unwrap();
    assert_eq!(store.import(ID, sink().0).await, Err(ImportError::Store(StoreError::Package(PackageProblem::Broken))));
    assert!(dir.join(PACKAGE_NAME).exists(), "the person's file stays for them to replace");
    let manager: &dyn ModelManager = &store;
    assert!(matches!(
        manager.import(ID, sink().0).await,
        Err(voltip_core::ModelImportError::Package(ModelDownloadError { package: Some(PackageProblem::Broken), .. }))
    ));
    // The real package: unpacked, checked, installed; the package is not needed any more.
    std::fs::File::create(dir.join(PACKAGE_NAME)).unwrap().write_all(&package()).unwrap();
    let (progress, seen) = sink();
    let state = store.import(ID, progress).await.unwrap();
    assert_installed(root.path(), &state);
    assert!(seen.lock().iter().all(|s| *s == ModelInstallState::Verifying));
    // Files put there by hand, without the package, import as before.
    store.remove(ID).unwrap();
    std::fs::create_dir_all(&dir).unwrap();
    for (name, ..) in SIZES {
        std::fs::write(dir.join(name), content(name)).unwrap();
    }
    assert_installed(root.path(), &store.import(ID, sink().0).await.unwrap());
}

// --- the command line -----------------------------------------------------------------------------

/// `voltip --download-model` and `voltip-server --download-model` call this with the store the
/// interface's downloads go through too (`ModelStore::download`): the package path is the same.
#[test]
fn the_command_line_download_takes_the_package_like_the_interface() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (mirror, release) = rt.block_on(async {
        let mirror = MockServer::start().await; // 404 for everything
        let release = MockServer::start().await;
        serve_release(&release, package()).await;
        (mirror, release)
    });
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path(), vec![Source::Base(mirror.uri()), Source::Release(release.uri())]);
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = voltip_asr_local::cli::download_model(&store, ID, &mut out, &mut err);
    let (out, err) = (String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap());
    assert_eq!(code, voltip_asr_local::cli::ExitCode::Ok, "{err}");
    assert_eq!(out, format!("{ID}\tinstalled\t{}\n", root.path().join(ID).display()));
    assert!(err.lines().any(|l| l.starts_with(&format!("voltip: {PACKAGE_NAME} ")) && l.ends_with("(100%)")), "{err}");
    assert_installed(root.path(), &store.install_state(ID).unwrap());
    // A package that cannot be used: the reason on stderr, nothing on stdout.
    rt.block_on(async {
        release.reset().await;
        serve_release(&release, b"not a package".to_vec()).await;
    });
    let root = tempfile::tempdir().unwrap();
    let store = self::store(root.path(), vec![Source::Release(release.uri())]);
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = voltip_asr_local::cli::download_model(&store, ID, &mut out, &mut err);
    assert_eq!(code, voltip_asr_local::cli::ExitCode::Failed);
    assert!(out.is_empty());
    assert!(String::from_utf8(err).unwrap().ends_with(&format!("voltip: {ID}: 模型包无法使用：压缩包损坏或不完整\n")));
}

/// The real package, served from 127.0.0.1: the real catalogue's streaming entry installs from it,
/// and the time it took is printed.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs VOLTIP_STREAMING_PACKAGE (sherpa-onnx's release tar.bz2 of the streaming model)"]
async fn real_the_streaming_model_installs_from_its_release_package() {
    let file = std::env::var_os("VOLTIP_STREAMING_PACKAGE").expect("VOLTIP_STREAMING_PACKAGE names the package");
    let body = std::fs::read(file).unwrap();
    let entry = voltip_asr_local::streaming_entry();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/{}", entry.package.unwrap().path)))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
        .mount(&server)
        .await;
    let root = tempfile::tempdir().unwrap();
    let store = ModelStore::with_sources(root.path(), voltip_asr_local::CATALOGUE, vec![Source::Release(server.uri())]);
    let started = std::time::Instant::now();
    let state = store.download(entry.id, sink().0, CancelToken::new()).await.unwrap();
    println!("installed from the package in {:?}", started.elapsed());
    assert!(state.is_installed(), "{state:?}");
    let dir = root.path().join(entry.id);
    for file in entry.files() {
        assert_eq!(voltip_asr_local::store::sha256_file(&dir.join(file.name)).unwrap(), file.sha256, "{}", file.name);
    }
    let mut left: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    left.sort();
    assert_eq!(left, ["bpe.model", "decoder.onnx", "encoder.int8.onnx", "joiner.int8.onnx", MANIFEST_FILE, "tokens.txt"]);
}
