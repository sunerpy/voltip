//! The level meter on the phone (docs/dictation.md §20). The phone opens no microphone just to meter
//! it: a subscription forwards the core's level stream, which carries frames while one of the
//! phone's takes records (the take's own capture), and stays quiet otherwise. The Tauri shell's
//! `meter.rs` on tokio instead of Tauri's runtime.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::broadcast::{self, error::RecvError};
use tokio::task::JoinHandle;
use voltip_core::dictation::LevelFrame;

/// The running meter subscriptions.
#[derive(Default)]
pub struct Meters {
    next: AtomicU64,
    tasks: Mutex<HashMap<u64, JoinHandle<()>>>,
}

impl Meters {
    /// Forward `levels` to `sink` on `runtime` until [`Meters::stop`], the end of the stream, or a
    /// `sink` that returns `false` (nobody listens any more). Returns the subscription id.
    pub fn start(
        &self,
        runtime: &tokio::runtime::Handle,
        mut levels: broadcast::Receiver<LevelFrame>,
        sink: impl Fn(LevelFrame) -> bool + Send + 'static,
    ) -> u64 {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let task = runtime.spawn(async move {
            loop {
                match levels.recv().await {
                    Ok(frame) if sink(frame) => {}
                    Ok(_) | Err(RecvError::Closed) => break,
                    Err(RecvError::Lagged(_)) => {}
                }
            }
        });
        self.tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(id, task);
        id
    }

    /// End subscription `id`; an unknown id is ignored (a page may stop twice).
    pub fn stop(&self, id: u64) {
        if let Some(task) = self.tasks.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&id) {
            task.abort();
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;

    fn frame(seq: u64) -> LevelFrame {
        LevelFrame { rms_dbfs: -30.0, peak_dbfs: -12.0, clipping: false, sample_rate_hz: 48_000, channels: 1, seq }
    }

    async fn until(what: &str, done: impl Fn() -> bool) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !done() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
    }

    #[tokio::test]
    async fn a_subscription_forwards_the_takes_levels_until_it_is_stopped() {
        let (tx, _) = broadcast::channel(16);
        let meters = Meters::default();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let id = meters.start(&tokio::runtime::Handle::current(), tx.subscribe(), move |f| {
            sink.lock().unwrap().push(f.seq);
            true
        });
        tx.send(frame(0)).unwrap();
        tx.send(frame(1)).unwrap();
        until("two frames", || seen.lock().unwrap().len() == 2).await;
        meters.stop(id);
        meters.stop(id);
        until("the task to end", || tx.receiver_count() == 0).await;
        assert!(tx.send(frame(2)).is_err(), "nobody listens after stop");
        assert_eq!(*seen.lock().unwrap(), [0, 1]);
    }

    /// A listener that went away (its sink refuses) ends the subscription by itself.
    #[tokio::test]
    async fn a_refusing_sink_ends_the_subscription() {
        let (tx, _) = broadcast::channel(16);
        let meters = Meters::default();
        let runtime = tokio::runtime::Handle::current();
        let first = meters.start(&runtime, tx.subscribe(), |_| false);
        let second = meters.start(&runtime, tx.subscribe(), |_| true);
        assert_ne!(first, second);
        tx.send(frame(0)).unwrap();
        until("the refused subscription to end", || tx.receiver_count() == 1).await;
        meters.stop(second);
        until("the second one to end", || tx.receiver_count() == 0).await;
    }
}
