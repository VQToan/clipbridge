use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chacha20poly1305::aead::rand_core::RngCore;
use chacha20poly1305::aead::OsRng;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::error::{CoreError, Result};

/// Context strings specified in `protocol.md` section 1.
const CONTEXT_GROUP_ID: &str = "clipbridge 2026-11 group-id v1";
const CONTEXT_RELAY_AUTH: &str = "clipbridge 2026-11 relay-auth v1";
const CONTEXT_ENVELOPE_ENC: &str = "clipbridge 2026-11 envelope-enc v1";

/// Length of the root symmetric group key in bytes.
pub const GROUP_KEY_LEN: usize = 32;

/// A 32-byte shared symmetric key root for a paired group of devices.
///
/// Derives sub-keys for group identification, relay authentication, and envelope encryption
/// using BLAKE3 in key-derivation mode (`derive_key`), as defined in `docs/protocol.md`.
/// Memory is zeroized on drop. `Debug` redaction prevents accidental logging of key material.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct GroupKey([u8; GROUP_KEY_LEN]);

impl GroupKey {
    /// Generates a new cryptographically secure random `GroupKey` using OS entropy.
    pub fn generate() -> Self {
        let mut bytes = [0u8; GROUP_KEY_LEN];
        OsRng.fill_bytes(&mut bytes);
        Self(bytes)
    }

    /// Constructs a `GroupKey` directly from raw 32 bytes.
    pub fn from_bytes(bytes: [u8; GROUP_KEY_LEN]) -> Self {
        Self(bytes)
    }

    /// Decodes a `GroupKey` from a URL-safe unpadded base64 string.
    pub fn from_base64url(s: &str) -> Result<Self> {
        let bytes = URL_SAFE_NO_PAD
            .decode(s.as_bytes())
            .map_err(|e| CoreError::Crypto(format!("invalid base64url group key: {e}")))?;
        if bytes.len() != GROUP_KEY_LEN {
            return Err(CoreError::Crypto(format!(
                "invalid group key length: expected {GROUP_KEY_LEN} bytes, got {}",
                bytes.len()
            )));
        }
        let mut key = [0u8; GROUP_KEY_LEN];
        key.copy_from_slice(&bytes);
        Ok(Self(key))
    }

    /// Encodes the `GroupKey` as a URL-safe unpadded base64 string (43 characters).
    pub fn to_base64url(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.0)
    }

    /// Borrows the raw key bytes.
    pub fn as_bytes(&self) -> &[u8; GROUP_KEY_LEN] {
        &self.0
    }

    /// Returns the public group identifier (64 lowercase hex characters).
    ///
    /// Derived with context string `"clipbridge 2026-11 group-id v1"`.
    pub fn group_id(&self) -> String {
        let derived = blake3::derive_key(CONTEXT_GROUP_ID, &self.0);
        hex::encode(derived)
    }

    /// Returns the relay authentication bearer token (43 URL-safe base64 characters).
    ///
    /// Derived with context string `"clipbridge 2026-11 relay-auth v1"`.
    pub fn relay_token(&self) -> String {
        let derived = blake3::derive_key(CONTEXT_RELAY_AUTH, &self.0);
        URL_SAFE_NO_PAD.encode(derived)
    }

    /// Returns the 32-byte envelope encryption key.
    ///
    /// Derived with context string `"clipbridge 2026-11 envelope-enc v1"`.
    /// This key never leaves the client device.
    pub(crate) fn enc_key(&self) -> [u8; GROUP_KEY_LEN] {
        blake3::derive_key(CONTEXT_ENVELOPE_ENC, &self.0)
    }
}

impl std::fmt::Debug for GroupKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("GroupKey").field(&"[REDACTED]").finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_derivation_determinism_and_uniqueness() {
        let key = GroupKey::from_bytes([42u8; 32]);
        let gid1 = key.group_id();
        let gid2 = key.group_id();
        assert_eq!(gid1, gid2);
        assert_eq!(gid1.len(), 64);
        assert!(gid1.chars().all(|c| c.is_ascii_hexdigit()));

        let token1 = key.relay_token();
        let token2 = key.relay_token();
        assert_eq!(token1, token2);
        assert_eq!(token1.len(), 43);

        let enc1 = key.enc_key();
        let enc2 = key.enc_key();
        assert_eq!(enc1, enc2);

        // Subkeys derived from different contexts must not be equal.
        let gid_bytes = hex::decode(&gid1).unwrap();
        let token_bytes = URL_SAFE_NO_PAD.decode(&token1).unwrap();
        assert_ne!(gid_bytes.as_slice(), token_bytes.as_slice());
        assert_ne!(gid_bytes.as_slice(), enc1.as_slice());
        assert_ne!(token_bytes.as_slice(), enc1.as_slice());
    }

    #[test]
    fn test_base64url_roundtrip() {
        let original = GroupKey::generate();
        let encoded = original.to_base64url();
        assert_eq!(encoded.len(), 43);
        let restored = GroupKey::from_base64url(&encoded).unwrap();
        assert_eq!(original, restored);
    }

    #[test]
    fn test_invalid_base64url_key() {
        // Too short
        assert!(GroupKey::from_base64url("AAAA").is_err());
        // Invalid characters
        assert!(GroupKey::from_base64url("invalid-base64-content!@#$%^&*()_+").is_err());
    }

    #[test]
    fn test_debug_redaction() {
        let key = GroupKey::from_bytes([0x7f; 32]);
        let debug_str = format!("{key:?}");
        assert_eq!(debug_str, "GroupKey(\"[REDACTED]\")");
        assert!(!debug_str.contains("7f"));
    }
}
