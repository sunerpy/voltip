//! The app's way into the shell (docs/mobile-rn.md §2), one interface for every platform: UniFFI
//! generates the Kotlin bindings (and, for an iOS app, the Swift ones) from the definitions here.
//! The app starts a [`VoltipShell`] with a [`PlatformHost`] of its own, sends commands through
//! [`VoltipShell::invoke`] (a coroutine in Kotlin, `async` in Swift), and receives the core's
//! events and the platform requests through the host.
//!
//! No hand-written FFI: UniFFI writes the scaffolding, so the crate keeps the workspace's
//! `unsafe_code = "forbid"`, and a panic in a call reaches the app as an error instead of unwinding
//! into it.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError, RwLock};

use serde_json::Value;
use tokio::runtime::{Handle, Runtime};
use voltip_core::CoreConfig;
use voltip_core::dictation::DictationPorts;
use voltip_identity::SecretStore;

use crate::host::Host;
use crate::shell::{Shell, phone_config, phone_ports};

/// What only the platform can do for the shell: hand the app the core's events and the level
/// meter's frames, and reach the clipboard, the share sheet, Wi-Fi multicast and the browser.
/// `VoltipHost.kt` implements it; Rust calls it from its own threads, never the main thread, and a
/// call may wait for the main thread.
#[uniffi::export(foreign)]
pub trait PlatformHost: Send + Sync {
    /// One `UiEvent` as JSON (`voltip://event`).
    fn event(&self, json: String) -> Result<(), HostError>;
    /// One frame of the stream `channel` (the level meter's `onFrame`) as JSON.
    fn channel(&self, channel: u64, json: String) -> Result<(), HostError>;
    /// The clipboard's text; `None` when it holds none.
    fn clipboard_read(&self) -> Result<Option<String>, HostError>;
    /// Put `text` on the clipboard.
    fn clipboard_write(&self, text: String) -> Result<(), HostError>;
    /// The system share sheet with `text`.
    fn share_text(&self, text: String) -> Result<(), HostError>;
    /// The system share sheet with `text` as the file `name` of type `mime`.
    fn share_file(&self, name: String, text: String, mime: String) -> Result<(), HostError>;
    /// Hold (`true`) or release the Wi-Fi multicast lock LAN discovery needs.
    fn multicast(&self, held: bool) -> Result<(), HostError>;
    /// Open `url` in the browser (or the app that handles it).
    fn open_url(&self, url: String) -> Result<(), HostError>;
}

/// Why the platform could not do what the shell asked.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum HostError {
    /// The platform's own reason.
    #[error("{reason}")]
    Failed {
        /// What the platform said.
        reason: String,
    },
}

impl From<uniffi::UnexpectedUniFFICallbackError> for HostError {
    fn from(e: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Failed { reason: e.reason }
    }
}

/// Why the shell did not start, or why a command failed. The text is what the Tauri shell's
/// commands answer with, so the app's labels read it unchanged.
#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum ShellError {
    /// The shell could not start.
    #[error("{0}")]
    Start(String),
    /// The command failed.
    #[error("{0}")]
    Command(String),
}

/// The platform's host as the shell's [`Host`]. A reloaded app starts again with a new one, which
/// takes the old one's place while the core keeps running.
struct ForeignHost {
    target: RwLock<Arc<dyn PlatformHost>>,
}

impl ForeignHost {
    fn current(&self) -> Arc<dyn PlatformHost> {
        self.target.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn replace(&self, host: Arc<dyn PlatformHost>) {
        *self.target.write().unwrap_or_else(PoisonError::into_inner) = host;
    }
}

impl Host for ForeignHost {
    fn event(&self, json: &str) {
        if let Err(e) = self.current().event(json.to_owned()) {
            tracing::warn!(error = %e, "an event did not reach the app");
        }
    }

    fn channel(&self, channel: u64, json: &str) {
        if let Err(e) = self.current().channel(channel, json.to_owned()) {
            tracing::warn!(error = %e, channel, "a stream frame did not reach the app");
        }
    }

    fn clipboard_read(&self) -> Result<Option<String>, String> {
        self.current().clipboard_read().map_err(|e| e.to_string())
    }

    fn clipboard_write(&self, text: &str) -> Result<(), String> {
        self.current().clipboard_write(text.to_owned()).map_err(|e| e.to_string())
    }

    fn share_text(&self, text: &str) -> Result<(), String> {
        self.current().share_text(text.to_owned()).map_err(|e| e.to_string())
    }

    fn share_file(&self, name: &str, text: &str, mime: &str) -> Result<(), String> {
        self.current().share_file(name.to_owned(), text.to_owned(), mime.to_owned()).map_err(|e| e.to_string())
    }

    fn multicast(&self, held: bool) {
        if let Err(e) = self.current().multicast(held) {
            tracing::warn!(error = %e, held, "multicast: the lock call failed; LAN discovery may hear nothing");
        }
    }

    fn open_url(&self, url: &str) -> Result<(), String> {
        self.current().open_url(url.to_owned()).map_err(|e| e.to_string())
    }
}

/// The running shell, as the app holds it.
#[derive(uniffi::Object)]
pub struct VoltipShell {
    shell: Shell,
    host: Arc<ForeignHost>,
}

impl std::fmt::Debug for VoltipShell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoltipShell").finish_non_exhaustive()
    }
}

#[uniffi::export]
impl VoltipShell {
    /// Start the shell for `data_dir` (the app's own files) and `app_version`, with `host` for the
    /// events and the platform requests. Once per process: a later start (the app's JavaScript
    /// reloaded) hands back the running shell with `host` in place of the old one.
    #[uniffi::constructor]
    pub fn start(data_dir: String, app_version: String, host: Arc<dyn PlatformHost>) -> Result<Arc<Self>, ShellError> {
        log_panics();
        RUNNING
            .get_or_start(host, |host| {
                let store = platform_store()?;
                let data_dir = PathBuf::from(data_dir);
                // An update from the Tauri phone app (0.0.49 and earlier): its files first.
                if let Some(root) = crate::legacy::app_root_of(&data_dir) {
                    crate::legacy::adopt_tauri_data(root, &data_dir);
                }
                let config = phone_config(data_dir, &app_version);
                let shell = Self::start_in(runtime()?.handle().clone(), config, store, phone_ports, host)?;
                tracing::info!(version = %app_version, "voltip shell started");
                Ok(shell)
            })
            .inspect_err(|e| tracing::error!(error = %e, "voltip shell did not start"))
    }

    /// Run `command` with `args` (a JSON object; empty for none): its answer as JSON, or the error
    /// text the Tauri shell's command gives.
    pub async fn invoke(&self, command: String, args: String) -> Result<String, ShellError> {
        let args = match args.trim() {
            "" => Value::Null,
            text => serde_json::from_str(text).map_err(|e| ShellError::Command(format!("invalid args json: {e}")))?,
        };
        // The command runs on the shell's runtime; this future only waits for its answer, so the
        // app's own executor drives it (a coroutine on Android).
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.shell.invoke(command.clone(), args, move |answer| {
            let _ = tx.send(answer);
        });
        match rx.await {
            Ok(answer) => answer.map(|value| value.to_string()).map_err(ShellError::Command),
            Err(_) => Err(ShellError::Command(format!("{command}: the shell stopped before it answered"))),
        }
    }
}

impl VoltipShell {
    /// Start a shell on `runtime` with `config`, `store`, and the dictation ports `ports` builds
    /// for the host. [`VoltipShell::start`] goes through here with the platform's own parts; tests
    /// start theirs here with fakes.
    pub fn start_in(
        runtime: Handle,
        config: CoreConfig,
        store: Arc<dyn SecretStore>,
        ports: impl FnOnce(Arc<dyn Host>) -> DictationPorts,
        host: Arc<dyn PlatformHost>,
    ) -> Result<Arc<Self>, ShellError> {
        let foreign = Arc::new(ForeignHost { target: RwLock::new(host) });
        let as_host: Arc<dyn Host> = foreign.clone();
        let shell = Shell::start(runtime, config, store, ports(as_host.clone()), as_host).map_err(ShellError::Start)?;
        Ok(Arc::new(Self { shell, host: foreign }))
    }

    /// The shell behind the interface (tests, and the platform layer).
    pub fn shell(&self) -> &Shell {
        &self.shell
    }
}

/// The process's shell: started once, handed back to every later start.
struct Running(Mutex<Option<Arc<VoltipShell>>>);

impl Running {
    fn get_or_start(
        &self,
        host: Arc<dyn PlatformHost>,
        start: impl FnOnce(Arc<dyn PlatformHost>) -> Result<Arc<VoltipShell>, ShellError>,
    ) -> Result<Arc<VoltipShell>, ShellError> {
        let mut running = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(shell) = running.as_ref() {
            tracing::info!("the shell is already running; the app reattached");
            shell.host.replace(host);
            return Ok(shell.clone());
        }
        let shell = start(host)?;
        *running = Some(shell.clone());
        Ok(shell)
    }
}

static RUNNING: Running = Running(Mutex::new(None));

/// The shell's runtime, built on the first start.
fn runtime() -> Result<&'static Runtime, ShellError> {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    if let Some(runtime) = RUNTIME.get() {
        return Ok(runtime);
    }
    let runtime =
        tokio::runtime::Builder::new_multi_thread().enable_all().thread_name("voltip-rn").build().map_err(|e| ShellError::Start(format!("runtime: {e}")))?;
    Ok(RUNTIME.get_or_init(|| runtime))
}

/// The Keystore, and nothing less secure: a phone without it does not start.
#[cfg(target_os = "android")]
fn platform_store() -> Result<Arc<dyn SecretStore>, ShellError> {
    let store = voltip_identity::AndroidKeystoreSecretStore::new(crate::shell::KEYSTORE_SERVICE, "voltip")
        .map_err(|e| ShellError::Start(format!("Android Keystore unavailable: {e}")))?;
    Ok(Arc::new(store))
}

/// No secure store on this platform yet (iOS's Keychain comes with an iOS app): the shell refuses
/// to start rather than keep the identity anywhere less safe.
#[cfg(not(target_os = "android"))]
fn platform_store() -> Result<Arc<dyn SecretStore>, ShellError> {
    Err(ShellError::Start("no secure secret store on this platform".to_owned()))
}

/// Panics go to the log (logcat on Android, through the subscriber `KeyringLog.setLog` installed)
/// before the default hook runs.
fn log_panics() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            tracing::error!(panic = %info, "voltip panicked");
            previous(info);
        }));
    });
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Quiet;

    impl PlatformHost for Quiet {
        fn event(&self, _json: String) -> Result<(), HostError> {
            Ok(())
        }
        fn channel(&self, _channel: u64, _json: String) -> Result<(), HostError> {
            Ok(())
        }
        fn clipboard_read(&self) -> Result<Option<String>, HostError> {
            Ok(None)
        }
        fn clipboard_write(&self, _text: String) -> Result<(), HostError> {
            Ok(())
        }
        fn share_text(&self, _text: String) -> Result<(), HostError> {
            Ok(())
        }
        fn share_file(&self, _name: String, _text: String, _mime: String) -> Result<(), HostError> {
            Ok(())
        }
        fn multicast(&self, _held: bool) -> Result<(), HostError> {
            Ok(())
        }
        fn open_url(&self, _url: String) -> Result<(), HostError> {
            Ok(())
        }
    }

    /// A start that cannot happen leaves nothing behind: the next start tries again.
    #[test]
    fn a_failed_start_is_not_remembered() {
        let running = Running(Mutex::new(None));
        let err = running.get_or_start(Arc::new(Quiet), |_| Err(ShellError::Start("no store".into()))).unwrap_err();
        assert_eq!(err.to_string(), "no store");
        assert!(running.0.lock().unwrap().is_none());
    }

    /// Off a phone there is no secure store, so the app's own start refuses (tests start theirs
    /// with `start_in`).
    #[test]
    fn the_apps_start_refuses_without_a_secure_store() {
        let dir = tempfile::tempdir().unwrap();
        let err = VoltipShell::start(dir.path().display().to_string(), "0.0.44".into(), Arc::new(Quiet)).unwrap_err();
        assert!(matches!(&err, ShellError::Start(reason) if reason == "no secure secret store on this platform"), "{err:?}");
    }

    #[test]
    fn unexpected_platform_errors_become_the_hosts_reason() {
        let e: HostError = uniffi::UnexpectedUniFFICallbackError::new("kotlin.IllegalStateException: gone").into();
        assert_eq!(e.to_string(), "kotlin.IllegalStateException: gone");
    }
}
