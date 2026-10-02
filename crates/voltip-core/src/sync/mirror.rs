//! A phone's copy of each computer's history and settings (docs/dictation.md §20.8): one
//! database per computer, `data_dir/mirror/<the computer's key in hex>.sqlite3`, with the tables
//! of `history.sqlite3` and a `shortened` table; its state (which history it came from, the last
//! change applied, the settings) sits in its own `meta`.
//!
//! The core writes through a [`MirrorStore`]; the bridge answers the interface's queries with a
//! read-only connection it opens and closes for each query. On Windows a file cannot be deleted
//! while a connection holds it, so every computer has a lock in [`MirrorFiles`]: a query holds it
//! shared, and a deletion takes it exclusively, closes the core's connection and removes the files.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use uuid::Uuid;

use super::wire::Profile;
use crate::CoreError;
use crate::history::{BUSY_TIMEOUT, HistoryEntry, HistoryReader, TABLES, insert_copy};

/// The directory of the copies inside the app data directory.
pub const MIRROR_DIR_NAME: &str = "mirror";

const MIRROR_TABLES: &str = "CREATE TABLE IF NOT EXISTS shortened (id TEXT PRIMARY KEY NOT NULL);";
const EPOCH_KEY: &str = "mirror_epoch";
const APPLIED_KEY: &str = "mirror_applied";
const PROFILE_TAG_KEY: &str = "mirror_profile_tag";
const PROFILE_KEY: &str = "mirror_profile";
const SYNCED_AT_KEY: &str = "mirror_synced_at_ms";

/// Where a copy stands.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MirrorState {
    /// The computer's history the copy came from.
    pub epoch: Option<Uuid>,
    /// The last change applied.
    pub applied: u64,
    /// The tag of the settings held.
    pub profile_tag: Option<[u8; 32]>,
    /// When a batch or the settings last arrived.
    pub synced_at_ms: Option<u64>,
    /// Entries in the copy.
    pub entries: u32,
}

/// The copies' files and their locks, shared by the core and the bridge.
#[derive(Debug)]
pub struct MirrorFiles {
    dir: PathBuf,
    locks: Mutex<HashMap<String, Arc<RwLock<()>>>>,
}

/// A computer's key in hex: 64 hex digits (a file name; nothing else is accepted).
pub fn valid_computer(computer: &str) -> bool {
    computer.len() == 64 && computer.bytes().all(|b| b.is_ascii_hexdigit())
}

impl MirrorFiles {
    /// The copies under `data_dir`.
    pub fn new(data_dir: &Path) -> Self {
        Self { dir: data_dir.join(MIRROR_DIR_NAME), locks: Mutex::new(HashMap::new()) }
    }

    /// The database of `computer`'s copy.
    pub fn path(&self, computer: &str) -> PathBuf {
        self.dir.join(format!("{computer}.sqlite3"))
    }

    fn lock(&self, computer: &str) -> Arc<RwLock<()>> {
        self.locks.lock().entry(computer.to_owned()).or_default().clone()
    }

    /// Run `read` on `computer`'s copy with a connection of its own (opened inside, closed when
    /// `read` returns); no deletion happens meanwhile. `Ok(None)` when there is no copy.
    pub fn read<T>(&self, computer: &str, read: impl FnOnce(&HistoryReader, &Path) -> Result<T, CoreError>) -> Result<Option<T>, CoreError> {
        if !valid_computer(computer) {
            return Err(CoreError::Invalid("unknown computer".into()));
        }
        let lock = self.lock(computer);
        let _shared = lock.read();
        let path = self.path(computer);
        if !path.exists() {
            return Ok(None);
        }
        let reader = HistoryReader::at(&path);
        read(&reader, &path).map(Some)
    }

    /// Delete `computer`'s copy: wait for the queries in progress, close the core's connection
    /// (`store`), remove the database and its `-wal` and `-shm`.
    pub fn delete(&self, computer: &str, store: Option<MirrorStore>) -> std::io::Result<()> {
        let lock = self.lock(computer);
        let _exclusive = lock.write();
        drop(store);
        let path = self.path(computer);
        for suffix in ["", "-wal", "-shm"] {
            let file = PathBuf::from(format!("{}{suffix}", path.display()));
            match std::fs::remove_file(&file) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

/// The core's connection to one computer's copy (in a mutex only so the runtime stays `Sync`;
/// writes take `&mut self` and never lock it).
#[derive(Debug)]
pub struct MirrorStore {
    conn: Mutex<Connection>,
}

fn err(e: impl std::fmt::Display) -> CoreError {
    CoreError::History(e.to_string())
}

fn meta(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| row.get(0)).optional()
}

fn set_meta(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute("INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value", params![key, value]).map(drop)
}

impl MirrorStore {
    /// Open (or create) `computer`'s copy.
    pub fn open(files: &MirrorFiles, computer: &str) -> Result<Self, CoreError> {
        if !valid_computer(computer) {
            return Err(CoreError::Invalid("unknown computer".into()));
        }
        std::fs::create_dir_all(&files.dir).map_err(err)?;
        let conn = Connection::open(files.path(computer)).map_err(err)?;
        conn.busy_timeout(BUSY_TIMEOUT).map_err(err)?;
        let _: String = conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0)).map_err(err)?;
        conn.pragma_update(None, "synchronous", "NORMAL").map_err(err)?;
        conn.pragma_update(None, "foreign_keys", true).map_err(err)?;
        conn.execute_batch(TABLES).map_err(err)?;
        conn.execute_batch(MIRROR_TABLES).map_err(err)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    /// Where the copy stands.
    pub fn state(&self) -> Result<MirrorState, CoreError> {
        let conn = self.conn.lock();
        let read = || -> rusqlite::Result<MirrorState> {
            Ok(MirrorState {
                epoch: meta(&conn, EPOCH_KEY)?.and_then(|v| v.parse().ok()),
                applied: meta(&conn, APPLIED_KEY)?.and_then(|v| v.parse().ok()).unwrap_or(0),
                profile_tag: meta(&conn, PROFILE_TAG_KEY)?.and_then(|v| hex::decode(v).ok()).and_then(|v| v.try_into().ok()),
                synced_at_ms: meta(&conn, SYNCED_AT_KEY)?.and_then(|v| v.parse().ok()),
                entries: conn.query_row("SELECT COUNT(*) FROM entries", [], |row| row.get::<_, i64>(0)).map(|n| u32::try_from(n).unwrap_or(u32::MAX))?,
            })
        };
        read().map_err(err)
    }

    /// The settings held, if any.
    pub fn profile(&self) -> Result<Option<Profile>, CoreError> {
        read_profile(&self.conn.lock()).map_err(err)
    }

    /// Replace the settings (one transaction).
    pub fn set_profile(&mut self, tag: [u8; 32], profile: &Profile, now_ms: u64) -> Result<(), CoreError> {
        let json = serde_json::to_string(profile).map_err(err)?;
        let tx = self.conn.get_mut().transaction().map_err(err)?;
        set_meta(&tx, PROFILE_TAG_KEY, &hex::encode(tag)).map_err(err)?;
        set_meta(&tx, PROFILE_KEY, &json).map_err(err)?;
        set_meta(&tx, SYNCED_AT_KEY, &now_ms.to_string()).map_err(err)?;
        tx.commit().map_err(err)
    }

    /// Apply one batch of changes in one transaction: a `reset` empties the copy first; then the
    /// deletions, then the entries (one already there is replaced, its hits with it); the change
    /// reached is recorded last. Applying a batch twice leaves the same copy.
    #[allow(clippy::too_many_arguments)]
    pub fn apply(
        &mut self,
        epoch: Uuid,
        reset: bool,
        to: u64,
        upserts: &[HistoryEntry],
        deletes: &[Uuid],
        shortened: &[Uuid],
        now_ms: u64,
    ) -> Result<(), CoreError> {
        let tx = self.conn.get_mut().transaction().map_err(err)?;
        apply_batch(&tx, epoch, reset, to, upserts, deletes, shortened, now_ms).map_err(err)?;
        tx.commit().map_err(err)
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_batch(
    tx: &Connection,
    epoch: Uuid,
    reset: bool,
    to: u64,
    upserts: &[HistoryEntry],
    deletes: &[Uuid],
    shortened: &[Uuid],
    now_ms: u64,
) -> rusqlite::Result<()> {
    if reset {
        tx.execute_batch("DELETE FROM hits; DELETE FROM entries; DELETE FROM shortened;")?;
    }
    set_meta(tx, EPOCH_KEY, &epoch.to_string())?;
    for id in deletes {
        tx.execute("DELETE FROM entries WHERE id = ?1", [id.to_string()])?;
        tx.execute("DELETE FROM shortened WHERE id = ?1", [id.to_string()])?;
    }
    for entry in upserts {
        let id = entry.id.to_string();
        tx.execute("DELETE FROM entries WHERE id = ?1", [&id])?;
        insert_copy(tx, entry)?;
        if shortened.contains(&entry.id) {
            tx.execute("INSERT OR IGNORE INTO shortened (id) VALUES (?1)", [&id])?;
        } else {
            tx.execute("DELETE FROM shortened WHERE id = ?1", [&id])?;
        }
    }
    set_meta(tx, APPLIED_KEY, &to.to_string())?;
    set_meta(tx, SYNCED_AT_KEY, &now_ms.to_string())
}

fn read_profile(conn: &Connection) -> rusqlite::Result<Option<Profile>> {
    Ok(meta(conn, PROFILE_KEY)?.and_then(|json| serde_json::from_str(&json).inspect_err(|e| tracing::warn!(error = %e, "copied settings unreadable")).ok()))
}

/// One entry of a copy and whether it was shortened, read through `path` (inside
/// [`MirrorFiles::read`]).
pub fn read_entry(path: &Path, id: Uuid) -> Result<Option<(HistoryEntry, bool)>, CoreError> {
    let conn = read_only(path)?;
    let row: Option<(String, bool)> = conn
        .query_row("SELECT e.json, EXISTS (SELECT 1 FROM shortened s WHERE s.id = e.id) FROM entries e WHERE e.id = ?1", [id.to_string()], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .optional()
        .map_err(err)?;
    Ok(row.and_then(|(json, shortened)| serde_json::from_str(&json).ok().map(|e| (e, shortened))))
}

/// The settings of a copy, read through `path` (inside [`MirrorFiles::read`]).
pub fn read_copied_profile(path: &Path) -> Result<Option<Profile>, CoreError> {
    read_profile(&read_only(path)?).map_err(err)
}

fn read_only(path: &Path) -> Result<Connection, CoreError> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(err)?;
    conn.busy_timeout(BUSY_TIMEOUT).map_err(err)?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::HistoryQuery;

    const COMPUTER: &str = "ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12";

    fn entry(text: &str, at_ms: u64) -> HistoryEntry {
        serde_json::from_value(serde_json::json!({
            "id": Uuid::new_v4(), "at_ms": at_ms, "raw_text": text, "text": text, "refined": false, "asr_model": "m",
            "duration_ms": 1, "asr_ms": 1, "outcome": { "kind": "inserted", "via": "paste" }
        }))
        .unwrap()
    }

    fn query(files: &MirrorFiles) -> Option<Vec<String>> {
        files
            .read(COMPUTER, |reader, _| {
                Ok(reader.query(&HistoryQuery { limit: 200, ..HistoryQuery::default() })?.entries.into_iter().map(|e| e.text).collect())
            })
            .unwrap()
    }

    #[test]
    fn batches_apply_once_and_again_to_the_same_copy() {
        let dir = tempfile::tempdir().unwrap();
        let files = MirrorFiles::new(dir.path());
        let mut store = MirrorStore::open(&files, COMPUTER).unwrap();
        assert_eq!(store.state().unwrap(), MirrorState::default());
        let epoch = Uuid::new_v4();
        let [a, b, c] = [entry("一", 1), entry("二", 2), entry("三", 3)];
        for _ in 0..2 {
            store.apply(epoch, true, 3, &[a.clone(), b.clone(), c.clone()], &[], &[c.id], 100).unwrap();
            let state = store.state().unwrap();
            assert_eq!((state.epoch, state.applied, state.entries, state.synced_at_ms), (Some(epoch), 3, 3, Some(100)), "the same batch twice");
        }
        let changed = HistoryEntry { starred: true, ..b.clone() };
        store.apply(epoch, false, 5, &[changed.clone(), c.clone()], &[a.id], &[], 200).unwrap();
        assert_eq!(query(&files).unwrap(), ["三", "二"]);
        let path = files.path(COMPUTER);
        assert_eq!(read_entry(&path, b.id).unwrap(), Some((changed, false)));
        assert_eq!(read_entry(&path, c.id).unwrap().map(|(_, s)| s), Some(false), "no longer shortened");
        assert_eq!(read_entry(&path, a.id).unwrap(), None);
        // A reset replaces everything, from another history too.
        let other = Uuid::new_v4();
        store.apply(other, true, 1, &[entry("新", 9)], &[], &[], 300).unwrap();
        assert_eq!(query(&files).unwrap(), ["新"]);
        assert_eq!(store.state().unwrap().epoch, Some(other));
    }

    /// A copy's statistics are never read, so it does not count corrections: an edit distance per
    /// entry was most of the cost of a first sync (docs/dictation.md §20.8).
    #[test]
    fn a_copy_keeps_no_correction_count() {
        let dir = tempfile::tempdir().unwrap();
        let files = MirrorFiles::new(dir.path());
        let mut store = MirrorStore::open(&files, COMPUTER).unwrap();
        let corrected = HistoryEntry { raw_text: "嗯那个明天开会".into(), ..entry("明天开会。", 4) };
        store.apply(Uuid::new_v4(), true, 1, std::slice::from_ref(&corrected), &[], &[], 100).unwrap();
        let conn = Connection::open(files.path(COMPUTER)).unwrap();
        let count: i64 = conn.query_row("SELECT corrected_chars FROM entries WHERE id = ?1", [corrected.id.to_string()], |row| row.get(0)).unwrap();
        assert_eq!(count, 0);
        assert_eq!(read_entry(&files.path(COMPUTER), corrected.id).unwrap(), Some((corrected.clone(), false)), "the entry itself is whole");
        let mut history = crate::history::HistoryStore::open(dir.path());
        history.push(corrected.clone(), 100).unwrap();
        let own: i64 = Connection::open(dir.path().join(crate::history::HISTORY_DB_FILE_NAME))
            .unwrap()
            .query_row("SELECT corrected_chars FROM entries WHERE id = ?1", [corrected.id.to_string()], |row| row.get(0))
            .unwrap();
        assert_eq!(own, 4, "the computer's own history counts them");
    }

    #[test]
    fn the_settings_are_kept_with_their_tag() {
        let dir = tempfile::tempdir().unwrap();
        let files = MirrorFiles::new(dir.path());
        let mut store = MirrorStore::open(&files, COMPUTER).unwrap();
        assert_eq!(store.profile().unwrap(), None);
        let profile = Profile::new(&crate::Settings::default(), &crate::EngineStatus::default(), &[], &[], &[], &[]);
        store.set_profile(profile.tag(), &profile, 5).unwrap();
        assert_eq!(store.profile().unwrap(), Some(profile.clone()));
        assert_eq!(store.state().unwrap().profile_tag, Some(profile.tag()));
        assert_eq!(read_copied_profile(&files.path(COMPUTER)).unwrap(), Some(profile));
    }

    /// regression (plan gate, M7 design round 2): Windows cannot delete a database another
    /// connection holds. A deletion waits for the query in progress, then every file is gone and
    /// later queries find nothing; a new copy shows only its own content.
    #[test]
    fn regression_a_deletion_waits_for_the_query_in_progress() {
        let dir = tempfile::tempdir().unwrap();
        let files = Arc::new(MirrorFiles::new(dir.path()));
        let mut store = MirrorStore::open(&files, COMPUTER).unwrap();
        store.apply(Uuid::new_v4(), true, 1, &[entry("旧", 1)], &[], &[], 1).unwrap();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let reading = {
            let files = files.clone();
            std::thread::spawn(move || {
                files
                    .read(COMPUTER, |reader, _| {
                        let page = reader.query(&HistoryQuery { limit: 200, ..HistoryQuery::default() })?;
                        started_tx.send(()).unwrap();
                        release_rx.recv_timeout(std::time::Duration::from_secs(30)).unwrap();
                        Ok(page.entries.len())
                    })
                    .unwrap()
            })
        };
        started_rx.recv_timeout(std::time::Duration::from_secs(30)).unwrap();
        let deleting = {
            let files = files.clone();
            std::thread::spawn(move || files.delete(COMPUTER, Some(store)))
        };
        // Give the deletion time to reach the lock; it cannot finish while the query holds it.
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(!deleting.is_finished(), "the deletion waits for the query");
        release_tx.send(()).unwrap();
        assert_eq!(reading.join().unwrap(), Some(1));
        deleting.join().unwrap().unwrap();
        for suffix in ["", "-wal", "-shm"] {
            assert!(!PathBuf::from(format!("{}{suffix}", files.path(COMPUTER).display())).exists(), "{suffix}");
        }
        assert_eq!(query(&files), None);
        let mut again = MirrorStore::open(&files, COMPUTER).unwrap();
        again.apply(Uuid::new_v4(), true, 1, &[entry("新", 2)], &[], &[], 2).unwrap();
        assert_eq!(query(&files).unwrap(), ["新"]);
        drop(again);
        files.delete(COMPUTER, None).unwrap();
        files.delete(COMPUTER, None).unwrap();
    }

    #[test]
    fn only_a_key_names_a_copy() {
        let dir = tempfile::tempdir().unwrap();
        let files = MirrorFiles::new(dir.path());
        for bad in ["", "../history", &"z".repeat(64), &"a".repeat(63)] {
            assert!(!valid_computer(bad), "{bad}");
            assert!(files.read(bad, |_, _| Ok(())).is_err());
            assert!(MirrorStore::open(&files, bad).is_err());
        }
        assert!(valid_computer(COMPUTER));
        assert_eq!(files.read(COMPUTER, |_, _| Ok(())).unwrap(), None, "no copy yet");
    }
}
