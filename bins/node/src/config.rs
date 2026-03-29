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

    /// Disable snap sync (header-first only). Defaults to false.
    #[serde(default)]
    pub no_snap_sync: bool,

    /// Seed mode: only serve confirmed blocks (N blocks deep).
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
        let net_name = network.data_dir_name();

        // Platform-aware default with legacy fallback
        #[cfg(target_os = "linux")]
        let platform_default = PathBuf::from("/var/lib/doli").join(net_name);
        #[cfg(target_os = "macos")]
        let platform_default = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("Library/Application Support/doli")
            .join(net_name);
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let platform_default = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".doli")
            .join(net_name);

        let data_dir = if platform_default.exists() {
            platform_default.clone()
        } else {
            let legacy = dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".doli")
                .join(net_name);
            if legacy.exists() {
                legacy
            } else {
                platform_default
            }
        };

        Self {
            network,
            data_dir,
            listen_addr: format!("0.0.0.0:{}", network.default_p2p_port()),
            bootstrap_nodes: network.bootstrap_nodes(),
            max_peers: network.params().max_peers,
            rpc: RpcConfig::for_network(network),
            producer: None,
            no_dht: false,
            relay_server: false,
            genesis_time_override: None,
            chainspec: None,
            slot_duration_override: None,
            external_address: None,
            no_snap_sync: false,
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
