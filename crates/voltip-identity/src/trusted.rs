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
/// Most phones a computer syncs with at once, and most computers a phone syncs with
/// (docs/dictation.md §20.8): the per-path windows of that many peers stay well inside the relay's
/// per-connection queue.
pub const MAX_SYNC_PEERS: usize = 5;

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
    /// On a computer: this phone gets the computer's history and settings and may upload its own
    /// records (docs/dictation.md §20.8). On by default; a record from before reads as on.
    #[serde(default = "sync_on")]
    pub sync: bool,
    /// Goes up every time `sync` changes and on every re-pair, so the phone tells the newest
    /// switch message from an older one that arrives late.
    #[serde(default)]
    pub sync_gen: u32,
}

fn sync_on() -> bool {
    true
}

impl TrustedDevice {
    /// A phone (the sync switch is a phone's).
    pub fn is_phone(&self) -> bool {
        matches!(self.platform, Platform::Android | Platform::Ios)
    }
}

/// What [`TrustedDeviceStore::set_sync`] did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SyncChange {
    /// Switched; the record as it is now.
    Changed(TrustedDevice),
    /// It was already so.
    Unchanged,
    /// Not switched on: [`MAX_SYNC_PEERS`] phones sync already.
    Limit,
    /// No such device.
    Unknown,
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
        let mut state = match std::fs::read(&path) {
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
        let capped = cap_sync(&mut state.devices);
        let store = Self { path: Some(path), state: Mutex::new(state) };
        if capped {
            store.flush()?;
        }
        Ok(store)
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
            sync: true,
            sync_gen: 0,
        };
        let record = {
            let mut state = self.state.lock();
            let others = state.devices.iter().filter(|d| d.public_key != key && d.is_phone() && d.sync).count();
            let sync = !record.is_phone() || others < MAX_SYNC_PEERS;
            if let Some(existing) = state.devices.iter_mut().find(|d| d.public_key == key) {
                // A re-pair: the generation only ever goes up, so the phone (which starts over
                // from 0 when a pairing completes) accepts what the computer sends from now on.
                *existing = TrustedDevice {
                    last_seen: existing.last_seen,
                    last_connection: existing.last_connection,
                    direct_hints: std::mem::take(&mut existing.direct_hints),
                    sync,
                    sync_gen: existing.sync_gen.wrapping_add(1),
                    ..record
                };
                existing.clone()
            } else {
                let record = TrustedDevice { sync, ..record };
                state.devices.push(record.clone());
                record
            }
        };
        self.flush()?;
        Ok(record)
    }

    /// Switch syncing with `key` on or off (docs/dictation.md §20.8); the generation goes up with
    /// every change. Switching on is refused once [`MAX_SYNC_PEERS`] phones sync.
    pub fn set_sync(&self, key: &PublicKey, on: bool) -> Result<SyncChange, IdentityError> {
        let change = {
            let mut state = self.state.lock();
            let others = state.devices.iter().filter(|d| &d.public_key != key && d.is_phone() && d.sync).count();
            match state.devices.iter_mut().find(|d| &d.public_key == key) {
                None => SyncChange::Unknown,
                Some(d) if d.sync == on => SyncChange::Unchanged,
                Some(d) if on && d.is_phone() && others >= MAX_SYNC_PEERS => SyncChange::Limit,
                Some(d) => {
                    d.sync = on;
                    d.sync_gen = d.sync_gen.wrapping_add(1);
                    SyncChange::Changed(d.clone())
                }
            }
        };
        if matches!(change, SyncChange::Changed(_)) {
            self.flush()?;
        }
        Ok(change)
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

/// More than [`MAX_SYNC_PEERS`] phones syncing (a file from before the switch reads every phone as
/// on): the ones paired first stay on, the others are switched off. `true` when anything changed.
fn cap_sync(devices: &mut [TrustedDevice]) -> bool {
    let mut syncing: Vec<usize> = (0..devices.len()).filter(|&i| devices[i].is_phone() && devices[i].sync).collect();
    if syncing.len() <= MAX_SYNC_PEERS {
        return false;
    }
    syncing.sort_by_key(|&i| (devices[i].trusted_at, i));
    for &i in &syncing[MAX_SYNC_PEERS..] {
        devices[i].sync = false;
        devices[i].sync_gen = devices[i].sync_gen.wrapping_add(1);
    }
    true
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

    fn phone(name: &str) -> DeviceInfo {
        DeviceInfo { device_id: DeviceId::random(), name: name.into(), platform: Platform::Android }
    }

    fn key(n: u8) -> PublicKey {
        PublicKey([n; 32])
    }

    /// regression (plan gate, M7 design round 3): a re-pair of the same key replaces the record,
    /// and its sync generation must go up, or the phone (which starts over from 0) could take a
    /// late message from before the re-pair as current. Forget-then-pair starts again from 0.
    #[test]
    fn regression_a_re_pair_raises_the_sync_generation() {
        let store = TrustedDeviceStore::in_memory();
        let first = store.trust(&phone("Pixel"), key(1), 10).unwrap();
        assert!(first.sync);
        assert_eq!(first.sync_gen, 0);
        assert!(matches!(store.set_sync(&key(1), false).unwrap(), SyncChange::Changed(d) if !d.sync && d.sync_gen == 1));
        let again = store.trust(&phone("Pixel"), key(1), 20).unwrap();
        assert!(again.sync, "a re-pair syncs again");
        assert_eq!(again.sync_gen, 2, "and its generation goes up");
        assert_eq!(store.get_by_key(&key(1)).unwrap(), again);
        store.forget(&key(1)).unwrap();
        assert_eq!(store.trust(&phone("Pixel"), key(1), 30).unwrap().sync_gen, 0, "a new pairing starts from 0");
    }

    #[test]
    fn the_sync_switch_changes_once_and_stops_at_the_limit() {
        let store = TrustedDeviceStore::in_memory();
        for n in 1..=MAX_SYNC_PEERS as u8 {
            assert!(store.trust(&phone(&format!("手机{n}")), key(n), u64::from(n)).unwrap().sync);
        }
        let sixth = store.trust(&phone("第六部"), key(9), 99).unwrap();
        assert!(!sixth.sync, "the sixth phone starts with sync off");
        assert_eq!(store.set_sync(&key(9), true).unwrap(), SyncChange::Limit);
        assert_eq!(store.set_sync(&key(1), true).unwrap(), SyncChange::Unchanged);
        assert!(matches!(store.set_sync(&key(1), false).unwrap(), SyncChange::Changed(d) if d.sync_gen == 1));
        assert!(matches!(store.set_sync(&key(9), true).unwrap(), SyncChange::Changed(d) if d.sync && d.sync_gen == 1), "a place came free");
        assert_eq!(store.set_sync(&key(42), true).unwrap(), SyncChange::Unknown);
        // Computers are not counted: a computer's record is never one of the phones.
        let desk = DeviceInfo { device_id: DeviceId::random(), name: "Desk".into(), platform: Platform::Windows };
        assert!(store.trust(&desk, key(50), 1).unwrap().sync);
    }

    #[test]
    fn a_file_from_before_reads_as_syncing_and_keeps_the_first_five_phones() {
        let dir = tempfile::tempdir().unwrap();
        let devices: Vec<serde_json::Value> = (1..=7u8)
            .map(|n| {
                serde_json::json!({
                    "device_id": DeviceId::random(), "name": format!("手机{n}"), "platform": "android",
                    "public_key": key(n), "fingerprint": key(n).fingerprint(), "trusted_at": 100 - u64::from(n)
                })
            })
            .collect();
        std::fs::write(dir.path().join(TRUSTED_FILE_NAME), serde_json::to_vec(&serde_json::json!({ "schema": 1, "devices": devices })).unwrap()).unwrap();
        let store = TrustedDeviceStore::open(dir.path()).unwrap();
        let syncing: Vec<u8> = store.list().iter().filter(|d| d.sync).map(|d| d.public_key.0[0]).collect();
        let mut syncing = syncing;
        syncing.sort_unstable();
        assert_eq!(syncing, [3, 4, 5, 6, 7], "paired first (smallest trusted_at) stay on");
        assert!(store.list().iter().filter(|d| !d.sync).all(|d| d.sync_gen == 1));
        // Written back: reading it again changes nothing.
        let again = TrustedDeviceStore::open(dir.path()).unwrap();
        assert_eq!(again.list(), store.list());
    }

    #[test]
    fn connection_kind_serializes_snake_case() {
        assert_eq!(serde_json::to_string(&ConnectionKind::Direct).unwrap(), r#""direct""#);
        let file = TrustedDevicesFile::default();
        assert_eq!(file.schema, TRUSTED_FILE_SCHEMA);
    }
}
