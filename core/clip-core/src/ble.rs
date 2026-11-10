//! Bluetooth Low Energy (BLE) framing and reassembly module.
//!
//! Provides MTU chunking, reassembly with bounded caching and timeouts,
//! service and characteristic UUIDs, and group beacon token derivation
//! for proximate device discovery.

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use crate::envelope::Envelope;
use crate::error::{CoreError, Result};
use crate::keys::GroupKey;
use crate::model::MAX_PAYLOAD_BYTES;

/// ClipBridge 128-bit BLE Service UUID.
pub const CLIPBRIDGE_BLE_SERVICE_UUID: &str = "0000cb01-0000-1000-8000-00805f9b34fb";

/// ClipBridge 128-bit BLE Characteristic UUID for envelope chunk transport.
pub const CLIPBRIDGE_BLE_CHAR_UUID: &str = "0000cb02-0000-1000-8000-00805f9b34fb";

/// BLAKE3 derivation context for BLE beacon group token.
pub const CONTEXT_BLE_BEACON: &str = "clipbridge 2026-11 ble-beacon v1";

/// Default BLE chunk payload capacity in bytes (fits header + payload in <= 512-byte ATT MTU).
pub const BLE_CHUNK_PAYLOAD_SIZE: usize = 490;

/// Header size of a binary BLE chunk (16-byte msg_id + 2-byte total_chunks + 2-byte chunk_index).
pub const BLE_HEADER_SIZE: usize = 20;

/// Default timeout for in-flight incomplete BLE chunk reassembly (30 seconds).
pub const DEFAULT_REASSEMBLY_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum allowed total envelope reassembly size (10 MiB) to prevent memory exhaustion attacks.
pub const MAX_REASSEMBLY_SIZE: usize = MAX_PAYLOAD_BYTES;

/// Maximum number of concurrent in-flight messages in the reassembler cache.
pub const MAX_INFLIGHT_MESSAGES: usize = 64;

/// Maximum number of recently completed message IDs cached to ignore retransmissions.
pub const MAX_COMPLETED_CACHE: usize = 256;

/// Derives a 16-byte BLE beacon identification token from the shared [`GroupKey`].
///
/// Uses `blake3::derive_key` with context `"clipbridge 2026-11 ble-beacon v1"` truncated
/// to the first 16 bytes. Nearby devices with the same group key can recognize each other
/// over BLE Manufacturer Data or Service Data advertisements without leaking the full group ID.
pub fn ble_beacon_token(key: &GroupKey) -> [u8; 16] {
    let derived = blake3::derive_key(CONTEXT_BLE_BEACON, key.as_bytes());
    let mut token = [0u8; 16];
    token.copy_from_slice(&derived[..16]);
    token
}

/// Represents a framed binary chunk of an [`Envelope`] for Bluetooth Low Energy transmission.
///
/// Binary layout:
/// - `0..16`: `msg_id` ([`[u8; 16]`])
/// - `16..18`: `total_chunks` (`u16`, big-endian)
/// - `18..20`: `chunk_index` (`u16`, big-endian)
/// - `20..`: `payload` (variable raw bytes)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BleChunk {
    /// 16-byte message identifier (e.g. ULID raw bytes or BLAKE3 hash).
    pub msg_id: [u8; 16],
    /// Total number of chunks that comprise the complete message.
    pub total_chunks: u16,
    /// 0-based index of this chunk (`0 <= chunk_index < total_chunks`).
    pub chunk_index: u16,
    /// Raw payload bytes of this chunk.
    pub payload: Vec<u8>,
}

impl BleChunk {
    /// Creates a new `BleChunk`.
    pub fn new(msg_id: [u8; 16], total_chunks: u16, chunk_index: u16, payload: Vec<u8>) -> Self {
        Self {
            msg_id,
            total_chunks,
            chunk_index,
            payload,
        }
    }

    /// Serializes the chunk into its 20-byte header + payload binary representation.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(BLE_HEADER_SIZE + self.payload.len());
        bytes.extend_from_slice(&self.msg_id);
        bytes.extend_from_slice(&self.total_chunks.to_be_bytes());
        bytes.extend_from_slice(&self.chunk_index.to_be_bytes());
        bytes.extend_from_slice(&self.payload);
        bytes
    }

    /// Deserializes a `BleChunk` from raw binary bytes.
    ///
    /// # Errors
    /// Returns [`CoreError::Decode`] if:
    /// - `bytes.len() < 20`
    /// - `total_chunks == 0`
    /// - `chunk_index >= total_chunks`
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < BLE_HEADER_SIZE {
            return Err(CoreError::Decode(format!(
                "BLE chunk too short: expected at least {BLE_HEADER_SIZE} bytes, got {}",
                bytes.len()
            )));
        }

        let mut msg_id = [0u8; 16];
        msg_id.copy_from_slice(&bytes[..16]);
        let total_chunks = u16::from_be_bytes([bytes[16], bytes[17]]);
        let chunk_index = u16::from_be_bytes([bytes[18], bytes[19]]);

        if total_chunks == 0 {
            return Err(CoreError::Decode(
                "BLE chunk total_chunks must be greater than 0".to_string(),
            ));
        }

        if chunk_index >= total_chunks {
            return Err(CoreError::Decode(format!(
                "BLE chunk_index ({chunk_index}) must be less than total_chunks ({total_chunks})"
            )));
        }

        let payload = bytes[BLE_HEADER_SIZE..].to_vec();

        Ok(Self {
            msg_id,
            total_chunks,
            chunk_index,
            payload,
        })
    }
}

/// Serializes an [`Envelope`] to JSON and partitions it into numbered [`BleChunk`]s.
///
/// Each chunk payload is bounded to at most `max_payload_size` bytes.
///
/// # Errors
/// Returns [`CoreError::Serialization`] if `max_payload_size == 0` or JSON serialization fails.
/// Returns [`CoreError::PayloadTooLarge`] if the serialized envelope cannot fit in `u16::MAX` chunks.
pub fn chunk_envelope(env: &Envelope, max_payload_size: usize) -> Result<Vec<BleChunk>> {
    if max_payload_size == 0 {
        return Err(CoreError::Serialization(
            "max_payload_size must be greater than 0".to_string(),
        ));
    }

    let serialized = serde_json::to_vec(env)?;
    let total_len = serialized.len();

    let msg_id = match ulid::Ulid::from_string(&env.id) {
        Ok(u) => u.to_bytes(),
        Err(_) => {
            let hash = blake3::hash(&serialized);
            let mut id = [0u8; 16];
            id.copy_from_slice(&hash.as_bytes()[..16]);
            id
        }
    };

    if total_len == 0 {
        return Ok(vec![BleChunk {
            msg_id,
            total_chunks: 1,
            chunk_index: 0,
            payload: Vec::new(),
        }]);
    }

    let chunk_slices: Vec<&[u8]> = serialized.chunks(max_payload_size).collect();
    if chunk_slices.len() > u16::MAX as usize {
        return Err(CoreError::PayloadTooLarge {
            size: total_len,
            max: max_payload_size.saturating_mul(u16::MAX as usize),
        });
    }

    let total_chunks = chunk_slices.len() as u16;
    let chunks = chunk_slices
        .into_iter()
        .enumerate()
        .map(|(i, slice)| BleChunk {
            msg_id,
            total_chunks,
            chunk_index: i as u16,
            payload: slice.to_vec(),
        })
        .collect();

    Ok(chunks)
}

fn deserialize_envelope(bytes: &[u8]) -> Result<Envelope> {
    if let Ok(env) = serde_json::from_slice::<Envelope>(bytes) {
        return Ok(env);
    }
    ciborium::from_reader::<Envelope, _>(bytes).map_err(|e| {
        CoreError::Decode(format!("failed to deserialize envelope: {e}"))
    })
}

#[derive(Debug)]
struct InflightMessage {
    total_chunks: u16,
    chunks: BTreeMap<u16, Vec<u8>>,
    total_bytes: usize,
    created_at: Instant,
}

/// Bounded cache of incoming BLE message chunks with automatic timeout eviction.
///
/// Reassembles out-of-order chunks into the original serialized [`Envelope`].
/// Protects against memory exhaustion attacks by enforcing both maximum per-message
/// payload limits and maximum concurrent in-flight message limits.
#[derive(Debug)]
pub struct BleReassembler {
    inflight: HashMap<[u8; 16], InflightMessage>,
    completed: HashMap<[u8; 16], Instant>,
    timeout: Duration,
    max_message_size: usize,
    max_inflight: usize,
}

impl BleReassembler {
    /// Creates a new `BleReassembler` with default 30-second timeout, 10 MiB max payload,
    /// and 64 concurrent in-flight message slots.
    pub fn new() -> Self {
        Self {
            inflight: HashMap::new(),
            completed: HashMap::new(),
            timeout: DEFAULT_REASSEMBLY_TIMEOUT,
            max_message_size: MAX_REASSEMBLY_SIZE,
            max_inflight: MAX_INFLIGHT_MESSAGES,
        }
    }

    /// Creates a `BleReassembler` with a custom inactivity timeout.
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            inflight: HashMap::new(),
            completed: HashMap::new(),
            timeout,
            max_message_size: MAX_REASSEMBLY_SIZE,
            max_inflight: MAX_INFLIGHT_MESSAGES,
        }
    }

    /// Creates a `BleReassembler` with custom parameters.
    pub fn with_config(timeout: Duration, max_message_size: usize, max_inflight: usize) -> Self {
        Self {
            inflight: HashMap::new(),
            completed: HashMap::new(),
            timeout,
            max_message_size,
            max_inflight,
        }
    }

    /// Removes all incomplete in-flight messages and completed history entries older than `self.timeout`.
    pub fn prune_expired(&mut self) {
        let now = Instant::now();
        self.inflight
            .retain(|_, msg| now.saturating_duration_since(msg.created_at) < self.timeout);
        self.completed
            .retain(|_, time| now.saturating_duration_since(*time) < self.timeout);
    }

    /// Ingests an incoming [`BleChunk`].
    ///
    /// Chunks may arrive in arbitrary order. Once all `total_chunks` for a given `msg_id`
    /// are received, the serialized [`Envelope`] is reconstructed, deserialized, and returned.
    ///
    /// Duplicate or redundant chunks are ignored gracefully. Incomplete messages are discarded
    /// upon reaching the timeout.
    ///
    /// # Errors
    /// Returns [`CoreError::Decode`] on invalid headers, inconsistent chunk metadata, or deserialization failures.
    /// Returns [`CoreError::PayloadTooLarge`] if the accumulated payload exceeds `max_message_size`.
    pub fn push_chunk(&mut self, chunk: BleChunk) -> Result<Option<Envelope>> {
        self.prune_expired();

        if chunk.total_chunks == 0 {
            return Err(CoreError::Decode(
                "total_chunks must be greater than 0".to_string(),
            ));
        }

        if chunk.chunk_index >= chunk.total_chunks {
            return Err(CoreError::Decode(format!(
                "chunk_index ({}) out of bounds for total_chunks ({})",
                chunk.chunk_index, chunk.total_chunks
            )));
        }

        // If this message was already completed recently, ignore duplicate retransmissions.
        if self.completed.contains_key(&chunk.msg_id) {
            return Ok(None);
        }

        if chunk.payload.len() > self.max_message_size {
            return Err(CoreError::PayloadTooLarge {
                size: chunk.payload.len(),
                max: self.max_message_size,
            });
        }

        // Fast path for single-chunk envelopes
        if chunk.total_chunks == 1 {
            self.inflight.remove(&chunk.msg_id);
            let envelope = deserialize_envelope(&chunk.payload)?;
            self.record_completed(chunk.msg_id);
            return Ok(Some(envelope));
        }

        if let Some(inflight) = self.inflight.get_mut(&chunk.msg_id) {
            if inflight.total_chunks != chunk.total_chunks {
                return Err(CoreError::Decode(format!(
                    "inconsistent total_chunks for msg_id: expected {}, got {}",
                    inflight.total_chunks, chunk.total_chunks
                )));
            }

            // Gracefully ignore duplicate chunks during reassembly
            if inflight.chunks.contains_key(&chunk.chunk_index) {
                return Ok(None);
            }

            let new_total = inflight.total_bytes + chunk.payload.len();
            if new_total > self.max_message_size {
                self.inflight.remove(&chunk.msg_id);
                return Err(CoreError::PayloadTooLarge {
                    size: new_total,
                    max: self.max_message_size,
                });
            }

            inflight.total_bytes = new_total;
            inflight.chunks.insert(chunk.chunk_index, chunk.payload);

            if inflight.chunks.len() == inflight.total_chunks as usize {
                let inflight = self.inflight.remove(&chunk.msg_id).unwrap();
                let mut payload_bytes = Vec::with_capacity(inflight.total_bytes);
                for i in 0..inflight.total_chunks {
                    if let Some(part) = inflight.chunks.get(&i) {
                        payload_bytes.extend_from_slice(part);
                    } else {
                        return Err(CoreError::Decode(format!(
                            "missing chunk {i} during reassembly"
                        )));
                    }
                }
                let envelope = deserialize_envelope(&payload_bytes)?;
                self.record_completed(chunk.msg_id);
                return Ok(Some(envelope));
            }

            Ok(None)
        } else {
            if self.inflight.len() >= self.max_inflight {
                self.evict_oldest();
            }

            let mut chunks = BTreeMap::new();
            let total_bytes = chunk.payload.len();
            let total_chunks = chunk.total_chunks;
            chunks.insert(chunk.chunk_index, chunk.payload);

            self.inflight.insert(
                chunk.msg_id,
                InflightMessage {
                    total_chunks,
                    chunks,
                    total_bytes,
                    created_at: Instant::now(),
                },
            );

            Ok(None)
        }
    }

    /// Number of distinct in-flight messages currently tracked.
    pub fn inflight_count(&self) -> usize {
        self.inflight.len()
    }

    /// Returns `true` if the specified message is currently in-flight.
    pub fn has_inflight(&self, msg_id: &[u8; 16]) -> bool {
        self.inflight.contains_key(msg_id)
    }

    /// Clears all in-flight reassembly buffers and completed message records.
    pub fn clear(&mut self) {
        self.inflight.clear();
        self.completed.clear();
    }

    fn record_completed(&mut self, msg_id: [u8; 16]) {
        if self.completed.len() >= MAX_COMPLETED_CACHE {
            let oldest_key = self
                .completed
                .iter()
                .min_by_key(|(_, time)| **time)
                .map(|(k, _)| *k);
            if let Some(key) = oldest_key {
                self.completed.remove(&key);
            }
        }
        self.completed.insert(msg_id, Instant::now());
    }

    fn evict_oldest(&mut self) {
        let oldest_key = self
            .inflight
            .iter()
            .min_by_key(|(_, msg)| msg.created_at)
            .map(|(k, _)| *k);
        if let Some(key) = oldest_key {
            self.inflight.remove(&key);
        }
    }
}

impl Default for BleReassembler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ClipPayload;

    #[test]
    fn test_beacon_token_determinism_and_uniqueness() {
        let key1 = GroupKey::generate();
        let key2 = GroupKey::generate();

        let token1_a = ble_beacon_token(&key1);
        let token1_b = ble_beacon_token(&key1);
        let token2 = ble_beacon_token(&key2);

        // Deterministic
        assert_eq!(token1_a, token1_b);
        assert_eq!(token1_a.len(), 16);
        assert_ne!(token1_a, [0u8; 16]);

        // Unique across different keys
        assert_ne!(token1_a, token2);

        // Does not match group_id prefix or relay token
        let gid_bytes = hex::decode(key1.group_id()).unwrap();
        assert_ne!(&token1_a[..], &gid_bytes[..16]);
    }

    #[test]
    fn test_ble_chunk_roundtrip() {
        let msg_id = [0x42u8; 16];
        let payload = b"ClipBridge binary framing payload test".to_vec();
        let chunk = BleChunk::new(msg_id, 5, 2, payload.clone());

        let bytes = chunk.to_bytes();
        assert_eq!(bytes.len(), BLE_HEADER_SIZE + payload.len());

        // Verify header layout:
        assert_eq!(&bytes[..16], &msg_id);
        assert_eq!(u16::from_be_bytes([bytes[16], bytes[17]]), 5);
        assert_eq!(u16::from_be_bytes([bytes[18], bytes[19]]), 2);
        assert_eq!(&bytes[20..], payload.as_slice());

        let decoded = BleChunk::from_bytes(&bytes).unwrap();
        assert_eq!(decoded, chunk);
        assert_eq!(decoded.msg_id, msg_id);
        assert_eq!(decoded.total_chunks, 5);
        assert_eq!(decoded.chunk_index, 2);
        assert_eq!(decoded.payload, payload);
    }

    #[test]
    fn test_ble_chunk_empty_payload_roundtrip() {
        let msg_id = [0x01u8; 16];
        let chunk = BleChunk::new(msg_id, 1, 0, Vec::new());
        let bytes = chunk.to_bytes();
        assert_eq!(bytes.len(), BLE_HEADER_SIZE);

        let decoded = BleChunk::from_bytes(&bytes).unwrap();
        assert_eq!(decoded, chunk);
        assert!(decoded.payload.is_empty());
    }

    #[test]
    fn test_ble_chunk_truncated_bytes_rejected() {
        let mut bytes = vec![0u8; 19]; // 1 byte short of 20
        assert!(BleChunk::from_bytes(&bytes).is_err());

        bytes.clear(); // 0 bytes
        assert!(BleChunk::from_bytes(&bytes).is_err());
    }

    #[test]
    fn test_ble_chunk_invalid_header_values() {
        let mut bytes = vec![0u8; 25];

        // total_chunks == 0
        bytes[16] = 0;
        bytes[17] = 0;
        assert!(BleChunk::from_bytes(&bytes).is_err());

        // total_chunks == 3, chunk_index == 3 (out of bounds)
        bytes[16] = 0;
        bytes[17] = 3;
        bytes[18] = 0;
        bytes[19] = 3;
        assert!(BleChunk::from_bytes(&bytes).is_err());

        // total_chunks == 3, chunk_index == 5
        bytes[18] = 0;
        bytes[19] = 5;
        assert!(BleChunk::from_bytes(&bytes).is_err());
    }

    #[test]
    fn test_chunk_envelope_and_reassembler_single_chunk() {
        let key = GroupKey::generate();
        let payload = ClipPayload::text("Single-chunk BLE clipboard test").unwrap();
        let env = Envelope::seal(&key, "sender-mac", &payload).unwrap();

        // 490 byte payload capacity easily accommodates small envelope
        let chunks = chunk_envelope(&env, BLE_CHUNK_PAYLOAD_SIZE).unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].total_chunks, 1);
        assert_eq!(chunks[0].chunk_index, 0);

        let mut reassembler = BleReassembler::new();
        let result = reassembler.push_chunk(chunks[0].clone()).unwrap();
        assert!(result.is_some());

        let reassembled = result.unwrap();
        assert_eq!(reassembled, env);

        let decrypted = reassembled.open(&key).unwrap();
        assert_eq!(decrypted.as_text(), Some("Single-chunk BLE clipboard test"));
    }

    #[test]
    fn test_chunk_envelope_and_reassembler_multi_chunk_reversed() {
        let key = GroupKey::generate();
        // Generate a 12 KB payload to ensure > 20 chunks at 490 bytes/chunk
        let large_text = "ClipBridge BLE chunking payload ".repeat(400);
        let payload = ClipPayload::text(large_text.clone()).unwrap();
        let env = Envelope::seal(&key, "sender-linux", &payload).unwrap();

        let mut chunks = chunk_envelope(&env, BLE_CHUNK_PAYLOAD_SIZE).unwrap();
        assert!(chunks.len() > 20);

        // Reverse chunk arrival order
        chunks.reverse();

        let mut reassembler = BleReassembler::new();
        let total = chunks.len();

        for (i, chunk) in chunks.into_iter().enumerate() {
            let res = reassembler.push_chunk(chunk).unwrap();
            if i + 1 == total {
                // The last pushed chunk completes the reassembly
                assert!(res.is_some());
                let reassembled = res.unwrap();
                assert_eq!(reassembled, env);
                let opened = reassembled.open(&key).unwrap();
                assert_eq!(opened.as_text(), Some(large_text.as_str()));
            } else {
                assert!(res.is_none());
            }
        }
    }

    #[test]
    fn test_chunk_envelope_and_reassembler_random_order() {
        let key = GroupKey::generate();
        let text = "Ble multi-chunk pseudo-random delivery order test ".repeat(150);
        let payload = ClipPayload::text(text.clone()).unwrap();
        let env = Envelope::seal(&key, "sender-ios", &payload).unwrap();

        // Use a smaller chunk payload size (128 bytes) to produce many chunks
        let chunks = chunk_envelope(&env, 128).unwrap();
        assert!(chunks.len() > 40);

        // Permute chunk order deterministically: odds first, then evens
        let mut reordered = Vec::with_capacity(chunks.len());
        for i in (1..chunks.len()).step_by(2) {
            reordered.push(chunks[i].clone());
        }
        for i in (0..chunks.len()).step_by(2) {
            reordered.push(chunks[i].clone());
        }
        assert_eq!(reordered.len(), chunks.len());

        let mut reassembler = BleReassembler::new();
        let total = reordered.len();

        for (i, chunk) in reordered.into_iter().enumerate() {
            let res = reassembler.push_chunk(chunk).unwrap();
            if i + 1 == total {
                assert!(res.is_some());
                let reassembled = res.unwrap();
                assert_eq!(reassembled, env);
                let opened = reassembled.open(&key).unwrap();
                assert_eq!(opened.as_text(), Some(text.as_str()));
            } else {
                assert!(res.is_none());
            }
        }
    }

    #[test]
    fn test_duplicate_chunks_graceful() {
        let key = GroupKey::generate();
        let payload = ClipPayload::text("Duplicate chunk test payload string".repeat(30)).unwrap();
        let env = Envelope::seal(&key, "sender-dup", &payload).unwrap();

        let chunks = chunk_envelope(&env, 200).unwrap();
        assert!(chunks.len() >= 3);

        let mut reassembler = BleReassembler::new();

        // Push chunk 0
        let r0 = reassembler.push_chunk(chunks[0].clone()).unwrap();
        assert!(r0.is_none());

        // Duplicate chunk 0
        let r0_dup = reassembler.push_chunk(chunks[0].clone()).unwrap();
        assert!(r0_dup.is_none());

        // Duplicate chunk 0 again
        let r0_dup2 = reassembler.push_chunk(chunks[0].clone()).unwrap();
        assert!(r0_dup2.is_none());

        // Push remaining chunks
        for chunk in &chunks[1..chunks.len() - 1] {
            assert!(reassembler.push_chunk(chunk.clone()).unwrap().is_none());
            // Duplicate intermediate chunk
            assert!(reassembler.push_chunk(chunk.clone()).unwrap().is_none());
        }

        // Final chunk completes
        let completed = reassembler
            .push_chunk(chunks.last().unwrap().clone())
            .unwrap();
        assert!(completed.is_some());
        assert_eq!(completed.unwrap(), env);

        // Retransmitted chunk after completion is ignored
        let post_complete = reassembler.push_chunk(chunks[0].clone()).unwrap();
        assert!(post_complete.is_none());
    }

    #[test]
    fn test_incomplete_chunks_and_timeout() {
        let key = GroupKey::generate();
        let payload = ClipPayload::text("Timeout incomplete message test".repeat(30)).unwrap();
        let env = Envelope::seal(&key, "sender-timeout", &payload).unwrap();

        let chunks = chunk_envelope(&env, 100).unwrap();
        assert!(chunks.len() > 2);

        // Reassembler with 30ms timeout
        let mut reassembler = BleReassembler::with_timeout(Duration::from_millis(30));

        // Push only the first chunk
        let res = reassembler.push_chunk(chunks[0].clone()).unwrap();
        assert!(res.is_none());
        assert!(reassembler.has_inflight(&chunks[0].msg_id));
        assert_eq!(reassembler.inflight_count(), 1);

        // Wait past timeout
        std::thread::sleep(Duration::from_millis(45));

        // Pruning should evict the stale message
        reassembler.prune_expired();
        assert!(!reassembler.has_inflight(&chunks[0].msg_id));
        assert_eq!(reassembler.inflight_count(), 0);
    }

    #[test]
    fn test_inconsistent_total_chunks_rejected() {
        let msg_id = [0x77u8; 16];
        let chunk1 = BleChunk::new(msg_id, 3, 0, b"part1".to_vec());
        let chunk2 = BleChunk::new(msg_id, 4, 1, b"part2".to_vec());

        let mut reassembler = BleReassembler::new();
        assert!(reassembler.push_chunk(chunk1).unwrap().is_none());

        // Mismatched total_chunks for the same msg_id
        let err = reassembler.push_chunk(chunk2).unwrap_err();
        match err {
            CoreError::Decode(_) => {}
            other => panic!("expected Decode error, got: {other:?}"),
        }
    }

    #[test]
    fn test_payload_too_large_rejection() {
        let mut reassembler = BleReassembler::with_config(Duration::from_secs(30), 200, 10);
        let msg_id = [0x88u8; 16];

        // Individual chunk payload exceeds limit
        let big_chunk = BleChunk::new(msg_id, 1, 0, vec![0u8; 300]);
        let err = reassembler.push_chunk(big_chunk).unwrap_err();
        match err {
            CoreError::PayloadTooLarge { size, max } => {
                assert_eq!(size, 300);
                assert_eq!(max, 200);
            }
            other => panic!("expected PayloadTooLarge, got: {other:?}"),
        }

        // Cumulative chunk payloads exceed limit
        let chunk1 = BleChunk::new(msg_id, 2, 0, vec![0u8; 150]);
        let chunk2 = BleChunk::new(msg_id, 2, 1, vec![0u8; 150]);
        assert!(reassembler.push_chunk(chunk1).unwrap().is_none());

        let err2 = reassembler.push_chunk(chunk2).unwrap_err();
        match err2 {
            CoreError::PayloadTooLarge { size, max } => {
                assert_eq!(size, 300);
                assert_eq!(max, 200);
            }
            other => panic!("expected PayloadTooLarge, got: {other:?}"),
        }
        // Inflight buffer must be purged on overflow
        assert!(!reassembler.has_inflight(&msg_id));
    }

    #[test]
    fn test_chunk_envelope_zero_max_payload_rejected() {
        let key = GroupKey::generate();
        let payload = ClipPayload::text("zero max payload test").unwrap();
        let env = Envelope::seal(&key, "sender", &payload).unwrap();

        let err = chunk_envelope(&env, 0).unwrap_err();
        match err {
            CoreError::Serialization(_) => {}
            other => panic!("expected Serialization error, got: {other:?}"),
        }
    }

    #[test]
    fn test_corrupted_payload_reassembly_failure() {
        let msg_id = [0x99u8; 16];
        let chunk1 = BleChunk::new(msg_id, 2, 0, b"{invalid json:".to_vec());
        let chunk2 = BleChunk::new(msg_id, 2, 1, b"corrupted bytes}".to_vec());

        let mut reassembler = BleReassembler::new();
        assert!(reassembler.push_chunk(chunk1).unwrap().is_none());

        let err = reassembler.push_chunk(chunk2).unwrap_err();
        match err {
            CoreError::Decode(_) => {}
            other => panic!("expected Decode error, got: {other:?}"),
        }
    }

    #[test]
    fn test_constants() {
        assert_eq!(
            CLIPBRIDGE_BLE_SERVICE_UUID,
            "0000cb01-0000-1000-8000-00805f9b34fb"
        );
        assert_eq!(
            CLIPBRIDGE_BLE_CHAR_UUID,
            "0000cb02-0000-1000-8000-00805f9b34fb"
        );
        assert_eq!(BLE_CHUNK_PAYLOAD_SIZE, 490);
        assert_eq!(BLE_HEADER_SIZE, 20);
    }
}
