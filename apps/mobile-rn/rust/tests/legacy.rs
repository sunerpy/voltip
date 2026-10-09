#![allow(clippy::unwrap_used, clippy::expect_used)]
//! An update from the Tauri phone app (the Android app up to 0.0.49) to this one, which took its
//! package over in 0.0.50 (user decision 2026-10-09): the core's files move from the root of the
//! app's data directory into `files/voltip`, and nothing else is touched.

use std::fs;
use std::path::Path;

use voltip_rn::legacy::{adopt_tauri_data, app_root_of};

fn write(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

#[test]
fn an_update_from_the_tauri_app_moves_its_files_into_the_new_data_dir() {
    let root = tempfile::tempdir().unwrap();
    let data_dir = root.path().join("files/voltip");
    fs::create_dir_all(&data_dir).unwrap();
    for name in [
        "settings.json",
        "presets.json",
        "scenes.json",
        "rules.json",
        "dictionary.json",
        "trusted-devices.json",
        "sent-texts.json",
        "history.sqlite3",
        "history.sqlite3-wal",
        "history.sqlite3-shm",
        "history.json.imported-1790000000",
    ] {
        write(&root.path().join(name), name);
    }
    write(&root.path().join("uploads/a1b2.json"), "upload");
    write(&root.path().join("mirror/0f1e.sqlite3"), "mirror");
    // Android's own folders and the Tauri app's webview stay where they are.
    write(&root.path().join("shared_prefs/dev.voltip.mobile.xml"), "keystore entries");
    write(&root.path().join("app_webview/Default/Preferences"), "webview");
    write(&root.path().join("cache/x"), "cache");

    let moved = adopt_tauri_data(root.path(), &data_dir);

    assert_eq!(moved.len(), 13, "{moved:?}");
    for name in ["settings.json", "trusted-devices.json", "history.sqlite3", "history.sqlite3-wal", "history.json.imported-1790000000"] {
        assert_eq!(fs::read_to_string(data_dir.join(name)).unwrap(), name);
        assert!(!root.path().join(name).exists(), "{name} is still at the root");
    }
    assert_eq!(fs::read_to_string(data_dir.join("uploads/a1b2.json")).unwrap(), "upload");
    assert_eq!(fs::read_to_string(data_dir.join("mirror/0f1e.sqlite3")).unwrap(), "mirror");
    assert!(root.path().join("shared_prefs/dev.voltip.mobile.xml").exists());
    assert!(root.path().join("app_webview/Default/Preferences").exists());
    assert!(root.path().join("cache/x").exists());
    // A second start finds nothing more to move.
    assert!(adopt_tauri_data(root.path(), &data_dir).is_empty());
}

#[test]
fn a_phone_that_already_has_its_own_data_keeps_it() {
    let root = tempfile::tempdir().unwrap();
    let data_dir = root.path().join("files/voltip");
    write(&data_dir.join("settings.json"), "the app's own");
    write(&root.path().join("settings.json"), "the Tauri app's");
    write(&root.path().join("history.sqlite3"), "old history");

    assert!(adopt_tauri_data(root.path(), &data_dir).is_empty());
    assert_eq!(fs::read_to_string(data_dir.join("settings.json")).unwrap(), "the app's own");
    assert!(root.path().join("history.sqlite3").exists());
    assert!(!data_dir.join("history.sqlite3").exists());
}

#[test]
fn a_fresh_install_moves_nothing() {
    let root = tempfile::tempdir().unwrap();
    let data_dir = root.path().join("files/voltip");
    fs::create_dir_all(&data_dir).unwrap();
    write(&root.path().join("shared_prefs/other.xml"), "x");

    assert!(adopt_tauri_data(root.path(), &data_dir).is_empty());
    assert_eq!(fs::read_dir(&data_dir).unwrap().count(), 0);
}

#[test]
fn the_app_root_is_two_levels_above_the_apps_data_dir() {
    let root = Path::new("/data/user/0/dev.voltip.mobile");
    assert_eq!(app_root_of(&root.join("files/voltip")), Some(root));
    // Any other layout (a test's temporary folder, a later change) migrates nothing.
    assert_eq!(app_root_of(Path::new("/tmp/voltip")), None);
    assert_eq!(app_root_of(Path::new("/data/user/0/dev.voltip.mobile/voltip")), None);
}
