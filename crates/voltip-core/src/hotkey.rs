//! The global dictation hotkey as the user writes it (`Ctrl+Alt+Space`).
//!
//! Parsing and validation live here so every shell registers exactly the chord the settings file
//! holds; the platform-specific registration (Tauri global-shortcut plugin) happens in the shell.

use serde::{Deserialize, Serialize};

/// Chord the app ships with.
pub const DEFAULT_HOTKEY: &str = "Ctrl+Alt+Space";
/// Chord of the voice edit (docs/dictation.md §19): the dictation modifiers with `E` for edit.
pub const DEFAULT_EDIT_HOTKEY: &str = "Ctrl+Alt+E";
/// Longest accepted chord text.
pub const MAX_HOTKEY_CHARS: usize = 48;

/// Modifier keys, in canonical display order.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modifier {
    /// `Ctrl` (Control on macOS as well; `Meta` is the Command key).
    Ctrl,
    /// `Alt` / Option.
    Alt,
    /// `Shift`.
    Shift,
    /// `Meta`: Windows key / Command.
    Meta,
}

impl Modifier {
    fn label(self) -> &'static str {
        match self {
            Self::Ctrl => "Ctrl",
            Self::Alt => "Alt",
            Self::Shift => "Shift",
            Self::Meta => "Meta",
        }
    }

    fn parse(token: &str) -> Option<Self> {
        match token.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => Some(Self::Ctrl),
            "alt" | "option" => Some(Self::Alt),
            "shift" => Some(Self::Shift),
            "meta" | "cmd" | "command" | "super" | "win" => Some(Self::Meta),
            _ => None,
        }
    }
}

/// A validated chord: at least one modifier plus exactly one non-modifier key.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Hotkey {
    /// Modifiers in canonical order (Ctrl, Alt, Shift, Meta), no duplicates.
    pub modifiers: Vec<Modifier>,
    /// The key: a single character (`A`, `1`, `/`) or a named key (`Space`, `F5`, `Enter`).
    pub key: String,
}

/// Why a chord text was refused.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum HotkeyError {
    /// Empty or too long.
    #[error("hotkey must be 1..={MAX_HOTKEY_CHARS} characters")]
    Length,
    /// No modifier at all (`Space`) — a bare key would swallow normal typing.
    #[error("hotkey needs at least one modifier (Ctrl / Alt / Shift / Meta)")]
    NoModifier,
    /// Only modifiers (`Ctrl+Alt`) — nothing to press.
    #[error("hotkey needs a key after the modifiers")]
    NoKey,
    /// More than one non-modifier key.
    #[error("hotkey may contain only one key")]
    TooManyKeys,
    /// A part between the `+` signs is empty.
    #[error("hotkey has an empty part")]
    EmptyPart,
}

const NAMED_KEYS: &[&str] = &[
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
];

impl Hotkey {
    /// Parse `Ctrl+Alt+Space` (case-insensitive modifiers, `Control`/`Option`/`Cmd` aliases).
    pub fn parse(text: &str) -> Result<Self, HotkeyError> {
        let n = text.chars().count();
        if n == 0 || n > MAX_HOTKEY_CHARS {
            return Err(HotkeyError::Length);
        }
        let mut modifiers = Vec::new();
        let mut key: Option<String> = None;
        for raw in text.split('+') {
            let part = raw.trim();
            if part.is_empty() {
                return Err(HotkeyError::EmptyPart);
            }
            if let Some(m) = Modifier::parse(part) {
                if !modifiers.contains(&m) {
                    modifiers.push(m);
                }
                continue;
            }
            if key.is_some() {
                return Err(HotkeyError::TooManyKeys);
            }
            key = Some(Self::canonical_key(part));
        }
        let Some(key) = key else { return Err(HotkeyError::NoKey) };
        if modifiers.is_empty() {
            return Err(HotkeyError::NoModifier);
        }
        modifiers.sort_by_key(|m| *m as u8);
        Ok(Self { modifiers, key })
    }

    fn canonical_key(part: &str) -> String {
        if part.chars().count() == 1 {
            return part.to_uppercase();
        }
        let lower = part.to_ascii_lowercase();
        if let Some(named) = NAMED_KEYS.iter().find(|k| k.to_ascii_lowercase() == lower) {
            return (*named).to_string();
        }
        if let Some(digits) = lower.strip_prefix('f')
            && !digits.is_empty()
            && digits.chars().all(|c| c.is_ascii_digit())
        {
            return format!("F{digits}");
        }
        part.to_string()
    }

    /// Whether two chord texts name the same chord once parsed (`alt+ctrl+e` = `Ctrl+Alt+E`); a
    /// text that does not parse is compared as written.
    pub fn same_chord(a: &str, b: &str) -> bool {
        match (Self::parse(a), Self::parse(b)) {
            (Ok(a), Ok(b)) => a == b,
            _ => a.trim() == b.trim(),
        }
    }

    /// Canonical display form: `Ctrl+Alt+Space`.
    pub fn display(&self) -> String {
        let mut parts: Vec<&str> = self.modifiers.iter().map(|m| m.label()).collect();
        parts.push(&self.key);
        parts.join("+")
    }

    /// Form understood by Tauri's global-shortcut plugin (`Control+Alt+Space`, `Super` for Meta).
    pub fn to_tauri_shortcut(&self) -> String {
        let mut parts: Vec<String> = self
            .modifiers
            .iter()
            .map(|m| match m {
                Modifier::Ctrl => "Control".to_string(),
                Modifier::Alt => "Alt".to_string(),
                Modifier::Shift => "Shift".to_string(),
                Modifier::Meta => "Super".to_string(),
            })
            .collect();
        let key = match self.key.as_str() {
            k if k.len() == 1 && k.chars().all(|c| c.is_ascii_digit()) => format!("Digit{k}"),
            k if k.len() == 1 && k.chars().all(|c| c.is_ascii_alphabetic()) => format!("Key{k}"),
            k => k.to_string(),
        };
        parts.push(key);
        parts.join("+")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_aliases_orders_modifiers_and_canonicalises_keys() {
        let h = Hotkey::parse("alt + control + space").unwrap();
        assert_eq!(h.display(), "Ctrl+Alt+Space");
        assert_eq!(h.to_tauri_shortcut(), "Control+Alt+Space");
        assert_eq!(Hotkey::parse("Cmd+Shift+d").unwrap().display(), "Shift+Meta+D");
        assert_eq!(Hotkey::parse("Ctrl+1").unwrap().to_tauri_shortcut(), "Control+Digit1");
        assert_eq!(Hotkey::parse("Ctrl+a").unwrap().to_tauri_shortcut(), "Control+KeyA");
        assert_eq!(Hotkey::parse("Ctrl+f5").unwrap().display(), "Ctrl+F5");
        assert_eq!(Hotkey::parse("Ctrl+Ctrl+Enter").unwrap().display(), "Ctrl+Enter");
        assert_eq!(Hotkey::parse(DEFAULT_HOTKEY).unwrap().display(), DEFAULT_HOTKEY);
        assert_eq!(Hotkey::parse(DEFAULT_EDIT_HOTKEY).unwrap().to_tauri_shortcut(), "Control+Alt+KeyE");
    }

    /// docs/dictation.md §19: the edit chord must differ from the dictation chord; the comparison
    /// is on the parsed chord, so spelling and modifier order do not hide a clash.
    #[test]
    fn same_chord_compares_parsed_chords() {
        assert!(Hotkey::same_chord("alt+control+e", DEFAULT_EDIT_HOTKEY));
        assert!(Hotkey::same_chord("Ctrl+Alt+Space", "control + option + space"));
        assert!(!Hotkey::same_chord(DEFAULT_HOTKEY, DEFAULT_EDIT_HOTKEY), "the two defaults do not clash");
        assert!(!Hotkey::same_chord("Ctrl+E", "Ctrl+Shift+E"));
        assert!(Hotkey::same_chord(" nonsense ", "nonsense"), "unparsable texts compare as written");
        assert!(!Hotkey::same_chord("Ctrl+E", "nonsense"));
    }

    #[test]
    fn refuses_bare_keys_modifier_only_and_malformed_text() {
        assert_eq!(Hotkey::parse("Space").unwrap_err(), HotkeyError::NoModifier);
        assert_eq!(Hotkey::parse("Ctrl+Alt").unwrap_err(), HotkeyError::NoKey);
        assert_eq!(Hotkey::parse("Ctrl+A+B").unwrap_err(), HotkeyError::TooManyKeys);
        assert_eq!(Hotkey::parse("Ctrl++A").unwrap_err(), HotkeyError::EmptyPart);
        assert_eq!(Hotkey::parse("").unwrap_err(), HotkeyError::Length);
        assert_eq!(Hotkey::parse(&"A+".repeat(40)).unwrap_err(), HotkeyError::Length);
    }
}
