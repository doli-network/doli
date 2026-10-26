//! Behaviour-level event handling.
//!
//! Processes gossipsub messages, Kademlia routing updates, Identify exchanges,
//! status/sync request-response protocol messages, and NAT traversal events.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use libp2p::{autonat, gossipsub, identify, kad, request_response, Multiaddr, PeerId, Swarm};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::behaviour::{DoliBehaviour, DoliBehaviourEvent};
use crate::config::NetworkConfig;
use crate::peer::PeerInfo;
use crate::peer_cache::PeerCache;
use crate::rate_limit::RateLimiter;

use super::event_sink::EventSink;
use super::helpers::{all_addresses_routable, is_routable_address};
use super::types::{NetworkEvent, GENESIS_MISMATCH_COOLDOWN_SECS};
use crate::protocols::status::{CURRENT_PROTOCOL_VERSION, MIN_PEER_PROTOCOL_VERSION};

/// Handle behaviour events
#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_behaviour_event(
    event: DoliBehaviourEvent,
    swarm: &mut Swarm<DoliBehaviour>,
    event_tx: &EventSink,
    peers: &Arc<RwLock<HashMap<PeerId, PeerInfo>>>,
    config: &NetworkConfig,
    peer_cache_path: &Option<PathBuf>,
    rate_limiter: &mut RateLimiter,
    genesis_mismatch_cooldown: &mut HashMap<PeerId, Instant>,
    stale_peer_ids: &mut HashMap<PeerId, Instant>,
    local_tip: &Arc<super::gossip_intake::LocalTip>,
    shed_metrics: &std::sync::Arc<super::backpressure::GossipShedMetrics>,
    memory_shed_flag: &Arc<AtomicBool>,
) {
    match event {
        DoliBehaviourEvent::Gossipsub(gossipsub::Event::Message {
            propagation_source,
            message_id,
            message,
        }) => super::gossip_intake::handle_gossip_message(
            swarm,
            event_tx,
            config,
            rate_limiter,
            local_tip,
            shed_metrics,
            memory_shed_flag,
            propagation_source,
            message_id,
            message,
        ),

        DoliBehaviourEvent::Kademlia(kad::Event::RoutingUpdated {
            peer, addresses, ..
        }) => {
            // INC-I-050: Filter non-routable addresses from Kademlia routing table.
            //
            // The Identify handler (below) filters listen_addrs via is_routable_address()
            // before calling kademlia.add_address(). But Kademlia's internal FIND_NODE
            // responses propagate addresses through the DHT routing table exchange,
            // bypassing the Identify filter entirely.
            //
            // Without this check, a node with a loopback address (e.g., 127.0.0.1:30300)
            // can poison the DHT, causing all nodes to self-dial and generating connection
            // error churn that degrades the gossip mesh. See INC-I-050 for the full
            // causal chain: DHT poisoning → event loop saturation → gossip mesh collapse
            // → 28 empty slots on mainnet.
            if !all_addresses_routable(addresses.iter(), config.network_id) {
                warn!(
                    "[DHT] Removing peer {} — non-routable address in Kademlia routing table",
                    peer
                );
                swarm.behaviour_mut().kademlia.remove_peer(&peer);
            } else {
                info!("[DHT] Routing updated for peer: {}", peer);
            }
            // Don't auto-dial on RoutingUpdated — this is the main trigger for
            // the simultaneous-dial race on co-located nodes (rust-libp2p#752).
        }
        DoliBehaviourEvent::Kademlia(_) => {
            // Other Kademlia events (query progress, etc.) — no action needed
        }

        DoliBehaviourEvent::Identify(identify::Event::Received { peer_id, info }) => {
            debug!(
                "Received identify info from {}: {:?}",
                peer_id, info.agent_version
            );

            // Filter out non-routable addresses (loopback, unspecified, link-local)
            // so remote peers don't learn 127.0.0.1 from multi-node hosts.
            let routable_addrs: Vec<Multiaddr> = info
                .listen_addrs
                .into_iter()
                .filter(|addr| {
                    let routable = is_routable_address(addr, config.network_id);
                    if !routable {
                        debug!("Filtered non-routable address from {}: {}", peer_id, addr);
                    }
                    routable
                })
                .collect();

            // Cache the peer's routable addresses for fast reconnection after restart.
            // Store address WITHOUT embedded peer ID — the peer_id field in CachedPeer
            // tracks identity separately, and peer IDs change after chain resets.
            if let Some(ref path) = peer_cache_path {
                if let Some(addr) = routable_addrs.first() {
                    let clean_addr = format!("{}", addr);
                    let mut cache = PeerCache::load(path).unwrap_or_default();
                    cache.add(&peer_id.to_string(), &clean_addr);
                    cache.save(path);
                }
            }

            // Add the peer's routable addresses to kademlia (unless DHT is disabled)
            // Skip our own peer ID — DHT-propagated self-addresses cause
            // self-connection loops (Noise handshake → "Local peer ID" rejection).
            let local_peer_id = *swarm.local_peer_id();
            if !config.no_dht && peer_id != local_peer_id && !stale_peer_ids.contains_key(&peer_id)
            {
                for addr in routable_addrs {
                    debug!("[DHT] Adding address for peer {}: {}", peer_id, addr);
                    swarm.behaviour_mut().kademlia.add_address(&peer_id, addr);
                }
                // Do NOT trigger kademlia.bootstrap() here. The periodic 60s timer
                // handles DHT refresh. Triggering on every identify event causes a
                // feedback loop: connect → identify → bootstrap → fetch stale entries
                // from peers → mismatch → reconnect → identify → bootstrap → ...
            }
        }

        DoliBehaviourEvent::Status(request_response::Event::Message { peer, message }) => {
            match message {
                request_response::Message::Request {
                    request, channel, ..
                } => {
                    debug!("Received status request from {}", peer);

                    // Validate network ID on incoming request
                    if request.network_id != config.network_id {
                        warn!(
                            "Network mismatch with peer {}: we are on network {}, they are on {}",
                            peer, config.network_id, request.network_id
                        );
                        event_tx.emit(NetworkEvent::NetworkMismatch {
                            peer_id: peer,
                            our_network_id: config.network_id,
                            their_network_id: request.network_id,
                        });
                        // Disconnect the peer
                        let _ = swarm.disconnect_peer_id(peer);
                        return;
                    }

                    // Genesis mismatch cooldown: silently reject peers we already know are on a different chain
                    if let Some(cooldown_start) = genesis_mismatch_cooldown.get(&peer) {
                        if cooldown_start.elapsed().as_secs() < GENESIS_MISMATCH_COOLDOWN_SECS {
                            let _ = swarm.disconnect_peer_id(peer);
                            return;
                        }
                        genesis_mismatch_cooldown.remove(&peer);
                    }

                    // Validate genesis hash
                    if request.genesis_hash != config.genesis_hash {
                        warn!("Genesis hash mismatch with peer {}: cooldown 1h", peer);
                        genesis_mismatch_cooldown.insert(peer, Instant::now());
                        event_tx.emit(NetworkEvent::GenesisMismatch { peer_id: peer });
                        let _ = swarm.disconnect_peer_id(peer);
                        return;
                    }

                    // Validate protocol version
                    if request.version < MIN_PEER_PROTOCOL_VERSION {
                        warn!(
                            "Protocol version mismatch with peer {}: we require >= {}, they report {}",
                            peer, MIN_PEER_PROTOCOL_VERSION, request.version
                        );
                        event_tx.emit(NetworkEvent::VersionMismatch {
                            peer_id: peer,
                            our_version: CURRENT_PROTOCOL_VERSION,
                            their_version: request.version,
                        });
                        let _ = swarm.disconnect_peer_id(peer);
                        return;
                    }

                    // Validate fork identity — disconnect peers with incompatible
                    // hard fork schedules. Hash::ZERO means old binary (tolerated).
                    if request.fork_id != crypto::Hash::ZERO && request.fork_id != config.fork_id {
                        warn!(
                            "Fork ID mismatch with peer {} (request): theirs={}, ours={} — disconnecting",
                            peer,
                            &request.fork_id.to_hex()[..16],
                            &config.fork_id.to_hex()[..16],
                        );
                        let _ = swarm.disconnect_peer_id(peer);
                        return;
                    }

                    event_tx.emit(NetworkEvent::StatusRequest {
                        peer_id: peer,
                        request,
                        channel,
                    });
                }
                request_response::Message::Response { response, .. } => {
                    debug!("Received status response from {}", peer);

                    // Validate network ID
                    if response.network_id != config.network_id {
                        warn!(
                            "Network mismatch with peer {}: we are on network {}, they are on {}",
                            peer, config.network_id, response.network_id
                        );
                        event_tx.emit(NetworkEvent::NetworkMismatch {
                            peer_id: peer,
                            our_network_id: config.network_id,
                            their_network_id: response.network_id,
                        });
                        let _ = swarm.disconnect_peer_id(peer);
                        return;
                    }

                    // Genesis mismatch cooldown: silently reject known-bad peers
                    if let Some(cooldown_start) = genesis_mismatch_cooldown.get(&peer) {
                        if cooldown_start.elapsed().as_secs() < GENESIS_MISMATCH_COOLDOWN_SECS {
                            let _ = swarm.disconnect_peer_id(peer);
                            return;
                        }
                        genesis_mismatch_cooldown.remove(&peer);
                    }

                    // Validate genesis hash
                    if response.genesis_hash != config.genesis_hash {
                        warn!("Genesis hash mismatch with peer {}: cooldown 1h", peer);
                        genesis_mismatch_cooldown.insert(peer, Instant::now());
                        event_tx.emit(NetworkEvent::GenesisMismatch { peer_id: peer });
                        let _ = swarm.disconnect_peer_id(peer);
                        return;
                    }

                    // Validate protocol version
                    if response.version < MIN_PEER_PROTOCOL_VERSION {
                        warn!(
                            "Protocol version mismatch with peer {}: we require >= {}, they report {}",
                            peer, MIN_PEER_PROTOCOL_VERSION, response.version
                        );
                        event_tx.emit(NetworkEvent::VersionMismatch {
                            peer_id: peer,
                            our_version: CURRENT_PROTOCOL_VERSION,
                            their_version: response.version,
                        });
                        let _ = swarm.disconnect_peer_id(peer);
                        return;
                    }

                    // Validate fork identity — disconnect peers with incompatible
                    // hard fork schedules. Hash::ZERO means old binary (tolerated).
                    if response.fork_id != crypto::Hash::ZERO && response.fork_id != config.fork_id
                    {
                        warn!(
                            "Fork ID mismatch with peer {} (response): theirs={}, ours={} — disconnecting",
                            peer,
                            &response.fork_id.to_hex()[..16],
                            &config.fork_id.to_hex()[..16],
                        );
                        let _ = swarm.disconnect_peer_id(peer);
                        return;
                    }

                    // Update peer info
                    let mut peers = peers.write().await;
                    if let Some(peer_info) = peers.get_mut(&peer) {
                        peer_info.best_height = response.best_height;
                        peer_info.best_hash = response.best_hash;
                        peer_info.touch();
                        // SCALE-T2-004: Mark producer peers so eviction logic
                        // protects them from being dropped at max_peers capacity.
                        if response.producer_pubkey.is_some() {
                            peer_info.is_producer = true;
                        }
                    }

                    event_tx.emit(NetworkEvent::PeerStatus {
                        peer_id: peer,
                        status: response,
                    });
                }
            }
        }

        DoliBehaviourEvent::Sync(request_response::Event::Message { peer, message }) => {
            match message {
                request_response::Message::Request {
                    request, channel, ..
                } => {
                    info!(
                        "[SYNC_DEBUG] Received sync request from peer={}, request={:?}",
                        peer, request
                    );
                    event_tx.emit(NetworkEvent::SyncRequest {
                        peer_id: peer,
                        request,
                        channel,
                    });
                }
                request_response::Message::Response { response, .. } => {
                    info!(
                        "[SYNC_DEBUG] Received sync response from peer={}, response_type={}",
                        peer,
                        response.type_name()
                    );
                    event_tx.emit(NetworkEvent::SyncResponse {
                        peer_id: peer,
                        response,
                    });
                }
            }
        }

        DoliBehaviourEvent::Txfetch(request_response::Event::Message { peer, message }) => {
            match message {
                request_response::Message::Request {
                    request, channel, ..
                } => {
                    debug!(
                        "Received tx fetch request from {} ({} hashes)",
                        peer,
                        request.hashes.len()
                    );
                    event_tx.emit(NetworkEvent::TxFetchRequest {
                        peer_id: peer,
                        hashes: request.hashes,
                        channel,
                    });
                }
                request_response::Message::Response { response, .. } => {
                    debug!(
                        "Received tx fetch response from {} ({} txs)",
                        peer,
                        response.transactions.len()
                    );
                    event_tx.emit(NetworkEvent::TxFetchResponse {
                        peer_id: peer,
                        transactions: response.transactions,
                    });
                }
            }
        }

        DoliBehaviourEvent::Txfetch(request_response::Event::OutboundFailure {
            peer,
            error,
            ..
        }) => {
            debug!("TxFetch outbound failure to {}: {:?}", peer, error);
        }

        DoliBehaviourEvent::Txfetch(request_response::Event::InboundFailure {
            peer,
            error,
            ..
        }) => {
            debug!("TxFetch inbound failure from {}: {:?}", peer, error);
        }

        DoliBehaviourEvent::Txfetch(_) => {}

        DoliBehaviourEvent::RelayClient(event) => {
            info!("[RELAY] Client: {:?}", event);
        }

        DoliBehaviourEvent::RelayServer(event) => {
            info!("[RELAY] Server: {:?}", event);
        }

        DoliBehaviourEvent::Dcutr(event) => {
            info!("[DCUTR] {:?}", event);
        }

        DoliBehaviourEvent::Autonat(autonat::Event::StatusChanged { new, .. }) => match new {
            autonat::NatStatus::Public(addr) => {
                info!("[NAT] Public address detected: {}", addr);
            }
            autonat::NatStatus::Private => {
                warn!("[NAT] Behind NAT — relying on relay for connectivity");
            }
            autonat::NatStatus::Unknown => {
                debug!("[NAT] NAT status unknown, waiting for probes");
            }
        },
        DoliBehaviourEvent::Autonat(_) => {}

        DoliBehaviourEvent::Gossipsub(gossipsub::Event::Subscribed { peer_id, topic }) => {
            debug!(
                "[GOSSIP_SUB] peer_subscribed peer={} topic={}",
                peer_id, topic
            );
        }

        DoliBehaviourEvent::Gossipsub(gossipsub::Event::Unsubscribed { peer_id, topic }) => {
            debug!(
                "[GOSSIP_SUB] peer_unsubscribed peer={} topic={}",
                peer_id, topic
            );
        }

        DoliBehaviourEvent::Gossipsub(gossipsub::Event::GossipsubNotSupported { peer_id }) => {
            warn!("[GOSSIP_SUB] gossipsub_not_supported peer={}", peer_id);
        }

        _ => {}
    }
}
