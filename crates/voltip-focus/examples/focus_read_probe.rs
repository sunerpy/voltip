//! Which applications' text fields 自动跟踪输入框里的修改 can follow (docs/dictation.md §35.10,
//! 覆盖面 and 复测; round 2 of 2026-11-26 ran it on windows-local). Read-only: it presses no key,
//! moves no window and never prints any text. Twice a second it reads the field that has the
//! keyboard focus the way the insert check reads it (§28) and, each time the focus moves to
//! another field or what can be read there changes, writes one line:
//!
//! `seconds,app,framework,control,class,password,patterns,readable,read_ms,reading,context`
//!
//! - `app`: the application's id as Voltip names it (the process image, `wechat`, `feishu`);
//! - `framework`, `control`, `class`: what UI Automation says the field is (`Chrome` / `Document`,
//!   `Win32` / `Edit` …), the names a reader needs to tell such fields apart;
//! - `patterns`: which text patterns the field offers (`text2`, `text`, `value`): with `text2` the
//!   caret can be read, with `value` only the whole text;
//! - `readable`: the insert check's reader got a reading; `read_ms` how long that took;
//! - `reading`: the reading's counts only — length, caret, selection, how many characters before
//!   the caret were read (`FocusText`'s `Debug` prints no text).
//! - `context`: the learning read's counts (`FocusContext`'s `Debug`: the paragraph's characters
//!   before, in and after the selection, no text) and how long it took. A line is also written
//!   whenever these counts change, so typing in a field shows whether the reader sees it.
//!
//! Run it the way Voltip runs (as administrator when Voltip does: a reader cannot see into a
//! window of a process with more rights), then click into the text field of each application to
//! check, type a few characters there (any; they are never printed) and wait for its line. Stop it
//! with Ctrl+C. The lines also go to `--out` (default `focus-read-probe.csv` in the current folder).
//!
//! `cargo run -p voltip-focus --example focus_read_probe -- [--minutes 15] [--out focus-read-probe.csv]`
//!
//! The first reading of a Chromium-based application (Chrome, Edge, Electron apps such as VS Code)
//! makes it turn its accessibility tree on; its first lines may say unreadable and a later one,
//! for the same field, readable. On macOS and Linux it reads through the same readers, without
//! the Windows-only columns.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Write as _;
use std::time::{Duration, Instant};

/// How often the focused field is read.
const EVERY: Duration = Duration::from_millis(500);

struct Args {
    /// How long it runs, in seconds (`--minutes`, or `--seconds` for a short check).
    seconds: u64,
    out: String,
}

fn args() -> Args {
    let mut parsed = Args { seconds: 15 * 60, out: "focus-read-probe.csv".into() };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = || it.next().unwrap_or_else(|| panic!("{arg} needs a value"));
        match arg.as_str() {
            "--minutes" => parsed.seconds = value().parse::<u64>().expect("--minutes <n>") * 60,
            "--seconds" => parsed.seconds = value().parse().expect("--seconds <n>"),
            "--out" => parsed.out = value(),
            other => panic!("unknown argument {other} (--minutes <n>, --seconds <n>, --out <file>)"),
        }
    }
    parsed
}

/// One line, to the console and the file.
struct Lines {
    file: std::fs::File,
    started: Instant,
}

impl Lines {
    fn open(path: &str) -> Self {
        let mut file = std::fs::File::create(path).unwrap_or_else(|e| panic!("{path}: {e}"));
        writeln!(file, "seconds,app,framework,control,class,password,patterns,readable,read_ms,reading,context").unwrap();
        println!("focus_read_probe: writing {path}; click into a text field, type a few characters, wait for its line; Ctrl+C stops");
        Self { file, started: Instant::now() }
    }

    fn write(&mut self, fields: &[String]) {
        let seconds = format!("{:.1}", self.started.elapsed().as_secs_f64());
        let quoted: Vec<String> =
            fields.iter().map(|f| if f.contains(',') || f.contains('"') { format!("\"{}\"", f.replace('"', "\"\"")) } else { f.clone() }).collect();
        let line = format!("{seconds},{}", quoted.join(","));
        println!("{line}");
        writeln!(self.file, "{line}").unwrap();
        self.file.flush().unwrap();
    }
}

fn main() {
    let args = args();
    let mut lines = Lines::open(&args.out);
    let until = Instant::now() + Duration::from_secs(args.seconds);
    platform::run(&mut lines, until);
    println!("focus_read_probe: its {} seconds are up; the lines are in {}", args.seconds, args.out);
}

#[cfg(target_os = "windows")]
mod platform {
    // UI Automation is COM: FFI. SAFETY (every block below): the interfaces are valid COM pointers
    // created on this thread's apartment, used as documented, none kept past its owner.
    #![allow(unsafe_code)]

    use std::time::Instant;

    use voltip_focus::FocusPlatform as _;
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
    use windows::Win32::UI::Accessibility::{
        CUIAutomation8, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern, IUIAutomationTextPattern2, IUIAutomationValuePattern,
        UIA_TextPattern2Id, UIA_TextPatternId, UIA_ValuePatternId,
    };
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};

    use super::{EVERY, Lines};

    /// UI Automation's control types the probe names; others print as their number.
    fn control_name(id: i32) -> String {
        let name = match id {
            50000 => "Button",
            50003 => "ComboBox",
            50004 => "Edit",
            50007 => "ListItem",
            50008 => "List",
            50020 => "Text",
            50025 => "Custom",
            50026 => "Group",
            50030 => "Document",
            50032 => "Window",
            50033 => "Pane",
            _ => return id.to_string(),
        };
        name.to_owned()
    }

    /// The image name of process `pid`, as Voltip's foreground probe names an application.
    fn app_of(pid: i32) -> String {
        let Ok(pid) = u32::try_from(pid) else { return "-".into() };
        // SAFETY: plain process queries; the buffer and its length outlive the call; the handle is
        // closed once read.
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return format!("pid {pid}");
            }
            let mut buffer = [0u16; 1024];
            let mut len = buffer.len() as u32;
            let ok = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, buffer.as_mut_ptr(), &raw mut len);
            CloseHandle(handle);
            if ok == 0 {
                return format!("pid {pid}");
            }
            let path = String::from_utf16_lossy(&buffer[..len as usize]);
            let file = path.rsplit(['\\', '/']).next().unwrap_or(&path);
            voltip_platform::foreground::normalize_app_id(file)
        }
    }

    /// What UI Automation says the field is, and which text patterns it offers.
    fn describe(element: &IUIAutomationElement) -> [String; 5] {
        // SAFETY: a valid element; unsupported properties and patterns are errors.
        unsafe {
            let app = element.CurrentProcessId().map_or_else(|_| "-".into(), app_of);
            let framework = element.CurrentFrameworkId().map_or_else(|_| "-".into(), |b| b.to_string());
            let control = element.CurrentControlType().map_or_else(|_| "-".into(), |t| control_name(t.0));
            let class = element.CurrentClassName().map_or_else(|_| "-".into(), |b| b.to_string());
            let mut patterns = Vec::new();
            if element.GetCurrentPatternAs::<IUIAutomationTextPattern2>(UIA_TextPattern2Id).is_ok() {
                patterns.push("text2");
            }
            if element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId).is_ok() {
                patterns.push("text");
            }
            if element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId).is_ok() {
                patterns.push("value");
            }
            [app, framework, control, class, if patterns.is_empty() { "none".into() } else { patterns.join("+") }]
        }
    }

    pub fn run(lines: &mut Lines, until: Instant) {
        // The insert check's reader (it initialises COM on this thread), and an automation object
        // of the probe's own for what the reader does not tell. `automation` goes first.
        let mut reader = voltip_focus::windows::open().expect("UI Automation");
        // SAFETY: COM is initialised on this thread by `open`.
        let automation: IUIAutomation = unsafe { CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER) }.expect("CUIAutomation8");
        let mut last: Option<(IUIAutomationElement, String)> = None;
        let mut nothing_said = false;
        while Instant::now() < until {
            // SAFETY: a valid automation object.
            match unsafe { automation.GetFocusedElement() } {
                Err(_) => {
                    if !nothing_said {
                        lines.write(&[
                            "-".into(),
                            "-".into(),
                            "-".into(),
                            "-".into(),
                            "-".into(),
                            "-".into(),
                            "nothing focused".into(),
                            "-".into(),
                            "-".into(),
                            "-".into(),
                        ]);
                        nothing_said = true;
                    }
                    last = None;
                }
                Ok(element) => {
                    let [app, framework, control, class, patterns] = describe(&element);
                    // SAFETY: a valid element.
                    let password = unsafe { element.CurrentIsPassword() }.is_ok_and(|b| b.as_bool());
                    let started = Instant::now();
                    let field = if password { None } else { reader.focused().ok() };
                    let reading = field.as_ref().and_then(|field| reader.text(field));
                    let read_ms = started.elapsed().as_millis();
                    let around = Instant::now();
                    let context = field.as_ref().and_then(|field| reader.context(field, None));
                    let context = context.map_or_else(|| "-".to_owned(), |c| format!("{c:?} in {} ms", around.elapsed().as_millis()));
                    let readable = reading.is_some();
                    let counts = reading.as_ref().map_or_else(String::new, |r| format!("{r:?}"));
                    let said = format!("{app}|{framework}|{control}|{class}|{patterns}|{readable}|{counts}|{}", context.split(" in ").next().unwrap_or(""));
                    // SAFETY: valid elements of this automation object.
                    let same = last.as_ref().is_some_and(|(before, _)| unsafe { automation.CompareElements(before, &element) }.is_ok_and(|b| b.as_bool()));
                    if !same || last.as_ref().is_some_and(|(_, before)| *before != said) {
                        let reading = if password { "password field: not read".into() } else { reading.map_or_else(|| "-".into(), |r| format!("{r:?}")) };
                        lines.write(&[
                            app,
                            framework,
                            control,
                            class,
                            password.to_string(),
                            patterns,
                            readable.to_string(),
                            read_ms.to_string(),
                            reading,
                            context,
                        ]);
                    }
                    last = Some((element, said));
                    nothing_said = false;
                }
            }
            std::thread::sleep(EVERY);
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod platform {
    use std::time::Instant;

    use voltip_focus::FocusPlatform;

    use super::{EVERY, Lines};

    pub fn run(lines: &mut Lines, until: Instant) {
        #[cfg(target_os = "macos")]
        let reader = voltip_focus::macos::open();
        #[cfg(target_os = "linux")]
        let reader = voltip_focus::linux::open();
        match reader {
            Ok(mut reader) => follow(&mut reader, lines, until),
            // On Linux: no accessibility bus, or accessibility switched off (docs/dictation.md §28.4).
            Err(why) => println!("focus_read_probe: the reader cannot read this desktop: {why}"),
        }
    }

    /// A line each time another field has the focus, or its reading starts or stops answering.
    fn follow<P: FocusPlatform>(reader: &mut P, lines: &mut Lines, until: Instant) {
        let blank = || "-".to_owned();
        let mut last: Option<(P::Control, String)> = None;
        let mut nothing_said = false;
        while Instant::now() < until {
            match reader.focused() {
                Err(why) => {
                    if !nothing_said {
                        lines.write(&[blank(), blank(), blank(), blank(), blank(), blank(), format!("nothing focused ({why:?})"), blank(), blank(), blank()]);
                        nothing_said = true;
                    }
                    last = None;
                }
                Ok(field) => {
                    let started = Instant::now();
                    let reading = reader.text(&field);
                    let read_ms = started.elapsed().as_millis();
                    let context = reader.context(&field, None).map_or_else(blank, |c| format!("{c:?}"));
                    let readable = reading.is_some();
                    let said = format!("{readable}|{}|{context}", reading.as_ref().map_or_else(String::new, |r| format!("{r:?}")));
                    let same = last.as_ref().is_some_and(|(before, _)| reader.same(before, &field));
                    if !same || last.as_ref().is_some_and(|(_, was)| *was != said) {
                        let reading = reading.map_or_else(blank, |r| format!("{r:?}"));
                        lines.write(&[blank(), blank(), blank(), blank(), blank(), blank(), readable.to_string(), read_ms.to_string(), reading, context]);
                    }
                    last = Some((field, said));
                    nothing_said = false;
                }
            }
            std::thread::sleep(EVERY);
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
mod platform {
    pub fn run(_lines: &mut super::Lines, _until: std::time::Instant) {
        println!("focus_read_probe: this platform has no reader");
    }
}
