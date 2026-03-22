use crate::block::BlockHeader;
use crate::consensus::is_producer_eligible_ms;
use crate::network::Network;
use crate::tpop::heartbeat::verify_hash_chain_vdf;

use super::{ValidationContext, ValidationError};

/// Validate the VDF proof in a block header.
///
/// Hash-chain VDF is fast to compute and self-verifying (recompute and compare output).
/// For networks with VDF disabled (e.g., devnet), this is a no-op.
pub(super) fn validate_vdf(header: &BlockHeader, network: Network) -> Result<(), ValidationError> {
    // Skip VDF validation if disabled for this network (e.g., devnet)
    if !network.vdf_enabled() {
        return Ok(());
    }

    let input = header.vdf_input();

    // Hash-chain VDF output should be exactly 32 bytes
    if header.vdf_output.value.len() != 32 {
        return Err(ValidationError::InvalidVdfProof);
    }

    let expected_output: [u8; 32] = header
        .vdf_output
        .value
        .as_slice()
        .try_into()
        .map_err(|_| ValidationError::InvalidVdfProof)?;

    // Verify by re-computing the hash-chain VDF
    // Use network-specific iterations (devnet uses fewer for faster blocks)
    if !verify_hash_chain_vdf(&input, &expected_output, network.heartbeat_vdf_iterations()) {
        return Err(ValidationError::InvalidVdfProof);
    }

    Ok(())
}

/// Compute the bootstrap fallback rank order for a slot.
///
/// Returns a deduped list of producers in fallback rank order:
/// rank 0 = `sorted_producers[slot % n]`, rank 1 = `sorted_producers[(slot+1) % n]`, etc.
/// At most `MAX_FALLBACK_RANKS` entries, deduplicated (important when n < MAX_FALLBACK_RANKS).
///
/// Both the production and validation sides must use this same function to ensure consensus.
pub fn bootstrap_fallback_order(
    slot: u32,
    sorted_producers: &[crypto::PublicKey],
) -> Vec<crypto::PublicKey> {
    let n = sorted_producers.len();
    if n == 0 {
        return Vec::new();
    }
    let max_ranks = crate::consensus::MAX_FALLBACK_RANKS;
    let mut result = Vec::with_capacity(max_ranks.min(n));
    for rank in 0..max_ranks {
        let idx = ((slot as usize) + rank) % n;
        let pk = sorted_producers[idx];
        if !result.contains(&pk) {
            result.push(pk);
        }
    }
    result
}

/// Build the eligible producer list for a bootstrap slot, applying liveness filter.
///
/// Normal slots: all ranks from `live_producers` only.
/// Re-entry slots: one stale producer at rank 0, `live_producers` at ranks 1+.
///
/// Both `live_producers` and `stale_producers` must be sorted by pubkey.
/// Returns the eligible list ordered by rank (index 0 = rank 0).
///
/// Re-entry slots are spread evenly: for stale producer `i`, re-entry fires when
/// `slot % reentry_interval == i * (reentry_interval / effective_stale)`.
/// Capped at `reentry_interval / 5` stale producers (20% max overhead).
pub fn bootstrap_schedule_with_liveness(
    slot: u32,
    live_producers: &[crypto::PublicKey],
    stale_producers: &[crypto::PublicKey],
    reentry_interval: u32,
) -> Vec<crypto::PublicKey> {
    let max_ranks = crate::consensus::MAX_FALLBACK_RANKS;

    // Cap stale count to avoid excessive re-entry overhead (max 20%)
    let effective_stale = stale_producers.len().min((reentry_interval / 5) as usize);

    // Determine if this is a re-entry slot for a stale producer
    let reentry_producer = if effective_stale > 0 && reentry_interval > 0 {
        let phase = slot % reentry_interval;
        let spacing = reentry_interval / effective_stale as u32;
        if spacing > 0 && phase.is_multiple_of(spacing) {
            let idx = (phase / spacing) as usize;
            if idx < effective_stale {
                Some(stale_producers[idx])
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    let mut result = Vec::with_capacity(max_ranks);

    if let Some(stale_pk) = reentry_producer {
        // RE-ENTRY SLOT: stale producer at rank 0, live producers at ranks 1+
        result.push(stale_pk);
        if !live_producers.is_empty() {
            for rank in 0..max_ranks.saturating_sub(1) {
                let idx = ((slot as usize) + rank) % live_producers.len();
                let pk = live_producers[idx];
                if !result.contains(&pk) {
                    result.push(pk);
                }
            }
        }
    } else {
        // NORMAL SLOT: all ranks from live producers
        if live_producers.is_empty() {
            return Vec::new();
        }
        for rank in 0..max_ranks {
            let idx = ((slot as usize) + rank) % live_producers.len();
            let pk = live_producers[idx];
            if !result.contains(&pk) {
                result.push(pk);
            }
        }
    }

    result
}

/// Validate a block's producer during bootstrap using fallback rank windows.
///
/// Same 2-second exclusive windows as the epoch scheduler:
/// rank 0 -> 0-2s, rank 1 -> 2-4s, rank 2 -> 4-6s, etc.
///
/// If no bootstrap_producers are set, falls back to accepting any producer
/// (backward compatibility for blocks produced before this fix).
fn validate_bootstrap_producer(
    header: &BlockHeader,
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    // No bootstrap list -> accept any producer (pre-fix blocks / early genesis)
    if ctx.bootstrap_producers.is_empty() {
        return Ok(());
    }

    // Producer must be in the known set
    if !ctx.bootstrap_producers.contains(&header.producer) {
        return Err(ValidationError::InvalidProducer);
    }

    // Compute fallback order for this slot.
    // Use liveness-aware scheduling when live/stale split is available.
    let eligible = if !ctx.live_bootstrap_producers.is_empty() {
        bootstrap_schedule_with_liveness(
            header.slot,
            &ctx.live_bootstrap_producers,
            &ctx.stale_bootstrap_producers,
            crate::consensus::REENTRY_INTERVAL,
        )
    } else {
        bootstrap_fallback_order(header.slot, &ctx.bootstrap_producers)
    };

    if eligible.is_empty() {
        return Err(ValidationError::InvalidProducer);
    }

    // Validate time window: producer's rank must match the block's timestamp offset
    let slot_start = ctx.params.slot_to_timestamp(header.slot);
    let slot_offset_secs = header.timestamp.saturating_sub(slot_start);
    let slot_offset_ms_low = slot_offset_secs * 1000;
    let slot_offset_ms_high = slot_offset_ms_low + 999;

    if !is_producer_eligible_ms(&header.producer, &eligible, slot_offset_ms_low)
        && !is_producer_eligible_ms(&header.producer, &eligible, slot_offset_ms_high)
    {
        return Err(ValidationError::InvalidProducer);
    }

    Ok(())
}

/// Validate that the block producer is eligible for the slot.
///
/// This checks that:
/// 1. The producer is in the eligible fallback list for the slot
/// 2. The block timestamp falls within the producer's allowed window
///
/// During bootstrap, uses the same 2-second fallback rank windows as the epoch
/// scheduler but with the GSet-derived producer list instead of bond-weighted tickets.
pub fn validate_producer_eligibility(
    header: &BlockHeader,
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    // During genesis phase, use bootstrap validation (aligned with production mode).
    // Production switches to bond-weighted scheduler at genesis_blocks + 1,
    // so validation must use the same threshold -- not bootstrap_blocks.
    if ctx.network.is_in_genesis(ctx.current_height) {
        return validate_bootstrap_producer(header, ctx);
    }

    // Post-genesis: ROUND-ROBIN validation (matches production).
    // One producer per slot, cycling through sorted active producers.
    // Bond weighting only affects rewards, not production scheduling.
    if !ctx.active_producers_weighted.is_empty() {
        let mut sorted: Vec<crypto::PublicKey> = ctx
            .active_producers_weighted
            .iter()
            .map(|(pk, _)| *pk)
            .collect();
        sorted.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

        if sorted.is_empty() {
            return Err(ValidationError::InvalidProducer);
        }

        let producer_index = (header.slot as usize) % sorted.len();
        let expected_producer = sorted[producer_index];

        if header.producer != expected_producer {
            return Err(ValidationError::InvalidProducer);
        }

        return Ok(());
    }

    // Fallback: syncing node without producer set -- use bootstrap validation
    // if still within bootstrap window (allows catching up without producer data)
    if ctx.params.is_bootstrap(ctx.current_height) {
        return validate_bootstrap_producer(header, ctx);
    }

    Err(ValidationError::InvalidProducer)
}
