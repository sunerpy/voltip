//! The Linux paste chain at run time (docs/dictation.md §14): judge the session, walk
//! [`crate::toolchain::candidates`] in order, run the first tool that is available, fall through
//! to the next one when a chord tool fails before anything can have reached the application. The
//! decisions are the pure functions in [`crate::toolchain`]; this module owns the three effects
//! (PATH lookup, running a subprocess, opening an enigo connection) behind traits, so the port
//! itself is tested with fake tools on a private `PATH`.

use std::sync::Arc;
use std::time::Duration;

use crate::injector::{Delivered, DeliveryError, KeystrokePort};
use crate::platform::{EnigoBackend, EnigoError, SystemKeys};
use crate::process::{self, CHORD_TIMEOUT, HELP_TIMEOUT, RunError};
use crate::session::{Desktop, Session, SessionKind};
use crate::toolchain::{Chord, CommandLine, Delivery, Modifier, Selection, ToolProbe, TypingTool, candidates, install_hint, probe_candidate, select};
use crate::{FallbackCode, InjectNote};

/// Runs one external tool to completion. [`SystemRunner`] is the real one.
pub trait ToolRunner: Send + Sync {
    /// Run `line` within `timeout` (see [`process::run`]).
    fn run(&self, line: &CommandLine, timeout: Duration) -> Result<(), RunError>;
}

/// Presses a chord through enigo. [`SystemRunner`] is the real one.
pub trait EnigoPort: Send + Sync {
    /// Whether the connection for `backend` opens right now (nothing is sent).
    fn available(&self, backend: EnigoBackend) -> bool;
    /// Press `chord` through `backend`.
    fn press(&self, backend: EnigoBackend, chord: Chord) -> Result<(), EnigoError>;
    /// Press the selection copy `chord` through `backend` after releasing `held`
    /// ([`crate::platform::copy_steps`], docs/dictation.md §19).
    fn copy(&self, backend: EnigoBackend, chord: Chord, held: &[Modifier]) -> Result<(), EnigoError>;
}

/// The real effects: subprocesses and enigo.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemRunner;

impl ToolRunner for SystemRunner {
    fn run(&self, line: &CommandLine, timeout: Duration) -> Result<(), RunError> {
        process::run(line, timeout)
    }
}

impl EnigoPort for SystemRunner {
    fn available(&self, backend: EnigoBackend) -> bool {
        SystemKeys::with_backend(backend).connect().is_ok()
    }

    fn press(&self, backend: EnigoBackend, chord: Chord) -> Result<(), EnigoError> {
        SystemKeys::with_backend(backend).press(chord)
    }

    fn copy(&self, backend: EnigoBackend, chord: Chord, held: &[Modifier]) -> Result<(), EnigoError> {
        SystemKeys::with_backend(backend).copy(chord, held)
    }
}

/// [`ToolProbe`] over a `PATH` string and an [`EnigoPort`]; `ydotool --help` is run from the
/// resolved path.
pub struct SystemProbe {
    path: Option<String>,
    enigo: Arc<dyn EnigoPort>,
}

impl SystemProbe {
    /// This process's `PATH` and the real enigo.
    pub fn system() -> Self {
        Self::new(std::env::var("PATH").ok(), Arc::new(SystemRunner))
    }

    /// An explicit `PATH` string and enigo port (tests).
    pub fn new(path: Option<String>, enigo: Arc<dyn EnigoPort>) -> Self {
        Self { path, enigo }
    }
}

impl ToolProbe for SystemProbe {
    fn which(&self, binary: &str) -> Option<std::path::PathBuf> {
        process::which_in(self.path.as_deref(), binary)
    }

    fn ydotool_help(&self) -> Option<String> {
        let ydotool = process::which_in(self.path.as_deref(), "ydotool")?;
        process::capture(&ydotool.display().to_string(), &["--help"], HELP_TIMEOUT)
    }

    fn enigo_available(&self, tool: TypingTool) -> bool {
        enigo_backend(tool).is_some_and(|backend| self.enigo.available(backend))
    }
}

/// The enigo connection behind an in-process candidate.
pub fn enigo_backend(tool: TypingTool) -> Option<EnigoBackend> {
    match tool {
        TypingTool::EnigoX11 => Some(EnigoBackend::X11),
        TypingTool::EnigoWayland => Some(EnigoBackend::Wayland),
        _ => None,
    }
}

/// What one tool did with the paste.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Attempt {
    /// The chord was pressed (or the text typed).
    Done(Delivered),
    /// Nothing ran: the binary could not be started, or the connection vanished since the probe.
    NotStarted(String),
    /// A chord tool ran and failed. Chord tools fail before sending (missing protocol, no
    /// `ydotoold`, no uinput permission), so the next tool may try.
    Retry(String),
    /// A failure after which a second tool could double the text: a typing tool (it may have
    /// typed part of it) or a tool killed at its deadline. The chain stops here.
    Stop(String),
}

/// How a failed run of `tool` continues the chain: [`Attempt::Retry`] for a chord tool's non-zero
/// exit, [`Attempt::Stop`] for timeouts and typing tools, [`Attempt::NotStarted`] when it never ran.
pub fn classify(tool: TypingTool, error: &RunError) -> Attempt {
    let reason = format!("{}: {error}", tool.name());
    match error {
        RunError::Spawn(_) => Attempt::NotStarted(reason),
        RunError::Timeout(_) => Attempt::Stop(reason),
        RunError::Exit(_) if tool.delivery() == Delivery::Chord => Attempt::Retry(reason),
        RunError::Exit(_) => Attempt::Stop(reason),
    }
}

/// The [`KeystrokePort`] of a Linux session. The chain is walked per paste (the user may install
/// `wtype` while the app runs): unavailable tools are skipped, a failing chord tool hands over to
/// the next one, and an empty chain is [`DeliveryError::Unavailable`] (the text stays on the
/// clipboard).
pub struct ToolchainKeys {
    session: Session,
    probe: Arc<dyn ToolProbe + Send + Sync>,
    runner: Arc<dyn ToolRunner>,
    enigo: Arc<dyn EnigoPort>,
}

impl ToolchainKeys {
    /// The real chain for `session`.
    pub fn system(session: Session) -> Self {
        let runner = Arc::new(SystemRunner);
        Self::new(session, Arc::new(SystemProbe::system()), runner.clone(), runner)
    }

    /// Explicit effects (tests).
    pub fn new(session: Session, probe: Arc<dyn ToolProbe + Send + Sync>, runner: Arc<dyn ToolRunner>, enigo: Arc<dyn EnigoPort>) -> Self {
        Self { session, probe, runner, enigo }
    }

    /// The session this chain was built for.
    pub fn session(&self) -> Session {
        self.session
    }

    /// The first tool the chain would try right now (what the shell logs at start-up).
    pub fn selection(&self) -> Selection {
        select(self.session, self.probe.as_ref())
    }

    /// Try one available tool.
    pub fn attempt(&self, tool: TypingTool, chord: Chord, text: &str) -> Attempt {
        self.attempt_with(tool, chord, Some(text), &[])
    }

    /// Try one available tool for the selection copy (docs/dictation.md §19): no text, so a typing
    /// tool is not started; enigo releases `held` first, `xdotool` clears modifiers itself.
    pub fn attempt_copy(&self, tool: TypingTool, chord: Chord, held: &[Modifier]) -> Attempt {
        self.attempt_with(tool, chord, None, held)
    }

    /// `text` is the paste's text, `None` for a bare chord (the copy).
    fn attempt_with(&self, tool: TypingTool, chord: Chord, text: Option<&str>, held: &[Modifier]) -> Attempt {
        if let Some(backend) = enigo_backend(tool) {
            let outcome = match text {
                Some(_) => self.enigo.press(backend, chord),
                None => self.enigo.copy(backend, chord, held),
            };
            return match outcome {
                Ok(()) => Attempt::Done(Delivered { tool: tool.name().to_string(), delivery: Delivery::Chord }),
                Err(e @ (EnigoError::Connect(_) | EnigoError::NoPermission(_))) => Attempt::NotStarted(format!("{}: {e}", tool.name())),
                Err(e @ EnigoError::Input(_)) => Attempt::Retry(format!("{}: {e}", tool.name())),
            };
        }
        if text.is_none() && tool.delivery() == Delivery::Type {
            return Attempt::NotStarted(format!("{}: {TYPES_ONLY}", tool.name()));
        }
        let (Some(mut line), Some(binary)) = (tool.command(chord, text.unwrap_or_default()), tool.binary()) else {
            return Attempt::NotStarted(format!("{}: no command line", tool.name()));
        };
        // Run the binary the probe found, so the result does not depend on this process's PATH.
        match self.probe.which(binary) {
            Some(found) => line.program = found.display().to_string(),
            None => return Attempt::NotStarted(format!("{}: vanished from PATH", tool.name())),
        }
        let timeout = match (tool.delivery(), text) {
            (Delivery::Type, Some(text)) => process::typing_timeout(text.chars().count()),
            _ => CHORD_TIMEOUT,
        };
        match self.runner.run(&line, timeout) {
            Ok(()) => Attempt::Done(Delivered { tool: tool.name().to_string(), delivery: tool.delivery() }),
            Err(e) => classify(tool, &e),
        }
    }

    /// Walk the chain for a paste (`text`) or a bare chord (`None`, the copy): unavailable tools
    /// are skipped, a failing chord tool hands over, a stop ends the walk.
    fn walk(&self, chord: Chord, text: Option<&str>, held: &[Modifier]) -> Result<Delivered, DeliveryError> {
        let what = if text.is_some() { "paste" } else { "copy" };
        let mut skipped: Vec<String> = Vec::new();
        let mut failures: Vec<String> = Vec::new();
        for candidate in candidates(self.session) {
            if text.is_none() && candidate.delivery() == Delivery::Type {
                skipped.push(format!("{}: {TYPES_ONLY}", candidate.name()));
                continue;
            }
            let tool = match probe_candidate(candidate, self.probe.as_ref()) {
                Ok(tool) => tool,
                Err(s) => {
                    skipped.push(format!("{}: {}", s.tool.name(), s.reason));
                    continue;
                }
            };
            match self.attempt_with(tool, chord, text, held) {
                Attempt::Done(delivered) => {
                    if !failures.is_empty() {
                        tracing::info!(session = %self.session, tool = %tool, failed_before = ?failures, "{what} delivered by a later tool in the chain");
                    }
                    return Ok(delivered);
                }
                Attempt::NotStarted(reason) => skipped.push(reason),
                Attempt::Retry(reason) => {
                    tracing::warn!(session = %self.session, %reason, "{what} tool failed; trying the next one");
                    failures.push(reason);
                }
                Attempt::Stop(reason) => {
                    failures.push(reason);
                    return Err(DeliveryError::Failed(failures.join("; ")));
                }
            }
        }
        if failures.is_empty() {
            Err(DeliveryError::Unavailable(InjectNote::new(
                FallbackCode::NoTool,
                format!("no {what} tool on {} ({}); {}", self.session, skipped.join(", "), install_hint(self.session)),
            )))
        } else {
            Err(DeliveryError::Failed(failures.join("; ")))
        }
    }
}

/// Why a typing tool (`kwtype`) is passed over for the copy: it can only type text.
const TYPES_ONLY: &str = "types text, cannot press a chord";

impl KeystrokePort for ToolchainKeys {
    fn deliver(&self, chord: Chord, text: &str) -> Result<Delivered, DeliveryError> {
        self.walk(chord, Some(text), &[])
    }

    fn press_chord(&self, chord: Chord, held: &[Modifier]) -> Result<Delivered, DeliveryError> {
        self.walk(chord, None, held)
    }
}

/// The session this process runs in, or `X11 · other` when neither display variable is set (the
/// clipboard then fails with `NoDisplay` before any keystroke is tried).
pub fn detected_session() -> Session {
    Session::detect().unwrap_or(Session { kind: SessionKind::X11, desktop: Desktop::Other })
}

/// [`ToolchainKeys::system`] for [`detected_session`].
pub fn system_toolchain_keys() -> ToolchainKeys {
    ToolchainKeys::system(detected_session())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::injector::tests::MemoryClipboard;
    use crate::injector::{ClipboardPasteInjector, KeystrokePort, PasteOptions};
    use crate::process::tests::{FakeTools, spawn_lock};
    use crate::toolchain::{PasteMethod, YdotoolSyntax};
    use crate::{ClipboardPort, InjectError, Injector, Via};

    /// An enigo that answers as told and records presses (and copies, with their held modifiers).
    struct FakeEnigo {
        available: Vec<EnigoBackend>,
        press: Result<(), EnigoError>,
        pressed: Mutex<Vec<(EnigoBackend, Chord)>>,
        copied: Mutex<Vec<(EnigoBackend, Chord, Vec<Modifier>)>>,
    }

    impl FakeEnigo {
        fn none() -> Arc<Self> {
            Self::with(&[], Ok(()))
        }
        fn with(available: &[EnigoBackend], press: Result<(), EnigoError>) -> Arc<Self> {
            Arc::new(Self { available: available.to_vec(), press, pressed: Mutex::new(vec![]), copied: Mutex::new(vec![]) })
        }
    }

    impl EnigoPort for FakeEnigo {
        fn available(&self, backend: EnigoBackend) -> bool {
            self.available.contains(&backend)
        }
        fn press(&self, backend: EnigoBackend, chord: Chord) -> Result<(), EnigoError> {
            self.pressed.lock().unwrap().push((backend, chord));
            self.press.clone()
        }
        fn copy(&self, backend: EnigoBackend, chord: Chord, held: &[Modifier]) -> Result<(), EnigoError> {
            self.copied.lock().unwrap().push((backend, chord, held.to_vec()));
            self.press.clone()
        }
    }

    fn make_keys(tools: &FakeTools, session: Session, enigo: Arc<FakeEnigo>) -> ToolchainKeys {
        let probe = Arc::new(SystemProbe::new(Some(tools.path()), enigo.clone()));
        ToolchainKeys::new(session, probe, Arc::new(SystemRunner), enigo)
    }

    fn injector_over(clipboard: &Arc<MemoryClipboard>, keys: ToolchainKeys, method: PasteMethod) -> ClipboardPasteInjector {
        let options = PasteOptions { method, paste_delay: Duration::from_millis(1), paste_delay_after: Duration::from_millis(20), ..PasteOptions::default() };
        ClipboardPasteInjector::with_ports(clipboard.clone() as Arc<dyn ClipboardPort>, Arc::new(keys), method.chord_for("linux"), options)
    }

    fn wait_for_clipboard(clipboard: &MemoryClipboard, expected: &str) {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while clipboard.current().as_deref() != Some(expected) {
            assert!(std::time::Instant::now() < deadline, "clipboard is {:?}, want {expected:?}", clipboard.current());
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Records its arguments into `<name>.log` and exits with `code`.
    fn recording_tool(tools: &FakeTools, name: &str, code: i32) -> std::path::PathBuf {
        let log = tools.log(&format!("{name}.log"));
        tools.script(name, &format!("printf '%s\\n' \"$*\" >> '{}'; exit {code}", log.display()));
        log
    }

    const KDE_WAYLAND: Session = Session { kind: SessionKind::Wayland, desktop: Desktop::Kde };
    const WLROOTS: Session = Session { kind: SessionKind::Wayland, desktop: Desktop::Wlroots };
    const X11: Session = Session { kind: SessionKind::X11, desktop: Desktop::Other };

    #[test]
    fn wtype_on_a_fake_path_presses_the_chord() {
        let _spawn = spawn_lock();
        let tools = FakeTools::new();
        let log = recording_tool(&tools, "wtype", 0);
        let keys = make_keys(&tools, KDE_WAYLAND, FakeEnigo::none());
        assert_eq!(keys.session(), KDE_WAYLAND);
        assert_eq!(keys.selection().tool, Some(TypingTool::Wtype));
        let delivered = keys.deliver(PasteMethod::CtrlV.chord_for("linux"), "hello").unwrap();
        assert_eq!(delivered, Delivered { tool: "wtype".into(), delivery: Delivery::Chord });
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "-M ctrl -k v -m ctrl\n");
        // Through the injector: save → write → wtype → delayed restore.
        let clipboard = MemoryClipboard::holding("before");
        let injector = injector_over(&clipboard, keys, PasteMethod::CtrlShiftV);
        assert_eq!(injector.inject("你好").unwrap().via, Via::Paste);
        wait_for_clipboard(&clipboard, "before");
        assert_eq!(clipboard.writes(), vec!["你好".to_string(), "before".to_string()]);
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "-M ctrl -k v -m ctrl\n-M ctrl -M shift -k v -m shift -m ctrl\n");
    }

    #[test]
    fn dotool_gets_the_command_on_stdin_and_ydotool_syntax_is_probed() {
        let _spawn = spawn_lock();
        let tools = FakeTools::new();
        let log = tools.log("dotool.log");
        tools.script("dotool", &format!("cat >> '{}'", log.display()));
        let keys = make_keys(&tools, KDE_WAYLAND, FakeEnigo::none());
        assert_eq!(keys.deliver(PasteMethod::ShiftInsert.chord_for("linux"), "t").unwrap().tool, "dotool");
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "key shift+Insert\n");

        let tools = FakeTools::new();
        let log = tools.log("ydotool.log");
        let help_1x = "echo 'Usage: ydotool <cmd> <args>'; echo 'Use environment variable YDOTOOL_SOCKET to specify daemon socket.'";
        tools.script("ydotool", &format!("if [ \"$1\" = --help ]; then {help_1x}; exit 0; fi; printf '%s\\n' \"$*\" >> '{}'", log.display()));
        let keys = make_keys(&tools, KDE_WAYLAND, FakeEnigo::none());
        assert_eq!(keys.selection().tool, Some(TypingTool::Ydotool(YdotoolSyntax::KeyCodes)));
        assert_eq!(keys.deliver(PasteMethod::CtrlV.chord_for("linux"), "t").unwrap().tool, "ydotool");
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "key 29:1 47:1 47:0 29:0\n");

        let tools = FakeTools::new();
        let log = tools.log("ydotool.log");
        tools.script(
            "ydotool",
            &format!(
                "if [ \"$1\" = --help ]; then echo 'Usage: ydotool <cmd> <args>'; echo '  recorder' >&2; exit 0; fi; printf '%s\\n' \"$*\" >> '{}'",
                log.display()
            ),
        );
        let keys = make_keys(&tools, KDE_WAYLAND, FakeEnigo::none());
        assert_eq!(keys.deliver(PasteMethod::CtrlV.chord_for("linux"), "t").unwrap().tool, "ydotool");
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "key ctrl+v\n");
    }

    #[test]
    fn kwtype_types_the_text() {
        let _spawn = spawn_lock();
        let tools = FakeTools::new();
        let log = tools.log("kwtype.log");
        tools.script("kwtype", &format!("printf '%s\\n' \"$@\" >> '{}'", log.display()));
        let keys = make_keys(&tools, KDE_WAYLAND, FakeEnigo::none());
        let delivered = keys.deliver(PasteMethod::CtrlV.chord_for("linux"), "-leading dash 文本").unwrap();
        assert_eq!(delivered, Delivered { tool: "kwtype".into(), delivery: Delivery::Type });
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "--\n-leading dash 文本\n");
    }

    /// KWin has no `zwp_virtual_keyboard_v1`: wtype exits 1 before sending anything, and the chain
    /// moves on to the next available tool.
    #[test]
    fn failing_chord_tool_falls_through_to_the_next() {
        let _spawn = spawn_lock();
        let tools = FakeTools::new();
        tools.script("wtype", "echo 'Compositor does not support the virtual keyboard protocol' >&2; exit 1");
        let kwtype = recording_tool(&tools, "kwtype", 0);
        let keys = make_keys(&tools, KDE_WAYLAND, FakeEnigo::none());
        let delivered = keys.deliver(PasteMethod::CtrlV.chord_for("linux"), "文本").unwrap();
        assert_eq!(delivered, Delivered { tool: "kwtype".into(), delivery: Delivery::Type });
        assert_eq!(std::fs::read_to_string(&kwtype).unwrap(), "-- 文本\n");

        // X11: enigo's XTEST refuses the event → xdotool --clearmodifiers.
        let tools = FakeTools::new();
        let xdotool = recording_tool(&tools, "xdotool", 0);
        let enigo = FakeEnigo::with(&[EnigoBackend::X11], Err(EnigoError::Input("BadAccess".into())));
        let keys = make_keys(&tools, X11, enigo.clone());
        assert_eq!(keys.deliver(PasteMethod::CtrlV.chord_for("linux"), "t").unwrap().tool, "xdotool");
        assert_eq!(enigo.pressed.lock().unwrap().len(), 1);
        assert_eq!(std::fs::read_to_string(&xdotool).unwrap(), "key --clearmodifiers ctrl+v\n");
    }

    /// Stop rules: a typing tool that failed may have typed part of the text, and a tool killed at
    /// its deadline may have sent the chord — no second tool after either.
    #[test]
    fn typing_failure_and_timeout_stop_the_chain() {
        let _spawn = spawn_lock();
        let tools = FakeTools::new();
        tools.script("kwtype", "echo 'Failed to authenticate fake input protoccol within timeout' >&2; exit 1");
        let enigo = FakeEnigo::with(&[EnigoBackend::Wayland], Ok(()));
        let keys = make_keys(&tools, KDE_WAYLAND, enigo.clone());
        let err = keys.deliver(PasteMethod::CtrlV.chord_for("linux"), "t").unwrap_err();
        assert_eq!(err, DeliveryError::Failed("kwtype: exited with exit status: 1: Failed to authenticate fake input protoccol within timeout".into()));
        assert!(enigo.pressed.lock().unwrap().is_empty(), "enigo-wayland is not tried after a typing failure");

        let tools = FakeTools::new();
        tools.script("wtype", "sleep 30");
        let dotool = recording_tool(&tools, "dotool", 0);
        struct ShortRunner;
        impl ToolRunner for ShortRunner {
            fn run(&self, line: &CommandLine, _timeout: Duration) -> Result<(), RunError> {
                process::run(line, Duration::from_millis(100))
            }
        }
        let probe = Arc::new(SystemProbe::new(Some(tools.path()), FakeEnigo::none()));
        let keys = ToolchainKeys::new(WLROOTS, probe, Arc::new(ShortRunner), FakeEnigo::none());
        let err = keys.deliver(PasteMethod::CtrlV.chord_for("linux"), "t").unwrap_err();
        assert_eq!(err, DeliveryError::Failed("wtype: no exit within 100 ms; killed".into()));
        assert!(!dotool.exists(), "dotool never ran");
    }

    #[test]
    fn all_failures_are_failed_and_an_empty_chain_is_unavailable() {
        let _spawn = spawn_lock();
        // Every available tool fails: Failed with each reason, in chain order.
        let tools = FakeTools::new();
        tools.script("wtype", "echo 'Compositor does not support the virtual keyboard protocol' >&2; exit 1");
        tools.script("ydotool", "if [ \"$1\" = --help ]; then echo '  recorder'; exit 0; fi; echo 'failed to open uinput device' >&2; exit 134");
        let keys = make_keys(&tools, KDE_WAYLAND, FakeEnigo::none());
        let err = keys.deliver(PasteMethod::CtrlV.chord_for("linux"), "t").unwrap_err();
        assert_eq!(
            err,
            DeliveryError::Failed(
                "wtype: exited with exit status: 1: Compositor does not support the virtual keyboard protocol; ydotool: exited with exit status: 134: failed to open uinput device".into()
            )
        );
        // Through the injector the failure restores the previous clipboard at once and is an error.
        let clipboard = MemoryClipboard::holding("before");
        let injector = injector_over(&clipboard, keys, PasteMethod::CtrlV);
        assert!(matches!(injector.inject("ours"), Err(InjectError::Keystroke(m)) if m.starts_with("wtype: ")));
        assert_eq!(clipboard.writes(), vec!["ours".to_string(), "before".to_string()]);

        // Nothing on PATH and no enigo connection: Unavailable, naming every skipped tool and the fix.
        let tools = FakeTools::new();
        let keys = make_keys(&tools, KDE_WAYLAND, FakeEnigo::none());
        let err = keys.deliver(PasteMethod::CtrlV.chord_for("linux"), "t").unwrap_err();
        assert_eq!(
            err,
            DeliveryError::Unavailable(InjectNote::new(
                FallbackCode::NoTool,
                "no paste tool on Wayland · KDE (wtype: not on PATH, dotool: not on PATH, ydotool: not on PATH, kwtype: not on PATH, enigo-wayland: no connection); install wtype, kwtype, dotool or ydotool"
            ))
        );
        // Through the injector the text stays on the clipboard for the user to paste.
        let clipboard = MemoryClipboard::holding("before");
        let injector = injector_over(&clipboard, keys, PasteMethod::CtrlV);
        let out = injector.inject("ours").unwrap();
        assert_eq!(out.via, Via::Clipboard);
        let note = out.note.unwrap();
        assert_eq!(note.code, FallbackCode::NoTool);
        assert!(note.detail.starts_with("no paste tool on Wayland · KDE"));
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(clipboard.current().as_deref(), Some("ours"));
    }

    #[test]
    fn enigo_candidates_go_through_the_enigo_port() {
        let _spawn = spawn_lock();
        let tools = FakeTools::new();
        let xdotool = recording_tool(&tools, "xdotool", 3);
        let chord = PasteMethod::CtrlV.chord_for("linux");
        // X11 with a working XTEST connection: enigo-x11 first, xdotool never runs.
        let enigo = FakeEnigo::with(&[EnigoBackend::X11], Ok(()));
        let keys = make_keys(&tools, X11, enigo.clone());
        assert_eq!(keys.deliver(chord, "t").unwrap(), Delivered { tool: "enigo-x11".into(), delivery: Delivery::Chord });
        assert_eq!(enigo.pressed.lock().unwrap().as_slice(), &[(EnigoBackend::X11, chord)]);
        assert!(!xdotool.exists());
        // Refused by enigo and by xdotool: both reasons.
        let enigo = FakeEnigo::with(&[EnigoBackend::X11], Err(EnigoError::Input("refused".into())));
        let keys = make_keys(&tools, X11, enigo);
        assert_eq!(
            keys.deliver(chord, "t").unwrap_err(),
            DeliveryError::Failed("enigo-x11: input refused: refused; xdotool: exited with exit status: 3".into())
        );
        // The connection vanished between probe and press: counted as not started.
        let enigo = FakeEnigo::with(&[EnigoBackend::X11], Err(EnigoError::Connect("gone".into())));
        let keys = make_keys(&tools, X11, enigo);
        assert_eq!(keys.attempt(TypingTool::EnigoX11, chord, "t"), Attempt::NotStarted("enigo-x11: no input connection: gone".into()));
        // Wayland tail: enigo-wayland after the missing tools.
        let tools = FakeTools::new();
        let enigo = FakeEnigo::with(&[EnigoBackend::Wayland], Ok(()));
        let keys = make_keys(&tools, WLROOTS, enigo.clone());
        assert_eq!(keys.deliver(chord, "t").unwrap().tool, "enigo-wayland");
        assert_eq!(enigo.pressed.lock().unwrap()[0].0, EnigoBackend::Wayland);
        assert_eq!(enigo_backend(TypingTool::Wtype), None);
        assert_eq!(enigo_backend(TypingTool::EnigoWayland), Some(EnigoBackend::Wayland));
        // A tool that vanished from PATH after the probe is not started.
        assert_eq!(keys.attempt(TypingTool::Wtype, chord, "t"), Attempt::NotStarted("wtype: vanished from PATH".into()));
    }

    /// docs/dictation.md §19: the copy walks the same chain with `Insert`; `kwtype` (a typing tool)
    /// is never started for it, enigo gets the held modifiers to release, `xdotool` clears them
    /// itself.
    #[test]
    fn the_copy_chain_presses_ctrl_insert_and_skips_typing_tools() {
        let _spawn = spawn_lock();
        let copy = crate::copy_chord_for("linux");
        // KDE with only kwtype: nothing can copy — Unavailable, naming why kwtype was passed over.
        let tools = FakeTools::new();
        let kwtype = recording_tool(&tools, "kwtype", 0);
        let keys = make_keys(&tools, KDE_WAYLAND, FakeEnigo::none());
        let err = keys.press_chord(copy, &[Modifier::Alt]).unwrap_err();
        assert!(
            matches!(&err, DeliveryError::Unavailable(n) if n.code == FallbackCode::NoTool && n.detail.starts_with("no copy tool on Wayland · KDE") && n.detail.contains("kwtype: types text, cannot press a chord")),
            "{err:?}"
        );
        assert!(!kwtype.exists(), "kwtype never ran for a copy");
        assert_eq!(keys.attempt_copy(TypingTool::Kwtype, copy, &[]), Attempt::NotStarted("kwtype: types text, cannot press a chord".into()));
        // wtype: the virtual keyboard has its own modifier state, the held ones do not matter.
        let tools = FakeTools::new();
        let log = recording_tool(&tools, "wtype", 0);
        let keys = make_keys(&tools, WLROOTS, FakeEnigo::none());
        assert_eq!(keys.press_chord(copy, &[Modifier::Alt]).unwrap(), Delivered { tool: "wtype".into(), delivery: Delivery::Chord });
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "-M ctrl -k Insert -m ctrl\n");
        // dotool on stdin, ydotool 1.x with KEY_INSERT.
        let tools = FakeTools::new();
        let log = tools.log("dotool.log");
        tools.script("dotool", &format!("cat >> '{}'", log.display()));
        assert_eq!(make_keys(&tools, KDE_WAYLAND, FakeEnigo::none()).press_chord(copy, &[]).unwrap().tool, "dotool");
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "key ctrl+Insert\n");
        let tools = FakeTools::new();
        let log = tools.log("ydotool.log");
        tools.script(
            "ydotool",
            &format!("if [ \"$1\" = --help ]; then echo 'Use environment variable YDOTOOL_SOCKET'; exit 0; fi; printf '%s\\n' \"$*\" >> '{}'", log.display()),
        );
        assert_eq!(make_keys(&tools, KDE_WAYLAND, FakeEnigo::none()).press_chord(copy, &[]).unwrap().tool, "ydotool");
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "key 29:1 110:1 110:0 29:0\n");
        // X11: enigo first, with the held modifiers to release; when it refuses, xdotool --clearmodifiers.
        let tools = FakeTools::new();
        let xdotool = recording_tool(&tools, "xdotool", 0);
        let enigo = FakeEnigo::with(&[EnigoBackend::X11], Ok(()));
        let keys = make_keys(&tools, X11, enigo.clone());
        assert_eq!(keys.press_chord(copy, &[Modifier::Control, Modifier::Alt]).unwrap().tool, "enigo-x11");
        assert_eq!(enigo.copied.lock().unwrap().as_slice(), &[(EnigoBackend::X11, copy, vec![Modifier::Control, Modifier::Alt])]);
        assert!(enigo.pressed.lock().unwrap().is_empty(), "a copy is not a paste");
        assert!(!xdotool.exists());
        let enigo = FakeEnigo::with(&[EnigoBackend::X11], Err(EnigoError::Input("BadAccess".into())));
        assert_eq!(make_keys(&tools, X11, enigo).press_chord(copy, &[Modifier::Alt]).unwrap().tool, "xdotool");
        assert_eq!(std::fs::read_to_string(&xdotool).unwrap(), "key --clearmodifiers ctrl+Insert\n");
        // Through the copier: the X11 chain copies what a fake application puts on the clipboard.
        let clipboard = MemoryClipboard::holding("before");
        let copier = crate::ClipboardSelection::with_ports(
            clipboard.clone() as Arc<dyn ClipboardPort>,
            Arc::new(make_keys(&tools, X11, FakeEnigo::with(&[EnigoBackend::X11], Ok(())))),
            copy,
            crate::CopyOptions { timeout: Duration::from_millis(30), poll: Duration::from_millis(5) },
        );
        assert_eq!(crate::SelectionSource::copy_selection(&copier, &[Modifier::Alt]).unwrap(), None, "the fake application copies nothing");
        assert_eq!(clipboard.current().as_deref(), Some("before"));
    }

    #[test]
    fn classify_table() {
        let exit = RunError::Exit("exited with exit status: 1".into());
        let timeout = RunError::Timeout(Duration::from_millis(5));
        let spawn = RunError::Spawn("Permission denied".into());
        assert_eq!(classify(TypingTool::Wtype, &exit), Attempt::Retry("wtype: exited with exit status: 1".into()));
        assert_eq!(classify(TypingTool::Xdotool, &exit), Attempt::Retry("xdotool: exited with exit status: 1".into()));
        assert_eq!(classify(TypingTool::Ydotool(YdotoolSyntax::KeyCodes), &exit), Attempt::Retry("ydotool: exited with exit status: 1".into()));
        assert_eq!(classify(TypingTool::Kwtype, &exit), Attempt::Stop("kwtype: exited with exit status: 1".into()));
        assert_eq!(classify(TypingTool::Dotool, &timeout), Attempt::Stop("dotool: no exit within 5 ms; killed".into()));
        assert_eq!(classify(TypingTool::Wtype, &spawn), Attempt::NotStarted("wtype: could not start: Permission denied".into()));
    }

    #[test]
    fn system_wiring_is_constructible_headless() {
        let _spawn = spawn_lock();
        let keys = system_toolchain_keys();
        assert_eq!(keys.session(), detected_session());
        // Whatever the host, selection is a walk over the probes and never panics.
        let _ = keys.selection();
        let probe = SystemProbe::system();
        assert!(probe.which("sh").is_some());
        assert!(probe.which("voltip-no-such-tool").is_none());
        assert!(!probe.enigo_available(TypingTool::Wtype), "only the enigo candidates ask enigo");
        let runner = SystemRunner;
        assert!(matches!(
            runner.run(&CommandLine { program: "/nonexistent/x".into(), args: vec![], stdin: None }, Duration::from_secs(1)),
            Err(RunError::Spawn(_))
        ));
        let empty = SystemProbe::new(Some(String::new()), Arc::new(SystemRunner));
        assert_eq!(empty.ydotool_help(), None);
        if crate::display_available().is_err() {
            assert_eq!(detected_session(), X11);
            assert!(!runner.available(EnigoBackend::X11));
            assert!(!runner.available(EnigoBackend::Wayland));
            assert!(matches!(runner.press(EnigoBackend::X11, PasteMethod::CtrlV.chord_for("linux")), Err(EnigoError::Connect(_))));
        }
    }
}
