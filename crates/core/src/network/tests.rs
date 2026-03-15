use super::*;

#[test]
fn test_network_ids_unique() {
    let networks = Network::all();
    let ids: Vec<u32> = networks.iter().map(|n| n.id()).collect();
    let unique: std::collections::HashSet<u32> = ids.iter().copied().collect();
    assert_eq!(ids.len(), unique.len(), "Network IDs must be unique");
}

#[test]
fn test_magic_bytes_unique() {
    let networks = Network::all();
    let magic: Vec<[u8; 4]> = networks.iter().map(|n| n.magic_bytes()).collect();
    let unique: std::collections::HashSet<[u8; 4]> = magic.iter().copied().collect();
    assert_eq!(magic.len(), unique.len(), "Magic bytes must be unique");
}

#[test]
fn test_address_prefixes_unique() {
    let networks = Network::all();
    let prefixes: Vec<&str> = networks.iter().map(|n| n.address_prefix()).collect();
    let unique: std::collections::HashSet<&str> = prefixes.iter().copied().collect();
    assert_eq!(
        prefixes.len(),
        unique.len(),
        "Address prefixes must be unique"
    );
}

#[test]
fn test_ports_unique() {
    let networks = Network::all();
    let p2p_ports: Vec<u16> = networks.iter().map(|n| n.default_p2p_port()).collect();
    let rpc_ports: Vec<u16> = networks.iter().map(|n| n.default_rpc_port()).collect();

    let unique_p2p: std::collections::HashSet<u16> = p2p_ports.iter().copied().collect();
    let unique_rpc: std::collections::HashSet<u16> = rpc_ports.iter().copied().collect();

    assert_eq!(
        p2p_ports.len(),
        unique_p2p.len(),
        "P2P ports must be unique"
    );
    assert_eq!(
        rpc_ports.len(),
        unique_rpc.len(),
        "RPC ports must be unique"
    );
}

#[test]
fn test_parse_network() {
    assert_eq!("mainnet".parse::<Network>().unwrap(), Network::Mainnet);
    assert_eq!("testnet".parse::<Network>().unwrap(), Network::Testnet);
    assert_eq!("devnet".parse::<Network>().unwrap(), Network::Devnet);
    assert_eq!("main".parse::<Network>().unwrap(), Network::Mainnet);
    assert_eq!("test".parse::<Network>().unwrap(), Network::Testnet);
    assert_eq!("dev".parse::<Network>().unwrap(), Network::Devnet);
    assert!("invalid".parse::<Network>().is_err());
}

#[test]
fn test_from_id() {
    assert_eq!(Network::from_id(1), Some(Network::Mainnet));
    assert_eq!(Network::from_id(2), Some(Network::Testnet));
    assert_eq!(Network::from_id(99), Some(Network::Devnet));
    assert_eq!(Network::from_id(0), None);
    assert_eq!(Network::from_id(100), None);
}

#[test]
fn test_mainnet_is_not_test() {
    assert!(!Network::Mainnet.is_test());
    assert!(Network::Testnet.is_test());
    assert!(Network::Devnet.is_test());
}

#[test]
fn test_display() {
    assert_eq!(format!("{}", Network::Mainnet), "mainnet");
    assert_eq!(format!("{}", Network::Testnet), "testnet");
    assert_eq!(format!("{}", Network::Devnet), "devnet");
}

// ==================== Time Acceleration Tests ====================

#[test]
fn test_devnet_time_acceleration() {
    let devnet = Network::Devnet;

    // 144 blocks = 1 simulated year (accelerated time)
    assert_eq!(devnet.blocks_per_year(), 144);

    // 576 blocks = 1 era (4 simulated years) ≈ 96 minutes at 10s slots
    assert_eq!(devnet.blocks_per_era(), 576);

    // 1 block = 10 seconds (same as mainnet for realistic testing)
    assert_eq!(devnet.slot_duration(), 10);

    // 144 blocks × 10 seconds = 1440 seconds = 24 minutes = 1 simulated year
    assert_eq!(devnet.blocks_per_year() * devnet.slot_duration(), 1440);

    // 1 month = 12 blocks
    assert_eq!(devnet.blocks_per_month(), 12);

    // Commitment period = 4 years = 576 blocks ≈ 96 minutes
    assert_eq!(devnet.commitment_period(), 576);

    // Exit history retention = 8 years = 1152 blocks ≈ 192 minutes
    assert_eq!(devnet.exit_history_retention(), 1152);
}

#[test]
fn test_testnet_same_as_mainnet() {
    let testnet = Network::Testnet;
    let mainnet = Network::Mainnet;

    // Testnet should have same parameters as mainnet
    assert_eq!(testnet.blocks_per_year(), mainnet.blocks_per_year());
    assert_eq!(testnet.blocks_per_era(), mainnet.blocks_per_era());
    assert_eq!(testnet.slot_duration(), mainnet.slot_duration());
    assert_eq!(testnet.initial_bond(), mainnet.initial_bond());
    assert_eq!(testnet.initial_reward(), mainnet.initial_reward());
    assert_eq!(
        testnet.vdf_discriminant_bits(),
        mainnet.vdf_discriminant_bits()
    );
    assert_eq!(
        testnet.heartbeat_vdf_iterations(),
        mainnet.heartbeat_vdf_iterations()
    );
}

#[test]
fn test_mainnet_real_time() {
    let mainnet = Network::Mainnet;

    // ~3.15M blocks per year (6 blocks per minute)
    assert_eq!(mainnet.blocks_per_year(), 3_153_600);

    // ~12.6M blocks per era (4 years)
    assert_eq!(mainnet.blocks_per_era(), 12_614_400);

    // 10 seconds per block
    assert_eq!(mainnet.slot_duration(), 10);

    // 3,153,600 blocks × 10 seconds = 31,536,000 seconds = 365.25 days
    assert_eq!(
        mainnet.blocks_per_year() * mainnet.slot_duration(),
        31_536_000
    );
}

#[test]
fn test_network_parameters_consistency() {
    for network in Network::all() {
        // blocks_per_month should be 1/12 of blocks_per_year
        assert_eq!(
            network.blocks_per_month() * 12,
            network.blocks_per_year(),
            "Months don't match years for {:?}",
            network
        );

        // blocks_per_era should be 4 × blocks_per_year
        assert_eq!(
            network.blocks_per_era(),
            network.blocks_per_year() * 4,
            "Era doesn't match 4 years for {:?}",
            network
        );

        // commitment_period equals blocks_per_era
        assert_eq!(
            network.commitment_period(),
            network.blocks_per_era(),
            "Commitment period doesn't match era for {:?}",
            network
        );

        // exit_history_retention is 2 eras (8 years)
        assert_eq!(
            network.exit_history_retention(),
            network.blocks_per_era() * 2,
            "Exit history retention doesn't match 2 eras for {:?}",
            network
        );
    }
}

#[test]
fn test_devnet_simulation_timing() {
    let devnet = Network::Devnet;

    // Verify era duration: 576 blocks × 10s = 5760 seconds ≈ 96 minutes
    assert_eq!(devnet.blocks_per_era(), 576);
    assert_eq!(
        devnet.blocks_per_era() * devnet.slot_duration(),
        5760,
        "1 era should = 5760 seconds ≈ 96 minutes (with 10s slots)"
    );

    // Verify 1 hour = 360 blocks (3600s / 10s), less than 1 era (576 blocks)
    let one_hour_blocks = 3600 / devnet.slot_duration();
    assert_eq!(one_hour_blocks, 360);
    let eras = one_hour_blocks / devnet.blocks_per_era();
    assert_eq!(eras, 0, "1 hour should < 1 era with 10s slots");

    // Verify inactivity threshold is quick for testing
    // 30 blocks × 10s = 300 seconds = 5 minutes
    assert_eq!(devnet.inactivity_threshold(), 30);

    // Verify unbonding is quick for testing
    // 60 blocks × 10s = 600 seconds = 10 minutes
    assert_eq!(devnet.unbonding_period(), 60);
}

#[test]
fn test_registration_fees_scale_by_network() {
    // Mainnet and Testnet have same fees
    assert_eq!(
        Network::Mainnet.registration_base_fee(),
        Network::Testnet.registration_base_fee()
    );
    assert!(Network::Testnet.registration_base_fee() > Network::Devnet.registration_base_fee());

    // Max fees: Mainnet and Testnet equal, Devnet lower
    assert_eq!(
        Network::Mainnet.max_registration_fee(),
        Network::Testnet.max_registration_fee()
    );
    assert!(Network::Testnet.max_registration_fee() > Network::Devnet.max_registration_fee());
}

#[test]
fn test_vdf_register_iterations_fixed() {
    // All networks use same fixed registration VDF (~30s)
    assert_eq!(
        Network::Mainnet.vdf_register_iterations(),
        Network::Testnet.vdf_register_iterations()
    );
    // All should be fast (5M iterations)
    assert!(Network::Mainnet.vdf_register_iterations() <= 10_000_000);
    assert!(Network::Devnet.vdf_register_iterations() <= 10_000_000);
}

#[test]
fn test_vdf_discriminant_bits_scale_by_network() {
    // Mainnet and Testnet have same discriminant (production security)
    assert_eq!(
        Network::Mainnet.vdf_discriminant_bits(),
        Network::Testnet.vdf_discriminant_bits()
    );
    assert!(Network::Testnet.vdf_discriminant_bits() > Network::Devnet.vdf_discriminant_bits());

    // Mainnet uses 2048-bit for production security
    assert_eq!(Network::Mainnet.vdf_discriminant_bits(), 2048);

    // Testnet uses same as mainnet (2048-bit)
    assert_eq!(Network::Testnet.vdf_discriminant_bits(), 2048);

    // Devnet uses 256-bit for rapid development
    assert_eq!(Network::Devnet.vdf_discriminant_bits(), 256);
}

#[test]
fn test_vdf_seeds_unique() {
    // Each network must have a unique VDF seed
    let mainnet_seed = Network::Mainnet.vdf_seed();
    let testnet_seed = Network::Testnet.vdf_seed();
    let devnet_seed = Network::Devnet.vdf_seed();

    assert_ne!(mainnet_seed, testnet_seed);
    assert_ne!(testnet_seed, devnet_seed);
    assert_ne!(mainnet_seed, devnet_seed);
}

#[test]
fn test_vdf_enabled() {
    // VDF should be enabled for all networks (using hash-chain VDF)
    assert!(Network::Mainnet.vdf_enabled());
    assert!(Network::Testnet.vdf_enabled());
    assert!(Network::Devnet.vdf_enabled()); // Uses fast hash-chain VDF (~700ms)
}

// ==================== Genesis Phase Tests ====================

#[test]
fn test_genesis_blocks_devnet() {
    let devnet = Network::Devnet;

    // Devnet has 40 block genesis phase
    assert_eq!(devnet.genesis_blocks(), 40);

    // Heights 1-40 are in genesis
    assert!(devnet.is_in_genesis(1));
    assert!(devnet.is_in_genesis(20));
    assert!(devnet.is_in_genesis(40));

    // Height 41 and beyond are NOT in genesis
    assert!(!devnet.is_in_genesis(41));
    assert!(!devnet.is_in_genesis(100));
}

#[test]
fn test_genesis_blocks_mainnet_testnet() {
    assert_eq!(Network::Mainnet.genesis_blocks(), 360);
    assert!(Network::Mainnet.is_in_genesis(1));
    assert!(Network::Mainnet.is_in_genesis(360));
    assert!(!Network::Mainnet.is_in_genesis(361));

    // Testnet has 1-hour genesis phase (same as mainnet)
    assert_eq!(Network::Testnet.genesis_blocks(), 360);
    assert!(Network::Testnet.is_in_genesis(1));
    assert!(Network::Testnet.is_in_genesis(360));
    assert!(!Network::Testnet.is_in_genesis(361));
}

#[test]
fn test_automatic_genesis_bond() {
    assert_eq!(Network::Mainnet.automatic_genesis_bond(), 1_000_000_000); // 10 DOLI
    assert_eq!(Network::Testnet.automatic_genesis_bond(), 1_000_000_000); // 10 DOLI
    assert_eq!(Network::Devnet.automatic_genesis_bond(), 100_000_000); // 1 DOLI
}

#[test]
fn test_genesis_math_devnet() {
    let devnet = Network::Devnet;

    // Verify the genesis math works out:
    // - Block reward: 20 DOLI per block
    // - 4 producers need 200 DOLI each = 800 DOLI total
    // - 800 DOLI / 20 DOLI per block = 40 blocks
    let block_reward_doli = devnet.initial_reward() / 100_000_000; // Convert to DOLI
    assert_eq!(block_reward_doli, 20);

    let genesis_blocks = devnet.genesis_blocks();
    let total_rewards = genesis_blocks * block_reward_doli;
    assert_eq!(total_rewards, 800); // Enough for 4 producers × 200 DOLI each
}

// ==================== Auto-Update System Parameter Tests ====================

#[test]
fn test_veto_period_by_network() {
    // Mainnet and Testnet: 5 minutes (early network, small maintainer set)
    assert_eq!(Network::Mainnet.veto_period_secs(), 5 * 60);
    assert_eq!(Network::Testnet.veto_period_secs(), 5 * 60);
    // Devnet: 1 minute for fast testing
    assert_eq!(Network::Devnet.veto_period_secs(), 60);
}

#[test]
fn test_grace_period_by_network() {
    // Mainnet and Testnet: 2 minutes (early network)
    assert_eq!(Network::Mainnet.grace_period_secs(), 2 * 60);
    assert_eq!(Network::Testnet.grace_period_secs(), 2 * 60);
    // Devnet: 30 seconds for fast testing
    assert_eq!(Network::Devnet.grace_period_secs(), 30);
}

#[test]
fn test_min_voting_age_by_network() {
    // Mainnet and Testnet: 30 days
    assert_eq!(Network::Mainnet.min_voting_age_secs(), 30 * 24 * 3600);
    assert_eq!(Network::Testnet.min_voting_age_secs(), 30 * 24 * 3600);
    // Devnet: 1 minute for fast testing
    assert_eq!(Network::Devnet.min_voting_age_secs(), 60);
}

#[test]
fn test_min_voting_age_blocks() {
    // Mainnet: 30 days / 10s per slot = 259,200 blocks
    assert_eq!(Network::Mainnet.min_voting_age_blocks(), 259_200);
    // Testnet: same as mainnet
    assert_eq!(Network::Testnet.min_voting_age_blocks(), 259_200);
    // Devnet: 60s / 10s per slot = 6 blocks
    assert_eq!(Network::Devnet.min_voting_age_blocks(), 6);
}

#[test]
fn test_update_check_interval_by_network() {
    // Mainnet and Testnet: 10 minutes (early network)
    assert_eq!(Network::Mainnet.update_check_interval_secs(), 10 * 60);
    assert_eq!(Network::Testnet.update_check_interval_secs(), 10 * 60);
    // Devnet: 10 seconds for fast testing
    assert_eq!(Network::Devnet.update_check_interval_secs(), 10);
}

#[test]
fn test_crash_window_by_network() {
    // Mainnet and Testnet: 1 hour
    assert_eq!(Network::Mainnet.crash_window_secs(), 3600);
    assert_eq!(Network::Testnet.crash_window_secs(), 3600);
    // Devnet: 1 minute for fast testing
    assert_eq!(Network::Devnet.crash_window_secs(), 60);
}

#[test]
fn test_crash_threshold_same_for_all() {
    // Crash threshold is 3 for all networks
    assert_eq!(Network::Mainnet.crash_threshold(), 3);
    assert_eq!(Network::Testnet.crash_threshold(), 3);
    assert_eq!(Network::Devnet.crash_threshold(), 3);
}

#[test]
fn test_seniority_maturity_blocks() {
    // Mainnet: 4 years × ~3.15M blocks/year = ~12.6M blocks
    assert_eq!(
        Network::Mainnet.seniority_maturity_blocks(),
        Network::Mainnet.blocks_per_year() * 4
    );
    assert_eq!(Network::Mainnet.seniority_maturity_blocks(), 12_614_400);

    // Testnet: same as mainnet
    assert_eq!(
        Network::Testnet.seniority_maturity_blocks(),
        Network::Testnet.blocks_per_year() * 4
    );

    // Devnet: 4 × 144 blocks = 576 blocks (~96 minutes with 10s slots)
    assert_eq!(Network::Devnet.seniority_maturity_blocks(), 576);
}

#[test]
fn test_seniority_step_blocks() {
    // Mainnet: 1 year = ~3.15M blocks
    assert_eq!(
        Network::Mainnet.seniority_step_blocks(),
        Network::Mainnet.blocks_per_year()
    );
    assert_eq!(Network::Mainnet.seniority_step_blocks(), 3_153_600);

    // Devnet: 1 year = 144 blocks (~24 minutes with 10s slots)
    assert_eq!(Network::Devnet.seniority_step_blocks(), 144);
}

#[test]
fn test_devnet_update_timing_acceleration() {
    let devnet = Network::Devnet;

    // Full update cycle in devnet:
    // veto (60s) + grace (30s) = 90 seconds total
    let full_cycle = devnet.veto_period_secs() + devnet.grace_period_secs();
    assert_eq!(full_cycle, 90);

    // Compare to mainnet:
    // veto (2 epochs = 7200s) + grace (1 epoch = 3600s) = 10800s total
    let mainnet = Network::Mainnet;
    let mainnet_cycle = mainnet.veto_period_secs() + mainnet.grace_period_secs();
    assert_eq!(mainnet_cycle, 420); // 7 minutes (5m veto + 2m grace)

    // Devnet is ~4.7x faster (7m vs 90s)
    let acceleration = mainnet_cycle / full_cycle;
    assert!(acceleration >= 4);
}

#[test]
fn test_seniority_weight_calculation_example() {
    // Example: Calculate vote weight for producer at different ages
    let devnet = Network::Devnet;
    let step = devnet.seniority_step_blocks(); // 144 blocks

    // weight = 1.0 + min(years, 4) * 0.75
    // 0 years (0-143 blocks): 1.00x
    // 1 year  (144-287 blocks): 1.75x
    // 2 years (288-431 blocks): 2.50x
    // 3 years (432-575 blocks): 3.25x
    // 4 years (576+ blocks): 4.00x

    // Helper to calculate weight
    let calc_weight = |blocks_active: u64| -> f64 {
        let years = blocks_active as f64 / step as f64;
        let capped_years = years.min(4.0);
        1.0 + capped_years * 0.75
    };

    // Test at various ages
    assert!((calc_weight(0) - 1.00).abs() < 0.01);
    assert!((calc_weight(step) - 1.75).abs() < 0.01);
    assert!((calc_weight(step * 2) - 2.50).abs() < 0.01);
    assert!((calc_weight(step * 3) - 3.25).abs() < 0.01);
    assert!((calc_weight(step * 4) - 4.00).abs() < 0.01);
    assert!((calc_weight(step * 10) - 4.00).abs() < 0.01); // Capped at 4x
}
