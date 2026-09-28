//! The OS-facing ports: arboard for the clipboard, enigo for the paste chord. Deliberately thin;
//! every decision lives in the injector ([`crate::ClipboardPasteInjector`]) and [`crate::toolchain`], where it can be tested
//! without a display.

use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use enigo::{Direction, Enigo, Keyboard, Settings};

use crate::InjectError;
use crate::injector::{ClipboardPort, Delivered, DeliveryError, KeystrokePort, check_display};
use crate::toolchain::{Chord, Delivery, Key, Modifier};

/// [`check_display`] against this process's environment and platform.
pub fn display_available() -> Result<(), InjectError> {
    check_display(|name| std::env::var(name).ok(), std::env::consts::OS)
}

/// The system clipboard through arboard.
///
/// One handle is opened on first use and kept for the life of the injector: on X11 the process
/// that wrote the clipboard must stay its owner, and arboard hands the content off (or loses it,
/// without a clipboard manager) as soon as its last handle is dropped. arboard's handle is
/// `Send + Sync`, so the restore thread shares it through the mutex. On a Wayland session arboard
/// uses wlr-data-control when the compositor offers it (`wayland-data-control` feature) and falls
/// back to X11 through XWayland otherwise.
#[derive(Default)]
pub struct SystemClipboard {
    handle: Mutex<Option<arboard::Clipboard>>,
}

impl SystemClipboard {
    /// Nothing is opened until the first read or write.
    pub fn new() -> Self {
        Self::default()
    }

    fn with_handle<T>(&self, op: impl FnOnce(&mut arboard::Clipboard) -> Result<T, arboard::Error>) -> Result<T, InjectError> {
        display_available()?;
        let mut guard = self.handle.lock().unwrap_or_else(PoisonError::into_inner);
        if guard.is_none() {
            *guard = Some(arboard::Clipboard::new().map_err(|e| InjectError::Clipboard(e.to_string()))?);
        }
        let Some(clipboard) = guard.as_mut() else { return Err(InjectError::Clipboard("clipboard handle missing".into())) };
        op(clipboard).map_err(|e| {
            // The failure may mean the display connection is gone; reopen on the next call.
            *guard = None;
            InjectError::Clipboard(e.to_string())
        })
    }
}

impl ClipboardPort for SystemClipboard {
    fn read_text(&self) -> Result<Option<String>, InjectError> {
        self.with_handle(|clipboard| match clipboard.get_text() {
            Ok(text) => Ok(Some(text)),
            Err(arboard::Error::ContentNotAvailable) => Ok(None),
            Err(e) => Err(e),
        })
    }

    fn write_text(&self, text: &str) -> Result<(), InjectError> {
        self.with_handle(|clipboard| clipboard.set_text(text))
    }

    fn clear(&self) -> Result<(), InjectError> {
        self.with_handle(arboard::Clipboard::clear)
    }
}

/// Which enigo connection [`SystemKeys`] opens. With both Linux features compiled in, enigo
/// would otherwise open X11 *and* Wayland and send every key through both — a double paste on
/// XWayland sessions — so exactly one is enabled per call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnigoBackend {
    /// The platform default (Windows `SendInput`, macOS `CGEvent`). On Linux: X11 when an X server
    /// is reachable (`DISPLAY`), the virtual keyboard otherwise ([`resolve_backend`]).
    Native,
    /// Linux: x11rb / XTEST only.
    X11,
    /// Linux: `zwp_virtual_keyboard_v1` only.
    Wayland,
}

impl EnigoBackend {
    /// Name for logs and the `Delivered.tool` field.
    pub fn name(self) -> &'static str {
        match self {
            Self::Native => "enigo",
            Self::X11 => "enigo-x11",
            Self::Wayland => "enigo-wayland",
        }
    }
}

/// A Wayland socket name that cannot exist under `XDG_RUNTIME_DIR`: enigo's Wayland connection
/// fails on it deterministically, which is how the X11-only backend is selected.
pub const NO_WAYLAND: &str = "voltip-no-wayland-socket";
/// An X11 display string x11rb cannot parse: the same trick for the Wayland-only backend.
pub const NO_X11: &str = "";

/// The connection `backend` stands for on `os`: on Linux [`EnigoBackend::Native`] becomes exactly
/// one of the two (X11 when `x11_reachable`), elsewhere everything is the native connection.
pub fn resolve_backend(backend: EnigoBackend, os: &str, x11_reachable: bool) -> EnigoBackend {
    match (os, backend) {
        ("linux", EnigoBackend::Native) if x11_reachable => EnigoBackend::X11,
        ("linux", EnigoBackend::Native) => EnigoBackend::Wayland,
        ("linux", other) => other,
        _ => EnigoBackend::Native,
    }
}

/// enigo settings for `backend` (already [`resolve_backend`]d). The Accessibility prompt is never
/// opened from here (`open_prompt_to_get_permissions = false`; only macOS reads it): without the
/// permission the paste is unavailable and the text stays on the clipboard, and the prompt comes
/// from the onboarding step's 请求授权 button instead of from the middle of a dictation.
pub fn enigo_settings(backend: EnigoBackend) -> Settings {
    let mut settings = Settings { open_prompt_to_get_permissions: false, ..Settings::default() };
    match backend {
        EnigoBackend::Native => {}
        EnigoBackend::X11 => settings.wayland_display = Some(NO_WAYLAND.into()),
        EnigoBackend::Wayland => settings.x11_display = Some(NO_X11.into()),
    }
    settings
}

/// Why enigo could not press a chord.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnigoError {
    /// No connection (no display, no protocol, no accessibility permission): nothing was sent.
    Connect(String),
    /// The connection exists but an event was refused.
    Input(String),
}

impl std::fmt::Display for EnigoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connect(e) => write!(f, "no input connection: {e}"),
            Self::Input(e) => write!(f, "input refused: {e}"),
        }
    }
}

/// A key of a [`KeyStep`], independent of enigo's per-platform key enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepKey {
    /// A modifier.
    Modifier(Modifier),
    /// A character, left to enigo's layout lookup (Windows, Linux).
    Char(char),
    /// The Insert key (not on macOS).
    Insert,
    /// A platform virtual key code, sent as is: a macOS `CGKeyCode`, or a Windows `VK_*` code
    /// ([`WINDOWS_VK_C`]).
    Code(u16),
}

/// One event of a synthesised chord.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyStep {
    /// Key down.
    Press(StepKey),
    /// Key up.
    Release(StepKey),
    /// Key down and up.
    Click(StepKey),
    /// Keep everything that is down, down this long.
    Hold(Duration),
}

/// The events enigo sends for `chord` on `os` ([`std::env::consts::OS`]).
///
/// * Windows / Linux: modifiers down in order, the key clicked, modifiers up in reverse.
/// * macOS: the `v` goes out as the virtual keycode `mac_v_keycode`
///   (`voltip_platform::macos::cmd_v_keycode`), never as `Key::Unicode('v')` — enigo's layout
///   lookup answers keycode 0 (`kVK_ANSI_A`) when no key of the current layout types `v` (Russian
///   and other non-Latin layouts), which turns Cmd+V into Cmd+A, select all. The key is then held
///   `voltip_platform::macos::PASTE_HOLD_MS` before key and modifiers are released: shorter, and
///   Electron / Java apps see a bare `v`.
pub fn chord_steps(chord: Chord, os: &str, mac_v_keycode: u16) -> Vec<KeyStep> {
    let modifiers = chord.modifiers();
    let mut steps: Vec<KeyStep> = modifiers.iter().map(|m| KeyStep::Press(StepKey::Modifier(*m))).collect();
    match (os, chord.key) {
        ("macos", Key::Char('v')) => {
            let v = StepKey::Code(mac_v_keycode);
            steps.push(KeyStep::Press(v));
            steps.push(KeyStep::Hold(Duration::from_millis(voltip_platform::macos::PASTE_HOLD_MS)));
            steps.push(KeyStep::Release(v));
        }
        (_, Key::Char(c)) => steps.push(KeyStep::Click(StepKey::Char(c))),
        (_, Key::Insert) => steps.push(KeyStep::Click(StepKey::Insert)),
    }
    steps.extend(modifiers.iter().rev().map(|m| KeyStep::Release(StepKey::Modifier(*m))));
    steps
}

/// The `v` keycode used when the shell supplies no `UCKeyTranslate` result:
/// `cmd_v_keycode(None, "")` = `kVK_ANSI_V` (9), right on every layout but pure Dvorak.
pub fn default_mac_v_keycode() -> u16 {
    voltip_platform::macos::cmd_v_keycode(None, "")
}

/// The `c` keycode of the selection copy without a `UCKeyTranslate` result:
/// `cmd_c_keycode(None, "")` = `kVK_ANSI_C` (8).
pub fn default_mac_c_keycode() -> u16 {
    voltip_platform::macos::cmd_c_keycode(None, "")
}

/// The events enigo sends for the selection copy `chord` on `os` (docs/dictation.md §19), after
/// letting go of `held` — the hotkey's modifiers the user may still be holding (`Alt` of
/// `Ctrl+Alt+E`), which would otherwise turn Ctrl+Insert into Ctrl+Alt+Insert:
///
/// * Windows / Linux: the chord's modifiers down, then every other `held` modifier released (after
///   the chord's own went down, so no modifier is ever released alone: a lone Alt release opens
///   the menu bar in Win32 applications), the key clicked (`Insert`: no layout decides it), modifiers
///   up.
/// * macOS: `held` is ignored — enigo posts from a private event source with explicit flags, so the
///   physically held keys do not mix in; `c` goes out as `mac_c_keycode` and is held
///   `PASTE_HOLD_MS` like the paste's `v`.
pub fn copy_steps(chord: Chord, os: &str, mac_c_keycode: u16, held: &[Modifier]) -> Vec<KeyStep> {
    let modifiers = chord.modifiers();
    let mut steps: Vec<KeyStep> = modifiers.iter().map(|m| KeyStep::Press(StepKey::Modifier(*m))).collect();
    if os != "macos" {
        let mut released: Vec<Modifier> = Vec::new();
        for m in held {
            if !modifiers.contains(m) && !released.contains(m) {
                released.push(*m);
                steps.push(KeyStep::Release(StepKey::Modifier(*m)));
            }
        }
    }
    match (os, chord.key) {
        ("macos", Key::Char('c')) => {
            let c = StepKey::Code(mac_c_keycode);
            steps.push(KeyStep::Press(c));
            steps.push(KeyStep::Hold(Duration::from_millis(voltip_platform::macos::PASTE_HOLD_MS)));
            steps.push(KeyStep::Release(c));
        }
        (_, Key::Char(c)) => steps.push(KeyStep::Click(StepKey::Char(c))),
        (_, Key::Insert) => steps.push(KeyStep::Click(StepKey::Insert)),
    }
    steps.extend(modifiers.iter().rev().map(|m| KeyStep::Release(StepKey::Modifier(*m))));
    steps
}

/// Synthetic keyboard input through enigo (`SendInput`, `CGEvent`, x11rb / XTEST,
/// `zwp_virtual_keyboard_v1`).
#[derive(Clone, Copy, Debug)]
pub struct SystemKeys {
    backend: EnigoBackend,
    mac_v_keycode: u16,
    mac_c_keycode: u16,
    /// macOS: take the keycodes from the current layout at every keystroke ([`mac_layout_id`])
    /// instead of the two fields; cleared by an explicit `with_mac_*_keycode`.
    mac_layout_live: bool,
}

/// The current keyboard layout's input source id on macOS (`com.apple.keylayout.US`,
/// `com.apple.keylayout.Dvorak`), as the input menu records it in the HIToolbox preference domain
/// (`AppleCurrentKeyboardLayoutInputSourceID`). NSUserDefaults is safe on any thread — the TIS
/// calls behind a per-key `UCKeyTranslate` lookup want the main thread, and the paste runs on a
/// worker. `None` elsewhere, or when the preference is unset.
pub fn mac_layout_id() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        use objc2::AllocAnyThread as _;
        use objc2_foundation::{NSUserDefaults, ns_string};
        let defaults = NSUserDefaults::initWithSuiteName(NSUserDefaults::alloc(), Some(ns_string!("com.apple.HIToolbox")))?;
        defaults.stringForKey(ns_string!("AppleCurrentKeyboardLayoutInputSourceID")).map(|id| id.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// The macOS `(v, c)` keycodes of the paste and the copy for `layout_id` (the input-source tables of
/// `voltip_platform::macos`: Dvorak moves both keys, every other layout keeps the ANSI positions).
pub fn mac_keycodes_for(layout_id: Option<&str>) -> (u16, u16) {
    let id = layout_id.unwrap_or_default();
    (voltip_platform::macos::cmd_v_keycode(None, id), voltip_platform::macos::cmd_c_keycode(None, id))
}

impl SystemKeys {
    /// The platform default connection.
    pub fn native() -> Self {
        Self::with_backend(EnigoBackend::Native)
    }

    /// A specific Linux connection.
    pub fn with_backend(backend: EnigoBackend) -> Self {
        Self { backend, mac_v_keycode: default_mac_v_keycode(), mac_c_keycode: default_mac_c_keycode(), mac_layout_live: true }
    }

    /// A fixed macOS `v` keycode (`voltip_platform::macos::cmd_v_keycode(Some(found), layout_id)`)
    /// instead of the live layout; ignored elsewhere.
    pub fn with_mac_v_keycode(self, mac_v_keycode: u16) -> Self {
        Self { mac_v_keycode, mac_layout_live: false, ..self }
    }

    /// Whether the macOS keycodes follow the current layout at every keystroke.
    pub fn mac_layout_live(&self) -> bool {
        self.mac_layout_live
    }

    /// The `(v, c)` keycodes a keystroke uses now: the live layout's on macOS, the fields otherwise.
    fn mac_keycodes(&self) -> (u16, u16) {
        if cfg!(target_os = "macos") && self.mac_layout_live { mac_keycodes_for(mac_layout_id().as_deref()) } else { (self.mac_v_keycode, self.mac_c_keycode) }
    }

    /// The macOS `v` keycode in force.
    pub fn mac_v_keycode(&self) -> u16 {
        self.mac_v_keycode
    }

    /// The macOS `c` keycode of the selection copy (`voltip_platform::macos::cmd_c_keycode`);
    /// ignored elsewhere.
    pub fn with_mac_c_keycode(self, mac_c_keycode: u16) -> Self {
        Self { mac_c_keycode, mac_layout_live: false, ..self }
    }

    /// The macOS `c` keycode in force.
    pub fn mac_c_keycode(&self) -> u16 {
        self.mac_c_keycode
    }

    /// The connection this port opens.
    pub fn backend(&self) -> EnigoBackend {
        self.backend
    }

    /// Open the connection without sending anything (the availability probe).
    pub fn connect(&self) -> Result<Enigo, EnigoError> {
        display_available().map_err(|e| EnigoError::Connect(e.to_string()))?;
        let x11_reachable = std::env::var("DISPLAY").is_ok_and(|d| !d.trim().is_empty());
        let backend = resolve_backend(self.backend, std::env::consts::OS, x11_reachable);
        Enigo::new(&enigo_settings(backend)).map_err(|e| EnigoError::Connect(e.to_string()))
    }

    /// Press `chord` once ([`chord_steps`] for this platform). Every key a failed step leaves down is
    /// released before the error returns, so the user's keyboard is never left stuck.
    pub fn press(&self, chord: Chord) -> Result<(), EnigoError> {
        self.run(&chord_steps(chord, std::env::consts::OS, self.mac_keycodes().0))
    }

    /// Press the selection copy `chord` after letting go of `held` ([`copy_steps`] for this
    /// platform, docs/dictation.md §19). Same cleanup as [`SystemKeys::press`].
    pub fn copy(&self, chord: Chord, held: &[Modifier]) -> Result<(), EnigoError> {
        self.run(&copy_steps(chord, std::env::consts::OS, self.mac_keycodes().1, held))
    }

    fn run(&self, steps: &[KeyStep]) -> Result<(), EnigoError> {
        // Map every key first: a chord this platform cannot send is refused before any connection.
        let mut plan = Vec::with_capacity(steps.len());
        for step in steps {
            plan.push(match *step {
                KeyStep::Press(k) => (Some((enigo_key(k)?, Direction::Press)), None),
                KeyStep::Release(k) => (Some((enigo_key(k)?, Direction::Release)), None),
                KeyStep::Click(k) => (Some((enigo_key(k)?, Direction::Click)), None),
                KeyStep::Hold(d) => (None, Some(d)),
            });
        }
        let mut enigo = self.connect()?;
        let mut down: Vec<enigo::Key> = Vec::new();
        let mut result = Ok(());
        for (event, hold) in plan {
            if let Some(d) = hold {
                std::thread::sleep(d);
            }
            let Some((key, direction)) = event else { continue };
            if let Err(e) = enigo.key(key, direction) {
                result = Err(EnigoError::Input(e.to_string()));
                break;
            }
            match direction {
                Direction::Press => down.push(key),
                Direction::Release => down.retain(|k| *k != key),
                Direction::Click => {}
            }
        }
        for key in down.into_iter().rev() {
            if let Err(e) = enigo.key(key, Direction::Release) {
                tracing::warn!(error = %e, ?key, "could not release a key after a failed chord");
            }
        }
        result
    }
}

/// The enigo key for a [`StepKey`]. macOS has no Insert key ([`crate::PasteMethod::ShiftInsert`]
/// never produces it there); asking for one anyway is refused before any connection is opened.
fn enigo_key(key: StepKey) -> Result<enigo::Key, EnigoError> {
    match key {
        StepKey::Modifier(Modifier::Control) => Ok(enigo::Key::Control),
        StepKey::Modifier(Modifier::Meta) => Ok(enigo::Key::Meta),
        StepKey::Modifier(Modifier::Shift) => Ok(enigo::Key::Shift),
        StepKey::Modifier(Modifier::Alt) => Ok(enigo::Key::Alt),
        StepKey::Char(c) => Ok(enigo::Key::Unicode(c)),
        StepKey::Code(code) => Ok(enigo::Key::Other(u32::from(code))),
        #[cfg(not(target_os = "macos"))]
        StepKey::Insert => Ok(enigo::Key::Insert),
        #[cfg(target_os = "macos")]
        StepKey::Insert => Err(EnigoError::Input("macOS has no Insert key".into())),
    }
}

impl SystemKeys {
    /// An enigo outcome as the injectors see it: no connection is [`DeliveryError::Unavailable`]
    /// (nothing was sent), a refused event is [`DeliveryError::Failed`].
    fn delivered(&self, outcome: Result<(), EnigoError>) -> Result<Delivered, DeliveryError> {
        match outcome {
            Ok(()) => Ok(Delivered { tool: self.backend.name().to_string(), delivery: Delivery::Chord }),
            Err(EnigoError::Connect(e)) => Err(DeliveryError::Unavailable(format!("{}: no input connection: {e}", self.backend.name()))),
            Err(EnigoError::Input(e)) => Err(DeliveryError::Failed(format!("{}: input refused: {e}", self.backend.name()))),
        }
    }
}

impl KeystrokePort for SystemKeys {
    fn deliver(&self, chord: Chord, _text: &str) -> Result<Delivered, DeliveryError> {
        self.delivered(self.press(chord))
    }

    fn press_chord(&self, chord: Chord, held: &[Modifier]) -> Result<Delivered, DeliveryError> {
        self.delivered(self.copy(chord, held))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::{ClipboardOnlyInjector, ClipboardPasteInjector, Injector};

    /// Headless: the ports fail typed, never panic. (The display-backed round trip lives in
    /// `tests/real_display.rs` and is `#[ignore]`d.)
    #[test]
    fn headless_ports_fail_typed() {
        if display_available().is_ok() {
            return;
        }
        let clipboard = SystemClipboard::new();
        assert!(matches!(clipboard.read_text(), Err(InjectError::NoDisplay(_))));
        assert!(matches!(clipboard.write_text("x"), Err(InjectError::NoDisplay(_))));
        for backend in [EnigoBackend::Native, EnigoBackend::X11, EnigoBackend::Wayland] {
            let keys = SystemKeys::with_backend(backend);
            assert_eq!(keys.backend(), backend);
            assert!(matches!(keys.connect(), Err(EnigoError::Connect(_))), "{backend:?}");
            assert!(matches!(keys.press(crate::paste_chord()), Err(EnigoError::Connect(_))), "{backend:?}");
            let err = keys.deliver(crate::paste_chord(), "x").unwrap_err();
            assert!(matches!(&err, DeliveryError::Unavailable(m) if m.starts_with(backend.name())), "{err:?}");
        }
        assert!(matches!(ClipboardPasteInjector::new(Duration::ZERO).inject("x"), Err(InjectError::NoDisplay(_))));
        assert!(matches!(ClipboardOnlyInjector::default().inject("x"), Err(InjectError::NoDisplay(_))));
    }

    #[test]
    fn chord_steps_table() {
        use crate::PasteMethod::{CtrlShiftV, CtrlV, ShiftInsert};
        use KeyStep::{Click, Hold, Press, Release};
        use StepKey::{Char, Code, Insert};
        let ctrl = StepKey::Modifier(Modifier::Control);
        let meta = StepKey::Modifier(Modifier::Meta);
        let shift = StepKey::Modifier(Modifier::Shift);
        let hold = Hold(Duration::from_millis(100));
        let cases: Vec<(&str, crate::PasteMethod, u16, Vec<KeyStep>)> = vec![
            ("linux", CtrlV, 9, vec![Press(ctrl), Click(Char('v')), Release(ctrl)]),
            ("windows", CtrlV, 9, vec![Press(ctrl), Click(Char('v')), Release(ctrl)]),
            ("linux", CtrlShiftV, 9, vec![Press(ctrl), Press(shift), Click(Char('v')), Release(shift), Release(ctrl)]),
            ("linux", ShiftInsert, 9, vec![Press(shift), Click(Insert), Release(shift)]),
            ("windows", ShiftInsert, 9, vec![Press(shift), Click(Insert), Release(shift)]),
            // macOS: the keycode, never Unicode('v'); held 100 ms before both keys go up.
            ("macos", CtrlV, 9, vec![Press(meta), Press(Code(9)), hold, Release(Code(9)), Release(meta)]),
            ("macos", CtrlV, 47, vec![Press(meta), Press(Code(47)), hold, Release(Code(47)), Release(meta)]),
            ("macos", CtrlShiftV, 9, vec![Press(meta), Press(shift), Press(Code(9)), hold, Release(Code(9)), Release(shift), Release(meta)]),
            ("macos", ShiftInsert, 9, vec![Press(meta), Press(Code(9)), hold, Release(Code(9)), Release(meta)]),
        ];
        for (os, method, code, expected) in cases {
            assert_eq!(chord_steps(method.chord_for(os), os, code), expected, "{os} {method:?} {code}");
        }
        // Regression (Russian layout → Cmd+A): no macOS chord leaves the `v` to enigo's layout lookup.
        for method in [CtrlV, CtrlShiftV, ShiftInsert] {
            assert!(
                chord_steps(method.chord_for("macos"), "macos", default_mac_v_keycode()).iter().all(|s| !matches!(s, Press(Char(_)) | Click(Char(_)))),
                "{method:?}"
            );
        }
        assert_eq!(default_mac_v_keycode(), voltip_platform::macos::ANSI_V_KEYCODE);
        let keys = SystemKeys::native();
        assert_eq!(keys.mac_v_keycode(), 9);
        assert_eq!(keys.with_mac_v_keycode(47).mac_v_keycode(), 47);
        assert_eq!(keys.with_mac_v_keycode(47).backend(), EnigoBackend::Native);
    }

    /// docs/dictation.md §19: the copy chord per platform. Windows / Linux press the chord's Ctrl
    /// first, then let go of the other held hotkey modifiers (never one alone), click `c` (`VK_C`
    /// on Windows) and release; macOS sends the `c` keycode held 100 ms and ignores `held`.
    #[test]
    fn copy_steps_table() {
        use KeyStep::{Click, Hold, Press, Release};
        use StepKey::{Char, Code};
        let ctrl = StepKey::Modifier(Modifier::Control);
        let meta = StepKey::Modifier(Modifier::Meta);
        let alt = StepKey::Modifier(Modifier::Alt);
        let shift = StepKey::Modifier(Modifier::Shift);
        let copy = crate::copy_chord_for;
        // Windows and Linux copy with Ctrl+Insert (never Ctrl+C, a terminal's interrupt).
        let ins = StepKey::Insert;
        let cases: Vec<(&str, u16, Vec<Modifier>, Vec<KeyStep>)> = vec![
            ("windows", 8, vec![], vec![Press(ctrl), Click(ins), Release(ctrl)]),
            ("windows", 8, vec![Modifier::Control, Modifier::Alt], vec![Press(ctrl), Release(alt), Click(ins), Release(ctrl)]),
            (
                "windows",
                8,
                vec![Modifier::Alt, Modifier::Shift, Modifier::Meta, Modifier::Alt],
                vec![Press(ctrl), Release(alt), Release(shift), Release(meta), Click(ins), Release(ctrl)],
            ),
            ("linux", 8, vec![Modifier::Alt], vec![Press(ctrl), Release(alt), Click(ins), Release(ctrl)]),
            ("linux", 8, vec![], vec![Press(ctrl), Click(ins), Release(ctrl)]),
            (
                "macos",
                8,
                vec![Modifier::Control, Modifier::Alt],
                vec![Press(meta), Press(Code(8)), Hold(Duration::from_millis(100)), Release(Code(8)), Release(meta)],
            ),
            ("macos", 34, vec![], vec![Press(meta), Press(Code(34)), Hold(Duration::from_millis(100)), Release(Code(34)), Release(meta)]),
        ];
        for (os, keycode, held, expected) in cases {
            assert_eq!(copy_steps(copy(os), os, keycode, &held), expected, "{os} {keycode} {held:?}");
        }
        // A character chord key goes out as the character.
        let other = Chord { key: Key::Char('x'), ..copy("windows") };
        assert_eq!(copy_steps(other, "windows", 8, &[]), vec![Press(ctrl), Click(Char('x')), Release(ctrl)]);
        assert_eq!(default_mac_c_keycode(), voltip_platform::macos::ANSI_C_KEYCODE);
        let keys = SystemKeys::native();
        assert_eq!(keys.mac_c_keycode(), 8);
        assert_eq!(keys.with_mac_c_keycode(34).mac_c_keycode(), 34);
        assert_eq!(enigo_key(alt), Ok(enigo::Key::Alt));
        if display_available().is_err() {
            assert!(matches!(keys.copy(copy(std::env::consts::OS), &[Modifier::Alt]), Err(EnigoError::Connect(_))));
            assert!(matches!(keys.press_chord(copy(std::env::consts::OS), &[]), Err(DeliveryError::Unavailable(m)) if m.starts_with("enigo")));
        }
    }

    /// docs/dictation.md §15.2: on macOS the keystrokes follow the layout in force (found in
    /// review: pure Dvorak got the ANSI positions, 9 / 8, which are `.` / `j` there).
    #[test]
    fn regression_the_mac_keycodes_follow_the_current_layout() {
        use voltip_platform::macos::{ANSI_C_KEYCODE, ANSI_V_KEYCODE, DVORAK_C_KEYCODE, DVORAK_V_KEYCODE};
        assert_eq!(mac_keycodes_for(Some("com.apple.keylayout.Dvorak")), (DVORAK_V_KEYCODE, DVORAK_C_KEYCODE));
        for id in [Some("com.apple.keylayout.US"), Some("com.apple.keylayout.ABC"), Some("com.apple.keylayout.DVORAK-QWERTYCMD"), Some(""), None] {
            assert_eq!(mac_keycodes_for(id), (ANSI_V_KEYCODE, ANSI_C_KEYCODE), "{id:?}");
        }
        // Live by default; a fixed keycode from the shell wins.
        let keys = SystemKeys::native();
        assert!(keys.mac_layout_live());
        assert!(!keys.with_mac_v_keycode(47).mac_layout_live() && !keys.with_mac_c_keycode(34).mac_layout_live());
        assert_eq!(keys.with_mac_v_keycode(47).mac_keycodes().0, 47);
        // Off macOS nothing is read. On a Mac the preference names the layout once the input menu
        // has recorded one; an account that never chose a layout has none (GitHub's macos-15-intel
        // image, 2026-09-28), and the paste keeps the ANSI keys (`mac_keycodes_for(None)` above).
        if cfg!(target_os = "macos") {
            let id = mac_layout_id();
            assert!(id.as_deref().is_none_or(|id| id.starts_with("com.apple.") || id.contains('.')), "{id:?}");
        } else {
            assert_eq!(mac_layout_id(), None);
        }
    }

    #[test]
    fn backend_names_settings_and_errors() {
        assert_eq!(SystemKeys::native().backend(), EnigoBackend::Native);
        assert_eq!(EnigoBackend::Native.name(), "enigo");
        assert_eq!(EnigoBackend::X11.name(), "enigo-x11");
        assert_eq!(EnigoBackend::Wayland.name(), "enigo-wayland");
        use EnigoBackend::{Native, Wayland, X11};
        assert_eq!(resolve_backend(Native, "linux", true), X11);
        assert_eq!(resolve_backend(Native, "linux", false), Wayland);
        assert_eq!(resolve_backend(X11, "linux", false), X11);
        assert_eq!(resolve_backend(Wayland, "linux", true), Wayland);
        assert_eq!(resolve_backend(Native, "windows", true), Native);
        assert_eq!(resolve_backend(X11, "macos", true), Native, "the Linux split means nothing elsewhere");
        let native = enigo_settings(EnigoBackend::Native);
        assert_eq!((native.x11_display.as_deref(), native.wayland_display.as_deref()), (None, None));
        let x11 = enigo_settings(EnigoBackend::X11);
        assert_eq!((x11.x11_display.as_deref(), x11.wayland_display.as_deref()), (None, Some(NO_WAYLAND)));
        let wayland = enigo_settings(EnigoBackend::Wayland);
        assert_eq!((wayland.x11_display.as_deref(), wayland.wayland_display.as_deref()), (Some(NO_X11), None));
        assert_eq!(enigo_key(StepKey::Char('v')), Ok(enigo::Key::Unicode('v')));
        assert_eq!(enigo_key(StepKey::Code(9)), Ok(enigo::Key::Other(9)));
        assert_eq!(enigo_key(StepKey::Modifier(Modifier::Meta)), Ok(enigo::Key::Meta));
        assert_eq!(enigo_key(StepKey::Modifier(Modifier::Shift)), Ok(enigo::Key::Shift));
        assert_eq!(enigo_key(StepKey::Modifier(Modifier::Control)), Ok(enigo::Key::Control));
        #[cfg(not(target_os = "macos"))]
        assert_eq!(enigo_key(StepKey::Insert), Ok(enigo::Key::Insert));
        assert!(!enigo_settings(EnigoBackend::Native).open_prompt_to_get_permissions, "the prompt comes from onboarding, not from a paste");
        assert!(!enigo_settings(EnigoBackend::X11).open_prompt_to_get_permissions);
        assert_eq!(EnigoError::Connect("x".into()).to_string(), "no input connection: x");
        assert_eq!(EnigoError::Input("y".into()).to_string(), "input refused: y");
    }
}
