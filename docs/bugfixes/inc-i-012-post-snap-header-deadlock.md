# INC-I-012: Post-Snap Sync Header Deadlock

**Date**: 2026-03-25
**Status**: Root cause found, fix designed
**Branch**: fix/sync-state-explosion-root-causes
**Severity**: Critical — 5+ nodes permanently stuck, cannot recover without restart
**Affected**: Any node that snap syncs when peers have divergent blocks at snapshot height

## Symptoms

After snap sync completes, header-first sync fails forever:
```
Starting sync epoch 92 → GetHeaders(start_hash=ef787bef) → empty response → 30s timeout → repeat
Genesis resync REFUSED: snap attempts exhausted (3/3)
```
Node stuck at snap height (h=240), gap grows indefinitely.

## Root Cause

**Snap sync accepts a snapshot from a peer that is on a fork, producing a local_hash that no full node has in its canonical chain.**

### Causal Chain

1. Fresh node at h=0, gap=244. Snap sync activates.
2. Quorum votes for `target_hash=cecbde79` at h=244 (majority-preferred).
3. Snapshot arrives from peer `12D3KooWCNN...` at **h=240, hash=`ef787bef`** — 4 blocks behind target, and a fork block.
4. State root matches (fork hadn't diverged in state), so snapshot is accepted.
5. Node now has `local_hash=ef787bef` at h=240. No block history.
6. Header-first sync sends `GetHeaders(start_hash=ef787bef)` to best peer.
7. Best peer (seed) has canonical hash `9f4efe01` at h=240 — **different hash**. Does not recognize `ef787bef`.
8. Seed responds with empty headers.
9. Node interprets empty as "fork detected", retries snap → snap attempts exhausted (3/3) → stuck.

### Proof

```
Seed canonical block at h=240:  9f4efe018c21...
Snap-synced node hash at h=240: ef787beff826...  (DIFFERENT — fork block)
Seed getBlockByHash(ef787bef):  "Block not found"
```

### Why the snapshot was accepted from the wrong height/hash

The snap sync protocol:
1. Sends `GetStateRoot` to all peers (batch)
2. Collects votes, groups by (height, state_root)
3. Picks majority state root → selects quorum peer
4. Sends `GetStateSnapshot` to quorum peer
5. **But**: the quorum peer's snapshot may be at a DIFFERENT height than the target (it reports its own best height, not the target height)
6. State root verification passes because the fork hadn't caused state divergence at h=240

## Recommended Fix: GetHeadersByHeight Fallback

**Approach**: When `GetHeaders` returns empty after snap sync, fall back to height-based header request instead of hash-based.

### Design

Add `GetHeadersByHeight { start_height: u64, max_count: u32 }` to the sync protocol:

```rust
// In crates/network/src/protocols/sync.rs
SyncRequest::GetHeadersByHeight {
    start_height: u64,
    max_count: u32,
}
```

**Server side** (`handle_sync_request` in `validation_checks.rs`):
- Look up canonical block at `start_height` via `get_hash_by_height()`
- Serve headers from `start_height + 1` (same logic as GetHeaders, but starting from height instead of hash)

**Client side** (`sync_engine.rs`):
- After snap sync, when GetHeaders returns empty (the "Post-snap empty headers" path), retry with `GetHeadersByHeight(local_height)` instead of retrying snap sync
- This bypasses the hash lookup entirely — the peer uses its OWN canonical hash at that height

### Why this works

- The peer looks up its canonical block at h=240 (which is `9f4efe01`), then serves headers from h=241+
- The requesting node receives headers that build on a different hash than its local_hash, BUT the state is identical (same state root)
- `apply_block()` on the first received block will succeed because state roots match
- After applying one block, local_hash updates to the canonical chain and everything chains normally

### Why alternatives are worse

| Alternative | Problem |
|---|---|
| Enforce snapshot hash == quorum target | Too restrictive — peers may not have snapshot at exact target height |
| Reset snap.attempts on restart | Band-aid — re-snaps to same non-canonical hash |
| Use genesis hash for GetHeaders | Re-downloads entire chain from h=0, defeats snap sync purpose |
| Only snap from seed nodes | Doesn't scale, seed becomes bottleneck |

## Files to Modify

1. `crates/network/src/protocols/sync.rs` — Add `GetHeadersByHeight` variant to `SyncRequest`
2. `bins/node/src/node/validation_checks.rs` — Handle `GetHeadersByHeight` in `handle_sync_request()`
3. `crates/network/src/sync/manager/sync_engine.rs` — In the "Post-snap empty headers" path, retry with height-based request instead of retrying snap sync

## Additional Finding: P3/P4/P5 Quick Wins Deployed

Three sync performance fixes were implemented and deployed during this session:

1. **P3** (`periodic.rs`): Changed `if let` → `while let` loop for sync requests — 4-8x body download throughput
2. **P4** (`sync_engine.rs`): Preserve valid downloaded headers/bodies across sync restart — eliminates "20-30K headers nuked every 30s"
3. **P5** (`event_loop.rs`): Send `SyncResponse::Error("busy")` instead of silently dropping rate-limited requests — eliminates 30s timeout waste

**Result**: Network went from 28/66 synced to 60/66 synced after deploying new binary. The 5 remaining stuck nodes are all affected by this post-snap deadlock bug.

## Test Plan

1. Start a 50+ node testnet, let it reach h=200+
2. Add 30 fresh nodes (h=0, gap > 50 triggers snap sync)
3. Verify fresh nodes snap sync AND catch up to tip within 60s
4. Kill 10 nodes, wait 100 slots, restart — verify they re-snap and catch up
5. Verify no node gets stuck in the "empty headers after snap" loop
