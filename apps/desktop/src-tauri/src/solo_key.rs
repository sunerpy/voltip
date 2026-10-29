//! The lone-key trigger (docs/dictation.md §13.1): `Settings.solo_key` names a right-hand
//! modifier, Fn or a mouse button that drives a take on its own, next to the chord. The
//! global-shortcut plugin registers chords only, so this module watches the key through the
//! platform's low-level input hook (the `voltip-hooks` crate, whose real-hook tests CI runs on
//! Windows, macOS and Xvfb) and forwards its press / release / chorded edges:
//!
//! - Windows: `WH_KEYBOARD_LL` and `WH_MOUSE_LL` on a thread of their own. A mouse trigger is
//!   swallowed (a held back button would navigate the browser back); a modifier is passed on, and
//!   an unassigned key is tapped while it is held so Windows sees no lone Alt / Win / Shift.
//! - macOS: an active `CGEventTap`, which needs the Accessibility permission the paste already
//!   needs. The mouse trigger is dropped, keys are passed on.
//! - Linux: XInput 2 raw events on the root window (they arrive whichever window has the focus)
//!   and, for a mouse trigger, a passive button grab that keeps the button from other clients.
//!   XWayland only sees input over X11 windows; a pure Wayland session has no hook at all.
//!
//! Only hardware input counts: Voltip's own paste and copy chords and other synthetic input are
//! ignored, so a paste never looks like a chord.
//!
//! The hooks keep themselves alive (docs/dictation.md §29): Windows puts back a hook the system
//! removed, and does so again after a wake and an unlock; macOS installs a disabled tap again and
//! keeps trying while it is refused; X11 reconnects. Each reports [`SoloEdge::Recovered`] /
//! [`SoloEdge::Lost`], which only the status hears. A hook whose thread ended anyway is
//! [`SoloHook::is_alive`] `false`, and the next settings sync installs a new one.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;

use voltip_core::SoloKey;
use voltip_inject::{SessionKind, X11Grab};
use voltip_platform::solo_key::{SoloEdge, SoloLoss};

/// How the settings page and the log name a key (the core stores the wire name).
pub fn label(key: SoloKey, os: &str) -> &'static str {
    match (key, os) {
        (SoloKey::RightCtrl, _) => "右 Ctrl",
        (SoloKey::RightAlt, "macos") => "右 Option",
        (SoloKey::RightAlt, _) => "右 Alt",
        (SoloKey::RightShift, _) => "右 Shift",
        (SoloKey::RightMeta, "macos") => "右 Command",
        (SoloKey::RightMeta, "windows") => "右 Win",
        (SoloKey::RightMeta, _) => "右 Super",
        (SoloKey::Fn, _) => "Fn",
        (SoloKey::MouseMiddle, _) => "鼠标中键",
        (SoloKey::MouseBack, _) => "鼠标后退键",
        (SoloKey::MouseForward, _) => "鼠标前进键",
    }
}

/// The keys this binary can watch on `os` in `session`: none on a pure Wayland session (no input
/// hook reaches other applications there) and none where no backend exists.
pub fn available(os: &str, session: Option<SessionKind>) -> Vec<SoloKey> {
    let host = match os {
        "macos" => voltip_platform::HostOs::Macos,
        "windows" => voltip_platform::HostOs::Windows,
        "linux" if session.map(SessionKind::x11_grab) != Some(X11Grab::Unavailable) => voltip_platform::HostOs::Linux,
        _ => voltip_platform::HostOs::Other,
    };
    SoloKey::available_on(host)
}

/// Why `key` cannot be watched on `os` in `session` (`HotkeyStatus.solo_error`), or `None`.
pub fn unavailable(key: SoloKey, os: &str, session: Option<SessionKind>) -> Option<String> {
    if available(os, session).contains(&key) {
        return None;
    }
    let name = label(key, os);
    Some(match (key, os) {
        (SoloKey::Fn, "windows" | "linux") => format!("{name} 只在 macOS 上可用：PC 键盘的 Fn 键不经过系统"),
        (_, "linux") => format!("{name} 无法单独触发：纯 Wayland 会话不允许应用监听全局按键，请改用组合键"),
        _ => format!("{name} 无法单独触发：这个平台没有全局输入钩子"),
    })
}

/// `solo_error` while the hook of `key` is gone and being brought back (docs/dictation.md §29).
pub fn lost_reason(key: SoloKey, loss: SoloLoss, os: &str) -> String {
    let name = label(key, os);
    match loss {
        SoloLoss::Hook => format!("{name} 暂时无法单独触发：系统移除了输入钩子，Voltip 正在重新安装"),
        SoloLoss::Accessibility => format!("{name} 无法单独触发：需要在「系统设置 › 隐私与安全性 › 辅助功能」里重新允许 Voltip，允许后自动恢复"),
        SoloLoss::Display => format!("{name} 暂时无法单独触发：与 X 服务器的连接断开了，正在重新连接"),
    }
}

/// A running input hook. Dropping it removes the hook and joins its threads.
pub struct SoloHook {
    key: SoloKey,
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    backend: Option<voltip_hooks::Backend>,
    forwarder: Option<JoinHandle<()>>,
    /// The forwarder still runs: the hook's thread has not ended.
    alive: Arc<AtomicBool>,
}

impl SoloHook {
    /// The key it watches.
    pub fn key(&self) -> SoloKey {
        self.key
    }

    /// Whether the hook's thread still runs. One that ended (it should not: every backend brings
    /// its hook back by itself) is replaced at the next settings sync.
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    /// The computer woke or the session became active again (macOS, docs/dictation.md §29): make
    /// sure the hook is installed and enabled. Windows does this on its hook thread, X11 needs
    /// nothing.
    pub fn refresh(&self) {
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        if let Some(backend) = &self.backend {
            backend.refresh();
        }
    }
}

impl Drop for SoloHook {
    fn drop(&mut self) {
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        if let Some(backend) = self.backend.take() {
            // Stopping the hook drops its sender, which ends the forwarder's loop.
            backend.stop();
        }
        if let Some(forwarder) = self.forwarder.take() {
            let _ = forwarder.join();
        }
    }
}

/// Watch `key` in `session`; every edge goes to `sink`, called on a thread of the hook's own (never
/// the hook's: a slow sink must not delay the system's input). `synthetic` also counts injected
/// input, which only the tests want. The error is the `solo_error` text.
pub fn watch(key: SoloKey, session: Option<SessionKind>, synthetic: bool, sink: impl Fn(SoloEdge) + Send + 'static) -> Result<SoloHook, String> {
    if let Some(reason) = unavailable(key, std::env::consts::OS, session) {
        return Err(reason);
    }
    let (tx, rx) = mpsc::channel::<SoloEdge>();
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    let backend = voltip_hooks::start(key, tx, synthetic).map_err(|e| format!("{} 无法单独触发：{e}", label(key, std::env::consts::OS)))?;
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (tx, synthetic);
    }
    let alive = Arc::new(AtomicBool::new(true));
    let running = alive.clone();
    let forwarder = std::thread::Builder::new()
        .name("voltip-solo-key".into())
        .spawn(move || {
            for edge in rx {
                sink(edge);
            }
            running.store(false, Ordering::SeqCst);
        })
        .map_err(|e| format!("{} 无法单独触发：{e}", label(key, std::env::consts::OS)))?;
    Ok(SoloHook {
        key,
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        backend: Some(backend),
        forwarder: Some(forwarder),
        alive,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_platform_names_its_keys_and_says_why_one_is_missing() {
        assert_eq!(available("windows", None).len(), 7);
        assert!(available("macos", None).contains(&SoloKey::Fn));
        assert_eq!(available("linux", Some(SessionKind::X11)), available("windows", None));
        assert_eq!(available("linux", Some(SessionKind::XWayland)), available("windows", None));
        assert!(available("linux", Some(SessionKind::Wayland)).is_empty());
        assert!(available("android", None).is_empty());
        assert_eq!(unavailable(SoloKey::RightCtrl, "windows", None), None);
        assert!(unavailable(SoloKey::Fn, "windows", None).is_some_and(|e| e.contains("macOS")));
        assert!(unavailable(SoloKey::RightCtrl, "linux", Some(SessionKind::Wayland)).is_some_and(|e| e.contains("纯 Wayland")));
        assert_eq!(label(SoloKey::RightMeta, "macos"), "右 Command");
        assert_eq!(label(SoloKey::RightMeta, "windows"), "右 Win");
        assert_eq!(label(SoloKey::RightAlt, "macos"), "右 Option");
        for key in SoloKey::ALL {
            for os in ["windows", "macos", "linux"] {
                assert!(!label(key, os).is_empty());
            }
        }
    }

    /// docs/dictation.md §29: while a hook is being brought back, `solo_error` says which key, why
    /// and that it comes back by itself.
    #[test]
    fn a_lost_hook_says_why_and_that_it_is_coming_back() {
        let hook = lost_reason(SoloKey::RightCtrl, SoloLoss::Hook, "windows");
        assert!(hook.starts_with("右 Ctrl ") && hook.contains("重新安装"), "{hook}");
        let tap = lost_reason(SoloKey::Fn, SoloLoss::Accessibility, "macos");
        assert!(tap.starts_with("Fn ") && tap.contains("辅助功能") && tap.contains("自动恢复"), "{tap}");
        let x = lost_reason(SoloKey::MouseBack, SoloLoss::Display, "linux");
        assert!(x.starts_with("鼠标后退键 ") && x.contains("重新连接"), "{x}");
    }
}
