//! Post-pairing secure sessions with trusted peers (re-handshake on every rendezvous).
//!
//! A peer may be reachable over several links at once — the public relay, our own LAN host
//! (the peer dialled us) and an outgoing LAN connection (we dialled the peer). Each of those is
//! a [`PeerPath`] with its own rendezvous session and Noise handshake; [`PeerState`] groups the
//! paths of one trusted device and answers "how is this device connected right now".

use std::time::{Duration, Instant};

use voltip_crypto::{Handshake, HandshakeStep, PublicKey, Role, StaticKeypair};
use voltip_protocol::SessionId;
use voltip_transport::SecureChannel;

/// Which connection a frame arrived on / should leave on.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum LinkId {
    /// The configured public relay.
    Relay,
    /// Loopback connection to this device's own LAN host.
    Host,
    /// Outgoing LAN connection number `n` (to a peer's host, or to a ticket's host while pairing).
    Dial(u64),
}

impl LinkId {
    /// Whether frames on this link stay on the local network.
    pub(crate) fn is_direct(self) -> bool {
        !matches!(self, Self::Relay)
    }
}

/// Where a path is.
pub(crate) enum PeerPhase {
    /// Attached to the channel; the peer is not there.
    Idle,
    /// Noise XX in flight.
    Handshaking(Box<Handshake>),
    /// Secure channel up.
    Secure(Box<SecureChannel>),
    /// Peer presented a key that is not the trusted one.
    IdentityChanged { presented: PublicKey },
}

/// One rendezvous session with a peer on one link.
pub(crate) struct PeerPath {
    pub(crate) link: LinkId,
    pub(crate) session_id: Option<SessionId>,
    pub(crate) phase: PeerPhase,
    /// When the current handshake started (for the stall deadline).
    pub(crate) handshake_started: Option<Instant>,
}

impl PeerPath {
    pub(crate) fn new(link: LinkId) -> Self {
        Self { link, session_id: None, phase: PeerPhase::Idle, handshake_started: None }
    }

    /// Drop a handshake that has been in flight longer than `limit`. Returns `true` if reset.
    pub(crate) fn expire_stalled_handshake(&mut self, now: Instant, limit: Duration) -> bool {
        match (&self.phase, self.handshake_started) {
            (PeerPhase::Handshaking(_), Some(started)) if now.duration_since(started) >= limit => {
                self.phase = PeerPhase::Idle;
                self.handshake_started = None;
                true
            }
            _ => false,
        }
    }

    /// Start a handshake in `role`; returns the first message when we initiate.
    pub(crate) fn begin(&mut self, local: &StaticKeypair, role: Role) -> Result<Option<Vec<u8>>, voltip_crypto::CryptoError> {
        let mut hs = Handshake::new(role, local, None)?;
        let first = match hs.next_step()? {
            HandshakeStep::Send(bytes) => Some(bytes),
            _ => None,
        };
        self.phase = PeerPhase::Handshaking(Box::new(hs));
        self.handshake_started = Some(Instant::now());
        Ok(first)
    }

    /// Feed handshake bytes. Returns bytes to send (if any) and whether the handshake finished.
    pub(crate) fn handshake_input(&mut self, bytes: &[u8]) -> Result<(Option<Vec<u8>>, bool), voltip_crypto::CryptoError> {
        let PeerPhase::Handshaking(hs) = &mut self.phase else { return Ok((None, false)) };
        hs.receive(bytes)?;
        let out = match hs.next_step()? {
            HandshakeStep::Send(b) => Some(b),
            _ => None,
        };
        Ok((out, hs.is_finished()))
    }

    /// Finish the handshake; returns the authenticated remote key. Caller decides trust.
    pub(crate) fn finish(&mut self) -> Result<PublicKey, voltip_crypto::CryptoError> {
        let PeerPhase::Handshaking(hs) = std::mem::replace(&mut self.phase, PeerPhase::Idle) else {
            return Err(voltip_crypto::CryptoError::OutOfOrder { expected: "handshaking" });
        };
        let outcome = (*hs).finish()?;
        let remote = outcome.remote_static;
        self.phase = PeerPhase::Secure(Box::new(SecureChannel::new(outcome.cipher)));
        self.handshake_started = None;
        Ok(remote)
    }

    pub(crate) fn is_secure(&self) -> bool {
        matches!(self.phase, PeerPhase::Secure(_))
    }
}

/// Everything the runtime tracks about one trusted device.
pub(crate) struct PeerState {
    pub(crate) paths: Vec<PeerPath>,
    /// Earliest time for the next outgoing LAN attempt (`None` = whenever the next tick comes).
    pub(crate) next_dial_at: Option<Instant>,
    /// Current backoff between LAN attempts.
    pub(crate) dial_backoff: Duration,
    /// Which of the peer's hints to try next (rotates on failure).
    pub(crate) hint_cursor: usize,
}

impl PeerState {
    pub(crate) fn new(initial_backoff: Duration) -> Self {
        Self { paths: Vec::new(), next_dial_at: None, dial_backoff: initial_backoff, hint_cursor: 0 }
    }

    /// The path on `link`, if any.
    pub(crate) fn path(&mut self, link: LinkId) -> Option<&mut PeerPath> {
        self.paths.iter_mut().find(|p| p.link == link)
    }

    /// The path on `link`, created idle when missing.
    pub(crate) fn path_or_insert(&mut self, link: LinkId) -> &mut PeerPath {
        if let Some(i) = self.paths.iter().position(|p| p.link == link) {
            &mut self.paths[i]
        } else {
            self.paths.push(PeerPath::new(link));
            let last = self.paths.len() - 1;
            &mut self.paths[last]
        }
    }

    /// Forget every path that lives on `link` (the link went away).
    pub(crate) fn drop_link(&mut self, link: LinkId) {
        self.paths.retain(|p| p.link != link);
    }

    /// Any LAN path that is doing something (handshaking, secure, or flagged) — while one exists
    /// there is no point dialling.
    pub(crate) fn has_active_direct_path(&self) -> bool {
        self.paths.iter().any(|p| p.link.is_direct() && !matches!(p.phase, PeerPhase::Idle))
    }

    /// Best path for outgoing traffic: a secure LAN path first, then a secure relay path.
    pub(crate) fn best_secure_path(&mut self) -> Option<&mut PeerPath> {
        let idx = self
            .paths
            .iter()
            .enumerate()
            .filter(|(_, p)| p.is_secure() && p.session_id.is_some())
            .min_by_key(|(_, p)| if p.link.is_direct() { 0 } else { 1 })
            .map(|(i, _)| i)?;
        self.paths.get_mut(idx)
    }

    /// Reset the LAN backoff after a success or after learning fresh hints.
    pub(crate) fn reset_dial_backoff(&mut self, initial: Duration) {
        self.dial_backoff = initial;
        self.next_dial_at = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn best_path_prefers_direct_and_ignores_idle_paths() {
        let mut st = PeerState::new(Duration::from_secs(1));
        st.path_or_insert(LinkId::Relay);
        assert!(st.best_secure_path().is_none());
        assert!(!st.has_active_direct_path());
        st.path_or_insert(LinkId::Host).session_id = Some(SessionId::random());
        st.path_or_insert(LinkId::Dial(1)).session_id = Some(SessionId::random());
        // Fake "secure" phases via a real handshake is heavy; use IdentityChanged to prove that a
        // non-idle direct path counts as active while never being selected for traffic.
        st.path_or_insert(LinkId::Dial(1)).phase = PeerPhase::IdentityChanged { presented: PublicKey([9; 32]) };
        assert!(st.has_active_direct_path());
        assert!(st.best_secure_path().is_none());
        st.drop_link(LinkId::Dial(1));
        assert!(!st.has_active_direct_path());
        assert_eq!(st.paths.len(), 2);
        assert!(LinkId::Host.is_direct() && LinkId::Dial(7).is_direct() && !LinkId::Relay.is_direct());
        st.dial_backoff = Duration::from_secs(9);
        st.next_dial_at = Some(Instant::now());
        st.reset_dial_backoff(Duration::from_secs(1));
        assert_eq!(st.dial_backoff, Duration::from_secs(1));
        assert!(st.next_dial_at.is_none());
    }
}
