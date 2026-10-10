//! The one-time pairing ticket carried by the QR code.
//!
//! `voltip://pair?v=1&t=<base64url(CBOR PairingTicket)>` — CBOR keeps the QR small; base64url
//! keeps it URL-safe. The ticket never contains long-term identity keys or credentials.

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::{CodecError, ProtocolVersion, SessionId};

/// URI scheme used by the QR code.
pub const TICKET_SCHEME: &str = "voltip";
/// URI host segment.
pub const TICKET_HOST: &str = "pair";
/// Hard cap on the encoded ticket so it fits a version-10 QR code comfortably.
pub const MAX_TICKET_CHARS: usize = 600;
/// Size of the anti-replay nonce.
pub const NONCE_LEN: usize = 16;
/// Size of an X25519 public key.
pub const PUBLIC_KEY_LEN: usize = 32;

/// What the phone learns by scanning the QR code.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct PairingTicket {
    /// Protocol version.
    pub version: ProtocolVersion,
    /// Session to join at the relay.
    pub session_id: SessionId,
    /// Initiator's Noise ephemeral public key. The first handshake message MUST carry
    /// exactly this key, which is what binds the QR code to the key exchange.
    #[serde(with = "serde_bytes")]
    pub ephemeral_pub: [u8; PUBLIC_KEY_LEN],
    /// Random nonce recorded by both sides; a ticket with a seen nonce is a replay.
    #[serde(with = "serde_bytes")]
    pub nonce: [u8; NONCE_LEN],
    /// Unix seconds after which the ticket is dead.
    pub expires_at: u64,
    /// Relay the initiator is waiting on, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_hint: Option<url::Url>,
    // Builds before 0.1.0 also wrote `direct_hints`, the initiator's LAN addresses: they are
    // ignored on decode (docs/pairing.md 「只走中继」).
}

impl PairingTicket {
    /// Encode as the `voltip://pair?...` URI that goes into the QR code.
    pub fn to_uri(&self) -> Result<String, CodecError> {
        let mut cbor = Vec::new();
        ciborium::into_writer(self, &mut cbor).map_err(|e| CodecError::Malformed(e.to_string()))?;
        let t = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(cbor);
        let uri = format!("{TICKET_SCHEME}://{TICKET_HOST}?v={}&t={t}", self.version.0);
        if uri.len() > MAX_TICKET_CHARS {
            return Err(CodecError::InvalidField { field: "ticket", reason: format!("{} chars exceeds {MAX_TICKET_CHARS}", uri.len()) });
        }
        Ok(uri)
    }

    /// Parse a scanned URI. Rejects wrong scheme/host, unknown version, oversize input.
    pub fn from_uri(uri: &str) -> Result<Self, CodecError> {
        if uri.len() > MAX_TICKET_CHARS {
            return Err(CodecError::InvalidField { field: "ticket", reason: "too long".into() });
        }
        let parsed = url::Url::parse(uri).map_err(|e| CodecError::Malformed(e.to_string()))?;
        if parsed.scheme() != TICKET_SCHEME || parsed.host_str() != Some(TICKET_HOST) {
            return Err(CodecError::InvalidField { field: "scheme", reason: "not a voltip pairing link".into() });
        }
        let mut version: Option<u16> = None;
        let mut payload: Option<String> = None;
        for (k, v) in parsed.query_pairs() {
            match k.as_ref() {
                "v" => version = Some(v.parse().map_err(|_| CodecError::InvalidField { field: "v", reason: "not a number".into() })?),
                "t" => payload = Some(v.into_owned()),
                _ => {}
            }
        }
        let version = version.ok_or(CodecError::InvalidField { field: "v", reason: "missing".into() })?;
        ProtocolVersion(version).check()?;
        let payload = payload.ok_or(CodecError::InvalidField { field: "t", reason: "missing".into() })?;
        let cbor = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload).map_err(|e| CodecError::Malformed(e.to_string()))?;
        let ticket: Self = ciborium::from_reader(cbor.as_slice()).map_err(|e| CodecError::Malformed(e.to_string()))?;
        ticket.version.check()?;
        Ok(ticket)
    }

    /// `true` when `now` (unix seconds) is past `expires_at`.
    pub fn is_expired_at(&self, now_unix_secs: u64) -> bool {
        now_unix_secs >= self.expires_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> PairingTicket {
        PairingTicket {
            version: ProtocolVersion::CURRENT,
            session_id: SessionId::random(),
            ephemeral_pub: [7; PUBLIC_KEY_LEN],
            nonce: [9; NONCE_LEN],
            expires_at: 1_800_000_120,
            relay_hint: Some(url::Url::parse("wss://relay.example/ws").unwrap()),
        }
    }

    #[test]
    fn ticket_roundtrips_and_fits_a_qr() {
        let t = sample();
        let uri = t.to_uri().unwrap();
        assert!(uri.starts_with("voltip://pair?v=1&t="), "{uri}");
        assert!(uri.len() < 300, "ticket too long for a comfortable QR: {}", uri.len());
        assert_eq!(PairingTicket::from_uri(&uri).unwrap(), t);
    }

    #[test]
    fn ticket_without_a_relay_hint_roundtrips() {
        let mut t = sample();
        t.relay_hint = None;
        let uri = t.to_uri().unwrap();
        assert_eq!(PairingTicket::from_uri(&uri).unwrap(), t);
    }

    #[test]
    fn rejects_foreign_links_and_garbage() {
        assert!(matches!(PairingTicket::from_uri("https://evil.example/?v=1&t=AAAA").unwrap_err(), CodecError::InvalidField { field: "scheme", .. }));
        assert!(matches!(PairingTicket::from_uri("voltip://other?v=1&t=AAAA").unwrap_err(), CodecError::InvalidField { field: "scheme", .. }));
        assert!(matches!(PairingTicket::from_uri("voltip://pair?t=AAAA").unwrap_err(), CodecError::InvalidField { field: "v", .. }));
        assert!(matches!(PairingTicket::from_uri("voltip://pair?v=x&t=AAAA").unwrap_err(), CodecError::InvalidField { field: "v", .. }));
        assert!(matches!(PairingTicket::from_uri("voltip://pair?v=1").unwrap_err(), CodecError::InvalidField { field: "t", .. }));
        assert!(matches!(PairingTicket::from_uri("voltip://pair?v=1&t=***").unwrap_err(), CodecError::Malformed(_)));
        assert!(matches!(PairingTicket::from_uri("voltip://pair?v=1&t=AAAA").unwrap_err(), CodecError::Malformed(_)));
        assert!(matches!(PairingTicket::from_uri("::not a url").unwrap_err(), CodecError::Malformed(_)));
        let long = format!("voltip://pair?v=1&t={}", "A".repeat(MAX_TICKET_CHARS));
        assert!(matches!(PairingTicket::from_uri(&long).unwrap_err(), CodecError::InvalidField { field: "ticket", .. }));
    }

    #[test]
    fn rejects_unknown_version_in_query_and_in_body() {
        assert!(matches!(PairingTicket::from_uri("voltip://pair?v=9&t=AAAA").unwrap_err(), CodecError::Version(_)));
        let mut t = sample();
        t.version = ProtocolVersion(2);
        let uri = t.to_uri().unwrap();
        assert!(matches!(PairingTicket::from_uri(&uri).unwrap_err(), CodecError::Version(_)));
    }

    #[test]
    fn oversize_ticket_is_refused_at_encode_time() {
        let mut t = sample();
        t.relay_hint = Some(url::Url::parse(&format!("wss://relay.example/{}", "w".repeat(MAX_TICKET_CHARS))).unwrap());
        assert!(matches!(t.to_uri().unwrap_err(), CodecError::InvalidField { field: "ticket", .. }));
    }

    #[test]
    fn expiry_check_is_inclusive_at_the_boundary() {
        let t = sample();
        assert!(!t.is_expired_at(t.expires_at - 1));
        assert!(t.is_expired_at(t.expires_at));
        assert!(t.is_expired_at(t.expires_at + 1));
    }

    #[test]
    fn ticket_never_carries_identity_material() {
        // Structural guarantee: the CBOR map has exactly these keys and nothing else.
        let t = sample();
        let mut cbor = Vec::new();
        ciborium::into_writer(&t, &mut cbor).unwrap();
        let value: ciborium::Value = ciborium::from_reader(cbor.as_slice()).unwrap();
        let keys: Vec<String> = match value {
            ciborium::Value::Map(m) => m.into_iter().map(|(k, _)| k.into_text().unwrap()).collect(),
            other => panic!("expected map, got {other:?}"),
        };
        assert_eq!(keys, ["version", "session_id", "ephemeral_pub", "nonce", "expires_at", "relay_hint"]);
    }

    /// A ticket from a build before 0.1.0 carries the initiator's LAN addresses as well
    /// (`direct_hints`): it still decodes, and the addresses are dropped.
    #[test]
    fn regression_a_ticket_with_the_old_lan_hints_still_decodes() {
        let t = sample();
        let mut cbor = Vec::new();
        ciborium::into_writer(&t, &mut cbor).unwrap();
        let mut value: ciborium::Value = ciborium::from_reader(cbor.as_slice()).unwrap();
        let ciborium::Value::Map(fields) = &mut value else { panic!("a map") };
        fields.push((ciborium::Value::Text("direct_hints".into()), ciborium::Value::Array(vec![ciborium::Value::Text("192.168.1.24:47831".into())])));
        let mut old = Vec::new();
        ciborium::into_writer(&value, &mut old).unwrap();
        let uri = format!("{TICKET_SCHEME}://{TICKET_HOST}?v=1&t={}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(old));
        assert_eq!(PairingTicket::from_uri(&uri).unwrap(), t);
    }
}
