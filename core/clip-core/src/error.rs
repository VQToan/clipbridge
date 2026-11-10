use thiserror::Error;

/// CoreError enumerates all error conditions within `clip-core`.
#[derive(Error, Debug)]
pub enum CoreError {
    /// Cryptographic operations (key derivation, encryption, decryption, invalid nonce/key length).
    #[error("cryptographic error: {0}")]
    Crypto(String),

    /// Decoding failures (base64, CBOR, envelope formatting, JSON).
    #[error("decoding error: {0}")]
    Decode(String),

    /// Payload size exceeds the allowed limit.
    #[error("payload too large: {size} bytes exceeds maximum of {max} bytes")]
    PayloadTooLarge {
        /// Actual size of the payload data in bytes.
        size: usize,
        /// Maximum allowed size in bytes.
        max: usize,
    },

    /// The envelope's group_id does not match the expected group key.
    #[error("group mismatch: envelope group_id does not match group key")]
    GroupMismatch,

    /// Integrity verification failed: computed hash does not match payload hash.
    #[error("hash mismatch: content integrity check failed")]
    HashMismatch,

    /// SQLite storage error.
    #[error("storage error: {0}")]
    Storage(#[from] rusqlite::Error),

    /// Image processing failure (decoding, encoding, invalid dimensions).
    #[error("image error: {0}")]
    Image(String),

    /// Pairing URI is invalid or malformed.
    #[error("invalid pairing URI: {0}")]
    InvalidPairingUri(String),

    /// Relay network / protocol error.
    #[error("relay error: {0}")]
    Relay(String),

    /// Underlying I/O error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization failures.
    #[error("serialization error: {0}")]
    Serialization(String),
}

/// Convenience alias for `Result` with `CoreError`.
pub type Result<T> = std::result::Result<T, CoreError>;

impl From<serde_json::Error> for CoreError {
    fn from(err: serde_json::Error) -> Self {
        CoreError::Serialization(err.to_string())
    }
}

impl From<image::ImageError> for CoreError {
    fn from(err: image::ImageError) -> Self {
        CoreError::Image(err.to_string())
    }
}

impl From<base64::DecodeError> for CoreError {
    fn from(err: base64::DecodeError) -> Self {
        CoreError::Decode(err.to_string())
    }
}
