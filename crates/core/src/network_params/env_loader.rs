//! Environment-based parameter loading
//!
//! Handles loading `.env` files and parsing environment variables into
//! [`NetworkParams`]. Mainnet parameters are locked — env overrides are ignored.

use std::path::Path;

use tracing::{debug, info, warn};

use crate::Network;

use super::NetworkParams;

/// Initialize parameters from environment (called once per network via `OnceLock`)
pub(super) fn load_from_env(network: Network) -> NetworkParams {
    let defaults = NetworkParams::defaults(network);

    // For mainnet, enforce locked parameters
    let is_mainnet = matches!(network, Network::Mainnet);

    NetworkParams {
        // Networking (configurable for all networks)
        default_p2p_port: env_parse("DOLI_P2P_PORT", defaults.default_p2p_port),
        default_rpc_port: env_parse("DOLI_RPC_PORT", defaults.default_rpc_port),
        default_metrics_port: env_parse("DOLI_METRICS_PORT", defaults.default_metrics_port),
        bootstrap_nodes: env_parse_vec("DOLI_BOOTSTRAP_NODES", defaults.bootstrap_nodes.clone()),
        max_peers: env_parse("DOLI_MAX_PEERS", defaults.max_peers),

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
            defaults.bond_unit // LOCKED for mainnet — consensus-critical
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
        max_registration_fee: env_parse("DOLI_MAX_REGISTRATION_FEE", defaults.max_registration_fee),
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
        min_voting_age_secs: env_parse("DOLI_MIN_VOTING_AGE_SECS", defaults.min_voting_age_secs),
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

        // Fallback timing (locked for mainnet - consensus critical)
        fallback_timeout_ms: if is_mainnet {
            defaults.fallback_timeout_ms // LOCKED for mainnet
        } else {
            env_parse("DOLI_FALLBACK_TIMEOUT_MS", defaults.fallback_timeout_ms)
        },
        max_fallback_ranks: if is_mainnet {
            defaults.max_fallback_ranks // LOCKED for mainnet
        } else {
            env_parse("DOLI_MAX_FALLBACK_RANKS", defaults.max_fallback_ranks)
        },
        network_margin_ms: if is_mainnet {
            defaults.network_margin_ms // LOCKED for mainnet
        } else {
            env_parse("DOLI_NETWORK_MARGIN_MS", defaults.network_margin_ms)
        },

        // Vesting (locked for mainnet — consensus critical)
        vesting_quarter_slots: if is_mainnet {
            defaults.vesting_quarter_slots // LOCKED for mainnet
        } else {
            env_parse("DOLI_VESTING_QUARTER_SLOTS", defaults.vesting_quarter_slots)
        },

        // Hard fork gates (configurable for all networks including mainnet)
        sig_verification_height: env_parse(
            "DOLI_SIG_VERIFICATION_HEIGHT",
            defaults.sig_verification_height,
        ),

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

/// Parse an environment variable with a default fallback
pub(super) fn env_parse<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Parse a comma-separated list of values from an environment variable
pub(super) fn env_parse_vec(key: &str, default: Vec<String>) -> Vec<String> {
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
// Environment file loading
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
