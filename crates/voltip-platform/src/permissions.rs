//! Permission model shared by the three shells: what the OS can grant, what the onboarding step
//! must wait for, and how the webview polls (docs/dictation.md §15.1).

use serde::{Deserialize, Serialize};

use crate::HostOs;

/// A capability the operating system gates behind a user decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    /// Recording (`NSMicrophoneUsageDescription` / the Windows microphone consent store).
    Microphone,
    /// macOS Accessibility (`AXIsProcessTrusted`): the paste chord and focused-field lookup.
    Accessibility,
}

impl Permission {
    /// Every permission, in the order the onboarding table lists them. The hotkey needs none: the
    /// global shortcut is a registered chord (Carbon hotkeys on macOS), not a key-event tap.
    pub const ALL: [Self; 2] = [Self::Microphone, Self::Accessibility];

    /// Dictation cannot work at all without it (the microphone hears, Accessibility delivers).
    pub const fn required(self) -> bool {
        matches!(self, Self::Microphone | Self::Accessibility)
    }

    /// Wire name (`microphone` / `accessibility`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Microphone => "microphone",
            Self::Accessibility => "accessibility",
        }
    }
}

/// What the OS currently says about one [`Permission`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionState {
    /// Granted; nothing to do.
    Granted,
    /// Refused (or, for the boolean macOS APIs, "not in the list"): the user must act in System
    /// Settings, a request from the app cannot flip it.
    Denied,
    /// Never asked: a request from the app will show the system prompt.
    NotDetermined,
    /// This platform has no such gate (Linux, or Accessibility on Windows).
    NotApplicable,
}

impl PermissionState {
    /// Nothing blocks on it: granted, or the platform never asks.
    pub const fn satisfied(self) -> bool {
        matches!(self, Self::Granted | Self::NotApplicable)
    }

    /// The boolean answers of `AXIsProcessTrusted`, `IOHIDCheckAccess` and the plugin's
    /// microphone check: `true` is granted; `false` cannot distinguish "never asked" from "refused",
    /// and is reported as [`Self::Denied`] so the onboarding step asks rather than assumes.
    pub const fn from_flag(granted: bool) -> Self {
        if granted { Self::Granted } else { Self::Denied }
    }

    /// Wire name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Granted => "granted",
            Self::Denied => "denied",
            Self::NotDetermined => "not_determined",
            Self::NotApplicable => "not_applicable",
        }
    }
}

/// The `permissions_status` answer: one state per permission plus the host it was measured on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionReport {
    /// Host the shell was compiled for (`macos` / `windows` / `linux` / `other`).
    pub platform: HostOs,
    /// Microphone.
    pub microphone: PermissionState,
    /// Accessibility (macOS only; `not_applicable` elsewhere).
    pub accessibility: PermissionState,
}

impl PermissionReport {
    /// Every permission `not_applicable`: the Linux / other answer, and the base every host starts from.
    pub const fn not_applicable(platform: HostOs) -> Self {
        Self { platform, microphone: PermissionState::NotApplicable, accessibility: PermissionState::NotApplicable }
    }

    /// The skeleton for the compiled host before any OS query: what the shell reports when a
    /// query fails, and what the query fills in. macOS asks for both, Windows only for the
    /// microphone, everything else asks nothing.
    pub const fn for_host() -> Self {
        Self::skeleton(HostOs::current())
    }

    /// [`Self::for_host`] for an explicit host (the table [`Self::for_host`] reads).
    pub const fn skeleton(platform: HostOs) -> Self {
        let base = Self::not_applicable(platform);
        match platform {
            HostOs::Macos => Self { microphone: PermissionState::NotDetermined, accessibility: PermissionState::NotDetermined, ..base },
            HostOs::Windows => Self { microphone: PermissionState::NotDetermined, ..base },
            HostOs::Linux | HostOs::Other => base,
        }
    }

    /// State of one permission.
    pub const fn get(&self, permission: Permission) -> PermissionState {
        match permission {
            Permission::Microphone => self.microphone,
            Permission::Accessibility => self.accessibility,
        }
    }

    /// Replace one state (builder style, for the shell's per-permission queries).
    #[must_use]
    pub const fn with(mut self, permission: Permission, state: PermissionState) -> Self {
        match permission {
            Permission::Microphone => self.microphone = state,
            Permission::Accessibility => self.accessibility = state,
        }
        self
    }

    /// `true` when this platform has nothing to grant (the onboarding step says so and moves on).
    pub fn nothing_to_grant(&self) -> bool {
        Permission::ALL.iter().all(|p| self.get(*p) == PermissionState::NotApplicable)
    }
}

/// Permissions the onboarding step must settle before "continue" is offered, in table order.
///
/// | permission | state | blocks |
/// |---|---|---|
/// | Microphone | `denied` | yes: a recording would fail on the first press |
/// | Microphone | `not_determined` | no: the OS prompts on the first recording (Windows allows desktop apps by default) |
/// | Accessibility | `denied` \| `not_determined` | yes: macOS never prompts by itself and the injector needs it |
/// | any | `granted` \| `not_applicable` | no |
pub fn onboarding_gate(report: &PermissionReport) -> Vec<Permission> {
    Permission::ALL.iter().copied().filter(|p| blocks(*p, report.get(*p))).collect()
}

const fn blocks(permission: Permission, state: PermissionState) -> bool {
    match (permission, state) {
        (_, PermissionState::Granted | PermissionState::NotApplicable) => false,
        (Permission::Microphone, PermissionState::Denied) => true,
        (Permission::Microphone, PermissionState::NotDetermined) => false,
        (Permission::Accessibility, PermissionState::Denied | PermissionState::NotDetermined) => true,
    }
}

/// How the onboarding step re-reads `permissions_status` while it is on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PollPlan {
    /// Time between two queries.
    pub interval_ms: u64,
    /// Consecutive failed queries after which polling stops and the step shows the error.
    pub max_consecutive_errors: u32,
}

impl PollPlan {
    /// The documented plan: every second, give up after three consecutive errors.
    pub const DEFAULT: Self = Self { interval_ms: 1_000, max_consecutive_errors: 3 };
}

impl Default for PollPlan {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// What the poller wants after one query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PollStep {
    /// Schedule the next query after `next_in_ms`.
    Continue {
        /// Delay until the next query.
        next_in_ms: u64,
    },
    /// Stop: `errors` consecutive queries failed. A manual "check again" calls [`Poller::resume`].
    Stopped {
        /// The consecutive error count that tripped the plan.
        errors: u32,
    },
}

/// The poll state machine the webview mirrors: a success resets the error streak, an error extends
/// it, and the streak reaching the plan's limit stops the loop until resumed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Poller {
    plan: PollPlan,
    consecutive_errors: u32,
    stopped: bool,
}

impl Poller {
    /// A running poller with zero errors.
    pub const fn new(plan: PollPlan) -> Self {
        Self { plan, consecutive_errors: 0, stopped: false }
    }

    /// One successful query.
    pub fn on_success(&mut self) -> PollStep {
        self.consecutive_errors = 0;
        self.step()
    }

    /// One failed query.
    pub fn on_error(&mut self) -> PollStep {
        self.consecutive_errors = self.consecutive_errors.saturating_add(1);
        if self.consecutive_errors >= self.plan.max_consecutive_errors {
            self.stopped = true;
        }
        self.step()
    }

    /// The user asked to check again: forget the streak and run.
    pub fn resume(&mut self) -> PollStep {
        self.consecutive_errors = 0;
        self.stopped = false;
        self.step()
    }

    /// Whether the loop has given up.
    pub const fn is_stopped(&self) -> bool {
        self.stopped
    }

    /// The current error streak.
    pub const fn consecutive_errors(&self) -> u32 {
        self.consecutive_errors
    }

    const fn step(&self) -> PollStep {
        if self.stopped { PollStep::Stopped { errors: self.consecutive_errors } } else { PollStep::Continue { next_in_ms: self.plan.interval_ms } }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use PermissionState::{Denied, Granted, NotApplicable, NotDetermined};

    fn report(mic: PermissionState, ax: PermissionState) -> PermissionReport {
        PermissionReport { platform: HostOs::Macos, microphone: mic, accessibility: ax }
    }

    #[test]
    fn wire_names_are_snake_case_and_round_trip() {
        let r = report(Granted, Denied);
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json, serde_json::json!({ "platform": "macos", "microphone": "granted", "accessibility": "denied" }));
        assert_eq!(serde_json::from_value::<PermissionReport>(json).unwrap(), r);
        for p in Permission::ALL {
            assert_eq!(serde_json::to_string(&p).unwrap().trim_matches('"'), p.as_str());
        }
        for s in [Granted, Denied, NotDetermined, NotApplicable] {
            assert_eq!(serde_json::to_string(&s).unwrap().trim_matches('"'), s.as_str());
            assert_eq!(s.satisfied(), matches!(s, Granted | NotApplicable));
        }
        assert_eq!(PermissionState::from_flag(true), Granted);
        assert_eq!(PermissionState::from_flag(false), Denied);
        assert!(Permission::ALL.iter().all(|p| p.required()));
        // Regression (public release, 2026-09-27): no permission for a trigger the app does not have.
        assert!(serde_json::from_str::<Permission>(r#""input_monitoring""#).is_err());
    }

    #[test]
    fn host_skeletons_ask_only_what_the_platform_gates() {
        let linux = PermissionReport::skeleton(HostOs::Linux);
        assert_eq!(linux, PermissionReport::not_applicable(HostOs::Linux));
        assert!(linux.nothing_to_grant());
        assert!(PermissionReport::skeleton(HostOs::Other).nothing_to_grant());
        let win = PermissionReport::skeleton(HostOs::Windows);
        assert_eq!((win.microphone, win.accessibility), (NotDetermined, NotApplicable));
        assert!(!win.nothing_to_grant());
        let mac = PermissionReport::skeleton(HostOs::Macos);
        assert_eq!((mac.microphone, mac.accessibility), (NotDetermined, NotDetermined));
        // The compiled host's skeleton is the one the shell starts from.
        assert_eq!(PermissionReport::for_host(), PermissionReport::skeleton(HostOs::current()));
        // `with` / `get` address the same field.
        let r = mac.with(Permission::Accessibility, Granted);
        assert_eq!(r.get(Permission::Accessibility), Granted);
        assert_eq!(r.get(Permission::Microphone), NotDetermined);
    }

    #[test]
    fn onboarding_gate_table() {
        // (microphone, accessibility) → what blocks.
        type Row = ((PermissionState, PermissionState), &'static [Permission]);
        let table: &[Row] = &[
            ((NotApplicable, NotApplicable), &[]),
            ((Granted, Granted), &[]),
            ((NotDetermined, NotApplicable), &[]),
            ((Denied, NotApplicable), &[Permission::Microphone]),
            ((Granted, Denied), &[Permission::Accessibility]),
            ((Granted, NotDetermined), &[Permission::Accessibility]),
            ((Denied, Denied), &[Permission::Microphone, Permission::Accessibility]),
            ((NotDetermined, NotDetermined), &[Permission::Accessibility]),
        ];
        for ((mic, ax), expected) in table {
            assert_eq!(onboarding_gate(&report(*mic, *ax)), *expected, "mic={mic:?} ax={ax:?}");
        }
    }

    #[test]
    fn poller_stops_after_three_consecutive_errors_and_a_success_resets_the_streak() {
        let mut p = Poller::new(PollPlan::default());
        assert_eq!(PollPlan::default(), PollPlan { interval_ms: 1_000, max_consecutive_errors: 3 });
        let go = PollStep::Continue { next_in_ms: 1_000 };
        assert_eq!(p.on_success(), go);
        assert_eq!(p.on_error(), go);
        assert_eq!(p.on_error(), go);
        assert_eq!(p.consecutive_errors(), 2);
        // A success in between resets the streak: two more errors do not stop it.
        assert_eq!(p.on_success(), go);
        assert_eq!(p.consecutive_errors(), 0);
        assert_eq!(p.on_error(), go);
        assert_eq!(p.on_error(), go);
        assert_eq!(p.on_error(), PollStep::Stopped { errors: 3 });
        assert!(p.is_stopped());
        // Stopped stays stopped: further results do not restart it by themselves.
        assert_eq!(p.on_success(), PollStep::Stopped { errors: 0 });
        assert_eq!(p.on_error(), PollStep::Stopped { errors: 1 });
        // "Check again" resumes with a clean streak.
        assert_eq!(p.resume(), go);
        assert!(!p.is_stopped());
        assert_eq!(p.consecutive_errors(), 0);
    }

    #[test]
    fn poller_honours_a_custom_plan() {
        let mut p = Poller::new(PollPlan { interval_ms: 250, max_consecutive_errors: 1 });
        assert_eq!(p.on_success(), PollStep::Continue { next_in_ms: 250 });
        assert_eq!(p.on_error(), PollStep::Stopped { errors: 1 });
    }
}
