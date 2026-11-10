//! Every window runs the interface this program carries (docs/dictation.md §33.4).
//!
//! In the first run after an update the user saw the dictionary page without its 学习建议 tab
//! until Voltip was restarted (user report 2026-11-02): a window that, by all signs, ran the
//! interface of the build before. Updating 0.2.1 on hosted Windows never showed it (§33.3), so the
//! cause is not known; this is the protection. [`init`] hands every document, as it is created,
//! what the program carries (`ui_guard.js`): the paths of its scripts and the build its bundle
//! declares (`apps/desktop/src/ui-build.ts`, from `apps/desktop/vite.config.ts`). A document
//! running other code is replaced once by one the program serves; one that still runs other code
//! is kept as it is. Each document reports what the check found ([`EVENT`]) until the program has
//! the report, and the program logs it once per document ([`Logged`], [`log`]).
//!
//! Both lists are read from the program's assets, so they are the ones this build serves. A
//! development build (`tauri dev`, the interface from Vite) and a program that carries no script
//! (the tests' mock context) get no check; a bundle that declares no build is checked by its
//! scripts alone.

use std::collections::VecDeque;

use parking_lot::Mutex;
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
/// How many documents [`Logged`] remembers: far more than a program's windows load.
const REMEMBERED: usize = 64;

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
    plugin_reporting_to(carried, log)
}

/// [`plugin`], with every document's report handed to `report` (the program's is [`log`]).
fn plugin_reporting_to<R: Runtime>(carried: &Carried, report: fn(&str)) -> TauriPlugin<R> {
    let builder = tauri::plugin::Builder::new("ui-guard");
    match check(carried) {
        Some(check) => builder
            .js_init_script(check)
            .setup(move |app, _api| {
                let logged = Logged::default();
                app.listen_any(EVENT, move |event| {
                    if logged.first(event.payload()) {
                        report(event.payload());
                    }
                });
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
    /// The document's id, the same in every send of its report.
    #[serde(default)]
    document: Option<String>,
    verdict: String,
    #[serde(default)]
    replaced: bool,
    #[serde(default)]
    scripts: Vec<String>,
    build: Option<String>,
}

/// The documents whose report the program has logged. A document sends its report again until the
/// program has it (`ui_guard.js`): a late answer and the send after it can both arrive, and the
/// program logs the document once. A report that names no document is logged every time.
#[derive(Debug, Default)]
struct Logged(Mutex<VecDeque<String>>);

impl Logged {
    /// Whether `payload` is the first report the program hears from its document.
    fn first(&self, payload: &str) -> bool {
        let Some(document) = serde_json::from_str::<Report>(payload).ok().and_then(|report| report.document) else {
            return true;
        };
        let document = bounded(&document, 64);
        let mut seen = self.0.lock();
        if seen.contains(&document) {
            return false;
        }
        if seen.len() == REMEMBERED {
            seen.pop_front();
        }
        seen.push_back(document);
        true
    }
}

/// How a report reads in the program's log. A window that runs the program's interface is one
/// line at startup; any replacement is a warning, and a window kept with other code an error.
fn classify(report: &Report) -> (Level, &'static str) {
    match (report.verdict.as_str(), report.replaced) {
        ("ok", false) => (Level::INFO, "the window runs the interface the program carries"),
        ("ok", true) => (Level::WARN, "the window ran interface code the program does not carry; its document was replaced once and now runs the program's"),
        ("replacing", _) => (Level::WARN, "the window runs interface code the program does not carry; replacing its document once"),
        ("kept", _) => {
            (Level::ERROR, "the window still runs interface code the program does not carry after its document was replaced once; it is kept as it is")
        }
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
        let payload =
            format!(r#"{{"window":"main","document":"k3v9q1x7","verdict":"ok","replaced":false,"scripts":["/assets/index-BBjVjT-j.js"],"build":"{BUILD}"}}"#);
        let report: Report = serde_json::from_str(&payload).unwrap();
        assert_eq!(
            report,
            Report {
                window: Some("main".to_owned()),
                document: Some("k3v9q1x7".to_owned()),
                verdict: "ok".to_owned(),
                replaced: false,
                scripts: vec!["/assets/index-BBjVjT-j.js".to_owned()],
                build: Some(BUILD.to_owned())
            }
        );
        // A bundle that declares no build, a window Tauri named no label for, and a page that names no
        // document.
        let bare: Report = serde_json::from_str(r#"{"window":null,"verdict":"replacing","replaced":false,"scripts":[],"build":null}"#).unwrap();
        assert_eq!((bare.window, bare.document, bare.build), (None, None, None));
        // The log line for anything else says so; it never panics.
        log("not json");
        log(&payload);
    }

    #[test]
    fn each_verdict_has_its_own_line_and_weight() {
        let report = |verdict: &str, replaced| Report { window: None, document: None, verdict: verdict.to_owned(), replaced, scripts: Vec::new(), build: None };
        assert_eq!(classify(&report("ok", false)).0, Level::INFO);
        assert_eq!(classify(&report("ok", true)).0, Level::WARN);
        assert_eq!(classify(&report("replacing", false)).0, Level::WARN);
        assert_eq!(
            classify(&report("kept", true)),
            (Level::ERROR, "the window still runs interface code the program does not carry after its document was replaced once; it is kept as it is")
        );
        assert_eq!(classify(&report("something", false)).0, Level::WARN);
        // Every verdict ui_guard.js sends has its own line.
        let lines: std::collections::BTreeSet<_> =
            [report("ok", false), report("ok", true), report("replacing", false), report("kept", true)].iter().map(|r| classify(r).1).collect();
        assert_eq!(lines.len(), 4);
        for verdict in ["\"ok\"", "\"replacing\"", "\"kept\""] {
            assert!(CHECK.contains(&format!("report({verdict}")), "ui_guard.js reports {verdict}");
        }
        assert!(CHECK.contains(EVENT), "ui_guard.js reports on {EVENT}");
    }

    #[test]
    fn the_plugin_hands_every_report_to_the_log() {
        use std::sync::Mutex;
        use tauri::Emitter;

        static HEARD: Mutex<Vec<String>> = Mutex::new(Vec::new());
        fn hear(payload: &str) {
            HEARD.lock().unwrap().push(payload.to_owned());
        }
        let app = |carried: &Carried| {
            tauri::test::mock_builder().plugin(plugin_reporting_to(carried, hear)).build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap()
        };
        let report = serde_json::json!({ "window": "main", "verdict": "kept", "replaced": true, "scripts": ["/assets/index-OLD00001.js"], "build": null });
        let with = Carried { scripts: vec!["/assets/index-BBjVjT-j.js".to_owned()], builds: vec![BUILD.to_owned()] };
        app(&with).emit(EVENT, &report).unwrap();
        let heard: Vec<serde_json::Value> = HEARD.lock().unwrap().iter().map(|p| serde_json::from_str(p).unwrap()).collect();
        assert_eq!(heard, std::slice::from_ref(&report));
        // A program without the check listens for nothing.
        app(&Carried::default()).emit(EVENT, &report).unwrap();
        assert_eq!(HEARD.lock().unwrap().len(), 1);
    }

    #[test]
    fn regression_a_document_whose_report_came_twice_is_logged_once() {
        // CI 42546063667 and 42772486837 (hosted macOS): the pill window's only report never reached
        // the log, so `ui_guard.js` now sends a report again when a send gets no answer in 5 s. The
        // late answer and the second send can both arrive; each carries the document's id.
        use std::sync::Mutex;
        use tauri::Emitter;

        static HEARD: Mutex<Vec<String>> = Mutex::new(Vec::new());
        fn hear(payload: &str) {
            HEARD.lock().unwrap().push(payload.to_owned());
        }
        let with = Carried { scripts: vec!["/assets/index-BBjVjT-j.js".to_owned()], builds: vec![BUILD.to_owned()] };
        let app = tauri::test::mock_builder().plugin(plugin_reporting_to(&with, hear)).build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let report = |document: Option<&str>| serde_json::json!({ "window": "overlay", "document": document, "verdict": "ok", "replaced": false, "scripts": ["/assets/index-BBjVjT-j.js"], "build": BUILD });
        app.emit(EVENT, report(Some("k3v9q1x7"))).unwrap();
        app.emit(EVENT, report(Some("k3v9q1x7"))).unwrap();
        assert_eq!(HEARD.lock().unwrap().len(), 1, "the second report of a document is not logged");
        // The window's next document, and a report that names no document, are.
        app.emit(EVENT, report(Some("p8d2m4c6"))).unwrap();
        app.emit(EVENT, report(None)).unwrap();
        app.emit(EVENT, report(None)).unwrap();
        assert_eq!(HEARD.lock().unwrap().len(), 4);
    }

    #[test]
    fn the_program_remembers_the_latest_documents_it_logged() {
        let logged = Logged::default();
        let report = |document: usize| format!(r#"{{"window":"main","document":"d{document}","verdict":"ok"}}"#);
        assert!(logged.first(&report(0)));
        assert!(!logged.first(&report(0)));
        // The oldest is forgotten once `REMEMBERED` others came after it; the latest are still known.
        for document in 1..=REMEMBERED {
            assert!(logged.first(&report(document)));
        }
        assert!(logged.first(&report(0)), "d0 was forgotten");
        assert!(!logged.first(&report(REMEMBERED)));
        // What does not parse goes on to the log, which says so.
        assert!(logged.first("not json"));
        assert!(logged.first("not json"));
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
