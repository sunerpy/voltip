//! The reading side of the history database (docs/dictation.md §4.4): the interface's history
//! queries, answered by the bridge from a read-only connection of its own instead of the core's
//! command channel. WAL lets it read while the core writes; before the core has created the
//! database every query answers empty.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use rusqlite::types::Value;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params, params_from_iter};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::store::{BUSY_TIMEOUT, NEWEST_FIRST, decode, int};
use super::{HISTORY_DB_FILE_NAME, HistoryEntry, derived};
use crate::CoreError;
use crate::scenes::AppRef;

/// The most entries one `history_query` returns.
pub const MAX_QUERY_LIMIT: u32 = 200;
/// The most boundaries one `history_stats` takes: the local midnights from the Monday five weeks
/// before this one to tomorrow (42 days, 43 boundaries).
pub const MAX_STATS_BOUNDARIES: usize = 43;

/// `history_query`: which entries, and which page of them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryQuery {
    /// Entries at or after this instant (a local midnight the page computed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since_ms: Option<u64>,
    /// Starred entries only.
    #[serde(default)]
    pub starred: bool,
    /// Entries whose text was not inserted (left on the clipboard, or failed).
    #[serde(default)]
    pub failed: bool,
    /// Case-insensitive text the entry contains (the fields the page's search always used).
    #[serde(default)]
    pub query: String,
    /// Entries to skip, newest first.
    #[serde(default)]
    pub offset: u32,
    /// Entries to return, 1..=[`MAX_QUERY_LIMIT`].
    pub limit: u32,
}

/// `history_query`'s answer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryPage {
    /// The page, newest first.
    pub entries: Vec<HistoryEntry>,
    /// Entries that match the query, on every page.
    pub matching: u32,
    /// Entries in the history.
    pub total: u32,
}

/// The dictations of one span (docs/dictation.md §4.5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryStatsBucket {
    /// Dictations.
    pub count: u32,
    /// Characters recognised.
    pub raw_chars: u64,
    /// Characters the clean-up changed.
    pub corrected_chars: u64,
    /// Recording time.
    pub spoken_ms: u64,
    /// Recognition plus clean-up, summed.
    pub latency_ms: u64,
}

/// `history_stats`'s answer: one bucket per span between two boundaries, and every dictation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryStats {
    /// `buckets[i]` holds `boundaries[i] <= at_ms < boundaries[i + 1]`.
    pub buckets: Vec<HistoryStatsBucket>,
    /// The whole history.
    pub total: HistoryStatsBucket,
}

/// `history_hits`: how often each dictionary entry and each rule fired in the history.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryHits {
    /// Dictionary entry id → corrections made.
    pub dictionary: BTreeMap<Uuid, u32>,
    /// Rule id → replacements made.
    pub rules: BTreeMap<Uuid, u32>,
}

/// The bridge's connection to `history.sqlite3`, opened read-only at the first query.
#[derive(Debug)]
pub struct HistoryReader {
    path: PathBuf,
    conn: Mutex<Option<Connection>>,
}

impl HistoryReader {
    /// The reader of `data_dir/history.sqlite3`; nothing is opened yet.
    pub fn new(data_dir: &Path) -> Self {
        Self::at(&data_dir.join(HISTORY_DB_FILE_NAME))
    }

    /// The reader of the history database at `path` (a phone's copy of a computer's history,
    /// docs/dictation.md §20.8); nothing is opened yet.
    pub fn at(path: &Path) -> Self {
        Self { path: path.to_path_buf(), conn: Mutex::new(None) }
    }

    /// Run `read` on the connection; `Ok(None)` while there is no database yet.
    fn read<T>(&self, read: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<Option<T>, CoreError> {
        let mut slot = self.conn.lock();
        if slot.is_none() {
            if !self.path.exists() {
                return Ok(None);
            }
            let conn = Connection::open_with_flags(&self.path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(err)?;
            conn.busy_timeout(BUSY_TIMEOUT).map_err(err)?;
            *slot = Some(conn);
        }
        let Some(conn) = slot.as_ref() else { return Ok(None) };
        match read(conn) {
            Ok(value) => Ok(Some(value)),
            // The core creates the tables right after the file.
            Err(rusqlite::Error::SqliteFailure(_, Some(message))) if message.starts_with("no such table") => Ok(None),
            Err(e) => Err(err(e)),
        }
    }

    /// A page of the entries `query` selects, newest first.
    pub fn query(&self, query: &HistoryQuery) -> Result<HistoryPage, CoreError> {
        if !(1..=MAX_QUERY_LIMIT).contains(&query.limit) {
            return Err(CoreError::Invalid(format!("history_query: limit 1–{MAX_QUERY_LIMIT}")));
        }
        let mut filter = String::from(" WHERE 1 = 1");
        let mut args: Vec<Value> = Vec::new();
        if let Some(since) = query.since_ms {
            filter.push_str(" AND at_ms >= ?");
            args.push(Value::Integer(int(since)));
        }
        if query.starred {
            filter.push_str(" AND starred = 1");
        }
        if query.failed {
            filter.push_str(" AND outcome <> 'inserted'");
        }
        if let Some(needle) = derived::search_needle(&query.query) {
            filter.push_str(" AND instr(search, ?) > 0");
            args.push(Value::Text(needle));
        }
        let page = self.read(|conn| {
            let total = count(conn, "SELECT COUNT(*) FROM entries", [])?;
            let matching = count(conn, &format!("SELECT COUNT(*) FROM entries{filter}"), params_from_iter(&args))?;
            let mut page_args = args.clone();
            page_args.extend([Value::Integer(i64::from(query.limit)), Value::Integer(i64::from(query.offset))]);
            let mut stmt = conn.prepare(&format!("SELECT json FROM entries{filter} {NEWEST_FIRST} LIMIT ? OFFSET ?"))?;
            let rows = stmt.query_map(params_from_iter(&page_args), |row| row.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(HistoryPage { entries: rows.iter().filter_map(|json| decode(json)).collect(), matching, total })
        })?;
        Ok(page.unwrap_or_default())
    }

    /// One entry, `None` when it is not (or no longer) in the history.
    pub fn entry(&self, id: Uuid) -> Result<Option<HistoryEntry>, CoreError> {
        let json = self.read(|conn| conn.query_row("SELECT json FROM entries WHERE id = ?1", [id.to_string()], |row| row.get::<_, String>(0)).optional())?;
        Ok(json.flatten().as_deref().and_then(decode))
    }

    /// The dictations between each two of `boundaries` (increasing, 2..=[`MAX_STATS_BOUNDARIES`]),
    /// and in the whole history.
    pub fn stats(&self, boundaries: &[u64]) -> Result<HistoryStats, CoreError> {
        let (Some(&first), Some(&last)) = (boundaries.first(), boundaries.last()) else { return Err(bad_boundaries()) };
        if boundaries.len() < 2 || boundaries.len() > MAX_STATS_BOUNDARIES || boundaries.windows(2).any(|w| w[0] >= w[1]) {
            return Err(bad_boundaries());
        }
        let empty = HistoryStats { buckets: vec![HistoryStatsBucket::default(); boundaries.len() - 1], total: HistoryStatsBucket::default() };
        let stats = self.read(|conn| {
            let mut stats = empty.clone();
            let mut stmt = conn.prepare(
                "SELECT at_ms, raw_chars, corrected_chars, spoken_ms, latency_ms FROM entries WHERE counts_for_stats = 1 AND at_ms >= ?1 AND at_ms < ?2",
            )?;
            let rows = stmt.query_map(params![int(first), int(last)], |row| Ok((row.get::<_, i64>(0)?, bucket_row(row, 1)?)))?;
            for row in rows {
                let (at_ms, one) = row?;
                let at_ms = u64::try_from(at_ms).unwrap_or(0);
                // At least one boundary is ≤ at_ms (the first) and at least one is > it (the last).
                let index = boundaries.partition_point(|b| *b <= at_ms).saturating_sub(1);
                if let Some(bucket) = stats.buckets.get_mut(index) {
                    add(bucket, &one);
                }
            }
            stats.total = conn.query_row(
                "SELECT COUNT(*), COALESCE(SUM(raw_chars), 0), COALESCE(SUM(corrected_chars), 0), COALESCE(SUM(spoken_ms), 0), COALESCE(SUM(latency_ms), 0) \
                 FROM entries WHERE counts_for_stats = 1",
                [],
                |row| {
                    let mut total = bucket_row(row, 1)?;
                    total.count = u32::try_from(row.get::<_, i64>(0)?).unwrap_or(u32::MAX);
                    Ok(total)
                },
            )?;
            Ok(stats)
        })?;
        Ok(stats.unwrap_or(empty))
    }

    /// Hits per dictionary entry and per rule over the whole history.
    pub fn hits(&self) -> Result<HistoryHits, CoreError> {
        let hits = self.read(|conn| {
            let mut out = HistoryHits::default();
            let mut stmt = conn.prepare("SELECT list, item_id, SUM(count) FROM hits GROUP BY list, item_id")?;
            let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?)))?;
            for row in rows {
                let (list, id, n) = row?;
                let (Ok(id), Ok(n)) = (id.parse::<Uuid>(), u32::try_from(n)) else { continue };
                match list.as_str() {
                    "corrections" => out.dictionary.insert(id, n),
                    "rules" => out.rules.insert(id, n),
                    _ => None,
                };
            }
            Ok(out)
        })?;
        Ok(hits.unwrap_or_default())
    }

    /// The applications the history saw, newest first, one per id, at most `limit` — what the
    /// scene editor offers to pick from (docs/dictation.md §18.6).
    pub fn recent_apps(&self, limit: usize) -> Result<Vec<AppRef>, CoreError> {
        let apps = self.read(|conn| {
            let mut out: Vec<AppRef> = Vec::new();
            let mut stmt = conn.prepare(&format!("SELECT json_extract(json, '$.app') FROM entries WHERE app_id IS NOT NULL {NEWEST_FIRST}"))?;
            let mut rows = stmt.query([])?;
            while out.len() < limit {
                let Some(row) = rows.next()? else { break };
                let Some(app) = row.get::<_, Option<String>>(0)?.and_then(|json| serde_json::from_str::<AppRef>(&json).ok()) else { continue };
                if !out.iter().any(|a| a.id == app.id) {
                    out.push(app);
                }
            }
            Ok(out)
        })?;
        Ok(apps.unwrap_or_default())
    }
}

fn err(e: impl std::fmt::Display) -> CoreError {
    CoreError::History(e.to_string())
}

fn bad_boundaries() -> CoreError {
    CoreError::Invalid(format!("history_stats: 2–{MAX_STATS_BOUNDARIES} increasing boundaries"))
}

fn count(conn: &Connection, sql: &str, args: impl rusqlite::Params) -> rusqlite::Result<u32> {
    conn.query_row(sql, args, |row| row.get::<_, i64>(0)).map(|n| u32::try_from(n).unwrap_or(u32::MAX))
}

/// The four sums of a stats row, starting at column `from`; `count` is 1 (one entry).
fn bucket_row(row: &rusqlite::Row<'_>, from: usize) -> rusqlite::Result<HistoryStatsBucket> {
    let at = |i: usize| row.get::<_, i64>(from + i).map(|n| u64::try_from(n).unwrap_or(0));
    Ok(HistoryStatsBucket { count: 1, raw_chars: at(0)?, corrected_chars: at(1)?, spoken_ms: at(2)?, latency_ms: at(3)? })
}

fn add(bucket: &mut HistoryStatsBucket, one: &HistoryStatsBucket) {
    bucket.count = bucket.count.saturating_add(one.count);
    bucket.raw_chars = bucket.raw_chars.saturating_add(one.raw_chars);
    bucket.corrected_chars = bucket.corrected_chars.saturating_add(one.corrected_chars);
    bucket.spoken_ms = bucket.spoken_ms.saturating_add(one.spoken_ms);
    bucket.latency_ms = bucket.latency_ms.saturating_add(one.latency_ms);
}
