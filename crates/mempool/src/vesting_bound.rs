//! INC-I-171 M5 — the shared vesting payout verdict (REQ-VEST-008).
//!
//! One function, so mempool admission and the block builder cannot drift from
//! `validate_block_economics`. It CALLS `check_withdrawal_payout_bound`; a
//! second copy of the bound is the INC-I-180 `allowance_with` lesson repeated.

use doli_core::transaction::{Transaction, TxType};
use doli_core::types::Slot;
use doli_core::validation::vesting::check_withdrawal_payout_bound;
use doli_core::validation::ValidationError;
use storage::producer::WithdrawalInputs;

/// `Err` carries the bracketed `[CODE]` the fleet greps; the caller skips
/// (builder) or refuses (admission). `inputs` is the caller's ALREADY resolved
/// `resolve_withdrawal_inputs` result — this adds no UTXO pass of its own.
///
/// INV-VEST-013: an undecodable `creation_slot` is an unknown age and fails
/// closed, never `unwrap_or(0)`. INV-VEST-007: outside
/// `[activation_height, disable_height)` this is a no-op.
#[allow(clippy::too_many_arguments)]
pub fn vesting_bound_verdict(
    tx: &Transaction,
    inputs: &WithdrawalInputs,
    slot: Slot,
    height: u64,
    quarter_slots: u64,
    activation_height: u64,
    disable_height: u64,
) -> Result<(), String> {
    if tx.tx_type != TxType::RequestWithdrawal
        || height < activation_height
        || height >= disable_height
    {
        return Ok(());
    }
    if let Some(&(input_index, len)) = inputs.malformed_bond_inputs.first() {
        let malformed = ValidationError::WithdrawalBondExtraDataMalformed {
            input_index: u32::try_from(input_index).unwrap_or(u32::MAX),
            len,
        };
        return Err(format!("[{}] {malformed}", malformed.error_code()));
    }
    // `validate_withdrawal_request_data` admits exactly one Normal output, so
    // this sum IS the payout.
    let payout = tx
        .outputs
        .iter()
        .fold(0u64, |acc, o| acc.saturating_add(o.amount));
    // Every creation slot is `Some`: the malformed guard above already returned.
    let spent: Vec<(u64, Slot)> = inputs
        .spent_bonds
        .iter()
        .filter_map(|(amount, creation)| creation.map(|c| (*amount, c)))
        .collect();
    check_withdrawal_payout_bound(
        payout,
        &spent,
        inputs.non_bond_total,
        slot,
        quarter_slots,
        height,
        activation_height,
        disable_height,
    )
    .map_err(|e| format!("[{}] {e} at height={height} slot={slot}", e.error_code()))
}
