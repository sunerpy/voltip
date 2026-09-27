//! Exponential backoff with jitter.

use std::time::Duration;

/// `delay(n) = min(max, base · 2^n) ± jitter%`, capped by `max_attempts` (None = forever).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReconnectPolicy {
    /// First delay.
    pub base: Duration,
    /// Longest delay.
    pub max: Duration,
    /// Jitter as a fraction of the delay (0.2 = ±20 %).
    pub jitter: f64,
    /// Give up after this many consecutive failures; `None` retries forever.
    pub max_attempts: Option<u32>,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self { base: Duration::from_millis(500), max: Duration::from_secs(30), jitter: 0.2, max_attempts: None }
    }
}

impl ReconnectPolicy {
    /// Policy that never retries (pairing sessions: a dropped socket is a failed pairing).
    pub fn never() -> Self {
        Self { max_attempts: Some(0), ..Self::default() }
    }

    /// Delay before attempt number `attempt` (1-based). `None` means give up.
    pub fn delay_for(&self, attempt: u32) -> Option<Duration> {
        if let Some(max) = self.max_attempts
            && attempt > max
        {
            return None;
        }
        let exp = attempt.saturating_sub(1).min(20);
        let raw = self.base.saturating_mul(1u32 << exp).min(self.max);
        Some(self.jittered(raw))
    }

    fn jittered(&self, d: Duration) -> Duration {
        if self.jitter <= 0.0 {
            return d;
        }
        use rand::Rng as _;
        let factor = 1.0 + rand::rng().random_range(-self.jitter..=self.jitter);
        d.mul_f64(factor.max(0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grows_exponentially_and_caps() {
        let p = ReconnectPolicy { jitter: 0.0, ..Default::default() };
        assert_eq!(p.delay_for(1), Some(Duration::from_millis(500)));
        assert_eq!(p.delay_for(2), Some(Duration::from_secs(1)));
        assert_eq!(p.delay_for(3), Some(Duration::from_secs(2)));
        assert_eq!(p.delay_for(7), Some(Duration::from_secs(30)), "capped at max");
        assert_eq!(p.delay_for(200), Some(Duration::from_secs(30)), "no overflow for huge attempts");
    }

    #[test]
    fn jitter_stays_within_bounds_and_limits_apply() {
        let p = ReconnectPolicy::default();
        for _ in 0..100 {
            let d = p.delay_for(1).unwrap();
            assert!(d >= Duration::from_millis(400) && d <= Duration::from_millis(600), "{d:?}");
        }
        let limited = ReconnectPolicy { max_attempts: Some(2), ..Default::default() };
        assert!(limited.delay_for(2).is_some());
        assert!(limited.delay_for(3).is_none());
        assert!(ReconnectPolicy::never().delay_for(1).is_none());
    }
}
