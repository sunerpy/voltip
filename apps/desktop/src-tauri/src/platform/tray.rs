//! The tray icon (macOS menu bar / Windows notification area, docs/dictation.md §15.4): the app
//! mark with a badge that follows the dictation phase (drawn by `voltip_platform::tray`), and a
//! menu in the UI's language: 打开 Voltip, the AI 润色 submenu (the switch and every preset,
//! docs/dictation.md §21), 设置…, 检查更新… (only when the build has an update source) and 退出
//! Voltip. On Windows a left click shows the main window and a right click opens the menu; on macOS
//! a click opens the menu, as with every menu bar item. The menu is rebuilt when the language, the
//! engine settings or the custom presets change.

use parking_lot::Mutex;
use serde::Serialize;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter as _, Manager as _, Runtime};
use voltip_core::ui::UiEvent;
use voltip_platform::HostOs;
use voltip_platform::tray::{
    TRAY_POLISH_ID, TRAY_POLISH_TOGGLE_ID, TrayAction, TrayGlyph, TrayLocale, TrayPolish, TrayPolishAction, TrayStyle, polish_menu_label, polish_toggle_label,
    render_tray_icon, tray_icon_size, tray_preset_id, tray_tooltip,
};
use voltip_tauri_bridge::{Bridge, UiCommand};

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

/// What the icon and the menu show. Managed as Tauri state once the tray is up.
pub struct TrayState {
    shown: Mutex<Shown>,
    menu: Mutex<MenuModel>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Shown {
    glyph: TrayGlyph,
    locale: TrayLocale,
}

/// What the menu lists; a change rebuilds it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct MenuModel {
    locale: TrayLocale,
    updater: bool,
    polish: TrayPolish,
}

/// The menu for `model`: the entries in `TrayAction` order with the AI 润色 submenu after 打开
/// Voltip, and a separator before 退出 Voltip. The presets are check items of which the current
/// one is checked (menus have no radio items).
fn build_menu<R: Runtime>(app: &AppHandle<R>, model: &MenuModel) -> tauri::Result<Menu<R>> {
    let locale = model.locale;
    let toggle = CheckMenuItem::with_id(app, TRAY_POLISH_TOGGLE_ID, polish_toggle_label(locale), true, model.polish.enabled, None::<&str>)?;
    let rule = PredefinedMenuItem::separator(app)?;
    let presets = model
        .polish
        .presets
        .iter()
        .map(|preset| CheckMenuItem::with_id(app, tray_preset_id(&preset.id), &preset.label, true, preset.checked, None::<&str>))
        .collect::<tauri::Result<Vec<_>>>()?;
    let mut polish_items: Vec<&dyn IsMenuItem<R>> = vec![&toggle, &rule];
    polish_items.extend(presets.iter().map(|item| item as &dyn IsMenuItem<R>));
    let polish = Submenu::with_id_and_items(app, TRAY_POLISH_ID, polish_menu_label(locale), true, &polish_items)?;
    let items = TrayAction::ALL
        .into_iter()
        .filter(|a| a.shown(model.updater))
        .map(|action| Ok((action, MenuItem::with_id(app, action.id(), action.label(locale), true, None::<&str>)?)))
        .collect::<tauri::Result<Vec<_>>>()?;
    let separator = PredefinedMenuItem::separator(app)?;
    let mut entries: Vec<&dyn IsMenuItem<R>> = Vec::new();
    for (action, item) in &items {
        if *action == TrayAction::Quit {
            entries.push(&separator);
        }
        entries.push(item);
        if *action == TrayAction::Open {
            entries.push(&polish);
        }
    }
    Menu::with_items(app, &entries)
}

fn image(glyph: TrayGlyph) -> Image<'static> {
    let os = HostOs::current();
    let size = tray_icon_size(os, super::small_icon_size());
    Image::new_owned(render_tray_icon(glyph, size, TrayStyle::for_host(os)), size, size)
}

/// Create the tray icon and its menu in `locale` with the AI 润色 submenu `polish`; `updater`
/// adds 检查更新….
pub fn install<R: Runtime>(app: &AppHandle<R>, locale: TrayLocale, updater: bool, polish: TrayPolish) -> tauri::Result<()> {
    let model = MenuModel { locale, updater, polish };
    let menu = build_menu(app, &model)?;
    let macos = cfg!(target_os = "macos");
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(image(TrayGlyph::Idle))
        .icon_as_template(macos)
        .tooltip(tray_tooltip(TrayGlyph::Idle, locale))
        .menu(&menu)
        .show_menu_on_left_click(macos)
        .on_menu_event(|app, event: MenuEvent| {
            let id = event.id().as_ref();
            if let Some(action) = TrayAction::from_id(id) {
                run(app, action);
            } else if let Some(action) = TrayPolishAction::from_id(id) {
                run_polish(app, action);
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
    app.manage(TrayState { shown: Mutex::new(Shown { glyph: TrayGlyph::Idle, locale }), menu: Mutex::new(model) });
    tracing::info!(?locale, updater, "tray icon installed");
    Ok(())
}

/// An AI 润色 entry: the engine settings with the switch flipped or the preset chosen, through the
/// bridge like the webview's own `settings_set_engines`; the core's `settings` event redraws the
/// menu. The OS flips a check item on its own when clicked, so the menu is also redrawn from the
/// settings as they are (choosing the preset in use changes nothing, and nothing else redraws it).
fn run_polish<R: Runtime>(app: &AppHandle<R>, action: TrayPolishAction<'_>) {
    let Some(bridge) = app.try_state::<Bridge>() else { return };
    let bridge = bridge.inner().clone();
    let state = bridge.state();
    match super::tray_polish_engines(&state.settings.engines, action) {
        Some(engines) => {
            tracing::info!(?action, "tray menu polish");
            if let Err(e) = bridge.dispatch(UiCommand::SettingsSetEngines { engines }) {
                tracing::warn!(error = %e, ?action, "tray polish entry not applied");
            }
        }
        None => tracing::warn!(?action, "tray polish entry names no preset"),
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move { sync(&app, &bridge, true) });
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
    let state = bridge.state();
    let locale = super::tray_locale(state.settings.locale);
    match install(app, locale, updater, super::tray_polish(&state, locale)) {
        Ok(()) => follow_dictation(app.clone(), bridge),
        Err(e) => tracing::warn!(error = %e, "tray icon reinstall failed"),
    }
}

/// Redraw what changed: the badge for `glyph`, the tooltip for `locale`. A missing tray (install
/// failed) is not an error worth more than a debug line.
fn show<R: Runtime>(app: &AppHandle<R>, next: Shown) {
    let (Some(tray), Some(state)) = (app.tray_by_id(TRAY_ID), app.try_state::<TrayState>()) else {
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
    if previous != next
        && let Err(e) = tray.set_tooltip(Some(tray_tooltip(next.glyph, next.locale)))
    {
        tracing::warn!(error = %e, "tray tooltip update failed");
    }
}

/// The badge for `glyph`.
pub fn set_glyph<R: Runtime>(app: &AppHandle<R>, glyph: TrayGlyph) {
    let Some(state) = app.try_state::<TrayState>() else { return };
    let current = *state.shown.lock();
    show(app, Shown { glyph, ..current });
}

/// The tooltip's language, and the menu for the bridge's state (its language, the AI 润色 switch
/// and the presets): rebuilt when it changed, or always with `force`.
fn sync<R: Runtime>(app: &AppHandle<R>, bridge: &Bridge, force: bool) {
    let (Some(tray), Some(tray_state)) = (app.tray_by_id(TRAY_ID), app.try_state::<TrayState>()) else { return };
    let state = bridge.state();
    let locale = super::tray_locale(state.settings.locale);
    let current = *tray_state.shown.lock();
    show(app, Shown { locale, ..current });
    let next = {
        let mut menu = tray_state.menu.lock();
        let next = MenuModel { locale, updater: menu.updater, polish: super::tray_polish(&state, locale) };
        if !force && *menu == next {
            return;
        }
        *menu = next.clone();
        next
    };
    match build_menu(app, &next) {
        Ok(menu) => match tray.set_menu(Some(menu)) {
            Ok(()) => tracing::info!(?locale, polish = next.polish.enabled, "tray menu rebuilt"),
            Err(e) => tracing::warn!(error = %e, "tray menu update failed"),
        },
        Err(e) => tracing::warn!(error = %e, "tray menu rebuild failed"),
    }
}

/// Keep the badge in step with `UiEvent::Dictation` for as long as the bridge lives (the same
/// subscription shape as `overlay::follow_dictation`), and the menu with the language, the engine
/// settings and the custom presets.
pub fn follow_dictation<R: Runtime>(app: AppHandle<R>, bridge: Bridge) {
    let mut events = bridge.events();
    tauri::async_runtime::spawn(async move {
        loop {
            match events.recv().await {
                Ok(UiEvent::Dictation(status)) => set_glyph(&app, glyph_for(&status.phase)),
                Ok(UiEvent::Settings(_) | UiEvent::State(_) | UiEvent::Presets { .. }) => sync(&app, &bridge, false),
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    set_glyph(&app, glyph_for(&bridge.state().dictation.phase));
                    sync(&app, &bridge, false);
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}
