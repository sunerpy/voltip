//! The key a held take is held with, and how each platform's key-state query names it
//! (docs/dictation.md §29): the shell asks whether that key is still physically down when a
//! hold-to-talk take may have missed its release. Pure tables, tested on every host; the queries
//! themselves are `voltip_hooks::is_down`.
//!
//! A chord is held by its key (the `Space` of `Ctrl+Alt+Space`): the global-shortcut plugin ends
//! a hold when that key goes up, so that is the key whose state counts. Names are the canonical
//! ones of `voltip_core::Hotkey` (`A`, `5`, `/`, `Space`, `F5`, `ArrowUp`, …); one this table does
//! not know is not looked up at all, and the shell then leaves the take alone.

use crate::solo_key::{SoloKey, WindowsButton, macos_button, macos_modifier, windows_button, windows_vk as solo_windows_vk, x11_button, x11_keycode};

/// What a take is held with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeldKey {
    /// The lone-key trigger.
    Solo(SoloKey),
    /// A chord's non-modifier key, by its canonical name.
    Chord(String),
}

/// `GetAsyncKeyState`'s virtual-key code for `key` (Windows): the solo keys' own codes, the mouse
/// buttons' `VK_MBUTTON` / `VK_XBUTTON1` / `VK_XBUTTON2`, a chord key's `VK_*`.
pub fn windows_vk(key: &HeldKey) -> Option<u16> {
    match key {
        HeldKey::Solo(solo) => solo_windows_vk(*solo).or(match windows_button(*solo) {
            Some(WindowsButton::Middle) => Some(0x04),
            Some(WindowsButton::X1) => Some(0x05),
            Some(WindowsButton::X2) => Some(0x06),
            None => None,
        }),
        HeldKey::Chord(name) => chord_windows_vk(name),
    }
}

fn chord_windows_vk(name: &str) -> Option<u16> {
    if let Some(c) = single_char(name) {
        return match c {
            'A'..='Z' | '0'..='9' => Some(c as u16),
            ';' => Some(0xBA),
            '=' => Some(0xBB),
            ',' => Some(0xBC),
            '-' => Some(0xBD),
            '.' => Some(0xBE),
            '/' => Some(0xBF),
            '`' => Some(0xC0),
            '[' => Some(0xDB),
            '\\' => Some(0xDC),
            ']' => Some(0xDD),
            '\'' => Some(0xDE),
            _ => None,
        };
    }
    if let Some(n) = function_key(name, 24) {
        return Some(0x70 + n - 1);
    }
    Some(match name {
        "Space" => 0x20,
        "Enter" => 0x0D,
        "Tab" => 0x09,
        "Escape" => 0x1B,
        "Backspace" => 0x08,
        "Delete" => 0x2E,
        "Insert" => 0x2D,
        "Home" => 0x24,
        "End" => 0x23,
        "PageUp" => 0x21,
        "PageDown" => 0x22,
        "ArrowLeft" => 0x25,
        "ArrowUp" => 0x26,
        "ArrowRight" => 0x27,
        "ArrowDown" => 0x28,
        "CapsLock" => 0x14,
        "ScrollLock" => 0x91,
        "Pause" => 0x13,
        "PrintScreen" => 0x2C,
        _ => return None,
    })
}

/// The one character of a single-character key name.
fn single_char(name: &str) -> Option<char> {
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    }
}

/// `n` of `F<n>`, 1 to `last`.
fn function_key(name: &str, last: u16) -> Option<u16> {
    let digits = name.strip_prefix('F')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) || digits.starts_with('0') {
        return None;
    }
    digits.parse::<u16>().ok().filter(|n| (1..=last).contains(n))
}

/// What macOS's `CGEventSourceKeyState` / `CGEventSourceButtonState` take for a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MacosKey {
    /// A `kVK_*` virtual keycode.
    Key(u16),
    /// A `CGMouseButton` number (2 = the wheel, 3 = back, 4 = forward).
    Button(u32),
}

/// `key` for macOS's key-state queries.
pub fn macos_key(key: &HeldKey) -> Option<MacosKey> {
    match key {
        HeldKey::Solo(solo) => {
            macos_modifier(*solo).map(|(code, _)| MacosKey::Key(code)).or_else(|| macos_button(*solo).and_then(|b| u32::try_from(b).ok()).map(MacosKey::Button))
        }
        HeldKey::Chord(name) => chord_macos_code(name).map(MacosKey::Key),
    }
}

/// The ANSI `kVK_*` codes (`HIToolbox/Events.h`): physical positions, as Carbon's hotkey
/// registration names them.
fn chord_macos_code(name: &str) -> Option<u16> {
    if let Some(c) = single_char(name) {
        return Some(match c {
            'A' => 0x00,
            'S' => 0x01,
            'D' => 0x02,
            'F' => 0x03,
            'H' => 0x04,
            'G' => 0x05,
            'Z' => 0x06,
            'X' => 0x07,
            'C' => 0x08,
            'V' => 0x09,
            'B' => 0x0B,
            'Q' => 0x0C,
            'W' => 0x0D,
            'E' => 0x0E,
            'R' => 0x0F,
            'Y' => 0x10,
            'T' => 0x11,
            '1' => 0x12,
            '2' => 0x13,
            '3' => 0x14,
            '4' => 0x15,
            '6' => 0x16,
            '5' => 0x17,
            '=' => 0x18,
            '9' => 0x19,
            '7' => 0x1A,
            '-' => 0x1B,
            '8' => 0x1C,
            '0' => 0x1D,
            ']' => 0x1E,
            'O' => 0x1F,
            'U' => 0x20,
            '[' => 0x21,
            'I' => 0x22,
            'P' => 0x23,
            'L' => 0x25,
            'J' => 0x26,
            '\'' => 0x27,
            'K' => 0x28,
            ';' => 0x29,
            '\\' => 0x2A,
            ',' => 0x2B,
            '/' => 0x2C,
            'N' => 0x2D,
            'M' => 0x2E,
            '.' => 0x2F,
            '`' => 0x32,
            _ => return None,
        });
    }
    if let Some(n) = function_key(name, 20) {
        const F: [u16; 20] = [0x7A, 0x78, 0x63, 0x76, 0x60, 0x61, 0x62, 0x64, 0x65, 0x6D, 0x67, 0x6F, 0x69, 0x6B, 0x71, 0x6A, 0x40, 0x4F, 0x50, 0x5A];
        return F.get(usize::from(n) - 1).copied();
    }
    Some(match name {
        "Space" => 0x31,
        "Enter" => 0x24,
        "Tab" => 0x30,
        "Escape" => 0x35,
        "Backspace" => 0x33,
        "Delete" => 0x75,
        // kVK_Help: where a Mac keyboard has the PC's Insert.
        "Insert" => 0x72,
        "Home" => 0x73,
        "End" => 0x77,
        "PageUp" => 0x74,
        "PageDown" => 0x79,
        "ArrowLeft" => 0x7B,
        "ArrowRight" => 0x7C,
        "ArrowDown" => 0x7D,
        "ArrowUp" => 0x7E,
        "CapsLock" => 0x39,
        _ => return None,
    })
}

/// What an X11 key-state query needs for a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum X11Key {
    /// A keycode (`QueryKeymap`): the solo modifiers, which are physical keys under evdev.
    Keycode(u8),
    /// A keysym the server's keyboard mapping turns into keycodes: a chord key, which
    /// `XGrabKey` registered by its keysym on the current layout.
    Keysym(u32),
    /// A pointer button number (`XIQueryPointer`'s button mask; the core `QueryPointer` mask has
    /// no buttons 8 and 9).
    Button(u8),
}

/// `key` for the X11 key-state queries.
pub fn x11_key(key: &HeldKey) -> Option<X11Key> {
    match key {
        HeldKey::Solo(solo) => x11_keycode(*solo).map(X11Key::Keycode).or_else(|| x11_button(*solo).map(X11Key::Button)),
        HeldKey::Chord(name) => chord_x11_keysym(name).map(X11Key::Keysym),
    }
}

/// The keysym the chord's grab was registered with (`X11/keysymdef.h`; Latin-1 keys are their
/// character, letters the upper-case one, as global-hotkey registers them).
fn chord_x11_keysym(name: &str) -> Option<u32> {
    if let Some(c) = single_char(name) {
        return matches!(c, 'A'..='Z' | '0'..='9' | '/' | '-' | '=' | '[' | ']' | '\\' | ';' | '\'' | ',' | '.' | '`').then_some(c as u32);
    }
    if let Some(n) = function_key(name, 24) {
        return Some(0xFFBE + u32::from(n) - 1);
    }
    Some(match name {
        "Space" => 0x20,
        "Enter" => 0xFF0D,
        "Tab" => 0xFF09,
        "Escape" => 0xFF1B,
        "Backspace" => 0xFF08,
        "Delete" => 0xFFFF,
        "Insert" => 0xFF63,
        "Home" => 0xFF50,
        "End" => 0xFF57,
        "PageUp" => 0xFF55,
        "PageDown" => 0xFF56,
        "ArrowLeft" => 0xFF51,
        "ArrowUp" => 0xFF52,
        "ArrowRight" => 0xFF53,
        "ArrowDown" => 0xFF54,
        "CapsLock" => 0xFFE5,
        "ScrollLock" => 0xFF14,
        "Pause" => 0xFF13,
        "PrintScreen" => 0xFF61,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(key: &str) -> HeldKey {
        HeldKey::Chord(key.to_owned())
    }

    /// Every key name `voltip_core::Hotkey` writes for a chord a recorder can produce.
    fn chord_keys() -> Vec<String> {
        let mut keys: Vec<String> = ('A'..='Z').chain('0'..='9').map(String::from).collect();
        keys.extend(["/", "-", "=", "[", "]", "\\", ";", "'", ",", ".", "`"].map(String::from));
        keys.extend(
            [
                "Space",
                "Enter",
                "Tab",
                "Escape",
                "Backspace",
                "Delete",
                "Insert",
                "Home",
                "End",
                "PageUp",
                "PageDown",
                "ArrowUp",
                "ArrowDown",
                "ArrowLeft",
                "ArrowRight",
                "CapsLock",
                "ScrollLock",
                "Pause",
                "PrintScreen",
            ]
            .map(String::from),
        );
        keys.extend((1..=24).map(|n| format!("F{n}")));
        keys
    }

    #[test]
    fn windows_names_every_chord_key_and_every_solo_key_once() {
        assert_eq!(windows_vk(&chord("A")), Some(0x41));
        assert_eq!(windows_vk(&chord("Z")), Some(0x5A));
        assert_eq!(windows_vk(&chord("0")), Some(0x30));
        assert_eq!(windows_vk(&chord("Space")), Some(0x20));
        assert_eq!(windows_vk(&chord("F1")), Some(0x70));
        assert_eq!(windows_vk(&chord("F24")), Some(0x87));
        assert_eq!(windows_vk(&chord("ArrowLeft")), Some(0x25));
        assert_eq!(windows_vk(&chord("/")), Some(0xBF));
        assert_eq!(windows_vk(&chord("PrintScreen")), Some(0x2C));
        // The mouse buttons are virtual keys too.
        assert_eq!(windows_vk(&HeldKey::Solo(SoloKey::MouseMiddle)), Some(0x04));
        assert_eq!(windows_vk(&HeldKey::Solo(SoloKey::MouseBack)), Some(0x05));
        assert_eq!(windows_vk(&HeldKey::Solo(SoloKey::MouseForward)), Some(0x06));
        assert_eq!(windows_vk(&HeldKey::Solo(SoloKey::RightCtrl)), Some(0xA3));
        assert_eq!(windows_vk(&HeldKey::Solo(SoloKey::Fn)), None, "a PC keyboard's Fn never reaches the system");
        let mut codes: Vec<u16> = chord_keys().iter().map(|k| windows_vk(&chord(k)).unwrap_or_else(|| panic!("{k}"))).collect();
        let n = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), n, "two chord keys share a code");
        for unknown in ["", "Numpad1", "AudioVolumeUp", "F25", "F0", "Ä", "ab"] {
            assert_eq!(windows_vk(&chord(unknown)), None, "{unknown}");
        }
    }

    #[test]
    fn macos_names_its_keys_and_has_no_scroll_lock() {
        assert_eq!(macos_key(&chord("A")), Some(MacosKey::Key(0x00)));
        assert_eq!(macos_key(&chord("S")), Some(MacosKey::Key(0x01)));
        assert_eq!(macos_key(&chord("Z")), Some(MacosKey::Key(0x06)));
        assert_eq!(macos_key(&chord("1")), Some(MacosKey::Key(0x12)));
        assert_eq!(macos_key(&chord("0")), Some(MacosKey::Key(0x1D)));
        assert_eq!(macos_key(&chord("Space")), Some(MacosKey::Key(0x31)));
        assert_eq!(macos_key(&chord("Enter")), Some(MacosKey::Key(0x24)));
        assert_eq!(macos_key(&chord("Backspace")), Some(MacosKey::Key(0x33)));
        assert_eq!(macos_key(&chord("Delete")), Some(MacosKey::Key(0x75)));
        assert_eq!(macos_key(&chord("F1")), Some(MacosKey::Key(0x7A)));
        assert_eq!(macos_key(&chord("F20")), Some(MacosKey::Key(0x5A)));
        assert_eq!(macos_key(&chord("ArrowUp")), Some(MacosKey::Key(0x7E)));
        assert_eq!(macos_key(&chord("/")), Some(MacosKey::Key(0x2C)));
        for none in ["ScrollLock", "Pause", "PrintScreen", "F21", "F24", "Numpad1"] {
            assert_eq!(macos_key(&chord(none)), None, "{none}");
        }
        assert_eq!(macos_key(&HeldKey::Solo(SoloKey::RightCtrl)), Some(MacosKey::Key(0x3E)));
        assert_eq!(macos_key(&HeldKey::Solo(SoloKey::Fn)), Some(MacosKey::Key(0x3F)));
        assert_eq!(macos_key(&HeldKey::Solo(SoloKey::MouseBack)), Some(MacosKey::Button(3)));
        let mut codes: Vec<u16> = chord_keys()
            .iter()
            .filter_map(|k| match macos_key(&chord(k)) {
                Some(MacosKey::Key(code)) => Some(code),
                _ => None,
            })
            .collect();
        assert_eq!(codes.len(), chord_keys().len() - 7, "all but scroll lock, pause, print screen and F21–F24");
        let n = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), n, "two chord keys share a code");
    }

    #[test]
    fn x11_names_chord_keys_by_keysym_and_solo_keys_by_code_or_button() {
        assert_eq!(x11_key(&chord("A")), Some(X11Key::Keysym(0x41)));
        assert_eq!(x11_key(&chord("7")), Some(X11Key::Keysym(0x37)));
        assert_eq!(x11_key(&chord("/")), Some(X11Key::Keysym(0x2F)));
        assert_eq!(x11_key(&chord("Space")), Some(X11Key::Keysym(0x20)));
        assert_eq!(x11_key(&chord("Enter")), Some(X11Key::Keysym(0xFF0D)));
        assert_eq!(x11_key(&chord("Delete")), Some(X11Key::Keysym(0xFFFF)));
        assert_eq!(x11_key(&chord("PageDown")), Some(X11Key::Keysym(0xFF56)));
        assert_eq!(x11_key(&chord("F1")), Some(X11Key::Keysym(0xFFBE)));
        assert_eq!(x11_key(&chord("F24")), Some(X11Key::Keysym(0xFFD5)));
        assert_eq!(x11_key(&chord("PrintScreen")), Some(X11Key::Keysym(0xFF61)));
        assert_eq!(x11_key(&HeldKey::Solo(SoloKey::RightCtrl)), Some(X11Key::Keycode(105)));
        assert_eq!(x11_key(&HeldKey::Solo(SoloKey::MouseForward)), Some(X11Key::Button(9)));
        assert_eq!(x11_key(&HeldKey::Solo(SoloKey::Fn)), None);
        let mut syms: Vec<u32> = chord_keys()
            .iter()
            .map(|k| match x11_key(&chord(k)) {
                Some(X11Key::Keysym(sym)) => sym,
                other => panic!("{k}: {other:?}"),
            })
            .collect();
        let n = syms.len();
        syms.sort_unstable();
        syms.dedup();
        assert_eq!(syms.len(), n, "two chord keys share a keysym");
        assert_eq!(x11_key(&chord("Numpad1")), None);
    }

    #[test]
    fn the_solo_keys_agree_with_the_hook_tables() {
        for key in SoloKey::ALL {
            let held = HeldKey::Solo(key);
            assert_eq!(windows_vk(&held).is_some(), solo_windows_vk(key).is_some() || windows_button(key).is_some(), "{key}");
            assert_eq!(macos_key(&held).is_some(), macos_modifier(key).is_some() || macos_button(key).is_some(), "{key}");
            assert_eq!(x11_key(&held).is_some(), x11_keycode(key).is_some() || x11_button(key).is_some(), "{key}");
        }
    }
}
