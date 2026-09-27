//! In-app updates (docs/dictation.md §9): the desktop shell owns `tauri-plugin-updater`, drives it
//! from Rust and reports through the bridge as [`UiEvent::Update`], so `core_state` and the event
//! stream tell every window the same [`UpdateStatus`].
//!
//! **Where the update source comes from.** The manifest URL and the minisign public key are baked
//! in at compile time from [`UPDATE_URL_ENV`] / [`UPDATE_PUBKEY_ENV`] (`option_env!`), exactly like
//! the engine defaults in `voltip-core::engines`: neither the source tree nor `tauri.conf.json`
//! carries them. A build without both values has no updater: the status is [`UpdateStatus::Disabled`]
//! and every update command answers [`NOT_CONFIGURED`]. The URL is a static `latest.json`-style
//! manifest (the format the Tauri CLI writes); the `{{target}}`, `{{arch}}`, `{{current_version}}`
//! and `{{bundle_type}}` placeholders the plugin understands are allowed in it. The plugin refuses to
//! initialise without a `plugins.updater` block, so [`run`](crate::run) injects
//! [`UpdaterConfig::plugin_config`] into the Tauri context at startup instead of shipping one.
//!
//! **Manual path.** `update_check` asks the manifest and publishes `Checking` → `UpToDate` /
//! `Available` / `Failed`. `update_install` (the 「立即重启更新」 button) downloads the package with
//! `Downloading` progress, verifies its signature, publishes `Ready`, then `Installing`, and hands
//! over to the installer: on Windows the NSIS installer takes over and relaunches the app itself; on
//! macOS / Linux the bundle is replaced in place and the shell asks Tauri to restart the process.
//!
//! **Automatic path** (`Settings.auto_update`, off by default). When it is on and the updater is
//! configured, the shell checks [`AUTO_CHECK_DELAY`] after start; an available package is
//! downloaded in the background and published as `Ready` — nothing is installed behind the user's
//! back: the settings page offers 「立即重启更新」, which is `update_install`. The version that
//! reached `Ready` is remembered in [`MARKER_FILE_NAME`] inside the app data directory. When the user
//! instead just quits and relaunches, the next startup check finds that same version again and, with
//! auto-update still on, downloads and installs it right away and relaunches — that is the "install
//! on next start" the toggle promises. A newer version than the remembered one goes through the
//! `Ready` step again, so a package the user has never seen is never installed unattended. Turning
//! the toggle on later triggers an immediate check (download only). The downloaded package is not
//! persisted across restarts (the plugin keeps it in memory), which is why the second start
//! downloads again.
//!
//! Everything that touches the plugin lives in [`drive`] behind the production wiring
//! (`ShellOptions::global_hotkey`); the mock runtime used by `tests/ipc.rs` never registers the
//! plugin, sees [`UpdateStatus::Disabled`] and gets [`NOT_CONFIGURED`] from the commands.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime, Url};
use tauri_plugin_updater::{Update, UpdaterExt as _};
use voltip_core::ui::{UiEvent, UpdateStatus};
use voltip_tauri_bridge::Bridge;

/// Compile-time environment variable carrying the manifest URL (`https://host/updates/latest.json`).
pub const UPDATE_URL_ENV: &str = "VOLTIP_UPDATE_URL";
/// Compile-time environment variable carrying the minisign public key the packages are signed with.
pub const UPDATE_PUBKEY_ENV: &str = "VOLTIP_UPDATE_PUBKEY";
/// Error every update command returns when the build has no update source.
pub const NOT_CONFIGURED: &str = "updater: 此构建未配置更新源";
/// Error returned while a check / download / install is already running.
pub const BUSY: &str = "updater: 正在检查或下载更新";
/// How long after start the automatic check runs (lets the window and the core settle first).
pub const AUTO_CHECK_DELAY: Duration = Duration::from_secs(10);
/// File in the app data directory remembering the version that reached `Ready` in auto mode.
pub const MARKER_FILE_NAME: &str = "update-ready.json";
/// Minimum number of bytes between two `Downloading` events (keeps the event bus quiet).
pub const PROGRESS_STEP: u64 = 512 * 1024;

/// The update source of this build. A value managed on the Tauri builder before `setup` overrides
/// the build's (`tests/update.rs` points it at a local manifest server).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdaterConfig {
    /// Manifest URL (placeholders allowed).
    pub endpoint: Url,
    /// minisign public key (the `dW50cnVzdGVk…` text the Tauri CLI prints).
    pub pubkey: String,
    /// Delay before the automatic check at startup ([`AUTO_CHECK_DELAY`] in production).
    pub auto_check_delay: Duration,
}

impl UpdaterConfig {
    /// The values baked into this binary, or `None` when either is missing.
    pub fn from_build() -> Option<Self> {
        Self::from_values(option_env!("VOLTIP_UPDATE_URL"), option_env!("VOLTIP_UPDATE_PUBKEY"))
    }

    /// Both values present, trimmed and non-empty, and the URL parses; anything else is "no updater".
    pub fn from_values(url: Option<&str>, pubkey: Option<&str>) -> Option<Self> {
        let url = url.map(str::trim).filter(|s| !s.is_empty())?;
        let pubkey = pubkey.map(str::trim).filter(|s| !s.is_empty())?;
        let endpoint = Url::parse(url).ok().filter(|u| matches!(u.scheme(), "https" | "http"))?;
        Some(Self { endpoint, pubkey: pubkey.to_owned(), auto_check_delay: AUTO_CHECK_DELAY })
    }

    /// The `plugins.updater` block the plugin deserialises at initialisation (injected into the
    /// Tauri context by [`crate::run`], so `tauri.conf.json` never holds the key).
    pub fn plugin_config(&self) -> serde_json::Value {
        serde_json::json!({ "pubkey": self.pubkey, "endpoints": [self.endpoint.as_str()] })
    }
}

/// What a run of the updater should do once it knows an update exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    /// Ask the manifest only (「检查更新」).
    Check,
    /// Check, download, install, relaunch (「立即重启更新」).
    Install,
    /// Auto mode: check and download to `Ready`; install too when the found version is the one
    /// already remembered as `Ready` from an earlier run (the user quit and relaunched).
    Auto {
        /// Version remembered in the marker file at startup; `None` after a toggle or with no marker.
        install_version: Option<String>,
    },
}

impl Intent {
    /// Whether the package should be downloaded once an update is found.
    pub fn downloads(&self) -> bool {
        !matches!(self, Self::Check)
    }

    /// Whether `version` should be installed right after the download.
    pub fn installs(&self, version: &str) -> bool {
        match self {
            Self::Check => false,
            Self::Install => true,
            Self::Auto { install_version } => install_version.as_deref() == Some(version),
        }
    }
}

/// Publishes `Downloading` at most every [`PROGRESS_STEP`] bytes, and always at the end.
#[derive(Debug, Default)]
pub struct ProgressGate {
    last: u64,
}

impl ProgressGate {
    /// `true` when `received` deserves an event.
    pub fn step(&mut self, received: u64, total: Option<u64>) -> bool {
        let due = received.saturating_sub(self.last) >= PROGRESS_STEP || total.is_some_and(|t| received >= t);
        if due {
            self.last = received;
        }
        due
    }
}

/// Unix time in seconds.
pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default()
}

/// The manifest's `pub_date` as published (RFC 3339), when present.
pub fn pub_date(raw: &serde_json::Value) -> Option<String> {
    raw.get("pub_date").and_then(serde_json::Value::as_str).map(str::to_owned)
}

/// Contents of [`MARKER_FILE_NAME`].
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Marker {
    version: String,
}

/// A found update, kept between check, download and install.
struct Pending {
    update: Update,
    /// The verified package once downloaded.
    bytes: Option<Vec<u8>>,
}

/// Managed Tauri state: configuration, the last published status, the pending update and the
/// "one run at a time" flag.
pub struct UpdateSlot {
    config: Option<UpdaterConfig>,
    marker_path: PathBuf,
    status: Mutex<UpdateStatus>,
    pending: Mutex<Option<Pending>>,
    busy: AtomicBool,
}

impl std::fmt::Debug for UpdateSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateSlot").field("config", &self.config).field("status", &self.status.lock()).finish_non_exhaustive()
    }
}

impl UpdateSlot {
    /// `None` config = no updater: the status starts as `Disabled` and stays there.
    pub fn new(config: Option<UpdaterConfig>, data_dir: &Path) -> Self {
        let status = if config.is_some() { UpdateStatus::Idle } else { UpdateStatus::Disabled };
        Self { config, marker_path: data_dir.join(MARKER_FILE_NAME), status: Mutex::new(status), pending: Mutex::new(None), busy: AtomicBool::new(false) }
    }

    /// This build can update itself.
    pub fn enabled(&self) -> bool {
        self.config.is_some()
    }

    /// Delay before the automatic check at startup (zero when there is no updater).
    pub fn auto_check_delay(&self) -> Duration {
        self.config.as_ref().map_or(Duration::ZERO, |c| c.auto_check_delay)
    }

    /// Last published status (what `update_status` answers).
    pub fn status(&self) -> UpdateStatus {
        self.status.lock().clone()
    }

    /// Record `status` and broadcast it like a core event.
    pub fn publish(&self, bridge: &Bridge, status: UpdateStatus) {
        *self.status.lock() = status.clone();
        bridge.publish(UiEvent::Update(status));
    }

    /// Claim the slot for one run; `false` while another run is in flight.
    pub fn try_begin(&self) -> bool {
        self.busy.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_ok()
    }

    /// Release the slot after a run.
    pub fn end(&self) {
        self.busy.store(false, Ordering::Release);
    }

    /// Version remembered as `Ready`, if the marker file exists and parses.
    pub fn read_marker(&self) -> Option<String> {
        let bytes = std::fs::read(&self.marker_path).ok()?;
        serde_json::from_slice::<Marker>(&bytes).ok().map(|m| m.version)
    }

    /// Remember `version` as `Ready` (errors are logged: the marker only decides install-on-restart).
    pub fn write_marker(&self, version: &str) {
        let write = || -> std::io::Result<()> {
            if let Some(dir) = self.marker_path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let bytes = serde_json::to_vec(&Marker { version: version.to_owned() })?;
            std::fs::write(&self.marker_path, bytes)
        };
        if let Err(e) = write() {
            tracing::warn!(error = %e, "update marker not written");
        }
    }

    /// Forget the remembered version (up to date, or installed).
    pub fn clear_marker(&self) {
        if let Err(e) = std::fs::remove_file(&self.marker_path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(error = %e, "update marker not removed");
        }
    }

    fn take_pending(&self) -> Option<Pending> {
        self.pending.lock().take()
    }

    fn set_pending(&self, pending: Pending) {
        *self.pending.lock() = Some(pending);
    }
}

/// Entry point of the commands and of the automatic check: refuse when unconfigured or busy,
/// otherwise run `intent` in the background and report through the bridge.
pub fn request<R: Runtime>(app: &AppHandle<R>, bridge: &Bridge, slot: &Arc<UpdateSlot>, intent: Intent) -> Result<(), String> {
    if !slot.enabled() {
        return Err(NOT_CONFIGURED.to_owned());
    }
    if !slot.try_begin() {
        return Err(BUSY.to_owned());
    }
    tauri::async_runtime::spawn(drive(app.clone(), bridge.clone(), slot.clone(), intent));
    Ok(())
}

/// One run against the plugin; failures become `Failed`, the slot is released at the end.
async fn drive<R: Runtime>(app: AppHandle<R>, bridge: Bridge, slot: Arc<UpdateSlot>, intent: Intent) {
    if let Err(message) = run(&app, &bridge, &slot, &intent).await {
        tracing::warn!(error = %message, ?intent, "updater run failed");
        slot.publish(&bridge, UpdateStatus::Failed { message });
    }
    slot.end();
}

async fn run<R: Runtime>(app: &AppHandle<R>, bridge: &Bridge, slot: &Arc<UpdateSlot>, intent: &Intent) -> Result<(), String> {
    let Some(config) = slot.config.clone() else { return Err(NOT_CONFIGURED.to_owned()) };
    // An explicit check always asks the manifest (and forgets what the last one found); the other
    // intents reuse the pending update.
    let reuse = slot.take_pending().filter(|_| *intent != Intent::Check);
    let mut pending = match reuse {
        Some(pending) => pending,
        None => {
            slot.publish(bridge, UpdateStatus::Checking);
            let updater =
                app.updater_builder().endpoints(vec![config.endpoint]).map_err(|e| e.to_string())?.pubkey(config.pubkey).build().map_err(|e| e.to_string())?;
            match updater.check().await.map_err(|e| e.to_string())? {
                None => {
                    slot.clear_marker();
                    let version = app.package_info().version.to_string();
                    tracing::info!(version, "no update available");
                    slot.publish(bridge, UpdateStatus::UpToDate { version, checked_at: now_secs() });
                    return Ok(());
                }
                Some(update) => {
                    tracing::info!(version = %update.version, current = %update.current_version, "update available");
                    slot.publish(
                        bridge,
                        UpdateStatus::Available {
                            version: update.version.clone(),
                            current: update.current_version.clone(),
                            notes: update.body.clone(),
                            date: pub_date(&update.raw_json),
                        },
                    );
                    Pending { update, bytes: None }
                }
            }
        }
    };
    let version = pending.update.version.clone();
    if !intent.downloads() {
        slot.set_pending(pending);
        return Ok(());
    }
    if pending.bytes.is_none() {
        slot.publish(bridge, UpdateStatus::Downloading { version: version.clone(), received: 0, total: None });
        let mut gate = ProgressGate::default();
        let mut received = 0u64;
        let bytes = pending
            .update
            .download(
                |chunk, total| {
                    received += u64::try_from(chunk).unwrap_or(u64::MAX);
                    if gate.step(received, total) {
                        slot.publish(bridge, UpdateStatus::Downloading { version: version.clone(), received, total });
                    }
                },
                || {},
            )
            .await
            .map_err(|e| e.to_string())?;
        tracing::info!(version, bytes = bytes.len(), "update downloaded and verified");
        pending.bytes = Some(bytes);
        slot.write_marker(&version);
        slot.publish(bridge, UpdateStatus::Ready { version: version.clone() });
    }
    if !intent.installs(&version) {
        slot.set_pending(pending);
        return Ok(());
    }
    slot.publish(bridge, UpdateStatus::Installing { version: version.clone() });
    let Pending { update, bytes } = pending;
    let bytes = bytes.unwrap_or_default();
    // Windows: the installer is launched and the process exits inside `install`; macOS / Linux: the
    // bundle is replaced and the app has to relaunch itself.
    tauri::async_runtime::spawn_blocking(move || update.install(bytes)).await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
    slot.clear_marker();
    tracing::info!(version, "update installed; restarting");
    app.request_restart();
    Ok(())
}

/// Follow `Settings.auto_update`: on at startup → check after [`AUTO_CHECK_DELAY`] (installing only
/// the version remembered as `Ready`); turned on later → check now, download only.
pub fn follow_settings<R: Runtime>(app: AppHandle<R>, bridge: Bridge, slot: Arc<UpdateSlot>) {
    let mut events = bridge.events();
    let initial = bridge.state();
    // `Ready` may already have been folded before this subscription: read it from the cache.
    let mut current: Option<bool> = initial.identity.is_some().then_some(initial.settings.auto_update);
    if current == Some(true) {
        schedule_auto(&app, &bridge, &slot, true);
    }
    tauri::async_runtime::spawn(async move {
        loop {
            let next = match events.recv().await {
                Ok(UiEvent::Settings(s)) => Some(s.auto_update),
                Ok(UiEvent::State(state)) => Some(state.settings.auto_update),
                Ok(_) => None,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => None,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            let Some(next) = next else { continue };
            let startup = current.is_none();
            if current == Some(next) {
                continue;
            }
            current = Some(next);
            if next {
                schedule_auto(&app, &bridge, &slot, startup);
            }
        }
    });
}

/// Spawn the automatic run: delayed and marker-aware at startup, immediate after a toggle.
fn schedule_auto<R: Runtime>(app: &AppHandle<R>, bridge: &Bridge, slot: &Arc<UpdateSlot>, startup: bool) {
    let (app, bridge, slot) = (app.clone(), bridge.clone(), slot.clone());
    tauri::async_runtime::spawn(async move {
        let install_version = if startup {
            tokio::time::sleep(slot.auto_check_delay()).await;
            slot.read_marker()
        } else {
            None
        };
        tracing::info!(startup, ?install_version, "automatic update check");
        if let Err(e) = request(&app, &bridge, &slot, Intent::Auto { install_version }) {
            tracing::info!(error = %e, "automatic update check skipped");
        }
    });
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn config_needs_both_values_and_a_parsable_url() {
        assert_eq!(UpdaterConfig::from_values(None, None), None);
        assert_eq!(UpdaterConfig::from_values(Some("https://updates.example.test/latest.json"), None), None);
        assert_eq!(UpdaterConfig::from_values(None, Some("key")), None);
        assert_eq!(UpdaterConfig::from_values(Some("  "), Some("key")), None, "blank URL is unset");
        assert_eq!(UpdaterConfig::from_values(Some("https://x.example"), Some("\n")), None, "blank key is unset");
        assert_eq!(UpdaterConfig::from_values(Some("not a url"), Some("key")), None);
        assert_eq!(UpdaterConfig::from_values(Some("ftp://x.example/latest.json"), Some("key")), None, "http(s) only");
        let cfg = UpdaterConfig::from_values(Some(" https://updates.example.test/{{target}}/{{arch}}/{{current_version}} "), Some(" dW50cnVzdGVk ")).unwrap();
        assert_eq!(cfg.pubkey, "dW50cnVzdGVk");
        assert_eq!(cfg.auto_check_delay, AUTO_CHECK_DELAY);
        // url::Url percent-encodes the braces in the path; the plugin replaces both spellings.
        assert!(cfg.endpoint.as_str().contains("%7B%7Btarget%7D%7D"), "{}", cfg.endpoint);
        let json = cfg.plugin_config();
        assert_eq!(json["pubkey"], "dW50cnVzdGVk");
        assert_eq!(json["endpoints"].as_array().unwrap().len(), 1);
        assert!(format!("{cfg:?}").contains("updates.example.test"));
        // The build-time values are whatever the environment held when this test binary compiled;
        // only the shape of the answer is fixed.
        let _ = UpdaterConfig::from_build();
    }

    #[test]
    fn intents_download_and_install_as_documented() {
        assert!(!Intent::Check.downloads());
        assert!(!Intent::Check.installs("2.1.0"));
        assert!(Intent::Install.downloads());
        assert!(Intent::Install.installs("2.1.0"));
        let fresh = Intent::Auto { install_version: None };
        assert!(fresh.downloads());
        assert!(!fresh.installs("2.1.0"), "a version never seen as Ready is only downloaded");
        let remembered = Intent::Auto { install_version: Some("2.1.0".into()) };
        assert!(remembered.installs("2.1.0"), "the remembered Ready version installs on the next start");
        assert!(!remembered.installs("2.2.0"), "a newer version than the remembered one goes through Ready again");
    }

    #[test]
    fn progress_gate_throttles_and_always_reports_the_end() {
        let mut gate = ProgressGate::default();
        assert!(!gate.step(1, Some(10 * PROGRESS_STEP)));
        assert!(!gate.step(PROGRESS_STEP - 1, Some(10 * PROGRESS_STEP)));
        assert!(gate.step(PROGRESS_STEP, Some(10 * PROGRESS_STEP)));
        assert!(!gate.step(PROGRESS_STEP + 1, Some(10 * PROGRESS_STEP)));
        assert!(gate.step(2 * PROGRESS_STEP, None), "no total: step-based only");
        assert!(gate.step(2 * PROGRESS_STEP + 5, Some(2 * PROGRESS_STEP + 5)), "reaching the total always reports");
        assert!(now_secs() > 1_700_000_000);
        assert_eq!(pub_date(&serde_json::json!({ "pub_date": "2026-09-25T08:00:00Z" })), Some("2026-09-25T08:00:00Z".into()));
        assert_eq!(pub_date(&serde_json::json!({ "version": "2.1.0" })), None);
        assert_eq!(pub_date(&serde_json::json!({ "pub_date": 7 })), None);
    }

    #[test]
    fn slot_without_config_is_disabled_and_the_marker_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let slot = UpdateSlot::new(None, dir.path());
        assert!(!slot.enabled());
        assert_eq!(slot.auto_check_delay(), Duration::ZERO);
        assert_eq!(slot.status(), UpdateStatus::Disabled);
        assert!(format!("{slot:?}").contains("Disabled"));
        assert!(slot.try_begin());
        assert!(!slot.try_begin(), "one run at a time");
        slot.end();
        assert!(slot.try_begin());
        slot.end();
        // Marker: absent → None; written → read back; cleared → None; a corrupt file reads as None.
        assert_eq!(slot.read_marker(), None);
        slot.clear_marker();
        slot.write_marker("2.1.0");
        assert_eq!(slot.read_marker(), Some("2.1.0".into()));
        assert_eq!(std::fs::read_to_string(dir.path().join(MARKER_FILE_NAME)).unwrap(), r#"{"version":"2.1.0"}"#);
        slot.clear_marker();
        assert_eq!(slot.read_marker(), None);
        std::fs::write(dir.path().join(MARKER_FILE_NAME), b"{").unwrap();
        assert_eq!(slot.read_marker(), None);
        // A missing parent directory is created on write.
        let nested = UpdateSlot::new(UpdaterConfig::from_values(Some("https://x.example/latest.json"), Some("k")), &dir.path().join("a/b"));
        assert!(nested.enabled());
        assert_eq!(nested.auto_check_delay(), AUTO_CHECK_DELAY);
        assert_eq!(nested.status(), UpdateStatus::Idle);
        nested.write_marker("3.0.0");
        assert_eq!(nested.read_marker(), Some("3.0.0".into()));
        assert!(nested.take_pending().is_none());
    }
}
