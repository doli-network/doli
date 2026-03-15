//! Swarm-level event handling.
//!
//! Processes connection lifecycle events, listen address changes, and
//! outgoing connection errors. Delegates behaviour events to the
//! behaviour_events module.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use libp2p::swarm::SwarmEvent;
use libp2p::{PeerId, Swarm};
use tokio::sync::{mpsc, RwLock};
use tracing::{info, warn};

use crate::behaviour::{DoliBehaviour, DoliBehaviourEvent};
use crate::config::NetworkConfig;
use crate::peer::PeerInfo;
use crate::peer_cache::PeerCache;
use crate::rate_limit::RateLimiter;

use super::behaviour_events::handle_behaviour_event;
use super::helpers::{is_routable_address, strip_p2p_suffix};
use super::types::NetworkEvent;

/// Handle swarm events
#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_swarm_event(
    event: SwarmEvent<DoliBehaviourEvent>,
    swarm: &mut Swarm<DoliBehaviour>,
    event_tx: &mpsc::Sender<NetworkEvent>,
    peers: &Arc<RwLock<HashMap<PeerId, PeerInfo>>>,
    config: &NetworkConfig,
    peer_cache_path: &Option<PathBuf>,
    rate_limiter: &mut RateLimiter,
    genesis_mismatch_cooldown: &mut HashMap<PeerId, Instant>,
    mismatch_redial_cooldown: &mut HashMap<String, Instant>,
) {
    match event {
        SwarmEvent::ConnectionEstablished {
            peer_id,
            endpoint,
            num_established,
            ..
        } => {
            info!(
                "Connected to peer: {} via {:?} (num_established={})",
                peer_id, endpoint, num_established
            );

            // Only register peer on first connection (dedup)
            if num_established.get() == 1 {
                let mut peers = peers.write().await;
                if peers.len() < config.max_peers {
                    let addr = endpoint.get_remote_address().to_string();
                    peers.insert(peer_id, PeerInfo::new(peer_id.to_string(), addr));

                    let _ = event_tx.send(NetworkEvent::PeerConnected(peer_id)).await;
                }
            }
        }

        SwarmEvent::ConnectionClosed {
            peer_id,
            cause,
            num_established,
            ..
        } => {
            info!(
                "Connection closed to peer: {} cause: {:?} (num_established={})",
                peer_id, cause, num_established
            );

            // Only remove peer when no connections remain
            if num_established == 0 {
                let mut peers = peers.write().await;
                peers.remove(&peer_id);

                // Clean up rate limiter state for disconnected peer
                rate_limiter.remove_peer(&peer_id);

                let _ = event_tx.send(NetworkEvent::PeerDisconnected(peer_id)).await;
            }
        }

        SwarmEvent::Behaviour(behaviour_event) => {
            handle_behaviour_event(
                behaviour_event,
                swarm,
                event_tx,
                peers,
                config,
                peer_cache_path,
                rate_limiter,
                genesis_mismatch_cooldown,
            )
            .await;
        }

        SwarmEvent::NewListenAddr { address, .. } => {
            info!("Listening on: {}", address);
            // If no explicit --external-address was configured, register routable
            // listen addresses as external so Identify only advertises public IPs.
            // Without this, Identify announces 127.0.0.1 to remote peers via DHT,
            // causing them to self-dial instead of connecting to us.
            if config.external_address.is_none() && is_routable_address(&address) {
                swarm.add_external_address(address.clone());
                info!(
                    "Auto-registered routable listen address as external: {}",
                    address
                );
            }
        }

        SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
            // Handle peer ID mismatch: the node at this address has a new identity
            // (e.g., after a chain reset that wiped its libp2p key).
            //
            // Strategy: ONLY clean up stale entries. Do NOT redial or add the new
            // peer ID to Kademlia here. The correct peer will be discovered naturally
            // through the periodic DHT bootstrap (every 60s) or incoming connections.
            //
            // Previous approach (update cache + immediate redial) caused amplification
            // storms: stale DHT entries from other peers would trigger mismatches
            // faster than we could clean them up, because adding the new address to
            // Kademlia and redialing just fed more entries into the distributed DHT
            // that other peers would re-propagate.
            if let libp2p::swarm::DialError::WrongPeerId { obtained, endpoint } = &error {
                if let libp2p::core::ConnectedPoint::Dialer { address, .. } = endpoint {
                    let clean_addr = strip_p2p_suffix(address);
                    let addr_key = clean_addr.to_string();

                    // Rate limit the WARN log to avoid log flooding
                    let now = Instant::now();
                    let should_log = !matches!(
                        mismatch_redial_cooldown.get(&addr_key),
                        Some(last) if now.duration_since(*last) < Duration::from_secs(30)
                    );

                    if should_log {
                        warn!(
                            "Peer ID mismatch at {} — expected {:?}, got {}. Removing stale entry.",
                            address, peer_id, obtained
                        );
                        mismatch_redial_cooldown.insert(addr_key, now);
                    }

                    // Remove the stale peer ID from Kademlia so it stops being
                    // returned in DHT queries and propagated to other peers.
                    if let Some(old_id) = &peer_id {
                        swarm.behaviour_mut().kademlia.remove_peer(old_id);
                    }

                    // Update peer cache: remove stale, record correct mapping.
                    // The cache is only used on restart — it won't trigger redials.
                    if let Some(ref path) = peer_cache_path {
                        let mut cache = PeerCache::load(path).unwrap_or_default();
                        if let Some(old_id) = &peer_id {
                            cache.remove(&old_id.to_string());
                        }
                        let full_addr = format!("{}/p2p/{}", clean_addr, obtained);
                        cache.add(&obtained.to_string(), &full_addr);
                        cache.save(path);
                    }

                    // Do NOT add to Kademlia or redial here. The correct peer will
                    // connect to us (incoming) or be discovered via DHT bootstrap.
                    // Adding here re-injects into the DHT propagation cycle.
                }
            } else {
                warn!("Failed to connect to peer {:?}: {}", peer_id, error);
            }
        }

        _ => {}
    }
}
