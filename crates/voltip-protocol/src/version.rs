//! Protocol version negotiation.

/// The single protocol version this build speaks.
pub const PROTOCOL_VERSION: u16 = 1;

/// A protocol version number as carried on the wire.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct ProtocolVersion(pub u16);

impl ProtocolVersion {
    /// The version this build speaks.
    pub const CURRENT: Self = Self(PROTOCOL_VERSION);

    /// Reject anything but the current version. Kept as a method so a future build that
    /// speaks several versions changes one function, not every decoder.
    pub fn check(self) -> Result<(), VersionError> {
        if self == Self::CURRENT { Ok(()) } else { Err(VersionError::Unsupported { got: self.0, supported: PROTOCOL_VERSION }) }
    }
}

impl Default for ProtocolVersion {
    fn default() -> Self {
        Self::CURRENT
    }
}

/// Version mismatch.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VersionError {
    /// The peer speaks a version we do not.
    #[error("unsupported protocol version {got} (this build speaks {supported})")]
    Unsupported {
        /// Version seen on the wire.
        got: u16,
        /// Version this build speaks.
        supported: u16,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_version_passes_and_others_fail() {
        assert!(ProtocolVersion::CURRENT.check().is_ok());
        assert_eq!(ProtocolVersion::default(), ProtocolVersion::CURRENT);
        let err = ProtocolVersion(99).check().unwrap_err();
        assert_eq!(err, VersionError::Unsupported { got: 99, supported: PROTOCOL_VERSION });
        assert!(err.to_string().contains("99"));
    }
}
