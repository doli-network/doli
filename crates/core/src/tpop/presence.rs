//! # Temporal Proof of Presence (TPoP) - Telemetry Module
//!
//! **IMPORTANT: This module is TELEMETRY ONLY and does NOT affect consensus.**
//!
//! Producer selection is determined by `consensus::select_producer_for_slot()` using
//! consecutive tickets based on bond count. Presence score does NOT affect:
//! - Block validity
//! - Producer selection
//! - Fallback eligibility
//!
//! This module provides metrics and tracking for network health monitoring.
//!
//! ## Overview
//!
//! The presence tracking system monitors producer activity for telemetry:
//! - Personal VDF chains prove temporal presence (informational)
//! - Presence score tracks commitment (metrics only)
//! - Records used for network health dashboards
//!
//! ## What This Module Does NOT Do
//!
//! - Does NOT affect which producer is selected for a slot
//! - Does NOT validate blocks based on presence
//! - Does NOT gate block production on presence threshold
//! - Does NOT influence reward distribution (bond-weighted)

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crypto::{hash::hash_concat, Hash, Hasher, PublicKey};
use vdf::{VdfOutput, VdfProof};

use crate::network::Network;
use crate::network_params::NetworkParams;
use crate::types::{Amount, BlockHeight, Slot};

// =============================================================================
// CONSTANTS
// =============================================================================

/// VDF iterations for presence proof (~55 seconds on reference hardware)
/// This is calibrated to fit within a 60-second slot with margin for propagation
///
/// Note: This is TELEMETRY ONLY and does not affect consensus.
/// Not yet configurable via NetworkParams since it's informational.
pub const PRESENCE_VDF_ITERATIONS: u64 = 55_000_000;

/// Checkpoint interval in slots (1 hour)
/// Presence state is anchored on-chain at these intervals
pub const CHECKPOINT_INTERVAL: u64 = 60;

/// Maximum VDF chain length before requiring checkpoint
/// Prevents unbounded growth of presence proofs
pub const MAX_CHAIN_LENGTH: usize = 120;

/// Slots of inactivity before presence score starts decaying (mainnet default: 5)
///
/// **Deprecated**: Use `presence_grace_period_for_network(network)` for network-aware calculations.
#[deprecated(
    note = "Use presence_grace_period_for_network(network) for network-aware calculations"
)]
pub const PRESENCE_GRACE_PERIOD: u64 = 5;

/// Get presence grace period for a specific network
pub fn presence_grace_period_for_network(network: Network) -> u64 {
    // Use the general grace_period_secs from NetworkParams
    // For presence-specific behavior, this can be customized
    NetworkParams::load(network).grace_period_secs
}

/// Decay rate for presence score during inactivity (per slot, in basis points)
/// 10 = 0.1% decay per missed slot
pub const PRESENCE_DECAY_RATE_BPS: u64 = 10;

/// Maximum presence score (prevents overflow, ~4 years of perfect presence)
pub const MAX_PRESENCE_SCORE: u64 = 2_000_000;

/// Eligibility windows: (slot_offset_secs, max_rank)
/// Producers with rank < max_rank can produce after slot_offset_secs
pub const PRESENCE_WINDOWS: [(u64, usize); 4] = [
    (0, 1),    // 0-15s: Only #1 can produce
    (15, 3),   // 15-30s: Top 3 can produce
    (30, 10),  // 30-45s: Top 10 can produce
    (45, 100), // 45-60s: Top 100 can produce
];

// =============================================================================
// CORE DATA STRUCTURES
// =============================================================================

/// A single link in the presence VDF chain.
///
/// Each link proves ~60 seconds of sequential computation.
/// The chain is unforgeable because each link depends on the previous output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VdfLink {
    /// Sequence number within the chain (0, 1, 2, ...)
    pub sequence: u64,

    /// Slot this link corresponds to
    pub slot: Slot,

    /// Input hash: H(prev_output || slot || producer_pubkey)
    pub input_hash: Hash,

    /// VDF output (the computed value)
    pub output: VdfOutput,

    /// Wesolowski proof for efficient verification
    pub proof: VdfProof,
}

impl VdfLink {
    /// Create a new VDF link (after computing the VDF)
    pub fn new(
        sequence: u64,
        slot: Slot,
        prev_output: &[u8],
        producer: &PublicKey,
        output: VdfOutput,
        proof: VdfProof,
    ) -> Self {
        let input_hash = Self::compute_input(prev_output, slot, producer);
        Self {
            sequence,
            slot,
            input_hash,
            output,
            proof,
        }
    }

    /// Compute the VDF input from components
    pub fn compute_input(prev_output: &[u8], slot: Slot, producer: &PublicKey) -> Hash {
        hash_concat(&[
            b"DOLI_PRESENCE_V1",
            prev_output,
            &slot.to_le_bytes(),
            producer.as_bytes(),
        ])
    }

    /// Verify this link's VDF proof
    ///
    /// FIXED: Corrected parameter order to match vdf::verify API
    pub fn verify(&self) -> bool {
        vdf::verify(
            &self.input_hash,
            &self.output,
            &self.proof,
            PRESENCE_VDF_ITERATIONS,
        )
        .is_ok()
    }

    /// Verify this link chains correctly from a previous link
    pub fn verify_chain(&self, prev_link: &VdfLink, producer: &PublicKey) -> bool {
        // Check sequence is incrementing
        if self.sequence != prev_link.sequence + 1 {
            return false;
        }

        // Check slot is advancing
        if self.slot <= prev_link.slot {
            return false;
        }

        // Check input is correctly derived from previous output
        let expected_input = Self::compute_input(&prev_link.output.value, self.slot, producer);
        if self.input_hash != expected_input {
            return false;
        }

        // Verify the VDF proof
        self.verify()
    }
}

/// Proof of presence for a specific slot.
///
/// This is broadcast by producers to claim eligibility for block production.
/// It includes the VDF chain from the last checkpoint to the current slot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceProof {
    /// Producer claiming presence
    pub producer: PublicKey,

    /// Slot for which presence is claimed
    pub slot: Slot,

    /// Chain of VDF links from last checkpoint
    /// First link chains from checkpoint, last link is for current slot
    pub vdf_chain: Vec<VdfLink>,

    /// Height of the checkpoint this proof chains from
    pub checkpoint_height: BlockHeight,

    /// Hash of the checkpoint (for verification)
    pub checkpoint_hash: Hash,
}

impl PresenceProof {
    /// Verify the presence proof
    pub fn verify(&self, expected_checkpoint: &PresenceCheckpoint) -> bool {
        // Verify checkpoint reference
        if self.checkpoint_height != expected_checkpoint.height {
            return false;
        }
        if self.checkpoint_hash != expected_checkpoint.hash() {
            return false;
        }

        // Chain must not be empty
        if self.vdf_chain.is_empty() {
            return false;
        }

        // Chain must not exceed maximum length
        if self.vdf_chain.len() > MAX_CHAIN_LENGTH {
            return false;
        }

        // Last link must be for the claimed slot
        if self.vdf_chain.last().unwrap().slot != self.slot {
            return false;
        }

        // Verify first link chains from checkpoint
        let first_link = &self.vdf_chain[0];
        let checkpoint_output = expected_checkpoint
            .get_producer_state(&self.producer)
            .map(|s| s.last_vdf_output.clone())
            .unwrap_or_else(|| vec![0u8; 32]); // Genesis output for new producers

        let expected_input =
            VdfLink::compute_input(&checkpoint_output, first_link.slot, &self.producer);
        if first_link.input_hash != expected_input {
            return false;
        }
        if !first_link.verify() {
            return false;
        }

        // Verify chain continuity
        for window in self.vdf_chain.windows(2) {
            if !window[1].verify_chain(&window[0], &self.producer) {
                return false;
            }
        }

        true
    }

    /// Count how many slots have valid presence in this proof
    pub fn slots_present(&self) -> u64 {
        self.vdf_chain.len() as u64
    }
}

/// State of a producer's presence at a checkpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProducerPresenceState {
    /// Producer's public key
    pub pubkey: PublicKey,

    /// Last verified VDF output (for chaining)
    pub last_vdf_output: Vec<u8>,

    /// Slot of last verified VDF
    pub last_slot: Slot,

    /// Sequence number of last VDF
    pub last_sequence: u64,

    /// Consecutive slots with presence
    pub consecutive_presence: u64,

    /// Total slots active since registration
    pub total_slots_active: u64,

    /// Total slots missed (where valid proof was expected but not provided)
    pub missed_slots: u64,

    /// Current presence score
    pub presence_score: u64,

    /// Era when producer registered (for age calculation)
    pub registered_era: u32,

    /// Number of bonds staked (for weighted selection)
    ///
    /// Bond Stacking: More bonds = proportionally more chances to produce.
    /// In selection, producers are expanded by their bond_count.
    /// Example: [Alice:1, Bob:5] → [Alice, Bob, Bob, Bob, Bob, Bob]
    #[serde(default = "default_bond_count")]
    pub bond_count: u32,
}

/// Default bond count for backwards compatibility
fn default_bond_count() -> u32 {
    1
}

impl ProducerPresenceState {
    /// Create initial state for a new producer with a single bond
    pub fn new(pubkey: PublicKey, current_era: u32) -> Self {
        Self::with_bonds(pubkey, current_era, 1)
    }

    /// Create initial state for a new producer with specified bond count
    pub fn with_bonds(pubkey: PublicKey, current_era: u32, bond_count: u32) -> Self {
        Self {
            pubkey,
            last_vdf_output: vec![0u8; 32],
            last_slot: 0,
            last_sequence: 0,
            consecutive_presence: 0,
            total_slots_active: 0,
            missed_slots: 0,
            presence_score: 0,
            registered_era: current_era,
            bond_count: bond_count.max(1),
        }
    }

    /// Update bond count (when producer adds/removes bonds)
    pub fn set_bond_count(&mut self, count: u32) {
        self.bond_count = count.max(1);
    }

    /// Update state after verifying a presence proof
    pub fn apply_presence(&mut self, proof: &PresenceProof, current_era: u32) {
        let last_link = proof.vdf_chain.last().unwrap();

        // Update VDF chain state
        self.last_vdf_output = last_link.output.value.clone();
        self.last_slot = last_link.slot;
        self.last_sequence = last_link.sequence;

        // Calculate slots in this proof
        let slots_in_proof = proof.vdf_chain.len() as u64;

        // Update presence tracking
        self.total_slots_active += slots_in_proof;
        self.consecutive_presence += slots_in_proof;

        // Recalculate presence score
        self.presence_score = calculate_presence_score(
            self.consecutive_presence,
            self.total_slots_active,
            self.missed_slots,
            current_era.saturating_sub(self.registered_era),
        );
    }

    /// Apply decay for missed slots
    pub fn apply_missed_slots(&mut self, count: u64) {
        self.missed_slots += count;
        self.consecutive_presence = 0; // Reset consecutive streak

        // Apply score decay
        let decay = (self.presence_score * PRESENCE_DECAY_RATE_BPS * count) / 10_000;
        self.presence_score = self.presence_score.saturating_sub(decay);
    }
}

/// Checkpoint anchoring presence state on-chain.
///
/// Created every CHECKPOINT_INTERVAL blocks to:
/// 1. Anchor presence state for light client verification
/// 2. Allow pruning of old VDF chains
/// 3. Provide recovery point after restarts
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceCheckpoint {
    /// Block height of this checkpoint
    pub height: BlockHeight,

    /// Slot of this checkpoint
    pub slot: Slot,

    /// Previous checkpoint hash (for chaining)
    pub prev_checkpoint_hash: Hash,

    /// All active producers' presence states
    pub producer_states: Vec<ProducerPresenceState>,

    /// Merkle root of producer states (for efficient verification)
    pub states_merkle_root: Hash,

    /// Total presence score across all producers
    pub total_presence_score: u64,
}

impl PresenceCheckpoint {
    /// Create a new checkpoint
    pub fn new(
        height: BlockHeight,
        slot: Slot,
        prev_checkpoint_hash: Hash,
        producer_states: Vec<ProducerPresenceState>,
    ) -> Self {
        let states_merkle_root = Self::compute_merkle_root(&producer_states);
        let total_presence_score = producer_states.iter().map(|s| s.presence_score).sum();

        Self {
            height,
            slot,
            prev_checkpoint_hash,
            producer_states,
            states_merkle_root,
            total_presence_score,
        }
    }

    /// Compute checkpoint hash
    pub fn hash(&self) -> Hash {
        let mut hasher = Hasher::new();
        hasher.update(b"DOLI_PRESENCE_CHECKPOINT_V1");
        hasher.update(&self.height.to_le_bytes());
        hasher.update(&self.slot.to_le_bytes());
        hasher.update(self.prev_checkpoint_hash.as_bytes());
        hasher.update(self.states_merkle_root.as_bytes());
        hasher.update(&self.total_presence_score.to_le_bytes());
        hasher.finalize()
    }

    /// Get a producer's presence state
    pub fn get_producer_state(&self, pubkey: &PublicKey) -> Option<&ProducerPresenceState> {
        self.producer_states.iter().find(|s| &s.pubkey == pubkey)
    }

    /// Compute Merkle root of producer states
    fn compute_merkle_root(states: &[ProducerPresenceState]) -> Hash {
        if states.is_empty() {
            return Hash::ZERO;
        }

        let mut hashes: Vec<Hash> = states
            .iter()
            .map(|s| {
                let mut hasher = Hasher::new();
                hasher.update(s.pubkey.as_bytes());
                hasher.update(&s.presence_score.to_le_bytes());
                hasher.update(&s.last_sequence.to_le_bytes());
                hasher.finalize()
            })
            .collect();

        while hashes.len() > 1 {
            if hashes.len() % 2 == 1 {
                hashes.push(*hashes.last().unwrap());
            }

            hashes = hashes
                .chunks(2)
                .map(|pair| {
                    let mut hasher = Hasher::new();
                    hasher.update(pair[0].as_bytes());
                    hasher.update(pair[1].as_bytes());
                    hasher.finalize()
                })
                .collect();
        }

        hashes[0]
    }
}

// =============================================================================
// PRESENCE SCORE CALCULATION
// =============================================================================

/// Calculate the presence score for a producer.
///
/// The score reflects temporal commitment:
/// - Consecutive presence rewards consistency
/// - Historical ratio rewards reliability
/// - Age bonus rewards loyalty (but logarithmically to prevent dynasties)
///
/// # Arguments
/// * `consecutive_slots` - Current streak of consecutive presence
/// * `total_slots_active` - Total slots since registration
/// * `missed_slots` - Slots where expected proof was not provided
/// * `age_in_eras` - Number of complete eras since registration
///
/// # Returns
/// Presence score (capped at MAX_PRESENCE_SCORE)
pub fn calculate_presence_score(
    consecutive_slots: u64,
    total_slots_active: u64,
    missed_slots: u64,
    age_in_eras: u32,
) -> u64 {
    // Base score: consecutive presence (capped contribution)
    // 1 point per consecutive slot, max 10,000 from this component
    let consecutive_bonus = consecutive_slots.min(10_000);

    // Historical presence ratio (0-100 points per 1000 slots)
    // This rewards reliability over time
    let presence_ratio = if total_slots_active > 0 {
        let present = total_slots_active.saturating_sub(missed_slots);
        (present * 100) / total_slots_active
    } else {
        0
    };
    let history_bonus = (total_slots_active / 1000) * presence_ratio;

    // Age bonus: logarithmic scaling
    // Era 0: 0, Era 1: 5, Era 2: 10, Era 4: 15, Era 8: 20, ...
    let age_bonus = if age_in_eras > 0 {
        ((age_in_eras as u64).ilog2() as u64 + 1) * 5
    } else {
        0
    };

    // Combine components
    let score = consecutive_bonus
        .saturating_add(history_bonus)
        .saturating_add(age_bonus);

    // Cap at maximum
    score.min(MAX_PRESENCE_SCORE)
}

// =============================================================================
// PRODUCER SELECTION (Bond Stacking)
// =============================================================================

/// Select the producer for a slot using Bond Stacking weighted selection.
///
/// # Bond Stacking Algorithm
/// 1. Filter: Must have a valid presence proof for this slot
/// 2. Weighted Selection: Each producer's weight = bond_count
/// 3. Deterministic lottery based on total weight
///
/// Example: [Alice(bonds=1), Bob(bonds=5)]
/// → Total weight = 6
/// → Alice has 1/6 probability, Bob has 5/6 probability
///
/// This ensures ROI is equitable: all producers get the same percentage return
/// on their bond investment. More bonds = more absolute return, same ROI %.
///
/// ## Deterministic Rotation (NOT Lottery)
///
/// Unlike PoS lottery systems, DOLI uses deterministic round-robin assignment:
///
/// ```text
/// Alice: 1 bond  → 1 turn per cycle
/// Bob:   5 bonds → 5 turns per cycle
/// Carol: 4 bonds → 4 turns per cycle
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
            return Some((*pubkey).clone());
        }
    }

    // Fallback: return last producer (should not reach here)
    eligible.last().map(|(pk, _)| (*pk).clone())
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
            return Some((*pubkey).clone());
        }
    }

    eligible.last().map(|(pk, _)| (*pk).clone())
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
        .map(|s| (s.pubkey.clone(), s.presence_score, s.bond_count.max(1)))
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

// =============================================================================
// REWARD DISTRIBUTION
// =============================================================================

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
            let score_multiplier_x1000 = (state.presence_score.min(2000)) as u64;
            let effective = (slots_present * score_multiplier_x1000) / 1000;

            (state.pubkey.clone(), effective)
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
        self.records.entry(producer.clone()).or_default().push(slot);
    }

    /// Get number of slots with valid proof for a producer
    pub fn slots_with_valid_proof(&self, producer: &PublicKey) -> u64 {
        self.records
            .get(producer)
            .map(|slots| slots.len() as u64)
            .unwrap_or(0)
    }
}

// =============================================================================
// VDF COMPUTATION HELPERS
// =============================================================================

/// Compute the next VDF link in a presence chain.
///
/// This is called continuously by producers to maintain their presence chain.
pub fn compute_next_presence_vdf(
    prev_link: &VdfLink,
    current_slot: Slot,
    producer: &PublicKey,
) -> Result<VdfLink, &'static str> {
    // Compute input from previous output
    let input = VdfLink::compute_input(&prev_link.output.value, current_slot, producer);

    // Compute VDF (this takes ~55 seconds)
    let (output, proof) =
        vdf::compute(&input, PRESENCE_VDF_ITERATIONS).map_err(|_| "VDF computation failed")?;

    Ok(VdfLink::new(
        prev_link.sequence + 1,
        current_slot,
        &prev_link.output.value,
        producer,
        output,
        proof,
    ))
}

/// Create the genesis VDF link for a new producer.
pub fn create_genesis_vdf_link(slot: Slot, producer: &PublicKey) -> Result<VdfLink, &'static str> {
    let genesis_output = vec![0u8; 32];
    let input = VdfLink::compute_input(&genesis_output, slot, producer);

    let (output, proof) =
        vdf::compute(&input, PRESENCE_VDF_ITERATIONS).map_err(|_| "VDF computation failed")?;

    Ok(VdfLink::new(
        0,
        slot,
        &genesis_output,
        producer,
        output,
        proof,
    ))
}

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_presence_score_calculation() {
        // New producer with no history
        assert_eq!(calculate_presence_score(0, 0, 0, 0), 0);

        // Producer with perfect short history
        assert_eq!(calculate_presence_score(100, 100, 0, 0), 100); // 100 consecutive

        // Producer with perfect long history
        // consecutive: 10000, history: (10000/1000)*100 = 1000
        assert_eq!(calculate_presence_score(10_000, 10_000, 0, 0), 11_000);

        // Producer with 50% reliability
        // consecutive: 0, history: (1000/1000)*50 = 50
        assert_eq!(calculate_presence_score(0, 1000, 500, 0), 50);

        // Producer with age bonus (era 4 = log2(4)+1 = 3, *5 = 15)
        assert_eq!(calculate_presence_score(0, 0, 0, 4), 15);

        // Score capping
        assert!(
            calculate_presence_score(MAX_PRESENCE_SCORE * 2, MAX_PRESENCE_SCORE * 2, 0, 100)
                <= MAX_PRESENCE_SCORE
        );
    }

    #[test]
    fn test_eligibility_windows() {
        // Rank 0 can produce immediately
        assert!(can_produce_at_time(0, 0));
        assert!(can_produce_at_time(0, 30));

        // Rank 1-2 must wait until 15s
        assert!(!can_produce_at_time(1, 0));
        assert!(!can_produce_at_time(2, 14));
        assert!(can_produce_at_time(1, 15));
        assert!(can_produce_at_time(2, 15));

        // Rank 3-9 must wait until 30s
        assert!(!can_produce_at_time(5, 29));
        assert!(can_produce_at_time(5, 30));
        assert!(can_produce_at_time(9, 30));

        // Rank 10-99 must wait until 45s
        assert!(!can_produce_at_time(50, 44));
        assert!(can_produce_at_time(50, 45));
        assert!(can_produce_at_time(99, 45));

        // Rank 100+ cannot produce
        assert!(!can_produce_at_time(100, 59));
    }

    #[test]
    fn test_producer_eligibility_offset() {
        assert_eq!(producer_eligibility_offset(0), 0);
        assert_eq!(producer_eligibility_offset(2), 15);
        assert_eq!(producer_eligibility_offset(9), 30);
        assert_eq!(producer_eligibility_offset(99), 45);
        assert_eq!(producer_eligibility_offset(100), u64::MAX);
    }

    #[test]
    fn test_vdf_link_input_determinism() {
        let output1 = vec![1, 2, 3];
        let output2 = vec![1, 2, 3];
        let slot = 42;
        let producer = PublicKey::from_bytes([7u8; 32]);

        let input1 = VdfLink::compute_input(&output1, slot, &producer);
        let input2 = VdfLink::compute_input(&output2, slot, &producer);

        assert_eq!(input1, input2);

        // Different inputs produce different hashes
        let input3 = VdfLink::compute_input(&output1, slot + 1, &producer);
        assert_ne!(input1, input3);
    }

    #[test]
    fn test_epoch_presence_records() {
        let mut records = EpochPresenceRecords::new();
        let producer = PublicKey::from_bytes([1u8; 32]);

        assert_eq!(records.slots_with_valid_proof(&producer), 0);

        records.record_presence(&producer, 10);
        records.record_presence(&producer, 11);
        records.record_presence(&producer, 12);

        assert_eq!(records.slots_with_valid_proof(&producer), 3);

        let other = PublicKey::from_bytes([2u8; 32]);
        assert_eq!(records.slots_with_valid_proof(&other), 0);
    }

    #[test]
    fn test_rank_producers_includes_bond_count() {
        // Test that rank_producers_by_presence includes bond_count in output
        let producer1 = PublicKey::from_bytes([1u8; 32]);
        let producer2 = PublicKey::from_bytes([2u8; 32]);

        let checkpoint = PresenceCheckpoint::new(
            0,
            0,
            Hash::default(),
            vec![
                ProducerPresenceState::with_bonds(producer1.clone(), 0, 5), // 5 bonds
                ProducerPresenceState::with_bonds(producer2.clone(), 0, 10), // 10 bonds
            ],
        );

        let rankings = rank_producers_by_presence(&checkpoint);

        assert_eq!(rankings.len(), 2);
        // Both have same score (0), so order might vary
        // Just verify bond counts are included correctly
        let prod1_entry = rankings
            .iter()
            .find(|(pk, _, _, _)| pk == &producer1)
            .unwrap();
        let prod2_entry = rankings
            .iter()
            .find(|(pk, _, _, _)| pk == &producer2)
            .unwrap();

        assert_eq!(prod1_entry.2, 5); // bond_count
        assert_eq!(prod2_entry.2, 10); // bond_count
    }

    #[test]
    fn test_deterministic_round_robin_selection() {
        // Test deterministic round-robin rotation (NOT lottery)
        //
        // With Alice:1, Bob:5, Carol:4 bonds (total 10):
        // - Alice gets slot 0 (1 turn per cycle)
        // - Bob gets slots 1-5 (5 turns per cycle)
        // - Carol gets slots 6-9 (4 turns per cycle)
        //
        // This is DETERMINISTIC, not probabilistic!

        // Create producers sorted by pubkey bytes (this is how production sorts)
        let alice = PublicKey::from_bytes([0x01; 32]); // Lowest pubkey
        let bob = PublicKey::from_bytes([0x02; 32]); // Middle pubkey
        let carol = PublicKey::from_bytes([0x03; 32]); // Highest pubkey

        // Eligible producers: (pubkey, bonds) - will be sorted by pubkey
        let mut eligible: Vec<(&PublicKey, u32)> = vec![
            (&alice, 1), // 1 bond = 1 ticket
            (&bob, 5),   // 5 bonds = 5 tickets
            (&carol, 4), // 4 bonds = 4 tickets
        ];
        eligible.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));

        let total_tickets: u64 = eligible.iter().map(|(_, bonds)| *bonds as u64).sum();
        assert_eq!(total_tickets, 10);

        // Verify exact assignments for a full cycle
        // Sorted order: Alice (0x01), Bob (0x02), Carol (0x03)
        // Tickets: [Alice, Bob, Bob, Bob, Bob, Bob, Carol, Carol, Carol, Carol]
        // Indices:    0      1    2    3    4    5     6      7      8      9

        fn select_for_slot(
            eligible: &[(&PublicKey, u32)],
            slot: u64,
            total_tickets: u64,
        ) -> PublicKey {
            let ticket_index = slot % total_tickets;
            let mut cumulative: u64 = 0;
            for (pubkey, bonds) in eligible {
                cumulative += *bonds as u64;
                if ticket_index < cumulative {
                    return (*pubkey).clone();
                }
            }
            (*eligible.last().unwrap().0).clone()
        }

        // Test first cycle (slots 0-9)
        assert_eq!(select_for_slot(&eligible, 0, total_tickets), alice); // ticket 0 → Alice
        assert_eq!(select_for_slot(&eligible, 1, total_tickets), bob); // ticket 1 → Bob
        assert_eq!(select_for_slot(&eligible, 2, total_tickets), bob); // ticket 2 → Bob
        assert_eq!(select_for_slot(&eligible, 3, total_tickets), bob); // ticket 3 → Bob
        assert_eq!(select_for_slot(&eligible, 4, total_tickets), bob); // ticket 4 → Bob
        assert_eq!(select_for_slot(&eligible, 5, total_tickets), bob); // ticket 5 → Bob
        assert_eq!(select_for_slot(&eligible, 6, total_tickets), carol); // ticket 6 → Carol
        assert_eq!(select_for_slot(&eligible, 7, total_tickets), carol); // ticket 7 → Carol
        assert_eq!(select_for_slot(&eligible, 8, total_tickets), carol); // ticket 8 → Carol
        assert_eq!(select_for_slot(&eligible, 9, total_tickets), carol); // ticket 9 → Carol

        // Test cycle repeats (slot 10 = same as slot 0)
        assert_eq!(select_for_slot(&eligible, 10, total_tickets), alice);
        assert_eq!(select_for_slot(&eligible, 11, total_tickets), bob);
        assert_eq!(select_for_slot(&eligible, 16, total_tickets), carol);

        // Count over 100 slots - must be EXACTLY proportional
        let mut alice_count = 0u32;
        let mut bob_count = 0u32;
        let mut carol_count = 0u32;

        for slot in 0..100 {
            let selected = select_for_slot(&eligible, slot, total_tickets);
            if selected == alice {
                alice_count += 1;
            } else if selected == bob {
                bob_count += 1;
            } else if selected == carol {
                carol_count += 1;
            }
        }

        // EXACT counts, not approximate!
        assert_eq!(
            alice_count, 10,
            "Alice (1 bond) should produce exactly 10 of 100 blocks"
        );
        assert_eq!(
            bob_count, 50,
            "Bob (5 bonds) should produce exactly 50 of 100 blocks"
        );
        assert_eq!(
            carol_count, 40,
            "Carol (4 bonds) should produce exactly 40 of 100 blocks"
        );

        // ROI verification: all producers have same ROI percentage
        // Alice: 10 blocks / 1 bond = 10 blocks per bond
        // Bob: 50 blocks / 5 bonds = 10 blocks per bond
        // Carol: 40 blocks / 4 bonds = 10 blocks per bond
        assert_eq!(alice_count as f64 / 1.0, 10.0);
        assert_eq!(bob_count as f64 / 5.0, 10.0);
        assert_eq!(carol_count as f64 / 4.0, 10.0);
    }
}

// =============================================================================
// PROPERTY-BASED TESTS
// =============================================================================

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Presence score is always within bounds
        #[test]
        fn prop_score_bounded(
            consecutive in 0u64..1_000_000,
            total in 0u64..10_000_000,
            missed in 0u64..10_000_000,
            age in 0u32..100,
        ) {
            let score = calculate_presence_score(consecutive, total, missed, age);
            prop_assert!(score <= MAX_PRESENCE_SCORE);
        }

        /// Score is monotonically increasing with consecutive presence
        #[test]
        fn prop_score_increases_with_consecutive(
            base in 0u64..10_000,
            delta in 1u64..1000,
        ) {
            let score1 = calculate_presence_score(base, base, 0, 0);
            let score2 = calculate_presence_score(base + delta, base + delta, 0, 0);
            prop_assert!(score2 >= score1);
        }

        /// Score decreases with missed slots (given same total)
        #[test]
        fn prop_score_decreases_with_missed(
            total in 1000u64..100_000,
            missed in 1u64..999,
        ) {
            let score1 = calculate_presence_score(0, total, 0, 0);
            let score2 = calculate_presence_score(0, total, missed, 0);
            prop_assert!(score2 <= score1);
        }

        /// Eligibility windows are properly ordered
        #[test]
        fn prop_windows_ordered(rank in 0usize..200) {
            let offset = producer_eligibility_offset(rank);
            if offset < u64::MAX {
                // If eligible at offset, should be eligible at any later time
                prop_assert!(can_produce_at_time(rank, offset));
                prop_assert!(can_produce_at_time(rank, offset + 10));
            }
        }
    }
}
