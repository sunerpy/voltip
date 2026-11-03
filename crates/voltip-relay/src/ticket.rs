//! Device tickets (docs/pairing.md 「设备票据」): what `voltip-auth` issues to a device that proved
//! it runs an official build, and what the relay checks in `auth`.
//!
//! A ticket is `base64url(payload) "." base64url(HMAC-SHA256(key, base64url(payload)))`, unpadded;
//! the MAC covers the payload's base64url text, so no party has to reproduce the JSON byte for
//! byte. The payload is `{"v":1,"d":<device id>,"ver":<proven version>,"exp":<unix seconds>,
//! "scope":[…]}`. `voltip-auth`, the built-in service's gateway and this module share the format
//! and the test vector in the tests below.

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, KeyInit as _, Mac as _};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::policy::ClientVersion;

/// The scope a ticket must carry for the relay.
pub const SCOPE_RELAY: &str = "relay";
/// Longest ticket the relay reads.
pub const MAX_TICKET_LEN: usize = 1024;
/// Longest device id a ticket may name.
const MAX_DEVICE_LEN: usize = 64;

/// The ticket key `voltip-auth` signs with (`--ticket-key-file`). Its `Debug` hides the bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct TicketKey(Arc<[u8]>);

impl std::fmt::Debug for TicketKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TicketKey(..)")
    }
}

impl TicketKey {
    /// A key of at least 32 bytes.
    pub fn new(bytes: &[u8]) -> Result<Self, TicketError> {
        if bytes.len() < 32 {
            return Err(TicketError::KeyLength(bytes.len()));
        }
        Ok(Self(bytes.into()))
    }

    /// A key file's text: 64 or more hex characters, surrounding white space ignored.
    pub fn from_hex(text: &str) -> Result<Self, TicketError> {
        let bytes = hex::decode(text.trim()).map_err(|_| TicketError::KeyFormat)?;
        Self::new(&bytes)
    }

    fn mac(&self, payload_b64: &str) -> Hmac<Sha256> {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.0).unwrap_or_else(|_| unreachable!("HMAC takes a key of any length"));
        mac.update(payload_b64.as_bytes());
        mac
    }

    /// Issue a ticket for `payload` (`voltip-auth` issues the real ones; tests and the relay's
    /// end-to-end checks use this).
    pub fn issue(&self, payload: &TicketPayload) -> String {
        let json = serde_json::to_vec(payload).unwrap_or_else(|_| unreachable!("a payload serialises"));
        let body = URL_SAFE_NO_PAD.encode(json);
        let tag = URL_SAFE_NO_PAD.encode(self.mac(&body).finalize().into_bytes());
        format!("{body}.{tag}")
    }

    /// Check `ticket` for `scope` at `now` (Unix seconds).
    pub fn verify(&self, ticket: &str, scope: &str, now: u64) -> Result<Verified, TicketError> {
        if ticket.len() > MAX_TICKET_LEN {
            return Err(TicketError::Format);
        }
        let (body, tag) = ticket.split_once('.').ok_or(TicketError::Format)?;
        let tag = URL_SAFE_NO_PAD.decode(tag).map_err(|_| TicketError::Format)?;
        self.mac(body).verify_slice(&tag).map_err(|_| TicketError::Mac)?;
        let json = URL_SAFE_NO_PAD.decode(body).map_err(|_| TicketError::Format)?;
        let payload: TicketPayload = serde_json::from_slice(&json).map_err(|_| TicketError::Format)?;
        if payload.v != 1 || payload.d.is_empty() || payload.d.len() > MAX_DEVICE_LEN {
            return Err(TicketError::Format);
        }
        if !payload.scope.iter().any(|s| s == scope) {
            return Err(TicketError::Scope);
        }
        if payload.exp <= now {
            return Err(TicketError::Expired);
        }
        Ok(Verified { device: payload.d, version: payload.ver.parse().ok(), exp: payload.exp })
    }
}

/// A ticket's payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TicketPayload {
    /// Format, 1.
    pub v: u32,
    /// The device id.
    pub d: String,
    /// The version the device last proved with its build key.
    pub ver: String,
    /// Unix seconds when the ticket runs out.
    pub exp: u64,
    /// What the ticket is good for (`relay`, `builtin`).
    pub scope: Vec<String>,
}

/// What a valid ticket says.
#[derive(Clone, PartialEq, Eq)]
pub struct Verified {
    /// The device id (never logged or counted by name).
    pub device: String,
    /// The proven version, when it is `x.y.z`.
    pub version: Option<ClientVersion>,
    /// Unix seconds when it runs out.
    pub exp: u64,
}

impl std::fmt::Debug for Verified {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Verified").field("version", &self.version).field("exp", &self.exp).finish_non_exhaustive()
    }
}

/// Why a ticket or a key was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TicketError {
    /// Not `payload.mac`, or a payload that is not a ticket's.
    #[error("not a ticket")]
    Format,
    /// The MAC does not match.
    #[error("bad ticket MAC")]
    Mac,
    /// It does not cover this scope.
    #[error("ticket not for this scope")]
    Scope,
    /// It ran out.
    #[error("ticket expired")]
    Expired,
    /// A key shorter than 32 bytes.
    #[error("ticket key is {0} bytes, fewer than 32")]
    KeyLength(usize),
    /// A key file that is not hex.
    #[error("ticket key is not hex")]
    KeyFormat,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shared test vector: `voltip-auth` (auth/test) and the gateway's `access.js` tests
    /// check the same key, payload and ticket.
    const VECTOR_KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
    const VECTOR_TICKET: &str = "eyJ2IjoxLCJkIjoiMDAwMDAwMDAtMDAwMC00MDAwLTgwMDAtMDAwMDAwMDAwMDAxIiwidmVyIjoiMC4zLjEiLCJleHAiOjE4MDAwMDAwMDAsInNjb3BlIjpbInJlbGF5IiwiYnVpbHRpbiJdfQ.G6ZRnvgPIuhuwFjIQkL9KIxRZ4NMq1n7BAXLeh5bTP0";

    fn payload() -> TicketPayload {
        TicketPayload {
            v: 1,
            d: "00000000-0000-4000-8000-000000000001".into(),
            ver: "0.3.1".into(),
            exp: 1_800_000_000,
            scope: vec!["relay".into(), "builtin".into()],
        }
    }

    #[test]
    fn the_shared_vector_holds() {
        let key = TicketKey::from_hex(VECTOR_KEY).unwrap();
        assert_eq!(key.issue(&payload()), VECTOR_TICKET);
        assert_eq!(key.verify(VECTOR_TICKET, SCOPE_RELAY, 0).unwrap().exp, 1_800_000_000);
    }

    #[test]
    fn a_ticket_verifies_only_whole_in_scope_and_in_time() {
        let key = TicketKey::from_hex(VECTOR_KEY).unwrap();
        let ticket = key.issue(&payload());
        let ok = key.verify(&ticket, SCOPE_RELAY, 1_799_999_999).unwrap();
        assert_eq!((ok.device.as_str(), ok.version, ok.exp), ("00000000-0000-4000-8000-000000000001", Some(ClientVersion::new(0, 3, 1)), 1_800_000_000));
        assert_eq!(key.verify(&ticket, SCOPE_RELAY, 1_800_000_000), Err(TicketError::Expired));
        assert_eq!(key.verify(&ticket, "other", 0), Err(TicketError::Scope));
        let other = TicketKey::new(&[7; 32]).unwrap();
        assert_eq!(other.verify(&ticket, SCOPE_RELAY, 0), Err(TicketError::Mac), "another key's ticket");
        let (body, tag) = ticket.split_once('.').unwrap();
        let forged = format!("{}.{tag}", URL_SAFE_NO_PAD.encode(br#"{"v":1,"d":"x","ver":"9.9.9","exp":9999999999,"scope":["relay"]}"#));
        assert_eq!(key.verify(&forged, SCOPE_RELAY, 0), Err(TicketError::Mac), "a changed payload");
        for broken in ["", "no-dot", &format!("{body}."), &format!("{body}.@@"), &"a".repeat(MAX_TICKET_LEN + 1)] {
            assert!(key.verify(broken, SCOPE_RELAY, 0).is_err(), "{broken}");
        }
        let unversioned = key.issue(&TicketPayload { ver: "dev".into(), ..payload() });
        assert_eq!(key.verify(&unversioned, SCOPE_RELAY, 0).unwrap().version, None);
        let long_device = key.issue(&TicketPayload { d: "d".repeat(65), ..payload() });
        assert_eq!(key.verify(&long_device, SCOPE_RELAY, 0), Err(TicketError::Format));
    }

    #[test]
    fn keys_are_long_hex_and_never_printed() {
        assert_eq!(TicketKey::new(&[1; 31]), Err(TicketError::KeyLength(31)));
        assert_eq!(TicketKey::from_hex("zz"), Err(TicketError::KeyFormat));
        assert!(TicketKey::from_hex(&format!(" {VECTOR_KEY}\n")).is_ok(), "a key file's line break");
        let key = TicketKey::from_hex(VECTOR_KEY).unwrap();
        assert_eq!(format!("{key:?}"), "TicketKey(..)");
        let verified = key.verify(&key.issue(&payload()), SCOPE_RELAY, 0).unwrap();
        assert!(!format!("{verified:?}").contains("0000-4000"), "the device id stays out of Debug");
    }
}
