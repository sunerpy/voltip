//! The tray icon (macOS menu bar / Windows notification area, docs/dictation.md §15.4): the app
//! mark with a badge that follows the dictation phase (drawn by `voltip_platform::tray`), and a
//! menu in the UI's language: 打开 Voltip, 设置…, 检查更新… (only when the build has an update
//! source) and 退出 Voltip. On Windows a left click shows the main window and a right click opens
//! the menu; on macOS a click opens the menu, as with every menu bar item.

use parking_lot::Mutex;
use serde::Serialize;
use tauri::image::Image;
use tauri::menu::{IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter as _, Manager as _, Runtime};
use voltip_core::ui::UiEvent;
use voltip_platform::HostOs;
use voltip_platform::tray::{TrayAction, TrayGlyph, TrayLocale, TrayStyle, render_tray_icon, tray_icon_size, tray_tooltip};
use voltip_tauri_bridge::Bridge;

use super::glyph_for;

/// Tray icon id (one per process).
pub const TRAY_ID: &str = "voltip-tray";
/// Event to the main window for a menu entry the webview completes once the window is up
/// (`{ "action": "settings" | "update" }`, `TrayAction::webview_action`).
pub const TRAY_EVENT: &str = "voltip://tray";

#[derive(Clone, Copy, Debug, Serialize)]
struct TrayRequest {
    action: &'static str,
}

/// The menu's items (their text follows the UI language) and what the icon shows. Managed as
/// Tauri state once the tray is up.
pub struct TrayState<R: Runtime> {
    items: Vec<(TrayAction, MenuItem<R>)>,
    shown: Mutex<Shown>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Shown {
    glyph: TrayGlyph,
    locale: TrayLocale,
}

fn image(glyph: TrayGlyph) -> Image<'static> {
    let os = HostOs::current();
    let size = tray_icon_size(os, super::small_icon_size());
    Image::new_owned(render_tray_icon(glyph, size, TrayStyle::for_host(os)), size, size)
}

/// Create the tray icon and its menu in `locale`; `updater` adds 检查更新….
pub fn install<R: Runtime>(app: &AppHandle<R>, locale: TrayLocale, updater: bool) -> tauri::Result<()> {
    let mut items = Vec::new();
    for action in TrayAction::ALL.into_iter().filter(|a| a.shown(updater)) {
        items.push((action, MenuItem::with_id(app, action.id(), action.label(locale), true, None::<&str>)?));
    }
    let separator = PredefinedMenuItem::separator(app)?;
    let mut entries: Vec<&dyn IsMenuItem<R>> = Vec::new();
    for (action, item) in &items {
        if *action == TrayAction::Quit {
            entries.push(&separator);
        }
        entries.push(item);
    }
    let menu = Menu::with_items(app, &entries)?;
    let macos = cfg!(target_os = "macos");
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(image(TrayGlyph::Idle))
        .icon_as_template(macos)
        .tooltip(tray_tooltip(TrayGlyph::Idle, locale))
        .menu(&menu)
        .show_menu_on_left_click(macos)
        .on_menu_event(|app, event: MenuEvent| {
            if let Some(action) = TrayAction::from_id(event.id().as_ref()) {
                run(app, action);
            }
        })
        .on_tray_icon_event(|tray, event| {
            if !cfg!(target_os = "macos")
                && let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event
            {
                crate::show_main_window(tray.app_handle());
            }
        })
        .build(app)?;
    app.manage(TrayState { items, shown: Mutex::new(Shown { glyph: TrayGlyph::Idle, locale }) });
    tracing::info!(?locale, updater, "tray icon installed");
    Ok(())
}

/// A menu entry: every one but 退出 brings the main window up first; the webview finishes the
/// ones that open a dialog.
pub fn run<R: Runtime>(app: &AppHandle<R>, action: TrayAction) {
    tracing::info!(?action, "tray menu");
    if action == TrayAction::Quit {
        app.exit(0);
        return;
    }
    crate::show_main_window(app);
    if let Some(action) = action.webview_action()
        && let Err(e) = app.emit_to(crate::MAIN_WINDOW, TRAY_EVENT, TrayRequest { action })
    {
        tracing::warn!(error = %e, action, "tray request not delivered");
    }
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
    let updater = app.try_state::<std::sync::Arc<crate::update::UpdateSlot>>().is_some_and(|slot| slot.enabled());
    match install(app, super::tray_locale(bridge.state().settings.locale), updater) {
        Ok(()) => follow_dictation(app.clone(), bridge),
        Err(e) => tracing::warn!(error = %e, "tray icon reinstall failed"),
    }
}

/// Redraw what changed: the badge for `glyph`, the menu and tooltip for `locale`. A missing tray
/// (install failed) is not an error worth more than a debug line.
fn show<R: Runtime>(app: &AppHandle<R>, next: Shown) {
    let (Some(tray), Some(state)) = (app.tray_by_id(TRAY_ID), app.try_state::<TrayState<R>>()) else {
        tracing::debug!(?next, "no tray icon to update");
        return;
    };
    let previous = std::mem::replace(&mut *state.shown.lock(), next);
    if previous.glyph != next.glyph {
        // `set_icon` alone would drop the template flag on macOS (the V turns black on a dark menu
        // bar); the combined call keeps it and falls back to `set_icon` elsewhere.
        if let Err(e) = tray.set_icon_with_as_template(Some(image(next.glyph)), cfg!(target_os = "macos")) {
            tracing::warn!(error = %e, glyph = ?next.glyph, "tray icon update failed");
        }
    }
    if previous.locale != next.locale {
        for (action, item) in &state.items {
            if let Err(e) = item.set_text(action.label(next.locale)) {
                tracing::warn!(error = %e, ?action, "tray menu text update failed");
            }
        }
    }
    if previous != next
        && let Err(e) = tray.set_tooltip(Some(tray_tooltip(next.glyph, next.locale)))
    {
        tracing::warn!(error = %e, "tray tooltip update failed");
    }
}

/// The badge for `glyph`.
pub fn set_glyph<R: Runtime>(app: &AppHandle<R>, glyph: TrayGlyph) {
    let Some(state) = app.try_state::<TrayState<R>>() else { return };
    let current = *state.shown.lock();
    show(app, Shown { glyph, ..current });
}

/// The menu's language (`settings.locale` changed).
pub fn set_locale<R: Runtime>(app: &AppHandle<R>, locale: TrayLocale) {
    let Some(state) = app.try_state::<TrayState<R>>() else { return };
    let current = *state.shown.lock();
    show(app, Shown { locale, ..current });
}

/// Keep the badge in step with `UiEvent::Dictation` for as long as the bridge lives (the same
/// subscription shape as `overlay::follow_dictation`), and the menu's language with
/// `settings.locale`.
pub fn follow_dictation<R: Runtime>(app: AppHandle<R>, bridge: Bridge) {
    let mut events = bridge.events();
    tauri::async_runtime::spawn(async move {
        loop {
            match events.recv().await {
                Ok(UiEvent::Dictation(status)) => set_glyph(&app, glyph_for(&status.phase)),
                Ok(UiEvent::Settings(settings)) => set_locale(&app, super::tray_locale(settings.locale)),
                Ok(UiEvent::State(state)) => set_locale(&app, super::tray_locale(state.settings.locale)),
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    let state = bridge.state();
                    set_glyph(&app, glyph_for(&state.dictation.phase));
                    set_locale(&app, super::tray_locale(state.settings.locale));
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}
