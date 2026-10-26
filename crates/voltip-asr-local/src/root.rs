//! The model library's folder, shared (docs/dictation.md §10.9): the store, the recognisers, the
//! streaming preview and the VAD each hold a clone, so moving the library to another folder
//! switches all of them at once, without a restart. Each reads it when it builds a path.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::RwLock;

/// Where the library is now.
#[derive(Clone)]
pub struct ModelsRoot(Arc<RwLock<PathBuf>>);

impl std::fmt::Debug for ModelsRoot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ModelsRoot").field(&*self.0.read()).finish()
    }
}

impl ModelsRoot {
    /// A root at `path`, shared with its clones only.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self(Arc::new(RwLock::new(path.into())))
    }

    /// The folder now.
    pub fn get(&self) -> PathBuf {
        self.0.read().clone()
    }

    /// `<root>/<id>`: one model's folder.
    pub fn join(&self, id: &str) -> PathBuf {
        self.0.read().join(id)
    }

    /// Point every clone at `path` (the end of a move).
    pub fn set(&self, path: impl Into<PathBuf>) {
        *self.0.write() = path.into();
    }
}

impl From<PathBuf> for ModelsRoot {
    fn from(path: PathBuf) -> Self {
        Self::new(path)
    }
}

impl From<&Path> for ModelsRoot {
    fn from(path: &Path) -> Self {
        Self::new(path)
    }
}

impl From<&PathBuf> for ModelsRoot {
    fn from(path: &PathBuf) -> Self {
        Self::new(path.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_reaches_every_clone() {
        let root = ModelsRoot::new("/a");
        let other = root.clone();
        assert_eq!(other.join("m"), PathBuf::from("/a/m"));
        root.set("/b");
        assert_eq!(other.get(), PathBuf::from("/b"));
        assert_eq!(other.join("m"), PathBuf::from("/b/m"));
        // A root made from a path is its own.
        let alone = ModelsRoot::from(Path::new("/a"));
        root.set("/c");
        assert_eq!(alone.get(), PathBuf::from("/a"));
    }
}
