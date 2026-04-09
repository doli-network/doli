# Forensic Report: N8 ai4 Repeated Behind/Recover Fork Cycle

**Incident**: INC-I-024
**Date**: 2026-04-06
**Analyst**: Claude (log forensics)
**Confidence**: HIGH (95%)

## Executive Summary

**Root cause**: Post-snap-sync attestation filter bug produced `eligible_len=1` (should be 21), causing scheduler divergence. The node ran fine for 10+ hours receiving blocks via gossip, but a burst of 43 rate-limited blocks at 13:48 UTC triggered a behind/rollback cascade that the node could not self-recover from.

## Timeline

| Time (UTC) | Event | Evidence |
|------------|-------|----------|
| 02:06:45 | N8 started via snap sync (fresh node, gap=777) | `[STARTUP] Block store floor set to 0 (snap sync gap)` |
| 02:06:45 | Block store: 5 genesis producers, StateDb: will grow to 6 | `Initial epoch bond snapshot: 5 producers` |
| ~02:30 | Snap sync completes, node begins following chain | state=Synchronized |
| 03:51:14 | **N8 RESTARTED** (--yes --force-start) | `Shutting down gracefully...` then `DOLI Node v6.7.1` |
| 03:51:15 | **INTEGRITY_RISK**: genesis producer mismatch (6 vs 5) | `StateDb has 6 producers but block store has 5` |
| 03:51:15 | DNS resolution fails for all seeds | `Failed to dial /dns4/seed1.doli.network` |
| 03:51:15 | `eligible_len=1` visible from first production check | `PROD_DIAG slot=1541 eligible_len=1` |
| 04:04:43 | First finality rejection (ancestor at h=66) | `FINALITY: plan_reorg rejecting reorg past finalized height 1473` |
| 04:04-13:48 | Node syncs fine via gossip despite eligible_len=1 | HEALTH shows h advancing, best_peer_h matching |
| 13:48:22 | N8 produced block at h=4967 (slot 5125) | `[BLOCK_PRODUCED] hash=90bfed97` |
| 13:48:27-40 | **43 blocks rate-limited** from 4+ peers | `Rate limit: dropping block from 12D3KooW...` |
| 13:48:32 | Last good block applied: h=4968 | `Applying block 81032da9 at height 4968` |
| 13:48:52 | h=4969 applied (slot 5128) | `Applying block d2566ee5 at height 4969` |
| 13:49:00 | HEALTH: h=4969, still in sync | `h=4969 s=5128 peers=20` |
| 13:49:02 | **Fork recovery starts** — begins behind/recover cycle | `Fork recovery: starting parent walk` |
| 13:49:04 | Reorg rejected: ancestor at h=3179, finalized at 4968 | `FINALITY: plan_reorg rejecting` |
| 13:50:00 | **STALE_TIP**: behind by 2 blocks | `Behind by 2 block(s) (local=4969, peer=4971)` |
| 13:51:02 | Rollback cascade begins: 4973-4972-4971-4970 | `[FORK] ROLLBACK gap=5 empties=3` |
| 13:51-14:00 | Repeated: rollback - catch up - behind - rollback | Multiple rollback/STALE_TIP entries |
| 14:00:00 | Brief recovery to h=5028 (state=Synchronized) | HEALTH shows convergence |
| 14:02:00 | **Stuck AGAIN** at h=5038 (before epoch 14 boundary at 5040) | h=5038 for 7+ minutes, peers advancing |
| 14:02-14:09 | Syncing:Headers / Syncing:Bodies oscillation | State alternates, height stuck |
| ~14:30-16:00 | Gradual recovery with continued behind/recover episodes | Height advancing slowly |
| 16:19:19 | **N8 restarted** (by operator) | systemd: `Started doli-mainnet-n8.service` |
| 16:19+ | Node snap-synced fresh, now healthy | state=Synchronized, h=6393 by 17:55 |

## Causal Chain (root to symptom)

```
[1] Snap sync leaves incomplete block history (block store floor=0, block 1 missing)
 |
 +-> [2] Attestation filter scans block store, finds insufficient history
      |
      +-> [3] eligible_len=1 instead of 21 (only 1 producer passes filter)
           |
           +-> [4] Node's scheduler diverges from network's scheduler
                |
                +-> [5] Node still RECEIVES blocks via gossip (doesn't need local scheduling)
                     |
                     +-> [6] 43 blocks rate-limited at 13:48 (burst from 4+ peers)
                          |
                          +-> [7] Node falls ~2 blocks behind peers
                               |
                               +-> [8] Sync manager requests headers from N8's tip hash
                                    |
                                    +-> [9] Peers return empty headers (N8's tip unknown to some)
                                         |
                                         +-> [10] After 3 consecutive empties: rollback triggered
                                              |
                                              +-> [11] Rollback creates deeper gap: more empties: more rollback
                                                   |
                                                   +-> [12] Fork recovery finds ancestors below finality: rejected
                                                        |
                                                        +-> [13] Behind/recover loop: node oscillates
```

## Key Evidence

### 1. eligible_len=1 (all ai4 nodes)
```
N6: [PROD_DIAG] slot=5123 eligible_len=1  (should be 21)
N7: [PROD_DIAG] slot=5123 eligible_len=1  (should be 21)
N8: [PROD_DIAG] slot=5127 eligible_len=1  (should be 21)
```
All three nodes on ai4 had the same bug. N6 had 31 finality rejections, N7 had 28, N8 had 34.

### 2. Rate limiting burst
43 blocks dropped from N8 at 13:48, 41 from N6, 37 from N7 — all at the same time. This was the trigger that pushed N8 into the cascade.

### 3. Genesis producer mismatch
```
[INTEGRITY_RISK] Genesis producer mismatch: StateDb has 6 producers but block store has 5
```
After the restart at 03:51, the node detected the snap sync inconsistency but continued running.

### 4. Deep divergence in reorg attempts
Common ancestors found at heights 0, 4, 13, 66, 71, 731, 824, 3179, 3180 — all far below finality. This proves the block store had blocks that peers' chains did not share.

## Why N8 Stuck While N6/N7 Recovered

All three nodes had identical underlying conditions (eligible_len=1, rate limiting, finality rejections). The difference was **timing and rollback depth**:
- N8 had 13 rollbacks vs N6's 9 and N7's 10 during the 13:XX window
- N8 was producing blocks (h=4967 was N8's block) which created more divergence surface
- N8's deeper rollbacks near epoch boundary (~h=5038/5040) created additional catch-up difficulty

## Gaps / Unresolved

1. **Why was N8 restarted at 03:51?** — The restart introduced the integrity risk. The first session (02:06 start) was running fine.
2. **Why eligible_len=1 persists** — This is a known bug (behavioral learning) but hasn't been code-fixed yet.
3. **Rate limiting burst source** — 43 blocks in ~13 seconds from 4+ peers. Possibly burst from nodes catching up after their own brief stall.

## Recommendations

1. **Fix the attestation filter for post-snap-sync nodes** (code change needed) — After snap sync, when block history is incomplete, fall back to "all active producers" instead of filtering. This is the root cause that made the node vulnerable.
2. **Add eligible_len sanity check** — If eligible_len < min_producers_threshold (e.g., 3), log a CRITICAL warning and use the full active producer set.
3. **Rate limiting backpressure** — When rate limiting drops blocks, the node should NOT immediately rollback. Instead, wait for gossip to naturally deliver the next blocks.
