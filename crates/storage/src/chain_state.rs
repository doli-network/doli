//! Chain state management

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
    /// Height at which snap sync last completed, if any.
    ///
    /// `Some(h)` means this chain state was produced by snap sync at height `h`
    /// and the block store is intentionally empty below that height.
    /// `None` means the chain state was built from full block replay.
    ///
    /// NOT included in `serialize_canonical()` — local bookkeeping only,
    /// not part of the consensus state root.
    #[serde(default)]
    pub snap_sync_height: Option<u64>,
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
            snap_sync_height: None,
        }
    }

    /// Load from disk
    pub fn load(path: &Path) -> Result<Self, StorageError> {
        let bytes = std::fs::read(path)?;
        bincode::deserialize(&bytes).map_err(|e| StorageError::Serialization(e.to_string()))
    }

    /// Save to disk (atomic: write to temp file, then rename)
    pub fn save(&self, path: &Path) -> Result<(), StorageError> {
        let bytes =
            bincode::serialize(self).map_err(|e| StorageError::Serialization(e.to_string()))?;
        let tmp = path.with_extension("bin.tmp");
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Update with a new best block
    pub fn update(&mut self, hash: Hash, height: u64, slot: u32) {
        self.best_hash = hash;
        self.best_height = height;
        self.best_slot = slot;
        // In PoT every block adds exactly 1 unit of work, so total_work == height.
        // Using assignment (not +=1) prevents divergence between nodes that started
        // from different chain_state.bin snapshots.
        self.total_work = height;
    }

    /// Serialize ChainState to canonical bytes for state root computation.
    ///
    /// Uses a fixed-field, version-independent encoding immune to bincode
    /// struct evolution. Never changes regardless of future field additions.
    ///
    /// Format (140 bytes):
    /// `[32B best_hash][8B best_height][4B best_slot][8B total_work]`
    /// `[32B genesis_hash][8B genesis_timestamp][32B last_registration_hash]`
    /// `[8B registration_sequence][8B total_minted]`
    pub fn serialize_canonical(&self) -> [u8; 140] {
        let mut buf = [0u8; 140];
        buf[0..32].copy_from_slice(self.best_hash.as_bytes());
        buf[32..40].copy_from_slice(&self.best_height.to_le_bytes());
        buf[40..44].copy_from_slice(&self.best_slot.to_le_bytes());
        buf[44..52].copy_from_slice(&self.total_work.to_le_bytes());
        buf[52..84].copy_from_slice(self.genesis_hash.as_bytes());
        buf[84..92].copy_from_slice(&self.genesis_timestamp.to_le_bytes());
        buf[92..124].copy_from_slice(self.last_registration_hash.as_bytes());
        buf[124..132].copy_from_slice(&self.registration_sequence.to_le_bytes());
        buf[132..140].copy_from_slice(&self.total_minted.to_le_bytes());
        buf
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
}

// ==================== Snap Sync State ====================
//
// `snap_sync_height` is persisted so the startup integrity check can
// distinguish "block store empty because snap sync, which is correct"
// from "block store empty because corruption, which is wrong".
// Cleared on the first successful `apply_block` after snap sync.

impl ChainState {
    /// Mark this chain state as having been snap-synced to the given height.
    /// Persists so the next restart knows not to reset to genesis.
    pub fn mark_snap_synced(&mut self, height: u64) {
        self.snap_sync_height = Some(height);
    }

    /// Clear the snap sync marker after the first post-snap-sync block is applied.
    pub fn clear_snap_sync(&mut self) {
        self.snap_sync_height = None;
    }

    /// Whether this chain state was loaded from a snap sync (block store may be empty).
    pub fn is_snap_synced(&self) -> bool {
        self.snap_sync_height.is_some()
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
        assert_eq!(state.remaining_supply(), TOTAL_SUPPLY - 1_000_000_000_000);

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

    // ==================== serialize_canonical Tests ====================

    #[test]
    fn test_serialize_canonical_fixed_size() {
        let state = ChainState::new(Hash::zero());
        let bytes = state.serialize_canonical();
        assert_eq!(bytes.len(), 140, "canonical bytes must be exactly 140");
    }

    #[test]
    fn test_serialize_canonical_deterministic() {
        let mut state = ChainState::new(crypto::hash::hash(b"genesis"));
        state.update(crypto::hash::hash(b"block42"), 42, 100);
        state.total_minted = 999_000_000;
        state.genesis_timestamp = 1_700_000_000;

        let b1 = state.serialize_canonical();
        let b2 = state.serialize_canonical();
        assert_eq!(b1, b2, "canonical serialization must be deterministic");
    }

    #[test]
    fn test_serialize_canonical_different_states_differ() {
        let s1 = ChainState::new(Hash::zero());
        let mut s2 = ChainState::new(Hash::zero());
        s2.update(crypto::hash::hash(b"block1"), 1, 1);

        assert_ne!(
            s1.serialize_canonical(),
            s2.serialize_canonical(),
            "different state must produce different canonical bytes"
        );
    }

    #[test]
    fn test_serialize_canonical_total_work_equals_height() {
        let mut state = ChainState::new(Hash::zero());
        // Simulate a node that was restarted at height 1000 and then processed 50 more blocks
        // Old bug: total_work would be 50 (counted from restart)
        // Fix: total_work = height always
        state.update(crypto::hash::hash(b"block1050"), 1050, 2100);
        assert_eq!(state.total_work, 1050, "total_work must equal height");

        // Two nodes with the same best block but different restart histories
        // must produce identical canonical bytes
        let mut node_a = ChainState::new(Hash::zero());
        let mut node_b = ChainState::new(Hash::zero());
        let block_hash = crypto::hash::hash(b"block5000");
        node_a.update(block_hash, 5000, 10000);
        node_b.total_work = 999; // simulate stale value from old code
        node_b.update(block_hash, 5000, 10000); // update overwrites with = height
        assert_eq!(node_a.total_work, node_b.total_work);
        assert_eq!(node_a.serialize_canonical(), node_b.serialize_canonical());
    }

    #[test]
    fn test_serialize_canonical_field_layout() {
        let best_hash = crypto::hash::hash(b"best");
        let genesis_hash = crypto::hash::hash(b"genesis");
        let last_reg = crypto::hash::hash(b"reg");
        let mut state = ChainState::new(genesis_hash);
        state.best_hash = best_hash;
        state.best_height = 0x0102030405060708u64;
        state.best_slot = 0x090A0B0Cu32;
        state.total_work = 0x0102030405060708u64; // equals height after fix
        state.genesis_timestamp = 0x1122334455667788u64;
        state.last_registration_hash = last_reg;
        state.registration_sequence = 0xAABBCCDDEEFF0011u64;
        state.total_minted = 0x1234567890ABCDEFu64;

        let bytes = state.serialize_canonical();
        // best_hash at [0..32]
        assert_eq!(&bytes[0..32], best_hash.as_bytes());
        // best_height at [32..40] LE
        assert_eq!(&bytes[32..40], &0x0102030405060708u64.to_le_bytes());
        // best_slot at [40..44] LE
        assert_eq!(&bytes[40..44], &0x090A0B0Cu32.to_le_bytes());
        // total_work at [44..52] LE
        assert_eq!(&bytes[44..52], &0x0102030405060708u64.to_le_bytes());
        // genesis_hash at [52..84]
        assert_eq!(&bytes[52..84], genesis_hash.as_bytes());
        // genesis_timestamp at [84..92] LE
        assert_eq!(&bytes[84..92], &0x1122334455667788u64.to_le_bytes());
        // last_registration_hash at [92..124]
        assert_eq!(&bytes[92..124], last_reg.as_bytes());
        // registration_sequence at [124..132] LE
        assert_eq!(&bytes[124..132], &0xAABBCCDDEEFF0011u64.to_le_bytes());
        // total_minted at [132..140] LE
        assert_eq!(&bytes[132..140], &0x1234567890ABCDEFu64.to_le_bytes());
    }
}
