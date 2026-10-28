//! The optional Voltip relay.
//!
//! [`RelayCore`] is **sans-IO**: it maps `(connection, frame, now)` to a list of frames to
//! deliver, and is the whole of the relay's logic — session minting, six-digit code lookup,
//! rendezvous channels, rate limits, failed-attempt counters, TTLs. The `server` feature adds
//! [`server::RelayHandle::serve`], a thin axum WebSocket adapter that also notices connections
//! gone silent.
//!
//! [`policy`] reads the minimum client version from the policy files, and [`stats`] keeps the
//! counts the admin console shows (`GET /stats`), with nothing that names a device.
//!
//! The relay never parses `forward.payload`. See `docs/threat-model.md`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod core;
pub mod forwarded;
mod limits;
pub mod policy;
#[cfg(feature = "server")]
pub mod server;
pub mod stats;

pub use core::{ConnId, Delivery, RelayConfig, RelayCore, RelayStats};
pub use limits::{RateLimiter, RateWindow};
pub use policy::{ClientVersion, PolicyFiles, RelayPolicy};
pub use stats::StatsReport;
