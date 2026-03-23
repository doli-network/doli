# INC-I-005 Sync Cascade — Redesign Problem Scoping

> **Analyst**: Problem scoping for architecture redesign
> **Date**: 2026-03-23
> **Incident**: INC-I-005 (snap sync cascade, 9 fixes, 10 sessions)
> **Scope**: Sync/recovery subsystem (~13K lines, 19 files)

---

## Q1: Architecture Problem or Bug Cluster?

**VERDICT: Architecture problem.** The evidence is conclusive across 5 independent lines of reasoning.

### Evidence Line 1: Nine targeted fixes failed because the architecture reroutes around them

Each of the 9 fixes addressed a real bug confirmed by trace logs or code tracing. Yet the cascade persisted after every fix. This is not because the fixes were wrong — it is because the architecture has multiple independent paths into the same destructive spiral. A bug cluster would show diminishing returns with each fix; here, we see zero reduction in cascade rate because the fundamental feedback loop is structural.

### Evidence Line 2: The state machine has 3 orthogonal axes with no valid-combination enforcement

The sync subsystem has three concurrent state axes:
- **SyncState** (8 variants): Idle, DownloadingHeaders, DownloadingBodies, Processing, Synchronized, SnapCollectingRoots, SnapDownloading, SnapReady
- **RecoveryPhase** (6 variants): Normal, StuckForkDetected, PostRollback, ResyncInProgress, PostRecoveryGrace, AwaitingCanonicalBlock
- **ForkState flags** (~8 significant flags): consecutive_empty_headers, consecutive_apply_failures, needs_genesis_resync, fork_mismatch_detected, fork_sync, consecutive_fork_syncs, peak_height, stable_gap_since

This creates a theoretical state space of 8 x 6 x 2^8 = 12,288 combinations. Most are undefined. The `cleanup()` function (543 lines, runs every tick) can mutate ALL three axes in a single call. There is no guard preventing illegal combinations. These undefined combinations create the cascade.

### Evidence Line 3: The `needs_genesis_resync` flag is set from 9 independent code paths

Confirmed locations:
1. `production_gate.rs:1087` (set_needs_genesis_resync API)
2. `sync_engine.rs:274` (post-rollback snap escalation)
3. `sync_engine.rs:415` (genesis fallback on 10+ empty headers)
4. `sync_engine.rs:767` (body download peer error)
5. `block_lifecycle.rs:226` (3+ apply failures, small gap)
6. `block_lifecycle.rs:232` (3+ apply failures, large gap)
7. `cleanup.rs:344` (all peers blacklisted, 20+ empty headers)
8. `cleanup.rs:483` (stuck-sync large gap)
9. `cleanup.rs:524` (height offset detection)

**Nine independent code paths can trigger a destructive state reset with no coordination between them.** This is not a bug cluster — it is an architectural deficiency.

### Evidence Line 4: Recovery mechanisms produce imperfect state that triggers other recovery mechanisms

Post-snap-sync state is structurally imperfect:
- Block store has only a canonical index seed at snap height (no blocks 1 through snap_height-1)
- No undo data for blocks below snap height
- If snap sync hash doesn't match peers, header-first sync fails → triggers fork recovery → triggers genesis resync → triggers snap sync → loop

This is not a bug. This is an architectural property: the recovery mechanisms were designed independently, and each makes assumptions about the state left by the others that are violated.

### Evidence Line 5: The fundamental missing invariant

No monotonic progress guarantee. The system can cycle `Synchronized → height=0 → Synchronized → height=0` indefinitely. Fix C addresses this partially but only guards `reset_local_state()`, not rollback or fork_sync paths.

---

## Q2: Acceptance Criteria for "Better"

### P1: Monotonic Progress (structural invariant)
Once a node has been Synchronized and applied 10+ blocks, it MUST NOT reset below that height via any automatic path.
**Measurable**: No node in a 60-node stress test resets to height 0 after reaching Synchronized.

### P2: Recovery Coordination Contract
All recovery decisions flow through a single coordinator that:
- Prevents concurrent recovery from interfering
- Enforces priority order (rollback > fork_sync > header-first > snap sync > genesis resync)
- Rate-limits destructive recoveries
- Logs every recovery decision with trigger + alternatives
**Measurable**: `needs_genesis_resync` set from exactly 1 code path (coordinator), not 9.

### P3: No Cascade Loop Possible
The state machine must not have cycles where recovery_A triggers recovery_B triggers recovery_A.
**Measurable**: No node enters more than 2 consecutive snap sync cycles.

### P4: Recovery Completes Within Bounded Time
Every RecoveryPhase variant carries `started: Instant` and a maximum duration.
**Measurable**: No RecoveryPhase persists >300s without completing or escalating.

### P5: Valid State Combination Enforcement
Prevent illegal SyncState x RecoveryPhase combinations at the type or runtime level.
**Measurable**: debug_assert on all state transitions, zero assertion failures in 60-node test.

---

## Q3: Minimal Scope

### Files that MUST change (7 files, ~5,600 lines)

| File | Lines | What changes | Risk |
|------|-------|-------------|------|
| `sync/manager/mod.rs` | 1177 | RecoveryCoordinator, valid state combinations, monotonic floor | **High** |
| `sync/manager/cleanup.rs` | 543 | Remove 5 of 9 `needs_genesis_resync` sites, route through coordinator | **High** |
| `sync/manager/block_lifecycle.rs` | 698 | Remove 2 `needs_genesis_resync` sites, strengthen floor | **Medium** |
| `sync/manager/sync_engine.rs` | 1036 | Remove 3 `needs_genesis_resync` sites, route through coordinator | **Medium** |
| `sync/manager/production_gate.rs` | 1138 | Deploy Fix D, remove `set_needs_genesis_resync()` API | **Low** |
| `node/periodic.rs` | 547 | Replace `force_recover_from_peers()` calls with coordinator API | **Medium** |
| `node/fork_recovery.rs` | 712 | Enforce monotonic floor in `reset_state_only()` | **Medium** |

### Files that CAN stay untouched (12 files, ~7,300 lines)

| File | Lines | Why untouched |
|------|-------|---------------|
| `sync/manager/snap_sync.rs` | 327 | Snap sync logic correct; problem is coordination |
| `sync/fork_sync.rs` | 711 | Binary search works; consumed by coordinator as-is |
| `sync/fork_recovery.rs` | 373 | Parent chain walk works |
| `sync/reorg.rs` | 900 | Weight-based fork choice correct |
| `sync/bodies.rs` | 459 | Body downloader correct |
| `sync/headers.rs` | 226 | Header downloader correct |
| `sync/equivocation.rs` | 463 | Self-contained |
| `node/event_loop.rs` | 815 | Dispatch correct |
| `node/block_handling.rs` | 723 | Block application correct (post Fix #5) |
| `node/rollback.rs` | 288 | Rollback logic correct |
| `sync/manager/tests.rs` | 1774 | Tests stay, must pass |

---

## Q4: MoSCoW Prioritization

### Must (behavior preservation + critical fixes)

| ID | Requirement | Acceptance Criteria |
|----|------------|---------------------|
| REQ-SYNC-100 | Deploy Fix D: false fork detection | 0 false fork_mismatch_detected in 60-node test |
| REQ-SYNC-101 | Deploy Fix A: AwaitingCanonicalBlock 60s timeout | No permanent AwaitingCanonicalBlock |
| REQ-SYNC-102 | Monotonic progress floor as hard abort | No node resets below floor via any path |
| REQ-SYNC-103 | RecoveryCoordinator: single entry for all recovery | `needs_genesis_resync` set from 1 path, not 9 |
| PRESERVE-1 | Snap sync for new nodes (join within 120s) | New nodes sync successfully |
| PRESERVE-2 | Fork recovery for all depths (1-1000) | All fork depths resolve automatically |
| PRESERVE-3 | 11-layer production gate safety | No weakening of production gates |
| PRESERVE-4 | Wire protocol compatibility | No SyncRequest/SyncResponse changes |
| PRESERVE-5 | Existing test suite passes | All 1,774 lines of tests.rs pass |

### Should (structural improvements)

| ID | Requirement |
|----|------------|
| REQ-SYNC-104 | Enforce valid SyncState x RecoveryPhase combinations |
| REQ-SYNC-105 | Every RecoveryPhase carries timeout |
| REQ-SYNC-106 | Post-snap hash validation before header-first sync |

### Could (organizational)

| ID | Requirement |
|----|------------|
| REQ-SYNC-107 | Decompose cleanup() into focused functions |
| REQ-SYNC-108 | Extract ProductionGate as sub-struct |
| REQ-SYNC-109 | Formal state machine documentation |

### Won't (deferred)

| ID | Requirement | Reason |
|----|------------|--------|
| REQ-SYNC-110 | Snap sync timeout proportional to peer count | Heuristic tuning, not structural |
| REQ-SYNC-111 | Event queue replacing flag communication | RecoveryCoordinator solves this differently |

---

## Implementation Phases

**Phase 1 (deploy existing fixes)**: REQ-SYNC-100, 101, 102
**Phase 2 (RecoveryCoordinator)**: REQ-SYNC-103, 104, 105
**Phase 3 (post-snap validation)**: REQ-SYNC-106
**Phase 4 (optional cleanup)**: REQ-SYNC-107, 108, 109
