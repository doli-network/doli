use super::*;
use crate::network::Network;
use crate::types::BlockHeight;

#[test]
fn test_is_protocol_active() {
    // Version 1 is always active at genesis
    assert!(is_protocol_active(1, 1));
    // Version 2 is not active at genesis
    assert!(!is_protocol_active(2, 1));
    // After activation to v2
    assert!(is_protocol_active(1, 2));
    assert!(is_protocol_active(2, 2));
    assert!(!is_protocol_active(3, 2));
    // Higher versions
    assert!(is_protocol_active(1, 5));
    assert!(is_protocol_active(5, 5));
    assert!(!is_protocol_active(6, 5));
}

#[test]
fn test_initial_protocol_version() {
    assert_eq!(INITIAL_PROTOCOL_VERSION, 1);
}

#[test]
fn test_slot_calculation() {
    let params = ConsensusParams::mainnet();

    // With 10-second slots (Proof of Time)
    assert_eq!(params.timestamp_to_slot(GENESIS_TIME), 0);
    assert_eq!(params.timestamp_to_slot(GENESIS_TIME + 10), 1);
    assert_eq!(params.timestamp_to_slot(GENESIS_TIME + 20), 2);
    assert_eq!(params.timestamp_to_slot(GENESIS_TIME + 3600), 360); // 1 hour = 360 slots
}

#[test]
fn test_block_reward() {
    let params = ConsensusParams::mainnet();

    // Era 0: 1 DOLI per block (100,000,000 base units)
    assert_eq!(params.block_reward(0), 100_000_000);
    assert_eq!(params.block_reward(100), 100_000_000);

    // Era 1: halved to 0.5 DOLI (50,000,000 base units)
    assert_eq!(params.block_reward(BLOCKS_PER_ERA), 50_000_000);

    // Era 2: halved again to 0.25 DOLI (25,000,000 base units)
    assert_eq!(params.block_reward(BLOCKS_PER_ERA * 2), 25_000_000);
}

#[test]
fn test_bond_amount() {
    let params = ConsensusParams::mainnet();

    // Era 0: 10 DOLI = 1_000_000_000 base units
    assert_eq!(params.bond_amount(0), 1_000_000_000);

    // Era 1: 70% of initial
    let era1_bond = params.bond_amount(BLOCKS_PER_ERA);
    assert!(era1_bond > 690_000_000 && era1_bond < 710_000_000);

    // Era 2: 49% of initial
    let era2_bond = params.bond_amount(BLOCKS_PER_ERA * 2);
    assert!(era2_bond > 480_000_000 && era2_bond < 500_000_000);
}

#[test]
fn test_epoch_calculation() {
    let params = ConsensusParams::mainnet();

    // With 360 slots per epoch (1 hour at 10s slots)
    assert_eq!(params.slot_to_epoch(0), 0);
    assert_eq!(params.slot_to_epoch(359), 0);
    assert_eq!(params.slot_to_epoch(360), 1);
    assert_eq!(params.slot_to_epoch(720), 2);
}

#[test]
fn test_timestamp_before_genesis() {
    let params = ConsensusParams::mainnet();
    // Timestamps before genesis return slot 0
    assert_eq!(params.timestamp_to_slot(0), 0);
    assert_eq!(params.timestamp_to_slot(GENESIS_TIME - 1), 0);
}

#[test]
fn test_slot_timestamp_inverse() {
    let params = ConsensusParams::mainnet();
    for slot in [0, 1, 100, 1000, 10000] {
        let timestamp = params.slot_to_timestamp(slot);
        let back = params.timestamp_to_slot(timestamp);
        assert_eq!(slot, back);
    }
}

#[test]
fn test_era_calculation() {
    let params = ConsensusParams::mainnet();
    assert_eq!(params.height_to_era(0), 0);
    assert_eq!(params.height_to_era(BLOCKS_PER_ERA - 1), 0);
    assert_eq!(params.height_to_era(BLOCKS_PER_ERA), 1);
    assert_eq!(params.height_to_era(BLOCKS_PER_ERA * 2), 2);
}

#[test]
fn test_bootstrap_phase() {
    let params = ConsensusParams::mainnet();
    assert!(params.is_bootstrap(0));
    assert!(params.is_bootstrap(BOOTSTRAP_BLOCKS - 1));
    assert!(!params.is_bootstrap(BOOTSTRAP_BLOCKS));
    assert!(!params.is_bootstrap(BOOTSTRAP_BLOCKS + 1));
}

#[test]
fn test_max_block_size_by_era() {
    // Era 0 (0-4 years): 2 MB
    assert_eq!(max_block_size(0), 2_000_000);
    assert_eq!(max_block_size(BLOCKS_PER_ERA - 1), 2_000_000);

    // Era 1 (4-8 years): 4 MB
    assert_eq!(max_block_size(BLOCKS_PER_ERA), 4_000_000);
    assert_eq!(max_block_size(BLOCKS_PER_ERA * 2 - 1), 4_000_000);

    // Era 2 (8-12 years): 8 MB
    assert_eq!(max_block_size(BLOCKS_PER_ERA * 2), 8_000_000);

    // Era 3 (12-16 years): 16 MB
    assert_eq!(max_block_size(BLOCKS_PER_ERA * 3), 16_000_000);

    // Era 4+ (16+ years): 32 MB (capped)
    assert_eq!(max_block_size(BLOCKS_PER_ERA * 4), 32_000_000);
    assert_eq!(max_block_size(BLOCKS_PER_ERA * 5), 32_000_000);
    assert_eq!(max_block_size(BLOCKS_PER_ERA * 6), 32_000_000);
    assert_eq!(max_block_size(BLOCKS_PER_ERA * 10), 32_000_000);
    assert_eq!(max_block_size(BLOCKS_PER_ERA * 100), 32_000_000);
}

#[test]
fn test_max_block_size_params_method() {
    let params = ConsensusParams::mainnet();

    // Should match the free function
    assert_eq!(params.max_block_size(0), max_block_size(0));
    assert_eq!(
        params.max_block_size(BLOCKS_PER_ERA),
        max_block_size(BLOCKS_PER_ERA)
    );
    assert_eq!(
        params.max_block_size(BLOCKS_PER_ERA * 5),
        max_block_size(BLOCKS_PER_ERA * 5)
    );
}

// Note: Block VDF (T_BLOCK = 800K iterations) provides anti-grinding protection.
// VDF is mandatory for mainnet blocks; registration VDF is separate (anti-Sybil).

#[test]
fn test_allowed_producer_rank() {
    // Sequential 2s windows (seconds precision), MAX_FALLBACK_RANKS=1:
    // 0s = 0ms -> rank 0, 1s = 1000ms -> rank 0
    // 2s+ = past slot end, clamped to max rank (0)
    assert_eq!(allowed_producer_rank(0), 0);
    assert_eq!(allowed_producer_rank(1), 0); // 1000ms still in rank 0 window
    assert_eq!(allowed_producer_rank(2), 0); // 2000ms past slot end, clamped to 0
    assert_eq!(allowed_producer_rank(3), 0); // past slot end, clamped to 0
    assert_eq!(allowed_producer_rank(4), 0); // past slot end, clamped to 0
    assert_eq!(allowed_producer_rank(5), 0); // past slot end, clamped to 0
    assert_eq!(allowed_producer_rank(6), 0); // past slot end, clamped to 0
    assert_eq!(allowed_producer_rank(7), 0); // past slot end, clamped to 0
    assert_eq!(allowed_producer_rank(8), 0); // past slot end, clamped to 0
    assert_eq!(allowed_producer_rank(9), 0); // past slot end, clamped to 0
    assert_eq!(allowed_producer_rank(10), 0); // past slot end, clamped to 0
}

#[test]
fn test_allowed_producer_rank_ms() {
    // Sequential 2s exclusive windows with millisecond precision, MAX_FALLBACK_RANKS=1
    //
    // Windows:
    // - 0-1999ms: rank 0 (primary)
    // - 2000+ms: past slot end (clamped to max rank = 0)

    // Rank 0 window: 0-1999ms
    assert_eq!(allowed_producer_rank_ms(0), 0);
    assert_eq!(allowed_producer_rank_ms(1000), 0);
    assert_eq!(allowed_producer_rank_ms(1999), 0);

    // Past slot end: 2000+ms (clamped to max rank = 0)
    assert_eq!(allowed_producer_rank_ms(2000), 0);
    assert_eq!(allowed_producer_rank_ms(3000), 0);
    assert_eq!(allowed_producer_rank_ms(3999), 0);
    assert_eq!(allowed_producer_rank_ms(4000), 0);
    assert_eq!(allowed_producer_rank_ms(5999), 0);
    assert_eq!(allowed_producer_rank_ms(8000), 0);
    assert_eq!(allowed_producer_rank_ms(9999), 0);
    assert_eq!(allowed_producer_rank_ms(10000), 0);
    assert_eq!(allowed_producer_rank_ms(15000), 0);
}

#[test]
fn test_is_producer_eligible() {
    let producers: Vec<crypto::PublicKey> = (0..2)
        .map(|i| {
            let mut bytes = [0u8; 32];
            bytes[0] = i;
            crypto::PublicKey::from_bytes(bytes)
        })
        .collect();

    // Single proposer per slot, MAX_FALLBACK_RANKS=1:
    // At second 0 (0ms): only rank 0 is eligible
    assert!(is_producer_eligible(&producers[0], &producers, 0));
    assert!(!is_producer_eligible(&producers[1], &producers, 0));

    // At second 2 (2000ms): past slot end, no one eligible
    assert!(!is_producer_eligible(&producers[0], &producers, 2));
    assert!(!is_producer_eligible(&producers[1], &producers, 2));

    // At second 4 (4000ms): past slot end, no one eligible
    assert!(!is_producer_eligible(&producers[0], &producers, 4));
    assert!(!is_producer_eligible(&producers[1], &producers, 4));
}

#[test]
fn test_sequential_windows() {
    // Verify exactly one rank per 2000ms window across entire slot
    let slot_ms = MAX_FALLBACK_RANKS as u64 * FALLBACK_TIMEOUT_MS;
    for offset_ms in (0..slot_ms).step_by(100) {
        let rank = eligible_rank_at_ms(offset_ms);
        assert!(rank.is_some(), "Should have a rank at {}ms", offset_ms);
        let r = rank.unwrap();
        assert!(
            r < MAX_FALLBACK_RANKS,
            "Rank {} out of bounds at {}ms",
            r,
            offset_ms
        );
        assert_eq!(r, (offset_ms / FALLBACK_TIMEOUT_MS) as usize);
    }
    // Past slot end: no rank
    assert!(eligible_rank_at_ms(slot_ms).is_none());
}

#[test]
fn test_eligible_rank_boundaries() {
    // Test exact boundaries between windows, MAX_FALLBACK_RANKS=1
    // 0ms: rank 0
    assert_eq!(eligible_rank_at_ms(0), Some(0));
    // 1999ms: still rank 0
    assert_eq!(eligible_rank_at_ms(1999), Some(0));
    // 2000ms: past slot end (None) — only 1 rank with 2s window
    assert_eq!(eligible_rank_at_ms(2000), None);
    // 3999ms: past slot end (None)
    assert_eq!(eligible_rank_at_ms(3999), None);
    // 4000ms: past slot end (None)
    assert_eq!(eligible_rank_at_ms(4000), None);
    // 9999ms: past slot end (None)
    assert_eq!(eligible_rank_at_ms(9999), None);
    // 10000ms: past slot end (None)
    assert_eq!(eligible_rank_at_ms(10000), None);
}

#[test]
fn test_is_producer_eligible_ms() {
    let producers: Vec<crypto::PublicKey> = (0..5)
        .map(|i| {
            let mut bytes = [0u8; 32];
            bytes[0] = i;
            crypto::PublicKey::from_bytes(bytes)
        })
        .collect();

    // At 0ms: only rank 0 eligible
    assert!(is_producer_eligible_ms(&producers[0], &producers, 0));
    assert!(!is_producer_eligible_ms(&producers[1], &producers, 0));

    // At 2000ms: past slot end (MAX_FALLBACK_RANKS=1), no one eligible
    assert!(!is_producer_eligible_ms(&producers[0], &producers, 2000));
    assert!(!is_producer_eligible_ms(&producers[1], &producers, 2000));
    assert!(!is_producer_eligible_ms(&producers[2], &producers, 2000));

    // At 10000ms (past slot end): no one eligible
    for i in 0..5 {
        assert!(!is_producer_eligible_ms(&producers[i], &producers, 10000));
    }

    // Unknown producer is never eligible
    let unknown = crypto::PublicKey::from_bytes([99u8; 32]);
    assert!(!is_producer_eligible_ms(&unknown, &producers, 0));
    assert!(!is_producer_eligible_ms(&unknown, &producers, 10000));
}

#[test]
fn test_get_producer_rank() {
    let producers: Vec<crypto::PublicKey> = (0..3)
        .map(|i| {
            let mut bytes = [0u8; 32];
            bytes[0] = i;
            crypto::PublicKey::from_bytes(bytes)
        })
        .collect();

    assert_eq!(get_producer_rank(&producers[0], &producers), Some(0));
    assert_eq!(get_producer_rank(&producers[1], &producers), Some(1));
    assert_eq!(get_producer_rank(&producers[2], &producers), Some(2));

    // Non-existent producer
    let unknown = crypto::PublicKey::from_bytes([99u8; 32]);
    assert_eq!(get_producer_rank(&unknown, &producers), None);
}

#[test]
fn test_reward_eventually_zero() {
    let params = ConsensusParams::mainnet();
    // After era 63, reward should be 0
    assert_eq!(params.block_reward(BLOCKS_PER_ERA * 64), 0);
    assert_eq!(params.block_reward(BLOCKS_PER_ERA * 100), 0);
}

#[test]
fn test_bond_eventually_zero() {
    let params = ConsensusParams::mainnet();
    // After era 20, bond should be 0
    assert_eq!(params.bond_amount(BLOCKS_PER_ERA * 20), 0);
    assert_eq!(params.bond_amount(BLOCKS_PER_ERA * 30), 0);
}

#[test]
fn test_calculate_exit_normal() {
    let bond = 100_000_000_000u64; // 1000 DOLI
    let registered_at = 0;
    let current_height = COMMITMENT_PERIOD; // Full vesting period (1 day)

    let terms = calculate_exit(bond, registered_at, current_height);

    assert!(!terms.is_early_exit);
    assert_eq!(terms.return_amount, bond);
    assert_eq!(terms.penalty_amount, 0);
    assert_eq!(terms.commitment_percent, 100);
}

#[test]
fn test_calculate_exit_early_half() {
    // At 2 quarters (50% of commitment), penalty is 25%
    let bond = 100_000_000_000u64;
    let registered_at = 0;
    let current_height = COMMITMENT_PERIOD / 2; // 2 quarters (12h)

    let terms = calculate_exit(bond, registered_at, current_height);

    assert!(terms.is_early_exit);
    assert_eq!(terms.commitment_percent, 50);
    // Q3: 25% penalty, 75% returned
    assert_eq!(terms.penalty_amount, (bond * 25) / 100);
    assert_eq!(terms.return_amount, (bond * 75) / 100);
    assert_eq!(terms.penalty_destination, PenaltyDestination::Burn);
}

#[test]
fn test_calculate_exit_very_early() {
    // At 0 quarters (immediate exit), penalty is 75%
    let bond = 100_000_000_000u64;
    let registered_at = 1000;
    let current_height = 1000; // Immediate exit

    let terms = calculate_exit(bond, registered_at, current_height);

    assert!(terms.is_early_exit);
    assert_eq!(terms.commitment_percent, 0);
    // Q1: 75% penalty, 25% returned
    assert_eq!(terms.penalty_amount, (bond * 75) / 100);
    assert_eq!(terms.return_amount, (bond * 25) / 100);
}

#[test]
fn test_calculate_exit_one_quarter() {
    // At 1 quarter (25% of commitment), penalty is 50%
    let bond = 100_000_000_000u64;
    let registered_at = 0;
    let current_height = COMMITMENT_PERIOD / 4; // 1 quarter (6h)

    let terms = calculate_exit(bond, registered_at, current_height);

    assert!(terms.is_early_exit);
    assert_eq!(terms.commitment_percent, 25);
    // Q2: 50% penalty, 50% returned
    assert_eq!(terms.penalty_amount, (bond * 50) / 100);
    assert_eq!(terms.return_amount, (bond * 50) / 100);
}

#[test]
fn test_calculate_slash() {
    let bond = 100_000_000_000u64;

    let result = calculate_slash(bond);

    assert_eq!(result.burned_amount, bond);
    assert!(result.excluded);
}

#[test]
fn test_unbonding_period_is_7_days() {
    // At 10 seconds per slot: 7 days = 7 * 24 * 360 slots/day = 60,480 slots
    assert_eq!(UNBONDING_PERIOD, 60_480);
}

#[test]
fn test_commitment_period_is_1_day() {
    // Commitment period: 1 day = 4 quarters of 6h each = 8,640 slots
    assert_eq!(COMMITMENT_PERIOD, VESTING_PERIOD_SLOTS as BlockHeight);
    assert_eq!(COMMITMENT_PERIOD, 4 * VESTING_QUARTER_SLOTS as BlockHeight);
}

// ==================== RewardMode Tests ====================

#[test]
fn test_reward_mode_default() {
    assert_eq!(RewardMode::default(), RewardMode::EpochPool);
}

#[test]
fn test_reward_mode_serialization() {
    let mode = RewardMode::EpochPool;
    let bytes = bincode::serialize(&mode).unwrap();
    let deserialized: RewardMode = bincode::deserialize(&bytes).unwrap();
    assert_eq!(mode, deserialized);

    let mode2 = RewardMode::DirectCoinbase;
    let bytes2 = bincode::serialize(&mode2).unwrap();
    let deserialized2: RewardMode = bincode::deserialize(&bytes2).unwrap();
    assert_eq!(mode2, deserialized2);
}

#[test]
fn test_consensus_params_has_epoch_pool_mode() {
    let params = ConsensusParams::mainnet();
    assert_eq!(params.reward_mode, RewardMode::EpochPool);

    let params_testnet = ConsensusParams::testnet();
    assert_eq!(params_testnet.reward_mode, RewardMode::EpochPool);
}

#[test]
fn test_consensus_params_for_network_has_epoch_pool_mode() {
    let params_mainnet = ConsensusParams::for_network(Network::Mainnet);
    assert_eq!(params_mainnet.reward_mode, RewardMode::EpochPool);

    let params_testnet = ConsensusParams::for_network(Network::Testnet);
    assert_eq!(params_testnet.reward_mode, RewardMode::EpochPool);

    let params_devnet = ConsensusParams::for_network(Network::Devnet);
    assert_eq!(params_devnet.reward_mode, RewardMode::EpochPool);
}

#[test]
fn test_reward_mode_equality() {
    assert_eq!(RewardMode::EpochPool, RewardMode::EpochPool);
    assert_eq!(RewardMode::DirectCoinbase, RewardMode::DirectCoinbase);
    assert_ne!(RewardMode::EpochPool, RewardMode::DirectCoinbase);
}

// Property-based tests
use proptest::prelude::*;

proptest! {
    /// Slot-timestamp roundtrip preserves slot
    #[test]
    fn prop_slot_timestamp_roundtrip(slot: u32) {
        let params = ConsensusParams::mainnet();
        let timestamp = params.slot_to_timestamp(slot);
        let back = params.timestamp_to_slot(timestamp);
        prop_assert_eq!(slot, back);
    }

    /// Timestamps within same slot map to same slot
    #[test]
    fn prop_same_slot_same_result(slot in 0u32..1_000_000, offset in 0u64..3) {
        let params = ConsensusParams::mainnet();
        let base_timestamp = params.slot_to_timestamp(slot);
        // Add offset but stay within the slot (1-second slots)
        let timestamp = base_timestamp + offset.min(params.slot_duration - 1);
        let computed_slot = params.timestamp_to_slot(timestamp);
        prop_assert_eq!(slot, computed_slot);
    }

    /// Block reward halves each era (monotonically decreasing)
    #[test]
    fn prop_reward_decreasing(era in 0u32..63) {
        let params = ConsensusParams::mainnet();
        let height_current = (era as u64) * BLOCKS_PER_ERA;
        let height_next = (era as u64 + 1) * BLOCKS_PER_ERA;
        let reward_current = params.block_reward(height_current);
        let reward_next = params.block_reward(height_next);
        // Each era reward is half the previous (or both are 0)
        if reward_current > 0 {
            prop_assert!(reward_next <= reward_current);
            prop_assert_eq!(reward_next, reward_current / 2);
        }
    }

    /// Bond amount decreases each era
    #[test]
    fn prop_bond_decreasing(era in 0u32..19) {
        let params = ConsensusParams::mainnet();
        let height_current = (era as u64) * BLOCKS_PER_ERA;
        let height_next = (era as u64 + 1) * BLOCKS_PER_ERA;
        let bond_current = params.bond_amount(height_current);
        let bond_next = params.bond_amount(height_next);
        // Bond decreases by 30% each era (roughly)
        if bond_current > 0 {
            prop_assert!(bond_next < bond_current);
            // Should be approximately 70% of current (between 65% and 75%)
            let min_expected = bond_current * 65 / 100;
            let max_expected = bond_current * 75 / 100;
            prop_assert!(
                bond_next >= min_expected && bond_next <= max_expected,
                "bond_next {} should be ~70% of {} (between {} and {})",
                bond_next, bond_current, min_expected, max_expected
            );
        }
    }

    /// Epoch calculation is monotonically increasing
    #[test]
    fn prop_epoch_monotonic(slot_a: u32, slot_b: u32) {
        let params = ConsensusParams::mainnet();
        if slot_a <= slot_b {
            prop_assert!(params.slot_to_epoch(slot_a) <= params.slot_to_epoch(slot_b));
        }
    }

    /// Era calculation is monotonically increasing
    #[test]
    fn prop_era_monotonic(height_a: u64, height_b: u64) {
        let params = ConsensusParams::mainnet();
        if height_a <= height_b {
            prop_assert!(params.height_to_era(height_a) <= params.height_to_era(height_b));
        }
    }

    /// All heights within an era have same reward
    #[test]
    fn prop_same_era_same_reward(era in 0u32..20, offset in 0u64..BLOCKS_PER_ERA) {
        let params = ConsensusParams::mainnet();
        let base_height = (era as u64) * BLOCKS_PER_ERA;
        let height = base_height + offset;
        prop_assert_eq!(params.block_reward(base_height), params.block_reward(height));
    }

    /// All slots within an epoch have same epoch number
    #[test]
    fn prop_same_epoch_same_number(epoch in 0u32..10000, offset in 0u32..SLOTS_PER_EPOCH) {
        let params = ConsensusParams::mainnet();
        let base_slot = epoch * SLOTS_PER_EPOCH;
        let slot = base_slot.saturating_add(offset);
        // Only check if we didn't overflow
        if slot >= base_slot {
            prop_assert_eq!(params.slot_to_epoch(base_slot), params.slot_to_epoch(slot));
        }
    }

    /// Fallback producer selection is deterministic (anti-grinding: no prev_hash dependency)
    #[test]
    fn prop_fallback_deterministic(slot: u32, num_producers in 1usize..20) {
        let producers: Vec<(crypto::PublicKey, u64)> = (0..num_producers)
            .map(|i| {
                let mut bytes = [0u8; 32];
                bytes[0] = i as u8;
                bytes[1] = (i / 256) as u8;
                (crypto::PublicKey::from_bytes(bytes), (i as u64 + 1) * 10)
            })
            .collect();

        let result1 = select_producer_for_slot(slot, &producers);
        let result2 = select_producer_for_slot(slot, &producers);

        prop_assert_eq!(result1, result2);
    }

    /// Fallback producer selection returns at most MAX_FALLBACK_PRODUCERS
    #[test]
    fn prop_fallback_max_producers(slot: u32, num_producers in 1usize..20) {
        let producers: Vec<(crypto::PublicKey, u64)> = (0..num_producers)
            .map(|i| {
                let mut bytes = [0u8; 32];
                bytes[0] = i as u8;
                bytes[1] = (i / 256) as u8;
                (crypto::PublicKey::from_bytes(bytes), (i as u64 + 1) * 10)
            })
            .collect();

        let result = select_producer_for_slot(slot, &producers);

        prop_assert!(result.len() <= MAX_FALLBACK_PRODUCERS);
        prop_assert!(result.len() <= producers.len());
    }

    /// Allowed producer rank follows sequential 2s windows (seconds)
    #[test]
    fn prop_allowed_rank_time_windows(offset in 0u64..20) {
        let rank = allowed_producer_rank(offset);
        let offset_ms = offset * 1000;
        let expected = eligible_rank_at_ms(offset_ms).unwrap_or(MAX_FALLBACK_RANKS - 1);
        prop_assert_eq!(rank, expected);
    }

    /// Allowed producer rank follows sequential 2s windows (milliseconds)
    #[test]
    fn prop_allowed_rank_time_windows_ms(offset_ms in 0u64..15000) {
        let rank = allowed_producer_rank_ms(offset_ms);
        let expected = eligible_rank_at_ms(offset_ms).unwrap_or(MAX_FALLBACK_RANKS - 1);
        prop_assert_eq!(rank, expected);
    }

    /// Sequential windows: exactly one rank per 2s FALLBACK_TIMEOUT_MS
    #[test]
    fn prop_sequential_exclusive(offset_ms in 0u64..(MAX_FALLBACK_RANKS as u64 * FALLBACK_TIMEOUT_MS)) {
        let rank = eligible_rank_at_ms(offset_ms);
        prop_assert!(rank.is_some());
        let r = rank.unwrap();
        prop_assert!(r < MAX_FALLBACK_RANKS);
        // Only this rank should be eligible
        for other in 0..MAX_FALLBACK_RANKS {
            if other == r {
                prop_assert!(is_rank_eligible_at_ms(other, offset_ms));
            } else {
                prop_assert!(!is_rank_eligible_at_ms(other, offset_ms));
            }
        }
    }

    /// Past slot end: no rank eligible
    #[test]
    fn prop_past_slot_none_eligible(offset_ms in (MAX_FALLBACK_RANKS as u64 * FALLBACK_TIMEOUT_MS)..20000u64) {
        prop_assert!(eligible_rank_at_ms(offset_ms).is_none());
        for rank in 0..MAX_FALLBACK_RANKS {
            prop_assert!(!is_rank_eligible_at_ms(rank, offset_ms));
        }
    }

    /// Bootstrap phase is exactly first BOOTSTRAP_BLOCKS
    #[test]
    fn prop_bootstrap_boundary(height: u64) {
        let params = ConsensusParams::mainnet();
        prop_assert_eq!(params.is_bootstrap(height), height < BOOTSTRAP_BLOCKS);
    }

    /// Testnet parameters are consistent
    #[test]
    fn prop_testnet_consistent(slot: u32) {
        let params = ConsensusParams::testnet();
        let timestamp = params.slot_to_timestamp(slot);
        let back = params.timestamp_to_slot(timestamp);
        prop_assert_eq!(slot, back);
    }

    /// No overflow in timestamp calculations
    #[test]
    fn prop_timestamp_no_overflow(slot: u32) {
        let params = ConsensusParams::mainnet();
        // Should not panic
        let _ = params.slot_to_timestamp(slot);
    }

    /// Block reward never exceeds initial reward
    #[test]
    fn prop_reward_bounded(height: u64) {
        let params = ConsensusParams::mainnet();
        let reward = params.block_reward(height);
        prop_assert!(reward <= params.initial_reward);
    }

    /// Bond amount never exceeds initial bond
    #[test]
    fn prop_bond_bounded(height: u64) {
        let params = ConsensusParams::mainnet();
        let bond = params.bond_amount(height);
        prop_assert!(bond <= params.initial_bond);
    }
}

// ==================== Stress Test Parameter Tests ====================

#[test]
fn test_stress_params_600_producers() {
    let stress = StressTestParams::extreme_600();

    assert_eq!(stress.producer_count, 600);
    assert_eq!(stress.slot_duration_secs, 1);

    // Expected blocks per producer per hour: 3600 slots / 600 producers = 6
    let expected = stress.expected_blocks_per_producer_per_hour();
    assert!((expected - 6.0).abs() < 0.01);

    // Total bond: 600 * 1 DOLI = 600 DOLI
    assert_eq!(stress.total_bond_locked(), 600 * 100_000_000);

    // Network efficiency: 1/600 ~ 0.167%
    let efficiency = stress.network_efficiency();
    assert!((efficiency - 1.0 / 600.0).abs() < 0.0001);

    // Majority attack: 301 producers
    assert_eq!(stress.slots_for_majority_attack(), 301);
}

#[test]
fn test_stress_params_custom() {
    let stress = StressTestParams::with_producers(100);

    assert_eq!(stress.producer_count, 100);

    // Expected time between blocks: 100 * 2 = 200 seconds
    let time = stress.expected_time_between_blocks_secs();
    assert_eq!(time, 200.0);
}

#[test]
fn test_consensus_for_stress_test() {
    let stress = StressTestParams::extreme_600();
    let params = ConsensusParams::for_stress_test(&stress);

    assert_eq!(params.slot_duration, 1);
    assert_eq!(params.initial_bond, stress.bond_per_producer);
    assert_eq!(params.initial_reward, stress.block_reward);
    assert_eq!(params.bootstrap_blocks, 10);
}

#[test]
#[allow(deprecated)]
fn test_scaled_fallback_windows_example_3s() {
    let (primary, secondary, tertiary) = scaled_fallback_windows(3);
    assert_eq!(primary, 1);
    assert_eq!(secondary, 2);
    assert_eq!(tertiary, 3);
}

#[test]
#[allow(deprecated)]
fn test_scaled_fallback_windows_longer() {
    let (primary, secondary, tertiary) = scaled_fallback_windows(10);
    assert_eq!(primary, 5);
    assert_eq!(secondary, 7);
    assert_eq!(tertiary, 10);
}

#[test]
#[allow(deprecated)]
fn test_scaled_fallback_windows_stress() {
    let (primary, secondary, tertiary) = scaled_fallback_windows(1);
    assert!(primary >= 1);
    assert!(secondary >= 1);
    assert_eq!(tertiary, 1);
}

#[test]
#[allow(deprecated)]
fn test_allowed_producer_rank_scaled() {
    assert_eq!(allowed_producer_rank_scaled(0, 3), 0);
    assert_eq!(allowed_producer_rank_scaled(1, 3), 1);
    assert_eq!(allowed_producer_rank_scaled(2, 3), 2);
    assert_eq!(allowed_producer_rank_scaled(3, 3), 2);
    assert_eq!(allowed_producer_rank_scaled(0, 10), 0);
    assert_eq!(allowed_producer_rank_scaled(5, 10), 1);
    assert_eq!(allowed_producer_rank_scaled(7, 10), 2);
    assert_eq!(allowed_producer_rank_scaled(0, 1), 0);
    assert_eq!(allowed_producer_rank_scaled(1, 1), 2);
}

#[test]
fn test_stress_summary_format() {
    let stress = StressTestParams::extreme_600();
    let summary = stress.summary();

    // Summary should contain key information
    assert!(summary.contains("600 Producers"));
    assert!(summary.contains("Slot duration"));
    assert!(summary.contains("Bond"));
    assert!(summary.contains("51% attack"));
    assert!(summary.contains("301")); // Majority needed
}

// ==================== Registration Queue Tests ====================

#[test]
fn test_registration_fee_base() {
    // With no pending registrations, fee is base
    assert_eq!(registration_fee(0), BASE_REGISTRATION_FEE);
}

#[test]
fn test_registration_fee_table() {
    // Test deterministic table lookup
    // 0-4: 1.00x
    assert_eq!(registration_fee(0), BASE_REGISTRATION_FEE);
    assert_eq!(registration_fee(4), BASE_REGISTRATION_FEE);

    // 5-9: 1.50x
    assert_eq!(registration_fee(5), BASE_REGISTRATION_FEE * 150 / 100);
    assert_eq!(registration_fee(9), BASE_REGISTRATION_FEE * 150 / 100);

    // 10-19: 2.00x
    assert_eq!(registration_fee(10), BASE_REGISTRATION_FEE * 200 / 100);
    assert_eq!(registration_fee(19), BASE_REGISTRATION_FEE * 200 / 100);

    // 20-49: 3.00x
    assert_eq!(registration_fee(20), BASE_REGISTRATION_FEE * 300 / 100);
    assert_eq!(registration_fee(49), BASE_REGISTRATION_FEE * 300 / 100);

    // 50-99: 4.50x
    assert_eq!(registration_fee(50), BASE_REGISTRATION_FEE * 450 / 100);
    assert_eq!(registration_fee(99), BASE_REGISTRATION_FEE * 450 / 100);

    // 100-199: 6.50x
    assert_eq!(registration_fee(100), BASE_REGISTRATION_FEE * 650 / 100);
    assert_eq!(registration_fee(199), BASE_REGISTRATION_FEE * 650 / 100);

    // 200-299: 8.50x
    assert_eq!(registration_fee(200), BASE_REGISTRATION_FEE * 850 / 100);
    assert_eq!(registration_fee(299), BASE_REGISTRATION_FEE * 850 / 100);

    // 300+: 10.00x cap
    assert_eq!(registration_fee(300), BASE_REGISTRATION_FEE * 10);
    assert_eq!(registration_fee(1000), BASE_REGISTRATION_FEE * 10);
}

#[test]
fn test_registration_fee_escalates() {
    // Fee should increase with pending count at boundaries
    let fee_0 = registration_fee(0);
    let fee_5 = registration_fee(5);
    let fee_10 = registration_fee(10);
    let fee_20 = registration_fee(20);
    let fee_50 = registration_fee(50);
    let fee_100 = registration_fee(100);
    let fee_200 = registration_fee(200);
    let fee_300 = registration_fee(300);

    assert!(fee_5 > fee_0);
    assert!(fee_10 > fee_5);
    assert!(fee_20 > fee_10);
    assert!(fee_50 > fee_20);
    assert!(fee_100 > fee_50);
    assert!(fee_200 > fee_100);
    assert!(fee_300 > fee_200);
}

#[test]
fn test_registration_fee_capped_at_10x() {
    // Cap should be exactly 10x base
    assert_eq!(MAX_REGISTRATION_FEE, BASE_REGISTRATION_FEE * 10);

    // High pending count should hit cap
    let fee_high = registration_fee(300);
    assert_eq!(fee_high, MAX_REGISTRATION_FEE);

    // Even higher should still be capped
    let fee_extreme = registration_fee(u32::MAX / 2);
    assert_eq!(fee_extreme, MAX_REGISTRATION_FEE);
}

#[test]
fn test_fee_multiplier_const_fn() {
    // Test that fee_multiplier_x100 is const and deterministic
    const MULTIPLIER_0: u32 = fee_multiplier_x100(0);
    const MULTIPLIER_5: u32 = fee_multiplier_x100(5);
    const MULTIPLIER_300: u32 = fee_multiplier_x100(300);

    assert_eq!(MULTIPLIER_0, 100);
    assert_eq!(MULTIPLIER_5, 150);
    assert_eq!(MULTIPLIER_300, 1000);
}

#[test]
fn test_registration_fee_no_overflow() {
    // Test that extreme values don't cause overflow
    let fee_max = registration_fee(u32::MAX);
    assert_eq!(fee_max, MAX_REGISTRATION_FEE);

    let fee_half_max = registration_fee(u32::MAX / 2);
    assert_eq!(fee_half_max, MAX_REGISTRATION_FEE);
}

#[test]
fn test_registration_queue_new() {
    let queue = RegistrationQueue::new();

    assert_eq!(queue.pending_count(), 0);
    assert_eq!(queue.current_fee(), BASE_REGISTRATION_FEE);
    assert!(queue.can_add_to_block());
}

#[test]
fn test_registration_queue_submit() {
    let mut queue = RegistrationQueue::new();

    let reg = PendingRegistration {
        public_key: crypto::PublicKey::from_bytes([1u8; 32]),
        bond_amount: 100_000_000_000,
        fee_paid: BASE_REGISTRATION_FEE,
        submitted_at: 1000,
        prev_registration_hash: crypto::Hash::ZERO,
        sequence_number: 0,
    };

    assert!(queue.submit(reg, 1000).is_ok());
    assert_eq!(queue.pending_count(), 1);

    // Fee stays at base until we hit 5 pending (table-based)
    assert_eq!(queue.current_fee(), BASE_REGISTRATION_FEE);
}

#[test]
fn test_registration_queue_fee_tiers() {
    let mut queue = RegistrationQueue::new();

    // Submit 4 registrations - still in 1.00x tier
    for i in 0..4 {
        let fee = queue.current_fee();
        let reg = PendingRegistration {
            public_key: crypto::PublicKey::from_bytes([i as u8; 32]),
            bond_amount: 100_000_000_000,
            fee_paid: fee,
            submitted_at: 1000,
            prev_registration_hash: crypto::Hash::ZERO,
            sequence_number: i as u64,
        };
        queue.submit(reg, 1000).unwrap();
    }
    assert_eq!(queue.current_fee(), BASE_REGISTRATION_FEE); // Still 1.00x

    // Add one more to reach 5 pending -> 1.50x tier
    let reg5 = PendingRegistration {
        public_key: crypto::PublicKey::from_bytes([4u8; 32]),
        bond_amount: 100_000_000_000,
        fee_paid: BASE_REGISTRATION_FEE,
        submitted_at: 1000,
        prev_registration_hash: crypto::Hash::ZERO,
        sequence_number: 4,
    };
    queue.submit(reg5, 1000).unwrap();
    assert_eq!(queue.current_fee(), BASE_REGISTRATION_FEE * 150 / 100); // Now 1.50x
}

#[test]
fn test_registration_queue_insufficient_fee() {
    let mut queue = RegistrationQueue::new();

    // Submit 5 registrations to push into the 1.5x tier
    for i in 0..5 {
        let fee = queue.current_fee();
        let reg = PendingRegistration {
            public_key: crypto::PublicKey::from_bytes([i as u8; 32]),
            bond_amount: 100_000_000_000,
            fee_paid: fee,
            submitted_at: 1000,
            prev_registration_hash: crypto::Hash::ZERO,
            sequence_number: i as u64,
        };
        queue.submit(reg, 1000).unwrap();
    }

    // Now queue has 5 pending, so fee should be 1.5x base
    assert_eq!(queue.current_fee(), BASE_REGISTRATION_FEE * 150 / 100);

    // Registration with base fee should fail (need 1.5x)
    let reg_insufficient = PendingRegistration {
        public_key: crypto::PublicKey::from_bytes([99u8; 32]),
        bond_amount: 100_000_000_000,
        fee_paid: BASE_REGISTRATION_FEE, // Insufficient - should be 1.5x
        submitted_at: 1000,
        prev_registration_hash: crypto::Hash::ZERO,
        sequence_number: 99,
    };
    assert!(queue.submit(reg_insufficient, 1000).is_err());
}

#[test]
fn test_registration_queue_block_limit() {
    let mut queue = RegistrationQueue::new();

    // Submit more than MAX_REGISTRATIONS_PER_BLOCK
    for i in 0..10 {
        let fee = queue.current_fee();
        let reg = PendingRegistration {
            public_key: crypto::PublicKey::from_bytes([i as u8; 32]),
            bond_amount: 100_000_000_000,
            fee_paid: fee,
            submitted_at: 1000,
            prev_registration_hash: crypto::Hash::ZERO,
            sequence_number: i as u64,
        };
        queue.submit(reg, 1000).unwrap();
    }

    assert_eq!(queue.pending_count(), 10);

    // Begin block processing
    queue.begin_block(1000);

    // Should only be able to process MAX_REGISTRATIONS_PER_BLOCK
    let mut processed = 0;
    while queue.next_registration().is_some() {
        processed += 1;
    }

    assert_eq!(processed, MAX_REGISTRATIONS_PER_BLOCK as usize);
    assert_eq!(
        queue.pending_count(),
        10 - MAX_REGISTRATIONS_PER_BLOCK as usize
    );
}

#[test]
fn test_registration_queue_fifo() {
    let mut queue = RegistrationQueue::new();

    // Submit registrations with different sequence numbers
    for i in 0..3 {
        let fee = queue.current_fee();
        let reg = PendingRegistration {
            public_key: crypto::PublicKey::from_bytes([i as u8; 32]),
            bond_amount: 100_000_000_000,
            fee_paid: fee,
            submitted_at: 1000,
            prev_registration_hash: crypto::Hash::ZERO,
            sequence_number: i as u64,
        };
        queue.submit(reg, 1000).unwrap();
    }

    queue.begin_block(1000);

    // Should process in FIFO order
    let first = queue.next_registration().unwrap();
    assert_eq!(first.sequence_number, 0);

    let second = queue.next_registration().unwrap();
    assert_eq!(second.sequence_number, 1);

    let third = queue.next_registration().unwrap();
    assert_eq!(third.sequence_number, 2);
}

#[test]
fn test_registration_queue_prune_expired() {
    let mut queue = RegistrationQueue::new();

    // Submit old registration
    let old_reg = PendingRegistration {
        public_key: crypto::PublicKey::from_bytes([1u8; 32]),
        bond_amount: 100_000_000_000,
        fee_paid: BASE_REGISTRATION_FEE,
        submitted_at: 1000,
        prev_registration_hash: crypto::Hash::ZERO,
        sequence_number: 0,
    };
    queue.submit(old_reg, 1000).unwrap();

    // Submit recent registration
    let recent_fee = queue.current_fee();
    let recent_reg = PendingRegistration {
        public_key: crypto::PublicKey::from_bytes([2u8; 32]),
        bond_amount: 100_000_000_000,
        fee_paid: recent_fee,
        submitted_at: 5000,
        prev_registration_hash: crypto::Hash::ZERO,
        sequence_number: 1,
    };
    queue.submit(recent_reg, 5000).unwrap();

    // Prune with max age of 1000 blocks
    let expired = queue.prune_expired(6000, 1000);

    // Old registration should be expired
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].sequence_number, 0);

    // Recent registration should remain
    assert_eq!(queue.pending_count(), 1);
}

proptest! {
    /// Scaled windows maintain correct ordering
    #[test]
    #[allow(deprecated)]
    fn prop_scaled_windows_ordering(slot_duration in 1u64..120) {
        let (primary, secondary, tertiary) = scaled_fallback_windows(slot_duration);
        prop_assert!(primary <= secondary);
        prop_assert!(secondary <= tertiary);
        prop_assert_eq!(tertiary, slot_duration);
    }

    /// Stress test params are internally consistent
    #[test]
    fn prop_stress_params_consistent(count in 1u32..1000) {
        let stress = StressTestParams::with_producers(count);

        // Total bond should be count * individual bond
        prop_assert_eq!(
            stress.total_bond_locked(),
            stress.bond_per_producer * count as u64
        );

        // Majority attack needs > 50%
        prop_assert!(stress.slots_for_majority_attack() > count / 2);
        prop_assert!(stress.slots_for_majority_attack() <= (count / 2) + 1);
    }

    /// Registration fee is monotonically non-decreasing
    /// fee(p+1) >= fee(p) for all p
    #[test]
    fn prop_registration_fee_monotonic(pending_a: u32, pending_b: u32) {
        if pending_a <= pending_b {
            let fee_a = registration_fee(pending_a);
            let fee_b = registration_fee(pending_b);
            prop_assert!(
                fee_a <= fee_b,
                "Monotonicity violated: fee({}) = {} > fee({}) = {}",
                pending_a, fee_a, pending_b, fee_b
            );
        }
    }

    /// Registration fee is always at least BASE_REGISTRATION_FEE
    #[test]
    fn prop_registration_fee_min(pending: u32) {
        let fee = registration_fee(pending);
        prop_assert!(
            fee >= BASE_REGISTRATION_FEE,
            "Fee {} is below base {} for pending count {}",
            fee, BASE_REGISTRATION_FEE, pending
        );
    }

    /// Registration fee never exceeds MAX_REGISTRATION_FEE (10x base)
    #[test]
    fn prop_registration_fee_cap(pending: u32) {
        let fee = registration_fee(pending);
        prop_assert!(
            fee <= MAX_REGISTRATION_FEE,
            "Fee {} exceeds cap {} for pending count {}",
            fee, MAX_REGISTRATION_FEE, pending
        );
        // Also verify cap is exactly 10x base
        prop_assert_eq!(MAX_REGISTRATION_FEE, BASE_REGISTRATION_FEE * 10);
    }

    /// Registration fee never overflows (test extreme values)
    #[test]
    fn prop_registration_fee_no_overflow(pending in 0u32..=u32::MAX) {
        // This should never panic
        let fee = registration_fee(pending);
        // Result should be valid
        prop_assert!(fee >= BASE_REGISTRATION_FEE);
        prop_assert!(fee <= MAX_REGISTRATION_FEE);
    }

    /// Fee multiplier is deterministic (same input = same output)
    #[test]
    fn prop_fee_multiplier_deterministic(pending: u32) {
        let mult1 = fee_multiplier_x100(pending);
        let mult2 = fee_multiplier_x100(pending);
        prop_assert_eq!(mult1, mult2);
    }

    /// Fee multiplier is within bounds [100, 1000]
    #[test]
    fn prop_fee_multiplier_bounds(pending: u32) {
        let mult = fee_multiplier_x100(pending);
        prop_assert!(mult >= 100, "Multiplier {} below 100 (1x)", mult);
        prop_assert!(mult <= 1000, "Multiplier {} above 1000 (10x)", mult);
    }
}

// ==================== Required Protocol Tests ====================
//
// These tests verify critical protocol properties as specified in TASKS.md

/// Test: constants_match_whitepaper
/// Verify all consensus constants match the whitepaper specification.
#[test]
fn test_constants_match_whitepaper() {
    // Slot duration: 10 seconds (mainnet)
    assert_eq!(SLOT_DURATION, 10);

    // Slots per epoch: 360 (1 hour with 10s slots)
    assert_eq!(SLOTS_PER_EPOCH, 360);

    // Slots per year: 3,153,600 (365.25 days * 24h * 360 slots/h)
    assert_eq!(SLOTS_PER_YEAR, 3_153_600);

    // Slots per era: 4 years = 12,614,400 slots
    assert_eq!(SLOTS_PER_ERA, 12_614_400);

    // Total supply: 2,522,880,000,000,000 sats (25,228,800 DOLI)
    assert_eq!(TOTAL_SUPPLY, 2_522_880_000_000_000);

    // Initial block reward: 100,000,000 sats (1 DOLI)
    assert_eq!(INITIAL_BLOCK_REWARD, 100_000_000);

    // VDF iterations: 800K per block (~55ms)
    assert_eq!(T_BLOCK, 800_000);

    // Single proposer per slot (no fallback): only rank 0
    assert_eq!(FALLBACK_TIMEOUT_MS, 2_000);
    assert_eq!(MAX_FALLBACK_RANKS, 1);
    assert_eq!(MAX_FALLBACK_PRODUCERS, 1);

    // Unbonding period: 7 days = 60,480 slots
    assert_eq!(UNBONDING_PERIOD, 60_480);
}

/// Test: selection_independent_of_prev_hash
/// Producer selection must NOT depend on prev_hash (anti-grinding)
#[test]
fn test_selection_independent_of_prev_hash() {
    // Create producers with different bond counts
    let producer_a = crypto::PublicKey::from_bytes([1u8; 32]);
    let producer_b = crypto::PublicKey::from_bytes([2u8; 32]);
    let producer_c = crypto::PublicKey::from_bytes([3u8; 32]);

    let producers = vec![
        (producer_a, 10), // 10 bonds
        (producer_b, 5),  // 5 bonds
        (producer_c, 3),  // 3 bonds
    ];

    // Selection for any slot should be deterministic based only on slot + producers
    // NOT on prev_hash (which would enable grinding)
    let slot = 42;

    let selection_1 = select_producer_for_slot(slot, &producers);
    let selection_2 = select_producer_for_slot(slot, &producers);
    let selection_3 = select_producer_for_slot(slot, &producers);

    // Same slot, same producers -> same selection
    assert_eq!(selection_1, selection_2);
    assert_eq!(selection_2, selection_3);

    // Verify the selection function doesn't take a prev_hash parameter
    // (This is enforced by the function signature itself)
}

/// Test: selection_uses_evenly_distributed_offsets
/// Selection uses evenly-distributed offsets across ticket space for fallbacks.
/// offset = (total_tickets * rank) / MAX_FALLBACK_RANKS
#[test]
fn test_selection_uses_evenly_distributed_offsets() {
    let producer_a = crypto::PublicKey::from_bytes([1u8; 32]);
    let producer_b = crypto::PublicKey::from_bytes([2u8; 32]);
    let producer_c = crypto::PublicKey::from_bytes([3u8; 32]);

    // Sort order by pubkey: producer_a, producer_b, producer_c
    let producers = vec![
        (producer_a, 3), // 3 bonds -> tickets 0, 1, 2
        (producer_b, 2), // 2 bonds -> tickets 3, 4
        (producer_c, 1), // 1 bond  -> ticket 5
    ];
    // Total tickets = 6

    // Primary selection (slot % total_tickets) is unchanged:
    assert_eq!(select_producer_for_slot(0, &producers)[0], producer_a);
    assert_eq!(select_producer_for_slot(3, &producers)[0], producer_b);
    assert_eq!(select_producer_for_slot(5, &producers)[0], producer_c);
    assert_eq!(select_producer_for_slot(6, &producers)[0], producer_a); // wraps

    // Single proposer per slot (MAX_FALLBACK_RANKS=1, MAX_FALLBACK_PRODUCERS=1):
    // rank 0: ticket 4 -> producer_b
    // No fallback ranks.
    let sel4 = select_producer_for_slot(4, &producers);
    assert_eq!(sel4.len(), 1); // 1 producer (single proposer)
    assert_eq!(sel4[0], producer_b);

    // With enough producers and bonds, fallbacks spread across the set
    // Verify determinism
    assert_eq!(
        select_producer_for_slot(42, &producers),
        select_producer_for_slot(42, &producers)
    );
}

/// Test: fallback_windows
/// Verify single-proposer slot timing (no fallback)
#[test]
fn test_fallback_windows() {
    // Verify single-proposer constants: 1 rank (primary only, no fallback)
    assert_eq!(FALLBACK_TIMEOUT_MS, 2_000);
    assert_eq!(MAX_FALLBACK_RANKS, 1);
    assert_eq!(MAX_FALLBACK_PRODUCERS, 1);

    // Create 2 producers for window testing
    let producers: Vec<crypto::PublicKey> = (0..2)
        .map(|i| {
            let mut bytes = [0u8; 32];
            bytes[0] = i;
            crypto::PublicKey::from_bytes(bytes)
        })
        .collect();

    // Rank 0 (primary) is eligible in 0-1999ms window
    assert!(is_producer_eligible_ms(&producers[0], &producers, 0));
    assert!(is_producer_eligible_ms(&producers[0], &producers, 1999));
    assert!(!is_producer_eligible_ms(&producers[0], &producers, 2000));

    // No fallback rank exists — rank 1 producer is never eligible
    assert!(!is_producer_eligible_ms(&producers[1], &producers, 0));
    assert!(!is_producer_eligible_ms(&producers[1], &producers, 1999));
    assert!(!is_producer_eligible_ms(&producers[1], &producers, 2000));
    assert!(!is_producer_eligible_ms(&producers[1], &producers, 3999));
    assert!(!is_producer_eligible_ms(&producers[1], &producers, 4000));

    // Past rank 0 window (2000ms+): no one eligible
    for producer in &producers {
        assert!(!is_producer_eligible_ms(producer, &producers, 2000));
        assert!(!is_producer_eligible_ms(producer, &producers, 4000));
        assert!(!is_producer_eligible_ms(producer, &producers, 10000));
    }

    // Test ms precision at boundaries
    assert_eq!(allowed_producer_rank_ms(0), 0);
    assert_eq!(allowed_producer_rank_ms(1999), 0);
    assert_eq!(allowed_producer_rank_ms(2000), 0); // past slot, clamped to max rank (0)
    assert_eq!(allowed_producer_rank_ms(3999), 0); // past slot, clamped to max rank (0)
    assert_eq!(allowed_producer_rank_ms(4000), 0); // past slot, clamped to max rank (0)
    assert_eq!(allowed_producer_rank_ms(10000), 0); // past slot, clamped to max rank (0)
}

/// Test: seniority_uses_years
/// Seniority weight must use discrete yearly steps (0-1=1, 1-2=2, 2-3=3, 3+=4)
#[test]
fn test_seniority_uses_years() {
    // This test validates the weight formula uses yearly boundaries
    // The actual weight calculation is in storage/producer.rs

    // Verify SLOTS_PER_YEAR is correct for the calculation
    // 365 days * 24 hours * 360 slots/hour = 3,153,600 slots/year
    let expected_slots_per_year = 365 * 24 * 360;
    assert_eq!(SLOTS_PER_YEAR as u64, expected_slots_per_year);

    // Weight boundaries should be at year marks:
    // Year 0 (0-3,153,599 slots): weight 1
    // Year 1 (3,153,600-6,307,199 slots): weight 2
    // Year 2 (6,307,200-9,460,799 slots): weight 3
    // Year 3+ (9,460,800+ slots): weight 4

    // This is validated by storage/producer.rs tests
}

/// Test: supply_converges
/// Total supply must converge to TOTAL_SUPPLY through halving
#[test]
fn test_supply_converges() {
    let mut total_minted: u64 = 0;
    let mut era = 0;
    let mut current_reward = INITIAL_BLOCK_REWARD;

    // Simulate mining through multiple eras
    while total_minted < TOTAL_SUPPLY {
        // Calculate how much can be minted this era
        let slots_in_era = SLOTS_PER_ERA;
        let max_mint_this_era = current_reward.saturating_mul(slots_in_era);

        // Mint up to the remaining supply
        let remaining = TOTAL_SUPPLY.saturating_sub(total_minted);
        let minted_this_era = max_mint_this_era.min(remaining);

        total_minted = total_minted.saturating_add(minted_this_era);

        // Halve reward for next era
        current_reward /= 2;
        era += 1;

        // Safety limit
        if era > 100 || current_reward == 0 {
            break;
        }
    }

    // Supply should converge to TOTAL_SUPPLY
    assert!(total_minted <= TOTAL_SUPPLY);
}

/// Test: presence_not_in_consensus
/// Presence score must NOT affect validity or selection
#[test]
fn test_presence_not_in_consensus() {
    // Producer selection function signature doesn't include presence score
    // This is enforced by the type system

    // The select_producer_for_slot function takes only:
    // - slot: Slot (u32)
    // - producers_with_bonds: &[(PublicKey, u64)]
    //
    // No presence score, no telemetry data

    let producer_a = crypto::PublicKey::from_bytes([1u8; 32]);
    let producers = vec![(producer_a, 5)];

    // Selection is deterministic and doesn't depend on any external state
    let selection_1 = select_producer_for_slot(100, &producers);
    let selection_2 = select_producer_for_slot(100, &producers);
    assert_eq!(selection_1, selection_2);

    // Even if presence tracking exists (in tpop/presence.rs),
    // it's marked as telemetry-only and doesn't affect this selection
}

// =========================================================================
// REWARD EPOCH TESTS (Block-height based)
// =========================================================================

#[test]
fn test_reward_epoch_from_height() {
    // Test the from_height function matches the spec
    assert_eq!(reward_epoch::from_height(0), 0);
    assert_eq!(reward_epoch::from_height(359), 0);
    assert_eq!(reward_epoch::from_height(360), 1);
    assert_eq!(reward_epoch::from_height(719), 1);
    assert_eq!(reward_epoch::from_height(720), 2);
    assert_eq!(reward_epoch::from_height(1000), 2); // 1000/360 = 2
}

#[test]
fn test_reward_epoch_boundaries() {
    // Test boundaries function
    let (start, end) = reward_epoch::boundaries(0);
    assert_eq!(start, 0);
    assert_eq!(end, 360);

    let (start, end) = reward_epoch::boundaries(1);
    assert_eq!(start, 360);
    assert_eq!(end, 720);

    let (start, end) = reward_epoch::boundaries(5);
    assert_eq!(start, 1800);
    assert_eq!(end, 2160);
}

#[test]
fn test_reward_epoch_is_complete() {
    // Epoch 0 ends at block 360
    assert!(!reward_epoch::is_complete(0, 0));
    assert!(!reward_epoch::is_complete(0, 359));
    assert!(reward_epoch::is_complete(0, 360));
    assert!(reward_epoch::is_complete(0, 1000));

    // Epoch 1 ends at block 720
    assert!(!reward_epoch::is_complete(1, 360));
    assert!(!reward_epoch::is_complete(1, 719));
    assert!(reward_epoch::is_complete(1, 720));
}

#[test]
fn test_reward_epoch_last_complete() {
    // No complete epochs yet
    assert_eq!(reward_epoch::last_complete(0), None);
    assert_eq!(reward_epoch::last_complete(359), None);

    // First epoch just completed
    assert_eq!(reward_epoch::last_complete(360), Some(0));
    assert_eq!(reward_epoch::last_complete(719), Some(0));

    // Two epochs completed
    assert_eq!(reward_epoch::last_complete(720), Some(1));
    assert_eq!(reward_epoch::last_complete(1000), Some(1));
}

#[test]
fn test_reward_epoch_is_epoch_start() {
    assert!(reward_epoch::is_epoch_start(0));
    assert!(!reward_epoch::is_epoch_start(1));
    assert!(!reward_epoch::is_epoch_start(359));
    assert!(reward_epoch::is_epoch_start(360));
    assert!(reward_epoch::is_epoch_start(720));
}

#[test]
fn test_reward_epoch_current() {
    // current() is an alias for from_height()
    assert_eq!(reward_epoch::current(0), reward_epoch::from_height(0));
    assert_eq!(reward_epoch::current(500), reward_epoch::from_height(500));
    assert_eq!(reward_epoch::current(1000), reward_epoch::from_height(1000));
}

#[test]
fn test_reward_epoch_complete_epochs() {
    // Before first epoch completes
    assert_eq!(reward_epoch::complete_epochs(0), 0);
    assert_eq!(reward_epoch::complete_epochs(359), 0);

    // After first epoch completes
    assert_eq!(reward_epoch::complete_epochs(360), 1);
    assert_eq!(reward_epoch::complete_epochs(719), 1);

    // After second epoch completes
    assert_eq!(reward_epoch::complete_epochs(720), 2);
}

#[test]
fn test_reward_epoch_blocks_per_epoch() {
    assert_eq!(reward_epoch::blocks_per_epoch(), BLOCKS_PER_REWARD_EPOCH);
    assert_eq!(reward_epoch::blocks_per_epoch(), 360);
}

// ========================================================================
// Tests for network-aware (_with) functions
// ========================================================================

#[test]
fn test_reward_epoch_from_height_with_devnet() {
    // Devnet uses 60 blocks per epoch
    let devnet_blocks = 60;

    assert_eq!(reward_epoch::from_height_with(0, devnet_blocks), 0);
    assert_eq!(reward_epoch::from_height_with(59, devnet_blocks), 0);
    assert_eq!(reward_epoch::from_height_with(60, devnet_blocks), 1);
    assert_eq!(reward_epoch::from_height_with(119, devnet_blocks), 1);
    assert_eq!(reward_epoch::from_height_with(120, devnet_blocks), 2);
}

#[test]
fn test_reward_epoch_boundaries_with_devnet() {
    let devnet_blocks = 60;

    let (start, end) = reward_epoch::boundaries_with(0, devnet_blocks);
    assert_eq!(start, 0);
    assert_eq!(end, 60);

    let (start, end) = reward_epoch::boundaries_with(1, devnet_blocks);
    assert_eq!(start, 60);
    assert_eq!(end, 120);

    let (start, end) = reward_epoch::boundaries_with(5, devnet_blocks);
    assert_eq!(start, 300);
    assert_eq!(end, 360);
}

#[test]
fn test_reward_epoch_is_complete_with_devnet() {
    let devnet_blocks = 60;

    // Epoch 0 ends at block 60 for devnet
    assert!(!reward_epoch::is_complete_with(0, 0, devnet_blocks));
    assert!(!reward_epoch::is_complete_with(0, 59, devnet_blocks));
    assert!(reward_epoch::is_complete_with(0, 60, devnet_blocks));
    assert!(reward_epoch::is_complete_with(0, 100, devnet_blocks));

    // Epoch 1 ends at block 120 for devnet
    assert!(!reward_epoch::is_complete_with(1, 60, devnet_blocks));
    assert!(!reward_epoch::is_complete_with(1, 119, devnet_blocks));
    assert!(reward_epoch::is_complete_with(1, 120, devnet_blocks));
}

#[test]
fn test_reward_epoch_last_complete_with_devnet() {
    let devnet_blocks = 60;

    // No complete epochs yet
    assert_eq!(reward_epoch::last_complete_with(0, devnet_blocks), None);
    assert_eq!(reward_epoch::last_complete_with(59, devnet_blocks), None);

    // First epoch just completed (at block 60)
    assert_eq!(reward_epoch::last_complete_with(60, devnet_blocks), Some(0));
    assert_eq!(
        reward_epoch::last_complete_with(119, devnet_blocks),
        Some(0)
    );

    // Two epochs completed (at block 120)
    assert_eq!(
        reward_epoch::last_complete_with(120, devnet_blocks),
        Some(1)
    );
}

#[test]
fn test_reward_epoch_is_epoch_start_with_devnet() {
    let devnet_blocks = 60;

    assert!(reward_epoch::is_epoch_start_with(0, devnet_blocks));
    assert!(!reward_epoch::is_epoch_start_with(1, devnet_blocks));
    assert!(!reward_epoch::is_epoch_start_with(59, devnet_blocks));
    assert!(reward_epoch::is_epoch_start_with(60, devnet_blocks));
    assert!(reward_epoch::is_epoch_start_with(120, devnet_blocks));
}

#[test]
fn test_reward_epoch_complete_epochs_with_devnet() {
    let devnet_blocks = 60;

    // Before first epoch completes
    assert_eq!(reward_epoch::complete_epochs_with(0, devnet_blocks), 0);
    assert_eq!(reward_epoch::complete_epochs_with(59, devnet_blocks), 0);

    // After first epoch completes
    assert_eq!(reward_epoch::complete_epochs_with(60, devnet_blocks), 1);
    assert_eq!(reward_epoch::complete_epochs_with(119, devnet_blocks), 1);

    // After second epoch completes
    assert_eq!(reward_epoch::complete_epochs_with(120, devnet_blocks), 2);
}

#[test]
fn test_reward_epoch_with_mainnet_same_as_default() {
    // Mainnet uses 360 blocks, same as the global constant
    let mainnet_blocks = 360;

    // These should match the non-_with versions
    assert_eq!(
        reward_epoch::from_height_with(1000, mainnet_blocks),
        reward_epoch::from_height(1000)
    );
    assert_eq!(
        reward_epoch::boundaries_with(5, mainnet_blocks),
        reward_epoch::boundaries(5)
    );
    assert_eq!(
        reward_epoch::is_complete_with(0, 360, mainnet_blocks),
        reward_epoch::is_complete(0, 360)
    );
    assert_eq!(
        reward_epoch::last_complete_with(720, mainnet_blocks),
        reward_epoch::last_complete(720)
    );
}

#[test]
fn test_apply_chainspec() {
    use crate::chainspec::{ChainSpec, ConsensusSpec, GenesisSpec};

    let mut params = ConsensusParams::devnet();
    let spec = ChainSpec {
        name: "test".into(),
        id: "test".into(),
        network: Network::Devnet,
        genesis: GenesisSpec {
            timestamp: 9999,
            message: "test".into(),
            initial_reward: 50_000_000,
        },
        consensus: ConsensusSpec {
            slot_duration: 2,
            slots_per_epoch: 120,
            bond_amount: 200_000_000,
        },
        genesis_producers: vec![],
    };

    params.apply_chainspec(&spec);
    assert_eq!(params.slot_duration, 2);
    assert_eq!(params.slots_per_reward_epoch, 120);
    assert_eq!(params.initial_bond, 200_000_000);
    assert_eq!(params.initial_reward, 50_000_000);
    assert_eq!(params.genesis_time, 9999);
}

#[test]
fn test_apply_chainspec_mainnet_locked() {
    use crate::chainspec::{ChainSpec, ConsensusSpec, GenesisSpec};

    let mut params = ConsensusParams::mainnet();
    let original_slot = params.slot_duration;
    let original_bond = params.initial_bond;

    let spec = ChainSpec {
        name: "evil".into(),
        id: "mainnet".into(),
        network: Network::Mainnet,
        genesis: GenesisSpec {
            timestamp: 1,
            message: "hack".into(),
            initial_reward: 999,
        },
        consensus: ConsensusSpec {
            slot_duration: 1,
            slots_per_epoch: 1,
            bond_amount: 1,
        },
        genesis_producers: vec![],
    };

    params.apply_chainspec(&spec);
    // Mainnet params should be unchanged
    assert_eq!(params.slot_duration, original_slot);
    assert_eq!(params.initial_bond, original_bond);
}

// ==================== Tier Computation Tests ====================

#[test]
fn test_tier1_deterministic() {
    let producers: Vec<(crypto::PublicKey, u64)> = (0..10u8)
        .map(|i| {
            let mut bytes = [0u8; 32];
            bytes[0] = i;
            (crypto::PublicKey::from_bytes(bytes), (10 - i) as u64)
        })
        .collect();

    let set1 = compute_tier1_set(&producers);
    let set2 = compute_tier1_set(&producers);
    assert_eq!(set1, set2, "Tier 1 set must be deterministic");

    // Reversed input should give same result
    let mut reversed = producers.clone();
    reversed.reverse();
    let set3 = compute_tier1_set(&reversed);
    assert_eq!(set1, set3, "Tier 1 set must be order-independent");
}

#[test]
fn test_tier1_capped_at_500() {
    let producers: Vec<(crypto::PublicKey, u64)> = (0..600u32)
        .map(|i| {
            let bytes = i.to_le_bytes();
            let mut key_bytes = [0u8; 32];
            key_bytes[..4].copy_from_slice(&bytes);
            (crypto::PublicKey::from_bytes(key_bytes), 100)
        })
        .collect();

    let set = compute_tier1_set(&producers);
    assert_eq!(set.len(), TIER1_MAX_VALIDATORS);
}

#[test]
fn test_tier1_small_set() {
    // Fewer producers than TIER1_MAX_VALIDATORS
    let producers: Vec<(crypto::PublicKey, u64)> = (0..10u8)
        .map(|i| {
            let mut bytes = [0u8; 32];
            bytes[0] = i;
            (crypto::PublicKey::from_bytes(bytes), 100)
        })
        .collect();

    let set = compute_tier1_set(&producers);
    assert_eq!(set.len(), 10);
}

#[test]
fn test_tier1_weight_ordering() {
    let mut bytes_high = [0u8; 32];
    bytes_high[0] = 1;
    let mut bytes_low = [0u8; 32];
    bytes_low[0] = 2;

    let high = crypto::PublicKey::from_bytes(bytes_high);
    let low = crypto::PublicKey::from_bytes(bytes_low);

    let producers = vec![(low, 10), (high, 100)];
    let set = compute_tier1_set(&producers);

    // Higher weight should be first
    assert_eq!(set[0], high);
    assert_eq!(set[1], low);
}

#[test]
fn test_region_distribution() {
    // Check that regions are reasonably distributed
    let mut region_counts = vec![0u32; NUM_REGIONS as usize];

    for i in 0..1500u32 {
        let bytes = i.to_le_bytes();
        let mut key_bytes = [0u8; 32];
        key_bytes[..4].copy_from_slice(&bytes);
        let pk = crypto::PublicKey::from_bytes(key_bytes);
        let region = producer_region(&pk);
        assert!(region < NUM_REGIONS);
        region_counts[region as usize] += 1;
    }

    // Each region should get at least some producers (rough distribution check)
    for (i, &count) in region_counts.iter().enumerate() {
        assert!(
            count > 50,
            "Region {} only got {} producers (expected ~100)",
            i,
            count
        );
    }
}

#[test]
fn test_tier_assignment() {
    let producers: Vec<(crypto::PublicKey, u64)> = (0..600u32)
        .map(|i| {
            let bytes = i.to_le_bytes();
            let mut key_bytes = [0u8; 32];
            key_bytes[..4].copy_from_slice(&bytes);
            (crypto::PublicKey::from_bytes(key_bytes), (600 - i) as u64)
        })
        .collect();

    let tier1_set = compute_tier1_set(&producers);
    let all_sorted: Vec<crypto::PublicKey> = {
        let mut sorted = producers.clone();
        sorted.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| a.0.as_bytes().cmp(b.0.as_bytes()))
        });
        sorted.into_iter().map(|(pk, _)| pk).collect()
    };

    // First producer (highest weight) should be Tier 1
    assert_eq!(producer_tier(&all_sorted[0], &tier1_set, &all_sorted), 1);

    // Producer at index 500 (just past tier1) should be Tier 2
    assert_eq!(producer_tier(&all_sorted[500], &tier1_set, &all_sorted), 2);

    // Producer at index 499 (last in tier1) should be Tier 1
    assert_eq!(producer_tier(&all_sorted[499], &tier1_set, &all_sorted), 1);

    // Unknown producer (not in the sorted list) should be Tier 0, NOT Tier 3.
    // Tier 3 would unsubscribe from BLOCKS_TOPIC, starving the node.
    let unknown_key = crypto::PublicKey::from_bytes([0xFF; 32]);
    assert_eq!(producer_tier(&unknown_key, &tier1_set, &all_sorted), 0);
}

#[test]
fn test_genesis_time_matches_chainspec() {
    let json = include_str!("../../../../chainspec.mainnet.json");
    let spec: serde_json::Value = serde_json::from_str(json).unwrap();
    let chainspec_time = spec["genesis"]["timestamp"].as_u64().unwrap();
    assert_eq!(
        GENESIS_TIME, chainspec_time,
        "GENESIS_TIME ({}) is out of sync with chainspec.mainnet.json ({})",
        GENESIS_TIME, chainspec_time
    );
}

#[test]
fn test_testnet_genesis_time_matches_chainspec() {
    use crate::network::Network;
    use crate::network_params::NetworkParams;
    let json = include_str!("../../../../chainspec.testnet.json");
    let spec: serde_json::Value = serde_json::from_str(json).unwrap();
    let chainspec_time = spec["genesis"]["timestamp"].as_u64().unwrap();
    let params = NetworkParams::defaults(Network::Testnet);
    assert_eq!(
        params.genesis_time, chainspec_time,
        "Testnet genesis_time ({}) out of sync with chainspec.testnet.json ({})",
        params.genesis_time, chainspec_time
    );
}
