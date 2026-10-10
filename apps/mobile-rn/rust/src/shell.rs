//! The shell (docs/mobile-rn.md §3): the core behind the bridge, the pump that hands every
//! `UiEvent` to the [`Host`], and the state the phone's commands keep beside the core (the level
//! meter subscriptions, the staged feedback attachments).

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;
use tokio::runtime::Handle;
use tokio::sync::broadcast::error::RecvError;
use voltip_cloud::feedback;
use voltip_core::CoreConfig;
use voltip_core::dictation::DictationPorts;
use voltip_core::ui::UiEvent;
use voltip_identity::SecretStore;
use voltip_tauri_bridge::{Bridge, UiCommand};

use crate::host::Host;
use crate::meter::Meters;
use crate::update::{PhoneUpdater, UpdateConfig};

/// Keystore service id, the Tauri phone shell's. Since 0.0.50 this app is the Android app under that
/// app's package (user decision 2026-10-09), so an update from it finds the device identity and the
/// provider keys where that app left them (`legacy` moves its files).
pub const KEYSTORE_SERVICE: &str = "dev.voltip.mobile";

/// The running shell; cheap to clone.
#[derive(Clone)]
pub struct Shell {
    pub(crate) inner: Arc<Inner>,
}

impl std::fmt::Debug for Shell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shell").finish_non_exhaustive()
    }
}

pub(crate) struct Inner {
    pub(crate) runtime: Handle,
    pub(crate) bridge: Bridge,
    pub(crate) host: Arc<dyn Host>,
    pub(crate) meters: Meters,
    pub(crate) attachments: feedback::Attachments,
    pub(crate) updater: Arc<PhoneUpdater>,
}

/// The phone's core configuration, the Tauri phone shell's (`production_config`): `<platform> 手机`
/// as the first name, the app version for the client string, no phone takes received, no live
/// preview, scenes picked by hand, and the phone's side of the sync.
pub fn phone_config(data_dir: PathBuf, app_version: &str) -> CoreConfig {
    let mut config = CoreConfig::new(data_dir);
    config.default_device_name = format!("{} 手机", platform_label());
    config.client_version = format!("voltip/{app_version}");
    config.app_version = app_version.to_owned();
    // The phone is the microphone; a desktop records its takes, never the other way round.
    config.accepts_phone_takes = false;
    // The phone shows no live preview: the built-in service sends none (docs/dictation.md §11.8).
    config.shows_live_preview = false;
    // The user picks a take's scene on the talk card (no foreground probe on a phone).
    config.manual_scenes = true;
    // Copies of the computers' histories and settings, read-only (docs/dictation.md §20.8).
    config.sync_role = voltip_core::sync::SyncRole::Phone;
    config
}

/// The phone's dictation ports: its microphone, the cloud clients of the resolved engines (the
/// built-in services unless the settings name others; no local models on a phone), the clipboard
/// for the result, and the HTTP provider probe (测试连接). No live preview, no foreground probe, no
/// VAD: the core's fallbacks apply. The Tauri phone shell's `phone_ports`.
pub fn phone_ports(host: Arc<dyn Host>) -> DictationPorts {
    DictationPorts {
        audio: Arc::new(crate::microphone::PhoneMicrophone::cpal()),
        injector: Arc::new(crate::clipboard::ClipboardInjector(host)),
        factory: Arc::new(|engines: &voltip_core::ResolvedEngines| (voltip_cloud::remote_transcriber(engines), voltip_cloud::refiner(engines))),
        models: None,
        streaming: None,
        probe: None,
        service_probe: Some(Arc::new(voltip_cloud::HttpServiceProbe)),
        segmenter: None,
    }
}

impl Shell {
    /// Start the core on `runtime` with `ports`, hand every `UiEvent` to `host`, and start the
    /// updater on the repository's releases (docs/dictation.md §20.9).
    pub fn start(runtime: Handle, config: CoreConfig, store: Arc<dyn SecretStore>, ports: DictationPorts, host: Arc<dyn Host>) -> Result<Self, String> {
        Self::start_with_updates(runtime, config, store, ports, host, UpdateConfig::production())
    }

    /// [`Shell::start`] with `updates` for where updates come from (tests point it elsewhere).
    pub fn start_with_updates(
        runtime: Handle,
        config: CoreConfig,
        store: Arc<dyn SecretStore>,
        ports: DictationPorts,
        host: Arc<dyn Host>,
        updates: UpdateConfig,
    ) -> Result<Self, String> {
        let updater = Arc::new(PhoneUpdater::new(updates, &config.app_version));
        let started = {
            // The core spawns its tasks with `tokio::spawn`: start it inside the runtime.
            let _entered = runtime.enter();
            Bridge::start_subscribed(config, store, ports)
        };
        let (bridge, mut events) = started.map_err(String::from)?;
        let pump = host.clone();
        runtime.spawn(async move {
            loop {
                match events.recv().await {
                    Ok(ev) => match serde_json::to_string(&ev) {
                        // Why a take ended without text, by its code only: the message can carry
                        // an endpoint, which never goes into a log line (AGENTS.md).
                        Ok(json) => {
                            if let Some(code) = failed_take(&ev) {
                                tracing::warn!(?code, "take failed");
                            }
                            pump.event(&json);
                        }
                        Err(e) => tracing::warn!(error = %e, "event does not serialize; dropped"),
                    },
                    Err(RecvError::Lagged(n)) => tracing::warn!(skipped = n, "the app lagged behind the events"),
                    Err(RecvError::Closed) => break,
                }
            }
        });
        bridge.publish(UiEvent::Update(updater.status()));
        crate::update::start(&runtime, bridge.clone(), updater.clone(), host.clone());
        Ok(Self { inner: Arc::new(Inner { runtime, bridge, host, meters: Meters::default(), attachments: feedback::Attachments::default(), updater }) })
    }

    /// The bridge, for the platform layer and tests.
    pub fn bridge(&self) -> &Bridge {
        &self.inner.bridge
    }

    /// Run `command` with `args` (an object, or `null` for none) and hand the answer to `done`:
    /// JSON on success, the error text otherwise, as the Tauri shell's commands answer.
    pub fn invoke(&self, command: String, args: Value, done: impl FnOnce(Result<Value, String>) + Send + 'static) {
        let shell = self.clone();
        self.inner.runtime.spawn(async move {
            done(crate::commands::run(&shell, &command, args).await);
        });
    }

    /// [`Shell::invoke`], waiting for the answer (tests, and callers outside the runtime).
    pub fn invoke_blocking(&self, command: &str, args: Value) -> Result<Value, String> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.invoke(command.to_owned(), args, move |answer| {
            let _ = tx.send(answer);
        });
        rx.recv().map_err(|_| format!("{command}: the shell stopped before it answered"))?
    }

    /// The network changed or the app came to the front (docs/pairing.md 「重连」): the relay link
    /// checks its socket now. Never waits.
    pub fn reconnect_relay(&self) {
        if let Err(e) = self.inner.bridge.dispatch(UiCommand::RelayReconnect) {
            tracing::warn!(error = %e, "the relay check was not passed on");
        }
    }

    /// Stop the core.
    pub fn shutdown(&self) {
        self.inner.bridge.shutdown();
    }
}

/// The failure code of a take that just failed, from its dictation event.
fn failed_take(event: &UiEvent) -> Option<voltip_core::dictation::FailureCode> {
    match event {
        UiEvent::Dictation(status) => match &status.phase {
            voltip_core::dictation::DictationPhase::Failed { code, .. } => Some(*code),
            _ => None,
        },
        _ => None,
    }
}

/// Human-readable platform for the default device name.
pub fn platform_label() -> &'static str {
    if cfg!(target_os = "android") { "Android" } else { "Voltip" }
}
