//! Every window runs the interface this program carries (docs/dictation.md §33.4).
//!
//! In the first run after an update the user saw the dictionary page without its 学习建议 tab
//! until Voltip was restarted (user report 2026-11-02): a window that, by all signs, ran the
//! interface of the build before. Updating 0.2.1 on hosted Windows never showed it (§33.3), so the
//! cause is not known; this is the protection. [`init`] hands every document, as it is created,
//! what the program carries (`ui_guard.js`): the paths of its scripts and the build its bundle
//! declares (`apps/desktop/src/ui-build.ts`, from `apps/desktop/vite.config.ts`). A document
//! running other code is replaced once by one the program serves; one that still runs other code
//! is kept as it is. Each document reports what the check found ([`EVENT`]), and the program logs
//! it ([`log`]).
//!
//! Both lists are read from the program's assets, so they are the ones this build serves. A
//! development build (`tauri dev`, the interface from Vite) and a program that carries no script
//! (the tests' mock context) get no check; a bundle that declares no build is checked by its
//! scripts alone.

use serde::Deserialize;
use tauri::plugin::TauriPlugin;
use tauri::utils::assets::AssetKey;
use tauri::{Assets, Listener, Runtime};
use tracing::Level;

/// The check, with [`SCRIPTS`] and [`BUILDS`] standing for what the program carries.
const CHECK: &str = include_str!("ui_guard.js");
const SCRIPTS: &str = "__VOLTIP_UI_SCRIPTS__";
const BUILDS: &str = "__VOLTIP_UI_BUILDS__";
/// How a bundle names its build: this, then 32 lowercase hex digits (`apps/desktop/vite.config.ts`).
const BUILD_PREFIX: &[u8] = b"voltip-ui-build-";
const BUILD_DIGITS: usize = 32;
/// The event every checked document reports what it found on (`ui_guard.js`).
const EVENT: &str = "voltip://ui-guard";

/// The plugin that checks every window's interface against what `assets` carries.
pub fn init<R: Runtime>(assets: &dyn Assets<R>) -> TauriPlugin<R> {
    let carried = if tauri::is_dev() { Carried::default() } else { carried(assets) };
    plugin(&carried)
}

/// What the program carries: its script paths as a document's URLs name them, and the builds its
/// scripts declare. Both sorted.
#[derive(Debug, Default, PartialEq, Eq)]
struct Carried {
    scripts: Vec<String>,
    builds: Vec<String>,
}

fn carried<R: Runtime>(assets: &dyn Assets<R>) -> Carried {
    let scripts = carried_scripts(assets.iter().map(|(path, _)| path));
    let mut builds: Vec<String> =
        scripts.iter().filter_map(|script| assets.get(&AssetKey::from(script.as_str()))).flat_map(|bytes| declared_builds(&bytes)).collect();
    builds.sort();
    builds.dedup();
    Carried { scripts, builds }
}

fn plugin<R: Runtime>(carried: &Carried) -> TauriPlugin<R> {
    let builder = tauri::plugin::Builder::new("ui-guard");
    match check(carried) {
        Some(check) => builder
            .js_init_script(check)
            .setup(|app, _api| {
                app.listen_any(EVENT, |event| log(event.payload()));
                Ok(())
            })
            .build(),
        None => builder.build(),
    }
}

/// What one document's check found, as `ui_guard.js` reports it. Written by the page, so every
/// field is bounded before it reaches the log ([`bounded`]).
#[derive(Debug, Deserialize, PartialEq, Eq)]
struct Report {
    window: Option<String>,
    verdict: String,
    #[serde(default)]
    replaced: bool,
    #[serde(default)]
    scripts: Vec<String>,
    build: Option<String>,
}

/// How a report reads in the program's log. A window that runs the program's interface is one
/// line at startup; any replacement is a warning, and a window kept with other code an error.
fn classify(report: &Report) -> (Level, &'static str) {
    match (report.verdict.as_str(), report.replaced) {
        ("ok", false) => (Level::INFO, "the window runs the interface the program carries"),
        ("ok", true) => (Level::WARN, "the window ran interface code the program does not carry; its document was replaced once and now runs the program's"),
        ("replacing", _) => (Level::WARN, "the window runs interface code the program does not carry; replacing its document once"),
        ("kept", _) => (Level::ERROR, "the window still runs interface code the program does not carry after its document was replaced once; it is kept as it is"),
        _ => (Level::WARN, "the interface check reported something it never reports"),
    }
}

/// At most `max` characters of `text`, as the log shows it.
fn bounded(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_owned(),
    }
}

/// Log one document's report (docs/dictation.md §33.4): which window, what the check found, the
/// scripts it runs and the build it declared.
fn log(payload: &str) {
    let Ok(report) = serde_json::from_str::<Report>(payload) else {
        tracing::warn!(payload = %bounded(payload, 200), "the interface check sent a report that does not parse");
        return;
    };
    let window = bounded(report.window.as_deref().unwrap_or("?"), 32);
    let verdict = bounded(&report.verdict, 16);
    let build = bounded(report.build.as_deref().unwrap_or("none"), 48);
    let scripts = report.scripts.iter().take(8).map(|s| bounded(s, 120)).collect::<Vec<_>>().join(" ");
    match classify(&report) {
        (Level::INFO, line) => tracing::info!(window, build, "{line}"),
        (Level::WARN, line) => tracing::warn!(window, verdict, replaced = report.replaced, build, scripts, "{line}"),
        (_, line) => tracing::error!(window, verdict, replaced = report.replaced, build, scripts, "{line}"),
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

/// The builds a script declares: every [`BUILD_PREFIX`] followed by exactly [`BUILD_DIGITS`]
/// lowercase hex digits, in order.
fn declared_builds(script: &[u8]) -> Vec<String> {
    let mut builds = Vec::new();
    let mut from = 0;
    while let Some(at) = script[from..].windows(BUILD_PREFIX.len()).position(|w| w == BUILD_PREFIX) {
        let start = from + at;
        let digits = &script[start + BUILD_PREFIX.len()..];
        let hex = digits.iter().take_while(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(*b)).count();
        if hex == BUILD_DIGITS && digits.get(hex).is_none_or(|b| !b.is_ascii_alphanumeric()) {
            builds.push(String::from_utf8_lossy(&script[start..start + BUILD_PREFIX.len() + hex]).into_owned());
        }
        from = start + BUILD_PREFIX.len();
    }
    builds
}

/// `ui_guard.js` for what the program carries; none when it carries no script.
fn check(carried: &Carried) -> Option<String> {
    if carried.scripts.is_empty() {
        return None;
    }
    // A JSON array of strings is a JavaScript array literal.
    let scripts = serde_json::to_string(&carried.scripts).ok()?;
    let builds = serde_json::to_string(&carried.builds).ok()?;
    Some(CHECK.replace(SCRIPTS, &scripts).replace(BUILDS, &builds))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const BUILD: &str = "voltip-ui-build-0123456789abcdef0123456789abcdef";

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
        assert_eq!(carried_scripts(assets.iter()), ["/assets/chunk-Zz9.js", "/assets/index-BBjVjT-j.js", "/assets/worker-Ab12Cd34.mjs"]);
        assert!(carried_scripts(["/index.html", "/assets/index.css"].iter()).is_empty());
    }

    #[test]
    fn a_bundle_declares_its_build_as_vite_writes_it() {
        // As the minified bundle carries the define: a string literal.
        let bundle = format!(r#"var a="{BUILD}";window.voltipUiBuild=a;"#);
        assert_eq!(declared_builds(bundle.as_bytes()), [BUILD]);
        // The prefix alone, too few or too many digits, upper case, a letter after: not a build.
        let others = [
            r#"s.startsWith("voltip-ui-build-")"#.to_owned(),
            format!("{}\"", &BUILD[..BUILD.len() - 1]),
            format!("{BUILD}0"),
            BUILD.to_uppercase(),
            format!("{BUILD}g"),
        ];
        for other in others {
            assert!(declared_builds(other.as_bytes()).is_empty(), "{other}");
        }
        // Two in one script, each found; one at the very end of the bytes, too.
        let two = format!("{BUILD} voltip-ui-build-ffffffffffffffffffffffffffffffff");
        assert_eq!(declared_builds(two.as_bytes()).len(), 2);
        assert_eq!(declared_builds(BUILD.as_bytes()), [BUILD]);
    }

    #[test]
    fn the_check_carries_both_lists_in_place_of_their_placeholders() {
        let check = check_for(&["/assets/index-BBjVjT-j.js", "/assets/a \"quoted\" name.js"], &[BUILD]);
        assert!(!check.contains(SCRIPTS) && !check.contains(BUILDS), "no placeholder left");
        assert!(check.contains(r#"const carried = ["/assets/index-BBjVjT-j.js","/assets/a \"quoted\" name.js"];"#), "{check}");
        assert!(check.contains(&format!(r#"const builds = ["{BUILD}"];"#)), "{check}");
        // A bundle that declares no build is checked by its scripts.
        let unnamed = check_for(&["/assets/index-BBjVjT-j.js"], &[]);
        assert!(unnamed.contains("const builds = [];"), "{unnamed}");
    }

    #[test]
    fn a_program_that_carries_no_script_gets_no_check() {
        assert_eq!(check(&Carried::default()), None);
        assert_eq!(check(&Carried { scripts: Vec::new(), builds: vec![BUILD.to_owned()] }), None);
        // Each placeholder appears once, where its list goes.
        assert_eq!(CHECK.matches(SCRIPTS).count(), 1);
        assert_eq!(CHECK.matches(BUILDS).count(), 1);
    }

    #[test]
    fn the_plugin_builds_with_and_without_a_check() {
        let with = Carried { scripts: vec!["/assets/index-BBjVjT-j.js".to_owned()], builds: vec![BUILD.to_owned()] };
        let _with: TauriPlugin<tauri::test::MockRuntime> = plugin(&with);
        let _without: TauriPlugin<tauri::test::MockRuntime> = plugin(&Carried::default());
    }

    #[test]
    fn a_report_reads_as_ui_guard_js_sends_it() {
        // As `ui_guard.js` emits it, with the window's label from Tauri's metadata.
        let payload = format!(r#"{{"window":"main","verdict":"ok","replaced":false,"scripts":["/assets/index-BBjVjT-j.js"],"build":"{BUILD}"}}"#);
        let report: Report = serde_json::from_str(&payload).unwrap();
        assert_eq!(
            report,
            Report { window: Some("main".to_owned()), verdict: "ok".to_owned(), replaced: false, scripts: vec!["/assets/index-BBjVjT-j.js".to_owned()], build: Some(BUILD.to_owned()) }
        );
        // A bundle that declares no build, and a window Tauri named no label for.
        let bare: Report = serde_json::from_str(r#"{"window":null,"verdict":"replacing","replaced":false,"scripts":[],"build":null}"#).unwrap();
        assert_eq!((bare.window, bare.build), (None, None));
        // The log line for anything else says so; it never panics.
        log("not json");
        log(&payload);
    }

    #[test]
    fn each_verdict_has_its_own_line_and_weight() {
        let report = |verdict: &str, replaced| Report { window: None, verdict: verdict.to_owned(), replaced, scripts: Vec::new(), build: None };
        assert_eq!(classify(&report("ok", false)).0, Level::INFO);
        assert_eq!(classify(&report("ok", true)).0, Level::WARN);
        assert_eq!(classify(&report("replacing", false)).0, Level::WARN);
        assert_eq!(classify(&report("kept", true)), (Level::ERROR, "the window still runs interface code the program does not carry after its document was replaced once; it is kept as it is"));
        assert_eq!(classify(&report("something", false)).0, Level::WARN);
        // Every verdict ui_guard.js sends has its own line.
        let lines: std::collections::BTreeSet<_> = [report("ok", false), report("ok", true), report("replacing", false), report("kept", true)].iter().map(|r| classify(r).1).collect();
        assert_eq!(lines.len(), 4);
        for verdict in ["\"ok\"", "\"replacing\"", "\"kept\""] {
            assert!(CHECK.contains(&format!("report({verdict}")), "ui_guard.js reports {verdict}");
        }
        assert!(CHECK.contains(EVENT), "ui_guard.js reports on {EVENT}");
    }

    #[test]
    fn the_plugin_logs_what_each_window_reports() {
        use std::io::Write;
        use std::sync::{Arc, Mutex};
        use tauri::Emitter;

        #[derive(Clone, Default)]
        struct Lines(Arc<Mutex<Vec<u8>>>);
        impl Write for Lines {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let lines = Lines::default();
        let writer = lines.clone();
        let subscriber = tracing_subscriber::fmt().with_ansi(false).with_writer(move || writer.clone()).finish();
        tracing::subscriber::with_default(subscriber, || {
            let with = Carried { scripts: vec!["/assets/index-BBjVjT-j.js".to_owned()], builds: vec![BUILD.to_owned()] };
            let app = tauri::test::mock_builder().plugin(plugin(&with)).build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
            let report = |verdict: &str, replaced: bool| serde_json::json!({ "window": "main", "verdict": verdict, "replaced": replaced, "scripts": ["/assets/index-OLD00001.js"], "build": null });
            app.emit(EVENT, report("ok", false)).unwrap();
            app.emit(EVENT, report("kept", true)).unwrap();
        });
        let logged = String::from_utf8(lines.0.lock().unwrap().clone()).unwrap();
        assert!(logged.contains("INFO") && logged.contains(r#"the window runs the interface the program carries window="main""#), "{logged}");
        assert!(logged.contains("ERROR") && logged.contains("it is kept as it is") && logged.contains("/assets/index-OLD00001.js"), "{logged}");
    }

    #[test]
    fn what_a_page_reports_is_bounded_before_the_log() {
        assert_eq!(bounded("main", 32), "main");
        assert_eq!(bounded(&"x".repeat(40), 32), format!("{}…", "x".repeat(32)));
        // Characters, not bytes: a cut never splits one.
        assert_eq!(bounded("学习建议学习建议", 4), "学习建议…");
    }

    fn check_for(scripts: &[&str], builds: &[&str]) -> String {
        check(&Carried { scripts: scripts.iter().map(|s| (*s).to_owned()).collect(), builds: builds.iter().map(|s| (*s).to_owned()).collect() }).unwrap()
    }
}
