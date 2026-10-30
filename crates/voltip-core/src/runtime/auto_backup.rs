//! Automatic backups in the core task (docs/dictation.md §4.6.1): the weekly one, the one the
//! desktop shell asks for before an update installs, and what `UiState.auto_backup` shows. The one
//! before a replacing import is written in that import's own step (`runtime/backup.rs`).
//!
//! Every automatic backup is made of the runtime's own lists and identity, never of the bridge's
//! cache, which follows the core's events and may be a step behind: the lists are copied on the
//! core task, after every command queued before the backup. The history comes from its database on
//! a read-only connection, one snapshot. The weekly and the update backups are written on a
//! blocking thread, one at a time; the others wait their turn. The weekly one needs
//! `CoreConfig.auto_backup` (the desktop): the folder is looked at [`AUTO_SCAN_DELAY`] after start
//! and again when the setting names another folder, so no tick reads a directory.

use std::collections::VecDeque;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use voltip_protocol::Platform;

use super::{CoreEvent, Runtime};
use crate::backup::auto::{
    self, AutoBackupFailure, AutoBackupView, AutoFile, AutoKind, AutoReply, DAY_MS, NOTICE_FILE_NAME, default_folder, local_now, weekly_due,
};
use crate::backup::{BackupError, ExportInput, ExportSummary};
use crate::dictation::DictationPhase;
use crate::presets::CustomPreset;
use crate::scenes::Scene;
use crate::vocabulary::{DictionaryEntry, ReplacementRule};
use crate::{CoreError, history::HISTORY_DB_FILE_NAME};

/// How long after start the automatic backups' folder is first looked at (the window and the
/// core settle first; a weekly backup that is due follows at once).
pub const AUTO_SCAN_DELAY: Duration = Duration::from_secs(60);

/// What reaches the core about automatic backups.
pub(super) enum AutoInbound {
    /// The folder was looked at.
    Scanned {
        /// Which one.
        folder: PathBuf,
        /// Its automatic backups, newest first.
        result: std::io::Result<Vec<AutoFile>>,
    },
    /// A backup was written, or failed.
    Written {
        /// Why it was made.
        kind: AutoKind,
        /// Where.
        folder: PathBuf,
        /// The file, or why there is none.
        result: Result<(AutoFile, ExportSummary), BackupError>,
        /// How long it took.
        took: Duration,
    },
}

/// The automatic backups' part of the runtime.
pub(super) struct AutoBackups {
    /// When the core started, Unix milliseconds.
    started_ms: u64,
    /// The folder the last scan answered for.
    scanned: Option<PathBuf>,
    /// A scan is out.
    scanning: bool,
    /// The newest automatic backup in the folder, as scanned or written.
    newest: Option<AutoFile>,
    /// A backup is being written.
    writing: bool,
    /// The backups waiting for it, with whom to answer.
    waiting: VecDeque<(AutoKind, Option<AutoReply>)>,
    /// The last weekly or update backup failed.
    failure: Option<AutoBackupFailure>,
    /// After a failed weekly backup, the next is not tried before this.
    retry_at_ms: Option<u64>,
    /// The backup before the update this run started on failed.
    notice: Option<AutoBackupFailure>,
    tx: mpsc::Sender<AutoInbound>,
}

impl AutoBackups {
    /// The part of a core started at `now_ms` on `data_dir`, with a failure the last run left for
    /// this one to show; the answers come back on the receiver.
    pub(super) fn new(data_dir: &Path, now_ms: u64) -> (Self, mpsc::Receiver<AutoInbound>) {
        let (tx, rx) = mpsc::channel(4);
        let notice = take_notice(data_dir);
        let auto = Self {
            started_ms: now_ms,
            scanned: None,
            scanning: false,
            newest: None,
            writing: false,
            waiting: VecDeque::new(),
            failure: notice.clone(),
            retry_at_ms: None,
            notice,
            tx,
        };
        (auto, rx)
    }
}

/// The failure the last run left in [`NOTICE_FILE_NAME`]: read once, the file goes.
fn take_notice(data_dir: &Path) -> Option<AutoBackupFailure> {
    let path = data_dir.join(NOTICE_FILE_NAME);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            tracing::warn!(error = %e, "the notice of a failed backup before the update could not be read");
            return None;
        }
    };
    if let Err(e) = std::fs::remove_file(&path) {
        tracing::warn!(error = %e, "the notice of a failed backup before the update could not be removed");
    }
    serde_json::from_slice(&bytes).map_err(|e| tracing::warn!(error = %e, "the notice of a failed backup before the update is damaged")).ok()
}

/// Leave `failure` for the next start: on disk before the shell hears of it, since the update
/// that follows may end this process at once (the Windows installer does).
fn leave_notice(data_dir: &Path, failure: &AutoBackupFailure) {
    let write = || -> std::io::Result<()> {
        let mut file = std::fs::File::create(data_dir.join(NOTICE_FILE_NAME))?;
        file.write_all(&serde_json::to_vec(failure)?)?;
        file.sync_all()
    };
    if let Err(e) = write() {
        tracing::warn!(error = %e, "the notice of a failed backup before the update could not be written");
    }
}

/// What an automatic backup is made of besides the history, copied out of the runtime.
struct Lists {
    app_version: String,
    device: String,
    platform: Platform,
    created_at_ms: u64,
    dictionary: Vec<DictionaryEntry>,
    rules: Vec<ReplacementRule>,
    scenes: Vec<Scene>,
    presets: Vec<CustomPreset>,
}

impl Lists {
    fn input(&self) -> ExportInput<'_> {
        ExportInput {
            app_version: &self.app_version,
            device: &self.device,
            platform: Some(self.platform),
            created_at_ms: self.created_at_ms,
            dictionary: &self.dictionary,
            rules: &self.rules,
            scenes: &self.scenes,
            presets: &self.presets,
        }
    }
}

/// An error as `AutoBackupFailure.detail` shows it: the system's message.
pub(super) fn failure_detail(error: &BackupError) -> String {
    let detail = error.detail();
    if detail.is_empty() { error.to_string() } else { detail }
}

fn ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

impl Runtime {
    /// Where the automatic backups go.
    pub(super) fn auto_folder(&self) -> PathBuf {
        self.settings.backup.folder.as_deref().map_or_else(|| default_folder(&self.config.data_dir), PathBuf::from)
    }

    /// `UiState.auto_backup`.
    pub(super) fn auto_view(&self) -> AutoBackupView {
        AutoBackupView {
            available: self.config.auto_backup,
            folder: self.auto_folder().display().to_string(),
            default_folder: default_folder(&self.config.data_dir).display().to_string(),
            custom: self.settings.backup.folder.is_some(),
            last: self.auto.newest.as_ref().map(AutoFile::view),
            failure: self.auto.failure.clone(),
            notice: self.auto.notice.clone(),
        }
    }

    /// The lists and the identity as the runtime holds them now: after every command before.
    fn auto_lists(&self) -> Lists {
        let identity = self.identity.public();
        Lists {
            app_version: self.config.app_version.clone(),
            device: identity.name,
            platform: identity.platform,
            created_at_ms: super::now_ms(),
            dictionary: self.dictionary.entries().to_vec(),
            rules: self.rules.rules().to_vec(),
            scenes: self.scenes.scenes().to_vec(),
            presets: self.presets.presets().to_vec(),
        }
    }

    /// A take runs: its entry would be recorded into a history being replaced.
    fn take_running(&self) -> bool {
        matches!(self.dictation.status().phase, DictationPhase::Listening { .. } | DictationPhase::Processing { .. })
    }

    pub(super) fn emit_auto_backup(&self) {
        self.emit(CoreEvent::AutoBackup(self.auto_view()));
    }

    /// Once per tick, at `now_ms`: look at the folder once it is due, then start the weekly backup
    /// when it is.
    pub(super) fn backup_tick_at(&mut self, now_ms: u64) {
        if !self.config.auto_backup {
            return;
        }
        let folder = self.auto_folder();
        if self.auto.scanned.as_ref() != Some(&folder) {
            if !self.auto.scanning && now_ms >= self.auto.started_ms.saturating_add(ms(AUTO_SCAN_DELAY)) {
                self.start_scan(folder);
            }
            return;
        }
        let auto = &self.auto;
        let due = self.settings.backup.weekly
            && !auto.writing
            && auto.waiting.is_empty()
            && auto.retry_at_ms.is_none_or(|at| now_ms >= at)
            && weekly_due(auto.newest.as_ref().map(|f| f.at_ms), now_ms);
        if due {
            self.request_auto_backup(AutoKind::Weekly, None);
        }
    }

    /// Look at `folder` on a blocking thread.
    fn start_scan(&mut self, folder: PathBuf) {
        self.auto.scanning = true;
        let tx = self.auto.tx.clone();
        tokio::task::spawn_blocking(move || {
            let result = auto::list(&folder);
            let _ = tx.blocking_send(AutoInbound::Scanned { folder, result });
        });
    }

    /// `BackupAuto` (and the weekly one): write a backup of `kind` once the one being written is.
    pub(super) fn request_auto_backup(&mut self, kind: AutoKind, reply: Option<AutoReply>) {
        self.auto.waiting.push_back((kind, reply));
        self.next_auto_backup();
    }

    /// Start the next backup waiting, unless one is being written: its lists copied here, the file
    /// written on a blocking thread, which answers the caller itself (the core may be busy, and an
    /// update waits on the answer).
    fn next_auto_backup(&mut self) {
        if self.auto.writing {
            return;
        }
        let Some((kind, reply)) = self.auto.waiting.pop_front() else { return };
        self.auto.writing = true;
        let folder = self.auto_folder();
        let lists = self.auto_lists();
        let history_db = self.config.data_dir.join(HISTORY_DB_FILE_NAME);
        let data_dir = self.config.data_dir.clone();
        let tx = self.auto.tx.clone();
        let stamp = local_now();
        tokio::task::spawn_blocking(move || {
            let started = Instant::now();
            let result = auto::write_auto(kind, &folder, &history_db, &lists.input(), stamp);
            let took = started.elapsed();
            if let (AutoKind::Update, Err(e)) = (kind, &result) {
                leave_notice(&data_dir, &AutoBackupFailure { kind, at_ms: super::now_ms(), detail: failure_detail(e) });
            }
            if let Some(reply) = reply {
                reply.send(result.as_ref().map(|(file, _)| file.view()).map_err(failure_detail));
            }
            let _ = tx.blocking_send(AutoInbound::Written { kind, folder, result, took });
        });
    }

    /// A scan or a write came back at `now_ms`.
    pub(super) fn on_auto_backup(&mut self, inbound: AutoInbound, now_ms: u64) {
        match inbound {
            AutoInbound::Scanned { folder, result } => {
                self.auto.scanning = false;
                // The setting moved on meanwhile: the next tick looks at the folder it names.
                if folder != self.auto_folder() {
                    return;
                }
                self.auto.newest = match result {
                    Ok(files) => files.into_iter().next(),
                    Err(e) => {
                        tracing::warn!(error = %e, "the automatic backups' folder could not be read");
                        None
                    }
                };
                self.auto.scanned = Some(folder);
                self.emit_auto_backup();
            }
            AutoInbound::Written { kind, folder, result, took } => {
                self.auto.writing = false;
                match result {
                    Ok((file, summary)) => {
                        tracing::info!(
                            kind = kind.as_str(),
                            file = %file.name,
                            entries = summary.counts.history,
                            bytes = summary.bytes,
                            ms = ms(took),
                            "automatic backup written"
                        );
                        if folder == self.auto_folder() {
                            self.auto.newest = Some(file);
                        }
                        self.auto.failure = None;
                        self.auto.retry_at_ms = None;
                    }
                    Err(e) => {
                        tracing::warn!(kind = kind.as_str(), error = %e, ms = ms(took), "automatic backup failed");
                        self.auto.failure = Some(AutoBackupFailure { kind, at_ms: now_ms, detail: failure_detail(&e) });
                        if kind == AutoKind::Weekly {
                            self.auto.retry_at_ms = Some(now_ms.saturating_add(DAY_MS));
                        }
                    }
                }
                self.emit_auto_backup();
                self.next_auto_backup();
            }
        }
    }

    /// `SetBackupFolder`: `folder` (absolute), or the default for `None`. Another folder is looked
    /// at anew, with nothing failed there yet.
    pub(super) fn set_backup_folder(&mut self, folder: Option<PathBuf>) -> Result<(), CoreError> {
        let folder = match folder {
            None => None,
            Some(path) => {
                if !path.is_absolute() {
                    return Err(CoreError::Invalid("backup.folder: 自动备份位置必须是完整路径".into()));
                }
                let text = path.to_str().ok_or_else(|| CoreError::Invalid("backup.folder: 文件夹路径包含无法识别的字符".into()))?;
                (path != default_folder(&self.config.data_dir)).then(|| text.to_owned())
            }
        };
        if folder == self.settings.backup.folder {
            return Ok(());
        }
        self.settings.backup.folder = folder;
        self.save_settings()?;
        self.auto.scanned = None;
        self.auto.newest = None;
        self.auto.failure = None;
        self.auto.retry_at_ms = None;
        self.emit_auto_backup();
        Ok(())
    }

    /// `BackupNoticeDismiss`.
    pub(super) fn dismiss_backup_notice(&mut self) {
        if self.auto.notice.take().is_some() {
            self.emit_auto_backup();
        }
    }

    /// Write an automatic backup of `kind` here and now, on the core task: the one before a
    /// replacing import, which must hold everything the core did before it. The core does nothing
    /// else meanwhile; the time it took is logged.
    pub(super) fn write_auto_now(&mut self, kind: AutoKind) -> Result<AutoFile, BackupError> {
        let lists = self.auto_lists();
        let history_db = self.config.data_dir.join(HISTORY_DB_FILE_NAME);
        let started = Instant::now();
        let result = auto::write_auto(kind, &self.auto_folder(), &history_db, &lists.input(), local_now());
        let took = ms(started.elapsed());
        match result {
            Ok((file, summary)) => {
                tracing::info!(kind = kind.as_str(), file = %file.name, entries = summary.counts.history, bytes = summary.bytes, ms = took, "automatic backup written");
                self.auto.newest = Some(file.clone());
                self.auto.failure = None;
                self.auto.retry_at_ms = None;
                self.emit_auto_backup();
                Ok(file)
            }
            Err(e) => {
                tracing::warn!(kind = kind.as_str(), error = %e, ms = took, "automatic backup failed");
                Err(e)
            }
        }
    }

    /// A replacing import while a take runs is refused (`runtime/backup.rs`).
    pub(super) fn import_needs_the_take_to_end(&self) -> bool {
        self.take_running()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::sync::Arc;

    use chrono::TimeZone as _;
    use tokio::sync::mpsc::Receiver;

    use super::*;
    use crate::backup::auto::{AUTO_BACKUP_DIR, DAY_MS, WEEK_MS, auto_name, list};
    use crate::runtime::{AppCore, Inbox};
    use crate::settings::BackupSettings;
    use crate::vocabulary::{DictionaryDraft, EntrySource};
    use crate::{CoreCommand, CoreConfig, Settings, SettingsStore};

    struct Rig {
        rt: Runtime,
        inbox: Inbox,
        _events: Receiver<CoreEvent>,
        dir: tempfile::TempDir,
    }

    fn rig_in(dir: tempfile::TempDir, backup: BackupSettings, auto_backup: bool) -> Rig {
        SettingsStore::new(dir.path()).save(&Settings { relay_enabled: false, backup, ..Settings::default() }).unwrap();
        let mut config = CoreConfig::new(dir.path().to_path_buf());
        config.auto_backup = auto_backup;
        config.default_device_name = "书房".into();
        let store: Arc<dyn voltip_identity::SecretStore> = Arc::new(voltip_identity::MemorySecretStore::new());
        let (rt, inbox, _handle, events) = AppCore::build(config, store, crate::dictation::fakes::ports()).unwrap();
        Rig { rt, inbox, _events: events, dir }
    }

    fn rig(backup: BackupSettings) -> Rig {
        rig_in(tempfile::tempdir().unwrap(), backup, true)
    }

    impl Rig {
        fn started(&self) -> u64 {
            self.rt.auto.started_ms
        }

        fn default_folder(&self) -> PathBuf {
            self.dir.path().join(AUTO_BACKUP_DIR)
        }

        /// What the core hears next about automatic backups (a scan or a write on its thread).
        async fn next(&mut self) -> AutoInbound {
            tokio::time::timeout(Duration::from_secs(30), self.inbox.auto_rx.recv()).await.expect("an answer within 30 s").expect("the core holds the sender")
        }

        /// A tick at `now`, and the answer of what it started handed back at `now`.
        async fn tick_and_answer(&mut self, now: u64) {
            self.rt.backup_tick_at(now);
            let inbound = self.next().await;
            self.rt.on_auto_backup(inbound, now);
        }

        /// Nothing is out: no scan, no write, none waiting.
        fn idle(&self) -> bool {
            !self.rt.auto.scanning && !self.rt.auto.writing && self.rt.auto.waiting.is_empty()
        }
    }

    /// A file named as an automatic backup made `age_ms` before now (local time, as the names are).
    fn made_ago(folder: &Path, kind: AutoKind, age_ms: u64) -> PathBuf {
        std::fs::create_dir_all(folder).unwrap();
        let at = crate::now_ms() - age_ms;
        let stamp = chrono::Local.timestamp_millis_opt(i64::try_from(at).unwrap()).unwrap().naive_local();
        let path = folder.join(auto_name(kind, stamp, 1));
        std::fs::write(&path, b"{}").unwrap();
        path
    }

    /// docs/dictation.md §4.6.1: nothing before the folder is looked at a minute after start; then
    /// a folder without an automatic backup gets the weekly one at once, which `UiState` names.
    #[tokio::test]
    async fn a_weekly_backup_is_written_a_minute_after_start_when_none_is_there() {
        let mut rig = rig(BackupSettings::default());
        let t0 = rig.started();
        rig.rt.backup_tick_at(t0 + AUTO_SCAN_DELAY.as_millis() as u64 - 1_000);
        assert!(rig.idle(), "not before the folder is looked at");
        rig.tick_and_answer(t0 + AUTO_SCAN_DELAY.as_millis() as u64).await;
        rig.tick_and_answer(t0 + 61_000).await;
        let files = list(&rig.default_folder()).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].kind, AutoKind::Weekly);
        let backup = crate::backup::read(&files[0].path).unwrap();
        assert_eq!(backup.manifest.device, "书房");
        let view = rig.rt.auto_view();
        assert!(view.available && !view.custom);
        assert_eq!(view.folder, rig.default_folder().display().to_string());
        assert_eq!(view.default_folder, view.folder);
        assert_eq!(view.last, Some(files[0].view()));
        assert_eq!((view.failure, view.notice), (None, None));
        // Written: not again for a week.
        rig.rt.backup_tick_at(t0 + 62_000);
        rig.rt.backup_tick_at(t0 + WEEK_MS - DAY_MS);
        assert!(rig.idle());
    }

    /// A backup made two days ago is new enough: the weekly one waits until it is a week old.
    #[tokio::test]
    async fn the_weekly_backup_waits_until_the_newest_one_is_a_week_old() {
        let dir = tempfile::tempdir().unwrap();
        let recent = made_ago(&dir.path().join(AUTO_BACKUP_DIR), AutoKind::Update, 2 * DAY_MS);
        let mut rig = rig_in(dir, BackupSettings::default(), true);
        let t0 = rig.started();
        rig.tick_and_answer(t0 + 60_000).await;
        assert_eq!(rig.rt.auto_view().last.map(|l| l.name), recent.file_name().map(|n| n.to_string_lossy().into_owned()));
        rig.rt.backup_tick_at(t0 + 61_000);
        rig.rt.backup_tick_at(t0 + 4 * DAY_MS);
        assert!(rig.idle(), "two days old, then six: not due");
        rig.tick_and_answer(t0 + 5 * DAY_MS + 60_000).await;
        assert_eq!(list(&rig.default_folder()).unwrap().len(), 2, "a week old: the weekly one");
    }

    /// Switched off, there is no weekly backup, however old the newest one is.
    #[tokio::test]
    async fn no_weekly_backup_when_it_is_switched_off() {
        let mut rig = rig(BackupSettings { weekly: false, folder: None });
        let t0 = rig.started();
        rig.tick_and_answer(t0 + 60_000).await;
        rig.rt.backup_tick_at(t0 + 61_000);
        rig.rt.backup_tick_at(t0 + 30 * DAY_MS);
        assert!(rig.idle());
        assert_eq!(list(&rig.default_folder()).unwrap(), Vec::new());
        // On again: due at the next tick.
        rig.rt.handle_command(CoreCommand::SetBackupWeekly(true)).await;
        assert!(rig.rt.settings.backup.weekly);
        rig.tick_and_answer(t0 + 30 * DAY_MS + 1_000).await;
        assert_eq!(list(&rig.default_folder()).unwrap().len(), 1);
    }

    /// Another folder is looked at anew; the backups go there from then on, and the old folder
    /// keeps what it has.
    #[tokio::test]
    async fn a_new_folder_is_looked_at_and_gets_the_backups() {
        let mut rig = rig(BackupSettings::default());
        let t0 = rig.started();
        rig.tick_and_answer(t0 + 60_000).await;
        rig.tick_and_answer(t0 + 61_000).await;
        assert_eq!(list(&rig.default_folder()).unwrap().len(), 1);
        let synced = rig.dir.path().join("同步盘").join("Voltip");
        rig.rt.handle_command(CoreCommand::SetBackupFolder(Some(synced.clone()))).await;
        assert_eq!(rig.rt.settings.backup.folder.as_deref(), Some(synced.to_str().unwrap()));
        let view = rig.rt.auto_view();
        assert!(view.custom);
        assert_eq!((view.folder, view.last), (synced.display().to_string(), None), "not looked at yet");
        rig.tick_and_answer(t0 + 62_000).await;
        rig.tick_and_answer(t0 + 63_000).await;
        assert_eq!(list(&synced).unwrap().len(), 1, "the new folder has none: the weekly one goes there");
        assert_eq!(list(&rig.default_folder()).unwrap().len(), 1, "the old one keeps its backup");
        // Back to the default: its backup is a minute old.
        rig.rt.handle_command(CoreCommand::SetBackupFolder(None)).await;
        assert_eq!(rig.rt.settings.backup.folder, None);
        rig.tick_and_answer(t0 + 64_000).await;
        rig.rt.backup_tick_at(t0 + 65_000);
        assert!(rig.idle());
        assert!(!rig.rt.auto_view().custom);
        // A relative folder is refused and changes nothing.
        rig.rt.handle_command(CoreCommand::SetBackupFolder(Some(PathBuf::from("relative/backups")))).await;
        assert_eq!(rig.rt.settings.backup.folder, None);
    }

    /// docs/dictation.md §4.6.1: a weekly backup that fails is shown, and not tried again for a
    /// day; one that succeeds then takes the failure away.
    #[tokio::test]
    async fn a_failed_weekly_backup_is_shown_and_tried_again_a_day_later() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("a file");
        std::fs::write(&blocker, b"x").unwrap();
        let folder = blocker.join(AUTO_BACKUP_DIR);
        let mut rig = rig_in(dir, BackupSettings { weekly: true, folder: Some(folder.display().to_string()) }, true);
        let t0 = rig.started();
        rig.tick_and_answer(t0 + 60_000).await;
        let failed_at = t0 + 61_000;
        rig.tick_and_answer(failed_at).await;
        let failure = rig.rt.auto_view().failure.expect("the failure is shown");
        assert_eq!((failure.kind, failure.at_ms), (AutoKind::Weekly, failed_at));
        assert!(!failure.detail.is_empty());
        rig.rt.backup_tick_at(failed_at + DAY_MS - 1);
        assert!(rig.idle(), "not again within a day");
        std::fs::remove_file(&blocker).unwrap();
        rig.tick_and_answer(failed_at + DAY_MS).await;
        assert_eq!(list(&folder).unwrap().len(), 1);
        assert_eq!(rig.rt.auto_view().failure, None, "gone once one succeeds");
    }

    /// The phone and the local speech service make no weekly backup and show none.
    #[tokio::test]
    async fn a_core_without_automatic_backups_never_writes_one() {
        let mut rig = rig_in(tempfile::tempdir().unwrap(), BackupSettings::default(), false);
        let t0 = rig.started();
        for at in [t0 + 60_000, t0 + 61_000, t0 + 8 * DAY_MS, t0 + 60 * DAY_MS] {
            rig.rt.backup_tick_at(at);
            assert!(rig.idle());
        }
        assert!(!rig.default_folder().exists());
        assert!(!rig.rt.auto_view().available);
    }

    /// The shell's backup before an update: written of the runtime's own lists (a dictionary entry
    /// added just before is in it), and the answer names the file.
    #[tokio::test]
    async fn the_backup_before_an_update_answers_with_its_file() {
        let mut rig = rig(BackupSettings::default());
        rig.rt
            .handle_command(CoreCommand::DictionaryAdd {
                draft: DictionaryDraft { term: "Voltip".into(), heard_as: vec!["沃提普".into()], enabled: true },
                source: EntrySource::Manual,
            })
            .await;
        let (reply, answer) = AutoReply::channel();
        rig.rt.handle_command(CoreCommand::BackupAuto { kind: AutoKind::Update, reply }).await;
        let inbound = rig.next().await;
        rig.rt.on_auto_backup(inbound, crate::now_ms());
        let file = answer.await.unwrap().expect("written");
        assert_eq!(file.kind, AutoKind::Update);
        let backup = crate::backup::read(&rig.default_folder().join(&file.name)).unwrap();
        assert_eq!(backup.dictionary.iter().map(|d| d.term.as_str()).collect::<Vec<_>>(), ["Voltip"]);
        assert_eq!(rig.rt.auto_view().last, Some(file));
    }

    /// One at a time: the backup before an update waits for the weekly one being written, then
    /// goes.
    #[tokio::test]
    async fn the_backup_before_an_update_waits_for_the_one_being_written() {
        let mut rig = rig(BackupSettings::default());
        let t0 = rig.started();
        rig.tick_and_answer(t0 + 60_000).await;
        rig.rt.backup_tick_at(t0 + 61_000);
        assert!(rig.rt.auto.writing, "the weekly one is being written");
        let (reply, mut answer) = AutoReply::channel();
        rig.rt.request_auto_backup(AutoKind::Update, Some(reply));
        assert_eq!(rig.rt.auto.waiting.len(), 1, "the update waits");
        let weekly = rig.next().await;
        rig.rt.on_auto_backup(weekly, t0 + 62_000);
        assert!(rig.rt.auto.writing && rig.rt.auto.waiting.is_empty(), "the update is being written");
        assert!(answer.try_recv().is_err(), "not answered yet");
        let update = rig.next().await;
        rig.rt.on_auto_backup(update, t0 + 63_000);
        assert_eq!(answer.await.unwrap().unwrap().kind, AutoKind::Update);
        let kinds: Vec<AutoKind> = list(&rig.default_folder()).unwrap().iter().map(|f| f.kind).collect();
        assert!(kinds.contains(&AutoKind::Weekly) && kinds.contains(&AutoKind::Update), "{kinds:?}");
    }

    /// docs/dictation.md §4.6.1: a backup before an update that fails answers with the reason (the
    /// update goes ahead) and leaves a notice the next start shows once, until it is closed.
    #[tokio::test]
    async fn a_failed_backup_before_an_update_is_shown_once_after_the_restart() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("a file");
        std::fs::write(&blocker, b"x").unwrap();
        let folder = blocker.join(AUTO_BACKUP_DIR);
        let backup = BackupSettings { weekly: true, folder: Some(folder.display().to_string()) };
        let mut rig = rig_in(dir, backup.clone(), true);
        let (reply, answer) = AutoReply::channel();
        rig.rt.request_auto_backup(AutoKind::Update, Some(reply));
        let inbound = rig.next().await;
        rig.rt.on_auto_backup(inbound, crate::now_ms());
        let reason = answer.await.unwrap().expect_err("not written");
        assert!(!reason.is_empty());
        assert_eq!(rig.rt.auto_view().failure.map(|f| f.kind), Some(AutoKind::Update));
        let notice_file = rig.dir.path().join(NOTICE_FILE_NAME);
        assert!(notice_file.exists(), "written before the answer: the update may end the process");
        assert_eq!(rig.rt.auto_view().notice, None, "this run is the one that failed");

        // The next start shows it, once.
        let Rig { dir, .. } = rig;
        let mut after = rig_in(dir, backup.clone(), true);
        let notice = after.rt.auto_view().notice.expect("shown after the restart");
        assert_eq!((notice.kind, notice.detail.as_str()), (AutoKind::Update, reason.as_str()));
        assert_eq!(after.rt.auto_view().failure, Some(notice));
        assert!(!notice_file.exists(), "shown once");
        after.rt.handle_command(CoreCommand::BackupNoticeDismiss).await;
        assert_eq!(after.rt.auto_view().notice, None);
        let Rig { dir, .. } = after;
        assert_eq!(rig_in(dir, backup, true).rt.auto_view().notice, None, "not again");
    }

    /// The backup before a replacing import is written on the core task itself, at once.
    #[tokio::test]
    async fn the_backup_before_an_import_is_written_at_once() {
        let mut rig = rig(BackupSettings::default());
        let file = rig.rt.write_auto_now(AutoKind::Import).unwrap();
        assert_eq!(file.kind, AutoKind::Import);
        assert!(file.path.starts_with(rig.default_folder()));
        assert_eq!(rig.rt.auto_view().last, Some(file.view()));
        assert!(rig.idle(), "nothing left on another thread");
    }
}
