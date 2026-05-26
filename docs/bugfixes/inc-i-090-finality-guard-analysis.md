# INC-I-090: Finality Guard Off-by-One — Triage & Requirements

> **Incident**: INC-I-090
> **Severity**: High — blocks legitimate fork recovery on mainnet
> **Date**: 2026-05-25 (mainnet, h=284677, N3 affected)
> **Status**: Diagnosed (conf 0.95), pending fix
> **Triage Verdict**: FAST

---

## 1. Bug Summary

N3 produced a fork block at height 284677. The canonical chain had a different block at 284677. To recover, N3's sync manager attempted a ShallowRollback to height 284676 (the finality height). The FINALITY_GUARD check in the recovery module refused the rollback because it uses `target_height <= finality_height` — treating rollback TO finality as illegal.

This is a fencepost error. Rolling back TO the finalized height preserves the finalized block and only removes the non-finalized block above it. Only rolling BELOW finality violates the finality invariant. The correct check is `target_height < finality_height`.

The 1-character fix (`<=` to `<`) restores the legal rollback path.

---

## 2. Architecture Context

### Module Boundaries

- **`crates/network/src/sync/manager/recovery.rs`**: Recovery state machine for the sync manager. Handles `SyncAction` variants including `ShallowRollback`, `DeepReorg`, and fork signal emission.
- **`crates/network/src/sync/manager.rs`**: Top-level sync state machine. Coordinates initial sync, catch-up sync, and fork recovery. Calls into `recovery.rs` when it detects the local chain has diverged from the network.
- **`bins/node/src/node/block_handling.rs`**: Consumes the results of sync recovery — when a ShallowRollback succeeds, the node's block handling module applies the rollback.
- **`bins/node/src/node/rollback.rs`**: Performs the actual state rollback (undo-based or rebuild).

### Data Flow

```
Network detects fork → sync manager → recovery.rs (decides ShallowRollback)
  → FINALITY_GUARD check (target_height vs finality_height)
    → IF PASS: proceed with rollback → node applies state changes
    → IF FAIL: rollback refused → node stuck on fork tip (BUG)
```

### Architectural Constraints & Invariants

1. **Finality invariant**: Once a block is finalized, no rollback should remove it.
2. **Contract of ShallowRollback**: Roll back to a target height, preserving the block AT the target height. Only removes blocks strictly ABOVE the target.
3. **Recovery must not deadlock**: If a legal recovery path exists, the sync manager must take it.

### Blast Radius

- **Direct impact**: `recovery.rs` — 1 line, 1 character change
- **Consumers**: The sync manager respects the guard's decision. Changing `<=` to `<` makes the guard permit `target == finality` (legal) while still refusing `target < finality` (illegal). All callers unaffected.
- **Indirect impact**: None.

### Brittleness Check

```
━━━ BRITTLENESS CHECK ━━━
Signals detected: 0/5
  1. Cross-module blast radius: NO
  2. Invariant gaps: NO
  3. Data flow reversal: NO
  4. Shared mutable state: NO
  5. Contract absence: NO
Verdict: LOCALIZED
━━━━━━━━━━━━━━━━━━━━━━━━━
```

---

## 3. Root Cause

**File**: `crates/network/src/sync/manager/recovery.rs`
**Line**: ~312 (diagnosis reports cite :312; one cited :311 — developer must confirm exact line by searching for `FINALITY_GUARD` in the ShallowRollback match arm)

The FINALITY_GUARD check uses `target_height <= finality_height` where it should use `target_height < finality_height`.

- `<=` refuses both `target < finality` (correct: illegal) AND `target == finality` (incorrect: legal)
- `<` refuses only `target < finality` (correct: illegal) and permits `target == finality` (correct: legal)

---

## 4. Impact Analysis

### Existing Code Affected
| File/Module | How Affected | Risk |
|-------------|-------------|------|
| `recovery.rs` ~line 312 | 1-char change: `<=` to `<` | Low — strictly less restrictive by one boundary case |

### Regression Risk
| Area | Risk |
|------|------|
| Rollback below finality | None — `<` still catches all `target < finality` cases |
| Test suite | None — no existing test asserts `<=` behavior at boundary |
| Other finality comparisons | Low — developer should verify consistency (AC-3) |

### Deploy Safety (Two-Question Check)
1. Does this change consensus RULES? **NO** — sync recovery logic is local to each node's fork-choice.
2. Does this change block CONTENT? **NO** — no change to what goes into blocks.

**Verdict**: Safe for rolling deploy. No activation height needed.

---

## 5. Requirements

| ID | Requirement | Priority | Acceptance Criteria |
|----|------------|----------|-------------------|
| REQ-I-090-001 | Fix FINALITY_GUARD fencepost: change `<=` to `<` in ShallowRollback handler | Must | AC-1, AC-2, AC-3, AC-4 |
| REQ-I-090-002 | Add unit test for boundary case `target == finality` | Must | AC-1 |
| REQ-I-090-003 | Verify other finality comparisons in recovery module are consistent | Should | AC-3 |
| REQ-I-090-004 | Wire `take_stuck_fork_signal()` consumer (dead code) | Won't | N/A (deferred) |
| REQ-I-090-005 | Tighten `recently_synced()` heuristic | Won't | N/A (deferred) |
| REQ-I-090-006 | Fork observability improvements | Won't | N/A (deferred — separate session) |
| REQ-I-090-007 | Investigate ai1 16-second gossip delay | Won't | N/A (separate connectivity investigation) |

---

## 6. Acceptance Criteria

### AC-1: Reproduction test (TEST-I-090-001)
- A new unit test in the recovery module constructs the case: `target_height == finality_height` with a `ShallowRollback` action
- The test asserts the rollback PROCEEDS (is not blocked by the finality guard)
- The test FAILS against current code (with `<=`) — guard incorrectly refuses
- The test PASSES after the 1-char fix (with `<`) — guard correctly permits
- Test name clearly identifies the boundary case (e.g., `test_finality_guard_permits_rollback_to_finality`)

### AC-2: No regression (TEST-I-090-002)
- `cargo test -p doli-network` passes
- `cargo test` workspace-wide passes (or scoped subset if feasible)
- No existing test changes behavior

### AC-3: Consistency check (TEST-I-090-003)
- Developer verifies all other finality comparisons in `recovery.rs` use consistent semantics
- If any other comparison uses `<=` where `<` is correct, flag (but NOT fix in this milestone)
- A test covering `target_height < finality_height` (strictly below) confirms the guard STILL REFUSES

### AC-4: Minimal change
- The ONLY code change is `<=` to `<` on the FINALITY_GUARD line
- No dead-code cleanup, no observability additions, no refactoring
- Commit message references INC-I-090

---

## 7. Triage Verdict

```
━━━ TRIAGE VERDICT ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Verdict:      FAST
Confidence:   conf(0.95, converged)
Reasoning:
  - Root cause identified by 3/4 independent investigators
  - Bug localized to 1 line, 1 character in 1 file
  - Logic is deterministic and well-understood (fencepost)
  - No architectural implications (LOCALIZED, 0/5 brittleness)
  - No consensus impact — safe for rolling deploy
  - Fix is strictly less restrictive (permits one additional legal case)
Causal chain:
  N3 produces fork block at h=284677
  → canonical chain has different block at 284677
  → sync manager plans ShallowRollback to h=284676 (= finality)
  → FINALITY_GUARD: 284676 <= 284676 → true → rollback REFUSED
  → N3 stuck on fork tip, cannot recover
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

---

## 8. Milestone Definition

### M1: Fix FINALITY_GUARD fencepost (INC-I-090)

**Scope**: 1 character change + 1 new unit test

**Requirements**: REQ-I-090-001 (Must), REQ-I-090-002 (Must), REQ-I-090-003 (Should)

**Acceptance Criteria**:
| ID | Criterion | Verified |
|----|-----------|----------|
| AC-1 | New unit test `target == finality` FAILS before fix, PASSES after | [ ] |
| AC-2 | Test suite passes (no regression) | [ ] |
| AC-3 | Existing FINALITY_GUARD callers still refuse `target < finality` | [ ] |
| AC-4 | Only change is `<=` to `<` — no cleanup, no refactor | [ ] |

**Deploy notes**: Safe for rolling deploy. No activation height. No consensus rule change. No block content change.

---

## 9. Traceability Matrix

| Requirement ID | Priority | Test IDs | Implementation Module |
|---------------|----------|----------|---------------------|
| REQ-I-090-001 | Must | TEST-I-090-001, TEST-I-090-003 | crates/network/src/sync/manager/recovery.rs |
| REQ-I-090-002 | Must | TEST-I-090-001 | recovery.rs test module |
| REQ-I-090-003 | Should | TEST-I-090-003 | recovery.rs all finality checks |

---

## 10. Assumptions

| # | Assumption | Confirmed |
|---|-----------|-----------|
| 1 | ShallowRollback to height H preserves the block at H | Yes (CLAUDE.md + diagnosis convergence) |
| 2 | `finality_height` is correctly computed | Yes (N3's finality matched canonical) |
| 3 | No other code path overrides the FINALITY_GUARD refusal | Yes (diagnosis confirmed stuck state) |
| 4 | Exact line is ~312 in recovery.rs | NO — developer must confirm via grep for `FINALITY_GUARD` |

---

## 11. Out of Scope (Won't)

| Item | Reason |
|------|--------|
| Wire `take_stuck_fork_signal()` consumer | Dead code — separate PR |
| Tighten `recently_synced()` heuristic | Improvement — not the bug |
| Fork observability improvements | Separate session |
| ai1 16-second gossip delay | Separate connectivity issue |
| Documentation of sync recovery module | Post-fix improvement |
