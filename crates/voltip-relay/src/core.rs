//! Sans-IO relay logic.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use voltip_protocol::relay::{
    DEFAULT_SESSION_TTL_SECS, FEATURE_CLIENT_TOO_OLD, FEATURE_TICKET, MAX_SESSION_TTL_SECS, RelayErrorCode, RelayFrame, RelayLimits, validate_channel,
};
use voltip_protocol::{PairCode, ProtocolVersion, SessionId};

use crate::limits::{RateLimiter, RateWindow};
use crate::policy::{ClientVersion, RelayPolicy};
use crate::stats::{Count, Gauges, IpPeaks, StatsBook, StatsReport};
use crate::ticket::{SCOPE_RELAY, TicketKey};

/// Client addresses whose hourly connection peak is tracked; the rest count as overflow.
pub const MAX_TRACKED_IPS: usize = 4096;

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
    /// Reverse proxies whose `X-Forwarded-For` names the client (`--trust-proxy`,
    /// [`crate::forwarded`]). Empty: the TCP peer is the client. Behind a proxy every peer is the
    /// proxy, and without it the per-IP limits above are one limit for all users together.
    pub trusted_proxies: Vec<ipnet::IpNet>,
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
    /// The key that checks device tickets (`--ticket-key-file`, docs/pairing.md 「设备票据」).
    /// Without one the relay neither asks for nor reads tickets.
    pub ticket_key: Option<TicketKey>,
    /// Frames queued for one connection before the server drops more (the server reads it).
    pub outbound_queue: usize,
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
            trusted_proxies: Vec::new(),
            max_forward_bytes: 70 * 1024,
            relay_version: format!("voltip-relay/{}", env!("CARGO_PKG_VERSION")),
            ping_interval: Duration::from_secs(10),
            idle_timeout: Duration::from_secs(25),
            ticket_key: None,
            outbound_queue: 64,
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
    /// The version its `hello` named, when it named one the relay can read.
    version: Option<ClientVersion>,
    /// Its `hello` listed `client_too_old`: below the minimum it is told so and disconnected.
    gate_aware: bool,
    /// An older client below the minimum: it may stay, but its pairing and `attach` requests are
    /// refused with `unsupported_version` (docs/pairing.md 「最低版本」).
    too_old: bool,
    /// Its `hello` listed `ticket`: it understands `ticket_required` (docs/pairing.md 「设备票据」).
    ticket_aware: bool,
    /// Unix seconds its ticket runs out, once it authenticated.
    auth_exp: Option<u64>,
    /// It already asked for something while unauthenticated (counted once).
    asked_unauthenticated: bool,
    /// Cut off: its sessions and channels are gone and every frame it sends is answered with a
    /// close, whether or not the close reaches it (a full queue).
    revoked: bool,
    /// It counts towards its address's live connections.
    counted_ip: bool,
    /// This minute's forwarding.
    forwarding: Minute,
    failed_code_attempts: u32,
    /// Pairing session this connection created or joined.
    session: Option<SessionId>,
    /// Channel bindings this connection is attached to (one per trusted peer).
    channel_sessions: Vec<SessionId>,
}

/// What one connection forwarded in one UTC minute.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Minute {
    /// Unix minute (seconds / 60).
    minute: u64,
    bytes: u64,
    frames: u64,
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
    policy: RelayPolicy,
    book: StatsBook,
    /// Live connections by client address.
    live_by_ip: HashMap<IpAddr, u64>,
    /// This hour's most connections at once by client address (at most [`MAX_TRACKED_IPS`]).
    hour_peaks: HashMap<IpAddr, u64>,
    /// Connections whose address could not be tracked this hour.
    ip_overflow: u64,
    /// The hour (Unix seconds at its start) `hour_peaks` belongs to.
    peaks_hour: u64,
    /// The minute the last tick saw.
    last_minute: u64,
    /// Unix seconds, for the statistics' hours (tests turn it by hand).
    wall: fn() -> u64,
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
            policy: RelayPolicy::default(),
            book: StatsBook::default(),
            live_by_ip: HashMap::new(),
            hour_peaks: HashMap::new(),
            ip_overflow: 0,
            peaks_hour: 0,
            last_minute: 0,
            wall: unix_now,
        }
    }

    /// The same relay with `clock` as the statistics' wall clock (Unix seconds; tests).
    pub fn with_wall_clock(mut self, clock: fn() -> u64) -> Self {
        self.wall = clock;
        self
    }

    /// The policy in force.
    pub fn policy(&self) -> &RelayPolicy {
        &self.policy
    }

    /// Put `policy` in force (docs/pairing.md 「最低版本」「设备票据」). A connection that knows
    /// `client_too_old` and is now below the minimum is told so and closed, which ends its
    /// sessions and channels; an older client keeps what it has until it goes and is refused
    /// anything new; a lowered minimum lifts the refusal.
    ///
    /// When the ticket gate comes into force, every connection without a valid ticket is cut off
    /// at once ([`RelayCore::revoke`]): told why where it can understand it, closed, and its
    /// sessions and channels ended here rather than when its socket goes.
    pub fn set_policy(&mut self, policy: RelayPolicy) -> Vec<Delivery> {
        let was_gated = self.gated();
        self.policy = policy;
        let gated = self.gated();
        let now = (self.wall)();
        let mut out = Vec::new();
        let mut ids: Vec<ConnId> = self.conns.iter().filter(|(_, c)| c.greeted && !c.revoked).map(|(id, _)| *id).collect();
        ids.sort();
        for id in ids {
            let Some(c) = self.conns.get(&id) else { continue };
            let too_old = self.policy.is_too_old(c.version) || (gated && !c.ticket_aware);
            let cut = gated && !was_gated && !authenticated(c, now);
            if too_old && c.gate_aware {
                let min = self.policy.min_version.map(|v| v.to_string()).unwrap_or_default();
                out.push(Delivery::Send(id, RelayFrame::client_too_old(min)));
                out.push(Delivery::Close(id));
                self.book.count(Count::TooOld, now);
                if cut {
                    self.revoke(id, &mut out);
                }
                continue;
            }
            if cut {
                let code = if c.ticket_aware { RelayErrorCode::TicketRequired } else { RelayErrorCode::UnsupportedVersion };
                out.push(Delivery::Send(id, RelayFrame::error(code)));
                out.push(Delivery::Close(id));
                self.book.count(Count::TicketRefused, now);
                self.revoke(id, &mut out);
                continue;
            }
            // Until its socket is gone, a connection being closed is refused like an older one.
            if let Some(c) = self.conns.get_mut(&id) {
                c.too_old = too_old;
            }
        }
        if gated && !was_gated {
            tracing::info!("the ticket gate is in force; connections without a ticket were closed");
        }
        // A stable order for the I/O layer and the tests; a connection's frames keep their order.
        out.sort_by_key(|d| match d {
            Delivery::Send(c, _) => (*c, 0),
            Delivery::Close(c) => (*c, 1),
        });
        out
    }

    /// The ticket gate is in force: the policy asks for it with a minimum version, and this relay
    /// holds the ticket key.
    fn gated(&self) -> bool {
        self.policy.require_ticket && self.policy.min_version.is_some() && self.config.ticket_key.is_some()
    }

    /// Cut `conn` off: end its pairing session and its channels now (its peers are told), and
    /// answer anything more it sends with a close. The caller queues the reason and the close.
    fn revoke(&mut self, conn: ConnId, out: &mut Vec<Delivery>) {
        let Some(c) = self.conns.get_mut(&conn) else { return };
        if c.revoked {
            return;
        }
        c.revoked = true;
        let session = c.session.take();
        let channel_sessions = std::mem::take(&mut c.channel_sessions);
        self.book.count(Count::Revoked, (self.wall)());
        self.release(conn, session, channel_sessions, out);
    }

    /// Free what `conn` held: its pairing session is destroyed (a half-finished pairing must not be
    /// resumable by a stranger) and its peers told; it leaves its channels.
    fn release(&mut self, conn: ConnId, session: Option<SessionId>, channel_sessions: Vec<SessionId>, out: &mut Vec<Delivery>) {
        if let Some(sid) = session
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
        for csid in channel_sessions {
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
    }

    /// The admin console's statistics (`GET /stats`).
    pub fn stats_report(&self) -> StatsReport {
        let gauges = Gauges {
            connections: self.conns.len() as u64,
            sessions: self.sessions.len() as u64,
            channels: self.channels.len() as u64,
            channels_paired: self.channels.values().filter(|ch| ch.members.len() >= 2).count() as u64,
        };
        let mut report = self.book.report(gauges, (self.wall)());
        if let Some(hour) = report.hours.iter_mut().find(|h| h.hour == self.peaks_hour) {
            self.peaks().apply(hour);
        }
        report
    }

    /// This hour's per-address peaks so far.
    fn peaks(&self) -> IpPeaks {
        IpPeaks::of(self.hour_peaks.values().copied(), self.ip_overflow)
    }

    /// Configuration in use.
    pub fn config(&self) -> &RelayConfig {
        &self.config
    }

    /// Snapshot counters.
    pub fn stats(&self) -> RelayStats {
        RelayStats { connections: self.conns.len(), sessions: self.sessions.len(), channels: self.channels.len(), forwarded: self.forwarded, dropped: 0 }
    }

    /// A transport accepted a connection. Over a connection limit (docs/pairing.md 「中继限额」) it
    /// is told `rate_limited` and closed.
    pub fn on_connect(&mut self, conn: ConnId, ip: IpAddr) -> Vec<Delivery> {
        let now = (self.wall)();
        let live_here = self.live_by_ip.get(&ip).copied().unwrap_or(0);
        let live = self.live_by_ip.values().sum::<u64>();
        let limits = self.policy.limits;
        let refused = limits.connections_per_ip.is_some_and(|l| live_here >= l) || limits.max_connections.is_some_and(|l| live >= l);
        self.conns.insert(
            conn,
            Connection {
                ip,
                greeted: false,
                version: None,
                gate_aware: false,
                too_old: false,
                ticket_aware: false,
                auth_exp: None,
                asked_unauthenticated: false,
                revoked: refused,
                counted_ip: !refused,
                forwarding: Minute::default(),
                failed_code_attempts: 0,
                session: None,
                channel_sessions: Vec::new(),
            },
        );
        if refused {
            self.book.count(Count::LimitedConnections, now);
            return vec![Delivery::Send(conn, RelayFrame::rate_limited(60)), Delivery::Close(conn)];
        }
        let here = self.live_by_ip.entry(ip).or_default();
        *here += 1;
        let here = *here;
        self.roll_hour(now);
        if let Some(peak) = self.hour_peaks.get_mut(&ip) {
            *peak = (*peak).max(here);
        } else if self.hour_peaks.len() < MAX_TRACKED_IPS {
            self.hour_peaks.insert(ip, here);
        } else {
            self.ip_overflow += 1;
        }
        self.book.connections(self.conns.len(), now);
        Vec::new()
    }

    /// A connection went away (any reason). Peers are told; sessions the connection created
    /// are destroyed (a half-finished pairing must not be resumable by a stranger).
    pub fn on_disconnect(&mut self, conn: ConnId, now: Instant) -> Vec<Delivery> {
        let mut out = Vec::new();
        let Some(c) = self.conns.remove(&conn) else { return out };
        if c.counted_ip
            && let Some(n) = self.live_by_ip.get_mut(&c.ip)
        {
            *n -= 1;
            if *n == 0 {
                self.live_by_ip.remove(&c.ip);
            }
        }
        self.book.forward_sample(c.forwarding.minute * 60, c.forwarding.bytes, c.forwarding.frames);
        self.release(conn, c.session, c.channel_sessions, &mut out);
        self.expire(now, &mut out);
        out
    }

    /// Housekeeping tick: expire sessions. Call every second or so.
    pub fn tick(&mut self, now: Instant) -> Vec<Delivery> {
        let mut out = Vec::new();
        self.expire(now, &mut out);
        let wall = (self.wall)();
        self.expire_tickets(wall, &mut out);
        let minute = wall / 60;
        if minute != self.last_minute {
            self.last_minute = minute;
            for c in self.conns.values_mut() {
                if c.forwarding.minute < minute && c.forwarding.frames > 0 {
                    self.book.forward_sample(c.forwarding.minute * 60, c.forwarding.bytes, c.forwarding.frames);
                    c.forwarding = Minute::default();
                }
            }
        }
        self.roll_hour(wall);
        out
    }

    /// A new hour: the closing hour keeps its per-address peaks, and the new one starts from the
    /// connections open now.
    fn roll_hour(&mut self, wall: u64) {
        let hour = wall - wall % 3600;
        if hour == self.peaks_hour {
            return;
        }
        if self.peaks_hour != 0 {
            self.book.ip_peaks(self.peaks_hour, &self.peaks());
        }
        self.peaks_hour = hour;
        self.hour_peaks = self.live_by_ip.iter().take(MAX_TRACKED_IPS).map(|(ip, n)| (*ip, *n)).collect();
        self.ip_overflow = 0;
        if !self.conns.is_empty() {
            self.book.connections(self.conns.len(), wall);
        }
    }

    /// Tickets that ran out: counted, and under the gate their connections are cut off.
    fn expire_tickets(&mut self, wall: u64, out: &mut Vec<Delivery>) {
        let gated = self.gated();
        let mut ran_out: Vec<ConnId> = Vec::new();
        for (id, c) in &mut self.conns {
            if !c.revoked && c.auth_exp.is_some_and(|exp| exp <= wall) {
                c.auth_exp = None;
                ran_out.push(*id);
            }
        }
        ran_out.sort();
        for id in ran_out {
            self.book.count(Count::AuthExpired, wall);
            if gated {
                out.push(Delivery::Send(id, RelayFrame::error(RelayErrorCode::TicketExpired)));
                out.push(Delivery::Close(id));
                self.revoke(id, out);
            }
        }
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
        if c.revoked {
            out.push(Delivery::Close(conn));
            return out;
        }
        if !c.greeted && !matches!(frame, RelayFrame::Hello { .. }) {
            out.push(Delivery::Send(conn, RelayFrame::error(RelayErrorCode::HelloRequired)));
            out.push(Delivery::Close(conn));
            return out;
        }
        match frame {
            RelayFrame::Hello { client_version, features, .. } => self.hello(conn, &client_version, &features, &mut out),
            RelayFrame::Auth { ticket, .. } => self.auth(conn, &ticket, &mut out),
            RelayFrame::CreateSession { ttl_secs, .. } => self.create_session(conn, ttl_secs, now, &mut out),
            RelayFrame::JoinByCode { code, .. } => self.join(conn, JoinKey::Code(code), now, &mut out),
            RelayFrame::JoinBySession { session_id, .. } => self.join(conn, JoinKey::Session(session_id), now, &mut out),
            RelayFrame::Leave { session_id, .. } => self.leave(conn, session_id, &mut out),
            RelayFrame::Forward { session_id, payload, .. } => self.forward(conn, session_id, payload, &mut out),
            RelayFrame::Attach { channel, .. } => self.attach(conn, channel, now, &mut out),
            RelayFrame::Bye { .. } => out.push(Delivery::Close(conn)),
            // Server-only frames from a client are protocol misuse: ignore, log.
            RelayFrame::HelloAck { .. }
            | RelayFrame::AuthAck { .. }
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

    /// `hello`: the minimum version decides how the client is served (docs/pairing.md 「最低版本」).
    fn hello(&mut self, conn: ConnId, client_version: &str, features: &[String], out: &mut Vec<Delivery>) {
        let version = ClientVersion::of_client(client_version);
        let gate_aware = features.iter().any(|f| f == FEATURE_CLIENT_TOO_OLD);
        let ticket_aware = features.iter().any(|f| f == FEATURE_TICKET);
        // Under the ticket gate a client that cannot authenticate is served like one too old.
        let too_old = self.policy.is_too_old(version) || (self.gated() && !ticket_aware);
        let wall = (self.wall)();
        self.book.hello(version, &self.policy, wall);
        if self.config.ticket_key.is_some() && !ticket_aware {
            self.book.count(Count::NoTicketFeature, wall);
        }
        if too_old
            && gate_aware
            && let Some(min) = self.policy.min_version
        {
            tracing::info!(?conn, %client_version, %min, "client below the minimum version; refusing it");
            self.book.count(Count::TooOld, wall);
            out.push(Delivery::Send(conn, RelayFrame::client_too_old(min.to_string())));
            out.push(Delivery::Close(conn));
            return;
        }
        if let Some(c) = self.conns.get_mut(&conn) {
            c.greeted = true;
            c.version = version;
            c.gate_aware = gate_aware;
            c.ticket_aware = ticket_aware;
            c.too_old = too_old;
        }
        let features = if self.config.ticket_key.is_some() { vec![FEATURE_TICKET.to_owned()] } else { Vec::new() };
        out.push(Delivery::Send(
            conn,
            RelayFrame::HelloAck {
                version: ProtocolVersion::CURRENT,
                relay_version: self.config.relay_version.clone(),
                limits: self.config.limits(),
                features,
            },
        ));
    }

    /// `auth`: a valid ticket authenticates the connection until it runs out, and its proven
    /// version stands in for the one `hello` named (docs/pairing.md 「设备票据」). An invalid one
    /// leaves the connection as it was.
    fn auth(&mut self, conn: ConnId, ticket: &str, out: &mut Vec<Delivery>) {
        let Some(key) = &self.config.ticket_key else {
            tracing::debug!(?conn, "auth on a relay without a ticket key; ignored");
            return;
        };
        let wall = (self.wall)();
        let verified = match key.verify(ticket, SCOPE_RELAY, wall) {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!(?conn, error = %e, "ticket refused");
                self.book.count(Count::AuthInvalid, wall);
                out.push(Delivery::Send(conn, RelayFrame::error(RelayErrorCode::TicketInvalid)));
                return;
            }
        };
        self.book.count(Count::AuthOk, wall);
        let Some(c) = self.conns.get_mut(&conn) else { return };
        c.auth_exp = Some(verified.exp);
        if verified.version.is_some() {
            c.version = verified.version;
        }
        let too_old = self.policy.is_too_old(c.version);
        c.too_old = too_old;
        if too_old
            && c.gate_aware
            && let Some(min) = self.policy.min_version
        {
            self.book.count(Count::TooOld, wall);
            out.push(Delivery::Send(conn, RelayFrame::client_too_old(min.to_string())));
            out.push(Delivery::Close(conn));
            return;
        }
        out.push(Delivery::Send(conn, RelayFrame::AuthAck { version: ProtocolVersion::CURRENT, exp: verified.exp }));
    }

    /// A connection without a valid ticket asked for a session or a channel: counted once, and
    /// under the gate refused with `ticket_required`.
    fn refuse_unauthenticated(&mut self, conn: ConnId, channel: Option<&str>, out: &mut Vec<Delivery>) -> bool {
        let wall = (self.wall)();
        let has_key = self.config.ticket_key.is_some();
        let gated = self.gated();
        let Some(c) = self.conns.get_mut(&conn) else { return false };
        if authenticated(c, wall) {
            return false;
        }
        if has_key && c.ticket_aware && !c.asked_unauthenticated {
            c.asked_unauthenticated = true;
            self.book.count(Count::TicketUnauthenticated, wall);
        }
        if !gated {
            return false;
        }
        self.book.count(Count::TicketRefused, wall);
        let refusal = RelayFrame::error(RelayErrorCode::TicketRequired);
        out.push(Delivery::Send(
            conn,
            match channel {
                Some(ch) => refusal.for_channel(ch),
                None => refusal,
            },
        ));
        true
    }

    /// An older client below the minimum asked for something new: `unsupported_version`, which
    /// its pairing screen shows as 「版本不兼容，请更新 Voltip」.
    fn refuse_old(&mut self, conn: ConnId, channel: Option<&str>, out: &mut Vec<Delivery>) -> bool {
        if !self.conns.get(&conn).is_some_and(|c| c.too_old) {
            return false;
        }
        self.book.count(Count::OldRefused, (self.wall)());
        let refusal = RelayFrame::error(RelayErrorCode::UnsupportedVersion);
        out.push(Delivery::Send(
            conn,
            match channel {
                Some(ch) => refusal.for_channel(ch),
                None => refusal,
            },
        ));
        true
    }

    fn rate_limited(&mut self, conn: ConnId, retry: Duration, channel: Option<&str>, out: &mut Vec<Delivery>) {
        self.book.count(Count::RateLimited, (self.wall)());
        let frame = RelayFrame::rate_limited(secs(retry));
        out.push(Delivery::Send(
            conn,
            match channel {
                Some(ch) => frame.for_channel(ch),
                None => frame,
            },
        ));
    }

    fn create_session(&mut self, conn: ConnId, ttl_secs: Option<u32>, now: Instant, out: &mut Vec<Delivery>) {
        if self.refuse_old(conn, None, out) || self.refuse_unauthenticated(conn, None, out) {
            return;
        }
        let Some(c) = self.conns.get(&conn) else { return };
        if c.session.is_some() {
            out.push(Delivery::Send(conn, RelayFrame::error(RelayErrorCode::SessionAlreadyActive)));
            return;
        }
        let ip = c.ip;
        if let Err(retry) = self.create_limiter.check(ip, now) {
            self.rate_limited(conn, retry, None, out);
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
        self.book.count(Count::SessionCreated, (self.wall)());
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
        if self.refuse_old(conn, None, out) || self.refuse_unauthenticated(conn, None, out) {
            return;
        }
        let Some(c) = self.conns.get(&conn) else { return };
        if c.session.is_some() {
            out.push(Delivery::Send(conn, RelayFrame::error(RelayErrorCode::SessionAlreadyActive)));
            return;
        }
        let ip = c.ip;
        if let Err(retry) = self.join_limiter.check(ip, now) {
            self.rate_limited(conn, retry, None, out);
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
        self.book.count(Count::SessionJoined, (self.wall)());
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
        let wall = (self.wall)();
        let gated = self.gated();
        let limits = self.policy.limits;
        let Some(c) = self.conns.get_mut(&conn) else { return };
        // Under the gate only an authenticated connection forwards, whatever it holds already.
        if gated && !authenticated(c, wall) {
            let code = if c.ticket_aware { RelayErrorCode::TicketRequired } else { RelayErrorCode::UnsupportedVersion };
            self.book.count(Count::TicketRefused, wall);
            out.push(Delivery::Send(conn, RelayFrame::error(code)));
            out.push(Delivery::Close(conn));
            self.revoke(conn, out);
            return;
        }
        let minute = wall / 60;
        if c.forwarding.minute != minute {
            let done = std::mem::replace(&mut c.forwarding, Minute { minute, bytes: 0, frames: 0 });
            self.book.forward_sample(done.minute * 60, done.bytes, done.frames);
        }
        let Some(c) = self.conns.get_mut(&conn) else { return };
        c.forwarding.bytes += payload.len() as u64;
        c.forwarding.frames += 1;
        let over = limits.forward_bytes_per_minute.is_some_and(|l| c.forwarding.bytes > l)
            || limits.forward_frames_per_minute.is_some_and(|l| c.forwarding.frames > l);
        if over {
            tracing::info!(?conn, "forwarding over the per-minute limit; closing the connection");
            self.book.count(Count::LimitedForward, wall);
            out.push(Delivery::Send(conn, RelayFrame::rate_limited(u32::try_from(60 - wall % 60).unwrap_or(60))));
            out.push(Delivery::Close(conn));
            self.revoke(conn, out);
            return;
        }
        let target = self.peer_of(conn, session_id);
        match target {
            Some(peer) => {
                self.forwarded += 1;
                self.book.count(Count::Forward, (self.wall)());
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
        if self.refuse_old(conn, Some(&channel), out) || self.refuse_unauthenticated(conn, Some(&channel), out) {
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
            self.rate_limited(conn, retry, Some(&channel), out);
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
        self.book.count(Count::Attach, (self.wall)());
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
            self.book.count(Count::SessionExpired, (self.wall)());
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

/// It holds a ticket that has not run out.
fn authenticated(c: &Connection, now: u64) -> bool {
    c.auth_exp.is_some_and(|exp| exp > now)
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
        RelayFrame::Hello { version: ProtocolVersion::CURRENT, client_version: "test".into(), features: Vec::new() }
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
            r.on_frame(
                A,
                RelayFrame::HelloAck {
                    version: ProtocolVersion::CURRENT,
                    relay_version: "x".into(),
                    limits: RelayConfig::default().limits(),
                    features: Vec::new()
                },
                t0
            )
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

    fn hello_as(client_version: &str, aware: bool) -> RelayFrame {
        if aware {
            RelayFrame::hello(client_version)
        } else {
            RelayFrame::Hello { version: ProtocolVersion::CURRENT, client_version: client_version.into(), features: Vec::new() }
        }
    }

    fn min(version: &str) -> RelayPolicy {
        RelayPolicy { min_version: Some(version.parse().unwrap()), ..RelayPolicy::default() }
    }

    fn create() -> RelayFrame {
        RelayFrame::CreateSession { version: ProtocolVersion::CURRENT, ttl_secs: None }
    }

    fn attach(channel: &str) -> RelayFrame {
        RelayFrame::Attach { version: ProtocolVersion::CURRENT, channel: channel.into() }
    }

    /// docs/pairing.md 「最低版本」: a client that knows `client_too_old` and is below the minimum
    /// is told the version it needs and closed; the minimum itself and anything above it, and a
    /// version the relay cannot read, are served.
    #[test]
    fn a_client_below_the_minimum_that_knows_the_error_is_told_and_closed() {
        let mut r = RelayCore::new(RelayConfig::default());
        assert!(r.set_policy(min("0.1.10")).is_empty(), "nobody connected yet");
        let t0 = Instant::now();
        for (i, (client, served)) in [("voltip/0.1.9", false), ("voltip/0.1.10", true), ("voltip/0.2.0", true), ("test", true)].into_iter().enumerate() {
            let conn = ConnId(10 + i as u64);
            r.on_connect(conn, IP_A);
            let out = r.on_frame(conn, hello_as(client, true), t0);
            if served {
                assert!(matches!(out[..], [Delivery::Send(c, RelayFrame::HelloAck { .. })] if c == conn), "{client}: {out:?}");
            } else {
                assert_eq!(out, vec![Delivery::Send(conn, RelayFrame::client_too_old("0.1.10")), Delivery::Close(conn)], "{client}");
                // Closed before it was greeted: nothing more is served.
                assert_eq!(err_code(&r.on_frame(conn, create(), t0)[0]), Some(RelayErrorCode::HelloRequired));
            }
        }
        assert_eq!(r.policy(), &min("0.1.10"));
    }

    /// Regression guard (user decision 2026-10-27, 「拦配对和重连」): a client before 0.2.0 cannot
    /// read `client_too_old`. Below the minimum it stays connected, and every pairing request and
    /// `attach` is refused with the `unsupported_version` its pairing screen already explains.
    #[test]
    fn regression_an_older_client_below_the_minimum_may_stay_but_not_pair_or_reconnect() {
        let mut r = RelayCore::new(RelayConfig::default());
        r.set_policy(min("0.1.10"));
        let t0 = Instant::now();
        r.on_connect(A, IP_A);
        r.on_connect(B, IP_B);
        assert!(matches!(r.on_frame(A, hello_as("voltip/0.1.9", false), t0)[..], [Delivery::Send(A, RelayFrame::HelloAck { .. })]));
        r.on_frame(B, hello_as("voltip/0.1.10", true), t0);
        assert_eq!(r.on_frame(A, create(), t0), vec![Delivery::Send(A, RelayFrame::error(RelayErrorCode::UnsupportedVersion))]);
        let (_, code, _) = created(&r.on_frame(B, create(), t0));
        let join = RelayFrame::JoinByCode { version: ProtocolVersion::CURRENT, code };
        assert_eq!(r.on_frame(A, join, t0), vec![Delivery::Send(A, RelayFrame::error(RelayErrorCode::UnsupportedVersion))]);
        let ch = "ab".repeat(32);
        assert_eq!(
            r.on_frame(A, attach(&ch), t0),
            vec![Delivery::Send(A, RelayFrame::error(RelayErrorCode::UnsupportedVersion).for_channel(&ch))],
            "names its channel, which 0.1.9 does not ask again for"
        );
        assert_eq!(r.stats().sessions, 1, "the refusals changed nothing");
        assert_eq!(r.stats_report().hours[0].old_refused, 3);
    }

    /// A minimum raised while devices are connected: the ones that know `client_too_old` are told
    /// and closed, which ends their channels; an older client keeps the channel it is on until it
    /// goes and gets nothing new; lowering the minimum lifts the refusal.
    #[test]
    fn raising_the_minimum_closes_aware_clients_and_keeps_what_older_ones_have() {
        let mut r = RelayCore::new(RelayConfig::default());
        let t0 = Instant::now();
        let (aware, old, peer) = (A, B, C);
        for (conn, client, is_aware) in [(aware, "voltip/0.1.10", true), (old, "voltip/0.1.9", false), (peer, "voltip/0.3.0", true)] {
            r.on_connect(conn, IP_A);
            r.on_frame(conn, hello_as(client, is_aware), t0);
        }
        let (kept, closed) = ("cd".repeat(32), "ef".repeat(32));
        r.on_frame(old, attach(&kept), t0);
        let Delivery::Send(_, RelayFrame::Attached { session_id: kept_sid, .. }) = r.on_frame(peer, attach(&kept), t0)[1].clone() else { panic!() };
        r.on_frame(aware, attach(&closed), t0);
        r.on_frame(peer, attach(&closed), t0);

        let out = r.set_policy(min("0.2.0"));
        assert_eq!(out, vec![Delivery::Send(aware, RelayFrame::client_too_old("0.2.0")), Delivery::Close(aware)]);
        // The I/O layer closes the socket; the core then forgets the connection.
        let out = r.on_disconnect(aware, t0);
        assert!(matches!(&out[..], [Delivery::Send(p, RelayFrame::PeerPresence { online: false, .. })] if *p == peer), "{out:?}");
        // The older client's channel still carries frames…
        let out = r.on_frame(old, RelayFrame::forward(kept_sid, vec![1]), t0);
        assert_eq!(out, vec![Delivery::Send(peer, RelayFrame::forward(kept_sid, vec![1]))]);
        // …and anything new is refused.
        assert_eq!(err_code(&r.on_frame(old, attach(&"aa".repeat(32)), t0)[0]), Some(RelayErrorCode::UnsupportedVersion));
        assert_eq!(err_code(&r.on_frame(old, create(), t0)[0]), Some(RelayErrorCode::UnsupportedVersion));
        // The minimum lowered again: nobody is closed, the older client is served again.
        assert!(r.set_policy(RelayPolicy::default()).is_empty());
        assert!(matches!(r.on_frame(old, create(), t0)[0], Delivery::Send(B, RelayFrame::SessionCreated { .. })));
        assert_eq!(r.stats_report().hours[0].too_old, 1);
    }

    /// docs/pairing.md 「中继统计」: counts only, nothing that names a device.
    #[test]
    fn the_statistics_count_what_happened_and_name_no_device() {
        fn clock() -> u64 {
            1_761_552_000 + 120
        }
        let cfg = RelayConfig { create_rate: RateWindow::new(1, Duration::from_secs(60)), ..RelayConfig::default() };
        let mut r = RelayCore::new(cfg).with_wall_clock(clock);
        r.set_policy(RelayPolicy { known_versions: vec!["0.1.10".parse().unwrap()], ..RelayPolicy::default() });
        let t0 = Instant::now();
        r.on_connect(A, IP_A);
        r.on_connect(B, IP_B);
        r.on_frame(A, hello_as("voltip/0.1.10", true), t0);
        r.on_frame(B, hello_as("voltip/0.1.8", false), t0);
        let (sid, code, _) = created(&r.on_frame(A, create(), t0));
        r.on_frame(B, RelayFrame::JoinByCode { version: ProtocolVersion::CURRENT, code }, t0);
        r.on_frame(A, RelayFrame::forward(sid, vec![1]), t0);
        r.on_connect(C, IP_A);
        r.on_frame(C, hello(), t0);
        assert_eq!(err_code(&r.on_frame(C, create(), t0)[0]), Some(RelayErrorCode::RateLimited));
        let ch = "ab".repeat(32);
        r.on_frame(A, attach(&ch), t0);
        r.on_frame(B, attach(&ch), t0);
        let report = r.stats_report();
        assert_eq!((report.gauges.connections, report.gauges.sessions, report.gauges.channels, report.gauges.channels_paired), (3, 1, 1, 1));
        let hour = &report.hours[0];
        assert_eq!((hour.hour, hour.peak_connections, hour.hellos), (1_761_552_000, 3, 3));
        assert_eq!(hour.versions, std::collections::BTreeMap::from([("0.1.10".to_owned(), 1), ("other".to_owned(), 2)]));
        assert_eq!((hour.sessions_created, hour.sessions_joined, hour.forwarded, hour.rate_limited, hour.attaches), (1, 1, 1, 1, 2));
        let json = serde_json::to_string(&report).unwrap();
        for secret in [IP_A.to_string(), IP_B.to_string(), ch, sid.to_string()] {
            assert!(!json.contains(&secret), "{secret} in {json}");
        }
        // Expired sessions are counted too: C's of 10 s and A's of the default 120 s.
        r.on_frame(C, RelayFrame::CreateSession { version: ProtocolVersion::CURRENT, ttl_secs: Some(10) }, t0 + Duration::from_secs(61));
        r.tick(t0 + Duration::from_secs(120));
        assert_eq!(r.stats_report().hours[0].sessions_expired, 2);
    }

    // ---- device tickets (docs/pairing.md 「设备票据」) and limits (「中继限额」) ----

    const T0: u64 = 1_761_552_000; // an hour's start
    thread_local! {
        static NOW: std::cell::Cell<u64> = const { std::cell::Cell::new(T0) };
    }
    fn now_wall() -> u64 {
        NOW.with(std::cell::Cell::get)
    }
    fn set_wall(t: u64) {
        NOW.with(|n| n.set(t));
    }

    fn key() -> TicketKey {
        TicketKey::new(&[9; 32]).unwrap()
    }

    fn ticket(ver: &str, exp: u64) -> String {
        key().issue(&crate::ticket::TicketPayload { v: 1, d: "dev-1".into(), ver: ver.into(), exp, scope: vec!["relay".into(), "builtin".into()] })
    }

    fn ticketed() -> RelayCore {
        set_wall(T0);
        RelayCore::new(RelayConfig { ticket_key: Some(key()), ..RelayConfig::default() }).with_wall_clock(now_wall)
    }

    fn gate(min_version: &str) -> RelayPolicy {
        RelayPolicy { min_version: Some(min_version.parse().unwrap()), require_ticket: true, ..RelayPolicy::default() }
    }

    fn greet(r: &mut RelayCore, conn: ConnId, ip: IpAddr, client: &str, features: &[&str], t0: Instant) -> Vec<Delivery> {
        assert!(r.on_connect(conn, ip).is_empty());
        let hello = RelayFrame::Hello {
            version: ProtocolVersion::CURRENT,
            client_version: client.into(),
            features: features.iter().map(|f| (*f).to_owned()).collect(),
        };
        r.on_frame(conn, hello, t0)
    }

    const NEW: &[&str] = &["client_too_old", "ticket"];

    fn authenticate(r: &mut RelayCore, conn: ConnId, t0: Instant) {
        let out = r.on_frame(conn, RelayFrame::auth(ticket("0.3.1", T0 + 3600)), t0);
        assert_eq!(out, vec![Delivery::Send(conn, RelayFrame::AuthAck { version: ProtocolVersion::CURRENT, exp: T0 + 3600 })]);
    }

    fn attached_sid(out: &[Delivery]) -> SessionId {
        out.iter()
            .find_map(|d| match d {
                Delivery::Send(_, RelayFrame::Attached { session_id, .. }) => Some(*session_id),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no attached in {out:?}"))
    }

    #[test]
    fn a_relay_with_the_key_asks_for_tickets_and_one_without_does_not() {
        let t0 = Instant::now();
        let mut plain = RelayCore::new(RelayConfig::default());
        let out = greet(&mut plain, A, IP_A, "voltip/0.3.1", NEW, t0);
        assert!(matches!(&out[..], [Delivery::Send(A, RelayFrame::HelloAck { features, .. })] if features.is_empty()));
        assert!(plain.on_frame(A, RelayFrame::auth("x.y"), t0).is_empty(), "a relay without the key ignores auth");

        let mut r = ticketed();
        let out = greet(&mut r, A, IP_A, "voltip/0.3.1", NEW, t0);
        assert!(matches!(&out[..], [Delivery::Send(A, RelayFrame::HelloAck { features, .. })] if features == &["ticket".to_owned()]));
        authenticate(&mut r, A, t0);
        // A bad ticket is refused and the connection stays as it was.
        assert_eq!(r.on_frame(A, RelayFrame::auth("bad.ticket"), t0), vec![Delivery::Send(A, RelayFrame::error(RelayErrorCode::TicketInvalid))]);
        let other = TicketKey::new(&[1; 32]).unwrap().issue(&crate::ticket::TicketPayload {
            v: 1,
            d: "x".into(),
            ver: "9.9.9".into(),
            exp: T0 + 60,
            scope: vec!["relay".into()],
        });
        assert_eq!(err_code(&r.on_frame(A, RelayFrame::auth(other), t0)[0]), Some(RelayErrorCode::TicketInvalid), "another key's ticket");
        let builtin_only = key().issue(&crate::ticket::TicketPayload { v: 1, d: "x".into(), ver: "0.3.1".into(), exp: T0 + 60, scope: vec!["builtin".into()] });
        assert_eq!(err_code(&r.on_frame(A, RelayFrame::auth(builtin_only), t0)[0]), Some(RelayErrorCode::TicketInvalid), "another scope");
        assert!(matches!(r.on_frame(A, create(), t0)[0], Delivery::Send(A, RelayFrame::SessionCreated { .. })), "still authenticated");
        let hour = &r.stats_report().hours[0];
        assert_eq!((hour.auth.ok, hour.auth.invalid), (1, 3));
    }

    /// The ticket's version is the one the device proved; the `hello` may claim any.
    #[test]
    fn the_minimum_version_reads_the_proven_version() {
        let t0 = Instant::now();
        let mut r = ticketed();
        r.set_policy(min("0.3.1"));
        greet(&mut r, A, IP_A, "voltip/9.9.9", NEW, t0);
        let out = r.on_frame(A, RelayFrame::auth(ticket("0.3.0", T0 + 3600)), t0);
        assert_eq!(out, vec![Delivery::Send(A, RelayFrame::client_too_old("0.3.1")), Delivery::Close(A)]);
    }

    /// Counting first (user decision 2026-11-03): without the gate nothing is refused, and the
    /// gate takes effect only with a minimum version and the key.
    #[test]
    fn without_the_gate_unauthenticated_clients_are_counted_and_served() {
        let t0 = Instant::now();
        let mut r = ticketed();
        r.set_policy(RelayPolicy { require_ticket: true, ..RelayPolicy::default() });
        greet(&mut r, A, IP_A, "voltip/0.3.1", NEW, t0);
        greet(&mut r, B, IP_B, "voltip/0.2.0", &["client_too_old"], t0);
        assert!(matches!(r.on_frame(A, create(), t0)[0], Delivery::Send(A, RelayFrame::SessionCreated { .. })), "no minimum: no gate");
        r.on_frame(A, attach(&"ab".repeat(32)), t0);
        assert!(matches!(r.on_frame(B, attach(&"cd".repeat(32)), t0)[0], Delivery::Send(B, RelayFrame::Attached { .. })));
        let hour = &r.stats_report().hours[0];
        assert_eq!((hour.auth.no_feature, hour.auth.unauthenticated, hour.auth.refused), (1, 1, 0), "the unauthenticated request is counted once");

        let mut keyless = RelayCore::new(RelayConfig::default());
        keyless.set_policy(gate("0.3.1"));
        greet(&mut keyless, A, IP_A, "voltip/0.3.1", NEW, t0);
        assert!(matches!(keyless.on_frame(A, create(), t0)[0], Delivery::Send(A, RelayFrame::SessionCreated { .. })), "a relay without the key never gates");
    }

    #[test]
    fn under_the_gate_only_authenticated_clients_are_served() {
        let t0 = Instant::now();
        let mut r = ticketed();
        r.set_policy(gate("0.2.0"));
        // 0.2.x knows client_too_old but cannot authenticate: told and closed.
        assert_eq!(
            greet(&mut r, A, IP_A, "voltip/0.2.5", &["client_too_old"], t0),
            vec![Delivery::Send(A, RelayFrame::client_too_old("0.2.0")), Delivery::Close(A)]
        );
        // Before 0.2.0: it may stay, and gets unsupported_version for anything new.
        greet(&mut r, B, IP_B, "voltip/0.1.9", &[], t0);
        assert_eq!(err_code(&r.on_frame(B, create(), t0)[0]), Some(RelayErrorCode::UnsupportedVersion));
        // A client with tickets, not yet authenticated: ticket_required, and it may authenticate.
        greet(&mut r, C, IP_A, "voltip/0.3.1", NEW, t0);
        assert_eq!(r.on_frame(C, create(), t0), vec![Delivery::Send(C, RelayFrame::error(RelayErrorCode::TicketRequired))]);
        let ch = "ab".repeat(32);
        assert_eq!(r.on_frame(C, attach(&ch), t0), vec![Delivery::Send(C, RelayFrame::error(RelayErrorCode::TicketRequired).for_channel(&ch))]);
        authenticate(&mut r, C, t0);
        assert!(matches!(r.on_frame(C, attach(&ch), t0)[0], Delivery::Send(C, RelayFrame::Attached { .. })));
        assert_eq!(r.stats_report().hours[0].auth.refused, 2);
    }

    /// A ticket that runs out: counted while counting, and under the gate the connection is cut
    /// off, so a revoked device is gone within a ticket's lifetime.
    #[test]
    fn an_expired_ticket_closes_the_connection_under_the_gate() {
        let t0 = Instant::now();
        let mut r = ticketed();
        greet(&mut r, A, IP_A, "voltip/0.3.1", NEW, t0);
        authenticate(&mut r, A, t0);
        set_wall(T0 + 3600);
        assert!(r.tick(t0).is_empty(), "counting: nothing is closed");
        assert_eq!(r.stats_report().hours.last().unwrap().auth.expired, 1);

        let mut r = ticketed();
        r.set_policy(gate("0.3.0"));
        for (conn, ip) in [(A, IP_A), (B, IP_B)] {
            greet(&mut r, conn, ip, "voltip/0.3.1", NEW, t0);
            authenticate(&mut r, conn, t0);
        }
        let ch = "ab".repeat(32);
        r.on_frame(A, attach(&ch), t0);
        r.on_frame(B, attach(&ch), t0);
        // B renews; A does not.
        set_wall(T0 + 1800);
        assert!(matches!(r.on_frame(B, RelayFrame::auth(ticket("0.3.1", T0 + 7200)), t0)[..], [Delivery::Send(B, RelayFrame::AuthAck { .. })]));
        set_wall(T0 + 3600);
        let out = r.tick(t0);
        assert_eq!(out.len(), 3, "{out:?}");
        assert_eq!(out[..2], [Delivery::Send(A, RelayFrame::error(RelayErrorCode::TicketExpired)), Delivery::Close(A)]);
        assert!(matches!(&out[2], Delivery::Send(B, RelayFrame::PeerPresence { online: false, .. })), "{out:?}");
        assert_eq!(r.on_frame(A, create(), t0), vec![Delivery::Close(A)], "a revoked connection gets only a close");
    }

    /// Regression (plan review 2026-11-03, rounds 2–4): connections that stayed on unauthenticated
    /// through the counting period could keep forwarding after the gate came into force. They are
    /// cut off the moment it does, here in the core, whether or not their close is delivered.
    #[test]
    fn regression_turning_the_gate_on_stops_unauthenticated_connections_forwarding() {
        let t0 = Instant::now();
        let mut r = ticketed();
        greet(&mut r, A, IP_A, "voltip/0.3.1", NEW, t0);
        authenticate(&mut r, A, t0);
        greet(&mut r, B, IP_B, "voltip/0.3.1", NEW, t0);
        greet(&mut r, C, IP_A, "voltip/0.1.9", &[], t0);
        let ch = "ab".repeat(32);
        r.on_frame(A, attach(&ch), t0);
        let sid = attached_sid(&r.on_frame(B, attach(&ch), t0));
        // A pairing session between the authenticated A and the older C.
        let (psid, code, _) = created(&r.on_frame(A, create(), t0));
        r.on_frame(C, RelayFrame::JoinByCode { version: ProtocolVersion::CURRENT, code }, t0);
        assert_eq!(r.on_frame(B, RelayFrame::forward(sid, vec![1]), t0), vec![Delivery::Send(A, RelayFrame::forward(sid, vec![1]))], "counting: B forwards");

        let out = r.set_policy(gate("0.3.0"));
        assert!(out.contains(&Delivery::Send(B, RelayFrame::error(RelayErrorCode::TicketRequired))));
        assert!(out.contains(&Delivery::Send(C, RelayFrame::error(RelayErrorCode::UnsupportedVersion))));
        assert!(out.contains(&Delivery::Close(B)) && out.contains(&Delivery::Close(C)));
        assert!(out.contains(&Delivery::Send(A, RelayFrame::PeerPresence { version: ProtocolVersion::CURRENT, session_id: sid, online: false })));
        assert!(out.contains(&Delivery::Send(A, RelayFrame::PeerLeft { version: ProtocolVersion::CURRENT, session_id: psid })));
        assert!(!out.iter().any(|d| matches!(d, Delivery::Send(A, RelayFrame::Error { .. }) | Delivery::Close(A))), "the authenticated device stays: {out:?}");
        // Neither direction carries anything any more, closed socket or not.
        assert_eq!(r.on_frame(A, RelayFrame::forward(sid, vec![2]), t0), vec![Delivery::Send(A, RelayFrame::error(RelayErrorCode::NotJoined))]);
        assert_eq!(r.on_frame(A, RelayFrame::forward(psid, vec![2]), t0), vec![Delivery::Send(A, RelayFrame::error(RelayErrorCode::NotJoined))]);
        assert_eq!(r.on_frame(B, RelayFrame::forward(sid, vec![3]), t0), vec![Delivery::Close(B)]);
        assert_eq!(r.on_frame(C, RelayFrame::forward(psid, vec![3]), t0), vec![Delivery::Close(C)]);
        // When the sockets do go, nothing is released twice.
        assert!(r.on_disconnect(B, t0).is_empty());
        assert!(r.on_disconnect(C, t0).is_empty());
        assert_eq!(r.stats().channels, 1, "A's side of the channel is left");
        assert_eq!(r.stats_report().hours[0].auth.revoked, 2);
        // Turning the gate on again changes nothing for those already cut off or authenticated.
        assert!(r.set_policy(gate("0.3.0")).is_empty());
    }

    /// Under the gate a connection that did not authenticate cannot forward even on what it held.
    #[test]
    fn under_the_gate_an_unauthenticated_forward_is_refused_and_closed() {
        let t0 = Instant::now();
        let mut r = ticketed();
        r.set_policy(gate("0.3.0"));
        greet(&mut r, A, IP_A, "voltip/0.3.1", NEW, t0);
        let sid = SessionId::random();
        let out = r.on_frame(A, RelayFrame::forward(sid, vec![1]), t0);
        assert_eq!(out, vec![Delivery::Send(A, RelayFrame::error(RelayErrorCode::TicketRequired)), Delivery::Close(A)]);
    }

    #[test]
    fn forwarding_over_a_minute_limit_closes_the_connection() {
        let t0 = Instant::now();
        let mut r = ticketed();
        let limits = crate::policy::UsageLimits { forward_frames_per_minute: Some(2), forward_bytes_per_minute: Some(100), ..Default::default() };
        r.set_policy(RelayPolicy { limits, ..RelayPolicy::default() });
        for (conn, ip) in [(A, IP_A), (B, IP_B)] {
            greet(&mut r, conn, ip, "voltip/0.3.1", NEW, t0);
        }
        let ch = "ab".repeat(32);
        r.on_frame(A, attach(&ch), t0);
        let sid = attached_sid(&r.on_frame(B, attach(&ch), t0));
        set_wall(T0 + 15);
        for _ in 0..2 {
            assert!(matches!(r.on_frame(A, RelayFrame::forward(sid, vec![0; 10]), t0)[..], [Delivery::Send(B, RelayFrame::Forward { .. })]));
        }
        let out = r.on_frame(A, RelayFrame::forward(sid, vec![0; 10]), t0);
        assert_eq!(out[..2], [Delivery::Send(A, RelayFrame::rate_limited(45)), Delivery::Close(A)]);
        assert!(matches!(&out[2], Delivery::Send(B, RelayFrame::PeerPresence { online: false, .. })));
        // A new minute starts the count again; bytes are a limit of their own.
        set_wall(T0 + 60);
        assert!(matches!(r.on_frame(B, RelayFrame::forward(sid, vec![0; 10]), t0)[..], [Delivery::Send(B, RelayFrame::Error { .. })]), "A is gone");
        r.on_disconnect(A, t0);
        r.on_connect(A, IP_A);
        r.on_frame(A, hello(), t0);
        r.on_frame(A, attach(&ch), t0);
        let out = r.on_frame(B, RelayFrame::forward(sid, vec![0; 101]), t0);
        assert_eq!(err_code(&out[0]), Some(RelayErrorCode::RateLimited));
        r.on_disconnect(B, t0);
        r.tick(t0);
        let hour = &r.stats_report().hours[0];
        assert_eq!(hour.limited_forward, 2);
        assert_eq!((hour.forward_frames_max, hour.forward_bytes_max), (3, 111));
        assert_eq!(hour.forward_frames_hist.iter().sum::<u64>(), 2, "A's first minute and B's second; a refused forward counts too");
    }

    #[test]
    fn connection_limits_refuse_new_connections() {
        let t0 = Instant::now();
        let mut r = ticketed();
        let limits = crate::policy::UsageLimits { connections_per_ip: Some(2), max_connections: Some(3), ..Default::default() };
        r.set_policy(RelayPolicy { limits, ..RelayPolicy::default() });
        assert!(r.on_connect(ConnId(1), IP_A).is_empty());
        assert!(r.on_connect(ConnId(2), IP_A).is_empty());
        assert_eq!(r.on_connect(ConnId(3), IP_A), vec![Delivery::Send(ConnId(3), RelayFrame::rate_limited(60)), Delivery::Close(ConnId(3))]);
        assert_eq!(r.on_frame(ConnId(3), hello(), t0), vec![Delivery::Close(ConnId(3))], "a refused connection is not served");
        r.on_disconnect(ConnId(3), t0);
        assert!(r.on_connect(ConnId(4), IP_B).is_empty());
        assert_eq!(err_code(&r.on_connect(ConnId(5), IP_B)[0]), Some(RelayErrorCode::RateLimited), "three in all");
        r.on_disconnect(ConnId(5), t0);
        r.on_disconnect(ConnId(1), t0);
        assert!(r.on_connect(ConnId(6), IP_A).is_empty(), "a place came free");
        let report = r.stats_report();
        let hour = &report.hours[0];
        assert_eq!(hour.limited_connections, 2);
        assert_eq!((hour.ip_connections_max, hour.ip_connections_hist), (2, [1, 1, 0, 0, 0, 0, 0]), "A peaked at 2, B at 1");
        let json = serde_json::to_string(&report).unwrap();
        assert!(!json.contains(&IP_A.to_string()) && !json.contains(&IP_B.to_string()));
        // The next hour keeps this hour's peaks and starts from what is open.
        set_wall(T0 + 3600);
        r.tick(t0);
        let report = r.stats_report();
        assert_eq!(report.hours[0].ip_connections_max, 2);
        assert_eq!(report.hours[1].ip_connections_hist, [1, 1, 0, 0, 0, 0, 0], "A with two open, B with one");
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
