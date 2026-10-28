//! UI-facing read models.

use serde::{Deserialize, Serialize};
use voltip_identity::TrustedDevice;
use voltip_transport::{ConnectionState, FailureKind};

/// Live connectivity of a trusted device.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum DeviceConnection {
    /// Not reachable right now.
    Offline,
    /// Peer is attached to the same channel; secure channel is being (re)established.
    Connecting,
    /// Secure channel up.
    Online,
    /// Peer presented a different identity key than the trusted record. Never auto-trusted.
    IdentityChanged {
        /// `A7:C4:…` of the key that was presented.
        presented_fingerprint: String,
    },
}

/// One row of the device list.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct DeviceView {
    /// Persisted record.
    pub device: TrustedDevice,
    /// Live state.
    pub connection: DeviceConnection,
}

/// Relay link status for the toolbar.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct RelayStatus {
    /// The relay the user configured (display form). `None` for the build's own relay — its host
    /// is never shown — and when no relay is in use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// Where the relay in use comes from.
    #[serde(default)]
    pub source: RelaySource,
    /// Link state; `Disconnected` when no relay is configured.
    pub state: ConnectionState,
    /// Consecutive reconnect attempts.
    pub attempts: u32,
    /// Why the last attempt failed or the connection went down; `None` once it is connected, and
    /// before any attempt failed (docs/pairing.md 「重连」).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_failure: Option<RelayFailure>,
}

/// Why the relay connection last failed: what the phone's pairing page says while it is not
/// connected (issue #193, where it could only say 「尚未连上中继」).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct RelayFailure {
    /// What kind of failure.
    pub kind: FailureKind,
    /// The transport's words, for a relay the user configured only: those of the build's own relay
    /// can name its host, which never reaches the UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The lowest version the relay serves, with `too_old` (docs/pairing.md 「最低版本」). A
    /// release version names no host, so the build's own relay keeps it too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_version: Option<String>,
    /// When (Unix milliseconds).
    pub at: u64,
}

impl RelayFailure {
    /// `failure` as the status of a relay from `source` shows it: the transport's words only for a
    /// relay the user configured.
    pub(crate) fn of(failure: voltip_transport::LinkFailure, source: RelaySource, at: u64) -> Self {
        Self { kind: failure.kind, detail: (source == RelaySource::User).then_some(failure.detail), min_version: failure.min_version, at }
    }
}

/// Where the relay in use comes from.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelaySource {
    /// No relay (switched off, or none configured in a release build).
    #[default]
    None,
    /// The build's own relay (`VOLTIP_RELAY_URL`, or the loopback default of a debug build).
    Builtin,
    /// The relay the user entered (`Settings.relay_url`).
    User,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression guard (goal review 2026-09-27, kept for issue #193's failure): the build's own
    /// relay is a build secret, so its failure keeps its kind and loses the words, which can name
    /// the host; a relay the user typed in keeps them.
    #[test]
    fn a_relay_failure_keeps_its_words_for_the_users_own_relay_only() {
        let failure =
            || voltip_transport::LinkFailure { kind: FailureKind::Dns, detail: "resolve: wss://relay.example.test: no such name".into(), min_version: None };
        let builtin = RelayFailure::of(failure(), RelaySource::Builtin, 7);
        assert_eq!(builtin, RelayFailure { kind: FailureKind::Dns, detail: None, min_version: None, at: 7 });
        assert_eq!(serde_json::to_string(&builtin).unwrap(), r#"{"kind":"dns","at":7}"#);
        let own = RelayFailure::of(failure(), RelaySource::User, 7);
        assert_eq!(own.detail.as_deref(), Some("resolve: wss://relay.example.test: no such name"));
    }

    /// The minimum version names no host: the build's own relay keeps it, so the apps can say which
    /// version to update to (docs/pairing.md 「最低版本」).
    #[test]
    fn a_too_old_failure_keeps_the_minimum_version_for_every_relay() {
        let failure =
            voltip_transport::LinkFailure { kind: FailureKind::TooOld, detail: "hello refused: ClientTooOld".into(), min_version: Some("0.2.0".into()) };
        let builtin = RelayFailure::of(failure, RelaySource::Builtin, 7);
        assert_eq!(serde_json::to_string(&builtin).unwrap(), r#"{"kind":"too_old","min_version":"0.2.0","at":7}"#);
        let old: RelayFailure = serde_json::from_str(r#"{"kind":"timeout","at":7}"#).unwrap();
        assert_eq!(old.min_version, None, "a status from before the field reads without it");
    }

    #[test]
    fn connection_serializes_tagged() {
        let j = serde_json::to_string(&DeviceConnection::Online).unwrap();
        assert_eq!(j, r#"{"state":"online"}"#);
        let j = serde_json::to_string(&DeviceConnection::IdentityChanged { presented_fingerprint: "AA".into() }).unwrap();
        assert!(j.contains("identity_changed"));
        let s = RelayStatus { endpoint: None, source: RelaySource::Builtin, state: ConnectionState::Disconnected, attempts: 0, last_failure: None };
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("endpoint") && json.contains(r#""source":"builtin""#), "{json}");
    }
}
