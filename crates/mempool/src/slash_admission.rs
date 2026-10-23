//! SlashProducer admission: the accused's producer-set state, and the slash
//! rules judged at the height the transaction would be mined at.

use std::collections::HashSet;

use crypto::{Hash, PublicKey};
use doli_core::consensus::MAX_SLASH_TXS_PER_BLOCK;
use doli_core::transaction::OutputType;
use doli_core::validation::{
    bond_owner, bond_withdrawal_producer, is_slash_burnable, spends_owned_bond,
    validate_slash_target, SlashTargetState, ValidationContext, ValidationError,
};
use doli_core::{Transaction, TxType};
use storage::{PendingProducerUpdate, ProducerSet, UtxoSet};

use crate::holdings::HoldingsSources;
use crate::MempoolError;

/// Producer-set state of the accused of every SlashProducer in `txs`, with
/// `locked_collateral` judged at `height`. A key that is not a registered
/// producer is left out. One owner lookup per registered accused, at most
/// `MAX_SLASH_TXS_PER_BLOCK` per call; past that `locked_collateral` is false.
pub fn slash_targets<'a>(
    producers: &ProducerSet,
    utxos: &UtxoSet,
    height: u64,
    txs: impl IntoIterator<Item = &'a Transaction>,
) -> Vec<(PublicKey, SlashTargetState)> {
    let mut lookups = 0usize;
    txs.into_iter()
        .filter_map(|tx| tx.slash_data())
        .filter_map(|data| {
            let pk = data.producer_pubkey;
            let info = producers.get_by_pubkey(&pk)?;
            let slash_queued = producers
                .pending_updates_for(&pk)
                .iter()
                .any(|u| matches!(u, PendingProducerUpdate::Slash { .. }));
            let locked_collateral = lookups < MAX_SLASH_TXS_PER_BLOCK && {
                lookups += 1;
                utxos
                    .get_by_pubkey_hash(&bond_owner(&pk))
                    .iter()
                    .any(|(_, e)| {
                        e.output.output_type == OutputType::Unbonding
                            && is_slash_burnable(&e.output, height)
                    })
            };
            let state = SlashTargetState {
                active: info.is_active(),
                slashed: info.is_slashed(),
                slash_queued,
                locked_collateral,
                registered_at: info.registered_at,
            };
            Some((pk, state))
        })
        .collect()
}

/// For a SlashProducer: `ctx` at the next height, carrying the accused's
/// state, after the shared state check. `None` for any other type. When the
/// live producer set cannot be read without blocking, a slash judged at/above
/// the gate is refused with the transient `Busy`; below the gate, or with no
/// live set wired, the state check is skipped.
pub(crate) fn admission_ctx(
    tx: &Transaction,
    ctx: &ValidationContext,
    sources: &HoldingsSources,
    utxo_set: &UtxoSet,
) -> Result<Option<ValidationContext>, MempoolError> {
    if tx.tx_type != TxType::SlashProducer {
        return Ok(None);
    }
    let mut mined_at = ctx.clone();
    mined_at.current_height = ctx.current_height.saturating_add(1);
    let Some(set) = sources.live.as_ref() else {
        return Ok(Some(mined_at));
    };
    let Ok(producers) = set.try_read() else {
        if mined_at.current_height >= mined_at.inc_i_255_slashing_activation_height {
            return Err(MempoolError::Busy);
        }
        return Ok(Some(mined_at));
    };
    let targets = slash_targets(&producers, utxo_set, mined_at.current_height, [tx]);
    drop(producers);
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

/// Resident RequestWithdrawals naming the gated accused of slash `tx`, and
/// resident txs spending a Bond UTXO in `utxo_set` owned by that accused.
pub(crate) fn withdrawals_for_slash<'a>(
    tx: &Transaction,
    mined_at: Option<&ValidationContext>,
    resident: impl IntoIterator<Item = (&'a Hash, &'a Transaction)>,
    utxo_set: &UtxoSet,
) -> Vec<Hash> {
    let Some(accused) = gated_accused(tx, mined_at) else {
        return Vec::new();
    };
    let owners = HashSet::from([bond_owner(&accused)]);
    resident
        .into_iter()
        .filter(|(_, r)| {
            bond_withdrawal_producer(r) == Some(accused) || spends_owned_bond(r, &owners, utxo_set)
        })
        .map(|(hash, _)| *hash)
        .collect()
}

/// Bond owners of the accused of the slashes in `resident`, judged at
/// `mined_at`; empty below `gate`.
pub(crate) fn slash_bond_owners<'a>(
    resident: impl IntoIterator<Item = &'a Transaction>,
    mined_at: u64,
    gate: u64,
) -> HashSet<Hash> {
    if mined_at < gate {
        return HashSet::new();
    }
    resident
        .into_iter()
        .filter_map(Transaction::slash_data)
        .map(|d| bond_owner(&d.producer_pubkey))
        .collect()
}

/// Refuses `tx` when it spends a Bond UTXO in `utxo_set` owned by one of
/// `owners` (from `slash_bond_owners`).
pub(crate) fn bond_spend_refusal(
    tx: &Transaction,
    owners: &HashSet<Hash>,
    utxo_set: &UtxoSet,
) -> Result<(), String> {
    if spends_owned_bond(tx, owners, utxo_set) {
        return Err(format!(
            "tx {} is a spend of the accused's bond with a resident slash",
            tx.hash()
        ));
    }
    Ok(())
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

/// Resident slashes that fail `validate_slash_target` judged at the height
/// after `ctx` (the tip). Empty below the gate or when the live producer set
/// is locked.
pub(crate) fn stale_slashes<'a>(
    resident: impl IntoIterator<Item = (&'a Hash, &'a Transaction)>,
    ctx: &ValidationContext,
    sources: &HoldingsSources,
    utxo_set: &UtxoSet,
) -> Vec<Hash> {
    let mut mined_at = ctx.clone();
    mined_at.current_height = ctx.current_height.saturating_add(1);
    if mined_at.current_height < mined_at.inc_i_255_slashing_activation_height {
        return Vec::new();
    }
    let Some(producers) = sources.live.as_ref().and_then(|set| set.try_read().ok()) else {
        return Vec::new();
    };
    let slashes: Vec<(&Hash, &Transaction)> = resident
        .into_iter()
        .filter(|(_, tx)| tx.tx_type == TxType::SlashProducer)
        .collect();
    let targets: Vec<_> = slashes
        .iter()
        .flat_map(|(_, tx)| slash_targets(&producers, utxo_set, mined_at.current_height, [*tx]))
        .collect();
    drop(producers);
    let mined_at = mined_at.with_slash_targets(targets);
    slashes
        .into_iter()
        .filter(|(_, tx)| validate_slash_target(tx, &mined_at).is_err())
        .map(|(hash, _)| *hash)
        .collect()
}
