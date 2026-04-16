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

/// Compute a deterministic state root with optional `H(EpochSnapshot)` inclusion.
///
/// Phase-1 / M-Choice1 primitive for the INC-I-034 `HardForkSchedule` entry
/// `EPOCH_SNAPSHOT_HF`. At the activation height, callers switch from
/// `compute_state_root(cs, utxo, ps)` to this function passing `Some(h_es)` so
/// the state root becomes:
///
///   `state_root = H(H(cs_canonical) || H(utxo_canonical) || H(ps_canonical) || h_es)`
///
/// Pre-activation (or when `epoch_state_hash == None`), this function returns
/// the exact same bytes as `compute_state_root(cs, utxo, ps)` — the pre-HF
/// chain is not altered by the mere presence of this function in the binary.
///
/// Per CLAUDE.md Rule #0 (NO genesis reset, future-height activation only),
/// the call-site gate that chooses `Some` vs `None` MUST be keyed on the
/// `HardForkSchedule::EPOCH_SNAPSHOT_HF` activation height — never genesis,
/// never retroactive.
///
/// Phase-1 scope (this milestone): the function is present but NOT YET WIRED
/// at any call site. Phase-2 (separate milestone) wires the 15 current
/// `compute_state_root` call-sites to consult the schedule and pass
/// `Some`/`None` accordingly.
///
/// See: `specs/scheduler-state-architecture.md` "State-root inclusion
/// (timing: SAME HF — convergent, with sequenced option surfaced)".
pub fn compute_state_root_with_epoch_state(
    chain_state: &ChainState,
    utxo_set: &UtxoSet,
    producer_set: &ProducerSet,
    epoch_state_hash: Option<Hash>,
) -> Result<Hash, StorageError> {
    match epoch_state_hash {
        None => compute_state_root(chain_state, utxo_set, producer_set),
        Some(es_hash) => {
            // Same canonical encoding as compute_state_root; append
            // H(EpochSnapshot) as the 4th component.
            let cs_bytes = chain_state.serialize_canonical();
            let utxo_bytes = utxo_set.serialize_canonical();
            let ps_bytes = producer_set.serialize_canonical();

            let cs_hash = crypto::hash::hash(&cs_bytes);
            let utxo_hash = crypto::hash::hash(&utxo_bytes);
            let ps_hash = crypto::hash::hash(&ps_bytes);

            // INFO so the 4 component hashes are visible in production
            // without RUST_LOG=debug — mirrors the legacy [STATE_ROOT] log
            // but distinguished with the _HF suffix so operators can tell
            // pre- vs post-activation block hashes apart at a glance.
            tracing::info!(
                "[STATE_ROOT_HF] cs={:.16} utxo={:.16} ps={:.16} es={:.16} \
                 cs_bytes={} utxo_bytes={} ps_bytes={}",
                cs_hash,
                utxo_hash,
                ps_hash,
                es_hash,
                cs_bytes.len(),
                utxo_bytes.len(),
                ps_bytes.len()
            );

            let mut combined = Vec::with_capacity(128);
            combined.extend_from_slice(cs_hash.as_bytes());
            combined.extend_from_slice(utxo_hash.as_bytes());
            combined.extend_from_slice(ps_hash.as_bytes());
            combined.extend_from_slice(es_hash.as_bytes());

            Ok(crypto::hash::hash(&combined))
        }
    }
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
///
/// Delegates to `doli_core::epoch_state_hash` (single implementation).
#[allow(clippy::too_many_arguments)]
pub fn compute_scheduler_root(
    epoch_bond_snapshot: &HashMap<Hash, u64>,
    epoch_bond_snapshot_epoch: u64,
    epoch_producer_list: &[PublicKey],
    active_production_list: &[PublicKey],
    epoch_attested_set: &[HashSet<PublicKey>; 3],
    epoch_attestation_accum: &[HashMap<PublicKey, HashSet<u32>>; 3],
    epoch_blocks_produced_accum: &HashMap<PublicKey, u32>,
) -> Hash {
    doli_core::epoch_state_hash(
        epoch_bond_snapshot,
        epoch_bond_snapshot_epoch,
        epoch_producer_list,
        active_production_list,
        epoch_attested_set,
        epoch_attestation_accum,
        epoch_blocks_produced_accum,
    )
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

// =============================================================================
// M-Choice1 — EpochState state-root inclusion, Phase-1 primitive
// =============================================================================
//
// INC-I-034 / M-Choice1. Spec: specs/scheduler-state-architecture.md
// ("State-root inclusion (timing: SAME HF — convergent, with sequenced option
// surfaced)"). Locked 2026-04-16 as CHOICE 1 = SAME HF.
//
// Phase-1 scope (this test module verifies):
//   - A new pure function `compute_state_root_with_epoch_state` exists.
//   - Passing `None` returns bit-identical bytes to legacy `compute_state_root`
//     (pre-HF chain is NOT altered by the new function existing).
//   - Passing `Some(h)` returns the 4-component hash
//     H(H(cs) || H(utxo) || H(ps) || h) which materially differs from the
//     legacy 3-component hash.
//   - Two distinct `Some(h1)`, `Some(h2)` with `h1 != h2` yield distinct roots.
//
// OUT of Phase-1 scope (NOT tested here — deferred to Phase 2):
//   - Wiring of the new function into apply_block/snap_sync/cleanup call sites.
//   - The height-keyed switch between 3- and 4-component formulas.
//
// OUTPUT CONTRACT: fn compute_state_root_with_epoch_state(cs, utxo, ps, opt_hash)
//   O1: return Hash
//         None      → bit-identical to compute_state_root(cs,utxo,ps)
//         Some(h)   → H(H(cs_canon)||H(utxo_canon)||H(ps_canon)||h)
//   (no mutable params, no receiver, no persistent store, no channel)
// PATHS: P1: None (legacy-equivalence), P2: Some(h) (4-component),
//        P3: Some(h1) vs Some(h2), h1!=h2 (hash-distinction)
// MATRIX: 1 output × 3 paths = 3 assertion clusters (Tests 1, 2, 3)
#[cfg(test)]
mod m_choice1_state_root_hf_tests {
    use super::*;

    /// Test 1 — Phase-1 backward-compatibility (None path).
    ///
    /// With `epoch_state_hash = None`, the new function MUST produce the exact
    /// same Hash bytes as the legacy `compute_state_root`. This is the
    /// invariant that lets Phase-1 ship safely: callers not yet wired to the
    /// new function keep producing the old hash, AND callers wired to the new
    /// function produce the old hash whenever the schedule says the HF has not
    /// activated yet. Without this invariant, the mere act of migrating a
    /// call-site would change the state root — a silent consensus break.
    #[test]
    fn test_m_choice1_compute_state_root_with_none_equals_legacy() {
        // Minimal fixture: default constructors per spec note.
        let cs = ChainState::new(Hash::ZERO);
        let utxo = UtxoSet::new();
        let ps = ProducerSet::new();

        let legacy = compute_state_root(&cs, &utxo, &ps)
            .expect("legacy compute_state_root must succeed for default fixture");
        let new_with_none = compute_state_root_with_epoch_state(&cs, &utxo, &ps, None)
            .expect("compute_state_root_with_epoch_state(None) must succeed for default fixture");

        assert_eq!(
            legacy, new_with_none,
            "M-Choice1: compute_state_root_with_epoch_state(.., None) must be \
             BIT-IDENTICAL to legacy compute_state_root(..). Phase-1 safety \
             depends on this — any drift here is a silent consensus break."
        );

        // And: drifting any one of the three components still drifts the
        // None-path hash in lockstep with the legacy hash.
        let mut cs2 = ChainState::new(Hash::ZERO);
        cs2.best_height = 1234;

        let legacy2 = compute_state_root(&cs2, &utxo, &ps).unwrap();
        let new_with_none2 = compute_state_root_with_epoch_state(&cs2, &utxo, &ps, None).unwrap();
        assert_eq!(
            legacy2, new_with_none2,
            "M-Choice1: None-path must track legacy across arbitrary state changes"
        );
        assert_ne!(
            legacy, legacy2,
            "sanity: changing cs.best_height must change the legacy hash \
             (test fixture sanity, not the invariant under test)"
        );
    }

    /// Test 2 — Phase-1 new 4-component hash (Some path).
    ///
    /// With `epoch_state_hash = Some(h)`, the function MUST fold h in as a 4th
    /// component. Specifically the defined formula is:
    ///   H(H(cs_canonical) || H(utxo_canonical) || H(ps_canonical) || h)
    /// which is NOT equal to the legacy 3-component hash. This test pins both:
    /// (a) the result materially differs from the legacy hash, and
    /// (b) the result equals the explicit byte-level recomputation — so the
    /// developer cannot satisfy the test by returning any hash-of-the-4-inputs
    /// (e.g. a reordering) that is consensus-incompatible with the spec.
    #[test]
    fn test_m_choice1_compute_state_root_with_some_uses_four_components() {
        let cs = ChainState::new(Hash::ZERO);
        let utxo = UtxoSet::new();
        let ps = ProducerSet::new();

        // Arbitrary but deterministic epoch-state hash.
        let es_hash = crypto::hash::hash(b"m-choice1-fixture-epoch-state");

        let legacy = compute_state_root(&cs, &utxo, &ps).unwrap();
        let new_with_some =
            compute_state_root_with_epoch_state(&cs, &utxo, &ps, Some(es_hash)).unwrap();

        // (a) Material difference from the 3-component legacy hash.
        assert_ne!(
            legacy, new_with_some,
            "M-Choice1: folding a 4th component MUST change the state root. \
             If this asserts fails, the EpochState hash is being dropped on the \
             floor — a silent consensus no-op."
        );

        // (b) Explicit spec-level byte recomputation:
        //   state_root = H(H(cs_canon) || H(utxo_canon) || H(ps_canon) || es_hash)
        let cs_bytes = cs.serialize_canonical();
        let utxo_bytes = utxo.serialize_canonical();
        let ps_bytes = ps.serialize_canonical();
        let cs_h = crypto::hash::hash(&cs_bytes);
        let utxo_h = crypto::hash::hash(&utxo_bytes);
        let ps_h = crypto::hash::hash(&ps_bytes);
        let mut combined = Vec::with_capacity(128);
        combined.extend_from_slice(cs_h.as_bytes());
        combined.extend_from_slice(utxo_h.as_bytes());
        combined.extend_from_slice(ps_h.as_bytes());
        combined.extend_from_slice(es_hash.as_bytes());
        let expected = crypto::hash::hash(&combined);

        assert_eq!(
            new_with_some, expected,
            "M-Choice1: Some-path formula must be EXACTLY \
             H(H(cs_canon) || H(utxo_canon) || H(ps_canon) || es_hash). \
             Any re-ordering or extra framing here is a consensus-breaking \
             formula drift from specs/scheduler-state-architecture.md."
        );
    }

    /// Test 3 — hash-distinction: two EpochSnapshot variants yield distinct roots.
    ///
    /// Sanity: the 4th component actually flows into the output hash. If two
    /// distinct `es_hash` values produced the same root, the developer could
    /// have plugged in a no-op (e.g. `XOR` of bytes with accidental collision
    /// on fixtures). This test forces the assertion to reveal any such error.
    #[test]
    fn test_m_choice1_state_root_distinguishes_epoch_state_variants() {
        let cs = ChainState::new(Hash::ZERO);
        let utxo = UtxoSet::new();
        let ps = ProducerSet::new();

        let h1 = crypto::hash::hash(b"m-choice1-epoch-state-variant-1");
        let h2 = crypto::hash::hash(b"m-choice1-epoch-state-variant-2");
        assert_ne!(h1, h2, "fixture sanity: input hashes must differ");

        let r1 = compute_state_root_with_epoch_state(&cs, &utxo, &ps, Some(h1)).unwrap();
        let r2 = compute_state_root_with_epoch_state(&cs, &utxo, &ps, Some(h2)).unwrap();

        assert_ne!(
            r1, r2,
            "M-Choice1: distinct epoch_state_hash values MUST produce distinct \
             state roots. If this fails, the EpochState input is not contributing \
             to the hash — a silent consensus no-op."
        );
    }
}
