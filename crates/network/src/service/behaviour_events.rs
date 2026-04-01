//! Behaviour-level event handling.
//!
//! Processes gossipsub messages, Kademlia routing updates, Identify exchanges,
//! status/sync request-response protocol messages, and NAT traversal events.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use libp2p::{autonat, gossipsub, identify, kad, request_response, Multiaddr, PeerId, Swarm};
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info, warn};

use doli_core::{decode_digest, decode_producer_set, is_legacy_bincode_format, Block, BlockHeader};

use crate::behaviour::{DoliBehaviour, DoliBehaviourEvent};
use crate::config::NetworkConfig;
use crate::gossip::{
    BLOCKS_TOPIC, HEADERS_TOPIC, HEARTBEATS_TOPIC, PRODUCERS_TOPIC, TRANSACTIONS_TOPIC, VOTES_TOPIC,
};
use crate::peer::PeerInfo;
use crate::peer_cache::PeerCache;
use crate::rate_limit::RateLimiter;
use crypto::PublicKey;

use super::helpers::is_routable_address;
use super::types::{NetworkEvent, GENESIS_MISMATCH_COOLDOWN_SECS};
use crate::protocols::status::{CURRENT_PROTOCOL_VERSION, MIN_PEER_PROTOCOL_VERSION};

/// Handle behaviour events
#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_behaviour_event(
    event: DoliBehaviourEvent,
    swarm: &mut Swarm<DoliBehaviour>,
    event_tx: &mpsc::Sender<NetworkEvent>,
    peers: &Arc<RwLock<HashMap<PeerId, PeerInfo>>>,
    config: &NetworkConfig,
    peer_cache_path: &Option<PathBuf>,
    rate_limiter: &mut RateLimiter,
    genesis_mismatch_cooldown: &mut HashMap<PeerId, Instant>,
    stale_peer_ids: &mut HashMap<PeerId, Instant>,
) {
    match event {
        DoliBehaviourEvent::Gossipsub(gossipsub::Event::Message {
            propagation_source,
            message_id: _,
            message,
        }) => {
            let topic = message.topic.as_str();
            let msg_size = message.data.len();
            debug!(
                "Received gossip message on topic {} from {}",
                topic, propagation_source
            );

            match topic {
                BLOCKS_TOPIC => {
                    if !rate_limiter.check_block(&propagation_source) {
                        warn!(
                            "Rate limit: dropping block from {} (block rate exceeded)",
                            propagation_source
                        );
                        return;
                    }
                    rate_limiter.record_block(&propagation_source, msg_size);
                    if let Some(block) = Block::deserialize(&message.data) {
                        let _ = event_tx
                            .send(NetworkEvent::NewBlock(block, propagation_source))
                            .await;
                    } else {
                        warn!("Failed to deserialize block from {}", propagation_source);
                    }
                }
                TRANSACTIONS_TOPIC => {
                    if !rate_limiter.check_transaction(&propagation_source) {
                        warn!(
                            "Rate limit: dropping tx from {} (tx rate exceeded)",
                            propagation_source
                        );
                        return;
                    }
                    rate_limiter.record_transaction(&propagation_source, msg_size);
                    match crate::gossip::decode_tx_gossip(&message.data) {
                        Some(crate::gossip::TxGossipMessage::FullBatch(txs)) => {
                            for tx in txs {
                                let _ = event_tx.send(NetworkEvent::NewTransaction(tx)).await;
                            }
                        }
                        Some(crate::gossip::TxGossipMessage::Announce(hashes)) => {
                            let _ = event_tx
                                .send(NetworkEvent::TxAnnouncement {
                                    peer_id: propagation_source,
                                    hashes,
                                })
                                .await;
                        }
                        None => {
                            warn!(
                                "Failed to deserialize transaction from {}",
                                propagation_source
                            );
                        }
                    }
                }
                PRODUCERS_TOPIC => {
                    if !rate_limiter.check_request(&propagation_source) {
                        warn!(
                            "Rate limit: dropping producer msg from {} (request rate exceeded)",
                            propagation_source
                        );
                        return;
                    }
                    rate_limiter.record_request(&propagation_source, msg_size);

                    // Decode order: bloom digest → ProducerSet protobuf → legacy bincode
                    //
                    // INC-I-002: Bloom filter digest MUST be tried FIRST.
                    // is_legacy_bincode_format() misclassifies ProducerSetDigest protobuf
                    // as legacy bincode (because ProducerSet::decode fails on digest data,
                    // the heuristic assumes it's bincode). This caused 39,000+ silent
                    // deserialization failures and prevented GSet CRDT convergence entirely.
                    let handled_as_digest = if let Ok(bloom) = decode_digest(&message.data) {
                        if bloom.size_bits() > 0 {
                            debug!(
                                "Received producer digest ({} elements, {} bits) from {}",
                                bloom.element_count(),
                                bloom.size_bits(),
                                propagation_source
                            );
                            let _ = event_tx
                                .send(NetworkEvent::ProducerDigestReceived {
                                    peer_id: propagation_source,
                                    digest: bloom,
                                })
                                .await;
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    if !handled_as_digest {
                        if is_legacy_bincode_format(&message.data) {
                            // Legacy bincode format: Vec<PublicKey>
                            match bincode::deserialize::<Vec<PublicKey>>(&message.data) {
                                Ok(pubkeys) => {
                                    debug!(
                                        "Received legacy producer list ({} producers) from {}",
                                        pubkeys.len(),
                                        propagation_source
                                    );
                                    let _ = event_tx
                                        .send(NetworkEvent::ProducersAnnounced(pubkeys))
                                        .await;
                                }
                                Err(e) => {
                                    warn!(
                                        "Failed to deserialize legacy producer list from {}: {}",
                                        propagation_source, e
                                    );
                                }
                            }
                        } else {
                            // New protobuf format: ProducerSet
                            match decode_producer_set(&message.data) {
                                Ok(announcements) => {
                                    debug!(
                                        "Received producer announcements ({} producers) from {}",
                                        announcements.len(),
                                        propagation_source
                                    );
                                    let _ = event_tx
                                        .send(NetworkEvent::ProducerAnnouncementsReceived(
                                            announcements,
                                        ))
                                        .await;
                                }
                                Err(e) => {
                                    warn!(
                                        "Failed to decode producer announcements from {}: {}",
                                        propagation_source, e
                                    );
                                }
                            }
                        }
                    }
                }
                VOTES_TOPIC => {
                    if !rate_limiter.check_request(&propagation_source) {
                        return;
                    }
                    rate_limiter.record_request(&propagation_source, msg_size);
                    debug!(
                        "Received vote message ({} bytes) from {}",
                        msg_size, propagation_source
                    );
                    let _ = event_tx
                        .send(NetworkEvent::NewVote(message.data.clone()))
                        .await;
                }
                HEARTBEATS_TOPIC => {
                    if !rate_limiter.check_request(&propagation_source) {
                        return;
                    }
                    rate_limiter.record_request(&propagation_source, msg_size);
                    debug!(
                        "Received heartbeat ({} bytes) from {}",
                        msg_size, propagation_source
                    );
                    let _ = event_tx
                        .send(NetworkEvent::NewHeartbeat(message.data.clone()))
                        .await;
                }
                topic if topic == crate::gossip::ATTESTATION_TOPIC => {
                    if !rate_limiter.check_request(&propagation_source) {
                        return;
                    }
                    rate_limiter.record_request(&propagation_source, msg_size);
                    debug!(
                        "Received attestation ({} bytes) from {}",
                        msg_size, propagation_source
                    );
                    let _ = event_tx
                        .send(NetworkEvent::NewAttestation(message.data.clone()))
                        .await;
                }
                HEADERS_TOPIC => {
                    if !rate_limiter.check_block(&propagation_source) {
                        return;
                    }
                    if let Some(header) = BlockHeader::deserialize(&message.data) {
                        rate_limiter.record_block(&propagation_source, msg_size);
                        let _ = event_tx.send(NetworkEvent::NewHeader(header)).await;
                    } else {
                        warn!("Failed to deserialize header from {}", propagation_source);
                    }
                }
                topic if topic == crate::gossip::TIER1_BLOCKS_TOPIC => {
                    // Tier 1 dense-mesh block: same payload as regular blocks
                    if !rate_limiter.check_block(&propagation_source) {
                        return;
                    }
                    if let Some(block) = Block::deserialize(&message.data) {
                        rate_limiter.record_block(&propagation_source, msg_size);
                        let _ = event_tx
                            .send(NetworkEvent::NewBlock(block, propagation_source))
                            .await;
                    } else {
                        warn!("Failed to deserialize t1 block from {}", propagation_source);
                    }
                }
                topic if topic.starts_with("/doli/r") && topic.ends_with("/blocks/1") => {
                    // Regional block (Tier 2 sharding): same payload as regular blocks
                    if !rate_limiter.check_block(&propagation_source) {
                        return;
                    }
                    if let Some(block) = Block::deserialize(&message.data) {
                        rate_limiter.record_block(&propagation_source, msg_size);
                        let _ = event_tx
                            .send(NetworkEvent::NewBlock(block, propagation_source))
                            .await;
                    }
                }
                _ => {}
            }
        }

        DoliBehaviourEvent::Kademlia(kad::Event::RoutingUpdated { peer, .. }) => {
            info!("[DHT] Routing updated for peer: {}", peer);
            // Don't auto-dial on RoutingUpdated — this is the main trigger for
            // the simultaneous-dial race on co-located nodes (rust-libp2p#752).
            // The periodic DHT bootstrap (every 60s) and explicit bootstrap dials
            // already handle peer discovery. Auto-dialing here is redundant and
            // on localhost (latency ≈ 0) it guarantees the race fires.
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
                        let _ = event_tx
                            .send(NetworkEvent::NetworkMismatch {
                                peer_id: peer,
                                our_network_id: config.network_id,
                                their_network_id: request.network_id,
                            })
                            .await;
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
                        let _ = event_tx
                            .send(NetworkEvent::GenesisMismatch { peer_id: peer })
                            .await;
                        let _ = swarm.disconnect_peer_id(peer);
                        return;
                    }

                    // Validate protocol version
                    if request.version < MIN_PEER_PROTOCOL_VERSION {
                        warn!(
                            "Protocol version mismatch with peer {}: we require >= {}, they report {}",
                            peer, MIN_PEER_PROTOCOL_VERSION, request.version
                        );
                        let _ = event_tx
                            .send(NetworkEvent::VersionMismatch {
                                peer_id: peer,
                                our_version: CURRENT_PROTOCOL_VERSION,
                                their_version: request.version,
                            })
                            .await;
                        let _ = swarm.disconnect_peer_id(peer);
                        return;
                    }

                    let _ = event_tx
                        .send(NetworkEvent::StatusRequest {
                            peer_id: peer,
                            request,
                            channel,
                        })
                        .await;
                }
                request_response::Message::Response { response, .. } => {
                    debug!("Received status response from {}", peer);

                    // Validate network ID
                    if response.network_id != config.network_id {
                        warn!(
                            "Network mismatch with peer {}: we are on network {}, they are on {}",
                            peer, config.network_id, response.network_id
                        );
                        let _ = event_tx
                            .send(NetworkEvent::NetworkMismatch {
                                peer_id: peer,
                                our_network_id: config.network_id,
                                their_network_id: response.network_id,
                            })
                            .await;
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
                        let _ = event_tx
                            .send(NetworkEvent::GenesisMismatch { peer_id: peer })
                            .await;
                        let _ = swarm.disconnect_peer_id(peer);
                        return;
                    }

                    // Validate protocol version
                    if response.version < MIN_PEER_PROTOCOL_VERSION {
                        warn!(
                            "Protocol version mismatch with peer {}: we require >= {}, they report {}",
                            peer, MIN_PEER_PROTOCOL_VERSION, response.version
                        );
                        let _ = event_tx
                            .send(NetworkEvent::VersionMismatch {
                                peer_id: peer,
                                our_version: CURRENT_PROTOCOL_VERSION,
                                their_version: response.version,
                            })
                            .await;
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

                    let _ = event_tx
                        .send(NetworkEvent::PeerStatus {
                            peer_id: peer,
                            status: response,
                        })
                        .await;
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
                    let _ = event_tx
                        .send(NetworkEvent::SyncRequest {
                            peer_id: peer,
                            request,
                            channel,
                        })
                        .await;
                }
                request_response::Message::Response { response, .. } => {
                    info!(
                        "[SYNC_DEBUG] Received sync response from peer={}, response_type={}",
                        peer,
                        response.type_name()
                    );
                    let _ = event_tx
                        .send(NetworkEvent::SyncResponse {
                            peer_id: peer,
                            response,
                        })
                        .await;
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
                    let _ = event_tx
                        .send(NetworkEvent::TxFetchRequest {
                            peer_id: peer,
                            hashes: request.hashes,
                            channel,
                        })
                        .await;
                }
                request_response::Message::Response { response, .. } => {
                    debug!(
                        "Received tx fetch response from {} ({} txs)",
                        peer,
                        response.transactions.len()
                    );
                    let _ = event_tx
                        .send(NetworkEvent::TxFetchResponse {
                            peer_id: peer,
                            transactions: response.transactions,
                        })
                        .await;
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

        _ => {}
    }
}
