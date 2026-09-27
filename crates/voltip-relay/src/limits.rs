//! Fixed-window rate limiting keyed by an arbitrary label (IP address in practice).

use std::collections::HashMap;
use std::hash::Hash;
use std::time::{Duration, Instant};

/// A window's parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RateWindow {
    /// Events allowed per window.
    pub limit: u32,
    /// Window length.
    pub per: Duration,
}

impl RateWindow {
    /// `limit` events every `per`.
    pub const fn new(limit: u32, per: Duration) -> Self {
        Self { limit, per }
    }
}

#[derive(Debug, Clone, Copy)]
struct Bucket {
    window_start: Instant,
    count: u32,
}

/// Fixed-window counter per key. Old keys are swept opportunistically on every check so the
/// map cannot grow without bound under a scan.
#[derive(Debug)]
pub struct RateLimiter<K: Hash + Eq + Clone> {
    window: RateWindow,
    buckets: HashMap<K, Bucket>,
    last_sweep: Option<Instant>,
}

impl<K: Hash + Eq + Clone> RateLimiter<K> {
    /// New limiter.
    pub fn new(window: RateWindow) -> Self {
        Self { window, buckets: HashMap::new(), last_sweep: None }
    }

    /// Record one event for `key`. `Ok(())` if allowed, `Err(retry_after)` otherwise.
    pub fn check(&mut self, key: K, now: Instant) -> Result<(), Duration> {
        self.sweep(now);
        let bucket = self.buckets.entry(key).or_insert(Bucket { window_start: now, count: 0 });
        if now.duration_since(bucket.window_start) >= self.window.per {
            *bucket = Bucket { window_start: now, count: 0 };
        }
        if bucket.count >= self.window.limit {
            let retry = self.window.per.saturating_sub(now.duration_since(bucket.window_start));
            return Err(retry.max(Duration::from_secs(1)));
        }
        bucket.count += 1;
        Ok(())
    }

    /// Number of tracked keys (for metrics / tests).
    pub fn tracked(&self) -> usize {
        self.buckets.len()
    }

    fn sweep(&mut self, now: Instant) {
        let due = self.last_sweep.is_none_or(|t| now.duration_since(t) >= self.window.per);
        if !due {
            return;
        }
        let per = self.window.per;
        self.buckets.retain(|_, b| now.duration_since(b.window_start) < per);
        self.last_sweep = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_up_to_limit_then_blocks_until_window_rolls() {
        let mut l = RateLimiter::new(RateWindow::new(3, Duration::from_secs(60)));
        let t0 = Instant::now();
        for _ in 0..3 {
            assert!(l.check("ip", t0).is_ok());
        }
        let retry = l.check("ip", t0 + Duration::from_secs(10)).unwrap_err();
        assert_eq!(retry, Duration::from_secs(50));
        // A different key is independent.
        assert!(l.check("other", t0).is_ok());
        assert_eq!(l.tracked(), 2);
        // Window rolls over.
        assert!(l.check("ip", t0 + Duration::from_secs(60)).is_ok());
    }

    #[test]
    fn retry_after_is_at_least_one_second_and_stale_keys_are_swept() {
        let mut l = RateLimiter::new(RateWindow::new(1, Duration::from_secs(5)));
        let t0 = Instant::now();
        l.check("a", t0).unwrap();
        let retry = l.check("a", t0 + Duration::from_millis(4_900)).unwrap_err();
        assert_eq!(retry, Duration::from_secs(1));
        // After a full window with no activity the key is gone once another check triggers a sweep.
        l.check("b", t0 + Duration::from_secs(11)).unwrap();
        assert_eq!(l.tracked(), 1);
    }
}
