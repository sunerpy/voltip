//! When the system keychain is unavailable as Voltip starts (docs/dictation.md §34): the core can
//! neither read nor create the device identity, so instead of the main window the shell shows a
//! native notice that says what Voltip needs ([`KeychainNotice`]) and quits with [`EXIT_CODE`]
//! once it is closed. Nothing falls back to a weaker store: a release build keeps its secrets in
//! the keychain or not at all (docs/threat-model.md).

use tauri::{AppHandle, Manager as _, Runtime};
use tauri_plugin_dialog::{DialogExt as _, MessageDialogButtons, MessageDialogKind};
use voltip_core::CoreError;
use voltip_identity::IdentityError;
use voltip_platform::HostOs;
use voltip_platform::keychain::KeychainNotice;
use voltip_platform::tray::TrayLocale;
use voltip_tauri_bridge::BridgeError;

/// The exit code once the notice is closed: a start that failed, as for `voltip-server`
/// (docs/dictation.md §23.5).
pub const EXIT_CODE: i32 = 1;

/// A start the keychain stopped: what the notice says and the code Voltip quits with. Managed as
/// Tauri state while the notice is up, so nothing brings up the main window behind it: without
/// the core it would stay empty.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeychainUnavailable {
    /// The notice, for this host in the system language.
    pub notice: KeychainNotice,
    /// The process exit code after the notice.
    pub exit_code: i32,
}

impl KeychainUnavailable {
    /// The notice for a core that did not start with `error`, in `locale`; `None` when the
    /// keychain was not what stopped it.
    pub fn for_error(error: &BridgeError, locale: TrayLocale) -> Option<Self> {
        matches!(error, BridgeError::Core(CoreError::Identity(IdentityError::StoreUnavailable(_))))
            .then(|| Self { notice: KeychainNotice::new(HostOs::current(), locale), exit_code: EXIT_CODE })
    }
}

/// What showing the notice did with the main window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MainWindow {
    /// Hidden, so the notice comes alone.
    Hidden,
    /// Hiding it failed: on macOS an empty main window stays behind the notice.
    NotHidden,
    /// There is no main window.
    Missing,
}

/// Show the notice and quit once it is closed, by its button or by closing its window. Called from
/// the setup hook, which runs on the event loop's thread: `show` returns at once and the loop keeps
/// turning, which the dialog needs to appear (`blocking_show` would wait for an answer the loop
/// never gets to draw). The main window is hidden first: Linux and Windows create it invisible,
/// but macOS creates it visible (`tauri.macos.conf.json` replaces the window list without
/// `"visible": false`), and without the core it would stay empty behind the notice.
/// `shows_notices` off (the mock runtime): the state is managed and the window hidden, no dialog.
pub fn show<R: Runtime>(app: &AppHandle<R>, blocked: KeychainUnavailable, shows_notices: bool) -> MainWindow {
    app.manage(blocked);
    let main_window = hide_main_window(app);
    if shows_notices {
        let handle = app.clone();
        app.dialog()
            .message(blocked.notice.message)
            .title(blocked.notice.title)
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCustom(blocked.notice.button.to_owned()))
            .show(move |_| quit(&handle, blocked.exit_code));
    }
    main_window
}

/// Hide the main window, when there is one, so that the notice shows alone.
fn hide_main_window<R: Runtime>(app: &AppHandle<R>) -> MainWindow {
    let Some(window) = app.get_webview_window(crate::MAIN_WINDOW) else { return MainWindow::Missing };
    match window.hide() {
        Ok(()) => MainWindow::Hidden,
        Err(e) => {
            tracing::warn!(error = %e, "the main window could not be hidden behind the keychain notice");
            MainWindow::NotHidden
        }
    }
}

/// Leave with `code` on the event loop's thread, where macOS and Windows run their exit handlers;
/// right here when the loop is gone. The dialog answers on a thread of its own.
fn quit<R: Runtime>(app: &AppHandle<R>, code: i32) {
    tracing::info!(code, "the keychain notice was closed; quitting");
    if app.run_on_main_thread(move || crate::exit::exit_process(code)).is_err() {
        crate::exit::exit_process(code);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only a keychain the core could not open stops the start with the notice: every other
    /// failure stays what it was.
    #[test]
    fn only_an_unavailable_keychain_is_explained() {
        let locked =
            BridgeError::Core(CoreError::Identity(IdentityError::StoreUnavailable("Couldn't access platform storage: SS error: prompt dismissed".into())));
        for locale in [TrayLocale::ZhCn, TrayLocale::En] {
            let notice = KeychainNotice::new(HostOs::current(), locale);
            assert_eq!(KeychainUnavailable::for_error(&locked, locale), Some(KeychainUnavailable { notice, exit_code: EXIT_CODE }));
        }
        assert_ne!(EXIT_CODE, 0, "closing the notice ends a start that did not happen");
        for other in [
            BridgeError::Core(CoreError::Identity(IdentityError::Corrupt)),
            BridgeError::Core(CoreError::Settings("settings.json: expected value".into())),
            BridgeError::BadArgument("no".into()),
        ] {
            assert_eq!(KeychainUnavailable::for_error(&other, TrayLocale::En), None, "{other}");
        }
    }
}
