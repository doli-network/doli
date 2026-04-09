# INC-I-024: excluded_producers scan range too short after rollback

## Status: ROOT CAUSE CONFIRMED — fix pending

## Summary

After rollback, `rebuild_excluded_from_headers()` scans from `epoch_start` to the rollback
target height. This produces fewer exclusions than the canonical chain (which has blocks
beyond the rollback target). The node's scheduler uses a different round-robin modulus
during catch-up, causing it to produce blocks the network rejects. This triggers more
rollbacks, creating a self-sustaining loop.

## Symptom

N8 on ai4 repeatedly: falls 3-5 blocks behind → rollback → catches up → produces a
block → block rejected by network → falls behind again. Cycle repeats every 30-60 seconds.

## Root Cause

**File**: `bins/node/src/node/rewards.rs:405-436` (`rebuild_excluded_from_headers`)

```rust
pub async fn rebuild_excluded_from_headers(&mut self) {
    let current_h = self.chain_state.read().await.best_height;  // ← rollback target
    let epoch_start = epoch * blocks_per_epoch;
    for h in start_h..=current_h {   // ← scans SHORT range
        if let Ok(Some(blk)) = self.block_store.get_block_by_height(h) {
            for pk in &blk.header.missed_producers {
                excluded.insert(*pk);
            }
        }
    }
    self.excluded_producers = excluded;  // ← FEWER exclusions than canonical chain
}
```

**Causal chain**:

1. Normal gossip delay → node falls 3 blocks behind (e.g., h=5181 while network is at h=5185)
2. Sync detects gap, peers return empty headers for node's tip → rollback triggered
3. Node rolls back from h=5181 to h=5178
4. `rebuild_excluded_from_headers()` scans epoch_start to h=5178 → finds `excluded=2`
5. Canonical chain at h=5185 has accumulated `excluded=7` (from missed_producers in blocks h=5179-5185)
6. Node's scheduler: `slot % 19` (21-2 excluded). Network's scheduler: `slot % 14` (21-7 excluded)
7. Different modulus → different producer selected for the same slot
8. Node produces at a slot where the network expects someone else → **rejected**
9. Rejected block → node falls behind again → goto step 1

**Why it self-sustains**: The rollback always resets excluded to a value LOWER than the
network's, because the scan range is always shorter. The catch-up window (re-applying
blocks h=5179-5185 which add the missing exclusions) overlaps with the production window.
If the node produces BEFORE finishing catch-up, it produces with the wrong scheduler.

## Evidence from production logs (2026-04-06)

| Time (UTC) | N8 excluded | N1 excluded | Match? |
|------------|------------|------------|--------|
| 13:45 | 1 | 1 | YES |
| 13:49 | 2 | 2 | YES |
| 13:52 | 7 | 7 | YES |
| 14:00 | 7 | 7 | YES |
| 14:21 | **2** | 7 | **NO** ← after rollback |

At 14:21, N8 rolled back and `rebuild_excluded_from_headers()` produced `excluded=2` while
the entire network had `excluded=7`. N8's scheduler used `slot%19` vs network `slot%14`.

**Verified across 6 nodes**: N1, N2, N3, N6, N7 all had `excluded=7` at 13:52. N8 agreed
at 13:52 but diverged after rollback at 14:21.

## Contributing factors (not root cause)

- **19 rogue peers from a different chain** (genesis mismatch) creating ~140 connect/disconnect
  events per minute. These waste CPU but do NOT directly cause the gossip failure.
- **Gossip mesh topology**: canonical block at slot 5127 (by 54323cef) never arrived at N8.
  This is the trigger that causes N8 to fall behind, but the bug is that the system doesn't
  recover cleanly from a normal gossip delay.

## Test gap

**Every test block uses `missed_producers: Vec::new()`**. No test covers:
1. Building blocks with non-empty `missed_producers` (slot gaps)
2. Rolling back mid-epoch after `excluded_producers` has accumulated from those blocks
3. Verifying `rebuild_excluded_from_headers()` produces the correct excluded set
4. Verifying the node does NOT produce during catch-up with a stale excluded set

**Specific test files affected**:
- `bins/node/tests/fork_recovery.rs:61` — `missed_producers: Vec::new()`
- `bins/node/tests/fork_recovery.rs:545` — test manually injects exclusions, verifies they
  clear to 0 after rollback. But test blocks have no missed_producers, so "clear to 0"
  is trivially correct. Never tests "clear to N where N > 0".

## Fix approach

The fix must ensure that after rollback + catch-up, the `excluded_producers` set matches
the canonical chain's BEFORE the node attempts to produce. Two options:

**Option A**: Block production until excluded set is fully reconstructed. After rollback,
suppress production until the node has caught up to the network tip height (where all
missed_producers from canonical blocks have been re-applied via post_commit_actions).

**Option B**: Make `rebuild_excluded_from_headers()` aware of the network tip. Instead of
scanning to `current_h` (rollback target), scan to `min(current_h, ...)` and also query
peers for their excluded count to detect divergence before producing.

Option A is simpler and more robust — it prevents the bad state from being acted on.

## Reproduction test outline

```
1. Create test node with 5 producers
2. Build 20 blocks, some with slot gaps → missed_producers accumulate → excluded grows
3. Verify excluded == expected_count (e.g., 3)
4. Rollback 5 blocks
5. Verify excluded == WRONG lower count (reproduces the bug)
6. Apply fix
7. Verify excluded == correct count after rollback OR production blocked until catch-up
8. Re-apply the 5 blocks
9. Verify excluded == original expected_count
10. Verify production only attempted AFTER excluded matches network
```
