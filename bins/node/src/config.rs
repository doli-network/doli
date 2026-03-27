//! Node configuration

use std::path::PathBuf;

use doli_core::chainspec::ChainSpec;
use doli_core::Network;
use serde::{Deserialize, Serialize};

/// Node configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeConfig {
    /// Network (mainnet, testnet, devnet)
    pub network: Network,

    /// Data directory
    pub data_dir: PathBuf,

    /// P2P listen address
    pub listen_addr: String,

    /// Bootstrap nodes
    pub bootstrap_nodes: Vec<String>,

    /// Maximum peers
    pub max_peers: usize,

    /// RPC settings
    pub rpc: RpcConfig,

    /// Producer settings (optional)
    pub producer: Option<ProducerConfig>,

    /// Disable DHT discovery (only connect to explicit bootstrap nodes)
    /// Use this to isolate test networks from external peers
    #[serde(default)]
    pub no_dht: bool,

    /// Enable relay server mode (allow NAT'd peers to relay through this node)
    #[serde(default)]
    pub relay_server: bool,

    /// Override genesis time from chainspec (for coordinated devnet startup)
    /// When set, this overrides the dynamic genesis time calculation.
    #[serde(default)]
    pub genesis_time_override: Option<u64>,

    /// Loaded chainspec (not serialized — passed in-memory from main)
    #[serde(skip)]
    pub chainspec: Option<ChainSpec>,

    /// Override slot duration from chainspec (ensures all nodes agree on slot timing)
    /// This is consensus-critical: different slot durations cause producer schedule divergence.
    #[serde(default)]
    pub slot_duration_override: Option<u64>,

    /// External address to advertise to peers (e.g., /ip4/198.51.100.1/tcp/30300)
    /// Use when running multiple nodes on the same host to prevent advertising 127.0.0.1.
    #[serde(skip)]
    pub external_address: Option<String>,

    /// Disable snap sync entirely. The node will only use header-first sync.
    /// Defaults to true on mainnet (full sync from genesis).
    /// Use --snap-sync to explicitly enable for faster initial sync.
    #[serde(default)]
    pub no_snap_sync: bool,

    /// Seed mode: only serve blocks that are confirmed (N blocks deep).
    /// Prevents fork propagation to syncing peers.
    #[serde(default)]
    pub seed_mode: bool,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self::for_network(Network::Mainnet)
    }
}

impl NodeConfig {
    /// Create default configuration for a specific network
    pub fn for_network(network: Network) -> Self {
        let data_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".doli")
            .join(network.data_dir_name());

        Self {
            network,
            data_dir,
            listen_addr: format!("0.0.0.0:{}", network.default_p2p_port()),
            bootstrap_nodes: network.bootstrap_nodes(),
            // INC-I-014: Allow env override for stress test tuning without rebuild.
            // Pattern matches DOLI_CONN_LIMIT, DOLI_PENDING_LIMIT, DOLI_BOOTSTRAP_SLOTS.
            max_peers: std::env::var("DOLI_MAX_PEERS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(network.params().max_peers),
            rpc: RpcConfig::for_network(network),
            producer: None,
            no_dht: false,
            relay_server: false,
            genesis_time_override: None,
            chainspec: None,
            slot_duration_override: None,
            external_address: None,
            // REQ-SYNC-002: Mainnet defaults to full sync (no snap sync).
            // Snap sync skips block_store, making producer_liveness empty and
            // consensus-critical scheduling divergent between full and snap-synced nodes.
            no_snap_sync: network == Network::Mainnet,
            seed_mode: false,
        }
    }
}

/// RPC server configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RpcConfig {
    /// Enable RPC server
    pub enabled: bool,

    /// RPC listen address
    pub listen_addr: String,

    /// Allowed methods (empty = all)
    pub allowed_methods: Vec<String>,
}

impl Default for RpcConfig {
    fn default() -> Self {
        Self::for_network(Network::Mainnet)
    }
}

impl RpcConfig {
    /// Create RPC configuration for a specific network
    pub fn for_network(network: Network) -> Self {
        Self {
            enabled: true,
            listen_addr: format!("127.0.0.1:{}", network.default_rpc_port()),
            allowed_methods: vec![],
        }
    }
}

/// Producer configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProducerConfig {
    /// Producer key file path
    pub key_file: PathBuf,

    /// Reward address
    pub reward_address: String,
}
