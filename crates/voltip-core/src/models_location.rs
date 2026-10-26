//! Where the local models live (docs/dictation.md §10.9, issue #192: 「模型下载，不能设置保存路径？
//! C 盘满了」): `<data_dir>/models` unless the person chose another folder. The choice belongs to
//! this computer, not to the settings a phone mirrors or a backup carries: it sits in
//! `<data_dir>/models-location.json`, which [`crate::CoreConfig::new`] reads before anything opens
//! the library, so the app, its command line and the speech service find the models in one place.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The file under the data directory.
pub const LOCATION_FILE: &str = "models-location.json";
/// Bumped when the file's meaning changes; another schema reads as the default.
const SCHEMA: u32 = 1;

/// What the file says.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelsLocation {
    /// The chosen folder (absolute); `None` is the default, `<data_dir>/models`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<PathBuf>,
    /// Model folders a move could not delete from where they were (a file still in use, Windows
    /// keeps a loaded model open): deleted at the next start, before anything loads.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub leftovers: Vec<PathBuf>,
}

/// `UiState.models_location` (docs/dictation.md §10.9): where the library is, a move in progress,
/// and how the last one ended.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelsLocationView {
    /// This app has a model library (the desktop; not the phone).
    pub available: bool,
    /// The folder the library is in.
    pub dir: String,
    /// `dir` is a folder the person chose, not the default.
    pub custom: bool,
    /// The default folder, `<data_dir>/models`.
    pub default_dir: String,
    /// A move in progress.
    #[serde(default)]
    pub moving: Option<ModelsMove>,
    /// How the last move ended, until the next one starts.
    #[serde(default)]
    pub last: Option<ModelsMoveOutcome>,
}

/// A move in progress.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelsMove {
    /// Where the library goes.
    pub to: String,
    /// Bytes copied so far.
    pub moved_bytes: u64,
    /// Bytes the move copies in all; 0 while it only renames (one disk) or has not counted yet.
    pub total_bytes: u64,
}

/// How a move ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelsMoveOutcome {
    /// The library is in `dir` now. `deferred` old model folders could not be deleted yet (a file
    /// in use): the next start deletes them.
    Moved {
        /// The new folder.
        dir: String,
        /// Old folders deleted at the next start.
        deferred: u32,
    },
    /// Cancelled before the switch: nothing changed.
    Cancelled,
    /// Failed before the switch: nothing changed.
    Failed {
        /// Why, in words.
        message: String,
    },
}

#[derive(Serialize, Deserialize)]
struct LocationFile {
    schema: u32,
    #[serde(flatten)]
    location: ModelsLocation,
}

/// The default folder: `models` under the data directory.
pub fn default_root(data_dir: &Path) -> PathBuf {
    data_dir.join(crate::runtime::MODELS_DIR_NAME)
}

/// The file's contents. No file, an unreadable one, another schema or a relative folder all read as
/// the default: the library is then where it always was, and a model found missing can be
/// downloaded again.
pub fn read(data_dir: &Path) -> ModelsLocation {
    let path = data_dir.join(LOCATION_FILE);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return ModelsLocation::default(),
        Err(e) => {
            tracing::warn!(error = %e, "the models' location could not be read; using the default");
            return ModelsLocation::default();
        }
    };
    match serde_json::from_slice::<LocationFile>(&bytes) {
        Ok(file) if file.schema == SCHEMA => {
            let mut location = file.location;
            if location.dir.as_ref().is_some_and(|d| !d.is_absolute()) {
                tracing::warn!("the models' location is not an absolute path; using the default");
                location.dir = None;
            }
            location
        }
        Ok(file) => {
            tracing::warn!(schema = file.schema, "the models' location has an unknown schema; using the default");
            ModelsLocation::default()
        }
        Err(e) => {
            tracing::warn!(error = %e, "the models' location is not valid; using the default");
            ModelsLocation::default()
        }
    }
}

/// Where the library is: the chosen folder, else the default.
pub fn root(data_dir: &Path) -> PathBuf {
    read(data_dir).dir.unwrap_or_else(|| default_root(data_dir))
}

/// Write `location`, through a temporary file renamed into place; the default with nothing left
/// to delete removes the file.
pub fn write(data_dir: &Path, location: &ModelsLocation) -> std::io::Result<()> {
    let path = data_dir.join(LOCATION_FILE);
    if location == &ModelsLocation::default() {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    std::fs::create_dir_all(data_dir)?;
    let bytes = serde_json::to_vec_pretty(&LocationFile { schema: SCHEMA, location: location.clone() }).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    let written = std::fs::File::create(&tmp).and_then(|mut file| {
        file.write_all(&bytes)?;
        file.sync_all()
    });
    if let Err(e) = written.and_then(|()| std::fs::rename(&tmp, &path)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// Delete the folders an earlier move left behind; the ones that still cannot go stay listed for
/// the next start. Only folders the file names are touched.
pub fn remove_leftovers(data_dir: &Path) {
    let mut location = read(data_dir);
    if location.leftovers.is_empty() {
        return;
    }
    let before = location.leftovers.len();
    location.leftovers.retain(|dir| match std::fs::remove_dir_all(dir) {
        Ok(()) => {
            tracing::info!(dir = %dir.display(), "deleted the models an earlier move left behind");
            false
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => {
            tracing::warn!(dir = %dir.display(), error = %e, "the models an earlier move left behind could not be deleted yet");
            true
        }
    });
    if location.leftovers.len() != before
        && let Err(e) = write(data_dir, &location)
    {
        tracing::warn!(error = %e, "the models' location could not be updated");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_models_under_the_data_directory_and_writes_no_file() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(root(dir.path()), dir.path().join("models"));
        assert_eq!(read(dir.path()), ModelsLocation::default());
        write(dir.path(), &ModelsLocation::default()).unwrap();
        assert!(!dir.path().join(LOCATION_FILE).exists());
    }

    #[test]
    fn a_chosen_folder_is_read_back_and_going_back_to_the_default_removes_the_file() {
        let data = tempfile::tempdir().unwrap();
        let chosen = tempfile::tempdir().unwrap();
        let location = ModelsLocation { dir: Some(chosen.path().to_path_buf()), leftovers: Vec::new() };
        write(data.path(), &location).unwrap();
        assert_eq!(read(data.path()), location);
        assert_eq!(root(data.path()), chosen.path());
        let text = std::fs::read_to_string(data.path().join(LOCATION_FILE)).unwrap();
        assert!(text.contains("\"schema\": 1"), "{text}");
        assert!(!data.path().join("models-location.json.tmp").exists());
        write(data.path(), &ModelsLocation::default()).unwrap();
        assert!(!data.path().join(LOCATION_FILE).exists());
        assert_eq!(root(data.path()), data.path().join("models"));
    }

    #[test]
    fn a_file_that_cannot_be_trusted_reads_as_the_default() {
        let data = tempfile::tempdir().unwrap();
        for text in ["not json", r#"{"schema":2,"dir":"/elsewhere"}"#, r#"{"schema":1,"dir":"relative/models"}"#, r#"{"dir":"/elsewhere"}"#] {
            std::fs::write(data.path().join(LOCATION_FILE), text).unwrap();
            assert_eq!(root(data.path()), data.path().join("models"), "{text}");
        }
    }

    #[test]
    fn leftovers_are_deleted_at_the_next_start_and_forgotten() {
        let data = tempfile::tempdir().unwrap();
        let old = tempfile::tempdir().unwrap();
        let left = old.path().join("qwen3-asr-0.6b");
        std::fs::create_dir_all(&left).unwrap();
        std::fs::write(left.join("model.gguf"), b"x").unwrap();
        let gone = old.path().join("never-there");
        let chosen = tempfile::tempdir().unwrap();
        write(data.path(), &ModelsLocation { dir: Some(chosen.path().to_path_buf()), leftovers: vec![left.clone(), gone] }).unwrap();
        remove_leftovers(data.path());
        assert!(!left.exists());
        assert_eq!(read(data.path()), ModelsLocation { dir: Some(chosen.path().to_path_buf()), leftovers: Vec::new() });
        // Back at the default with nothing left over: no file at all.
        write(data.path(), &ModelsLocation { dir: None, leftovers: vec![data.path().join("x")] }).unwrap();
        remove_leftovers(data.path());
        assert!(!data.path().join(LOCATION_FILE).exists());
    }
}
