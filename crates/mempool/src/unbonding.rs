//! Input spendability at admission: the consensus spend-lock verdict judged at
//! the mined-at height `tip + 1`, then coinbase maturity judged at `tip`.
//! The INC-I-257 output rules are also judged at `tip + 1`, on admission and on revalidate.

use crypto::Hash;
use doli_core::consensus::ConsensusParams;
use doli_core::network::Network;
use doli_core::transaction::Output;
use doli_core::validation::producer_auth::check_producer_auth;
use doli_core::validation::{check_unbonding_rules, spend_lock_verdict, ValidationContext};
use doli_core::{BlockHeight, Transaction, TxType};
use storage::UtxoEntry;
use tracing::info;

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

/// Residents that fail at `tip + 1`: the INC-I-257 output rules (RequestWithdrawal)
/// or the INC-I-182 producer-auth rule (Registration, AddBond, RequestWithdrawal).
pub(crate) fn unmineable_producer_txs<'a>(
    entries: impl IntoIterator<Item = (&'a Hash, &'a Transaction)>,
    params: &ConsensusParams,
    network: Network,
    tip: BlockHeight,
) -> Vec<Hash> {
    let p = network.params();
    let ctx = ValidationContext::new(params.clone(), network, 0, tip)
        .with_inc_i_255_slashing_activation_height(p.effective_slashing_activation_height())
        .with_inc_i_257_unbonding(p.inc_i_257_unbonding_activation_height, p.unbonding_period)
        .with_inc_i_182_producer_auth(p.inc_i_182_producer_auth_activation_height)
        .with_inc_i_257_rules_at_next_height();
    entries
        .into_iter()
        .filter_map(|(hash, tx)| {
            let reason = match tx.tx_type {
                TxType::RequestWithdrawal => check_unbonding_rules(tx, &ctx)
                    .and_then(|()| check_producer_auth(tx, &ctx))
                    .err()?,
                TxType::Registration | TxType::AddBond => check_producer_auth(tx, &ctx).err()?,
                _ => return None,
            };
            info!("evicting producer tx {hash} — {reason}");
            Some(*hash)
        })
        .collect()
}
