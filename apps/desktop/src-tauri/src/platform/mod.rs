//! Platform facts for the shell (docs/dictation.md §15, §18): the three query commands
//! `permissions_status` / `permissions_request` / `inject_preflight`, the tray, and the
//! foreground-application probe the core asks when a take starts ([`PlatformProbe`]).
//!
//! The decisions live in `voltip-platform` (pure tables, tested on every host); this module only
//! collects the facts the OS hands out. Each `#[cfg]` branch answers with the same wire types, so
//! the webview and `tests/ipc.rs` see one contract on Linux, macOS and Windows.

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub mod tray;
#[cfg(target_os = "windows")]
pub mod windows;

use tauri::{AppHandle, Manager as _, Runtime};
use voltip_core::ui::UiState;
use voltip_core::{BuiltinPreset, DictationPhase, EngineSettings, ForegroundApp, ForegroundProbe, Locale, PresetId};
use voltip_platform::tray::{CloseAction, TrayGlyph, TrayLocale, TrayPolish, TrayPolishAction, TrayPreset, builtin_preset_label, main_window_close};
use voltip_tauri_bridge::Bridge;

pub use voltip_platform::{HostOs, InjectDecision, InjectPreflight, Permission, PermissionReport, PermissionState};

/// The tray glyph for a dictation phase (docs/dictation.md §15.4): the disc while the microphone is
/// open, the dotted ring while the pipeline runs, the plain ring otherwise (including the `done` /
/// `failed` / `cancelled` dwell). Compiled on every host so the match is checked against the core's
/// enum even where no tray exists.
pub const fn glyph_for(phase: &DictationPhase) -> TrayGlyph {
    match phase {
        DictationPhase::Listening { .. } => TrayGlyph::Listening,
        DictationPhase::Processing { .. } => TrayGlyph::Processing,
        DictationPhase::Idle | DictationPhase::Done { .. } | DictationPhase::Failed { .. } | DictationPhase::Cancelled { .. } => TrayGlyph::Idle,
    }
}

/// Whether this build carries a tray icon (macOS menu bar, Windows notification area). Linux ships
/// none in this increment, so its `--start-hidden` still relies on the hotkey and a second launch.
pub const TRAY_AVAILABLE: bool = cfg!(any(target_os = "macos", target_os = "windows"));

/// The tray menu's language: `settings.locale`, with `system` resolved the way the webview
/// resolves `navigator.language` (the OS display language: `GetUserDefaultUILanguage` on Windows,
/// the first preferred language on macOS). Chinese when the OS cannot say.
pub fn tray_locale(locale: Locale) -> TrayLocale {
    match locale {
        Locale::ZhCn => TrayLocale::ZhCn,
        Locale::En => TrayLocale::En,
        Locale::System => system_tray_locale().unwrap_or_default(),
    }
}

fn system_tray_locale() -> Option<TrayLocale> {
    #[cfg(target_os = "windows")]
    {
        Some(if windows::ui_language_is_chinese() { TrayLocale::ZhCn } else { TrayLocale::En })
    }
    #[cfg(target_os = "macos")]
    {
        macos::preferred_language().map(|language| TrayLocale::for_language(&language))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        None
    }
}

/// The tray's AI 润色 submenu for `state` (docs/dictation.md §21): the settings' switch and every
/// preset, the built-in ones in the menu's language, then the custom ones by their names.
pub fn tray_polish(state: &UiState, locale: TrayLocale) -> TrayPolish {
    let engines = &state.settings.engines;
    let current = engines.refine_preset;
    let builtin = BuiltinPreset::ALL.into_iter().map(|preset| TrayPreset {
        id: preset.as_str().to_owned(),
        label: builtin_preset_label(preset.as_str(), locale).unwrap_or(preset.display_name()).to_owned(),
        checked: current == PresetId::Builtin(preset),
    });
    let custom = state.presets.iter().map(|preset| TrayPreset {
        id: preset.id.to_string(),
        label: preset.name.clone(),
        checked: current == PresetId::Custom(preset.id),
    });
    TrayPolish { enabled: engines.refine_enabled, presets: builtin.chain(custom).collect() }
}

/// The engine settings an AI 润色 entry asks for: the switch flipped, or the preset it names; the
/// rest as they are. `None` when the entry names no preset.
pub fn tray_polish_engines(engines: &EngineSettings, action: TrayPolishAction<'_>) -> Option<EngineSettings> {
    let mut next = engines.clone();
    match action {
        TrayPolishAction::Toggle => next.refine_enabled = !next.refine_enabled,
        TrayPolishAction::Preset(id) => next.refine_preset = PresetId::parse(id)?,
    }
    Some(next)
}

/// The notification area's small-icon size at the system DPI (Windows); `None` elsewhere.
pub fn small_icon_size() -> Option<u32> {
    #[cfg(target_os = "windows")]
    {
        windows::small_icon_size()
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

/// The main window's close button (the title bar's ×, Alt+F4, the red traffic light): hidden
/// where the tray or the Dock brings it back, otherwise the app quits
/// (`voltip_platform::tray::main_window_close`). Without the explicit quit the prewarmed pill
/// window would keep the process alive with no window left to show (user report 2026-09-28).
pub fn on_window_event<R: Runtime>(window: &tauri::Window<R>, event: &tauri::WindowEvent) {
    let tauri::WindowEvent::CloseRequested { api, .. } = event else { return };
    if window.label() != crate::MAIN_WINDOW {
        return;
    }
    let app = window.app_handle();
    match main_window_close(voltip_platform::HostOs::current(), tray_installed(app)) {
        CloseAction::Hide => {
            api.prevent_close();
            if let Err(e) = window.hide() {
                tracing::warn!(error = %e, "main window hide failed");
            }
        }
        CloseAction::Quit => app.exit(0),
    }
}

fn tray_installed<R: Runtime>(app: &AppHandle<R>) -> bool {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        app.tray_by_id(tray::TRAY_ID).is_some()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = app;
        false
    }
}

/// One state per permission for the compiled host. Linux / other: everything `not_applicable`.
/// macOS: TCC through `tauri-plugin-macos-permissions`. Windows: the microphone consent store; the
/// two macOS-only permissions read `not_applicable`.
pub async fn permissions_status() -> PermissionReport {
    #[cfg(target_os = "macos")]
    {
        macos::permissions_status().await
    }
    #[cfg(target_os = "windows")]
    {
        windows::permissions_status()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        PermissionReport::not_applicable(HostOs::current())
    }
}

/// Ask the OS for `permission`: on macOS the system prompt (microphone) or the Accessibility prompt
/// that opens System Settings; on Windows the microphone privacy page
/// of Settings (Windows never prompts a desktop app). Everything else is an accepted no-op, so the
/// webview needs no platform branch.
pub async fn permissions_request(permission: Permission) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        macos::permissions_request(permission).await
    }
    #[cfg(target_os = "windows")]
    {
        windows::permissions_request(permission)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        tracing::debug!(permission = permission.as_str(), "permissions_request is a no-op on this platform");
        Ok(())
    }
}

/// Whether an injection into the current foreground window would land (docs/dictation.md §15.3).
/// Windows compares token integrity levels and checks the input desktop; other hosts have no
/// UIPI and answer `proceed` with `checked: false`.
pub fn inject_preflight() -> InjectPreflight {
    #[cfg(target_os = "windows")]
    {
        windows::inject_preflight()
    }
    #[cfg(not(target_os = "windows"))]
    {
        InjectPreflight::not_applicable(HostOs::current())
    }
}

/// The desktop's foreground probe (docs/dictation.md §18.2): Win32 on Windows, AppKit on macOS,
/// X11 / XWayland on Linux (pure Wayland answers `None`); other hosts never answer. The core calls
/// it on a blocking thread when a take starts and waits at most `PROBE_DEADLINE`.
#[derive(Default)]
pub struct PlatformProbe {
    #[cfg(target_os = "linux")]
    x11: linux::X11Probe,
}

impl PlatformProbe {
    /// A probe for this host (nothing is opened until the first take).
    pub fn new() -> Self {
        Self::default()
    }
}

impl ForegroundProbe for PlatformProbe {
    fn foreground(&self) -> Result<Option<ForegroundApp>, String> {
        #[cfg(target_os = "windows")]
        {
            windows::foreground_app()
        }
        #[cfg(target_os = "macos")]
        {
            macos::foreground_app()
        }
        #[cfg(target_os = "linux")]
        {
            self.x11.foreground()
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        {
            Ok(None)
        }
    }
}

/// The tray where this build has one (macOS / Windows, docs/dictation.md §15.4). A failed install
/// with `--start-hidden` would leave the user without any way back to the window (on macOS the
/// Dock icon is gone too under the `Accessory` policy, applied before the event loop started), so
/// that case restores the `Regular` policy and shows the window instead.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn install_tray<R: Runtime>(app: &AppHandle<R>, bridge: &Bridge, start_hidden: bool, updater: bool) {
    let state = bridge.state();
    let locale = tray_locale(state.settings.locale);
    match tray::install(app, locale, updater, tray_polish(&state, locale)) {
        Ok(()) => tray::follow_dictation(app.clone(), bridge.clone()),
        Err(e) => {
            tracing::warn!(error = %e, "tray icon not installed");
            if start_hidden {
                #[cfg(target_os = "macos")]
                if let Err(e) = app.set_activation_policy(tauri::ActivationPolicy::Regular) {
                    tracing::warn!(error = %e, "activation policy fallback failed");
                }
                crate::show_main_window(app);
            }
        }
    }
}

/// No tray on this platform in this increment: `--start-hidden` relies on the hotkey and a second
/// launch (which shows the window).
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn install_tray<R: Runtime>(_app: &AppHandle<R>, _bridge: &Bridge, start_hidden: bool, _updater: bool) {
    if start_hidden {
        tracing::info!("no tray on this platform: --start-hidden relies on the hotkey and a second launch");
    }
}

/// Between `Builder::build` and `App::run` (docs/dictation.md §15.2): on macOS apply
/// `activation_policy_plan(start_hidden, TRAY_AVAILABLE)`; only a menu-bar launch drops the Dock
/// icon. The `setup` hook (tray install and its fallback) runs later, on the event loop's `Ready`.
#[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
pub fn before_run<R: Runtime>(app: &mut tauri::App<R>, start_hidden: bool) {
    #[cfg(target_os = "macos")]
    {
        use voltip_platform::macos::{ActivationPolicy, PolicyTiming, activation_policy_plan};
        let plan = activation_policy_plan(start_hidden, TRAY_AVAILABLE);
        if plan.timing == PolicyTiming::BetweenBuildAndRun {
            app.set_activation_policy(match plan.policy {
                ActivationPolicy::Accessory => tauri::ActivationPolicy::Accessory,
                ActivationPolicy::Regular => tauri::ActivationPolicy::Regular,
            });
            tracing::info!(?plan, "activation policy applied");
        }
    }
}

/// The event-loop callback. macOS `Reopen` (Dock click / `open -a Voltip` while running): show the
/// main window (it may be hidden or on another Space) and put the tray back if it went missing;
/// other hosts never emit it. `Exit`: after an update, start the new build (`restart::on_exit`).
pub fn on_run_event<R: Runtime>(app: &AppHandle<R>, event: &tauri::RunEvent) {
    #[cfg(target_os = "macos")]
    if let tauri::RunEvent::Reopen { .. } = event {
        crate::show_main_window(app);
        tray::ensure_installed(app);
    }
    if let tauri::RunEvent::Exit = event {
        crate::restart::on_exit(app);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn phase(json: serde_json::Value) -> DictationPhase {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn tray_glyph_follows_the_dictation_phase() {
        let table = [
            (serde_json::json!({ "phase": "idle" }), TrayGlyph::Idle),
            (serde_json::json!({ "phase": "listening", "started_at": 1 }), TrayGlyph::Listening),
            (serde_json::json!({ "phase": "listening", "started_at": 1, "locked": true }), TrayGlyph::Listening),
            (serde_json::json!({ "phase": "processing", "stage": "transcribing", "started_at": 1 }), TrayGlyph::Processing),
            (serde_json::json!({ "phase": "processing", "stage": "inserting", "started_at": 1 }), TrayGlyph::Processing),
            (
                serde_json::json!({ "phase": "done", "text": "x", "raw_text": "x", "chars": 1, "via": "paste", "refined": false, "duration_ms": 1, "asr_ms": 1 }),
                TrayGlyph::Idle,
            ),
            (serde_json::json!({ "phase": "failed", "code": "asr", "message": "x" }), TrayGlyph::Idle),
            (serde_json::json!({ "phase": "cancelled" }), TrayGlyph::Idle),
        ];
        for (json, glyph) in table {
            assert_eq!(glyph_for(&phase(json.clone())), glyph, "{json}");
        }
        assert_eq!(TRAY_AVAILABLE, cfg!(any(target_os = "macos", target_os = "windows")));
    }

    /// docs/dictation.md §21: the tray's AI 润色 submenu lists every preset with the one in use
    /// checked, and its entries ask for the engine settings with only that one field changed.
    #[test]
    fn the_tray_polish_submenu_follows_the_settings_and_the_presets() {
        let weekly = voltip_core::CustomPreset {
            id: uuid::Uuid::parse_str("7e57ab1e-0b0e-4c0d-9e5e-7e57ab1e0b0e").unwrap(),
            name: "周报".into(),
            prompt: "整理成周报".into(),
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        let mut state = UiState { presets: vec![weekly.clone()], ..UiState::default() };
        state.settings.engines.refine_preset = PresetId::Builtin(BuiltinPreset::Notes);
        let menu = tray_polish(&state, TrayLocale::ZhCn);
        assert!(menu.enabled);
        let labels: Vec<&str> = menu.presets.iter().map(|p| p.label.as_str()).collect();
        assert_eq!(labels, ["校对", "提示词优化", "意图整理", "口语聊天", "中英互译", "要点纪要", "只加标点", "书面语", "周报"]);
        let checked: Vec<&str> = menu.presets.iter().filter(|p| p.checked).map(|p| p.id.as_str()).collect();
        assert_eq!(checked, ["notes"]);
        // The built-in names in the menu's language are the core's names in Chinese.
        for preset in BuiltinPreset::ALL {
            assert_eq!(builtin_preset_label(preset.as_str(), TrayLocale::ZhCn), Some(preset.display_name()));
        }
        assert_eq!(tray_polish(&state, TrayLocale::En).presets[0].label, "Proofread");
        state.settings.engines.refine_preset = PresetId::Custom(weekly.id);
        state.settings.engines.refine_enabled = false;
        let menu = tray_polish(&state, TrayLocale::En);
        assert!(!menu.enabled);
        assert_eq!(menu.presets.iter().filter(|p| p.checked).map(|p| p.label.as_str()).collect::<Vec<_>>(), ["周报"]);

        let engines = state.settings.engines.clone();
        let toggled = tray_polish_engines(&engines, TrayPolishAction::Toggle).unwrap();
        assert_eq!(toggled, EngineSettings { refine_enabled: true, ..engines.clone() });
        let chosen = tray_polish_engines(&engines, TrayPolishAction::Preset("translate")).unwrap();
        assert_eq!(chosen, EngineSettings { refine_preset: PresetId::Builtin(BuiltinPreset::Translate), ..engines.clone() });
        assert_eq!(tray_polish_engines(&engines, TrayPolishAction::Preset("casual")), None);
    }

    #[test]
    fn the_tray_menu_follows_the_locale_setting() {
        assert_eq!(tray_locale(Locale::ZhCn), TrayLocale::ZhCn);
        assert_eq!(tray_locale(Locale::En), TrayLocale::En);
        // `system`: the OS display language where the shell can read it; hosts without a tray
        // (Linux) fall back to Chinese.
        if cfg!(not(any(target_os = "macos", target_os = "windows"))) {
            assert_eq!(tray_locale(Locale::System), TrayLocale::ZhCn);
            assert_eq!(small_icon_size(), None);
        }
    }

    #[tokio::test]
    async fn host_answers_match_the_leaf_crate_skeletons() {
        let report = permissions_status().await;
        assert_eq!(report.platform, HostOs::current());
        let preflight = inject_preflight();
        assert_eq!(preflight.platform, HostOs::current());
        if cfg!(not(any(target_os = "macos", target_os = "windows"))) {
            assert_eq!(report, PermissionReport::not_applicable(HostOs::current()));
            assert_eq!(preflight, InjectPreflight::not_applicable(HostOs::current()));
            assert_eq!(permissions_request(Permission::Accessibility).await, Ok(()));
        }
    }
}
