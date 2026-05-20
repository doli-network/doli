//! Small helper functions for apply_block — extracted to keep mod.rs under 500 lines.

use doli_core::{Block, TxType};

/// Returns true if the block carries any transaction that mid-epoch mutates
/// the `ProducerSet` (registrations, bond changes, exits, delegation changes).
/// Used by the INC-I-071 fix to decide whether the per-block undo entry needs
/// a full ProducerSet snapshot or can use the empty-Vec sentinel.
///
/// Per CLAUDE.md: producer mutations driven by these tx types are DEFERRED
/// to the next epoch boundary, but they still mark the producer set as
/// pending-dirty — the safe rule is to snapshot whenever such a tx is present.
pub(super) fn block_mutates_producer_set(block: &Block) -> bool {
    block.transactions.iter().any(|tx| {
        matches!(
            tx.tx_type,
            TxType::Registration
                | TxType::Exit
                | TxType::AddBond
                | TxType::RequestWithdrawal
                | TxType::ClaimWithdrawal
                | TxType::DelegateBond
                | TxType::RevokeDelegation
        )
    })
}
