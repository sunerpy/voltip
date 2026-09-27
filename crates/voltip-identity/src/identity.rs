//! The device's own long-term identity.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use voltip_crypto::{PublicKey, SecretKey, StaticKeypair};
use voltip_protocol::{DeviceId, DeviceInfo, Platform};

use crate::{IdentityError, SECRET_KEY_ENTRY, SecretStore};

/// Entry name for the public metadata (id, name, platform) — not secret, but kept beside
/// the key so both halves come from one place.
const META_ENTRY: &str = "voltip.identity.meta";

/// A device identity with its private key loaded.
#[derive(Clone, Debug)]
pub struct DeviceIdentity {
    /// Stable id.
    pub device_id: DeviceId,
    /// User-visible name.
    pub name: String,
    /// Platform.
    pub platform: Platform,
    /// Long-term keypair.
    pub keypair: StaticKeypair,
}

/// What the identity looks like to everyone else — the only serializable form.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct DeviceIdentityPublic {
    /// Stable id.
    pub device_id: DeviceId,
    /// User-visible name.
    pub name: String,
    /// Platform.
    pub platform: Platform,
    /// Long-term public key.
    pub public_key: PublicKey,
    /// `SHA-256(public_key)` rendered as `A7:C4:19:8E · 3D:F2:61:09`.
    pub fingerprint: String,
}

impl DeviceIdentity {
    /// Public view.
    pub fn public(&self) -> DeviceIdentityPublic {
        DeviceIdentityPublic {
            device_id: self.device_id,
            name: self.name.clone(),
            platform: self.platform,
            public_key: self.keypair.public,
            fingerprint: self.keypair.public.fingerprint(),
        }
    }

    /// Protocol-level description sent to peers after the E2EE channel is up.
    pub fn info(&self) -> DeviceInfo {
        DeviceInfo { device_id: self.device_id, name: self.name.clone(), platform: self.platform }
    }
}

#[derive(Serialize, Deserialize)]
struct StoredMeta {
    device_id: DeviceId,
    name: String,
    platform: Platform,
}

/// Loads or creates the identity from a [`SecretStore`].
pub struct IdentityManager {
    store: Arc<dyn SecretStore>,
}

impl std::fmt::Debug for IdentityManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IdentityManager").field("backend", &self.store.backend_name()).finish()
    }
}

impl IdentityManager {
    /// Wrap a store.
    pub fn new(store: Arc<dyn SecretStore>) -> Self {
        Self { store }
    }

    /// The underlying store's backend name.
    pub fn backend_name(&self) -> &'static str {
        self.store.backend_name()
    }

    /// Load the identity if one exists.
    pub fn load(&self) -> Result<Option<DeviceIdentity>, IdentityError> {
        let Some(secret) = self.store.get(SECRET_KEY_ENTRY)? else { return Ok(None) };
        let Some(meta) = self.store.get(META_ENTRY)? else { return Err(IdentityError::Corrupt) };
        let meta: StoredMeta = serde_json::from_slice(&meta).map_err(|_| IdentityError::Corrupt)?;
        let secret = SecretKey::from_slice(&secret).map_err(|_| IdentityError::Corrupt)?;
        let keypair = StaticKeypair::from_secret(secret)?;
        Ok(Some(DeviceIdentity { device_id: meta.device_id, name: meta.name, platform: meta.platform, keypair }))
    }

    /// Load, or create a new identity with `default_name` and persist it.
    pub fn load_or_create(&self, default_name: &str) -> Result<DeviceIdentity, IdentityError> {
        if let Some(existing) = self.load()? {
            return Ok(existing);
        }
        let keypair = StaticKeypair::generate()?;
        let identity = DeviceIdentity { device_id: DeviceId::random(), name: default_name.to_owned(), platform: Platform::current(), keypair };
        identity.info().validate()?;
        self.persist(&identity)?;
        tracing::info!(device_id = %identity.device_id, backend = self.store.backend_name(), "created device identity");
        Ok(identity)
    }

    /// Rename the device (validated) and persist.
    pub fn rename(&self, identity: &mut DeviceIdentity, name: &str) -> Result<(), IdentityError> {
        let mut candidate = identity.info();
        candidate.name = name.trim().to_owned();
        candidate.validate()?;
        identity.name = candidate.name;
        self.persist(identity)
    }

    /// Delete the identity (used by "reset this device"). Trusted devices must be cleared by
    /// the caller — they are worthless without the key anyway.
    pub fn reset(&self) -> Result<(), IdentityError> {
        self.store.delete(SECRET_KEY_ENTRY)?;
        self.store.delete(META_ENTRY)
    }

    fn persist(&self, identity: &DeviceIdentity) -> Result<(), IdentityError> {
        let meta = StoredMeta { device_id: identity.device_id, name: identity.name.clone(), platform: identity.platform };
        self.store.set(SECRET_KEY_ENTRY, identity.keypair.secret.expose())?;
        self.store.set(META_ENTRY, &serde_json::to_vec(&meta)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemorySecretStore;

    fn manager() -> (Arc<MemorySecretStore>, IdentityManager) {
        let store = Arc::new(MemorySecretStore::new());
        (store.clone(), IdentityManager::new(store))
    }

    #[test]
    fn create_then_load_is_stable() {
        let (_, m) = manager();
        assert!(m.load().unwrap().is_none());
        let created = m.load_or_create("Surface-Laptop").unwrap();
        let loaded = m.load_or_create("ignored").unwrap();
        assert_eq!(loaded.device_id, created.device_id);
        assert_eq!(loaded.name, "Surface-Laptop");
        assert_eq!(loaded.keypair.public, created.keypair.public);
        assert_eq!(loaded.platform, Platform::current());
        assert_eq!(m.backend_name(), "memory");
        assert!(format!("{m:?}").contains("memory"));
    }

    #[test]
    fn public_view_never_contains_the_secret() {
        let (store, m) = manager();
        let id = m.load_or_create("Pixel 10").unwrap();
        let json = serde_json::to_string(&id.public()).unwrap();
        let secret_hex = hex_of(id.keypair.secret.expose());
        assert!(!json.contains(&secret_hex));
        assert!(json.contains(&id.device_id.to_string()));
        assert_eq!(id.public().fingerprint, id.keypair.public.fingerprint());
        assert_eq!(id.info().name, "Pixel 10");
        // The meta entry in the store is JSON without key bytes either.
        let meta = store.get("voltip.identity.meta").unwrap().unwrap();
        assert!(!String::from_utf8_lossy(&meta).contains(&secret_hex));
        let roundtrip: DeviceIdentityPublic = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtrip, id.public());
    }

    fn hex_of(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn rename_validates_and_persists() {
        let (_, m) = manager();
        let mut id = m.load_or_create("A").unwrap();
        m.rename(&mut id, "  Studio Mac  ").unwrap();
        assert_eq!(id.name, "Studio Mac");
        assert_eq!(m.load().unwrap().unwrap().name, "Studio Mac");
        assert!(matches!(m.rename(&mut id, "").unwrap_err(), IdentityError::Protocol(_)));
        assert!(matches!(m.rename(&mut id, &"x".repeat(65)).unwrap_err(), IdentityError::Protocol(_)));
        assert_eq!(id.name, "Studio Mac", "failed rename must not mutate");
    }

    #[test]
    fn reset_removes_everything() {
        let (store, m) = manager();
        m.load_or_create("A").unwrap();
        m.reset().unwrap();
        assert!(m.load().unwrap().is_none());
        assert!(store.get(SECRET_KEY_ENTRY).unwrap().is_none());
    }

    #[test]
    fn corrupt_store_contents_are_reported() {
        let (store, m) = manager();
        store.set(SECRET_KEY_ENTRY, b"short").unwrap();
        assert!(matches!(m.load().unwrap_err(), IdentityError::Corrupt));
        store.set(SECRET_KEY_ENTRY, &[1u8; 32]).unwrap();
        assert!(matches!(m.load().unwrap_err(), IdentityError::Corrupt), "missing meta");
        store.set("voltip.identity.meta", b"{not json").unwrap();
        assert!(matches!(m.load().unwrap_err(), IdentityError::Corrupt));
    }

    #[test]
    fn unavailable_store_surfaces_as_store_unavailable() {
        let (store, m) = manager();
        store.set_unavailable(true);
        assert!(matches!(m.load().unwrap_err(), IdentityError::StoreUnavailable(_)));
        assert!(matches!(m.load_or_create("A").unwrap_err(), IdentityError::StoreUnavailable(_)));
        assert!(m.load_or_create("A").unwrap_err().to_string().contains("unavailable"));
    }

    #[test]
    fn default_name_is_validated() {
        let (_, m) = manager();
        assert!(matches!(m.load_or_create("").unwrap_err(), IdentityError::Protocol(_)));
    }
}
