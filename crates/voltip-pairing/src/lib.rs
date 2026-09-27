//! Pairing state machines. **Sans-IO**: nothing here touches sockets or clocks. The caller
//! feeds [`Event`]s (relay frames, peer bytes, user decisions, ticks) and executes the
//! returned [`Action`]s. That is what makes every path unit-testable without a network.
//!
//! ```text
//! Idle → CreatingSession → WaitingForPeer → KeyExchange → AwaitingVerification → Trusted
//!                                    ↘ Expired            ↘ Failed(reason)    ↘ Rejected
//! ```
//! See `docs/pairing.md`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod common;
mod engine;
mod initiator;
mod nonce_ledger;
mod responder;

pub use common::{Action, Established, Event, FailureReason, JoinMethod, Now, PairingState, Snapshot, Timeouts};
pub use initiator::{Initiator, Reachability};
pub use nonce_ledger::NonceLedger;
pub use responder::Responder;

/// Errors that make a step impossible (programming errors or corrupted input), as opposed to
/// protocol outcomes, which are states.
#[derive(Debug, thiserror::Error)]
pub enum PairingError {
    /// Event not valid in the current state (e.g. `UserConfirm` while idle).
    #[error("event {event} not allowed in state {state:?}")]
    InvalidTransition {
        /// Event name.
        event: &'static str,
        /// State at the time.
        state: PairingState,
    },
    /// Underlying crypto failure (already mapped to a `Failed` state by the machine; surfaced
    /// only when the caller drives a finished machine).
    #[error(transparent)]
    Crypto(#[from] voltip_crypto::CryptoError),
    /// Protocol encode/decode failure.
    #[error(transparent)]
    Protocol(#[from] voltip_protocol::CodecError),
}
