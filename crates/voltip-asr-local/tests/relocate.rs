#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Moving the model library to another folder (docs/dictation.md §10.9, issue #192): a rename on
//! one file system, a checked copy otherwise, the commit before the switch, and everything left
//! as it was when the move fails, is cancelled, or finds the new folder in the way.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use sha2::{Digest as _, Sha256};
use voltip_asr_local::{
    CATALOGUE_VERSION, Capability, Engine, FreeSpace, MANIFEST_FILE, Manifest, ModelEntry, ModelFile, ModelStore, STAGING_SUFFIX, Source, StoreError, Tier,
};
use voltip_core::{CancelToken, ModelManager, RelocateCommit};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MODEL_SIZE: usize = 3 * 1024 * 1024 + 517;
const TOKENS_SIZE: usize = 4096;
const AUX_SIZE: usize = 2048;

fn bytes(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed;
    (0..len)
        .map(|_| {
            x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (x >> 33) as u8
        })
        .collect()
}

fn sha(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

/// A recognition model and an auxiliary VAD with the fake bytes above (leaked: the store wants
/// `'static`).
fn catalogue() -> &'static [ModelEntry] {
    let files: &'static [ModelFile] = Box::leak(
        vec![
            ModelFile { name: "model.int8.onnx", size: MODEL_SIZE as u64, sha256: Box::leak(sha(&bytes(MODEL_SIZE, 7)).into_boxed_str()) },
            ModelFile { name: "tokens.txt", size: TOKENS_SIZE as u64, sha256: Box::leak(sha(&bytes(TOKENS_SIZE, 11)).into_boxed_str()) },
        ]
        .into_boxed_slice(),
    );
    let aux_files: &'static [ModelFile] = Box::leak(
        vec![ModelFile { name: "silero_vad.onnx", size: AUX_SIZE as u64, sha256: Box::leak(sha(&bytes(AUX_SIZE, 13)).into_boxed_str()) }].into_boxed_slice(),
    );
    Box::leak(
        vec![
            ModelEntry {
                id: "tiny",
                name: "Tiny",
                engine: Engine::SenseVoice,
                tier: Tier::Light,
                capabilities: &[Capability::Offline],
                languages: &["zh"],
                description: "test",
                recommended: true,
                repo: "test/tiny-model",
                package: None,
                files,
                max_piece: Duration::from_secs(119),
            },
            ModelEntry {
                id: "tiny-vad",
                name: "Tiny VAD",
                engine: Engine::SileroVad,
                tier: Tier::Auxiliary,
                capabilities: &[Capability::Vad],
                languages: &["zh"],
                description: "test",
                recommended: false,
                repo: "test/tiny-vad",
                package: None,
                files: aux_files,
                max_piece: Duration::ZERO,
            },
        ]
        .into_boxed_slice(),
    )
}

fn entry(id: &str) -> &'static ModelEntry {
    catalogue().iter().find(|e| e.id == id).unwrap()
}

/// `<root>/tiny`, complete: its files and the manifest a download writes.
fn install_tiny(root: &Path) -> PathBuf {
    let dir = root.join("tiny");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("model.int8.onnx"), bytes(MODEL_SIZE, 7)).unwrap();
    std::fs::write(dir.join("tokens.txt"), bytes(TOKENS_SIZE, 11)).unwrap();
    let manifest = Manifest {
        id: "tiny".into(),
        version: CATALOGUE_VERSION,
        downloaded_at: 1_758_700_000,
        files: entry("tiny").files.iter().map(|f| (f.name.to_owned(), f.sha256.to_owned())).collect(),
    };
    std::fs::write(dir.join(MANIFEST_FILE), serde_json::to_vec(&manifest).unwrap()).unwrap();
    dir
}

/// `<root>/tiny-vad`, half downloaded: a `.part` the next download resumes.
fn partial_vad(root: &Path) -> PathBuf {
    let dir = root.join("tiny-vad");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("silero_vad.onnx.part"), &bytes(AUX_SIZE, 13)[..1000]).unwrap();
    dir
}

fn store(root: &Path) -> ModelStore {
    ModelStore::with_sources(root, catalogue(), Vec::new())
}

fn installed(store: &ModelStore, id: &str) -> bool {
    store.install_state(id).unwrap().is_installed()
}

/// What the commit was called with, and what it saw of the disk at that moment.
#[derive(Default)]
struct Commits {
    calls: Vec<(PathBuf, Vec<PathBuf>)>,
    new_copy_complete: Vec<bool>,
}

/// A commit that records its calls; with `fail`, it refuses.
/// A commit as [`ModelStore::move_to`] takes it.
type CommitFn = Box<dyn Fn(&Path, &[PathBuf]) -> Result<(), String>>;

fn recording_commit(fail: bool) -> (CommitFn, Arc<Mutex<Commits>>) {
    let commits = Arc::new(Mutex::new(Commits::default()));
    let seen = commits.clone();
    let commit = move |to: &Path, doomed: &[PathBuf]| {
        let mut seen = seen.lock();
        seen.calls.push((to.to_path_buf(), doomed.to_vec()));
        let copy = to.join("tiny");
        seen.new_copy_complete.push(voltip_asr_local::store::is_installed(&copy, entry("tiny")));
        if fail { Err("disk full".to_owned()) } else { Ok(()) }
    };
    (Box::new(commit), commits)
}

fn no_progress(_: u64, _: u64) {}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir).map(|d| d.map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    names.sort();
    names
}

#[test]
fn a_move_on_one_file_system_renames_each_model_and_leaves_the_rest_of_the_folder_alone() {
    let base = tempfile::tempdir().unwrap();
    let from = base.path().join("data").join("models");
    install_tiny(&from);
    partial_vad(&from);
    // A probe an interrupted move left behind is not taken for this move's own.
    std::fs::write(from.join(".voltip-move-probe"), b"stale").unwrap();
    let to = base.path().join("chosen");
    std::fs::create_dir_all(to.join("other")).unwrap();
    std::fs::write(to.join("notes.txt"), b"mine").unwrap();
    let store = store(&from);
    let shared = store.shared_root();
    let (commit, commits) = recording_commit(false);
    let progress = Mutex::new(Vec::new());
    let leftovers = store.move_to(&to, &|moved, total| progress.lock().push((moved, total)), &CancelToken::new(), &commit).unwrap();
    assert!(leftovers.is_empty());
    assert_eq!(store.root(), to);
    assert_eq!(shared.get(), to, "everything that shares the root reads the new folder");
    assert!(installed(&store, "tiny"));
    assert_eq!(names(&to.join("tiny-vad")), ["silero_vad.onnx.part"], "a download in progress moves with its model");
    assert_eq!(names(&to), ["notes.txt", "other", "tiny", "tiny-vad"], "nothing else in the folder changed");
    assert_eq!(std::fs::read(to.join("notes.txt")).unwrap(), b"mine");
    assert!(names(&from).is_empty(), "the old folder holds no model");
    let commits = commits.lock();
    assert_eq!(commits.calls, [(to.clone(), Vec::new())], "renamed folders leave nothing to delete");
    assert_eq!(commits.new_copy_complete, [true], "the commit runs once the model is complete in the new place");
    assert_eq!(*progress.lock(), [(0, 0)], "a rename copies nothing");
}

#[test]
fn a_move_between_disks_copies_checks_commits_switches_and_then_deletes_the_old_copies() {
    let base = tempfile::tempdir().unwrap();
    let from = base.path().join("models");
    install_tiny(&from);
    partial_vad(&from);
    std::fs::create_dir_all(from.join("unrelated")).unwrap();
    let to = base.path().join("disk2").join("Voltip models");
    let store = store(&from).copying_moves();
    let shared = store.shared_root();
    let root_at_commit = Arc::new(Mutex::new(None));
    let (record, commits) = recording_commit(false);
    let at_commit = root_at_commit.clone();
    let commit = move |to: &Path, doomed: &[PathBuf]| {
        *at_commit.lock() = Some(shared.get());
        record(to, doomed)
    };
    let progress = Mutex::new(Vec::new());
    let leftovers = store.move_to(&to, &|moved, total| progress.lock().push((moved, total)), &CancelToken::new(), &commit).unwrap();
    assert!(leftovers.is_empty());
    assert_eq!(store.root(), to);
    assert!(installed(&store, "tiny"));
    assert_eq!(std::fs::read(to.join("tiny").join("model.int8.onnx")).unwrap(), bytes(MODEL_SIZE, 7));
    assert_eq!(names(&to.join("tiny-vad")), ["silero_vad.onnx.part"]);
    assert_eq!(names(&to), ["tiny", "tiny-vad"], "no staging folder is left");
    assert_eq!(names(&from), ["unrelated"], "the old copies are gone; what is not a model stays");
    assert_eq!(*root_at_commit.lock(), Some(from.clone()), "nothing reads the new folder before the commit");
    let commits = commits.lock();
    assert_eq!(commits.calls, [(to.clone(), vec![from.join("tiny"), from.join("tiny-vad")])]);
    assert_eq!(commits.new_copy_complete, [true]);
    let progress = progress.lock();
    let total = (MODEL_SIZE + TOKENS_SIZE + 1000) as u64 + std::fs::metadata(to.join("tiny").join(MANIFEST_FILE)).unwrap().len();
    assert_eq!(progress.first(), Some(&(0, total)));
    assert_eq!(progress.last(), Some(&(total, total)));
    assert!(progress.windows(2).all(|w| w[0].0 <= w[1].0 && w[1].1 == total), "{progress:?}");
}

#[test]
fn a_cancelled_or_failed_move_leaves_everything_where_it_was() {
    for copying in [false, true] {
        let base = tempfile::tempdir().unwrap();
        let from = base.path().join("models");
        install_tiny(&from);
        partial_vad(&from);
        let to = base.path().join("chosen");
        let store = if copying { store(&from).copying_moves() } else { store(&from) };

        // The commit cannot save the new place: the move is undone.
        let (commit, commits) = recording_commit(true);
        let err = store.move_to(&to, &no_progress, &CancelToken::new(), &commit).unwrap_err();
        assert_eq!(err, StoreError::Move("无法保存模型的新位置：disk full".into()), "copying {copying}");
        assert_eq!(commits.lock().calls.len(), 1);
        assert_eq!(store.root(), from);
        assert!(installed(&store, "tiny"), "copying {copying}");
        assert_eq!(names(&from), ["tiny", "tiny-vad"]);
        assert_eq!(names(&from.join("tiny-vad")), ["silero_vad.onnx.part"]);
        assert!(names(&to).is_empty(), "copying {copying}: {:?}", names(&to));
    }

    // Cancelled halfway through the copy.
    let base = tempfile::tempdir().unwrap();
    let from = base.path().join("models");
    install_tiny(&from);
    let to = base.path().join("chosen");
    let store = store(&from).copying_moves();
    let cancel = CancelToken::new();
    let (commit, commits) = recording_commit(false);
    let err = store
        .move_to(
            &to,
            &|moved, _| {
                if moved > 0 {
                    cancel.cancel();
                }
            },
            &cancel,
            &commit,
        )
        .unwrap_err();
    assert_eq!(err, StoreError::Cancelled);
    assert!(commits.lock().calls.is_empty());
    assert_eq!(store.root(), from);
    assert!(installed(&store, "tiny"));
    assert!(names(&to).is_empty(), "the half copy is gone: {:?}", names(&to));
}

#[test]
fn the_new_folder_must_have_room_and_nothing_else_under_a_models_name() {
    let base = tempfile::tempdir().unwrap();
    let from = base.path().join("models");
    install_tiny(&from);
    let to = base.path().join("chosen");
    // Not enough room for the copy, with the headroom.
    let small = store(&from).copying_moves().with_free_space(FreeSpace::fixed(101_000_000));
    let (commit, commits) = recording_commit(false);
    let err = small.move_to(&to, &no_progress, &CancelToken::new(), &commit).unwrap_err();
    let StoreError::MoveNoSpace { needed, available } = err.clone() else { panic!("{err:?}") };
    assert_eq!(available, 101_000_000);
    assert!(needed > 100_000_000 + MODEL_SIZE as u64, "{needed}");
    assert_eq!(err.to_string(), format!("所选位置空间不足：移动模型需要 {} MB 空闲空间，当前可用 101 MB", needed.div_ceil(1_000_000)));
    assert!(commits.lock().calls.is_empty());
    assert!(installed(&small, "tiny") && small.root() == from);

    // Something else under the model's name: nothing changes, and the folder is named.
    std::fs::create_dir_all(to.join("tiny")).unwrap();
    std::fs::write(to.join("tiny").join("model.int8.onnx"), b"downloaded by hand").unwrap();
    let store = store(&from);
    let err = store.move_to(&to, &no_progress, &CancelToken::new(), &commit).unwrap_err();
    assert_eq!(err, StoreError::Move(format!("所选文件夹里已经有 {}，而且不是完整的模型。请先把它移走或删除，再移动模型", to.join("tiny").display())));
    assert_eq!(std::fs::read(to.join("tiny").join("model.int8.onnx")).unwrap(), b"downloaded by hand");
    assert!(installed(&store, "tiny") && store.root() == from);

    // A complete copy there already is kept; an earlier, interrupted copy is cleared away.
    std::fs::remove_dir_all(to.join("tiny")).unwrap();
    install_tiny(&to);
    std::fs::write(to.join("tiny").join("kept.txt"), b"x").unwrap();
    std::fs::create_dir_all(to.join(format!("tiny{STAGING_SUFFIX}"))).unwrap();
    let store = store.copying_moves();
    let leftovers = store.move_to(&to, &no_progress, &CancelToken::new(), &commit).unwrap();
    assert!(leftovers.is_empty());
    assert!(to.join("tiny").join("kept.txt").exists(), "the copy already there was not replaced");
    assert_eq!(names(&to), ["tiny"]);
    assert!(!from.join("tiny").exists(), "the old copy went");
    assert_eq!(commits.lock().calls.last().unwrap().1, [from.join("tiny")]);
}

#[test]
fn a_move_into_a_models_own_folder_or_onto_itself_changes_nothing() {
    let base = tempfile::tempdir().unwrap();
    let from = base.path().join("models");
    install_tiny(&from);
    let store = store(&from);
    let (commit, commits) = recording_commit(false);
    for inside in [from.join("tiny"), from.join("tiny").join("deeper")] {
        let err = store.move_to(&inside, &no_progress, &CancelToken::new(), &commit).unwrap_err();
        assert_eq!(err, StoreError::Move("不能把模型移到模型自己的文件夹里，请选择别的文件夹".into()));
    }
    assert!(commits.lock().calls.is_empty());
    assert_eq!(names(&from.join("tiny")), ["manifest.json", "model.int8.onnx", "tokens.txt"], "a refused folder is not created");
    let err = store.move_to(Path::new("relative/models"), &no_progress, &CancelToken::new(), &commit).unwrap_err();
    assert_eq!(err, StoreError::Move("relative/models 不是完整的路径".into()));
    // The same folder by another spelling: nothing moves, the commit records it.
    let again = base.path().join("models").join("..").join("models");
    assert!(store.move_to(&again, &no_progress, &CancelToken::new(), &commit).unwrap().is_empty());
    assert_eq!(commits.lock().calls, [(again.clone(), Vec::new())]);
    assert!(installed(&store, "tiny"));
    assert_eq!(names(&from), ["tiny"], "no probe file is left behind");
}

#[test]
fn regression_the_same_folder_spelled_in_other_letter_case_is_not_a_move_where_names_ignore_case() {
    let base = tempfile::tempdir().unwrap();
    let from = base.path().join("models");
    install_tiny(&from);
    let shouting = base.path().join("MODELS");
    if !shouting.exists() {
        // A file system that tells the letter cases apart (Linux): another folder, another test.
        return;
    }
    let store = store(&from).copying_moves();
    let (commit, commits) = recording_commit(false);
    let leftovers = store.move_to(&shouting, &no_progress, &CancelToken::new(), &commit).unwrap();
    assert!(leftovers.is_empty());
    assert!(installed(&store, "tiny"), "the model is still there");
    assert_eq!(names(&from.join("tiny")), ["manifest.json", "model.int8.onnx", "tokens.txt"], "and nothing was added to it");
    assert_eq!(commits.lock().calls, [(shouting.clone(), Vec::new())], "nothing is deleted");
}

#[test]
fn nothing_else_writes_into_the_library_while_it_moves() {
    let base = tempfile::tempdir().unwrap();
    let from = base.path().join("models");
    install_tiny(&from);
    let store = store(&from).copying_moves();
    let refusals = Mutex::new(Vec::new());
    let (commit, _) = recording_commit(false);
    let to = base.path().join("chosen");
    store
        .move_to(
            &to,
            &|moved, _| {
                if moved > 0 && refusals.lock().is_empty() {
                    refusals.lock().push(store.remove("tiny").unwrap_err());
                    let elsewhere = base.path().join("elsewhere");
                    refusals.lock().push(store.move_to(&elsewhere, &no_progress, &CancelToken::new(), &commit).unwrap_err());
                }
            },
            &CancelToken::new(),
            &commit,
        )
        .unwrap();
    assert_eq!(*refusals.lock(), [StoreError::Busy("正在移动模型，请稍后再试"), StoreError::Busy("正在移动模型，请稍后再试")]);
    assert!(installed(&store, "tiny"), "the refused removal did not run");
    // Free again afterwards.
    assert_eq!(store.remove("tiny"), Ok(()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_move_waits_for_the_background_vad_download_and_that_download_waits_for_a_move() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/test/tiny-vad/silero_vad.onnx"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes(AUX_SIZE, 13)).set_delay(Duration::from_millis(300)))
        .mount(&server)
        .await;
    let base = tempfile::tempdir().unwrap();
    let from = base.path().join("models");
    install_tiny(&from);
    let store = ModelStore::with_sources(&from, catalogue(), vec![Source::Base(server.uri())]);
    assert!(store.spawn_auxiliary_fetch(), "the VAD download started");
    let commit: RelocateCommit = Arc::new(|_: &Path, _: &[PathBuf]| Ok(()));
    let to = base.path().join("chosen");
    let err = store.relocate(&to, Arc::new(|_, _| {}), CancelToken::new(), commit.clone()).await.unwrap_err();
    assert_eq!(err, "正在下载模型，请等下载结束后再移动");
    assert_eq!(store.root(), from);
    // Once it is done, the move goes, and the VAD moves with it.
    tokio::time::timeout(Duration::from_secs(30), async {
        while !installed(&store, "tiny-vad") || store.spawn_auxiliary_fetch() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let moved = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            match store.relocate(&to, Arc::new(|_, _| {}), CancelToken::new(), commit.clone()).await {
                Err(e) if e.starts_with("正在下载") => tokio::time::sleep(Duration::from_millis(20)).await,
                other => break other,
            }
        }
    })
    .await
    .unwrap()
    .unwrap();
    assert!(moved.leftovers.is_empty());
    assert_eq!(ModelManager::root(&store), to);
    assert!(installed(&store, "tiny") && installed(&store, "tiny-vad"));
}

#[cfg(unix)]
#[test]
fn an_old_copy_that_cannot_be_deleted_is_reported_for_the_next_start() {
    use std::os::unix::fs::PermissionsExt as _;
    let base = tempfile::tempdir().unwrap();
    let from = base.path().join("models");
    let old = install_tiny(&from);
    std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o555)).unwrap();
    // Some accounts may delete anyway (root): then there is nothing to report.
    let enforced = std::fs::write(old.join("probe"), b"").is_err();
    let to = base.path().join("chosen");
    let store = store(&from).copying_moves();
    let (commit, _) = recording_commit(false);
    let leftovers = store.move_to(&to, &no_progress, &CancelToken::new(), &commit).unwrap();
    // Gone when the account could delete it anyway.
    let _ = std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o755));
    assert!(installed(&store, "tiny"), "the move itself went through");
    if enforced {
        assert_eq!(leftovers, [old]);
    }
}
