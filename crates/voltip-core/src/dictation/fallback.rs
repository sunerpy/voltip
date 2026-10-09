//! Fallback models (docs/dictation.md §3.5): the selected model first, then the fallback models,
//! tried in order when a model's quota is used up — and only then. Nothing here knows a provider
//! or a port: a [`QuotaLedger`] remembers which models ran out and when, [`first_with_quota`] runs
//! one call along a chain of [`Link`]s, and [`FallbackTranscriber`] / [`FallbackRefiner`] put a
//! chain behind the recognition and clean-up ports. The engine owns the ledger: it outlives the
//! clients, which are rebuilt on every configuration change.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::watch;

use super::ports::{DictationError, RefineHints, Refined, Refiner, StreamEvent, StreamFinal, StreamingSession, StreamingTranscriber, Transcriber, Transcript};
use crate::providers::{ProviderId, ServiceKind};

/// How long a model whose quota ran out is skipped. Then it is tried once more: a refused request
/// is not billed, and the quota may be back (some of Model Studio's reset on the 1st of the month,
/// or the owner topped the account up).
pub const QUOTA_RETRY_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

/// One model of one service at one endpoint: what the ledger remembers. `Debug` leaves the
/// endpoint out, which may be the built-in service's.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct QuotaKey {
    /// Recognition or clean-up.
    pub kind: ServiceKind,
    /// Whose model.
    pub provider: ProviderId,
    /// The model id.
    pub model: String,
    /// The endpoint the requests go to: another workspace or account is another quota.
    pub url: String,
}

impl std::fmt::Debug for QuotaKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QuotaKey").field("kind", &self.kind).field("provider", &self.provider).field("model", &self.model).finish_non_exhaustive()
    }
}

/// Which models ran out of quota, and when (milliseconds since the epoch). Cheap to clone: every
/// clone is the same ledger. Kept in memory only: after a restart every model is tried again.
#[derive(Clone)]
pub struct QuotaLedger {
    inner: Arc<LedgerInner>,
}

struct LedgerInner {
    marks: Mutex<BTreeMap<QuotaKey, u64>>,
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

fn retry_after_ms() -> u64 {
    u64::try_from(QUOTA_RETRY_AFTER.as_millis()).unwrap_or(u64::MAX)
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

    /// When `key` is tried again, while it is skipped; `None` when it has quota as far as the
    /// ledger knows (never ran out, or [`QUOTA_RETRY_AFTER`] has passed).
    pub fn retry_at(&self, key: &QuotaKey) -> Option<u64> {
        let at = *self.inner.marks.lock().get(key)?;
        let retry = at.saturating_add(retry_after_ms());
        (retry > self.now_ms()).then_some(retry)
    }

    /// `key` ran out of quota now.
    pub fn mark(&self, key: &QuotaKey) {
        let now = self.now_ms();
        self.inner.marks.lock().insert(key.clone(), now);
        self.notify();
    }

    /// `key` answered: it has quota again.
    pub fn unmark(&self, key: &QuotaKey) {
        let removed = self.inner.marks.lock().remove(key).is_some();
        if removed {
            self.notify();
        }
    }

    /// Forget every model of `kind` that ran out (重新检查 on the 语音模型 or AI 模型 page).
    pub fn clear(&self, kind: ServiceKind) {
        self.forget(|key| key.kind == kind);
    }

    /// Forget every model of `provider` that ran out: its key changed, maybe to another account.
    pub fn clear_provider(&self, provider: ProviderId) {
        self.forget(|key| key.provider == provider);
    }

    /// A receiver whose value changes whenever the ledger does.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.inner.changes.subscribe()
    }

    fn forget(&self, matches: impl Fn(&QuotaKey) -> bool) {
        let removed = {
            let mut marks = self.inner.marks.lock();
            let before = marks.len();
            marks.retain(|key, _| !matches(key));
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

/// The positions of a chain's `keys` a call goes through, in order: those with quota left; when
/// none has, the first alone (its quota may be back sooner than the ledger thinks). The status the
/// pages show follows the same order (`ResolvedEngines::status_with`).
pub fn open_order(keys: &[&QuotaKey], ledger: &QuotaLedger) -> Vec<usize> {
    let open: Vec<usize> = (0..keys.len()).filter(|&i| ledger.retry_at(keys[i]).is_none()).collect();
    if open.is_empty() && !keys.is_empty() { vec![0] } else { open }
}

/// The links a call goes through, in order ([`open_order`]).
fn order<P: ?Sized>(links: &[Link<P>], ledger: &QuotaLedger) -> Vec<usize> {
    let keys: Vec<&QuotaKey> = links.iter().map(|link| &link.key).collect();
    open_order(&keys, ledger)
}

/// The future a [`first_with_quota`] call makes of one port.
pub type LinkCall<'a, T> = Pin<Box<dyn Future<Output = Result<T, DictationError>> + Send + 'a>>;

/// Run `call` on the first link with quota left. A link that answers that its quota is used up
/// ([`DictationError::is_quota_exhausted`]) is marked in `ledger` and the next one is tried; any
/// other error is the answer, and so is a success (which clears the link's mark). Returns the
/// answer and the index of the link that gave it. With every link out of quota the last refusal
/// is returned.
pub async fn first_with_quota<'a, P, T, F>(links: &'a [Link<P>], ledger: &QuotaLedger, mut call: F) -> Result<(T, usize), DictationError>
where
    P: ?Sized + Send + Sync,
    F: FnMut(&'a P) -> LinkCall<'a, T>,
{
    let mut refused = None;
    for i in order(links, ledger) {
        let link = &links[i];
        match call(link.port.as_ref()).await {
            Ok(value) => {
                ledger.unmark(&link.key);
                return Ok((value, i));
            }
            Err(e) if e.is_quota_exhausted() => {
                tracing::info!(kind = ?link.key.kind, provider = link.key.provider.as_str(), model = %link.key.model, "the model's quota is used up; trying the next model");
                ledger.mark(&link.key);
                refused = Some(e);
            }
            Err(e) => return Err(e),
        }
    }
    Err(refused.unwrap_or_else(|| DictationError::Asr("no model to try".to_owned())))
}

/// Recognition along a chain (docs/dictation.md §3.5). A transcript names the model that
/// recognised it; the live stream and the warm-up are the first link's with quota left.
pub struct FallbackTranscriber {
    links: Vec<Link<dyn Transcriber>>,
    ledger: QuotaLedger,
}

impl FallbackTranscriber {
    /// `links` in order: the selected model first.
    pub fn new(links: Vec<Link<dyn Transcriber>>, ledger: QuotaLedger) -> Self {
        Self { links, ledger }
    }

    fn first(&self) -> Option<&Link<dyn Transcriber>> {
        order(&self.links, &self.ledger).first().map(|&i| &self.links[i])
    }
}

#[async_trait]
impl Transcriber for FallbackTranscriber {
    async fn transcribe(&self, wav: &[u8], language: Option<&str>, glossary: &[String]) -> Result<Transcript, DictationError> {
        let (mut transcript, i) = first_with_quota(&self.links, &self.ledger, |port| port.transcribe(wav, language, glossary)).await?;
        if transcript.model.is_none() {
            transcript.model = Some(self.links[i].key.model.clone());
        }
        Ok(transcript)
    }

    fn warm(&self, language: Option<&str>) {
        if let Some(link) = self.first() {
            link.port.warm(language);
        }
    }

    fn streaming(&self, glossary: &[String]) -> Option<Arc<dyn StreamingTranscriber>> {
        let link = self.first()?;
        let inner = link.port.streaming(glossary)?;
        Some(Arc::new(LinkStream { inner, model: link.key.model.clone() }))
    }
}

/// A link's stream, whose text names the link's model when the client does not.
struct LinkStream {
    inner: Arc<dyn StreamingTranscriber>,
    model: String,
}

impl StreamingTranscriber for LinkStream {
    fn open(&self, language: Option<&str>) -> Result<Box<dyn StreamingSession>, DictationError> {
        Ok(Box::new(LinkSession { inner: self.inner.open(language)?, model: self.model.clone() }))
    }

    fn warm(&self) {
        self.inner.warm();
    }
}

struct LinkSession {
    inner: Box<dyn StreamingSession>,
    model: String,
}

impl StreamingSession for LinkSession {
    fn feed(&mut self, pcm16k: &[f32]) {
        self.inner.feed(pcm16k);
    }

    fn poll(&mut self) -> StreamEvent {
        self.inner.poll()
    }

    fn finish(self: Box<Self>) -> Result<StreamFinal, DictationError> {
        let Self { inner, model } = *self;
        inner.finish().map(|fin| StreamFinal { model: fin.model.or(Some(model)), ..fin })
    }

    fn model(&self) -> Option<String> {
        self.inner.model().or_else(|| Some(self.model.clone()))
    }
}

/// Clean-up and voice edits along a chain (docs/dictation.md §3.5); the answer names the model
/// that wrote it.
pub struct FallbackRefiner {
    links: Vec<Link<dyn Refiner>>,
    ledger: QuotaLedger,
}

impl FallbackRefiner {
    /// `links` in order: the selected model first.
    pub fn new(links: Vec<Link<dyn Refiner>>, ledger: QuotaLedger) -> Self {
        Self { links, ledger }
    }
}

#[async_trait]
impl Refiner for FallbackRefiner {
    async fn refine(&self, text: &str, hints: &RefineHints) -> Result<Refined, DictationError> {
        first_with_quota(&self.links, &self.ledger, |port| port.refine(text, hints)).await.map(|(refined, _)| refined)
    }

    async fn edit(&self, selection: &str, instruction: &str, hints: &RefineHints) -> Result<Refined, DictationError> {
        first_with_quota(&self.links, &self.ledger, |port| port.edit(selection, instruction, hints)).await.map(|(refined, _)| refined)
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
        let t = FallbackTranscriber::new(vec![link("a", &first), link("b", &second)], ledger.clone());
        let out = t.transcribe(&wav(), Some("zh"), &[]).await.unwrap();
        assert_eq!((out.text.as_str(), out.model.as_deref()), ("第二个模型", Some("b")), "the receipt names the link that answered");
        assert_eq!((first.calls(), second.calls()), (1, 1));
        assert_eq!(ledger.retry_at(&asr("a")), Some(1_000_000 + DAY_MS));
        assert_eq!(ledger.retry_at(&asr("b")), None);
        // Skipped while marked: no second refused request.
        t.transcribe(&wav(), None, &[]).await.unwrap();
        assert_eq!((first.calls(), second.calls()), (1, 2));
        // A day later it is tried again (and refuses again).
        clock.fetch_add(DAY_MS, Ordering::SeqCst);
        assert_eq!(ledger.retry_at(&asr("a")), None);
        t.transcribe(&wav(), None, &[]).await.unwrap();
        assert_eq!((first.calls(), second.calls()), (2, 3));
        assert_eq!(ledger.retry_at(&asr("a")), Some(1_000_000 + 2 * DAY_MS));
    }

    #[tokio::test]
    async fn with_every_model_out_of_quota_only_the_first_is_asked_again() {
        let (ledger, _) = ledger();
        let (first, second) = (Arc::new(FakeTranscriber::quota()), Arc::new(FakeTranscriber::quota()));
        let t = FallbackTranscriber::new(vec![link("a", &first), link("b", &second)], ledger.clone());
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
        let t = FallbackTranscriber::new(vec![link("a", &first), link("b", &second)], ledger.clone());
        assert_eq!(t.transcribe(&wav(), None, &[]).await.unwrap_err(), DictationError::Asr("ASR server error 500".into()));
        assert_eq!(second.calls(), 0, "only a used-up quota moves on");
        assert_eq!(ledger.retry_at(&asr("a")), None);
        // An empty chain says so instead of hanging.
        let empty: Vec<Link<dyn Transcriber>> = Vec::new();
        let err = first_with_quota(&empty, &ledger, |port| port.transcribe(b"", None, &[])).await.unwrap_err();
        assert!(matches!(err, DictationError::Asr(_)));
    }

    #[tokio::test]
    async fn an_answer_clears_the_mark_and_a_reported_model_is_kept() {
        let (ledger, clock) = ledger();
        ledger.mark(&asr("a"));
        clock.fetch_add(DAY_MS + 1, Ordering::SeqCst);
        let first = Arc::new(FakeTranscriber::ok("回来了").with_model("qwen-audio-3.1-asr-flash-2026-09-01"));
        let t = FallbackTranscriber::new(vec![link("a", &first)], ledger.clone());
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
        ledger.mark(&asr("a"));
        ledger.mark(&key(ServiceKind::Llm, ProviderId::Aliyun, "qwen3.8-flash"));
        ledger.mark(&key(ServiceKind::Llm, ProviderId::Openai, "gpt-5.5-mini"));
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
        ledger.mark(&asr("a"));
        assert!(ledger.retry_at(&elsewhere).is_none());
        assert!(ledger.now_ms() > 0 && QuotaLedger::default().now_ms() > 1_600_000_000_000, "the default clock is the wall clock");
    }

    #[tokio::test]
    async fn the_stream_and_the_warm_up_are_the_first_link_with_quota_left() {
        let (ledger, _) = ledger();
        let first = Arc::new(FakeTranscriber::ok("a").with_stream(Arc::new(FakeStreaming::script().with_model("流式一号"))));
        let second = Arc::new(FakeTranscriber::ok("b").with_stream(Arc::new(FakeStreaming::script())));
        let third = Arc::new(FakeTranscriber::ok("c"));
        let t = FallbackTranscriber::new(vec![link("a", &first), link("b", &second), link("c", &third)], ledger.clone());
        let session = t.streaming(&["Voltip".into()]).unwrap().open(None).unwrap();
        assert_eq!(session.model().as_deref(), Some("流式一号"), "the client's own receipt");
        assert_eq!(first.stream_glossaries(), vec![vec!["Voltip".to_owned()]]);
        t.warm(Some("zh"));
        assert_eq!(first.warms(), vec![Some("zh".to_owned())]);
        ledger.mark(&asr("a"));
        let stream = t.streaming(&[]).unwrap();
        stream.warm();
        let mut session = stream.open(None).unwrap();
        assert_eq!(session.model().as_deref(), Some("b"), "the link's model when the session cannot say");
        session.feed(&[0.1; 1600]);
        assert!(matches!(session.poll(), StreamEvent::Partial { .. }));
        assert_eq!(session.finish().unwrap().model.as_deref(), Some("b"), "and with the flush");
        t.warm(None);
        assert_eq!(second.warms(), vec![None]);
        ledger.mark(&asr("b"));
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
        let r = FallbackRefiner::new(links, ledger.clone());
        let out = r.refine("原文", &RefineHints::default()).await.unwrap();
        assert_eq!((out.text.as_str(), out.model.as_str()), ("润色后的文字。", "qwen3.8-flash"));
        let edited = r.edit("选中的文字", "改正式", &RefineHints::default()).await.unwrap();
        assert_eq!(edited.model, "qwen3.8-flash");
        assert_eq!((first.calls(), second.calls()), (1, 1), "the edit skipped the used-up model");
        assert_eq!(second.edits().len(), 1);
        assert!(ledger.retry_at(&refine_key("a")).is_some());
        assert_ne!(FAKE_REFINE_MODEL, "qwen3.8-flash");
    }
}
