//! Chain state management

use std::collections::HashMap;
use std::path::Path;

use crypto::Hash;
use doli_core::consensus::TOTAL_SUPPLY;
use doli_core::types::Amount;
use serde::{Deserialize, Serialize};

use crate::StorageError;

/// Persistent chain state
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChainState {
    /// Best block hash
    pub best_hash: Hash,
    /// Best block height
    pub best_height: u64,
    /// Best block slot
    pub best_slot: u32,
    /// Total difficulty (not used in VDF, but kept for compatibility)
    pub total_work: u64,
    /// Genesis hash
    pub genesis_hash: Hash,
    /// Genesis timestamp (for devnet dynamic genesis_time)
    /// 0 means not yet set - will be initialized on first block production or sync
    #[serde(default)]
    pub genesis_timestamp: u64,
    /// Hash of the last registration transaction (for chained VDF anti-Sybil)
    ///
    /// New registrations must reference this hash, creating a chain that
    /// prevents parallel registration attacks. Hash::ZERO before any registration.
    #[serde(default)]
    pub last_registration_hash: Hash,
    /// Global registration sequence number (for chained VDF anti-Sybil)
    ///
    /// Increments with each new registration. The next registration must
    /// have sequence_number = registration_sequence + 1.
    #[serde(default)]
    pub registration_sequence: u64,
    /// Total coins minted (including genesis reward)
    ///
    /// This tracks all coinbase rewards ever issued. Used to enforce
    /// the TOTAL_SUPPLY cap: `total_minted + reward <= TOTAL_SUPPLY`.
    #[serde(default)]
    pub total_minted: Amount,

    // ==================== Epoch State Persistence ====================
    // These fields persist epoch tracking across node restarts.
    // Without persistence, epoch counters reset and rewards are never distributed.

    /// Epoch tracking: maps producer pubkey (hex) to blocks produced in current epoch
    /// Used for proportional reward distribution in EpochPool mode
    #[serde(default)]
    pub epoch_producer_blocks: HashMap<String, u64>,

    /// Epoch tracking: starting height of current reward epoch
    #[serde(default)]
    pub epoch_start_height: u64,

    /// Epoch tracking: total rewards accumulated in current epoch (not yet distributed)
    #[serde(default)]
    pub epoch_reward_pool: u64,

    /// Current reward epoch number (increments at epoch boundaries)
    #[serde(default)]
    pub current_reward_epoch: u64,
}

impl ChainState {
    /// Create initial chain state
    pub fn new(genesis_hash: Hash) -> Self {
        Self {
            best_hash: genesis_hash,
            best_height: 0,
            best_slot: 0,
            total_work: 0,
            genesis_hash,
            genesis_timestamp: 0, // Will be set when first block is produced/synced
            last_registration_hash: Hash::ZERO,
            registration_sequence: 0,
            total_minted: 0,
            // Epoch state (persisted across restarts)
            epoch_producer_blocks: HashMap::new(),
            epoch_start_height: 0,
            epoch_reward_pool: 0,
            current_reward_epoch: 0,
        }
    }

    /// Load from disk
    pub fn load(path: &Path) -> Result<Self, StorageError> {
        let bytes = std::fs::read(path)?;
        bincode::deserialize(&bytes).map_err(|e| StorageError::Serialization(e.to_string()))
    }

    /// Save to disk
    pub fn save(&self, path: &Path) -> Result<(), StorageError> {
        let bytes =
            bincode::serialize(self).map_err(|e| StorageError::Serialization(e.to_string()))?;
        std::fs::write(path, bytes)?;
        Ok(())
    }

    /// Update with a new best block
    pub fn update(&mut self, hash: Hash, height: u64, slot: u32) {
        self.best_hash = hash;
        self.best_height = height;
        self.best_slot = slot;
        self.total_work += 1;
    }

    /// Check if this is the genesis state
    pub fn is_genesis(&self) -> bool {
        self.best_height == 0 && self.best_hash == self.genesis_hash
    }

    /// Update the registration chain after a successful registration
    ///
    /// # Arguments
    /// - `registration_tx_hash`: Hash of the registration transaction
    ///
    /// This should be called after a registration transaction is validated
    /// and included in a block.
    pub fn record_registration(&mut self, registration_tx_hash: Hash) {
        self.last_registration_hash = registration_tx_hash;
        self.registration_sequence += 1;
    }

    /// Get the expected prev_registration_hash for the next registration
    ///
    /// Returns the hash that the next registration's `prev_registration_hash`
    /// field must match.
    pub fn expected_prev_registration_hash(&self) -> Hash {
        self.last_registration_hash
    }

    /// Get the expected sequence number for the next registration
    ///
    /// Returns the sequence number that the next registration must have.
    /// For the first registration, this returns 0.
    pub fn expected_registration_sequence(&self) -> u64 {
        if self.last_registration_hash == Hash::ZERO {
            0 // First registration
        } else {
            self.registration_sequence + 1
        }
    }

    /// Verify that a registration's chain fields are valid
    ///
    /// # Arguments
    /// - `prev_hash`: The `prev_registration_hash` from the registration data
    /// - `sequence`: The `sequence_number` from the registration data
    ///
    /// # Returns
    /// `true` if the registration chain fields are valid
    pub fn verify_registration_chain(&self, prev_hash: Hash, sequence: u64) -> bool {
        let expected_prev = self.expected_prev_registration_hash();
        let expected_seq = self.expected_registration_sequence();

        prev_hash == expected_prev && sequence == expected_seq
    }

    // ==================== Supply Tracking ====================

    /// Apply a coinbase reward to the chain state
    ///
    /// This records the minted coins and enforces the TOTAL_SUPPLY cap.
    /// Returns `Err` if applying this reward would exceed the cap.
    ///
    /// # Arguments
    /// - `reward`: The coinbase reward amount
    ///
    /// # Returns
    /// `Ok(())` if the reward was applied successfully
    /// `Err(amount_over)` if this would exceed TOTAL_SUPPLY
    pub fn apply_coinbase(&mut self, reward: Amount) -> Result<(), Amount> {
        let new_total = self.total_minted.saturating_add(reward);
        if new_total > TOTAL_SUPPLY {
            let over = new_total - TOTAL_SUPPLY;
            return Err(over);
        }
        self.total_minted = new_total;
        Ok(())
    }

    /// Check if a coinbase reward would exceed the supply cap
    ///
    /// # Arguments
    /// - `reward`: The proposed coinbase reward amount
    ///
    /// # Returns
    /// `true` if the reward can be applied without exceeding TOTAL_SUPPLY
    pub fn can_mint(&self, reward: Amount) -> bool {
        self.total_minted.saturating_add(reward) <= TOTAL_SUPPLY
    }

    /// Get the remaining mintable supply
    ///
    /// Returns how many coins can still be minted before reaching TOTAL_SUPPLY.
    pub fn remaining_supply(&self) -> Amount {
        TOTAL_SUPPLY.saturating_sub(self.total_minted)
    }

    /// Calculate the effective block reward given remaining supply
    ///
    /// If the calculated reward would exceed remaining supply, returns
    /// only the remaining supply (partial reward for the final era).
    ///
    /// # Arguments
    /// - `calculated_reward`: The reward calculated from halving schedule
    ///
    /// # Returns
    /// The actual reward to issue (may be less than calculated if near cap)
    pub fn effective_reward(&self, calculated_reward: Amount) -> Amount {
        let remaining = self.remaining_supply();
        calculated_reward.min(remaining)
    }

    /// Get total minted coins
    pub fn total_minted(&self) -> Amount {
        self.total_minted
    }

    // ==================== Epoch State Management ====================

    /// Record a block produced by a producer in the current epoch
    pub fn record_epoch_block(&mut self, producer_pubkey: &str, reward: u64) {
        *self.epoch_producer_blocks.entry(producer_pubkey.to_string()).or_insert(0) += 1;
        self.epoch_reward_pool += reward;
    }

    /// Reset epoch state for a new epoch
    pub fn start_new_epoch(&mut self, new_epoch: u64, start_height: u64) {
        self.current_reward_epoch = new_epoch;
        self.epoch_start_height = start_height;
        self.epoch_producer_blocks.clear();
        self.epoch_reward_pool = 0;
    }

    /// Get epoch producer blocks map
    pub fn epoch_producer_blocks(&self) -> &HashMap<String, u64> {
        &self.epoch_producer_blocks
    }

    /// Get current epoch reward pool
    pub fn epoch_reward_pool(&self) -> u64 {
        self.epoch_reward_pool
    }

    /// Get current epoch number
    pub fn current_reward_epoch(&self) -> u64 {
        self.current_reward_epoch
    }

    /// Get epoch start height
    pub fn epoch_start_height(&self) -> u64 {
        self.epoch_start_height
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chain_state_new() {
        let genesis = Hash::zero();
        let state = ChainState::new(genesis);

        assert!(state.is_genesis());
        assert_eq!(state.best_height, 0);
        assert_eq!(state.best_hash, genesis);
        // Registration chain starts empty
        assert_eq!(state.last_registration_hash, Hash::ZERO);
        assert_eq!(state.registration_sequence, 0);
        // Supply tracking starts at 0
        assert_eq!(state.total_minted, 0);
    }

    #[test]
    fn test_chain_state_update() {
        let genesis = Hash::zero();
        let mut state = ChainState::new(genesis);

        let new_hash = crypto::hash::hash(b"block1");
        state.update(new_hash, 1, 1);

        assert!(!state.is_genesis());
        assert_eq!(state.best_height, 1);
        assert_eq!(state.best_hash, new_hash);
    }

    #[test]
    fn test_registration_chain_first_registration() {
        let genesis = Hash::zero();
        let state = ChainState::new(genesis);

        // First registration expects ZERO hash and sequence 0
        assert_eq!(state.expected_prev_registration_hash(), Hash::ZERO);
        assert_eq!(state.expected_registration_sequence(), 0);

        // Verify first registration chain data
        assert!(state.verify_registration_chain(Hash::ZERO, 0));
        assert!(!state.verify_registration_chain(Hash::ZERO, 1)); // Wrong sequence
        assert!(!state.verify_registration_chain(crypto::hash::hash(b"wrong"), 0));
        // Wrong hash
    }

    #[test]
    fn test_registration_chain_subsequent_registrations() {
        let genesis = Hash::zero();
        let mut state = ChainState::new(genesis);

        // Record first registration
        let reg1_hash = crypto::hash::hash(b"registration1");
        state.record_registration(reg1_hash);

        assert_eq!(state.last_registration_hash, reg1_hash);
        assert_eq!(state.registration_sequence, 1);

        // Second registration expects reg1's hash and sequence 2
        assert_eq!(state.expected_prev_registration_hash(), reg1_hash);
        assert_eq!(state.expected_registration_sequence(), 2);

        // Verify second registration chain data
        assert!(state.verify_registration_chain(reg1_hash, 2));
        assert!(!state.verify_registration_chain(Hash::ZERO, 2)); // Wrong hash
        assert!(!state.verify_registration_chain(reg1_hash, 1)); // Wrong sequence

        // Record second registration
        let reg2_hash = crypto::hash::hash(b"registration2");
        state.record_registration(reg2_hash);

        assert_eq!(state.last_registration_hash, reg2_hash);
        assert_eq!(state.registration_sequence, 2);

        // Third registration expects reg2's hash and sequence 3
        assert_eq!(state.expected_prev_registration_hash(), reg2_hash);
        assert_eq!(state.expected_registration_sequence(), 3);
    }

    #[test]
    fn test_registration_chain_prevents_parallel_attack() {
        let genesis = Hash::zero();
        let mut state = ChainState::new(genesis);

        // Attacker tries to register two nodes "simultaneously"
        // Both claim to be the first registration
        let attacker_reg1 = crypto::hash::hash(b"attacker1");
        let attacker_reg2 = crypto::hash::hash(b"attacker2");

        // First registration succeeds
        assert!(state.verify_registration_chain(Hash::ZERO, 0));
        state.record_registration(attacker_reg1);

        // Second "parallel" registration fails - it still references ZERO
        // but the chain has moved forward
        assert!(!state.verify_registration_chain(Hash::ZERO, 0));

        // The attacker would need to reference the first registration
        assert!(state.verify_registration_chain(attacker_reg1, 2));
    }

    // ==================== Supply Tracking Tests ====================

    #[test]
    fn test_apply_coinbase() {
        let genesis = Hash::zero();
        let mut state = ChainState::new(genesis);

        // Apply first coinbase
        assert!(state.apply_coinbase(100_000_000).is_ok());
        assert_eq!(state.total_minted, 100_000_000);

        // Apply more coinbases
        assert!(state.apply_coinbase(100_000_000).is_ok());
        assert_eq!(state.total_minted, 200_000_000);
    }

    #[test]
    fn test_apply_coinbase_enforces_supply_cap() {
        let genesis = Hash::zero();
        let mut state = ChainState::new(genesis);

        // Mint almost all supply
        state.total_minted = TOTAL_SUPPLY - 100;

        // Can mint remaining 100
        assert!(state.can_mint(100));
        assert!(state.apply_coinbase(100).is_ok());
        assert_eq!(state.total_minted, TOTAL_SUPPLY);

        // Cannot mint any more
        assert!(!state.can_mint(1));
        let result = state.apply_coinbase(1);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), 1); // Over by 1

        // Total minted unchanged
        assert_eq!(state.total_minted, TOTAL_SUPPLY);
    }

    #[test]
    fn test_remaining_supply() {
        let genesis = Hash::zero();
        let mut state = ChainState::new(genesis);

        // Initially all supply remains
        assert_eq!(state.remaining_supply(), TOTAL_SUPPLY);

        // After minting some
        state.total_minted = 1_000_000_000_000;
        assert_eq!(
            state.remaining_supply(),
            TOTAL_SUPPLY - 1_000_000_000_000
        );

        // At max supply
        state.total_minted = TOTAL_SUPPLY;
        assert_eq!(state.remaining_supply(), 0);
    }

    #[test]
    fn test_effective_reward() {
        let genesis = Hash::zero();
        let mut state = ChainState::new(genesis);

        let full_reward = 100_000_000;

        // Normal case: full reward available
        state.total_minted = 0;
        assert_eq!(state.effective_reward(full_reward), full_reward);

        // Partial reward: near cap
        state.total_minted = TOTAL_SUPPLY - 50_000_000;
        assert_eq!(state.effective_reward(full_reward), 50_000_000);

        // No reward: at cap
        state.total_minted = TOTAL_SUPPLY;
        assert_eq!(state.effective_reward(full_reward), 0);
    }

    #[test]
    fn test_supply_never_exceeds_cap() {
        let genesis = Hash::zero();
        let mut state = ChainState::new(genesis);

        // Simulate mining many blocks
        let reward = 100_000_000;
        let mut blocks_mined = 0;

        while state.can_mint(1) {
            let effective = state.effective_reward(reward);
            if effective == 0 {
                break;
            }
            assert!(state.apply_coinbase(effective).is_ok());
            blocks_mined += 1;

            // Safety limit for test
            if blocks_mined > TOTAL_SUPPLY / reward + 10 {
                panic!("Too many iterations");
            }
        }

        // Should never exceed cap
        assert!(state.total_minted <= TOTAL_SUPPLY);
    }
}
