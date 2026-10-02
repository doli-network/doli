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
/// - UtxoSet: `canonical_digest()` — entries sorted by outpoint key, 61-byte-base values
/// - ProducerSet: `serialize_canonical()` — entries sorted by pubkey hash
pub fn compute_state_root(
    chain_state: &ChainState,
    utxo_set: &UtxoSet,
    producer_set: &ProducerSet,
) -> Result<Hash, StorageError> {
    compute_state_root_gated(chain_state, utxo_set, producer_set, u64::MAX)
}

/// [`compute_state_root`] with the INC-I-181 gate: from `pending_root_ah`
/// (keyed on `chain_state.best_height`) the ProducerSet component also covers
/// a non-empty `pending_updates` queue. `u64::MAX` = the legacy root.
pub fn compute_state_root_gated(
    chain_state: &ChainState,
    utxo_set: &UtxoSet,
    producer_set: &ProducerSet,
    pending_root_ah: u64,
) -> Result<Hash, StorageError> {
    // Canonical fixed-byte encodings — immune to bincode struct evolution.
    let cs_bytes = chain_state.serialize_canonical();
    let (ps_hash, ps_bytes_len) =
        producer_set.state_root_component(chain_state.best_height >= pending_root_ah);

    // Hash each component individually, then combine. The UTXO component is
    // streamed: its canonical image is never materialised.
    let cs_hash = crypto::hash::hash(&cs_bytes);
    let (utxo_hash, utxo_bytes_len) = utxo_set.canonical_digest_and_len()?;

    // F-D0-2 canary seam (State-Root Lazy Tier-0, M1 / B1): emit the full
    // per-component `[STATE_ROOT]` breadcrumb through a reusable helper. In M1
    // this fires on every eager compute (unchanged behavior); the helper is the
    // stable seam epoch-cadence callers use once eager compute is removed in M2.
    log_state_root_components(
        &cs_hash,
        &utxo_hash,
        &ps_hash,
        cs_bytes.len(),
        utxo_bytes_len as usize,
        ps_bytes_len,
    );

    Ok(compose_state_root(&cs_hash, &utxo_hash, &ps_hash))
}

/// Combine the three component hashes into the state root.
///
/// Split out so a caller that already holds the UTXO digest — the M3 [F3]
/// manifest reads it from its pinned view — does not fold the whole set twice.
pub fn compose_state_root(cs_hash: &Hash, utxo_hash: &Hash, ps_hash: &Hash) -> Hash {
    let mut combined = Vec::with_capacity(96);
    combined.extend_from_slice(cs_hash.as_bytes());
    combined.extend_from_slice(utxo_hash.as_bytes());
    combined.extend_from_slice(ps_hash.as_bytes());
    crypto::hash::hash(&combined)
}

/// Emit the per-component `[STATE_ROOT]` breadcrumb: the chain_state, utxo, and
/// producer_set hashes plus their canonical byte lengths.
///
/// F-D0-2 canary seam (State-Root Lazy Tier-0). Logged at INFO so the three
/// component hashes are visible in production without `RUST_LOG=debug`: state
/// root divergence is the canonical hard incident — one grep per node reveals
/// which component (chain_state, utxo, or producer_set) diverged. This helper is
/// behavior-neutral (logging only) and is the stable seam for epoch-cadence
/// callers once the eager per-block compute is removed in M2.
pub fn log_state_root_components(
    cs_hash: &Hash,
    utxo_hash: &Hash,
    ps_hash: &Hash,
    cs_bytes_len: usize,
    utxo_bytes_len: usize,
    ps_bytes_len: usize,
) {
    tracing::info!(
        "[STATE_ROOT] cs={:.16} utxo={:.16} ps={:.16} cs_bytes={} utxo_bytes={} ps_bytes={}",
        cs_hash,
        utxo_hash,
        ps_hash,
        cs_bytes_len,
        utxo_bytes_len,
        ps_bytes_len
    );
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
/// at any call site. Phase-2 (separate milestone) wires the 6 current
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
            let ps_bytes = producer_set.serialize_canonical();

            let cs_hash = crypto::hash::hash(&cs_bytes);
            let (utxo_hash, utxo_bytes_len) = utxo_set.canonical_digest_and_len()?;
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
                utxo_bytes_len,
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
    /// Create a snapshot from the current state (legacy root).
    pub fn create(
        chain_state: &ChainState,
        utxo_set: &UtxoSet,
        producer_set: &ProducerSet,
    ) -> Result<Self, StorageError> {
        Self::create_gated(chain_state, utxo_set, producer_set, u64::MAX)
    }

    /// Create a snapshot whose root is [`compute_state_root_gated`].
    pub fn create_gated(
        chain_state: &ChainState,
        utxo_set: &UtxoSet,
        producer_set: &ProducerSet,
        pending_root_ah: u64,
    ) -> Result<Self, StorageError> {
        let chain_state_bytes = bincode::serialize(chain_state)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let utxo_set_bytes = utxo_set.serialize_canonical();
        // Wire format uses bincode for ProducerSet (deserializable).
        // State root uses serialize_canonical() (deterministic).
        let producer_set_bytes = bincode::serialize(producer_set)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;

        let state_root =
            compute_state_root_gated(chain_state, utxo_set, producer_set, pending_root_ah)?;

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
/// - `utxo_set_bytes`: canonical format (sorted outpoints, 61-byte-base values)
/// - `producer_set_bytes`: bincode-serialized `ProducerSet`
pub fn compute_state_root_from_bytes(
    chain_state_bytes: &[u8],
    utxo_set_bytes: &[u8],
    producer_set_bytes: &[u8],
) -> Result<Hash, StorageError> {
    compute_state_root_from_bytes_gated(
        chain_state_bytes,
        utxo_set_bytes,
        producer_set_bytes,
        u64::MAX,
    )
}

/// [`compute_state_root_from_bytes`] with the INC-I-181 gate.
pub fn compute_state_root_from_bytes_gated(
    chain_state_bytes: &[u8],
    utxo_set_bytes: &[u8],
    producer_set_bytes: &[u8],
    pending_root_ah: u64,
) -> Result<Hash, StorageError> {
    verify_state_root_from_bytes_gated(
        chain_state_bytes,
        utxo_set_bytes,
        producer_set_bytes,
        pending_root_ah,
    )
    .map(|(root, _, _, _)| root)
}

/// Same verification as [`compute_state_root_from_bytes`], but hands the decoded
/// components back so an installing caller does not decode the blobs a second time.
pub fn verify_state_root_from_bytes(
    chain_state_bytes: &[u8],
    utxo_set_bytes: &[u8],
    producer_set_bytes: &[u8],
) -> Result<(Hash, ChainState, UtxoSet, ProducerSet), StorageError> {
    verify_state_root_from_bytes_gated(
        chain_state_bytes,
        utxo_set_bytes,
        producer_set_bytes,
        u64::MAX,
    )
}

/// [`verify_state_root_from_bytes`] with the INC-I-181 gate, keyed on the decoded
/// `ChainState::best_height`. The whole decoded `ProducerSet`, queue included, is
/// what an installer takes, so from the gate the root must cover the queue too.
pub fn verify_state_root_from_bytes_gated(
    chain_state_bytes: &[u8],
    utxo_set_bytes: &[u8],
    producer_set_bytes: &[u8],
    pending_root_ah: u64,
) -> Result<(Hash, ChainState, UtxoSet, ProducerSet), StorageError> {
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
    let root = compute_state_root_gated(&cs, &utxo, &ps, pending_root_ah)?;
    Ok((root, cs, utxo, ps))
}

#[cfg(test)]
#[path = "snapshot_tests.rs"]
mod snapshot_tests;
