//! Post-join engine shared by initiator and responder: drives the Noise handshake, then the
//! safety-code verification exchange, and hands out an [`Established`] session. When the phone
//! scanned a ticket with a secret, it proves so right after the handshake and the computer
//! confirms by itself (docs/pairing.md 「扫码配对只在手机上确认」).

use std::time::Instant;

use voltip_crypto::{Handshake, HandshakeOutcome, HandshakeStep, PROOF_LEN};
use voltip_identity::DeviceIdentity;
use voltip_protocol::app::{AppMessage, RejectReason};
use voltip_protocol::{DeviceInfo, ProtocolVersion, SessionId};

use crate::common::{Action, Established, FailureReason, Now, PairingState, Timeouts};

/// Result of feeding the engine.
pub(crate) enum Step {
    /// Still going; actions to perform.
    Continue(Vec<Action>),
    /// Terminal: the machine should switch to this state (actions first).
    Terminal(PairingState, Vec<Action>),
}

/// What the engine does with the ticket's secret.
pub(crate) enum Proof {
    /// No secret: both users confirm (a typed code, or a ticket from a build up to 0.1.9).
    None,
    /// Responder that scanned a ticket with this secret: proves it once the handshake is done.
    Give([u8; PROOF_LEN]),
    /// Initiator whose ticket carried this secret: a proof under it confirms for its user.
    Expect([u8; PROOF_LEN]),
}

pub(crate) struct Engine {
    pub(crate) session_id: SessionId,
    identity: DeviceIdentity,
    timeouts: Timeouts,
    phase: Phase,
    deadline: Instant,
    proof: Proof,
    local_confirmed: bool,
    peer_confirmed: bool,
    /// Responder: it sent the proof. Initiator: a valid proof arrived.
    scanned: bool,
    peer_info: Option<DeviceInfo>,
}

enum Phase {
    Handshaking(Box<Handshake>),
    Verifying(Box<HandshakeOutcome>),
    Done,
}

impl Engine {
    pub(crate) fn new(session_id: SessionId, identity: DeviceIdentity, handshake: Handshake, timeouts: Timeouts, now: Now, proof: Proof) -> Self {
        Self {
            session_id,
            identity,
            timeouts,
            phase: Phase::Handshaking(Box::new(handshake)),
            deadline: now.instant + timeouts.handshake,
            proof,
            local_confirmed: false,
            peer_confirmed: false,
            scanned: false,
            peer_info: None,
        }
    }

    pub(crate) fn state(&self) -> PairingState {
        match self.phase {
            Phase::Handshaking(_) => PairingState::KeyExchange,
            Phase::Verifying(_) => PairingState::AwaitingVerification,
            Phase::Done => PairingState::Trusted,
        }
    }

    pub(crate) fn safety_code(&self) -> Option<voltip_crypto::SafetyCode> {
        match &self.phase {
            Phase::Verifying(o) => Some(o.safety_code.clone()),
            _ => None,
        }
    }

    pub(crate) fn peer_info(&self) -> Option<DeviceInfo> {
        self.peer_info.clone()
    }

    pub(crate) fn local_confirmed(&self) -> bool {
        self.local_confirmed
    }

    pub(crate) fn peer_confirmed(&self) -> bool {
        self.peer_confirmed
    }

    pub(crate) fn scanned(&self) -> bool {
        self.scanned
    }

    /// What the user sees change without the state changing.
    pub(crate) fn flags(&self) -> [bool; 4] {
        [self.local_confirmed, self.peer_confirmed, self.scanned, self.peer_info.is_some()]
    }

    /// Kick the handshake: returns the first message to send, if it is our turn.
    pub(crate) fn pump(&mut self) -> Step {
        let Phase::Handshaking(hs) = &mut self.phase else { return Step::Continue(Vec::new()) };
        let mut actions = Vec::new();
        loop {
            match hs.next_step() {
                Ok(HandshakeStep::Send(bytes)) => actions.push(Action::SendPeer(bytes)),
                Ok(HandshakeStep::AwaitPeer) => return Step::Continue(actions),
                Ok(HandshakeStep::Complete) => break,
                Err(_) => return Step::Terminal(PairingState::Failed { reason: FailureReason::Handshake }, vec![Action::Close]),
            }
        }
        self.finish_handshake(actions)
    }

    fn finish_handshake(&mut self, mut actions: Vec<Action>) -> Step {
        let Phase::Handshaking(hs) = std::mem::replace(&mut self.phase, Phase::Done) else { return Step::Continue(actions) };
        match (*hs).finish() {
            Ok(mut outcome) => {
                tracing::info!(session = %self.session_id, peer = ?outcome.remote_static, "handshake complete");
                if let Proof::Give(secret) = &self.proof {
                    let Some(proof) = self.prove(secret, &mut outcome) else {
                        actions.push(Action::Close);
                        return Step::Terminal(PairingState::Failed { reason: FailureReason::Handshake }, actions);
                    };
                    actions.push(Action::SendPeer(proof));
                    self.scanned = true;
                }
                self.phase = Phase::Verifying(Box::new(outcome));
                // Deadline for the humans is measured from here.
                self.deadline = self.deadline_after_handshake();
                Step::Continue(actions)
            }
            Err(_) => {
                actions.push(Action::Close);
                Step::Terminal(PairingState::Failed { reason: FailureReason::Handshake }, actions)
            }
        }
    }

    /// The encrypted `pair_proof` for this handshake.
    fn prove(&self, secret: &[u8; PROOF_LEN], outcome: &mut HandshakeOutcome) -> Option<Vec<u8>> {
        let proof = voltip_crypto::pairing_proof(secret, &outcome.handshake_hash).ok()?;
        let plain = AppMessage::PairProof { version: ProtocolVersion::CURRENT, proof, device: self.identity.info() }.encode().ok()?;
        outcome.cipher.encrypt(&plain).ok()
    }

    /// Initiator: a valid proof means the phone read the QR code shown here, and this side
    /// confirms for its user; anything else fails the pairing.
    fn on_proof(&mut self, proof: &[u8; PROOF_LEN], device: DeviceInfo) -> Step {
        let (Proof::Expect(secret), Phase::Verifying(outcome)) = (&self.proof, &self.phase) else {
            return Step::Terminal(PairingState::Failed { reason: FailureReason::Protocol }, vec![Action::Close]);
        };
        if self.scanned {
            return Step::Terminal(PairingState::Failed { reason: FailureReason::Protocol }, vec![Action::Close]);
        }
        if !voltip_crypto::verify_pairing_proof(secret, &outcome.handshake_hash, proof) {
            tracing::warn!(session = %self.session_id, "pairing proof rejected");
            return Step::Terminal(PairingState::Failed { reason: FailureReason::Handshake }, vec![Action::Close]);
        }
        tracing::info!(session = %self.session_id, "the phone read the QR code; confirming");
        self.scanned = true;
        self.peer_info = Some(device);
        self.confirm().unwrap_or_else(|()| Step::Terminal(PairingState::Failed { reason: FailureReason::Handshake }, vec![Action::Close]))
    }

    fn deadline_after_handshake(&self) -> Instant {
        // `self.deadline` currently holds the handshake deadline start + handshake timeout;
        // re-base on "now" as best we know it (the last tick/event) — callers pass `now` to
        // `tick`, which refreshes precisely.
        self.deadline - self.timeouts.handshake + self.timeouts.verification
    }

    pub(crate) fn on_peer(&mut self, bytes: &[u8], now: Now) -> Step {
        match &mut self.phase {
            Phase::Handshaking(hs) => {
                if let Err(e) = hs.receive(bytes) {
                    tracing::warn!(session = %self.session_id, error = %e, "handshake message rejected");
                    return Step::Terminal(PairingState::Failed { reason: FailureReason::Handshake }, vec![Action::Close]);
                }
                // Refresh the verification deadline base to the real "now".
                self.deadline = now.instant + self.timeouts.handshake;
                self.pump()
            }
            Phase::Verifying(outcome) => {
                let plain = match outcome.cipher.decrypt(bytes) {
                    Ok(p) => p,
                    Err(_) => return Step::Terminal(PairingState::Failed { reason: FailureReason::Handshake }, vec![Action::Close]),
                };
                let msg = match AppMessage::decode(&plain) {
                    Ok(m) => m,
                    Err(_) => return Step::Terminal(PairingState::Failed { reason: FailureReason::Protocol }, vec![Action::Close]),
                };
                match msg {
                    AppMessage::PairConfirm { device, .. } => {
                        self.peer_confirmed = true;
                        self.peer_info = Some(device);
                        self.maybe_complete()
                    }
                    AppMessage::PairProof { proof, device, .. } => self.on_proof(&proof, device),
                    AppMessage::PairReject { .. } => Step::Terminal(PairingState::Rejected, vec![Action::Close]),
                    _ => Step::Terminal(PairingState::Failed { reason: FailureReason::Protocol }, vec![Action::Close]),
                }
            }
            Phase::Done => Step::Continue(Vec::new()),
        }
    }

    pub(crate) fn on_user_confirm(&mut self) -> Result<Step, ()> {
        self.confirm()
    }

    /// Send this side's `pair_confirm` (once), for its user or, after a valid proof, by itself.
    fn confirm(&mut self) -> Result<Step, ()> {
        let Phase::Verifying(outcome) = &mut self.phase else { return Err(()) };
        if self.local_confirmed {
            return Ok(Step::Continue(Vec::new()));
        }
        let msg = AppMessage::PairConfirm { version: ProtocolVersion::CURRENT, device: self.identity.info() };
        let plain = msg.encode().map_err(|_| ())?;
        let cipher_text = outcome.cipher.encrypt(&plain).map_err(|_| ())?;
        self.local_confirmed = true;
        let mut step = self.maybe_complete();
        // The confirm must go out before anything the completion produced.
        match &mut step {
            Step::Continue(actions) | Step::Terminal(_, actions) => actions.insert(0, Action::SendPeer(cipher_text)),
        }
        Ok(step)
    }

    pub(crate) fn on_user_reject(&mut self) -> Result<Step, ()> {
        let Phase::Verifying(outcome) = &mut self.phase else { return Err(()) };
        let msg = AppMessage::PairReject { version: ProtocolVersion::CURRENT, reason: RejectReason::UserDeclined };
        let plain = msg.encode().map_err(|_| ())?;
        let cipher_text = outcome.cipher.encrypt(&plain).map_err(|_| ())?;
        self.phase = Phase::Done;
        Ok(Step::Terminal(PairingState::Rejected, vec![Action::SendPeer(cipher_text), Action::Close]))
    }

    pub(crate) fn tick(&mut self, now: Now) -> Step {
        if matches!(self.phase, Phase::Done) {
            return Step::Continue(Vec::new());
        }
        if now.instant >= self.deadline {
            let state = match self.phase {
                Phase::Handshaking(_) => PairingState::Failed { reason: FailureReason::Timeout },
                _ => PairingState::Expired,
            };
            self.phase = Phase::Done;
            return Step::Terminal(state, vec![Action::Close]);
        }
        Step::Continue(Vec::new())
    }

    fn maybe_complete(&mut self) -> Step {
        if !(self.local_confirmed && self.peer_confirmed) {
            return Step::Continue(Vec::new());
        }
        let Phase::Verifying(outcome) = std::mem::replace(&mut self.phase, Phase::Done) else { return Step::Continue(Vec::new()) };
        let outcome = *outcome;
        let Some(peer) = self.peer_info.clone() else {
            return Step::Terminal(PairingState::Failed { reason: FailureReason::Protocol }, vec![Action::Close]);
        };
        let established =
            Established { session_id: self.session_id, cipher: outcome.cipher, remote_static: outcome.remote_static, peer, safety_code: outcome.safety_code };
        Step::Terminal(PairingState::Trusted, vec![Action::Trusted(Box::new(established))])
    }
}
