use super::constants::{NUM_REGIONS, TIER1_MAX_VALIDATORS, TIER2_MAX_ATTESTORS};

// =============================================================================
// TIERED ARCHITECTURE FUNCTIONS
// =============================================================================

/// Compute the Tier 1 validator set: top N producers by effective_weight.
///
/// Selection is deterministic — all nodes compute the same set for the same input.
/// Tiebreaker: lexicographic ordering of pubkey bytes (ensures determinism).
///
/// # Arguments
/// * `producers_with_weights` - All active producers with their effective weights
///
/// # Returns
/// Vec of public keys for Tier 1 validators, capped at TIER1_MAX_VALIDATORS.
pub fn compute_tier1_set(
    producers_with_weights: &[(crypto::PublicKey, u64)],
) -> Vec<crypto::PublicKey> {
    let mut sorted = producers_with_weights.to_vec();
    // Sort by weight descending, then by pubkey bytes ascending for deterministic tiebreak
    // (unstable is safe: pubkey tiebreak ensures unique ordering)
    sorted.sort_unstable_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| a.0.as_bytes().cmp(b.0.as_bytes()))
    });
    sorted.truncate(TIER1_MAX_VALIDATORS);
    sorted.into_iter().map(|(pk, _)| pk).collect()
}

/// Compute a producer's region (deterministic from pubkey hash).
///
/// Uses BLAKE3 hash of the public key bytes, taking the first 4 bytes as a u32
/// and modding by NUM_REGIONS. This gives uniform distribution across regions.
pub fn producer_region(pubkey: &crypto::PublicKey) -> u32 {
    let h = crypto::hash::hash(pubkey.as_bytes());
    let bytes: [u8; 4] = h.as_bytes()[0..4].try_into().unwrap();
    u32::from_le_bytes(bytes) % NUM_REGIONS
}

/// Determine a producer's tier.
///
/// - Tier 1: In the tier1_set (top validators by weight)
/// - Tier 2: Not in tier1, but within tier2_count (attestors)
/// - Tier 3: Beyond tier2 threshold (header-only validation)
/// - Tier 0: Producer not found in on-chain set (safe default)
pub fn producer_tier(
    pubkey: &crypto::PublicKey,
    tier1_set: &[crypto::PublicKey],
    all_producers_sorted: &[crypto::PublicKey],
) -> u8 {
    // Build HashSet for O(1) lookup instead of O(n) linear scan.
    // tier1_set is capped at 500 entries, so this is negligible.
    let tier1: std::collections::HashSet<&crypto::PublicKey> = tier1_set.iter().collect();
    if tier1.contains(pubkey) {
        return 1;
    }
    if let Some(pos) = all_producers_sorted.iter().position(|p| p == pubkey) {
        if pos < TIER1_MAX_VALIDATORS + TIER2_MAX_ATTESTORS {
            return 2;
        }
        return 3;
    }
    // Not found in on-chain set: stay Tier 0 (all topics, safe during sync)
    0
}
