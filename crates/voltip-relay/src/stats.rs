//! The relay's statistics for the admin console (docs/pairing.md 「中继统计」): counts only, by
//! hour, for the last 48 hours. Nothing here names an address, a channel, a session or a code; a
//! client's version is counted under its name only when the policy lists that release, or while
//! an hour has named fewer than [`UNLISTED_VERSION_SLOTS`] versions.

use std::collections::{BTreeMap, VecDeque};

use serde::Serialize;

use crate::policy::{ClientVersion, RelayPolicy};

/// Hours kept.
pub const HOURS_KEPT: usize = 48;
/// Versions an hour names by themselves when the policy lists none; the rest count as `other`.
pub const UNLISTED_VERSION_SLOTS: usize = 32;
/// The name of every version not counted by its own.
pub const OTHER_VERSION: &str = "other";
/// Bucket edges (inclusive upper bounds) of the bytes one connection forwarded in a minute; one
/// more bucket holds what is above the last edge.
pub const FORWARD_BYTES_EDGES: [u64; 6] = [64 << 10, 256 << 10, 1 << 20, 4 << 20, 16 << 20, 64 << 20];
/// Bucket edges of the frames one connection forwarded in a minute.
pub const FORWARD_FRAMES_EDGES: [u64; 4] = [60, 300, 1200, 6000];
/// Bucket edges of one address's most connections at once in an hour.
pub const IP_CONNECTIONS_EDGES: [u64; 6] = [1, 2, 4, 8, 16, 32];

fn bucket(value: u64, edges: &[u64]) -> usize {
    edges.iter().position(|e| value <= *e).unwrap_or(edges.len())
}

/// One hour's counts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct HourStats {
    /// Unix seconds at the start of the hour.
    pub hour: u64,
    /// Most connections open at once.
    pub peak_connections: u64,
    /// `hello`s answered.
    pub hellos: u64,
    /// `hello`s by client version (`other` for the rest).
    pub versions: BTreeMap<String, u64>,
    /// Pairing sessions created.
    pub sessions_created: u64,
    /// Pairing sessions the other device joined: both ends met. Whether the users then confirmed
    /// the pairing travels encrypted and is not the relay's to see.
    pub sessions_joined: u64,
    /// Pairing sessions that expired.
    pub sessions_expired: u64,
    /// Requests refused by a per-address limit.
    pub rate_limited: u64,
    /// Clients told `client_too_old`.
    pub too_old: u64,
    /// Requests of older clients below the minimum refused with `unsupported_version`.
    pub old_refused: u64,
    /// Successful `attach`es.
    pub attaches: u64,
    /// Frames forwarded.
    pub forwarded: u64,
    /// Device tickets (docs/pairing.md 「设备票据」).
    pub auth: AuthCounts,
    /// Minutes in which a connection forwarded, by the bytes it forwarded
    /// ([`FORWARD_BYTES_EDGES`], plus one bucket above the last edge).
    pub forward_bytes_hist: [u64; 7],
    /// The same minutes by the frames forwarded ([`FORWARD_FRAMES_EDGES`]).
    pub forward_frames_hist: [u64; 5],
    /// Most bytes one connection forwarded in a minute.
    pub forward_bytes_max: u64,
    /// Most frames one connection forwarded in a minute.
    pub forward_frames_max: u64,
    /// Connections closed for forwarding over a limit.
    pub limited_forward: u64,
    /// Connections refused for a connection limit.
    pub limited_connections: u64,
    /// Client addresses by their most connections at once ([`IP_CONNECTIONS_EDGES`]).
    pub ip_connections_hist: [u64; 7],
    /// Most connections one address held at once.
    pub ip_connections_max: u64,
    /// Connections from addresses past the first 4096 of the hour, not in the histogram.
    pub ip_overflow: u64,
}

/// The ticket counts of an hour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct AuthCounts {
    /// Tickets accepted.
    pub ok: u64,
    /// Tickets refused (forged, expired, another scope).
    pub invalid: u64,
    /// Tickets that ran out on a connection.
    pub expired: u64,
    /// `hello`s without the `ticket` feature (clients before 0.3.0).
    pub no_feature: u64,
    /// Connections with the feature that asked for a session or a channel without a ticket.
    pub unauthenticated: u64,
    /// Requests refused for want of a ticket (under the gate).
    pub refused: u64,
    /// Connections cut off (the gate, an expired ticket, a limit).
    pub revoked: u64,
}

/// One hour's per-address connection peaks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct IpPeaks {
    hist: [u64; 7],
    max: u64,
    overflow: u64,
}

impl IpPeaks {
    pub(crate) fn of(peaks: impl Iterator<Item = u64>, overflow: u64) -> Self {
        let mut out = Self { overflow, ..Self::default() };
        for peak in peaks {
            out.hist[bucket(peak, &IP_CONNECTIONS_EDGES)] += 1;
            out.max = out.max.max(peak);
        }
        out
    }

    pub(crate) fn apply(&self, hour: &mut HourStats) {
        hour.ip_connections_hist = self.hist;
        hour.ip_connections_max = self.max;
        hour.ip_overflow = self.overflow;
    }
}

/// What `GET /stats` answers (`schema` 1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StatsReport {
    /// Always 1.
    pub schema: u32,
    /// Unix seconds.
    pub now: u64,
    /// Right now.
    pub gauges: Gauges,
    /// The hours with any activity, oldest first.
    pub hours: Vec<HourStats>,
    /// The histograms' bucket edges.
    pub edges: Edges,
}

/// The bucket edges of the histograms in [`HourStats`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Edges {
    /// [`FORWARD_BYTES_EDGES`].
    pub forward_bytes: [u64; 6],
    /// [`FORWARD_FRAMES_EDGES`].
    pub forward_frames: [u64; 4],
    /// [`IP_CONNECTIONS_EDGES`].
    pub ip_connections: [u64; 6],
}

/// The relay as it is now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Gauges {
    /// Open connections.
    pub connections: u64,
    /// Live pairing sessions.
    pub sessions: u64,
    /// Rendezvous channels with at least one device.
    pub channels: u64,
    /// Rendezvous channels with both devices.
    pub channels_paired: u64,
}

/// Something worth counting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Count {
    SessionCreated,
    SessionJoined,
    SessionExpired,
    RateLimited,
    TooOld,
    OldRefused,
    Attach,
    Forward,
    AuthOk,
    AuthInvalid,
    AuthExpired,
    NoTicketFeature,
    TicketUnauthenticated,
    TicketRefused,
    Revoked,
    LimitedForward,
    LimitedConnections,
}

/// The hours kept, newest last.
#[derive(Debug, Default)]
pub(crate) struct StatsBook {
    hours: VecDeque<HourStats>,
}

impl StatsBook {
    fn hour(&mut self, unix: u64) -> &mut HourStats {
        let start = unix - unix % 3600;
        if self.hours.back().is_none_or(|h| h.hour < start) {
            self.hours.push_back(HourStats { hour: start, ..HourStats::default() });
            while self.hours.len() > HOURS_KEPT {
                self.hours.pop_front();
            }
        }
        // A wall clock that went back counts into the newest hour.
        self.hours.back_mut().unwrap_or_else(|| unreachable!("an hour was just pushed"))
    }

    pub(crate) fn count(&mut self, what: Count, unix: u64) {
        let hour = self.hour(unix);
        let field = match what {
            Count::SessionCreated => &mut hour.sessions_created,
            Count::SessionJoined => &mut hour.sessions_joined,
            Count::SessionExpired => &mut hour.sessions_expired,
            Count::RateLimited => &mut hour.rate_limited,
            Count::TooOld => &mut hour.too_old,
            Count::OldRefused => &mut hour.old_refused,
            Count::Attach => &mut hour.attaches,
            Count::Forward => &mut hour.forwarded,
            Count::AuthOk => &mut hour.auth.ok,
            Count::AuthInvalid => &mut hour.auth.invalid,
            Count::AuthExpired => &mut hour.auth.expired,
            Count::NoTicketFeature => &mut hour.auth.no_feature,
            Count::TicketUnauthenticated => &mut hour.auth.unauthenticated,
            Count::TicketRefused => &mut hour.auth.refused,
            Count::Revoked => &mut hour.auth.revoked,
            Count::LimitedForward => &mut hour.limited_forward,
            Count::LimitedConnections => &mut hour.limited_connections,
        };
        *field += 1;
    }

    /// What one connection forwarded in the minute starting at `minute_start` (nothing when it
    /// forwarded nothing).
    pub(crate) fn forward_sample(&mut self, minute_start: u64, bytes: u64, frames: u64) {
        if frames == 0 {
            return;
        }
        let hour = self.hour(minute_start);
        hour.forward_bytes_hist[bucket(bytes, &FORWARD_BYTES_EDGES)] += 1;
        hour.forward_frames_hist[bucket(frames, &FORWARD_FRAMES_EDGES)] += 1;
        hour.forward_bytes_max = hour.forward_bytes_max.max(bytes);
        hour.forward_frames_max = hour.forward_frames_max.max(frames);
    }

    /// The finished hour starting at `start` keeps `peaks`. An hour no longer kept, or one older
    /// than the newest that never had an entry, is left alone.
    pub(crate) fn ip_peaks(&mut self, start: u64, peaks: &IpPeaks) {
        if let Some(hour) = self.hours.iter_mut().rev().find(|h| h.hour == start) {
            peaks.apply(hour);
        } else if self.hours.back().is_none_or(|h| h.hour < start) {
            peaks.apply(self.hour(start));
        }
    }

    pub(crate) fn connections(&mut self, open: usize, unix: u64) {
        let hour = self.hour(unix);
        hour.peak_connections = hour.peak_connections.max(open as u64);
    }

    /// A `hello` from a client of `version` (`None`: none could be read).
    pub(crate) fn hello(&mut self, version: Option<ClientVersion>, policy: &RelayPolicy, unix: u64) {
        let hour = self.hour(unix);
        hour.hellos += 1;
        let name = version.map(|v| v.to_string()).filter(|name| {
            if policy.known_versions.is_empty() {
                hour.versions.contains_key(name) || hour.versions.len() < UNLISTED_VERSION_SLOTS
            } else {
                version.is_some_and(|v| policy.known_versions.binary_search(&v).is_ok())
            }
        });
        *hour.versions.entry(name.unwrap_or_else(|| OTHER_VERSION.to_owned())).or_default() += 1;
    }

    pub(crate) fn report(&self, gauges: Gauges, now: u64) -> StatsReport {
        StatsReport {
            schema: 1,
            now,
            gauges,
            hours: self.hours.iter().cloned().collect(),
            edges: Edges { forward_bytes: FORWARD_BYTES_EDGES, forward_frames: FORWARD_FRAMES_EDGES, ip_connections: IP_CONNECTIONS_EDGES },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H0: u64 = 1_761_552_000; // an hour's start

    #[test]
    fn counts_go_to_their_hour_and_48_hours_are_kept() {
        let mut book = StatsBook::default();
        book.count(Count::SessionCreated, H0 + 5);
        book.count(Count::SessionCreated, H0 + 3599);
        book.count(Count::SessionJoined, H0 + 3600);
        book.connections(3, H0 + 3600);
        book.connections(2, H0 + 3601);
        let report = book.report(Gauges::default(), H0 + 3700);
        assert_eq!(report.hours.len(), 2);
        assert_eq!((report.hours[0].hour, report.hours[0].sessions_created), (H0, 2));
        assert_eq!((report.hours[1].sessions_joined, report.hours[1].peak_connections), (1, 3), "the peak, not the last value");
        for i in 0..60 {
            book.count(Count::Forward, H0 + i * 3600);
        }
        let report = book.report(Gauges::default(), H0);
        assert_eq!(report.hours.len(), HOURS_KEPT);
        assert_eq!(report.hours[0].hour, H0 + 12 * 3600, "the oldest hours went");
        // A clock that went back counts into the newest hour.
        book.count(Count::Attach, H0);
        assert_eq!(book.report(Gauges::default(), H0).hours.last().unwrap().attaches, 1);
        let json = serde_json::to_value(book.report(Gauges { connections: 4, sessions: 1, channels: 2, channels_paired: 1 }, 9)).unwrap();
        assert_eq!(json["schema"], 1);
        assert_eq!(json["gauges"]["channels_paired"], 1);
        assert!(json["hours"][0].get("versions").is_some());
    }

    #[test]
    fn samples_and_peaks_fall_into_their_buckets() {
        assert_eq!(bucket(0, &FORWARD_BYTES_EDGES), 0);
        assert_eq!(bucket(64 << 10, &FORWARD_BYTES_EDGES), 0, "an edge is inclusive");
        assert_eq!(bucket((64 << 10) + 1, &FORWARD_BYTES_EDGES), 1);
        assert_eq!(bucket(u64::MAX, &FORWARD_BYTES_EDGES), 6, "the overflow bucket");
        let mut book = StatsBook::default();
        book.forward_sample(H0, 10, 0);
        assert!(book.report(Gauges::default(), H0).hours.is_empty(), "a minute without frames is no sample");
        book.forward_sample(H0, 100, 2);
        book.forward_sample(H0 + 60, 2 << 20, 700);
        let hour = &book.report(Gauges::default(), H0).hours[0];
        assert_eq!(hour.forward_bytes_hist, [1, 0, 0, 1, 0, 0, 0]);
        assert_eq!(hour.forward_frames_hist, [1, 0, 1, 0, 0]);
        assert_eq!((hour.forward_bytes_max, hour.forward_frames_max), (2 << 20, 700));
        let peaks = IpPeaks::of([1, 1, 3, 40].into_iter(), 2);
        book.ip_peaks(H0, &peaks);
        let hour = &book.report(Gauges::default(), H0).hours[0];
        assert_eq!((hour.ip_connections_hist, hour.ip_connections_max, hour.ip_overflow), ([2, 0, 1, 0, 0, 0, 1], 40, 2));
        book.count(Count::Attach, H0 + 7200);
        book.ip_peaks(H0 + 3600, &peaks);
        assert_eq!(book.report(Gauges::default(), H0).hours.len(), 2, "an older hour that never had an entry is left alone");
        let json = serde_json::to_value(book.report(Gauges::default(), H0)).unwrap();
        assert_eq!(json["edges"]["forward_frames"], serde_json::json!([60, 300, 1200, 6000]));
        assert_eq!(json["hours"][0]["auth"]["ok"], 0);
    }

    /// The version map has a bound (plan review 2026-10-27: `client_version` is whatever the
    /// client sends, so naming every one would let a client grow the relay's memory).
    #[test]
    fn regression_versions_are_named_only_from_the_policy_or_up_to_the_hours_slots() {
        let listed = RelayPolicy { known_versions: vec![ClientVersion::new(0, 1, 9), ClientVersion::new(0, 1, 10)], ..RelayPolicy::default() };
        let mut book = StatsBook::default();
        book.hello(Some(ClientVersion::new(0, 1, 10)), &listed, H0);
        book.hello(Some(ClientVersion::new(7, 7, 7)), &listed, H0);
        book.hello(None, &listed, H0);
        let hour = &book.report(Gauges::default(), H0).hours[0];
        assert_eq!(hour.hellos, 3);
        assert_eq!(hour.versions, BTreeMap::from([("0.1.10".to_owned(), 1), (OTHER_VERSION.to_owned(), 2)]));

        let unlisted = RelayPolicy::default();
        let mut book = StatsBook::default();
        for patch in 0..100 {
            book.hello(Some(ClientVersion::new(9, 9, patch)), &unlisted, H0);
        }
        book.hello(Some(ClientVersion::new(9, 9, 0)), &unlisted, H0);
        let hour = &book.report(Gauges::default(), H0).hours[0];
        assert_eq!(hour.versions.len(), UNLISTED_VERSION_SLOTS + 1, "32 names and `other`");
        assert_eq!(hour.versions["9.9.0"], 2, "a named version keeps counting");
        assert_eq!(hour.versions[OTHER_VERSION], 100 - UNLISTED_VERSION_SLOTS as u64);
    }
}
