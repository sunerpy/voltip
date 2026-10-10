#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Updates on the phone (docs/dictation.md §20.9) through the shell's commands, against a local
//! stand-in for GitHub's latest-release API: Google Play updates what it installed, so its install
//! checks nothing and points to the listing; an APK from a release asks GitHub and opens the newer
//! release's APK; 自动检查更新 checks by itself. The Tauri phone app had this up to 0.0.49; this app
//! has it again from the version after 0.0.50 (user decision 2026-10-09).

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use voltip_core::dictation::fakes;
use voltip_core::{Settings, SettingsStore};
use voltip_identity::MemorySecretStore;
use voltip_rn::Shell;
use voltip_rn::host::{HostCall, RecordingHost};
use voltip_rn::update::{PACKAGE, SOURCE_PENDING, UpdateConfig, store_listing};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const STEP_TIMEOUT: Duration = Duration::from_secs(15);

/// GitHub's answer for a release `tag` with the phone's APK among its files.
fn release(server: &MockServer, tag: &str) -> Value {
    let version = tag.trim_start_matches('v');
    json!({
        "tag_name": tag,
        "body": "## 新功能\n\n* 检查更新",
        "published_at": "2026-10-10T08:00:00Z",
        "assets": [
            { "name": format!("Voltip_{version}_android_arm64.aab"), "browser_download_url": format!("{}/download/{version}.aab", server.uri()) },
            { "name": format!("Voltip_{version}_android_arm64.apk"), "browser_download_url": format!("{}/download/{version}.apk", server.uri()) },
        ],
    })
}

struct Phone {
    shell: Shell,
    host: Arc<RecordingHost>,
    runtime: Option<tokio::runtime::Runtime>,
    _dir: tempfile::TempDir,
}

impl Phone {
    /// The app at version 0.0.51, installed by `installer`, with `settings`, asking `server`.
    fn start(installer: Option<&str>, settings: Settings, server: &MockServer) -> Self {
        let dir = tempfile::tempdir().unwrap();
        SettingsStore::new(dir.path()).save(&Settings { relay_enabled: false, ..settings }).unwrap();
        let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().worker_threads(2).build().unwrap();
        let config = voltip_rn::shell::phone_config(dir.path().to_path_buf(), "0.0.51");
        let host = RecordingHost::default();
        host.set_installer(installer);
        let host = Arc::new(host);
        // As in production, the installer is the host's to say.
        let updates = UpdateConfig {
            source: None,
            latest_release: format!("{}/repos/sunerpy/voltip/releases/latest", server.uri()),
            listing: store_listing(PACKAGE),
            auto_check_delay: Duration::ZERO,
        };
        let shell =
            Shell::start_with_updates(runtime.handle().clone(), config, Arc::new(MemorySecretStore::new()), fakes::ports(), host.clone(), updates).unwrap();
        Self { shell, host, runtime: Some(runtime), _dir: dir }
    }

    fn invoke(&self, command: &str) -> Result<Value, String> {
        self.shell.invoke_blocking(command, Value::Null)
    }

    /// Wait until the shell knows who installed the app (the host answers on a blocking thread):
    /// until then `update_install` is refused as pending. Its answer then. Only for an install
    /// from a release, where it opens nothing without a newer release.
    fn wait_source(&self) -> Result<Value, String> {
        let deadline = Instant::now() + STEP_TIMEOUT;
        loop {
            let answer = self.invoke("update_install");
            if answer.as_ref().err().map(String::as_str) != Some(SOURCE_PENDING) {
                return answer;
            }
            assert!(Instant::now() < deadline, "install source still unknown after {STEP_TIMEOUT:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Wait until `update_status` satisfies `done`; the status then.
    fn wait_status(&self, done: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + STEP_TIMEOUT;
        loop {
            let status = self.invoke("update_status").unwrap();
            if done(&status) {
                return status;
            }
            assert!(Instant::now() < deadline, "update status still {status} after {STEP_TIMEOUT:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Phone {
    fn drop(&mut self) {
        self.shell.shutdown();
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(Duration::from_secs(2));
        }
    }
}

/// A local stand-in for GitHub on a runtime of its own, answering with `tag`.
fn github(tag: &str) -> (tokio::runtime::Runtime, MockServer) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(MockServer::start());
    let body = release(&server, tag);
    runtime.block_on(
        Mock::given(method("GET"))
            .and(path("/repos/sunerpy/voltip/releases/latest"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server),
    );
    (runtime, server)
}

fn requests(runtime: &tokio::runtime::Runtime, server: &MockServer) -> usize {
    runtime.block_on(server.received_requests()).map_or(0, |r| r.len())
}

#[test]
fn an_install_from_a_release_checks_github_and_opens_the_newer_apk() {
    let (gh, server) = github("v0.0.52");
    let phone = Phone::start(Some("com.google.android.packageinstaller"), Settings::default(), &server);
    assert_eq!(phone.wait_source().unwrap_err(), "updater: 没有可下载的新版本，请先检查更新");
    assert_eq!(phone.invoke("update_status").unwrap()["state"], "idle");
    assert_eq!(phone.invoke("update_check").unwrap(), Value::Null);
    let status = phone.wait_status(|s| s["state"] == "available");
    assert_eq!(status["version"], "0.0.52");
    assert_eq!(status["current"], "0.0.51");
    assert_eq!(status["notes"], "## 新功能\n\n* 检查更新");
    phone.invoke("update_install").unwrap();
    let apk = format!("{}/download/0.0.52.apk", server.uri());
    assert!(phone.host.calls().contains(&HostCall::OpenUrl(apk)), "{:?}", phone.host.calls());
    assert!(phone.host.events().iter().any(|e| e["type"] == "update" && e["state"] == "available"), "the outcome went out as an event");
    assert_eq!(requests(&gh, &server), 1);
}

#[test]
fn the_same_version_is_up_to_date_and_nothing_opens() {
    let (_gh, server) = github("v0.0.51");
    let phone = Phone::start(None, Settings::default(), &server);
    phone.wait_source().unwrap_err();
    phone.invoke("update_check").unwrap();
    assert_eq!(phone.wait_status(|s| s["state"] == "up_to_date")["version"], "0.0.51");
    assert_eq!(phone.invoke("update_install").unwrap_err(), "updater: 没有可下载的新版本，请先检查更新");
}

#[test]
fn an_install_from_google_play_points_to_the_listing_and_checks_nothing() {
    let (gh, server) = github("v0.0.52");
    let phone = Phone::start(Some("com.android.vending"), Settings { auto_update: true, ..Settings::default() }, &server);
    assert_eq!(phone.wait_status(|s| s["state"] == "store")["version"], "0.0.51");
    assert_eq!(phone.invoke("update_check").unwrap(), Value::Null);
    phone.invoke("update_install").unwrap();
    assert!(phone.host.calls().contains(&HostCall::OpenUrl("https://play.google.com/store/apps/details?id=dev.voltip.mobile".into())));
    assert_eq!(phone.invoke("update_status").unwrap()["state"], "store");
    // Neither the check nor 自动检查更新 asked GitHub.
    assert_eq!(requests(&gh, &server), 0);
}

#[test]
fn automatic_checks_follow_the_setting() {
    let (gh, server) = github("v0.0.52");
    let phone = Phone::start(None, Settings { auto_update: true, ..Settings::default() }, &server);
    assert_eq!(phone.wait_status(|s| s["state"] == "available")["version"], "0.0.52");
    assert_eq!(requests(&gh, &server), 1);
    drop(phone);
    // Off (the default): nothing is asked until the person turns it on.
    let (gh, server) = github("v0.0.52");
    let phone = Phone::start(None, Settings::default(), &server);
    phone.wait_source().unwrap_err();
    assert_eq!(phone.invoke("update_status").unwrap()["state"], "idle");
    assert_eq!(requests(&gh, &server), 0);
    phone.shell.invoke_blocking("settings_set_auto_update", json!({ "enabled": true })).unwrap();
    phone.wait_status(|s| s["state"] == "available");
    assert_eq!(requests(&gh, &server), 1);
}
