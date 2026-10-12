//! INC-I-237 M7.1 (REQ-GRV-026) — attribution of the soak's retained heap per
//! (swarm, structure): B = node under test; A1-A3, H, C = harness swarms.
//!
//! covers: vendor/libp2p-gossipsub/src/behaviour.rs vendor/libp2p-gossipsub/src/time_cache.rs
//!   vendor/libp2p-gossipsub/src/mcache.rs vendor/libp2p-gossipsub/src/peer_score.rs
//!   vendor/libp2p-gossipsub/src/lib.rs crates/network/src/service/swarm_loop.rs
//!   crates/network/src/service/command_api.rs crates/network/src/service/mod.rs
//!   crates/network/src/service/retention.rs crates/network/src/gossip/config.rs
//!   bins/node/src/node/mod.rs bins/node/src/node/attestation/ingress.rs
//!   crates/network/src/rate_limit.rs crates/network/src/service/types.rs
//!
//! OUTPUT CONTRACT: soak `run` attribution (O5, process heap by owner)
//!   per row: entries at warm / end, bytes per entry at end, bytes(end) - bytes(warm);
//!   totals: B, harness, attributed, unattributed = retained - attributed
//! PATHS: P7 every swarm's duplicate cache is non-empty at end (rows are live);
//!   P8 attributed >= 90% of retained (30 s soak)
//! INPUT PARTITIONS: swarm {B, A1, A2, A3, H, C} x structure {dup_cache,
//!   published_ids, peer_score_deliveries, mcache}; B only {attest_seen,
//!   parent_sig_pool, attest_fetch_tracker, rate_limit_peers, event_lanes, mempool}
//! MATRIX: P7 x every swarm, P8 x the summed rows — asserted in `check_attribution`.

use std::mem::size_of;
use std::time::Instant;

use crypto::{Hash, PublicKey};
use doli_node::node::Node;
use libp2p::gossipsub::{RetentionStats, TimeCacheStats};
use libp2p::PeerId;
use network::service::Retention;
use tokio::sync::oneshot;

use crate::{Attacker, MIB};

/// `gossip/config.rs` message ids: the first 20 bytes of BLAKE3.
pub const ID_BYTES: usize = 20;
/// hashbrown trailing control bytes (widest SIMD group).
const GROUP_WIDTH: usize = 16;
pub const NODE: &str = "B";
pub const SWARMS: [&str; 6] = ["B", "A1", "A2", "A3", "H", "C"];

#[derive(Clone, Copy, Debug, Default)]
pub struct Cell {
    pub entries: usize,
    pub bytes: usize,
}

pub type Sample = Vec<(&'static str, &'static str, Cell)>;

/// hashbrown allocation for a table whose `capacity()` is `capacity`.
pub fn hash_map_bytes(capacity: usize, slot_bytes: usize) -> usize {
    if capacity == 0 {
        return 0;
    }
    let buckets = if capacity < 8 {
        capacity + 1
    } else {
        capacity * 8 / 7
    }
    .next_power_of_two();
    buckets * (slot_bytes + 1) + GROUP_WIDTH
}

/// Map table + expiry deque + the two heap copies of each 20-byte id.
pub fn time_cache_bytes(s: &TimeCacheStats) -> usize {
    hash_map_bytes(s.map_capacity, s.map_slot_bytes)
        + s.deque_capacity * s.deque_slot_bytes
        + 2 * ID_BYTES * s.entries
}

fn cache(s: &TimeCacheStats) -> Cell {
    Cell {
        entries: s.entries,
        bytes: time_cache_bytes(s),
    }
}

/// Small B-side maps at the minimum load factor (a lower bound).
fn map_entries<T>(entries: usize) -> Cell {
    Cell {
        entries,
        bytes: entries * (size_of::<T>() + 1) * 8 / 7,
    }
}

fn gossipsub(swarm: &'static str, r: &RetentionStats, out: &mut Sample) {
    out.push((swarm, "dup_cache", cache(&r.duplicate_cache)));
    out.push((swarm, "published_ids", cache(&r.published_message_ids)));
    out.push((
        swarm,
        "peer_score_deliveries",
        cache(&r.peer_score_deliveries),
    ));
    out.push((
        swarm,
        "mcache",
        Cell {
            entries: r.mcache_entries,
            bytes: r.mcache_bytes,
        },
    ));
}

pub async fn attacker_retention(a: &Attacker) -> RetentionStats {
    let (tx, rx) = oneshot::channel();
    a.retention.send(tx).await.expect("attacker task alive");
    rx.await.expect("attacker retention reply")
}

/// One snapshot of every attributed structure, B first.
pub async fn sample(b: &Node, attackers: &[Attacker], h: &Retention, c: &Retention) -> Sample {
    let mut out = Sample::new();
    let net = b.network.as_ref().expect("B network");
    let snap = net.retention().snapshot();
    gossipsub(NODE, &snap.gossipsub, &mut out);
    let lanes = net.lane_stats().snapshot();
    let mempool = b.mempool.read().await;
    out.extend([
        (
            NODE,
            "attest_seen",
            map_entries::<((PublicKey, u32), bool)>(b.attest_seen_len()),
        ),
        (
            NODE,
            "parent_sig_pool",
            map_entries::<(PublicKey, [u8; 96])>(b.parent_sig_pool.total_signatures()),
        ),
        (
            NODE,
            "attest_fetch_tracker",
            map_entries::<(Hash, (Instant, u8, PeerId))>(b.attest_fetch_tracker.len()),
        ),
        (
            NODE,
            "rate_limit_peers",
            map_entries::<PeerId>(snap.rate_limit_peers),
        ),
        (
            NODE,
            "event_lanes",
            Cell {
                entries: lanes.consensus_len + lanes.bulk_len + lanes.backlog_len,
                bytes: lanes.consensus_bytes + lanes.bulk_bytes + lanes.backlog_bytes,
            },
        ),
        (
            NODE,
            "mempool",
            Cell {
                entries: mempool.len(),
                bytes: mempool.size(),
            },
        ),
    ]);
    drop(mempool);
    for (name, a) in ["A1", "A2", "A3"].into_iter().zip(attackers) {
        gossipsub(name, &attacker_retention(a).await, &mut out);
    }
    gossipsub("H", &h.snapshot().gossipsub, &mut out);
    gossipsub("C", &c.snapshot().gossipsub, &mut out);
    out
}

pub struct Row {
    pub swarm: &'static str,
    pub structure: &'static str,
    pub warm: Cell,
    pub end: Cell,
}

impl Row {
    pub fn delta(&self) -> i64 {
        self.end.bytes as i64 - self.warm.bytes as i64
    }
}

pub struct Attribution {
    pub rows: Vec<Row>,
    pub retained: i64,
}

impl Attribution {
    pub fn new(warm: &Sample, end: &Sample, retained: i64) -> Self {
        let rows = end
            .iter()
            .map(|&(swarm, structure, end)| {
                let warm = warm
                    .iter()
                    .find(|(s, st, _)| *s == swarm && *st == structure)
                    .map(|r| r.2)
                    .unwrap_or_default();
                Row {
                    swarm,
                    structure,
                    warm,
                    end,
                }
            })
            .collect();
        Self { rows, retained }
    }

    pub fn attributed(&self) -> i64 {
        self.rows.iter().map(Row::delta).sum()
    }

    fn total(&self, node: bool) -> i64 {
        self.rows
            .iter()
            .filter(|r| (r.swarm == NODE) == node)
            .map(Row::delta)
            .sum()
    }

    pub fn print(&self) {
        let mib = |b: i64| b as f64 / MIB as f64;
        for r in &self.rows {
            let per = r.end.bytes as f64 / r.end.entries.max(1) as f64;
            println!(
                "ATTRIBUTION swarm={} structure={} entries_warm={} entries_end={} bytes_per_entry={per:.1} delta_mib={:.3}",
                r.swarm, r.structure, r.warm.entries, r.end.entries, mib(r.delta())
            );
        }
        println!("ATTRIBUTION total=B delta_mib={:.3}", mib(self.total(true)));
        println!(
            "ATTRIBUTION total=harness delta_mib={:.3}",
            mib(self.total(false))
        );
        let attributed = self.attributed();
        println!(
            "ATTRIBUTION retained_mib={:.3} attributed_mib={:.3} unattributed_mib={:.3} coverage={:.3}",
            mib(self.retained),
            mib(attributed),
            mib(self.retained - attributed),
            attributed as f64 / self.retained as f64
        );
    }
}

pub fn check_attribution(a: &Attribution) {
    for swarm in SWARMS {
        let row = a
            .rows
            .iter()
            .find(|r| r.swarm == swarm && r.structure == "dup_cache")
            .unwrap_or_else(|| panic!("REQ-GRV-026: no duplicate-cache row for {swarm}"));
        assert!(
            row.end.entries > 0,
            "REQ-GRV-026: {swarm} duplicate cache reads 0 entries after the flood"
        );
    }
    assert!(
        a.retained > 0,
        "REQ-GRV-026 non-vacuity: retained {} B",
        a.retained
    );
    let coverage = a.attributed() as f64 / a.retained as f64;
    assert!(
        coverage >= 0.9,
        "REQ-GRV-026: attributed rows cover {coverage:.3} of retained {} B (< 0.90)",
        a.retained
    );
}
