//! Transports.
//!
//! * [`ConnectionState`] + [`ConnectionMachine`] — the explicit lifecycle every transport
//!   reports (`Disconnected → Connecting → Authenticating → Connected → Reconnecting → Closed`).
//! * [`ReconnectPolicy`] — exponential backoff with jitter.
//! * [`RelayLink`] — a WebSocket link to a relay (or to a direct host, which speaks the same
//!   frames), with automatic reconnect and typed frame I/O.
//! * [`DirectHost`] — a LAN WebSocket listener that embeds `RelayCore` in single-session mode,
//!   so a phone on the same network can pair without any server.
//! * [`SecureChannel`] — Noise transport mode over any framed link.
//!
//! Endpoints are always injected ([`RelayEndpoint`]); nothing here knows a production URL.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod direct;
mod endpoint;
mod link;
mod reconnect;
mod secure;
mod state;

pub use direct::{DirectHost, primary_lan_ip};
pub use endpoint::{DEFAULT_DEV_RELAY, RelayEndpoint};
pub use link::{LinkConfig, LinkEvent, ProbeFailure, RelayLink, probe};
pub use reconnect::ReconnectPolicy;
pub use secure::SecureChannel;
pub use state::{ConnectionMachine, ConnectionState, StateChange};

/// Transport-layer errors.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    /// TCP / TLS / WebSocket failure.
    #[error("websocket: {0}")]
    WebSocket(String),
    /// Connect or handshake timed out.
    #[error("timed out after {0:?}")]
    Timeout(std::time::Duration),
    /// The relay rejected us at the protocol level.
    #[error("relay error: {0:?}")]
    Relay(voltip_protocol::relay::RelayErrorCode),
    /// Frame could not be (de)coded.
    #[error(transparent)]
    Codec(#[from] voltip_protocol::CodecError),
    /// Noise failure inside the secure channel.
    #[error(transparent)]
    Crypto(#[from] voltip_crypto::CryptoError),
    /// The link is closed and will not reconnect.
    #[error("link closed")]
    Closed,
    /// An operation was attempted in the wrong state.
    #[error("not connected (state {0:?})")]
    NotConnected(ConnectionState),
    /// Bad configuration (unparseable URL, wrong scheme).
    #[error("invalid endpoint: {0}")]
    Endpoint(String),
    /// Local I/O.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
