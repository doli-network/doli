# Diagnosis Report: INC-I-015 — ProducerSet diverges under connection storm

## Symptom Profile
- **What happens**: Nodes reject valid blocks with "invalid producer for slot". The seed stuck at h=96 rejecting h=97 (slot 9098). N1 also rejecting at slot 9123. All 5 producers on different chain tips at the same height.
- **When**: During a connection storm (106 nodes on localhost). Specifically after the seed experienced peer loss (25->0->reconnect) and height regression (h=87->h=85 rollback).
- **Deterministic**: No -- timing-dependent. The connection storm causes different nodes to receive blocks in different orders and at different times.
- **Failure boundary**: ALL producer nodes affected. Seed AND producers diverge. 100+ stress test nodes exacerbate but are not the root cause.

## Hypotheses

### H1: Reactive scheduling diverges from different block sequences -- CONFIRMED -- conf(0.85, inferred)
- Initial: conf(0.65, inferred)
- The `scheduled` flag on each producer is derived from block history (missed slots, fallback detection, attestation bitfield). It is consensus-critical (used by `scheduled_producers_at_height()` for producer validation) but is locally computed from the block store -- there is no cross-node reconciliation.
- When nodes process different block sequences (due to mini-forks during the connection storm), the reactive scheduling accumulates different unschedule/schedule decisions.
- With 5 producers each having 1 ticket, `select_producer_for_slot(slot, scheduled_list)` uses `slot % N`. Changing N from 5 to 4 (one producer unscheduled) changes ALL rank assignments.
- Evidence: The logs show the seed rolled back from h=87 to h=85 and then synced forward -- potentially via different blocks than N1-N5.

### H2: apply_block skip-already-stored bug after rollback -- CONFIRMED -- conf(0.80, inferred)
- Initial: not hypothesized until code analysis
- In `apply_block` (mod.rs lines 27-55), the "already applied" guard checks `stored_height < height` to detect poisoned blocks.
- After rollback from h=87 to h=85, blocks at h=86 and h=87 remain in the store with their height indexes intact.
- When the sync pipeline re-delivers the canonical block at h=86 (same hash), `apply_block` sees `stored_height (86) < height (86)` = FALSE and SKIPS the block. Returns `Ok(())`.
- The chain_state remains at h=85 while the sync pipeline thinks the block was applied.
- This forces the node to accept ONLY blocks not yet in its store -- i.e., blocks from a DIFFERENT fork -- amplifying the scheduling divergence.

### H3: missed_slot_scheduler ordering issue -- ELIMINATED -- conf(0.0, inferred)
- Code analysis shows both step 1 and step 2 in apply_block read the same producer set state. The `pre_step1_scheduled` snapshot in `rebuild_scheduled_from_blocks` correctly mirrors apply_block's ordering.

### H4: Block store contamination from fork blocks -- SUBSUMED into H1+H2 -- conf(0.0)
- Not an independent mechanism -- the block store having fork blocks is a consequence of the connection storm, and the "already stored" skip bug (H2) determines which blocks the node re-applies.

## Root Cause

The root cause is the **interaction** of two mechanisms:

**Mechanism A (Trigger): `apply_block` incorrectly skips blocks after rollback.**

In `bins/node/src/node/apply_block/mod.rs` lines 27-55, the "already applied" guard uses `stored_height < height` to distinguish between "truly applied" and "poisoned" blocks. After rollback, rolled-back blocks remain in the store with their height indexes. When the canonical block at the rolled-back height is re-delivered by sync, the guard sees `stored_height == height` and concludes the block is "truly applied" -- but it was UNDONE by the rollback. The block is skipped.

This has two consequences:
1. If the canonical blocks match what was in the store, they ALL get skipped, and the node's state never advances past the rollback target. The node then receives blocks from a different peer/fork that ARE new to its store, applies them, and ends up on a divergent chain.
2. Even if the node eventually converges to the same chain tip, the intermediate block sequence differs from other nodes.

**Mechanism B (Amplifier): Reactive scheduling is consensus-critical but locally derived.**

The `scheduled` flag in `ProducerSet` is used by `scheduled_producers_at_height()` to build the weighted producer list for `validate_producer_eligibility()` (the function that emits "invalid producer for slot"). This flag is updated by the reactive scheduling code in `apply_block` lines 198-318, which processes:
1. Missed slot detection: `current_slot > prev_slot + 1` -> unschedule rank 0
2. Fallback detection: expected rank 0 != block.header.producer -> unschedule
3. Block producer alive -> schedule
4. Attestation bitfield -> schedule attesters

These decisions depend on the SPECIFIC blocks at each height (their slot numbers, producers, and attestation data). Different block sequences -> different scheduling decisions -> different `scheduled_producers_at_height()` results -> different rank assignments -> "invalid producer for slot".

**Why this is permanent (self-reinforcing):**

`rebuild_scheduled_from_blocks` replays scheduling from the block store. But the block store has the divergent chain's blocks, so the rebuild reproduces the divergent state. The node cannot accept canonical blocks because its scheduling disagrees, and its scheduling cannot converge because it cannot accept canonical blocks.

## Causal Chain

```
Connection storm (106 nodes)
  -> Seed loses all peers (25 -> 0)
  -> Seed's chain falls behind (h=3 stuck for 2+ minutes)
  -> Network produces blocks without seed (potential mini-forks)
  -> Seed reconnects, syncs from whichever peer responds first
  -> Seed at h=87, batch 2 connection storm triggers rollback to h=85
  -> [Mechanism A] Canonical blocks at h=86-87 already in store -> skipped
  -> Seed receives fork blocks (or re-syncs from forked peer) -> applied
  -> Seed now on a different chain than N1-N5 for heights 86-87
  -> [Mechanism B] Different blocks -> different reactive scheduling
  -> ProducerSet.scheduled flags diverge
  -> scheduled_producers_at_height() returns different set on seed vs N1-N5
  -> select_producer_for_slot(9098, seed_list) != select_producer_for_slot(9098, n1_list)
  -> Block at slot 9098 produced by N1 (valid per N1's scheduling) rejected by seed
  -> "invalid producer for slot" -- seed stuck at h=96, N1 stuck at slot 9123
  -> All producers diverge (each has slightly different scheduling state)
  -> No self-healing: rebuild_scheduled_from_blocks reproduces divergent state
```

## Code Locations

| What | File | Lines |
|------|------|-------|
| apply_block skip-already-stored bug | `bins/node/src/node/apply_block/mod.rs` | 27-55 |
| Reactive scheduling (missed slots) | `bins/node/src/node/apply_block/mod.rs` | 234-264 |
| Reactive scheduling (fallback detection) | `bins/node/src/node/apply_block/mod.rs` | 267-279 |
| scheduled_producers_at_height | `crates/storage/src/producer/set_core.rs` | 325-335 |
| validate_producer_eligibility | `crates/core/src/validation/producer.rs` | 200-252 |
| select_producer_for_slot | `crates/core/src/consensus/selection.rs` | 24-79 |
| rebuild_scheduled_from_blocks | `bins/node/src/node/rewards.rs` | 378-488 |
| rollback_one_block (no canonical index update) | `bins/node/src/node/rollback.rs` | 12-192 |
| set_canonical_chain (only called in apply_block) | `crates/storage/src/block_store/writes.rs` | 101-149 |

## Fix Direction (NOT a fix proposal -- diagnosis only)

The fix must address BOTH mechanisms:

**For Mechanism A**: The "already applied" check in `apply_block` must account for rollback. After rollback from h=N to h=M, blocks at heights M+1..N in the store are NOT applied -- their state was undone. The check should compare `stored_height` against `chain_state.best_height`, not against the expected height. If `stored_height > chain_state.best_height`, the block was rolled back and must be re-applied.

**For Mechanism B**: The reactive scheduling state must be made convergence-safe. Options:
1. **Rebuild scheduling from block store on every fork switch** (already done in execute_reorg, rollback_one_block, and startup -- but not after the skip-already-stored path)
2. **Make scheduling state part of the block header** (costly protocol change)
3. **Epoch-bounded scheduling reset**: At each epoch boundary, reset all scheduled flags to true and only accumulate scheduling changes within the epoch. This bounds divergence to one epoch and ensures convergence at epoch boundaries.

Option 1 (fixing the skip bug) is the minimal fix. If blocks are never skipped after rollback, the node always processes the correct canonical chain, and scheduling converges naturally.

## Residual Risks

1. If two nodes genuinely receive different blocks via gossip (not sync) for the same height during a temporary partition, reactive scheduling will diverge even without the skip bug. The network relies on fork choice + reorg to converge, but reorg itself uses `validate_producer_eligibility` which depends on scheduling. This creates a circular dependency that needs the epoch-bounded reset (option 3) as a defense-in-depth mechanism.
2. The skip bug may also affect non-stress-test scenarios where a node rolls back due to shallow forks during normal operation.
