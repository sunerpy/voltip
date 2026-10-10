//! Versioned wire protocol for Voltip.
//!
//! Three layers share this crate and nothing else:
//!
//! * [`relay`] — control frames the relay **can** read (JSON text frames).
//! * [`ticket`] — the one-time pairing ticket carried in a QR code.
//! * [`app`] — messages that travel **inside** the end-to-end encrypted channel; the relay
//!   only ever sees them as opaque `forward.payload` bytes.
//!
//! Every frame carries `version`. A receiver that sees a version it does not understand
//! must answer `error{unsupported_version}` and close; an unknown `type` inside a supported
//! version is ignored (forward compatibility, see `docs/protocol.md`).

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod app;
pub mod relay;
pub mod ticket;
mod version;

pub use version::{PROTOCOL_VERSION, ProtocolVersion, VersionError};

/// Errors produced while encoding or decoding protocol frames.
#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    /// The frame is not valid JSON / CBOR for the expected type.
    #[error("malformed frame: {0}")]
    Malformed(String),
    /// The frame declares a protocol version this build does not speak.
    #[error(transparent)]
    Version(#[from] VersionError),
    /// A field failed semantic validation (length, range, charset).
    #[error("invalid field `{field}`: {reason}")]
    InvalidField {
        /// Name of the offending field.
        field: &'static str,
        /// Human-readable reason.
        reason: String,
    },
}

impl From<serde_json::Error> for CodecError {
    fn from(value: serde_json::Error) -> Self {
        Self::Malformed(value.to_string())
    }
}

/// 6-digit pairing code as shown to the user (`483 921`).
#[derive(Clone, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(transparent)]
pub struct PairCode(String);

impl<'de> serde::Deserialize<'de> for PairCode {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        Self::new(&raw).map_err(serde::de::Error::custom)
    }
}

impl PairCode {
    /// Number of decimal digits in a pairing code.
    pub const DIGITS: usize = 6;

    /// Build a code from exactly six ASCII digits.
    pub fn new(digits: &str) -> Result<Self, CodecError> {
        if digits.len() != Self::DIGITS || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err(CodecError::InvalidField { field: "code", reason: format!("expected {} ASCII digits", Self::DIGITS) });
        }
        Ok(Self(digits.to_owned()))
    }

    /// Build a code from a number in `0..1_000_000`, zero-padded.
    pub fn from_u32(value: u32) -> Result<Self, CodecError> {
        if value >= 1_000_000 {
            return Err(CodecError::InvalidField { field: "code", reason: format!("{value} does not fit in {} digits", Self::DIGITS) });
        }
        Ok(Self(format!("{value:06}")))
    }

    /// Accept user input with spaces or dashes (`483 921`, `483-921`).
    pub fn parse_user_input(input: &str) -> Result<Self, CodecError> {
        let cleaned: String = input.chars().filter(|c| !matches!(c, ' ' | '-' | '\u{2009}' | '·')).collect();
        Self::new(&cleaned)
    }

    /// The raw six digits.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Display form with a thin gap in the middle: `483 921`.
    pub fn display_grouped(&self) -> String {
        format!("{} {}", &self.0[..3], &self.0[3..])
    }
}

impl std::fmt::Debug for PairCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Codes are short-lived secrets: keep them out of debug logs.
        f.write_str("PairCode(******)")
    }
}

impl std::fmt::Display for PairCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.display_grouped())
    }
}

/// Identifier of a pairing session, minted by the relay.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub uuid::Uuid);

impl SessionId {
    /// Mint a fresh random session id.
    pub fn random() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Stable identifier of a device (random UUID minted once at first launch).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct DeviceId(pub uuid::Uuid);

impl DeviceId {
    /// Mint a fresh random device id.
    pub fn random() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl std::fmt::Display for DeviceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Platform a device runs on; mirrors the design's device list column.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    /// Windows desktop.
    Windows,
    /// macOS desktop.
    Macos,
    /// Linux desktop (phase 2).
    Linux,
    /// Android phone.
    Android,
    /// iOS phone (phase 2).
    Ios,
    /// Anything else (web admin, tests).
    Other,
}

impl Platform {
    /// Detect the platform this binary was compiled for.
    pub fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(target_os = "android") {
            Self::Android
        } else if cfg!(target_os = "ios") {
            Self::Ios
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else {
            Self::Other
        }
    }

    /// Human-readable label used by the UI.
    pub fn label(self) -> &'static str {
        match self {
            Self::Windows => "Windows",
            Self::Macos => "macOS",
            Self::Linux => "Linux",
            Self::Android => "Android",
            Self::Ios => "iOS",
            Self::Other => "Other",
        }
    }
}

/// Public device description exchanged after the E2EE channel is up.
#[derive(Clone, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub struct DeviceInfo {
    /// Stable id.
    pub device_id: DeviceId,
    /// User-visible name (`Surface-Laptop`, `Pixel 10`).
    pub name: String,
    /// Platform.
    pub platform: Platform,
}

impl DeviceInfo {
    /// Maximum accepted length of a device name (chars).
    pub const MAX_NAME_CHARS: usize = 64;

    /// Validate name length; names are shown in UI lists and must stay bounded.
    pub fn validate(&self) -> Result<(), CodecError> {
        let n = self.name.chars().count();
        if n == 0 || n > Self::MAX_NAME_CHARS {
            return Err(CodecError::InvalidField { field: "name", reason: format!("expected 1..={} chars, got {n}", Self::MAX_NAME_CHARS) });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_code_accepts_six_digits_and_user_formats() {
        let c = PairCode::new("483921").expect("valid");
        assert_eq!(c.as_str(), "483921");
        assert_eq!(c.display_grouped(), "483 921");
        assert_eq!(c.to_string(), "483 921");
        assert_eq!(PairCode::parse_user_input("483 921").expect("spaced"), c);
        assert_eq!(PairCode::parse_user_input("483-921").expect("dashed"), c);
        assert_eq!(PairCode::from_u32(483_921).expect("num"), c);
        assert_eq!(PairCode::from_u32(7).expect("num").as_str(), "000007");
    }

    #[test]
    fn pair_code_rejects_bad_input() {
        assert!(PairCode::new("12345").is_err());
        assert!(PairCode::new("1234567").is_err());
        assert!(PairCode::new("12a456").is_err());
        assert!(PairCode::from_u32(1_000_000).is_err());
        assert!(PairCode::parse_user_input("48 39 2").is_err());
    }

    #[test]
    fn pair_code_debug_is_redacted() {
        let c = PairCode::new("483921").expect("valid");
        assert_eq!(format!("{c:?}"), "PairCode(******)");
    }

    #[test]
    fn ids_roundtrip_through_json_as_plain_strings() {
        let s = SessionId::random();
        let json = serde_json::to_string(&s).expect("json");
        assert!(json.starts_with('"'));
        let back: SessionId = serde_json::from_str(&json).expect("parse");
        assert_eq!(back, s);
        let d = DeviceId::random();
        assert_eq!(d.to_string(), d.0.to_string());
        assert_eq!(s.to_string(), s.0.to_string());
    }

    #[test]
    fn platform_labels_and_current() {
        for p in [Platform::Windows, Platform::Macos, Platform::Linux, Platform::Android, Platform::Ios, Platform::Other] {
            assert!(!p.label().is_empty());
            let json = serde_json::to_string(&p).expect("json");
            let back: Platform = serde_json::from_str(&json).expect("parse");
            assert_eq!(back, p);
        }
        // Whatever we compile on must map to a known variant.
        let _ = Platform::current().label();
    }

    #[test]
    fn device_info_validation() {
        let mut info = DeviceInfo { device_id: DeviceId::random(), name: "Pixel 10".into(), platform: Platform::Android };
        assert!(info.validate().is_ok());
        info.name.clear();
        assert!(info.validate().is_err());
        info.name = "x".repeat(65);
        assert!(matches!(info.validate(), Err(CodecError::InvalidField { field: "name", .. })));
    }

    #[test]
    fn codec_error_from_serde_json() {
        let err: CodecError = serde_json::from_str::<u8>("nope").unwrap_err().into();
        assert!(matches!(err, CodecError::Malformed(_)));
        assert!(err.to_string().starts_with("malformed frame"));
    }
}
