//! Writing and reading a backup file (docs/dictation.md §4.6).
//!
//! The writer streams: the four lists come from the caller, the history from a read-only
//! connection to `history.sqlite3`, one row at a time, decoded into a [`HistoryEntry`] and written
//! straight into a buffered file, so twenty thousand entries never sit in memory as text. It writes
//! `<name>.part` beside the chosen file and renames it into place once everything is on disk; a
//! failure leaves the chosen file as it was. One object member per line and one item per line:
//!
//! ```text
//! {
//! "format":"voltip-backup",
//! "version":1,
//! "app_version":"0.1.4",
//! "created_at_ms":1760337000000,
//! "device":"Studio",
//! "platform":"windows",
//! "dictionary":[
//! {"id":"…","term":"Voltip",…}
//! ],
//! "rules":[…],
//! "scenes":[…],
//! "presets":[…],
//! "history":[
//! {"id":"…","at_ms":…,…}
//! ],
//! "counts":{"history":1,"dictionary":1,"rules":0,"scenes":7,"presets":0}
//! }
//! ```
//!
//! The reader takes the whole file (at most [`MAX_BACKUP_BYTES`]) and checks it before anything
//! changes: the format and the version first, so a file of a newer version is named as such rather
//! than as damaged, then every part, then the counts against what the parts hold.

use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use voltip_protocol::Platform;

use super::{BACKUP_EXTENSION, BACKUP_FORMAT, BACKUP_VERSION, Backup, BackupCounts, BackupError, BackupManifest, MAX_BACKUP_BYTES, count};
use crate::history::HistoryEntry;
use crate::presets::CustomPreset;
use crate::scenes::Scene;
use crate::vocabulary::{DictionaryEntry, ReplacementRule};

/// Bytes the writer buffers before each write to the file.
const WRITE_BUFFER: usize = 1 << 20;
/// How much of the start of a file that does not parse is searched for the format's name: a backup
/// names it on its second line.
const HEAD_BYTES: usize = 4096;

/// What a backup is made of besides the history.
#[derive(Clone, Copy, Debug)]
pub struct ExportInput<'a> {
    /// The Voltip version writing it.
    pub app_version: &'a str,
    /// This device's name.
    pub device: &'a str,
    /// This device's system; `None`: the one this build runs on.
    pub platform: Option<Platform>,
    /// Now, in Unix milliseconds.
    pub created_at_ms: u64,
    /// The dictionary, in its order.
    pub dictionary: &'a [DictionaryEntry],
    /// The rules, in execution order.
    pub rules: &'a [ReplacementRule],
    /// The scenes, in matching order.
    pub scenes: &'a [Scene],
    /// The custom presets.
    pub presets: &'a [CustomPreset],
}

/// What [`export`] wrote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExportSummary {
    /// Items per part.
    pub counts: BackupCounts,
    /// The file's size.
    pub bytes: u64,
}

/// The layout of the file as [`parse`] reads it; every part may be missing (an empty list).
#[derive(Deserialize)]
struct BackupFile {
    format: String,
    version: u32,
    #[serde(default)]
    app_version: String,
    #[serde(default)]
    created_at_ms: u64,
    #[serde(default)]
    device: String,
    #[serde(default = "unknown_platform")]
    platform: Platform,
    #[serde(default)]
    history: Vec<HistoryEntry>,
    #[serde(default)]
    dictionary: Vec<DictionaryEntry>,
    #[serde(default)]
    rules: Vec<ReplacementRule>,
    #[serde(default)]
    scenes: Vec<Scene>,
    #[serde(default)]
    presets: Vec<CustomPreset>,
    counts: BackupCounts,
}

fn unknown_platform() -> Platform {
    Platform::Other
}

/// The two fields [`parse`] looks at before the rest: whatever else the file holds is skipped.
#[derive(Deserialize)]
struct Head {
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    version: Option<serde_json::Value>,
}

/// The name the save dialog offers: `name` without what a file system refuses (path separators,
/// `<>:"|?*`, control characters, leading dots and trailing dots or spaces), at most 120
/// characters, with [`BACKUP_EXTENSION`]; `voltip` when nothing is left. The same rules as the
/// history's exports (`history::export::file_name`).
pub fn file_name(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if c.is_control() || "/\\<>:\"|?*".contains(c) { ' ' } else { c }).collect();
    let stem: String = cleaned.trim().trim_start_matches('.').trim_start().chars().take(120).collect();
    let stem = stem.trim_end_matches(['.', ' ']);
    let stem = if stem.is_empty() { "voltip" } else { stem };
    format!("{stem}.{BACKUP_EXTENSION}")
}

/// Where a backup the user saved as `chosen` goes: `chosen` itself, or, when its name has no
/// extension (a save dialog that does not add one) and nothing of that name exists, the same name
/// with [`BACKUP_EXTENSION`], so the open dialog finds it again.
pub fn save_path(chosen: &Path) -> PathBuf {
    if chosen.extension().is_some() || chosen.file_name().is_none() {
        return chosen.to_path_buf();
    }
    let mut named = chosen.as_os_str().to_owned();
    named.push(format!(".{BACKUP_EXTENSION}"));
    let named = PathBuf::from(named);
    if named.exists() { chosen.to_path_buf() } else { named }
}

/// Write a backup of `input` and of the history in `history_db` to `path`, through `<name>.part`
/// beside it: `path` changes only once the whole file is on disk. A missing database is an empty
/// history.
pub fn export(path: &Path, history_db: &Path, input: &ExportInput<'_>) -> Result<ExportSummary, BackupError> {
    let part = partial_path(path);
    let counts = match write_file(&part, history_db, input) {
        Ok(counts) => counts,
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            return Err(e);
        }
    };
    if let Err(e) = std::fs::rename(&part, path) {
        let _ = std::fs::remove_file(&part);
        return Err(write_err(e));
    }
    let bytes = std::fs::metadata(path).map(|m| m.len()).map_err(write_err)?;
    Ok(ExportSummary { counts, bytes })
}

/// `<name>.part` beside `path`.
fn partial_path(path: &Path) -> PathBuf {
    match path.file_name() {
        Some(name) => {
            let mut part = name.to_owned();
            part.push(".part");
            path.with_file_name(part)
        }
        None => path.with_extension("part"),
    }
}

fn write_file(part: &Path, history_db: &Path, input: &ExportInput<'_>) -> Result<BackupCounts, BackupError> {
    let file = std::fs::File::create(part).map_err(write_err)?;
    let mut out = BufWriter::with_capacity(WRITE_BUFFER, file);
    let counts = write_backup(&mut out, history_db, input)?;
    let file = out.into_inner().map_err(|e| write_err(e.into_error()))?;
    file.sync_all().map_err(write_err)?;
    Ok(counts)
}

/// The whole backup into `out`, in the layout of the module docs; the counts it wrote.
pub(super) fn write_backup(out: &mut impl Write, history_db: &Path, input: &ExportInput<'_>) -> Result<BackupCounts, BackupError> {
    out.write_all(b"{\n").map_err(write_err)?;
    member(out, "format", BACKUP_FORMAT)?;
    member(out, "version", &BACKUP_VERSION)?;
    member(out, "app_version", input.app_version)?;
    member(out, "created_at_ms", &input.created_at_ms)?;
    member(out, "device", input.device)?;
    member(out, "platform", &input.platform.unwrap_or_else(Platform::current))?;
    let dictionary = list(out, "dictionary", input.dictionary)?;
    let rules = list(out, "rules", input.rules)?;
    let scenes = list(out, "scenes", input.scenes)?;
    let presets = list(out, "presets", input.presets)?;
    let history = history(out, history_db)?;
    let counts = BackupCounts { history, dictionary, rules, scenes, presets };
    out.write_all(b"\"counts\":").map_err(write_err)?;
    serde_json::to_writer(&mut *out, &counts).map_err(json_write_err)?;
    out.write_all(b"\n}\n").map_err(write_err)?;
    Ok(counts)
}

/// `"name":value,` and a line end.
fn member<T: Serialize + ?Sized>(out: &mut impl Write, name: &str, value: &T) -> Result<(), BackupError> {
    write!(out, "\"{name}\":").map_err(write_err)?;
    serde_json::to_writer(&mut *out, value).map_err(json_write_err)?;
    out.write_all(b",\n").map_err(write_err)
}

/// `"name":[` and one item per line; the items written.
fn list<T: Serialize>(out: &mut impl Write, name: &str, items: &[T]) -> Result<u32, BackupError> {
    write!(out, "\"{name}\":[").map_err(write_err)?;
    for (i, item) in items.iter().enumerate() {
        out.write_all(if i == 0 { b"\n" } else { b",\n" }).map_err(write_err)?;
        serde_json::to_writer(&mut *out, item).map_err(json_write_err)?;
    }
    out.write_all(b"\n],\n").map_err(write_err)?;
    Ok(count(items.len()))
}

/// `"history":[` and every entry of `db`, newest first (the history's own order), one per line. A
/// row that does not decode is left out (and logged), as the history's own reads leave it out.
fn history(out: &mut impl Write, db: &Path) -> Result<u32, BackupError> {
    out.write_all(b"\"history\":[").map_err(write_err)?;
    let mut written = 0usize;
    if db.exists() {
        let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(history_err)?;
        conn.busy_timeout(crate::history::BUSY_TIMEOUT).map_err(history_err)?;
        // One statement reads one snapshot of the database: the core may write meanwhile (WAL).
        match conn.prepare("SELECT json FROM entries ORDER BY at_ms DESC, rowid DESC") {
            Ok(mut stmt) => {
                let mut rows = stmt.query([]).map_err(history_err)?;
                while let Some(row) = rows.next().map_err(history_err)? {
                    let value = row.get_ref(0).map_err(history_err)?;
                    let json = value.as_str().map_err(|e| BackupError::History(e.to_string()))?;
                    let entry: HistoryEntry = match serde_json::from_str(json) {
                        Ok(entry) => entry,
                        Err(e) => {
                            tracing::warn!(error = %e, "history entry unreadable; left out of the backup");
                            continue;
                        }
                    };
                    out.write_all(if written == 0 { b"\n" } else { b",\n" }).map_err(write_err)?;
                    serde_json::to_writer(&mut *out, &entry).map_err(json_write_err)?;
                    written += 1;
                }
            }
            // The core creates the tables right after the file.
            Err(rusqlite::Error::SqliteFailure(_, Some(message))) if message.starts_with("no such table") => {}
            Err(e) => return Err(history_err(e)),
        }
    }
    out.write_all(b"\n],\n").map_err(write_err)?;
    Ok(count(written))
}

fn write_err(e: std::io::Error) -> BackupError {
    BackupError::Write(e.to_string())
}

/// A serializer error: always the writer's (the parts are plain data).
fn json_write_err(e: serde_json::Error) -> BackupError {
    BackupError::Write(e.to_string())
}

fn history_err(e: rusqlite::Error) -> BackupError {
    BackupError::History(e.to_string())
}

/// Read and check the backup at `path` ([`parse`]); nothing anywhere changes.
pub fn read(path: &Path) -> Result<Backup, BackupError> {
    let unreadable = |e: std::io::Error| BackupError::Unreadable(e.to_string());
    let meta = std::fs::metadata(path).map_err(unreadable)?;
    if !meta.is_file() {
        return Err(BackupError::Unreadable("not a file".into()));
    }
    if meta.len() > MAX_BACKUP_BYTES {
        return Err(BackupError::TooLarge);
    }
    let file = std::fs::File::open(path).map_err(unreadable)?;
    let mut bytes = Vec::with_capacity(usize::try_from(meta.len()).unwrap_or(0));
    // The file may have grown since its size was read.
    file.take(MAX_BACKUP_BYTES + 1).read_to_end(&mut bytes).map_err(unreadable)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_BACKUP_BYTES {
        return Err(BackupError::TooLarge);
    }
    parse(&bytes)
}

/// Check a backup's bytes and read it: a Voltip backup (else [`BackupError::NotBackup`]) of a
/// version this build reads (else [`BackupError::TooNew`]), every part parsing as its stored
/// structs and the counts matching them (else [`BackupError::Damaged`]). A byte-order mark an
/// editor may have added is skipped.
pub fn parse(bytes: &[u8]) -> Result<Backup, BackupError> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let head: Head = match serde_json::from_slice(bytes) {
        Ok(head) => head,
        Err(e) if names_the_format(bytes) => return Err(BackupError::Damaged(e.to_string())),
        Err(_) => return Err(BackupError::NotBackup),
    };
    if head.format.as_deref() != Some(BACKUP_FORMAT) {
        return Err(BackupError::NotBackup);
    }
    let version =
        head.version.as_ref().and_then(serde_json::Value::as_u64).filter(|v| *v >= 1).ok_or_else(|| BackupError::Damaged("no format version".into()))?;
    let version = u32::try_from(version).unwrap_or(u32::MAX);
    if version > BACKUP_VERSION {
        return Err(BackupError::TooNew(version));
    }
    let file = migrate(version, bytes)?;
    let counts = BackupCounts {
        history: count(file.history.len()),
        dictionary: count(file.dictionary.len()),
        rules: count(file.rules.len()),
        scenes: count(file.scenes.len()),
        presets: count(file.presets.len()),
    };
    if counts != file.counts {
        return Err(BackupError::Damaged("the counts do not match the content".into()));
    }
    Ok(Backup {
        manifest: BackupManifest {
            format: file.format,
            version: file.version,
            app_version: file.app_version,
            created_at_ms: file.created_at_ms,
            device: file.device,
            platform: file.platform,
            counts,
        },
        history: file.history,
        dictionary: file.dictionary,
        rules: file.rules,
        scenes: file.scenes,
        presets: file.presets,
    })
}

/// A file of format `version` in this build's layout. When a format change raises
/// [`BACKUP_VERSION`], the older version gets a step here that reads its layout as a
/// `serde_json::Value`, rewrites what changed and hands it on to the next version's step; this
/// build's version reads straight into [`BackupFile`]. A version without a step is newer than this
/// build ([`parse`] says so before it gets here).
fn migrate(version: u32, bytes: &[u8]) -> Result<BackupFile, BackupError> {
    match version {
        1 => serde_json::from_slice(bytes).map_err(|e| BackupError::Damaged(e.to_string())),
        newer => Err(BackupError::TooNew(newer)),
    }
}

/// The start of `bytes` names the format: a backup that does not parse is damaged, not foreign.
fn names_the_format(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(HEAD_BYTES)]);
    head.contains("\"format\"") && head.contains(BACKUP_FORMAT)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn version_one() -> Vec<u8> {
        let input = ExportInput {
            app_version: "0.3.0",
            device: "书房",
            platform: Some(Platform::Windows),
            created_at_ms: 1_793_000_000_000,
            dictionary: &[],
            rules: &[],
            scenes: &[],
            presets: &[],
        };
        let mut bytes = Vec::new();
        write_backup(&mut bytes, Path::new("no-history.sqlite3"), &input).unwrap();
        bytes
    }

    /// docs/dictation.md §4.6.1: a file is read through the migration of its format version.
    /// Version 1 is this build's layout and passes through as it is; a version without a step
    /// here is newer than this build, and a file that does not parse in its version's layout is
    /// damaged.
    #[test]
    fn a_file_is_read_through_the_migration_of_its_version() {
        let bytes = version_one();
        let file = migrate(1, &bytes).unwrap();
        assert_eq!((file.format.as_str(), file.version, file.device.as_str(), file.platform), (BACKUP_FORMAT, 1, "书房", Platform::Windows));
        assert_eq!(parse(&bytes).unwrap().manifest.device, "书房", "parse goes the same way");
        assert_eq!(migrate(2, &bytes).err(), Some(BackupError::TooNew(2)));
        assert!(matches!(migrate(1, b"{\"format\":\"voltip-backup\",\"version\":1}"), Err(BackupError::Damaged(_))), "no counts");
    }
}
