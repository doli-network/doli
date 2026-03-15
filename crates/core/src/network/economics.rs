//! Economic parameters for DOLI networks
//!
//! Bond lifecycle, genesis phase, rewards, registration fees, and coinbase maturity.

use super::Network;

impl Network {
    /// Get genesis timestamp for this network
    ///
    /// Configurable via `DOLI_GENESIS_TIME` environment variable (devnet only).
    /// Locked for mainnet/testnet to ensure consensus compatibility.
    pub fn genesis_time(&self) -> u64 {
        self.params().genesis_time
    }

    /// Get initial bond amount for this network
    ///
    /// Initial bond = 1 bond unit = minimum to become a producer.
    /// For mainnet/testnet: 100 DOLI (1 slot per cycle)
    pub fn initial_bond(&self) -> u64 {
        self.bond_unit() // Initial bond equals 1 bond unit
    }

    /// Get bond unit size for slot allocation
    ///
    /// Each bond unit grants 1 consecutive slot per cycle.
    /// - 100 DOLI = 1 bond unit = 1 slot per cycle
    /// - 1,000 DOLI = 10 bond units = 10 slots per cycle
    ///
    /// Configurable via `DOLI_BOND_UNIT` environment variable (devnet only).
    /// Locked for mainnet to ensure consensus compatibility.
    pub fn bond_unit(&self) -> u64 {
        self.params().bond_unit
    }

    /// Get initial block reward for this network
    ///
    /// Configurable via `DOLI_INITIAL_REWARD` environment variable (devnet only).
    /// Locked for mainnet to ensure emission schedule compatibility.
    pub fn initial_reward(&self) -> u64 {
        self.params().initial_reward
    }

    /// Get genesis phase duration in blocks
    ///
    /// During genesis, producers can produce blocks without a bond.
    /// After genesis ends, each participating producer is automatically
    /// registered with an automatic_genesis_bond().
    ///
    /// Configurable via `DOLI_GENESIS_BLOCKS` environment variable (devnet only).
    pub fn genesis_blocks(&self) -> u64 {
        self.params().genesis_blocks
    }

    /// Check if the given height is within the genesis phase
    pub fn is_in_genesis(&self, height: u64) -> bool {
        let genesis_blocks = self.genesis_blocks();
        genesis_blocks > 0 && height <= genesis_blocks
    }

    /// Get the automatic bond amount for genesis producers
    ///
    /// After genesis ends, each producer who participated gets this amount
    /// automatically bonded (deducted from their earned rewards).
    /// This equals 1 bond unit (100 DOLI on mainnet/testnet, 100 DOLI on devnet).
    ///
    /// Configurable via `DOLI_AUTOMATIC_GENESIS_BOND` environment variable (devnet only).
    pub fn automatic_genesis_bond(&self) -> u64 {
        self.params().automatic_genesis_bond
    }

    /// Get coinbase maturity (blocks until coinbase rewards can be spent)
    ///
    /// Like Bitcoin, coinbase rewards must wait for confirmations before spending.
    /// Devnet uses shorter maturity for faster testing.
    ///
    /// Configurable via `DOLI_COINBASE_MATURITY` environment variable (devnet only).
    pub fn coinbase_maturity(&self) -> u64 {
        self.params().coinbase_maturity
    }

    /// Maximum registrations allowed per block
    ///
    /// Limits registration throughput to prevent spam attacks.
    ///
    /// Configurable via `DOLI_MAX_REGISTRATIONS_PER_BLOCK` environment variable.
    pub fn max_registrations_per_block(&self) -> u32 {
        self.params().max_registrations_per_block
    }

    /// Base registration fee
    ///
    /// This is multiplied by 1.5^pending_count for anti-DoS escalation.
    ///
    /// Configurable via `DOLI_REGISTRATION_BASE_FEE` environment variable.
    pub fn registration_base_fee(&self) -> u64 {
        self.params().registration_base_fee
    }

    /// Maximum registration fee cap
    ///
    /// Prevents fees from becoming unreasonable during congestion.
    ///
    /// Configurable via `DOLI_MAX_REGISTRATION_FEE` environment variable.
    pub fn max_registration_fee(&self) -> u64 {
        self.params().max_registration_fee
    }
}
