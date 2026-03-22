#[allow(deprecated)]
use super::*;
use crypto::{Hash, KeyPair};
use doli_core::network::Network;

fn make_producer(height: u64) -> ProducerInfo {
    let keypair = KeyPair::generate();
    ProducerInfo::new(
        *keypair.public_key(),
        height,
        100_000_000_000, // 1000 coins
        (Hash::ZERO, 0),
        0,
        BOND_UNIT, // Use mainnet/testnet bond unit for tests
    )
}

#[test]
fn test_producer_lifecycle() {
    let mut info = make_producer(1000);

    // Initially active
    assert!(info.is_active());
    assert!(info.can_produce());

    // Start unbonding (test uses 43,200 blocks as arbitrary duration)
    info.start_unbonding(2000);
    assert!(info.is_active()); // Still active during unbonding
    assert!(info.can_produce());
    assert!(!info.is_unbonding_complete(2000, 43_200));
    assert!(!info.is_unbonding_complete(45_000, 43_200)); // Just before completion
    assert!(info.is_unbonding_complete(45_200, 43_200)); // After unbonding period

    // Complete exit
    info.complete_exit();
    assert!(!info.is_active());
    assert!(!info.can_produce());
}

#[test]
fn test_bond_unlock() {
    let info = make_producer(1000);
    let lock_duration = 2_102_400; // 4 years

    assert!(!info.is_bond_unlocked(1000, lock_duration));
    assert!(!info.is_bond_unlocked(2_102_399, lock_duration));
    assert!(info.is_bond_unlocked(2_103_400, lock_duration));
}

#[test]
fn test_slashing() {
    let mut info = make_producer(1000);

    // Slashing is always 100% (full bond burned)
    assert!(!info.is_slashed());
    info.slash(2000);
    assert!(info.is_slashed());
    assert_eq!(info.slashed_amount(), 100_000_000_000);
    assert_eq!(info.remaining_bond(), 0);

    // Slashed producers cannot produce
    assert!(!info.is_active());
    assert!(!info.can_produce());
}

#[test]
fn test_producer_set() {
    let mut set = ProducerSet::new();

    let keypair = KeyPair::generate();
    let info = ProducerInfo::new(
        *keypair.public_key(),
        1000,
        100_000_000_000,
        (Hash::ZERO, 0),
        0,
        BOND_UNIT,
    );

    // Register
    set.register(info.clone(), 1000).unwrap();
    assert_eq!(set.active_count(), 1);

    // Can't register twice
    assert!(set.register(info, 1000).is_err());

    // Request exit (starts 7-day unbonding period)
    set.request_exit(keypair.public_key(), 2000).unwrap();
    assert_eq!(set.active_count(), 1); // Still active during unbonding

    // Process unbonding (test uses 43,200 blocks as arbitrary duration)
    let completed = set.process_unbonding(45_200, 43_200);
    assert_eq!(completed.len(), 1);
    assert_eq!(set.active_count(), 0);
}

#[test]
fn test_renewal() {
    let mut set = ProducerSet::new();

    let keypair = KeyPair::generate();
    let info = ProducerInfo::new(
        *keypair.public_key(),
        1000,
        100_000_000_000,
        (Hash::ZERO, 0),
        0,
        BOND_UNIT,
    );

    set.register(info, 1000).unwrap();

    // Renew with new bond
    let old_bond = set
        .renew(
            keypair.public_key(),
            70_000_000_000, // Era 2 bond
            (Hash::ZERO, 1),
            2_103_400,
            1,
        )
        .unwrap();

    assert_eq!(old_bond, 100_000_000_000);

    let renewed = set.get_by_pubkey(keypair.public_key()).unwrap();
    assert_eq!(renewed.bond_amount, 70_000_000_000);
    assert_eq!(renewed.registration_era, 1);
}

#[test]
fn test_pending_rewards() {
    let mut info = make_producer(1000);

    // Initially no pending rewards
    assert_eq!(info.pending_rewards, 0);
    assert!(!info.has_pending_rewards());

    // Credit some rewards
    info.credit_reward(500_000_000);
    assert_eq!(info.pending_rewards, 500_000_000);
    assert!(info.has_pending_rewards());

    // Credit more rewards (accumulates)
    info.credit_reward(300_000_000);
    assert_eq!(info.pending_rewards, 800_000_000);

    // Claim rewards
    let claimed = info.claim_rewards();
    assert_eq!(claimed, Some(800_000_000));
    assert_eq!(info.pending_rewards, 0);
    assert!(!info.has_pending_rewards());

    // Claim again when empty
    let claimed_again = info.claim_rewards();
    assert_eq!(claimed_again, None);
}

#[test]
fn test_producer_set_rewards() {
    let mut set = ProducerSet::new();

    let keypair = KeyPair::generate();
    let info = ProducerInfo::new(
        *keypair.public_key(),
        1000,
        100_000_000_000,
        (Hash::ZERO, 0),
        0,
        BOND_UNIT,
    );

    set.register(info, 1000).unwrap();

    // No pending rewards initially
    assert_eq!(set.pending_rewards(keypair.public_key()), 0);

    // Distribute block reward
    set.distribute_block_reward(keypair.public_key(), 500_000_000);
    assert_eq!(set.pending_rewards(keypair.public_key()), 500_000_000);

    // Check blocks_produced was incremented
    let producer = set.get_by_pubkey(keypair.public_key()).unwrap();
    assert_eq!(producer.blocks_produced, 1);

    // Distribute more rewards
    set.distribute_block_reward(keypair.public_key(), 500_000_000);
    assert_eq!(set.pending_rewards(keypair.public_key()), 1_000_000_000);
    assert_eq!(
        set.get_by_pubkey(keypair.public_key())
            .unwrap()
            .blocks_produced,
        2
    );

    // Process claim
    let claimed = set.process_claim(keypair.public_key());
    assert_eq!(claimed, Some(1_000_000_000));
    assert_eq!(set.pending_rewards(keypair.public_key()), 0);

    // Claim again when empty
    let claimed_again = set.process_claim(keypair.public_key());
    assert_eq!(claimed_again, None);
}

#[test]
fn test_reward_overflow_protection() {
    let mut info = make_producer(1000);

    // Credit a large amount
    info.credit_reward(u64::MAX - 100);
    assert_eq!(info.pending_rewards, u64::MAX - 100);

    // Credit more - should saturate at u64::MAX instead of overflow
    info.credit_reward(1000);
    assert_eq!(info.pending_rewards, u64::MAX);
}

// ==================== Seniority Weight Tests ====================

#[test]
fn test_weight_increases_with_age() {
    // Weight increases in discrete yearly steps: 0-1=1, 1-2=2, 2-3=3, 3+=4
    assert_eq!(producer_weight(0, 0), 1); // Year 0
    assert_eq!(producer_weight(0, BLOCKS_PER_YEAR - 1), 1); // Just before year 1
    assert_eq!(producer_weight(0, BLOCKS_PER_YEAR), 2); // Year 1
    assert_eq!(producer_weight(0, 2 * BLOCKS_PER_YEAR), 3); // Year 2
    assert_eq!(producer_weight(0, 3 * BLOCKS_PER_YEAR), 4); // Year 3
    assert_eq!(producer_weight(0, 4 * BLOCKS_PER_YEAR), 4); // Year 4 (max)
    assert_eq!(producer_weight(0, 10 * BLOCKS_PER_YEAR), 4); // Year 10 (still max)
}

#[test]
fn test_new_producer_has_weight_1() {
    let info = make_producer(1000);
    assert_eq!(info.weight(1000), 1);
    assert_eq!(info.weight(1500), 1);
    assert_eq!(info.weight(500_000), 1); // Still in year 0
}

#[test]
fn test_producer_weight_over_time() {
    let info = make_producer(0);

    // Discrete yearly steps: 0-1=1, 1-2=2, 2-3=3, 3+=4
    assert_eq!(info.weight(BLOCKS_PER_YEAR), 2); // Year 1
    assert_eq!(info.weight(2 * BLOCKS_PER_YEAR), 3); // Year 2
    assert_eq!(info.weight(3 * BLOCKS_PER_YEAR), 4); // Year 3
    assert_eq!(info.weight(4 * BLOCKS_PER_YEAR), 4); // Year 4 (max)

    // After 10 years, still weight 4 (max)
    assert_eq!(info.weight(10 * BLOCKS_PER_YEAR), 4);
}

#[test]
fn test_weighted_veto_threshold() {
    let mut set = ProducerSet::new();
    let current_height = 0;

    // Register 3 new producers (each weight 1)
    for i in 0..3 {
        let keypair = KeyPair::generate();
        let info = ProducerInfo::new(
            *keypair.public_key(),
            current_height,
            100_000_000_000,
            (Hash::ZERO, i),
            0,
            BOND_UNIT,
        );
        set.register(info, current_height).unwrap();
    }

    // Total weight = 3 (3 producers × weight 1)
    assert_eq!(set.total_weight(current_height), 3);

    // 40% of 3 = 1.2, rounded up = 2
    let threshold = set.weighted_veto_threshold(current_height);
    assert_eq!(threshold, 2);
}

#[test]
fn test_weighted_veto_threshold_mixed_ages() {
    let mut set = ProducerSet::new();
    let current_height = 9 * BLOCKS_PER_YEAR; // 9 years into network for max weight difference

    // Register a producer from genesis (9 years old, weight 4 = max)
    let keypair1 = KeyPair::generate();
    let info1 = ProducerInfo::new(
        *keypair1.public_key(),
        0, // registered at genesis
        100_000_000_000,
        (Hash::ZERO, 0),
        0,
        BOND_UNIT,
    );
    set.register(info1, 0).unwrap();

    // Register a new producer (0 years old, weight 1)
    let keypair2 = KeyPair::generate();
    let info2 = ProducerInfo::new(
        *keypair2.public_key(),
        current_height,
        100_000_000_000,
        (Hash::ZERO, 1),
        0,
        BOND_UNIT,
    );
    set.register(info2, current_height).unwrap();

    // Both producers must be actively producing to have governance power
    // Record recent activity (simulates they produced blocks recently)
    if let Some(p) = set.get_by_pubkey_mut(keypair1.public_key()) {
        p.last_activity = current_height;
    }
    if let Some(p) = set.get_by_pubkey_mut(keypair2.public_key()) {
        p.last_activity = current_height;
    }

    // Total weight = 4 + 1 = 5
    assert_eq!(set.total_weight(current_height), 5);

    // For governance (veto), only Active producers count
    // Both are now Active, so effective weight for governance = 4 + 1 = 5
    // 40% of 5 = 2.0, rounded up = 2
    let threshold = set.effective_veto_threshold(current_height);
    assert_eq!(threshold, 2);

    // Senior producer alone (weight 4) can veto (4 >= 2 threshold)
    assert!(set.has_weighted_veto(&[*keypair1.public_key()], current_height));

    // Junior producer alone (weight 1) cannot veto
    assert!(!set.has_weighted_veto(&[*keypair2.public_key()], current_height));
}

#[test]
fn test_rewards_proportional_to_weight() {
    let mut set = ProducerSet::new();
    let current_height = BLOCKS_PER_YEAR; // 1 year in

    // Register a producer from genesis (1 year old, weight 2)
    let keypair1 = KeyPair::generate();
    let info1 = ProducerInfo::new(
        *keypair1.public_key(),
        0,
        100_000_000_000,
        (Hash::ZERO, 0),
        0,
        BOND_UNIT,
    );
    set.register(info1, 0).unwrap();

    // Register a new producer (0 years old, weight 1)
    let keypair2 = KeyPair::generate();
    let info2 = ProducerInfo::new(
        *keypair2.public_key(),
        current_height,
        100_000_000_000,
        (Hash::ZERO, 1),
        0,
        BOND_UNIT,
    );
    set.register(info2, current_height).unwrap();

    // Total weight = 2 + 1 = 3
    assert_eq!(set.total_weight(current_height), 3);

    // Distribute 300 units of reward
    let total_reward = 300_000_000u64;
    let distributed = set.distribute_weighted_rewards(total_reward, current_height);
    assert_eq!(distributed, 2);

    // Senior producer should get 2/3 of reward (200M)
    let senior_rewards = set.pending_rewards(keypair1.public_key());
    assert_eq!(senior_rewards, 200_000_000);

    // Junior producer should get 1/3 of reward (100M)
    let junior_rewards = set.pending_rewards(keypair2.public_key());
    assert_eq!(junior_rewards, 100_000_000);
}

#[test]
fn test_weighted_active_producers() {
    let mut set = ProducerSet::new();
    let current_height = 9 * BLOCKS_PER_YEAR; // 9 years for max weight

    // Register producer at genesis (weight 4 at 9 years)
    let keypair = KeyPair::generate();
    let info = ProducerInfo::new(
        *keypair.public_key(),
        0,
        100_000_000_000,
        (Hash::ZERO, 0),
        0,
        BOND_UNIT,
    );
    set.register(info, 0).unwrap();

    let weighted = set.weighted_active_producers(current_height);
    assert_eq!(weighted.len(), 1);
    assert_eq!(weighted[0].1, 4); // weight 4 at year 9 (max)
}

#[test]
fn test_weight_calculation_boundary() {
    // Test exactly at year boundaries
    assert_eq!(producer_weight(0, BLOCKS_PER_YEAR - 1), 1); // Just before year 1
    assert_eq!(producer_weight(0, BLOCKS_PER_YEAR), 2); // Exactly year 1
    assert_eq!(producer_weight(0, BLOCKS_PER_YEAR + 1), 2); // Just after year 1
}

// ==================== Maturity Cooldown Tests (Anti-Sybil) ====================

#[test]
fn test_new_producer_has_no_prior_exit() {
    let info = make_producer(1000);
    assert!(!info.has_prior_exit);
}

#[test]
fn test_exit_records_in_history() {
    let mut set = ProducerSet::new();

    let keypair = KeyPair::generate();
    let info = ProducerInfo::new(
        *keypair.public_key(),
        0,
        100_000_000_000,
        (Hash::ZERO, 0),
        0,
        BOND_UNIT,
    );

    set.register(info, 0).unwrap();

    // Initially not in exit history
    assert!(!set.has_prior_exit(keypair.public_key(), 0));

    // Request exit
    set.request_exit(keypair.public_key(), 1000).unwrap();

    // Still not in exit history (unbonding not complete)
    assert!(!set.has_prior_exit(keypair.public_key(), 1000));

    // Complete unbonding (test uses arbitrary 43,200 blocks)
    let unbonding_duration = 43_200;
    let exit_height = 1000 + unbonding_duration;
    set.process_unbonding(exit_height, unbonding_duration);

    // Now in exit history
    assert!(set.has_prior_exit(keypair.public_key(), exit_height));
}

#[test]
fn test_re_registration_after_exit_has_prior_exit_flag() {
    let mut set = ProducerSet::new();

    let keypair = KeyPair::generate();
    let pubkey = *keypair.public_key();

    // First registration
    let info1 = ProducerInfo::new(pubkey, 0, 100_000_000_000, (Hash::ZERO, 0), 0, BOND_UNIT);
    set.register(info1, 0).unwrap();

    // Exit completely
    set.request_exit(&pubkey, 1000).unwrap();
    set.process_unbonding(50_000, 43_200);
    set.cleanup_exited();

    // Verify producer is gone
    assert!(set.get_by_pubkey(&pubkey).is_none());
    assert!(set.has_prior_exit(&pubkey, 50_000));

    // Re-register
    let info2 = ProducerInfo::new(
        pubkey,
        100_000,
        100_000_000_000,
        (Hash::ZERO, 1),
        0,
        BOND_UNIT,
    );
    set.register(info2, 100_000).unwrap();

    // Verify has_prior_exit flag is set
    let producer = set.get_by_pubkey(&pubkey).unwrap();
    assert!(producer.has_prior_exit);

    // Weight should be 1 (starts fresh)
    assert_eq!(producer.weight(100_000), 1);
}

#[test]
fn test_maturity_cooldown_prevents_seniority_transfer() {
    let mut set = ProducerSet::new();

    let keypair = KeyPair::generate();
    let pubkey = *keypair.public_key();

    // Register at genesis
    let info1 = ProducerInfo::new(pubkey, 0, 100_000_000_000, (Hash::ZERO, 0), 0, BOND_UNIT);
    set.register(info1, 0).unwrap();

    // After 9 years, weight is 4 (max, with sqrt formula)
    let nine_years = 9 * BLOCKS_PER_YEAR;
    assert_eq!(set.get_by_pubkey(&pubkey).unwrap().weight(nine_years), 4);

    // Exit at year 9
    set.request_exit(&pubkey, nine_years).unwrap();
    set.process_unbonding(nine_years + 50_000, 43_200);
    set.cleanup_exited();

    // Re-register at year 10
    let ten_years = 10 * BLOCKS_PER_YEAR;
    let info2 = ProducerInfo::new(
        pubkey,
        ten_years,
        100_000_000_000,
        (Hash::ZERO, 1),
        0,
        BOND_UNIT,
    );
    set.register(info2, ten_years).unwrap();

    // Weight should be 1, not 4! (lost all seniority due to exit)
    let producer = set.get_by_pubkey(&pubkey).unwrap();
    assert!(producer.has_prior_exit);
    assert_eq!(producer.weight(ten_years), 1);

    // After another year, weight is 2
    assert_eq!(producer.weight(ten_years + BLOCKS_PER_YEAR), 2);
}

#[test]
fn test_slashed_producer_in_exit_history() {
    let mut set = ProducerSet::new();

    let keypair = KeyPair::generate();
    let pubkey = *keypair.public_key();

    let info = ProducerInfo::new(pubkey, 0, 100_000_000_000, (Hash::ZERO, 0), 0, BOND_UNIT);
    set.register(info, 0).unwrap();

    // Slash the producer
    set.slash_producer(&pubkey, 1000).unwrap();

    // Should be in exit history
    assert!(set.has_prior_exit(&pubkey, 1000));
}

#[test]
fn test_new_with_prior_exit_constructor() {
    let keypair = KeyPair::generate();
    let info = ProducerInfo::new_with_prior_exit(
        *keypair.public_key(),
        1000,
        100_000_000_000,
        (Hash::ZERO, 0),
        0,
        BOND_UNIT,
    );

    assert!(info.has_prior_exit);
    assert_eq!(info.weight(1000), 1);
}

// ==================== Network-Aware Tests ====================

#[test]
fn test_devnet_time_acceleration_weight() {
    let devnet = Network::Devnet;

    // On devnet, blocks_per_year = 144 (accelerated time for ~10min eras)
    // Discrete yearly steps: 0-1=1, 1-2=2, 2-3=3, 3+=4
    let registered_at = 0;

    // After 0 blocks, weight = 1
    assert_eq!(producer_weight_for_network(registered_at, 0, devnet), 1);

    // After 144 blocks (1 simulated year), weight = 2
    assert_eq!(producer_weight_for_network(registered_at, 144, devnet), 2);

    // After 288 blocks (2 simulated years), weight = 3
    assert_eq!(producer_weight_for_network(registered_at, 288, devnet), 3);

    // After 432 blocks (3 simulated years), weight = 4 (max)
    assert_eq!(producer_weight_for_network(registered_at, 432, devnet), 4);

    // After 576 blocks (4 simulated years), still weight = 4
    assert_eq!(producer_weight_for_network(registered_at, 576, devnet), 4);
}

#[test]
fn test_devnet_vs_mainnet_time_scales() {
    let devnet = Network::Devnet;
    let mainnet = Network::Mainnet;

    let registered_at = 0;

    // Discrete yearly steps: 0-1=1, 1-2=2, 2-3=3, 3+=4

    // Devnet: 144 blocks = 1 year (devnet.blocks_per_year() = 144)
    assert_eq!(producer_weight_for_network(registered_at, 144, devnet), 2);

    // Mainnet: blocks_per_year() blocks = 1 year
    assert_eq!(
        producer_weight_for_network(registered_at, mainnet.blocks_per_year(), mainnet),
        2
    );

    // Devnet: 432 blocks = 3 years (max weight)
    assert_eq!(producer_weight_for_network(registered_at, 432, devnet), 4);

    // Mainnet: 3 * blocks_per_year() blocks = 3 years (max weight)
    assert_eq!(
        producer_weight_for_network(registered_at, 3 * mainnet.blocks_per_year(), mainnet),
        4
    );
}

#[test]
fn test_producer_info_weight_for_network() {
    let keypair = KeyPair::generate();
    let info = ProducerInfo::new(
        *keypair.public_key(),
        0,
        100_000_000_000,
        (Hash::ZERO, 0),
        0,
        BOND_UNIT,
    );

    let devnet = Network::Devnet;

    // Discrete yearly steps: 0-1=1, 1-2=2, 2-3=3, 3+=4

    // After 144 blocks on devnet (1 simulated year)
    assert_eq!(info.weight_for_network(144, devnet), 2);

    // After 432 blocks on devnet (3 simulated years)
    assert_eq!(info.weight_for_network(432, devnet), 4);
}

#[test]
fn test_producer_set_network_aware_methods() {
    let mut set = ProducerSet::new();
    let devnet = Network::Devnet;

    // Register producer at block 0
    let keypair1 = KeyPair::generate();
    let info1 = ProducerInfo::new(
        *keypair1.public_key(),
        0,
        100_000_000_000,
        (Hash::ZERO, 0),
        0,
        BOND_UNIT,
    );
    set.register_for_network(info1, 0, devnet).unwrap();

    // Register second producer at block 540 (9 years on devnet: 60 × 9 = 540)
    let keypair2 = KeyPair::generate();
    let info2 = ProducerInfo::new(
        *keypair2.public_key(),
        540,
        100_000_000_000,
        (Hash::ZERO, 1),
        0,
        BOND_UNIT,
    );
    set.register_for_network(info2, 540, devnet).unwrap();

    // Both producers must be actively producing to have governance power
    // Record recent activity (devnet has 10-block inactivity threshold)
    if let Some(p) = set.get_by_pubkey_mut(keypair1.public_key()) {
        p.last_activity = 540; // Recently active
    }
    if let Some(p) = set.get_by_pubkey_mut(keypair2.public_key()) {
        p.last_activity = 540; // Just registered, so active
    }

    // At block 540:
    // - Producer 1: 540 blocks = 9 years, weight = 4
    // - Producer 2: 0 blocks = 0 years, weight = 1
    // Total weight = 5 (for reward distribution, which uses all producers)
    assert_eq!(set.total_weight_for_network(540, devnet), 5);

    // For governance, both are Active (recently produced blocks)
    // Effective veto threshold = 40% of 5 = 2.0, rounded up = 2
    assert_eq!(set.effective_veto_threshold_for_network(540, devnet), 2);

    // Senior producer (weight 4) can veto alone (4 >= 2 threshold)
    assert!(set.has_weighted_veto_for_network(&[*keypair1.public_key()], 540, devnet));

    // Junior producer (weight 1) cannot veto alone
    assert!(!set.has_weighted_veto_for_network(&[*keypair2.public_key()], 540, devnet));
}

#[test]
fn test_devnet_exit_history_expiration() {
    let mut set = ProducerSet::new();
    let devnet = Network::Devnet;

    let keypair = KeyPair::generate();
    let pubkey = *keypair.public_key();

    // Register at block 0
    let info1 = ProducerInfo::new(pubkey, 0, 100_000_000_000, (Hash::ZERO, 0), 0, BOND_UNIT);
    set.register_for_network(info1, 0, devnet).unwrap();

    // Exit at block 576 (4 years on devnet: 144 blocks/year × 4)
    set.request_exit(&pubkey, 576).unwrap();
    set.process_unbonding(576 + 60, 60); // devnet unbonding = 60 blocks
    set.cleanup_exited();

    // At block 700, exit is recent (within 8 year retention = 1152 blocks)
    assert!(set.has_prior_exit_for_network(&pubkey, 700, devnet));

    // At block 2000, exit has expired (636 + 1152 = 1788, and 2000 > 1788)
    assert!(!set.has_prior_exit_for_network(&pubkey, 2000, devnet));
}

#[test]
fn test_devnet_inactivity_threshold() {
    let keypair = KeyPair::generate();
    let mut info = ProducerInfo::new(
        *keypair.public_key(),
        0,
        100_000_000_000,
        (Hash::ZERO, 0),
        0,
        BOND_UNIT,
    );

    let devnet = Network::Devnet;

    // Devnet inactivity threshold = 30 blocks
    assert!(!info.is_inactive_for_network(15, devnet)); // 15 block gap, not inactive
    assert!(!info.is_inactive_for_network(30, devnet)); // 30 blocks, still not inactive
    assert!(info.is_inactive_for_network(31, devnet)); // 31 blocks, inactive

    // Record activity updates last_activity timestamp
    // Note: Activity gaps are no longer tracked per alignment decision
    info.record_activity_for_network(35, devnet);
    assert_eq!(info.last_activity, 35);

    info.record_activity_for_network(50, devnet);
    assert_eq!(info.last_activity, 50);

    info.record_activity_for_network(85, devnet);
    assert_eq!(info.last_activity, 85);
}

#[test]
fn test_devnet_weighted_rewards_distribution() {
    let mut set = ProducerSet::new();
    let devnet = Network::Devnet;

    // Register producer at block 0 (will have 1 year seniority at block 144)
    let keypair1 = KeyPair::generate();
    let info1 = ProducerInfo::new(
        *keypair1.public_key(),
        0,
        100_000_000_000,
        (Hash::ZERO, 0),
        0,
        BOND_UNIT,
    );
    set.register_for_network(info1, 0, devnet).unwrap();

    // Register producer at block 144 (0 seniority at block 144)
    let keypair2 = KeyPair::generate();
    let info2 = ProducerInfo::new(
        *keypair2.public_key(),
        144,
        100_000_000_000,
        (Hash::ZERO, 1),
        0,
        BOND_UNIT,
    );
    set.register_for_network(info2, 144, devnet).unwrap();

    // At block 144:
    // Producer 1: 144 blocks = 1 year, weight = 2
    // Producer 2: 0 blocks = 0 years, weight = 1
    // Total weight = 3

    let total_reward = 300_000_000u64;
    let distributed = set.distribute_weighted_rewards_for_network(total_reward, 144, devnet);
    assert_eq!(distributed, 2);

    // Producer 1 gets 2/3 = 200M
    assert_eq!(set.pending_rewards(keypair1.public_key()), 200_000_000);

    // Producer 2 gets 1/3 = 100M
    assert_eq!(set.pending_rewards(keypair2.public_key()), 100_000_000);
}

#[test]
fn test_era_simulation() {
    let devnet = Network::Devnet;

    // Verify the math for devnet era timing
    // Devnet: 10 second slots, 144 blocks = 1 year
    // Era = 576 blocks = 4 years (576 / 144 = 4)
    // Era duration = 576 * 10 = 5760 seconds = 96 minutes real time

    let era_blocks = 576u64;
    let simulated_years = era_blocks / devnet.blocks_per_year();
    let real_seconds = era_blocks * devnet.slot_duration();
    let real_minutes = real_seconds / 60;

    assert_eq!(simulated_years, 4);
    assert_eq!(real_minutes, 96); // 5760 seconds = 96 minutes

    // Verify weight after 1 era (4 simulated years) = max (4)
    let weight_at_4_years = producer_weight_for_network(0, era_blocks, devnet);
    assert_eq!(weight_at_4_years, 4);
}

#[test]
fn test_activity_status_system() {
    // Test the "el silencio no bloquea" principle:
    // Only Active producers can participate in governance
    let mut set = ProducerSet::new();

    // Register 3 producers at genesis
    let keypair_active = KeyPair::generate();
    let keypair_dormant = KeyPair::generate();
    let keypair_recent = KeyPair::generate();

    for (i, kp) in [&keypair_active, &keypair_dormant, &keypair_recent]
        .iter()
        .enumerate()
    {
        let info = ProducerInfo::new(
            *kp.public_key(),
            0,
            100_000_000_000,
            (Hash::ZERO, i as u32),
            0,
            BOND_UNIT,
        );
        set.register(info, 0).unwrap();
    }

    // After 4 years, check at height 4 * BLOCKS_PER_YEAR
    let current_height = 4 * BLOCKS_PER_YEAR;

    // Simulate activity patterns:
    // - keypair_active: produced block recently (within 1 week)
    // - keypair_dormant: no activity since genesis (Dormant)
    // - keypair_recent: inactive ~10 days ago (RecentlyInactive)
    if let Some(p) = set.get_by_pubkey_mut(keypair_active.public_key()) {
        // ~2 hours ago at 10s slots, still Active (within 1 week threshold)
        p.last_activity = current_height - 720;
    }
    if let Some(p) = set.get_by_pubkey_mut(keypair_dormant.public_key()) {
        p.last_activity = 0; // Genesis, way past threshold
    }
    if let Some(p) = set.get_by_pubkey_mut(keypair_recent.public_key()) {
        // ~10 days ago: INACTIVITY_THRESHOLD + ~3 days = between 1-2 weeks
        p.last_activity = current_height - (INACTIVITY_THRESHOLD + 25_000);
    }

    // Verify activity statuses
    let active_info = set.get_by_pubkey(keypair_active.public_key()).unwrap();
    let dormant_info = set.get_by_pubkey(keypair_dormant.public_key()).unwrap();
    let recent_info = set.get_by_pubkey(keypair_recent.public_key()).unwrap();

    assert_eq!(
        active_info.activity_status(current_height),
        ActivityStatus::Active
    );
    assert_eq!(
        dormant_info.activity_status(current_height),
        ActivityStatus::Dormant
    );
    assert_eq!(
        recent_info.activity_status(current_height),
        ActivityStatus::RecentlyInactive
    );

    // Only Active producer has governance power
    assert!(active_info.has_governance_power(current_height));
    assert!(!dormant_info.has_governance_power(current_height));
    assert!(!recent_info.has_governance_power(current_height));

    // Only Active producer counts for quorum
    assert!(active_info.counts_for_quorum(current_height));
    assert!(!dormant_info.counts_for_quorum(current_height));
    assert!(!recent_info.counts_for_quorum(current_height));

    // All 3 have weight (for reward distribution - not governance)
    // All registered at genesis, so after 4 years they all have weight 4 (max)
    assert_eq!(set.total_weight(current_height), 12); // 3 * 4 = 12

    // But for governance (effective weight), only 1 Active producer counts
    assert_eq!(set.governance_participant_count(current_height), 1);
    assert_eq!(set.total_effective_weight(current_height), 4); // Only active producer's weight (max)

    // Veto test: The active producer alone has 100% of governance weight
    // 40% of 4 = 1.6, rounded up = 2
    // Active producer has weight 4, so they CAN veto alone (4 >= 2)
    assert!(set.has_weighted_veto(&[*keypair_active.public_key()], current_height));

    // Dormant producer has no governance power, cannot veto even with high seniority
    assert!(!set.has_weighted_veto(&[*keypair_dormant.public_key()], current_height));

    // RecentlyInactive producer also has no governance power
    assert!(!set.has_weighted_veto(&[*keypair_recent.public_key()], current_height));

    // Key insight: "El silencio no bloquea"
    // Even if dormant + recent tried to veto together, they have 0 governance weight
    assert!(!set.has_weighted_veto(
        &[*keypair_dormant.public_key(), *keypair_recent.public_key()],
        current_height
    ));
}

#[test]
fn test_cancel_exit() {
    let mut set = ProducerSet::new();
    let keypair = KeyPair::generate();
    let pubkey = *keypair.public_key();

    // Register producer
    let info = ProducerInfo::new(pubkey, 0, 100_000_000_000, (Hash::ZERO, 0), 0, BOND_UNIT);
    set.register(info, 0).unwrap();

    // Verify initially active
    assert!(set.get_by_pubkey(&pubkey).unwrap().is_active());
    assert_eq!(
        set.get_by_pubkey(&pubkey).unwrap().status,
        ProducerStatus::Active
    );

    // Cannot cancel exit when not in unbonding
    assert!(set.cancel_exit(&pubkey).is_err());

    // Request exit (start unbonding)
    set.request_exit(&pubkey, 1000).unwrap();
    assert!(matches!(
        set.get_by_pubkey(&pubkey).unwrap().status,
        ProducerStatus::Unbonding { started_at: 1000 }
    ));

    // Producer can still produce during unbonding
    assert!(set.get_by_pubkey(&pubkey).unwrap().can_produce());

    // Cancel exit - returns to Active
    set.cancel_exit(&pubkey).unwrap();
    assert_eq!(
        set.get_by_pubkey(&pubkey).unwrap().status,
        ProducerStatus::Active
    );

    // Can request exit again after canceling
    set.request_exit(&pubkey, 2000).unwrap();
    assert!(matches!(
        set.get_by_pubkey(&pubkey).unwrap().status,
        ProducerStatus::Unbonding { started_at: 2000 }
    ));

    // Cancel again
    set.cancel_exit(&pubkey).unwrap();
    assert_eq!(
        set.get_by_pubkey(&pubkey).unwrap().status,
        ProducerStatus::Active
    );

    // Cannot cancel exit after it's complete
    set.request_exit(&pubkey, 3000).unwrap();
    // Simulate unbonding completion (test uses 43,200 blocks as arbitrary duration)
    set.process_unbonding(3000 + 43_200 + 1, 43_200);

    // Now producer is Exited, cannot cancel
    assert!(set.cancel_exit(&pubkey).is_err());
}

#[test]
fn test_cancel_exit_preserves_seniority() {
    let mut set = ProducerSet::new();
    let keypair = KeyPair::generate();
    let pubkey = *keypair.public_key();

    // Register producer at block 0
    let info = ProducerInfo::new(
        pubkey,
        0, // registered_at = 0
        100_000_000_000,
        (Hash::ZERO, 0),
        0,
        BOND_UNIT,
    );
    set.register(info, 0).unwrap();

    // After 4 years, check weight (discrete steps: 3+ years = weight 4)
    let height_4_years = 4 * BLOCKS_PER_YEAR;
    let weight_before = set.get_by_pubkey(&pubkey).unwrap().weight(height_4_years);
    assert_eq!(weight_before, 4); // 4 years = max weight

    // Request exit
    set.request_exit(&pubkey, height_4_years).unwrap();

    // Cancel exit
    set.cancel_exit(&pubkey).unwrap();

    // Weight should be preserved (registered_at unchanged)
    let weight_after = set.get_by_pubkey(&pubkey).unwrap().weight(height_4_years);
    assert_eq!(weight_after, weight_before);

    // Verify registered_at is still 0
    assert_eq!(set.get_by_pubkey(&pubkey).unwrap().registered_at, 0);
}

#[test]
fn test_active_cache_basic() {
    let mut set = ProducerSet::new();
    let p1 = make_producer(0);
    let pubkey1 = p1.public_key;
    set.register(p1, 0).unwrap();

    let height = ACTIVATION_DELAY + 1;

    // Before caching: slow path works
    assert_eq!(set.active_producers_at_height(height).len(), 1);
    assert!(set.active_cache.is_none());

    // Build cache
    set.ensure_active_cache(height);
    assert!(set.active_cache.is_some());

    // Cached path returns same result
    assert_eq!(set.active_producers_at_height(height).len(), 1);
    assert_eq!(
        set.active_producers_at_height(height)[0].public_key,
        pubkey1
    );
}

#[test]
fn test_active_cache_invalidation() {
    let mut set = ProducerSet::new();
    let p1 = make_producer(0);
    let pubkey1 = p1.public_key;
    set.register(p1, 0).unwrap();

    let height = ACTIVATION_DELAY + 1;
    set.ensure_active_cache(height);
    assert!(set.active_cache.is_some());

    // Register new producer → invalidates cache
    let p2 = make_producer(0);
    set.register(p2, 0).unwrap();
    assert!(set.active_cache.is_none());

    // Rebuild and verify both producers present
    set.ensure_active_cache(height);
    assert_eq!(set.active_producers_at_height(height).len(), 2);

    // Exit → invalidates cache
    set.request_exit(&pubkey1, height).unwrap();
    assert!(set.active_cache.is_none());
}

#[test]
fn test_active_cache_height_change() {
    let mut set = ProducerSet::new();
    let p1 = make_producer(100);
    set.register(p1, 100).unwrap();

    // At height 105: producer NOT yet eligible (ACTIVATION_DELAY=10)
    set.ensure_active_cache(105);
    assert_eq!(set.active_producers_at_height(105).len(), 0);

    // Cache is valid at same height
    assert!(set.active_cache.is_some());

    // At height 111: producer IS eligible
    set.ensure_active_cache(111);
    assert_eq!(set.active_producers_at_height(111).len(), 1);

    // Different height → cache miss (uses slow path but still correct)
    assert_eq!(set.active_producers_at_height(200).len(), 1);
}

#[test]
fn test_active_cache_slash_invalidation() {
    let mut set = ProducerSet::new();
    let p1 = make_producer(0);
    let pubkey1 = p1.public_key;
    set.register(p1, 0).unwrap();

    let height = ACTIVATION_DELAY + 1;
    set.ensure_active_cache(height);
    assert_eq!(set.active_producers_at_height(height).len(), 1);

    // Slash → invalidates cache
    set.slash_producer(&pubkey1, height).unwrap();
    assert!(set.active_cache.is_none());

    // No active producers after slash
    assert_eq!(set.active_producers_at_height(height).len(), 0);
}

#[test]
fn test_duplicate_register_preserves_existing_producer() {
    let mut ps = ProducerSet::new();
    let kp = KeyPair::generate();

    // Register producer at height 100
    let info =
        ProducerInfo::new_with_bonds(*kp.public_key(), 100, BOND_UNIT, (Hash::ZERO, 0), 0, 1);
    ps.register(info, 100).unwrap();
    assert_eq!(ps.active_count(), 1);

    // Queue duplicate registration at height 200
    let dup_info =
        ProducerInfo::new_with_bonds(*kp.public_key(), 200, BOND_UNIT, (Hash::ZERO, 1), 0, 1);
    ps.queue_update(PendingProducerUpdate::Register {
        info: Box::new(dup_info),
        height: 200,
    });

    // Apply pending updates — duplicate rejected, original preserved
    ps.apply_pending_updates();
    assert_eq!(ps.active_count(), 1);

    let producer = ps.get_by_pubkey(kp.public_key()).unwrap();
    assert_eq!(producer.registered_at, 100); // Original, not 200
}

// ==================== Reactive Round-Robin Scheduling Tests ====================

/// Test: Producers start scheduled by default
#[test]
fn test_reactive_producers_start_scheduled() {
    let info = make_producer(100);
    assert!(info.scheduled, "New producers must be scheduled by default");
}

/// Test: scheduled_producers_at_height returns only scheduled producers
#[test]
fn test_reactive_scheduled_producers_filter() {
    let mut ps = ProducerSet::new();
    let kp1 = KeyPair::generate();
    let kp2 = KeyPair::generate();
    let kp3 = KeyPair::generate();

    // Register 3 genesis producers (registered_at=0 bypasses activation delay)
    for kp in [&kp1, &kp2, &kp3] {
        let info = ProducerInfo::new(
            *kp.public_key(),
            0, // genesis
            100_000_000_000,
            (Hash::ZERO, 0),
            0,
            BOND_UNIT,
        );
        ps.register(info, 0).unwrap();
    }

    // All 3 should be scheduled
    assert_eq!(ps.scheduled_producers_at_height(100).len(), 3);
    assert_eq!(ps.scheduled_count_at_height(100), 3);

    // Unschedule one producer
    ps.unschedule_producer(kp2.public_key());

    // Now only 2 should be scheduled
    assert_eq!(ps.scheduled_producers_at_height(100).len(), 2);
    assert_eq!(ps.scheduled_count_at_height(100), 2);

    // The unscheduled one should still be active (just not scheduled)
    let info = ps.get_by_pubkey(kp2.public_key()).unwrap();
    assert!(info.is_active());
    assert!(!info.scheduled);
}

/// Test: unschedule and reschedule round-trip
#[test]
fn test_reactive_unschedule_reschedule() {
    let mut ps = ProducerSet::new();
    let kp = KeyPair::generate();
    let info = ProducerInfo::new(
        *kp.public_key(),
        0,
        100_000_000_000,
        (Hash::ZERO, 0),
        0,
        BOND_UNIT,
    );
    ps.register(info, 0).unwrap();

    // Initially scheduled
    assert!(ps.get_by_pubkey(kp.public_key()).unwrap().scheduled);

    // Unschedule
    ps.unschedule_producer(kp.public_key());
    assert!(!ps.get_by_pubkey(kp.public_key()).unwrap().scheduled);

    // Reschedule
    ps.schedule_producer(kp.public_key());
    assert!(ps.get_by_pubkey(kp.public_key()).unwrap().scheduled);
}

/// Test: Equal-weight round-robin scheduler behavior
#[test]
fn test_reactive_equal_weight_round_robin() {
    use doli_core::scheduler::{DeterministicScheduler, ScheduledProducer};

    let mut producers = Vec::new();
    let mut pubkeys = Vec::new();
    for i in 0..5u8 {
        let mut bytes = [0u8; 32];
        bytes[0] = i;
        let pk = crypto::PublicKey::from_bytes(bytes);
        pubkeys.push(pk);
        // Each producer gets exactly 1 ticket (equal weight)
        producers.push(ScheduledProducer::new(pk, 1));
    }

    let scheduler = DeterministicScheduler::new(producers);

    // With 5 producers and 1 ticket each, total = 5
    assert_eq!(scheduler.total_bonds(), 5);

    // Pure round-robin: each producer gets exactly 1 slot per cycle
    let mut slot_counts = [0u32; 5];
    for slot in 0..50 {
        if let Some(pk) = scheduler.select_producer(slot, 0) {
            for (i, ref_pk) in pubkeys.iter().enumerate() {
                if pk == ref_pk {
                    slot_counts[i] += 1;
                    break;
                }
            }
        }
    }

    // Each producer should get exactly 10 slots out of 50 (50/5 = 10 each)
    for (i, count) in slot_counts.iter().enumerate() {
        assert_eq!(
            *count, 10,
            "Producer {} got {} slots, expected 10 (equal distribution)",
            i, count
        );
    }
}

/// Test: Schedule shrinks when producers are removed, expands when they return
#[test]
fn test_reactive_schedule_shrinks_and_expands() {
    use doli_core::scheduler::{DeterministicScheduler, ScheduledProducer};

    let mut pubkeys = Vec::new();
    for i in 0..5u8 {
        let mut bytes = [0u8; 32];
        bytes[0] = i;
        pubkeys.push(crypto::PublicKey::from_bytes(bytes));
    }

    // Full schedule: 5 producers, 1 ticket each
    let scheduler_full = DeterministicScheduler::new(
        pubkeys
            .iter()
            .map(|pk| ScheduledProducer::new(*pk, 1))
            .collect(),
    );
    assert_eq!(scheduler_full.producer_count(), 5);

    // Simulate: producer 0 and 1 go offline → rebuild with 3 producers
    let scheduler_reduced = DeterministicScheduler::new(
        pubkeys[2..]
            .iter()
            .map(|pk| ScheduledProducer::new(*pk, 1))
            .collect(),
    );
    assert_eq!(scheduler_reduced.producer_count(), 3);

    // With only 3 producers, cycle is 3 slots — 100% coverage if all online
    let mut produced = 0u32;
    for slot in 0..30 {
        if scheduler_reduced.select_producer(slot, 0).is_some() {
            produced += 1;
        }
    }
    assert_eq!(
        produced, 30,
        "All 30 slots should be filled with 3 producers"
    );

    // Producer 0 comes back → rebuild with 4 producers
    let scheduler_partial = DeterministicScheduler::new(
        [pubkeys[0]]
            .iter()
            .chain(pubkeys[2..].iter())
            .map(|pk| ScheduledProducer::new(*pk, 1))
            .collect(),
    );
    assert_eq!(scheduler_partial.producer_count(), 4);
}

/// Test: Bond weight does NOT affect scheduling (only rewards)
#[test]
fn test_reactive_bond_weight_irrelevant_for_scheduling() {
    use doli_core::scheduler::{DeterministicScheduler, ScheduledProducer};

    let mut bytes_whale = [0u8; 32];
    bytes_whale[0] = 1;
    let whale = crypto::PublicKey::from_bytes(bytes_whale);

    let mut bytes_small = [0u8; 32];
    bytes_small[0] = 2;
    let small = crypto::PublicKey::from_bytes(bytes_small);

    // Both get 1 ticket regardless of actual bond amount
    let scheduler = DeterministicScheduler::new(vec![
        ScheduledProducer::new(whale, 1), // whale with 885 bonds → 1 ticket
        ScheduledProducer::new(small, 1), // small with 7 bonds → 1 ticket
    ]);

    // Each gets exactly 50% of slots
    let mut whale_slots = 0u32;
    let mut small_slots = 0u32;
    for slot in 0..100 {
        if let Some(pk) = scheduler.select_producer(slot, 0) {
            if *pk == whale {
                whale_slots += 1;
            } else if *pk == small {
                small_slots += 1;
            }
        }
    }

    assert_eq!(whale_slots, 50, "Whale should get exactly 50% of slots");
    assert_eq!(
        small_slots, 50,
        "Small producer should get exactly 50% of slots"
    );
}
