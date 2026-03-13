//! Channel state machine — validates transitions between channel states.
//!
//! Every state transition must go through `validate_transition()` before
//! being applied. This prevents invalid states like closing an unopened channel.

use crate::error::{ChannelError, Result};
use crate::types::ChannelState;

/// Validate that a state transition is allowed.
pub fn validate_transition(from: &ChannelState, to: &ChannelState) -> Result<()> {
    let valid = match (from, to) {
        // Opening flow
        (ChannelState::Opening, ChannelState::FundingSigned) => true,
        (ChannelState::FundingSigned, ChannelState::FundingBroadcast) => true,
        (ChannelState::FundingBroadcast, ChannelState::Active) => true,

        // From Active: cooperative or unilateral close
        (ChannelState::Active, ChannelState::CooperativeClosing) => true,
        (ChannelState::Active, ChannelState::ForceClosing) => true,
        (ChannelState::Active, ChannelState::CounterpartyClosing) => true,
        (ChannelState::Active, ChannelState::PenaltyInFlight) => true,

        // Cooperative close → closed
        (ChannelState::CooperativeClosing, ChannelState::Closed) => true,

        // Force close → awaiting claim → closed
        (ChannelState::ForceClosing, ChannelState::AwaitingClaim) => true,
        (ChannelState::ForceClosing, ChannelState::Closed) => true,

        // Counterparty close → penalty or awaiting claim
        (ChannelState::CounterpartyClosing, ChannelState::PenaltyInFlight) => true,
        (ChannelState::CounterpartyClosing, ChannelState::AwaitingClaim) => true,
        (ChannelState::CounterpartyClosing, ChannelState::Closed) => true,

        // Awaiting claim → closed
        (ChannelState::AwaitingClaim, ChannelState::Closed) => true,

        // Penalty → closed
        (ChannelState::PenaltyInFlight, ChannelState::Closed) => true,

        // Opening can abort
        (ChannelState::Opening, ChannelState::Closed) => true,
        (ChannelState::FundingSigned, ChannelState::Closed) => true,
        (ChannelState::FundingBroadcast, ChannelState::Closed) => true,

        _ => false,
    };

    if valid {
        Ok(())
    } else {
        Err(ChannelError::InvalidTransition {
            from: from.to_string(),
            to: to.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_opening_flow() {
        assert!(validate_transition(&ChannelState::Opening, &ChannelState::FundingSigned).is_ok());
        assert!(validate_transition(
            &ChannelState::FundingSigned,
            &ChannelState::FundingBroadcast
        )
        .is_ok());
        assert!(
            validate_transition(&ChannelState::FundingBroadcast, &ChannelState::Active).is_ok()
        );
    }

    #[test]
    fn valid_cooperative_close() {
        assert!(
            validate_transition(&ChannelState::Active, &ChannelState::CooperativeClosing).is_ok()
        );
        assert!(
            validate_transition(&ChannelState::CooperativeClosing, &ChannelState::Closed).is_ok()
        );
    }

    #[test]
    fn valid_force_close() {
        assert!(validate_transition(&ChannelState::Active, &ChannelState::ForceClosing).is_ok());
        assert!(
            validate_transition(&ChannelState::ForceClosing, &ChannelState::AwaitingClaim).is_ok()
        );
        assert!(validate_transition(&ChannelState::AwaitingClaim, &ChannelState::Closed).is_ok());
    }

    #[test]
    fn valid_penalty_flow() {
        assert!(validate_transition(&ChannelState::Active, &ChannelState::PenaltyInFlight).is_ok());
        assert!(validate_transition(&ChannelState::PenaltyInFlight, &ChannelState::Closed).is_ok());
    }

    #[test]
    fn invalid_transitions_rejected() {
        // Can't go from Closed to anything
        assert!(validate_transition(&ChannelState::Closed, &ChannelState::Active).is_err());
        // Can't skip states
        assert!(validate_transition(&ChannelState::Opening, &ChannelState::Active).is_err());
        // Can't go backwards
        assert!(validate_transition(&ChannelState::Active, &ChannelState::Opening).is_err());
    }

    #[test]
    fn abort_from_early_states() {
        assert!(validate_transition(&ChannelState::Opening, &ChannelState::Closed).is_ok());
        assert!(validate_transition(&ChannelState::FundingSigned, &ChannelState::Closed).is_ok());
        assert!(
            validate_transition(&ChannelState::FundingBroadcast, &ChannelState::Closed).is_ok()
        );
    }

    #[test]
    fn counterparty_close_to_penalty() {
        assert!(validate_transition(
            &ChannelState::CounterpartyClosing,
            &ChannelState::PenaltyInFlight
        )
        .is_ok());
    }
}
