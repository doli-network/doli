//! Presence score calculation.
//!
//! Calculates a producer's presence score based on their activity history.
//! This is TELEMETRY ONLY and does not affect consensus.

use super::MAX_PRESENCE_SCORE;

/// Calculate the presence score for a producer.
///
/// The score reflects temporal commitment:
/// - Consecutive presence rewards consistency
/// - Historical ratio rewards reliability
/// - Age bonus rewards loyalty (but logarithmically to prevent dynasties)
///
/// # Arguments
/// * `consecutive_slots` - Current streak of consecutive presence
/// * `total_slots_active` - Total slots since registration
/// * `missed_slots` - Slots where expected proof was not provided
/// * `age_in_eras` - Number of complete eras since registration
///
/// # Returns
/// Presence score (capped at MAX_PRESENCE_SCORE)
pub fn calculate_presence_score(
    consecutive_slots: u64,
    total_slots_active: u64,
    missed_slots: u64,
    age_in_eras: u32,
) -> u64 {
    // Base score: consecutive presence (capped contribution)
    // 1 point per consecutive slot, max 10,000 from this component
    let consecutive_bonus = consecutive_slots.min(10_000);

    // Historical presence ratio (0-100 points per 1000 slots)
    // This rewards reliability over time
    let presence_ratio = if total_slots_active > 0 {
        let present = total_slots_active.saturating_sub(missed_slots);
        (present * 100) / total_slots_active
    } else {
        0
    };
    let history_bonus = (total_slots_active / 1000) * presence_ratio;

    // Age bonus: logarithmic scaling
    // Era 0: 0, Era 1: 5, Era 2: 10, Era 4: 15, Era 8: 20, ...
    let age_bonus = if age_in_eras > 0 {
        ((age_in_eras as u64).ilog2() as u64 + 1) * 5
    } else {
        0
    };

    // Combine components
    let score = consecutive_bonus
        .saturating_add(history_bonus)
        .saturating_add(age_bonus);

    // Cap at maximum
    score.min(MAX_PRESENCE_SCORE)
}
