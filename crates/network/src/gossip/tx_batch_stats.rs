//! Size distribution of received tx gossip batches (INC-I-237 M0).
//! Plain atomics: the node delta-scrapes them into Prometheus.

use std::sync::atomic::{AtomicU64, Ordering};

/// Prometheus `size` label per bucket, in `bucket_index` order.
pub const TX_BATCH_BUCKETS: [&str; 7] =
    ["1", "2_5", "6_10", "11_25", "26_50", "51_100", "101_plus"];

/// Process-wide stats the swarm records into.
pub static TX_GOSSIP_BATCH_STATS: TxGossipBatchStats = TxGossipBatchStats::new();

pub struct TxGossipBatchStats {
    msgs: AtomicU64,
    txs: AtomicU64,
    buckets: [AtomicU64; 7],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TxBatchSnapshot {
    pub msgs: u64,
    pub txs: u64,
    pub buckets: [u64; 7],
}

/// Bucket of a batch of `n_txs` (0 is never recorded).
pub fn bucket_index(n_txs: usize) -> usize {
    match n_txs {
        0..=1 => 0,
        2..=5 => 1,
        6..=10 => 2,
        11..=25 => 3,
        26..=50 => 4,
        51..=100 => 5,
        _ => 6,
    }
}

impl TxGossipBatchStats {
    pub const fn new() -> Self {
        Self {
            msgs: AtomicU64::new(0),
            txs: AtomicU64::new(0),
            buckets: [const { AtomicU64::new(0) }; 7],
        }
    }

    /// Count one received batch message of `n_txs` txs; `0` is a no-op.
    pub fn record(&self, n_txs: usize) {
        let seen = u64::from(n_txs > 0);
        self.msgs.fetch_add(seen, Ordering::Relaxed);
        self.txs.fetch_add(n_txs as u64, Ordering::Relaxed);
        self.buckets[bucket_index(n_txs)].fetch_add(seen, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> TxBatchSnapshot {
        TxBatchSnapshot {
            msgs: self.msgs.load(Ordering::Relaxed),
            txs: self.txs.load(Ordering::Relaxed),
            buckets: std::array::from_fn(|i| self.buckets[i].load(Ordering::Relaxed)),
        }
    }
}

impl Default for TxGossipBatchStats {
    fn default() -> Self {
        Self::new()
    }
}
