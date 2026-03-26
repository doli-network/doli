# INC-I-012: Structural Architecture Analysis

**Date**: 2026-03-25
**INC_ID**: INC-I-012
**RUN_ID**: 72
**Branch**: fix/sync-state-explosion-root-causes
**Scope**: Structural analysis only — why the architecture allows these specific bugs.

## Architecture Constraint Table

| Source | Constraint | Implication for Design |
|--------|-----------|----------------------|
| INC-I-009 (incident) | max_peers=200 caused 86GB RAM in 3 min (Yamux buffer explosion) | Connection headroom cannot be raised without per-connection memory bound |
| INC-I-011 (commit 928440d3) | Eviction cooldown needed to break reconnect thrashing | Connection management operates at wrong abstraction level — reacts to symptoms, not phases |
| INC-I-012 (commit 2c94404e, reverted) | conn_headroom=max_peers*2+20 fixed bootstrap but caused RAM explosion + fork storms | Bootstrap and steady-state have irreconcilable connection count requirements at current abstraction level |
| REPORT_FORKS (commit ca0bd78) | `should_reorg_by_weight_with_tiebreak` was written and tested but never wired into `check_reorg_weighted` | Incomplete refactor — code was added alongside the existing path, not integrated into it |
| fork_recovery.rs lines 77-78, 112 | Fork recovery uses a DIFFERENT tie-breaking rule (solo-fork yields on tie) than the one written but not wired in (`should_reorg_by_weight_with_tiebreak` = lower hash wins) | Two fork-choice tie-breaking strategies exist in the codebase. Neither is used in the primary gossip path |

---

## Structural Explanation 1: Dual-Path Fork Choice Without Shared Invariant

**conf(0.90, observed)** — strongest explanation

### What the Code Shows

There are **three separate fork-choice decision points**, each with a different tie-breaking rule:

| Path | File | Tie-breaking behavior | When invoked |
|------|------|-----------------------|-------------|
| **Gossip path** | `reorg.rs:262` via `check_reorg_weighted()` | Incumbent always wins ties (`<=`) | `handle_new_block` -> `handle_new_block_weighted` (block_handling.rs:70) |
| **Fork recovery simple** | `fork_recovery.rs:77-78` via `check_reorg_weighted()` + solo override | Solo fork yields on tie (`weight_delta == 0 && our_fork_is_solo`) | `handle_completed_fork_recovery` after parent-chain walk completes |
| **Fork recovery deep** | `fork_recovery.rs:112` via `plan_reorg()` + solo override | Solo fork yields on tie (same as above) | Fallback when simple reorg fails for deeper forks |

The written-but-unused fourth option:

| Path | File | Tie-breaking behavior | When invoked |
|------|------|-----------------------|-------------|
| **Dead code** | `reorg.rs:201` `should_reorg_by_weight_with_tiebreak()` | Lower hash wins deterministically | NEVER — only called from tests |

### Why This Is the Root Architecture Flaw

There is no **single, canonical fork-choice function** that all paths go through. Instead, fork-choice logic is scattered across three call sites with **three different semantics for the same decision** (what to do when weights are equal).

The gossip path (the hottest path — every block from every peer) uses the most restrictive rule: incumbent always wins ties. The fork recovery path (cold, slow, serialized) is more lenient but only for solo forks. The deterministic tie-breaking function that would solve both problems exists but is orphaned.

### Missing Invariant

**"Fork-choice tie-breaking must be a single function called from all decision points."**

This invariant does not exist in the codebase. Each path was developed or patched independently:
- `check_reorg_weighted` was the original implementation (incumbent wins, conservative)
- `should_reorg_by_weight_with_tiebreak` was written during the REPORT_FORKS fix (commit ca0bd78) but was added *alongside* `check_reorg_weighted` rather than *replacing its tie-breaking logic*
- Fork recovery's solo-fork override was added in a separate pass to handle the N4 solo-fork bug

The result: a complete tie-breaking solution exists (`should_reorg_by_weight_with_tiebreak`), documented in REPORT_FORKS, tested with 2 test cases — but never integrated into the path that actually needs it.

### Blast Radius

The gossip path's `<=` rejection cascades into:
1. Block cached as orphan in `fork_block_cache`
2. Fork recovery triggered (serialized, one-at-a-time)
3. Fork recovery takes 10s+ per resolution (needs network round-trips)
4. Meanwhile, next slot arrives and production may use the wrong tip
5. With N producers all at weight=1, this produces O(N) forks per slot

Fixing `check_reorg_weighted` alone would fix 80%+ of the fork storm problem without touching fork recovery. The fork recovery path's solo-fork override would still handle the remaining edge cases (node isolated on its own chain). The two strategies are complementary, not contradictory.

---

## Structural Explanation 2: Connection Management Has No Phase Concept

**conf(0.85, observed)** — strong explanation

### What the Code Shows

Connection limits are set once at construction time in `service/mod.rs:137`:

```rust
let conn_headroom = (config.max_peers + 10) as u32;
```

This value is passed to `libp2p::connection_limits::ConnectionLimits` and cannot be changed after `Swarm::new()`. The libp2p `ConnectionLimits` behaviour is baked into the swarm at construction.

Meanwhile, the sync manager's `is_in_bootstrap_phase()` (production_gate.rs:310) already distinguishes bootstrap from steady-state:

```rust
pub fn is_in_bootstrap_phase(&self) -> bool {
    if self.local_height == 0 { return true; }
    if self.first_peer_status_received.is_some() && self.peers.is_empty() { return true; }
    false
}
```

But this phase information **never reaches the connection layer**. The architecture has a one-way dependency: `NetworkService` creates the swarm and spawns it into a background task (`run_swarm`). The swarm loop processes events and sends them to the node via channels. The node has no back-channel to dynamically adjust connection limits.

### Missing Invariant

**"Connection limits must adapt to the node's lifecycle phase (bootstrap vs. steady-state)."**

The current architecture makes this structurally impossible because:
1. `ConnectionLimits` is set at swarm creation time
2. The swarm runs in a spawned tokio task with no reconfiguration channel
3. Phase detection lives in the sync manager (node layer), not the network layer
4. libp2p's `ConnectionLimits` behaviour has no `update_limits()` method in the version used

### Why Previous Fixes Failed (Architectural Explanation)

Every fix for the connection headroom problem was a **steady-state parameter change** because the architecture only supports steady-state parameters:

| Fix | What it actually was | Why it failed |
|-----|---------------------|--------------|
| conn_headroom = max_peers*2+20 | Higher steady-state limit | RAM explosion in steady state |
| max_peers 200->50 | Lower steady-state ceiling | Didn't change the headroom formula |
| Eviction cooldown 30s | Steady-state rate limit | Doesn't help if connections are rejected before they exist |
| Revert to max_peers+10 | Original steady-state limit | Re-exposes bootstrap bottleneck |

The architectural gap is that **bootstrap needs temporarily high limits (max_peers*2+20 for ~120s) while steady-state needs low limits (max_peers+10)**. This requires either:
- Dynamic limits (libp2p `ConnectionLimits` doesn't support this directly)
- A lifecycle callback from node to swarm (doesn't exist)
- An application-level connection manager that pre-screens connections (doesn't exist)
- Staggered startup (operational, not architectural)

### Blast Radius

The connection bottleneck affects:
1. Late-joining nodes during bootstrap (15/50 stuck at genesis)
2. Nodes after a network partition when many reconnect simultaneously
3. Any scenario where the connection count transiently exceeds `max_peers + 10`

The blast radius is bounded to the bootstrap phase (~120s) and does NOT affect steady-state operation. This is why the problem only appears in stress tests and mass-restart scenarios.

---

## Structural Explanation 3: Dead Code as Architecture Symptom

**conf(0.80, observed)**

### The Pattern

The codebase contains a recurring pattern: a correct solution is implemented, tested, and documented — but not wired into the production path. `should_reorg_by_weight_with_tiebreak` is the clearest example:

| Artifact | Status |
|----------|--------|
| Function implementation | Complete (reorg.rs:201-211) |
| Tests | 2 tests passing (reorg.rs:724-755) |
| Documentation | REPORT_FORKS says "Added `should_reorg_by_weight_with_tiebreak()` method" |
| Production integration | MISSING — `check_reorg_weighted` still uses the old `<=` logic |
| `check_reorg_weighted` test for equal weight | EXISTS (line 678-721) but tests the NEW function's SEMANTIC via a conditional assertion, not the actual code path |

### Why This Happened

The REPORT_FORKS commit (ca0bd78) describes Bug 2 as "No Fork Choice Tie-Breaker for Equal-Weight Chains." The fix was to add `should_reorg_by_weight_with_tiebreak` as a new function. But the existing `check_reorg_weighted` was the function actually called from the production path, and it was NOT modified to use the new tie-breaking logic.

The likely sequence:
1. `should_reorg_by_weight` existed with `>` (strictly greater)
2. Bug report: equal-weight forks never resolve
3. Fix: add `should_reorg_by_weight_with_tiebreak` alongside
4. `check_reorg_weighted` was modified separately (the `<=` + comment about "incumbent wins ties") — but this change reinforced the old behavior rather than integrating the tie-breaking logic
5. The `<=` comment ("Incumbent wins ties — equal-weight reorgs are unnecessary churn") suggests the `<=` was INTENTIONAL but WRONG for young networks. The comment references "the epoch-boundary fork bug" as justification
6. The test at line 678-721 tests equal-weight tie-breaking correctly, but against the BLOCK-LEVEL comparison, not against `check_reorg_weighted`'s actual `<=` gate

### The Architectural Anti-Pattern

This is the **"Swiss cheese" anti-pattern**: each layer has a hole (dead code, untested integration, misleading comments), and the holes align perfectly to let the bug through. Individually, each piece looks correct:
- The function exists and works
- Tests pass
- The documentation says "fixed"
- The production function has a comment explaining why `<=` is used

But no integration test verifies that the production path actually uses deterministic tie-breaking under equal weights.

---

## Structural Explanation 4: Event-Driven Architecture Without Priority Queuing

**conf(0.65, inferred)**

### What the Code Shows

The event loop in `event_loop.rs` receives ALL network events through a single mpsc channel:
- PeerConnected/Disconnected
- NewBlock (critical for consensus)
- ProducerAnnouncementsReceived (CPU-intensive but not time-critical)
- GSet gossip (bloom filters)
- Identify, Kademlia events (background maintenance)

The drain cap (50 events) before production was added as a mitigation (INC-I-012), but it treats all events equally. Under a connection storm (50 nodes joining), the channel fills with Identify/Kademlia/GSet events. Block events are buried behind non-critical events.

### Missing Invariant

**"Consensus-critical events (NewBlock) must be processed with higher priority than maintenance events (Identify, Kademlia, GSet)."**

The current architecture has no mechanism for this. All events go through the same channel in FIFO order. The drain cap is a blunt instrument — it drains up to 50 events regardless of type, which may or may not include the block events that matter.

### Blast Radius

This is a contributing factor to fork storms, not a root cause. Even with perfect event priority, the `<=` tie-breaking would still reject equal-weight blocks. But event starvation makes the problem worse because:
1. Block events arrive late (behind connection noise)
2. Production runs on stale tip (block not yet processed)
3. Stale-tip production creates additional forks
4. More forks = more fork recovery events = more channel congestion

### Why This Is a Lower-Confidence Explanation

The drain cap at 50 is already a partial mitigation. The gossip-spawning refactor (ProducerAnnouncementsReceived, ProducerDigestReceived spawn to tasks) removed the heaviest non-block events from inline processing. The remaining inline events (PeerConnected, PeerStatus) are lightweight. So while the single-priority channel is architecturally impure, it's already been partially compensated.

---

## Key Questions Answered

### Why does dead code (`should_reorg_by_weight_with_tiebreak`) exist without being wired in?

**Incomplete refactor.** The function was added as part of the REPORT_FORKS fix (commit ca0bd78) alongside the existing `check_reorg_weighted`. The commit log says "Bug 2 (tie-breaker in `check_reorg_weighted` + new `should_reorg_by_weight_with_tiebreak` + 2 tests)" — suggesting the intent was for both to exist. But `check_reorg_weighted` was simultaneously given the `<=` logic with a comment attributing it to preventing "the epoch-boundary fork bug," which overrode the tie-breaking intent. The two changes were semantically contradictory (one adds tie-breaking, the other explicitly prevents it), and the contradiction was not caught because no integration test exercises the gossip-path fork choice under equal weights.

### Why does the fork choice path (gossip) use different logic than fork recovery (`plan_reorg`)?

**Independent development of the two paths.** The gossip path (fast, inline) was designed for the common case (heavier chain always wins, incumbent wins ties). Fork recovery (slow, serialized, active download) was developed later to handle orphan blocks that the gossip path couldn't resolve. Fork recovery added its own tie-breaking heuristic (solo-fork yields) because it operates at a different abstraction level — it knows whether the local fork was self-produced. The gossip path doesn't have this context (it only sees the block, not whether the local chain is solo-produced), so it uses a different rule.

The result is two strategies that address different aspects of the same problem, but the most important one (gossip path, where 99% of blocks arrive) has the weakest tie-breaking.

### Why does connection management have no concept of "bootstrap phase" vs "steady state"?

**Architectural mismatch between node-layer phase awareness and network-layer configuration rigidity.** The sync manager (node layer) has a `is_in_bootstrap_phase()` method that correctly identifies when a node is bootstrapping. But connection limits are set once at `NetworkService::new()` and baked into the swarm via `libp2p::connection_limits::Behaviour`. The swarm runs in a spawned task with command channels for peer operations (Connect, Bootstrap, BroadcastBlock) but NO channel for reconfiguring swarm behaviour parameters. The architecture was designed for static network configuration, not lifecycle-aware adaptation.

### What architectural invariant is missing that would prevent this class of bug?

**Two invariants are missing:**

1. **Fork-choice single-source-of-truth**: There must be exactly ONE function that determines whether chain A should replace chain B, and ALL decision points must call it. Currently there are three different tie-breaking strategies across three call sites.

2. **Connection lifecycle adaptation**: Network-layer parameters that have different optimal values at different node lifecycle phases must be dynamically adjustable, not baked in at construction.

The first invariant is the higher-priority fix. It requires changing ~5 lines in `check_reorg_weighted` (integrate the tie-breaking logic from `should_reorg_by_weight_with_tiebreak`). The second requires either architectural changes to the swarm lifecycle or an operational workaround (staggered startup).

---

## Module Boundary / Data Flow Map

```
                          GOSSIP BLOCK ARRIVES
                                 |
                     +-----------v-----------+
                     |   event_loop.rs       |
                     |   handle_network_event|
                     |   (single mpsc chan)   |
                     +-----------+-----------+
                                 |
                     +-----------v-----------+
                     |  block_handling.rs     |
                     |  handle_new_block()    |
                     |                        |
                     |  prev_hash == tip?     |
                     |    YES -> apply_block()|
                     |    NO  -> fork path    |
                     +------+----------------+
                            |
              +-------------v--------------+
              |  sync_manager              |
              |  handle_new_block_weighted()|
              +-------------+--------------+
                            |
              +-------------v--------------+
              |  reorg.rs                   |
              |  check_reorg_weighted()     |
              |                             |
              |  new_weight <= current?     |
              |    YES -> return None       | <-- RC1: EQUAL WEIGHT REJECTED
              |    NO  -> return ReorgResult|
              +------+-----+---------------+
                     |     |
           (returns None)  (returns Some)
                     |     |
        +------------v-+  +v-----------------+
        | fork recovery |  | execute_reorg()  |
        | (SLOW path)   |  | (FAST path)      |
        | 10s per tick  |  | inline           |
        | solo-fork     |  +------------------+
        | tiebreak ONLY |
        +---------------+

     +------- DEAD CODE -------+
     |  should_reorg_by_weight |
     |  _with_tiebreak()       |
     |  reorg.rs:201           |
     |  Lower hash wins (det.) |
     |  Called from: tests only |
     +--------------------------+
```

Connection layer:
```
     NetworkService::new()
            |
            v
     conn_headroom = max_peers + 10    <-- RC2: FIXED AT CONSTRUCTION
            |
            v
     ConnectionLimits::default()
       .with_max_established_incoming(conn_headroom)
       .with_max_established_outgoing(conn_headroom)
            |
            v
     Swarm::new(transport, behaviour, ...)
            |
            v
     tokio::spawn(run_swarm(...))      <-- NO RECONFIGURATION CHANNEL
            |
            v
     [runs forever with fixed limits]

     Meanwhile, at the node layer:
     sync_manager.is_in_bootstrap_phase()  <-- KNOWS about phases
                                              but CANNOT reach swarm limits
```

---

## Summary of Structural Findings

| # | Finding | Confidence | Root Cause Type |
|---|---------|-----------|----------------|
| S1 | Three separate fork-choice decision points with three different tie-breaking rules — no single canonical function | conf(0.90, observed) | Missing invariant: fork-choice single-source-of-truth |
| S2 | Connection limits fixed at construction; no lifecycle adaptation channel between node layer and swarm layer | conf(0.85, observed) | Architectural mismatch: static config vs. dynamic requirements |
| S3 | Complete solution (`should_reorg_by_weight_with_tiebreak`) exists as tested dead code due to incomplete refactor; no integration test caught the gap | conf(0.80, observed) | Process gap: function added alongside rather than integrated into production path |
| S4 | Single-priority event channel allows maintenance events to delay consensus-critical block processing | conf(0.65, inferred) | Missing priority: no event classification in the channel |

### Fix Priority (Structural View)

1. **S1/S3 (tie-breaking)** — Highest impact, lowest risk, smallest change. Wire the existing `should_reorg_by_weight_with_tiebreak` logic into `check_reorg_weighted`. This is a ~5-line change in reorg.rs. Eliminates fork storms at their source.

2. **S2 (connection lifecycle)** — Medium impact, medium risk. Requires architectural choice:
   - **Simplest**: Stagger startup operationally (no code change)
   - **Simple code change**: Use proportional headroom (`max_peers * 1.5`) as compromise — accepts higher steady-state memory for bootstrap reliability
   - **Proper fix**: Add a `NetworkCommand::UpdateConnectionLimits` variant and rebuild `ConnectionLimits` behaviour dynamically (requires investigating libp2p API for runtime reconfiguration)

3. **S4 (event priority)** — Low priority. Current mitigations (drain cap, spawned gossip tasks) are sufficient. Full fix would require splitting the event channel or adding event classification, which is over-engineering for the current failure rate.
