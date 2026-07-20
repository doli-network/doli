//! Behaviour-level event handling.
//!
//! Processes gossipsub messages, Kademlia routing updates, Identify exchanges,
//! status/sync request-response protocol messages, and NAT traversal events.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use libp2p::{autonat, gossipsub, identify, kad, request_response, Multiaddr, PeerId, Swarm};
use std::time::SystemTime;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info, warn};

use doli_core::{decode_digest, decode_producer_set, is_legacy_bincode_format, BlockHeader};

use crate::behaviour::{DoliBehaviour, DoliBehaviourEvent};
use crate::config::NetworkConfig;
use crate::gossip::{
    classify_block_gossip, now_unix_secs, BLOCKS_TOPIC, HEADERS_TOPIC, HEARTBEATS_TOPIC,
    PRODUCERS_TOPIC, TRANSACTIONS_TOPIC, VOTES_TOPIC,
};
use crate::peer::PeerInfo;
use crate::peer_cache::PeerCache;
use crate::rate_limit::RateLimiter;
use crypto::PublicKey;

use super::backpressure::enqueue_or_shed;
use super::helpers::{all_addresses_routable, is_routable_address};
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
    gossip_seen_cache: &mut crate::gossip::staleness::SeenCache,
    best_slot: &Arc<std::sync::atomic::AtomicU32>,
    shed_metrics: &std::sync::Arc<super::backpressure::GossipShedMetrics>,
    memory_shed_flag: &Arc<AtomicBool>,
) {
    match event {
        DoliBehaviourEvent::Gossipsub(gossipsub::Event::Message {
            propagation_source,
            message_id,
            message,
        }) => {
            let topic = message.topic.as_str();
            let msg_size = message.data.len();
            debug!(
                "Received gossip message on topic {} from {}",
                topic, propagation_source
            );

            // INC-I-114: With validate_messages=true, every message is held
            // un-forwarded until we call report_message_validation_result().
            // We MUST report for ALL topics or gossip propagation dies.
            //
            // Block-body topics get staleness classification via classify_block_gossip.
            // P0-001: HEADERS_TOPIC is NOT a block topic — header bytes are
            // BlockHeader-serialized (no Vec<Transaction>), so Block::deserialize
            // fails on them. Routing headers through classify_block_gossip would
            // Reject every header → P4 penalty on honest producers → mesh
            // expulsion cascade (INC-I-016 shape).
            //
            // INC-I-142 M6: every OTHER (non-block-body) subscribed re-forward-risk
            // topic now routes through the unified `classify_gossip` staleness gate
            // (persistent per-event-loop SeenCache, identity dedup PRIMARY) — NOT
            // unconditional Accept. Only a genuinely-unclassified/unsubscribed topic
            // still Accepts by default (fail-open, preserves propagation).
            let is_block_body_topic = topic == BLOCKS_TOPIC
                || topic == crate::gossip::TIER1_BLOCKS_TOPIC
                || (topic.starts_with("/doli/r") && topic.ends_with("/blocks/1"));

            // P1-001: Deserialize ONCE via classify_block_gossip, which returns
            // both the acceptance verdict and the deserialized Block (if valid).
            // The handler reuses the returned Block instead of deserializing again.
            if is_block_body_topic {
                let now = now_unix_secs();
                let (acceptance, maybe_block) = classify_block_gossip(
                    &message.data,
                    config.genesis_time,
                    config.slot_duration,
                    now,
                );
                let is_accepted = matches!(acceptance, gossipsub::MessageAcceptance::Accept);

                // CRITICAL: report_message_validation_result MUST be called BEFORE
                // any early return (including the memory watchdog shed below).
                // This preserves gossipsub propagation semantics (INV-NET-002):
                // the validation result controls whether the message is forwarded
                // to other peers. Skipping it would silently kill gossip propagation.
                if let Err(e) = swarm
                    .behaviour_mut()
                    .gossipsub
                    .report_message_validation_result(&message_id, &propagation_source, acceptance)
                {
                    warn!(
                        "[GOSSIP_VALIDATE] report failed topic={} msg_id={} err={:?}",
                        topic, message_id, e
                    );
                }
                // If the message was Ignored or Rejected, skip further processing
                if !is_accepted {
                    debug!(
                        "[GOSSIP_VALIDATE] stale/invalid block on topic={} from={}",
                        topic, propagation_source
                    );
                    return;
                }

                // INC-I-114 M2: Memory watchdog shed gate.
                // When the watchdog has tripped (process RSS above soft threshold),
                // shed ALL accepted gossip blocks to reduce memory pressure.
                // This check is AFTER report_message_validation_result (so gossip
                // propagation to other peers is preserved) but BEFORE enqueue
                // (so the block never reaches the node's event channel or apply_block).
                // Dropped blocks are recoverable via the sync protocol (GetBlocks).
                if memory_shed_flag.load(Ordering::Relaxed) {
                    shed_metrics.record_block_drop();
                    debug!(
                        "[MEM_WATCHDOG] Shedding block s={} from {} (memory pressure)",
                        maybe_block.as_ref().map(|b| b.header.slot).unwrap_or(0),
                        propagation_source
                    );
                    return;
                }

                // At this point: acceptance=Accept, maybe_block=Some(block)
                // (Accept is only returned when deserialization succeeds).
                // Unwrap is safe — classify_block_gossip returns Some on Accept.
                let block = match maybe_block {
                    Some(b) => b,
                    None => return, // defensive — cannot happen for Accept
                };

                // Dispatch to the per-topic handler using the already-deserialized block
                match topic {
                    BLOCKS_TOPIC => {
                        // INC-I-049: Rate limiting for blocks. Candidate-next blocks
                        // (at or beyond our best slot) skip per-peer limits.
                        let current_best = best_slot.load(std::sync::atomic::Ordering::Relaxed);
                        let is_candidate_next = block.header.slot >= current_best;

                        if is_candidate_next {
                            if !rate_limiter.check_block_global_only() {
                                warn!(
                                    "Rate limit: dropping candidate block s={} from {} (global rate exceeded)",
                                    block.header.slot, propagation_source
                                );
                                return;
                            }
                        } else if !rate_limiter.check_block(&propagation_source) {
                            warn!(
                                "Rate limit: dropping block s={} from {} (per-peer rate exceeded)",
                                block.header.slot, propagation_source
                            );
                            return;
                        }

                        rate_limiter.record_block(&propagation_source, msg_size);
                        let recv_ts = SystemTime::now()
                            .duration_since(SystemTime::UNIX_EPOCH)
                            .map(|d| d.as_millis())
                            .unwrap_or(0);
                        info!(
                            "[GOSSIP_BLOCK] recv s={} hash={:.8} producer={:.8} block_ts={} from={} size={} recv_ts_ms={} next={}",
                            block.header.slot,
                            block.hash(),
                            block.header.producer,
                            block.header.timestamp,
                            propagation_source,
                            msg_size,
                            recv_ts,
                            is_candidate_next
                        );
                        enqueue_or_shed(
                            event_tx,
                            NetworkEvent::NewBlock(block, propagation_source),
                            shed_metrics,
                        );
                    }
                    t if t == crate::gossip::TIER1_BLOCKS_TOPIC => {
                        if !rate_limiter.check_block(&propagation_source) {
                            return;
                        }
                        rate_limiter.record_block(&propagation_source, msg_size);
                        let recv_ts = SystemTime::now()
                            .duration_since(SystemTime::UNIX_EPOCH)
                            .map(|d| d.as_millis())
                            .unwrap_or(0);
                        info!(
                            "[GOSSIP_BLOCK] recv_t1 s={} hash={:.8} producer={:.8} block_ts={} from={} size={} recv_ts_ms={}",
                            block.header.slot,
                            block.hash(),
                            block.header.producer,
                            block.header.timestamp,
                            propagation_source,
                            msg_size,
                            recv_ts
                        );
                        enqueue_or_shed(
                            event_tx,
                            NetworkEvent::NewBlock(block, propagation_source),
                            shed_metrics,
                        );
                    }
                    t if t.starts_with("/doli/r") && t.ends_with("/blocks/1") => {
                        if !rate_limiter.check_block(&propagation_source) {
                            return;
                        }
                        rate_limiter.record_block(&propagation_source, msg_size);
                        enqueue_or_shed(
                            event_tx,
                            NetworkEvent::NewBlock(block, propagation_source),
                            shed_metrics,
                        );
                    }
                    _ => {} // unreachable — is_block_body_topic already filtered
                }

                return; // block topics fully handled above
            }

            // INC-I-142 M6: unified staleness gate (see the block comment above).
            // Producers keep bit-identical behavior (classify_gossip's Producers arm
            // delegates to the same classify_producer_gossip). An unclassified /
            // unsubscribed topic fails open to Accept+forward (dispatch then no-ops)
            // so propagation is never stalled.
            let acceptance = match crate::gossip::staleness::GossipTopic::from_topic_str(topic) {
                Some(topic_enum) => {
                    let mut ctx = crate::gossip::staleness::StalenessCtx {
                        now_unix: now_unix_secs(),
                        genesis_time: config.genesis_time,
                        slot_duration: config.slot_duration,
                        best_slot: best_slot.load(std::sync::atomic::Ordering::Relaxed),
                        seen: gossip_seen_cache,
                    };
                    crate::gossip::staleness::classify_gossip(topic_enum, &message.data, &mut ctx)
                }
                None => gossipsub::MessageAcceptance::Accept,
            };
            let is_accepted = matches!(acceptance, gossipsub::MessageAcceptance::Accept);
            if let Err(e) = swarm
                .behaviour_mut()
                .gossipsub
                .report_message_validation_result(&message_id, &propagation_source, acceptance)
            {
                warn!(
                    "[GOSSIP_VALIDATE] report failed topic={} msg_id={} err={:?}",
                    topic, message_id, e
                );
            }
            // Stale / already-seen gossip: reported Ignore above (suppresses
            // re-forward, no peer penalty) — skip local processing too. A duplicate
            // re-delivery carries no new information, and a fully-stale producer set
            // would be rejected by the GSet merge anyway.
            if !is_accepted {
                debug!(
                    "[GOSSIP_VALIDATE] stale/duplicate gossip suppressed topic={} from={}",
                    topic, propagation_source
                );
                return;
            }

            // Non-block topic dispatch (block topics already returned above)
            match topic {
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
                        .send(NetworkEvent::NewAttestation(
                            message.data.clone(),
                            propagation_source,
                        ))
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
                _ => {}
            }
        }

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
