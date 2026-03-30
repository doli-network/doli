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

use std::sync::OnceLock;

use crate::Network;

mod chainspec_loader;
mod defaults;
mod env_loader;
#[cfg(test)]
mod tests;

pub use chainspec_loader::apply_chainspec_defaults;
pub use env_loader::{get_default_data_dir, init_env_for_network, load_env_for_network};

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
    /// Maximum peer connections per node (application layer).
    /// Transport layer allows 1.5× this for handshake headroom.
    /// Mainnet/Testnet: 50 (Ethereum default). Devnet: 150.
    /// Override: DOLI_MAX_PEERS env var.
    pub max_peers: usize,

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

    // === Fallback timing ===
    /// Sequential fallback timeout per rank in milliseconds
    pub fallback_timeout_ms: u64,
    /// Maximum fallback ranks per slot
    pub max_fallback_ranks: usize,
    /// Network margin / clock drift tolerance in milliseconds
    pub network_margin_ms: u64,

    // === Vesting (locked for mainnet — consensus critical) ===
    /// Vesting quarter duration in slots (default: 2,160 = 6 hours)
    pub vesting_quarter_slots: u64,

    // === Hard fork gates ===
    /// Height at which Input.public_key becomes mandatory for signature verification.
    /// Before this height: public_key=None accepted (legacy, no sig verification).
    /// At or after: public_key must be Some, signature + pubkey_hash verified.
    /// Mainnet: u64::MAX (not yet activated). Testnet/Devnet: 0 (always enforce).
    pub sig_verification_height: u64,

    /// Height at which post-snap attestation skip becomes active (INC-I-010).
    /// Before this height: epoch boundary always filters by attestation (old behavior).
    /// At or after: skip attestation filtering when block history is incomplete.
    /// Mainnet: 8500. Testnet/Devnet: 0 (always active).
    pub snap_attestation_skip_height: u64,

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

        lock.get_or_init(|| env_loader::load_from_env(network))
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
