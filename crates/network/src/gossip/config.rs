use std::time::Duration;

use libp2p::gossipsub::{
    Behaviour as Gossipsub, Config, ConfigBuilder, IdentTopic, Message, MessageAuthenticity,
    MessageId, PeerScoreParams, PeerScoreThresholds, TopicScoreParams, ValidationMode,
};
use libp2p::identity::Keypair;

use super::{
    GossipError, MeshConfig, ATTESTATION_TOPIC, BLOCKS_TOPIC, HEADERS_TOPIC, HEARTBEATS_TOPIC,
    PRODUCERS_TOPIC, TRANSACTIONS_TOPIC, VOTES_TOPIC,
};

/// Maximum message size that gossipsub will transmit (bytes).
///
/// Any block whose serialized size exceeds this value is rejected by
/// `gossipsub.publish()` with `PublishError::MessageTooLarge`. This cap MUST
/// be >= the largest block the network produces (`BASE_BLOCK_SIZE` for Era 0)
/// plus envelope overhead, or consensus-valid blocks cannot propagate via
/// gossip (INC-I-091). Sized to fit a full Era-0 block; Era 1+ blocks (>2 MB)
/// require announce-then-fetch propagation, not a larger gossip message.
///
/// Production gates actual block size to ~1 MB until
/// `NetworkParams::large_block_activation_height`, so raising this cap is a
/// transport-only change that ships safely to all nodes ahead of the AH.
pub const GOSSIP_MAX_TRANSMIT_SIZE: usize =
    doli_core::consensus::BASE_BLOCK_SIZE + doli_core::consensus::GOSSIP_ENVELOPE_MARGIN;

/// Maximum mesh_n value. Prevents over-meshing in very large networks.
const MESH_N_CAP: usize = 50;

/// Duplicate-cache-time at or below which the gossip config is considered
/// "aggressive" for dedup purposes (INV-NETWORK-002).
///
/// **Rationale**: libp2p-gossipsub deduplicates messages for this window.
/// After expiry, a re-received message is treated as new and (without
/// `validate_messages`) auto-forwarded to every mesh peer — creating a
/// re-forward storm that grows exponentially with the number of peers.
///
/// - **30s** is half the standard 60s default. At 30s or below, a stale block
///   that arrives once per slot (10s) will escape the dedup cache within 3
///   slots, triggering the re-forward amplification loop. Empirically, the
///   INC-I-114 fleet-wide OOM occurred with dedup=60s because flood_publish
///   was the aggression vector, not short dedup — but a future config with
///   dedup <= 30s AND no validation would reproduce the same shape faster.
/// - Values above 30s are "standard" dedup and do not independently trigger
///   the amplification loop (messages expire too slowly to re-enter during
///   normal gossip heartbeat cadence). However, flood_publish=true is
///   independently aggressive regardless of dedup time.
///
/// The threshold is a **documented, tested constant** — changing it requires
/// updating the boundary test and documenting the new rationale.
pub const AGGRESSIVE_DEDUP_THRESHOLD: Duration = Duration::from_secs(30);

/// Verify that a gossipsub [`Config`] satisfies INV-NETWORK-002: aggressive
/// propagation settings require both application-level validation and a
/// bounded event queue to prevent heap exhaustion.
///
/// # What is "aggressive"
///
/// A config is aggressive if:
/// - `flood_publish() == true` — the node sends every locally-published
///   message to ALL peers, not just the mesh subset, AND/OR
/// - `duplicate_cache_time() <= AGGRESSIVE_DEDUP_THRESHOLD` — the dedup
///   cache expires fast enough that stale messages re-enter the forwarding
///   pipeline within a few slots.
///
/// # Required mitigations (both must be present)
///
/// 1. `validate_messages() == true` — messages are held un-forwarded until
///    the application calls `report_message_validation_result()`. Without
///    this, libp2p auto-forwards every received message into its internal
///    unbounded `VecDeque`.
/// 2. `has_bounded_queue == true` — the block ingestion path uses a bounded,
///    load-shedding queue (M1 backpressure) so the application-side event
///    channel cannot grow without bound.
///
/// # Incident lineage
///
/// INC-I-009 (yamux buffer explosion), INC-I-014 (RAM explosion at 103+
/// nodes), INC-I-118 (gossip storm), INC-I-120 (gossip storm repeat),
/// INC-I-114 (stale-block re-forward OOM cascade). All 5 incidents share
/// the same root shape: unbounded internal queues + aggressive propagation.
///
/// # Errors
///
/// Returns `GossipError::Config` with a descriptive message naming which
/// mitigation half is missing and citing INV-NETWORK-002.
pub fn assert_gossip_hardening_invariant(
    cfg: &Config,
    has_bounded_queue: bool,
) -> Result<(), GossipError> {
    let aggressive =
        cfg.flood_publish() || cfg.duplicate_cache_time() <= AGGRESSIVE_DEDUP_THRESHOLD;

    if !aggressive {
        return Ok(());
    }

    // Aggressive config — both mitigations required.
    if !cfg.validate_messages() && !has_bounded_queue {
        return Err(GossipError::Config(format!(
            "INV-NETWORK-002 violation: aggressive gossip config \
             (flood_publish={}, dedup={}s) requires BOTH validate_messages \
             AND a bounded event queue, but NEITHER is present. Without \
             these, libp2p auto-reforwards stale messages into an unbounded \
             VecDeque -> heap exhaustion (INC-I-009/014/118/120/114).",
            cfg.flood_publish(),
            cfg.duplicate_cache_time().as_secs(),
        )));
    }

    if !cfg.validate_messages() {
        return Err(GossipError::Config(format!(
            "INV-NETWORK-002 violation: aggressive gossip config \
             (flood_publish={}, dedup={}s) requires validate_messages=true \
             to prevent auto-forwarding of stale messages. Call \
             .validate_messages() on the ConfigBuilder.",
            cfg.flood_publish(),
            cfg.duplicate_cache_time().as_secs(),
        )));
    }

    if !has_bounded_queue {
        return Err(GossipError::Config(format!(
            "INV-NETWORK-002 violation: aggressive gossip config \
             (flood_publish={}, dedup={}s) has validate_messages but \
             requires a bounded (load-shedding) event queue to cap \
             application-side backpressure. The block ingestion path \
             must use a capacity-limited channel with shed-on-full \
             (see backpressure.rs).",
            cfg.flood_publish(),
            cfg.duplicate_cache_time().as_secs(),
        )));
    }

    Ok(())
}

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
    // INC-I-012 F9: Message ID uses BLAKE3 (deterministic across platforms).
    // DefaultHasher is NOT guaranteed to produce the same output across Rust
    // versions or platforms (x86 vs ARM). If two nodes compute different IDs
    // for the same message, gossipsub breaks deduplication. BLAKE3 is already
    // a project dependency (via crypto crate) and is platform-independent.
    let message_id_fn = |message: &Message| {
        let hash = crypto::hash::hash(&message.data);
        // Use first 20 bytes of BLAKE3 hash as message ID (standard gossipsub practice)
        MessageId::from(hash.as_bytes()[..20].to_vec())
    };

    let config = ConfigBuilder::default()
        // Heartbeat interval
        .heartbeat_interval(Duration::from_secs(1))
        // Message validation
        .validation_mode(ValidationMode::Strict)
        // INC-I-114: Application-level validation — every received message is
        // held un-forwarded until the application calls
        // report_message_validation_result(). This prevents stale blocks from
        // being auto-forwarded after the dedup cache expires, which caused a
        // fleet-wide gossip amplification storm and OOM cascade.
        .validate_messages()
        // Message ID function
        .message_id_fn(message_id_fn)
        // Mesh parameters (from NetworkParams)
        .mesh_n(mesh.mesh_n)
        .mesh_n_low(mesh.mesh_n_low)
        .mesh_n_high(mesh.mesh_n_high)
        .mesh_outbound_min((mesh.mesh_n / 3).max(1).min(mesh.mesh_n / 2)) // Scale with mesh_n, capped at mesh_n/2 (gossipsub constraint)
        // Gossip parameters
        .gossip_lazy(mesh.gossip_lazy)
        // INC-I-015: 50% ensures blocks reach non-mesh peers in 1-2 heartbeats
        // instead of 3-4. At 106 nodes: 94 non-mesh × 0.5 = 47 IHAVE/heartbeat.
        .gossip_factor(0.50)
        // History
        .history_length(5)
        .history_gossip(3)
        // Message size limit — uses named constant for testability
        .max_transmit_size(GOSSIP_MAX_TRANSMIT_SIZE)
        // Duplicate cache time
        .duplicate_cache_time(Duration::from_secs(60))
        // Flood publish: send OUR messages to ALL peers, not just mesh.
        // Defensive — ensures our blocks/attestations reach everyone regardless
        // of mesh topology. At 42 nodes the bandwidth cost is negligible.
        .flood_publish(true)
        .build()
        .map_err(|e| GossipError::Config(e.to_string()))?;

    // INV-NETWORK-002: construction-time hardening gate — fail fast if
    // aggressive config is missing validation or bounded queue mitigations.
    // Production passes true for has_bounded_queue because the block
    // ingestion path uses M1 backpressure (enqueue_or_shed).
    assert_gossip_hardening_invariant(&config, true)?;

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
    // INC-I-012 F8: Add scoring for TRANSACTIONS and PRODUCERS topics.
    // Without topic scoring, mesh manipulation and invalid message flooding
    // on these topics face no penalty. Transactions use moderate weight
    // (high volume, low criticality). Producers use higher weight (GSet
    // flooding caused O(N*mesh_n) event storms at 106 nodes).
    topic_scores.insert(
        IdentTopic::new(TRANSACTIONS_TOPIC).hash(),
        TopicScoreParams {
            topic_weight: 0.5,
            first_message_deliveries_weight: 2.0,
            first_message_deliveries_decay: 0.5,
            first_message_deliveries_cap: 50.0,
            invalid_message_deliveries_weight: -10.0,
            invalid_message_deliveries_decay: 0.3,
            ..Default::default()
        },
    );
    topic_scores.insert(
        IdentTopic::new(PRODUCERS_TOPIC).hash(),
        TopicScoreParams {
            topic_weight: 0.7,
            first_message_deliveries_weight: 5.0,
            first_message_deliveries_decay: 0.5,
            first_message_deliveries_cap: 50.0,
            invalid_message_deliveries_weight: -20.0,
            invalid_message_deliveries_decay: 0.3,
            ..Default::default()
        },
    );
    topic_scores.insert(
        IdentTopic::new(ATTESTATION_TOPIC).hash(),
        TopicScoreParams {
            topic_weight: 1.0,
            first_message_deliveries_weight: 8.0,
            first_message_deliveries_decay: 0.5,
            first_message_deliveries_cap: 100.0,
            invalid_message_deliveries_weight: -10.0,
            invalid_message_deliveries_decay: 0.3,
            // Disable mesh_message_deliveries penalty. With 38 producers
            // sending 1 attestation per 10s block and decay=0.5/s, the
            // delivery counter converges to ~3-4, far below libp2p's
            // default threshold of 20. Every peer gets penalized →
            // mesh degenerates to random selection → N2/N6 stuck in
            // negative feedback loop.
            mesh_message_deliveries_weight: 0.0,
            mesh_message_deliveries_threshold: 0.0,
            mesh_message_deliveries_decay: 0.5,
            mesh_message_deliveries_cap: 0.0,
            mesh_message_deliveries_activation: std::time::Duration::from_secs(3600),
            ..Default::default()
        },
    );
    // AUDIT-GOSSIP-006: ip_colocation_factor_threshold defaults to 5 (Sybil
    // resistant). Set DOLI_IP_COLOCATION_THRESHOLD=500 for local testnets
    // with many nodes on 127.0.0.1. At threshold=1 with N co-located peers,
    // the penalty is -35*(N-1)^2 — 33+ peers on one IP exceed graylist (-16K).
    let ip_colocation_threshold: f64 = std::env::var("DOLI_IP_COLOCATION_THRESHOLD")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5.0);

    let peer_score_params = PeerScoreParams {
        topics: topic_scores,
        ip_colocation_factor_threshold: ip_colocation_threshold,
        ..Default::default()
    };
    gossipsub
        .with_peer_score(peer_score_params, PeerScoreThresholds::default())
        .map_err(GossipError::Config)?;

    Ok(gossipsub)
}

/// Create a GossipSub behaviour identical to [`new_gossipsub`] but with a
/// custom `duplicate_cache_time`. Intended for integration tests that need
/// short cache TTLs (e.g., 2-3s) to exercise dedup-expiry behavior without
/// waiting 60 seconds. Production code should use [`new_gossipsub`].
pub fn new_gossipsub_with_cache_time(
    keypair: &Keypair,
    mesh: &MeshConfig,
    duplicate_cache_time: Duration,
) -> Result<Gossipsub, GossipError> {
    let message_id_fn = |message: &Message| {
        let hash = crypto::hash::hash(&message.data);
        MessageId::from(hash.as_bytes()[..20].to_vec())
    };

    let config = ConfigBuilder::default()
        .heartbeat_interval(Duration::from_secs(1))
        .validation_mode(ValidationMode::Strict)
        // INC-I-114: validate_messages enabled (behavior-identical to production)
        .validate_messages()
        .message_id_fn(message_id_fn)
        .mesh_n(mesh.mesh_n)
        .mesh_n_low(mesh.mesh_n_low)
        .mesh_n_high(mesh.mesh_n_high)
        .mesh_outbound_min((mesh.mesh_n / 3).max(1).min(mesh.mesh_n / 2))
        .gossip_lazy(mesh.gossip_lazy)
        .gossip_factor(0.50)
        .history_length(5)
        .history_gossip(3)
        .max_transmit_size(GOSSIP_MAX_TRANSMIT_SIZE)
        .duplicate_cache_time(duplicate_cache_time)
        .flood_publish(true)
        .build()
        .map_err(|e| GossipError::Config(e.to_string()))?;

    // INV-NETWORK-002: construction-time hardening gate (same as production).
    // Test variant also uses bounded queue (true) since integration tests
    // exercise the same backpressure path.
    assert_gossip_hardening_invariant(&config, true)?;

    let mut gossipsub = Gossipsub::new(MessageAuthenticity::Signed(keypair.clone()), config)
        .map_err(|e| GossipError::Init(e.to_string()))?;

    let ip_colocation_threshold: f64 = std::env::var("DOLI_IP_COLOCATION_THRESHOLD")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5.0);

    let peer_score_params = PeerScoreParams {
        ip_colocation_factor_threshold: ip_colocation_threshold,
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
