//! Rendezvous channel for two paired devices.

use sha2::{Digest as _, Sha256};
use voltip_crypto::PublicKey;

/// `hex(SHA-256(min(a,b) || max(a,b)))` — identical on both sides, meaningless to the relay
/// beyond "these two connections belong together".
pub fn rendezvous_channel(a: &PublicKey, b: &PublicKey) -> String {
    let (lo, hi) = if a.as_bytes() <= b.as_bytes() { (a, b) } else { (b, a) };
    let mut h = Sha256::new();
    h.update(b"voltip rendezvous v1");
    h.update(lo.as_bytes());
    h.update(hi.as_bytes());
    hex::encode(h.finalize())
}

/// Deterministic role for the re-handshake between paired peers: the smaller static key
/// initiates. Both sides agree without talking.
pub fn is_initiator(local: &PublicKey, remote: &PublicKey) -> bool {
    local.as_bytes() < remote.as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_is_symmetric_and_well_formed() {
        let a = PublicKey([1; 32]);
        let b = PublicKey([2; 32]);
        let ab = rendezvous_channel(&a, &b);
        assert_eq!(ab, rendezvous_channel(&b, &a));
        assert_eq!(ab.len(), 64);
        assert!(voltip_protocol::relay::validate_channel(&ab).is_ok());
        assert_ne!(ab, rendezvous_channel(&a, &PublicKey([3; 32])));
    }

    #[test]
    fn exactly_one_side_initiates() {
        let a = PublicKey([1; 32]);
        let b = PublicKey([2; 32]);
        assert!(is_initiator(&a, &b));
        assert!(!is_initiator(&b, &a));
        assert!(!is_initiator(&a, &a));
    }
}
