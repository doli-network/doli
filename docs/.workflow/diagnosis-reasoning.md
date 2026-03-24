# Diagnostic Reasoning Trace: INC-I-005 — FINAL SYSTEMIC ANALYSIS

**Date**: 2026-03-23
**Incident**: INC-I-005 (Session 10 — Why 9 fixes failed)
**Run**: 52

## Prior Evidence Summary (from 9 sessions)

### Constraint Table from Failed Approaches

| # | Fix Attempted | What It Changed | Result | What This Eliminates |
|---|---|---|---|---|
| 1 | SyncManager redesign (eliminated 43x/sec destructive restarts) | sync_engine.rs: idempotent start_sync() | Fixed real bug, cascade continued | Root cause is NOT start_sync() thrashing |
| 2 | chain_state.best_slot fallback after snap sync | apply_block: prev_slot source | Fixed real bug, cascade continued | Root cause is NOT solely prev_slot default |
| 3 | Snap quorum cap at 15 | snap_sync.rs: quorum formula | Fixed real bug, cascade continued | Root cause is NOT unreachable quorum |
| 4 | Force snap sync on apply failures | block_lifecycle.rs: needs_genesis_resync | Fixed real bug, cascade continued | Root cause is NOT the gap < threshold check |
| 5 | Eliminate block store from prev_slot | apply_block: use chain_state instead | Superseded #2, cascade continued | Root cause is NOT block store dependency |
| 6 | False fork detection in production gate | production_gate.rs: removed 1-2 ahead heuristic | CONFIRMED by trace logs, applied to branch | **NOT YET VALIDATED IN PRODUCTION** |
| 7 | Rollback death spiral cap (peak_height tracking) | rollback.rs: MAX_SAFE_ROLLBACK=10 | Defense-in-depth, cascade continued | Root cause is NOT unbounded rollback alone |
| 8 | Disabled automatic snap sync | config: snap threshold = MAX | Broke new node onboarding | Snap sync IS needed — disabling is not viable |
| 9 | Connection capacity increase | network config: max_peers | Fixed bottleneck, cascade continued | Root cause is NOT peer connection limits |

### Critical Observation: Fix #6 IS Applied to Code

I verified that both `update_production_state()` (line 40-48) and `can_produce()` Layer 9 (line 528-546) now only compare hashes at the SAME height. The "1-2 blocks ahead = disagree" heuristic was removed. **This fix is on the current branch but has NOT been validated in production.**

## Explorer Round 1 — Systemic Hypotheses

### H-SYS-1: The cascade is a feedback loop with MULTIPLE independent entry points — conf(0.85, inferred)

Thesis: The cascade persists because fixing one entry point (e.g., false fork detection) does not prevent other entry points (e.g., apply failure, stuck sync, empty headers) from triggering the same downstream cascade. The system has 5+ independent paths into the same destructive spiral.

Falsification: If ALL independent entry points are enumerated and blocked, the cascade stops. If blocking only the false fork detection entry point is sufficient, this hypothesis is wrong (H-SYS-2 would be right).

### H-SYS-2: Fix #6 (false fork detection) IS sufficient but hasn't been deployed — conf(0.40, assumed)

Thesis: The false fork detection was the PRIMARY entry point accounting for 90%+ of cascade instances. The other entry points (apply failure, stuck sync) are secondary and rarely trigger without the false fork detection as the instigator. Deploying Fix #6 alone would resolve the cascade.

Falsification: If the cascade can be triggered WITHOUT false fork detection (e.g., by a clean node joining a 60-node network where no fork exists), then Fix #6 alone is insufficient.

### H-SYS-3: The recovery mechanisms themselves CREATE new cascade entry points — conf(0.75, inferred)

Thesis: Each recovery mechanism (snap sync, rollback, fork sync, genesis resync) has post-recovery state that is imperfect — missing blocks, missing undo data, wrong block store floor, AwaitingCanonicalBlock with no timeout. These imperfections trigger the NEXT cascade iteration through a different entry point than the one that was just fixed.

Falsification: If every recovery mechanism produces a state that is indistinguishable from a clean genesis-synced node, this hypothesis is false.

### H-SYS-4: The 4 unfixed P0/P1 bugs independently maintain the cascade — conf(0.65, inferred)

Thesis: The 4 bugs documented in the evidence (--no-snap-sync deadlock, <= vs < weight check, empty headers blacklisting, best_peer_for_recovery quality) are not just secondary issues. Each one is an independent entry point or cascade amplifier that can sustain the cascade even after Fix #6.

Falsification: If the 4 bugs can be shown to only trigger in scenarios that Fix #6 prevents, they are secondary.

### H-SYS-5: The fundamental architecture — a state machine with no monotonic progress guarantee — ensures any fix creates a new loop — conf(0.60, inferred)

Thesis: The sync/recovery state machine allows backward transitions (Synchronized -> Idle, Normal -> ResyncInProgress) without a monotonic progress counter. This means the system can cycle through the same states indefinitely without making forward progress. Fixes that address individual transitions don't help because the machine finds other backward paths.

Falsification: If a monotonic progress counter (e.g., "highest height reached without resync") is introduced and enforced, the cascade becomes impossible. If the cascade persists even with such a counter, the problem is elsewhere.

## Skeptic Round 1

### H-SYS-1 (Multiple entry points): SURVIVES — conf(0.85, inferred)

Evidence FOR: I traced 6 independent entry points into the cascade (see Phase 3 analysis below). The 9 fixes each addressed at most 1-2 entry points. The system model shows at least 3 entry points that remain unpatched on the current branch.

Evidence AGAINST: None. All 9 failed fixes are consistent with this hypothesis.

### H-SYS-2 (Fix #6 alone sufficient): WEAKENED — conf(0.25, inferred -> inferred)

Evidence AGAINST: The cascade was documented at 60 nodes where the thundering herd problem is minimal. But more critically, I traced 3 scenarios where the cascade starts WITHOUT false fork detection:
1. Snap sync -> AwaitingCanonicalBlock (no timeout) -> cleanup detects "stuck" -> needs_genesis_resync -> new snap sync -> repeat
2. Apply failure from synced blocks -> block_apply_failed() -> 3 failures -> needs_genesis_resync -> snap sync -> post-snap-sync state mismatch -> more apply failures
3. Header-first sync -> empty headers (on a fork after snap sync) -> blacklist all peers -> stuck 120s -> snap sync escalation -> repeat

These three scenarios can trigger without the false fork detection ever firing.

### H-SYS-3 (Recovery creates new entry points): SURVIVES — conf(0.80, inferred)

Evidence FOR: I verified in the code:
- Post-snap-sync: block store has only a seeded canonical index at snap height. No undo data for blocks before snap. Fork sync binary search hits store floor -> `fork_sync_store_limited()` -> unclear recovery.
- Post-snap-sync: `AwaitingCanonicalBlock` has NO timeout. If the first gossip block doesn't build on tip (it was produced between snap sync and the gossip arriving), node is stuck forever.
- Post-rollback: `RecoveryPhase::PostRollback` triggers fork_sync, but if fork_sync finds equal-weight chains and is rejected (correctly), the node enters rollback cooldown -> header-first sync -> empty headers -> more rollbacks -> death spiral (capped at 10 by Fix #7, then escalated to snap sync -> back to square 1).
- Post-genesis-resync: `reset_state_only()` clears UTXO and ProducerSet but preserves block data. But block store indexes are cleared (LAYER 9.5). If the preserved block data was from a fork, the next sync replays those fork blocks -> apply failure -> cascade.

Evidence AGAINST: None found. Every recovery path has at least one imperfection that can trigger the next cascade.

### H-SYS-4 (4 unfixed bugs): WEAKENED — conf(0.45, inferred)

Evidence FOR: The `<=` vs `<` weight check (block_handling.rs:568) uses `<` which is CORRECT for strict-lighter rejection. The code at line 609 handles equal weight correctly with ping-pong prevention. So the P0 "weight check uses `<=` instead of `<`" bug does NOT exist in the current code — it was likely already fixed.

The `--no-snap-sync` deadlock IS real (production_gate.rs:1064-1080 now allows the signal through), so it was already addressed on this branch.

The empty headers blacklisting IS real — first 1-2 empties blacklist the responding peer (sync_engine.rs:797-799), reducing the pool of recovery peers.

`best_peer_for_recovery()` does have a quality threshold — it requires peers within 10 blocks of network_tip_height (mod.rs:1142). So the P1 bug about "no quality threshold" does NOT exist in the current code.

Revised: Only the empty headers blacklisting remains as a genuine unfixed amplifier.

### H-SYS-5 (No monotonic progress): SURVIVES — conf(0.70, inferred)

Evidence FOR: I traced the following backward transitions in the code:
- `Synchronized -> Idle` (cleanup.rs:146 — stall detection)
- `DownloadingHeaders/Bodies -> Idle` (cleanup.rs:270,285 — stuck sync reset)
- `RecoveryPhase::Normal -> ResyncInProgress` (via force_recover_from_peers)
- Height resets to 0 via `reset_state_only()` (fork_recovery.rs:593-609)
- Height decreases via `rollback_one_block()` (rollback.rs:12-186)

None of these transitions enforce a "don't go below the high-water mark" invariant. The peak_height tracking (Fix #7) only caps rollback depth at 10, but snap sync resets height to 0 without any similar constraint.

## Explorer Round 2 — Refined (Entry Point Enumeration)

I identified 6 independent entry points into the cascade loop:

### EP-1: False fork detection (PATCHED by Fix #6 on this branch)
Path: `update_production_state()` -> `fork_mismatch_detected=true` -> `BlockedChainMismatch` -> `try_trigger_fork_recovery()` -> `maybe_auto_resync()` -> `force_recover_from_peers()` -> snap sync -> post-snap-sync state gap -> EP-2 or EP-3

### EP-2: Post-snap-sync AwaitingCanonicalBlock stuck (UNPATCHED)
Path: Snap sync completes -> `AwaitingCanonicalBlock` (no timeout) -> cleanup sees "stuck" in some sync state -> various timeout-driven escalations -> BUT AwaitingCanonicalBlock is checked BEFORE Layer 3 (syncing) in can_produce(), so production is blocked forever if no canonical gossip block arrives

Why this can happen: After snap sync, the node is at height H. The network is at height H+N. The first gossip block that arrives is for height H+N+1, which builds on hash(H+N), not hash(H). So `block.header.prev_hash != state.best_hash` (block_handling.rs:28). The block is cached as orphan, NOT applied. `clear_awaiting_canonical_block()` (line 169) is never called because it's only called after `apply_block` succeeds on a gossip block.

The node must first sync from H to H+N via header-first sync (which DOES work — it applies blocks and advances height). But header-first sync calls `apply_block` with `ValidationMode::Light`, and `clear_awaiting_canonical_block` is NOT called after synced blocks — only after gossip blocks (line 164-169 in block_handling.rs, after the gossip path succeeds).

Wait — let me verify this more carefully.

### EP-3: Apply failure cascade (PARTIALLY PATCHED by Fix #4)
Path: Synced block fails apply (validation error, state mismatch) -> `block_apply_failed()` -> after 3 failures, `needs_genesis_resync=true` -> `force_recover_from_peers()` -> snap sync -> height reset -> try to sync again -> if the same blocks fail again -> same cascade

### EP-4: Empty headers escalation (PARTIALLY PATCHED)
Path: Header-first sync -> peer returns empty headers (our tip is on fork) -> blacklist peer -> eventually ALL peers blacklisted -> stuck 120s -> if gap>12, escalate to snap sync -> snap sync -> post-snap-sync state -> new empty headers -> repeat

### EP-5: Stuck sync timeout escalation (PARTIALLY PATCHED)
Path: Sync stuck for 30s (stuck_threshold in cleanup.rs:162-166) -> hard reset to Idle -> restart sync -> if blocks can't be applied (fork) -> stuck again -> after 120s (cleanup.rs:457), force snap sync or fork_sync -> cascade

### EP-6: Height offset detection false positive (UNPATCHED)
Path: Node is syncing (blocks being applied) with gap >= 2 for >120s -> cleanup.rs:500 triggers `needs_genesis_resync=true` -> snap sync -> but the node was making genuine progress! The gap was stable because both the node AND the network were advancing. This is correct behavior, not a bug, but the cleanup heuristic can't distinguish "stable gap during active sync" from "height offset from bad reorg."

## Skeptic Round 2

### EP-1 (False fork detection): PATCHED — conf(0.90, measured)
Code verified. Both `update_production_state()` and `can_produce()` now only compare at same height. But NOT validated in production with 60+ nodes.

### EP-2 (AwaitingCanonicalBlock stuck): UNPATCHED, REAL — conf(0.80, inferred)
I need to verify whether synced blocks (from header-first sync in periodic.rs:104-108) also clear this flag. Let me check.

Looking at periodic.rs:101-111: Synced blocks go through `self.apply_block(block, ValidationMode::Light)`, which goes through `apply_block/mod.rs`. After apply, `block_applied_with_weight()` is called. But `clear_awaiting_canonical_block()` is ONLY called in `block_handling.rs:169` after a GOSSIP block succeeds.

However, `block_applied_with_weight()` in block_lifecycle.rs:73-87 handles the `PostRecoveryGrace` phase and clears it after 10 blocks. But `AwaitingCanonicalBlock` is a DIFFERENT phase — it's NOT `PostRecoveryGrace`.

CONFIRMED: `AwaitingCanonicalBlock` is ONLY cleared by `clear_awaiting_canonical_block()` which is ONLY called when a gossip block is applied successfully (block_handling.rs:164-169). Header-first synced blocks do NOT clear it.

BUT WAIT: `AwaitingCanonicalBlock` only blocks PRODUCTION (production_gate.rs:146-151), not sync. So the node can still sync via header-first or gossip. The question is whether the node can receive a gossip block that builds on its tip WHILE it is also syncing via header-first.

If the node is syncing via header-first (state=DownloadingHeaders/Bodies/Processing), gossip blocks can still arrive. But they won't build on the node's tip because the node is behind. Eventually the node catches up via sync, reaches the tip, and then the next gossip block WILL build on its tip -> apply_block succeeds -> clear_awaiting_canonical_block.

REVISED: EP-2 is only a problem if the node can't catch up to the network tip via sync. If sync works, AwaitingCanonicalBlock eventually clears when a gossip block is applied. If sync is broken (e.g., because of apply failures or empty headers), then AwaitingCanonicalBlock is permanently stuck.

EP-2 DOWNGRADED — conf(0.45, inferred). It's a secondary failure mode, not a primary entry point. It amplifies other failures (EP-3, EP-4) by preventing production recovery even after sync succeeds.

### EP-3 (Apply failure cascade): PARTIALLY PATCHED — conf(0.70, inferred)
Fix #4 ensures apply failures trigger snap sync even when gap < threshold. But the root question is: WHY do apply failures happen after snap sync? If the snap sync snapshot is from the canonical chain (verified by state root), the next blocks should apply correctly.

The answer is in the validation mode: blocks are applied with `ValidationMode::Light` during sync (periodic.rs:105). Light validation skips some checks. If the block store doesn't have the parent block (post-snap-sync), certain lookups may fail.

This was supposedly fixed by Fix #5 (eliminate block store from prev_slot). But there may be OTHER block store lookups that fail post-snap-sync. Without reading the full apply_block code, I can't be certain, but the pattern of "snap sync -> apply fails" suggests the block store dependency wasn't fully eliminated.

### EP-4 (Empty headers escalation): PARTIALLY PATCHED — conf(0.60, inferred)
The escalation from empty headers to snap sync is aggressive but correct when the node IS on a fork. The problem is when it triggers for a node that ISN'T on a fork — e.g., when a node just finished snap sync and tries header-first sync but the peer's chain has advanced past the snap point.

### EP-5 (Stuck sync timeout): PARTIALLY PATCHED — conf(0.50, inferred)
The 30s/120s stuck thresholds are heuristic. On a loaded network (113 nodes), a seed serving 50+ simultaneous syncs can be legitimately slow. A 30s timeout is too aggressive — it triggers false "stuck" detection that cascades into snap sync.

### EP-6 (Height offset false positive): LOW RISK — conf(0.30, inferred)
The 120s timeout and gap >=2 requirement make false positives unlikely during normal operation. Downgraded.

## Analogist

This failure pattern is well-known in distributed systems as **"recovery-induced cascading failure"** or the **"thundering herd recovery problem."** The canonical example is a system where:

1. A component fails
2. The recovery mechanism kicks in
3. The recovery mechanism itself is imperfect (slow, resource-intensive, or partially correct)
4. The imperfect recovery triggers a new failure
5. Which triggers a new recovery
6. The system oscillates between failure and imperfect recovery indefinitely

**Known solutions from distributed systems:**

1. **Jitter + backoff on recovery actions** (Netflix/Amazon pattern): Every recovery action adds random jitter to prevent synchronization. DOLI has backoff but no jitter.

2. **Circuit breakers with monotonic state** (Hystrix pattern): Once a component enters "open" (failed) state, it can only transition to "half-open" (testing) and then "closed" (healthy) — never back to "open" without passing through "closed" first. DOLI's state machine allows Synchronized -> Idle -> snap sync -> Synchronized -> Idle in a tight loop.

3. **Graduated recovery** (Kubernetes pattern): First try the cheapest recovery (restart pod). If that fails, try the next level (reschedule to different node). Never skip levels. DOLI does have graduated recovery (rollback -> fork_sync -> snap sync) but the escalation is timer-driven (30s, 120s) rather than evidence-driven.

4. **Split-brain resolution via epoch numbers** (Raft/Paxos pattern): A node that has gone through recovery is in a higher epoch and accepts that it may need to discard its state. DOLI tracks `consecutive_resync_count` but doesn't use it to fundamentally change behavior.

The most relevant pattern is **Kubernetes's "CrashLoopBackOff"**: when a pod keeps crashing, Kubernetes doesn't keep restarting it at full speed. It backs off exponentially AND caps the backoff. The pod must demonstrate stability (running for X seconds) before the backoff resets. DOLI's `blocks_since_resync_completed >= 5` reset (block_lifecycle.rs:62) is analogous but the threshold is too low — 5 blocks in 50 seconds is not "stable."

## Final Hypothesis Ranking

1. **H-SYS-1 + H-SYS-3: Multiple entry points + recovery creates new entry points** — conf(0.90, inferred)
   The cascade persists because the system has 4-5 entry points, fixes address 1-2 at a time, and each recovery creates post-recovery state that feeds into a different entry point.

2. **H-SYS-5: No monotonic progress guarantee** — conf(0.75, inferred)
   The state machine allows arbitrary backward transitions. Without a "high-water mark" invariant, the system can cycle indefinitely.

3. **H-SYS-2: Fix #6 alone sufficient** — conf(0.25, assumed)
   Possible but unlikely. The cascade has been observed in scenarios where false fork detection is not the trigger (post-snap-sync apply failures, stuck sync timeouts).
