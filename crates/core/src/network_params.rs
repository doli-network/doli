//! Network parameters loaded from environment variables
//!
//! This module provides configurable network parameters that can be overridden
//! via `.env` files in the data directory (`~/.doli/{network}/.env`).
//!
//! Security-critical parameters are locked for mainnet to prevent accidental
//! or malicious modification of consensus rules.
//!
//! ## Usage
//!
//! ```ignore
//! use doli_core::network_params::{load_env_for_network, NetworkParams};
//! use doli_core::Network;
//! use std::path::PathBuf;
//!
//! // Load .env file into process environment
//! let data_dir = PathBuf::from("/home/user/.doli/devnet");
//! load_env_for_network("devnet", &data_dir);
//!
//! // Now NetworkParams will read from the loaded environment
//! let params = NetworkParams::load(Network::Devnet);
//! ```

use std::path::Path;
use std::sync::OnceLock;

use tracing::{debug, info, warn};

use crate::Network;

/// Cached network parameters (one per network)
static MAINNET_PARAMS: OnceLock<NetworkParams> = OnceLock::new();
static TESTNET_PARAMS: OnceLock<NetworkParams> = OnceLock::new();
static DEVNET_PARAMS: OnceLock<NetworkParams> = OnceLock::new();

/// Configurable network parameters
///
/// These parameters can be loaded from environment variables or `.env` files.
/// Default values match the hardcoded constants for backward compatibility.
#[derive(Debug, Clone)]
pub struct NetworkParams {
    // === Networking ===
    /// Default P2P port for this network
    pub default_p2p_port: u16,
    /// Default RPC port for this network
    pub default_rpc_port: u16,
    /// Default metrics port for this network
    pub default_metrics_port: u16,
    /// Bootstrap nodes (multiaddr format)
    pub bootstrap_nodes: Vec<String>,

    // === Timing ===
    /// Slot duration in seconds
    pub slot_duration: u64,
    /// Genesis timestamp (Unix timestamp)
    pub genesis_time: u64,
    /// Veto period for updates in seconds
    pub veto_period_secs: u64,
    /// Grace period after update approval in seconds
    pub grace_period_secs: u64,
    /// Bootstrap grace period in seconds (wait at genesis for chain evidence)
    pub bootstrap_grace_period_secs: u64,
    /// Unbonding period in blocks
    pub unbonding_period: u64,
    /// Inactivity threshold in blocks
    pub inactivity_threshold: u64,

    // === Economics ===
    /// Bond unit size (minimum bond = 1 unit)
    pub bond_unit: u64,
    /// Initial block reward in base units
    pub initial_reward: u64,
    /// Base registration fee in base units
    pub registration_base_fee: u64,
    /// Maximum registration fee cap
    pub max_registration_fee: u64,
    /// Automatic genesis bond amount
    pub automatic_genesis_bond: u64,
    /// Genesis phase duration in blocks
    pub genesis_blocks: u64,

    // === VDF (locked for mainnet) ===
    /// VDF iterations for block production
    pub vdf_iterations: u64,
    /// Heartbeat VDF iterations
    pub heartbeat_vdf_iterations: u64,
    /// VDF iterations for registration proof
    pub vdf_register_iterations: u64,

    // === Time structure ===
    /// Blocks per simulated "year"
    pub blocks_per_year: u64,
    /// Blocks per reward epoch
    pub blocks_per_reward_epoch: u64,
    /// Coinbase maturity (blocks until spendable)
    pub coinbase_maturity: u64,
    /// Slots per reward epoch (legacy)
    pub slots_per_reward_epoch: u32,
    /// Bootstrap blocks count
    pub bootstrap_blocks: u64,

    // === Update system ===
    /// Minimum voting age in seconds
    pub min_voting_age_secs: u64,
    /// Update check interval in seconds
    pub update_check_interval_secs: u64,
    /// Crash window for automatic rollback in seconds
    pub crash_window_secs: u64,
    /// Maximum registrations per block
    pub max_registrations_per_block: u32,

    // === Presence (telemetry) ===
    /// Presence window duration in milliseconds (for telemetry only, does not affect consensus)
    pub presence_window_ms: u64,

    // === Gossip mesh ===
    /// Target number of peers in gossipsub mesh per topic
    pub mesh_n: usize,
    /// Minimum peers in gossipsub mesh before requesting more
    pub mesh_n_low: usize,
    /// Maximum peers in gossipsub mesh before pruning
    pub mesh_n_high: usize,
    /// Number of peers to lazily gossip IHAVE messages to
    pub gossip_lazy: usize,
}

impl NetworkParams {
    /// Load network parameters from environment variables
    ///
    /// Parameters are loaded from:
    /// 1. Process environment variables
    /// 2. `.env` file in the data directory (if it exists)
    ///
    /// Missing parameters fall back to network defaults.
    pub fn load(network: Network) -> &'static NetworkParams {
        let lock = match network {
            Network::Mainnet => &MAINNET_PARAMS,
            Network::Testnet => &TESTNET_PARAMS,
            Network::Devnet => &DEVNET_PARAMS,
        };

        lock.get_or_init(|| Self::load_from_env(network))
    }

    /// Initialize parameters from environment (called once per network)
    fn load_from_env(network: Network) -> NetworkParams {
        let defaults = Self::defaults(network);

        // For mainnet, enforce locked parameters
        let is_mainnet = matches!(network, Network::Mainnet);

        NetworkParams {
            // Networking (configurable for all networks)
            default_p2p_port: env_parse("DOLI_P2P_PORT", defaults.default_p2p_port),
            default_rpc_port: env_parse("DOLI_RPC_PORT", defaults.default_rpc_port),
            default_metrics_port: env_parse("DOLI_METRICS_PORT", defaults.default_metrics_port),
            bootstrap_nodes: env_parse_vec(
                "DOLI_BOOTSTRAP_NODES",
                defaults.bootstrap_nodes.clone(),
            ),

            // Timing (some locked for mainnet)
            slot_duration: if is_mainnet {
                defaults.slot_duration // LOCKED for mainnet
            } else {
                env_parse("DOLI_SLOT_DURATION", defaults.slot_duration)
            },
            genesis_time: if is_mainnet {
                defaults.genesis_time // LOCKED for mainnet
            } else {
                env_parse("DOLI_GENESIS_TIME", defaults.genesis_time)
            },
            veto_period_secs: env_parse("DOLI_VETO_PERIOD_SECS", defaults.veto_period_secs),
            grace_period_secs: env_parse("DOLI_GRACE_PERIOD_SECS", defaults.grace_period_secs),
            bootstrap_grace_period_secs: if is_mainnet {
                defaults.bootstrap_grace_period_secs // LOCKED for mainnet
            } else {
                env_parse(
                    "DOLI_BOOTSTRAP_GRACE_PERIOD_SECS",
                    defaults.bootstrap_grace_period_secs,
                )
            },
            unbonding_period: if is_mainnet {
                defaults.unbonding_period // LOCKED for mainnet
            } else {
                env_parse("DOLI_UNBONDING_PERIOD", defaults.unbonding_period)
            },
            inactivity_threshold: if is_mainnet {
                defaults.inactivity_threshold
            } else {
                env_parse("DOLI_INACTIVITY_THRESHOLD", defaults.inactivity_threshold)
            },

            // Economics (some locked for mainnet)
            bond_unit: if is_mainnet {
                defaults.bond_unit // LOCKED for mainnet
            } else {
                env_parse("DOLI_BOND_UNIT", defaults.bond_unit)
            },
            initial_reward: if is_mainnet {
                defaults.initial_reward // LOCKED for mainnet
            } else {
                env_parse("DOLI_INITIAL_REWARD", defaults.initial_reward)
            },
            registration_base_fee: env_parse(
                "DOLI_REGISTRATION_BASE_FEE",
                defaults.registration_base_fee,
            ),
            max_registration_fee: env_parse(
                "DOLI_MAX_REGISTRATION_FEE",
                defaults.max_registration_fee,
            ),
            automatic_genesis_bond: if is_mainnet {
                defaults.automatic_genesis_bond // LOCKED for mainnet
            } else {
                env_parse(
                    "DOLI_AUTOMATIC_GENESIS_BOND",
                    defaults.automatic_genesis_bond,
                )
            },
            genesis_blocks: if is_mainnet {
                defaults.genesis_blocks // LOCKED for mainnet
            } else {
                env_parse("DOLI_GENESIS_BLOCKS", defaults.genesis_blocks)
            },

            // VDF (LOCKED for mainnet - security critical)
            vdf_iterations: if is_mainnet {
                defaults.vdf_iterations // LOCKED for mainnet
            } else {
                env_parse("DOLI_VDF_ITERATIONS", defaults.vdf_iterations)
            },
            heartbeat_vdf_iterations: if is_mainnet {
                defaults.heartbeat_vdf_iterations // LOCKED for mainnet
            } else {
                env_parse(
                    "DOLI_HEARTBEAT_VDF_ITERATIONS",
                    defaults.heartbeat_vdf_iterations,
                )
            },
            vdf_register_iterations: if is_mainnet {
                defaults.vdf_register_iterations // LOCKED for mainnet
            } else {
                env_parse(
                    "DOLI_VDF_REGISTER_ITERATIONS",
                    defaults.vdf_register_iterations,
                )
            },

            // Time structure (some locked for mainnet)
            blocks_per_year: if is_mainnet {
                defaults.blocks_per_year // LOCKED for mainnet
            } else {
                env_parse("DOLI_BLOCKS_PER_YEAR", defaults.blocks_per_year)
            },
            blocks_per_reward_epoch: if is_mainnet {
                defaults.blocks_per_reward_epoch // LOCKED for mainnet
            } else {
                env_parse(
                    "DOLI_BLOCKS_PER_REWARD_EPOCH",
                    defaults.blocks_per_reward_epoch,
                )
            },
            coinbase_maturity: if is_mainnet {
                defaults.coinbase_maturity // LOCKED for mainnet
            } else {
                env_parse("DOLI_COINBASE_MATURITY", defaults.coinbase_maturity)
            },
            slots_per_reward_epoch: if is_mainnet {
                defaults.slots_per_reward_epoch // LOCKED for mainnet
            } else {
                env_parse(
                    "DOLI_SLOTS_PER_REWARD_EPOCH",
                    defaults.slots_per_reward_epoch,
                )
            },
            bootstrap_blocks: if is_mainnet {
                defaults.bootstrap_blocks
            } else {
                env_parse("DOLI_BOOTSTRAP_BLOCKS", defaults.bootstrap_blocks)
            },

            // Update system (configurable for all networks)
            min_voting_age_secs: env_parse(
                "DOLI_MIN_VOTING_AGE_SECS",
                defaults.min_voting_age_secs,
            ),
            update_check_interval_secs: env_parse(
                "DOLI_UPDATE_CHECK_INTERVAL_SECS",
                defaults.update_check_interval_secs,
            ),
            crash_window_secs: env_parse("DOLI_CRASH_WINDOW_SECS", defaults.crash_window_secs),
            max_registrations_per_block: env_parse(
                "DOLI_MAX_REGISTRATIONS_PER_BLOCK",
                defaults.max_registrations_per_block,
            ),

            // Presence (telemetry - configurable for all networks)
            presence_window_ms: env_parse("DOLI_PRESENCE_WINDOW_MS", defaults.presence_window_ms),

            // Gossip mesh (locked for mainnet - wrong values could isolate nodes)
            mesh_n: if is_mainnet {
                defaults.mesh_n
            } else {
                env_parse("DOLI_MESH_N", defaults.mesh_n)
            },
            mesh_n_low: if is_mainnet {
                defaults.mesh_n_low
            } else {
                env_parse("DOLI_MESH_N_LOW", defaults.mesh_n_low)
            },
            mesh_n_high: if is_mainnet {
                defaults.mesh_n_high
            } else {
                env_parse("DOLI_MESH_N_HIGH", defaults.mesh_n_high)
            },
            gossip_lazy: if is_mainnet {
                defaults.gossip_lazy
            } else {
                env_parse("DOLI_GOSSIP_LAZY", defaults.gossip_lazy)
            },
        }
    }

    /// Get hardcoded default parameters for a network
    ///
    /// These match the original hardcoded values in consensus.rs (the DNA)
    pub fn defaults(network: Network) -> NetworkParams {
        use crate::consensus;

        match network {
            Network::Mainnet => NetworkParams {
                // Networking
                default_p2p_port: 30303,
                default_rpc_port: 8545,
                default_metrics_port: 9090,
                bootstrap_nodes: vec![
                    "/dns4/seed1.doli.network/tcp/30303".to_string(),
                    "/dns4/seed2.doli.network/tcp/30303".to_string(),
                ],

                // Timing
                slot_duration: consensus::SLOT_DURATION,
                genesis_time: consensus::GENESIS_TIME,
                veto_period_secs: 7 * 24 * 3600, // 7 days (policy, not consensus rule per se, but good default)
                grace_period_secs: 48 * 3600,    // 48 hours
                bootstrap_grace_period_secs: consensus::BOOTSTRAP_GRACE_PERIOD_SECS,
                unbonding_period: consensus::UNBONDING_PERIOD as u64, // blocks
                inactivity_threshold: consensus::INACTIVITY_THRESHOLD as u64,

                // Economics
                bond_unit: consensus::BOND_UNIT,
                initial_reward: consensus::INITIAL_REWARD,
                registration_base_fee: 100_000,      // 0.001 DOLI
                max_registration_fee: 1_000_000_000, // 10 DOLI
                automatic_genesis_bond: consensus::BOND_UNIT,
                genesis_blocks: 0,

                // VDF
                vdf_iterations: 100_000,
                heartbeat_vdf_iterations: 10_000_000,
                vdf_register_iterations: 600_000_000,

                // Time structure
                blocks_per_year: consensus::SLOTS_PER_YEAR as u64,
                blocks_per_reward_epoch: consensus::BLOCKS_PER_REWARD_EPOCH,
                coinbase_maturity: consensus::COINBASE_MATURITY,
                slots_per_reward_epoch: consensus::SLOTS_PER_REWARD_EPOCH,
                bootstrap_blocks: consensus::BOOTSTRAP_BLOCKS,

                // Update system
                min_voting_age_secs: 30 * 24 * 3600,  // 30 days
                update_check_interval_secs: 6 * 3600, // 6 hours
                crash_window_secs: 3600,              // 1 hour
                max_registrations_per_block: 5,

                // Presence (telemetry)
                presence_window_ms: consensus::NETWORK_MARGIN_MS, // Use consensus margin

                // Gossip mesh (standard for DHT-enabled networks)
                mesh_n: 6,
                mesh_n_low: 4,
                mesh_n_high: 12,
                gossip_lazy: 6,
            },

            Network::Testnet => NetworkParams {
                // Networking
                default_p2p_port: 40303,
                default_rpc_port: 18545,
                default_metrics_port: 19090,
                bootstrap_nodes: vec![
                    "/dns4/bootstrap1.testnet.doli.network/tcp/40303".to_string(),
                    "/dns4/bootstrap2.testnet.doli.network/tcp/40304".to_string(),
                ],

                // Timing (same as mainnet)
                slot_duration: consensus::SLOT_DURATION,
                genesis_time: 1769738400, // 2026-01-29T22:00:00Z (Testnet specific)
                veto_period_secs: 7 * 24 * 3600,
                grace_period_secs: 48 * 3600,
                bootstrap_grace_period_secs: consensus::BOOTSTRAP_GRACE_PERIOD_SECS,
                unbonding_period: consensus::UNBONDING_PERIOD as u64,
                inactivity_threshold: consensus::INACTIVITY_THRESHOLD as u64,

                // Economics (same as mainnet)
                bond_unit: consensus::BOND_UNIT,
                initial_reward: consensus::INITIAL_REWARD,
                registration_base_fee: 100_000,
                max_registration_fee: 1_000_000_000,
                automatic_genesis_bond: consensus::BOND_UNIT,
                genesis_blocks: 0,

                // VDF (same as mainnet)
                vdf_iterations: 100_000,
                heartbeat_vdf_iterations: 10_000_000,
                vdf_register_iterations: 600_000_000,

                // Time structure (same as mainnet)
                blocks_per_year: consensus::SLOTS_PER_YEAR as u64,
                blocks_per_reward_epoch: consensus::BLOCKS_PER_REWARD_EPOCH,
                coinbase_maturity: consensus::COINBASE_MATURITY,
                slots_per_reward_epoch: consensus::SLOTS_PER_REWARD_EPOCH, // 1 hour for faster testing
                bootstrap_blocks: consensus::BOOTSTRAP_BLOCKS,

                // Update system (same as mainnet)
                min_voting_age_secs: 30 * 24 * 3600,
                update_check_interval_secs: 6 * 3600,
                crash_window_secs: 3600,
                max_registrations_per_block: 5,

                // Presence (telemetry)
                presence_window_ms: consensus::NETWORK_MARGIN_MS,

                // Gossip mesh (standard for DHT-enabled networks)
                mesh_n: 6,
                mesh_n_low: 4,
                mesh_n_high: 12,
                gossip_lazy: 6,
            },

            Network::Devnet => NetworkParams {
                // Networking
                default_p2p_port: 50303,
                default_rpc_port: 28545,
                default_metrics_port: 29090,
                bootstrap_nodes: vec![], // No bootstrap for local devnet

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
                automatic_genesis_bond: consensus::BOND_UNIT, // Use mainnet bond for genesis?
                // Original code: 10_000_000_000 (100 DOLI)
                // Devnet bond_unit is 1 DOLI.
                // Wait, line 417 in original was 10_000_000_000.
                // So it uses MAINNET bond unit for genesis bond.
                genesis_blocks: 40,

                // VDF (fast for development)
                vdf_iterations: 1,                    // Single iteration
                heartbeat_vdf_iterations: 10_000_000, // Same as mainnet
                vdf_register_iterations: 5_000_000,   // ~5 seconds

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

                // Gossip mesh (larger for --no-dht star topology)
                // With --no-dht, all nodes connect to bootstrap only.
                // Gossipsub must keep all peers in mesh since pruned
                // nodes have no alternative peers for discovery.
                // Sized for ~30 node devnets; override via .env for larger.
                mesh_n: 15,
                mesh_n_low: 10,
                mesh_n_high: 35,
                gossip_lazy: 15,
            },
        }
    }

    // === Derived parameters ===

    /// Get blocks per month (1/12 of blocks per year)
    pub fn blocks_per_month(&self) -> u64 {
        self.blocks_per_year / 12
    }

    /// Get blocks per era (4 years)
    pub fn blocks_per_era(&self) -> u64 {
        self.blocks_per_year * 4
    }

    /// Get commitment period (same as era)
    pub fn commitment_period(&self) -> u64 {
        self.blocks_per_era()
    }

    /// Get exit history retention (8 years)
    pub fn exit_history_retention(&self) -> u64 {
        self.blocks_per_era() * 2
    }

    /// Get seniority maturity in blocks (4 years)
    pub fn seniority_maturity_blocks(&self) -> u64 {
        self.blocks_per_year * 4
    }

    /// Get seniority step blocks (1 year)
    pub fn seniority_step_blocks(&self) -> u64 {
        self.blocks_per_year
    }

    /// Get minimum voting age in blocks
    pub fn min_voting_age_blocks(&self) -> u64 {
        self.min_voting_age_secs / self.slot_duration
    }

    /// Get veto period in blocks
    pub fn veto_period_blocks(&self) -> u64 {
        self.veto_period_secs / self.slot_duration
    }
}

/// Parse an environment variable with a default fallback
fn env_parse<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Parse a comma-separated list of values from an environment variable
fn env_parse_vec(key: &str, default: Vec<String>) -> Vec<String> {
    std::env::var(key)
        .ok()
        .map(|v| {
            v.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or(default)
}

// ============================================================================
// Environment Loading (merged from env_loader.rs)
// ============================================================================

/// Load environment variables for a specific network
///
/// Loads from `{data_dir}/.env` if it exists.
/// This populates `std::env` with the values from the file,
/// which are then read by `NetworkParams::load()`.
///
/// # Arguments
///
/// * `network_name` - The network name (mainnet, testnet, devnet)
/// * `data_dir` - The data directory path (e.g., `~/.doli/mainnet`)
pub fn load_env_for_network(network_name: &str, data_dir: &Path) {
    let env_path = data_dir.join(".env");

    if env_path.exists() {
        match dotenvy::from_path(&env_path) {
            Ok(()) => {
                info!(
                    "Loaded environment from {:?} for {} network",
                    env_path, network_name
                );
            }
            Err(e) => {
                warn!("Failed to load environment from {:?}: {}", env_path, e);
            }
        }
    } else {
        // Fallback: check the network root directory (~/.doli/{network}/.env)
        // This handles custom --data-dir pointing to a subdirectory
        let network_root = get_default_data_dir(network_name);
        let root_env = network_root.join(".env");
        if root_env.exists() && root_env != env_path {
            match dotenvy::from_path(&root_env) {
                Ok(()) => {
                    info!(
                        "Loaded environment from {:?} (fallback) for {} network",
                        root_env, network_name
                    );
                }
                Err(e) => {
                    warn!(
                        "Failed to load environment from {:?} (fallback): {}",
                        root_env, e
                    );
                }
            }
        } else {
            debug!(
                "No .env file found at {:?} or {:?}, using defaults for {} network",
                env_path, root_env, network_name
            );
        }
    }
}

/// Get the default data directory for a network
///
/// Returns `~/.doli/{network_name}` or falls back to `./.doli/{network_name}`
/// if the home directory cannot be determined.
pub fn get_default_data_dir(network_name: &str) -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".doli")
        .join(network_name)
}

/// Initialize environment for a network using the default data directory
///
/// Convenience function that combines `get_default_data_dir` and `load_env_for_network`.
pub fn init_env_for_network(network_name: &str) {
    let data_dir = get_default_data_dir(network_name);
    load_env_for_network(network_name, &data_dir);
}

/// Apply chainspec consensus parameters as environment variable defaults.
///
/// Reads a chainspec JSON file and sets environment variables for consensus
/// parameters that are not already set. This makes the chainspec the lowest
/// priority source (below parent env and .env file) but above hardcoded defaults.
///
/// Priority hierarchy: Parent ENV > .env file > Chainspec > consensus.rs defaults
///
/// Skipped entirely for mainnet (defense-in-depth: mainnet params are locked).
pub fn apply_chainspec_defaults(chainspec_path: &Path) {
    use crate::chainspec::ChainSpec;

    let spec = match ChainSpec::load(chainspec_path) {
        Ok(spec) => spec,
        Err(e) => {
            warn!(
                "Could not load chainspec from {:?} for defaults: {}",
                chainspec_path, e
            );
            return;
        }
    };

    // Defense-in-depth: never override mainnet params from chainspec
    if matches!(spec.network, Network::Mainnet) {
        debug!("Skipping chainspec defaults for mainnet (locked parameters)");
        return;
    }

    let mut applied = Vec::new();

    if set_env_if_absent(
        "DOLI_SLOT_DURATION",
        &spec.consensus.slot_duration.to_string(),
    ) {
        applied.push(format!(
            "DOLI_SLOT_DURATION={}",
            spec.consensus.slot_duration
        ));
    }
    if set_env_if_absent("DOLI_BOND_UNIT", &spec.consensus.bond_amount.to_string()) {
        applied.push(format!("DOLI_BOND_UNIT={}", spec.consensus.bond_amount));
    }
    if set_env_if_absent(
        "DOLI_SLOTS_PER_REWARD_EPOCH",
        &spec.consensus.slots_per_epoch.to_string(),
    ) {
        applied.push(format!(
            "DOLI_SLOTS_PER_REWARD_EPOCH={}",
            spec.consensus.slots_per_epoch
        ));
    }
    if set_env_if_absent(
        "DOLI_INITIAL_REWARD",
        &spec.genesis.initial_reward.to_string(),
    ) {
        applied.push(format!(
            "DOLI_INITIAL_REWARD={}",
            spec.genesis.initial_reward
        ));
    }
    if spec.genesis.timestamp != 0
        && set_env_if_absent("DOLI_GENESIS_TIME", &spec.genesis.timestamp.to_string())
    {
        applied.push(format!("DOLI_GENESIS_TIME={}", spec.genesis.timestamp));
    }

    if applied.is_empty() {
        debug!("Chainspec defaults: all vars already set, nothing applied");
    } else {
        info!(
            "Applied {} chainspec defaults: {}",
            applied.len(),
            applied.join(", ")
        );
    }
}

/// Set an environment variable only if it is not already set.
/// Returns true if the variable was set (i.e., it was absent).
fn set_env_if_absent(key: &str, value: &str) -> bool {
    if std::env::var(key).is_err() {
        std::env::set_var(key, value);
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Global mutex to serialize tests that modify process environment variables.
    /// Env vars are process-global, so parallel tests can interfere with each other.
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn test_defaults_match_network_rs() {
        // Verify that defaults match the original hardcoded values
        let mainnet = NetworkParams::defaults(Network::Mainnet);
        assert_eq!(mainnet.default_p2p_port, 30303);
        assert_eq!(mainnet.default_rpc_port, 8545);
        assert_eq!(mainnet.slot_duration, 10);
        assert_eq!(mainnet.bond_unit, 10_000_000_000);
        assert_eq!(mainnet.blocks_per_year, 3_153_600);

        let devnet = NetworkParams::defaults(Network::Devnet);
        assert_eq!(devnet.default_p2p_port, 50303);
        assert_eq!(devnet.default_rpc_port, 28545);
        assert_eq!(devnet.bond_unit, 100_000_000);
        assert_eq!(devnet.blocks_per_year, 144);
    }

    #[test]
    fn test_derived_parameters() {
        let mainnet = NetworkParams::defaults(Network::Mainnet);
        assert_eq!(mainnet.blocks_per_month(), mainnet.blocks_per_year / 12);
        assert_eq!(mainnet.blocks_per_era(), mainnet.blocks_per_year * 4);
        assert_eq!(mainnet.commitment_period(), mainnet.blocks_per_era());
        assert_eq!(
            mainnet.exit_history_retention(),
            mainnet.blocks_per_era() * 2
        );
    }

    #[test]
    fn test_env_override() {
        let _lock = ENV_MUTEX.lock().unwrap();

        // Save original value to restore later
        let original_val = std::env::var("DOLI_SLOT_DURATION");

        // Set test value (override default of 10s/1s)
        std::env::set_var("DOLI_SLOT_DURATION", "42");

        // Load params for Devnet (which allows env overrides)
        let params = NetworkParams::load_from_env(Network::Devnet);

        // Restore environment
        if let Ok(val) = original_val {
            std::env::set_var("DOLI_SLOT_DURATION", val);
        } else {
            std::env::remove_var("DOLI_SLOT_DURATION");
        }

        // Verify override took effect
        assert_eq!(params.slot_duration, 42);

        // Verify Mainnet IGNORES the override (locked params)
        let mainnet_params = NetworkParams::load_from_env(Network::Mainnet);
        assert_eq!(mainnet_params.slot_duration, 10); // Should remain 10 despite env var
    }
    #[test]
    fn test_env_parse() {
        // Test with non-existent env var (should use default)
        let result: u16 = env_parse("NONEXISTENT_VAR_12345", 42);
        assert_eq!(result, 42);
    }

    #[test]
    fn test_env_parse_vec() {
        // Test with non-existent env var (should use default)
        let default = vec!["a".to_string(), "b".to_string()];
        let result = env_parse_vec("NONEXISTENT_VAR_12345", default.clone());
        assert_eq!(result, default);
    }

    #[test]
    fn test_load_env_for_network_no_file() {
        // Should not panic when .env file doesn't exist
        let temp_dir = tempfile::TempDir::new().unwrap();
        load_env_for_network("testnet", temp_dir.path());
    }

    #[test]
    fn test_load_env_for_network_with_file() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let temp_dir = tempfile::TempDir::new().unwrap();
        let env_path = temp_dir.path().join(".env");

        // Write a test .env file
        std::fs::write(&env_path, "DOLI_TEST_VAR_NETWORK_PARAMS=test_value\n").unwrap();

        // Clear any existing value
        std::env::remove_var("DOLI_TEST_VAR_NETWORK_PARAMS");

        // Load the env file
        load_env_for_network("testnet", temp_dir.path());

        // Verify the value was loaded
        assert_eq!(
            std::env::var("DOLI_TEST_VAR_NETWORK_PARAMS").ok(),
            Some("test_value".to_string())
        );

        // Clean up
        std::env::remove_var("DOLI_TEST_VAR_NETWORK_PARAMS");
    }

    #[test]
    fn test_get_default_data_dir() {
        let data_dir = get_default_data_dir("mainnet");
        assert!(data_dir.ends_with(".doli/mainnet"));
    }

    #[test]
    fn test_load_env_fallback_to_network_root() {
        let _lock = ENV_MUTEX.lock().unwrap();
        // Create a "network root" dir with .env, and a "subdir" without .env
        let root_dir = tempfile::TempDir::new().unwrap();
        let sub_dir = root_dir.path().join("data").join("node5");
        std::fs::create_dir_all(&sub_dir).unwrap();

        // Write .env only in root
        let env_path = root_dir.path().join(".env");
        std::fs::write(&env_path, "DOLI_TEST_FALLBACK_VAR=from_root\n").unwrap();
        std::env::remove_var("DOLI_TEST_FALLBACK_VAR");

        // The sub_dir has no .env, so load_env_for_network won't find it there.
        // The fallback uses get_default_data_dir which goes to ~/.doli/{network},
        // so we can't fully test the fallback path here without mocking HOME.
        // Instead, verify the function doesn't panic on subdirs without .env.
        load_env_for_network("devnet", &sub_dir);

        // Clean up
        std::env::remove_var("DOLI_TEST_FALLBACK_VAR");
    }

    #[test]
    fn test_apply_chainspec_defaults_sets_vars() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let temp_dir = tempfile::TempDir::new().unwrap();
        let chainspec_path = temp_dir.path().join("chainspec.json");

        // Write a minimal devnet chainspec
        let chainspec_json = r#"{
            "name": "Test Devnet",
            "id": "devnet",
            "network": "Devnet",
            "genesis": {
                "timestamp": 1700000000,
                "message": "test",
                "initial_reward": 5000000000
            },
            "consensus": {
                "slot_duration": 7,
                "slots_per_epoch": 42,
                "bond_amount": 200000000
            },
            "genesis_producers": []
        }"#;
        std::fs::write(&chainspec_path, chainspec_json).unwrap();

        // Clear all related vars
        std::env::remove_var("DOLI_SLOT_DURATION");
        std::env::remove_var("DOLI_BOND_UNIT");
        std::env::remove_var("DOLI_SLOTS_PER_REWARD_EPOCH");
        std::env::remove_var("DOLI_INITIAL_REWARD");
        std::env::remove_var("DOLI_GENESIS_TIME");

        apply_chainspec_defaults(&chainspec_path);

        assert_eq!(std::env::var("DOLI_SLOT_DURATION").unwrap(), "7");
        assert_eq!(std::env::var("DOLI_BOND_UNIT").unwrap(), "200000000");
        assert_eq!(std::env::var("DOLI_SLOTS_PER_REWARD_EPOCH").unwrap(), "42");
        assert_eq!(std::env::var("DOLI_INITIAL_REWARD").unwrap(), "5000000000");
        assert_eq!(std::env::var("DOLI_GENESIS_TIME").unwrap(), "1700000000");

        // Clean up
        std::env::remove_var("DOLI_SLOT_DURATION");
        std::env::remove_var("DOLI_BOND_UNIT");
        std::env::remove_var("DOLI_SLOTS_PER_REWARD_EPOCH");
        std::env::remove_var("DOLI_INITIAL_REWARD");
        std::env::remove_var("DOLI_GENESIS_TIME");
    }

    #[test]
    fn test_apply_chainspec_defaults_no_override() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let temp_dir = tempfile::TempDir::new().unwrap();
        let chainspec_path = temp_dir.path().join("chainspec.json");

        let chainspec_json = r#"{
            "name": "Test Devnet",
            "id": "devnet",
            "network": "Devnet",
            "genesis": {
                "timestamp": 0,
                "message": "test",
                "initial_reward": 5000000000
            },
            "consensus": {
                "slot_duration": 7,
                "slots_per_epoch": 42,
                "bond_amount": 200000000
            },
            "genesis_producers": []
        }"#;
        std::fs::write(&chainspec_path, chainspec_json).unwrap();

        // Pre-set a var — chainspec should NOT override it
        std::env::set_var("DOLI_SLOT_DURATION", "99");

        apply_chainspec_defaults(&chainspec_path);

        // Should remain 99, not 7 from chainspec
        assert_eq!(std::env::var("DOLI_SLOT_DURATION").unwrap(), "99");

        // Clean up
        std::env::remove_var("DOLI_SLOT_DURATION");
        std::env::remove_var("DOLI_BOND_UNIT");
        std::env::remove_var("DOLI_SLOTS_PER_REWARD_EPOCH");
        std::env::remove_var("DOLI_INITIAL_REWARD");
    }

    #[test]
    fn test_apply_chainspec_defaults_mainnet_skipped() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let temp_dir = tempfile::TempDir::new().unwrap();
        let chainspec_path = temp_dir.path().join("chainspec.json");

        let chainspec_json = r#"{
            "name": "Test Mainnet",
            "id": "mainnet",
            "network": "Mainnet",
            "genesis": {
                "timestamp": 1700000000,
                "message": "test",
                "initial_reward": 999
            },
            "consensus": {
                "slot_duration": 999,
                "slots_per_epoch": 999,
                "bond_amount": 999
            },
            "genesis_producers": []
        }"#;
        std::fs::write(&chainspec_path, chainspec_json).unwrap();

        // Clear vars
        std::env::remove_var("DOLI_SLOT_DURATION_MAINNET_TEST");

        apply_chainspec_defaults(&chainspec_path);

        // Mainnet chainspec should be skipped entirely — vars should NOT be set
        assert!(
            std::env::var("DOLI_SLOT_DURATION").is_err()
                || std::env::var("DOLI_SLOT_DURATION").unwrap() != "999"
        );
    }

    #[test]
    fn test_apply_chainspec_defaults_malformed_file() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let chainspec_path = temp_dir.path().join("chainspec.json");

        // Write invalid JSON
        std::fs::write(&chainspec_path, "{ not valid json }").unwrap();

        // Should not panic, just log a warning
        apply_chainspec_defaults(&chainspec_path);
    }
}
