//! Foreground-application facts → the app identity scenes match on (docs/dictation.md §18.2 /
//! §18.3). Pure functions: the shell's OS calls hand over raw facts — a Windows image path, the
//! X11 `WM_CLASS` / title bytes, a macOS bundle identifier — and these turn them into the
//! normalised `app_id` plus a display name, identically on every host.
//!
//! The id rule is the core's (`voltip_core::scenes::normalize_app_id`): trim, lower-case, drop
//! trailing `.exe`, trim again. The core applies it once more when matching (it is idempotent), so
//! an id produced here and an id the user typed into a scene always compare equal.
//!
//! [`is_terminal`] is the voice edit's guard (docs/dictation.md §19.2): the copy chord is Ctrl+C
//! on Windows and Linux — the interrupt of a terminal — so an edit is refused when the probe names
//! one of [`terminal_ids`].

use crate::HostOs;

/// An application as the probe names it: the id scenes match on and the name people read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppIdentity {
    /// Normalised id (`slack`, `code`, `com.microsoft.vscode`).
    pub app_id: String,
    /// Display name (`slack`, `WINWORD`, `Code`, `Visual Studio Code`).
    pub name: String,
}

/// The id normalisation of docs/dictation.md §18.3: trim → lower-case → strip trailing `.exe`
/// (repeatedly, so the rule is idempotent) → trim. Empty when nothing is left.
pub fn normalize_app_id(raw: &str) -> String {
    let lower = raw.trim().to_lowercase();
    let mut id = lower.as_str();
    while let Some(stem) = id.strip_suffix(".exe") {
        id = stem.trim_end();
    }
    id.trim().to_owned()
}

/// `name` without a trailing `.exe` (any case), keeping the rest as written.
fn strip_exe(name: &str) -> &str {
    let bytes = name.as_bytes();
    if bytes.len() >= 4 && bytes[bytes.len() - 4..].eq_ignore_ascii_case(b".exe") { &name[..name.len() - 4] } else { name }
}

/// Windows (`QueryFullProcessImageNameW`): `C:\Program Files\Slack\slack.exe` → id `slack`, name
/// `slack`; `…\Office16\WINWORD.EXE` → id `winword`, name `WINWORD`. `None` for an empty path or a
/// path that ends in a separator.
pub fn from_exe_path(path: &str) -> Option<AppIdentity> {
    let file = path.trim().rsplit(['\\', '/']).next()?.trim();
    let name = strip_exe(file).trim();
    let app_id = normalize_app_id(name);
    (!app_id.is_empty()).then(|| AppIdentity { app_id, name: name.to_owned() })
}

/// X11 text property bytes: `UTF8_STRING` is UTF-8; `STRING` is Latin-1 by the ICCCM, but many
/// clients write UTF-8 into it anyway, so valid UTF-8 wins and everything else is read as Latin-1.
pub fn decode_x11_text(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => bytes.iter().map(|&b| char::from(b)).collect(),
    }
}

/// X11 `WM_CLASS`: two NUL-terminated strings, instance then class (`code\0Code\0`). The class is
/// the application (`Code`, `firefox`, `Slack`); the instance stands in when the class is empty.
pub fn from_wm_class(bytes: &[u8]) -> Option<AppIdentity> {
    let mut parts = bytes.split(|&b| b == 0).map(|part| decode_x11_text(part).trim().to_owned());
    let instance = parts.next().unwrap_or_default();
    let class = parts.next().unwrap_or_default();
    let name = if class.is_empty() { instance } else { class };
    let app_id = normalize_app_id(&name);
    (!app_id.is_empty()).then_some(AppIdentity { app_id, name })
}

/// macOS (`NSRunningApplication`): the bundle identifier is the id (`com.tinyspeck.slackmacgap`),
/// the localised name the display name (`Slack`, falling back to the bundle id). No bundle id — a
/// bare executable without an `Info.plist` — gives `None`: its name alone is not a stable id.
pub fn from_bundle(bundle_id: Option<&str>, localized_name: Option<&str>) -> Option<AppIdentity> {
    let bundle = bundle_id.map(str::trim).filter(|b| !b.is_empty())?;
    let app_id = normalize_app_id(bundle);
    if app_id.is_empty() {
        return None;
    }
    let name = localized_name.map(str::trim).filter(|n| !n.is_empty()).unwrap_or(bundle).to_owned();
    Some(AppIdentity { app_id, name })
}

/// Windows terminals by the id the probe reports (the image name, normalised): Windows Terminal,
/// the console host and its shells, and the third-party terminals people run there.
pub const WINDOWS_TERMINALS: [&str; 11] =
    ["windowsterminal", "cmd", "conhost", "powershell", "pwsh", "wezterm-gui", "alacritty", "mintty", "kitty", "hyper", "tabby"];

/// Linux terminals by their X11 `WM_CLASS` class (the id the probe reports, normalised), with the
/// reverse-DNS classes newer releases use next to the old names.
pub const LINUX_TERMINALS: [&str; 33] = [
    "gnome-terminal-server",
    "gnome-terminal",
    "konsole",
    "xfce4-terminal",
    "xterm",
    "uxterm",
    "urxvt",
    "rxvt",
    "alacritty",
    "kitty",
    "foot",
    "tilix",
    "terminator",
    "wezterm",
    "org.wezfurlong.wezterm",
    "com.mitchellh.ghostty",
    "ghostty",
    "ptyxis",
    "org.gnome.ptyxis",
    "org.gnome.console",
    "kgx",
    "st",
    "st-256color",
    "qterminal",
    "lxterminal",
    "mate-terminal",
    "terminology",
    "yakuake",
    "guake",
    "tilda",
    "cool-retro-term",
    "sakura",
    "deepin-terminal",
];

/// The terminals on `os` where a voice edit is refused before any key is pressed
/// (docs/dictation.md §19.2). Their selection is program output, which a rewrite cannot replace,
/// and on Windows and Linux most terminal emulators do not bind the copy chord (Ctrl+Insert — never
/// Ctrl+C, the interrupt): it would reach the running program as an escape sequence. macOS has
/// none: Cmd+C copies in Terminal, iTerm2 and the rest, and nothing reaches the program. Other hosts
/// send no copy chord at all.
pub const fn terminal_ids(os: HostOs) -> &'static [&'static str] {
    match os {
        HostOs::Windows => &WINDOWS_TERMINALS,
        HostOs::Linux => &LINUX_TERMINALS,
        HostOs::Macos | HostOs::Other => &[],
    }
}

/// Whether `app_id` (as the probe reports it; normalised again here) is a terminal on `os` — a
/// voice edit there is refused before any key is pressed.
pub fn is_terminal(os: HostOs, app_id: &str) -> bool {
    let id = normalize_app_id(app_id);
    terminal_ids(os).contains(&id.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(app_id: &str, name: &str) -> Option<AppIdentity> {
        Some(AppIdentity { app_id: app_id.into(), name: name.into() })
    }

    #[test]
    fn app_ids_normalise_trim_case_and_the_exe_suffix() {
        for (raw, want) in [
            ("slack", "slack"),
            ("  Slack.EXE ", "slack"),
            ("WINWORD.exe", "winword"),
            ("Code - Insiders.exe", "code - insiders"),
            ("com.Microsoft.VSCode", "com.microsoft.vscode"),
            ("exe", "exe"),
            (".exe", ""),
            ("   ", ""),
            ("notepad.exe.exe", "notepad"),
            ("notepad .exe", "notepad"),
            ("微信.exe", "微信"),
        ] {
            assert_eq!(normalize_app_id(raw), want, "{raw:?}");
            assert_eq!(normalize_app_id(&normalize_app_id(raw)), normalize_app_id(raw), "idempotent for {raw:?}");
        }
    }

    /// Windows image paths: the file name without `.exe` is the name (case kept), its normalised
    /// form the id; both separators work; a path without a file name is nothing.
    #[test]
    fn windows_exe_paths_become_ids() {
        assert_eq!(from_exe_path(r"C:\Program Files\Slack\slack.exe"), id("slack", "slack"));
        assert_eq!(from_exe_path(r"C:\Program Files\Microsoft Office\root\Office16\WINWORD.EXE"), id("winword", "WINWORD"));
        assert_eq!(from_exe_path(r"C:\Users\me\AppData\Local\Programs\Microsoft VS Code\Code.exe"), id("code", "Code"));
        assert_eq!(from_exe_path("C:/tools/Code - Insiders.exe"), id("code - insiders", "Code - Insiders"));
        assert_eq!(from_exe_path(r"\\?\C:\Windows\System32\notepad.exe"), id("notepad", "notepad"));
        assert_eq!(from_exe_path(r"D:\apps\WeChat\微信.exe"), id("微信", "微信"));
        assert_eq!(from_exe_path("explorer"), id("explorer", "explorer"), "no directory, no suffix");
        for bad in ["", "   ", r"C:\Windows\", "C:/x/.exe"] {
            assert_eq!(from_exe_path(bad), None, "{bad:?}");
        }
    }

    /// X11 `WM_CLASS`: the class names the app, the instance stands in for an empty class; the
    /// strings are UTF-8 when valid, Latin-1 otherwise.
    #[test]
    fn x11_wm_class_becomes_an_id() {
        assert_eq!(from_wm_class(b"code\0Code\0"), id("code", "Code"));
        assert_eq!(from_wm_class(b"Navigator\0firefox\0"), id("firefox", "firefox"));
        assert_eq!(from_wm_class(b"slack\0Slack"), id("slack", "Slack"), "a missing final NUL is tolerated");
        assert_eq!(from_wm_class(b"jetbrains-idea\0\0"), id("jetbrains-idea", "jetbrains-idea"), "empty class: the instance");
        assert_eq!(from_wm_class(b"\0\0"), None);
        assert_eq!(from_wm_class(b""), None);
        assert_eq!(from_wm_class(" wechat \0 WeChat \0".as_bytes()), id("wechat", "WeChat"), "trimmed");
        assert_eq!(from_wm_class("微信\0微信\0".as_bytes()), id("微信", "微信"), "UTF-8 in STRING is accepted");
        assert_eq!(decode_x11_text(b"caf\xe9"), "café", "Latin-1 when not UTF-8");
        assert_eq!(decode_x11_text("日本".as_bytes()), "日本");
    }

    /// macOS: the bundle id (normalised) is the id, the localised name the display name; no
    /// bundle id means no identity.
    #[test]
    fn macos_bundle_ids_become_ids() {
        assert_eq!(from_bundle(Some("com.tinyspeck.slackmacgap"), Some("Slack")), id("com.tinyspeck.slackmacgap", "Slack"));
        assert_eq!(from_bundle(Some("com.microsoft.VSCode"), Some("Code")), id("com.microsoft.vscode", "Code"));
        assert_eq!(from_bundle(Some("com.apple.Terminal"), None), id("com.apple.terminal", "com.apple.Terminal"));
        assert_eq!(from_bundle(Some("com.apple.Safari"), Some("  ")), id("com.apple.safari", "com.apple.Safari"));
        assert_eq!(from_bundle(None, Some("a.out")), None);
        assert_eq!(from_bundle(Some("  "), Some("x")), None);
    }

    /// docs/dictation.md §19.2: the voice edit's terminal guard, per OS, on the ids the probes
    /// report (`from_exe_path` / `from_wm_class` / `from_bundle` output). macOS is exempt — Cmd+C
    /// copies in every terminal there.
    #[test]
    fn terminal_ids_are_per_os_and_macos_is_exempt() {
        // Windows: the image names as `from_exe_path` reports them.
        for path in [
            r"C:\Program Files\WindowsApps\Microsoft.WindowsTerminal_1.21\WindowsTerminal.exe",
            r"C:\Windows\System32\cmd.exe",
            r"C:\Windows\System32\conhost.exe",
            r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",
            r"C:\Program Files\PowerShell\7\pwsh.exe",
            r"C:\Program Files\WezTerm\wezterm-gui.exe",
            r"C:\Program Files\Git\usr\bin\mintty.exe",
        ] {
            let app = from_exe_path(path).unwrap();
            assert!(is_terminal(HostOs::Windows, &app.app_id), "{path}");
            assert!(!is_terminal(HostOs::Macos, &app.app_id), "{path} on macOS");
        }
        for id in WINDOWS_TERMINALS {
            assert!(is_terminal(HostOs::Windows, id), "{id}");
        }
        // Linux: the WM_CLASS classes as `from_wm_class` reports them.
        for wm_class in [
            &b"gnome-terminal-server\0Gnome-terminal\0"[..],
            b"konsole\0konsole\0",
            b"xterm\0XTerm\0",
            b"urxvt\0URxvt\0",
            b"Alacritty\0Alacritty\0",
            b"org.wezfurlong.wezterm\0org.wezfurlong.wezterm\0",
            b"com.mitchellh.ghostty\0com.mitchellh.ghostty\0",
            b"kgx\0org.gnome.Console\0",
            b"xfce4-terminal\0Xfce4-terminal\0",
        ] {
            let app = from_wm_class(wm_class).unwrap();
            assert!(is_terminal(HostOs::Linux, &app.app_id), "{app:?}");
        }
        for id in LINUX_TERMINALS {
            assert!(is_terminal(HostOs::Linux, id), "{id}");
        }
        assert!(is_terminal(HostOs::Linux, " Gnome-Terminal-Server "), "normalised again");
        assert!(is_terminal(HostOs::Windows, "WindowsTerminal.EXE"));
        // Not terminals: editors, browsers, chat apps — including ones with a built-in terminal
        // pane (the pane is inside the editor, which takes Ctrl+Insert as copy).
        for id in ["code", "slack", "chrome", "winword", "notepad", "firefox", "com.microsoft.vscode", "", "terminal-notes"] {
            assert!(!is_terminal(HostOs::Windows, id) && !is_terminal(HostOs::Linux, id), "{id}");
        }
        // macOS: Cmd+C copies in Terminal and iTerm2 — no guard; other hosts send no copy chord.
        assert!(terminal_ids(HostOs::Macos).is_empty() && terminal_ids(HostOs::Other).is_empty());
        for id in ["com.apple.terminal", "com.googlecode.iterm2", "windowsterminal", "gnome-terminal-server"] {
            assert!(!is_terminal(HostOs::Macos, id) && !is_terminal(HostOs::Other, id), "{id}");
        }
        // The two tables are their own: a Linux class is not a Windows image name.
        assert!(!is_terminal(HostOs::Windows, "gnome-terminal-server"));
        assert!(!is_terminal(HostOs::Linux, "windowsterminal"));
        assert!(WINDOWS_TERMINALS.iter().chain(LINUX_TERMINALS.iter()).all(|id| normalize_app_id(id) == *id), "ids are stored normalised");
    }
}
