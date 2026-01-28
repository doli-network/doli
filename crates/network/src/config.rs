//! Network configuration

use std::net::SocketAddr;
use std::path::PathBuf;

use crypto::Hash;
use doli_core::Network;

/// Network configuration
#[derive(Clone, Debug)]
pub struct NetworkConfig {
    /// Listen address
    pub listen_addr: SocketAddr,
    /// Bootstrap nodes (multiaddrs)
    pub bootstrap_nodes: Vec<String>,
    /// Maximum number of peers
    pub max_peers: usize,
    /// Node key path (for persistent identity)
    pub node_key_path: Option<PathBuf>,
    /// Network ID (1 = mainnet, 2 = testnet, 99 = devnet)
    pub network_id: u32,
    /// Genesis hash for chain verification
    pub genesis_hash: Hash,
    /// Disable DHT discovery (only connect to explicit bootstrap nodes)
    /// Use this to isolate test networks from external peers
    pub no_dht: bool,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self::for_network(Network::Mainnet, Hash::default())
    }
}

impl NetworkConfig {
    /// Create a config for a specific network
    pub fn for_network(network: Network, genesis_hash: Hash) -> Self {
        Self {
            listen_addr: format!("0.0.0.0:{}", network.default_p2p_port())
                .parse()
                .unwrap(),
            bootstrap_nodes: network
                .bootstrap_nodes()
                .into_iter()
                .map(String::from)
                .collect(),
            max_peers: 50,
            node_key_path: None,
            network_id: network.id(),
            genesis_hash,
            no_dht: false,
        }
    }

    /// Create a config for mainnet
    pub fn mainnet(genesis_hash: Hash) -> Self {
        Self::for_network(Network::Mainnet, genesis_hash)
    }

    /// Create a config for testnet
    pub fn testnet(genesis_hash: Hash) -> Self {
        Self::for_network(Network::Testnet, genesis_hash)
    }

    /// Create a config for local development
    pub fn devnet(genesis_hash: Hash) -> Self {
        let mut config = Self::for_network(Network::Devnet, genesis_hash);
        config.max_peers = 10;
        config
    }

    /// Create a config for local development (alias for devnet)
    pub fn local(genesis_hash: Hash) -> Self {
        Self::devnet(genesis_hash)
    }

    /// Set the node key path
    pub fn with_node_key_path(mut self, path: PathBuf) -> Self {
        self.node_key_path = Some(path);
        self
    }

    /// Add a bootstrap node
    pub fn with_bootstrap_node(mut self, addr: String) -> Self {
        self.bootstrap_nodes.push(addr);
        self
    }

    /// Disable DHT discovery (only connect to explicit bootstrap nodes)
    pub fn with_no_dht(mut self, no_dht: bool) -> Self {
        self.no_dht = no_dht;
        self
    }
}
