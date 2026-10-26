//! The audio cache (docs/dictation.md §26): the recordings of the newest takes, kept in the app
//! data directory so a take whose recognition failed — or whose text came out wrong — can be
//! recognised again without being spoken again.
//!
//! A take of up to two minutes is one WAV, exactly what the recogniser was given, as
//! `<data_dir>/audio-cache/<id>.wav`; a long take is a directory `<id>/` of 30 s Ogg Opus parts and
//! their manifest (`dictation::archive`, written while the take was recorded). [`INDEX_FILE_NAME`]
//! beside them says when each was recorded, how long it is, the application it was recorded in,
//! and what became of it ([`KeptStatus`]). Local only: no recording is synced to a phone, put in a
//! backup or offered to the MCP tools; on Unix the files are this user's only.
//!
//! The runtime owns the only [`AudioCache`]. A WAV is written whole under a temporary name and
//! renamed before the index names it; a long take's directory is moved in whole; every change
//! writes the index the same way. At start, a recording the index names whose file is gone is
//! forgotten, a file or directory it does not name is deleted, and a take that was still being
//! processed when the app stopped reads [`KeptStatus::Interrupted`]. An index that cannot be used
//! leaves its recordings unaccounted for: they are deleted and the cache starts empty — it is a
//! cache.

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::CoreError;
use crate::dictation::FailureCode;
use crate::dictation::archive::{self, KeptLong};
use crate::engines::EngineSettings;
use crate::providers::ProviderId;
use crate::scenes::AppRef;
use crate::settings::AudioCacheSettings;

/// The cache's directory, under the data directory.
pub const AUDIO_CACHE_DIR: &str = "audio-cache";
/// What each recording is for, in the cache's directory.
pub const INDEX_FILE_NAME: &str = "index.json";
/// Schema version of the index.
const INDEX_SCHEMA: u16 = 1;
/// The fewest recordings `AudioCacheSettings.keep` may ask for.
pub const MIN_KEEP: u32 = 1;
/// The most recordings `AudioCacheSettings.keep` may ask for.
pub const MAX_KEEP: u32 = 200;
/// Recordings kept unless the settings say otherwise.
pub const DEFAULT_KEEP: u32 = 20;
/// The least space `AudioCacheSettings.max_mb` may allow, in MB.
pub const MIN_MB: u32 = 50;
/// The most space `AudioCacheSettings.max_mb` may allow, in MB (10 GB).
pub const MAX_MB: u32 = 10_000;
/// The space allowed unless the settings say otherwise, in MB.
pub const DEFAULT_MAX_MB: u32 = 200;
/// A megabyte as the settings count it.
pub const MB: u64 = 1_000_000;
/// Why a recording cannot be recognised again: the cache no longer holds it.
pub const RECORDING_GONE: &str = "这段录音已不在录音缓存中";
/// Why nothing can be recognised again here: this app keeps no recordings.
pub const CACHE_UNAVAILABLE: &str = "这台设备不保留录音";

/// The recognition a 重新识别 runs with instead of the settings' (docs/dictation.md §26.4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecognitionChoice {
    /// A provider's model, the built-in service's included.
    Remote {
        /// Whose model.
        provider: ProviderId,
        /// The model.
        model: String,
    },
    /// An installed local recognition model.
    Local {
        /// Its catalogue id.
        id: String,
    },
}

/// The clean-up a 重新识别 runs with instead of the settings' (docs/dictation.md §26.4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PolishChoice {
    /// None: the text is the recognised one.
    Off,
    /// A provider's model, whatever the scene of the recording says.
    Remote {
        /// Whose model.
        provider: ProviderId,
        /// The model.
        model: String,
    },
}

/// The models one 重新识别 runs with (docs/dictation.md §26.4); what is left out is the settings'.
/// Only that take uses them: the settings stay as they are.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetakeModels {
    /// The recognition, when it is not the settings'.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asr: Option<RecognitionChoice>,
    /// The clean-up, when it is not the settings'.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refine: Option<PolishChoice>,
}

impl RetakeModels {
    /// Nothing chosen: the settings' models.
    pub fn is_empty(&self) -> bool {
        self.asr.is_none() && self.refine.is_none()
    }

    /// `engines` with these choices made, as switching to them in the settings would (the endpoint
    /// a provider was set up with stays).
    pub fn apply(&self, engines: &EngineSettings) -> EngineSettings {
        let mut next = engines.clone();
        match &self.asr {
            Some(RecognitionChoice::Remote { provider, model }) => {
                next.asr_provider = *provider;
                next.providers.entry(*provider).or_default().asr_model = Some(model.clone());
            }
            Some(RecognitionChoice::Local { id }) => {
                next.asr_provider = ProviderId::Local;
                next.local_model = Some(id.clone());
            }
            None => {}
        }
        match &self.refine {
            Some(PolishChoice::Off) => next.refine_enabled = false,
            Some(PolishChoice::Remote { provider, model }) => {
                next.refine_enabled = true;
                next.llm_provider = *provider;
                next.providers.entry(*provider).or_default().llm_model = Some(model.clone());
            }
            None => {}
        }
        next
    }

    /// Whether the clean-up runs, when chosen: on for a model, off for none.
    pub fn refine(&self) -> Option<bool> {
        self.refine.as_ref().map(|r| matches!(r, PolishChoice::Remote { .. }))
    }
}

/// What became of a kept recording.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum KeptStatus {
    /// Its take is being processed.
    Pending,
    /// A history entry holds its text: the newest one made from it.
    Recognized {
        /// The entry.
        entry: Uuid,
    },
    /// Its recognition gave no text.
    Failed {
        /// The stage that stopped it.
        code: FailureCode,
        /// Why, as the pipeline said it (Chinese; the interface explains `code`).
        message: String,
    },
    /// Its take was cancelled while it was being recognised.
    Cancelled,
    /// The app stopped while its take was being processed.
    Interrupted,
}

impl KeptStatus {
    /// The history entry made from it, if any.
    pub fn entry(&self) -> Option<Uuid> {
        match self {
            Self::Recognized { entry } => Some(*entry),
            _ => None,
        }
    }
}

/// How a kept recording is stored.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeptFormat {
    /// `<id>.wav`: a take of up to two minutes.
    #[default]
    Wav,
    /// `<id>/`: a long take's Opus parts and manifest.
    Opus,
}

impl KeptFormat {
    fn is_wav(&self) -> bool {
        *self == Self::Wav
    }
}

/// One kept recording (`UiState.audio_cache.recordings`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeptRecording {
    /// Its id, also its file's or directory's name.
    pub id: Uuid,
    /// How it is stored; a WAV unless it was a long take.
    #[serde(default, skip_serializing_if = "KeptFormat::is_wav")]
    pub format: KeptFormat,
    /// When it was recorded, Unix milliseconds.
    pub at_ms: u64,
    /// Its length.
    pub duration_ms: u64,
    /// Its size on disk.
    pub bytes: u64,
    /// The application in front when it was recorded (docs/dictation.md §18.6), when known: a
    /// recognition again matches the scenes against it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<AppRef>,
    /// What became of it.
    pub status: KeptStatus,
}

/// `UiState.audio_cache`: what the cache holds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioCacheView {
    /// This app keeps recordings (the desktop; not the phone).
    pub available: bool,
    /// The recordings, newest first.
    pub recordings: Vec<KeptRecording>,
    /// Their size in all.
    pub bytes: u64,
}

/// The audio of a take to keep.
#[derive(Clone, PartialEq, Eq)]
pub enum KeptAudio {
    /// The WAV the recogniser is given (a take of up to two minutes).
    Wav(Vec<u8>),
    /// A long take's parts directory (`dictation::archive`), moved into the cache whole, and its
    /// size.
    Parts {
        /// The directory, under `<data_dir>/recordings`.
        dir: PathBuf,
        /// Its parts' size.
        bytes: u64,
    },
}

/// A take's recording to keep.
#[derive(Clone, PartialEq, Eq)]
pub struct NewRecording {
    /// Its id.
    pub id: Uuid,
    /// When it was recorded (its start), Unix milliseconds.
    pub at_ms: u64,
    /// Its length.
    pub duration_ms: u64,
    /// Its audio.
    pub audio: KeptAudio,
    /// The application in front.
    pub app: Option<AppRef>,
}

impl std::fmt::Debug for NewRecording {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let audio = match &self.audio {
            KeptAudio::Wav(wav) => format!("wav: {} bytes", wav.len()),
            KeptAudio::Parts { bytes, .. } => format!("parts: {bytes} bytes"),
        };
        f.debug_struct("NewRecording")
            .field("id", &self.id)
            .field("at_ms", &self.at_ms)
            .field("duration_ms", &self.duration_ms)
            .field("audio", &audio)
            .field("app", &self.app.is_some())
            .finish()
    }
}

/// A kept recording's audio, read back to be recognised again.
#[derive(Debug)]
pub enum KeptRead {
    /// Its WAV.
    Wav(Vec<u8>),
    /// Its parts.
    Long(KeptLong),
}

/// How much the cache may hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Recordings at most.
    pub keep: usize,
    /// Bytes at most.
    pub max_bytes: u64,
}

impl From<&AudioCacheSettings> for Limits {
    /// Within the bounds [`validate`] checks: a value edited into `settings.json` by hand neither
    /// empties the cache nor lets it grow past them.
    fn from(settings: &AudioCacheSettings) -> Self {
        Self { keep: settings.keep.clamp(MIN_KEEP, MAX_KEEP) as usize, max_bytes: u64::from(settings.max_mb.clamp(MIN_MB, MAX_MB)) * MB }
    }
}

/// Checks `AudioCacheSettings`' bounds; the error names the field.
pub fn validate(settings: &AudioCacheSettings) -> Result<(), String> {
    if !(MIN_KEEP..=MAX_KEEP).contains(&settings.keep) {
        return Err(format!("audio_cache.keep: {MIN_KEEP}–{MAX_KEEP}"));
    }
    if !(MIN_MB..=MAX_MB).contains(&settings.max_mb) {
        return Err(format!("audio_cache.max_mb: {MIN_MB}–{MAX_MB}"));
    }
    Ok(())
}

/// `index.json`.
#[derive(Serialize, Deserialize)]
struct IndexFile {
    schema: u16,
    recordings: Vec<KeptRecording>,
}

/// The kept recordings and their files.
#[derive(Debug)]
pub struct AudioCache {
    /// `None`: this app keeps no recordings (the phone).
    dir: Option<PathBuf>,
    /// Newest first.
    recordings: Vec<KeptRecording>,
}

impl AudioCache {
    /// The cache under `data_dir`, reconciled with the files there.
    pub fn open(data_dir: &Path) -> Self {
        let dir = data_dir.join(AUDIO_CACHE_DIR);
        let recordings = load(&dir);
        Self { dir: Some(dir), recordings }
    }

    /// A cache that keeps nothing (an app that does not keep recordings).
    pub fn unavailable() -> Self {
        Self { dir: None, recordings: Vec::new() }
    }

    /// This cache can keep recordings.
    pub fn is_available(&self) -> bool {
        self.dir.is_some()
    }

    /// The recordings, newest first.
    pub fn recordings(&self) -> &[KeptRecording] {
        &self.recordings
    }

    /// One recording.
    pub fn get(&self, id: Uuid) -> Option<&KeptRecording> {
        self.recordings.iter().find(|r| r.id == id)
    }

    /// What the interface shows.
    pub fn view(&self) -> AudioCacheView {
        AudioCacheView { available: self.is_available(), recordings: self.recordings.clone(), bytes: self.bytes() }
    }

    /// The recordings' size in all.
    pub fn bytes(&self) -> u64 {
        self.recordings.iter().map(|r| r.bytes).sum()
    }

    /// Keep `new` as the newest recording ([`KeptStatus::Pending`]), then drop the oldest past
    /// `limits`. Its file is written before the index names it; when the index cannot be written
    /// the file goes again and nothing changes.
    /// A long take's parts directory is moved in (it is on the same file system: both are under
    /// the data directory); when keeping fails it is left where it was, for the caller to remove.
    pub fn keep(&mut self, new: NewRecording, limits: Limits) -> Result<(), CoreError> {
        let dir = self.dir.clone().ok_or_else(|| CoreError::Invalid(CACHE_UNAVAILABLE.to_owned()))?;
        std::fs::create_dir_all(&dir).map_err(io_error)?;
        let (format, bytes) = match &new.audio {
            KeptAudio::Wav(wav) => {
                write_private(&wav_path(&dir, new.id), wav).map_err(io_error)?;
                (KeptFormat::Wav, wav.len() as u64)
            }
            KeptAudio::Parts { dir: parts, bytes } => {
                std::fs::rename(parts, parts_path(&dir, new.id)).map_err(io_error)?;
                archive::sync_dir(Some(&dir));
                (KeptFormat::Opus, *bytes)
            }
        };
        let before = self.recordings.clone();
        self.recordings.retain(|r| r.id != new.id);
        let recording = KeptRecording { id: new.id, format, at_ms: new.at_ms, duration_ms: new.duration_ms, bytes, app: new.app, status: KeptStatus::Pending };
        self.recordings.insert(0, recording.clone());
        self.recordings.sort_by_key(|r| std::cmp::Reverse(r.at_ms));
        let evicted = self.evict(limits);
        if let Err(e) = self.save() {
            self.recordings = before;
            match &new.audio {
                KeptAudio::Wav(_) => remove_recording(&dir, &recording),
                // Back where it came from, so the caller's clean-up finds it.
                KeptAudio::Parts { dir: parts, .. } => {
                    let _ = std::fs::rename(parts_path(&dir, new.id), parts);
                }
            }
            return Err(e);
        }
        for gone in evicted {
            remove_recording(&dir, &gone);
        }
        Ok(())
    }

    /// Record what became of `id`'s take; `Ok(false)` when nothing changed (an unknown id, the
    /// same status). A failure, a cancel or an interruption never replaces
    /// [`KeptStatus::Recognized`]: the entry still holds the text recognised before.
    pub fn set_status(&mut self, id: Uuid, status: KeptStatus) -> Result<bool, CoreError> {
        let Some(at) = self.recordings.iter().position(|r| r.id == id) else { return Ok(false) };
        let current = &self.recordings[at].status;
        if *current == status || (matches!(current, KeptStatus::Recognized { .. }) && status.entry().is_none()) {
            return Ok(false);
        }
        let before = std::mem::replace(&mut self.recordings[at].status, status);
        if let Err(e) = self.save() {
            self.recordings[at].status = before;
            return Err(e);
        }
        Ok(true)
    }

    /// `id` and its audio; `Ok(None)` when it is not kept (or its file is gone, which forgets it).
    pub fn read(&mut self, id: Uuid) -> Result<Option<(KeptRecording, KeptRead)>, CoreError> {
        let (Some(dir), Some(recording)) = (&self.dir, self.get(id).cloned()) else { return Ok(None) };
        let read = match recording.format {
            KeptFormat::Wav => std::fs::read(wav_path(dir, id)).map(KeptRead::Wav),
            KeptFormat::Opus => KeptLong::open(&parts_path(dir, id)).map(KeptRead::Long),
        };
        match read {
            Ok(audio) => Ok(Some((recording, audio))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::warn!(%id, "a kept recording's file is gone; forgetting it");
                self.remove(id)?;
                Ok(None)
            }
            Err(e) => Err(io_error(e)),
        }
    }

    /// Delete one recording; `Ok(false)` when it was not kept.
    pub fn remove(&mut self, id: Uuid) -> Result<bool, CoreError> {
        self.remove_where(|r| r.id == id)
    }

    /// Delete the recordings the history entry `entry` was made from.
    pub fn remove_entry(&mut self, entry: Uuid) -> Result<bool, CoreError> {
        self.remove_where(|r| r.status.entry() == Some(entry))
    }

    /// Delete the recordings whose history entry is not among `existing` (the history dropped or
    /// deleted it); a recording no entry was made from stays.
    pub fn remove_unlinked(&mut self, existing: &HashSet<Uuid>) -> Result<bool, CoreError> {
        self.remove_where(|r| r.status.entry().is_some_and(|entry| !existing.contains(&entry)))
    }

    /// The history entries made from kept recordings.
    pub fn linked_entries(&self) -> Vec<Uuid> {
        self.recordings.iter().filter_map(|r| r.status.entry()).collect()
    }

    /// Delete every recording; the number deleted.
    pub fn clear(&mut self) -> Result<usize, CoreError> {
        let n = self.recordings.len();
        if n == 0 {
            return Ok(0);
        }
        self.remove_where(|_| true)?;
        Ok(n)
    }

    /// Drop the oldest recordings past `limits`; `Ok(true)` when any went.
    pub fn enforce(&mut self, limits: Limits) -> Result<bool, CoreError> {
        let before = self.recordings.clone();
        let evicted = self.evict(limits);
        if evicted.is_empty() {
            return Ok(false);
        }
        if let Err(e) = self.save() {
            self.recordings = before;
            return Err(e);
        }
        if let Some(dir) = &self.dir {
            for gone in evicted {
                remove_recording(dir, &gone);
            }
        }
        Ok(true)
    }

    /// Take the oldest recordings out of the list until it is within `limits`.
    fn evict(&mut self, limits: Limits) -> Vec<KeptRecording> {
        let mut total = self.bytes();
        let mut evicted = Vec::new();
        while self.recordings.len() > limits.keep || (total > limits.max_bytes && !self.recordings.is_empty()) {
            let Some(oldest) = self.recordings.pop() else { break };
            total -= oldest.bytes;
            evicted.push(oldest);
        }
        if !evicted.is_empty() {
            tracing::info!(evicted = evicted.len(), keep = limits.keep, max_bytes = limits.max_bytes, "the audio cache dropped its oldest recordings");
        }
        evicted
    }

    /// Delete the recordings `doomed` picks: the index first, then their files.
    fn remove_where(&mut self, doomed: impl Fn(&KeptRecording) -> bool) -> Result<bool, CoreError> {
        let (gone, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.recordings).into_iter().partition(|r| doomed(r));
        self.recordings = kept;
        if gone.is_empty() {
            return Ok(false);
        }
        if let Err(e) = self.save() {
            self.recordings.extend(gone);
            self.recordings.sort_by_key(|r| std::cmp::Reverse(r.at_ms));
            return Err(e);
        }
        if let Some(dir) = &self.dir {
            for recording in &gone {
                remove_recording(dir, recording);
            }
        }
        tracing::info!(removed = gone.len(), "kept recordings deleted");
        Ok(true)
    }

    /// Write the index (none at all once it is empty).
    fn save(&self) -> Result<(), CoreError> {
        let Some(dir) = &self.dir else { return Ok(()) };
        let path = dir.join(INDEX_FILE_NAME);
        if self.recordings.is_empty() {
            return match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(io_error(e)),
            };
        }
        std::fs::create_dir_all(dir).map_err(io_error)?;
        let bytes =
            serde_json::to_vec(&IndexFile { schema: INDEX_SCHEMA, recordings: self.recordings.clone() }).map_err(|e| io_error(std::io::Error::other(e)))?;
        write_private(&path, &bytes).map_err(io_error)
    }
}

/// `<dir>/<id>.wav`.
fn wav_path(dir: &Path, id: Uuid) -> PathBuf {
    dir.join(format!("{id}.wav"))
}

/// `<dir>/<id>/`: a long take's parts.
fn parts_path(dir: &Path, id: Uuid) -> PathBuf {
    dir.join(id.to_string())
}

/// Delete a recording's file or directory; one that is already gone is not an error.
fn remove_recording(dir: &Path, recording: &KeptRecording) {
    match recording.format {
        KeptFormat::Wav => remove_file(&wav_path(dir, recording.id)),
        KeptFormat::Opus => match std::fs::remove_dir_all(parts_path(dir, recording.id)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!(id = %recording.id, error = %e, "a kept recording could not be deleted"),
        },
    }
}

fn io_error(e: std::io::Error) -> CoreError {
    CoreError::History(format!("录音缓存：{e}"))
}

/// Write `bytes` to `path` whole: a temporary file beside it (this user's only on Unix), then a
/// rename over it.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options.open(&tmp).and_then(|mut file| file.write_all(bytes));
    if let Err(e) = written.and_then(|()| std::fs::rename(&tmp, path)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// Delete a recording's file; one that is already gone is not an error.
fn remove_file(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!(path = %path.display(), error = %e, "a kept recording could not be deleted"),
    }
}

/// The index of `dir`, reconciled with the files there (see the module's documentation).
fn load(dir: &Path) -> Vec<KeptRecording> {
    let index = dir.join(INDEX_FILE_NAME);
    let listed = match std::fs::read(&index) {
        Ok(bytes) => match serde_json::from_slice::<IndexFile>(&bytes) {
            Ok(file) if file.schema == INDEX_SCHEMA => Some(file.recordings),
            Ok(file) => {
                tracing::warn!(schema = file.schema, "the audio cache's index has another schema; starting empty");
                None
            }
            Err(e) => {
                tracing::warn!(error = %e, "the audio cache's index is unusable; starting empty");
                None
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(Vec::new()),
        Err(e) => {
            tracing::warn!(error = %e, "the audio cache's index is unreadable; starting empty");
            None
        }
    };
    let changed = listed.is_none();
    let listed = listed.unwrap_or_default();
    let mut seen = HashSet::new();
    let mut recordings = Vec::with_capacity(listed.len());
    let mut fixed = changed;
    for mut recording in listed {
        if !seen.insert(recording.id) {
            fixed = true;
            continue;
        }
        let size = match recording.format {
            KeptFormat::Wav => std::fs::metadata(wav_path(dir, recording.id)).ok().filter(std::fs::Metadata::is_file).map(|m| m.len()),
            KeptFormat::Opus => KeptLong::open(&parts_path(dir, recording.id)).ok().map(|kept| archive::parts_bytes(&kept.dir, kept.manifest.parts)),
        };
        match size {
            Some(bytes) => {
                if recording.bytes != bytes {
                    recording.bytes = bytes;
                    fixed = true;
                }
                if recording.status == KeptStatus::Pending {
                    recording.status = KeptStatus::Interrupted;
                    fixed = true;
                }
                recordings.push(recording);
            }
            _ => fixed = true,
        }
    }
    recordings.sort_by_key(|r| std::cmp::Reverse(r.at_ms));
    let named: HashSet<String> = recordings
        .iter()
        .map(|r| match r.format {
            KeptFormat::Wav => format!("{}.wav", r.id),
            KeptFormat::Opus => r.id.to_string(),
        })
        .collect();
    let mut strays = 0;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if named.contains(&name) {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                // A long take's directory no recording names (one being moved in when the app stopped).
                if std::fs::remove_dir_all(&path).is_ok() {
                    strays += 1;
                }
            } else if name.ends_with(".wav") || name.ends_with(".tmp") {
                remove_file(&path);
                strays += 1;
            }
        }
    }
    if strays > 0 {
        tracing::info!(strays, "removed the audio cache's files no recording names");
    }
    if fixed {
        let cache = AudioCache { dir: Some(dir.to_path_buf()), recordings };
        if let Err(e) = cache.save() {
            tracing::warn!(error = %e, "the audio cache's index could not be rewritten");
        }
        return cache.recordings;
    }
    recordings
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: Limits = Limits { keep: 20, max_bytes: 200 * MB };

    fn new(at_ms: u64, bytes: usize) -> NewRecording {
        NewRecording {
            id: Uuid::new_v4(),
            at_ms,
            duration_ms: 1_000,
            audio: KeptAudio::Wav(vec![7u8; bytes]),
            app: Some(AppRef { id: "code".into(), name: "Code".into() }),
        }
    }

    fn files(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir.join(AUDIO_CACHE_DIR))
            .map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        names.sort();
        names
    }

    #[test]
    fn a_kept_recording_is_written_listed_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AudioCache::open(dir.path());
        assert!(cache.is_available());
        assert_eq!(cache.view(), AudioCacheView { available: true, recordings: Vec::new(), bytes: 0 });
        let first = new(1_000, 10);
        cache.keep(first.clone(), LIMITS).unwrap();
        let view = cache.view();
        assert_eq!(view.bytes, 10);
        assert_eq!(view.recordings.len(), 1);
        assert_eq!(view.recordings[0].status, KeptStatus::Pending);
        assert_eq!(view.recordings[0].app.as_ref().map(|a| a.id.as_str()), Some("code"));
        let Some((recording, KeptRead::Wav(wav))) = cache.read(first.id).unwrap() else { panic!("a WAV") };
        assert_eq!((recording.id, recording.format, wav), (first.id, KeptFormat::Wav, vec![7u8; 10]));
        assert_eq!(files(dir.path()), vec![format!("{}.wav", first.id), INDEX_FILE_NAME.to_owned()]);
        // The newest first, and the index survives a restart (a pending take reads interrupted).
        let second = new(2_000, 20);
        cache.keep(second.clone(), LIMITS).unwrap();
        assert_eq!(cache.recordings().iter().map(|r| r.id).collect::<Vec<_>>(), vec![second.id, first.id]);
        let reopened = AudioCache::open(dir.path());
        assert_eq!(reopened.recordings().iter().map(|r| r.id).collect::<Vec<_>>(), vec![second.id, first.id]);
        assert!(reopened.recordings().iter().all(|r| r.status == KeptStatus::Interrupted));
    }

    /// A long take's parts (docs/dictation.md §26.2) are moved in whole, read back as parts, follow
    /// the same limits and statuses, and go as a directory.
    #[test]
    fn a_long_takes_parts_are_moved_in_read_back_and_deleted_whole() {
        use crate::dictation::archive::{ArchiveWriter, MANIFEST, PART_SAMPLES};
        let dir = tempfile::tempdir().unwrap();
        let parts = dir.path().join("recordings").join("take-1-1.parts");
        let mut writer = ArchiveWriter::create(&parts).unwrap();
        writer.push(&vec![0i16; usize::try_from(PART_SAMPLES).unwrap() + 16_000]);
        let report = writer.finish(vec![PART_SAMPLES], Vec::new());
        let mut cache = AudioCache::open(dir.path());
        let long = NewRecording {
            id: Uuid::new_v4(),
            at_ms: 5_000,
            duration_ms: 31_000,
            audio: KeptAudio::Parts { dir: parts.clone(), bytes: report.bytes },
            app: None,
        };
        cache.keep(long.clone(), LIMITS).unwrap();
        assert!(!parts.exists(), "moved, not copied");
        let mut expected = vec![INDEX_FILE_NAME.to_owned(), long.id.to_string()];
        expected.sort();
        assert_eq!(files(dir.path()), expected);
        let recording = cache.get(long.id).unwrap().clone();
        assert_eq!((recording.format, recording.bytes), (KeptFormat::Opus, report.bytes));
        let wire = serde_json::to_value(&recording).unwrap();
        assert_eq!(wire["format"], "opus", "a long take says so on the wire; a WAV does not");
        let Some((_, KeptRead::Long(kept))) = cache.read(long.id).unwrap() else { panic!("parts") };
        assert_eq!((kept.manifest.total, kept.manifest.parts, kept.manifest.cuts.clone()), (PART_SAMPLES + 16_000, 2, vec![PART_SAMPLES]));
        // A WAV beside it; a restart keeps both and their sizes.
        cache.keep(new(6_000, 10), LIMITS).unwrap();
        let reopened = AudioCache::open(dir.path());
        assert_eq!(reopened.recordings().len(), 2);
        assert_eq!(reopened.get(long.id).unwrap().bytes, report.bytes);
        // Past the count, the oldest — the long one — goes as a directory.
        let mut cache = reopened;
        cache.enforce(Limits { keep: 1, max_bytes: 200 * MB }).unwrap();
        assert!(cache.get(long.id).is_none());
        assert!(!dir.path().join(AUDIO_CACHE_DIR).join(long.id.to_string()).exists());
        // A directory without its manifest is forgotten; one the index does not name is deleted.
        let mut writer = ArchiveWriter::create(&parts).unwrap();
        writer.push(&[0i16; 160]);
        let report = writer.finish(Vec::new(), Vec::new());
        let lost = NewRecording { id: Uuid::new_v4(), at_ms: 7_000, duration_ms: 10, audio: KeptAudio::Parts { dir: parts, bytes: report.bytes }, app: None };
        cache.keep(lost.clone(), LIMITS).unwrap();
        let cache_dir = dir.path().join(AUDIO_CACHE_DIR);
        std::fs::remove_file(cache_dir.join(lost.id.to_string()).join(MANIFEST)).unwrap();
        std::fs::create_dir_all(cache_dir.join("stray-dir")).unwrap();
        let reopened = AudioCache::open(dir.path());
        assert!(reopened.get(lost.id).is_none());
        assert!(!cache_dir.join("stray-dir").exists() && !cache_dir.join(lost.id.to_string()).exists());
    }

    /// A long take that fails to be moved in stays where it was, for the caller to remove.
    #[test]
    fn a_long_take_that_cannot_be_moved_in_is_left_where_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AudioCache::open(dir.path());
        let missing = dir.path().join("recordings").join("take-2-2.parts");
        let new = NewRecording { id: Uuid::new_v4(), at_ms: 1, duration_ms: 1, audio: KeptAudio::Parts { dir: missing, bytes: 1 }, app: None };
        assert!(cache.keep(new, LIMITS).is_err());
        assert!(cache.recordings().is_empty());
        assert!(
            format!("{:?}", NewRecording { id: Uuid::nil(), at_ms: 1, duration_ms: 1, audio: KeptAudio::Wav(vec![1, 2, 3]), app: None })
                .contains("wav: 3 bytes")
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_files_are_this_users_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AudioCache::open(dir.path());
        let one = new(1, 4);
        cache.keep(one.clone(), LIMITS).unwrap();
        let cache_dir = dir.path().join(AUDIO_CACHE_DIR);
        for name in [format!("{}.wav", one.id), INDEX_FILE_NAME.to_owned()] {
            let mode = std::fs::metadata(cache_dir.join(&name)).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{name}");
        }
    }

    #[test]
    fn the_oldest_go_past_the_count_and_the_size() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AudioCache::open(dir.path());
        let limits = Limits { keep: 3, max_bytes: 1_000 };
        let made: Vec<NewRecording> = (0..5).map(|i| new(i, 100)).collect();
        for n in &made {
            cache.keep(n.clone(), limits).unwrap();
        }
        assert_eq!(cache.recordings().iter().map(|r| r.id).collect::<Vec<_>>(), vec![made[4].id, made[3].id, made[2].id]);
        assert_eq!(files(dir.path()).len(), 4, "three recordings and the index");
        // A big one pushes the older ones out by size: 950 + 100 > 1 000.
        let big = new(10, 950);
        cache.keep(big.clone(), limits).unwrap();
        assert_eq!(cache.recordings().iter().map(|r| r.id).collect::<Vec<_>>(), vec![big.id]);
        assert_eq!(cache.bytes(), 950);
        // Tighter settings apply at once.
        let roomy = Limits { keep: 3, max_bytes: 10_000 };
        cache.keep(new(11, 100), roomy).unwrap();
        cache.keep(new(12, 100), roomy).unwrap();
        assert_eq!(cache.recordings().len(), 3);
        let tight = Limits { keep: 1, max_bytes: 10_000 };
        assert!(cache.enforce(tight).unwrap());
        assert_eq!(cache.recordings().len(), 1);
        assert!(!cache.enforce(tight).unwrap(), "nothing more to drop");
        assert_eq!(files(dir.path()).len(), 2);
    }

    #[test]
    fn a_recognised_recording_keeps_its_entry_when_a_later_attempt_fails() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AudioCache::open(dir.path());
        let one = new(1, 4);
        cache.keep(one.clone(), LIMITS).unwrap();
        let failed = KeptStatus::Failed { code: FailureCode::Asr, message: "网络超时".into() };
        assert!(cache.set_status(one.id, failed.clone()).unwrap());
        assert!(!cache.set_status(one.id, failed).unwrap(), "the same status changes nothing");
        let entry = Uuid::new_v4();
        assert!(cache.set_status(one.id, KeptStatus::Recognized { entry }).unwrap());
        for later in [KeptStatus::Cancelled, KeptStatus::Interrupted, KeptStatus::Failed { code: FailureCode::NoSpeech, message: String::new() }] {
            assert!(!cache.set_status(one.id, later).unwrap());
        }
        assert_eq!(cache.get(one.id).unwrap().status, KeptStatus::Recognized { entry });
        // A newer entry made from it takes its place.
        let newer = Uuid::new_v4();
        assert!(cache.set_status(one.id, KeptStatus::Recognized { entry: newer }).unwrap());
        assert_eq!(cache.linked_entries(), vec![newer]);
        assert!(!cache.set_status(Uuid::new_v4(), KeptStatus::Cancelled).unwrap(), "an unknown id");
    }

    #[test]
    fn recordings_go_with_their_entries_and_on_clear() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AudioCache::open(dir.path());
        let (a, b, c) = (new(1, 4), new(2, 4), new(3, 4));
        for n in [&a, &b, &c] {
            cache.keep(n.clone(), LIMITS).unwrap();
        }
        let (entry_a, entry_b) = (Uuid::new_v4(), Uuid::new_v4());
        cache.set_status(a.id, KeptStatus::Recognized { entry: entry_a }).unwrap();
        cache.set_status(b.id, KeptStatus::Recognized { entry: entry_b }).unwrap();
        cache.set_status(c.id, KeptStatus::Cancelled).unwrap();
        assert!(cache.remove_entry(entry_a).unwrap());
        assert!(!cache.remove_entry(entry_a).unwrap());
        assert!(cache.get(a.id).is_none());
        // The history no longer has entry_b: its recording goes; the unrecognised one stays.
        assert!(cache.remove_unlinked(&HashSet::new()).unwrap());
        assert_eq!(cache.recordings().iter().map(|r| r.id).collect::<Vec<_>>(), vec![c.id]);
        assert!(cache.remove(c.id).unwrap());
        assert!(!cache.remove(c.id).unwrap());
        assert_eq!(files(dir.path()), Vec::<String>::new(), "no files and no index once it is empty");
        cache.keep(new(4, 4), LIMITS).unwrap();
        cache.keep(new(5, 4), LIMITS).unwrap();
        assert_eq!(cache.clear().unwrap(), 2);
        assert_eq!(cache.clear().unwrap(), 0);
        assert!(files(dir.path()).is_empty());
    }

    #[test]
    fn a_missing_file_is_forgotten_and_a_stray_one_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AudioCache::open(dir.path());
        let (kept, lost) = (new(1, 4), new(2, 4));
        cache.keep(kept.clone(), LIMITS).unwrap();
        cache.keep(lost.clone(), LIMITS).unwrap();
        let cache_dir = dir.path().join(AUDIO_CACHE_DIR);
        std::fs::remove_file(cache_dir.join(format!("{}.wav", lost.id))).unwrap();
        std::fs::write(cache_dir.join("stray.wav"), b"x").unwrap();
        std::fs::write(cache_dir.join("half.tmp"), b"x").unwrap();
        std::fs::write(cache_dir.join("notes.txt"), b"not ours").unwrap();
        let mut reopened = AudioCache::open(dir.path());
        assert_eq!(reopened.recordings().iter().map(|r| r.id).collect::<Vec<_>>(), vec![kept.id]);
        assert_eq!(files(dir.path()), vec![format!("{}.wav", kept.id), INDEX_FILE_NAME.to_owned(), "notes.txt".to_owned()]);
        // A file that goes while the app runs is forgotten when it is read.
        std::fs::remove_file(cache_dir.join(format!("{}.wav", kept.id))).unwrap();
        assert!(reopened.read(kept.id).unwrap().is_none());
        assert!(reopened.recordings().is_empty());
        assert!(reopened.read(Uuid::new_v4()).unwrap().is_none());
    }

    #[test]
    fn an_unusable_index_starts_the_cache_empty() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AudioCache::open(dir.path());
        let one = new(1, 4);
        cache.keep(one, LIMITS).unwrap();
        let cache_dir = dir.path().join(AUDIO_CACHE_DIR);
        std::fs::write(cache_dir.join(INDEX_FILE_NAME), b"{ not json").unwrap();
        let reopened = AudioCache::open(dir.path());
        assert!(reopened.recordings().is_empty());
        assert!(files(dir.path()).is_empty(), "the recordings it named cannot be told apart: they go");
        std::fs::create_dir_all(&cache_dir).unwrap();
        std::fs::write(cache_dir.join(INDEX_FILE_NAME), br#"{"schema":99,"recordings":[]}"#).unwrap();
        assert!(AudioCache::open(dir.path()).recordings().is_empty());
    }

    #[test]
    fn a_cache_that_keeps_nothing_refuses_to_keep() {
        let mut cache = AudioCache::unavailable();
        assert!(!cache.is_available());
        assert!(matches!(cache.keep(new(1, 4), LIMITS), Err(CoreError::Invalid(_))));
        assert_eq!(cache.view(), AudioCacheView::default());
        assert!(cache.read(Uuid::new_v4()).unwrap().is_none());
        assert!(!cache.remove(Uuid::new_v4()).unwrap());
    }

    #[test]
    fn the_settings_bounds() {
        assert_eq!(validate(&AudioCacheSettings::default()), Ok(()));
        let base = AudioCacheSettings::default();
        assert!(validate(&AudioCacheSettings { keep: 0, ..base }).unwrap_err().contains("keep"));
        assert!(validate(&AudioCacheSettings { keep: MAX_KEEP + 1, ..base }).unwrap_err().contains("keep"));
        assert!(validate(&AudioCacheSettings { max_mb: MIN_MB - 1, ..base }).unwrap_err().contains("max_mb"));
        assert!(validate(&AudioCacheSettings { max_mb: MAX_MB + 1, ..base }).unwrap_err().contains("max_mb"));
        // A hand-edited file cannot empty the cache or let it grow past the bounds.
        let limits = Limits::from(&AudioCacheSettings { enabled: true, keep: 0, max_mb: u32::MAX });
        assert_eq!((limits.keep, limits.max_bytes), (MIN_KEEP as usize, u64::from(MAX_MB) * MB));
        let limits = Limits::from(&AudioCacheSettings { enabled: true, keep: u32::MAX, max_mb: 0 });
        assert_eq!((limits.keep, limits.max_bytes), (MAX_KEEP as usize, u64::from(MIN_MB) * MB));
        assert_eq!(Limits::from(&base), Limits { keep: 20, max_bytes: 200 * MB });
    }

    /// A 重新识别's choices (docs/dictation.md §26.4) are the settings with those models picked,
    /// the endpoints kept; what is not chosen stays the settings'.
    #[test]
    fn retake_models_pick_their_models_in_a_copy_of_the_settings() {
        use crate::engines::ProviderSettings;
        let mut engines = EngineSettings { refine_enabled: false, ..EngineSettings::default() };
        engines.providers.insert(ProviderId::Custom, ProviderSettings { asr_url: Some("https://asr.example.test/v1".into()), ..ProviderSettings::default() });
        assert!(RetakeModels::default().is_empty());
        assert_eq!(RetakeModels::default().apply(&engines), engines);
        assert_eq!(RetakeModels::default().refine(), None);
        let models = RetakeModels {
            asr: Some(RecognitionChoice::Remote { provider: ProviderId::Custom, model: "whisper-2".into() }),
            refine: Some(PolishChoice::Remote { provider: ProviderId::Groq, model: "openai/gpt-oss-120b".into() }),
        };
        let next = models.apply(&engines);
        assert_eq!(next.asr_provider, ProviderId::Custom);
        let custom = &next.providers[&ProviderId::Custom];
        assert_eq!((custom.asr_model.as_deref(), custom.asr_url.as_deref()), (Some("whisper-2"), Some("https://asr.example.test/v1")));
        assert!(next.refine_enabled && next.llm_provider == ProviderId::Groq);
        assert_eq!(next.providers[&ProviderId::Groq].llm_model.as_deref(), Some("openai/gpt-oss-120b"));
        assert_eq!(models.refine(), Some(true));
        assert!(!engines.providers.contains_key(&ProviderId::Groq), "the settings themselves are not touched");
        let local = RetakeModels { asr: Some(RecognitionChoice::Local { id: "sense-voice-small".into() }), refine: Some(PolishChoice::Off) };
        let next = local.apply(&EngineSettings::default());
        assert_eq!((next.asr_provider, next.local_model.as_deref(), next.refine_enabled), (ProviderId::Local, Some("sense-voice-small"), false));
        assert_eq!(local.refine(), Some(false));
        let wire = serde_json::to_value(&models).unwrap();
        assert_eq!(wire["asr"], serde_json::json!({ "kind": "remote", "provider": "custom", "model": "whisper-2" }));
        assert_eq!(serde_json::to_value(&local).unwrap()["refine"], serde_json::json!({ "kind": "off" }));
        assert_eq!(serde_json::to_value(RetakeModels::default()).unwrap(), serde_json::json!({}));
    }

    #[test]
    fn the_wire_names() {
        let recording = KeptRecording {
            id: Uuid::nil(),
            at_ms: 1,
            duration_ms: 2,
            bytes: 3,
            app: None,
            status: KeptStatus::Failed { code: FailureCode::Asr, message: "网络超时".into() },
            format: KeptFormat::Wav,
        };
        let json = serde_json::to_value(&recording).unwrap();
        assert_eq!(json["status"], serde_json::json!({ "kind": "failed", "code": "asr", "message": "网络超时" }));
        assert!(json.get("app").is_none());
        assert!(json.get("format").is_none(), "a WAV recording is written as before");
        let long = KeptRecording { format: KeptFormat::Opus, ..recording };
        assert_eq!(serde_json::to_value(&long).unwrap()["format"], "opus");
        assert_eq!(serde_json::to_value(KeptStatus::Recognized { entry: Uuid::nil() }).unwrap()["kind"], "recognized");
        assert_eq!(serde_json::to_value(KeptStatus::Interrupted).unwrap(), serde_json::json!({ "kind": "interrupted" }));
        let debug = format!("{:?}", new(1, 4));
        assert!(debug.contains("wav: 4 bytes") && !debug.contains("[7"), "{debug}");
    }
}
