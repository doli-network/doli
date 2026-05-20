# Domain Investigation Report: Parameters/Tuning — INC-I-083

**INC_ID:** INC-I-083
**RUN_ID:** 345
**Domain lens:** Protocol parameter configuration — recovery thresholds, gossipsub mesh sizing, snap-sync gates, activation heights, and their interaction with the 18-node testnet topology. Looking for parameter values that are technically valid but wrong for this fleet size or that create irrecoverable states.

## Chain Context
- **Chain:** DOLI PoS, 10s slots, 18-node LOCAL testnet (seed + n1-n17, 14 producers)
- **Consensus:** scheduled-producer + pooled epoch rewards
- **Client:** doli-node 6.21.20 (binary at HEAD `479711b5`)
- **Time window:** 2026-05-19 20:16 (deploy) through 22:51+ (snapshot + ongoing)
- **Activation heights:** INC-I-078/080 at testnet h=109,559 (crossed, current ~110,427)

## Domain Relevance Assessment

**Relevance: HIGH**

The root cause of the deadlock persistence is directly in the parameters/configuration domain. The `RecoveryCoordinator` **correctly classifies SnapSync** for frozen nodes — the classification logic and thresholds are working. But the downstream `request_genesis_resync()` dispatch function has **three independent gates** that block execution, each rooted in a parameter or configuration value. These are not code bugs (the gates were designed intentionally for safety) — they are parameter/configuration values that create irrecoverable states.

## Hypotheses

### H1: `CoordinatorSnapEscalation` not classified as emergency reason blocks `--no-snap-sync` nodes — conf(0.70, measured)

**Kill test:** If `CoordinatorSnapEscalation` were in the emergency list, would snap sync fire on n10?
**Result:** CONFIRMED. n10 log shows the exact sequence every 30s:
- `[COORDINATOR] action=SnapSync gap=55 last_applied=1471s peers=16 shallow_rb=0 snap_attempts=0`
- `[RECOVERY] Genesis resync REFUSED: snap sync disabled (reason: CoordinatorSnapEscalation). Header-first recovery only.`

The coordinator classifies correctly. `request_genesis_resync` at Gate 4 (`production_gate.rs:660-668`) checks:
```rust
if self.snap.threshold == u64::MAX && !is_emergency { /* REFUSED */ }
```
`CoordinatorSnapEscalation` is NOT in the `is_emergency` match (`production_gate.rs:614-619`: only `GenesisFallbackEmptyHeaders`, `AllPeersBlacklistedDeepFork`, `ApplyFailuresSnapThreshold`).

**Affected nodes:** n9, n10, n11, n12 — all have `--no-snap-sync` in launchd plist. n10 confirmed stuck at h=110,367 with `snap_attempts=0`: the snap-attempt budget is full but can never be used.

**Evidence:**
- `~/Library/LaunchAgents/network.doli.testnet-n10.plist`: `--no-snap-sync` present
- n10 log: `[COORDINATOR] action=SnapSync` followed immediately by `[RECOVERY] Genesis resync REFUSED: snap sync disabled (reason: CoordinatorSnapEscalation)` — repeating every ~30s from 21:57 through 22:04+
- `production_gate.rs:614-619`: emergency reasons = `{GenesisFallbackEmptyHeaders, AllPeersBlacklistedDeepFork, ApplyFailuresSnapThreshold}` — does NOT include `CoordinatorSnapEscalation`

### H2: `SNAP_ATTEMPTS_MAX = 3` with no reset mechanism creates permanent exhaustion — conf(0.70, measured)

**Kill test:** If snap_attempts were resettable, would n7 recover?
**Result:** CONFIRMED. n7 log shows:
- `snap_attempts=3` in every COORDINATOR log line
- `[RECOVERY] Genesis resync REFUSED: snap attempts exhausted (3/3)` — repeating every second
- `GenesisFallbackEmptyHeaders` IS an emergency reason (bypasses Gate 1 and Gate 4), but Gate 5 at `production_gate.rs:681-688` (`snap.attempts >= 3`) is unconditional — NO bypass exists.

Once 3 snap attempts are consumed (possibly from an earlier, unrelated incident), the node can NEVER escalate to SnapSync again for its entire lifetime. The coordinator's SnapSync classification is also blocked because `ctx.snap_attempts < SNAP_ATTEMPTS_MAX` (`recovery.rs:337`) is false, so `classify()` falls through to Rule 3 (HeaderFirstSync).

**Evidence:**
- n7 log: `[COORDINATOR] action=HeaderFirstSync gap=39 last_applied=1198s ... snap_attempts=3`
- `recovery.rs:337`: `ctx.snap_attempts < thresholds::SNAP_ATTEMPTS_MAX` — evaluated before snap is attempted
- `recovery.rs:203`: `pub const SNAP_ATTEMPTS_MAX: u8 = 3;`
- No code path resets `snap.attempts` once incremented (verified by grep)

### H3: `confirmed_height_floor > 0` blocks non-emergency coordinator escalation on n13 — conf(0.55, measured)

**Kill test:** Is `confirmed_height_floor` the reason n13 is stuck?
**Result:** PARTIALLY CONFIRMED. n13 log from May 18 (before this session) shows:
- `[RECOVERY] Genesis resync REFUSED: confirmed_height_floor=101100 (reason: CoordinatorSnapEscalation)`
- Gate 1 at `production_gate.rs:622`: `if self.confirmed_height_floor > 0 && !is_emergency` — REFUSED

However, n13 had prior emergency bypasses that succeeded (`GenesisFallbackEmptyHeaders` at 06:45:01 May 18, `ApplyFailuresSnapThreshold` at 08:01:02 May 18). These consumed snap_attempts, so n13 may also be snap-exhausted. Lower confidence because the current n13 state is from a prior incident.

### H4: Gossipsub `mesh_n_low=20` exceeds available peer count (17) in 18-node fleet — conf(0.40, inferred)

**Kill test:** Does `mesh_n_low > available_peers` cause measurable gossip delivery failure?
**Result:** NOT CONFIRMED as root cause, but the mismatch exists.

`defaults.rs:254-256`:
```
mesh_n: 25,      // = max_peers — but only 17 possible peers exist
mesh_n_low: 20,  // below this triggers GRAFT — but 17 < 20 → always in GRAFT mode
mesh_n_high: 50, // = max_peers*2
```

In an 18-node fleet (17 peers per node), the gossipsub mesh can never reach `mesh_n_low=20`. Every node is perpetually in "mesh underflow" mode, triggering continuous GRAFT requests. While this doesn't directly cause forking, it creates gossip-mesh instability that could contribute to delayed block delivery and transient tip races.

### H5: Activation-height edge case at testnet AH=109,559 — conf(0.05, measured) — **DEAD**

**Result:** KILLED. `psHash = 6eb003ff40` on EVERY node. Activation crossed 800+ blocks ago. All nodes agree on ProducerSet state. The activation transition did not create the fork.

## Key Evidence

### 1. RecoveryCoordinator classifies correctly; dispatch is blocked

`crates/network/src/sync/manager/recovery.rs:330-340`:
```rust
let deep_fork_confirmed = deep_fork > 0
    || (empty_count >= 10 && ctx.last_applied_secs >= thresholds::STALE_TIP_SECS);
if (rollback_exhausted || large_gap || deep_fork_confirmed)
    && ctx.snap_attempts < thresholds::SNAP_ATTEMPTS_MAX
    && ctx.peer_count >= thresholds::SNAP_MIN_PEERS
{ return RecoveryAction::SnapSync; }
```
This correctly fires for n10 (empty_count >> 10, last_applied=1471s >> 300s, snap_attempts=0 < 3, peers=16 >= 3).

`crates/network/src/sync/manager/production_gate.rs:614-619`:
```rust
let is_emergency = matches!(reason,
    RecoveryReason::GenesisFallbackEmptyHeaders
    | RecoveryReason::AllPeersBlacklistedDeepFork
    | RecoveryReason::ApplyFailuresSnapThreshold { .. }
);
```
`CoordinatorSnapEscalation` is NOT listed.

`crates/network/src/sync/manager/production_gate.rs:662-668`:
```rust
if self.snap.threshold == u64::MAX && !is_emergency {
    info!("[RECOVERY] Genesis resync REFUSED: snap sync disabled ...");
    return false;
}
```

### 2. Three independent gates block recovery

| Gate | Location | Blocks | Affected nodes |
|---|---|---|---|
| Gate 4: snap disabled + non-emergency | production_gate.rs:662-668 | `--no-snap-sync` + `CoordinatorSnapEscalation` not emergency | n9, n10, n11, n12 |
| Gate 5: snap_attempts exhausted | production_gate.rs:681-688 | `snap.attempts >= 3`, unconditional, no bypass | n7 |
| Gate 1: confirmed_height_floor | production_gate.rs:622-628 | `confirmed_height_floor > 0` + non-emergency | n13 |

### 3. Recovery thresholds DO fire — the problem is downstream

Expected behavior for n10:
- `gap = 55` (110,422 - 110,367)
- `last_applied_secs = 1471` (>> 300)
- empty_count in coordinator window: ~120 entries over 120 s TTL >> 10
- `deep_fork_confirmed = (0 > 0 || (120 >= 10 && 1471 >= 300))` = TRUE
- `snap_attempts = 0 < 3`, `peer_count = 16 >= 3`
- **Classify result: SnapSync** (confirmed by log)
- **Dispatch result: REFUSED** at Gate 4

### 4. `STALE_TIP_SECS=300` IS reachable from inside the HeaderFirstSync loop

The brief asks: "is `STALE_TIP_SECS=300` reachable from inside the HeaderFirstSync loop or is it on a path that requires `recently_synced=true`?"

**Answer: YES, it is reachable.** `STALE_TIP_SECS` is used in `deep_fork_confirmed` (`recovery.rs:334`), which is in Rule 2 — evaluated AFTER Rule 1 (which requires `recently_synced`). Rule 2 has no `recently_synced` precondition.

Path:
1. Periodic task reports StaleTip when `last_applied >= 30 && gap > 0` (`periodic.rs:601`)
2. Periodic task reports EmptyHeaders when `consecutive_empty_headers >= 3` (`periodic.rs:598`)
3. Coordinator classify() skips Rule 1 (recently_synced=false)
4. Rule 2 evaluates `deep_fork_confirmed = (empty_count >= 10 && last_applied_secs >= 300)` — TRUE
5. Returns SnapSync

The frozen nodes DO get a SnapSync classification. The blockage is in the dispatch, not the classification.

### 5. `recently_synced=60s` is NOT the bottleneck for the deadlock

The 60 s window is irrelevant to the deadlock. `recently_synced()` gates Rule 1 (ShallowRollback). The frozen nodes need SnapSync (Rule 2), which does NOT require `recently_synced`. Making `recently_synced` wider would allow FINALITY_GUARD to engage longer, but wouldn't solve the HeaderFirstSync loop for nodes that need full SnapSync escalation.

### 6. Gossipsub mesh parameters vs fleet size

| Parameter | Value | Fleet reality | Mismatch |
|---|---|---|---|
| `max_peers` | 25 | 17 possible peers | YES — over-provisioned |
| `mesh_n` | 25 | max 17 mesh members | YES — can never fill |
| `mesh_n_low` | 20 | 17 < 20 | YES — perpetual underflow |
| `mesh_n_high` | 50 | 17 << 50 | YES — unreachable |
| `gossip_lazy` | 25 | 17 possible targets | YES — redundant |

Structural over-provisioning. May contribute to fork probability but not root cause.

### 7. No "escalate to full SnapSync after N chain-breaks" parameter exists

The chain-break path (`response.rs:390-401`) increments `consecutive_sync_failures` and `consecutive_empty_headers` but has no threshold-based escalation to SnapSync. Indirect path via `STALE_TIP_SECS` works but is fragile.

## Causal Chain

| # | Item | Derived? | Source |
|---|---|---|---|
| 1 | Node ends up on a minority fork (tip race or partition) | NO — UNEXPLAINED | Trigger in fork domain |
| 2 | HeaderFirstSync returns chain-breaks (headers don't chain to forked tip) | YES | response.rs:390 → valid_count=0 |
| 3 | consecutive_empty_headers climbs; periodic task feeds EmptyHeaders to coordinator | YES | periodic.rs:598 |
| 4 | After 300s, coordinator classifies SnapSync (Rule 2: deep_fork_confirmed) | YES | recovery.rs:330-340 |
| 5 | Dispatch calls `request_genesis_resync(CoordinatorSnapEscalation)` | YES | periodic.rs:628-629 |
| 6a | **Gate 4 blocks** on `--no-snap-sync` nodes (n10, n9, n11, n12) | YES | production_gate.rs:662 |
| 6b | **Gate 5 blocks** on snap-exhausted nodes (n7) | YES | production_gate.rs:681 |
| 6c | **Gate 1 blocks** on floor-set nodes (n13) | YES | production_gate.rs:622 |
| 7 | Node remains in HeaderFirstSync loop indefinitely — permanent deadlock | YES | No recovery path remains |

## Cross-Domain Signals

1. **For Code/Logic:** The classification of `CoordinatorSnapEscalation` as non-emergency in `request_genesis_resync` is arguably a code-logic bug, not a parameter issue. The emergency list at `production_gate.rs:614-619` should include `CoordinatorSnapEscalation` if the coordinator's authority is to be respected over the `--no-snap-sync` flag.
2. **For Code/Logic:** `snap.attempts` has no reset mechanism. Once at 3, it's permanent.
3. **For Fork/Divergence:** Initial fork trigger is not in parameters. Parameters prevent recovery FROM the fork, not the fork itself.
4. **For Connectivity:** `mesh_n_low=20 > fleet_size_minus_one=17` may contribute to gossip delivery delays that make tip races more likely.
5. **For Code/Logic:** Chain-break path at `response.rs:390-401` increments counters but does NOT directly report evidence to RecoveryCoordinator — relies on periodic task indirection.

## Gaps

1. Cannot determine why `--no-snap-sync` was set on n9–n12 (intentional or accidental).
2. Cannot determine n7's snap-attempt history (session vs prior incident).
3. Cannot determine n13's full history of `confirmed_height_floor=101,100`.
4. Did not verify the seed's deadlock mechanism (no `--no-snap-sync` but still frozen).
5. Gossipsub mesh impact on fork probability not quantitatively measured.

---

## Key files referenced

- `crates/network/src/sync/manager/recovery.rs` — lines 180-211 (thresholds), 252-363 (classify()), 330-340 (deep_fork_confirmed → SnapSync)
- `crates/network/src/sync/manager/production_gate.rs` — lines 608-688 (request_genesis_resync gates), 614-619 (emergency reason list)
- `crates/network/src/sync/manager/sync_engine/response.rs` — lines 244-254 (EmptyHeaders report path), 390-401 (chain-break path)
- `bins/node/src/node/periodic.rs` — lines 585-638 (coordinator dispatch), 598-602 (evidence reporting)
- `crates/core/src/network_params/defaults.rs` — lines 149-258 (testnet defaults), 254-257 (gossipsub mesh params)
- `~/Library/LaunchAgents/network.doli.testnet-n10.plist` — `--no-snap-sync` flag
- `~/testnet/logs/n10.log` — COORDINATOR SnapSync + REFUSED dispatch (measured)
- `~/testnet/logs/n7.log` — snap_attempts=3 exhaustion + REFUSED (measured)
- `~/testnet/logs/n13.log` — confirmed_height_floor blocking (May 18)
