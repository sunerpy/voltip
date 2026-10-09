//! The side that creates the pairing session and shows QR + code (desktop in phase 1).

use voltip_crypto::{Handshake, Role};
use voltip_identity::DeviceIdentity;
use voltip_protocol::relay::RelayFrame;
use voltip_protocol::ticket::{NONCE_LEN, PairingTicket};
use voltip_protocol::{PairCode, ProtocolVersion, SessionId};

use crate::PairingError;
use crate::common::{Action, Event, FailureReason, Now, PairingState, Snapshot, Timeouts};
use crate::engine::{Engine, Step};

/// Where the initiator can be reached, embedded in the ticket.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reachability {
    /// Relay the initiator is connected to, if any.
    pub relay_hint: Option<url::Url>,
    /// LAN listeners (`ip:port`) for a direct connection.
    pub direct_hints: Vec<String>,
}

struct Waiting {
    session_id: SessionId,
    code: PairCode,
    ticket: PairingTicket,
    ticket_uri: String,
    expires_at: u64,
}

enum Phase {
    Idle,
    CreatingSession { deadline: std::time::Instant },
    WaitingForPeer(Box<Waiting>),
    Engaged(Box<Engine>),
    Terminal(PairingState),
}

/// Initiator state machine.
pub struct Initiator {
    identity: DeviceIdentity,
    timeouts: Timeouts,
    reach: Reachability,
    phase: Phase,
    handshake: Option<Handshake>,
    last_expires_at: Option<u64>,
    joined_once: bool,
}

impl std::fmt::Debug for Initiator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Initiator").field("state", &self.state()).finish_non_exhaustive()
    }
}

impl Initiator {
    /// New idle initiator.
    pub fn new(identity: DeviceIdentity, timeouts: Timeouts, reach: Reachability) -> Self {
        Self { identity, timeouts, reach, phase: Phase::Idle, handshake: None, last_expires_at: None, joined_once: false }
    }

    /// Current state.
    pub fn state(&self) -> PairingState {
        match &self.phase {
            Phase::Idle => PairingState::Idle,
            Phase::CreatingSession { .. } => PairingState::CreatingSession,
            Phase::WaitingForPeer(_) => PairingState::WaitingForPeer,
            Phase::Engaged(e) => e.state(),
            Phase::Terminal(s) => *s,
        }
    }

    /// The ticket being shown, while waiting for a peer.
    pub fn ticket(&self) -> Option<&PairingTicket> {
        match &self.phase {
            Phase::WaitingForPeer(w) => Some(&w.ticket),
            _ => None,
        }
    }

    /// UI snapshot.
    pub fn snapshot(&self, now: Now) -> Snapshot {
        let mut snap = Snapshot {
            state: self.state(),
            session_id: None,
            code: None,
            ticket_uri: None,
            expires_at: self.last_expires_at,
            remaining_secs: None,
            safety_code: None,
            peer: None,
            local_confirmed: false,
            peer_confirmed: false,
        };
        match &self.phase {
            Phase::WaitingForPeer(w) => {
                snap.session_id = Some(w.session_id);
                snap.code = Some(w.code.display_grouped());
                snap.ticket_uri = Some(w.ticket_uri.clone());
                snap.expires_at = Some(w.expires_at);
                snap.remaining_secs = Some(w.expires_at.saturating_sub(now.unix_secs));
            }
            Phase::Engaged(e) => {
                snap.session_id = Some(e.session_id);
                snap.safety_code = e.safety_code();
                snap.peer = e.peer_info();
                snap.local_confirmed = e.local_confirmed();
                snap.peer_confirmed = e.peer_confirmed();
            }
            _ => {}
        }
        snap
    }

    /// Feed one event. Returns the actions to perform, in order. The last action is an
    /// `Emit` whenever the visible state changed.
    pub fn step(&mut self, event: Event, now: Now) -> Result<Vec<Action>, PairingError> {
        let before = self.state();
        let mut actions = match (&mut self.phase, event) {
            (_, Event::Reset) => {
                self.phase = Phase::Idle;
                self.handshake = None;
                self.joined_once = false;
                self.last_expires_at = None;
                Vec::new()
            }
            (Phase::Idle, Event::Start) => {
                let hs = Handshake::new(Role::Initiator, &self.identity.keypair, None)?;
                self.handshake = Some(hs);
                self.phase = Phase::CreatingSession { deadline: now.instant + self.timeouts.relay_response };
                let ttl = u32::try_from(self.timeouts.session_ttl.as_secs()).unwrap_or(u32::MAX);
                vec![Action::SendRelay(RelayFrame::CreateSession { version: ProtocolVersion::CURRENT, ttl_secs: Some(ttl) })]
            }
            (Phase::CreatingSession { .. }, Event::Relay(RelayFrame::SessionCreated { session_id, code, expires_at, .. })) => {
                let Some(hs) = &self.handshake else { return Err(PairingError::InvalidTransition { event: "relay", state: before }) };
                let ephemeral_pub = hs.initiator_ephemeral().ok_or(PairingError::InvalidTransition { event: "relay", state: before })?;
                let nonce: [u8; NONCE_LEN] = voltip_crypto::random_nonce();
                let ticket = PairingTicket {
                    version: ProtocolVersion::CURRENT,
                    session_id,
                    ephemeral_pub,
                    nonce,
                    expires_at,
                    relay_hint: self.reach.relay_hint.clone(),
                    direct_hints: self.reach.direct_hints.clone(),
                };
                let ticket_uri = ticket.to_uri()?;
                self.last_expires_at = Some(expires_at);
                self.phase = Phase::WaitingForPeer(Box::new(Waiting { session_id, code, ticket, ticket_uri, expires_at }));
                Vec::new()
            }
            (Phase::CreatingSession { deadline }, Event::Tick) => {
                if now.instant >= *deadline {
                    self.fail(FailureReason::Timeout)
                } else {
                    Vec::new()
                }
            }
            (Phase::WaitingForPeer(w), Event::Relay(RelayFrame::PeerJoined { session_id: joined, .. })) => {
                let session_id = w.session_id;
                if joined != session_id {
                    self.fail(FailureReason::Protocol)
                } else if self.joined_once {
                    self.fail(FailureReason::Replay)
                } else {
                    self.joined_once = true;
                    let Some(hs) = self.handshake.take() else { return Err(PairingError::InvalidTransition { event: "relay", state: before }) };
                    let mut engine = Engine::new(session_id, self.identity.clone(), hs, self.timeouts, now);
                    let step = engine.pump();
                    self.phase = Phase::Engaged(Box::new(engine));
                    self.apply(step)
                }
            }
            (Phase::WaitingForPeer(w), Event::Tick) => {
                if now.unix_secs >= w.expires_at {
                    self.phase = Phase::Terminal(PairingState::Expired);
                    vec![Action::Close]
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
            (Phase::Engaged(_), Event::Relay(RelayFrame::PeerJoined { .. })) => self.fail(FailureReason::Replay),
            (Phase::Engaged(_) | Phase::WaitingForPeer(_), Event::Relay(RelayFrame::PeerLeft { .. })) => self.fail(FailureReason::PeerLeft),
            (_, Event::Relay(RelayFrame::Error { code, .. })) if !before.is_terminal() => self.fail(FailureReason::Relay { code }),
            (_, Event::Cancel) if !before.is_terminal() && before != PairingState::Idle => self.fail(FailureReason::Cancelled),
            (_, Event::Cancel) => Vec::new(),
            (_, Event::Relay(_)) => Vec::new(), // Unrelated relay frames (presence, acks) are ignored.
            (Phase::Terminal(_), Event::Tick | Event::Peer(_)) => Vec::new(),
            (_, other) => return Err(PairingError::InvalidTransition { event: other.name(), state: before }),
        };
        if self.state() != before {
            actions.push(Action::Emit(Box::new(self.snapshot(now))));
        }
        Ok(actions)
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
