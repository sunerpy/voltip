//! Windows policy tables (docs/dictation.md §15.3). Compiled on every host so the tables are
//! tested on Linux CI; the shell collects the facts under `#[cfg(target_os = "windows")]`.

use serde::{Deserialize, Serialize};

use crate::HostOs;
use crate::permissions::PermissionState;

/// Mandatory integrity level of a process token (`SECURITY_MANDATORY_*_RID`, the last
/// sub-authority of the `TokenIntegrityLevel` SID). Ordered: a lower level cannot send input to a
/// window of a higher one (UIPI).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrityLevel {
    /// `0x0000`: anonymous / sandboxed to nothing.
    Untrusted,
    /// `0x1000`: AppContainer-like sandboxes (browser renderers).
    Low,
    /// `0x2000`: an ordinary user process — Voltip itself when not elevated.
    Medium,
    /// `0x2100`: medium plus (rare; some UAC-aware processes).
    MediumPlus,
    /// `0x3000`: elevated ("Run as administrator").
    High,
    /// `0x4000`: services / SYSTEM.
    System,
    /// `0x5000`: protected processes (anti-malware).
    ProtectedProcess,
}

impl IntegrityLevel {
    /// Bucket a RID into a level: Windows documents the exact values, but any RID inside a band
    /// (`0x2000..0x2100` is medium) means that band.
    pub const fn from_rid(rid: u32) -> Self {
        match rid {
            0..0x1000 => Self::Untrusted,
            0x1000..0x2000 => Self::Low,
            0x2000..0x2100 => Self::Medium,
            0x2100..0x3000 => Self::MediumPlus,
            0x3000..0x4000 => Self::High,
            0x4000..0x5000 => Self::System,
            _ => Self::ProtectedProcess,
        }
    }
}

/// What the shell could learn about the foreground window before an injection.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ForegroundFacts {
    /// Voltip's own token level (`None`: `OpenProcessToken` on ourselves failed).
    pub self_level: Option<IntegrityLevel>,
    /// The foreground window's process level (`None`: no foreground window, or `OpenProcess` /
    /// `GetTokenInformation` refused — which is itself what happens for protected processes).
    pub target_level: Option<IntegrityLevel>,
    /// Whether the input desktop is not the interactive `Default` desktop (`Winlogon` for UAC
    /// prompts and the lock screen, `Screen-saver`). `None`: `OpenInputDesktop` failed.
    pub secure_desktop: Option<bool>,
    /// File name of the foreground process image, for the UI.
    pub target_process: Option<String>,
}

/// Whether the injector should go ahead.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InjectDecision {
    /// Same or lower integrity target on the interactive desktop: `SendInput` / the paste chord will land.
    Proceed,
    /// The foreground window belongs to a higher-integrity process: UIPI drops synthetic input,
    /// the text stays in the clipboard and the history entry should say so.
    ElevatedTarget,
    /// A UAC prompt, the lock screen or the screen saver own the input: nothing can be pasted.
    SecureDesktop,
    /// One of the facts is missing; the injector proceeds and reports the outcome honestly.
    Unknown,
}

/// Decision table:
///
/// | secure desktop | self | target | decision |
/// |---|---|---|---|
/// | `Some(true)` | any | any | `SecureDesktop` |
/// | not `Some(true)` | `Some(s)` | `Some(t)`, `t > s` | `ElevatedTarget` |
/// | not `Some(true)` | `Some(s)` | `Some(t)`, `t <= s` | `Proceed` |
/// | not `Some(true)` | `None` or | `None` | `Unknown` |
pub fn decide_inject(facts: &ForegroundFacts) -> InjectDecision {
    if facts.secure_desktop == Some(true) {
        return InjectDecision::SecureDesktop;
    }
    match (facts.self_level, facts.target_level) {
        (Some(own), Some(target)) if target > own => InjectDecision::ElevatedTarget,
        (Some(_), Some(_)) => InjectDecision::Proceed,
        _ => InjectDecision::Unknown,
    }
}

/// The `inject_preflight` answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InjectPreflight {
    /// Host the check ran on.
    pub platform: HostOs,
    /// `true` when the host performs this check (Windows). Elsewhere the decision is always
    /// `proceed` and the other fields are `null`.
    pub checked: bool,
    /// The decision.
    pub decision: InjectDecision,
    /// Foreground process image name (`explorer.exe`), when known.
    pub target_process: Option<String>,
    /// Voltip's own integrity level, when known.
    pub self_level: Option<IntegrityLevel>,
    /// The foreground process's integrity level, when known.
    pub target_level: Option<IntegrityLevel>,
}

impl InjectPreflight {
    /// The answer on a host without UIPI (macOS, Linux): nothing checked, proceed.
    pub const fn not_applicable(platform: HostOs) -> Self {
        Self { platform, checked: false, decision: InjectDecision::Proceed, target_process: None, self_level: None, target_level: None }
    }

    /// The answer from a set of Windows facts.
    pub fn from_facts(facts: ForegroundFacts) -> Self {
        let decision = decide_inject(&facts);
        Self {
            platform: HostOs::Windows,
            checked: true,
            decision,
            target_process: facts.target_process,
            self_level: facts.self_level,
            target_level: facts.target_level,
        }
    }
}

/// A value of the Windows `CapabilityAccessManager\ConsentStore` (`Value` = `Allow` / `Deny` /
/// `Prompt`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsentValue {
    /// `Allow`.
    Allow,
    /// `Deny`.
    Deny,
    /// `Prompt` (ask; desktop apps are never actually prompted, so it inherits).
    Prompt,
}

impl ConsentValue {
    /// Parse the registry string (case-insensitive); anything else is unknown (`None`).
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "allow" => Some(Self::Allow),
            "deny" => Some(Self::Deny),
            "prompt" => Some(Self::Prompt),
            _ => None,
        }
    }
}

/// `HKLM\SOFTWARE\Policies\Microsoft\Windows\AppPrivacy\LetAppsAccessMicrophone` (group policy).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MicrophonePolicy {
    /// `0`: the user decides (the consent store applies).
    UserControl,
    /// `1`: force allow.
    ForceAllow,
    /// `2`: force deny.
    ForceDeny,
}

impl MicrophonePolicy {
    /// From the policy DWORD; other values are unknown (`None`).
    pub const fn from_dword(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::UserControl),
            1 => Some(Self::ForceAllow),
            2 => Some(Self::ForceDeny),
            _ => None,
        }
    }
}

/// The registry facts about microphone consent, most general first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MicrophoneConsent {
    /// Group policy (`None`: not configured).
    pub policy: Option<MicrophonePolicy>,
    /// `HKCU\…\ConsentStore\microphone` `Value`: the "Microphone access" switch.
    pub global: Option<ConsentValue>,
    /// `HKCU\…\ConsentStore\microphone\NonPackaged` `Value`: the "Let desktop apps access your
    /// microphone" switch.
    pub non_packaged: Option<ConsentValue>,
    /// `HKCU\…\ConsentStore\microphone\NonPackaged\<exe path with # for \>` `Value`: this
    /// executable's own row, present once it has recorded at least once.
    pub app: Option<ConsentValue>,
}

/// Key priority (first match wins):
///
/// | fact | value | result |
/// |---|---|---|
/// | policy | force deny | `denied` |
/// | policy | force allow | `granted` |
/// | global | `Deny` | `denied` |
/// | non_packaged | `Deny` | `denied` |
/// | app | `Deny` | `denied` |
/// | app | `Allow` | `granted` |
/// | non_packaged | `Allow` | `granted` (the app row inherits) |
/// | global | `Allow` with `non_packaged` unreadable | `granted` |
/// | otherwise (everything missing or `Prompt`) | | `not_determined` |
pub fn microphone_consent(facts: &MicrophoneConsent) -> PermissionState {
    use ConsentValue::{Allow, Deny};
    match facts.policy {
        Some(MicrophonePolicy::ForceDeny) => return PermissionState::Denied,
        Some(MicrophonePolicy::ForceAllow) => return PermissionState::Granted,
        Some(MicrophonePolicy::UserControl) | None => {}
    }
    if facts.global == Some(Deny) || facts.non_packaged == Some(Deny) || facts.app == Some(Deny) {
        return PermissionState::Denied;
    }
    if facts.app == Some(Allow) || facts.non_packaged == Some(Allow) || (facts.global == Some(Allow) && facts.non_packaged.is_none()) {
        return PermissionState::Granted;
    }
    PermissionState::NotDetermined
}

/// The per-app key name under `NonPackaged`: the full image path with every `\` turned into `#`
/// (`C:#Program Files#Voltip#voltip-desktop.exe`).
pub fn consent_store_app_key(exe_path: &str) -> String {
    exe_path.replace('\\', "#")
}

/// What the shell reads about a top-level window below Voltip's in the Z order, to find the window
/// the history paste goes to (apps/desktop/src-tauri/src/paste.rs).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StackedWindow<'a> {
    /// `IsWindowVisible`.
    pub visible: bool,
    /// `IsIconic`.
    pub minimized: bool,
    /// Width or height 0.
    pub empty: bool,
    /// `WS_EX_TOOLWINDOW` or `WS_EX_NOACTIVATE`: a palette, an IME window, never the one typed into.
    pub tool: bool,
    /// Hidden by DWM: a suspended UWP frame, a window on another virtual desktop.
    pub cloaked: bool,
    /// One of Voltip's own windows.
    pub own_process: bool,
    /// The window class.
    pub class: &'a str,
}

/// The desktop and the taskbar: in the Z order, but not a window anyone pastes into.
const SHELL_CLASSES: [&str; 4] = ["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"];

/// Whether `window` is the one the user came from. Activating a window puts it on top of the
/// others, so the first ordinary application window below Voltip's is the one that was active
/// before Voltip.
pub fn is_paste_target(window: &StackedWindow<'_>) -> bool {
    window.visible && !window.minimized && !window.empty && !window.tool && !window.cloaked && !window.own_process && !SHELL_CLASSES.contains(&window.class)
}

#[cfg(test)]
mod tests {
    use super::*;
    use IntegrityLevel::{High, Low, Medium, MediumPlus, ProtectedProcess, System, Untrusted};

    #[test]
    fn regression_the_paste_goes_to_the_first_ordinary_window_below_voltip() {
        // CI 2026-09-29: minimising Voltip left no other window in front, so the history paste only
        // copied. The shell now brings the window below Voltip to the front itself.
        let notepad = StackedWindow { visible: true, class: "Notepad", ..StackedWindow::default() };
        assert!(is_paste_target(&notepad));
        for (skipped, why) in [
            (StackedWindow { visible: false, ..notepad }, "hidden"),
            (StackedWindow { minimized: true, ..notepad }, "minimised"),
            (StackedWindow { empty: true, ..notepad }, "no size"),
            (StackedWindow { tool: true, ..notepad }, "tool window"),
            (StackedWindow { cloaked: true, ..notepad }, "cloaked"),
            (StackedWindow { own_process: true, ..notepad }, "Voltip's own"),
            (StackedWindow { class: "Progman", ..notepad }, "the desktop"),
            (StackedWindow { class: "Shell_TrayWnd", ..notepad }, "the taskbar"),
        ] {
            assert!(!is_paste_target(&skipped), "{why}");
        }
    }

    #[test]
    fn integrity_levels_bucket_rids_and_order() {
        let table = [
            (0x0000, Untrusted),
            (0x0FFF, Untrusted),
            (0x1000, Low),
            (0x1FFF, Low),
            (0x2000, Medium),
            (0x20FF, Medium),
            (0x2100, MediumPlus),
            (0x2FFF, MediumPlus),
            (0x3000, High),
            (0x3FFF, High),
            (0x4000, System),
            (0x4FFF, System),
            (0x5000, ProtectedProcess),
            (u32::MAX, ProtectedProcess),
        ];
        for (rid, level) in table {
            assert_eq!(IntegrityLevel::from_rid(rid), level, "rid={rid:#x}");
        }
        assert!(Untrusted < Low && Low < Medium && Medium < MediumPlus && MediumPlus < High && High < System && System < ProtectedProcess);
        assert_eq!(serde_json::to_string(&MediumPlus).unwrap(), "\"medium_plus\"");
    }

    #[test]
    fn inject_decision_table() {
        let facts = |own: Option<IntegrityLevel>, target: Option<IntegrityLevel>, secure: Option<bool>| ForegroundFacts {
            self_level: own,
            target_level: target,
            secure_desktop: secure,
            target_process: None,
        };
        let table = [
            (facts(Some(Medium), Some(Medium), Some(false)), InjectDecision::Proceed),
            (facts(Some(Medium), Some(Low), Some(false)), InjectDecision::Proceed),
            (facts(Some(High), Some(Medium), Some(false)), InjectDecision::Proceed),
            (facts(Some(Medium), Some(High), Some(false)), InjectDecision::ElevatedTarget),
            (facts(Some(Medium), Some(System), Some(false)), InjectDecision::ElevatedTarget),
            (facts(Some(Medium), Some(MediumPlus), Some(false)), InjectDecision::ElevatedTarget),
            // Secure desktop wins over everything, even a readable equal target.
            (facts(Some(Medium), Some(Medium), Some(true)), InjectDecision::SecureDesktop),
            (facts(None, None, Some(true)), InjectDecision::SecureDesktop),
            // Missing facts are unknown, not a guess; an unreadable desktop alone does not block.
            (facts(None, Some(Medium), Some(false)), InjectDecision::Unknown),
            (facts(Some(Medium), None, Some(false)), InjectDecision::Unknown),
            (facts(Some(Medium), Some(Medium), None), InjectDecision::Proceed),
            (facts(None, None, None), InjectDecision::Unknown),
        ];
        for (f, expected) in table {
            assert_eq!(decide_inject(&f), expected, "{f:?}");
        }
        let pre = InjectPreflight::from_facts(ForegroundFacts {
            self_level: Some(Medium),
            target_level: Some(High),
            secure_desktop: Some(false),
            target_process: Some("regedit.exe".into()),
        });
        assert_eq!(pre.decision, InjectDecision::ElevatedTarget);
        assert!(pre.checked);
        let json = serde_json::to_value(&pre).unwrap();
        assert_eq!(json["decision"], "elevated_target");
        assert_eq!(json["target_process"], "regedit.exe");
        assert_eq!(json["self_level"], "medium");
        assert_eq!(json["platform"], "windows");
        let na = InjectPreflight::not_applicable(HostOs::Linux);
        assert_eq!((na.checked, na.decision), (false, InjectDecision::Proceed));
        assert_eq!(serde_json::to_value(&na).unwrap()["target_process"], serde_json::Value::Null);
    }

    #[test]
    fn consent_store_key_priority_table() {
        use ConsentValue::{Allow, Deny, Prompt};
        use MicrophonePolicy::{ForceAllow, ForceDeny, UserControl};
        use PermissionState::{Denied, Granted, NotDetermined};
        let f = |policy, global, non_packaged, app| MicrophoneConsent { policy, global, non_packaged, app };
        let table = [
            // Policy forces both ways, whatever the user rows say.
            (f(Some(ForceDeny), Some(Allow), Some(Allow), Some(Allow)), Denied),
            (f(Some(ForceAllow), Some(Deny), Some(Deny), Some(Deny)), Granted),
            (f(Some(UserControl), Some(Allow), Some(Allow), None), Granted),
            // A Deny anywhere in the chain denies.
            (f(None, Some(Deny), Some(Allow), Some(Allow)), Denied),
            (f(None, Some(Allow), Some(Deny), Some(Allow)), Denied),
            (f(None, Some(Allow), Some(Allow), Some(Deny)), Denied),
            // The most specific Allow grants; a missing app row inherits.
            (f(None, Some(Allow), Some(Allow), Some(Allow)), Granted),
            (f(None, Some(Allow), Some(Allow), None), Granted),
            (f(None, Some(Allow), Some(Allow), Some(Prompt)), Granted),
            (f(None, Some(Allow), None, None), Granted),
            (f(None, None, Some(Allow), None), Granted),
            (f(None, None, None, Some(Allow)), Granted),
            // Nothing readable, or only Prompt: undetermined, never a guess.
            (f(None, None, None, None), NotDetermined),
            (f(None, Some(Prompt), Some(Prompt), Some(Prompt)), NotDetermined),
            (f(None, Some(Allow), Some(Prompt), None), NotDetermined),
        ];
        for (facts, expected) in table {
            assert_eq!(microphone_consent(&facts), expected, "{facts:?}");
        }
        assert_eq!(ConsentValue::parse("Allow"), Some(Allow));
        assert_eq!(ConsentValue::parse(" deny "), Some(Deny));
        assert_eq!(ConsentValue::parse("PROMPT"), Some(Prompt));
        assert_eq!(ConsentValue::parse("maybe"), None);
        assert_eq!(MicrophonePolicy::from_dword(0), Some(UserControl));
        assert_eq!(MicrophonePolicy::from_dword(1), Some(ForceAllow));
        assert_eq!(MicrophonePolicy::from_dword(2), Some(ForceDeny));
        assert_eq!(MicrophonePolicy::from_dword(7), None);
        assert_eq!(consent_store_app_key(r"C:\Program Files\Voltip\voltip-desktop.exe"), "C:#Program Files#Voltip#voltip-desktop.exe");
    }
}
