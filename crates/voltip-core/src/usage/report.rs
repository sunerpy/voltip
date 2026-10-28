//! What the built-in service says about a device's day (docs/dictation.md §27): the
//! `X-Voltip-Quota` header of its answers and the body of its quota query, read into one
//! [`QuotaReport`].

use serde::{Deserialize, Serialize};

/// Whether the service only counts or also refuses past the limits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaMode {
    /// The counting period: nothing is refused for its use.
    #[default]
    Count,
    /// Past a limit the service refuses with `insufficient_quota` until the reset.
    Limit,
}

/// What the day is counted by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaBasis {
    /// This device (its `X-Voltip-Device`).
    #[default]
    Device,
    /// The address the requests came from (a request without a device: versions before 0.2.0).
    Ip,
}

/// Recognition used today, in seconds of audio.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsrQuota {
    /// Seconds used.
    pub used_seconds: u64,
    /// The day's limit; `None`: none (the counting period).
    pub limit_seconds: Option<u64>,
}

/// Clean-ups and voice edits used today.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefineQuota {
    /// Requests used.
    pub used: u64,
    /// The day's limit; `None`: none.
    pub limit: Option<u64>,
}

/// The built-in service's account of this device's day (`UiState.usage.quota`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuotaReport {
    /// Counting or limiting.
    pub mode: QuotaMode,
    /// Counted by device or by address.
    pub basis: QuotaBasis,
    /// The service's day, `YYYY-MM-DD` (Beijing time).
    pub day: String,
    /// When the day's use starts again from zero, Unix seconds.
    pub reset_at: u64,
    /// Recognition.
    pub asr: AsrQuota,
    /// Clean-up.
    pub refine: RefineQuota,
}

/// The body of `GET …/voltip/quota`.
#[derive(Deserialize)]
struct QuotaAnswer {
    schema: u32,
    #[serde(flatten)]
    report: QuotaReport,
}

impl QuotaReport {
    /// Read an `X-Voltip-Quota` header:
    /// `mode=count; basis=device; day=2026-10-27; reset=1793116800; asr=95/-; refine=3/100`, the
    /// limits `-` when there are none. Fields it does not know are skipped; one it needs that is
    /// missing or unreadable makes the whole report unreadable.
    pub fn parse_header(header: &str) -> Option<Self> {
        let mut report = Self::default();
        let (mut mode, mut basis, mut day, mut reset, mut asr, mut refine) = (None, None, None, None, None, None);
        for part in header.split(';') {
            let Some((key, value)) = part.split_once('=') else { continue };
            let value = value.trim();
            match key.trim() {
                "mode" => mode = Some(serde_json::from_value::<QuotaMode>(serde_json::Value::String(value.to_owned())).ok()?),
                "basis" => basis = Some(serde_json::from_value::<QuotaBasis>(serde_json::Value::String(value.to_owned())).ok()?),
                "day" => day = Some(value.to_owned()),
                "reset" => reset = Some(value.parse::<u64>().ok()?),
                "asr" => asr = Some(used_and_limit(value)?),
                "refine" => refine = Some(used_and_limit(value)?),
                _ => {}
            }
        }
        report.mode = mode?;
        report.basis = basis?;
        report.day = day.filter(|d| is_day(d))?;
        report.reset_at = reset?;
        let (used_seconds, limit_seconds) = asr?;
        report.asr = AsrQuota { used_seconds, limit_seconds };
        let (used, limit) = refine?;
        report.refine = RefineQuota { used, limit };
        Some(report)
    }

    /// Read the body of the quota query (`schema` 1).
    pub fn parse_answer(body: &str) -> Result<Self, String> {
        let answer: QuotaAnswer = serde_json::from_str(body).map_err(|e| format!("not a quota answer: {e}"))?;
        if answer.schema != 1 {
            return Err(format!("quota answer schema {} is not 1", answer.schema));
        }
        if !is_day(&answer.report.day) {
            return Err(format!("not a day: {:?}", answer.report.day));
        }
        Ok(answer.report)
    }
}

/// `95/3600`, `95/-`.
fn used_and_limit(value: &str) -> Option<(u64, Option<u64>)> {
    let (used, limit) = value.split_once('/')?;
    let used = used.trim().parse().ok()?;
    let limit = match limit.trim() {
        "-" => None,
        limit => Some(limit.parse().ok()?),
    };
    Some((used, limit))
}

/// `YYYY-MM-DD`.
fn is_day(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 10 && bytes[4] == b'-' && bytes[7] == b'-' && bytes.iter().enumerate().all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_header_reads_into_a_report() {
        let report = QuotaReport::parse_header("mode=limit; basis=device; day=2026-10-27; reset=1793116800; asr=95/3600; refine=3/100").unwrap();
        assert_eq!(
            report,
            QuotaReport {
                mode: QuotaMode::Limit,
                basis: QuotaBasis::Device,
                day: "2026-10-27".into(),
                reset_at: 1_793_116_800,
                asr: AsrQuota { used_seconds: 95, limit_seconds: Some(3600) },
                refine: RefineQuota { used: 3, limit: Some(100) },
            }
        );
        let counting = QuotaReport::parse_header("mode=count;basis=ip;day=2026-10-27;reset=1;asr=0/-;refine=7/-;later=field").unwrap();
        assert_eq!((counting.mode, counting.basis, counting.asr.limit_seconds, counting.refine.limit), (QuotaMode::Count, QuotaBasis::Ip, None, None));
        for broken in [
            "",
            "mode=count; basis=device; day=2026-10-27; reset=1; asr=0/-",
            "mode=always; basis=device; day=2026-10-27; reset=1; asr=0/-; refine=0/-",
            "mode=count; basis=device; day=27.10.2026; reset=1; asr=0/-; refine=0/-",
            "mode=count; basis=device; day=2026-10-27; reset=soon; asr=0/-; refine=0/-",
            "mode=count; basis=device; day=2026-10-27; reset=1; asr=0; refine=0/-",
        ] {
            assert_eq!(QuotaReport::parse_header(broken), None, "{broken:?}");
        }
    }

    #[test]
    fn the_quota_answer_reads_into_the_same_report() {
        let body = r#"{"schema":1,"mode":"count","basis":"device","day":"2026-10-27","reset_at":1793116800,"asr":{"used_seconds":95,"limit_seconds":null},"refine":{"used":3,"limit":null}}"#;
        let report = QuotaReport::parse_answer(body).unwrap();
        assert_eq!(report, QuotaReport::parse_header("mode=count; basis=device; day=2026-10-27; reset=1793116800; asr=95/-; refine=3/-").unwrap());
        assert!(QuotaReport::parse_answer(&body.replace(r#""schema":1"#, r#""schema":2"#)).is_err());
        assert!(QuotaReport::parse_answer("<html>").is_err());
        // The interface reads the report in the same shape.
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["asr"]["limit_seconds"], serde_json::Value::Null);
        assert_eq!(json["mode"], "count");
    }
}
