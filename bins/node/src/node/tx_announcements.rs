//! Pending transaction announcement tracker.
//!
//! Tracks tx hashes we've heard about via announcements but don't yet have in
//! the mempool. Groups missing hashes by announcing peer for efficient batch
//! fetching via the txfetch request-response protocol.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use crypto::Hash;
use network::PeerId;

/// Maximum number of pending announcements to track (memory bound).
#[allow(dead_code)]
const MAX_PENDING: usize = 10_000;

/// Maximum hashes per txfetch request.
#[allow(dead_code)]
const MAX_FETCH_BATCH: usize = 50;

/// Tracks transaction hashes announced by peers that we don't yet have.
/// Currently unused — network_events.rs uses plain HashMap for simplicity.
/// Will be wired up when txfetch protocol is fully integrated.
#[allow(dead_code)]
pub struct PendingTxAnnouncements {
    /// hash → (announcing peers, first_seen)
    pending: HashMap<Hash, (Vec<PeerId>, Instant)>,
    /// Hashes we've already sent a fetch request for (avoid duplicate requests)
    #[allow(dead_code)]
    requested: HashSet<Hash>,
}

#[allow(dead_code)]
impl PendingTxAnnouncements {
    pub fn new() -> Self {
        Self {
            pending: HashMap::new(),
            requested: HashSet::new(),
        }
    }

    /// Record an announcement: peer `peer_id` claims to have transaction `hash`.
    /// Returns true if this is a new hash we haven't seen before.
    pub fn record(&mut self, hash: Hash, peer_id: PeerId) -> bool {
        // Don't track if already requested
        if self.requested.contains(&hash) {
            // Still record the peer as a backup announcer
            if let Some((peers, _)) = self.pending.get_mut(&hash) {
                if !peers.contains(&peer_id) {
                    peers.push(peer_id);
                }
            }
            return false;
        }

        if let Some((peers, _)) = self.pending.get_mut(&hash) {
            if !peers.contains(&peer_id) {
                peers.push(peer_id);
            }
            false
        } else {
            // Enforce capacity limit
            if self.pending.len() >= MAX_PENDING {
                return false;
            }
            self.pending.insert(hash, (vec![peer_id], Instant::now()));
            true
        }
    }

    /// Take a batch of unrequested hashes grouped by their first announcing peer.
    /// Returns Vec<(peer_id, hashes_to_request)> — each inner vec is at most MAX_FETCH_BATCH.
    /// Marks returned hashes as requested.
    pub fn take_batch(&mut self) -> Vec<(PeerId, Vec<Hash>)> {
        let mut by_peer: HashMap<PeerId, Vec<Hash>> = HashMap::new();

        for (hash, (peers, _)) in &self.pending {
            if self.requested.contains(hash) {
                continue;
            }
            if let Some(peer) = peers.first() {
                let entry = by_peer.entry(*peer).or_default();
                if entry.len() < MAX_FETCH_BATCH {
                    entry.push(*hash);
                }
            }
        }

        let mut result = Vec::new();
        for (peer, hashes) in by_peer {
            for hash in &hashes {
                self.requested.insert(*hash);
            }
            result.push((peer, hashes));
        }
        result
    }

    /// Mark a hash as completed (transaction received). Removes from pending and requested.
    pub fn complete(&mut self, hash: &Hash) {
        self.pending.remove(hash);
        self.requested.remove(hash);
    }

    /// Expire stale entries older than `max_age`. Cleans up both pending and requested.
    pub fn expire_old(&mut self, max_age: Duration) {
        let now = Instant::now();
        self.pending.retain(|hash, (_, first_seen)| {
            if now.duration_since(*first_seen) > max_age {
                self.requested.remove(hash);
                false
            } else {
                true
            }
        });
        // Also clean requested entries that have no pending entry
        self.requested.retain(|h| self.pending.contains_key(h));
    }

    /// Number of pending announcements.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.pending.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::Hash;

    fn make_hash(val: u8) -> Hash {
        let mut bytes = [0u8; 32];
        bytes[0] = val;
        Hash::from(bytes)
    }

    fn make_peer() -> PeerId {
        // Use network's re-exported PeerId::random()
        PeerId::random()
    }

    #[test]
    fn test_record_and_take_batch() {
        let mut tracker = PendingTxAnnouncements::new();
        let peer = make_peer();
        let h1 = make_hash(1);
        let h2 = make_hash(2);

        assert!(tracker.record(h1, peer));
        assert!(tracker.record(h2, peer));
        // Duplicate record returns false
        assert!(!tracker.record(h1, peer));

        let batches = tracker.take_batch();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].0, peer);
        assert_eq!(batches[0].1.len(), 2);

        // After take_batch, hashes are marked as requested — take_batch returns empty
        let batches2 = tracker.take_batch();
        assert!(batches2.is_empty() || batches2.iter().all(|(_, h)| h.is_empty()));
    }

    #[test]
    fn test_complete_removes_entry() {
        let mut tracker = PendingTxAnnouncements::new();
        let peer = make_peer();
        let h1 = make_hash(1);

        tracker.record(h1, peer);
        tracker.take_batch();
        tracker.complete(&h1);

        assert_eq!(tracker.len(), 0);
    }

    #[test]
    fn test_capacity_limit() {
        let mut tracker = PendingTxAnnouncements::new();
        let peer = make_peer();

        // Fill to capacity
        for i in 0..MAX_PENDING {
            let mut bytes = [0u8; 32];
            bytes[0..4].copy_from_slice(&(i as u32).to_le_bytes());
            let hash = Hash::from(bytes);
            tracker.record(hash, peer);
        }

        // Next record should be rejected
        let overflow_hash = make_hash(255);
        assert!(!tracker.record(overflow_hash, peer));
        assert_eq!(tracker.len(), MAX_PENDING);
    }
}
