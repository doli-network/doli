//! Timing and time-acceleration parameters for DOLI networks
//!
//! Slot duration, epochs, seniority, bootstrap, and simulated time scaling.

use super::Network;

impl Network {
    /// Get slot duration for this network (in seconds)
    ///
    /// Configurable via `DOLI_SLOT_DURATION` environment variable (devnet only).
    /// Locked for mainnet to ensure consensus compatibility.
    pub fn slot_duration(&self) -> u64 {
        self.params().slot_duration
    }

    /// Get bootstrap blocks count
    ///
    /// Configurable via `DOLI_BOOTSTRAP_BLOCKS` environment variable (devnet only).
    pub fn bootstrap_blocks(&self) -> u64 {
        self.params().bootstrap_blocks
    }

    /// Get bootstrap grace period in seconds
    ///
    /// This is the wait time at genesis before allowing block production.
    /// Configurable via `DOLI_BOOTSTRAP_GRACE_PERIOD_SECS` environment variable (devnet only).
    /// Locked for mainnet (15 seconds).
    pub fn bootstrap_grace_period_secs(&self) -> u64 {
        self.params().bootstrap_grace_period_secs
    }

    /// Get slots per reward epoch for this network
    /// Reward epochs determine when accumulated rewards are distributed equally
    ///
    /// Configurable via `DOLI_SLOTS_PER_REWARD_EPOCH` environment variable (devnet only).
    pub fn slots_per_reward_epoch(&self) -> u32 {
        self.params().slots_per_reward_epoch
    }

    /// Get blocks per reward epoch for this network (block-height based epochs).
    ///
    /// This is the primary constant for the weighted presence reward system.
    /// Block-height based epochs are simpler than slot-based epochs because
    /// block heights are sequential with no gaps.
    ///
    /// Configurable via `DOLI_BLOCKS_PER_REWARD_EPOCH` environment variable (devnet only).
    /// Locked for mainnet to ensure consensus compatibility.
    pub fn blocks_per_reward_epoch(&self) -> u64 {
        self.params().blocks_per_reward_epoch
    }

    /// Get default bootstrap nodes for this network
    ///
    /// Configurable via `DOLI_BOOTSTRAP_NODES` environment variable (comma-separated).
    pub fn bootstrap_nodes(&self) -> Vec<String> {
        self.params().bootstrap_nodes.clone()
    }

    // ==================== Time Acceleration Parameters ====================
    //
    // These parameters enable accelerated time simulation for testing:
    // - Mainnet: Real time (1 block/minute, 4 years per era)
    // - Testnet: 1 real day = 1 simulated year
    // - Devnet:  1 real minute = 1 simulated year (20 min test = 20 years)

    /// Blocks per "year" (simulated)
    ///
    /// - Mainnet: 3,153,600 blocks (~365.25 days at 6 blocks/minute)
    /// - Testnet: 3,153,600 blocks (same as mainnet)
    /// - Devnet:  144 blocks (2.4 minutes = 1 simulated year, era ≈ 10 minutes)
    ///
    /// Configurable via `DOLI_BLOCKS_PER_YEAR` environment variable (devnet only).
    /// Locked for mainnet to ensure consensus compatibility.
    pub fn blocks_per_year(&self) -> u64 {
        self.params().blocks_per_year
    }

    /// Blocks per "month" (simulated)
    pub fn blocks_per_month(&self) -> u64 {
        self.params().blocks_per_month()
    }

    /// Blocks per era (4 simulated "years")
    pub fn blocks_per_era(&self) -> u64 {
        self.params().blocks_per_era()
    }

    /// Commitment period (4 years) in blocks
    ///
    /// Producers who complete this period get their full bond back.
    pub fn commitment_period(&self) -> u64 {
        self.params().commitment_period()
    }

    /// Exit history retention period (8 years) in blocks
    ///
    /// After this period, exit records expire and producers can re-register
    /// without the prior_exit penalty.
    pub fn exit_history_retention(&self) -> u64 {
        self.params().exit_history_retention()
    }

    /// Inactivity threshold in blocks
    ///
    /// After this many blocks without activity, a producer incurs an
    /// activity gap penalty.
    ///
    /// Configurable via `DOLI_INACTIVITY_THRESHOLD` environment variable (devnet only).
    pub fn inactivity_threshold(&self) -> u64 {
        self.params().inactivity_threshold
    }

    /// Unbonding period in blocks
    ///
    /// After requesting exit, producers must wait this long before
    /// claiming their bond. (7 days)
    ///
    /// Configurable via `DOLI_UNBONDING_PERIOD` environment variable (devnet only).
    /// Locked for mainnet to ensure consensus compatibility.
    pub fn unbonding_period(&self) -> u64 {
        self.params().unbonding_period
    }

    /// Get blocks needed to reach full seniority (maximum vote weight)
    ///
    /// After this many blocks as a producer, the vote weight reaches 4x.
    /// Uses blocks_per_year * 4 for real time networks.
    pub fn seniority_maturity_blocks(&self) -> u64 {
        self.params().seniority_maturity_blocks()
    }

    /// Get blocks per seniority step (for vote weight calculation)
    ///
    /// Each step increases vote weight by 0.75x, up to 4 steps (4x total).
    /// - 0 steps (0-1 year): 1.00x
    /// - 1 step  (1-2 year): 1.75x
    /// - 2 steps (2-3 year): 2.50x
    /// - 3 steps (3-4 year): 3.25x
    /// - 4 steps (4+ years): 4.00x
    pub fn seniority_step_blocks(&self) -> u64 {
        self.params().seniority_step_blocks()
    }
}
