# Incident Report: Gossip Mesh Dilution — Producer Isolation by Non-Producer Peers

**Date:** 2026-03-14 19:50-19:55 PDT
**Severity:** Critical (recurring forks across all 5 producers, integrity gaps, snap sync triggers)
**Duration:** Ongoing while stress test nodes are connected
**Related:** [Chain Stall Incident](incident-2026-03-14-chain-stall.md), [Stress Test Fork](../incident-2026-03-14-stress-test-fork.md)

## Summary

After starting 50 non-producing stress test nodes that connect to the genesis producers, all 5 producers began experiencing recurring forks. Producers created "solo" blocks that no other producer received, triggering fork recovery cycles. N1 and N4 accumulated integrity gaps of -1219 and -1212 blocks respectively, requiring snap sync recovery.

## Root Cause

**GossipSub mesh dilution by non-producer peers.**

### The Math

| Metric | Value |
|--------|-------|
| Total peers per producer | 50 (max_peers limit) |
| Peers that are producers | ~10 (N1-N5 + seed + some stress at tip) |
| Peers that are stress nodes | ~40 (syncing, non-producing) |
| GossipSub mesh_n | 8 |
| P(0 producers in mesh) | **14.3%** |
| P(at least 1 isolated producer per slot) | **~72%** |

GossipSub selects mesh peers **randomly** from connected peers. With 80% of peer slots consumed by non-producing stress nodes, each producer has a **14.3% chance** of having zero other producers in its eager-push mesh.

When a producer builds a block and gossips it to its mesh, if the mesh contains only stress nodes:
1. Stress nodes receive the block but are syncing and may not forward it quickly
2. Other producers never receive the block via gossip
3. Other producers build their own block for the same/next slot
4. Two competing blocks at the same height → **fork**
5. Fork recovery triggers → rollback → binary search → sometimes snap sync

### Chain of Events

```
Producer N4 builds block 1206
  → Gossips to 8 mesh peers (all stress nodes, no producers)
  → N1, N2, N3, N5 never receive block 1206
  → N3 builds its own block 1206 (different hash)
  → N4 sees N3's block via gossip → fork detected
  → Fork recovery: rollback 1206 → binary search → common ancestor at 1204
  → Apply canonical chain
  → But N4 produced ANOTHER solo block during recovery → new fork
  → Cycle repeats → binary search hits floor → snap sync
  → Snap sync restores state but NOT block store → integrity gap -1212
```

### Evidence

```
N1: 5 solo forks, 25 slot boundary crosses, integrity -1219
N2: 3 solo forks, 26 slot boundary crosses
N3: 3 solo forks, 26 slot boundary crosses
N4: 4 solo forks, 29 slot boundary crosses, integrity -1212
N5: 4 solo forks, 30 slot boundary crosses

N1 peer composition: 10 at tip (producers), 40 behind (stress nodes)
```

## Urgent Production Changes Required

### P0: Producer Peer Reservation (Peer Prioritization)

**Files:** `crates/network/src/service.rs`, `crates/network/src/peer.rs`

**The fix:** Reserve peer slots for producers. When a peer announces itself as a producer (via the status handshake), it should be given priority and protected from eviction. Non-producer peers should be evictable when the peer table is full.

Implementation:
```
max_peers = 50
reserved_producer_slots = 20  // Always room for producers
evictable_slots = 30          // Non-producers can use these

When a new producer connects and peer table is full:
  → Evict lowest-scored non-producer peer
  → Never evict a producer peer for a non-producer
```

**Severity:** CRITICAL — without this, any influx of non-producing nodes (new joiners, light clients, stress tests) dilutes the gossip mesh and causes producer forks. This WILL happen on production mainnet when the network grows beyond 50 nodes.

### P0: GossipSub Score Function for Producer Peers

**File:** `crates/network/src/gossip/config.rs`

GossipSub supports peer scoring to influence mesh composition. Producers should have higher scores so they're preferentially kept in the mesh.

```rust
// In gossipsub config:
peer_score_params.topics.insert(BLOCKS_TOPIC, TopicScoreParams {
    topic_weight: 1.0,
    // Peers that deliver first-seen blocks get high score
    first_message_deliveries_weight: 10.0,
    // Peers that deliver duplicate/late blocks get lower score
    mesh_message_deliveries_weight: -1.0,
    ..Default::default()
});
```

**Severity:** CRITICAL — GossipSub's built-in scoring mechanism exists specifically for this purpose. Without it, mesh composition is random and producers are diluted.

### P1: Automatic Backfill After Snap Sync

**File:** `bins/node/src/node/fork_recovery.rs`

After snap sync restores state, the block store has gaps (blocks from the old fork are gone). Currently this requires manual `backfillFromPeer`. It should be automatic.

**Severity:** HIGH — integrity gaps (-1212, -1219) persist until manually repaired.

### P1: Connection Limit Per Subnet

**File:** `crates/network/src/peer.rs`

The peer diversity system tracks IP prefixes but doesn't enforce hard limits during the connection phase. When 50 nodes connect from the same `/24` subnet (localhost), they all get accepted.

**Severity:** MEDIUM — on production networks, nodes are geographically distributed. But on testnet/local, this amplifies the dilution problem.

## Reproduction

```bash
# Start 5 genesis producers + seed
scripts/mainnet.sh start seed n1 n2 n3 n4 n5

# Wait for stable chain (height 1000+)

# Start 50 non-producing nodes bootstrapping to N1
scripts/stress-batch.sh start 1

# Within 5 minutes: observe forks on explorer
# http://localhost:8080/network.html
# N1-N5 will show FORK, integrity gaps, height mismatches
```

## Lessons

1. **GossipSub mesh is random by default** — it doesn't distinguish between producers and non-producers. The mesh will naturally dilute as the network grows.
2. **max_peers is a hard ceiling** — once saturated, new connections evict random peers. Producers can be evicted in favor of non-producers.
3. **Non-producers in the gossip mesh are gossip sinks** — they receive blocks but may not forward them to other producers (especially if syncing). This breaks the assumption that "gossip = broadcast to all producers."
4. **This is a production-critical issue** — when mainnet grows to 100+ nodes with 12 producers, the same dilution will occur. The probability math scales: with 12 producers and 100 non-producers, P(0 producers in mesh of 8) = 36%.
5. **Every PoS gossip chain has this problem** — Ethereum solved it with validator subnet assignments. Polkadot uses authority discovery. DOLI needs producer peer reservation.
