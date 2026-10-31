//! Fallback models (docs/dictation.md §3.5, §3.14): the selected model first, then the fallback
//! models, tried in order when a model's quota is used up — and, with 服务出错时也改用候补 on, when
//! a model fails. Nothing here knows a provider's endpoint or a port: a [`QuotaLedger`] remembers
//! which models ran out or failed and until when, [`first_available`] runs one call along a chain
//! of [`Link`]s, and [`FallbackTranscriber`] / [`FallbackRefiner`] put a chain behind the
//! recognition and clean-up ports. The engine owns the ledger: it outlives the clients, which are
//! rebuilt on every configuration change.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::watch;

use super::ports::{
    DictationError, RefineHints, Refined, Refiner, StreamEvent, StreamFinal, StreamingSession, StreamingTranscriber, SwitchReason, Transcriber, Transcript,
};
use crate::providers::{ProviderId, ServiceKind};

/// How long a model whose quota ran out is skipped. Then it is tried once more: a refused request
/// is not billed, and the quota may be back (some of Model Studio's reset on the 1st of the month,
/// or the owner topped the account up).
pub const QUOTA_RETRY_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

/// How long a model that failed is skipped (docs/dictation.md §3.14), unless it said how long to
/// wait (a 429's `Retry-After`).
pub const OUTAGE_RETRY_AFTER: Duration = Duration::from_secs(60);

/// Most models one request asks that fail (docs/dictation.md §3.14): the selected one and two
/// fallback models. A used-up quota's refusal is not counted: it is not billed, and the quota rule
/// is as before.
pub const MAX_OUTAGE_TRIES: usize = 3;

/// The whole of one recognition request along the chain while it moves on for failures
/// (docs/dictation.md §3.14), from its first attempt; a long take's segments each have their own.
pub const ASR_DEADLINE: Duration = Duration::from_secs(150);

/// The whole of one clean-up or edit request along the chain while it moves on for failures.
pub const REFINE_DEADLINE: Duration = Duration::from_secs(60);

/// No further model is asked with less than this left before the deadline.
pub const MIN_ATTEMPT: Duration = Duration::from_secs(5);

/// One model of one service at one endpoint: what the ledger remembers. `Debug` leaves the
/// endpoint out, which may be the built-in service's. A model on this device has no endpoint.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct QuotaKey {
    /// Recognition or clean-up.
    pub kind: ServiceKind,
    /// Whose model.
    pub provider: ProviderId,
    /// The model id.
    pub model: String,
    /// The endpoint the requests go to: another workspace or account is another quota. Empty for
    /// a model on this device.
    pub url: String,
}

impl std::fmt::Debug for QuotaKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QuotaKey").field("kind", &self.kind).field("provider", &self.provider).field("model", &self.model).finish_non_exhaustive()
    }
}

/// A model the ledger skips: until when (milliseconds since the epoch) and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mark {
    /// When it is tried again.
    pub until_ms: u64,
    /// Why it is skipped.
    pub reason: SwitchReason,
}

/// Which models ran out of quota or failed, and until when (milliseconds since the epoch). Cheap to
/// clone: every clone is the same ledger. Kept in memory only: after a restart every model is tried
/// again.
#[derive(Clone)]
pub struct QuotaLedger {
    inner: Arc<LedgerInner>,
}

struct LedgerInner {
    /// Each skipped model, why and until when.
    marks: Mutex<BTreeMap<QuotaKey, Mark>>,
    /// Bumped on every change; the runtime re-sends the engines' status on it.
    changes: watch::Sender<u64>,
    clock: Box<dyn Fn() -> u64 + Send + Sync>,
}

impl Default for QuotaLedger {
    fn default() -> Self {
        Self::with_clock(wall_clock_ms)
    }
}

impl std::fmt::Debug for QuotaLedger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QuotaLedger").field("marks", &self.inner.marks.lock().len()).finish()
    }
}

fn wall_clock_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)).unwrap_or(0)
}

fn ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

impl QuotaLedger {
    /// An empty ledger on `clock` (milliseconds since the epoch; tests turn it by hand).
    pub fn with_clock(clock: impl Fn() -> u64 + Send + Sync + 'static) -> Self {
        let (changes, _) = watch::channel(0);
        Self { inner: Arc::new(LedgerInner { marks: Mutex::new(BTreeMap::new()), changes, clock: Box::new(clock) }) }
    }

    /// The ledger's time, in milliseconds since the epoch.
    pub fn now_ms(&self) -> u64 {
        (self.inner.clock)()
    }

    /// Why `key` is skipped and until when, while it is; `None` when it is asked as far as the
    /// ledger knows (never skipped, or its time has come).
    pub fn retry_at(&self, key: &QuotaKey) -> Option<Mark> {
        let mark = *self.inner.marks.lock().get(key)?;
        (mark.until_ms > self.now_ms()).then_some(mark)
    }

    /// `key` ran out of quota, or failed, now: it is skipped for [`QUOTA_RETRY_AFTER`] or
    /// [`OUTAGE_RETRY_AFTER`].
    pub fn mark(&self, key: &QuotaKey, reason: SwitchReason) {
        let span = match reason {
            SwitchReason::Quota => QUOTA_RETRY_AFTER,
            SwitchReason::Outage => OUTAGE_RETRY_AFTER,
        };
        self.mark_until(key, self.now_ms().saturating_add(ms(span)), reason);
    }

    /// `key` is skipped until `retry_at_ms` (milliseconds since the epoch) for `reason`: the
    /// built-in service's day starts again at Beijing midnight (docs/dictation.md §27), a service
    /// that limited the requests says when to come back.
    pub fn mark_until(&self, key: &QuotaKey, retry_at_ms: u64, reason: SwitchReason) {
        self.inner.marks.lock().insert(key.clone(), Mark { until_ms: retry_at_ms, reason });
        self.notify();
    }

    /// `key` answered: it is asked again.
    pub fn unmark(&self, key: &QuotaKey) {
        let removed = self.inner.marks.lock().remove(key).is_some();
        if removed {
            self.notify();
        }
    }

    /// Forget every model of `kind` that was skipped (重新检查 on the 语音模型 or AI 模型 page).
    pub fn clear(&self, kind: ServiceKind) {
        self.forget(|key, _| key.kind == kind);
    }

    /// Forget every model of `provider` that was skipped: its key changed, maybe to another account.
    pub fn clear_provider(&self, provider: ProviderId) {
        self.forget(|key, _| key.provider == provider);
    }

    /// Forget the marks whose time has come (the runtime's tick, docs/dictation.md §3.14): the
    /// change re-sends the engines' status, so a model skipped for a minute shows as asked again
    /// once its minute is over.
    pub fn forget_expired(&self) {
        let now = self.now_ms();
        self.forget(|_, mark| mark.until_ms <= now);
    }

    /// A receiver whose value changes whenever the ledger does.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.inner.changes.subscribe()
    }

    fn forget(&self, matches: impl Fn(&QuotaKey, &Mark) -> bool) {
        let removed = {
            let mut marks = self.inner.marks.lock();
            let before = marks.len();
            marks.retain(|key, mark| !matches(key, mark));
            marks.len() != before
        };
        if removed {
            self.notify();
        }
    }

    fn notify(&self) {
        self.inner.changes.send_modify(|n| *n = n.wrapping_add(1));
    }
}

/// One model of a chain: who it is and the port that serves it.
pub struct Link<P: ?Sized> {
    /// The ledger's name for it.
    pub key: QuotaKey,
    /// Its client.
    pub port: Arc<P>,
}

impl<P: ?Sized> Clone for Link<P> {
    fn clone(&self) -> Self {
        Self { key: self.key.clone(), port: self.port.clone() }
    }
}

/// The positions of a chain's `keys` a call goes through, in order: those the ledger does not
/// skip; when it skips them all, the first alone (its quota may be back sooner than the ledger
/// thinks, the service may be back). The status the pages show follows the same order
/// (`ResolvedEngines::status_with`).
pub fn open_order(keys: &[&QuotaKey], ledger: &QuotaLedger) -> Vec<usize> {
    let open: Vec<usize> = (0..keys.len()).filter(|&i| ledger.retry_at(keys[i]).is_none()).collect();
    if open.is_empty() && !keys.is_empty() { vec![0] } else { open }
}

/// The links a call goes through, in order ([`open_order`]).
fn order<P: ?Sized>(links: &[Link<P>], ledger: &QuotaLedger) -> Vec<usize> {
    let keys: Vec<&QuotaKey> = links.iter().map(|link| &link.key).collect();
    open_order(&keys, ledger)
}

/// How a chain moves on (docs/dictation.md §3.14).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChainPolicy {
    /// 服务出错时也改用候补: also move on when a model fails ([`DictationError::is_outage`]), at most
    /// [`MAX_OUTAGE_TRIES`] failing models and `deadline` per request. Off: only a used-up quota
    /// moves on, with neither limit (§3.5).
    pub on_outage: bool,
    /// The whole request's time while `on_outage` is on, from its first attempt.
    pub deadline: Duration,
}

impl ChainPolicy {
    /// A recognition chain ([`ASR_DEADLINE`]).
    pub const fn asr(on_outage: bool) -> Self {
        Self { on_outage, deadline: ASR_DEADLINE }
    }

    /// A clean-up and edit chain ([`REFINE_DEADLINE`]).
    pub const fn refine(on_outage: bool) -> Self {
        Self { on_outage, deadline: REFINE_DEADLINE }
    }
}

/// Which link answered a call along a chain, and why it was not the selected model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Served {
    /// Its position in the chain: 0 is the selected model.
    pub index: usize,
    /// Whose model it is.
    pub provider: ProviderId,
    /// Its model id (the client's own receipt may name it more precisely).
    pub model: String,
    /// Why the selected model did not answer: it was skipped (the ledger's reason) or refused this
    /// request; `None` when it answered.
    pub reason: Option<SwitchReason>,
}

/// The future a [`first_available`] call makes of one port.
pub type LinkCall<'a, T> = Pin<Box<dyn Future<Output = Result<T, DictationError>> + Send + 'a>>;

/// Run `call` along the chain (docs/dictation.md §3.5, §3.14), starting at the first link the
/// ledger does not skip. A link that answers that its quota is used up
/// ([`DictationError::is_quota_exhausted`]) is marked in `ledger` and the next one is asked. With
/// `policy.on_outage`, so is a link that fails ([`DictationError::is_outage`]): it is skipped for
/// [`OUTAGE_RETRY_AFTER`], or as long as it asked; at most [`MAX_OUTAGE_TRIES`] links fail before
/// the last failure is the answer, and the whole request ends at `policy.deadline` from its first
/// attempt — a link that is still at it then is cut off (and skipped like a failed one when it had
/// the whole time), and no link is asked with less than [`MIN_ATTEMPT`] left. Any other error is
/// the answer, and so is a success (which clears the link's mark). Returns the answer and who gave
/// it. With every link refused, the last refusal is returned.
pub async fn first_available<'a, P, T, F>(links: &'a [Link<P>], ledger: &QuotaLedger, policy: ChainPolicy, mut call: F) -> Result<(T, Served), DictationError>
where
    P: ?Sized + Send + Sync,
    F: FnMut(&'a P) -> LinkCall<'a, T>,
{
    let order = order(links, ledger);
    // Why the selected model does not answer: the ledger skips it, or it refuses below.
    let mut passed = order.first().and_then(|&i| skipped_reason(links, ledger, i));
    let deadline = policy.on_outage.then(|| tokio::time::Instant::now() + policy.deadline);
    let (mut failed, mut refused) = (0, None);
    for i in order {
        let link = &links[i];
        let answer = match deadline {
            None => call(link.port.as_ref()).await,
            Some(deadline) => {
                let left = deadline.saturating_duration_since(tokio::time::Instant::now());
                if refused.is_some() && left < MIN_ATTEMPT {
                    tracing::info!(kind = ?link.key.kind, left_ms = ms(left), "too little of the request's time left to ask another model");
                    break;
                }
                match tokio::time::timeout(left, call(link.port.as_ref())).await {
                    Ok(answer) => answer,
                    Err(_) => {
                        tracing::warn!(kind = ?link.key.kind, provider = link.key.provider.as_str(), model = %link.key.model, "the request's time along the models ran out");
                        // It had the whole time and did not answer: the next request skips it.
                        if refused.is_none() {
                            ledger.mark(&link.key, SwitchReason::Outage);
                        }
                        return Err(deadline_passed(link.key.kind));
                    }
                }
            }
        };
        match answer {
            Ok(value) => {
                ledger.unmark(&link.key);
                let reason = if i == 0 { None } else { passed };
                return Ok((value, Served { index: i, provider: link.key.provider, model: link.key.model.clone(), reason }));
            }
            Err(e) if e.is_quota_exhausted() => {
                tracing::info!(kind = ?link.key.kind, provider = link.key.provider.as_str(), model = %link.key.model, "the model's quota is used up; trying the next model");
                // The built-in service's quota is a day's (docs/dictation.md §27): back when it
                // says, else at Beijing midnight; any other is tried again a day later.
                let builtin_day = (link.key.provider == ProviderId::Builtin).then(|| crate::usage::next_beijing_midnight_ms(ledger.now_ms()));
                match e.quota_retry_at().or(builtin_day) {
                    Some(at) => ledger.mark_until(&link.key, at, SwitchReason::Quota),
                    None => ledger.mark(&link.key, SwitchReason::Quota),
                }
                if i == 0 {
                    passed = Some(SwitchReason::Quota);
                }
                refused = Some(e);
            }
            Err(e) if policy.on_outage && e.is_outage() => {
                tracing::info!(kind = ?link.key.kind, provider = link.key.provider.as_str(), model = %link.key.model, error = %e, "the model failed; trying the next model");
                // A service that said how long to wait is left alone that long (at most a day).
                match e.retry_after_ms() {
                    Some(wait) => ledger.mark_until(&link.key, ledger.now_ms().saturating_add(wait.min(ms(QUOTA_RETRY_AFTER))), SwitchReason::Outage),
                    None => ledger.mark(&link.key, SwitchReason::Outage),
                }
                if i == 0 {
                    passed = Some(SwitchReason::Outage);
                }
                failed += 1;
                refused = Some(e);
                if failed >= MAX_OUTAGE_TRIES {
                    break;
                }
            }
            Err(e) => return Err(e),
        }
    }
    Err(refused.unwrap_or_else(|| DictationError::Asr("no model to try".to_owned())))
}

/// What a request whose time ran out along the chain fails with: a timeout, in the client's words.
fn deadline_passed(kind: ServiceKind) -> DictationError {
    let detail = match kind {
        ServiceKind::Asr => "ASR request timed out",
        ServiceKind::Llm => "refine request timed out",
    };
    DictationError::Unreachable { service: kind, cause: super::ports::Outage::Timeout, detail: detail.to_owned() }
}

/// Recognition along a chain (docs/dictation.md §3.5, §3.14). A transcript names the model and the
/// provider that recognised it, and why a fallback model did; the live stream and the warm-up are
/// the first link's the ledger does not skip.
pub struct FallbackTranscriber {
    links: Vec<Link<dyn Transcriber>>,
    ledger: QuotaLedger,
    policy: ChainPolicy,
}

impl FallbackTranscriber {
    /// `links` in order: the selected model first.
    pub fn new(links: Vec<Link<dyn Transcriber>>, ledger: QuotaLedger, policy: ChainPolicy) -> Self {
        Self { links, ledger, policy }
    }

    /// The first link a call goes to, and why it is not the selected model.
    fn first(&self) -> Option<(&Link<dyn Transcriber>, Option<SwitchReason>)> {
        let i = *order(&self.links, &self.ledger).first()?;
        Some((&self.links[i], skipped_reason(&self.links, &self.ledger, i)))
    }
}

/// Why the call starts at link `i` rather than the selected model: the ledger's reason for
/// skipping it; `None` at the selected model.
fn skipped_reason<P: ?Sized>(links: &[Link<P>], ledger: &QuotaLedger, i: usize) -> Option<SwitchReason> {
    if i == 0 {
        return None;
    }
    links.first().and_then(|link| ledger.retry_at(&link.key)).map(|mark| mark.reason)
}

#[async_trait]
impl Transcriber for FallbackTranscriber {
    async fn transcribe(&self, wav: &[u8], language: Option<&str>, glossary: &[String]) -> Result<Transcript, DictationError> {
        self.transcribe_as(wav, language, glossary, &crate::usage::AsrCall::default()).await
    }

    async fn transcribe_as(&self, wav: &[u8], language: Option<&str>, glossary: &[String], call: &crate::usage::AsrCall) -> Result<Transcript, DictationError> {
        let (mut transcript, served) =
            first_available(&self.links, &self.ledger, self.policy, |port| port.transcribe_as(wav, language, glossary, call)).await?;
        if transcript.model.is_none() {
            transcript.model = Some(served.model);
        }
        transcript.provider = Some(served.provider);
        transcript.fallback = served.reason;
        Ok(transcript)
    }

    fn warm(&self, language: Option<&str>) {
        if let Some((link, _)) = self.first() {
            link.port.warm(language);
        }
    }

    fn streaming(&self, glossary: &[String]) -> Option<Arc<dyn StreamingTranscriber>> {
        let (link, reason) = self.first()?;
        let inner = link.port.streaming(glossary)?;
        Some(Arc::new(LinkStream { inner, model: link.key.model.clone(), provider: link.key.provider, reason }))
    }
}

/// A link's stream, whose text names the link's model when the client does not, and the link's
/// provider and why it streams instead of the selected model.
struct LinkStream {
    inner: Arc<dyn StreamingTranscriber>,
    model: String,
    provider: ProviderId,
    reason: Option<SwitchReason>,
}

impl StreamingTranscriber for LinkStream {
    fn open(&self, language: Option<&str>) -> Result<Box<dyn StreamingSession>, DictationError> {
        Ok(Box::new(LinkSession { inner: self.inner.open(language)?, model: self.model.clone(), provider: self.provider, reason: self.reason }))
    }

    fn warm(&self) {
        self.inner.warm();
    }
}

struct LinkSession {
    inner: Box<dyn StreamingSession>,
    model: String,
    provider: ProviderId,
    reason: Option<SwitchReason>,
}

impl StreamingSession for LinkSession {
    fn feed(&mut self, pcm16k: &[f32]) {
        self.inner.feed(pcm16k);
    }

    fn poll(&mut self) -> StreamEvent {
        self.inner.poll()
    }

    fn finish(self: Box<Self>) -> Result<StreamFinal, DictationError> {
        let Self { inner, model, provider, reason } = *self;
        inner.finish().map(|fin| StreamFinal { model: fin.model.or(Some(model)), provider: Some(provider), fallback: reason, ..fin })
    }

    fn model(&self) -> Option<String> {
        self.inner.model().or_else(|| Some(self.model.clone()))
    }

    fn served(&self) -> (Option<ProviderId>, Option<SwitchReason>) {
        (Some(self.provider), self.reason)
    }
}

/// Clean-up and voice edits along a chain (docs/dictation.md §3.5, §3.14); the answer names the
/// model and the provider that wrote it, and why a fallback model did.
pub struct FallbackRefiner {
    links: Vec<Link<dyn Refiner>>,
    ledger: QuotaLedger,
    policy: ChainPolicy,
}

impl FallbackRefiner {
    /// `links` in order: the selected model first.
    pub fn new(links: Vec<Link<dyn Refiner>>, ledger: QuotaLedger, policy: ChainPolicy) -> Self {
        Self { links, ledger, policy }
    }
}

/// `refined` as `served` gave it: a client that names no model is the link's.
fn served_refined((refined, served): (Refined, Served)) -> Refined {
    let model = if refined.model.is_empty() { served.model } else { refined.model };
    Refined { model, provider: Some(served.provider), fallback: served.reason, ..refined }
}

#[async_trait]
impl Refiner for FallbackRefiner {
    async fn refine(&self, text: &str, hints: &RefineHints) -> Result<Refined, DictationError> {
        first_available(&self.links, &self.ledger, self.policy, |port| port.refine(text, hints)).await.map(served_refined)
    }

    async fn edit(&self, selection: &str, instruction: &str, hints: &RefineHints) -> Result<Refined, DictationError> {
        first_available(&self.links, &self.ledger, self.policy, |port| port.edit(selection, instruction, hints)).await.map(served_refined)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::dictation::fakes::{FAKE_REFINE_MODEL, FakeRefiner, FakeStreaming, FakeTranscriber};

    const DAY_MS: u64 = 24 * 60 * 60 * 1000;

    /// 100 ms of a quiet tone, as the fakes want a real WAV.
    fn wav() -> Vec<u8> {
        crate::dictation::wav::encode_pcm16(&[100; 1600], 16_000)
    }

    fn key(kind: ServiceKind, provider: ProviderId, model: &str) -> QuotaKey {
        QuotaKey { kind, provider, model: model.into(), url: "https://<asr-host>/v1".into() }
    }

    fn asr(model: &str) -> QuotaKey {
        key(ServiceKind::Asr, ProviderId::Aliyun, model)
    }

    /// A ledger on a clock the test turns.
    fn ledger() -> (QuotaLedger, Arc<AtomicU64>) {
        let clock = Arc::new(AtomicU64::new(1_000_000));
        let c = clock.clone();
        (QuotaLedger::with_clock(move || c.load(Ordering::SeqCst)), clock)
    }

    fn link(model: &str, port: &Arc<FakeTranscriber>) -> Link<dyn Transcriber> {
        Link { key: asr(model), port: port.clone() as Arc<dyn Transcriber> }
    }

    #[tokio::test]
    async fn a_used_up_model_hands_the_call_to_the_next_and_is_skipped_for_a_day() {
        let (ledger, clock) = ledger();
        let (first, second) = (Arc::new(FakeTranscriber::quota()), Arc::new(FakeTranscriber::ok("第二个模型")));
        let t = FallbackTranscriber::new(vec![link("a", &first), link("b", &second)], ledger.clone(), ChainPolicy::asr(false));
        let out = t.transcribe(&wav(), Some("zh"), &[]).await.unwrap();
        assert_eq!((out.text.as_str(), out.model.as_deref()), ("第二个模型", Some("b")), "the receipt names the link that answered");
        assert_eq!((first.calls(), second.calls()), (1, 1));
        assert_eq!(ledger.retry_at(&asr("a")).map(|m| m.until_ms), Some(1_000_000 + DAY_MS));
        assert_eq!(ledger.retry_at(&asr("b")), None);
        // Skipped while marked: no second refused request.
        t.transcribe(&wav(), None, &[]).await.unwrap();
        assert_eq!((first.calls(), second.calls()), (1, 2));
        // A day later it is tried again (and refuses again).
        clock.fetch_add(DAY_MS, Ordering::SeqCst);
        assert_eq!(ledger.retry_at(&asr("a")), None);
        t.transcribe(&wav(), None, &[]).await.unwrap();
        assert_eq!((first.calls(), second.calls()), (2, 3));
        assert_eq!(ledger.retry_at(&asr("a")).map(|m| m.until_ms), Some(1_000_000 + 2 * DAY_MS));
    }

    #[tokio::test]
    async fn with_every_model_out_of_quota_only_the_first_is_asked_again() {
        let (ledger, _) = ledger();
        let (first, second) = (Arc::new(FakeTranscriber::quota()), Arc::new(FakeTranscriber::quota()));
        let t = FallbackTranscriber::new(vec![link("a", &first), link("b", &second)], ledger.clone(), ChainPolicy::asr(false));
        let err = t.transcribe(&wav(), None, &[]).await.unwrap_err();
        assert!(err.is_quota_exhausted(), "{err:?}");
        assert_eq!((first.calls(), second.calls()), (1, 1));
        let err = t.transcribe(&wav(), None, &[]).await.unwrap_err();
        assert!(err.is_quota_exhausted());
        assert_eq!((first.calls(), second.calls()), (2, 1), "both marked: the selected model alone, once");
    }

    #[tokio::test]
    async fn any_other_failure_is_the_answer() {
        let (ledger, _) = ledger();
        let (first, second) = (Arc::new(FakeTranscriber::err("ASR server error 500")), Arc::new(FakeTranscriber::ok("x")));
        let t = FallbackTranscriber::new(vec![link("a", &first), link("b", &second)], ledger.clone(), ChainPolicy::asr(false));
        assert_eq!(t.transcribe(&wav(), None, &[]).await.unwrap_err(), DictationError::Asr("ASR server error 500".into()));
        assert_eq!(second.calls(), 0, "only a used-up quota moves on");
        assert_eq!(ledger.retry_at(&asr("a")), None);
        // An empty chain says so instead of hanging.
        let empty: Vec<Link<dyn Transcriber>> = Vec::new();
        let err = first_available(&empty, &ledger, ChainPolicy::asr(true), |port| port.transcribe(b"", None, &[])).await.unwrap_err();
        assert!(matches!(err, DictationError::Asr(_)));
    }

    #[tokio::test]
    async fn an_answer_clears_the_mark_and_a_reported_model_is_kept() {
        let (ledger, clock) = ledger();
        ledger.mark(&asr("a"), SwitchReason::Quota);
        clock.fetch_add(DAY_MS + 1, Ordering::SeqCst);
        let first = Arc::new(FakeTranscriber::ok("回来了").with_model("qwen-audio-3.1-asr-flash-2026-09-01"));
        let t = FallbackTranscriber::new(vec![link("a", &first)], ledger.clone(), ChainPolicy::asr(false));
        let out = t.transcribe(&wav(), None, &[]).await.unwrap();
        assert_eq!(out.model.as_deref(), Some("qwen-audio-3.1-asr-flash-2026-09-01"), "the client's own receipt wins");
        assert_eq!(format!("{ledger:?}"), "QuotaLedger { marks: 0 }");
    }

    #[test]
    fn clearing_forgets_one_service_or_one_provider_and_notifies_only_on_a_change() {
        let (ledger, _) = ledger();
        let mut changes = ledger.subscribe();
        ledger.unmark(&asr("a"));
        ledger.clear(ServiceKind::Asr);
        ledger.clear_provider(ProviderId::Aliyun);
        assert!(!changes.has_changed().unwrap(), "nothing to forget, nothing to say");
        ledger.mark(&asr("a"), SwitchReason::Quota);
        ledger.mark(&key(ServiceKind::Llm, ProviderId::Aliyun, "qwen3.8-flash"), SwitchReason::Quota);
        ledger.mark(&key(ServiceKind::Llm, ProviderId::Openai, "gpt-5.5-mini"), SwitchReason::Quota);
        assert!(changes.has_changed().unwrap());
        changes.borrow_and_update();
        ledger.clear(ServiceKind::Asr);
        assert!(changes.has_changed().unwrap());
        assert!(ledger.retry_at(&asr("a")).is_none());
        assert!(ledger.retry_at(&key(ServiceKind::Llm, ProviderId::Aliyun, "qwen3.8-flash")).is_some());
        changes.borrow_and_update();
        ledger.clear_provider(ProviderId::Aliyun);
        assert!(changes.has_changed().unwrap());
        assert!(ledger.retry_at(&key(ServiceKind::Llm, ProviderId::Aliyun, "qwen3.8-flash")).is_none());
        assert!(ledger.retry_at(&key(ServiceKind::Llm, ProviderId::Openai, "gpt-5.5-mini")).is_some(), "another provider's mark stays");
        // The endpoint is part of the key, and never of its `Debug`.
        assert!(!format!("{:?}", asr("a")).contains("asr-host"));
        let elsewhere = QuotaKey { url: "https://<other-host>/v1".into(), ..asr("a") };
        ledger.mark(&asr("a"), SwitchReason::Quota);
        assert!(ledger.retry_at(&elsewhere).is_none());
        assert!(ledger.now_ms() > 0 && QuotaLedger::default().now_ms() > 1_600_000_000_000, "the default clock is the wall clock");
    }

    /// docs/dictation.md §27: the built-in service's day is skipped until it says the quota is back
    /// (its report's reset), or else until Beijing midnight; any other service is tried again a
    /// day later, as before. The chain hands each model the call it was given.
    #[tokio::test]
    async fn a_used_up_built_in_day_is_skipped_until_its_reset() {
        let (ledger, _) = ledger();
        let builtin = |model: &str| key(ServiceKind::Asr, ProviderId::Builtin, model);
        let reset = 1_000_000 + 3 * 60 * 60 * 1000;
        let (said, unsaid) = (Arc::new(FakeTranscriber::quota_until(reset)), Arc::new(FakeTranscriber::quota()));
        let other = Arc::new(FakeTranscriber::quota());
        let next = Arc::new(FakeTranscriber::ok("候补"));
        let links: Vec<Link<dyn Transcriber>> = vec![
            Link { key: builtin("said"), port: said.clone() as Arc<dyn Transcriber> },
            Link { key: builtin("unsaid"), port: unsaid.clone() as Arc<dyn Transcriber> },
            link("other", &other),
            link("next", &next),
        ];
        let t = FallbackTranscriber::new(links, ledger.clone(), ChainPolicy::asr(false));
        let call = crate::usage::AsrCall::take(Some(crate::usage::UsageOrigin::Phone { name: "Pixel 8".into() }));
        t.transcribe_as(&wav(), None, &[], &call).await.unwrap();
        assert_eq!(ledger.retry_at(&builtin("said")).map(|m| m.until_ms), Some(reset), "back when the service said");
        assert_eq!(
            ledger.retry_at(&builtin("unsaid")).map(|m| m.until_ms),
            Some(crate::usage::next_beijing_midnight_ms(1_000_000)),
            "else at Beijing midnight"
        );
        assert_eq!(ledger.retry_at(&asr("other")).map(|m| m.until_ms), Some(1_000_000 + DAY_MS), "any other service: a day");
        assert_eq!(next.usage_calls(), std::slice::from_ref(&call), "the call reached the model that answered");
        assert_eq!(said.usage_calls(), [call]);
        let manual = asr("manual");
        ledger.mark_until(&manual, 1_000_000, SwitchReason::Quota);
        assert_eq!(ledger.retry_at(&manual), None, "a time already reached is no skip");
    }

    #[tokio::test]
    async fn the_stream_and_the_warm_up_are_the_first_link_with_quota_left() {
        let (ledger, _) = ledger();
        let first = Arc::new(FakeTranscriber::ok("a").with_stream(Arc::new(FakeStreaming::script().with_model("流式一号"))));
        let second = Arc::new(FakeTranscriber::ok("b").with_stream(Arc::new(FakeStreaming::script())));
        let third = Arc::new(FakeTranscriber::ok("c"));
        let t = FallbackTranscriber::new(vec![link("a", &first), link("b", &second), link("c", &third)], ledger.clone(), ChainPolicy::asr(false));
        let session = t.streaming(&["Voltip".into()]).unwrap().open(None).unwrap();
        assert_eq!(session.model().as_deref(), Some("流式一号"), "the client's own receipt");
        assert_eq!(first.stream_glossaries(), vec![vec!["Voltip".to_owned()]]);
        t.warm(Some("zh"));
        assert_eq!(first.warms(), vec![Some("zh".to_owned())]);
        ledger.mark(&asr("a"), SwitchReason::Quota);
        let stream = t.streaming(&[]).unwrap();
        stream.warm();
        let mut session = stream.open(None).unwrap();
        assert_eq!(session.model().as_deref(), Some("b"), "the link's model when the session cannot say");
        session.feed(&[0.1; 1600]);
        assert!(matches!(session.poll(), StreamEvent::Partial { .. }));
        assert_eq!(session.finish().unwrap().model.as_deref(), Some("b"), "and with the flush");
        t.warm(None);
        assert_eq!(second.warms(), vec![None]);
        ledger.mark(&asr("b"), SwitchReason::Quota);
        assert!(t.streaming(&[]).is_none(), "the next model with quota takes whole recordings only");
    }

    #[tokio::test]
    async fn the_clean_up_and_the_edit_go_along_the_chain_too() {
        let (ledger, _) = ledger();
        let refine_key = |model: &str| key(ServiceKind::Llm, ProviderId::Aliyun, model);
        let first = Arc::new(FakeRefiner::quota());
        let second = Arc::new(FakeRefiner::ok("润色后的文字。").with_model("qwen3.8-flash"));
        let links: Vec<Link<dyn Refiner>> = vec![
            Link { key: refine_key("a"), port: first.clone() as Arc<dyn Refiner> },
            Link { key: refine_key("qwen3.8-flash"), port: second.clone() as Arc<dyn Refiner> },
        ];
        let r = FallbackRefiner::new(links, ledger.clone(), ChainPolicy::refine(false));
        let out = r.refine("原文", &RefineHints::default()).await.unwrap();
        assert_eq!((out.text.as_str(), out.model.as_str()), ("润色后的文字。", "qwen3.8-flash"));
        let edited = r.edit("选中的文字", "改正式", &RefineHints::default()).await.unwrap();
        assert_eq!(edited.model, "qwen3.8-flash");
        assert_eq!((first.calls(), second.calls()), (1, 1), "the edit skipped the used-up model");
        assert_eq!(second.edits().len(), 1);
        assert!(ledger.retry_at(&refine_key("a")).is_some());
        assert_ne!(FAKE_REFINE_MODEL, "qwen3.8-flash");
    }

    // ---- 服务出错时也改用候补 (docs/dictation.md §3.14) ----------------------------------------

    use crate::dictation::{LocalFailure, Outage};

    /// [`ledger`], under a name a test's own `ledger` does not shadow.
    fn fresh() -> (QuotaLedger, Arc<AtomicU64>) {
        ledger()
    }

    fn unreachable(cause: Outage, detail: &str) -> DictationError {
        DictationError::Unreachable { service: ServiceKind::Asr, cause, detail: detail.into() }
    }

    fn local_failed(failure: LocalFailure) -> DictationError {
        DictationError::LocalFailed { failure, detail: format!("本地识别失败：{failure}") }
    }

    /// The failures that pass: a timeout, the network, a server error, a busy or rate-limited
    /// service, on this device the compute device, its memory or the model.
    fn outages() -> Vec<DictationError> {
        vec![
            unreachable(Outage::Timeout, "ASR request timed out"),
            unreachable(Outage::Network, "ASR network error: connection reset"),
            unreachable(Outage::Server(500), "ASR server error 500"),
            DictationError::Overloaded { service: ServiceKind::Asr, detail: "ASR server error 503".into(), retry_after_ms: None },
            DictationError::RateLimited { service: ServiceKind::Asr, detail: "ASR rate limited".into(), retry_after_ms: None },
            local_failed(LocalFailure::Device),
            local_failed(LocalFailure::Memory),
            local_failed(LocalFailure::Model),
        ]
    }

    /// The failures that never move on: a refused key or request, nothing heard, a cancel, an
    /// input too long, anything an engine does not classify.
    fn refusals() -> Vec<DictationError> {
        vec![
            DictationError::Asr("ASR rejected the credentials: Invalid API Key (invalid_api_key)".into()),
            DictationError::Asr("ASR server error 400: unsupported audio".into()),
            DictationError::NoSpeech,
            local_failed(LocalFailure::Aborted),
            local_failed(LocalFailure::InputTooLong),
            local_failed(LocalFailure::Other),
        ]
    }

    fn chain(ports: &[&Arc<FakeTranscriber>], ledger: &QuotaLedger, on_outage: bool) -> FallbackTranscriber {
        let links = ports.iter().enumerate().map(|(i, port)| link(&format!("m{i}"), port)).collect();
        FallbackTranscriber::new(links, ledger.clone(), ChainPolicy::asr(on_outage))
    }

    /// With the switch off, a failure is the answer as before, whatever its kind; nothing is
    /// marked, and a slow model is waited for: no deadline.
    #[tokio::test(start_paused = true)]
    async fn with_the_switch_off_only_a_used_up_quota_moves_on() {
        for failure in outages() {
            let (ledger, _) = fresh();
            let (first, second) = (Arc::new(FakeTranscriber::failing(failure.clone())), Arc::new(FakeTranscriber::ok("候补")));
            let t = chain(&[&first, &second], &ledger, false);
            assert_eq!(t.transcribe(&wav(), None, &[]).await.unwrap_err(), failure);
            assert_eq!(second.calls(), 0, "{failure:?}");
            assert_eq!(ledger.retry_at(&asr("m0")), None, "{failure:?}");
        }
        let (ledger, _) = fresh();
        let (slow, second) = (Arc::new(FakeTranscriber::slow("慢慢识别", Duration::from_secs(400))), Arc::new(FakeTranscriber::ok("候补")));
        let started = tokio::time::Instant::now();
        let out = chain(&[&slow, &second], &ledger, false).transcribe(&wav(), None, &[]).await.unwrap();
        assert_eq!((out.text.as_str(), started.elapsed()), ("慢慢识别", Duration::from_secs(400)));
        assert_eq!(second.calls(), 0);
    }

    /// With the switch on, every failure that passes hands the call to the next model, which
    /// says it stood in for a failure; a refused key, a refused request, silence, a cancel never
    /// do, and mark nothing.
    #[tokio::test(start_paused = true)]
    async fn with_the_switch_on_a_failure_moves_on_and_a_refusal_does_not() {
        for failure in outages() {
            let (ledger, _) = fresh();
            let (first, second) = (Arc::new(FakeTranscriber::failing(failure.clone())), Arc::new(FakeTranscriber::ok("候补")));
            let out = chain(&[&first, &second], &ledger, true).transcribe(&wav(), None, &[]).await.unwrap();
            assert_eq!(
                (out.text.as_str(), out.model.as_deref(), out.provider, out.fallback),
                ("候补", Some("m1"), Some(ProviderId::Aliyun), Some(SwitchReason::Outage)),
                "{failure:?}"
            );
            assert_eq!(ledger.retry_at(&asr("m0")).map(|m| m.reason), Some(SwitchReason::Outage), "{failure:?}");
        }
        for refusal in refusals() {
            let (ledger, _) = fresh();
            let (first, second) = (Arc::new(FakeTranscriber::failing(refusal.clone())), Arc::new(FakeTranscriber::ok("候补")));
            assert_eq!(chain(&[&first, &second], &ledger, true).transcribe(&wav(), None, &[]).await.unwrap_err(), refusal);
            assert_eq!(second.calls(), 0, "{refusal:?}");
            assert_eq!(ledger.retry_at(&asr("m0")), None, "{refusal:?}");
        }
    }

    /// A model that failed is skipped for a minute, or as long as a 429 asked; the next requests
    /// go to the next model and say why. A used-up quota is marked as such.
    #[tokio::test(start_paused = true)]
    async fn a_failed_model_is_skipped_for_a_minute_or_as_long_as_it_asked() {
        let (ledger, clock) = fresh();
        let (first, second) =
            (Arc::new(FakeTranscriber::failing(unreachable(Outage::Timeout, "ASR request timed out"))), Arc::new(FakeTranscriber::ok("候补")));
        let t = chain(&[&first, &second], &ledger, true);
        t.transcribe(&wav(), None, &[]).await.unwrap();
        assert_eq!(ledger.retry_at(&asr("m0")), Some(Mark { until_ms: 1_060_000, reason: SwitchReason::Outage }));
        let out = t.transcribe(&wav(), None, &[]).await.unwrap();
        assert_eq!((first.calls(), second.calls()), (1, 2), "skipped while it cools down");
        assert_eq!(out.fallback, Some(SwitchReason::Outage), "the next request says why the selected model did not answer");
        clock.fetch_add(60_000, Ordering::SeqCst);
        t.transcribe(&wav(), None, &[]).await.unwrap();
        assert_eq!((first.calls(), second.calls()), (2, 3), "a minute later it is asked again");
        assert_eq!(ledger.retry_at(&asr("m0")).map(|m| m.until_ms), Some(1_120_000));

        let (ledger, _) = fresh();
        let limited =
            DictationError::RateLimited { service: ServiceKind::Asr, detail: "ASR rate limited (retry after 355000 ms)".into(), retry_after_ms: Some(355_000) };
        let (first, second) = (Arc::new(FakeTranscriber::failing(limited)), Arc::new(FakeTranscriber::ok("候补")));
        chain(&[&first, &second], &ledger, true).transcribe(&wav(), None, &[]).await.unwrap();
        assert_eq!(ledger.retry_at(&asr("m0")), Some(Mark { until_ms: 1_355_000, reason: SwitchReason::Outage }), "as long as the service asked");

        let (ledger, _) = fresh();
        let (first, second) = (Arc::new(FakeTranscriber::quota()), Arc::new(FakeTranscriber::ok("候补")));
        let out = chain(&[&first, &second], &ledger, true).transcribe(&wav(), None, &[]).await.unwrap();
        assert_eq!(ledger.retry_at(&asr("m0")).map(|m| m.reason), Some(SwitchReason::Quota));
        assert_eq!(out.fallback, Some(SwitchReason::Quota));
    }

    /// With every model skipped, the selected one alone is asked, once, at once: no waiting.
    #[tokio::test(start_paused = true)]
    async fn with_every_model_skipped_only_the_first_is_asked_once() {
        let (ledger, _) = fresh();
        ledger.mark(&asr("m0"), SwitchReason::Outage);
        ledger.mark(&asr("m1"), SwitchReason::Quota);
        let failure = unreachable(Outage::Network, "ASR network error: dns");
        let (first, second) = (Arc::new(FakeTranscriber::failing(failure.clone())), Arc::new(FakeTranscriber::ok("候补")));
        let started = tokio::time::Instant::now();
        assert_eq!(chain(&[&first, &second], &ledger, true).transcribe(&wav(), None, &[]).await.unwrap_err(), failure);
        assert_eq!((first.calls(), second.calls(), started.elapsed()), (1, 0, Duration::ZERO));
    }

    /// At most three models that fail are asked in one request; a used-up quota's refusal is not
    /// counted.
    #[tokio::test(start_paused = true)]
    async fn at_most_three_failing_models_are_asked() {
        let (ledger, _) = fresh();
        let failing: Vec<Arc<FakeTranscriber>> =
            (0..4).map(|i| Arc::new(FakeTranscriber::failing(unreachable(Outage::Server(500 + i), &format!("ASR server error {}", 500 + i))))).collect();
        let ok = Arc::new(FakeTranscriber::ok("第五个"));
        let t = chain(&[&failing[0], &failing[1], &failing[2], &failing[3], &ok], &ledger, true);
        assert_eq!(t.transcribe(&wav(), None, &[]).await.unwrap_err(), unreachable(Outage::Server(502), "ASR server error 502"), "the third's");
        assert_eq!(failing.iter().map(|f| f.calls()).collect::<Vec<_>>(), [1, 1, 1, 0]);
        assert_eq!(ok.calls(), 0);

        let (ledger, _) = fresh();
        let (quota, a, b) = (
            Arc::new(FakeTranscriber::quota()),
            Arc::new(FakeTranscriber::failing(local_failed(LocalFailure::Device))),
            Arc::new(FakeTranscriber::failing(unreachable(Outage::Timeout, "ASR request timed out"))),
        );
        let ok = Arc::new(FakeTranscriber::ok("第四个"));
        let out = chain(&[&quota, &a, &b, &ok], &ledger, true).transcribe(&wav(), None, &[]).await.unwrap();
        assert_eq!(out.text, "第四个", "four models asked: the used-up quota is not a failure");
    }

    /// A model that never answers is cut at the request's deadline, which is the whole request's:
    /// with less than five seconds left no further model is asked, and the next model only gets
    /// what remains. The clean-up's deadline is a minute.
    #[tokio::test(start_paused = true)]
    async fn a_model_that_never_answers_is_cut_at_the_deadline() {
        let hour = Duration::from_secs(3600);
        let (ledger, _) = fresh();
        let (hung, second) = (Arc::new(FakeTranscriber::never()), Arc::new(FakeTranscriber::ok("候补")));
        let started = tokio::time::Instant::now();
        let err = tokio::time::timeout(hour, chain(&[&hung, &second], &ledger, true).transcribe(&wav(), None, &[]))
            .await
            .expect("the chain's own deadline")
            .unwrap_err();
        assert!(matches!(&err, DictationError::Unreachable { service: ServiceKind::Asr, cause: Outage::Timeout, .. }), "{err:?}");
        assert_eq!(err.to_string(), "asr: ASR request timed out", "it reads as a timeout");
        assert_eq!(started.elapsed(), ASR_DEADLINE);
        assert_eq!(second.calls(), 0, "no time was left for it");
        assert_eq!(ledger.retry_at(&asr("m0")).map(|m| m.reason), Some(SwitchReason::Outage), "it had the whole time: the next request skips it");

        let (ledger, _) = fresh();
        let late = unreachable(Outage::Server(500), "ASR server error 500");
        let (slow, second) = (Arc::new(FakeTranscriber::slow_failing(late.clone(), Duration::from_secs(146))), Arc::new(FakeTranscriber::ok("候补")));
        assert_eq!(chain(&[&slow, &second], &ledger, true).transcribe(&wav(), None, &[]).await.unwrap_err(), late);
        assert_eq!(second.calls(), 0, "four seconds left: not enough for another model");

        let (ledger, _) = fresh();
        let (slow, hung) = (Arc::new(FakeTranscriber::slow_failing(late, Duration::from_secs(100))), Arc::new(FakeTranscriber::never()));
        let started = tokio::time::Instant::now();
        let err = tokio::time::timeout(hour, chain(&[&slow, &hung], &ledger, true).transcribe(&wav(), None, &[])).await.unwrap().unwrap_err();
        assert!(matches!(err, DictationError::Unreachable { cause: Outage::Timeout, .. }), "{err:?}");
        assert_eq!((hung.calls(), started.elapsed()), (1, ASR_DEADLINE), "the second model had the fifty seconds left");
        assert_eq!(ledger.retry_at(&asr("m1")), None, "cut off with part of the time: not held against it");

        let (ledger, _) = fresh();
        let refine_key = |model: &str| key(ServiceKind::Llm, ProviderId::Aliyun, model);
        let (hung, next) = (Arc::new(FakeRefiner::never()), Arc::new(FakeRefiner::ok("润色")));
        let links: Vec<Link<dyn Refiner>> =
            vec![Link { key: refine_key("a"), port: hung.clone() as Arc<dyn Refiner> }, Link { key: refine_key("b"), port: next.clone() as Arc<dyn Refiner> }];
        let started = tokio::time::Instant::now();
        let r = FallbackRefiner::new(links, ledger.clone(), ChainPolicy::refine(true));
        let err = tokio::time::timeout(hour, r.refine("原文", &RefineHints::default())).await.unwrap().unwrap_err();
        assert!(matches!(&err, DictationError::Unreachable { service: ServiceKind::Llm, cause: Outage::Timeout, .. }), "{err:?}");
        assert_eq!((err.to_string(), started.elapsed()), ("refine: refine request timed out".to_owned(), REFINE_DEADLINE));
    }

    /// The answer says which link gave it and why the selected model did not: the clean-up's and
    /// the stream's too.
    #[tokio::test]
    async fn the_answer_says_who_gave_it_and_why() {
        let audio = wav();
        let (ledger, _) = fresh();
        let (first, second) = (Arc::new(FakeTranscriber::quota()), Arc::new(FakeTranscriber::ok("x")));
        let links = vec![link("a", &first), link("b", &second)];
        let (_, served) =
            first_available(&links, &ledger, ChainPolicy::asr(true), |port| port.transcribe(&audio, None, &[])).await.unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(served, Served { index: 1, provider: ProviderId::Aliyun, model: "b".into(), reason: Some(SwitchReason::Quota) });
        let (ledger, _) = fresh();
        let links = vec![link("b", &second)];
        let (_, served) =
            first_available(&links, &ledger, ChainPolicy::asr(true), |port| port.transcribe(&audio, None, &[])).await.unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(served.reason, None, "the selected model answered");

        let (ledger, _) = fresh();
        let refine_key = |model: &str| key(ServiceKind::Llm, ProviderId::Openai, model);
        let failing = Arc::new(FakeRefiner::failing(DictationError::Unreachable {
            service: ServiceKind::Llm,
            cause: Outage::Network,
            detail: "refine network error: reset".into(),
        }));
        let next = Arc::new(FakeRefiner::ok("润色后。"));
        let links: Vec<Link<dyn Refiner>> =
            vec![Link { key: refine_key("a"), port: failing as Arc<dyn Refiner> }, Link { key: refine_key("b"), port: next as Arc<dyn Refiner> }];
        let out = FallbackRefiner::new(links, ledger, ChainPolicy::refine(true)).refine("原文", &RefineHints::default()).await.unwrap();
        assert_eq!((out.provider, out.fallback), (Some(ProviderId::Openai), Some(SwitchReason::Outage)));

        let (ledger, _) = fresh();
        let first = Arc::new(FakeTranscriber::ok("a").with_stream(Arc::new(FakeStreaming::script())));
        let second = Arc::new(FakeTranscriber::ok("b").with_stream(Arc::new(FakeStreaming::script())));
        let t = FallbackTranscriber::new(vec![link("a", &first), link("b", &second)], ledger.clone(), ChainPolicy::asr(true));
        let session = t.streaming(&[]).unwrap().open(None).unwrap();
        assert_eq!(session.served(), (Some(ProviderId::Aliyun), None));
        ledger.mark(&asr("a"), SwitchReason::Outage);
        let mut session = t.streaming(&[]).unwrap().open(None).unwrap();
        assert_eq!(session.served(), (Some(ProviderId::Aliyun), Some(SwitchReason::Outage)), "the fallback's stream, for a failure");
        session.feed(&[0.1; 1600]);
        let fin = session.finish().unwrap();
        assert_eq!((fin.model.as_deref(), fin.provider, fin.fallback), (Some("b"), Some(ProviderId::Aliyun), Some(SwitchReason::Outage)));
    }

    /// The runtime's tick forgets the marks whose time has come: the change re-sends the status.
    #[test]
    fn the_marks_whose_time_has_come_are_forgotten() {
        let (ledger, clock) = fresh();
        ledger.mark(&asr("a"), SwitchReason::Outage);
        ledger.mark(&asr("b"), SwitchReason::Quota);
        let mut changes = ledger.subscribe();
        changes.borrow_and_update();
        ledger.forget_expired();
        assert!(!changes.has_changed().unwrap(), "nothing expired yet");
        clock.fetch_add(60_000, Ordering::SeqCst);
        ledger.forget_expired();
        assert!(changes.has_changed().unwrap());
        assert_eq!(format!("{ledger:?}"), "QuotaLedger { marks: 1 }", "the day's quota mark stays");
    }
}
