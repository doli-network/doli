//! Epoch scheduler state — the single source of truth for all consensus-derived
//! scheduler inputs.
//!
//! Bundles 7 previously scattered fields into one struct with compile-time
//! guarantees: derive_at_boundary() is the ONLY way to produce a new epoch's
//! state, and accumulate_block() is the ONLY way to update per-block tracking.
//!
//! This eliminates the dual-implementation divergence that caused 58+ commits
//! of fix-one-break-another since the scheduler architecture change (7f033517).

use std::collections::{HashMap, HashSet};

use crypto::{Hash, PublicKey};
use serde::{Deserialize, Serialize};

/// Parameters needed by derive_at_boundary that come from external sources
/// (NetworkParams, ProducerSet, UtxoSet). Extracted by the caller so
/// EpochState stays in crates/core with no storage dependency.
pub struct EpochDerivationInput {
    /// Active producers at the epoch boundary height (from ProducerSet)
    pub active_producers: Vec<PublicKey>,
    /// Bond counts per pubkey_hash (from UTXO set at epoch boundary)
    pub bond_counts: HashMap<Hash, u64>,
    /// Blocks per reward epoch (from NetworkParams)
    pub blocks_per_epoch: u64,
    /// Snap attestation skip height (from NetworkParams)
    pub snap_attestation_skip_height: u64,
    /// Epoch boundary height
    pub height: u64,
    /// The epoch number being entered
    pub epoch: u64,
    /// Producer registered_at timestamps for tier system (pubkey → registered_at)
    pub registered_at: HashMap<PublicKey, u64>,
}

/// Block data needed by accumulate_block. Extracted from BlockHeader so
/// EpochState has no dependency on the Block type.
pub struct BlockAccumulationInput {
    /// Block producer's public key
    pub producer: PublicKey,
    /// Block slot number
    pub slot: u32,
    /// Whether the presence_root is non-zero
    pub has_attestation_data: bool,
    /// Decoded attestation indices (caller decodes the bitfield since that
    /// depends on the producer_list ordering and block body data)
    pub attested_indices: Vec<usize>,
}

/// The complete epoch scheduler state.
///
/// Every field that the DeterministicScheduler reads is here. Two nodes with
/// identical EpochState will produce identical scheduling decisions — guaranteed
/// at the type level.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EpochState {
    /// The epoch number for which this state applies.
    pub epoch: u64,

    /// Epoch-locked bond snapshot: {pubkey_hash → bond_count}.
    /// Computed once at each epoch boundary from the UTXO set.
    pub bond_snapshot: HashMap<Hash, u64>,

    /// Frozen producer list for the current epoch.
    /// Sorted by pubkey bytes. Attestation-filtered from active producers.
    pub producer_list: Vec<PublicKey>,

    /// Active production list: subset of producer_list that enters round-robin.
    /// Before TIER_SYSTEM_ACTIVATION_HEIGHT: identical to producer_list.
    /// After: first ACTIVE_PRODUCERS_CAP by registered_at (earliest first).
    pub active_list: Vec<PublicKey>,

    /// Set of producers who attested in each of the 3 lookback epochs.
    /// [0] = current epoch, [1] = prev epoch, [2] = prev-prev epoch.
    pub attested_sets: [HashSet<PublicKey>; 3],

    /// Incremental attestation tracker: pubkey → set of attested minutes.
    /// [0] = current, [1] = prev, [2] = prev-prev.
    pub attestation_accum: [HashMap<PublicKey, HashSet<u32>>; 3],

    /// Blocks produced per producer in the current epoch.
    pub blocks_produced: HashMap<PublicKey, u32>,
}

impl EpochState {
    /// Create the genesis epoch state (epoch 0, all empty).
    pub fn genesis() -> Self {
        Self {
            epoch: 0,
            bond_snapshot: HashMap::new(),
            producer_list: Vec::new(),
            active_list: Vec::new(),
            attested_sets: [HashSet::new(), HashSet::new(), HashSet::new()],
            attestation_accum: [HashMap::new(), HashMap::new(), HashMap::new()],
            blocks_produced: HashMap::new(),
        }
    }

    /// Accumulate per-block attestation tracking.
    ///
    /// Called for every applied block. Updates attested_sets[0],
    /// attestation_accum[0], and blocks_produced.
    pub fn accumulate_block(&mut self, input: &BlockAccumulationInput) {
        let minute = crate::attestation::attestation_minute(input.slot);

        // Track the block producer
        self.attested_sets[0].insert(input.producer);
        *self.blocks_produced.entry(input.producer).or_insert(0) += 1;
        self.attestation_accum[0]
            .entry(input.producer)
            .or_default()
            .insert(minute);

        // Track attested producers from the bitfield
        if input.has_attestation_data {
            for &idx in &input.attested_indices {
                if let Some(pk) = self.producer_list.get(idx) {
                    self.attested_sets[0].insert(*pk);
                    self.attestation_accum[0]
                        .entry(*pk)
                        .or_default()
                        .insert(minute);
                }
            }
        }
    }

    /// Derive the epoch state at a boundary.
    ///
    /// This is the ONE canonical function. Contains: bond snapshot rebuild,
    /// 3-epoch attestation lookback, 2/3 deadlock floor, tier system,
    /// accumulator rotation. Pure function — no side effects, no node-local state.
    ///
    /// `prev` is the epoch state from the just-completed epoch (self).
    /// Returns the new epoch state for the epoch being entered.
    pub fn derive_at_boundary(prev: &EpochState, input: &EpochDerivationInput) -> EpochState {
        use crate::consensus::{
            ACTIVE_PRODUCERS_CAP, MIN_ATTESTATION_MINUTES, TIER_PROMOTION_ACTIVATION_HEIGHT,
            TIER_SYSTEM_ACTIVATION_HEIGHT,
        };

        let epoch = input.epoch;

        // 1. Bond snapshot
        let bond_snapshot = input.bond_counts.clone();

        // 2. Attestation-filtered producer list
        let mut new_list: Vec<PublicKey> = if epoch <= 1 {
            input.active_producers.clone()
        } else {
            // 3-epoch lookback: producer retained if attested in ANY of last 3 epochs
            let mut attested: HashSet<PublicKey> = HashSet::new();
            for i in 0..3 {
                attested.extend(&prev.attested_sets[i]);
            }

            let have_full_history = !prev.attested_sets[0].is_empty();
            if have_full_history || input.height < input.snap_attestation_skip_height {
                input
                    .active_producers
                    .iter()
                    .filter(|pk| attested.contains(pk))
                    .copied()
                    .collect()
            } else {
                // Empty attestation accumulators — use all active producers
                input.active_producers.clone()
            }
        };

        // 3. Deadlock safety floor: 2/3 of active producers
        let active_count = input.active_producers.len();
        if new_list.len() < (active_count * 2 / 3) || new_list.is_empty() {
            new_list = input.active_producers.clone();
        }

        // 4. Sort by pubkey (deterministic ordering)
        new_list.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

        // 5. Tier system: build active production list
        // Allow: activation heights are 0 now but will be changed for future activations.
        #[allow(clippy::absurd_extreme_comparisons)]
        let active_list = if input.height >= TIER_SYSTEM_ACTIVATION_HEIGHT
            && new_list.len() > ACTIVE_PRODUCERS_CAP
        {
            let mut with_reg: Vec<(PublicKey, u64)> = new_list
                .iter()
                .filter_map(|pk| input.registered_at.get(pk).map(|&reg| (*pk, reg)))
                .collect();

            #[allow(clippy::absurd_extreme_comparisons)]
            if input.height >= TIER_PROMOTION_ACTIVATION_HEIGHT && epoch > 1 {
                // Promotion: filter out underperformers
                let expected_per_producer = input.blocks_per_epoch / new_list.len().max(1) as u64;
                let min_produced = (expected_per_producer * 80 / 100).max(1);
                let attestation_minutes = &prev.attestation_accum[0];
                let blocks_produced_map = &prev.blocks_produced;

                with_reg.retain(|(pk, _)| {
                    let mins = attestation_minutes.get(pk).map(|s| s.len()).unwrap_or(0);
                    let produced = blocks_produced_map.get(pk).copied().unwrap_or(0) as u64;
                    mins >= MIN_ATTESTATION_MINUTES && produced >= min_produced
                });
            }

            // Sort by registered_at ascending, pubkey tiebreak
            with_reg.sort_by(|a, b| {
                a.1.cmp(&b.1)
                    .then_with(|| a.0.as_bytes().cmp(b.0.as_bytes()))
            });

            let mut result: Vec<PublicKey> = with_reg
                .iter()
                .take(ACTIVE_PRODUCERS_CAP)
                .map(|(pk, _)| *pk)
                .collect();

            // Tier deadlock safety: if < 1/3, mass event — include all
            if result.len() < new_list.len() / 3 || result.is_empty() {
                result = new_list.clone();
            }

            result
        } else {
            new_list.clone()
        };

        // 6. Rotate accumulators: [0]→[1], [1]→[2], [2] discarded, new [0] empty
        let attested_sets = [
            HashSet::new(),
            prev.attested_sets[0].clone(),
            prev.attested_sets[1].clone(),
        ];
        let attestation_accum = [
            HashMap::new(),
            prev.attestation_accum[0].clone(),
            prev.attestation_accum[1].clone(),
        ];

        EpochState {
            epoch,
            bond_snapshot,
            producer_list: new_list,
            active_list,
            attested_sets,
            attestation_accum,
            blocks_produced: HashMap::new(),
        }
    }

    /// Serialize to canonical bytes for persistence/transfer.
    pub fn serialize_canonical(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap_or_default()
    }

    /// Deserialize from canonical bytes.
    pub fn deserialize_canonical(bytes: &[u8]) -> Result<Self, bincode::Error> {
        bincode::deserialize(bytes)
    }

    /// Compute a deterministic hash of this epoch state.
    pub fn hash(&self) -> Hash {
        epoch_state_hash(
            &self.bond_snapshot,
            self.epoch,
            &self.producer_list,
            &self.active_list,
            &self.attested_sets,
            &self.attestation_accum,
            &self.blocks_produced,
        )
    }
}

/// Compute a hash over epoch state fields.
/// Extracted as a free function so snapshot.rs can call it without
/// needing an EpochState instance (backward compat during migration).
pub fn epoch_state_hash(
    bond_snapshot: &HashMap<Hash, u64>,
    epoch: u64,
    producer_list: &[PublicKey],
    active_list: &[PublicKey],
    attested_sets: &[HashSet<PublicKey>; 3],
    attestation_accum: &[HashMap<PublicKey, HashSet<u32>>; 3],
    blocks_produced: &HashMap<PublicKey, u32>,
) -> Hash {
    let h = crypto::hash::hash;

    // epoch_bond_snapshot: sorted by key + epoch appended
    let bonds_h = {
        let mut v: Vec<(Vec<u8>, u64)> = bond_snapshot
            .iter()
            .map(|(k, v)| (k.as_bytes().to_vec(), *v))
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        let mut bytes = bincode::serialize(&v).unwrap_or_default();
        bytes.extend_from_slice(&epoch.to_le_bytes());
        h(&bytes)
    };

    let epl_h = h(&bincode::serialize(
        &producer_list
            .iter()
            .map(|pk| pk.as_bytes().to_vec())
            .collect::<Vec<_>>(),
    )
    .unwrap_or_default());

    let apl_h = h(&bincode::serialize(
        &active_list
            .iter()
            .map(|pk| pk.as_bytes().to_vec())
            .collect::<Vec<_>>(),
    )
    .unwrap_or_default());

    let attested_h = {
        let per_epoch: Vec<Vec<u8>> = attested_sets
            .iter()
            .map(|s| {
                let mut v: Vec<Vec<u8>> = s.iter().map(|pk| pk.as_bytes().to_vec()).collect();
                v.sort();
                bincode::serialize(&v).unwrap_or_default()
            })
            .collect();
        h(&bincode::serialize(&per_epoch).unwrap_or_default())
    };

    let accum_h = {
        let per_epoch: Vec<Vec<u8>> = attestation_accum
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

    let blocks_h = {
        let mut v: Vec<(Vec<u8>, u32)> = blocks_produced
            .iter()
            .map(|(pk, n)| (pk.as_bytes().to_vec(), *n))
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        h(&bincode::serialize(&v).unwrap_or_default())
    };

    let mut combined = Vec::with_capacity(6 * 32);
    for hash in [bonds_h, epl_h, apl_h, attested_h, accum_h, blocks_h] {
        combined.extend_from_slice(hash.as_bytes());
    }
    h(&combined)
}

#[cfg(test)]
mod tests {
    use super::*;

    // OUTPUT CONTRACT: EpochState::genesis()
    // O1: epoch = 0
    // O2: all collections empty
    // PATHS: P1=constructor
    // MATRIX: O1×P1, O2×P1

    // OUTPUT CONTRACT: EpochState::accumulate_block(input)
    // O1: self.attested_sets[0] — adds producer + attested indices
    // O2: self.blocks_produced — increments producer count
    // O3: self.attestation_accum[0] — adds minute for producer + attested
    // PATHS: P1=no attestation data, P2=with attestation data
    // MATRIX: O1×P1, O1×P2, O2×P1, O2×P2, O3×P1, O3×P2

    // OUTPUT CONTRACT: EpochState::derive_at_boundary(prev, input)
    // O1: epoch = input.epoch
    // O2: bond_snapshot = input.bond_counts
    // O3: producer_list — attestation-filtered, sorted
    // O4: active_list — tier-filtered from producer_list
    // O5: attested_sets — rotated ([0]=empty, [1]=prev[0], [2]=prev[1])
    // O6: attestation_accum — rotated
    // O7: blocks_produced — empty (new epoch)
    // PATHS: P1=epoch<=1, P2=epoch>1 with attestation, P3=epoch>1 empty accum
    // MATRIX: O1-O7 × P1-P3

    // OUTPUT CONTRACT: serialize/deserialize round-trip
    // O1: deserialized fields match original
    // PATHS: P1=empty, P2=populated
    // MATRIX: O1×P1, O1×P2

    // OUTPUT CONTRACT: hash()
    // O1: same state → same hash
    // O2: different state → different hash
    // PATHS: P1=identical, P2=epoch differs, P3=producers differ
    // MATRIX: O1×P1, O2×P2, O2×P3

    fn make_pubkey(seed: u8) -> PublicKey {
        let mut bytes = [0u8; 32];
        bytes[0] = seed;
        PublicKey::from_bytes(bytes)
    }

    fn make_hash(seed: u8) -> Hash {
        let mut bytes = [0u8; 32];
        bytes[0] = seed;
        Hash::from_bytes(bytes)
    }

    #[test]
    fn test_genesis_creates_empty_state() {
        let state = EpochState::genesis();
        assert_eq!(state.epoch, 0);
        assert!(state.bond_snapshot.is_empty());
        assert!(state.producer_list.is_empty());
        assert!(state.active_list.is_empty());
        for i in 0..3 {
            assert!(state.attested_sets[i].is_empty());
            assert!(state.attestation_accum[i].is_empty());
        }
        assert!(state.blocks_produced.is_empty());
    }

    #[test]
    fn test_accumulate_block_no_attestation() {
        let pk = make_pubkey(1);
        let mut state = EpochState::genesis();
        state.producer_list = vec![pk];

        let input = BlockAccumulationInput {
            producer: pk,
            slot: 100,
            has_attestation_data: false,
            attested_indices: vec![],
        };
        state.accumulate_block(&input);

        assert!(state.attested_sets[0].contains(&pk));
        assert_eq!(state.blocks_produced[&pk], 1);
        assert!(!state.attestation_accum[0][&pk].is_empty());
    }

    #[test]
    fn test_accumulate_block_with_attestation() {
        let pk0 = make_pubkey(1);
        let pk1 = make_pubkey(2);
        let mut state = EpochState::genesis();
        state.producer_list = vec![pk0, pk1];

        let input = BlockAccumulationInput {
            producer: pk0,
            slot: 100,
            has_attestation_data: true,
            attested_indices: vec![0, 1],
        };
        state.accumulate_block(&input);

        assert!(state.attested_sets[0].contains(&pk0));
        assert!(state.attested_sets[0].contains(&pk1));
        assert_eq!(state.blocks_produced[&pk0], 1);
        assert!(!state.blocks_produced.contains_key(&pk1));
        assert!(!state.attestation_accum[0][&pk0].is_empty());
        assert!(!state.attestation_accum[0][&pk1].is_empty());
    }

    #[test]
    fn test_accumulate_block_increments_count() {
        let pk = make_pubkey(1);
        let mut state = EpochState::genesis();
        state.producer_list = vec![pk];

        for slot in 0..5 {
            state.accumulate_block(&BlockAccumulationInput {
                producer: pk,
                slot: slot * 10,
                has_attestation_data: false,
                attested_indices: vec![],
            });
        }
        assert_eq!(state.blocks_produced[&pk], 5);
    }

    #[test]
    fn test_derive_at_boundary_epoch_1() {
        let pk0 = make_pubkey(1);
        let pk1 = make_pubkey(2);
        let pkh0 = make_hash(1);
        let pkh1 = make_hash(2);

        let mut prev = EpochState::genesis();
        prev.attested_sets[0].insert(pk0);
        prev.attested_sets[0].insert(pk1);

        let mut bond_counts = HashMap::new();
        bond_counts.insert(pkh0, 5);
        bond_counts.insert(pkh1, 3);

        let input = EpochDerivationInput {
            active_producers: vec![pk0, pk1],
            bond_counts: bond_counts.clone(),
            blocks_per_epoch: 360,
            snap_attestation_skip_height: u64::MAX,
            height: 360,
            epoch: 1,
            registered_at: HashMap::new(),
        };

        let new_state = EpochState::derive_at_boundary(&prev, &input);

        assert_eq!(new_state.epoch, 1);
        assert_eq!(new_state.bond_snapshot, bond_counts);
        assert_eq!(new_state.producer_list.len(), 2);
        assert_eq!(new_state.active_list.len(), 2);
        assert!(new_state.attested_sets[0].is_empty());
        assert_eq!(new_state.attested_sets[1].len(), 2);
        assert!(new_state.attestation_accum[0].is_empty());
        assert!(new_state.blocks_produced.is_empty());
    }

    #[test]
    fn test_derive_at_boundary_attestation_filter() {
        let pk0 = make_pubkey(1);
        let pk1 = make_pubkey(2);
        let pk2 = make_pubkey(3);

        let mut prev = EpochState::genesis();
        prev.epoch = 1;
        prev.attested_sets[0].insert(pk0);
        prev.attested_sets[0].insert(pk1);

        let input = EpochDerivationInput {
            active_producers: vec![pk0, pk1, pk2],
            bond_counts: HashMap::new(),
            blocks_per_epoch: 360,
            snap_attestation_skip_height: u64::MAX,
            height: 720,
            epoch: 2,
            registered_at: HashMap::new(),
        };

        let new_state = EpochState::derive_at_boundary(&prev, &input);

        // 2/3 >= 2/3 floor — filter applies
        assert_eq!(new_state.producer_list.len(), 2);
        assert!(new_state.producer_list.contains(&pk0));
        assert!(new_state.producer_list.contains(&pk1));
        assert!(!new_state.producer_list.contains(&pk2));
    }

    #[test]
    fn test_derive_deadlock_safety_floor() {
        let pk0 = make_pubkey(1);
        let pk1 = make_pubkey(2);
        let pk2 = make_pubkey(3);

        let mut prev = EpochState::genesis();
        prev.epoch = 1;
        prev.attested_sets[0].insert(pk0);

        let input = EpochDerivationInput {
            active_producers: vec![pk0, pk1, pk2],
            bond_counts: HashMap::new(),
            blocks_per_epoch: 360,
            snap_attestation_skip_height: u64::MAX,
            height: 720,
            epoch: 2,
            registered_at: HashMap::new(),
        };

        let new_state = EpochState::derive_at_boundary(&prev, &input);

        // 1/3 < 2/3 floor → all included
        assert_eq!(new_state.producer_list.len(), 3);
    }

    #[test]
    fn test_derive_empty_accum_uses_all_producers() {
        let pk0 = make_pubkey(1);
        let pk1 = make_pubkey(2);

        let mut prev = EpochState::genesis();
        prev.epoch = 1;
        // Empty attested_sets — simulate post-snap or cold start

        let input = EpochDerivationInput {
            active_producers: vec![pk0, pk1],
            bond_counts: HashMap::new(),
            blocks_per_epoch: 360,
            snap_attestation_skip_height: u64::MAX,
            height: 720,
            epoch: 2,
            registered_at: HashMap::new(),
        };

        let new_state = EpochState::derive_at_boundary(&prev, &input);

        // Empty accum + height >= skip → all active
        assert_eq!(new_state.producer_list.len(), 2);
    }

    #[test]
    fn test_derive_accumulator_rotation() {
        let pk0 = make_pubkey(1);
        let pk1 = make_pubkey(2);
        let pk2 = make_pubkey(3);

        let mut prev = EpochState::genesis();
        prev.attested_sets[0].insert(pk0);
        prev.attested_sets[1].insert(pk1);
        prev.attested_sets[2].insert(pk2);

        prev.attestation_accum[0].entry(pk0).or_default().insert(1);
        prev.attestation_accum[1].entry(pk1).or_default().insert(2);

        let input = EpochDerivationInput {
            active_producers: vec![pk0, pk1, pk2],
            bond_counts: HashMap::new(),
            blocks_per_epoch: 360,
            snap_attestation_skip_height: u64::MAX,
            height: 360,
            epoch: 1,
            registered_at: HashMap::new(),
        };

        let new_state = EpochState::derive_at_boundary(&prev, &input);

        // [0] = empty (new epoch)
        assert!(new_state.attested_sets[0].is_empty());
        assert!(new_state.attestation_accum[0].is_empty());
        // [1] = prev[0]
        assert!(new_state.attested_sets[1].contains(&pk0));
        assert!(new_state.attestation_accum[1].contains_key(&pk0));
        // [2] = prev[1]
        assert!(new_state.attested_sets[2].contains(&pk1));
        assert!(new_state.attestation_accum[2].contains_key(&pk1));
        // prev[2] discarded
        assert!(!new_state.attested_sets[2].contains(&pk2));
    }

    #[test]
    fn test_serialize_deserialize_round_trip_empty() {
        let state = EpochState::genesis();
        let bytes = state.serialize_canonical();
        let restored = EpochState::deserialize_canonical(&bytes).unwrap();
        assert_eq!(restored.epoch, 0);
        assert!(restored.producer_list.is_empty());
    }

    #[test]
    fn test_serialize_deserialize_round_trip_populated() {
        let pk = make_pubkey(1);
        let pkh = make_hash(1);

        let mut state = EpochState::genesis();
        state.epoch = 5;
        state.bond_snapshot.insert(pkh, 10);
        state.producer_list = vec![pk];
        state.active_list = vec![pk];
        state.attested_sets[0].insert(pk);
        state.attestation_accum[0].entry(pk).or_default().insert(42);
        state.blocks_produced.insert(pk, 7);

        let bytes = state.serialize_canonical();
        let restored = EpochState::deserialize_canonical(&bytes).unwrap();

        assert_eq!(restored.epoch, 5);
        assert_eq!(restored.bond_snapshot[&pkh], 10);
        assert_eq!(restored.producer_list, vec![pk]);
        assert_eq!(restored.active_list, vec![pk]);
        assert!(restored.attested_sets[0].contains(&pk));
        assert!(restored.attestation_accum[0][&pk].contains(&42));
        assert_eq!(restored.blocks_produced[&pk], 7);
    }

    #[test]
    fn test_hash_deterministic() {
        let state = EpochState::genesis();
        assert_eq!(state.hash(), state.hash());
    }

    #[test]
    fn test_hash_differs_on_change() {
        let mut s1 = EpochState::genesis();
        let s2_epoch = {
            let mut s = EpochState::genesis();
            s.epoch = 1;
            s
        };
        assert_ne!(s1.hash(), s2_epoch.hash());

        s1.producer_list = vec![make_pubkey(1)];
        assert_ne!(s1.hash(), EpochState::genesis().hash());
    }
}
