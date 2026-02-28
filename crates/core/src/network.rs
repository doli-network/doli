//! Network definitions for DOLI
//!
//! Defines the different networks (mainnet, testnet, devnet) with their
//! unique identifiers, parameters, and security boundaries.
//!
//! ## Environment Configuration
//!
//! Network parameters can be configured via environment variables or `.env` files
//! in the data directory (`~/.doli/{network}/.env`). See [`crate::network_params`]
//! for the full list of configurable parameters.
//!
//! Security-critical parameters (VDF, emission, timing) are locked for mainnet.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use std::sync::OnceLock;

use vdf::VdfParams;

use crate::network_params::NetworkParams;

/// Cached VDF parameters for each network (generated once, reused forever)
static MAINNET_VDF_PARAMS: OnceLock<VdfParams> = OnceLock::new();
static TESTNET_VDF_PARAMS: OnceLock<VdfParams> = OnceLock::new();
static DEVNET_VDF_PARAMS: OnceLock<VdfParams> = OnceLock::new();

/// Network identifier
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u32)]
pub enum Network {
    /// Production network - real money
    #[default]
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

    /// Get the network parameters (loaded from environment)
    ///
    /// Parameters are loaded once from environment variables and cached.
    /// For mainnet, security-critical parameters are locked to hardcoded values.
    pub fn params(&self) -> &'static NetworkParams {
        NetworkParams::load(*self)
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
    ///
    /// Configurable via `DOLI_P2P_PORT` environment variable.
    pub fn default_p2p_port(&self) -> u16 {
        self.params().default_p2p_port
    }

    /// Get default RPC port for this network
    ///
    /// Configurable via `DOLI_RPC_PORT` environment variable.
    pub fn default_rpc_port(&self) -> u16 {
        self.params().default_rpc_port
    }

    /// Get default metrics port for this network
    ///
    /// Configurable via `DOLI_METRICS_PORT` environment variable.
    pub fn default_metrics_port(&self) -> u16 {
        self.params().default_metrics_port
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
    ///
    /// Configurable via `DOLI_GENESIS_TIME` environment variable (devnet only).
    /// Locked for mainnet/testnet to ensure consensus compatibility.
    pub fn genesis_time(&self) -> u64 {
        self.params().genesis_time
    }

    /// Get initial bond amount for this network
    ///
    /// Initial bond = 1 bond unit = minimum to become a producer.
    /// For mainnet/testnet: 100 DOLI (1 slot per cycle)
    pub fn initial_bond(&self) -> u64 {
        self.bond_unit() // Initial bond equals 1 bond unit
    }

    /// Get bond unit size for slot allocation
    ///
    /// Each bond unit grants 1 consecutive slot per cycle.
    /// - 100 DOLI = 1 bond unit = 1 slot per cycle
    /// - 1,000 DOLI = 10 bond units = 10 slots per cycle
    ///
    /// Configurable via `DOLI_BOND_UNIT` environment variable (devnet only).
    /// Locked for mainnet to ensure consensus compatibility.
    pub fn bond_unit(&self) -> u64 {
        self.params().bond_unit
    }

    /// Get initial block reward for this network
    ///
    /// Configurable via `DOLI_INITIAL_REWARD` environment variable (devnet only).
    /// Locked for mainnet to ensure emission schedule compatibility.
    pub fn initial_reward(&self) -> u64 {
        self.params().initial_reward
    }

    /// Get genesis phase duration in blocks
    ///
    /// During genesis, producers can produce blocks without a bond.
    /// After genesis ends, each participating producer is automatically
    /// registered with an automatic_genesis_bond().
    ///
    /// Configurable via `DOLI_GENESIS_BLOCKS` environment variable (devnet only).
    pub fn genesis_blocks(&self) -> u64 {
        self.params().genesis_blocks
    }

    /// Check if the given height is within the genesis phase
    pub fn is_in_genesis(&self, height: u64) -> bool {
        let genesis_blocks = self.genesis_blocks();
        genesis_blocks > 0 && height <= genesis_blocks
    }

    /// Get the automatic bond amount for genesis producers
    ///
    /// After genesis ends, each producer who participated gets this amount
    /// automatically bonded (deducted from their earned rewards).
    /// This equals 1 bond unit (100 DOLI on mainnet/testnet, 100 DOLI on devnet).
    ///
    /// Configurable via `DOLI_AUTOMATIC_GENESIS_BOND` environment variable (devnet only).
    pub fn automatic_genesis_bond(&self) -> u64 {
        self.params().automatic_genesis_bond
    }

    /// Get coinbase maturity (blocks until coinbase rewards can be spent)
    ///
    /// Like Bitcoin, coinbase rewards must wait for confirmations before spending.
    /// Devnet uses shorter maturity for faster testing.
    ///
    /// Configurable via `DOLI_COINBASE_MATURITY` environment variable (devnet only).
    pub fn coinbase_maturity(&self) -> u64 {
        self.params().coinbase_maturity
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
    ///
    /// Configurable via `DOLI_VDF_ITERATIONS` environment variable (devnet only).
    /// Locked for mainnet to ensure consensus compatibility.
    pub fn vdf_iterations(&self) -> u64 {
        self.params().vdf_iterations
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
    ///
    /// Configurable via `DOLI_VETO_PERIOD_SECS` environment variable.
    pub fn veto_period_secs(&self) -> u64 {
        self.params().veto_period_secs
    }

    /// Get grace period after update approval before enforcement (in seconds)
    ///
    /// After the veto period ends and an update is approved, producers have
    /// this grace period to apply the update before version enforcement begins.
    ///
    /// Configurable via `DOLI_GRACE_PERIOD_SECS` environment variable.
    pub fn grace_period_secs(&self) -> u64 {
        self.params().grace_period_secs
    }

    /// Get minimum producer age before voting is allowed (in seconds)
    ///
    /// Producers must be registered for at least this long before they can
    /// vote on updates. This prevents flash Sybil attacks where an attacker
    /// registers many producers just before a vote.
    ///
    /// Configurable via `DOLI_MIN_VOTING_AGE_SECS` environment variable.
    pub fn min_voting_age_secs(&self) -> u64 {
        self.params().min_voting_age_secs
    }

    /// Get minimum producer age for voting in blocks
    ///
    /// Converts min_voting_age_secs to blocks using slot_duration.
    pub fn min_voting_age_blocks(&self) -> u64 {
        self.params().min_voting_age_blocks()
    }

    /// Get interval between automatic update checks (in seconds)
    ///
    /// Configurable via `DOLI_UPDATE_CHECK_INTERVAL_SECS` environment variable.
    pub fn update_check_interval_secs(&self) -> u64 {
        self.params().update_check_interval_secs
    }

    /// Get crash detection window for automatic rollback (in seconds)
    ///
    /// If the node crashes multiple times within this window after an update,
    /// the watchdog will automatically rollback to the previous version.
    ///
    /// Configurable via `DOLI_CRASH_WINDOW_SECS` environment variable.
    pub fn crash_window_secs(&self) -> u64 {
        self.params().crash_window_secs
    }

    /// Get number of crashes within crash_window that trigger automatic rollback
    pub fn crash_threshold(&self) -> u32 {
        // Same for all networks - 3 crashes triggers rollback
        3
    }

    /// Get blocks needed to reach full seniority (maximum vote weight)
    ///
    /// After this many blocks as a producer, the vote weight reaches 4x.
    /// Uses blocks_per_year * 4 for real time networks.
    pub fn seniority_maturity_blocks(&self) -> u64 {
        self.params().seniority_maturity_blocks()
    }

    /// Get blocks per seniority step (for vote weight calculation)
    ///
    /// Each step increases vote weight by 0.75x, up to 4 steps (4x total).
    /// - 0 steps (0-1 year): 1.00x
    /// - 1 step  (1-2 year): 1.75x
    /// - 2 steps (2-3 year): 2.50x
    /// - 3 steps (3-4 year): 3.25x
    /// - 4 steps (4+ years): 4.00x
    pub fn seniority_step_blocks(&self) -> u64 {
        self.params().seniority_step_blocks()
    }

    /// Get slot duration for this network (in seconds)
    ///
    /// Configurable via `DOLI_SLOT_DURATION` environment variable (devnet only).
    /// Locked for mainnet to ensure consensus compatibility.
    pub fn slot_duration(&self) -> u64 {
        self.params().slot_duration
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
    /// | Mainnet | 10s   | ~55ms      | Block VDF proof            |
    /// | Testnet | 10s   | ~55ms      | Block VDF proof            |
    /// | Devnet  | 1s    | ~55ms      | Fast development cycles    |
    pub fn vdf_target_time_ms(&self) -> u64 {
        match self {
            Network::Mainnet => 55, // ~55ms (800K iterations)
            Network::Testnet => 55, // ~55ms (800K iterations)
            Network::Devnet => 55,  // ~55ms (800K iterations)
        }
    }

    /// Get heartbeat VDF iterations for this network
    ///
    /// Hash-chain VDF iterations calibrated for target time:
    /// - 800K iterations ≈ 55ms on modern hardware
    ///
    /// Configurable via `DOLI_HEARTBEAT_VDF_ITERATIONS` environment variable (devnet only).
    /// Locked for mainnet to ensure consensus compatibility.
    pub fn heartbeat_vdf_iterations(&self) -> u64 {
        self.params().heartbeat_vdf_iterations
    }

    /// Get bootstrap blocks count
    ///
    /// Configurable via `DOLI_BOOTSTRAP_BLOCKS` environment variable (devnet only).
    pub fn bootstrap_blocks(&self) -> u64 {
        self.params().bootstrap_blocks
    }

    /// Get bootstrap grace period in seconds
    ///
    /// This is the wait time at genesis before allowing block production.
    /// Configurable via `DOLI_BOOTSTRAP_GRACE_PERIOD_SECS` environment variable (devnet only).
    /// Locked for mainnet (15 seconds).
    pub fn bootstrap_grace_period_secs(&self) -> u64 {
        self.params().bootstrap_grace_period_secs
    }

    /// Get slots per reward epoch for this network
    /// Reward epochs determine when accumulated rewards are distributed equally
    ///
    /// Configurable via `DOLI_SLOTS_PER_REWARD_EPOCH` environment variable (devnet only).
    pub fn slots_per_reward_epoch(&self) -> u32 {
        self.params().slots_per_reward_epoch
    }

    /// Get blocks per reward epoch for this network (block-height based epochs).
    ///
    /// This is the primary constant for the weighted presence reward system.
    /// Block-height based epochs are simpler than slot-based epochs because
    /// block heights are sequential with no gaps.
    ///
    /// Configurable via `DOLI_BLOCKS_PER_REWARD_EPOCH` environment variable (devnet only).
    /// Locked for mainnet to ensure consensus compatibility.
    pub fn blocks_per_reward_epoch(&self) -> u64 {
        self.params().blocks_per_reward_epoch
    }

    /// Get default bootstrap nodes for this network
    ///
    /// Configurable via `DOLI_BOOTSTRAP_NODES` environment variable (comma-separated).
    pub fn bootstrap_nodes(&self) -> Vec<String> {
        self.params().bootstrap_nodes.clone()
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
    ///
    /// Configurable via `DOLI_BLOCKS_PER_YEAR` environment variable (devnet only).
    /// Locked for mainnet to ensure consensus compatibility.
    pub fn blocks_per_year(&self) -> u64 {
        self.params().blocks_per_year
    }

    /// Blocks per "month" (simulated)
    pub fn blocks_per_month(&self) -> u64 {
        self.params().blocks_per_month()
    }

    /// Blocks per era (4 simulated "years")
    pub fn blocks_per_era(&self) -> u64 {
        self.params().blocks_per_era()
    }

    /// Commitment period (4 years) in blocks
    ///
    /// Producers who complete this period get their full bond back.
    pub fn commitment_period(&self) -> u64 {
        self.params().commitment_period()
    }

    /// Exit history retention period (8 years) in blocks
    ///
    /// After this period, exit records expire and producers can re-register
    /// without the prior_exit penalty.
    pub fn exit_history_retention(&self) -> u64 {
        self.params().exit_history_retention()
    }

    /// Inactivity threshold in blocks
    ///
    /// After this many blocks without activity, a producer incurs an
    /// activity gap penalty.
    ///
    /// Configurable via `DOLI_INACTIVITY_THRESHOLD` environment variable (devnet only).
    pub fn inactivity_threshold(&self) -> u64 {
        self.params().inactivity_threshold
    }

    /// Unbonding period in blocks
    ///
    /// After requesting exit, producers must wait this long before
    /// claiming their bond. (7 days)
    ///
    /// Configurable via `DOLI_UNBONDING_PERIOD` environment variable (devnet only).
    /// Locked for mainnet to ensure consensus compatibility.
    pub fn unbonding_period(&self) -> u64 {
        self.params().unbonding_period
    }

    /// Veto period for software updates in blocks
    pub fn veto_period_blocks(&self) -> u64 {
        self.params().veto_period_blocks()
    }

    /// Maximum registrations allowed per block
    ///
    /// Limits registration throughput to prevent spam attacks.
    ///
    /// Configurable via `DOLI_MAX_REGISTRATIONS_PER_BLOCK` environment variable.
    pub fn max_registrations_per_block(&self) -> u32 {
        self.params().max_registrations_per_block
    }

    /// Base registration fee
    ///
    /// This is multiplied by 1.5^pending_count for anti-DoS escalation.
    ///
    /// Configurable via `DOLI_REGISTRATION_BASE_FEE` environment variable.
    pub fn registration_base_fee(&self) -> u64 {
        self.params().registration_base_fee
    }

    /// VDF iterations for registration proof
    ///
    /// Configurable via `DOLI_VDF_REGISTER_ITERATIONS` environment variable (devnet only).
    /// Locked for mainnet to ensure anti-Sybil protection.
    pub fn vdf_register_iterations(&self) -> u64 {
        self.params().vdf_register_iterations
    }

    /// Maximum registration fee cap
    ///
    /// Prevents fees from becoming unreasonable during congestion.
    ///
    /// Configurable via `DOLI_MAX_REGISTRATION_FEE` environment variable.
    pub fn max_registration_fee(&self) -> u64 {
        self.params().max_registration_fee
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

        // Testnet has no genesis phase (pre-registered producers)
        assert_eq!(Network::Testnet.genesis_blocks(), 0);
        assert!(!Network::Testnet.is_in_genesis(0));
        assert!(!Network::Testnet.is_in_genesis(1));
    }

    #[test]
    fn test_automatic_genesis_bond() {
        // PRODUCTION: 10 DOLI, TEST: 1 DOLI
        assert_eq!(Network::Mainnet.automatic_genesis_bond(), 100_000_000);
        assert_eq!(Network::Testnet.automatic_genesis_bond(), 100_000_000);
        assert_eq!(Network::Devnet.automatic_genesis_bond(), 100_000_000);
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
        // Mainnet and Testnet: 7 days
        assert_eq!(Network::Mainnet.veto_period_secs(), 7 * 24 * 3600);
        assert_eq!(Network::Testnet.veto_period_secs(), 7 * 24 * 3600);
        // Devnet: 1 minute for fast testing
        assert_eq!(Network::Devnet.veto_period_secs(), 60);
    }

    #[test]
    fn test_grace_period_by_network() {
        // Mainnet and Testnet: 48 hours
        assert_eq!(Network::Mainnet.grace_period_secs(), 48 * 3600);
        assert_eq!(Network::Testnet.grace_period_secs(), 48 * 3600);
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
        // Mainnet and Testnet: 6 hours
        assert_eq!(Network::Mainnet.update_check_interval_secs(), 6 * 3600);
        assert_eq!(Network::Testnet.update_check_interval_secs(), 6 * 3600);
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
        // veto (7 days) + grace (48h) = 9 days total
        let mainnet = Network::Mainnet;
        let mainnet_cycle = mainnet.veto_period_secs() + mainnet.grace_period_secs();
        assert_eq!(mainnet_cycle, 9 * 24 * 3600); // 9 days in seconds

        // Devnet is ~8640x faster (9 days vs 90 seconds)
        let acceleration = mainnet_cycle / full_cycle;
        assert!(acceleration > 8000);
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
}
