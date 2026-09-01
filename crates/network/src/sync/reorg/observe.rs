//! INC-I-204 M0 — plain observation counters for [`super::ReorgHandler`].
//!
//! `crates/network` has no `prometheus` dependency, so the handler keeps plain
//! atomics and `bins/node` scrapes a snapshot into Prometheus through
//! `metrics::apply_reorg_observations` — the same seam
//! `storage::RocksDbMetrics` -> `metrics::apply_rocksdb_metrics` already uses.
//!
//! These counters OBSERVE branches; nothing here is read back into a decision.

use std::sync::atomic::{AtomicU64, Ordering};

/// Cumulative-since-start snapshot of one handler's observation counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReorgObservations {
    /// `check_reorg_weighted` reached the finality comparison.
    pub check_reorg_finality_entries: u64,
    /// `check_reorg_weighted` refused a reorg at the finality comparison.
    pub check_reorg_finality_rejects: u64,
    /// `plan_reorg` reached the finality comparison.
    pub plan_reorg_finality_entries: u64,
    /// `plan_reorg` refused a reorg at the finality comparison.
    pub plan_reorg_finality_rejects: u64,
    /// `record_block_with_height` took the pre-INC-I-147 branch.
    pub pre_activation_record_height: u64,
    /// `plan_reorg`'s finality comparison took the pre-INC-I-147 branch.
    pub pre_activation_plan_reorg_finality: u64,
}

/// Interior-mutable backing store: `check_reorg_weighted` and `plan_reorg` take
/// `&self`. `Relaxed` is sufficient — no other state is published through these.
#[derive(Debug, Default)]
pub(crate) struct ReorgCounters {
    pub(crate) check_reorg_finality_entries: AtomicU64,
    pub(crate) check_reorg_finality_rejects: AtomicU64,
    pub(crate) plan_reorg_finality_entries: AtomicU64,
    pub(crate) plan_reorg_finality_rejects: AtomicU64,
    pub(crate) pre_activation_record_height: AtomicU64,
    pub(crate) pre_activation_plan_reorg_finality: AtomicU64,
}

impl ReorgCounters {
    pub(crate) fn snapshot(&self) -> ReorgObservations {
        ReorgObservations {
            check_reorg_finality_entries: self.check_reorg_finality_entries.load(Ordering::Relaxed),
            check_reorg_finality_rejects: self.check_reorg_finality_rejects.load(Ordering::Relaxed),
            plan_reorg_finality_entries: self.plan_reorg_finality_entries.load(Ordering::Relaxed),
            plan_reorg_finality_rejects: self.plan_reorg_finality_rejects.load(Ordering::Relaxed),
            pre_activation_record_height: self.pre_activation_record_height.load(Ordering::Relaxed),
            pre_activation_plan_reorg_finality: self
                .pre_activation_plan_reorg_finality
                .load(Ordering::Relaxed),
        }
    }
}

/// Add one to an observation counter.
pub(crate) fn bump(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
}
