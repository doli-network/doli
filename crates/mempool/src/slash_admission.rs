//! SlashProducer admission: the accused's producer-set state, and the slash
//! rules judged at the height the transaction would be mined at.

use crypto::{Hash, PublicKey};
use doli_core::validation::{
    bond_withdrawal_producer, validate_slash_target, SlashTargetState, ValidationContext,
    ValidationError,
};
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

/// RequestWithdrawal mirror of the block rules, judged at `mined_at`: refused
/// when its producer has a slash in `resident`, is Slashed, or has a queued
/// Slash. The producer-set arm fails open when the live set is locked.
pub(crate) fn withdrawal_refusal<'a>(
    producer: &PublicKey,
    mined_at: u64,
    gate: u64,
    sources: &HoldingsSources,
    resident: impl IntoIterator<Item = &'a Transaction>,
) -> Result<(), String> {
    if mined_at < gate {
        return Ok(());
    }
    let beside_slash = resident
        .into_iter()
        .filter_map(Transaction::slash_data)
        .any(|d| d.producer_pubkey == *producer);
    if beside_slash {
        return Err(format!(
            "RequestWithdrawal by producer {} with a resident slash",
            producer
        ));
    }
    let refused = sources
        .live
        .as_ref()
        .and_then(|set| set.try_read().ok())
        .is_some_and(|producers| producers.is_slashed_or_slash_queued(producer));
    if refused {
        return Err(format!(
            "RequestWithdrawal by slashed or slash-queued producer {}",
            producer
        ));
    }
    Ok(())
}

/// The accused of slash `tx` when judged at/above the slash gate at
/// `mined_at` (from `admission_ctx`).
fn gated_accused(tx: &Transaction, mined_at: Option<&ValidationContext>) -> Option<PublicKey> {
    let mined_at = mined_at?;
    if mined_at.current_height < mined_at.inc_i_255_slashing_activation_height {
        return None;
    }
    tx.slash_data().map(|d| d.producer_pubkey)
}

/// Resident RequestWithdrawals naming the gated accused of slash `tx`.
pub(crate) fn withdrawals_for_slash<'a>(
    tx: &Transaction,
    mined_at: Option<&ValidationContext>,
    resident: impl IntoIterator<Item = (&'a Hash, &'a Transaction)>,
) -> Vec<Hash> {
    let Some(accused) = gated_accused(tx, mined_at) else {
        return Vec::new();
    };
    resident
        .into_iter()
        .filter(|(_, r)| bond_withdrawal_producer(r) == Some(accused))
        .map(|(hash, _)| *hash)
        .collect()
}

/// At/above the slash gate judged at `mined_at` (from `admission_ctx`),
/// refuse a slash whose accused already has one resident.
pub(crate) fn resident_refusal<'a>(
    tx: &Transaction,
    mined_at: Option<&ValidationContext>,
    resident: impl IntoIterator<Item = &'a Transaction>,
) -> Result<(), ValidationError> {
    let Some(accused) = gated_accused(tx, mined_at) else {
        return Ok(());
    };
    let taken = resident
        .into_iter()
        .filter_map(Transaction::slash_data)
        .any(|d| d.producer_pubkey == accused);
    if taken {
        return Err(ValidationError::InvalidSlash(
            "slash target already has a resident slash".to_string(),
        ));
    }
    Ok(())
}

/// Resident slashes whose accused is Slashed or has a queued Slash, judged at
/// `mined_at`. Empty below the gate or when the live producer set is locked.
pub(crate) fn stale_slashes<'a>(
    resident: impl IntoIterator<Item = (&'a Hash, &'a Transaction)>,
    mined_at: u64,
    gate: u64,
    sources: &HoldingsSources,
) -> Vec<Hash> {
    if mined_at < gate {
        return Vec::new();
    }
    let Some(producers) = sources.live.as_ref().and_then(|set| set.try_read().ok()) else {
        return Vec::new();
    };
    resident
        .into_iter()
        .filter(|(_, tx)| {
            tx.slash_data()
                .is_some_and(|d| producers.is_slashed_or_slash_queued(&d.producer_pubkey))
        })
        .map(|(hash, _)| *hash)
        .collect()
}
