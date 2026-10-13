//! INC-I-237 M7.1 (REQ-GRV-026 step 2) — bytes per entry of THE vendor
//! duplicate cache, measured by this binary's counting allocator.
//!
//! covers: vendor/libp2p-gossipsub/src/time_cache.rs vendor/libp2p-gossipsub/src/lib.rs
//!
//! OUTPUT CONTRACT: DuplicateCache<MessageId> (vendor)
//!   O3: `insert` return; `stats()` entries / map_capacity / deque_capacity /
//!       map_slot_bytes / deque_slot_bytes
//!   O5: live-heap delta of the filled cache (CURRENT)
//! PATHS: P1 N distinct ids -> every insert new; P2 a re-inserted id -> not new,
//!   entries unchanged
//! INPUT PARTITIONS: N {100_000, 500_000}, fresh cache each; 20-byte ids
//! MATRIX: O3 x P1-P2, O5 x P1 (measured vs the layout model of
//!   `attribution::time_cache_bytes`, which the soak table uses).

use std::mem::size_of;
use std::sync::atomic::Ordering::Relaxed;
use std::time::Duration;

use libp2p::gossipsub::{DuplicateCache, MessageId};

use crate::attribution::{time_cache_bytes, ID_BYTES};
use crate::{CURRENT, SERIAL};

fn id(i: u64) -> MessageId {
    let mut b = [0x1Du8; ID_BYTES];
    b[..8].copy_from_slice(&i.to_le_bytes());
    MessageId::new(&b)
}

fn calibrate(n: usize) {
    let before = CURRENT.load(Relaxed);
    let mut cache = DuplicateCache::new(Duration::from_secs(300));
    let fresh = (0..n as u64).filter(|i| cache.insert(id(*i))).count();
    let measured = CURRENT.load(Relaxed) as i64 - before as i64;
    let s = cache.stats();
    let model = time_cache_bytes(&s);
    let per_entry = measured as f64 / n as f64;
    println!(
        "CALIBRATION n={n} bytes={measured} bytes_per_entry={per_entry:.1} model_bytes={model} map_cap={} deque_cap={} map_slot={} deque_slot={}",
        s.map_capacity, s.deque_capacity, s.map_slot_bytes, s.deque_slot_bytes
    );
    assert_eq!(fresh, n, "P1: every distinct id is new");
    assert_eq!(s.entries, n, "P1: entries after {n} distinct inserts");
    assert!(!cache.insert(id(0)), "P2: a re-inserted id is not new");
    assert_eq!(cache.stats().entries, n, "P2: a duplicate adds no entry");
    assert!(
        s.map_capacity >= n && s.deque_capacity >= n,
        "capacities below entries: {s:?}"
    );
    assert!(
        s.map_slot_bytes >= size_of::<MessageId>() && s.deque_slot_bytes >= size_of::<MessageId>(),
        "a slot holds at least one MessageId: {s:?}"
    );
    assert!(
        per_entry.is_finite() && per_entry > 0.0,
        "bytes per entry {per_entry}"
    );
    let ratio = measured as f64 / model as f64;
    assert!(
        (0.9..=1.1).contains(&ratio),
        "measured {measured} B vs layout model {model} B (ratio {ratio:.3}) at n={n}"
    );
}

// REQ-GRV-026 — Decision: a failure means the layout model the soak uses to turn duplicate-cache counts into bytes disagrees with what the allocator measures for the vendor type, so the attribution table would mis-assign the retained heap.
#[test]
fn req_grv_026_duplicate_cache_bytes_per_entry_calibration() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    calibrate(100_000);
    calibrate(500_000);
}
