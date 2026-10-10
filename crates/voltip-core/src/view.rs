//! UI-facing read models.

use serde::{Deserialize, Serialize};
use voltip_identity::TrustedDevice;
use voltip_transport::ConnectionState;

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

    #[test]
    fn connection_serializes_tagged() {
        let j = serde_json::to_string(&DeviceConnection::Online).unwrap();
        assert_eq!(j, r#"{"state":"online"}"#);
        let j = serde_json::to_string(&DeviceConnection::IdentityChanged { presented_fingerprint: "AA".into() }).unwrap();
        assert!(j.contains("identity_changed"));
        let s = RelayStatus { endpoint: None, source: RelaySource::Builtin, state: ConnectionState::Disconnected, attempts: 0 };
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("endpoint") && json.contains(r#""source":"builtin""#), "{json}");
    }
}
