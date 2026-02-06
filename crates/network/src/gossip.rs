//! GossipSub for block and transaction propagation
//!
//! Implements efficient message propagation using libp2p GossipSub protocol.

use libp2p::gossipsub::{
    Behaviour as Gossipsub, ConfigBuilder, IdentTopic, Message, MessageAuthenticity, MessageId,
    ValidationMode,
};
use libp2p::identity::Keypair;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Duration;

/// GossipSub topic for new blocks
pub const BLOCKS_TOPIC: &str = "/doli/blocks/1";

/// GossipSub topic for new transactions
pub const TRANSACTIONS_TOPIC: &str = "/doli/txs/1";

/// GossipSub topic for producer announcements (bootstrap protocol)
pub const PRODUCERS_TOPIC: &str = "/doli/producers/1";

/// GossipSub topic for update votes (governance veto system)
pub const VOTES_TOPIC: &str = "/doli/votes/1";

/// GossipSub topic for presence heartbeats (weighted rewards)
pub const HEARTBEATS_TOPIC: &str = "/doli/heartbeats/1";

/// Gossipsub mesh configuration
pub struct MeshConfig {
    pub mesh_n: usize,
    pub mesh_n_low: usize,
    pub mesh_n_high: usize,
    pub gossip_lazy: usize,
}

/// Create a new GossipSub behaviour with configurable mesh parameters.
///
/// Mesh parameters are loaded from `NetworkParams` via env vars / `.env` / defaults.
/// Devnet uses larger mesh (12/8/48) for --no-dht star topology.
/// Mainnet/testnet use standard mesh (6/4/12) with DHT peer rotation.
pub fn new_gossipsub(keypair: &Keypair, mesh: &MeshConfig) -> Result<Gossipsub, GossipError> {
    // Message ID function: hash of message data
    let message_id_fn = |message: &Message| {
        let mut hasher = DefaultHasher::new();
        message.data.hash(&mut hasher);
        MessageId::from(hasher.finish().to_be_bytes().to_vec())
    };

    let config = ConfigBuilder::default()
        // Heartbeat interval
        .heartbeat_interval(Duration::from_secs(1))
        // Message validation
        .validation_mode(ValidationMode::Strict)
        // Message ID function
        .message_id_fn(message_id_fn)
        // Mesh parameters (from NetworkParams)
        .mesh_n(mesh.mesh_n)
        .mesh_n_low(mesh.mesh_n_low)
        .mesh_n_high(mesh.mesh_n_high)
        .mesh_outbound_min(2) // Minimum outbound peers in mesh
        // Gossip parameters
        .gossip_lazy(mesh.gossip_lazy)
        .gossip_factor(0.25) // Gossip to 25% of non-mesh peers
        // History
        .history_length(5)
        .history_gossip(3)
        // Message size limit (1MB for blocks)
        .max_transmit_size(1024 * 1024)
        // Duplicate cache time
        .duplicate_cache_time(Duration::from_secs(60))
        .build()
        .map_err(|e| GossipError::Config(e.to_string()))?;

    Gossipsub::new(MessageAuthenticity::Signed(keypair.clone()), config)
        .map_err(|e| GossipError::Init(e.to_string()))
}

/// Subscribe to block, transaction, producer, vote, and heartbeat topics
pub fn subscribe_to_topics(gossipsub: &mut Gossipsub) -> Result<(), GossipError> {
    let blocks_topic = IdentTopic::new(BLOCKS_TOPIC);
    let txs_topic = IdentTopic::new(TRANSACTIONS_TOPIC);
    let producers_topic = IdentTopic::new(PRODUCERS_TOPIC);
    let votes_topic = IdentTopic::new(VOTES_TOPIC);
    let heartbeats_topic = IdentTopic::new(HEARTBEATS_TOPIC);

    gossipsub
        .subscribe(&blocks_topic)
        .map_err(|e| GossipError::Subscribe(format!("blocks: {}", e)))?;
    gossipsub
        .subscribe(&txs_topic)
        .map_err(|e| GossipError::Subscribe(format!("txs: {}", e)))?;
    gossipsub
        .subscribe(&producers_topic)
        .map_err(|e| GossipError::Subscribe(format!("producers: {}", e)))?;
    gossipsub
        .subscribe(&votes_topic)
        .map_err(|e| GossipError::Subscribe(format!("votes: {}", e)))?;
    gossipsub
        .subscribe(&heartbeats_topic)
        .map_err(|e| GossipError::Subscribe(format!("heartbeats: {}", e)))?;

    Ok(())
}

/// Publish a block to the network
pub fn publish_block(gossipsub: &mut Gossipsub, block_data: Vec<u8>) -> Result<(), GossipError> {
    let topic = IdentTopic::new(BLOCKS_TOPIC);
    gossipsub
        .publish(topic, block_data)
        .map_err(|e| GossipError::Publish(format!("block: {}", e)))?;
    Ok(())
}

/// Publish a transaction to the network
pub fn publish_transaction(gossipsub: &mut Gossipsub, tx_data: Vec<u8>) -> Result<(), GossipError> {
    let topic = IdentTopic::new(TRANSACTIONS_TOPIC);
    gossipsub
        .publish(topic, tx_data)
        .map_err(|e| GossipError::Publish(format!("tx: {}", e)))?;
    Ok(())
}

/// Publish a producer announcement to the network
pub fn publish_producer(
    gossipsub: &mut Gossipsub,
    producer_data: Vec<u8>,
) -> Result<(), GossipError> {
    let topic = IdentTopic::new(PRODUCERS_TOPIC);
    gossipsub
        .publish(topic, producer_data)
        .map_err(|e| GossipError::Publish(format!("producer: {}", e)))?;
    Ok(())
}

/// Publish a vote message to the network (for governance veto system)
pub fn publish_vote(gossipsub: &mut Gossipsub, vote_data: Vec<u8>) -> Result<(), GossipError> {
    let topic = IdentTopic::new(VOTES_TOPIC);
    gossipsub
        .publish(topic, vote_data)
        .map_err(|e| GossipError::Publish(format!("vote: {}", e)))?;
    Ok(())
}

/// Publish a heartbeat to the network (for weighted presence rewards)
pub fn publish_heartbeat(
    gossipsub: &mut Gossipsub,
    heartbeat_data: Vec<u8>,
) -> Result<(), GossipError> {
    let topic = IdentTopic::new(HEARTBEATS_TOPIC);
    gossipsub
        .publish(topic, heartbeat_data)
        .map_err(|e| GossipError::Publish(format!("heartbeat: {}", e)))?;
    Ok(())
}

/// GossipSub errors
#[derive(Debug, thiserror::Error)]
pub enum GossipError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("initialization error: {0}")]
    Init(String),

    #[error("subscribe error: {0}")]
    Subscribe(String),

    #[error("publish error: {0}")]
    Publish(String),
}
