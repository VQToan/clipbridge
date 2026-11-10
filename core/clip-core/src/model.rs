use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};

/// Maximum allowed payload plaintext size (10 MiB) per `protocol.md` section 5.
pub const MAX_PAYLOAD_BYTES: usize = 10 * 1024 * 1024;

/// Default MIME type for plain text clipboard payloads.
pub const MIME_TEXT_PLAIN: &str = "text/plain; charset=utf-8";

/// Standard MIME type for PNG image payloads.
pub const MIME_IMAGE_PNG: &str = "image/png";

/// Kind of clipboard content supported by ClipBridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
pub enum ClipKind {
    /// UTF-8 plain text.
    Text = 1,
    /// PNG-encoded image data.
    Image = 2,
}

impl ClipKind {
    /// Domain-separation tag byte used in BLAKE3 content hashing.
    #[inline]
    pub const fn tag_byte(self) -> u8 {
        self as u8
    }

    /// Converts a raw tag byte to `ClipKind`, if valid.
    pub const fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            1 => Some(ClipKind::Text),
            2 => Some(ClipKind::Image),
            _ => None,
        }
    }
}

impl std::fmt::Display for ClipKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClipKind::Text => write!(f, "text"),
            ClipKind::Image => write!(f, "image"),
        }
    }
}

/// Computes a 32-byte BLAKE3 content hash over `tag_byte + mime + 0x00 + data`.
///
/// The separator `0x00` guarantees collision resistance between MIME and data boundaries.
pub fn content_hash(kind: ClipKind, mime: &str, data: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&[kind.tag_byte()]);
    hasher.update(mime.as_bytes());
    hasher.update(&[0x00]);
    hasher.update(data);
    *hasher.finalize().as_bytes()
}

/// Decrypted clipboard payload transferred inside an encrypted envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipPayload {
    /// High-level content classification.
    pub kind: ClipKind,
    /// MIME type identifying the binary format of `data`.
    pub mime: String,
    /// Raw payload bytes (UTF-8 text or PNG image).
    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>,
    /// Cryptographic checksum of kind, mime, and data.
    #[serde(with = "serde_bytes")]
    pub hash: Vec<u8>,
}

impl ClipPayload {
    /// Constructs a UTF-8 text payload with default MIME `text/plain; charset=utf-8`.
    pub fn text(text: impl Into<String>) -> Result<Self> {
        let s = text.into();
        let data = s.into_bytes();
        Self::new(ClipKind::Text, MIME_TEXT_PLAIN, data)
    }

    /// Constructs an image payload containing PNG-encoded image bytes.
    pub fn image_png(data: Vec<u8>) -> Result<Self> {
        Self::new(ClipKind::Image, MIME_IMAGE_PNG, data)
    }

    /// Constructs a payload with an explicit MIME type and validates the size limit.
    pub fn new(kind: ClipKind, mime: impl Into<String>, data: Vec<u8>) -> Result<Self> {
        let size = data.len();
        if size > MAX_PAYLOAD_BYTES {
            return Err(CoreError::PayloadTooLarge {
                size,
                max: MAX_PAYLOAD_BYTES,
            });
        }
        let mime = mime.into();
        let hash = content_hash(kind, &mime, &data).to_vec();
        Ok(Self {
            kind,
            mime,
            data,
            hash,
        })
    }

    /// Returns the text content if this payload is `ClipKind::Text` and valid UTF-8.
    pub fn as_text(&self) -> Option<&str> {
        if self.kind == ClipKind::Text {
            std::str::from_utf8(&self.data).ok()
        } else {
            None
        }
    }

    /// Verifies that the internal hash matches the recomputed content hash.
    pub fn verify_hash(&self) -> bool {
        let expected = content_hash(self.kind, &self.mime, &self.data);
        self.hash.as_slice() == expected.as_slice()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_payload_creation() {
        let payload = ClipPayload::text("Hello, ClipBridge!").unwrap();
        assert_eq!(payload.kind, ClipKind::Text);
        assert_eq!(payload.mime, MIME_TEXT_PLAIN);
        assert_eq!(payload.as_text(), Some("Hello, ClipBridge!"));
        assert!(payload.verify_hash());
    }

    #[test]
    fn test_image_payload_creation() {
        let dummy_png = vec![0x89, b'P', b'N', b'G', 1, 2, 3, 4];
        let payload = ClipPayload::image_png(dummy_png.clone()).unwrap();
        assert_eq!(payload.kind, ClipKind::Image);
        assert_eq!(payload.mime, MIME_IMAGE_PNG);
        assert_eq!(payload.data, dummy_png);
        assert_eq!(payload.as_text(), None);
        assert!(payload.verify_hash());
    }

    #[test]
    fn test_payload_too_large() {
        let oversized = vec![0u8; MAX_PAYLOAD_BYTES + 1];
        let err = ClipPayload::image_png(oversized).unwrap_err();
        match err {
            CoreError::PayloadTooLarge { size, max } => {
                assert_eq!(size, MAX_PAYLOAD_BYTES + 1);
                assert_eq!(max, MAX_PAYLOAD_BYTES);
            }
            other => panic!("expected PayloadTooLarge, got: {other:?}"),
        }
    }

    #[test]
    fn test_content_hash_domain_separation() {
        let data = b"common data";
        let h_text = content_hash(ClipKind::Text, "text/plain", data);
        let h_image = content_hash(ClipKind::Image, "text/plain", data);
        assert_ne!(h_text, h_image);
    }
}
