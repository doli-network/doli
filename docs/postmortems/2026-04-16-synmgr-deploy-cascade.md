# Post-mortem: synmgr deploy cascade — 2026-04-16

## Summary

Rolling deploy of `v6.13.29-synmgr` to ai1, ai3, ai5 caused a multi-hour fork cascade affecting 7/15 nodes. No funds lost, no state corruption on the canonical chain. Recovery via wipe+rsync from healthy peers.

**Severity**: High
**Duration**: ~5 hours (deploy at 01:15 UTC → user detection at 06:15 UTC → full recovery by 06:30 UTC)
**Affected nodes**: Seed1, Seed3, N5, N6, N8, N10, N11, Santiago
**Root cause**: rolling restarts created competing fork chains; synmgr `reset_sync_for_rollback REFUSED` bug amplified recovery failures

## Timeline (all times UTC, 2026-04-16)

### Phase 1: Deploy (01:15–01:25)

| Time | Action | Server |
|------|--------|--------|
| 01:15 | Restart Seed1 + N2 with synmgr dropin | ai1 |
| 01:17 | Restart N10, N11, N12 with synmgr dropin | ai5 |
| 01:22 | Restart Seed3, Ivan, Santiago with synmgr dropin | ai3 |
| 01:23 | **Ivan UTXO divergence detected** (sr mismatch, 37KB less UTXO data) — wipe+resync fixed immediately | ai3 |

8 nodes restarted across 3 servers in ~10 minutes. All came back online and synced. Network appeared healthy.

### Phase 2: Fork block production (01:21–03:55)

FORK_GUARD correctly dropped competing blocks at occupied heights. Same fork block hashes appeared across multiple nodes simultaneously — a single producer was emitting fork blocks.

| Time | Fork block hash | Height | Seen by |
|------|----------------|--------|---------|
| 01:21 | `1fa2ab77` | 38,235 | Seed3, N5, N6, N11 |
| 01:39 | `1f441eb0` | 38,343 | all |
| 01:46 | `d674ff70` | 38,379 | all |
| 01:58 | `090354f9` | 38,452 | all |
| 02:04 | `749f1992` | 38,488 | all |
| 02:53 | `21664c10` | 38,781 | all |
| 03:49 | `bf3d087b` | 39,113 | all |
| 03:55 | `b2e96b46` | 39,148 | all |

FORK_GUARD handled these correctly — no state damage during this phase. Fork frequency: one every ~20-50 min.

### Phase 3: Fork escalation (04:59–06:17)

At h=39,527 the pattern shifted from FORK_GUARD drops to actual rollbacks:

| Time | Node | Event | Height |
|------|------|-------|--------|
| 04:59 | Seed3 | First rollback (stuck_fork_signal, gap=2) | 39,527→39,526 |
| 05:11 | Seed3 | Second rollback (gap=5) + `REFUSED` | 39,598→39,597 |
| 05:11 | N5 | First rollback (gap=1) | 39,602→39,601 |
| 05:11 | N5 | Repeated FORK_GUARD: fork block `60299040` at h=39,600 (5x) | 39,600 |
| 05:21 | N11 | First rollback (gap=1) | 39,657→39,656 |
| 05:28 | N6 | First rollback (gap=1) | 39,696→39,695 |
| 05:28 | N5 | Second stuck_fork_signal (gap=2) | 39,696 |
| 06:17 | Seed3 | **Cascade**: rollback_count=26, h=39,963 | 39,963→39,962 |
| 06:17 | N5 | Cascading rollbacks: 39,962→39,961→39,960 | 39,962–39,960 |

Critical log from Seed3 at 06:17:
```
[RECOVERY] reset_sync_for_rollback REFUSED: height 39962 at or below floor 39963
```

Critical log from N5 at 06:17:
```
[RECOVERY] reset_sync_for_rollback REFUSED: height 39960 at or below floor 39963
```

### Phase 4: Detection and recovery (06:15–06:30)

| Time | Action |
|------|--------|
| 06:15 | User detects fork via explorer (screenshot: multiple heights, hash divergence) |
| 06:17 | Investigation reveals rollback loops + `REFUSED` bug |
| 06:20 | Wipe+rsync N10, N11 from N12 (ai5) |
| 06:22 | Wipe+rsync N6, N8 from N7 (ai4) |
| 06:24 | Wipe+rsync Seed1 from N1 (ai1) |
| 06:25 | Wipe+rsync Seed3 from N1 via ai1 (ai3) |
| 06:27 | Wipe+rsync N5 from N4 (ai2) |
| 06:30 | All 15/15 synced, sr matching |

## Root cause analysis — forensic detail

### The trigger block: h=39,527

At **04:58:34 UTC**, Seed3 applied a block at h=39,527 from producer `d13ae338` (slot 40840):

```
04:58:34 [BLOCK] Applied h=39527 hash=0d38e3cd producer=d13ae338 slot=40840
```

But the canonical chain (majority) had a DIFFERENT block at h=39,527 from producer `d4b5451b` (slot 40841):

```
04:59:23 [BLOCK] Applied h=39527 hash=8d366e17 producer=d4b5451b slot=40841
```

Two producers built valid blocks at h=39,527, arriving at different nodes in different order. Seed3 chose the wrong one.

### Why two blocks at the same height?

The deploy restarts (01:15-01:25 UTC) took 8 producers offline briefly. When they came back:
- Some caught up and produced blocks for their scheduled slots
- Other producers already filled those slots during the ~5s downtime
- Both blocks are valid — which one wins depends on gossip arrival order

FORK_GUARD correctly dropped 8 fork blocks from 01:21 to 03:55 at various heights. The system was working. But at h=39,527, the fork block arrived BEFORE the canonical one to Seed3, so Seed3 applied it as its chain tip.

### The first rollback — successful recovery (04:59)

```
04:59:00 [HEALTH] h=39527 gap=0 — but net_tip_s=40842 vs local_s=40840
04:59:00 [SYNC_STATE] gap=1 last_applied_ago=25s
04:59:00 [COORDINATOR] shadow action=HeaderFirstSync gap=1

04:59:14 [SYNC] 3 orphan gossip blocks (local_h=39527, tip_h=39529, gap=2) —
         recently synced (40s since last apply), suppressing sync escalation.
         Signaling stuck fork for shallow rollback (anti-cascade-orphan).

04:59:15 [FORK] stuck_fork_signal consumed (gap=2, last_applied=40s)
04:59:15 [ROLLBACK] Initiating: depth=1 local_h=39527 target_h=39526
04:59:15 [ROLLBACK] Restored epoch state from undo: epoch=109
04:59:15 [FORK] ROLLBACK_DONE h=39526

04:59:23 [BLOCK] Applied h=39527 hash=8d366e17 producer=d4b5451b slot=40841 ← CORRECT block
04:59:23 [BLOCK] Applied h=39528, h=39529, h=39530 ← caught up in burst

04:59:30 [SYNC_STATE] gap=0 phase="Idle" ← RECOVERED
```

**The first rollback worked perfectly.** Seed3 rolled back 1 block, re-applied the canonical chain, and caught up. No damage.

### The cascade — rollback loop (05:11)

12 minutes later, the SAME PATTERN repeated at h=39,598. But this time the recovery failed:

```
05:11:04 [SYNC] 3 orphan gossip blocks (local_h=39598, tip_h=39601, gap=3) —
         applied since last rollback (rb_h=39526) → BEHIND not forked.
         Suppressing stuck_fork signal, running normal sync.

05:11:15 [FORK] stuck_fork_signal consumed (gap=3, last_applied=40s)
         ← signal NOT suppressed this time — bypassed by anti-cascade-orphan path

05:11:34 [FORK] ROLLBACK h=39598→39597 rollback_count=2
05:11:35 [FORK] ROLLBACK h=39597→39596 rollback_count=3  ← 1 second later!
05:11:45 [FORK] ROLLBACK h=39596→39595 rollback_count=4
05:11:55 [FORK] ROLLBACK h=39595→39594 rollback_count=5
05:12:05 [FORK] ROLLBACK h=39594→39593 rollback_count=6  gap=12!
05:12:15 [FORK] ROLLBACK h=39593→39592 rollback_count=7  gap=13!
05:12:25 [FORK] stuck_fork_signal consumed gap=14 ...
```

**The rollback loop**: every 10 seconds, a new stuck_fork_signal fires. Each one rolls back 1 more block. The gap INCREASES (2→3→5→7→8→12→13→14) because the canonical chain advances while Seed3 keeps rolling backward. The node can never catch up — it's running backward while the network runs forward.

### Why didn't the second rollback recover like the first?

The first rollback (04:59) recovered because:
1. Rolled back 1 block
2. Sync retry found the correct chain
3. Applied correct blocks in burst

The second rollback (05:11) cascaded because:
1. The node was on a fork that diverged at a DIFFERENT point
2. Each rollback only removes 1 block, but the fork may be deeper
3. The `anti-cascade-orphan` path bypasses the suppression logic
4. No backoff between rollbacks — stuck_fork_signal fires every tick

This is the **core design flaw**: the rollback mechanism assumes 1-block forks. When the fork is deeper, it enters a loop: rollback 1 → still forked → rollback 1 more → still forked → gap grows.

### Amplifier: `reset_sync_for_rollback REFUSED`

By 06:17, the cascade reached h=39,963 with `rollback_count=26`:

```
06:17:32 [ROLLBACK] Initiating: depth=1 local_h=39963 target_h=39962
06:17:32 [RECOVERY] reset_sync_for_rollback REFUSED: height 39962 at or below floor 39963
06:17:32 [FORK] ROLLBACK_DONE h=39962
```

The sync manager's floor stayed at 39,963 after the rollback to 39,962. The node rolled back but couldn't re-request the correct blocks from peers — permanently stuck.

### Contributing factor: Ivan UTXO divergence

Ivan's UTXO set was 37,121 bytes smaller than Seed3's at the same height after restart. Same block hash, different state root. Probable cause: restart interrupted mid-`apply_block()`, partial UTXO write committed to disk.

This was detected and fixed immediately (wipe+resync), but demonstrates that the restart procedure can corrupt state if timing is unlucky.

## Node status at detection (06:15 UTC)

| Node | Height | Gap | Version | Issue |
|------|--------|-----|---------|-------|
| N1-N4, N7, N9, N12, Ivan | 39,960-39,975 | 0 | mixed | healthy |
| Seed1 | 39,950 | -25 | synmgr | rollback loop |
| Seed3 | 39,963 | stuck | synmgr | REFUSED + rollback_count=26 |
| Santiago | 39,962 | -13 | synmgr | behind |
| N5 | 39,960 | cascading | 6.13.28 | rollback loop |
| N6 | 39,949 | -26 | 6.13.28 | rollback loop |
| N8 | 3,151 | -36,824 | 6.13.28 | broken (resyncing from genesis) |
| N10 | 39,974 | -1 | synmgr | slight behind |
| N11 | 39,947 | -28 | synmgr | rollback loop |

## Impact

- **Funds**: none lost
- **Chain integrity**: canonical chain unaffected (majority of producers continued normally)
- **Missed slots**: estimated ~50-100 missed slots across affected producers over 5 hours
- **Block archive gaps**: multiple nodes had integrity gaps requiring backfill
- **Recovery**: full wipe+rsync from healthy peers on same server

## Bugs identified

### BUG-1: `reset_sync_for_rollback REFUSED` (synmgr branch)

**File**: `crates/network/src/sync/manager/block_lifecycle.rs` (estimated)
**Severity**: High
**Description**: After rollback, sync floor is not lowered to match new height. Node is stuck: rolled back but cannot re-request blocks.
**Fix**: reset floor to `rollback_target_height - 1` after successful rollback.

### BUG-2: UTXO corruption on restart mid-apply

**Severity**: Medium
**Description**: If systemd SIGTERM arrives during `apply_block()`, partial UTXO writes may be committed. RocksDB batch atomicity should prevent this, but Ivan's case shows 37KB of missing UTXOs.
**Status**: needs investigation — may be pre-existing, may be synmgr-specific.

## Lessons

1. **Rolling deploy spacing must be wider**. 10 minutes for 8 nodes is too fast. Minimum 1-2 minutes between individual node restarts, verify sync before proceeding.
2. **Never restart seeds and producers in the same batch**. Seeds should be last (they're relay/archive, not producers).
3. **The `REFUSED` floor bug must be fixed before scaling synmgr to remaining servers** (ai2, ai4, external nodes).
4. **Wipe+rsync from a healthy same-server peer is the fastest recovery** (~10s per node, zero downtime for the donor).
5. **N8 at h=3,151 shows catastrophic regression** — a node on 6.13.28 (main branch) got so confused by the fork cascade that it appears to have resynced from genesis. Needs deeper investigation.

## Action items

- [ ] Fix `reset_sync_for_rollback REFUSED` in synmgr branch
- [ ] Investigate N8 genesis-level regression
- [ ] Investigate Ivan UTXO corruption mechanism
- [ ] Update deploy procedure: min 60s between node restarts, seeds last
- [ ] Add integrity check to post-deploy verification script
