//! Main swarm event loop.
//!
//! Runs the tokio::select! loop that drives the libp2p swarm, dispatching
//! events, commands, and periodic tasks.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use libp2p::gossipsub::IdentTopic;
use libp2p::{PeerId, Swarm};
use tokio::sync::{mpsc, RwLock};
use tracing::warn;

use crypto::Hash;
use doli_core::Transaction;

use crate::behaviour::DoliBehaviour;
use crate::config::NetworkConfig;
use crate::gossip::TRANSACTIONS_TOPIC;
use crate::peer::PeerInfo;
use crate::rate_limit::{RateLimitConfig, RateLimiter};

use super::command_handling::handle_command;
use super::swarm_events::handle_swarm_event;
use super::types::{NetworkCommand, NetworkEvent};

/// Run the swarm event loop
pub(super) async fn run_swarm(
    mut swarm: Swarm<DoliBehaviour>,
    mut command_rx: mpsc::Receiver<NetworkCommand>,
    event_tx: mpsc::Sender<NetworkEvent>,
    peers: Arc<RwLock<HashMap<PeerId, PeerInfo>>>,
    config: NetworkConfig,
    peer_cache_path: Option<PathBuf>,
) {
    // Periodic DHT refresh: re-run Kademlia bootstrap every 60s so that peers
    // discover each other through shared bootstrap nodes. Without this, a node
    // only queries the DHT on initial connection (identify event), missing peers
    // that join later.
    let mut dht_refresh = tokio::time::interval(std::time::Duration::from_secs(60));
    dht_refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // Rate limiter: protects against excessive gossip from individual peers
    let mut rate_limiter = RateLimiter::new(RateLimitConfig::default());

    // Cleanup stale rate-limit entries every 5 minutes
    let mut rate_limit_cleanup = tokio::time::interval(std::time::Duration::from_secs(300));
    rate_limit_cleanup.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // Genesis mismatch cooldown: peers that failed genesis check are silently
    // rejected for 1 hour. Prevents reconnection spam from stale-chain nodes.
    let mut genesis_mismatch_cooldown: HashMap<PeerId, Instant> = HashMap::new();

    // REQ-SCALE-004: Dead peer exponential backoff.
    // Track failed connection attempts per peer: (failure_count, last_attempt).
    // On each failure, backoff doubles: 1s → 2s → 4s → ... → 5min max.
    // Reset on successful connection.
    let mut dial_backoff: HashMap<PeerId, (u32, Instant)> = HashMap::new();

    // Peer ID mismatch redial cooldown: tracks (address → last_redial_time).
    // Prevents exponential redial storms when stale DHT entries circulate
    // multiple old peer IDs for the same address after a chain reset.
    let mut mismatch_redial_cooldown: HashMap<String, Instant> = HashMap::new();

    // INC-I-011: Eviction cooldown — recently evicted peers cannot reconnect
    // for 30 seconds, breaking the evict→reconnect→evict thrashing loop that
    // causes RAM explosion when network_nodes > max_peers.
    let mut eviction_cooldown: HashMap<PeerId, Instant> = HashMap::new();

    // Bootstrap-only peers: temporarily accepted for DHT exchange when peer table is full.
    // Each entry maps PeerId → connection time. After BOOTSTRAP_TTL, the connection is
    // closed to free the slot for the next bootstrapping node.
    let mut bootstrap_peers: HashMap<PeerId, Instant> = HashMap::new();
    const BOOTSTRAP_TTL: Duration = Duration::from_secs(10);
    let mut bootstrap_cleanup = tokio::time::interval(Duration::from_secs(5));
    bootstrap_cleanup.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // INC-I-014: Periodic connection budget log — shows steady-state connection
    // distribution every 60s. Makes limit enforcement visible during stress tests
    // without requiring lsof or external monitoring.
    let mut conn_budget_log = tokio::time::interval(Duration::from_secs(60));
    conn_budget_log.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // TX batching: buffer outbound transactions and flush every 100ms.
    // When tx_announce_enabled, we batch hashes (32 bytes each) instead of full txs.
    let mut tx_batch: Vec<Transaction> = Vec::new();
    let mut tx_announce_batch: Vec<Hash> = Vec::new();
    let tx_announce_enabled = config.tx_announce_enabled;
    let mut tx_flush = tokio::time::interval(Duration::from_millis(100));
    tx_flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            // Handle swarm events
            event = swarm.select_next_some() => {
                handle_swarm_event(event, &mut swarm, &event_tx, &peers, &config, &peer_cache_path, &mut rate_limiter, &mut genesis_mismatch_cooldown, &mut mismatch_redial_cooldown, &mut dial_backoff, &mut eviction_cooldown, &mut bootstrap_peers).await;
            }

            // Handle commands — intercept BroadcastTransaction for batching
            Some(command) = command_rx.recv() => {
                if let NetworkCommand::BroadcastTransaction(tx) = command {
                    if tx_announce_enabled {
                        // Announce mode: batch hashes, not full txs
                        tx_announce_batch.push(tx.hash());
                        if tx_announce_batch.len() >= 50 {
                            let hashes = std::mem::take(&mut tx_announce_batch);
                            let data = crate::gossip::encode_tx_announce(&hashes);
                            let topic = IdentTopic::new(TRANSACTIONS_TOPIC);
                            if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic, data) {
                                warn!("Failed to flush tx announce batch: {}", e);
                            }
                        }
                    } else {
                        // Legacy mode: batch full txs
                        tx_batch.push(tx);
                        if tx_batch.len() >= 50 {
                            let batch = std::mem::take(&mut tx_batch);
                            let data = crate::gossip::encode_tx_batch(&batch);
                            let topic = IdentTopic::new(TRANSACTIONS_TOPIC);
                            if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic, data) {
                                warn!("Failed to flush tx batch: {}", e);
                            }
                        }
                    }
                } else {
                    handle_command(command, &mut swarm, &config).await;
                }
            }

            // Flush buffered transactions every 100ms
            _ = tx_flush.tick() => {
                if tx_announce_enabled {
                    if !tx_announce_batch.is_empty() {
                        let hashes = std::mem::take(&mut tx_announce_batch);
                        let data = crate::gossip::encode_tx_announce(&hashes);
                        let topic = IdentTopic::new(TRANSACTIONS_TOPIC);
                        if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic, data) {
                            warn!("Failed to flush tx announce batch: {}", e);
                        }
                    }
                } else if !tx_batch.is_empty() {
                    let batch = std::mem::take(&mut tx_batch);
                    let data = crate::gossip::encode_tx_batch(&batch);
                    let topic = IdentTopic::new(TRANSACTIONS_TOPIC);
                    if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic, data) {
                        warn!("Failed to flush tx batch: {}", e);
                    }
                }
            }

            // Periodic DHT peer discovery
            _ = dht_refresh.tick() => {
                if !config.no_dht {
                    match swarm.behaviour_mut().kademlia.bootstrap() {
                        Ok(query_id) => {
                            tracing::info!("[DHT] Periodic bootstrap started (query={:?})", query_id);
                        }
                        Err(e) => {
                            warn!("[DHT] Bootstrap failed: {:?} — no peers in routing table", e);
                        }
                    }
                }
            }

            // Bootstrap-only peer cleanup: disconnect peers that have exceeded their TTL.
            // These peers were accepted solely for Kademlia DHT exchange when the peer
            // table was full. After 10s they've had enough time to discover other peers.
            _ = bootstrap_cleanup.tick() => {
                if !bootstrap_peers.is_empty() {
                    let expired: Vec<PeerId> = bootstrap_peers
                        .iter()
                        .filter(|(_, connected_at)| connected_at.elapsed() >= BOOTSTRAP_TTL)
                        .map(|(pid, _)| *pid)
                        .collect();
                    for pid in &expired {
                        bootstrap_peers.remove(pid);
                        let _ = swarm.disconnect_peer_id(*pid);
                    }
                    if !expired.is_empty() {
                        tracing::info!(
                            "[BOOTSTRAP] Disconnected {} expired bootstrap peers ({} still active)",
                            expired.len(),
                            bootstrap_peers.len()
                        );
                    }
                }
            }

            // Periodic rate limiter cleanup
            _ = rate_limit_cleanup.tick() => {
                rate_limiter.cleanup(Duration::from_secs(600));
                // Purge expired mismatch redial cooldowns (older than 60s)
                mismatch_redial_cooldown.retain(|_, last| last.elapsed() < Duration::from_secs(60));
                // Purge expired dial backoff entries (older than 10 minutes)
                dial_backoff.retain(|_, (_, last)| last.elapsed() < Duration::from_secs(600));
                // INC-I-011: Purge expired eviction cooldowns (older than 60s, 2x the cooldown)
                let before = eviction_cooldown.len();
                eviction_cooldown.retain(|_, evicted_at| evicted_at.elapsed() < Duration::from_secs(60));
                let purged = before - eviction_cooldown.len();
                if purged > 0 || !eviction_cooldown.is_empty() {
                    tracing::info!(
                        "[EVICTION] Cooldown: {} active, {} purged",
                        eviction_cooldown.len(), purged
                    );
                }
            }

            // INC-I-014: Periodic connection budget log
            _ = conn_budget_log.tick() => {
                let network_info = swarm.network_info();
                let cc = network_info.connection_counters();
                let peer_count = peers.read().await.len();
                // Count evictions in last 60s (recent cooldown entries = recent evictions)
                let recent_evictions = eviction_cooldown.values()
                    .filter(|t| t.elapsed() < Duration::from_secs(60))
                    .count();
                let pending_in = cc.num_pending_incoming();
                let pending_out = cc.num_pending_outgoing();
                tracing::info!(
                    "[MEM-CONN-BUDGET] peers={} established={} (in={} out={}) pending=(in={} out={}) bootstrap={} eviction_cooldown={} evictions_1m={}",
                    peer_count,
                    cc.num_established(),
                    cc.num_established_incoming(),
                    cc.num_established_outgoing(),
                    pending_in,
                    pending_out,
                    bootstrap_peers.len(),
                    eviction_cooldown.len(),
                    recent_evictions
                );
                // Warn if pending connections are high (INC-I-014: pending bypass detection)
                if pending_in + pending_out > 20 {
                    tracing::warn!(
                        "[MEM-CONN-BUDGET] HIGH PENDING: in={} out={} — may indicate connection limit bypass",
                        pending_in, pending_out
                    );
                }
                // Warn if eviction churn rate is high (INC-I-014: churn loop detection)
                if recent_evictions > 10 {
                    tracing::warn!(
                        "[MEM-CONN-BUDGET] HIGH EVICTION CHURN: {} evictions in last 60s — possible evict/reconnect loop",
                        recent_evictions
                    );
                }
            }
        }
    }
}
