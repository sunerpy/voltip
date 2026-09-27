//! Long-term device identity keys (X25519, as required by Noise XX).

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::CryptoError;

/// Length of an X25519 key.
pub const PUBLIC_KEY_LEN: usize = 32;

/// Public half of a device identity. Safe to log, serialize and share.
///
/// Serializes as lower-case hex in human-readable formats (JSON, for the UI) and as raw bytes
/// in binary ones (CBOR, on the wire).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PublicKey(pub [u8; PUBLIC_KEY_LEN]);

impl Serialize for PublicKey {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if s.is_human_readable() { s.serialize_str(&self.to_hex()) } else { serde_bytes::Bytes::new(&self.0).serialize(s) }
    }
}

impl<'de> Deserialize<'de> for PublicKey {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        if d.is_human_readable() {
            let text = String::deserialize(d)?;
            Self::from_hex(&text).map_err(serde::de::Error::custom)
        } else {
            let bytes: serde_bytes::ByteBuf = serde_bytes::ByteBuf::deserialize(d)?;
            Self::from_slice(&bytes).map_err(serde::de::Error::custom)
        }
    }
}

impl PublicKey {
    /// Build from a slice; fails on wrong length.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, CryptoError> {
        let arr: [u8; PUBLIC_KEY_LEN] = bytes.try_into().map_err(|_| CryptoError::KeyLength(bytes.len()))?;
        Ok(Self(arr))
    }

    /// Raw bytes.
    pub fn as_bytes(&self) -> &[u8; PUBLIC_KEY_LEN] {
        &self.0
    }

    /// Lower-case hex, for logs and debug UIs.
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Parse lower/upper-case hex.
    pub fn from_hex(text: &str) -> Result<Self, CryptoError> {
        let bytes = hex::decode(text).map_err(|_| CryptoError::KeyLength(text.len()))?;
        Self::from_slice(&bytes)
    }
}

impl std::fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PublicKey({}…)", &self.to_hex()[..8])
    }
}

/// Secret half of a device identity. Zeroized on drop; `Debug` is redacted; **not**
/// `Serialize` — persistence goes through `voltip-identity::SecretStore` as raw bytes only.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SecretKey([u8; PUBLIC_KEY_LEN]);

impl SecretKey {
    /// Wrap raw bytes (e.g. read back from the platform secret store).
    pub fn from_slice(bytes: &[u8]) -> Result<Self, CryptoError> {
        let arr: [u8; PUBLIC_KEY_LEN] = bytes.try_into().map_err(|_| CryptoError::KeyLength(bytes.len()))?;
        Ok(Self(arr))
    }

    /// Expose the raw bytes for the secret store. Callers must zeroize their copy.
    pub fn expose(&self) -> &[u8; PUBLIC_KEY_LEN] {
        &self.0
    }
}

impl std::fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretKey(<redacted>)")
    }
}

impl PartialEq for SecretKey {
    fn eq(&self, other: &Self) -> bool {
        use subtle::ConstantTimeEq as _;
        self.0.ct_eq(&other.0).into()
    }
}

/// A device's long-term X25519 keypair.
#[derive(Clone, Debug)]
pub struct StaticKeypair {
    /// Public half.
    pub public: PublicKey,
    /// Secret half.
    pub secret: SecretKey,
}

impl StaticKeypair {
    /// Generate a fresh identity from the OS CSPRNG.
    pub fn generate() -> Result<Self, CryptoError> {
        let builder = snow::Builder::new(crate::handshake::PATTERN.parse()?);
        let kp = builder.generate_keypair()?;
        Ok(Self { public: PublicKey::from_slice(&kp.public)?, secret: SecretKey::from_slice(&kp.private)? })
    }

    /// Rebuild from a stored secret; recomputes the public key so the two can never drift.
    pub fn from_secret(secret: SecretKey) -> Result<Self, CryptoError> {
        // Derive the public key by running the secret through x25519 base-point multiplication
        // via snow's resolver-independent helper: build a keypair with a fixed private key.
        let public = x25519_public(secret.expose());
        Ok(Self { public: PublicKey(public), secret })
    }
}

/// X25519 base-point multiplication. Implemented with the same `x25519-dalek` that `snow`
/// uses internally, reached through snow's default resolver so there is exactly one X25519
/// implementation in the dependency tree.
fn x25519_public(secret: &[u8; PUBLIC_KEY_LEN]) -> [u8; PUBLIC_KEY_LEN] {
    use snow::resolvers::{CryptoResolver as _, DefaultResolver};
    let resolver = DefaultResolver;
    let mut dh = resolver.resolve_dh(&snow::params::DHChoice::Curve25519).unwrap_or_else(|| unreachable!("default resolver always provides Curve25519"));
    dh.set(secret);
    let mut out = [0u8; PUBLIC_KEY_LEN];
    out.copy_from_slice(dh.pubkey());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_then_rebuild_from_secret_yields_same_public_key() {
        let kp = StaticKeypair::generate().unwrap();
        let rebuilt = StaticKeypair::from_secret(kp.secret.clone()).unwrap();
        assert_eq!(rebuilt.public, kp.public);
        assert_eq!(rebuilt.secret, kp.secret);
    }

    #[test]
    fn two_identities_differ() {
        let a = StaticKeypair::generate().unwrap();
        let b = StaticKeypair::generate().unwrap();
        assert_ne!(a.public, b.public);
        assert_ne!(a.secret, b.secret);
    }

    #[test]
    fn public_key_hex_roundtrip_and_debug() {
        let kp = StaticKeypair::generate().unwrap();
        let hex = kp.public.to_hex();
        assert_eq!(hex.len(), 64);
        assert_eq!(PublicKey::from_hex(&hex).unwrap(), kp.public);
        assert_eq!(PublicKey::from_hex(&hex.to_uppercase()).unwrap(), kp.public);
        assert!(format!("{:?}", kp.public).starts_with("PublicKey("));
        assert!(PublicKey::from_hex("zz").is_err());
        assert!(PublicKey::from_hex("abcd").is_err());
        assert!(PublicKey::from_slice(&[1, 2, 3]).is_err());
        assert_eq!(kp.public.as_bytes().len(), PUBLIC_KEY_LEN);
    }

    #[test]
    fn secret_key_is_redacted_and_length_checked() {
        let kp = StaticKeypair::generate().unwrap();
        assert_eq!(format!("{:?}", kp.secret), "SecretKey(<redacted>)");
        assert!(format!("{kp:?}").contains("<redacted>"));
        assert!(SecretKey::from_slice(&[0; 31]).is_err());
        assert_eq!(kp.secret.expose().len(), PUBLIC_KEY_LEN);
    }

    #[test]
    fn public_key_is_bytes_in_cbor_and_hex_in_json() {
        let kp = StaticKeypair::generate().unwrap();
        let mut cbor = Vec::new();
        ciborium::into_writer(&kp.public, &mut cbor).unwrap();
        // 32 bytes + 2-byte CBOR byte-string header, not a 32-element integer array.
        assert_eq!(cbor.len(), 34);
        let back: PublicKey = ciborium::from_reader(cbor.as_slice()).unwrap();
        assert_eq!(back, kp.public);
        let json = serde_json::to_string(&kp.public).unwrap();
        assert_eq!(json, format!("\"{}\"", kp.public.to_hex()));
        let back: PublicKey = serde_json::from_str(&json).unwrap();
        assert_eq!(back, kp.public);
        assert!(serde_json::from_str::<PublicKey>("\"zz\"").is_err());
    }
}
