//! INC-I-237 M12.3 / REQ-GRV-SEC-021 — the mined-check negative cache cap, checked
//! from inside the crate so `MinedCheck::record` need not be reachable from outside it.
//!
//! OUTPUT CONTRACT: MinedCheck::{record, cached_len, is_cached}
//!   O1: none   O2: the instance's pins   O3: cached_len, is_cached   O4-O6: none
//! PATHS: 10,001 records -> 10,000 kept, oldest gone, other instance empty
//! INPUT PARTITIONS: records {cap + 1 distinct txids}; instances {recording, other}.
// covers: zero_flow_mined zero_flow_budget

use crypto::Hash;

use crate::zero_flow_mined::{MinedCheck, MINED_CACHE_CAP};

fn txid(i: u32) -> Hash {
    let mut b = [0u8; 32];
    b[..4].copy_from_slice(&i.to_le_bytes());
    Hash::from_bytes(b)
}

// REQ-GRV-SEC-017 (FM3), REQ-GRV-SEC-021 — Decision: a failure means the negative cache is unbounded, evicts the newest instead of the oldest, or is one process-wide store instead of one per node.
#[test]
fn m12_2_sec_017_cache_keeps_the_newest_10_000_per_instance() {
    let check = MinedCheck::default();
    let other = MinedCheck::default();
    let n = MINED_CACHE_CAP as u32 + 1;
    for i in 0..n {
        check.record(txid(i), u64::from(i), Hash::from_bytes([0xB7; 32]));
    }
    let mut bad = Vec::new();
    if MINED_CACHE_CAP != 10_000 {
        bad.push(format!("cap={MINED_CACHE_CAP}"));
    }
    if check.cached_len() != MINED_CACHE_CAP {
        bad.push(format!("cached_len={}", check.cached_len()));
    }
    if check.is_cached(&txid(0)) {
        bad.push("oldest entry still cached".to_string());
    }
    if !check.is_cached(&txid(1)) || !check.is_cached(&txid(n - 1)) {
        bad.push("second or newest entry evicted".to_string());
    }
    if other.cached_len() != 0 || other.is_cached(&txid(n - 1)) {
        bad.push(format!("other instance holds {}", other.cached_len()));
    }
    assert!(bad.is_empty(), "REQ-GRV-SEC-017 cap: {bad:#?}");
}
