//! GossipSub for block and transaction propagation
//!
//! Implements efficient message propagation using libp2p GossipSub protocol.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::time::Duration;

use libp2p::gossipsub::{
    Behaviour as Gossipsub, ConfigBuilder, IdentTopic, Message, MessageAuthenticity, MessageId,
    TopicHash, ValidationMode,
};
use libp2p::identity::Keypair;
use tracing::debug;

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

/// Return the set of topic strings appropriate for the given tier.
///
/// Single source of truth for tier→topic mapping. Used by
/// `reconfigure_topics_for_tier()` to compute the subscription delta.
pub fn topics_for_tier(tier: u8, region: Option<u32>) -> Vec<String> {
    let mut topics = vec![PRODUCERS_TOPIC.to_string(), VOTES_TOPIC.to_string()];
    match tier {
        1 => {
            topics.extend([
                BLOCKS_TOPIC.to_string(),
                TRANSACTIONS_TOPIC.to_string(),
                HEARTBEATS_TOPIC.to_string(),
                TIER1_BLOCKS_TOPIC.to_string(),
                ATTESTATION_TOPIC.to_string(),
                HEADERS_TOPIC.to_string(),
            ]);
        }
        2 => {
            topics.extend([
                BLOCKS_TOPIC.to_string(),
                TRANSACTIONS_TOPIC.to_string(),
                HEARTBEATS_TOPIC.to_string(),
                ATTESTATION_TOPIC.to_string(),
                HEADERS_TOPIC.to_string(),
            ]);
            if let Some(r) = region {
                topics.push(region_topic(r));
            }
        }
        3 => {
            topics.push(HEADERS_TOPIC.to_string());
        }
        _ => {
            // Tier 0 / legacy
            topics.extend([
                BLOCKS_TOPIC.to_string(),
                TRANSACTIONS_TOPIC.to_string(),
                HEARTBEATS_TOPIC.to_string(),
                HEADERS_TOPIC.to_string(),
            ]);
        }
    }
    topics
}

/// Safety-critical topics that must NEVER be unsubscribed on a producing node.
/// Unsubscribing from BLOCKS_TOPIC causes the node to stop receiving blocks
/// via gossip, leading to permanent desync.
const PROTECTED_TOPICS: &[&str] = &[BLOCKS_TOPIC, TRANSACTIONS_TOPIC];

/// Reconfigure gossipsub subscriptions for a tier change.
///
/// Performs a delta operation:
/// 1. Computes desired topic set for the new tier
/// 2. Unsubscribes from topics not in the desired set (except protected topics)
/// 3. Subscribes to new topics via `subscribe_to_topics_for_tier()`
///
/// SAFETY: BLOCKS_TOPIC and TRANSACTIONS_TOPIC are never unsubscribed regardless
/// of tier. A node without blocks is a dead node. Tier 3 header-only mode is only
/// safe for non-producing light clients (not yet supported).
///
/// Safe to call multiple times with the same tier (idempotent).
pub fn reconfigure_topics_for_tier(
    gossipsub: &mut Gossipsub,
    tier: u8,
    region: Option<u32>,
) -> Result<(), GossipError> {
    // Build desired topic hashes
    let desired: HashSet<TopicHash> = topics_for_tier(tier, region)
        .iter()
        .map(|s| IdentTopic::new(s).hash())
        .collect();

    // Build protected set
    let protected: HashSet<TopicHash> = PROTECTED_TOPICS
        .iter()
        .map(|s| IdentTopic::new(*s).hash())
        .collect();

    // Unsubscribe from topics not in the desired set, but NEVER unsubscribe protected topics
    let current: Vec<TopicHash> = gossipsub.topics().cloned().collect();
    for topic_hash in &current {
        if !desired.contains(topic_hash) && !protected.contains(topic_hash) {
            debug!("Unsubscribing from topic: {}", topic_hash);
            let topic = IdentTopic::new(topic_hash.as_str());
            let _ = gossipsub.unsubscribe(&topic);
        }
    }

    // Subscribe to desired topics (idempotent — already-subscribed is a no-op)
    subscribe_to_topics_for_tier(gossipsub, tier, region)
}

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

    let headers_topic = IdentTopic::new(HEADERS_TOPIC);
    gossipsub
        .subscribe(&headers_topic)
        .map_err(|e| GossipError::Subscribe(format!("headers: {}", e)))?;

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

use doli_core::Transaction;

/// Version prefix for batched transaction messages.
/// Must not collide with the first byte of a bincode-serialized Transaction
/// (version field: u32 LE, so 0x01 for v1, 0x02 for v2, etc.).
const TX_MSG_BATCH: u8 = 0xBA;

/// Encode a batch of transactions with version prefix.
///
/// Format: `[0x01][u32 count LE][u32 len1 LE][tx1 bytes][u32 len2 LE][tx2 bytes]...`
pub fn encode_tx_batch(transactions: &[Transaction]) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.push(TX_MSG_BATCH);
    buf.extend_from_slice(&(transactions.len() as u32).to_le_bytes());
    for tx in transactions {
        let data = tx.serialize();
        buf.extend_from_slice(&(data.len() as u32).to_le_bytes());
        buf.extend_from_slice(&data);
    }
    buf
}

/// Decode a transaction message. Handles both single (legacy) and batched formats.
///
/// - If the first byte is `0x01`, decodes as a batch.
/// - Otherwise, attempts legacy single-tx bincode deserialization.
/// - Returns `None` on empty input or decode failure.
pub fn decode_tx_message(data: &[u8]) -> Option<Vec<Transaction>> {
    if data.is_empty() {
        return None;
    }

    if data[0] == TX_MSG_BATCH {
        // Batch format
        if data.len() < 5 {
            return None;
        }
        let count = u32::from_le_bytes(data[1..5].try_into().ok()?) as usize;
        if count == 0 {
            return None;
        }
        let mut txs = Vec::with_capacity(count);
        let mut offset = 5;
        for _ in 0..count {
            if offset + 4 > data.len() {
                return None;
            }
            let len = u32::from_le_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
            offset += 4;
            if offset + len > data.len() {
                return None;
            }
            let tx = Transaction::deserialize(&data[offset..offset + len])?;
            txs.push(tx);
            offset += len;
        }
        Some(txs)
    } else {
        // Legacy single-tx format
        Transaction::deserialize(data).map(|tx| vec![tx])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_topic_constants() {
        assert_eq!(BLOCKS_TOPIC, "/doli/blocks/1");
        assert_eq!(TRANSACTIONS_TOPIC, "/doli/txs/1");
        assert_eq!(PRODUCERS_TOPIC, "/doli/producers/1");
        assert_eq!(VOTES_TOPIC, "/doli/votes/1");
        assert_eq!(HEARTBEATS_TOPIC, "/doli/heartbeats/1");
        assert_eq!(TIER1_BLOCKS_TOPIC, "/doli/t1/blocks/1");
        assert_eq!(HEADERS_TOPIC, "/doli/headers/1");
        assert_eq!(ATTESTATION_TOPIC, "/doli/attestations/1");
    }

    #[test]
    fn test_region_topic_format() {
        assert_eq!(region_topic(0), "/doli/r0/blocks/1");
        assert_eq!(region_topic(1), "/doli/r1/blocks/1");
        assert_eq!(region_topic(42), "/doli/r42/blocks/1");
    }

    #[test]
    fn test_tier1_gossipsub_creation() {
        let keypair = libp2p::identity::Keypair::generate_ed25519();
        let gs = new_gossipsub_for_tier(&keypair, 1);
        assert!(gs.is_ok());
    }

    #[test]
    fn test_tier2_gossipsub_creation() {
        let keypair = libp2p::identity::Keypair::generate_ed25519();
        let gs = new_gossipsub_for_tier(&keypair, 2);
        assert!(gs.is_ok());
    }

    #[test]
    fn test_tier3_gossipsub_creation() {
        let keypair = libp2p::identity::Keypair::generate_ed25519();
        let gs = new_gossipsub_for_tier(&keypair, 3);
        assert!(gs.is_ok());
    }

    #[test]
    fn test_default_tier_gossipsub_creation() {
        let keypair = libp2p::identity::Keypair::generate_ed25519();
        let gs = new_gossipsub_for_tier(&keypair, 0);
        assert!(gs.is_ok());
    }

    #[test]
    fn test_mesh_config_default() {
        let config = MeshConfig {
            mesh_n: 6,
            mesh_n_low: 4,
            mesh_n_high: 12,
            gossip_lazy: 6,
        };
        assert!(config.mesh_n >= config.mesh_n_low);
        assert!(config.mesh_n <= config.mesh_n_high);
    }

    #[test]
    fn test_gossip_error_display() {
        let e = GossipError::Config("bad config".into());
        assert!(e.to_string().contains("bad config"));
        let e = GossipError::Subscribe("topic failed".into());
        assert!(e.to_string().contains("topic failed"));
    }

    #[test]
    fn test_tx_batch_roundtrip() {
        let tx1 = doli_core::Transaction::new_coinbase(100, crypto::Hash::ZERO, 0);
        let tx2 = doli_core::Transaction::new_coinbase(200, crypto::Hash::ZERO, 1);

        let encoded = encode_tx_batch(&[tx1.clone(), tx2.clone()]);
        assert_eq!(encoded[0], TX_MSG_BATCH);

        let decoded = decode_tx_message(&encoded).expect("decode should succeed");
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].hash(), tx1.hash());
        assert_eq!(decoded[1].hash(), tx2.hash());
    }

    #[test]
    fn test_tx_single_legacy_decode() {
        let tx = doli_core::Transaction::new_coinbase(500, crypto::Hash::ZERO, 42);
        let raw = tx.serialize();

        let decoded = decode_tx_message(&raw).expect("legacy decode should succeed");
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].hash(), tx.hash());
    }

    #[test]
    fn test_tx_batch_empty_returns_none() {
        assert!(decode_tx_message(&[]).is_none());
        // Batch prefix with count=0
        let mut data = vec![TX_MSG_BATCH];
        data.extend_from_slice(&0u32.to_le_bytes());
        assert!(decode_tx_message(&data).is_none());
    }

    #[test]
    fn test_topics_for_tier0_is_legacy() {
        let topics = topics_for_tier(0, None);
        assert!(topics.contains(&BLOCKS_TOPIC.to_string()));
        assert!(topics.contains(&TRANSACTIONS_TOPIC.to_string()));
        assert!(topics.contains(&HEARTBEATS_TOPIC.to_string()));
        assert!(topics.contains(&HEADERS_TOPIC.to_string()));
        assert!(topics.contains(&PRODUCERS_TOPIC.to_string()));
        assert!(topics.contains(&VOTES_TOPIC.to_string()));
        // Tier 0 should NOT have tier1-specific topics
        assert!(!topics.contains(&TIER1_BLOCKS_TOPIC.to_string()));
        assert!(!topics.contains(&ATTESTATION_TOPIC.to_string()));
    }

    #[test]
    fn test_topics_for_tier1_includes_tier1_blocks() {
        let topics = topics_for_tier(1, None);
        assert!(topics.contains(&TIER1_BLOCKS_TOPIC.to_string()));
        assert!(topics.contains(&ATTESTATION_TOPIC.to_string()));
        assert!(topics.contains(&BLOCKS_TOPIC.to_string()));
        assert_eq!(topics.len(), 8); // producers, votes, blocks, txs, heartbeats, t1_blocks, attestations, headers
    }

    #[test]
    fn test_topics_for_tier3_headers_only() {
        let topics = topics_for_tier(3, None);
        assert!(topics.contains(&HEADERS_TOPIC.to_string()));
        assert!(topics.contains(&PRODUCERS_TOPIC.to_string()));
        assert!(topics.contains(&VOTES_TOPIC.to_string()));
        assert!(!topics.contains(&BLOCKS_TOPIC.to_string()));
        assert!(!topics.contains(&TRANSACTIONS_TOPIC.to_string()));
        assert!(!topics.contains(&HEARTBEATS_TOPIC.to_string()));
        assert_eq!(topics.len(), 3);
    }

    #[test]
    fn test_reconfigure_tier_unsubscribes() {
        let keypair = libp2p::identity::Keypair::generate_ed25519();
        let mut gs = new_gossipsub_for_tier(&keypair, 0).unwrap();

        // Start with Tier 0 (legacy) subscriptions
        subscribe_to_topics_for_tier(&mut gs, 0, None).unwrap();
        let initial_count = gs.topics().count();
        assert_eq!(initial_count, 6); // blocks, txs, heartbeats, headers, producers, votes

        // Reconfigure to Tier 3 (header-only)
        // BLOCKS_TOPIC and TRANSACTIONS_TOPIC are protected — never unsubscribed
        reconfigure_topics_for_tier(&mut gs, 3, None).unwrap();
        let final_count = gs.topics().count();
        // headers + producers + votes + blocks(protected) + txs(protected) = 5
        assert_eq!(final_count, 5);
    }

    #[test]
    fn test_protected_topics_never_unsubscribed() {
        let keypair = libp2p::identity::Keypair::generate_ed25519();
        let mut gs = new_gossipsub_for_tier(&keypair, 0).unwrap();

        // Subscribe to all Tier 0 topics
        subscribe_to_topics_for_tier(&mut gs, 0, None).unwrap();

        // Even reconfiguring to Tier 3 must keep blocks and txs
        reconfigure_topics_for_tier(&mut gs, 3, None).unwrap();

        let subscribed: Vec<String> = gs.topics().map(|t| t.to_string()).collect();
        assert!(
            subscribed.contains(&BLOCKS_TOPIC.to_string()),
            "BLOCKS_TOPIC must never be unsubscribed"
        );
        assert!(
            subscribed.contains(&TRANSACTIONS_TOPIC.to_string()),
            "TRANSACTIONS_TOPIC must never be unsubscribed"
        );
    }
}
