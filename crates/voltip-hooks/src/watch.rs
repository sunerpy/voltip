//! What keeps the lone-key hook alive (docs/dictation.md §29), as pure rules every host tests.
//!
//! - [`HookWatch`] (Windows): the system removes a low-level hook it finds slow without telling
//!   anyone; Raw Input on the hook thread's own window keeps arriving. Presses Raw Input sees that
//!   no hook callback saw prove the hook gone. A hook put back that still hears nothing (another
//!   program's hook ahead of it keeps the input) is put back again less and less often.
//! - [`recovery_for_message`] (Windows): waking from sleep and unlocking the session put the hook
//!   back as well, whatever its state.
//! - [`retry_after`] (macOS, X11): how long to wait before the next attempt to bring a hook back
//!   that could not be put back at once.

use std::time::{Duration, Instant};

/// Presses Raw Input reports without a hook callback before the hook counts as removed…
pub const LOST_AFTER_PRESSES: u32 = 3;
/// …once the first of them is at least this old. No presses, no judgement: a hook nobody presses a
/// key for is never taken for a removed one.
pub const LOST_AFTER: Duration = Duration::from_secs(1);
/// The longest wait between two attempts to bring a hook back ([`retry_after`]).
pub const RETRY_CAP: Duration = Duration::from_secs(60);

/// Why a hook was put back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recovery {
    /// [`HookWatch`] found presses the hook never saw: the system removed it.
    Removed,
    /// The computer woke from sleep.
    Resumed,
    /// The session was unlocked.
    Unlocked,
    /// The system stopped the hook (a disabled event tap, a lost X connection) and it was
    /// installed again.
    Reinstalled,
}

impl Recovery {
    /// The log's name for it.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Removed => "removed",
            Self::Resumed => "resumed",
            Self::Unlocked => "unlocked",
            Self::Reinstalled => "reinstalled",
        }
    }
}

/// Whether the low-level hook still sees the input Raw Input sees (Windows). Fed from the hook
/// thread: [`HookWatch::on_hook`] for every hook callback, [`HookWatch::on_raw`] for every key or
/// button press Raw Input reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HookWatch {
    /// Injected input (no device) counts too: the tests' `SendInput`.
    synthetic: bool,
    /// Presses since the last hook callback.
    presses: u32,
    /// When the first of them came.
    first: Option<Instant>,
    /// Verdicts no hook callback has answered since, and when the last of them came: a hook put
    /// back that still hears nothing is put back again after [`retry_after`], not at every third
    /// press.
    misses: u32,
    last: Option<Instant>,
}

impl HookWatch {
    /// A watch that has seen nothing; `synthetic` counts injected presses as the tests need.
    pub const fn new(synthetic: bool) -> Self {
        Self { synthetic, presses: 0, first: None, misses: 0, last: None }
    }

    /// A hook callback came: the hook is there, and neither the presses before it nor the verdicts
    /// before them say anything.
    pub fn on_hook(&mut self) {
        *self = Self::new(self.synthetic);
    }

    /// The hook was installed again (a verdict, a wake, an unlock): the presses so far say nothing
    /// about the new one. The verdicts it has not answered still count.
    pub fn on_reinstall(&mut self) {
        self.presses = 0;
        self.first = None;
    }

    /// Raw Input reported a press at `now`; `from_device` is false for injected input (no
    /// device handle). Whether the hook is to be put back: [`LOST_AFTER_PRESSES`] presses no hook
    /// callback answered, the first of them [`LOST_AFTER`] old, and, when an earlier verdict has
    /// gone unanswered, [`retry_after`] since it. The verdict starts the count again.
    pub fn on_raw(&mut self, now: Instant, from_device: bool) -> Option<Recovery> {
        if !from_device && !self.synthetic {
            return None;
        }
        let first = *self.first.get_or_insert(now);
        self.presses = self.presses.saturating_add(1);
        if self.presses < LOST_AFTER_PRESSES || now.saturating_duration_since(first) < LOST_AFTER {
            return None;
        }
        if let Some(last) = self.last
            && now.saturating_duration_since(last) < retry_after(self.misses)
        {
            return None;
        }
        self.on_reinstall();
        self.misses = self.misses.saturating_add(1);
        self.last = Some(now);
        Some(Recovery::Removed)
    }
}

/// `WM_POWERBROADCAST` (`winuser.h`).
pub const WM_POWERBROADCAST: u32 = 0x0218;
/// `PBT_APMRESUMESUSPEND`: resumed after a suspend the user started.
pub const PBT_APMRESUMESUSPEND: usize = 0x0007;
/// `PBT_APMRESUMEAUTOMATIC`: resumed, for any reason.
pub const PBT_APMRESUMEAUTOMATIC: usize = 0x0012;
/// `WM_WTSSESSION_CHANGE` (`winuser.h`).
pub const WM_WTSSESSION_CHANGE: u32 = 0x02B1;
/// `WTS_SESSION_UNLOCK` (`wtsapi32.h`).
pub const WTS_SESSION_UNLOCK: usize = 0x0008;

/// What a message to the hook thread's window means for the hook (Windows): waking from sleep
/// and unlocking the session put it back; every other message, nothing.
pub fn recovery_for_message(message: u32, wparam: usize) -> Option<Recovery> {
    match (message, wparam) {
        (WM_POWERBROADCAST, PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND) => Some(Recovery::Resumed),
        (WM_WTSSESSION_CHANGE, WTS_SESSION_UNLOCK) => Some(Recovery::Unlocked),
        _ => None,
    }
}

/// How long to wait before attempt `attempt` (0 = the first retry) to bring a hook back:
/// 1, 2, 4, … seconds, at most [`RETRY_CAP`].
pub fn retry_after(attempt: u32) -> Duration {
    Duration::from_secs(1u64.checked_shl(attempt).unwrap_or(u64::MAX)).min(RETRY_CAP)
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Duration = Duration::from_secs(1);
    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn no_presses_no_judgement() {
        let t0 = Instant::now();
        let mut watch = HookWatch::new(false);
        // Hours without a key: nothing is decided, ever.
        watch.on_hook();
        assert_eq!(watch, HookWatch::new(false));
        // Two presses the hook missed, however old: not enough.
        assert_eq!(watch.on_raw(t0, true), None);
        assert_eq!(watch.on_raw(t0 + 3600 * S, true), None);
    }

    #[test]
    fn three_presses_the_hook_missed_over_a_second_mean_it_is_gone() {
        let t0 = Instant::now();
        let mut watch = HookWatch::new(false);
        assert_eq!(watch.on_raw(t0, true), None);
        assert_eq!(watch.on_raw(t0 + 200 * MS, true), None);
        // The third press, but the first one is not a second old yet.
        assert_eq!(watch.on_raw(t0 + 999 * MS, true), None);
        // A fourth, a second after the first: the hook is gone.
        assert_eq!(watch.on_raw(t0 + 1000 * MS, true), Some(Recovery::Removed));
        // The verdict starts the count again; the hook put back calls back.
        assert_eq!(watch.on_raw(t0 + 1100 * MS, true), None);
        assert_eq!(watch.on_raw(t0 + 1200 * MS, true), None);
        watch.on_hook();
        assert_eq!(watch.on_raw(t0 + 1300 * MS, true), None);
        assert_eq!(watch.on_raw(t0 + 1400 * MS, true), None);
        assert_eq!(watch.on_raw(t0 + 2300 * MS, true), Some(Recovery::Removed));
    }

    #[test]
    fn a_hook_that_still_hears_nothing_is_put_back_less_and_less_often() {
        // Raw Input sees presses a hook put back still does not (another program's hook ahead of
        // it, a window it may not hear): the verdicts back off as `retry_after` does, from the
        // second on, until a hook callback comes.
        let t0 = Instant::now();
        let mut watch = HookWatch::new(false);
        let mut verdicts = Vec::new();
        for i in 0..400 {
            let at = t0 + i * 250 * MS;
            if watch.on_raw(at, true).is_some() {
                verdicts.push(at.duration_since(t0).as_millis());
            }
        }
        // A press every quarter second for 100 s: the first verdict a second in, each later one
        // 2, 4, 8, … s (at most a minute) after the one before.
        assert_eq!(verdicts, [1000, 3000, 7000, 15_000, 31_000, 63_000]);
        // A hook callback: the hook hears again, and the next removal is judged at once.
        watch.on_hook();
        assert_eq!(watch.on_raw(t0 + 100 * S, true), None);
        assert_eq!(watch.on_raw(t0 + 100 * S + 500 * MS, true), None);
        assert_eq!(watch.on_raw(t0 + 101 * S, true), Some(Recovery::Removed));
    }

    #[test]
    fn a_hook_put_back_starts_its_count_afresh() {
        let t0 = Instant::now();
        let mut watch = HookWatch::new(false);
        assert_eq!(watch.on_raw(t0, true), None);
        assert_eq!(watch.on_raw(t0 + 900 * MS, true), None);
        // Woken from sleep, the hook installed again: the two presses before say nothing about it.
        watch.on_reinstall();
        assert_eq!(watch.on_raw(t0 + 1000 * MS, true), None);
        assert_eq!(watch.on_raw(t0 + 1100 * MS, true), None);
        assert_eq!(watch.on_raw(t0 + 1900 * MS, true), None);
        assert_eq!(watch.on_raw(t0 + 2000 * MS, true), Some(Recovery::Removed));
        // A reinstall does not forget the verdicts no callback answered: the backoff stays.
        watch.on_reinstall();
        assert_eq!(watch.on_raw(t0 + 2100 * MS, true), None);
        assert_eq!(watch.on_raw(t0 + 2200 * MS, true), None);
        assert_eq!(watch.on_raw(t0 + 3100 * MS, true), None, "the second verdict waits 2 s after the first");
        assert_eq!(watch.on_raw(t0 + 4000 * MS, true), Some(Recovery::Removed));
    }

    #[test]
    fn a_hook_callback_clears_the_count() {
        let t0 = Instant::now();
        let mut watch = HookWatch::new(false);
        assert_eq!(watch.on_raw(t0, true), None);
        assert_eq!(watch.on_raw(t0 + 600 * MS, true), None);
        watch.on_hook();
        // The hook saw a key: the presses before it say nothing, the count starts at the next.
        assert_eq!(watch.on_raw(t0 + 1500 * MS, true), None);
        assert_eq!(watch.on_raw(t0 + 1600 * MS, true), None);
        assert_eq!(watch.on_raw(t0 + 2000 * MS, true), None, "the first counted press is 0.5 s old");
        assert_eq!(watch.on_raw(t0 + 2500 * MS, true), Some(Recovery::Removed));
        // A live hook calls back for every key: every press is preceded by its callback.
        let mut alive = HookWatch::new(false);
        for i in 0..100 {
            alive.on_hook();
            assert_eq!(alive.on_raw(t0 + i * 300 * MS, true), None);
        }
    }

    #[test]
    fn injected_presses_count_only_in_synthetic_mode() {
        let t0 = Instant::now();
        let mut real = HookWatch::new(false);
        for i in 0..10 {
            // Voltip's own paste, another program's SendInput: no device, never a verdict.
            assert_eq!(real.on_raw(t0 + i * 500 * MS, false), None);
        }
        let mut tests = HookWatch::new(true);
        assert_eq!(tests.on_raw(t0, false), None);
        assert_eq!(tests.on_raw(t0 + 500 * MS, false), None);
        assert_eq!(tests.on_raw(t0 + 1000 * MS, false), Some(Recovery::Removed));
    }

    #[test]
    fn waking_and_unlocking_put_the_hook_back() {
        assert_eq!(recovery_for_message(WM_POWERBROADCAST, PBT_APMRESUMEAUTOMATIC), Some(Recovery::Resumed));
        assert_eq!(recovery_for_message(WM_POWERBROADCAST, PBT_APMRESUMESUSPEND), Some(Recovery::Resumed));
        // PBT_APMSUSPEND (going to sleep) and PBT_POWERSETTINGCHANGE are not a wake.
        assert_eq!(recovery_for_message(WM_POWERBROADCAST, 0x0004), None);
        assert_eq!(recovery_for_message(WM_POWERBROADCAST, 0x8013), None);
        assert_eq!(recovery_for_message(WM_WTSSESSION_CHANGE, WTS_SESSION_UNLOCK), Some(Recovery::Unlocked));
        // WTS_SESSION_LOCK, WTS_CONSOLE_CONNECT: nothing.
        assert_eq!(recovery_for_message(WM_WTSSESSION_CHANGE, 0x0007), None);
        assert_eq!(recovery_for_message(WM_WTSSESSION_CHANGE, 0x0001), None);
        // Another message with the same parameter.
        assert_eq!(recovery_for_message(0x0100, PBT_APMRESUMEAUTOMATIC), None);
        assert_eq!(Recovery::Unlocked.as_str(), "unlocked");
    }

    #[test]
    fn retries_back_off_to_a_minute() {
        let waits: Vec<u64> = (0..10).map(|n| retry_after(n).as_secs()).collect();
        assert_eq!(waits, [1, 2, 4, 8, 16, 32, 60, 60, 60, 60]);
        assert_eq!(retry_after(u32::MAX), RETRY_CAP);
    }
}
