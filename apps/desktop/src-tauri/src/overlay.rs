//! The floating pill window: a transparent, always-on-top window at the bottom centre of
//! the monitor that shows one pill state. It follows the core's dictation phase
//! ([`follow_dictation`]): `listening` while the microphone is open, `processing`, then
//! `inserted` / `error` / `cancelled` for the dwell, and hidden again on `Idle`.
//!
//! Lifecycle: the window is **prewarmed** hidden at startup and then
//! only shown / hidden, never created per use, so the first press does not pay for a webview and
//! does not flash a default state. State changes travel as a window-scoped event
//! ([`OVERLAY_EVENT`]) instead of a navigation, so the document is never reloaded; hiding first
//! paints [`BLANK_STATE`] (nothing) and hides a frame later, so the next show never starts from a
//! stale pill.

use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter as _, Manager as _, Runtime, WebviewUrl, WebviewWindowBuilder};
use voltip_core::ui::UiEvent;
use voltip_core::{DictationPhase, OverlayPlacement};
use voltip_tauri_bridge::Bridge;

/// Window label.
pub const OVERLAY_LABEL: &str = "overlay";
/// Pill window size: the widest state is 420 px, plus the shadow margin.
pub const OVERLAY_WIDTH: f64 = 480.0;
/// Pill window height (40 px pill plus shadow margin; the extra slack survives Windows text scaling).
pub const OVERLAY_HEIGHT: f64 = 64.0;
/// Distance from the work area's edge, bottom or top: 24 px.
pub const OVERLAY_EDGE_MARGIN: f64 = 24.0;
/// Event carrying the pill state to the overlay webview (`{ "state": "listening" }`).
pub const OVERLAY_EVENT: &str = "voltip://overlay";
/// The state that paints nothing: what the prewarmed window shows, and what is painted before hiding.
pub const BLANK_STATE: &str = "blank";
/// The route state in which the pill follows `UiState.dictation` on its own (the webview maps the
/// phase to a pill state and reads the live meter); the shell then only shows / hides the window.
pub const LIVE_STATE: &str = "live";
/// Delay between painting `blank` and hiding the window: long enough for one webview frame.
pub const HIDE_AFTER_BLANK: Duration = Duration::from_millis(80);
/// The pill's flash after 复制上一条结果 (docs/dictation.md §28): `已复制上一条结果`, between takes.
pub const COPIED_STATE: &str = "copied";
/// Debug-build knob: `=1` draws the pill window opaque. X11 without a compositor (Xvfb smoke
/// tests) renders an RGBA window as a black rectangle, so the headless run cannot prove the pill
/// paints unless transparency is off. Release builds ignore it.
pub const DEV_OPAQUE_OVERLAY_ENV: &str = "VOLTIP_DEV_OPAQUE_OVERLAY";

/// `true` only when a debug build was explicitly asked for an opaque pill window.
pub fn dev_opaque_overlay_requested(value: Option<&str>, debug_build: bool) -> bool {
    debug_build && value == Some("1")
}

/// Payload of [`OVERLAY_EVENT`].
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct OverlayState<'a> {
    /// Pill state name (`listening`, `processing`, …) or [`BLANK_STATE`].
    pub state: &'a str,
}

/// The state the pill window should currently show (managed Tauri state). The webview pulls it
/// through `overlay_state` once its listener is attached, because a window that was prewarmed
/// hidden may only finish loading after the first `show()`: an event emitted before the page ran
/// would otherwise be lost and the pill would stay blank.
#[derive(Debug)]
pub struct OverlaySlot {
    state: Mutex<String>,
}

impl Default for OverlaySlot {
    fn default() -> Self {
        Self { state: Mutex::new(LIVE_STATE.to_owned()) }
    }
}

impl OverlaySlot {
    /// Current desired state.
    pub fn current(&self) -> String {
        self.state.lock().clone()
    }

    fn set(&self, state: &str) {
        *self.state.lock() = state.to_owned();
    }
}

fn remember<R: Runtime>(app: &AppHandle<R>, state: &str) {
    if let Some(slot) = app.try_state::<OverlaySlot>() {
        slot.set(state);
    }
}

/// Route the pill window loads for a given state.
pub fn route_for(state: &str) -> String {
    format!("index.html#/overlay?state={state}")
}

/// Create the hidden pill window at startup so the first hotkey press only has to show it.
pub fn prewarm<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    if app.get_webview_window(OVERLAY_LABEL).is_some() {
        return Ok(());
    }
    // Placed for the default; every show re-anchors for the saved placement.
    let (x, y) = anchor(app, OverlayPlacement::Bottom).unwrap_or((0.0, 0.0));
    let opaque = dev_opaque_overlay_requested(std::env::var(DEV_OPAQUE_OVERLAY_ENV).ok().as_deref(), cfg!(debug_assertions));
    if opaque {
        tracing::warn!("{DEV_OPAQUE_OVERLAY_ENV}=1: pill window drawn opaque (debug smoke test)");
    }
    WebviewWindowBuilder::new(app, OVERLAY_LABEL, WebviewUrl::App(route_for(LIVE_STATE).into()))
        .title("Voltip Overlay")
        .inner_size(OVERLAY_WIDTH, OVERLAY_HEIGHT)
        .min_inner_size(OVERLAY_WIDTH, OVERLAY_HEIGHT)
        .max_inner_size(OVERLAY_WIDTH, OVERLAY_HEIGHT)
        .position(x, y)
        .decorations(false)
        .transparent(!opaque)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .focused(false)
        .focusable(false)
        .visible(false)
        .visible_on_all_workspaces(true)
        .build()?;
    tracing::info!("overlay window prewarmed");
    Ok(())
}

/// Paint nothing, then hide the window one frame later (kept alive for the next press).
pub fn hide<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window(OVERLAY_LABEL) else { return };
    // A live window paints nothing on `Idle` by itself; a fixed-state window is told to go blank.
    let live = app.try_state::<OverlaySlot>().is_some_and(|s| s.current() == LIVE_STATE);
    if !live {
        remember(app, BLANK_STATE);
    }
    if !live && let Err(e) = window.emit_to(OVERLAY_LABEL, OVERLAY_EVENT, OverlayState { state: BLANK_STATE }) {
        tracing::warn!(error = %e, "overlay blank failed");
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(HIDE_AFTER_BLANK).await;
        if let Err(e) = window.hide() {
            tracing::warn!(error = %e, "overlay hide failed");
        }
    });
}

/// Pill state for a dictation phase; `None` hides the pill (docs/dictation.md, shell paragraph).
pub fn pill_for(phase: &DictationPhase) -> Option<&'static str> {
    match phase {
        DictationPhase::Idle => None,
        DictationPhase::Listening { .. } => Some("listening"),
        DictationPhase::Processing { .. } => Some("processing"),
        DictationPhase::Done { .. } => Some("inserted"),
        DictationPhase::Failed { .. } => Some("error"),
        DictationPhase::Cancelled { .. } => Some("cancelled"),
    }
}

/// Apply one dictation phase to the pill window: the webview (route `state=live`) renders the phase
/// itself from `UiState.dictation`, so the shell only decides visibility and place
/// (`Settings.overlay`; `off` never shows it). A `Failed` that carries text stays until the user
/// dismisses or the core returns to `Idle`.
pub fn apply_phase<R: Runtime>(app: &AppHandle<R>, phase: &DictationPhase, placement: OverlayPlacement) {
    match pill_for(phase) {
        Some(_) if placement != OverlayPlacement::Off => {
            if let Err(e) = show_live(app, placement) {
                tracing::warn!(error = %e, "overlay show failed");
            }
        }
        _ => hide(app),
    }
}

/// Show the prewarmed live window (re-anchored for `placement`) without changing its route; a
/// window that last showed a fixed state (the [`COPIED_STATE`] flash) is told to follow the take
/// again.
pub fn show_live<R: Runtime>(app: &AppHandle<R>, placement: OverlayPlacement) -> tauri::Result<()> {
    prewarm(app)?;
    let Some(window) = app.get_webview_window(OVERLAY_LABEL) else { return Ok(()) };
    let Some((x, y)) = anchor(app, placement) else { return Ok(()) };
    window.set_position(tauri::LogicalPosition::new(x, y))?;
    let was_live = app.try_state::<OverlaySlot>().is_none_or(|s| s.current() == LIVE_STATE);
    remember(app, LIVE_STATE);
    if !was_live {
        window.emit_to(OVERLAY_LABEL, OVERLAY_EVENT, OverlayState { state: LIVE_STATE })?;
    }
    if !window.is_visible().unwrap_or(false) {
        window.show()?;
        window.set_always_on_top(true)?;
    }
    Ok(())
}

/// Whether 复制上一条结果 may flash the pill now (docs/dictation.md §28): only between takes — a
/// take under way or a finished one still on screen keeps the pill — and never with the pill off.
pub fn copied_flash_wanted(phase: &DictationPhase, placement: OverlayPlacement) -> bool {
    placement != OverlayPlacement::Off && *phase == DictationPhase::Idle
}

/// Show `已复制上一条结果` for `voltip_core::dictation::DWELL`, then hide the pill, unless a take
/// has taken it over by then.
pub fn flash_copied<R: Runtime>(app: &AppHandle<R>, phase: &DictationPhase, placement: OverlayPlacement) {
    if !copied_flash_wanted(phase, placement) {
        return;
    }
    if let Err(e) = show_fixed(app, COPIED_STATE, placement) {
        tracing::warn!(error = %e, "overlay flash failed");
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(voltip_core::dictation::DWELL).await;
        if app.try_state::<OverlaySlot>().is_some_and(|slot| slot.current() == COPIED_STATE) {
            hide(&app);
        }
    });
}

/// Show the window with the fixed `state`, re-anchored for `placement`.
fn show_fixed<R: Runtime>(app: &AppHandle<R>, state: &'static str, placement: OverlayPlacement) -> tauri::Result<()> {
    prewarm(app)?;
    let Some(window) = app.get_webview_window(OVERLAY_LABEL) else { return Ok(()) };
    let Some((x, y)) = anchor(app, placement) else { return Ok(()) };
    window.set_position(tauri::LogicalPosition::new(x, y))?;
    remember(app, state);
    window.emit_to(OVERLAY_LABEL, OVERLAY_EVENT, OverlayState { state })?;
    if !window.is_visible().unwrap_or(false) {
        window.show()?;
        window.set_always_on_top(true)?;
    }
    Ok(())
}

/// Keep the pill in step with `UiEvent::Dictation` for as long as the bridge lives.
pub fn follow_dictation<R: Runtime>(app: AppHandle<R>, bridge: Bridge) {
    let mut events = bridge.events();
    tauri::async_runtime::spawn(async move {
        loop {
            match events.recv().await {
                Ok(UiEvent::Dictation(status)) => apply_phase(&app, &status.phase, bridge.state().settings.overlay),
                // Switched off while a take shows the pill: hide it now, not at the next phase.
                Ok(UiEvent::Settings(settings)) if settings.overlay == OverlayPlacement::Off => hide(&app),
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // Catch up from the cached state rather than replaying stale phases.
                    let state = bridge.state();
                    apply_phase(&app, &state.dictation.phase, state.settings.overlay);
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

/// A work area in logical pixels: left, top, width, height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorkArea {
    /// Left edge.
    pub left: f64,
    /// Top edge.
    pub top: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

/// Where the pill window goes in `area` for `placement`: centred, [`OVERLAY_EDGE_MARGIN`] from the
/// bottom or the top edge; `None` for `off`.
pub fn pill_origin(area: WorkArea, placement: OverlayPlacement) -> Option<(f64, f64)> {
    let x = area.left + (area.width - OVERLAY_WIDTH) / 2.0;
    match placement {
        OverlayPlacement::Bottom => Some((x, area.top + area.height - OVERLAY_HEIGHT - OVERLAY_EDGE_MARGIN)),
        OverlayPlacement::Top => Some((x, area.top + OVERLAY_EDGE_MARGIN)),
        OverlayPlacement::Off => None,
    }
}

/// The pill's position on the primary monitor's work area for `placement`, in logical pixels.
fn anchor<R: Runtime>(app: &AppHandle<R>, placement: OverlayPlacement) -> Option<(f64, f64)> {
    let Ok(Some(monitor)) = app.primary_monitor() else {
        // No monitor reported (headless X server without RandR): the window's own origin.
        return (placement != OverlayPlacement::Off).then_some((0.0, 0.0));
    };
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    pill_origin(
        WorkArea {
            left: f64::from(area.position.x) / scale,
            top: f64::from(area.position.y) / scale,
            width: f64::from(area.size.width) / scale,
            height: f64::from(area.size.height) / scale,
        },
        placement,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// docs/dictation.md §28: 复制上一条结果 flashes the pill only between takes, never over a take
    /// under way or a card still on screen, and never with the pill off.
    #[test]
    fn the_copied_flash_shows_only_between_takes() {
        assert!(copied_flash_wanted(&DictationPhase::Idle, OverlayPlacement::Bottom));
        assert!(copied_flash_wanted(&DictationPhase::Idle, OverlayPlacement::Top));
        assert!(!copied_flash_wanted(&DictationPhase::Idle, OverlayPlacement::Off));
        assert!(!copied_flash_wanted(&DictationPhase::Listening { started_at: 1, ready: true, live: None, locked: false }, OverlayPlacement::Bottom));
        assert!(!copied_flash_wanted(&DictationPhase::CANCELLED, OverlayPlacement::Bottom), "a finished take is still on screen");
    }

    /// Settings › 外观 › 悬浮胶囊 (2026-09-27): the saved placement decides where the shell puts
    /// the pill window, and `off` shows none (it used to live in the webview's localStorage only).
    #[test]
    fn the_pill_goes_where_the_placement_says() {
        let area = WorkArea { left: 100.0, top: 30.0, width: 1280.0, height: 770.0 };
        let x = 100.0 + (1280.0 - OVERLAY_WIDTH) / 2.0;
        assert_eq!(pill_origin(area, OverlayPlacement::Bottom), Some((x, 30.0 + 770.0 - OVERLAY_HEIGHT - OVERLAY_EDGE_MARGIN)));
        assert_eq!(pill_origin(area, OverlayPlacement::Top), Some((x, 30.0 + OVERLAY_EDGE_MARGIN)));
        assert_eq!(pill_origin(area, OverlayPlacement::Off), None);
        assert_eq!(OverlayPlacement::default(), OverlayPlacement::Bottom);
    }
}
