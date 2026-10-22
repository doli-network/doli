//! Producer-set preconditions of a SlashProducer, evaluated before the
//! validation-mode split so every mode reaches one verdict.

use std::collections::HashSet;

use crypto::PublicKey;

use crate::consensus::MAX_SLASH_TXS_PER_BLOCK;
use crate::transaction::{Transaction, TxType};

use super::slash_evidence::{decode_slash_data, slash_rule_active, validate_slash};
use super::{ValidationContext, ValidationError};

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
/// Slashed, no queued Slash, Active or Unbonding. `Ok` for any other tx type
/// and below the slash rule.
pub fn validate_slash_target(
    tx: &Transaction,
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    if tx.tx_type != TxType::SlashProducer || !slash_rule_active(ctx) {
        return Ok(());
    }
    let accused = validate_slash(tx, ctx, false)?.producer_pubkey;
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
    if !state.active {
        return refuse("slash target is not an active or unbonding producer");
    }
    Ok(())
}

/// The producer whose Bond UTXOs `tx` may spend: the producer a
/// RequestWithdrawal names. `None` for any other type.
pub fn bond_withdrawal_producer(tx: &Transaction) -> Option<PublicKey> {
    tx.withdrawal_request_data().map(|d| d.producer_pubkey)
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
/// an accused; then each target's state.
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
    }
    for tx in txs {
        validate_slash_target(tx, ctx)?;
    }
    Ok(())
}

/// Builder mirror of the block slash rules: the accused already picked for
/// the block under construction.
#[derive(Debug, Default)]
pub struct SlashBlockBudget {
    accused: HashSet<PublicKey>,
}

impl SlashBlockBudget {
    /// True when the slash rule is inactive; for a RequestWithdrawal, when it
    /// names no picked accused; for a slash, when the block has room for a
    /// not-yet-picked accused; true for any other type.
    pub fn fits(&self, tx: &Transaction, ctx: &ValidationContext) -> bool {
        if !slash_rule_active(ctx) {
            return true;
        }
        if let Some(producer) = bond_withdrawal_producer(tx) {
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

    /// Record `tx` as included in the block.
    pub fn accept(&mut self, tx: &Transaction, ctx: &ValidationContext) {
        if tx.tx_type != TxType::SlashProducer || !slash_rule_active(ctx) {
            return;
        }
        if let Ok(data) = decode_slash_data(&tx.extra_data, ctx) {
            self.accused.insert(data.producer_pubkey);
        }
    }
}
