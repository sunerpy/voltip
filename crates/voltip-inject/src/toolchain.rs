//! The Linux paste tool chain as pure decisions (docs/dictation.md §14): which tools can deliver
//! a paste in a given [`Session`], in which order, and the exact command line each one takes.
//! Nothing here touches the OS; [`crate::linux`] runs the chosen command, the tests here run the
//! tables.

use crate::session::{Desktop, Session, SessionKind};

/// Which key combination pastes in the focused application. Terminals want
/// [`PasteMethod::CtrlShiftV`]; some X11 programs only honour [`PasteMethod::ShiftInsert`].
/// A constructor parameter of the injectors for now, not a setting (see §14).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PasteMethod {
    /// `Ctrl+V` (`Cmd+V` on macOS): every GUI text field.
    #[default]
    CtrlV,
    /// `Ctrl+Shift+V`: GNOME Terminal, Konsole, Alacritty, kitty, the VS Code terminal.
    CtrlShiftV,
    /// `Shift+Insert`: the X11 classic; xterm, urxvt and most Qt / GTK widgets accept it too.
    /// macOS has no Insert key: there it presses `Cmd+V`.
    ShiftInsert,
}

impl PasteMethod {
    /// The chord this method presses on `os` (as [`std::env::consts::OS`] names it).
    pub fn chord_for(self, os: &str) -> Chord {
        let primary = if os == "macos" { Modifier::Meta } else { Modifier::Control };
        match self {
            Self::CtrlV => Chord { control: primary == Modifier::Control, meta: primary == Modifier::Meta, shift: false, key: Key::Char('v') },
            Self::CtrlShiftV => Chord { control: primary == Modifier::Control, meta: primary == Modifier::Meta, shift: true, key: Key::Char('v') },
            Self::ShiftInsert if os == "macos" => Self::CtrlV.chord_for(os),
            Self::ShiftInsert => Chord { control: false, meta: false, shift: true, key: Key::Insert },
        }
    }

    /// Stable name for logs and settings (`ctrl_v` / `ctrl_shift_v` / `shift_insert`).
    pub fn name(self) -> &'static str {
        match self {
            Self::CtrlV => "ctrl_v",
            Self::CtrlShiftV => "ctrl_shift_v",
            Self::ShiftInsert => "shift_insert",
        }
    }
}

/// Modifier key of a chord, or one the user may still hold from the hotkey when the selection
/// copy goes out (docs/dictation.md §19).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Modifier {
    /// `Ctrl` (Windows, Linux).
    Control,
    /// `Cmd` (macOS).
    Meta,
    /// `Shift`.
    Shift,
    /// `Alt` / Option. Never part of a paste or copy [`Chord`]; only released before the copy
    /// chord when the hotkey (`Ctrl+Alt+E`) may still hold it down.
    Alt,
}

impl Modifier {
    /// `ctrl` / `super` / `shift` / `alt`: the spelling of `xdotool`, `dotool` and ydotool 0.x.
    fn tool_name(self) -> &'static str {
        match self {
            Self::Control => "ctrl",
            Self::Meta => "super",
            Self::Shift => "shift",
            Self::Alt => "alt",
        }
    }
}

/// The non-modifier key of a chord.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// A letter, lower case.
    Char(char),
    /// The `Insert` key.
    Insert,
}

/// Modifiers plus a key, held and clicked in this order: `Ctrl`/`Cmd`, `Shift`, key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Chord {
    /// Hold `Ctrl`.
    pub control: bool,
    /// Hold `Cmd`.
    pub meta: bool,
    /// Hold `Shift`.
    pub shift: bool,
    /// The key clicked while the modifiers are held.
    pub key: Key,
}

impl Chord {
    /// Modifiers in press order (released in reverse).
    pub fn modifiers(&self) -> Vec<Modifier> {
        let mut out = Vec::with_capacity(3);
        if self.control {
            out.push(Modifier::Control);
        }
        if self.meta {
            out.push(Modifier::Meta);
        }
        if self.shift {
            out.push(Modifier::Shift);
        }
        out
    }

    /// `ctrl` / `shift` / `super` — the spelling `wtype`, `dotool`, `xdotool` and ydotool 0.x share.
    fn tool_modifiers(&self) -> Vec<&'static str> {
        self.modifiers().into_iter().map(Modifier::tool_name).collect()
    }

    /// The key as an XKB keysym name (`v`, `Insert`).
    fn keysym(&self) -> String {
        match self.key {
            Key::Char(c) => c.to_string(),
            Key::Insert => "Insert".to_string(),
        }
    }

    /// `ctrl+shift+v` — the `+`-joined spelling of xdotool, dotool and ydotool 0.x.
    fn plus_joined(&self) -> String {
        let mut parts = self.tool_modifiers();
        let key = self.keysym();
        parts.push(key.as_str());
        parts.join("+")
    }

    /// Linux evdev key codes in press order (release is the reverse): what ydotool ≥ 1.0 wants.
    pub fn evdev_codes(&self) -> Vec<u16> {
        const KEY_LEFTCTRL: u16 = 29;
        const KEY_LEFTSHIFT: u16 = 42;
        const KEY_LEFTALT: u16 = 56;
        const KEY_LEFTMETA: u16 = 125;
        const KEY_INSERT: u16 = 110;
        const KEY_C: u16 = 46;
        const KEY_V: u16 = 47;
        let mut codes: Vec<u16> = self
            .modifiers()
            .into_iter()
            .map(|m| match m {
                Modifier::Control => KEY_LEFTCTRL,
                Modifier::Meta => KEY_LEFTMETA,
                Modifier::Shift => KEY_LEFTSHIFT,
                Modifier::Alt => KEY_LEFTALT,
            })
            .collect();
        codes.push(match self.key {
            Key::Insert => KEY_INSERT,
            // Only `v` (paste) and `c` (the selection copy, docs/dictation.md §19) are ever
            // pressed here; the table stays honest instead of guessing a layout.
            Key::Char('c') => KEY_C,
            Key::Char(_) => KEY_V,
        });
        codes
    }
}

impl std::fmt::Display for Chord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts: Vec<String> = self
            .modifiers()
            .into_iter()
            .map(|m| match m {
                Modifier::Control => "Ctrl".to_string(),
                Modifier::Meta => "Cmd".to_string(),
                Modifier::Shift => "Shift".to_string(),
                Modifier::Alt => "Alt".to_string(),
            })
            .collect();
        parts.push(match self.key {
            Key::Char(c) => c.to_ascii_uppercase().to_string(),
            Key::Insert => "Insert".to_string(),
        });
        f.write_str(&parts.join("+"))
    }
}

/// Which `ydotool key` grammar the installed binary speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum YdotoolSyntax {
    /// ydotool ≥ 1.0 (C rewrite, needs `ydotoold`): `key 29:1 47:1 47:0 29:0`, evdev codes.
    KeyCodes,
    /// ydotool 0.1.x (C++): `key ctrl+v`, key names.
    KeyNames,
}

impl YdotoolSyntax {
    /// Judge the grammar from `ydotool --help` (stdout and stderr together). The 1.x client tells
    /// the user about `YDOTOOL_SOCKET` and lists `debug`; 0.1.x lists a `recorder` command and
    /// never mentions the socket. Unknown output is read as the current grammar.
    pub fn from_help(help: &str) -> Self {
        let lower = help.to_ascii_lowercase();
        if lower.contains("ydotool_socket") || lower.contains("bakers") || lower.contains("\n  debug") {
            Self::KeyCodes
        } else if lower.contains("recorder") {
            Self::KeyNames
        } else {
            Self::KeyCodes
        }
    }
}

/// A way to type into the focused application.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TypingTool {
    /// enigo's x11rb backend (XTEST), in-process. The X11 default.
    EnigoX11,
    /// `xdotool key --clearmodifiers …` (XTEST through a subprocess).
    Xdotool,
    /// `wtype` — `zwp_virtual_keyboard_v1`; wlroots compositors and KWin, not Mutter.
    Wtype,
    /// `dotool` — uinput daemon-less; any compositor, needs the `input` group.
    Dotool,
    /// `ydotool` — uinput through `ydotoold`; any compositor.
    Ydotool(YdotoolSyntax),
    /// `kwtype` — KDE fake-input protocol; types text, cannot press a chord.
    Kwtype,
    /// enigo's Wayland backend (`zwp_virtual_keyboard_v1`), in-process.
    EnigoWayland,
}

/// How a tool delivers the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    /// Presses the paste chord; the text must already be on the clipboard.
    Chord,
    /// Types the text itself, character by character.
    Type,
}

/// One external command: program, arguments, and what to feed on stdin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandLine {
    /// Program name, looked up on `PATH` (or an absolute path).
    pub program: String,
    /// Arguments.
    pub args: Vec<String>,
    /// Bytes for stdin (`None` = stdin closed).
    pub stdin: Option<String>,
}

impl TypingTool {
    /// Name for logs and the injection note (`wtype`, `ydotool`, `enigo-x11`, …).
    pub fn name(self) -> &'static str {
        match self {
            Self::EnigoX11 => "enigo-x11",
            Self::Xdotool => "xdotool",
            Self::Wtype => "wtype",
            Self::Dotool => "dotool",
            Self::Ydotool(_) => "ydotool",
            Self::Kwtype => "kwtype",
            Self::EnigoWayland => "enigo-wayland",
        }
    }

    /// The executable to look for on `PATH`; `None` for the in-process backends.
    pub fn binary(self) -> Option<&'static str> {
        match self {
            Self::EnigoX11 | Self::EnigoWayland => None,
            Self::Xdotool => Some("xdotool"),
            Self::Wtype => Some("wtype"),
            Self::Dotool => Some("dotool"),
            Self::Ydotool(_) => Some("ydotool"),
            Self::Kwtype => Some("kwtype"),
        }
    }

    /// Whether the tool presses the chord or types the text.
    pub fn delivery(self) -> Delivery {
        match self {
            Self::Kwtype => Delivery::Type,
            _ => Delivery::Chord,
        }
    }

    /// The command line that delivers `text` (already on the clipboard) with `chord`; `None` for
    /// the in-process enigo backends.
    pub fn command(self, chord: Chord, text: &str) -> Option<CommandLine> {
        let s = |v: &str| v.to_string();
        let line = match self {
            Self::EnigoX11 | Self::EnigoWayland => return None,
            Self::Xdotool => CommandLine { program: s("xdotool"), args: vec![s("key"), s("--clearmodifiers"), chord.plus_joined()], stdin: None },
            Self::Wtype => {
                // Press the modifiers, click the key, release in reverse: `-M ctrl -k v -m ctrl`.
                // wtype names the Super modifier `logo`.
                let mods: Vec<&str> = chord.tool_modifiers().into_iter().map(|m| if m == "super" { "logo" } else { m }).collect();
                let mut args: Vec<String> = mods.iter().flat_map(|m| [s("-M"), s(m)]).collect();
                args.push(s("-k"));
                args.push(chord.keysym());
                args.extend(mods.iter().rev().flat_map(|m| [s("-m"), s(m)]));
                CommandLine { program: s("wtype"), args, stdin: None }
            }
            Self::Dotool => CommandLine { program: s("dotool"), args: Vec::new(), stdin: Some(format!("key {}\n", chord.plus_joined())) },
            Self::Ydotool(YdotoolSyntax::KeyNames) => CommandLine { program: s("ydotool"), args: vec![s("key"), chord.plus_joined()], stdin: None },
            Self::Ydotool(YdotoolSyntax::KeyCodes) => {
                let codes = chord.evdev_codes();
                let mut args = vec![s("key")];
                args.extend(codes.iter().map(|c| format!("{c}:1")));
                args.extend(codes.iter().rev().map(|c| format!("{c}:0")));
                CommandLine { program: s("ydotool"), args, stdin: None }
            }
            Self::Kwtype => CommandLine { program: s("kwtype"), args: vec![s("--"), s(text)], stdin: None },
        };
        Some(line)
    }
}

impl std::fmt::Display for TypingTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ydotool(YdotoolSyntax::KeyCodes) => f.write_str("ydotool (≥1.0)"),
            Self::Ydotool(YdotoolSyntax::KeyNames) => f.write_str("ydotool (0.x)"),
            other => f.write_str(other.name()),
        }
    }
}

/// The tools worth trying in `session`, best first. The ydotool entry carries
/// [`YdotoolSyntax::KeyCodes`] as a placeholder; [`select`] replaces it with the probed grammar.
///
/// * X11: enigo (XTEST) → `xdotool --clearmodifiers`.
/// * Wayland / XWayland: `wtype` → `dotool` → `ydotool` → `kwtype` (KDE) → enigo Wayland; on
///   XWayland the X11 pair follows (XTEST through XWayland reaches the focused client on Mutter
///   and X11 windows elsewhere). GNOME skips the `zwp_virtual_keyboard_v1` tools (`wtype`, enigo
///   Wayland): Mutter does not offer the protocol, so they can only fail.
pub fn candidates(session: Session) -> Vec<TypingTool> {
    let mut out = Vec::new();
    if session.kind.is_wayland() {
        let virtual_keyboard = session.desktop != Desktop::Gnome;
        if virtual_keyboard {
            out.push(TypingTool::Wtype);
        }
        out.push(TypingTool::Dotool);
        out.push(TypingTool::Ydotool(YdotoolSyntax::KeyCodes));
        if session.desktop == Desktop::Kde {
            out.push(TypingTool::Kwtype);
        }
        if virtual_keyboard {
            out.push(TypingTool::EnigoWayland);
        }
    }
    if session.kind.has_x11() {
        out.push(TypingTool::EnigoX11);
        out.push(TypingTool::Xdotool);
    }
    out
}

/// What the selector needs to know about this machine. [`crate::linux::SystemProbe`] is the real
/// one; tests hand in tables.
pub trait ToolProbe {
    /// Where `binary` is on `PATH` (what `which(1)` prints), `None` when it is not.
    fn which(&self, binary: &str) -> Option<std::path::PathBuf>;
    /// Output of `ydotool --help` (stdout and stderr), `None` when it could not run.
    fn ydotool_help(&self) -> Option<String>;
    /// Whether enigo can open the given in-process connection right now.
    fn enigo_available(&self, tool: TypingTool) -> bool;
}

/// Why a candidate was passed over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    /// The candidate.
    pub tool: TypingTool,
    /// Short reason (`not on PATH`, `no connection`).
    pub reason: &'static str,
}

/// Outcome of [`select`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    /// The first usable tool, `None` when nothing in the chain is available (clipboard only).
    pub tool: Option<TypingTool>,
    /// Candidates that were passed over before `tool`, in order.
    pub skipped: Vec<Skipped>,
}

/// Whether `candidate` can be tried right now; for ydotool the grammar is probed and filled in.
pub fn probe_candidate(candidate: TypingTool, probe: &dyn ToolProbe) -> Result<TypingTool, Skipped> {
    match candidate {
        TypingTool::EnigoX11 | TypingTool::EnigoWayland => {
            if probe.enigo_available(candidate) {
                Ok(candidate)
            } else {
                Err(Skipped { tool: candidate, reason: "no connection" })
            }
        }
        TypingTool::Ydotool(_) => {
            if probe.which("ydotool").is_none() {
                return Err(Skipped { tool: candidate, reason: "not on PATH" });
            }
            match probe.ydotool_help() {
                Some(help) => Ok(TypingTool::Ydotool(YdotoolSyntax::from_help(&help))),
                None => Err(Skipped { tool: candidate, reason: "`ydotool --help` failed" }),
            }
        }
        other => match other.binary() {
            Some(binary) if probe.which(binary).is_some() => Ok(other),
            _ => Err(Skipped { tool: other, reason: "not on PATH" }),
        },
    }
}

/// Walk [`candidates`] and pick the first one `probe` says is available (what the shell logs at
/// start-up; the paste itself walks the whole chain, see [`crate::linux::ToolchainKeys`]).
pub fn select(session: Session, probe: &dyn ToolProbe) -> Selection {
    let mut skipped = Vec::new();
    for candidate in candidates(session) {
        match probe_candidate(candidate, probe) {
            Ok(tool) => return Selection { tool: Some(tool), skipped },
            Err(s) => skipped.push(s),
        }
    }
    Selection { tool: None, skipped }
}

/// The plain-language hint for a session where no tool was found, for the injection note and §14.
pub fn install_hint(session: Session) -> &'static str {
    match (session.kind, session.desktop) {
        (SessionKind::X11, _) => "install xdotool",
        (_, Desktop::Gnome) => "install dotool or ydotool (uinput); GNOME has no virtual-keyboard protocol",
        (_, Desktop::Kde) => "install wtype, kwtype, dotool or ydotool",
        _ => "install wtype, dotool or ydotool",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    struct TableProbe {
        on_path: HashSet<&'static str>,
        ydotool_help: Option<&'static str>,
        enigo: HashSet<TypingTool>,
    }

    impl TableProbe {
        fn with(on_path: &[&'static str], enigo: &[TypingTool]) -> Self {
            Self { on_path: on_path.iter().copied().collect(), ydotool_help: None, enigo: enigo.iter().copied().collect() }
        }
    }

    impl ToolProbe for TableProbe {
        fn which(&self, binary: &str) -> Option<std::path::PathBuf> {
            self.on_path.contains(binary).then(|| std::path::PathBuf::from(format!("/fake/bin/{binary}")))
        }
        fn ydotool_help(&self) -> Option<String> {
            self.ydotool_help.map(str::to_string)
        }
        fn enigo_available(&self, tool: TypingTool) -> bool {
            self.enigo.contains(&tool)
        }
    }

    fn session(kind: SessionKind, desktop: Desktop) -> Session {
        Session { kind, desktop }
    }

    /// `ydotool --help` of v1.0.4 (Client/ydotool.c `show_help`, tool list verbatim).
    const YDOTOOL_1X_HELP: &str = "Usage: ydotool <cmd> <args>\nAvailable commands:\n  click\n  mousemove\n  type\n  key\n  debug\n  bakers\n  stdin\nUse environment variable YDOTOOL_SOCKET to specify daemon socket.\n";
    /// `ydotool --help` of Ubuntu 24.04's 0.1.8 package, captured on the build host.
    const YDOTOOL_0X_HELP: &str = "Usage: ydotool <cmd> <args>\nAvailable commands:\n  type\n  recorder\n  mousemove\n  key\n  click\n";

    #[test]
    fn paste_method_chords() {
        assert_eq!(PasteMethod::default(), PasteMethod::CtrlV);
        assert_eq!(PasteMethod::CtrlV.chord_for("linux").to_string(), "Ctrl+V");
        assert_eq!(PasteMethod::CtrlV.chord_for("windows").to_string(), "Ctrl+V");
        assert_eq!(PasteMethod::CtrlV.chord_for("macos").to_string(), "Cmd+V");
        assert_eq!(PasteMethod::CtrlShiftV.chord_for("linux").to_string(), "Ctrl+Shift+V");
        assert_eq!(PasteMethod::CtrlShiftV.chord_for("macos").to_string(), "Cmd+Shift+V");
        assert_eq!(PasteMethod::ShiftInsert.chord_for("linux").to_string(), "Shift+Insert");
        assert_eq!(PasteMethod::ShiftInsert.chord_for("windows").to_string(), "Shift+Insert");
        assert_eq!(PasteMethod::ShiftInsert.chord_for("macos").to_string(), "Cmd+V", "no Insert key on a Mac");
        assert_eq!(PasteMethod::CtrlV.name(), "ctrl_v");
        assert_eq!(PasteMethod::CtrlShiftV.name(), "ctrl_shift_v");
        assert_eq!(PasteMethod::ShiftInsert.name(), "shift_insert");
        let chord = PasteMethod::CtrlShiftV.chord_for("linux");
        assert_eq!(chord.modifiers(), vec![Modifier::Control, Modifier::Shift]);
        assert_eq!(PasteMethod::CtrlV.chord_for("macos").modifiers(), vec![Modifier::Meta]);
        assert_eq!(chord.evdev_codes(), vec![29, 42, 47]);
        assert_eq!(PasteMethod::ShiftInsert.chord_for("linux").evdev_codes(), vec![42, 110]);
        assert_eq!(PasteMethod::CtrlV.chord_for("macos").evdev_codes(), vec![125, 47]);
    }

    #[test]
    fn command_lines_per_tool() {
        let ctrl_v = PasteMethod::CtrlV.chord_for("linux");
        let ctrl_shift_v = PasteMethod::CtrlShiftV.chord_for("linux");
        let shift_insert = PasteMethod::ShiftInsert.chord_for("linux");
        let args = |c: &CommandLine| c.args.join(" ");

        let c = TypingTool::Xdotool.command(ctrl_v, "t").unwrap();
        assert_eq!((c.program.as_str(), args(&c), c.stdin.clone()), ("xdotool", "key --clearmodifiers ctrl+v".into(), None));
        assert_eq!(args(&TypingTool::Xdotool.command(ctrl_shift_v, "t").unwrap()), "key --clearmodifiers ctrl+shift+v");
        assert_eq!(args(&TypingTool::Xdotool.command(shift_insert, "t").unwrap()), "key --clearmodifiers shift+Insert");

        assert_eq!(args(&TypingTool::Wtype.command(ctrl_v, "t").unwrap()), "-M ctrl -k v -m ctrl");
        assert_eq!(args(&TypingTool::Wtype.command(ctrl_shift_v, "t").unwrap()), "-M ctrl -M shift -k v -m shift -m ctrl");
        assert_eq!(args(&TypingTool::Wtype.command(shift_insert, "t").unwrap()), "-M shift -k Insert -m shift");
        assert_eq!(args(&TypingTool::Wtype.command(PasteMethod::CtrlV.chord_for("macos"), "t").unwrap()), "-M logo -k v -m logo", "wtype says logo, not super");
        assert_eq!(args(&TypingTool::Xdotool.command(PasteMethod::CtrlV.chord_for("macos"), "t").unwrap()), "key --clearmodifiers super+v");

        let c = TypingTool::Dotool.command(ctrl_v, "t").unwrap();
        assert_eq!((c.program.as_str(), c.args.is_empty(), c.stdin.as_deref()), ("dotool", true, Some("key ctrl+v\n")));
        assert_eq!(TypingTool::Dotool.command(shift_insert, "t").unwrap().stdin.as_deref(), Some("key shift+Insert\n"));

        assert_eq!(args(&TypingTool::Ydotool(YdotoolSyntax::KeyNames).command(ctrl_v, "t").unwrap()), "key ctrl+v");
        assert_eq!(args(&TypingTool::Ydotool(YdotoolSyntax::KeyCodes).command(ctrl_v, "t").unwrap()), "key 29:1 47:1 47:0 29:0");
        assert_eq!(args(&TypingTool::Ydotool(YdotoolSyntax::KeyCodes).command(ctrl_shift_v, "t").unwrap()), "key 29:1 42:1 47:1 47:0 42:0 29:0");
        assert_eq!(args(&TypingTool::Ydotool(YdotoolSyntax::KeyCodes).command(shift_insert, "t").unwrap()), "key 42:1 110:1 110:0 42:0");

        let c = TypingTool::Kwtype.command(ctrl_v, "你好 -x").unwrap();
        assert_eq!((c.program.as_str(), c.args.clone()), ("kwtype", vec!["--".to_string(), "你好 -x".to_string()]));
        assert_eq!(TypingTool::Kwtype.delivery(), Delivery::Type);
        assert_eq!(TypingTool::Wtype.delivery(), Delivery::Chord);

        assert_eq!(TypingTool::EnigoX11.command(ctrl_v, "t"), None);
        assert_eq!(TypingTool::EnigoWayland.command(ctrl_v, "t"), None);
        assert_eq!(TypingTool::EnigoX11.binary(), None);
        assert_eq!(TypingTool::Ydotool(YdotoolSyntax::KeyNames).binary(), Some("ydotool"));
        assert_eq!(TypingTool::Ydotool(YdotoolSyntax::KeyCodes).to_string(), "ydotool (≥1.0)");
        assert_eq!(TypingTool::Ydotool(YdotoolSyntax::KeyNames).to_string(), "ydotool (0.x)");
        assert_eq!(TypingTool::EnigoWayland.to_string(), "enigo-wayland");
        assert_eq!(TypingTool::Ydotool(YdotoolSyntax::KeyNames).name(), "ydotool");
    }

    #[test]
    fn ydotool_syntax_from_help() {
        assert_eq!(YdotoolSyntax::from_help(YDOTOOL_1X_HELP), YdotoolSyntax::KeyCodes);
        assert_eq!(YdotoolSyntax::from_help(YDOTOOL_0X_HELP), YdotoolSyntax::KeyNames);
        // The 0.1.8 Debian build prints the command list and a daemon notice on stderr.
        assert_eq!(
            YdotoolSyntax::from_help(
                "ydotool: notice: ydotoold backend unavailable\nUsage: ydotool <cmd> <args>\nAvailable commands:\n  type\n  recorder\n  mousemove\n  key\n  click\n"
            ),
            YdotoolSyntax::KeyNames
        );
        assert_eq!(YdotoolSyntax::from_help(""), YdotoolSyntax::KeyCodes, "unknown output is read as the current grammar");
        assert_eq!(
            YdotoolSyntax::from_help("Usage: ydotool <cmd> <args>\nAvailable commands:\n  click\n  mousemove\n  type\n  key\n  debug\n"),
            YdotoolSyntax::KeyCodes
        );
    }

    #[test]
    fn candidate_table_desktop_by_session() {
        use Desktop::{Gnome, Kde, Other, Wlroots};
        use SessionKind::{Wayland, X11, XWayland};
        use TypingTool::{Dotool, EnigoWayland, EnigoX11, Kwtype, Wtype, Xdotool};
        let ydotool = TypingTool::Ydotool(YdotoolSyntax::KeyCodes);
        let cases: &[(SessionKind, Desktop, Vec<TypingTool>)] = &[
            (X11, Kde, vec![EnigoX11, Xdotool]),
            (X11, Gnome, vec![EnigoX11, Xdotool]),
            (X11, Other, vec![EnigoX11, Xdotool]),
            (Wayland, Kde, vec![Wtype, Dotool, ydotool, Kwtype, EnigoWayland]),
            (Wayland, Gnome, vec![Dotool, ydotool]),
            (Wayland, Wlroots, vec![Wtype, Dotool, ydotool, EnigoWayland]),
            (Wayland, Other, vec![Wtype, Dotool, ydotool, EnigoWayland]),
            (XWayland, Kde, vec![Wtype, Dotool, ydotool, Kwtype, EnigoWayland, EnigoX11, Xdotool]),
            (XWayland, Gnome, vec![Dotool, ydotool, EnigoX11, Xdotool]),
            (XWayland, Wlroots, vec![Wtype, Dotool, ydotool, EnigoWayland, EnigoX11, Xdotool]),
        ];
        for (kind, desktop, expected) in cases {
            assert_eq!(&candidates(session(*kind, *desktop)), expected, "{kind:?} {desktop:?}");
        }
    }

    #[test]
    fn selection_walks_the_chain() {
        let kde = session(SessionKind::Wayland, Desktop::Kde);
        // Everything installed: wtype wins.
        let probe = TableProbe::with(&["wtype", "dotool", "ydotool", "kwtype"], &[TypingTool::EnigoWayland]);
        assert_eq!(select(kde, &probe), Selection { tool: Some(TypingTool::Wtype), skipped: vec![] });
        // Only ydotool 1.x: the two before it are recorded as skipped and the grammar is probed.
        let mut probe = TableProbe::with(&["ydotool"], &[]);
        probe.ydotool_help = Some(YDOTOOL_1X_HELP);
        let sel = select(kde, &probe);
        assert_eq!(sel.tool, Some(TypingTool::Ydotool(YdotoolSyntax::KeyCodes)));
        assert_eq!(
            sel.skipped.iter().map(|s| (s.tool, s.reason)).collect::<Vec<_>>(),
            vec![(TypingTool::Wtype, "not on PATH"), (TypingTool::Dotool, "not on PATH")]
        );
        probe.ydotool_help = Some(YDOTOOL_0X_HELP);
        assert_eq!(select(kde, &probe).tool, Some(TypingTool::Ydotool(YdotoolSyntax::KeyNames)));
        // ydotool present but `--help` cannot run: skipped with its own reason, kwtype next.
        let mut probe = TableProbe::with(&["ydotool", "kwtype"], &[]);
        probe.ydotool_help = None;
        let sel = select(kde, &probe);
        assert_eq!(sel.tool, Some(TypingTool::Kwtype));
        assert_eq!(sel.skipped.last().map(|s| s.reason), Some("`ydotool --help` failed"));
        // Nothing but enigo's Wayland connection.
        let probe = TableProbe::with(&[], &[TypingTool::EnigoWayland]);
        let sel = select(kde, &probe);
        assert_eq!(sel.tool, Some(TypingTool::EnigoWayland));
        assert_eq!(sel.skipped.len(), 4);
        // Nothing at all: clipboard only, every candidate listed.
        let probe = TableProbe::with(&[], &[]);
        let sel = select(kde, &probe);
        assert_eq!(sel.tool, None);
        assert_eq!(sel.skipped.len(), 5);
        assert_eq!(sel.skipped.last().map(|s| s.reason), Some("no connection"));
        assert_eq!(probe_candidate(TypingTool::Kwtype, &probe), Err(Skipped { tool: TypingTool::Kwtype, reason: "not on PATH" }));
        assert_eq!(probe_candidate(TypingTool::EnigoX11, &probe), Err(Skipped { tool: TypingTool::EnigoX11, reason: "no connection" }));
    }

    #[test]
    fn selection_x11_and_xwayland() {
        let x11 = session(SessionKind::X11, Desktop::Other);
        let probe = TableProbe::with(&["xdotool", "wtype"], &[TypingTool::EnigoX11]);
        assert_eq!(select(x11, &probe).tool, Some(TypingTool::EnigoX11), "wtype is never a candidate on X11");
        let probe = TableProbe::with(&["xdotool"], &[]);
        let sel = select(x11, &probe);
        assert_eq!(sel.tool, Some(TypingTool::Xdotool));
        assert_eq!(sel.skipped, vec![Skipped { tool: TypingTool::EnigoX11, reason: "no connection" }]);
        assert_eq!(select(x11, &TableProbe::with(&[], &[])).tool, None);
        // GNOME on XWayland: no wtype even when installed; XTEST through XWayland is the tail.
        let gnome = session(SessionKind::XWayland, Desktop::Gnome);
        let probe = TableProbe::with(&["wtype", "xdotool"], &[TypingTool::EnigoX11, TypingTool::EnigoWayland]);
        assert_eq!(select(gnome, &probe).tool, Some(TypingTool::EnigoX11));
        // wlroots on XWayland with only X11 tools: enigo Wayland is tried before them.
        let sway = session(SessionKind::XWayland, Desktop::Wlroots);
        let probe = TableProbe::with(&["xdotool"], &[TypingTool::EnigoX11, TypingTool::EnigoWayland]);
        assert_eq!(select(sway, &probe).tool, Some(TypingTool::EnigoWayland));
        let probe = TableProbe::with(&["xdotool"], &[TypingTool::EnigoX11]);
        assert_eq!(select(sway, &probe).tool, Some(TypingTool::EnigoX11));
    }

    #[test]
    fn install_hints() {
        assert_eq!(install_hint(session(SessionKind::X11, Desktop::Kde)), "install xdotool");
        assert!(install_hint(session(SessionKind::Wayland, Desktop::Gnome)).contains("GNOME"));
        assert!(install_hint(session(SessionKind::XWayland, Desktop::Kde)).contains("kwtype"));
        assert_eq!(install_hint(session(SessionKind::Wayland, Desktop::Wlroots)), "install wtype, dotool or ydotool");
    }
}
