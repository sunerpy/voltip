//! The lone-key trigger (docs/dictation.md §13.1): a right-hand modifier, Fn or a mouse button held
//! on its own drives a take the way the hotkey does. The global-shortcut plugin only registers
//! chords, so the shell watches these keys through each platform's low-level input hook (Windows
//! `WH_KEYBOARD_LL` / `WH_MOUSE_LL`, a macOS event tap, X11 XInput 2 raw events) and feeds what it
//! sees into a [`SoloTracker`].
//!
//! Everything here is a table or a pure state machine, tested on every host: which keys a platform
//! offers, the codes its hook reports them with, and how raw input becomes press / release /
//! chorded edges.

use serde::{Deserialize, Serialize};

use crate::HostOs;

/// A key or button that triggers a take on its own (`Settings.solo_key`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SoloKey {
    /// The right Ctrl key (Control on macOS).
    RightCtrl,
    /// The right Alt key (Option on macOS; AltGr on many European layouts).
    RightAlt,
    /// The right Shift key.
    RightShift,
    /// The right Windows / Command / Super key.
    RightMeta,
    /// The Fn (🌐) key of Apple keyboards. macOS only: on a PC keyboard Fn never reaches the OS.
    Fn,
    /// The middle mouse button (the wheel click).
    MouseMiddle,
    /// The mouse's back side button (X1).
    MouseBack,
    /// The mouse's forward side button (X2).
    MouseForward,
}

impl SoloKey {
    /// Every key, in the order the settings page lists them.
    pub const ALL: [Self; 8] =
        [Self::RightCtrl, Self::RightAlt, Self::RightShift, Self::RightMeta, Self::Fn, Self::MouseMiddle, Self::MouseBack, Self::MouseForward];

    /// Wire name (`right_ctrl`, `mouse_back`, …).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RightCtrl => "right_ctrl",
            Self::RightAlt => "right_alt",
            Self::RightShift => "right_shift",
            Self::RightMeta => "right_meta",
            Self::Fn => "fn",
            Self::MouseMiddle => "mouse_middle",
            Self::MouseBack => "mouse_back",
            Self::MouseForward => "mouse_forward",
        }
    }

    /// A mouse button: the hook swallows it (a held back button would otherwise navigate the
    /// browser back), and keys pressed meanwhile are no chord.
    pub const fn is_mouse(self) -> bool {
        matches!(self, Self::MouseMiddle | Self::MouseBack | Self::MouseForward)
    }

    /// The keys `os` can watch. Fn exists only on macOS; hosts without a hook offer nothing.
    pub fn available_on(os: HostOs) -> Vec<Self> {
        match os {
            HostOs::Macos => Self::ALL.to_vec(),
            HostOs::Windows | HostOs::Linux => Self::ALL.into_iter().filter(|k| *k != Self::Fn).collect(),
            HostOs::Other => Vec::new(),
        }
    }
}

impl std::fmt::Display for SoloKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A mouse button as the Windows low-level mouse hook names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowsButton {
    /// `WM_MBUTTONDOWN` / `WM_MBUTTONUP`.
    Middle,
    /// `WM_XBUTTONDOWN` / `WM_XBUTTONUP` with `XBUTTON1` in the high word of `mouseData`.
    X1,
    /// The same with `XBUTTON2`.
    X2,
}

/// The unassigned virtual key the Windows hook taps while a lone modifier is held (AutoHotkey's
/// "menu mask key"): Windows then no longer sees a lone Alt (which activates the menu bar of the
/// focused window on release), a lone Win (which opens Start) or a lone Shift (which an IME takes
/// as its Chinese / English switch). Applications ignore the key.
pub const WINDOWS_MASK_VK: u16 = 0xE8;

/// The virtual-key code of `key` in a `KBDLLHOOKSTRUCT` (the low-level hook reports left and right
/// modifiers apart): `VK_RCONTROL`, `VK_RMENU`, `VK_RSHIFT`, `VK_RWIN`.
pub const fn windows_vk(key: SoloKey) -> Option<u16> {
    match key {
        SoloKey::RightCtrl => Some(0xA3),
        SoloKey::RightAlt => Some(0xA5),
        SoloKey::RightShift => Some(0xA1),
        SoloKey::RightMeta => Some(0x5C),
        SoloKey::Fn | SoloKey::MouseMiddle | SoloKey::MouseBack | SoloKey::MouseForward => None,
    }
}

/// The low-level mouse hook's name for `key`.
pub const fn windows_button(key: SoloKey) -> Option<WindowsButton> {
    match key {
        SoloKey::MouseMiddle => Some(WindowsButton::Middle),
        SoloKey::MouseBack => Some(WindowsButton::X1),
        SoloKey::MouseForward => Some(WindowsButton::X2),
        _ => None,
    }
}

/// The `kVK_*` virtual keycode a macOS `flagsChanged` event carries for `key`, and the
/// device-dependent flag (`NX_DEVICER*KEYMASK`, `kCGEventFlagMaskSecondaryFn` for Fn) that is set
/// while it is down. The flag, not the event count, says whether the key went down or up.
pub const fn macos_modifier(key: SoloKey) -> Option<(u16, u64)> {
    match key {
        SoloKey::RightCtrl => Some((0x3E, 0x2000)),
        SoloKey::RightAlt => Some((0x3D, 0x40)),
        SoloKey::RightShift => Some((0x3C, 0x04)),
        SoloKey::RightMeta => Some((0x36, 0x10)),
        SoloKey::Fn => Some((0x3F, 0x0080_0000)),
        SoloKey::MouseMiddle | SoloKey::MouseBack | SoloKey::MouseForward => None,
    }
}

/// `kCGMouseEventButtonNumber` of an `otherMouseDown` / `otherMouseUp` for `key`.
pub const fn macos_button(key: SoloKey) -> Option<i64> {
    match key {
        SoloKey::MouseMiddle => Some(2),
        SoloKey::MouseBack => Some(3),
        SoloKey::MouseForward => Some(4),
        _ => None,
    }
}

/// The X11 keycode of `key` under the evdev / libinput drivers every current X server and XWayland
/// use (Linux input code + 8): `KEY_RIGHTCTRL`, `KEY_RIGHTALT`, `KEY_RIGHTSHIFT`, `KEY_RIGHTMETA`.
/// Keycodes rather than keysyms: Right Alt is `ISO_Level3_Shift` on many layouts.
pub const fn x11_keycode(key: SoloKey) -> Option<u8> {
    match key {
        SoloKey::RightCtrl => Some(105),
        SoloKey::RightAlt => Some(108),
        SoloKey::RightShift => Some(62),
        SoloKey::RightMeta => Some(134),
        SoloKey::Fn | SoloKey::MouseMiddle | SoloKey::MouseBack | SoloKey::MouseForward => None,
    }
}

/// The X11 button number of `key` (2 = middle, 8 = back, 9 = forward).
pub const fn x11_button(key: SoloKey) -> Option<u8> {
    match key {
        SoloKey::MouseMiddle => Some(2),
        SoloKey::MouseBack => Some(8),
        SoloKey::MouseForward => Some(9),
        _ => None,
    }
}

/// What a hook saw, already sorted against the trigger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoloInput {
    /// The trigger went down (auto-repeat included).
    TriggerDown,
    /// The trigger went up.
    TriggerUp,
    /// Any other key or mouse button went down.
    OtherDown,
}

/// What the core hears (a `HotkeyEdge`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoloEdge {
    /// The trigger went down on its own.
    Press,
    /// It went up without anything joining it.
    Release,
    /// Another key or button went down while it was held: the press was the start of a shortcut
    /// (Right Ctrl + C), not a take. Its physical release is not reported.
    Chorded,
}

/// Raw input → edges for one trigger: auto-repeat is dropped, the first other key or button during
/// a modifier's hold turns the press into [`SoloEdge::Chorded`], and a mouse trigger never chords
/// (typing while the side button is held is not a shortcut).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SoloTracker {
    chords: bool,
    held: bool,
    chorded: bool,
}

impl SoloTracker {
    /// A tracker for `key`, with nothing held.
    pub const fn new(key: SoloKey) -> Self {
        Self { chords: !key.is_mouse(), held: false, chorded: false }
    }

    /// Whether the trigger is down, as far as the input says.
    pub const fn held(&self) -> bool {
        self.held
    }

    /// Fold one input; the edge to report, if any.
    pub fn feed(&mut self, input: SoloInput) -> Option<SoloEdge> {
        match input {
            SoloInput::TriggerDown if self.held => None,
            SoloInput::TriggerDown => {
                self.held = true;
                self.chorded = false;
                Some(SoloEdge::Press)
            }
            SoloInput::TriggerUp if !self.held => None,
            SoloInput::TriggerUp => {
                self.held = false;
                (!std::mem::take(&mut self.chorded)).then_some(SoloEdge::Release)
            }
            SoloInput::OtherDown if self.held && self.chords && !self.chorded => {
                self.chorded = true;
                Some(SoloEdge::Chorded)
            }
            SoloInput::OtherDown => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use SoloEdge::{Chorded, Press, Release};
    use SoloInput::{OtherDown, TriggerDown, TriggerUp};

    fn run(key: SoloKey, inputs: &[SoloInput]) -> Vec<Option<SoloEdge>> {
        let mut tracker = SoloTracker::new(key);
        inputs.iter().map(|input| tracker.feed(*input)).collect()
    }

    #[test]
    fn a_lone_press_is_a_press_and_a_release_and_auto_repeat_is_dropped() {
        assert_eq!(run(SoloKey::RightCtrl, &[TriggerDown, TriggerDown, TriggerDown, TriggerUp]), vec![Some(Press), None, None, Some(Release)]);
        // An up without a down (the hook started while the key was held) and a stray other key.
        assert_eq!(run(SoloKey::RightCtrl, &[TriggerUp, OtherDown, TriggerDown, TriggerUp]), vec![None, None, Some(Press), Some(Release)]);
    }

    #[test]
    fn regression_a_modifier_joined_by_another_key_is_a_chord_not_a_take() {
        // Right Ctrl + C: the press, then Chorded once, then nothing for the rest of the hold.
        assert_eq!(run(SoloKey::RightCtrl, &[TriggerDown, OtherDown, OtherDown, TriggerDown, TriggerUp]), vec![Some(Press), Some(Chorded), None, None, None]);
        // The next lone press is a take again.
        assert_eq!(
            run(SoloKey::RightAlt, &[TriggerDown, OtherDown, TriggerUp, TriggerDown, TriggerUp]),
            vec![Some(Press), Some(Chorded), None, Some(Press), Some(Release)]
        );
        // A key held before the trigger went down is no chord.
        assert_eq!(run(SoloKey::RightShift, &[OtherDown, TriggerDown, TriggerUp]), vec![None, Some(Press), Some(Release)]);
    }

    #[test]
    fn a_mouse_trigger_never_chords() {
        assert_eq!(run(SoloKey::MouseBack, &[TriggerDown, OtherDown, OtherDown, TriggerUp]), vec![Some(Press), None, None, Some(Release)]);
    }

    #[test]
    fn every_platform_names_the_keys_it_offers() {
        for key in SoloKey::available_on(HostOs::Windows) {
            assert!(windows_vk(key).is_some() ^ windows_button(key).is_some(), "{key}");
        }
        for key in SoloKey::available_on(HostOs::Linux) {
            assert!(x11_keycode(key).is_some() ^ x11_button(key).is_some(), "{key}");
        }
        for key in SoloKey::available_on(HostOs::Macos) {
            assert!(macos_modifier(key).is_some() ^ macos_button(key).is_some(), "{key}");
        }
        assert!(SoloKey::available_on(HostOs::Macos).contains(&SoloKey::Fn));
        assert!(!SoloKey::available_on(HostOs::Windows).contains(&SoloKey::Fn));
        assert!(!SoloKey::available_on(HostOs::Linux).contains(&SoloKey::Fn));
        assert!(SoloKey::available_on(HostOs::Other).is_empty());
        // Codes are distinct within a platform.
        let mut vks: Vec<u16> = SoloKey::ALL.into_iter().filter_map(windows_vk).collect();
        vks.push(WINDOWS_MASK_VK);
        let n = vks.len();
        vks.sort_unstable();
        vks.dedup();
        assert_eq!(vks.len(), n);
        let flags: Vec<u64> = SoloKey::ALL.into_iter().filter_map(macos_modifier).map(|(_, flag)| flag).collect();
        assert!(flags.iter().all(|f| f.count_ones() == 1));
        assert_eq!(flags.iter().fold(0, |acc, f| acc | f).count_ones() as usize, flags.len());
    }

    #[test]
    fn wire_names_round_trip() {
        for key in SoloKey::ALL {
            let json = serde_json::to_string(&key).unwrap();
            assert_eq!(json, format!("\"{}\"", key.as_str()));
            assert_eq!(serde_json::from_str::<SoloKey>(&json).unwrap(), key);
            assert_eq!(key.to_string(), key.as_str());
        }
        assert!(serde_json::from_str::<SoloKey>("\"left_ctrl\"").is_err());
    }
}
