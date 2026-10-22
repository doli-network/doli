//! Input spendability at admission: the consensus spend-lock verdict judged at
//! the mined-at height `tip + 1`, then coinbase maturity judged at `tip`.

use doli_core::network::Network;
use doli_core::transaction::Output;
use doli_core::validation::spend_lock_verdict;
use doli_core::{BlockHeight, TxType};
use storage::UtxoEntry;

use crate::MempoolError;

/// Lock verdict for a spent output (UTXO-set entry or in-mempool parent output).
pub(crate) fn check_output(
    network: Network,
    tx_type: TxType,
    spent: &Output,
    tip: BlockHeight,
) -> Result<(), MempoolError> {
    let gate = network.params().inc_i_257_unbonding_activation_height;
    spend_lock_verdict(tx_type, spent, tip.saturating_add(1), gate)
        .map_err(|e| MempoolError::InvalidTransaction(format!("[MPTX009] {e}")))
}

/// Lock verdict, then maturity of a minted entry (RequestWithdrawal and Exit skip maturity).
pub(crate) fn check_utxo(
    network: Network,
    tx_type: TxType,
    utxo: &UtxoEntry,
    tip: BlockHeight,
) -> Result<(), MempoolError> {
    check_output(network, tx_type, &utxo.output, tip)?;
    let maturity = network.coinbase_maturity();
    if (utxo.is_coinbase || utxo.is_epoch_reward)
        && !matches!(tx_type, TxType::RequestWithdrawal | TxType::Exit)
        && tip.saturating_sub(utxo.height) < maturity
    {
        return Err(MempoolError::InvalidTransaction(format!(
            "[MPTX009] output not spendable until height {} (current={tip}, maturity={maturity})",
            utxo.height.saturating_add(maturity)
        )));
    }
    Ok(())
}
