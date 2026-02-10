# ISSUE-5: Restart Sync Height Double-Counting — Test Report

**Date:** 2026-02-10
**Commit:** fd5e222 (fix(node): fix height double-counting on restart sync (ISSUE-5))
**Network:** Devnet (5 producers, 1 bond each)

---

## Bug Summary

Restarting killed nodes caused height double-counting: a node at height 120 would restart, re-download ALL 163 blocks from genesis, and reach height 283 (120 + 163). This corrupted height propagated to peers via sync protocol.

**Root Cause (two bugs):**
1. `SyncManager::new()` always started at genesis — never initialized with stored ChainState
2. `apply_block()` had no duplicate check — blindly incremented height for already-stored blocks

## Fixes Applied

| Fix | Description | File | Lines |
|-----|-------------|------|-------|
| **Fix A** (Root cause) | Initialize SyncManager with stored ChainState on restart | `bins/node/src/node.rs` | 324-337 |
| **Fix B** (Defense-in-depth) | Skip `apply_block()` if block already in store | `bins/node/src/node.rs` | 2045-2053 |

## Test Results

### Pre-Commit Gate
| Check | Result |
|-------|--------|
| `cargo build` | PASS |
| `cargo clippy -- -D warnings` | PASS |
| `cargo fmt --check` | PASS |
| `cargo test` (649 tests) | PASS (0 failures) |

### Live Devnet Test (5-node restart scenario)

**Setup:** 5 producer nodes, all using fixed binary, chain at height 55.

**Procedure:**
1. Killed nodes 2, 3, 4 at height 55
2. Waited 40s for surviving nodes to advance
3. Restarted killed nodes with fixed binary
4. Verified convergence

**Results per node:**

| Node | Pre-kill Height | Post-restart Height | Fix A Log | Fix B Duplicates | Status |
|------|-----------------|--------------------:|-----------|------------------|--------|
| 0 (alive) | 55 | 63 | N/A | N/A | PASS - continued producing |
| 1 (alive) | 55 | 63 | N/A | N/A | PASS - continued producing |
| 2 (killed) | 55 | 58 | "Sync manager initialized at height 50" | 5 duplicates caught | **PASS** |
| 3 (killed) | 55 | CRASH | N/A | N/A | FAIL (RocksDB LOCK issue, separate bug) |
| 4 (killed) | 55 | 58 | (verified in earlier run) | 0 | **PASS** |

### Earlier run (node 3 manual restart with LOCK cleanup):

| Node | Pre-kill | Post-restart | Fix A | Duplicates | Notes |
|------|----------|-------------|-------|------------|-------|
| 3 | 40 | 46 (synced) | "Sync manager initialized at height 40" | 0 | **PASS** after LOCK file removal |

### Key Verifications

1. **No height double-counting:** Restarted nodes at height 58, reference at 63. If the old bug were present, heights would be 55+63=118.
2. **Fix A works:** Log confirms "Sync manager initialized at height 50" — node starts sync from stored tip, not genesis.
3. **Fix B works:** 5 duplicate blocks caught and skipped on node 2 (some blocks received via gossip during restart overlap).
4. **Hash consensus:** All running nodes share the same hash at their respective heights.
5. **No regression:** Fresh start from genesis still works correctly.

## Discovered Pre-Existing Issue

**RocksDB LOCK file prevents restart (macOS SIGTRAP crash):**

When a node is killed (SIGTERM/SIGKILL), RocksDB leaves a stale `LOCK` file in `data/node*/blocks/`. On restart, the new process crashes with `Trace/BPT trap: 5` (SIGTRAP) when RocksDB tries to open the database.

**Workaround:** Remove `blocks/LOCK` and `signed_slots.db/LOCK` before restart.

**Fix needed:** The node should handle stale LOCK files gracefully (most RocksDB wrappers have a `destroy_on_lock` or similar option). This is a separate issue from ISSUE-5.

## Test Script

`scripts/test_restart_sync_issue5.sh` — automated test that:
- Verifies devnet health
- Waits for minimum chain height
- Kills 3 nodes, waits for fallback production
- Restarts with LOCK file cleanup
- Verifies convergence and no double-counting
- Checks log output for Fix A and Fix B evidence
