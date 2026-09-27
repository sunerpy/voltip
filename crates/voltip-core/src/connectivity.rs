//! The connectivity self-check (docs/pairing.md): what this device can reach right now, measured
//! when the user asks. Its LAN host, the relay (a fresh connection and `hello`), and every paired
//! device: an encrypted ping on the live channel and a probe of each LAN address it announced, so
//! a phone and a computer on one network can tell "the port is blocked" from "the relay is down".

use serde::{Deserialize, Serialize};
use voltip_identity::ConnectionKind;

/// How long one probe may take (connect, `hello`, `hello_ack`).
pub const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
/// When the report goes out even if a ping is still unanswered.
pub const CHECK_DEADLINE: std::time::Duration = std::time::Duration::from_secs(4);

/// One probe's outcome.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum ProbeResult {
    /// A Voltip relay or LAN host answered `hello` after `ms` milliseconds.
    Ok {
        /// Connect to `hello_ack`.
        ms: u32,
    },
    /// Nothing answered in time: a firewall that drops, Wi-Fi client isolation, a host that is gone.
    Timeout,
    /// The address refused the connection: nothing listens on that port, or a firewall rejects it.
    Refused,
    /// Anything else (DNS, TLS, an endpoint that is not Voltip).
    Failed {
        /// What went wrong.
        reason: String,
    },
}

impl ProbeResult {
    /// The outcome of a transport probe.
    pub fn from_probe(result: Result<std::time::Duration, voltip_transport::ProbeFailure>) -> Self {
        match result {
            Ok(took) => Self::Ok { ms: u32::try_from(took.as_millis()).unwrap_or(u32::MAX) },
            Err(voltip_transport::ProbeFailure::Timeout) => Self::Timeout,
            Err(voltip_transport::ProbeFailure::Refused) => Self::Refused,
            Err(voltip_transport::ProbeFailure::Failed(reason)) => Self::Failed { reason },
        }
    }
}

/// This device's LAN host, where paired devices on the same network connect.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanHostCheck {
    /// The host is up.
    pub listening: bool,
    /// The `ip:port` it announces to paired devices.
    pub addresses: Vec<String>,
}

/// The relay.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayCheck {
    /// A relay is configured and switched on.
    pub configured: bool,
    /// The probe, when there was one to run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<ProbeResult>,
}

/// One LAN address a paired device announced.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddressCheck {
    /// `ip:port`.
    pub address: String,
    /// It is on this device's own IPv4 /24, so a failure points at a firewall or Wi-Fi isolation
    /// rather than at routing.
    pub same_subnet: bool,
    /// The probe.
    pub result: ProbeResult,
}

/// One paired device.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerCheck {
    /// Its public key (hex).
    pub public_key: String,
    /// Its name.
    pub name: String,
    /// How the live channel runs; absent while it is offline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<ConnectionKind>,
    /// Round trip of an encrypted ping on that channel; absent when offline or unanswered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtt_ms: Option<u32>,
    /// Each LAN address it announced, probed from here.
    pub addresses: Vec<AddressCheck>,
}

/// One self-check.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectivityReport {
    /// When it finished (Unix milliseconds).
    pub checked_at: u64,
    /// This device's LAN host.
    pub lan: LanHostCheck,
    /// The relay.
    pub relay: RelayCheck,
    /// Every paired device, in the device list's order.
    pub peers: Vec<PeerCheck>,
}

/// The self-check as the UI shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectivityStatus {
    /// A check is running.
    pub running: bool,
    /// The last finished check, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<ConnectivityReport>,
}

/// The relay probe as the report carries it. A transport error can quote the URL, and the build's
/// own relay is a build secret that never reaches the UI (the relay status leaves its endpoint out
/// too), so its failure keeps the kind and loses the reason.
pub fn shown_relay_result(result: Option<ProbeResult>, builtin: bool) -> Option<ProbeResult> {
    match result {
        Some(ProbeResult::Failed { .. }) if builtin => Some(ProbeResult::Failed { reason: String::new() }),
        other => other,
    }
}

/// Whether `address` (`ip:port`) is on `own`'s IPv4 /24.
pub fn same_subnet(address: &str, own: Option<std::net::IpAddr>) -> bool {
    let (Some(std::net::IpAddr::V4(own)), Ok(std::net::SocketAddr::V4(peer))) = (own, address.parse::<std::net::SocketAddr>()) else { return false };
    own.octets()[..3] == peer.ip().octets()[..3]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_results_map_and_serialise_with_a_tag() {
        assert_eq!(ProbeResult::from_probe(Ok(std::time::Duration::from_millis(12))), ProbeResult::Ok { ms: 12 });
        assert_eq!(ProbeResult::from_probe(Err(voltip_transport::ProbeFailure::Timeout)), ProbeResult::Timeout);
        assert_eq!(ProbeResult::from_probe(Err(voltip_transport::ProbeFailure::Refused)), ProbeResult::Refused);
        assert_eq!(ProbeResult::from_probe(Err(voltip_transport::ProbeFailure::Failed("tls".into()))), ProbeResult::Failed { reason: "tls".into() });
        assert_eq!(serde_json::to_string(&ProbeResult::Ok { ms: 5 }).unwrap(), r#"{"result":"ok","ms":5}"#);
        assert_eq!(serde_json::to_string(&ProbeResult::Timeout).unwrap(), r#"{"result":"timeout"}"#);
    }

    /// Regression (goal review 2026-09-27): the built-in relay's host is a build secret; a DNS or
    /// TLS error that quotes it must not reach the connectivity report. A user's own relay keeps
    /// the reason.
    #[test]
    fn regression_the_built_in_relays_failure_names_no_host() {
        let failed = || Some(ProbeResult::Failed { reason: "Unable to connect to wss://relay.example.test/ws".into() });
        assert_eq!(shown_relay_result(failed(), true), Some(ProbeResult::Failed { reason: String::new() }));
        assert_eq!(shown_relay_result(failed(), false), failed());
        assert_eq!(shown_relay_result(Some(ProbeResult::Ok { ms: 3 }), true), Some(ProbeResult::Ok { ms: 3 }));
        assert_eq!(shown_relay_result(Some(ProbeResult::Timeout), true), Some(ProbeResult::Timeout));
        assert_eq!(shown_relay_result(None, true), None);
    }

    #[test]
    fn same_subnet_compares_the_ipv4_slash_24() {
        let own = Some("192.168.1.24".parse().unwrap());
        assert!(same_subnet("192.168.1.30:47831", own));
        assert!(!same_subnet("192.168.2.30:47831", own));
        assert!(!same_subnet("not an address", own));
        assert!(!same_subnet("192.168.1.30:47831", None));
        assert!(!same_subnet("[fe80::1]:47831", own));
    }
}
