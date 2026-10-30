//! How soon the insert check's reader sees a paste (docs/dictation.md §28, 测量). With an
//! application in front and the caret in one of its fields, it pastes `--runs` times the way a
//! take is pasted (one paste session: the clipboard, the paste chord, the clipboard put back
//! afterwards), reads the field every 10 ms from the paste until it changes (at most 3 s), and
//! writes one CSV row per paste:
//!
//! `app,run,read_ms,first_change_ms,first_change,shows_text,present_at_3s,error`
//!
//! - `read_ms`: how long the reading before the paste took;
//! - `first_change_ms`: from the moment the paste chord went out to the first reading that differs
//!   from the one before (empty when none did within 3 s) — what `CONFIRM_WITHIN` has to cover;
//! - `first_change`: what that reading was, `text` (the field's text changed) or `other-control`
//!   (another control has the focus);
//! - `shows_text`: that first changed reading already showed the pasted text at the caret;
//! - `present_at_3s`: a reading 3 s after the paste shows it (`false` with no change at all is a
//!   paste the reader missed, if the text did arrive: such an application goes into the skip table).
//!
//! First it reads the field until it answers, for at most `--warmup` seconds (default 20):
//! Chromium builds its accessibility tree on the first query, so the first readings of a browser
//! may find nothing. After each paste the field is put back (`--reset undo`: Ctrl+Z / Cmd+Z), or the next paste
//! replaces all of its text (`--reset select-all`: Ctrl+A / Cmd+A first; GTK 3 entries have no
//! undo). The last line on standard error sums the runs up. It pastes into whatever is in front:
//! run it on a desktop nobody is using, as CI's measurement steps do:
//!
//! `cargo run -p voltip-focus --example insert_confirm_probe -- --app notepad --app-id notepad --runs 50 --out probe.csv`
//!
//! `--app-id` is the probe's id of the application in front (the table of skipped applications
//! goes by it); without it, `--app` stands in.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use voltip_focus::{Expected, Reader, Sample};
use voltip_inject::{AfterPaste, Chord, Injector as _, Key, KeystrokePort, PasteOptions};
use voltip_platform::insert_check::rules::{InsertCheck, decide};

/// How often the field is read while the probe waits for the paste.
const EVERY: Duration = Duration::from_millis(10);
/// How long the probe waits for the paste to show.
const WITHIN: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Reset {
    Undo,
    SelectAll,
}

struct Args {
    app: String,
    app_id: Option<String>,
    runs: usize,
    out: Option<String>,
    reset: Reset,
    warmup: Duration,
}

fn args() -> Args {
    let mut parsed = Args { app: "app".into(), app_id: None, runs: 50, out: None, reset: Reset::Undo, warmup: Duration::from_secs(20) };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = || it.next().unwrap_or_else(|| panic!("{arg} needs a value"));
        match arg.as_str() {
            "--app" => parsed.app = value(),
            "--app-id" => parsed.app_id = Some(value()),
            "--runs" => parsed.runs = value().parse().expect("--runs <n>"),
            "--out" => parsed.out = Some(value()),
            "--warmup" => parsed.warmup = Duration::from_secs(value().parse().expect("--warmup <seconds>")),
            "--reset" => {
                parsed.reset = match value().as_str() {
                    "undo" => Reset::Undo,
                    "select-all" => Reset::SelectAll,
                    other => panic!("--reset undo|select-all, not {other}"),
                };
            }
            other => panic!("unknown argument {other}"),
        }
    }
    parsed
}

/// The keyboard as the paste uses it, for the reset chords.
fn keys() -> Box<dyn KeystrokePort> {
    #[cfg(target_os = "linux")]
    {
        Box::new(voltip_inject::linux::system_toolchain_keys())
    }
    #[cfg(not(target_os = "linux"))]
    {
        Box::new(voltip_inject::SystemKeys::native())
    }
}

/// Ctrl+`key` (Cmd+`key` on macOS).
fn chord(key: char) -> Chord {
    let mac = cfg!(target_os = "macos");
    Chord { control: !mac, meta: mac, shift: false, key: Key::Char(key) }
}

/// One paste's measurement.
#[derive(Default)]
struct Row {
    read_ms: Option<u128>,
    first_change_ms: Option<u128>,
    first_change: &'static str,
    shows_text: bool,
    present_at_3s: bool,
    error: Option<String>,
}

fn main() {
    let args = args();
    // On macOS the Accessibility API answers a process only once it is connected to the window
    // server, as an application is (CI 2026-10-29: a command-line probe read nothing until its first
    // paste); NSApplication makes the connection.
    #[cfg(target_os = "macos")]
    let _app = objc2_app_kit::NSApplication::sharedApplication(objc2::MainThreadMarker::new().expect("the main thread"));
    let reader = Reader::system().expect("no reader on this platform");
    let options = PasteOptions::for_os(std::env::consts::OS);
    let injector = Arc::new(voltip_inject::system_paste_injector(options));
    let keys = keys();
    let mut out: Box<dyn std::io::Write> = match &args.out {
        Some(path) => Box::new(std::fs::File::create(path).expect("--out")),
        None => Box::new(std::io::stdout()),
    };
    writeln!(out, "app,run,read_ms,first_change_ms,first_change,shows_text,present_at_3s,error").unwrap();
    let app_id = args.app_id.as_deref().unwrap_or(&args.app);
    let warming = Instant::now();
    loop {
        match reader.watch(app_id, &Expected::of("warm-up")) {
            Ok(_) => {
                eprintln!("insert_confirm_probe {}: the field answers after {} ms", args.app, warming.elapsed().as_millis());
                break;
            }
            Err(why) if warming.elapsed() >= args.warmup => {
                eprintln!("insert_confirm_probe {}: the field did not answer within {:?} ({why:?}); measuring anyway", args.app, args.warmup);
                break;
            }
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    }
    let mut rows = Vec::new();
    for run in 0..args.runs {
        if args.reset == Reset::SelectAll
            && let Err(e) = keys.press_chord(chord('a'), &[])
        {
            panic!("select all: {e:?}");
        }
        let text = format!("voltip probe {run}");
        let expected = Expected::of(&text);
        let mut row = Row::default();
        let started = Instant::now();
        match reader.watch(app_id, &expected) {
            Err(why) => row.error = Some(format!("{why:?}")),
            Ok(watch) => {
                row.read_ms = Some(started.elapsed().as_millis());
                let mut session = injector.clone().begin_session(AfterPaste::Restore);
                session.piece(&text, &|| true).expect("the paste went out");
                // `piece` returns as the chord goes out: the check's window starts here.
                let pasted = Instant::now();
                let before = watch.before().clone();
                let first_changed = loop {
                    let sample = watch.sample();
                    let changed = matches!(&sample, Sample::Text(now) if *now != before) || sample == Sample::OtherControl;
                    if changed {
                        break Some((pasted.elapsed(), sample));
                    }
                    if pasted.elapsed() >= WITHIN {
                        break None;
                    }
                    std::thread::sleep(EVERY);
                };
                if let Some((took, sample)) = &first_changed {
                    row.first_change_ms = Some(took.as_millis());
                    row.first_change = if *sample == Sample::OtherControl { "other-control" } else { "text" };
                    row.shows_text = decide(&before, std::slice::from_ref(sample), &expected).check == InsertCheck::Written;
                }
                std::thread::sleep(WITHIN.saturating_sub(pasted.elapsed()));
                row.present_at_3s = decide(&before, &[watch.sample()], &expected).check == InsertCheck::Written;
                session.finish();
            }
        }
        // The clipboard comes back after the paste's restore delay; then the field is put back.
        std::thread::sleep(options.paste_delay_after + Duration::from_millis(200));
        if args.reset == Reset::Undo
            && let Err(e) = keys.press_chord(chord('z'), &[])
        {
            panic!("undo: {e:?}");
        }
        std::thread::sleep(Duration::from_millis(200));
        let cell = |v: Option<u128>| v.map(|v| v.to_string()).unwrap_or_default();
        writeln!(
            out,
            "{},{run},{},{},{},{},{},{}",
            args.app,
            cell(row.read_ms),
            cell(row.first_change_ms),
            row.first_change,
            row.shows_text,
            row.present_at_3s,
            row.error.clone().unwrap_or_default()
        )
        .unwrap();
        rows.push(row);
    }
    out.flush().unwrap();
    let changes: Vec<u128> = rows.iter().filter_map(|r| r.first_change_ms).collect();
    let missed = rows.iter().filter(|r| r.error.is_none() && r.first_change_ms.is_none()).count();
    let unread = rows.iter().filter(|r| r.error.is_some()).count();
    let present = rows.iter().filter(|r| r.present_at_3s).count();
    eprintln!(
        "insert_confirm_probe {}: {} runs, {} read, {} unreadable, {} changed (max {} ms, max read {} ms), {} no change in 3 s, {} present at 3 s",
        args.app,
        rows.len(),
        rows.len() - unread,
        unread,
        changes.len(),
        changes.iter().max().copied().unwrap_or(0),
        rows.iter().filter_map(|r| r.read_ms).max().unwrap_or(0),
        missed,
        present
    );
}
