//! Time bounds of DoubleProduction evidence: the unbonding-period window and
//! the accused's tenure floor, both counted in slots up to `prev_slot + 1`.

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

/// Refuse evidence whose age reaches `unbonding_period`, then evidence whose
/// age reaches the blocks since `max(registered_at, slash gate)`.
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
    let floor = state
        .registered_at
        .max(ctx.inc_i_255_slashing_activation_height);
    if age >= ctx.current_height.saturating_sub(floor) {
        return Err(ValidationError::InvalidSlash(PREDATES.to_string()));
    }
    Ok(())
}
