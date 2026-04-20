# Post-mortem: ORPHAN_FORK Rollback Cascade (2026-04-19)

**Date**: 2026-04-19 07:00–10:30 UTC  
**Severity**: High  
**Duration**: ~3.5 hours  
**Impact**: Multiple nodes stuck behind, repeated manual recovery (rsync), ~200 missed slots  
**Versions involved**: v6.17.2 (stable), v6.17.3 (fix attempt, worsened the problem)  
**Root cause**: Aggressive rollback on fork blocks from external producers  

---

## Timeline

| Time (UTC) | Event |
|-----------|-------|
| 03:55:28 | Fork block at slot 23811 arrives. N1 has h=23672 (slot 23810), N2 doesn't yet. N2 applies 23811 as h=23672. N1 rejects as orphan (parent mismatch). N1 enters sync stuck. |
| 03:56:30 | N1 `gap=1, last_applied_ago=31s`. Sync manager activates. |
| 03:57:00 | N1 `gap=2, Syncing:Headers, last_applied_ago=61s`. Production blocked (`BlockedSyncing`). |
| 04:14:00 | N1 `gap=27, last_applied_ago=838s`. 14 minutes stuck. ~24/40 producers affected. |
| 04:15–04:44 | ~200 missed slots. Only ~16 producers active during this window. |
| 04:44 | Network stabilizes. Affected nodes catch up via sync. |
| 06:15 | Explorer shows integrity failures: 9/15 nodes with ✗ -47 to -51. |
| 06:21–06:34 | Manual recovery: rsync from good nodes to fix integrity. |
| 07:25 | N4 (ai2) forks at h=24763. Fork block `878078d5` from producer `b96f86f0` (slot 25069 vs canonical 25068). N4 stuck in ORPHAN_CHASE loop. |
| 07:44 | N9, N11 (ai5) fork at h=24860. Fork block `aa56d71a` from producer `ac7b3e0e` (slot 25185 vs canonical 25184). |
| 07:58 | v6.17.3 deployed to ai3 (canary) with 3 fixes. Includes ORPHAN_FORK auto-rollback. |
| 08:00–09:30 | v6.17.3 rolling deploy to ai1-ai5. |
| 09:30 | Explorer shows widespread BEHIND: N1 -35, N2 -5, N3 -28, Seed3 -23, N5 -19, N6 -6. v6.17.3 made the problem significantly worse. |
| 09:45–10:15 | Emergency rollback to v6.17.2 on all servers. Multiple rsync recoveries. |
| 10:15 | All nodes recovered on v6.17.2. Network stable. |

---

## Root Cause

### The original problem (pre-v6.17.3)

External producers (`b96f86f0`, `8eca5332`, `fe56bf16`, `ac7b3e0e` — all 5-10 bonds) are consistently 1 block behind the network. When their scheduled slot arrives, they produce a block on top of height N-1 instead of N, creating a competing block for the same height as the canonical chain.

This is caused by network latency: these producers receive blocks ~1-2 seconds late, and by the time the block arrives, their production window has already started. They produce on the stale tip.

When other nodes receive these fork blocks:

1. If the node hasn't applied a block at that height yet: it applies the fork block. Later, the canonical block arrives but can't be applied (height occupied). The node is now on a 1-block fork.
2. If the node already has the canonical block: FORK_GUARD drops the fork block silently. No harm.

Nodes in case 1 get stuck because:
- The next canonical block has `prev_hash` pointing to the canonical block at height N, not the fork block.
- ORPHAN_CHASE fires, requesting `GetBlockByHeight(N+1)`, but the response has a `prev_hash` that doesn't match the node's block at height N.
- The request repeats infinitely — the node never considers that its own block is wrong.

**Evidence** (N1 log, `/var/log/doli/mainnet/n1.log` on ai1):
```
03:55:18 — Applied h=23672 hash=29fa5944... slot=23810 (canonical)
03:55:28 — GOSSIP_RECV slot=23811 hash=7ce20975... → APPLY_START, no APPLY_END (orphan)
```

**Comparison** (N2 log — same time, different outcome):
```
03:55:28 — Applied h=23672 hash=7ce20975... slot=23811 (fork block applied first)
```

N1 and N2 have **different blocks at h=23672**. Same canonical chain tip eventually, but different block store contents at intermediate heights. The chain commitment diverges permanently from that point.

### How v6.17.3 made it worse

v6.17.3 included 3 fixes:

1. **rollback.rs** — `delete_chain_commitment()` on rollback (defensive, correct)
2. **backfill.rs** — `get_chain_commitment().is_none()` guard (defensive, correct)  
3. **block_handling.rs** — ORPHAN_FORK: if orphan's `prev_hash != local_hash`, rollback 1 (DESTRUCTIVE)

Fix #3 was designed to solve the ORPHAN_CHASE infinite loop: instead of repeating the request, detect the fork and rollback. The logic:

```rust
if let Ok(Some(our_tip)) = self.block_store.get_block_by_height(current_height) {
    if our_tip.hash() != block.header.prev_hash {
        // Fork detected — rollback
        self.rollback_one_block().await?;
    }
}
```

**The flaw**: this triggers on ANY orphan block whose `prev_hash` doesn't match the local tip. It doesn't check whether the LOCAL chain is wrong or the ORPHAN is wrong. In most cases, the orphan is a fork block from a late external producer — the local chain is correct.

**Cascade evidence** (N1 log, `/var/log/doli/mainnet/n1.log` on ai1):

190 ORPHAN_FORK rollbacks in 19 minutes (08:05–08:24 UTC). A single `prev_hash` (`b461a1bf`) from one fork node triggered consecutive rollbacks:

```
08:05:38 — ORPHAN_FORK h=24971 → rollback
08:05:48 — ORPHAN_FORK h=24970 → rollback
08:05:56 — ORPHAN_FORK h=24969 → rollback
08:05:57 — ORPHAN_FORK h=24968 → rollback
08:05:58 — ORPHAN_FORK h=24967 → rollback
... (190 total, descending)
```

The same 3 `prev_hash` values repeat (`09dee1e3`, `8f08e706`, `236ecc6f`) — just 3 orphan blocks from fork nodes circulating via gossip. Each time they arrive, the node has rolled back one more block, so the mismatch check still fails → another rollback → one more block behind → the orphans arrive again via gossip re-delivery → infinite loop.

**The `prev_hash` values never existed in the canonical chain.** They came from fork nodes hundreds of blocks behind producing their own chain. The fix blindly assumed that a `prev_hash` mismatch meant the LOCAL chain was wrong, when in fact the orphan was from a completely divergent chain.

**Why 10x worse**: each rollback made the node more vulnerable to the next fork block (more behind → more orphans arriving → more rollbacks). A self-reinforcing destruction loop. One fork node could destroy 190 canonical blocks in 19 minutes on any core node.

---

## Chain Commitment Corruption Mechanism

The chain commitment is computed incrementally:
```
commitment[h] = BLAKE3(commitment[h-1] || block_hash[h])
```

When two nodes apply different blocks at the same height (fork), their commitments diverge permanently — even if the block stores eventually converge to the same canonical chain. The commitment was computed with the original (possibly fork) block's hash, and is never recalculated.

The startup integrity scan (periodic.rs) recomputes from h=1, producing the correct value. But the incremental commitment in post_commit.rs continues from wherever it was, perpetuating the divergence.

**Evidence**: 9/15 nodes showed integrity ✗ with offsets -47 to -51 after the 03:55 fork event. Block stores were identical across all nodes (confirmed via RPC `getBlockByHeight`), but commitments differed.

---

## Fork Block Producers

| Producer | Bonds | Fork blocks | Slots |
|----------|-------|-------------|-------|
| `b96f86f0` | 5 | h=24763 | s=25069 (canonical 25068) |
| `8eca5332` | 10 | h=24789 | s=25098 (canonical 25097) |
| `fe56bf16` | 5 | h=24805 | s=25119 (canonical 25117) |
| `ac7b3e0e` | 10 | h=24860 | s=25185 (canonical 25184) |

All are external producers with few bonds and higher network latency. They consistently produce 1 slot late, creating competing blocks at already-filled heights.

---

## What Worked

- FORK_GUARD correctly dropped fork blocks when the node already had the canonical block
- Manual recovery via rsync restored nodes consistently
- Per-node binaries enabled rolling restart without affecting sibling nodes
- The attestation/rewards fixes (v6.17.0, v6.17.1) remained stable throughout

## What Failed

- ORPHAN_CHASE loops infinitely when local block differs from canonical (pre-existing)
- Sync manager enters `BlockedSyncing` state during ORPHAN_CHASE, blocking production (pre-existing)
- Fix #3 (ORPHAN_FORK) rolled back correct blocks, causing cascade (v6.17.3)
- Fix #1 amplified fix #3's damage by deleting commitment on each unnecessary rollback (v6.17.3)
- No mechanism to distinguish "we're on a fork" from "the orphan is from a fork"

---

## Recovery Actions

1. Rolled back all servers (ai1-ai5) from v6.17.3 to v6.17.2
2. Rsync'd block stores from known-good nodes to affected nodes
3. `systemctl reset-failed` for services that hit StartLimitBurst

---

## Remaining Issues

1. **ORPHAN_CHASE infinite loop**: still exists in v6.17.2. When a node applies a fork block first, it gets stuck requesting the canonical block but can't apply it. No automatic recovery.

2. **Chain commitment corruption**: still exists in v6.17.2. Fork blocks produce a different commitment that persists even after the block store converges to canonical.

3. **External producer fork blocks**: structural. Late producers create competing blocks. Cannot be eliminated without production gate improvements.

---

## Proposed Fixes (not yet implemented)

### 1. ORPHAN_CHASE fork detection (corrected)

Before rolling back, verify with peers:
- Query ≥3 peers for their block hash at `current_height`
- If majority differs from local → rollback (we're on the fork)
- If majority matches local → drop the orphan (it's a fork block)

This distinguishes "we're wrong" from "they're wrong" without the aggressive unconditional rollback.

### 2. Production stale-tip guard

Before producing, check `last_applied_ago`. If > 15s, skip the slot — the producer is behind and will create a fork block. Prevents the fork blocks at their source.

### 3. Chain commitment recalculation

After any block replacement in the block store (sync recovery), mark the commitment as dirty. On the next periodic tick, recompute from the divergence point.

---

## Lessons

1. **Fork recovery must not assume the local chain is wrong.** The majority of orphan blocks are fork blocks from late producers, not evidence that the local chain diverged. Any rollback triggered by an orphan must verify with multiple peers first.

2. **Defensive fixes can amplify damage.** Fix #1 (delete commitment on rollback) was correct in isolation but amplified fix #3's cascade — each unnecessary rollback also destroyed the commitment.

3. **Test fork recovery with adversarial producers.** The ORPHAN_FORK fix was tested against the N4 stuck scenario but not against constant fork block gossip from external producers. The production environment has 4+ producers consistently generating fork blocks.

4. **Rolling deploy of consensus changes needs longer canary.** v6.17.3 was deployed to ai3, briefly verified, then rolled to all servers within 90 minutes. The cascade only became visible after 30+ minutes of operation under fork block pressure.
