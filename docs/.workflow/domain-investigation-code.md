# Domain Investigation Report: Code/Logic

## Domain Lens
Source code correctness -- control flow, state transitions, validation, consensus rule implementation, regression bugs in the ce1a72dc..HEAD fix batch. Specifically: does the HeaderFirstSync code path have a legitimate exit for dead-fork nodes? Does `recently_synced()` create a fundamental coverage hole? Is `signal_stuck_fork()` wired to anything?

## Chain Context
- Chain: DOLI (PoS, 10s slots, 18-node LOCAL testnet: seed + n1-n17, 14 producers)
- Branch: `main` @ `479711b5`
- Time window: 2026-05-19 20:16 (deploy) to 22:51 (locked snapshot)
- Activation heights: INC-I-078/080 AH=109,559 crossed (active)

## What I Don't Understand
1. Why n2's log stops at 22:05:53 but the snapshot shows it frozen at h=110,361 (suggests the node recovered briefly then re-froze, or log file was rotated)
2. Why n3/n10 are on the EXACT same forked chain (same height, hash, cs, utxo) -- they are validating each other's fork but not the canonical chain
3. What the original trigger event was that put these nodes on a minority fork (need fork forensics, not code analysis)
4. Whether the two advancing clusters (A vs seed-cluster) are the same chain or a separate fork

## Domain Relevance Assessment
**Relevance: HIGH**
The frozen nodes are stuck in a code-level deadlock in the sync recovery state machine. The root cause is a coverage hole in the `RecoveryCoordinator::classify()` logic: it has no escalation path for nodes that (a) are on a dead fork, (b) have `last_applied_secs >= 60`, and (c) have a gap < 500. This is definitively a code/logic bug, not a parameter, connectivity, or fork-origin issue (though the initial fork trigger is a cross-domain question).

## Hypotheses

### H1: The `recently_synced()` precondition on Rule 1 (ShallowRollback) creates a FUNDAMENTAL coverage hole -- conf(0.65, measured)

**Kill test**: Find ANY code path that escalates dead-fork nodes (gap 25-44, last_applied > 60s) to a rollback or snap sync.

**Kill test result**: NO such path exists. Exhaustive trace of ALL recovery paths confirms the hole.

**Evidence**:

The classify() function at `crates/network/src/sync/manager/recovery.rs:252-363` has this structure:

- **Rule 1** (line 301-321): ShallowRollback -- requires `ctx.recently_synced()` which is `last_applied_secs < 60` (line 180-183). Frozen nodes with `last_applied_secs > 60` NEVER enter Rule 1. The FINALITY_GUARD (lines 307-319) is INSIDE Rule 1 and is therefore unreachable.

- **Rule 2** (line 330-341): SnapSync -- requires `rollback_exhausted || large_gap || deep_fork_confirmed`. For frozen nodes: `rollback_exhausted = false` (shallow_rollback_count=0 because Rule 1 never fires), `large_gap = false` (gap < 500), `deep_fork_confirmed = false` (requires either DeepForkSuspected evidence which is never reported in production code, OR empty_count >= 10 which is prevented by the reset mechanism described in H2).

- **Rule 3** (line 346-349): HeaderFirstSync -- `medium_gap` (0 < gap < 500) or `stale_and_behind`. This ALWAYS matches for the frozen nodes. Returns HeaderFirstSync every tick.

- **Rule 4** (line 356-359): GenesisResync -- requires `apply_fails >= 5` and `last_applied_secs >= 600`. No apply attempts happen on a dead fork.

Observed in live logs: n7 at 22:06:22 shows `[COORDINATOR] action=HeaderFirstSync gap=44 last_applied=1351s` -- 22+ minutes stuck with the coordinator returning HeaderFirstSync every second.

### H2: The HeaderFirstSync action's `reset_empty_headers()` creates a counter-oscillation that PREVENTS escalation thresholds from being reached -- conf(0.65, measured)

**Kill test**: Find evidence that `consecutive_empty_headers` reaches 10 despite the resets.

**Kill test result**: PARTIALLY KILLED. Some nodes (n14, n7) DO eventually reach 10 via the `dispatch.rs:96` path, but the escalation still fails (see H3). However, for nodes caught early (like n2 with gap=5), the oscillation prevents reaching 10.

**Evidence**:

At `bins/node/src/node/periodic.rs:621-626`, the HeaderFirstSync action calls `sync.reset_empty_headers()` which sets `consecutive_empty_headers = 0` (production_gate.rs:523-525). This happens every periodic tick (~1s). The counter can only accumulate between ticks.

The flow: reset to 0 -> hash-based GetHeaders(forked_hash) -> empty -> count=1 -> height-based GetHeadersByHeight -> chain break -> count back to 0 or +1 -> repeat. The counter oscillates between 0 and 3-5.

Additionally, `height_fallback_attempted` (types.rs:531-533) is set to true after the FIRST height-based attempt and is ONLY cleared on successful header validation (response.rs:365 `valid_count > 0`). After the first failed height-based attempt, the node can never attempt it again. The fallback is one-shot.

BUT: for nodes stuck long enough (n14, n7), the `dispatch.rs:96-139` path eventually sees 10+ empties. This happens because the coordinator's cooldown (30s) sometimes allows empties to accumulate past the reset cycle. So the kill test partially disproves this as the SOLE mechanism.

### H3: The snap sync gap threshold (50 blocks) creates a DEAD ZONE where snap escalation is unreachable for moderate forks -- conf(0.60, measured)

**Kill test**: Check if snap sync succeeds for nodes with gap < 50.

**Kill test result**: Confirmed dead zone. n7 (gap=44) and n14 (gap=34 at snapshot) show snap attempts exhausting without success.

**Evidence**:

Three interacting conditions create the dead zone:

1. `dispatch.rs:105`: snap redirect requires `gap > self.snap.threshold` (50). Gaps of 25-44 FAIL this check -> genesis resync fallback instead.

2. `production_gate.rs:681-688`: snap attempts exhausted (3/3) -> REFUSED. n7 log at 22:06:15 shows: `[RECOVERY] Genesis resync REFUSED: snap attempts exhausted (3/3)`.

3. `cleanup.rs:475-494`: snap attempt reset requires `gap > self.snap.threshold` (50). With gaps < 50, attempts are NEVER reset.

n14 log shows the progression:
- 21:15-21:30: 3/3 snap attempts failed (no quorum -- fleet is fragmented)
- 21:56-22:00: another 3/3 snap attempts failed
- After that: continuously REFUSED due to exhausted attempts + gap < threshold

### H4: `signal_stuck_fork()` is a dead signal -- the flag it sets is never read in production code -- conf(0.65, measured)

**Kill test**: Find production code that calls `take_stuck_fork_signal()`.

**Kill test result**: NOT FOUND. Only test code calls it.

**Evidence**:

`signal_stuck_fork()` at production_gate.rs:547-560 sets `self.fork.stuck_fork_signal = true`. The corresponding consumer `take_stuck_fork_signal()` at production_gate.rs:539-543 is called ONLY in test code:
- tests.rs:2345 (`assert!(manager.take_stuck_fork_signal())`)
- tests.rs:4232 (`manager.fork.stuck_fork_signal`)

No file in `bins/node/src/` calls `take_stuck_fork_signal()` or reads `stuck_fork_signal`. The comment at types.rs:520 says "Consumed by take_stuck_fork_signal() in resolve_shallow_fork()" but `resolve_shallow_fork()` does not exist in the node code.

This means the empty headers handler at response.rs:316 (`self.signal_stuck_fork()`) fires but the signal goes nowhere.

### H5: INC-I-081 fixes (e25a9a97, 52116b64, 4349403a) did NOT introduce this deadlock -- conf(0.55, inferred)

**Kill test**: Find evidence that the deadlock is CAUSED by one of the INC-I-081 commits.

**Kill test result**: No causation found. The deadlock exists in the base recovery coordinator logic, not in the INC-I-081 additions.

**Evidence**:

- `cbaa3963` (FINALITY_GUARD): Adds a check INSIDE Rule 1 that refuses rollbacks below finality. Since Rule 1 is unreachable for frozen nodes, this code never executes. It neither helps nor hurts.

- `e25a9a97` (plan_reorg ancestor fallback): Changes plan_reorg to use block_store height lookup. Only relevant when fork recovery actually reaches plan_reorg -- the frozen nodes never get that far.

- `52116b64` (direct-apply fallback): Adds fallback apply in fork recovery completion. Only relevant when fork recovery completes -- frozen nodes are stuck in header-first sync, not fork recovery.

- `4349403a` (clear finality on rollback): Clears stale finality markers after rollbacks. No rollbacks happen on frozen nodes (shallow_rollback_count=0).

The deadlock predates the INC-I-081 bundle. The bundle's fixes are correctly scoped to their targeted scenarios but do NOT cover the dead-fork / HeaderFirstSync loop.

## Key Evidence Found

### Log evidence (measured):

1. **n7 at 22:06:22** (~/testnet/logs/n7.log): `[COORDINATOR] action=HeaderFirstSync gap=44 last_applied=1351s peers=15 shallow_rb=0 last_rb_h=Some(110378) snap_attempts=3 grace=false` -- coordinator returning HeaderFirstSync with 22+ minutes stuck, snap exhausted.

2. **n7 at 22:06:15-22:06:21** (~/testnet/logs/n7.log): Alternating `[RECOVERY] Genesis resync BYPASSING floor=110377 for emergency recovery` followed by `[RECOVERY] Genesis resync REFUSED: snap attempts exhausted (3/3)` -- every second.

3. **n14 at 22:05:43-22:06:13** (~/testnet/logs/n14.log): Same pattern as n7. `sync_fails=138` at HEALTH log.

4. **n2 at 22:05:53** (~/testnet/logs/n2.log): Last log entry shows APPLY_START via SILENCE_PULL -- node MAY have briefly recovered then re-froze. gap=5, last_applied climbing.

5. **n3 at 22:06:00** (~/testnet/logs/n3.log): Serving `GetHeadersByHeight` requests from OTHER frozen nodes (n14 at h=110385, another at h=110388) but returning empty headers because n3 itself is at h=110367. Frozen nodes polling each other.

### Code evidence (measured):

6. **recovery.rs:180-183**: `recently_synced()` = `last_applied_secs < 60`. Frozen nodes fail this.

7. **recovery.rs:301-304**: Rule 1 requires `recently_synced()`. Dead code for frozen nodes.

8. **recovery.rs:330-341**: Rule 2 requires `rollback_exhausted || large_gap(>=500) || deep_fork_confirmed`. All false for gaps 25-44.

9. **production_gate.rs:523-525**: `reset_empty_headers()` only resets the counter, does NOT reset `height_fallback_attempted`.

10. **production_gate.rs:539-543**: `take_stuck_fork_signal()` has ZERO callers in bins/node/src.

11. **cleanup.rs:475-494**: snap attempt reset requires `gap > self.snap.threshold` (50). Gaps < 50 never reset.

12. **dispatch.rs:105**: snap redirect requires `gap > self.snap.threshold` (50).

## Causal Chain (root cause identified)

| # | Item | Derived? | Derivation |
|---|------|----------|------------|
| 1 | Node ends up on a minority fork (trigger, unknown cause) | NO -- UNEXPLAINED | Fork forensics needed (fork/divergence domain) |
| 2 | No gossip blocks arrive that chain from the forked tip | YES | Canonical chain has diverged; gossip blocks have different prev_hash |
| 3 | `last_applied_secs` grows past 60s | YES | No blocks can be applied on a dead fork |
| 4 | `recently_synced()` returns false | YES | `last_applied_secs >= 60` (recovery.rs:181) |
| 5 | Rule 1 (ShallowRollback) in classify() is UNREACHABLE | YES | Rule 1 requires `recently_synced()` (recovery.rs:304) |
| 6 | Rule 3 (HeaderFirstSync) is returned every tick | YES | `medium_gap = true` for 0 < gap < 500 (recovery.rs:346-349) |
| 7 | `reset_empty_headers()` prevents consecutive_empty_headers from reaching escalation thresholds | YES | HeaderFirstSync action at periodic.rs:624 |
| 8 | Height-based GetHeadersByHeight returns canonical headers that chain-break against forked local_hash | YES | headers.rs:68 checks prev_hash against local_tip; canonical prev_hash != forked local_hash |
| 9 | `height_fallback_attempted` becomes permanently true after first chain-break | YES | response.rs:229 sets it; only cleared at response.rs:365 on valid_count > 0 |
| 10 | For nodes that DO reach 10+ empties, snap sync gap check (gap < 50) blocks snap redirect | YES | dispatch.rs:105 requires gap > snap.threshold(50) |
| 11 | Snap attempt counter never resets because gap < threshold | YES | cleanup.rs:479 requires gap > snap.threshold |
| 12 | `signal_stuck_fork()` fires but nobody reads the flag | YES | Zero callers of take_stuck_fork_signal() in production code |
| 13 | Node stays frozen PERMANENTLY until manual intervention | YES | No remaining code path can escape the loop |

## Definitive Answers to Key Questions

### Is the `recently_synced()` precondition on FINALITY_GUARD a fundamental code gap?

**YES.** The `recently_synced()` check at recovery.rs:304 gates ALL of Rule 1, including the FINALITY_GUARD at lines 307-319. Once a node has been on a dead fork for > 60 seconds, Rule 1 is completely unreachable. The FINALITY_GUARD has a coverage hole: it protects nodes that are actively syncing (recently applied a block) from rolling back past finality, but it cannot help nodes that have been stuck on a dead fork long enough to lose `recently_synced()` status.

However, the FINALITY_GUARD itself is NOT the root cause. Even without the FINALITY_GUARD, Rule 1 would be unreachable. The root cause is that the classify() function has NO escalation path for nodes in the state (dead fork + not recently synced + gap < 500). Rule 3 (HeaderFirstSync) catches them but cannot resolve the underlying problem.

### Is there ANY code path that recovers a node whose local tip is on a fork no peer has the headers for?

**NO.** There is no automatic code path that can recover this condition. The node is permanently stuck. The only exits are:
1. Manual wipe + snap sync (operational, not code-driven)
2. If by chance a gossip block arrives whose prev_hash matches the forked tip (extremely unlikely since the fork is abandoned by all other nodes)

## Cross-Domain Signals

1. **Fork/Divergence domain**: The TRIGGER for the deadlock is a node ending up on a minority fork. I cannot determine from code analysis alone what caused the fork. The two advancing clusters (cluster A at h=110,396 vs seed-cluster at h=110,388) need forensic analysis to determine if they are the same chain or a separate fork. n3/n10 being on the same forked tip suggests a producer emitted a block that some nodes accepted while others didn't.

2. **Parameters/Tuning domain**: The `snap.threshold = 50` creates a dead zone for moderate forks (25-49 blocks). Lowering this threshold would allow snap sync escalation for smaller forks. Also, `SNAP_SYNC_GAP_MIN = 500` in recovery.rs is far too high for a testnet with 14 producers -- most forks will have gaps well under 500. The `EVIDENCE_TTL = 120s` and `ACTION_COOLDOWN = 30s` may also need tuning, though these are secondary.

3. **Connectivity domain**: n3 at 22:06:00 is answering `GetHeadersByHeight` requests from OTHER frozen nodes -- frozen nodes are polling each other for headers neither of them has. This creates a gossip-mesh subgraph of mutually stuck nodes. Not a root cause but amplifies the deadlock.

## Gaps

1. **Fork trigger analysis**: This investigation cannot determine WHAT put the nodes on a minority fork. That requires block-level forensics (fork/divergence domain).
2. **n2's recovery/re-freeze**: n2's log shows it applying a block at 22:05:53 via SILENCE_PULL, but the snapshot at 22:51:36 shows it at h=110,361. Either the node recovered and re-froze, or there's a log timing artifact. More log analysis needed.
3. **INC-I-082 rebuild correctness**: I verified that `psHash` is identical fleet-wide, confirming rebuild is not contributing to the fork. But I did not trace the full rebuild path for a snap-synced node because the fork is in ChainState/UtxoSet (block content), not ProducerSet (epoch state).
4. **Cluster A vs seed-cluster**: Whether these two advancing clusters are on the same chain (seed lagging) or a separate fork is unknown. This affects the severity assessment -- if even the advancing clusters are forked, the problem is deeper than just dead-fork recovery.
