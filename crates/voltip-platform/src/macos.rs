//! macOS policy tables (docs/dictation.md §15.2). Compiled on every host so the tables are tested
//! on Linux CI; the shell applies them only under `#[cfg(target_os = "macos")]`.

/// `NSApplication.activationPolicy` as Tauri names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivationPolicy {
    /// Dock icon, Cmd-Tab entry, menu bar: an ordinary application.
    Regular,
    /// No Dock icon, no Cmd-Tab entry: a menu-bar (tray) utility that still owns windows.
    Accessory,
}

/// When the shell applies the policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyTiming {
    /// `App::set_activation_policy` after `Builder::build` and before `App::run`: the only window
    /// in which the runtime exists but `NSApplication` has not finished launching, so the Dock
    /// icon never flashes.
    BetweenBuildAndRun,
    /// Leave the default (`Regular`); nothing is called.
    Untouched,
}

/// The shell's launch decision for one `start_hidden × tray_available` cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActivationPolicyPlan {
    /// Policy to apply.
    pub policy: ActivationPolicy,
    /// When to apply it.
    pub timing: PolicyTiming,
    /// `RunEvent::Reopen` (Dock click, `open -a`) shows the main window. Always true: with a Dock
    /// icon that is what the click means, and without one the event can only come from `open -a`.
    pub reopen_shows_main_window: bool,
}

/// Decision table:
///
/// | `start_hidden` | `tray_available` | policy | why |
/// |---|---|---|---|
/// | true | true | `Accessory`, between build and run | menu-bar launch: the tray is the way back to the window, no Dock icon wanted |
/// | true | false | `Regular`, untouched | without a tray the Dock icon is the only way back to a hidden window |
/// | false | true | `Regular`, untouched | the window is on screen at launch (first run, onboarding): a normal app the user can Cmd-Tab to |
/// | false | false | `Regular`, untouched | plain application |
pub const fn activation_policy_plan(start_hidden: bool, tray_available: bool) -> ActivationPolicyPlan {
    let (policy, timing) = if start_hidden && tray_available {
        (ActivationPolicy::Accessory, PolicyTiming::BetweenBuildAndRun)
    } else {
        (ActivationPolicy::Regular, PolicyTiming::Untouched)
    };
    ActivationPolicyPlan { policy, timing, reopen_shows_main_window: true }
}

/// `kVK_ANSI_V`: the key that is `V` on ANSI / ISO QWERTY, QWERTZ, AZERTY and Colemak.
pub const ANSI_V_KEYCODE: u16 = 9;
/// `kVK_ANSI_Period`: where `v` sits on Dvorak (the QWERTY `.` key).
pub const DVORAK_V_KEYCODE: u16 = 47;
/// Highest virtual keycode `CGEventCreateKeyboardEvent` accepts.
pub const MAX_KEYCODE: u16 = 127;
/// How long the modifier stays down after the `V` press before both are released: shorter and
/// Electron / Java apps see a bare `v` and type the letter instead of pasting.
pub const PASTE_HOLD_MS: u64 = 100;

/// The Cmd+V chord the injector should synthesise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PasteChord {
    /// Virtual keycode of the key labelled `v` in the current layout.
    pub v_keycode: u16,
    /// [`PASTE_HOLD_MS`].
    pub hold_ms: u64,
}

/// Layout-aware keycode for the `v` key.
///
/// `translated` is the `UCKeyTranslate` reverse lookup the shell ran against the current keyboard
/// layout (`TISCopyCurrentKeyboardLayoutInputSource` → `kTISPropertyUnicodeKeyLayoutData`):
/// `Some(code)` when a key produced `v`, `None` when the layout data was missing or no key did.
/// Fallback table by input source id, then [`ANSI_V_KEYCODE`]:
///
/// | `layout_id` | keycode |
/// |---|---|
/// | `com.apple.keylayout.Dvorak` | 47 (`v` is on the QWERTY `.` key) |
/// | `com.apple.keylayout.DVORAK-QWERTYCMD` | 9 (Cmd chords use the QWERTY positions) |
/// | anything else (QWERTY, QWERTZ, AZERTY, Colemak, unknown) | 9 |
///
/// A `translated` value above [`MAX_KEYCODE`] is treated as a failed lookup.
pub fn cmd_v_keycode(translated: Option<u16>, layout_id: &str) -> u16 {
    match translated {
        Some(code) if code <= MAX_KEYCODE => code,
        _ => match layout_id {
            "com.apple.keylayout.Dvorak" => DVORAK_V_KEYCODE,
            _ => ANSI_V_KEYCODE,
        },
    }
}

/// [`cmd_v_keycode`] plus the hold: everything the injector needs for one paste.
pub fn paste_chord(translated: Option<u16>, layout_id: &str) -> PasteChord {
    PasteChord { v_keycode: cmd_v_keycode(translated, layout_id), hold_ms: PASTE_HOLD_MS }
}

/// `kVK_ANSI_C`: the key that is `C` on ANSI / ISO QWERTY, QWERTZ, AZERTY and Colemak.
pub const ANSI_C_KEYCODE: u16 = 8;
/// `kVK_ANSI_I`: where `c` sits on Dvorak (the QWERTY `i` key).
pub const DVORAK_C_KEYCODE: u16 = 34;

/// Layout-aware keycode for the `c` key of the selection copy (docs/dictation.md §19), the same
/// rule as [`cmd_v_keycode`]: the `UCKeyTranslate` answer when there is a sane one, then the
/// input-source table, then [`ANSI_C_KEYCODE`]. The copy holds the key [`PASTE_HOLD_MS`] too.
///
/// | `layout_id` | keycode |
/// |---|---|
/// | `com.apple.keylayout.Dvorak` | 34 (`c` is on the QWERTY `i` key) |
/// | `com.apple.keylayout.DVORAK-QWERTYCMD` | 8 (Cmd chords use the QWERTY positions) |
/// | anything else | 8 |
pub fn cmd_c_keycode(translated: Option<u16>, layout_id: &str) -> u16 {
    match translated {
        Some(code) if code <= MAX_KEYCODE => code,
        _ => match layout_id {
            "com.apple.keylayout.Dvorak" => DVORAK_C_KEYCODE,
            _ => ANSI_C_KEYCODE,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_policy_table() {
        let table = [
            ((true, true), ActivationPolicy::Accessory, PolicyTiming::BetweenBuildAndRun),
            ((true, false), ActivationPolicy::Regular, PolicyTiming::Untouched),
            ((false, true), ActivationPolicy::Regular, PolicyTiming::Untouched),
            ((false, false), ActivationPolicy::Regular, PolicyTiming::Untouched),
        ];
        for ((start_hidden, tray), policy, timing) in table {
            let plan = activation_policy_plan(start_hidden, tray);
            assert_eq!(plan.policy, policy, "start_hidden={start_hidden} tray={tray}");
            assert_eq!(plan.timing, timing, "start_hidden={start_hidden} tray={tray}");
            assert!(plan.reopen_shows_main_window);
        }
        // Accessory is never applied without the tray that gets the user back to the window.
        assert_eq!(activation_policy_plan(true, false).policy, ActivationPolicy::Regular);
    }

    #[test]
    fn cmd_v_keycode_fallback_table() {
        let table: &[(Option<u16>, &str, u16)] = &[
            // UCKeyTranslate answered: trust it whatever the layout id says.
            (Some(9), "com.apple.keylayout.US", 9),
            (Some(47), "com.apple.keylayout.Dvorak", 47),
            (Some(11), "com.apple.keylayout.Workman", 11),
            (Some(9), "com.apple.keylayout.Dvorak", 9),
            // Lookup failed: the layout table.
            (None, "com.apple.keylayout.US", 9),
            (None, "com.apple.keylayout.ABC", 9),
            (None, "com.apple.keylayout.German", 9),
            (None, "com.apple.keylayout.French", 9),
            (None, "com.apple.keylayout.Colemak", 9),
            (None, "com.apple.keylayout.Dvorak", 47),
            (None, "com.apple.keylayout.DVORAK-QWERTYCMD", 9),
            (None, "", 9),
            // Garbage from the lookup is a failed lookup.
            (Some(200), "com.apple.keylayout.Dvorak", 47),
            (Some(u16::MAX), "com.apple.keylayout.US", 9),
        ];
        for (translated, layout, expected) in table {
            assert_eq!(cmd_v_keycode(*translated, layout), *expected, "translated={translated:?} layout={layout}");
        }
        assert_eq!(paste_chord(None, "com.apple.keylayout.US"), PasteChord { v_keycode: ANSI_V_KEYCODE, hold_ms: 100 });
        assert_eq!(PASTE_HOLD_MS, 100);
    }

    /// docs/dictation.md §19: the copy chord's `c` follows the same fallback rule as the paste's `v`.
    #[test]
    fn cmd_c_keycode_fallback_table() {
        let table: &[(Option<u16>, &str, u16)] = &[
            (Some(8), "com.apple.keylayout.US", 8),
            (Some(34), "com.apple.keylayout.Dvorak", 34),
            (Some(5), "com.apple.keylayout.Workman", 5),
            (None, "com.apple.keylayout.US", 8),
            (None, "com.apple.keylayout.ABC", 8),
            (None, "com.apple.keylayout.German", 8),
            (None, "com.apple.keylayout.French", 8),
            (None, "com.apple.keylayout.Colemak", 8),
            (None, "com.apple.keylayout.Dvorak", 34),
            (None, "com.apple.keylayout.DVORAK-QWERTYCMD", 8),
            (None, "", 8),
            (Some(128), "com.apple.keylayout.Dvorak", 34),
            (Some(u16::MAX), "com.apple.keylayout.US", 8),
        ];
        for (translated, layout, expected) in table {
            assert_eq!(cmd_c_keycode(*translated, layout), *expected, "translated={translated:?} layout={layout}");
        }
        assert_eq!((ANSI_C_KEYCODE, DVORAK_C_KEYCODE), (8, 34));
    }
}
