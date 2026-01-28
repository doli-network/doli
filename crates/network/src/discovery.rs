//! Peer discovery using Kademlia DHT
//!
//! Provides peer discovery and routing using the Kademlia distributed hash table.

use libp2p::kad::{store::MemoryStore, Behaviour as Kademlia, Config as KademliaConfig};
use libp2p::PeerId;

/// Protocol identifier for DOLI Kademlia
pub const KAD_PROTOCOL: &str = "/doli/kad/1.0.0";

/// Create a new Kademlia behaviour for peer discovery
pub fn new_kademlia(local_peer_id: PeerId) -> Kademlia<MemoryStore> {
    let store = MemoryStore::new(local_peer_id);

    let mut config = KademliaConfig::default();
    config.set_protocol_names(vec![libp2p::StreamProtocol::new(KAD_PROTOCOL)]);

    // Set replication factor
    config.set_replication_factor(std::num::NonZeroUsize::new(20).unwrap());

    // Enable server mode to respond to queries
    config.set_query_timeout(std::time::Duration::from_secs(60));

    Kademlia::with_config(local_peer_id, store, config)
}

/// Events emitted by the discovery layer
#[derive(Debug)]
pub enum DiscoveryEvent {
    /// A new peer was discovered
    PeerDiscovered(PeerId),
    /// Peer discovery completed a round
    DiscoveryComplete,
}
