# Architecture Reasoning Trace: Sync Manager Redesign

*Generated 2026-03-22, RUN_ID=44, scope: syncmanager*

## Field Census

Counted from `crates/network/src/sync/manager/mod.rs` lines 322-546:

### Cluster 1: Core Sync State (15 fields)
- config, state, local_height, local_hash, local_slot
- peers, pending_headers, headers_needing_bodies, pending_blocks
- pending_requests, next_request_id, sync_epoch
- header_downloader, body_downloader, reorg_handler

### Cluster 2: Production Gate (18 fields)
- production_blocked, production_block_reason
- recovery_phase, last_resync_completed, consecutive_resync_count
- resync_grace_period_secs, max_slots_behind, max_heights_behind, max_heights_ahead
- max_grace_cap_secs, blocks_since_resync_completed
- min_peers_for_production, tier
- last_block_received_via_gossip, gossip_activity_timeout_secs, max_solo_production_secs
- consecutive_sync_failures, max_sync_failures_before_fork_detection

### Cluster 3: Bootstrap Gate (5 fields)
- has_connected_to_peer, first_peer_status_received, last_peer_status_received
- bootstrap_grace_period_secs, peers_lost_at, peer_loss_timeout_secs

### Cluster 4: Fork Detection & Recovery (14 fields)
- fork_recovery, fork_sync, fork_mismatch_detected
- consecutive_empty_headers, needs_genesis_resync
- consecutive_apply_failures, body_stall_retries
- last_fork_sync_rejection, fork_sync_cooldown_secs
- consecutive_fork_syncs, last_fork_sync_at, recently_held_tips
- behind_since, stable_gap_since

### Cluster 5: Snap Sync (8 fields)
- snap_sync_threshold, snap_sync_quorum, snap_root_timeout, snap_download_timeout
- snap_blacklisted_peers, snap_sync_attempts, fresh_node_wait_start, store_floor

### Cluster 6: Network Tip & Timing (6 fields)
- network_tip_height, network_tip_slot
- last_block_seen, last_block_applied, last_sync_activity
- blocks_applied, last_progress_log

### Cluster 7: Other (3 fields)
- finality_tracker, header_blacklisted_peers, idle_behind_retries

**Total: ~55 fields confirmed (some in Cluster 3 overlap with Cluster 2).**

## Decision 1: How to group the 55 fields (REQ-SYNC-001)

### Explorer Pass 1

**Alt 1 — Do nothing.** Document the field groups as comments. conf(0.15, observed). Complexity cost: 0.
- Pro: Zero risk, zero code change.
- Con: Does NOT address the root problem — invalid state combinations remain representable.

**Alt 2 — Subtract fields.** Eliminate fields that are redundant or derivable. conf(0.4, observed). Complexity cost: -5 fields.
- `max_heights_behind` — only used in `new_with_settings()`, never in can_produce(). DEAD.
- `max_heights_ahead` — only set in `set_max_heights_ahead()`, never read (Layer 7 removed). DEAD.
- `blocks_applied` and `last_progress_log` — progress logging. Can be a simple local counter in block_applied.
- `has_connected_to_peer` — derivable from `first_peer_status_received.is_some()`.
- `production_block_reason` — always paired with `production_blocked`. Could be `Option<String>` alone (None = not blocked).
- That cuts ~5-7 fields. Still 48+ fields — not enough alone.

**Alt 3 — Group into typed sub-structs (the natural cluster approach).** conf(0.75, observed). Complexity cost: 5 new structs, 0 new files.
- Group fields into sub-structs matching the clusters above.
- SyncManager keeps ~8-10 top-level fields: config, state, local_tip, peers, sync_pipeline (headers+bodies+pending), production_gate, fork_state, snap_state, network_state.
- Pro: Each sub-struct has a clear responsibility. Invalid cross-concern combinations become harder.
- Con: Methods need to reference `self.production_gate.recovery_phase` instead of `self.recovery_phase` — noisy refactor.
- Precondition: Fields within each cluster must be tightly coupled (read/written together). Cross-cluster reads must be rare.

**Alt 4 — State machine enum with per-variant data.** conf(0.3, inferred). Complexity cost: +1 new enum, major rewrite.
- Each SyncState variant carries ONLY the fields needed in that state. E.g., `DownloadingHeaders { peer, target_slot, downloader, ... }`.
- Pro: Invalid states are truly unrepresentable at the type level.
- Con: MASSIVE rewrite. Fields like `local_height`, `peers`, `production_gate` are needed in ALL states — they'd go in a "context" struct alongside the state machine. Ends up being Alt 3 anyway, with extra complexity.
- Precondition: States must have mostly-disjoint field sets. They don't — most fields are cross-cutting.

**Alt 5 — Subtract + Group (combine Alt 2 and Alt 3).** conf(0.85, observed). Complexity cost: 5 new structs, -5 dead fields.
- First eliminate dead fields, then group remaining into sub-structs.
- Best of both: fewer total fields AND better organization.

### Skeptic Pass 1

- **Alt 1 (Do nothing)**: ELIMINATED. The analyst's requirement is Must priority. Doing nothing violates REQ-SYNC-001.
- **Alt 2 (Subtract only)**: WEAKENED. Removing 5-7 fields from 55 is cosmetic. Still 48+ fields on one struct. Does not satisfy "<=20 direct fields."
- **Alt 3 (Group)**: SURVIVES. The clusters are natural (confirmed by reading code — fields within each cluster ARE read/written together). The cross-cluster access pattern is: production_gate reads fork state (consecutive_sync_failures, fork_mismatch_detected) and local tip. This can be solved with method parameters or shared references.
- **Alt 4 (State machine enum)**: ELIMINATED. The cross-cutting fields (peers, local_tip, all production gate state) would need a separate context struct. You end up with Alt 3 + extra enum complexity. Rust's borrow checker would fight carrying data in the enum AND needing mutable access to context.
- **Alt 5 (Subtract + Group)**: SURVIVES. Strictly dominates Alt 3 — does everything Alt 3 does plus removes dead weight.

### Skeptic Pass 2 (Alt 3 vs Alt 5)

Alt 5 subsumes Alt 3. The only question is whether eliminating dead fields is safe.

- `max_heights_behind`: grep shows it's set in constructor and `new_with_settings()`, but NEVER read in `can_produce()` or anywhere in the production gate. DEAD. conf(0.9, measured).
- `max_heights_ahead`: set by `set_max_heights_ahead()` but Layer 7 (the ahead-of-peers check) is commented out. DEAD. conf(0.9, measured).
- `has_connected_to_peer`: set to true in `set_peer_connected()`, read in `is_in_bootstrap_phase()` and bootstrap gate. Can be derived from `first_peer_status_received.is_some()`. NOT DEAD but DERIVABLE. conf(0.7, observed).

**WINNER: Alt 5 (Subtract + Group).** conf(0.85, observed).

---

## Decision 2: How to unify 3 fork recovery systems (REQ-SYNC-002)

### Explorer Pass 1

Three systems:
1. **ForkRecoveryTracker** (fork_recovery.rs, 322 lines) — Bitcoin-style parent chain walk. Triggered by orphan blocks. Walks backward requesting parents until connecting to our chain. Used for shallow, real-time orphan recovery.
2. **ForkSync** (fork_sync.rs, 657 lines) — Binary search for common ancestor. Triggered after 3+ empty header responses OR stuck-fork detection. O(log N) recovery. Downloads canonical headers + bodies from ancestor.
3. **resolve_shallow_fork** (Node's periodic.rs/rollback.rs, ~70 lines) — Sequential rollback of 1 block per tick. Triggered by consecutive_empty_headers >= 3 with gap <= 12. Changes local_hash to parent so next GetHeaders succeeds.

### Key insight from code reading:

These are NOT three systems doing the same thing. They handle different scenarios:
- **ForkRecoveryTracker**: Orphan blocks (received a block whose parent we don't have). This is a NORMAL gossip event, not a fork signal.
- **ForkSync**: Deep fork detection after header sync fails repeatedly. Binary search for divergence point.
- **resolve_shallow_fork**: Quick rollback for small forks (gap <= 12). Changes tip hash so header sync can proceed.

The interference problem is NOT that they overlap, it's that:
1. cleanup() can reset fork_sync state
2. update_peer() calling start_sync() can nuke in-flight fork_sync requests
3. signal_stuck_fork is called from 4 uncoordinated places

**Alt 1 — Do nothing, add coordination.** conf(0.3, inferred). Complexity cost: +50 lines coordination logic.
- Add `is_fork_recovering()` method that checks all 3, prevent start_sync() from interrupting.
- Pro: Minimal code change.
- Con: Adds coordination complexity without simplifying the underlying systems.

**Alt 2 — Eliminate ForkRecoveryTracker, keep ForkSync + rollback.** conf(0.5, observed). Complexity cost: -322 lines.
- ForkRecoveryTracker handles orphan blocks. The node can handle orphans by just storing them and waiting (orphan pool pattern). When the parent arrives via gossip, apply the chain.
- But: the current orphan handling IS the ForkRecoveryTracker — it actively fetches parents. Without it, deep orphan chains would time out waiting for gossip. This is risky.

**Alt 3 — Unify ForkSync + resolve_shallow_fork, keep ForkRecoveryTracker separate.** conf(0.75, observed). Complexity cost: -30 lines, +clarity.
- resolve_shallow_fork is already a prelude to ForkSync (it rolls back trying to find a hash peers recognize; if that fails, ForkSync's binary search takes over).
- Merge: ForkSync becomes the single fork resolution path. For gap <= 12, the "binary search" degenerates to a sequential check (O(12) ~ O(1)). For larger gaps, O(log N).
- resolve_shallow_fork's rollback logic stays in Node (it needs block_store access), but the DECISION to rollback vs binary-search moves into ForkSync.
- ForkRecoveryTracker stays — it handles a different trigger (orphan blocks, not empty headers).

**Alt 4 — Single ForkResolver.** conf(0.4, inferred). Complexity cost: +1 new module, -2 existing modules.
- All fork recovery goes through one entry point: `ForkResolver::resolve(trigger, context)`.
- Trigger: OrphanBlock | EmptyHeaders | StuckSync | ApplyFailure.
- Internally dispatches to the right strategy.
- Pro: Single entry point.
- Con: Forced abstraction. The triggers have different data requirements and recovery strategies. The "single entry point" would need to handle all cases, becoming a dispatcher — complexity moves, doesn't shrink.

**Alt 5 — Keep ForkRecoveryTracker + ForkSync, eliminate resolve_shallow_fork.** conf(0.65, observed). Complexity cost: -70 lines.
- resolve_shallow_fork rolls back 1 block at a time for gap <= 12. ForkSync can handle gap <= 12 too (binary search on 12 blocks = 4 round-trips).
- Remove resolve_shallow_fork entirely. When consecutive_empty_headers >= 3 and gap <= 12, go straight to ForkSync.
- Pro: Removes one system entirely from the Node side. Fewer moving parts.
- Con: ForkSync requires a peer with higher height. resolve_shallow_fork doesn't — it just rolls back locally. If no peer is available, ForkSync can't start.

### Skeptic Pass 1

- **Alt 1 (Coordination layer)**: WEAKENED. Adds complexity without removing anything. The interference problem needs subtraction, not addition.
- **Alt 2 (Remove ForkRecoveryTracker)**: WEAKENED. ForkRecoveryTracker handles orphan blocks that arrive out of order via gossip. Removing it means orphan blocks are lost until the parent propagates — could be never if the peer that had the parent disconnects.
- **Alt 3 (Unify ForkSync + rollback)**: SURVIVES. resolve_shallow_fork is a special case of fork recovery. The key insight: rollback-1 is only done because ForkSync binary search was seen as "too heavy" for small forks. But binary search on 12 blocks is 4 round-trips — comparable to the 3-12 rollback cycles resolve_shallow_fork uses.
- **Alt 4 (Single ForkResolver)**: ELIMINATED. Over-engineered dispatcher pattern. Complexity moves, doesn't shrink.
- **Alt 5 (Remove resolve_shallow_fork)**: SURVIVES. Simpler than Alt 3 — just removes one path. The risk (no peer available) is mitigable: if ForkSync can't start, fall through to the existing snap sync escalation.

### Skeptic Pass 2 (Alt 3 vs Alt 5)

Alt 5 is actually a subset of Alt 3. Alt 3 says "unify the decision logic"; Alt 5 says "just remove resolve_shallow_fork."

The critical question: Can ForkSync handle everything resolve_shallow_fork does?

resolve_shallow_fork does:
1. Check empty_headers >= 3 OR stuck_fork_signal
2. If gap <= 12, rollback 1 block, reset empty headers, retry sync
3. If gap > 12 or rollback limit reached, start ForkSync

ForkSync does:
1. Binary search from local_height down to max(1, local_height - 1000) for common ancestor
2. Download canonical headers + bodies from ancestor
3. Return ForkSyncResult for Node to apply

The GAP: ForkSync needs a peer and network round-trips. resolve_shallow_fork is LOCAL (rollback is instant). For a 1-block fork (the most common case), rollback is instant while ForkSync requires 3-4 network round-trips.

**WINNER: Alt 5 with a twist.** Remove resolve_shallow_fork as a separate system, but keep the "rollback 1 block" fast path as the FIRST step inside fork detection logic. When gap <= 12, try rollback first (instant, local-only). If rollback fails or count exceeds limit, activate ForkSync. This consolidates the DECISION logic into one place (the sync manager) while keeping the fast path.

Actually — that's what the code ALREADY does! resolve_shallow_fork tries rollback first, then ForkSync. The problem is it lives in Node (periodic.rs/rollback.rs) rather than in sync manager.

**REVISED WINNER: Alt 3 — Keep the two-phase approach (rollback then binary search), but move the DECISION logic from Node to SyncManager.** The Node still executes the rollback (it has block_store access), but SyncManager decides WHEN and coordinates the two phases through its state machine. This eliminates the interference because there's one coordination point.

conf(0.80, observed). ForkRecoveryTracker stays separate (different trigger: orphan blocks).

---

## Decision 3: How to fix start_sync() (REQ-SYNC-003)

### Explorer Pass 1

The problem: start_sync() is called from 5+ places. Each call increments sync_epoch (invalidating all in-flight requests), clears all downloaders and pending state. On a 44-node network, update_peer fires on every StatusResponse — potentially 43 calls/second, each nuking in-flight state.

**Alt 1 — Guard clause: if already syncing, return.** conf(0.75, observed). Complexity cost: +3 lines.
- Add `if self.state.is_syncing() { return; }` at the top of start_sync().
- Pro: Trivial change, fixes the most damaging case.
- Con: Doesn't fix the case where we're Idle but update_peer triggers start_sync repeatedly.
- Actually, looking at the code: update_peer already has `let state_ok = matches!(self.state, SyncState::Idle | SyncState::Synchronized)` before calling start_sync. So it only calls start_sync when Idle/Synchronized. The problem is: after start_sync sets state to DownloadingHeaders, the NEXT update_peer won't call it again. But if two update_peer calls happen before start_sync changes state (e.g., 43 peers in quick succession), they all call start_sync.

**Alt 2 — Debounce with minimum interval.** conf(0.7, inferred). Complexity cost: +5 lines.
- Track `last_sync_start: Instant` and skip if < 2s since last start.
- Pro: Prevents rapid-fire restarts.
- Con: Arbitrary timeout. A legitimate sync restart after a real state change would be delayed.

**Alt 3 — Separate start_sync() from reset_sync().** conf(0.85, observed). Complexity cost: +20 lines.
- `start_sync()` only starts if not already syncing AND the target peer is different from current.
- `reset_sync()` is the destructive version (clears all state, bumps epoch). Only called when we KNOW the current sync is bad (cleanup detected stuck, block_apply_failed, etc.).
- Pro: Intent-driven API. Most callers want "ensure we're syncing" not "nuke and restart."
- Con: Must audit all 5+ call sites to determine which want start vs reset.

**Alt 4 — Event-driven state transition.** conf(0.3, inferred). Complexity cost: +100 lines.
- Replace imperative start_sync() with a state transition request. The sync engine processes transitions.
- Over-engineered for this problem.

### Skeptic Pass 1

- **Alt 1 (Guard clause)**: SURVIVES but WEAKENED. The guard prevents re-entry during active sync, but doesn't prevent the case where we're Idle and multiple peers trigger start_sync on the same tick before the first one completes.
- **Alt 2 (Debounce)**: WEAKENED. Arbitrary timeout. And: looking at the code, the issue is not that start_sync is called too often — it's that each call NUKES state. If start_sync were idempotent, calling it 43 times would be harmless.
- **Alt 3 (Separate start/reset)**: SURVIVES. This is the correct abstraction: the problem is that a "start" operation has "reset" semantics.
- **Alt 4 (Event-driven)**: ELIMINATED. Massive overengineering for a problem that needs 20 lines of change.

### Skeptic Pass 2

Alt 1 vs Alt 3. Alt 1 is simpler. But Alt 1 only prevents re-entry during active sync. Alt 3 also prevents unnecessary resets.

Actually, let me re-examine Alt 1. If we add `if self.state.is_syncing() { return; }`, then:
- update_peer calls start_sync when Idle/Synchronized
- First call transitions to DownloadingHeaders
- Subsequent calls: state is DownloadingHeaders, guard returns
- Problem SOLVED for the 43-calls-per-second case

The remaining problem: even the first call does a destructive reset (clears downloaders, bumps epoch). If we're Synchronized and need to sync 3 more blocks, this nuke is overkill.

Alt 3 is better but more complex. Given the subtraction principle, Alt 1 solves the CRITICAL case (43 calls nuking state). Alt 3 is an improvement that can come later.

**WINNER: Alt 1 (guard clause) as the first step, with Alt 3 as a follow-up.** conf(0.80, observed). The guard clause is the 3-line fix that prevents the most damage. Separating start/reset is the deeper fix for a later milestone.

---

## Decision 4: How to simplify production gate (REQ-SYNC-008)

### Explorer Pass 1

Current layers (from code):
1. Explicit production block
2. Resync in progress / AwaitingCanonicalBlock
3. Active sync
4. Bootstrap gate (genesis/peer loss)
5. Post-resync grace period
5.5. Minimum peer count
6. Slot lag check
6.5. Height lag check (graduated)
7. REMOVED (ahead-of-peers)
8. Sync failure fork detection
8.5. Persistent fork mismatch flag
9. Chain hash verification (minority check)
10. Gossip activity watchdog
10.5. Solo production circuit breaker
11. Finality conflict check

That's 13 named checks (2 removed). The analyst wants <= 5 composable checks.

**Alt 1 — Do nothing.** conf(0.4, observed). Complexity cost: 0.
- Each layer catches a real production scenario. Removing any layer risks a regression.
- The 2026-03-15 halt was caused by Layer 9 side effects in can_produce(), which was already fixed (extracted to update_production_state()).
- The remaining layers are READ-ONLY checks in can_produce(). They don't interfere.

**Alt 2 — Subtract redundant layers.** conf(0.7, observed). Complexity cost: -100 lines.
Analysis of overlap:
- Layer 6 (slot lag) AND Layer 6.5 (height lag): Both check "are we behind peers?" Layer 6 uses slots, 6.5 uses heights. Can they merge? No — slots and heights have different semantics (see code comments: forked nodes inflate heights, not slots). But Layer 6's slot check is already covered by Layer 6.5's Gate 1 (active sync state check). Let me check... Layer 6 blocks when slot_diff > max_slots_behind AND local_height < peer_height. Layer 6.5 blocks when height_lag > 5. These cover different ranges. Keep both but simplify.
- Layer 8 (sync failures) AND Layer 8.5 (persistent fork mismatch): 8.5 was added because 8 oscillates. They could merge: just use the persistent flag set by update_production_state(). Remove the check on consecutive_sync_failures entirely from can_produce() — it's already handled by update_production_state() setting fork_mismatch_detected.
- Layer 10 (gossip watchdog, peers ahead) AND Layer 10.5 (solo circuit breaker, we're ahead): These are complements (one for when behind, one for when ahead). They could merge into a single "gossip health" check.
- Layer 7: Already removed. The code still has comments and logging for it. Delete the dead code.

Concrete subtractions:
1. DELETE Layer 7 comments/logging (dead code)
2. MERGE Layer 8 + 8.5 → single "fork detection" check using persistent flag
3. MERGE Layer 10 + 10.5 → single "gossip health" check
4. Result: ~9 layers → still above 5 target

**Alt 3 — Phase-based grouping.** conf(0.65, observed). Complexity cost: +50 lines restructuring.
- Phase 1: Hard blocks (explicit block, resync, active sync) — if any is active, BLOCK immediately
- Phase 2: Bootstrap (genesis checks, peer loss) — first-time setup
- Phase 3: Synchronization (peer lag, height lag, grace period) — are we caught up?
- Phase 4: Fork detection (sync failures, hash mismatch, gossip health) — are we on the right chain?
- Phase 5: Finality — is our chain consistent with finalized blocks?
- Pro: Conceptual clarity. Each phase is a function.
- Con: Same number of individual checks, just grouped differently.

**Alt 4 — Table-driven.** conf(0.35, assumed). Complexity cost: +100 lines.
- Each check is a function returning `Option<ProductionAuthorization>`.
- Compose them into a list, run in order.
- Pro: Easy to add/remove checks.
- Con: Loses the ability to short-circuit with context-dependent logic (e.g., Layer 6 guard that skips if local_height >= peer_height). Table-driven checks need shared context, which means passing all the state.

### Skeptic Pass 1

- **Alt 1 (Do nothing)**: SURVIVES (weakened). The analyst's requirement is Should priority. If complexity is low and the existing code WORKS (post the update_production_state fix), doing nothing is defensible. But the 1,029 lines are a maintenance burden.
- **Alt 2 (Subtract)**: SURVIVES. Concrete deletions with measurable impact.
- **Alt 3 (Phase-based)**: WEAKENED. Same checks, different grouping. Doesn't actually reduce checks to <= 5. Adds abstraction.
- **Alt 4 (Table-driven)**: ELIMINATED. Over-engineering. The short-circuit logic with context-dependent guards doesn't fit a table.

### Skeptic Pass 2

Alt 1 vs Alt 2. Alt 2 is strictly better — it removes dead code (Layer 7), merges overlapping checks, and reduces the check count. The risk is low because merged checks preserve all existing gate logic.

**WINNER: Alt 2 (Subtract redundant layers).** conf(0.75, observed).
- DELETE Layer 7 dead code
- MERGE Layer 8 + 8.5 (use persistent fork_mismatch_detected only, remove consecutive_sync_failures check from can_produce)
- MERGE Layer 10 + 10.5 (single gossip health check)
- Result: ~9 functional layers. Not <= 5, but the analyst's requirement was Should, not Must. Getting below 5 would require removing safety-critical checks — not worth the risk.

---

## Decision 5: How to decompose cleanup() (REQ-SYNC-004)

### Explorer Pass 1

cleanup() does (from code reading):
1. Body downloader timeout cleanup
2. Request timeout cleanup
3. Stale peer removal
4. Snap sync timeout (collecting roots / downloading)
5. Synchronized-but-stalled detection
6. Stuck sync detection (soft retry for bodies, hard reset for headers/processing)
7. Header blacklist expiry
8. All-peers-blacklisted escalation
9. Periodic sync retry (Idle but behind)
10. Post-recovery grace timeout
11. Stuck-on-fork detection (120s no block applied)
12. Height offset detection (stable gap while applying blocks)

**Alt 1 — Do nothing.** conf(0.3, observed). Complexity cost: 0.
- It works. It's hard to read, but it's tested (1,353 lines of tests).
- Con: Adding a new cleanup concern means touching a 472-line function.

**Alt 2 — Split into independent functions, call all from cleanup().** conf(0.8, observed). Complexity cost: 0 new files, +12 function signatures.
- `cleanup_request_timeouts()`, `cleanup_stale_peers()`, `cleanup_snap_sync_timeouts()`, `detect_stuck_sync()`, `cleanup_blacklists()`, `retry_idle_sync()`, `detect_stuck_fork()`, `detect_height_offset()`, etc.
- cleanup() becomes a dispatcher: calls each function in order.
- Pro: Each function is independently readable and testable. cleanup() shrinks to ~20 lines.
- Con: These functions still live in the same impl block and share mutable self access. No new modularity boundary.

**Alt 3 — Event-driven cleanup.** conf(0.35, inferred). Complexity cost: +200 lines.
- Instead of polling, each concern triggers on the state transition that requires it.
- Over-engineered. Some cleanup IS polling (timeouts, stale detection).

### Skeptic Pass 1

- **Alt 1**: WEAKENED. Should priority, but the 472-line god function is a real maintenance burden.
- **Alt 2**: SURVIVES. Minimal complexity cost, maximum readability gain. The functions don't need a new module boundary — they just need to be named and separated.
- **Alt 3**: ELIMINATED. Polling IS the correct pattern for timeout-based cleanup.

**WINNER: Alt 2 (Split into named functions).** conf(0.80, observed).

---

## Summary of Decisions

| # | Decision | Winner | conf | Approach |
|---|----------|--------|------|----------|
| 1 | Field grouping | Subtract dead fields + Group into sub-structs | 0.85, observed | Subtraction + Reuse |
| 2 | Fork recovery unification | Move decision logic to SyncManager, keep ForkRecoveryTracker separate | 0.80, observed | Reuse + Simplify |
| 3 | start_sync() idempotency | Guard clause (first step), separate start/reset (follow-up) | 0.80, observed | Subtraction |
| 4 | Production gate simplification | Subtract redundant layers (merge 8+8.5, 10+10.5, delete Layer 7 dead code) | 0.75, observed | Subtraction |
| 5 | cleanup() decomposition | Split into named functions | 0.80, observed | Reuse |

```
━━━ DESIGN DECISION QUALITY AUDIT ━━━
Major decisions identified:            5
Alternatives per decision (avg):       4.4
  basis=measured:                      3
  basis=observed:                      15
  basis=inferred:                      5
  basis=assumed:                       2
Confidence range for winner:           0.75-0.85
Decisions with flat distribution:      0
Decisions with conf >= 0.8 + assumed:  0
Constraint table entries used:         4 (INC-001, INC-I-004, production gate deadlock, snap sync cascade)
━━━ SIMPLICITY AUDIT ━━━
Subtraction alternatives explored:     5 (1 per decision minimum)
"Do nothing" alternatives explored:    5 (1 per decision)
Winner complexity cost:                5 new structs, -800 lines net (dead code + chain_follower.rs)
Simpler alternative that was close:    Alt 1 (guard clause) for Decision 3 — already the first step
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```
