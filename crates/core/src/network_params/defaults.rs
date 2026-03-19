//! Hardcoded default parameters for each network
//!
//! These match the original values in `consensus.rs` (the DNA).
//! Mainnet values are immutable; testnet/devnet may be overridden via env.

use crate::Network;

use super::NetworkParams;

impl NetworkParams {
    /// Get hardcoded default parameters for a network
    ///
    /// These match the original hardcoded values in consensus.rs (the DNA)
    pub fn defaults(network: Network) -> NetworkParams {
        use crate::consensus;

        match network {
            Network::Mainnet => NetworkParams {
                // Networking
                default_p2p_port: 30300,
                default_rpc_port: 8500,
                default_metrics_port: 9000,
                bootstrap_nodes: vec![
                    "/dns4/seed1.doli.network/tcp/30300".to_string(),
                    "/dns4/seed2.doli.network/tcp/30300".to_string(),
                    "/dns4/seeds.doli.network/tcp/30300".to_string(),
                ],
                max_peers: 50, // Mainnet: conservative, tiered architecture handles scale

                // Timing
                slot_duration: consensus::SLOT_DURATION,
                genesis_time: consensus::GENESIS_TIME,
                veto_period_secs: 5 * 60, // 5 minutes (early network, small maintainer set)
                grace_period_secs: 2 * 60, // 2 minutes
                bootstrap_grace_period_secs: consensus::BOOTSTRAP_GRACE_PERIOD_SECS,
                unbonding_period: consensus::UNBONDING_PERIOD, // blocks (already u64)
                inactivity_threshold: u64::from(consensus::INACTIVITY_THRESHOLD),

                // Economics
                bond_unit: consensus::BOND_UNIT,
                initial_reward: consensus::INITIAL_REWARD,
                registration_base_fee: 100_000,      // 0.001 DOLI
                max_registration_fee: 1_000_000_000, // 10 DOLI
                automatic_genesis_bond: consensus::BOND_UNIT,
                genesis_blocks: 360, // 1 hour (open registration period)

                // VDF (800K iterations ~= 55ms for 2s sequential fallback windows)
                vdf_iterations: 800_000,
                heartbeat_vdf_iterations: 800_000,
                vdf_register_iterations: 5_000_000, // Fixed ~30s, no escalation

                // Time structure
                blocks_per_year: consensus::SLOTS_PER_YEAR as u64,
                blocks_per_reward_epoch: consensus::BLOCKS_PER_REWARD_EPOCH,
                coinbase_maturity: consensus::COINBASE_MATURITY,
                slots_per_reward_epoch: consensus::SLOTS_PER_REWARD_EPOCH,
                bootstrap_blocks: consensus::BOOTSTRAP_BLOCKS,

                // Update system
                min_voting_age_secs: 30 * 24 * 3600, // 30 days
                update_check_interval_secs: 10 * 60, // 10 minutes (early network)
                crash_window_secs: 3600,             // 1 hour
                max_registrations_per_block: 5,

                // Presence (telemetry)
                presence_window_ms: consensus::NETWORK_MARGIN_MS, // Use consensus margin

                // Fallback timing (locked for mainnet)
                fallback_timeout_ms: consensus::FALLBACK_TIMEOUT_MS,
                max_fallback_ranks: consensus::MAX_FALLBACK_RANKS,
                network_margin_ms: consensus::NETWORK_MARGIN_MS,

                // Vesting (locked for mainnet — consensus critical)
                vesting_quarter_slots: consensus::VESTING_QUARTER_SLOTS as u64,

                // Gossip mesh: universal config for all network sizes.
                // mesh_n=12 keeps all peers in eager-push for networks ≤24 (mesh_n_high),
                // and scales to 1000+ nodes at ~3-4 hops with 10s slot margin.
                // Ethereum runs D=8 for 800K validators — 12 is conservative.
                mesh_n: 12,
                mesh_n_low: 8,
                mesh_n_high: 24,
                gossip_lazy: 12,
            },

            Network::Testnet => NetworkParams {
                // Networking
                default_p2p_port: 40300,
                default_rpc_port: 18500,
                default_metrics_port: 19000,
                bootstrap_nodes: vec![
                    "/dns4/bootstrap1.testnet.doli.network/tcp/40300".to_string(),
                    "/dns4/bootstrap2.testnet.doli.network/tcp/40300".to_string(),
                    "/dns4/seeds.testnet.doli.network/tcp/40300".to_string(),
                ],
                max_peers: 100, // Testnet: higher for stress testing with 100+ nodes

                // Timing
                slot_duration: consensus::SLOT_DURATION,
                genesis_time: 1773950222, // Testnet v47 genesis - Full chain reset 2026-03-19
                veto_period_secs: 5 * 60, // 5 minutes (early network)
                grace_period_secs: 2 * 60, // 2 minutes
                bootstrap_grace_period_secs: consensus::BOOTSTRAP_GRACE_PERIOD_SECS,
                unbonding_period: 720, // 2 epochs (2 × 360 blocks) — fast withdrawal for testing
                inactivity_threshold: u64::from(consensus::INACTIVITY_THRESHOLD),

                // Economics (lower bond for testnet)
                bond_unit: 1_000_000, // 0.01 DOLI (testnet-friendly)
                initial_reward: consensus::INITIAL_REWARD,
                registration_base_fee: 100_000,
                max_registration_fee: 1_000_000_000,
                automatic_genesis_bond: 1_000_000, // 0.01 DOLI (matches testnet bond_unit)
                genesis_blocks: 360,               // 1 hour

                // VDF (800K iterations ~= 55ms, same as mainnet)
                vdf_iterations: 800_000,
                heartbeat_vdf_iterations: 800_000,
                vdf_register_iterations: 5_000_000, // Fixed ~30s, same as mainnet

                // Time structure (same as mainnet)
                blocks_per_year: consensus::SLOTS_PER_YEAR as u64,
                blocks_per_reward_epoch: consensus::BLOCKS_PER_REWARD_EPOCH,
                coinbase_maturity: consensus::COINBASE_MATURITY,
                slots_per_reward_epoch: consensus::SLOTS_PER_REWARD_EPOCH, // 1 hour for faster testing
                bootstrap_blocks: consensus::BOOTSTRAP_BLOCKS,

                // Update system
                min_voting_age_secs: 30 * 24 * 3600,
                update_check_interval_secs: 10 * 60, // 10 minutes (early network)
                crash_window_secs: 3600,
                max_registrations_per_block: 5,

                // Presence (telemetry)
                presence_window_ms: consensus::NETWORK_MARGIN_MS,

                // Fallback timing (same as mainnet)
                fallback_timeout_ms: consensus::FALLBACK_TIMEOUT_MS,
                max_fallback_ranks: consensus::MAX_FALLBACK_RANKS,
                network_margin_ms: consensus::NETWORK_MARGIN_MS,

                // Vesting (1-day: 6h quarters — faster than mainnet for testing)
                vesting_quarter_slots: 2_160,

                // Gossip mesh: same universal config as mainnet.
                mesh_n: 12,
                mesh_n_low: 8,
                mesh_n_high: 24,
                gossip_lazy: 12,
            },

            Network::Devnet => NetworkParams {
                // Networking
                default_p2p_port: 50300,
                default_rpc_port: 28500,
                default_metrics_port: 29000,
                bootstrap_nodes: vec![], // No bootstrap for local devnet
                max_peers: 150,          // Devnet: local machine, 100+ nodes stress tests

                // Timing (accelerated for testing)
                slot_duration: consensus::SLOT_DURATION, // Same as mainnet for realistic testing
                genesis_time: 0,                         // Dynamic
                veto_period_secs: 60,                    // 1 minute
                grace_period_secs: 30,                   // 30 seconds
                bootstrap_grace_period_secs: 5,          // 5s for fast devnet startup
                unbonding_period: 60,                    // ~10 minutes with 10s slots
                inactivity_threshold: 30,

                // Economics (lower values for testing)
                bond_unit: 100_000_000,           // 1 DOLI (Devnet override)
                initial_reward: 2_000_000_000,    // 20 DOLI (Devnet override)
                registration_base_fee: 1_000,     // 0.00001 DOLI
                max_registration_fee: 10_000_000, // 0.1 DOLI
                automatic_genesis_bond: 100_000_000, // 1 DOLI (matches devnet bond_unit)
                genesis_blocks: 40,

                // VDF (fast for development)
                vdf_iterations: 1,                  // Single iteration
                heartbeat_vdf_iterations: 800_000,  // 800K ~= 55ms
                vdf_register_iterations: 5_000_000, // ~5 seconds

                // Time structure (accelerated)
                blocks_per_year: 144,       // ~24 minutes
                blocks_per_reward_epoch: 4, // ~40 seconds
                coinbase_maturity: 10,
                slots_per_reward_epoch: 30, // 30 seconds
                bootstrap_blocks: 60,

                // Update system (fast for testing)
                min_voting_age_secs: 60,         // 1 minute
                update_check_interval_secs: 10,  // 10 seconds
                crash_window_secs: 60,           // 1 minute
                max_registrations_per_block: 20, // Higher for rapid testing

                // Presence (telemetry)
                presence_window_ms: consensus::NETWORK_MARGIN_MS,

                // Fallback timing (configurable for devnet)
                fallback_timeout_ms: consensus::FALLBACK_TIMEOUT_MS,
                max_fallback_ranks: consensus::MAX_FALLBACK_RANKS,
                network_margin_ms: consensus::NETWORK_MARGIN_MS,

                // Vesting (fast for devnet testing: 10 min per quarter, 40 min full vest)
                vesting_quarter_slots: 60,

                // Gossip mesh: same universal config as mainnet.
                // With --no-dht, mesh_n_high=24 keeps all devnet peers in mesh.
                mesh_n: 12,
                mesh_n_low: 8,
                mesh_n_high: 24,
                gossip_lazy: 12,
            },
        }
    }
}
