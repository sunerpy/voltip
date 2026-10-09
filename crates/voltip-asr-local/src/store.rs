//! The on-disk model library (docs/dictation.md §10): `<models_root>/<id>/` holds the model files,
//! a `manifest.json` once everything is verified, and `<file>.part` while a download is in flight.
//!
//! Download: sources are tried in order (`VOLTIP_MODEL_BASE_URL` mirror when the build has one,
//! huggingface.co, hf-mirror.com); each file streams into `<file>.part` with a `Range` header when a
//! partial file exists, then its sha256 is checked and only then is it flushed to the disk and
//! renamed into place (the manifest the same way), so a crash never leaves a final name over blocks
//! that did not reach the disk. A wrong hash is a failure that keeps the `.part` for the retry
//! (which re-checks it and starts over when it is still wrong). Cancellation stops between chunks
//! and keeps the `.part` too. Before the first byte, the bytes still missing plus
//! [`DISK_HEADROOM`] must fit in the free space of the library's file system.
//!
//! Auxiliary entries (docs/dictation.md §12: the Silero VAD, `Tier::Auxiliary`) are never listed
//! by [`ModelStore::scan`] — the UI has no card for them — and are installed as a dependency: after
//! every recognition model while the [`ModelStore::with_auxiliary`] flag (`vad_trim`) is on, or on
//! demand through [`ModelStore::spawn_auxiliary_download`] when the flag turns on later. Their
//! failure never fails the model they came with (the feature is fail-open).

use std::collections::BTreeMap;
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tokio::io::AsyncWriteExt as _;
use voltip_core::{CancelToken, ModelFileView, ModelImportError, ModelInstallState, ModelManager, ModelState, ProgressSink};

use crate::catalogue::{CATALOGUE, ModelEntry, ModelFile};

/// Name of the per-model manifest.
pub const MANIFEST_FILE: &str = "manifest.json";
/// Bumped when the on-disk layout or the catalogue's file set changes incompatibly.
pub const CATALOGUE_VERSION: u32 = 1;
/// Build-time first-choice mirror (`<base>/<repo>/<file>`); absent from source and docs.
pub const MODEL_BASE_URL_ENV: &str = "VOLTIP_MODEL_BASE_URL";
/// TCP connect deadline per attempt.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// A source that sends nothing for this long is abandoned for the next one.
pub const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// When a source is too slow to wait for: measured over each rolling `window`, a rate below
/// `min_bytes_per_sec` moves the download to the next source, which resumes the `.part` with a
/// `Range` request. The last source is never abandoned for being slow — only a stall
/// ([`READ_TIMEOUT`]) or an error ends it. Measured 2026-09-26 from one host: the build mirror
/// delivered 29 KB/s while huggingface.co delivered 3.9 MB/s; without this the 239 MB SenseVoice
/// model would have taken over two hours on the first source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlowSourcePolicy {
    /// Length of one measurement window.
    pub window: Duration,
    /// Minimum average rate over a window before another source is tried.
    pub min_bytes_per_sec: u64,
}

impl Default for SlowSourcePolicy {
    /// 15 s windows, 256 KiB/s (a 239 MB model in about 16 minutes at the floor).
    fn default() -> Self {
        Self { window: Duration::from_secs(15), min_bytes_per_sec: 256 * 1024 }
    }
}

/// Free space a download leaves on the disk: a model never fills the file system to the last byte.
pub const DISK_HEADROOM: u64 = 100_000_000;

/// Reads the free bytes under a path.
type FreeSpaceProbe = dyn Fn(&Path) -> std::io::Result<u64> + Send + Sync;

/// Free bytes on the file system that holds a path (tests pin the answer).
#[derive(Clone)]
pub struct FreeSpace(Arc<FreeSpaceProbe>);

impl FreeSpace {
    /// What the operating system says (`statvfs` / `GetDiskFreeSpaceExW`), for this user.
    pub fn system() -> Self {
        Self::from_fn(|path| fs4::available_space(path))
    }

    /// Always `bytes`.
    pub fn fixed(bytes: u64) -> Self {
        Self::from_fn(move |_| Ok(bytes))
    }

    /// Any probe.
    pub fn from_fn(probe: impl Fn(&Path) -> std::io::Result<u64> + Send + Sync + 'static) -> Self {
        Self(Arc::new(probe))
    }

    /// Free bytes under `path`.
    pub fn at(&self, path: &Path) -> std::io::Result<u64> {
        (self.0)(path)
    }
}

impl std::fmt::Debug for FreeSpace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FreeSpace")
    }
}

/// Written after every file of a model verified; its presence is what makes a model "installed".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Catalogue id.
    pub id: String,
    /// [`CATALOGUE_VERSION`] at download time.
    pub version: u32,
    /// Unix time in seconds when the last file verified.
    pub downloaded_at: u64,
    /// File name → verified sha256.
    pub files: BTreeMap<String, String>,
}

/// Where a file may be fetched from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// `<base>/<repo>/<file>` (a plain mirror of the repo trees).
    Base(String),
    /// `<base>/<repo>/resolve/main/<file>` (Hugging Face and its mirrors).
    HfResolve(String),
}

impl Source {
    /// The URL of `file` in `repo`.
    pub fn url(&self, repo: &str, file: &str) -> String {
        match self {
            Self::Base(base) => format!("{}/{repo}/{file}", base.trim_end_matches('/')),
            Self::HfResolve(base) => format!("{}/{repo}/resolve/main/{file}", base.trim_end_matches('/')),
        }
    }

    /// The documented order: the build's mirror (if any), huggingface.co, hf-mirror.com.
    pub fn defaults() -> Vec<Self> {
        let mut sources = Vec::with_capacity(3);
        if let Some(base) = option_env!("VOLTIP_MODEL_BASE_URL").map(str::trim).filter(|b| !b.is_empty()) {
            sources.push(Self::Base(base.to_owned()));
        }
        sources.push(Self::HfResolve("https://huggingface.co".into()));
        sources.push(Self::HfResolve("https://hf-mirror.com".into()));
        sources
    }
}

/// Why [`ModelStore::import`] did not install a model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    /// Catalogue files absent from the directory (or not of their size), and files there whose
    /// sha256 is not the catalogue's.
    Incomplete {
        /// Absent, or not of the catalogue size.
        missing: Vec<String>,
        /// The right size, the wrong content.
        mismatched: Vec<String>,
    },
    /// The library failed (unknown id, file system trouble).
    Store(StoreError),
}

/// Why a library operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    /// The id is not in the catalogue.
    #[error("目录中没有 {0}")]
    UnknownModel(String),
    /// The cancel token fired.
    #[error("已取消")]
    Cancelled,
    /// File system trouble.
    #[error("io: {0}")]
    Io(String),
    /// Every source failed; carries the last reason.
    #[error("下载失败：{0}")]
    Download(String),
    /// A source delivered less than the [`SlowSourcePolicy`] floor; the next source takes over.
    /// Never the final result of a download: sources dropped for this get a patient second pass.
    #[error("下载源太慢：{kib_per_sec} KB/s")]
    TooSlow {
        /// Measured rate over the last window.
        kib_per_sec: u64,
    },
    /// The downloaded file's sha256 is not the catalogue's.
    #[error("sha256 mismatch: {file}")]
    Checksum {
        /// Which file.
        file: String,
    },
    /// The bytes still missing plus [`DISK_HEADROOM`] do not fit on the disk; nothing was fetched.
    #[error("磁盘空间不足：此模型还需 {} MB 空闲空间，当前可用 {} MB", megabytes_up(.needed), megabytes(.available))]
    NoSpace {
        /// Bytes still to download plus the headroom.
        needed: u64,
        /// Free bytes on the library's file system.
        available: u64,
    },
}

/// Whole decimal megabytes, rounded down (what is free).
fn megabytes(bytes: &u64) -> u64 {
    bytes / 1_000_000
}

/// Whole decimal megabytes, rounded up (what is needed).
fn megabytes_up(bytes: &u64) -> u64 {
    bytes.div_ceil(1_000_000)
}

impl From<StoreError> for String {
    fn from(value: StoreError) -> Self {
        value.to_string()
    }
}

impl From<std::io::Error> for StoreError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value.to_string())
    }
}

/// The model library rooted at one directory.
#[derive(Clone, Debug)]
pub struct ModelStore {
    root: PathBuf,
    catalogue: &'static [ModelEntry],
    sources: Vec<Source>,
    /// Install the auxiliary entries along with every recognition model (docs/dictation.md §12
    /// `vad_trim`); shared with whoever tracks the setting.
    auxiliary: Arc<AtomicBool>,
    /// A background auxiliary download is running.
    auxiliary_in_flight: Arc<AtomicBool>,
    /// When to give up on a slow source for the next one.
    slow: SlowSourcePolicy,
    /// Where the free-space check before a download reads the disk.
    free_space: FreeSpace,
}

impl ModelStore {
    /// The real catalogue and the documented sources under `root` (`<app data dir>/models`).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self::with_sources(root, CATALOGUE, Source::defaults())
    }

    /// Any catalogue and sources (tests point at a local server with tiny files).
    pub fn with_sources(root: impl Into<PathBuf>, catalogue: &'static [ModelEntry], sources: Vec<Source>) -> Self {
        Self {
            root: root.into(),
            catalogue,
            sources,
            auxiliary: Arc::new(AtomicBool::new(false)),
            auxiliary_in_flight: Arc::new(AtomicBool::new(false)),
            slow: SlowSourcePolicy::default(),
            free_space: FreeSpace::system(),
        }
    }

    /// Replace the [`SlowSourcePolicy`] (tests use millisecond windows).
    pub fn with_slow_source_policy(self, slow: SlowSourcePolicy) -> Self {
        Self { slow, ..self }
    }

    /// Replace the free-space probe (tests pin a small disk).
    pub fn with_free_space(self, free_space: FreeSpace) -> Self {
        Self { free_space, ..self }
    }

    /// Share the flag that says whether the auxiliary entries (the VAD) are wanted: while it is
    /// `true`, every recognition model download also installs them.
    pub fn with_auxiliary(self, flag: Arc<AtomicBool>) -> Self {
        Self { auxiliary: flag, ..self }
    }

    /// Whether auxiliary entries are installed along with the recognition models right now.
    pub fn auxiliary_wanted(&self) -> bool {
        self.auxiliary.load(Ordering::SeqCst)
    }

    /// Flip the shared flag (the engine factory calls this with `EngineSettings.vad_trim`).
    pub fn set_auxiliary_wanted(&self, wanted: bool) {
        self.auxiliary.store(wanted, Ordering::SeqCst);
    }

    /// The library directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The catalogue this store serves.
    pub fn catalogue(&self) -> &'static [ModelEntry] {
        self.catalogue
    }

    /// The sources in the order they are tried.
    pub fn sources(&self) -> &[Source] {
        &self.sources
    }

    /// Directory of one model.
    pub fn dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    fn entry(&self, id: &str) -> Result<&'static ModelEntry, StoreError> {
        self.catalogue.iter().find(|e| e.id == id).ok_or_else(|| StoreError::UnknownModel(id.to_owned()))
    }

    /// Every catalogue entry the UI shows as a card, with its install state (`active` left
    /// `false`). Auxiliary entries are left out: they are dependencies, not choices.
    pub fn scan(&self) -> Vec<ModelState> {
        self.catalogue.iter().filter(|e| e.tier.is_visible()).map(|e| self.view(e)).collect()
    }

    /// Every catalogue entry, auxiliary ones included, with its install state.
    pub fn scan_all(&self) -> Vec<ModelState> {
        self.catalogue.iter().map(|e| self.view(e)).collect()
    }

    /// One entry as the UI sees it: its install state, its directory, and its files with the
    /// public addresses a manual download can use (docs/dictation.md §10).
    fn view(&self, entry: &ModelEntry) -> ModelState {
        let dir = self.dir(entry.id);
        let mut state = entry.state(installed_state(&dir, entry));
        state.dir = dir.to_string_lossy().into_owned();
        state.files =
            entry.files().iter().map(|f| ModelFileView { name: f.name.to_owned(), size_bytes: f.size, urls: self.public_urls(entry, f.name) }).collect();
        state
    }

    /// Where `file` of `entry` can be downloaded by hand: the Hugging Face sources in their order.
    /// A build's own mirror (`Source::Base`) is a host the interface never shows.
    pub fn public_urls(&self, entry: &ModelEntry, file: &str) -> Vec<String> {
        self.sources.iter().filter(|s| matches!(s, Source::HfResolve(_))).map(|s| s.url(entry.repo, file)).collect()
    }

    /// The auxiliary entries that are not installed yet.
    fn missing_auxiliary(&self) -> Vec<&'static ModelEntry> {
        self.catalogue.iter().filter(|e| !e.tier.is_visible() && !is_installed(&self.dir(e.id), e)).collect()
    }

    /// Install every missing auxiliary entry now. Errors are returned but never stop the caller
    /// from using the recognition models: trimming is fail-open.
    pub async fn install_auxiliary(&self, progress: ProgressSink, cancel: CancelToken) -> Result<usize, StoreError> {
        let mut installed = 0;
        for entry in self.missing_auxiliary() {
            self.install(entry, &progress, &cancel).await?;
            installed += 1;
        }
        Ok(installed)
    }

    /// Install the missing auxiliary entries on a background task when the flag is on and nothing
    /// is missing or running already; returns whether a download was started. Called from inside a
    /// Tokio runtime (the core task) whenever the setting changes.
    pub fn spawn_auxiliary_download(&self) -> bool {
        self.auxiliary_wanted() && self.spawn_auxiliary_fetch()
    }

    /// [`ModelStore::spawn_auxiliary_download`] whatever the flag says: a long take cuts at the
    /// pauses the VAD finds whether trimming is on or not (docs/dictation.md §22).
    pub fn spawn_auxiliary_fetch(&self) -> bool {
        if self.missing_auxiliary().is_empty() {
            return false;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            tracing::debug!("no Tokio runtime here; the auxiliary models wait for the next model download");
            return false;
        };
        if self.auxiliary_in_flight.swap(true, Ordering::SeqCst) {
            return false;
        }
        let store = self.clone();
        runtime.spawn(async move {
            match store.install_auxiliary(Arc::new(|_| {}), CancelToken::new()).await {
                Ok(n) => tracing::info!(installed = n, "auxiliary models installed"),
                Err(e) => tracing::warn!(error = %e, "auxiliary model download failed; what needs it stays off until the next try"),
            }
            store.auxiliary_in_flight.store(false, Ordering::SeqCst);
        });
        true
    }

    /// Install state of one entry.
    pub fn install_state(&self, id: &str) -> Result<ModelInstallState, StoreError> {
        let entry = self.entry(id)?;
        Ok(installed_state(&self.dir(id), entry))
    }

    /// Fetch and verify every file of `id`; see the module documentation for the resume and
    /// verification rules. `progress` receives `Downloading` (throttling is the caller's business),
    /// `Verifying`, and nothing after the returned `Installed`.
    pub async fn download(&self, id: &str, progress: ProgressSink, cancel: CancelToken) -> Result<ModelInstallState, StoreError> {
        let entry = self.entry(id)?;
        let state = self.install(entry, &progress, &cancel).await?;
        // A recognition model brings its dependencies along when they are wanted (§12); their
        // failure is logged, never propagated — the model itself is installed and usable.
        if entry.engine.is_offline() && self.auxiliary_wanted() {
            for aux in self.missing_auxiliary() {
                if let Err(e) = self.install(aux, &progress, &cancel).await {
                    tracing::warn!(model = aux.id, error = %e, "auxiliary model did not install; the feature that needs it stays off");
                }
            }
        }
        Ok(state)
    }

    /// Install `id` from files a person put into its directory: every catalogue file must be
    /// there with its size and sha256; then the manifest is written, as a download writes it.
    /// What is missing or not the catalogue's file comes back by name.
    pub async fn import(&self, id: &str, progress: ProgressSink) -> Result<ModelInstallState, ImportError> {
        let entry = self.entry(id).map_err(ImportError::Store)?;
        let dir = self.dir(id);
        progress(ModelInstallState::Verifying);
        let check_dir = dir.clone();
        let (missing, mismatched) = tokio::task::spawn_blocking(move || -> Result<(Vec<String>, Vec<String>), StoreError> {
            let (mut missing, mut mismatched) = (Vec::new(), Vec::new());
            for file in entry.files() {
                let path = check_dir.join(file.name);
                if !std::fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() == file.size) {
                    missing.push(file.name.to_owned());
                } else if sha256_file(&path)? != file.sha256 {
                    mismatched.push(file.name.to_owned());
                }
            }
            Ok((missing, mismatched))
        })
        .await
        .map_err(|e| ImportError::Store(StoreError::Io(e.to_string())))?
        .map_err(ImportError::Store)?;
        if !missing.is_empty() || !mismatched.is_empty() {
            tracing::info!(model = id, ?missing, ?mismatched, "manual import incomplete");
            return Err(ImportError::Incomplete { missing, mismatched });
        }
        let manifest = Manifest {
            id: entry.id.to_owned(),
            version: CATALOGUE_VERSION,
            downloaded_at: now_secs(),
            files: entry.files().iter().map(|f| (f.name.to_owned(), f.sha256.to_owned())).collect(),
        };
        write_manifest(&dir, &manifest).await.map_err(ImportError::Store)?;
        tracing::info!(model = id, dir = %dir.display(), "model imported by hand");
        // As after a download: the dependencies come along when they are wanted, best effort.
        if entry.engine.is_offline() {
            self.spawn_auxiliary_download();
        }
        Ok(ModelInstallState::Installed { path: dir.to_string_lossy().into_owned(), installed_at: manifest.downloaded_at })
    }

    /// Fetch and verify every file of `entry` and write its manifest.
    async fn install(&self, entry: &'static ModelEntry, progress: &ProgressSink, cancel: &CancelToken) -> Result<ModelInstallState, StoreError> {
        let dir = self.dir(entry.id);
        // Its files were verified before the manifest was written (by a download or an import):
        // downloading an installed model again fetches and hashes nothing.
        if let installed @ ModelInstallState::Installed { .. } = installed_state(&dir, entry) {
            return Ok(installed);
        }
        tokio::fs::create_dir_all(&dir).await?;
        self.check_space(entry, &dir)?;
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
            .user_agent(concat!("voltip/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| StoreError::Download(e.to_string()))?;
        let mut manifest = Manifest { id: entry.id.to_owned(), version: CATALOGUE_VERSION, downloaded_at: 0, files: BTreeMap::new() };
        for file in entry.files() {
            self.fetch_file(&client, entry, file, &dir, progress, cancel).await?;
            manifest.files.insert(file.name.to_owned(), file.sha256.to_owned());
        }
        manifest.downloaded_at = now_secs();
        write_manifest(&dir, &manifest).await?;
        tracing::info!(model = entry.id, dir = %dir.display(), "model installed");
        Ok(ModelInstallState::Installed { path: dir.to_string_lossy().into_owned(), installed_at: manifest.downloaded_at })
    }

    /// Refuse the download before its first byte when what is still missing, plus
    /// [`DISK_HEADROOM`], does not fit. A disk that cannot say how much is free does not block it:
    /// the write then fails on its own.
    fn check_space(&self, entry: &ModelEntry, dir: &Path) -> Result<(), StoreError> {
        let missing = bytes_to_fetch(entry, dir);
        if missing == 0 {
            return Ok(());
        }
        let needed = missing.saturating_add(DISK_HEADROOM);
        match self.free_space.at(dir) {
            Ok(available) if available < needed => {
                tracing::warn!(model = entry.id, needed, available, "not enough free space for the download");
                Err(StoreError::NoSpace { needed, available })
            }
            Ok(_) => Ok(()),
            Err(e) => {
                tracing::warn!(model = entry.id, error = %e, "free space unknown; downloading anyway");
                Ok(())
            }
        }
    }

    async fn fetch_file(
        &self,
        client: &reqwest::Client,
        entry: &ModelEntry,
        file: &ModelFile,
        dir: &Path,
        progress: &ProgressSink,
        cancel: &CancelToken,
    ) -> Result<(), StoreError> {
        let final_path = dir.join(file.name);
        if tokio::fs::metadata(&final_path).await.is_ok_and(|m| m.len() == file.size) {
            // A file under its final name passed the hash check on an earlier run, or a person put
            // it there (a manual download, docs/dictation.md §10): check it rather than trust it.
            progress(ModelInstallState::Verifying);
            let path = final_path.clone();
            let digest = tokio::task::spawn_blocking(move || sha256_file(&path)).await.map_err(|e| StoreError::Io(e.to_string()))??;
            if digest == file.sha256 {
                return Ok(());
            }
            tracing::warn!(file = file.name, "a file under its final name has the wrong sha256; downloading it again");
            tokio::fs::remove_file(&final_path).await?;
        }
        let part = dir.join(format!("{}.part", file.name));
        let mut received = tokio::fs::metadata(&part).await.map(|m| m.len()).unwrap_or(0);
        if received > file.size {
            tracing::warn!(file = file.name, received, expected = file.size, "partial file longer than the catalogue size; starting over");
            truncate(&part, 0).await?;
            received = 0;
        }
        if received == file.size {
            // A previous attempt got everything but failed (or was cancelled) before the rename:
            // check it; when it is still wrong, start over rather than fail forever.
            progress(ModelInstallState::Verifying);
            match verify_and_rename(&part, &final_path, file).await {
                Ok(()) => return Ok(()),
                Err(StoreError::Checksum { .. }) => {
                    tracing::warn!(file = file.name, "kept partial file has the wrong sha256; starting over");
                    truncate(&part, 0).await?;
                    received = 0;
                }
                Err(e) => return Err(e),
            }
        }
        let mut last_error = String::from("没有可用的下载源");
        let mut complete = false;
        // First pass: a source with another one behind it may be dropped for being slow. Second
        // pass: the sources dropped that way, patiently — a slow source that is the only one that
        // works still finishes the download.
        let mut slow_ones = Vec::new();
        'passes: for pass in 0..2 {
            let order: Vec<usize> = if pass == 0 { (0..self.sources.len()).collect() } else { std::mem::take(&mut slow_ones) };
            for index in order {
                if cancel.is_cancelled() {
                    return Err(StoreError::Cancelled);
                }
                let url = self.sources[index].url(entry.repo, file.name);
                let slow = (pass == 0 && index + 1 < self.sources.len()).then_some(self.slow);
                match self.stream(client, &url, &part, file, &mut received, progress, cancel, slow).await {
                    Ok(()) => {
                        complete = true;
                        break 'passes;
                    }
                    Err(StoreError::Cancelled) => return Err(StoreError::Cancelled),
                    Err(e @ StoreError::TooSlow { .. }) => {
                        tracing::info!(file = file.name, source = %redact(&url), error = %e, received, "source too slow; trying the next one");
                        slow_ones.push(index);
                        last_error = e.to_string();
                    }
                    Err(e) => {
                        tracing::warn!(file = file.name, source = %redact(&url), error = %e, received, "source failed; trying the next one");
                        last_error = e.to_string();
                    }
                }
            }
        }
        if !complete {
            return Err(StoreError::Download(last_error));
        }
        progress(ModelInstallState::Verifying);
        verify_and_rename(&part, &final_path, file).await
    }

    /// Stream `url` into `part`, resuming at `*received`. `Ok` means the file is complete on disk.
    #[allow(clippy::too_many_arguments)]
    async fn stream(
        &self,
        client: &reqwest::Client,
        url: &str,
        part: &Path,
        file: &ModelFile,
        received: &mut u64,
        progress: &ProgressSink,
        cancel: &CancelToken,
        slow: Option<SlowSourcePolicy>,
    ) -> Result<(), StoreError> {
        let report = |received: u64| progress(ModelInstallState::Downloading { received, total: file.size, file: file.name.to_owned() });
        report(*received);
        let mut request = client.get(url);
        if *received > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={}-", *received));
        }
        let mut response = tokio::select! {
            r = request.send() => r.map_err(|e| StoreError::Download(e.to_string()))?,
            () = cancel.cancelled() => return Err(StoreError::Cancelled),
        };
        match response.status() {
            reqwest::StatusCode::PARTIAL_CONTENT => {
                // Trust the offset only when the server confirms it.
                let confirmed = response
                    .headers()
                    .get(reqwest::header::CONTENT_RANGE)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.strip_prefix("bytes "))
                    .and_then(|v| v.split('-').next())
                    .and_then(|v| v.parse::<u64>().ok());
                if confirmed != Some(*received) {
                    tracing::warn!(file = file.name, ?confirmed, received = *received, "server resumed from a different offset; starting over");
                    truncate(part, 0).await?;
                    *received = 0;
                    return Err(StoreError::Download("range offset mismatch".into()));
                }
            }
            reqwest::StatusCode::OK => {
                if *received > 0 {
                    tracing::info!(file = file.name, "server ignored the range; starting over");
                    truncate(part, 0).await?;
                    *received = 0;
                    report(0);
                }
            }
            reqwest::StatusCode::RANGE_NOT_SATISFIABLE if *received == file.size => return Ok(()),
            status => return Err(StoreError::Download(format!("HTTP {status}"))),
        }
        let mut out = tokio::fs::OpenOptions::new().create(true).append(true).open(part).await?;
        // Rolling throughput window (see `SlowSourcePolicy`): start instant and byte count.
        let mut window = (Instant::now(), *received);
        loop {
            let chunk = tokio::select! {
                c = response.chunk() => c.map_err(|e| StoreError::Download(e.to_string()))?,
                () = cancel.cancelled() => {
                    out.flush().await?;
                    return Err(StoreError::Cancelled);
                }
            };
            let Some(chunk) = chunk else { break };
            if *received + chunk.len() as u64 > file.size {
                out.flush().await?;
                truncate(part, 0).await?;
                *received = 0;
                return Err(StoreError::Download(format!("{} is longer than the catalogue says", file.name)));
            }
            out.write_all(&chunk).await?;
            *received += chunk.len() as u64;
            report(*received);
            if let Some(policy) = slow {
                let elapsed = window.0.elapsed();
                if elapsed >= policy.window {
                    let bytes = *received - window.1;
                    let rate = u64::try_from(u128::from(bytes) * 1000 / elapsed.as_millis().max(1)).unwrap_or(u64::MAX);
                    if rate < policy.min_bytes_per_sec {
                        out.flush().await?;
                        return Err(StoreError::TooSlow { kib_per_sec: rate / 1024 });
                    }
                    window = (Instant::now(), *received);
                }
            }
        }
        out.flush().await?;
        if *received == file.size { Ok(()) } else { Err(StoreError::Download(format!("connection closed after {} of {} bytes", *received, file.size))) }
    }

    /// Delete the model directory: files, `.part`s and manifest.
    pub fn remove(&self, id: &str) -> Result<(), StoreError> {
        self.entry(id)?;
        let dir = self.dir(id);
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => {
                tracing::info!(model = id, "model removed");
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

#[async_trait]
impl ModelManager for ModelStore {
    fn scan(&self) -> Vec<ModelState> {
        Self::scan(self)
    }

    async fn download(&self, id: &str, progress: ProgressSink, cancel: CancelToken) -> Result<ModelInstallState, String> {
        Self::download(self, id, progress, cancel).await.map_err(String::from)
    }

    fn remove(&self, id: &str) -> Result<(), String> {
        Self::remove(self, id).map_err(String::from)
    }

    async fn import(&self, id: &str, progress: ProgressSink) -> Result<ModelInstallState, ModelImportError> {
        Self::import(self, id, progress).await.map_err(|e| match e {
            ImportError::Incomplete { missing, mismatched } => ModelImportError::Incomplete { missing, mismatched },
            ImportError::Store(e) => ModelImportError::Failed(String::from(e)),
        })
    }
}

/// `Installed` when the manifest is there, names this entry and version, and every file is on
/// disk with the catalogue's size and the manifest's (verified) hash; `NotInstalled` otherwise.
pub fn installed_state(dir: &Path, entry: &ModelEntry) -> ModelInstallState {
    match read_manifest(dir) {
        Some(m) if is_complete(dir, entry, &m) => ModelInstallState::Installed { path: dir.to_string_lossy().into_owned(), installed_at: m.downloaded_at },
        _ => ModelInstallState::NotInstalled,
    }
}

/// Whether `dir` holds a verified copy of `entry` (what the transcriber asks before loading).
pub fn is_installed(dir: &Path, entry: &ModelEntry) -> bool {
    installed_state(dir, entry).is_installed()
}

fn is_complete(dir: &Path, entry: &ModelEntry, manifest: &Manifest) -> bool {
    manifest.id == entry.id
        && manifest.version == CATALOGUE_VERSION
        && entry
            .files()
            .iter()
            .all(|f| manifest.files.get(f.name).is_some_and(|sha| sha == f.sha256) && std::fs::metadata(dir.join(f.name)).is_ok_and(|m| m.len() == f.size))
}

/// The manifest, or `None` when absent or unreadable (either way: not installed).
pub fn read_manifest(dir: &Path) -> Option<Manifest> {
    let bytes = std::fs::read(dir.join(MANIFEST_FILE)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Bytes still to download for `entry` into `dir`: every file not yet under its final name with the
/// catalogue size, less what its `.part` already holds (a `.part` longer than the file starts over).
pub fn bytes_to_fetch(entry: &ModelEntry, dir: &Path) -> u64 {
    entry
        .files()
        .iter()
        .map(|f| {
            if std::fs::metadata(dir.join(f.name)).is_ok_and(|m| m.len() == f.size) {
                return 0;
            }
            let part = std::fs::metadata(dir.join(format!("{}.part", f.name))).map_or(0, |m| m.len());
            if part <= f.size { f.size - part } else { f.size }
        })
        .sum()
}

async fn write_manifest(dir: &Path, manifest: &Manifest) -> Result<(), StoreError> {
    let tmp = dir.join(format!("{MANIFEST_FILE}.tmp"));
    let bytes = serde_json::to_vec_pretty(manifest).map_err(|e| StoreError::Io(e.to_string()))?;
    tokio::fs::write(&tmp, bytes).await?;
    let target = dir.join(MANIFEST_FILE);
    tokio::task::spawn_blocking(move || publish(&tmp, &target)).await.map_err(|e| StoreError::Io(e.to_string()))??;
    Ok(())
}

/// Give `from` its final name durably: flush its blocks to the disk, rename it over `to`, then flush
/// the directory entry. Blocking.
fn publish(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::OpenOptions::new().write(true).open(from)?.sync_all()?;
    std::fs::rename(from, to)?;
    sync_dir(to.parent());
    Ok(())
}

/// Flush a directory's entries (Unix). Best effort: some file systems refuse it, and the file's own
/// blocks are already on the disk. Windows has no directory handle to flush; NTFS journals the rename.
fn sync_dir(dir: Option<&Path>) {
    #[cfg(unix)]
    if let Some(dir) = dir
        && let Err(e) = std::fs::File::open(dir).and_then(|d| d.sync_all())
    {
        tracing::debug!(dir = %dir.display(), error = %e, "directory flush refused");
    }
    #[cfg(not(unix))]
    let _ = dir;
}

async fn truncate(path: &Path, len: u64) -> Result<(), StoreError> {
    let file = tokio::fs::OpenOptions::new().create(true).truncate(false).write(true).open(path).await?;
    file.set_len(len).await?;
    Ok(())
}

/// sha256 of `part` on a blocking thread; on a match the file takes its final name ([`publish`]).
async fn verify_and_rename(part: &Path, final_path: &Path, file: &ModelFile) -> Result<(), StoreError> {
    let path = part.to_path_buf();
    let digest = tokio::task::spawn_blocking(move || sha256_file(&path)).await.map_err(|e| StoreError::Io(e.to_string()))??;
    if digest != file.sha256 {
        tracing::warn!(file = file.name, expected = file.sha256, actual = %digest, "sha256 mismatch; keeping the partial file");
        return Err(StoreError::Checksum { file: file.name.to_owned() });
    }
    let (from, to) = (part.to_path_buf(), final_path.to_path_buf());
    tokio::task::spawn_blocking(move || publish(&from, &to)).await.map_err(|e| StoreError::Io(e.to_string()))??;
    Ok(())
}

/// Lower-case hex sha256 of a whole file.
pub fn sha256_file(path: &Path) -> Result<String, StoreError> {
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(0))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex_lower(&hasher.finalize()))
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Host + path only, so a log line never carries a query string.
fn redact(url: &str) -> String {
    url.split('?').next().unwrap_or(url).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_build_the_documented_urls_in_order() {
        let repo = "csukuangfj/sherpa-onnx-paraformer-zh-2024-03-09";
        assert_eq!(Source::Base("https://mirror.example.test/".into()).url(repo, "tokens.txt"), format!("https://mirror.example.test/{repo}/tokens.txt"));
        assert_eq!(
            Source::HfResolve("https://huggingface.co".into()).url(repo, "model.int8.onnx"),
            format!("https://huggingface.co/{repo}/resolve/main/model.int8.onnx")
        );
        let defaults = Source::defaults();
        let public: Vec<&Source> = defaults.iter().rev().take(2).collect();
        assert_eq!(public[1], &Source::HfResolve("https://huggingface.co".into()), "huggingface.co before its mirror");
        assert_eq!(public[0], &Source::HfResolve("https://hf-mirror.com".into()));
        assert!(defaults.len() == 2 || matches!(defaults[0], Source::Base(_)), "an optional build mirror comes first");
        assert_eq!(MODEL_BASE_URL_ENV, "VOLTIP_MODEL_BASE_URL");
        assert_eq!(redact("https://h.test/a/b?token=x"), "https://h.test/a/b");
        assert_eq!(hex_lower(&[0, 255, 16]), "00ff10");
        assert_eq!(StoreError::UnknownModel("x".into()).to_string(), "目录中没有 x");
        assert_eq!(String::from(StoreError::Checksum { file: "f".into() }), "sha256 mismatch: f");
        assert_eq!(StoreError::from(std::io::Error::other("disk")), StoreError::Io("disk".into()));
    }

    #[test]
    fn manifest_roundtrips_and_decides_installed() {
        let dir = tempfile::tempdir().unwrap();
        let entry = crate::catalogue::entry("paraformer-zh").unwrap();
        let (model_file, tokens_file) = (entry.file("model.int8.onnx").unwrap(), entry.file("tokens.txt").unwrap());
        let model_dir = dir.path().join(entry.id);
        std::fs::create_dir_all(&model_dir).unwrap();
        assert_eq!(installed_state(&model_dir, entry), ModelInstallState::NotInstalled);
        // Files of the right size but no manifest: not installed (never verified).
        for f in entry.files() {
            let file = std::fs::File::create(model_dir.join(f.name)).unwrap();
            file.set_len(f.size).unwrap();
        }
        assert!(!is_installed(&model_dir, entry));
        let manifest = Manifest {
            id: entry.id.into(),
            version: CATALOGUE_VERSION,
            downloaded_at: 1_758_700_000,
            files: entry.files().iter().map(|f| (f.name.to_owned(), f.sha256.to_owned())).collect(),
        };
        std::fs::write(model_dir.join(MANIFEST_FILE), serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert_eq!(read_manifest(&model_dir), Some(manifest.clone()));
        assert_eq!(
            installed_state(&model_dir, entry),
            ModelInstallState::Installed { path: model_dir.to_string_lossy().into_owned(), installed_at: 1_758_700_000 }
        );
        // A file that shrank, a foreign manifest, an old version, a wrong hash: all not installed.
        std::fs::File::create(model_dir.join(tokens_file.name)).unwrap().set_len(tokens_file.size - 1).unwrap();
        assert!(!is_installed(&model_dir, entry));
        std::fs::File::create(model_dir.join(tokens_file.name)).unwrap().set_len(tokens_file.size).unwrap();
        assert!(is_installed(&model_dir, entry));
        let other = Manifest { id: "sense-voice-small".into(), ..manifest.clone() };
        std::fs::write(model_dir.join(MANIFEST_FILE), serde_json::to_vec(&other).unwrap()).unwrap();
        assert!(!is_installed(&model_dir, entry));
        let old = Manifest { version: CATALOGUE_VERSION + 1, ..manifest.clone() };
        std::fs::write(model_dir.join(MANIFEST_FILE), serde_json::to_vec(&old).unwrap()).unwrap();
        assert!(!is_installed(&model_dir, entry));
        let mut bad = manifest.clone();
        bad.files.insert(model_file.name.into(), "00".repeat(32));
        std::fs::write(model_dir.join(MANIFEST_FILE), serde_json::to_vec(&bad).unwrap()).unwrap();
        assert!(!is_installed(&model_dir, entry));
        std::fs::write(model_dir.join(MANIFEST_FILE), b"not json").unwrap();
        assert_eq!(read_manifest(&model_dir), None);
        // The store's scan reports every catalogue entry.
        let store = ModelStore::new(dir.path());
        assert_eq!(store.root(), dir.path());
        assert_eq!(store.catalogue().len(), CATALOGUE.len());
        assert!(store.sources().len() >= 2);
        assert!(!store.auxiliary_wanted(), "auxiliary entries are not wanted until the setting says so");
        let scan = store.scan();
        assert_eq!(scan.len(), CATALOGUE.len() - 1, "the auxiliary VAD entry is not a card");
        assert!(scan.iter().all(|m| m.state == ModelInstallState::NotInstalled && !m.active && !m.is_vad()));
        let all = store.scan_all();
        assert_eq!(all.len(), CATALOGUE.len());
        assert_eq!(all.iter().filter(|m| m.is_vad()).count(), 1);
        assert_eq!(store.install_state(crate::catalogue::VAD_MODEL_ID).unwrap(), ModelInstallState::NotInstalled);
        assert_eq!(store.install_state("paraformer-zh").unwrap(), ModelInstallState::NotInstalled);
        assert_eq!(store.install_state("ghost"), Err(StoreError::UnknownModel("ghost".into())));
        assert_eq!(store.remove("ghost"), Err(StoreError::UnknownModel("ghost".into())));
        assert_eq!(store.remove("paraformer-zh"), Ok(()));
        assert!(!model_dir.exists());
        assert_eq!(store.remove("paraformer-zh"), Ok(()), "removing an absent model is fine");
        assert_eq!(sha256_file(&dir.path().join("missing")).unwrap_err().to_string().split(':').next(), Some("io"));
        std::fs::write(dir.path().join("abc"), b"abc").unwrap();
        assert_eq!(sha256_file(&dir.path().join("abc")).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
