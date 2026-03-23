# Incident Report: N1 Fork During 50-Node Stress Test

**Date:** 2026-03-14
**Severity:** Medium (single producer forked, chain continued via N2-N5)
**Duration:** ~10 minutes (auto-recovered to canonical tip, required backfill for block store)

## Summary

Starting 50 stress-test nodes simultaneously on the same host caused genesis producer N1 to fork at the block store level. N1 continued producing on the canonical chain tip but lost blocks 1-593 from its local store, resulting in a block integrity gap of -593.

## Timeline

| Time | Event |
|------|-------|
| T+0s | 50 nodes launched, all bootstrapping to N1 (port 30301) |
| T+5s | All 50 nodes initiate header-first sync against N1 simultaneously |
| T+10s | N1 saturated: 50 concurrent `GetBlockByHeight` requests competing for disk I/O and CPU |
| T+15s | N1's sync responses slow down; gossip block propagation from N2-N5 stalls at N1 |
| T+30s | N1 falls behind peers (Layer 6.5 blocks production correctly) |
| T+50s | Stress nodes discover N2-N5 via DHT/gossip, begin syncing from them instead |
| T+60s | N1 enters fork recovery — binary search finds common ancestor at height 0 |
| T+90s | N1 recovers to canonical tip via fork sync from N2-N5 |
| T+90s | N1 block store missing blocks 1-593 (overwritten during fork recovery) |
| T+10m | Manual `backfillFromPeer` from N2 restores all 593 blocks |

## Root Cause

**Resource starvation on bootstrap node.** When 50 nodes simultaneously bootstrap to a single producer:

1. **Inbound connection storm**: 50 TCP connections + libp2p handshakes + gossipsub mesh joins within seconds. N1's `max_peers=50` limit was hit, causing connection churn.

2. **Sync request amplification**: Each of the 50 nodes requests the full block history (500+ blocks) via `GetBlockByHeight` one at a time. That's 50 × 500 = 25,000 RPC-over-libp2p requests hitting N1's event loop.

3. **CPU contention**: All processes share 16 cores. N1's block production thread competes with its sync-serving threads and 50 other processes doing VDF verification. Block production misses slots.

4. **Gossip starvation**: N1 is so busy serving sync requests that it can't process incoming gossip blocks from N2-N5. The gossip activity watchdog sees silence, and Layer 6.5 correctly blocks N1 from producing (lag > 3 blocks).

5. **Fork recovery path**: When N1 finally processes peer updates showing height 500+, fork sync triggers. Binary search finds common ancestor at height 0 (N1 had produced a few orphan blocks before being blocked). Fork sync replaces N1's block store from height 0, but only downloads the range needed — the intermediate blocks are lost.

## Impact at Scale (300+ Nodes)

With 300+ nodes reorging simultaneously on a single host:

- **Cascading fork recovery**: If multiple producers fall behind and fork, each one entering fork sync generates additional `GetBlockByHeight` traffic to the surviving producers, creating a feedback loop.
- **Disk I/O saturation**: 300 nodes all writing rollback undo data + re-applying blocks = sequential disk writes competing for the same SSD. RocksDB compaction stalls.
- **Gossip mesh collapse**: GossipSub `mesh_n=20` means each node maintains 20 eager-push peers. With 300 nodes, the mesh reconfigures constantly as nodes fork/recover, causing message duplication and missed blocks.
- **Block store corruption risk**: Fork recovery overwrites block store ranges. If two producers fork simultaneously with different orphan chains, their block stores diverge until backfill repairs them.

## Fixes Applied

1. **Layer 6.5 rewrite** — Production unconditionally blocked during active sync (`is_syncing() || fork_sync.is_some()`) and when lag > 3 blocks. Removed the 60s timeout escape that allowed production at any lag size (the original cause of infinite reorg loops).

2. **Circuit breaker relaxed** — `max_solo_production_secs` set to 86400 for local dev (was 50s). Localhost gossip isn't tracked as "received via gossip" so the breaker was halting the entire chain after 50s.

## Recommendations

1. **Bootstrap fan-out**: Stress nodes should bootstrap to multiple producers (not just one), distributing the sync load. Use round-robin: batch 1 → N1, batch 2 → N2, etc.
2. **Staggered startup**: Launch nodes in waves (10 per second) rather than all at once to avoid connection storms.
3. **Sync rate limiting**: Implement per-peer sync request rate limiting in the network layer to prevent a single node from being overwhelmed.
4. **Block store backfill on recovery**: After fork sync, automatically detect and backfill missing block store ranges from peers instead of requiring manual `backfillFromPeer`.
