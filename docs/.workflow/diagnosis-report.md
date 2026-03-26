# Diagnosis Report: INC-I-012 — 50-Node Network Stress Test Hardening

**Date**: 2026-03-25
**INC_ID**: INC-I-012
**RUN_ID**: 72
**Branch**: fix/sync-state-explosion-root-causes

## Symptom Profile

- **What happens**: Under a 50-node local stress test, the network exhibits fork storms, bootstrap failures (15/50 nodes stuck at genesis), event loop starvation, and connection-layer RAM explosion. Previous fixes (conn_headroom increase, event drain cap, eviction cooldown, max_peers reduction) each addressed one symptom but exposed others.
- **When**: During initial bootstrap of 50+ nodes on a single Mac (16 cores, 128GB RAM). Severity peaks in first 60 seconds.
- **Deterministic**: The categories of failure are deterministic. The specific nodes affected vary.
- **Failure boundary**: Affects late-joining nodes most severely. Seeds and early nodes are resilient.

## Fundamentals Check

| Item | Status | Evidence |
|------|--------|---------|
| Disk space | PASS | 296GB available |
| Memory | PASS | 84GB free (marginal but sufficient for 50 nodes) |
| CPU | FAIL | Load average 19.8 on 16-core (already oversubscribed) |
| File descriptors | PASS | Unlimited |
| Binary version | PASS | 4.0.4 (2c94404e) current |
| Network params | PASS | max_peers=50, mesh_n=12, conn_headroom=60 |

## Dimensional Audit

| Axis | Assessment | Implication |
|------|------------|-------------|
| Scale/Volume | 50 nodes near hardware limit. 136 nodes proved catastrophic. | Stay at 50 max for single-machine testing. |
| Time/Latency | 10s slots with mesh_n=12 = 3-4 hops = <1s propagation. BUT CPU oversubscription (16 cores / 50 nodes) means ~0.3 cores/node, so all timeouts fire 3x faster than designed. | Timeouts calibrated for normal CPU; stress test will surface false positives. |
| Complexity | 50 interacting state machines with gossipsub, sync, fork recovery, snap sync. Previous failures were emergent (gossip x sync x memory). | Interaction effects are the primary failure class. |

## Invariant Mapping

1. **Connection limit invariant**: conn_headroom must allow all nodes to bootstrap AND not cause RAM explosion. Currently violated: 60 connections too few for 50 nodes, but 120 causes RAM issues.
2. **Fork choice invariant**: Heavier chain wins. Currently violated for equal-weight chains via `<=` comparison.
3. **Event processing invariant**: Block events must be processed before production to prevent stale-tip forks. Partially violated when >50 events queue.
4. **Resource bounds invariant**: System load must stay below core count. Violated at 50 nodes on 16 cores.

## Occam's Ordering

| Level | Checked | Finding |
|-------|---------|---------|
| 1. Configuration | YES | max_peers=50, mesh_n=12 correct for 50 nodes |
| 2. Resources | YES | CPU oversubscribed (19.8 on 16 cores) |
| 3. Dependencies | N/A | Local test |
| 4. Data | N/A | Fresh testnet |
| 5. Build/Deploy | YES | Binary current |
| 6. Logic errors | YES | Equal-weight tie-breaking bug in reorg.rs |
| 7. Interaction effects | **PRIMARY** | conn_headroom vs RAM, forks vs recovery speed |
| 8. Race conditions | POSSIBLE | CPU starvation causes timing-dependent failures |
| 9. Emergent behavior | YES | Bootstrap storm (gossip + connections + events) |

## Evidence: Failed Approaches

| # | Fix Attempted | What It Changed | Result | What This Eliminates |
|---|---|---|---|---|
| 1 | conn_headroom = max_peers*2+20 (2c94404e) | Raised connection limit from 60 to 120 | Fixed 15 stuck nodes, caused RAM explosion + fork storms | Root cause NOT solely connection limits. Must not raise steady-state connections. |
| 2 | Event drain (50 events) before production | Added try_next_event() drain loop | Prevents stale-tip forks but GSet flooding still creates churn | Event order helps but insufficient alone. |
| 3 | max_peers 200 -> 50 (16d6a5fb) | Reduced per-node connection ceiling | Eliminated Yamux explosion at 136 nodes | Helped RAM but doesn't fix O(N^2) gossip at old N |
| 4 | Eviction cooldown 30s (928440d3) | Prevents rapid evict-reconnect loops | Breaks thrashing loop | Symptom relief for reconnect churn |
| 5 | GSet delta sync (bloom filter) | Replaced full-state broadcast with bloom filter | ~10x reduction in gossip traffic for established networks | Gossip amplification fixed for steady state. Bootstrap still uses full sync. |
| 6 | conn_headroom reverted to max_peers+10 | Reverted 2c94404e | Re-exposes bootstrap bottleneck | Need a different approach to bootstrap |

## System Model

```
                    ┌─────────────────────────────────┐
                    │        50-NODE BOOTSTRAP         │
                    └────────────┬────────────────────┘
                                 │
                    ┌────────────▼────────────────────┐
                    │     CONNECTION STORM             │
                    │  50 nodes x ~2 conn each = 100  │
                    │  conn_headroom = 60 per node     │
                    │  SATURATED: late nodes rejected  │
                    └────────────┬────────────────────┘
                                 │
              ┌──────────────────┼──────────────────┐
              │                  │                  │
    ┌─────────▼──────┐ ┌────────▼───────┐ ┌───────▼─────────┐
    │ GOSSIP STORM   │ │ EVENT FLOOD    │ │ FORK STORM      │
    │ 50 bloom/sec   │ │ Identify+Kad+  │ │ All weight=1    │
    │ for first 30s  │ │ GSet+Status    │ │ <=  rejects     │
    │ (adaptive 1s)  │ │ in event queue │ │ equal-weight    │
    │ Self-resolving │ │ Drain cap: 50  │ │ gossip blocks   │
    └────────────────┘ └────────────────┘ └───────┬─────────┘
                                                   │
                                          ┌────────▼─────────┐
                                          │ FORK RECOVERY    │
                                          │ Serialized (1x)  │
                                          │ plan_reorg works │
                                          │ but slow (10s/   │
                                          │ tick per fork)   │
                                          └──────────────────┘
```

**Critical feedback loop**: Fork storm (D1) -> orphan blocks -> fork recovery (one at a time) -> 10s per resolution -> more forks accumulate -> event queue fills -> production starved -> more forks.

## Hypotheses

### H1: Bootstrap conn_headroom bottleneck — CONFIRMED — conf(0.75, observed)
- **Evidence**: 15/50 nodes stuck at genesis when conn_headroom=60. Raising to 120 fixed bootstrap but caused RAM issues. Reverted.
- **Root cause**: With max_established_per_peer=2 and 50 nodes, each node averages ~1.5 connections. Total connections per node = ~75 (50*1.5), exceeding limit of 60.
- **Fix direction**: Transient connection burst during bootstrap, not permanent increase.

### H2: Equal-weight fork stalls from `<=` in check_reorg_weighted — CONFIRMED — conf(0.85, measured)
- **Evidence**: reorg.rs line 262: `if new_chain_weight <= self.current_chain_weight { return None; }`. Dead code: `should_reorg_by_weight_with_tiebreak()` at reorg.rs:201 exists but is NEVER CALLED.
- **Root cause**: In young networks, all producers have weight=1. Equal-weight canonical blocks are rejected by gossip path. Falls through to slow fork recovery (plan_reorg + fork_block_cache + serialized processing). On 10s slots, this is too slow for convergence.
- **Fix**: Use `should_reorg_by_weight_with_tiebreak` in the gossip block path. Lower block hash wins deterministic ties.

### H3: CPU saturation amplifying all failure modes — CONFIRMED — conf(0.80, observed)
- **Evidence**: Load average 19.8 measured before test starts. 50 processes on 16 cores = 3x oversubscription.
- **Root cause**: Not a code bug. Testing environment constraint. But it causes all timeouts to fire early, creating spurious recovery events.
- **Fix**: Accept as testing limitation. Or reduce stress test to 30 nodes to stay within CPU budget.

### H4: Bootstrap gossip storm (first 30s) — WEAKENED — conf(0.40, inferred)
- **Evidence**: Bloom filter delta sync implemented. Fresh nodes use full sync only when GSet empty. GSet processing spawned to tasks.
- **Root cause**: Self-resolving after ~30s when adaptive gossip backs off.
- **Fix**: Not needed. Self-corrects.

### H5: Sync request deference — SURVIVES — conf(0.45, assumed)
- **Evidence**: MAX_SYNC_REQUESTS_PER_INTERVAL=8, silently deferred requests not retried.
- **Status**: Needs measurement. Most initial sync goes through snap sync (different path), so this may only matter for smaller gaps.

```
━━━ CONFIDENCE DISTRIBUTION ━━━
Hypotheses presented: 5
  basis=measured:  1  (real evidence)
  basis=observed:  2  (saw it happen)
  basis=inferred:  1  (logical deduction)
  basis=assumed:   1  ← INVESTIGATE FIRST
Confidence range: 0.40 – 0.85
Spread: 0.45
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

## Root Causes (Confirmed)

### Root Cause 1: Equal-Weight Tie-Breaking Bug (H2)

**One paragraph**: The `check_reorg_weighted()` function in `reorg.rs` uses `<=` to compare chain weights, causing the incumbent chain to always win ties. In young networks where all producers have weight=1, this means gossip blocks from the canonical chain are rejected when received by a node on a competing 1-block fork. The node must wait for the slower fork recovery path (serialized, one-at-a-time, 10s per tick) to resolve each fork. With 50 nodes and weight=1, this creates a sustained fork storm where forks accumulate faster than they can be resolved. The deterministic tie-breaking function `should_reorg_by_weight_with_tiebreak()` already exists but is dead code.

**Causal chain**:
1. Node A produces block at slot S (weight=1)
2. Node B produces competing block at slot S (weight=1)
3. Node C applies A's block first (becomes tip, weight=1)
4. B's block arrives via gossip. `check_reorg_weighted`: 1 <= 1 -> return None
5. B's block cached as orphan in fork_block_cache
6. Fork recovery starts (serialized, next tick)
7. Meanwhile, slot S+1 arrives. Node C may produce on A's fork (wrong tip)
8. Fork recovery completes via plan_reorg (delta=0), but by then S+1 has already forked
9. With 50 nodes, steps 1-8 happen on 10+ nodes simultaneously

### Root Cause 2: Connection Headroom / Bootstrap Bottleneck (H1)

**One paragraph**: With `conn_headroom = max_peers + 10 = 60` and `max_established_per_peer = 2`, the libp2p connection limits reject incoming connections before the peer table can manage eviction. When 50 nodes start simultaneously, each node attempts ~2 connections to each peer. The first 30 nodes fill the connection slots; the remaining 20 are rejected at the protocol level. Raising conn_headroom to 120 (commit 2c94404e) fixed bootstrap but caused Yamux buffer explosion (~5MB per connection * 120 connections = 600MB/node). The fix was reverted, leaving bootstrap broken.

**Causal chain**:
1. 50 nodes start simultaneously, each dialing bootstrap and cached peers
2. Per-node connection limit: 60 (max_peers + 10)
3. With 50 peers averaging 1.5 connections each = 75 needed, but only 60 allowed
4. libp2p rejects connections at ConnectionLimits level (before peer discovery)
5. Late-joining nodes never receive PeerConnected events
6. Without peers, they stay at height 0 forever (no sync, no gossip)

## Why Previous Fixes Failed

| Fix | Why It Didn't Work |
|---|---|
| conn_headroom = max_peers*2+20 (2c94404e) | Fixed bootstrap but doubled Yamux buffer memory. At 50 nodes * 120 connections * 5MB = 30GB+ RAM. The fix addressed connection count but ignored per-connection memory cost. |
| Event drain cap at 50 | Helps prevent stale-tip production but doesn't address the RATE at which forks are created (H2) or the SPEED at which they're resolved. 50 events is also arbitrary; under peak load, many more events queue. |
| max_peers 200->50 | Correct for scale reduction (prevented Yamux explosion at 136 nodes) but doesn't change the conn_headroom formula. At max_peers=50, headroom is still just 60. |
| Eviction cooldown 30s | Prevents reconnection thrashing but doesn't prevent initial connection rejection. Eviction requires a connection to exist first. |

## Fix Strategy (Minimum Set for Smooth 50-Node Stress Test)

### Subtraction Gate

```
━━━ SUBTRACTION GATE ━━━
□ Can the root cause be fixed by REMOVING code?
  → H2: YES — Remove the `<=` check in check_reorg_weighted and USE
    the existing should_reorg_by_weight_with_tiebreak instead.
□ Can a layer be COLLAPSED?
  → No. The gossip→reorg→fork_recovery pipeline serves different roles.
□ Can a dependency be DROPPED?
  → No.
□ Can the broken behavior be STOPPED?
  → H1: Not directly. Nodes need connections to function.
□ Is the code defending against an IMPOSSIBLE case?
  → The `<=` in check_reorg_weighted was intentional ("incumbent wins ties")
    but wrong for young networks where ALL forks are ties.
━━━━━━━━━━━━━━━━━━━━━━━━
Subtraction viable: PARTIAL — H2 fix is a code replacement, not addition.
```

### Priority 1: Fix Equal-Weight Tie-Breaking (H2) — SURGICAL

**File**: `crates/network/src/sync/reorg.rs`
**Change**: In `check_reorg_weighted()`, replace the `<=` comparison with a call to the existing `should_reorg_by_weight_with_tiebreak` logic. When weights are equal, use lower block hash as deterministic tie-breaker.

**Why this works**: All nodes will deterministically converge to the same chain (the one with the lower hash at the fork point). No fork recovery needed for equal-weight 1-block forks. This eliminates the fork storm at its source.

**Risk**: Changes fork choice behavior. Must verify that:
1. Heavier chain still always wins (no regression)
2. Tie-breaking is deterministic across all nodes
3. Finality check still respected

### Priority 2: Transient Bootstrap Connection Burst (H1)

**File**: `crates/network/src/service/mod.rs`
**Change**: Allow `conn_headroom = max_peers * 2 + 20` during first 120s after node start, then tighten to `max_peers + 10`. Implement via a timer that adjusts connection limits.

**Why this works**: Bootstrap connections are transient — most disconnect within seconds as the mesh stabilizes. The Yamux memory spike is brief (<120s) and bounded. After 120s, the tighter limit prevents steady-state RAM growth.

**Risk**: Yamux memory spike during bootstrap. At 50 nodes * 120 connections * 5MB = ~300MB per node for 120s. Bounded and temporary.

**Alternative** (simpler): Stagger node startup by 2s each (operational fix). Avoids code change entirely.

### Priority 3: Reduce Default Event Drain Cap or Make Adaptive (OPTIONAL)

**File**: `bins/node/src/node/event_loop.rs`
**Change**: Increase drain cap from 50 to `max_peers * 3` (150 for max_peers=50), or drain until no more NewBlock events remain.

**Why**: With 50 nodes, 50 events may not be enough to reach the block events buried behind connection/gossip events. A higher cap or selective drain (only drain non-block events, always process blocks) would be more effective.

**Risk**: Longer drain time before production. But production already has the escape hatch interval check.

## Residual Risks

1. **CPU oversubscription**: 50 nodes on 16 cores will always cause timing issues. This is a test environment limit, not a code bug. Recommend 30-node tests for reliable results or accept timing-related false positives.

2. **Gossip bootstrap storm**: Self-resolving after ~30s. Adaptive gossip backs off to 60s. Not worth fixing unless 30s startup instability is unacceptable.

3. **Sync request deference**: MAX_SYNC_REQUESTS_PER_INTERVAL=8 may throttle initial sync. Needs measurement under actual test conditions.

4. **Disk I/O contention**: 50 RocksDB instances on one volume. Needs measurement. May require lighter RocksDB configuration for stress testing.

## Implementation Order

1. **Fix H2 (tie-breaking)** — Highest impact, lowest risk. Eliminates fork storms at source.
2. **Fix H1 (bootstrap burst)** — Second highest impact. Eliminates bootstrap failures.
3. **H3 accepted** — CPU oversubscription is a test environment limit.
4. **H5 measured during test** — May not need a fix if snap sync handles initial catch-up.
