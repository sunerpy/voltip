//! Global hotkeys: the core owns the chord texts (`Settings.hotkey`, and `Settings.edit_hotkey` for
//! voice edit, docs/dictation.md §19), this module owns the OS registration through the
//! global-shortcut plugin and reports back through the bridge as [`UiEvent::Hotkey`], so
//! `core_state` and the event stream tell the settings page the truth (registered / conflict /
//! pressed) instead of a fixture. Every press and release goes to the core as a `HotkeyEdge`
//! (docs/dictation.md §13) tagged with its purpose (dictation or edit); the core's activation
//! machines decide what it means (`hold` / `toggle` / `hold_or_toggle`) and the pill follows the
//! dictation phase, not the key.
//!
//! The cancel key (docs/dictation.md §5: 取消 at any moment) is `Escape`, registered only while a
//! take is listening or processing, so it never takes Esc from other applications otherwise; its
//! press is a `DictationCancel`. Every backend matches modifiers exactly (`XGrabKey`,
//! `RegisterHotKey`, Carbon), and a `hold` take is cancelled with the chord still down, so Esc is
//! also registered under each chord's modifiers (`Control+Alt+Escape` for `Ctrl+Alt+Space`).
//!
//! The lone-key trigger (`Settings.solo_key`, docs/dictation.md §13.1) is watched by
//! [`crate::solo_key`] next to the chords, with the same lifecycle: installed by [`apply`],
//! suspended while the recorder is open, reported as `solo_registered` / `solo_error` /
//! `solo_pressed`. Its edges are dictation `HotkeyEdge`s; a chord made with it (Right Ctrl + C)
//! goes to the core as `chorded`, which cancels the take that press started.
//!
//! Linux (docs/dictation.md §14): the plugin grabs keys through X11 (`XGrabKey`). The session kind
//! goes into `HotkeyStatus.backend` (`global-shortcut · Linux · XWayland`). On a pure Wayland
//! session nothing is registered and `HotkeyStatus.error` tells the user to bind `--toggle` in the
//! compositor; under XWayland the grab only fires while an X11 window has focus, which is logged.

use std::sync::{Arc, Weak};

use parking_lot::Mutex;
use tauri::{AppHandle, Runtime};
use tauri_plugin_global_shortcut::{GlobalShortcutExt as _, ShortcutState};
use voltip_core::ui::{HotkeyCapabilities, HotkeyStatus, UiEvent};
use voltip_core::{DictationPhase, EdgeSource, Hotkey, Modifier, SelectionTiming, Settings, SoloKey, TakeKind, now_ms};
use voltip_inject::{Session, SessionKind, X11Grab};
use voltip_platform::solo_key::SoloEdge;
use voltip_tauri_bridge::{Bridge, UiCommand};

use crate::solo_key::{self, SoloHook};

/// The core command for a key transition of the `purpose` key: a `HotkeyEdge` stamped with the
/// core's clock. The activation machine (`Settings.activation`) turns it into start / stop / lock.
pub fn edge_command(pressed: bool, source: EdgeSource, purpose: TakeKind) -> UiCommand {
    UiCommand::HotkeyEdge { pressed, at_ms: now_ms(), source, purpose, chorded: false }
}

/// What the shell registers: the dictation hotkey, the voice-edit hotkey (`None` = off) and the
/// lone-key trigger (`None` = off).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chords {
    /// `Settings.hotkey`.
    pub hotkey: String,
    /// `Settings.edit_hotkey`.
    pub edit_hotkey: Option<String>,
    /// `Settings.solo_key`.
    pub solo_key: Option<SoloKey>,
}

impl From<&Settings> for Chords {
    fn from(settings: &Settings) -> Self {
        Self { hotkey: settings.hotkey.clone(), edit_hotkey: settings.edit_hotkey.clone(), solo_key: settings.solo_key }
    }
}

/// The chord modifier a lone modifier key stands for: what a `hold` take keeps down, so the
/// cancel key is registered under it too. Fn and the mouse buttons have none.
pub fn solo_modifier(key: SoloKey) -> Option<Modifier> {
    match key {
        SoloKey::RightCtrl => Some(Modifier::Ctrl),
        SoloKey::RightAlt => Some(Modifier::Alt),
        SoloKey::RightShift => Some(Modifier::Shift),
        SoloKey::RightMeta => Some(Modifier::Meta),
        SoloKey::Fn | SoloKey::MouseMiddle | SoloKey::MouseBack | SoloKey::MouseForward => None,
    }
}

/// When the voice edit's copy chord can reach the application in `session` (docs/dictation.md
/// §19): the X11 key grab (X11, and XWayland where the grab is served by XWayland) routes every key
/// event to the grabbing client while the hotkey's key is down, so the copy waits for the key-up;
/// everywhere else (Windows, macOS, pure Wayland's compositor shortcut) it goes out at the press.
pub fn selection_timing_for(session: Option<SessionKind>) -> SelectionTiming {
    match session.map(SessionKind::x11_grab) {
        Some(X11Grab::Everywhere | X11Grab::X11WindowsOnly) => SelectionTiming::AfterKeyUp,
        Some(X11Grab::Unavailable) | None => SelectionTiming::AtPress,
    }
}

/// Registration backend as shown on the settings page, for the platform this binary was built for
/// and (on Linux) the session it runs in.
pub fn backend_name() -> String {
    backend_name_for(std::env::consts::OS, linux_session().map(|s| s.kind))
}

/// [`backend_name`] as a pure function of the OS name ([`std::env::consts::OS`]) and the Linux
/// session kind: `global-shortcut · Windows · RegisterHotKey`, `global-shortcut · macOS · Carbon`,
/// `global-shortcut · Linux · X11` / `· XWayland` / `· Wayland`. Without a detected session
/// (headless) the Linux backend is named after what the plugin speaks, X11.
pub fn backend_name_for(os: &str, session: Option<SessionKind>) -> String {
    let platform = match os {
        "windows" => "Windows · RegisterHotKey".to_string(),
        "macos" => "macOS · Carbon".to_string(),
        _ => format!("Linux · {}", session.unwrap_or(SessionKind::X11)),
    };
    format!("global-shortcut · {platform}")
}

/// [`HotkeyCapabilities`] of this binary in the session it runs in.
pub fn capabilities() -> HotkeyCapabilities {
    capabilities_for(std::env::consts::OS, linux_session().map(|s| s.kind), toggle_command(), edit_toggle_command())
}

/// [`capabilities`] as a pure function: every backend that registers (RegisterHotKey on Windows,
/// Carbon on macOS, the X11 key grab) reports both edges, so press-and-hold works; XWayland's grab
/// fires only while an X11 window has the focus; a pure Wayland session registers nothing, and the
/// compositor shortcut that replaces it runs `--toggle` on the press only. Linux without a detected
/// session is taken as X11, like [`backend_name_for`].
pub fn capabilities_for(os: &str, session: Option<SessionKind>, toggle_command: String, edit_toggle_command: String) -> HotkeyCapabilities {
    let grab = if os == "linux" { session.map(SessionKind::x11_grab) } else { None };
    let (global, everywhere) = match grab {
        Some(X11Grab::Unavailable) => (false, false),
        Some(X11Grab::X11WindowsOnly) => (true, false),
        Some(X11Grab::Everywhere) | None => (true, true),
    };
    HotkeyCapabilities { global, everywhere, hold: global, toggle_command, edit_toggle_command, solo_keys: solo_key::available(os, session) }
}

/// The graphical session on Linux (`None` on other platforms and without a display).
pub fn linux_session() -> Option<Session> {
    if cfg!(target_os = "linux") { Session::detect() } else { None }
}

/// The command a compositor shortcut should run to toggle dictation in this installation
/// (`voltip-desktop --toggle`, the AppImage path, or a dev build's absolute path).
pub fn toggle_command() -> String {
    remote_command("--toggle")
}

/// The same for the voice-edit key (docs/dictation.md §19): `voltip-desktop --edit-toggle`.
pub fn edit_toggle_command() -> String {
    remote_command("--edit-toggle")
}

fn remote_command(flag: &str) -> String {
    #[cfg(unix)]
    {
        voltip_inject::remote_command(std::env::var("APPIMAGE").ok().as_deref(), std::env::current_exe().ok().as_deref(), voltip_inject::process::which, flag)
    }
    #[cfg(not(unix))]
    {
        voltip_inject::remote_command(None, std::env::current_exe().ok().as_deref(), |_| None, flag)
    }
}

/// Why `label` cannot be registered at all in `session`, as the text of `HotkeyStatus.error`:
/// only a pure Wayland session, where there is no X server to grab keys from (global-hotkey 0.8
/// answers `Ok` there anyway, because its X11 thread has already given up).
pub fn session_block(session: Option<SessionKind>, label: &str, toggle: &str) -> Option<String> {
    match session.map(SessionKind::x11_grab) {
        Some(X11Grab::Unavailable) => {
            Some(format!("{label} 注册失败：纯 Wayland 会话不允许应用注册全局热键。请在系统设置的自定义快捷键里把 {label} 绑定到命令 `{toggle}`"))
        }
        _ => None,
    }
}

/// What the log should say after a successful registration in `session`: under XWayland the grab
/// fires only while an X11 window has focus.
pub fn session_caveat(session: Option<SessionKind>, toggle: &str) -> Option<String> {
    match session.map(SessionKind::x11_grab) {
        Some(X11Grab::X11WindowsOnly) => Some(format!(
            "registered through XWayland: it fires only while an X11 window has focus; bind `{toggle}` in the compositor's shortcuts for native Wayland windows"
        )),
        _ => None,
    }
}

/// The key that cancels a running take.
pub const CANCEL_KEY: &str = "Escape";

/// Esc combinations the system keeps for itself: Ctrl+Shift+Esc (Windows: Task Manager) and
/// Cmd+Option+Esc (macOS: Force Quit). A take never takes those over.
fn reserved_cancel(modifiers: &[Modifier]) -> bool {
    let set = |m: &[Modifier]| modifiers.len() == m.len() && m.iter().all(|x| modifiers.contains(x));
    set(&[Modifier::Ctrl, Modifier::Shift]) || set(&[Modifier::Meta, Modifier::Alt])
}

/// The shortcuts the cancel key is registered as: bare, and under the modifiers of each chord and
/// of a lone modifier key (the ones a `hold` take keeps down). Chords that do not parse, and
/// reserved combinations, add nothing.
pub fn cancel_shortcuts(chords: &Chords) -> Vec<String> {
    let mut out = vec![CANCEL_KEY.to_owned()];
    let held = std::iter::once(&chords.hotkey)
        .chain(chords.edit_hotkey.as_ref())
        .filter_map(|chord| Hotkey::parse(chord).ok().map(|hotkey| hotkey.modifiers))
        .chain(chords.solo_key.and_then(solo_modifier).map(|m| vec![m]));
    for modifiers in held {
        if reserved_cancel(&modifiers) {
            continue;
        }
        let shortcut = Hotkey { modifiers, key: CANCEL_KEY.to_owned() }.to_tauri_shortcut();
        if !out.contains(&shortcut) {
            out.push(shortcut);
        }
    }
    out
}

/// Whether the cancel key belongs registered in `phase`: while the take can still be abandoned.
pub fn cancel_key_wanted(phase: &DictationPhase) -> bool {
    matches!(phase, DictationPhase::Listening { .. } | DictationPhase::Processing { .. })
}

/// What the cancel key's registration has to do next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelKeyStep {
    /// A take began (or the registration was dropped under it): take Esc.
    Register,
    /// The take is over, or the chord recorder needs Esc: give it back.
    Unregister,
}

/// Cancel-key bookkeeping: whether a take wants it and whether it is registered. `unregister_all`
/// (a settings change, the chord recorder) drops the registration behind this state's back, so
/// those paths call [`CancelKey::dropped`] and the next [`CancelKey::step`] registers it again.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CancelKey {
    wanted: bool,
    registered: bool,
}

impl CancelKey {
    /// The registration step for the current wish; `capturing` (the chord recorder owns every key,
    /// Esc included, to cancel itself) keeps it unregistered.
    pub fn step(&self, capturing: bool) -> Option<CancelKeyStep> {
        match (self.wanted && !capturing, self.registered) {
            (true, false) => Some(CancelKeyStep::Register),
            (false, true) => Some(CancelKeyStep::Unregister),
            _ => None,
        }
    }

    /// Record whether the current phase wants the key.
    pub fn want(&mut self, wanted: bool) {
        self.wanted = wanted;
    }

    /// The OS registration is gone (`unregister_all`).
    pub fn dropped(&mut self) {
        self.registered = false;
    }

    /// The step ran. A registration the OS refused counts as done too: it is retried with the next
    /// take, never in a loop.
    pub fn done(&mut self, step: CancelKeyStep) {
        self.registered = step == CancelKeyStep::Register;
    }
}

/// Shared registration bookkeeping (managed Tauri state).
#[derive(Default)]
pub struct HotkeyRegistry {
    status: Mutex<HotkeyStatus>,
    /// The settings page is recording a new chord: the OS registration is suspended so the chord
    /// reaches the webview instead of being swallowed by `RegisterHotKey` (or starting a capture).
    capturing: Mutex<bool>,
    cancel: Mutex<CancelKey>,
    /// The cancel shortcuts the OS accepted for the running take (what `Unregister` releases).
    cancel_registered: Mutex<Vec<String>>,
    /// The lone-key trigger's input hook while one is installed.
    solo: Mutex<Option<SoloHook>>,
}

impl HotkeyRegistry {
    /// Last status published.
    pub fn status(&self) -> HotkeyStatus {
        self.status.lock().clone()
    }

    /// Whether registration is currently suspended for the recorder.
    pub fn capturing(&self) -> bool {
        *self.capturing.lock()
    }

    /// Headless shells (no global-shortcut plugin) only track the flag.
    pub fn set_capturing_flag(&self, active: bool) {
        *self.capturing.lock() = active;
    }
}

/// Enter or leave recorder mode. On `true` every shortcut is unregistered and the status says so;
/// on `false` the saved chords are registered again. Idempotent, so a recorder that cancels twice
/// (Esc + blur) is harmless.
pub fn set_capture<R: Runtime>(app: &AppHandle<R>, bridge: &Bridge, registry: &Arc<HotkeyRegistry>, active: bool) {
    {
        let mut capturing = registry.capturing.lock();
        if *capturing == active {
            return;
        }
        *capturing = active;
    }
    if active {
        if let Err(e) = app.global_shortcut().unregister_all() {
            tracing::warn!(error = %e, "unregister_all failed");
        }
        // The recorder types chords, Right Ctrl + K among them: no lone-key edges meanwhile.
        drop(registry.solo.lock().take());
        registry.cancel.lock().dropped();
        registry.cancel_registered.lock().clear();
        let status = HotkeyStatus { backend: backend_name(), capabilities: capabilities(), capturing: true, ..HotkeyStatus::default() };
        *registry.status.lock() = status.clone();
        bridge.publish(UiEvent::Hotkey(status));
        tracing::info!("global hotkeys suspended for recording");
    } else {
        let current = Chords::from(&bridge.state().settings);
        apply(app, bridge, registry, &current);
    }
}

/// (Re)register the dictation chord and the voice-edit chord, replacing whatever was registered
/// before, and publish the outcome. A chord another application owns surfaces as `error` /
/// `edit_error`, never as a silent no-op; an edit chord equal to the dictation chord is not
/// registered (the core refuses to save one, a hand-edited `settings.json` may still hold it).
pub fn apply<R: Runtime>(app: &AppHandle<R>, bridge: &Bridge, registry: &Arc<HotkeyRegistry>, chords: &Chords) {
    if registry.capturing() {
        // The recorder owns the keyboard right now; `set_capture(false)` re-applies the saved chords.
        return;
    }
    if let Err(e) = app.global_shortcut().unregister_all() {
        tracing::warn!(error = %e, "unregister_all failed");
    }
    registry.cancel.lock().dropped();
    registry.cancel_registered.lock().clear();
    let mut status = HotkeyStatus { backend: backend_name(), capabilities: capabilities(), ..HotkeyStatus::default() };
    let session = linux_session().map(|s| s.kind);
    match register(app, bridge, registry, &chords.hotkey, TakeKind::Dictation, session) {
        Ok(label) => status.registered = Some(label),
        Err(e) => status.error = Some(e),
    }
    if let Some(edit) = &chords.edit_hotkey {
        if Hotkey::same_chord(edit, &chords.hotkey) {
            status.edit_error = Some(format!("{edit} 与听写热键相同，未注册"));
        } else {
            match register(app, bridge, registry, edit, TakeKind::Edit, session) {
                Ok(label) => status.edit_registered = Some(label),
                Err(e) => status.edit_error = Some(e),
            }
        }
    }
    sync_solo(bridge, registry, chords.solo_key, session, &mut status);
    *registry.status.lock() = status.clone();
    bridge.publish(UiEvent::Hotkey(status));
    // A take running across a settings change keeps its cancel key.
    sync_cancel_key(app, bridge, registry);
}

/// Install, keep or remove the lone-key hook for `key` and say so in `status`. A running hook for
/// the same key stays; a different key replaces it (the old hook goes first: the Windows hook
/// procedures share one state); a key that failed is tried again.
fn sync_solo(bridge: &Bridge, registry: &Arc<HotkeyRegistry>, key: Option<SoloKey>, session: Option<SessionKind>, status: &mut HotkeyStatus) {
    let mut slot = registry.solo.lock();
    if slot.as_ref().map(SoloHook::key) != key {
        drop(slot.take());
        if let Some(key) = key {
            match solo_key::watch(key, session, false, solo_sink(bridge.clone(), Arc::downgrade(registry))) {
                Ok(hook) => {
                    tracing::info!(key = %key, "lone-key trigger watched");
                    *slot = Some(hook);
                }
                Err(e) => {
                    tracing::warn!(key = %key, error = %e, "lone-key trigger not watched");
                    status.solo_error = Some(e);
                }
            }
        }
    }
    status.solo_registered = slot.as_ref().map(SoloHook::key);
}

/// Where the hook's edges go: the settings page's `solo_pressed`, then the core as dictation
/// `HotkeyEdge`s (a chord as `chorded`). Holds the registry weakly: the hook lives inside it.
fn solo_sink(bridge: Bridge, registry: Weak<HotkeyRegistry>) -> impl Fn(SoloEdge) + Send + 'static {
    move |edge| {
        let Some(registry) = registry.upgrade() else { return };
        if registry.capturing() {
            return;
        }
        {
            let mut st = registry.status.lock();
            st.solo_pressed = edge == SoloEdge::Press;
            let snapshot = st.clone();
            drop(st);
            bridge.publish(UiEvent::Hotkey(snapshot));
        }
        let command = match edge {
            SoloEdge::Press => edge_command(true, EdgeSource::Hotkey, TakeKind::Dictation),
            SoloEdge::Release => edge_command(false, EdgeSource::Hotkey, TakeKind::Dictation),
            SoloEdge::Chorded => chorded_command(),
        };
        if let Err(e) = bridge.dispatch(command) {
            tracing::warn!(error = %e, ?edge, "lone-key edge not accepted");
        }
    }
}

/// The core command for "another key joined the held lone-key trigger" (docs/dictation.md §13.1).
pub fn chorded_command() -> UiCommand {
    UiCommand::HotkeyEdge { pressed: false, at_ms: now_ms(), source: EdgeSource::Hotkey, purpose: TakeKind::Dictation, chorded: true }
}

/// Arm the cancel key for a take in `phase`, or release it once the take can no longer be cancelled.
pub fn follow_phase<R: Runtime>(app: &AppHandle<R>, bridge: &Bridge, registry: &Arc<HotkeyRegistry>, phase: &DictationPhase) {
    registry.cancel.lock().want(cancel_key_wanted(phase));
    sync_cancel_key(app, bridge, registry);
}

fn sync_cancel_key<R: Runtime>(app: &AppHandle<R>, bridge: &Bridge, registry: &Arc<HotkeyRegistry>) {
    let Some(step) = registry.cancel.lock().step(registry.capturing()) else { return };
    match step {
        CancelKeyStep::Register => {
            // Pure Wayland grabs nothing; the compositor shortcut would have to run `--cancel`.
            if linux_session().is_some_and(|s| s.kind == SessionKind::Wayland) {
                return;
            }
            let mut accepted = Vec::new();
            for shortcut in cancel_shortcuts(&Chords::from(&bridge.state().settings)) {
                let bridge = bridge.clone();
                let result = app.global_shortcut().on_shortcut(shortcut.as_str(), move |_app, _shortcut, event| {
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    if let Err(e) = bridge.dispatch(UiCommand::DictationCancel) {
                        tracing::warn!(error = %e, "cancel key not accepted");
                    }
                });
                match result {
                    Ok(()) => accepted.push(shortcut),
                    // Another application (or the system) owns this combination; the others still work.
                    Err(e) => tracing::warn!(shortcut = %shortcut, error = %e, "cancel key registration failed"),
                }
            }
            tracing::info!(shortcuts = ?accepted, "cancel key registered for the take");
            *registry.cancel_registered.lock() = accepted;
        }
        CancelKeyStep::Unregister => {
            let registered = std::mem::take(&mut *registry.cancel_registered.lock());
            for shortcut in &registered {
                if let Err(e) = app.global_shortcut().unregister(shortcut.as_str()) {
                    tracing::warn!(shortcut = %shortcut, error = %e, "cancel key unregister failed");
                }
            }
            tracing::info!(shortcuts = ?registered, "cancel key released");
        }
    }
    // A failed registration is not retried until the next phase change, never in a loop.
    registry.cancel.lock().done(step);
}

/// Register one chord whose edges go to the core as `purpose`; the display label, or why not.
fn register<R: Runtime>(
    app: &AppHandle<R>,
    bridge: &Bridge,
    registry: &Arc<HotkeyRegistry>,
    text: &str,
    purpose: TakeKind,
    session: Option<SessionKind>,
) -> Result<String, String> {
    let hotkey = Hotkey::parse(text).map_err(|e| format!("{text}: {e}"))?;
    let label = hotkey.display();
    let remote = match purpose {
        TakeKind::Dictation => toggle_command(),
        TakeKind::Edit => edit_toggle_command(),
    };
    // Pure Wayland: nothing to grab from, and the plugin would answer `Ok` anyway (§14).
    if let Some(blocked) = session_block(session, &label, &remote) {
        // `error` is the HotkeyStatus text the settings page shows; the log carries it too (the
        // Wayland smoke reads it from there).
        tracing::warn!(hotkey = %label, purpose = purpose.as_str(), backend = %backend_name(), error = %blocked, "no global hotkey on this session; the compositor must run {remote}");
        return Err(blocked);
    }
    let bridge_for_handler = bridge.clone();
    let registry_for_handler = registry.clone();
    let shortcut = hotkey.to_tauri_shortcut();
    let result = app.global_shortcut().on_shortcut(shortcut.as_str(), move |_app, _shortcut, event| {
        let pressed = event.state() == ShortcutState::Pressed;
        // `pressed` on the settings page is the dictation chord's.
        if purpose == TakeKind::Dictation {
            let mut st = registry_for_handler.status.lock();
            st.pressed = pressed;
            let snapshot = st.clone();
            drop(st);
            bridge_for_handler.publish(UiEvent::Hotkey(snapshot));
        }
        if let Err(e) = bridge_for_handler.dispatch(edge_command(pressed, EdgeSource::Hotkey, purpose)) {
            tracing::warn!(error = %e, pressed, purpose = purpose.as_str(), "hotkey edge not accepted");
        }
    });
    match result {
        Ok(()) => {
            tracing::info!(hotkey = %label, shortcut = %shortcut, purpose = purpose.as_str(), "global hotkey registered");
            if let Some(caveat) = session_caveat(session, &remote) {
                tracing::warn!(hotkey = %label, "{caveat}");
            }
            Ok(label)
        }
        Err(e) => {
            tracing::warn!(hotkey = %label, error = %e, purpose = purpose.as_str(), "global hotkey registration failed");
            Err(format!("{label} 注册失败：{e}"))
        }
    }
}

/// Watch the bridge for settings changes and keep the OS registration in sync.
pub fn follow_settings<R: Runtime>(app: AppHandle<R>, bridge: Bridge, registry: Arc<HotkeyRegistry>) {
    if let Some(session) = linux_session() {
        tracing::info!(
            session = %session,
            x11_grab = ?session.kind.x11_grab(),
            backend = %backend_name(),
            toggle = %toggle_command(),
            edit_toggle = %edit_toggle_command(),
            selection_timing = ?selection_timing_for(Some(session.kind)),
            "linux session"
        );
    }
    let mut events = bridge.events();
    let mut current = Chords::from(&bridge.state().settings);
    apply(&app, &bridge, &registry, &current);
    tauri::async_runtime::spawn(async move {
        loop {
            let next = match events.recv().await {
                Ok(UiEvent::Settings(s)) => Some(Chords::from(&s)),
                Ok(UiEvent::State(state)) => {
                    follow_phase(&app, &bridge, &registry, &state.dictation.phase);
                    Some(Chords::from(&state.settings))
                }
                Ok(UiEvent::Dictation(status)) => {
                    follow_phase(&app, &bridge, &registry, &status.phase);
                    None
                }
                Ok(_) => None,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // Catch up from the cached state rather than replaying stale phases.
                    follow_phase(&app, &bridge, &registry, &bridge.state().dictation.phase);
                    None
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            if let Some(chords) = next
                && chords != current
            {
                current = chords;
                apply(&app, &bridge, &registry, &current);
            }
        }
    });
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// docs/dictation.md §5: Esc cancels at any moment of a take, and only then is it taken from
    /// the other applications.
    #[test]
    fn the_cancel_key_is_held_only_while_a_take_can_be_cancelled() {
        use voltip_core::ProcessingStage;
        let listening = DictationPhase::Listening { started_at: 0, ready: true, locked: false, live: None };
        let processing = DictationPhase::Processing { stage: ProcessingStage::Transcribing, started_at: 0, preview: None };
        assert!(cancel_key_wanted(&listening));
        assert!(cancel_key_wanted(&processing));
        assert!(!cancel_key_wanted(&DictationPhase::Idle));
        assert!(!cancel_key_wanted(&DictationPhase::Cancelled { injected_chars: 0 }));

        // Bare, and under each chord's modifiers: a `hold` take is cancelled with the chord down.
        let chords = Chords { hotkey: "Ctrl+Alt+Space".into(), edit_hotkey: Some("Ctrl+Alt+E".into()), solo_key: None };
        assert_eq!(cancel_shortcuts(&chords), ["Escape", "Control+Alt+Escape"]);
        let chords = Chords { hotkey: "Alt+Shift+Z".into(), edit_hotkey: Some("Meta+E".into()), solo_key: None };
        assert_eq!(cancel_shortcuts(&chords), ["Escape", "Alt+Shift+Escape", "Super+Escape"]);
        let chords = Chords { hotkey: "not a chord".into(), edit_hotkey: None, solo_key: None };
        assert_eq!(cancel_shortcuts(&chords), ["Escape"]);
        // Task Manager and Force Quit stay the system's.
        let chords = Chords { hotkey: "Shift+Ctrl+D".into(), edit_hotkey: Some("Alt+Meta+E".into()), solo_key: None };
        assert_eq!(cancel_shortcuts(&chords), ["Escape"]);
        // docs/dictation.md §13.1: a lone modifier held in `hold` adds its own; a mouse button none.
        let chords = Chords { hotkey: "Ctrl+Alt+Space".into(), edit_hotkey: None, solo_key: Some(SoloKey::RightShift) };
        assert_eq!(cancel_shortcuts(&chords), ["Escape", "Control+Alt+Escape", "Shift+Escape"]);
        let chords = Chords { hotkey: "Ctrl+Alt+Space".into(), edit_hotkey: None, solo_key: Some(SoloKey::MouseBack) };
        assert_eq!(cancel_shortcuts(&chords), ["Escape", "Control+Alt+Escape"]);
        assert_eq!(SoloKey::ALL.into_iter().filter_map(solo_modifier).count(), 4, "the four right-hand modifiers");

        let mut key = CancelKey::default();
        assert_eq!(key.step(false), None, "idle: nothing to do");
        key.want(true);
        assert_eq!(key.step(true), None, "the chord recorder owns Esc");
        assert_eq!(key.step(false), Some(CancelKeyStep::Register));
        key.done(CancelKeyStep::Register);
        assert_eq!(key.step(false), None, "registered once per take");
        // A settings change mid-take re-registers every chord: the cancel key comes back.
        key.dropped();
        assert_eq!(key.step(false), Some(CancelKeyStep::Register));
        key.done(CancelKeyStep::Register);
        // The recorder opens mid-take: Esc is released to it.
        assert_eq!(key.step(true), Some(CancelKeyStep::Unregister));
        key.done(CancelKeyStep::Unregister);
        assert_eq!(key.step(true), None);
        key.want(false);
        assert_eq!(key.step(false), None, "the take ended while the recorder had Esc");
        key.want(true);
        key.done(CancelKeyStep::Register);
        key.want(false);
        assert_eq!(key.step(false), Some(CancelKeyStep::Unregister));
        key.done(CancelKeyStep::Unregister);
        assert_eq!(key, CancelKey::default());
    }

    /// Regression (goal review 2026-09-27): the hotkey pane's capability table was a fixture listing
    /// backends the app does not use; it now shows what this session's backend can do.
    #[test]
    fn capabilities_follow_the_session() {
        let cmd = |flag: &str| format!("voltip-desktop {flag}");
        let caps = |os: &str, session| capabilities_for(os, session, cmd("--toggle"), cmd("--edit-toggle"));
        for (os, session) in [("windows", None), ("macos", None), ("linux", Some(SessionKind::X11)), ("linux", None), ("windows", Some(SessionKind::Wayland))] {
            let c = caps(os, session);
            assert!(c.global && c.everywhere && c.hold, "{os} {session:?}: {c:?}");
        }
        let xwayland = caps("linux", Some(SessionKind::XWayland));
        assert!(xwayland.global && xwayland.hold && !xwayland.everywhere, "{xwayland:?}");
        let wayland = caps("linux", Some(SessionKind::Wayland));
        assert!(!wayland.global && !wayland.everywhere && !wayland.hold, "{wayland:?}");
        // docs/dictation.md §13.1: no input hook on a pure Wayland session, Fn only on macOS.
        assert!(wayland.solo_keys.is_empty());
        assert!(caps("macos", None).solo_keys.contains(&SoloKey::Fn));
        assert!(!caps("windows", None).solo_keys.contains(&SoloKey::Fn) && caps("windows", None).solo_keys.contains(&SoloKey::MouseBack));
        assert_eq!(caps("linux", Some(SessionKind::XWayland)).solo_keys, caps("windows", None).solo_keys);
        assert_eq!((wayland.toggle_command.as_str(), wayland.edit_toggle_command.as_str()), ("voltip-desktop --toggle", "voltip-desktop --edit-toggle"));
        assert!(capabilities().toggle_command.ends_with("--toggle"));
    }

    #[test]
    fn backend_name_table() {
        assert_eq!(backend_name_for("windows", None), "global-shortcut · Windows · RegisterHotKey");
        assert_eq!(
            backend_name_for("windows", Some(SessionKind::Wayland)),
            "global-shortcut · Windows · RegisterHotKey",
            "the session means nothing off Linux"
        );
        assert_eq!(backend_name_for("macos", None), "global-shortcut · macOS · Carbon");
        assert_eq!(backend_name_for("linux", None), "global-shortcut · Linux · X11");
        assert_eq!(backend_name_for("linux", Some(SessionKind::X11)), "global-shortcut · Linux · X11");
        assert_eq!(backend_name_for("linux", Some(SessionKind::XWayland)), "global-shortcut · Linux · XWayland");
        assert_eq!(backend_name_for("linux", Some(SessionKind::Wayland)), "global-shortcut · Linux · Wayland");
        assert_eq!(backend_name(), backend_name_for(std::env::consts::OS, linux_session().map(|s| s.kind)));
    }

    #[test]
    fn wayland_blocks_registration_and_names_the_toggle_command() {
        let toggle = "voltip-desktop --toggle";
        let reason = session_block(Some(SessionKind::Wayland), "Ctrl+Alt+Space", toggle).unwrap();
        assert_eq!(
            reason,
            "Ctrl+Alt+Space 注册失败：纯 Wayland 会话不允许应用注册全局热键。请在系统设置的自定义快捷键里把 Ctrl+Alt+Space 绑定到命令 `voltip-desktop --toggle`"
        );
        assert_eq!(session_block(Some(SessionKind::XWayland), "Ctrl+Alt+Space", toggle), None);
        assert_eq!(session_block(Some(SessionKind::X11), "Ctrl+Alt+Space", toggle), None);
        assert_eq!(session_block(None, "Ctrl+Alt+Space", toggle), None, "headless / other platforms: let the plugin answer");

        let caveat = session_caveat(Some(SessionKind::XWayland), toggle).unwrap();
        assert!(caveat.contains("XWayland") && caveat.contains("`voltip-desktop --toggle`"), "{caveat}");
        assert_eq!(session_caveat(Some(SessionKind::X11), toggle), None);
        assert_eq!(session_caveat(Some(SessionKind::Wayland), toggle), None);
        assert_eq!(session_caveat(None, toggle), None);
        assert!(toggle_command().ends_with(" --toggle"));
        assert!(edit_toggle_command().ends_with(" --edit-toggle"));
        let edit = session_block(Some(SessionKind::Wayland), "Ctrl+Alt+E", "voltip-desktop --edit-toggle").unwrap();
        assert!(edit.contains("Ctrl+Alt+E") && edit.contains("`voltip-desktop --edit-toggle`"), "{edit}");
        if !cfg!(target_os = "linux") {
            assert_eq!(linux_session(), None);
        }
    }

    /// docs/dictation.md §19: X11 grabs the keyboard while the hotkey is down — the copy waits for
    /// the key-up there (XWayland too); elsewhere it goes out at the press.
    #[test]
    fn selection_timing_follows_the_x11_grab() {
        assert_eq!(selection_timing_for(Some(SessionKind::X11)), SelectionTiming::AfterKeyUp);
        assert_eq!(selection_timing_for(Some(SessionKind::XWayland)), SelectionTiming::AfterKeyUp);
        assert_eq!(selection_timing_for(Some(SessionKind::Wayland)), SelectionTiming::AtPress, "the compositor shortcut runs --edit-toggle");
        assert_eq!(selection_timing_for(None), SelectionTiming::AtPress, "Windows / macOS");
        let settings = Settings { hotkey: "Ctrl+Alt+Space".into(), edit_hotkey: None, solo_key: Some(SoloKey::RightCtrl), ..Settings::default() };
        assert_eq!(Chords::from(&settings), Chords { hotkey: "Ctrl+Alt+Space".into(), edit_hotkey: None, solo_key: Some(SoloKey::RightCtrl) });
        assert!(matches!(
            chorded_command(),
            UiCommand::HotkeyEdge { pressed: false, chorded: true, purpose: TakeKind::Dictation, source: EdgeSource::Hotkey, .. }
        ));
        for (pressed, purpose) in [(true, TakeKind::Edit), (false, TakeKind::Dictation)] {
            assert!(
                matches!(edge_command(pressed, EdgeSource::Hotkey, purpose), UiCommand::HotkeyEdge { pressed: p, purpose: q, .. } if p == pressed && q == purpose)
            );
        }
    }
}
