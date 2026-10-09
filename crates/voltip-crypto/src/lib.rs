//! Cryptography for Voltip — a thin, audited-primitive-only layer.
//!
//! * [`StaticKeypair`] — the long-term device identity (X25519). The secret half is wrapped in
//!   a zeroizing type whose `Debug`/`Serialize` never expose bytes.
//! * [`Handshake`] — `Noise_XX_25519_ChaChaPoly_SHA256` via the `snow` crate; produces a
//!   [`SessionCipher`] plus a [`SafetyCode`] derived from the handshake hash.
//! * [`SafetyCode`] — four BIP-39 words + hex fingerprint that both users compare.
//! * [`random_pair_code`] / [`random_nonce`] — CSPRNG helpers for the pairing layer.
//!
//! Nothing in this crate implements a primitive; it only composes `snow`, `sha2`, `rand`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod handshake;
mod keys;
mod safety;

#[doc(hidden)]
pub use handshake::complete_in_memory;
pub use handshake::{HANDSHAKE_MESSAGE_LENS, Handshake, HandshakeOutcome, HandshakeStep, MAX_NOISE_MESSAGE_LEN, Role, SessionCipher, TAG_LEN};
pub use keys::{PUBLIC_KEY_LEN, PublicKey, SecretKey, StaticKeypair};
pub use safety::{SafetyCode, WORD_COUNT, fingerprint_hex};

/// Errors from the crypto layer. Deliberately coarse: callers must not branch on
/// cryptographic failure details.
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    /// Handshake or transport message was rejected by Noise.
    #[error("noise protocol failure")]
    Noise(#[source] snow::Error),
    /// The handshake state machine was driven out of order.
    #[error("handshake step out of order: expected {expected}")]
    OutOfOrder {
        /// What the caller should have done instead.
        expected: &'static str,
    },
    /// The first handshake message did not carry the ephemeral key promised by the ticket.
    #[error("ephemeral key does not match the pairing ticket")]
    TicketMismatch,
    /// A message exceeded the Noise packet limit.
    #[error("message too large for a single noise packet ({0} bytes)")]
    TooLarge(usize),
    /// A key had the wrong length.
    #[error("invalid key length {0}")]
    KeyLength(usize),
}

impl From<snow::Error> for CryptoError {
    fn from(value: snow::Error) -> Self {
        Self::Noise(value)
    }
}

/// Generate a fresh six-digit pairing code from the OS CSPRNG (uniform in `0..1_000_000`).
pub fn random_pair_code() -> u32 {
    use rand::RngExt as _;
    rand::rng().random_range(0..1_000_000)
}

/// Generate a fresh random nonce.
pub fn random_nonce<const N: usize>() -> [u8; N] {
    use rand::Rng as _;
    let mut out = [0u8; N];
    rand::rng().fill_bytes(&mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_codes_are_in_range_and_not_constant() {
        let codes: Vec<u32> = (0..64).map(|_| random_pair_code()).collect();
        assert!(codes.iter().all(|c| *c < 1_000_000));
        assert!(codes.iter().any(|c| *c != codes[0]), "64 draws never differed");
    }

    #[test]
    fn nonces_are_random() {
        let a: [u8; 16] = random_nonce();
        let b: [u8; 16] = random_nonce();
        assert_ne!(a, b);
        let c: [u8; 32] = random_nonce();
        assert_eq!(c.len(), 32);
    }

    #[test]
    fn error_display_and_conversion() {
        let e = CryptoError::OutOfOrder { expected: "write" };
        assert!(e.to_string().contains("write"));
        assert!(CryptoError::TicketMismatch.to_string().contains("ticket"));
        assert!(CryptoError::TooLarge(70_000).to_string().contains("70000"));
        assert!(CryptoError::KeyLength(3).to_string().contains('3'));
        let n: CryptoError = snow::Error::Input.into();
        assert!(matches!(n, CryptoError::Noise(_)));
        assert_eq!(n.to_string(), "noise protocol failure");
    }
}
