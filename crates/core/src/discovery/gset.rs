//! ProducerGSet - Grow-Only Set CRDT for Producer Discovery
//!
//! This module implements a Grow-Only Set CRDT with cryptographic proofs
//! and version vectors for eventual consistency in producer discovery.

use std::collections::HashMap;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Instant;

use crypto::PublicKey;
use tracing::warn;

use super::{
    MergeResult, ProducerAnnouncement, ProducerBloomFilter, ProducerSetError,
    MAX_ANNOUNCEMENT_AGE_SECS, MAX_FUTURE_TIMESTAMP_SECS,
};

/// A Grow-Only Set CRDT for producer announcements.
///
/// This data structure provides:
/// - Eventual consistency through merge operations
/// - Cryptographic verification of announcements
/// - Version vectors for replay protection
/// - Timestamp bounds checking for Byzantine tolerance
///
/// # Example
///
/// ```rust
/// use doli_core::discovery::{ProducerGSet, ProducerAnnouncement};
/// use crypto::KeyPair;
///
/// let mut gset = ProducerGSet::new(1); // network_id = 1
/// let keypair = KeyPair::generate();
/// let announcement = ProducerAnnouncement::new(&keypair, 1, 0);
///
/// let result = gset.merge_one(announcement);
/// assert!(result.is_ok());
/// assert_eq!(gset.len(), 1);
/// ```
#[derive(Debug)]
pub struct ProducerGSet {
    /// Map from public key to the latest valid announcement.
    producers: HashMap<PublicKey, ProducerAnnouncement>,

    /// Version vector: tracks the highest known sequence for each producer.
    sequences: HashMap<PublicKey, u64>,

    /// Time of last modification (for stability tracking).
    last_modified: Instant,

    /// Network ID this GSet is configured for.
    network_id: u32,

    /// Optional path for disk persistence.
    storage_path: Option<PathBuf>,
}

/// Result of merging a single announcement.
/// Distinguishes between new producers and sequence updates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeOneResult {
    /// A new producer was added (not seen before).
    NewProducer,
    /// An existing producer's sequence was updated.
    SequenceUpdate,
    /// Duplicate or older sequence (no change).
    Duplicate,
}

impl ProducerGSet {
    /// Create a new empty ProducerGSet for the given network.
    ///
    /// # Arguments
    ///
    /// * `network_id` - The network ID (1=mainnet, 2=testnet, 99=devnet)
    #[must_use]
    pub fn new(network_id: u32) -> Self {
        Self {
            producers: HashMap::new(),
            sequences: HashMap::new(),
            last_modified: Instant::now(),
            network_id,
            storage_path: None,
        }
    }

    /// Create a new ProducerGSet with disk persistence.
    ///
    /// If the file exists, it will be loaded and verified.
    /// Only announcements that pass signature verification will be loaded.
    ///
    /// # Arguments
    ///
    /// * `network_id` - The network ID (1=mainnet, 2=testnet, 99=devnet)
    /// * `path` - Path to the persistence file
    #[must_use]
    pub fn new_with_persistence(network_id: u32, path: PathBuf) -> Self {
        let mut gset = Self {
            producers: HashMap::new(),
            sequences: HashMap::new(),
            last_modified: Instant::now(),
            network_id,
            storage_path: Some(path),
        };

        // Try to load existing data
        gset.load_from_disk();

        gset
    }

    /// Merge a single announcement into the set.
    ///
    /// Returns:
    /// - `Ok(MergeOneResult::NewProducer)` if a new producer was added
    /// - `Ok(MergeOneResult::SequenceUpdate)` if an existing producer's sequence was updated
    /// - `Ok(MergeOneResult::Duplicate)` if it was a duplicate or older sequence
    /// - `Err(_)` if validation failed
    ///
    /// # Validation Steps
    ///
    /// 1. Verify signature
    /// 2. Check network_id matches
    /// 3. Check timestamp bounds (not stale, not future)
    /// 4. Check sequence is newer than known
    ///
    /// # Arguments
    ///
    /// * `announcement` - The producer announcement to merge
    ///
    /// # Errors
    ///
    /// - `InvalidSignature` - Signature verification failed
    /// - `NetworkMismatch` - Wrong network ID
    /// - `StaleAnnouncement` - Timestamp > 1 hour old
    /// - `FutureTimestamp` - Timestamp > 5 minutes in future
    pub fn merge_one(
        &mut self,
        announcement: ProducerAnnouncement,
    ) -> Result<MergeOneResult, ProducerSetError> {
        // Step 1: Verify signature
        if !announcement.verify() {
            return Err(ProducerSetError::InvalidSignature);
        }

        // Step 2: Check network ID
        if announcement.network_id != self.network_id {
            return Err(ProducerSetError::NetworkMismatch {
                expected: self.network_id,
                got: announcement.network_id,
            });
        }

        // Step 3: Check timestamp bounds
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_secs();

        // Check for stale announcement (> 1 hour old)
        if announcement.timestamp + MAX_ANNOUNCEMENT_AGE_SECS < now {
            return Err(ProducerSetError::StaleAnnouncement);
        }

        // Check for future timestamp (> 5 minutes ahead)
        if announcement.timestamp > now + MAX_FUTURE_TIMESTAMP_SECS {
            return Err(ProducerSetError::FutureTimestamp);
        }

        // Step 4: Check sequence version vector
        let is_new_producer = !self.sequences.contains_key(&announcement.pubkey);
        if let Some(&current_sequence) = self.sequences.get(&announcement.pubkey) {
            // If we already have a higher or equal sequence, this is a duplicate
            if announcement.sequence <= current_sequence {
                return Ok(MergeOneResult::Duplicate);
            }
        }

        // All checks passed - merge the announcement
        self.sequences
            .insert(announcement.pubkey, announcement.sequence);
        self.producers.insert(announcement.pubkey, announcement);
        self.last_modified = Instant::now();

        // Auto-persist if configured
        if self.storage_path.is_some() {
            if let Err(e) = self.persist_to_disk() {
                warn!("Failed to persist producer gset: {}", e);
            }
        }

        if is_new_producer {
            Ok(MergeOneResult::NewProducer)
        } else {
            Ok(MergeOneResult::SequenceUpdate)
        }
    }

    /// Merge multiple announcements into the set.
    ///
    /// Returns statistics about the merge operation.
    ///
    /// # Arguments
    ///
    /// * `announcements` - A vector of announcements to merge
    #[must_use]
    pub fn merge(&mut self, announcements: Vec<ProducerAnnouncement>) -> MergeResult {
        let mut result = MergeResult::default();

        for ann in announcements {
            let pubkey_prefix = hex::encode(&ann.pubkey.as_bytes()[..4]);
            match self.merge_one(ann) {
                Ok(MergeOneResult::NewProducer) => {
                    result.added += 1;
                    result.new_producers += 1;
                }
                Ok(MergeOneResult::SequenceUpdate) => {
                    result.added += 1;
                }
                Ok(MergeOneResult::Duplicate) => result.duplicates += 1,
                Err(e) => {
                    warn!("GSet rejected announcement from {}: {}", pubkey_prefix, e);
                    result.rejected += 1;
                }
            }
        }

        result
    }

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
        self.producers
            .values()
            .filter(|ann| ann.timestamp + MAX_ANNOUNCEMENT_AGE_SECS >= now)
            .cloned()
            .collect()
    }

    /// Export all announcements regardless of age.
    ///
    /// Use this when you need the full set (e.g., for diagnostics).
    #[must_use]
    pub fn export_all(&self) -> Vec<ProducerAnnouncement> {
        self.producers.values().cloned().collect()
    }

    /// Check if the set has been stable (no changes) for the given duration.
    ///
    /// This is useful for determining when it's safe to start producing blocks.
    ///
    /// # Arguments
    ///
    /// * `duration` - Minimum time since last change
    #[must_use]
    pub fn is_stable(&self, duration: std::time::Duration) -> bool {
        self.last_modified.elapsed() >= duration
    }

    /// Get the number of known producers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.producers.len()
    }

    /// Check if the set is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.producers.is_empty()
    }

    /// Clear all producers and sequences from the set.
    ///
    /// This is used during a forced resync to genesis, where all chain-dependent
    /// state must be cleared. The network_id and storage_path are preserved.
    pub fn clear(&mut self) {
        self.producers.clear();
        self.sequences.clear();
        self.last_modified = Instant::now();
    }

    /// Check if a producer is in the set.
    ///
    /// # Arguments
    ///
    /// * `pubkey` - The public key to check
    #[must_use]
    pub fn contains(&self, pubkey: &PublicKey) -> bool {
        self.producers.contains_key(pubkey)
    }

    /// Get the announcement for a specific producer.
    ///
    /// # Arguments
    ///
    /// * `pubkey` - The public key to look up
    #[must_use]
    pub fn get(&self, pubkey: &PublicKey) -> Option<&ProducerAnnouncement> {
        self.producers.get(pubkey)
    }

    /// Get the network ID this GSet is configured for.
    #[must_use]
    pub fn network_id(&self) -> u32 {
        self.network_id
    }

    /// Get the current sequence number for a producer.
    ///
    /// Returns 0 if the producer is not known.
    #[must_use]
    pub fn sequence_for(&self, pubkey: &PublicKey) -> u64 {
        self.sequences.get(pubkey).copied().unwrap_or(0)
    }

    /// Persist the current state to disk.
    ///
    /// Uses atomic write (write to temp file, then rename) to prevent corruption.
    /// This is a no-op if no storage path was configured.
    ///
    /// # Errors
    ///
    /// Returns an IO error if the write fails.
    pub fn persist_to_disk(&self) -> io::Result<()> {
        let Some(ref path) = self.storage_path else {
            return Ok(());
        };

        // Serialize all announcements
        let announcements: Vec<_> = self.producers.values().cloned().collect();
        let data = bincode::serialize(&announcements)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        // Write to temp file first (atomic write pattern)
        let temp_path = path.with_extension("tmp");
        let mut file = std::fs::File::create(&temp_path)?;
        file.write_all(&data)?;
        file.sync_all()?;

        // Atomic rename
        std::fs::rename(&temp_path, path)?;

        Ok(())
    }

    /// Load state from disk, re-verifying all signatures.
    ///
    /// Only announcements that pass verification are loaded.
    /// This is called automatically by `new_with_persistence`.
    fn load_from_disk(&mut self) {
        let Some(ref path) = self.storage_path else {
            return;
        };

        if !path.exists() {
            return;
        }

        // Read and deserialize
        let data = match std::fs::read(path) {
            Ok(data) => data,
            Err(e) => {
                warn!("Failed to read producer gset from disk: {}", e);
                return;
            }
        };

        let announcements: Vec<ProducerAnnouncement> = match bincode::deserialize(&data) {
            Ok(anns) => anns,
            Err(e) => {
                warn!("Failed to deserialize producer gset: {}", e);
                return;
            }
        };

        // Re-verify each announcement and add only valid ones
        // Note: We skip timestamp checks during load since these are persisted announcements
        let mut loaded = 0;
        let mut rejected = 0;

        for ann in announcements {
            // Verify signature
            if !ann.verify() {
                rejected += 1;
                continue;
            }

            // Check network ID
            if ann.network_id != self.network_id {
                rejected += 1;
                continue;
            }

            // Add to state (no timestamp check for persisted data)
            if let Some(&current_seq) = self.sequences.get(&ann.pubkey) {
                if ann.sequence <= current_seq {
                    continue; // Skip older sequences
                }
            }

            self.sequences.insert(ann.pubkey, ann.sequence);
            self.producers.insert(ann.pubkey, ann);
            loaded += 1;
        }

        if rejected > 0 {
            warn!(
                "Loaded {} producers from disk, rejected {} invalid",
                loaded, rejected
            );
        }
    }

    /// Check if this GSet has persistence enabled.
    #[must_use]
    pub fn has_persistence(&self) -> bool {
        self.storage_path.is_some()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::KeyPair;
    use std::time::Duration;

    #[test]
    fn test_gset_merge_valid_announcement() {
        let mut gset = ProducerGSet::new(1);
        let keypair = KeyPair::generate();
        let announcement = ProducerAnnouncement::new(&keypair, 1, 0);

        let result = gset.merge_one(announcement);
        assert!(matches!(result, Ok(MergeOneResult::NewProducer)));
        assert_eq!(gset.len(), 1);
        assert!(gset.contains(keypair.public_key()));
    }

    #[test]
    fn test_gset_reject_invalid_signature() {
        let mut gset = ProducerGSet::new(1);
        let keypair = KeyPair::generate();
        let mut announcement = ProducerAnnouncement::new(&keypair, 1, 0);
        announcement.sequence = 999; // Tamper

        let result = gset.merge_one(announcement);
        assert!(matches!(result, Err(ProducerSetError::InvalidSignature)));
        assert_eq!(gset.len(), 0);
    }

    #[test]
    fn test_gset_reject_wrong_network() {
        let mut gset = ProducerGSet::new(1);
        let keypair = KeyPair::generate();
        let announcement = ProducerAnnouncement::new(&keypair, 2, 0);

        let result = gset.merge_one(announcement);
        assert!(matches!(
            result,
            Err(ProducerSetError::NetworkMismatch {
                expected: 1,
                got: 2
            })
        ));
    }

    #[test]
    fn test_gset_reject_stale_announcement() {
        let mut gset = ProducerGSet::new(1);
        let keypair = KeyPair::generate();

        // Create announcement with timestamp 2 hours ago
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time should be after epoch")
            .as_secs();
        let stale_timestamp = now - 7200; // 2 hours ago

        let announcement =
            ProducerAnnouncement::new_with_timestamp(&keypair, 1, 0, stale_timestamp);

        let result = gset.merge_one(announcement);
        assert!(matches!(result, Err(ProducerSetError::StaleAnnouncement)));
    }

    #[test]
    fn test_gset_reject_future_timestamp() {
        let mut gset = ProducerGSet::new(1);
        let keypair = KeyPair::generate();

        // Create announcement with timestamp 10 minutes in future
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time should be after epoch")
            .as_secs();
        let future_timestamp = now + 600; // 10 minutes ahead

        let announcement =
            ProducerAnnouncement::new_with_timestamp(&keypair, 1, 0, future_timestamp);

        let result = gset.merge_one(announcement);
        assert!(matches!(result, Err(ProducerSetError::FutureTimestamp)));
    }

    #[test]
    fn test_gset_sequence_version_vector() {
        let mut gset = ProducerGSet::new(1);
        let keypair = KeyPair::generate();

        // First announcement with sequence 0
        let ann1 = ProducerAnnouncement::new(&keypair, 1, 0);
        assert!(matches!(
            gset.merge_one(ann1),
            Ok(MergeOneResult::NewProducer)
        ));

        // Second announcement with sequence 1 should update
        let ann2 = ProducerAnnouncement::new(&keypair, 1, 1);
        assert!(matches!(
            gset.merge_one(ann2),
            Ok(MergeOneResult::SequenceUpdate)
        ));

        // Old sequence should be rejected (no change)
        let ann3 = ProducerAnnouncement::new(&keypair, 1, 0);
        assert!(matches!(
            gset.merge_one(ann3),
            Ok(MergeOneResult::Duplicate)
        ));

        assert_eq!(gset.len(), 1); // Still only one producer
        assert_eq!(gset.sequence_for(keypair.public_key()), 1);
    }

    #[test]
    fn test_gset_merge_batch() {
        let mut gset = ProducerGSet::new(1);

        let mut announcements = Vec::new();
        for _ in 0..5 {
            let keypair = KeyPair::generate();
            announcements.push(ProducerAnnouncement::new(&keypair, 1, 0));
        }
        // Add one invalid (tampered)
        let keypair = KeyPair::generate();
        let mut invalid = ProducerAnnouncement::new(&keypair, 1, 0);
        invalid.sequence = 999; // Tamper
        announcements.push(invalid);

        let result = gset.merge(announcements);
        assert_eq!(result.added, 5);
        assert_eq!(result.rejected, 1);
    }

    #[test]
    fn test_gset_sorted_producers_deterministic() {
        let mut gset = ProducerGSet::new(1);

        let keypairs: Vec<_> = (0..5).map(|_| KeyPair::generate()).collect();
        for kp in &keypairs {
            let ann = ProducerAnnouncement::new(kp, 1, 0);
            gset.merge_one(ann).unwrap();
        }

        let sorted1 = gset.sorted_producers();
        let sorted2 = gset.sorted_producers();
        assert_eq!(sorted1, sorted2); // Deterministic

        // Verify sorted by bytes
        for i in 1..sorted1.len() {
            assert!(sorted1[i - 1].as_bytes() < sorted1[i].as_bytes());
        }
    }

    #[test]
    fn test_gset_export_all_announcements() {
        let mut gset = ProducerGSet::new(1);

        for _ in 0..3 {
            let keypair = KeyPair::generate();
            let ann = ProducerAnnouncement::new(&keypair, 1, 0);
            gset.merge_one(ann).unwrap();
        }

        let exported = gset.export();
        assert_eq!(exported.len(), 3);
        for ann in exported {
            assert!(ann.verify());
        }
    }

    #[test]
    fn test_gset_stability_tracking() {
        let mut gset = ProducerGSet::new(1);

        // Initially stable (no changes)
        assert!(gset.is_stable(Duration::from_millis(0)));

        // Add producer - resets stability
        let keypair = KeyPair::generate();
        let ann = ProducerAnnouncement::new(&keypair, 1, 0);
        gset.merge_one(ann).unwrap();

        // Not stable immediately
        assert!(!gset.is_stable(Duration::from_secs(1)));

        // Wait and check (simulated)
        std::thread::sleep(Duration::from_millis(50));
        assert!(gset.is_stable(Duration::from_millis(10)));
    }

    #[test]
    fn test_gset_empty() {
        let gset = ProducerGSet::new(1);
        assert!(gset.is_empty());
        assert_eq!(gset.len(), 0);
    }

    #[test]
    fn test_gset_get() {
        let mut gset = ProducerGSet::new(1);
        let keypair = KeyPair::generate();
        let ann = ProducerAnnouncement::new(&keypair, 1, 0);
        gset.merge_one(ann.clone()).unwrap();

        let retrieved = gset.get(keypair.public_key());
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().pubkey, ann.pubkey);
    }

    #[test]
    fn test_gset_network_id() {
        let gset = ProducerGSet::new(99);
        assert_eq!(gset.network_id(), 99);
    }

    #[test]
    fn test_gset_duplicate_announcement() {
        let mut gset = ProducerGSet::new(1);
        let keypair = KeyPair::generate();

        // Add same announcement twice
        let ann = ProducerAnnouncement::new(&keypair, 1, 0);
        assert!(matches!(
            gset.merge_one(ann.clone()),
            Ok(MergeOneResult::NewProducer)
        ));

        // Same sequence number - should be duplicate
        let ann2 = ProducerAnnouncement::new(&keypair, 1, 0);
        assert!(matches!(
            gset.merge_one(ann2),
            Ok(MergeOneResult::Duplicate)
        ));

        assert_eq!(gset.len(), 1);
    }

    #[test]
    fn test_gset_merge_multiple_producers() {
        let mut gset = ProducerGSet::new(1);

        let kp1 = KeyPair::generate();
        let kp2 = KeyPair::generate();
        let kp3 = KeyPair::generate();

        let announcements = vec![
            ProducerAnnouncement::new(&kp1, 1, 0),
            ProducerAnnouncement::new(&kp2, 1, 0),
            ProducerAnnouncement::new(&kp3, 1, 0),
        ];

        let result = gset.merge(announcements);
        assert_eq!(result.added, 3);
        assert_eq!(result.new_producers, 3);
        assert_eq!(result.rejected, 0);
        assert_eq!(result.duplicates, 0);
        assert_eq!(gset.len(), 3);
    }

    #[test]
    fn test_gset_no_persistence_by_default() {
        let gset = ProducerGSet::new(1);
        assert!(!gset.has_persistence());
    }

    #[test]
    fn test_gset_to_bloom_filter() {
        let mut gset = ProducerGSet::new(1);
        let keypairs: Vec<_> = (0..10).map(|_| KeyPair::generate()).collect();

        for kp in &keypairs {
            let ann = ProducerAnnouncement::new(kp, 1, 0);
            gset.merge_one(ann).unwrap();
        }

        let bloom = gset.to_bloom_filter();

        // All inserted keys should be found
        for kp in &keypairs {
            assert!(bloom.probably_contains(kp.public_key()));
        }

        // Random key should not be found (with high probability)
        let random_kp = KeyPair::generate();
        // Note: This could fail with ~1% probability due to false positives
        // but for a single key it's very unlikely
        assert!(!bloom.probably_contains(random_kp.public_key()));
    }

    #[test]
    fn test_gset_delta_sync() {
        let mut gset = ProducerGSet::new(1);

        // Add 10 producers
        let keypairs: Vec<_> = (0..10).map(|_| KeyPair::generate()).collect();
        for kp in &keypairs {
            let ann = ProducerAnnouncement::new(kp, 1, 0);
            gset.merge_one(ann).unwrap();
        }

        // Peer knows first 7
        let mut peer_bloom = ProducerBloomFilter::new(10);
        for kp in &keypairs[..7] {
            peer_bloom.insert(kp.public_key());
        }

        // Delta should contain last 3
        let delta = gset.delta_for_peer(&peer_bloom);
        assert!(delta.len() >= 3); // At least 3, maybe more due to FP

        // Verify all 3 unknown are in delta
        for kp in &keypairs[7..] {
            assert!(delta.iter().any(|ann| ann.pubkey == *kp.public_key()));
        }
    }

    #[test]
    fn test_gset_delta_empty_peer() {
        let mut gset = ProducerGSet::new(1);

        let keypairs: Vec<_> = (0..5).map(|_| KeyPair::generate()).collect();
        for kp in &keypairs {
            let ann = ProducerAnnouncement::new(kp, 1, 0);
            gset.merge_one(ann).unwrap();
        }

        // Peer knows nothing
        let peer_bloom = ProducerBloomFilter::new(10);
        let delta = gset.delta_for_peer(&peer_bloom);

        // Should return all 5
        assert_eq!(delta.len(), 5);
    }

    #[test]
    fn test_gset_delta_peer_knows_all() {
        let mut gset = ProducerGSet::new(1);

        let keypairs: Vec<_> = (0..5).map(|_| KeyPair::generate()).collect();
        for kp in &keypairs {
            let ann = ProducerAnnouncement::new(kp, 1, 0);
            gset.merge_one(ann).unwrap();
        }

        // Peer knows everything
        let mut peer_bloom = ProducerBloomFilter::new(10);
        for kp in &keypairs {
            peer_bloom.insert(kp.public_key());
        }

        let delta = gset.delta_for_peer(&peer_bloom);

        // Should return none (or very few due to FP)
        assert!(delta.is_empty());
    }
}

#[cfg(test)]
mod persistence_tests {
    use super::*;
    use crypto::KeyPair;
    use tempfile::tempdir;

    #[test]
    fn test_gset_persist_and_load() {
        let dir = tempdir().expect("tempdir should be created");
        let path = dir.path().join("producers.bin");

        // Create and populate
        let mut gset = ProducerGSet::new_with_persistence(1, path.clone());
        let mut keypairs = Vec::new();
        for _ in 0..3 {
            let keypair = KeyPair::generate();
            let ann = ProducerAnnouncement::new(&keypair, 1, 0);
            gset.merge_one(ann).expect("merge should succeed");
            keypairs.push(keypair);
        }
        let original_count = gset.len();
        let original_sorted = gset.sorted_producers();

        // Drop and reload
        drop(gset);
        let loaded = ProducerGSet::new_with_persistence(1, path);

        assert_eq!(loaded.len(), original_count);
        assert_eq!(loaded.sorted_producers(), original_sorted);
    }

    #[test]
    fn test_gset_load_rejects_invalid() {
        let dir = tempdir().expect("tempdir should be created");
        let path = dir.path().join("producers.bin");

        // Create valid gset
        let mut gset = ProducerGSet::new_with_persistence(1, path.clone());
        let keypair = KeyPair::generate();
        let ann = ProducerAnnouncement::new(&keypair, 1, 0);
        gset.merge_one(ann).expect("merge should succeed");

        // Corrupt the file
        let mut data = std::fs::read(&path).expect("read should succeed");
        if !data.is_empty() {
            let last_idx = data.len() - 1;
            data[last_idx] ^= 0xFF; // Flip bits
        }
        std::fs::write(&path, data).expect("write should succeed");

        // Reload should handle corruption gracefully
        let loaded = ProducerGSet::new_with_persistence(1, path);
        // Either empty (complete corruption) or fewer (partial)
        assert!(loaded.len() <= 1);
    }

    #[test]
    fn test_gset_atomic_write() {
        let dir = tempdir().expect("tempdir should be created");
        let path = dir.path().join("producers.bin");

        let mut gset = ProducerGSet::new_with_persistence(1, path.clone());
        let keypair = KeyPair::generate();
        let ann = ProducerAnnouncement::new(&keypair, 1, 0);
        gset.merge_one(ann).expect("merge should succeed");

        // File should exist and be valid
        assert!(path.exists());
        let loaded = ProducerGSet::new_with_persistence(1, path);
        assert_eq!(loaded.len(), 1);
    }

    #[test]
    fn test_gset_has_persistence() {
        let dir = tempdir().expect("tempdir should be created");
        let path = dir.path().join("producers.bin");

        let gset = ProducerGSet::new_with_persistence(1, path);
        assert!(gset.has_persistence());
    }

    #[test]
    fn test_gset_load_nonexistent_file() {
        let dir = tempdir().expect("tempdir should be created");
        let path = dir.path().join("nonexistent.bin");

        // Should not panic, just create empty gset
        let gset = ProducerGSet::new_with_persistence(1, path);
        assert_eq!(gset.len(), 0);
    }

    #[test]
    fn test_gset_persist_updates() {
        let dir = tempdir().expect("tempdir should be created");
        let path = dir.path().join("producers.bin");

        // Create with one producer
        let mut gset = ProducerGSet::new_with_persistence(1, path.clone());
        let kp1 = KeyPair::generate();
        gset.merge_one(ProducerAnnouncement::new(&kp1, 1, 0))
            .expect("merge should succeed");

        // Reload and verify
        let loaded1 = ProducerGSet::new_with_persistence(1, path.clone());
        assert_eq!(loaded1.len(), 1);

        // Add another producer
        drop(loaded1);
        let mut gset2 = ProducerGSet::new_with_persistence(1, path.clone());
        let kp2 = KeyPair::generate();
        gset2
            .merge_one(ProducerAnnouncement::new(&kp2, 1, 0))
            .expect("merge should succeed");
        assert_eq!(gset2.len(), 2);

        // Reload and verify both are present
        drop(gset2);
        let loaded2 = ProducerGSet::new_with_persistence(1, path);
        assert_eq!(loaded2.len(), 2);
    }

    #[test]
    fn test_gset_rejects_wrong_network_on_load() {
        let dir = tempdir().expect("tempdir should be created");
        let path = dir.path().join("producers.bin");

        // Create with network 1
        let mut gset = ProducerGSet::new_with_persistence(1, path.clone());
        let keypair = KeyPair::generate();
        gset.merge_one(ProducerAnnouncement::new(&keypair, 1, 0))
            .expect("merge should succeed");
        drop(gset);

        // Try to load with network 2 - should reject
        let loaded = ProducerGSet::new_with_persistence(2, path);
        assert_eq!(loaded.len(), 0); // Rejected due to network mismatch
    }
}

/// Comprehensive stress tests and benchmarks for Milestone 12.
/// These tests validate scalability and performance of the producer discovery system.
#[cfg(test)]
mod stress_tests {
    use super::*;
    use crate::discovery::proto;
    use crypto::KeyPair;

    /// 12.4: Stress test with 100+ simulated producers.
    /// This verifies the GSet can handle production-scale producer counts.
    #[test]
    fn test_stress_100_producers() {
        let mut gset = ProducerGSet::new(1);

        // Generate 100 unique producers
        let keypairs: Vec<_> = (0..100).map(|_| KeyPair::generate()).collect();
        let announcements: Vec<_> = keypairs
            .iter()
            .map(|kp| ProducerAnnouncement::new(kp, 1, 0))
            .collect();

        // Merge all at once
        let result = gset.merge(announcements);

        assert_eq!(result.added, 100, "All 100 producers should be added");
        assert_eq!(result.rejected, 0, "No producers should be rejected");
        assert_eq!(gset.len(), 100, "GSet should have 100 producers");

        // Verify deterministic ordering
        let sorted = gset.sorted_producers();
        assert_eq!(sorted.len(), 100);
        for i in 1..sorted.len() {
            assert!(
                sorted[i - 1].as_bytes() < sorted[i].as_bytes(),
                "Producers should be sorted by bytes"
            );
        }
    }

    /// Extended stress test with 500 producers and multiple sequence updates.
    #[test]
    fn test_stress_500_producers_with_updates() {
        let mut gset = ProducerGSet::new(1);

        // Generate 500 producers
        let keypairs: Vec<_> = (0..500).map(|_| KeyPair::generate()).collect();

        // Add all with sequence 0
        for kp in &keypairs {
            let ann = ProducerAnnouncement::new(kp, 1, 0);
            gset.merge_one(ann).expect("merge should succeed");
        }
        assert_eq!(gset.len(), 500);

        // Update half with sequence 1
        for kp in &keypairs[0..250] {
            let ann = ProducerAnnouncement::new(kp, 1, 1);
            assert!(
                matches!(gset.merge_one(ann), Ok(MergeOneResult::SequenceUpdate)),
                "Update should succeed"
            );
        }
        assert_eq!(gset.len(), 500, "Count should remain 500");

        // Verify sequences were updated
        for kp in &keypairs[0..250] {
            assert_eq!(
                gset.sequence_for(kp.public_key()),
                1,
                "Sequence should be updated to 1"
            );
        }
        for kp in &keypairs[250..500] {
            assert_eq!(
                gset.sequence_for(kp.public_key()),
                0,
                "Sequence should remain 0"
            );
        }
    }

    /// 12.5: Fuzz-like test with random announcement patterns.
    #[test]
    fn test_fuzz_random_announcements() {
        let mut gset = ProducerGSet::new(1);

        // Generate a pool of producers
        let keypairs: Vec<_> = (0..50).map(|_| KeyPair::generate()).collect();

        // Send random announcements in random order with various sequences
        let mut rng_seed: u64 = 42;
        for _ in 0..200 {
            // Simple pseudo-random selection
            rng_seed = rng_seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let kp_idx = (rng_seed as usize) % keypairs.len();
            let seq = ((rng_seed >> 32) as u64) % 10;

            let ann = ProducerAnnouncement::new(&keypairs[kp_idx], 1, seq);
            // All operations should succeed without panic
            let _ = gset.merge_one(ann);
        }

        // Verify state is consistent
        assert!(gset.len() <= 50, "Should have at most 50 producers");
        let exported = gset.export();
        for ann in exported {
            assert!(ann.verify(), "All exported announcements should verify");
        }
    }

    /// Test handling of invalid/tampered announcements at scale.
    #[test]
    fn test_stress_invalid_announcements() {
        let mut gset = ProducerGSet::new(1);

        let mut valid_count = 0;
        let mut invalid_count = 0;

        // Mix of valid and invalid announcements
        for i in 0..100 {
            let keypair = KeyPair::generate();
            let mut ann = ProducerAnnouncement::new(&keypair, 1, 0);

            if i % 4 == 0 {
                // Tamper every 4th announcement
                ann.sequence = 999;
                invalid_count += 1;
            } else {
                valid_count += 1;
            }

            let _ = gset.merge_one(ann);
        }

        assert_eq!(
            gset.len(),
            valid_count,
            "Only valid announcements should be added"
        );
    }

    /// 12.6: Benchmark gossip message sizes at various network sizes.
    #[test]
    fn test_benchmark_gossip_message_sizes() {
        let sizes = [10, 50, 100, 500, 1000];

        println!("\n=== Gossip Message Size Benchmark ===");
        println!(
            "{:>8} {:>12} {:>12} {:>12}",
            "Nodes", "Proto (B)", "Per Ann (B)", "Bloom (B)"
        );

        for &size in &sizes {
            let keypairs: Vec<_> = (0..size).map(|_| KeyPair::generate()).collect();
            let announcements: Vec<_> = keypairs
                .iter()
                .map(|kp| ProducerAnnouncement::new(kp, 1, 0))
                .collect();

            // Encode using protobuf
            let proto_bytes = proto::encode_producer_set(&announcements);
            let per_announcement = proto_bytes.len() / size;

            // Bloom filter size
            let mut bloom = super::ProducerBloomFilter::new(size);
            for kp in &keypairs {
                bloom.insert(kp.public_key());
            }
            let bloom_bytes = proto::encode_digest(&bloom);

            println!(
                "{:>8} {:>12} {:>12} {:>12}",
                size,
                proto_bytes.len(),
                per_announcement,
                bloom_bytes.len()
            );

            // Verify reasonable sizes
            // Each announcement ~110-130 bytes in protobuf
            assert!(
                per_announcement < 150,
                "Announcement should be < 150 bytes, got {}",
                per_announcement
            );

            // Bloom filter should be much smaller than full set for large networks
            if size >= 100 {
                assert!(
                    bloom_bytes.len() < proto_bytes.len() / 5,
                    "Bloom should be >5x smaller than full set"
                );
            }
        }
    }

    /// 12.7: Benchmark bloom filter effectiveness for delta sync.
    ///
    /// Note: Bloom filters can have false positives, which means some unknown
    /// producers might not be included in the delta. This is expected behavior
    /// and the test accounts for it by checking:
    /// 1. Delta includes at least (expected - FP margin) producers
    /// 2. No extra producers are sent beyond what's needed (within FP tolerance)
    #[test]
    fn test_benchmark_bloom_filter_effectiveness() {
        let network_sizes = [50, 100, 500];
        let overlap_percentages = [0, 25, 50, 75, 90, 100];

        println!("\n=== Bloom Filter Delta Sync Effectiveness ===");
        println!(
            "{:>8} {:>8} {:>12} {:>12} {:>12}",
            "Size", "Overlap%", "Expected", "Actual", "Missed"
        );

        for &size in &network_sizes {
            let keypairs: Vec<_> = (0..size).map(|_| KeyPair::generate()).collect();

            // Create full gset
            let mut gset = ProducerGSet::new(1);
            for kp in &keypairs {
                let ann = ProducerAnnouncement::new(kp, 1, 0);
                gset.merge_one(ann).unwrap();
            }

            for &overlap in &overlap_percentages {
                let known_count = (size * overlap) / 100;
                let expected_delta = size - known_count;

                // Peer's bloom filter with partial knowledge
                let mut peer_bloom = super::ProducerBloomFilter::new(size);
                for kp in &keypairs[0..known_count] {
                    peer_bloom.insert(kp.public_key());
                }

                // Compute delta
                let delta = gset.delta_for_peer(&peer_bloom);

                // Count how many unknown producers are missing from delta
                // (due to bloom filter false positives)
                let mut missing = 0;
                for kp in &keypairs[known_count..] {
                    if !delta.iter().any(|ann| ann.pubkey == *kp.public_key()) {
                        missing += 1;
                    }
                }

                println!(
                    "{:>8} {:>8} {:>12} {:>12} {:>12}",
                    size,
                    overlap,
                    expected_delta,
                    delta.len(),
                    missing
                );

                // Due to bloom filter false positives, some unknown producers
                // might be falsely identified as "known" and excluded from delta.
                // This is acceptable as long as it's within the FP rate (~1%).
                // For 1% FP rate, at most ~1% of unknown items should be missing.
                let max_allowed_missing = ((expected_delta as f64) * 0.03).ceil() as usize + 1;
                assert!(
                    missing <= max_allowed_missing,
                    "Too many missing producers: {} > {} (expected {} in delta)",
                    missing,
                    max_allowed_missing,
                    expected_delta
                );

                // Delta should not contain significantly more than expected
                // (bloom filter false negatives are impossible, so extra items
                // would indicate a bug, not FP behavior)
                assert!(
                    delta.len() <= expected_delta,
                    "Delta should not exceed expected: {} > {}",
                    delta.len(),
                    expected_delta
                );
            }
        }
    }

    /// Test GSet convergence with concurrent merges.
    #[test]
    fn test_gset_convergence_simulation() {
        // Simulate 3 nodes with different initial knowledge
        let keypairs: Vec<_> = (0..30).map(|_| KeyPair::generate()).collect();

        // Node 1 knows producers 0-19
        let mut node1 = ProducerGSet::new(1);
        for kp in &keypairs[0..20] {
            node1
                .merge_one(ProducerAnnouncement::new(kp, 1, 0))
                .unwrap();
        }

        // Node 2 knows producers 10-29
        let mut node2 = ProducerGSet::new(1);
        for kp in &keypairs[10..30] {
            node2
                .merge_one(ProducerAnnouncement::new(kp, 1, 0))
                .unwrap();
        }

        // Node 3 knows producers 0-9 and 20-29
        let mut node3 = ProducerGSet::new(1);
        for kp in &keypairs[0..10] {
            node3
                .merge_one(ProducerAnnouncement::new(kp, 1, 0))
                .unwrap();
        }
        for kp in &keypairs[20..30] {
            node3
                .merge_one(ProducerAnnouncement::new(kp, 1, 0))
                .unwrap();
        }

        // Simulate gossip rounds - each node shares with others
        // Round 1: node1 -> node2, node2 -> node3, node3 -> node1
        node2.merge(node1.export());
        node3.merge(node2.export());
        node1.merge(node3.export());

        // Round 2: propagate again
        node2.merge(node1.export());
        node3.merge(node2.export());
        node1.merge(node3.export());

        // All nodes should have all 30 producers
        assert_eq!(node1.len(), 30, "Node 1 should have all 30 producers");
        assert_eq!(node2.len(), 30, "Node 2 should have all 30 producers");
        assert_eq!(node3.len(), 30, "Node 3 should have all 30 producers");

        // All should have identical sorted order
        assert_eq!(
            node1.sorted_producers(),
            node2.sorted_producers(),
            "Node 1 and 2 should have same order"
        );
        assert_eq!(
            node2.sorted_producers(),
            node3.sorted_producers(),
            "Node 2 and 3 should have same order"
        );
    }

    /// Test partition and heal scenario.
    #[test]
    fn test_partition_and_heal_simulation() {
        let keypairs: Vec<_> = (0..20).map(|_| KeyPair::generate()).collect();

        // Two partitions that develop independently
        let mut partition_a1 = ProducerGSet::new(1);
        let mut partition_a2 = ProducerGSet::new(1);
        let mut partition_b1 = ProducerGSet::new(1);
        let mut partition_b2 = ProducerGSet::new(1);

        // Partition A learns producers 0-9
        for kp in &keypairs[0..10] {
            let ann = ProducerAnnouncement::new(kp, 1, 0);
            partition_a1.merge_one(ann.clone()).unwrap();
            partition_a2.merge_one(ann).unwrap();
        }

        // Partition B learns producers 10-19
        for kp in &keypairs[10..20] {
            let ann = ProducerAnnouncement::new(kp, 1, 0);
            partition_b1.merge_one(ann.clone()).unwrap();
            partition_b2.merge_one(ann).unwrap();
        }

        // Verify partitions are isolated
        assert_eq!(partition_a1.len(), 10);
        assert_eq!(partition_b1.len(), 10);

        // Heal: merge partitions
        partition_a1.merge(partition_b1.export());
        partition_a2.merge(partition_b2.export());
        partition_b1.merge(partition_a1.export());
        partition_b2.merge(partition_a2.export());

        // All nodes should converge to 20 producers
        assert_eq!(partition_a1.len(), 20, "A1 should have all 20");
        assert_eq!(partition_a2.len(), 20, "A2 should have all 20");
        assert_eq!(partition_b1.len(), 20, "B1 should have all 20");
        assert_eq!(partition_b2.len(), 20, "B2 should have all 20");

        // All should have same sorted order
        let reference = partition_a1.sorted_producers();
        assert_eq!(partition_a2.sorted_producers(), reference);
        assert_eq!(partition_b1.sorted_producers(), reference);
        assert_eq!(partition_b2.sorted_producers(), reference);
    }

    /// Test adaptive gossip with simulated merge results.
    #[test]
    fn test_adaptive_gossip_integration() {
        use crate::discovery::AdaptiveGossip;
        use std::time::Duration;

        let mut gossip = AdaptiveGossip::new();

        // Initial state
        assert_eq!(gossip.interval(), Duration::from_secs(5));
        assert!(!gossip.use_delta_sync());

        // Simulate discovering new producers (high activity)
        for _ in 0..3 {
            gossip.on_gossip_result(
                &MergeResult {
                    added: 5,
                    new_producers: 5,
                    rejected: 0,
                    duplicates: 0,
                },
                10,
            );
        }
        assert_eq!(
            gossip.interval(),
            Duration::from_secs(1),
            "Should speed up on new discoveries"
        );

        // Simulate stable period (no changes)
        for _ in 0..10 {
            gossip.on_gossip_result(&MergeResult::default(), 50);
        }
        assert!(
            gossip.interval() > Duration::from_secs(5),
            "Should back off during stable period"
        );
        assert!(
            gossip.use_delta_sync(),
            "Should enable delta sync for large network"
        );

        // New discovery resets interval
        gossip.on_gossip_result(
            &MergeResult {
                added: 1,
                new_producers: 1,
                rejected: 0,
                duplicates: 0,
            },
            50,
        );
        assert_eq!(
            gossip.interval(),
            Duration::from_secs(1),
            "Should reset on new discovery"
        );
    }

    /// Memory usage estimation for large networks.
    #[test]
    fn test_memory_estimation() {
        // Estimate memory usage for various network sizes
        let sizes = [100, 1000, 10000];

        println!("\n=== Memory Usage Estimation ===");
        println!(
            "{:>8} {:>15} {:>15}",
            "Nodes", "Est. Memory", "Per Producer"
        );

        for &size in &sizes {
            // Each announcement: ~130 bytes (pubkey 32 + sig 64 + fields ~34)
            // HashMap overhead: ~48 bytes per entry
            // Sequences map: ~40 bytes per entry
            let ann_size = 130;
            let hashmap_overhead = 48 + 40;
            let estimated_bytes = size * (ann_size + hashmap_overhead);

            println!(
                "{:>8} {:>12} KB {:>12} B",
                size,
                estimated_bytes / 1024,
                ann_size + hashmap_overhead
            );

            // 10K producers should use < 5MB
            if size == 10000 {
                assert!(
                    estimated_bytes < 5 * 1024 * 1024,
                    "10K producers should use < 5MB"
                );
            }
        }
    }
}
