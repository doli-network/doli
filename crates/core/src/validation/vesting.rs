//! INC-I-171 — the vesting-penalty consensus predicate.
//!
//! Pure integer arithmetic. No storage type appears in any signature: the caller
//! resolves the spent Bond UTXOs and passes `(amount, creation_slot)` pairs.

use crate::types::Slot;

use super::ValidationError;

/// Penalized net value of one Bond at `block_slot`.
///
/// PENALTY-FIRST truncation, byte-identical to the released CLI
/// (`bins/cli/src/cmd_producer/common.rs:33-38`): 101 at the 75% tier nets 26,
/// not 25. A net-first node would reject every honest Q1 withdrawal from the
/// activation height onward. The multiply widens to `u128` so `u64::MAX` cannot
/// wrap; `pct <= 75` keeps the subtraction below zero-crossing.
pub fn penalized_bond_net(
    amount: u64,
    creation_slot: Slot,
    block_slot: Slot,
    quarter_slots: Slot,
) -> u64 {
    let age = block_slot.saturating_sub(creation_slot);
    let pct = crate::consensus::withdrawal_penalty_rate_with_quarter(age, quarter_slots);
    let penalty = (amount as u128 * pct as u128 / 100) as u64;
    amount - penalty
}

/// Bound a `RequestWithdrawal` payout by the penalized net of the Bond inputs it
/// spends plus its non-Bond input value. `payout <= bound` passes; over-burning
/// is allowed.
///
/// GUARD ORDER IS LOAD-BEARING. The activation gate returns first, so a dormant
/// rule never reads `quarter_slots` — otherwise one node's bad config would turn
/// into block rejections at every height. The quarter guard is second because
/// `withdrawal_penalty_rate_with_quarter` divides unguarded.
///
/// The window is `[activation_height, disable_height)`; both ship `u64::MAX`.
#[allow(clippy::too_many_arguments)]
pub fn check_withdrawal_payout_bound(
    payout: u64,
    spent_bonds: &[(u64, Slot)],
    non_bond_input_total: u64,
    block_slot: Slot,
    quarter_slots: u64,
    height: u64,
    activation_height: u64,
    disable_height: u64,
) -> Result<(), ValidationError> {
    if height < activation_height || height >= disable_height {
        return Ok(());
    }

    if quarter_slots == 0 || quarter_slots > u32::MAX as u64 {
        return Err(ValidationError::VestingQuarterInvalid { quarter_slots });
    }
    let quarter = quarter_slots as Slot;

    let vested = spent_bonds
        .iter()
        .fold(0u128, |acc, (amount, creation_slot)| {
            acc.saturating_add(
                penalized_bond_net(*amount, *creation_slot, block_slot, quarter) as u128,
            )
        });
    let bound = u64::try_from(vested)
        .unwrap_or(u64::MAX)
        .saturating_add(non_bond_input_total);

    if payout > bound {
        return Err(ValidationError::WithdrawalPayoutExceedsVestedNet {
            payout,
            bound,
            bond_inputs: u32::try_from(spent_bonds.len()).unwrap_or(u32::MAX),
            non_bond_value: non_bond_input_total,
        });
    }
    Ok(())
}
