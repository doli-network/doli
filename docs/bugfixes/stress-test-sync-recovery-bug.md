# BUG REPORT: Sync Recovery Failure Under Concurrent Node Join

**Date**: 2026-03-17
**Version**: 3.7.2 (commit 7259d47)
**Severity**: High
**Component**: `crates/network/src/sync/manager/` (sync_engine, fork_sync, production_gate)
**Affects**: Any node joining a network where competing fork blocks exist

---

## Summary

When 50 non-producing sync-only nodes join a 6-node network (5 genesis producers + 1 seed) simultaneously, 16% of them (8/50) become trapped in an infinite fork sync oscillation loop lasting 30+ minutes. Three `--no-snap-sync` nodes (seed, n3, n4) never recover at all.

**Critical finding**: The 50 nodes are NOT registered producers. They have `--producer` flags but zero on-chain registrations, zero bonds, and produced zero blocks. They are pure sync clients. The failure is entirely in the sync engine's recovery path.

---

## Environment

| Parameter | Value |
|-----------|-------|
| Machine | macOS Darwin 25.2.0, 16 cores, 128 GB RAM |
| Nodes | 1 seed + 5 genesis producers + 50 sync-only nodes = 56 total |
| Network | Local testnet, all on 127.0.0.1 |
| Binary | `doli-node` v3.7.2 (commit 7259d47) |
| Script | `scripts/stress-batch.sh start 1` from `../localdoli` |
| Load avg | Peaked at 24 / 101 / 94 during test |

### Node configuration

| Group | Nodes | Flags | Registered on-chain |
|-------|-------|-------|---------------------|
| Seed | 1 (port 8500) | `--relay-server --no-snap-sync` | N/A |
| Genesis producers | n1-n5 (ports 8501-8505) | `--producer --no-snap-sync --force-start` | Yes (5 registered, bonded) |
| Stress batch | n13-n62 (ports 8513-8562) | `--producer --force-start` | **No** (0 registered, 0 bonds, 0 blocks produced) |

---

## Reproduction

```bash
# 1. Start a healthy 6-node testnet (seed + 5 genesis producers)
#    Let it run until chain reaches h > 100

# 2. Launch 50 sync-only nodes simultaneously
cd ../localdoli
scripts/stress-batch.sh start 1

# 3. Observe: ~84% of nodes sync within minutes. ~16% enter infinite loop.
```

---

## Observed Behavior

### Timeline

| Time (T+min) | Event |
|--------------|-------|
| T+0 | 50 nodes launched. All bootstrap to seed (port 30300). |
| T+3 | ~42/50 nodes fully synced to chain tip (h~499). 8 nodes stuck at h=1-8. |
| T+5 | Chain tip at h~513. Stuck nodes oscillating between fork sync and empty headers. |
| T+15 | Stuck stress nodes still at h=1-2. Seed at h=1-3. n3 at h=6 (slot frozen at 469). |
| T+30 | All 5 stuck stress nodes (n14, n21, n27, n54, n62) finally recover to chain tip. |
| T+45 | n46 recovers (was slow, not stuck). n1 at h=229, slowly recovering. |
| T+60+ | Seed (h=1), n3 (h=6), n4 (DEAD) — **never recover**. |

### Final state at T+45

| Node | Height | Slot | Status |
|------|--------|------|--------|
| seed | 1 | 733 | Stuck forever (`--no-snap-sync`) |
| n1 | 229 | 733 | Recovering very slowly |
| n2 | 851 | 865 | Healthy, producing |
| n3 | 6 | 469 | Stuck forever, **slot frozen** (`--no-snap-sync`) |
| n4 | DEAD | — | RPC unresponsive, stuck in genesis rebuild |
| n5 | 851 | 865 | Healthy, producing |
| n13 | 851 | — | Synced |
| n14 | 851 | — | Recovered after ~30 min |
| n21 | 851 | — | Recovered after ~30 min |
| n27 | 851 | — | Recovered after ~30 min |
| n46 | 851 | — | Recovered (was slow, not stuck) |
| n54 | 851 | — | Recovered after ~30 min |
| n62 | 851 | — | Recovered after ~30 min |
| Other 43 | 851 | — | Synced within first 3-5 min |

---

## Root Cause Analysis

### How sync-only nodes end up on a fork

The 50 nodes don't produce blocks. But during the initial connection stampede:

1. The 5 genesis producers are actively producing blocks
2. Under extreme load (56 processes, load avg 101), block propagation is uneven
3. Some sync nodes receive blocks from different producers at different times
4. If producer A's block at h=1 reaches n14 before producer B's block (which the majority adopted), n14 stores A's block as its chain tip
5. n14 now has a valid h=1 block with a hash that most peers don't share

This is normal in any PoS network. The sync engine should resolve it. **It doesn't.**

### Bug 1: Fork sync weight check rejects equal-weight canonical chain

**File**: `bins/node/src/node/block_handling.rs:568`

```rust
if weight_delta <= 0 {
    info!("Fork sync: new chain not heavier (delta={}, new={}, old={}) — keeping current", ...);
    sync.mark_fork_sync_rejected();
    sync.reset_sync_for_rollback();
    return Ok(());
}
```

Fork sync finds common ancestor at genesis (h=0), downloads 1 canonical block at h=1. The node already has 1 fork block at h=1. Weight comparison:
- Old chain: 1 block, weight = producer_A's effective_weight
- New chain: 1 block, weight = producer_B's effective_weight

When weights are equal (`delta=0`), the reorg is **rejected**. The node keeps its fork block — the one that the rest of the network has moved past.

**The fix should prefer the canonical chain (the one peers agree on) when weights are equal**, or fork sync should download MORE than just the fork depth to ensure the new chain is strictly heavier.

**Observed log pattern** (every ~7 seconds, indefinitely):
```
Fork sync: new chain not heavier (delta=0, new=1, old=1) — keeping current
Empty headers from PEER (peer_h=588, local_h=1, gap=587) — fork evidence (consecutive=1). Blacklisted peer.
Empty headers from PEER (peer_h=495, local_h=1, gap=494) — fork evidence (consecutive=2). Blacklisted peer.
Empty headers from PEER (peer_h=493, local_h=1, gap=492) — fork evidence (consecutive=3). Blacklisted peer.
Fork sync: starting binary search (low=0, high=1, store_floor=0)
Fork sync: ancestor confirmed at height 0
Fork sync: received 1 canonical blocks — reorg ready
Fork sync: new chain not heavier (delta=0, new=1, old=1) — keeping current
[... repeats ...]
```

### Bug 2: Empty headers → blacklist escalation prevents recovery

**File**: `crates/network/src/sync/manager/sync_engine.rs:698-726`

When a node has a fork block at h=1, it sends `GetHeaders { start_hash: <fork_hash> }` to peers. Peers don't have that hash, so they return empty headers. The sync engine:

1. Interprets this as "fork evidence"
2. Blacklists the peer (3 peers per cycle)
3. Resets to `Idle`
4. Eventually blacklists all healthy peers, leaving only other stuck nodes as sync partners

**Compounding effect — stuck nodes poison each other**: n3 (stuck at h=6, port 30303) became a sync partner for n54, n27, and others. When n54 connects to n3:
- n3 reports `peer_h=6` → gap=5 from n54's perspective
- n54 triggers "minor fork" path (gap <= 50)
- Downloads 1 block from n3 (which is also a fork block)
- Reorg fails → back to the loop

This was confirmed: `12D3KooWBTumQm4MM38yyy4TRQZYugAjCQmaRe4wEi1A2WMucxZ6` = n3 (connected via `/ip4/192.168.210.1/tcp/30303`).

### Bug 3: `--no-snap-sync` creates permanent deadlock

**File**: `crates/network/src/sync/manager/production_gate.rs:991-1003`

```rust
if self.snap_sync_threshold == u64::MAX {
    // Snap sync disabled — never allow genesis resync regardless of signals.
    tracing::warn!("--no-snap-sync: suppressing genesis resync signal (local_h={}, gap={})", ...);
    return false;
}
```

For seed, n1, and n3 (`--no-snap-sync`):
- Header sync fails (fork hash unrecognized → empty headers)
- Fork sync fails (equal weight rejection)
- Snap sync is the last resort but is **explicitly disabled**
- No other recovery path exists
- **These nodes are permanently stuck with no way to self-recover**

n3 is the worst case: its `bestSlot` is frozen at 469 (never advanced from first value), suggesting its event loop is saturated processing futile sync requests and GSet merge floods.

### Bug 4: Fork sync downloads insufficient blocks

When the gap is 580+ blocks but the node's fork is only 1-8 blocks deep, fork sync:
1. Finds ancestor at h=0
2. Downloads only as many canonical headers/blocks as the fork depth (1-8)
3. This means the node ends up at h=1-8 after reorg — still 570+ blocks behind
4. Header sync from h=1-8 fails again (same hash mismatch)

Fork sync should download **beyond the fork depth** when the gap is large, to ensure the node has enough of the canonical chain to continue with normal header sync.

---

## The Infinite Loop (composite)

All four bugs interact to create an unbreakable cycle:

```
              ┌─────────────────────────────────────────────────────────┐
              │                                                         │
              ▼                                                         │
         Node at h=1                                                    │
         (fork block)                                                   │
              │                                                         │
              ▼                                                         │
    GetHeaders(start_hash=<fork_hash>)                                  │
              │                                                         │
              ▼                                                         │
    Peers don't recognize hash                                          │
    → Empty headers x3                                                  │
    → Blacklist 3 healthy peers          ◄── Bug 2: escalation          │
              │                                                         │
              ▼                                                         │
    Fork sync triggers                                                  │
    → Find ancestor at h=0                                              │
    → Download 1 canonical block         ◄── Bug 4: insufficient        │
              │                                                         │
              ▼                                                         │
    Weight check: delta=0                                               │
    → Reject reorg, keep fork block      ◄── Bug 1: strict check       │
              │                                                         │
              ▼                                                         │
    Snap sync suppressed                 ◄── Bug 3: --no-snap-sync     │
              │                                                         │
              └─────────── back to top (~7 second cycle) ───────────────┘
```

For nodes WITHOUT `--no-snap-sync`: the loop eventually breaks (after ~30 min) when the blacklist expires or a peer happens to serve useful headers. Recovery is non-deterministic.

For nodes WITH `--no-snap-sync`: the loop **never breaks**. Permanent deadlock.

---

## Impact Assessment

| Scenario | Severity | Risk |
|----------|----------|------|
| Network launch with many simultaneous joiners | **High** | 16% of sync nodes stuck for 30+ minutes |
| Exchange/service adding nodes during active production | **Medium** | Some nodes may sync slowly or get stuck |
| Seed nodes with `--no-snap-sync` | **Critical** | Permanent deadlock, requires manual restart |
| Normal operation with occasional new node | **Low** | Single joiners rarely hit this (no fork contention) |

---

## Proposed Fixes

### Fix 1: Fork sync weight tie-breaking (Bug 1)

In `bins/node/src/node/block_handling.rs:568`:

Change `weight_delta <= 0` to `weight_delta < 0`, or better: when `weight_delta == 0`, prefer the chain that the sync peer (representing network consensus) provided. The node is in fork sync specifically because it knows its chain is wrong — rejecting the canonical chain at equal weight defeats the purpose.

### Fix 2: Fork sync should download beyond fork depth (Bug 4)

When fork sync finds ancestor at h=0 and gap is > 50, download `min(gap, 500)` canonical headers instead of just `fork_depth` headers. This ensures the node gets enough of the canonical chain to continue with normal header sync.

### Fix 3: Limit blacklist escalation (Bug 2)

When ALL peers return empty headers and the node is far behind (gap > 100), this isn't "fork evidence against individual peers" — it means the node itself is on a fork. Instead of blacklisting peers, the node should:
1. Recognize it's the one on the wrong chain
2. Trigger snap sync (or genesis resync) directly
3. At minimum, clear the blacklist after one full cycle

### Fix 4: `--no-snap-sync` needs an alternative recovery path (Bug 3)

When `--no-snap-sync` is set and the node is stuck (repeated fork sync failures, all peers return empty headers), it should:
1. Try requesting headers by **height** instead of hash (if supported by the protocol)
2. Or: wipe local chain data and rebuild from genesis (since `--no-snap-sync` nodes already accept slow sync)
3. Or: at minimum, log a CRITICAL error telling the operator to restart

---

## Test Validity

This stress test is **valid** for the following reasons:

1. **The 50 nodes are pure sync clients** — not registered, no bonds, no blocks produced. `--force-start` and `--producer` flags are irrelevant since the scheduler never assigns them slots.
2. **The failure is in the sync engine**, not block production. Any node joining a network with active producers can receive competing fork blocks.
3. **50 simultaneous joins is realistic** — network launches, exchange integrations, and validator onboarding events create exactly this pattern.
4. **Single-machine limitation doesn't invalidate the finding** — the fork blocks come from genesis producers, not from the stress nodes. The overloaded machine simply increases the probability of uneven block propagation, which occurs naturally over real networks with variable latency.
5. **The `--no-snap-sync` deadlock is 100% reproducible** regardless of machine count or load — any node on a fork with this flag is permanently stuck.

### What the test does NOT prove

- The 16% failure rate is specific to this extreme scenario. Normal node joins (one at a time, adequate resources) are unlikely to hit this.
- The 30-minute recovery time for stress nodes may be shorter on dedicated hardware.
- The GSet merge flood (50 producers announcing to each other) contributed to CPU pressure but is not the root cause of the sync failure.

---

## Raw Evidence

### n54 loop (representative of all stuck stress nodes)

```
16:56:25 Fork sync: new chain not heavier (delta=0, new=1, old=1) — keeping current
16:56:26 Empty headers from PEER (gap=5, consecutive=1) — minor fork.
16:56:31 Fork sync reorg complete: now at height 1
16:56:31 Empty headers from PEER (peer_h=588, local_h=1, gap=587) — fork evidence (consecutive=1). Blacklisted.
16:56:32 Empty headers from PEER (peer_h=495, local_h=1, gap=494) — fork evidence (consecutive=2). Blacklisted.
16:56:33 Empty headers from PEER (peer_h=493, local_h=1, gap=492) — fork evidence (consecutive=3). Blacklisted.
16:56:37 Fork sync: new chain not heavier (delta=0, new=1, old=1) — keeping current
16:56:38 Empty headers from PEER (gap=5, consecutive=1) — minor fork.
16:56:42 Fork sync: new chain not heavier (delta=0, new=1, old=1) — keeping current
[... repeats every ~7 seconds ...]
```

### Seed log (permanent deadlock)

```
16:55:55 Fork sync: ancestor confirmed at height 0, downloading canonical chain
16:55:55 Fork sync: received 3 canonical blocks — reorg ready
16:55:57 Fork sync reorg complete: now at height 1
16:55:58 --no-snap-sync: suppressing genesis resync signal (local_h=1, gap=584)
16:55:58 Empty headers from PEER (peer_h=584, local_h=1, gap=583) — fork evidence (consecutive=1). Blacklisted.
16:55:59 Empty headers from PEER (peer_h=584, local_h=1, gap=583) — fork evidence (consecutive=2). Blacklisted.
[... never recovers ...]
```

### n3 (worst case — slot frozen)

```
16:56:00 [HEALTH] h=6 s=469 | peers=3 best_peer_h=585 net_tip_h=585 | sync_fails=3 state="DownloadingHeaders"
16:56:01 --no-snap-sync: suppressing genesis resync signal (local_h=6, gap=579)
16:56:02 --no-snap-sync: suppressing genesis resync signal (local_h=6, gap=579)
[... slot stuck at 469, never advances ...]
```

---

## Diagnostic Methodology

### RPC format

DOLI uses **JSON-RPC** (not REST). All queries are POST to the root path:

```bash
curl -s --max-time 3 -X POST http://127.0.0.1:$PORT/ \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getChainInfo","params":[],"id":1}'
```

Response fields: `bestHeight`, `bestSlot`, `bestHash`, `network`, `version`.

### Port mapping

| Group | Nodes | RPC Ports | Formula |
|-------|-------|-----------|---------|
| Seed | 1 | 8500 | Fixed |
| Genesis (n1-n5) | 5 | 8501-8505 | `8500 + n` |
| Stress batch 1 (n13-n62) | 50 | 8513-8562 | `8500 + n` |

P2P ports follow the same pattern: `30300 + n`. Metrics: `9000 + n`.

### Full network scan

```bash
# Scan all 56 nodes in one pass
for port in $(seq 8500 8505) $(seq 8513 8562); do
  result=$(curl -s --max-time 3 -X POST http://127.0.0.1:$port/ \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":[],"id":1}')
  h=$(echo "$result" | python3 -c \
    "import sys,json; print(json.load(sys.stdin)['result']['bestHeight'])" \
    2>/dev/null || echo "DEAD")
  s=$(echo "$result" | python3 -c \
    "import sys,json; print(json.load(sys.stdin)['result']['bestSlot'])" \
    2>/dev/null || echo "?")
  hash=$(echo "$result" | python3 -c \
    "import sys,json; print(json.load(sys.stdin)['result']['bestHash'][:12])" \
    2>/dev/null || echo "?")
  echo "port=$port h=$h s=$s hash=$hash"
done
```

### Checking registered producers

```bash
curl -s -X POST http://127.0.0.1:8502/ \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getProducers","params":[],"id":1}' | python3 -m json.tool
```

This confirmed only 5 genesis producers are registered — none of the 50 stress nodes.

### Log analysis

Key log patterns to grep for when diagnosing stuck nodes:

```bash
# Fork sync oscillation (the core loop)
grep "new chain not heavier" ~/testnet/logs/nodes1/n$N.log

# Empty headers blacklisting
grep "fork evidence" ~/testnet/logs/nodes1/n$N.log

# Snap sync suppression (--no-snap-sync nodes only)
grep "suppressing genesis resync" ~/testnet/logs/n$N.log

# Health status (30-second interval)
grep "HEALTH" ~/testnet/logs/n$N.log | tail -5

# Stuck peer poisoning (identify which peer is stuck)
grep "gap=5" ~/testnet/logs/nodes1/n$N.log
# Then cross-reference the peer ID to find it's n3 (port 30303)
```

### Process-level diagnostics

```bash
# Count running nodes
pgrep -la doli-node | wc -l

# Check CPU usage (identify overloaded nodes)
ps -eo pid,pcpu,pmem,command | grep doli-node | sort -k2 -rn | head -10

# System load
uptime

# Verify RPC ports are listening
lsof -i -P -n | grep doli-node | grep LISTEN | head -10
```

---

## Related

- Active decision in memory.db: "Root cause is combinatorial state explosion in SyncManager"
- Previous fixes: 7259d47 (3 root causes of persistent sync cascades)
- Hotspot files: `sync/manager/mod.rs` (20 touches), `cleanup.rs` (15), `production_gate.rs` (15)
