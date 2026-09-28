//! Messages that travel **inside** the end-to-end encrypted channel.
//!
//! Encoded as CBOR (compact, binary-safe). The relay never sees these; it only forwards the
//! ciphertext that wraps them.

use serde::{Deserialize, Serialize};

use crate::{CodecError, DeviceInfo, ProtocolVersion, validate_direct_hints};

/// Why a peer rejected pairing.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectReason {
    /// The user compared safety codes and they differed (or they simply declined).
    UserDeclined,
    /// The peer's identity did not match the trusted record.
    IdentityMismatch,
    /// The peer took too long.
    Timeout,
}

/// The one sample rate a phone take streams at: PCM16 little-endian mono, 16 kHz.
pub const TAKE_SAMPLE_RATE_HZ: u32 = 16_000;
/// Largest PCM payload of one [`AppMessage::TakeAudio`]: one second of audio.
pub const MAX_TAKE_AUDIO_BYTES: usize = 32_000;
/// Longest text a [`TakeState::Done`] carries back (characters); longer text is cut with `…`.
pub const MAX_TAKE_TEXT_CHARS: usize = 2_000;
/// Longest failure explanation a [`TakeState::Failed`] carries (characters).
pub const MAX_TAKE_MESSAGE_CHARS: usize = 200;
/// Length of one Opus packet of a phone take ([`AppMessage::TakeOpus`]): 20 ms, 320 samples at
/// [`TAKE_SAMPLE_RATE_HZ`].
pub const TAKE_OPUS_FRAME_SAMPLES: usize = 320;
/// Most Opus packets one [`AppMessage::TakeOpus`] carries: one second of audio.
pub const MAX_TAKE_OPUS_PACKETS: usize = 50;
/// Largest Opus packet (RFC 6716 §3.2.1).
pub const MAX_OPUS_PACKET_BYTES: usize = 1275;
/// Longest text a phone sends for the desktop to insert ([`AppMessage::PhoneText`], characters).
pub const MAX_PHONE_TEXT_CHARS: usize = 10_000;

/// Where a phone's text came from (docs/dictation.md §20.6).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhoneTextSource {
    /// Typed (or pasted) into the phone's text box.
    Typed,
    /// The phone's clipboard, sent as it was.
    Clipboard,
}

/// Why a phone's text was not delivered (desktop → phone).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhoneTextFailure {
    /// Too many texts are already waiting for the desktop's take to end.
    Busy,
    /// This device does not insert texts from a phone.
    Unavailable,
    /// The insertion failed (see the message).
    Failed,
}

/// What became of a phone's text on the desktop ([`AppMessage::PhoneTextStatus`]).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PhoneTextState {
    /// Waiting: the desktop is in the middle of a take, the text goes in once it is over.
    Queued,
    /// Inserted at the cursor (`pasted`) or left on the desktop's clipboard.
    Delivered {
        /// Pasted rather than left on the clipboard.
        pasted: bool,
    },
    /// Not delivered.
    Failed {
        /// Why.
        code: PhoneTextFailure,
        /// The desktop's explanation (at most [`MAX_TAKE_MESSAGE_CHARS`]).
        message: String,
    },
}

impl PhoneTextState {
    /// `Failed`, with the message cut to [`MAX_TAKE_MESSAGE_CHARS`].
    pub fn failed(code: PhoneTextFailure, message: &str) -> Self {
        Self::Failed { code, message: clip(message, MAX_TAKE_MESSAGE_CHARS) }
    }

    /// No further status follows.
    pub fn is_final(&self) -> bool {
        !matches!(self, Self::Queued)
    }
}

/// Why a phone's take did not deliver (desktop → phone).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TakeFailure {
    /// The desktop is running another take.
    Busy,
    /// The desktop has no recognition configured, or does not take phone audio.
    Unavailable,
    /// Nothing was heard.
    NoSpeech,
    /// Recognition, clean-up or delivery failed (see the message).
    Failed,
}

/// Where a phone's take is on the desktop ([`AppMessage::TakeStatus`]).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum TakeState {
    /// The desktop is recording what the phone streams.
    Listening,
    /// The phone stopped; the desktop is recognising, cleaning up, inserting.
    Processing,
    /// Delivered into the desktop's front application (`pasted`) or left on its clipboard.
    Done {
        /// What was delivered (at most [`MAX_TAKE_TEXT_CHARS`]).
        text: String,
        /// Pasted at the cursor rather than left on the clipboard.
        pasted: bool,
    },
    /// Did not deliver.
    Failed {
        /// Why.
        code: TakeFailure,
        /// The desktop's explanation (at most [`MAX_TAKE_MESSAGE_CHARS`]).
        message: String,
    },
    /// Discarded (by either side).
    Cancelled,
}

impl TakeState {
    /// `Done`, with the text cut to [`MAX_TAKE_TEXT_CHARS`].
    pub fn done(text: &str, pasted: bool) -> Self {
        Self::Done { text: clip(text, MAX_TAKE_TEXT_CHARS), pasted }
    }

    /// `Failed`, with the message cut to [`MAX_TAKE_MESSAGE_CHARS`].
    pub fn failed(code: TakeFailure, message: &str) -> Self {
        Self::Failed { code, message: clip(message, MAX_TAKE_MESSAGE_CHARS) }
    }

    /// The take is over (no further status follows).
    pub fn is_final(&self) -> bool {
        matches!(self, Self::Done { .. } | Self::Failed { .. } | Self::Cancelled)
    }
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Application-level message.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AppMessage {
    /// Sent by each side once its user confirmed the safety code.
    PairConfirm {
        /// Protocol version.
        version: ProtocolVersion,
        /// Who is confirming.
        device: DeviceInfo,
    },
    /// Sent by a side that rejected pairing; the channel is torn down afterwards.
    PairReject {
        /// Protocol version.
        version: ProtocolVersion,
        /// Why.
        reason: RejectReason,
    },
    /// Liveness probe.
    Ping {
        /// Protocol version.
        version: ProtocolVersion,
        /// Echoed back in `pong`.
        seq: u64,
    },
    /// Liveness reply.
    Pong {
        /// Protocol version.
        version: ProtocolVersion,
        /// Sequence from the ping.
        seq: u64,
    },
    /// Free-form UTF-8 text (the first "real" payload: dictation results, clipboard, ...).
    Text {
        /// Protocol version.
        version: ProtocolVersion,
        /// Body.
        body: String,
    },
    /// Announce updated device info (rename) and current LAN endpoints to a trusted peer.
    DeviceInfoUpdate {
        /// Protocol version.
        version: ProtocolVersion,
        /// New info.
        device: DeviceInfo,
        /// `ip:port` endpoints where this device's LAN host listens right now (may be empty).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        direct_hints: Vec<String>,
    },
    /// Phone → desktop: start a dictation take whose audio this phone streams (the phone is the
    /// microphone; recognition, clean-up and delivery run on the desktop).
    TakeStart {
        /// Protocol version.
        version: ProtocolVersion,
        /// The phone's id for this take; every other take message names it.
        take: u32,
        /// Always [`TAKE_SAMPLE_RATE_HZ`].
        sample_rate_hz: u32,
    },
    /// Phone → desktop: the next chunk of the take's audio, PCM16 little-endian mono.
    TakeAudio {
        /// Protocol version.
        version: ProtocolVersion,
        /// The take.
        take: u32,
        /// Chunk counter from `0`; a duplicate or an older chunk is dropped.
        seq: u32,
        /// Samples (at most [`MAX_TAKE_AUDIO_BYTES`], an even number of bytes).
        #[serde(with = "serde_bytes")]
        pcm: Vec<u8>,
    },
    /// Phone → desktop: the next chunk of the take's audio as 20 ms Opus packets (16 kHz mono),
    /// sent once the desktop's [`AppMessage::TakeStatus`] said it decodes them (`opus`); it shares
    /// the `seq` counter with [`AppMessage::TakeAudio`].
    TakeOpus {
        /// Protocol version.
        version: ProtocolVersion,
        /// The take.
        take: u32,
        /// Chunk counter, continuing the take's [`AppMessage::TakeAudio`] ones.
        seq: u32,
        /// 1..=[`MAX_TAKE_OPUS_PACKETS`] packets of [`TAKE_OPUS_FRAME_SAMPLES`] samples each, every one
        /// 1..=[`MAX_OPUS_PACKET_BYTES`] bytes.
        packets: Vec<serde_bytes::ByteBuf>,
    },
    /// Phone → desktop: the speaker let go; recognise and deliver what was streamed.
    TakeStop {
        /// Protocol version.
        version: ProtocolVersion,
        /// The take.
        take: u32,
    },
    /// Phone → desktop: discard the take.
    TakeCancel {
        /// Protocol version.
        version: ProtocolVersion,
        /// The take.
        take: u32,
    },
    /// Desktop → phone: where the take is.
    TakeStatus {
        /// Protocol version.
        version: ProtocolVersion,
        /// The take.
        take: u32,
        /// Its state.
        state: TakeState,
        /// The desktop decodes [`AppMessage::TakeOpus`]: the phone may switch from PCM for the rest
        /// of the take. Absent from desktops that only take PCM, which a phone then keeps sending.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        opus: bool,
    },
    /// Phone → desktop: insert this text at the cursor, like a dictation's result (docs/dictation.md
    /// §20.6). Nothing is recognised or cleaned up; the desktop answers with
    /// [`AppMessage::PhoneTextStatus`].
    PhoneText {
        /// Protocol version.
        version: ProtocolVersion,
        /// The phone's id for this text; the status names it.
        id: u32,
        /// 1..=[`MAX_PHONE_TEXT_CHARS`] characters.
        body: String,
        /// Typed or the clipboard.
        source: PhoneTextSource,
    },
    /// Desktop → phone: what became of a [`AppMessage::PhoneText`].
    PhoneTextStatus {
        /// Protocol version.
        version: ProtocolVersion,
        /// The text.
        id: u32,
        /// Its state.
        state: PhoneTextState,
    },
    /// Sent right before the sender forgets this peer (docs/pairing.md): the receiver forgets the
    /// sender too, so neither side keeps a record the other one no longer honours.
    Unpair {
        /// Protocol version.
        version: ProtocolVersion,
    },
}

impl AppMessage {
    /// Largest accepted plaintext, to bound memory on the receiving side.
    pub const MAX_ENCODED_BYTES: usize = 64 * 1024;

    /// The version carried by this message.
    pub fn version(&self) -> ProtocolVersion {
        match self {
            Self::PairConfirm { version, .. }
            | Self::PairReject { version, .. }
            | Self::Ping { version, .. }
            | Self::Pong { version, .. }
            | Self::Text { version, .. }
            | Self::DeviceInfoUpdate { version, .. }
            | Self::TakeStart { version, .. }
            | Self::TakeAudio { version, .. }
            | Self::TakeOpus { version, .. }
            | Self::TakeStop { version, .. }
            | Self::TakeCancel { version, .. }
            | Self::TakeStatus { version, .. }
            | Self::PhoneText { version, .. }
            | Self::PhoneTextStatus { version, .. }
            | Self::Unpair { version } => *version,
        }
    }

    /// CBOR-encode.
    pub fn encode(&self) -> Result<Vec<u8>, CodecError> {
        let mut out = Vec::new();
        ciborium::into_writer(self, &mut out).map_err(|e| CodecError::Malformed(e.to_string()))?;
        if out.len() > Self::MAX_ENCODED_BYTES {
            return Err(CodecError::InvalidField { field: "message", reason: format!("{} bytes exceeds {}", out.len(), Self::MAX_ENCODED_BYTES) });
        }
        Ok(out)
    }

    /// CBOR-decode and enforce the version.
    pub fn decode(bytes: &[u8]) -> Result<Self, CodecError> {
        if bytes.len() > Self::MAX_ENCODED_BYTES {
            return Err(CodecError::InvalidField { field: "message", reason: "too large".into() });
        }
        let msg: Self = ciborium::from_reader(bytes).map_err(|e| CodecError::Malformed(e.to_string()))?;
        msg.version().check()?;
        match &msg {
            Self::PairConfirm { device, .. } => device.validate()?,
            Self::DeviceInfoUpdate { device, direct_hints, .. } => {
                device.validate()?;
                validate_direct_hints(direct_hints)?;
            }
            Self::TakeStart { sample_rate_hz, .. } if *sample_rate_hz != TAKE_SAMPLE_RATE_HZ => {
                return Err(CodecError::InvalidField { field: "sample_rate_hz", reason: format!("{sample_rate_hz} Hz, only {TAKE_SAMPLE_RATE_HZ}") });
            }
            Self::TakeAudio { pcm, .. } if pcm.len() > MAX_TAKE_AUDIO_BYTES || !pcm.len().is_multiple_of(2) => {
                return Err(CodecError::InvalidField { field: "pcm", reason: format!("{} bytes (even, at most {MAX_TAKE_AUDIO_BYTES})", pcm.len()) });
            }
            Self::TakeOpus { packets, .. }
                if packets.is_empty() || packets.len() > MAX_TAKE_OPUS_PACKETS || packets.iter().any(|p| p.is_empty() || p.len() > MAX_OPUS_PACKET_BYTES) =>
            {
                return Err(CodecError::InvalidField {
                    field: "packets",
                    reason: format!("{} packets (1..={MAX_TAKE_OPUS_PACKETS}, each 1..={MAX_OPUS_PACKET_BYTES} bytes)", packets.len()),
                });
            }
            Self::TakeStatus { state: TakeState::Done { text, .. }, .. } if text.chars().count() > MAX_TAKE_TEXT_CHARS => {
                return Err(CodecError::InvalidField { field: "text", reason: format!("more than {MAX_TAKE_TEXT_CHARS} characters") });
            }
            Self::TakeStatus { state: TakeState::Failed { message, .. }, .. } if message.chars().count() > MAX_TAKE_MESSAGE_CHARS => {
                return Err(CodecError::InvalidField { field: "message", reason: format!("more than {MAX_TAKE_MESSAGE_CHARS} characters") });
            }
            Self::PhoneText { body, .. } if body.is_empty() || body.chars().count() > MAX_PHONE_TEXT_CHARS => {
                return Err(CodecError::InvalidField { field: "body", reason: format!("expected 1..={MAX_PHONE_TEXT_CHARS} characters") });
            }
            Self::PhoneTextStatus { state: PhoneTextState::Failed { message, .. }, .. } if message.chars().count() > MAX_TAKE_MESSAGE_CHARS => {
                return Err(CodecError::InvalidField { field: "message", reason: format!("more than {MAX_TAKE_MESSAGE_CHARS} characters") });
            }
            _ => {}
        }
        Ok(msg)
    }

    /// Convenience: ping.
    pub fn ping(seq: u64) -> Self {
        Self::Ping { version: ProtocolVersion::CURRENT, seq }
    }

    /// Convenience: pong.
    pub fn pong(seq: u64) -> Self {
        Self::Pong { version: ProtocolVersion::CURRENT, seq }
    }

    /// Convenience: text.
    pub fn text(body: impl Into<String>) -> Self {
        Self::Text { version: ProtocolVersion::CURRENT, body: body.into() }
    }

    /// Convenience: a take's status from this build's desktop, which decodes Opus.
    pub fn take_status(take: u32, state: TakeState) -> Self {
        Self::TakeStatus { version: ProtocolVersion::CURRENT, take, state, opus: true }
    }

    /// Convenience: a phone text's status.
    pub fn phone_text_status(id: u32, state: PhoneTextState) -> Self {
        Self::PhoneTextStatus { version: ProtocolVersion::CURRENT, id, state }
    }

    /// Convenience: unpair.
    pub fn unpair() -> Self {
        Self::Unpair { version: ProtocolVersion::CURRENT }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DeviceId, Platform};

    fn device() -> DeviceInfo {
        DeviceInfo { device_id: DeviceId::random(), name: "Surface-Laptop".into(), platform: Platform::Windows }
    }

    #[test]
    fn messages_roundtrip() {
        let v = ProtocolVersion::CURRENT;
        for m in [
            AppMessage::PairConfirm { version: v, device: device() },
            AppMessage::PairReject { version: v, reason: RejectReason::UserDeclined },
            AppMessage::ping(1),
            AppMessage::pong(1),
            AppMessage::text("把 fetchUser 改成 async"),
            AppMessage::DeviceInfoUpdate { version: v, device: device(), direct_hints: vec!["192.168.1.24:47831".into()] },
            AppMessage::DeviceInfoUpdate { version: v, device: device(), direct_hints: Vec::new() },
            AppMessage::unpair(),
        ] {
            let bytes = m.encode().unwrap();
            assert_eq!(AppMessage::decode(&bytes).unwrap(), m);
            assert_eq!(m.version(), v);
        }
    }

    /// The phone-microphone messages (docs/dictation.md §20): they round-trip, the PCM travels as
    /// a CBOR byte string (not an array of numbers), and malformed takes are refused on decode.
    #[test]
    fn take_messages_roundtrip_and_are_validated() {
        let v = ProtocolVersion::CURRENT;
        let pcm: Vec<u8> = (0..3200u32).map(|i| (i % 251) as u8).collect();
        for m in [
            AppMessage::TakeStart { version: v, take: 7, sample_rate_hz: TAKE_SAMPLE_RATE_HZ },
            AppMessage::TakeAudio { version: v, take: 7, seq: 0, pcm: pcm.clone() },
            AppMessage::TakeOpus { version: v, take: 7, seq: 1, packets: vec![serde_bytes::ByteBuf::from(vec![0x08, 1, 2]); 5] },
            AppMessage::TakeStop { version: v, take: 7 },
            AppMessage::TakeCancel { version: v, take: 7 },
            AppMessage::take_status(7, TakeState::Listening),
            AppMessage::take_status(7, TakeState::Processing),
            AppMessage::take_status(7, TakeState::done("把 fetchUser 改成 async", true)),
            AppMessage::take_status(7, TakeState::failed(TakeFailure::Busy, "正在听写")),
            AppMessage::take_status(7, TakeState::Cancelled),
        ] {
            let bytes = m.encode().unwrap();
            assert_eq!(AppMessage::decode(&bytes).unwrap(), m);
        }
        // 3200 bytes of PCM cost 3200 bytes plus a few of framing, not ~2x as a number array would.
        let audio = AppMessage::TakeAudio { version: v, take: 7, seq: 0, pcm }.encode().unwrap();
        assert!(audio.len() < 3200 + 64, "{} bytes", audio.len());
        let bad = [
            (AppMessage::TakeStart { version: v, take: 1, sample_rate_hz: 48_000 }, "sample_rate_hz"),
            (AppMessage::TakeAudio { version: v, take: 1, seq: 0, pcm: vec![0; 3] }, "pcm"),
            (AppMessage::TakeAudio { version: v, take: 1, seq: 0, pcm: vec![0; MAX_TAKE_AUDIO_BYTES + 2] }, "pcm"),
            (AppMessage::TakeOpus { version: v, take: 1, seq: 0, packets: Vec::new() }, "packets"),
            (AppMessage::TakeOpus { version: v, take: 1, seq: 0, packets: vec![serde_bytes::ByteBuf::from(vec![8]); MAX_TAKE_OPUS_PACKETS + 1] }, "packets"),
            (AppMessage::TakeOpus { version: v, take: 1, seq: 0, packets: vec![serde_bytes::ByteBuf::new()] }, "packets"),
            (AppMessage::TakeOpus { version: v, take: 1, seq: 0, packets: vec![serde_bytes::ByteBuf::from(vec![8; MAX_OPUS_PACKET_BYTES + 1])] }, "packets"),
            (AppMessage::take_status(1, TakeState::Done { text: "x".repeat(MAX_TAKE_TEXT_CHARS + 1), pasted: true }), "text"),
            (AppMessage::take_status(1, TakeState::Failed { code: TakeFailure::Failed, message: "x".repeat(MAX_TAKE_MESSAGE_CHARS + 1) }), "message"),
        ];
        for (m, field) in bad {
            let bytes = m.encode().unwrap();
            assert!(matches!(AppMessage::decode(&bytes).unwrap_err(), CodecError::InvalidField { field: f, .. } if f == field), "{field}");
        }
        // The constructors clip instead of producing an undecodable status.
        let TakeState::Done { text, .. } = TakeState::done(&"字".repeat(MAX_TAKE_TEXT_CHARS + 5), false) else { unreachable!() };
        assert_eq!(text.chars().count(), MAX_TAKE_TEXT_CHARS);
        assert!(text.ends_with('…'));
        assert!(TakeState::Cancelled.is_final() && !TakeState::Listening.is_final() && !TakeState::Processing.is_final());
        // docs/dictation.md §20.1: this build's desktop says it decodes Opus; a status without the
        // flag (a desktop that only takes PCM) reads as `false`, so the phone keeps sending PCM.
        assert!(matches!(AppMessage::take_status(1, TakeState::Listening), AppMessage::TakeStatus { opus: true, .. }));
        let pcm_only = AppMessage::TakeStatus { version: v, take: 1, state: TakeState::Listening, opus: false }.encode().unwrap();
        assert!(matches!(AppMessage::decode(&pcm_only).unwrap(), AppMessage::TakeStatus { opus: false, .. }));
        let with_flag = AppMessage::take_status(1, TakeState::Listening).encode().unwrap();
        assert!(with_flag.len() > pcm_only.len(), "the flag is left out when false");
        assert_eq!(serde_json::to_string(&TakeFailure::NoSpeech).unwrap(), r#""no_speech""#);
    }

    /// docs/dictation.md §20.6: a phone's text and its status round-trip; an empty or oversized
    /// text and an oversized failure message are refused on decode.
    #[test]
    fn phone_text_messages_roundtrip_and_are_validated() {
        let v = ProtocolVersion::CURRENT;
        for m in [
            AppMessage::PhoneText { version: v, id: 1, body: "把 fetchUser 改成 async".into(), source: PhoneTextSource::Typed },
            AppMessage::PhoneText { version: v, id: 2, body: "字".repeat(MAX_PHONE_TEXT_CHARS), source: PhoneTextSource::Clipboard },
            AppMessage::phone_text_status(1, PhoneTextState::Queued),
            AppMessage::phone_text_status(1, PhoneTextState::Delivered { pasted: true }),
            AppMessage::phone_text_status(2, PhoneTextState::failed(PhoneTextFailure::Busy, "排队的文字太多")),
        ] {
            let bytes = m.encode().unwrap();
            assert_eq!(AppMessage::decode(&bytes).unwrap(), m);
        }
        for (m, field) in [
            (AppMessage::PhoneText { version: v, id: 1, body: String::new(), source: PhoneTextSource::Typed }, "body"),
            (AppMessage::PhoneText { version: v, id: 1, body: "x".repeat(MAX_PHONE_TEXT_CHARS + 1), source: PhoneTextSource::Typed }, "body"),
            (
                AppMessage::PhoneTextStatus {
                    version: v,
                    id: 1,
                    state: PhoneTextState::Failed { code: PhoneTextFailure::Failed, message: "x".repeat(MAX_TAKE_MESSAGE_CHARS + 1) },
                },
                "message",
            ),
        ] {
            let bytes = m.encode().unwrap();
            assert!(matches!(AppMessage::decode(&bytes).unwrap_err(), CodecError::InvalidField { field: f, .. } if f == field), "{field}");
        }
        assert!(!PhoneTextState::Queued.is_final() && PhoneTextState::Delivered { pasted: false }.is_final());
        assert_eq!(serde_json::to_string(&PhoneTextSource::Clipboard).unwrap(), r#""clipboard""#);
    }

    #[test]
    fn version_is_enforced_on_decode() {
        let m = AppMessage::Ping { version: ProtocolVersion(3), seq: 1 };
        let bytes = m.encode().unwrap();
        assert!(matches!(AppMessage::decode(&bytes).unwrap_err(), CodecError::Version(_)));
    }

    #[test]
    fn device_info_is_validated_on_decode() {
        let mut d = device();
        d.name = "x".repeat(65);
        let bytes = AppMessage::PairConfirm { version: ProtocolVersion::CURRENT, device: d }.encode().unwrap();
        assert!(matches!(AppMessage::decode(&bytes).unwrap_err(), CodecError::InvalidField { field: "name", .. }));
    }

    #[test]
    fn device_info_update_hints_are_validated_on_decode() {
        let bytes = AppMessage::DeviceInfoUpdate { version: ProtocolVersion::CURRENT, device: device(), direct_hints: vec!["relay.example.org:1".into()] }
            .encode()
            .unwrap();
        assert!(matches!(AppMessage::decode(&bytes).unwrap_err(), CodecError::InvalidField { field: "direct_hints", .. }));
    }

    #[test]
    fn size_limits_apply_both_ways() {
        let big = AppMessage::text("x".repeat(AppMessage::MAX_ENCODED_BYTES));
        assert!(matches!(big.encode().unwrap_err(), CodecError::InvalidField { field: "message", .. }));
        let too_many = vec![0u8; AppMessage::MAX_ENCODED_BYTES + 1];
        assert!(matches!(AppMessage::decode(&too_many).unwrap_err(), CodecError::InvalidField { field: "message", .. }));
        assert!(matches!(AppMessage::decode(&[0xff, 0x00]).unwrap_err(), CodecError::Malformed(_)));
    }

    #[test]
    fn reject_reasons_serialize_snake_case() {
        let json = serde_json::to_string(&RejectReason::IdentityMismatch).unwrap();
        assert_eq!(json, r#""identity_mismatch""#);
        let json = serde_json::to_string(&RejectReason::Timeout).unwrap();
        assert_eq!(json, r#""timeout""#);
    }
}
