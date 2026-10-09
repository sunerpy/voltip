//! Bounded set of ticket nonces already consumed by this device — a re-scanned QR code is a
//! replay even if the relay would still accept the session.

use std::collections::VecDeque;

use voltip_protocol::ticket::NONCE_LEN;

/// Remembers the last `capacity` nonces.
#[derive(Debug, Clone)]
pub struct NonceLedger {
    seen: VecDeque<[u8; NONCE_LEN]>,
    capacity: usize,
}

impl Default for NonceLedger {
    fn default() -> Self {
        Self::new(256)
    }
}

impl NonceLedger {
    /// Ledger remembering at most `capacity` nonces (oldest evicted first).
    pub fn new(capacity: usize) -> Self {
        Self { seen: VecDeque::with_capacity(capacity.min(1024)), capacity: capacity.max(1) }
    }

    /// Record a nonce. Returns `false` if it was already present (replay).
    pub fn record(&mut self, nonce: [u8; NONCE_LEN]) -> bool {
        if self.seen.contains(&nonce) {
            return false;
        }
        if self.seen.len() == self.capacity {
            self.seen.pop_front();
        }
        self.seen.push_back(nonce);
        true
    }

    /// Whether a nonce has been seen.
    pub fn contains(&self, nonce: &[u8; NONCE_LEN]) -> bool {
        self.seen.contains(nonce)
    }

    /// Number of nonces remembered.
    pub fn len(&self) -> usize {
        self.seen.len()
    }

    /// Whether nothing has been recorded.
    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_once_and_detects_replay() {
        let mut l = NonceLedger::default();
        assert!(l.is_empty());
        assert!(l.record([1; NONCE_LEN]));
        assert!(!l.record([1; NONCE_LEN]));
        assert!(l.contains(&[1; NONCE_LEN]));
        assert!(!l.contains(&[2; NONCE_LEN]));
        assert_eq!(l.len(), 1);
    }

    #[test]
    fn evicts_oldest_when_full() {
        let mut l = NonceLedger::new(2);
        assert!(l.record([1; NONCE_LEN]));
        assert!(l.record([2; NONCE_LEN]));
        assert!(l.record([3; NONCE_LEN]));
        assert_eq!(l.len(), 2);
        assert!(!l.contains(&[1; NONCE_LEN]));
        assert!(l.contains(&[3; NONCE_LEN]));
        let tiny = NonceLedger::new(0);
        assert_eq!(tiny.capacity, 1);
    }
}
