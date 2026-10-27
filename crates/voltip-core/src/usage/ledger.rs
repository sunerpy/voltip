//! This device's record of what it used (`usage.json`, docs/dictation.md §27): by Beijing day,
//! service, provider, model, purpose and origin, for [`LEDGER_DAYS`] days. The built-in service's
//! own count is the authority for today's numbers; the record shows the days before and what
//! today's use was made of, and the other providers' month. Written by the app alone; not synced,
//! not backed up.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{UsageEvent, UsageOrigin, UsagePurpose, beijing_day_index, day_string};
use crate::providers::{ProviderId, ServiceKind};

/// The file under the data directory.
pub const LEDGER_FILE_NAME: &str = "usage.json";
/// Days kept, today included: a month and its first days, for 本月.
pub const LEDGER_DAYS: i64 = 35;
/// Days the usage page charts, today included.
pub const CHART_DAYS: i64 = 7;

/// A usage notice the user closed: it stays closed for the rest of the day.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageNotice {
    /// Recognition is near its limit (80 % and more).
    AsrNear,
    /// Recognition's limit is reached.
    AsrOut,
    /// Clean-up is near its limit.
    RefineNear,
    /// Clean-up's limit is reached.
    RefineOut,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Row {
    service: ServiceKind,
    provider: ProviderId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    purpose: UsagePurpose,
    origin: UsageOrigin,
    /// Recognition: milliseconds of audio; clean-up: requests.
    amount: u64,
}

impl Row {
    fn same(&self, e: &UsageEvent) -> bool {
        self.service == e.service && self.provider == e.provider && self.model == e.model && self.purpose == e.purpose && self.origin == e.origin
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Day {
    /// `YYYY-MM-DD`, Beijing time.
    day: String,
    #[serde(default)]
    rows: Vec<Row>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    dismissed: Vec<UsageNotice>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct LedgerFile {
    schema: u32,
    #[serde(default)]
    days: Vec<Day>,
}

/// One day of the built-in service's use on this device.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageDayView {
    /// `YYYY-MM-DD`, Beijing time.
    pub day: String,
    /// Seconds of audio recognised.
    pub asr_seconds: u64,
    /// Clean-ups and voice edits.
    pub refine: u64,
}

/// Today's built-in recognition from one origin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginUse {
    /// Where the takes came from.
    pub origin: UsageOrigin,
    /// Seconds of audio.
    pub seconds: u64,
}

/// Today's built-in clean-up for one purpose.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PurposeUse {
    /// [`UsagePurpose::Refine`] or [`UsagePurpose::Edit`].
    pub purpose: UsagePurpose,
    /// Requests.
    pub count: u64,
}

/// This month's use of another provider's model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderUse {
    /// Whose.
    pub provider: ProviderId,
    /// Recognition or clean-up.
    pub service: ServiceKind,
    /// The model, when the client named it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Recognition: seconds of audio; clean-up: requests.
    pub amount: u64,
}

/// What the record shows (`UiState.usage`'s own part).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerView {
    /// The last [`CHART_DAYS`] days, oldest first, today last.
    pub days: Vec<UsageDayView>,
    /// Today's built-in recognition by origin, most first.
    pub asr_origins: Vec<OriginUse>,
    /// Today's built-in clean-up by purpose, most first.
    pub refine_purposes: Vec<PurposeUse>,
    /// This month's other providers, most first within each service.
    pub others: Vec<ProviderUse>,
    /// Today's other providers, most first within each service (the usage card when the built-in
    /// service is not in use).
    pub others_today: Vec<ProviderUse>,
    /// The notices closed today.
    pub dismissed: Vec<UsageNotice>,
}

/// `usage.json` and what it holds.
#[derive(Debug)]
pub struct UsageLedger {
    path: Option<PathBuf>,
    days: Vec<Day>,
}

impl UsageLedger {
    /// The record under `dir`; a missing or unreadable file starts empty (an unreadable one is
    /// moved aside, never overwritten).
    pub fn open(dir: &Path) -> Self {
        let path = dir.join(LEDGER_FILE_NAME);
        let days = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<LedgerFile>(&bytes) {
                Ok(file) if file.schema == 1 => file.days,
                Ok(file) => {
                    tracing::warn!(schema = file.schema, "usage.json has another schema; starting a new record");
                    set_aside(&path);
                    Vec::new()
                }
                Err(e) => {
                    tracing::warn!(error = %e, "usage.json could not be read; starting a new record");
                    set_aside(&path);
                    Vec::new()
                }
            },
            Err(_) => Vec::new(),
        };
        Self { path: Some(path), days }
    }

    /// A record kept in memory only (tests, a shell without a data directory).
    pub fn in_memory() -> Self {
        Self { path: None, days: Vec::new() }
    }

    /// Add what `event` used, keep the last [`LEDGER_DAYS`] days and save.
    pub fn record(&mut self, event: &UsageEvent) {
        let today = beijing_day_index(event.at_ms);
        let day = self.day_mut(today);
        match day.rows.iter_mut().find(|r| r.same(event)) {
            Some(row) => row.amount = row.amount.saturating_add(event.amount),
            None => day.rows.push(Row {
                service: event.service,
                provider: event.provider,
                model: event.model.clone(),
                purpose: event.purpose,
                origin: event.origin.clone(),
                amount: event.amount,
            }),
        }
        self.prune(today);
        self.save();
    }

    /// Close `notice` for the rest of the day of `now_ms`.
    pub fn dismiss(&mut self, notice: UsageNotice, now_ms: u64) {
        let today = beijing_day_index(now_ms);
        let day = self.day_mut(today);
        if !day.dismissed.contains(&notice) {
            day.dismissed.push(notice);
            day.dismissed.sort();
        }
        self.prune(today);
        self.save();
    }

    /// The record as of `now_ms`.
    pub fn view(&self, now_ms: u64) -> LedgerView {
        let today = beijing_day_index(now_ms);
        let today_name = day_string(today);
        let builtin = |r: &&Row| r.provider == ProviderId::Builtin && r.purpose != UsagePurpose::Preview;
        let days = (today - CHART_DAYS + 1..=today)
            .map(|index| {
                let name = day_string(index);
                let rows = self.days.iter().find(|d| d.day == name).map(|d| d.rows.as_slice()).unwrap_or_default();
                let asr_ms: u64 = rows.iter().filter(builtin).filter(|r| r.service == ServiceKind::Asr).map(|r| r.amount).sum();
                let refine = rows.iter().filter(builtin).filter(|r| r.service == ServiceKind::Llm).map(|r| r.amount).sum();
                UsageDayView { day: name, asr_seconds: seconds(asr_ms), refine }
            })
            .collect();
        let today_rows = self.days.iter().find(|d| d.day == today_name);
        let rows = today_rows.map(|d| d.rows.as_slice()).unwrap_or_default();
        let mut asr_origins: Vec<OriginUse> = Vec::new();
        for row in rows.iter().filter(builtin).filter(|r| r.service == ServiceKind::Asr) {
            match asr_origins.iter_mut().find(|o| o.origin == row.origin) {
                Some(o) => o.seconds += row.amount,
                None => asr_origins.push(OriginUse { origin: row.origin.clone(), seconds: row.amount }),
            }
        }
        for o in &mut asr_origins {
            o.seconds = seconds(o.seconds);
        }
        asr_origins.sort_by(|a, b| b.seconds.cmp(&a.seconds).then_with(|| a.origin.cmp(&b.origin)));
        let mut refine_purposes: Vec<PurposeUse> = Vec::new();
        for row in rows.iter().filter(builtin).filter(|r| r.service == ServiceKind::Llm) {
            match refine_purposes.iter_mut().find(|p| p.purpose == row.purpose) {
                Some(p) => p.count += row.amount,
                None => refine_purposes.push(PurposeUse { purpose: row.purpose, count: row.amount }),
            }
        }
        refine_purposes.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.purpose.cmp(&b.purpose)));
        let month = &today_name[..7];
        let others = other_providers(self.days.iter().filter(|d| d.day.starts_with(month)));
        let others_today = other_providers(today_rows.into_iter());
        LedgerView { days, asr_origins, refine_purposes, others, others_today, dismissed: today_rows.map(|d| d.dismissed.clone()).unwrap_or_default() }
    }

    fn day_mut(&mut self, index: i64) -> &mut Day {
        let name = day_string(index);
        let at = match self.days.iter().position(|d| d.day == name) {
            Some(at) => at,
            None => {
                self.days.push(Day { day: name, ..Day::default() });
                self.days.sort_by(|a, b| a.day.cmp(&b.day));
                self.days.iter().position(|d| d.day == day_string(index)).unwrap_or(0)
            }
        };
        &mut self.days[at]
    }

    fn prune(&mut self, today: i64) {
        let oldest = day_string(today - LEDGER_DAYS + 1);
        self.days.retain(|d| d.day >= oldest);
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        let file = LedgerFile { schema: 1, days: self.days.clone() };
        let write = || -> std::io::Result<()> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = path.with_extension("json.tmp");
            std::fs::write(&tmp, serde_json::to_vec(&file).map_err(std::io::Error::other)?)?;
            std::fs::rename(&tmp, path)
        };
        if let Err(e) = write() {
            tracing::warn!(error = %e, "usage.json could not be saved");
        }
    }
}

/// The use of every provider but the built-in service over `days`: by provider, service and
/// model, recognition in seconds, most first within each service.
fn other_providers<'a>(days: impl Iterator<Item = &'a Day>) -> Vec<ProviderUse> {
    let mut others: Vec<ProviderUse> = Vec::new();
    for row in days.flat_map(|d| d.rows.iter()).filter(|r| r.provider != ProviderId::Builtin) {
        match others.iter_mut().find(|o| o.provider == row.provider && o.service == row.service && o.model == row.model) {
            Some(o) => o.amount += row.amount,
            None => others.push(ProviderUse { provider: row.provider, service: row.service, model: row.model.clone(), amount: row.amount }),
        }
    }
    for o in others.iter_mut().filter(|o| o.service == ServiceKind::Asr) {
        o.amount = seconds(o.amount);
    }
    others.sort_by(|a, b| a.service.cmp(&b.service).then_with(|| b.amount.cmp(&a.amount)).then_with(|| a.provider.cmp(&b.provider)));
    others
}

/// Milliseconds as whole seconds, rounded.
fn seconds(ms: u64) -> u64 {
    ms.saturating_add(500) / 1000
}

/// Move an unreadable record aside.
fn set_aside(path: &Path) {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let _ = std::fs::rename(path, path.with_file_name(format!("{LEDGER_FILE_NAME}.corrupt-{stamp}")));
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// 2026-10-27 12:00 in Beijing.
    const NOON: u64 = 1_793_073_600_000;
    const DAY: u64 = 24 * 60 * 60 * 1000;

    fn event(at_ms: u64, provider: ProviderId, service: ServiceKind, purpose: UsagePurpose, amount: u64, origin: UsageOrigin) -> UsageEvent {
        UsageEvent { at_ms, provider, model: Some("m".into()), service, purpose, amount, origin }
    }

    fn asr(at_ms: u64, ms: u64, origin: UsageOrigin) -> UsageEvent {
        event(at_ms, ProviderId::Builtin, ServiceKind::Asr, UsagePurpose::Take, ms, origin)
    }

    fn refine(at_ms: u64, purpose: UsagePurpose) -> UsageEvent {
        event(at_ms, ProviderId::Builtin, ServiceKind::Llm, purpose, 1, UsageOrigin::ThisDevice)
    }

    #[test]
    fn the_record_charts_seven_days_and_breaks_today_down() {
        let mut ledger = UsageLedger::in_memory();
        ledger.record(&asr(NOON - 2 * DAY, 90_000, UsageOrigin::ThisDevice));
        ledger.record(&asr(NOON, 60_400, UsageOrigin::ThisDevice));
        ledger.record(&asr(NOON, 30_000, UsageOrigin::Phone { name: "Pixel 8".into() }));
        ledger.record(&asr(NOON, 1_200, UsageOrigin::ThisDevice));
        ledger.record(&refine(NOON, UsagePurpose::Refine));
        ledger.record(&refine(NOON, UsagePurpose::Refine));
        ledger.record(&refine(NOON, UsagePurpose::Edit));
        ledger.record(&event(NOON, ProviderId::Aliyun, ServiceKind::Asr, UsagePurpose::Take, 125_000, UsageOrigin::ThisDevice));
        ledger.record(&event(NOON - 30 * DAY, ProviderId::Aliyun, ServiceKind::Asr, UsagePurpose::Take, 5_000, UsageOrigin::ThisDevice));
        ledger.record(&event(NOON, ProviderId::Openai, ServiceKind::Llm, UsagePurpose::Refine, 1, UsageOrigin::ThisDevice));
        let view = ledger.view(NOON);
        assert_eq!(view.days.len(), 7);
        assert_eq!((view.days[0].day.as_str(), view.days[6].day.as_str()), ("2026-10-21", "2026-10-27"));
        assert_eq!((view.days[4].asr_seconds, view.days[6].asr_seconds, view.days[6].refine), (90, 92, 3), "only the built-in service");
        assert_eq!(
            view.asr_origins,
            [OriginUse { origin: UsageOrigin::ThisDevice, seconds: 62 }, OriginUse { origin: UsageOrigin::Phone { name: "Pixel 8".into() }, seconds: 30 }]
        );
        assert_eq!(view.refine_purposes, [PurposeUse { purpose: UsagePurpose::Refine, count: 2 }, PurposeUse { purpose: UsagePurpose::Edit, count: 1 }]);
        assert_eq!(
            view.others,
            [
                ProviderUse { provider: ProviderId::Aliyun, service: ServiceKind::Asr, model: Some("m".into()), amount: 125 },
                ProviderUse { provider: ProviderId::Openai, service: ServiceKind::Llm, model: Some("m".into()), amount: 1 },
            ],
            "this month only: the 27th of September is last month"
        );
        ledger.record(&event(NOON - DAY, ProviderId::Openai, ServiceKind::Llm, UsagePurpose::Refine, 1, UsageOrigin::ThisDevice));
        assert_eq!(
            ledger.view(NOON).others_today,
            [
                ProviderUse { provider: ProviderId::Aliyun, service: ServiceKind::Asr, model: Some("m".into()), amount: 125 },
                ProviderUse { provider: ProviderId::Openai, service: ServiceKind::Llm, model: Some("m".into()), amount: 1 },
            ],
            "today only: yesterday's clean-up is not in it"
        );
    }

    #[test]
    fn days_turn_at_beijing_midnight_and_closed_notices_stay_closed_for_the_day() {
        let mut ledger = UsageLedger::in_memory();
        let late = NOON + 12 * 60 * 60 * 1000 - 1; // 23:59:59.999
        ledger.record(&asr(late, 1_000, UsageOrigin::ThisDevice));
        ledger.record(&asr(late + 1, 2_000, UsageOrigin::ThisDevice));
        ledger.dismiss(UsageNotice::AsrNear, late);
        ledger.dismiss(UsageNotice::AsrNear, late);
        assert_eq!(ledger.view(late).dismissed, [UsageNotice::AsrNear]);
        let next = ledger.view(late + 1);
        assert_eq!(next.days[6].asr_seconds, 2);
        assert_eq!(next.days[5].asr_seconds, 1);
        assert!(next.dismissed.is_empty(), "a new day, notices again");
    }

    #[test]
    fn the_record_keeps_35_days_and_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let mut ledger = UsageLedger::open(dir.path());
        for i in 0..40 {
            ledger.record(&asr(NOON - (39 - i) * DAY, 1_000, UsageOrigin::ThisDevice));
        }
        let reopened = UsageLedger::open(dir.path());
        assert_eq!(reopened.days.len(), LEDGER_DAYS as usize);
        assert_eq!(reopened.days[0].day, "2026-09-23");
        assert_eq!(reopened.view(NOON).days[6].asr_seconds, 1);
        std::fs::write(dir.path().join(LEDGER_FILE_NAME), "{ not json").unwrap();
        assert!(UsageLedger::open(dir.path()).days.is_empty(), "an unreadable record starts again");
        assert!(std::fs::read_dir(dir.path()).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().starts_with("usage.json.corrupt-")), "kept aside");
    }
}
