# Sync Subsystem Architecture Evaluation

> **Evaluator**: Architect (adversarial analysis)
> **Date**: 2026-03-23
> **Branch**: `fix/sync-state-explosion-root-causes`
> **Incident**: INC-I-005 (snap sync cascade, 9 fixes, 10 sessions)
> **Scope**: ~9,900 lines across 19 files in `crates/network/src/sync/` + `bins/node/src/node/` (fork_recovery, rollback, block_handling, event_loop)

---

## Verdict

**This is a structural architecture problem, not a bug cluster.**

conf(0.92, observed)

Nine targeted fixes over 10 sessions each resolved a real bug. Yet the cascade persists because the fixes operate inside a system whose architecture _generates_ new cascade entry points faster than point fixes can close them. The evidence is unambiguous:

1. **28 state transition sites** (`set_state()`) and **18 recovery_phase mutation sites** with no transition validator -- any combination can fire in any order.
2. **Three parallel fork recovery systems** (ForkRecoveryTracker, ForkSync, resolve_shallow_fork) with no coordination protocol -- each can trigger the others.
3. **The cleanup() god function** (543 lines, 13 responsibilities) runs every tick and can trigger state transitions that conflict with transitions initiated by other subsystems in the same tick.
4. **No monotonic progress invariant** is enforced at the state machine level. The `confirmed_height_floor` (INC-I-005 Fix C) is a band-aid on `reset_local_state()` only -- rollback, fork_sync, and resolve_shallow_fork can all reduce height below any previously-confirmed floor without checking it.
5. **The bug pattern is consistent**: each fix closes one entry point into the cascade, the cascade reroutes through another. This is the hallmark of an implicit state machine with undefined transitions, not a finite set of bugs.

A targeted fix strategy _cannot_ work because the number of potential cascade paths grows combinatorially with the number of state variables, recovery mechanisms, and timeout-driven actions. The system currently has:
- 8 SyncState variants x 6 RecoveryPhase variants = 48 state pairs (most undefined)
- 3 parallel recovery subsystems = 3 independent escalation paths
- 10+ timeout-driven actions in cleanup() = 10+ potential conflicting transitions per tick
- ~55 boolean/counter fields that form an implicit state space of 2^55

Fixing individual entry points in this space is a game of whack-a-mole. The architecture must change.

---

## D1: State Machine Integrity

**Finding**: The SyncState x RecoveryPhase state space is not well-defined. Most combinations are not explicitly handled, and external events can drive the system into states where multiple recovery mechanisms fight each other.

**Evidence from code**:

1. **SyncState has 8 variants, RecoveryPhase has 6 variants = 48 combinations.** Only ~12 are explicitly handled in code. The remaining ~36 are implicitly "whatever happens, happens." Example: `SnapCollectingRoots` + `StuckForkDetected` is reachable (fork detected during snap sync collection), but no code handles this combination. Cleanup's snap timeout and fork recovery can both fire.

2. **No transition validator.** The `set_state()` method (found via grep at 28 call sites) accepts any `SyncState` from any state. There is no guard like "you can only enter DownloadingHeaders from Idle." Example: `cleanup_stuck_sync` sets Idle from any syncing state, while `block_applied` can set Synchronized from any syncing state -- if both fire in the same tick, the last write wins with no conflict detection.

3. **RecoveryPhase mutations are scattered.** 18 sites mutate `recovery_phase` across 5 files (production_gate.rs: 6, block_lifecycle.rs: 4, sync_engine.rs: 1, cleanup.rs: 2, snap_sync.rs: 1, plus tests). There is no single function that validates transitions. Example: `cleanup()` can force-clear `PostRecoveryGrace` at 120s (cleanup.rs:434) while `block_applied()` is simultaneously counting blocks to clear it at 10 blocks (block_lifecycle.rs:93). Race depends on tick ordering.

4. **ForkSync creates a shadow state machine.** `fork_sync: Option<ForkSync>` operates in parallel with `SyncState`. When `fork_sync.is_some()`, the system is effectively in a "ForkSyncing" state that is invisible to `SyncState`. Code checks `self.fork.fork_sync.is_some()` at 8+ sites to branch behavior, creating an implicit compound state `(SyncState, Option<ForkSync>)` with no type-level enforcement.

**Confidence**: conf(0.93, observed) -- direct code reading, 28 transition sites counted, 18 recovery_phase mutations counted.

---

## D2: Invariant Coverage

**Finding**: The system has one explicitly declared invariant (monotonic progress via `confirmed_height_floor`) that is only partially enforced. Multiple assumed invariants are violated in practice.

**Evidence**:

### Explicitly declared invariants

| Invariant | Where declared | Where enforced | Gaps |
|-----------|---------------|----------------|------|
| Monotonic progress (height floor) | block_lifecycle.rs:248 | `reset_local_state()` only | **Rollback, fork_sync, resolve_shallow_fork all bypass it.** `rollback_one_block()` (rollback.rs:22) simply decrements height with no floor check. `fork_sync` reorg via `execute_reorg()` rolls back to ancestor height with no floor check. |
| Idempotent start_sync | sync_engine.rs:95 | `is_syncing()` guard | Works for the guard itself, but cleanup() can reset to Idle and immediately call `start_sync()`, effectively bypassing the idempotency by cycling through Idle first. |
| Sync epoch invalidates stale responses | sync_engine.rs:116 | Epoch check on response handling | Appears correctly enforced. |

### Assumed but unenforced invariants

1. **"A node should never oscillate between height H and height 0."** The `reset_local_state()` function can set height=0, and `rollback_one_block()` can progressively reach height=0 through depth-unlimited rollback. The `confirmed_height_floor` only guards `reset_local_state()`, not rollback paths. conf(0.90, observed)

2. **"Recovery mechanisms should not trigger each other."** There is no interlock. Fork_sync completion calls `reset_sync_for_rollback()` which sets `PostRollback`, which causes `start_sync()` to start another fork_sync. This was the exact INC-001 rollback loop. The fix (`reset_sync_after_successful_reorg`) addressed one path, but `signal_stuck_fork()` (called from 4 sites) and `needs_genesis_resync` (set from 8 sites) can still re-enter the cascade. conf(0.88, observed)

3. **"Block store and chain_state should be consistent."** Snap sync replaces ChainState/UtxoSet/ProducerSet but leaves block_store with old fork blocks. Any code reading `block_store.get_block_by_height()` after snap sync may get wrong data. This invariant was violated by at least 3 bugs (fixes #2, #5, and block_handling code). conf(0.95, observed)

4. **"Production gate should eventually allow production."** Multiple paths can permanently block production: `fork_mismatch_detected` + no peers at same height (Layer 8.5 never clears), `AwaitingCanonicalBlock` with no gossip (only has 60s timeout added as INC-I-005 Fix A), exponential backoff grace period (capped at 60s by PGD-002 but was previously unbounded). conf(0.85, observed)

**Confidence**: conf(0.90, observed) -- invariants identified by code reading, gaps verified by tracing execution paths.

---

## D3: Recovery Architecture

**Finding**: Three independent recovery mechanisms with no coordination protocol. Recovery actions can trigger other recovery mechanisms, forming feedback loops.

### Recovery mechanism inventory

| # | Mechanism | Location | Trigger | Action | Can trigger another? |
|---|-----------|----------|---------|--------|---------------------|
| 1 | ForkRecoveryTracker | fork_recovery.rs (322 lines) | Orphan block received | Walk parent chain, download, reorg | Yes: reorg can trigger reset_sync_for_rollback -> PostRollback -> fork_sync |
| 2 | ForkSync | fork_sync.rs (711 lines) | PostRollback in start_sync(), signal_stuck_fork() | Binary search for ancestor, download canonical, reorg | Yes: reorg calls reset_sync_for_rollback (rejection) or reset_sync_after_successful_reorg (success). Both can lead to start_sync() -> potentially another fork_sync |
| 3 | resolve_shallow_fork | Node's periodic.rs | consecutive_empty_headers >= 3, gap < 100 | Rollback one block at a time | Yes: rollback calls reset_sync_for_rollback -> PostRollback -> start_sync -> fork_sync |
| 4 | Snap sync | snap_sync.rs (327 lines) | Gap > threshold, or needs_genesis_resync | Download full state snapshot | Yes: snap sync -> AwaitingCanonicalBlock -> timeout -> Normal -> cleanup detects behind -> start_sync |
| 5 | Genesis resync | block_lifecycle.rs:237 | needs_genesis_resync flag | reset_local_state() -> height=0 | Yes: resets to height 0 -> start_sync -> snap_sync or header-first |
| 6 | Cleanup stuck detection | cleanup.rs:454-496 | No block applied for 120s | Either signal_stuck_fork() or needs_genesis_resync | Yes: directly triggers mechanisms 2 or 5 |

**There is no recovery coordinator.** Each mechanism operates independently with its own triggers, its own state, and its own side effects. The coordination is implicit through shared mutable state (fields on SyncManager).

### Documented feedback loops

```
Loop A (CONFIRMED, was INC-001 primary bug):
  fork_sync success -> reset_sync_for_rollback -> PostRollback
  -> start_sync -> PostRollback branch -> start_fork_sync -> LOOP
  STATUS: Partially fixed by reset_sync_after_successful_reorg.
  SURVIVING PATH: If fork_sync is REJECTED (delta <= 0),
  reset_sync_for_rollback still sets PostRollback, which can
  trigger another fork_sync against a different peer.

Loop B (CONFIRMED, rollback death spiral):
  false fork detection -> resolve_shallow_fork -> rollback_one_block
  -> height decreases -> still behind -> more rollbacks -> height=0
  STATUS: Partially fixed by peak_height tracking (Fix #7).
  SURVIVING PATH: peak_height is only checked during fork_sync
  reorg evaluation, NOT during resolve_shallow_fork rollbacks.

Loop C (CONFIRMED, snap sync cascade):
  snap sync -> imperfect state -> apply failure -> needs_genesis_resync
  -> reset_local_state -> height=0 -> snap sync -> LOOP
  STATUS: Partially fixed by confirmed_height_floor (Fix C).
  SURVIVING PATH: confirmed_height_floor only guards
  reset_local_state(). If the cascade enters via rollback
  instead of reset, the floor is bypassed.

Loop D (POTENTIAL, timeout-driven):
  stuck detection (120s) -> signal_stuck_fork -> fork_sync
  -> fails (peer mismatch) -> Idle -> still behind -> stuck detection (120s)
  -> signal_stuck_fork -> LOOP
  STATUS: Fork sync circuit breaker (3 within 5min) provides
  partial protection. After breaker trips, falls back to header-first
  which may also fail -> reset to Idle -> stuck detection again.
```

**Confidence**: conf(0.92, observed) -- loops A, B, C confirmed by incident history and code tracing. Loop D inferred from code paths.

---

## D4: Boundary Analysis

**Finding**: The boundary between "sync" (SyncManager in crates/network) and "node" (bins/node) is poorly defined. The node layer can bypass sync manager invariants, and both layers write to shared conceptual state without coordination.

### Responsibility confusion

| Responsibility | Where it lives | Problem |
|---------------|----------------|---------|
| Fork detection | **Both**: SyncManager (Layer 9, consecutive_empty_headers, fork_mismatch_detected) AND Node (resolve_shallow_fork, handle_completed_fork_recovery) | SyncManager detects forks via heuristics. Node also detects forks via block_store comparisons. No single authority. |
| Recovery decision | **Both**: SyncManager (cleanup, signal_stuck_fork, needs_genesis_resync) AND Node (fork_recovery.rs, rollback.rs, periodic.rs) | SyncManager recommends actions via flags. Node sometimes follows, sometimes makes its own decisions. |
| Height tracking | **Both**: SyncManager (local_height, confirmed_height_floor) AND Node (chain_state.best_height) | Two sources of truth. Node updates chain_state, then tells SyncManager via update_local_tip/block_applied. If either update fails or is skipped, they diverge. |
| Block store access | **Node only** | SyncManager has no access to block_store but makes decisions (snap sync, fork_sync depth) that depend on block_store contents. When block_store is inconsistent (old fork blocks after snap sync), SyncManager cannot detect this. |

### Node bypassing sync invariants

1. **rollback_one_block()** (rollback.rs:12-186): Updates chain_state and calls `sync.reset_sync_for_rollback()`, but does NOT check `confirmed_height_floor`. A rollback to height 0 bypasses the floor that `reset_local_state()` enforces.

2. **execute_reorg()** (block_handling.rs): Performs rollback + apply through block_store operations, then updates SyncManager. The SyncManager learns about the reorg _after_ it has already happened. It cannot veto.

3. **reset_state_only()** (fork_recovery.rs, now guarded at node layer per commit 2bbc3ee): Was previously callable from any code path to reset to genesis. The guard is a conditional check at the call site, not a type-level enforcement.

**Confidence**: conf(0.88, observed) -- 81 call sites from Node into SyncManager confirmed in requirements doc, responsibilities mapped from code reading.

---

## D5: Timeout Architecture

**Finding**: 10+ independent timeout mechanisms with no coordination. Multiple timeouts can fire in the same tick, creating conflicting recovery actions.

### Timeout inventory

| # | Timeout | Duration | Location | Action on fire | Conflicts with |
|---|---------|----------|----------|---------------|---------------|
| 1 | Request timeout | 30s | cleanup.rs:39 | Remove pending request, free peer | -- |
| 2 | Stale peer timeout | 300s | cleanup.rs:63 | Remove peer entirely | Peer removal can change best_peer(), affecting sync target |
| 3 | Snap root collection | 10s | cleanup.rs:76 | Pick best group or fallback | Can overlap with stuck sync detection |
| 4 | Snap download | 60s | cleanup.rs:121 | Try alternate peer | Can trigger snap_download_error -> fallback -> Idle |
| 5 | Synchronized stall | 5x max_slots_behind | cleanup.rs:140 | Reset to Idle, start_sync | Can restart sync during other recovery |
| 6 | Stuck sync (headers/processing) | 30s | cleanup.rs:166 | Soft retry or hard reset | Can conflict with body download soft retry |
| 7 | Stuck sync (bodies) | 120s | cleanup.rs:162 | Soft retry, then hard reset | Extended timeout, but 3 retries can span 360s |
| 8 | Header blacklist expiry | 30s | cleanup.rs:312 | Re-enable peers for sync | Can re-enable peers mid-recovery |
| 9 | All-peers-blacklisted escalation | 120s | cleanup.rs:326 | Clear blacklist, escalate to snap/fork | Can override ongoing fork_sync |
| 10 | Post-recovery grace | 120s | cleanup.rs:428 | Force-clear grace | Can enable production during incomplete recovery |
| 11 | AwaitingCanonicalBlock | 60s | cleanup.rs:444 | Clear gate | Can enable production before node is stable |
| 12 | Stuck-on-fork detection | 120s | cleanup.rs:473 | signal_stuck_fork or needs_genesis_resync | Can trigger fork_sync during snap sync |
| 13 | Height offset detection | 120s | cleanup.rs:516 | needs_genesis_resync | Can force snap sync during other recovery |
| 14 | Bootstrap grace | 15s | production_gate.rs:223 | Allow genesis production | -- |
| 15 | Gossip activity | 180s | production_gate.rs:581 | Block production | Can block production after valid recovery |
| 16 | Solo production | 50s | production_gate.rs:617 | Block production (circuit breaker) | Can fire during post-snap catch-up |
| 17 | Behind-peers graduated | 60s | production_gate.rs:416 | Allow production despite 4-5 block lag | -- |
| 18 | Peer loss | 30s | production_gate.rs:195 | Resume solo production | Can create solo fork |

**All timeouts are fixed durations, not adaptive to network size.** On a 60-node network:
- Gossip propagation is slower (more hops through mesh)
- Snap sync serving takes longer (more concurrent requests)
- Status polling takes longer (60 peers x 5s each)
- But timeouts remain identical to a 6-node network

**Multiple timeouts can fire in the same cleanup() call.** cleanup() runs every tick (1s for testnet/mainnet). Within a single call:
1. Snap root timeout fires (10s) -> sets state to SnapDownloading
2. Stuck sync timeout fires (30s) -> resets to Idle
3. Both act on the same state variable (`self.state`)
4. Last writer wins, no conflict detection

This is mitigated by the match arms in cleanup() -- snap timeout is checked first, then stuck sync. But the sequential ordering is fragile: adding a new timeout or reordering code changes priority without explicit documentation.

**Confidence**: conf(0.90, observed) -- all 18 timeouts enumerated from code, conflict analysis based on cleanup() execution order.

---

## D6: Failure Mode Analysis

**Finding**: The system's intended failure mode is "retry progressively harder until recovered." The actual failure mode is "retry progressively harder, trigger side effects that cause new failures, spiral to reset."

### Intended failure modes (from comments and code structure)

1. **Transient network issue**: Header-first sync fails -> retry via cleanup stuck detection -> eventually succeeds.
2. **Shallow fork (1-10 blocks)**: Detected via empty headers -> resolve_shallow_fork rolls back -> header-first catches up.
3. **Medium fork (10-1000 blocks)**: Detected via consecutive empty headers or apply failures -> fork_sync binary search -> atomic reorg.
4. **Deep fork (>1000 blocks)**: Detected via many empty headers or fork_sync failure -> snap sync replaces state -> catch up from tip.
5. **Permanent partition**: Gossip watchdog + solo production circuit breaker -> halt production to prevent orphan chain.

### Actual failure modes (from incident history)

1. **The cascade spiral**: Any fork detection (real or false) triggers recovery -> recovery has side effects -> side effects trigger more recovery -> height decreases -> more recovery needed -> height=0 -> snap sync -> imperfect state -> cascade continues. This is the recurring INC-I-005 pattern across 10 sessions.

2. **Timeout-induced oscillation**: Node reaches stable state -> timeout fires (120s stuck detection) -> triggers unnecessary recovery -> destabilizes node -> needs recovery again. Example: Node is Synchronized but 3 blocks behind peers (normal gossip lag). Stall detection at 5x max_slots_behind fires -> resets to Idle -> starts sync -> sync takes >30s -> stuck sync fires -> hard reset. The node was fine; the timeouts created the problem.

3. **Cross-recovery interference**: Fork_sync succeeds and sets recovery_phase=Normal. Cleanup fires in the same tick, sees node is "behind" (because fork_sync reorg dropped height temporarily), triggers signal_stuck_fork(). The success is undone before the node can catch up.

### Gap between intended and actual

| Intended | Actual | Gap |
|----------|--------|-----|
| Progressive escalation: header -> fork_sync -> snap | Each level can re-trigger lower levels via side effects | Escalation is not one-way; it oscillates |
| Recovery converges to stable state | Recovery generates new instability | No convergence guarantee |
| Snap sync is the nuclear option (last resort) | Snap sync creates new problems (block store inconsistency, AwaitingCanonicalBlock) | Nuclear option has its own cascade |
| Timeouts are safety nets for stuck states | Timeouts fire during normal slow operations and destabilize | Fixed timeouts inappropriate for variable-speed operations |

### Does the system fail closed or fail open?

**Neither consistently.** The system oscillates between:
- **Fail closed** (block production to prevent forks) -- production gate has 11+ layers, each a separate blocker
- **Fail open** (keep trying to recover) -- cleanup() continuously triggers new recovery attempts

The oscillation between "block everything" and "try everything" is the cascade. A well-designed system would pick one: either halt and wait for manual intervention, or recover automatically with provable convergence. The current system does both, creating interference.

**Confidence**: conf(0.88, observed) -- failure modes mapped from incident history (4 incidents) and code analysis.

---

## Structural Flaws That Survive Even If All 5 Known Entry Points Are Fixed

These flaws are architectural, not bug-specific. Fixing the 5 known cascade entry points does not address them.

### F1: No State Transition Validator

**28 call sites** to `set_state()` and **18 mutation sites** for `recovery_phase` with no guard. Any code path can set any state from any other state. This means:
- New code added to cleanup() or sync_engine can create new invalid transitions
- Timeout handlers can overwrite transitions made by response handlers in the same tick
- There is no way to test "this transition should be impossible" because all transitions are possible

**Survivability**: conf(0.95, observed). This is structural. No point fix addresses it.

### F2: No Recovery Coordination Protocol

Three recovery systems + cleanup's timeout-driven actions operate independently on shared state. Even if the current feedback loops (A, B, C, D above) are all fixed with specific interlocks, adding any new recovery logic or changing any timeout creates potential for new uncoordinated interactions.

**Survivability**: conf(0.93, observed). The interlock pattern (cooldowns, circuit breakers, recently-held-tips) works for known loops but is reactive, not structural. Each new loop discovered requires a new specific interlock.

### F3: Cleanup() As Implicit Scheduler

cleanup() runs every tick and makes decisions based on timing (how long since X). It is effectively a cron-based scheduler with 13 jobs and no priority system. Any change to tick frequency, cleanup ordering, or timeout values changes the behavior in unpredictable ways. This is the delivery mechanism for most cascade paths.

**Survivability**: conf(0.90, observed). Decomposing cleanup() into named functions (as recommended in REQ-SYNC-004) helps readability but does not fix the fundamental problem of multiple timeout-driven actions competing for state mutations.

### F4: Block Store Inconsistency After Snap Sync

Snap sync replaces ChainState/UtxoSet/ProducerSet but leaves block_store intact. Any code that reads block_store after snap sync can get data from a prior fork. This is a cross-module invariant violation. Fixes #2 and #5 addressed `apply_block`'s prev_slot lookup, but the same class of bug exists in:
- `rollback_one_block()` reads `block_store.get_block_by_height()` to find parent
- `fork_recovery` reads block_store to check connection points
- `block_handling` reads block_store during reorg planning

**Survivability**: conf(0.88, observed). Each new code path that reads block_store is a potential bug. The fix is either (a) clear block_store during snap sync, or (b) mark block_store as "unreliable" after snap sync and force all readers to check.

### F5: Implicit State Space

The effective state of the sync subsystem is not `(SyncState, RecoveryPhase)` but rather `(SyncState, RecoveryPhase, Option<ForkSync>, needs_genesis_resync, fork_mismatch_detected, consecutive_empty_headers, consecutive_apply_failures, consecutive_fork_syncs, confirmed_height_floor, behind_since, stable_gap_since, ...)`. This is at least a 15-dimensional implicit state space. Testing all meaningful combinations is infeasible. New fields added for future fixes expand this space further.

**Survivability**: conf(0.95, observed). This is the deepest structural flaw. Every field added to "fix" a cascade path increases the state space and creates potential for new unforeseen combinations.

---

## Severity Assessment

### Can targeted fixes work?

**No.** conf(0.85, inferred)

The targeted fix strategy has been tried 9 times across 10 sessions. Each fix was correct for its specific bug. The cascade persisted because the architecture generates new cascade paths from the same structural flaws.

The evidence pattern is clear:
- Fix addresses entry point A -> cascade reroutes through B
- Fix addresses entry point B -> cascade reroutes through C
- This will continue because the number of potential cascade paths is combinatorial in the state space

### Is a full rewrite required?

**No.** conf(0.80, inferred)

The subsystem's _components_ are sound (HeaderDownloader, BodyDownloader, ReorgHandler, ForkSync binary search). The problem is the _coordination_ between components. A rewrite of the coordination layer -- the state machine, the recovery arbitration, and the cleanup scheduler -- addresses the structural flaws without rewriting the protocol-level logic.

### Recommended scope of redesign

A **state machine redesign** targeting the coordination layer:

1. **Explicit state machine with validated transitions.** Replace the implicit (SyncState x RecoveryPhase x N flags) space with a single enum where each variant carries all its required data, and transitions are functions that accept the old state and return the new state (or an error for invalid transitions). This collapses F1 and F5.

2. **Single recovery coordinator.** Replace the 3 parallel recovery systems + cleanup's timeout actions with a single arbitration point that receives recovery signals and decides which (at most one) recovery action to take. This addresses F2.

3. **Event-driven cleanup, not tick-driven.** Replace "check every timeout every tick" with a priority queue of scheduled events. When an event fires, it is the ONLY action taken. This addresses F3.

4. **Block store contract after snap sync.** Either clear block_store during snap sync, or add a `store_reliable: bool` flag that all block_store readers must check. This addresses F4.

This is approximately 40-50% of the sync subsystem by line count (~4,000-5,000 lines), touching primarily:
- `manager/mod.rs` (state definitions)
- `manager/cleanup.rs` (timeout coordination)
- `manager/block_lifecycle.rs` (recovery transitions)
- `manager/production_gate.rs` (recovery phase checks)
- `manager/sync_engine.rs` (start_sync decision tree)

The protocol-level components (fork_sync.rs, headers.rs, bodies.rs, reorg.rs, snap_sync.rs) can remain largely unchanged.

---

## Appendix: Metrics Summary

| Metric | Value | Assessment |
|--------|-------|-----------|
| Total sync subsystem lines | ~9,900 | Large, but manageable for incremental redesign |
| SyncManager fields | ~33 (post substruct extraction) | Still too many for a single state machine |
| SyncState variants | 8 | Reasonable count, but transitions unconstrained |
| RecoveryPhase variants | 6 | Reasonable count, but parallel to SyncState creates compound state |
| set_state() call sites | 28 | No transition validation |
| recovery_phase mutation sites | 18 | Scattered across 5 files |
| Independent recovery mechanisms | 3 (+ cleanup + genesis resync + snap sync) | No coordination |
| Independent timeouts | 18 | No adaptive scaling |
| Production gate layers | 11+ | Each layer is an independent blocker |
| Known cascade loops | 4 (A, B, C, D) | Partially addressed by targeted fixes |
| Confirmed incidents from this subsystem | 4 (INC-I-003, INC-I-004, INC-I-005, INC-001) | Recurring pattern |
| Fix attempts on INC-I-005 | 9 across 10 sessions | Each correct but insufficient |

---

## Appendix: Confidence Summary

| Dimension | Finding | Confidence |
|-----------|---------|-----------|
| D1: State Machine Integrity | Ill-defined, unconstrained transitions | conf(0.93, observed) |
| D2: Invariant Coverage | One explicit invariant, partially enforced; 4 assumed invariants violated | conf(0.90, observed) |
| D3: Recovery Architecture | 6 independent mechanisms, 4 confirmed feedback loops | conf(0.92, observed) |
| D4: Boundary Analysis | Tangled responsibilities, node can bypass sync invariants | conf(0.88, observed) |
| D5: Timeout Architecture | 18 independent timeouts, fixed durations, no coordination | conf(0.90, observed) |
| D6: Failure Mode Analysis | System oscillates between fail-closed and fail-open, creating cascade | conf(0.88, observed) |
| Verdict: Architecture problem, not bug cluster | | conf(0.92, observed) |
| Targeted fixes cannot resolve | | conf(0.85, inferred) |
| Full rewrite not required | | conf(0.80, inferred) |
