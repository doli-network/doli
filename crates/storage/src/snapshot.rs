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
/// Deterministic because all three components use canonical serialization:
/// - ChainState: bincode (only scalar fields, no collections)
/// - UtxoSet: `serialize_canonical()` — entries sorted by outpoint key
/// - ProducerSet: `serialize_canonical()` — entries sorted by pubkey hash
pub fn compute_state_root(
    chain_state: &ChainState,
    utxo_set: &UtxoSet,
    producer_set: &ProducerSet,
) -> Result<Hash, StorageError> {
    let cs_bytes =
        bincode::serialize(chain_state).map_err(|e| StorageError::Serialization(e.to_string()))?;
    let utxo_bytes = utxo_set.serialize_canonical();
    let ps_bytes = producer_set.serialize_canonical();

    // Hash each component individually, then combine
    let cs_hash = crypto::hash::hash(&cs_bytes);
    let utxo_hash = crypto::hash::hash(&utxo_bytes);
    let ps_hash = crypto::hash::hash(&ps_bytes);

    let mut combined = Vec::with_capacity(96);
    combined.extend_from_slice(cs_hash.as_bytes());
    combined.extend_from_slice(utxo_hash.as_bytes());
    combined.extend_from_slice(ps_hash.as_bytes());
    let final_root = crypto::hash::hash(&combined);

    // TEMPORARY FORENSIC LOGGING — identifies which component diverges across nodes.
    // Compare [STATE_ROOT_FORENSICS] lines at the same height across N1-N5.
    // Remove once root cause is confirmed and fixed.
    tracing::info!(
        "[STATE_ROOT_FORENSICS] h={} slot={} total_work={} genesis_ts={} \
         last_reg={:.8} reg_seq={} total_minted={} | \
         cs={:.16} utxo={:.16} ps={:.16} root={:.16}",
        chain_state.best_height,
        chain_state.best_slot,
        chain_state.total_work,
        chain_state.genesis_timestamp,
        chain_state.last_registration_hash,
        chain_state.registration_sequence,
        chain_state.total_minted,
        cs_hash,
        utxo_hash,
        ps_hash,
        final_root,
    );

    Ok(final_root)
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
    /// Serialized UtxoSet (canonical format)
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
        let utxo_set_bytes = utxo_set.serialize_canonical();
        // Wire format uses bincode for ProducerSet (deserializable).
        // State root uses serialize_canonical() (deterministic).
        let producer_set_bytes = bincode::serialize(producer_set)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;

        let state_root = compute_state_root(chain_state, utxo_set, producer_set)?;

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
    ///
    /// Deserializes the snapshot to compute the canonical state root,
    /// ensuring it matches regardless of wire serialization format.
    pub fn verify(&self, expected_root: &Hash) -> bool {
        match self.deserialize() {
            Ok((cs, utxo, ps)) => match compute_state_root(&cs, &utxo, &ps) {
                Ok(computed) => &computed == expected_root,
                Err(_) => false,
            },
            Err(_) => false,
        }
    }

    /// Deserialize the snapshot into live state objects.
    ///
    /// Returns (ChainState, UtxoSet, ProducerSet) or an error if
    /// deserialization fails. The UtxoSet is reconstructed from canonical
    /// bytes into an in-memory backend.
    pub fn deserialize(&self) -> Result<(ChainState, UtxoSet, ProducerSet), StorageError> {
        let chain_state: ChainState = bincode::deserialize(&self.chain_state_bytes)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;

        let utxo_set = deserialize_canonical_utxo(&self.utxo_set_bytes)?;

        let producer_set: ProducerSet = bincode::deserialize(&self.producer_set_bytes)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;

        Ok((chain_state, utxo_set, producer_set))
    }

    /// Total size of the serialized state in bytes.
    pub fn total_bytes(&self) -> usize {
        self.chain_state_bytes.len() + self.utxo_set_bytes.len() + self.producer_set_bytes.len()
    }
}

/// Deserialize canonical UTXO bytes into an in-memory UtxoSet.
///
/// Format: `[8-byte LE count] [key1 (36 bytes)][value1 (bincode)] ...`
fn deserialize_canonical_utxo(bytes: &[u8]) -> Result<UtxoSet, StorageError> {
    use crate::utxo::{InMemoryUtxoStore, Outpoint, UtxoEntry};

    if bytes.len() < 8 {
        return Err(StorageError::Serialization(
            "canonical UTXO bytes too short".into(),
        ));
    }

    let count = u64::from_le_bytes(bytes[..8].try_into().unwrap()) as usize;
    let mut store = InMemoryUtxoStore::new();
    let mut pos = 8;

    for _ in 0..count {
        // Read outpoint key (36 bytes)
        if pos + 36 > bytes.len() {
            return Err(StorageError::Serialization(
                "truncated canonical UTXO key".into(),
            ));
        }
        let outpoint = Outpoint::from_bytes(&bytes[pos..pos + 36]).ok_or_else(|| {
            StorageError::Serialization("invalid outpoint in canonical UTXO".into())
        })?;
        pos += 36;

        // Read bincode-encoded UtxoEntry (variable length)
        let entry: UtxoEntry = bincode::deserialize(&bytes[pos..])
            .map_err(|e| StorageError::Serialization(format!("canonical UTXO entry: {}", e)))?;

        // Compute the serialized size to advance pos correctly
        let entry_size = bincode::serialized_size(&entry)
            .map_err(|e| StorageError::Serialization(format!("entry size: {}", e)))?
            as usize;
        pos += entry_size;

        store.insert(outpoint, entry);
    }

    Ok(UtxoSet::InMemory(store))
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
    fn test_state_root_deterministic_across_calls() {
        let cs = ChainState::new(Hash::zero());
        let utxo = UtxoSet::new();
        let ps = ProducerSet::new();

        // Multiple calls must produce identical roots
        let root1 = compute_state_root(&cs, &utxo, &ps).unwrap();
        let root2 = compute_state_root(&cs, &utxo, &ps).unwrap();
        let root3 = compute_state_root(&cs, &utxo, &ps).unwrap();

        assert_eq!(root1, root2);
        assert_eq!(root2, root3);
    }

    #[test]
    fn test_snapshot_with_utxos_roundtrip() {
        use doli_core::transaction::Transaction;

        let cs = ChainState::new(Hash::zero());
        let mut utxo = UtxoSet::new();
        let ps = ProducerSet::new();

        // Add some UTXOs
        let pk_hash = crypto::hash::hash(b"test");
        let tx = Transaction::new_coinbase(1_000_000, pk_hash, 0);
        utxo.add_transaction(&tx, 0, true);

        // Create and verify snapshot
        let snapshot = StateSnapshot::create(&cs, &utxo, &ps).unwrap();
        assert!(snapshot.verify(&snapshot.state_root));

        // Deserialize and check UTXOs preserved
        let (_cs_out, utxo_out, _ps_out) = snapshot.deserialize().unwrap();
        assert_eq!(utxo_out.len(), 1);
        assert_eq!(utxo_out.total_value(), 1_000_000);
    }

    #[test]
    fn test_producer_set_canonical_deterministic_insertion_order() {
        use crate::producer::ProducerInfo;

        // Create two ProducerSets with same producers inserted in different order
        let pk1 = crypto::PublicKey::from_bytes([1u8; 32]);
        let pk2 = crypto::PublicKey::from_bytes([2u8; 32]);
        let pk3 = crypto::PublicKey::from_bytes([3u8; 32]);

        let mut ps_a = ProducerSet::new();
        let mut ps_b = ProducerSet::new();

        let bond_unit = 1_000_000_000u64;

        // Insert in order 1, 2, 3
        let _ = ps_a.register_genesis_producer(pk1, 1, bond_unit);
        let _ = ps_a.register_genesis_producer(pk2, 1, bond_unit);
        let _ = ps_a.register_genesis_producer(pk3, 1, bond_unit);

        // Insert in order 3, 1, 2
        let _ = ps_b.register_genesis_producer(pk3, 1, bond_unit);
        let _ = ps_b.register_genesis_producer(pk1, 1, bond_unit);
        let _ = ps_b.register_genesis_producer(pk2, 1, bond_unit);

        let bytes_a = ps_a.serialize_canonical();
        let bytes_b = ps_b.serialize_canonical();

        assert_eq!(
            bytes_a, bytes_b,
            "ProducerSets with same data in different insertion order must produce identical canonical bytes"
        );

        // State roots must also match
        let cs = ChainState::new(Hash::zero());
        let utxo = UtxoSet::new();

        let root_a = compute_state_root(&cs, &utxo, &ps_a).unwrap();
        let root_b = compute_state_root(&cs, &utxo, &ps_b).unwrap();

        assert_eq!(
            root_a, root_b,
            "State roots must be identical regardless of insertion order"
        );
    }
}
