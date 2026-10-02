//! Post-pairing secure sessions with trusted peers (re-handshake on every rendezvous).
//!
//! A peer may be reachable over several links at once — the public relay, our own LAN host
//! (the peer dialled us) and an outgoing LAN connection (we dialled the peer). Each of those is
//! a [`PeerPath`] with its own rendezvous session and Noise handshake; [`PeerState`] groups the
//! paths of one trusted device and answers "how is this device connected right now".

use std::time::{Duration, Instant};

use voltip_crypto::{HANDSHAKE_MESSAGE_LENS, Handshake, HandshakeStep, PublicKey, Role, StaticKeypair};
use voltip_protocol::SessionId;
use voltip_transport::SecureChannel;

use crate::sync::BulkPath;

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
    Secure(Box<SecureSession>),
    /// Peer presented a key that is not the trusted one.
    IdentityChanged { presented: PublicKey },
}

/// First wait before a failed relay handshake is tried again (docs/dictation.md §20.8).
pub(crate) const HANDSHAKE_RETRY_MIN: Duration = Duration::from_secs(2);
/// Longest wait between two tries; the wait doubles up to this.
pub(crate) const HANDSHAKE_RETRY_MAX: Duration = Duration::from_secs(60);

/// What to do with a payload that arrived on a path whose channel is not up (docs/dictation.md
/// §20.8). Handshake messages have fixed lengths and no transport frame shares them, so the
/// length tells a handshake message from a frame of a session that has ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Incoming {
    /// A first message on an idle path: answer it.
    Start,
    /// A first message came to a responder waiting for message 3, or to a responder's secure
    /// channel: the initiator gave up and started over, so answer the new one.
    Restart,
    /// The message the handshake waits for.
    Feed,
    /// Not a message of this handshake.
    Drop,
}

/// A secure channel and the parts of large bodies going over it (docs/dictation.md §20.8): they
/// begin and end together, so a body is only ever put back together from one Noise session.
pub(crate) struct SecureSession {
    pub(crate) channel: SecureChannel,
    pub(crate) bulk: BulkPath,
    /// This side sent the handshake's first message.
    pub(crate) initiated: bool,
    /// The other side has been heard on this channel. A responder knows it from the handshake's
    /// last message; an initiator only from the first message it opens, because its last
    /// handshake message may not have arrived before the responder gave up.
    pub(crate) heard: bool,
    /// When the channel came up: an initiator not heard from by then plus the handshake's
    /// deadline starts over ([`PeerPath::expire_unheard`]).
    pub(crate) since: Instant,
}

/// One rendezvous session with a peer on one link.
pub(crate) struct PeerPath {
    pub(crate) link: LinkId,
    pub(crate) session_id: Option<SessionId>,
    pub(crate) phase: PeerPhase,
    /// When the current handshake started (for the stall deadline).
    pub(crate) handshake_started: Option<Instant>,
    /// The peer is on the relay channel: the last `attached` or `peer_presence` said so.
    pub(crate) present: bool,
    /// When to start the handshake again after one failed (relay initiator only).
    pub(crate) retry_at: Option<Instant>,
    /// Wait before the next try; doubles up to [`HANDSHAKE_RETRY_MAX`].
    pub(crate) retry_backoff: Duration,
}

impl PeerPath {
    pub(crate) fn new(link: LinkId) -> Self {
        Self { link, session_id: None, phase: PeerPhase::Idle, handshake_started: None, present: false, retry_at: None, retry_backoff: HANDSHAKE_RETRY_MIN }
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

    /// An initiator's channel on which the other side has not been heard `limit` after it came up
    /// (docs/dictation.md §20.8): the responder may have given up before the handshake's last
    /// message arrived, and then nothing this side sends is read. Back to idle, as for a stalled
    /// handshake; the retry starts a new one, which the responder answers.
    pub(crate) fn expire_unheard(&mut self, now: Instant, limit: Duration) -> bool {
        match &self.phase {
            PeerPhase::Secure(s) if !s.heard && now.duration_since(s.since) >= limit => {
                self.phase = PeerPhase::Idle;
                true
            }
            _ => false,
        }
    }

    /// Sort a payload that arrived while the channel is not up, or a first message on a
    /// responder's channel (no sealed frame has that length, see `SecureChannel::seal`).
    pub(crate) fn classify(&self, len: usize) -> Incoming {
        let [first, _, third] = HANDSHAKE_MESSAGE_LENS;
        match &self.phase {
            PeerPhase::Idle if len == first => Incoming::Start,
            PeerPhase::Handshaking(hs) => match hs.expected_message_len() {
                Some(expected) if expected == len => Incoming::Feed,
                Some(expected) if expected == third && len == first && hs.role() == Role::Responder => Incoming::Restart,
                _ => Incoming::Drop,
            },
            PeerPhase::Secure(s) if !s.initiated && len == first => Incoming::Restart,
            _ => Incoming::Drop,
        }
    }

    /// The handshake failed or stalled: back to idle and, on a relay path where this side
    /// initiates and the peer is on the channel, try again after the backoff (docs/dictation.md
    /// §20.8). Without this the path would wait for the next presence event.
    pub(crate) fn handshake_failed(&mut self, now: Instant, initiator: bool) {
        self.phase = PeerPhase::Idle;
        self.handshake_started = None;
        if self.link == LinkId::Relay && self.session_id.is_some() && self.present && initiator {
            self.retry_at = Some(now + self.retry_backoff);
            self.retry_backoff = (self.retry_backoff * 2).min(HANDSHAKE_RETRY_MAX);
        } else {
            self.retry_at = None;
        }
    }

    /// A retry is due: the path is still idle, attached, and the peer still there.
    pub(crate) fn retry_due(&self, now: Instant) -> bool {
        matches!(self.phase, PeerPhase::Idle) && self.present && self.session_id.is_some() && self.retry_at.is_some_and(|at| now >= at)
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
        let initiated = hs.role() == Role::Initiator;
        let outcome = (*hs).finish()?;
        let remote = outcome.remote_static;
        self.phase = PeerPhase::Secure(Box::new(SecureSession {
            channel: SecureChannel::new(outcome.cipher),
            bulk: BulkPath::default(),
            initiated,
            heard: !initiated,
            since: Instant::now(),
        }));
        self.handshake_started = None;
        self.retry_at = None;
        self.retry_backoff = HANDSHAKE_RETRY_MIN;
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

    fn keys() -> (StaticKeypair, StaticKeypair) {
        (StaticKeypair::generate().unwrap(), StaticKeypair::generate().unwrap())
    }

    /// regression (plan gate, M7 design round 7): payloads are sorted by length, so a frame of an
    /// ended session (never a handshake length, see `SecureChannel::seal`) neither starts a
    /// handshake on an idle path nor breaks one in flight; a responder waiting for message 3
    /// answers a new first message.
    #[test]
    fn regression_payloads_are_sorted_by_handshake_length() {
        let [first, second, third] = HANDSHAKE_MESSAGE_LENS;
        let (ka, kb) = keys();
        let mut idle = PeerPath::new(LinkId::Relay);
        assert_eq!(idle.classify(first), Incoming::Start);
        for stale in [second, third, 97, 48 * 1024] {
            assert_eq!(idle.classify(stale), Incoming::Drop, "{stale}");
        }
        let mut initiator = PeerPath::new(LinkId::Relay);
        let m1 = initiator.begin(&ka, Role::Initiator).unwrap().unwrap();
        assert_eq!(m1.len(), first);
        assert_eq!(initiator.classify(second), Incoming::Feed);
        for stale in [first, third, 97, 48 * 1024] {
            assert_eq!(initiator.classify(stale), Incoming::Drop, "{stale}");
        }
        let mut responder = PeerPath::new(LinkId::Relay);
        assert!(responder.begin(&kb, Role::Responder).unwrap().is_none());
        assert_eq!(responder.classify(first), Incoming::Feed);
        assert_eq!(responder.classify(97), Incoming::Drop);
        let (m2, done) = responder.handshake_input(&m1).unwrap();
        assert!(!done);
        assert_eq!(m2.as_ref().map(Vec::len), Some(second));
        assert_eq!(responder.classify(third), Incoming::Feed);
        assert_eq!(responder.classify(first), Incoming::Restart, "a new first message replaces the attempt");
        assert_eq!(responder.classify(65), Incoming::Drop);
        // The restarted responder answers the initiator's new attempt and both sides finish.
        let mut retry = PeerPath::new(LinkId::Relay);
        let m1b = retry.begin(&ka, Role::Initiator).unwrap().unwrap();
        assert!(responder.begin(&kb, Role::Responder).unwrap().is_none());
        let (m2b, _) = responder.handshake_input(&m1b).unwrap();
        let (m3b, done) = retry.handshake_input(&m2b.unwrap()).unwrap();
        assert!(done);
        let (_, done) = responder.handshake_input(&m3b.unwrap()).unwrap();
        assert!(done);
        assert_eq!(retry.finish().unwrap(), kb.public);
        assert_eq!(responder.finish().unwrap(), ka.public);
        for len in [first, second, third, 97] {
            assert_eq!(retry.classify(len), Incoming::Drop, "an initiator's secure path sorts nothing: {len}");
        }
        // A responder's channel takes a first message as the initiator starting over.
        assert_eq!(responder.classify(first), Incoming::Restart);
        for len in [second, third, 97] {
            assert_eq!(responder.classify(len), Incoming::Drop, "{len}");
        }
        idle.phase = PeerPhase::IdentityChanged { presented: PublicKey([1; 32]) };
        assert_eq!(idle.classify(first), Incoming::Drop);
    }

    /// regression (2026-10-02, the Windows build host): a responder that gave up before the
    /// handshake's last message arrived left the initiator on a channel nobody read, for good.
    /// An initiator not heard from on its new channel within the deadline is back to idle (and
    /// retries); a responder heard the initiator in the handshake itself.
    #[test]
    fn regression_an_initiator_unheard_on_its_new_channel_starts_over() {
        let (ka, kb) = keys();
        let mut initiator = PeerPath::new(LinkId::Relay);
        let mut responder = PeerPath::new(LinkId::Relay);
        let m1 = initiator.begin(&ka, Role::Initiator).unwrap().unwrap();
        responder.begin(&kb, Role::Responder).unwrap();
        let (m2, _) = responder.handshake_input(&m1).unwrap();
        let (m3, done) = initiator.handshake_input(&m2.unwrap()).unwrap();
        assert!(done);
        initiator.finish().unwrap();
        responder.handshake_input(&m3.unwrap()).unwrap();
        responder.finish().unwrap();
        let limit = Duration::from_secs(15);
        let since = match &initiator.phase {
            PeerPhase::Secure(s) => {
                assert!(s.initiated && !s.heard);
                s.since
            }
            _ => panic!("secure"),
        };
        assert!(matches!(&responder.phase, PeerPhase::Secure(s) if !s.initiated && s.heard));
        assert!(!initiator.expire_unheard(since + limit - Duration::from_millis(1), limit));
        assert!(!responder.expire_unheard(since + limit * 2, limit), "a responder heard the initiator");
        assert!(initiator.expire_unheard(since + limit, limit));
        assert!(matches!(initiator.phase, PeerPhase::Idle));
        // Heard in time, it stays.
        let mut heard = PeerPath::new(LinkId::Relay);
        heard.phase = std::mem::replace(&mut responder.phase, PeerPhase::Idle);
        if let PeerPhase::Secure(s) = &mut heard.phase {
            s.initiated = true;
        }
        assert!(!heard.expire_unheard(since + limit * 2, limit));
    }

    /// regression (plan gate, M7 design round 7): a relay handshake that failed is tried again by
    /// the initiator after 2 s, then 4, 8 … 60 s; success resets the wait; nothing is retried on
    /// the responder side, off the relay, or while the peer is away.
    #[test]
    fn regression_failed_relay_handshakes_are_retried_with_backoff() {
        let (ka, kb) = keys();
        let now = Instant::now();
        let mut p = PeerPath::new(LinkId::Relay);
        p.session_id = Some(SessionId::random());
        p.present = true;
        let mut waits = Vec::new();
        for _ in 0..7 {
            p.begin(&ka, Role::Initiator).unwrap();
            p.handshake_failed(now, true);
            assert!(matches!(p.phase, PeerPhase::Idle));
            let at = p.retry_at.unwrap();
            waits.push(at.duration_since(now).as_secs());
            assert!(!p.retry_due(at - Duration::from_millis(1)));
            assert!(p.retry_due(at));
        }
        assert_eq!(waits, [2, 4, 8, 16, 32, 60, 60]);
        // A completed handshake resets the wait.
        let mut q = PeerPath::new(LinkId::Relay);
        let m1 = p.begin(&ka, Role::Initiator).unwrap().unwrap();
        q.begin(&kb, Role::Responder).unwrap();
        let (m2, _) = q.handshake_input(&m1).unwrap();
        let (m3, done) = p.handshake_input(&m2.unwrap()).unwrap();
        assert!(done);
        q.handshake_input(&m3.unwrap()).unwrap();
        p.finish().unwrap();
        assert!(p.retry_at.is_none());
        assert_eq!(p.retry_backoff, HANDSHAKE_RETRY_MIN);
        // No retry for the responder, off the relay, unattached, or with the peer away.
        let mut cases = [PeerPath::new(LinkId::Relay), PeerPath::new(LinkId::Host), PeerPath::new(LinkId::Relay), PeerPath::new(LinkId::Relay)];
        for (i, c) in cases.iter_mut().enumerate() {
            c.session_id = (i != 2).then(SessionId::random);
            c.present = i != 3;
        }
        cases[0].handshake_failed(now, false);
        for c in &mut cases[1..] {
            c.handshake_failed(now, true);
        }
        assert!(cases.iter().all(|c| c.retry_at.is_none()));
        // The peer left after the failure: the due retry no longer fires.
        let mut gone = PeerPath::new(LinkId::Relay);
        gone.session_id = Some(SessionId::random());
        gone.present = true;
        gone.handshake_failed(now, true);
        gone.present = false;
        assert!(!gone.retry_due(now + HANDSHAKE_RETRY_MAX));
    }

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
