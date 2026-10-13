//! INC-I-237 M7.2 (REQ-GRV-027 / REQ-GRV-028) — count cap with oldest-first
//! eviction on the vendor duplicate cache and peer-score `deliveries` cache.
//!
//! covers: vendor/libp2p-gossipsub/src/time_cache.rs vendor/libp2p-gossipsub/src/peer_score.rs
//!   vendor/libp2p-gossipsub/src/behaviour.rs vendor/libp2p-gossipsub/src/config.rs
//!   crates/network/src/gossip/config.rs
//!
//! OUTPUT CONTRACT: vendor DuplicateCache::{new_capped, new}
//!   O3: insert -> bool (true = key was absent); stats() entries / map_capacity /
//!       deque_capacity / map_slot_bytes / deque_slot_bytes / max_entries
//!   new_gossipsub / new_gossipsub_with_cache_time -> Behaviour
//!   O3: retention_stats().duplicate_cache.max_entries,
//!       retention_stats().peer_score_deliveries.max_entries (+ slot bytes)
//!   O5: consts GOSSIP_DUPLICATE_CACHE_MAX_ENTRIES, PEER_SCORE_DELIVERIES_MAX_ENTRIES
//! PATHS: P1 new key at len == cap -> oldest evicted, newest kept; P1b duplicate
//!   at cap -> no eviction; P2 TTL expiry below the cap; P3 honest count < cap at
//!   the production TTL -> nothing evicted; P4 both production builders wire both
//!   caps; P5 churn at the cap -> modelled bytes inside the budget
//! INPUT PARTITIONS: cap {1_000, production}; ttl {100 ms, 3_600 s, production};
//!   inserts {< cap, == cap, 2 x cap, 4 x cap}; builder {default TTL, custom TTL};
//!   constructor {new_capped, new (uncapped)}
//! MATRIX: P1 x {small, production} cap x {entries, newest present, oldest absent};
//!   P1b x small cap; P2 x {absent, entries}; P3 x {entries, all present};
//!   P4 x 2 builders x 2 caps; P5 x {dup cache (empirical churn), deliveries
//!   (churn rule checked against the empirical dup cache)}.

use std::time::Duration;

use libp2p::gossipsub::{DuplicateCache, MessageId, TimeCacheStats};
use libp2p::identity::Keypair;
use network::gossip::{
    new_gossipsub, new_gossipsub_with_cache_time, MeshConfig, GOSSIP_DUPLICATE_CACHE_MAX_ENTRIES,
    GOSSIP_DUPLICATE_CACHE_TIME, PEER_SCORE_DELIVERIES_MAX_ENTRIES,
};

const MIB: usize = 1 << 20;
const ID_BYTES: usize = 20;
const LONG_TTL: Duration = Duration::from_secs(3_600);
const SMALL_CAP: usize = 1_000;
const DUP_CACHE_BUDGET: usize = 32 * MIB;
const DELIVERIES_BUDGET: usize = 20 * MIB;
/// Gate "a few hundred ids/s" (upper end): Q6 100 tx/s + attestations at producers/slot.
const HONEST_IDS_PER_S: usize = 500;
/// Seconds an honest id must stay deduplicated (production TTL is 300 s).
const HONEST_WINDOW_SECS: usize = 299;
/// M7.1 600 s soak, B peer_score_deliveries row: entries at end, model bytes per entry.
const M71_DELIVERIES_ENTRIES: usize = 216_114;
const M71_DELIVERIES_BYTES_PER_ENTRY: f64 = 382.1;
/// hashbrown trailing control bytes (widest SIMD group), as in the soak model.
const GROUP_WIDTH: usize = 16;

fn id(i: u64) -> MessageId {
    let mut b = [0x2Bu8; ID_BYTES];
    b[..8].copy_from_slice(&i.to_le_bytes());
    MessageId::new(&b)
}

fn mesh() -> MeshConfig {
    MeshConfig {
        mesh_n: 6,
        mesh_n_low: 4,
        mesh_n_high: 12,
        gossip_lazy: 6,
    }
}

/// hashbrown buckets behind a reported `capacity()` (insert-only sizing for `n` items).
fn buckets(capacity: usize) -> usize {
    match capacity {
        0 => 0,
        1..=7 => (capacity + 1).next_power_of_two(),
        _ => (capacity * 8 / 7).next_power_of_two(),
    }
}

/// Buckets a FIFO-churned map of `n` live items reaches: tombstones force one doubling
/// when `n` exceeds half the insert-only table's full capacity.
fn churned_buckets(n: usize) -> usize {
    let b0 = buckets(n);
    if n > b0 * 7 / 16 {
        2 * b0
    } else {
        b0
    }
}

fn deque_capacity(n: usize) -> usize {
    n.next_power_of_two().max(4)
}

/// M7.1 calibration model: map table + expiry deque + two heap copies of each 20-byte id.
fn model_bytes(buckets: usize, s: &TimeCacheStats, deque_cap: usize, entries: usize) -> usize {
    buckets * (s.map_slot_bytes + 1)
        + GROUP_WIDTH
        + deque_cap * s.deque_slot_bytes
        + 2 * ID_BYTES * entries
}

fn mib(bytes: f64) -> f64 {
    bytes / MIB as f64
}

/// Production-TTL, production-cap duplicate cache after 4 x cap distinct inserts.
fn churned_dup_cache() -> TimeCacheStats {
    let cap = GOSSIP_DUPLICATE_CACHE_MAX_ENTRIES;
    let mut c = DuplicateCache::new_capped(GOSSIP_DUPLICATE_CACHE_TIME, cap);
    for i in 0..(4 * cap) as u64 {
        c.insert(id(i));
    }
    c.stats()
}

fn fill_twice_and_check(cap: usize, ttl: Duration) {
    let mut c = DuplicateCache::new_capped(ttl, cap);
    assert_eq!(c.stats().max_entries, cap, "stats reports the cap");
    let fresh = (0..(2 * cap) as u64).filter(|i| c.insert(id(*i))).count();
    assert_eq!(fresh, 2 * cap, "every distinct id is new on first insert");
    assert_eq!(
        c.stats().entries,
        cap,
        "entries pinned at the cap after 2 x cap inserts"
    );
    for i in cap as u64..(2 * cap) as u64 {
        assert!(!c.insert(id(i)), "newest id {i} evicted (cap {cap})");
    }
    assert!(
        c.insert(id(0)),
        "oldest id 0 still present after 2 x cap inserts (cap {cap})"
    );
    assert!(
        c.insert(id(cap as u64 - 1)),
        "id {} still present: eviction is not oldest-first (cap {cap})",
        cap - 1
    );
    assert_eq!(
        c.stats().entries,
        cap,
        "re-inserts at the cap keep entries == cap"
    );
}

// REQ-GRV-028 — Decision: a failure means the vendor cache no longer evicts oldest-first at the count cap, so a distinct-id flood grows it without bound again (F-13) or drops fresh ids before old ones.
#[test]
fn inc_i_237_m7_2_cap_evicts_oldest_small_cap() {
    let mut c = DuplicateCache::new_capped(LONG_TTL, SMALL_CAP);
    for i in 0..SMALL_CAP as u64 {
        assert!(c.insert(id(i)), "id {i} new below the cap");
    }
    assert!(!c.insert(id(0)), "P1b: duplicate at the cap reads as new");
    assert_eq!(
        c.stats().entries,
        SMALL_CAP,
        "P1b: a duplicate at the cap changed entries"
    );
    for i in 0..SMALL_CAP as u64 {
        assert!(
            !c.insert(id(i)),
            "P1b: id {i} evicted by a duplicate insert"
        );
    }
    fill_twice_and_check(SMALL_CAP, LONG_TTL);
    let uncapped = DuplicateCache::<MessageId>::new(LONG_TTL);
    assert_eq!(
        uncapped.stats().max_entries,
        usize::MAX,
        "uncapped `new` must report usize::MAX"
    );
}

// REQ-GRV-028 — Decision: a failure means the production cap constant does not bound the production-TTL cache, so B's dup cache re-grows past its budget under the soak flood.
#[test]
fn inc_i_237_m7_2_cap_evicts_oldest_production_cap() {
    fill_twice_and_check(
        GOSSIP_DUPLICATE_CACHE_MAX_ENTRIES,
        GOSSIP_DUPLICATE_CACHE_TIME,
    );
}

// REQ-GRV-027 — Decision: a failure means adding the cap broke TTL expiry below the cap, so ids outlive their TTL or the cache never frees under normal load.
#[test]
fn inc_i_237_m7_2_ttl_expiry_below_cap() {
    let ttl = Duration::from_millis(100);
    let mut c = DuplicateCache::new_capped(ttl, SMALL_CAP);
    for i in 0..10u64 {
        assert!(c.insert(id(i)));
    }
    assert_eq!(c.stats().entries, 10);
    std::thread::sleep(ttl * 3);
    assert!(c.insert(id(1_000_000)), "trigger id is new");
    assert_eq!(
        c.stats().entries,
        1,
        "expired ids not removed on insert below the cap"
    );
    for i in 0..10u64 {
        assert!(
            c.insert(id(i)),
            "id {i} survived past its TTL below the cap"
        );
    }
}

// REQ-GRV-027 — Decision: a failure means the cap evicts ids an honest network still needs inside the 300 s TTL, re-opening the INC-I-142 stale re-forward class under normal load.
#[test]
fn inc_i_237_m7_2_honest_load_fully_deduplicated() {
    let cap = GOSSIP_DUPLICATE_CACHE_MAX_ENTRIES;
    let n = HONEST_IDS_PER_S * HONEST_WINDOW_SECS;
    assert!(
        GOSSIP_DUPLICATE_CACHE_TIME.as_secs() as usize > HONEST_WINDOW_SECS,
        "production TTL {:?} shorter than the honest window",
        GOSSIP_DUPLICATE_CACHE_TIME
    );
    assert!(
        n < cap,
        "honest {HONEST_IDS_PER_S} ids/s x {HONEST_WINDOW_SECS} s = {n} ids >= cap {cap}: the cap evicts honest ids inside the TTL"
    );
    let mut c = DuplicateCache::new_capped(GOSSIP_DUPLICATE_CACHE_TIME, cap);
    for i in 0..n as u64 {
        assert!(c.insert(id(i)));
    }
    assert_eq!(c.stats().entries, n, "honest ids evicted below the cap");
    for i in 0..n as u64 {
        assert!(
            !c.insert(id(i)),
            "honest id {i} not deduplicated inside the TTL"
        );
    }
}

// REQ-GRV-028 — Decision: a failure means a production gossipsub builder runs an uncapped dup cache or deliveries cache, so the node re-grows past 64 MiB under a distinct-id flood.
#[tokio::test]
async fn inc_i_237_m7_2_production_builders_wire_both_caps() {
    const {
        assert!(
            GOSSIP_DUPLICATE_CACHE_MAX_ENTRIES < usize::MAX
                && PEER_SCORE_DELIVERIES_MAX_ENTRIES < usize::MAX,
            "a cap constant is the uncapped sentinel"
        )
    };
    let kp = Keypair::generate_ed25519();
    let built = [
        ("new_gossipsub", new_gossipsub(&kp, &mesh())),
        (
            "new_gossipsub_with_cache_time",
            new_gossipsub_with_cache_time(&kp, &mesh(), Duration::from_secs(5)),
        ),
    ];
    for (name, b) in built {
        let r = b.expect("production gossipsub").retention_stats();
        assert_eq!(
            r.duplicate_cache.max_entries, GOSSIP_DUPLICATE_CACHE_MAX_ENTRIES,
            "{name}: duplicate cache cap not wired"
        );
        assert_eq!(
            r.peer_score_deliveries.max_entries, PEER_SCORE_DELIVERIES_MAX_ENTRIES,
            "{name}: peer-score deliveries cap not wired (peer scoring off?)"
        );
    }
}

// REQ-GRV-028 — Decision: a failure means the dup-cache cap, under steady churn at the cap, allocates more than its 32 MiB share of B's 64 MiB budget.
#[test]
fn inc_i_237_m7_2_dup_cache_churned_model_bytes_within_budget() {
    let cap = GOSSIP_DUPLICATE_CACHE_MAX_ENTRIES;
    let s = churned_dup_cache();
    assert_eq!(s.entries, cap, "churn did not fill to the cap: {s:?}");
    let b = buckets(s.map_capacity);
    let bytes = model_bytes(b, &s, s.deque_capacity, s.entries);
    println!(
        "M7.2 dup_cache cap={cap} map_capacity={} buckets={b} (insert-only {}) deque_capacity={} model_mib={:.1}",
        s.map_capacity,
        buckets(cap),
        s.deque_capacity,
        mib(bytes as f64)
    );
    assert!(
        bytes <= DUP_CACHE_BUDGET,
        "dup cache at cap {cap} after 4 x cap churn models {:.1} MiB (buckets {b}, deque {}) > 32 MiB",
        mib(bytes as f64),
        s.deque_capacity
    );
}

// REQ-GRV-028 — Decision: a failure means the deliveries cap, at the bucket count a churned map reaches, allocates more than its 20 MiB share of B's 64 MiB budget.
#[tokio::test]
async fn inc_i_237_m7_2_deliveries_model_bytes_within_budget() {
    let cap = PEER_SCORE_DELIVERIES_MAX_ENTRIES;
    let kp = Keypair::generate_ed25519();
    let d = new_gossipsub(&kp, &mesh())
        .expect("production gossipsub")
        .retention_stats()
        .peer_score_deliveries;
    assert!(
        d.map_slot_bytes > 0 && d.deque_slot_bytes > 0,
        "deliveries slot sizes unreported (peer scoring off): {d:?}"
    );
    let dup_cap = GOSSIP_DUPLICATE_CACHE_MAX_ENTRIES;
    let dup = churned_dup_cache();
    let dup_buckets = buckets(dup.map_capacity);
    assert!(
        dup_buckets <= churned_buckets(dup_cap),
        "churn rule under-estimates: dup cache reached {dup_buckets} buckets > rule {}",
        churned_buckets(dup_cap)
    );
    let pinned = churned_buckets(dup_cap) > buckets(dup_cap) && dup_buckets == buckets(dup_cap);
    let rule = |n: usize| {
        if pinned {
            buckets(n)
        } else {
            churned_buckets(n)
        }
    };
    let model = |n: usize| model_bytes(rule(n), &d, deque_capacity(n), n) as f64;
    let m71 = model(M71_DELIVERIES_ENTRIES) / M71_DELIVERIES_ENTRIES as f64;
    let allowance = (M71_DELIVERIES_BYTES_PER_ENTRY - m71).max(0.0);
    let bytes = model(cap) + cap as f64 * allowance;
    println!(
        "M7.2 deliveries cap={cap} buckets={} pinned={pinned} map_slot={} deque_slot={} m71_model_per_entry={m71:.1} allowance_per_record={allowance:.1} model_mib={:.1}",
        rule(cap),
        d.map_slot_bytes,
        d.deque_slot_bytes,
        mib(bytes)
    );
    assert!(
        bytes <= DELIVERIES_BUDGET as f64,
        "deliveries at cap {cap} models {:.1} MiB (buckets {}, +{allowance:.1} B/record) > 20 MiB",
        mib(bytes),
        rule(cap)
    );
}
