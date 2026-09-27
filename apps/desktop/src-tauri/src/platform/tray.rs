//! The tray icon (macOS menu bar / Windows notification area): a left click shows the main window,
//! the glyph follows the dictation phase (docs/dictation.md §15.4). Glyphs are rendered by
//! `voltip_platform::tray` at runtime; on macOS the image is a *template* so the menu bar tints it
//! for the light and dark themes, on Windows it is drawn in the accent colour.

use tauri::image::Image;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager as _, Runtime};
use voltip_core::ui::UiEvent;
use voltip_platform::tray::{TRAY_ACCENT_RGB, TRAY_ICON_SIZE, TRAY_TEMPLATE_RGB, TrayGlyph, render_glyph};
use voltip_tauri_bridge::Bridge;

use super::glyph_for;

/// Tray icon id (one per process).
pub const TRAY_ID: &str = "voltip-tray";

fn image(glyph: TrayGlyph) -> Image<'static> {
    let rgb = if cfg!(target_os = "macos") { TRAY_TEMPLATE_RGB } else { TRAY_ACCENT_RGB };
    Image::new_owned(render_glyph(glyph, TRAY_ICON_SIZE, rgb), TRAY_ICON_SIZE, TRAY_ICON_SIZE)
}

/// Create the tray icon. A left click brings the main window to the front; there is no menu in
/// this increment (the window is the menu).
pub fn install<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(image(TrayGlyph::Idle))
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("Voltip")
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                crate::show_main_window(tray.app_handle());
            }
        })
        .build(app)?;
    tracing::info!("tray icon installed");
    Ok(())
}

/// `RunEvent::Reopen` (macOS Dock click / `open -a`): put the tray back when it is missing (the
/// install failed at launch) and wire it to the running core again. Nothing to do before the core
/// is up or when the tray exists.
pub fn ensure_installed<R: Runtime>(app: &AppHandle<R>) {
    if app.tray_by_id(TRAY_ID).is_some() {
        return;
    }
    let Some(bridge) = app.try_state::<Bridge>() else { return };
    let bridge = bridge.inner().clone();
    match install(app) {
        Ok(()) => follow_dictation(app.clone(), bridge),
        Err(e) => tracing::warn!(error = %e, "tray icon reinstall failed"),
    }
}

/// Swap the glyph; a missing tray (install failed) is not an error worth more than a debug line.
pub fn set_glyph<R: Runtime>(app: &AppHandle<R>, glyph: TrayGlyph) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        tracing::debug!(?glyph, "no tray icon to update");
        return;
    };
    // `set_icon` alone would drop the template flag on macOS (the glyph turns black on a dark menu
    // bar); the combined call keeps it and falls back to `set_icon` elsewhere.
    if let Err(e) = tray.set_icon_with_as_template(Some(image(glyph)), cfg!(target_os = "macos")) {
        tracing::warn!(error = %e, ?glyph, "tray icon update failed");
    }
}

/// Keep the glyph in step with `UiEvent::Dictation` for as long as the bridge lives (the same
/// subscription shape as `overlay::follow_dictation`).
pub fn follow_dictation<R: Runtime>(app: AppHandle<R>, bridge: Bridge) {
    let mut events = bridge.events();
    tauri::async_runtime::spawn(async move {
        let mut shown = TrayGlyph::Idle;
        loop {
            let glyph = match events.recv().await {
                Ok(UiEvent::Dictation(status)) => glyph_for(&status.phase),
                Ok(_) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => glyph_for(&bridge.state().dictation.phase),
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            if glyph != shown {
                shown = glyph;
                set_glyph(&app, glyph);
            }
        }
    });
}
