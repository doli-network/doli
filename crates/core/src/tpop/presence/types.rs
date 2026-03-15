//! Core data structures for the presence system.
//!
//! Contains VdfLink, PresenceProof, ProducerPresenceState, and PresenceCheckpoint.

use serde::{Deserialize, Serialize};

use crypto::{hash::hash_concat, Hash, Hasher, PublicKey};
use vdf::{VdfOutput, VdfProof};

use crate::types::{BlockHeight, Slot};

use super::{MAX_CHAIN_LENGTH, PRESENCE_DECAY_RATE_BPS, PRESENCE_VDF_ITERATIONS};

// =============================================================================
// VdfLink
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

// =============================================================================
// PresenceProof
// =============================================================================

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

// =============================================================================
// ProducerPresenceState
// =============================================================================

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
    /// Example: [Alice:1, Bob:5] -> [Alice, Bob, Bob, Bob, Bob, Bob]
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
        self.presence_score = super::calculate_presence_score(
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

// =============================================================================
// PresenceCheckpoint
// =============================================================================

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
