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

    // Peer ID mismatch redial cooldown: tracks (address → last_redial_time).
    // Prevents exponential redial storms when stale DHT entries circulate
    // multiple old peer IDs for the same address after a chain reset.
    let mut mismatch_redial_cooldown: HashMap<String, Instant> = HashMap::new();

    // TX batching: buffer outbound transactions and flush every 100ms
    let mut tx_batch: Vec<Transaction> = Vec::new();
    let mut tx_flush = tokio::time::interval(Duration::from_millis(100));
    tx_flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            // Handle swarm events
            event = swarm.select_next_some() => {
                handle_swarm_event(event, &mut swarm, &event_tx, &peers, &config, &peer_cache_path, &mut rate_limiter, &mut genesis_mismatch_cooldown, &mut mismatch_redial_cooldown).await;
            }

            // Handle commands — intercept BroadcastTransaction for batching
            Some(command) = command_rx.recv() => {
                if let NetworkCommand::BroadcastTransaction(tx) = command {
                    tx_batch.push(tx);
                    if tx_batch.len() >= 50 {
                        let batch = std::mem::take(&mut tx_batch);
                        let data = crate::gossip::encode_tx_batch(&batch);
                        let topic = IdentTopic::new(TRANSACTIONS_TOPIC);
                        if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic, data) {
                            warn!("Failed to flush tx batch: {}", e);
                        }
                    }
                } else {
                    handle_command(command, &mut swarm, &config).await;
                }
            }

            // Flush buffered transactions every 100ms
            _ = tx_flush.tick() => {
                if !tx_batch.is_empty() {
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

            // Periodic rate limiter cleanup
            _ = rate_limit_cleanup.tick() => {
                rate_limiter.cleanup(Duration::from_secs(600));
                // Purge expired mismatch redial cooldowns (older than 60s)
                mismatch_redial_cooldown.retain(|_, last| last.elapsed() < Duration::from_secs(60));
            }
        }
    }
}
