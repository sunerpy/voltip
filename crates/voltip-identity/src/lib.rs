//! Device identity and trust.
//!
//! * [`SecretStore`] — where the long-term private key lives. Production builds use the
//!   platform store ([`KeyringSecretStore`], feature `keyring`); Android supplies its own
//!   Keystore-backed implementation from the mobile app; tests use [`MemorySecretStore`].
//! * [`DeviceIdentity`] — `device_id` + name + platform + X25519 keypair. Its `Serialize`
//!   output ([`DeviceIdentityPublic`]) never contains the secret.
//! * [`TrustedDeviceStore`] — the registry of paired peers keyed by their static public key.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod identity;
mod secret_store;
mod trusted;

pub use identity::{DeviceIdentity, DeviceIdentityPublic, IdentityManager};
#[cfg(feature = "android-keystore")]
pub use secret_store::AndroidKeystoreSecretStore;
#[cfg(feature = "keyring")]
pub use secret_store::{KeyringSecretStore, SIGNED_ACCOUNT_SUFFIX};
pub use secret_store::{MemorySecretStore, SECRET_KEY_ENTRY, SecretStore};
pub use trusted::{ConnectionKind, IdentityCheck, TrustedDevice, TrustedDeviceStore, TrustedDevicesFile};

/// Errors from the identity layer.
#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    /// The secret store could not be reached (locked keychain, missing daemon, timeout).
    #[error("secret store unavailable: {0}")]
    StoreUnavailable(String),
    /// Stored bytes were not a valid key.
    #[error("stored identity is corrupt")]
    Corrupt,
    /// Cryptographic failure while (re)building the keypair.
    #[error(transparent)]
    Crypto(#[from] voltip_crypto::CryptoError),
    /// Trusted-device file could not be read or written.
    #[error("trusted device store i/o: {0}")]
    Io(#[from] std::io::Error),
    /// Trusted-device file is not valid JSON for the current schema.
    #[error("trusted device store is corrupt: {0}")]
    Serde(#[from] serde_json::Error),
    /// Validation failure (device name too long, etc.).
    #[error(transparent)]
    Protocol(#[from] voltip_protocol::CodecError),
}
