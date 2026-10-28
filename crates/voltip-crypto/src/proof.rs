//! The proof that the joining side read the pairing ticket (docs/pairing.md 「扫码配对只在手机上
//! 确认」): HMAC-SHA256 keyed by the ticket's one-time secret over the handshake hash. The secret
//! travels only in the QR code, never through the relay, so a peer that knows the session (the
//! relay, or someone who typed the six digits) cannot produce it; the handshake hash binds it to
//! this one key exchange, so an old proof is worth nothing in another.

use hmac::{Hmac, KeyInit as _, Mac as _};
use sha2::Sha256;
use subtle::ConstantTimeEq as _;

use crate::CryptoError;

/// Length of the ticket's secret and of a proof.
pub const PROOF_LEN: usize = 32;

/// Separates this use of the key from any other.
const LABEL: &[u8] = b"voltip/pairing-proof/v1";

/// The proof for `handshake_hash` under the ticket's `secret`.
pub fn pairing_proof(secret: &[u8; PROOF_LEN], handshake_hash: &[u8; 32]) -> Result<[u8; PROOF_LEN], CryptoError> {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).map_err(|_| CryptoError::KeyLength(secret.len()))?;
    mac.update(LABEL);
    mac.update(handshake_hash);
    let mut out = [0u8; PROOF_LEN];
    out.copy_from_slice(&mac.finalize().into_bytes());
    Ok(out)
}

/// Whether `proof` is the proof for `handshake_hash` under `secret`, compared in constant time.
pub fn verify_pairing_proof(secret: &[u8; PROOF_LEN], handshake_hash: &[u8; 32], proof: &[u8; PROOF_LEN]) -> bool {
    pairing_proof(secret, handshake_hash).is_ok_and(|expected| bool::from(expected.ct_eq(proof)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the construction: a build that computed it differently could not pair with this one.
    /// The value is Python's `hmac.new(bytes(range(32)), b"voltip/pairing-proof/v1" +
    /// bytes([0xaa] * 32), hashlib.sha256)`.
    #[test]
    fn the_proof_is_hmac_sha256_over_the_label_and_the_hash() {
        let secret: [u8; 32] = std::array::from_fn(|i| i as u8);
        let proof = pairing_proof(&secret, &[0xaa; 32]).unwrap();
        assert_eq!(hex::encode(proof), "af50b83b92fbfb2128235704eed260736af407e564675c20034da8430673aeab");
    }

    #[test]
    fn only_the_same_secret_and_hash_verify() {
        let secret = [7; PROOF_LEN];
        let hash = [1; 32];
        let proof = pairing_proof(&secret, &hash).unwrap();
        assert!(verify_pairing_proof(&secret, &hash, &proof));
        assert!(!verify_pairing_proof(&[8; PROOF_LEN], &hash, &proof), "another ticket's secret");
        assert!(!verify_pairing_proof(&secret, &[2; 32], &proof), "another handshake");
        let mut flipped = proof;
        flipped[31] ^= 1;
        assert!(!verify_pairing_proof(&secret, &hash, &flipped));
    }
}
