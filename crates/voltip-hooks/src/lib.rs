//! The lone-key trigger's low-level input hooks (docs/dictation.md §13.1). [`start`] watches one
//! [`SoloKey`] and sends what it sees as [`SoloEdge`]s (press / release / chorded, from a
//! `SoloTracker`); [`Backend::stop`] removes the hook. One backend per platform:
//!
//! - Windows: `WH_KEYBOARD_LL` and `WH_MOUSE_LL` on a thread of their own (`windows.rs`).
//! - macOS: an active `CGEventTap` on the session, which needs the Accessibility permission
//!   (`macos.rs`).
//! - Linux: XInput 2 raw events on the root window and a passive grab for a mouse trigger
//!   (`x11.rs`); a pure Wayland session has no hook.
//!
//! Only hardware input counts unless `synthetic` is set (the tests): Voltip's own paste and copy
//! chords never look like a chord. The desktop shell decides which keys a session offers and
//! forwards the edges; this crate only installs, reports and removes.
//!
//! Keeping the hook (docs/dictation.md §29): each backend notices when the system took its hook
//! away and puts it back (Windows: Raw Input against the hooks, waking and unlocking; macOS: a
//! disabled tap; X11: a lost connection), reporting [`SoloEdge::Recovered`], or
//! [`SoloEdge::Lost`] while it cannot. [`is_down`] reads a trigger's physical state, which the
//! shell checks a hold-to-talk take against when a release may have been missed.
//!
//! Each backend has `#[ignore]`d tests that install the real hook and feed it synthetic input;
//! CI runs them on a Windows Server desktop, a macOS session and Xvfb.

#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::time::Duration;

pub use voltip_platform::held_key::HeldKey;
pub use voltip_platform::solo_key::{SoloEdge, SoloKey, SoloLoss};

pub mod watch;

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod backend;
#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod backend;
#[cfg(target_os = "linux")]
#[path = "x11.rs"]
mod backend;

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub use backend::{Backend, is_down, start};

/// Whether `key` is physically down: without a backend, nobody knows.
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub fn is_down(_key: &HeldKey) -> Option<bool> {
    None
}

/// Whether this build has a hook backend at all.
pub const HAS_BACKEND: bool = cfg!(any(target_os = "macos", target_os = "windows", target_os = "linux"));

/// How long a hook may take to install before the attempt counts as failed.
#[cfg(any(target_os = "macos", target_os = "windows"))]
const START_TIMEOUT: Duration = Duration::from_secs(5);
