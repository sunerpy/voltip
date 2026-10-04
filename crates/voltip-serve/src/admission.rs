//! How many takes are processed at once (docs/dictation.md §23.4): `concurrency` hold a permit,
//! twice as many wait for one without their body being read, and the rest are told to come back.
//! A permit is held by the task that processes the take, until it ends, also after its client went
//! away: a recognition already running cannot be stopped, and the limit counts it.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::openai::ApiError;

/// The permits and the queue.
#[derive(Debug)]
pub struct Admission {
    permits: Arc<Semaphore>,
    waiting: AtomicUsize,
    max_waiting: usize,
}

/// One place in the queue, given back when the wait ends either way.
struct Waiting<'a>(&'a AtomicUsize);

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Admission {
    /// `concurrency` permits and a queue twice as long.
    pub fn new(concurrency: usize) -> Self {
        let concurrency = concurrency.max(1);
        Self { permits: Arc::new(Semaphore::new(concurrency)), waiting: AtomicUsize::new(0), max_waiting: concurrency * 2 }
    }

    /// A permit now, after waiting in the queue, or 503 when the queue is full too.
    pub async fn admit(&self) -> Result<OwnedSemaphorePermit, ApiError> {
        if let Ok(permit) = self.permits.clone().try_acquire_owned() {
            return Ok(permit);
        }
        if self.waiting.fetch_add(1, Ordering::SeqCst) >= self.max_waiting {
            self.waiting.fetch_sub(1, Ordering::SeqCst);
            return Err(ApiError::unavailable("server_busy", "正在处理的请求已满，请稍后重试"));
        }
        let _place = Waiting(&self.waiting);
        self.permits.clone().acquire_owned().await.map_err(|_| ApiError::unavailable("shutting_down", "服务正在停止"))
    }

    /// Refuse every request from now on, the waiting ones too (the service stops); the takes
    /// holding a permit go on.
    pub fn close(&self) {
        self.permits.close();
    }

    /// Permits not taken.
    pub fn available(&self) -> usize {
        self.permits.available_permits()
    }

    /// Requests waiting for a permit.
    pub fn waiting(&self) -> usize {
        self.waiting.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_full_queue_is_refused_and_a_freed_permit_lets_the_next_in() {
        let admission = Arc::new(Admission::new(1));
        let first = admission.admit().await.unwrap();
        let (a, b) = (admission.clone(), admission.clone());
        let second = tokio::spawn(async move { a.admit().await.map(|_| ()) });
        let third = tokio::spawn(async move { b.admit().await.map(|_| ()) });
        while admission.waiting() < 2 {
            tokio::task::yield_now().await;
        }
        let refused = admission.admit().await.unwrap_err();
        assert_eq!((refused.status.as_u16(), refused.code, refused.retry_after), (503, "server_busy", Some(5)));
        drop(first);
        second.await.unwrap().unwrap();
        third.await.unwrap().unwrap();
        assert_eq!((admission.available(), admission.waiting()), (1, 0));
    }
}
