//! Producer selection using Bond Stacking weighted round-robin.
//!
//! This is TELEMETRY ONLY and does not affect consensus.
//! Actual block production uses `consensus::select_producer_for_slot()`.

use std::collections::HashMap;

use crypto::{Hash, PublicKey};

use crate::types::Slot;

use super::types::{PresenceCheckpoint, PresenceProof};
use super::PRESENCE_WINDOWS;

/// Select the producer for a slot using Bond Stacking weighted selection.
///
/// # Bond Stacking Algorithm
/// 1. Filter: Must have a valid presence proof for this slot
/// 2. Weighted Selection: Each producer's weight = bond_count
/// 3. Deterministic lottery based on total weight
///
/// Example: [Alice(bonds=1), Bob(bonds=5)]
/// -> Total weight = 6
/// -> Alice has 1/6 probability, Bob has 5/6 probability
///
/// This ensures ROI is equitable: all producers get the same percentage return
/// on their bond investment. More bonds = more absolute return, same ROI %.
///
/// ## Deterministic Rotation (NOT Lottery)
///
/// Unlike PoS lottery systems, DOLI uses deterministic round-robin assignment:
///
/// ```text
/// Alice: 1 bond  -> 1 turn per cycle
/// Bob:   5 bonds -> 5 turns per cycle
/// Carol: 4 bonds -> 4 turns per cycle
/// Total: 10 bonds = 10 slots per cycle
///
/// Rotation: [Alice, Bob, Bob, Bob, Bob, Bob, Carol, Carol, Carol, Carol]
///           slot 0  1    2    3    4    5    6      7      8      9
/// ```
///
/// Bob ALWAYS produces exactly 5 of every 10 blocks. No variance, no luck.
///
/// # Arguments
/// * `slot` - The slot to select a producer for
/// * `presence_state` - Current presence checkpoint
/// * `received_proofs` - Presence proofs received from producers
/// * `_prev_hash` - Unused (kept for API compatibility)
///
/// # Returns
/// The selected producer's public key, or None if no valid producer
pub fn select_producer_by_presence(
    slot: Slot,
    presence_state: &PresenceCheckpoint,
    received_proofs: &HashMap<PublicKey, PresenceProof>,
    _prev_hash: &Hash,
) -> Option<PublicKey> {
    // Collect eligible producers (those with valid proofs for this slot)
    // Include: pubkey, bond_count
    let mut eligible: Vec<(&PublicKey, u32)> = presence_state
        .producer_states
        .iter()
        .filter_map(|state| {
            received_proofs
                .get(&state.pubkey)
                .filter(|proof| proof.slot == slot && proof.verify(presence_state))
                .map(|_| (&state.pubkey, state.bond_count.max(1)))
        })
        .collect();

    if eligible.is_empty() {
        return None;
    }

    // Sort by pubkey for deterministic ordering
    // This ensures all nodes compute the same rotation order
    eligible.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));

    // Calculate total tickets (sum of all bond counts)
    let total_tickets: u64 = eligible.iter().map(|(_, bonds)| *bonds as u64).sum();

    if total_tickets == 0 {
        return None;
    }

    // Deterministic round-robin: slot mod total_tickets gives ticket index
    let ticket_index = (slot as u64) % total_tickets;

    // Walk through producers, accumulating tickets until we find the owner
    // Example: [Alice:1, Bob:5, Carol:4] with ticket_index=6
    //   Alice: cumulative=1, 6 >= 1, continue
    //   Bob: cumulative=6, 6 >= 6, continue
    //   Carol: cumulative=10, 6 < 10, SELECT CAROL
    let mut cumulative_tickets: u64 = 0;
    for (pubkey, bonds) in &eligible {
        cumulative_tickets += *bonds as u64;
        if ticket_index < cumulative_tickets {
            return Some(**pubkey);
        }
    }

    // Fallback: return last producer (should not reach here)
    eligible.last().map(|(pk, _)| **pk)
}

/// Select producer with minimum presence score filter.
///
/// Filters out producers below a minimum presence score before
/// applying deterministic round-robin selection. Used for quality control.
pub fn select_producer_by_presence_filtered(
    slot: Slot,
    presence_state: &PresenceCheckpoint,
    received_proofs: &HashMap<PublicKey, PresenceProof>,
    _prev_hash: &Hash,
    min_presence_score: u64,
) -> Option<PublicKey> {
    let mut eligible: Vec<(&PublicKey, u32)> = presence_state
        .producer_states
        .iter()
        .filter(|state| state.presence_score >= min_presence_score)
        .filter_map(|state| {
            received_proofs
                .get(&state.pubkey)
                .filter(|proof| proof.slot == slot && proof.verify(presence_state))
                .map(|_| (&state.pubkey, state.bond_count.max(1)))
        })
        .collect();

    if eligible.is_empty() {
        return None;
    }

    // Sort by pubkey for deterministic ordering
    eligible.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));

    let total_tickets: u64 = eligible.iter().map(|(_, bonds)| *bonds as u64).sum();

    if total_tickets == 0 {
        return None;
    }

    // Deterministic round-robin: slot mod total_tickets
    let ticket_index = (slot as u64) % total_tickets;

    let mut cumulative_tickets: u64 = 0;
    for (pubkey, bonds) in &eligible {
        cumulative_tickets += *bonds as u64;
        if ticket_index < cumulative_tickets {
            return Some(**pubkey);
        }
    }

    eligible.last().map(|(pk, _)| **pk)
}

/// Get the ranked list of producers by presence score and bond count.
///
/// Used for determining fallback eligibility and weighted selection.
/// Returns: (pubkey, score, bond_count, rank)
pub fn rank_producers_by_presence(
    presence_state: &PresenceCheckpoint,
) -> Vec<(PublicKey, u64, u32, usize)> {
    let mut ranked: Vec<_> = presence_state
        .producer_states
        .iter()
        .map(|s| (s.pubkey, s.presence_score, s.bond_count.max(1)))
        .collect();

    // Sort by score descending (bonds don't affect rank, only selection weight)
    ranked.sort_by(|a, b| b.1.cmp(&a.1));

    // Add ranks
    ranked
        .into_iter()
        .enumerate()
        .map(|(rank, (pubkey, score, bonds))| (pubkey, score, bonds, rank))
        .collect()
}

/// Check if a producer can produce at a given time based on their rank.
///
/// Uses PRESENCE_WINDOWS to determine eligibility.
pub fn can_produce_at_time(producer_rank: usize, slot_offset_secs: u64) -> bool {
    for (window_start, max_rank) in PRESENCE_WINDOWS.iter().rev() {
        if slot_offset_secs >= *window_start {
            return producer_rank < *max_rank;
        }
    }
    false
}

/// Determine when a producer becomes eligible to produce.
///
/// # Returns
/// The slot offset (in seconds) when the producer can start producing
pub fn producer_eligibility_offset(producer_rank: usize) -> u64 {
    for (window_start, max_rank) in PRESENCE_WINDOWS.iter() {
        if producer_rank < *max_rank {
            return *window_start;
        }
    }
    // Rank too high, not eligible in any window
    u64::MAX
}
