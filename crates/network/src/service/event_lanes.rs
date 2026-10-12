//! Node-side receiver of the two event lanes (INC-I-237 R1).

use std::sync::Arc;

use tokio::sync::mpsc;

use super::event_sink::{Lane, LaneItem, LaneStats};
use super::types::NetworkEvent;

/// Consensus-first receiver; releases each event's lane bytes and depth on dequeue.
pub struct EventLanes {
    consensus: mpsc::Receiver<LaneItem>,
    bulk: mpsc::Receiver<LaneItem>,
    stats: Arc<LaneStats>,
}

impl EventLanes {
    pub(super) fn new(
        consensus: mpsc::Receiver<LaneItem>,
        bulk: mpsc::Receiver<LaneItem>,
        stats: Arc<LaneStats>,
    ) -> Self {
        Self {
            consensus,
            bulk,
            stats,
        }
    }

    fn take(&self, lane: Lane, (ev, size): LaneItem) -> NetworkEvent {
        self.stats.dequeued(lane, size);
        ev
    }

    /// Next event, consensus lane first; `None` once both lanes are closed and empty.
    pub async fn recv(&mut self) -> Option<NetworkEvent> {
        let (lane, item) = tokio::select! {
            biased;
            Some(item) = self.consensus.recv() => (Lane::Consensus, item),
            Some(item) = self.bulk.recv() => (Lane::Bulk, item),
            else => return None,
        };
        Some(self.take(lane, item))
    }

    /// Next queued event without waiting, consensus lane first.
    pub fn try_recv(&mut self) -> Option<NetworkEvent> {
        if let Ok(item) = self.consensus.try_recv() {
            return Some(self.take(Lane::Consensus, item));
        }
        let item = self.bulk.try_recv().ok()?;
        Some(self.take(Lane::Bulk, item))
    }
}
