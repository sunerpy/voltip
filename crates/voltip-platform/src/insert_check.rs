//! The insert check (docs/dictation.md §28): its rules ([`rules`], what the core and the platform
//! readers decide with) and the applications whose fields it never reads. A take
//! pasted into one of them reports `unconfirmed`, never `not_written`:
//!
//! - terminals, whose "field" is program output: the shell echoes, redraws and completes what it
//!   is given, so the line before the cursor says nothing about the paste (the voice edit's tables,
//!   [`crate::foreground::terminal_ids`], plus the macOS terminals, which the voice edit does not
//!   need);
//! - remote-desktop and virtual-machine clients, whose window shows another computer: the local
//!   accessibility tree holds no field of the remote one, so a read there would see nothing change
//!   even when the paste arrived.
//!
//! Ids are the probe's, normalised (`crate::foreground::normalize_app_id`): the image name on
//! Windows, the `WM_CLASS` class on Linux, the bundle identifier on macOS.

use crate::HostOs;
use crate::foreground::{LINUX_TERMINALS, WINDOWS_TERMINALS, normalize_app_id};

pub mod rules;

/// macOS terminals by bundle identifier: Terminal, iTerm2, Warp, kitty, Alacritty, WezTerm,
/// Ghostty, Hyper and Tabby.
pub const MACOS_TERMINALS: [&str; 10] = [
    "com.apple.terminal",
    "com.googlecode.iterm2",
    "dev.warp.warp-stable",
    "net.kovidgoyal.kitty",
    "org.alacritty",
    "io.alacritty",
    "com.github.wez.wezterm",
    "com.mitchellh.ghostty",
    "co.zeit.hyper",
    "org.tabby",
];

/// Windows remote-desktop and virtual-machine clients by image name: Remote Desktop Connection
/// (`mstsc`) and the Remote Desktop client (`msrdc`, `msrdcw`), AnyDesk, TeamViewer, Parsec,
/// RustDesk, VNC viewers, VMware Workstation and Player, VirtualBox, Hyper-V's console and the
/// Citrix viewers.
pub const WINDOWS_REMOTE: [&str; 15] = [
    "mstsc",
    "msrdc",
    "msrdcw",
    "anydesk",
    "teamviewer",
    "parsecd",
    "rustdesk",
    "vncviewer",
    "tvnviewer",
    "vmware",
    "vmplayer",
    "virtualboxvm",
    "vmconnect",
    "wfica32",
    "cdviewer",
];

/// macOS remote-desktop and virtual-machine clients by bundle identifier: Microsoft's Remote
/// Desktop (now Windows App), Screen Sharing, AnyDesk, TeamViewer, Parsec, RustDesk, VNC Viewer,
/// VMware Fusion, VirtualBox, Parallels, UTM and Citrix Viewer.
pub const MACOS_REMOTE: [&str; 12] = [
    "com.microsoft.rdc.macos",
    "com.apple.screensharing",
    "com.philandro.anydesk",
    "com.teamviewer.teamviewer",
    "tv.parsec.www",
    "com.carriez.rustdesk",
    "com.realvnc.vncviewer",
    "com.vmware.fusion",
    "org.virtualbox.app.virtualboxvm",
    "com.parallels.desktop.console",
    "com.utmapp.utm",
    "com.citrix.receiver.icaviewer.mac",
];

/// Linux remote-desktop and virtual-machine clients by `WM_CLASS` class: Remmina, FreeRDP, GNOME
/// Connections, AnyDesk, TeamViewer, Parsec, RustDesk, VNC viewers, virt-viewer and
/// virt-manager, GNOME Boxes, VMware, VirtualBox and Citrix.
pub const LINUX_REMOTE: [&str; 17] = [
    "org.remmina.remmina",
    "remmina",
    "xfreerdp",
    "wlfreerdp",
    "org.gnome.connections",
    "anydesk",
    "teamviewer",
    "parsecd",
    "rustdesk",
    "vncviewer",
    "remote-viewer",
    "virt-manager",
    "org.gnome.boxes",
    "vmware",
    "virtualbox machine",
    "virtualboxvm",
    "wfica",
];

/// The applications on `os` whose fields the insert check does not read: its terminals, then its
/// remote-desktop and virtual-machine clients.
pub fn skipped_apps(os: HostOs) -> impl Iterator<Item = &'static str> {
    let (terminals, remote): (&'static [&'static str], &'static [&'static str]) = match os {
        HostOs::Windows => (&WINDOWS_TERMINALS, &WINDOWS_REMOTE),
        HostOs::Linux => (&LINUX_TERMINALS, &LINUX_REMOTE),
        HostOs::Macos => (&MACOS_TERMINALS, &MACOS_REMOTE),
        HostOs::Other => (&[], &[]),
    };
    terminals.iter().chain(remote).copied()
}

/// `app_id` (as the probe reports it, normalised again here) is one whose field the insert check
/// does not read on `os`.
pub fn skips(os: HostOs, app_id: &str) -> bool {
    let id = normalize_app_id(app_id);
    !id.is_empty() && skipped_apps(os).any(|skipped| skipped == id)
}

/// `app_id` (as the probe reports it, normalised again here) is a terminal on `os`, macOS's
/// included: its selection is program output, so 从选中文字学习 copies nothing there
/// (docs/dictation.md §35). The voice edit's table (`foreground::terminal_ids`) has none on macOS,
/// where Cmd+C is safe; the text is still not a dictation.
pub fn holds_program_output(os: HostOs, app_id: &str) -> bool {
    let id = normalize_app_id(app_id);
    let terminals: &[&str] = match os {
        HostOs::Windows => &WINDOWS_TERMINALS,
        HostOs::Linux => &LINUX_TERMINALS,
        HostOs::Macos => &MACOS_TERMINALS,
        HostOs::Other => &[],
    };
    !id.is_empty() && terminals.contains(&id.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression (goal review of PR #260, 2026-11-18): 从选中文字学习 asked the voice edit's
    /// table, which has no macOS terminal, so Terminal's selection was copied. Its own table names
    /// every host's terminals and nothing else.
    #[test]
    fn regression_every_hosts_terminals_hold_program_output() {
        for (os, id) in [
            (HostOs::Macos, "com.apple.Terminal"),
            (HostOs::Macos, "com.googlecode.iterm2"),
            (HostOs::Windows, "WindowsTerminal.exe"),
            (HostOs::Linux, "gnome-terminal-server"),
        ] {
            assert!(holds_program_output(os, id), "{os:?} {id}");
        }
        assert!(!crate::foreground::is_terminal(HostOs::Macos, "com.apple.Terminal"), "the voice edit still allows macOS's terminals");
        for (os, id) in [
            (HostOs::Macos, "com.apple.TextEdit"),
            (HostOs::Windows, "notepad.exe"),
            (HostOs::Macos, "com.microsoft.rdc.macos"),
            (HostOs::Other, "xterm"),
            (HostOs::Linux, ""),
        ] {
            assert!(!holds_program_output(os, id), "{os:?} {id}");
        }
    }

    /// docs/dictation.md §28: terminals and remote screens are never read, on every host; an
    /// ordinary app is.
    #[test]
    fn terminals_and_remote_screens_are_skipped_and_ordinary_apps_are_read() {
        assert!(skips(HostOs::Windows, "MSTSC.EXE"), "normalised again");
        assert!(skips(HostOs::Windows, "WindowsTerminal"), "the voice edit's table");
        assert!(skips(HostOs::Windows, "vmware"));
        assert!(skips(HostOs::Linux, "gnome-terminal-server"));
        assert!(skips(HostOs::Linux, "org.remmina.Remmina"));
        assert!(skips(HostOs::Macos, "com.apple.Terminal"), "macOS terminals too, unlike the voice edit");
        assert!(skips(HostOs::Macos, "com.microsoft.rdc.macos"));
        for (os, app) in [
            (HostOs::Windows, "notepad"),
            (HostOs::Windows, "msedge"),
            (HostOs::Linux, "gedit"),
            (HostOs::Macos, "com.apple.textedit"),
            (HostOs::Macos, "com.apple.safari"),
        ] {
            assert!(!skips(os, app), "{app}");
        }
        assert!(!skips(HostOs::Other, "mstsc"), "a host without a reader skips nothing");
        assert!(!skips(HostOs::Windows, "  "), "no id, nothing to match");
    }

    #[test]
    fn every_table_is_stored_normalised_and_without_repeats() {
        for os in [HostOs::Windows, HostOs::Linux, HostOs::Macos] {
            let ids: Vec<&str> = skipped_apps(os).collect();
            for id in &ids {
                assert_eq!(normalize_app_id(id), *id, "{os:?}: {id}");
            }
            let unique: std::collections::BTreeSet<&str> = ids.iter().copied().collect();
            assert_eq!(unique.len(), ids.len(), "{os:?}: an id listed twice");
        }
        assert_eq!(skipped_apps(HostOs::Other).count(), 0);
    }
}
