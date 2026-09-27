//! What kind of Linux graphical session this process runs in, judged from the environment only
//! (docs/dictation.md §14). Pure: every input is a variable lookup, so the decision table is
//! tested without a display. The shells put [`SessionKind`] into `HotkeyStatus.backend`
//! (`global-shortcut · Linux · Wayland`) and the injector picks its tool chain from the whole
//! [`Session`].

/// Which display protocol the focused applications speak.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SessionKind {
    /// An X server owns the session (`DISPLAY` only). XTEST and `XGrabKey` work everywhere.
    X11,
    /// A Wayland compositor with XWayland (`WAYLAND_DISPLAY` and `DISPLAY` both set): native
    /// Wayland clients take the input, X11 tools only reach XWayland windows.
    XWayland,
    /// A Wayland compositor without XWayland (`WAYLAND_DISPLAY` only): no X11 at all.
    Wayland,
}

impl SessionKind {
    /// Whether a Wayland compositor is in charge (pure Wayland or XWayland).
    pub fn is_wayland(self) -> bool {
        matches!(self, Self::XWayland | Self::Wayland)
    }

    /// Whether an X server is reachable (X11 or XWayland).
    pub fn has_x11(self) -> bool {
        matches!(self, Self::X11 | Self::XWayland)
    }
}

/// How far an X11 key grab (`XGrabKey`, what `tauri-plugin-global-shortcut` uses on Linux) reaches
/// in a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum X11Grab {
    /// X11 session: the grab sees every key press.
    Everywhere,
    /// XWayland: the grab lives inside XWayland and fires only while an X11 window has focus
    /// (KWin can forward more keys when "legacy X11 app support" allows it). The Voltip window
    /// itself is a native Wayland surface there.
    X11WindowsOnly,
    /// Pure Wayland: no X server, no grab. (global-hotkey 0.8 still answers `Ok` to `register`,
    /// because its event thread has already exited; the shell must not trust that answer.)
    Unavailable,
}

impl SessionKind {
    /// Reach of an X11 key grab in this session.
    pub fn x11_grab(self) -> X11Grab {
        match self {
            Self::X11 => X11Grab::Everywhere,
            Self::XWayland => X11Grab::X11WindowsOnly,
            Self::Wayland => X11Grab::Unavailable,
        }
    }
}

/// The command a compositor keyboard shortcut should run to toggle dictation in the running
/// instance (docs/dictation.md §13 `--toggle`): the AppImage itself when running from one
/// (`$APPIMAGE`), the bare program name when `PATH` finds this very executable (the deb / rpm
/// install), the absolute path otherwise (a dev build). Paths with spaces are single-quoted.
pub fn toggle_command(appimage: Option<&str>, exe: Option<&std::path::Path>, on_path: impl Fn(&str) -> Option<std::path::PathBuf>) -> String {
    remote_command(appimage, exe, on_path, "--toggle")
}

/// [`toggle_command`] for any remote-control `flag` of the running instance (`--edit-toggle`,
/// the voice-edit key of docs/dictation.md §19).
pub fn remote_command(appimage: Option<&str>, exe: Option<&std::path::Path>, on_path: impl Fn(&str) -> Option<std::path::PathBuf>, flag: &str) -> String {
    let quote = |p: &str| if p.contains(char::is_whitespace) { format!("'{p}'") } else { p.to_string() };
    if let Some(appimage) = appimage.map(str::trim).filter(|a| !a.is_empty()) {
        return format!("{} {flag}", quote(appimage));
    }
    let Some(exe) = exe else { return format!("voltip-desktop {flag}") };
    let name = exe.file_name().and_then(|n| n.to_str());
    match name {
        Some(name) if on_path(name).is_some_and(|found| found == exe) => format!("{name} {flag}"),
        _ => format!("{} {flag}", quote(&exe.display().to_string())),
    }
}

impl std::fmt::Display for SessionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::X11 => "X11",
            Self::XWayland => "XWayland",
            Self::Wayland => "Wayland",
        })
    }
}

/// Desktop family, as far as the injector cares: it decides which Wayland input protocols exist.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Desktop {
    /// KWin: `zwp_virtual_keyboard_v1`, KDE fake-input (`kwtype`), wlr-data-control.
    Kde,
    /// Mutter: none of the virtual-keyboard protocols, no wlr-data-control; XTEST through
    /// XWayland is forwarded to the focused client.
    Gnome,
    /// sway, Hyprland, river, labwc, wayfire, niri, …: the wlr protocol family.
    Wlroots,
    /// Anything else (weston, a bare X11 window manager, unknown).
    Other,
}

impl std::fmt::Display for Desktop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Kde => "KDE",
            Self::Gnome => "GNOME",
            Self::Wlroots => "wlroots",
            Self::Other => "other",
        })
    }
}

/// The graphical session: protocol plus desktop family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Session {
    /// Display protocol.
    pub kind: SessionKind,
    /// Desktop family.
    pub desktop: Desktop,
}

impl Session {
    /// Judge the session from `env` (a lookup of environment variables, `None` when unset).
    /// Returns `None` when neither `DISPLAY` nor `WAYLAND_DISPLAY` is set: no display server,
    /// nothing to inject into.
    ///
    /// Sockets decide the protocol, `XDG_SESSION_TYPE` only breaks the tie when both are set
    /// (an X11 session that inherited a stray `WAYLAND_DISPLAY` says `x11`; a Wayland session
    /// with XWayland says `wayland` or nothing). The desktop comes from `XDG_CURRENT_DESKTOP`
    /// (colon-separated list, case-insensitive), then `DESKTOP_SESSION`, then
    /// `KDE_FULL_SESSION`.
    pub fn from_env(env: impl Fn(&str) -> Option<String>) -> Option<Self> {
        let set = |name: &str| env(name).filter(|v| !v.trim().is_empty());
        let wayland = set("WAYLAND_DISPLAY").is_some();
        let x11 = set("DISPLAY").is_some();
        let session_type = set("XDG_SESSION_TYPE").map(|v| v.trim().to_ascii_lowercase());
        let kind = match (wayland, x11) {
            (false, false) => return None,
            (false, true) => SessionKind::X11,
            (true, false) => SessionKind::Wayland,
            (true, true) if session_type.as_deref() == Some("x11") => SessionKind::X11,
            (true, true) => SessionKind::XWayland,
        };
        let desktop = desktop_from(set("XDG_CURRENT_DESKTOP").as_deref(), set("DESKTOP_SESSION").as_deref(), set("KDE_FULL_SESSION").is_some());
        Some(Self { kind, desktop })
    }

    /// [`Session::from_env`] against this process's environment.
    pub fn detect() -> Option<Self> {
        Self::from_env(|name| std::env::var(name).ok())
    }

    /// Short label for logs and status strings: `Wayland · KDE`.
    pub fn label(&self) -> String {
        format!("{} · {}", self.kind, self.desktop)
    }
}

impl std::fmt::Display for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label())
    }
}

/// Desktop family from the freedesktop variables. `XDG_CURRENT_DESKTOP` wins; it is a
/// colon-separated list (`ubuntu:GNOME`, `KDE`, `sway`), matched case-insensitively.
pub fn desktop_from(current_desktop: Option<&str>, desktop_session: Option<&str>, kde_full_session: bool) -> Desktop {
    const WLROOTS: [&str; 10] = ["sway", "hyprland", "river", "labwc", "wayfire", "niri", "dwl", "cage", "hikari", "cosmic"];
    let classify = |value: &str| -> Option<Desktop> {
        for part in value.split(':').map(str::trim).filter(|p| !p.is_empty()) {
            let lower = part.to_ascii_lowercase();
            if lower.contains("kde") || lower.contains("plasma") {
                return Some(Desktop::Kde);
            }
            if lower.contains("gnome") || lower == "unity" || lower == "cinnamon" || lower == "budgie" {
                return Some(Desktop::Gnome);
            }
            if WLROOTS.iter().any(|w| lower.starts_with(w)) {
                return Some(Desktop::Wlroots);
            }
        }
        None
    };
    current_desktop
        .and_then(classify)
        .or_else(|| desktop_session.and_then(classify))
        .or(if kde_full_session { Some(Desktop::Kde) } else { None })
        .unwrap_or(Desktop::Other)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let vars: Vec<(String, String)> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |name: &str| vars.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
    }

    /// One row: the environment and the kind it must produce.
    type KindCase = (&'static [(&'static str, &'static str)], Option<SessionKind>);

    #[test]
    fn session_kind_table() {
        let cases: &[KindCase] = &[
            (&[], None),
            (&[("DISPLAY", "  "), ("WAYLAND_DISPLAY", "")], None),
            (&[("XDG_SESSION_TYPE", "wayland")], None),
            (&[("DISPLAY", ":0")], Some(SessionKind::X11)),
            (&[("DISPLAY", ":0"), ("XDG_SESSION_TYPE", "x11")], Some(SessionKind::X11)),
            (&[("WAYLAND_DISPLAY", "wayland-0")], Some(SessionKind::Wayland)),
            (&[("WAYLAND_DISPLAY", "wayland-0"), ("XDG_SESSION_TYPE", "wayland")], Some(SessionKind::Wayland)),
            (&[("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", ":1")], Some(SessionKind::XWayland)),
            (&[("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", ":1"), ("XDG_SESSION_TYPE", "wayland")], Some(SessionKind::XWayland)),
            (&[("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", ":1"), ("XDG_SESSION_TYPE", "tty")], Some(SessionKind::XWayland)),
            // A stray WAYLAND_DISPLAY inside a session that calls itself X11 is still X11.
            (&[("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", ":1"), ("XDG_SESSION_TYPE", "X11")], Some(SessionKind::X11)),
        ];
        for (vars, expected) in cases {
            assert_eq!(Session::from_env(env(vars)).map(|s| s.kind), *expected, "{vars:?}");
        }
    }

    #[test]
    fn desktop_table() {
        let cases: &[(Option<&str>, Option<&str>, bool, Desktop)] = &[
            (Some("KDE"), None, false, Desktop::Kde),
            (Some("plasma"), None, false, Desktop::Kde),
            (Some("GNOME"), None, false, Desktop::Gnome),
            (Some("ubuntu:GNOME"), None, false, Desktop::Gnome),
            (Some("Budgie:GNOME"), None, false, Desktop::Gnome),
            (Some("GNOME-Classic:GNOME"), None, false, Desktop::Gnome),
            (Some("Unity"), None, false, Desktop::Gnome),
            (Some("X-Cinnamon"), None, false, Desktop::Other),
            (Some("sway"), None, false, Desktop::Wlroots),
            (Some("Hyprland"), None, false, Desktop::Wlroots),
            (Some("river"), None, false, Desktop::Wlroots),
            (Some("niri"), None, false, Desktop::Wlroots),
            (Some("COSMIC"), None, false, Desktop::Wlroots),
            (Some("wlroots:labwc"), None, false, Desktop::Wlroots),
            (Some("XFCE"), None, false, Desktop::Other),
            (Some("weston"), None, false, Desktop::Other),
            (Some(""), Some("plasmawayland"), false, Desktop::Kde),
            (None, Some("gnome-xorg"), false, Desktop::Gnome),
            (None, Some("sway"), false, Desktop::Wlroots),
            (None, None, true, Desktop::Kde),
            (None, None, false, Desktop::Other),
            // The first variable wins even when the second would say otherwise.
            (Some("sway"), Some("plasma"), true, Desktop::Wlroots),
        ];
        for (current, session, kde, expected) in cases {
            assert_eq!(desktop_from(*current, *session, *kde), *expected, "{current:?} {session:?} {kde}");
        }
    }

    #[test]
    fn x11_grab_reach_per_session() {
        assert_eq!(SessionKind::X11.x11_grab(), X11Grab::Everywhere);
        assert_eq!(SessionKind::XWayland.x11_grab(), X11Grab::X11WindowsOnly);
        assert_eq!(SessionKind::Wayland.x11_grab(), X11Grab::Unavailable);
    }

    #[test]
    fn toggle_command_table() {
        use std::path::{Path, PathBuf};
        let path_has = |dir: &'static str| move |name: &str| Some(PathBuf::from(format!("{dir}/{name}")));
        let nowhere = |_: &str| None::<PathBuf>;
        // AppImage wins; empty values are ignored; spaces are quoted.
        assert_eq!(
            toggle_command(Some("/home/u/Apps/Voltip_2.0.0_amd64.AppImage"), Some(Path::new("/tmp/.mount_x/usr/bin/voltip-desktop")), nowhere),
            "/home/u/Apps/Voltip_2.0.0_amd64.AppImage --toggle"
        );
        assert_eq!(toggle_command(Some("/home/u/My Apps/Voltip.AppImage"), None, nowhere), "'/home/u/My Apps/Voltip.AppImage' --toggle");
        assert_eq!(toggle_command(Some("  "), Some(Path::new("/usr/bin/voltip-desktop")), path_has("/usr/bin")), "voltip-desktop --toggle");
        // Installed: PATH finds this very binary → the bare name.
        assert_eq!(toggle_command(None, Some(Path::new("/usr/bin/voltip-desktop")), path_has("/usr/bin")), "voltip-desktop --toggle");
        // PATH finds a different binary of the same name → the absolute path.
        assert_eq!(toggle_command(None, Some(Path::new("/opt/voltip/voltip-desktop")), path_has("/usr/bin")), "/opt/voltip/voltip-desktop --toggle");
        // Dev build, not on PATH.
        assert_eq!(toggle_command(None, Some(Path::new("/src/target/debug/voltip-desktop")), nowhere), "/src/target/debug/voltip-desktop --toggle");
        assert_eq!(toggle_command(None, Some(Path::new("/src/my target/voltip-desktop")), nowhere), "'/src/my target/voltip-desktop' --toggle");
        // Nothing known.
        assert_eq!(toggle_command(None, None, nowhere), "voltip-desktop --toggle");
        // docs/dictation.md §19: the voice-edit key's command follows the same rule.
        assert_eq!(remote_command(None, None, nowhere, "--edit-toggle"), "voltip-desktop --edit-toggle");
        assert_eq!(remote_command(Some("/home/u/My Apps/Voltip.AppImage"), None, nowhere, "--edit-toggle"), "'/home/u/My Apps/Voltip.AppImage' --edit-toggle");
        assert_eq!(remote_command(None, Some(Path::new("/usr/bin/voltip-desktop")), path_has("/usr/bin"), "--edit-toggle"), "voltip-desktop --edit-toggle");
    }

    #[test]
    fn session_from_env_combines_kind_and_desktop() {
        let s = Session::from_env(env(&[("WAYLAND_DISPLAY", "wayland-1"), ("XDG_CURRENT_DESKTOP", "KDE")])).unwrap();
        assert_eq!(s, Session { kind: SessionKind::Wayland, desktop: Desktop::Kde });
        assert_eq!(s.label(), "Wayland · KDE");
        assert_eq!(s.to_string(), "Wayland · KDE");
        assert!(s.kind.is_wayland() && !s.kind.has_x11());
        let s = Session::from_env(env(&[("DISPLAY", ":0"), ("WAYLAND_DISPLAY", "wayland-0"), ("XDG_CURRENT_DESKTOP", "ubuntu:GNOME")])).unwrap();
        assert_eq!(s.label(), "XWayland · GNOME");
        assert!(s.kind.is_wayland() && s.kind.has_x11());
        let s = Session::from_env(env(&[("DISPLAY", ":99")])).unwrap();
        assert_eq!(s.label(), "X11 · other");
        assert!(!s.kind.is_wayland() && s.kind.has_x11());
        assert_eq!(SessionKind::Wayland.to_string(), "Wayland");
        assert_eq!(SessionKind::XWayland.to_string(), "XWayland");
        assert_eq!(Desktop::Wlroots.to_string(), "wlroots");
        // `detect()` reads this process's environment; it agrees with `from_env` on the same lookup.
        assert_eq!(Session::detect(), Session::from_env(|n| std::env::var(n).ok()));
    }
}
