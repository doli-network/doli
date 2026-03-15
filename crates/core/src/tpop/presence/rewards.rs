//! Epoch reward distribution based on presence.
//!
//! This is TELEMETRY ONLY and does not affect consensus.
//! Actual reward distribution is bond-weighted.

use std::collections::HashMap;

use crypto::PublicKey;

use crate::types::{Amount, Slot};

use super::types::PresenceCheckpoint;

/// Calculate reward distribution for an epoch.
///
/// Rewards are distributed proportionally to demonstrated presence,
/// weighted by presence score.
///
/// # Arguments
/// * `total_reward` - Total reward to distribute
/// * `presence_records` - Record of presence proofs in this epoch
/// * `checkpoint` - Current presence checkpoint
///
/// # Returns
/// List of (producer, reward_amount) pairs
pub fn distribute_epoch_rewards(
    total_reward: Amount,
    presence_records: &EpochPresenceRecords,
    checkpoint: &PresenceCheckpoint,
) -> Vec<(PublicKey, Amount)> {
    if checkpoint.producer_states.is_empty() || total_reward == 0 {
        return Vec::new();
    }

    // Calculate effective time for each producer
    // Effective time = slots_present * score_multiplier
    let effective_times: Vec<(PublicKey, u64)> = checkpoint
        .producer_states
        .iter()
        .map(|state| {
            let slots_present = presence_records.slots_with_valid_proof(&state.pubkey);

            // Score multiplier: score / 1000, capped at 2.0
            // This gives high-score producers up to 2x weight
            let score_multiplier_x1000 = state.presence_score.min(2000);
            let effective = (slots_present * score_multiplier_x1000) / 1000;

            (state.pubkey, effective)
        })
        .filter(|(_, effective)| *effective > 0)
        .collect();

    let total_effective: u64 = effective_times.iter().map(|(_, t)| *t).sum();

    if total_effective == 0 {
        return Vec::new();
    }

    // Distribute proportionally
    effective_times
        .into_iter()
        .map(|(pubkey, effective)| {
            let share = ((total_reward as u128) * (effective as u128)) / (total_effective as u128);
            (pubkey, share as Amount)
        })
        .filter(|(_, amount)| *amount > 0)
        .collect()
}

/// Records of presence proofs within an epoch.
#[derive(Clone, Debug, Default)]
pub struct EpochPresenceRecords {
    /// Map of producer -> set of slots with valid proofs
    records: HashMap<PublicKey, Vec<Slot>>,
}

impl EpochPresenceRecords {
    pub fn new() -> Self {
        Self {
            records: HashMap::new(),
        }
    }

    /// Record a valid presence proof
    pub fn record_presence(&mut self, producer: &PublicKey, slot: Slot) {
        self.records.entry(*producer).or_default().push(slot);
    }

    /// Get number of slots with valid proof for a producer
    pub fn slots_with_valid_proof(&self, producer: &PublicKey) -> u64 {
        self.records
            .get(producer)
            .map(|slots| slots.len() as u64)
            .unwrap_or(0)
    }
}
