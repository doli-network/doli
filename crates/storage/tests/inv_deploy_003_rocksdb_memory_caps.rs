//! INV-DEPLOY-003 (INC-I-150) — the aggregate memory of all co-located
//! doli-node instances (per-node RocksDB caps + heap) must fit host MemTotal at
//! <= 60 %, with swap on every producer host.
//!
//! Only the per-node RocksDB caps are code. The host arithmetic (N nodes x
//! budget vs MemTotal, swap present) is an operator check and is NOT tested
//! here. This file pins the code half: every per-node cap is SET (a zero
//! `db_write_buffer_size` means "no memtable cap" in RocksDB) and the four
//! caps of one node sum to no more than the 192 MiB design point that the
//! INC-I-150 host sizing used (block_store 48 + 32, state_db 64 + 48).
//!
//! Breaking it looks like: a cap removed (`db_write_buffer_size=0`), a cache
//! or write buffer raised so one node needs more than the budget the fleet
//! hosts were sized for, or a store that no longer opens with its caps.
//!
//! Guarded files for mutation: `crates/storage/src/state_db/open.rs` and
//! `crates/storage/src/block_store/open.rs` (each test opens one store; the
//! budget test opens both). The memtable cap is read from the RocksDB OPTIONS
//! file the open writes, so it is the value open.rs really applied, not a
//! constant. The block cache capacity is not in OPTIONS; it comes from the
//! `block_cache_capacity_bytes` that open.rs stores and `metrics()` reports.
//!
//! OUTPUT CONTRACT
//!   Functions under test: `BlockStore::open(path)`, `StateDb::open(path)`.
//!   O1 open succeeds; O2 `db_write_buffer_size` in the OPTIONS-* file the
//!   open writes (the applied memtable cap); O3 `max_total_wal_size` in that
//!   file; O4 `metrics().block_cache_capacity` (the LRU cache size).
//!   NOT outputs: data in the store (fresh empty dir), heap use (not code).
//!   Paths: fresh open of each store (the only open path that sets caps).
//!
//! INPUT PARTITIONS
//!   IP-BS  fresh tempdir, BlockStore  -> O1..O4 (caps > 0)
//!   IP-SD  fresh tempdir, StateDb     -> O1..O4 (caps > 0)
//!   IP-SUM both stores of one node    -> O2 + O4 summed <= budget

use std::path::Path;

use tempfile::TempDir;

const MIB: u64 = 1024 * 1024;

/// Per-node RocksDB budget the INC-I-150 host sizing used: 6 nodes x ~192 MB
/// RocksDB + heap (~240 MB/node) on a 3.8 GiB host. Raising any cap past this
/// total needs a new host-memory review first (INV-DEPLOY-003).
const PER_NODE_ROCKSDB_BUDGET: u64 = 192 * MIB;

/// Read `key=value` from the newest OPTIONS-* file RocksDB wrote in `dir`.
fn options_value(dir: &Path, key: &str) -> u64 {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .expect("read store dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("OPTIONS-"))
        })
        .collect();
    files.sort();
    let newest = files
        .last()
        .expect("RocksDB writes an OPTIONS file on open");
    let text = std::fs::read_to_string(newest).expect("read OPTIONS file");
    let needle = format!("{key}=");
    text.lines()
        .map(str::trim)
        .find_map(|l| l.strip_prefix(needle.as_str()))
        .unwrap_or_else(|| panic!("{key} missing from {}", newest.display()))
        .parse()
        .unwrap_or_else(|e| panic!("{key} is not a number: {e}"))
}

/// Memtable cap, WAL cap and block cache capacity of one store.
struct Caps {
    memtable: u64,
    wal: u64,
    cache: u64,
}

fn block_store_caps() -> Caps {
    let dir = TempDir::new().unwrap();
    let store = storage::BlockStore::open(dir.path()).expect("BlockStore opens");
    let cache = store.metrics().block_cache_capacity;
    Caps {
        memtable: options_value(dir.path(), "db_write_buffer_size"),
        wal: options_value(dir.path(), "max_total_wal_size"),
        cache,
    }
}

fn state_db_caps() -> Caps {
    let dir = TempDir::new().unwrap();
    let sdb = storage::StateDb::open(dir.path()).expect("StateDb opens");
    let cache = sdb.metrics().block_cache_capacity;
    Caps {
        memtable: options_value(dir.path(), "db_write_buffer_size"),
        wal: options_value(dir.path(), "max_total_wal_size"),
        cache,
    }
}

fn assert_capped(name: &str, c: &Caps) {
    assert!(
        c.memtable > 0,
        "INV-DEPLOY-003: {name} db_write_buffer_size is 0 — the memtable is uncapped"
    );
    assert!(
        c.wal > 0,
        "INV-DEPLOY-003: {name} max_total_wal_size is 0 — the WAL is uncapped"
    );
    assert!(
        c.cache > 0,
        "INV-DEPLOY-003: {name} block cache capacity is 0 — no explicit LRU cap"
    );
}

/// IP-BS: the block store opens with every cap set.
#[test]
fn inv_deploy_003_block_store_opens_with_memory_caps() {
    assert_capped("block_store", &block_store_caps());
}

/// IP-SD: the state DB opens with every cap set.
#[test]
fn inv_deploy_003_state_db_opens_with_memory_caps() {
    assert_capped("state_db", &state_db_caps());
}

/// IP-SUM: one node's RocksDB memory caps fit the per-node budget the hosts
/// were sized for.
#[test]
fn inv_deploy_003_per_node_rocksdb_caps_fit_the_budget() {
    let bs = block_store_caps();
    let sd = state_db_caps();
    let total = bs.memtable + bs.cache + sd.memtable + sd.cache;
    assert!(
        total <= PER_NODE_ROCKSDB_BUDGET,
        "INV-DEPLOY-003: per-node RocksDB caps are {} MiB (block_store memtable {} + \
         cache {}, state_db memtable {} + cache {}), above the {} MiB budget. Re-run the \
         host check (N nodes x budget + heap <= 60% MemTotal) before raising a cap",
        total / MIB,
        bs.memtable / MIB,
        bs.cache / MIB,
        sd.memtable / MIB,
        sd.cache / MIB,
        PER_NODE_ROCKSDB_BUDGET / MIB
    );
}
