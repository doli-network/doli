//! INC-I-171 M3 — the ONE resolver for the withdrawal bond-input scan
//! (INV-VEST-008), replacing three copies that could drift to three verdicts.
//!
//! It runs BELOW the activation height too, so any behavioural change here is
//! an UNGATED consensus change (INV-VEST-007). It is therefore infallible: a
//! malformed `extra_data` is RECORDED, never rejected — the gated M4 check is
//! what raises `WithdrawalBondExtraDataMalformed`.
//!
//! Iteration follows `tx.inputs`, never a storage enumeration, so both
//! `UtxoSet` backends yield the same result (INV-VEST-003).

use crypto::Hash;
use doli_core::transaction::{OutputType, Transaction};

use crate::utxo::{Outpoint, UtxoSet};

/// What the UTXO set says the inputs of one withdrawal are.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WithdrawalInputs {
    /// Bond inputs whose `pubkey_hash` equals the named owner.
    pub owned_bonds: u32,
    /// Bond inputs of any owner.
    pub all_bonds: u32,
    /// `(amount, creation_slot)` per Bond input, in tx input order. The slot is
    /// `None` when `extra_data` is not the 4 bytes `Output::bond` writes.
    pub spent_bonds: Vec<(u64, Option<u32>)>,
    /// Saturating sum of the non-Bond input amounts.
    pub non_bond_total: u64,
    /// `(input_index, extra_data.len())` per Bond input with an undecodable slot.
    pub malformed_bond_inputs: Vec<(usize, usize)>,
}

/// One pass over `tx.inputs`. An input the set cannot resolve contributes nothing.
pub fn resolve_withdrawal_inputs(
    tx: &Transaction,
    utxo: &UtxoSet,
    owner: &Hash,
) -> WithdrawalInputs {
    let mut resolved = WithdrawalInputs::default();
    for (index, inp) in tx.inputs.iter().enumerate() {
        let Some(entry) = utxo.get(&Outpoint::new(inp.prev_tx_hash, inp.output_index)) else {
            continue;
        };
        if entry.output.output_type != OutputType::Bond {
            resolved.non_bond_total = resolved.non_bond_total.saturating_add(entry.output.amount);
            continue;
        }
        resolved.all_bonds = resolved.all_bonds.saturating_add(1);
        if entry.output.pubkey_hash == *owner {
            resolved.owned_bonds = resolved.owned_bonds.saturating_add(1);
        }
        let slot = <[u8; 4]>::try_from(entry.output.extra_data.as_slice())
            .ok()
            .map(u32::from_le_bytes);
        if slot.is_none() {
            resolved
                .malformed_bond_inputs
                .push((index, entry.output.extra_data.len()));
        }
        resolved.spent_bonds.push((entry.output.amount, slot));
    }
    resolved
}
