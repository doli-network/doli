# Code Review: INC-I-034 M-RC11 — FORK_GUARD reorg backfill invariant fix

**Date**: 2026-04-16
**Reviewer**: reviewer
**Milestone**: M-RC11 (third of INC-I-034 trigger-removal trio)
**Requirement**: REQ-REDESIGN-011 (chain_state ↔ block_store completeness invariant)
**Verdict**: **APPROVE**

## Root cause fix verification — PASSED

Cascade chain step the fix targets:
```
(1) execute_reorg called with target_height where block_store has gap
(2) common_ancestor_block = block_store.get_block_by_height(target_height) → Ok(None)
(3) BUG: unwrap_or(genesis_hash) silently substitutes
(4) chain_state.write(...).best_hash = corrupt_anchor
(5) for new_block in new_blocks { apply_block(new_block) }   ← propagates corruption
(6) verifyChainIntegrity reports gaps; santiago/ivan/seed3 cascade
```

| Step | Pre-fix | Post-fix gate |
|------|---------|---------------|
| (1)→(2) | `get_block_by_height` returns `Ok(None)` silently | `ensure_blocks_present(1, target_height)` returns `Err(NotFound)` first; `?` propagates BEFORE step (4) reachable |
| (3) | `unwrap_or(genesis_hash)` substitutes | Explicit `match` at lines 437–457; `None` branch is `bail!` with `[FORK_GUARD_BACKFILL_REQUIRED]` (defense-in-depth against TOCTOU) |
| (4) | `chain_state.best_hash` corrupted | First `chain_state.write` is at line 519 — gated by guard at line 412–428 |
| (5) | Downstream `apply_block` ratchets corruption | Unreachable when (4) is gated |

Both layers fire on the same input — by design as belt-and-braces.

## Pre-flight placement — PASSED

`grep "chain_state.write" block_handling.rs`:
- line 519 (undo path)
- line 541 (legacy fallback path)

Pre-flight at line 412 with `?` propagation at line 428 sits BEFORE both. Both rollback paths are gated.

## Defense-in-depth `match` — PASSED

The `match` at lines 437-457 does NOT silently substitute `genesis_hash`. The `None` branch logs `error!` with `[FORK_GUARD_BACKFILL_REQUIRED]` and `bail!`s. The genesis case (`target_height == 0`) is handled by an outer `if` at line 434 BEFORE the `match` — legitimate genesis path never reaches the bail.

## Genesis edge case — PASSED (3 reasons)

- `target_height == 0` → outer `if` returns `None` without invoking `get_block_by_height`
- `ensure_blocks_present(1, 0)` is `low > high` → returns `Ok(())`
- `unwrap_or(genesis_hash)` at lines 463-466 is reached only when `common_ancestor_block` is `None`, which post-gate happens ONLY when `target_height == 0` — safe by construction

## `ensure_blocks_present` helper — PASSED

| Property | Verification |
|---|---|
| Empty/inverted range → Ok(()) | Lines 194-196; tested |
| `low == 0` tolerated (genesis not in height_index) | Line 198 (`low.max(1)`); tested |
| Returns FIRST missing height | Line 199; test asserts `"height 3"` not `"height 5"` |
| O(1) per-height lookup | Line 200 calls `get_hash_by_height` only |
| Project-standard error type | Returns `StorageError::NotFound`, not `anyhow::Error` |
| Greppable diagnostic tag | `[FORK_GUARD_BACKFILL]` in error message |

All 4 unit tests pass.

## Performance — ACCEPTABLE

- `get_hash_by_height` is one RocksDB `get_cf` on `CF_HEIGHT_INDEX` (8-byte key, 32-byte value, bloom-filter-assisted)
- Pathological 10k-block reorg: ~10 ms total (well below slot budget)
- Production reorg depths typically <100 blocks

## Concurrency — TOCTOU window covered

The pre-flight is read-only on block_store. TOCTOU window between pre-flight and anchor read is closed by the defense-in-depth `match` — concurrent prune triggers the same `[FORK_GUARD_BACKFILL_REQUIRED]` error.

## Sibling silent-substitution scan

Only one `unwrap_or(genesis_hash)` remains in `block_handling.rs:466` — the legitimate `target_height == 0` genesis branch.

Pre-existing observation: `block_handling.rs:547-549` legacy fallback rebuild loop still uses `.ok().flatten()` silent-skip pattern. With M-RC11 pre-flight, this site is now defensively unreachable on any path with a gap. Pattern remains as latent fragility — P3 follow-up, not a M-RC11 blocker.

## fork_recovery suite (11/11 PASS)

QA confirmed. The fix is correctly tightened, NOT over-tightened. This is the load-bearing regression check for any reorg-touching fix.

## Acceptance evidence

| Gate | Result |
|---|---|
| `m_rc11_fork_guard_backfill_regression` | 3/3 PASS (test B FAIL→PASS deterministic) |
| `fork_recovery` (canonical) | 11/11 PASS |
| `m_rc9_silent_vec_regression` | 3/3 PASS |
| `m_rc10_apply_after_reject_regression` | 3/3 + pre-existing fixture intermittency (test B) |
| `cargo test -p doli-node --lib` | 10/10 PASS |
| `cargo test -p storage --lib` | 170/170 PASS (incl 4 new) |
| `cargo test -p doli-node --test test_network` | 13/13 PASS |
| `cargo test -p doli-node --test epoch_reward_explicit_inputs` | 7/7 PASS |
| `cargo build --release / clippy / fmt` | clean (test-file drift pre-existing) |

## P3 follow-ups (non-blocking)

1. **Legacy rebuild loop** (`block_handling.rs:547-549`): `.ok().flatten()` silent-skip pattern remains. Now defensively unreachable post-M-RC11 but worth cleanup.
2. **Spec wording drift**: `specs/scheduler-state-architecture.md:185` says `fork_recovery.rs::execute_reorg` — actually lives in `block_handling.rs`. Pre-existing.
3. **Pre-flight depth**: For >10k-block reorgs, `multi_get_cf` batching could collapse the scan. Not currently needed.
4. **Test file fmt drift**: M-RC10 + M-RC11 test files. Test-writer scope.

## Verdict

**APPROVE for merge.** Fix is at root cause, both layers belt-and-braces, genesis preserved, helper exemplary, fork_recovery 11/11 unbroken, FAIL→PASS deterministic. Three P3 follow-ups are post-merge scope.

Trio complete: M-RC9 (trigger removal) + M-RC10 (apply-after-reject) + M-RC11 (FORK_GUARD backfill) — the live mainnet cascade is now structurally impossible.
