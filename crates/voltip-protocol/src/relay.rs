//! Relay control frames — the **only** thing a relay is allowed to understand.
//!
//! Frames are JSON text frames with `version` and a `type` tag. The relay never inspects
//! [`RelayFrame::Forward::payload`]; it is opaque ciphertext produced by the Noise channel.

use serde::{Deserialize, Serialize};

use crate::{CodecError, PairCode, ProtocolVersion, SessionId};

/// Default pairing-session lifetime the relay applies when a client sends none.
pub const DEFAULT_SESSION_TTL_SECS: u32 = 120;
/// Longest session lifetime a client may request.
pub const MAX_SESSION_TTL_SECS: u32 = 300;

/// Limits the relay advertises in `hello_ack` so clients can render honest UI copy.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct RelayLimits {
    /// Failed `join_by_code` attempts allowed per connection before it is closed.
    pub code_attempts_per_connection: u32,
    /// Failed attempts allowed against one session before the session is voided.
    pub code_attempts_per_session: u32,
    /// Session lifetime the relay will apply by default.
    pub session_ttl_secs: u32,
}

/// Length of a rendezvous channel label in hex characters.
pub const CHANNEL_HEX_LEN: usize = 64;

/// Validate a channel label: exactly 64 lower-case hex characters.
pub fn validate_channel(channel: &str) -> Result<(), CodecError> {
    let ok = channel.len() == CHANNEL_HEX_LEN && channel.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if ok { Ok(()) } else { Err(CodecError::InvalidField { field: "channel", reason: format!("expected {CHANNEL_HEX_LEN} lower-case hex chars") }) }
}

/// Machine-readable relay error codes (stable strings on the wire).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelayErrorCode {
    /// The six digits did not match any live session.
    InvalidCode,
    /// The session expired, was consumed, or was voided by too many failures.
    SessionExpired,
    /// Somebody already joined this session.
    SessionFull,
    /// Per-connection / per-IP limit hit; `retry_after_secs` says when to try again.
    RateLimited,
    /// `attach.channel` is not 64 hex chars.
    InvalidChannel,
    /// A third connection tried to attach to a channel that already has two parties.
    ChannelFull,
    /// `version` is not one this relay speaks.
    UnsupportedVersion,
    /// `forward` sent before `joined` / `peer_joined`.
    NotJoined,
    /// The first frame was not `hello`.
    HelloRequired,
    /// One live pairing session per connection.
    SessionAlreadyActive,
    /// Frame could not be parsed.
    Malformed,
}

/// Every frame the relay sends or receives.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RelayFrame {
    /// First frame from a client.
    Hello {
        /// Protocol version.
        version: ProtocolVersion,
        /// Client build string, for relay-side metrics only.
        client_version: String,
    },
    /// Relay's answer to `hello`.
    HelloAck {
        /// Protocol version.
        version: ProtocolVersion,
        /// Relay build string.
        relay_version: String,
        /// Advertised limits.
        limits: RelayLimits,
    },
    /// Initiator asks for a pairing session.
    CreateSession {
        /// Protocol version.
        version: ProtocolVersion,
        /// Requested lifetime; clamped to [`MAX_SESSION_TTL_SECS`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ttl_secs: Option<u32>,
    },
    /// Relay minted a session; only the creator receives the code.
    SessionCreated {
        /// Protocol version.
        version: ProtocolVersion,
        /// Session id (goes into the QR ticket).
        session_id: SessionId,
        /// Six-digit code (typed by the responder).
        code: PairCode,
        /// Unix seconds when the session dies.
        expires_at: u64,
    },
    /// Responder joins by typed code.
    JoinByCode {
        /// Protocol version.
        version: ProtocolVersion,
        /// Six digits.
        code: PairCode,
    },
    /// Responder joins by scanned ticket.
    JoinBySession {
        /// Protocol version.
        version: ProtocolVersion,
        /// Session id from the ticket.
        session_id: SessionId,
    },
    /// Sent to the responder after a successful join.
    Joined {
        /// Protocol version.
        version: ProtocolVersion,
        /// Session id.
        session_id: SessionId,
    },
    /// Sent to the initiator when a responder joined.
    PeerJoined {
        /// Protocol version.
        version: ProtocolVersion,
        /// Session id.
        session_id: SessionId,
    },
    /// A party is done with a pairing session (success, failure or cancel). The relay frees the
    /// slot and tells the other party `peer_left`.
    Leave {
        /// Protocol version.
        version: ProtocolVersion,
        /// Session id.
        session_id: SessionId,
    },
    /// Opaque end-to-end payload, forwarded verbatim to the other party.
    Forward {
        /// Protocol version.
        version: ProtocolVersion,
        /// Session id.
        session_id: SessionId,
        /// Ciphertext (base64 on the wire).
        #[serde(with = "b64")]
        payload: Vec<u8>,
    },
    /// Two already-paired devices meet again on a stable rendezvous channel
    /// (`hex(SHA-256(sorted static public keys))`). The relay learns only that two connections
    /// share a 32-byte label.
    Attach {
        /// Protocol version.
        version: ProtocolVersion,
        /// 64 lower-case hex chars.
        channel: String,
    },
    /// Attach succeeded; `session_id` is what `forward` frames must carry.
    Attached {
        /// Protocol version.
        version: ProtocolVersion,
        /// Relay-minted id for this channel binding.
        session_id: SessionId,
        /// Whether the other party is currently attached.
        peer_online: bool,
        /// The `attach.channel` this answers (relays from 0.1.0 on; older ones leave it out and
        /// answer every `attach` in order).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel: Option<String>,
    },
    /// The other party of a channel attached (`true`) or detached (`false`).
    PeerPresence {
        /// Protocol version.
        version: ProtocolVersion,
        /// Session id of the channel binding.
        session_id: SessionId,
        /// Online or not.
        online: bool,
    },
    /// The other side of a session went away.
    PeerLeft {
        /// Protocol version.
        version: ProtocolVersion,
        /// Session id.
        session_id: SessionId,
    },
    /// Error.
    Error {
        /// Protocol version.
        version: ProtocolVersion,
        /// Machine-readable code.
        code: RelayErrorCode,
        /// Present for `rate_limited`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        retry_after_secs: Option<u32>,
        /// The `attach.channel` when this refuses an `attach` (relays from 0.1.0 on).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel: Option<String>,
    },
    /// Graceful close.
    Bye {
        /// Protocol version.
        version: ProtocolVersion,
    },
}

impl RelayFrame {
    /// The version carried by this frame.
    pub fn version(&self) -> ProtocolVersion {
        match self {
            Self::Hello { version, .. }
            | Self::HelloAck { version, .. }
            | Self::CreateSession { version, .. }
            | Self::SessionCreated { version, .. }
            | Self::JoinByCode { version, .. }
            | Self::JoinBySession { version, .. }
            | Self::Joined { version, .. }
            | Self::PeerJoined { version, .. }
            | Self::Leave { version, .. }
            | Self::Forward { version, .. }
            | Self::Attach { version, .. }
            | Self::Attached { version, .. }
            | Self::PeerPresence { version, .. }
            | Self::PeerLeft { version, .. }
            | Self::Error { version, .. }
            | Self::Bye { version } => *version,
        }
    }

    /// Serialize to the JSON text frame that goes over the WebSocket.
    pub fn encode(&self) -> Result<String, CodecError> {
        Ok(serde_json::to_string(self)?)
    }

    /// Parse a text frame and enforce the version.
    pub fn decode(text: &str) -> Result<Self, CodecError> {
        // Peek at the version first so an unknown version yields `Version`, not `Malformed`
        // (a v2 frame may well have fields v1 cannot parse).
        #[derive(Deserialize)]
        struct Head {
            version: ProtocolVersion,
        }
        let head: Head = serde_json::from_str(text)?;
        head.version.check()?;
        Ok(serde_json::from_str(text)?)
    }

    /// Convenience constructor for an error frame.
    pub fn error(code: RelayErrorCode) -> Self {
        Self::Error { version: ProtocolVersion::CURRENT, code, retry_after_secs: None, channel: None }
    }

    /// Convenience constructor for a rate-limit error.
    pub fn rate_limited(retry_after_secs: u32) -> Self {
        Self::Error { version: ProtocolVersion::CURRENT, code: RelayErrorCode::RateLimited, retry_after_secs: Some(retry_after_secs), channel: None }
    }

    /// This frame refusing the `attach` to `channel` ([`RelayFrame::Error`] only).
    pub fn for_channel(self, channel: &str) -> Self {
        match self {
            Self::Error { version, code, retry_after_secs, .. } => Self::Error { version, code, retry_after_secs, channel: Some(channel.to_owned()) },
            other => other,
        }
    }

    /// Convenience constructor for a forward frame.
    pub fn forward(session_id: SessionId, payload: Vec<u8>) -> Self {
        Self::Forward { version: ProtocolVersion::CURRENT, session_id, payload }
    }
}

mod b64 {
    use base64::Engine as _;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        base64::engine::general_purpose::STANDARD.encode(bytes).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        base64::engine::general_purpose::STANDARD.decode(text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(frame: RelayFrame) {
        let text = frame.encode().expect("encode");
        let back = RelayFrame::decode(&text).expect("decode");
        assert_eq!(back, frame);
        assert_eq!(back.version(), ProtocolVersion::CURRENT);
    }

    #[test]
    fn every_frame_roundtrips() {
        let sid = SessionId::random();
        let v = ProtocolVersion::CURRENT;
        roundtrip(RelayFrame::Hello { version: v, client_version: "2.0.0".into() });
        roundtrip(RelayFrame::Attach { version: v, channel: "ab".repeat(32) });
        roundtrip(RelayFrame::Attached { version: v, session_id: sid, peer_online: false, channel: None });
        roundtrip(RelayFrame::Attached { version: v, session_id: sid, peer_online: true, channel: Some("ab".repeat(32)) });
        roundtrip(RelayFrame::PeerPresence { version: v, session_id: sid, online: true });
        roundtrip(RelayFrame::HelloAck {
            version: v,
            relay_version: "relay 2.0".into(),
            limits: RelayLimits { code_attempts_per_connection: 5, code_attempts_per_session: 10, session_ttl_secs: 120 },
        });
        roundtrip(RelayFrame::CreateSession { version: v, ttl_secs: Some(90) });
        roundtrip(RelayFrame::CreateSession { version: v, ttl_secs: None });
        roundtrip(RelayFrame::SessionCreated { version: v, session_id: sid, code: PairCode::new("000123").unwrap(), expires_at: 1_800_000_000 });
        roundtrip(RelayFrame::JoinByCode { version: v, code: PairCode::new("999999").unwrap() });
        roundtrip(RelayFrame::JoinBySession { version: v, session_id: sid });
        roundtrip(RelayFrame::Joined { version: v, session_id: sid });
        roundtrip(RelayFrame::PeerJoined { version: v, session_id: sid });
        roundtrip(RelayFrame::forward(sid, vec![0, 1, 2, 255]));
        roundtrip(RelayFrame::Leave { version: v, session_id: sid });
        roundtrip(RelayFrame::PeerLeft { version: v, session_id: sid });
        roundtrip(RelayFrame::error(RelayErrorCode::InvalidCode));
        roundtrip(RelayFrame::rate_limited(30));
        roundtrip(RelayFrame::error(RelayErrorCode::ChannelFull).for_channel(&"ab".repeat(32)));
        roundtrip(RelayFrame::rate_limited(3).for_channel(&"cd".repeat(32)));
        roundtrip(RelayFrame::Bye { version: v });
    }

    /// The `channel` an attach answer names (relays from 0.1.0 on) is optional both ways: an
    /// older relay's answers decode, and an older client reads a newer relay's (it ignores the
    /// field it does not know).
    #[test]
    fn attach_answers_name_their_channel_only_when_they_can() {
        let ch = "ab".repeat(32);
        let older = r#"{"type":"attached","version":1,"session_id":"00000000-0000-0000-0000-000000000000","peer_online":true}"#;
        assert!(matches!(RelayFrame::decode(older).unwrap(), RelayFrame::Attached { channel: None, peer_online: true, .. }));
        let older = r#"{"type":"error","version":1,"code":"channel_full"}"#;
        assert!(matches!(RelayFrame::decode(older).unwrap(), RelayFrame::Error { code: RelayErrorCode::ChannelFull, channel: None, .. }));
        let text = RelayFrame::error(RelayErrorCode::ChannelFull).for_channel(&ch).encode().unwrap();
        assert_eq!(text, format!(r#"{{"type":"error","version":1,"code":"channel_full","channel":"{ch}"}}"#));
        assert!(!RelayFrame::error(RelayErrorCode::NotJoined).encode().unwrap().contains("channel"));
        // Only an error names a channel this way.
        let bye = RelayFrame::Bye { version: ProtocolVersion::CURRENT };
        assert_eq!(bye.clone().for_channel(&ch), bye);
    }

    #[test]
    fn wire_shape_is_snake_case_tagged_json() {
        let text = RelayFrame::JoinByCode { version: ProtocolVersion::CURRENT, code: PairCode::new("483921").unwrap() }.encode().unwrap();
        assert_eq!(text, r#"{"type":"join_by_code","version":1,"code":"483921"}"#);
        let text = RelayFrame::forward(SessionId(uuid::Uuid::nil()), b"hi".to_vec()).encode().unwrap();
        assert!(text.contains(r#""payload":"aGk=""#), "{text}");
        let text = RelayFrame::error(RelayErrorCode::UnsupportedVersion).encode().unwrap();
        assert!(text.contains(r#""code":"unsupported_version""#));
        assert!(!text.contains("retry_after_secs"));
    }

    #[test]
    fn unknown_version_is_a_version_error_even_with_unknown_fields() {
        let err = RelayFrame::decode(r#"{"type":"totally_new","version":7,"weird":true}"#).unwrap_err();
        assert!(matches!(err, CodecError::Version(_)), "{err:?}");
    }

    #[test]
    fn malformed_frames_are_malformed_errors() {
        assert!(matches!(RelayFrame::decode("not json").unwrap_err(), CodecError::Malformed(_)));
        assert!(matches!(RelayFrame::decode(r#"{"version":1,"type":"nope"}"#).unwrap_err(), CodecError::Malformed(_)));
        assert!(matches!(RelayFrame::decode(r#"{"version":1,"type":"join_by_code","code":"12"}"#).unwrap_err(), CodecError::Malformed(_)));
        assert!(matches!(
            RelayFrame::decode(r#"{"version":1,"type":"forward","session_id":"00000000-0000-0000-0000-000000000000","payload":"@@"}"#).unwrap_err(),
            CodecError::Malformed(_)
        ));
    }

    #[test]
    fn channel_validation() {
        assert!(validate_channel(&"0f".repeat(32)).is_ok());
        assert!(validate_channel(&"0F".repeat(32)).is_err(), "upper-case is not canonical");
        assert!(validate_channel(&"0f".repeat(31)).is_err());
        assert!(validate_channel(&"zz".repeat(32)).is_err());
    }

    #[test]
    fn limits_and_constants_are_sane() {
        const { assert!(DEFAULT_SESSION_TTL_SECS <= MAX_SESSION_TTL_SECS) };
        let json = serde_json::to_string(&RelayErrorCode::SessionAlreadyActive).unwrap();
        assert_eq!(json, r#""session_already_active""#);
    }
}
