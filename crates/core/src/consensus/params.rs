use crate::network::Network;
use crate::types::{Amount, BlockHeight, Epoch, Era, Slot};

use super::constants::*;
use super::exit::RewardMode;

/// Consensus parameters (can be adjusted for testnets)
#[derive(Clone, Debug)]
pub struct ConsensusParams {
    pub genesis_time: u64,
    pub slot_duration: u64,
    pub slots_per_epoch: u32,
    pub slots_per_reward_epoch: u32,
    pub attestation_interval: u32,
    pub min_attestation_rate: u32,
    pub blocks_per_era: BlockHeight,
    pub bootstrap_blocks: BlockHeight,
    /// Bootstrap grace period in seconds (wait at genesis for chain evidence)
    pub bootstrap_grace_period_secs: u64,
    pub initial_reward: Amount,
    pub initial_bond: Amount,
    pub base_block_size: usize,
    pub max_block_size_cap: usize,
    /// Reward distribution mode (DirectCoinbase or EpochPool)
    pub reward_mode: RewardMode,
    /// Genesis hash — chain identity fingerprint. Blocks with a different
    /// genesis_hash are rejected immediately. Set from chainspec at startup.
    pub genesis_hash: crypto::Hash,
}

impl ConsensusParams {
    /// Create mainnet parameters
    #[allow(deprecated)]
    pub fn mainnet() -> Self {
        // Compute genesis_hash from the embedded mainnet chainspec
        let spec = crate::chainspec::ChainSpec::mainnet();
        Self {
            genesis_time: GENESIS_TIME,
            slot_duration: SLOT_DURATION,
            slots_per_epoch: SLOTS_PER_EPOCH,
            slots_per_reward_epoch: SLOTS_PER_REWARD_EPOCH,
            attestation_interval: ATTESTATION_INTERVAL,
            min_attestation_rate: MIN_ATTESTATION_RATE,
            blocks_per_era: BLOCKS_PER_ERA,
            bootstrap_blocks: BOOTSTRAP_BLOCKS,
            bootstrap_grace_period_secs: BOOTSTRAP_GRACE_PERIOD_SECS,
            initial_reward: INITIAL_REWARD,
            initial_bond: INITIAL_BOND,
            base_block_size: BASE_BLOCK_SIZE,
            max_block_size_cap: MAX_BLOCK_SIZE_CAP,
            reward_mode: RewardMode::EpochPool,
            genesis_hash: spec.genesis_hash(),
        }
    }

    /// Create testnet parameters (identical to mainnet)
    ///
    /// # Proof of Time on Testnet
    ///
    /// Testnet uses EXACTLY the same parameters as mainnet to ensure
    /// realistic testing of the Proof of Time consensus with VDF.
    #[allow(deprecated)]
    pub fn testnet() -> Self {
        let spec = crate::chainspec::ChainSpec::testnet();
        Self {
            genesis_time: 0,                  // Will be set at testnet launch
            slot_duration: SLOT_DURATION,     // Same as mainnet (10 seconds)
            slots_per_epoch: SLOTS_PER_EPOCH, // Same as mainnet (360)
            slots_per_reward_epoch: 36,       // Testnet: ~6 min per epoch (10x faster)
            attestation_interval: ATTESTATION_INTERVAL,
            min_attestation_rate: MIN_ATTESTATION_RATE,
            blocks_per_era: BLOCKS_PER_ERA, // Same as mainnet (~4 years)
            bootstrap_blocks: BOOTSTRAP_BLOCKS, // Same as mainnet (~1 week)
            bootstrap_grace_period_secs: BOOTSTRAP_GRACE_PERIOD_SECS, // Same as mainnet (15s)
            initial_reward: INITIAL_REWARD,
            initial_bond: spec.consensus.bond_amount, // Use chainspec (1 DOLI for testnet)
            base_block_size: BASE_BLOCK_SIZE,
            max_block_size_cap: MAX_BLOCK_SIZE_CAP,
            reward_mode: RewardMode::EpochPool,
            genesis_hash: spec.genesis_hash(),
        }
    }

    /// Create devnet parameters (fast slots and epochs for local development)
    ///
    /// # Devnet Time Acceleration
    ///
    /// - Slot duration: 1 second (fast block production)
    /// - Era duration: ≈10 minutes (576 blocks = 9.6 minutes)
    /// - Year simulation: 2.4 minutes (144 blocks)
    /// - Reward epoch: 30 seconds (30 blocks)
    ///
    /// This allows testing the full tokenomics lifecycle in ~1 hour (6 eras).
    pub fn devnet() -> Self {
        let spec = crate::chainspec::ChainSpec::devnet();
        Self {
            genesis_time: 0,            // Will be set at devnet start
            slot_duration: 1,           // 1 second (fast)
            slots_per_epoch: 60,        // 1 minute per epoch
            slots_per_reward_epoch: 30, // 30 seconds per reward epoch
            attestation_interval: 1,    // Every block (presence signatures)
            min_attestation_rate: MIN_ATTESTATION_RATE,
            blocks_per_era: 576, // ≈10 minutes per era (576 blocks × 1s = 9.6 min)
            bootstrap_blocks: 60, // ~1 minute bootstrap
            bootstrap_grace_period_secs: 5, // 5s for fast devnet startup
            initial_reward: INITIAL_REWARD, // 1 DOLI per block (same as mainnet)
            initial_bond: 100_000_000, // 1 DOLI
            base_block_size: BASE_BLOCK_SIZE,
            max_block_size_cap: MAX_BLOCK_SIZE_CAP,
            reward_mode: RewardMode::EpochPool,
            genesis_hash: spec.genesis_hash(),
        }
    }

    /// Create consensus parameters for a specific network
    #[allow(deprecated)]
    pub fn for_network(network: Network) -> Self {
        match network {
            Network::Mainnet => Self::mainnet(),
            Network::Testnet => Self::testnet(),
            Network::Devnet => Self::devnet(),
        }
    }

    /// Apply chainspec overrides directly to these params.
    ///
    /// This makes `--chainspec` authoritative for consensus parameters,
    /// bypassing the OnceLock/env pipeline. Mainnet params are locked.
    pub fn apply_chainspec(&mut self, spec: &crate::chainspec::ChainSpec) {
        if matches!(spec.network, Network::Mainnet) {
            return; // Mainnet params are LOCKED
        }
        self.slot_duration = spec.consensus.slot_duration;
        self.slots_per_reward_epoch = spec.consensus.slots_per_epoch;
        self.initial_bond = spec.consensus.bond_amount;
        self.initial_reward = spec.genesis.initial_reward;
        if spec.genesis.timestamp != 0 {
            self.genesis_time = spec.genesis.timestamp;
        }
        self.genesis_hash = spec.genesis_hash();
    }

    /// Returns the activation height for covenant (conditioned) outputs.
    ///
    /// Before this height, nodes reject any OutputType >= 2 (Multisig, Hashlock, etc.).
    /// This ensures coordinated activation: all nodes must upgrade before covenants activate.
    pub fn covenants_activation_height(&self, network: &Network) -> BlockHeight {
        match network {
            Network::Devnet => 0,     // Immediate on devnet
            Network::Testnet => 6900, // Covenants activate at height 6900
            Network::Mainnet => 9150, // Activated after v3.4.0 testnet validation
        }
    }

    /// Calculate max block size for a given height.
    ///
    /// Block size doubles every era until reaching the cap.
    #[must_use]
    pub fn max_block_size(&self, height: BlockHeight) -> usize {
        let era = self.height_to_era(height);
        if era >= 5 {
            self.max_block_size_cap
        } else {
            self.base_block_size << era
        }
    }

    /// Calculate slot from timestamp.
    ///
    /// Returns 0 for timestamps before genesis, and caps the result
    /// at `Slot::MAX` to prevent overflow.
    pub fn timestamp_to_slot(&self, timestamp: u64) -> Slot {
        if timestamp < self.genesis_time {
            return 0;
        }
        let elapsed = timestamp - self.genesis_time;
        let slot_u64 = elapsed / self.slot_duration;
        // Cap at Slot::MAX to prevent overflow when casting
        if slot_u64 > u64::from(Slot::MAX) {
            Slot::MAX
        } else {
            slot_u64 as Slot
        }
    }

    /// Calculate timestamp from slot start
    pub fn slot_to_timestamp(&self, slot: Slot) -> u64 {
        self.genesis_time + (slot as u64 * self.slot_duration)
    }

    /// Calculate epoch from slot
    pub fn slot_to_epoch(&self, slot: Slot) -> Epoch {
        slot / self.slots_per_epoch
    }

    /// Calculate era from block height.
    ///
    /// Caps at `Era::MAX` to prevent overflow.
    pub fn height_to_era(&self, height: BlockHeight) -> Era {
        let era_u64 = height / self.blocks_per_era;
        if era_u64 > u64::from(Era::MAX) {
            Era::MAX
        } else {
            era_u64 as Era
        }
    }

    /// Calculate block reward for a given height.
    ///
    /// The reward halves each era. Returns 0 after era 63 (shift would overflow).
    pub fn block_reward(&self, height: BlockHeight) -> Amount {
        let era = self.height_to_era(height);
        // After era 63, reward becomes 0 (shift by >= 64 bits is undefined)
        if era >= 64 {
            return 0;
        }
        self.initial_reward >> era
    }

    /// Calculate bond amount for a given height.
    ///
    /// The bond decreases by 30% each era (multiplied by 0.7).
    /// Uses integer arithmetic to avoid floating-point precision issues.
    /// Formula: bond = initial_bond * 7^era / 10^era
    pub fn bond_amount(&self, height: BlockHeight) -> Amount {
        let era = self.height_to_era(height);

        // After era 20, the bond is effectively 0 (< 1 base unit)
        if era >= 20 {
            return 0;
        }

        // Use u128 arithmetic to avoid overflow during computation.
        // initial_bond * 7^era can exceed u64 for era >= 10.
        let mut numerator: u128 = self.initial_bond as u128;
        let mut denominator: u128 = 1;

        for _ in 0..era {
            // Multiply by 7/10 each era
            numerator *= 7;
            denominator *= 10;
        }

        // Perform the division (rounding down) and convert back to Amount
        (numerator / denominator) as Amount
    }

    /// Check if height is in bootstrap phase
    pub fn is_bootstrap(&self, height: BlockHeight) -> bool {
        height < self.bootstrap_blocks
    }

    // ==================== Reward Epoch Methods ====================

    /// Calculate reward epoch from slot.
    ///
    /// Reward epochs are separate from consensus epochs and are used
    /// for determining when to distribute accumulated rewards.
    pub fn slot_to_reward_epoch(&self, slot: Slot) -> Epoch {
        slot / self.slots_per_reward_epoch
    }

    /// Check if a slot is the first slot of a new reward epoch.
    ///
    /// Useful for producer set updates and statistics.
    pub fn is_reward_epoch_boundary(&self, slot: Slot) -> bool {
        slot > 0 && slot.is_multiple_of(self.slots_per_reward_epoch)
    }

    /// Get the starting slot of a reward epoch.
    pub fn reward_epoch_start_slot(&self, reward_epoch: Epoch) -> Slot {
        reward_epoch * self.slots_per_reward_epoch
    }

    /// Get the ending slot (exclusive) of a reward epoch.
    pub fn reward_epoch_end_slot(&self, reward_epoch: Epoch) -> Slot {
        (reward_epoch + 1) * self.slots_per_reward_epoch
    }

    // Legacy attestation methods - kept for API compatibility but unused in PoT

    #[doc(hidden)]
    #[deprecated(note = "Attestations not used in Proof of Time")]
    pub fn attestations_per_reward_epoch(&self) -> u32 {
        self.slots_per_reward_epoch / self.attestation_interval
    }

    #[doc(hidden)]
    #[deprecated(note = "Attestations not used in Proof of Time")]
    pub fn min_attestations_required(&self) -> u32 {
        #[allow(deprecated)]
        let total = self.attestations_per_reward_epoch();
        (total * self.min_attestation_rate) / 100
    }

    #[doc(hidden)]
    #[deprecated(note = "Attestations not used in Proof of Time")]
    pub fn is_attestation_required(&self, _height: BlockHeight) -> bool {
        false // No attestations in PoT - VDF is used instead
    }

    #[doc(hidden)]
    #[deprecated(note = "Rewards go directly to producer in PoT")]
    pub fn calculate_epoch_reward_share(&self, total_reward: Amount, active_count: u32) -> Amount {
        if active_count == 0 {
            return 0;
        }
        total_reward / (active_count as Amount)
    }

    /// Calculate total rewards produced in an epoch (for statistics).
    ///
    /// In Proof of Time, each block's reward goes to its producer.
    /// This calculates the total rewards for all blocks in an epoch.
    pub fn total_epoch_reward(&self, height: BlockHeight) -> Amount {
        let block_reward = self.block_reward(height);
        block_reward.saturating_mul(self.slots_per_reward_epoch as u64)
    }
}

impl Default for ConsensusParams {
    fn default() -> Self {
        Self::mainnet()
    }
}
