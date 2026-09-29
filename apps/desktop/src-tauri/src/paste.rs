//! 「粘贴到上一个窗口」 on the home and history pages (`paste_text`, `voltip_core::paste`): the shell
//! gets Voltip out of the way, waits for the window the user came from, and hands the paste to the
//! core, which checks that window once more right before it pastes. The decisions are pure
//! functions ([`first_step`], [`target_after`], [`bring_back`]) so they are tested without a window
//! system; [`wait_for_target`] takes any probe.

use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Manager as _, Runtime};
use voltip_core::DictationPhase;
use voltip_core::dictation::{ForegroundApp, ForegroundProbe};
use voltip_core::paste::{CopyReason, PasteFailure, PasteOutcome, PasteTarget, valid_paste_text};
use voltip_inject::SessionKind;
use voltip_tauri_bridge::Bridge;

/// How often the window in front is asked for while Voltip gets out of the way.
pub const PROBE_EVERY: Duration = Duration::from_millis(50);
/// How long the shell waits for another window to come to the front.
pub const FIND_TARGET_WITHIN: Duration = Duration::from_millis(1500);
/// How long the shell waits for the core's answer.
pub const ANSWER_WITHIN: Duration = Duration::from_secs(5);

/// The foreground probe the core was given (the same `Arc`), managed as Tauri state; `None` on
/// the headless wiring.
pub struct PasteProbe(pub Option<Arc<dyn ForegroundProbe>>);

/// What the shell does before it moves any window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FirstStep {
    /// Answer now; nothing is pasted or copied.
    Refuse(PasteFailure),
    /// Copy only, right away: the session cannot name the window in front (pure Wayland).
    CopyOnly(CopyReason),
    /// Move Voltip out of the way and look for the window in front.
    FindTarget,
}

/// The first decision: empty or too long text is refused, a running take makes it busy (no
/// queueing), a pure Wayland session only copies, anything else looks for the target.
pub fn first_step(text: &str, take_running: bool, session: Option<SessionKind>) -> FirstStep {
    if !valid_paste_text(text) {
        FirstStep::Refuse(PasteFailure::Invalid)
    } else if take_running {
        FirstStep::Refuse(PasteFailure::Busy)
    } else if session == Some(SessionKind::Wayland) {
        FirstStep::CopyOnly(CopyReason::NoProbe)
    } else {
        FirstStep::FindTarget
    }
}

/// A take is under way (a finished one still on screen does not count).
pub fn take_running(phase: &DictationPhase) -> bool {
    !matches!(phase, DictationPhase::Idle) && !phase.is_terminal()
}

/// The paste's target once the wait is over: the window found, or copy only.
pub fn target_after(found: Option<ForegroundApp>) -> PasteTarget {
    found.map_or(PasteTarget::CopyOnly(CopyReason::Timeout), PasteTarget::App)
}

/// Voltip comes back unless the text went into the other window.
pub fn bring_back(outcome: PasteOutcome) -> bool {
    outcome != PasteOutcome::Pasted
}

/// Ask `probe` every `every` until it names a window, for at most `within`. The probes answer
/// `None` while one of Voltip's own windows is in front, so the first answer is another
/// application's window.
pub async fn wait_for_target(probe: Arc<dyn ForegroundProbe>, every: Duration, within: Duration) -> Option<ForegroundApp> {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        let asked = probe.clone();
        match tokio::task::spawn_blocking(move || asked.foreground()).await {
            Ok(Ok(Some(app))) => return Some(app),
            Ok(Ok(None)) => {}
            Ok(Err(e)) => tracing::debug!(error = %e, "foreground probe failed while waiting for the paste target"),
            Err(e) => {
                tracing::warn!(error = %e, "foreground probe task failed");
                return None;
            }
        }
        if tokio::time::Instant::now() + every > deadline {
            return None;
        }
        tokio::time::sleep(every).await;
    }
}

/// Get Voltip out of the way so the window the user came from comes back to the front: the main
/// window is minimised, and on macOS the whole application is hidden (a minimised window leaves
/// the application active there).
fn step_aside<R: Runtime>(app: &AppHandle<R>) {
    #[cfg(target_os = "macos")]
    if let Err(e) = app.hide() {
        tracing::warn!(error = %e, "hiding the application for the paste failed");
    }
    #[cfg(not(target_os = "macos"))]
    if let Some(window) = app.get_webview_window(crate::MAIN_WINDOW)
        && let Err(e) = window.minimize()
    {
        tracing::warn!(error = %e, "minimising the main window for the paste failed");
    }
}

/// Bring Voltip back to the front after a paste that did not go into the other window.
fn come_back<R: Runtime>(app: &AppHandle<R>) {
    #[cfg(target_os = "macos")]
    if let Err(e) = app.show() {
        tracing::warn!(error = %e, "showing the application after the paste failed");
    }
    crate::show_main_window(app);
}

/// The `paste_text` command from start to end.
pub async fn paste_text<R: Runtime>(app: &AppHandle<R>, text: String) -> PasteOutcome {
    let Some(bridge) = app.try_state::<Bridge>().map(|b| b.inner().clone()) else {
        return PasteOutcome::Failed { reason: PasteFailure::Timeout };
    };
    let running = take_running(&bridge.state().dictation.phase);
    let probe = app.try_state::<PasteProbe>().and_then(|p| p.0.clone());
    let (target, moved) = match (first_step(&text, running, crate::hotkey::linux_session().map(|s| s.kind)), probe) {
        (FirstStep::Refuse(reason), _) => return PasteOutcome::Failed { reason },
        (FirstStep::CopyOnly(reason), _) => (PasteTarget::CopyOnly(reason), false),
        (FirstStep::FindTarget, None) => (PasteTarget::CopyOnly(CopyReason::NoProbe), false),
        (FirstStep::FindTarget, Some(probe)) => {
            step_aside(app);
            (target_after(wait_for_target(probe, PROBE_EVERY, FIND_TARGET_WITHIN).await), true)
        }
    };
    let outcome = bridge.paste(text, target, ANSWER_WITHIN).await;
    if moved && bring_back(outcome) {
        come_back(app);
    }
    outcome
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use voltip_core::paste::MAX_PASTE_TEXT_CHARS;

    use super::*;

    fn app(id: &str) -> ForegroundApp {
        ForegroundApp { app_id: id.into(), name: id.into(), title: None, window: Some(7) }
    }

    #[test]
    fn the_first_step_refuses_copies_or_looks_for_the_target() {
        let too_long = "字".repeat(MAX_PASTE_TEXT_CHARS + 1);
        assert_eq!(first_step(" \n", false, None), FirstStep::Refuse(PasteFailure::Invalid));
        assert_eq!(first_step(&too_long, false, None), FirstStep::Refuse(PasteFailure::Invalid));
        assert_eq!(first_step("你好", true, None), FirstStep::Refuse(PasteFailure::Busy), "a running take: refused, not queued");
        assert_eq!(first_step("你好", false, Some(SessionKind::Wayland)), FirstStep::CopyOnly(CopyReason::NoProbe), "pure Wayland: no 1.5 s wait");
        assert_eq!(first_step("你好", false, Some(SessionKind::XWayland)), FirstStep::FindTarget);
        assert_eq!(first_step("你好", false, Some(SessionKind::X11)), FirstStep::FindTarget);
        assert_eq!(first_step("你好", false, None), FirstStep::FindTarget, "Windows and macOS");
    }

    #[test]
    fn only_a_take_under_way_counts_as_running() {
        assert!(!take_running(&DictationPhase::Idle));
        assert!(!take_running(&DictationPhase::CANCELLED), "a finished take still on screen");
        assert!(take_running(&DictationPhase::Listening { started_at: 1, ready: true, live: None, locked: false }));
    }

    #[test]
    fn the_wait_ends_in_a_target_or_copy_only_and_voltip_comes_back_unless_pasted() {
        assert_eq!(target_after(Some(app("notepad"))), PasteTarget::App(app("notepad")));
        assert_eq!(target_after(None), PasteTarget::CopyOnly(CopyReason::Timeout));
        assert!(!bring_back(PasteOutcome::Pasted));
        assert!(bring_back(PasteOutcome::Copied { reason: CopyReason::TargetChanged }));
        assert!(bring_back(PasteOutcome::Failed { reason: PasteFailure::Inject }));
    }

    /// Answers "Voltip is in front" (`None`) for the first `hidden` calls, then `answer`.
    struct SlowProbe {
        hidden: usize,
        calls: AtomicUsize,
        answer: Result<Option<ForegroundApp>, String>,
    }

    impl ForegroundProbe for SlowProbe {
        fn foreground(&self) -> Result<Option<ForegroundApp>, String> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if call < self.hidden {
                return Ok(None);
            }
            self.answer.clone()
        }
    }

    fn probe(hidden: usize, answer: Result<Option<ForegroundApp>, String>) -> Arc<SlowProbe> {
        Arc::new(SlowProbe { hidden, calls: AtomicUsize::new(0), answer })
    }

    #[tokio::test]
    async fn the_wait_takes_the_first_window_that_comes_up_and_gives_up_in_time() {
        let every = Duration::from_millis(5);
        let within = Duration::from_millis(200);
        let found = probe(3, Ok(Some(app("notepad"))));
        assert_eq!(wait_for_target(found.clone(), every, within).await, Some(app("notepad")));
        assert_eq!(found.calls.load(Ordering::SeqCst), 4, "asked until the window came up, not after");
        // Voltip stays in front, or the probe keeps failing: nothing in time.
        let never = probe(usize::MAX, Ok(None));
        assert_eq!(wait_for_target(never.clone(), every, within).await, None);
        assert!(never.calls.load(Ordering::SeqCst) >= 2, "asked more than once");
        assert_eq!(wait_for_target(probe(0, Err("no display".into())), every, within).await, None);
    }
}
