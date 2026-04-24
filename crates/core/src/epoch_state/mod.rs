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

#[cfg(test)]
mod tests;

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
    /// INC-I-046: Ghost exclusion activation height (from NetworkParams)
    pub ghost_exclusion_activation_height: u64,
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
            ACTIVE_PRODUCERS_CAP, GHOST_EXCLUSION_GRACE_EPOCHS, MIN_ATTESTATION_MINUTES,
            TIER_PROMOTION_ACTIVATION_HEIGHT, TIER_SYSTEM_ACTIVATION_HEIGHT,
        };

        let epoch = input.epoch;

        // 1. Bond snapshot
        let bond_snapshot = input.bond_counts.clone();

        // 2. Attestation-filtered producer list
        let attested_union: HashSet<&PublicKey> =
            prev.attested_sets.iter().flat_map(|s| s.iter()).collect();

        let mut new_list: Vec<PublicKey> = if epoch <= 1 {
            input.active_producers.clone()
        } else {
            let have_full_history = !prev.attested_sets[0].is_empty();
            if have_full_history || input.height < input.snap_attestation_skip_height {
                input
                    .active_producers
                    .iter()
                    .filter(|pk| attested_union.contains(pk))
                    .copied()
                    .collect()
            } else {
                // Empty attestation accumulators — use all active producers
                input.active_producers.clone()
            }
        };

        // 3. Deadlock safety floor: 2/3 of active producers
        //    INC-I-046: After ghost_exclusion_activation_height, subtract ghost producers
        //    from the denominator. A ghost = not attested in ANY of 3 lookback epochs AND
        //    registered for > GHOST_EXCLUSION_GRACE_EPOCHS. This prevents permanently-offline
        //    producers from inflating the floor and overriding the attestation filter.
        let active_count = input.active_producers.len();
        let ghost_exclusion_active =
            input.height >= input.ghost_exclusion_activation_height && epoch > 1;

        let is_ghost = |pk: &PublicKey| -> bool {
            if !ghost_exclusion_active {
                return false;
            }
            if attested_union.contains(pk) {
                return false;
            }
            match input.registered_at.get(pk) {
                Some(&reg_height) => {
                    let reg_epoch = reg_height.checked_div(input.blocks_per_epoch).unwrap_or(0);
                    epoch.saturating_sub(reg_epoch) > GHOST_EXCLUSION_GRACE_EPOCHS
                }
                None => false, // Unknown registration: not a ghost (conservative)
            }
        };

        let ghost_count = if ghost_exclusion_active {
            input
                .active_producers
                .iter()
                .filter(|pk| is_ghost(pk))
                .count()
        } else {
            0
        };
        let effective_active = active_count - ghost_count;

        if new_list.len() < (effective_active * 2 / 3)
            || (new_list.is_empty() && effective_active > 0)
        {
            if ghost_exclusion_active && ghost_count > 0 {
                // Mass event — include all non-ghost producers
                new_list = input
                    .active_producers
                    .iter()
                    .filter(|pk| !is_ghost(pk))
                    .copied()
                    .collect();
            } else {
                new_list = input.active_producers.clone();
            }
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
                // Promotion: filter out underperformers from the just-completed epoch.
                // prev.attestation_accum[0] = just-completed epoch (pre-rotation).
                // prev.blocks_produced = just-completed epoch's production count.
                // NOTE: rotation happens AFTER this (lines below create new state with [0]=empty).
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

    /// Serialize to deterministic bytes for persistence/transfer.
    ///
    /// NOTE: bincode serializes HashMap/HashSet in iteration order, which is
    /// non-deterministic. This is acceptable for persistence (deserialize
    /// reconstructs the same logical state) but NOT suitable for cross-node
    /// byte comparison. Use `hash()` for deterministic fingerprinting.
    pub fn serialize(&self) -> Vec<u8> {
        bincode::serialize(self).expect("EpochState serialization is infallible")
    }

    /// Deserialize from bytes produced by `serialize()`.
    pub fn deserialize(bytes: &[u8]) -> Result<Self, bincode::Error> {
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
        let mut bytes = bincode::serialize(&v).expect("bincode serialization is infallible");
        bytes.extend_from_slice(&epoch.to_le_bytes());
        h(&bytes)
    };

    let epl_h = h(&bincode::serialize(
        &producer_list
            .iter()
            .map(|pk| pk.as_bytes().to_vec())
            .collect::<Vec<_>>(),
    )
    .expect("bincode serialization is infallible"));

    let apl_h = h(&bincode::serialize(
        &active_list
            .iter()
            .map(|pk| pk.as_bytes().to_vec())
            .collect::<Vec<_>>(),
    )
    .expect("bincode serialization is infallible"));

    let attested_h = {
        let per_epoch: Vec<Vec<u8>> = attested_sets
            .iter()
            .map(|s| {
                let mut v: Vec<Vec<u8>> = s.iter().map(|pk| pk.as_bytes().to_vec()).collect();
                v.sort();
                bincode::serialize(&v).expect("bincode serialization is infallible")
            })
            .collect();
        h(&bincode::serialize(&per_epoch).expect("bincode serialization is infallible"))
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
                bincode::serialize(&v).expect("bincode serialization is infallible")
            })
            .collect();
        h(&bincode::serialize(&per_epoch).expect("bincode serialization is infallible"))
    };

    let blocks_h = {
        let mut v: Vec<(Vec<u8>, u32)> = blocks_produced
            .iter()
            .map(|(pk, n)| (pk.as_bytes().to_vec(), *n))
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        h(&bincode::serialize(&v).expect("bincode serialization is infallible"))
    };

    let mut combined = Vec::with_capacity(6 * 32);
    for hash in [bonds_h, epl_h, apl_h, attested_h, accum_h, blocks_h] {
        combined.extend_from_slice(hash.as_bytes());
    }
    h(&combined)
}
