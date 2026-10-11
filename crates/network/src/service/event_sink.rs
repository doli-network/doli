//! Non-blocking event delivery from the swarm loop to the node (INC-I-237).
//!
//! The swarm loop used to `event_tx.send(..).await`. With the event channel
//! full it parked inside `handle_swarm_event`, so its `command_rx` arm was not
//! polled. The node loop, parked on `command_tx.send(..).await` with the
//! command channel full, could not drain events again: a mutual deadlock.
//!
//! `EventSink::emit` never awaits:
//! - gossip events are shed (counted) when the channel is full — the same
//!   policy as gossip blocks (`backpressure::enqueue_or_shed`);
//! - control events (peer, status, sync, mismatch) go to a FIFO backlog that
//!   the swarm loop drains in its own `select!` arm, so they are not lost.
//!
//! RESOURCE COST: the backlog is capped at [`BACKLOG_CAP`] events (control
//! events are small and low-rate; ~few MB worst case). Above the cap a control
//! event is dropped and counted, so growth is bounded (INV-NETWORK-002).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use tokio::sync::mpsc;
use tracing::warn;

use super::backpressure::WARN_EVERY_N_DROPS;
use super::types::NetworkEvent;

/// Max control events held while the node is not draining.
pub const BACKLOG_CAP: usize = 8_192;

/// Swarm-side handle for delivering events to the node without awaiting.
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
            | NetworkEvent::NewHeader(..)
            | NetworkEvent::NewVote(..)
            | NetworkEvent::NewHeartbeat(..)
            | NetworkEvent::NewAttestation(..)
            | NetworkEvent::ProducersAnnounced(..)
            | NetworkEvent::ProducerAnnouncementsReceived(..)
            | NetworkEvent::ProducerDigestReceived { .. }
    )
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

    /// The raw sender, for `backpressure::enqueue_or_shed` (gossip blocks).
    pub fn sender(&self) -> &mpsc::Sender<NetworkEvent> {
        &self.tx
    }

    /// Deliver `ev` without awaiting. Never blocks the swarm loop.
    pub fn emit(&self, ev: NetworkEvent) {
        let mut backlog = self.backlog.lock().unwrap_or_else(|e| e.into_inner());
        // Keep control events in order behind any backlog.
        let ev = if backlog.is_empty() || is_sheddable(&ev) {
            match self.tx.try_send(ev) {
                Ok(()) => return,
                Err(mpsc::error::TrySendError::Closed(_)) => return,
                Err(mpsc::error::TrySendError::Full(ev)) => ev,
            }
        } else {
            ev
        };
        if is_sheddable(&ev) {
            let n = self.gossip_dropped.fetch_add(1, Ordering::Relaxed) + 1;
            if n % WARN_EVERY_N_DROPS == 1 {
                warn!("INC-I-237 event channel full: shedding gossip events dropped_total={n}");
            }
        } else if backlog.len() < BACKLOG_CAP {
            backlog.push_back(ev);
        } else {
            let n = self.control_dropped.fetch_add(1, Ordering::Relaxed) + 1;
            if n % WARN_EVERY_N_DROPS == 1 {
                warn!("INC-I-237 event backlog full: dropping control events dropped_total={n}");
            }
        }
    }

    pub fn has_backlog(&self) -> bool {
        !self
            .backlog
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
    }

    /// Hand the oldest backlog event to a reserved channel slot.
    pub fn flush_one(&self, permit: mpsc::Permit<'_, NetworkEvent>) {
        let mut backlog = self.backlog.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(ev) = backlog.pop_front() {
            permit.send(ev);
        }
    }

    /// Drop the backlog (the node side closed the channel).
    pub fn clear_backlog(&self) {
        self.backlog
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }
}
