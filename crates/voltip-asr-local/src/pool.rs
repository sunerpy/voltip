//! The recognisers in memory (docs/dictation.md §29): a small pool per transcriber, shared by its
//! clones.
//!
//! A take leases an instance of the model, language and compute it needs ([`Key`]) and gives it
//! back when it is done. An instance a cancelled take still holds is busy until the piece in
//! progress ends: neither engine stops a run once it has begun (transcribe.cpp 0.4.1 looks at a
//! cancel only before a Qwen3-ASR run). So sherpa-onnx may have two instances ([`cap`]), and a
//! take that finds every instance busy waits at most its lock wait instead of queueing behind them. One model at a time: an idle
//! instance of another model is dropped before a new one loads, and a busy one is waited for.
//! A model failure makes every instance of the model unusable ([`Pool::evict`]): the idle ones go
//! at once, the busy ones when they come back.

use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::{Condvar, Mutex};

use crate::catalogue::Engine;
use crate::compute::{Compute, LocalDevice};
use crate::transcriber::Recognizer;

/// How long a take waits for a recogniser before it gives up (docs/dictation.md §29.8, measured):
/// twice the slowest recognition of a 121 s input on the two machines measured, rounded up to 5 s.
/// The slowest is Qwen3-ASR 1.7B on the Windows build machine's CPU, 75.3 s, estimated from its
/// Linux time and 0.6B's Windows / Linux ratio (D17: 1.7B is measured on Linux only). A take
/// recognised in pieces of at most `max_piece` takes less than one inference over all of it, so a
/// take gives up on a recogniser that is stuck, not on one that is merely busy.
pub const LOCK_WAIT: Duration = Duration::from_secs(155);

/// Two GPU failures this close together turn the GPU off until Voltip restarts.
pub const GPU_STRIKE_WINDOW: Duration = Duration::from_secs(10 * 60);

/// Instances of one model that may be in memory at once: one for transcribe.cpp, whose models are
/// large (a cancelled take frees it after the piece in progress, at most `max_piece` of audio);
/// two for sherpa-onnx, so the take after a cancelled one runs on a second instance at once.
pub fn cap(engine: Engine) -> usize {
    match engine {
        Engine::TranscribeCpp => 1,
        Engine::SenseVoice | Engine::Paraformer | Engine::ZipformerStreaming | Engine::SileroVad => 2,
    }
}

/// What an instance was loaded for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Key {
    /// The catalogue id.
    pub id: String,
    /// The language the recogniser was told.
    pub language: Option<String>,
    /// Where it runs.
    pub compute: Compute,
}

/// A recogniser in memory.
pub struct Loaded {
    pub(crate) key: Key,
    /// The pool's generation when it was loaded: an eviction since makes it unusable.
    generation: u64,
    pub(crate) recognizer: Box<dyn Recognizer>,
}

#[derive(Default)]
struct State {
    idle: Vec<Loaded>,
    /// Instances lent out, by key.
    busy: Vec<Key>,
    /// Loads under way, by key.
    loading: Vec<Key>,
    /// Bumped by [`Pool::evict`] and [`Pool::clear`].
    generation: u64,
}

impl State {
    fn others_live(&self, key: &Key) -> bool {
        self.busy.iter().chain(&self.loading).any(|k| k != key)
    }

    fn live(&self, key: &Key) -> usize {
        self.busy.iter().chain(&self.loading).filter(|k| *k == key).count()
    }

    fn take_idle(&mut self, key: &Key) -> Option<Loaded> {
        let at = self.idle.iter().position(|l| &l.key == key)?;
        self.busy.push(key.clone());
        Some(self.idle.remove(at))
    }

    /// The idle instances of other keys, taken out to be dropped outside the lock.
    fn drain_others(&mut self, key: &Key) -> Vec<Loaded> {
        let (others, same) = std::mem::take(&mut self.idle).into_iter().partition(|l| &l.key != key);
        self.idle = same;
        others
    }

    fn remove_one(list: &mut Vec<Key>, key: &Key) {
        if let Some(at) = list.iter().position(|k| k == key) {
            list.remove(at);
        }
    }
}

/// What a lease attempt got.
pub enum Grant {
    /// An idle instance, now lent to the caller.
    Ready(Loaded),
    /// The right to load one (the caller loads, then [`Pool::loaded`] or [`Pool::load_failed`]).
    Load,
}

/// The pool; see the module documentation.
#[derive(Default)]
pub struct Pool {
    state: Mutex<State>,
    changed: Condvar,
}

impl Pool {
    /// Lend an idle instance of `key` or the right to load one (at most `cap` of the key live,
    /// none of another key), waiting until `deadline` for an instance to come back; `None` when
    /// none did.
    pub fn acquire(&self, key: &Key, cap: usize, deadline: Instant) -> Option<Grant> {
        let mut state = self.state.lock();
        loop {
            if let Some(loaded) = state.take_idle(key) {
                return Some(Grant::Ready(loaded));
            }
            // A load of this key under way is waited for, not doubled; another model is waited
            // for to finish (one model in memory).
            if !state.loading.contains(key) && !state.others_live(key) && state.live(key) < cap {
                state.loading.push(key.clone());
                let others = state.drain_others(key);
                drop(state);
                drop(others);
                return Some(Grant::Load);
            }
            if self.changed.wait_until(&mut state, deadline).timed_out() {
                return None;
            }
        }
    }

    /// Load `key` once nothing else is in memory, unless an instance of it is there or on its way
    /// already (a warm-up); waits for another model's busy instances, and gives up when
    /// `superseded` says a newer request wants something else.
    pub fn reserve_load(&self, key: &Key, superseded: impl Fn() -> bool) -> bool {
        let mut state = self.state.lock();
        loop {
            if superseded() || state.idle.iter().any(|l| &l.key == key) || state.live(key) > 0 {
                return false;
            }
            if !state.others_live(key) {
                state.loading.push(key.clone());
                let others = state.drain_others(key);
                drop(state);
                drop(others);
                return true;
            }
            self.changed.wait(&mut state);
        }
    }

    /// A load [`Pool::acquire`] granted worked: the instance is lent to the caller.
    pub fn loaded(&self, key: Key, recognizer: Box<dyn Recognizer>) -> Loaded {
        let mut state = self.state.lock();
        State::remove_one(&mut state.loading, &key);
        state.busy.push(key.clone());
        let loaded = Loaded { key, generation: state.generation, recognizer };
        self.changed.notify_all();
        loaded
    }

    /// A load [`Pool::reserve_load`] reserved worked: the instance waits in the pool.
    pub fn stock(&self, key: Key, recognizer: Box<dyn Recognizer>) {
        let mut state = self.state.lock();
        State::remove_one(&mut state.loading, &key);
        let loaded = Loaded { key, generation: state.generation, recognizer };
        state.idle.push(loaded);
        self.changed.notify_all();
    }

    /// A load that was granted failed: its place is free again.
    pub fn load_failed(&self, key: &Key) {
        let mut state = self.state.lock();
        State::remove_one(&mut state.loading, key);
        self.changed.notify_all();
    }

    /// `loaded` comes back: kept for the next take when `keep` and no eviction happened since it
    /// was loaded, dropped (outside the lock) otherwise.
    pub fn give_back(&self, loaded: Loaded, keep: bool) {
        let mut state = self.state.lock();
        State::remove_one(&mut state.busy, &loaded.key);
        let dropped = if keep && loaded.generation == state.generation {
            state.idle.push(loaded);
            None
        } else {
            Some(loaded)
        };
        self.changed.notify_all();
        drop(state);
        drop(dropped);
    }

    /// Every instance of `id` is unusable (a model failure): the idle ones are dropped now, the
    /// busy ones when they come back.
    pub fn evict(&self, id: &str) {
        let mut state = self.state.lock();
        state.generation += 1;
        let (gone, kept) = std::mem::take(&mut state.idle).into_iter().partition(|l| l.key.id == id);
        state.idle = kept;
        self.changed.notify_all();
        drop(state);
        drop::<Vec<Loaded>>(gone);
    }

    /// Drop every instance: the idle ones now, the busy ones when they come back.
    pub fn clear(&self) {
        let mut state = self.state.lock();
        state.generation += 1;
        let gone = std::mem::take(&mut state.idle);
        self.changed.notify_all();
        drop(state);
        drop(gone);
    }

    /// The model in memory: an idle instance's, else a busy one's.
    pub fn model(&self) -> Option<String> {
        let state = self.state.lock();
        state.idle.first().map(|l| l.key.id.clone()).or_else(|| state.busy.first().map(|k| k.id.clone()))
    }

    /// The backend an idle instance runs on.
    pub fn idle_backend(&self) -> Option<String> {
        self.state.lock().idle.first().map(|l| l.recognizer.backend())
    }

    /// Whether an idle instance of `key` waits.
    pub fn has_idle(&self, key: &Key) -> bool {
        self.state.lock().idle.iter().any(|l| &l.key == key)
    }

    /// Instances in memory (idle and busy) and loads under way.
    pub fn counts(&self) -> (usize, usize, usize) {
        let state = self.state.lock();
        (state.idle.len(), state.busy.len(), state.loading.len())
    }
}

/// An instance lent to one recognition; given back when dropped, kept unless
/// [`Lease::discard`] was called (or a panic unwinds through it).
pub struct Lease {
    pool: Arc<Pool>,
    loaded: Option<Loaded>,
    keep: bool,
}

impl Lease {
    /// `loaded`, lent from `pool`.
    pub fn new(pool: Arc<Pool>, loaded: Loaded) -> Self {
        Self { pool, loaded: Some(loaded), keep: true }
    }

    /// The recogniser.
    pub fn recognizer(&mut self) -> Option<&mut (dyn Recognizer + 'static)> {
        self.loaded.as_mut().map(|l| l.recognizer.as_mut())
    }

    /// The backend it runs on.
    pub fn backend(&self) -> String {
        self.loaded.as_ref().map_or_else(String::new, |l| l.recognizer.backend())
    }

    /// Do not keep it: the instance failed.
    pub fn discard(&mut self) {
        self.keep = false;
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(loaded) = self.loaded.take() {
            self.pool.give_back(loaded, self.keep && !std::thread::panicking());
        }
    }
}

/// Whether the GPU is still trusted (docs/dictation.md §29): a GPU failure is retried on the CPU,
/// and a second one within [`GPU_STRIKE_WINDOW`] keeps the local models on the CPU until Voltip
/// restarts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuHealth {
    last_failure: Option<Instant>,
    off: bool,
}

impl GpuHealth {
    /// A GPU failure at `now`; whether the GPU is off from now on.
    pub fn failed(&mut self, now: Instant) -> bool {
        if self.last_failure.is_some_and(|last| now.saturating_duration_since(last) <= GPU_STRIKE_WINDOW) {
            self.off = true;
        }
        self.last_failure = Some(now);
        self.off
    }

    /// Whether the GPU is off until restart.
    pub fn is_off(&self) -> bool {
        self.off
    }

    /// Where a recogniser asked for on `compute` runs: on the CPU once the GPU is off.
    pub fn compute(&self, compute: &Compute) -> Compute {
        if self.off && compute.device != LocalDevice::Cpu { Compute { device: LocalDevice::Cpu, gpu: None, ..compute.clone() } } else { compute.clone() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcriber::{Abort, RecognizeError};

    struct Nothing;

    impl Recognizer for Nothing {
        fn transcribe(&mut self, _sample_rate: u32, _samples: &[f32], _abort: &Abort) -> Result<String, RecognizeError> {
            Ok(String::new())
        }
    }

    fn key(id: &str) -> Key {
        Key { id: id.to_owned(), language: None, compute: Compute::default() }
    }

    fn soon() -> Instant {
        Instant::now() + Duration::from_millis(50)
    }

    fn lend(pool: &Pool, key: &Key, cap: usize) -> Loaded {
        match pool.acquire(key, cap, soon()) {
            Some(Grant::Ready(loaded)) => loaded,
            Some(Grant::Load) => pool.loaded(key.clone(), Box::new(Nothing)),
            None => panic!("nothing for {key:?}"),
        }
    }

    #[test]
    fn instances_are_reused_capped_and_given_back() {
        let pool = Pool::default();
        let a = key("a");
        let first = lend(&pool, &a, 2);
        assert_eq!(pool.counts(), (0, 1, 0));
        // The first is busy: a second may load (cap 2), a third has to wait and gives up.
        let second = lend(&pool, &a, 2);
        assert!(pool.acquire(&a, 2, soon()).is_none(), "both busy");
        pool.give_back(first, true);
        assert!(matches!(pool.acquire(&a, 2, soon()), Some(Grant::Ready(_))), "the one given back");
        pool.give_back(second, false);
        assert_eq!(pool.counts(), (0, 1, 0), "the discarded one is gone; the reused one is lent");
        // A load under way is waited for, not doubled.
        let pool = Pool::default();
        assert!(matches!(pool.acquire(&a, 2, soon()), Some(Grant::Load)));
        assert!(pool.acquire(&a, 2, soon()).is_none(), "the load under way is waited for");
        pool.load_failed(&a);
        assert!(matches!(pool.acquire(&a, 2, soon()), Some(Grant::Load)), "its place is free again");
    }

    #[test]
    fn one_model_at_a_time() {
        let pool = Pool::default();
        let (a, b) = (key("a"), key("b"));
        let lent = lend(&pool, &a, 2);
        assert!(pool.acquire(&b, 2, soon()).is_none(), "another model's busy instance is waited for");
        pool.give_back(lent, true);
        assert!(pool.has_idle(&a));
        assert!(matches!(pool.acquire(&b, 2, soon()), Some(Grant::Load)), "the idle one of another model makes way");
        assert_eq!(pool.counts(), (0, 0, 1));
        assert!(!pool.has_idle(&a));
    }

    #[test]
    fn an_eviction_drops_the_idle_instances_now_and_the_busy_ones_on_return() {
        let pool = Pool::default();
        let a = key("a");
        let (one, two) = (lend(&pool, &a, 2), lend(&pool, &a, 2));
        pool.give_back(one, true);
        pool.evict("a");
        assert_eq!(pool.counts(), (0, 1, 0));
        pool.give_back(two, true);
        assert_eq!(pool.counts(), (0, 0, 0), "loaded before the eviction: dropped");
        let fresh = lend(&pool, &a, 2);
        pool.give_back(fresh, true);
        assert_eq!(pool.counts(), (1, 0, 0), "loaded after it: kept");
        assert_eq!(pool.model().as_deref(), Some("a"));
        assert_eq!(pool.idle_backend().as_deref(), Some("CPU"));
        pool.clear();
        assert_eq!((pool.counts(), pool.model()), ((0, 0, 0), None));
    }

    #[test]
    fn a_warm_up_loads_only_what_is_not_there() {
        let pool = Pool::default();
        let (a, b) = (key("a"), key("b"));
        assert!(pool.reserve_load(&a, || false));
        assert!(!pool.reserve_load(&a, || false), "on its way already");
        pool.stock(a.clone(), Box::new(Nothing));
        assert!(!pool.reserve_load(&a, || false), "there already");
        assert!(!pool.reserve_load(&b, || true), "superseded");
        assert!(pool.reserve_load(&b, || false), "another model: the idle one makes way");
        assert_eq!(pool.counts(), (0, 0, 1));
    }

    #[test]
    fn a_second_gpu_failure_within_ten_minutes_keeps_the_cpu_until_restart() {
        let t0 = Instant::now();
        let mut gpu = GpuHealth::default();
        let wanted = Compute { device: LocalDevice::Gpu, gpu: Some("Vulkan0".into()), threads: Some(4) };
        assert_eq!(gpu.compute(&wanted), wanted);
        assert!(!gpu.failed(t0), "one failure: retried on the CPU, the GPU stays");
        assert!(!gpu.failed(t0 + GPU_STRIKE_WINDOW + Duration::from_secs(1)), "eleven minutes later: still one strike");
        assert!(gpu.failed(t0 + GPU_STRIKE_WINDOW + Duration::from_secs(2)), "two within ten minutes");
        assert!(gpu.is_off());
        assert_eq!(gpu.compute(&wanted), Compute { device: LocalDevice::Cpu, gpu: None, threads: Some(4) });
        let cpu = Compute { device: LocalDevice::Cpu, ..Compute::default() };
        assert_eq!(gpu.compute(&cpu), cpu);
        assert_eq!((cap(Engine::TranscribeCpp), cap(Engine::SenseVoice), cap(Engine::Paraformer)), (1, 2, 2));
    }
}
