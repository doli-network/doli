# Diagnostic Reasoning Trace: Fork Sync Recovery Infinite Loop (4 Bugs)

**Date**: 2026-03-17
**Session**: Code-level root cause analysis of 4 interacting fork sync bugs
**Builds on**: Previous structural diagnosis (state explosion in SyncManager)

---

## Phase 1: Symptom Characterization

### Symptom Profile (extracted from bug report + log evidence)

1. **16% of sync-only nodes** become trapped in a 7-second infinite loop
2. Three `--no-snap-sync` nodes (seed, n3, n4) **permanently deadlocked** -- never recover
3. The loop: fork_sync finds ancestor at h=0, downloads 1 canonical block, reorg rejected (weight_delta=0), empty headers blacklist peers, cycle repeats
4. **Trigger condition**: 50+ nodes join simultaneously, genesis producers compete on blocks at same height (natural fork propagation race)
5. **Determinism**: The `--no-snap-sync` deadlock is 100% deterministic. The 16% rate is probabilistic (depends on which fork block a node receives first).

### What Changed Recently

Commit 7259d47 ("fix: address 3 root causes of persistent sync cascades") made 3 changes:
1. `remove_peer()` now recomputes `network_tip_height` from remaining peers
2. `stuck_fork_signal` boolean replaces force-setting `consecutive_empty_headers = 3`
3. `can_produce()` side effects extracted to dedicated `update_production_state()` method

These fixes addressed the state management layer. The 4 bugs under investigation exist at the decision logic layer above that.

---

## Phase 2: Evidence Assembly

### Constraint Table (from prior failed approaches)

| Fix Attempted | What It Changed | Result | What This Eliminates |
|---|---|---|---|
| 7259d47 fix 1: network_tip_height recompute | `remove_peer()` recalculates tip from live peers | Correct, but stuck nodes have REAL gap, not phantom | The problem is not phantom gap inflation |
| 7259d47 fix 2: stuck_fork_signal decoupling | New boolean replaces counter force-setting to 3 | Fork_sync triggers correctly now, but reorg is rejected | Counter oscillation was one problem; reorg rejection is another |
| 7259d47 fix 3: can_produce() side effects | Mutations moved out of query function | Irrelevant to sync-only nodes (they never produce) | Production gate bugs are orthogonal to sync recovery |
| 0d49b78: 30s fork_sync cooldown | Timer after fork_sync rejection | Prevents rapid re-triggering but creates delayed loop | The loop itself is not the root cause -- the rejection is |
| fb37e7c: --no-snap-sync single gate | `needs_genesis_resync()` returns false when flag set | Prevents snap sync correctly but blocks ALL recovery | The bug is not that snap sync fires -- it is that recovery is blocked |

### Data Flow Mapping

I mapped the exact code flow for the stuck sync cycle. Key files in the path:

1. `rollback.rs:198` -- `resolve_shallow_fork()` triggers fork_sync
2. `block_lifecycle.rs:391` -- `start_fork_sync()` picks peer via `best_peer_for_recovery()`
3. `fork_sync.rs` -- Binary search, download headers+bodies
4. `block_handling.rs:455` -- `execute_fork_sync_reorg()` makes weight decision
5. `sync_engine.rs:698` -- `handle_headers_response()` handles empty responses
6. `production_gate.rs:991` -- `needs_genesis_resync()` gates recovery escalation
7. `block_lifecycle.rs:497` -- `reset_sync_for_rollback()` resets counters

### Trust Boundaries Identified

- **Weight computation boundary** (block_handling.rs:540-565): Both chains weighted by same producer_set. Seniority-based `producer_weight()` returns 1 for ALL producers on young networks. Weight is useless as a fork choice signal in year 1.
- **Empty headers interpretation** (sync_engine.rs:698-726): Code always blames the peer. Never considers "we might be on a fork."
- **Recovery gating** (production_gate.rs:991-1003): `needs_genesis_resync()` hardcodes false for `--no-snap-sync`, conflating "snap sync" with "any state reset."
- **Peer selection** (mod.rs:985-991): `best_peer_for_recovery()` takes max height without filtering degraded/stuck peers.

---

## Explorer Round 1: Hypothesis Generation

### H1: Weight comparison uses strict `<=` instead of `<`, rejecting equal-weight canonical chains during remedial fork sync
- **Thesis**: `execute_fork_sync_reorg()` rejects the canonical chain when `weight_delta == 0`. For fork_sync (which is remedial -- the node KNOWS it is on a fork), equal weight should accept.
- **Preconditions**: Fork sync downloads 1 canonical block, both producers have weight 1, delta = 0.
- **Falsification**: If fork_sync always downloads multiple blocks (delta would be > 0). Disproved if fork_sync.rs limits downloads.

### H2: Empty headers cause immediate peer blacklist, isolating the node from canonical peers
- **Thesis**: When our tip is a fork hash, canonical peers return empty headers. The handler blacklists each peer, eventually removing all canonical peers.
- **Preconditions**: The handler blacklists on empty response, `best_peer()` filters blacklisted peers.
- **Falsification**: If `best_peer()` does NOT filter blacklisted peers, or if blacklist expires fast enough to prevent isolation.

### H3: `--no-snap-sync` unconditionally blocks the only recovery escalation path
- **Thesis**: `needs_genesis_resync()` returns false when `snap_sync_threshold == u64::MAX`, blocking state-reset recovery that actually preserves block data.
- **Preconditions**: The recovery path uses `reset_state_only()` which preserves blocks. The gate doesn't distinguish between "snap sync" and "state reset."
- **Falsification**: If `is_deep_fork_detected()` provides an alternative recovery path. Requires checking whether `consecutive_empty_headers` can reach 10.

### H4: Fork sync downloads only fork_depth blocks (as bug report claims)
- **Thesis**: Fork sync limits header download count to `fork_depth`, which might be 1-3. This means the canonical chain is short, making weight_delta=0.
- **Preconditions**: Fork sync code uses `fork_depth` or similar limit in GetHeaders.
- **Falsification**: If fork_sync.rs requests 500 headers regardless of fork_depth.

### H5: `consecutive_empty_headers` counter is reset to 0 by every fork_sync run, preventing escalation threshold (10) from being reached
- **Thesis**: `reset_sync_for_rollback()` resets the counter to 0 after both successful AND rejected reorgs. Since fork_sync runs every ~30-60s, the counter oscillates 0->3->0 and never reaches 10.
- **Preconditions**: `reset_sync_for_rollback()` always resets the counter. Fork_sync runs frequently enough.
- **Falsification**: If the counter is NOT reset on rejection, or if fork_sync runs infrequently enough for the counter to climb.

### H6: Peer selection (`best_peer_for_recovery()`) picks stuck/low-height nodes under load
- **Thesis**: Under extreme load, canonical peers disconnect. The remaining peer table has mostly stuck nodes. `best_peer_for_recovery()` picks max height from degraded pool.
- **Preconditions**: No minimum height filter in peer selection. Load causes canonical peers to drop.
- **Falsification**: If `best_peer_for_recovery()` filters by minimum height or network tip proximity.

### H7: The bug report's 4 bugs are actually a single feedback loop, not 4 independent issues
- **Thesis**: Bugs 1-4 interact in a cycle. Fixing any one might break the loop. The loop is: fork_sync triggered -> rejected at equal weight (Bug 1) -> empty headers blacklist peers (Bug 2) -> escalation blocked (Bug 3) -> peer degradation (Bug 4) -> fork_sync from stuck peer -> back to start.
- **Preconditions**: The bugs form a dependency chain where each one feeds the next.
- **Falsification**: If any bug is independently sufficient to cause permanent stuck state without the others.

---

## Skeptic Round 1: Hypothesis Elimination

### H1 (Weight `<=` vs `<`): SURVIVES
- **Evidence**: Code at block_handling.rs:568 confirms `if weight_delta <= 0`. The comment says "new chain not heavier -- keeping current."
- Weight computation at seniority.rs:20 confirms ALL producers return weight 1 for < 1 year.
- For 1-block fork vs 1 canonical block: delta = 1 - 1 = 0. REJECTED.
- **Consistent with failed approaches**: 7259d47 did not touch this line. The fork_sync_cooldown (0d49b78) delays re-triggering but does not change the rejection decision.
- **Status**: SURVIVES. This is a confirmed logic error in the weight gate.

### H2 (Blacklist escalation): SURVIVES
- **Evidence**: sync_engine.rs:720 confirms `self.header_blacklisted_peers.insert(peer, Instant::now())` for gap > 50.
- sync_engine.rs:80-86 confirms `best_peer()` filters blacklisted peers.
- cleanup.rs:303 confirms 30s blacklist expiry.
- cleanup.rs:310-357 confirms "all peers blacklisted > 120s" recovery -- but this is 2 minutes of dead time.
- **Consistent with failed approaches**: 7259d47 did not modify the blacklist logic.
- **Status**: SURVIVES. Blacklisting canonical peers is counterproductive when the node is the one on a fork.

### H3 (`--no-snap-sync` blocking recovery): SURVIVES
- **Evidence**: production_gate.rs:991 confirms `if self.snap_sync_threshold == u64::MAX { return false; }`.
- fork_recovery.rs:518-584 confirms `force_recover_from_peers()` uses `reset_state_only()` which preserves block data.
- The gate conflates "snap sync disabled" with "all recovery disabled."
- **Consistent with failed approaches**: fb37e7c explicitly added this gate. It was intentional but based on a flawed premise.
- **Status**: SURVIVES. This is a confirmed semantic error in the recovery gate.

### H4 (Fork sync downloads only fork_depth blocks): ELIMINATED
- **Evidence**: fork_sync.rs:193-196 shows `GetHeaders { start_hash, max_count: 500 }`. The download limit is 500, not fork_depth.
- The bug report's claim is wrong. Fork sync requests up to 500 headers.
- The reason nodes download only 1-3 blocks is not the request limit -- it is the PEER's height.
- **Status**: ELIMINATED. The bug report's diagnosis was incorrect.

### H5 (Counter reset preventing escalation): SURVIVES
- **Evidence**: block_lifecycle.rs:498 confirms `self.consecutive_empty_headers = 0` in `reset_sync_for_rollback()`.
- `execute_fork_sync_reorg()` at block_handling.rs:577 calls `reset_sync_for_rollback()` AFTER rejection.
- The counter goes: 0 -> 1 -> 2 -> 3 (triggers fork_sync) -> 0 (reset after rejection) -> 1 -> 2 -> 3 -> 0 -> ...
- Never reaches 10 (the threshold for `is_deep_fork_detected()` escalation).
- **Consistent with failed approaches**: 7259d47's `stuck_fork_signal` bypasses the counter for TRIGGERING fork_sync, but the counter is still needed for ESCALATION gating. The reset after rejection prevents escalation.
- **Status**: SURVIVES. This is a confirmed amplifier for Bug 3.

### H6 (Peer selection degradation): SURVIVES
- **Evidence**: mod.rs:985-991 confirms `best_peer_for_recovery()` uses `max_by_key(height)` with no minimum height filter.
- Under load with 56 processes on one machine, canonical peers can disconnect or become unresponsive.
- Log evidence: "Fork sync: new chain not heavier (delta=0, new=1, old=1)" -- only 1 canonical block downloaded.
- If the peer were at h=500, it would return 500 headers. Getting 1 means the peer is at h=1 (a stuck node).
- **Status**: SURVIVES. This explains WHY fork_sync gets insufficient data even though it requests 500.

### H7 (Single feedback loop): PARTIALLY SURVIVES
- Bugs 1-4 DO interact in a cycle, but Bug 3 is independently sufficient for `--no-snap-sync` deadlock.
- Bug 1 alone can trap nodes even without Bugs 2, 3, 4 (if fork_sync picks a near-tip peer that has only 1 more block).
- **Status**: WEAKENED to "contributing interaction" rather than "single unified cause."

---

## Explorer Round 2: Refined Hypotheses

After eliminating H4 and weakening H7, the surviving hypotheses refine as follows:

### H1a: Weight gate is the PRIMARY loop mechanism
- For ANY fork_sync that downloads exactly as many canonical blocks as rollback blocks, delta=0 and reorg is rejected.
- On young networks (all weights = 1), this is: "canonical blocks downloaded == fork blocks replaced."
- For 1-block forks, this means 1 canonical block -> rejection. For 2-block forks, 2 canonical blocks -> rejection.
- The ONLY escape is downloading MORE canonical blocks than fork blocks. This requires the peer to be ahead by more than the fork depth.

### H3a: Counter reset is the critical Bug 3 amplifier
- Even if `needs_genesis_resync()` were fixed to allow `--no-snap-sync` recovery, the counter never reaches 10 anyway.
- Both gates must be fixed: the unconditional suppression AND the counter reset.
- Without fixing the counter reset, the deep fork detection path (`is_deep_fork_detected()`) is also permanently blocked for ALL nodes.

### H2a: Blacklist threshold should match fork detection threshold
- After 3 empty header responses, the node knows it is on a fork (this is the `resolve_shallow_fork()` threshold).
- Blacklisting past this point is counterproductive -- it removes the canonical peers the node needs for recovery.
- Fix: stop blacklisting after `consecutive_empty_headers >= 3`, clear existing blacklist, set `stuck_fork_signal`.

### H6a: Peer selection should filter by network tip proximity
- `best_peer_for_recovery()` should require peers to be near `network_tip_height` (within 10 blocks).
- Fallback to max height if no near-tip peers exist.
- This prevents "stuck peer poisoning" where two low-height nodes sync from each other.

---

## Skeptic Round 2

### H1a (Weight gate = primary loop mechanism): CONFIRMED
- Code evidence at block_handling.rs:568 is unambiguous.
- Explains the log evidence: "delta=0, new=1, old=1" -- the exact scenario.
- Not contradicted by any failed approach (no prior fix touched this comparison).
- Cross-reference: The `mark_fork_sync_rejected()` + 30s cooldown (0d49b78) is a mitigation for THIS problem -- it slows the loop but doesn't fix the rejection.

### H3a (Counter reset = Bug 3 amplifier): CONFIRMED
- `reset_sync_for_rollback()` at block_lifecycle.rs:498 unconditionally resets to 0.
- Called after BOTH success and rejection at block_handling.rs:577.
- The counter path: 0->1->2->3 (fork_sync triggered) -> 0 (reset) -> repeat. Never reaches 10.
- This blocks `is_deep_fork_detected()` for ALL nodes, not just `--no-snap-sync`.

### H2a (Blacklist threshold alignment): CONFIRMED
- sync_engine.rs:698-726 blacklists immediately on first empty response (for gap > 50).
- After 3 empties, the node already knows it is on a fork. Continuing to blacklist removes recovery peers.
- The 120s "all peers blacklisted" timeout at cleanup.rs:310-357 is a safety valve but adds 2 minutes of dead time.

### H6a (Peer selection filtering): CONFIRMED
- mod.rs:985-991 confirms no height filter.
- Log evidence (1 block downloaded) confirms peers at low height are being selected.
- The fallback mechanism (try all peers if no near-tip peers) preserves current behavior as safety net.

---

## Analogist: Known Failure Patterns

### Pattern 1: "Remedial reorg should have different acceptance criteria than opportunistic reorg"
From Bitcoin's fork choice: Bitcoin uses "first seen" for equal-weight blocks during normal operation (no reorg). But during IBD (initial block download) or explicit fork recovery, the node accepts ANY chain that extends its known-good ancestry. DOLI's fork_sync is analogous to IBD -- the node knows it is broken and is actively seeking repair. Applying the same conservative "strictly heavier" rule as normal gossip reorgs defeats the purpose.

### Pattern 2: "Blame-self before blame-peer in fork detection"
From Ethereum's sync protocol: When a peer returns unexpected data during sync, Ethereum's protocol first checks "am I on a minority fork?" before penalizing the peer. The heuristic: if N out of M peers disagree with our state, and N > M/2, we are wrong. DOLI's current code always blames the peer.

### Pattern 3: "Recovery escalation counters must be monotonic until resolution"
From Raft's election timeout: Raft's election counter is never reset until a leader is actually elected. Resetting it on failed elections would prevent elections from ever succeeding. Similarly, DOLI's `consecutive_empty_headers` counter should not reset until the fork is actually resolved (successful reorg), not merely attempted (rejected reorg).

### Pattern 4: "Peer selection for recovery must use quality thresholds, not just max"
From BitTorrent's piece selection: BitTorrent picks peers based on their upload speed, not just their piece availability. Picking the peer with the "most" pieces from a degraded swarm leads to downloading from peers that are themselves incomplete. DOLI's `best_peer_for_recovery()` is equivalent to picking the peer with the most pieces without checking if they are "good enough."

---

## Diagnostic Tests (Designed, Not Yet Executed)

### Test 1: Reproduce weight_delta=0 rejection
- Start 3-node network
- Kill one producer mid-slot so it misses a block
- Force node to store a fork block at h=1 (from different producer)
- Observe: does `execute_fork_sync_reorg()` reject with delta=0?
- **Expected**: Yes, because both blocks have weight 1.
- **Distinguishes**: H1a (confirmed) vs "weight works fine" (eliminated)

### Test 2: Observe counter oscillation
- On the stuck node from Test 1, add logging to `consecutive_empty_headers` at every write site
- Observe: does the counter oscillate 0->3->0->3?
- **Expected**: Yes, because `reset_sync_for_rollback()` resets to 0 after rejection.
- **Distinguishes**: H3a (confirmed) vs "counter reaches 10 eventually" (eliminated)

### Test 3: Verify `--no-snap-sync` deadlock
- Start a seed node with `--no-snap-sync`
- Force it onto a fork
- Observe: does it recover?
- **Expected**: No, permanently stuck. `needs_genesis_resync()` returns false, counter never reaches 10.
- **Distinguishes**: H3 (confirmed) vs "alternative recovery path exists" (eliminated)

### Test 4: Verify peer selection under load
- Start 10 nodes, force 5 onto forks at h=1-3
- Check which peer each stuck node selects for fork_sync
- **Expected**: Some stuck nodes select other stuck nodes, leading to delta=0 even with weight fix.
- **Distinguishes**: H6a (confirmed) vs "peer selection always picks canonical peer" (eliminated)

---

## Results: Root Cause Synthesis

All 4 hypotheses (H1a, H2a, H3a, H6a) CONFIRMED through code analysis:

| Bug | Root Cause | Code Location | Confirmed By |
|-----|-----------|---------------|-------------|
| Bug 1 | `weight_delta <= 0` rejects equal-weight canonical chain during remedial fork_sync | block_handling.rs:568 | Code reads `<=`, seniority.rs returns 1 for all producers |
| Bug 2 | Empty headers always blacklist the responding peer, never self-detect fork | sync_engine.rs:698-726 | Code unconditionally blacklists; `best_peer()` filters blacklisted |
| Bug 3 | `needs_genesis_resync()` hardcodes false for `--no-snap-sync` + counter reset prevents escalation | production_gate.rs:991 + block_lifecycle.rs:498 | Code returns false when `snap_sync_threshold == u64::MAX`; counter reset on rejection |
| Bug 4 | `best_peer_for_recovery()` picks max height without quality threshold | mod.rs:985-991 | Code has no minimum height filter |

### The Interaction Loop

```
Fork block stored -> empty headers from canonical peers (they don't know fork hash)
    -> [Bug 2] blacklist canonical peers -> only stuck peers left
    -> [Bug 6a/4] fork_sync picks stuck peer -> downloads 1-3 blocks
    -> [Bug 1] weight_delta=0, rejected -> counter reset [Bug 3 amplifier]
    -> escalation blocked [Bug 3] for --no-snap-sync nodes
    -> back to start
```

### Why Commit 7259d47 Did Not Fix This

7259d47 fixed three bugs in the SyncManager's internal state management (phantom gap, counter oscillation via force-setting, production gate side effects). These are prerequisites -- fork_sync now triggers correctly. But the decision logic AFTER fork_sync triggers was not modified:
- The weight comparison was not changed
- The blacklist logic was not changed
- The `--no-snap-sync` gate was not changed
- The peer selection was not changed
- The counter reset after rejection was not changed

The 7259d47 fixes ensured fork_sync STARTS correctly. These 4 bugs cause it to FAIL correctly-started fork_sync.

---

## Fix Design Rationale

### Fix Priority: Bug 1 -> Bug 3 -> Bug 2 + Bug 4

**Bug 1** is the primary loop mechanism. Fix this and fork_sync succeeds when it downloads canonical blocks from a good peer. Estimated to fix ~50% of stuck nodes immediately.

**Bug 3** is the permanent deadlock for `--no-snap-sync` nodes. Requires both allowing the recovery signal AND fixing the counter reset. This is the second priority because `--no-snap-sync` affects seed/archiver nodes which are critical infrastructure.

**Bugs 2+4** together prevent the peer degradation that makes Bug 1 fixes insufficient under extreme load. With blacklist fixed (Bug 2), canonical peers stay available. With peer selection filtered (Bug 4), fork_sync always picks a good peer.

All 4 fixes together close the loop completely. No single fix is sufficient under all conditions, but Bug 1 alone handles the common case.
