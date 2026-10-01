//! Restarting once an update has installed itself. macOS persists the hand-over in the staged new
//! build before installation ([`crate::keychain_handoff`]); after installation every platform can
//! use Tauri's normal restart.

use std::sync::Arc;
use tauri::{AppHandle, Runtime};
use voltip_identity::{Entries, SecretStore};

/// Managed state: the secret store the core uses, including what a macOS update stages.
pub struct Restart {
    store: Arc<dyn SecretStore>,
}

impl Restart {
    /// State for `store` (the one the core was started with).
    pub fn new(store: Arc<dyn SecretStore>) -> Self {
        Self { store }
    }

    /// What a staged macOS build must persist before installation; `None` on stores shared by all
    /// builds (Windows, Linux and debug runs).
    pub fn handoff_state(&self) -> Option<Entries> {
        self.store.handoff_state()
    }
}

impl std::fmt::Debug for Restart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Restart").field("backend", &self.store.backend_name()).finish()
    }
}

/// The update is installed and the staged macOS build has already persisted its secrets.
pub fn after_update<R: Runtime>(app: &AppHandle<R>) {
    app.request_restart();
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
        assert!(restart.handoff_state().is_none());
        assert!(format!("{restart:?}").contains("memory"));
    }
}
