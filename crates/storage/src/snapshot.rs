//! State snapshot for snap sync
//!
//! Provides deterministic state root computation and snapshot
//! serialization/deserialization for fast node bootstrapping.

use crypto::Hash;

use crate::chain_state::ChainState;
use crate::producer::ProducerSet;
use crate::utxo::UtxoSet;
use crate::StorageError;

/// Compute a deterministic state root from the three state components.
///
/// The state root is: `H(H(chain_state) || H(utxo_set) || H(producer_set))`
/// where H is the crypto hash function and || is concatenation.
///
/// This is deterministic because:
/// - `bincode::serialize` produces canonical byte sequences
/// - The hash function is deterministic
/// - The concatenation order is fixed
pub fn compute_state_root(
    chain_state: &ChainState,
    utxo_set: &UtxoSet,
    producer_set: &ProducerSet,
) -> Result<Hash, StorageError> {
    let cs_bytes =
        bincode::serialize(chain_state).map_err(|e| StorageError::Serialization(e.to_string()))?;
    let utxo_bytes =
        bincode::serialize(utxo_set).map_err(|e| StorageError::Serialization(e.to_string()))?;
    let ps_bytes =
        bincode::serialize(producer_set).map_err(|e| StorageError::Serialization(e.to_string()))?;

    Ok(compute_state_root_from_bytes(
        &cs_bytes,
        &utxo_bytes,
        &ps_bytes,
    ))
}

/// Compute state root from pre-serialized byte slices.
///
/// Used when we already have the serialized components (e.g., during
/// snapshot verification).
pub fn compute_state_root_from_bytes(
    chain_state_bytes: &[u8],
    utxo_set_bytes: &[u8],
    producer_set_bytes: &[u8],
) -> Hash {
    let cs_hash = crypto::hash::hash(chain_state_bytes);
    let utxo_hash = crypto::hash::hash(utxo_set_bytes);
    let ps_hash = crypto::hash::hash(producer_set_bytes);

    let mut combined = Vec::with_capacity(96); // 3 * 32 bytes
    combined.extend_from_slice(cs_hash.as_bytes());
    combined.extend_from_slice(utxo_hash.as_bytes());
    combined.extend_from_slice(ps_hash.as_bytes());
    crypto::hash::hash(&combined)
}

/// A serialized state snapshot ready for transfer.
pub struct StateSnapshot {
    /// Block hash this snapshot is valid at
    pub block_hash: Hash,
    /// Block height at snapshot
    pub block_height: u64,
    /// Serialized ChainState (bincode)
    pub chain_state_bytes: Vec<u8>,
    /// Serialized UtxoSet (bincode)
    pub utxo_set_bytes: Vec<u8>,
    /// Serialized ProducerSet (bincode)
    pub producer_set_bytes: Vec<u8>,
    /// State root for verification
    pub state_root: Hash,
}

impl StateSnapshot {
    /// Create a snapshot from the current state.
    pub fn create(
        chain_state: &ChainState,
        utxo_set: &UtxoSet,
        producer_set: &ProducerSet,
    ) -> Result<Self, StorageError> {
        let chain_state_bytes = bincode::serialize(chain_state)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let utxo_set_bytes =
            bincode::serialize(utxo_set).map_err(|e| StorageError::Serialization(e.to_string()))?;
        let producer_set_bytes = bincode::serialize(producer_set)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;

        let state_root =
            compute_state_root_from_bytes(&chain_state_bytes, &utxo_set_bytes, &producer_set_bytes);

        Ok(Self {
            block_hash: chain_state.best_hash,
            block_height: chain_state.best_height,
            chain_state_bytes,
            utxo_set_bytes,
            producer_set_bytes,
            state_root,
        })
    }

    /// Verify the snapshot's state root matches the expected root.
    pub fn verify(&self, expected_root: &Hash) -> bool {
        let computed = compute_state_root_from_bytes(
            &self.chain_state_bytes,
            &self.utxo_set_bytes,
            &self.producer_set_bytes,
        );
        &computed == expected_root
    }

    /// Deserialize the snapshot into live state objects.
    ///
    /// Returns (ChainState, UtxoSet, ProducerSet) or an error if
    /// deserialization fails.
    pub fn deserialize(&self) -> Result<(ChainState, UtxoSet, ProducerSet), StorageError> {
        let chain_state: ChainState = bincode::deserialize(&self.chain_state_bytes)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let utxo_set: UtxoSet = bincode::deserialize(&self.utxo_set_bytes)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let producer_set: ProducerSet = bincode::deserialize(&self.producer_set_bytes)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;

        Ok((chain_state, utxo_set, producer_set))
    }

    /// Total size of the serialized state in bytes.
    pub fn total_bytes(&self) -> usize {
        self.chain_state_bytes.len() + self.utxo_set_bytes.len() + self.producer_set_bytes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_root_deterministic() {
        let cs = ChainState::new(Hash::zero());
        let utxo = UtxoSet::new();
        let ps = ProducerSet::new();

        let root1 = compute_state_root(&cs, &utxo, &ps).unwrap();
        let root2 = compute_state_root(&cs, &utxo, &ps).unwrap();

        assert_eq!(root1, root2, "State root must be deterministic");
    }

    #[test]
    fn test_state_root_changes_with_state() {
        let utxo = UtxoSet::new();
        let ps = ProducerSet::new();

        let cs1 = ChainState::new(Hash::zero());
        let mut cs2 = ChainState::new(Hash::zero());
        cs2.best_height = 100;

        let root1 = compute_state_root(&cs1, &utxo, &ps).unwrap();
        let root2 = compute_state_root(&cs2, &utxo, &ps).unwrap();

        assert_ne!(root1, root2, "Different state must produce different root");
    }

    #[test]
    fn test_snapshot_create_verify_roundtrip() {
        let cs = ChainState::new(Hash::zero());
        let utxo = UtxoSet::new();
        let ps = ProducerSet::new();

        let snapshot = StateSnapshot::create(&cs, &utxo, &ps).unwrap();
        assert!(snapshot.verify(&snapshot.state_root));

        // Tamper detection
        let wrong_root = Hash::zero();
        assert!(!snapshot.verify(&wrong_root));
    }

    #[test]
    fn test_snapshot_deserialize() {
        let mut cs = ChainState::new(Hash::zero());
        cs.best_height = 42;
        cs.total_minted = 1_000_000;
        let utxo = UtxoSet::new();
        let ps = ProducerSet::new();

        let snapshot = StateSnapshot::create(&cs, &utxo, &ps).unwrap();
        let (cs_out, _utxo_out, _ps_out) = snapshot.deserialize().unwrap();

        assert_eq!(cs_out.best_height, 42);
        assert_eq!(cs_out.total_minted, 1_000_000);
    }

    #[test]
    fn test_state_root_from_bytes_matches_struct() {
        let cs = ChainState::new(Hash::zero());
        let utxo = UtxoSet::new();
        let ps = ProducerSet::new();

        let root_struct = compute_state_root(&cs, &utxo, &ps).unwrap();

        let cs_bytes = bincode::serialize(&cs).unwrap();
        let utxo_bytes = bincode::serialize(&utxo).unwrap();
        let ps_bytes = bincode::serialize(&ps).unwrap();
        let root_bytes = compute_state_root_from_bytes(&cs_bytes, &utxo_bytes, &ps_bytes);

        assert_eq!(root_struct, root_bytes);
    }
}
