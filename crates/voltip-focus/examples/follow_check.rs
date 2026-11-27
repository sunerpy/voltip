//! 自动跟踪输入框里的修改 in real applications (docs/dictation.md §35.10, 复测). With an
//! application in front and the caret on an empty line of its text field, it pastes
//! 「今天测试一下千问四Max的推理能力」 the way a take is pasted, keeps the field the way the follow
//! keeps it (`settle_and_keep`), changes 「千问四」 into 「Qwen 4 」 with the keyboard (the caret
//! moved left, the three characters selected, the replacement pasted over them), reads the kept
//! field until the change shows (at most 3 s), and deletes the line again (Shift+Home, Delete).
//! It writes one CSV row, no text in it:
//!
//! `app,written,kept,pasted_read,edited_read,edit_ms,focus_moved,error`
//!
//! - `written`, `kept`: the insert check's verdict was `written`, and the field was kept;
//! - `pasted_read`: a reading of the kept field held the sentence as pasted;
//! - `edited_read`, `edit_ms`: a reading held the changed sentence, and how long after the change;
//! - `focus_moved`: with `--then-tab`, Tab moved the focus to another field (a page with two) and
//!   the kept field answered `ControlChanged`.
//!
//! It types into whatever is in front: run it on a desktop nobody is using, in the user's session
//! (the probe of 2026-11-26 ran as a scheduled task there), and only in a field whose recipient you
//! checked (in a chat application: its 文件传输助手 or your own chat; it never presses Enter).
//!
//! `cargo run -p voltip-focus --example follow_check -- --app-id notepad [--then-tab] [--out follow-check.csv]`
//!
//! Windows only: the keys go out through `SendInput`.

// The keys are Win32 FFI (`SendInput`).
#![allow(unsafe_code, clippy::unwrap_used, clippy::expect_used)]

/// What the take "pastes".
#[cfg(windows)]
const SENTENCE: &str = "今天测试一下千问四Max的推理能力";
/// The sentence with the user's change.
#[cfg(windows)]
const CHANGED: &str = "今天测试一下Qwen 4 Max的推理能力";
/// What replaces 「千问四」.
#[cfg(windows)]
const REPLACEMENT: &str = "Qwen 4 ";
/// Characters after 「千问四」: the caret moves left over them.
#[cfg(windows)]
const AFTER_CHANGE: usize = 8;

#[cfg(windows)]
fn main() {
    use std::io::Write as _;
    use std::time::{Duration, Instant};

    use voltip_focus::{Expected, FocusContext, Follow, Reader, UnconfirmedWhy};
    use voltip_inject::{Injector as _, PasteOptions};
    use voltip_platform::insert_check::rules::InsertCheck;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY, VK_DELETE, VK_END, VK_HOME, VK_LEFT,
        VK_SHIFT, VK_TAB,
    };

    /// How long a paste is watched: well over `CONFIRM_WITHIN`.
    const WATCH: Duration = Duration::from_secs(3);
    /// How long a change may take to show in the field.
    const SHOW: Duration = Duration::from_secs(3);

    struct Args {
        app_id: String,
        then_tab: bool,
        out: String,
    }

    fn args() -> Args {
        let mut parsed = Args { app_id: String::new(), then_tab: false, out: "follow-check.csv".into() };
        let mut it = std::env::args().skip(1);
        while let Some(arg) = it.next() {
            let mut value = || it.next().unwrap_or_else(|| panic!("{arg} needs a value"));
            match arg.as_str() {
                "--app-id" => parsed.app_id = value(),
                "--then-tab" => parsed.then_tab = true,
                "--out" => parsed.out = value(),
                other => panic!("unknown argument {other} (--app-id <id>, --then-tab, --out <file>)"),
            }
        }
        assert!(!parsed.app_id.is_empty(), "--app-id <the application in front, as Voltip names it>");
        parsed
    }

    /// Press (`up` false) or release (`up` true) `vks`, in order, as a keyboard would.
    fn keys(vks: &[VIRTUAL_KEY], up: bool) {
        // The arrows, Home, End and Delete are extended keys: sent without the flag they are the
        // numeric keypad's, which with Num Lock on lets go of Shift (no selection) or types digits.
        let extended = |vk: VIRTUAL_KEY| if [VK_LEFT, VK_HOME, VK_END, VK_DELETE].contains(&vk) { KEYEVENTF_EXTENDEDKEY } else { 0 };
        let up = if up { KEYEVENTF_KEYUP } else { 0 };
        let inputs: Vec<INPUT> = vks
            .iter()
            .map(|&vk| INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: up | extended(vk), time: 0, dwExtraInfo: 0 } },
            })
            .collect();
        // SAFETY: an array of INPUTs that outlives the call.
        let sent = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
        assert_eq!(sent as usize, inputs.len(), "the keys {vks:?} (up: {up})");
    }

    /// Press and release `vks` together (the first ones held while the last is clicked).
    fn chord(vks: &[VIRTUAL_KEY]) {
        keys(vks, false);
        let mut up = vks.to_vec();
        up.reverse();
        keys(&up, true);
        std::thread::sleep(Duration::from_millis(30));
    }

    /// The kept field's paragraph, whole.
    fn paragraph(c: &FocusContext) -> String {
        format!("{}{}{}", c.before, c.selected, c.after)
    }

    /// Read the kept field until `wanted` holds of a reading, for at most `within`; how long it took.
    fn until(follow: &Follow, within: Duration, wanted: impl Fn(&Result<FocusContext, UnconfirmedWhy>) -> bool) -> Option<Duration> {
        let started = Instant::now();
        loop {
            if wanted(&follow.context(None)) {
                return Some(started.elapsed());
            }
            if started.elapsed() >= within {
                return None;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// What one run found.
    #[derive(Default)]
    struct Row {
        written: bool,
        kept: bool,
        pasted_read: bool,
        edited_read: bool,
        edit_ms: Option<u128>,
        focus_moved: Option<bool>,
        error: String,
    }

    let args = args();
    let reader = Reader::system().expect("no reader on this platform");
    let paste = voltip_inject::system_paste_injector(PasteOptions::for_os("windows"));
    let mut row = Row::default();
    'run: {
        let watch = match reader.watch(&args.app_id, &Expected::of(SENTENCE)) {
            Ok(watch) => watch,
            Err(why) => {
                row.error = format!("watch: {why:?}");
                break 'run;
            }
        };
        if let Err(e) = paste.inject(SENTENCE) {
            row.error = format!("paste: {e}");
            break 'run;
        }
        let (settled, follow) = watch.settle_and_keep(WATCH);
        row.written = settled.check == InsertCheck::Written;
        let Some(follow) = follow else {
            row.error = format!("not kept: {settled:?}");
            break 'run;
        };
        row.kept = true;
        row.pasted_read = until(&follow, SHOW, |r| r.as_ref().is_ok_and(|c| paragraph(c).contains(SENTENCE))).is_some();
        // 「千问四」 selected from the end of the line, and the replacement pasted over it.
        for _ in 0..AFTER_CHANGE {
            chord(&[VK_LEFT]);
        }
        for _ in 0.."千问四".chars().count() {
            chord(&[VK_SHIFT, VK_LEFT]);
        }
        let changed_at = Instant::now();
        if let Err(e) = paste.inject(REPLACEMENT) {
            row.error = format!("the change: {e}");
            break 'run;
        }
        let shown = until(&follow, SHOW, |r| r.as_ref().is_ok_and(|c| paragraph(c).contains(CHANGED)));
        row.edited_read = shown.is_some();
        row.edit_ms = shown.map(|_| changed_at.elapsed().as_millis());
        if args.then_tab {
            chord(&[VK_TAB]);
            row.focus_moved = Some(until(&follow, SHOW, |r| *r == Err(UnconfirmedWhy::ControlChanged)).is_some());
            // Back to the field to delete the line.
            chord(&[VK_SHIFT, VK_TAB]);
        }
        drop(follow);
    }
    // The line goes again, whatever happened: from its end, Shift+Home and Delete.
    chord(&[VK_END]);
    chord(&[VK_SHIFT, VK_HOME]);
    chord(&[VK_DELETE]);
    let line = format!(
        "{},{},{},{},{},{},{},{}",
        args.app_id,
        row.written,
        row.kept,
        row.pasted_read,
        row.edited_read,
        row.edit_ms.map_or_else(String::new, |ms| ms.to_string()),
        row.focus_moved.map_or_else(String::new, |m| m.to_string()),
        row.error.replace(',', ";"),
    );
    println!("app,written,kept,pasted_read,edited_read,edit_ms,focus_moved,error\n{line}");
    let mut out = std::fs::File::create(&args.out).expect("--out");
    writeln!(out, "app,written,kept,pasted_read,edited_read,edit_ms,focus_moved,error\n{line}").unwrap();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("follow_check: Windows only (the keys go out through SendInput); the CI tests in tests/focus_*.rs cover macOS and Linux");
}
