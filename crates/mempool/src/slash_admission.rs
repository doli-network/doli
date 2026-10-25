//! SlashProducer admission: the accused's producer-set state, and the slash
//! rules judged at the height the transaction would be mined at.

use std::collections::HashSet;

use crypto::{Hash, PublicKey};
use doli_core::consensus::MAX_SLASH_TXS_PER_BLOCK;
use doli_core::transaction::OutputType;
use doli_core::validation::{
    bond_owner, bond_withdrawal_producer, is_slash_burnable, slash_fee_exempt, slash_registrant,
    spends_owned_bond, validate_not_slashed_registrant, validate_slash_target, SlashTargetState,
    ValidationContext, ValidationError,
};
use doli_core::{Transaction, TxType};
use storage::{PendingProducerUpdate, ProducerSet, UtxoSet};

use crate::holdings::HoldingsSources;
use crate::MempoolError;

/// Producer-set state of the accused of every SlashProducer in `txs`, with
/// `locked_collateral` judged at `height`, then of each Registration / AddBond
/// key not already named (`slash_registrant`, no UTXO read). A key that is not
/// a registered producer is left out. One owner lookup per registered accused,
/// at most `MAX_SLASH_TXS_PER_BLOCK` per call; past that `locked_collateral` is false.
pub fn slash_targets<'a>(
    producers: &ProducerSet,
    utxos: &UtxoSet,
    height: u64,
    txs: impl IntoIterator<Item = &'a Transaction>,
) -> Vec<(PublicKey, SlashTargetState)> {
    let txs: Vec<&Transaction> = txs.into_iter().collect();
    let mut lookups = 0usize;
    let mut targets: Vec<(PublicKey, SlashTargetState)> = txs
        .iter()
        .filter_map(|tx| tx.slash_data())
        .filter_map(|data| {
            let pk = data.producer_pubkey;
            let mut state = target_state(producers, &pk)?;
            state.locked_collateral = lookups < MAX_SLASH_TXS_PER_BLOCK && {
                lookups += 1;
                utxos
                    .get_by_pubkey_hash(&bond_owner(&pk))
                    .iter()
                    .any(|(_, e)| {
                        e.output.output_type == OutputType::Unbonding
                            && is_slash_burnable(&e.output, height)
                    })
            };
            Some((pk, state))
        })
        .collect();
    let mut named: HashSet<PublicKey> = targets.iter().map(|(pk, _)| *pk).collect();
    for pk in txs.iter().filter_map(|tx| slash_registrant(tx)) {
        if named.insert(pk) {
            targets.extend(target_state(producers, &pk).map(|state| (pk, state)));
        }
    }
    targets
}

fn target_state(producers: &ProducerSet, pk: &PublicKey) -> Option<SlashTargetState> {
    let info = producers.get_by_pubkey(pk)?;
    let slash_queued = producers
        .pending_updates_for(pk)
        .iter()
        .any(|u| matches!(u, PendingProducerUpdate::Slash { .. }));
    Some(SlashTargetState {
        active: info.is_active(),
        slashed: info.is_slashed(),
        slash_queued,
        locked_collateral: false,
        registered_at: info.registered_at,
    })
}

/// For a SlashProducer: `ctx` at the next height, carrying the accused's
/// state, after the shared state check. The gate and the evidence are checked
/// first, so a slash below the rule or with failing evidence reads no state.
/// When the live producer set cannot be read without blocking, a slash with
/// verified evidence is refused with the transient `Busy`. With no live set
/// wired, the state check is skipped. A Registration / AddBond runs the
/// slashed-registrant rule at the next height (fail-open when the set is
/// locked) and gets `None`, like any other type.
pub(crate) fn admission_ctx(
    tx: &Transaction,
    ctx: &ValidationContext,
    sources: &HoldingsSources,
    utxo_set: &UtxoSet,
) -> Result<Option<ValidationContext>, MempoolError> {
    if matches!(tx.tx_type, TxType::Registration | TxType::AddBond) {
        registrant_refusal(tx, ctx, sources)?;
        return Ok(None);
    }
    if tx.tx_type != TxType::SlashProducer {
        return Ok(None);
    }
    let mut mined_at = ctx.clone();
    mined_at.current_height = ctx.current_height.saturating_add(1);
    let Some(set) = sources.live.as_ref() else {
        return Ok(Some(mined_at));
    };
    if !slash_fee_exempt(tx, &mined_at) {
        validate_slash_target(tx, &mined_at)?;
        return Ok(Some(mined_at));
    }
    let Ok(producers) = set.try_read() else {
        return Err(MempoolError::Busy);
    };
    let targets = slash_targets(&producers, utxo_set, mined_at.current_height, [tx]);
    drop(producers);
    let mined_at = mined_at.with_slash_targets(targets);
    validate_slash_target(tx, &mined_at)?;
    Ok(Some(mined_at))
}

/// The slashed-registrant rule for a Registration / AddBond, judged at the
/// height after `ctx` (the tip). `Ok` below the gate, for any other type, or
/// when the live producer set is locked or absent.
pub(crate) fn registrant_refusal(
    tx: &Transaction,
    ctx: &ValidationContext,
    sources: &HoldingsSources,
) -> Result<(), ValidationError> {
    let height = ctx.current_height.saturating_add(1);
    if height < ctx.inc_i_255_slashing_activation_height {
        return Ok(());
    }
    let Some(key) = slash_registrant(tx) else {
        return Ok(());
    };
    let Some(producers) = sources.live.as_ref().and_then(|set| set.try_read().ok()) else {
        return Ok(());
    };
    let state = target_state(&producers, &key);
    drop(producers);
    let mut mined_at = ctx.clone().with_slash_targets(state.map(|s| (key, s)));
    mined_at.current_height = height;
    validate_not_slashed_registrant(tx, &mined_at)
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

/// Resident RequestWithdrawals and Registrations / AddBonds naming the gated
/// accused of slash `tx`, and resident txs spending a Bond UTXO in `utxo_set`
/// owned by that accused.
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
            bond_withdrawal_producer(r).or_else(|| slash_registrant(r)) == Some(accused)
                || spends_owned_bond(r, &owners, utxo_set)
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

/// At/above the slash gate judged at the height after `ctx` (the tip), refuse
/// a slash whose accused already has one resident, and a Registration /
/// AddBond naming the accused of a resident slash (`[ERRTX071]`).
pub(crate) fn resident_refusal<'a>(
    tx: &Transaction,
    ctx: &ValidationContext,
    resident: impl IntoIterator<Item = &'a Transaction>,
) -> Result<(), ValidationError> {
    if ctx.current_height.saturating_add(1) < ctx.inc_i_255_slashing_activation_height {
        return Ok(());
    }
    let accused = tx.slash_data().map(|d| d.producer_pubkey);
    let Some(key) = accused.or_else(|| slash_registrant(tx)) else {
        return Ok(());
    };
    let taken = resident
        .into_iter()
        .filter_map(Transaction::slash_data)
        .any(|d| d.producer_pubkey == key);
    match (taken, accused) {
        (false, _) => Ok(()),
        (true, Some(_)) => Err(ValidationError::InvalidSlash(
            "slash target already has a resident slash".to_string(),
        )),
        (true, None) => Err(ValidationError::SlashedRegistrant {
            producer: key.to_hex(),
        }),
    }
}

/// Resident slashes that fail `validate_slash_target`, and resident
/// Registrations / AddBonds that fail `validate_not_slashed_registrant`,
/// judged at the height after `ctx` (the tip). Empty below the gate or when
/// the live producer set is locked.
pub(crate) fn slash_rule_evictions<'a>(
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
    let judged: Vec<(&Hash, &Transaction)> = resident
        .into_iter()
        .filter(|(_, tx)| {
            matches!(
                tx.tx_type,
                TxType::SlashProducer | TxType::Registration | TxType::AddBond
            )
        })
        .collect();
    let is_slash = |tx: &Transaction| tx.tx_type == TxType::SlashProducer;
    let h = mined_at.current_height;
    let mut targets = slash_targets(
        &producers,
        utxo_set,
        h,
        judged.iter().map(|(_, tx)| *tx).filter(|tx| !is_slash(tx)),
    );
    // Slash entries last: they carry `locked_collateral` and win the map.
    targets.extend(
        judged
            .iter()
            .filter(|(_, tx)| is_slash(tx))
            .flat_map(|(_, tx)| slash_targets(&producers, utxo_set, h, [*tx])),
    );
    drop(producers);
    let mined_at = mined_at.with_slash_targets(targets);
    judged
        .into_iter()
        .filter(|(_, tx)| {
            validate_slash_target(tx, &mined_at).is_err()
                || validate_not_slashed_registrant(tx, &mined_at).is_err()
        })
        .map(|(hash, _)| *hash)
        .collect()
}
