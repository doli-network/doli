# INC-001: Sync State Explosion — Analysis

> Incident: INC-001 | Date: 2026-03-19 | Severity: Critical
> Resume: `/omega:bugfix --incident=INC-001`

## Summary

During a 56-node local testnet stress test (v36 genesis), three interrelated bugs were observed:
1. **Solo fork creation** — genesis producers build solo chains before syncing
2. **Rollback loop** — forked nodes oscillate between chains indefinitely
3. **Excessive empty slots** — 40% of slots produce no block

## Reproduction

1. Deploy testnet v36 with 6 nodes (1 seed + 5 genesis producers)
2. Wait for chain to reach h=30+
3. Deploy 50 additional producer nodes simultaneously
4. Observe: N4 enters rollback loop (h=31→0→31→0), multiple nodes stuck, slot fill rate ~60%

## Architecture Context

```
Production Timer (1s)
  → try_produce_block()
    → sync_manager.can_produce()  [production_gate.rs — 11 layers]
    → scheduling checks            [scheduling.rs — bootstrap guards]
    → build + broadcast block

Gossip Block Received
  → handle_new_block()
    → fork detection
    → handle_new_block_weighted() → execute_reorg()   [block_handling.rs]
    → OR sync_manager triggers fork_sync              [fork_sync.rs]

Fork Recovery
  → fork_sync: binary search for common ancestor     [fork_sync.rs]
  → download canonical chain
  → execute_fork_sync_reorg()                         [block_handling.rs:560-660]
    → execute_reorg() → rollback + apply              [block_handling.rs + rollback.rs]
    → reset_sync_for_rollback()                       [block_lifecycle.rs:500-529]
      → sets recovery_phase = PostRollback            ← THE BUG

Post-Rollback Sync
  → cleanup() calls start_sync()                      [cleanup.rs]
  → start_sync() sees PostRollback                    [sync_engine.rs:197]
    → starts ANOTHER fork_sync                        ← LOOP TRIGGER
```

## Root Cause Analysis

### Bug 1: Solo Fork Creation During Bootstrap

**RC-1A: Minimum peers too low during genesis**
- File: `bins/node/src/node/init.rs`
- Layer 5.5 (`production_gate.rs:274-276`) requires only 1 peer during genesis
- Any non-producer peer (including new joining nodes at h=0) satisfies this
- A genesis producer connected to a single new node can produce blocks that the majority never sees

**RC-1B: Solo production circuit breaker disabled**
- File: `crates/network/src/sync/manager/mod.rs:578`
- `max_solo_production_secs = 86400` (24 hours — effectively disabled)
- Layer 10.5 (`production_gate.rs:574-614`) never fires
- A producer can produce indefinitely without receiving ANY gossip block

**RC-1C: Gossip mesh disruption on mass join**
- When 50 nodes join simultaneously, GossipSub mesh reconfiguration disconnects genesis producers from each other
- N4 may end up connected only to new nodes at h=0
- Layer 6.5 sees `best_peer_height = 0`, computes `height_lag = 0`, allows production
- Layer 9 hash mismatch detection only compares peers at same height ±2, missing the fork

### Bug 2: Rollback Loop (THE PRIMARY BUG)

**RC-2A (PRIMARY): `reset_sync_for_rollback()` called on SUCCESS path**
- File: `bins/node/src/node/block_handling.rs:620`
- After a SUCCESSFUL fork sync reorg, `sync.reset_sync_for_rollback()` is called
- This sets `recovery_phase = PostRollback` (`block_lifecycle.rs:511`)
- The comment says: "After a fork rollback, our tip is still on the fork"
- **But this is WRONG on the success path** — after a successful reorg, the tip IS on the canonical chain
- When `start_sync()` runs next (`sync_engine.rs:197`), it sees PostRollback and starts another fork_sync

**The exact loop:**
```
State 0: Node at h=31 (solo fork). Network at h=49.

Step 1: Fork sync binary search → common ancestor at h=0 (genesis)
Step 2: Download 31 canonical blocks. weight_delta=0.
Step 3: "equal weight — accepting canonical chain (remedial reorg)"
Step 4: execute_reorg() rolls back 31, applies 31. Now at h=31 (canonical).
Step 5: reset_sync_for_rollback() → recovery_phase = PostRollback  ← BUG
Step 6: cleanup() → start_sync() → sees PostRollback
Step 7: start_fork_sync() starts another binary search          ← LOOP
Step 8: Second fork sync targets different peer (possibly one that
        received our OLD fork blocks via gossip)
Step 9: Binary search finds common ancestor at h=0 AGAIN
Step 10: Downloads 31 blocks from THAT peer (our old fork!)
Step 11: weight_delta=0 → remedial reorg accepts
Step 12: Now at h=31 on OLD fork. GOTO Step 1.  ← PING-PONG
```

**RC-2B: Cooldown doesn't fire on success path**
- File: `sync_engine.rs:209-214`
- Cooldown checks `last_fork_sync_rejection.elapsed()`
- `mark_fork_sync_rejected()` is only called on REJECTION paths (`block_handling.rs:507,577`)
- After success, `last_fork_sync_rejection` retains initial value (startup - 300s)
- Cooldown is always elapsed → fork sync always starts

**RC-2C: Reorg handler cleared on success**
- File: `block_lifecycle.rs:514`
- `self.reorg_handler.clear()` erases all block weight history after success
- Next fork detection has zero context about previously-seen chains

**RC-2D: Equal-weight remedial reorg enables ping-pong**
- File: `block_handling.rs:584-589`
- When `weight_delta == 0`, the remedial reorg unconditionally accepts
- Two genesis producers with 31 solo blocks each = equal weight
- Node oscillates between them with no memory of previously-held tips

### Bug 3: Excessive Empty Slots

**RC-3A: Layer 6.5 unconditionally blocks on height_lag > 3**
- File: `production_gate.rs:363-376`
- No timeout escape for lag 4-5 blocks
- During 56-node bootstrap, gossip delays cause brief 4+ block lags, skipping entire slots

**RC-3B: 90-second bootstrap sync grace period**
- File: `bins/node/src/node/production/scheduling.rs:90-93`
- `bootstrap_sync_grace_secs = 90` for testnet
- During this window, Check 2 blocks production if `slot_gap > 1`
- Wastes 9 potential slots per joining producer

**RC-3C: Producer list stability timer keeps resetting**
- File: `scheduling.rs:32-46`
- `producer_list_stability_secs = 15`
- Each new producer discovery resets `last_producer_list_change`
- With 50 nodes joining, discoveries arrive continuously, preventing production for minutes

## Impact Analysis

| Component | Impact | Blast Radius |
|-----------|--------|-------------|
| `block_lifecycle.rs` | `reset_sync_for_rollback()` — change needed on success path | 8 call sites, only 1 wrong |
| `block_handling.rs` | Line 620 — the trigger | 1 line change |
| `sync_engine.rs` | PostRollback branch — add cooldown for success | 1 branch |
| `mod.rs` | `max_solo_production_secs` default | 1 constant |
| `init.rs` | `min_peers_for_production` for genesis | 1 assignment |
| `production_gate.rs` | Layer 6.5 timeout for lag 4-5 | 1 condition |
| `scheduling.rs` | Grace period + stability debounce | 2 functions |

## Requirements

| ID | Requirement | Priority | Acceptance Criteria |
|----|------------|----------|-------------------|
| REQ-SYNC-001 | After successful fork sync reorg, set `recovery_phase = Normal` (not PostRollback) | **Must** | Success path at `block_handling.rs:620` does NOT call `reset_sync_for_rollback()`. Uses new method or passes flag. No fork sync loop on canonical switch. Rejected paths unchanged. |
| REQ-SYNC-002 | Add cooldown after successful fork sync to prevent immediate re-trigger | **Must** | After successful fork sync, a timestamp is updated. Next `start_sync()` PostRollback branch respects 30s cooldown. |
| REQ-SYNC-004 | Prevent equal-weight remedial reorg ping-pong | **Must** | Record tip hash before remedial reorg in a "rejected tips" set (capacity 10, TTL 5min). Refuse to reorg back to a previously-held tip. |
| REQ-SYNC-005 | Add fork sync loop detection circuit breaker | **Must** | Counter: consecutive fork syncs within 5 minutes. At 3: block further fork syncs, use header-first only. Reset on successful header-first sync. |
| REQ-SYNC-006 | Use header-first sync after successful fork sync reorg | **Should** | After successful reorg, remaining blocks (h=31→49) filled via header-first. Peers recognize canonical tip. |
| REQ-GATE-001 | Reduce `max_solo_production_secs` to 50s for testnet/mainnet | **Must** | Testnet/mainnet = 50 (5 slots). Devnet = 86400 (unchanged). Layer 10.5 fires after 50s solo. |
| REQ-GATE-002 | Set `min_peers_for_production = 2` during genesis for testnet/mainnet | **Must** | Testnet genesis = 2. Devnet genesis = 1. Layer 5.5 requires 2+ peers at genesis. |
| REQ-GATE-003 | Graduated timeout for Layer 6.5 height lag > 3 | **Should** | Lag 4-5: block for 60s then allow. Lag > 5: unconditional block (existing). |
| REQ-BOOT-001 | Reduce `bootstrap_sync_grace_secs` to 30s for testnet | **Should** | `scheduling.rs`: testnet = 30 (was 90). |
| REQ-BOOT-002 | Debounce producer list stability timer | **Should** | First discovery sets deadline = now + 15s. Further discoveries within deadline do NOT reset timer. |

## Milestones

### M1: Fix Rollback Loop (CRITICAL — blocks all stress testing)
- **Scope (modules)**: `block_lifecycle.rs`, `block_handling.rs`, `sync_engine.rs`
- **Scope (requirements)**: REQ-SYNC-001, REQ-SYNC-002, REQ-SYNC-004, REQ-SYNC-005, REQ-SYNC-006
- **Dependencies**: None (foundational fix)
- **Key change**: Line 620 of `block_handling.rs` — replace `reset_sync_for_rollback()` with a success-specific reset that sets `recovery_phase = Normal`

### M2: Fix Production Gate for Bootstrap
- **Scope (modules)**: `mod.rs` (SyncManager defaults), `init.rs`, `production_gate.rs`
- **Scope (requirements)**: REQ-GATE-001, REQ-GATE-002, REQ-GATE-003
- **Dependencies**: M1 (rollback fix needed first to validate)

### M3: Reduce Empty Slots During Bootstrap
- **Scope (modules)**: `scheduling.rs`
- **Scope (requirements)**: REQ-BOOT-001, REQ-BOOT-002
- **Dependencies**: M1, M2

## Bug 4: Circuit Breaker Genesis Deadlock (found during live testing)

**Discovered**: 2026-03-19 during genesis validation of M1-M3 fixes.

**Symptoms**: Chain stalled at h=6 s=13. Four nodes converged (seed, N1, N2, N3 at h=6) but production blocked by `"No gossip activity for 130s with 5 peers"`. Two nodes (N4 h=4, N5 h=2) still syncing.

**Root cause**: `production_gate.rs:606-610` — PGD-003 circuit breaker bypass requires ALL peers at our height. With N4/N5 still syncing, the check fails. Tip nodes see "not all peers at our height" → assume isolation → block production. But nobody produces → no gossip → nobody catches up → deadlock.

**RC-4A: ALL-peer stall check too strict**
- File: `production_gate.rs:606-610`
- `all_peers_at_our_height` is false when even 1 peer is syncing
- Changed to MAJORITY (>50% of peers at our height) to break the deadlock

**Status**: Fix implemented, build clean, tests pass. **NOT YET VALIDATED on live testnet.**

### Added requirement

| ID | Requirement | Priority | Acceptance Criteria |
|----|------------|----------|-------------------|
| REQ-GATE-004 | Circuit breaker bypass uses majority, not all peers | **Must** | - [ ] Stall bypass triggers when >50% peers at our height<br>- [ ] Chain does not stall during genesis with 2/5 nodes syncing |

## Bug 5: GSet Divergence During Genesis — THE TRUE ROOT CAUSE (confirmed 2026-03-19)

**This is the primary cause of ALL fork-related bugs during genesis.**

**Evidence**: N4 log shows persistent GSet divergence for 2+ minutes:
```
Producer schedule DIVERGENCE: gset=["ce0a95b6"] (count=1) vs known=["85b35002", "4b74478c", "81d7dfa5", "565638cc"] (count=4)
```

**Root cause**: `scheduling.rs:277-300` — during genesis, the on-chain ProducerSet is empty (producers register via VDF proof in blocks). The code falls through to the GSet (producer discovery CRDT) as the source for the scheduler. But the GSet has DIFFERENT contents on different nodes because anti-entropy convergence takes time.

**Result**: Each node computes a different scheduler:
- N1 with GSet [N1, N2, N3]: slot 0 → N1 (primary)
- N4 with GSet [N4]: slot 0 → N4 (primary)
- Both produce for slot 0 → fork

**RC-5A: GSet used for scheduling during genesis**
- File: `bins/node/src/node/production/scheduling.rs:277-300`
- Fallback chain: on-chain (empty) → GSet (divergent!) → known_producers (also divergent!)
- All nodes have the SAME hardcoded genesis producer list from `genesis.rs`
- **Fix**: During genesis, use the hardcoded genesis producer list instead of GSet for scheduling

| ID | Requirement | Priority | Acceptance Criteria |
|----|------------|----------|-------------------|
| REQ-SCHED-001 | During genesis, use hardcoded genesis producers for scheduling (not GSet) | **Must** | - [ ] All nodes compute identical scheduler during genesis<br>- [ ] No competing blocks for the same slot<br>- [ ] Slot fill rate >80% during first 100 blocks |

## Bug 6: Processing Stall — Zero Block Extraction (RC-6)

`get_blocks_to_apply()` returns 0 blocks while state is `Processing` — nothing calls `block_applied_with_weight()` to transition out. State stuck for up to 30s (cleanup timeout). During stall, production is blocked (`BlockedSyncing`), gossip blocks queued not applied.

**Fix applied**: Immediate reset to Idle when 0 blocks extracted in Processing state.

## Bug 7: Layer 5.5 Genesis Bypass (RC-7)

`production_gate.rs:275` — `local_height > 0` check lets ALL nodes at h=0 produce with just 1 peer, even when `min_required=2`. Evidence: N1/N4/N5 all produced competing blocks for slot 3.

**Fix applied**: Only bypass peer check at h=0 when `min_peers <= 1` (devnet). Testnet/mainnet enforce 2 peers even at h=0.

**Result**: RC-7 fix confirmed deployed and blocking. But forks STILL occur after nodes get 2+ peers.

## Bug 8: Fallback Window Too Short (UNSOLVED — requires architectural decision)

The 2-second exclusive fallback window assumes gossip propagation < 2s. But actual block delivery takes:
- VDF computation: ~85ms
- `apply_block()`: ~270ms (serialized, blocks event loop)
- GossipSub mesh delivery: variable
- Total: **>500ms per hop**, but with event loop serialization can exceed 2s

Evidence: slot 3 had 3 competing blocks at t=0s, t=+2s, t=+4s — each in correct rank window, none saw the previous block. **This is the UNSOLVED root cause.**

## Incident Status

**Status: OPEN — Bug 8 requires architectural decision.**

### Fixes Applied (all necessary, all insufficient alone)

| Fix | What | Status |
|-----|------|--------|
| M1 | Rollback loop: `reset_sync_after_successful_reorg()` | Deployed, working |
| M2 | Production gate: `min_peers=2`, `max_solo=50s` | Deployed, working |
| M3 | Empty slots: stability debounce, grace 90→30s | Deployed, working |
| RC-4A | Circuit breaker: majority bypass (not all peers) | Deployed, working |
| RC-5A | Scheduling: hardcoded genesis producers | Deployed, working |
| RC-6 | Processing stall: immediate Idle on 0 blocks | Deployed, needs verification |
| RC-7 | Layer 5.5: no genesis bypass when min_peers≥2 | Deployed, working |

### Unsolved

| Bug | Problem | Possible Approaches |
|-----|---------|-------------------|
| **RC-8** | 2s fallback window < actual gossip propagation time | (a) Increase window to 4-5s, (b) Broadcast header BEFORE VDF so peers know a block is coming, (c) Only rank 0 produces for first N blocks, (d) Check gossip queue not just block_store |

### Next Session

Resume with `/omega:bugfix --incident=INC-001`. Focus on RC-8: why blocks take >2s to propagate on localhost and how to fix the fallback timing model.

## Specs/Docs Drift

- `docs/troubleshooting.md` does not cover the rollback loop scenario
- No documentation on `recovery_phase` state machine or its invariants
