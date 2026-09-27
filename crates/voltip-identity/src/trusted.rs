//! Registry of paired devices.

use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use voltip_crypto::PublicKey;
use voltip_protocol::{DeviceId, DeviceInfo, Platform};

use crate::IdentityError;

/// Schema version of the on-disk file.
pub const TRUSTED_FILE_SCHEMA: u16 = 1;
/// File name inside the app data directory.
pub const TRUSTED_FILE_NAME: &str = "trusted-devices.json";

/// How the peer was last reached.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionKind {
    /// Same LAN, no relay.
    Direct,
    /// Through a relay.
    Relay,
}

/// A paired peer.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct TrustedDevice {
    /// Peer's stable id (self-reported at pairing, informational).
    pub device_id: DeviceId,
    /// Peer's name at pairing (or last update).
    pub name: String,
    /// Peer platform.
    pub platform: Platform,
    /// **The** trust anchor: the static public key authenticated by the handshake.
    pub public_key: PublicKey,
    /// `public_key.fingerprint()` cached for the UI.
    pub fingerprint: String,
    /// Unix seconds when the user confirmed the safety code.
    pub trusted_at: u64,
    /// Unix seconds of the last completed connection, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<u64>,
    /// Transport used at `last_seen`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_connection: Option<ConnectionKind>,
    /// Last known `ip:port` endpoints of the peer's LAN host — tried first on every reconnect,
    /// before falling back to the relay. Refreshed by the peer over the encrypted channel.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub direct_hints: Vec<String>,
}

/// Outcome of comparing a freshly authenticated peer against the registry.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum IdentityCheck {
    /// Key matches a trusted record.
    Trusted(TrustedDevice),
    /// A record with this `device_id` exists but its key differs. **Never** trust silently.
    IdentityChanged {
        /// The stored record.
        previous: TrustedDevice,
        /// The key that just authenticated.
        presented: PublicKey,
    },
    /// Nobody we know.
    Unknown,
}

/// On-disk shape.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct TrustedDevicesFile {
    /// Schema version.
    pub schema: u16,
    /// Records.
    pub devices: Vec<TrustedDevice>,
}

impl Default for TrustedDevicesFile {
    fn default() -> Self {
        Self { schema: TRUSTED_FILE_SCHEMA, devices: Vec::new() }
    }
}

/// Thread-safe registry persisted as JSON. Writes are atomic (temp file + rename).
pub struct TrustedDeviceStore {
    path: Option<PathBuf>,
    state: Mutex<TrustedDevicesFile>,
}

impl std::fmt::Debug for TrustedDeviceStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrustedDeviceStore").field("path", &self.path).field("count", &self.state.lock().devices.len()).finish()
    }
}

impl TrustedDeviceStore {
    /// In-memory registry (tests, relay).
    pub fn in_memory() -> Self {
        Self { path: None, state: Mutex::new(TrustedDevicesFile::default()) }
    }

    /// Open (or create) the registry at `dir/trusted-devices.json`.
    pub fn open(dir: &Path) -> Result<Self, IdentityError> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(TRUSTED_FILE_NAME);
        let state = match std::fs::read(&path) {
            Ok(bytes) => {
                let file: TrustedDevicesFile = serde_json::from_slice(&bytes)?;
                if file.schema != TRUSTED_FILE_SCHEMA {
                    return Err(IdentityError::Corrupt);
                }
                file
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => TrustedDevicesFile::default(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self { path: Some(path), state: Mutex::new(state) })
    }

    /// All records, sorted by name for stable UI ordering.
    pub fn list(&self) -> Vec<TrustedDevice> {
        let mut v = self.state.lock().devices.clone();
        v.sort_by(|a, b| a.name.cmp(&b.name).then(a.device_id.0.cmp(&b.device_id.0)));
        v
    }

    /// Look up by static key.
    pub fn get_by_key(&self, key: &PublicKey) -> Option<TrustedDevice> {
        self.state.lock().devices.iter().find(|d| &d.public_key == key).cloned()
    }

    /// Compare an authenticated peer against the registry.
    pub fn check(&self, presented: &PublicKey, claimed_id: DeviceId) -> IdentityCheck {
        let state = self.state.lock();
        if let Some(d) = state.devices.iter().find(|d| &d.public_key == presented) {
            return IdentityCheck::Trusted(d.clone());
        }
        if let Some(d) = state.devices.iter().find(|d| d.device_id == claimed_id) {
            return IdentityCheck::IdentityChanged { previous: d.clone(), presented: *presented };
        }
        IdentityCheck::Unknown
    }

    /// Record a newly trusted peer (after both users confirmed the safety code). Replaces an
    /// existing record with the same key; a same-`device_id`-different-key record is *not*
    /// overwritten — the caller must go through [`Self::forget`] first, which is what makes a
    /// deliberate re-pair explicit.
    pub fn trust(&self, info: &DeviceInfo, key: PublicKey, now_unix: u64) -> Result<TrustedDevice, IdentityError> {
        info.validate()?;
        let record = TrustedDevice {
            device_id: info.device_id,
            name: info.name.clone(),
            platform: info.platform,
            public_key: key,
            fingerprint: key.fingerprint(),
            trusted_at: now_unix,
            last_seen: None,
            last_connection: None,
            direct_hints: Vec::new(),
        };
        {
            let mut state = self.state.lock();
            if let Some(existing) = state.devices.iter_mut().find(|d| d.public_key == key) {
                *existing = TrustedDevice {
                    last_seen: existing.last_seen,
                    last_connection: existing.last_connection,
                    direct_hints: std::mem::take(&mut existing.direct_hints),
                    ..record.clone()
                };
            } else {
                state.devices.push(record.clone());
            }
        }
        self.flush()?;
        Ok(record)
    }

    /// Update presence metadata after a successful connection.
    pub fn mark_seen(&self, key: &PublicKey, now_unix: u64, via: ConnectionKind) -> Result<bool, IdentityError> {
        let updated = {
            let mut state = self.state.lock();
            match state.devices.iter_mut().find(|d| &d.public_key == key) {
                Some(d) => {
                    d.last_seen = Some(now_unix);
                    d.last_connection = Some(via);
                    true
                }
                None => false,
            }
        };
        if updated {
            self.flush()?;
        }
        Ok(updated)
    }

    /// Apply a peer-announced rename.
    pub fn update_info(&self, key: &PublicKey, info: &DeviceInfo) -> Result<bool, IdentityError> {
        info.validate()?;
        let updated = {
            let mut state = self.state.lock();
            match state.devices.iter_mut().find(|d| &d.public_key == key) {
                Some(d) => {
                    d.name = info.name.clone();
                    d.platform = info.platform;
                    true
                }
                None => false,
            }
        };
        if updated {
            self.flush()?;
        }
        Ok(updated)
    }

    /// Replace the peer's LAN endpoints. Returns `Ok(true)` when the stored list changed.
    /// Hints are validated (bounded count, `ip:port` only) so a peer cannot plant garbage.
    pub fn update_hints(&self, key: &PublicKey, hints: &[String]) -> Result<bool, IdentityError> {
        voltip_protocol::validate_direct_hints(hints)?;
        let changed = {
            let mut state = self.state.lock();
            match state.devices.iter_mut().find(|d| &d.public_key == key) {
                Some(d) if d.direct_hints != hints => {
                    d.direct_hints = hints.to_vec();
                    true
                }
                _ => false,
            }
        };
        if changed {
            self.flush()?;
        }
        Ok(changed)
    }

    /// Remove a peer. Returns whether anything was removed.
    pub fn forget(&self, key: &PublicKey) -> Result<bool, IdentityError> {
        let removed = {
            let mut state = self.state.lock();
            let before = state.devices.len();
            state.devices.retain(|d| &d.public_key != key);
            state.devices.len() != before
        };
        if removed {
            self.flush()?;
        }
        Ok(removed)
    }

    /// Remove everything (used with identity reset).
    pub fn clear(&self) -> Result<(), IdentityError> {
        self.state.lock().devices.clear();
        self.flush()
    }

    fn flush(&self) -> Result<(), IdentityError> {
        let Some(path) = &self.path else { return Ok(()) };
        let bytes = serde_json::to_vec_pretty(&*self.state.lock())?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(name: &str) -> DeviceInfo {
        DeviceInfo { device_id: DeviceId::random(), name: name.into(), platform: Platform::Android }
    }

    #[test]
    fn trust_check_seen_forget_roundtrip_in_memory() {
        let store = TrustedDeviceStore::in_memory();
        let phone = info("Pixel 10");
        let key = PublicKey([1u8; 32]);
        assert_eq!(store.check(&key, phone.device_id), IdentityCheck::Unknown);
        let rec = store.trust(&phone, key, 1_000).unwrap();
        assert_eq!(rec.fingerprint, key.fingerprint());
        assert_eq!(store.list().len(), 1);
        assert_eq!(store.get_by_key(&key).unwrap().name, "Pixel 10");
        assert!(matches!(store.check(&key, phone.device_id), IdentityCheck::Trusted(_)));
        assert!(store.mark_seen(&key, 2_000, ConnectionKind::Relay).unwrap());
        let seen = store.get_by_key(&key).unwrap();
        assert_eq!(seen.last_seen, Some(2_000));
        assert_eq!(seen.last_connection, Some(ConnectionKind::Relay));
        assert!(!store.mark_seen(&PublicKey([9u8; 32]), 3_000, ConnectionKind::Direct).unwrap());
        assert!(store.forget(&key).unwrap());
        assert!(!store.forget(&key).unwrap());
        assert!(store.list().is_empty());
        assert!(format!("{store:?}").contains("count"));
    }

    #[test]
    fn direct_hints_persist_survive_retrust_and_are_validated() {
        let dir = tempfile::tempdir().unwrap();
        let store = TrustedDeviceStore::open(dir.path()).unwrap();
        let phone = info("Pixel 10");
        let key = PublicKey([1u8; 32]);
        store.trust(&phone, key, 1_000).unwrap();
        assert!(store.get_by_key(&key).unwrap().direct_hints.is_empty());
        let hints = vec!["192.168.1.24:47831".to_string()];
        assert!(store.update_hints(&key, &hints).unwrap());
        assert!(!store.update_hints(&key, &hints).unwrap(), "same hints are not a change");
        assert!(!store.update_hints(&PublicKey([9u8; 32]), &hints).unwrap(), "unknown key is a no-op");
        assert!(store.update_hints(&key, &["relay.example.org:1".to_string()]).is_err(), "hostnames are refused");
        // Re-trusting the same key (re-pair) keeps the hints we already learned.
        store.trust(&phone, key, 2_000).unwrap();
        assert_eq!(store.get_by_key(&key).unwrap().direct_hints, hints);
        let reopened = TrustedDeviceStore::open(dir.path()).unwrap();
        assert_eq!(reopened.get_by_key(&key).unwrap().direct_hints, hints);
        assert!(store.update_hints(&key, &[]).unwrap());
        assert!(!std::fs::read_to_string(dir.path().join(TRUSTED_FILE_NAME)).unwrap().contains("direct_hints"), "empty list is not serialized");
    }

    #[test]
    fn identity_change_is_flagged_not_trusted() {
        let store = TrustedDeviceStore::in_memory();
        let phone = info("Pixel 10");
        let old_key = PublicKey([1u8; 32]);
        store.trust(&phone, old_key, 1).unwrap();
        let new_key = PublicKey([2u8; 32]);
        match store.check(&new_key, phone.device_id) {
            IdentityCheck::IdentityChanged { previous, presented } => {
                assert_eq!(previous.public_key, old_key);
                assert_eq!(presented, new_key);
            }
            other => panic!("expected IdentityChanged, got {other:?}"),
        }
        // Re-trusting with the new key keeps the old record too (explicit forget required).
        store.trust(&phone, new_key, 2).unwrap();
        assert_eq!(store.list().len(), 2);
    }

    #[test]
    fn re_trusting_same_key_refreshes_metadata_but_keeps_presence() {
        let store = TrustedDeviceStore::in_memory();
        let key = PublicKey([1u8; 32]);
        let phone = info("Pixel 10");
        store.trust(&phone, key, 1).unwrap();
        store.mark_seen(&key, 5, ConnectionKind::Direct).unwrap();
        let renamed = DeviceInfo { name: "Pixel 10 Pro".into(), ..phone.clone() };
        store.trust(&renamed, key, 9).unwrap();
        let rec = store.get_by_key(&key).unwrap();
        assert_eq!(rec.name, "Pixel 10 Pro");
        assert_eq!(rec.trusted_at, 9);
        assert_eq!(rec.last_seen, Some(5));
        assert_eq!(store.list().len(), 1);
        assert!(store.update_info(&key, &DeviceInfo { name: "Renamed".into(), ..phone.clone() }).unwrap());
        assert_eq!(store.get_by_key(&key).unwrap().name, "Renamed");
        assert!(!store.update_info(&PublicKey([7u8; 32]), &phone).unwrap());
        assert!(store.update_info(&key, &DeviceInfo { name: String::new(), ..phone }).is_err());
    }

    #[test]
    fn list_is_sorted_by_name() {
        let store = TrustedDeviceStore::in_memory();
        store.trust(&info("Zed"), PublicKey([1; 32]), 1).unwrap();
        store.trust(&info("Alpha"), PublicKey([2; 32]), 1).unwrap();
        let names: Vec<_> = store.list().into_iter().map(|d| d.name).collect();
        assert_eq!(names, ["Alpha", "Zed"]);
        store.clear().unwrap();
        assert!(store.list().is_empty());
    }

    #[test]
    fn file_store_persists_atomically_and_validates_schema() {
        let dir = tempfile::tempdir().unwrap();
        let key = PublicKey([3u8; 32]);
        {
            let store = TrustedDeviceStore::open(dir.path()).unwrap();
            store.trust(&info("Pixel"), key, 42).unwrap();
        }
        let path = dir.path().join(TRUSTED_FILE_NAME);
        assert!(path.exists());
        assert!(!dir.path().join("trusted-devices.json.tmp").exists(), "temp file must be renamed away");
        let reopened = TrustedDeviceStore::open(dir.path()).unwrap();
        assert_eq!(reopened.get_by_key(&key).unwrap().trusted_at, 42);
        // Corrupt JSON.
        std::fs::write(&path, b"{{{").unwrap();
        assert!(matches!(TrustedDeviceStore::open(dir.path()).unwrap_err(), IdentityError::Serde(_)));
        // Wrong schema.
        std::fs::write(&path, br#"{"schema":99,"devices":[]}"#).unwrap();
        assert!(matches!(TrustedDeviceStore::open(dir.path()).unwrap_err(), IdentityError::Corrupt));
    }

    #[test]
    fn trust_rejects_invalid_device_info() {
        let store = TrustedDeviceStore::in_memory();
        let bad = DeviceInfo { device_id: DeviceId::random(), name: String::new(), platform: Platform::Ios };
        assert!(matches!(store.trust(&bad, PublicKey([1; 32]), 1).unwrap_err(), IdentityError::Protocol(_)));
    }

    #[test]
    fn connection_kind_serializes_snake_case() {
        assert_eq!(serde_json::to_string(&ConnectionKind::Direct).unwrap(), r#""direct""#);
        let file = TrustedDevicesFile::default();
        assert_eq!(file.schema, TRUSTED_FILE_SCHEMA);
    }
}
