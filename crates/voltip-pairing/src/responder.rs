//! The side that joins a session by code or ticket (phone in phase 1).

use voltip_crypto::{Handshake, Role};
use voltip_identity::DeviceIdentity;
use voltip_protocol::relay::RelayFrame;
use voltip_protocol::{ProtocolVersion, SessionId};

use crate::common::{Action, Event, FailureReason, JoinMethod, Now, PairingState, Snapshot, Timeouts};
use crate::engine::{Engine, Proof, Step};
use crate::{NonceLedger, PairingError};

enum Phase {
    Idle,
    Joining { deadline: std::time::Instant },
    Engaged(Box<Engine>),
    Terminal(PairingState),
}

/// Responder state machine.
pub struct Responder {
    identity: DeviceIdentity,
    timeouts: Timeouts,
    method: JoinMethod,
    phase: Phase,
    handshake: Option<Handshake>,
    session_id: Option<SessionId>,
}

impl std::fmt::Debug for Responder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Responder").field("state", &self.state()).finish_non_exhaustive()
    }
}

impl Responder {
    /// Prepare to join. Ticket-based joins are validated against `ledger` (replay) and `now`
    /// (expiry) *before* anything is sent, and the ticket's nonce is recorded on success.
    pub fn new(identity: DeviceIdentity, timeouts: Timeouts, method: JoinMethod, ledger: &mut NonceLedger, now: Now) -> Result<Self, PairingState> {
        if let JoinMethod::Ticket(t) = &method {
            if t.is_expired_at(now.unix_secs) {
                return Err(PairingState::Expired);
            }
            if !ledger.record(t.nonce) {
                return Err(PairingState::Failed { reason: FailureReason::Replay });
            }
        }
        Ok(Self { identity, timeouts, method, phase: Phase::Idle, handshake: None, session_id: None })
    }

    /// Current state.
    pub fn state(&self) -> PairingState {
        match &self.phase {
            Phase::Idle => PairingState::Idle,
            Phase::Joining { .. } => PairingState::CreatingSession,
            Phase::Engaged(e) => e.state(),
            Phase::Terminal(s) => *s,
        }
    }

    /// UI snapshot.
    pub fn snapshot(&self, _now: Now) -> Snapshot {
        let mut snap = Snapshot {
            state: self.state(),
            session_id: self.session_id,
            code: None,
            ticket_uri: None,
            expires_at: None,
            remaining_secs: None,
            safety_code: None,
            peer: None,
            local_confirmed: false,
            peer_confirmed: false,
            scanned: false,
        };
        if let JoinMethod::Ticket(t) = &self.method {
            snap.expires_at = Some(t.expires_at);
        }
        if let Phase::Engaged(e) = &self.phase {
            snap.safety_code = e.safety_code();
            snap.peer = e.peer_info();
            snap.local_confirmed = e.local_confirmed();
            snap.peer_confirmed = e.peer_confirmed();
            snap.scanned = e.scanned();
        }
        snap
    }

    /// What the screen shows: the state and, once engaged, who confirmed.
    fn visible(&self) -> (PairingState, Option<[bool; 4]>) {
        (self.state(), if let Phase::Engaged(e) = &self.phase { Some(e.flags()) } else { None })
    }

    /// Feed one event. The last action is an `Emit` whenever what the screen shows changed.
    pub fn step(&mut self, event: Event, now: Now) -> Result<Vec<Action>, PairingError> {
        let before = self.state();
        let shown = self.visible();
        let mut actions = match (&mut self.phase, event) {
            (_, Event::Reset) => {
                self.phase = Phase::Idle;
                self.handshake = None;
                self.session_id = None;
                Vec::new()
            }
            (Phase::Idle, Event::Start) => {
                let expected = match &self.method {
                    JoinMethod::Ticket(t) => Some(t.ephemeral_pub),
                    JoinMethod::Code(_) => None,
                };
                self.handshake = Some(Handshake::new(Role::Responder, &self.identity.keypair, expected)?);
                self.phase = Phase::Joining { deadline: now.instant + self.timeouts.relay_response };
                let frame = match &self.method {
                    JoinMethod::Code(code) => RelayFrame::JoinByCode { version: ProtocolVersion::CURRENT, code: code.clone() },
                    JoinMethod::Ticket(t) => RelayFrame::JoinBySession { version: ProtocolVersion::CURRENT, session_id: t.session_id },
                };
                vec![Action::SendRelay(frame)]
            }
            (Phase::Joining { .. }, Event::Relay(RelayFrame::Joined { session_id, .. })) => {
                let mismatch = matches!(&self.method, JoinMethod::Ticket(t) if t.session_id != session_id);
                if mismatch {
                    let actions = self.fail(FailureReason::Protocol);
                    return Ok(self.finish_with(actions, shown, now));
                }
                self.session_id = Some(session_id);
                let Some(hs) = self.handshake.take() else { return Err(PairingError::InvalidTransition { event: "relay", state: before }) };
                // A ticket with a secret is proved right after the handshake; a typed code (the
                // relay knows it) or an older computer's ticket leaves both users to confirm.
                let proof = match &self.method {
                    JoinMethod::Ticket(t) => t.secret.map_or(Proof::None, Proof::Give),
                    JoinMethod::Code(_) => Proof::None,
                };
                let mut engine = Engine::new(session_id, self.identity.clone(), hs, self.timeouts, now, proof);
                let step = engine.pump(); // responder has nothing to send yet
                self.phase = Phase::Engaged(Box::new(engine));
                self.apply(step)
            }
            (Phase::Joining { deadline }, Event::Tick) => {
                if now.instant >= *deadline {
                    self.fail(FailureReason::Timeout)
                } else {
                    Vec::new()
                }
            }
            (Phase::Engaged(engine), Event::Peer(bytes)) => {
                let step = engine.on_peer(&bytes, now);
                self.apply(step)
            }
            (Phase::Engaged(engine), Event::Tick) => {
                let step = engine.tick(now);
                self.apply(step)
            }
            (Phase::Engaged(engine), Event::UserConfirm) => match engine.on_user_confirm() {
                Ok(step) => self.apply(step),
                Err(()) => return Err(PairingError::InvalidTransition { event: "user_confirm", state: before }),
            },
            (Phase::Engaged(engine), Event::UserReject) => match engine.on_user_reject() {
                Ok(step) => self.apply(step),
                Err(()) => return Err(PairingError::InvalidTransition { event: "user_reject", state: before }),
            },
            (Phase::Engaged(_), Event::Relay(RelayFrame::PeerLeft { .. })) => self.fail(FailureReason::PeerLeft),
            (_, Event::Relay(RelayFrame::Error { code, .. })) if !before.is_terminal() => self.fail(FailureReason::Relay { code }),
            (_, Event::Cancel) if !before.is_terminal() && before != PairingState::Idle => self.fail(FailureReason::Cancelled),
            (_, Event::Cancel) => Vec::new(),
            (_, Event::Relay(_)) => Vec::new(),
            (Phase::Terminal(_), Event::Tick | Event::Peer(_)) => Vec::new(),
            (_, other) => return Err(PairingError::InvalidTransition { event: other.name(), state: before }),
        };
        if self.visible() != shown {
            actions.push(Action::Emit(Box::new(self.snapshot(now))));
        }
        Ok(actions)
    }

    fn finish_with(&mut self, mut actions: Vec<Action>, shown: (PairingState, Option<[bool; 4]>), now: Now) -> Vec<Action> {
        if self.visible() != shown {
            actions.push(Action::Emit(Box::new(self.snapshot(now))));
        }
        actions
    }

    fn fail(&mut self, reason: FailureReason) -> Vec<Action> {
        self.phase = Phase::Terminal(PairingState::Failed { reason });
        self.handshake = None;
        vec![Action::Close]
    }

    fn apply(&mut self, step: Step) -> Vec<Action> {
        match step {
            Step::Continue(actions) => actions,
            Step::Terminal(state, actions) => {
                self.phase = Phase::Terminal(state);
                actions
            }
        }
    }
}
