//! ProducerGSet - Grow-Only Set CRDT for Producer Discovery
//!
//! This module implements a Grow-Only Set CRDT with cryptographic proofs
//! and version vectors for eventual consistency in producer discovery.

mod merge;
mod persistence;
mod query;

#[cfg(test)]
mod persistence_tests;
#[cfg(test)]
mod stress_tests;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use crypto::{Hash, PublicKey};

use super::ProducerAnnouncement;

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
/// use crypto::{KeyPair, Hash};
///
/// let mut gset = ProducerGSet::new(1, Hash::ZERO); // network_id = 1
/// let keypair = KeyPair::generate();
/// let announcement = ProducerAnnouncement::new(&keypair, 1, 0, Hash::ZERO);
///
/// let result = gset.merge_one(announcement);
/// assert!(result.is_ok());
/// assert_eq!(gset.len(), 1);
/// ```
#[derive(Debug)]
pub struct ProducerGSet {
    /// Map from public key to the latest valid announcement.
    pub(super) producers: HashMap<PublicKey, ProducerAnnouncement>,

    /// Version vector: tracks the highest known sequence for each producer.
    pub(super) sequences: HashMap<PublicKey, u64>,

    /// Time of last modification (for stability tracking).
    pub(super) last_modified: Instant,

    /// Network ID this GSet is configured for.
    pub(super) network_id: u32,

    /// Genesis hash this GSet is configured for (prevents cross-genesis contamination).
    pub(super) genesis_hash: Hash,

    /// Optional path for disk persistence.
    pub(super) storage_path: Option<PathBuf>,
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
    /// Create a new empty ProducerGSet for the given network and genesis.
    ///
    /// # Arguments
    ///
    /// * `network_id` - The network ID (1=mainnet, 2=testnet, 99=devnet)
    /// * `genesis_hash` - The genesis hash for this chain
    #[must_use]
    pub fn new(network_id: u32, genesis_hash: Hash) -> Self {
        Self {
            producers: HashMap::new(),
            sequences: HashMap::new(),
            last_modified: Instant::now(),
            network_id,
            genesis_hash,
            storage_path: None,
        }
    }

    /// Create a new ProducerGSet with disk persistence.
    ///
    /// If the file exists, it will be loaded and verified.
    /// Only announcements that pass signature and genesis_hash verification will be loaded.
    ///
    /// # Arguments
    ///
    /// * `network_id` - The network ID (1=mainnet, 2=testnet, 99=devnet)
    /// * `genesis_hash` - The genesis hash for this chain
    /// * `path` - Path to the persistence file
    #[must_use]
    pub fn new_with_persistence(network_id: u32, genesis_hash: Hash, path: PathBuf) -> Self {
        let mut gset = Self {
            producers: HashMap::new(),
            sequences: HashMap::new(),
            last_modified: Instant::now(),
            network_id,
            genesis_hash,
            storage_path: Some(path),
        };

        // Try to load existing data
        gset.load_from_disk();

        gset
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

    /// Get the genesis hash this GSet is configured for.
    #[must_use]
    pub fn genesis_hash(&self) -> Hash {
        self.genesis_hash
    }

    /// Get the current sequence number for a producer.
    ///
    /// Returns 0 if the producer is not known.
    #[must_use]
    pub fn sequence_for(&self, pubkey: &PublicKey) -> u64 {
        self.sequences.get(pubkey).copied().unwrap_or(0)
    }

    /// Check if this GSet has persistence enabled.
    #[must_use]
    pub fn has_persistence(&self) -> bool {
        self.storage_path.is_some()
    }
}
