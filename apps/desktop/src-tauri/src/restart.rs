//! Restarting once an update has installed itself. On macOS a release keeps its secrets in keychain
//! items of its own, so it starts the new build itself and hands them over
//! ([`crate::keychain_handoff`]); everywhere else, and for builds whose store has nothing to hand
//! over, Tauri restarts the app.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Manager, Runtime};
use voltip_identity::SecretStore;

/// Managed state: the secret store the core uses, and whether an update asked for the hand-over.
pub struct Restart {
    store: Arc<dyn SecretStore>,
    handing_over: AtomicBool,
}

impl Restart {
    /// State for `store` (the one the core was started with).
    pub fn new(store: Arc<dyn SecretStore>) -> Self {
        Self { store, handing_over: AtomicBool::new(false) }
    }

    /// Whether leaving now should hand the secrets to the new build.
    pub fn handing_over(&self) -> bool {
        self.handing_over.load(Ordering::SeqCst)
    }
}

impl std::fmt::Debug for Restart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Restart").field("backend", &self.store.backend_name()).field("handing_over", &self.handing_over()).finish()
    }
}

/// The update is installed: leave and start the new build, handing the secrets over when the store
/// has any to hand (macOS releases), or let Tauri restart the app.
pub fn after_update<R: Runtime>(app: &AppHandle<R>) {
    if let Some(restart) = app.try_state::<Restart>()
        && cfg!(target_os = "macos")
        && restart.store.handoff_state().is_some()
    {
        restart.handing_over.store(true, Ordering::SeqCst);
        app.exit(0);
        return;
    }
    app.request_restart();
}

/// `RunEvent::Exit`: start the new build with the hand-over when [`after_update`] asked for it
/// (does not return then). Only macOS asks.
#[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
pub fn on_exit<R: Runtime>(app: &AppHandle<R>) {
    #[cfg(target_os = "macos")]
    if let Some(restart) = app.try_state::<Restart>()
        && restart.handing_over()
    {
        let entries = restart.store.handoff_state().unwrap_or_default();
        relaunch(app, &entries, crate::keychain_handoff::start_new_build);
    }
}

/// Clean up as Tauri does before a restart, start the new build with `start`, and leave. A new
/// build that could not be started at all is restarted by Tauri instead. Never returns.
#[cfg(unix)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn relaunch<R: Runtime>(
    app: &AppHandle<R>,
    entries: &voltip_identity::Entries,
    start: impl FnOnce(&voltip_identity::Entries) -> Result<(), voltip_identity::handoff::StartError>,
) -> ! {
    use voltip_identity::handoff::StartError;
    app.cleanup_before_exit();
    match start(entries) {
        Ok(()) => tracing::info!(entries = entries.len(), "the new build started with the keychain hand-over"),
        Err(StartError::WithoutHandOver(e)) => tracing::warn!(error = %e, "the new build started without the keychain hand-over"),
        Err(StartError::NotStarted(e)) => {
            tracing::warn!(error = %e, "could not start the new build with the keychain hand-over; restarting without it");
            tauri::process::restart(&app.env());
        }
    }
    crate::exit::exit_process(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use voltip_identity::MemorySecretStore;

    /// A store every build reads alike (Windows, Linux, the debug store) has nothing to hand over,
    /// so an update restarts through Tauri as before.
    #[test]
    fn a_store_without_a_hand_over_restarts_through_tauri() {
        let restart = Restart::new(Arc::new(MemorySecretStore::new()));
        assert!(restart.store.handoff_state().is_none());
        assert!(!restart.handing_over());
        assert!(format!("{restart:?}").contains("memory"));
    }
}
