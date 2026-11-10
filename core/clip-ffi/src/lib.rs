use std::path::Path;
use std::sync::{Arc, Mutex};

uniffi::setup_scaffolding!();

/// Errors that can occur across the ClipBridge FFI boundary.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum FfiError {
    #[error("cryptographic error: {msg}")]
    Crypto { msg: String },
    #[error("decoding error: {msg}")]
    Decode { msg: String },
    #[error("payload too large: {size} bytes exceeds maximum of {max} bytes")]
    PayloadTooLarge { size: u64, max: u64 },
    #[error("group mismatch: envelope group_id does not match group key")]
    GroupMismatch,
    #[error("hash mismatch: content integrity check failed")]
    HashMismatch,
    #[error("storage error: {msg}")]
    Storage { msg: String },
    #[error("image error: {msg}")]
    Image { msg: String },
    #[error("invalid pairing URI: {msg}")]
    InvalidPairingUri { msg: String },
    #[error("relay error: {msg}")]
    Relay { msg: String },
    #[error("io error: {msg}")]
    Io { msg: String },
    #[error("serialization error: {msg}")]
    Serialization { msg: String },
    #[error("generic error: {msg}")]
    Generic { msg: String },
}

impl From<clip_core::CoreError> for FfiError {
    fn from(err: clip_core::CoreError) -> Self {
        match err {
            clip_core::CoreError::Crypto(msg) => FfiError::Crypto { msg },
            clip_core::CoreError::Decode(msg) => FfiError::Decode { msg },
            clip_core::CoreError::PayloadTooLarge { size, max } => FfiError::PayloadTooLarge {
                size: size as u64,
                max: max as u64,
            },
            clip_core::CoreError::GroupMismatch => FfiError::GroupMismatch,
            clip_core::CoreError::HashMismatch => FfiError::HashMismatch,
            clip_core::CoreError::Storage(e) => FfiError::Storage {
                msg: e.to_string(),
            },
            clip_core::CoreError::Image(msg) => FfiError::Image { msg },
            clip_core::CoreError::InvalidPairingUri(msg) => FfiError::InvalidPairingUri { msg },
            clip_core::CoreError::Relay(msg) => FfiError::Relay { msg },
            clip_core::CoreError::Io(e) => FfiError::Io {
                msg: e.to_string(),
            },
            clip_core::CoreError::Serialization(msg) => FfiError::Serialization { msg },
        }
    }
}

impl From<anyhow::Error> for FfiError {
    fn from(err: anyhow::Error) -> Self {
        FfiError::Generic {
            msg: err.to_string(),
        }
    }
}

/// Content kind supported by ClipBridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiClipKind {
    Text,
    Image,
}

impl From<clip_core::ClipKind> for FfiClipKind {
    fn from(kind: clip_core::ClipKind) -> Self {
        match kind {
            clip_core::ClipKind::Text => FfiClipKind::Text,
            clip_core::ClipKind::Image => FfiClipKind::Image,
        }
    }
}

impl From<FfiClipKind> for clip_core::ClipKind {
    fn from(kind: FfiClipKind) -> Self {
        match kind {
            FfiClipKind::Text => clip_core::ClipKind::Text,
            FfiClipKind::Image => clip_core::ClipKind::Image,
        }
    }
}

/// Decrypted clipboard payload.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Object)]
pub struct FfiClipPayload {
    pub(crate) inner: clip_core::ClipPayload,
}

#[uniffi::export]
impl FfiClipPayload {
    /// Creates a text payload with default UTF-8 text MIME.
    #[uniffi::constructor]
    pub fn text(text: String) -> Result<Arc<Self>, FfiError> {
        let inner = clip_core::ClipPayload::text(text)?;
        Ok(Arc::new(Self { inner }))
    }

    /// Creates an image payload containing PNG-encoded image bytes.
    #[uniffi::constructor]
    pub fn image(data: Vec<u8>) -> Result<Arc<Self>, FfiError> {
        let inner = clip_core::ClipPayload::image_png(data)?;
        Ok(Arc::new(Self { inner }))
    }

    /// Creates a payload with an explicit MIME type.
    #[uniffi::constructor]
    pub fn new(kind: FfiClipKind, mime: String, data: Vec<u8>) -> Result<Arc<Self>, FfiError> {
        let inner = clip_core::ClipPayload::new(kind.into(), mime, data)?;
        Ok(Arc::new(Self { inner }))
    }

    pub fn kind(&self) -> FfiClipKind {
        self.inner.kind.into()
    }

    pub fn mime(&self) -> String {
        self.inner.mime.clone()
    }

    pub fn data(&self) -> Vec<u8> {
        self.inner.data.clone()
    }

    pub fn hash(&self) -> Vec<u8> {
        self.inner.hash.clone()
    }

    pub fn as_text(&self) -> Option<String> {
        self.inner.as_text().map(|s| s.to_string())
    }

    pub fn verify_hash(&self) -> bool {
        self.inner.verify_hash()
    }
}

/// Shared 32-byte symmetric group root key.
#[derive(Debug, Clone, uniffi::Object)]
pub struct FfiGroupKey {
    pub(crate) inner: clip_core::GroupKey,
}

#[uniffi::export]
impl FfiGroupKey {
    /// Generates a new random cryptographic group key.
    #[uniffi::constructor]
    pub fn generate() -> Arc<Self> {
        Arc::new(Self {
            inner: clip_core::GroupKey::generate(),
        })
    }

    /// Parses a group key from a URL-safe unpadded base64 string.
    #[uniffi::constructor]
    pub fn from_base64url(s: String) -> Result<Arc<Self>, FfiError> {
        let inner = clip_core::GroupKey::from_base64url(&s)?;
        Ok(Arc::new(Self { inner }))
    }

    /// Constructs a group key from raw 32 bytes.
    #[uniffi::constructor]
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Arc<Self>, FfiError> {
        if bytes.len() != clip_core::GROUP_KEY_LEN {
            return Err(FfiError::Crypto {
                msg: format!(
                    "invalid group key length: expected {}, got {}",
                    clip_core::GROUP_KEY_LEN,
                    bytes.len()
                ),
            });
        }
        let mut arr = [0u8; clip_core::GROUP_KEY_LEN];
        arr.copy_from_slice(&bytes);
        Ok(Arc::new(Self {
            inner: clip_core::GroupKey::from_bytes(arr),
        }))
    }

    pub fn to_base64url(&self) -> String {
        self.inner.to_base64url()
    }

    pub fn group_id(&self) -> String {
        self.inner.group_id()
    }

    pub fn relay_token(&self) -> String {
        self.inner.relay_token()
    }

    pub fn as_bytes(&self) -> Vec<u8> {
        self.inner.as_bytes().to_vec()
    }
}

/// Configuration for pairing via deep links or QR codes.
#[derive(Debug, Clone, uniffi::Object)]
pub struct FfiPairingInfo {
    pub(crate) inner: clip_core::PairingInfo,
}

#[uniffi::export]
impl FfiPairingInfo {
    #[uniffi::constructor]
    pub fn new(relay_url: String, group_key: &FfiGroupKey) -> Arc<Self> {
        Arc::new(Self {
            inner: clip_core::PairingInfo::new(relay_url, &group_key.inner),
        })
    }

    #[uniffi::constructor]
    pub fn from_uri(uri: String) -> Result<Arc<Self>, FfiError> {
        let inner = clip_core::PairingInfo::from_uri(&uri)?;
        Ok(Arc::new(Self { inner }))
    }

    pub fn to_uri(&self) -> String {
        self.inner.to_uri()
    }

    pub fn group_key(&self) -> Result<Arc<FfiGroupKey>, FfiError> {
        let key = self.inner.group_key()?;
        Ok(Arc::new(FfiGroupKey { inner: key }))
    }

    pub fn relay_url(&self) -> String {
        self.inner.relay_url.clone()
    }

    pub fn v(&self) -> u8 {
        self.inner.v
    }

    pub fn group_key_str(&self) -> String {
        self.inner.group_key.clone()
    }
}

/// End-to-end encrypted envelope containing clipboard ciphertext and routing metadata.
#[derive(Debug, Clone, uniffi::Object)]
pub struct FfiEnvelope {
    pub(crate) inner: clip_core::Envelope,
}

#[uniffi::export]
impl FfiEnvelope {
    /// Seals (encrypts) a `ClipPayload` into an `Envelope` using the group key.
    #[uniffi::constructor]
    pub fn seal(
        key: &FfiGroupKey,
        sender: String,
        payload: &FfiClipPayload,
    ) -> Result<Arc<Self>, FfiError> {
        let inner = clip_core::Envelope::seal(&key.inner, &sender, &payload.inner)?;
        Ok(Arc::new(Self { inner }))
    }

    /// Constructs an envelope from raw header fields and ciphertext.
    #[uniffi::constructor]
    pub fn new(
        v: u8,
        id: String,
        group_id: String,
        sender: String,
        created_at: i64,
        nonce: String,
        ciphertext: String,
    ) -> Arc<Self> {
        Arc::new(Self {
            inner: clip_core::Envelope {
                v,
                id,
                group_id,
                sender,
                created_at,
                nonce,
                ciphertext,
            },
        })
    }

    /// Decrypts and verifies the payload within this envelope.
    pub fn open(&self, key: &FfiGroupKey) -> Result<Arc<FfiClipPayload>, FfiError> {
        let payload = self.inner.open(&key.inner)?;
        Ok(Arc::new(FfiClipPayload { inner: payload }))
    }

    pub fn v(&self) -> u8 {
        self.inner.v
    }

    pub fn id(&self) -> String {
        self.inner.id.clone()
    }

    pub fn group_id(&self) -> String {
        self.inner.group_id.clone()
    }

    pub fn sender(&self) -> String {
        self.inner.sender.clone()
    }

    pub fn created_at(&self) -> i64 {
        self.inner.created_at
    }

    pub fn nonce(&self) -> String {
        self.inner.nonce.clone()
    }

    pub fn ciphertext(&self) -> String {
        self.inner.ciphertext.clone()
    }
}

/// Deduplication and echo-loop prevention guard.
#[derive(Debug, Default, uniffi::Object)]
pub struct FfiEchoGuard {
    inner: Mutex<clip_core::EchoGuard>,
}

#[uniffi::export]
impl FfiEchoGuard {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(clip_core::EchoGuard::new()),
        })
    }

    pub fn mark_applied(&self, hash: Vec<u8>) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.mark_applied(&hash);
        }
    }

    pub fn should_publish(&self, hash: Vec<u8>) -> bool {
        if let Ok(mut guard) = self.inner.lock() {
            guard.should_publish(&hash)
        } else {
            false
        }
    }

    pub fn seen_envelope(&self, id: String) -> bool {
        if let Ok(mut guard) = self.inner.lock() {
            guard.seen_envelope(&id)
        } else {
            false
        }
    }

    pub fn last_applied(&self) -> Option<Vec<u8>> {
        self.inner.lock().ok().and_then(|g| g.last_applied().map(|h| h.to_vec()))
    }

    pub fn last_published(&self) -> Option<Vec<u8>> {
        self.inner.lock().ok().and_then(|g| g.last_published().map(|h| h.to_vec()))
    }
}

/// A stored clipboard history item record.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiHistoryEntry {
    pub id: String,
    pub origin: String,
    pub kind: FfiClipKind,
    pub mime: String,
    pub hash: Vec<u8>,
    pub size: u64,
    pub created_at: i64,
    pub data: Vec<u8>,
    pub is_remote: bool,
}

impl From<clip_core::HistoryEntry> for FfiHistoryEntry {
    fn from(e: clip_core::HistoryEntry) -> Self {
        Self {
            id: e.id,
            origin: e.origin,
            kind: e.kind.into(),
            mime: e.mime,
            hash: e.hash,
            size: e.size as u64,
            created_at: e.created_at,
            data: e.data,
            is_remote: e.is_remote,
        }
    }
}

impl From<FfiHistoryEntry> for clip_core::HistoryEntry {
    fn from(e: FfiHistoryEntry) -> Self {
        Self {
            id: e.id,
            origin: e.origin,
            kind: e.kind.into(),
            mime: e.mime,
            hash: e.hash,
            size: e.size as usize,
            created_at: e.created_at,
            data: e.data,
            is_remote: e.is_remote,
        }
    }
}

/// Persistent clipboard history database backed by SQLite.
#[derive(uniffi::Object)]
pub struct FfiHistory {
    inner: clip_core::History,
}

#[uniffi::export]
impl FfiHistory {
    /// Opens or creates the SQLite history database at `path`.
    #[uniffi::constructor]
    pub fn open(path: String) -> Result<Arc<Self>, FfiError> {
        let inner = clip_core::History::open(Path::new(&path))?;
        Ok(Arc::new(Self { inner }))
    }

    /// Opens an in-memory SQLite history database (for testing).
    #[uniffi::constructor]
    pub fn open_in_memory() -> Result<Arc<Self>, FfiError> {
        let inner = clip_core::History::open_in_memory()?;
        Ok(Arc::new(Self { inner }))
    }

    pub fn insert(&self, entry: FfiHistoryEntry) -> Result<(), FfiError> {
        self.inner.insert(&entry.into())?;
        Ok(())
    }

    pub fn recent(&self, limit: u32) -> Result<Vec<FfiHistoryEntry>, FfiError> {
        let list = self.inner.recent(limit as usize)?;
        Ok(list.into_iter().map(FfiHistoryEntry::from).collect())
    }

    pub fn contains_hash(&self, hash: Vec<u8>) -> Result<bool, FfiError> {
        Ok(self.inner.contains_hash(&hash)?)
    }

    pub fn prune_older_than(&self, cutoff_ms: i64) -> Result<u64, FfiError> {
        let count = self.inner.prune_older_than(cutoff_ms)?;
        Ok(count as u64)
    }

    pub fn last_remote_envelope_id(&self) -> Result<Option<String>, FfiError> {
        Ok(self.inner.last_remote_envelope_id()?)
    }
}

/// Events delivered by the WebSocket relay hub.
#[derive(Debug, Clone, uniffi::Enum)]
pub enum FfiRelayEvent {
    Deliver {
        envelope: Arc<FfiEnvelope>,
    },
    Ack {
        id: String,
    },
    Error {
        code: String,
        message: String,
        id: Option<String>,
    },
}

impl From<clip_core::RelayEvent> for FfiRelayEvent {
    fn from(ev: clip_core::RelayEvent) -> Self {
        match ev {
            clip_core::RelayEvent::Deliver(env) => FfiRelayEvent::Deliver {
                envelope: Arc::new(FfiEnvelope { inner: env }),
            },
            clip_core::RelayEvent::Ack(id) => FfiRelayEvent::Ack { id },
            clip_core::RelayEvent::Error { code, message, id } => {
                FfiRelayEvent::Error { code, message, id }
            }
        }
    }
}

/// Active WebSocket client connection to the ClipBridge relay hub.
#[derive(uniffi::Object)]
pub struct FfiRelayClient {
    sender: tokio::sync::Mutex<clip_core::RelaySender>,
    receiver: tokio::sync::Mutex<clip_core::RelayReceiver>,
}

#[uniffi::export(async_runtime = "tokio")]
impl FfiRelayClient {
    /// Connects to the relay server with the given parameters.
    #[uniffi::constructor]
    pub async fn connect(
        base_url: String,
        device_id: String,
        key: &FfiGroupKey,
        since: Option<String>,
    ) -> Result<Arc<Self>, FfiError> {
        let cfg = clip_core::RelayConfig {
            base_url,
            device_id,
            key: key.inner.clone(),
            since,
        };
        let conn = clip_core::RelayConnection::connect(&cfg).await?;
        let (sender, receiver) = conn.split();
        Ok(Arc::new(Self {
            sender: tokio::sync::Mutex::new(sender),
            receiver: tokio::sync::Mutex::new(receiver),
        }))
    }

    /// Publishes an encrypted envelope to the group via the relay.
    pub async fn publish(&self, envelope: &FfiEnvelope) -> Result<(), FfiError> {
        let mut sender = self.sender.lock().await;
        sender.publish(&envelope.inner).await?;
        Ok(())
    }

    /// Waits for the next event from the relay server.
    /// Returns None when the connection is closed.
    pub async fn next_event(&self) -> Result<Option<FfiRelayEvent>, FfiError> {
        let mut receiver = self.receiver.lock().await;
        let event = receiver.next_event().await?;
        Ok(event.map(FfiRelayEvent::from))
    }
}

/// Generates a new random unique device identifier formatted as a ULID string.
#[uniffi::export]
pub fn ffi_new_device_id() -> String {
    clip_core::new_device_id()
}

/// Returns the supported wire protocol version.
#[uniffi::export]
pub fn ffi_protocol_version() -> u8 {
    clip_core::PROTOCOL_VERSION
}

/// Decodes an image from memory formats (PNG, JPEG) and normalizes it to PNG bytes.
#[uniffi::export]
pub fn ffi_normalize_to_png(bytes: Vec<u8>) -> Result<Vec<u8>, FfiError> {
    Ok(clip_core::normalize_to_png(&bytes)?)
}

/// Converts raw RGBA pixel bytes (width * height * 4) into PNG-encoded image bytes.
#[uniffi::export]
pub fn ffi_rgba_to_png(width: u32, height: u32, rgba: Vec<u8>) -> Result<Vec<u8>, FfiError> {
    Ok(clip_core::rgba_to_png(width, height, &rgba)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ffi_clip_payload_text() {
        let payload = FfiClipPayload::text("Hello FFI!".to_string()).unwrap();
        assert_eq!(payload.kind(), FfiClipKind::Text);
        assert_eq!(payload.mime(), "text/plain; charset=utf-8");
        assert_eq!(payload.as_text(), Some("Hello FFI!".to_string()));
        assert!(payload.verify_hash());
        assert_eq!(payload.data(), b"Hello FFI!");
        assert!(!payload.hash().is_empty());
    }

    #[test]
    fn test_ffi_clip_payload_image() {
        let dummy_png = vec![0x89, b'P', b'N', b'G', 1, 2, 3, 4];
        let payload = FfiClipPayload::image(dummy_png.clone()).unwrap();
        assert_eq!(payload.kind(), FfiClipKind::Image);
        assert_eq!(payload.mime(), "image/png");
        assert_eq!(payload.as_text(), None);
        assert_eq!(payload.data(), dummy_png);
        assert!(payload.verify_hash());
    }

    #[test]
    fn test_ffi_clip_payload_too_large() {
        let oversized = vec![0u8; clip_core::MAX_PAYLOAD_BYTES + 1];
        let err = FfiClipPayload::image(oversized).unwrap_err();
        match err {
            FfiError::PayloadTooLarge { size, max } => {
                assert_eq!(size, (clip_core::MAX_PAYLOAD_BYTES + 1) as u64);
                assert_eq!(max, clip_core::MAX_PAYLOAD_BYTES as u64);
            }
            other => panic!("expected PayloadTooLarge, got: {other:?}"),
        }
    }

    #[test]
    fn test_ffi_group_key_and_derivation() {
        let key = FfiGroupKey::generate();
        let b64 = key.to_base64url();
        assert_eq!(b64.len(), 43);

        let parsed = FfiGroupKey::from_base64url(b64.clone()).unwrap();
        assert_eq!(parsed.to_base64url(), b64);
        assert_eq!(parsed.group_id(), key.group_id());
        assert_eq!(parsed.relay_token(), key.relay_token());
        assert_eq!(parsed.as_bytes(), key.as_bytes());

        let bytes = key.as_bytes();
        let from_raw = FfiGroupKey::from_bytes(bytes).unwrap();
        assert_eq!(from_raw.to_base64url(), b64);

        assert!(FfiGroupKey::from_base64url("invalid-base64!".to_string()).is_err());
    }

    #[test]
    fn test_ffi_pairing_info_roundtrip() {
        let key = FfiGroupKey::generate();
        let info = FfiPairingInfo::new("wss://relay.clipbridge.dev:443".to_string(), &key);
        assert_eq!(info.v(), 1);
        assert_eq!(info.relay_url(), "wss://relay.clipbridge.dev:443");
        assert_eq!(info.group_key_str(), key.to_base64url());

        let uri = info.to_uri();
        assert!(uri.starts_with("clipbridge://pair?d="));

        let parsed = FfiPairingInfo::from_uri(uri).unwrap();
        assert_eq!(parsed.v(), 1);
        assert_eq!(parsed.relay_url(), "wss://relay.clipbridge.dev:443");
        let parsed_key = parsed.group_key().unwrap();
        assert_eq!(parsed_key.group_id(), key.group_id());
    }

    #[test]
    fn test_ffi_envelope_seal_and_open() {
        let key = FfiGroupKey::generate();
        let payload = FfiClipPayload::text("Secret clipboard message".to_string()).unwrap();

        let env = FfiEnvelope::seal(&key, "device-1".to_string(), &payload).unwrap();
        assert_eq!(env.v(), 1);
        assert_eq!(env.sender(), "device-1");
        assert_eq!(env.group_id(), key.group_id());
        assert!(!env.id().is_empty());
        assert!(!env.nonce().is_empty());
        assert!(!env.ciphertext().is_empty());
        assert!(env.created_at() > 0);

        let decrypted = env.open(&key).unwrap();
        assert_eq!(decrypted.as_text(), Some("Secret clipboard message".to_string()));
        assert_eq!(decrypted.data(), payload.data());

        // Opening with different key should fail
        let wrong_key = FfiGroupKey::generate();
        let err = env.open(&wrong_key).unwrap_err();
        assert!(matches!(err, FfiError::GroupMismatch));
    }

    #[test]
    fn test_ffi_envelope_raw_constructor() {
        let env = FfiEnvelope::new(
            1,
            "01JC0000000000000000000001".to_string(),
            "group-123".to_string(),
            "macbook".to_string(),
            123456789,
            "nonce123".to_string(),
            "ciphertext123".to_string(),
        );
        assert_eq!(env.v(), 1);
        assert_eq!(env.id(), "01JC0000000000000000000001");
        assert_eq!(env.group_id(), "group-123");
        assert_eq!(env.sender(), "macbook");
        assert_eq!(env.created_at(), 123456789);
        assert_eq!(env.nonce(), "nonce123");
        assert_eq!(env.ciphertext(), "ciphertext123");
    }

    #[test]
    fn test_ffi_echo_guard() {
        let guard = FfiEchoGuard::new();
        let hash = vec![1, 2, 3, 4];

        assert!(guard.should_publish(hash.clone()));
        // Same hash published again should be suppressed
        assert!(!guard.should_publish(hash.clone()));

        // Mark hash as applied from remote
        let remote_hash = vec![5, 6, 7, 8];
        guard.mark_applied(remote_hash.clone());
        assert_eq!(guard.last_applied(), Some(remote_hash.clone()));
        // Should not publish applied remote content
        assert!(!guard.should_publish(remote_hash));

        // Deduplication of envelope IDs
        let env_id = "01JC0000000000000000000001".to_string();
        assert!(!guard.seen_envelope(env_id.clone()));
        assert!(guard.seen_envelope(env_id));
    }

    #[test]
    fn test_ffi_history() {
        let history = FfiHistory::open_in_memory().unwrap();

        let entry = FfiHistoryEntry {
            id: "01JC0000000000000000000001".to_string(),
            origin: "device-mac".to_string(),
            kind: FfiClipKind::Text,
            mime: "text/plain".to_string(),
            hash: vec![10, 20, 30],
            size: 5,
            created_at: 1000,
            data: b"hello".to_vec(),
            is_remote: true,
        };

        history.insert(entry.clone()).unwrap();
        // Duplicate insert should be ignored
        history.insert(entry.clone()).unwrap();

        assert!(history.contains_hash(vec![10, 20, 30]).unwrap());
        assert!(!history.contains_hash(vec![99, 99]).unwrap());

        let recent = history.recent(10).unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0], entry);

        assert_eq!(
            history.last_remote_envelope_id().unwrap(),
            Some("01JC0000000000000000000001".to_string())
        );

        let pruned = history.prune_older_than(2000).unwrap();
        assert_eq!(pruned, 1);
        assert_eq!(history.recent(10).unwrap().len(), 0);
    }

    #[test]
    fn test_ffi_utilities() {
        let dev_id = ffi_new_device_id();
        assert_eq!(dev_id.len(), 26);
        assert_eq!(ffi_protocol_version(), 1);

        // 2x2 RGBA image (16 bytes)
        let rgba = vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255];
        let png = ffi_rgba_to_png(2, 2, rgba).unwrap();
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));

        let normalized = ffi_normalize_to_png(png).unwrap();
        assert!(normalized.starts_with(&[0x89, b'P', b'N', b'G']));
    }
}

