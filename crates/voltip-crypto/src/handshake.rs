//! `Noise_XX_25519_ChaChaPoly_SHA256` handshake and the transport-mode cipher it yields.
//!
//! The initiator's first message is exactly its ephemeral public key. We write it eagerly in
//! [`Handshake::new`] so the pairing layer can put that key into the QR ticket *before* any
//! peer shows up; the responder then checks the received message 1 against the ticket, which
//! binds the QR code to the key exchange (`docs/protocol.md` §2).

use snow::{HandshakeState, TransportState};

use crate::{CryptoError, PUBLIC_KEY_LEN, PublicKey, SafetyCode, StaticKeypair};

/// The one Noise pattern this build speaks.
pub const PATTERN: &str = "Noise_XX_25519_ChaChaPoly_SHA256";
/// Noise's hard limit on a single message (handshake or transport), in bytes.
pub const MAX_NOISE_MESSAGE_LEN: usize = 65_535;
/// AEAD tag length ChaCha20-Poly1305 appends to every transport message.
const TAG_LEN: usize = 16;

/// Which side of the handshake we are.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    /// Created the pairing session (desktop in the phase-1 flow).
    Initiator,
    /// Joined the session (phone).
    Responder,
}

/// What the driver should do next.
#[derive(Debug)]
pub enum HandshakeStep {
    /// Send these bytes to the peer.
    Send(Vec<u8>),
    /// Nothing to send; wait for the peer's next message.
    AwaitPeer,
    /// Handshake complete; call [`Handshake::finish`].
    Complete,
}

/// Result of a completed handshake.
pub struct HandshakeOutcome {
    /// Cipher for the rest of the session.
    pub cipher: SessionCipher,
    /// The peer's long-term identity key, authenticated by the handshake.
    pub remote_static: PublicKey,
    /// The Noise handshake hash — identical on both sides, binds every message and both
    /// static keys. Source of the safety code.
    pub handshake_hash: [u8; 32],
    /// Human-comparable rendering of `handshake_hash`.
    pub safety_code: SafetyCode,
}

impl std::fmt::Debug for HandshakeOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HandshakeOutcome").field("remote_static", &self.remote_static).field("safety_code", &self.safety_code).finish_non_exhaustive()
    }
}

/// An in-progress Noise XX handshake.
pub struct Handshake {
    state: HandshakeState,
    role: Role,
    pending_out: Option<Vec<u8>>,
    expected_initiator_ephemeral: Option<[u8; PUBLIC_KEY_LEN]>,
    initiator_ephemeral: Option<[u8; PUBLIC_KEY_LEN]>,
    messages_seen: u8,
}

impl std::fmt::Debug for Handshake {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Handshake").field("role", &self.role).field("messages_seen", &self.messages_seen).finish_non_exhaustive()
    }
}

impl Handshake {
    /// Start a handshake.
    ///
    /// * `expected_initiator_ephemeral` — responder only: the `ephemeral_pub` from a scanned
    ///   ticket. Message 1 must carry exactly this key. Pass `None` for code-based joins
    ///   (the safety code still protects them).
    pub fn new(role: Role, local: &StaticKeypair, expected_initiator_ephemeral: Option<[u8; PUBLIC_KEY_LEN]>) -> Result<Self, CryptoError> {
        let builder = snow::Builder::new(PATTERN.parse()?).local_private_key(local.secret.expose())?;
        let mut hs = match role {
            Role::Initiator => Self {
                state: builder.build_initiator()?,
                role,
                pending_out: None,
                expected_initiator_ephemeral: None,
                initiator_ephemeral: None,
                messages_seen: 0,
            },
            Role::Responder => {
                Self { state: builder.build_responder()?, role, pending_out: None, expected_initiator_ephemeral, initiator_ephemeral: None, messages_seen: 0 }
            }
        };
        if role == Role::Initiator {
            // Message 1 = `e`: write it now so the ephemeral key is known for the ticket.
            let msg1 = hs.write()?;
            let mut e = [0u8; PUBLIC_KEY_LEN];
            e.copy_from_slice(&msg1[..PUBLIC_KEY_LEN]);
            hs.initiator_ephemeral = Some(e);
            hs.pending_out = Some(msg1);
        }
        Ok(hs)
    }

    /// Our role.
    pub fn role(&self) -> Role {
        self.role
    }

    /// The initiator's ephemeral public key (available immediately for the initiator, after
    /// message 1 for the responder). This is what goes into the pairing ticket.
    pub fn initiator_ephemeral(&self) -> Option<[u8; PUBLIC_KEY_LEN]> {
        self.initiator_ephemeral
    }

    /// Feed a message from the peer.
    pub fn receive(&mut self, msg: &[u8]) -> Result<(), CryptoError> {
        if self.state.is_handshake_finished() {
            return Err(CryptoError::OutOfOrder { expected: "finish" });
        }
        if self.pending_out.is_some() || self.state.is_my_turn() {
            return Err(CryptoError::OutOfOrder { expected: "next_step (send)" });
        }
        if msg.len() > MAX_NOISE_MESSAGE_LEN {
            return Err(CryptoError::TooLarge(msg.len()));
        }
        if self.role == Role::Responder && self.messages_seen == 0 {
            // Message 1 must start with the ephemeral key promised by the ticket.
            if msg.len() < PUBLIC_KEY_LEN {
                return Err(CryptoError::Noise(snow::Error::Input));
            }
            let mut e = [0u8; PUBLIC_KEY_LEN];
            e.copy_from_slice(&msg[..PUBLIC_KEY_LEN]);
            if let Some(expected) = self.expected_initiator_ephemeral {
                use subtle::ConstantTimeEq as _;
                if !bool::from(e.ct_eq(&expected)) {
                    return Err(CryptoError::TicketMismatch);
                }
            }
            self.initiator_ephemeral = Some(e);
        }
        let mut payload = vec![0u8; MAX_NOISE_MESSAGE_LEN];
        self.state.read_message(msg, &mut payload)?;
        self.messages_seen += 1;
        Ok(())
    }

    /// Advance: returns the next message to send, or tells the driver to wait / finish.
    pub fn next_step(&mut self) -> Result<HandshakeStep, CryptoError> {
        if let Some(out) = self.pending_out.take() {
            return Ok(HandshakeStep::Send(out));
        }
        if self.state.is_handshake_finished() {
            return Ok(HandshakeStep::Complete);
        }
        if self.state.is_my_turn() {
            let out = self.write()?;
            return Ok(HandshakeStep::Send(out));
        }
        Ok(HandshakeStep::AwaitPeer)
    }

    /// `true` once all three messages have been processed.
    pub fn is_finished(&self) -> bool {
        self.state.is_handshake_finished() && self.pending_out.is_none()
    }

    /// Consume the handshake and enter transport mode.
    pub fn finish(self) -> Result<HandshakeOutcome, CryptoError> {
        if !self.is_finished() {
            return Err(CryptoError::OutOfOrder { expected: "complete the handshake first" });
        }
        let mut handshake_hash = [0u8; 32];
        handshake_hash.copy_from_slice(self.state.get_handshake_hash());
        let remote = self.state.get_remote_static().ok_or(CryptoError::OutOfOrder { expected: "remote static key" })?;
        let remote_static = PublicKey::from_slice(remote)?;
        let transport = self.state.into_transport_mode()?;
        Ok(HandshakeOutcome {
            cipher: SessionCipher { inner: transport },
            remote_static,
            handshake_hash,
            safety_code: SafetyCode::from_handshake_hash(&handshake_hash),
        })
    }

    fn write(&mut self) -> Result<Vec<u8>, CryptoError> {
        let mut out = vec![0u8; MAX_NOISE_MESSAGE_LEN];
        let n = self.state.write_message(&[], &mut out)?;
        out.truncate(n);
        self.messages_seen += 1;
        Ok(out)
    }
}

/// Transport-mode Noise cipher. Nonces are managed by Noise (monotonic per direction) so a
/// caller cannot reuse one; a rejected/reordered message is a hard error, not a retry.
pub struct SessionCipher {
    inner: TransportState,
}

impl std::fmt::Debug for SessionCipher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionCipher(<redacted>)")
    }
}

impl SessionCipher {
    /// Largest plaintext that fits one message.
    pub const MAX_PLAINTEXT_LEN: usize = MAX_NOISE_MESSAGE_LEN - TAG_LEN;

    /// Encrypt one message.
    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if plaintext.len() > Self::MAX_PLAINTEXT_LEN {
            return Err(CryptoError::TooLarge(plaintext.len()));
        }
        let mut out = vec![0u8; plaintext.len() + TAG_LEN];
        let n = self.inner.write_message(plaintext, &mut out)?;
        out.truncate(n);
        Ok(out)
    }

    /// Decrypt one message.
    pub fn decrypt(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if ciphertext.len() > MAX_NOISE_MESSAGE_LEN {
            return Err(CryptoError::TooLarge(ciphertext.len()));
        }
        let mut out = vec![0u8; ciphertext.len()];
        let n = self.inner.read_message(ciphertext, &mut out)?;
        out.truncate(n);
        Ok(out)
    }

    /// Number of messages sent so far (Noise nonce of the sending direction).
    pub fn sending_nonce(&self) -> u64 {
        self.inner.sending_nonce()
    }
}

/// Drive two handshakes against each other in memory. Test-only helper, exported so
/// downstream crates' tests can build a ready session in one line.
#[doc(hidden)]
pub fn complete_in_memory(mut a: Handshake, mut b: Handshake) -> Result<(HandshakeOutcome, HandshakeOutcome), CryptoError> {
    for _ in 0..8 {
        match a.next_step()? {
            HandshakeStep::Send(m) => b.receive(&m)?,
            HandshakeStep::AwaitPeer => {}
            HandshakeStep::Complete => {}
        }
        match b.next_step()? {
            HandshakeStep::Send(m) => a.receive(&m)?,
            HandshakeStep::AwaitPeer => {}
            HandshakeStep::Complete => {}
        }
        if a.is_finished() && b.is_finished() {
            return Ok((a.finish()?, b.finish()?));
        }
    }
    Err(CryptoError::OutOfOrder { expected: "handshake to converge" })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> (StaticKeypair, StaticKeypair) {
        (StaticKeypair::generate().unwrap(), StaticKeypair::generate().unwrap())
    }

    #[test]
    fn full_handshake_authenticates_both_static_keys_and_agrees_on_safety_code() {
        let (ka, kb) = pair();
        let a = Handshake::new(Role::Initiator, &ka, None).unwrap();
        let e = a.initiator_ephemeral().unwrap();
        let b = Handshake::new(Role::Responder, &kb, Some(e)).unwrap();
        assert_eq!(a.role(), Role::Initiator);
        assert_eq!(b.role(), Role::Responder);
        let (oa, ob) = complete_in_memory(a, b).unwrap();
        assert_eq!(oa.remote_static, kb.public);
        assert_eq!(ob.remote_static, ka.public);
        assert_eq!(oa.handshake_hash, ob.handshake_hash);
        assert_eq!(oa.safety_code, ob.safety_code);
        assert!(format!("{oa:?}").contains("safety_code"));
    }

    #[test]
    fn transport_roundtrip_both_directions_with_monotonic_nonces() {
        let (ka, kb) = pair();
        let a = Handshake::new(Role::Initiator, &ka, None).unwrap();
        let b = Handshake::new(Role::Responder, &kb, None).unwrap();
        let (mut oa, mut ob) = complete_in_memory(a, b).unwrap();
        assert_eq!(oa.cipher.sending_nonce(), 0);
        let c1 = oa.cipher.encrypt(b"hello").unwrap();
        assert_eq!(oa.cipher.sending_nonce(), 1);
        assert_ne!(&c1[..5], b"hello");
        assert_eq!(ob.cipher.decrypt(&c1).unwrap(), b"hello");
        let c2 = ob.cipher.encrypt(b"").unwrap();
        assert_eq!(c2.len(), TAG_LEN);
        assert_eq!(oa.cipher.decrypt(&c2).unwrap(), b"");
        assert_eq!(format!("{:?}", oa.cipher), "SessionCipher(<redacted>)");
    }

    #[test]
    fn tampered_or_replayed_ciphertext_is_rejected() {
        let (ka, kb) = pair();
        let (mut oa, mut ob) =
            complete_in_memory(Handshake::new(Role::Initiator, &ka, None).unwrap(), Handshake::new(Role::Responder, &kb, None).unwrap()).unwrap();
        let c = oa.cipher.encrypt(b"payload").unwrap();
        let mut bad = c.clone();
        bad[0] ^= 1;
        assert!(matches!(ob.cipher.decrypt(&bad).unwrap_err(), CryptoError::Noise(_)));
        // A failed decryption does not advance the nonce, so the genuine message still decrypts…
        assert_eq!(ob.cipher.decrypt(&c).unwrap(), b"payload");
        // …and replaying it afterwards fails because the receive nonce has moved on.
        assert_eq!(ob.cipher.decrypt(&c).unwrap_err().to_string(), "noise protocol failure");
    }

    #[test]
    fn ticket_mismatch_is_detected_on_message_one() {
        let (ka, kb) = pair();
        let a = Handshake::new(Role::Initiator, &ka, None).unwrap();
        let wrong = [1u8; PUBLIC_KEY_LEN];
        let mut b = Handshake::new(Role::Responder, &kb, Some(wrong)).unwrap();
        let mut a = a;
        let HandshakeStep::Send(m1) = a.next_step().unwrap() else { panic!("initiator must send first") };
        assert!(matches!(b.receive(&m1).unwrap_err(), CryptoError::TicketMismatch));
    }

    #[test]
    fn mitm_yields_different_safety_codes() {
        // Attacker M sits between A and B running two independent handshakes.
        let (ka, kb) = pair();
        let km = StaticKeypair::generate().unwrap();
        let a = Handshake::new(Role::Initiator, &ka, None).unwrap();
        let m_as_responder = Handshake::new(Role::Responder, &km, None).unwrap();
        let m_as_initiator = Handshake::new(Role::Initiator, &km, None).unwrap();
        let b = Handshake::new(Role::Responder, &kb, None).unwrap();
        let (oa, _) = complete_in_memory(a, m_as_responder).unwrap();
        let (_, ob) = complete_in_memory(m_as_initiator, b).unwrap();
        assert_ne!(oa.safety_code, ob.safety_code, "users would see different words and refuse");
        assert_ne!(oa.remote_static, kb.public);
    }

    #[test]
    fn out_of_order_driving_is_rejected() {
        let (ka, kb) = pair();
        let mut a = Handshake::new(Role::Initiator, &ka, None).unwrap();
        let mut b = Handshake::new(Role::Responder, &kb, None).unwrap();
        // Responder has nothing to send before message 1.
        assert!(matches!(b.next_step().unwrap(), HandshakeStep::AwaitPeer));
        // Initiator cannot receive before sending message 1.
        assert!(matches!(a.receive(&[0u8; 32]).unwrap_err(), CryptoError::OutOfOrder { .. }));
        assert!(matches!(a.finish().unwrap_err(), CryptoError::OutOfOrder { .. }));
        // Responder rejects a too-short first message and an oversize one.
        assert!(matches!(b.receive(&[0u8; 8]).unwrap_err(), CryptoError::Noise(_)));
        assert!(matches!(b.receive(&vec![0u8; MAX_NOISE_MESSAGE_LEN + 1]).unwrap_err(), CryptoError::TooLarge(_)));
    }

    #[test]
    fn finished_handshake_refuses_more_messages() {
        let (ka, kb) = pair();
        let mut a = Handshake::new(Role::Initiator, &ka, None).unwrap();
        let mut b = Handshake::new(Role::Responder, &kb, None).unwrap();
        let HandshakeStep::Send(m1) = a.next_step().unwrap() else { panic!() };
        b.receive(&m1).unwrap();
        let HandshakeStep::Send(m2) = b.next_step().unwrap() else { panic!() };
        a.receive(&m2).unwrap();
        let HandshakeStep::Send(m3) = a.next_step().unwrap() else { panic!() };
        b.receive(&m3).unwrap();
        assert!(matches!(a.next_step().unwrap(), HandshakeStep::Complete));
        assert!(matches!(b.next_step().unwrap(), HandshakeStep::Complete));
        assert!(a.is_finished() && b.is_finished());
        assert!(matches!(a.receive(&m3).unwrap_err(), CryptoError::OutOfOrder { .. }));
        assert!(format!("{a:?}").contains("Initiator"));
        assert_eq!(b.initiator_ephemeral(), a.initiator_ephemeral());
    }

    #[test]
    fn oversize_plaintext_and_ciphertext_are_refused() {
        let (ka, kb) = pair();
        let (mut oa, mut ob) =
            complete_in_memory(Handshake::new(Role::Initiator, &ka, None).unwrap(), Handshake::new(Role::Responder, &kb, None).unwrap()).unwrap();
        let big = vec![0u8; SessionCipher::MAX_PLAINTEXT_LEN + 1];
        assert!(matches!(oa.cipher.encrypt(&big).unwrap_err(), CryptoError::TooLarge(_)));
        let big_c = vec![0u8; MAX_NOISE_MESSAGE_LEN + 1];
        assert!(matches!(ob.cipher.decrypt(&big_c).unwrap_err(), CryptoError::TooLarge(_)));
        let max = vec![7u8; SessionCipher::MAX_PLAINTEXT_LEN];
        let c = oa.cipher.encrypt(&max).unwrap();
        assert_eq!(ob.cipher.decrypt(&c).unwrap(), max);
    }
}
