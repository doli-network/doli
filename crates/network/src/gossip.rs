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

// ==================== Tiered Architecture Topics ====================

/// Tier 1 block propagation (dense mesh, validators only)
pub const TIER1_BLOCKS_TOPIC: &str = "/doli/t1/blocks/1";

/// Lightweight header topic (all tiers — for Tier 3 header-only validation)
pub const HEADERS_TOPIC: &str = "/doli/headers/1";

/// Attestation topic (Tier 1 + Tier 2 — for finality gadget)
pub const ATTESTATION_TOPIC: &str = "/doli/attestations/1";

/// Generate a regional block topic for Tier 2 sharding.
pub fn region_topic(region: u32) -> String {
    format!("/doli/r{}/blocks/1", region)
}

/// Create a new GossipSub behaviour with tier-appropriate mesh parameters.
///
/// - Tier 1: mesh_n=20 (dense mesh for instant block propagation among 500 validators)
/// - Tier 2: mesh_n=8 (moderate mesh within regional shards)
/// - Tier 3: mesh_n=4 (light mesh for header-only validation)
/// - Tier 0 (default/legacy): mesh_n=6 (backward compatible)
pub fn new_gossipsub_for_tier(keypair: &Keypair, tier: u8) -> Result<Gossipsub, GossipError> {
    let message_id_fn = |message: &Message| {
        let mut hasher = DefaultHasher::new();
        message.data.hash(&mut hasher);
        MessageId::from(hasher.finish().to_be_bytes().to_vec())
    };

    let (mesh_n, mesh_n_low, mesh_n_high, mesh_outbound_min) = match tier {
        1 => (20, 15, 30, 5), // Tier 1: dense mesh
        2 => (8, 5, 15, 3),   // Tier 2: moderate mesh
        3 => (4, 2, 8, 1),    // Tier 3: light mesh
        _ => (6, 4, 12, 2),   // Default/legacy
    };

    let config = ConfigBuilder::default()
        .heartbeat_interval(Duration::from_secs(1))
        .validation_mode(ValidationMode::Strict)
        .message_id_fn(message_id_fn)
        .mesh_n(mesh_n)
        .mesh_n_low(mesh_n_low)
        .mesh_n_high(mesh_n_high)
        .mesh_outbound_min(mesh_outbound_min)
        .gossip_lazy(6)
        .gossip_factor(0.25)
        .history_length(5)
        .history_gossip(3)
        .max_transmit_size(1024 * 1024)
        .duplicate_cache_time(Duration::from_secs(60))
        .build()
        .map_err(|e| GossipError::Config(e.to_string()))?;

    Gossipsub::new(MessageAuthenticity::Signed(keypair.clone()), config)
        .map_err(|e| GossipError::Init(e.to_string()))
}

/// Subscribe to tier-appropriate topics.
///
/// - Tier 1: All base topics + TIER1_BLOCKS_TOPIC + ATTESTATION_TOPIC + HEADERS_TOPIC
/// - Tier 2: All base topics + regional block topic + ATTESTATION_TOPIC + HEADERS_TOPIC
/// - Tier 3: HEADERS_TOPIC + PRODUCERS_TOPIC + VOTES_TOPIC (no full blocks)
/// - Tier 0 (default): All base topics (backward compatible)
pub fn subscribe_to_topics_for_tier(
    gossipsub: &mut Gossipsub,
    tier: u8,
    region: Option<u32>,
) -> Result<(), GossipError> {
    // Base topics (all tiers get producers and votes for governance)
    let producers_topic = IdentTopic::new(PRODUCERS_TOPIC);
    let votes_topic = IdentTopic::new(VOTES_TOPIC);
    gossipsub
        .subscribe(&producers_topic)
        .map_err(|e| GossipError::Subscribe(format!("producers: {}", e)))?;
    gossipsub
        .subscribe(&votes_topic)
        .map_err(|e| GossipError::Subscribe(format!("votes: {}", e)))?;

    match tier {
        1 => {
            // Tier 1: full blocks + tier1 blocks + attestations + headers + heartbeats + txs
            for (name, topic_str) in [
                ("blocks", BLOCKS_TOPIC),
                ("txs", TRANSACTIONS_TOPIC),
                ("heartbeats", HEARTBEATS_TOPIC),
                ("t1_blocks", TIER1_BLOCKS_TOPIC),
                ("attestations", ATTESTATION_TOPIC),
                ("headers", HEADERS_TOPIC),
            ] {
                let topic = IdentTopic::new(topic_str);
                gossipsub
                    .subscribe(&topic)
                    .map_err(|e| GossipError::Subscribe(format!("{}: {}", name, e)))?;
            }
        }
        2 => {
            // Tier 2: full blocks + regional topic + attestations + headers + heartbeats + txs
            for (name, topic_str) in [
                ("blocks", BLOCKS_TOPIC),
                ("txs", TRANSACTIONS_TOPIC),
                ("heartbeats", HEARTBEATS_TOPIC),
                ("attestations", ATTESTATION_TOPIC),
                ("headers", HEADERS_TOPIC),
            ] {
                let topic = IdentTopic::new(topic_str);
                gossipsub
                    .subscribe(&topic)
                    .map_err(|e| GossipError::Subscribe(format!("{}: {}", name, e)))?;
            }
            // Subscribe to regional topic
            if let Some(r) = region {
                let rtopic = IdentTopic::new(region_topic(r));
                gossipsub
                    .subscribe(&rtopic)
                    .map_err(|e| GossipError::Subscribe(format!("region_{}: {}", r, e)))?;
            }
        }
        3 => {
            // Tier 3: headers only (no full blocks, no heartbeats)
            let headers = IdentTopic::new(HEADERS_TOPIC);
            gossipsub
                .subscribe(&headers)
                .map_err(|e| GossipError::Subscribe(format!("headers: {}", e)))?;
        }
        _ => {
            // Default/legacy: subscribe to all base topics
            subscribe_to_topics(gossipsub)?;
            return Ok(());
        }
    }

    Ok(())
}

/// Create a new GossipSub behaviour
pub fn new_gossipsub(keypair: &Keypair) -> Result<Gossipsub, GossipError> {
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
        // Mesh parameters
        .mesh_n(6) // Target number of peers in mesh
        .mesh_n_low(4) // Minimum peers in mesh
        .mesh_n_high(12) // Maximum peers in mesh
        .mesh_outbound_min(2) // Minimum outbound peers in mesh
        // Gossip parameters
        .gossip_lazy(6) // Peers to gossip to
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

/// Publish a block header to the lightweight headers topic (all tiers)
pub fn publish_header(gossipsub: &mut Gossipsub, header_data: Vec<u8>) -> Result<(), GossipError> {
    let topic = IdentTopic::new(HEADERS_TOPIC);
    gossipsub
        .publish(topic, header_data)
        .map_err(|e| GossipError::Publish(format!("header: {}", e)))?;
    Ok(())
}

/// Publish a block to the Tier 1 dense mesh topic
pub fn publish_tier1_block(
    gossipsub: &mut Gossipsub,
    block_data: Vec<u8>,
) -> Result<(), GossipError> {
    let topic = IdentTopic::new(TIER1_BLOCKS_TOPIC);
    gossipsub
        .publish(topic, block_data)
        .map_err(|e| GossipError::Publish(format!("t1_block: {}", e)))?;
    Ok(())
}

/// Publish an attestation message
pub fn publish_attestation(
    gossipsub: &mut Gossipsub,
    attestation_data: Vec<u8>,
) -> Result<(), GossipError> {
    let topic = IdentTopic::new(ATTESTATION_TOPIC);
    gossipsub
        .publish(topic, attestation_data)
        .map_err(|e| GossipError::Publish(format!("attestation: {}", e)))?;
    Ok(())
}

/// Publish a block to a regional topic (Tier 2 sharding)
pub fn publish_to_region(
    gossipsub: &mut Gossipsub,
    region: u32,
    block_data: Vec<u8>,
) -> Result<(), GossipError> {
    let topic = IdentTopic::new(region_topic(region));
    gossipsub
        .publish(topic, block_data)
        .map_err(|e| GossipError::Publish(format!("region_{}: {}", region, e)))?;
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
