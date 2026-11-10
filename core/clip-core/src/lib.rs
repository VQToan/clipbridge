//! # ClipBridge Core (`clip-core`)
//!
//! Shared core library implementing end-to-end encrypted (E2EE) cross-platform clipboard synchronization.
//!
//! Provides cryptographic primitives (BLAKE3 key derivation, XChaCha20-Poly1305 encryption),
//! wire protocol formatting, local history storage via SQLite, deduplication guards, image normalization,
//! and optional WebSocket relay client connectivity.

pub mod envelope;
pub mod error;
pub mod guard;
pub mod image;
pub mod keys;
pub mod model;
pub mod pairing;
#[cfg(feature = "relay")]
pub mod relay;
pub mod storage;

pub use envelope::{Envelope, ENVELOPE_VERSION, XCHACHA_NONCE_LEN};
pub use error::{CoreError, Result};
pub use guard::{EchoGuard, ECHO_GUARD_CAPACITY};
pub use image::{normalize_to_png, png_to_rgba, rgba_to_png, thumbnail_png};
pub use keys::{GroupKey, GROUP_KEY_LEN};
pub use model::{content_hash, ClipKind, ClipPayload, MAX_PAYLOAD_BYTES, MIME_IMAGE_PNG, MIME_TEXT_PLAIN};
pub use pairing::{PairingInfo, PAIRING_HOST, PAIRING_SCHEME, PAIRING_VERSION};
#[cfg(feature = "relay")]
pub use relay::{
    RelayConfig, RelayConnection, RelayEvent, RelayReceiver, RelaySender, MAX_WS_FRAME_SIZE,
};
pub use storage::{History, HistoryEntry};

/// Supported wire protocol version.
pub const PROTOCOL_VERSION: u8 = 1;

/// Generates a new random unique device identifier formatted as a ULID string.
pub fn new_device_id() -> String {
    ulid::Ulid::generate().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_protocol_version() {
        assert_eq!(PROTOCOL_VERSION, 1);
    }

    #[test]
    fn test_new_device_id_format() {
        let id = new_device_id();
        assert_eq!(id.len(), 26);
        assert!(ulid::Ulid::from_string(&id).is_ok());
    }
}
