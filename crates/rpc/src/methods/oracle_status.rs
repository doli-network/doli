//! Pure-function helpers for M11 `getOracleStatus`.
//!
//! Split from `oracle.rs` to keep that file under the 500-LOC source
//! budget (Rule 19). All items are `pub(super)` so the `oracle` handler
//! module (sibling, since both are children of `methods`) can call
//! them, and the M11 tests in `tests_oracle_m11.rs` can exercise them
//! directly with mock structural-hash sets.
//!
//! See `oracle::get_oracle_status` for the full RPC-handler wiring and
//! field semantics.

use serde_json::Value;

use super::oracle::ORACLE_TRUST_MODEL;

/// `inputs` bundles the 8 status-builder inputs to keep clippy's
/// `too_many_arguments` lint happy without losing per-field clarity at
/// the call sites.
pub(super) struct OracleStatusInputs<'a> {
    pub current_height: u64,
    pub activation_height: u64,
    pub structural_hashes: &'a [crypto::Hash],
    pub registered_at: &'a std::collections::HashMap<crypto::Hash, u64>,
    pub bond_snapshot: &'a std::collections::HashMap<crypto::Hash, u64>,
    pub blocks_per_epoch: u64,
    pub last_update_height: Option<u64>,
    pub attester_count: u64,
}

/// Build the byte-deterministic JSON response for `getOracleStatus`.
/// Extracted as a pure function so tests can inject arbitrary
/// `structural_hashes` (production uses the mainnet-derived constant,
/// whose preimages cannot be reproduced without real mainnet pubkeys).
pub(super) fn build_oracle_status_response(inputs: OracleStatusInputs<'_>) -> Value {
    let OracleStatusInputs {
        current_height,
        activation_height,
        structural_hashes,
        registered_at,
        bond_snapshot,
        blocks_per_epoch,
        last_update_height,
        attester_count,
    } = inputs;

    let current_epoch = current_height.checked_div(blocks_per_epoch).unwrap_or(0);
    let current_epoch_start_height = current_epoch.saturating_mul(blocks_per_epoch);

    let share_bps_opt = doli_core::oracle::compute_structural_share_bps(
        bond_snapshot,
        registered_at,
        current_epoch_start_height,
        blocks_per_epoch,
        structural_hashes,
    );
    let (structural_share_bps, sunset_triggered) = match share_bps_opt {
        Some(bps) => (bps, bps < doli_core::oracle::SUNSET_THRESHOLD_BPS),
        None => (0u16, true),
    };
    let structural_share = f64::from(structural_share_bps) / 10_000.0;
    let sunset_threshold = f64::from(doli_core::oracle::SUNSET_THRESHOLD_BPS) / 10_000.0;

    let active = current_height >= activation_height && !sunset_triggered;

    serde_json::json!({
        "active":                    active,
        "trust_model":               ORACLE_TRUST_MODEL,
        "structural_share":          structural_share,
        "sunset_threshold":          sunset_threshold,
        "sunset_triggered":          sunset_triggered,
        "last_update_height":        last_update_height,
        "attester_count":            attester_count,
        "activation_height":         activation_height,
        "centralization_disclosure": CENTRALIZATION_DISCLOSURE,
    })
}

/// Count distinct attesters (by `hash_with_domain(ADDRESS_DOMAIN,
/// pubkey)`) for `epoch` by scanning all blocks in
/// `[epoch * blocks_per_epoch, (epoch + 1) * blocks_per_epoch)`. Used
/// by `getOracleStatus` to report attestation participation in the
/// most-recently CLOSED epoch.
///
/// Missing blocks (pruned, never-produced) are silently skipped — same
/// empty-list contract as M10.
pub(super) fn count_distinct_attesters_in_epoch(
    block_store: &storage::BlockStore,
    epoch: u64,
    blocks_per_epoch: u64,
) -> u64 {
    let start = epoch.saturating_mul(blocks_per_epoch);
    let end_exclusive = start.saturating_add(blocks_per_epoch);
    let mut signers: std::collections::HashSet<crypto::Hash> = std::collections::HashSet::new();
    for height in start..end_exclusive {
        let Some(block) = block_store.get_block_by_height(height).ok().flatten() else {
            continue;
        };
        for tx in &block.transactions {
            if !tx.is_price_attestation() {
                continue;
            }
            let Some(data) = tx.price_attestation_data() else {
                continue;
            };
            if data.epoch_number != epoch {
                continue;
            }
            let h = crypto::hash::hash_with_domain(
                crypto::ADDRESS_DOMAIN,
                data.signer_pubkey.as_bytes(),
            );
            signers.insert(h);
        }
    }
    signers.len() as u64
}

/// Verbatim centralization disclosure from
/// `specs/oracle-structural-anchored-economics.md` §6.
///
/// **Drift gate**: `tests_oracle_m11::m11_centralization_disclosure_byte_equal_to_spec`
/// asserts byte-equality between this constant and the §6 paragraph
/// extracted from the spec file. ANY edit to this constant must be
/// matched by the spec, and vice-versa. The test is the source of
/// truth for the gate.
pub(super) const CENTRALIZATION_DISCLOSURE: &str = "\
**DOLI Trust Disclosure -- Phase 2.1 Oracle**\n\
\n\
DOLI's Phase 2.1 oracle price is reported by bonded producers using \
bond-weighted median aggregation. As of activation, the operator-controlled \
structural set (N1-N12) holds 62.7% of total bonded stake (176,650 of \
281,717 DOLI), giving them unilateral control over the oracle median. The \
oracle's correctness depends on this structural majority maintaining honest \
behavior. The security model is operator economic alignment (176,650 DOLI \
at risk plus a future epoch reward stream valued at approximately 1.98M \
DOLI per year), NOT distributed consensus. An external attacker with the \
remaining 37.3% of bonds cannot manipulate the oracle under any \
circumstances. An automatic sunset fires when structural bond share falls \
below 55% -- at that point, the oracle halts and the protocol must be \
upgraded to either restore structural majority or transition to a \
decentralized attestation model. This is explicitly NOT a decentralized \
oracle and makes no claim to be one. Users of oracle-dependent DeFi \
primitives (lending, liquidation) in Phase 2.3 and beyond explicitly accept \
this trust model.\n\
\n\
During Phase 2.1, oracle attestation is funded entirely by the structural \
set's implicit economic alignment with DOLI value capture. No explicit \
emission or fee carve-out funds attestation. Oracle compensation becomes \
fee-funded when lending (Phase 2.3) activates and generates sufficient \
consumer fees.";
