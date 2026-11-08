//! The evdev codes of a chord's keys, looked up in the keymap the EIS keyboard announced
//! (docs/dictation.md §14.9). The remote-desktop keyboard takes evdev codes and Mutter turns them
//! into keysyms with the user's own layout, so a fixed table would press the wrong key on Dvorak
//! or with a swapped Caps Lock:
//!
//! * a letter is the key whose first level types it in the active layout; then the key that types
//!   it in another layout of the keymap (Ctrl+V on a Russian layout is the key with `v` in the
//!   Latin one, the key applications match the shortcut by); the US position when no layout has it;
//! * a modifier is a key whose first level is that modifier's keysym and that sets the modifier
//!   (`ctrl:swapcaps` moves Control to the Caps Lock key); any key that sets it next; the US
//!   position last;
//! * Insert is the key whose first level is `Insert` (not the keypad's `KP_Insert`).

use std::time::Duration;

use xkbcommon::xkb::{self, Keycode, Keysym, keysyms};

use crate::platform::{KeyStep, StepKey};
use crate::toolchain::Modifier;

/// xkb keycodes are evdev codes plus 8.
const EVDEV_OFFSET: u32 = 8;
/// evdev `KEY_INSERT`.
const KEY_INSERT: u32 = 110;

/// One event of a chord on the EIS keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyEvent {
    /// The key with this evdev code goes down (`pressed`) or up.
    Key {
        /// evdev code.
        code: u32,
        /// Down or up.
        pressed: bool,
    },
    /// Keep the keys down this long (macOS steps only; none on Linux).
    Hold(Duration),
}

/// Where the keys of a chord are on the keyboard the compositor announced.
pub struct KeyLookup {
    keymap: Option<xkb::Keymap>,
    group: u32,
}

impl std::fmt::Debug for KeyLookup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyLookup").field("keymap", &self.keymap.is_some()).field("group", &self.group).finish()
    }
}

impl KeyLookup {
    /// The keymap `text` (`XKB_KEYMAP_FORMAT_TEXT_V1`, as `ei_keyboard.keymap` carries it) with the
    /// layout `group` active; `None` when it does not compile.
    pub fn new(text: &str, group: u32) -> Option<Self> {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap = xkb::Keymap::new_from_string(&context, text.to_owned(), xkb::KEYMAP_FORMAT_TEXT_V1, xkb::KEYMAP_COMPILE_NO_FLAGS)?;
        let group = if group < keymap.num_layouts() { group } else { 0 };
        Some(Self { keymap: Some(keymap), group })
    }

    /// No keymap was announced: the US positions, what `ydotool` sends.
    pub fn us() -> Self {
        Self { keymap: None, group: 0 }
    }

    /// The evdev code of `modifier`.
    pub fn modifier(&self, modifier: Modifier) -> u32 {
        let (name, syms, us) = match modifier {
            Modifier::Control => (xkb::MOD_NAME_CTRL, [keysyms::KEY_Control_L, keysyms::KEY_Control_R], 29),
            Modifier::Shift => (xkb::MOD_NAME_SHIFT, [keysyms::KEY_Shift_L, keysyms::KEY_Shift_R], 42),
            Modifier::Alt => (xkb::MOD_NAME_ALT, [keysyms::KEY_Alt_L, keysyms::KEY_Alt_R], 56),
            Modifier::Meta => (xkb::MOD_NAME_LOGO, [keysyms::KEY_Super_L, keysyms::KEY_Super_R], 125),
        };
        let Some(keymap) = &self.keymap else { return us };
        let sets = |key: u32| self.sets(keymap, key, name);
        syms.iter()
            .find_map(|sym| keys(keymap).find(|&key| first_level(keymap, key, self.group).contains(&Keysym::new(*sym)) && sets(key)))
            .or_else(|| keys(keymap).find(|&key| sets(key)))
            .map_or(us, evdev)
    }

    /// The evdev code of the key that types `c`, `None` for a character no layout of the keymap
    /// and no US key types.
    pub fn char(&self, c: char) -> Option<u32> {
        let wanted = xkb::utf32_to_keysym(u32::from(c));
        if let Some(keymap) = &self.keymap {
            let others = (0..keymap.num_layouts()).filter(|&layout| layout != self.group);
            for layout in std::iter::once(self.group).chain(others) {
                if let Some(key) = keys(keymap).find(|&key| first_level(keymap, key, layout).contains(&wanted)) {
                    return Some(evdev(key));
                }
            }
        }
        us_letter(c)
    }

    /// The evdev code of Insert.
    pub fn insert(&self) -> u32 {
        let Some(keymap) = &self.keymap else { return KEY_INSERT };
        let insert = Keysym::new(keysyms::KEY_Insert);
        keys(keymap).find(|&key| first_level(keymap, key, self.group).contains(&insert)).map_or(KEY_INSERT, evdev)
    }

    /// `steps` ([`crate::platform::chord_steps`], [`crate::platform::copy_steps`]) as key events.
    /// A release of a key this keyboard never pressed (a hotkey modifier the user still holds) is
    /// left out: the compositor drops it anyway, the virtual keyboard cannot let go of the user's
    /// key. Whatever is still down at the end is released.
    pub fn plan(&self, steps: &[KeyStep]) -> Result<Vec<KeyEvent>, String> {
        let mut out = Vec::new();
        let mut down: Vec<u32> = Vec::new();
        for step in steps {
            match *step {
                KeyStep::Press(key) => {
                    let code = self.code(key)?;
                    if !down.contains(&code) {
                        down.push(code);
                        out.push(KeyEvent::Key { code, pressed: true });
                    }
                }
                KeyStep::Release(key) => {
                    let code = self.code(key)?;
                    if let Some(at) = down.iter().position(|&c| c == code) {
                        down.remove(at);
                        out.push(KeyEvent::Key { code, pressed: false });
                    }
                }
                KeyStep::Click(key) => {
                    let code = self.code(key)?;
                    out.push(KeyEvent::Key { code, pressed: true });
                    out.push(KeyEvent::Key { code, pressed: false });
                }
                KeyStep::Hold(duration) => out.push(KeyEvent::Hold(duration)),
            }
        }
        out.extend(down.into_iter().rev().map(|code| KeyEvent::Key { code, pressed: false }));
        Ok(out)
    }

    fn code(&self, key: StepKey) -> Result<u32, String> {
        match key {
            StepKey::Modifier(modifier) => Ok(self.modifier(modifier)),
            StepKey::Char(c) => self.char(c).ok_or_else(|| format!("no key of the keymap types {c:?}")),
            StepKey::Insert => Ok(self.insert()),
            StepKey::Code(code) => Err(format!("platform key code {code} is not an evdev key")),
        }
    }

    /// Whether pressing `key` alone, with the active layout, sets the modifier `name`.
    fn sets(&self, keymap: &xkb::Keymap, key: u32, name: &str) -> bool {
        let mut state = xkb::State::new(keymap);
        state.update_mask(0, 0, 0, 0, 0, self.group);
        state.update_key(Keycode::new(key), xkb::KeyDirection::Down);
        state.mod_name_is_active(name, xkb::STATE_MODS_EFFECTIVE)
    }
}

/// Every keycode of `keymap`, lowest first.
fn keys(keymap: &xkb::Keymap) -> impl Iterator<Item = u32> {
    keymap.min_keycode().raw()..=keymap.max_keycode().raw()
}

/// The keysyms on the first level of `key` in `layout`.
fn first_level(keymap: &xkb::Keymap, key: u32, layout: u32) -> &[Keysym] {
    keymap.key_get_syms_by_level(Keycode::new(key), layout, 0)
}

fn evdev(key: u32) -> u32 {
    key.saturating_sub(EVDEV_OFFSET)
}

/// The US QWERTY evdev code of a lowercase ASCII letter.
fn us_letter(c: char) -> Option<u32> {
    const ROWS: [(&str, u32); 3] = [("qwertyuiop", 16), ("asdfghjkl", 30), ("zxcvbnm", 44)];
    ROWS.iter().find_map(|(row, first)| row.find(c).and_then(|at| u32::try_from(at).ok()).map(|at| first + at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{chord_steps, copy_steps};
    use crate::toolchain::{Chord, Key};

    /// A keymap compiled from RMLVO names, as Mutter compiles the user's input sources, in the
    /// text form `ei_keyboard.keymap` carries. Needs the XKB data (`xkb-data`).
    fn keymap(layout: &str, variant: &str, options: Option<&str>, group: u32) -> KeyLookup {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap = xkb::Keymap::new_from_names(&context, "evdev", "pc105", layout, variant, options.map(str::to_owned), xkb::KEYMAP_COMPILE_NO_FLAGS)
            .unwrap_or_else(|| panic!("no XKB keymap for {layout}({variant}); is xkb-data installed?"));
        KeyLookup::new(&keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1), group).expect("the keymap text compiles again")
    }

    fn keys_of(events: &[KeyEvent]) -> Vec<(u32, bool)> {
        events
            .iter()
            .filter_map(|e| match e {
                KeyEvent::Key { code, pressed } => Some((*code, *pressed)),
                KeyEvent::Hold(_) => None,
            })
            .collect()
    }

    const CTRL_V: Chord = Chord { control: true, meta: false, shift: false, key: Key::Char('v') };

    #[test]
    fn us_keymap() {
        let us = keymap("us", "", None, 0);
        assert_eq!([us.modifier(Modifier::Control), us.modifier(Modifier::Shift), us.modifier(Modifier::Alt), us.modifier(Modifier::Meta)], [29, 42, 56, 125]);
        assert_eq!((us.char('v'), us.char('c'), us.insert()), (Some(47), Some(46), 110));
        assert_eq!(keys_of(&us.plan(&chord_steps(CTRL_V, "linux", 0)).unwrap()), [(29, true), (47, true), (47, false), (29, false)]);
    }

    /// Non-QWERTY layouts: Dvorak types `v` with the key US calls `.`, German swaps Y and Z.
    #[test]
    fn letters_follow_the_layout() {
        let dvorak = keymap("us", "dvorak", None, 0);
        assert_eq!((dvorak.char('v'), dvorak.char('c')), (Some(52), Some(23)), "KEY_DOT and KEY_I");
        assert_eq!(keys_of(&dvorak.plan(&chord_steps(CTRL_V, "linux", 0)).unwrap()), [(29, true), (52, true), (52, false), (29, false)]);
        let german = keymap("de", "", None, 0);
        assert_eq!((german.char('z'), german.char('y'), german.char('v')), (Some(21), Some(44), Some(47)));
    }

    /// Ctrl+V on a Russian layout: the key that types `v` in the Latin layout of the same keymap,
    /// whichever of the two is active.
    #[test]
    fn letters_from_another_layout() {
        assert_eq!(keymap("ru,us", ",", None, 0).char('v'), Some(47), "Russian active");
        assert_eq!(keymap("ru,us", ",", None, 1).char('v'), Some(47), "US active");
        let colemak_and_ru = keymap("us,ru", "colemak,", None, 1);
        assert_eq!(colemak_and_ru.char('v'), Some(47), "the Latin layout of the keymap decides, not the US table");
        assert_eq!(keymap("ru", "", None, 0).char('v'), Some(47), "no layout types it: the US position");
        assert_eq!(keymap("us", "", None, 5).group, 0, "an out-of-range group is the first layout");
    }

    /// `ctrl:swapcaps`: Control is the Caps Lock key, and the Control key types Caps Lock.
    #[test]
    fn modifiers_follow_the_options() {
        let swapped = keymap("us", "", Some("ctrl:swapcaps"), 0);
        assert_eq!(swapped.modifier(Modifier::Control), 58, "KEY_CAPSLOCK");
        let nocaps = keymap("us", "", Some("ctrl:nocaps"), 0);
        assert_eq!(nocaps.modifier(Modifier::Control), 29, "the Control key is still Control");
    }

    /// Shift+Insert and the selection copy (Ctrl+Insert, docs/dictation.md §19): a modifier the
    /// user holds is not released by the virtual keyboard, which never pressed it.
    #[test]
    fn plans_of_other_chords() {
        let us = keymap("us", "", None, 0);
        let shift_insert = Chord { control: false, meta: false, shift: true, key: Key::Insert };
        assert_eq!(keys_of(&us.plan(&chord_steps(shift_insert, "linux", 0)).unwrap()), [(42, true), (110, true), (110, false), (42, false)]);
        let ctrl_shift_v = Chord { control: true, meta: false, shift: true, key: Key::Char('v') };
        assert_eq!(
            keys_of(&us.plan(&chord_steps(ctrl_shift_v, "linux", 0)).unwrap()),
            [(29, true), (42, true), (47, true), (47, false), (42, false), (29, false)]
        );
        let copy = crate::copy_chord_for("linux");
        let held = [Modifier::Control, Modifier::Alt];
        assert_eq!(keys_of(&us.plan(&copy_steps(copy, "linux", 0, &held)).unwrap()), [(29, true), (110, true), (110, false), (29, false)]);
        // Anything still down at the end goes up; a platform key code is refused.
        assert_eq!(keys_of(&us.plan(&[KeyStep::Press(StepKey::Modifier(Modifier::Shift))]).unwrap()), [(42, true), (42, false)]);
        assert!(us.plan(&[KeyStep::Click(StepKey::Code(9))]).is_err());
        assert_eq!(us.plan(&[KeyStep::Hold(Duration::from_millis(5))]).unwrap(), [KeyEvent::Hold(Duration::from_millis(5))]);
    }

    /// No keymap announced, or one that does not compile.
    #[test]
    fn without_a_keymap() {
        assert!(KeyLookup::new("not a keymap", 0).is_none());
        let us = KeyLookup::us();
        assert_eq!((us.modifier(Modifier::Control), us.modifier(Modifier::Shift), us.char('v'), us.char('c'), us.insert()), (29, 42, Some(47), Some(46), 110));
        assert_eq!((us.char('q'), us.char('m'), us.char('л')), (Some(16), Some(50), None));
        assert!(us.plan(&[KeyStep::Click(StepKey::Char('л'))]).is_err());
        assert_eq!(format!("{us:?}"), "KeyLookup { keymap: false, group: 0 }");
    }
}
