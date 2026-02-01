//! # Claim Registry
//!
//! Storage for tracking which (producer, epoch) reward claims have been made.
//!
//! ## Purpose
//!
//! The claim registry prevents double-claiming of epoch rewards:
//! - Each producer can only claim rewards for an epoch once
//! - Claims are recorded permanently and survive restarts
//! - Reorgs can revert claims to maintain consistency
//!
//! ## Storage Format
//!
//! Uses a HashMap with composite keys (producer pubkey hash + epoch) for O(1) lookups.
//! Persisted to disk using bincode serialization.

use std::collections::HashMap;
use std::path::Path;

use crypto::hash::hash_concat;
use crypto::{Hash, PublicKey};
use doli_core::types::{Amount, BlockHeight};
use serde::{Deserialize, Serialize};

use crate::StorageError;

// =============================================================================
// CLAIM RECORD
// =============================================================================

/// Record of a completed epoch reward claim.
///
/// Stored in the registry to:
/// - Prevent double-claiming
/// - Provide audit trail
/// - Support reorg handling
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimRecord {
    /// Hash of the transaction that executed the claim
    pub tx_hash: Hash,
    /// Block height where the claim was confirmed
    pub height: BlockHeight,
    /// Amount claimed (in base units)
    pub amount: Amount,
    /// Timestamp when the claim was confirmed
    pub timestamp: u64,
}

impl ClaimRecord {
    /// Create a new claim record.
    pub fn new(tx_hash: Hash, height: BlockHeight, amount: Amount, timestamp: u64) -> Self {
        Self {
            tx_hash,
            height,
            amount,
            timestamp,
        }
    }
}

// =============================================================================
// CLAIM KEY
// =============================================================================

/// A unique key for a (producer, epoch) claim.
///
/// Used internally as the HashMap key.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClaimKey {
    /// Producer's public key (32 bytes)
    producer: [u8; 32],
    /// Epoch number
    epoch: u64,
}

impl ClaimKey {
    /// Create a new claim key.
    pub fn new(producer: &PublicKey, epoch: u64) -> Self {
        Self {
            producer: *producer.as_bytes(),
            epoch,
        }
    }

    /// Compute a deterministic hash of this key.
    ///
    /// Useful for external indexing or RocksDB column family keys.
    pub fn hash(&self) -> Hash {
        hash_concat(&[
            b"DOLI_CLAIM_KEY_V1",
            &self.producer,
            &self.epoch.to_le_bytes(),
        ])
    }
}

// =============================================================================
// CLAIM REGISTRY
// =============================================================================

/// Registry tracking which (producer, epoch) pairs have been claimed.
///
/// Provides:
/// - O(1) lookup to check if a claim exists
/// - O(1) insertion to record new claims
/// - List of unclaimed epochs for a producer
/// - Revert capability for reorg handling
///
/// # Persistence
///
/// The registry is stored in memory and periodically saved to disk.
/// Use `save()` after making changes to ensure durability.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ClaimRegistry {
    /// Map of (producer, epoch) → claim record
    claims: HashMap<ClaimKey, ClaimRecord>,
    /// Total number of claims ever made
    total_claims: u64,
    /// Total amount ever claimed (in base units)
    total_claimed: Amount,
}

impl ClaimRegistry {
    /// Create a new empty claim registry.
    pub fn new() -> Self {
        Self {
            claims: HashMap::new(),
            total_claims: 0,
            total_claimed: 0,
        }
    }

    /// Load the registry from disk.
    ///
    /// Returns a new empty registry if the file doesn't exist.
    pub fn load(path: &Path) -> Result<Self, StorageError> {
        if !path.exists() {
            return Ok(Self::new());
        }
        let bytes = std::fs::read(path)?;
        bincode::deserialize(&bytes).map_err(|e| StorageError::Serialization(e.to_string()))
    }

    /// Save the registry to disk.
    pub fn save(&self, path: &Path) -> Result<(), StorageError> {
        let bytes =
            bincode::serialize(self).map_err(|e| StorageError::Serialization(e.to_string()))?;
        std::fs::write(path, bytes)?;
        Ok(())
    }

    /// Check if a (producer, epoch) pair has been claimed.
    pub fn is_claimed(&self, producer: &PublicKey, epoch: u64) -> bool {
        let key = ClaimKey::new(producer, epoch);
        self.claims.contains_key(&key)
    }

    /// Mark a (producer, epoch) pair as claimed.
    ///
    /// Returns `Ok(())` if successful, or `Err` if already claimed.
    pub fn mark_claimed(
        &mut self,
        producer: &PublicKey,
        epoch: u64,
        record: ClaimRecord,
    ) -> Result<(), StorageError> {
        let key = ClaimKey::new(producer, epoch);

        if self.claims.contains_key(&key) {
            return Err(StorageError::AlreadyExists(format!(
                "claim for producer {:?} epoch {}",
                &producer.as_bytes()[..8],
                epoch
            )));
        }

        self.total_claims += 1;
        self.total_claimed = self.total_claimed.saturating_add(record.amount);
        self.claims.insert(key, record);

        Ok(())
    }

    /// Get the claim record for a (producer, epoch) pair.
    pub fn get_claim(&self, producer: &PublicKey, epoch: u64) -> Option<&ClaimRecord> {
        let key = ClaimKey::new(producer, epoch);
        self.claims.get(&key)
    }

    /// Get list of unclaimed epochs for a producer in a range.
    ///
    /// Returns epochs in `[start_epoch, end_epoch)` that have not been claimed.
    pub fn get_unclaimed_epochs(
        &self,
        producer: &PublicKey,
        start_epoch: u64,
        end_epoch: u64,
    ) -> Vec<u64> {
        (start_epoch..end_epoch)
            .filter(|&epoch| !self.is_claimed(producer, epoch))
            .collect()
    }

    /// Revert a claim (for reorg handling).
    ///
    /// Removes the claim record and adjusts totals.
    /// Returns the removed record if it existed.
    pub fn revert_claim(&mut self, producer: &PublicKey, epoch: u64) -> Option<ClaimRecord> {
        let key = ClaimKey::new(producer, epoch);

        if let Some(record) = self.claims.remove(&key) {
            self.total_claims = self.total_claims.saturating_sub(1);
            self.total_claimed = self.total_claimed.saturating_sub(record.amount);
            Some(record)
        } else {
            None
        }
    }

    /// Get all claims for a producer.
    pub fn get_producer_claims(&self, producer: &PublicKey) -> Vec<(u64, &ClaimRecord)> {
        let producer_bytes = *producer.as_bytes();
        self.claims
            .iter()
            .filter(|(key, _)| key.producer == producer_bytes)
            .map(|(key, record)| (key.epoch, record))
            .collect()
    }

    /// Get the total number of claims in the registry.
    pub fn total_claims(&self) -> u64 {
        self.total_claims
    }

    /// Get the total amount claimed across all producers.
    pub fn total_claimed(&self) -> Amount {
        self.total_claimed
    }

    /// Get the number of unique producers who have made claims.
    pub fn unique_producers(&self) -> usize {
        let unique: std::collections::HashSet<_> =
            self.claims.keys().map(|k| k.producer).collect();
        unique.len()
    }

    /// Check if the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.claims.is_empty()
    }

    /// Clear all claims (for testing only).
    #[cfg(test)]
    pub fn clear(&mut self) {
        self.claims.clear();
        self.total_claims = 0;
        self.total_claimed = 0;
    }
}

// =============================================================================
// CLAIM CHECKER TRAIT IMPLEMENTATION
// =============================================================================

/// Implement ClaimChecker trait for ClaimRegistry.
///
/// This allows the ClaimRegistry to be used with `validate_claim_epoch_reward`
/// in the core crate for full claim validation.
impl doli_core::ClaimChecker for ClaimRegistry {
    fn is_claimed(&self, producer: &PublicKey, epoch: u64) -> bool {
        self.is_claimed(producer, epoch)
    }
}

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_pubkey(seed: u8) -> PublicKey {
        let mut bytes = [0u8; 32];
        bytes[0] = seed;
        PublicKey::from_bytes(bytes)
    }

    fn mock_hash(seed: u8) -> Hash {
        let mut bytes = [0u8; 32];
        bytes[0] = seed;
        Hash::from_bytes(bytes)
    }

    #[test]
    fn test_new_registry_is_empty() {
        let registry = ClaimRegistry::new();

        assert!(registry.is_empty());
        assert_eq!(registry.total_claims(), 0);
        assert_eq!(registry.total_claimed(), 0);
    }

    #[test]
    fn test_mark_claimed() {
        let mut registry = ClaimRegistry::new();
        let producer = mock_pubkey(1);
        let record = ClaimRecord::new(mock_hash(1), 100, 1_000_000, 12345);

        // First claim should succeed
        assert!(registry.mark_claimed(&producer, 5, record.clone()).is_ok());
        assert!(registry.is_claimed(&producer, 5));
        assert_eq!(registry.total_claims(), 1);
        assert_eq!(registry.total_claimed(), 1_000_000);
    }

    #[test]
    fn test_double_claim_rejected() {
        let mut registry = ClaimRegistry::new();
        let producer = mock_pubkey(1);
        let record1 = ClaimRecord::new(mock_hash(1), 100, 1_000_000, 12345);
        let record2 = ClaimRecord::new(mock_hash(2), 101, 2_000_000, 12346);

        // First claim succeeds
        assert!(registry.mark_claimed(&producer, 5, record1).is_ok());

        // Second claim for same epoch fails
        let result = registry.mark_claimed(&producer, 5, record2);
        assert!(matches!(result, Err(StorageError::AlreadyExists(_))));

        // Totals unchanged
        assert_eq!(registry.total_claims(), 1);
        assert_eq!(registry.total_claimed(), 1_000_000);
    }

    #[test]
    fn test_different_epochs() {
        let mut registry = ClaimRegistry::new();
        let producer = mock_pubkey(1);

        // Can claim different epochs
        for epoch in 0..5 {
            let record = ClaimRecord::new(mock_hash(epoch as u8), 100 + epoch, 1_000_000, 12345);
            assert!(registry.mark_claimed(&producer, epoch, record).is_ok());
        }

        assert_eq!(registry.total_claims(), 5);
        assert_eq!(registry.total_claimed(), 5_000_000);

        // All epochs are claimed
        for epoch in 0..5 {
            assert!(registry.is_claimed(&producer, epoch));
        }

        // Epoch 5 is not claimed
        assert!(!registry.is_claimed(&producer, 5));
    }

    #[test]
    fn test_different_producers() {
        let mut registry = ClaimRegistry::new();
        let producer1 = mock_pubkey(1);
        let producer2 = mock_pubkey(2);
        let record = ClaimRecord::new(mock_hash(1), 100, 1_000_000, 12345);

        // Same epoch, different producers
        assert!(registry.mark_claimed(&producer1, 5, record.clone()).is_ok());
        assert!(registry.mark_claimed(&producer2, 5, record).is_ok());

        assert!(registry.is_claimed(&producer1, 5));
        assert!(registry.is_claimed(&producer2, 5));
        assert_eq!(registry.unique_producers(), 2);
    }

    #[test]
    fn test_get_claim() {
        let mut registry = ClaimRegistry::new();
        let producer = mock_pubkey(1);
        let record = ClaimRecord::new(mock_hash(42), 100, 1_000_000, 12345);

        registry.mark_claimed(&producer, 5, record.clone()).unwrap();

        let retrieved = registry.get_claim(&producer, 5).unwrap();
        assert_eq!(retrieved.tx_hash, mock_hash(42));
        assert_eq!(retrieved.height, 100);
        assert_eq!(retrieved.amount, 1_000_000);

        // Non-existent claim returns None
        assert!(registry.get_claim(&producer, 6).is_none());
    }

    #[test]
    fn test_get_unclaimed_epochs() {
        let mut registry = ClaimRegistry::new();
        let producer = mock_pubkey(1);

        // Claim epochs 2 and 4
        let record = ClaimRecord::new(mock_hash(1), 100, 1_000_000, 12345);
        registry.mark_claimed(&producer, 2, record.clone()).unwrap();
        registry.mark_claimed(&producer, 4, record).unwrap();

        // Get unclaimed in range 0..6
        let unclaimed = registry.get_unclaimed_epochs(&producer, 0, 6);
        assert_eq!(unclaimed, vec![0, 1, 3, 5]);
    }

    #[test]
    fn test_revert_claim() {
        let mut registry = ClaimRegistry::new();
        let producer = mock_pubkey(1);
        let record = ClaimRecord::new(mock_hash(1), 100, 1_000_000, 12345);

        registry.mark_claimed(&producer, 5, record.clone()).unwrap();
        assert!(registry.is_claimed(&producer, 5));
        assert_eq!(registry.total_claims(), 1);

        // Revert the claim
        let reverted = registry.revert_claim(&producer, 5);
        assert!(reverted.is_some());
        assert_eq!(reverted.unwrap().amount, 1_000_000);

        // Claim is gone
        assert!(!registry.is_claimed(&producer, 5));
        assert_eq!(registry.total_claims(), 0);
        assert_eq!(registry.total_claimed(), 0);

        // Can claim again
        assert!(registry.mark_claimed(&producer, 5, record).is_ok());
    }

    #[test]
    fn test_revert_nonexistent() {
        let mut registry = ClaimRegistry::new();
        let producer = mock_pubkey(1);

        // Reverting non-existent claim returns None
        let reverted = registry.revert_claim(&producer, 5);
        assert!(reverted.is_none());
    }

    #[test]
    fn test_get_producer_claims() {
        let mut registry = ClaimRegistry::new();
        let producer = mock_pubkey(1);

        for epoch in [2, 5, 8] {
            let record = ClaimRecord::new(mock_hash(epoch as u8), 100 + epoch, 1_000_000, 12345);
            registry.mark_claimed(&producer, epoch, record).unwrap();
        }

        let claims = registry.get_producer_claims(&producer);
        assert_eq!(claims.len(), 3);

        let epochs: Vec<u64> = claims.iter().map(|(e, _)| *e).collect();
        assert!(epochs.contains(&2));
        assert!(epochs.contains(&5));
        assert!(epochs.contains(&8));
    }

    #[test]
    fn test_claim_key_hash_deterministic() {
        let producer = mock_pubkey(1);
        let epoch = 42u64;

        let key1 = ClaimKey::new(&producer, epoch);
        let key2 = ClaimKey::new(&producer, epoch);

        assert_eq!(key1.hash(), key2.hash());

        // Different epoch = different hash
        let key3 = ClaimKey::new(&producer, 43);
        assert_ne!(key1.hash(), key3.hash());
    }

    #[test]
    fn test_serialization_roundtrip() {
        let mut registry = ClaimRegistry::new();
        let producer = mock_pubkey(1);

        for epoch in 0..3 {
            let record = ClaimRecord::new(mock_hash(epoch as u8), 100 + epoch, 1_000_000, 12345);
            registry.mark_claimed(&producer, epoch, record).unwrap();
        }

        // Serialize and deserialize
        let bytes = bincode::serialize(&registry).unwrap();
        let restored: ClaimRegistry = bincode::deserialize(&bytes).unwrap();

        assert_eq!(restored.total_claims(), 3);
        assert_eq!(restored.total_claimed(), 3_000_000);
        assert!(restored.is_claimed(&producer, 0));
        assert!(restored.is_claimed(&producer, 1));
        assert!(restored.is_claimed(&producer, 2));
    }

    #[test]
    fn test_file_persistence() {
        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join("test_claim_registry.bin");

        // Clean up from previous runs
        let _ = std::fs::remove_file(&path);

        // Create and save
        {
            let mut registry = ClaimRegistry::new();
            let producer = mock_pubkey(1);
            let record = ClaimRecord::new(mock_hash(1), 100, 1_000_000, 12345);
            registry.mark_claimed(&producer, 5, record).unwrap();
            registry.save(&path).unwrap();
        }

        // Load and verify
        {
            let registry = ClaimRegistry::load(&path).unwrap();
            let producer = mock_pubkey(1);
            assert!(registry.is_claimed(&producer, 5));
            assert_eq!(registry.total_claims(), 1);
        }

        // Clean up
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_nonexistent_returns_empty() {
        let path = Path::new("/nonexistent/path/to/registry.bin");
        let registry = ClaimRegistry::load(path).unwrap();
        assert!(registry.is_empty());
    }
}
