//! Producer state management
//!
//! Tracks the state of block producers including their bond status,
//! registration height, exit/cooldown state, and seniority-based weight.
//!
//! ## Anti-Sybil: Weight by Seniority (Discrete Steps)
//!
//! A producer's power increases in discrete yearly steps based on their
//! time active in the network. This prevents whale attacks where an
//! attacker could instantly acquire significant influence.
//!
//! Weight by years active:
//! - 0-1 years: weight 1
//! - 1-2 years: weight 2
//! - 2-3 years: weight 3
//! - 3+ years: weight 4 (cap)
//!
//! This uses SLOTS_PER_YEAR (3,153,600 slots at 10s per slot) for
//! time calculation.
//!
//! Weight affects:
//! - Reward distribution (proportional to weight)
//! - Veto power (40% of total weight, not count)
//! - Producer selection probability (proportional to weight)

use std::collections::HashMap;
use std::path::Path;

use crypto::hash::hash as crypto_hash;
use crypto::{Hash, PublicKey};
use doli_core::consensus::MAX_BONDS_PER_PRODUCER;
use doli_core::network::Network;
use doli_core::network_params::NetworkParams;
use serde::{Deserialize, Serialize};

use crate::StorageError;

// ==================== Seniority Weight System ====================

/// Slots per year (mainnet default: 3,153,600 at 10s slots)
///
/// **Deprecated**: Use `NetworkParams::load(network).blocks_per_year` instead.
/// Devnet uses 144 blocks/year for accelerated testing.
#[deprecated(
    note = "Use NetworkParams::load(network).blocks_per_year for network-aware calculations"
)]
pub const SLOTS_PER_YEAR: u64 = 3_153_600;

/// Blocks per month - legacy alias (mainnet default)
///
/// **Deprecated**: Use `NetworkParams::load(network).blocks_per_year / 12` instead.
#[deprecated(
    note = "Use NetworkParams::load(network).blocks_per_year / 12 for network-aware calculations"
)]
pub const BLOCKS_PER_MONTH: u64 = 3_153_600 / 12;

/// Blocks per year - legacy alias (mainnet default)
///
/// **Deprecated**: Use `NetworkParams::load(network).blocks_per_year` instead.
/// Devnet uses 144 blocks/year for accelerated testing.
#[deprecated(
    note = "Use NetworkParams::load(network).blocks_per_year for network-aware calculations"
)]
pub const BLOCKS_PER_YEAR: u64 = 3_153_600;

/// Get blocks per year for a specific network
pub fn blocks_per_year_for_network(network: Network) -> u64 {
    NetworkParams::load(network).blocks_per_year
}

/// Get inactivity threshold for a specific network
pub fn inactivity_threshold_for_network(network: Network) -> u64 {
    NetworkParams::load(network).inactivity_threshold
}

/// Maximum weight a producer can achieve
pub const MAX_WEIGHT: u64 = 4;

/// Minimum weight for any producer
pub const MIN_WEIGHT: u64 = 1;

/// Veto threshold percentage (40% of effective weight required to block)
///
/// Why 40% instead of 33%:
/// - 33% allows a $44K early attacker to block governance for 4 years
/// - 40% raises the bar: requires more nodes or longer sustained attack
/// - Combined with activity penalty, makes "register and wait" attacks expensive
pub const VETO_THRESHOLD_PERCENT: u64 = 40;

/// Veto bond amount (in smallest units) required to vote BLOCK
///
/// This is a temporary bond locked when voting to block a proposal.
/// The bond is returned after the vote concludes (regardless of outcome).
/// This adds friction to frivolous or attack-driven vetoes.
///
/// 10 DOLI = 1_000_000_000 units (same as registration fee)
pub const VETO_BOND_AMOUNT: u64 = 1_000_000_000;

/// Activation delay in blocks before a new producer can participate in scheduling.
///
/// When a producer registers, they must wait this many blocks before they become
/// eligible for block production. This ensures all nodes have time to see the
/// registration transaction and update their producer sets, preventing scheduling
/// conflicts where some nodes see the new producer and others don't.
///
/// With 10-second slots:
/// - 10 blocks = ~100 seconds of propagation time
/// - All nodes will have received and confirmed the registration block
/// - ProducerSet becomes consistent across the network
pub const ACTIVATION_DELAY: u64 = 10;

/// Calculate producer weight based on seniority (mainnet defaults)
///
/// **Deprecated**: Use `producer_weight_for_network()` for network-aware calculations.
///
/// Weight is determined by years active:
/// - 0-1 years: weight 1
/// - 1-2 years: weight 2
/// - 2-3 years: weight 3
/// - 3+ years: weight 4 (cap)
#[deprecated(
    note = "Use producer_weight_for_network(registered_at, current_height, network) for network-aware calculations"
)]
pub fn producer_weight(registered_at: u64, current_height: u64) -> u64 {
    // Use mainnet default for backwards compatibility
    #[allow(deprecated)]
    let slots_per_year = SLOTS_PER_YEAR;
    let slots_active = current_height.saturating_sub(registered_at);
    let years_active = slots_active / slots_per_year;

    // Discrete yearly steps: 0-1=1, 1-2=2, 2-3=3, 3+=4
    match years_active {
        0 => 1,
        1 => 2,
        2 => 3,
        _ => 4, // 3+ years
    }
}

/// Calculate producer weight with decimal precision (for proportional calculations)
///
/// Same as `producer_weight` but returns f64 for API compatibility.
/// Uses discrete yearly steps.
#[allow(deprecated)]
pub fn producer_weight_precise(registered_at: u64, current_height: u64) -> f64 {
    producer_weight(registered_at, current_height) as f64
}

// ==================== Network-Aware Weight Functions ====================

/// Calculate producer weight for a specific network
///
/// Uses network-specific blocks_per_year for time acceleration on devnet/testnet.
pub fn producer_weight_for_network(
    registered_at: u64,
    current_height: u64,
    network: Network,
) -> u64 {
    let blocks_active = current_height.saturating_sub(registered_at);
    let blocks_per_year = network.blocks_per_year();
    let years_active = blocks_active / blocks_per_year;

    // Discrete yearly steps: 0-1=1, 1-2=2, 2-3=3, 3+=4
    match years_active {
        0 => 1,
        1 => 2,
        2 => 3,
        _ => 4,
    }
}

/// Calculate producer weight with decimal precision for a specific network
pub fn producer_weight_precise_for_network(
    registered_at: u64,
    current_height: u64,
    network: Network,
) -> f64 {
    producer_weight_for_network(registered_at, current_height, network) as f64
}

/// Calculate total weight of all active producers for a specific network
pub fn total_weight_for_network(
    producers: &[ProducerInfo],
    current_height: u64,
    network: Network,
) -> u64 {
    producers
        .iter()
        .filter(|p| p.is_active())
        .map(|p| producer_weight_for_network(p.registered_at, current_height, network))
        .sum()
}

/// Calculate the weighted veto threshold for a specific network
///
/// Returns the minimum total weight required for a veto to pass (40%).
pub fn weighted_veto_threshold_for_network(
    producers: &[ProducerInfo],
    current_height: u64,
    network: Network,
) -> u64 {
    let total = total_weight_for_network(producers, current_height, network);
    // 40% threshold, rounded up to be conservative
    (total * VETO_THRESHOLD_PERCENT).div_ceil(100)
}

/// Calculate total weight of all active producers
///
/// # Arguments
/// - `producers`: Slice of producer information
/// - `current_height`: Current block height
///
/// # Returns
/// Sum of all active producers' weights
#[allow(deprecated)]
pub fn total_weight(producers: &[ProducerInfo], current_height: u64) -> u64 {
    producers
        .iter()
        .filter(|p| p.is_active())
        .map(|p| producer_weight(p.registered_at, current_height))
        .sum()
}

/// Calculate the weighted veto threshold
///
/// Returns the minimum total weight required for a veto to pass.
/// This is 40% of the total weight of all active producers.
///
/// # Arguments
/// - `producers`: Slice of producer information
/// - `current_height`: Current block height
///
/// # Returns
/// The weight threshold needed for veto (40% of total weight)
#[allow(deprecated)]
pub fn weighted_veto_threshold(producers: &[ProducerInfo], current_height: u64) -> u64 {
    let total = total_weight(producers, current_height);
    // 40% threshold, rounded up to be conservative
    (total * VETO_THRESHOLD_PERCENT).div_ceil(100)
}

/// Producer status in the network
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProducerStatus {
    /// Producer is actively participating in block production
    Active,
    /// Producer has requested exit, in unbonding period (7 days)
    /// During this period, producer can still produce blocks
    Unbonding {
        /// Block height when unbonding started
        started_at: u64,
    },
    /// Producer has completed unbonding and can claim their bond
    Exited,
    /// Producer was slashed for misbehavior (100% bond burned)
    /// They are permanently excluded from the network
    Slashed {
        /// Block height when slashing occurred
        slashed_at: u64,
    },
}

/// Inactivity threshold (mainnet default): ~1 week without producing triggers inactive status
/// At 10s slots (6 blocks/minute): 7 days × 24 hours × 360 blocks/hour = 60,480 blocks
///
/// **Deprecated**: Use `inactivity_threshold_for_network(network)` for network-aware calculations.
/// Devnet uses 30 blocks for faster testing.
#[deprecated(note = "Use inactivity_threshold_for_network(network) for network-aware calculations")]
pub const INACTIVITY_THRESHOLD: u64 = 60_480;

/// Reactivation threshold (mainnet default): ~1 day of continuous activity to regain Active status
/// At 10s slots (6 blocks/minute): 1 day × 24 hours × 360 blocks/hour = 8,640 blocks
pub const REACTIVATION_THRESHOLD: u64 = 8_640;

// ==================== Activity Status System ====================

/// Activity status of a producer
///
/// Determines governance power and quorum participation.
/// **Principle: "El silencio no bloquea"** - only active producers
/// can block governance changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActivityStatus {
    /// Producing blocks regularly. Full governance power.
    /// - Can vote on proposals
    /// - Counts for quorum denominator
    /// - Receives rewards for blocks produced
    Active,

    /// Recently stopped producing (< 2 weeks). Grace period.
    /// - Cannot vote on proposals
    /// - Does NOT count for quorum
    /// - Can reactivate by producing blocks
    RecentlyInactive,

    /// Dormant for extended period (>= 2 weeks). No governance power.
    /// - Cannot vote on proposals
    /// - Does NOT count for quorum
    /// - Bond remains locked (auto-renew)
    /// - Can reactivate by producing blocks for 24h
    Dormant,
}

/// Information about a registered producer
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProducerInfo {
    /// Producer's public key
    pub public_key: PublicKey,
    /// Block height when registered
    pub registered_at: u64,
    /// Bond amount locked (total across all bonds)
    pub bond_amount: u64,
    /// Bond output location (tx_hash, output_index) - primary bond
    pub bond_outpoint: (Hash, u32),
    /// Current status
    pub status: ProducerStatus,
    /// DEPRECATED: Blocks produced counter (never updated by EpochPool rewards)
    ///
    /// This field is vestigial from the Pull/Claim reward model. The active
    /// EpochPool system calculates block counts from the BlockStore on-demand.
    /// Field retained for serialization backwards compatibility only.
    #[deprecated(
        note = "Block counts are calculated from BlockStore - this field is never updated"
    )]
    #[serde(default)]
    pub blocks_produced: u64,
    /// Number of missed slots
    pub slots_missed: u64,
    /// Era when registered (for renewal tracking)
    pub registration_era: u32,
    /// DEPRECATED: Pending rewards counter (never updated by EpochPool rewards)
    ///
    /// This field is vestigial from the Pull/Claim reward model. The active
    /// EpochPool system distributes rewards directly to UTXOs via EpochReward
    /// transactions. Field retained for serialization backwards compatibility only.
    #[deprecated(
        note = "Rewards go directly to UTXOs via EpochReward txs - this field is never updated"
    )]
    #[serde(default)]
    pub pending_rewards: u64,
    /// Whether this producer has previously exited (anti-Sybil: maturity cooldown)
    ///
    /// When true, this producer has previously exited the network and re-registered.
    /// They start fresh with weight 1, losing all prior seniority.
    /// This prevents "hit-and-run" attacks where a producer votes for something
    /// malicious and then exits before consequences.
    #[serde(default)]
    pub has_prior_exit: bool,
    /// Last block height where this producer showed activity (produced block or voted)
    ///
    /// Used for uptime tracking. Dormant nodes have reduced effective weight.
    #[serde(default)]
    pub last_activity: u64,
    /// DEPRECATED: Activity gaps are no longer tracked or penalized.
    /// Per alignment decision: No activity penalty. Producers who miss slots
    /// simply miss rewards - no weight reduction. Only slashable offense is
    /// double production (equivocation).
    /// Field retained for serialization backwards compatibility only.
    #[serde(default)]
    #[deprecated(
        note = "Activity gaps no longer affect consensus - field kept for backwards compatibility"
    )]
    pub activity_gaps: u64,
    /// Number of bonds staked (1-100)
    ///
    /// Bond Stacking System: More bonds = more chances to produce blocks.
    /// Each bond is 1,000 DOLI. Maximum 100 bonds per producer.
    /// In selection, producers are expanded by their bond_count for weighted lottery.
    /// Example: [Alice:1, Bob:5] → [Alice, Bob, Bob, Bob, Bob, Bob]
    /// This ensures ROI is proportional: 10 bonds cost 10x, earn ~10x.
    #[serde(default = "default_bond_count")]
    pub bond_count: u32,
    /// Additional bond outpoints (for bond stacking)
    /// First bond uses bond_outpoint, additional bonds stored here
    #[serde(default)]
    pub additional_bonds: Vec<(Hash, u32)>,
    /// Who this producer delegates bond weight to (Tier 3 → Tier 1/2)
    #[serde(default)]
    pub delegated_to: Option<PublicKey>,
    /// How many bonds are delegated
    #[serde(default)]
    pub delegated_bonds: u32,
    /// Delegations received from other producers: (delegator pubkey hash, bond count)
    #[serde(default)]
    pub received_delegations: Vec<(Hash, u32)>,
}

/// Default bond count for backwards compatibility
fn default_bond_count() -> u32 {
    1
}

/// Bond unit constant for mainnet/testnet: 1 bond = 100 DOLI = 10,000,000,000 base units
/// Note: For devnet, use Network::initial_bond() which returns 100_000_000 (1 DOLI per bond)
/// IMPORTANT: For production code, prefer using `NetworkParams::bond_unit()` instead.
pub const BOND_UNIT: u64 = 10_000_000_000;

/// Get the bond unit for a specific network
/// - Mainnet/Testnet: 100_000_000_000 (1000 DOLI per bond)
/// - Devnet: 100_000_000 (1 DOLI per bond)
pub fn bond_unit_for_network(network: Network) -> u64 {
    network.initial_bond()
}

impl ProducerInfo {
    /// Create a new active producer with a single bond
    ///
    /// # Arguments
    /// - `public_key`: The producer's public key
    /// - `registered_at`: Block height at registration
    /// - `bond_amount`: Total bond amount in base units
    /// - `bond_outpoint`: UTXO reference for the bond
    /// - `registration_era`: Era at registration
    /// - `bond_unit`: Amount per bond (use `bond_unit_for_network(network)`)
    #[allow(deprecated)]
    pub fn new(
        public_key: PublicKey,
        registered_at: u64,
        bond_amount: u64,
        bond_outpoint: (Hash, u32),
        registration_era: u32,
        bond_unit: u64,
    ) -> Self {
        // Calculate bond count from amount using network-specific bond unit
        // For devnet: bond_unit = 100_000_000 (1 DOLI per bond)
        // For mainnet/testnet: bond_unit = 100_000_000_000 (1000 DOLI per bond)
        let bond_unit = if bond_unit == 0 { BOND_UNIT } else { bond_unit };
        let bond_count = (bond_amount / bond_unit).min(MAX_BONDS_PER_PRODUCER as u64) as u32;
        let bond_count = bond_count.max(1); // Minimum 1 bond

        Self {
            public_key,
            registered_at,
            bond_amount,
            bond_outpoint,
            status: ProducerStatus::Active,
            blocks_produced: 0,
            slots_missed: 0,
            registration_era,
            pending_rewards: 0,
            has_prior_exit: false,
            last_activity: registered_at,
            activity_gaps: 0,
            bond_count,
            additional_bonds: Vec::new(),
            delegated_to: None,
            delegated_bonds: 0,
            received_delegations: Vec::new(),
        }
    }

    /// Create a new producer with specified bond count (for multi-bond registration)
    #[allow(deprecated)]
    pub fn new_with_bonds(
        public_key: PublicKey,
        registered_at: u64,
        bond_amount: u64,
        bond_outpoint: (Hash, u32),
        registration_era: u32,
        bond_count: u32,
    ) -> Self {
        let bond_count = bond_count.clamp(1, MAX_BONDS_PER_PRODUCER);

        Self {
            public_key,
            registered_at,
            bond_amount,
            bond_outpoint,
            status: ProducerStatus::Active,
            blocks_produced: 0,
            slots_missed: 0,
            registration_era,
            pending_rewards: 0,
            has_prior_exit: false,
            last_activity: registered_at,
            activity_gaps: 0,
            bond_count,
            additional_bonds: Vec::new(),
            delegated_to: None,
            delegated_bonds: 0,
            received_delegations: Vec::new(),
        }
    }

    /// Create a new producer with prior exit flag (for re-registration)
    ///
    /// Used when a producer who previously exited is re-registering.
    /// They start fresh with weight 1 (registered_at = current height).
    ///
    /// # Arguments
    /// * `bond_unit` - Amount per bond (use `NetworkParams::bond_unit()` or `bond_unit_for_network()`)
    #[allow(deprecated)]
    pub fn new_with_prior_exit(
        public_key: PublicKey,
        registered_at: u64,
        bond_amount: u64,
        bond_outpoint: (Hash, u32),
        registration_era: u32,
        bond_unit: u64,
    ) -> Self {
        let bond_unit = if bond_unit == 0 { BOND_UNIT } else { bond_unit };
        let bond_count = (bond_amount / bond_unit).min(MAX_BONDS_PER_PRODUCER as u64) as u32;
        let bond_count = bond_count.max(1);

        Self {
            public_key,
            registered_at,
            bond_amount,
            bond_outpoint,
            status: ProducerStatus::Active,
            blocks_produced: 0,
            slots_missed: 0,
            registration_era,
            pending_rewards: 0,
            has_prior_exit: true,
            last_activity: registered_at,
            activity_gaps: 0,
            bond_count,
            additional_bonds: Vec::new(),
            delegated_to: None,
            delegated_bonds: 0,
            received_delegations: Vec::new(),
        }
    }

    /// Check if producer is active (can produce blocks)
    /// Producers in unbonding period are still considered active
    pub fn is_active(&self) -> bool {
        matches!(
            self.status,
            ProducerStatus::Active | ProducerStatus::Unbonding { .. }
        )
    }

    /// Check if producer can produce blocks in the current slot
    /// Producers in unbonding period can still produce blocks
    pub fn can_produce(&self) -> bool {
        matches!(
            self.status,
            ProducerStatus::Active | ProducerStatus::Unbonding { .. }
        )
    }

    /// Check if bond lock period has expired
    pub fn is_bond_unlocked(&self, current_height: u64, lock_duration: u64) -> bool {
        current_height >= self.registered_at + lock_duration
    }

    /// Start unbonding period for exit
    pub fn start_unbonding(&mut self, current_height: u64) {
        self.status = ProducerStatus::Unbonding {
            started_at: current_height,
        };
    }

    /// Check if unbonding period has completed
    pub fn is_unbonding_complete(&self, current_height: u64, unbonding_duration: u64) -> bool {
        match self.status {
            ProducerStatus::Unbonding { started_at } => {
                current_height >= started_at + unbonding_duration
            }
            _ => false,
        }
    }

    /// Get the height when unbonding started (if in unbonding)
    pub fn unbonding_started_at(&self) -> Option<u64> {
        match self.status {
            ProducerStatus::Unbonding { started_at } => Some(started_at),
            _ => None,
        }
    }

    /// Complete the exit process
    pub fn complete_exit(&mut self) {
        self.status = ProducerStatus::Exited;
    }

    /// Apply slashing penalty (100% bond burned)
    /// Slashing is always complete - the full bond is destroyed
    pub fn slash(&mut self, current_height: u64) {
        self.status = ProducerStatus::Slashed {
            slashed_at: current_height,
        };
    }

    /// Get the slashed bond amount (100% for slashed producers)
    pub fn slashed_amount(&self) -> u64 {
        match self.status {
            ProducerStatus::Slashed { .. } => self.bond_amount,
            _ => 0,
        }
    }

    /// Get the remaining bond amount after slashing (always 0 for slashed)
    pub fn remaining_bond(&self) -> u64 {
        match self.status {
            ProducerStatus::Slashed { .. } => 0,
            _ => self.bond_amount,
        }
    }

    /// Check if producer has been slashed
    pub fn is_slashed(&self) -> bool {
        matches!(self.status, ProducerStatus::Slashed { .. })
    }

    // ==================== Bond Stacking Methods ====================

    /// Get the number of bonds staked
    pub fn bonds(&self) -> u32 {
        self.bond_count
    }

    /// Check if producer can add more bonds
    pub fn can_add_bonds(&self, count: u32) -> bool {
        self.is_active() && self.bond_count + count <= MAX_BONDS_PER_PRODUCER
    }

    /// Add additional bonds to this producer
    ///
    /// # Arguments
    /// * `bond_outpoints` - List of (tx_hash, output_index) for the new bonds
    /// * `amount_per_bond` - Amount per bond (should be BOND_UNIT)
    ///
    /// # Returns
    /// Number of bonds actually added (may be less if hitting MAX_BONDS_PER_PRODUCER)
    pub fn add_bonds(&mut self, bond_outpoints: Vec<(Hash, u32)>, amount_per_bond: u64) -> u32 {
        if !self.is_active() {
            return 0;
        }

        let available_slots = MAX_BONDS_PER_PRODUCER.saturating_sub(self.bond_count);
        let bonds_to_add = (bond_outpoints.len() as u32).min(available_slots);

        for outpoint in bond_outpoints.into_iter().take(bonds_to_add as usize) {
            self.additional_bonds.push(outpoint);
            self.bond_amount = self.bond_amount.saturating_add(amount_per_bond);
        }

        self.bond_count += bonds_to_add;
        bonds_to_add
    }

    /// Remove bonds from this producer (for partial withdrawal)
    ///
    /// # Arguments
    /// * `count` - Number of bonds to remove
    /// * `bond_unit` - Amount per bond (use `NetworkParams::bond_unit()` or `bond_unit_for_network()`)
    ///
    /// # Returns
    /// List of bond outpoints that were removed (for unlocking)
    pub fn remove_bonds(&mut self, count: u32, bond_unit: u64) -> Vec<(Hash, u32)> {
        if count == 0 || !self.is_active() {
            return Vec::new();
        }

        // Can't remove all bonds - must keep at least 1
        let removable = self.bond_count.saturating_sub(1).min(count);
        if removable == 0 {
            return Vec::new();
        }

        let bond_unit = if bond_unit == 0 { BOND_UNIT } else { bond_unit };
        let mut removed = Vec::new();

        // Remove from additional_bonds first
        for _ in 0..removable {
            if let Some(outpoint) = self.additional_bonds.pop() {
                removed.push(outpoint);
                self.bond_amount = self.bond_amount.saturating_sub(bond_unit);
            }
        }

        self.bond_count -= removed.len() as u32;
        removed
    }

    /// Get all bond outpoints (primary + additional)
    pub fn all_bond_outpoints(&self) -> Vec<(Hash, u32)> {
        let mut all = vec![self.bond_outpoint];
        all.extend(self.additional_bonds.iter().cloned());
        all
    }

    /// Get the selection weight including bond delegations.
    ///
    /// This is used for weighted producer selection and tier computation.
    /// Includes own bonds plus all delegated bonds received from other producers.
    /// Returns 0 if producer is not active.
    pub fn selection_weight(&self) -> u64 {
        if self.is_active() {
            let own_bonds = self.bond_count as u64;
            let delegated: u64 = self
                .received_delegations
                .iter()
                .map(|(_, count)| *count as u64)
                .sum();
            own_bonds + delegated
        } else {
            0
        }
    }

    /// DEPRECATED: Credit reward to this producer's pending balance
    ///
    /// This method is vestigial from the Pull/Claim reward model.
    /// The active EpochPool system distributes rewards directly to UTXOs.
    #[deprecated(note = "Pull/Claim model replaced by EpochPool - rewards go directly to UTXOs")]
    #[allow(dead_code)]
    pub fn credit_reward(&mut self, amount: u64) {
        #[allow(deprecated)]
        {
            self.pending_rewards = self.pending_rewards.saturating_add(amount);
        }
    }

    /// DEPRECATED: Claim all pending rewards
    ///
    /// This method is vestigial from the Pull/Claim reward model.
    /// The active EpochPool system distributes rewards directly to UTXOs.
    #[deprecated(note = "Pull/Claim model replaced by EpochPool - rewards go directly to UTXOs")]
    #[allow(dead_code)]
    pub fn claim_rewards(&mut self) -> Option<u64> {
        #[allow(deprecated)]
        {
            if self.pending_rewards == 0 {
                return None;
            }
            let amount = self.pending_rewards;
            self.pending_rewards = 0;
            Some(amount)
        }
    }

    /// DEPRECATED: Check if producer has pending rewards to claim
    ///
    /// This method is vestigial from the Pull/Claim reward model.
    #[deprecated(note = "Pull/Claim model replaced by EpochPool - rewards go directly to UTXOs")]
    #[allow(dead_code)]
    pub fn has_pending_rewards(&self) -> bool {
        #[allow(deprecated)]
        {
            self.pending_rewards > 0
        }
    }

    /// Get the producer's base weight based on seniority
    ///
    /// Uses discrete yearly steps: 0-1yr=1, 1-2yr=2, 2-3yr=3, 3+yr=4 (max)
    #[allow(deprecated)]
    pub fn weight(&self, current_height: u64) -> u64 {
        producer_weight(self.registered_at, current_height)
    }

    /// Get the producer's base weight for a specific network
    ///
    /// Uses network-specific time acceleration for devnet/testnet.
    pub fn weight_for_network(&self, current_height: u64, network: Network) -> u64 {
        producer_weight_for_network(self.registered_at, current_height, network)
    }

    /// Get the producer's effective weight
    ///
    /// Per alignment decision: NO activity penalty. Weight is purely seniority-based.
    /// Producers who miss slots simply miss rewards - no weight reduction.
    /// Only slashable offense is double production (equivocation).
    ///
    /// Note: activity_gaps field is retained for tracking purposes but does not
    /// affect weight calculation.
    #[allow(deprecated)]
    pub fn effective_weight(&self, current_height: u64) -> u64 {
        // No penalty - just return base seniority weight
        producer_weight(self.registered_at, current_height)
    }

    /// Get the producer's effective weight with decimal precision
    ///
    /// Per alignment decision: NO activity penalty.
    pub fn effective_weight_precise(&self, current_height: u64) -> f64 {
        // No penalty - just return base seniority weight
        producer_weight_precise(self.registered_at, current_height)
    }

    /// Get the producer's effective weight for a specific network
    ///
    /// Per alignment decision: NO activity penalty.
    pub fn effective_weight_for_network(&self, current_height: u64, network: Network) -> u64 {
        // No penalty - just return base seniority weight
        producer_weight_for_network(self.registered_at, current_height, network)
    }

    /// Get the producer's effective weight with decimal precision for a specific network
    ///
    /// Per alignment decision: NO activity penalty.
    pub fn effective_weight_precise_for_network(
        &self,
        current_height: u64,
        network: Network,
    ) -> f64 {
        // No penalty - just return base seniority weight
        producer_weight_precise_for_network(self.registered_at, current_height, network)
    }

    /// Record activity (block production or voting)
    ///
    /// Call this when the producer produces a block or casts a vote.
    /// Updates last_activity timestamp for activity status tracking.
    /// Note: Activity gaps are no longer tracked or penalized per alignment decision.
    pub fn record_activity(&mut self, current_height: u64) {
        self.last_activity = current_height;
    }

    /// Record activity with network-specific inactivity threshold
    ///
    /// Note: Activity gaps are no longer tracked or penalized per alignment decision.
    pub fn record_activity_for_network(&mut self, current_height: u64, _network: Network) {
        self.last_activity = current_height;
    }

    /// Check if producer has been inactive for too long (mainnet default)
    ///
    /// **Deprecated**: Use `is_inactive_for_network()` for network-aware calculations.
    #[deprecated(
        note = "Use is_inactive_for_network(current_height, network) for network-aware calculations"
    )]
    pub fn is_inactive(&self, current_height: u64) -> bool {
        #[allow(deprecated)]
        let threshold = INACTIVITY_THRESHOLD;
        current_height.saturating_sub(self.last_activity) > threshold
    }

    /// Check if producer has been inactive for a specific network
    pub fn is_inactive_for_network(&self, current_height: u64, network: Network) -> bool {
        current_height.saturating_sub(self.last_activity) > network.inactivity_threshold()
    }

    /// Get the current inactivity duration in blocks
    pub fn inactivity_duration(&self, current_height: u64) -> u64 {
        current_height.saturating_sub(self.last_activity)
    }

    /// DEPRECATED: Activity gaps are no longer tracked or penalized.
    /// This method is kept for API compatibility but does nothing meaningful.
    #[deprecated(note = "Activity gaps no longer affect consensus")]
    pub fn reset_activity_gaps(&mut self) {
        #[allow(deprecated)]
        {
            self.activity_gaps = 0;
        }
    }

    // ==================== Activity Status System ====================

    /// Determine the producer's current activity status (mainnet default)
    ///
    /// **Deprecated**: Use `activity_status_for_network()` for network-aware calculations.
    ///
    /// Based on how long since last block production:
    /// - Active: < 1 week (INACTIVITY_THRESHOLD)
    /// - RecentlyInactive: 1-2 weeks (grace period)
    /// - Dormant: >= 2 weeks
    #[deprecated(
        note = "Use activity_status_for_network(current_height, network) for network-aware calculations"
    )]
    pub fn activity_status(&self, current_height: u64) -> ActivityStatus {
        let blocks_since_last = current_height.saturating_sub(self.last_activity);

        #[allow(deprecated)]
        let threshold = INACTIVITY_THRESHOLD;

        if blocks_since_last < threshold {
            ActivityStatus::Active
        } else if blocks_since_last < threshold * 2 {
            ActivityStatus::RecentlyInactive
        } else {
            ActivityStatus::Dormant
        }
    }

    /// Determine the producer's activity status for a specific network
    pub fn activity_status_for_network(
        &self,
        current_height: u64,
        network: Network,
    ) -> ActivityStatus {
        let blocks_since_last = current_height.saturating_sub(self.last_activity);
        let threshold = network.inactivity_threshold();

        if blocks_since_last < threshold {
            ActivityStatus::Active
        } else if blocks_since_last < threshold * 2 {
            ActivityStatus::RecentlyInactive
        } else {
            ActivityStatus::Dormant
        }
    }

    /// Does this producer have governance power (can vote, counts for quorum)?
    ///
    /// **Only Active producers have governance power.**
    /// This implements "el silencio no bloquea" - inactive producers
    /// cannot block governance changes.
    #[allow(deprecated)]
    pub fn has_governance_power(&self, current_height: u64) -> bool {
        self.is_active() && self.activity_status(current_height) == ActivityStatus::Active
    }

    /// Does this producer have governance power for a specific network?
    pub fn has_governance_power_for_network(&self, current_height: u64, network: Network) -> bool {
        self.is_active()
            && self.activity_status_for_network(current_height, network) == ActivityStatus::Active
    }

    /// Does this producer count for quorum calculations?
    ///
    /// Same as has_governance_power - only Active status counts.
    /// Inactive/dormant producers don't affect quorum denominator.
    pub fn counts_for_quorum(&self, current_height: u64) -> bool {
        self.has_governance_power(current_height)
    }

    /// Does this producer count for quorum for a specific network?
    pub fn counts_for_quorum_for_network(&self, current_height: u64, network: Network) -> bool {
        self.has_governance_power_for_network(current_height, network)
    }
}

/// Exit history retention period: 8 years (2 eras)
/// After this period, a producer can re-register without the prior_exit penalty.
pub const EXIT_HISTORY_RETENTION: u64 = 2 * 2_102_400; // 2 eras = ~8 years

/// Set of producers with their states
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProducerSet {
    /// All producers indexed by public key hash
    producers: HashMap<Hash, ProducerInfo>,
    /// Exit history: pubkey hash -> exit block height (anti-Sybil with expiration)
    ///
    /// Used to track producers who have previously exited the network.
    /// When they re-register within EXIT_HISTORY_RETENTION, they start fresh
    /// with weight 1 (maturity cooldown). After 8 years, the record expires
    /// and they can re-register without penalty.
    ///
    /// This bounds the exit_history size while maintaining anti-Sybil protection
    /// beyond any realistic attack horizon.
    #[serde(default)]
    exit_history: HashMap<Hash, u64>,
    /// Epoch-based cache of active producer keys at a given height.
    /// Avoids O(n) scan of all producers every slot.
    /// Invalidated on any mutation (register, exit, slash, etc.).
    #[serde(skip)]
    active_cache: Option<(u64, Vec<Hash>)>,
    /// Index of unbonding producers by their unbonding start height.
    /// Enables O(k) lookup in `process_unbonding()` instead of O(n) full scan.
    /// Rebuilt on deserialization from `producers` map.
    #[serde(skip)]
    unbonding_index: std::collections::BTreeMap<u64, Vec<Hash>>,
}

impl ProducerSet {
    /// Create a new empty producer set
    pub fn new() -> Self {
        Self {
            producers: HashMap::new(),
            exit_history: HashMap::new(),
            active_cache: None,
            unbonding_index: std::collections::BTreeMap::new(),
        }
    }

    /// Rebuild the unbonding index from producers map.
    /// Called after deserialization to restore the skip-serialized index.
    pub fn rebuild_unbonding_index(&mut self) {
        self.unbonding_index.clear();
        for (key, info) in &self.producers {
            if let ProducerStatus::Unbonding { started_at } = info.status {
                self.unbonding_index
                    .entry(started_at)
                    .or_default()
                    .push(*key);
            }
        }
    }

    /// Clear all producers (used during chain reorganization)
    pub fn clear(&mut self) {
        self.producers.clear();
        self.active_cache = None;
        // Keep exit_history as it's needed for anti-sybil checks
    }

    /// Load producer set from file
    pub fn load(path: &Path) -> Result<Self, StorageError> {
        if !path.exists() {
            return Ok(Self::new());
        }

        let data = std::fs::read(path)?;
        let mut set: Self =
            bincode::deserialize(&data).map_err(|e| StorageError::Serialization(e.to_string()))?;
        set.rebuild_unbonding_index();
        Ok(set)
    }

    /// Save producer set to file
    pub fn save(&self, path: &Path) -> Result<(), StorageError> {
        let data =
            bincode::serialize(self).map_err(|e| StorageError::Serialization(e.to_string()))?;
        std::fs::write(path, data)?;
        Ok(())
    }

    /// Register a new producer
    ///
    /// If the producer has previously exited (in exit_history and not expired),
    /// they will have `has_prior_exit = true` and start with weight 1 (maturity cooldown).
    ///
    /// Exit history expires after EXIT_HISTORY_RETENTION (~8 years).
    pub fn register(
        &mut self,
        mut info: ProducerInfo,
        current_height: u64,
    ) -> Result<(), StorageError> {
        let key = crypto_hash(info.public_key.as_bytes());
        if self.producers.contains_key(&key) {
            return Err(StorageError::AlreadyExists(
                "Producer already registered".to_string(),
            ));
        }

        // Anti-Sybil: Check if this producer has recently exited (not expired)
        // If so, mark them as having prior exit (they start fresh with weight 1)
        if self.exited_recently(&key, current_height) {
            info.has_prior_exit = true;
        }

        self.producers.insert(key, info);
        self.active_cache = None;
        Ok(())
    }

    /// Register a new producer with network-specific exit history retention
    pub fn register_for_network(
        &mut self,
        mut info: ProducerInfo,
        current_height: u64,
        network: Network,
    ) -> Result<(), StorageError> {
        let key = crypto_hash(info.public_key.as_bytes());
        if self.producers.contains_key(&key) {
            return Err(StorageError::AlreadyExists(
                "Producer already registered".to_string(),
            ));
        }

        // Anti-Sybil: Check if this producer has recently exited (not expired for this network)
        if self.exited_recently_for_network(&key, current_height, network) {
            info.has_prior_exit = true;
        }

        self.producers.insert(key, info);
        self.active_cache = None;
        Ok(())
    }

    /// Check if a pubkey hash has recently exited (within retention period)
    fn exited_recently(&self, pubkey_hash: &Hash, current_height: u64) -> bool {
        match self.exit_history.get(pubkey_hash) {
            Some(&exit_height) => {
                current_height.saturating_sub(exit_height) < EXIT_HISTORY_RETENTION
            }
            None => false,
        }
    }

    /// Check if a pubkey hash has recently exited for a specific network
    fn exited_recently_for_network(
        &self,
        pubkey_hash: &Hash,
        current_height: u64,
        network: Network,
    ) -> bool {
        match self.exit_history.get(pubkey_hash) {
            Some(&exit_height) => {
                current_height.saturating_sub(exit_height) < network.exit_history_retention()
            }
            None => false,
        }
    }

    /// Check if a pubkey has previously exited (for anti-Sybil verification)
    ///
    /// Note: This checks if the exit is still within the retention period.
    /// After 8 years, the exit record expires and this returns false.
    pub fn has_prior_exit(&self, pubkey: &PublicKey, current_height: u64) -> bool {
        let key = crypto_hash(pubkey.as_bytes());
        self.exited_recently(&key, current_height)
    }

    /// Check if a pubkey has previously exited for a specific network
    pub fn has_prior_exit_for_network(
        &self,
        pubkey: &PublicKey,
        current_height: u64,
        network: Network,
    ) -> bool {
        let key = crypto_hash(pubkey.as_bytes());
        self.exited_recently_for_network(&key, current_height, network)
    }

    /// Prune expired exit history entries
    ///
    /// Call this periodically (e.g., once per era) to clean up old entries
    /// and bound the exit_history size.
    pub fn prune_exit_history(&mut self, current_height: u64) {
        self.exit_history.retain(|_, &mut exit_height| {
            current_height.saturating_sub(exit_height) < EXIT_HISTORY_RETENTION
        });
    }

    /// Prune expired exit history entries for a specific network
    pub fn prune_exit_history_for_network(&mut self, current_height: u64, network: Network) {
        let retention = network.exit_history_retention();
        self.exit_history
            .retain(|_, &mut exit_height| current_height.saturating_sub(exit_height) < retention);
    }

    /// Register a genesis producer (special registration without VDF/UTXO verification)
    ///
    /// Genesis producers are registered at height 0 with synthetic bond outpoints
    /// (Hash::ZERO). They cannot unbond their genesis bonds but can participate
    /// in block production from the start.
    ///
    /// # Arguments
    /// - `pubkey`: Producer's public key
    /// - `bond_count`: Number of bonds (typically 1)
    ///
    /// # Returns
    /// `Ok(())` if registered, `Err` if already exists
    #[allow(deprecated)]
    pub fn register_genesis_producer(
        &mut self,
        pubkey: PublicKey,
        bond_count: u32,
        bond_unit: u64,
    ) -> Result<(), StorageError> {
        let key = crypto_hash(pubkey.as_bytes());
        if self.producers.contains_key(&key) {
            return Err(StorageError::AlreadyExists(
                "Genesis producer already registered".to_string(),
            ));
        }

        // Use network-specific bond_unit instead of hardcoded BOND_UNIT
        let bond_amount = (bond_count as u64) * bond_unit;
        let info = ProducerInfo {
            public_key: pubkey,
            registered_at: 0, // Genesis registration
            bond_amount,
            bond_outpoint: (Hash::ZERO, 0), // Synthetic - cannot unbond
            status: ProducerStatus::Active,
            blocks_produced: 0,
            slots_missed: 0,
            registration_era: 0,
            pending_rewards: 0,
            has_prior_exit: false,
            last_activity: 0,
            activity_gaps: 0,
            bond_count,
            additional_bonds: Vec::new(),
            delegated_to: None,
            delegated_bonds: 0,
            received_delegations: Vec::new(),
        };

        self.producers.insert(key, info);
        self.active_cache = None;
        Ok(())
    }

    /// Create a ProducerSet with genesis producers pre-registered
    ///
    /// # Arguments
    /// - `producers`: Vec of (PublicKey, bond_count) tuples
    /// - `bond_unit`: Network-specific bond unit (e.g., 1 DOLI for devnet)
    pub fn with_genesis_producers(producers: Vec<(PublicKey, u32)>, bond_unit: u64) -> Self {
        let mut set = Self::new();
        for (pubkey, bond_count) in producers {
            // Ignore errors (shouldn't happen for fresh set)
            let _ = set.register_genesis_producer(pubkey, bond_count, bond_unit);
        }
        set
    }

    /// Get the size of the exit history (for monitoring)
    pub fn exit_history_size(&self) -> usize {
        self.exit_history.len()
    }

    /// Get producer info by public key hash
    pub fn get(&self, pubkey_hash: &Hash) -> Option<&ProducerInfo> {
        self.producers.get(pubkey_hash)
    }

    /// Get mutable producer info
    pub fn get_mut(&mut self, pubkey_hash: &Hash) -> Option<&mut ProducerInfo> {
        self.producers.get_mut(pubkey_hash)
    }

    /// Get producer by public key
    pub fn get_by_pubkey(&self, pubkey: &PublicKey) -> Option<&ProducerInfo> {
        self.producers.get(&crypto_hash(pubkey.as_bytes()))
    }

    /// Get mutable producer by public key
    pub fn get_by_pubkey_mut(&mut self, pubkey: &PublicKey) -> Option<&mut ProducerInfo> {
        self.producers.get_mut(&crypto_hash(pubkey.as_bytes()))
    }

    /// Delegate bonds from one producer to another.
    ///
    /// The delegator's `delegated_to` and `delegated_bonds` are set.
    /// The delegatee's `received_delegations` list is updated.
    /// Returns an error string if either producer doesn't exist or isn't active.
    pub fn delegate_bonds(
        &mut self,
        delegator_pubkey: &PublicKey,
        delegatee_pubkey: &PublicKey,
        bond_count: u32,
    ) -> Result<(), String> {
        let delegator_hash = crypto_hash(delegator_pubkey.as_bytes());
        let delegatee_hash = crypto_hash(delegatee_pubkey.as_bytes());

        // Validate delegator exists and is active
        let delegator = self
            .producers
            .get(&delegator_hash)
            .ok_or("delegator not found")?;
        if !delegator.is_active() {
            return Err("delegator is not active".into());
        }
        if bond_count > delegator.bond_count {
            return Err(format!(
                "insufficient bonds: has {}, delegating {}",
                delegator.bond_count, bond_count
            ));
        }
        if delegator.delegated_to.is_some() {
            return Err("delegator already has an active delegation".into());
        }

        // Validate delegatee exists and is active
        let delegatee = self
            .producers
            .get(&delegatee_hash)
            .ok_or("delegatee not found")?;
        if !delegatee.is_active() {
            return Err("delegatee is not active".into());
        }

        // Apply delegation
        if let Some(delegator) = self.producers.get_mut(&delegator_hash) {
            delegator.delegated_to = Some(*delegatee_pubkey);
            delegator.delegated_bonds = bond_count;
        }
        if let Some(delegatee) = self.producers.get_mut(&delegatee_hash) {
            delegatee
                .received_delegations
                .push((delegator_hash, bond_count));
        }

        self.active_cache = None;
        Ok(())
    }

    /// Revoke delegation from a producer.
    ///
    /// Clears the delegator's delegation state and removes the entry
    /// from the delegatee's received_delegations list.
    pub fn revoke_delegation(&mut self, delegator_pubkey: &PublicKey) -> Result<(), String> {
        let delegator_hash = crypto_hash(delegator_pubkey.as_bytes());

        let delegator = self
            .producers
            .get(&delegator_hash)
            .ok_or("delegator not found")?;
        let delegatee_pubkey = delegator
            .delegated_to
            .ok_or("no active delegation to revoke")?;

        let delegatee_hash = crypto_hash(delegatee_pubkey.as_bytes());

        // Clear delegator state
        if let Some(delegator) = self.producers.get_mut(&delegator_hash) {
            delegator.delegated_to = None;
            delegator.delegated_bonds = 0;
        }

        // Remove from delegatee's received list
        if let Some(delegatee) = self.producers.get_mut(&delegatee_hash) {
            delegatee
                .received_delegations
                .retain(|(hash, _)| hash != &delegator_hash);
        }

        self.active_cache = None;
        Ok(())
    }

    /// Get all active producers
    pub fn active_producers(&self) -> Vec<&ProducerInfo> {
        self.producers.values().filter(|p| p.is_active()).collect()
    }

    /// Get active producers that are eligible for scheduling at the given height.
    ///
    /// A producer is eligible for scheduling if:
    /// 1. They are in active status
    /// 2. They have passed the ACTIVATION_DELAY since registration
    ///
    /// Genesis producers (registered_at == 0) are exempt from ACTIVATION_DELAY
    /// because they are pre-registered in the chainspec and don't need network
    /// propagation time.
    ///
    /// This ensures all nodes have the same view of the producer set for a given
    /// height, preventing scheduling conflicts when new producers join.
    ///
    /// Uses epoch-based cache when available (call `ensure_active_cache()` to populate).
    pub fn active_producers_at_height(&self, current_height: u64) -> Vec<&ProducerInfo> {
        // Fast path: use cached keys if available and valid for this height
        if let Some((cached_height, ref keys)) = self.active_cache {
            if cached_height == current_height {
                return keys.iter().filter_map(|k| self.producers.get(k)).collect();
            }
        }

        // Slow path: full scan
        self.producers
            .values()
            .filter(|p| {
                p.is_active()
                    && (p.registered_at == 0
                        || current_height >= p.registered_at.saturating_add(ACTIVATION_DELAY))
            })
            .collect()
    }

    /// Pre-build the active producer cache for the given height.
    ///
    /// Call once per slot before the hot path to avoid repeated O(n) scans.
    /// The cache is automatically invalidated on any producer state mutation.
    pub fn ensure_active_cache(&mut self, current_height: u64) {
        if let Some((cached_height, _)) = &self.active_cache {
            if *cached_height == current_height {
                return; // Cache already valid
            }
        }

        let keys: Vec<Hash> = self
            .producers
            .iter()
            .filter(|(_, p)| {
                p.is_active() && current_height >= p.registered_at.saturating_add(ACTIVATION_DELAY)
            })
            .map(|(k, _)| *k)
            .collect();

        self.active_cache = Some((current_height, keys));
    }

    /// Get all producers (all states)
    pub fn all_producers(&self) -> Vec<&ProducerInfo> {
        self.producers.values().collect()
    }

    /// Get count of active producers
    pub fn active_count(&self) -> usize {
        self.producers.values().filter(|p| p.is_active()).count()
    }

    /// Get count of active producers eligible for scheduling at the given height.
    pub fn active_count_at_height(&self, current_height: u64) -> usize {
        self.active_producers_at_height(current_height).len()
    }

    /// Get total producer count (all states)
    pub fn total_count(&self) -> usize {
        self.producers.len()
    }

    /// Process exit request for a producer (starts unbonding period)
    pub fn request_exit(
        &mut self,
        pubkey: &PublicKey,
        current_height: u64,
    ) -> Result<(), StorageError> {
        let info = self
            .get_by_pubkey_mut(pubkey)
            .ok_or_else(|| StorageError::NotFound("Producer not found".to_string()))?;

        let key = crypto_hash(pubkey.as_bytes());
        match info.status {
            ProducerStatus::Active => {
                info.start_unbonding(current_height);
                self.active_cache = None;
                // Maintain unbonding index
                self.unbonding_index
                    .entry(current_height)
                    .or_default()
                    .push(key);
                Ok(())
            }
            ProducerStatus::Unbonding { .. } => Err(StorageError::AlreadyExists(
                "Already in unbonding".to_string(),
            )),
            ProducerStatus::Exited => {
                Err(StorageError::AlreadyExists("Already exited".to_string()))
            }
            ProducerStatus::Slashed { .. } => Err(StorageError::AlreadyExists(
                "Producer was slashed".to_string(),
            )),
        }
    }

    /// Cancel an exit request during unbonding period
    ///
    /// Allows a producer to change their mind before exit is final.
    /// This is a safety feature for:
    /// - Accidental exit requests
    /// - Compromised wallet recovery
    /// - Changed circumstances
    ///
    /// Returns producer to Active status. No penalty is applied.
    /// Seniority (registered_at) is preserved.
    ///
    /// # Errors
    /// - NotFound: Producer doesn't exist
    /// - InvalidState: Producer is not in unbonding (already active, exited, or slashed)
    pub fn cancel_exit(&mut self, pubkey: &PublicKey) -> Result<(), StorageError> {
        let key = crypto_hash(pubkey.as_bytes());
        let info = self
            .get_by_pubkey_mut(pubkey)
            .ok_or_else(|| StorageError::NotFound("Producer not found".to_string()))?;

        match info.status {
            ProducerStatus::Unbonding { started_at } => {
                info.status = ProducerStatus::Active;
                self.active_cache = None;
                // Remove from unbonding index
                if let Some(entries) = self.unbonding_index.get_mut(&started_at) {
                    entries.retain(|k| k != &key);
                    if entries.is_empty() {
                        self.unbonding_index.remove(&started_at);
                    }
                }
                Ok(())
            }
            ProducerStatus::Active => Err(StorageError::AlreadyExists(
                "Not in unbonding - already active".to_string(),
            )),
            ProducerStatus::Exited => Err(StorageError::NotFound(
                "Cannot cancel - already exited".to_string(),
            )),
            ProducerStatus::Slashed { .. } => Err(StorageError::NotFound(
                "Cannot cancel - producer was slashed".to_string(),
            )),
        }
    }

    /// Process completed unbonding periods and mark as exited.
    ///
    /// Uses the unbonding index for O(k) lookup where k = completed producers,
    /// instead of O(n) scan of all producers.
    ///
    /// Returns producers that have completed unbonding and are ready for bond claim.
    /// Also records the exit in `exit_history` with the exit height for expiration tracking.
    pub fn process_unbonding(
        &mut self,
        current_height: u64,
        unbonding_duration: u64,
    ) -> Vec<ProducerInfo> {
        // Completion threshold: producers who started unbonding at or before this height are done
        let threshold = current_height.saturating_sub(unbonding_duration);

        // Collect all keys from the index with started_at ≤ threshold
        let mut candidate_keys: Vec<(u64, Hash)> = Vec::new();
        for (&started_at, keys) in self.unbonding_index.range(..=threshold) {
            for key in keys {
                candidate_keys.push((started_at, *key));
            }
        }

        let mut completed = Vec::new();

        for (started_at, key) in &candidate_keys {
            if let Some(info) = self.producers.get_mut(key) {
                if info.is_unbonding_complete(current_height, unbonding_duration) {
                    info.complete_exit();
                    completed.push(info.clone());
                    self.exit_history.insert(*key, current_height);
                }
            }
            // Remove from index
            if let Some(entries) = self.unbonding_index.get_mut(started_at) {
                entries.retain(|k| k != key);
            }
        }

        // Clean up empty index entries
        self.unbonding_index.retain(|_, v| !v.is_empty());

        if !completed.is_empty() {
            self.active_cache = None;
        }

        completed
    }

    /// Slash a producer for misbehavior (100% bond burned)
    ///
    /// Returns the amount burned. Also records the slash in exit_history
    /// so if they ever re-register, they start fresh (maturity cooldown).
    /// Exit record expires after EXIT_HISTORY_RETENTION (~8 years).
    pub fn slash_producer(
        &mut self,
        pubkey: &PublicKey,
        current_height: u64,
    ) -> Result<u64, StorageError> {
        let key = crypto_hash(pubkey.as_bytes());
        let info = self
            .get_by_pubkey_mut(pubkey)
            .ok_or_else(|| StorageError::NotFound("Producer not found".to_string()))?;

        let slashed_amount = info.bond_amount;
        info.slash(current_height);

        // Record in exit history with current height (slashed producers lose all seniority)
        self.exit_history.insert(key, current_height);
        self.active_cache = None;

        Ok(slashed_amount)
    }

    /// Check if a producer can renew (bond lock expired)
    pub fn can_renew(&self, pubkey: &PublicKey, current_height: u64, lock_duration: u64) -> bool {
        self.get_by_pubkey(pubkey)
            .map(|info| info.is_bond_unlocked(current_height, lock_duration))
            .unwrap_or(false)
    }

    /// Renew a producer with a new bond (for era transition)
    pub fn renew(
        &mut self,
        pubkey: &PublicKey,
        new_bond_amount: u64,
        new_bond_outpoint: (Hash, u32),
        current_height: u64,
        new_era: u32,
    ) -> Result<u64, StorageError> {
        let info = self
            .get_by_pubkey_mut(pubkey)
            .ok_or_else(|| StorageError::NotFound("Producer not found".to_string()))?;

        // Return old bond amount for release
        let old_bond = info.bond_amount;

        // Update to new bond
        info.bond_amount = new_bond_amount;
        info.bond_outpoint = new_bond_outpoint;
        info.registered_at = current_height;
        info.registration_era = new_era;
        info.status = ProducerStatus::Active;
        self.active_cache = None;

        Ok(old_bond)
    }

    /// Remove exited producers from the set (cleanup)
    pub fn cleanup_exited(&mut self) {
        let before = self.producers.len();
        self.producers
            .retain(|_, info| !matches!(info.status, ProducerStatus::Exited));
        if self.producers.len() != before {
            self.active_cache = None;
        }
    }

    /// Iterate over all producers
    pub fn iter(&self) -> impl Iterator<Item = (&Hash, &ProducerInfo)> {
        self.producers.iter()
    }

    /// DEPRECATED: Record a produced block for a producer
    ///
    /// This method is vestigial from the Pull/Claim reward model.
    /// Block counts are now calculated from BlockStore on-demand.
    #[deprecated(note = "Block counts calculated from BlockStore - this is never called")]
    #[allow(dead_code)]
    pub fn record_block_produced(&mut self, pubkey: &PublicKey) {
        #[allow(deprecated)]
        if let Some(info) = self.get_by_pubkey_mut(pubkey) {
            info.blocks_produced += 1;
        }
    }

    /// Record a missed slot for a producer
    pub fn record_slot_missed(&mut self, pubkey: &PublicKey) {
        if let Some(info) = self.get_by_pubkey_mut(pubkey) {
            info.slots_missed += 1;
        }
    }

    /// DEPRECATED: Credit reward to a producer (Pull/Claim model)
    ///
    /// This method is vestigial from the Pull/Claim reward model.
    /// The active EpochPool system distributes rewards directly to UTXOs.
    #[deprecated(note = "Pull/Claim model replaced by EpochPool - rewards go directly to UTXOs")]
    #[allow(dead_code)]
    pub fn credit_reward(&mut self, pubkey: &PublicKey, amount: u64) {
        #[allow(deprecated)]
        if let Some(info) = self.get_by_pubkey_mut(pubkey) {
            info.credit_reward(amount);
        }
    }

    /// DEPRECATED: Process a reward claim from a producer
    ///
    /// This method is vestigial from the Pull/Claim reward model.
    #[deprecated(note = "Pull/Claim model replaced by EpochPool - rewards go directly to UTXOs")]
    #[allow(dead_code)]
    pub fn process_claim(&mut self, pubkey: &PublicKey) -> Option<u64> {
        #[allow(deprecated)]
        self.get_by_pubkey_mut(pubkey)?.claim_rewards()
    }

    /// DEPRECATED: Get pending rewards for a producer without claiming them
    ///
    /// This method is vestigial from the Pull/Claim reward model.
    #[deprecated(note = "Pull/Claim model replaced by EpochPool - rewards go directly to UTXOs")]
    #[allow(dead_code)]
    pub fn pending_rewards(&self, pubkey: &PublicKey) -> u64 {
        #[allow(deprecated)]
        self.get_by_pubkey(pubkey)
            .map(|info| info.pending_rewards)
            .unwrap_or(0)
    }

    /// DEPRECATED: Distribute block reward among producers (Pull/Claim model)
    ///
    /// This method is vestigial from the Pull/Claim reward model.
    /// The active EpochPool system distributes rewards via EpochReward transactions
    /// calculated deterministically from the BlockStore.
    #[deprecated(note = "Pull/Claim model replaced by EpochPool - see calculate_epoch_rewards()")]
    #[allow(dead_code)]
    pub fn distribute_block_reward(&mut self, producer_pubkey: &PublicKey, reward: u64) {
        #[allow(deprecated)]
        {
            self.credit_reward(producer_pubkey, reward);
            self.record_block_produced(producer_pubkey);
        }
    }

    // ==================== Seniority Weight Methods ====================

    /// Get total weight of all active producers
    ///
    /// Weight is based on seniority (time active in the network).
    /// Used for weighted veto calculations and reward distribution.
    pub fn total_weight(&self, current_height: u64) -> u64 {
        self.producers
            .values()
            .filter(|p| p.is_active())
            .map(|p| p.weight(current_height))
            .sum()
    }

    /// Get total weight for a specific network
    pub fn total_weight_for_network(&self, current_height: u64, network: Network) -> u64 {
        self.producers
            .values()
            .filter(|p| p.is_active())
            .map(|p| p.weight_for_network(current_height, network))
            .sum()
    }

    /// Get the weighted veto threshold
    ///
    /// Returns the minimum total weight required for a veto to pass (40%).
    pub fn weighted_veto_threshold(&self, current_height: u64) -> u64 {
        let total = self.total_weight(current_height);
        // 40% threshold, rounded up
        (total * VETO_THRESHOLD_PERCENT).div_ceil(100)
    }

    /// Get the weighted veto threshold for a specific network
    pub fn weighted_veto_threshold_for_network(
        &self,
        current_height: u64,
        network: Network,
    ) -> u64 {
        let total = self.total_weight_for_network(current_height, network);
        // 40% threshold, rounded up
        (total * VETO_THRESHOLD_PERCENT).div_ceil(100)
    }

    /// Get total effective weight of producers with governance power
    ///
    /// **Only counts Active producers** (recently producing blocks).
    /// Inactive/dormant producers do NOT count - "el silencio no bloquea".
    ///
    /// Effective weight also accounts for activity gap penalties.
    pub fn total_effective_weight(&self, current_height: u64) -> u64 {
        self.producers
            .values()
            .filter(|p| p.has_governance_power(current_height))
            .map(|p| p.effective_weight(current_height))
            .sum()
    }

    /// Get total effective weight for a specific network
    ///
    /// **Only counts Active producers** for that network's threshold.
    pub fn total_effective_weight_for_network(&self, current_height: u64, network: Network) -> u64 {
        self.producers
            .values()
            .filter(|p| p.has_governance_power_for_network(current_height, network))
            .map(|p| p.effective_weight_for_network(current_height, network))
            .sum()
    }

    /// Get the effective veto threshold
    ///
    /// Uses effective weight of **only Active producers** as the base.
    /// Inactive/dormant producers don't affect the threshold denominator.
    /// Requires 40% of total effective weight to veto.
    pub fn effective_veto_threshold(&self, current_height: u64) -> u64 {
        let total = self.total_effective_weight(current_height);
        // 40% threshold, rounded up
        (total * VETO_THRESHOLD_PERCENT).div_ceil(100)
    }

    /// Get the effective veto threshold for a specific network
    pub fn effective_veto_threshold_for_network(
        &self,
        current_height: u64,
        network: Network,
    ) -> u64 {
        let total = self.total_effective_weight_for_network(current_height, network);
        // 40% threshold, rounded up
        (total * VETO_THRESHOLD_PERCENT).div_ceil(100)
    }

    /// Check if veto votes have reached the weighted threshold
    ///
    /// **Only Active producers can vote and count for quorum.**
    /// This implements "el silencio no bloquea" - inactive/dormant
    /// producers cannot block governance changes.
    ///
    /// # Arguments
    /// - `veto_pubkeys`: Public keys of producers who have voted to veto
    /// - `current_height`: Current block height for weight calculation
    ///
    /// # Returns
    /// `true` if the total effective weight of Active veto voters >= 40% of total Active weight
    pub fn has_weighted_veto(&self, veto_pubkeys: &[PublicKey], current_height: u64) -> bool {
        // Only count veto votes from producers with governance power (Active status)
        let veto_weight: u64 = veto_pubkeys
            .iter()
            .filter_map(|pk| self.get_by_pubkey(pk))
            .filter(|p| p.has_governance_power(current_height))
            .map(|p| p.effective_weight(current_height))
            .sum();

        let threshold = self.effective_veto_threshold(current_height);
        veto_weight >= threshold
    }

    /// Check if veto votes have reached the weighted threshold for a specific network
    ///
    /// **Only Active producers can vote and count for quorum.**
    pub fn has_weighted_veto_for_network(
        &self,
        veto_pubkeys: &[PublicKey],
        current_height: u64,
        network: Network,
    ) -> bool {
        // Only count veto votes from producers with governance power
        let veto_weight: u64 = veto_pubkeys
            .iter()
            .filter_map(|pk| self.get_by_pubkey(pk))
            .filter(|p| p.has_governance_power_for_network(current_height, network))
            .map(|p| p.effective_weight_for_network(current_height, network))
            .sum();

        let threshold = self.effective_veto_threshold_for_network(current_height, network);
        veto_weight >= threshold
    }

    /// Count producers with governance power (Active status)
    pub fn governance_participant_count(&self, current_height: u64) -> usize {
        self.producers
            .values()
            .filter(|p| p.has_governance_power(current_height))
            .count()
    }

    /// Count producers with governance power for a specific network
    pub fn governance_participant_count_for_network(
        &self,
        current_height: u64,
        network: Network,
    ) -> usize {
        self.producers
            .values()
            .filter(|p| p.has_governance_power_for_network(current_height, network))
            .count()
    }

    /// Get all active producers with their weights
    ///
    /// Returns a vector of (PublicKey, weight) tuples for weighted selection.
    pub fn weighted_active_producers(&self, current_height: u64) -> Vec<(PublicKey, u64)> {
        self.producers
            .values()
            .filter(|p| p.is_active())
            .map(|p| (p.public_key, p.weight(current_height)))
            .collect()
    }

    /// Get all active producers with their weights for a specific network
    pub fn weighted_active_producers_for_network(
        &self,
        current_height: u64,
        network: Network,
    ) -> Vec<(PublicKey, u64)> {
        self.producers
            .values()
            .filter(|p| p.is_active())
            .map(|p| (p.public_key, p.weight_for_network(current_height, network)))
            .collect()
    }

    /// Distribute rewards proportionally by weight
    ///
    /// Instead of equal distribution, producers with higher seniority
    /// receive proportionally larger rewards.
    ///
    /// # Arguments
    /// - `total_reward`: Total reward to distribute
    /// - `current_height`: Current block height for weight calculation
    ///
    /// # Returns
    /// Number of producers who received rewards
    #[allow(deprecated)]
    pub fn distribute_weighted_rewards(&mut self, total_reward: u64, current_height: u64) -> usize {
        let total_weight = self.total_weight(current_height);
        if total_weight == 0 {
            return 0;
        }

        // Collect pubkeys and weights first to avoid borrow issues
        let distributions: Vec<(Hash, u64)> = self
            .producers
            .iter()
            .filter(|(_, p)| p.is_active())
            .map(|(hash, p)| {
                let weight = p.weight(current_height);
                let share = (total_reward * weight) / total_weight;
                (*hash, share)
            })
            .collect();

        let count = distributions.len();

        // Apply the distributions
        for (hash, share) in distributions {
            if let Some(producer) = self.producers.get_mut(&hash) {
                producer.credit_reward(share);
            }
        }

        count
    }

    /// Distribute rewards proportionally by weight for a specific network
    #[allow(deprecated)]
    pub fn distribute_weighted_rewards_for_network(
        &mut self,
        total_reward: u64,
        current_height: u64,
        network: Network,
    ) -> usize {
        let total_weight = self.total_weight_for_network(current_height, network);
        if total_weight == 0 {
            return 0;
        }

        // Collect pubkeys and weights first to avoid borrow issues
        let distributions: Vec<(Hash, u64)> = self
            .producers
            .iter()
            .filter(|(_, p)| p.is_active())
            .map(|(hash, p)| {
                let weight = p.weight_for_network(current_height, network);
                let share = (total_reward * weight) / total_weight;
                (*hash, share)
            })
            .collect();

        let count = distributions.len();

        // Apply the distributions
        for (hash, share) in distributions {
            if let Some(producer) = self.producers.get_mut(&hash) {
                producer.credit_reward(share);
            }
        }

        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::KeyPair;

    fn make_producer(height: u64) -> ProducerInfo {
        let keypair = KeyPair::generate();
        ProducerInfo::new(
            *keypair.public_key(),
            height,
            100_000_000_000, // 1000 coins
            (Hash::ZERO, 0),
            0,
            BOND_UNIT, // Use mainnet/testnet bond unit for tests
        )
    }

    #[test]
    fn test_producer_lifecycle() {
        let mut info = make_producer(1000);

        // Initially active
        assert!(info.is_active());
        assert!(info.can_produce());

        // Start unbonding (test uses 43,200 blocks as arbitrary duration)
        info.start_unbonding(2000);
        assert!(info.is_active()); // Still active during unbonding
        assert!(info.can_produce());
        assert!(!info.is_unbonding_complete(2000, 43_200));
        assert!(!info.is_unbonding_complete(45_000, 43_200)); // Just before completion
        assert!(info.is_unbonding_complete(45_200, 43_200)); // After unbonding period

        // Complete exit
        info.complete_exit();
        assert!(!info.is_active());
        assert!(!info.can_produce());
    }

    #[test]
    fn test_bond_unlock() {
        let info = make_producer(1000);
        let lock_duration = 2_102_400; // 4 years

        assert!(!info.is_bond_unlocked(1000, lock_duration));
        assert!(!info.is_bond_unlocked(2_102_399, lock_duration));
        assert!(info.is_bond_unlocked(2_103_400, lock_duration));
    }

    #[test]
    fn test_slashing() {
        let mut info = make_producer(1000);

        // Slashing is always 100% (full bond burned)
        assert!(!info.is_slashed());
        info.slash(2000);
        assert!(info.is_slashed());
        assert_eq!(info.slashed_amount(), 100_000_000_000);
        assert_eq!(info.remaining_bond(), 0);

        // Slashed producers cannot produce
        assert!(!info.is_active());
        assert!(!info.can_produce());
    }

    #[test]
    fn test_producer_set() {
        let mut set = ProducerSet::new();

        let keypair = KeyPair::generate();
        let info = ProducerInfo::new(
            *keypair.public_key(),
            1000,
            100_000_000_000,
            (Hash::ZERO, 0),
            0,
            BOND_UNIT,
        );

        // Register
        set.register(info.clone(), 1000).unwrap();
        assert_eq!(set.active_count(), 1);

        // Can't register twice
        assert!(set.register(info, 1000).is_err());

        // Request exit (starts 7-day unbonding period)
        set.request_exit(&*keypair.public_key(), 2000).unwrap();
        assert_eq!(set.active_count(), 1); // Still active during unbonding

        // Process unbonding (test uses 43,200 blocks as arbitrary duration)
        let completed = set.process_unbonding(45_200, 43_200);
        assert_eq!(completed.len(), 1);
        assert_eq!(set.active_count(), 0);
    }

    #[test]
    fn test_renewal() {
        let mut set = ProducerSet::new();

        let keypair = KeyPair::generate();
        let info = ProducerInfo::new(
            *keypair.public_key(),
            1000,
            100_000_000_000,
            (Hash::ZERO, 0),
            0,
            BOND_UNIT,
        );

        set.register(info, 1000).unwrap();

        // Renew with new bond
        let old_bond = set
            .renew(
                &*keypair.public_key(),
                70_000_000_000, // Era 2 bond
                (Hash::ZERO, 1),
                2_103_400,
                1,
            )
            .unwrap();

        assert_eq!(old_bond, 100_000_000_000);

        let renewed = set.get_by_pubkey(&*keypair.public_key()).unwrap();
        assert_eq!(renewed.bond_amount, 70_000_000_000);
        assert_eq!(renewed.registration_era, 1);
    }

    #[test]
    fn test_pending_rewards() {
        let mut info = make_producer(1000);

        // Initially no pending rewards
        assert_eq!(info.pending_rewards, 0);
        assert!(!info.has_pending_rewards());

        // Credit some rewards
        info.credit_reward(500_000_000);
        assert_eq!(info.pending_rewards, 500_000_000);
        assert!(info.has_pending_rewards());

        // Credit more rewards (accumulates)
        info.credit_reward(300_000_000);
        assert_eq!(info.pending_rewards, 800_000_000);

        // Claim rewards
        let claimed = info.claim_rewards();
        assert_eq!(claimed, Some(800_000_000));
        assert_eq!(info.pending_rewards, 0);
        assert!(!info.has_pending_rewards());

        // Claim again when empty
        let claimed_again = info.claim_rewards();
        assert_eq!(claimed_again, None);
    }

    #[test]
    fn test_producer_set_rewards() {
        let mut set = ProducerSet::new();

        let keypair = KeyPair::generate();
        let info = ProducerInfo::new(
            *keypair.public_key(),
            1000,
            100_000_000_000,
            (Hash::ZERO, 0),
            0,
            BOND_UNIT,
        );

        set.register(info, 1000).unwrap();

        // No pending rewards initially
        assert_eq!(set.pending_rewards(&*keypair.public_key()), 0);

        // Distribute block reward
        set.distribute_block_reward(&*keypair.public_key(), 500_000_000);
        assert_eq!(set.pending_rewards(&*keypair.public_key()), 500_000_000);

        // Check blocks_produced was incremented
        let producer = set.get_by_pubkey(&*keypair.public_key()).unwrap();
        assert_eq!(producer.blocks_produced, 1);

        // Distribute more rewards
        set.distribute_block_reward(&*keypair.public_key(), 500_000_000);
        assert_eq!(set.pending_rewards(&*keypair.public_key()), 1_000_000_000);
        assert_eq!(
            set.get_by_pubkey(&*keypair.public_key())
                .unwrap()
                .blocks_produced,
            2
        );

        // Process claim
        let claimed = set.process_claim(&*keypair.public_key());
        assert_eq!(claimed, Some(1_000_000_000));
        assert_eq!(set.pending_rewards(&*keypair.public_key()), 0);

        // Claim again when empty
        let claimed_again = set.process_claim(&*keypair.public_key());
        assert_eq!(claimed_again, None);
    }

    #[test]
    fn test_reward_overflow_protection() {
        let mut info = make_producer(1000);

        // Credit a large amount
        info.credit_reward(u64::MAX - 100);
        assert_eq!(info.pending_rewards, u64::MAX - 100);

        // Credit more - should saturate at u64::MAX instead of overflow
        info.credit_reward(1000);
        assert_eq!(info.pending_rewards, u64::MAX);
    }

    // ==================== Seniority Weight Tests ====================

    #[test]
    fn test_weight_increases_with_age() {
        // Weight increases in discrete yearly steps: 0-1=1, 1-2=2, 2-3=3, 3+=4
        assert_eq!(producer_weight(0, 0), 1); // Year 0
        assert_eq!(producer_weight(0, BLOCKS_PER_YEAR - 1), 1); // Just before year 1
        assert_eq!(producer_weight(0, BLOCKS_PER_YEAR), 2); // Year 1
        assert_eq!(producer_weight(0, 2 * BLOCKS_PER_YEAR), 3); // Year 2
        assert_eq!(producer_weight(0, 3 * BLOCKS_PER_YEAR), 4); // Year 3
        assert_eq!(producer_weight(0, 4 * BLOCKS_PER_YEAR), 4); // Year 4 (max)
        assert_eq!(producer_weight(0, 10 * BLOCKS_PER_YEAR), 4); // Year 10 (still max)
    }

    #[test]
    fn test_new_producer_has_weight_1() {
        let info = make_producer(1000);
        assert_eq!(info.weight(1000), 1);
        assert_eq!(info.weight(1500), 1);
        assert_eq!(info.weight(500_000), 1); // Still in year 0
    }

    #[test]
    fn test_producer_weight_over_time() {
        let info = make_producer(0);

        // Discrete yearly steps: 0-1=1, 1-2=2, 2-3=3, 3+=4
        assert_eq!(info.weight(1 * BLOCKS_PER_YEAR), 2); // Year 1
        assert_eq!(info.weight(2 * BLOCKS_PER_YEAR), 3); // Year 2
        assert_eq!(info.weight(3 * BLOCKS_PER_YEAR), 4); // Year 3
        assert_eq!(info.weight(4 * BLOCKS_PER_YEAR), 4); // Year 4 (max)

        // After 10 years, still weight 4 (max)
        assert_eq!(info.weight(10 * BLOCKS_PER_YEAR), 4);
    }

    #[test]
    fn test_weighted_veto_threshold() {
        let mut set = ProducerSet::new();
        let current_height = 0;

        // Register 3 new producers (each weight 1)
        for i in 0..3 {
            let keypair = KeyPair::generate();
            let info = ProducerInfo::new(
                *keypair.public_key(),
                current_height,
                100_000_000_000,
                (Hash::ZERO, i),
                0,
                BOND_UNIT,
            );
            set.register(info, current_height).unwrap();
        }

        // Total weight = 3 (3 producers × weight 1)
        assert_eq!(set.total_weight(current_height), 3);

        // 40% of 3 = 1.2, rounded up = 2
        let threshold = set.weighted_veto_threshold(current_height);
        assert_eq!(threshold, 2);
    }

    #[test]
    fn test_weighted_veto_threshold_mixed_ages() {
        let mut set = ProducerSet::new();
        let current_height = 9 * BLOCKS_PER_YEAR; // 9 years into network for max weight difference

        // Register a producer from genesis (9 years old, weight 4 = max)
        let keypair1 = KeyPair::generate();
        let info1 = ProducerInfo::new(
            *keypair1.public_key(),
            0, // registered at genesis
            100_000_000_000,
            (Hash::ZERO, 0),
            0,
            BOND_UNIT,
        );
        set.register(info1, 0).unwrap();

        // Register a new producer (0 years old, weight 1)
        let keypair2 = KeyPair::generate();
        let info2 = ProducerInfo::new(
            *keypair2.public_key(),
            current_height,
            100_000_000_000,
            (Hash::ZERO, 1),
            0,
            BOND_UNIT,
        );
        set.register(info2, current_height).unwrap();

        // Both producers must be actively producing to have governance power
        // Record recent activity (simulates they produced blocks recently)
        if let Some(p) = set.get_by_pubkey_mut(keypair1.public_key()) {
            p.last_activity = current_height;
        }
        if let Some(p) = set.get_by_pubkey_mut(keypair2.public_key()) {
            p.last_activity = current_height;
        }

        // Total weight = 4 + 1 = 5
        assert_eq!(set.total_weight(current_height), 5);

        // For governance (veto), only Active producers count
        // Both are now Active, so effective weight for governance = 4 + 1 = 5
        // 40% of 5 = 2.0, rounded up = 2
        let threshold = set.effective_veto_threshold(current_height);
        assert_eq!(threshold, 2);

        // Senior producer alone (weight 4) can veto (4 >= 2 threshold)
        assert!(set.has_weighted_veto(&[*keypair1.public_key()], current_height));

        // Junior producer alone (weight 1) cannot veto
        assert!(!set.has_weighted_veto(&[*keypair2.public_key()], current_height));
    }

    #[test]
    fn test_rewards_proportional_to_weight() {
        let mut set = ProducerSet::new();
        let current_height = BLOCKS_PER_YEAR; // 1 year in

        // Register a producer from genesis (1 year old, weight 2)
        let keypair1 = KeyPair::generate();
        let info1 = ProducerInfo::new(
            *keypair1.public_key(),
            0,
            100_000_000_000,
            (Hash::ZERO, 0),
            0,
            BOND_UNIT,
        );
        set.register(info1, 0).unwrap();

        // Register a new producer (0 years old, weight 1)
        let keypair2 = KeyPair::generate();
        let info2 = ProducerInfo::new(
            *keypair2.public_key(),
            current_height,
            100_000_000_000,
            (Hash::ZERO, 1),
            0,
            BOND_UNIT,
        );
        set.register(info2, current_height).unwrap();

        // Total weight = 2 + 1 = 3
        assert_eq!(set.total_weight(current_height), 3);

        // Distribute 300 units of reward
        let total_reward = 300_000_000u64;
        let distributed = set.distribute_weighted_rewards(total_reward, current_height);
        assert_eq!(distributed, 2);

        // Senior producer should get 2/3 of reward (200M)
        let senior_rewards = set.pending_rewards(&*keypair1.public_key());
        assert_eq!(senior_rewards, 200_000_000);

        // Junior producer should get 1/3 of reward (100M)
        let junior_rewards = set.pending_rewards(&*keypair2.public_key());
        assert_eq!(junior_rewards, 100_000_000);
    }

    #[test]
    fn test_weighted_active_producers() {
        let mut set = ProducerSet::new();
        let current_height = 9 * BLOCKS_PER_YEAR; // 9 years for max weight

        // Register producer at genesis (weight 4 at 9 years)
        let keypair = KeyPair::generate();
        let info = ProducerInfo::new(
            *keypair.public_key(),
            0,
            100_000_000_000,
            (Hash::ZERO, 0),
            0,
            BOND_UNIT,
        );
        set.register(info, 0).unwrap();

        let weighted = set.weighted_active_producers(current_height);
        assert_eq!(weighted.len(), 1);
        assert_eq!(weighted[0].1, 4); // weight 4 at year 9 (max)
    }

    #[test]
    fn test_weight_calculation_boundary() {
        // Test exactly at year boundaries
        assert_eq!(producer_weight(0, BLOCKS_PER_YEAR - 1), 1); // Just before year 1
        assert_eq!(producer_weight(0, BLOCKS_PER_YEAR), 2); // Exactly year 1
        assert_eq!(producer_weight(0, BLOCKS_PER_YEAR + 1), 2); // Just after year 1
    }

    // ==================== Maturity Cooldown Tests (Anti-Sybil) ====================

    #[test]
    fn test_new_producer_has_no_prior_exit() {
        let info = make_producer(1000);
        assert!(!info.has_prior_exit);
    }

    #[test]
    fn test_exit_records_in_history() {
        let mut set = ProducerSet::new();

        let keypair = KeyPair::generate();
        let info = ProducerInfo::new(
            *keypair.public_key(),
            0,
            100_000_000_000,
            (Hash::ZERO, 0),
            0,
            BOND_UNIT,
        );

        set.register(info, 0).unwrap();

        // Initially not in exit history
        assert!(!set.has_prior_exit(&*keypair.public_key(), 0));

        // Request exit
        set.request_exit(&*keypair.public_key(), 1000).unwrap();

        // Still not in exit history (unbonding not complete)
        assert!(!set.has_prior_exit(&*keypair.public_key(), 1000));

        // Complete unbonding (test uses arbitrary 43,200 blocks)
        let unbonding_duration = 43_200;
        let exit_height = 1000 + unbonding_duration;
        set.process_unbonding(exit_height, unbonding_duration);

        // Now in exit history
        assert!(set.has_prior_exit(&*keypair.public_key(), exit_height));
    }

    #[test]
    fn test_re_registration_after_exit_has_prior_exit_flag() {
        let mut set = ProducerSet::new();

        let keypair = KeyPair::generate();
        let pubkey = *keypair.public_key();

        // First registration
        let info1 = ProducerInfo::new(
            pubkey.clone(),
            0,
            100_000_000_000,
            (Hash::ZERO, 0),
            0,
            BOND_UNIT,
        );
        set.register(info1, 0).unwrap();

        // Exit completely
        set.request_exit(&pubkey, 1000).unwrap();
        set.process_unbonding(50_000, 43_200);
        set.cleanup_exited();

        // Verify producer is gone
        assert!(set.get_by_pubkey(&pubkey).is_none());
        assert!(set.has_prior_exit(&pubkey, 50_000));

        // Re-register
        let info2 = ProducerInfo::new(
            pubkey.clone(),
            100_000,
            100_000_000_000,
            (Hash::ZERO, 1),
            0,
            BOND_UNIT,
        );
        set.register(info2, 100_000).unwrap();

        // Verify has_prior_exit flag is set
        let producer = set.get_by_pubkey(&pubkey).unwrap();
        assert!(producer.has_prior_exit);

        // Weight should be 1 (starts fresh)
        assert_eq!(producer.weight(100_000), 1);
    }

    #[test]
    fn test_maturity_cooldown_prevents_seniority_transfer() {
        let mut set = ProducerSet::new();

        let keypair = KeyPair::generate();
        let pubkey = *keypair.public_key();

        // Register at genesis
        let info1 = ProducerInfo::new(
            pubkey.clone(),
            0,
            100_000_000_000,
            (Hash::ZERO, 0),
            0,
            BOND_UNIT,
        );
        set.register(info1, 0).unwrap();

        // After 9 years, weight is 4 (max, with sqrt formula)
        let nine_years = 9 * BLOCKS_PER_YEAR;
        assert_eq!(set.get_by_pubkey(&pubkey).unwrap().weight(nine_years), 4);

        // Exit at year 9
        set.request_exit(&pubkey, nine_years).unwrap();
        set.process_unbonding(nine_years + 50_000, 43_200);
        set.cleanup_exited();

        // Re-register at year 10
        let ten_years = 10 * BLOCKS_PER_YEAR;
        let info2 = ProducerInfo::new(
            pubkey.clone(),
            ten_years,
            100_000_000_000,
            (Hash::ZERO, 1),
            0,
            BOND_UNIT,
        );
        set.register(info2, ten_years).unwrap();

        // Weight should be 1, not 4! (lost all seniority due to exit)
        let producer = set.get_by_pubkey(&pubkey).unwrap();
        assert!(producer.has_prior_exit);
        assert_eq!(producer.weight(ten_years), 1);

        // After another year, weight is 2
        assert_eq!(producer.weight(ten_years + BLOCKS_PER_YEAR), 2);
    }

    #[test]
    fn test_slashed_producer_in_exit_history() {
        let mut set = ProducerSet::new();

        let keypair = KeyPair::generate();
        let pubkey = *keypair.public_key();

        let info = ProducerInfo::new(
            pubkey.clone(),
            0,
            100_000_000_000,
            (Hash::ZERO, 0),
            0,
            BOND_UNIT,
        );
        set.register(info, 0).unwrap();

        // Slash the producer
        set.slash_producer(&pubkey, 1000).unwrap();

        // Should be in exit history
        assert!(set.has_prior_exit(&pubkey, 1000));
    }

    #[test]
    fn test_new_with_prior_exit_constructor() {
        let keypair = KeyPair::generate();
        let info = ProducerInfo::new_with_prior_exit(
            *keypair.public_key(),
            1000,
            100_000_000_000,
            (Hash::ZERO, 0),
            0,
            BOND_UNIT,
        );

        assert!(info.has_prior_exit);
        assert_eq!(info.weight(1000), 1);
    }

    // ==================== Network-Aware Tests ====================

    #[test]
    fn test_devnet_time_acceleration_weight() {
        let devnet = Network::Devnet;

        // On devnet, blocks_per_year = 144 (accelerated time for ~10min eras)
        // Discrete yearly steps: 0-1=1, 1-2=2, 2-3=3, 3+=4
        let registered_at = 0;

        // After 0 blocks, weight = 1
        assert_eq!(producer_weight_for_network(registered_at, 0, devnet), 1);

        // After 144 blocks (1 simulated year), weight = 2
        assert_eq!(producer_weight_for_network(registered_at, 144, devnet), 2);

        // After 288 blocks (2 simulated years), weight = 3
        assert_eq!(producer_weight_for_network(registered_at, 288, devnet), 3);

        // After 432 blocks (3 simulated years), weight = 4 (max)
        assert_eq!(producer_weight_for_network(registered_at, 432, devnet), 4);

        // After 576 blocks (4 simulated years), still weight = 4
        assert_eq!(producer_weight_for_network(registered_at, 576, devnet), 4);
    }

    #[test]
    fn test_devnet_vs_mainnet_time_scales() {
        let devnet = Network::Devnet;
        let mainnet = Network::Mainnet;

        let registered_at = 0;

        // Discrete yearly steps: 0-1=1, 1-2=2, 2-3=3, 3+=4

        // Devnet: 144 blocks = 1 year (devnet.blocks_per_year() = 144)
        assert_eq!(producer_weight_for_network(registered_at, 144, devnet), 2);

        // Mainnet: blocks_per_year() blocks = 1 year
        assert_eq!(
            producer_weight_for_network(registered_at, mainnet.blocks_per_year(), mainnet),
            2
        );

        // Devnet: 432 blocks = 3 years (max weight)
        assert_eq!(producer_weight_for_network(registered_at, 432, devnet), 4);

        // Mainnet: 3 * blocks_per_year() blocks = 3 years (max weight)
        assert_eq!(
            producer_weight_for_network(registered_at, 3 * mainnet.blocks_per_year(), mainnet),
            4
        );
    }

    #[test]
    fn test_producer_info_weight_for_network() {
        let keypair = KeyPair::generate();
        let info = ProducerInfo::new(
            *keypair.public_key(),
            0,
            100_000_000_000,
            (Hash::ZERO, 0),
            0,
            BOND_UNIT,
        );

        let devnet = Network::Devnet;

        // Discrete yearly steps: 0-1=1, 1-2=2, 2-3=3, 3+=4

        // After 144 blocks on devnet (1 simulated year)
        assert_eq!(info.weight_for_network(144, devnet), 2);

        // After 432 blocks on devnet (3 simulated years)
        assert_eq!(info.weight_for_network(432, devnet), 4);
    }

    #[test]
    fn test_producer_set_network_aware_methods() {
        let mut set = ProducerSet::new();
        let devnet = Network::Devnet;

        // Register producer at block 0
        let keypair1 = KeyPair::generate();
        let info1 = ProducerInfo::new(
            *keypair1.public_key(),
            0,
            100_000_000_000,
            (Hash::ZERO, 0),
            0,
            BOND_UNIT,
        );
        set.register_for_network(info1, 0, devnet).unwrap();

        // Register second producer at block 540 (9 years on devnet: 60 × 9 = 540)
        let keypair2 = KeyPair::generate();
        let info2 = ProducerInfo::new(
            *keypair2.public_key(),
            540,
            100_000_000_000,
            (Hash::ZERO, 1),
            0,
            BOND_UNIT,
        );
        set.register_for_network(info2, 540, devnet).unwrap();

        // Both producers must be actively producing to have governance power
        // Record recent activity (devnet has 10-block inactivity threshold)
        if let Some(p) = set.get_by_pubkey_mut(keypair1.public_key()) {
            p.last_activity = 540; // Recently active
        }
        if let Some(p) = set.get_by_pubkey_mut(keypair2.public_key()) {
            p.last_activity = 540; // Just registered, so active
        }

        // At block 540:
        // - Producer 1: 540 blocks = 9 years, weight = 4
        // - Producer 2: 0 blocks = 0 years, weight = 1
        // Total weight = 5 (for reward distribution, which uses all producers)
        assert_eq!(set.total_weight_for_network(540, devnet), 5);

        // For governance, both are Active (recently produced blocks)
        // Effective veto threshold = 40% of 5 = 2.0, rounded up = 2
        assert_eq!(set.effective_veto_threshold_for_network(540, devnet), 2);

        // Senior producer (weight 4) can veto alone (4 >= 2 threshold)
        assert!(set.has_weighted_veto_for_network(&[*keypair1.public_key()], 540, devnet));

        // Junior producer (weight 1) cannot veto alone
        assert!(!set.has_weighted_veto_for_network(&[*keypair2.public_key()], 540, devnet));
    }

    #[test]
    fn test_devnet_exit_history_expiration() {
        let mut set = ProducerSet::new();
        let devnet = Network::Devnet;

        let keypair = KeyPair::generate();
        let pubkey = *keypair.public_key();

        // Register at block 0
        let info1 = ProducerInfo::new(
            pubkey.clone(),
            0,
            100_000_000_000,
            (Hash::ZERO, 0),
            0,
            BOND_UNIT,
        );
        set.register_for_network(info1, 0, devnet).unwrap();

        // Exit at block 576 (4 years on devnet: 144 blocks/year × 4)
        set.request_exit(&pubkey, 576).unwrap();
        set.process_unbonding(576 + 60, 60); // devnet unbonding = 60 blocks
        set.cleanup_exited();

        // At block 700, exit is recent (within 8 year retention = 1152 blocks)
        assert!(set.has_prior_exit_for_network(&pubkey, 700, devnet));

        // At block 2000, exit has expired (636 + 1152 = 1788, and 2000 > 1788)
        assert!(!set.has_prior_exit_for_network(&pubkey, 2000, devnet));
    }

    #[test]
    fn test_devnet_inactivity_threshold() {
        let keypair = KeyPair::generate();
        let mut info = ProducerInfo::new(
            *keypair.public_key(),
            0,
            100_000_000_000,
            (Hash::ZERO, 0),
            0,
            BOND_UNIT,
        );

        let devnet = Network::Devnet;

        // Devnet inactivity threshold = 30 blocks
        assert!(!info.is_inactive_for_network(15, devnet)); // 15 block gap, not inactive
        assert!(!info.is_inactive_for_network(30, devnet)); // 30 blocks, still not inactive
        assert!(info.is_inactive_for_network(31, devnet)); // 31 blocks, inactive

        // Record activity updates last_activity timestamp
        // Note: Activity gaps are no longer tracked per alignment decision
        info.record_activity_for_network(35, devnet);
        assert_eq!(info.last_activity, 35);

        info.record_activity_for_network(50, devnet);
        assert_eq!(info.last_activity, 50);

        info.record_activity_for_network(85, devnet);
        assert_eq!(info.last_activity, 85);
    }

    #[test]
    fn test_devnet_weighted_rewards_distribution() {
        let mut set = ProducerSet::new();
        let devnet = Network::Devnet;

        // Register producer at block 0 (will have 1 year seniority at block 144)
        let keypair1 = KeyPair::generate();
        let info1 = ProducerInfo::new(
            *keypair1.public_key(),
            0,
            100_000_000_000,
            (Hash::ZERO, 0),
            0,
            BOND_UNIT,
        );
        set.register_for_network(info1, 0, devnet).unwrap();

        // Register producer at block 144 (0 seniority at block 144)
        let keypair2 = KeyPair::generate();
        let info2 = ProducerInfo::new(
            *keypair2.public_key(),
            144,
            100_000_000_000,
            (Hash::ZERO, 1),
            0,
            BOND_UNIT,
        );
        set.register_for_network(info2, 144, devnet).unwrap();

        // At block 144:
        // Producer 1: 144 blocks = 1 year, weight = 2
        // Producer 2: 0 blocks = 0 years, weight = 1
        // Total weight = 3

        let total_reward = 300_000_000u64;
        let distributed = set.distribute_weighted_rewards_for_network(total_reward, 144, devnet);
        assert_eq!(distributed, 2);

        // Producer 1 gets 2/3 = 200M
        assert_eq!(set.pending_rewards(&*keypair1.public_key()), 200_000_000);

        // Producer 2 gets 1/3 = 100M
        assert_eq!(set.pending_rewards(&*keypair2.public_key()), 100_000_000);
    }

    #[test]
    fn test_era_simulation() {
        let devnet = Network::Devnet;

        // Verify the math for devnet era timing
        // Devnet: 10 second slots, 144 blocks = 1 year
        // Era = 576 blocks = 4 years (576 / 144 = 4)
        // Era duration = 576 * 10 = 5760 seconds = 96 minutes real time

        let era_blocks = 576u64;
        let simulated_years = era_blocks / devnet.blocks_per_year();
        let real_seconds = era_blocks * devnet.slot_duration();
        let real_minutes = real_seconds / 60;

        assert_eq!(simulated_years, 4);
        assert_eq!(real_minutes, 96); // 5760 seconds = 96 minutes

        // Verify weight after 1 era (4 simulated years) = max (4)
        let weight_at_4_years = producer_weight_for_network(0, era_blocks, devnet);
        assert_eq!(weight_at_4_years, 4);
    }

    #[test]
    fn test_activity_status_system() {
        // Test the "el silencio no bloquea" principle:
        // Only Active producers can participate in governance
        let mut set = ProducerSet::new();

        // Register 3 producers at genesis
        let keypair_active = KeyPair::generate();
        let keypair_dormant = KeyPair::generate();
        let keypair_recent = KeyPair::generate();

        for (i, kp) in [&keypair_active, &keypair_dormant, &keypair_recent]
            .iter()
            .enumerate()
        {
            let info = ProducerInfo::new(
                *kp.public_key(),
                0,
                100_000_000_000,
                (Hash::ZERO, i as u32),
                0,
                BOND_UNIT,
            );
            set.register(info, 0).unwrap();
        }

        // After 4 years, check at height 4 * BLOCKS_PER_YEAR
        let current_height = 4 * BLOCKS_PER_YEAR;

        // Simulate activity patterns:
        // - keypair_active: produced block recently (within 1 week)
        // - keypair_dormant: no activity since genesis (Dormant)
        // - keypair_recent: inactive ~10 days ago (RecentlyInactive)
        if let Some(p) = set.get_by_pubkey_mut(keypair_active.public_key()) {
            // ~2 hours ago at 10s slots, still Active (within 1 week threshold)
            p.last_activity = current_height - 720;
        }
        if let Some(p) = set.get_by_pubkey_mut(keypair_dormant.public_key()) {
            p.last_activity = 0; // Genesis, way past threshold
        }
        if let Some(p) = set.get_by_pubkey_mut(keypair_recent.public_key()) {
            // ~10 days ago: INACTIVITY_THRESHOLD + ~3 days = between 1-2 weeks
            p.last_activity = current_height - (INACTIVITY_THRESHOLD + 25_000);
        }

        // Verify activity statuses
        let active_info = set.get_by_pubkey(keypair_active.public_key()).unwrap();
        let dormant_info = set.get_by_pubkey(keypair_dormant.public_key()).unwrap();
        let recent_info = set.get_by_pubkey(keypair_recent.public_key()).unwrap();

        assert_eq!(
            active_info.activity_status(current_height),
            ActivityStatus::Active
        );
        assert_eq!(
            dormant_info.activity_status(current_height),
            ActivityStatus::Dormant
        );
        assert_eq!(
            recent_info.activity_status(current_height),
            ActivityStatus::RecentlyInactive
        );

        // Only Active producer has governance power
        assert!(active_info.has_governance_power(current_height));
        assert!(!dormant_info.has_governance_power(current_height));
        assert!(!recent_info.has_governance_power(current_height));

        // Only Active producer counts for quorum
        assert!(active_info.counts_for_quorum(current_height));
        assert!(!dormant_info.counts_for_quorum(current_height));
        assert!(!recent_info.counts_for_quorum(current_height));

        // All 3 have weight (for reward distribution - not governance)
        // All registered at genesis, so after 4 years they all have weight 4 (max)
        assert_eq!(set.total_weight(current_height), 12); // 3 * 4 = 12

        // But for governance (effective weight), only 1 Active producer counts
        assert_eq!(set.governance_participant_count(current_height), 1);
        assert_eq!(set.total_effective_weight(current_height), 4); // Only active producer's weight (max)

        // Veto test: The active producer alone has 100% of governance weight
        // 40% of 4 = 1.6, rounded up = 2
        // Active producer has weight 4, so they CAN veto alone (4 >= 2)
        assert!(set.has_weighted_veto(&[*keypair_active.public_key()], current_height));

        // Dormant producer has no governance power, cannot veto even with high seniority
        assert!(!set.has_weighted_veto(&[*keypair_dormant.public_key()], current_height));

        // RecentlyInactive producer also has no governance power
        assert!(!set.has_weighted_veto(&[*keypair_recent.public_key()], current_height));

        // Key insight: "El silencio no bloquea"
        // Even if dormant + recent tried to veto together, they have 0 governance weight
        assert!(!set.has_weighted_veto(
            &[*keypair_dormant.public_key(), *keypair_recent.public_key()],
            current_height
        ));
    }

    #[test]
    fn test_cancel_exit() {
        let mut set = ProducerSet::new();
        let keypair = KeyPair::generate();
        let pubkey = *keypair.public_key();

        // Register producer
        let info = ProducerInfo::new(
            pubkey.clone(),
            0,
            100_000_000_000,
            (Hash::ZERO, 0),
            0,
            BOND_UNIT,
        );
        set.register(info, 0).unwrap();

        // Verify initially active
        assert!(set.get_by_pubkey(&pubkey).unwrap().is_active());
        assert_eq!(
            set.get_by_pubkey(&pubkey).unwrap().status,
            ProducerStatus::Active
        );

        // Cannot cancel exit when not in unbonding
        assert!(set.cancel_exit(&pubkey).is_err());

        // Request exit (start unbonding)
        set.request_exit(&pubkey, 1000).unwrap();
        assert!(matches!(
            set.get_by_pubkey(&pubkey).unwrap().status,
            ProducerStatus::Unbonding { started_at: 1000 }
        ));

        // Producer can still produce during unbonding
        assert!(set.get_by_pubkey(&pubkey).unwrap().can_produce());

        // Cancel exit - returns to Active
        set.cancel_exit(&pubkey).unwrap();
        assert_eq!(
            set.get_by_pubkey(&pubkey).unwrap().status,
            ProducerStatus::Active
        );

        // Can request exit again after canceling
        set.request_exit(&pubkey, 2000).unwrap();
        assert!(matches!(
            set.get_by_pubkey(&pubkey).unwrap().status,
            ProducerStatus::Unbonding { started_at: 2000 }
        ));

        // Cancel again
        set.cancel_exit(&pubkey).unwrap();
        assert_eq!(
            set.get_by_pubkey(&pubkey).unwrap().status,
            ProducerStatus::Active
        );

        // Cannot cancel exit after it's complete
        set.request_exit(&pubkey, 3000).unwrap();
        // Simulate unbonding completion (test uses 43,200 blocks as arbitrary duration)
        set.process_unbonding(3000 + 43_200 + 1, 43_200);

        // Now producer is Exited, cannot cancel
        assert!(set.cancel_exit(&pubkey).is_err());
    }

    #[test]
    fn test_cancel_exit_preserves_seniority() {
        let mut set = ProducerSet::new();
        let keypair = KeyPair::generate();
        let pubkey = *keypair.public_key();

        // Register producer at block 0
        let info = ProducerInfo::new(
            pubkey.clone(),
            0, // registered_at = 0
            100_000_000_000,
            (Hash::ZERO, 0),
            0,
            BOND_UNIT,
        );
        set.register(info, 0).unwrap();

        // After 4 years, check weight (discrete steps: 3+ years = weight 4)
        let height_4_years = 4 * BLOCKS_PER_YEAR;
        let weight_before = set.get_by_pubkey(&pubkey).unwrap().weight(height_4_years);
        assert_eq!(weight_before, 4); // 4 years = max weight

        // Request exit
        set.request_exit(&pubkey, height_4_years).unwrap();

        // Cancel exit
        set.cancel_exit(&pubkey).unwrap();

        // Weight should be preserved (registered_at unchanged)
        let weight_after = set.get_by_pubkey(&pubkey).unwrap().weight(height_4_years);
        assert_eq!(weight_after, weight_before);

        // Verify registered_at is still 0
        assert_eq!(set.get_by_pubkey(&pubkey).unwrap().registered_at, 0);
    }

    #[test]
    fn test_active_cache_basic() {
        let mut set = ProducerSet::new();
        let p1 = make_producer(0);
        let pubkey1 = p1.public_key;
        set.register(p1, 0).unwrap();

        let height = ACTIVATION_DELAY + 1;

        // Before caching: slow path works
        assert_eq!(set.active_producers_at_height(height).len(), 1);
        assert!(set.active_cache.is_none());

        // Build cache
        set.ensure_active_cache(height);
        assert!(set.active_cache.is_some());

        // Cached path returns same result
        assert_eq!(set.active_producers_at_height(height).len(), 1);
        assert_eq!(
            set.active_producers_at_height(height)[0].public_key,
            pubkey1
        );
    }

    #[test]
    fn test_active_cache_invalidation() {
        let mut set = ProducerSet::new();
        let p1 = make_producer(0);
        let pubkey1 = p1.public_key;
        set.register(p1, 0).unwrap();

        let height = ACTIVATION_DELAY + 1;
        set.ensure_active_cache(height);
        assert!(set.active_cache.is_some());

        // Register new producer → invalidates cache
        let p2 = make_producer(0);
        set.register(p2, 0).unwrap();
        assert!(set.active_cache.is_none());

        // Rebuild and verify both producers present
        set.ensure_active_cache(height);
        assert_eq!(set.active_producers_at_height(height).len(), 2);

        // Exit → invalidates cache
        set.request_exit(&pubkey1, height).unwrap();
        assert!(set.active_cache.is_none());
    }

    #[test]
    fn test_active_cache_height_change() {
        let mut set = ProducerSet::new();
        let p1 = make_producer(100);
        set.register(p1, 100).unwrap();

        // At height 105: producer NOT yet eligible (ACTIVATION_DELAY=10)
        set.ensure_active_cache(105);
        assert_eq!(set.active_producers_at_height(105).len(), 0);

        // Cache is valid at same height
        assert!(set.active_cache.is_some());

        // At height 111: producer IS eligible
        set.ensure_active_cache(111);
        assert_eq!(set.active_producers_at_height(111).len(), 1);

        // Different height → cache miss (uses slow path but still correct)
        assert_eq!(set.active_producers_at_height(200).len(), 1);
    }

    #[test]
    fn test_active_cache_slash_invalidation() {
        let mut set = ProducerSet::new();
        let p1 = make_producer(0);
        let pubkey1 = p1.public_key;
        set.register(p1, 0).unwrap();

        let height = ACTIVATION_DELAY + 1;
        set.ensure_active_cache(height);
        assert_eq!(set.active_producers_at_height(height).len(), 1);

        // Slash → invalidates cache
        set.slash_producer(&pubkey1, height).unwrap();
        assert!(set.active_cache.is_none());

        // No active producers after slash
        assert_eq!(set.active_producers_at_height(height).len(), 0);
    }
}
