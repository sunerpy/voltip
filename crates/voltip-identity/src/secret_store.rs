//! Abstraction over where the private key lives.

use std::collections::HashMap;

use parking_lot::Mutex;
use zeroize::Zeroizing;

use crate::{Entries, IdentityError};

/// Entry name under which the identity secret is stored.
pub const SECRET_KEY_ENTRY: &str = "voltip.identity.x25519";

/// Suffix of the account under which a signed macOS build keeps the items it created
/// (`<user>.signed` in 0.0.12–0.0.14, `<user>.signed.<cdhash>` since; see [`crate::PerBuildStore`]).
pub const SIGNED_ACCOUNT_SUFFIX: &str = ".signed";

/// Byte-oriented secret storage. Implementations must never log values.
pub trait SecretStore: Send + Sync {
    /// Read an entry; `Ok(None)` when absent.
    fn get(&self, entry: &str) -> Result<Option<Zeroizing<Vec<u8>>>, IdentityError>;
    /// Read an entry with no side effect at all: no item is written, moved or removed and no
    /// system dialog is shown; an item that only a dialog would give reads as absent. A second
    /// process reading the app's secrets (the local speech service, docs/dictation.md §23) uses
    /// this. The default is [`Self::get`], right for stores whose `get` has no side effect; the
    /// keychain stores, whose `get` moves items between builds, override it.
    fn peek(&self, entry: &str) -> Result<Option<Zeroizing<Vec<u8>>>, IdentityError> {
        self.get(entry)
    }
    /// Write (or overwrite) an entry.
    fn set(&self, entry: &str, value: &[u8]) -> Result<(), IdentityError>;
    /// Remove an entry; removing a missing entry is not an error.
    fn delete(&self, entry: &str) -> Result<(), IdentityError>;
    /// Human-readable backend name for diagnostics (`keychain`, `credential-manager`, `memory`).
    fn backend_name(&self) -> &'static str;
    /// What an update should hand to the next build ([`crate::handoff`]); `None` for a store
    /// whose items every build reads alike.
    fn handoff_state(&self) -> Option<Entries> {
        None
    }
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

/// One stored secret, as moving it from one item to another needs it.
pub(crate) trait Slot {
    /// The value; `Ok(None)` when the item is absent.
    fn read(&self) -> Result<Option<Zeroizing<Vec<u8>>>, IdentityError>;
    /// Create or overwrite the item.
    fn write(&self, value: &[u8]) -> Result<(), IdentityError>;
    /// Remove the item; a missing one is not an error.
    fn remove(&self) -> Result<(), IdentityError>;
}

/// Read a secret without moving it ([`SecretStore::peek`]): a signed build reads only the item it
/// owns (an older build's item could ask, and reading it is how it would be moved), any other
/// build its usual item. Nothing is ever written or removed.
pub(crate) fn read_only(signed: bool, owned: &dyn Slot, legacy: &dyn Slot) -> Result<Option<Zeroizing<Vec<u8>>>, IdentityError> {
    if signed { owned.read() } else { legacy.read() }
}

/// Read a secret from the item this build created (`owned`), moving it there from the item an
/// older build created (`legacy`) the first time. The value is written, then read back, and only
/// then is the old item removed; when any of that fails the old item stays in use.
///
/// Why (macOS, user report 2026-09-30): the keychain trusts an item's creator by its signing
/// requirement, so an item a signed Voltip created is read by every later release without a
/// prompt. An item from ad-hoc Voltip (0.0.6 and older) trusts that one build only, and
/// 「始终允许」 in the system dialog adds the one build it was pressed for: such an item asked again
/// on every update.
pub(crate) fn read_moving(owned: &dyn Slot, legacy: &dyn Slot) -> Result<Option<Zeroizing<Vec<u8>>>, IdentityError> {
    if let Some(value) = owned.read()? {
        return Ok(Some(value));
    }
    let Some(value) = legacy.read()? else { return Ok(None) };
    match owned.write(&value).and_then(|()| owned.read()) {
        Ok(Some(back)) if back.as_slice() == value.as_slice() => {
            if let Err(e) = legacy.remove() {
                tracing::warn!(error = %e, "the older keychain item could not be removed; it stays, unused");
            }
            tracing::info!("keychain item moved to one this build owns");
        }
        outcome => {
            tracing::warn!(stored = outcome.is_ok(), "the keychain item could not be stored as this build's; the older one stays in use");
            let _ = owned.remove();
        }
    }
    Ok(Some(value))
}

/// Platform secret storage via the `keyring` crate.
///
/// Excluded from the coverage gate on purpose: it can only execute where a real keychain
/// exists, and CI runners have none unlocked. On macOS a build signed with a certificate (the
/// releases) keeps its secrets in items it created itself, under the account `<user>.signed`, and
/// moves each older item there on its first read ([`read_moving`]); an ad-hoc build (local and CI
/// builds) keeps using the older items, so it never takes them over for itself.
#[cfg(feature = "keyring")]
#[derive(Debug, Clone)]
pub struct KeyringSecretStore {
    service: String,
    user: String,
    /// This build is signed with a certificate (macOS; `false` elsewhere).
    signed: bool,
}

#[cfg(feature = "keyring")]
impl KeyringSecretStore {
    /// `service` is the OS-visible application id (`dev.voltip.desktop`), `user` scopes
    /// several profiles on one machine (usually the OS user name).
    pub fn new(service: impl Into<String>, user: impl Into<String>) -> Self {
        Self { service: service.into(), user: user.into(), signed: signed_with_a_certificate() }
    }

    fn slot(&self, entry: &str, account: &str) -> Result<KeyringSlot, IdentityError> {
        keyring::Entry::new(&format!("{}/{entry}", self.service), account).map(KeyringSlot).map_err(|e| IdentityError::StoreUnavailable(e.to_string()))
    }

    /// The item every build before this change used, and unsigned builds still use.
    fn legacy(&self, entry: &str) -> Result<KeyringSlot, IdentityError> {
        self.slot(entry, &self.user)
    }

    /// The item a signed build created itself.
    fn owned(&self, entry: &str) -> Result<KeyringSlot, IdentityError> {
        self.slot(entry, &format!("{}{SIGNED_ACCOUNT_SUFFIX}", self.user))
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

/// Whether this process is signed with a certificate, not ad hoc (macOS code signing).
#[cfg(all(feature = "keyring", target_os = "macos"))]
pub fn signed_with_a_certificate() -> bool {
    use security_framework::os::macos::code_signing::{Flags, SecCode, SecRequirement};
    let Ok(requirement) = "certificate leaf[subject.CN] exists".parse::<SecRequirement>() else { return false };
    SecCode::for_self(Flags::NONE).and_then(|code| code.check_validity(Flags::NONE, &requirement)).is_ok()
}

/// Whether this process is signed with a certificate (only macOS signs code this way).
#[cfg(all(feature = "keyring", not(target_os = "macos")))]
pub fn signed_with_a_certificate() -> bool {
    false
}

/// A `keyring` entry as a [`Slot`].
#[cfg(feature = "keyring")]
struct KeyringSlot(keyring::Entry);

#[cfg(feature = "keyring")]
impl Slot for KeyringSlot {
    fn read(&self) -> Result<Option<Zeroizing<Vec<u8>>>, IdentityError> {
        match self.0.get_secret() {
            Ok(bytes) => Ok(Some(Zeroizing::new(bytes))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(IdentityError::StoreUnavailable(e.to_string())),
        }
    }

    fn write(&self, value: &[u8]) -> Result<(), IdentityError> {
        self.0.set_secret(value).map_err(|e| IdentityError::StoreUnavailable(e.to_string()))
    }

    fn remove(&self) -> Result<(), IdentityError> {
        match self.0.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(IdentityError::StoreUnavailable(e.to_string())),
        }
    }
}

#[cfg(feature = "keyring")]
impl SecretStore for KeyringSecretStore {
    fn get(&self, entry: &str) -> Result<Option<Zeroizing<Vec<u8>>>, IdentityError> {
        if self.signed {
            return read_moving(&self.owned(entry)?, &self.legacy(entry)?);
        }
        self.legacy(entry)?.read()
    }

    fn peek(&self, entry: &str) -> Result<Option<Zeroizing<Vec<u8>>>, IdentityError> {
        read_only(self.signed, &self.owned(entry)?, &self.legacy(entry)?)
    }

    fn set(&self, entry: &str, value: &[u8]) -> Result<(), IdentityError> {
        if self.signed {
            self.owned(entry)?.write(value)?;
            if let Err(e) = self.legacy(entry)?.remove() {
                tracing::warn!(error = %e, "the older keychain item could not be removed; it stays, unused");
            }
            return Ok(());
        }
        self.legacy(entry)?.write(value)
    }

    fn delete(&self, entry: &str) -> Result<(), IdentityError> {
        if self.signed {
            self.owned(entry)?.remove()?;
        }
        self.legacy(entry)?.remove()
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

    /// A slot in memory that counts what was done to it and can be made to fail.
    #[derive(Default)]
    struct TestSlot {
        value: Mutex<Option<Vec<u8>>>,
        reads: Mutex<usize>,
        fail_write: bool,
        fail_remove: bool,
        /// A write stores this instead (a store that corrupts what it keeps).
        garble: Option<Vec<u8>>,
    }

    impl TestSlot {
        fn holding(value: &[u8]) -> Self {
            Self { value: Mutex::new(Some(value.to_vec())), ..Self::default() }
        }
        fn value(&self) -> Option<Vec<u8>> {
            self.value.lock().clone()
        }
    }

    impl Slot for TestSlot {
        fn read(&self) -> Result<Option<Zeroizing<Vec<u8>>>, IdentityError> {
            *self.reads.lock() += 1;
            Ok(self.value.lock().clone().map(Zeroizing::new))
        }
        fn write(&self, value: &[u8]) -> Result<(), IdentityError> {
            if self.fail_write {
                return Err(IdentityError::StoreUnavailable("keychain locked".into()));
            }
            *self.value.lock() = Some(self.garble.clone().unwrap_or_else(|| value.to_vec()));
            Ok(())
        }
        fn remove(&self) -> Result<(), IdentityError> {
            if self.fail_remove {
                return Err(IdentityError::StoreUnavailable("no".into()));
            }
            *self.value.lock() = None;
            Ok(())
        }
    }

    /// Regression (user report 2026-09-30: updating 0.0.10 → 0.0.11 asked twice, for
    /// voltip.identity.x25519 and .meta): a secret an older build stored moves to an item this
    /// build owns on its first read — written, read back, then the old item removed — and is read
    /// from there on, without touching the old item again.
    #[test]
    fn regression_a_secret_from_an_older_build_moves_to_an_item_this_build_owns() {
        let (owned, legacy) = (TestSlot::default(), TestSlot::holding(b"key"));
        assert_eq!(read_moving(&owned, &legacy).unwrap().unwrap().as_slice(), b"key");
        assert_eq!(owned.value().as_deref(), Some(&b"key"[..]));
        assert_eq!(legacy.value(), None, "the older item is gone once the copy read back");
        let legacy_reads = *legacy.reads.lock();
        assert_eq!(read_moving(&owned, &legacy).unwrap().unwrap().as_slice(), b"key");
        assert_eq!(*legacy.reads.lock(), legacy_reads, "an owned item is read without asking for the older one");
    }

    /// docs/dictation.md §23: a read-only read never moves a secret: a signed build looks at its
    /// own item only (an older build's item is left unread, it could ask), an unsigned build at its
    /// usual item; neither item changes.
    #[test]
    fn a_read_only_read_moves_nothing() {
        let (owned, legacy) = (TestSlot::default(), TestSlot::holding(b"key"));
        assert!(read_only(true, &owned, &legacy).unwrap().is_none(), "a signed build does not take the older item");
        assert_eq!((owned.value(), legacy.value().as_deref(), *legacy.reads.lock()), (None, Some(&b"key"[..]), 0));
        assert_eq!(read_only(false, &owned, &legacy).unwrap().unwrap().as_slice(), b"key");
        assert_eq!((owned.value(), legacy.value().as_deref()), (None, Some(&b"key"[..])), "nothing written or removed");
        let mine = TestSlot::holding(b"mine");
        assert_eq!(read_only(true, &mine, &legacy).unwrap().unwrap().as_slice(), b"mine");
        assert_eq!(*legacy.reads.lock(), 1, "only the unsigned read looked at the older item");
    }

    /// The default `peek` is `get`: the memory store has no side effect to avoid.
    #[test]
    fn the_memory_store_peeks_what_it_holds() {
        let s = MemorySecretStore::new();
        s.set("k", b"v").unwrap();
        assert_eq!(s.peek("k").unwrap().unwrap().as_slice(), b"v");
        assert!(s.peek("absent").unwrap().is_none());
    }

    /// Nothing stored anywhere: nothing is created either.
    #[test]
    fn nothing_to_move_is_none() {
        let (owned, legacy) = (TestSlot::default(), TestSlot::default());
        assert!(read_moving(&owned, &legacy).unwrap().is_none());
        assert_eq!(owned.value(), None);
    }

    /// The secret is never lost: a copy that cannot be written, or reads back different, leaves
    /// the older item in use (and no bad copy behind); an older item that cannot be removed
    /// stays unused next to the good copy.
    #[test]
    fn a_failed_move_keeps_the_older_item() {
        let (owned, legacy) = (TestSlot { fail_write: true, ..TestSlot::default() }, TestSlot::holding(b"key"));
        assert_eq!(read_moving(&owned, &legacy).unwrap().unwrap().as_slice(), b"key");
        assert_eq!(legacy.value().as_deref(), Some(&b"key"[..]));
        let (owned, legacy) = (TestSlot { garble: Some(b"kez".to_vec()), ..TestSlot::default() }, TestSlot::holding(b"key"));
        assert_eq!(read_moving(&owned, &legacy).unwrap().unwrap().as_slice(), b"key");
        assert_eq!(legacy.value().as_deref(), Some(&b"key"[..]), "a copy that reads back different is not trusted");
        assert_eq!(owned.value(), None, "and not left behind");
        let (owned, legacy) = (TestSlot::default(), TestSlot { fail_remove: true, ..TestSlot::holding(b"key") });
        assert_eq!(read_moving(&owned, &legacy).unwrap().unwrap().as_slice(), b"key");
        assert_eq!(owned.value().as_deref(), Some(&b"key"[..]));
        assert_eq!(legacy.value().as_deref(), Some(&b"key"[..]));
    }
}

/// Android Keystore-backed store: secrets live in `SharedPreferences`, encrypted with an AES
/// key generated inside the hardware-backed Keystore, via `android-native-keyring-store`.
/// The store reads the application context through `ndk-context`, which Tauri's Android runtime
/// does not set: the mobile shell's `MainActivity` hands it over through the crate's JNI entry
/// before Rust starts (`Keyring.kt`). Without that, `new` panics.
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
