//! Synchronous swarm→node event delivery over two bounded lanes (INC-I-237 R1).
//! Gossip is shed (counted) on a lane full by count or bytes; control events
//! wait in a FIFO backlog, drained by `try_flush`.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicIsize, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::mpsc::{self, error::TrySendError};
use tracing::{debug, warn};

use super::backpressure::{GossipShedMetrics, WARN_EVERY_N_DROPS};
use super::event_lanes::EventLanes;
use super::event_size::{block_bytes, sync_response_bytes, tx_bytes};
use super::types::NetworkEvent;
use crate::gossip::staleness::GossipTopic;

/// Bytes the consensus lane may hold.
pub const CONSENSUS_LANE_BYTES: usize = 64 << 20;
/// Bytes the bulk lane may hold.
pub const BULK_LANE_BYTES: usize = 16 << 20;
/// Payload bytes the control backlog may hold.
pub const BACKLOG_BYTES: usize = 64 << 20;
/// Max control events held while the node is not draining.
pub const BACKLOG_CAP: usize = 8_192;

const HASH_BYTES: usize = 32;
const ANNOUNCEMENT_BYTES: usize = 192;

/// An event and the bytes charged to its lane.
pub(super) type LaneItem = (NetworkEvent, usize);

/// Which receive queue an event travels in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    Consensus,
    Bulk,
}

/// `Emitted` = sent or backlogged; `Shed` = dropped (full, budget, closed).
#[derive(Debug, PartialEq, Eq)]
pub enum EmitOutcome {
    Emitted,
    Shed,
}

/// Slot and byte bounds of both lanes and of the control backlog.
#[derive(Debug, Clone, Copy)]
pub struct LaneBudgets {
    pub consensus_cap: usize,
    pub bulk_cap: usize,
    pub consensus_bytes: usize,
    pub bulk_bytes: usize,
    pub backlog_bytes: usize,
    pub backlog_cap: usize,
}

impl LaneBudgets {
    /// Production budgets; slot counts scale with `max_peers`.
    pub fn for_max_peers(max_peers: usize) -> Self {
        let slots = max_peers.saturating_mul(16).clamp(4096, 32768);
        Self {
            consensus_cap: slots,
            bulk_cap: slots,
            consensus_bytes: CONSENSUS_LANE_BYTES,
            bulk_bytes: BULK_LANE_BYTES,
            backlog_bytes: BACKLOG_BYTES,
            backlog_cap: BACKLOG_CAP,
        }
    }

    /// INV-NETWORK-002 queue half: every budget is finite and non-zero.
    pub fn is_bounded(&self) -> bool {
        [
            self.consensus_cap,
            self.bulk_cap,
            self.consensus_bytes,
            self.bulk_bytes,
            self.backlog_bytes,
            self.backlog_cap,
        ]
        .iter()
        .all(|&b| b > 0 && b < usize::MAX)
    }
}

/// Gossip events refused by a full lane, per topic.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TopicCounts {
    pub blocks: u64,
    pub txs: u64,
    pub attestations: u64,
    pub votes: u64,
    pub producers: u64,
}

impl TopicCounts {
    /// The count of `topic`.
    pub fn get(&self, topic: GossipTopic) -> u64 {
        match topic {
            GossipTopic::Blocks => self.blocks,
            GossipTopic::Transactions => self.txs,
            GossipTopic::Attestations => self.attestations,
            GossipTopic::Votes => self.votes,
            GossipTopic::Producers => self.producers,
        }
    }
}

/// Point-in-time copy of [`LaneStats`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LaneSnapshot {
    pub consensus_bytes: usize,
    pub bulk_bytes: usize,
    pub backlog_bytes: usize,
    pub backlog_len: usize,
    pub consensus_shed: u64,
    pub bulk_shed: u64,
    pub control_dropped: u64,
    pub consensus_len: usize,
    pub bulk_len: usize,
    pub shed_by_topic: TopicCounts,
}

/// In-flight bytes and drop counters shared by the sink, the receiver and readers.
#[derive(Debug, Default)]
pub struct LaneStats {
    consensus_bytes: AtomicUsize,
    bulk_bytes: AtomicUsize,
    backlog_bytes: AtomicUsize,
    backlog_len: AtomicUsize,
    consensus_shed: AtomicU64,
    bulk_shed: AtomicU64,
    control_dropped: AtomicU64,
    // Signed: a dequeue can land before the post-send increment.
    consensus_len: AtomicIsize,
    bulk_len: AtomicIsize,
    shed_by_topic: [AtomicU64; 5],
}

const TOPIC_ORDER: [GossipTopic; 5] = [
    GossipTopic::Blocks,
    GossipTopic::Transactions,
    GossipTopic::Attestations,
    GossipTopic::Votes,
    GossipTopic::Producers,
];

fn depth(n: &AtomicIsize) -> usize {
    usize::try_from(n.load(Ordering::Acquire)).unwrap_or(0)
}

impl LaneStats {
    /// Current values of every counter.
    pub fn snapshot(&self) -> LaneSnapshot {
        LaneSnapshot {
            consensus_bytes: self.consensus_bytes.load(Ordering::Acquire),
            bulk_bytes: self.bulk_bytes.load(Ordering::Acquire),
            backlog_bytes: self.backlog_bytes.load(Ordering::Acquire),
            backlog_len: self.backlog_len.load(Ordering::Acquire),
            consensus_shed: self.consensus_shed.load(Ordering::Relaxed),
            bulk_shed: self.bulk_shed.load(Ordering::Relaxed),
            control_dropped: self.control_dropped.load(Ordering::Relaxed),
            consensus_len: depth(&self.consensus_len),
            bulk_len: depth(&self.bulk_len),
            shed_by_topic: {
                let [blocks, txs, attestations, votes, producers] = self
                    .shed_by_topic
                    .each_ref()
                    .map(|c| c.load(Ordering::Relaxed));
                TopicCounts {
                    blocks,
                    txs,
                    attestations,
                    votes,
                    producers,
                }
            },
        }
    }

    fn len(&self, lane: Lane) -> &AtomicIsize {
        match lane {
            Lane::Consensus => &self.consensus_len,
            Lane::Bulk => &self.bulk_len,
        }
    }

    /// Count a gossip event refused by a full lane under its topic.
    fn shed_topic(&self, ev: &NetworkEvent) {
        if let Some(t) = ev.gossip_topic() {
            let i = TOPIC_ORDER.iter().position(|&x| x == t).unwrap_or(0);
            self.shed_by_topic[i].fetch_add(1, Ordering::Relaxed);
        }
    }

    fn bytes(&self, lane: Lane) -> &AtomicUsize {
        match lane {
            Lane::Consensus => &self.consensus_bytes,
            Lane::Bulk => &self.bulk_bytes,
        }
    }

    /// Return `size` bytes of `lane` on a refused send.
    pub(super) fn release(&self, lane: Lane, size: usize) {
        self.bytes(lane).fetch_sub(size, Ordering::AcqRel);
    }

    /// An event of `size` bytes left `lane`'s queue.
    pub(super) fn dequeued(&self, lane: Lane, size: usize) {
        self.release(lane, size);
        self.len(lane).fetch_sub(1, Ordering::AcqRel);
    }
}

/// Lane of every event; exhaustive so a new variant must be classified.
pub fn lane_of(ev: &NetworkEvent) -> Lane {
    match ev {
        NetworkEvent::NewBlock(..)
        | NetworkEvent::NewAttestation(..)
        | NetworkEvent::PeerConnected(..)
        | NetworkEvent::PeerDisconnected(..)
        | NetworkEvent::PeerStatus { .. }
        | NetworkEvent::NetworkMismatch { .. }
        | NetworkEvent::GenesisMismatch { .. }
        | NetworkEvent::VersionMismatch { .. }
        | NetworkEvent::StatusRequest { .. }
        | NetworkEvent::SyncRequest { .. }
        | NetworkEvent::SyncResponse { .. } => Lane::Consensus,
        NetworkEvent::NewTransaction(..)
        | NetworkEvent::TxAnnouncement { .. }
        | NetworkEvent::TxFetchRequest { .. }
        | NetworkEvent::TxFetchResponse { .. }
        | NetworkEvent::NewVote(..)
        | NetworkEvent::ProducersAnnounced(..)
        | NetworkEvent::ProducerAnnouncementsReceived(..)
        | NetworkEvent::ProducerDigestReceived { .. } => Lane::Bulk,
    }
}

/// Cheap, deterministic estimate of the heap an event holds while queued.
pub fn event_bytes(ev: &NetworkEvent) -> usize {
    let payload = match ev {
        NetworkEvent::NewBlock(b, ..) => block_bytes(b),
        NetworkEvent::NewTransaction(txs, _) => txs.iter().map(tx_bytes).sum(),
        NetworkEvent::SyncResponse { response, .. } => sync_response_bytes(response),
        NetworkEvent::NewAttestation(d, ..) | NetworkEvent::NewVote(d, _) => d.len(),
        NetworkEvent::TxFetchResponse { transactions, .. } => {
            transactions.iter().map(tx_bytes).sum()
        }
        NetworkEvent::TxAnnouncement { hashes, .. }
        | NetworkEvent::TxFetchRequest { hashes, .. } => hashes.len() * HASH_BYTES,
        NetworkEvent::ProducersAnnounced(keys, _) => keys.len() * HASH_BYTES,
        NetworkEvent::ProducerAnnouncementsReceived(a, _) => a.len() * ANNOUNCEMENT_BYTES,
        NetworkEvent::ProducerDigestReceived { digest, .. } => digest.size_bits() / 8,
        _ => 0,
    };
    std::mem::size_of::<NetworkEvent>() + payload
}

/// Control events that carry a payload; they are byte-bounded in the backlog.
fn is_payload_control(ev: &NetworkEvent) -> bool {
    matches!(
        ev,
        NetworkEvent::StatusRequest { .. }
            | NetworkEvent::SyncRequest { .. }
            | NetworkEvent::SyncResponse { .. }
    )
}

/// Consensus-lane gossip: shed on a full lane instead of backlogged.
fn is_consensus_gossip(ev: &NetworkEvent) -> bool {
    matches!(
        ev,
        NetworkEvent::NewBlock(..) | NetworkEvent::NewAttestation(..)
    )
}

fn count_drop(counter: &AtomicU64, what: &str) -> EmitOutcome {
    let n = counter.fetch_add(1, Ordering::Relaxed) + 1;
    if n % WARN_EVERY_N_DROPS == 1 {
        warn!("INC-I-237 event lane full: dropping {what} dropped_total={n}");
    }
    EmitOutcome::Shed
}

/// Add `size` to `counter` if it fits `budget`; `lenient` also admits into an empty lane.
fn reserve_bytes(counter: &AtomicUsize, size: usize, budget: usize, lenient: bool) -> bool {
    counter
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |cur| {
            let fits = cur.saturating_add(size) <= budget || (lenient && cur == 0);
            fits.then_some(cur + size)
        })
        .is_ok()
}

/// A lane refusal; `Full` hands the event back without allocating.
#[allow(clippy::large_enum_variant)]
enum Refused {
    Full(LaneItem),
    Closed,
}

/// Build the swarm-side sink and the node-side receiver of the two lanes.
pub fn event_lanes(b: LaneBudgets) -> (EventSink, EventLanes) {
    let (consensus, consensus_rx) = mpsc::channel(b.consensus_cap.max(1));
    let (bulk, bulk_rx) = mpsc::channel(b.bulk_cap.max(1));
    let stats = Arc::new(LaneStats::default());
    let sink = EventSink {
        consensus,
        bulk,
        budgets: b,
        stats: stats.clone(),
        backlog: Mutex::new(VecDeque::new()),
    };
    (sink, EventLanes::new(consensus_rx, bulk_rx, stats))
}

/// Swarm-side handle for delivering events to the node without waiting.
pub struct EventSink {
    consensus: mpsc::Sender<LaneItem>,
    bulk: mpsc::Sender<LaneItem>,
    budgets: LaneBudgets,
    stats: Arc<LaneStats>,
    backlog: Mutex<VecDeque<LaneItem>>,
}

impl EventSink {
    /// Shared lane counters.
    pub fn stats(&self) -> Arc<LaneStats> {
        self.stats.clone()
    }

    fn lock_backlog(&self) -> MutexGuard<'_, VecDeque<LaneItem>> {
        self.backlog.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[allow(clippy::result_large_err)]
    fn try_lane(&self, lane: Lane, item: LaneItem, lenient: bool) -> Result<(), Refused> {
        let (tx, budget) = match lane {
            Lane::Consensus => (&self.consensus, self.budgets.consensus_bytes),
            Lane::Bulk => (&self.bulk, self.budgets.bulk_bytes),
        };
        if tx.is_closed() {
            return Err(Refused::Closed);
        }
        let size = item.1;
        if !reserve_bytes(self.stats.bytes(lane), size, budget, lenient) {
            return Err(Refused::Full(item));
        }
        match tx.try_send(item) {
            Ok(()) => {
                self.stats.len(lane).fetch_add(1, Ordering::AcqRel);
                Ok(())
            }
            Err(TrySendError::Full(item)) => {
                self.stats.release(lane, size);
                Err(Refused::Full(item))
            }
            Err(TrySendError::Closed(_)) => {
                self.stats.release(lane, size);
                Err(Refused::Closed)
            }
        }
    }

    /// Deliver `ev` without waiting, routed by [`lane_of`]; control events keep their order.
    pub fn emit(&self, ev: NetworkEvent) -> EmitOutcome {
        let size = event_bytes(&ev);
        let lane = lane_of(&ev);
        if lane == Lane::Bulk || is_consensus_gossip(&ev) {
            let shed = match lane {
                Lane::Bulk => &self.stats.bulk_shed,
                Lane::Consensus => &self.stats.consensus_shed,
            };
            return match self.try_lane(lane, (ev, size), false) {
                Ok(()) => EmitOutcome::Emitted,
                Err(Refused::Full((ev, _))) => {
                    self.stats.shed_topic(&ev);
                    count_drop(shed, "gossip events")
                }
                Err(Refused::Closed) => EmitOutcome::Shed,
            };
        }
        let payload = is_payload_control(&ev);
        let mut backlog = self.lock_backlog();
        let item = if backlog.is_empty() {
            match self.try_lane(Lane::Consensus, (ev, size), true) {
                Ok(()) => return EmitOutcome::Emitted,
                Err(Refused::Closed) => return EmitOutcome::Shed,
                Err(Refused::Full(item)) => item,
            }
        } else if self.consensus.is_closed() {
            return EmitOutcome::Shed;
        } else {
            (ev, size)
        };
        self.admit_backlog(&mut backlog, item, payload)
    }

    fn admit_backlog(
        &self,
        backlog: &mut VecDeque<LaneItem>,
        item: LaneItem,
        payload: bool,
    ) -> EmitOutcome {
        let used = self.stats.backlog_bytes.load(Ordering::Acquire);
        let over_bytes = payload && used.saturating_add(item.1) > self.budgets.backlog_bytes;
        if backlog.len() >= self.budgets.backlog_cap || over_bytes {
            return count_drop(&self.stats.control_dropped, "control events");
        }
        self.stats.backlog_bytes.fetch_add(item.1, Ordering::AcqRel);
        backlog.push_back(item);
        self.stats
            .backlog_len
            .store(backlog.len(), Ordering::Release);
        EmitOutcome::Emitted
    }

    /// Deliver a gossip block; a full consensus lane sheds it and counts the drop.
    pub fn emit_block(&self, ev: NetworkEvent, metrics: &GossipShedMetrics) -> EmitOutcome {
        let size = event_bytes(&ev);
        match self.try_lane(Lane::Consensus, (ev, size), false) {
            Ok(()) => return EmitOutcome::Emitted,
            Err(Refused::Full((ev, _))) => {
                self.stats.shed_topic(&ev);
                self.stats.consensus_shed.fetch_add(1, Ordering::Relaxed);
                metrics.record_block_drop();
                let total = metrics.blocks_dropped();
                if total.is_multiple_of(WARN_EVERY_N_DROPS) {
                    warn!("[GOSSIP_SHED] consensus lane full — dropped {total} block(s) total");
                }
            }
            Err(Refused::Closed) => debug!("[GOSSIP_SHED] consensus lane closed (shutdown)"),
        }
        EmitOutcome::Shed
    }

    /// Move backlog events into the consensus lane, oldest first.
    /// Returns false once the node side is gone.
    pub fn try_flush(&self) -> bool {
        let mut backlog = self.lock_backlog();
        while let Some(item) = backlog.pop_front() {
            let size = item.1;
            match self.try_lane(Lane::Consensus, item, true) {
                Ok(()) => {
                    self.stats.backlog_bytes.fetch_sub(size, Ordering::AcqRel);
                }
                Err(Refused::Full(item)) => {
                    backlog.push_front(item);
                    break;
                }
                Err(Refused::Closed) => {
                    backlog.clear();
                    self.stats.backlog_bytes.store(0, Ordering::Release);
                }
            }
        }
        self.stats
            .backlog_len
            .store(backlog.len(), Ordering::Release);
        !self.consensus.is_closed()
    }
}
