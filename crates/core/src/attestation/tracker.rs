//! In-memory attendance tracker for minute attestations received via gossip.

use std::collections::{HashMap, HashSet};

use crypto::PublicKey;

/// In-memory tracker for minute attestations received via gossip.
///
/// Used by the block producer to build the bitfield for `presence_root`.
/// NOT used for epoch reward qualification — that comes from scanning on-chain bitfields.
pub struct MinuteAttestationTracker {
    /// pubkey → set of minutes they attested in
    attested: HashMap<PublicKey, HashSet<u32>>,
}

impl MinuteAttestationTracker {
    /// Create an empty tracker.
    pub fn new() -> Self {
        Self {
            attested: HashMap::new(),
        }
    }

    /// Record that a producer attested in a given minute.
    pub fn record(&mut self, pubkey: PublicKey, minute: u32) {
        self.attested.entry(pubkey).or_default().insert(minute);
    }

    /// Deterministic fingerprint of the attested map for cross-node
    /// divergence detection. Sorts by pubkey then by minute to eliminate
    /// HashMap/HashSet iteration-order variance, then hashes.
    /// Returns zeroed Hash if internally empty.
    pub fn fingerprint(&self) -> crypto::Hash {
        let mut entries: Vec<(Vec<u8>, Vec<u32>)> = self
            .attested
            .iter()
            .map(|(pk, mins)| {
                let mut m: Vec<u32> = mins.iter().copied().collect();
                m.sort();
                (pk.as_bytes().to_vec(), m)
            })
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        crypto::hash::hash(&bincode::serialize(&entries).unwrap_or_default())
    }

    /// Total count of (pubkey, minute) entries.
    pub fn total_entries(&self) -> usize {
        self.attested.values().map(|m| m.len()).sum()
    }

    /// Get all producers that attested in a specific minute.
    pub fn attested_in_minute(&self, minute: u32) -> Vec<&PublicKey> {
        self.attested
            .iter()
            .filter(|(_, minutes)| minutes.contains(&minute))
            .map(|(pk, _)| pk)
            .collect()
    }

    /// Reset the tracker (at epoch boundary).
    pub fn reset(&mut self) {
        self.attested.clear();
    }
}

impl Default for MinuteAttestationTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tracker_basic() {
        let mut tracker = MinuteAttestationTracker::new();
        let pk = PublicKey::from_bytes([1u8; 32]);

        tracker.record(pk, 5);
        tracker.record(pk, 6);

        let minute_5 = tracker.attested_in_minute(5);
        assert_eq!(minute_5.len(), 1);
        assert_eq!(*minute_5[0], pk);

        let minute_7 = tracker.attested_in_minute(7);
        assert!(minute_7.is_empty());
    }

    #[test]
    fn test_tracker_multiple_producers() {
        let mut tracker = MinuteAttestationTracker::new();
        let pk1 = PublicKey::from_bytes([1u8; 32]);
        let pk2 = PublicKey::from_bytes([2u8; 32]);

        tracker.record(pk1, 5);
        tracker.record(pk2, 5);
        tracker.record(pk1, 6);

        assert_eq!(tracker.attested_in_minute(5).len(), 2);
        assert_eq!(tracker.attested_in_minute(6).len(), 1);
    }

    #[test]
    fn test_tracker_reset() {
        let mut tracker = MinuteAttestationTracker::new();
        let pk = PublicKey::from_bytes([1u8; 32]);

        tracker.record(pk, 5);
        assert_eq!(tracker.attested_in_minute(5).len(), 1);

        tracker.reset();
        assert!(tracker.attested_in_minute(5).is_empty());
    }
}
