//! The Tauri phone app's files (docs/mobile-rn.md §1). Up to 0.0.49 the Android app was the Tauri
//! shell (`apps/mobile`), which kept the core's files at the root of the app's data directory
//! (`Context.getDataDir()`, Tauri's `app_data_dir`). This app took its package over in 0.0.50 (user
//! decision 2026-10-09) and keeps them in `files/voltip`, so on its first start after the update it
//! moves them there: the phone keeps its pairings, history and settings. The Keystore entries (the
//! device identity, the provider keys) need nothing: both shells use the same service id in the
//! same package.

use std::path::Path;

/// The core's files and folders on a phone, by name; every `history.json…` and `history.sqlite3…`
/// file goes too (the database's journal files, an import's leftovers, a retired `history.json`).
const CORE_ENTRIES: &[&str] = &[
    "settings.json",
    "presets.json",
    "scenes.json",
    "rules.json",
    "dictionary.json",
    "trusted-devices.json",
    "sent-texts.json",
    "uploads",
    "recordings",
    "mirror",
];
const CORE_PREFIXES: &[&str] = &["history.json", "history.sqlite3"];

/// The app's data directory for `data_dir`, which the app sets to `<data directory>/files/voltip`
/// (`VoltipNativeModule.kt`); `None` for any other layout, which then migrates nothing.
pub fn app_root_of(data_dir: &Path) -> Option<&Path> {
    let files = data_dir.parent()?;
    (data_dir.file_name()? == "voltip" && files.file_name()? == "files").then_some(())?;
    files.parent()
}

/// Move the Tauri app's files from `root` into `data_dir`, unless `data_dir` already holds the
/// app's own settings (it has run before) or `root` holds none of the core's files (a new
/// install). Returns the names moved. A file that cannot be moved stays where it was, logged:
/// the core then starts without it, as on a new install.
pub fn adopt_tauri_data(root: &Path, data_dir: &Path) -> Vec<String> {
    if data_dir.join("settings.json").exists() {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| CORE_ENTRIES.contains(&name.as_str()) || CORE_PREFIXES.iter().any(|prefix| name.starts_with(prefix)))
        .collect();
    if names.is_empty() {
        return names;
    }
    names.sort();
    if let Err(e) = std::fs::create_dir_all(data_dir) {
        tracing::warn!(error = %e, "the app's data folder could not be made; the Tauri app's files stay where they are");
        return Vec::new();
    }
    names.retain(|name| {
        let to = data_dir.join(name);
        if to.exists() {
            tracing::warn!(%name, "already in the app's data folder; the Tauri app's copy stays where it is");
            return false;
        }
        match std::fs::rename(root.join(name), &to) {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!(%name, error = %e, "the Tauri app's file could not be moved");
                false
            }
        }
    });
    tracing::info!(moved = names.len(), "the Tauri app's files moved into the app's data folder");
    names
}
