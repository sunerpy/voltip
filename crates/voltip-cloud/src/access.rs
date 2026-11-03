//! The device's tickets for the shells (docs/pairing.md 「设备票据」): a build that carries the build
//! key and names `voltip-auth` (`VOLTIP_AUTH_URL`) renews a ticket for this device's usage id,
//! keeps its device secret in the system keychain, and asks over the same HTTPS client as the
//! built-in service (Mozilla's roots on Android, the proxy plan). Any other build has none.

use std::path::Path;
use std::sync::Arc;

use voltip_access::{AccessClient, AccessConfig, BuildKey, CredentialStore, DeviceSecret, HttpFactory, TicketSource};
use voltip_identity::SecretStore;
use voltip_net::Network;

/// The keychain entry of the device secret.
pub const ACCESS_ENTRY: &str = "voltip.access.device";

/// `voltip-auth` behind the build's gateway (`VOLTIP_AUTH_URL`, a build secret like the relay's).
pub const AUTH_URL: Option<&str> = option_env!("VOLTIP_AUTH_URL");

/// The device secret in the app's secret store.
pub struct SecretCredentials(pub Arc<dyn SecretStore>);

impl CredentialStore for SecretCredentials {
    fn load(&self) -> Option<DeviceSecret> {
        let bytes = self.0.get(ACCESS_ENTRY).ok()??;
        String::from_utf8(bytes.to_vec()).ok().map(DeviceSecret::new)
    }

    fn save(&self, secret: &DeviceSecret) -> Result<(), String> {
        self.0.set(ACCESS_ENTRY, secret.as_str().as_bytes()).map_err(|e| e.to_string())
    }

    fn clear(&self) {
        if let Err(e) = self.0.delete(ACCESS_ENTRY) {
            tracing::warn!(error = %e, "the device secret could not be removed");
        }
    }
}

/// This build's ticket source for the device whose usage id is under `data_dir`, keeping its
/// secret in `secrets`; `None` in a build without the build key or `voltip-auth`.
pub fn device_tickets(data_dir: &Path, secrets: Arc<dyn SecretStore>, network: &Arc<Network>, platform: &str) -> Option<Arc<dyn TicketSource>> {
    // Nothing is made, not even the usage id, in a build that has no tickets.
    AUTH_URL.map(str::trim).filter(|u| !u.is_empty())?;
    voltip_access::build_key()?;
    let device_id = match voltip_core::usage::device_id(data_dir) {
        Ok(id) => id,
        Err(e) => {
            tracing::warn!(error = %e, "no usage id; the device gets no ticket");
            return None;
        }
    };
    tickets_with(AUTH_URL, voltip_access::build_key(), device_id, Arc::new(SecretCredentials(secrets)), network, platform)
}

/// [`device_tickets`] with every part named: the device `device_id` (its usage id, the one its
/// metering names), its secret in `store` (the server's file; tests).
pub fn tickets_with(
    auth_url: Option<&str>,
    key: Option<BuildKey>,
    device_id: String,
    store: Arc<dyn CredentialStore>,
    network: &Arc<Network>,
    platform: &str,
) -> Option<Arc<dyn TicketSource>> {
    let auth_url = auth_url.map(str::trim).filter(|u| !u.is_empty())?;
    let key = key?;
    let network = network.clone();
    let http: HttpFactory = Arc::new(move || crate::http_client_builder(&network).build());
    let config = AccessConfig { auth_url: auth_url.to_owned(), device_id, platform: platform.to_owned() };
    Some(AccessClient::new(config, key, store, http))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use voltip_identity::MemorySecretStore;

    use super::*;

    #[test]
    fn a_build_without_the_key_or_the_address_has_no_tickets() {
        let store: Arc<dyn CredentialStore> = Arc::new(SecretCredentials(Arc::new(MemorySecretStore::new())));
        let net = Network::new();
        let key = BuildKey::new([1; 32], "0.3.1");
        let device = || "6f1c2d3e-0000-4000-8000-000000000001".to_owned();
        assert!(tickets_with(None, Some(key.clone()), device(), store.clone(), &net, "linux").is_none());
        assert!(tickets_with(Some("  "), Some(key.clone()), device(), store.clone(), &net, "linux").is_none());
        assert!(tickets_with(Some("https://auth.example.test/auth/v1"), None, device(), store.clone(), &net, "linux").is_none());
        let tickets = tickets_with(Some("https://auth.example.test/auth/v1"), Some(key), device(), store, &net, "linux").unwrap();
        assert!(tickets.current().is_none(), "nothing until it renews");
        // This test build has no key: the shells' helper makes nothing, not even a usage id.
        let dir = tempfile::tempdir().unwrap();
        assert!(device_tickets(dir.path(), Arc::new(MemorySecretStore::new()), &net, "linux").is_none());
        assert!(voltip_core::usage::read_device_id(dir.path()).is_none());
    }

    #[test]
    fn the_secret_lives_in_the_secret_store() {
        let secrets = Arc::new(MemorySecretStore::new());
        let store = SecretCredentials(secrets.clone());
        assert_eq!(store.load(), None);
        store.save(&DeviceSecret::new("ab".repeat(32))).unwrap();
        assert_eq!(store.load().unwrap().as_str(), "ab".repeat(32));
        assert!(secrets.get(ACCESS_ENTRY).unwrap().is_some());
        store.clear();
        assert_eq!(store.load(), None);
    }
}
