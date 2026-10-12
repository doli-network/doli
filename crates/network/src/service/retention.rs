//! INC-I-237 M7.1: the swarm's cache sizes, refreshed on an existing swarm tick
//! and read by node metrics and the soak harness.

use std::sync::Mutex;

use libp2p::gossipsub::RetentionStats;

/// Sizes of the swarm-owned caches at the last refresh.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RetentionSnapshot {
    pub gossipsub: RetentionStats,
    pub rate_limit_peers: usize,
}

/// Last [`RetentionSnapshot`] written by the swarm task.
#[derive(Debug, Default)]
pub struct Retention(Mutex<RetentionSnapshot>);

impl Retention {
    /// The last refreshed sizes (all zero before the first refresh).
    pub fn snapshot(&self) -> RetentionSnapshot {
        *self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(super) fn store(&self, snap: RetentionSnapshot) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = snap;
    }
}
