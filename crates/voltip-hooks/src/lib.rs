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
//! Each backend has an `#[ignore]`d test that installs the real hook and feeds it synthetic input;
//! CI runs them on a Windows Server desktop, a macOS session and Xvfb.

#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::time::Duration;

pub use voltip_platform::solo_key::{SoloEdge, SoloKey};

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
pub use backend::{Backend, start};

/// Whether this build has a hook backend at all.
pub const HAS_BACKEND: bool = cfg!(any(target_os = "macos", target_os = "windows", target_os = "linux"));

/// How long a hook may take to install before the attempt counts as failed.
#[cfg(any(target_os = "macos", target_os = "windows"))]
const START_TIMEOUT: Duration = Duration::from_secs(5);
