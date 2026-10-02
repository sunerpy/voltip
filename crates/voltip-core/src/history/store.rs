//! The writing side of the history database (docs/dictation.md §4): the core's only connection to
//! `history.sqlite3`, and the one-time import of a `history.json` from before.
//!
//! The import can stop at any step and runs again at the next start, without losing or doubling
//! an entry:
//! 1. a `history.sqlite3.importing` left by a run that stopped is deleted;
//! 2. `history.json` and no database: every entry and the file's SHA-256 (in `meta`) go into
//!    `history.sqlite3.importing` in one transaction; the file is closed and renamed
//!    `history.sqlite3`;
//! 3. `history.json` is renamed `history.json.imported-<unix seconds>` and never deleted;
//! 4. `history.json` beside a database: when its hash is the one in `meta` (the run stopped
//!    between steps 2 and 3), only step 3 is done; otherwise the file is moved aside like a file
//!    that cannot be read, and nothing is imported twice.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use rusqlite::{Connection, ErrorCode, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::revs::{self, ChangeBatch, Outbox, RECEIVED_KEEP_MS};
use super::{HISTORY_DB_FILE_NAME, HISTORY_FILE_NAME, HISTORY_SCHEMA, HistoryEntry, MAX_ENTRIES, ProcessedText, derived};
use crate::CoreError;

/// A database the import is still writing (step 2); deleted when found at start (step 1).
pub(super) const IMPORTING_FILE_NAME: &str = "history.sqlite3.importing";
/// `meta` key: the SHA-256 of the `history.json` that was imported.
pub(super) const IMPORTED_DIGEST_KEY: &str = "imported_json_sha256";
/// `PRAGMA user_version` of the tables below: 2 added `sync_revs`, `phone_received` and
/// `uploaded` (docs/dictation.md §20.8). Tables are only ever added, never changed.
pub(super) const SCHEMA_VERSION: i32 = 2;
/// How long a statement waits for another connection's lock before it fails.
pub(crate) const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
/// Newest first: the order of every list.
pub(super) const NEWEST_FIRST: &str = "ORDER BY at_ms DESC, rowid DESC";

/// One row per entry: the entry's JSON is the record, the other columns are derived from it when
/// it is written (`derived`) for the filters, the search and the statistics. `hits` holds the
/// entry's dictionary and rule hits for `history_hits`. For the phones (docs/dictation.md §20.8):
/// `sync_revs` numbers the changes (`revs`), `phone_received` remembers the records a phone
/// uploaded, and on a phone `uploaded` says which of its own records a computer has.
pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS entries (
    id TEXT PRIMARY KEY NOT NULL,
    at_ms INTEGER NOT NULL,
    starred INTEGER NOT NULL,
    kind TEXT NOT NULL,
    outcome TEXT NOT NULL,
    app_id TEXT,
    scene_id TEXT,
    spoken_ms INTEGER NOT NULL,
    raw_chars INTEGER NOT NULL,
    corrected_chars INTEGER NOT NULL,
    latency_ms INTEGER NOT NULL,
    counts_for_stats INTEGER NOT NULL,
    search TEXT NOT NULL,
    json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS entries_at ON entries (at_ms);
CREATE INDEX IF NOT EXISTS entries_starred ON entries (starred, at_ms);
CREATE INDEX IF NOT EXISTS entries_stats ON entries (counts_for_stats, at_ms);
CREATE TABLE IF NOT EXISTS hits (
    entry_id TEXT NOT NULL REFERENCES entries (id) ON DELETE CASCADE,
    list TEXT NOT NULL,
    item_id TEXT NOT NULL,
    count INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS hits_entry ON hits (entry_id);
CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS sync_revs (
    id TEXT PRIMARY KEY NOT NULL,
    rev INTEGER NOT NULL,
    deleted INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS sync_revs_rev ON sync_revs (rev);
CREATE TABLE IF NOT EXISTS phone_received (
    id TEXT PRIMARY KEY NOT NULL,
    at_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS uploaded (
    id TEXT PRIMARY KEY NOT NULL REFERENCES entries (id) ON DELETE CASCADE,
    computer TEXT NOT NULL,
    at_ms INTEGER NOT NULL
);
";

/// The layout of `history.json`.
#[derive(Serialize, Deserialize)]
pub(super) struct HistoryFile {
    pub(super) schema: u16,
    pub(super) entries: Vec<HistoryEntry>,
}

/// Writes the history. `None` in `conn`: the directory cannot hold the database; reads answer
/// empty and every write fails with the reason. The connection sits in a mutex only so the store
/// is `Sync` (the runtime is borrowed across awaits); writes take `&mut self` and never lock it.
#[derive(Debug)]
pub struct HistoryStore {
    path: PathBuf,
    conn: Option<Mutex<Connection>>,
    /// Entries in the database, kept so a push knows without counting whether to drop the oldest.
    len: usize,
}

impl HistoryStore {
    /// Open `dir/history.sqlite3`, importing a `history.json` from before first. A file that is
    /// not a database is moved aside to `history.sqlite3.corrupt` and a new one is started.
    pub fn open(dir: &Path) -> Self {
        let path = dir.join(HISTORY_DB_FILE_NAME);
        if let Err(e) = std::fs::create_dir_all(dir) {
            tracing::warn!(error = %e, dir = %dir.display(), "history directory unavailable; nothing is recorded");
            return Self { path, conn: None, len: 0 };
        }
        remove_unfinished_import(dir);
        import_legacy(dir, &path);
        let conn = match open_writer(&path) {
            Ok(conn) => Some(conn),
            Err(e) if not_a_database(&e) => {
                set_aside_database(&path, &e.to_string());
                open_writer(&path).inspect_err(|e| tracing::warn!(error = %e, "history database unavailable; nothing is recorded")).ok()
            }
            Err(e) => {
                tracing::warn!(error = %e, path = %path.display(), "history database unavailable; nothing is recorded");
                None
            }
        };
        let mut conn = conn;
        if let Some(conn) = &mut conn
            && let Err(e) = revs::prepare(conn)
        {
            tracing::warn!(error = %e, "history changes could not be numbered; phones may get the whole history again");
        }
        let len = conn.as_ref().map_or(0, |conn| count(conn).unwrap_or(0));
        Self { path, conn: conn.map(Mutex::new), len }
    }

    /// The newest `n` entries, newest first.
    pub fn recent(&self, n: usize) -> Vec<HistoryEntry> {
        let Some(conn) = &self.conn else { return Vec::new() };
        let rows = conn
            .lock()
            .prepare_cached(&format!("SELECT json FROM entries {NEWEST_FIRST} LIMIT ?1"))
            .and_then(|mut stmt| stmt.query_map([int(n)], |row| row.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>());
        match rows {
            Ok(rows) => rows.iter().filter_map(|json| decode(json)).collect(),
            Err(e) => {
                tracing::warn!(error = %e, "history unreadable");
                Vec::new()
            }
        }
    }

    /// Entries in the history.
    pub fn total(&self) -> usize {
        self.len
    }

    /// Add `entry` as the newest; drops the oldest past `keep` (at most [`MAX_ENTRIES`]).
    pub fn push(&mut self, entry: HistoryEntry, keep: usize) -> Result<(), CoreError> {
        let len = self.len + 1;
        let tx = self.conn_mut()?.transaction().map_err(err)?;
        insert(&tx, &entry, false).map_err(err)?;
        revs::record(&tx, &entry.id.to_string(), false).map_err(err)?;
        let dropped = drop_oldest(&tx, len.saturating_sub(keep.min(MAX_ENTRIES))).map_err(err)?;
        revs::prune(&tx).map_err(err)?;
        tx.commit().map_err(err)?;
        self.len = len.saturating_sub(dropped);
        Ok(())
    }

    /// A record a phone uploaded (docs/dictation.md §20.8): written like [`Self::push`], unless
    /// its id is in the history already or was uploaded before (and perhaps deleted since, which
    /// must not bring it back). `Ok(false)` when it was not written.
    pub fn insert_received(&mut self, entry: HistoryEntry, keep: usize, now_ms: u64) -> Result<bool, CoreError> {
        let len = self.len + 1;
        let tx = self.conn_mut()?.transaction().map_err(err)?;
        let id = entry.id.to_string();
        let known: bool = tx
            .query_row("SELECT EXISTS (SELECT 1 FROM phone_received WHERE id = ?1) OR EXISTS (SELECT 1 FROM entries WHERE id = ?1)", [&id], |row| row.get(0))
            .map_err(err)?;
        if known {
            return Ok(false);
        }
        insert(&tx, &entry, false).map_err(err)?;
        revs::record(&tx, &id, false).map_err(err)?;
        tx.execute("INSERT INTO phone_received (id, at_ms) VALUES (?1, ?2)", params![id, int(now_ms)]).map_err(err)?;
        tx.execute("DELETE FROM phone_received WHERE at_ms < ?1", [int(now_ms.saturating_sub(RECEIVED_KEEP_MS))]).map_err(err)?;
        let dropped = drop_oldest(&tx, len.saturating_sub(keep.min(MAX_ENTRIES))).map_err(err)?;
        revs::prune(&tx).map_err(err)?;
        tx.commit().map_err(err)?;
        self.len = len.saturating_sub(dropped);
        Ok(true)
    }

    /// Drop the oldest entries past `keep`; `Ok(true)` when anything was dropped.
    pub fn retain_newest(&mut self, keep: usize) -> Result<bool, CoreError> {
        let excess = self.len.saturating_sub(keep.min(MAX_ENTRIES));
        if excess == 0 {
            return Ok(false);
        }
        let tx = self.conn_mut()?.transaction().map_err(err)?;
        let dropped = drop_oldest(&tx, excess).map_err(err)?;
        revs::prune(&tx).map_err(err)?;
        tx.commit().map_err(err)?;
        self.len = self.len.saturating_sub(dropped);
        Ok(dropped > 0)
    }

    /// Remove one entry; `Ok(false)` when it was not there.
    pub fn delete(&mut self, id: Uuid) -> Result<bool, CoreError> {
        let key = id.to_string();
        let tx = self.conn_mut()?.transaction().map_err(err)?;
        let removed = tx.execute("DELETE FROM entries WHERE id = ?1", [&key]).map_err(err)?;
        if removed > 0 {
            revs::record(&tx, &key, true).map_err(err)?;
            revs::prune(&tx).map_err(err)?;
        }
        tx.commit().map_err(err)?;
        self.len = self.len.saturating_sub(removed);
        Ok(removed > 0)
    }

    /// Remove everything.
    pub fn clear(&mut self) -> Result<(), CoreError> {
        let tx = self.conn_mut()?.transaction().map_err(err)?;
        tx.execute_batch("DELETE FROM hits; DELETE FROM entries;").map_err(err)?;
        revs::record_clear(&tx).map_err(err)?;
        tx.commit().map_err(err)?;
        self.len = 0;
        Ok(())
    }

    /// The changes after `since` for a phone whose copy came from the history `epoch`, up to
    /// `budget` encoded bytes (docs/dictation.md §20.8).
    pub fn changes_since(&self, epoch: Option<Uuid>, since: u64, budget: usize) -> Result<ChangeBatch, CoreError> {
        let conn = self.conn.as_ref().ok_or_else(|| CoreError::History(format!("{} cannot be opened", self.path.display())))?;
        revs::changes_since(&conn.lock(), epoch, since, budget).map_err(err)
    }

    /// The newest change's revision (`0` when the history cannot be opened).
    pub fn head(&self) -> u64 {
        self.conn.as_ref().and_then(|conn| revs::position(&conn.lock()).ok()).map_or(0, |(_, head, _)| head)
    }

    /// On a phone: the next batch of its own records no computer has confirmed (docs/dictation.md
    /// §20.8).
    pub fn outbox(&self, budget: usize, max_records: usize, max_entry: usize) -> Result<Outbox, CoreError> {
        let conn = self.conn.as_ref().ok_or_else(|| CoreError::History(format!("{} cannot be opened", self.path.display())))?;
        revs::outbox(&conn.lock(), budget, max_records, max_entry).map_err(err)
    }

    /// On a phone: `computer` (its key in hex) confirmed these records; the number recorded.
    pub fn mark_uploaded(&mut self, ids: &[Uuid], computer: &str, at_ms: u64) -> Result<usize, CoreError> {
        revs::mark_uploaded(self.conn_mut()?, ids, computer, at_ms).map_err(err)
    }

    /// Flag / unflag; `Ok(false)` when the id is unknown.
    pub fn star(&mut self, id: Uuid, starred: bool) -> Result<bool, CoreError> {
        let conn = self.conn_mut()?;
        let key = id.to_string();
        let json: Option<String> = conn.query_row("SELECT json FROM entries WHERE id = ?1", [&key], |row| row.get(0)).optional().map_err(err)?;
        let Some(mut entry) = json.as_deref().and_then(decode) else { return Ok(false) };
        if entry.starred == starred {
            return Ok(true);
        }
        entry.starred = starred;
        let json = serde_json::to_string(&entry).map_err(err)?;
        let tx = conn.transaction().map_err(err)?;
        tx.execute("UPDATE entries SET starred = ?2, json = ?3 WHERE id = ?1", params![key, starred, json]).map_err(err)?;
        revs::record(&tx, &key, false).map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(true)
    }

    /// One entry by id.
    pub fn get(&self, id: Uuid) -> Result<Option<HistoryEntry>, CoreError> {
        let Some(conn) = &self.conn else { return Err(CoreError::History(format!("{} cannot be opened", self.path.display()))) };
        let json: Option<String> =
            conn.lock().query_row("SELECT json FROM entries WHERE id = ?1", [id.to_string()], |row| row.get(0)).optional().map_err(err)?;
        Ok(json.as_deref().and_then(decode))
    }

    /// Keep 用 AI 预设处理's result with the entry (docs/dictation.md §22), replacing an earlier
    /// one; the search finds its text too. `Ok(false)` when the id is unknown.
    pub fn set_processed(&mut self, id: Uuid, processed: ProcessedText) -> Result<bool, CoreError> {
        let conn = self.conn_mut()?;
        let key = id.to_string();
        let json: Option<String> = conn.query_row("SELECT json FROM entries WHERE id = ?1", [&key], |row| row.get(0)).optional().map_err(err)?;
        let Some(mut entry) = json.as_deref().and_then(decode) else { return Ok(false) };
        entry.processed = Some(Box::new(processed));
        let json = serde_json::to_string(&entry).map_err(err)?;
        let tx = conn.transaction().map_err(err)?;
        tx.execute("UPDATE entries SET search = ?2, json = ?3 WHERE id = ?1", params![key, derived::search_text(&entry), json]).map_err(err)?;
        revs::record(&tx, &key, false).map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(true)
    }

    fn conn_mut(&mut self) -> Result<&mut Connection, CoreError> {
        let path = &self.path;
        self.conn.as_mut().map(Mutex::get_mut).ok_or_else(|| CoreError::History(format!("{} cannot be opened", path.display())))
    }
}

fn err(e: impl std::fmt::Display) -> CoreError {
    CoreError::History(e.to_string())
}

/// SQLite integers are signed; nothing the history stores comes near the limit.
pub(super) fn int<T: TryInto<i64>>(value: T) -> i64 {
    value.try_into().unwrap_or(i64::MAX)
}

/// An entry's JSON as stored; a row that does not parse is left out (and logged).
pub(super) fn decode(json: &str) -> Option<HistoryEntry> {
    serde_json::from_str(json).inspect_err(|e| tracing::warn!(error = %e, "history entry unreadable; left out")).ok()
}

fn count(conn: &Connection) -> rusqlite::Result<usize> {
    conn.query_row("SELECT COUNT(*) FROM entries", [], |row| row.get::<_, i64>(0)).map(|n| usize::try_from(n).unwrap_or(0))
}

/// Delete the `n` oldest entries (their hits and `uploaded` rows go with them), each with its
/// deletion record; the number deleted.
fn drop_oldest(conn: &Connection, n: usize) -> rusqlite::Result<usize> {
    if n == 0 {
        return Ok(0);
    }
    let ids: Vec<String> = {
        let mut stmt = conn.prepare_cached("SELECT id FROM entries ORDER BY at_ms ASC, rowid ASC LIMIT ?1")?;
        stmt.query_map([int(n)], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?
    };
    let mut delete = conn.prepare_cached("DELETE FROM entries WHERE id = ?1")?;
    for id in &ids {
        delete.execute([id])?;
        revs::record(conn, id, true)?;
    }
    Ok(ids.len())
}

/// Write one entry and its hits. `skip_duplicate`: an id already there is left as it is (the
/// import); otherwise it is an error.
pub(crate) fn insert(conn: &Connection, entry: &HistoryEntry, skip_duplicate: bool) -> rusqlite::Result<()> {
    let json = serde_json::to_string(entry).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let id = entry.id.to_string();
    let verb = if skip_duplicate { "INSERT OR IGNORE" } else { "INSERT" };
    let written = conn.execute(
        &format!(
            "{verb} INTO entries (id, at_ms, starred, kind, outcome, app_id, scene_id, spoken_ms, raw_chars, corrected_chars, latency_ms, counts_for_stats, search, json) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)"
        ),
        params![
            id,
            int(entry.at_ms),
            entry.starred,
            derived::kind_name(entry.kind),
            derived::outcome_name(&entry.outcome),
            entry.app.as_ref().map(|a| a.id.as_str()),
            entry.scene.as_ref().map(|s| s.id.to_string()),
            int(entry.duration_ms),
            int(derived::raw_chars(entry)),
            int(derived::corrected_chars(&entry.raw_text, &entry.text)),
            int(derived::latency_ms(entry)),
            derived::counts_for_stats(entry),
            derived::search_text(entry),
            json,
        ],
    )?;
    if written == 0 {
        return Ok(());
    }
    if let Some(hits) = &entry.vocabulary {
        let mut stmt = conn.prepare_cached("INSERT INTO hits (entry_id, list, item_id, count) VALUES (?1, ?2, ?3, ?4)")?;
        for (list, hits) in [("corrections", &hits.corrections), ("rules", &hits.rules)] {
            for hit in hits {
                stmt.execute(params![id, list, hit.id.to_string(), hit.count])?;
            }
        }
    }
    Ok(())
}

/// The core's connection: WAL, so the bridge's reader reads while it writes.
fn open_writer(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    let mode: String = conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        tracing::warn!(mode, "history database is not in WAL mode; reads wait for writes");
    }
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.execute_batch(SCHEMA)?;
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(conn)
}

fn not_a_database(e: &rusqlite::Error) -> bool {
    matches!(e, rusqlite::Error::SqliteFailure(f, _) if matches!(f.code, ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt))
}

/// `history.sqlite3` (and its `-wal` / `-shm`) → `history.sqlite3.corrupt`.
fn set_aside_database(path: &Path, why: &str) {
    let aside = path.with_file_name(format!("{HISTORY_DB_FILE_NAME}.corrupt"));
    match std::fs::rename(path, &aside) {
        Ok(()) => tracing::warn!(%why, moved_to = %aside.display(), "history database unusable; moved aside, a new one is started"),
        Err(e) => tracing::warn!(%why, error = %e, "history database unusable and could not be moved aside"),
    }
    for side in ["-wal", "-shm"] {
        let from = path.with_file_name(format!("{HISTORY_DB_FILE_NAME}{side}"));
        if from.exists() {
            let _ = std::fs::rename(&from, aside.with_file_name(format!("{HISTORY_DB_FILE_NAME}.corrupt{side}")));
        }
    }
}

/// `history.json` → `history.json.corrupt`: a file that cannot be imported, kept for the user.
fn set_aside_json(path: &Path, why: &str) {
    let aside = path.with_file_name(format!("{HISTORY_FILE_NAME}.corrupt"));
    match std::fs::rename(path, &aside) {
        Ok(()) => tracing::warn!(%why, moved_to = %aside.display(), "history.json not imported; moved aside"),
        Err(e) => tracing::warn!(%why, error = %e, "history.json not imported and could not be moved aside"),
    }
}

/// Step 1.
fn remove_unfinished_import(dir: &Path) {
    let importing = dir.join(IMPORTING_FILE_NAME);
    match std::fs::remove_file(&importing) {
        Ok(()) => tracing::info!("history: an import that stopped before it finished was removed; it runs again"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!(error = %e, path = %importing.display(), "history: an unfinished import could not be removed"),
    }
    let _ = std::fs::remove_file(dir.join(format!("{IMPORTING_FILE_NAME}-journal")));
}

/// Steps 2 to 4.
fn import_legacy(dir: &Path, db: &Path) {
    let json = dir.join(HISTORY_FILE_NAME);
    let bytes = match std::fs::read(&json) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) => {
            tracing::warn!(error = %e, path = %json.display(), "history.json unreadable; not imported");
            return;
        }
    };
    let digest = hex::encode(Sha256::digest(&bytes));
    if db.exists() {
        if imported_digest(db).as_deref() == Some(digest.as_str()) {
            retire(&json);
        } else {
            set_aside_json(&json, "the database already holds a history");
        }
        return;
    }
    let entries = match serde_json::from_slice::<HistoryFile>(&bytes) {
        Ok(file) if file.schema == HISTORY_SCHEMA => file.entries,
        Ok(file) => return set_aside_json(&json, &format!("unsupported history schema {}", file.schema)),
        Err(e) => return set_aside_json(&json, &e.to_string()),
    };
    let importing = dir.join(IMPORTING_FILE_NAME);
    if let Err(e) = write_import(&importing, &entries, &digest) {
        tracing::warn!(error = %e, "history import failed; it runs again at the next start");
        let _ = std::fs::remove_file(&importing);
        return;
    }
    if let Err(e) = std::fs::rename(&importing, db) {
        tracing::warn!(error = %e, "history import not put in place; it runs again at the next start");
        return;
    }
    tracing::info!(entries = entries.len(), "history.json imported into the history database");
    retire(&json);
}

/// Step 2: the whole file in one transaction, then closed (no WAL: it is renamed as one file).
fn write_import(path: &Path, entries: &[HistoryEntry], digest: &str) -> rusqlite::Result<()> {
    let mut conn = Connection::open(path)?;
    conn.execute_batch(SCHEMA)?;
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    let tx = conn.transaction()?;
    // Oldest first, so two entries of the same millisecond keep the file's order.
    for entry in entries.iter().take(MAX_ENTRIES).rev() {
        insert(&tx, entry, true)?;
    }
    tx.execute("INSERT INTO meta (key, value) VALUES (?1, ?2)", params![IMPORTED_DIGEST_KEY, digest])?;
    tx.commit()?;
    conn.close().map_err(|(_, e)| e)
}

/// The digest step 2 wrote, `None` when the database has none (or cannot be read).
fn imported_digest(db: &Path) -> Option<String> {
    let conn = Connection::open(db).ok()?;
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [IMPORTED_DIGEST_KEY], |row| row.get(0)).optional().ok().flatten()
}

/// Step 3.
fn retire(json: &Path) {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let kept = json.with_file_name(format!("{HISTORY_FILE_NAME}.imported-{secs}"));
    match std::fs::rename(json, &kept) {
        Ok(()) => tracing::info!(kept = %kept.display(), "history.json kept under a new name"),
        Err(e) => tracing::warn!(error = %e, "history.json imported but not renamed; it is recognised next time"),
    }
}
