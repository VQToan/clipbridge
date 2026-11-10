use std::time::SystemTime;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chacha20poly1305::aead::rand_core::RngCore;
use chacha20poly1305::aead::{Aead, KeyInit, OsRng, Payload as AeadPayload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::keys::GroupKey;
use crate::model::{content_hash, ClipPayload, MAX_PAYLOAD_BYTES};

/// Current envelope schema version.
pub const ENVELOPE_VERSION: u8 = 1;

/// Nonce byte length for XChaCha20-Poly1305.
pub const XCHACHA_NONCE_LEN: usize = 24;

/// Additional overhead allowance for AEAD tag and CBOR structure overhead.
const MAX_ENCRYPTED_OVERHEAD: usize = 64 * 1024;

/// End-to-end encrypted envelope containing clipboard payload and routing metadata.
///
/// Serialized to JSON when transmitted over WebSocket relay or stored in transit mailboxes.
/// The relay verifies only envelope metadata (`v`, `id`, `group_id`, `sender`) and cannot
/// view the plaintext or tamper with header fields without invalidating the AEAD authentication tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    /// Protocol version (currently 1).
    pub v: u8,
    /// Unique ULID identifier of this envelope.
    pub id: String,
    /// 64-char lowercase hex group identifier.
    pub group_id: String,
    /// Originating device identifier.
    pub sender: String,
    /// Creation timestamp in milliseconds since Unix epoch.
    pub created_at: i64,
    /// Base64 (standard) encoded 24-byte XChaCha20-Poly1305 nonce.
    pub nonce: String,
    /// Base64 (standard) encoded ciphertext of the CBOR-encoded `ClipPayload`.
    pub ciphertext: String,
}

impl Envelope {
    /// Encrypts and packages a `ClipPayload` into an `Envelope`.
    ///
    /// The payload is serialized with CBOR, then encrypted using XChaCha20-Poly1305.
    /// Additional Authenticated Data (AAD) binds `id`, `group_id`, `sender`, and `created_at`
    /// to the ciphertext, preventing relay re-tagging or spoofing attacks.
    pub fn seal(key: &GroupKey, sender: &str, payload: &ClipPayload) -> Result<Self> {
        if payload.data.len() > MAX_PAYLOAD_BYTES {
            return Err(CoreError::PayloadTooLarge {
                size: payload.data.len(),
                max: MAX_PAYLOAD_BYTES,
            });
        }

        let id = ulid::Ulid::generate().to_string();
        let group_id = key.group_id();
        let created_at = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(|e| CoreError::Crypto(e.to_string()))?
            .as_millis() as i64;

        let mut nonce_bytes = [0u8; XCHACHA_NONCE_LEN];
        OsRng.fill_bytes(&mut nonce_bytes);

        // Serialize payload into CBOR
        let mut cbor_bytes = Vec::new();
        ciborium::into_writer(payload, &mut cbor_bytes)
            .map_err(|e| CoreError::Serialization(e.to_string()))?;

        // AAD binds routing headers into the AEAD authentication tag
        let aad = format!("cb1|{id}|{group_id}|{sender}|{created_at}");

        let cipher = XChaCha20Poly1305::new_from_slice(&key.enc_key())
            .map_err(|e| CoreError::Crypto(e.to_string()))?;

        let ciphertext_bytes = cipher
            .encrypt(
                XNonce::from_slice(&nonce_bytes),
                AeadPayload {
                    msg: &cbor_bytes,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|e| CoreError::Crypto(format!("encryption failed: {e}")))?;

        Ok(Self {
            v: ENVELOPE_VERSION,
            id,
            group_id,
            sender: sender.to_string(),
            created_at,
            nonce: STANDARD.encode(nonce_bytes),
            ciphertext: STANDARD.encode(ciphertext_bytes),
        })
    }

    /// Decrypts and verifies the `ClipPayload` contained within this envelope.
    ///
    /// Verifies version compatibility, validates that `group_id` matches the provided `GroupKey`,
    /// authenticates the header fields against the AEAD tag, enforces payload size constraints,
    /// and checks the content integrity hash.
    pub fn open(&self, key: &GroupKey) -> Result<ClipPayload> {
        if self.v != ENVELOPE_VERSION {
            return Err(CoreError::Decode(format!(
                "unsupported envelope version: {}",
                self.v
            )));
        }

        if self.group_id != key.group_id() {
            return Err(CoreError::GroupMismatch);
        }

        let nonce_bytes = STANDARD
            .decode(&self.nonce)
            .map_err(|e| CoreError::Decode(format!("invalid base64 nonce: {e}")))?;
        if nonce_bytes.len() != XCHACHA_NONCE_LEN {
            return Err(CoreError::Crypto(format!(
                "nonce must be {XCHACHA_NONCE_LEN} bytes, got {}",
                nonce_bytes.len()
            )));
        }

        let ciphertext_bytes = STANDARD
            .decode(&self.ciphertext)
            .map_err(|e| CoreError::Decode(format!("invalid base64 ciphertext: {e}")))?;

        if ciphertext_bytes.len() > MAX_PAYLOAD_BYTES + MAX_ENCRYPTED_OVERHEAD {
            return Err(CoreError::PayloadTooLarge {
                size: ciphertext_bytes.len(),
                max: MAX_PAYLOAD_BYTES,
            });
        }

        let aad = format!(
            "cb1|{}|{}|{}|{}",
            self.id, self.group_id, self.sender, self.created_at
        );

        let cipher = XChaCha20Poly1305::new_from_slice(&key.enc_key())
            .map_err(|e| CoreError::Crypto(e.to_string()))?;

        let decrypted = cipher
            .decrypt(
                XNonce::from_slice(&nonce_bytes),
                AeadPayload {
                    msg: &ciphertext_bytes,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|e| CoreError::Crypto(format!("decryption failed: {e}")))?;

        let payload: ClipPayload = ciborium::from_reader(decrypted.as_slice())
            .map_err(|e| CoreError::Decode(format!("cbor decode error: {e}")))?;

        if payload.data.len() > MAX_PAYLOAD_BYTES {
            return Err(CoreError::PayloadTooLarge {
                size: payload.data.len(),
                max: MAX_PAYLOAD_BYTES,
            });
        }

        let expected_hash = content_hash(payload.kind, &payload.mime, &payload.data);
        if payload.hash.as_slice() != expected_hash.as_slice() {
            return Err(CoreError::HashMismatch);
        }

        Ok(payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ClipKind;

    #[test]
    fn test_seal_and_open_roundtrip_text() {
        let key = GroupKey::generate();
        let payload = ClipPayload::text("Confidential clipboard data").unwrap();
        let env = Envelope::seal(&key, "device-a", &payload).unwrap();

        assert_eq!(env.v, 1);
        assert_eq!(env.sender, "device-a");
        assert_eq!(env.group_id, key.group_id());

        let opened = env.open(&key).unwrap();
        assert_eq!(opened, payload);
        assert_eq!(opened.as_text(), Some("Confidential clipboard data"));
    }

    #[test]
    fn test_seal_and_open_roundtrip_image() {
        let key = GroupKey::generate();
        let dummy_png = vec![0x89, b'P', b'N', b'G', 13, 10, 26, 10, 0, 0, 0, 1];
        let payload = ClipPayload::image_png(dummy_png).unwrap();
        let env = Envelope::seal(&key, "device-b", &payload).unwrap();

        let opened = env.open(&key).unwrap();
        assert_eq!(opened.kind, ClipKind::Image);
        assert_eq!(opened, payload);
    }

    #[test]
    fn test_tampering_header_fails_aead() {
        let key = GroupKey::generate();
        let payload = ClipPayload::text("tamper test").unwrap();
        let mut env = Envelope::seal(&key, "device-a", &payload).unwrap();

        // Tamper sender
        env.sender = "device-attacker".to_string();
        assert!(env.open(&key).is_err());

        // Restore sender, tamper id
        env.sender = "device-a".to_string();
        env.id = ulid::Ulid::generate().to_string();
        assert!(env.open(&key).is_err());

        // Tamper created_at
        env.created_at += 1000;
        assert!(env.open(&key).is_err());
    }

    #[test]
    fn test_tampering_ciphertext_fails() {
        let key = GroupKey::generate();
        let payload = ClipPayload::text("crypto tamper test").unwrap();
        let mut env = Envelope::seal(&key, "device-a", &payload).unwrap();

        let mut ct_bytes = STANDARD.decode(&env.ciphertext).unwrap();
        if let Some(first) = ct_bytes.first_mut() {
            *first ^= 0xff;
        }
        env.ciphertext = STANDARD.encode(ct_bytes);

        let err = env.open(&key).unwrap_err();
        match err {
            CoreError::Crypto(_) => {}
            other => panic!("expected Crypto error on tampered ciphertext, got: {other:?}"),
        }
    }

    #[test]
    fn test_wrong_key_group_mismatch() {
        let key1 = GroupKey::generate();
        let key2 = GroupKey::generate();
        let payload = ClipPayload::text("multi-group test").unwrap();
        let env = Envelope::seal(&key1, "device-a", &payload).unwrap();

        let err = env.open(&key2).unwrap_err();
        match err {
            CoreError::GroupMismatch => {}
            other => panic!("expected GroupMismatch, got: {other:?}"),
        }
    }

    #[test]
    fn test_envelope_json_serialization_matches_protocol_spec() {
        let key = GroupKey::generate();
        let payload = ClipPayload::text("protocol spec check").unwrap();
        let env = Envelope::seal(&key, "device-123", &payload).unwrap();

        let json_val = serde_json::to_value(&env).unwrap();
        let obj = json_val.as_object().expect("Envelope must serialize to JSON object");

        // Verify exact field names required by protocol.md Section 4
        assert!(obj.contains_key("v"));
        assert!(obj.contains_key("id"));
        assert!(obj.contains_key("group_id"));
        assert!(obj.contains_key("sender"));
        assert!(obj.contains_key("created_at"));
        assert!(obj.contains_key("nonce"));
        assert!(obj.contains_key("ciphertext"));
        assert_eq!(obj.len(), 7, "Envelope JSON must not contain extraneous fields");
    }
}
