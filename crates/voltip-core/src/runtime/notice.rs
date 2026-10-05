//! The built-in clean-up service's notice (docs/dictation.md §3.6): a take whose clean-up the
//! built-in service turned down for want of capacity (too many requests, or its quota used up)
//! keeps the raw text, and the interface suggests a provider of the user's own. The notice goes
//! once a take is cleaned up again, once the clean-up uses another provider, or when the user
//! closes it; closed, it stays away for [`REFINE_NOTICE_SNOOZE_MS`].

use super::{CoreEvent, Runtime, now_ms};
use crate::dictation::DictationPhase;
use crate::providers::ProviderId;
use crate::ui::RefineNotice;

/// How long a closed notice stays away (a day: the service's daily quota has moved on by then).
pub const REFINE_NOTICE_SNOOZE_MS: u64 = 24 * 60 * 60 * 1000;

/// The notice's part of the runtime: what the interface shows, and when the user last closed it
/// (in memory: a restart may show it again).
#[derive(Default)]
pub(super) struct NoticeRuntime {
    refine: Option<RefineNotice>,
    refine_closed_ms: Option<u64>,
}

/// What a take that ended in `phase` does to the notice `current`, with the clean-up on `provider`
/// at `now_ms`: `Some(next)` to change it, `None` to leave it.
pub(super) fn after_take(
    current: Option<RefineNotice>,
    closed_ms: Option<u64>,
    phase: &DictationPhase,
    provider: ProviderId,
    now_ms: u64,
) -> Option<Option<RefineNotice>> {
    let DictationPhase::Done { refined, refine_failure, .. } = phase else { return None };
    if *refined {
        return current.is_some().then_some(None);
    }
    let failure = refine_failure.filter(|f| f.is_capacity())?;
    let snoozed = closed_ms.is_some_and(|at| now_ms.saturating_sub(at) < REFINE_NOTICE_SNOOZE_MS);
    if provider != ProviderId::Builtin || snoozed || current.is_some_and(|c| c.failure == failure) {
        return None;
    }
    Some(Some(RefineNotice { failure, at_ms: now_ms }))
}

impl Runtime {
    /// A take ended in `phase` (every dictation status passes through).
    pub(super) fn follow_refine_notice(&mut self, phase: &DictationPhase) {
        if let Some(next) = after_take(self.notice.refine, self.notice.refine_closed_ms, phase, self.settings.engines.llm_provider, now_ms()) {
            self.set_refine_notice(next);
        }
    }

    /// The engines changed: a clean-up off the built-in service has no notice.
    pub(super) fn check_refine_notice_provider(&mut self) {
        if self.settings.engines.llm_provider != ProviderId::Builtin {
            self.set_refine_notice(None);
        }
    }

    /// `RefineNoticeClose`: gone, and away for [`REFINE_NOTICE_SNOOZE_MS`].
    pub(super) fn close_refine_notice(&mut self) {
        self.notice.refine_closed_ms = Some(now_ms());
        self.set_refine_notice(None);
    }

    fn set_refine_notice(&mut self, next: Option<RefineNotice>) {
        if self.notice.refine != next {
            self.notice.refine = next;
            self.emit(CoreEvent::RefineNotice(next));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dictation::{OutputMode, RefineFailure, Via};

    fn done(refined: bool, refine_failure: Option<RefineFailure>) -> DictationPhase {
        DictationPhase::Done {
            text: "原文".into(),
            raw_text: "原文".into(),
            chars: 2,
            via: Via::Paste,
            refined,
            duration_ms: 1,
            asr_ms: 1,
            refine_ms: None,
            refine_error: refine_failure.map(|_| "x".into()),
            refine_failure,
            mode: OutputMode::WholeTake,
            segments: None,
            live_error: None,
        }
    }

    const NOW: u64 = 10 * REFINE_NOTICE_SNOOZE_MS;

    fn notice(failure: RefineFailure, at_ms: u64) -> Option<RefineNotice> {
        Some(RefineNotice { failure, at_ms })
    }

    #[test]
    fn a_take_the_built_in_service_turned_down_raises_the_notice() {
        let busy = done(false, Some(RefineFailure::RateLimited));
        assert_eq!(after_take(None, None, &busy, ProviderId::Builtin, NOW), Some(notice(RefineFailure::RateLimited, NOW)));
        let used_up = done(false, Some(RefineFailure::Quota));
        assert_eq!(after_take(None, None, &used_up, ProviderId::Builtin, NOW), Some(notice(RefineFailure::Quota, NOW)));
        // A quota after a rate limit is news; the same kind again changes nothing.
        assert_eq!(after_take(notice(RefineFailure::RateLimited, 1), None, &used_up, ProviderId::Builtin, NOW), Some(notice(RefineFailure::Quota, NOW)));
        assert_eq!(after_take(notice(RefineFailure::RateLimited, 1), None, &busy, ProviderId::Builtin, NOW), None);
    }

    #[test]
    fn only_the_built_in_services_capacity_raises_it() {
        let busy = done(false, Some(RefineFailure::RateLimited));
        assert_eq!(after_take(None, None, &busy, ProviderId::Groq, NOW), None, "the user's own provider: their own quota");
        for other in [RefineFailure::Failed, RefineFailure::Empty, RefineFailure::Unconfigured, RefineFailure::TooLong] {
            assert_eq!(after_take(None, None, &done(false, Some(other)), ProviderId::Builtin, NOW), None, "{other:?}");
        }
        assert_eq!(after_take(None, None, &done(false, None), ProviderId::Builtin, NOW), None, "no clean-up was asked for");
        assert_eq!(after_take(None, None, &DictationPhase::Idle, ProviderId::Builtin, NOW), None);
    }

    #[test]
    fn a_cleaned_up_take_clears_it_and_a_closed_one_stays_away_a_day() {
        let shown = notice(RefineFailure::RateLimited, 1);
        assert_eq!(after_take(shown, None, &done(true, None), ProviderId::Builtin, NOW), Some(None));
        assert_eq!(after_take(None, None, &done(true, None), ProviderId::Builtin, NOW), None, "nothing to clear");
        let busy = done(false, Some(RefineFailure::RateLimited));
        let closed = NOW - REFINE_NOTICE_SNOOZE_MS + 1;
        assert_eq!(after_take(None, Some(closed), &busy, ProviderId::Builtin, NOW), None, "closed less than a day ago");
        let closed = NOW - REFINE_NOTICE_SNOOZE_MS;
        assert_eq!(after_take(None, Some(closed), &busy, ProviderId::Builtin, NOW), Some(notice(RefineFailure::RateLimited, NOW)));
    }
}
