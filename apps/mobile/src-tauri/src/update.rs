//! Updates on the phone (docs/dictation.md §20.9). How the app was installed decides where they
//! come from. Google Play updates the app itself: the phone only points to its listing
//! ([`UpdateStatus::Store`]). An APK from a GitHub release asks GitHub for the latest release and,
//! when that one is newer, opens its APK in the browser; Android installs it over this one only
//! when it is signed with the same key, and asks the person first. The app installs nothing itself:
//! an app from Google Play may not update itself outside Play, and the permission it would need
//! (`REQUEST_INSTALL_PACKAGES`) is not allowed there for that.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde::Deserialize;
use tauri::{AppHandle, Runtime};
use voltip_core::ui::{UiEvent, UpdateStatus};
use voltip_tauri_bridge::Bridge;

/// Google Play's package: an app it installed is updated by it.
pub const PLAY_STORE: &str = "com.android.vending";
/// How long after start the automatic check runs (as on the desktop).
pub const AUTO_CHECK_DELAY: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// A check is already running.
pub const BUSY: &str = "updater: 正在检查更新";
/// The first moments after start, before the system said who installed the app.
pub const SOURCE_PENDING: &str = "updater: 正在读取安装来源，请稍后再试";
/// `update_install` before a check found a newer release.
pub const NOTHING_TO_INSTALL: &str = "updater: 没有可下载的新版本，请先检查更新";

/// Who installed the app, as far as updates go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallSource {
    /// Google Play, which updates the app itself.
    Store,
    /// Anything else: the APK of a GitHub release (through a browser or a file manager), `adb`.
    Direct,
}

impl InstallSource {
    /// The source for the installer package the system recorded (`None`: none recorded).
    pub fn from_installer(installer: Option<&str>) -> Self {
        if installer == Some(PLAY_STORE) { Self::Store } else { Self::Direct }
    }
}

/// Where the phone looks for updates.
#[derive(Clone, Debug)]
pub struct UpdateConfig {
    /// Who installed the app; `None`: ask the system once the app runs ([`start`]).
    pub source: Option<InstallSource>,
    /// GitHub's latest-release endpoint for the repository ([`latest_release_api`]).
    pub latest_release: String,
    /// The app's Google Play listing ([`store_listing`]).
    pub listing: String,
    /// Delay of the automatic check after start ([`AUTO_CHECK_DELAY`] in production).
    pub auto_check_delay: Duration,
}

impl UpdateConfig {
    /// The repository's releases, and the installer the system reports, which [`start`] asks for.
    pub fn production<R: Runtime>(app: &AppHandle<R>) -> Self {
        Self {
            source: None,
            latest_release: latest_release_api(crate::REPOSITORY),
            listing: store_listing(&app.config().identifier),
            auto_check_delay: AUTO_CHECK_DELAY,
        }
    }
}

/// `https://github.com/<owner>/<repo>` → GitHub's API for that repository's latest release.
pub fn latest_release_api(repository: &str) -> String {
    let path = repository.trim().trim_end_matches('/').trim_start_matches("https://github.com/");
    format!("https://api.github.com/repos/{path}/releases/latest")
}

/// The Google Play page of `package`; Android hands it to the Play app when it is there.
pub fn store_listing(package: &str) -> String {
    format!("https://play.google.com/store/apps/details?id={package}")
}

/// The APK a release publishes for the phone (`.github/release-targets.json`, the android target).
pub fn apk_name(version: &str) -> String {
    format!("Voltip_{version}_android_arm64.apk")
}

/// The Android plugin that names the installer (`UpdatePlugin.kt`).
#[cfg(target_os = "android")]
pub struct InstallSourcePlugin<R: Runtime>(tauri::plugin::PluginHandle<R>);

/// Registers the Android plugin; a no-op elsewhere.
pub fn init<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("voltip-update")
        .setup(|_app, _api| {
            #[cfg(target_os = "android")]
            {
                use tauri::Manager as _;
                let handle = _api.register_android_plugin("dev.voltip.mobile", "UpdatePlugin")?;
                _app.manage(InstallSourcePlugin(handle));
            }
            Ok(())
        })
        .build()
}

/// The installer package the system recorded; `None` when it recorded none, and off Android.
fn installer<R: Runtime>(app: &AppHandle<R>) -> Option<String> {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager as _;
        let plugin = app.try_state::<InstallSourcePlugin<R>>()?;
        match plugin.0.run_mobile_plugin::<serde_json::Value>("installSource", ()) {
            Ok(answer) => answer.get("installer").and_then(serde_json::Value::as_str).map(str::to_owned),
            Err(e) => {
                tracing::warn!(error = %e, "install source unknown; looking for releases on GitHub");
                None
            }
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        None
    }
}

/// The fields of GitHub's latest-release answer that the phone reads.
#[derive(Debug, Deserialize)]
pub struct Release {
    /// `v0.0.28`.
    pub tag_name: String,
    /// The release notes (Markdown).
    #[serde(default)]
    pub body: Option<String>,
    /// RFC 3339.
    #[serde(default)]
    pub published_at: Option<String>,
    /// The files of the release.
    #[serde(default)]
    pub assets: Vec<Asset>,
}

/// One file of a release.
#[derive(Debug, Deserialize)]
pub struct Asset {
    /// File name.
    pub name: String,
    /// Where a browser downloads it.
    pub browser_download_url: String,
}

/// What `release` means for the running `current` version: a newer one is `Available`, with the
/// address of its APK; the same or an older one is `UpToDate` (checked at `now`, Unix seconds).
pub fn read_release(release: &Release, current: &str, now: u64) -> Result<(UpdateStatus, Option<String>), String> {
    let version = release.tag_name.trim().trim_start_matches('v').to_owned();
    let (Some(latest), Some(running)) = (version_of(&version), version_of(current)) else {
        return Err(format!("无法识别版本号 {} / {current}", release.tag_name));
    };
    if latest <= running {
        return Ok((UpdateStatus::UpToDate { version: current.to_owned(), checked_at: now }, None));
    }
    let name = apk_name(&version);
    let Some(apk) = release.assets.iter().find(|a| a.name == name) else {
        return Err(format!("{version} 没有 Android 安装包（{name}）"));
    };
    let notes = release.body.as_deref().map(str::trim).filter(|b| !b.is_empty()).map(str::to_owned);
    let available = UpdateStatus::Available { version, current: current.to_owned(), notes, date: release.published_at.clone() };
    Ok((available, Some(apk.browser_download_url.clone())))
}

/// `0.0.28` → `(0, 0, 28)`; anything else (a pre-release suffix, a missing part) is no version.
fn version_of(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.trim().split('.').map(|p| p.parse::<u64>().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

/// Ask GitHub for the latest release.
async fn fetch(url: &str) -> Result<Release, String> {
    // Not a bare reqwest client: its verifier panics on Android at the first HTTPS request.
    let client = voltip_cloud::http_client_builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        // GitHub's API refuses requests without one.
        .user_agent(concat!("voltip-android/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;
    let response = client.get(url).header("Accept", "application/vnd.github+json").send().await.map_err(|e| format!("无法连接 GitHub：{e}"))?;
    let status = response.status();
    if status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err("GitHub 暂时不接受查询（每小时的查询次数有限），请稍后再试".into());
    }
    if !status.is_success() {
        return Err(format!("GitHub 返回 {status}"));
    }
    response.json::<Release>().await.map_err(|e| format!("GitHub 的回答无法读取：{e}"))
}

/// The phone's updater: the status `update_status` answers and the release a check found.
pub struct PhoneUpdater {
    config: UpdateConfig,
    source: Mutex<Option<InstallSource>>,
    current: String,
    status: Mutex<UpdateStatus>,
    /// The APK of the newer release the last check found.
    apk: Mutex<Option<String>>,
    busy: AtomicBool,
}

impl std::fmt::Debug for PhoneUpdater {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhoneUpdater").field("source", &self.source()).field("status", &self.status()).finish_non_exhaustive()
    }
}

impl PhoneUpdater {
    /// The updater of a build that runs `current`: `store` from the start when Play installed it,
    /// `idle` otherwise and while the source is not known yet.
    pub fn new(config: UpdateConfig, current: &str) -> Self {
        let status = match config.source {
            Some(InstallSource::Store) => UpdateStatus::Store { version: current.to_owned() },
            Some(InstallSource::Direct) | None => UpdateStatus::Idle,
        };
        Self {
            source: Mutex::new(config.source),
            config,
            current: current.to_owned(),
            status: Mutex::new(status),
            apk: Mutex::new(None),
            busy: AtomicBool::new(false),
        }
    }

    /// Record who installed the app; the status to send when that changes it (`store`).
    fn note_source(&self, source: InstallSource) -> Option<UpdateStatus> {
        *self.source.lock().unwrap_or_else(PoisonError::into_inner) = Some(source);
        (source == InstallSource::Store).then(|| {
            let status = UpdateStatus::Store { version: self.current.clone() };
            *self.status.lock().unwrap_or_else(PoisonError::into_inner) = status.clone();
            status
        })
    }

    /// What `update_status` answers.
    pub fn status(&self) -> UpdateStatus {
        self.status.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Who installed the app; `None` until the system said.
    pub fn source(&self) -> Option<InstallSource> {
        *self.source.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Record `status` and send it to the webview like a core event.
    fn publish(&self, bridge: &Bridge, status: UpdateStatus) {
        *self.status.lock().unwrap_or_else(PoisonError::into_inner) = status.clone();
        bridge.publish(UiEvent::Update(status));
    }

    /// `update_check`: Play checks by itself, so nothing happens there; otherwise GitHub is asked in
    /// the background and the outcome arrives as an `update` event.
    pub fn check(self: &Arc<Self>, bridge: &Bridge) -> Result<(), String> {
        match self.source() {
            None => return Err(SOURCE_PENDING.into()),
            Some(InstallSource::Store) => return Ok(()),
            Some(InstallSource::Direct) => {}
        }
        if self.busy.swap(true, Ordering::AcqRel) {
            return Err(BUSY.into());
        }
        self.publish(bridge, UpdateStatus::Checking);
        let (updater, bridge) = (self.clone(), bridge.clone());
        tauri::async_runtime::spawn(async move {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
            let outcome = fetch(&updater.config.latest_release).await.and_then(|release| read_release(&release, &updater.current, now));
            let status = match outcome {
                Ok((status, apk)) => {
                    *updater.apk.lock().unwrap_or_else(PoisonError::into_inner) = apk;
                    status
                }
                Err(message) => {
                    tracing::warn!(error = %message, "update check failed");
                    UpdateStatus::Failed { message }
                }
            };
            tracing::info!(?status, "update check");
            // Free before the outcome goes out: whoever reacts to it can check again.
            updater.busy.store(false, Ordering::Release);
            updater.publish(&bridge, status);
        });
        Ok(())
    }

    /// `update_install`: the page that installs the update, opened in the browser by the command:
    /// the Play listing, or the APK of the newer release the last check found.
    pub fn install_url(&self) -> Result<String, String> {
        match self.source() {
            None => Err(SOURCE_PENDING.into()),
            Some(InstallSource::Store) => Ok(self.config.listing.clone()),
            Some(InstallSource::Direct) => self.apk.lock().unwrap_or_else(PoisonError::into_inner).clone().ok_or_else(|| NOTHING_TO_INSTALL.to_owned()),
        }
    }
}

/// Start the updater. When the config does not say who installed the app, ask the system first:
/// a plugin call, so off the setup thread, which holds the main thread the call waits for. Then
/// follow `Settings.auto_update`.
pub fn start<R: Runtime>(app: &AppHandle<R>, bridge: Bridge, updater: Arc<PhoneUpdater>) {
    if updater.source().is_some() {
        follow_settings(bridge, updater);
        return;
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let installer = tauri::async_runtime::spawn_blocking(move || installer(&handle)).await.ok().flatten();
        tracing::info!(installer = installer.as_deref().unwrap_or("none"), "install source");
        if let Some(status) = updater.note_source(InstallSource::from_installer(installer.as_deref())) {
            bridge.publish(UiEvent::Update(status));
        }
        follow_settings(bridge, updater);
    });
}

/// Follow `Settings.auto_update` (off by default, as on the desktop): on at startup → check after
/// the configured delay; turned on later → check now. Play installs never check: Play does.
fn follow_settings(bridge: Bridge, updater: Arc<PhoneUpdater>) {
    if updater.source() == Some(InstallSource::Store) {
        return;
    }
    let mut events = bridge.events();
    let initial = bridge.state();
    // `Ready` may already have been folded before this subscription: read it from the cache.
    let mut current: Option<bool> = initial.identity.is_some().then_some(initial.settings.auto_update);
    if current == Some(true) {
        schedule(&bridge, &updater, true);
    }
    tauri::async_runtime::spawn(async move {
        loop {
            let next = match events.recv().await {
                Ok(UiEvent::Settings(s)) => Some(s.auto_update),
                Ok(UiEvent::State(state)) => Some(state.settings.auto_update),
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => None,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            let Some(next) = next else { continue };
            let startup = current.is_none();
            if current == Some(next) {
                continue;
            }
            current = Some(next);
            if next {
                schedule(&bridge, &updater, startup);
            }
        }
    });
}

/// The automatic check: after the delay at startup, at once after a toggle.
fn schedule(bridge: &Bridge, updater: &Arc<PhoneUpdater>, startup: bool) {
    let (bridge, updater) = (bridge.clone(), updater.clone());
    tauri::async_runtime::spawn(async move {
        if startup {
            // A deliberate pause, not a wait for anything: the start of the app comes first.
            tokio::time::sleep(updater.config.auto_check_delay).await;
        }
        if let Err(e) = updater.check(&bridge) {
            tracing::info!(error = %e, "automatic update check skipped");
        }
    });
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn release(tag: &str, assets: &[&str]) -> Release {
        Release {
            tag_name: tag.into(),
            body: Some("## 0.0.29\n\n* fix".into()),
            published_at: Some("2026-10-03T08:00:00Z".into()),
            assets: assets.iter().map(|name| Asset { name: (*name).into(), browser_download_url: format!("https://example.test/download/{name}") }).collect(),
        }
    }

    #[test]
    fn a_newer_release_is_available_with_its_apk_and_the_same_or_an_older_one_is_up_to_date() {
        let newer = release("v0.0.29", &["Voltip_0.0.29_android_arm64.aab", "Voltip_0.0.29_android_arm64.apk", "SHA256SUMS"]);
        let (status, apk) = read_release(&newer, "0.0.28", 7).unwrap();
        assert_eq!(
            status,
            UpdateStatus::Available {
                version: "0.0.29".into(),
                current: "0.0.28".into(),
                notes: Some("## 0.0.29\n\n* fix".into()),
                date: Some("2026-10-03T08:00:00Z".into())
            }
        );
        assert_eq!(apk.as_deref(), Some("https://example.test/download/Voltip_0.0.29_android_arm64.apk"));
        for (tag, current) in [("v0.0.28", "0.0.28"), ("v0.0.27", "0.0.28"), ("v0.0.9", "0.0.10"), ("0.1.0", "0.1.0")] {
            let (status, apk) = read_release(&release(tag, &[]), current, 7).unwrap();
            assert_eq!(status, UpdateStatus::UpToDate { version: current.into(), checked_at: 7 }, "{tag} vs {current}");
            assert!(apk.is_none());
        }
        // Compared as numbers, not as text: 0.0.10 is newer than 0.0.9.
        assert!(matches!(read_release(&release("v0.0.10", &["Voltip_0.0.10_android_arm64.apk"]), "0.0.9", 7).unwrap().0, UpdateStatus::Available { .. }));
        let mut blank = release("v1.0.0", &["Voltip_1.0.0_android_arm64.apk"]);
        blank.body = Some("  \n".into());
        assert!(matches!(read_release(&blank, "0.0.28", 7).unwrap().0, UpdateStatus::Available { notes: None, .. }));
    }

    #[test]
    fn a_newer_release_without_an_apk_or_an_unreadable_version_is_an_error() {
        assert_eq!(
            read_release(&release("v0.0.29", &["Voltip_0.0.29_android_arm64.aab"]), "0.0.28", 7).unwrap_err(),
            "0.0.29 没有 Android 安装包（Voltip_0.0.29_android_arm64.apk）"
        );
        for (tag, current) in [("v0.0.29-rc.1", "0.0.28"), ("nightly", "0.0.28"), ("v0.0.29", "dev"), ("v0.0", "0.0.28"), ("v0.0.29.1", "0.0.28")] {
            assert!(read_release(&release(tag, &[]), current, 7).unwrap_err().starts_with("无法识别版本号"), "{tag} vs {current}");
        }
    }

    /// Regression (2026-10-03, the goal gate): the update check built a bare reqwest client, whose
    /// platform verifier panics on Android at the first HTTPS request; a test host never reaches
    /// that branch. The check takes voltip-cloud's builder, which trusts Mozilla's roots there.
    #[test]
    fn regression_the_update_check_takes_the_client_that_works_on_android() {
        let code: String = include_str!("update.rs").lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
        assert!(code.contains("voltip_cloud::http_client_builder()"));
        assert!(!code.contains(concat!("reqwest::Client", "::builder()")), "a bare client panics on Android");
        assert!(voltip_cloud::http_client_builder().build().is_ok());
    }

    /// Regression (user report 2026-10-03, 「更新失败 · GitHub 返回 404 Not Found」): the phone
    /// crate did not take the workspace's `repository`, so `REPOSITORY` was empty. The update check
    /// asked `https://api.github.com/repos//releases/latest`, and 关于's links opened nothing.
    #[test]
    fn regression_the_phone_asks_its_own_repository() {
        assert_eq!(crate::REPOSITORY, "https://github.com/sunerpy/voltip");
        assert_eq!(latest_release_api(crate::REPOSITORY), "https://api.github.com/repos/sunerpy/voltip/releases/latest");
        assert_eq!(voltip_core::ui::ProjectLink::Releases.url(crate::REPOSITORY), "https://github.com/sunerpy/voltip/releases");
    }

    #[test]
    fn the_store_listing_and_the_github_api_come_from_the_package_and_the_repository() {
        assert_eq!(latest_release_api("https://github.com/sunerpy/voltip"), "https://api.github.com/repos/sunerpy/voltip/releases/latest");
        assert_eq!(latest_release_api(" https://github.com/sunerpy/voltip/ "), "https://api.github.com/repos/sunerpy/voltip/releases/latest");
        assert_eq!(store_listing("dev.voltip.mobile"), "https://play.google.com/store/apps/details?id=dev.voltip.mobile");
        assert_eq!(apk_name("0.0.28"), "Voltip_0.0.28_android_arm64.apk");
        assert_eq!(InstallSource::from_installer(Some(PLAY_STORE)), InstallSource::Store);
        for other in [None, Some("com.android.chrome"), Some("com.google.android.packageinstaller"), Some("")] {
            assert_eq!(InstallSource::from_installer(other), InstallSource::Direct, "{other:?}");
        }
    }

    #[test]
    fn a_play_install_points_to_its_listing_and_a_direct_one_needs_a_newer_release_first() {
        let config = |source| UpdateConfig {
            source: Some(source),
            latest_release: "https://example.test/latest".into(),
            listing: store_listing("dev.voltip.mobile"),
            auto_check_delay: Duration::ZERO,
        };
        let store = PhoneUpdater::new(config(InstallSource::Store), "0.0.28");
        assert_eq!(store.status(), UpdateStatus::Store { version: "0.0.28".into() });
        assert_eq!(store.install_url().unwrap(), "https://play.google.com/store/apps/details?id=dev.voltip.mobile");
        let direct = PhoneUpdater::new(config(InstallSource::Direct), "0.0.28");
        assert_eq!(direct.status(), UpdateStatus::Idle);
        assert_eq!(direct.install_url().unwrap_err(), NOTHING_TO_INSTALL);
        *direct.apk.lock().unwrap() = Some("https://example.test/a.apk".into());
        assert_eq!(direct.install_url().unwrap(), "https://example.test/a.apk");
        assert!(format!("{direct:?}").contains("Direct"));
    }

    #[test]
    fn until_the_system_names_the_installer_nothing_is_checked_or_opened() {
        let config = UpdateConfig {
            source: None,
            latest_release: "https://example.test/latest".into(),
            listing: store_listing("dev.voltip.mobile"),
            auto_check_delay: Duration::ZERO,
        };
        let pending = PhoneUpdater::new(config.clone(), "0.0.28");
        assert_eq!((pending.source(), pending.status()), (None, UpdateStatus::Idle));
        assert_eq!(pending.install_url().unwrap_err(), SOURCE_PENDING);
        // Google Play: `store` from then on, to be sent; anything else leaves `idle` as it was.
        assert_eq!(pending.note_source(InstallSource::Store), Some(UpdateStatus::Store { version: "0.0.28".into() }));
        assert_eq!((pending.source(), pending.status()), (Some(InstallSource::Store), UpdateStatus::Store { version: "0.0.28".into() }));
        assert_eq!(pending.install_url().unwrap(), "https://play.google.com/store/apps/details?id=dev.voltip.mobile");
        let direct = PhoneUpdater::new(config, "0.0.28");
        assert_eq!(direct.note_source(InstallSource::Direct), None);
        assert_eq!((direct.source(), direct.status()), (Some(InstallSource::Direct), UpdateStatus::Idle));
        assert_eq!(direct.install_url().unwrap_err(), NOTHING_TO_INSTALL);
    }
}
