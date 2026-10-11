//! Gossipsub message intake (INC-I-237 D2/D6): one relay decision, one
//! validation report, then local delivery to the node.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::SystemTime;

use doli_core::Block;
use libp2p::gossipsub::{self, MessageId};
use libp2p::{PeerId, Swarm};
use tracing::{debug, info, warn};

use crate::behaviour::DoliBehaviour;
use crate::config::NetworkConfig;
use crate::gossip::staleness::{SeenCache, StalenessCtx};
use crate::gossip::tx_batch_stats::TX_GOSSIP_BATCH_STATS;
use crate::gossip::{decide, now_unix_secs, Payload, RelayDecision, RelayTopic, TxGossipMessage};
use crate::rate_limit::RateLimiter;

use super::backpressure::GossipShedMetrics;
use super::event_sink::EventSink;
use super::types::NetworkEvent;
use super::GossipHandle;

/// Decide, report exactly once, then deliver what the decision processes.
/// Order on blocks: per-peer bucket (non-candidates) BEFORE the report;
/// memory shed and the global bucket AFTER it, as local drops (F-1).
#[allow(clippy::too_many_arguments)]
pub(super) fn handle_gossip_message(
    swarm: &mut Swarm<DoliBehaviour>,
    event_tx: &EventSink,
    config: &NetworkConfig,
    rate_limiter: &mut RateLimiter,
    seen: &mut SeenCache,
    best_slot: &AtomicU32,
    shed_metrics: &GossipShedMetrics,
    memory_shed_flag: &AtomicBool,
    source: PeerId,
    message_id: MessageId,
    message: gossipsub::Message,
) {
    let handle = GossipHandle::now();
    let topic = RelayTopic::from_topic_str(message.topic.as_str());
    let best = best_slot.load(Ordering::Relaxed);
    let mut ctx = StalenessCtx {
        now_unix: now_unix_secs(),
        genesis_time: config.genesis_time,
        slot_duration: config.slot_duration,
        best_slot: best,
        seen,
    };
    let (decision, payload) = match decide(topic, &message.data, &mut ctx) {
        (RelayDecision::AcceptBlock(b), payload)
            if b.header.slot < best && !rate_limiter.check_block_peer_only(&source) =>
        {
            warn!(
                "Rate limit: ignoring block s={} from {} (per-peer rate exceeded)",
                b.header.slot, source
            );
            (RelayDecision::Ignore, payload)
        }
        other => other,
    };
    if let Err(e) = swarm
        .behaviour_mut()
        .gossipsub
        .report_message_validation_result(&message_id, &source, decision.acceptance())
    {
        warn!(
            "[GOSSIP_VALIDATE] report failed topic={} msg_id={} err={:?}",
            message.topic, message_id, e
        );
    }
    let size = message.data.len();
    match decision {
        RelayDecision::AcceptBlock(block) => {
            let candidate = block.header.slot >= best;
            if memory_shed_flag.load(Ordering::Relaxed) {
                shed_metrics.record_block_drop();
                debug!(
                    "[MEM_WATCHDOG] Shedding block s={} from {} (memory pressure)",
                    block.header.slot, source
                );
            } else if !rate_limiter.check_block_global_only() {
                warn!(
                    "Rate limit: dropping block s={} from {} (global rate exceeded)",
                    block.header.slot, source
                );
            } else {
                rate_limiter.record_block(&source, size);
                log_block_recv(&block, source, size, candidate);
                event_tx.emit_block(NetworkEvent::NewBlock(*block, source, handle), shed_metrics);
            }
        }
        RelayDecision::LocalOnly | RelayDecision::AcceptTransitional => deliver_local(
            payload,
            message.data,
            source,
            size,
            handle,
            event_tx,
            rate_limiter,
        ),
        RelayDecision::Reject | RelayDecision::Ignore => {
            debug!("[GOSSIP_VALIDATE] not processed topic={topic:?} from={source}")
        }
    }
}

fn log_block_recv(block: &Block, source: PeerId, size: usize, candidate: bool) {
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
        source,
        size,
        recv_ts,
        candidate
    );
}

fn deliver_local(
    payload: Payload,
    data: Vec<u8>,
    source: PeerId,
    size: usize,
    handle: GossipHandle,
    event_tx: &EventSink,
    rate_limiter: &mut RateLimiter,
) {
    let (event, is_producer) = match payload {
        Payload::Txs(msg) => return deliver_txs(msg, source, size, handle, event_tx, rate_limiter),
        Payload::Digest(digest) => (
            NetworkEvent::ProducerDigestReceived {
                peer_id: source,
                digest,
                handle,
            },
            true,
        ),
        Payload::LegacyProducers(keys) => (NetworkEvent::ProducersAnnounced(keys, handle), true),
        Payload::Announcements(anns) => (
            NetworkEvent::ProducerAnnouncementsReceived(anns, handle),
            true,
        ),
        Payload::Vote => (NetworkEvent::NewVote(data, handle), false),
        Payload::Attestation => (NetworkEvent::NewAttestation(data, source, handle), false),
        Payload::None => return,
    };
    if rate_limiter.check_request(&source) {
        rate_limiter.record_request(&source, size);
        event_tx.emit(event);
    } else if is_producer {
        warn!("Rate limit: dropping producer msg from {source} (request rate exceeded)");
    }
}

fn deliver_txs(
    msg: TxGossipMessage,
    source: PeerId,
    size: usize,
    handle: GossipHandle,
    event_tx: &EventSink,
    rate_limiter: &mut RateLimiter,
) {
    if !rate_limiter.check_transaction(&source) {
        warn!("Rate limit: dropping tx from {source} (tx rate exceeded)");
    } else {
        rate_limiter.record_transaction(&source, size);
        match msg {
            TxGossipMessage::FullBatch(txs) => {
                TX_GOSSIP_BATCH_STATS.record(txs.len());
                for tx in txs {
                    event_tx.emit(NetworkEvent::NewTransaction(tx, handle.clone()));
                }
            }
            TxGossipMessage::Announce(hashes) => {
                event_tx.emit(NetworkEvent::TxAnnouncement {
                    peer_id: source,
                    hashes,
                    handle,
                });
            }
        }
    }
}
