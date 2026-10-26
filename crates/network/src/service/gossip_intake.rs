//! Gossipsub message intake (INC-I-237 D1/D2/D6/R2): one relay decision, then
//! one validation report — by the swarm, or by the node's verdict on Defer.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Instant, SystemTime};

use doli_core::Block;
use libp2p::gossipsub::{self, MessageAcceptance, MessageId};
use libp2p::{PeerId, Swarm};
use tracing::{debug, info, warn};

use crate::behaviour::DoliBehaviour;
use crate::config::NetworkConfig;
use crate::gossip::staleness::{GossipTopic, StalenessCtx};
use crate::gossip::tx_batch_stats::TX_GOSSIP_BATCH_STATS;
use crate::gossip::{decide, now_unix_secs, Payload, RelayDecision, RelayTopic};
use crate::rate_limit::RateLimiter;

use super::backpressure::GossipShedMetrics;
use super::event_sink::{EmitOutcome, EventSink};
use super::types::NetworkEvent;
use super::GossipHandle;

/// The node-written local tip the gossip intake reads.
#[derive(Default)]
pub(super) struct LocalTip {
    pub(super) slot: AtomicU32,
    pub(super) height: AtomicU64,
}

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
    local_tip: &LocalTip,
    shed_metrics: &GossipShedMetrics,
    memory_shed_flag: &AtomicBool,
    source: PeerId,
    message_id: MessageId,
    message: gossipsub::Message,
) {
    let handle = GossipHandle::now();
    let topic = RelayTopic::from_topic_str(message.topic.as_str());
    let best = local_tip.slot.load(Ordering::Relaxed);
    let ctx = StalenessCtx {
        now_unix: now_unix_secs(),
        genesis_time: config.genesis_time,
        slot_duration: config.slot_duration,
        best_slot: best,
        best_height: local_tip.height.load(Ordering::Relaxed),
        block_signature_activation_height: config.block_signature_activation_height,
    };
    let (decision, payload) = match decide(topic, &message.data, &ctx) {
        (RelayDecision::AcceptBlock(b), payload)
            if b.header.slot < best && !rate_limiter.check_block_peer_only(&source) =>
        {
            warn!(
                "Rate limit: ignoring block s={} from {} (per-peer rate exceeded)",
                b.header.slot, source
            );
            (RelayDecision::Ignore, payload)
        }
        (RelayDecision::Defer, payload) if !within_peer_budget(&payload, &source, rate_limiter) => {
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
                message.data,
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
        RelayDecision::LocalOnly => {
            deliver_local(payload, source, size, handle, event_tx, rate_limiter)
        }
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
    source: PeerId,
    size: usize,
    handle: GossipHandle,
    event_tx: &EventSink,
    rate_limiter: &mut RateLimiter,
) {
    let event = match payload {
        Payload::TxHashes(hashes) => {
            let announce = NetworkEvent::TxAnnouncement {
                peer_id: source,
                hashes,
                handle,
            };
            return deliver_txs(announce, source, size, event_tx, rate_limiter);
        }
        Payload::Digest(digest) => NetworkEvent::ProducerDigestReceived {
            peer_id: source,
            digest,
            handle,
        },
        Payload::LegacyProducers(keys) => NetworkEvent::ProducersAnnounced(keys, handle),
        Payload::None
        | Payload::Txs(_)
        | Payload::Announcements(_)
        | Payload::Vote
        | Payload::Attestation => return,
    };
    if rate_limiter.check_request(&source) {
        rate_limiter.record_request(&source, size);
        event_tx.emit(event);
    } else {
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
    if rate_limiter.check_transaction(&source, 1) {
        rate_limiter.record_transaction(&source, 1, size);
        event_tx.emit(event);
    } else {
        warn!("Rate limit: dropping tx from {source} (tx rate exceeded)");
    }
}

/// Per-peer bucket of a deferred message: txs draw on the tx bucket,
/// attestations on their own bucket, every other topic on the request bucket,
/// never on the tx bucket (F-2).
fn within_peer_budget(payload: &Payload, source: &PeerId, rate_limiter: &mut RateLimiter) -> bool {
    if let Payload::Txs(txs) = payload {
        let ok = rate_limiter.check_transaction(source, txs.len() as u64);
        if !ok {
            warn!("Rate limit: dropping tx from {source} (tx rate exceeded)");
        }
        return ok;
    }
    if let Payload::Attestation = payload {
        return rate_limiter.check_attestation(source);
    }
    let ok = rate_limiter.check_request(source);
    if !ok && matches!(payload, Payload::Announcements(_)) {
        warn!("Rate limit: dropping producer msg from {source} (request rate exceeded)");
    }
    ok
}

/// Carry a Deferred message to the node as ONE event whose handle names the
/// original message; the caller reports Ignore when this returns `Shed`.
#[allow(clippy::too_many_arguments)]
fn defer(
    payload: Payload,
    data: Vec<u8>,
    msg_id: MessageId,
    source: PeerId,
    size: usize,
    recv_at: Instant,
    event_tx: &EventSink,
    rate_limiter: &mut RateLimiter,
) -> EmitOutcome {
    let stamp = move |topic| {
        let mut handle = GossipHandle::new(topic, msg_id, source);
        handle.recv_at = recv_at;
        handle
    };
    let event = match payload {
        Payload::Txs(txs) => {
            rate_limiter.record_transaction(&source, txs.len() as u64, size);
            TX_GOSSIP_BATCH_STATS.record(txs.len());
            let handle = stamp(GossipTopic::Transactions);
            return event_tx.emit(NetworkEvent::NewTransaction(txs, handle));
        }
        Payload::Announcements(anns) => {
            NetworkEvent::ProducerAnnouncementsReceived(anns, stamp(GossipTopic::Producers))
        }
        Payload::Vote => NetworkEvent::NewVote(data, stamp(GossipTopic::Votes)),
        Payload::Attestation => {
            rate_limiter.record_attestation(&source, size);
            let handle = stamp(GossipTopic::Attestations);
            return event_tx.emit(NetworkEvent::NewAttestation(data, source, handle));
        }
        Payload::None | Payload::TxHashes(_) | Payload::Digest(_) | Payload::LegacyProducers(_) => {
            return EmitOutcome::Shed
        }
    };
    rate_limiter.record_request(&source, size);
    event_tx.emit(event)
}
