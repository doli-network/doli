use std::collections::HashSet;
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

/// Maximum mesh_n when scaling dynamically with network size.
/// Raised from 20 to 50 to support 100+ node networks.
/// For 106 nodes: mesh_n=11, for 500: mesh_n=23, for 1000: mesh_n=32.
const MESH_N_CAP: usize = 50;

/// Create a new GossipSub behaviour with tier-appropriate mesh parameters.
///
/// - Tier 1: mesh_n=20 (dense mesh for instant block propagation among 500 validators)
/// - Tier 2: mesh_n=8 (moderate mesh within regional shards)
/// - Tier 3: mesh_n=4 (light mesh for header-only validation)
/// - Tier 0 (default/legacy): mesh_n=6 (backward compatible)
pub fn new_gossipsub_for_tier(keypair: &Keypair, tier: u8) -> Result<Gossipsub, GossipError> {
    // Ethereum-aligned: SHA256 message IDs
    let message_id_fn = |message: &Message| {
        let hash = crypto::hash::hash(&message.data);
        MessageId::from(hash.as_bytes()[..20].to_vec())
    };

    // Tier mesh sizes aligned with Ethereum's D=8 as the baseline
    let (mesh_n, mesh_n_low, mesh_n_high, mesh_outbound_min) = match tier {
        1 => (12, 8, 20, 4), // Tier 1: dense (Ethereum-like for validators)
        2 => (8, 6, 12, 3),  // Tier 2: standard (matches Ethereum D=8)
        3 => (4, 2, 8, 1),   // Tier 3: light mesh
        _ => (8, 6, 12, 3),  // Default: Ethereum standard D=8
    };

    let config = ConfigBuilder::default()
        .heartbeat_interval(Duration::from_millis(700)) // Ethereum: 0.7s
        .validation_mode(ValidationMode::Strict)
        .message_id_fn(message_id_fn)
        .mesh_n(mesh_n)
        .mesh_n_low(mesh_n_low)
        .mesh_n_high(mesh_n_high)
        .mesh_outbound_min(mesh_outbound_min)
        .gossip_lazy(6)
        .gossip_factor(0.25)
        .flood_publish(true)
        .history_length(6)
        .history_gossip(3)
        .max_transmit_size(1024 * 1024)
        .duplicate_cache_time(Duration::from_secs(330))
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

/// Compute gossipsub mesh parameters dynamically based on total network peers.
///
/// For small networks (≤20): mesh_n = total_peers - 1 (full mesh, every node in eager-push).
/// For large networks (>20): mesh_n = sqrt(total_peers) * 1.5, capped at MESH_N_CAP.
/// sqrt(N) ensures O(log N) propagation hops even with 1000+ nodes.
///
/// Examples: 8 peers → mesh_n=7, 50 peers → mesh_n=11, 106 peers → mesh_n=16, 500 → mesh_n=34.
pub fn compute_dynamic_mesh(total_peers: usize) -> MeshConfig {
    if total_peers <= 1 {
        return MeshConfig {
            mesh_n: 8,       // Ethereum baseline D=8
            mesh_n_low: 6,   // Ethereum D_low=6
            mesh_n_high: 12, // Ethereum D_high=12
            gossip_lazy: 6,
        };
    }

    let mesh_n = if total_peers <= 20 {
        // Small network: full mesh (all peers in eager-push)
        total_peers - 1
    } else {
        // Large network: sqrt scaling for O(log N) propagation
        let sqrt_n = (total_peers as f64).sqrt();
        (sqrt_n * 1.5).ceil() as usize
    }
    .clamp(8, MESH_N_CAP); // Min D=8 (Ethereum baseline)

    let mesh_n_low = (mesh_n * 3 / 4).max(6); // Ethereum D_low=6 minimum
    let mesh_n_high = (mesh_n * 3 / 2).min(MESH_N_CAP * 2); // 1.5x mesh_n
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
/// Parameters aligned with Ethereum's consensus layer (Lighthouse/Prysm)
/// for battle-tested reliability. Key differences from previous config:
/// - SHA256 message IDs (collision-resistant, matches Ethereum)
/// - 700ms heartbeat (30% faster mesh maintenance, matches Ethereum)
/// - Flood publishing for self-originated messages
/// - Full P1-P4 peer scoring with harsh invalid message penalties
/// - Longer seen_ttl (330s vs 60s) to prevent message re-delivery
/// - Scoring on blocks AND attestation topics
pub fn new_gossipsub(keypair: &Keypair, mesh: &MeshConfig) -> Result<Gossipsub, GossipError> {
    // Ethereum-style message ID: SHA256(data)[:20] — collision-resistant.
    // Previous DefaultHasher (SipHash) was non-cryptographic and could collide,
    // causing valid blocks to be silently dropped as "duplicates."
    let message_id_fn = |message: &Message| {
        let hash = crypto::hash::hash(&message.data);
        MessageId::from(hash.as_bytes()[..20].to_vec())
    };

    let config = ConfigBuilder::default()
        // Heartbeat: 700ms (Ethereum uses 0.7s for faster mesh repair)
        .heartbeat_interval(Duration::from_millis(700))
        // Message validation
        .validation_mode(ValidationMode::Strict)
        // Cryptographic message ID
        .message_id_fn(message_id_fn)
        // Mesh parameters (from NetworkParams)
        .mesh_n(mesh.mesh_n)
        .mesh_n_low(mesh.mesh_n_low)
        .mesh_n_high(mesh.mesh_n_high)
        .mesh_outbound_min((mesh.mesh_n / 3).max(1).min(mesh.mesh_n / 2))
        // Gossip parameters
        .gossip_lazy(mesh.gossip_lazy)
        .gossip_factor(0.25) // Gossip to 25% of non-mesh peers (Ethereum: same)
        // Flood publishing: send self-originated messages to ALL connected peers,
        // not just mesh peers. Critical for block producers — ensures their blocks
        // propagate even if the mesh is temporarily degraded.
        .flood_publish(true)
        // History: 6 windows (Ethereum: 6), gossip last 3
        .history_length(6)
        .history_gossip(3)
        // Message size limit (1MB for blocks)
        .max_transmit_size(1024 * 1024)
        // Seen cache: 330s (~471 heartbeats at 700ms). Ethereum uses 550 heartbeats
        // (~385s). Prevents re-delivery of messages during partitions.
        .duplicate_cache_time(Duration::from_secs(330))
        .build()
        .map_err(|e| GossipError::Config(e.to_string()))?;

    let mut gossipsub = Gossipsub::new(MessageAuthenticity::Signed(keypair.clone()), config)
        .map_err(|e| GossipError::Init(e.to_string()))?;

    // =========================================================================
    // PEER SCORING — Ethereum-aligned P1-P4 parameters
    //
    // Ethereum's scoring is what makes its mesh resilient:
    // - P1: time_in_mesh — reward long-lived mesh connections
    // - P2: first_message_deliveries — reward peers who deliver blocks first
    // - P3: mesh_message_deliveries — penalize freeloading mesh peers
    // - P4: invalid_message_deliveries — harshly punish bad data
    // - IP colocation — penalize Sybil from single machine
    // =========================================================================
    let mut topic_scores = std::collections::HashMap::new();

    // BLOCKS topic — highest weight, harshest penalties (Ethereum: beacon_block)
    topic_scores.insert(
        IdentTopic::new(BLOCKS_TOPIC).hash(),
        TopicScoreParams {
            topic_weight: 0.8, // Ethereum: 0.8
            // P1: time in mesh — reward stable connections
            time_in_mesh_weight: 0.5,
            time_in_mesh_quantum: Duration::from_secs(12), // 1 slot
            time_in_mesh_cap: 300.0,
            // P2: first message deliveries — reward block originators
            first_message_deliveries_weight: 1.0,
            first_message_deliveries_decay: 0.631, // ~12s half-life at 700ms heartbeat
            first_message_deliveries_cap: 23.0,    // Ethereum: 23
            // P3: mesh message deliveries — penalize freeloaders
            mesh_message_deliveries_weight: -0.717, // Ethereum: -0.717
            mesh_message_deliveries_decay: 0.631,
            mesh_message_deliveries_threshold: 0.5,
            mesh_message_deliveries_cap: 5.0,
            mesh_message_deliveries_activation: Duration::from_secs(384), // ~1 epoch
            mesh_message_deliveries_window: Duration::from_millis(2000),
            // P4: invalid messages — HARSH penalty (Ethereum: -140x)
            invalid_message_deliveries_weight: -140.0,
            invalid_message_deliveries_decay: 0.997, // Slow decay — bad actors stay punished
            ..Default::default()
        },
    );

    // ATTESTATION topic — moderate weight
    topic_scores.insert(
        IdentTopic::new(ATTESTATION_TOPIC).hash(),
        TopicScoreParams {
            topic_weight: 0.5,
            first_message_deliveries_weight: 1.0,
            first_message_deliveries_decay: 0.631,
            first_message_deliveries_cap: 23.0,
            invalid_message_deliveries_weight: -50.0,
            invalid_message_deliveries_decay: 0.997,
            ..Default::default()
        },
    );

    // TRANSACTIONS topic — lower weight but still scored
    topic_scores.insert(
        IdentTopic::new(TRANSACTIONS_TOPIC).hash(),
        TopicScoreParams {
            topic_weight: 0.3,
            first_message_deliveries_weight: 0.5,
            first_message_deliveries_decay: 0.631,
            first_message_deliveries_cap: 50.0,
            invalid_message_deliveries_weight: -20.0,
            invalid_message_deliveries_decay: 0.997,
            ..Default::default()
        },
    );

    let peer_score_params = PeerScoreParams {
        topics: topic_scores,
        // IP colocation penalty: penalize many peers from same IP (Sybil defense).
        // Threshold raised from 10 to 500: with threshold=10, ALL peers get
        // graylisted at 33+ nodes on the same IP (devnet/testnet on 127.0.0.1),
        // killing gossip entirely and causing immediate forks. The bond system
        // provides Sybil protection — IP colocation is a secondary signal that
        // should only trigger for extreme cases (datacenter-scale colocation).
        ip_colocation_factor_weight: -35.0,
        ip_colocation_factor_threshold: 500.0,
        // Behaviour penalty: punish protocol violations
        behaviour_penalty_weight: -16.0, // Ethereum: -15.92
        behaviour_penalty_threshold: 6.0,
        behaviour_penalty_decay: 0.631,
        // Decay: 12s interval (1 slot), matching Ethereum
        decay_interval: Duration::from_secs(12),
        decay_to_zero: 0.01,
        // Retain score for ~1 hour after disconnect
        retain_score: Duration::from_secs(3600),
        ..Default::default()
    };

    // Scoring thresholds — Ethereum-inspired but scaled for smaller network.
    // These determine when a peer gets restricted:
    // - gossip_threshold: below this, no gossip exchanged
    // - publish_threshold: below this, flood-published msgs not sent
    // - graylist_threshold: below this, all RPCs ignored
    // - accept_px_threshold: must be above this to accept peer exchange
    let thresholds = PeerScoreThresholds {
        gossip_threshold: -4000.0,          // Ethereum: -4000
        publish_threshold: -8000.0,         // Ethereum: -8000
        graylist_threshold: -16000.0,       // Ethereum: -16000
        accept_px_threshold: 100.0,         // Ethereum: 100
        opportunistic_graft_threshold: 5.0, // Ethereum: 5
    };

    gossipsub
        .with_peer_score(peer_score_params, thresholds)
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
