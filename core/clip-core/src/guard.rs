use std::collections::{HashSet, VecDeque};

/// Bounded queue/set capacity for tracking deduplication state.
pub const ECHO_GUARD_CAPACITY: usize = 256;

/// Guard mechanism preventing clipboard synchronization echo loops and duplicate delivery.
///
/// When an item is received from a remote device and written to the local clipboard, the local
/// clipboard listener fires a change event. `EchoGuard` tracks the last applied hash and the last
/// published hash so local listeners do not echo the item back to the network.
///
/// It also tracks recently observed envelope ULIDs in a bounded cache (256 items) to deduplicate
/// replayed envelopes across reconnection cycles.
#[derive(Debug, Clone)]
pub struct EchoGuard {
    last_applied: Option<Vec<u8>>,
    last_published: Option<Vec<u8>>,
    seen_ids: HashSet<String>,
    seen_order: VecDeque<String>,
}

impl Default for EchoGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl EchoGuard {
    /// Constructs a new `EchoGuard` with empty state.
    pub fn new() -> Self {
        Self {
            last_applied: None,
            last_published: None,
            seen_ids: HashSet::with_capacity(ECHO_GUARD_CAPACITY),
            seen_order: VecDeque::with_capacity(ECHO_GUARD_CAPACITY),
        }
    }

    /// Records that `hash` was written to the local clipboard from a remote device.
    ///
    /// Subsequent clipboard read events matching this hash will be suppressed from publishing.
    pub fn mark_applied(&mut self, hash: &[u8]) {
        self.last_applied = Some(hash.to_vec());
    }

    /// Determines whether content with `hash` should be broadcast to the group.
    ///
    /// Returns `false` if `hash` equals either the last applied remote hash or the last published
    /// local hash. Otherwise, records `hash` as `last_published` and returns `true`.
    pub fn should_publish(&mut self, hash: &[u8]) -> bool {
        if self.last_applied.as_deref() == Some(hash) || self.last_published.as_deref() == Some(hash) {
            return false;
        }
        self.last_published = Some(hash.to_vec());
        true
    }

    /// Checks if envelope `id` has already been seen.
    ///
    /// Returns `true` if the ID was already recorded. Otherwise, stores the ID in the bounded
    /// cache (evicting the oldest entry when capacity 256 is reached) and returns `false`.
    pub fn seen_envelope(&mut self, id: &str) -> bool {
        if self.seen_ids.contains(id) {
            return true;
        }

        if self.seen_order.len() >= ECHO_GUARD_CAPACITY {
            if let Some(oldest) = self.seen_order.pop_front() {
                self.seen_ids.remove(&oldest);
            }
        }

        self.seen_order.push_back(id.to_string());
        self.seen_ids.insert(id.to_string());
        false
    }

    /// Returns the last hash applied to the clipboard from a remote event, if any.
    pub fn last_applied(&self) -> Option<&[u8]> {
        self.last_applied.as_deref()
    }

    /// Returns the last hash published by this device, if any.
    pub fn last_published(&self) -> Option<&[u8]> {
        self.last_published.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_echo_prevention_on_applied() {
        let mut guard = EchoGuard::new();
        let hash = [1u8; 32];

        guard.mark_applied(&hash);
        // The local clipboard watcher detects the change, but should NOT publish
        assert!(!guard.should_publish(&hash));

        // Another new local copy SHOULD publish
        let new_hash = [2u8; 32];
        assert!(guard.should_publish(&new_hash));

        // Re-copying the same new hash immediately should NOT publish
        assert!(!guard.should_publish(&new_hash));
    }

    #[test]
    fn test_seen_envelope_deduplication_and_bounded_eviction() {
        let mut guard = EchoGuard::new();

        assert!(!guard.seen_envelope("env-1"));
        assert!(guard.seen_envelope("env-1"));

        // Fill capacity (256 entries)
        for i in 0..ECHO_GUARD_CAPACITY {
            let id = format!("bulk-{i}");
            assert!(!guard.seen_envelope(&id));
        }

        // env-1 should now be evicted
        assert!(!guard.seen_envelope("env-1"));
        assert!(guard.seen_envelope("env-1"));
    }
}
