//! The history's numbered changes, for the phones that keep a copy of it (docs/dictation.md
//! §20.8), and the phone's side: which of its own records still have to go to a computer.
//!
//! Every write takes a new revision in the same transaction. `sync_revs` holds each entry's
//! latest revision and a deletion record for each entry that left. A phone asks for the changes
//! after the revision it applied. Deletion records older than every live entry are pruned; the
//! `floor` says which revisions may be gone, and a phone behind it gets the whole history again.
//! Invariant: every live entry's revision is above the floor.

use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use super::store::{decode, int};
use super::{EntryOrigin, HistoryEntry, Outcome, ProcessedText};
use crate::presets::PresetRef;
use crate::scenes::{AppRef, SceneRef};
use crate::sync::{MAX_BATCH_DELETES, MAX_BATCH_UPSERTS, MAX_ENTRY_BYTES, cbor_len};

/// `meta` key: this database's identity, a random UUID given when it is created.
pub(super) const EPOCH_KEY: &str = "sync_epoch";
/// `meta` key: the revision the next change takes, from 1.
pub(super) const NEXT_REV_KEY: &str = "sync_next_rev";
/// `meta` key: deletions at or below this revision may no longer be recorded.
pub(super) const FLOOR_KEY: &str = "sync_floor";
/// What one deletion adds to a batch (a UUID and its framing).
const DELETE_COST: usize = 40;
/// `text`, `raw_text` and `processed.text` of a bounded projection are cut to this many
/// characters.
pub const SHORTENED_TEXT_CHARS: usize = 100_000;
/// Every other string of a bounded projection is cut to this many characters.
pub const SHORTENED_FIELD_CHARS: usize = 256;
/// Uploaded records are remembered this long, so one the user deleted is not written back when
/// the phone sends it again.
pub(super) const RECEIVED_KEEP_MS: u64 = 90 * 24 * 60 * 60 * 1000;

/// A batch of changes for one phone (docs/dictation.md §20.8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeBatch {
    /// This history's identity.
    pub epoch: Uuid,
    /// The phone's copy is replaced: it was empty, of another history, or too far behind.
    pub reset: bool,
    /// The newest revision.
    pub head: u64,
    /// The revision this batch reaches; the next request asks for what follows it.
    pub to: u64,
    /// Entries written or changed, `segments` left out.
    pub upserts: Vec<HistoryEntry>,
    /// Entries that left the history.
    pub deletes: Vec<Uuid>,
    /// Entries of `upserts` sent as their bounded projection.
    pub shortened: Vec<Uuid>,
    /// More changes follow.
    pub more: bool,
}

impl ChangeBatch {
    fn is_empty(&self) -> bool {
        self.upserts.is_empty() && self.deletes.is_empty()
    }
}

/// A phone's records that still have to go to a computer, oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outbox {
    /// The next batch, `segments` left out.
    pub records: Vec<HistoryEntry>,
    /// Records too large to upload, met while choosing the batch.
    pub too_large: Vec<Uuid>,
}

fn meta(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| row.get(0)).optional()
}

fn set_meta(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute("INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value", params![key, value]).map(drop)
}

fn meta_u64(conn: &Connection, key: &str) -> rusqlite::Result<u64> {
    Ok(meta(conn, key)?.and_then(|v| v.parse().ok()).unwrap_or(0))
}

/// Give the database its identity and number the entries that have no revision yet (a database
/// from before, a fresh import), oldest first. One transaction.
pub(super) fn prepare(conn: &mut Connection) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    if meta(&tx, EPOCH_KEY)?.is_none() {
        set_meta(&tx, EPOCH_KEY, &Uuid::new_v4().to_string())?;
    }
    if meta(&tx, NEXT_REV_KEY)?.is_none() {
        set_meta(&tx, NEXT_REV_KEY, "1")?;
    }
    if meta(&tx, FLOOR_KEY)?.is_none() {
        set_meta(&tx, FLOOR_KEY, "0")?;
    }
    let missing: Vec<String> = {
        let mut stmt = tx.prepare("SELECT id FROM entries WHERE id NOT IN (SELECT id FROM sync_revs WHERE deleted = 0) ORDER BY at_ms ASC, rowid ASC")?;
        stmt.query_map([], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?
    };
    for id in &missing {
        record(&tx, id, false)?;
    }
    tx.commit()
}

/// The revision the next change takes.
fn take_rev(conn: &Connection) -> rusqlite::Result<u64> {
    let rev = meta_u64(conn, NEXT_REV_KEY)?.max(1);
    set_meta(conn, NEXT_REV_KEY, &(rev + 1).to_string())?;
    Ok(rev)
}

/// `id` was written or changed (`deleted == false`), or it left the history.
pub(super) fn record(conn: &Connection, id: &str, deleted: bool) -> rusqlite::Result<()> {
    let rev = take_rev(conn)?;
    conn.execute(
        "INSERT INTO sync_revs (id, rev, deleted) VALUES (?1, ?2, ?3) ON CONFLICT (id) DO UPDATE SET rev = excluded.rev, deleted = excluded.deleted",
        params![id, int(rev), deleted],
    )?;
    Ok(())
}

/// Everything left: no record survives, and the floor moves up to a new revision, so every phone
/// gets the (empty) history again.
pub(super) fn record_clear(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM sync_revs", [])?;
    let rev = take_rev(conn)?;
    set_meta(conn, FLOOR_KEY, &rev.to_string())
}

/// Drop the deletion records older than every live entry (all of them when none is left); the
/// floor rises to the newest one dropped. A phone at or above the floor had them already; one
/// below it gets a reset and does not need them.
pub(super) fn prune(conn: &Connection) -> rusqlite::Result<()> {
    let oldest_live: Option<i64> = conn.query_row("SELECT MIN(rev) FROM sync_revs WHERE deleted = 0", [], |row| row.get(0))?;
    let bound = oldest_live.unwrap_or(i64::MAX);
    let newest_gone: Option<i64> = conn.query_row("SELECT MAX(rev) FROM sync_revs WHERE deleted = 1 AND rev < ?1", [bound], |row| row.get(0))?;
    let Some(newest_gone) = newest_gone else { return Ok(()) };
    conn.execute("DELETE FROM sync_revs WHERE deleted = 1 AND rev < ?1", [bound])?;
    let floor = meta_u64(conn, FLOOR_KEY)?.max(u64::try_from(newest_gone).unwrap_or(0));
    set_meta(conn, FLOOR_KEY, &floor.to_string())
}

/// This database's identity, newest revision and floor.
pub(super) fn position(conn: &Connection) -> rusqlite::Result<(Uuid, u64, u64)> {
    let epoch = meta(conn, EPOCH_KEY)?.and_then(|v| v.parse().ok()).unwrap_or_else(Uuid::nil);
    Ok((epoch, meta_u64(conn, NEXT_REV_KEY)?.saturating_sub(1), meta_u64(conn, FLOOR_KEY)?))
}

/// The changes after `since` for a phone whose copy came from `epoch`, up to `budget` encoded
/// bytes (at least one change).
pub(super) fn changes_since(conn: &Connection, epoch: Option<Uuid>, since: u64, budget: usize) -> rusqlite::Result<ChangeBatch> {
    let (mine, head, floor) = position(conn)?;
    let reset = since == 0 || epoch != Some(mine) || since < floor || since > head;
    // A reset starts from nothing and takes the live entries only: `since` may be another
    // history's. Their revisions are above the floor, so the next request does not reset again.
    let mut stmt = if reset {
        conn.prepare("SELECT s.rev, s.id, s.deleted, e.json FROM sync_revs s JOIN entries e ON e.id = s.id WHERE s.deleted = 0 ORDER BY s.rev ASC")?
    } else {
        conn.prepare("SELECT s.rev, s.id, s.deleted, e.json FROM sync_revs s LEFT JOIN entries e ON e.id = s.id WHERE s.rev > ?1 ORDER BY s.rev ASC")?
    };
    let mut rows = if reset { stmt.query([])? } else { stmt.query([int(since)])? };
    let mut batch = ChangeBatch {
        epoch: mine,
        reset,
        head,
        to: if reset { 0 } else { since },
        upserts: Vec::new(),
        deletes: Vec::new(),
        shortened: Vec::new(),
        more: false,
    };
    let mut used = 0usize;
    while let Some(row) = rows.next()? {
        let rev = u64::try_from(row.get::<_, i64>(0)?).unwrap_or(0);
        let id: String = row.get(1)?;
        let deleted: bool = row.get(2)?;
        let json: Option<String> = row.get(3)?;
        let Ok(uuid) = id.parse::<Uuid>() else {
            batch.to = rev;
            continue;
        };
        match json.filter(|_| !deleted) {
            None => {
                if batch.deletes.len() >= MAX_BATCH_DELETES || (used + DELETE_COST > budget && !batch.is_empty()) {
                    batch.more = true;
                    break;
                }
                batch.deletes.push(uuid);
                used += DELETE_COST;
            }
            Some(json) => {
                let Some(mut entry) = decode(&json) else {
                    batch.to = rev;
                    continue;
                };
                entry.segments = None;
                let mut size = cbor_len(&entry);
                let shortened = size > MAX_ENTRY_BYTES;
                if shortened {
                    entry = bounded(&entry);
                    size = cbor_len(&entry);
                }
                if batch.upserts.len() >= MAX_BATCH_UPSERTS || (used.saturating_add(size) > budget && !batch.is_empty()) {
                    batch.more = true;
                    break;
                }
                if shortened {
                    batch.shortened.push(uuid);
                }
                batch.upserts.push(entry);
                used = used.saturating_add(size);
            }
        }
        batch.to = rev;
    }
    if !batch.more {
        batch.to = head;
    }
    Ok(batch)
}

/// The entry as a batch may carry it whatever its size (docs/dictation.md §20.8): the three texts
/// cut to [`SHORTENED_TEXT_CHARS`], every other string to [`SHORTENED_FIELD_CHARS`], the
/// segments, hits and voice edit left out. At most about 1.3 MB.
pub fn bounded(entry: &HistoryEntry) -> HistoryEntry {
    let long = |s: &str| clip(s, SHORTENED_TEXT_CHARS);
    let short = |s: &str| clip(s, SHORTENED_FIELD_CHARS);
    HistoryEntry {
        raw_text: long(&entry.raw_text),
        text: long(&entry.text),
        asr_model: short(&entry.asr_model),
        refine_model: entry.refine_model.as_deref().map(short),
        outcome: match &entry.outcome {
            Outcome::Inserted { via } => Outcome::Inserted { via: *via },
            Outcome::Clipboard { reason, code } => Outcome::Clipboard { reason: short(reason), code: *code },
            Outcome::Failed { reason } => Outcome::Failed { reason: short(reason) },
        },
        segments: None,
        live_error: entry.live_error.as_deref().map(short),
        vocabulary: None,
        edit: None,
        app: entry.app.as_ref().map(|a| AppRef { id: short(&a.id), name: short(&a.name) }),
        scene: entry.scene.as_ref().map(|s| SceneRef { id: s.id, name: short(&s.name), builtin: s.builtin }),
        preset: entry.preset.as_ref().map(|p| PresetRef { id: p.id, name: short(&p.name) }),
        origin: entry.origin.as_ref().map(|o| EntryOrigin { device: short(&o.device), kind: o.kind }),
        processed: entry
            .processed
            .as_ref()
            .map(|p| Box::new(ProcessedText { text: long(&p.text), preset: PresetRef { id: p.preset.id, name: short(&p.preset.name) }, at_ms: p.at_ms })),
        ..shallow(entry)
    }
}

/// The scalar fields of `entry`, every string and list empty (so the projection never copies a
/// huge field only to drop it).
fn shallow(entry: &HistoryEntry) -> HistoryEntry {
    HistoryEntry {
        id: entry.id,
        at_ms: entry.at_ms,
        raw_text: String::new(),
        text: String::new(),
        refined: entry.refined,
        asr_model: String::new(),
        refine_model: None,
        duration_ms: entry.duration_ms,
        asr_ms: entry.asr_ms,
        refine_ms: entry.refine_ms,
        outcome: Outcome::Failed { reason: String::new() },
        starred: entry.starred,
        mode: entry.mode,
        segments: None,
        live_error: None,
        vocabulary: None,
        kind: entry.kind,
        edit: None,
        app: None,
        scene: None,
        preset: None,
        origin: None,
        processed: None,
    }
}

fn clip(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        None => text.to_owned(),
        Some(_) => {
            let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
            out.push('…');
            out
        }
    }
}

/// The phone's records no computer has confirmed, oldest first: one batch of at most
/// `max_records` adding up to `budget` encoded bytes (at least one record), passing over records
/// larger than `max_entry` ([`MAX_ENTRY_BYTES`] outside the tests).
pub(super) fn outbox(conn: &Connection, budget: usize, max_records: usize, max_entry: usize) -> rusqlite::Result<Outbox> {
    let mut stmt = conn.prepare(
        "SELECT json FROM entries WHERE json_extract(json, '$.origin') IS NULL AND id NOT IN (SELECT id FROM uploaded) ORDER BY at_ms ASC, rowid ASC",
    )?;
    let mut rows = stmt.query([])?;
    let mut out = Outbox::default();
    let mut used = 0usize;
    while let Some(row) = rows.next()? {
        let json: String = row.get(0)?;
        let Some(mut entry) = decode(&json) else { continue };
        entry.segments = None;
        let size = cbor_len(&entry);
        if size > max_entry {
            out.too_large.push(entry.id);
            continue;
        }
        if out.records.len() >= max_records || (used.saturating_add(size) > budget && !out.records.is_empty()) {
            break;
        }
        used = used.saturating_add(size);
        out.records.push(entry);
    }
    Ok(out)
}

/// A computer confirmed these records. A record the user deleted meanwhile is skipped: the
/// confirmation never leaves a row behind for an entry that is gone.
pub(super) fn mark_uploaded(conn: &mut Connection, ids: &[Uuid], computer: &str, at_ms: u64) -> rusqlite::Result<usize> {
    let tx = conn.transaction()?;
    let mut written = 0;
    {
        let mut stmt =
            tx.prepare_cached("INSERT OR IGNORE INTO uploaded (id, computer, at_ms) SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM entries WHERE id = ?1)")?;
        for id in ids {
            written += stmt.execute(params![id.to_string(), computer, int(at_ms)])?;
        }
    }
    tx.commit()?;
    Ok(written)
}
