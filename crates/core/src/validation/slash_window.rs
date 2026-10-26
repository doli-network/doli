//! Time bounds of DoubleProduction evidence, judged at `prev_slot + 1`: the
//! unbonding-period window, the slash-gate bound and the registration floor slot.

use crate::transaction::SlashingEvidence;

use super::{SlashTargetState, ValidationContext, ValidationError};

const EXPIRED: &str = "slash evidence expired: offence older than the unbonding period";
const PREDATES: &str = "slash evidence predates the accused's current registration";

impl ValidationContext {
    /// Set the parent (tip) slot; the evidence window judges at `slot + 1`.
    #[must_use]
    pub fn with_prev_slot(mut self, slot: u32) -> Self {
        self.prev_slot = slot;
        self
    }
}

/// Refuse evidence whose age reaches `unbonding_period`; evidence judged in the
/// block at `max(registered_at, slash gate)`; evidence older than the blocks
/// since the gate; and, without locked collateral, an offence slot below
/// `oldest_bond_slot` (or no such slot).
pub(super) fn validate_slash_window(
    evidence: &SlashingEvidence,
    state: &SlashTargetState,
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    let SlashingEvidence::DoubleProduction { block_header_1, .. } = evidence else {
        return Ok(());
    };
    let judged = ctx.prev_slot.saturating_add(1);
    let age = u64::from(judged.saturating_sub(block_header_1.slot));
    if age >= ctx.unbonding_period {
        return Err(ValidationError::InvalidSlash(EXPIRED.to_string()));
    }
    let gate = ctx.inc_i_255_slashing_activation_height;
    let tenure = ctx
        .current_height
        .saturating_sub(state.registered_at.max(gate));
    // Slots strictly increase, so the gate block's slot is at most
    // `judged - (current_height - gate)`.
    let before_gate = age > ctx.current_height.saturating_sub(gate);
    let before_bonds = !state.locked_collateral
        && state
            .oldest_bond_slot
            .is_none_or(|floor| block_header_1.slot < floor);
    if tenure == 0 || before_gate || before_bonds {
        return Err(ValidationError::InvalidSlash(PREDATES.to_string()));
    }
    Ok(())
}
