//! Per-transaction coinbase credit (INC-I-233 F4).
//!
//! A block's coinbase may claim `block_reward + Σ credit(tx)` over its user
//! transactions. The credit is the per-byte fee on output `extra_data`. It is
//! backed only when the tx itself paid at least that much native DOLI as fee:
//! the L1 fee floor (`Transaction::minimum_fee`) guarantees this for every
//! fee-checked type, because Σ floor(len_i/100) ≤ floor(Σ len_i/100) < minimum_fee.
//!
//! The six fee-exempt types skip the L1 fee check (utxo.rs) and may destroy
//! 0 native DOLI, so from `inc_i_233_activation_height` they credit 0.
//! Below the gate the credit is byte-identical to the pre-fix formula.
//!
//! Both the validator (`validate_block_economics`) and the block builder
//! call [`tx_coinbase_credit`], so they cannot drift apart.

use crate::consensus::{FEE_DIVISOR, FEE_PER_BYTE};
use crate::transaction::{Transaction, TxType};

/// Tx types that skip the native balance-fee check. Their amounts are
/// non-native (asset/LP units) or move DOLI into pool reserves.
pub fn is_native_fee_exempt(tx_type: TxType) -> bool {
    matches!(
        tx_type,
        TxType::CreatePool
            | TxType::Swap
            | TxType::AddLiquidity
            | TxType::RemoveLiquidity
            | TxType::MintAsset
            | TxType::BurnAsset
    )
}

/// Coinbase credit (sats) that `tx` adds to its block.
///
/// `exempt_gate_active` = `height >= NetworkParams::inc_i_233_activation_height`.
pub fn tx_coinbase_credit(tx: &Transaction, exempt_gate_active: bool) -> u64 {
    if exempt_gate_active && is_native_fee_exempt(tx.tx_type) {
        return 0;
    }
    tx.outputs
        .iter()
        .map(|o| o.extra_data.len() as u64 * FEE_PER_BYTE / FEE_DIVISOR)
        .sum()
}
