use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Duration;

use libp2p::gossipsub::{
    Behaviour as Gossipsub, ConfigBuilder, IdentTopic, Message, MessageAuthenticity, MessageId,
    PeerScoreParams, PeerScoreThresholds, TopicScoreParams, ValidationMode,
};
use libp2p::identity::Keypair;

use super::{
    GossipError, MeshConfig, ATTESTATION_TOPIC, BLOCKS_TOPIC, HEADERS_TOPIC, HEARTBEATS_TOPIC,
    PRODUCERS_TOPIC, TRANSACTIONS_TOPIC, VOTES_TOPIC,
};

/// Maximum mesh_n value. Prevents over-meshing in very large networks.
const MESH_N_CAP: usize = 50;

/// Compute dynamic gossipsub mesh parameters based on expected peer count.
///
/// Small networks (<=20): near-full mesh for reliability.
/// Large networks (>20): sqrt scaling for O(log N) propagation.
pub fn compute_dynamic_mesh(total_peers: usize) -> MeshConfig {
    if total_peers <= 1 {
        return MeshConfig {
            mesh_n: 8,
            mesh_n_low: 6,
            mesh_n_high: 12,
            gossip_lazy: 6,
        };
    }

    let mesh_n = if total_peers <= 20 {
        total_peers - 1
    } else {
        let sqrt_n = (total_peers as f64).sqrt();
        (sqrt_n * 1.5).ceil() as usize
    }
    .clamp(8, MESH_N_CAP);

    let mesh_n_low = (mesh_n * 3 / 4).max(6);
    let mesh_n_high = (mesh_n * 3 / 2).min(MESH_N_CAP * 2);
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
        // Default ip_colocation_factor_threshold is 1, which means at 33+ nodes
        // on the same IP (e.g., local testnet on 127.0.0.1), the penalty reaches
        // -35 × (33-1)² = -35,840, exceeding the graylist threshold of -16,000.
        // ALL gossip RPCs are silently dropped → blocks never propagate → forks.
        // Set to 500 to allow large local testnets without penalty.
        ip_colocation_factor_threshold: 500.0,
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
