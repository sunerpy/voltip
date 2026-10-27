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
        };
        *field += 1;
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
        StatsReport { schema: 1, now, gauges, hours: self.hours.iter().cloned().collect() }
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
