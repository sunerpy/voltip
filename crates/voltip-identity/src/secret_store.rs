//! Abstraction over where the private key lives.

use std::collections::HashMap;

use parking_lot::Mutex;
use zeroize::Zeroizing;

use crate::IdentityError;

/// Entry name under which the identity secret is stored.
pub const SECRET_KEY_ENTRY: &str = "voltip.identity.x25519";

/// Byte-oriented secret storage. Implementations must never log values.
pub trait SecretStore: Send + Sync {
    /// Read an entry; `Ok(None)` when absent.
    fn get(&self, entry: &str) -> Result<Option<Zeroizing<Vec<u8>>>, IdentityError>;
    /// Write (or overwrite) an entry.
    fn set(&self, entry: &str, value: &[u8]) -> Result<(), IdentityError>;
    /// Remove an entry; removing a missing entry is not an error.
    fn delete(&self, entry: &str) -> Result<(), IdentityError>;
    /// Human-readable backend name for diagnostics (`keychain`, `credential-manager`, `memory`).
    fn backend_name(&self) -> &'static str;
}

/// In-memory store for tests and for the relay binary (which has no identity of its own).
#[derive(Default, Debug)]
pub struct MemorySecretStore {
    entries: Mutex<HashMap<String, Zeroizing<Vec<u8>>>>,
    fail: Mutex<bool>,
}

impl MemorySecretStore {
    /// Empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Make every operation fail with `StoreUnavailable` — simulates a locked keychain.
    pub fn set_unavailable(&self, unavailable: bool) {
        *self.fail.lock() = unavailable;
    }

    fn check(&self) -> Result<(), IdentityError> {
        if *self.fail.lock() {
            return Err(IdentityError::StoreUnavailable("memory store forced unavailable".into()));
        }
        Ok(())
    }
}

impl SecretStore for MemorySecretStore {
    fn get(&self, entry: &str) -> Result<Option<Zeroizing<Vec<u8>>>, IdentityError> {
        self.check()?;
        Ok(self.entries.lock().get(entry).cloned())
    }

    fn set(&self, entry: &str, value: &[u8]) -> Result<(), IdentityError> {
        self.check()?;
        self.entries.lock().insert(entry.to_owned(), Zeroizing::new(value.to_vec()));
        Ok(())
    }

    fn delete(&self, entry: &str) -> Result<(), IdentityError> {
        self.check()?;
        self.entries.lock().remove(entry);
        Ok(())
    }

    fn backend_name(&self) -> &'static str {
        "memory"
    }
}

/// Platform secret storage via the `keyring` crate.
///
/// Excluded from the coverage gate on purpose: it can only execute where a real keychain
/// exists, and CI runners have none unlocked.
#[cfg(feature = "keyring")]
#[derive(Debug, Clone)]
pub struct KeyringSecretStore {
    service: String,
    user: String,
}

#[cfg(feature = "keyring")]
impl KeyringSecretStore {
    /// `service` is the OS-visible application id (`dev.voltip.desktop`), `user` scopes
    /// several profiles on one machine (usually the OS user name).
    pub fn new(service: impl Into<String>, user: impl Into<String>) -> Self {
        Self { service: service.into(), user: user.into() }
    }

    fn entry(&self, entry: &str) -> Result<keyring::Entry, IdentityError> {
        keyring::Entry::new(&format!("{}/{entry}", self.service), &self.user).map_err(|e| IdentityError::StoreUnavailable(e.to_string()))
    }

    fn backend() -> &'static str {
        if cfg!(target_os = "macos") {
            "keychain"
        } else if cfg!(target_os = "windows") {
            "credential-manager"
        } else {
            "secret-service"
        }
    }
}

#[cfg(feature = "keyring")]
impl SecretStore for KeyringSecretStore {
    fn get(&self, entry: &str) -> Result<Option<Zeroizing<Vec<u8>>>, IdentityError> {
        match self.entry(entry)?.get_secret() {
            Ok(bytes) => Ok(Some(Zeroizing::new(bytes))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(IdentityError::StoreUnavailable(e.to_string())),
        }
    }

    fn set(&self, entry: &str, value: &[u8]) -> Result<(), IdentityError> {
        self.entry(entry)?.set_secret(value).map_err(|e| IdentityError::StoreUnavailable(e.to_string()))
    }

    fn delete(&self, entry: &str) -> Result<(), IdentityError> {
        match self.entry(entry)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(IdentityError::StoreUnavailable(e.to_string())),
        }
    }

    fn backend_name(&self) -> &'static str {
        Self::backend()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_roundtrip_and_delete() {
        let s = MemorySecretStore::new();
        assert_eq!(s.backend_name(), "memory");
        assert!(s.get("k").unwrap().is_none());
        s.set("k", b"v").unwrap();
        assert_eq!(s.get("k").unwrap().unwrap().as_slice(), b"v");
        s.set("k", b"w").unwrap();
        assert_eq!(s.get("k").unwrap().unwrap().as_slice(), b"w");
        s.delete("k").unwrap();
        s.delete("k").unwrap();
        assert!(s.get("k").unwrap().is_none());
    }

    #[test]
    fn memory_store_can_simulate_a_locked_keychain() {
        let s = MemorySecretStore::new();
        s.set_unavailable(true);
        assert!(matches!(s.get("k").unwrap_err(), IdentityError::StoreUnavailable(_)));
        assert!(matches!(s.set("k", b"v").unwrap_err(), IdentityError::StoreUnavailable(_)));
        assert!(matches!(s.delete("k").unwrap_err(), IdentityError::StoreUnavailable(_)));
        s.set_unavailable(false);
        assert!(s.get("k").unwrap().is_none());
        assert!(format!("{s:?}").contains("MemorySecretStore"));
    }
}

/// Android Keystore-backed store: secrets live in `SharedPreferences`, encrypted with an AES
/// key generated inside the hardware-backed Keystore, via `android-native-keyring-store`.
/// Tauri Mobile initialises `ndk-context` before Rust runs, which is all the store needs.
///
/// Compiled only for `target_os = "android"`; on other targets the type exists but `new`
/// reports `StoreUnavailable`, so the mobile shell can share one code path.
#[cfg(feature = "android-keystore")]
#[derive(Debug, Clone)]
pub struct AndroidKeystoreSecretStore {
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    service: String,
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    user: String,
}

#[cfg(feature = "android-keystore")]
impl AndroidKeystoreSecretStore {
    /// Install the Android store as the process-wide keyring default and return a handle.
    pub fn new(service: impl Into<String>, user: impl Into<String>) -> Result<Self, IdentityError> {
        #[cfg(target_os = "android")]
        {
            let store = android_native_keyring_store::Store::new().map_err(|e| IdentityError::StoreUnavailable(e.to_string()))?;
            keyring_core::set_default_store(store);
            Ok(Self { service: service.into(), user: user.into() })
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = (service.into(), user.into());
            Err(IdentityError::StoreUnavailable("Android Keystore is only available on Android".into()))
        }
    }

    #[cfg(target_os = "android")]
    fn entry(&self, entry: &str) -> Result<keyring_core::Entry, IdentityError> {
        keyring_core::Entry::new(&format!("{}/{entry}", self.service), &self.user).map_err(|e| IdentityError::StoreUnavailable(e.to_string()))
    }
}

#[cfg(all(feature = "android-keystore", target_os = "android"))]
impl SecretStore for AndroidKeystoreSecretStore {
    fn get(&self, entry: &str) -> Result<Option<Zeroizing<Vec<u8>>>, IdentityError> {
        match self.entry(entry)?.get_secret() {
            Ok(bytes) => Ok(Some(Zeroizing::new(bytes))),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(IdentityError::StoreUnavailable(e.to_string())),
        }
    }

    fn set(&self, entry: &str, value: &[u8]) -> Result<(), IdentityError> {
        self.entry(entry)?.set_secret(value).map_err(|e| IdentityError::StoreUnavailable(e.to_string()))
    }

    fn delete(&self, entry: &str) -> Result<(), IdentityError> {
        match self.entry(entry)?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(e) => Err(IdentityError::StoreUnavailable(e.to_string())),
        }
    }

    fn backend_name(&self) -> &'static str {
        "android-keystore"
    }
}

#[cfg(all(feature = "android-keystore", not(target_os = "android")))]
impl SecretStore for AndroidKeystoreSecretStore {
    fn get(&self, _entry: &str) -> Result<Option<Zeroizing<Vec<u8>>>, IdentityError> {
        Err(IdentityError::StoreUnavailable("not android".into()))
    }

    fn set(&self, _entry: &str, _value: &[u8]) -> Result<(), IdentityError> {
        Err(IdentityError::StoreUnavailable("not android".into()))
    }

    fn delete(&self, _entry: &str) -> Result<(), IdentityError> {
        Err(IdentityError::StoreUnavailable("not android".into()))
    }

    fn backend_name(&self) -> &'static str {
        "android-keystore"
    }
}

#[cfg(all(test, feature = "android-keystore", not(target_os = "android")))]
mod android_tests {
    use super::*;

    #[test]
    fn android_store_is_unavailable_off_android() {
        assert!(matches!(AndroidKeystoreSecretStore::new("svc", "u").unwrap_err(), IdentityError::StoreUnavailable(_)));
    }
}
