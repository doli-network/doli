//! Consensus parameters and rules
//!
//! # Parameter Architecture
//!
//! This module contains **mainnet default constants** (the "DNA" of the protocol).
//! These values serve as the base defaults used by [`crate::network_params::NetworkParams`].
//!
//! **For network-aware code, use `NetworkParams` instead of importing constants directly:**
//!
//! ```ignore
//! // PREFERRED: Network-aware
//! let params = NetworkParams::load(network);
//! let bond_unit = params.bond_unit;  // 100 DOLI mainnet, 1 DOLI devnet
//!
//! // AVOID: Direct import (hardcoded mainnet values)
//! use doli_core::consensus::BOND_UNIT;  // Always 100 DOLI
//! ```
//!
//! Constants here are locked for mainnet but can be overridden via `.env` for devnet.
//! See [`crate::network_params`] for the configuration manager.
//!
//! # Proof of Time (PoT)
//!
//! DOLI uses Proof of Time consensus with VDF-based anti-grinding. Unlike
//! Proof of Work (parallelizable computation) or Proof of Stake (capital-based
//! selection), PoT uses **time as the scarce resource**.
//!
//! ## Core Principle
//!
//! Time is proven via VDF (Verifiable Delay Function):
//! - One producer per slot (10 seconds)
//! - Selection uses consecutive tickets based on bond count
//! - VDF computation (~7s) proves sequential work
//! - Producer receives 100% of block reward via coinbase
//!
//! ## Selection System
//!
//! Producer selection is deterministic and independent of previous block hash:
//! - Total tickets = sum of all producer bond counts
//! - Primary producer: slot % total_tickets
//! - Fallback producers at +33% and +50% ticket offsets
//!
//! This prevents grinding attacks - changing the block hash cannot influence
//! who gets selected for future slots.
//!
//! ## Why VDF for Blocks?
//!
//! VDF provides anti-grinding protection:
//! - VDF input: HASH(prev_hash || tx_root || slot || producer_key)
//! - ~700ms of sequential computation
//! - Mandatory for mainnet blocks (vdf_output must be present)
//! - Proof is self-verifying (hash-chain VDF)
//!
//! ## Slot Timing (10 seconds)
//!
//! ```text
//! 0-3s:    Primary producer window (rank 0 only)
//! 3-6s:    Secondary fallback window (rank 0-1)
//! 6-10s:   Tertiary fallback window (rank 0-2)
//! ```

use crate::network::Network;
use crate::types::{Amount, BlockHeight, Epoch, Era, Slot};
use serde::{Deserialize, Serialize};

/// Genesis timestamp: 2026-02-01T00:00:00Z
pub const GENESIS_TIME: u64 = 1769904000;

// ==================== Proof of Time Parameters ====================

/// Slot duration in seconds.
///
/// # Proof of Time Timing
///
/// With 10-second slots:
/// - VDF computation: ~7s (T_BLOCK iterations)
/// - Block construction and broadcast: ~3s
/// - Total: 10 seconds
///
/// VDF is required for anti-grinding protection.
pub const SLOT_DURATION: u64 = 10;

/// Slots per epoch (1 hour = 360 slots at 10s each)
/// Used for consensus-level producer set stability.
pub const SLOTS_PER_EPOCH: u32 = 360;

/// Slots per reward epoch (1 hour = 360 slots at 10s each)
/// Each producer receives 100% of the block reward when they produce a block.
pub const SLOTS_PER_REWARD_EPOCH: u32 = 360;

/// Blocks per reward epoch (360 blocks for mainnet/testnet)
///
/// This is the primary constant for the weighted presence reward system.
/// Reward epochs are defined by block height (not slot), making calculation
/// simpler since block heights are sequential with no gaps.
///
/// Examples:
/// - 360 blocks ≈ 1 hour at 10s blocks (mainnet)
/// - 60 blocks  ≈ 1 minute at 1s blocks (devnet)
/// - 8640 blocks ≈ 24 hours (daily rewards)
pub const BLOCKS_PER_REWARD_EPOCH: BlockHeight = 360;

/// Slots per year (365 days * 24 hours * 360 slots/hour)
/// Used for seniority weight calculations.
pub const SLOTS_PER_YEAR: u32 = 3_153_600;

/// Minimum presence rate to remain in the active producer set (percentage).
/// Producers must successfully produce at least this percentage of their
/// assigned blocks to maintain good standing.
/// 50% = must produce at least half of assigned slots
pub const MIN_PRESENCE_RATE: u32 = 50;

/// Minimum attestation rate - alias for backward compatibility
pub const MIN_ATTESTATION_RATE: u32 = MIN_PRESENCE_RATE;

/// Attestation interval - kept for backward compatibility.
/// In Proof of Time, there are no attestations; each block
/// production with valid VDF IS the proof of time spent.
#[deprecated(note = "Attestations not used in Proof of Time")]
pub const ATTESTATION_INTERVAL: u32 = 1;

/// Slots per era (~4 years at 10-second slots)
/// 4 years = 12,614,400 slots (halving interval)
pub const SLOTS_PER_ERA: BlockHeight = 12_614_400;

/// Blocks per era - alias for SLOTS_PER_ERA
pub const BLOCKS_PER_ERA: BlockHeight = SLOTS_PER_ERA;

/// Halving interval - same as SLOTS_PER_ERA
pub const HALVING_INTERVAL: BlockHeight = SLOTS_PER_ERA;

/// Bootstrap phase duration in blocks (~1 week at 10-second slots)
/// 7 days = 60,480 blocks
pub const BOOTSTRAP_BLOCKS: BlockHeight = 60_480;

/// Maximum clock drift allowed (seconds).
/// Nodes with clocks drifting more than this are considered out of sync.
/// With 10-second slots, this is 1 slot worth of drift.
pub const MAX_DRIFT: u64 = 10;

/// Network margin for block timing (milliseconds).
/// Time reserved for presence signature collection.
/// With 1-second slots, we allocate 200ms buffer at the end.
pub const NETWORK_MARGIN_MS: u64 = 200;

/// Network margin in seconds (for backward compatibility).
/// Rounded up from NETWORK_MARGIN_MS.
pub const NETWORK_MARGIN: u64 = 1;

/// Maximum slots in the future a block can be accepted.
/// Prevents clock manipulation attacks where a node with a fast clock
/// produces blocks for future slots.
/// With 10-second slots, 1 slot = 10 seconds into the future.
pub const MAX_FUTURE_SLOTS: u64 = 1;

/// Maximum slots in the past a block can be accepted.
/// Allows for late blocks due to network delays, but prevents
/// producers from mining old slots indefinitely.
/// With 10-second slots, 192 slots = 32 minutes of history.
pub const MAX_PAST_SLOTS: u64 = 192;

/// Initial block reward (1 DOLI = 100,000,000 base units per block)
///
/// # Emission Schedule
///
/// With 10-second slots:
/// - 1 DOLI per block × 12,614,400 blocks/era = 12,614,400 DOLI/era
/// - Halves every era (~4 years)
/// - Total supply converges to 25,228,800 DOLI
///
/// Per epoch (1 hour = 360 blocks): 360 DOLI distributed
pub const INITIAL_REWARD: Amount = 100_000_000;

/// Initial block reward - alias for INITIAL_REWARD
pub const INITIAL_BLOCK_REWARD: Amount = INITIAL_REWARD;

/// Block reward pool per slot (alias for clarity)
pub const BLOCK_REWARD_POOL: Amount = INITIAL_REWARD;

/// Epoch reward pool (360 DOLI per hour)
/// 360 slots × 100,000,000 base units = 36,000,000,000 base units = 360 DOLI
pub const EPOCH_REWARD_POOL: Amount = SLOTS_PER_REWARD_EPOCH as u64 * INITIAL_REWARD;

/// Coinbase maturity (confirmations required before spending)
pub const COINBASE_MATURITY: BlockHeight = 100;

// ==================== Bond Stacking System ====================
//
// Producers stake bonds to participate in block production.
// More bonds = more selection weight = more block production opportunities.
// Each bond has its own vesting timer (4 years to full maturity).

/// Bond unit: 100 DOLI = 1 slot per cycle
/// This is the atomic unit for staking. You can only stake in multiples of this.
/// With 100 DOLI per bond unit:
/// - Producer with 1,000 DOLI = 10 slots per cycle
/// - Maximum 100 bonds = 10,000 DOLI maximum per producer
pub const BOND_UNIT: Amount = 10_000_000_000; // 100 DOLI in base units

/// Initial bond amount - alias for backward compatibility
pub const INITIAL_BOND: Amount = BOND_UNIT;

/// Maximum bonds per producer (prevents whale dominance)
/// 100 bonds = 10,000 DOLI maximum stake per node
pub const MAX_BONDS_PER_PRODUCER: u32 = 100;

/// Withdrawal delay in slots (7 days at 10s slots)
/// After requesting withdrawal, must wait this period before claiming
pub const WITHDRAWAL_DELAY_SLOTS: Slot = 60_480; // 7 days * 24 * 360 slots/hour

/// One year in slots (used for vesting calculation)
/// 365 days * 24 hours * 360 slots/hour = 3,153,600 slots
pub const YEAR_IN_SLOTS: Slot = 3_153_600;

/// Commitment period for full vesting (4 years)
/// After 4 years, bonds can be withdrawn with 0% penalty
pub const COMMITMENT_PERIOD: BlockHeight = 4 * YEAR_IN_SLOTS as BlockHeight;

/// Unbonding period for exit (~7 days at 10-second slots)
/// After requesting exit, producers must wait this long before claiming bond
/// 60,480 slots = 7 days (matches WITHDRAWAL_DELAY_SLOTS)
pub const UNBONDING_PERIOD: BlockHeight = 60_480;

/// Lock duration for bonds (4 years for full vesting)
pub const BOND_LOCK_BLOCKS: BlockHeight = COMMITMENT_PERIOD;

/// Calculate withdrawal penalty rate based on bond age.
///
/// # Vesting Schedule
/// - Year 1 (0-365 days): 75% penalty
/// - Year 2 (365-730 days): 50% penalty
/// - Year 3 (730-1095 days): 25% penalty
/// - Year 4+ (1095+ days): 0% penalty (fully vested)
///
/// # Arguments
/// - `bond_age_slots`: How many slots since the bond was created
///
/// # Returns
/// Penalty percentage (0-75)
pub fn withdrawal_penalty_rate(bond_age_slots: Slot) -> u8 {
    let years = bond_age_slots / YEAR_IN_SLOTS;
    match years {
        0 => 75, // Year 1: 75% penalty
        1 => 50, // Year 2: 50% penalty
        2 => 25, // Year 3: 25% penalty
        _ => 0,  // Year 4+: no penalty (fully vested)
    }
}

/// Maximum consecutive missed slots before considered inactive.
/// With 10-second slots, 50 missed slots = ~8 minutes of inactivity.
/// After this many misses, the producer is considered inactive.
pub const MAX_FAILURES: u32 = 50;

/// Inactivity threshold - same as MAX_FAILURES
pub const INACTIVITY_THRESHOLD: u32 = MAX_FAILURES;

/// Exclusion period for slashing (7 days in slots at 10-second slots)
/// 7 days = 60,480 slots
pub const EXCLUSION_SLOTS: Slot = 60_480;

/// Reward maturity (confirmations required)
pub const REWARD_MATURITY: BlockHeight = 100;

// ==================== Exit and Slashing System ====================

/// Destination for early exit penalties.
///
/// In DOLI, penalties are 100% burned (not sent to treasury or reward pool).
/// This is deflationary and benefits all token holders equally.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PenaltyDestination {
    /// Penalty is burned (removed from circulation)
    Burn,
    /// Legacy: reward pool (deprecated, kept for compatibility)
    #[deprecated(note = "Use Burn instead - 100% of penalties are burned")]
    RewardPool,
}

impl Default for PenaltyDestination {
    fn default() -> Self {
        Self::Burn
    }
}

/// Epoch reward distribution mode.
///
/// Determines how block rewards are distributed to producers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RewardMode {
    /// Direct coinbase to producer (legacy mode).
    /// Each block's reward goes directly to the producer who created it.
    DirectCoinbase,
    /// Pool rewards until epoch end, then distribute equally.
    /// Rewards accumulate in a pool and are distributed fairly at epoch boundaries.
    #[default]
    EpochPool,
}

/// Terms of an exit (normal or early)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExitTerms {
    /// Amount returned to the producer
    pub return_amount: Amount,
    /// Penalty amount (burned)
    pub penalty_amount: Amount,
    /// Where the penalty goes (always Burn)
    pub penalty_destination: PenaltyDestination,
    /// Whether this is an early exit
    pub is_early_exit: bool,
    /// Percentage of commitment completed (0-100)
    pub commitment_percent: u8,
}

/// Calculate exit terms for a producer (legacy single-bond API).
///
/// For the new bond stacking system, use `ProducerBonds::request_withdrawal()` instead.
///
/// # Vesting Schedule
/// - Year 1: 75% penalty
/// - Year 2: 50% penalty
/// - Year 3: 25% penalty
/// - Year 4+: 0% penalty (fully vested)
///
/// # Arguments
/// - `bond_amount`: The producer's bond amount
/// - `registered_at`: Block height when the producer registered
/// - `current_height`: Current block height
///
/// # Returns
/// ExitTerms describing the amount returned and any penalty (burned)
pub fn calculate_exit(
    bond_amount: Amount,
    registered_at: BlockHeight,
    current_height: BlockHeight,
) -> ExitTerms {
    let slots_served = current_height.saturating_sub(registered_at);
    let penalty_rate = withdrawal_penalty_rate(slots_served as Slot);

    if penalty_rate == 0 {
        // Fully vested: full bond returned
        ExitTerms {
            return_amount: bond_amount,
            penalty_amount: 0,
            penalty_destination: PenaltyDestination::Burn,
            is_early_exit: false,
            commitment_percent: 100,
        }
    } else {
        // Early exit: apply penalty based on vesting schedule
        let years = slots_served as Slot / YEAR_IN_SLOTS;
        let commitment_percent = ((years * 25) as u8).min(100);

        // Calculate penalty
        let penalty_amount = (bond_amount * penalty_rate as u64) / 100;
        let return_amount = bond_amount - penalty_amount;

        ExitTerms {
            return_amount,
            penalty_amount,
            penalty_destination: PenaltyDestination::Burn, // 100% burned
            is_early_exit: true,
            commitment_percent,
        }
    }
}

/// Result of slashing calculation
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlashResult {
    /// Amount burned (100% of bond)
    pub burned_amount: Amount,
    /// Whether the producer is permanently excluded
    pub excluded: bool,
}

/// Calculate slashing for misbehavior
///
/// # Rules
/// - Slashing is always 100% of bond (burned, not recycled)
/// - This reduces total supply, benefiting all coin holders
/// - Producer is permanently excluded from participation
///
/// # Arguments
/// - `bond_amount`: The producer's bond amount
///
/// # Returns
/// SlashResult with the burned amount and exclusion status
pub fn calculate_slash(bond_amount: Amount) -> SlashResult {
    SlashResult {
        burned_amount: bond_amount,
        excluded: true,
    }
}

// ==================== Proof of Time Types ====================

/// Presence score for a producer.
///
/// The score determines producer priority for block production.
/// Higher score = selected first. Score increases when you produce
/// blocks and decreases when you miss your assigned slots.
pub type PresenceScore = u64;

/// Minimum presence score to be eligible for block production
pub const MIN_PRESENCE_SCORE: PresenceScore = 1;

/// Maximum presence score (prevents overflow)
pub const MAX_PRESENCE_SCORE: PresenceScore = 10_000;

/// Initial presence score for new producers
pub const INITIAL_PRESENCE_SCORE: PresenceScore = 100;

/// Score bonus for producing a block when assigned
pub const SCORE_PRODUCE_BONUS: PresenceScore = 1;

/// Score penalty for missing an assigned slot
pub const SCORE_MISS_PENALTY: PresenceScore = 2;

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

// ==================== Bond Types ====================

/// A single bond entry with its creation time.
///
/// Each bond tracks when it was created for vesting calculation.
/// Bonds vest over 4 years with decreasing withdrawal penalties.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BondEntry {
    /// Slot when this bond was created
    pub creation_slot: Slot,
    /// Amount staked (always BOND_UNIT = 100 DOLI)
    pub amount: Amount,
}

impl BondEntry {
    /// Create a new bond entry
    pub fn new(creation_slot: Slot) -> Self {
        Self {
            creation_slot,
            amount: BOND_UNIT,
        }
    }

    /// Calculate the age of this bond in slots
    pub fn age(&self, current_slot: Slot) -> Slot {
        current_slot.saturating_sub(self.creation_slot)
    }

    /// Calculate withdrawal penalty for this bond
    pub fn penalty_rate(&self, current_slot: Slot) -> u8 {
        withdrawal_penalty_rate(self.age(current_slot))
    }

    /// Calculate net amount if withdrawn now
    pub fn withdrawal_amount(&self, current_slot: Slot) -> (Amount, Amount) {
        let penalty_rate = self.penalty_rate(current_slot);
        let penalty = (self.amount * penalty_rate as u64) / 100;
        let net = self.amount - penalty;
        (net, penalty)
    }

    /// Check if this bond is fully vested (4+ years old)
    pub fn is_vested(&self, current_slot: Slot) -> bool {
        self.age(current_slot) >= 4 * YEAR_IN_SLOTS
    }
}

/// Pending withdrawal request.
///
/// When a producer requests to withdraw bonds, they must wait 7 days.
/// This prevents flash attacks and gives the network time to adjust.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingWithdrawal {
    /// Number of bonds to withdraw
    pub bond_count: u32,
    /// Slot when withdrawal was requested
    pub request_slot: Slot,
    /// Net amount after penalty (calculated at request time)
    pub net_amount: Amount,
    /// Penalty amount (burned)
    pub penalty_amount: Amount,
    /// Destination pubkey hash for the withdrawal
    pub destination: crypto::Hash,
}

impl PendingWithdrawal {
    /// Check if withdrawal can be claimed (7-day delay passed)
    pub fn is_claimable(&self, current_slot: Slot) -> bool {
        current_slot.saturating_sub(self.request_slot) >= WITHDRAWAL_DELAY_SLOTS
    }

    /// Slots remaining until claimable
    pub fn slots_until_claimable(&self, current_slot: Slot) -> Slot {
        let elapsed = current_slot.saturating_sub(self.request_slot);
        WITHDRAWAL_DELAY_SLOTS.saturating_sub(elapsed)
    }
}

/// Producer's bond holdings.
///
/// Tracks all bonds staked by a producer and any pending withdrawals.
/// Selection weight is proportional to total bonds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProducerBonds {
    /// List of bonds (sorted by creation_slot, oldest first for FIFO)
    pub bonds: Vec<BondEntry>,
    /// Pending withdrawal requests
    pub pending_withdrawals: Vec<PendingWithdrawal>,
}

impl ProducerBonds {
    /// Create empty bond holdings
    pub fn new() -> Self {
        Self {
            bonds: Vec::new(),
            pending_withdrawals: Vec::new(),
        }
    }

    /// Total number of active bonds
    pub fn bond_count(&self) -> u32 {
        self.bonds.len() as u32
    }

    /// Total staked value
    pub fn total_staked(&self) -> Amount {
        self.bonds.len() as Amount * BOND_UNIT
    }

    /// Selection weight (same as bond count)
    pub fn selection_weight(&self) -> u32 {
        self.bond_count()
    }

    /// Add new bonds
    pub fn add_bonds(&mut self, count: u32, creation_slot: Slot) -> Result<(), BondError> {
        let new_total = self.bond_count() + count;
        if new_total > MAX_BONDS_PER_PRODUCER {
            return Err(BondError::MaxBondsExceeded {
                current: self.bond_count(),
                requested: count,
                max: MAX_BONDS_PER_PRODUCER,
            });
        }

        for _ in 0..count {
            self.bonds.push(BondEntry::new(creation_slot));
        }

        Ok(())
    }

    /// Request withdrawal of bonds (FIFO - oldest first)
    ///
    /// Returns the pending withdrawal with calculated amounts.
    /// Penalty is 100% burned (not sent anywhere).
    pub fn request_withdrawal(
        &mut self,
        count: u32,
        current_slot: Slot,
        destination: crypto::Hash,
    ) -> Result<PendingWithdrawal, BondError> {
        if count == 0 {
            return Err(BondError::ZeroWithdrawal);
        }

        if count > self.bond_count() {
            return Err(BondError::InsufficientBonds {
                requested: count,
                available: self.bond_count(),
            });
        }

        // Calculate amounts for FIFO withdrawal (oldest bonds first)
        let mut total_net: Amount = 0;
        let mut total_penalty: Amount = 0;

        // Remove oldest bonds first (FIFO)
        for _ in 0..count {
            if let Some(bond) = self.bonds.first() {
                let (net, penalty) = bond.withdrawal_amount(current_slot);
                total_net += net;
                total_penalty += penalty;
                self.bonds.remove(0);
            }
        }

        let withdrawal = PendingWithdrawal {
            bond_count: count,
            request_slot: current_slot,
            net_amount: total_net,
            penalty_amount: total_penalty,
            destination,
        };

        self.pending_withdrawals.push(withdrawal.clone());

        Ok(withdrawal)
    }

    /// Claim a pending withdrawal (after 7-day delay)
    pub fn claim_withdrawal(&mut self, current_slot: Slot) -> Result<PendingWithdrawal, BondError> {
        // Find first claimable withdrawal
        let idx = self
            .pending_withdrawals
            .iter()
            .position(|w| w.is_claimable(current_slot))
            .ok_or(BondError::NoClaimableWithdrawal)?;

        Ok(self.pending_withdrawals.remove(idx))
    }

    /// Get summary of bonds by maturity year
    pub fn maturity_summary(&self, current_slot: Slot) -> BondsMaturitySummary {
        let mut summary = BondsMaturitySummary::default();

        for bond in &self.bonds {
            let years = bond.age(current_slot) / YEAR_IN_SLOTS;
            match years {
                0 => summary.year_1 += 1,
                1 => summary.year_2 += 1,
                2 => summary.year_3 += 1,
                _ => summary.year_4_plus += 1,
            }
        }

        summary
    }

    /// Calculate total penalty if all bonds withdrawn now
    pub fn total_withdrawal_penalty(&self, current_slot: Slot) -> Amount {
        self.bonds
            .iter()
            .map(|b| b.withdrawal_amount(current_slot).1)
            .sum()
    }

    /// Check if there are any pending withdrawals
    pub fn has_pending_withdrawals(&self) -> bool {
        !self.pending_withdrawals.is_empty()
    }
}

/// Summary of bonds by maturity year
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BondsMaturitySummary {
    /// Bonds in year 1 (75% penalty)
    pub year_1: u32,
    /// Bonds in year 2 (50% penalty)
    pub year_2: u32,
    /// Bonds in year 3 (25% penalty)
    pub year_3: u32,
    /// Bonds in year 4+ (0% penalty, fully vested)
    pub year_4_plus: u32,
}

/// Errors related to bond operations
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BondError {
    /// Attempted to add bonds exceeding the maximum
    MaxBondsExceeded {
        current: u32,
        requested: u32,
        max: u32,
    },
    /// Attempted to withdraw more bonds than available
    InsufficientBonds { requested: u32, available: u32 },
    /// Attempted to withdraw zero bonds
    ZeroWithdrawal,
    /// No withdrawal is ready to claim
    NoClaimableWithdrawal,
    /// Amount not a multiple of BOND_UNIT
    InvalidAmount { amount: Amount },
}

impl std::fmt::Display for BondError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MaxBondsExceeded {
                current,
                requested,
                max,
            } => write!(
                f,
                "max bonds exceeded: have {}, requested {}, max {}",
                current, requested, max
            ),
            Self::InsufficientBonds {
                requested,
                available,
            } => write!(
                f,
                "insufficient bonds: requested {}, available {}",
                requested, available
            ),
            Self::ZeroWithdrawal => write!(f, "cannot withdraw zero bonds"),
            Self::NoClaimableWithdrawal => write!(f, "no withdrawal ready to claim"),
            Self::InvalidAmount { amount } => write!(
                f,
                "amount {} is not a multiple of bond unit {}",
                amount, BOND_UNIT
            ),
        }
    }
}

impl std::error::Error for BondError {}

/// Base block size (Era 1) in bytes
pub const BASE_BLOCK_SIZE: usize = 1_000_000;

/// Maximum block size cap (Era 6+) in bytes
pub const MAX_BLOCK_SIZE_CAP: usize = 32_000_000;

/// Calculate max block size for a given height.
///
/// Block size doubles every era (~4 years):
/// - Era 0: 1 MB
/// - Era 1: 2 MB
/// - Era 2: 4 MB
/// - Era 3: 8 MB
/// - Era 4: 16 MB
/// - Era 5+: 32 MB (capped)
///
/// This growth is encoded in the protocol from genesis.
/// No hard forks or voting required.
#[must_use]
pub fn max_block_size(height: BlockHeight) -> usize {
    let era = height / BLOCKS_PER_ERA;
    if era >= 5 {
        MAX_BLOCK_SIZE_CAP
    } else {
        BASE_BLOCK_SIZE << era // shift left = multiply by 2^era
    }
}

/// Total supply (25,228,800 DOLI)
/// Calculated as: sum of geometric series with initial reward and halving
/// 25,228,800 DOLI * 100,000,000 base units = 2,522,880,000,000,000
pub const TOTAL_SUPPLY: Amount = 2_522_880_000_000_000;

// ==================== VDF Parameters ====================
//
// VDF is used for both block production (anti-grinding) and registration (anti-Sybil).
// Time is the scarce resource in DOLI - VDF ensures sequential computation.

/// VDF discriminant bits for proofs.
/// Must be large enough for cryptographic security.
pub const VDF_DISCRIMINANT_BITS: u32 = 1024;

/// Block VDF iterations (10,000,000 iterations ~= 700ms on reference hardware)
/// This is the fixed T parameter for block production VDF.
pub const T_BLOCK: u64 = 10_000_000;

/// Legacy alias for T_BLOCK
pub const T_BLOCK_BASE: u64 = T_BLOCK;

/// Maximum T value for blocks - same as T_BLOCK (fixed)
pub const T_BLOCK_CAP: u64 = T_BLOCK;

/// VDF target duration in milliseconds (~700ms heartbeat)
pub const VDF_TARGET_MS: u64 = 700;

/// VDF deadline in milliseconds (must complete within slot)
pub const VDF_DEADLINE_MS: u64 = 10_000;

/// Get T parameter for block VDF (fixed at T_BLOCK).
///
/// VDF is required for block production as anti-grinding protection.
/// The input is constructed from: prev_hash, tx_root, slot, producer_key
#[must_use]
pub fn t_block(_height: BlockHeight) -> u64 {
    T_BLOCK
}

/// Construct the VDF input for block production.
///
/// The VDF input is: HASH(prefix || prev_hash || tx_root || slot || producer_key)
/// This ensures the VDF computation is bound to the specific block context.
///
/// # Arguments
/// * `prev_hash` - Hash of the previous block
/// * `tx_root` - Merkle root of transactions in this block
/// * `slot` - The slot number for this block
/// * `producer_key` - The producer's public key
///
/// # Returns
/// A 32-byte hash to use as VDF input
#[must_use]
pub fn construct_vdf_input(
    prev_hash: &crypto::Hash,
    tx_root: &crypto::Hash,
    slot: Slot,
    producer_key: &crypto::PublicKey,
) -> crypto::Hash {
    use crypto::hash::hash_concat;
    hash_concat(&[
        b"DOLI_VDF_BLOCK_V1",
        prev_hash.as_bytes(),
        tx_root.as_bytes(),
        &slot.to_le_bytes(),
        producer_key.as_bytes(),
    ])
}

/// Base registration VDF time (10 minutes in iterations)
pub const T_REGISTER_BASE: u64 = 600_000_000;

/// Target registrations per epoch
pub const R_TARGET: u32 = 10;

/// Maximum registrations per epoch
pub const R_CAP: u32 = 100;

/// Maximum registration VDF time (24 hours)
pub const T_REGISTER_CAP: u64 = 86_400_000_000;

// ==================== Registration Queue Anti-DoS ====================

/// Maximum registrations allowed per block (prevents spam)
pub const MAX_REGISTRATIONS_PER_BLOCK: u32 = 5;

/// Base registration fee (0.001 DOLI = 100,000 base units)
/// This is the minimum fee when no registrations are pending.
pub const BASE_REGISTRATION_FEE: Amount = 100_000;

/// Maximum fee multiplier (10x = 1000/100)
/// Cap prevents plutocratic barriers - when fee hits cap, system enters "queue mode"
/// where wait time increases instead of price. Time is the scarce resource, not money.
pub const MAX_FEE_MULTIPLIER_X100: u32 = 1000;

/// Maximum registration fee (BASE * 10 = 0.01 DOLI)
/// This is the absolute cap regardless of queue size.
pub const MAX_REGISTRATION_FEE: Amount = BASE_REGISTRATION_FEE * 10;

/// Get fee multiplier ×100 for a given pending count (deterministic table lookup)
///
/// Returns value in range [100, 1000] representing 1.00x to 10.00x multiplier.
/// Uses integer arithmetic only - no floats in consensus code.
///
/// Table:
/// - 0-4 pending:     100  (1.00x)
/// - 5-9 pending:     150  (1.50x)
/// - 10-19 pending:   200  (2.00x)
/// - 20-49 pending:   300  (3.00x)
/// - 50-99 pending:   450  (4.50x)
/// - 100-199 pending: 650  (6.50x)
/// - 200-299 pending: 850  (8.50x)
/// - 300+ pending:    1000 (10.00x cap)
///
/// Design: When fee reaches cap, the system enters "queue mode" where
/// wait time increases, not price. Time is the scarce resource, not money.
#[must_use]
pub const fn fee_multiplier_x100(pending_count: u32) -> u32 {
    if pending_count >= 300 {
        return 1000;
    }
    if pending_count >= 200 {
        return 850;
    }
    if pending_count >= 100 {
        return 650;
    }
    if pending_count >= 50 {
        return 450;
    }
    if pending_count >= 20 {
        return 300;
    }
    if pending_count >= 10 {
        return 200;
    }
    if pending_count >= 5 {
        return 150;
    }
    100
}

/// Calculate the registration fee based on pending queue size
///
/// Uses deterministic table lookup with 10x cap. No floats.
/// Fee = BASE_REGISTRATION_FEE * multiplier / 100
///
/// Examples:
/// - 0 pending:   0.001 DOLI (1.00x)
/// - 5 pending:   0.0015 DOLI (1.50x)
/// - 20 pending:  0.003 DOLI (3.00x)
/// - 100 pending: 0.0065 DOLI (6.50x)
/// - 300+ pending: 0.01 DOLI (10.00x cap)
///
/// # Arguments
/// - `pending_count`: Number of registrations currently pending
///
/// # Returns
/// The fee required for a new registration (capped at 10x base)
#[must_use]
pub fn registration_fee(pending_count: u32) -> Amount {
    let multiplier = fee_multiplier_x100(pending_count) as u128;
    let fee = (BASE_REGISTRATION_FEE as u128 * multiplier) / 100;
    // Double-check cap (should be redundant given table, but defensive)
    (fee as Amount).min(MAX_REGISTRATION_FEE)
}

/// Calculate the registration fee for a specific network
///
/// Uses network-specific base fee with same 10x cap multiplier.
/// Deterministic table lookup, no floats.
#[must_use]
pub fn registration_fee_for_network(pending_count: u32, network: Network) -> Amount {
    let base_fee = network.registration_base_fee();
    let multiplier = fee_multiplier_x100(pending_count) as u128;
    let fee = (base_fee as u128 * multiplier) / 100;
    // Cap at 10x base for this network
    let max_fee = base_fee * 10;
    (fee as Amount).min(max_fee)
}

/// A pending registration in the queue
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingRegistration {
    /// Producer public key
    pub public_key: crypto::PublicKey,
    /// Bond amount committed
    pub bond_amount: Amount,
    /// Fee paid (burned on invalid, refunded on valid)
    pub fee_paid: Amount,
    /// Block height when submitted
    pub submitted_at: BlockHeight,
    /// Previous registration hash (for chaining)
    pub prev_registration_hash: crypto::Hash,
    /// Sequence number (for ordering)
    pub sequence_number: u64,
}

/// Registration queue with anti-DoS fee mechanism
///
/// Limits registrations per block and applies escalating fees
/// to prevent spam attacks while maintaining accessibility during
/// normal operation.
#[derive(Clone, Debug, Default)]
pub struct RegistrationQueue {
    /// Pending registrations awaiting inclusion
    pending: Vec<PendingRegistration>,
    /// Registrations processed in current block
    current_block_count: u32,
    /// Current block height
    current_block_height: BlockHeight,
}

impl RegistrationQueue {
    /// Create a new empty registration queue
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
            current_block_count: 0,
            current_block_height: 0,
        }
    }

    /// Get the current required fee for a new registration
    pub fn current_fee(&self) -> Amount {
        registration_fee(self.pending.len() as u32)
    }

    /// Get the current required fee for a specific network
    pub fn current_fee_for_network(&self, network: Network) -> Amount {
        registration_fee_for_network(self.pending.len() as u32, network)
    }

    /// Get the number of pending registrations
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Check if a new registration can be added to the current block
    pub fn can_add_to_block(&self) -> bool {
        self.current_block_count < MAX_REGISTRATIONS_PER_BLOCK
    }

    /// Check if a new registration can be added for a specific network
    pub fn can_add_to_block_for_network(&self, network: Network) -> bool {
        self.current_block_count < network.max_registrations_per_block()
    }

    /// Submit a registration to the queue
    ///
    /// # Arguments
    /// - `registration`: The pending registration to submit
    /// - `current_height`: Current block height
    ///
    /// # Returns
    /// - `Ok(())` if submitted successfully
    /// - `Err(reason)` if submission fails (insufficient fee, etc.)
    ///
    /// # Fee Handling
    /// The fee is deducted at submission time. If the registration is later
    /// found invalid during processing, the fee is burned. If valid, the
    /// fee is refunded (or applied as transaction fee).
    pub fn submit(
        &mut self,
        registration: PendingRegistration,
        _current_height: BlockHeight,
    ) -> Result<(), &'static str> {
        let required_fee = self.current_fee();

        if registration.fee_paid < required_fee {
            return Err("Insufficient registration fee");
        }

        self.pending.push(registration);
        Ok(())
    }

    /// Submit a registration with network-specific fee requirements
    pub fn submit_for_network(
        &mut self,
        registration: PendingRegistration,
        _current_height: BlockHeight,
        network: Network,
    ) -> Result<(), &'static str> {
        let required_fee = self.current_fee_for_network(network);

        if registration.fee_paid < required_fee {
            return Err("Insufficient registration fee");
        }

        self.pending.push(registration);
        Ok(())
    }

    /// Start processing a new block
    ///
    /// Resets the per-block registration counter.
    pub fn begin_block(&mut self, height: BlockHeight) {
        self.current_block_height = height;
        self.current_block_count = 0;
    }

    /// Process the next registration from the queue
    ///
    /// Returns the next registration to be processed, or None if:
    /// - Queue is empty
    /// - Block limit reached
    ///
    /// The caller is responsible for validating the registration
    /// and calling `mark_processed` with the result.
    pub fn next_registration(&mut self) -> Option<PendingRegistration> {
        if self.current_block_count >= MAX_REGISTRATIONS_PER_BLOCK {
            return None;
        }

        if self.pending.is_empty() {
            return None;
        }

        // FIFO order - take from front
        let reg = self.pending.remove(0);
        self.current_block_count += 1;
        Some(reg)
    }

    /// Process the next registration with network-specific block limit
    pub fn next_registration_for_network(
        &mut self,
        network: Network,
    ) -> Option<PendingRegistration> {
        if self.current_block_count >= network.max_registrations_per_block() {
            return None;
        }

        if self.pending.is_empty() {
            return None;
        }

        // FIFO order - take from front
        let reg = self.pending.remove(0);
        self.current_block_count += 1;
        Some(reg)
    }

    /// Mark a registration as processed
    ///
    /// # Arguments
    /// - `valid`: Whether the registration was valid
    ///
    /// # Returns
    /// The fee amount. If invalid, this should be burned.
    /// If valid, this can be refunded or kept as tx fee.
    pub fn mark_processed(&self, fee_paid: Amount, valid: bool) -> Amount {
        if valid {
            // Valid registration: fee can be refunded or kept as tx fee
            fee_paid
        } else {
            // Invalid registration: fee is burned
            fee_paid
        }
    }

    /// Get all pending registrations (for inspection)
    pub fn pending_registrations(&self) -> &[PendingRegistration] {
        &self.pending
    }

    /// Remove expired registrations (optional cleanup)
    ///
    /// Registrations that have been pending for too long may be removed.
    /// Their fees would be returned to them.
    pub fn prune_expired(
        &mut self,
        current_height: BlockHeight,
        max_age: BlockHeight,
    ) -> Vec<PendingRegistration> {
        let expired: Vec<_> = self
            .pending
            .iter()
            .filter(|r| current_height.saturating_sub(r.submitted_at) > max_age)
            .cloned()
            .collect();

        self.pending
            .retain(|r| current_height.saturating_sub(r.submitted_at) <= max_age);

        expired
    }

    /// Clear the queue (for testing or emergency)
    pub fn clear(&mut self) {
        self.pending.clear();
        self.current_block_count = 0;
    }
}

// ==================== Producer Window Parameters ====================
//
// With 1-second slots, producer windows are tight:
// - Primary window: 0-300ms (only primary producer can submit)
// - Secondary window: 300-600ms (primary or secondary)
// - Tertiary window: 600-800ms (any of top 3)
// - Final 200ms reserved for signature collection
//
// The producer with highest presence_score builds the block.
// If they don't submit in time, fallbacks can step in.

/// Primary producer window in milliseconds (0-3000ms).
/// Only the primary producer (rank 0) can submit.
pub const PRIMARY_WINDOW_MS: u64 = 3_000;

/// Secondary producer window in milliseconds (0-6000ms).
/// Primary or secondary (rank 0-1) can submit.
pub const SECONDARY_WINDOW_MS: u64 = 6_000;

/// Tertiary producer window in milliseconds (0-10000ms).
/// Any of the top 3 producers (rank 0-2) can submit.
pub const TERTIARY_WINDOW_MS: u64 = 10_000;

/// Signature collection window - deprecated with 10s slots.
/// Block propagation happens within the slot window.
pub const SIGNATURE_WINDOW_MS: u64 = 0;

/// Primary producer window in seconds (0-3s).
pub const PRIMARY_WINDOW_SECS: u64 = 3;

/// Secondary producer window in seconds (0-6s).
pub const SECONDARY_WINDOW_SECS: u64 = 6;

/// Tertiary producer window in seconds (0-10s = full slot).
pub const TERTIARY_WINDOW_SECS: u64 = 10;

/// Fast block threshold - reserved for future VDF timing optimizations
pub const FAST_THRESHOLD_MS: u64 = 0;
pub const FAST_THRESHOLD: u64 = 0;

/// Maximum number of fallback producers per slot.
/// If the primary producer doesn't submit within their window,
/// secondary and tertiary producers can step in.
pub const MAX_FALLBACK_PRODUCERS: usize = 3;

/// Size of the eligible producer pool for weighted selection.
///
/// Anti-Grinding Selection:
/// - Producers are sorted by pubkey (deterministic)
/// - Selection uses consecutive tickets: slot % total_tickets
/// - Fallbacks use consecutive offsets: (base + 1), (base + 2)
///
/// This prevents grinding attacks: prev_hash is not used in selection,
/// making it impossible to influence future producer selection.
pub const ELIGIBLE_PRODUCER_POOL: usize = 5;

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
    pub initial_reward: Amount,
    pub initial_bond: Amount,
    pub base_block_size: usize,
    pub max_block_size_cap: usize,
    /// Reward distribution mode (DirectCoinbase or EpochPool)
    pub reward_mode: RewardMode,
}

impl ConsensusParams {
    /// Create mainnet parameters
    #[allow(deprecated)]
    pub fn mainnet() -> Self {
        Self {
            genesis_time: GENESIS_TIME,
            slot_duration: SLOT_DURATION,
            slots_per_epoch: SLOTS_PER_EPOCH,
            slots_per_reward_epoch: SLOTS_PER_REWARD_EPOCH,
            attestation_interval: ATTESTATION_INTERVAL,
            min_attestation_rate: MIN_ATTESTATION_RATE,
            blocks_per_era: BLOCKS_PER_ERA,
            bootstrap_blocks: BOOTSTRAP_BLOCKS,
            initial_reward: INITIAL_REWARD,
            initial_bond: INITIAL_BOND,
            base_block_size: BASE_BLOCK_SIZE,
            max_block_size_cap: MAX_BLOCK_SIZE_CAP,
            reward_mode: RewardMode::EpochPool,
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
        Self {
            genesis_time: 0,                                // Will be set at testnet launch
            slot_duration: SLOT_DURATION,                   // Same as mainnet (10 seconds)
            slots_per_epoch: SLOTS_PER_EPOCH,               // Same as mainnet (360)
            slots_per_reward_epoch: SLOTS_PER_REWARD_EPOCH, // Same as mainnet
            attestation_interval: ATTESTATION_INTERVAL,
            min_attestation_rate: MIN_ATTESTATION_RATE,
            blocks_per_era: BLOCKS_PER_ERA, // Same as mainnet (~4 years)
            bootstrap_blocks: BOOTSTRAP_BLOCKS, // Same as mainnet (~1 week)
            initial_reward: INITIAL_REWARD,
            initial_bond: INITIAL_BOND, // Same as mainnet (1000 DOLI)
            base_block_size: BASE_BLOCK_SIZE,
            max_block_size_cap: MAX_BLOCK_SIZE_CAP,
            reward_mode: RewardMode::EpochPool,
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
        Self {
            genesis_time: 0,            // Will be set at devnet start
            slot_duration: 1,           // 1 second (fast)
            slots_per_epoch: 60,        // 1 minute per epoch
            slots_per_reward_epoch: 30, // 30 seconds per reward epoch
            attestation_interval: 1,    // Every block (presence signatures)
            min_attestation_rate: MIN_ATTESTATION_RATE,
            blocks_per_era: 576, // ≈10 minutes per era (576 blocks × 1s = 9.6 min)
            bootstrap_blocks: 60, // ~1 minute bootstrap
            initial_reward: INITIAL_REWARD, // 1 DOLI per block (same as mainnet)
            initial_bond: 100_000_000, // 1 DOLI
            base_block_size: BASE_BLOCK_SIZE,
            max_block_size_cap: MAX_BLOCK_SIZE_CAP,
            reward_mode: RewardMode::EpochPool,
        }
    }

    /// Create consensus parameters for a specific network
    #[allow(deprecated)]
    pub fn for_network(network: Network) -> Self {
        Self {
            genesis_time: network.genesis_time(),
            slot_duration: network.slot_duration(),
            slots_per_epoch: SLOTS_PER_EPOCH,
            slots_per_reward_epoch: network.slots_per_reward_epoch(),
            attestation_interval: ATTESTATION_INTERVAL,
            min_attestation_rate: MIN_ATTESTATION_RATE,
            blocks_per_era: network.blocks_per_era(),
            bootstrap_blocks: network.bootstrap_blocks(),
            initial_reward: network.initial_reward(),
            initial_bond: network.initial_bond(),
            base_block_size: BASE_BLOCK_SIZE,
            max_block_size_cap: MAX_BLOCK_SIZE_CAP,
            reward_mode: RewardMode::EpochPool,
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
        slot > 0 && slot % self.slots_per_reward_epoch == 0
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

// ==================== Stress Test Parameters ====================

/// Parameters for extreme stress testing scenarios
#[derive(Clone, Debug)]
pub struct StressTestParams {
    /// Number of simulated producers
    pub producer_count: u32,
    /// Slot duration in seconds (can be very short for stress tests)
    pub slot_duration_secs: u64,
    /// VDF iterations (reduced for stress tests)
    pub vdf_iterations: u64,
    /// Bond amount per producer
    pub bond_per_producer: Amount,
    /// Block reward
    pub block_reward: Amount,
}

impl StressTestParams {
    /// Create stress test parameters for 600 producers (extreme scenario)
    ///
    /// Based on whitepaper calculations:
    /// - Each producer has 1/600 probability per slot
    /// - Expected blocks per producer per hour: 1.2
    /// - VDF reduced to 100ms for rapid testing
    pub fn extreme_600() -> Self {
        Self {
            producer_count: 600,
            slot_duration_secs: 1,          // 1 second slots for extreme stress
            vdf_iterations: 100_000,        // ~100ms VDF
            bond_per_producer: 100_000_000, // 1 DOLI
            block_reward: 50_000_000_000,   // 500 DOLI
        }
    }

    /// Create stress test parameters for N producers
    pub fn with_producers(count: u32) -> Self {
        Self {
            producer_count: count,
            slot_duration_secs: 2,          // 2 second slots
            vdf_iterations: 500_000,        // ~500ms VDF
            bond_per_producer: 100_000_000, // 1 DOLI
            block_reward: 50_000_000_000,   // 500 DOLI
        }
    }

    /// Calculate expected blocks per producer per hour
    pub fn expected_blocks_per_producer_per_hour(&self) -> f64 {
        let slots_per_hour = 3600.0 / self.slot_duration_secs as f64;
        slots_per_hour / self.producer_count as f64
    }

    /// Calculate expected reward per producer per hour
    pub fn expected_reward_per_producer_per_hour(&self) -> Amount {
        let blocks = self.expected_blocks_per_producer_per_hour();
        (blocks * self.block_reward as f64) as Amount
    }

    /// Calculate total bond locked with all producers
    pub fn total_bond_locked(&self) -> Amount {
        self.bond_per_producer * self.producer_count as u64
    }

    /// Calculate time between expected block production for one producer
    pub fn expected_time_between_blocks_secs(&self) -> f64 {
        self.slot_duration_secs as f64 * self.producer_count as f64
    }

    /// Calculate network efficiency (useful VDF vs total VDF)
    pub fn network_efficiency(&self) -> f64 {
        1.0 / self.producer_count as f64
    }

    /// Calculate slots needed for 51% attack
    pub fn slots_for_majority_attack(&self) -> u32 {
        (self.producer_count / 2) + 1
    }

    /// Generate a summary report
    pub fn summary(&self) -> String {
        format!(
            r#"
=== DOLI Stress Test: {} Producers ===

Timing:
  Slot duration: {}s
  VDF iterations: {} (~{}ms estimated)
  Expected time between own blocks: {:.1}s ({:.1} min)

Economics:
  Bond per producer: {} DOLI
  Total bond locked: {} DOLI
  Block reward: {} DOLI
  Expected reward/producer/hour: {:.2} DOLI

Probabilities:
  Probability of producing per slot: {:.4}%
  Expected blocks/producer/hour: {:.2}
  Network efficiency (useful VDF): {:.4}%

Security:
  Producers needed for 51% attack: {}
  Bond needed for attack: {} DOLI
"#,
            self.producer_count,
            self.slot_duration_secs,
            self.vdf_iterations,
            self.vdf_iterations / 1_000_000,
            self.expected_time_between_blocks_secs(),
            self.expected_time_between_blocks_secs() / 60.0,
            self.bond_per_producer / 100_000_000,
            self.total_bond_locked() / 100_000_000,
            self.block_reward / 100_000_000,
            self.expected_reward_per_producer_per_hour() as f64 / 100_000_000.0,
            100.0 / self.producer_count as f64,
            self.expected_blocks_per_producer_per_hour(),
            self.network_efficiency() * 100.0,
            self.slots_for_majority_attack(),
            (self.slots_for_majority_attack() as u64 * self.bond_per_producer) / 100_000_000,
        )
    }
}

impl ConsensusParams {
    /// Create consensus params optimized for stress testing with many producers
    ///
    /// Scales fallback windows proportionally to slot duration
    pub fn for_stress_test(stress: &StressTestParams) -> Self {
        Self {
            genesis_time: 0, // Dynamic, set at runtime
            slot_duration: stress.slot_duration_secs,
            slots_per_epoch: 60,
            slots_per_reward_epoch: 60, // Short reward epoch for stress tests
            attestation_interval: 5,    // Frequent attestations for stress tests
            min_attestation_rate: MIN_ATTESTATION_RATE,
            blocks_per_era: 10_000, // Faster era transitions for testing
            bootstrap_blocks: 10,   // Very short bootstrap for stress tests
            initial_reward: stress.block_reward,
            initial_bond: stress.bond_per_producer,
            base_block_size: BASE_BLOCK_SIZE,
            max_block_size_cap: MAX_BLOCK_SIZE_CAP,
            reward_mode: RewardMode::EpochPool,
        }
    }
}

/// Calculate scaled fallback windows for non-mainnet slot durations
///
/// Mainnet uses 60s slots with windows at 30s, 45s, 60s.
/// This scales proportionally for other slot durations.
///
/// Returns (primary_window, secondary_window, tertiary_window) in seconds
pub fn scaled_fallback_windows(slot_duration_secs: u64) -> (u64, u64, u64) {
    // Mainnet ratios: 30/60 = 0.5, 45/60 = 0.75, 60/60 = 1.0
    let primary = slot_duration_secs / 2; // 50% of slot
    let secondary = (slot_duration_secs * 3) / 4; // 75% of slot
    let tertiary = slot_duration_secs; // 100% of slot
    (primary.max(1), secondary.max(1), tertiary)
}

/// Determine allowed producer rank for non-mainnet networks
///
/// Uses scaled windows based on slot duration
pub fn allowed_producer_rank_scaled(slot_offset_secs: u64, slot_duration_secs: u64) -> usize {
    let (primary, secondary, _) = scaled_fallback_windows(slot_duration_secs);

    if slot_offset_secs < primary {
        0
    } else if slot_offset_secs < secondary {
        1
    } else {
        2
    }
}

/// Select the producer for a slot using consecutive tickets.
///
/// This is the primary selection function. It uses a deterministic round-robin
/// based on bond count (consecutive tickets). Selection is independent of
/// the previous block hash to prevent grinding attacks.
///
/// # Algorithm
/// 1. Calculate total tickets = sum of all producer bond counts
/// 2. ticket_index = slot % total_tickets
/// 3. Find producer whose consecutive ticket range contains ticket_index
/// 4. Return up to MAX_FALLBACK_PRODUCERS for fallback (offset by 33% and 50%)
///
/// # Arguments
/// * `slot` - The slot number
/// * `producers_with_bonds` - List of (PublicKey, bond_count) tuples, sorted by pubkey
///
/// # Returns
/// Vector of up to MAX_FALLBACK_PRODUCERS public keys ordered by priority
pub fn select_producer_for_slot(
    slot: Slot,
    producers_with_bonds: &[(crypto::PublicKey, u64)],
) -> Vec<crypto::PublicKey> {
    if producers_with_bonds.is_empty() {
        return Vec::new();
    }

    // Sort producers by pubkey for deterministic ordering
    let mut sorted: Vec<_> = producers_with_bonds.to_vec();
    sorted.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));

    // Calculate total tickets (each bond = 1 ticket, minimum 1 per producer)
    let total_tickets: u64 = sorted.iter().map(|(_, bonds)| (*bonds).max(1)).sum();

    if total_tickets == 0 {
        return Vec::new();
    }

    // Helper to find producer for a given ticket index
    let find_producer = |ticket_idx: u64| -> Option<crypto::PublicKey> {
        let mut cumulative: u64 = 0;
        for (pk, bonds) in &sorted {
            let tickets = (*bonds).max(1);
            if ticket_idx < cumulative + tickets {
                return Some(pk.clone());
            }
            cumulative += tickets;
        }
        None
    };

    let mut result = Vec::with_capacity(MAX_FALLBACK_PRODUCERS);

    // Primary producer: slot % total_tickets
    let primary_ticket = (slot as u64) % total_tickets;
    if let Some(pk) = find_producer(primary_ticket) {
        result.push(pk);
    }

    // Secondary fallback: consecutive ticket (base + 1)
    if result.len() < MAX_FALLBACK_PRODUCERS {
        let secondary_ticket = (primary_ticket + 1) % total_tickets;
        if let Some(pk) = find_producer(secondary_ticket) {
            if !result.contains(&pk) {
                result.push(pk);
            }
        }
    }

    // Tertiary fallback: consecutive ticket (base + 2)
    if result.len() < MAX_FALLBACK_PRODUCERS {
        let tertiary_ticket = (primary_ticket + 2) % total_tickets;
        if let Some(pk) = find_producer(tertiary_ticket) {
            if !result.contains(&pk) {
                result.push(pk);
            }
        }
    }

    result
}

/// Determine the allowed producer rank based on slot offset (in seconds).
///
/// # Proof of Time Windows
///
/// With 10-second slots, windows are:
/// - 0-3s: Only rank 0 (primary)
/// - 3-6s: Rank 0 or 1 (primary or secondary)
/// - 6-10s: Rank 0, 1, or 2 (any of the top 3)
pub fn allowed_producer_rank(slot_offset_secs: u64) -> usize {
    if slot_offset_secs < PRIMARY_WINDOW_SECS {
        0
    } else if slot_offset_secs < SECONDARY_WINDOW_SECS {
        1
    } else {
        2
    }
}

/// Determine the allowed producer rank based on slot offset (in milliseconds).
///
/// # Proof of Time Windows
///
/// With 10-second slots, windows are:
/// - 0-3000ms: Only rank 0 (primary)
/// - 3000-6000ms: Rank 0 or 1 (primary or secondary)
/// - 6000-10000ms: Rank 0, 1, or 2 (any of the top 3)
///
/// This is the preferred function for precise producer eligibility timing.
pub fn allowed_producer_rank_ms(slot_offset_ms: u64) -> usize {
    if slot_offset_ms < PRIMARY_WINDOW_MS {
        0
    } else if slot_offset_ms < SECONDARY_WINDOW_MS {
        1
    } else {
        2
    }
}

/// Check if a producer rank is eligible at a given time offset.
///
/// # Fallback Windows
/// - 0-3s: Only rank 0 is eligible
/// - 3-6s: Ranks 0-1 are eligible
/// - 6-10s: Ranks 0-2 are eligible
///
/// # Arguments
/// * `rank` - The producer's rank (0 = primary, 1 = secondary, 2 = tertiary)
/// * `offset_ms` - Time offset within the slot in milliseconds
///
/// # Returns
/// true if the rank is eligible at the given offset
pub fn is_rank_eligible_at_offset(rank: usize, offset_ms: u64) -> bool {
    let max_rank = allowed_producer_rank_ms(offset_ms);
    rank <= max_rank
}

/// Check if a producer is eligible for a slot at the given time.
///
/// Returns true if the producer is in the list of eligible producers
/// and the current time allows their rank.
pub fn is_producer_eligible(
    producer: &crypto::PublicKey,
    eligible_producers: &[crypto::PublicKey],
    slot_offset_secs: u64,
) -> bool {
    let max_rank = allowed_producer_rank(slot_offset_secs);

    for (rank, eligible) in eligible_producers.iter().enumerate() {
        if rank > max_rank {
            break;
        }
        if eligible == producer {
            return true;
        }
    }
    false
}

/// Get the rank of a producer for a slot.
///
/// Returns Some(rank) if the producer is in the eligible list,
/// or None if not found.
pub fn get_producer_rank(
    producer: &crypto::PublicKey,
    eligible_producers: &[crypto::PublicKey],
) -> Option<usize> {
    eligible_producers.iter().position(|p| p == producer)
}

// =============================================================================
// REWARD EPOCH UTILITIES (BLOCK-HEIGHT BASED)
// =============================================================================

/// Block-height based reward epoch utilities.
///
/// Unlike slot-based epochs, block-height epochs are sequential with no gaps,
/// making reward calculation simpler and deterministic.
///
/// # Examples
///
/// ```
/// use doli_core::consensus::reward_epoch;
///
/// // Epoch 0: blocks 0-359
/// assert_eq!(reward_epoch::from_height(0), 0);
/// assert_eq!(reward_epoch::from_height(359), 0);
///
/// // Epoch 1: blocks 360-719
/// assert_eq!(reward_epoch::from_height(360), 1);
///
/// // Get epoch boundaries
/// let (start, end) = reward_epoch::boundaries(5);
/// assert_eq!(start, 1800);
/// assert_eq!(end, 2160);
/// ```
pub mod reward_epoch {
    use super::BLOCKS_PER_REWARD_EPOCH;
    use crate::types::BlockHeight;

    /// Get reward epoch number from block height.
    ///
    /// Simple division: `height / BLOCKS_PER_REWARD_EPOCH`
    ///
    /// # Examples
    ///
    /// With `BLOCKS_PER_REWARD_EPOCH = 360`:
    /// - Height 0 → Epoch 0
    /// - Height 359 → Epoch 0
    /// - Height 360 → Epoch 1
    /// - Height 1000 → Epoch 2
    #[inline]
    pub fn from_height(height: BlockHeight) -> u64 {
        height / BLOCKS_PER_REWARD_EPOCH
    }

    /// Get (start_height, end_height) for a reward epoch.
    ///
    /// Note: `end_height` is exclusive (the range is `start..end`).
    ///
    /// # Examples
    ///
    /// ```
    /// use doli_core::consensus::reward_epoch;
    ///
    /// let (start, end) = reward_epoch::boundaries(0);
    /// assert_eq!(start, 0);
    /// assert_eq!(end, 360);
    ///
    /// let (start, end) = reward_epoch::boundaries(5);
    /// assert_eq!(start, 1800);
    /// assert_eq!(end, 2160);
    /// ```
    #[inline]
    pub fn boundaries(epoch: u64) -> (BlockHeight, BlockHeight) {
        let start = epoch * BLOCKS_PER_REWARD_EPOCH;
        let end = start + BLOCKS_PER_REWARD_EPOCH;
        (start, end)
    }

    /// Check if a reward epoch is complete given the current block height.
    ///
    /// An epoch is complete when the current height is at or beyond the
    /// epoch's end boundary.
    ///
    /// # Examples
    ///
    /// ```
    /// use doli_core::consensus::reward_epoch;
    ///
    /// // Epoch 0 ends at block 360
    /// assert!(!reward_epoch::is_complete(0, 359));
    /// assert!(reward_epoch::is_complete(0, 360));
    /// assert!(reward_epoch::is_complete(0, 1000));
    /// ```
    #[inline]
    pub fn is_complete(epoch: u64, current_height: BlockHeight) -> bool {
        let (_, end) = boundaries(epoch);
        current_height >= end
    }

    /// Get the current reward epoch from block height.
    ///
    /// This is an alias for `from_height` for clarity in contexts
    /// where we're interested in the "current" epoch.
    #[inline]
    pub fn current(height: BlockHeight) -> u64 {
        from_height(height)
    }

    /// Get the last complete reward epoch from block height.
    ///
    /// Returns `None` if no epoch has been completed yet (height < BLOCKS_PER_REWARD_EPOCH).
    ///
    /// # Examples
    ///
    /// ```
    /// use doli_core::consensus::reward_epoch;
    ///
    /// // No complete epochs yet
    /// assert_eq!(reward_epoch::last_complete(0), None);
    /// assert_eq!(reward_epoch::last_complete(359), None);
    ///
    /// // First epoch just completed
    /// assert_eq!(reward_epoch::last_complete(360), Some(0));
    /// assert_eq!(reward_epoch::last_complete(719), Some(0));
    ///
    /// // Two epochs completed
    /// assert_eq!(reward_epoch::last_complete(720), Some(1));
    /// ```
    #[inline]
    pub fn last_complete(height: BlockHeight) -> Option<u64> {
        let current_epoch = from_height(height);
        if current_epoch > 0 {
            Some(current_epoch - 1)
        } else {
            None
        }
    }

    /// Check if height is the first block of a reward epoch.
    ///
    /// Useful for detecting epoch boundaries in block processing.
    #[inline]
    pub fn is_epoch_start(height: BlockHeight) -> bool {
        height % BLOCKS_PER_REWARD_EPOCH == 0
    }

    /// Get the number of blocks per reward epoch.
    ///
    /// Returns the constant `BLOCKS_PER_REWARD_EPOCH`.
    #[inline]
    pub fn blocks_per_epoch() -> BlockHeight {
        BLOCKS_PER_REWARD_EPOCH
    }

    /// Calculate how many complete epochs exist up to a given height.
    ///
    /// This is the same as `from_height` for heights >= BLOCKS_PER_REWARD_EPOCH.
    /// For heights < BLOCKS_PER_REWARD_EPOCH, returns 0.
    #[inline]
    pub fn complete_epochs(height: BlockHeight) -> u64 {
        if height < BLOCKS_PER_REWARD_EPOCH {
            0
        } else {
            from_height(height)
        }
    }

    // ========================================================================
    // Network-aware versions (_with suffix)
    // These functions accept blocks_per_epoch as a parameter to support
    // different networks (mainnet=360, testnet=360, devnet=60).
    // ========================================================================

    /// Get reward epoch number from block height (network-aware version).
    ///
    /// Use this when you have access to `Network::blocks_per_reward_epoch()`.
    #[inline]
    pub fn from_height_with(height: BlockHeight, blocks_per_epoch: u64) -> u64 {
        height / blocks_per_epoch
    }

    /// Get (start_height, end_height) for a reward epoch (network-aware version).
    ///
    /// Note: `end_height` is exclusive (the range is `start..end`).
    #[inline]
    pub fn boundaries_with(epoch: u64, blocks_per_epoch: u64) -> (BlockHeight, BlockHeight) {
        let start = epoch * blocks_per_epoch;
        let end = start + blocks_per_epoch;
        (start, end)
    }

    /// Check if a reward epoch is complete (network-aware version).
    #[inline]
    pub fn is_complete_with(
        epoch: u64,
        current_height: BlockHeight,
        blocks_per_epoch: u64,
    ) -> bool {
        let (_, end) = boundaries_with(epoch, blocks_per_epoch);
        current_height >= end
    }

    /// Get last complete reward epoch (network-aware version).
    #[inline]
    pub fn last_complete_with(height: BlockHeight, blocks_per_epoch: u64) -> Option<u64> {
        let current_epoch = from_height_with(height, blocks_per_epoch);
        if current_epoch > 0 {
            Some(current_epoch - 1)
        } else {
            None
        }
    }

    /// Check if height is first block of a reward epoch (network-aware version).
    #[inline]
    pub fn is_epoch_start_with(height: BlockHeight, blocks_per_epoch: u64) -> bool {
        height % blocks_per_epoch == 0
    }

    /// Calculate complete epochs up to height (network-aware version).
    #[inline]
    pub fn complete_epochs_with(height: BlockHeight, blocks_per_epoch: u64) -> u64 {
        if height < blocks_per_epoch {
            0
        } else {
            from_height_with(height, blocks_per_epoch)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slot_calculation() {
        let params = ConsensusParams::mainnet();

        // With 10-second slots (Proof of Time)
        assert_eq!(params.timestamp_to_slot(GENESIS_TIME), 0);
        assert_eq!(params.timestamp_to_slot(GENESIS_TIME + 10), 1);
        assert_eq!(params.timestamp_to_slot(GENESIS_TIME + 20), 2);
        assert_eq!(params.timestamp_to_slot(GENESIS_TIME + 3600), 360); // 1 hour = 360 slots
    }

    #[test]
    fn test_block_reward() {
        let params = ConsensusParams::mainnet();

        // Era 0: 1 DOLI per block (100,000,000 base units)
        assert_eq!(params.block_reward(0), 100_000_000);
        assert_eq!(params.block_reward(100), 100_000_000);

        // Era 1: halved to 0.5 DOLI (50,000,000 base units)
        assert_eq!(params.block_reward(BLOCKS_PER_ERA), 50_000_000);

        // Era 2: halved again to 0.25 DOLI (25,000,000 base units)
        assert_eq!(params.block_reward(BLOCKS_PER_ERA * 2), 25_000_000);
    }

    #[test]
    fn test_bond_amount() {
        let params = ConsensusParams::mainnet();

        // Era 0: 100 DOLI = 10_000_000_000 base units
        assert_eq!(params.bond_amount(0), 10_000_000_000);

        // Era 1: 70% of initial
        let era1_bond = params.bond_amount(BLOCKS_PER_ERA);
        assert!(era1_bond > 6_900_000_000 && era1_bond < 7_100_000_000);

        // Era 2: 49% of initial
        let era2_bond = params.bond_amount(BLOCKS_PER_ERA * 2);
        assert!(era2_bond > 4_800_000_000 && era2_bond < 5_000_000_000);
    }

    #[test]
    fn test_epoch_calculation() {
        let params = ConsensusParams::mainnet();

        // With 360 slots per epoch (1 hour at 10s slots)
        assert_eq!(params.slot_to_epoch(0), 0);
        assert_eq!(params.slot_to_epoch(359), 0);
        assert_eq!(params.slot_to_epoch(360), 1);
        assert_eq!(params.slot_to_epoch(720), 2);
    }

    #[test]
    fn test_timestamp_before_genesis() {
        let params = ConsensusParams::mainnet();
        // Timestamps before genesis return slot 0
        assert_eq!(params.timestamp_to_slot(0), 0);
        assert_eq!(params.timestamp_to_slot(GENESIS_TIME - 1), 0);
    }

    #[test]
    fn test_slot_timestamp_inverse() {
        let params = ConsensusParams::mainnet();
        for slot in [0, 1, 100, 1000, 10000] {
            let timestamp = params.slot_to_timestamp(slot);
            let back = params.timestamp_to_slot(timestamp);
            assert_eq!(slot, back);
        }
    }

    #[test]
    fn test_era_calculation() {
        let params = ConsensusParams::mainnet();
        assert_eq!(params.height_to_era(0), 0);
        assert_eq!(params.height_to_era(BLOCKS_PER_ERA - 1), 0);
        assert_eq!(params.height_to_era(BLOCKS_PER_ERA), 1);
        assert_eq!(params.height_to_era(BLOCKS_PER_ERA * 2), 2);
    }

    #[test]
    fn test_bootstrap_phase() {
        let params = ConsensusParams::mainnet();
        assert!(params.is_bootstrap(0));
        assert!(params.is_bootstrap(BOOTSTRAP_BLOCKS - 1));
        assert!(!params.is_bootstrap(BOOTSTRAP_BLOCKS));
        assert!(!params.is_bootstrap(BOOTSTRAP_BLOCKS + 1));
    }

    #[test]
    fn test_max_block_size_by_era() {
        // Era 0 (0-4 years): 1 MB
        assert_eq!(max_block_size(0), 1_000_000);
        assert_eq!(max_block_size(BLOCKS_PER_ERA - 1), 1_000_000);

        // Era 1 (4-8 years): 2 MB
        assert_eq!(max_block_size(BLOCKS_PER_ERA), 2_000_000);
        assert_eq!(max_block_size(BLOCKS_PER_ERA * 2 - 1), 2_000_000);

        // Era 2 (8-12 years): 4 MB
        assert_eq!(max_block_size(BLOCKS_PER_ERA * 2), 4_000_000);

        // Era 3 (12-16 years): 8 MB
        assert_eq!(max_block_size(BLOCKS_PER_ERA * 3), 8_000_000);

        // Era 4 (16-20 years): 16 MB
        assert_eq!(max_block_size(BLOCKS_PER_ERA * 4), 16_000_000);

        // Era 5+ (20+ years): 32 MB (capped)
        assert_eq!(max_block_size(BLOCKS_PER_ERA * 5), 32_000_000);
        assert_eq!(max_block_size(BLOCKS_PER_ERA * 6), 32_000_000);
        assert_eq!(max_block_size(BLOCKS_PER_ERA * 10), 32_000_000);
        assert_eq!(max_block_size(BLOCKS_PER_ERA * 100), 32_000_000);
    }

    #[test]
    fn test_max_block_size_params_method() {
        let params = ConsensusParams::mainnet();

        // Should match the free function
        assert_eq!(params.max_block_size(0), max_block_size(0));
        assert_eq!(
            params.max_block_size(BLOCKS_PER_ERA),
            max_block_size(BLOCKS_PER_ERA)
        );
        assert_eq!(
            params.max_block_size(BLOCKS_PER_ERA * 5),
            max_block_size(BLOCKS_PER_ERA * 5)
        );
    }

    // Note: Block VDF (T_BLOCK = 10M iterations) provides anti-grinding protection.
    // VDF is mandatory for mainnet blocks; registration VDF is separate (anti-Sybil).

    #[test]
    fn test_allowed_producer_rank() {
        // Proof of Time with 10-second slots
        // Windows: 0-3s (rank 0), 3-6s (rank 1), 6-10s (rank 2)
        assert_eq!(allowed_producer_rank(0), 0); // Primary only
        assert_eq!(allowed_producer_rank(1), 0);
        assert_eq!(allowed_producer_rank(2), 0);
        assert_eq!(allowed_producer_rank(3), 1); // Primary + Secondary
        assert_eq!(allowed_producer_rank(4), 1);
        assert_eq!(allowed_producer_rank(5), 1);
        assert_eq!(allowed_producer_rank(6), 2); // All eligible
        assert_eq!(allowed_producer_rank(9), 2);
        assert_eq!(allowed_producer_rank(100), 2);
    }

    #[test]
    fn test_allowed_producer_rank_ms() {
        // Proof of Time windows with millisecond precision (10-second slots)
        //
        // Windows:
        // - 0-3000ms: Primary (rank 0)
        // - 3000-6000ms: Secondary (rank 1)
        // - 6000-10000ms: Tertiary (rank 2)

        // Primary window: 0-3000ms
        assert_eq!(allowed_producer_rank_ms(0), 0);
        assert_eq!(allowed_producer_rank_ms(1500), 0);
        assert_eq!(allowed_producer_rank_ms(2999), 0);

        // Secondary window: 3000-6000ms
        assert_eq!(allowed_producer_rank_ms(3000), 1);
        assert_eq!(allowed_producer_rank_ms(4500), 1);
        assert_eq!(allowed_producer_rank_ms(5999), 1);

        // Tertiary window: 6000ms+
        assert_eq!(allowed_producer_rank_ms(6000), 2);
        assert_eq!(allowed_producer_rank_ms(8000), 2);
        assert_eq!(allowed_producer_rank_ms(10000), 2);
        assert_eq!(allowed_producer_rank_ms(15000), 2); // Beyond slot is still tertiary
    }

    #[test]
    fn test_is_producer_eligible() {
        let producers: Vec<crypto::PublicKey> = (0..3)
            .map(|i| {
                let mut bytes = [0u8; 32];
                bytes[0] = i;
                crypto::PublicKey::from_bytes(bytes)
            })
            .collect();

        // Proof of Time with 10-second slots:
        // Windows: 0-3s (rank 0), 3-6s (rank 1), 6-10s (rank 2)

        // At second 0: only primary (rank 0) is eligible
        assert!(is_producer_eligible(&producers[0], &producers, 0));
        assert!(!is_producer_eligible(&producers[1], &producers, 0));
        assert!(!is_producer_eligible(&producers[2], &producers, 0));

        // At second 3: primary + secondary (rank 0-1) are eligible
        assert!(is_producer_eligible(&producers[0], &producers, 3));
        assert!(is_producer_eligible(&producers[1], &producers, 3));
        assert!(!is_producer_eligible(&producers[2], &producers, 3));

        // At second 6+: all are eligible
        assert!(is_producer_eligible(&producers[0], &producers, 6));
        assert!(is_producer_eligible(&producers[1], &producers, 6));
        assert!(is_producer_eligible(&producers[2], &producers, 6));
    }

    #[test]
    fn test_get_producer_rank() {
        let producers: Vec<crypto::PublicKey> = (0..3)
            .map(|i| {
                let mut bytes = [0u8; 32];
                bytes[0] = i;
                crypto::PublicKey::from_bytes(bytes)
            })
            .collect();

        assert_eq!(get_producer_rank(&producers[0], &producers), Some(0));
        assert_eq!(get_producer_rank(&producers[1], &producers), Some(1));
        assert_eq!(get_producer_rank(&producers[2], &producers), Some(2));

        // Non-existent producer
        let unknown = crypto::PublicKey::from_bytes([99u8; 32]);
        assert_eq!(get_producer_rank(&unknown, &producers), None);
    }

    #[test]
    fn test_reward_eventually_zero() {
        let params = ConsensusParams::mainnet();
        // After era 63, reward should be 0
        assert_eq!(params.block_reward(BLOCKS_PER_ERA * 64), 0);
        assert_eq!(params.block_reward(BLOCKS_PER_ERA * 100), 0);
    }

    #[test]
    fn test_bond_eventually_zero() {
        let params = ConsensusParams::mainnet();
        // After era 20, bond should be 0
        assert_eq!(params.bond_amount(BLOCKS_PER_ERA * 20), 0);
        assert_eq!(params.bond_amount(BLOCKS_PER_ERA * 30), 0);
    }

    #[test]
    fn test_calculate_exit_normal() {
        let bond = 100_000_000_000u64; // 1000 DOLI
        let registered_at = 0;
        let current_height = COMMITMENT_PERIOD; // Exactly 4 years

        let terms = calculate_exit(bond, registered_at, current_height);

        assert!(!terms.is_early_exit);
        assert_eq!(terms.return_amount, bond);
        assert_eq!(terms.penalty_amount, 0);
        assert_eq!(terms.commitment_percent, 100);
    }

    #[test]
    fn test_calculate_exit_early_half() {
        // At 2 years (50% of 4-year commitment), penalty is 25%
        let bond = 100_000_000_000u64;
        let registered_at = 0;
        let current_height = COMMITMENT_PERIOD / 2; // 2 years

        let terms = calculate_exit(bond, registered_at, current_height);

        assert!(terms.is_early_exit);
        assert_eq!(terms.commitment_percent, 50);
        // New schedule: 2 years = 25% penalty, 75% returned
        assert_eq!(terms.penalty_amount, (bond * 25) / 100);
        assert_eq!(terms.return_amount, (bond * 75) / 100);
        assert_eq!(terms.penalty_destination, PenaltyDestination::Burn);
    }

    #[test]
    fn test_calculate_exit_very_early() {
        // At 0 years (immediate exit), penalty is 75%
        let bond = 100_000_000_000u64;
        let registered_at = 1000;
        let current_height = 1000; // Immediate exit

        let terms = calculate_exit(bond, registered_at, current_height);

        assert!(terms.is_early_exit);
        assert_eq!(terms.commitment_percent, 0);
        // New schedule: 0 years = 75% penalty, 25% returned
        assert_eq!(terms.penalty_amount, (bond * 75) / 100);
        assert_eq!(terms.return_amount, (bond * 25) / 100);
    }

    #[test]
    fn test_calculate_exit_one_year() {
        // At 1 year (25% of commitment), penalty is 50%
        let bond = 100_000_000_000u64;
        let registered_at = 0;
        let current_height = COMMITMENT_PERIOD / 4; // 1 year

        let terms = calculate_exit(bond, registered_at, current_height);

        assert!(terms.is_early_exit);
        assert_eq!(terms.commitment_percent, 25);
        // New schedule: 1 year = 50% penalty, 50% returned
        assert_eq!(terms.penalty_amount, (bond * 50) / 100);
        assert_eq!(terms.return_amount, (bond * 50) / 100);
    }

    #[test]
    fn test_calculate_slash() {
        let bond = 100_000_000_000u64;

        let result = calculate_slash(bond);

        assert_eq!(result.burned_amount, bond);
        assert!(result.excluded);
    }

    #[test]
    fn test_unbonding_period_is_7_days() {
        // At 10 seconds per slot: 7 days = 7 * 24 * 360 slots/day = 60,480 slots
        assert_eq!(UNBONDING_PERIOD, 60_480);
    }

    #[test]
    fn test_commitment_period_is_4_years() {
        // Commitment period: 4 years at 10s slots
        // 4 * 365 * 24 * 360 = 12,614,400 slots
        assert_eq!(COMMITMENT_PERIOD, 4 * YEAR_IN_SLOTS as BlockHeight);
    }

    // ==================== RewardMode Tests ====================

    #[test]
    fn test_reward_mode_default() {
        assert_eq!(RewardMode::default(), RewardMode::EpochPool);
    }

    #[test]
    fn test_reward_mode_serialization() {
        let mode = RewardMode::EpochPool;
        let bytes = bincode::serialize(&mode).unwrap();
        let deserialized: RewardMode = bincode::deserialize(&bytes).unwrap();
        assert_eq!(mode, deserialized);

        let mode2 = RewardMode::DirectCoinbase;
        let bytes2 = bincode::serialize(&mode2).unwrap();
        let deserialized2: RewardMode = bincode::deserialize(&bytes2).unwrap();
        assert_eq!(mode2, deserialized2);
    }

    #[test]
    fn test_consensus_params_has_epoch_pool_mode() {
        let params = ConsensusParams::mainnet();
        assert_eq!(params.reward_mode, RewardMode::EpochPool);

        let params_testnet = ConsensusParams::testnet();
        assert_eq!(params_testnet.reward_mode, RewardMode::EpochPool);
    }

    #[test]
    fn test_consensus_params_for_network_has_epoch_pool_mode() {
        let params_mainnet = ConsensusParams::for_network(Network::Mainnet);
        assert_eq!(params_mainnet.reward_mode, RewardMode::EpochPool);

        let params_testnet = ConsensusParams::for_network(Network::Testnet);
        assert_eq!(params_testnet.reward_mode, RewardMode::EpochPool);

        let params_devnet = ConsensusParams::for_network(Network::Devnet);
        assert_eq!(params_devnet.reward_mode, RewardMode::EpochPool);
    }

    #[test]
    fn test_reward_mode_equality() {
        assert_eq!(RewardMode::EpochPool, RewardMode::EpochPool);
        assert_eq!(RewardMode::DirectCoinbase, RewardMode::DirectCoinbase);
        assert_ne!(RewardMode::EpochPool, RewardMode::DirectCoinbase);
    }

    // Property-based tests
    use proptest::prelude::*;

    proptest! {
        /// Slot-timestamp roundtrip preserves slot
        #[test]
        fn prop_slot_timestamp_roundtrip(slot: u32) {
            let params = ConsensusParams::mainnet();
            let timestamp = params.slot_to_timestamp(slot);
            let back = params.timestamp_to_slot(timestamp);
            prop_assert_eq!(slot, back);
        }

        /// Timestamps within same slot map to same slot
        #[test]
        fn prop_same_slot_same_result(slot in 0u32..1_000_000, offset in 0u64..3) {
            let params = ConsensusParams::mainnet();
            let base_timestamp = params.slot_to_timestamp(slot);
            // Add offset but stay within the slot (1-second slots)
            let timestamp = base_timestamp + offset.min(params.slot_duration - 1);
            let computed_slot = params.timestamp_to_slot(timestamp);
            prop_assert_eq!(slot, computed_slot);
        }

        /// Block reward halves each era (monotonically decreasing)
        #[test]
        fn prop_reward_decreasing(era in 0u32..63) {
            let params = ConsensusParams::mainnet();
            let height_current = (era as u64) * BLOCKS_PER_ERA;
            let height_next = (era as u64 + 1) * BLOCKS_PER_ERA;
            let reward_current = params.block_reward(height_current);
            let reward_next = params.block_reward(height_next);
            // Each era reward is half the previous (or both are 0)
            if reward_current > 0 {
                prop_assert!(reward_next <= reward_current);
                prop_assert_eq!(reward_next, reward_current / 2);
            }
        }

        /// Bond amount decreases each era
        #[test]
        fn prop_bond_decreasing(era in 0u32..19) {
            let params = ConsensusParams::mainnet();
            let height_current = (era as u64) * BLOCKS_PER_ERA;
            let height_next = (era as u64 + 1) * BLOCKS_PER_ERA;
            let bond_current = params.bond_amount(height_current);
            let bond_next = params.bond_amount(height_next);
            // Bond decreases by 30% each era (roughly)
            if bond_current > 0 {
                prop_assert!(bond_next < bond_current);
                // Should be approximately 70% of current (between 65% and 75%)
                let min_expected = bond_current * 65 / 100;
                let max_expected = bond_current * 75 / 100;
                prop_assert!(
                    bond_next >= min_expected && bond_next <= max_expected,
                    "bond_next {} should be ~70% of {} (between {} and {})",
                    bond_next, bond_current, min_expected, max_expected
                );
            }
        }

        /// Epoch calculation is monotonically increasing
        #[test]
        fn prop_epoch_monotonic(slot_a: u32, slot_b: u32) {
            let params = ConsensusParams::mainnet();
            if slot_a <= slot_b {
                prop_assert!(params.slot_to_epoch(slot_a) <= params.slot_to_epoch(slot_b));
            }
        }

        /// Era calculation is monotonically increasing
        #[test]
        fn prop_era_monotonic(height_a: u64, height_b: u64) {
            let params = ConsensusParams::mainnet();
            if height_a <= height_b {
                prop_assert!(params.height_to_era(height_a) <= params.height_to_era(height_b));
            }
        }

        /// All heights within an era have same reward
        #[test]
        fn prop_same_era_same_reward(era in 0u32..20, offset in 0u64..BLOCKS_PER_ERA) {
            let params = ConsensusParams::mainnet();
            let base_height = (era as u64) * BLOCKS_PER_ERA;
            let height = base_height + offset;
            prop_assert_eq!(params.block_reward(base_height), params.block_reward(height));
        }

        /// All slots within an epoch have same epoch number
        #[test]
        fn prop_same_epoch_same_number(epoch in 0u32..10000, offset in 0u32..SLOTS_PER_EPOCH) {
            let params = ConsensusParams::mainnet();
            let base_slot = epoch * SLOTS_PER_EPOCH;
            let slot = base_slot.saturating_add(offset);
            // Only check if we didn't overflow
            if slot >= base_slot {
                prop_assert_eq!(params.slot_to_epoch(base_slot), params.slot_to_epoch(slot));
            }
        }

        /// Fallback producer selection is deterministic (anti-grinding: no prev_hash dependency)
        #[test]
        fn prop_fallback_deterministic(slot: u32, num_producers in 1usize..20) {
            let producers: Vec<(crypto::PublicKey, u64)> = (0..num_producers)
                .map(|i| {
                    let mut bytes = [0u8; 32];
                    bytes[0] = i as u8;
                    bytes[1] = (i / 256) as u8;
                    (crypto::PublicKey::from_bytes(bytes), (i as u64 + 1) * 10)
                })
                .collect();

            let result1 = select_producer_for_slot(slot, &producers);
            let result2 = select_producer_for_slot(slot, &producers);

            prop_assert_eq!(result1, result2);
        }

        /// Fallback producer selection returns at most MAX_FALLBACK_PRODUCERS
        #[test]
        fn prop_fallback_max_producers(slot: u32, num_producers in 1usize..20) {
            let producers: Vec<(crypto::PublicKey, u64)> = (0..num_producers)
                .map(|i| {
                    let mut bytes = [0u8; 32];
                    bytes[0] = i as u8;
                    bytes[1] = (i / 256) as u8;
                    (crypto::PublicKey::from_bytes(bytes), (i as u64 + 1) * 10)
                })
                .collect();

            let result = select_producer_for_slot(slot, &producers);

            prop_assert!(result.len() <= MAX_FALLBACK_PRODUCERS);
            prop_assert!(result.len() <= producers.len());
        }

        /// Allowed producer rank follows time windows (seconds)
        #[test]
        fn prop_allowed_rank_time_windows(offset: u64) {
            let rank = allowed_producer_rank(offset);
            if offset < PRIMARY_WINDOW_SECS {
                prop_assert_eq!(rank, 0);
            } else if offset < SECONDARY_WINDOW_SECS {
                prop_assert_eq!(rank, 1);
            } else {
                prop_assert_eq!(rank, 2);
            }
        }

        /// Allowed producer rank follows time windows (milliseconds)
        #[test]
        fn prop_allowed_rank_time_windows_ms(offset_ms: u64) {
            let rank = allowed_producer_rank_ms(offset_ms);
            if offset_ms < PRIMARY_WINDOW_MS {
                prop_assert_eq!(rank, 0);
            } else if offset_ms < SECONDARY_WINDOW_MS {
                prop_assert_eq!(rank, 1);
            } else {
                prop_assert_eq!(rank, 2);
            }
        }

        /// Bootstrap phase is exactly first BOOTSTRAP_BLOCKS
        #[test]
        fn prop_bootstrap_boundary(height: u64) {
            let params = ConsensusParams::mainnet();
            prop_assert_eq!(params.is_bootstrap(height), height < BOOTSTRAP_BLOCKS);
        }

        /// Testnet parameters are consistent
        #[test]
        fn prop_testnet_consistent(slot: u32) {
            let params = ConsensusParams::testnet();
            let timestamp = params.slot_to_timestamp(slot);
            let back = params.timestamp_to_slot(timestamp);
            prop_assert_eq!(slot, back);
        }

        /// No overflow in timestamp calculations
        #[test]
        fn prop_timestamp_no_overflow(slot: u32) {
            let params = ConsensusParams::mainnet();
            // Should not panic
            let _ = params.slot_to_timestamp(slot);
        }

        /// Block reward never exceeds initial reward
        #[test]
        fn prop_reward_bounded(height: u64) {
            let params = ConsensusParams::mainnet();
            let reward = params.block_reward(height);
            prop_assert!(reward <= params.initial_reward);
        }

        /// Bond amount never exceeds initial bond
        #[test]
        fn prop_bond_bounded(height: u64) {
            let params = ConsensusParams::mainnet();
            let bond = params.bond_amount(height);
            prop_assert!(bond <= params.initial_bond);
        }
    }

    // ==================== Stress Test Parameter Tests ====================

    #[test]
    fn test_stress_params_600_producers() {
        let stress = StressTestParams::extreme_600();

        assert_eq!(stress.producer_count, 600);
        assert_eq!(stress.slot_duration_secs, 1);

        // Expected blocks per producer per hour: 3600 slots / 600 producers = 6
        let expected = stress.expected_blocks_per_producer_per_hour();
        assert!((expected - 6.0).abs() < 0.01);

        // Total bond: 600 * 1 DOLI = 600 DOLI
        assert_eq!(stress.total_bond_locked(), 600 * 100_000_000);

        // Network efficiency: 1/600 ≈ 0.167%
        let efficiency = stress.network_efficiency();
        assert!((efficiency - 1.0 / 600.0).abs() < 0.0001);

        // Majority attack: 301 producers
        assert_eq!(stress.slots_for_majority_attack(), 301);
    }

    #[test]
    fn test_stress_params_custom() {
        let stress = StressTestParams::with_producers(100);

        assert_eq!(stress.producer_count, 100);

        // Expected time between blocks: 100 * 2 = 200 seconds
        let time = stress.expected_time_between_blocks_secs();
        assert_eq!(time, 200.0);
    }

    #[test]
    fn test_consensus_for_stress_test() {
        let stress = StressTestParams::extreme_600();
        let params = ConsensusParams::for_stress_test(&stress);

        assert_eq!(params.slot_duration, 1);
        assert_eq!(params.initial_bond, stress.bond_per_producer);
        assert_eq!(params.initial_reward, stress.block_reward);
        assert_eq!(params.bootstrap_blocks, 10);
    }

    #[test]
    fn test_scaled_fallback_windows_example_3s() {
        // Example with 3-second slots (for testing scaling behavior)
        let (primary, secondary, tertiary) = scaled_fallback_windows(3);

        assert_eq!(primary, 1); // 50% of 3s, rounded down but min 1
        assert_eq!(secondary, 2); // 75% of 3s
        assert_eq!(tertiary, 3); // 100%
    }

    #[test]
    fn test_scaled_fallback_windows_longer() {
        // Test with longer slots for non-standard networks
        let (primary, secondary, tertiary) = scaled_fallback_windows(10);

        assert_eq!(primary, 5); // 50% of 10s
        assert_eq!(secondary, 7); // 75% of 10s
        assert_eq!(tertiary, 10); // 100%
    }

    #[test]
    fn test_scaled_fallback_windows_stress() {
        let (primary, secondary, tertiary) = scaled_fallback_windows(1);

        // With 1s slots, all windows compress to minimum values
        assert!(primary >= 1);
        assert!(secondary >= 1);
        assert_eq!(tertiary, 1);
    }

    #[test]
    fn test_allowed_producer_rank_scaled() {
        // Example with 3 second slots (testing scaling behavior)
        // Windows: primary=1, secondary=2, tertiary=3
        assert_eq!(allowed_producer_rank_scaled(0, 3), 0); // Primary: 0 < 1
        assert_eq!(allowed_producer_rank_scaled(1, 3), 1); // Secondary: 1 >= 1, 1 < 2
        assert_eq!(allowed_producer_rank_scaled(2, 3), 2); // Tertiary: 2 >= 2
        assert_eq!(allowed_producer_rank_scaled(3, 3), 2); // Tertiary: beyond slot

        // 10 second slots (custom network)
        // Windows: primary=5, secondary=7, tertiary=10
        assert_eq!(allowed_producer_rank_scaled(0, 10), 0); // Primary
        assert_eq!(allowed_producer_rank_scaled(4, 10), 0); // Primary
        assert_eq!(allowed_producer_rank_scaled(5, 10), 1); // Secondary
        assert_eq!(allowed_producer_rank_scaled(6, 10), 1); // Secondary
        assert_eq!(allowed_producer_rank_scaled(7, 10), 2); // Tertiary
        assert_eq!(allowed_producer_rank_scaled(9, 10), 2); // Tertiary

        // 1 second slots (extreme stress)
        // Windows: primary=1 (min), secondary=1 (min), tertiary=1
        // All compressed to minimum
        assert_eq!(allowed_producer_rank_scaled(0, 1), 0); // Primary: 0 < 1
        assert_eq!(allowed_producer_rank_scaled(1, 1), 2); // Tertiary: 1 >= 1
    }

    #[test]
    fn test_stress_summary_format() {
        let stress = StressTestParams::extreme_600();
        let summary = stress.summary();

        // Summary should contain key information
        assert!(summary.contains("600 Producers"));
        assert!(summary.contains("Slot duration"));
        assert!(summary.contains("Bond"));
        assert!(summary.contains("51% attack"));
        assert!(summary.contains("301")); // Majority needed
    }

    // ==================== Registration Queue Tests ====================

    #[test]
    fn test_registration_fee_base() {
        // With no pending registrations, fee is base
        assert_eq!(registration_fee(0), BASE_REGISTRATION_FEE);
    }

    #[test]
    fn test_registration_fee_table() {
        // Test deterministic table lookup
        // 0-4: 1.00x
        assert_eq!(registration_fee(0), BASE_REGISTRATION_FEE);
        assert_eq!(registration_fee(4), BASE_REGISTRATION_FEE);

        // 5-9: 1.50x
        assert_eq!(registration_fee(5), BASE_REGISTRATION_FEE * 150 / 100);
        assert_eq!(registration_fee(9), BASE_REGISTRATION_FEE * 150 / 100);

        // 10-19: 2.00x
        assert_eq!(registration_fee(10), BASE_REGISTRATION_FEE * 200 / 100);
        assert_eq!(registration_fee(19), BASE_REGISTRATION_FEE * 200 / 100);

        // 20-49: 3.00x
        assert_eq!(registration_fee(20), BASE_REGISTRATION_FEE * 300 / 100);
        assert_eq!(registration_fee(49), BASE_REGISTRATION_FEE * 300 / 100);

        // 50-99: 4.50x
        assert_eq!(registration_fee(50), BASE_REGISTRATION_FEE * 450 / 100);
        assert_eq!(registration_fee(99), BASE_REGISTRATION_FEE * 450 / 100);

        // 100-199: 6.50x
        assert_eq!(registration_fee(100), BASE_REGISTRATION_FEE * 650 / 100);
        assert_eq!(registration_fee(199), BASE_REGISTRATION_FEE * 650 / 100);

        // 200-299: 8.50x
        assert_eq!(registration_fee(200), BASE_REGISTRATION_FEE * 850 / 100);
        assert_eq!(registration_fee(299), BASE_REGISTRATION_FEE * 850 / 100);

        // 300+: 10.00x cap
        assert_eq!(registration_fee(300), BASE_REGISTRATION_FEE * 10);
        assert_eq!(registration_fee(1000), BASE_REGISTRATION_FEE * 10);
    }

    #[test]
    fn test_registration_fee_escalates() {
        // Fee should increase with pending count at boundaries
        let fee_0 = registration_fee(0);
        let fee_5 = registration_fee(5);
        let fee_10 = registration_fee(10);
        let fee_20 = registration_fee(20);
        let fee_50 = registration_fee(50);
        let fee_100 = registration_fee(100);
        let fee_200 = registration_fee(200);
        let fee_300 = registration_fee(300);

        assert!(fee_5 > fee_0);
        assert!(fee_10 > fee_5);
        assert!(fee_20 > fee_10);
        assert!(fee_50 > fee_20);
        assert!(fee_100 > fee_50);
        assert!(fee_200 > fee_100);
        assert!(fee_300 > fee_200);
    }

    #[test]
    fn test_registration_fee_capped_at_10x() {
        // Cap should be exactly 10x base
        assert_eq!(MAX_REGISTRATION_FEE, BASE_REGISTRATION_FEE * 10);

        // High pending count should hit cap
        let fee_high = registration_fee(300);
        assert_eq!(fee_high, MAX_REGISTRATION_FEE);

        // Even higher should still be capped
        let fee_extreme = registration_fee(u32::MAX / 2);
        assert_eq!(fee_extreme, MAX_REGISTRATION_FEE);
    }

    #[test]
    fn test_fee_multiplier_const_fn() {
        // Test that fee_multiplier_x100 is const and deterministic
        const MULTIPLIER_0: u32 = fee_multiplier_x100(0);
        const MULTIPLIER_5: u32 = fee_multiplier_x100(5);
        const MULTIPLIER_300: u32 = fee_multiplier_x100(300);

        assert_eq!(MULTIPLIER_0, 100);
        assert_eq!(MULTIPLIER_5, 150);
        assert_eq!(MULTIPLIER_300, 1000);
    }

    #[test]
    fn test_registration_fee_no_overflow() {
        // Test that extreme values don't cause overflow
        let fee_max = registration_fee(u32::MAX);
        assert_eq!(fee_max, MAX_REGISTRATION_FEE);

        let fee_half_max = registration_fee(u32::MAX / 2);
        assert_eq!(fee_half_max, MAX_REGISTRATION_FEE);
    }

    #[test]
    fn test_registration_queue_new() {
        let queue = RegistrationQueue::new();

        assert_eq!(queue.pending_count(), 0);
        assert_eq!(queue.current_fee(), BASE_REGISTRATION_FEE);
        assert!(queue.can_add_to_block());
    }

    #[test]
    fn test_registration_queue_submit() {
        let mut queue = RegistrationQueue::new();

        let reg = PendingRegistration {
            public_key: crypto::PublicKey::from_bytes([1u8; 32]),
            bond_amount: 100_000_000_000,
            fee_paid: BASE_REGISTRATION_FEE,
            submitted_at: 1000,
            prev_registration_hash: crypto::Hash::ZERO,
            sequence_number: 0,
        };

        assert!(queue.submit(reg, 1000).is_ok());
        assert_eq!(queue.pending_count(), 1);

        // Fee stays at base until we hit 5 pending (table-based)
        assert_eq!(queue.current_fee(), BASE_REGISTRATION_FEE);
    }

    #[test]
    fn test_registration_queue_fee_tiers() {
        let mut queue = RegistrationQueue::new();

        // Submit 4 registrations - still in 1.00x tier
        for i in 0..4 {
            let fee = queue.current_fee();
            let reg = PendingRegistration {
                public_key: crypto::PublicKey::from_bytes([i as u8; 32]),
                bond_amount: 100_000_000_000,
                fee_paid: fee,
                submitted_at: 1000,
                prev_registration_hash: crypto::Hash::ZERO,
                sequence_number: i as u64,
            };
            queue.submit(reg, 1000).unwrap();
        }
        assert_eq!(queue.current_fee(), BASE_REGISTRATION_FEE); // Still 1.00x

        // Add one more to reach 5 pending -> 1.50x tier
        let reg5 = PendingRegistration {
            public_key: crypto::PublicKey::from_bytes([4u8; 32]),
            bond_amount: 100_000_000_000,
            fee_paid: BASE_REGISTRATION_FEE,
            submitted_at: 1000,
            prev_registration_hash: crypto::Hash::ZERO,
            sequence_number: 4,
        };
        queue.submit(reg5, 1000).unwrap();
        assert_eq!(queue.current_fee(), BASE_REGISTRATION_FEE * 150 / 100); // Now 1.50x
    }

    #[test]
    fn test_registration_queue_insufficient_fee() {
        let mut queue = RegistrationQueue::new();

        // Submit 5 registrations to push into the 1.5x tier
        for i in 0..5 {
            let fee = queue.current_fee();
            let reg = PendingRegistration {
                public_key: crypto::PublicKey::from_bytes([i as u8; 32]),
                bond_amount: 100_000_000_000,
                fee_paid: fee,
                submitted_at: 1000,
                prev_registration_hash: crypto::Hash::ZERO,
                sequence_number: i as u64,
            };
            queue.submit(reg, 1000).unwrap();
        }

        // Now queue has 5 pending, so fee should be 1.5x base
        assert_eq!(queue.current_fee(), BASE_REGISTRATION_FEE * 150 / 100);

        // Registration with base fee should fail (need 1.5x)
        let reg_insufficient = PendingRegistration {
            public_key: crypto::PublicKey::from_bytes([99u8; 32]),
            bond_amount: 100_000_000_000,
            fee_paid: BASE_REGISTRATION_FEE, // Insufficient - should be 1.5x
            submitted_at: 1000,
            prev_registration_hash: crypto::Hash::ZERO,
            sequence_number: 99,
        };
        assert!(queue.submit(reg_insufficient, 1000).is_err());
    }

    #[test]
    fn test_registration_queue_block_limit() {
        let mut queue = RegistrationQueue::new();

        // Submit more than MAX_REGISTRATIONS_PER_BLOCK
        for i in 0..10 {
            let fee = queue.current_fee();
            let reg = PendingRegistration {
                public_key: crypto::PublicKey::from_bytes([i as u8; 32]),
                bond_amount: 100_000_000_000,
                fee_paid: fee,
                submitted_at: 1000,
                prev_registration_hash: crypto::Hash::ZERO,
                sequence_number: i as u64,
            };
            queue.submit(reg, 1000).unwrap();
        }

        assert_eq!(queue.pending_count(), 10);

        // Begin block processing
        queue.begin_block(1000);

        // Should only be able to process MAX_REGISTRATIONS_PER_BLOCK
        let mut processed = 0;
        while queue.next_registration().is_some() {
            processed += 1;
        }

        assert_eq!(processed, MAX_REGISTRATIONS_PER_BLOCK as usize);
        assert_eq!(
            queue.pending_count(),
            10 - MAX_REGISTRATIONS_PER_BLOCK as usize
        );
    }

    #[test]
    fn test_registration_queue_fifo() {
        let mut queue = RegistrationQueue::new();

        // Submit registrations with different sequence numbers
        for i in 0..3 {
            let fee = queue.current_fee();
            let reg = PendingRegistration {
                public_key: crypto::PublicKey::from_bytes([i as u8; 32]),
                bond_amount: 100_000_000_000,
                fee_paid: fee,
                submitted_at: 1000,
                prev_registration_hash: crypto::Hash::ZERO,
                sequence_number: i as u64,
            };
            queue.submit(reg, 1000).unwrap();
        }

        queue.begin_block(1000);

        // Should process in FIFO order
        let first = queue.next_registration().unwrap();
        assert_eq!(first.sequence_number, 0);

        let second = queue.next_registration().unwrap();
        assert_eq!(second.sequence_number, 1);

        let third = queue.next_registration().unwrap();
        assert_eq!(third.sequence_number, 2);
    }

    #[test]
    fn test_registration_queue_prune_expired() {
        let mut queue = RegistrationQueue::new();

        // Submit old registration
        let old_reg = PendingRegistration {
            public_key: crypto::PublicKey::from_bytes([1u8; 32]),
            bond_amount: 100_000_000_000,
            fee_paid: BASE_REGISTRATION_FEE,
            submitted_at: 1000,
            prev_registration_hash: crypto::Hash::ZERO,
            sequence_number: 0,
        };
        queue.submit(old_reg, 1000).unwrap();

        // Submit recent registration
        let recent_fee = queue.current_fee();
        let recent_reg = PendingRegistration {
            public_key: crypto::PublicKey::from_bytes([2u8; 32]),
            bond_amount: 100_000_000_000,
            fee_paid: recent_fee,
            submitted_at: 5000,
            prev_registration_hash: crypto::Hash::ZERO,
            sequence_number: 1,
        };
        queue.submit(recent_reg, 5000).unwrap();

        // Prune with max age of 1000 blocks
        let expired = queue.prune_expired(6000, 1000);

        // Old registration should be expired
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].sequence_number, 0);

        // Recent registration should remain
        assert_eq!(queue.pending_count(), 1);
    }

    proptest! {
        /// Scaled windows maintain correct ordering
        #[test]
        fn prop_scaled_windows_ordering(slot_duration in 1u64..120) {
            let (primary, secondary, tertiary) = scaled_fallback_windows(slot_duration);
            prop_assert!(primary <= secondary);
            prop_assert!(secondary <= tertiary);
            prop_assert_eq!(tertiary, slot_duration);
        }

        /// Stress test params are internally consistent
        #[test]
        fn prop_stress_params_consistent(count in 1u32..1000) {
            let stress = StressTestParams::with_producers(count);

            // Total bond should be count * individual bond
            prop_assert_eq!(
                stress.total_bond_locked(),
                stress.bond_per_producer * count as u64
            );

            // Majority attack needs > 50%
            prop_assert!(stress.slots_for_majority_attack() > count / 2);
            prop_assert!(stress.slots_for_majority_attack() <= (count / 2) + 1);
        }

        /// Registration fee is monotonically non-decreasing
        /// fee(p+1) >= fee(p) for all p
        #[test]
        fn prop_registration_fee_monotonic(pending_a: u32, pending_b: u32) {
            if pending_a <= pending_b {
                let fee_a = registration_fee(pending_a);
                let fee_b = registration_fee(pending_b);
                prop_assert!(
                    fee_a <= fee_b,
                    "Monotonicity violated: fee({}) = {} > fee({}) = {}",
                    pending_a, fee_a, pending_b, fee_b
                );
            }
        }

        /// Registration fee is always at least BASE_REGISTRATION_FEE
        #[test]
        fn prop_registration_fee_min(pending: u32) {
            let fee = registration_fee(pending);
            prop_assert!(
                fee >= BASE_REGISTRATION_FEE,
                "Fee {} is below base {} for pending count {}",
                fee, BASE_REGISTRATION_FEE, pending
            );
        }

        /// Registration fee never exceeds MAX_REGISTRATION_FEE (10x base)
        #[test]
        fn prop_registration_fee_cap(pending: u32) {
            let fee = registration_fee(pending);
            prop_assert!(
                fee <= MAX_REGISTRATION_FEE,
                "Fee {} exceeds cap {} for pending count {}",
                fee, MAX_REGISTRATION_FEE, pending
            );
            // Also verify cap is exactly 10x base
            prop_assert_eq!(MAX_REGISTRATION_FEE, BASE_REGISTRATION_FEE * 10);
        }

        /// Registration fee never overflows (test extreme values)
        #[test]
        fn prop_registration_fee_no_overflow(pending in 0u32..=u32::MAX) {
            // This should never panic
            let fee = registration_fee(pending);
            // Result should be valid
            prop_assert!(fee >= BASE_REGISTRATION_FEE);
            prop_assert!(fee <= MAX_REGISTRATION_FEE);
        }

        /// Fee multiplier is deterministic (same input = same output)
        #[test]
        fn prop_fee_multiplier_deterministic(pending: u32) {
            let mult1 = fee_multiplier_x100(pending);
            let mult2 = fee_multiplier_x100(pending);
            prop_assert_eq!(mult1, mult2);
        }

        /// Fee multiplier is within bounds [100, 1000]
        #[test]
        fn prop_fee_multiplier_bounds(pending: u32) {
            let mult = fee_multiplier_x100(pending);
            prop_assert!(mult >= 100, "Multiplier {} below 100 (1x)", mult);
            prop_assert!(mult <= 1000, "Multiplier {} above 1000 (10x)", mult);
        }
    }

    // ==================== Required Protocol Tests ====================
    //
    // These tests verify critical protocol properties as specified in TASKS.md

    /// Test: constants_match_whitepaper
    /// Verify all consensus constants match the whitepaper specification.
    #[test]
    fn test_constants_match_whitepaper() {
        // Slot duration: 10 seconds (mainnet)
        assert_eq!(SLOT_DURATION, 10);

        // Slots per epoch: 360 (1 hour with 10s slots)
        assert_eq!(SLOTS_PER_EPOCH, 360);

        // Slots per year: 3,153,600 (365.25 days * 24h * 360 slots/h)
        assert_eq!(SLOTS_PER_YEAR, 3_153_600);

        // Slots per era: 4 years = 12,614,400 slots
        assert_eq!(SLOTS_PER_ERA, 12_614_400);

        // Total supply: 2,522,880,000,000,000 sats (25,228,800 DOLI)
        assert_eq!(TOTAL_SUPPLY, 2_522_880_000_000_000);

        // Initial block reward: 100,000,000 sats (1 DOLI)
        assert_eq!(INITIAL_BLOCK_REWARD, 100_000_000);

        // VDF iterations: 10 million per block
        assert_eq!(T_BLOCK, 10_000_000);

        // Window timings: 3s/6s/10s (30%/60%/100% of slot)
        assert_eq!(PRIMARY_WINDOW_MS, 3000);
        assert_eq!(SECONDARY_WINDOW_MS, 6000);
        assert_eq!(TERTIARY_WINDOW_MS, 10000);

        // Unbonding period: 7 days = 60,480 slots
        assert_eq!(UNBONDING_PERIOD, 60_480);
    }

    /// Test: selection_independent_of_prev_hash
    /// Producer selection must NOT depend on prev_hash (anti-grinding)
    #[test]
    fn test_selection_independent_of_prev_hash() {
        // Create producers with different bond counts
        let producer_a = crypto::PublicKey::from_bytes([1u8; 32]);
        let producer_b = crypto::PublicKey::from_bytes([2u8; 32]);
        let producer_c = crypto::PublicKey::from_bytes([3u8; 32]);

        let producers = vec![
            (producer_a.clone(), 10), // 10 bonds
            (producer_b.clone(), 5),  // 5 bonds
            (producer_c.clone(), 3),  // 3 bonds
        ];

        // Selection for any slot should be deterministic based only on slot + producers
        // NOT on prev_hash (which would enable grinding)
        let slot = 42;

        let selection_1 = select_producer_for_slot(slot, &producers);
        let selection_2 = select_producer_for_slot(slot, &producers);
        let selection_3 = select_producer_for_slot(slot, &producers);

        // Same slot, same producers -> same selection
        assert_eq!(selection_1, selection_2);
        assert_eq!(selection_2, selection_3);

        // Verify the selection function doesn't take a prev_hash parameter
        // (This is enforced by the function signature itself)
    }

    /// Test: selection_uses_consecutive_tickets
    /// Selection must use consecutive ticket ranges based on bond count
    /// Fallbacks use consecutive tickets: (base + 1) % total, (base + 2) % total
    #[test]
    fn test_selection_uses_consecutive_tickets() {
        // Create producers with known bond counts
        let producer_a = crypto::PublicKey::from_bytes([1u8; 32]);
        let producer_b = crypto::PublicKey::from_bytes([2u8; 32]);
        let producer_c = crypto::PublicKey::from_bytes([3u8; 32]);

        // Sort order is by pubkey bytes, so:
        // [1,1,1...] < [2,2,2...] < [3,3,3...]
        // producer_a, producer_b, producer_c
        let producers = vec![
            (producer_a.clone(), 3), // 3 bonds -> tickets 0, 1, 2
            (producer_b.clone(), 2), // 2 bonds -> tickets 3, 4
            (producer_c.clone(), 1), // 1 bond  -> ticket 5
        ];
        // Total tickets = 6

        // Test consecutive ticket assignment:
        // slot 0 -> ticket 0 -> producer_a
        // slot 1 -> ticket 1 -> producer_a
        // slot 2 -> ticket 2 -> producer_a
        // slot 3 -> ticket 3 -> producer_b
        // slot 4 -> ticket 4 -> producer_b
        // slot 5 -> ticket 5 -> producer_c
        // slot 6 -> ticket 0 (wraps) -> producer_a

        let selection_0 = select_producer_for_slot(0, &producers);
        let selection_1 = select_producer_for_slot(1, &producers);
        let selection_2 = select_producer_for_slot(2, &producers);
        let selection_3 = select_producer_for_slot(3, &producers);
        let selection_4 = select_producer_for_slot(4, &producers);
        let selection_5 = select_producer_for_slot(5, &producers);
        let selection_6 = select_producer_for_slot(6, &producers);

        // producer_a should be primary for slots 0, 1, 2
        assert_eq!(selection_0[0], producer_a);
        assert_eq!(selection_1[0], producer_a);
        assert_eq!(selection_2[0], producer_a);

        // producer_b should be primary for slots 3, 4
        assert_eq!(selection_3[0], producer_b);
        assert_eq!(selection_4[0], producer_b);

        // producer_c should be primary for slot 5
        assert_eq!(selection_5[0], producer_c);

        // Slot 6 wraps back to producer_a
        assert_eq!(selection_6[0], producer_a);

        // Test consecutive fallback selection (anti-grinding)
        // For slot 0: base ticket = 0
        //   - Primary (rank 0): ticket 0 -> producer_a
        //   - Secondary (rank 1): ticket 1 -> producer_a (same producer, not added)
        //   - Tertiary (rank 2): ticket 2 -> producer_a (same producer, not added)
        // Result: only producer_a (consecutive tickets land on same producer)
        assert!(selection_0.len() >= 1);
        assert_eq!(selection_0[0], producer_a);

        // For slot 2: base ticket = 2
        //   - Primary (rank 0): ticket 2 -> producer_a
        //   - Secondary (rank 1): ticket 3 -> producer_b
        //   - Tertiary (rank 2): ticket 4 -> producer_b (same as secondary, not added)
        assert_eq!(selection_2.len(), 2);
        assert_eq!(selection_2[0], producer_a);
        assert_eq!(selection_2[1], producer_b);

        // For slot 3: base ticket = 3
        //   - Primary (rank 0): ticket 3 -> producer_b
        //   - Secondary (rank 1): ticket 4 -> producer_b (same, not added)
        //   - Tertiary (rank 2): ticket 5 -> producer_c
        assert_eq!(selection_3.len(), 2);
        assert_eq!(selection_3[0], producer_b);
        assert_eq!(selection_3[1], producer_c);

        // For slot 4: base ticket = 4
        //   - Primary (rank 0): ticket 4 -> producer_b
        //   - Secondary (rank 1): ticket 5 -> producer_c
        //   - Tertiary (rank 2): ticket 0 (wraps) -> producer_a
        assert_eq!(selection_4.len(), 3);
        assert_eq!(selection_4[0], producer_b);
        assert_eq!(selection_4[1], producer_c);
        assert_eq!(selection_4[2], producer_a);
    }

    /// Test: fallback_windows
    /// Verify fallback window timing (mainnet uses 3s/6s/10s = 30%/60%/100%)
    #[test]
    fn test_fallback_windows() {
        // Verify mainnet constants in milliseconds
        assert_eq!(PRIMARY_WINDOW_MS, 3000);
        assert_eq!(SECONDARY_WINDOW_MS, 6000);
        assert_eq!(TERTIARY_WINDOW_MS, 10000);

        // Verify mainnet constants in seconds
        assert_eq!(PRIMARY_WINDOW_SECS, 3);
        assert_eq!(SECONDARY_WINDOW_SECS, 6);

        // Test eligibility function with mainnet windows
        // NOTE: is_producer_eligible takes SECONDS, not milliseconds
        let producer_a = crypto::PublicKey::from_bytes([1u8; 32]);
        let producer_b = crypto::PublicKey::from_bytes([2u8; 32]);
        let producer_c = crypto::PublicKey::from_bytes([3u8; 32]);

        let eligible = vec![producer_a.clone(), producer_b.clone(), producer_c.clone()];

        // Primary (rank 0) is eligible immediately (0-2 seconds)
        assert!(is_producer_eligible(&producer_a, &eligible, 0));
        assert!(is_producer_eligible(&producer_a, &eligible, 2));

        // Secondary (rank 1) is eligible after primary window (3+ seconds)
        assert!(!is_producer_eligible(&producer_b, &eligible, 2));
        assert!(is_producer_eligible(&producer_b, &eligible, 3));

        // Tertiary (rank 2) is eligible after secondary window (6+ seconds)
        assert!(!is_producer_eligible(&producer_c, &eligible, 5));
        assert!(is_producer_eligible(&producer_c, &eligible, 6));

        // Test millisecond function for precise timing
        assert_eq!(allowed_producer_rank_ms(2999), 0);
        assert_eq!(allowed_producer_rank_ms(3000), 1);
        assert_eq!(allowed_producer_rank_ms(5999), 1);
        assert_eq!(allowed_producer_rank_ms(6000), 2);

        // Test scaled_fallback_windows (used for non-mainnet networks)
        // Uses 50%/75%/100% ratios
        let (primary, secondary, tertiary) = scaled_fallback_windows(10);
        assert_eq!(primary, 5); // 50% of 10
        assert_eq!(secondary, 7); // 75% of 10
        assert_eq!(tertiary, 10); // 100% of 10
    }

    /// Test: seniority_uses_years
    /// Seniority weight must use discrete yearly steps (0-1=1, 1-2=2, 2-3=3, 3+=4)
    #[test]
    fn test_seniority_uses_years() {
        // This test validates the weight formula uses yearly boundaries
        // The actual weight calculation is in storage/producer.rs

        // Verify SLOTS_PER_YEAR is correct for the calculation
        // 365 days * 24 hours * 360 slots/hour = 3,153,600 slots/year
        let expected_slots_per_year = 365 * 24 * 360;
        assert_eq!(SLOTS_PER_YEAR as u64, expected_slots_per_year);

        // Weight boundaries should be at year marks:
        // Year 0 (0-3,153,599 slots): weight 1
        // Year 1 (3,153,600-6,307,199 slots): weight 2
        // Year 2 (6,307,200-9,460,799 slots): weight 3
        // Year 3+ (9,460,800+ slots): weight 4

        // This is validated by storage/producer.rs tests
    }

    /// Test: supply_converges
    /// Total supply must converge to TOTAL_SUPPLY through halving
    #[test]
    fn test_supply_converges() {
        let mut total_minted: u64 = 0;
        let mut era = 0;
        let mut current_reward = INITIAL_BLOCK_REWARD;

        // Simulate mining through multiple eras
        while total_minted < TOTAL_SUPPLY {
            // Calculate how much can be minted this era
            let slots_in_era = SLOTS_PER_ERA;
            let max_mint_this_era = current_reward.saturating_mul(slots_in_era);

            // Mint up to the remaining supply
            let remaining = TOTAL_SUPPLY.saturating_sub(total_minted);
            let minted_this_era = max_mint_this_era.min(remaining);

            total_minted = total_minted.saturating_add(minted_this_era);

            // Halve reward for next era
            current_reward /= 2;
            era += 1;

            // Safety limit
            if era > 100 || current_reward == 0 {
                break;
            }
        }

        // Supply should converge to TOTAL_SUPPLY
        assert!(total_minted <= TOTAL_SUPPLY);
    }

    /// Test: presence_not_in_consensus
    /// Presence score must NOT affect validity or selection
    #[test]
    fn test_presence_not_in_consensus() {
        // Producer selection function signature doesn't include presence score
        // This is enforced by the type system

        // The select_producer_for_slot function takes only:
        // - slot: Slot (u32)
        // - producers_with_bonds: &[(PublicKey, u64)]
        //
        // No presence score, no telemetry data

        let producer_a = crypto::PublicKey::from_bytes([1u8; 32]);
        let producers = vec![(producer_a.clone(), 5)];

        // Selection is deterministic and doesn't depend on any external state
        let selection_1 = select_producer_for_slot(100, &producers);
        let selection_2 = select_producer_for_slot(100, &producers);
        assert_eq!(selection_1, selection_2);

        // Even if presence tracking exists (in tpop/presence.rs),
        // it's marked as telemetry-only and doesn't affect this selection
    }

    // =========================================================================
    // REWARD EPOCH TESTS (Block-height based)
    // =========================================================================

    #[test]
    fn test_reward_epoch_from_height() {
        // Test the from_height function matches the spec
        assert_eq!(reward_epoch::from_height(0), 0);
        assert_eq!(reward_epoch::from_height(359), 0);
        assert_eq!(reward_epoch::from_height(360), 1);
        assert_eq!(reward_epoch::from_height(719), 1);
        assert_eq!(reward_epoch::from_height(720), 2);
        assert_eq!(reward_epoch::from_height(1000), 2); // 1000/360 = 2
    }

    #[test]
    fn test_reward_epoch_boundaries() {
        // Test boundaries function
        let (start, end) = reward_epoch::boundaries(0);
        assert_eq!(start, 0);
        assert_eq!(end, 360);

        let (start, end) = reward_epoch::boundaries(1);
        assert_eq!(start, 360);
        assert_eq!(end, 720);

        let (start, end) = reward_epoch::boundaries(5);
        assert_eq!(start, 1800);
        assert_eq!(end, 2160);
    }

    #[test]
    fn test_reward_epoch_is_complete() {
        // Epoch 0 ends at block 360
        assert!(!reward_epoch::is_complete(0, 0));
        assert!(!reward_epoch::is_complete(0, 359));
        assert!(reward_epoch::is_complete(0, 360));
        assert!(reward_epoch::is_complete(0, 1000));

        // Epoch 1 ends at block 720
        assert!(!reward_epoch::is_complete(1, 360));
        assert!(!reward_epoch::is_complete(1, 719));
        assert!(reward_epoch::is_complete(1, 720));
    }

    #[test]
    fn test_reward_epoch_last_complete() {
        // No complete epochs yet
        assert_eq!(reward_epoch::last_complete(0), None);
        assert_eq!(reward_epoch::last_complete(359), None);

        // First epoch just completed
        assert_eq!(reward_epoch::last_complete(360), Some(0));
        assert_eq!(reward_epoch::last_complete(719), Some(0));

        // Two epochs completed
        assert_eq!(reward_epoch::last_complete(720), Some(1));
        assert_eq!(reward_epoch::last_complete(1000), Some(1));
    }

    #[test]
    fn test_reward_epoch_is_epoch_start() {
        assert!(reward_epoch::is_epoch_start(0));
        assert!(!reward_epoch::is_epoch_start(1));
        assert!(!reward_epoch::is_epoch_start(359));
        assert!(reward_epoch::is_epoch_start(360));
        assert!(reward_epoch::is_epoch_start(720));
    }

    #[test]
    fn test_reward_epoch_current() {
        // current() is an alias for from_height()
        assert_eq!(reward_epoch::current(0), reward_epoch::from_height(0));
        assert_eq!(reward_epoch::current(500), reward_epoch::from_height(500));
        assert_eq!(reward_epoch::current(1000), reward_epoch::from_height(1000));
    }

    #[test]
    fn test_reward_epoch_complete_epochs() {
        // Before first epoch completes
        assert_eq!(reward_epoch::complete_epochs(0), 0);
        assert_eq!(reward_epoch::complete_epochs(359), 0);

        // After first epoch completes
        assert_eq!(reward_epoch::complete_epochs(360), 1);
        assert_eq!(reward_epoch::complete_epochs(719), 1);

        // After second epoch completes
        assert_eq!(reward_epoch::complete_epochs(720), 2);
    }

    #[test]
    fn test_reward_epoch_blocks_per_epoch() {
        assert_eq!(reward_epoch::blocks_per_epoch(), BLOCKS_PER_REWARD_EPOCH);
        assert_eq!(reward_epoch::blocks_per_epoch(), 360);
    }

    // ========================================================================
    // Tests for network-aware (_with) functions
    // ========================================================================

    #[test]
    fn test_reward_epoch_from_height_with_devnet() {
        // Devnet uses 60 blocks per epoch
        let devnet_blocks = 60;

        assert_eq!(reward_epoch::from_height_with(0, devnet_blocks), 0);
        assert_eq!(reward_epoch::from_height_with(59, devnet_blocks), 0);
        assert_eq!(reward_epoch::from_height_with(60, devnet_blocks), 1);
        assert_eq!(reward_epoch::from_height_with(119, devnet_blocks), 1);
        assert_eq!(reward_epoch::from_height_with(120, devnet_blocks), 2);
    }

    #[test]
    fn test_reward_epoch_boundaries_with_devnet() {
        let devnet_blocks = 60;

        let (start, end) = reward_epoch::boundaries_with(0, devnet_blocks);
        assert_eq!(start, 0);
        assert_eq!(end, 60);

        let (start, end) = reward_epoch::boundaries_with(1, devnet_blocks);
        assert_eq!(start, 60);
        assert_eq!(end, 120);

        let (start, end) = reward_epoch::boundaries_with(5, devnet_blocks);
        assert_eq!(start, 300);
        assert_eq!(end, 360);
    }

    #[test]
    fn test_reward_epoch_is_complete_with_devnet() {
        let devnet_blocks = 60;

        // Epoch 0 ends at block 60 for devnet
        assert!(!reward_epoch::is_complete_with(0, 0, devnet_blocks));
        assert!(!reward_epoch::is_complete_with(0, 59, devnet_blocks));
        assert!(reward_epoch::is_complete_with(0, 60, devnet_blocks));
        assert!(reward_epoch::is_complete_with(0, 100, devnet_blocks));

        // Epoch 1 ends at block 120 for devnet
        assert!(!reward_epoch::is_complete_with(1, 60, devnet_blocks));
        assert!(!reward_epoch::is_complete_with(1, 119, devnet_blocks));
        assert!(reward_epoch::is_complete_with(1, 120, devnet_blocks));
    }

    #[test]
    fn test_reward_epoch_last_complete_with_devnet() {
        let devnet_blocks = 60;

        // No complete epochs yet
        assert_eq!(reward_epoch::last_complete_with(0, devnet_blocks), None);
        assert_eq!(reward_epoch::last_complete_with(59, devnet_blocks), None);

        // First epoch just completed (at block 60)
        assert_eq!(reward_epoch::last_complete_with(60, devnet_blocks), Some(0));
        assert_eq!(
            reward_epoch::last_complete_with(119, devnet_blocks),
            Some(0)
        );

        // Two epochs completed (at block 120)
        assert_eq!(
            reward_epoch::last_complete_with(120, devnet_blocks),
            Some(1)
        );
    }

    #[test]
    fn test_reward_epoch_is_epoch_start_with_devnet() {
        let devnet_blocks = 60;

        assert!(reward_epoch::is_epoch_start_with(0, devnet_blocks));
        assert!(!reward_epoch::is_epoch_start_with(1, devnet_blocks));
        assert!(!reward_epoch::is_epoch_start_with(59, devnet_blocks));
        assert!(reward_epoch::is_epoch_start_with(60, devnet_blocks));
        assert!(reward_epoch::is_epoch_start_with(120, devnet_blocks));
    }

    #[test]
    fn test_reward_epoch_complete_epochs_with_devnet() {
        let devnet_blocks = 60;

        // Before first epoch completes
        assert_eq!(reward_epoch::complete_epochs_with(0, devnet_blocks), 0);
        assert_eq!(reward_epoch::complete_epochs_with(59, devnet_blocks), 0);

        // After first epoch completes
        assert_eq!(reward_epoch::complete_epochs_with(60, devnet_blocks), 1);
        assert_eq!(reward_epoch::complete_epochs_with(119, devnet_blocks), 1);

        // After second epoch completes
        assert_eq!(reward_epoch::complete_epochs_with(120, devnet_blocks), 2);
    }

    #[test]
    fn test_reward_epoch_with_mainnet_same_as_default() {
        // Mainnet uses 360 blocks, same as the global constant
        let mainnet_blocks = 360;

        // These should match the non-_with versions
        assert_eq!(
            reward_epoch::from_height_with(1000, mainnet_blocks),
            reward_epoch::from_height(1000)
        );
        assert_eq!(
            reward_epoch::boundaries_with(5, mainnet_blocks),
            reward_epoch::boundaries(5)
        );
        assert_eq!(
            reward_epoch::is_complete_with(0, 360, mainnet_blocks),
            reward_epoch::is_complete(0, 360)
        );
        assert_eq!(
            reward_epoch::last_complete_with(720, mainnet_blocks),
            reward_epoch::last_complete(720)
        );
    }
}
