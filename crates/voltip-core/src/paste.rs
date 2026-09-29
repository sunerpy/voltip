//! Pasting a result from the history into the window the user came from (the home page's and the
//! history page's 「粘贴到上一个窗口」).
//!
//! The desktop shell moves Voltip out of the way, waits for another window to come to the front
//! and names it ([`PasteTarget::App`]), or says why there is none ([`PasteTarget::CopyOnly`]). The
//! core then checks the front window once more right before the paste: if it is no longer the one
//! the shell found, the text only goes to the clipboard ([`CopyReason::TargetChanged`]). A paste
//! from the history is not a new take and writes no history entry.

use serde::{Deserialize, Serialize};

use crate::dictation::{ForegroundApp, ForegroundProbe, Injection, Injector, Via};

/// Most characters a paste hands to the injector.
pub const MAX_PASTE_TEXT_CHARS: usize = 50_000;

/// Where a paste should go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PasteTarget {
    /// The window the shell found in front once Voltip got out of the way: paste there if it is
    /// still in front, else copy.
    App(ForegroundApp),
    /// No window to aim at: copy only, for this reason.
    CopyOnly(CopyReason),
}

/// Why the text went to the clipboard instead of the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CopyReason {
    /// The session cannot name the window in front (pure Wayland).
    NoProbe,
    /// No other window came to the front in time.
    Timeout,
    /// Another window came to the front before the paste.
    TargetChanged,
    /// The insert setting is clipboard only (`EngineSettings.inject`).
    ClipboardOnly,
    /// The paste did not go through, so the injector left the text on the clipboard.
    PasteFailed,
}

/// Why nothing was pasted or copied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PasteFailure {
    /// A take or another paste is running.
    Busy,
    /// Nothing to paste, or more than [`MAX_PASTE_TEXT_CHARS`].
    Invalid,
    /// The core did not answer in time.
    Timeout,
    /// Neither the paste nor the clipboard worked.
    Inject,
    /// This shell cannot paste (the phone).
    Unsupported,
}

/// What became of a paste (the `paste_text` command's answer).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PasteOutcome {
    /// Inserted into the window the user came from.
    Pasted,
    /// Left on the clipboard.
    Copied {
        /// Why.
        reason: CopyReason,
    },
    /// Neither.
    Failed {
        /// Why.
        reason: PasteFailure,
    },
}

/// A paste may go ahead: text within [`MAX_PASTE_TEXT_CHARS`] that is not only whitespace.
pub fn valid_paste_text(text: &str) -> bool {
    !text.trim().is_empty() && text.chars().count() <= MAX_PASTE_TEXT_CHARS
}

/// The window in front is still the one the shell found: the same application, and the same
/// window where both answers name one.
pub fn same_target(expected: &ForegroundApp, now: &ForegroundApp) -> bool {
    expected.app_id == now.app_id && expected.window.zip(now.window).is_none_or(|(a, b)| a == b)
}

/// The paste itself, blocking (the probe and the injector both block): copy for a
/// [`PasteTarget::CopyOnly`]; otherwise look at the front window once more and paste when it is
/// still the target, copy when it is not.
pub fn paste_blocking(injector: &dyn Injector, probe: Option<&dyn ForegroundProbe>, text: &str, target: PasteTarget) -> PasteOutcome {
    let copy = |reason| match injector.copy(text) {
        Ok(()) => PasteOutcome::Copied { reason },
        Err(e) => {
            tracing::warn!(error = %e, "the paste could not copy either");
            PasteOutcome::Failed { reason: PasteFailure::Inject }
        }
    };
    let expected = match target {
        PasteTarget::CopyOnly(reason) => return copy(reason),
        PasteTarget::App(app) => app.sanitized(),
    };
    let now = probe.and_then(|p| {
        p.foreground().unwrap_or_else(|e| {
            tracing::debug!(error = %e, "foreground probe failed before the paste");
            None
        })
    });
    let still_there = expected.zip(now.and_then(ForegroundApp::sanitized)).is_some_and(|(expected, now)| same_target(&expected, &now));
    if !still_there {
        return copy(CopyReason::TargetChanged);
    }
    match injector.inject(text) {
        Ok(Injection { via: Via::Paste, .. }) => PasteOutcome::Pasted,
        Ok(Injection { via: Via::Clipboard, note: None }) => PasteOutcome::Copied { reason: CopyReason::ClipboardOnly },
        Ok(Injection { via: Via::Clipboard, note: Some(note) }) => {
            tracing::info!(%note, "the paste fell back to the clipboard");
            PasteOutcome::Copied { reason: CopyReason::PasteFailed }
        }
        Err(e) => {
            tracing::warn!(error = %e, "the paste failed");
            PasteOutcome::Failed { reason: PasteFailure::Inject }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::dictation::fakes::{FakeInjector, FakeProbe};

    fn app(id: &str, window: Option<u64>) -> ForegroundApp {
        ForegroundApp { app_id: id.into(), name: id.into(), title: None, window }
    }

    #[test]
    fn the_wire_names_the_outcome_by_kind_and_reason() {
        assert_eq!(serde_json::to_string(&PasteOutcome::Pasted).unwrap(), r#"{"kind":"pasted"}"#);
        assert_eq!(
            serde_json::to_string(&PasteOutcome::Copied { reason: CopyReason::TargetChanged }).unwrap(),
            r#"{"kind":"copied","reason":"target_changed"}"#
        );
        assert_eq!(serde_json::to_string(&PasteOutcome::Failed { reason: PasteFailure::Busy }).unwrap(), r#"{"kind":"failed","reason":"busy"}"#);
    }

    #[test]
    fn a_paste_needs_some_text_within_the_limit() {
        assert!(valid_paste_text("你好"));
        assert!(!valid_paste_text("  \n"));
        assert!(valid_paste_text(&"字".repeat(MAX_PASTE_TEXT_CHARS)));
        assert!(!valid_paste_text(&"字".repeat(MAX_PASTE_TEXT_CHARS + 1)));
    }

    #[test]
    fn the_target_is_the_same_app_and_the_same_window_where_both_name_one() {
        assert!(same_target(&app("notepad", Some(7)), &app("notepad", Some(7))));
        assert!(!same_target(&app("notepad", Some(7)), &app("notepad", Some(8))), "another window of the same app");
        assert!(!same_target(&app("notepad", Some(7)), &app("code", Some(7))));
        assert!(same_target(&app("notepad", None), &app("notepad", Some(8))), "one side names no window: the app decides");
    }

    #[test]
    fn copy_only_never_pastes() {
        let injector = FakeInjector::paste();
        let outcome = paste_blocking(&injector, None, "你好", PasteTarget::CopyOnly(CopyReason::NoProbe));
        assert_eq!(outcome, PasteOutcome::Copied { reason: CopyReason::NoProbe });
        assert!(injector.injected().is_empty());
        assert_eq!(injector.clipboard_copies(), vec!["你好".to_owned()]);
    }

    #[test]
    fn the_same_window_gets_the_paste_and_another_one_the_clipboard() {
        let injector = FakeInjector::paste();
        let probe = FakeProbe::app("notepad", "Notepad", None);
        probe.set_window(Some(7));
        let target = PasteTarget::App(app("Notepad.exe", Some(7)));
        assert_eq!(paste_blocking(&injector, Some(&probe), "一", target.clone()), PasteOutcome::Pasted, "ids are compared normalised");
        assert_eq!(injector.injected(), vec!["一".to_owned()]);
        probe.set_window(Some(8));
        assert_eq!(paste_blocking(&injector, Some(&probe), "二", target.clone()), PasteOutcome::Copied { reason: CopyReason::TargetChanged });
        probe.set_nothing();
        assert_eq!(paste_blocking(&injector, Some(&probe), "三", target), PasteOutcome::Copied { reason: CopyReason::TargetChanged });
        assert_eq!(injector.injected(), vec!["一".to_owned()], "nothing was pasted into another window");
        assert_eq!(injector.clipboard_copies(), vec!["二".to_owned(), "三".to_owned()]);
    }

    #[test]
    fn the_insert_setting_and_a_failed_paste_are_told_apart() {
        let probe = FakeProbe::app("notepad", "Notepad", None);
        let target = PasteTarget::App(app("notepad", None));
        let clipboard_only = FakeInjector::clipboard(None);
        assert_eq!(paste_blocking(&clipboard_only, Some(&probe), "x", target.clone()), PasteOutcome::Copied { reason: CopyReason::ClipboardOnly });
        let fell_back = FakeInjector::clipboard(Some("no input connection"));
        assert_eq!(paste_blocking(&fell_back, Some(&probe), "x", target.clone()), PasteOutcome::Copied { reason: CopyReason::PasteFailed });
        let broken = FakeInjector::err("clipboard busy");
        assert_eq!(paste_blocking(&broken, Some(&probe), "x", target), PasteOutcome::Failed { reason: PasteFailure::Inject });
    }
}
