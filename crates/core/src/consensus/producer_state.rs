use crate::types::Slot;

use super::constants::{
    PresenceScore, INITIAL_PRESENCE_SCORE, MAX_PRESENCE_SCORE, MIN_PRESENCE_RATE,
    MIN_PRESENCE_SCORE, SCORE_MISS_PENALTY, SCORE_PRODUCE_BONUS,
};

/// Producer state tracked on-chain.
///
/// # Proof of Time
///
/// In PoT, time is proven by producing blocks with valid VDF when selected:
/// - One producer per slot (10 seconds)
/// - Producer receives 100% of block reward
/// - Bond count determines selection (round-robin)
/// - VDF provides anti-grinding protection
///
/// There are no multi-signature attestations. The act of producing
/// a valid block with VDF IS the proof of time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProducerState {
    /// Producer's public key hash
    pub pubkey_hash: crypto::Hash,
    /// Current presence score (higher = selected more often)
    pub presence_score: PresenceScore,
    /// Total blocks successfully produced
    pub blocks_produced: u64,
    /// Total assigned slots where producer failed to produce
    pub blocks_missed: u64,
    /// Slot of last successful block production
    pub last_produced_slot: Slot,
    /// Slot when producer registered
    pub registered_slot: Slot,
}

impl ProducerState {
    /// Create a new producer state with initial score
    pub fn new(pubkey_hash: crypto::Hash, registered_slot: Slot) -> Self {
        Self {
            pubkey_hash,
            presence_score: INITIAL_PRESENCE_SCORE,
            blocks_produced: 0,
            blocks_missed: 0,
            last_produced_slot: 0,
            registered_slot,
        }
    }

    /// Calculate presence rate as a percentage (0-100)
    pub fn presence_rate(&self) -> u8 {
        let total = self.blocks_produced + self.blocks_missed;
        if total == 0 {
            return 100; // New producer, assume good faith
        }
        ((self.blocks_produced * 100) / total).min(100) as u8
    }

    /// Check if producer meets minimum presence requirement
    pub fn meets_minimum(&self) -> bool {
        self.presence_rate() >= MIN_PRESENCE_RATE as u8
    }

    /// Record successful block production
    pub fn record_produced(&mut self, slot: Slot) {
        self.blocks_produced += 1;
        self.last_produced_slot = slot;
        self.presence_score = self
            .presence_score
            .saturating_add(SCORE_PRODUCE_BONUS)
            .min(MAX_PRESENCE_SCORE);
    }

    /// Record a missed slot
    pub fn record_missed(&mut self) {
        self.blocks_missed += 1;
        self.presence_score = self
            .presence_score
            .saturating_sub(SCORE_MISS_PENALTY)
            .max(MIN_PRESENCE_SCORE);
    }

    /// Check if producer is active (produced recently)
    pub fn is_active(&self, current_slot: Slot, inactivity_threshold: Slot) -> bool {
        if self.blocks_produced == 0 {
            // New producer, check if registration is recent
            return current_slot.saturating_sub(self.registered_slot) < inactivity_threshold;
        }
        current_slot.saturating_sub(self.last_produced_slot) < inactivity_threshold
    }
}
