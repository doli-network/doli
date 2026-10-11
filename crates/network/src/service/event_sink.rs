//! Synchronous swarm→node event delivery (INC-I-237 D4). Gossip events and
//! blocks are shed (counted) on a full channel; control events wait in a FIFO
//! backlog, capped at [`BACKLOG_CAP`] (INV-NETWORK-002), drained by `try_flush`.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use tokio::sync::mpsc::{self, error::TrySendError};
use tracing::{debug, warn};

use super::backpressure::{GossipShedMetrics, WARN_EVERY_N_DROPS};
use super::types::NetworkEvent;

/// Max control events held while the node is not draining.
pub const BACKLOG_CAP: usize = 8_192;

/// `Emitted` = sent or backlogged; `Shed` = dropped (full, backlog cap, closed).
#[derive(Debug, PartialEq, Eq)]
pub enum EmitOutcome {
    Emitted,
    Shed,
}

/// Swarm-side handle for delivering events to the node without waiting.
pub struct EventSink {
    tx: mpsc::Sender<NetworkEvent>,
    backlog: Mutex<VecDeque<NetworkEvent>>,
    gossip_dropped: AtomicU64,
    control_dropped: AtomicU64,
}

/// Gossip events: a lost one is replaced by the next message or by sync.
fn is_sheddable(ev: &NetworkEvent) -> bool {
    matches!(
        ev,
        NetworkEvent::NewTransaction(..)
            | NetworkEvent::TxAnnouncement { .. }
            | NetworkEvent::NewVote(..)
            | NetworkEvent::NewAttestation(..)
            | NetworkEvent::ProducersAnnounced(..)
            | NetworkEvent::ProducerAnnouncementsReceived(..)
            | NetworkEvent::ProducerDigestReceived { .. }
    )
}

fn count_drop(counter: &AtomicU64, what: &str) -> EmitOutcome {
    let n = counter.fetch_add(1, Ordering::Relaxed) + 1;
    if n % WARN_EVERY_N_DROPS == 1 {
        warn!("INC-I-237 event sink full: dropping {what} dropped_total={n}");
    }
    EmitOutcome::Shed
}

impl EventSink {
    pub fn new(tx: mpsc::Sender<NetworkEvent>) -> Self {
        Self {
            tx,
            backlog: Mutex::new(VecDeque::new()),
            gossip_dropped: AtomicU64::new(0),
            control_dropped: AtomicU64::new(0),
        }
    }

    /// Deliver `ev` without waiting; control events keep their order.
    pub fn emit(&self, ev: NetworkEvent) -> EmitOutcome {
        let mut backlog = self.backlog.lock().unwrap_or_else(|e| e.into_inner());
        let sheddable = is_sheddable(&ev);
        let ev = if backlog.is_empty() || sheddable || self.tx.is_closed() {
            match self.tx.try_send(ev) {
                Ok(()) => return EmitOutcome::Emitted,
                Err(TrySendError::Closed(_)) => return EmitOutcome::Shed,
                Err(TrySendError::Full(ev)) => ev,
            }
        } else {
            ev
        };
        if sheddable {
            count_drop(&self.gossip_dropped, "gossip events")
        } else if backlog.len() < BACKLOG_CAP {
            backlog.push_back(ev);
            EmitOutcome::Emitted
        } else {
            count_drop(&self.control_dropped, "control events")
        }
    }

    /// Deliver a gossip block; a full channel sheds it and counts the drop.
    pub fn emit_block(&self, ev: NetworkEvent, metrics: &GossipShedMetrics) -> EmitOutcome {
        match self.tx.try_send(ev) {
            Ok(()) => return EmitOutcome::Emitted,
            Err(TrySendError::Full(_)) => {
                metrics.record_block_drop();
                let total = metrics.blocks_dropped();
                if total.is_multiple_of(WARN_EVERY_N_DROPS) {
                    warn!("[GOSSIP_SHED] channel full — dropped {total} block(s) total");
                }
            }
            Err(TrySendError::Closed(_)) => debug!("[GOSSIP_SHED] channel closed (shutdown)"),
        }
        EmitOutcome::Shed
    }

    /// Move backlog events into free channel slots, oldest first.
    /// Returns false once the node side is gone.
    pub fn try_flush(&self) -> bool {
        let mut backlog = self.backlog.lock().unwrap_or_else(|e| e.into_inner());
        while !backlog.is_empty() {
            match self.tx.try_reserve() {
                Ok(permit) => {
                    if let Some(ev) = backlog.pop_front() {
                        permit.send(ev);
                    }
                }
                Err(TrySendError::Full(())) => break,
                Err(TrySendError::Closed(())) => backlog.clear(),
            }
        }
        !self.tx.is_closed()
    }
}
