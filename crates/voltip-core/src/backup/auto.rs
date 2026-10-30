//! Automatic backups (docs/dictation.md §4.6.1): the same file as a manual backup, written by
//! Voltip itself once a week, before an update installs and before a replacing import, into the
//! automatic backups' folder (`<data dir>/backups`, or the folder `Settings.backup.folder` names).
//!
//! A file is an automatic backup by its name alone: `voltip-auto-<yyyyMMdd-HHmmss>-<kind>-<device>`,
//! in local time, with [`BACKUP_EXTENSION`] (and `-2`, `-3`… before it when the second is taken).
//! `<device>` is the [`device_tag`] of the computer that wrote it (user decision D23, 2026-10-30):
//! several computers may keep their backups in one synced folder, and each manages its own alone.
//! [`prune`] keeps a computer's newest [`AUTO_KEEP`], its three kinds together, ordered by the time
//! in their names, and never touches another computer's or a file of any other name, a backup the
//! user exported into the same folder included. Everything here is a plain function of the folder
//! and the clock it is handed; the runtime decides when (`runtime/auto_backup.rs`).

use std::path::{Path, PathBuf};

use chrono::{NaiveDate, NaiveDateTime, TimeZone as _, Timelike as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use voltip_crypto::PublicKey;

use super::{BACKUP_EXTENSION, BackupError, ExportInput, ExportSummary, export};

/// The folder under the data directory the automatic backups go to by default.
pub const AUTO_BACKUP_DIR: &str = "backups";
/// The automatic backups each computer keeps, its three kinds together (user decision 2026-10-28).
pub const AUTO_KEEP: usize = 5;
/// The hex digits of a [`device_tag`].
pub const DEVICE_TAG_LEN: usize = 8;
/// How old the newest automatic backup may be before the weekly one is due.
pub const WEEK_MS: u64 = 7 * DAY_MS;
/// A day: a failed weekly backup is not tried again sooner, and a newest backup more than this far
/// in the future means the clock went back.
pub const DAY_MS: u64 = 24 * 60 * 60 * 1000;

/// The file in the data directory that carries a failed backup before an update into the next
/// start, which shows it once (the update may end the process right after the backup).
pub const NOTICE_FILE_NAME: &str = "auto-backup-notice.json";

/// What every automatic backup's name starts with.
const PREFIX: &str = "voltip-auto-";
/// The time in a name, local.
const STAMP: &str = "%Y%m%d-%H%M%S";
/// The most `-N` a name tries before the second counts as full.
const MAX_SAME_SECOND: u32 = 99;

/// Why an automatic backup was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoKind {
    /// The weekly one (`Settings.backup.weekly`).
    Weekly,
    /// Before an update installed.
    Update,
    /// Before a replacing import.
    Import,
}

impl AutoKind {
    /// Its word in a file name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Weekly => "weekly",
            Self::Update => "update",
            Self::Import => "import",
        }
    }

    fn parse(word: &str) -> Option<Self> {
        match word {
            "weekly" => Some(Self::Weekly),
            "update" => Some(Self::Update),
            "import" => Some(Self::Import),
            _ => None,
        }
    }
}

/// An automatic backup as `UiState.auto_backup` names it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoBackupFile {
    /// Its file name.
    pub name: String,
    /// The time its name says, Unix milliseconds.
    pub at_ms: u64,
    /// Why it was made.
    pub kind: AutoKind,
}

/// An automatic backup that could not be written.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoBackupFailure {
    /// Which one.
    pub kind: AutoKind,
    /// When, Unix milliseconds.
    pub at_ms: u64,
    /// The system's message.
    pub detail: String,
}

/// `UiState.auto_backup` (docs/dictation.md §4.6.1): where the automatic backups go, the newest
/// one there and the last failure; `available` is false where the app makes none (the phone).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoBackupView {
    /// This app makes automatic backups (the desktop).
    pub available: bool,
    /// This computer's [`device_tag`]: the automatic backups whose names carry it are the ones it
    /// keeps and counts; the folder may hold other computers' too.
    pub device: String,
    /// Where they go: `Settings.backup.folder`, or [`default_folder`].
    pub folder: String,
    /// [`default_folder`].
    pub default_folder: String,
    /// `folder` is one the user chose.
    pub custom: bool,
    /// This computer's newest automatic backup in `folder`; `None` before the folder was looked at,
    /// or when it holds none of this computer's.
    pub last: Option<AutoBackupFile>,
    /// The last automatic backup failed (the weekly one or the one before an update); gone once
    /// one succeeds or the folder changes.
    pub failure: Option<AutoBackupFailure>,
    /// The backup before the update this run started on failed: home says so until it is closed.
    pub notice: Option<AutoBackupFailure>,
}

/// Where the automatic backups go by default: `<data dir>/backups`.
pub fn default_folder(data_dir: &Path) -> PathBuf {
    data_dir.join(AUTO_BACKUP_DIR)
}

/// What a [`crate::CoreCommand::BackupAuto`] answers: the backup written, or the system's message.
pub type AutoOutcome = Result<AutoBackupFile, String>;

/// Where the core answers a [`crate::CoreCommand::BackupAuto`]: once, when the file is written or
/// failed.
#[derive(Clone)]
pub struct AutoReply(std::sync::Arc<parking_lot::Mutex<Option<tokio::sync::oneshot::Sender<AutoOutcome>>>>);

impl AutoReply {
    /// The reply to hand the command, and the receiver the caller waits on.
    pub fn channel() -> (Self, tokio::sync::oneshot::Receiver<AutoOutcome>) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        (Self(std::sync::Arc::new(parking_lot::Mutex::new(Some(tx)))), rx)
    }

    /// Answer; a second answer, or one nobody waits for any more, goes nowhere.
    pub fn send(&self, outcome: AutoOutcome) {
        if let Some(tx) = self.0.lock().take() {
            let _ = tx.send(outcome);
        }
    }
}

impl std::fmt::Debug for AutoReply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AutoReply")
    }
}

/// This computer's mark in its automatic backups' names (user decision D23, 2026-10-30): the first
/// [`DEVICE_TAG_LEN`] hex digits of its device identity's fingerprint (SHA-256 of the public key,
/// the device list's `A7:C4:19:8E · …`), in lower case.
pub fn device_tag(public_key: &PublicKey) -> String {
    let digest = Sha256::digest(public_key.as_bytes());
    digest[..DEVICE_TAG_LEN / 2].iter().map(|b| format!("{b:02x}")).collect()
}

/// `text` is a [`device_tag`]: [`DEVICE_TAG_LEN`] lower-case hex digits.
fn is_device_tag(text: &str) -> bool {
    text.len() == DEVICE_TAG_LEN && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// What an automatic backup's name says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutoName {
    /// Its local time, to the second.
    pub stamp: NaiveDateTime,
    /// Why it was made.
    pub kind: AutoKind,
    /// The [`device_tag`] of the computer that wrote it.
    pub device: String,
    /// Its `-N`; 1 without one.
    pub same: u32,
}

/// An automatic backup's name: `stamp` (local time), `kind` and `device`, and `-same` when
/// `same` > 1.
pub fn auto_name(kind: AutoKind, stamp: NaiveDateTime, device: &str, same: u32) -> String {
    let same = if same > 1 { format!("-{same}") } else { String::new() };
    format!("{PREFIX}{}-{}-{device}{same}.{BACKUP_EXTENSION}", stamp.format(STAMP), kind.as_str())
}

/// What an automatic backup's name says; `None` for any other name.
pub fn parse_auto_name(name: &str) -> Option<AutoName> {
    let rest = name.strip_prefix(PREFIX)?.strip_suffix(BACKUP_EXTENSION)?.strip_suffix('.')?;
    let (date, rest) = rest.split_at_checked(8)?;
    let (time, rest) = rest.strip_prefix('-')?.split_at_checked(6)?;
    let rest = rest.strip_prefix('-')?;
    let all_digits = |text: &str| text.bytes().all(|b| b.is_ascii_digit());
    // Checked before the slices below: a name of the user's may hold any character there.
    if !all_digits(date) || !all_digits(time) {
        return None;
    }
    let digits = |text: &str| all_digits(text).then(|| text.parse::<u32>().ok()).flatten();
    let (year, month, day) = (digits(&date[..4])?, digits(&date[4..6])?, digits(&date[6..])?);
    let (hour, minute, second) = (digits(&time[..2])?, digits(&time[2..4])?, digits(&time[4..])?);
    let stamp = NaiveDate::from_ymd_opt(i32::try_from(year).ok()?, month, day)?.and_hms_opt(hour, minute, second)?;
    let (word, rest) = rest.split_once('-')?;
    let (device, same) = match rest.split_once('-') {
        // Written by `auto_name` alone: 2 or more, without a leading zero.
        Some((device, n)) if !n.starts_with('0') => (device, digits(n).filter(|n| *n >= 2)?),
        Some(_) => return None,
        None => (rest, 1),
    };
    if !is_device_tag(device) {
        return None;
    }
    Some(AutoName { stamp, kind: AutoKind::parse(word)?, device: device.to_owned(), same })
}

/// An automatic backup in a folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutoFile {
    /// Where.
    pub path: PathBuf,
    /// Its file name.
    pub name: String,
    /// The local time its name says.
    pub stamp: NaiveDateTime,
    /// That time in Unix milliseconds, read in this computer's time zone.
    pub at_ms: u64,
    /// Why it was made.
    pub kind: AutoKind,
    /// The [`device_tag`] of the computer that wrote it.
    pub device: String,
    /// The `-N` of its name; 1 without one.
    pub same: u32,
}

impl AutoFile {
    /// As `UiState.auto_backup` names it.
    pub fn view(&self) -> AutoBackupFile {
        AutoBackupFile { name: self.name.clone(), at_ms: self.at_ms, kind: self.kind }
    }
}

/// `stamp`, a local time, in Unix milliseconds: the earlier of two readings when the clock went
/// back over it, and the time read as UTC where the time zone skips it.
pub fn local_ms(stamp: NaiveDateTime) -> u64 {
    let ms = chrono::Local.from_local_datetime(&stamp).earliest().map_or_else(|| stamp.and_utc().timestamp_millis(), |t| t.timestamp_millis());
    u64::try_from(ms).unwrap_or(0)
}

/// Now, as an automatic backup's name and its manifest put it.
pub fn local_now() -> NaiveDateTime {
    chrono::Local::now().naive_local()
}

/// The automatic backups in `folder`, newest first (by the time in their names, then the `-N`):
/// the ones `device` wrote, or every computer's for `None`. A folder that is not there holds none.
pub fn list(folder: &Path, device: Option<&str>) -> std::io::Result<Vec<AutoFile>> {
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry?;
        // A directory or a link of that name is not a backup Voltip wrote.
        if !entry.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else { continue };
        let Some(AutoName { stamp, kind, device: by, same }) = parse_auto_name(&name) else { continue };
        if device.is_some_and(|device| device != by) {
            continue;
        }
        files.push(AutoFile { path: entry.path(), name, stamp, at_ms: local_ms(stamp), kind, device: by, same });
    }
    files.sort_by(|a, b| (b.stamp, b.same, &b.name).cmp(&(a.stamp, a.same, &a.name)));
    Ok(files)
}

/// Delete all but the newest `keep` automatic backups `device` wrote in `folder`; `written`, the
/// one just made, is never deleted, whatever its name says (a clock that went back). Another
/// computer's backups and every other file stay. Returns what was deleted; a file that cannot be
/// deleted is logged and left.
pub fn prune(folder: &Path, device: &str, keep: usize, written: Option<&Path>) -> Vec<PathBuf> {
    let files = match list(folder, Some(device)) {
        Ok(files) => files,
        Err(e) => {
            tracing::warn!(error = %e, "the automatic backups could not be listed; none deleted");
            return Vec::new();
        }
    };
    let protected = written.filter(|w| files.iter().any(|f| f.path == *w));
    let others = keep.saturating_sub(usize::from(protected.is_some()));
    let mut excess: Vec<&AutoFile> = files.iter().filter(|f| Some(f.path.as_path()) != protected).skip(others).collect();
    excess.reverse();
    let mut removed = Vec::new();
    for file in excess {
        match std::fs::remove_file(&file.path) {
            Ok(()) => removed.push(file.path.clone()),
            Err(e) => tracing::warn!(error = %e, file = %file.name, "an old automatic backup could not be deleted"),
        }
    }
    removed
}

/// The weekly backup is due at `now_ms`, the newest automatic backup being from `newest_ms`: when
/// there is none, when it is a week old, or when it is more than a day in the future (the clock
/// went back, so its age says nothing).
pub fn weekly_due(newest_ms: Option<u64>, now_ms: u64) -> bool {
    match newest_ms {
        None => true,
        Some(at) if at > now_ms.saturating_add(DAY_MS) => true,
        Some(at) => now_ms.saturating_sub(at) >= WEEK_MS,
    }
}

/// Write an automatic backup of `kind` of `input` and the history in `history_db` into `folder`
/// (created when missing) under the name `stamp` and `device` give it, through [`export`]'s
/// `.part`; then [`prune`] `device`'s backups there to [`AUTO_KEEP`]. An existing file is never
/// replaced: the next free `-N` is taken.
pub fn write_auto(
    kind: AutoKind,
    folder: &Path,
    device: &str,
    history_db: &Path,
    input: &ExportInput<'_>,
    stamp: NaiveDateTime,
) -> Result<(AutoFile, ExportSummary), BackupError> {
    // The time a file is known by is the one its name says, to the second.
    let stamp = stamp.with_nanosecond(0).unwrap_or(stamp);
    std::fs::create_dir_all(folder).map_err(|e| BackupError::Write(e.to_string()))?;
    let (path, name, same) = free_name(kind, folder, device, stamp)?;
    let summary = export(&path, history_db, input)?;
    prune(folder, device, AUTO_KEEP, Some(&path));
    Ok((AutoFile { path, name, stamp, at_ms: local_ms(stamp), kind, device: device.to_owned(), same }, summary))
}

/// The first name of `kind` at `stamp` by `device` nothing in `folder` has.
fn free_name(kind: AutoKind, folder: &Path, device: &str, stamp: NaiveDateTime) -> Result<(PathBuf, String, u32), BackupError> {
    for same in 1..=MAX_SAME_SECOND {
        let name = auto_name(kind, stamp, device, same);
        let path = folder.join(&name);
        if std::fs::symlink_metadata(&path).is_err() {
            return Ok((path, name, same));
        }
    }
    Err(BackupError::Write(format!("{MAX_SAME_SECOND} automatic backups have the same time")))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::dictation::fakes::history_entry;
    use crate::history::{HISTORY_DB_FILE_NAME, HistoryStore};

    /// This computer's tag, and another computer's sharing the folder.
    const HERE: &str = "a7c4198e";
    const THERE: &str = "3d9f2b10";

    fn at(text: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    fn input<'a>(device: &'a str) -> ExportInput<'a> {
        ExportInput { app_version: "0.3.0", device, platform: None, created_at_ms: 1_793_000_000_000, dictionary: &[], rules: &[], scenes: &[], presets: &[] }
    }

    fn parsed(stamp: NaiveDateTime, kind: AutoKind, device: &str, same: u32) -> Option<AutoName> {
        Some(AutoName { stamp, kind, device: device.into(), same })
    }

    /// The name says the local time, to the second, the kind and the computer (user decision D23,
    /// 2026-10-30); ASCII, so a folder sorted by name in any file manager or sync service lists the
    /// automatic backups in time order.
    #[test]
    fn a_name_says_its_time_kind_and_computer_and_reads_back() {
        let stamp = at("2026-10-30 07:05:09");
        assert_eq!(auto_name(AutoKind::Weekly, stamp, HERE, 1), "voltip-auto-20261030-070509-weekly-a7c4198e.voltip-backup");
        assert_eq!(auto_name(AutoKind::Update, stamp, HERE, 1), "voltip-auto-20261030-070509-update-a7c4198e.voltip-backup");
        assert_eq!(auto_name(AutoKind::Import, stamp, HERE, 2), "voltip-auto-20261030-070509-import-a7c4198e-2.voltip-backup");
        // A tag of digits alone is still the tag, and a `-N` after it still the `-N`.
        for kind in [AutoKind::Weekly, AutoKind::Update, AutoKind::Import] {
            for device in [HERE, THERE, "12345678"] {
                for same in [1, 2, 37] {
                    let name = auto_name(kind, stamp, device, same);
                    assert!(name.is_ascii(), "{name}");
                    assert_eq!(parse_auto_name(&name), parsed(stamp, kind, device, same), "{name}");
                }
            }
        }
        // Earlier sorts first by name too, whichever computer wrote it.
        assert!(auto_name(AutoKind::Weekly, at("2026-10-29 23:59:59"), THERE, 1) < auto_name(AutoKind::Import, at("2026-10-30 00:00:00"), HERE, 1));
    }

    /// Only the pattern is an automatic backup: a manual export, a half-written file, another
    /// kind, a date that does not exist, a changed case, or a name without its computer's tag
    /// (the form before user decision D23, which no release wrote) are not.
    #[test]
    fn any_other_name_is_not_an_automatic_backup() {
        for name in [
            "Voltip 备份 2026-10-30.voltip-backup",
            "voltip-auto-20261030-070509-weekly-a7c4198e.voltip-backup.part",
            "voltip-auto-20261030-070509-weekly-a7c4198e.json",
            "voltip-auto-20261030-070509-monthly-a7c4198e.voltip-backup",
            "voltip-auto-20261030-070509-a7c4198e.voltip-backup",
            "voltip-auto-20261030-070509.voltip-backup",
            "voltip-auto-20261332-070509-weekly-a7c4198e.voltip-backup",
            "voltip-auto-20261030-250000-weekly-a7c4198e.voltip-backup",
            "voltip-auto-2026103-070509-weekly-a7c4198e.voltip-backup",
            "voltip-auto-20261030-070509-weekly-a7c4198e-0.voltip-backup",
            "voltip-auto-20261030-070509-weekly-a7c4198e-1.voltip-backup",
            "voltip-auto-20261030-070509-weekly-a7c4198e-02.voltip-backup",
            "voltip-auto-20261030-070509-weekly-a7c4198e-x.voltip-backup",
            "voltip-auto-20261030-070509-weekly-a7c4198e-.voltip-backup",
            "voltip-auto-20261030-070509-weekly-a7c4198e-2-3.voltip-backup",
            "VOLTIP-AUTO-20261030-070509-WEEKLY-A7C4198E.voltip-backup",
            "my-voltip-auto-20261030-070509-weekly-a7c4198e.voltip-backup",
            // The computer's tag: eight lower-case hex digits, nothing else.
            "voltip-auto-20261030-070509-weekly.voltip-backup",
            "voltip-auto-20261030-070509-weekly-2.voltip-backup",
            "voltip-auto-20261030-070509-weekly-A7C4198E.voltip-backup",
            "voltip-auto-20261030-070509-weekly-a7c4198.voltip-backup",
            "voltip-auto-20261030-070509-weekly-a7c4198e0.voltip-backup",
            "voltip-auto-20261030-070509-weekly-a7c4198g.voltip-backup",
            "voltip-auto-20261030-070509-weekly--a7c4198e.voltip-backup",
            "voltip-auto-20261030-070509-weekly-a7c4198é.voltip-backup",
            "voltip-auto-20261030-070509-weekly-书房电脑.voltip-backup",
            // Eight bytes that are not eight digits: read without slicing into a character.
            "voltip-auto-2026年1-070509-weekly-a7c4198e.voltip-backup",
            "voltip-auto-20261030-0é509-weekly-a7c4198e.voltip-backup",
            "voltip-auto-20261030-070509-weekly-a7c4198e-二.voltip-backup",
            "",
        ] {
            assert_eq!(parse_auto_name(name), None, "{name}");
        }
    }

    /// The time a name says is read in this computer's time zone: back to the instant it was
    /// written in.
    #[test]
    fn a_names_time_is_the_instant_it_was_written() {
        let now_ms = 1_793_000_000_123;
        let local = chrono::Local.timestamp_millis_opt(i64::try_from(now_ms).unwrap()).unwrap().naive_local();
        let name = auto_name(AutoKind::Weekly, local, HERE, 1);
        let stamp = parse_auto_name(&name).unwrap().stamp;
        assert_eq!(local_ms(stamp), now_ms - 123, "to the second");
    }

    /// User decision D23 (2026-10-30): a computer's tag is the start of the fingerprint the device
    /// list shows for it, lower case: the first four bytes of SHA-256 of its public key.
    #[test]
    fn a_computers_tag_is_the_start_of_its_fingerprint() {
        // SHA-256 of 32 zero bytes is 66687aad f862bd77 …
        assert_eq!(device_tag(&PublicKey([0; 32])), "66687aad");
        for key in [PublicKey([0; 32]), PublicKey([7; 32]), voltip_crypto::StaticKeypair::generate().unwrap().public] {
            let tag = device_tag(&key);
            assert_eq!(tag.len(), DEVICE_TAG_LEN);
            assert!(tag.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)), "{tag}");
            let fingerprint: String = key.fingerprint().chars().filter(char::is_ascii_hexdigit).take(DEVICE_TAG_LEN).collect();
            assert_eq!(tag, fingerprint.to_ascii_lowercase());
            // What it names reads back as that computer's.
            let name = auto_name(AutoKind::Weekly, at("2026-10-30 07:05:09"), &tag, 1);
            assert_eq!(parse_auto_name(&name).map(|n| n.device), Some(tag));
        }
        assert_ne!(device_tag(&PublicKey([0; 32])), device_tag(&PublicKey([7; 32])));
    }

    /// docs/dictation.md §4.6.1: due when there is no automatic backup, exactly a week after the
    /// newest, and when the newest is more than a day in the future (a clock that went back);
    /// not a millisecond before the week is up, nor for a newest a little ahead of the clock.
    #[test]
    fn the_weekly_backup_is_due_a_week_after_the_newest_one() {
        let now = 1_793_000_000_000;
        assert!(weekly_due(None, now), "no automatic backup yet");
        assert!(weekly_due(Some(now - WEEK_MS), now), "exactly seven days");
        assert!(!weekly_due(Some(now - WEEK_MS + 1), now), "a millisecond short of seven days");
        assert!(!weekly_due(Some(now), now));
        assert!(!weekly_due(Some(now - 3 * DAY_MS), now));
        assert!(weekly_due(Some(now - 30 * DAY_MS), now));
        // The clock went back: a newest backup two days ahead of it says nothing about its age.
        assert!(weekly_due(Some(now + 2 * DAY_MS), now), "the clock was set back");
        assert!(!weekly_due(Some(now + DAY_MS), now), "a day ahead is a time zone or a small correction");
        assert!(!weekly_due(Some(now + 3_600_000), now));
        assert!(weekly_due(Some(u64::MAX), now));
        assert!(!weekly_due(Some(0), 0));
    }

    fn touch(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, b"{}").unwrap();
        path
    }

    fn names(files: &[AutoFile]) -> Vec<&str> {
        files.iter().map(|f| f.name.as_str()).collect()
    }

    /// The listing holds the automatic backups alone, newest first: one computer's, or every
    /// computer's; a folder that is not there holds none.
    #[test]
    fn a_folder_lists_one_computers_automatic_backups_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(list(&dir.path().join("not-there"), Some(HERE)).unwrap(), Vec::new());
        touch(dir.path(), "voltip-auto-20261001-090000-weekly-a7c4198e.voltip-backup");
        touch(dir.path(), "voltip-auto-20261030-070509-import-a7c4198e.voltip-backup");
        touch(dir.path(), "voltip-auto-20261030-070509-import-a7c4198e-2.voltip-backup");
        touch(dir.path(), "voltip-auto-20261015-120000-update-a7c4198e.voltip-backup");
        touch(dir.path(), "voltip-auto-20261031-080000-weekly-3d9f2b10.voltip-backup");
        touch(dir.path(), "voltip-auto-20261002-080000-update-3d9f2b10.voltip-backup");
        touch(dir.path(), "voltip-auto-20261101-080000-weekly.voltip-backup");
        touch(dir.path(), "Voltip 备份 2026-10-30.voltip-backup");
        touch(dir.path(), "voltip-auto-20261031-000000-weekly-a7c4198e.voltip-backup.part");
        std::fs::create_dir(dir.path().join("voltip-auto-20261101-000000-weekly-a7c4198e.voltip-backup")).unwrap();
        let found = list(dir.path(), Some(HERE)).unwrap();
        assert_eq!(
            names(&found),
            [
                "voltip-auto-20261030-070509-import-a7c4198e-2.voltip-backup",
                "voltip-auto-20261030-070509-import-a7c4198e.voltip-backup",
                "voltip-auto-20261015-120000-update-a7c4198e.voltip-backup",
                "voltip-auto-20261001-090000-weekly-a7c4198e.voltip-backup",
            ]
        );
        let newest = &found[0];
        assert_eq!((newest.kind, newest.same, newest.stamp, newest.device.as_str()), (AutoKind::Import, 2, at("2026-10-30 07:05:09"), HERE));
        assert_eq!(newest.path, dir.path().join(&newest.name));
        assert_eq!(newest.at_ms, local_ms(newest.stamp));
        assert_eq!(
            names(&list(dir.path(), Some(THERE)).unwrap()),
            ["voltip-auto-20261031-080000-weekly-3d9f2b10.voltip-backup", "voltip-auto-20261002-080000-update-3d9f2b10.voltip-backup"]
        );
        let every = list(dir.path(), None).unwrap();
        assert_eq!(every.len(), 6, "both computers' together");
        assert_eq!(every[0].name, "voltip-auto-20261031-080000-weekly-3d9f2b10.voltip-backup");
    }

    /// docs/dictation.md §4.6.1: the three kinds together keep this computer's newest five; a
    /// manual backup, a half-written file and any other file in the folder stay.
    #[test]
    fn prune_keeps_this_computers_newest_five_and_touches_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let old: Vec<PathBuf> = (1..=7).map(|day| touch(dir.path(), &format!("voltip-auto-202610{day:02}-090000-weekly-{HERE}.voltip-backup"))).collect();
        let update = touch(dir.path(), &format!("voltip-auto-20261008-100000-update-{HERE}.voltip-backup"));
        let manual = touch(dir.path(), "Voltip 备份 2026-10-01.voltip-backup");
        let part = touch(dir.path(), &format!("voltip-auto-20261001-000000-weekly-{HERE}.voltip-backup.part"));
        let notes = touch(dir.path(), "notes.txt");
        let untagged = touch(dir.path(), "voltip-auto-20260901-090000-weekly.voltip-backup");
        let removed = prune(dir.path(), HERE, AUTO_KEEP, None);
        assert_eq!(removed, old[..3].to_vec(), "the three oldest go");
        let kept = list(dir.path(), Some(HERE)).unwrap();
        assert_eq!(kept.len(), AUTO_KEEP);
        assert_eq!(kept[0].path, update, "the kinds count together");
        for other in [&manual, &part, &notes, &untagged] {
            assert!(other.exists(), "{}", other.display());
        }
        assert_eq!(prune(dir.path(), HERE, AUTO_KEEP, None), Vec::<PathBuf>::new(), "five is five");
        assert_eq!(prune(&dir.path().join("not-there"), HERE, AUTO_KEEP, None), Vec::<PathBuf>::new());
    }

    /// Regression (user decision D23, 2026-10-30): two computers sharing one synced folder kept the
    /// newest five of both together, so the one that backed up more deleted the other's. Each
    /// computer now deletes its own alone, however many the other has, older or newer.
    #[test]
    fn regression_another_computers_backups_are_neither_counted_nor_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let theirs: Vec<PathBuf> = (1..=9).map(|day| touch(dir.path(), &format!("voltip-auto-202610{day:02}-120000-weekly-{THERE}.voltip-backup"))).collect();
        let newer = touch(dir.path(), &format!("voltip-auto-20261031-120000-update-{THERE}.voltip-backup"));
        let mine: Vec<PathBuf> = (20..=25).map(|day| touch(dir.path(), &format!("voltip-auto-202610{day}-090000-weekly-{HERE}.voltip-backup"))).collect();
        let removed = prune(dir.path(), HERE, AUTO_KEEP, None);
        assert_eq!(removed, [mine[0].clone()], "six of this computer's: its oldest goes, and nothing else");
        for path in theirs.iter().chain([&newer]) {
            assert!(path.exists(), "another computer's backup stays: {}", path.display());
        }
        assert_eq!(list(dir.path(), Some(HERE)).unwrap().len(), AUTO_KEEP);
        assert_eq!(list(dir.path(), Some(THERE)).unwrap().len(), 10, "the other computer prunes its own when it writes");
        // The other computer's turn: its own go, this one's stay.
        let removed = prune(dir.path(), THERE, AUTO_KEEP, None);
        assert_eq!(removed, theirs[..5].to_vec());
        assert_eq!(list(dir.path(), Some(HERE)).unwrap().len(), AUTO_KEEP);
    }

    /// A clock that went back names the backup just written older than the five there: it stays
    /// all the same, and the oldest of the others goes in its place.
    #[test]
    fn prune_never_deletes_the_backup_just_written() {
        let dir = tempfile::tempdir().unwrap();
        let ahead: Vec<PathBuf> = (1..=5).map(|day| touch(dir.path(), &format!("voltip-auto-202711{day:02}-090000-weekly-{HERE}.voltip-backup"))).collect();
        let written = touch(dir.path(), &format!("voltip-auto-20261030-090000-weekly-{HERE}.voltip-backup"));
        let removed = prune(dir.path(), HERE, AUTO_KEEP, Some(&written));
        assert_eq!(removed, [ahead[0].clone()]);
        assert!(written.exists());
        assert_eq!(list(dir.path(), Some(HERE)).unwrap().len(), AUTO_KEEP);
    }

    /// docs/dictation.md §4.6.1: an automatic backup is the manual backup's file under the
    /// automatic name with this computer's tag, in a folder made when missing; the folder then
    /// keeps five of this computer's, and a name taken already gets the next `-N` instead of
    /// replacing the file.
    #[test]
    fn an_automatic_backup_is_a_whole_backup_and_never_replaces_one() {
        let data = tempfile::tempdir().unwrap();
        let mut history = HistoryStore::open(data.path());
        history.push(history_entry("第一句"), 20_000).unwrap();
        history.push(history_entry("第二句"), 20_000).unwrap();
        drop(history);
        let db = data.path().join(HISTORY_DB_FILE_NAME);
        let folder = data.path().join("同步盘").join(AUTO_BACKUP_DIR);
        let stamp = at("2026-10-30 07:05:09");

        let (file, summary) = write_auto(AutoKind::Weekly, &folder, HERE, &db, &input("书房"), stamp).unwrap();
        assert_eq!(file.name, "voltip-auto-20261030-070509-weekly-a7c4198e.voltip-backup");
        assert_eq!((file.kind, file.stamp, file.same, file.path.clone()), (AutoKind::Weekly, stamp, 1, folder.join(&file.name)));
        assert_eq!(file.device, HERE);
        assert_eq!(summary.counts.history, 2);
        let backup = crate::backup::read(&file.path).unwrap();
        assert_eq!((backup.manifest.device.as_str(), backup.history.len()), ("书房", 2));
        assert!(!folder.join(format!("{}.part", file.name)).exists());

        let first = std::fs::read(&file.path).unwrap();
        let (again, _) = write_auto(AutoKind::Weekly, &folder, HERE, &db, &input("客厅"), stamp).unwrap();
        assert_eq!(again.name, "voltip-auto-20261030-070509-weekly-a7c4198e-2.voltip-backup");
        assert_eq!(std::fs::read(&file.path).unwrap(), first, "the first one is as it was");
        // Another computer writing in the same second takes its own name.
        let (theirs, _) = write_auto(AutoKind::Weekly, &folder, THERE, &db, &input("客厅"), stamp).unwrap();
        assert_eq!(theirs.name, "voltip-auto-20261030-070509-weekly-3d9f2b10.voltip-backup");

        for day in 1..=6 {
            touch(&folder, &format!("voltip-auto-202609{day:02}-090000-weekly-{HERE}.voltip-backup"));
        }
        let (latest, _) = write_auto(AutoKind::Import, &folder, HERE, &db, &input("书房"), at("2026-10-31 08:00:00")).unwrap();
        let kept = list(&folder, Some(HERE)).unwrap();
        assert_eq!(kept.len(), AUTO_KEEP);
        assert_eq!(kept[0], latest);
        assert!(kept.iter().any(|k| k.path == file.path) && kept.iter().any(|k| k.path == again.path));
        assert!(theirs.path.exists(), "not one of this computer's five");
    }

    /// A folder that cannot be made, or a file in its place, fails with the system's reason and
    /// leaves nothing behind.
    #[test]
    fn an_automatic_backup_that_cannot_be_written_says_why() {
        let data = tempfile::tempdir().unwrap();
        let blocker = touch(data.path(), "a file");
        let err = write_auto(
            AutoKind::Weekly,
            &blocker.join(AUTO_BACKUP_DIR),
            HERE,
            &data.path().join(HISTORY_DB_FILE_NAME),
            &input("书房"),
            at("2026-10-30 07:05:09"),
        )
        .unwrap_err();
        assert!(matches!(&err, BackupError::Write(why) if !why.is_empty()), "{err:?}");
        assert_eq!(std::fs::read(&blocker).unwrap(), b"{}");
    }
}
