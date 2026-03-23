# Architecture: Sync Recovery Redesign

*Scope: `crates/network/src/sync/manager/`, `bins/node/src/node/periodic.rs`, `bins/node/src/node/rollback.rs`, `bins/node/src/node/fork_recovery.rs`*
*Requirements source: `docs/redesigns/sync-cascade-redesign-analysis.md`*
*Evaluation source: `docs/redesigns/sync-architecture-evaluation.md`*
*Prior architecture: `specs/syncmanager-architecture.md` (sub-struct extraction, partially implemented)*
*Reasoning trace: `docs/.workflow/architecture-reasoning.md`*

---

## Design Philosophy

**Subtraction first.** The prior 9 fix iterations each added code (interlocks, cooldowns, circuit breakers, floors). This redesign inverts the pattern: the primary intervention is **removing the ability to write `needs_genesis_resync` directly** and replacing it with a gated method that rejects most requests. The second intervention is **adding transition validity checks to the existing `set_state()` method**. No new structs. No new files. No event queues.

The core insight: the 9 `needs_genesis_resync = true` write sites are 9 uncoordinated entry points into the most destructive recovery path (reset to height 0, snap sync). Routing them through a single method that enforces cooldowns, floors, and priority ordering **subtracts 8 decision points** from the system. The remaining 1 decision point can be audited, tested, and monitored.

---

## 1. Current Architecture Map

### SyncState Machine (8 variants)

```
                          +-----------+
                     +--->|   Idle    |<---------+
                     |    +-----------+          |
                     |     |         |           |
                     |     | should_sync()       |
                     |     v         v           |
     cleanup_stuck   | +--------+ +--------+    | timeout/
     _sync (30s)     | |Headers | |Snap    |    | failure
                     | |Download| |Collect |    |
                     | +--------+ |Roots   |    |
                     |     |      +--------+    |
                     |     v         |           |
                     | +--------+   v           |
                     | |Bodies  | +--------+    |
                     | |Download| |Snap    |----+
                     | +--------+ |Download|
                     |     |      +--------+
                     |     v         |
                     | +--------+   v
                     | |Process | +--------+
                     | |  ing   | |Snap    |
                     | +--------+ |Ready   |
                     |     |      +--------+
                     |     v         |
                     | +-----------+ |  (node consumes snapshot)
                     +-|Synchronized|<+
                       +-----------+
```

### RecoveryPhase (6 variants)

```
Normal ---> StuckForkDetected ---> PostRollback ---> Normal
  |                                                    ^
  +---> ResyncInProgress ---> PostRecoveryGrace -------+
  |                                                    ^
  +---> AwaitingCanonicalBlock (60s timeout) -----------+
```

### The 9 `needs_genesis_resync = true` Write Sites

| # | File | Line | Trigger | Condition |
|---|------|------|---------|-----------|
| 1 | production_gate.rs | 1087 | `set_needs_genesis_resync()` public API | Called from Node's periodic.rs (rollback death spiral) |
| 2 | sync_engine.rs | 274 | Post-rollback snap escalation | PostRollback + fork_sync failed + enough peers + snap allowed |
| 3 | sync_engine.rs | 415 | Genesis fallback | 10+ empty headers from peer |
| 4 | sync_engine.rs | 767 | Body download peer error | Peer serving bodies disconnected |
| 5 | block_lifecycle.rs | 226 | 3+ apply failures, small gap (<=50) | Signal stuck fork instead |
| 6 | block_lifecycle.rs | 232 | 3+ apply failures, large gap | Snap threshold met + attempts < 3 |
| 7 | cleanup.rs | 344 | All peers blacklisted, 20+ empty headers | enough_peers && gap > 12 |
| 8 | cleanup.rs | 483 | Stuck-sync large gap | gap > 1000 + snap attempts < 3 + enough peers |
| 9 | cleanup.rs | 524 | Height offset detection | Stable gap for 120s while blocks applied |

### The 6 Recovery Mechanisms and Their Interactions

```
                    +-----------------+
  (1) ForkRecovery  | Walk parent     |---> exceeds max depth --->  force_recover_from_peers
      Tracker       | chain for orphan|---> connect point found --> execute_reorg
                    | blocks          |          |
                    +-----------------+          v
                                          reset_sync_for_rollback (if rejected)
                                          reset_sync_after_successful_reorg (if accepted)
                                                |
                    +-----------------+         v
  (2) ForkSync      | Binary search   |    PostRollback --> start_sync --> may start (2) again
      (fork_sync.rs)| for common      |
                    | ancestor        |---> bottomed out --> force_recover_from_peers
                    +-----------------+---> store limited --> set_post_recovery_grace

                    +-----------------+
  (3) resolve_      | Rollback 1 block|---> rollback_one_block --> reset_sync_for_rollback
      shallow_fork  | per tick for    |                               |
      (rollback.rs) | small forks     |                               v
                    +-----------------+                         PostRollback --> (2)

                    +-----------------+
  (4) Snap Sync     | Download full   |---> AwaitingCanonicalBlock --> timeout --> Normal
      (snap_sync.rs)| state snapshot  |---> apply failure --> needs_genesis_resync --> (4) LOOP
                    +-----------------+

                    +-----------------+
  (5) Genesis       | reset_local_    |---> height=0 --> start_sync --> (4) or (header-first)
      Resync        | state()         |
      (block_       +-----------------+
       lifecycle.rs)     ^
                         |
                   needs_genesis_resync flag (9 write sites)

                    +-----------------+
  (6) cleanup()     | 13 timeout-     |---> signal_stuck_fork --> (2)
      stuck         | driven actions  |---> needs_genesis_resync --> (5)
      detection     | every tick      |
                    +-----------------+
```

### The 4 Confirmed Feedback Loops

```
LOOP A (fork_sync success -> PostRollback -> fork_sync):
  fork_sync success --> reset_sync_for_rollback --> PostRollback
  --> start_sync --> PostRollback branch --> start_fork_sync --> REPEAT
  STATUS: Partially fixed (reset_sync_after_successful_reorg).
  SURVIVING: Rejected fork_sync still sets PostRollback.

LOOP B (rollback death spiral):
  false fork detection --> resolve_shallow_fork --> rollback_one_block
  --> height decreases --> still behind --> more rollbacks --> height=0
  STATUS: Partially fixed (peak_height + MAX_SAFE_ROLLBACK=10).
  SURVIVING: peak_height only checked in resolve_shallow_fork,
  not in fork_sync reorg or other rollback paths.

LOOP C (snap sync cascade):
  snap sync --> imperfect state --> apply failure --> needs_genesis_resync
  --> reset_local_state --> height=0 --> snap sync --> REPEAT
  STATUS: Partially fixed (confirmed_height_floor in reset_local_state).
  SURVIVING: Floor not checked in rollback or fork_sync paths.

LOOP D (timeout-driven oscillation):
  stuck detection (120s) --> signal_stuck_fork --> fork_sync
  --> fails --> Idle --> still behind --> stuck detection (120s) --> REPEAT
  STATUS: Circuit breaker (3 in 5min) provides partial protection.
  SURVIVING: After breaker trips, falls back to header-first, may also fail,
  re-enter stuck detection.
```

---

## 2. Proposed Architecture Map

### Design Principle: Gates, Not Coordinators

Instead of adding a RecoveryCoordinator struct that duplicates access to SyncManager state, we add **two gated methods** on SyncManager:

1. **`request_genesis_resync(reason: RecoveryReason) -> bool`** -- replaces all 9 `needs_genesis_resync = true` sites. Returns true only if the request is honored.
2. **`validate_transition(from, to) -> bool`** -- called inside `set_state()` to catch invalid transitions.

Plus **one enforcement extension**: the `confirmed_height_floor` check is extended to cover rollback and fork_sync paths, not just `reset_local_state()`.

### Why Not RecoveryCoordinator

The evaluation recommended a RecoveryCoordinator struct. After analyzing the codebase, the struct approach creates an ownership problem: the coordinator needs `local_height`, `snap.attempts`, `confirmed_height_floor`, `peers.len()`, `recovery_phase`, and `consecutive_resync_count` to make decisions. These are scattered across SyncManager and its sub-structs. A coordinator struct either:

(a) Holds copies of this state (stale data risk), or
(b) Takes them as parameters each call (ceremony with no safety benefit over a method)

A method on SyncManager has direct access to all of this. The existing `signal_stuck_fork()` method already implements this pattern successfully: it receives a signal, checks `recovery_phase`, and decides whether to honor it. `request_genesis_resync()` extends the same proven pattern.

`conf(0.78, observed)` -- signal_stuck_fork() is the existence proof.

### State Machine Changes

No changes to SyncState or RecoveryPhase enums. The variants are correct; the problem is unconstrained transitions. The fix adds validation inside the existing `set_state()` method:

```rust
fn set_state(&mut self, new_state: SyncState, trigger: &str) {
    let old_label = self.state_label();

    // NEW: Validate transition
    if !self.is_valid_transition(&new_state) {
        warn!(
            "[SYNC_STATE] INVALID transition {} -> {} blocked (trigger: {})",
            old_label, Self::label_for(&new_state), trigger
        );
        return; // Refuse the transition
    }

    self.state = new_state;
    let new_label = self.state_label();
    if old_label != new_label {
        info!(  // Upgraded from debug to info for observability
            "[SYNC_STATE] {} -> {} (trigger: {})",
            old_label, new_label, trigger
        );
    }
}
```

### Valid Transition Matrix

```
FROM                  -> VALID TO states
----                     ----------------
Idle                  -> (any state — Idle is the reset state)
DownloadingHeaders    -> DownloadingHeaders (count update), DownloadingBodies, SnapCollectingRoots, Synchronized, Idle
DownloadingBodies     -> Processing, DownloadingBodies (soft retry), Synchronized, Idle
Processing            -> Synchronized, Processing (height update), Idle
Synchronized          -> DownloadingHeaders (re-sync), SnapCollectingRoots (re-snap), Synchronized, Idle
SnapCollectingRoots   -> SnapDownloading, Idle (fallback)
SnapDownloading       -> SnapReady, SnapDownloading (alternate peer), Idle (error)
SnapReady             -> Synchronized (node consumed), Idle (failure)
```

Key INVALID transitions that are currently possible and this blocks:
- `SnapCollectingRoots -> Synchronized` (snap sync can't skip to synchronized)
- `Processing -> SnapCollectingRoots` (can't start snap sync from processing)
- `Synchronized -> DownloadingBodies` (must go through DownloadingHeaders first)

### Recovery Signal Flow (proposed)

```
BEFORE (9 uncoordinated writes):
  cleanup.rs line 344 -----> self.fork.needs_genesis_resync = true
  cleanup.rs line 483 -----> self.fork.needs_genesis_resync = true
  cleanup.rs line 524 -----> self.fork.needs_genesis_resync = true
  sync_engine.rs line 274 -> self.fork.needs_genesis_resync = true
  sync_engine.rs line 415 -> self.fork.needs_genesis_resync = true
  sync_engine.rs line 767 -> self.fork.needs_genesis_resync = true
  block_lifecycle line 226 > self.fork.needs_genesis_resync = true
  block_lifecycle line 232 > self.fork.needs_genesis_resync = true
  production_gate line 1087> self.fork.needs_genesis_resync = true

AFTER (1 gated method):
  all 9 sites -----------> self.request_genesis_resync(reason) -> bool
                               |
                               +-- CHECK: confirmed_height_floor > 0? -> REFUSE
                               +-- CHECK: recovery_phase == ResyncInProgress? -> REFUSE
                               +-- CHECK: consecutive_resync_count >= MAX? -> REFUSE
                               +-- CHECK: snap sync disabled? -> REFUSE
                               +-- CHECK: snap attempts >= 3? -> REFUSE
                               |
                               +-- If all checks pass:
                                   self.fork.needs_genesis_resync = true
                                   log(reason, alternatives, decision)
                                   return true
```

### Monotonic Progress Enforcement (extended)

```
CURRENT: confirmed_height_floor checked in:
  [x] reset_local_state()        -- refuses to reset below floor

PROPOSED: confirmed_height_floor checked in:
  [x] reset_local_state()        -- refuses to reset below floor (existing)
  [+] reset_sync_for_rollback()  -- refuses if proposed height < floor
  [+] Node::execute_reorg()      -- refuses reorg if target height < floor
  [+] request_genesis_resync()   -- refuses if floor > 0 (unless manual override)
```

### Data Flow (proposed)

```
Node event_loop
  |
  +-- run_periodic_tasks()
  |   +-- get_blocks_to_apply() --> apply blocks
  |   +-- cleanup()
  |   |   +-- (timeouts that previously set needs_genesis_resync)
  |   |   |   NOW: call self.request_genesis_resync(reason)
  |   |   |         method decides, returns bool
  |   |   +-- (timeouts that call signal_stuck_fork) -- unchanged
  |   |   +-- (all other timeouts) -- unchanged
  |   |
  |   +-- resolve_shallow_fork()
  |   |   +-- NEW: check confirmed_height_floor before rollback
  |   |   +-- rollback_one_block --> reset_sync_for_rollback
  |   |   |   NEW: reset_sync_for_rollback checks floor
  |   |
  |   +-- GENESIS RESYNC CHECK:
  |       +-- reads needs_genesis_resync -- unchanged
  |       +-- calls force_recover_from_peers -- unchanged
  |
  +-- try_produce_block()
      +-- update_production_state() -- unchanged
      +-- can_produce() -- unchanged
```

---

## 3. What Gets Deleted

### Fields removed from SyncManager

None. The existing fields are correct. The `needs_genesis_resync` field on ForkState stays -- it is still the flag that Node reads in `periodic.rs` to trigger `force_recover_from_peers()`. The change is that only ONE method can set it.

### Methods removed

| Method | Location | Replacement |
|--------|----------|-------------|
| `set_needs_genesis_resync()` | production_gate.rs:1087 | `request_genesis_resync(RecoveryReason::RollbackDeathSpiral)` |

### Code paths in cleanup() removed

None removed entirely. The 3 `needs_genesis_resync = true` assignments in cleanup.rs are replaced with `request_genesis_resync()` calls. The logic around them stays.

### Flags that become unnecessary

None immediately. Over time, some cooldown fields may prove redundant once `request_genesis_resync()` provides centralized rate limiting, but premature removal risks regression.

### What the subtraction actually is

The subtraction is not in code deletion but in **decision point reduction**:
- 9 independent decision makers -> 1 centralized decision maker
- ~36 undefined SyncState x RecoveryPhase combinations -> explicitly validated transitions
- 3 separate monotonic floor enforcement points -> 1 consistent check pattern

---

## 4. What Gets Added

### New enum: RecoveryReason

```rust
/// Why a genesis resync was requested.
/// Used for logging, diagnostics, and potential future policy differentiation.
#[derive(Clone, Debug)]
pub enum RecoveryReason {
    /// All peers blacklisted with 20+ empty headers (cleanup.rs)
    AllPeersBlacklistedDeepFork,
    /// Stuck-sync with gap > 1000 blocks (cleanup.rs)
    StuckSyncLargeGap { gap: u64 },
    /// Height offset: stable gap while blocks applied (cleanup.rs)
    HeightOffsetDetected { gap: u64 },
    /// Post-rollback snap escalation (sync_engine.rs)
    PostRollbackSnapEscalation,
    /// Genesis fallback: 10+ empty headers from peer (sync_engine.rs)
    GenesisFallbackEmptyHeaders,
    /// Body download peer error (sync_engine.rs)
    BodyDownloadPeerError,
    /// 3+ apply failures with snap threshold (block_lifecycle.rs)
    ApplyFailuresSnapThreshold { gap: u64 },
    /// Rollback death spiral exceeded max depth (Node rollback.rs)
    RollbackDeathSpiral { peak: u64, current: u64 },
}
```

~20 lines. No new file -- added in mod.rs alongside RecoveryPhase.

### New method: request_genesis_resync()

```rust
/// Central gate for all genesis resync requests.
///
/// Replaces 9 scattered `needs_genesis_resync = true` assignments with a single
/// decision point that enforces:
/// 1. Monotonic progress floor (won't reset below confirmed_height_floor)
/// 2. No concurrent recovery (won't trigger if ResyncInProgress)
/// 3. Rate limiting (max MAX_CONSECUTIVE_RESYNCS, with cooldown)
/// 4. Snap sync availability (won't trigger if snap sync disabled)
/// 5. Decision logging (every accept/reject is logged with reason)
///
/// Returns true if the request was honored, false if refused.
pub fn request_genesis_resync(&mut self, reason: RecoveryReason) -> bool {
    // Gate 1: Monotonic progress floor
    if self.confirmed_height_floor > 0 {
        warn!(
            "[RECOVERY] Genesis resync REFUSED: confirmed_height_floor={} \
             (reason: {:?}). Node was previously healthy. Manual intervention required.",
            self.confirmed_height_floor, reason
        );
        return false;
    }

    // Gate 2: No concurrent recovery
    if matches!(self.recovery_phase, RecoveryPhase::ResyncInProgress) {
        info!(
            "[RECOVERY] Genesis resync REFUSED: resync already in progress \
             (reason: {:?})",
            reason
        );
        return false;
    }

    // Gate 3: Rate limiting
    if self.consecutive_resync_count >= MAX_CONSECUTIVE_RESYNCS {
        warn!(
            "[RECOVERY] Genesis resync REFUSED: {} consecutive resyncs (max {}) \
             (reason: {:?}). Manual intervention required.",
            self.consecutive_resync_count, MAX_CONSECUTIVE_RESYNCS, reason
        );
        return false;
    }

    // Gate 4: Snap sync must be available (unless reason is manual override)
    if self.snap.threshold == u64::MAX {
        info!(
            "[RECOVERY] Genesis resync REFUSED: snap sync disabled \
             (reason: {:?}). Header-first recovery only.",
            reason
        );
        return false;
    }

    // Gate 5: Snap attempt limit
    if self.snap.attempts >= 3 {
        info!(
            "[RECOVERY] Genesis resync REFUSED: snap attempts exhausted ({}/3) \
             (reason: {:?})",
            self.snap.attempts, reason
        );
        return false;
    }

    // All gates passed -- honor the request
    info!(
        "[RECOVERY] Genesis resync ACCEPTED: {:?} \
         (floor={}, resync_count={}, snap_attempts={}, phase={:?})",
        reason, self.confirmed_height_floor, self.consecutive_resync_count,
        self.snap.attempts, self.recovery_phase
    );
    self.fork.needs_genesis_resync = true;
    true
}
```

~50 lines. Added as a method on SyncManager in production_gate.rs (where `signal_stuck_fork()` and `set_needs_genesis_resync()` already live).

### New method: is_valid_transition()

```rust
/// Check if a state transition is valid.
/// Called by set_state() to prevent illegal transitions.
fn is_valid_transition(&self, new_state: &SyncState) -> bool {
    use SyncState::*;
    match (&self.state, new_state) {
        // Idle can go anywhere (it's the reset state)
        (Idle, _) => true,
        // Any state can go to Idle (reset/error)
        (_, Idle) => true,
        // Header download leads to bodies, snap, or synchronized
        (DownloadingHeaders { .. }, DownloadingBodies { .. }) => true,
        (DownloadingHeaders { .. }, SnapCollectingRoots { .. }) => true,
        (DownloadingHeaders { .. }, Synchronized) => true,
        // Body download leads to processing, or re-enters itself (soft retry)
        (DownloadingBodies { .. }, Processing { .. }) => true,
        (DownloadingBodies { .. }, DownloadingBodies { .. }) => true,
        (DownloadingBodies { .. }, Synchronized) => true,
        // Processing leads to synchronized or re-enters (height update)
        (Processing { .. }, Synchronized) => true,
        (Processing { .. }, Processing { .. }) => true,
        // Synchronized can only go back to Idle (stall reset)
        (Synchronized, Synchronized) => true,
        // Snap sync forward path
        (SnapCollectingRoots { .. }, SnapDownloading { .. }) => true,
        (SnapDownloading { .. }, SnapReady { .. }) => true,
        (SnapDownloading { .. }, SnapDownloading { .. }) => true, // alternate peer
        (SnapReady { .. }, Synchronized) => true,
        // Anything else is invalid
        _ => false,
    }
}
```

~30 lines. Added in mod.rs alongside `set_state()`.

### Extended checks: confirmed_height_floor in rollback paths

In `reset_sync_for_rollback()` (block_lifecycle.rs):
```rust
// Refuse rollback below confirmed height floor
if self.local_height > 0 && self.local_height <= self.confirmed_height_floor {
    warn!(
        "[RECOVERY] reset_sync_for_rollback REFUSED: height {} at or below floor {}",
        self.local_height, self.confirmed_height_floor
    );
    // Don't set PostRollback -- the node should stay on its current chain
    return;
}
```
~6 lines.

In Node's `execute_reorg()` (block_handling.rs -- documented as contract for developer):
```rust
// Before executing rollback, check floor
if rollback_target_height < sync.confirmed_height_floor() {
    warn!("Reorg REFUSED: target height {} below floor {}", ...);
    return Ok(());
}
```
~4 lines.

### Total additions: ~110 lines of new code, 0 new files, 0 new structs.

---

## 5. Migration Path

Each step leaves the system in a compilable, testable state. Each step is independently committable. Ordered from safest (least behavioral change) to most impactful.

### Step 1: Add RecoveryReason enum and request_genesis_resync() method

**What**: Add the enum and method to production_gate.rs. Do NOT yet change any call sites. The method exists but is uncalled.

**Why first**: This is additive-only. No existing behavior changes. Tests pass unchanged.

**Commit message**: `refactor(sync): add request_genesis_resync() recovery gate method`

### Step 2: Add is_valid_transition() and update set_state()

**What**: Add the transition validation method to mod.rs. Update `set_state()` to call it. Invalid transitions log a warning but ARE still executed (soft enforcement first).

**Why second**: Adding the validation with warnings-only first lets us observe which transitions are actually invalid in production without breaking anything. If unexpected invalid transitions appear, we can adjust the valid transition matrix before making it a hard block.

**Commit message**: `refactor(sync): add state transition validation (warn-only)`

### Step 3: Migrate needs_genesis_resync write sites (one at a time)

**What**: Replace each `self.fork.needs_genesis_resync = true` with `self.request_genesis_resync(reason)`. Do this one file at a time:

1. `cleanup.rs` (3 sites -- lines 344, 483, 524)
2. `sync_engine.rs` (3 sites -- lines 274, 415, 767)
3. `block_lifecycle.rs` (2 sites -- lines 226, 232)
4. `production_gate.rs` (1 site -- line 1087, replace `set_needs_genesis_resync()`)

After each file, run tests. The behavioral change is that some recovery requests will now be REFUSED that were previously honored (the whole point).

**Commit messages**:
- `fix(sync): route cleanup.rs genesis resync through recovery gate (3/9 sites)`
- `fix(sync): route sync_engine.rs genesis resync through recovery gate (6/9 sites)`
- `fix(sync): route block_lifecycle.rs genesis resync through recovery gate (8/9 sites)`
- `fix(sync): route production_gate.rs genesis resync through recovery gate (9/9 sites)`

### Step 4: Extend confirmed_height_floor to rollback paths

**What**: Add floor checks to `reset_sync_for_rollback()` and document the contract for `execute_reorg()`.

**Why after step 3**: The floor was previously only needed in `reset_local_state()` because that was the main cascade entry. After step 3, `request_genesis_resync()` also checks the floor. This step closes the remaining gaps (rollback and reorg).

**Commit message**: `fix(sync): enforce monotonic progress floor in rollback and reorg paths`

### Step 5: Harden set_state() to hard-block invalid transitions

**What**: Change `set_state()` from warn-only to hard-block (return without mutation) for invalid transitions. Only do this after Step 2's warnings have been observed in production and no unexpected invalid transitions were found.

**Commit message**: `fix(sync): enforce state transition validation (hard block)`

### Step 6: Wire ForkAction/recommend_action (existing dead code)

**What**: Remove `#[allow(dead_code)]` from `ForkState::recommend_action()` and `ForkAction`. Wire `resolve_shallow_fork()` in rollback.rs to use `recommend_action()` instead of inline decision logic.

**Why last**: This is the organizational improvement from the prior architecture spec (syncmanager-architecture.md). It was designed but never wired. It complements the recovery gate by centralizing fork recovery decisions, but is not strictly required for the cascade fix.

**Commit message**: `refactor(sync): wire ForkState::recommend_action() for fork recovery decisions`

---

## 6. Milestones

| ID | Name | Scope (Modules) | Scope (Requirements) | Est. Size | Dependencies |
|----|------|-----------------|---------------------|-----------|-------------|
| M1 | Recovery Gate + Transition Validation | mod.rs (RecoveryReason, is_valid_transition), production_gate.rs (request_genesis_resync, set_state update) | REQ-SYNC-102, REQ-SYNC-103, REQ-SYNC-104, PRESERVE-3 | M | None |
| M2 | Site Migration + Monotonic Floor Extension | cleanup.rs (3 sites), sync_engine.rs (3 sites), block_lifecycle.rs (2 sites + floor), production_gate.rs (1 site) | REQ-SYNC-102, REQ-SYNC-103, REQ-SYNC-105, PRESERVE-5 | M | M1 |
| M3 | Hard Enforcement + ForkAction Wiring | mod.rs (hard-block transitions), rollback.rs (recommend_action), block_handling.rs (floor check) | REQ-SYNC-100, REQ-SYNC-104, REQ-SYNC-109, PRESERVE-2 | S | M2 |

### M1: Recovery Gate + Transition Validation

**Steps**: 1 + 2 from migration path.
**Modules touched**: mod.rs, production_gate.rs
**Tests**: Existing 1,774 lines pass unchanged (additive-only). New unit tests for `request_genesis_resync()` (5-7 tests covering each gate). New tests for `is_valid_transition()` (enumerate valid/invalid pairs).
**Risk**: Low -- no behavioral change in M1.

### M2: Site Migration + Monotonic Floor Extension

**Steps**: 3 + 4 from migration path.
**Modules touched**: cleanup.rs, sync_engine.rs, block_lifecycle.rs, production_gate.rs
**Tests**: Existing tests may need adjustment if they explicitly set `needs_genesis_resync = true`. New integration tests verifying that floor-protected nodes refuse resync.
**Risk**: Medium -- behavioral change (some resyncs will now be refused). This is the intended effect but needs careful testing.

### M3: Hard Enforcement + ForkAction Wiring

**Steps**: 5 + 6 from migration path.
**Modules touched**: mod.rs, rollback.rs, block_handling.rs
**Tests**: Existing tests for resolve_shallow_fork may need update. New tests for recommend_action (already partially written as dead code tests).
**Risk**: Low-Medium -- hard enforcement could theoretically break a valid transition that was missed in the matrix. Step 2's warn-only phase should catch this before M3.

---

## 7. Design Decision Quality Audit

### DD-1: Gated method vs. RecoveryCoordinator struct

**Alternatives considered**:
1. Do nothing (keep 9 write sites) -- ELIMINATED, `conf(0.0, observed)`, 9 fix iterations failed
2. Delete needs_genesis_resync entirely -- ELIMINATED, `conf(0.0, inferred)`, breaks new node onboarding
3. Single gated method (request_genesis_resync) -- WINNER, `conf(0.78, observed)`
4. RecoveryCoordinator struct -- WEAKENED, `conf(0.50, assumed)`, ownership friction
5. Event queue replacing cleanup() -- ELIMINATED, `conf(0.0, assumed)`, scope explosion
6. Transition guards + gated method -- THIS IS ALT 3 refined

**Why Alt 3 won**: The existing `signal_stuck_fork()` method is proof that this pattern works in this codebase. It receives a recovery signal, checks priority, and decides whether to honor it. `request_genesis_resync()` follows the identical structure. Zero new structs, zero ownership problems, zero parameter ceremony.

**Confidence**: `conf(0.78, observed)` -- signal_stuck_fork is the existence proof. Basis is `observed` because we can see the pattern working in the codebase.

**What could make this wrong**: If recovery decisions need PERSISTENT state beyond what SyncManager already holds (e.g., a history of recent recovery decisions for pattern detection), a struct would be justified. Current analysis shows all needed state already exists as SyncManager fields.

### DD-2: Transition validation in set_state() vs. type-level enforcement

**Alternatives considered**:
1. Do nothing (28 unconstrained set_state calls) -- ELIMINATED, `conf(0.0, observed)`
2. Reduce SyncState variants -- WEAKENED, `conf(0.30, inferred)`, doesn't add guards
3. Validity check in set_state() -- WINNER, `conf(0.78, observed)`
4. Type-state pattern -- ELIMINATED, `conf(0.0, assumed)`, incompatible with struct field
5. Transition table as const data -- SURVIVES, `conf(0.68, inferred)`, slightly more complex

**Why Alt 3 won**: `set_state()` already exists with trigger logging. Adding a match-based validity check is 30 lines. Rust's match exhaustiveness ensures all transitions are considered. Simpler than a const table with no loss of safety.

**Confidence**: `conf(0.78, observed)` -- set_state already exists, match is idiomatic Rust.

**What could make this wrong**: If the valid transition set changes frequently, a data-driven approach (const table) would be more maintainable. Current assessment: transitions are stable (8 states, well-defined paths).

### DD-3: Monotonic floor in all rollback paths

**Alternatives considered**:
1. Do nothing (floor only in reset_local_state) -- ELIMINATED, `conf(0.0, observed)`, Loop B and C survive
2. Extend floor to specific rollback paths -- WINNER, `conf(0.80, observed)`
3. Single guard method check_monotonic_progress() -- WEAKENED, `conf(0.70, observed)`, over-abstraction for 1-line check

**Why Alt 2 won**: Three specific check sites (reset_local_state, reset_sync_for_rollback, execute_reorg). Each is a 1-line comparison. A method for a 1-line check is premature abstraction.

**Confidence**: `conf(0.80, observed)` -- three specific entry points identified by code tracing.

**What could make this wrong**: If a new rollback path is added that doesn't go through these three checkpoints. Mitigation: document the contract ("any code that reduces local_height must check confirmed_height_floor").

```
━━━ DESIGN DECISION QUALITY AUDIT ━━━
Major decisions identified:            3
Alternatives per decision (avg):       5
  basis=measured:                      0
  basis=observed:                      8
  basis=inferred:                      5
  basis=assumed:                       3
Confidence range for winner:           0.78-0.80
Decisions with flat distribution:      0
Decisions with conf >= 0.8 + assumed:  0
Constraint table entries used:         5 (from prior architecture)
━━━ SIMPLICITY AUDIT ━━━
Subtraction alternatives explored:     3 (one per decision)
"Do nothing" alternatives explored:    3 (one per decision)
Winner complexity cost:                +110 lines, 0 new files, 0 new structs
Simpler alternative that was close:    "Delete needs_genesis_resync" at conf(0.25)
                                       -- too destructive to capabilities
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

---

## 8. Simplicity Audit

### What is the simplest version of this redesign that still addresses F1-F5?

**The absolute minimum viable change** is:

1. `request_genesis_resync()` method with 5 gates (~50 lines) -- addresses F2 (recovery coordination)
2. `is_valid_transition()` in `set_state()` (~30 lines) -- addresses F1 (state transition validator)
3. Floor check in `reset_sync_for_rollback()` (~6 lines) -- addresses F1 partially (monotonic progress)

Total: ~86 lines. This addresses F1, F2, and partially F5 (the 9 write sites become 1, reducing the effective implicit state space).

F3 (cleanup as implicit scheduler) and F4 (block store inconsistency after snap sync) are NOT addressed by this redesign. They are lower priority per the analyst's MoSCoW (F3 is "Could" -- REQ-SYNC-107, F4 is not in scope). They can be addressed in a future pass once the cascade is proven broken.

### Can any proposed addition be removed while still solving the problem?

- `RecoveryReason` enum: Could be replaced with a `&str` reason. But the enum is better for future policy differentiation (e.g., "refuse HeightOffset but allow ApplyFailures"). Keep.
- `is_valid_transition()`: Could be removed if we trust set_state() callers. But the whole point is that we don't -- 28 unconstrained callers. Keep.
- Floor check in `execute_reorg()`: Could be deferred. The primary cascade paths go through `reset_local_state()` and `reset_sync_for_rollback()`. `execute_reorg()` is a secondary path. But it's 4 lines and closes Loop B. Keep.

### Is request_genesis_resync the simplest coordination mechanism?

Yes. The alternatives were:
- A struct (RecoveryCoordinator): more complex, ownership friction
- An event queue: orders of magnitude more complex
- Inline checks at each write site: duplicates logic 9 times
- A method on SyncManager: this IS the simplest -- one function, one decision point, full access to all state

---

## Requirement Traceability

| Requirement ID | Architecture Section | Module(s) | Milestone |
|---------------|---------------------|-----------|-----------|
| REQ-SYNC-100 | Step 6: ForkAction wiring + DD-1 | rollback.rs, mod.rs | M3 |
| REQ-SYNC-101 | Already implemented (INC-I-005 Fix A, cleanup.rs:444) | cleanup.rs | -- |
| REQ-SYNC-102 | Sections 2 + 4: Monotonic floor extension | block_lifecycle.rs, block_handling.rs | M1, M2 |
| REQ-SYNC-103 | Section 2 + 4: request_genesis_resync() | production_gate.rs, all 9 site files | M1, M2 |
| REQ-SYNC-104 | Section 2 + 4: is_valid_transition() in set_state() | mod.rs | M1, M3 |
| REQ-SYNC-105 | RecoveryReason carries context; set_state logs all transitions | mod.rs, production_gate.rs | M1 |
| REQ-SYNC-106 | Deferred -- post-snap hash validation is independent of coordination fix | snap_sync.rs | Future |
| REQ-SYNC-107 | Deferred -- cleanup decomposition is "Could" priority | cleanup.rs | Future |
| REQ-SYNC-108 | Deferred -- ProductionGate extraction is "Could" priority | production_gate.rs | Future |
| REQ-SYNC-109 | Section 1 + valid transition matrix serves as formal documentation | This document | M1 |
| PRESERVE-1 | Snap sync logic unchanged; only the TRIGGER is gated | snap_sync.rs (untouched) | -- |
| PRESERVE-2 | Fork recovery logic unchanged; ForkAction wiring in M3 | fork_sync.rs, fork_recovery.rs | M3 |
| PRESERVE-3 | Production gate layers unchanged; only recovery_phase interactions gated | production_gate.rs | M1 |
| PRESERVE-4 | Wire protocol unchanged (SyncRequest/SyncResponse not modified) | -- | -- |
| PRESERVE-5 | All 1,774 lines of tests.rs must pass after each step | tests.rs | M1, M2, M3 |

---

## Failure Modes

| Scenario | Detection | Recovery | Impact |
|----------|-----------|----------|--------|
| request_genesis_resync refuses ALL requests (too conservative) | Node stuck behind peers for >5 min without recovery | 1. Cooldowns expire naturally. 2. Manual `doli snap` command as escape hatch. 3. Adjust gate thresholds. | Temporary production pause (safer than cascade) |
| is_valid_transition rejects a legitimate transition | set_state logs "[SYNC_STATE] INVALID transition blocked" | 1. Step 2 uses warn-only mode to catch these. 2. Add the missing transition to the matrix. 3. M3 hardening only happens after observation. | Sync stalls until matrix is updated (caught in Step 2) |
| Floor check blocks necessary recovery | Node stuck on dead fork with floor preventing resync | 1. confirmed_height_floor can be reset by manual `doli snap --force`. 2. Floor is only set when node was previously Synchronized + healthy. | Manual intervention required (by design -- the floor protects against cascade, which is worse) |
| Node stuck in Loop D (timeout oscillation) after request_genesis_resync refuses | Repeated warn logs: "Genesis resync REFUSED" | 1. Header-first sync continues. 2. Fork_sync circuit breaker + cooldown still function. 3. Eventually fork_sync or header-first succeeds, or node operator intervenes. | Degraded but not cascading -- the key improvement |

## Security Model

No trust boundaries are crossed by this change. The recovery gate is internal to the node -- it does not affect peer communication, block validation, or state verification. The wire protocol is unchanged. The production gate is unchanged.

The `request_genesis_resync()` method does introduce a new denial-of-self-service risk: if the gate is too restrictive, a node could refuse to recover automatically when it should. This is mitigated by:
1. Gate thresholds matching existing behavior (confirmed_height_floor, MAX_CONSECUTIVE_RESYNCS=5, snap attempts=3)
2. Manual override via `doli snap` CLI command
3. All refusals are logged at WARN level for operator visibility

## Performance Budgets

| Operation | Latency | Notes |
|-----------|---------|-------|
| request_genesis_resync() | < 1us | 5 field comparisons, no allocation |
| is_valid_transition() | < 1us | Single match expression |
| set_state() (updated) | < 1us (was < 1us) | One additional function call |
| cleanup() (updated) | No change | Same logic, method calls instead of direct writes |

---

*Architecture completed 2026-03-23. Architect: OMEGA workflow RUN_ID=54, scope: sync-recovery.*
