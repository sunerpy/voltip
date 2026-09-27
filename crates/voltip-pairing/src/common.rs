//! Types shared by both sides of the pairing state machine.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use voltip_crypto::{PublicKey, SafetyCode, SessionCipher};
use voltip_protocol::relay::{RelayErrorCode, RelayFrame};
use voltip_protocol::ticket::PairingTicket;
use voltip_protocol::{DeviceInfo, PairCode, SessionId};

/// Clock reading passed with every event: monotonic for deadlines, unix for ticket expiry.
#[derive(Clone, Copy, Debug)]
pub struct Now {
    /// Monotonic instant.
    pub instant: Instant,
    /// Unix seconds.
    pub unix_secs: u64,
}

impl Now {
    /// Convenience for tests and callers: read both clocks from the OS.
    pub fn system() -> Self {
        let unix_secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        Self { instant: Instant::now(), unix_secs }
    }

    /// Shift both clocks forward (tests).
    pub fn plus(self, d: Duration) -> Self {
        Self { instant: self.instant + d, unix_secs: self.unix_secs + d.as_secs() }
    }
}

/// Per-phase deadlines. Defaults mirror `docs/pairing.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timeouts {
    /// Waiting for the relay to answer `create_session` / `join_*`.
    pub relay_response: Duration,
    /// Noise handshake, message 1 → message 3.
    pub handshake: Duration,
    /// Both users comparing safety codes.
    pub verification: Duration,
    /// Session TTL to request from the relay.
    pub session_ttl: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            relay_response: Duration::from_secs(10),
            handshake: Duration::from_secs(15),
            verification: Duration::from_secs(120),
            session_ttl: Duration::from_secs(120),
        }
    }
}

/// Why pairing failed (terminal).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum FailureReason {
    /// A phase deadline passed.
    Timeout,
    /// A ticket / session was presented twice.
    Replay,
    /// Noise handshake failed (tampering, wrong ticket, corrupt message).
    Handshake,
    /// The relay refused.
    Relay {
        /// Relay's code.
        code: RelayErrorCode,
    },
    /// Peer sent something the protocol does not allow here.
    Protocol,
    /// Local user cancelled.
    Cancelled,
    /// Peer disconnected mid-way.
    PeerLeft,
    /// The peer's identity did not match an existing trusted record.
    IdentityChanged,
}

/// The state machine's position.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum PairingState {
    /// Nothing happening.
    Idle,
    /// Asked the relay for a session (initiator) / to join (responder).
    CreatingSession,
    /// Session exists; showing QR + code, waiting for the phone.
    WaitingForPeer,
    /// Noise XX in flight.
    KeyExchange,
    /// Handshake done; users compare safety codes.
    AwaitingVerification,
    /// Both confirmed. Terminal (success).
    Trusted,
    /// Session lifetime ran out before a peer joined / confirmed.
    Expired,
    /// A user rejected.
    Rejected,
    /// Terminal failure.
    Failed {
        /// Why.
        reason: FailureReason,
    },
}

impl PairingState {
    /// Terminal states accept only `Reset`.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Trusted | Self::Expired | Self::Rejected | Self::Failed { .. })
    }
}

/// How the responder found the session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JoinMethod {
    /// Typed six digits.
    Code(PairCode),
    /// Scanned QR.
    Ticket(PairingTicket),
}

/// Inputs to either machine.
#[derive(Debug)]
pub enum Event {
    /// Begin (initiator: create a session; responder: join).
    Start,
    /// A control frame from the relay (or synthesized by a direct transport).
    Relay(RelayFrame),
    /// Bytes from the peer (payload of a `forward`, or a direct-transport message).
    Peer(Vec<u8>),
    /// Periodic clock tick — drives every timeout.
    Tick,
    /// Local user compared the safety code and accepted.
    UserConfirm,
    /// Local user rejected.
    UserReject,
    /// Local user cancelled the whole thing.
    Cancel,
    /// Back to `Idle` from any terminal state.
    Reset,
}

impl Event {
    /// Stable name for errors and logs.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Relay(_) => "relay",
            Self::Peer(_) => "peer",
            Self::Tick => "tick",
            Self::UserConfirm => "user_confirm",
            Self::UserReject => "user_reject",
            Self::Cancel => "cancel",
            Self::Reset => "reset",
        }
    }
}

/// Everything the UI needs to render the pairing screen.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    /// Current state.
    pub state: PairingState,
    /// Session id once known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    /// `483 921` — initiator only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// `voltip://pair?...` — initiator only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket_uri: Option<String>,
    /// Unix seconds when the session dies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    /// Seconds left, clamped at zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining_secs: Option<u64>,
    /// Shown in `AwaitingVerification`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety_code: Option<SafetyCode>,
    /// Peer's self-description once it confirmed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer: Option<DeviceInfo>,
    /// Whether the local user already confirmed.
    pub local_confirmed: bool,
    /// Whether the peer already confirmed.
    pub peer_confirmed: bool,
}

/// A pairing that completed: everything the transport layer needs to keep talking.
pub struct Established {
    /// Session id (for `forward` frames on a relay).
    pub session_id: SessionId,
    /// Transport cipher, positioned right after the `pair_confirm` exchange.
    pub cipher: SessionCipher,
    /// Peer's authenticated static key — the trust anchor.
    pub remote_static: PublicKey,
    /// Peer's self-description.
    pub peer: DeviceInfo,
    /// Safety code the users compared.
    pub safety_code: SafetyCode,
}

impl std::fmt::Debug for Established {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Established")
            .field("session_id", &self.session_id)
            .field("peer", &self.peer)
            .field("remote_static", &self.remote_static)
            .finish_non_exhaustive()
    }
}

/// Side effects the caller must perform, in order.
#[derive(Debug)]
pub enum Action {
    /// Send a control frame to the relay.
    SendRelay(RelayFrame),
    /// Send bytes to the peer (wrap in `forward` on a relay, or write to a direct socket).
    SendPeer(Vec<u8>),
    /// State changed; re-render.
    Emit(Box<Snapshot>),
    /// Pairing finished successfully.
    Trusted(Box<Established>),
    /// Tear down the network session (after failure / rejection / cancel).
    Close,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_states() {
        assert!(PairingState::Trusted.is_terminal());
        assert!(PairingState::Expired.is_terminal());
        assert!(PairingState::Rejected.is_terminal());
        assert!(PairingState::Failed { reason: FailureReason::Timeout }.is_terminal());
        assert!(!PairingState::Idle.is_terminal());
        assert!(!PairingState::KeyExchange.is_terminal());
    }

    #[test]
    fn state_serializes_as_tagged_snake_case() {
        let json = serde_json::to_string(&PairingState::Failed { reason: FailureReason::Relay { code: RelayErrorCode::InvalidCode } }).unwrap();
        assert_eq!(json, r#"{"state":"failed","reason":{"kind":"relay","code":"invalid_code"}}"#);
        assert_eq!(serde_json::to_string(&PairingState::AwaitingVerification).unwrap(), r#"{"state":"awaiting_verification"}"#);
    }

    #[test]
    fn now_shifts_both_clocks_and_events_have_names() {
        let n = Now::system();
        let later = n.plus(Duration::from_secs(5));
        assert_eq!(later.unix_secs, n.unix_secs + 5);
        assert!(later.instant > n.instant);
        assert_eq!(Event::Start.name(), "start");
        assert_eq!(Event::Tick.name(), "tick");
        assert_eq!(Event::Peer(vec![]).name(), "peer");
        assert_eq!(Event::Relay(RelayFrame::error(RelayErrorCode::Malformed)).name(), "relay");
        assert_eq!(Event::UserConfirm.name(), "user_confirm");
        assert_eq!(Event::UserReject.name(), "user_reject");
        assert_eq!(Event::Cancel.name(), "cancel");
        assert_eq!(Event::Reset.name(), "reset");
        assert!(Timeouts::default().handshake < Timeouts::default().verification);
    }
}
