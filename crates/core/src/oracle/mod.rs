//! Phase 2.1 Oracle — pure aggregation primitives.
//!
//! Spec: `specs/oracle-structural-anchored-economics.md` §1.3.
//!
//! This module is the pure-function core of the oracle aggregator:
//!   - `bond_weighted_median(attestations, bond_snapshot)` computes
//!     the per-pair median price that the apply_block epoch-boundary
//!     orchestrator (M6 integration) writes into the OraclePrice
//!     UTXO.
//!
//! The function is deliberately decoupled from the node:
//!   - No I/O, no UtxoSet, no BlockStore. Inputs are slices and
//!     `HashMap`s; outputs are values.
//!   - Every input is owned by the orchestrator. Tests substitute
//!     synthetic distributions to verify economic invariants
//!     (37.3% adversary, 50.1% adversary, tie boundary).
//!
//! Spec §1.3 algorithm, reproduced verbatim:
//!   1. Collect all valid PriceAttestation TXs for the closing epoch
//!      from the block chain (all blocks in the epoch).
//!   2. For each attester, take the LATEST attestation if multiple
//!      were included across blocks (should be at most 1 per
//!      validation rule M4#5, but defense-in-depth).
//!   3. Sort attestations by `price_cents` ascending.
//!   4. Walk sorted list, accumulating bond weight from
//!      `bond_snapshot[attester_pubkey_hash]`.
//!   5. The price at which cumulative weight CROSSES 50% of total
//!      attesting weight is the median.
//!   6. Create new OraclePrice UTXO with the computed median.
//!   7. Consume previous OraclePrice UTXO if one exists; first
//!      epoch creates from nothing.
//!
//! Steps 1-2 (collection + per-attester dedup) live in the
//! orchestrator. Steps 3-5 (the median computation) live here.
//! Steps 6-7 (UTXO mutation) live in the orchestrator.
//!
//! Attesters whose pubkey_hash is missing from `bond_snapshot` or
//! whose weight is zero are SKIPPED — they cannot affect the
//! median. This is consistent with M4 rule 2 (signer must be in
//! `active_producers`) which gives validation-time evidence that
//! the attester was bonded at submission, but defends against the
//! racy edge case where the attester's bond was zeroed by an
//! intra-epoch slash before the boundary.
//!
//! Pubkey-hash convention (matches M2 and the bond_snapshot key):
//!   `hash_with_domain(b"DOLI_ADDR_V1", attester_pubkey.as_bytes())`
//!   (`crate::consensus::ADDRESS_DOMAIN`). The aggregator caller
//!   computes this; the function below accepts the pre-hashed
//!   `(price_cents, attester_hash)` pairs to avoid pulling the
//!   `crypto` crate's hashing into pure-function tests.

use crypto::Hash;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// A single attester's contribution to one pair's epoch median.
///
/// The orchestrator (apply_block epoch-boundary code) builds a
/// `Vec<AttestationContribution>` per `pair_id` by walking blocks
/// in the closing epoch and:
///   1. Decoding each PriceAttestation tx into its
///      `(signer_pubkey, price_cents, pair_id, epoch_number)`.
///   2. Computing `signer_hash = hash_with_domain(ADDRESS_DOMAIN,
///      signer_pubkey.as_bytes())`.
///   3. Taking the LATEST contribution per `signer_hash` (defense
///      in depth — M4 rule 5 at validation already rejects
///      duplicates within the same epoch).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttestationContribution {
    /// `hash_with_domain(b"DOLI_ADDR_V1", signer_pubkey.as_bytes())`.
    pub signer_hash: Hash,
    /// Attested price in USD cents.
    pub price_cents: u64,
}

/// Bond-weighted median of a set of attestations, weighted by
/// `bond_snapshot[signer_hash]`.
///
/// Returns `None` when there are no attestations whose signer is
/// present in `bond_snapshot` with weight > 0. Otherwise returns
/// `Some((median_cents, contributor_count))`.
///
/// `contributor_count` is the number of distinct attesters that
/// contributed positive weight (capped at `u16::MAX` to fit the
/// OraclePrice extra_data layout from M5 — anything more is bizarre
/// for a per-epoch attestation set; we saturate rather than overflow).
///
/// Spec §1.3 steps 3-5. Median rule: walk attestations sorted by
/// `price_cents` ascending, accumulate weight, return the
/// `price_cents` of the FIRST attester whose cumulative weight is
/// at least 50% of total weight. This is the standard "50% crossing"
/// median:
///   - At 37.3% adversary attempting to push the median: 0%
///     deviation possible (`Mechanism-Skeptic Q3`,
///     `Adversarial-Capital Table §3`).
///   - At 50.1% adversary: median is fully controllable. This is
///     defended by the sunset trigger at M8, not by this function.
///
/// Tie-breaking: when total_weight is EVEN and the cumulative
/// weight reaches exactly `total_weight / 2` at some attester, the
/// median is that attester's `price_cents`. The NEXT attester in
/// the sort order would also be a valid "median" candidate under
/// the classical definition (lower-median vs upper-median). We
/// pick the FIRST attester whose cumulative weight reaches the
/// 50% threshold — equivalent to lower-median for ties. This is
/// deterministic and tested explicitly.
pub fn bond_weighted_median(
    attestations: &[AttestationContribution],
    bond_snapshot: &HashMap<Hash, u64>,
) -> Option<(u64, u16)> {
    // Filter to attesters with positive bonded weight and pair the
    // weight alongside the price for sorting.
    let mut weighted: Vec<(u64, u64)> = attestations
        .iter()
        .filter_map(|a| {
            let w = bond_snapshot.get(&a.signer_hash).copied().unwrap_or(0);
            if w == 0 {
                None
            } else {
                Some((a.price_cents, w))
            }
        })
        .collect();

    if weighted.is_empty() {
        return None;
    }

    weighted.sort_by_key(|(price, _)| *price);
    let total_weight: u128 = weighted.iter().map(|(_, w)| *w as u128).sum();
    // Median crossing: cumulative weight >= ceil(total_weight / 2).
    // We use the standard "first crossing" semantics: median is the
    // price at the attester whose cumulative weight first reaches
    // 50% of total. For odd totals this is unambiguous; for even
    // totals (cumulative exactly equals total/2 at some attester)
    // we deterministically pick the lower-median (the attester at
    // the crossing, NOT the next one up).
    //
    // ceil_half = (total_weight + 1) / 2 — equivalent to
    // ceil(total_weight / 2.0) but without floating point.
    let ceil_half = total_weight.div_ceil(2);
    let mut cumulative: u128 = 0;
    let mut median_price = weighted[0].0;
    for (price, weight) in &weighted {
        cumulative = cumulative.saturating_add(*weight as u128);
        if cumulative >= ceil_half {
            median_price = *price;
            break;
        }
    }

    let count = u16::try_from(weighted.len()).unwrap_or(u16::MAX);
    Some((median_price, count))
}

/// Take the LATEST contribution per attester (spec §1.3 step 2).
///
/// The orchestrator walks blocks in chronological order and feeds
/// every PriceAttestation tx for a single pair through this
/// function (one call per pair). The vec returned has at most one
/// entry per distinct `signer_hash`, holding the price from the
/// LAST occurrence in the input slice. Order is not preserved.
///
/// M4 rule 5 already rejects duplicates at validation time, so in
/// practice the input has at most one contribution per attester
/// already. This dedup step exists as defense-in-depth: if a
/// reorg or an off-spec relay implementation lets two
/// PriceAttestations from the same attester both land in the
/// closing epoch's blocks, we deterministically pick the second
/// one rather than the first.
pub fn dedupe_latest_per_attester(
    contributions: &[AttestationContribution],
) -> Vec<AttestationContribution> {
    // AUDIT-P3-001: BTreeMap (not HashMap) — deterministic iteration order
    // by signer_hash. The downstream median is order-independent today
    // (sort by price + commutative weight sum), but using BTreeMap
    // eliminates the class of "future maintenance introduces an order-
    // sensitive secondary effect → silent consensus fork".
    let mut by_signer: BTreeMap<Hash, AttestationContribution> = BTreeMap::new();
    for c in contributions {
        by_signer.insert(c.signer_hash, *c);
    }
    by_signer.into_values().collect()
}

/// Phase 2.1 Oracle sunset threshold, in basis points.
///
/// When the structural-bond-share metric falls strictly below this
/// value at an epoch boundary, the oracle HALTs (spec §1.8):
///   - New `PriceAttestation` txs are rejected with `[ERRTX-ORACLE003]`.
///   - The aggregator skips the median computation; the last
///     committed `OraclePrice` UTXO is left in place (readable but
///     stale).
///
/// 5500 bps = 55.00%. The spec calls out this exact threshold (§1.8,
/// "Threshold: structural_share < 55%").
pub const SUNSET_THRESHOLD_BPS: u16 = 5500;

/// Warning zone threshold, in basis points (D.3 sunset gradient).
///
/// When the structural-bond-share metric falls below this value but
/// remains at-or-above `SUNSET_THRESHOLD_BPS`, the oracle is in the
/// WARNING state: aggregation continues normally, but metrics/logs
/// and `getOracleStatus` report `health: "warning"`.
///
/// 6000 bps = 60.00%.
///
/// Spec: `specs/defi-l1-foundations-architecture.md` §D.3.
pub const SUNSET_WARNING_BPS: u16 = 6000;

/// Number of consecutive epochs in HALT before the halt becomes
/// permanent (D.3 recovery window).
///
/// If the structural share rises back above `SUNSET_THRESHOLD_BPS`
/// within this many epochs after entering HALT, the oracle resumes
/// automatically. After this window elapses with sustained low
/// share, recovery requires a binary upgrade (existing behavior).
///
/// 4 epochs at 360 blocks/epoch at 10s/block = ~4 hours.
///
/// Spec: `specs/defi-l1-foundations-architecture.md` §D.3.
pub const ORACLE_RECOVERY_EPOCHS: u64 = 4;

/// Oracle health state, derived from structural share and epoch
/// tracking (D.3 sunset gradient state machine).
///
/// The state machine replaces the previous single-cliff sunset at
/// `SUNSET_THRESHOLD_BPS` with a 3-zone gradient:
///
/// | structural_share_bps | State            | Action                        |
/// |----------------------|------------------|-------------------------------|
/// | >= 6000              | Healthy          | Aggregate normally            |
/// | 5500-5999            | Warning          | Aggregate, emit warning       |
/// | < 5500 (recoverable) | HaltRecoverable | Stop aggregating, may recover |
/// | < 5500 for >= 4 ep   | HaltPermanent   | Binary upgrade required       |
///
/// Spec: `specs/defi-l1-foundations-architecture.md` §D.3.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OracleHealthState {
    /// Structural share >= `SUNSET_WARNING_BPS` (6000 bps = 60%).
    /// Oracle fully active.
    Healthy,
    /// Structural share in `[SUNSET_THRESHOLD_BPS, SUNSET_WARNING_BPS)`
    /// (5500-5999 bps). Oracle still aggregates but emits warnings.
    Warning,
    /// Structural share < `SUNSET_THRESHOLD_BPS` (5500 bps) for fewer
    /// than `ORACLE_RECOVERY_EPOCHS` epochs. Oracle stops aggregating
    /// but can auto-recover if share rises.
    HaltRecoverable,
    /// Structural share < `SUNSET_THRESHOLD_BPS` for >=
    /// `ORACLE_RECOVERY_EPOCHS` epochs. Binary upgrade required.
    HaltPermanent,
}

impl OracleHealthState {
    /// Returns the RPC string representation of this health state.
    pub fn as_rpc_str(&self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Warning => "warning",
            Self::HaltRecoverable => "halted_recoverable",
            Self::HaltPermanent => "halted_permanent",
        }
    }

    /// Whether aggregation should proceed in this state.
    pub fn should_aggregate(&self) -> bool {
        matches!(self, Self::Healthy | Self::Warning)
    }

    /// Whether the sunset flag should be set (reject PriceAttestation txs).
    pub fn is_sunset_triggered(&self) -> bool {
        matches!(self, Self::HaltRecoverable | Self::HaltPermanent)
    }
}

/// Persisted oracle sunset state, stored in the node's StateDB as
/// metadata keys. These fields are deterministically derivable from
/// chain history (set at epoch boundaries based on structural share)
/// but persisted for restart safety so the node does not need to
/// re-scan the entire chain to determine the current health state.
///
/// NOT part of the consensus state root (not in ChainState canonical
/// encoding or EpochState). Local bookkeeping only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OracleSunsetState {
    /// Epoch at which the oracle first entered WARNING (share < 6000).
    /// `None` when share >= 6000 (HEALTHY).
    pub warning_since_epoch: Option<u64>,
    /// Epoch at which the oracle entered HALT (share < 5500).
    /// `None` when share >= 5500 (HEALTHY or WARNING).
    pub halt_since_epoch: Option<u64>,
    /// Set to `true` once HALT has persisted for >=
    /// `ORACLE_RECOVERY_EPOCHS`. Sticky: once permanent, only a
    /// binary upgrade can clear it.
    pub halt_permanent: bool,
}

impl OracleSunsetState {
    /// Determine the current health state from persisted tracking
    /// fields and the current structural share.
    pub fn health(&self, current_epoch: u64) -> OracleHealthState {
        if self.halt_permanent {
            return OracleHealthState::HaltPermanent;
        }
        if let Some(halt_epoch) = self.halt_since_epoch {
            let elapsed = current_epoch.saturating_sub(halt_epoch);
            if elapsed >= ORACLE_RECOVERY_EPOCHS {
                return OracleHealthState::HaltPermanent;
            }
            return OracleHealthState::HaltRecoverable;
        }
        if self.warning_since_epoch.is_some() {
            return OracleHealthState::Warning;
        }
        OracleHealthState::Healthy
    }

    /// Advance the state machine based on the structural share at the
    /// current epoch boundary. Returns the new health state.
    ///
    /// This is the core D.3 state transition function. All inputs are
    /// integers — no floats, no walltime, fully deterministic.
    ///
    /// `share_bps`: `Some(bps)` from `compute_structural_share_bps`,
    /// or `None` if no eligible bonds exist (treated as share=0).
    pub fn transition(&mut self, share_bps: Option<u16>, current_epoch: u64) -> OracleHealthState {
        // If already permanently halted, stay there.
        if self.halt_permanent {
            return OracleHealthState::HaltPermanent;
        }

        let bps = share_bps.unwrap_or(0);

        if bps >= SUNSET_WARNING_BPS {
            // HEALTHY zone: clear all tracking.
            self.warning_since_epoch = None;
            self.halt_since_epoch = None;
            OracleHealthState::Healthy
        } else if bps >= SUNSET_THRESHOLD_BPS {
            // WARNING zone (5500-5999 bps).
            // If we were halted and share recovered to >= 5500,
            // check if we're within the recovery window.
            if let Some(halt_epoch) = self.halt_since_epoch {
                let elapsed = current_epoch.saturating_sub(halt_epoch);
                if elapsed >= ORACLE_RECOVERY_EPOCHS {
                    // Too late — permanent halt.
                    self.halt_permanent = true;
                    return OracleHealthState::HaltPermanent;
                }
                // Recovery: share rose back above SUNSET_THRESHOLD.
                // Clear halt, keep warning.
                self.halt_since_epoch = None;
            }
            // Set warning epoch if not already set.
            if self.warning_since_epoch.is_none() {
                self.warning_since_epoch = Some(current_epoch);
            }
            OracleHealthState::Warning
        } else {
            // HALT zone (< 5500 bps).
            // Set warning if not already set.
            if self.warning_since_epoch.is_none() {
                self.warning_since_epoch = Some(current_epoch);
            }
            // Set halt epoch if not already set.
            if self.halt_since_epoch.is_none() {
                self.halt_since_epoch = Some(current_epoch);
            }
            // Check if permanent.
            let halt_epoch = self.halt_since_epoch.unwrap();
            let elapsed = current_epoch.saturating_sub(halt_epoch);
            if elapsed >= ORACLE_RECOVERY_EPOCHS {
                self.halt_permanent = true;
                OracleHealthState::HaltPermanent
            } else {
                OracleHealthState::HaltRecoverable
            }
        }
    }
}

/// Compute the structural-bond-share metric, in basis points
/// (0..=10000), against the given `bond_snapshot` and per-producer
/// `registered_at` map.
///
/// Spec: `specs/oracle-structural-anchored-economics.md` §1.8.
///
/// Algorithm:
/// ```text
/// structural_bonds      = sum(bond_snapshot[k] for k in STRUCTURAL_PUBKEY_HASHES)
/// total_bonds_eligible  = sum(bond_snapshot[k] for k where bond_age >= 1 epoch)
/// structural_share      = structural_bonds / total_bonds_eligible
/// ```
///
/// Inputs:
///   - `bond_snapshot`: the PREVIOUS epoch's snapshot (1-epoch lag
///     per spec §1.8 — "prevents same-epoch manipulation"). The
///     orchestrator calls this with `self.epoch_state.bond_snapshot`
///     BEFORE rotating into the new epoch.
///   - `registered_at`: per-producer registration height. The
///     orchestrator builds this from
///     `ProducerSet::active_producers_at_height(height)`.
///   - `current_epoch_start_height`: the height of the first block
///     of the NEW epoch (= the boundary height the aggregator is
///     processing).
///   - `blocks_per_epoch`: from `NetworkParams`.
///
/// Returns:
///   - `None` if `total_bonds_eligible == 0` (no eligible bonds —
///     by spec, the oracle is non-operational, equivalent to
///     sunset-triggered).
///   - `Some(bps)` otherwise, where `bps` is the structural share
///     in 1/10000 units (5500 = 55.00%). Clamped to `[0, 10000]`.
///
/// Anti-dilution (spec §1.8): bonds whose owning producer was
/// registered LESS than one epoch ago are excluded from
/// `total_bonds_eligible`. This is a producer-level filter
/// (`bond_age` here means "owner's registration age") because
/// `bond_snapshot` is keyed by producer pubkey-hash, not per-bond.
/// The structural set's registrations are baked into genesis and
/// thus always satisfy the age check.
pub fn compute_structural_share_bps(
    bond_snapshot: &HashMap<Hash, u64>,
    registered_at: &HashMap<Hash, u64>,
    current_epoch_start_height: u64,
    blocks_per_epoch: u64,
    structural_hashes: &[Hash],
) -> Option<u16> {
    let one_epoch_ago = current_epoch_start_height.saturating_sub(blocks_per_epoch);

    let structural_bonds: u128 = structural_hashes
        .iter()
        .map(|k| bond_snapshot.get(k).copied().unwrap_or(0) as u128)
        .sum();

    let total_bonds_eligible: u128 = bond_snapshot
        .iter()
        .filter_map(|(k, w)| {
            // Producer is eligible only if registered at-or-before
            // (current_epoch_start_height - blocks_per_epoch). A
            // missing `registered_at` entry means we cannot prove
            // the producer is at least 1 epoch old — exclude them
            // (conservative: anti-dilution defends harder when in
            // doubt).
            let regd = registered_at.get(k).copied()?;
            if regd <= one_epoch_ago {
                Some(*w as u128)
            } else {
                None
            }
        })
        .sum();

    if total_bonds_eligible == 0 {
        return None;
    }

    // bps = structural * 10_000 / total. u128 arithmetic prevents
    // overflow even at TOTAL_SUPPLY-sized weights.
    let bps = (structural_bonds.saturating_mul(10_000)) / total_bonds_eligible;
    Some(bps.min(10_000) as u16)
}

/// Deterministic outpoint key for the per-pair `OraclePrice` UTXO.
///
/// The OraclePrice UTXO is a singleton — there is exactly one
/// per `pair_id`. Because it is system-spent (apply_block at the
/// epoch boundary writes it directly into the UtxoSet, never via
/// a regular tx), there is no natural `tx_hash` to key the
/// outpoint against. We synthesize one deterministically:
///
///   `outpoint = (oracle_price_address(pair_id), 0)`
///
/// where `oracle_price_address(pair_id) = hash_with_domain(
/// b"ORACLE_PRICE", pair_id)`. This:
///   - Lets every node compute the same outpoint key for the
///     same `pair_id` without coordination, so the aggregator's
///     `remove(outpoint)` + `insert(outpoint, ...)` mutation is
///     bit-identical across nodes (snap-sync invariant).
///   - Is collision-free against real tx hashes: a real tx hash
///     is BLAKE3 of the serialized tx body; this synthetic hash
///     is BLAKE3 of `b"ORACLE_PRICE" || pair_id`. The 256-bit
///     domain separation makes accidental collision negligible.
///
/// Spec: implementation note for §1.2 (per-pair singleton UTXO).
#[must_use]
pub fn oracle_price_outpoint(pair_id: &Hash) -> (Hash, u32) {
    (crate::transaction::Output::oracle_price_address(pair_id), 0)
}

#[cfg(test)]
mod tests;
