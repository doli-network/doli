//! INC-I-237 M7.2 (REQ-GRV-007 / REQ-GRV-027) — node-under-test bounds of the
//! soak: B-only retained heap and B's duplicate-cache retention under the flood.
//!
//! covers: vendor/libp2p-gossipsub/src/time_cache.rs vendor/libp2p-gossipsub/src/peer_score.rs
//!   vendor/libp2p-gossipsub/src/behaviour.rs crates/network/src/gossip/config.rs
//!   crates/network/src/service/retention.rs
//!
//! OUTPUT CONTRACT: soak `run` -> Bound (O5 heap by owner; O3 B's cache stats)
//!   b_retained = retained - sum(harness rows) (unattributed charged to B);
//!   effective_retention_s = min(TTL, B dup max_entries / ingress ids/s)
//! PATHS: P4 B-only retained < 64 MiB; P9 dup-cache retention >= block stale
//!   window; P10 B's dup / deliveries entries <= their caps
//! INPUT PARTITIONS: duration {SOAK_SECS 30 default, 600 soak_10min}
//! MATRIX: P4, P9, P10 x both durations, asserted in `check_bound`.

use libp2p::gossipsub::TimeCacheStats;
use network::gossip::validation::STALE_BLOCK_SLOT_THRESHOLD;
use network::gossip::GOSSIP_DUPLICATE_CACHE_TIME;

use crate::attribution::{Attribution, Row, NODE};
use crate::MIB;

const B_RETAINED_BOUND_MIB: f64 = 64.0;

pub struct Bound {
    pub retained: i64,
    pub harness: i64,
    pub dup: TimeCacheStats,
    pub deliveries: TimeCacheStats,
    pub ingress_ids_per_s: f64,
}

fn mib(b: i64) -> f64 {
    b as f64 / MIB as f64
}

fn block_stale_window_s() -> f64 {
    (u64::from(STALE_BLOCK_SLOT_THRESHOLD) * doli_core::consensus::SLOT_DURATION) as f64
}

impl Bound {
    pub fn new(
        table: &Attribution,
        dup: TimeCacheStats,
        deliveries: TimeCacheStats,
        ingress_ids_per_s: f64,
    ) -> Self {
        let harness = table
            .rows
            .iter()
            .filter(|r| r.swarm != NODE)
            .map(Row::delta)
            .sum();
        Self {
            retained: table.retained,
            harness,
            dup,
            deliveries,
            ingress_ids_per_s,
        }
    }

    pub fn b_retained(&self) -> i64 {
        self.retained - self.harness
    }

    pub fn effective_retention_s(&self) -> f64 {
        let by_cap = self.dup.max_entries as f64 / self.ingress_ids_per_s;
        by_cap.min(GOSSIP_DUPLICATE_CACHE_TIME.as_secs_f64())
    }

    pub fn print(&self) {
        println!(
            "BOUND retained_mib={:.3} harness_mib={:.3} (printed, not counted)",
            mib(self.retained),
            mib(self.harness)
        );
        println!("b_retained_mib={:.1}", mib(self.b_retained()));
        println!(
            "BOUND b_dup entries={} max_entries={} b_deliveries entries={} max_entries={} ingress_ids_per_s={:.1} effective_retention_s={:.1} stale_window_s={:.0}",
            self.dup.entries,
            self.dup.max_entries,
            self.deliveries.entries,
            self.deliveries.max_entries,
            self.ingress_ids_per_s,
            self.effective_retention_s(),
            block_stale_window_s()
        );
    }
}

pub fn check_bound(b: &Bound) {
    assert!(
        b.ingress_ids_per_s > 0.0 && b.dup.entries > 0,
        "non-vacuity: no ingress into B ({:.1} ids/s, {} dup entries)",
        b.ingress_ids_per_s,
        b.dup.entries
    );
    assert!(
        mib(b.b_retained()) < B_RETAINED_BOUND_MIB,
        "REQ-GRV-007: node-under-test retained {:.1} MiB >= 64 MiB (retained {:.1}, harness {:.1} not counted)",
        mib(b.b_retained()),
        mib(b.retained),
        mib(b.harness)
    );
    assert!(
        b.effective_retention_s() >= block_stale_window_s(),
        "REQ-GRV-027: B dup-cache retention {:.1} s (cap {} / {:.1} ids/s) < block stale window {:.0} s",
        b.effective_retention_s(),
        b.dup.max_entries,
        b.ingress_ids_per_s,
        block_stale_window_s()
    );
    assert!(
        b.dup.entries <= b.dup.max_entries && b.deliveries.entries <= b.deliveries.max_entries,
        "REQ-GRV-007: B cache above its cap: dup {}/{}, deliveries {}/{}",
        b.dup.entries,
        b.dup.max_entries,
        b.deliveries.entries,
        b.deliveries.max_entries
    );
}
