//! Gossipsub message intake (INC-I-237 D1/D2/D6): one relay decision, then
//! one validation report — by the swarm, or by the node's verdict on Defer.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Instant, SystemTime};

use doli_core::Block;
use libp2p::gossipsub::{self, MessageAcceptance, MessageId};
use libp2p::{PeerId, Swarm};
use tracing::{debug, info, warn};

use crate::behaviour::DoliBehaviour;
use crate::config::NetworkConfig;
use crate::gossip::staleness::{GossipTopic, SeenCache, StalenessCtx};
use crate::gossip::tx_batch_stats::TX_GOSSIP_BATCH_STATS;
use crate::gossip::{decide, now_unix_secs, Payload, RelayDecision, RelayTopic};
use crate::rate_limit::RateLimiter;

use super::backpressure::GossipShedMetrics;
use super::event_sink::{EmitOutcome, EventSink};
use super::types::NetworkEvent;
use super::GossipHandle;

/// Decide, report at most once, then deliver what the decision processes.
/// Order on blocks: per-peer bucket (non-candidates) BEFORE the report;
/// memory shed and the global bucket AFTER it, as local drops (F-1).
/// A Deferred message dropped here is reported Ignore at the drop.
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
        (RelayDecision::Defer, _) if !rate_limiter.check_transaction(&source) => {
            warn!("Rate limit: dropping tx from {source} (tx rate exceeded)");
            (RelayDecision::Ignore, Payload::None)
        }
        other => other,
    };
    let mut report = |acceptance| {
        if let Err(e) = swarm
            .behaviour_mut()
            .gossipsub
            .report_message_validation_result(&message_id, &source, acceptance)
        {
            warn!(
                "[GOSSIP_VALIDATE] report failed topic={} msg_id={} err={:?}",
                message.topic, message_id, e
            );
        }
    };
    if let Some(acceptance) = decision.acceptance() {
        report(acceptance);
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
        RelayDecision::Defer => {
            let id = message_id.clone();
            let outcome = defer(
                payload,
                id,
                source,
                size,
                handle.recv_at,
                event_tx,
                rate_limiter,
            );
            if outcome == EmitOutcome::Shed {
                report(MessageAcceptance::Ignore);
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
        Payload::TxHashes(hashes) => {
            let announce = NetworkEvent::TxAnnouncement {
                peer_id: source,
                hashes,
                handle,
            };
            return deliver_txs(announce, source, size, event_tx, rate_limiter);
        }
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
        Payload::None | Payload::Txs(_) => return,
    };
    if rate_limiter.check_request(&source) {
        rate_limiter.record_request(&source, size);
        event_tx.emit(event);
    } else if is_producer {
        warn!("Rate limit: dropping producer msg from {source} (request rate exceeded)");
    }
}

fn deliver_txs(
    event: NetworkEvent,
    source: PeerId,
    size: usize,
    event_tx: &EventSink,
    rate_limiter: &mut RateLimiter,
) {
    if rate_limiter.check_transaction(&source) {
        rate_limiter.record_transaction(&source, size);
        event_tx.emit(event);
    } else {
        warn!("Rate limit: dropping tx from {source} (tx rate exceeded)");
    }
}

/// Carry a Deferred tx message to the node as ONE event whose handle names the
/// original message; the caller reports Ignore when this returns `Shed`.
fn defer(
    payload: Payload,
    msg_id: MessageId,
    source: PeerId,
    size: usize,
    recv_at: Instant,
    event_tx: &EventSink,
    rate_limiter: &mut RateLimiter,
) -> EmitOutcome {
    let Payload::Txs(txs) = payload else {
        return EmitOutcome::Shed;
    };
    rate_limiter.record_transaction(&source, size);
    TX_GOSSIP_BATCH_STATS.record(txs.len());
    let mut handle = GossipHandle::new(GossipTopic::Transactions, msg_id, source);
    handle.recv_at = recv_at;
    event_tx.emit(NetworkEvent::NewTransaction(txs, handle))
}
