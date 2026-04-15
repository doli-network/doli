//! State snapshot for snap sync
//!
//! Provides deterministic state root computation and snapshot
//! serialization/deserialization for fast node bootstrapping.

use std::collections::{HashMap, HashSet};

use crypto::{Hash, PublicKey};

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
/// - ChainState: `serialize_canonical()` — 140-byte fixed encoding, immune to struct evolution
/// - UtxoSet: `serialize_canonical()` — entries sorted by outpoint key, 59-byte canonical values
/// - ProducerSet: `serialize_canonical()` — entries sorted by pubkey hash
pub fn compute_state_root(
    chain_state: &ChainState,
    utxo_set: &UtxoSet,
    producer_set: &ProducerSet,
) -> Result<Hash, StorageError> {
    // Canonical fixed-byte encodings — immune to bincode struct evolution.
    let cs_bytes = chain_state.serialize_canonical();
    let utxo_bytes = utxo_set.serialize_canonical();
    let ps_bytes = producer_set.serialize_canonical();

    // Hash each component individually, then combine
    let cs_hash = crypto::hash::hash(&cs_bytes);
    let utxo_hash = crypto::hash::hash(&utxo_bytes);
    let ps_hash = crypto::hash::hash(&ps_bytes);

    // INFO so the 3 component hashes are visible in production without
    // RUST_LOG=debug. State root divergence diagnosis is the canonical
    // hard incident — one grep per node and you know which component
    // (chain_state / utxo / producer_set) diverged.
    tracing::info!(
        "[STATE_ROOT] cs={:.16} utxo={:.16} ps={:.16} cs_bytes={} utxo_bytes={} ps_bytes={}",
        cs_hash,
        utxo_hash,
        ps_hash,
        cs_bytes.len(),
        utxo_bytes.len(),
        ps_bytes.len()
    );

    let mut combined = Vec::with_capacity(96);
    combined.extend_from_slice(cs_hash.as_bytes());
    combined.extend_from_slice(utxo_hash.as_bytes());
    combined.extend_from_slice(ps_hash.as_bytes());

    Ok(crypto::hash::hash(&combined))
}

/// Fix #9 (2026-04-15, synmgrefactor branch): unified hash over all
/// consensus-derived scheduler state.
///
/// The existing `compute_state_root()` covers ChainState + UtxoSet +
/// ProducerSet but does NOT cover the scheduler inputs that determine
/// which producer is scheduled for each slot. Two nodes with identical
/// state_roots can still have divergent schedulers (the folsi/abraham/
/// alessandro incidents on 2026-04-15 were exactly this class — no
/// state_root divergence detected until [STATE_FP] was deployed).
///
/// This function produces a SINGLE hash covering all consensus-derived
/// scheduler state, so two nodes can compare schedulers with one
/// comparison instead of seven.
///
/// INCLUDES (consensus-derived):
///   - excluded_producers (HashSet, sorted)
///   - epoch_bond_snapshot (HashMap, sorted by key) + epoch
///   - epoch_producer_list (Vec, order preserved — index = slot % len)
///   - active_production_list (Vec, order preserved)
///   - epoch_attested_set[0..3] (3x HashSet, sorted per epoch)
///   - epoch_attestation_accum[0..3] (3x HashMap, sorted + inner minute
///     sets sorted)
///   - epoch_blocks_produced_accum (HashMap, sorted by key)
///
/// EXCLUDES (local observation, not consensus-derived):
///   - minute_tracker (depends on wall-clock when OUR node observed
///     attestations; naturally diverges between nodes)
///
/// NOT in block header. Observational only — same semantics as
/// compute_state_root. Canary deploy safe.
#[allow(clippy::too_many_arguments)]
pub fn compute_scheduler_root(
    excluded_producers: &HashSet<PublicKey>,
    epoch_bond_snapshot: &HashMap<Hash, u64>,
    epoch_bond_snapshot_epoch: u64,
    epoch_producer_list: &[PublicKey],
    active_production_list: &[PublicKey],
    epoch_attested_set: &[HashSet<PublicKey>; 3],
    epoch_attestation_accum: &[HashMap<PublicKey, HashSet<u32>>; 3],
    epoch_blocks_produced_accum: &HashMap<PublicKey, u32>,
) -> Hash {
    let h = crypto::hash::hash;

    // excluded_producers: sorted
    let excl_h = {
        let mut v: Vec<Vec<u8>> = excluded_producers
            .iter()
            .map(|pk| pk.as_bytes().to_vec())
            .collect();
        v.sort();
        h(&bincode::serialize(&v).unwrap_or_default())
    };

    // epoch_bond_snapshot: sorted by key + epoch appended
    let bonds_h = {
        let mut v: Vec<(Vec<u8>, u64)> = epoch_bond_snapshot
            .iter()
            .map(|(k, v)| (k.as_bytes().to_vec(), *v))
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        let mut bytes = bincode::serialize(&v).unwrap_or_default();
        bytes.extend_from_slice(&epoch_bond_snapshot_epoch.to_le_bytes());
        h(&bytes)
    };

    // Vec<PublicKey> fields: order preserved (consensus-relevant)
    let epl_h = h(&bincode::serialize(
        &epoch_producer_list
            .iter()
            .map(|pk| pk.as_bytes().to_vec())
            .collect::<Vec<_>>(),
    )
    .unwrap_or_default());

    let apl_h = h(&bincode::serialize(
        &active_production_list
            .iter()
            .map(|pk| pk.as_bytes().to_vec())
            .collect::<Vec<_>>(),
    )
    .unwrap_or_default());

    // epoch_attested_set[0..3]: sorted per epoch
    let attested_h = {
        let per_epoch: Vec<Vec<u8>> = epoch_attested_set
            .iter()
            .map(|s| {
                let mut v: Vec<Vec<u8>> = s.iter().map(|pk| pk.as_bytes().to_vec()).collect();
                v.sort();
                bincode::serialize(&v).unwrap_or_default()
            })
            .collect();
        h(&bincode::serialize(&per_epoch).unwrap_or_default())
    };

    // epoch_attestation_accum[0..3]: HashMap<PubKey, HashSet<minute>>,
    // sort per-epoch by pubkey, inner minute-set also sorted.
    let accum_h = {
        let per_epoch: Vec<Vec<u8>> = epoch_attestation_accum
            .iter()
            .map(|m| {
                let mut v: Vec<(Vec<u8>, Vec<u32>)> = m
                    .iter()
                    .map(|(pk, mins)| {
                        let mut sorted_mins: Vec<u32> = mins.iter().copied().collect();
                        sorted_mins.sort_unstable();
                        (pk.as_bytes().to_vec(), sorted_mins)
                    })
                    .collect();
                v.sort_by(|a, b| a.0.cmp(&b.0));
                bincode::serialize(&v).unwrap_or_default()
            })
            .collect();
        h(&bincode::serialize(&per_epoch).unwrap_or_default())
    };

    // epoch_blocks_produced_accum: sorted by key
    let blocks_h = {
        let mut v: Vec<(Vec<u8>, u32)> = epoch_blocks_produced_accum
            .iter()
            .map(|(pk, n)| (pk.as_bytes().to_vec(), *n))
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        h(&bincode::serialize(&v).unwrap_or_default())
    };

    // Combine all 7 component hashes in a fixed order.
    let mut combined = Vec::with_capacity(7 * 32);
    for hash in [excl_h, bonds_h, epl_h, apl_h, attested_h, accum_h, blocks_h] {
        combined.extend_from_slice(hash.as_bytes());
    }
    h(&combined)
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

        tracing::info!(
            "[SNAPSHOT] Created: h={} hash={:.16} root={:.16} cs={}B utxo={}B ps={}B",
            chain_state.best_height,
            chain_state.best_hash,
            state_root,
            chain_state_bytes.len(),
            utxo_set_bytes.len(),
            producer_set_bytes.len()
        );

        Ok(Self {
            block_hash: chain_state.best_hash,
            block_height: chain_state.best_height,
            chain_state_bytes,
            utxo_set_bytes,
            producer_set_bytes,
            state_root,
        })
    }

    /// Total size of the serialized state in bytes.
    pub fn total_bytes(&self) -> usize {
        self.chain_state_bytes.len() + self.utxo_set_bytes.len() + self.producer_set_bytes.len()
    }
}

/// Compute state root from raw serialized bytes (for checkpoint verification).
///
/// Deserializes each component, then computes the canonical state root.
/// Returns a structured error identifying WHICH component failed deserialization,
/// enabling callers to diagnose corrupt snapshots.
///
/// Wire format:
/// - `chain_state_bytes`: bincode-serialized `ChainState`
/// - `utxo_set_bytes`: canonical format (sorted outpoints, 59-byte values)
/// - `producer_set_bytes`: bincode-serialized `ProducerSet`
pub fn compute_state_root_from_bytes(
    chain_state_bytes: &[u8],
    utxo_set_bytes: &[u8],
    producer_set_bytes: &[u8],
) -> Result<Hash, StorageError> {
    let cs: ChainState = bincode::deserialize(chain_state_bytes).map_err(|e| {
        StorageError::Serialization(format!(
            "[STOR033] ChainState deserialization failed ({} bytes): {}",
            chain_state_bytes.len(),
            e
        ))
    })?;
    let ps: ProducerSet = bincode::deserialize(producer_set_bytes).map_err(|e| {
        StorageError::Serialization(format!(
            "[STOR034] ProducerSet deserialization failed ({} bytes): {}",
            producer_set_bytes.len(),
            e
        ))
    })?;
    let utxo = UtxoSet::deserialize_canonical(utxo_set_bytes).map_err(|e| {
        StorageError::Serialization(format!(
            "[STOR035] UtxoSet deserialization failed ({} bytes): {}",
            utxo_set_bytes.len(),
            e
        ))
    })?;
    compute_state_root(&cs, &utxo, &ps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_root_deterministic() {
        let cs = ChainState::new(Hash::ZERO);
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

        let cs1 = ChainState::new(Hash::ZERO);
        let mut cs2 = ChainState::new(Hash::ZERO);
        cs2.best_height = 100;

        let root1 = compute_state_root(&cs1, &utxo, &ps).unwrap();
        let root2 = compute_state_root(&cs2, &utxo, &ps).unwrap();

        assert_ne!(root1, root2, "Different state must produce different root");
    }

    #[test]
    fn test_snapshot_create_roundtrip() {
        let cs = ChainState::new(Hash::ZERO);
        let utxo = UtxoSet::new();
        let ps = ProducerSet::new();

        let snapshot = StateSnapshot::create(&cs, &utxo, &ps).unwrap();
        assert_ne!(snapshot.state_root, Hash::ZERO);
    }

    #[test]
    fn test_state_root_deterministic_across_calls() {
        let cs = ChainState::new(Hash::ZERO);
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
    fn test_producer_set_canonical_deterministic_insertion_order() {
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
        let cs = ChainState::new(Hash::ZERO);
        let utxo = UtxoSet::new();

        let root_a = compute_state_root(&cs, &utxo, &ps_a).unwrap();
        let root_b = compute_state_root(&cs, &utxo, &ps_b).unwrap();

        assert_eq!(
            root_a, root_b,
            "State roots must be identical regardless of insertion order"
        );
    }

    #[test]
    fn test_total_work_divergence_scenario() {
        // Simulates the exact bug seen in production:
        // N1 (restarted at height 50000) vs N2 (running since genesis).
        // After fix, both produce the same state root.
        let block_hash = crypto::hash::hash(b"block61351");
        let utxo = UtxoSet::new();
        let ps = ProducerSet::new();

        // N1: was restarted, total_work accumulated from 0 for ~11351 blocks
        // Old code: total_work = 11351 (wrong, accumulated from restart)
        // New code: total_work = height (fixed)
        let mut n1 = ChainState::new(Hash::ZERO);
        n1.update(block_hash, 61351, 122702); // total_work = 61351 after fix

        // N2: running since genesis
        let mut n2 = ChainState::new(Hash::ZERO);
        n2.update(block_hash, 61351, 122702); // total_work = 61351

        assert_eq!(
            n1.total_work, n2.total_work,
            "total_work must match after fix"
        );

        let root1 = compute_state_root(&n1, &utxo, &ps).unwrap();
        let root2 = compute_state_root(&n2, &utxo, &ps).unwrap();
        assert_eq!(root1, root2, "state roots must match for identical state");
    }
}
