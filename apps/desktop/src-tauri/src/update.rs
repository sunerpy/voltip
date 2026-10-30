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
//! and `{{bundle_type}}` placeholders the plugin understands are allowed in it. It is https,
//! except in a test build whose manifest server is on this computer or the local network
//! ([`UpdaterConfig::local_http`]). An optional mirror
//! ([`UPDATE_MIRROR_ENV`], the base of the docs site's `/updates/`) is asked first and the URL
//! second: the plugin moves on to the next address when one fails or does not answer 2xx, and the
//! packages come from wherever the manifest that answered points. The plugin refuses to
//! initialise without a `plugins.updater` block, so [`run`](crate::run) injects
//! [`UpdaterConfig::plugin_config`] into the Tauri context at startup instead of shipping one.
//!
//! **Manual path.** `update_check` asks the manifest and publishes `Checking` → `UpToDate` /
//! `Available` / `Failed`. `update_install` (the 「立即重启更新」 button) downloads the package with
//! `Downloading` progress, verifies its signature, publishes `Ready`, then `Installing`, and hands
//! over to the installer: on Windows the NSIS installer takes over, runs without a window
//! ([`windows_install_mode`]) and relaunches the app itself; on macOS / Linux the bundle is replaced
//! in place and the shell asks Tauri to restart the process. Right before the installer, on the
//! same blocking thread, the core writes the automatic backup before an update
//! (docs/dictation.md §4.6.1, [`backup_then_install`]): the Windows installer ends this process
//! from inside `install`, so nothing after it runs.
//!
//! **Automatic path** (`Settings.auto_update`, off by default). When it is on and the updater is
//! configured, the shell checks [`AUTO_CHECK_DELAY`] after start; an available package is
//! downloaded in the background and published as `Ready` — nothing is installed behind the user's
//! back: the title bar's 「重启以更新」 and the settings page offer the restart, which is
//! `update_install`. The version that reached `Ready` is remembered in [`MARKER_FILE_NAME`] inside
//! the app data directory. When the user instead just quits and relaunches, the next startup check
//! finds that same version again and, with auto-update still on, downloads and installs it right
//! away and relaunches — that is the "install on next start" the toggle promises. A newer version
//! than the remembered one goes through the `Ready` step again, so a package the user has never
//! seen is never installed unattended. Turning the toggle on later triggers an immediate check
//! (download only). The downloaded package is not persisted across restarts (the plugin keeps it in
//! memory), which is why the second start downloads again.
//!
//! **Looking again while Voltip runs** (user request 2026-10-25, after Lockra 0.16.3). An app that
//! stays in the tray for days would otherwise hear of a release only at its next start. With the
//! toggle on, every run that ends (a check by hand included) schedules the next look
//! [`AUTO_CHECK_INTERVAL`] later ([`AutoSchedule`]), keeping a look that is due sooner (the one at
//! start). These later looks only download: only the look at start may install the remembered
//! version, so a look hours later never restarts the app under the user. A package waiting in
//! `Ready` stops the looks until the restart; a failed look is tried again an interval later,
//! never in a loop. Turning the toggle off drops the look that is due. The clock is monotonic, so
//! time the computer spends asleep does not count.
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
#[cfg(target_os = "macos")]
use tauri::Manager as _;
use tauri::utils::config::BundleType;
use tauri::{AppHandle, Runtime, Url};
use tauri_plugin_updater::{Update, UpdaterExt as _};
use tokio::sync::Notify;
use tokio::sync::broadcast::error::RecvError;
use tokio::time::Instant;
use voltip_core::backup::auto::{AutoKind, AutoOutcome};
use voltip_core::ui::{UiEvent, UpdateStatus};
use voltip_identity::Entries;
use voltip_tauri_bridge::Bridge;

/// Compile-time environment variable carrying the manifest URL (`https://host/updates/latest.json`).
pub const UPDATE_URL_ENV: &str = "VOLTIP_UPDATE_URL";
/// Compile-time environment variable carrying the base of a mirror of the release's update files
/// (`https://host/updates`, docs/dictation.md §9): its `latest.json` is asked first, the manifest of
/// [`UPDATE_URL_ENV`] when the mirror cannot answer. Optional.
pub const UPDATE_MIRROR_ENV: &str = "VOLTIP_UPDATE_MIRROR";
/// Compile-time environment variable carrying the minisign public key the packages are signed with.
pub const UPDATE_PUBKEY_ENV: &str = "VOLTIP_UPDATE_PUBKEY";
/// Error every update command returns when the build has no update source.
pub const NOT_CONFIGURED: &str = "updater: 此构建未配置更新源";
/// Error returned while a check / download / install is already running.
pub const BUSY: &str = "updater: 正在检查或下载更新";
/// How long after start the automatic check runs (lets the window and the core settle first).
pub const AUTO_CHECK_DELAY: Duration = Duration::from_secs(10);
/// How long after a run ends the automatic update looks again while Voltip runs.
pub const AUTO_CHECK_INTERVAL: Duration = Duration::from_secs(4 * 60 * 60);
/// Compile-time environment variable that makes a test build look again sooner than
/// [`AUTO_CHECK_INTERVAL`], in seconds (at least [`MIN_AUTO_CHECK_INTERVAL`]). The release builds
/// leave it unset.
pub const UPDATE_INTERVAL_ENV: &str = "VOLTIP_UPDATE_INTERVAL_SECS";
/// The shortest interval [`UPDATE_INTERVAL_ENV`] can set.
pub const MIN_AUTO_CHECK_INTERVAL: Duration = Duration::from_secs(60);
/// File in the app data directory remembering the version that reached `Ready` in auto mode.
pub const MARKER_FILE_NAME: &str = "update-ready.json";
/// Minimum number of bytes between two `Downloading` events (keeps the event bus quiet).
pub const PROGRESS_STEP: u64 = 512 * 1024;
/// How long opening a connection to the update host may take (TCP and TLS), for the manifest and
/// the package alike. The plugin sets none, which leaves it to the operating system (about 20 s on
/// Windows, longer elsewhere) while the status sits at `Checking` (user report 2026-09-29).
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// How long one read may stall (the manifest's answer, the next chunk of the package) before the
/// run fails as timed out. Without it a connection that goes quiet keeps the run waiting for good.
pub const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// The update source of this build. A value managed on the Tauri builder before `setup` overrides
/// the build's (`tests/update.rs` points it at a local manifest server).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdaterConfig {
    /// Manifest URLs in the order they are asked (placeholders allowed): the mirror's, then the
    /// build's. Never empty.
    pub endpoints: Vec<Url>,
    /// minisign public key (the `dW50cnVzdGVk…` text the Tauri CLI prints).
    pub pubkey: String,
    /// Delay before the automatic check at startup ([`AUTO_CHECK_DELAY`] in production).
    pub auto_check_delay: Duration,
    /// How long after a run the automatic update looks again ([`AUTO_CHECK_INTERVAL`] in
    /// production, [`UPDATE_INTERVAL_ENV`] in a test build).
    pub auto_check_interval: Duration,
    /// [`CONNECT_TIMEOUT`] in production.
    pub connect_timeout: Duration,
    /// [`READ_TIMEOUT`] in production.
    pub read_timeout: Duration,
}

impl UpdaterConfig {
    /// The values baked into this binary, or `None` when the URL or the key is missing.
    pub fn from_build() -> Option<Self> {
        Self::from_values(option_env!("VOLTIP_UPDATE_URL"), option_env!("VOLTIP_UPDATE_MIRROR"), option_env!("VOLTIP_UPDATE_PUBKEY"))
            .map(|config| Self { auto_check_interval: auto_check_interval(option_env!("VOLTIP_UPDATE_INTERVAL_SECS")), ..config })
    }

    /// The URL and the key present, trimmed and non-empty, and the URL parses; anything else is
    /// "no updater". A mirror that is set and parses is asked first (`<mirror>/latest.json`); one
    /// that does not parse is left out, so a bad optional value never takes the updater away.
    pub fn from_values(url: Option<&str>, mirror: Option<&str>, pubkey: Option<&str>) -> Option<Self> {
        let web = |text: &str| Url::parse(text).ok().filter(|u| matches!(u.scheme(), "https" | "http"));
        let url = url.map(str::trim).filter(|s| !s.is_empty())?;
        let pubkey = pubkey.map(str::trim).filter(|s| !s.is_empty())?;
        let endpoint = web(url)?;
        let mirror = mirror.map(str::trim).filter(|s| !s.is_empty()).and_then(|base| web(&format!("{}/latest.json", base.trim_end_matches('/'))));
        let endpoints = mirror.into_iter().chain([endpoint]).collect();
        Some(Self {
            endpoints,
            pubkey: pubkey.to_owned(),
            auto_check_delay: AUTO_CHECK_DELAY,
            auto_check_interval: AUTO_CHECK_INTERVAL,
            connect_timeout: CONNECT_TIMEOUT,
            read_timeout: READ_TIMEOUT,
        })
    }

    /// The `plugins.updater` block the plugin deserialises at initialisation (injected into the
    /// Tauri context by [`crate::run`], so `tauri.conf.json` never holds the key), for the package
    /// this binary was bundled into.
    pub fn plugin_config(&self) -> serde_json::Value {
        self.plugin_config_for(tauri::utils::platform::bundle_type())
    }

    /// [`Self::plugin_config`] for a binary bundled as `bundle`: the Windows install mode follows
    /// the package ([`windows_install_mode`]); the plugin reads it on Windows only.
    pub fn plugin_config_for(&self, bundle: Option<BundleType>) -> serde_json::Value {
        let endpoints: Vec<&str> = self.endpoints.iter().map(Url::as_str).collect();
        let mut config = serde_json::json!({ "pubkey": self.pubkey, "endpoints": endpoints, "windows": { "installMode": windows_install_mode(bundle) } });
        if self.local_http() {
            config["dangerousInsecureTransportProtocol"] = true.into();
        }
        config
    }

    /// The source asks a manifest server over plain http, and every such server is on this
    /// computer or the local network: a test build pointed at a server on the LAN
    /// (docs/dictation.md §9). The plugin refuses http in a release build unless told otherwise,
    /// which would stop such a build from starting; a public host still needs https, and the
    /// package's signature is checked either way.
    pub fn local_http(&self) -> bool {
        self.endpoints.iter().any(|url| url.scheme() == "http") && self.endpoints.iter().all(|url| url.scheme() != "http" || on_this_network(url))
    }
}

/// `url` names this computer or an address of the local network: `localhost`, a loopback address,
/// or a private or link-local IPv4 address. Host names other than `localhost` do not count, since
/// any name can resolve to a public address.
fn on_this_network(url: &Url) -> bool {
    let Some(host) = url.host_str() else { return false };
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match host.trim_start_matches('[').trim_end_matches(']').parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        Ok(std::net::IpAddr::V6(ip)) => ip.is_loopback(),
        Err(_) => false,
    }
}

/// The interval [`UPDATE_INTERVAL_ENV`] sets, in seconds and at least
/// [`MIN_AUTO_CHECK_INTERVAL`]; [`AUTO_CHECK_INTERVAL`] when it is unset or not a number.
pub fn auto_check_interval(secs: Option<&str>) -> Duration {
    secs.and_then(|text| text.trim().parse::<u64>().ok()).map_or(AUTO_CHECK_INTERVAL, |secs| Duration::from_secs(secs).max(MIN_AUTO_CHECK_INTERVAL))
}

/// How the plugin runs a Windows package (docs/dictation.md §9; user request 2026-10-17: an
/// in-app update showed the installer's window). The NSIS installer Voltip ships installs for the
/// current user and needs no elevation, so it runs silently (`/S`) and opens Voltip again when it
/// is done. Anything else (an MSI, an exe the bundler never marked) keeps the plugin's passive
/// mode, which can still show the elevation prompt a per-machine package needs; the installer
/// hides the window of a passive update run itself (`nsis/installer-hooks.nsh`), which is also what
/// an update from a release before this one runs.
pub fn windows_install_mode(bundle: Option<BundleType>) -> &'static str {
    match bundle {
        Some(BundleType::Nsis) => "quiet",
        _ => "passive",
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

/// When the automatic update looks next (module documentation, 「Looking again while Voltip
/// runs」). Times are `tokio::time::Instant`, so the tests run hours on a paused clock.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AutoSchedule {
    /// `Settings.auto_update` as last read.
    on: bool,
    /// When the next look is due; `None` when none is.
    at: Option<Instant>,
    /// The next look may install the version the marker remembers: true for the look at start
    /// alone, which finishes what an earlier start downloaded.
    installs: bool,
}

impl AutoSchedule {
    /// The setting as first read after start: on → a look `delay` from `now`, which may install.
    pub fn start(&mut self, on: bool, delay: Duration, now: Instant) {
        *self = Self { on, at: on.then(|| now + delay), installs: on };
    }

    /// The toggle moved: on → a look at once, download only; off → no look at all.
    pub fn switch(&mut self, on: bool, now: Instant) {
        *self = Self { on, at: on.then_some(now), installs: false };
    }

    /// A run ended on `status`: with the toggle on, look again `interval` from `now`, or keep a look
    /// that is due sooner (the one at start). A package waiting for the restart needs no more looks,
    /// and an install in progress none at all.
    pub fn look_again(&mut self, status: &UpdateStatus, interval: Duration, now: Instant) {
        if !self.on || matches!(status, UpdateStatus::Ready { .. } | UpdateStatus::Installing { .. }) {
            return;
        }
        let next = now + interval;
        self.at = Some(self.at.map_or(next, |at| at.min(next)));
    }

    /// When the next look is due.
    pub fn due_at(&self) -> Option<Instant> {
        self.at
    }

    /// The look due at `now`, if one is: `Some(installs)`. Taking it spends the permission to
    /// install, so only the look at start ever has it.
    pub fn take_due(&mut self, now: Instant) -> Option<bool> {
        if !self.at.is_some_and(|at| now >= at) {
            return None;
        }
        self.at = None;
        Some(std::mem::take(&mut self.installs))
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
    /// The proxy the check and the download go through (docs/dictation.md §24).
    network: voltip_net::NetworkRef,
    marker_path: PathBuf,
    status: Mutex<UpdateStatus>,
    pending: Mutex<Option<Pending>>,
    busy: AtomicBool,
    /// The automatic looks ([`follow_settings`] keeps them; every run's end moves them on). Taken
    /// after `status` when both are held.
    schedule: Mutex<AutoSchedule>,
    /// Wakes [`follow_settings`] when a run's end has moved the schedule.
    wake: Notify,
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
        Self {
            config,
            network: voltip_net::NetworkRef::default(),
            marker_path: data_dir.join(MARKER_FILE_NAME),
            status: Mutex::new(status),
            pending: Mutex::new(None),
            busy: AtomicBool::new(false),
            schedule: Mutex::new(AutoSchedule::default()),
            wake: Notify::new(),
        }
    }

    /// The check and the download go through `network` (`CoreConfig::network`).
    pub fn with_network(mut self, network: &Arc<voltip_net::Network>) -> Self {
        self.network = voltip_net::NetworkRef::new(network);
        self
    }

    /// This build can update itself.
    pub fn enabled(&self) -> bool {
        self.config.is_some()
    }

    /// Delay before the automatic check at startup (zero when there is no updater).
    pub fn auto_check_delay(&self) -> Duration {
        self.config.as_ref().map_or(Duration::ZERO, |c| c.auto_check_delay)
    }

    /// How long after a run the automatic update looks again.
    pub fn auto_check_interval(&self) -> Duration {
        self.config.as_ref().map_or(AUTO_CHECK_INTERVAL, |c| c.auto_check_interval)
    }

    /// The automatic looks as they stand.
    pub fn schedule(&self) -> AutoSchedule {
        self.schedule.lock().clone()
    }

    /// Last published status (what `update_status` answers).
    pub fn status(&self) -> UpdateStatus {
        self.status.lock().clone()
    }

    /// Record `status` and broadcast it like a core event. The broadcast happens under the status
    /// lock, so the events go out in the order the statuses were recorded.
    pub fn publish(&self, bridge: &Bridge, status: UpdateStatus) {
        self.publish_with(status, |status| bridge.publish(UiEvent::Update(status)));
    }

    /// [`Self::publish`] with the broadcast handed in.
    fn publish_with(&self, status: UpdateStatus, broadcast: impl FnOnce(UpdateStatus)) {
        let mut current = self.status.lock();
        *current = status.clone();
        broadcast(status);
    }

    /// End a run: release the slot, then broadcast the run's final status (if it has one). Whoever
    /// reacts to that status may start the next run at once (CI 2026-09-29: 「安装」 right after
    /// 「已是最新」 was refused as busy); a run that starts in between waits for the lock, so its
    /// `checking` still goes out after this status. With auto-update on, the next automatic look is
    /// scheduled from here ([`AutoSchedule::look_again`]); an install schedules none, and still
    /// releases the slot and broadcasts.
    pub fn finish(&self, bridge: &Bridge, last: Option<UpdateStatus>) {
        self.finish_with(last, |status| bridge.publish(UiEvent::Update(status)));
    }

    /// [`Self::finish`] with the broadcast handed in.
    fn finish_with(&self, last: Option<UpdateStatus>, broadcast: impl FnOnce(UpdateStatus)) {
        let mut current = self.status.lock();
        self.end();
        if let Some(status) = &last {
            *current = status.clone();
        }
        self.schedule.lock().look_again(&current, self.auto_check_interval(), Instant::now());
        self.wake.notify_one();
        if let Some(status) = last {
            broadcast(status);
        }
    }

    /// Release the slot (a run that ends goes through [`Self::finish`]).
    pub fn end(&self) {
        self.busy.store(false, Ordering::Release);
    }

    /// Claim the slot for one run; `false` while another run is in flight.
    pub fn try_begin(&self) -> bool {
        self.busy.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_ok()
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

/// `error` followed by its causes, outermost first: `error sending request for url (…): client
/// error (Connect): tcp connect error: Connection refused (os error 111)`. The first line alone never
/// says why a request failed, and that line is all `to_string` gives (user report 2026-09-29). A
/// cause whose text is already in the message is left out.
pub fn describe(error: &dyn std::error::Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let text = cause.to_string();
        if !text.is_empty() && !message.contains(&text) {
            message.push_str(": ");
            message.push_str(&text);
        }
        source = cause.source();
    }
    message
}

/// One run against the plugin; failures become `Failed`. The slot is released before the run's
/// final status goes out ([`UpdateSlot::finish`]).
async fn drive<R: Runtime>(app: AppHandle<R>, bridge: Bridge, slot: Arc<UpdateSlot>, intent: Intent) {
    let last = match run(&app, &bridge, &slot, &intent).await {
        Ok(last) => last,
        Err(message) => {
            tracing::warn!(error = %message, ?intent, "updater run failed");
            Some(UpdateStatus::Failed { message })
        }
    };
    slot.finish(&bridge, last);
}

/// The run itself: the statuses along the way are published here, the final one (`up_to_date`,
/// `available`, `ready`) is returned for [`drive`] to publish once the slot is free. `None`: the
/// last published status stands, or the app is restarting.
async fn run<R: Runtime>(app: &AppHandle<R>, bridge: &Bridge, slot: &Arc<UpdateSlot>, intent: &Intent) -> Result<Option<UpdateStatus>, String> {
    let Some(config) = slot.config.clone() else { return Err(NOT_CONFIGURED.to_owned()) };
    // An explicit check always asks the manifest (and forgets what the last one found); the other
    // intents reuse the pending update.
    let reuse = slot.take_pending().filter(|_| *intent != Intent::Check);
    let (mut pending, found) = match reuse {
        Some(pending) => (pending, None),
        None => {
            slot.publish(bridge, UpdateStatus::Checking);
            let (connect, read) = (config.connect_timeout, config.read_timeout);
            let network = slot.network.get().clone();
            // The client settings reach the package download too: the plugin hands them on to the
            // `Update` it returns. The proxy goes on last: the plugin adds none of its own here.
            let updater = app
                .updater_builder()
                .endpoints(config.endpoints)
                .map_err(|e| describe(&e))?
                .pubkey(config.pubkey)
                .configure_client(move |client| network.apply(client.connect_timeout(connect).read_timeout(read)))
                .build()
                .map_err(|e| describe(&e))?;
            match updater.check().await.map_err(|e| describe(&e))? {
                None => {
                    slot.clear_marker();
                    let version = app.package_info().version.to_string();
                    tracing::info!(version, "no update available");
                    return Ok(Some(UpdateStatus::UpToDate { version, checked_at: now_secs() }));
                }
                Some(update) => {
                    tracing::info!(version = %update.version, current = %update.current_version, "update available");
                    let available = UpdateStatus::Available {
                        version: update.version.clone(),
                        current: update.current_version.clone(),
                        notes: update.body.clone(),
                        date: pub_date(&update.raw_json),
                    };
                    (Pending { update, bytes: None }, Some(available))
                }
            }
        }
    };
    let version = pending.update.version.clone();
    if !intent.downloads() {
        slot.set_pending(pending);
        return Ok(found);
    }
    if let Some(available) = found {
        slot.publish(bridge, available);
    }
    let mut downloaded = false;
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
            .map_err(|e| describe(&e))?;
        tracing::info!(version, bytes = bytes.len(), "update downloaded and verified");
        pending.bytes = Some(bytes);
        slot.write_marker(&version);
        downloaded = true;
    }
    let ready = downloaded.then(|| UpdateStatus::Ready { version: version.clone() });
    if !intent.installs(&version) {
        slot.set_pending(pending);
        return Ok(ready);
    }
    if let Some(ready) = ready {
        slot.publish(bridge, ready);
    }
    slot.publish(bridge, UpdateStatus::Installing { version: version.clone() });
    let Pending { update, bytes } = pending;
    let bytes = bytes.unwrap_or_default();
    #[cfg(target_os = "macos")]
    let entries = app.try_state::<crate::restart::Restart>().and_then(|restart| restart.handoff_state());
    #[cfg(not(target_os = "macos"))]
    let entries = None;
    // Windows: the installer is launched and the process exits inside `install`; macOS / Linux: the
    // bundle is replaced and the app has to relaunch itself. The automatic backup goes first.
    let backup = bridge.clone();
    tauri::async_runtime::spawn_blocking(move || {
        backup_then_install(
            || backup.backup_auto(AutoKind::Update),
            || {
                prepare_then_install(
                    bytes,
                    entries,
                    |package, entries| {
                        #[cfg(target_os = "macos")]
                        {
                            crate::keychain_handoff::prepare_update(package, entries)?;
                            tracing::info!(entries = entries.len(), "staged update persisted the keychain hand-over");
                        }
                        #[cfg(not(target_os = "macos"))]
                        let _ = (package, entries);
                        Ok(())
                    },
                    |package| update.install(package).map_err(|e| describe(&e)),
                )
            },
        )
    })
    .await
    .map_err(|e| describe(&e))??;
    slot.clear_marker();
    tracing::info!(version, "update installed; restarting");
    crate::restart::after_update(app);
    Ok(None)
}

/// The automatic backup before an update (docs/dictation.md §4.6.1), then `install`. The backup
/// takes as long as the history does, with no time limit (given up halfway, it would leave a
/// `.part`); its time goes to the log. One that fails does not hold the update up: it is logged,
/// and the core shows it once after the restart.
fn backup_then_install(backup: impl FnOnce() -> AutoOutcome, install: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    let started = std::time::Instant::now();
    let outcome = backup();
    let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    match outcome {
        Ok(file) => tracing::info!(file = %file.name, ms, "automatic backup before the update written"),
        Err(e) => tracing::warn!(error = %e, ms, "automatic backup before the update failed; installing all the same"),
    }
    install()
}

fn prepare_then_install(
    package: Vec<u8>,
    entries: Option<Entries>,
    prepare: impl FnOnce(&[u8], &Entries) -> Result<(), String>,
    install: impl FnOnce(Vec<u8>) -> Result<(), String>,
) -> Result<(), String> {
    if let Some(entries) = entries {
        prepare(&package, &entries)?;
    }
    install(package)
}

/// Follow `Settings.auto_update` and keep the automatic looks ([`AutoSchedule`]): on at startup →
/// a look after [`AUTO_CHECK_DELAY`], the only one that may install (the version remembered as
/// `Ready`); turned on later → a look now, download only; turned off → no look; each run that ends
/// while it is on schedules the next look ([`UpdateSlot::finish`]). One task: it sleeps until the
/// next look is due and wakes on every settings change and every run's end.
pub fn follow_settings<R: Runtime>(app: AppHandle<R>, bridge: Bridge, slot: Arc<UpdateSlot>) {
    let mut events = bridge.events();
    let initial = bridge.state();
    // `Ready` may already have been folded before this subscription: read it from the cache.
    let mut current: Option<bool> = initial.identity.is_some().then_some(initial.settings.auto_update);
    if let Some(on) = current {
        slot.schedule.lock().start(on, slot.auto_check_delay(), Instant::now());
    }
    tauri::async_runtime::spawn(async move {
        loop {
            let due = slot.schedule.lock().due_at();
            let next = tokio::select! {
                event = events.recv() => match event {
                    Ok(UiEvent::Settings(s)) => Some(s.auto_update),
                    Ok(UiEvent::State(state)) => Some(state.settings.auto_update),
                    Ok(_) | Err(RecvError::Lagged(_)) => None,
                    Err(RecvError::Closed) => break,
                },
                () = slot.wake.notified() => None,
                () = sleep_until(due) => {
                    look(&app, &bridge, &slot);
                    None
                }
            };
            let Some(next) = next else { continue };
            match current {
                None => slot.schedule.lock().start(next, slot.auto_check_delay(), Instant::now()),
                Some(was) if was != next => slot.schedule.lock().switch(next, Instant::now()),
                Some(_) => {}
            }
            current = Some(next);
        }
    });
}

/// Wait until `due`; for good when nothing is.
async fn sleep_until(due: Option<Instant>) {
    match due {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

/// Run the automatic look that is due, if one is. A look that falls due during another run (the
/// user's) is dropped: that run answers the same question and schedules the next look as it ends.
fn look<R: Runtime>(app: &AppHandle<R>, bridge: &Bridge, slot: &Arc<UpdateSlot>) {
    let Some(installs) = slot.schedule.lock().take_due(Instant::now()) else { return };
    let install_version = if installs { slot.read_marker() } else { None };
    tracing::info!(installs, ?install_version, "automatic update check");
    if let Err(e) = request(app, bridge, slot, Intent::Auto { install_version }) {
        tracing::info!(error = %e, "automatic update check skipped");
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn config_needs_both_values_and_a_parsable_url() {
        assert_eq!(UpdaterConfig::from_values(None, None, None), None);
        assert_eq!(UpdaterConfig::from_values(Some("https://updates.example.test/latest.json"), None, None), None);
        assert_eq!(UpdaterConfig::from_values(None, None, Some("key")), None);
        assert_eq!(UpdaterConfig::from_values(Some("  "), None, Some("key")), None, "blank URL is unset");
        assert_eq!(UpdaterConfig::from_values(Some("https://x.example"), None, Some("\n")), None, "blank key is unset");
        assert_eq!(UpdaterConfig::from_values(Some("not a url"), None, Some("key")), None);
        assert_eq!(UpdaterConfig::from_values(Some("ftp://x.example/latest.json"), None, Some("key")), None, "http(s) only");
        let cfg =
            UpdaterConfig::from_values(Some(" https://updates.example.test/{{target}}/{{arch}}/{{current_version}} "), None, Some(" dW50cnVzdGVk ")).unwrap();
        assert_eq!(cfg.pubkey, "dW50cnVzdGVk");
        assert_eq!(cfg.auto_check_delay, AUTO_CHECK_DELAY);
        assert_eq!((cfg.connect_timeout, cfg.read_timeout), (CONNECT_TIMEOUT, READ_TIMEOUT));
        // url::Url percent-encodes the braces in the path; the plugin replaces both spellings.
        assert_eq!(cfg.endpoints.len(), 1);
        assert!(cfg.endpoints[0].as_str().contains("%7B%7Btarget%7D%7D"), "{}", cfg.endpoints[0]);
        let json = cfg.plugin_config();
        assert_eq!(json["pubkey"], "dW50cnVzdGVk");
        assert_eq!(json["endpoints"].as_array().unwrap().len(), 1);
        assert!(format!("{cfg:?}").contains("updates.example.test"));
        // The build-time values are whatever the environment held when this test binary compiled;
        // only the shape of the answer is fixed.
        let _ = UpdaterConfig::from_build();
    }

    /// docs/dictation.md §9: the mirror's manifest is asked first, the build's URL second, and a
    /// mirror that is blank or does not parse leaves only the URL; it never takes the updater away.
    #[test]
    fn the_mirror_is_asked_before_the_release_manifest() {
        let github = "https://github.com/owner/repo/releases/latest/download/latest.json";
        let both = UpdaterConfig::from_values(Some(github), Some(" https://mirror.example.test/updates/ "), Some("key")).unwrap();
        let asked: Vec<&str> = both.endpoints.iter().map(Url::as_str).collect();
        assert_eq!(asked, ["https://mirror.example.test/updates/latest.json", github]);
        assert_eq!(both.plugin_config()["endpoints"], serde_json::json!(asked));
        for mirror in [None, Some(""), Some("  "), Some("not a url"), Some("ftp://mirror.example.test/updates")] {
            let only = UpdaterConfig::from_values(Some(github), mirror, Some("key")).unwrap();
            assert_eq!(only.endpoints.iter().map(Url::as_str).collect::<Vec<_>>(), [github], "{mirror:?}");
        }
        assert_eq!(UpdaterConfig::from_values(None, Some("https://mirror.example.test/updates"), Some("key")), None, "a mirror alone is no updater");
    }

    /// User request 2026-10-17: an in-app update on Windows showed the installer's window. The
    /// block the plugin reads asks for its quiet mode for the NSIS package, so the installer runs
    /// with `/S`, and keeps the passive mode (`/P`) for any other package.
    #[test]
    fn regression_a_windows_update_runs_the_nsis_installer_without_a_window() {
        let cfg = UpdaterConfig::from_values(Some("https://updates.example.test/latest.json"), None, Some("key")).unwrap();
        let read = |bundle: Option<BundleType>| -> tauri_plugin_updater::Config { serde_json::from_value(cfg.plugin_config_for(bundle)).unwrap() };
        let nsis = read(Some(BundleType::Nsis));
        let mode = nsis.windows.as_ref().map(|w| w.install_mode.clone()).unwrap();
        assert_eq!(mode.to_string(), "quiet");
        assert_eq!(mode.nsis_args(), ["/S"]);
        assert_eq!(nsis.pubkey, "key");
        assert_eq!(nsis.endpoints.iter().map(Url::as_str).collect::<Vec<_>>(), ["https://updates.example.test/latest.json"]);
        for other in [Some(BundleType::Msi), None] {
            let mode = read(other.clone()).windows.map(|w| w.install_mode).unwrap();
            assert_eq!((mode.to_string(), mode.nsis_args()), ("passive".to_owned(), &["/P"][..]), "{other:?}");
        }
        assert_eq!(windows_install_mode(Some(BundleType::Nsis)), "quiet");
        assert_eq!(windows_install_mode(Some(BundleType::Deb)), "passive");
        // What this binary injects follows the package the bundler marked it for.
        assert_eq!(cfg.plugin_config(), cfg.plugin_config_for(tauri::utils::platform::bundle_type()));
    }

    #[test]
    fn an_install_runs_only_after_the_staged_hand_over_commits() {
        let order = std::sync::Mutex::new(Vec::new());
        prepare_then_install(
            vec![7],
            Some(vec![("entry".into(), None)]),
            |package, entries| {
                assert_eq!(package, [7]);
                assert_eq!(entries.len(), 1);
                order.lock().unwrap().push("prepare");
                Ok(())
            },
            |package| {
                assert_eq!(package, [7]);
                order.lock().unwrap().push("install");
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(*order.lock().unwrap(), ["prepare", "install"]);

        assert_eq!(
            prepare_then_install(
                vec![8],
                Some(vec![("entry".into(), None)]),
                |_, _| Err("persistence failed".into()),
                |_| panic!("install must not run after a failed hand-over"),
            ),
            Err("persistence failed".into())
        );
    }

    fn written() -> AutoOutcome {
        Ok(voltip_core::backup::auto::AutoBackupFile {
            name: "voltip-auto-20261030-152000-update.voltip-backup".into(),
            at_ms: 1_793_000_000_000,
            kind: AutoKind::Update,
        })
    }

    /// docs/dictation.md §4.6.1: the backup is written before the installer runs; one that fails
    /// does not stop the update; an install that fails still says why.
    #[test]
    fn the_backup_before_an_update_runs_first_and_never_stops_it() {
        let order = std::sync::Mutex::new(Vec::new());
        backup_then_install(
            || {
                order.lock().unwrap().push("backup");
                written()
            },
            || {
                order.lock().unwrap().push("install");
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(*order.lock().unwrap(), ["backup", "install"]);

        let installed = std::sync::atomic::AtomicBool::new(false);
        backup_then_install(
            || Err("No space left on device (os error 28)".into()),
            || {
                installed.store(true, Ordering::SeqCst);
                Ok(())
            },
        )
        .unwrap();
        assert!(installed.load(Ordering::SeqCst), "a failed backup does not hold the update up");
        assert_eq!(backup_then_install(written, || Err("signature mismatch".into())), Err("signature mismatch".into()));
    }

    /// Through the real bridge and core: a backup before an update that cannot be written still
    /// lets the (fake) installer run, and leaves the notice the next start shows; one that can is
    /// in the automatic backups' folder before the installer starts.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_update_backs_up_through_the_core_before_its_installer() {
        let start = |dir: &Path, folder: Option<String>| {
            voltip_core::SettingsStore::new(dir)
                .save(&voltip_core::Settings {
                    relay_enabled: false,
                    engines: voltip_core::dictation::fakes::fake_engines(),
                    backup: voltip_core::BackupSettings { weekly: true, folder },
                    ..Default::default()
                })
                .unwrap();
            let mut config = voltip_core::CoreConfig::new(dir.to_path_buf());
            config.auto_backup = true;
            Bridge::start(config, Arc::new(voltip_identity::MemorySecretStore::new())).unwrap()
        };
        let dir = tempfile::tempdir().unwrap();
        let bridge = start(dir.path(), None);
        let folder = dir.path().join(voltip_core::backup::auto::AUTO_BACKUP_DIR);
        let seen = folder.clone();
        let core = bridge.clone();
        let outcome = tauri::async_runtime::spawn_blocking(move || {
            backup_then_install(
                || core.backup_auto(AutoKind::Update),
                move || {
                    let kinds: Vec<AutoKind> = voltip_core::backup::auto::list(&seen).unwrap().iter().map(|f| f.kind).collect();
                    assert_eq!(kinds, [AutoKind::Update], "written before the installer starts");
                    Ok(())
                },
            )
        })
        .await
        .unwrap();
        assert_eq!(outcome, Ok(()));
        bridge.shutdown();

        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("a file");
        std::fs::write(&blocker, b"x").unwrap();
        let bridge = start(dir.path(), Some(blocker.join("backups").display().to_string()));
        let installed = Arc::new(AtomicBool::new(false));
        let ran = installed.clone();
        let core = bridge.clone();
        let outcome = tauri::async_runtime::spawn_blocking(move || {
            backup_then_install(
                || core.backup_auto(AutoKind::Update),
                move || {
                    ran.store(true, Ordering::SeqCst);
                    Ok(())
                },
            )
        })
        .await
        .unwrap();
        assert_eq!(outcome, Ok(()));
        assert!(installed.load(Ordering::SeqCst));
        assert!(dir.path().join(voltip_core::backup::auto::NOTICE_FILE_NAME).exists(), "the next start shows the failure");
        bridge.shutdown();
    }

    /// An error with an optional cause, standing in for reqwest's and hyper's chain.
    #[derive(Debug)]
    struct Layer(&'static str, Option<Box<Layer>>);

    impl std::fmt::Display for Layer {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }

    impl std::error::Error for Layer {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            self.1.as_deref().map(|cause| cause as &(dyn std::error::Error + 'static))
        }
    }

    #[test]
    fn regression_describe_keeps_every_cause_of_a_failed_request() {
        // User report 2026-09-29: 「更新失败 · error sending request for url (…)」 and nothing
        // about why; the causes below that line were dropped.
        let chain = Layer(
            "error sending request for url (https://updates.example.test/latest.json)",
            Some(Box::new(Layer(
                "client error (Connect)",
                Some(Box::new(Layer("tcp connect error", Some(Box::new(Layer("Connection refused (os error 111)", None)))))),
            ))),
        );
        assert_eq!(
            describe(&chain),
            "error sending request for url (https://updates.example.test/latest.json): client error (Connect): tcp connect error: Connection refused (os error 111)"
        );
        // A cause already spelled out by the layer above, or with no text, adds nothing.
        let repeated = Layer("operation timed out: deadline", Some(Box::new(Layer("deadline", Some(Box::new(Layer("", None)))))));
        assert_eq!(describe(&repeated), "operation timed out: deadline");
        assert_eq!(describe(&Layer("builder error", None)), "builder error");
    }

    /// User request 2026-10-25: with auto-update on, Voltip looks again while it runs, and only the
    /// look at start may install.
    #[tokio::test(start_paused = true)]
    async fn the_automatic_looks_follow_the_toggle_and_each_runs_end() {
        let up_to_date = UpdateStatus::UpToDate { version: "0.1.8".into(), checked_at: 1 };
        let mut schedule = AutoSchedule::default();
        // Off at start: nothing is due, and a run's end schedules nothing.
        schedule.start(false, AUTO_CHECK_DELAY, Instant::now());
        schedule.look_again(&up_to_date, AUTO_CHECK_INTERVAL, Instant::now());
        assert_eq!(schedule.due_at(), None);
        tokio::time::advance(3 * AUTO_CHECK_INTERVAL).await;
        assert_eq!(schedule.take_due(Instant::now()), None);

        // On at start: one look after the delay, which may install.
        let started = Instant::now();
        schedule.start(true, AUTO_CHECK_DELAY, started);
        assert_eq!(schedule.due_at(), Some(started + AUTO_CHECK_DELAY));
        // A check by hand that ends first keeps the look at start, which is due sooner.
        schedule.look_again(&up_to_date, AUTO_CHECK_INTERVAL, started + Duration::from_secs(1));
        assert_eq!(schedule.due_at(), Some(started + AUTO_CHECK_DELAY));
        tokio::time::advance(AUTO_CHECK_DELAY - Duration::from_millis(1)).await;
        assert_eq!(schedule.take_due(Instant::now()), None, "not due yet");
        tokio::time::advance(Duration::from_millis(1)).await;
        assert_eq!(schedule.take_due(Instant::now()), Some(true), "the look at start may install");
        assert_eq!(schedule.take_due(Instant::now()), None, "taken once");

        // Its end schedules the next look an interval later, which only downloads.
        let ended = Instant::now();
        schedule.look_again(&up_to_date, AUTO_CHECK_INTERVAL, ended);
        assert_eq!(schedule.due_at(), Some(ended + AUTO_CHECK_INTERVAL));
        tokio::time::advance(AUTO_CHECK_INTERVAL - Duration::from_secs(1)).await;
        assert_eq!(schedule.take_due(Instant::now()), None);
        tokio::time::advance(Duration::from_secs(1)).await;
        assert_eq!(schedule.take_due(Instant::now()), Some(false), "a later look never installs");

        // A failed look is tried again an interval later, never at once.
        let failed = UpdateStatus::Failed { message: "offline".into() };
        schedule.look_again(&failed, AUTO_CHECK_INTERVAL, Instant::now());
        assert_eq!(schedule.due_at(), Some(Instant::now() + AUTO_CHECK_INTERVAL));

        // A package waiting for the restart, or an install, stops the looks.
        for waiting in [UpdateStatus::Ready { version: "0.1.9".into() }, UpdateStatus::Installing { version: "0.1.9".into() }] {
            schedule.switch(true, Instant::now());
            assert_eq!(schedule.take_due(Instant::now()), Some(false), "turned on: a look at once, download only");
            schedule.look_again(&waiting, AUTO_CHECK_INTERVAL, Instant::now());
            assert_eq!(schedule.due_at(), None, "{waiting:?}");
        }

        // Turned off: the look that is due goes, and so does the permission to install.
        schedule.start(true, AUTO_CHECK_DELAY, Instant::now());
        schedule.switch(false, Instant::now());
        assert_eq!(schedule, AutoSchedule { on: false, at: None, installs: false });
        schedule.look_again(&up_to_date, AUTO_CHECK_INTERVAL, Instant::now());
        tokio::time::advance(3 * AUTO_CHECK_INTERVAL).await;
        assert_eq!(schedule.take_due(Instant::now()), None);
        schedule.switch(true, Instant::now());
        assert_eq!(schedule.take_due(Instant::now()), Some(false), "on again: it only downloads");
    }

    /// The end of every run moves the schedule on through the slot itself: the next look an
    /// interval later, and nothing after an install, which still frees the slot.
    #[tokio::test(start_paused = true)]
    async fn a_runs_end_schedules_the_next_look_through_the_slot() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = UpdaterConfig::from_values(Some("https://x.example/latest.json"), None, Some("k")).unwrap();
        config.auto_check_interval = Duration::from_secs(90);
        let slot = UpdateSlot::new(Some(config), dir.path());
        assert_eq!(slot.auto_check_interval(), Duration::from_secs(90));
        slot.schedule.lock().start(true, AUTO_CHECK_DELAY, Instant::now());
        tokio::time::advance(AUTO_CHECK_DELAY).await;
        assert_eq!(slot.schedule.lock().take_due(Instant::now()), Some(true));
        assert!(slot.try_begin());
        slot.finish_with(Some(UpdateStatus::UpToDate { version: "0.1.8".into(), checked_at: 1 }), |_| {});
        assert_eq!(slot.schedule().due_at(), Some(Instant::now() + Duration::from_secs(90)));
        // A run that installed: the status stands at `installing`, nothing more is scheduled.
        slot.schedule.lock().switch(true, Instant::now());
        assert_eq!(slot.schedule.lock().take_due(Instant::now()), Some(false));
        slot.publish_with(UpdateStatus::Installing { version: "0.1.9".into() }, |_| {});
        assert!(slot.try_begin());
        let mut sent = false;
        slot.finish_with(None, |_| sent = true);
        assert!(!sent, "no final status: nothing more to send");
        assert_eq!(slot.schedule().due_at(), None);
        assert!(slot.try_begin(), "and the slot is free");
        // Without an updater the interval is still the production one.
        assert_eq!(UpdateSlot::new(None, dir.path()).auto_check_interval(), AUTO_CHECK_INTERVAL);
    }

    #[test]
    fn a_test_build_can_look_again_sooner_but_not_in_a_loop() {
        assert_eq!(auto_check_interval(None), AUTO_CHECK_INTERVAL);
        assert_eq!(auto_check_interval(Some("")), AUTO_CHECK_INTERVAL);
        assert_eq!(auto_check_interval(Some("soon")), AUTO_CHECK_INTERVAL, "not a number");
        assert_eq!(auto_check_interval(Some("-5")), AUTO_CHECK_INTERVAL);
        assert_eq!(auto_check_interval(Some(" 120 ")), Duration::from_secs(120));
        assert_eq!(auto_check_interval(Some("1")), MIN_AUTO_CHECK_INTERVAL, "at least a minute");
        assert_eq!(auto_check_interval(Some("0")), MIN_AUTO_CHECK_INTERVAL);
        let cfg = UpdaterConfig::from_values(Some("https://updates.example.test/latest.json"), None, Some("key")).unwrap();
        assert_eq!(cfg.auto_check_interval, AUTO_CHECK_INTERVAL);
    }

    /// A test build pointed at a manifest server on the LAN starts and checks; a public host
    /// still needs https.
    #[test]
    fn plain_http_is_allowed_only_on_this_computer_and_the_local_network() {
        let source = |url: &str, mirror: Option<&str>| UpdaterConfig::from_values(Some(url), mirror, Some("key")).unwrap();
        let insecure = |cfg: &UpdaterConfig| cfg.plugin_config().get("dangerousInsecureTransportProtocol").cloned();
        for local in [
            "http://192.168.157.161:18765/latest.json",
            "http://10.0.0.2/latest.json",
            "http://172.16.4.1/latest.json",
            "http://127.0.0.1:8080/latest.json",
            "http://localhost:8080/latest.json",
            "http://LOCALHOST/latest.json",
            "http://169.254.10.1/latest.json",
            "http://[::1]:8080/latest.json",
        ] {
            let cfg = source(local, None);
            assert!(cfg.local_http(), "{local}");
            assert_eq!(insecure(&cfg), Some(serde_json::Value::Bool(true)), "{local}");
            // The plugin takes the block, here and in a release build alike.
            let read: tauri_plugin_updater::Config = serde_json::from_value(cfg.plugin_config()).unwrap();
            assert!(read.dangerous_insecure_transport_protocol, "{local}");
        }
        for public in
            ["http://8.8.8.8/latest.json", "http://updates.example.test/latest.json", "http://code-server:8080/latest.json", "http://[2001:db8::1]/latest.json"]
        {
            let cfg = source(public, None);
            assert!(!cfg.local_http(), "{public}");
            assert_eq!(insecure(&cfg), None, "{public}");
        }
        let https = source("https://updates.example.test/latest.json", None);
        assert!(!https.local_http(), "https needs nothing");
        assert_eq!(insecure(&https), None);
        // One public http endpoint among local ones keeps the plugin's rule.
        assert!(!source("http://192.168.1.2/latest.json", Some("http://updates.example.test/updates")).local_http());
        assert!(source("http://192.168.1.2/latest.json", Some("https://mirror.example.test/updates")).local_http());
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
    fn regression_the_slot_is_free_before_the_final_status_goes_out() {
        // CI 2026-09-29 (main, tests/update.rs check_against_a_204_manifest_reports_up_to_date):
        // 「安装」 right after 「已是最新」 was refused as busy, because the run published its last
        // status and only then released the slot.
        let dir = tempfile::tempdir().unwrap();
        let slot = UpdateSlot::new(UpdaterConfig::from_values(Some("https://x.example/latest.json"), None, Some("k")), dir.path());
        assert!(slot.try_begin());
        let status = UpdateStatus::UpToDate { version: "1.0.0".into(), checked_at: 1 };
        let mut free_when_sent = None;
        slot.finish_with(Some(status.clone()), |sent| {
            assert_eq!(sent, status);
            free_when_sent = Some(!slot.busy.load(Ordering::Acquire));
        });
        assert_eq!(free_when_sent, Some(true), "whoever reacts to the status can start the next run");
        assert_eq!(slot.status(), status);
        // No final status (the last one stands): the slot is released all the same.
        assert!(slot.try_begin());
        slot.finish_with(None, |_| panic!("nothing to send"));
        assert!(slot.try_begin());
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
        let nested = UpdateSlot::new(UpdaterConfig::from_values(Some("https://x.example/latest.json"), None, Some("k")), &dir.path().join("a/b"));
        assert!(nested.enabled());
        assert_eq!(nested.auto_check_delay(), AUTO_CHECK_DELAY);
        assert_eq!(nested.status(), UpdateStatus::Idle);
        nested.write_marker("3.0.0");
        assert_eq!(nested.read_marker(), Some("3.0.0".into()));
        assert!(nested.take_pending().is_none());
    }
}
