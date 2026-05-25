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
use std::collections::HashMap;

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
    let mut by_signer: HashMap<Hash, AttestationContribution> = HashMap::new();
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
///   - Recovery requires a binary upgrade — no on-chain recovery
///     path is provided (HC-8 / spec §0 NEVER constraints).
///
/// 5500 bps = 55.00%. The spec calls out this exact threshold (§1.8,
/// "Threshold: structural_share < 55%").
pub const SUNSET_THRESHOLD_BPS: u16 = 5500;

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
