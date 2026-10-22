//! SlashProducer admission: the accused's producer-set state, and the slash
//! rules judged at the height the transaction would be mined at.

use crypto::PublicKey;
use doli_core::validation::{validate_slash_target, SlashTargetState, ValidationContext};
use doli_core::{Transaction, TxType};
use storage::{PendingProducerUpdate, ProducerSet};

use crate::holdings::HoldingsSources;
use crate::MempoolError;

/// Producer-set state of the accused of every SlashProducer in `txs`. A key
/// that is not a registered producer is left out.
pub fn slash_targets<'a>(
    producers: &ProducerSet,
    txs: impl IntoIterator<Item = &'a Transaction>,
) -> Vec<(PublicKey, SlashTargetState)> {
    txs.into_iter()
        .filter_map(|tx| tx.slash_data())
        .filter_map(|data| {
            let pk = data.producer_pubkey;
            let info = producers.get_by_pubkey(&pk)?;
            let slash_queued = producers
                .pending_updates_for(&pk)
                .iter()
                .any(|u| matches!(u, PendingProducerUpdate::Slash { .. }));
            let state = SlashTargetState {
                active: info.is_active(),
                slashed: info.is_slashed(),
                slash_queued,
            };
            Some((pk, state))
        })
        .collect()
}

/// For a SlashProducer: `ctx` at the next height, carrying the accused's
/// state, after the shared state check. `None` for any other type. When the
/// live producer set cannot be read without blocking, the state check is
/// skipped (fail open); the evidence rules still run in the caller.
pub(crate) fn admission_ctx(
    tx: &Transaction,
    ctx: &ValidationContext,
    sources: &HoldingsSources,
) -> Result<Option<ValidationContext>, MempoolError> {
    if tx.tx_type != TxType::SlashProducer {
        return Ok(None);
    }
    let mut mined_at = ctx.clone();
    mined_at.current_height = ctx.current_height.saturating_add(1);
    let targets = sources
        .live
        .as_ref()
        .and_then(|set| set.try_read().ok())
        .map(|producers| slash_targets(&producers, [tx]));
    let Some(targets) = targets else {
        return Ok(Some(mined_at));
    };
    let mined_at = mined_at.with_slash_targets(targets);
    validate_slash_target(tx, &mined_at)?;
    Ok(Some(mined_at))
}
