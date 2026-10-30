//! The built-in service's usage and daily quota (docs/dictation.md §27). Every request to the
//! built-in service says which device asks ([`Metering`]: a random id in `usage-id`, kept apart
//! from the pairing keys) and what for; its answers report the device's day ([`QuotaReport`]).
//! The recognisers and clean-up clients the engine builds are wrapped in [`MeteredTranscriber`]
//! and [`MeteredRefiner`], which tell the runtime what each answered request used
//! ([`UsageEvent`]); the runtime keeps them by day in [`UsageLedger`] for the usage page.
//! Previews are reported to the service (it counts the audio it decodes) but not kept here.

mod ledger;
mod report;

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::dictation::{DictationError, RefineHints, Refined, Refiner, StreamingTranscriber, Transcriber, Transcript};
use crate::providers::{ProviderId, ServiceKind};

pub use ledger::{CHART_DAYS, LEDGER_DAYS, LEDGER_FILE_NAME, LedgerView, OriginUse, ProviderUse, PurposeUse, UsageDayView, UsageLedger, UsageNotice};
pub use report::{AsrQuota, QuotaBasis, QuotaMode, QuotaReport, RefineQuota};

/// The file that holds this device's usage id.
pub const DEVICE_ID_FILE_NAME: &str = "usage-id";
/// The request header naming the device.
pub const DEVICE_HEADER: &str = "x-voltip-device";
/// The request header naming the client: `voltip/<version> (<platform>)`.
pub const CLIENT_HEADER: &str = "x-voltip-client";
/// The request header saying what a recognition is for: `take` or `preview`.
pub const PURPOSE_HEADER: &str = "x-voltip-purpose";
/// The request header carrying a recognition's length of audio, in milliseconds.
pub const AUDIO_MS_HEADER: &str = "x-voltip-audio-ms";

/// What a request was for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsagePurpose {
    /// A take's text (whole, a long take's segment, or the streaming sentences).
    #[default]
    Take,
    /// The live preview's decode of the sentence being spoken.
    Preview,
    /// The clean-up of a take (or of a history entry, a preset tried).
    Refine,
    /// A voice edit's rewrite.
    Edit,
}

impl UsagePurpose {
    /// The wire value ([`PURPOSE_HEADER`]).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Take => "take",
            Self::Preview => "preview",
            Self::Refine => "refine",
            Self::Edit => "edit",
        }
    }
}

/// Where the take or the text came from.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UsageOrigin {
    /// Dictation on this device, and everything else it asks for (history, presets tried).
    #[default]
    ThisDevice,
    /// A take a paired phone streamed to this computer (docs/dictation.md §20).
    Phone {
        /// The phone's name.
        name: String,
    },
    /// A kept recording recognised again (docs/dictation.md §26).
    Retake,
    /// The local speech service (docs/dictation.md §23).
    Serve,
    /// An imported audio or video file (docs/dictation.md §31).
    Import,
}

/// One recognition call: what it is for and, when the caller knows it, where the take came from
/// (otherwise the [`Metering`]'s default).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AsrCall {
    /// What the call is for.
    pub purpose: UsagePurpose,
    /// Where the take came from.
    pub origin: Option<UsageOrigin>,
}

impl AsrCall {
    /// A take's recognition from `origin`.
    pub fn take(origin: Option<UsageOrigin>) -> Self {
        Self { purpose: UsagePurpose::Take, origin }
    }

    /// A live preview's decode.
    pub fn preview(origin: Option<UsageOrigin>) -> Self {
        Self { purpose: UsagePurpose::Preview, origin }
    }
}

/// What one answered request used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageEvent {
    /// When it was answered, Unix milliseconds.
    pub at_ms: u64,
    /// Whose service answered it.
    pub provider: ProviderId,
    /// The model that answered, when the client said.
    pub model: Option<String>,
    /// Recognition or clean-up.
    pub service: ServiceKind,
    /// What for.
    pub purpose: UsagePurpose,
    /// Recognition: milliseconds of audio; clean-up: 1.
    pub amount: u64,
    /// Where the take came from.
    pub origin: UsageOrigin,
}

/// What a [`Metering`] tells its owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UsageNote {
    /// A request was answered.
    Used(UsageEvent),
    /// The built-in service reported the device's day.
    Quota(QuotaReport),
}

/// `UiState.usage` (docs/dictation.md §27): the built-in service's account of today and this
/// device's record.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UsageView {
    /// The build carries the built-in service: the usage card and page are there.
    pub builtin: bool,
    /// Today as the service last reported it; `None` before its first report.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota: Option<QuotaReport>,
    /// When `quota` was reported, Unix milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reported_at: Option<u64>,
    /// The service could not be asked for a minute (offline): `quota` is the last known.
    pub unavailable: bool,
    /// This device's record.
    #[serde(flatten)]
    pub record: LedgerView,
}

/// The milliseconds since the epoch.
fn wall_clock_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)).unwrap_or(0)
}

/// Who this device is to the built-in service and where its usage goes. Cheap to clone; clones
/// tell the same owner. Two meterings are equal when they name the same device and client.
#[derive(Clone)]
pub struct Metering {
    device: String,
    client: String,
    default_origin: UsageOrigin,
    sink: Arc<dyn Fn(UsageNote) + Send + Sync>,
    clock: Arc<dyn Fn() -> u64 + Send + Sync>,
}

impl PartialEq for Metering {
    fn eq(&self, other: &Self) -> bool {
        self.device == other.device && self.client == other.client && self.default_origin == other.default_origin
    }
}

impl Eq for Metering {}

impl std::fmt::Debug for Metering {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Metering").field("client", &self.client).field("default_origin", &self.default_origin).finish_non_exhaustive()
    }
}

impl Metering {
    /// `device` (its usage id) as `client` (`voltip/<version> (<platform>)`), telling `sink`; a
    /// call that names no origin is `default_origin`'s.
    pub fn new(device: impl Into<String>, client: impl Into<String>, default_origin: UsageOrigin, sink: impl Fn(UsageNote) + Send + Sync + 'static) -> Self {
        Self { device: device.into(), client: client.into(), default_origin, sink: Arc::new(sink), clock: Arc::new(wall_clock_ms) }
    }

    /// The same metering on `clock` (milliseconds since the epoch; tests).
    pub fn with_clock(mut self, clock: impl Fn() -> u64 + Send + Sync + 'static) -> Self {
        self.clock = Arc::new(clock);
        self
    }

    /// The same metering whose calls without an origin are `origin`'s.
    pub fn with_default_origin(mut self, origin: UsageOrigin) -> Self {
        self.default_origin = origin;
        self
    }

    /// The device's usage id.
    pub fn device(&self) -> &str {
        &self.device
    }

    /// The headers every request to the built-in service carries.
    pub fn headers(&self) -> Vec<(String, String)> {
        vec![(DEVICE_HEADER.to_owned(), self.device.clone()), (CLIENT_HEADER.to_owned(), self.client.clone())]
    }

    /// An answer's `X-Voltip-Quota`: the device's day, to the owner. An unreadable report is
    /// dropped with a debug line.
    pub fn report(&self, header: &str) {
        match QuotaReport::parse_header(header) {
            Some(report) => (self.sink)(UsageNote::Quota(report)),
            None => tracing::debug!("the built-in service's usage report could not be read"),
        }
    }

    /// A request `provider` answered for `purpose`, from `origin` (the default when `None`).
    fn used(&self, provider: ProviderId, model: Option<String>, service: ServiceKind, purpose: UsagePurpose, amount: u64, origin: Option<UsageOrigin>) {
        let origin = origin.unwrap_or_else(|| self.default_origin.clone());
        (self.sink)(UsageNote::Used(UsageEvent { at_ms: (self.clock)(), provider, model, service, purpose, amount, origin }));
    }
}

/// A recogniser whose answered takes are told to a [`Metering`] as `provider`'s (previews are not).
pub struct MeteredTranscriber {
    inner: Arc<dyn Transcriber>,
    provider: ProviderId,
    metering: Metering,
}

impl MeteredTranscriber {
    /// `inner`, serving `provider`'s model.
    pub fn new(inner: Arc<dyn Transcriber>, provider: ProviderId, metering: Metering) -> Self {
        Self { inner, provider, metering }
    }
}

#[async_trait]
impl Transcriber for MeteredTranscriber {
    async fn transcribe(&self, wav: &[u8], language: Option<&str>, glossary: &[String]) -> Result<Transcript, DictationError> {
        self.transcribe_as(wav, language, glossary, &AsrCall::default()).await
    }

    async fn transcribe_as(&self, wav: &[u8], language: Option<&str>, glossary: &[String], call: &AsrCall) -> Result<Transcript, DictationError> {
        let transcript = self.inner.transcribe_as(wav, language, glossary, call).await?;
        if call.purpose != UsagePurpose::Preview {
            let ms = crate::dictation::wav::duration_ms(wav).unwrap_or(0);
            self.metering.used(self.provider, transcript.model.clone(), ServiceKind::Asr, call.purpose, ms, call.origin.clone());
        }
        Ok(transcript)
    }

    fn warm(&self, language: Option<&str>) {
        self.inner.warm(language);
    }

    fn streaming(&self, glossary: &[String]) -> Option<Arc<dyn StreamingTranscriber>> {
        self.inner.streaming(glossary)
    }
}

/// A clean-up client whose answered requests are told to a [`Metering`] as `provider`'s.
pub struct MeteredRefiner {
    inner: Arc<dyn Refiner>,
    provider: ProviderId,
    metering: Metering,
}

impl MeteredRefiner {
    /// `inner`, serving `provider`'s model.
    pub fn new(inner: Arc<dyn Refiner>, provider: ProviderId, metering: Metering) -> Self {
        Self { inner, provider, metering }
    }
}

#[async_trait]
impl Refiner for MeteredRefiner {
    async fn refine(&self, text: &str, hints: &RefineHints) -> Result<Refined, DictationError> {
        let refined = self.inner.refine(text, hints).await?;
        self.metering.used(self.provider, Some(refined.model.clone()), ServiceKind::Llm, UsagePurpose::Refine, 1, hints.origin.clone());
        Ok(refined)
    }

    async fn edit(&self, selection: &str, instruction: &str, hints: &RefineHints) -> Result<Refined, DictationError> {
        let refined = self.inner.edit(selection, instruction, hints).await?;
        self.metering.used(self.provider, Some(refined.model.clone()), ServiceKind::Llm, UsagePurpose::Edit, 1, hints.origin.clone());
        Ok(refined)
    }
}

/// The built-in service's quota for one device, asked directly (docs/dictation.md §27): the usage
/// card's numbers before the first take and while the app is idle.
#[async_trait]
pub trait UsageProbe: Send + Sync + std::fmt::Debug {
    /// `GET {base}/voltip/quota` (`base` as the recognition or clean-up endpoint has it, `/v1`
    /// added) with the service's `token` and the metering's `headers`.
    async fn quota(&self, base_url: &str, token: Option<&str>, headers: &[(String, String)]) -> Result<QuotaReport, String>;
}

/// This device's usage id (`usage-id` under `dir`), made on first use: a random UUID, unrelated to
/// the pairing keys, never synced or backed up.
pub fn device_id(dir: &Path) -> std::io::Result<String> {
    if let Some(id) = read_device_id(dir) {
        return Ok(id);
    }
    let id = uuid::Uuid::new_v4().to_string();
    std::fs::create_dir_all(dir)?;
    let path = dir.join(DEVICE_ID_FILE_NAME);
    let tmp = dir.join(format!("{DEVICE_ID_FILE_NAME}.tmp"));
    std::fs::write(&tmp, &id)?;
    std::fs::rename(&tmp, &path)?;
    Ok(id)
}

/// The usage id under `dir`, when there is a readable one; never writes.
pub fn read_device_id(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(DEVICE_ID_FILE_NAME)).ok()?;
    let id = uuid::Uuid::parse_str(text.trim()).ok()?;
    Some(id.hyphenated().to_string())
}

/// Beijing time's offset from UTC (no daylight saving).
const BEIJING_OFFSET_MS: u64 = 8 * 60 * 60 * 1000;
const DAY_MS: u64 = 24 * 60 * 60 * 1000;

/// The Beijing day of `at_ms` as days since 1970-01-01.
pub fn beijing_day_index(at_ms: u64) -> i64 {
    i64::try_from((at_ms.saturating_add(BEIJING_OFFSET_MS)) / DAY_MS).unwrap_or(i64::MAX)
}

/// The Beijing day of `at_ms`, `YYYY-MM-DD`: the built-in service's day.
pub fn beijing_day(at_ms: u64) -> String {
    day_string(beijing_day_index(at_ms))
}

/// The next Beijing midnight after `at_ms`, milliseconds since the epoch: when the built-in
/// service's day starts again.
pub fn next_beijing_midnight_ms(at_ms: u64) -> u64 {
    ((at_ms.saturating_add(BEIJING_OFFSET_MS)) / DAY_MS + 1) * DAY_MS - BEIJING_OFFSET_MS
}

/// `YYYY-MM-DD` of the day `index` days after 1970-01-01 (proleptic Gregorian).
pub fn day_string(index: i64) -> String {
    let z = index + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use parking_lot::Mutex;

    use super::*;
    use crate::dictation::fakes::{FakeRefiner, FakeTranscriber};

    /// A metering whose notes the test reads.
    fn metering() -> (Metering, Arc<Mutex<Vec<UsageNote>>>) {
        let notes = Arc::new(Mutex::new(Vec::new()));
        let sink = notes.clone();
        (
            Metering::new("6f1c2d3e-0000-4000-8000-000000000001", "voltip/0.1.10 (linux)", UsageOrigin::ThisDevice, move |n| sink.lock().push(n))
                .with_clock(|| 7),
            notes,
        )
    }

    fn wav(ms: usize) -> Vec<u8> {
        crate::dictation::wav::encode_pcm16(&vec![100; ms * 16], 16_000)
    }

    #[tokio::test]
    async fn a_metered_recogniser_tells_its_takes_with_their_length_and_origin_but_not_previews() {
        let (metering, notes) = metering();
        let t = MeteredTranscriber::new(Arc::new(FakeTranscriber::ok("好")), ProviderId::Builtin, metering);
        t.transcribe(&wav(1500), None, &[]).await.unwrap();
        t.transcribe_as(&wav(400), None, &[], &AsrCall::preview(None)).await.unwrap();
        t.transcribe_as(&wav(2000), None, &[], &AsrCall::take(Some(UsageOrigin::Phone { name: "Pixel 8".into() }))).await.unwrap();
        let notes = notes.lock();
        assert_eq!(notes.len(), 2, "the preview is not kept: {notes:?}");
        let UsageNote::Used(first) = &notes[0] else { panic!() };
        assert_eq!(
            (first.provider, first.service, first.purpose, first.amount, &first.origin, first.at_ms),
            (ProviderId::Builtin, ServiceKind::Asr, UsagePurpose::Take, 1500, &UsageOrigin::ThisDevice, 7)
        );
        let UsageNote::Used(second) = &notes[1] else { panic!() };
        assert_eq!(second.origin, UsageOrigin::Phone { name: "Pixel 8".into() });
        assert_eq!(second.amount, 2000);
    }

    #[tokio::test]
    async fn a_failed_request_uses_nothing() {
        let (metering, notes) = metering();
        let t = MeteredTranscriber::new(Arc::new(FakeTranscriber::err("ASR server error 500")), ProviderId::Openai, metering.clone());
        assert!(t.transcribe(&wav(100), None, &[]).await.is_err());
        let r = MeteredRefiner::new(Arc::new(FakeRefiner::err("500")), ProviderId::Openai, metering);
        assert!(r.refine("x", &RefineHints::default()).await.is_err());
        assert!(notes.lock().is_empty());
    }

    #[tokio::test]
    async fn a_metered_refiner_tells_clean_ups_and_edits_with_the_hints_origin() {
        let (metering, notes) = metering();
        let r = MeteredRefiner::new(Arc::new(FakeRefiner::ok("好的。")), ProviderId::Builtin, metering.with_default_origin(UsageOrigin::Serve));
        r.refine("好的", &RefineHints::default()).await.unwrap();
        let hints = RefineHints { origin: Some(UsageOrigin::Retake), ..RefineHints::default() };
        r.edit("你好", "改成英文", &hints).await.unwrap();
        let used: Vec<(UsagePurpose, UsageOrigin, u64)> = notes
            .lock()
            .iter()
            .map(|n| match n {
                UsageNote::Used(e) => (e.purpose, e.origin.clone(), e.amount),
                UsageNote::Quota(_) => panic!("{n:?}"),
            })
            .collect();
        assert_eq!(used, [(UsagePurpose::Refine, UsageOrigin::Serve, 1), (UsagePurpose::Edit, UsageOrigin::Retake, 1)]);
    }

    #[test]
    fn a_report_reaches_the_owner_and_an_unreadable_one_is_dropped() {
        let (metering, notes) = metering();
        metering.report("mode=count; basis=device; day=2026-10-27; reset=1793116800; asr=95/-; refine=3/-");
        metering.report("garbage");
        assert!(matches!(&notes.lock()[..], [UsageNote::Quota(r)] if r.asr.used_seconds == 95));
        assert_eq!(metering.headers()[0], (DEVICE_HEADER.to_owned(), "6f1c2d3e-0000-4000-8000-000000000001".to_owned()));
        assert_eq!(metering.headers()[1], (CLIENT_HEADER.to_owned(), "voltip/0.1.10 (linux)".to_owned()));
        assert!(!format!("{metering:?}").contains("6f1c"), "the debug line leaves the id out");
        let (other, _) = self::metering();
        assert_eq!(metering, other, "the sink is not compared");
        assert_ne!(metering, other.with_default_origin(UsageOrigin::Serve));
    }

    #[test]
    fn the_device_id_is_made_once_and_read_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_device_id(dir.path()), None);
        let id = device_id(dir.path()).unwrap();
        assert!(uuid::Uuid::parse_str(&id).is_ok(), "{id}");
        assert_eq!(device_id(dir.path()).unwrap(), id, "made once");
        assert_eq!(read_device_id(dir.path()).as_deref(), Some(id.as_str()));
        std::fs::write(dir.path().join(DEVICE_ID_FILE_NAME), "not an id").unwrap();
        assert_eq!(read_device_id(dir.path()), None, "an unreadable id is no id");
        assert_ne!(device_id(dir.path()).unwrap(), id, "and is replaced");
    }

    #[test]
    fn beijing_days_turn_at_its_midnight() {
        // 2026-10-27 15:59:59.999 UTC is 23:59:59.999 in Beijing.
        let before = 1_793_116_799_999;
        assert_eq!(beijing_day(before), "2026-10-27");
        assert_eq!(beijing_day(before + 1), "2026-10-28");
        assert_eq!(next_beijing_midnight_ms(before), before + 1);
        assert_eq!(next_beijing_midnight_ms(before + 1), before + 1 + DAY_MS);
        assert_eq!(day_string(0), "1970-01-01");
        assert_eq!(day_string(beijing_day_index(before) - 299), "2026-01-01");
        assert_eq!(day_string(19_782), "2024-02-29", "a leap day");
    }
}
