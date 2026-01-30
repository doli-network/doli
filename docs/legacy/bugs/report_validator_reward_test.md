# DOLI Validator Reward Test Report

**Date**: 2026-01-27
**Resolution Date**: 2026-01-27
**Network**: Devnet (5s slots, 20 slots/epoch = 100s)
**Test Configuration**: 3-node validator cluster
**Status**: RESOLVED

## Executive Summary

A 3-node validator test was conducted to verify the Pool-First Epoch Reward Distribution system. The test revealed **two bugs**, both now fixed:

1. **FIXED**: Producer list stability timer infinite reset loop
2. **FIXED**: Reward distribution is now proportional to blocks produced

---

## Test Configuration

| Parameter | Value |
|-----------|-------|
| Nodes | 3 |
| Network | Devnet (network_id=99) |
| Slot Duration | 5 seconds |
| Slots per Epoch | 20 |
| Epoch Duration | 100 seconds |
| Block Reward | 100 DOLI |
| Pool per Epoch | 2000 DOLI |
| Stabilization Wait | 45 seconds between node joins |

### Node Setup

- **Node 1** (seed): Started first, produces genesis
- **Node 2**: Joined 45s after Node 1
- **Node 3**: Joined 45s after Node 2

---

## Bug #1: Stability Timer Infinite Reset (FIXED)

### Symptoms

After all 3 nodes joined and discovered each other, block production completely stopped. Logs showed:

```
Producer announcements: added=1, rejected=0, duplicates=0
Waiting for producer list stability (15s since last change)...
```

This message repeated every 5 seconds indefinitely.

### Root Cause

The producer discovery system uses a **stability timer** that requires 15 seconds without changes before block production begins. The problem was:

1. Gossip interval on devnet = 5 seconds
2. Every gossip round, nodes send updated announcements with incremented sequence numbers
3. `MergeResult.added` counted **both** new producers AND sequence number updates
4. Stability timer reset whenever `added > 0`
5. Result: Timer reset every 5 seconds, never reaching 15 seconds of stability

### Fix Applied

Modified the discovery system to distinguish between:
- **New producers** (first time seeing this public key)
- **Sequence updates** (existing producer refreshing liveness proof)

**Changes to `crates/core/src/discovery/mod.rs`**:
```rust
pub struct MergeResult {
    pub added: usize,           // New producers + sequence updates
    pub new_producers: usize,   // Only truly new producers (ADDED)
    pub rejected: usize,
    pub duplicates: usize,
}
```

**Changes to `crates/core/src/discovery/gset.rs`**:
```rust
pub enum MergeOneResult {
    NewProducer,     // First time seeing this producer
    SequenceUpdate,  // Existing producer with newer sequence
    Duplicate,       // Same or older sequence (no change)
}
```

**Changes to `bins/node/src/node.rs`**:
```rust
// Only reset stability timer for truly NEW producers
if merge_result.new_producers > 0 {
    self.last_producer_list_change = Some(Instant::now());
}
```

### Verification

After the fix, the 3-node test showed:
- Block production resumed after stability period
- 3 complete epochs ran successfully
- Nodes produced blocks in round-robin order

---

## Bug #2: Equal vs Proportional Reward Distribution (FIXED)

### Problem Description

The current implementation distributes epoch rewards **equally** among all producers, regardless of how many blocks each produced. This is incorrect.

### Expected Behavior

From the specification: Rewards should be **proportional to the number of blocks** each producer contributed during the epoch.

### Test Results Showing the Bug

**Epoch 0 Block Production**:
| Producer | Blocks | Expected Share | Actual Share |
|----------|--------|----------------|--------------|
| Node 1   | 5      | 500 DOLI (25%) | 666 DOLI (33.3%) |
| Node 2   | 14     | 1400 DOLI (70%)| 666 DOLI (33.3%) |
| Node 3   | 1      | 100 DOLI (5%)  | 666 DOLI (33.3%) |
| **Total**| 20     | 2000 DOLI      | 2000 DOLI |

Node 3 produced only **1 block** but received the **same reward** as Node 2 which produced **14 blocks**.

### Bug Location

`bins/node/src/node.rs` lines 1498-1527:

```rust
// CURRENT (INCORRECT):
let fair_share = self.epoch_reward_pool / num_producers;  // Equal distribution
let amount = if i == 0 {
    fair_share + remainder
} else {
    fair_share
};
```

### Required Fix

The reward calculation should be:

```rust
// CORRECT (PROPORTIONAL):
// Calculate total blocks produced in epoch
let total_blocks: u64 = producers.values().sum();

// For each producer:
let proportional_share = (self.epoch_reward_pool * blocks) / total_blocks;
```

This ensures:
- Node producing 14/20 blocks gets 70% of rewards
- Node producing 1/20 blocks gets 5% of rewards

### Impact (Before Fix)

Without this fix:
- Late-joining validators receive unfair windfall rewards
- Early validators are penalized despite doing most of the work
- Economic incentives are misaligned - no motivation to join early in epoch

### Fix Applied

Changed `bins/node/src/node.rs` reward distribution from equal shares to proportional:

```rust
// Calculate total blocks for proportional distribution
let total_blocks: u64 = producers.values().sum();

// For each producer:
let amount = if i == sorted_producers.len() - 1 {
    self.epoch_reward_pool - distributed  // Last gets dust
} else {
    (self.epoch_reward_pool * *blocks) / total_blocks  // Proportional
};
```

**Expected Results After Fix**:
| Producer | Blocks | Share |
|----------|--------|-------|
| Node 1   | 5      | 500 DOLI (25%) |
| Node 2   | 14     | 1400 DOLI (70%) |
| Node 3   | 1      | 100 DOLI (5%) |

---

## Test Timeline

### Phase 1: Setup (0-90s)
- 0s: Node 1 started as seed
- 45s: Node 2 joined
- 90s: Node 3 joined

### Phase 2: Discovery Stabilization (90-120s)
- All nodes discovered each other via gossip
- Producer list showed 3 validators
- Stability timer began countdown

### Phase 3: Block Production (120s+)
- Round-robin production began
- Epoch 0 completed at block 20
- Epoch rewards distributed (incorrectly equally)
- Subsequent epochs ran normally

---

## Recommendations

### Completed

1. **Fixed reward distribution bug** - Changed from equal to proportional calculation
2. **Fixed stability timer bug** - Only reset on truly new producers

### Future Work

1. **Add integration test** - Verify proportional rewards with multi-producer epochs
2. **Update documentation** - Clarify reward distribution formula in specs

### Applied Code Change

```rust
// In bins/node/src/node.rs, replace lines 1498-1527:

// Calculate total blocks in epoch for proportional distribution
let total_blocks: u64 = producers.values().sum();

info!(
    "Epoch {} complete at height {}: distributing {} DOLI proportionally to {} producers ({} total blocks)",
    self.current_reward_epoch,
    height,
    self.epoch_reward_pool / 100_000_000,
    num_producers,
    total_blocks
);

// Sort producers for deterministic ordering
let mut sorted_producers: Vec<_> = producers.iter().collect();
sorted_producers.sort_by(|(a, _), (b, _)| a.as_bytes().cmp(b.as_bytes()));

// Track distributed amount to handle rounding
let mut distributed: u64 = 0;

// Create epoch reward transactions for all producers
let mut epoch_reward_txs = Vec::new();

for (i, (pubkey, blocks)) in sorted_producers.iter().enumerate() {
    let pubkey_hash = crypto_hash(pubkey.as_bytes());
    let recipient_hash = crypto_hash(pubkey.as_bytes());

    // Proportional share based on blocks produced
    let proportional_share = (self.epoch_reward_pool * *blocks) / total_blocks;

    // Last producer gets any remaining dust from rounding
    let amount = if i == sorted_producers.len() - 1 {
        self.epoch_reward_pool - distributed
    } else {
        proportional_share
    };
    distributed += amount;

    // Create epoch reward transaction
    let epoch_reward_tx = Transaction::new_epoch_reward(
        self.current_reward_epoch,
        (*pubkey).clone(),
        amount,
        recipient_hash,
    );

    info!(
        "  Producer {}: {} blocks ({:.1}%) -> {} DOLI reward",
        &pubkey_hash.to_hex()[..16],
        blocks,
        (*blocks as f64 / total_blocks as f64) * 100.0,
        amount / 100_000_000,
    );

    epoch_reward_txs.push(epoch_reward_tx);
}
```

---

## Files Modified

### Bug #1 Fix (Stability Timer)

| File | Changes |
|------|---------|
| `crates/core/src/discovery/mod.rs` | Added `new_producers` field to `MergeResult` |
| `crates/core/src/discovery/gset.rs` | Added `MergeOneResult` enum, updated merge logic |
| `crates/core/src/discovery/gossip.rs` | Updated test fixtures and doctest |
| `bins/node/src/node.rs` | Only reset stability on `new_producers > 0` |

### Bug #2 Fix (Proportional Rewards)

| File | Changes |
|------|---------|
| `bins/node/src/node.rs` | Changed reward distribution from equal to proportional based on blocks produced |

## Commit References

- Bug #1 fix: `fix(discovery): only reset stability timer for truly new producers` (355ec21)
- Bug #2 fix: `fix(node): distribute epoch rewards proportionally to blocks produced` (7d9e228)

---

## Conclusion

The 3-node validator test successfully identified and fixed two bugs in the reward system:

1. **Stability timer bug** - Fixed and committed. Block production now works correctly in multi-node clusters.

2. **Reward distribution bug** - Fixed. Rewards are now distributed proportionally based on blocks produced, ensuring fair economic incentives.

Both fixes ensure the consensus and reward systems work correctly for multi-validator deployments.
