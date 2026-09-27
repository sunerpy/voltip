//! Noise transport mode over any framed link.

use voltip_crypto::{CryptoError, SessionCipher};
use voltip_protocol::app::AppMessage;

use crate::TransportError;

/// Encrypts / decrypts [`AppMessage`]s with a session cipher. Owns the cipher so nonces cannot
/// be advanced from two places.
pub struct SecureChannel {
    cipher: SessionCipher,
    sent: u64,
    received: u64,
}

impl std::fmt::Debug for SecureChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecureChannel").field("sent", &self.sent).field("received", &self.received).finish()
    }
}

impl SecureChannel {
    /// Wrap a cipher produced by a completed handshake / pairing.
    pub fn new(cipher: SessionCipher) -> Self {
        Self { cipher, sent: 0, received: 0 }
    }

    /// Encrypt an application message into bytes for `forward.payload`.
    pub fn seal(&mut self, msg: &AppMessage) -> Result<Vec<u8>, TransportError> {
        let plain = msg.encode()?;
        let bytes = self.cipher.encrypt(&plain)?;
        self.sent += 1;
        Ok(bytes)
    }

    /// Decrypt bytes from `forward.payload` into an application message.
    pub fn open(&mut self, bytes: &[u8]) -> Result<AppMessage, TransportError> {
        let plain = self.cipher.decrypt(bytes).map_err(|e: CryptoError| TransportError::Crypto(e))?;
        let msg = AppMessage::decode(&plain)?;
        self.received += 1;
        Ok(msg)
    }

    /// Messages sealed so far.
    pub fn sent(&self) -> u64 {
        self.sent
    }

    /// Messages opened so far.
    pub fn received(&self) -> u64 {
        self.received
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use voltip_crypto::{Handshake, Role, StaticKeypair, complete_in_memory};

    fn channels() -> (SecureChannel, SecureChannel) {
        let ka = StaticKeypair::generate().unwrap();
        let kb = StaticKeypair::generate().unwrap();
        let (oa, ob) = complete_in_memory(Handshake::new(Role::Initiator, &ka, None).unwrap(), Handshake::new(Role::Responder, &kb, None).unwrap()).unwrap();
        (SecureChannel::new(oa.cipher), SecureChannel::new(ob.cipher))
    }

    #[test]
    fn seal_open_roundtrip_counts_and_rejects_garbage() {
        let (mut a, mut b) = channels();
        let bytes = a.seal(&AppMessage::text("hello")).unwrap();
        assert!(!bytes.windows(5).any(|w| w == b"hello"), "ciphertext must not contain plaintext");
        assert_eq!(b.open(&bytes).unwrap(), AppMessage::text("hello"));
        assert_eq!(a.sent(), 1);
        assert_eq!(b.received(), 1);
        assert!(matches!(b.open(&bytes).unwrap_err(), TransportError::Crypto(_)), "replay");
        assert!(matches!(b.open(&[0u8; 8]).unwrap_err(), TransportError::Crypto(_)));
        assert!(format!("{a:?}").contains("sent: 1"));
        // Oversize message fails at encode time.
        let big = AppMessage::text("x".repeat(AppMessage::MAX_ENCODED_BYTES));
        assert!(matches!(a.seal(&big).unwrap_err(), TransportError::Codec(_)));
    }
}
