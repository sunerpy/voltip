//! The audio cache's side of the runtime (docs/dictation.md §26): its settings and commands, the
//! engine's effects that keep a take's recording and say what became of it, the long takes an
//! earlier run left behind, and the history changes the recordings follow — an entry deleted or
//! dropped takes its recording along, a cleared history all of them.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::mpsc;
use uuid::Uuid;

use super::{CoreConfig, CoreEvent, Runtime, now_ms};
use crate::CoreError;
use crate::audio_cache::{self, AudioCache, CACHE_UNAVAILABLE, KeptAudio, KeptRead, KeptStatus, Limits, NewRecording, RECORDING_GONE, RetakeModels};
use crate::dictation::archive::{self, ArchiveReport, Leftover};
use crate::dictation::long;
use crate::dictation::{Retake, RetakeAudio, TakeClients};
use crate::engines::{ResolvedEngines, ServiceKind};
use crate::history::HistoryStore;
use crate::settings::{AudioCacheSettings, Settings};

/// Whether the takes keep their recordings: this app keeps some, the cache is on, and so is the
/// history (a recording is reached from it).
pub(super) fn keeps_audio(config: &CoreConfig, settings: &Settings) -> bool {
    config.keeps_audio && settings.audio_cache.enabled && settings.history.enabled
}

/// The long takes an earlier run left behind in `<data_dir>/recordings` (it stopped while
/// recording, docs/dictation.md §26.3): the ones with a copy are put together once the core runs
/// ([`Runtime::start_recovery`]); the others, and every one without the cache, go now, as before.
pub(super) fn leftovers_at_start(config: &CoreConfig, settings: &Settings) -> Vec<Leftover> {
    let recordings = long::recordings_dir(&config.data_dir);
    if !keeps_audio(config, settings) {
        let cleared = long::clear_leftovers(&recordings);
        if cleared > 0 {
            tracing::info!(cleared, "removed the recording files an earlier run left behind");
        }
        return Vec::new();
    }
    let (copies, bare): (Vec<Leftover>, Vec<Leftover>) = archive::leftovers(&recordings).into_iter().partition(|l| l.dir.is_some());
    for pcm in bare.iter().filter_map(|l| l.pcm.as_deref()) {
        long::remove(pcm);
    }
    if !bare.is_empty() {
        tracing::info!(cleared = bare.len(), "removed the recording files an earlier run left behind without a copy");
    }
    copies
}

/// A leftover take put together on a blocking thread.
#[derive(Debug)]
pub(super) struct Recovered {
    report: ArchiveReport,
    at_ms: u64,
}

/// The leftover takes still to be put together, and the way back.
pub(super) struct Recovery {
    pub(super) leftovers: Vec<Leftover>,
    /// Set when the core stops: what is not done yet waits for the next start.
    pub(super) stop: Arc<AtomicBool>,
    pub(super) tx: mpsc::Sender<Recovered>,
}

/// The cache as the app starts: emptied when it is off, within its limits, and without the
/// recordings whose entries are gone.
pub(super) fn audio_at_start(audio: &mut AudioCache, settings: &AudioCacheSettings, history: &HistoryStore) -> Result<(), CoreError> {
    if !audio.is_available() {
        return Ok(());
    }
    if !settings.enabled {
        let removed = audio.clear()?;
        if removed > 0 {
            tracing::info!(removed, "the audio cache is off; its recordings were deleted");
        }
        return Ok(());
    }
    audio.enforce(Limits::from(settings))?;
    let existing = history.existing(&audio.linked_entries())?;
    audio.remove_unlinked(&existing)?;
    Ok(())
}

impl Runtime {
    /// Tell the interface what the cache holds.
    pub(super) fn emit_audio_cache(&self) {
        self.emit(CoreEvent::AudioCache(self.audio.view()));
    }

    /// `SetAudioCache`: persisted; off deletes the kept recordings, tighter limits drop the oldest.
    pub(super) fn set_audio_cache(&mut self, settings: AudioCacheSettings) -> Result<(), CoreError> {
        audio_cache::validate(&settings).map_err(CoreError::Invalid)?;
        self.settings.audio_cache = settings;
        self.save_settings()?;
        self.dictation.set_keep_audio(keeps_audio(&self.config, &self.settings));
        let changed = if settings.enabled { self.audio.enforce(Limits::from(&settings))? } else { self.audio.clear()? > 0 };
        if changed {
            self.emit_audio_cache();
        }
        Ok(())
    }

    /// `AudioCacheClear`.
    pub(super) fn audio_cache_clear(&mut self) -> Result<(), CoreError> {
        if self.audio.clear()? > 0 {
            self.emit_audio_cache();
        }
        Ok(())
    }

    /// `AudioCacheDelete`.
    pub(super) fn audio_cache_delete(&mut self, id: Uuid) -> Result<(), CoreError> {
        if self.audio.remove(id)? {
            self.emit_audio_cache();
        }
        Ok(())
    }

    /// `AudioRetake`: the kept recording through the pipeline again, as a take of its own, on the
    /// settings' models or the ones chosen for it (docs/dictation.md §26.4).
    pub(super) fn audio_retake(&mut self, id: Uuid, models: RetakeModels) -> Result<(), CoreError> {
        if !self.audio.is_available() {
            return Err(CoreError::Invalid(CACHE_UNAVAILABLE.to_owned()));
        }
        let clients = if models.is_empty() {
            self.check_recogniser()?;
            None
        } else {
            Some(self.retake_clients(&models)?)
        };
        let Some((recording, audio)) = self.audio.read(id)? else {
            // Reading it may have found its file gone and forgotten it.
            self.emit_audio_cache();
            return Err(CoreError::Invalid(RECORDING_GONE.to_owned()));
        };
        let audio = match audio {
            KeptRead::Wav(wav) => RetakeAudio::Wav(wav),
            KeptRead::Long(kept) => RetakeAudio::Long(kept),
        };
        let effects = self.dictation.retake(Retake { recording: id, audio, duration_ms: recording.duration_ms, app: recording.app, clients })?;
        self.apply_dictation(effects);
        Ok(())
    }

    /// The clients a 重新识别 on `models` runs with: the settings with those choices made, checked
    /// as a saved change would be and resolved, but neither saved nor applied to other takes.
    fn retake_clients(&self, models: &RetakeModels) -> Result<TakeClients, CoreError> {
        let engines = models.apply(&self.settings.engines);
        self.check_engines(&engines)?;
        let resolved = ResolvedEngines::resolve_with_models(&engines, &self.user_secrets, &self.built_in, &self.library);
        Self::check_recognition(&resolved)?;
        if models.refine() == Some(true)
            && let Some(issue) = resolved.refine_issue
        {
            return Err(CoreError::Invalid(issue.message(ServiceKind::Llm).to_owned()));
        }
        Ok(self.dictation.clients_for(&resolved, models.refine()))
    }

    /// `Effect::Keep`: the take's recording into the cache, the oldest out past the limits. A
    /// recording that cannot be kept is logged (and a long take's copy removed): the take goes on
    /// without it.
    pub(super) fn keep_recording(&mut self, new: NewRecording) {
        let parts = match &new.audio {
            KeptAudio::Parts { dir, .. } => Some(dir.clone()),
            KeptAudio::Wav(_) => None,
        };
        if keeps_audio(&self.config, &self.settings) {
            match self.audio.keep(new, Limits::from(&self.settings.audio_cache)) {
                Ok(()) => self.emit_audio_cache(),
                Err(e) => tracing::warn!(error = %e, "the take's recording could not be kept"),
            }
        }
        // A long take's copy that was not moved into the cache: nothing else removes it.
        if let Some(dir) = parts.filter(|dir| dir.exists()) {
            remove_dir(&dir);
        }
    }

    /// Put the leftover takes together, one after the other on a blocking thread; each reports as
    /// [`Recovered`].
    pub(super) fn start_recovery(&mut self) {
        let leftovers = std::mem::take(&mut self.recovery.leftovers);
        if leftovers.is_empty() {
            return;
        }
        tracing::info!(takes = leftovers.len(), "putting together the takes an earlier run left behind");
        let (stop, tx) = (self.recovery.stop.clone(), self.recovery.tx.clone());
        tokio::task::spawn_blocking(move || {
            for leftover in leftovers {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                if let Some(report) = archive::recover(&leftover, &stop) {
                    let at_ms = leftover.at_ms.unwrap_or_else(now_ms);
                    if tx.blocking_send(Recovered { report, at_ms }).is_err() {
                        break;
                    }
                }
            }
        });
    }

    /// A leftover take is put together: it joins the cache as interrupted, to be recognised again.
    pub(super) fn on_recovered(&mut self, recovered: Recovered) {
        let Recovered { report, at_ms } = recovered;
        if !keeps_audio(&self.config, &self.settings) {
            remove_dir(&report.dir);
            return;
        }
        let id = Uuid::new_v4();
        let new = NewRecording {
            id,
            at_ms,
            duration_ms: report.manifest.total * 1000 / long::RATE,
            audio: KeptAudio::Parts { dir: report.dir.clone(), bytes: report.bytes },
            app: None,
        };
        match self.audio.keep(new, Limits::from(&self.settings.audio_cache)) {
            Ok(()) => {
                if let Err(e) = self.audio.set_status(id, KeptStatus::Interrupted) {
                    tracing::warn!(error = %e, "a recovered take's status could not be saved");
                }
                tracing::info!(recording = %id, seconds = report.manifest.total / long::RATE, "a take an earlier run left behind is kept");
                self.emit_audio_cache();
            }
            Err(e) => {
                tracing::warn!(error = %e, "a take an earlier run left behind could not be kept");
                remove_dir(&report.dir);
            }
        }
    }

    /// `Effect::Kept`: what became of a kept recording's take. An entry the history did not keep
    /// (history off, a write that failed) cannot reach the recording: it goes. When the history
    /// cannot say, the link is kept; the next start sweeps it if the entry is gone.
    pub(super) fn kept_status(&mut self, id: Uuid, status: KeptStatus) {
        let gone = match status.entry() {
            Some(entry) => match self.history.existing(&[entry]) {
                Ok(found) => !found.contains(&entry),
                Err(e) => {
                    tracing::warn!(error = %e, "could not look up the entry a kept recording was linked to");
                    false
                }
            },
            None => false,
        };
        let result = if gone { self.audio.remove(id) } else { self.audio.set_status(id, status) };
        match result {
            Ok(true) => self.emit_audio_cache(),
            Ok(false) => {}
            Err(e) => tracing::warn!(error = %e, "a kept recording's status could not be saved"),
        }
    }

    /// An entry was deleted: its recording goes with it.
    pub(super) fn audio_entry_deleted(&mut self, entry: Uuid) {
        match self.audio.remove_entry(entry) {
            Ok(true) => self.emit_audio_cache(),
            Ok(false) => {}
            Err(e) => tracing::warn!(error = %e, "a deleted entry's recording could not be deleted"),
        }
    }

    /// The history was cleared: every recording goes, the unrecognised ones too.
    pub(super) fn audio_history_cleared(&mut self) {
        match self.audio.clear() {
            Ok(0) => {}
            Ok(_) => self.emit_audio_cache(),
            Err(e) => tracing::warn!(error = %e, "the kept recordings could not be deleted with the history"),
        }
    }

    /// The history dropped or replaced entries: the recordings of the ones that are gone go too.
    pub(super) fn audio_follow_history(&mut self) {
        let linked = self.audio.linked_entries();
        if linked.is_empty() {
            return;
        }
        let result = self.history.existing(&linked).and_then(|existing| self.audio.remove_unlinked(&existing));
        match result {
            Ok(true) => self.emit_audio_cache(),
            Ok(false) => {}
            Err(e) => tracing::warn!(error = %e, "the kept recordings could not follow the history"),
        }
    }
}

/// Remove a long take's copy that is not kept; one that is gone is not an error.
fn remove_dir(dir: &Path) {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!(dir = %dir.display(), error = %e, "a long take's copy could not be removed"),
    }
}
