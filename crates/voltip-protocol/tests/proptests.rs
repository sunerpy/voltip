#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Property tests: every value the encoders accept must round-trip, and decoders must never
//! panic on arbitrary input.

use proptest::prelude::*;
use voltip_protocol::app::AppMessage;
use voltip_protocol::relay::RelayFrame;
use voltip_protocol::ticket::{NONCE_LEN, PUBLIC_KEY_LEN, PairingTicket};
use voltip_protocol::{PairCode, ProtocolVersion, SessionId};

proptest! {
    #[test]
    fn pair_codes_roundtrip(n in 0u32..1_000_000) {
        let code = PairCode::from_u32(n).unwrap();
        prop_assert_eq!(PairCode::parse_user_input(&code.display_grouped()).unwrap(), code.clone());
        let json = serde_json::to_string(&code).unwrap();
        prop_assert_eq!(serde_json::from_str::<PairCode>(&json).unwrap(), code);
    }

    #[test]
    fn tickets_roundtrip(e in proptest::array::uniform32(any::<u8>()), nonce in proptest::array::uniform16(any::<u8>()), exp in any::<u64>()) {
        let _: [u8; PUBLIC_KEY_LEN] = e;
        let _: [u8; NONCE_LEN] = nonce;
        let t = PairingTicket { version: ProtocolVersion::CURRENT, session_id: SessionId::random(), ephemeral_pub: e, nonce, expires_at: exp, relay_hint: None };
        let uri = t.to_uri().unwrap();
        prop_assert_eq!(PairingTicket::from_uri(&uri).unwrap(), t);
    }

    #[test]
    fn text_messages_roundtrip(body in "\\PC{0,200}") {
        let m = AppMessage::text(body);
        let bytes = m.encode().unwrap();
        prop_assert_eq!(AppMessage::decode(&bytes).unwrap(), m);
    }

    #[test]
    fn decoders_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..256), text in "\\PC{0,128}") {
        let _ = AppMessage::decode(&bytes);
        let _ = RelayFrame::decode(&text);
        let _ = PairingTicket::from_uri(&text);
        let _ = PairCode::parse_user_input(&text);
    }
}
