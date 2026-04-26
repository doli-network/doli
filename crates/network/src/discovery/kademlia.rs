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

    // INC-I-050: Replication factor 20 in a 26-node network (77%) amplified DHT
    // poisoning — a single bad entry reached nearly every node within one 60s
    // bootstrap cycle. Reduced to 8 (~31%) to limit amplification while
    // maintaining sufficient redundancy for peer discovery.
    config.set_replication_factor(std::num::NonZeroUsize::new(8).unwrap());

    config.set_query_timeout(std::time::Duration::from_secs(60));

    let mut kad = Kademlia::with_config(local_peer_id, store, config);
    // Server mode: respond to DHT queries from other peers
    kad.set_mode(Some(libp2p::kad::Mode::Server));
    kad
}
