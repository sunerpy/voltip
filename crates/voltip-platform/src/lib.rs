//! Platform policy for the Voltip shells (docs/dictation.md §15): the **decisions** the desktop
//! shell makes on macOS and Windows, written as pure functions and tables so they compile and are
//! unit-tested on every host, including the Linux CI that can neither prompt for a macOS
//! permission nor open a Windows token.
//!
//! The crate never talks to the operating system. The shell (`apps/desktop/src-tauri/src/platform/`)
//! collects the raw facts (a TCC answer, a token integrity RID, a registry value) and feeds them
//! into the tables here; the leaf crate is what `cargo check --target aarch64-apple-darwin` and
//! `--target x86_64-pc-windows-msvc` verify in `scripts/verify-all.sh` without a C toolchain.
//!
//! - [`permissions`]: [`Permission`] / [`PermissionState`] / [`PermissionReport`] (the
//!   `permissions_status` wire), the onboarding gate and the 1 s / 3-error poll plan.
//! - [`macos`]: activation policy timing (`start_hidden × tray_available`), the Cmd+V and Cmd+C
//!   keycode fallbacks and the modifier hold.
//! - [`windows`]: integrity-level comparison for the injection preflight (`inject_preflight`
//!   wire) and the microphone `ConsentStore` key priority.
//! - [`tray`]: the three-state tray glyph, rendered to RGBA so no icon asset is needed.
//! - [`foreground`]: the focused application's raw facts (Windows image path, X11 `WM_CLASS`,
//!   macOS bundle id) → the normalised app id scenes match on (docs/dictation.md §18).
//! - [`solo_key`]: the lone-key trigger (docs/dictation.md §13.1): the keys each platform offers,
//!   the codes its input hook reports them with, and raw input → press / release / chorded.
//! - [`HostOs`]: which platform this binary was compiled for; [`PermissionReport::for_host`] is
//!   the `not_applicable` skeleton the shell starts from.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod foreground;
pub mod macos;
pub mod permissions;
pub mod solo_key;
pub mod tray;
pub mod windows;

pub use permissions::{Permission, PermissionReport, PermissionState, PollPlan, PollStep, Poller, onboarding_gate};
pub use windows::{InjectDecision, InjectPreflight, IntegrityLevel};

/// The operating system this binary was compiled for, as the wire names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostOs {
    /// macOS: TCC governs the microphone and Accessibility.
    Macos,
    /// Windows: the microphone consent store; injection is subject to UIPI.
    Windows,
    /// Linux: no permission prompts (the desktop portal is a later phase).
    Linux,
    /// Anything else (Android / iOS shells, BSDs): nothing to ask.
    Other,
}

impl HostOs {
    /// The host this crate was compiled for.
    pub const fn current() -> Self {
        #[cfg(target_os = "macos")]
        {
            Self::Macos
        }
        #[cfg(target_os = "windows")]
        {
            Self::Windows
        }
        #[cfg(target_os = "linux")]
        {
            Self::Linux
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            Self::Other
        }
    }

    /// Wire name (`macos` / `windows` / `linux` / `other`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Macos => "macos",
            Self::Windows => "windows",
            Self::Linux => "linux",
            Self::Other => "other",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_os_names_the_compiled_target() {
        let host = HostOs::current();
        if cfg!(target_os = "linux") {
            assert_eq!(host, HostOs::Linux);
        } else if cfg!(target_os = "macos") {
            assert_eq!(host, HostOs::Macos);
        } else if cfg!(target_os = "windows") {
            assert_eq!(host, HostOs::Windows);
        } else {
            assert_eq!(host, HostOs::Other);
        }
        assert_eq!(serde_json::to_string(&host).unwrap(), format!("\"{}\"", host.as_str()));
        for os in [HostOs::Macos, HostOs::Windows, HostOs::Linux, HostOs::Other] {
            let json = serde_json::to_string(&os).unwrap();
            assert_eq!(json.trim_matches('"'), os.as_str());
            assert_eq!(serde_json::from_str::<HostOs>(&json).unwrap(), os);
        }
    }
}
