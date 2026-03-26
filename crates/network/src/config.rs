//! Network configuration

use std::net::SocketAddr;
use std::path::PathBuf;

use crypto::Hash;
use doli_core::Network;

use libp2p::Multiaddr;

use crate::nat::NatConfig;

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
    /// Path to persistent peer cache file
    pub peer_cache_path: Option<PathBuf>,
    /// Target number of peers in gossipsub mesh per topic
    pub mesh_n: usize,
    /// Minimum peers in gossipsub mesh before requesting more
    pub mesh_n_low: usize,
    /// Maximum peers in gossipsub mesh before pruning
    pub mesh_n_high: usize,
    /// Number of peers to lazily gossip IHAVE messages to
    pub gossip_lazy: usize,
    /// NAT traversal configuration (relay, DCUtR, AutoNAT)
    pub nat_config: NatConfig,
    /// External address to advertise (e.g., public IP for multi-node hosts)
    pub external_address: Option<Multiaddr>,
    /// Enable transaction announce-request protocol (EIP-4938 style).
    /// When true, nodes broadcast tx hashes instead of full txs via gossipsub,
    /// and peers fetch missing transactions via the txfetch request-response protocol.
    /// Reduces bandwidth ~80% on redundant tx data at scale (1000+ nodes).
    pub tx_announce_enabled: bool,
    /// Maximum number of temporary "bootstrap-only" connections.
    /// When the peer table is full (at max_peers), extra incoming connections
    /// are accepted into a bootstrap-only state for DHT exchange only.
    /// After a brief window (10s), the connection is closed to free the slot.
    /// This solves the chicken-and-egg problem: new nodes need a connected peer
    /// to run Kademlia discovery, but the seed's peer table may be full.
    /// Default: same as max_peers.
    pub bootstrap_slots: usize,
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
            bootstrap_nodes: network.bootstrap_nodes(),
            max_peers: 50, // Default; overridden from NetworkParams in node startup
            node_key_path: None,
            network_id: network.id(),
            genesis_hash,
            no_dht: false,
            peer_cache_path: None,
            // Universal gossipsub mesh (same for all networks)
            mesh_n: 12,
            mesh_n_low: 8,
            mesh_n_high: 24,
            gossip_lazy: 12,
            nat_config: NatConfig::default(),
            external_address: None,
            tx_announce_enabled: false,
            bootstrap_slots: 50, // Default; overridden from max_peers in node startup
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
        Self::for_network(Network::Devnet, genesis_hash)
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

    /// Set NAT traversal configuration
    pub fn with_nat_config(mut self, nat_config: NatConfig) -> Self {
        self.nat_config = nat_config;
        self
    }
}
