//! Always-on pairing (docs/pairing.md 「常开配对」): with `Settings.pairing_always_on`, a desktop
//! always has a session waiting for a phone. A waiting session is renewed shortly before it
//! lapses; a finished one (trusted, rejected, failed) is followed by the next after a pause that
//! leaves its outcome on screen; a start that finds no relay to open a session on is tried again.
//! Every pairing still needs the safety code confirmed on this desktop.

use std::time::{Duration, Instant};

use voltip_pairing::PairingState;

use super::Runtime;
use crate::CoreError;

/// A waiting session this close to its end is replaced, so a phone never gets a code that dies
/// in its hands.
pub const RENEW_BEFORE_SECS: u64 = 10;
/// How long a finished pairing stays on screen before the next session opens.
pub const FINISHED_PAUSE: Duration = Duration::from_secs(4);
/// How soon a start that found nothing to open a session on is tried again.
pub const RETRY: Duration = Duration::from_secs(5);

impl Runtime {
    /// `SetPairingAlwaysOn`: persist, then open a session now, or close one nobody joined yet (a
    /// pairing under way runs to its end).
    pub(super) async fn set_pairing_always_on(&mut self, enabled: bool) -> Result<(), CoreError> {
        if enabled && !self.config.accepts_phone_takes {
            return Err(CoreError::Invalid("pairing: 常开配对只在电脑上可用".into()));
        }
        self.settings.pairing_always_on = enabled;
        self.save_settings()?;
        self.always_on_at = None;
        if enabled {
            self.keep_pairing_open().await;
        } else if matches!(self.pairing_state(), Some(PairingState::CreatingSession | PairingState::WaitingForPeer)) {
            self.reset_pairing().await?;
        }
        Ok(())
    }

    /// With always-on pairing, keep a session waiting; called on every tick.
    pub(super) async fn keep_pairing_open(&mut self) {
        if !self.settings.pairing_always_on || !self.config.accepts_phone_takes {
            return;
        }
        let wait = match self.pairing_snapshot() {
            // Nothing running: open one.
            None => Duration::ZERO,
            Some(s) => match s.state {
                PairingState::WaitingForPeer if s.remaining_secs.is_some_and(|r| r <= RENEW_BEFORE_SECS) => Duration::ZERO,
                PairingState::Expired => Duration::ZERO,
                PairingState::Trusted | PairingState::Rejected | PairingState::Failed { .. } => FINISHED_PAUSE,
                // Opening, waiting, a phone joining or comparing codes: leave it alone.
                _ => {
                    self.always_on_at = None;
                    return;
                }
            },
        };
        let now = Instant::now();
        let at = *self.always_on_at.get_or_insert(now + wait);
        if now < at {
            return;
        }
        self.always_on_at = None;
        if self.pairing_state().is_some()
            && let Err(e) = self.reset_pairing().await
        {
            tracing::warn!(error = %e, "always-on pairing: reset failed");
        }
        if let Err(e) = self.start_pairing().await {
            tracing::debug!(error = %e, "always-on pairing: nothing to open a session on yet");
            self.always_on_at = Some(now + RETRY);
        }
    }
}
