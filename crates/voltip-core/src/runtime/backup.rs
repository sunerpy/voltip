//! A backup's import in the core task (docs/dictation.md §4.6): each chosen part is merged with the
//! list here or put in its place ([`crate::backup::merge`]), checked again and written by its store
//! in one go, then announced like any other change of that list, so every page, the engine and the
//! phones that sync follow. A merge that adds nothing writes nothing. A part that cannot be written
//! stays as it was; the others go on.

use std::collections::HashMap;

use super::{Runtime, now_ms};
use crate::backup::{Backup, BackupPart, BackupParts, ImportReport, PartFailure, PartReport, count, merge};
use crate::vocabulary::ImportMode;

impl Runtime {
    /// Restore `parts` of `backup`: the presets first (the scenes may name them), the history last.
    pub(super) fn import_backup(&mut self, backup: &Backup, parts: BackupParts, mode: ImportMode) -> ImportReport {
        let mut report = ImportReport { mode, parts: Vec::new() };
        // A preset of the backup skipped for one of the same name here: its scenes name that one.
        let mut same_preset = HashMap::new();
        if parts.presets {
            let (merge::Merged { list, report: part }, same) = merge::presets(self.presets.presets(), &backup.presets, mode);
            same_preset = same;
            report.parts.push(if unchanged(mode, &part) {
                part
            } else {
                match self.presets.restore_list(list) {
                    Ok(()) => {
                        let _ = self.presets_changed(Ok(()));
                        part
                    }
                    Err(e) => not_written(part, &e),
                }
            });
        }
        if parts.scenes {
            let merge::Merged { list, report: part } =
                merge::scenes(self.scenes.scenes(), &backup.scenes, mode, self.scenes.platform(), backup.manifest.platform, &same_preset);
            report.parts.push(if unchanged(mode, &part) {
                part
            } else {
                match self.scenes.restore_list(list, now_ms()) {
                    Ok(()) => {
                        let _ = self.scenes_changed(Ok(()));
                        part
                    }
                    Err(e) => not_written(part, &e),
                }
            });
        }
        if parts.dictionary {
            let merge::Merged { list, report: part } = merge::dictionary(self.dictionary.entries(), &backup.dictionary, mode);
            report.parts.push(if unchanged(mode, &part) {
                part
            } else {
                match self.dictionary.restore_list(list) {
                    Ok(()) => {
                        let _ = self.dictionary_changed(Ok(()));
                        part
                    }
                    Err(e) => not_written(part, &e),
                }
            });
        }
        if parts.rules {
            let merge::Merged { list, report: part } = merge::rules(self.rules.rules(), &backup.rules, mode);
            report.parts.push(if unchanged(mode, &part) {
                part
            } else {
                match self.rules.restore_list(list) {
                    Ok(()) => {
                        let _ = self.rules_changed(Ok(()));
                        part
                    }
                    Err(e) => not_written(part, &e),
                }
            });
        }
        if parts.history {
            report.parts.push(self.import_history(backup, mode));
        }
        tracing::info!(?mode, parts = report.parts.len(), "backup imported");
        report
    }

    /// The history part: the newest `Settings.history.keep` of the backup's entries and the ones
    /// here (both merged, or the backup's alone), in one transaction.
    fn import_history(&mut self, backup: &Backup, mode: ImportMode) -> PartReport {
        let mut part = PartReport::new(BackupPart::History, backup.history.len());
        if !self.history.is_open() {
            tracing::warn!("backup history not restored: the history database is not open");
            return part.failed(PartFailure::Unavailable);
        }
        match self.history.restore(&backup.history, mode == ImportMode::Replace, self.settings.history.keep as usize) {
            Ok(done) => {
                part.restored = count(done.written);
                part.skipped.duplicate = count(done.present);
                part.skipped.older = count(done.left_out);
                part.removed = count(done.removed);
                if mode == ImportMode::Replace || done.written > 0 || done.removed > 0 {
                    self.emit_history();
                    // The entries a replace or the limit removed take their recordings along (§26).
                    self.audio_follow_history();
                }
                part
            }
            Err(e) => not_written(part, &e),
        }
    }
}

/// A merge that added nothing: the list here is the list it would write.
fn unchanged(mode: ImportMode, part: &PartReport) -> bool {
    mode == ImportMode::Merge && part.restored == 0
}

/// The part failed to be written: nothing of it changed.
fn not_written(part: PartReport, error: &dyn std::fmt::Display) -> PartReport {
    tracing::warn!(part = ?part.part, error = %error, "backup part not restored; it stays as it was");
    part.failed(PartFailure::Write)
}
