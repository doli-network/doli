//! Producer-set preconditions of a SlashProducer, evaluated before the
//! validation-mode split so every mode reaches one verdict.

use crypto::PublicKey;

use crate::transaction::{Transaction, TxType};

use super::slash_evidence::{slash_rule_active, validate_slash};
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
