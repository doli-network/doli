use crate::types::{Amount, BlockHeight, Slot};
use serde::{Deserialize, Serialize};

use super::constants::{withdrawal_penalty_rate_with_quarter, VESTING_QUARTER_SLOTS};

// ==================== Exit and Slashing System ====================

/// Destination for early exit penalties.
///
/// In DOLI, penalties are 100% burned (not sent to treasury or reward pool).
/// This is deflationary and benefits all token holders equally.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PenaltyDestination {
    /// Penalty is burned (removed from circulation)
    #[default]
    Burn,
    /// Legacy: reward pool (deprecated, kept for compatibility)
    #[deprecated(note = "Use Burn instead - 100% of penalties are burned")]
    RewardPool,
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
/// # Vesting Schedule (4-year, year-based — mainnet)
/// - Y1 (0-1yr): 75% penalty
/// - Y2 (1-2yr): 50% penalty
/// - Y3 (2-3yr): 25% penalty
/// - Y4+ (3yr+): 0% penalty (fully vested)
///
/// Testnet uses 1-day schedule (6h quarters) via NetworkParams.
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
    calculate_exit_with_quarter(
        bond_amount,
        registered_at,
        current_height,
        VESTING_QUARTER_SLOTS,
    )
}

/// Calculate exit terms with custom quarter duration (network-aware).
pub fn calculate_exit_with_quarter(
    bond_amount: Amount,
    registered_at: BlockHeight,
    current_height: BlockHeight,
    quarter_slots: Slot,
) -> ExitTerms {
    let slots_served = current_height.saturating_sub(registered_at);
    let penalty_rate = withdrawal_penalty_rate_with_quarter(slots_served as Slot, quarter_slots);

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
        let quarters = slots_served as Slot / quarter_slots;
        let commitment_percent = ((quarters * 25) as u8).min(100);

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
