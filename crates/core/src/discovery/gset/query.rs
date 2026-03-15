//! Query and export operations for ProducerGSet.
//!
//! Contains methods for retrieving, filtering, and exporting producer
//! announcements, as well as bloom filter and delta sync operations.

use std::time::Instant;

use crypto::PublicKey;
use tracing::info;

use super::super::{ProducerAnnouncement, ProducerBloomFilter, MAX_ANNOUNCEMENT_AGE_SECS};
use super::ProducerGSet;

impl ProducerGSet {
    /// Get a sorted list of all known producer public keys.
    ///
    /// The list is sorted by the byte representation of the public keys
    /// to ensure deterministic ordering across all nodes.
    #[must_use]
    pub fn sorted_producers(&self) -> Vec<PublicKey> {
        let mut producers: Vec<PublicKey> = self.producers.keys().copied().collect();
        producers.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        producers
    }

    /// Get sorted producers filtered by announcement freshness.
    ///
    /// Only includes producers whose announcement timestamp is within
    /// `max_age_secs` of the current time. This prevents ghost producers
    /// (announced once but now offline) from inflating the round-robin denominator.
    #[must_use]
    pub fn active_producers(&self, max_age_secs: u64) -> Vec<PublicKey> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut producers: Vec<PublicKey> = self
            .producers
            .iter()
            .filter(|(_, ann)| ann.timestamp + max_age_secs >= now)
            .map(|(pk, _)| *pk)
            .collect();
        producers.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        producers
    }

    /// Export non-stale announcements from the set.
    ///
    /// Filters out announcements older than `MAX_ANNOUNCEMENT_AGE_SECS` to prevent
    /// relaying stale entries that receiving nodes would reject. This ensures CRDT
    /// convergence after restarts where persisted announcements have old timestamps.
    #[must_use]
    pub fn export(&self) -> Vec<ProducerAnnouncement> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let total = self.producers.len();
        let result: Vec<ProducerAnnouncement> = self
            .producers
            .values()
            .filter(|ann| ann.timestamp + MAX_ANNOUNCEMENT_AGE_SECS >= now)
            .cloned()
            .collect();
        let exported_keys: Vec<String> = result
            .iter()
            .map(|ann| hex::encode(&ann.pubkey.as_bytes()[..8]))
            .collect();
        let filtered_keys: Vec<String> = self
            .producers
            .values()
            .filter(|ann| ann.timestamp + MAX_ANNOUNCEMENT_AGE_SECS < now)
            .map(|ann| {
                format!(
                    "{}(age={}s)",
                    hex::encode(&ann.pubkey.as_bytes()[..8]),
                    now.saturating_sub(ann.timestamp)
                )
            })
            .collect();
        info!(
            "GSet export: total={} exported={} keys={:?} filtered_out={:?}",
            total,
            result.len(),
            exported_keys,
            filtered_keys
        );
        result
    }

    /// Export all announcements regardless of age.
    ///
    /// Use this when you need the full set (e.g., for diagnostics).
    #[must_use]
    pub fn export_all(&self) -> Vec<ProducerAnnouncement> {
        self.producers.values().cloned().collect()
    }

    /// Remove entries whose announcement timestamp is older than `max_age_secs`.
    ///
    /// Unlike `active_producers()` which only filters the view, this permanently
    /// removes stale entries from the set and their sequence numbers. This prevents
    /// ghost producers (nodes that announced once but are long gone) from persisting
    /// indefinitely and inflating the `slot % n` round-robin denominator.
    ///
    /// Returns the number of entries purged.
    pub fn purge_stale(&mut self, max_age_secs: u64) -> usize {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let stale_keys: Vec<PublicKey> = self
            .producers
            .iter()
            .filter(|(_, ann)| ann.timestamp + max_age_secs < now)
            .map(|(pk, _)| *pk)
            .collect();
        let count = stale_keys.len();
        for pk in &stale_keys {
            self.producers.remove(pk);
            self.sequences.remove(pk);
        }
        if count > 0 {
            self.last_modified = Instant::now();
        }
        count
    }

    /// Create a bloom filter containing all known producers.
    ///
    /// This is useful for efficient delta synchronization with peers.
    /// The bloom filter can be sent to peers who will respond with
    /// only the announcements not in the filter.
    #[must_use]
    pub fn to_bloom_filter(&self) -> ProducerBloomFilter {
        let expected = self.producers.len().max(1);
        let mut bloom = ProducerBloomFilter::new(expected);

        for pubkey in self.producers.keys() {
            bloom.insert(pubkey);
        }

        bloom
    }

    /// Get announcements that the peer probably doesn't have.
    ///
    /// This compares our set against the peer's bloom filter and returns
    /// announcements for producers that are not in the peer's filter.
    ///
    /// Note: Due to bloom filter false positives, this may not return
    /// all announcements the peer is missing, but it will never return
    /// announcements the peer definitely has.
    ///
    /// # Arguments
    ///
    /// * `peer_bloom` - The peer's bloom filter of known producers
    #[must_use]
    pub fn delta_for_peer(&self, peer_bloom: &ProducerBloomFilter) -> Vec<ProducerAnnouncement> {
        self.producers
            .iter()
            .filter(|(pubkey, _)| !peer_bloom.probably_contains(pubkey))
            .map(|(_, ann)| ann.clone())
            .collect()
    }
}
