//! The optional Voltip relay.
//!
//! [`RelayCore`] is **sans-IO**: it maps `(connection, frame, now)` to a list of frames to
//! deliver, and is the whole of the relay's logic — session minting, six-digit code lookup,
//! rendezvous channels, rate limits, failed-attempt counters, TTLs. The `server` feature adds
//! [`server::RelayHandle::serve`], a thin axum WebSocket adapter. The direct LAN transport embeds the same
//! core in single-session mode, so LAN and relay behave identically.
//!
//! The relay never parses `forward.payload`. See `docs/threat-model.md`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod core;
mod limits;
#[cfg(feature = "server")]
pub mod server;

pub use core::{ConnId, Delivery, RelayConfig, RelayCore, RelayStats};
pub use limits::{RateLimiter, RateWindow};
