//! macOS facts: TCC answers through `tauri-plugin-macos-permissions` (docs/dictation.md §15.1) and
//! the frontmost application through AppKit (§18.2). The plugin's functions are plain
//! `pub async fn`s; calling them from Rust keeps its twelve webview commands unregistered (no
//! capability grant needed).

use objc2::runtime::{AnyObject, NSObjectProtocol as _};
use objc2_app_kit::{NSRunningApplication, NSWorkspace};
use tauri_plugin_macos_permissions as tcc;
use voltip_core::ForegroundApp;
use voltip_platform::foreground::from_bundle;
use voltip_platform::{Permission, PermissionReport, PermissionState};

/// The frontmost application when a take starts (docs/dictation.md §18.2): its bundle identifier
/// (the id) and localised name. Voltip itself, or an application without a bundle id, is no
/// answer. No window title: reading it needs the Accessibility API, not used here. `NSWorkspace`
/// is not main-thread-only, so the core's blocking thread may ask.
pub fn foreground_app() -> Result<Option<ForegroundApp>, String> {
    let Some(front) = NSWorkspace::sharedWorkspace().frontmostApplication() else { return Ok(None) };
    let current = NSRunningApplication::currentApplication();
    let current: &AnyObject = &current;
    if front.isEqual(Some(current)) {
        return Ok(None);
    }
    let bundle = front.bundleIdentifier().map(|s| s.to_string());
    let name = front.localizedName().map(|s| s.to_string());
    Ok(from_bundle(bundle.as_deref(), name.as_deref()).map(|id| ForegroundApp { app_id: id.app_id, name: id.name, title: None }))
}

/// Both TCC states. The APIs are boolean (`AXIsProcessTrusted`,
/// `AVCaptureDevice.authorizationStatus == authorized`), so `false` reads as `denied`: the
/// onboarding step then offers the request rather than assuming a prompt will come.
pub async fn permissions_status() -> PermissionReport {
    let microphone = tcc::check_microphone_permission().await;
    let accessibility = tcc::check_accessibility_permission().await;
    PermissionReport::for_host()
        .with(Permission::Microphone, PermissionState::from_flag(microphone))
        .with(Permission::Accessibility, PermissionState::from_flag(accessibility))
}

/// Trigger the system's own flow: the microphone consent sheet (`requestAccessForMediaType:`) or
/// the Accessibility prompt (`AXIsProcessTrustedWithOptions` with the prompt option, which opens
/// System Settings ▸ Privacy ▸ Accessibility).
pub async fn permissions_request(permission: Permission) -> Result<(), String> {
    match permission {
        Permission::Microphone => tcc::request_microphone_permission().await,
        Permission::Accessibility => {
            tcc::request_accessibility_permission().await;
            Ok(())
        }
    }
}
