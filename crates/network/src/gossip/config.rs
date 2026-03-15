use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::time::Duration;

use libp2p::gossipsub::{
    Behaviour as Gossipsub, ConfigBuilder, IdentTopic, Message, MessageAuthenticity, MessageId,
    PeerScoreParams, PeerScoreThresholds, TopicHash, TopicScoreParams, ValidationMode,
};
use libp2p::identity::Keypair;
use tracing::debug;

use super::{
    region_topic, GossipError, MeshConfig, ATTESTATION_TOPIC, BLOCKS_TOPIC, HEADERS_TOPIC,
    HEARTBEATS_TOPIC, PRODUCERS_TOPIC, TIER1_BLOCKS_TOPIC, TRANSACTIONS_TOPIC, VOTES_TOPIC,
};

/// Safety-critical topics that must NEVER be unsubscribed on a producing node.
/// Unsubscribing from BLOCKS_TOPIC causes the node to stop receiving blocks
/// via gossip, leading to permanent desync.
const PROTECTED_TOPICS: &[&str] = &[BLOCKS_TOPIC, TRANSACTIONS_TOPIC];

/// Maximum mesh_n when scaling dynamically with producer count.
/// Matches Tier 1 (dense validator mesh) value.
const MESH_N_CAP: usize = 20;

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
            topics.push(ATTESTATION_TOPIC.to_string());
        }
        _ => {
            // Tier 0 / legacy — includes attestations so all nodes can track finality
            topics.extend([
                BLOCKS_TOPIC.to_string(),
                TRANSACTIONS_TOPIC.to_string(),
                HEARTBEATS_TOPIC.to_string(),
                HEADERS_TOPIC.to_string(),
                ATTESTATION_TOPIC.to_string(),
            ]);
        }
    }
    topics
}

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

/// Compute gossipsub mesh parameters dynamically based on active producer count.
///
/// Formula: mesh_n = min(active_producers - 1, MESH_N_CAP)
///
/// This ensures all producers are in each other's eager-push mesh for small networks,
/// eliminating lazy-gossip propagation delay that causes missed slots and sync loops.
/// For large networks (>21 producers), caps at MESH_N_CAP=20 to bound bandwidth.
pub fn compute_dynamic_mesh(active_producers: usize) -> MeshConfig {
    // Need at least 2 producers for meaningful mesh; fall back to defaults
    if active_producers <= 1 {
        return MeshConfig {
            mesh_n: 6,
            mesh_n_low: 4,
            mesh_n_high: 12,
            gossip_lazy: 6,
        };
    }

    let mesh_n = (active_producers - 1).min(MESH_N_CAP);
    let mesh_n_low = (mesh_n * 3 / 4).max(1);
    let mesh_n_high = mesh_n * 2;
    let gossip_lazy = mesh_n.max(6);

    MeshConfig {
        mesh_n,
        mesh_n_low,
        mesh_n_high,
        gossip_lazy,
    }
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
        .mesh_outbound_min((mesh.mesh_n / 3).max(1).min(mesh.mesh_n / 2)) // Scale with mesh_n, capped at mesh_n/2 (gossipsub constraint)
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

    let mut gossipsub = Gossipsub::new(MessageAuthenticity::Signed(keypair.clone()), config)
        .map_err(|e| GossipError::Init(e.to_string()))?;

    // REQ-NET-002: Peer scoring to prioritize producers in the mesh.
    // Producers naturally deliver first-seen blocks (they create them).
    // Non-producers only relay. This makes GossipSub preferentially keep
    // producers in the mesh without any explicit "is_producer" check.
    let mut topic_scores = std::collections::HashMap::new();
    topic_scores.insert(
        IdentTopic::new(BLOCKS_TOPIC).hash(),
        TopicScoreParams {
            topic_weight: 1.0,
            first_message_deliveries_weight: 10.0,
            first_message_deliveries_decay: 0.5,
            first_message_deliveries_cap: 100.0,
            ..Default::default()
        },
    );
    let peer_score_params = PeerScoreParams {
        topics: topic_scores,
        ..Default::default()
    };
    gossipsub
        .with_peer_score(peer_score_params, PeerScoreThresholds::default())
        .map_err(GossipError::Config)?;

    Ok(gossipsub)
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

    let attestation_topic = IdentTopic::new(ATTESTATION_TOPIC);
    gossipsub
        .subscribe(&attestation_topic)
        .map_err(|e| GossipError::Subscribe(format!("attestations: {}", e)))?;

    Ok(())
}
