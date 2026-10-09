//! The persistence the user-maintained lists share — `dictionary.json` / `rules.json`
//! (docs/dictation.md §16.1) and `scenes.json` (§18.1): load with validation, atomic save, and the
//! quarantine of a file that cannot be used.
//!
//! Every mutation builds the new list, validates it as a whole, writes it atomically (temporary
//! file + rename) and only then replaces the list in memory, so a refused or failed change leaves
//! both the file and the list as they were. A file that cannot be used does not stop the app: it is
//! renamed to `<file>.corrupt-<unix seconds>` (never deleted) and the store starts empty; if even
//! the rename fails the store refuses to write, so the unreadable file is never overwritten.
//!
//! [`ListFile::read`] is the same reading and the same checks without any of that: a second process
//! sharing the data directory (the local speech service, docs/dictation.md §23) reads the lists the
//! app maintains and never renames, moves or writes them.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde::de::DeserializeOwned;
use uuid::Uuid;

/// A JSON list file: where it is and whether it may be written.
#[derive(Debug)]
pub(crate) struct ListFile {
    path: PathBuf,
    /// `false` after a file that could not be read or moved aside: writing would overwrite it.
    writable: bool,
}

impl ListFile {
    /// Load `dir/name` as `T`, validated by `check` (its error text lands in the notice). Missing →
    /// empty. Unusable → moved aside (or, when that fails, left alone and the store made
    /// read-only), empty, plus a notice for the UI. `schema` is the one schema number `into_items`
    /// accepts (only used in the notice text).
    pub(crate) fn load<T, F: DeserializeOwned>(
        dir: &Path,
        name: &str,
        schema: u16,
        into_items: impl FnOnce(F) -> Option<Vec<T>>,
        check: impl FnOnce(&[T]) -> Result<(), String>,
    ) -> (Self, Vec<T>, Option<String>) {
        let path = dir.join(name);
        match read_items(&path, schema, into_items, check) {
            Contents::Missing => (Self { path, writable: true }, Vec::new(), None),
            Contents::Items(items) => (Self { path, writable: true }, items, None),
            Contents::Unreadable(e) => {
                tracing::warn!(error = %e, path = %path.display(), "list file unreadable; starting empty and not writing it");
                let notice = format!("{name} 无法读取（{e}），本次从空列表开始，且不会覆盖该文件");
                (Self { path, writable: false }, Vec::new(), Some(notice))
            }
            Contents::Unusable(why) => {
                let (writable, notice) = quarantine(&path, name, &why);
                (Self { path, writable }, Vec::new(), Some(notice))
            }
        }
    }

    /// Read `dir/name` as [`Self::load`] does, with no side effect at all: missing → empty;
    /// unreadable or unusable → the reason, the file left exactly as it is.
    pub(crate) fn read<T, F: DeserializeOwned>(
        dir: &Path,
        name: &str,
        schema: u16,
        into_items: impl FnOnce(F) -> Option<Vec<T>>,
        check: impl FnOnce(&[T]) -> Result<(), String>,
    ) -> Result<Vec<T>, String> {
        match read_items(&dir.join(name), schema, into_items, check) {
            Contents::Missing => Ok(Vec::new()),
            Contents::Items(items) => Ok(items),
            Contents::Unreadable(e) => Err(format!("{name} 无法读取（{e}）")),
            Contents::Unusable(why) => Err(format!("{name} 无法使用（{why}）")),
        }
    }

    /// Write `value` atomically. The error text is `<path>：<reason>`; the caller adds its prefix.
    pub(crate) fn save<V: Serialize>(&self, value: &V) -> Result<(), String> {
        let err = |e: &dyn std::fmt::Display| format!("{}：{e}", self.path.display());
        if !self.writable {
            return Err(format!("{} 启动时无法读取或移开，为避免覆盖不写入；请检查该文件后重启", self.path.display()));
        }
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| err(&e))?;
        }
        let bytes = serde_json::to_vec_pretty(value).map_err(|e| err(&e))?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, bytes).map_err(|e| err(&e))?;
        std::fs::rename(&tmp, &self.path).map_err(|e| err(&e))
    }
}

/// What a list file holds.
enum Contents<T> {
    /// No file.
    Missing,
    /// The validated list.
    Items(Vec<T>),
    /// The file is there but could not be read.
    Unreadable(std::io::Error),
    /// The file was read but is not a list `check` accepts (or not the schema).
    Unusable(String),
}

fn read_items<T, F: DeserializeOwned>(
    path: &Path,
    schema: u16,
    into_items: impl FnOnce(F) -> Option<Vec<T>>,
    check: impl FnOnce(&[T]) -> Result<(), String>,
) -> Contents<T> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Contents::Missing,
        Err(e) => return Contents::Unreadable(e),
    };
    match serde_json::from_slice::<F>(&bytes) {
        Ok(file) => match into_items(file) {
            Some(items) => match check(&items) {
                Ok(()) => Contents::Items(items),
                Err(e) => Contents::Unusable(e),
            },
            None => Contents::Unusable(format!("schema 不是 {schema}")),
        },
        Err(e) => Contents::Unusable(e.to_string()),
    }
}

/// Move an unusable file to `<file>.corrupt-<unix seconds>` (a `-N` suffix when that exists).
/// Returns whether the store may write the original path again, and the notice for the UI.
fn quarantine(path: &Path, name: &str, why: &str) -> (bool, String) {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let base = format!("{name}.corrupt-{secs}");
    let mut aside = path.with_file_name(&base);
    let mut n = 1;
    while aside.exists() {
        aside = path.with_file_name(format!("{base}-{n}"));
        n += 1;
    }
    match std::fs::rename(path, &aside) {
        Ok(()) => {
            tracing::warn!(%why, moved_to = %aside.display(), "list file is unusable; moved aside, starting empty");
            (true, format!("{name} 无法使用（{why}），已移到 {}，从空列表开始", aside.display()))
        }
        Err(e) => {
            tracing::warn!(%why, error = %e, "list file is unusable and could not be moved aside; starting empty, not writing it");
            (false, format!("{name} 无法使用（{why}），也无法移开（{e}）；本次从空列表开始且不会覆盖该文件"))
        }
    }
}

/// `items` in the order of `ids` when `ids` is a permutation of their ids.
pub(crate) fn permute<T: Clone>(items: &[T], ids: &[Uuid], id_of: impl Fn(&T) -> Uuid) -> Option<Vec<T>> {
    if ids.len() != items.len() {
        return None;
    }
    let mut out = Vec::with_capacity(items.len());
    for (i, id) in ids.iter().enumerate() {
        if ids[..i].contains(id) {
            return None;
        }
        out.push(items.iter().find(|item| id_of(item) == *id)?.clone());
    }
    Some(out)
}
