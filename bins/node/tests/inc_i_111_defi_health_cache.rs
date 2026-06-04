//! INC-I-111: defi_health_inputs() CPU regression — 30-second cache test
//!
//! `run_periodic_tasks()` calls `utxo_set.read().await.defi_health_inputs()`
//! every 1-second tick. Each call does 2 full RocksDB cf_utxo scans
//! (iter_all for bonds + get_all_pools for AMM). On mainnet with 375K+ blocks
//! this consumes 25-35% of process CPU — a 2x baseline regression shipped in
//! v6.23.1 (Phase 5 BlobDB).
//!
//! Fix: cache the `(total_active_bonds, max_pool)` tuple for 30 seconds.
//! Only refresh when stale. Pattern: mirror `UtxoSizeMonitor` (60s TTL,
//! AtomicU64 computation counter, `Mutex<Option<(value, Instant)>>` cache).
//!
//! This test calls `run_periodic_tasks()` 10 times in rapid succession (no
//! wall-clock sleep) and asserts that `defi_health_inputs()` was invoked at
//! most once (the first call refreshes; the remaining 9 return cached).
//!
//! Requirement: INC-I-111 (Must)
//! Acceptance: defi_health_inputs scans run at most once per 30-second window
//!
//! OUTPUT CONTRACT: run_periodic_tasks() defi_health refresh
//! Outputs:
//!   O1: defi_health_refresh_count (AtomicU64 on Node) — increments only on
//!       cache miss (real scan executed). Exposed via
//!       `Node::defi_health_refresh_count()` (`#[cfg(test)]` getter).
//!   O2: doli_defi_total_bonds / doli_defi_max_pool_tvl Prometheus gauges —
//!       set on every call (cached value pushed when cache is fresh).
//! PATHS:
//!   P1: fresh call (no prior refresh) — counter increments, scan runs
//!   P2: cached call (last refresh < 30s ago) — counter unchanged, no scan
//!   P3: stale call (last refresh >= 30s ago) — counter increments, scan runs
//! INPUT PARTITIONS:
//!   P1a: empty UTXO set (no bonds, no pools) — scan returns (0, None),
//!        counter goes from 0 to 1. Only partition for P1: regardless of UTXO
//!        content the cache-miss logic is the same (TTL-based, content-agnostic).
//!   P2a: same empty set, called within 30s of P1a — cache hit, counter stays 1.
//!        Only partition for P2: cache-hit logic is TTL-based, content-agnostic.
//!   P3a: empty set, called after 30s+ since last refresh — cache stale,
//!        counter goes from 1 to 2. Not exercised here (needs time control).
//!        One partition sufficient: staleness is purely time-based.
//! MATRIX: 2 outputs x 3 partitions = 6 cells
//!   P1a: O1(counter == 1)  O2(gauges set)
//!   P2a: O1(counter == 1)  O2(gauges unchanged — cached value pushed)
//!   P3a: O1(counter == 2)  O2(gauges set to new value) [not exercised]
//!
//! NOTE: P3 (stale refresh) is not exercised in this test because it would
//! require either `tokio::time::pause()/advance()` or real 30s wall-clock
//! sleep. The primary assertion (P1+P2: 10 calls => 1 refresh) is sufficient
//! to prove the cache works. P3 is a SHOULD-level concern that can be tested
//! separately with time mocking if needed.

use crypto::KeyPair;
use doli_node::node::Node;
use tempfile::TempDir;

/// Create a test Node with N producers, real RocksDB, real everything.
/// Mirrors the helper in fork_recovery.rs.
async fn make_node(n_producers: usize) -> (Node, Vec<KeyPair>, TempDir) {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n_producers).map(|_| KeyPair::generate()).collect();
    let node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");
    (node, producers, temp)
}

/// Requirement: INC-I-111 (Must)
/// Acceptance: calling run_periodic_tasks() 10 times in rapid succession
/// results in at most 1 actual defi_health_inputs() scan (the first call).
/// The remaining 9 calls return the cached value without scanning.
///
/// This test MUST FAIL before the cache is implemented (counter will be 10).
/// It MUST PASS after the Developer adds the 30-second cache with the
/// AtomicU64 refresh counter.
#[tokio::test]
async fn defi_health_inputs_runs_at_most_once_within_30s_window() {
    let (mut node, _producers, _temp) = make_node(3).await;

    // P1a: Baseline — counter should start at 0
    let before = node.defi_health_refresh_count();
    assert_eq!(before, 0, "counter should start at 0");

    // Call run_periodic_tasks() 10 times in rapid succession.
    // With the cache, only the first call should trigger a real scan.
    // Without the cache (current code), all 10 calls will scan.
    for _ in 0..10 {
        node.run_periodic_tasks()
            .await
            .expect("run_periodic_tasks should not fail");
    }

    let after = node.defi_health_refresh_count();

    // P1a x O1 + P2a x O1: at most 1 refresh in a tight loop (all calls
    // within <1ms, well inside the 30s TTL window).
    //
    // Without the cache, `after` will be 10 (one scan per call) => FAIL.
    // With the cache, `after` will be 1 (first call refreshes, 9 cached) => PASS.
    assert!(
        after <= 1,
        "defi_health_inputs should have been called at most 1 time within 30s, \
         but was called {} times. The 30-second cache is missing or broken.",
        after
    );
}
