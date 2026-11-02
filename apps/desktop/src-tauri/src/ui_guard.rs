//! Every window runs the interface this program carries (docs/dictation.md §33.4).
//!
//! In the first run after an update the user saw the dictionary page without its 学习建议 tab
//! until Voltip was restarted (user report 2026-11-02): a window that, by all signs, ran the
//! interface of the build before. Updating 0.2.1 on hosted Windows never showed it (§33.3), so the
//! cause is not known; this is the protection. [`init`] hands every document, as it is created, the
//! paths of the scripts the program carries (`ui_guard.js`); a document running a module script
//! that is not among them reloads, once per window.
//!
//! The scripts are read from the program's assets, so the list is the one this build serves. A
//! development build (`tauri dev`, the interface from Vite) and a program that carries no script
//! (the tests' mock context) get no check.

use tauri::plugin::TauriPlugin;
use tauri::{Assets, Runtime};

/// The check, with [`SCRIPTS`] standing for the list of carried script paths.
const CHECK: &str = include_str!("ui_guard.js");
const SCRIPTS: &str = "__VOLTIP_UI_SCRIPTS__";

/// The plugin that checks every window's interface against the scripts in `assets`.
pub fn init<R: Runtime>(assets: &dyn Assets<R>) -> TauriPlugin<R> {
    let scripts = if tauri::is_dev() { Vec::new() } else { carried_scripts(assets.iter().map(|(path, _)| path)) };
    plugin(&scripts)
}

fn plugin<R: Runtime>(scripts: &[String]) -> TauriPlugin<R> {
    let builder = tauri::plugin::Builder::new("ui-guard");
    match check(scripts) {
        Some(check) => builder.js_init_script(check).build(),
        None => builder.build(),
    }
}

/// The JavaScript and module script paths among `assets`, as a document's URLs name them
/// (`/assets/index-….js`), sorted.
fn carried_scripts<S: AsRef<str>>(assets: impl Iterator<Item = S>) -> Vec<String> {
    let mut scripts: Vec<String> = assets
        .filter_map(|path| {
            let path = path.as_ref().replace('\\', "/");
            let script = [".js", ".mjs"].iter().any(|ext| path.ends_with(ext));
            script.then(|| if path.starts_with('/') { path } else { format!("/{path}") })
        })
        .collect();
    scripts.sort();
    scripts.dedup();
    scripts
}

/// `ui_guard.js` for these scripts; none when there is nothing to compare with.
fn check(scripts: &[String]) -> Option<String> {
    if scripts.is_empty() {
        return None;
    }
    // A JSON array of strings is a JavaScript array literal.
    let list = serde_json::to_string(scripts).ok()?;
    Some(CHECK.replace(SCRIPTS, &list))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn the_carried_scripts_are_the_programs_javascript_files_as_urls_name_them() {
        let assets = [
            "/index.html",
            "/assets/index-BBjVjT-j.js",
            "/assets/index-BNb5zgef.css",
            "/assets/noto-sans-sc-100-wght-normal-DrqXJETY.woff2",
            "assets/worker-Ab12Cd34.mjs",
            "\\assets\\chunk-Zz9.js",
            "/assets/index-BBjVjT-j.js",
        ];
        assert_eq!(
            carried_scripts(assets.iter()),
            ["/assets/chunk-Zz9.js", "/assets/index-BBjVjT-j.js", "/assets/worker-Ab12Cd34.mjs"],
        );
        assert!(carried_scripts(["/index.html", "/assets/index.css"].iter()).is_empty());
    }

    #[test]
    fn the_check_carries_the_list_in_place_of_its_placeholder() {
        let scripts = ["/assets/index-BBjVjT-j.js".to_owned(), "/assets/a \"quoted\" name.js".to_owned()];
        let check = check(&scripts).unwrap();
        assert!(!check.contains(SCRIPTS), "no placeholder left");
        assert!(check.contains(r#"const carried = ["/assets/index-BBjVjT-j.js","/assets/a \"quoted\" name.js"];"#), "{check}");
        // Only the placeholder changed.
        assert_eq!(check.len() - CHECK.len() + SCRIPTS.len(), serde_json::to_string(&scripts).unwrap().len());
    }

    #[test]
    fn a_program_that_carries_no_script_gets_no_check() {
        assert_eq!(check(&[]), None);
        // The placeholder appears once, where the list goes.
        assert_eq!(CHECK.matches(SCRIPTS).count(), 1);
    }

    #[test]
    fn the_plugin_builds_with_and_without_a_check() {
        let _with: TauriPlugin<tauri::test::MockRuntime> = plugin(&["/assets/index-BBjVjT-j.js".to_owned()]);
        let _without: TauriPlugin<tauri::test::MockRuntime> = plugin(&[]);
    }
}
