use std::time::Duration;

use libp2p::gossipsub::{
    Behaviour as Gossipsub, Config, ConfigBuilder, IdentTopic, Message, MessageAuthenticity,
    MessageId, PeerScoreParams, PeerScoreThresholds, TopicScoreParams, ValidationMode,
};
use libp2p::identity::Keypair;

use crate::service::LaneBudgets;

use super::staleness::ATTEST_STALE_SLOTS;
use super::{
    GossipError, MeshConfig, ATTESTATION_TOPIC, BLOCKS_TOPIC, PRODUCERS_TOPIC, TRANSACTIONS_TOPIC,
    VOTES_TOPIC,
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

/// Production gossipsub duplicate-cache lifetime.
///
/// INC-I-237: was 60 s. On mainnet 2026-10-09 stale messages re-arrived
/// 60-119 s after first sight, just past the 60 s window; gossipsub treated
/// them as new and the fleet re-forwarded them in a loop until the unbounded
/// per-peer send queues OOM-killed every node. 300 s drops those copies
/// before validation, so they are never forwarded. This vendor cache (keyed
/// by BLAKE3 of the raw data) is the only re-delivery dedup.
///
/// RESOURCE COST: count-capped at [`GOSSIP_DUPLICATE_CACHE_MAX_ENTRIES`];
/// measured 137 B per id at the cap (table pinned, ~29 MiB per node).
/// Local per-node policy: no consensus, wire or block-content change.
pub const GOSSIP_DUPLICATE_CACHE_TIME: Duration = Duration::from_secs(300);

// Not shorter than the deleted 180 s app cache, nor the attestation age window
// (every network's default slot is `consensus::SLOT_DURATION`).
const _: () = assert!(
    GOSSIP_DUPLICATE_CACHE_TIME.as_secs() >= 180
        && GOSSIP_DUPLICATE_CACHE_TIME.as_secs()
            >= ATTEST_STALE_SLOTS as u64 * doli_core::consensus::SLOT_DURATION
);

/// Count cap of the vendor duplicate cache (INC-I-237 M7.2): oldest-first eviction.
pub const GOSSIP_DUPLICATE_CACHE_MAX_ENTRIES: usize = 218_000;

/// Count cap of the vendor peer-score `deliveries` cache (INC-I-237 M7.2).
pub const PEER_SCORE_DELIVERIES_MAX_ENTRIES: usize = 50_000;

// Pinned table (M7.1 model): buckets x (slot + 1) + expiry deque x 40 + per-entry heap.
const fn capped_cache_model_bytes(cap: usize, slot: usize, per_entry: usize) -> usize {
    (cap * 8 / 7).next_power_of_two() * (slot + 1) + cap.next_power_of_two() * 40 + cap * per_entry
}

const fn rebuild_slack_ok(cap: usize) -> bool {
    (cap * 8 / 7).next_power_of_two() * 7 / 8 >= cap * 105 / 100
}

// Honest 500 ids/s deduplicated for the full TTL; soak flood (1,804 ids/s) kept >= 120 s.
const _: () = assert!(
    GOSSIP_DUPLICATE_CACHE_MAX_ENTRIES as u64 >= 500 * GOSSIP_DUPLICATE_CACHE_TIME.as_secs()
        && GOSSIP_DUPLICATE_CACHE_MAX_ENTRIES >= 1_804 * 120
);

// >= 5% rebuild slack; modelled bytes inside the 32 MiB / 20 MiB shares of the 64 MiB budget.
// Deliveries slot 120 B and 187 B heap per record: M7.2 calibration of the M7.1 soak row.
const _: () = assert!(
    rebuild_slack_ok(GOSSIP_DUPLICATE_CACHE_MAX_ENTRIES)
        && rebuild_slack_ok(PEER_SCORE_DELIVERIES_MAX_ENTRIES)
        && capped_cache_model_bytes(
            GOSSIP_DUPLICATE_CACHE_MAX_ENTRIES,
            std::mem::size_of::<(MessageId, std::time::Instant)>(),
            40,
        ) <= 32 << 20
        && capped_cache_model_bytes(PEER_SCORE_DELIVERIES_MAX_ENTRIES, 120, 187) <= 20 << 20
);

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

/// Production GossipSub behaviour with configurable mesh parameters.
///
/// Mesh parameters are loaded from `NetworkParams` via env vars / `.env` / defaults.
/// Devnet uses larger mesh (12/8/48) for --no-dht star topology.
/// Mainnet/testnet use standard mesh (6/4/12) with DHT peer rotation.
pub fn new_gossipsub(keypair: &Keypair, mesh: &MeshConfig) -> Result<Gossipsub, GossipError> {
    build_gossipsub(
        keypair,
        gossipsub_config(mesh, GOSSIP_DUPLICATE_CACHE_TIME)?,
    )
}

/// [`new_gossipsub`] with a custom `duplicate_cache_time`, for integration
/// tests that exercise dedup expiry in seconds.
pub fn new_gossipsub_with_cache_time(
    keypair: &Keypair,
    mesh: &MeshConfig,
    duplicate_cache_time: Duration,
) -> Result<Gossipsub, GossipError> {
    build_gossipsub(keypair, gossipsub_config(mesh, duplicate_cache_time)?)
}

/// INV-NETWORK-002 for the [`new_gossipsub`] config and the node's event lanes.
pub(crate) fn assert_production_hardening(
    mesh: &MeshConfig,
    lanes: &LaneBudgets,
) -> Result<(), GossipError> {
    let config = gossipsub_config(mesh, GOSSIP_DUPLICATE_CACHE_TIME)?;
    assert_gossip_hardening_invariant(&config, lanes.is_bounded())
}

/// The one gossipsub config.
fn gossipsub_config(
    mesh: &MeshConfig,
    duplicate_cache_time: Duration,
) -> Result<Config, GossipError> {
    // INC-I-012 F9: BLAKE3 of the raw data (platform-independent), first 20 bytes.
    let message_id_fn = |message: &Message| {
        let hash = crypto::hash::hash(&message.data);
        MessageId::from(hash.as_bytes()[..20].to_vec())
    };

    ConfigBuilder::default()
        .heartbeat_interval(Duration::from_secs(1))
        .validation_mode(ValidationMode::Strict)
        // INC-I-114: every message is held un-forwarded until the application
        // reports its validation result.
        .validate_messages()
        .message_id_fn(message_id_fn)
        .mesh_n(mesh.mesh_n)
        .mesh_n_low(mesh.mesh_n_low)
        .mesh_n_high(mesh.mesh_n_high)
        .mesh_outbound_min((mesh.mesh_n / 3).max(1).min(mesh.mesh_n / 2)) // capped at mesh_n/2 (gossipsub constraint)
        .gossip_lazy(mesh.gossip_lazy)
        // INC-I-015: 50% ensures blocks reach non-mesh peers in 1-2 heartbeats.
        .gossip_factor(0.50)
        .history_length(5)
        .history_gossip(3)
        .max_transmit_size(GOSSIP_MAX_TRANSMIT_SIZE)
        .duplicate_cache_time(duplicate_cache_time)
        .duplicate_cache_max_entries(GOSSIP_DUPLICATE_CACHE_MAX_ENTRIES)
        .peer_score_deliveries_max_entries(PEER_SCORE_DELIVERIES_MAX_ENTRIES)
        // Flood publish: send OUR messages to ALL peers, not just the mesh.
        .flood_publish(true)
        .build()
        .map_err(|e| GossipError::Config(e.to_string()))
}

/// The behaviour for `config`, with the production topic scores.
fn build_gossipsub(keypair: &Keypair, config: Config) -> Result<Gossipsub, GossipError> {
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

/// Subscribe to the five consumed topics (headers/heartbeats are not, D3).
pub fn subscribe_to_topics(gossipsub: &mut Gossipsub) -> Result<(), GossipError> {
    for topic in [
        BLOCKS_TOPIC,
        TRANSACTIONS_TOPIC,
        PRODUCERS_TOPIC,
        VOTES_TOPIC,
        ATTESTATION_TOPIC,
    ] {
        gossipsub
            .subscribe(&IdentTopic::new(topic))
            .map_err(|e| GossipError::Subscribe(format!("{topic}: {e}")))?;
    }
    Ok(())
}
