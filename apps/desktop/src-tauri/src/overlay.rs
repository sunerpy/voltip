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
//!
//! Stacking (docs/dictation.md §32): showing the window does not move it up the stacking order,
//! so every show and every phase of a take puts it back on top ([`keep_on_top`], through
//! `voltip-overlay`), and on Windows a keeper does it again whenever another window comes to the
//! front while the pill shows. None of it takes the focus: the result card's 粘贴 still goes to the
//! window in front.

use std::sync::atomic::{AtomicU64, Ordering};
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
    /// How many times the pill was shown: a hide waits a frame, and a show in that frame (the next
    /// take's first phase) cancels it.
    shows: AtomicU64,
}

impl Default for OverlaySlot {
    fn default() -> Self {
        Self { state: Mutex::new(LIVE_STATE.to_owned()), shows: AtomicU64::new(0) }
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

    /// A show of the pill (every phase of a take, and the copied flash).
    pub fn shown(&self) {
        self.shows.fetch_add(1, Ordering::SeqCst);
    }

    /// Taken when a hide starts: [`OverlaySlot::may_hide`] answers whether no show came since.
    pub fn hide_ticket(&self) -> u64 {
        self.shows.load(Ordering::SeqCst)
    }

    /// Whether the hide that took `ticket` may still hide the window: not when the pill was shown
    /// since (it would hide the pill of a take that has just started).
    pub fn may_hide(&self, ticket: u64) -> bool {
        self.shows.load(Ordering::SeqCst) == ticket
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
    let ticket = app.try_state::<OverlaySlot>().map(|slot| slot.hide_ticket());
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(HIDE_AFTER_BLANK).await;
        // A take that showed the pill in the meantime keeps it: before 2026-11-01 this hid the pill
        // of a take started within the frame, until its next phase showed it again.
        if let Some(ticket) = ticket
            && app.try_state::<OverlaySlot>().is_some_and(|slot| !slot.may_hide(ticket))
        {
            return;
        }
        if let Err(e) = window.hide() {
            tracing::warn!(error = %e, "overlay hide failed");
        }
        let_go(&app);
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
    if let Some(slot) = app.try_state::<OverlaySlot>() {
        slot.shown();
    }
    if !was_live {
        window.emit_to(OVERLAY_LABEL, OVERLAY_EVENT, OverlayState { state: LIVE_STATE })?;
    }
    if !window.is_visible().unwrap_or(false) {
        window.show()?;
    }
    keep_on_top(app, &window);
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
    if let Some(slot) = app.try_state::<OverlaySlot>() {
        slot.shown();
    }
    window.emit_to(OVERLAY_LABEL, OVERLAY_EVENT, OverlayState { state })?;
    if !window.is_visible().unwrap_or(false) {
        window.show()?;
    }
    keep_on_top(app, &window);
    Ok(())
}

/// What keeps the pill above the other windows (docs/dictation.md §32; managed state): on Windows
/// the keeper of the pill on screen, on Linux the X11 connection the restacking goes through.
#[derive(Debug, Default)]
pub struct PillStacking {
    #[cfg(target_os = "windows")]
    keeper: Mutex<Option<voltip_overlay::windows::Keeper>>,
    #[cfg(target_os = "linux")]
    x11: voltip_overlay::x11::Stacker,
}

/// Put the pill on screen back above the other windows, on every show and every phase of a take,
/// without taking the focus. Windows: the top of the topmost windows, kept there while it shows.
/// macOS: the status window level, every Space and full-screen Spaces, ordered to the front
/// without activating the app. Linux: on top of the X11 stacking order (pure Wayland: the
/// compositor's choice). A failure is logged and the pill shows wherever it is.
fn keep_on_top<R: Runtime>(app: &AppHandle<R>, window: &tauri::WebviewWindow<R>) {
    let Some(stacking) = app.try_state::<PillStacking>() else { return };
    #[cfg(target_os = "windows")]
    {
        let Ok(hwnd) = window.hwnd() else { return };
        let hwnd = hwnd.0 as isize;
        if !voltip_overlay::windows::raise(hwnd) {
            tracing::warn!("overlay raise refused");
        }
        let mut keeper = stacking.keeper.lock();
        if keeper.is_none() {
            match voltip_overlay::windows::Keeper::start(hwnd) {
                Ok(started) => *keeper = Some(started),
                Err(e) => tracing::warn!(error = %e, "overlay keeper not started"),
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = &stacking;
        let target = window.clone();
        if let Err(e) = window.run_on_main_thread(move || match target.ns_window() {
            Ok(ns_window) => voltip_overlay::macos::raise(ns_window),
            Err(e) => tracing::warn!(error = %e, "overlay window handle unavailable"),
        }) {
            tracing::warn!(error = %e, "overlay raise not scheduled");
        }
    }
    #[cfg(target_os = "linux")]
    {
        use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
        // tao's keep-above request again (`_NET_WM_STATE_ABOVE`), for a window manager that let it go.
        if let Err(e) = window.set_always_on_top(true) {
            tracing::debug!(error = %e, "overlay keep-above request failed");
        }
        let xid = window.window_handle().ok().and_then(|handle| match handle.as_raw() {
            RawWindowHandle::Xlib(xlib) => u32::try_from(xlib.window).ok(),
            _ => None,
        });
        if let Some(xid) = xid
            && let Err(e) = stacking.x11.raise(xid)
        {
            tracing::debug!(error = %e, "overlay restack failed");
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    let _ = (stacking, window);
}

/// The pill is hidden: nothing keeps it on top any more (Windows: the keeper's hook goes).
fn let_go<R: Runtime>(app: &AppHandle<R>) {
    #[cfg(target_os = "windows")]
    if let Some(stacking) = app.try_state::<PillStacking>() {
        drop(stacking.keeper.lock().take());
    }
    #[cfg(not(target_os = "windows"))]
    let _ = app;
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

    /// 2026-11-01: hiding waits a frame; a take that showed the pill in that frame (its first phase
    /// right after the previous take's idle) must keep it, and a hide with no show since still hides.
    #[test]
    fn regression_a_show_after_a_hide_began_keeps_the_pill() {
        let slot = OverlaySlot::default();
        let ticket = slot.hide_ticket();
        assert!(slot.may_hide(ticket), "nothing was shown since the hide began");
        let ticket = slot.hide_ticket();
        slot.shown();
        assert!(!slot.may_hide(ticket), "the next take showed the pill while the hide waited");
        assert!(slot.may_hide(slot.hide_ticket()), "a later hide, with no show after it, hides");
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
