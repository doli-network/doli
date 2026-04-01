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
use tracing::{debug, info, warn};

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
    dial_backoff: &mut HashMap<PeerId, (u32, Instant)>,
    eviction_cooldown: &mut HashMap<PeerId, Instant>,
    bootstrap_peers: &mut HashMap<PeerId, Instant>,
    stale_peer_ids: &mut HashMap<PeerId, Instant>,
) {
    match event {
        SwarmEvent::ConnectionEstablished {
            peer_id,
            endpoint,
            num_established,
            ..
        } => {
            let network_info = swarm.network_info();
            let conn_counters = network_info.connection_counters();
            let total_established = conn_counters.num_established();
            debug!(
                "[MEM-CONN] +conn peer={} num_established={} | total_conns={} (in={} out={})",
                peer_id,
                num_established,
                total_established,
                conn_counters.num_established_incoming(),
                conn_counters.num_established_outgoing()
            );
            // Milestone log every 10 connections
            if total_established.is_multiple_of(10) {
                info!(
                    "[MEM-CONN] connections={} (in={} out={}) pending_in={} pending_out={}",
                    total_established,
                    conn_counters.num_established_incoming(),
                    conn_counters.num_established_outgoing(),
                    conn_counters.num_pending_incoming(),
                    conn_counters.num_pending_outgoing()
                );
            }

            // REQ-SCALE-004: Reset dial backoff on successful connection
            dial_backoff.remove(&peer_id);

            // Only register peer on first connection (dedup)
            if num_established.get() == 1 {
                // INC-I-011: Check eviction cooldown BEFORE acquiring the peer table lock.
                // If this peer was recently evicted, disconnect immediately to break
                // the evict→reconnect→evict thrashing loop that causes RAM explosion
                // when network_nodes > max_peers.
                if let Some(evicted_at) = eviction_cooldown.get(&peer_id) {
                    if evicted_at.elapsed() < Duration::from_secs(120) {
                        let _ = swarm.disconnect_peer_id(peer_id);
                        return;
                    } else {
                        eviction_cooldown.remove(&peer_id);
                    }
                }

                // Genesis mismatch: peer on a different chain. Reject immediately
                // before the status handshake to avoid the full noise+yamux+status
                // cycle that repeats ~1/sec with zombie nodes.
                if let Some(mismatch_at) = genesis_mismatch_cooldown.get(&peer_id) {
                    if mismatch_at.elapsed()
                        < Duration::from_secs(super::types::GENESIS_MISMATCH_COOLDOWN_SECS)
                    {
                        let _ = swarm.disconnect_peer_id(peer_id);
                        return;
                    } else {
                        genesis_mismatch_cooldown.remove(&peer_id);
                    }
                }

                let mut peers = peers.write().await;
                if peers.len() >= config.max_peers {
                    // INC-I-014: Eviction rate limiting — max 5 evictions per 60s.
                    // When network has more nodes than max_peers, constant connect/evict
                    // cycles allocate ~456KB yamux+noise buffers per connection. The macOS
                    // allocator doesn't return freed pages → RSS grows monotonically.
                    // Rate limiting breaks the churn loop: refuse new peers instead of
                    // evicting when churn is already high.
                    let recent_evictions = eviction_cooldown
                        .values()
                        .filter(|t| t.elapsed() < Duration::from_secs(60))
                        .count();
                    if recent_evictions >= 5 {
                        let _ = swarm.disconnect_peer_id(peer_id);
                        return;
                    }

                    // SCALE-T2-004: Producer-aware peer eviction with grace period.
                    // When at max capacity, evict the lowest-gossipsub-scored peer,
                    // but NEVER evict a peer marked as a producer (is_producer=true).
                    //
                    // INC-I-016: Connection grace period + age tiebreaker.
                    // Two protections for non-producer peers (seeds, relays):
                    //
                    // 1. GRACE PERIOD (default 30s, tunable via DOLI_EVICTION_GRACE_SECS):
                    //    Never evict peers connected less than this duration.
                    //    Without this, a seed's outbound connection to a full node
                    //    is immediately evicted as the youngest peer — the seed can
                    //    never establish outbound connections. The grace period gives
                    //    new connections time to join the gossip mesh and build score.
                    //
                    // 2. AGE TIEBREAKER: When scores tie (common at 0.0), evict the
                    //    youngest connection outside the grace window. This protects
                    //    long-lived mesh participants from churn cascades.
                    let grace_secs = std::env::var("DOLI_EVICTION_GRACE_SECS")
                        .ok()
                        .and_then(|v| v.parse::<u64>().ok())
                        .unwrap_or(30);
                    let eviction_grace_period = Duration::from_secs(grace_secs);
                    let evictable = peers
                        .iter()
                        .filter(|(_, info)| {
                            !info.is_producer
                                && info.connected_at.elapsed() >= eviction_grace_period
                        })
                        .map(|(pid, info)| {
                            let score = swarm.behaviour().gossipsub.peer_score(pid).unwrap_or(0.0);
                            let age = info.connected_at.elapsed();
                            (*pid, score, age)
                        })
                        .min_by(|a, b| {
                            // Primary: lowest gossipsub score evicted first.
                            // Tiebreaker: youngest connection evicted first (LIFO).
                            a.1.partial_cmp(&b.1)
                                .unwrap_or(std::cmp::Ordering::Equal)
                                .then_with(|| a.2.cmp(&b.2))
                        });

                    if let Some((evict_id, score, _age)) = evictable {
                        info!(
                            "Peer table full ({}) — evicting {} (score={:.1}) for new peer {} | cooldowns={} bootstrap_slots={}",
                            config.max_peers, evict_id, score, peer_id,
                            eviction_cooldown.len(), bootstrap_peers.len()
                        );
                        peers.remove(&evict_id);
                        let _ = swarm.disconnect_peer_id(evict_id);
                        // INC-I-014: Remove evicted peer from Kademlia routing table.
                        // Without this, DHT bootstrap (every 60s) rediscovers evicted
                        // peers and re-dials them, creating O(N - max_peers) connection
                        // attempts per cycle. At 213 nodes: ~188 unnecessary dials/60s.
                        swarm.behaviour_mut().kademlia.remove_peer(&evict_id);
                        let _ = event_tx
                            .send(NetworkEvent::PeerDisconnected(evict_id))
                            .await;
                        // INC-I-011: Add evicted peer to cooldown so it can't
                        // immediately reconnect and trigger another eviction.
                        eviction_cooldown.insert(evict_id, Instant::now());
                    }
                }
                if peers.len() < config.max_peers {
                    let addr = endpoint.get_remote_address().to_string();
                    peers.insert(peer_id, PeerInfo::new(peer_id.to_string(), addr));

                    let _ = event_tx.send(NetworkEvent::PeerConnected(peer_id)).await;
                } else if config.bootstrap_slots > 0
                    && bootstrap_peers.len() < config.bootstrap_slots
                {
                    // Bootstrap-only: peer table is full but we have bootstrap headroom.
                    // Accept the connection temporarily for Kademlia DHT exchange.
                    // The peer gets Identify + Kademlia routing table entries, then we
                    // disconnect after 10s to free the slot. This solves the chicken-and-egg:
                    // new nodes need at least one connected peer to bootstrap DHT discovery.
                    bootstrap_peers.insert(peer_id, Instant::now());
                    info!(
                        "[BOOTSTRAP] Accepted {} as bootstrap-only peer ({}/{} bootstrap slots) — \
                         will disconnect after DHT exchange",
                        peer_id,
                        bootstrap_peers.len(),
                        config.bootstrap_slots
                    );
                }
            }
        }

        SwarmEvent::ConnectionClosed {
            peer_id,
            cause,
            num_established,
            ..
        } => {
            let network_info = swarm.network_info();
            let conn_counters = network_info.connection_counters();
            debug!(
                "[MEM-CONN] -conn peer={} cause={:?} remaining={} | total_conns={}",
                peer_id,
                cause,
                num_established,
                conn_counters.num_established()
            );

            // Only remove peer when no connections remain
            if num_established == 0 {
                // Clean up bootstrap-only tracking (before peer table, no event needed)
                if bootstrap_peers.remove(&peer_id).is_some() {
                    info!(
                        "[BOOTSTRAP] Bootstrap-only peer {} disconnected ({} bootstrap slots remaining)",
                        peer_id, bootstrap_peers.len()
                    );
                    rate_limiter.remove_peer(&peer_id);
                    return;
                }

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
                stale_peer_ids,
            )
            .await;
        }

        SwarmEvent::NewListenAddr { address, .. } => {
            info!("Listening on: {}", address);
            // If no explicit --external-address was configured, register routable
            // listen addresses as external so Identify only advertises public IPs.
            // Without this, Identify announces 127.0.0.1 to remote peers via DHT,
            // causing them to self-dial instead of connecting to us.
            if config.external_address.is_none() && is_routable_address(&address, config.network_id)
            {
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
                    // Also add to 24h blocklist to prevent DHT re-injection.
                    if let Some(old_id) = &peer_id {
                        swarm.behaviour_mut().kademlia.remove_peer(old_id);
                        stale_peer_ids.insert(*old_id, Instant::now());
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
                // REQ-SCALE-004: Dead peer exponential backoff.
                // Track failures per peer. After repeated failures, remove from
                // Kademlia to stop redial storms that starve the event loop.
                if let Some(pid) = &peer_id {
                    let (prev_count, _) = dial_backoff
                        .get(pid)
                        .copied()
                        .unwrap_or((0, Instant::now()));
                    let count = prev_count + 1;
                    let backoff_secs = std::cmp::min(300u64, 1u64 << count.min(8));
                    dial_backoff.insert(*pid, (count, Instant::now()));

                    if count >= 5 {
                        // Persistently failing — remove from Kademlia routing table
                        // to prevent automatic redial. Peer will be rediscovered via
                        // DHT bootstrap if it comes back online.
                        swarm.behaviour_mut().kademlia.remove_peer(pid);
                        warn!(
                            "Dead peer {:?}: {} failures, backoff {}s — removed from DHT",
                            pid, count, backoff_secs
                        );
                    } else {
                        warn!(
                            "Failed to connect to peer {:?}: {} (attempt {}, backoff {}s)",
                            pid, error, count, backoff_secs
                        );
                    }
                } else {
                    warn!("Failed to connect to peer {:?}: {}", peer_id, error);
                }
            }
        }

        SwarmEvent::IncomingConnectionError {
            local_addr,
            send_back_addr,
            error,
            ..
        } => {
            // Demoted to debug — these are expected at scale (duplicate connections,
            // max_established_per_peer rejections, Kademlia discovery races).
            debug!(
                "[MEM-CONN] Incoming connection REJECTED: {} (local={}, remote={})",
                error, local_addr, send_back_addr
            );
        }

        _ => {}
    }
}
