//! The built-in service's usage (docs/dictation.md §27): the metering every client is built with,
//! this device's record of what it used, and the service's account of today — from the reports
//! its answers carry, and from asking it ([`UsageProbe`]) after start, every
//! [`USAGE_PROBE_INTERVAL`], when asked to refresh, after the day's reset and after a network
//! change. A failed question is tried again every [`USAGE_RETRY`]; one minute without an answer
//! shows the numbers as 无法获取, kept as they were. Nothing here stops a take.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use super::{CoreEvent, Runtime, now_ms};
use crate::providers::ProviderId;
use crate::usage::{Metering, QuotaReport, UsageLedger, UsageNote, UsageNotice, UsageOrigin, UsageProbe, UsageView, beijing_day, next_beijing_midnight_ms};

/// How often the service is asked while nothing else reports.
pub const USAGE_PROBE_INTERVAL: Duration = Duration::from_secs(10 * 60);
/// How soon a failed question is asked again.
pub const USAGE_RETRY: Duration = Duration::from_secs(60);
/// How long without an answer before the numbers show as 无法获取.
pub const USAGE_UNAVAILABLE_AFTER: Duration = Duration::from_secs(60);
/// The usage notes queued for the core before the newest are dropped.
const USAGE_QUEUE: usize = 256;

fn ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// What reaches the core about usage.
pub(super) enum UsageInbound {
    /// From a client: what a request used, or the service's report.
    Note(UsageNote),
    /// The answer to a question ([`UsageProbe::quota`]).
    Probed(Result<QuotaReport, String>),
}

/// What one tick of [`UsageRuntime::tick`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct UsageTick {
    /// The view changed (it became 无法获取).
    pub(super) changed: bool,
    /// A question is due.
    pub(super) ask: bool,
}

/// The usage part of the runtime.
pub(super) struct UsageRuntime {
    pub(super) metering: Metering,
    ledger: UsageLedger,
    quota: Option<QuotaReport>,
    reported_at: Option<u64>,
    /// Questions have failed since then, with no report in between.
    failing_since: Option<u64>,
    unavailable: bool,
    next_probe_ms: u64,
    probing: bool,
    tx: mpsc::Sender<UsageInbound>,
}

impl UsageRuntime {
    /// The metering of `device` as `client`, its record in `dir`; the notes come back on the
    /// returned receiver.
    pub(super) fn new(dir: &std::path::Path, client: String) -> (Self, mpsc::Receiver<UsageInbound>) {
        let device = crate::usage::device_id(dir).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "the usage id could not be saved; using one for this run");
            uuid::Uuid::new_v4().to_string()
        });
        let (tx, rx) = mpsc::channel(USAGE_QUEUE);
        let sink = tx.clone();
        let metering = Metering::new(device, client, UsageOrigin::ThisDevice, move |note| {
            if sink.try_send(UsageInbound::Note(note)).is_err() {
                tracing::debug!("usage note dropped: the core is behind");
            }
        });
        let runtime = Self {
            metering,
            ledger: UsageLedger::open(dir),
            quota: None,
            reported_at: None,
            failing_since: None,
            unavailable: false,
            next_probe_ms: 0,
            probing: false,
            tx,
        };
        (runtime, rx)
    }

    /// Ask the service at the next tick.
    pub(super) fn probe_soon(&mut self) {
        self.next_probe_ms = 0;
    }

    fn reported(&mut self, report: QuotaReport, now: u64) {
        // The next question after the reset at the latest, else in the usual interval.
        let reset = report.reset_at.saturating_mul(1000).saturating_add(5_000);
        self.next_probe_ms = now.saturating_add(ms(USAGE_PROBE_INTERVAL)).min(reset.max(now.saturating_add(5_000)));
        self.quota = Some(report);
        self.reported_at = Some(now);
        self.failing_since = None;
        self.unavailable = false;
    }

    /// What reached the core at `now`.
    pub(super) fn receive(&mut self, inbound: UsageInbound, now: u64) {
        match inbound {
            UsageInbound::Note(UsageNote::Used(event)) => self.ledger.record(&event),
            UsageInbound::Note(UsageNote::Quota(report)) => self.reported(report, now),
            UsageInbound::Probed(result) => {
                self.probing = false;
                match result {
                    Ok(report) => self.reported(report, now),
                    Err(e) => {
                        tracing::debug!(error = %e, "the built-in service's usage could not be asked");
                        self.failing_since.get_or_insert(now);
                        self.next_probe_ms = now.saturating_add(ms(USAGE_RETRY));
                    }
                }
            }
        }
    }

    /// A tick at `now`: the numbers become 无法获取 a minute into the service's silence, and a
    /// question is due when none is out and its time has come.
    pub(super) fn tick(&mut self, now: u64) -> UsageTick {
        let mut changed = false;
        if !self.unavailable && self.failing_since.is_some_and(|since| now.saturating_sub(since) >= ms(USAGE_UNAVAILABLE_AFTER)) {
            self.unavailable = true;
            changed = true;
        }
        UsageTick { changed, ask: !self.probing && now >= self.next_probe_ms }
    }

    /// A question went out; the next is due after its answer.
    pub(super) fn asking(&mut self) {
        self.probing = true;
    }

    /// `UiState.usage` at `now`: a report of a day that ended is today's with nothing used yet.
    pub(super) fn view(&self, builtin: bool, now: u64) -> UsageView {
        let quota = self.quota.clone().map(|mut q| {
            if now >= q.reset_at.saturating_mul(1000) {
                q.asr.used_seconds = 0;
                q.refine.used = 0;
                q.day = beijing_day(now);
                q.reset_at = next_beijing_midnight_ms(now) / 1000;
            }
            q
        });
        UsageView { builtin, quota, reported_at: self.reported_at, unavailable: self.unavailable, record: self.ledger.view(now) }
    }
}

impl Runtime {
    /// The build has the built-in service: the usage card is there.
    fn usage_builtin(&self) -> bool {
        self.built_in.asr_url.is_some() || self.built_in.refine_url.is_some()
    }

    pub(super) fn emit_usage(&mut self) {
        let view = self.usage.view(self.usage_builtin(), now_ms());
        self.emit(CoreEvent::Usage(view));
    }

    pub(super) fn on_usage(&mut self, inbound: UsageInbound) {
        self.usage.receive(inbound, now_ms());
        self.emit_usage();
    }

    /// `UsageNoticeDismiss`: closed for the rest of the day.
    pub(super) fn dismiss_usage_notice(&mut self, notice: UsageNotice) {
        self.usage.ledger.dismiss(notice, now_ms());
        self.emit_usage();
    }

    /// Once per tick: ask the service when due, and say 无法获取 a minute into its silence.
    pub(super) fn usage_tick(&mut self) {
        let tick = self.usage.tick(now_ms());
        if tick.changed {
            self.emit_usage();
        }
        if !tick.ask {
            return;
        }
        let Some((probe, base, token)) = self.usage_question() else { return };
        self.usage.asking();
        let (headers, tx) = (self.usage.metering.headers(), self.usage.tx.clone());
        tokio::spawn(async move {
            let result = probe.quota(base, token, &headers).await;
            let _ = tx.send(UsageInbound::Probed(result)).await;
        });
    }

    /// Whom to ask and where: the built-in recognition's endpoint when it recognises, else the
    /// built-in clean-up's; nothing when neither is in use or the shell cannot ask.
    fn usage_question(&self) -> Option<(Arc<dyn UsageProbe>, &'static str, Option<&'static str>)> {
        let probe = self.config.usage_probe.clone()?;
        let engines = &self.settings.engines;
        let asr = (engines.asr_provider == ProviderId::Builtin).then_some(self.built_in.asr_url).flatten().map(|url| (url, self.built_in.asr_token));
        let llm = (engines.llm_provider == ProviderId::Builtin).then_some(self.built_in.refine_url).flatten().map(|url| (url, self.built_in.refine_api_key));
        let (base, token) = asr.or(llm)?;
        Some((probe, base, token))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::providers::ServiceKind;
    use crate::usage::{AsrQuota, QuotaBasis, QuotaMode, RefineQuota, UsageEvent, UsagePurpose};

    /// 2026-10-27 10:00 Beijing time.
    const NOW: u64 = 1_793_066_400_000;
    /// The next Beijing midnight, Unix seconds.
    const RESET: u64 = 1_793_116_800;

    fn report(reset_at: u64, asr_used: u64) -> QuotaReport {
        QuotaReport {
            mode: QuotaMode::Count,
            basis: QuotaBasis::Device,
            day: beijing_day(NOW),
            reset_at,
            asr: AsrQuota { used_seconds: asr_used, limit_seconds: Some(3600) },
            refine: RefineQuota { used: 3, limit: Some(100) },
        }
    }

    fn runtime() -> (UsageRuntime, mpsc::Receiver<UsageInbound>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let (usage, rx) = UsageRuntime::new(dir.path(), "voltip/test (linux)".into());
        (usage, rx, dir)
    }

    #[test]
    fn the_first_tick_asks_and_no_second_question_goes_out_while_one_is_out() {
        let (mut usage, _rx, _dir) = runtime();
        assert_eq!(usage.tick(NOW), UsageTick { changed: false, ask: true });
        usage.asking();
        assert!(!usage.tick(NOW + 120_000).ask, "the answer is awaited");
        usage.receive(UsageInbound::Probed(Ok(report(RESET, 95))), NOW + 1_000);
        assert!(!usage.tick(NOW + 2_000).ask);
        usage.probe_soon();
        assert!(usage.tick(NOW + 2_000).ask, "usage_refresh, a network change");
    }

    /// docs/dictation.md §27: a failed question is asked again every minute; a minute without an
    /// answer shows the numbers as 无法获取, kept as they were, until a report arrives.
    #[test]
    fn a_minute_of_failed_questions_shows_the_last_numbers_as_unavailable() {
        let (mut usage, _rx, _dir) = runtime();
        usage.receive(UsageInbound::Probed(Ok(report(RESET, 95))), NOW);
        usage.probe_soon();
        assert!(usage.tick(NOW + 1_000).ask);
        usage.asking();
        usage.receive(UsageInbound::Probed(Err("offline".into())), NOW + 2_000);
        assert_eq!(usage.tick(NOW + 30_000), UsageTick { changed: false, ask: false });
        assert!(!usage.view(true, NOW + 30_000).unavailable, "not yet a minute");
        assert_eq!(usage.tick(NOW + 62_000), UsageTick { changed: true, ask: true }, "a minute later: unavailable, asked again");
        usage.asking();
        usage.receive(UsageInbound::Probed(Err("offline".into())), NOW + 63_000);
        assert_eq!(usage.tick(NOW + 64_000), UsageTick { changed: false, ask: false }, "said once");
        let view = usage.view(true, NOW + 64_000);
        assert!(view.unavailable);
        assert_eq!(view.quota.as_ref().map(|q| q.asr.used_seconds), Some(95), "the last numbers stay");
        assert_eq!(view.reported_at, Some(NOW));
        assert!(usage.tick(NOW + 123_000).ask, "asked every minute");

        // A client's answer reports the day too: the numbers are back.
        usage.receive(UsageInbound::Note(UsageNote::Quota(report(RESET, 120))), NOW + 70_000);
        let view = usage.view(true, NOW + 70_000);
        assert!(!view.unavailable);
        assert_eq!((view.quota.map(|q| q.asr.used_seconds), view.reported_at), (Some(120), Some(NOW + 70_000)));
        assert!(!usage.tick(NOW + 200_000).changed, "no failure left to time");
    }

    /// The next question is in ten minutes, or just after the day's reset when that is sooner.
    #[test]
    fn the_next_question_is_in_ten_minutes_or_just_after_the_reset() {
        let (mut usage, _rx, _dir) = runtime();
        usage.receive(UsageInbound::Probed(Ok(report(RESET, 95))), NOW);
        let interval = ms(USAGE_PROBE_INTERVAL);
        assert!(!usage.tick(NOW + interval - 1).ask);
        assert!(usage.tick(NOW + interval).ask);

        let soon = NOW / 1000 + 120;
        usage.receive(UsageInbound::Probed(Ok(report(soon, 95))), NOW);
        assert!(!usage.tick(soon * 1000 + 4_999).ask);
        assert!(usage.tick(soon * 1000 + 5_000).ask, "five seconds after the reset");

        // A report of a day that already ended (a clock behind the service's) is asked again soon.
        usage.receive(UsageInbound::Probed(Ok(report(NOW / 1000 - 60, 95))), NOW);
        assert!(!usage.tick(NOW + 4_999).ask);
        assert!(usage.tick(NOW + 5_000).ask);
    }

    /// Past the reset the view shows today's day with nothing used, before the service is asked.
    #[test]
    fn a_day_that_ended_shows_as_todays_with_nothing_used() {
        let (mut usage, _rx, _dir) = runtime();
        assert_eq!(usage.view(true, NOW).quota, None, "nothing reported yet");
        usage.receive(UsageInbound::Probed(Ok(report(RESET, 95))), NOW);
        assert_eq!(usage.view(true, RESET * 1000 - 1).quota.unwrap().asr.used_seconds, 95);
        let next = usage.view(true, RESET * 1000).quota.unwrap();
        assert_eq!((next.asr.used_seconds, next.refine.used), (0, 0));
        assert_eq!((next.day.as_str(), next.reset_at), ("2026-10-28", RESET + 86_400));
        assert_eq!(next.asr.limit_seconds, Some(3600), "the limits stay");
        assert!(!usage.view(false, NOW).builtin);
    }

    /// The metering's notes reach the runtime's queue; an answered request goes into the record.
    #[tokio::test]
    async fn the_meterings_notes_reach_the_record() {
        let (mut usage, mut rx, _dir) = runtime();
        let header = format!("mode=count; basis=device; day={}; reset={RESET}; asr=95/3600; refine=3/100", beijing_day(NOW));
        usage.metering.report(&header);
        let event = UsageEvent {
            at_ms: NOW,
            provider: ProviderId::Builtin,
            model: None,
            service: ServiceKind::Asr,
            purpose: UsagePurpose::Take,
            amount: 90_000,
            origin: UsageOrigin::ThisDevice,
        };
        usage.receive(UsageInbound::Note(UsageNote::Used(event)), NOW);
        let inbound = rx.recv().await.unwrap();
        assert!(matches!(&inbound, UsageInbound::Note(UsageNote::Quota(q)) if q.asr.used_seconds == 95));
        usage.receive(inbound, NOW);
        let view = usage.view(true, NOW);
        assert_eq!(view.quota.map(|q| q.reset_at), Some(RESET));
        assert_eq!(view.record.days.last().map(|d| d.asr_seconds), Some(90), "{:?}", view.record);
    }
}
