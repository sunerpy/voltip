//! Running the external paste tools: `PATH` lookup, and a subprocess runner with null stdio, a
//! bounded stderr capture and a deadline, so a hung `ydotool` (no `ydotoold`) or a `wtype` waiting
//! on a compositor that never answers cannot stall a dictation. Unix only; tested with fake tools
//! written into a temporary `PATH` directory.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::toolchain::CommandLine;

/// Deadline for one `--help` probe.
pub const HELP_TIMEOUT: Duration = Duration::from_secs(3);
/// Deadline for a paste chord (the tool exits as soon as the events are queued).
pub const CHORD_TIMEOUT: Duration = Duration::from_secs(5);
/// Per-character allowance on top of [`CHORD_TIMEOUT`] when a tool types the text itself.
pub const TYPING_PER_CHAR: Duration = Duration::from_millis(25);
/// Upper bound for a typing run, whatever the length.
pub const TYPING_MAX: Duration = Duration::from_secs(60);
/// How much of stderr is kept for the error message.
const STDERR_KEEP: usize = 512;

/// Deadline for a tool that types `chars` characters itself.
pub fn typing_timeout(chars: usize) -> Duration {
    (CHORD_TIMEOUT + TYPING_PER_CHAR.saturating_mul(u32::try_from(chars).unwrap_or(u32::MAX))).min(TYPING_MAX)
}

/// First executable named `binary` on the `PATH` string (`:`-separated), what `which(1)` prints.
pub fn which_in(path: Option<&str>, binary: &str) -> Option<PathBuf> {
    if binary.is_empty() || binary.contains('/') {
        return None;
    }
    path?.split(':').filter(|d| !d.is_empty()).map(|d| Path::new(d).join(binary)).find(|candidate| is_executable(candidate))
}

/// [`which_in`] against this process's `PATH`.
pub fn which(binary: &str) -> Option<PathBuf> {
    which_in(std::env::var("PATH").ok().as_deref(), binary)
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// What happened to a subprocess.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunError {
    /// The program could not be started (not on `PATH`, not executable).
    Spawn(String),
    /// It ran past the deadline and was killed.
    Timeout(Duration),
    /// It exited non-zero (or by signal): `exited with exit status: 1: <stderr tail>`.
    Exit(String),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(e) => write!(f, "could not start: {e}"),
            Self::Timeout(d) => write!(f, "no exit within {} ms; killed", d.as_millis()),
            Self::Exit(e) => f.write_str(e),
        }
    }
}

/// Run `line` to completion within `timeout`. stdin gets `line.stdin` (or is closed), stdout is
/// discarded, stderr is captured (tail) for the error message. Success is exit status 0 with no
/// output expected.
pub fn run(line: &CommandLine, timeout: Duration) -> Result<(), RunError> {
    let mut command = Command::new(&line.program);
    command.args(&line.args).stdin(if line.stdin.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|e| RunError::Spawn(e.to_string()))?;
    if let (Some(input), Some(mut stdin)) = (&line.stdin, child.stdin.take()) {
        // A tool that exits before reading (usage error) closes the pipe; that is its exit status's
        // story, not ours.
        let _ = stdin.write_all(input.as_bytes());
        drop(stdin);
    }
    let stderr = child.stderr.take();
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RunError::Timeout(timeout));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(e) => return Err(RunError::Spawn(e.to_string())),
        }
    };
    let tail = stderr.map(|mut s| {
        let mut buf = Vec::new();
        let _ = s.read_to_end(&mut buf);
        tail_of(&String::from_utf8_lossy(&buf))
    });
    if status.success() {
        return Ok(());
    }
    let detail = tail.filter(|t| !t.is_empty()).map(|t| format!(": {t}")).unwrap_or_default();
    // No program name here: the caller prefixes the tool name, and `line.program` is an absolute
    // path after the PATH lookup, which would only add noise to the injection note.
    Err(RunError::Exit(format!("exited with {status}{detail}")))
}

/// Run `program args…` and return stdout and stderr together (for `ydotool --help`, which prints
/// to either depending on the version). `None` when it could not start or did not exit in time.
pub fn capture(program: &str, args: &[&str], timeout: Duration) -> Option<String> {
    let mut child = Command::new(program).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().ok()?;
    let deadline = Instant::now() + timeout;
    let (mut out, mut err) = (child.stdout.take(), child.stderr.take());
    // Small outputs: read after exit is fine, and neither pipe can fill 64 KiB with a usage text.
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(_) => return None,
        }
    }
    let mut text = String::new();
    let mut append = |pipe: &mut dyn std::io::Read| {
        let mut buf = Vec::new();
        let _ = pipe.read_to_end(&mut buf);
        text.push_str(&String::from_utf8_lossy(&buf));
        text.push('\n');
    };
    if let Some(pipe) = out.as_mut() {
        append(pipe);
    }
    if let Some(pipe) = err.as_mut() {
        append(pipe);
    }
    Some(text)
}

fn tail_of(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.len() <= STDERR_KEEP {
        return trimmed.to_string();
    }
    let start = trimmed.len() - STDERR_KEEP;
    let start = trimmed.char_indices().map(|(i, _)| i).find(|&i| i >= start).unwrap_or(start);
    format!("…{}", &trimmed[start..])
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Serialises the tests that write fake executables or spawn processes. A `fork` in one test
    /// thread copies every open descriptor of the process, including the write descriptor of a
    /// script another test thread is still writing; `exec` of that script then fails with
    /// `ETXTBSY` until the forked child has exec'd itself (rust-lang/rust#114554). Holding this
    /// for the whole test keeps writes and forks from overlapping.
    pub fn spawn_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// A directory of fake tools on a private `PATH`.
    pub struct FakeTools {
        pub dir: tempfile::TempDir,
    }

    impl FakeTools {
        pub fn new() -> Self {
            Self { dir: tempfile::tempdir().unwrap() }
        }

        pub fn path(&self) -> String {
            self.dir.path().display().to_string()
        }

        /// Install a shell script named `name` on the fake `PATH`.
        pub fn script(&self, name: &str, body: &str) -> PathBuf {
            use std::os::unix::fs::PermissionsExt as _;
            let path = self.dir.path().join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        }

        /// Path of a file the scripts can write into.
        pub fn log(&self, name: &str) -> PathBuf {
            self.dir.path().join(name)
        }
    }

    fn line(program: &Path, args: &[&str], stdin: Option<&str>) -> CommandLine {
        CommandLine { program: program.display().to_string(), args: args.iter().map(|a| a.to_string()).collect(), stdin: stdin.map(str::to_string) }
    }

    #[test]
    fn which_walks_the_path_in_order() {
        let _spawn = spawn_lock();
        let tools = FakeTools::new();
        let first = tools.script("wtype", "exit 0");
        let other = tempfile::tempdir().unwrap();
        std::fs::write(other.path().join("wtype"), "not executable").unwrap();
        let path = format!("{}:{}:/nonexistent", other.path().display(), tools.path());
        assert_eq!(which_in(Some(&path), "wtype"), Some(first), "the non-executable file is skipped");
        assert_eq!(which_in(Some(&path), "ydotool"), None);
        assert_eq!(which_in(Some(""), "wtype"), None);
        assert_eq!(which_in(None, "wtype"), None);
        assert_eq!(which_in(Some(&path), ""), None);
        assert_eq!(which_in(Some(&path), "bin/wtype"), None, "no path components");
        assert!(which("sh").is_some(), "the real PATH has a shell");
        assert!(which("voltip-no-such-tool-xyz").is_none());
    }

    #[test]
    fn run_reports_success_exit_status_and_stderr_tail() {
        let _spawn = spawn_lock();
        let tools = FakeTools::new();
        let log = tools.log("calls.log");
        tools.script("wtype", &format!("printf '%s\\n' \"$*\" >> '{}'", log.display()));
        let wtype = tools.dir.path().join("wtype");
        // The fake is addressed by absolute path; the real chain resolves the name through PATH.
        assert_eq!(run(&line(&wtype, &["-M", "ctrl", "-k", "v"], None), CHORD_TIMEOUT), Ok(()));
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "-M ctrl -k v\n");

        let failing = tools.script("fail", "echo 'Compositor does not support the virtual keyboard protocol' >&2; exit 1");
        let err = run(&line(&failing, &[], None), CHORD_TIMEOUT).unwrap_err();
        match &err {
            RunError::Exit(m) => {
                assert!(m.starts_with("exited with exit status: 1: "), "{m}");
                assert!(m.contains("virtual keyboard protocol"), "{m}");
            }
            other => panic!("{other:?}"),
        }
        assert!(err.to_string().contains("virtual keyboard"));

        let missing = line(Path::new("/nonexistent/voltip-tool"), &[], None);
        assert!(matches!(run(&missing, CHORD_TIMEOUT), Err(RunError::Spawn(_))));
        assert!(RunError::Spawn("x".into()).to_string().starts_with("could not start"));
    }

    #[test]
    fn run_feeds_stdin_and_enforces_the_deadline() {
        let _spawn = spawn_lock();
        let tools = FakeTools::new();
        let log = tools.log("stdin.log");
        let dotool = tools.script("dotool", &format!("cat > '{}'", log.display()));
        let l = line(&dotool, &[], Some("key ctrl+v\n"));
        assert_eq!(run(&l, CHORD_TIMEOUT), Ok(()));
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "key ctrl+v\n");

        let hang = tools.script("hang", "sleep 5");
        let started = Instant::now();
        let err = run(&line(&hang, &[], None), Duration::from_millis(150)).unwrap_err();
        assert_eq!(err, RunError::Timeout(Duration::from_millis(150)));
        assert!(started.elapsed() < Duration::from_secs(3), "killed, not waited for");
        assert!(err.to_string().contains("150 ms"));
    }

    #[test]
    fn capture_joins_stdout_and_stderr_and_times_out() {
        let _spawn = spawn_lock();
        let tools = FakeTools::new();
        let help = tools.script("ydotool", "echo 'notice: daemon unavailable' >&2; echo 'Usage: ydotool <cmd> <args>'; echo '  recorder'");
        let text = capture(&help.display().to_string(), &["--help"], HELP_TIMEOUT).unwrap();
        assert!(text.contains("Usage: ydotool") && text.contains("recorder") && text.contains("daemon unavailable"), "{text}");
        let hang = tools.script("hang", "sleep 5");
        assert_eq!(capture(&hang.display().to_string(), &[], Duration::from_millis(100)), None);
        assert_eq!(capture("/nonexistent/voltip-tool", &[], HELP_TIMEOUT), None);
    }

    #[test]
    fn typing_timeout_scales_and_caps() {
        assert_eq!(typing_timeout(0), CHORD_TIMEOUT);
        assert_eq!(typing_timeout(40), CHORD_TIMEOUT + Duration::from_secs(1));
        assert_eq!(typing_timeout(1_000_000), TYPING_MAX);
    }

    #[test]
    fn stderr_tail_is_bounded_on_char_boundaries() {
        assert_eq!(tail_of("  short  "), "short");
        let long = "界".repeat(400); // 1200 bytes
        let tail = tail_of(&long);
        assert!(tail.starts_with('…'));
        assert!(tail.len() <= STDERR_KEEP + '…'.len_utf8() + 3);
        assert!(tail.chars().skip(1).all(|c| c == '界'));
    }
}
