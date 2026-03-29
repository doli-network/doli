# Adversarial Cargo Test Findings Report

**Date:** 2026-03-27
**Scope:** Network crate (`crates/network/`), sync layer, reorg handler, rate limiter
**Incident context:** INC-I-014 (RAM explosion), INC-I-010 (fork-stuck node)
**Tests written:** 36 (24 in `sync/adversarial_tests.rs`, 12 in `sync/manager/tests.rs`)
**All tests pass:** Yes (after documenting bugs as expected behavior)

## Bug Inventory

### BUG-1: TokenBucket::refill() integer overflow (P2)
- **File:** `crates/network/src/rate_limit.rs:41`
- **Code:** `self.tokens = (self.tokens + new_tokens).min(self.capacity);`
- **Issue:** When `self.tokens` is near `u64::MAX` and `new_tokens > 0`, the addition panics in debug builds (overflow) and wraps in release builds (giving a tiny token count).
- **Fix:** Use `self.tokens.saturating_add(new_tokens).min(self.capacity)`
- **Impact:** Debug build panic on any peer with a high-capacity bucket after time passes. In release, the wrap-around permanently rate-limits the peer.
- **Test:** `test_token_bucket_extreme_capacity_overflow_bug`

### BUG-2: plan_reorg cannot find genesis as common ancestor (P1)
- **File:** `crates/network/src/sync/reorg/mod.rs` (plan_reorg method)
- **Code:** `if parent.is_zero() { break; }` in the chain walk loop
- **Issue:** The chain walk breaks at genesis (Hash::ZERO) BEFORE adding it to the `current_ancestors` set. When two chains share only genesis as the common ancestor, plan_reorg returns None.
- **Fix:** Add genesis to `current_ancestors` before breaking: `current_ancestors.insert(hash); if parent.is_zero() { break; }`
- **Impact:** Deep forks (where the only common ancestor is genesis) cannot be resolved via plan_reorg. Falls through to "Could not plan reorg" warning. Fork recovery relies on the slower check_reorg_weighted path or manual intervention.
- **Test:** `test_plan_reorg_deep_fork_genesis_boundary_bug`

### BUG-3: check_reorg_weighted blind to genesis forks (P1)
- **File:** `crates/network/src/sync/reorg/mod.rs` (check_reorg_weighted method)
- **Issue:** `record_block_with_weight()` only inserts the BLOCK hash into `recent_blocks`, not its parent. Genesis (Hash::ZERO) is never explicitly recorded. When a fork block's `prev_hash` is genesis, the check `!self.recent_blocks.contains(&prev_hash)` returns true, causing an early None return.
- **Fix:** Seed `recent_blocks` with genesis hash at ReorgHandler initialization, or in the node's initialization code.
- **Impact:** Single-block forks from genesis cannot be detected or tie-broken via the fast in-memory path. Nodes rely on slow fork recovery. Masked by the bootstrap grace period where only one producer is active.
- **Test:** `test_deterministic_tiebreak_genesis_fork_bug`

### BUG-4: complete_resync() skips PostRecoveryGrace (P2)
- **File:** `crates/network/src/sync/manager/production_gate.rs:215`
- **Code:** `self.recovery_phase = RecoveryPhase::Normal;`
- **Issue:** The `RecoveryPhase` enum defines the lifecycle as Normal -> ResyncInProgress -> PostRecoveryGrace -> Normal. But `complete_resync()` transitions directly to Normal, skipping PostRecoveryGrace entirely. The PostRecoveryGrace variant exists but is never entered via this path.
- **Fix:** Change to `self.recovery_phase = RecoveryPhase::PostRecoveryGrace { started: Instant::now(), blocks_applied: 0 };`
- **Impact:** After a forced resync, the node can immediately start producing blocks before confirming it's on the canonical chain. The AwaitingCanonicalBlock phase (entered via snap sync) partially mitigates this.
- **Test:** `test_adversarial_recovery_phase_lifecycle`

### BUG-5: ForkRecoveryTracker::cancel() silent no-op without active recovery (P3)
- **File:** `crates/network/src/sync/fork_recovery.rs` (cancel method)
- **Issue:** `cancel("exceeded max depth")` only sets the `exceeded_max_depth` flag when `self.active.is_some()`. Calling cancel without an active recovery silently does nothing - the flag is NOT set. Callers may assume cancel always sets the flag.
- **Impact:** Low. The only caller that uses "exceeded max depth" is the depth check inside `handle_block()`, which only fires when `active.is_some()`. But the API contract is misleading.
- **Test:** `test_fork_recovery_exceeded_max_depth_requires_active`

## Attack Surface Map

### P0 (Can crash/hang the node)
- TokenBucket overflow (BUG-1) - panic in debug builds
- Finality bypass via check_reorg_weighted - VERIFIED PROTECTED (test passes)
- Finality bypass via plan_reorg - VERIFIED PROTECTED (test passes)
- Circular parent chain in plan_reorg - VERIFIED SAFE (bounded by MAX_REORG_DEPTH)

### P1 (Can cause network partition or fork)
- Genesis fork detection blind spot (BUG-2, BUG-3) - forks from genesis not detected
- Weight saturation - VERIFIED SAFE (no panic, deterministic behavior)
- ReorgHandler eviction loses ancestors - VERIFIED KNOWN LIMITATION (documented)

### P2 (Memory/resource leaks)
- ReorgHandler memory - VERIFIED BOUNDED at max_tracked=10,000
- EquivocationDetector memory - VERIFIED BOUNDED at MAX_TRACKED_ENTRIES=10,000
- Rate limiter memory - VERIFIED BOUNDED at MAX_TRACKED_PEERS=1,000
- PostRecoveryGrace skipped (BUG-4) - premature production after resync

### P3 (Correctness bugs)
- ForkState.recommend_action - VERIFIED CORRECT for all edge cases
- SyncPipelineData.is_snap_syncing - VERIFIED CORRECT for all variants
- Gossip mesh parameters - VERIFIED INVARIANTS HOLD for all network sizes
- AdaptiveGossip backoff - VERIFIED NO OVERFLOW after 1000 rounds

## Hardening Tests (no bugs found, kept as regression tests)
1. `test_reorg_finality_prevents_deep_reorg` - finality guard works
2. `test_plan_reorg_finality_guard` - plan_reorg finality guard works
3. `test_reorg_handler_memory_bounded` - LRU eviction keeps memory bounded
4. `test_reorg_weight_delta_no_overflow` - lighter forks always rejected
5. `test_fork_recovery_no_response_memory_stable` - no memory leak on pending
6. `test_fork_recovery_wrong_peer_ignored` - wrong peer responses ignored
7. `test_fork_recovery_no_concurrent` - concurrent recovery prevented
8. `test_equivocation_detector_memory_bounded` - bounded at 10K entries
9. `test_equivocation_detected_correctly` - double-signing detected
10. `test_dynamic_mesh_invariants` - mesh_n_low <= mesh_n <= mesh_n_high always
11. `test_rate_limiter_lru_eviction` - LRU caps at 1000 peers
12. `test_fork_block_does_not_update_current_weight` - fork blocks don't corrupt weight
13. `test_reorg_handler_clear_resets_all` - clear fully resets state

## Recommended Fixes (Priority Order)

1. **BUG-1** (Easy, P2): Change `self.tokens + new_tokens` to `self.tokens.saturating_add(new_tokens)` in `rate_limit.rs:41`. One-line fix.

2. **BUG-2** (Easy, P1): Add `current_ancestors.insert(hash);` before the `is_zero()` break in `plan_reorg()`. Two-line fix.

3. **BUG-3** (Easy, P1): In Node initialization (or SyncManager::new), call `reorg_handler.record_block(genesis_hash, Hash::ZERO)` to seed genesis. One call.

4. **BUG-4** (Medium, P2): Change `complete_resync()` to transition to PostRecoveryGrace instead of Normal. Requires testing the grace period timeout logic.

5. **BUG-5** (Low, P3): Document or add a debug_assert in cancel() that active.is_some().

## Advanced Tooling Recommendations

- **cargo-fuzz**: Fuzz `plan_reorg` and `check_reorg_weighted` with random chain topologies
- **miri**: Run equivocation detector tests under miri to check for UB in hash comparisons
- **loom**: Test the concurrent access patterns in swarm_loop.rs (rate_limiter, eviction_cooldown, bootstrap_peers accessed in same tokio::select!)
