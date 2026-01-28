//! # Temporal Proof of Presence (TPoP) for DOLI
//!
//! A revolutionary consensus mechanism where **time is the primary resource**.
//!
//! ## Architecture Overview
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                        TPoP System                              │
//! ├─────────────────────────────────────────────────────────────────┤
//! │                                                                 │
//! │  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐        │
//! │  │  Heartbeat  │───▶│  Collector  │───▶│   Scorer    │        │
//! │  │  (1s VDF)   │    │  (per slot) │    │  (ranking)  │        │
//! │  └─────────────┘    └─────────────┘    └─────────────┘        │
//! │         │                                     │                 │
//! │         ▼                                     ▼                 │
//! │  ┌─────────────┐                      ┌─────────────┐          │
//! │  │  Producer   │◀─────────────────────│  Selection  │          │
//! │  │  (blocks)   │                      │  (by score) │          │
//! │  └─────────────┘                      └─────────────┘          │
//! │                                                                 │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Key Innovation: Micro-VDF
//!
//! Instead of 55-second VDFs (designed for anti-grinding in lotteries),
//! TPoP uses 1-second micro-VDFs. This is sufficient because:
//!
//! 1. **No lottery to grind** - Selection is by score, not hash luck
//! 2. **Bond is anti-Sybil** - VDF doesn't need to be the barrier
//! 3. **Timing is the proof** - Must arrive during slot window
//!
//! ## Resource Efficiency
//!
//! | Metric          | Old (55s VDF) | New (1s VDF) |
//! |-----------------|---------------|--------------|
//! | CPU per slot    | 91%           | 3%           |
//! | Min hardware    | Intel i5      | Raspberry Pi |
//! | Verification    | 500ms         | 10ms         |
//! | Heartbeats/sec  | 0.018         | 1.0          |
//!
//! ## Modules
//!
//! - [`heartbeat`] - Micro-VDF presence proofs (1 second)
//! - [`presence`] - Checkpoints and state management
//! - [`producer`] - Block production with TPoP
//! - [`calibration`] - Dynamic VDF calibration for consistent timing

pub mod calibration;
pub mod heartbeat;
pub mod presence;
pub mod producer;

// Re-export calibration types (VDF tuning)
pub use calibration::{
    CalibrationStats, VdfCalibrator, DEFAULT_VDF_ITERATIONS, MAX_VDF_ITERATIONS,
    MIN_VDF_ITERATIONS, TARGET_VDF_TIME_MS,
};

// Re-export heartbeat types (primary API)
pub use heartbeat::{
    calculate_heartbeat_score, validate_heartbeat_timing, HeartbeatCollector, HeartbeatError,
    PresenceHeartbeat, HEARTBEAT_DEADLINE_SECS, HEARTBEAT_DISCRIMINANT_BITS,
    HEARTBEAT_GRACE_PERIOD_SECS, HEARTBEAT_VDF_ITERATIONS,
};

// Re-export presence types
pub use presence::{
    calculate_presence_score, can_produce_at_time, distribute_epoch_rewards,
    producer_eligibility_offset, rank_producers_by_presence, select_producer_by_presence,
    EpochPresenceRecords, PresenceCheckpoint, PresenceProof, ProducerPresenceState, VdfLink,
    CHECKPOINT_INTERVAL, MAX_PRESENCE_SCORE, PRESENCE_WINDOWS,
};

// Re-export producer types
pub use producer::{
    BootstrapPresenceProducer, PresenceMessage, PresenceMessageHandler, PresenceProducer,
};

// =============================================================================
// SIMPLIFIED PRESENCE STATE
// =============================================================================

use crate::types::{BlockHeight, Epoch, Slot};
use crypto::{Hash, PublicKey};
use std::collections::HashMap;

/// Simplified presence tracking based on heartbeats
#[derive(Clone, Debug, Default)]
pub struct SimplePresenceState {
    /// Producer states indexed by public key
    pub producers: HashMap<PublicKey, SimpleProducerState>,

    /// Current epoch
    pub current_epoch: Epoch,

    /// Last checkpoint height
    pub last_checkpoint: BlockHeight,
}

/// Simplified per-producer state
#[derive(Clone, Debug)]
pub struct SimpleProducerState {
    /// Public key
    pub pubkey: PublicKey,

    /// Heartbeats submitted in current epoch
    pub epoch_heartbeats: u64,

    /// Total heartbeats submitted (lifetime)
    pub total_heartbeats: u64,

    /// Total slots since registration
    pub total_slots: u64,

    /// Consecutive slots with heartbeats
    pub consecutive_present: u64,

    /// Era when registered
    pub registered_era: u32,

    /// Current presence score
    pub score: u64,
}

impl Default for SimpleProducerState {
    fn default() -> Self {
        Self {
            pubkey: PublicKey::from_bytes([0u8; 32]),
            epoch_heartbeats: 0,
            total_heartbeats: 0,
            total_slots: 0,
            consecutive_present: 0,
            registered_era: 0,
            score: 0,
        }
    }
}

impl SimplePresenceState {
    /// Create new state
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a valid heartbeat
    pub fn record_heartbeat(&mut self, producer: &PublicKey) {
        let state = self
            .producers
            .entry(producer.clone())
            .or_insert_with(|| SimpleProducerState {
                pubkey: producer.clone(),
                ..Default::default()
            });

        state.epoch_heartbeats += 1;
        state.total_heartbeats += 1;
        state.consecutive_present += 1;

        // Recalculate score
        state.score = calculate_heartbeat_score(
            state.total_heartbeats,
            state.total_slots.max(1),
            state.consecutive_present,
            state.registered_era,
        );
    }

    /// Record a missed slot for a producer
    pub fn record_missed(&mut self, producer: &PublicKey) {
        if let Some(state) = self.producers.get_mut(producer) {
            state.total_slots += 1;
            state.consecutive_present = 0; // Reset streak

            // Recalculate score
            state.score = calculate_heartbeat_score(
                state.total_heartbeats,
                state.total_slots,
                state.consecutive_present,
                state.registered_era,
            );
        }
    }

    /// Advance to next slot (call for all producers)
    pub fn advance_slot(&mut self) {
        for state in self.producers.values_mut() {
            state.total_slots += 1;
        }
    }

    /// Get ranked producers by score
    pub fn ranked_producers(&self) -> Vec<(PublicKey, u64)> {
        let mut ranked: Vec<_> = self
            .producers
            .iter()
            .map(|(pk, state)| (pk.clone(), state.score))
            .collect();

        // Sort by score descending
        ranked.sort_by(|a, b| b.1.cmp(&a.1));
        ranked
    }

    /// Get producer's current rank (0-indexed)
    pub fn producer_rank(&self, producer: &PublicKey) -> Option<usize> {
        let ranked = self.ranked_producers();
        ranked.iter().position(|(pk, _)| pk == producer)
    }

    /// Check if producer can produce at given time offset
    pub fn can_produce(&self, producer: &PublicKey, slot_offset_secs: u64) -> bool {
        if let Some(rank) = self.producer_rank(producer) {
            can_produce_at_time(rank, slot_offset_secs)
        } else {
            // Unknown producer - use fallback window
            slot_offset_secs >= 45
        }
    }

    /// Start new epoch
    pub fn new_epoch(&mut self, epoch: Epoch) {
        self.current_epoch = epoch;
        for state in self.producers.values_mut() {
            state.epoch_heartbeats = 0;
        }
    }
}

// =============================================================================
// INTEGRATION TRAIT
// =============================================================================

/// Trait for integrating TPoP into existing consensus
pub trait TpopConsensus {
    /// Get the current presence state
    fn presence_state(&self) -> &SimplePresenceState;

    /// Get the heartbeat collector
    fn heartbeat_collector(&self) -> &HeartbeatCollector;

    /// Get the previous block hash (for VDF input)
    fn prev_block_hash(&self) -> Hash;

    /// Get current slot
    fn current_slot(&self) -> Slot;

    /// Check if TPoP is enabled for this height
    fn tpop_enabled(&self, height: BlockHeight) -> bool;

    /// Select producer for block using TPoP
    fn select_producer_tpop(&self, slot_offset_secs: u64) -> Option<PublicKey> {
        let state = self.presence_state();
        let ranked = state.ranked_producers();

        // Find first producer who can produce at this time
        for (rank, (producer, _score)) in ranked.iter().enumerate() {
            if can_produce_at_time(rank, slot_offset_secs) {
                // Check they have a valid heartbeat for this slot
                let collector = self.heartbeat_collector();
                if collector.has_heartbeat(self.current_slot(), producer) {
                    return Some(producer.clone());
                }
            }
        }

        None
    }
}

// =============================================================================
// MIGRATION CONFIG
// =============================================================================

/// Configuration for TPoP activation
#[derive(Clone, Debug)]
pub struct TpopConfig {
    /// Block height to activate TPoP
    pub activation_height: BlockHeight,

    /// Whether to run legacy consensus in parallel during transition
    pub parallel_mode: bool,

    /// Duration of parallel mode (blocks)
    pub parallel_duration: BlockHeight,
}

impl TpopConfig {
    /// Mainnet config (activate after bootstrap)
    pub fn mainnet() -> Self {
        Self {
            activation_height: 10_080, // After 1 week bootstrap
            parallel_mode: true,
            parallel_duration: 1_440, // 1 day parallel
        }
    }

    /// Testnet config (immediate)
    pub fn testnet() -> Self {
        Self {
            activation_height: 100, // Quick activation for testing
            parallel_mode: false,
            parallel_duration: 0,
        }
    }

    /// Devnet config (immediate)
    pub fn devnet() -> Self {
        Self {
            activation_height: 0,
            parallel_mode: false,
            parallel_duration: 0,
        }
    }

    /// Check if TPoP is active
    pub fn is_active(&self, height: BlockHeight) -> bool {
        height >= self.activation_height
    }

    /// Check if in parallel mode
    pub fn is_parallel(&self, height: BlockHeight) -> bool {
        self.parallel_mode
            && height >= self.activation_height
            && height < self.activation_height + self.parallel_duration
    }
}

impl Default for TpopConfig {
    fn default() -> Self {
        Self::mainnet()
    }
}

// =============================================================================
// LEGACY COMPATIBILITY (TpopMigrationConfig)
// =============================================================================

/// Legacy configuration for TPoP migration (alias for TpopConfig)
pub type TpopMigrationConfig = TpopConfig;

// =============================================================================
// METRICS AND MONITORING
// =============================================================================

/// Metrics for TPoP system health
#[derive(Clone, Debug, Default)]
pub struct TpopMetrics {
    /// Number of valid heartbeats received this slot
    pub heartbeats_received: u64,

    /// Number of active producers (with recent valid heartbeats)
    pub active_producers: u64,

    /// Total presence score in the network
    pub total_presence_score: u64,

    /// Average presence score
    pub avg_presence_score: u64,

    /// Gini coefficient of presence scores (0 = perfect equality)
    pub score_gini: f64,

    /// Percentage of slots with successful block production
    pub slot_fill_rate: f64,

    /// Average heartbeats per slot
    pub avg_heartbeats_per_slot: f64,
}

impl TpopMetrics {
    /// Calculate metrics from current state
    pub fn calculate(state: &SimplePresenceState, recent_slots: &[SlotStats]) -> Self {
        let scores: Vec<u64> = state.producers.values().map(|s| s.score).collect();

        let total: u64 = scores.iter().sum();
        let count = scores.len() as u64;
        let avg = if count > 0 { total / count } else { 0 };

        let gini = calculate_gini(&scores);

        let filled = recent_slots.iter().filter(|s| s.block_produced).count();
        let fill_rate = if recent_slots.is_empty() {
            1.0
        } else {
            filled as f64 / recent_slots.len() as f64
        };

        let total_heartbeats: u64 = recent_slots.iter().map(|s| s.heartbeats_received).sum();
        let avg_heartbeats = if recent_slots.is_empty() {
            0.0
        } else {
            total_heartbeats as f64 / recent_slots.len() as f64
        };

        Self {
            heartbeats_received: recent_slots
                .last()
                .map(|s| s.heartbeats_received)
                .unwrap_or(0),
            active_producers: count,
            total_presence_score: total,
            avg_presence_score: avg,
            score_gini: gini,
            slot_fill_rate: fill_rate,
            avg_heartbeats_per_slot: avg_heartbeats,
        }
    }
}

/// Statistics for a single slot
#[derive(Clone, Debug)]
pub struct SlotStats {
    pub slot: Slot,
    pub heartbeats_received: u64,
    pub block_produced: bool,
    pub producer_rank: Option<usize>,
}

/// Calculate Gini coefficient for score distribution
fn calculate_gini(scores: &[u64]) -> f64 {
    if scores.is_empty() || scores.iter().all(|&s| s == 0) {
        return 0.0;
    }

    let n = scores.len() as f64;
    let mean = scores.iter().sum::<u64>() as f64 / n;

    if mean == 0.0 {
        return 0.0;
    }

    let mut sum = 0.0;
    for &xi in scores.iter() {
        for &xj in scores.iter() {
            sum += (xi as f64 - xj as f64).abs();
        }
    }

    sum / (2.0 * n * n * mean)
}

// =============================================================================
// LEGACY TRAIT (PresenceConsensus)
// =============================================================================

/// Legacy trait for integrating TPoP with existing consensus code.
///
/// This allows gradual migration from the current lottery-based
/// selection to presence-based selection.
pub trait PresenceConsensus {
    /// Get the current presence checkpoint
    fn current_checkpoint(&self) -> &PresenceCheckpoint;

    /// Get received presence proofs for a slot
    fn proofs_for_slot(&self, slot: u32) -> Vec<&PresenceProof>;

    /// Select producer using TPoP rules
    fn select_producer_tpop(&self, slot: u32, prev_hash: &Hash) -> Option<PublicKey> {
        let checkpoint = self.current_checkpoint();
        let proofs: HashMap<_, _> = self
            .proofs_for_slot(slot)
            .into_iter()
            .map(|p| (p.producer.clone(), p.clone()))
            .collect();

        select_producer_by_presence(slot, checkpoint, &proofs, prev_hash)
    }

    /// Check if a producer is eligible at a given time
    fn is_eligible_tpop(&self, producer: &PublicKey, slot_offset: u64) -> bool {
        let checkpoint = self.current_checkpoint();
        let rankings = rank_producers_by_presence(checkpoint);

        if let Some((_, _, _, rank)) = rankings.iter().find(|(pk, _, _, _)| pk == producer) {
            can_produce_at_time(*rank, slot_offset)
        } else {
            // Unknown producer - use fallback rank
            can_produce_at_time(rankings.len(), slot_offset)
        }
    }
}

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_pubkey(seed: u8) -> PublicKey {
        let mut bytes = [0u8; 32];
        bytes[0] = seed;
        PublicKey::from_bytes(bytes)
    }

    #[test]
    fn test_simple_presence_state() {
        let mut state = SimplePresenceState::new();

        let alice = mock_pubkey(1);
        let bob = mock_pubkey(2);

        // Alice submits 10 heartbeats
        for _ in 0..10 {
            state.record_heartbeat(&alice);
        }

        // Bob submits 5 heartbeats
        for _ in 0..5 {
            state.record_heartbeat(&bob);
        }

        // Alice should rank higher
        let ranked = state.ranked_producers();
        assert_eq!(ranked[0].0, alice);
        assert_eq!(ranked[1].0, bob);
        assert!(ranked[0].1 > ranked[1].1);
    }

    #[test]
    fn test_producer_ranking() {
        let mut state = SimplePresenceState::new();

        let alice = mock_pubkey(1);
        let bob = mock_pubkey(2);
        let charlie = mock_pubkey(3);

        // Different activity levels
        for _ in 0..100 {
            state.record_heartbeat(&alice);
        }
        for _ in 0..50 {
            state.record_heartbeat(&bob);
        }
        for _ in 0..10 {
            state.record_heartbeat(&charlie);
        }

        assert_eq!(state.producer_rank(&alice), Some(0));
        assert_eq!(state.producer_rank(&bob), Some(1));
        assert_eq!(state.producer_rank(&charlie), Some(2));
    }

    #[test]
    fn test_eligibility_by_rank() {
        let mut state = SimplePresenceState::new();

        let alice = mock_pubkey(1); // Will be rank 0
        let bob = mock_pubkey(2); // Will be rank 1

        for _ in 0..100 {
            state.record_heartbeat(&alice);
        }
        for _ in 0..50 {
            state.record_heartbeat(&bob);
        }

        // At t=0, only rank 0 (alice) can produce
        assert!(state.can_produce(&alice, 0));
        assert!(!state.can_produce(&bob, 0));

        // At t=15, rank 0-2 can produce
        assert!(state.can_produce(&alice, 15));
        assert!(state.can_produce(&bob, 15));

        // At t=45, everyone can produce
        assert!(state.can_produce(&alice, 45));
        assert!(state.can_produce(&bob, 45));
    }

    #[test]
    fn test_missed_slot_penalty() {
        let mut state = SimplePresenceState::new();
        let alice = mock_pubkey(1);

        // Build up presence
        for _ in 0..100 {
            state.record_heartbeat(&alice);
        }
        let score_before = state.producers.get(&alice).unwrap().score;

        // Miss some slots
        for _ in 0..50 {
            state.record_missed(&alice);
        }
        let score_after = state.producers.get(&alice).unwrap().score;

        // Score should decrease
        assert!(score_after < score_before);
    }

    #[test]
    fn test_tpop_config() {
        let config = TpopConfig::mainnet();

        assert!(!config.is_active(0));
        assert!(!config.is_active(10_079));
        assert!(config.is_active(10_080));

        assert!(config.is_parallel(10_080));
        assert!(config.is_parallel(11_000));
        assert!(!config.is_parallel(12_000));
    }

    #[test]
    fn test_gini_perfect_equality() {
        // All same score = Gini 0
        let scores = vec![100, 100, 100, 100];
        let gini = calculate_gini(&scores);
        assert!((gini - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_gini_inequality() {
        // One has everything = Gini approaching 1
        let scores = vec![1000, 0, 0, 0];
        let gini = calculate_gini(&scores);
        assert!(gini > 0.7);
    }
}
