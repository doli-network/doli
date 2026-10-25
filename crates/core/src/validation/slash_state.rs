//! Producer-set preconditions of a SlashProducer, evaluated before the
//! validation-mode split so every mode reaches one verdict.

use std::collections::HashSet;

use crypto::hash::hash_with_domain;
use crypto::{Hash, PublicKey, ADDRESS_DOMAIN};

use crate::consensus::MAX_SLASH_TXS_PER_BLOCK;
use crate::transaction::{Output, OutputType, Transaction, TxType};

use super::slash_evidence::{decode_slash_data, slash_rule_active, validate_slash};
use super::slash_registration::slash_registrant;
use super::slash_window::validate_slash_window;
use super::{UtxoProvider, ValidationContext, ValidationError};

/// Producer-set facts about one accused key. A key absent from
/// `ValidationContext::slash_targets` is not a registered producer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SlashTargetState {
    /// Status is Active or Unbonding.
    pub active: bool,
    /// Status is Slashed.
    pub slashed: bool,
    /// A Slash for this key is queued in the pending producer updates.
    pub slash_queued: bool,
    /// The bond owner holds an Unbonding UTXO still locked at the judged height.
    pub locked_collateral: bool,
    /// Height of the accused's current registration.
    pub registered_at: u64,
    /// Smallest stamped slot of the bond owner's Bond UTXOs; `None` when none or not looked up.
    pub oldest_bond_slot: Option<u32>,
}

/// True when a slash at `height` burns `output`: every Bond, and an Unbonding
/// output while `height < lock_until`.
pub fn is_slash_burnable(output: &Output, height: u64) -> bool {
    match output.output_type {
        OutputType::Bond => true,
        OutputType::Unbonding => height < output.lock_until,
        _ => false,
    }
}

impl ValidationContext {
    /// Set the slashing gate; production sites pass
    /// `NetworkParams::effective_slashing_activation_height()`.
    #[must_use]
    pub fn with_inc_i_255_slashing_activation_height(mut self, height: u64) -> Self {
        self.inc_i_255_slashing_activation_height = height;
        self
    }

    /// Set the producer-set state of the accused keys under validation.
    #[must_use]
    pub fn with_slash_targets(
        mut self,
        targets: impl IntoIterator<Item = (PublicKey, SlashTargetState)>,
    ) -> Self {
        self.slash_targets = targets.into_iter().collect();
        self
    }
}

/// Evidence check (VDF excluded) then the accused's state: registered, not
/// Slashed, no queued Slash, Active or Unbonding or holding locked collateral;
/// then the evidence window. `Ok` for any other tx type and below the slash rule.
pub fn validate_slash_target(
    tx: &Transaction,
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    if tx.tx_type != TxType::SlashProducer || !slash_rule_active(ctx) {
        return Ok(());
    }
    let data = validate_slash(tx, ctx, false)?;
    let accused = data.producer_pubkey;
    let refuse = |msg: &str| Err(ValidationError::InvalidSlash(msg.to_string()));
    let Some(state) = ctx.slash_targets.get(&accused) else {
        return refuse("slash target is not a registered producer");
    };
    if state.slashed {
        return refuse("slash target is already slashed");
    }
    if state.slash_queued {
        return refuse("slash target already has a pending slash");
    }
    if !state.active && !state.locked_collateral {
        return refuse("slash target is not an active or unbonding producer");
    }
    validate_slash_window(&data.evidence, state, ctx)
}

/// The producer whose Bond UTXOs `tx` may spend: the producer a
/// RequestWithdrawal names. `None` for any other type.
pub fn bond_withdrawal_producer(tx: &Transaction) -> Option<PublicKey> {
    tx.withdrawal_request_data().map(|d| d.producer_pubkey)
}

/// Owner of the Bond UTXOs that a slash of `producer` burns.
pub fn bond_owner(producer: &PublicKey) -> Hash {
    hash_with_domain(ADDRESS_DOMAIN, producer.as_bytes())
}

/// True when an input of `tx` spends a Bond UTXO in `utxos` owned by one of
/// `owners`. No lookup when `owners` is empty.
pub fn spends_owned_bond<U: UtxoProvider + ?Sized>(
    tx: &Transaction,
    owners: &HashSet<Hash>,
    utxos: &U,
) -> bool {
    !owners.is_empty()
        && tx.inputs.iter().any(|input| {
            utxos
                .get_utxo(&input.prev_tx_hash, input.output_index)
                .is_some_and(|u| {
                    u.output.output_type == OutputType::Bond
                        && owners.contains(&u.output.pubkey_hash)
                })
        })
}

/// From the slash rule, a block that holds a slash for p spends no Bond UTXO
/// owned by p: neither one in the pre-block `utxos` nor one created by an
/// earlier tx of the block. Any tx type, any order.
pub fn validate_slash_bond_spends<U: UtxoProvider + ?Sized>(
    txs: &[Transaction],
    ctx: &ValidationContext,
    utxos: &U,
) -> Result<(), ValidationError> {
    if !slash_rule_active(ctx) {
        return Ok(());
    }
    let owners: HashSet<Hash> = txs
        .iter()
        .filter(|tx| tx.tx_type == TxType::SlashProducer)
        .filter_map(|tx| decode_slash_data(&tx.extra_data, ctx).ok())
        .map(|data| bond_owner(&data.producer_pubkey))
        .collect();
    if owners.is_empty() {
        return Ok(());
    }
    let mut created: HashSet<(Hash, u32)> = HashSet::new();
    for tx in txs {
        let spends_created = tx
            .inputs
            .iter()
            .any(|i| created.contains(&(i.prev_tx_hash, i.output_index)));
        if spends_created || spends_owned_bond(tx, &owners, utxos) {
            return Err(ValidationError::InvalidSlash(
                "block holds a slash and a spend of the accused's bond".to_string(),
            ));
        }
        let bonds: Vec<u32> = (0u32..)
            .zip(&tx.outputs)
            .filter(|(_, o)| o.output_type == OutputType::Bond && owners.contains(&o.pubkey_hash))
            .map(|(index, _)| index)
            .collect();
        if !bonds.is_empty() {
            let hash = tx.hash();
            created.extend(bonds.into_iter().map(|index| (hash, index)));
        }
    }
    Ok(())
}

/// Builder candidate order: at/above the slash rule every slash comes first
/// (stable); below it the order of `txs` is kept.
pub fn slashes_first<'a>(txs: &'a [Transaction], ctx: &ValidationContext) -> Vec<&'a Transaction> {
    let mut order: Vec<&Transaction> = txs.iter().collect();
    if slash_rule_active(ctx) {
        order.sort_by_key(|tx| tx.tx_type != TxType::SlashProducer);
    }
    order
}

/// Block-level slash rules: at most `MAX_SLASH_TXS_PER_BLOCK` slashes and one
/// per accused, counted before any evidence check; no RequestWithdrawal naming
/// an accused, no Registration / AddBond for one (`[ERRTX071]`); then each
/// target's state.
pub(super) fn validate_block_slashes(
    txs: &[Transaction],
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    if slash_rule_active(ctx) {
        let slashes: Vec<&Transaction> = txs
            .iter()
            .filter(|tx| tx.tx_type == TxType::SlashProducer)
            .collect();
        if slashes.len() > MAX_SLASH_TXS_PER_BLOCK {
            return Err(ValidationError::InvalidSlash(format!(
                "block holds {} slashes (max {})",
                slashes.len(),
                MAX_SLASH_TXS_PER_BLOCK
            )));
        }
        let mut accused = HashSet::new();
        for data in slashes
            .iter()
            .filter_map(|tx| decode_slash_data(&tx.extra_data, ctx).ok())
        {
            if !accused.insert(data.producer_pubkey) {
                return Err(ValidationError::InvalidSlash(
                    "block holds two slashes for one accused".to_string(),
                ));
            }
        }
        if txs
            .iter()
            .filter_map(bond_withdrawal_producer)
            .any(|p| accused.contains(&p))
        {
            return Err(ValidationError::InvalidSlash(
                "block holds a slash and a bond withdrawal for one producer".to_string(),
            ));
        }
        if let Some(key) = txs
            .iter()
            .filter_map(slash_registrant)
            .find(|k| accused.contains(k))
        {
            return Err(ValidationError::SlashedRegistrant {
                producer: key.to_hex(),
            });
        }
    }
    for tx in txs {
        validate_slash_target(tx, ctx)?;
    }
    Ok(())
}

/// Builder mirror of the block slash rules: the accused already picked for
/// the block under construction, and the owner of their Bond UTXOs.
#[derive(Debug, Default)]
pub struct SlashBlockBudget {
    accused: HashSet<PublicKey>,
    owners: HashSet<Hash>,
}

impl SlashBlockBudget {
    /// True when the slash rule is inactive; for a RequestWithdrawal,
    /// Registration or AddBond, when it names no picked accused; for a slash,
    /// when the block has room for a not-yet-picked accused; true otherwise.
    pub fn fits(&self, tx: &Transaction, ctx: &ValidationContext) -> bool {
        if !slash_rule_active(ctx) {
            return true;
        }
        if let Some(producer) = bond_withdrawal_producer(tx).or_else(|| slash_registrant(tx)) {
            return !self.accused.contains(&producer);
        }
        if tx.tx_type != TxType::SlashProducer {
            return true;
        }
        let Ok(data) = decode_slash_data(&tx.extra_data, ctx) else {
            return false;
        };
        self.accused.len() < MAX_SLASH_TXS_PER_BLOCK
            && !self.accused.contains(&data.producer_pubkey)
    }

    /// True when `tx` spends a Bond UTXO in `utxos` owned by a picked accused.
    pub fn spends_accused_bond<U: UtxoProvider + ?Sized>(
        &self,
        tx: &Transaction,
        utxos: &U,
    ) -> bool {
        spends_owned_bond(tx, &self.owners, utxos)
    }

    /// Record `tx` as included in the block.
    pub fn accept(&mut self, tx: &Transaction, ctx: &ValidationContext) {
        if tx.tx_type != TxType::SlashProducer || !slash_rule_active(ctx) {
            return;
        }
        if let Ok(data) = decode_slash_data(&tx.extra_data, ctx) {
            self.accused.insert(data.producer_pubkey);
            self.owners.insert(bond_owner(&data.producer_pubkey));
        }
    }
}
