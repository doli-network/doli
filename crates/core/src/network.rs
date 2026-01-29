//! Network definitions for DOLI
//!
//! Defines the different networks (mainnet, testnet, devnet) with their
//! unique identifiers, parameters, and security boundaries.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use std::sync::OnceLock;

use vdf::VdfParams;

/// Cached VDF parameters for each network (generated once, reused forever)
static MAINNET_VDF_PARAMS: OnceLock<VdfParams> = OnceLock::new();
static TESTNET_VDF_PARAMS: OnceLock<VdfParams> = OnceLock::new();
static DEVNET_VDF_PARAMS: OnceLock<VdfParams> = OnceLock::new();

/// Network identifier
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u32)]
pub enum Network {
    /// Production network - real money
    Mainnet = 1,
    /// Public test network - valueless coins for testing
    Testnet = 2,
    /// Local development network - fast blocks for development
    Devnet = 99,
}

impl Network {
    /// Get the numeric network ID
    pub fn id(&self) -> u32 {
        *self as u32
    }

    /// Get the network name
    pub fn name(&self) -> &'static str {
        match self {
            Network::Mainnet => "mainnet",
            Network::Testnet => "testnet",
            Network::Devnet => "devnet",
        }
    }

    /// Get the address prefix for this network
    /// Prevents sending coins to wrong network
    pub fn address_prefix(&self) -> &'static str {
        match self {
            Network::Mainnet => "doli",
            Network::Testnet => "tdoli",
            Network::Devnet => "ddoli",
        }
    }

    /// Get magic bytes for P2P protocol
    /// Used to identify network at protocol level
    pub fn magic_bytes(&self) -> [u8; 4] {
        match self {
            Network::Mainnet => [0xD0, 0x11, 0x00, 0x01], // "DOLI" mainnet
            Network::Testnet => [0xD0, 0x11, 0x00, 0x02], // "DOLI" testnet
            Network::Devnet => [0xD0, 0x11, 0x00, 0x63],  // "DOLI" devnet (0x63 = 99)
        }
    }

    /// Get default P2P port for this network
    pub fn default_p2p_port(&self) -> u16 {
        match self {
            Network::Mainnet => 30303,
            Network::Testnet => 40303,
            Network::Devnet => 50303,
        }
    }

    /// Get default RPC port for this network
    pub fn default_rpc_port(&self) -> u16 {
        match self {
            Network::Mainnet => 8545,
            Network::Testnet => 18545,
            Network::Devnet => 28545,
        }
    }

    /// Get default data directory suffix
    pub fn data_dir_name(&self) -> &'static str {
        match self {
            Network::Mainnet => "mainnet",
            Network::Testnet => "testnet",
            Network::Devnet => "devnet",
        }
    }

    /// Get genesis timestamp for this network
    pub fn genesis_time(&self) -> u64 {
        match self {
            // 2026-02-01T00:00:00Z
            Network::Mainnet => 1769904000,
            // 2025-06-01T00:00:00Z
            Network::Testnet => 1748736000,
            // Dynamic - use current time (will be set at runtime)
            Network::Devnet => 0,
        }
    }

    /// Get initial bond amount for this network
    pub fn initial_bond(&self) -> u64 {
        match self {
            Network::Mainnet => 100_000_000_000, // 1000 DOLI
            Network::Testnet => 100_000_000_000, // 1000 DOLI (same as mainnet)
            Network::Devnet => 100_000_000,      // 1 DOLI
        }
    }

    /// Get initial block reward for this network
    pub fn initial_reward(&self) -> u64 {
        match self {
            Network::Mainnet => 100_000_000, // 1 DOLI
            Network::Testnet => 100_000_000, // 1 DOLI
            Network::Devnet => 100_000_000,  // 1 DOLI
        }
    }

    /// Check if VDF is enabled for this network
    ///
    /// All networks use VDF (hash-chain based) for Proof of Time.
    /// Devnet uses faster parameters for testing.
    pub fn vdf_enabled(&self) -> bool {
        match self {
            Network::Mainnet => true,
            Network::Testnet => true,
            Network::Devnet => true, // Enabled with fast hash-chain VDF (~700ms)
        }
    }

    /// Get VDF iterations for block production
    ///
    /// These values are calibrated for practical block production times
    /// based on the network-specific discriminant size.
    ///
    /// Note: Check vdf_enabled() first - if false, VDF is skipped entirely.
    pub fn vdf_iterations(&self) -> u64 {
        match self {
            Network::Mainnet => 100_000, // ~10-100 seconds with 2048-bit discriminant
            Network::Testnet => 100_000, // Same as mainnet
            Network::Devnet => 1,        // Single iteration for fast development
        }
    }

    /// Get VDF discriminant size in bits for this network
    ///
    /// The discriminant size determines security vs. speed tradeoff:
    /// - Larger discriminants are more secure but slower
    /// - Smaller discriminants are faster but provide less security
    ///
    /// Mainnet and Testnet use production-grade security.
    /// Devnet uses minimal security for fast development.
    pub fn vdf_discriminant_bits(&self) -> usize {
        match self {
            Network::Mainnet => 2048, // Production security (~112-bit)
            Network::Testnet => 2048, // Same as mainnet (production security)
            Network::Devnet => 256,   // Minimal security, very fast
        }
    }

    /// Get VDF seed for deterministic discriminant generation
    ///
    /// Each network uses a unique seed to generate its discriminant,
    /// ensuring proofs from different networks are incompatible.
    pub fn vdf_seed(&self) -> &'static [u8] {
        match self {
            Network::Mainnet => b"DOLI_VDF_DISCRIMINANT_V1_MAINNET",
            Network::Testnet => b"DOLI_VDF_DISCRIMINANT_V1_TESTNET",
            Network::Devnet => b"DOLI_VDF_DISCRIMINANT_V1_DEVNET",
        }
    }

    /// Get cached VDF parameters for this network
    ///
    /// The VDF discriminant is expensive to generate (involves finding large primes),
    /// so we cache it once per network. All subsequent calls return the cached params.
    ///
    /// This is the recommended way to get VDF parameters for computation and verification.
    pub fn vdf_params(&self) -> &'static VdfParams {
        match self {
            Network::Mainnet => MAINNET_VDF_PARAMS.get_or_init(|| {
                VdfParams::with_seed(self.vdf_discriminant_bits(), self.vdf_seed())
            }),
            Network::Testnet => TESTNET_VDF_PARAMS.get_or_init(|| {
                VdfParams::with_seed(self.vdf_discriminant_bits(), self.vdf_seed())
            }),
            Network::Devnet => DEVNET_VDF_PARAMS.get_or_init(|| {
                VdfParams::with_seed(self.vdf_discriminant_bits(), self.vdf_seed())
            }),
        }
    }

    /// Get veto period for software updates (in seconds)
    pub fn veto_period_secs(&self) -> u64 {
        match self {
            Network::Mainnet => 7 * 24 * 3600, // 7 days
            Network::Testnet => 7 * 24 * 3600, // Same as mainnet (7 days)
            Network::Devnet => 60,             // 1 minute
        }
    }

    /// Get slot duration for this network (in seconds)
    pub fn slot_duration(&self) -> u64 {
        match self {
            Network::Mainnet => 10, // 10 seconds
            Network::Testnet => 10, // 10 seconds
            Network::Devnet => 1,   // 1 second (fast development)
        }
    }

    /// Get VDF target time for this network (in milliseconds)
    ///
    /// With Epoch Lookahead selection (deterministic round-robin), VDF only
    /// needs to prove presence (heartbeat), not prevent grinding.
    ///
    /// Grinding prevention comes from:
    /// 1. Leaders determined at epoch start (not per-slot)
    /// 2. Selection uses slot number + bond distribution, NOT prev_hash
    /// 3. Attackers cannot influence future leader selection
    ///
    /// | Network | Slot  | VDF Target | Purpose                    |
    /// |---------|-------|------------|----------------------------|
    /// | Mainnet | 10s   | ~700ms     | Heartbeat proof of presence|
    /// | Testnet | 10s   | ~700ms     | Heartbeat proof of presence|
    /// | Devnet  | 1s    | ~70ms      | Fast development cycles    |
    pub fn vdf_target_time_ms(&self) -> u64 {
        match self {
            Network::Mainnet => 700, // ~700ms
            Network::Testnet => 700, // ~700ms
            Network::Devnet => 70,   // ~70ms (fast development)
        }
    }

    /// Get heartbeat VDF iterations for this network
    ///
    /// Hash-chain VDF iterations calibrated for target time:
    /// - 10M iterations ≈ 700ms on modern hardware
    /// - 1M iterations ≈ 70ms on modern hardware
    ///
    /// Devnet uses fewer iterations for faster block production.
    pub fn heartbeat_vdf_iterations(&self) -> u64 {
        match self {
            Network::Mainnet => 10_000_000, // ~700ms
            Network::Testnet => 10_000_000, // Same as mainnet (~700ms)
            Network::Devnet => 1_000_000,   // ~70ms (fast development)
        }
    }

    /// Get bootstrap blocks count
    pub fn bootstrap_blocks(&self) -> u64 {
        match self {
            Network::Mainnet => 60_480, // ~1 week at 10s slots
            Network::Testnet => 60_480, // Same as mainnet (~1 week)
            Network::Devnet => 60,      // ~1 minute at 1s slots
        }
    }

    /// Get slots per reward epoch for this network
    /// Reward epochs determine when accumulated rewards are distributed equally
    pub fn slots_per_reward_epoch(&self) -> u32 {
        match self {
            Network::Mainnet => 8_640, // 1 day (86,400 seconds / 10s slots)
            Network::Testnet => 8_640, // Same as mainnet (1 day)
            Network::Devnet => 30,     // 30 seconds at 1s slots (fast testing)
        }
    }

    /// Get default bootstrap nodes for this network
    pub fn bootstrap_nodes(&self) -> Vec<&'static str> {
        match self {
            Network::Mainnet => vec![
                "/dns4/seed1.doli.network/tcp/30303",
                "/dns4/seed2.doli.network/tcp/30303",
            ],
            Network::Testnet => vec![
                "/ip4/198.51.100.1/tcp/40303", // omegacortex.ai
            ],
            Network::Devnet => vec![], // No bootstrap for local devnet
        }
    }

    /// Check if this is a test network (testnet or devnet)
    pub fn is_test(&self) -> bool {
        matches!(self, Network::Testnet | Network::Devnet)
    }

    // ==================== Time Acceleration Parameters ====================
    //
    // These parameters enable accelerated time simulation for testing:
    // - Mainnet: Real time (1 block/minute, 4 years per era)
    // - Testnet: 1 real day = 1 simulated year
    // - Devnet:  1 real minute = 1 simulated year (20 min test = 20 years)

    /// Blocks per "year" (simulated)
    ///
    /// - Mainnet: 3,153,600 blocks (~365.25 days at 6 blocks/minute)
    /// - Testnet: 3,153,600 blocks (same as mainnet)
    /// - Devnet:  144 blocks (2.4 minutes = 1 simulated year, era ≈ 10 minutes)
    pub fn blocks_per_year(&self) -> u64 {
        match self {
            Network::Mainnet => 3_153_600, // 365.25 days × 24h × 60min × 6 blocks/min
            Network::Testnet => 3_153_600, // Same as mainnet
            Network::Devnet => 144,        // 144 blocks × 1s = 2.4 min/year, 576 blocks/era ≈ 10 min
        }
    }

    /// Blocks per "month" (simulated)
    pub fn blocks_per_month(&self) -> u64 {
        self.blocks_per_year() / 12
    }

    /// Blocks per era (4 simulated "years")
    pub fn blocks_per_era(&self) -> u64 {
        self.blocks_per_year() * 4
    }

    /// Commitment period (4 years) in blocks
    ///
    /// Producers who complete this period get their full bond back.
    pub fn commitment_period(&self) -> u64 {
        self.blocks_per_era()
    }

    /// Exit history retention period (8 years) in blocks
    ///
    /// After this period, exit records expire and producers can re-register
    /// without the prior_exit penalty.
    pub fn exit_history_retention(&self) -> u64 {
        self.blocks_per_era() * 2
    }

    /// Inactivity threshold in blocks
    ///
    /// After this many blocks without activity, a producer incurs an
    /// activity gap penalty.
    pub fn inactivity_threshold(&self) -> u64 {
        match self {
            Network::Mainnet => 60_480, // ~1 week at 10s slots
            Network::Testnet => 60_480, // Same as mainnet (~1 week)
            Network::Devnet => 30,      // 30 seconds with 1s slots
        }
    }

    /// Unbonding period in blocks
    ///
    /// After requesting exit, producers must wait this long before
    /// claiming their bond.
    pub fn unbonding_period(&self) -> u64 {
        match self {
            Network::Mainnet => 259_200, // ~30 days at 10s slots
            Network::Testnet => 259_200, // Same as mainnet (~30 days)
            Network::Devnet => 60,       // ~1 minute with 1s slots
        }
    }

    /// Veto period for software updates in blocks
    pub fn veto_period_blocks(&self) -> u64 {
        match self {
            Network::Mainnet => 60_480, // 7 days at 10s slots
            Network::Testnet => 60_480, // Same as mainnet (7 days)
            Network::Devnet => 60,      // ~1 minute with 1s slots
        }
    }

    /// Maximum registrations allowed per block
    ///
    /// Limits registration throughput to prevent spam attacks.
    pub fn max_registrations_per_block(&self) -> u32 {
        match self {
            Network::Mainnet => 5,
            Network::Testnet => 5,  // Same as mainnet
            Network::Devnet => 20,  // Higher for rapid testing
        }
    }

    /// Base registration fee
    ///
    /// This is multiplied by 1.5^pending_count for anti-DoS escalation.
    pub fn registration_base_fee(&self) -> u64 {
        match self {
            Network::Mainnet => 100_000, // 0.001 DOLI
            Network::Testnet => 100_000, // Same as mainnet (0.001 DOLI)
            Network::Devnet => 1_000,    // 0.00001 DOLI (nearly free)
        }
    }

    /// VDF iterations for registration proof
    pub fn vdf_register_iterations(&self) -> u64 {
        match self {
            Network::Mainnet => 600_000_000, // ~10 minutes
            Network::Testnet => 600_000_000, // Same as mainnet (~10 minutes)
            Network::Devnet => 5_000_000,    // ~5 seconds
        }
    }

    /// Maximum registration fee cap
    ///
    /// Prevents fees from becoming unreasonable during congestion.
    pub fn max_registration_fee(&self) -> u64 {
        match self {
            Network::Mainnet => 1_000_000_000, // 10 DOLI
            Network::Testnet => 1_000_000_000, // Same as mainnet (10 DOLI)
            Network::Devnet => 10_000_000,     // 0.1 DOLI
        }
    }

    /// Get all available networks
    pub fn all() -> &'static [Network] {
        &[Network::Mainnet, Network::Testnet, Network::Devnet]
    }

    /// Parse from network ID
    pub fn from_id(id: u32) -> Option<Network> {
        match id {
            1 => Some(Network::Mainnet),
            2 => Some(Network::Testnet),
            99 => Some(Network::Devnet),
            _ => None,
        }
    }
}

impl fmt::Display for Network {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

impl FromStr for Network {
    type Err = NetworkParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "mainnet" | "main" => Ok(Network::Mainnet),
            "testnet" | "test" => Ok(Network::Testnet),
            "devnet" | "dev" | "local" => Ok(Network::Devnet),
            _ => Err(NetworkParseError(s.to_string())),
        }
    }
}

impl Default for Network {
    fn default() -> Self {
        Network::Mainnet
    }
}

/// Error when parsing network name
#[derive(Debug, Clone)]
pub struct NetworkParseError(String);

impl fmt::Display for NetworkParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown network '{}'. Valid options: mainnet, testnet, devnet",
            self.0
        )
    }
}

impl std::error::Error for NetworkParseError {}

#[cfg(test)]
mod tests {
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

        // 144 blocks = 1 simulated year (with 1s slots)
        assert_eq!(devnet.blocks_per_year(), 144);

        // 576 blocks = 1 era (4 simulated years) ≈ 9.6 minutes
        assert_eq!(devnet.blocks_per_era(), 576);

        // 1 block = 1 second
        assert_eq!(devnet.slot_duration(), 1);

        // 144 blocks × 1 second = 144 seconds = 2.4 minutes = 1 simulated year
        assert_eq!(devnet.blocks_per_year() * devnet.slot_duration(), 144);

        // 1 month = 12 blocks
        assert_eq!(devnet.blocks_per_month(), 12);

        // Commitment period = 4 years = 576 blocks ≈ 9.6 minutes
        assert_eq!(devnet.commitment_period(), 576);

        // Exit history retention = 8 years = 1152 blocks ≈ 19.2 minutes
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
        assert_eq!(testnet.vdf_discriminant_bits(), mainnet.vdf_discriminant_bits());
        assert_eq!(testnet.heartbeat_vdf_iterations(), mainnet.heartbeat_vdf_iterations());
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

        // Verify era duration: 576 blocks = 576 seconds ≈ 9.6 minutes
        assert_eq!(devnet.blocks_per_era(), 576);
        assert_eq!(
            devnet.blocks_per_era() * devnet.slot_duration(),
            576,
            "1 era should = 576 seconds ≈ 9.6 minutes"
        );

        // Verify 1 hour ≈ 6.25 eras (25 simulated years)
        let one_hour_blocks = 3600 / devnet.slot_duration();
        let eras = one_hour_blocks / devnet.blocks_per_era();
        assert_eq!(eras, 6, "1 hour should ≈ 6 eras (6.25 rounded down)");

        // Verify inactivity threshold is quick for testing
        assert_eq!(devnet.inactivity_threshold(), 30); // 30 seconds with 1s slots

        // Verify unbonding is quick for testing
        assert_eq!(devnet.unbonding_period(), 60); // 60 seconds with 1s slots
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
    fn test_vdf_iterations_scale_by_network() {
        // Mainnet and Testnet have same iterations
        assert_eq!(
            Network::Mainnet.vdf_register_iterations(),
            Network::Testnet.vdf_register_iterations()
        );
        assert!(
            Network::Testnet.vdf_register_iterations() > Network::Devnet.vdf_register_iterations()
        );

        // Devnet should be fast enough for testing
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
}
