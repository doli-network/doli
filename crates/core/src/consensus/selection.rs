use crate::types::Slot;

use super::constants::{FALLBACK_TIMEOUT_MS, MAX_FALLBACK_PRODUCERS, MAX_FALLBACK_RANKS};

/// Select the producer for a slot using evenly-distributed ticket offsets.
///
/// This is the primary selection function. It uses a deterministic round-robin
/// based on bond count (consecutive tickets). Selection is independent of
/// the previous block hash to prevent grinding attacks.
///
/// # Algorithm
/// 1. Calculate total tickets = sum of all producer bond counts
/// 2. ticket_index = slot % total_tickets
/// 3. Find producer whose consecutive ticket range contains ticket_index
/// 4. Return up to MAX_FALLBACK_PRODUCERS using evenly-distributed offsets:
///    rank_offset = (total_tickets * rank) / MAX_FALLBACK_RANKS
///
/// # Arguments
/// * `slot` - The slot number
/// * `producers_with_bonds` - List of (PublicKey, bond_count) tuples, sorted by pubkey
///
/// # Returns
/// Vector of up to MAX_FALLBACK_PRODUCERS public keys ordered by priority
pub fn select_producer_for_slot(
    slot: Slot,
    producers_with_bonds: &[(crypto::PublicKey, u64)],
) -> Vec<crypto::PublicKey> {
    if producers_with_bonds.is_empty() {
        return Vec::new();
    }

    // Sort producers by pubkey for deterministic ordering
    let mut sorted: Vec<_> = producers_with_bonds.to_vec();
    sorted.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));

    // Calculate total tickets (each bond = 1 ticket, minimum 1 per producer)
    let total_tickets: u64 = sorted.iter().map(|(_, bonds)| (*bonds).max(1)).sum();

    if total_tickets == 0 {
        return Vec::new();
    }

    // Helper to find producer for a given ticket index
    let find_producer = |ticket_idx: u64| -> Option<crypto::PublicKey> {
        let mut cumulative: u64 = 0;
        for (pk, bonds) in &sorted {
            let tickets = (*bonds).max(1);
            if ticket_idx < cumulative + tickets {
                return Some(*pk);
            }
            cumulative += tickets;
        }
        None
    };

    let mut result = Vec::with_capacity(MAX_FALLBACK_PRODUCERS);

    // Primary producer: slot % total_tickets
    let primary_ticket = (slot as u64) % total_tickets;
    if let Some(pk) = find_producer(primary_ticket) {
        result.push(pk);
    }

    // Fallback producers: evenly-distributed offsets across ticket space
    for rank in 1..MAX_FALLBACK_RANKS {
        if result.len() >= MAX_FALLBACK_PRODUCERS {
            break;
        }
        let offset = (total_tickets * rank as u64) / MAX_FALLBACK_RANKS as u64;
        let ticket = (primary_ticket + offset) % total_tickets;
        if let Some(pk) = find_producer(ticket) {
            if !result.contains(&pk) {
                result.push(pk);
            }
        }
    }

    result
}

/// Determine the exclusively eligible rank at a given millisecond offset.
///
/// # Sequential 2s Fallback Windows
///
/// Each rank gets an exclusive 2s window. Only ONE rank is eligible at a time:
/// - 0-1999ms: rank 0 (primary)
/// - 2000-3999ms: rank 1
/// - 4000-5999ms: rank 2
/// - 6000-7999ms: rank 3
/// - 8000-9999ms: rank 4
///
/// Returns Some(rank) for the exclusively eligible rank, or None if past slot end.
pub const fn eligible_rank_at_ms(offset_ms: u64) -> Option<usize> {
    let rank = (offset_ms / FALLBACK_TIMEOUT_MS) as usize;
    if rank < MAX_FALLBACK_RANKS {
        Some(rank)
    } else {
        None // Past slot end: no rank eligible
    }
}

/// Check if a specific rank is eligible at a given millisecond offset.
///
/// Uses exclusive semantics: exactly one rank per 2s window, none past slot end.
pub const fn is_rank_eligible_at_ms(rank: usize, offset_ms: u64) -> bool {
    match eligible_rank_at_ms(offset_ms) {
        Some(current_rank) => rank == current_rank,
        None => false, // Past slot end
    }
}

/// Check if a producer is eligible for a slot at the given time (ms precision).
///
/// Uses sequential 2s exclusive windows. Only the producer whose rank matches
/// the current window is eligible (exclusive, not cumulative).
pub fn is_producer_eligible_ms(
    producer: &crypto::PublicKey,
    eligible_producers: &[crypto::PublicKey],
    slot_offset_ms: u64,
) -> bool {
    if let Some(rank) = eligible_producers.iter().position(|p| p == producer) {
        is_rank_eligible_at_ms(rank, slot_offset_ms)
    } else {
        false
    }
}

/// Determine the allowed producer rank based on slot offset (in seconds).
///
/// Delegates to eligible_rank_at_ms() with seconds-to-ms conversion.
/// For exclusive sequential semantics, use eligible_rank_at_ms() directly.
///
/// # Sequential 2s Fallback Windows
/// - 0s: rank 0
/// - 1s: rank 0 (still in 0-1999ms window)
/// - 2s: rank 1 (in 2000-3999ms window)
/// - 4s: rank 2, 6s: rank 3, 8s: rank 4
pub fn allowed_producer_rank(slot_offset_secs: u64) -> usize {
    let offset_ms = slot_offset_secs * 1000;
    match eligible_rank_at_ms(offset_ms) {
        Some(rank) => rank,
        None => MAX_FALLBACK_RANKS - 1, // Clamp: past slot end returns last rank
    }
}

/// Determine the allowed producer rank based on slot offset (in milliseconds).
///
/// Delegates to eligible_rank_at_ms(). Returns the exclusively eligible rank.
pub fn allowed_producer_rank_ms(slot_offset_ms: u64) -> usize {
    match eligible_rank_at_ms(slot_offset_ms) {
        Some(rank) => rank,
        None => MAX_FALLBACK_RANKS - 1, // Clamp: past slot end returns last rank
    }
}

/// Check if a producer rank is eligible at a given time offset.
///
/// # Sequential Fallback Windows (exclusive)
/// Each rank gets an exclusive 2s window. 5 ranks x 2s = 10s (full slot).
pub fn is_rank_eligible_at_offset(rank: usize, offset_ms: u64) -> bool {
    is_rank_eligible_at_ms(rank, offset_ms)
}

/// Check if a producer is eligible for a slot at the given time.
///
/// Uses sequential 2s exclusive windows via is_producer_eligible_ms().
pub fn is_producer_eligible(
    producer: &crypto::PublicKey,
    eligible_producers: &[crypto::PublicKey],
    slot_offset_secs: u64,
) -> bool {
    is_producer_eligible_ms(producer, eligible_producers, slot_offset_secs * 1000)
}

/// Get the rank of a producer for a slot.
///
/// Returns Some(rank) if the producer is in the eligible list,
/// or None if not found.
pub fn get_producer_rank(
    producer: &crypto::PublicKey,
    eligible_producers: &[crypto::PublicKey],
) -> Option<usize> {
    eligible_producers.iter().position(|p| p == producer)
}

/// Calculate scaled fallback windows for non-mainnet slot durations - DEPRECATED.
/// Use sequential 2s windows (FALLBACK_TIMEOUT_MS) instead.
#[deprecated(note = "Use FALLBACK_TIMEOUT_MS for sequential 2s windows")]
pub fn scaled_fallback_windows(slot_duration_secs: u64) -> (u64, u64, u64) {
    let primary = slot_duration_secs / 2;
    let secondary = (slot_duration_secs * 3) / 4;
    let tertiary = slot_duration_secs;
    (primary.max(1), secondary.max(1), tertiary)
}

/// Determine allowed producer rank for non-mainnet networks - DEPRECATED.
/// Use eligible_rank_at_ms() instead.
#[deprecated(note = "Use eligible_rank_at_ms() for sequential 2s windows")]
pub fn allowed_producer_rank_scaled(slot_offset_secs: u64, slot_duration_secs: u64) -> usize {
    #[allow(deprecated)]
    let (primary, secondary, _) = scaled_fallback_windows(slot_duration_secs);

    if slot_offset_secs < primary {
        0
    } else if slot_offset_secs < secondary {
        1
    } else {
        2
    }
}
