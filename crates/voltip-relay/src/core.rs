//! Sans-IO relay logic.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use voltip_protocol::relay::{DEFAULT_SESSION_TTL_SECS, MAX_SESSION_TTL_SECS, RelayErrorCode, RelayFrame, RelayLimits, validate_channel};
use voltip_protocol::{PairCode, ProtocolVersion, SessionId};

use crate::limits::{RateLimiter, RateWindow};

/// Opaque connection identifier assigned by the I/O layer.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct ConnId(pub u64);

/// Tunables. Defaults implement `docs/pairing.md` § Relay 侧防护.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelayConfig {
    /// Default session lifetime.
    pub session_ttl: Duration,
    /// Maximum session lifetime a client may request.
    pub max_session_ttl: Duration,
    /// Failed `join_by_code` per connection before the connection is closed.
    pub code_attempts_per_connection: u32,
    /// Failed attempts against one session before it is voided.
    pub code_attempts_per_session: u32,
    /// `join_*` per IP.
    pub join_rate: RateWindow,
    /// `create_session` per IP.
    pub create_rate: RateWindow,
    /// `attach` per IP.
    pub attach_rate: RateWindow,
    /// Max `forward` payload the relay will carry (bytes, decoded).
    pub max_forward_bytes: usize,
    /// Build string returned in `hello_ack`.
    pub relay_version: String,
    /// The server pings every connection this often (docs/pairing.md 「中继侧的连接检测」).
    pub ping_interval: Duration,
    /// A connection the relay hears nothing from for this long (no frame, no pong) is closed: a
    /// device that left without closing (another network, a frozen app) must not keep its place
    /// on a rendezvous channel, or its next connection finds the channel full.
    pub idle_timeout: Duration,
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            session_ttl: Duration::from_secs(u64::from(DEFAULT_SESSION_TTL_SECS)),
            max_session_ttl: Duration::from_secs(u64::from(MAX_SESSION_TTL_SECS)),
            code_attempts_per_connection: 5,
            code_attempts_per_session: 10,
            join_rate: RateWindow::new(20, Duration::from_secs(60)),
            create_rate: RateWindow::new(10, Duration::from_secs(60)),
            attach_rate: RateWindow::new(30, Duration::from_secs(60)),
            max_forward_bytes: 70 * 1024,
            relay_version: format!("voltip-relay/{}", env!("CARGO_PKG_VERSION")),
            ping_interval: Duration::from_secs(10),
            idle_timeout: Duration::from_secs(25),
        }
    }
}

impl RelayConfig {
    fn limits(&self) -> RelayLimits {
        RelayLimits {
            code_attempts_per_connection: self.code_attempts_per_connection,
            code_attempts_per_session: self.code_attempts_per_session,
            session_ttl_secs: u32::try_from(self.session_ttl.as_secs()).unwrap_or(u32::MAX),
        }
    }
}

/// Something to send to a connection, or an instruction to close it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Delivery {
    /// Send this frame.
    Send(ConnId, RelayFrame),
    /// Close this connection (after any frames queued before it).
    Close(ConnId),
}

#[derive(Debug)]
struct Connection {
    ip: IpAddr,
    greeted: bool,
    failed_code_attempts: u32,
    /// Pairing session this connection created or joined.
    session: Option<SessionId>,
    /// Channel bindings this connection is attached to (one per trusted peer).
    channel_sessions: Vec<SessionId>,
}

#[derive(Debug)]
struct PairingSession {
    code: PairCode,
    creator: ConnId,
    joiner: Option<ConnId>,
    expires_at: Instant,
    expires_at_unix: u64,
    failed_attempts: u32,
}

#[derive(Debug)]
struct Channel {
    session_id: SessionId,
    members: Vec<ConnId>,
}

/// Counters for metrics endpoints and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RelayStats {
    /// Live connections.
    pub connections: usize,
    /// Live pairing sessions.
    pub sessions: usize,
    /// Live channels (with ≥1 member).
    pub channels: usize,
    /// Frames forwarded since start.
    pub forwarded: u64,
    /// Frames dropped since start because a connection's outbound queue was full (counted by
    /// the server that owns the queues; the core reports `0`).
    pub dropped: u64,
}

/// The relay's entire state and logic.
pub struct RelayCore {
    config: RelayConfig,
    conns: HashMap<ConnId, Connection>,
    sessions: HashMap<SessionId, PairingSession>,
    codes: HashMap<PairCode, SessionId>,
    channels: HashMap<String, Channel>,
    channel_by_session: HashMap<SessionId, String>,
    join_limiter: RateLimiter<IpAddr>,
    create_limiter: RateLimiter<IpAddr>,
    attach_limiter: RateLimiter<IpAddr>,
    forwarded: u64,
}

impl std::fmt::Debug for RelayCore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayCore").field("stats", &self.stats()).finish_non_exhaustive()
    }
}

impl RelayCore {
    /// New empty relay.
    pub fn new(config: RelayConfig) -> Self {
        Self {
            join_limiter: RateLimiter::new(config.join_rate),
            create_limiter: RateLimiter::new(config.create_rate),
            attach_limiter: RateLimiter::new(config.attach_rate),
            config,
            conns: HashMap::new(),
            sessions: HashMap::new(),
            codes: HashMap::new(),
            channels: HashMap::new(),
            channel_by_session: HashMap::new(),
            forwarded: 0,
        }
    }

    /// Configuration in use.
    pub fn config(&self) -> &RelayConfig {
        &self.config
    }

    /// Snapshot counters.
    pub fn stats(&self) -> RelayStats {
        RelayStats { connections: self.conns.len(), sessions: self.sessions.len(), channels: self.channels.len(), forwarded: self.forwarded, dropped: 0 }
    }

    /// A transport accepted a connection.
    pub fn on_connect(&mut self, conn: ConnId, ip: IpAddr) {
        self.conns.insert(conn, Connection { ip, greeted: false, failed_code_attempts: 0, session: None, channel_sessions: Vec::new() });
    }

    /// A connection went away (any reason). Peers are told; sessions the connection created
    /// are destroyed (a half-finished pairing must not be resumable by a stranger).
    pub fn on_disconnect(&mut self, conn: ConnId, now: Instant) -> Vec<Delivery> {
        let mut out = Vec::new();
        let Some(c) = self.conns.remove(&conn) else { return out };
        if let Some(sid) = c.session
            && let Some(s) = self.sessions.get(&sid)
        {
            let other = if s.creator == conn { s.joiner } else { Some(s.creator) };
            if let Some(o) = other {
                out.push(Delivery::Send(o, RelayFrame::PeerLeft { version: ProtocolVersion::CURRENT, session_id: sid }));
                if let Some(oc) = self.conns.get_mut(&o) {
                    oc.session = None;
                }
            }
            self.remove_session(sid);
        }
        for csid in c.channel_sessions {
            if let Some(name) = self.channel_by_session.get(&csid).cloned()
                && let Some(ch) = self.channels.get_mut(&name)
            {
                ch.members.retain(|m| *m != conn);
                for m in &ch.members {
                    out.push(Delivery::Send(*m, RelayFrame::PeerPresence { version: ProtocolVersion::CURRENT, session_id: csid, online: false }));
                }
                if ch.members.is_empty() {
                    self.channels.remove(&name);
                    self.channel_by_session.remove(&csid);
                }
            }
        }
        self.expire(now, &mut out);
        out
    }

    /// Housekeeping tick: expire sessions. Call every second or so.
    pub fn tick(&mut self, now: Instant) -> Vec<Delivery> {
        let mut out = Vec::new();
        self.expire(now, &mut out);
        out
    }

    /// A text frame arrived from `conn`.
    pub fn on_text(&mut self, conn: ConnId, text: &str, now: Instant) -> Vec<Delivery> {
        let frame = match RelayFrame::decode(text) {
            Ok(f) => f,
            Err(voltip_protocol::CodecError::Version(_)) => {
                return vec![Delivery::Send(conn, RelayFrame::error(RelayErrorCode::UnsupportedVersion)), Delivery::Close(conn)];
            }
            Err(_) => return vec![Delivery::Send(conn, RelayFrame::error(RelayErrorCode::Malformed))],
        };
        self.on_frame(conn, frame, now)
    }

    /// A decoded frame arrived from `conn`.
    pub fn on_frame(&mut self, conn: ConnId, frame: RelayFrame, now: Instant) -> Vec<Delivery> {
        let mut out = Vec::new();
        self.expire(now, &mut out);
        let Some(c) = self.conns.get(&conn) else { return out };
        if !c.greeted && !matches!(frame, RelayFrame::Hello { .. }) {
            out.push(Delivery::Send(conn, RelayFrame::error(RelayErrorCode::HelloRequired)));
            out.push(Delivery::Close(conn));
            return out;
        }
        match frame {
            RelayFrame::Hello { .. } => {
                if let Some(c) = self.conns.get_mut(&conn) {
                    c.greeted = true;
                }
                out.push(Delivery::Send(
                    conn,
                    RelayFrame::HelloAck { version: ProtocolVersion::CURRENT, relay_version: self.config.relay_version.clone(), limits: self.config.limits() },
                ));
            }
            RelayFrame::CreateSession { ttl_secs, .. } => self.create_session(conn, ttl_secs, now, &mut out),
            RelayFrame::JoinByCode { code, .. } => self.join(conn, JoinKey::Code(code), now, &mut out),
            RelayFrame::JoinBySession { session_id, .. } => self.join(conn, JoinKey::Session(session_id), now, &mut out),
            RelayFrame::Leave { session_id, .. } => self.leave(conn, session_id, &mut out),
            RelayFrame::Forward { session_id, payload, .. } => self.forward(conn, session_id, payload, &mut out),
            RelayFrame::Attach { channel, .. } => self.attach(conn, channel, now, &mut out),
            RelayFrame::Bye { .. } => out.push(Delivery::Close(conn)),
            // Server-only frames from a client are protocol misuse: ignore, log.
            RelayFrame::HelloAck { .. }
            | RelayFrame::SessionCreated { .. }
            | RelayFrame::Joined { .. }
            | RelayFrame::PeerJoined { .. }
            | RelayFrame::Attached { .. }
            | RelayFrame::PeerPresence { .. }
            | RelayFrame::PeerLeft { .. }
            | RelayFrame::Error { .. } => {
                tracing::debug!(?conn, "ignoring server-only frame from client");
            }
        }
        out
    }

    fn create_session(&mut self, conn: ConnId, ttl_secs: Option<u32>, now: Instant, out: &mut Vec<Delivery>) {
        let Some(c) = self.conns.get(&conn) else { return };
        if c.session.is_some() {
            out.push(Delivery::Send(conn, RelayFrame::error(RelayErrorCode::SessionAlreadyActive)));
            return;
        }
        let ip = c.ip;
        if let Err(retry) = self.create_limiter.check(ip, now) {
            out.push(Delivery::Send(conn, RelayFrame::rate_limited(secs(retry))));
            return;
        }
        let ttl = ttl_secs.map_or(self.config.session_ttl, |s| Duration::from_secs(u64::from(s))).min(self.config.max_session_ttl).max(Duration::from_secs(10));
        let code = self.mint_code();
        let session_id = SessionId::random();
        let expires_at_unix = unix_now().saturating_add(ttl.as_secs());
        self.sessions
            .insert(session_id, PairingSession { code: code.clone(), creator: conn, joiner: None, expires_at: now + ttl, expires_at_unix, failed_attempts: 0 });
        self.codes.insert(code.clone(), session_id);
        if let Some(c) = self.conns.get_mut(&conn) {
            c.session = Some(session_id);
        }
        tracing::info!(%session_id, ttl_secs = ttl.as_secs(), "pairing session created");
        out.push(Delivery::Send(conn, RelayFrame::SessionCreated { version: ProtocolVersion::CURRENT, session_id, code, expires_at: expires_at_unix }));
    }

    fn mint_code(&self) -> PairCode {
        use rand::RngExt as _;
        // Avoid handing out a code that is live for another session; the space is 10^6 so a
        // handful of retries is always enough.
        for _ in 0..32 {
            let n: u32 = rand::rng().random_range(0..1_000_000);
            if let Ok(code) = PairCode::from_u32(n)
                && !self.codes.contains_key(&code)
            {
                return code;
            }
        }
        // Astronomically unlikely; fall back to a linear scan for a free code.
        (0..1_000_000u32)
            .filter_map(|n| PairCode::from_u32(n).ok())
            .find(|c| !self.codes.contains_key(c))
            .unwrap_or_else(|| unreachable!("a million codes cannot all be live"))
    }

    fn join(&mut self, conn: ConnId, key: JoinKey, now: Instant, out: &mut Vec<Delivery>) {
        let Some(c) = self.conns.get(&conn) else { return };
        if c.session.is_some() {
            out.push(Delivery::Send(conn, RelayFrame::error(RelayErrorCode::SessionAlreadyActive)));
            return;
        }
        let ip = c.ip;
        if let Err(retry) = self.join_limiter.check(ip, now) {
            out.push(Delivery::Send(conn, RelayFrame::rate_limited(secs(retry))));
            return;
        }
        let looked_up = match &key {
            JoinKey::Code(code) => self.codes.get(code).copied(),
            JoinKey::Session(sid) => self.sessions.contains_key(sid).then_some(*sid),
        };
        let Some(session_id) = looked_up else {
            self.record_failed_join(conn, &key, out);
            return;
        };
        let Some(session) = self.sessions.get_mut(&session_id) else { return };
        if session.joiner.is_some() || session.creator == conn {
            out.push(Delivery::Send(conn, RelayFrame::error(RelayErrorCode::SessionFull)));
            return;
        }
        session.joiner = Some(conn);
        let creator = session.creator;
        // The code is single-use: drop it from the index the moment somebody joins.
        self.codes.remove(&session.code);
        if let Some(c) = self.conns.get_mut(&conn) {
            c.session = Some(session_id);
        }
        tracing::info!(%session_id, "peer joined pairing session");
        out.push(Delivery::Send(conn, RelayFrame::Joined { version: ProtocolVersion::CURRENT, session_id }));
        out.push(Delivery::Send(creator, RelayFrame::PeerJoined { version: ProtocolVersion::CURRENT, session_id }));
    }

    fn record_failed_join(&mut self, conn: ConnId, key: &JoinKey, out: &mut Vec<Delivery>) {
        // Code guessing counts against the connection; a wrong session id is just an error.
        let error = match key {
            JoinKey::Code(_) => RelayErrorCode::InvalidCode,
            JoinKey::Session(_) => RelayErrorCode::SessionExpired,
        };
        out.push(Delivery::Send(conn, RelayFrame::error(error)));
        if matches!(key, JoinKey::Code(_)) {
            // Every live session absorbs a failed attempt: an attacker enumerating codes is
            // trying to hit *some* session, and voiding them all after N misses caps the
            // expected number of guesses regardless of how many sessions are live.
            let mut voided = Vec::new();
            for (sid, s) in &mut self.sessions {
                s.failed_attempts += 1;
                if s.failed_attempts >= self.config.code_attempts_per_session {
                    voided.push((*sid, s.creator));
                }
            }
            for (sid, creator) in voided {
                tracing::warn!(session_id = %sid, "session voided after too many failed code attempts");
                out.push(Delivery::Send(creator, RelayFrame::error(RelayErrorCode::SessionExpired)));
                self.remove_session(sid);
                if let Some(cc) = self.conns.get_mut(&creator) {
                    cc.session = None;
                }
            }
            if let Some(c) = self.conns.get_mut(&conn) {
                c.failed_code_attempts += 1;
                if c.failed_code_attempts >= self.config.code_attempts_per_connection {
                    tracing::warn!(?conn, "connection closed after too many failed code attempts");
                    out.push(Delivery::Close(conn));
                }
            }
        }
    }

    fn leave(&mut self, conn: ConnId, session_id: SessionId, out: &mut Vec<Delivery>) {
        let Some(s) = self.sessions.get(&session_id) else { return };
        if s.creator != conn && s.joiner != Some(conn) {
            return;
        }
        let other = if s.creator == conn { s.joiner } else { Some(s.creator) };
        if let Some(o) = other {
            out.push(Delivery::Send(o, RelayFrame::PeerLeft { version: ProtocolVersion::CURRENT, session_id }));
        }
        tracing::info!(%session_id, "pairing session left");
        self.remove_session(session_id);
    }

    fn forward(&mut self, conn: ConnId, session_id: SessionId, payload: Vec<u8>, out: &mut Vec<Delivery>) {
        if payload.len() > self.config.max_forward_bytes {
            out.push(Delivery::Send(conn, RelayFrame::error(RelayErrorCode::Malformed)));
            return;
        }
        let target = self.peer_of(conn, session_id);
        match target {
            Some(peer) => {
                self.forwarded += 1;
                out.push(Delivery::Send(peer, RelayFrame::Forward { version: ProtocolVersion::CURRENT, session_id, payload }));
            }
            None => out.push(Delivery::Send(conn, RelayFrame::error(RelayErrorCode::NotJoined))),
        }
    }

    fn peer_of(&self, conn: ConnId, session_id: SessionId) -> Option<ConnId> {
        if let Some(s) = self.sessions.get(&session_id) {
            if s.creator == conn {
                return s.joiner;
            }
            if s.joiner == Some(conn) {
                return Some(s.creator);
            }
            return None;
        }
        let name = self.channel_by_session.get(&session_id)?;
        let ch = self.channels.get(name)?;
        if !ch.members.contains(&conn) {
            return None;
        }
        ch.members.iter().copied().find(|m| *m != conn)
    }

    /// Every answer names the channel (`attached.channel`, `error.channel`), so a client with
    /// several attaches under way knows which one each answers; an invalid label is not echoed.
    fn attach(&mut self, conn: ConnId, channel: String, now: Instant, out: &mut Vec<Delivery>) {
        if validate_channel(&channel).is_err() {
            out.push(Delivery::Send(conn, RelayFrame::error(RelayErrorCode::InvalidChannel)));
            return;
        }
        let Some(c) = self.conns.get(&conn) else { return };
        if let Some(existing) = self.channels.get(&channel)
            && existing.members.contains(&conn)
        {
            out.push(Delivery::Send(conn, RelayFrame::error(RelayErrorCode::SessionAlreadyActive).for_channel(&channel)));
            return;
        }
        let ip = c.ip;
        if let Err(retry) = self.attach_limiter.check(ip, now) {
            out.push(Delivery::Send(conn, RelayFrame::rate_limited(secs(retry)).for_channel(&channel)));
            return;
        }
        if self.channels.get(&channel).is_some_and(|ch| ch.members.len() >= 2) {
            out.push(Delivery::Send(conn, RelayFrame::error(RelayErrorCode::ChannelFull).for_channel(&channel)));
            return;
        }
        let entry = self.channels.entry(channel.clone()).or_insert_with(|| Channel { session_id: SessionId::random(), members: Vec::new() });
        let session_id = entry.session_id;
        let peer_online = !entry.members.is_empty();
        for m in &entry.members {
            out.push(Delivery::Send(*m, RelayFrame::PeerPresence { version: ProtocolVersion::CURRENT, session_id, online: true }));
        }
        entry.members.push(conn);
        self.channel_by_session.insert(session_id, channel.clone());
        if let Some(c) = self.conns.get_mut(&conn) {
            c.channel_sessions.push(session_id);
        }
        out.push(Delivery::Send(conn, RelayFrame::Attached { version: ProtocolVersion::CURRENT, session_id, peer_online, channel: Some(channel) }));
    }

    fn remove_session(&mut self, sid: SessionId) {
        if let Some(s) = self.sessions.remove(&sid) {
            self.codes.remove(&s.code);
            for who in [Some(s.creator), s.joiner].into_iter().flatten() {
                if let Some(c) = self.conns.get_mut(&who)
                    && c.session == Some(sid)
                {
                    c.session = None;
                }
            }
        }
    }

    fn expire(&mut self, now: Instant, out: &mut Vec<Delivery>) {
        let expired: Vec<(SessionId, ConnId, Option<ConnId>)> =
            self.sessions.iter().filter(|(_, s)| now >= s.expires_at).map(|(id, s)| (*id, s.creator, s.joiner)).collect();
        for (sid, creator, joiner) in expired {
            tracing::info!(session_id = %sid, "pairing session expired");
            for who in [Some(creator), joiner].into_iter().flatten() {
                if self.conns.contains_key(&who) {
                    out.push(Delivery::Send(who, RelayFrame::error(RelayErrorCode::SessionExpired)));
                }
            }
            self.remove_session(sid);
        }
    }

    /// Unix expiry of a live session (tests / metrics).
    pub fn session_expires_at(&self, sid: SessionId) -> Option<u64> {
        self.sessions.get(&sid).map(|s| s.expires_at_unix)
    }
}

enum JoinKey {
    Code(PairCode),
    Session(SessionId),
}

fn secs(d: Duration) -> u32 {
    u32::try_from(d.as_secs()).unwrap_or(u32::MAX)
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const IP_A: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(10, 0, 0, 1));
    const IP_B: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(10, 0, 0, 2));
    const A: ConnId = ConnId(1);
    const B: ConnId = ConnId(2);
    const C: ConnId = ConnId(3);

    fn hello() -> RelayFrame {
        RelayFrame::Hello { version: ProtocolVersion::CURRENT, client_version: "test".into() }
    }

    fn relay() -> (RelayCore, Instant) {
        let mut r = RelayCore::new(RelayConfig::default());
        let t0 = Instant::now();
        r.on_connect(A, IP_A);
        r.on_connect(B, IP_B);
        assert!(matches!(r.on_frame(A, hello(), t0)[0], Delivery::Send(A, RelayFrame::HelloAck { .. })));
        assert!(matches!(r.on_frame(B, hello(), t0)[0], Delivery::Send(B, RelayFrame::HelloAck { .. })));
        (r, t0)
    }

    fn created(out: &[Delivery]) -> (SessionId, PairCode, u64) {
        match &out[0] {
            Delivery::Send(_, RelayFrame::SessionCreated { session_id, code, expires_at, .. }) => (*session_id, code.clone(), *expires_at),
            other => panic!("expected session_created, got {other:?}"),
        }
    }

    fn err_code(d: &Delivery) -> Option<RelayErrorCode> {
        match d {
            Delivery::Send(_, RelayFrame::Error { code, .. }) => Some(*code),
            _ => None,
        }
    }

    #[test]
    fn hello_is_required_first() {
        let mut r = RelayCore::new(RelayConfig::default());
        let t0 = Instant::now();
        r.on_connect(A, IP_A);
        let out = r.on_frame(A, RelayFrame::CreateSession { version: ProtocolVersion::CURRENT, ttl_secs: None }, t0);
        assert_eq!(err_code(&out[0]), Some(RelayErrorCode::HelloRequired));
        assert_eq!(out[1], Delivery::Close(A));
        assert!(format!("{r:?}").contains("RelayCore"));
        // Unknown connection ids are ignored.
        assert!(r.on_frame(ConnId(99), hello(), t0).is_empty());
        assert!(r.on_disconnect(ConnId(99), t0).is_empty());
    }

    #[test]
    fn text_frames_are_decoded_with_version_and_malformed_handling() {
        let (mut r, t0) = relay();
        let out = r.on_text(A, r#"{"type":"nope","version":9}"#, t0);
        assert_eq!(err_code(&out[0]), Some(RelayErrorCode::UnsupportedVersion));
        assert_eq!(out[1], Delivery::Close(A));
        let out = r.on_text(A, "garbage", t0);
        assert_eq!(err_code(&out[0]), Some(RelayErrorCode::Malformed));
        assert_eq!(out.len(), 1);
        let out = r.on_text(A, &RelayFrame::Bye { version: ProtocolVersion::CURRENT }.encode().unwrap(), t0);
        assert_eq!(out, vec![Delivery::Close(A)]);
    }

    #[test]
    fn create_join_by_code_forward_both_ways() {
        let (mut r, t0) = relay();
        let out = r.on_frame(A, RelayFrame::CreateSession { version: ProtocolVersion::CURRENT, ttl_secs: Some(90) }, t0);
        let (sid, code, exp) = created(&out);
        assert!(exp >= unix_now() + 89);
        assert_eq!(r.session_expires_at(sid), Some(exp));
        assert_eq!(r.stats().sessions, 1);
        // Forward before anyone joined is refused.
        let out = r.on_frame(A, RelayFrame::forward(sid, vec![1]), t0);
        assert_eq!(err_code(&out[0]), Some(RelayErrorCode::NotJoined));
        let out = r.on_frame(B, RelayFrame::JoinByCode { version: ProtocolVersion::CURRENT, code: code.clone() }, t0);
        assert_eq!(out[0], Delivery::Send(B, RelayFrame::Joined { version: ProtocolVersion::CURRENT, session_id: sid }));
        assert_eq!(out[1], Delivery::Send(A, RelayFrame::PeerJoined { version: ProtocolVersion::CURRENT, session_id: sid }));
        // Code is single-use.
        r.on_connect(C, IP_B);
        r.on_frame(C, hello(), t0);
        let out = r.on_frame(C, RelayFrame::JoinByCode { version: ProtocolVersion::CURRENT, code }, t0);
        assert_eq!(err_code(&out[0]), Some(RelayErrorCode::InvalidCode));
        // Session is full for a third party by session id.
        let out = r.on_frame(C, RelayFrame::JoinBySession { version: ProtocolVersion::CURRENT, session_id: sid }, t0);
        assert_eq!(err_code(&out[0]), Some(RelayErrorCode::SessionFull));
        // Forwarding both ways, payload untouched.
        let out = r.on_frame(A, RelayFrame::forward(sid, vec![9, 9, 9]), t0);
        assert_eq!(out, vec![Delivery::Send(B, RelayFrame::forward(sid, vec![9, 9, 9]))]);
        let out = r.on_frame(B, RelayFrame::forward(sid, vec![7]), t0);
        assert_eq!(out, vec![Delivery::Send(A, RelayFrame::forward(sid, vec![7]))]);
        assert_eq!(r.stats().forwarded, 2);
        // A stranger cannot forward into the session.
        let out = r.on_frame(C, RelayFrame::forward(sid, vec![1]), t0);
        assert_eq!(err_code(&out[0]), Some(RelayErrorCode::NotJoined));
        // Oversize payload.
        let out = r.on_frame(A, RelayFrame::forward(sid, vec![0; 80 * 1024]), t0);
        assert_eq!(err_code(&out[0]), Some(RelayErrorCode::Malformed));
        // Disconnect tells the peer and destroys the session.
        let out = r.on_disconnect(B, t0);
        assert_eq!(out, vec![Delivery::Send(A, RelayFrame::PeerLeft { version: ProtocolVersion::CURRENT, session_id: sid })]);
        assert_eq!(r.stats().sessions, 0);
        assert_eq!(r.stats().connections, 2);
    }

    #[test]
    fn join_by_session_from_ticket_and_creator_cannot_join_own_session() {
        let (mut r, t0) = relay();
        let (sid, _, _) = created(&r.on_frame(A, RelayFrame::CreateSession { version: ProtocolVersion::CURRENT, ttl_secs: None }, t0));
        let out = r.on_frame(A, RelayFrame::JoinBySession { version: ProtocolVersion::CURRENT, session_id: sid }, t0);
        assert_eq!(err_code(&out[0]), Some(RelayErrorCode::SessionAlreadyActive));
        let out = r.on_frame(B, RelayFrame::JoinBySession { version: ProtocolVersion::CURRENT, session_id: sid }, t0);
        assert!(matches!(out[0], Delivery::Send(B, RelayFrame::Joined { .. })));
        // Unknown session id -> session_expired, and it does NOT count as a code guess.
        r.on_connect(C, IP_B);
        r.on_frame(C, hello(), t0);
        let out = r.on_frame(C, RelayFrame::JoinBySession { version: ProtocolVersion::CURRENT, session_id: SessionId::random() }, t0);
        assert_eq!(err_code(&out[0]), Some(RelayErrorCode::SessionExpired));
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn regression_brute_force_closes_connection_and_voids_sessions() {
        let cfg = RelayConfig { code_attempts_per_connection: 3, code_attempts_per_session: 4, ..RelayConfig::default() };
        let mut r = RelayCore::new(cfg);
        let t0 = Instant::now();
        r.on_connect(A, IP_A);
        r.on_connect(B, IP_B);
        r.on_frame(A, hello(), t0);
        r.on_frame(B, hello(), t0);
        let (sid, code, _) = created(&r.on_frame(A, RelayFrame::CreateSession { version: ProtocolVersion::CURRENT, ttl_secs: None }, t0));
        let wrong = |n: u32| RelayFrame::JoinByCode { version: ProtocolVersion::CURRENT, code: PairCode::from_u32(n).unwrap() };
        // Pick guesses that are not the real code.
        let real: u32 = code.as_str().parse().unwrap();
        let guesses: Vec<u32> = (0..10).filter(|n| *n != real).collect();
        let out = r.on_frame(B, wrong(guesses[0]), t0);
        assert_eq!(err_code(&out[0]), Some(RelayErrorCode::InvalidCode));
        assert_eq!(out.len(), 1);
        r.on_frame(B, wrong(guesses[1]), t0);
        let out = r.on_frame(B, wrong(guesses[2]), t0);
        assert!(out.contains(&Delivery::Close(B)), "third miss closes the connection: {out:?}");
        // Session survived 3 misses (limit 4)…
        assert_eq!(r.stats().sessions, 1);
        // …a fourth miss from another connection voids it and tells the creator.
        r.on_connect(C, IP_B);
        r.on_frame(C, hello(), t0);
        let out = r.on_frame(C, wrong(guesses[3]), t0);
        assert!(out.iter().any(|d| matches!(d, Delivery::Send(A, RelayFrame::Error { code: RelayErrorCode::SessionExpired, .. }))), "{out:?}");
        assert_eq!(r.stats().sessions, 0);
        // Even the right code is dead now.
        let out = r.on_frame(C, RelayFrame::JoinByCode { version: ProtocolVersion::CURRENT, code }, t0);
        assert_eq!(err_code(&out[0]), Some(RelayErrorCode::InvalidCode));
        assert!(r.session_expires_at(sid).is_none());
        // Creator can create again (its slot was freed).
        let out = r.on_frame(A, RelayFrame::CreateSession { version: ProtocolVersion::CURRENT, ttl_secs: None }, t0);
        assert!(matches!(out[0], Delivery::Send(A, RelayFrame::SessionCreated { .. })));
    }

    #[test]
    fn regression_ip_rate_limits_for_join_create_and_attach() {
        let cfg = RelayConfig {
            join_rate: RateWindow::new(2, Duration::from_secs(60)),
            create_rate: RateWindow::new(1, Duration::from_secs(60)),
            attach_rate: RateWindow::new(1, Duration::from_secs(60)),
            code_attempts_per_connection: 100,
            ..RelayConfig::default()
        };
        let mut r = RelayCore::new(cfg);
        let t0 = Instant::now();
        r.on_connect(A, IP_A);
        r.on_connect(B, IP_B);
        r.on_frame(A, hello(), t0);
        r.on_frame(B, hello(), t0);
        // create: 1 per minute per IP.
        let (sid, _, _) = created(&r.on_frame(A, RelayFrame::CreateSession { version: ProtocolVersion::CURRENT, ttl_secs: None }, t0));
        // A already has a session -> already_active, not rate-limited.
        let out = r.on_frame(A, RelayFrame::CreateSession { version: ProtocolVersion::CURRENT, ttl_secs: None }, t0);
        assert_eq!(err_code(&out[0]), Some(RelayErrorCode::SessionAlreadyActive));
        r.on_connect(C, IP_A);
        r.on_frame(C, hello(), t0);
        let out = r.on_frame(C, RelayFrame::CreateSession { version: ProtocolVersion::CURRENT, ttl_secs: None }, t0);
        assert!(matches!(out[0], Delivery::Send(C, RelayFrame::Error { code: RelayErrorCode::RateLimited, retry_after_secs: Some(60), .. })), "{out:?}");
        // join: 2 per minute per IP.
        let wrong = RelayFrame::JoinByCode { version: ProtocolVersion::CURRENT, code: PairCode::new("000000").unwrap() };
        r.on_frame(B, wrong.clone(), t0);
        r.on_frame(B, wrong.clone(), t0);
        let out = r.on_frame(B, wrong, t0);
        assert_eq!(err_code(&out[0]), Some(RelayErrorCode::RateLimited));
        // Window rolls over.
        let out = r.on_frame(B, RelayFrame::JoinBySession { version: ProtocolVersion::CURRENT, session_id: sid }, t0 + Duration::from_secs(61));
        assert!(matches!(out[0], Delivery::Send(B, RelayFrame::Joined { .. })), "{out:?}");
        // attach: 1 per minute per IP (C is on IP_A, A already used create but attach is separate).
        let ch = "ab".repeat(32);
        let out = r.on_frame(C, RelayFrame::Attach { version: ProtocolVersion::CURRENT, channel: ch.clone() }, t0);
        assert!(matches!(out[0], Delivery::Send(C, RelayFrame::Attached { peer_online: false, .. })));
        r.on_connect(ConnId(4), IP_A);
        r.on_frame(ConnId(4), hello(), t0);
        let out = r.on_frame(ConnId(4), RelayFrame::Attach { version: ProtocolVersion::CURRENT, channel: ch }, t0);
        assert_eq!(err_code(&out[0]), Some(RelayErrorCode::RateLimited));
    }

    #[test]
    fn sessions_expire_on_tick_and_ttl_is_clamped() {
        let (mut r, t0) = relay();
        let out = r.on_frame(A, RelayFrame::CreateSession { version: ProtocolVersion::CURRENT, ttl_secs: Some(10_000) }, t0);
        let (sid, _, exp) = created(&out);
        assert!(exp <= unix_now() + u64::from(MAX_SESSION_TTL_SECS) + 1, "ttl clamped to max");
        assert!(r.tick(t0 + Duration::from_secs(299)).is_empty());
        let out = r.tick(t0 + Duration::from_secs(300));
        assert_eq!(out, vec![Delivery::Send(A, RelayFrame::error(RelayErrorCode::SessionExpired))]);
        assert!(r.session_expires_at(sid).is_none());
        // Tiny ttl is floored to 10 s.
        let out = r.on_frame(A, RelayFrame::CreateSession { version: ProtocolVersion::CURRENT, ttl_secs: Some(1) }, t0);
        let (_, _, exp) = created(&out);
        assert!(exp >= unix_now() + 9);
        // Expiry with a joiner present notifies both.
        let (sid, code, _) = created(&r.on_frame(B, RelayFrame::CreateSession { version: ProtocolVersion::CURRENT, ttl_secs: Some(20) }, t0));
        r.on_connect(C, IP_B);
        r.on_frame(C, hello(), t0);
        r.on_frame(C, RelayFrame::JoinByCode { version: ProtocolVersion::CURRENT, code }, t0);
        let out = r.tick(t0 + Duration::from_secs(20));
        assert!(out.contains(&Delivery::Send(B, RelayFrame::error(RelayErrorCode::SessionExpired))));
        assert!(out.contains(&Delivery::Send(C, RelayFrame::error(RelayErrorCode::SessionExpired))));
        assert!(r.session_expires_at(sid).is_none());
        // After expiry, forward is refused.
        let out = r.on_frame(B, RelayFrame::forward(sid, vec![1]), t0 + Duration::from_secs(21));
        assert_eq!(err_code(&out[0]), Some(RelayErrorCode::NotJoined));
    }

    #[test]
    fn channels_attach_presence_forward_and_detach() {
        let (mut r, t0) = relay();
        let ch = "0f".repeat(32);
        let out = r.on_frame(A, RelayFrame::Attach { version: ProtocolVersion::CURRENT, channel: ch.clone() }, t0);
        let Delivery::Send(A, RelayFrame::Attached { session_id, peer_online: false, .. }) = out[0].clone() else { panic!("{out:?}") };
        // Second attach to the same channel from the same connection is refused…
        let out = r.on_frame(A, RelayFrame::Attach { version: ProtocolVersion::CURRENT, channel: ch.clone() }, t0);
        assert_eq!(err_code(&out[0]), Some(RelayErrorCode::SessionAlreadyActive));
        // …but a connection may attach to several different channels (one per trusted peer).
        let out = r.on_frame(A, RelayFrame::Attach { version: ProtocolVersion::CURRENT, channel: "cd".repeat(32) }, t0);
        assert!(matches!(out[0], Delivery::Send(A, RelayFrame::Attached { peer_online: false, .. })));
        assert_eq!(r.stats().channels, 2);
        // Forward with nobody on the other end.
        let out = r.on_frame(A, RelayFrame::forward(session_id, vec![1]), t0);
        assert_eq!(err_code(&out[0]), Some(RelayErrorCode::NotJoined));
        // B attaches: A gets presence, B learns peer is online with the same session id.
        let out = r.on_frame(B, RelayFrame::Attach { version: ProtocolVersion::CURRENT, channel: ch.clone() }, t0);
        assert_eq!(out[0], Delivery::Send(A, RelayFrame::PeerPresence { version: ProtocolVersion::CURRENT, session_id, online: true }));
        assert_eq!(
            out[1],
            Delivery::Send(B, RelayFrame::Attached { version: ProtocolVersion::CURRENT, session_id, peer_online: true, channel: Some(ch.clone()) })
        );
        assert_eq!(r.stats().channels, 2);
        // Third party is refused.
        r.on_connect(C, IP_B);
        r.on_frame(C, hello(), t0);
        let out = r.on_frame(C, RelayFrame::Attach { version: ProtocolVersion::CURRENT, channel: ch.clone() }, t0);
        assert_eq!(err_code(&out[0]), Some(RelayErrorCode::ChannelFull));
        // Invalid label.
        let out = r.on_frame(C, RelayFrame::Attach { version: ProtocolVersion::CURRENT, channel: "xyz".into() }, t0);
        assert_eq!(err_code(&out[0]), Some(RelayErrorCode::InvalidChannel));
        // Forward both ways.
        let out = r.on_frame(A, RelayFrame::forward(session_id, vec![5]), t0);
        assert_eq!(out, vec![Delivery::Send(B, RelayFrame::forward(session_id, vec![5]))]);
        let out = r.on_frame(B, RelayFrame::forward(session_id, vec![6]), t0);
        assert_eq!(out, vec![Delivery::Send(A, RelayFrame::forward(session_id, vec![6]))]);
        let out = r.on_frame(C, RelayFrame::forward(session_id, vec![6]), t0);
        assert_eq!(err_code(&out[0]), Some(RelayErrorCode::NotJoined));
        // B leaves: A told offline; channel persists with A; then A leaves: channel gone.
        let out = r.on_disconnect(B, t0);
        assert_eq!(out, vec![Delivery::Send(A, RelayFrame::PeerPresence { version: ProtocolVersion::CURRENT, session_id, online: false })]);
        assert_eq!(r.stats().channels, 2);
        assert!(r.on_disconnect(A, t0).is_empty());
        assert_eq!(r.stats().channels, 0);
        // A fresh attach mints a new session id.
        r.on_connect(A, IP_A);
        r.on_frame(A, hello(), t0);
        let out = r.on_frame(A, RelayFrame::Attach { version: ProtocolVersion::CURRENT, channel: ch }, t0);
        let Delivery::Send(A, RelayFrame::Attached { session_id: fresh, .. }) = out[0].clone() else { panic!() };
        assert_ne!(fresh, session_id);
    }

    /// Every answer to an `attach` names its channel (an invalid label excepted), so a client
    /// with several attaches under way knows which one a refusal is for.
    #[test]
    fn attach_answers_name_their_channel() {
        let (mut r, t0) = relay();
        let ch = "0f".repeat(32);
        let out = r.on_frame(A, RelayFrame::Attach { version: ProtocolVersion::CURRENT, channel: ch.clone() }, t0);
        assert!(matches!(&out[0], Delivery::Send(A, RelayFrame::Attached { channel: Some(c), .. }) if *c == ch), "{out:?}");
        let out = r.on_frame(A, RelayFrame::Attach { version: ProtocolVersion::CURRENT, channel: ch.clone() }, t0);
        assert!(
            matches!(&out[0], Delivery::Send(A, RelayFrame::Error { code: RelayErrorCode::SessionAlreadyActive, channel: Some(c), .. }) if *c == ch),
            "{out:?}"
        );
        r.on_frame(B, RelayFrame::Attach { version: ProtocolVersion::CURRENT, channel: ch.clone() }, t0);
        r.on_connect(C, IP_B);
        r.on_frame(C, hello(), t0);
        let out = r.on_frame(C, RelayFrame::Attach { version: ProtocolVersion::CURRENT, channel: ch.clone() }, t0);
        assert!(matches!(&out[0], Delivery::Send(C, RelayFrame::Error { code: RelayErrorCode::ChannelFull, channel: Some(c), .. }) if *c == ch), "{out:?}");
        let out = r.on_frame(C, RelayFrame::Attach { version: ProtocolVersion::CURRENT, channel: "xyz".into() }, t0);
        assert_eq!(out, vec![Delivery::Send(C, RelayFrame::error(RelayErrorCode::InvalidChannel))]);
        // A refused attach leaves the channel as it was.
        assert_eq!(r.stats().channels, 1);
    }

    #[test]
    fn server_only_frames_from_clients_are_ignored_and_creator_disconnect_notifies_joiner() {
        let (mut r, t0) = relay();
        assert!(
            r.on_frame(A, RelayFrame::HelloAck { version: ProtocolVersion::CURRENT, relay_version: "x".into(), limits: RelayConfig::default().limits() }, t0)
                .is_empty()
        );
        assert!(r.on_frame(A, RelayFrame::error(RelayErrorCode::Malformed), t0).is_empty());
        assert!(r.on_frame(A, RelayFrame::Joined { version: ProtocolVersion::CURRENT, session_id: SessionId::random() }, t0).is_empty());
        let (sid, code, _) = created(&r.on_frame(A, RelayFrame::CreateSession { version: ProtocolVersion::CURRENT, ttl_secs: None }, t0));
        r.on_frame(B, RelayFrame::JoinByCode { version: ProtocolVersion::CURRENT, code }, t0);
        let out = r.on_disconnect(A, t0);
        assert_eq!(out, vec![Delivery::Send(B, RelayFrame::PeerLeft { version: ProtocolVersion::CURRENT, session_id: sid })]);
        // B is free to join something else now.
        let out = r.on_frame(B, RelayFrame::JoinBySession { version: ProtocolVersion::CURRENT, session_id: sid }, t0);
        assert_eq!(err_code(&out[0]), Some(RelayErrorCode::SessionExpired));
    }

    #[test]
    fn leave_frees_the_session_and_tells_the_peer() {
        let (mut r, t0) = relay();
        let (sid, code, _) = created(&r.on_frame(A, RelayFrame::CreateSession { version: ProtocolVersion::CURRENT, ttl_secs: None }, t0));
        // Leaving a session you are not part of is ignored.
        r.on_connect(C, IP_B);
        r.on_frame(C, hello(), t0);
        assert!(r.on_frame(C, RelayFrame::Leave { version: ProtocolVersion::CURRENT, session_id: sid }, t0).is_empty());
        assert_eq!(r.stats().sessions, 1);
        r.on_frame(B, RelayFrame::JoinByCode { version: ProtocolVersion::CURRENT, code }, t0);
        let out = r.on_frame(B, RelayFrame::Leave { version: ProtocolVersion::CURRENT, session_id: sid }, t0);
        assert_eq!(out, vec![Delivery::Send(A, RelayFrame::PeerLeft { version: ProtocolVersion::CURRENT, session_id: sid })]);
        assert_eq!(r.stats().sessions, 0);
        // Both slots are free again.
        assert!(matches!(
            r.on_frame(A, RelayFrame::CreateSession { version: ProtocolVersion::CURRENT, ttl_secs: None }, t0)[0],
            Delivery::Send(A, RelayFrame::SessionCreated { .. })
        ));
        assert!(matches!(
            r.on_frame(B, RelayFrame::CreateSession { version: ProtocolVersion::CURRENT, ttl_secs: None }, t0)[0],
            Delivery::Send(B, RelayFrame::SessionCreated { .. })
        ));
        // Unknown session id is a no-op.
        assert!(r.on_frame(A, RelayFrame::Leave { version: ProtocolVersion::CURRENT, session_id: SessionId::random() }, t0).is_empty());
    }

    #[test]
    fn minted_codes_are_unique_among_live_sessions() {
        let cfg = RelayConfig { create_rate: RateWindow::new(1000, Duration::from_secs(60)), ..RelayConfig::default() };
        let mut r = RelayCore::new(cfg);
        let t0 = Instant::now();
        let mut codes = std::collections::HashSet::new();
        for i in 0..50u64 {
            let c = ConnId(100 + i);
            r.on_connect(c, IP_A);
            r.on_frame(c, hello(), t0);
            let (_, code, _) = created(&r.on_frame(c, RelayFrame::CreateSession { version: ProtocolVersion::CURRENT, ttl_secs: None }, t0));
            assert!(codes.insert(code.as_str().to_owned()));
        }
        assert_eq!(r.stats().sessions, 50);
        assert_eq!(r.stats().connections, 50);
    }
}
