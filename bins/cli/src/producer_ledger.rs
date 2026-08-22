//! Pure producer-ledger arithmetic for withdrawal/exit (INC-I-180 M3,
//! REQ-I180-005 / AUDIT-P2-003). No I/O; unit- and integration-testable.

/// ProducerSet allowance P: own bonds recovered by inverting the selection
/// weight. selection_weight = own - delegated_away + received, so
/// own = selection_weight + delegated_away - received.
pub fn producer_set_allowance(
    selection_weight: u64,
    received_delegations_total: u64,
    delegated_bonds: u64,
) -> u64 {
    (selection_weight + delegated_bonds).saturating_sub(received_delegations_total)
}

/// Bonds withdrawable this epoch = allowance minus already-pending withdrawals.
pub fn max_withdrawable(allowance_p: u64, withdrawal_pending_count: u64) -> u64 {
    allowance_p.saturating_sub(withdrawal_pending_count)
}

/// True when the UTXO-derived bond count disagrees with the ProducerSet allowance.
pub fn withdrawal_ledger_mismatch(utxo_bond_count: u64, allowance_p: u64) -> bool {
    utxo_bond_count != allowance_p
}

/// Select exactly `count` Bond UTXO input indices (0..count). Binds COUNT not
/// VALUE so the emitted tx has exactly `count` Bond inputs (AUDIT-P2-003).
pub fn select_bond_inputs_by_count(
    bond_utxo_amounts: &[u64],
    count: u32,
) -> Result<Vec<usize>, String> {
    let count = count as usize;
    if count <= bond_utxo_amounts.len() {
        Ok((0..count).collect())
    } else {
        Err(format!(
            "not enough Bond UTXOs: need {} inputs, own {}",
            count,
            bond_utxo_amounts.len()
        ))
    }
}
