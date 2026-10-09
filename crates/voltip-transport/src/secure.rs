//! Noise transport mode over any framed link.

use voltip_crypto::{CryptoError, HANDSHAKE_MESSAGE_LENS, SessionCipher, TAG_LEN};
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
    ///
    /// The result is never as long as a handshake message (docs/dictation.md §20.8): when it
    /// would be, one zero byte follows the CBOR. [`AppMessage::decode`] reads one CBOR item and
    /// ignores what comes after it, so every build reads the message unchanged.
    pub fn seal(&mut self, msg: &AppMessage) -> Result<Vec<u8>, TransportError> {
        let mut plain = msg.encode()?;
        if HANDSHAKE_MESSAGE_LENS.contains(&(plain.len() + TAG_LEN)) {
            plain.push(0);
        }
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

    /// regression (plan gate, M7 design round 7): a frame of an ended session that happened to be
    /// as long as a handshake message would be fed to the next handshake and break it, e.g. a
    /// part whose CBOR is 80 bytes sealed to 96 bytes, message 2's length. No sealed frame is one
    /// of those lengths, and the padded ones open to the same message.
    #[test]
    fn regression_no_sealed_frame_has_a_handshake_message_length() {
        let (mut a, mut b) = channels();
        let mut padded = 0;
        for n in 0..=200 {
            for msg in [AppMessage::text("x".repeat(n)), AppMessage::bulk(0, true, vec![7; n + 1])] {
                let plain = msg.encode().unwrap().len();
                let bytes = a.seal(&msg).unwrap();
                assert!(!HANDSHAKE_MESSAGE_LENS.contains(&bytes.len()), "{} bytes for a {plain}-byte message", bytes.len());
                if bytes.len() != plain + TAG_LEN {
                    padded += 1;
                    assert_eq!(bytes.len(), plain + TAG_LEN + 1);
                }
                assert_eq!(b.open(&bytes).unwrap(), msg);
            }
        }
        assert!(padded >= 3, "every handshake length was hit and moved ({padded})");
        // The example from the review: a 41-byte tail part with `seq < 24` is 80 bytes of CBOR.
        let tail = AppMessage::bulk(3, true, vec![1; 41]);
        assert_eq!(tail.encode().unwrap().len(), 80);
        assert_eq!(a.seal(&tail).unwrap().len(), 97);
    }
}
