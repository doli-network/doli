# REWARDS.md - Deterministic Epoch Rewards Refactoring

**Status**: In Progress (Milestones 1-3 Complete)
**Created**: 2026-01-30
**Author**: Protocol Team

---

## Executive Summary

This document describes the refactoring of the DOLI reward distribution system from a local-state model to a fully deterministic, BlockStore-derived model. The change eliminates consensus drift, lost rewards, and synchronization fragility.

---

## Table of Contents

1. [Problem Statement](#1-problem-statement)
2. [Root Cause Analysis](#2-root-cause-analysis)
3. [Proposed Solution](#3-proposed-solution)
4. [Architecture Comparison](#4-architecture-comparison)
5. [Implementation Milestones](#5-implementation-milestones)
6. [Edge Cases](#6-edge-cases)
7. [Validation Rules](#7-validation-rules)
8. [Testing Strategy](#8-testing-strategy)
9. [Rollback Plan](#9-rollback-plan)
10. [Data Persistence Verification](#10-data-persistence-verification)

---

## 1. Problem Statement

### 1.1 Observed Symptoms

During testnet simulation with 5 producer nodes:

```
Chain height: 5,936 blocks
All 5 producers actively producing blocks (verified via block headers)

RPC getProducers response:
- blocksProduced: 0 (all producers)
- pendingRewards: 0 (all producers)
```

Blocks ARE being produced, but reward tracking shows zero.

### 1.2 Impact

| Impact | Severity |
|--------|----------|
| Producers cannot verify their earnings | High |
| Potential reward loss on node restart | Critical |
| Inconsistent state across nodes | Critical |
| RPC returns incorrect data | Medium |

---

## 2. Root Cause Analysis

### 2.1 Two Reward Systems Exist

The codebase contains two separate reward models:

#### Model A: Pull/Claim (in `crates/storage/src/producer.rs`)
```rust
pub fn distribute_block_reward(&mut self, producer_pubkey: &PublicKey, reward: u64) {
    self.credit_reward(producer_pubkey, reward);  // Updates pending_rewards
    self.record_block_produced(producer_pubkey);  // Updates blocks_produced
}
```
- Updates `ProducerInfo.pending_rewards`
- Updates `ProducerInfo.blocks_produced`
- **NEVER CALLED** by the node

#### Model B: EpochPool (in `bins/node/src/node.rs`)
```rust
if self.params.reward_mode == RewardMode::EpochPool {
    // Creates EpochReward transactions at epoch boundaries
    // Goes directly to UTXO set
}
```
- Creates `EpochReward` transactions
- Rewards go to UTXO set directly
- **ACTIVE** - used by all networks
- Does NOT update `ProducerInfo` fields

### 2.2 The Active System's Flaws

The EpochPool system maintains **local state** not part of consensus:

```rust
// In Node struct - LOCAL STATE
epoch_producer_blocks: Arc<RwLock<HashMap<PublicKey, u64>>>,
epoch_start_height: u64,
epoch_reward_pool: u64,
current_reward_epoch: u64,
```

**Problems with local state:**

| Issue | Description |
|-------|-------------|
| Node restart | State lost, rewards miscalculated |
| Sync from peers | State doesn't sync, each node calculates independently |
| Chain reorg | Local state may not match new chain |
| Validation | `<= max` is too loose, allows under-distribution |

### 2.3 Why RPC Shows Zeros

```
RewardMode::EpochPool
       │
       ▼
EpochReward Transactions → UTXO Set (rewards distributed here)
       │
       ✗ ProducerInfo.pending_rewards = 0 (never updated)
       ✗ ProducerInfo.blocks_produced = 0 (never updated)
```

The RPC reads from `ProducerInfo`, but rewards go to UTXO. The fields are vestigial.

---

## 3. Proposed Solution

### 3.1 Core Principle

> **Everything is calculated from the BlockStore. Zero local state.**

Any node can independently verify rewards by reading the same blocks. No synchronization required.

### 3.2 New Rules

| Rule | Definition |
|------|------------|
| Epoch | `slot / 360` (derived, never tracked) |
| Boundary Detection | `current_epoch > last_rewarded_epoch` |
| Pool Calculation | `produced_blocks × block_reward` (empty slots don't count) |
| Distribution | Proportional to blocks produced |
| Validation | Exact match (validator recalculates from BlockStore) |

### 3.3 Flow Diagrams

#### Normal Case (boundary slot has block)

```
Slot 359: Block (epoch 0)
Slot 360: Block by Alice
          │
          ├── current_epoch = 360/360 = 1
          ├── last_rewarded = 0 (from chain scan)
          ├── 1 > 0 → Must distribute epoch 0!
          │
          ├── Scan BlockStore: slots 1-359
          ├── Count: Alice=72, Bob=71, Carol=72, Dave=71, Eve=72
          ├── Pool = 358 blocks × 1 DOLI = 358 DOLI
          │
          └── Create 5 EpochReward transactions (proportional)
```

#### Empty Boundary Case

```
Slot 359: Block (epoch 0)
Slot 360: [EMPTY]
Slot 361: [EMPTY]
Slot 362: Block by Bob
          │
          ├── current_epoch = 362/360 = 1
          ├── last_rewarded = 0 (from chain scan)
          ├── 1 > 0 → Must distribute epoch 0!
          │
          └── Same calculation, Bob's block includes rewards
```

#### Multiple Empty Epochs Catch-up

```
Slot 1080 (epoch 3), last_rewarded = 0:

Block N+0: current_epoch=3 > last_rewarded=0 → distribute epoch 1
           (after apply: last_rewarded = 1)

Block N+1: current_epoch=3 > last_rewarded=1 → distribute epoch 2
           (after apply: last_rewarded = 2)

Block N+2: current_epoch=3 > last_rewarded=2 → distribute epoch 3
           (after apply: last_rewarded = 3)

Block N+3: current_epoch=3 == last_rewarded=3 → normal block, no rewards
```

---

## 4. Architecture Comparison

| Aspect | Before (Local State) | After (BlockStore) |
|--------|---------------------|-------------------|
| State location | Node memory + ChainState | BlockStore only |
| Deterministic | No | Yes |
| Restart-safe | No (state lost) | Yes |
| Sync-safe | No (each node differs) | Yes |
| Fork handling | Fragile | Automatic |
| Empty slots | Inflated pool | Correct (no phantom rewards) |
| Empty boundary | Lost rewards | Next block distributes |
| Validation | `<= max` (loose) | Exact match |
| Complexity | High (state management) | Low (pure calculation) |

---

## 5. Implementation Milestones

### Overview

```
Milestone 1: BlockStore Query Methods     [~2 hours]
Milestone 2: Remove Local State           [~1 hour]
Milestone 3: Deterministic Calculation    [~2 hours]
Milestone 4: Producer Integration         [~1 hour]
Milestone 5: Validation Refactor          [~2 hours]
Milestone 6: Testing                      [~2 hours]
Milestone 7: Cleanup                      [~1 hour]
                                    Total: ~11 hours
```

---

### Milestone 1: BlockStore Query Methods ✅ COMPLETE

**File**: `crates/storage/src/block_store.rs`

**Tasks**:

- [x] **1.1** Add `get_block_by_slot(slot: u32) -> Option<Block>`
  - Uses existing `CF_SLOT_INDEX` to find block hash
  - Returns `None` for empty slots

- [x] **1.2** Add `get_blocks_in_slot_range(start: u32, end: u32) -> Vec<Block>`
  - Iterates slot range
  - Skips empty slots
  - Returns blocks in slot order

- [x] **1.3** Add `get_last_rewarded_epoch() -> u64`
  - Scans backwards from tip
  - Finds most recent block with `EpochReward` transaction
  - Extracts epoch number from `EpochRewardData`
  - Returns 0 if no rewards ever distributed

- [x] **1.4** Add unit tests for all three methods (10 tests total)

**Acceptance Criteria**: ✅ All met
- All methods work with empty slots
- `get_last_rewarded_epoch()` returns 0 on fresh chain
- Range queries work correctly (iterate slot by slot)

**Commit**: `22de5c5 feat(storage): add BlockStore query methods for epoch rewards`

---

### Milestone 2: Remove Local State from Node ✅ COMPLETE

**File**: `bins/node/src/node.rs`

**Tasks**:

- [x] **2.1** Remove fields from `Node` struct:
  ```rust
  // REMOVED these fields:
  epoch_producer_blocks: Arc<RwLock<HashMap<PublicKey, u64>>>,
  epoch_start_height: u64,
  epoch_reward_pool: u64,
  current_reward_epoch: u64,
  pending_epoch_rewards: Vec<Transaction>,
  ```

- [x] **2.2** Remove from `Node::new()`:
  - Loading epoch state from ChainState
  - Initialization of epoch tracking fields

- [x] **2.3** Remove from `apply_block()`:
  - Epoch tracking block (~lines 1555-1728)
  - Keep `known_producers` tracking for round-robin

- [x] **2.4** Remove from `shutdown()`:
  - Saving epoch state to ChainState

- [x] **2.5** Remove from `force_resync_from_genesis()`:
  - Resetting epoch variables

**Acceptance Criteria**: ✅ All met
- Node compiles without epoch state fields
- Node starts and runs (rewards temporarily broken)

**Commit**: `4dae71c refactor(node): remove local epoch state for deterministic rewards`

---

### Milestone 3: Deterministic Reward Calculation ✅ COMPLETE

**File**: `bins/node/src/node.rs`

**Tasks**:

- [x] **3.1** Add function `calculate_epoch_rewards()`:
  ```rust
  fn calculate_epoch_rewards(
      &self,
      epoch: u64,
      current_height: u64,
  ) -> Result<Vec<Transaction>>
  ```

  Implementation:
  - Calculate slot range using `self.params.slots_per_reward_epoch`
  - For epoch 0: `start = 1` (skip genesis)
  - Call `self.block_store.get_blocks_in_slot_range(start, end)`
  - Count blocks per producer (exclude null producer)
  - Calculate pool: `total_blocks × block_reward(current_height)`
  - Distribute proportionally using u128 intermediate calculation
  - Last producer (by sorted pubkey) gets rounding dust
  - Return `Vec<Transaction>` of `EpochReward` txs

- [x] **3.2** Add helper `should_include_epoch_rewards()`:
  ```rust
  fn should_include_epoch_rewards(&self, current_slot: u32) -> Option<u64>
  ```
  - Calculate `current_epoch = current_slot / slots_per_reward_epoch`
  - Get `last_rewarded = block_store.get_last_rewarded_epoch()`
  - If `current_epoch > last_rewarded`: return `Some(last_rewarded + 1)`
  - Else: return `None`

- [x] **3.3** Add unit tests with BlockStore (13 tests total)
  - `test_should_include_rewards_slot_zero`
  - `test_should_include_rewards_first_epoch_boundary`
  - `test_should_include_rewards_already_rewarded`
  - `test_should_include_rewards_multi_epoch_catchup`
  - `test_epoch_rewards_empty_epoch`
  - `test_epoch_rewards_single_producer`
  - `test_epoch_rewards_multiple_producers_equal`
  - `test_epoch_rewards_proportional_distribution`
  - `test_epoch_rewards_rounding_dust`
  - `test_epoch_rewards_deterministic_ordering`
  - `test_epoch_rewards_skip_null_producer`
  - `test_epoch_rewards_epoch_1_slot_range`
  - `test_epoch_rewards_transaction_structure`

- [x] **3.4** Integrate with `try_produce_block()` in `RewardMode::EpochPool` branch

**Acceptance Criteria**: ✅ All met
- Correct rewards for normal epoch
- Correct handling of empty slots (reduced pool)
- Correct proportional distribution
- Deterministic output (same input → same output)

**Commit**: `feat(node): add deterministic epoch reward calculation`

---

### Milestone 4: Producer Integration

**File**: `bins/node/src/node.rs`

**Tasks**:

- [x] **4.1** Modify `produce_block()`: *(completed in Milestone 3)*
  ```rust
  // In RewardMode::EpochPool branch:
  if let Some(epoch_to_reward) = self.should_include_epoch_rewards(current_slot) {
      let epoch_txs = self.calculate_epoch_rewards(epoch_to_reward, height)?;
      transactions.extend(epoch_txs);
  }
  ```

- [ ] **4.2** Ensure block includes EpochReward txs at correct position
  - EpochReward txs should be first (before user txs)
  - Or define clear ordering rule

- [ ] **4.3** Integration test: produce block at epoch boundary

**Acceptance Criteria**:
- Producer creates correct EpochReward txs at boundary
- Non-boundary blocks have no reward txs
- Multiple empty epochs catch up correctly

---

### Milestone 5: Validation Refactor

**File**: `crates/core/src/validation.rs`

**Tasks**:

- [ ] **5.1** Add `BlockStore` to validation context (or pass separately)

- [ ] **5.2** Modify `validate_block_rewards()`:
  ```rust
  fn validate_block_rewards(
      block: &Block,
      ctx: &ValidationContext,
      block_store: &BlockStore,  // NEW
  ) -> Result<(), ValidationError>
  ```

  Implementation:
  - Calculate `current_epoch = block.slot / 360`
  - Get `last_rewarded = block_store.get_last_rewarded_epoch()`
  - If `current_epoch > last_rewarded`:
    - Recalculate expected rewards (same algorithm as producer)
    - Compare block's EpochReward txs to expected
    - Must match **exactly** (count, amounts, recipients)
  - If `current_epoch == last_rewarded`:
    - Block must have NO EpochReward txs

- [ ] **5.3** Update `ValidationError` enum:
  ```rust
  InvalidEpochReward(String),
  UnexpectedEpochReward,      // NEW: rewards in non-boundary block
  MissingEpochReward,         // NEW: no rewards at boundary
  EpochRewardMismatch {       // NEW: wrong amounts/recipients
      expected: Vec<(PublicKey, u64)>,
      actual: Vec<(PublicKey, u64)>,
  },
  ```

- [ ] **5.4** Update all validation tests

**Acceptance Criteria**:
- Validator rejects incorrect reward amounts
- Validator rejects rewards at non-boundary
- Validator rejects missing rewards at boundary
- Validator accepts exact match

---

### Milestone 6: Testing

**Tasks**:

- [ ] **6.1** Unit tests for `BlockStore` methods
  - Empty chain
  - Chain with gaps
  - Multiple epochs

- [ ] **6.2** Unit tests for `calculate_epoch_rewards()`
  - Normal epoch (all slots filled)
  - Epoch with empty slots
  - Epoch with single producer
  - Epoch with many producers

- [ ] **6.3** Integration tests for producer
  - Produce at boundary
  - Produce after empty boundary
  - Multi-epoch catch-up

- [ ] **6.4** Integration tests for validation
  - Accept valid rewards
  - Reject over-distribution
  - Reject under-distribution
  - Reject wrong recipients

- [ ] **6.5** Multi-node testnet test
  - 5 nodes, run for 2+ epochs
  - Verify all nodes agree on rewards
  - Restart a node mid-epoch, verify consistency

**Acceptance Criteria**:
- All tests pass
- No consensus failures in multi-node test
- Rewards match expected values

---

### Milestone 7: Cleanup

**Tasks**:

- [ ] **7.1** Remove unused code in `chain_state.rs`:
  - Epoch tracking fields (or mark deprecated)

- [ ] **7.2** Remove Pull/Claim model code (optional):
  - `ProducerInfo.pending_rewards` field
  - `ProducerInfo.blocks_produced` field
  - Related methods
  - OR: Repurpose for RPC display (calculate from BlockStore)

- [ ] **7.3** Update RPC `getProducers`:
  - Option A: Remove misleading fields
  - Option B: Calculate from BlockStore on-demand

- [ ] **7.4** Update documentation:
  - `docs/protocol.md` - reward distribution section
  - `specs/protocol.md` - epoch reward rules

- [ ] **7.5** Run `/sync-docs` to verify alignment

**Acceptance Criteria**:
- No dead code
- RPC returns accurate data
- Documentation matches implementation

---

## 6. Edge Cases

| Case | Handling |
|------|----------|
| Empty boundary slot | First block of new epoch distributes |
| Genesis block (slot 0) | Exclude from rewards (null producer) |
| Epoch with no blocks | No rewards distributed (pool = 0) |
| Multiple empty epochs | Distribute one at a time, oldest first |
| Rounding dust | Last producer (sorted by pubkey) receives remainder |
| Chain reorg across epoch | New fork recalculates from its own blocks |
| Node restart mid-epoch | No state to lose, recalculates from BlockStore |
| Sync from genesis | Validates all historical rewards from BlockStore |

---

## 7. Validation Rules

### At Epoch Boundary (current_epoch > last_rewarded)

```
MUST have EpochReward transactions
MUST have correct epoch number in EpochRewardData
MUST have correct total (sum of block rewards for produced blocks)
MUST distribute to correct producers (those who produced blocks)
MUST have correct individual amounts (proportional to blocks)
MUST NOT have coinbase transaction
```

### At Non-Boundary (current_epoch == last_rewarded)

```
MUST NOT have EpochReward transactions
MUST NOT have coinbase transaction (in EpochPool mode)
```

---

## 8. Testing Strategy

### Unit Test Coverage

| Component | Test Cases |
|-----------|------------|
| `get_blocks_in_slot_range` | Empty range, full range, gaps |
| `get_last_rewarded_epoch` | No rewards, one epoch, many epochs |
| `calculate_epoch_rewards` | Normal, empty slots, single producer |
| `validate_block_rewards` | Valid, over, under, wrong recipients |

### Integration Test Scenarios

| Scenario | Description |
|----------|-------------|
| Happy path | 5 producers, 2 epochs, all slots filled |
| Empty slots | Random 10% empty slots |
| Empty boundary | Boundary slot empty, next block distributes |
| Node restart | Stop node, restart, verify consistency |
| Multi-epoch gap | 3 consecutive empty epochs, catch-up |

### Testnet Validation

```bash
# Launch 5-node testnet
./scripts/launch_testnet.sh

# Run for 2+ epochs (720+ slots on testnet = 2+ hours)
# Verify via RPC:
curl -s -X POST http://127.0.0.1:18545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getProducers","params":{},"id":1}'

# Check all nodes agree on rewards
for port in 18545 18546 18547 18548 18549; do
  curl -s -X POST http://127.0.0.1:$port ...
done
```

---

## 9. Rollback Plan

If critical issues arise post-deployment:

1. **Immediate**: Nodes can revert to previous binary
2. **Data**: BlockStore is append-only, no corruption risk
3. **State**: No local state to corrupt (that's the point)
4. **Chain**: EpochReward transactions are valid, chain continues

### Compatibility

- Old nodes cannot validate new reward distribution (hard fork)
- Coordinate upgrade across all testnet nodes before activation
- Mainnet: Plan upgrade window with producer notification

---

## 10. Data Persistence Verification

### Storage Architecture

Each node maintains a complete copy of the blockchain in RocksDB:

```
Node Data Directory
├── blocks/                  ← RocksDB database
│   ├── *.sst               ← Sorted String Tables (immutable block data)
│   ├── *.log               ← Write-ahead logs
│   ├── MANIFEST-*          ← Database metadata
│   └── CURRENT             ← Current manifest pointer
├── signed_slots.db/        ← Double-sign prevention
├── producer_gset.bin       ← Producer set cache
└── producer.lock           ← Process lock file
```

### What Survives Restart

| Data | Storage Location | Persisted |
|------|------------------|-----------|
| All blocks | `blocks/*.sst` | **YES** |
| Block headers | `blocks/` (CF_HEADERS) | **YES** |
| Transactions | `blocks/` (CF_BODIES) | **YES** |
| Height index | `blocks/` (CF_HEIGHT_INDEX) | **YES** |
| Slot index | `blocks/` (CF_SLOT_INDEX) | **YES** |
| Chain tip | `blocks/` | **YES** |
| Signed slots | `signed_slots.db/` | **YES** |

### Restart Verification Checklist

Before deploying the refactoring, verify data persistence:

```bash
# 1. Record current state BEFORE restart
BEFORE_HEIGHT=$(curl -s -X POST http://127.0.0.1:18545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | jq -r '.result.bestHeight')
BEFORE_HASH=$(curl -s -X POST http://127.0.0.1:18545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | jq -r '.result.bestHash')
echo "Before restart: height=$BEFORE_HEIGHT hash=$BEFORE_HASH"

# 2. Stop all nodes
pkill -f doli-node

# 3. Verify RocksDB files still exist
ls -la ~/.doli/testnet/node1/data/blocks/

# 4. Restart nodes
# (use your launch script or manual commands)

# 5. Wait for nodes to initialize (10-30 seconds)
sleep 30

# 6. Verify chain state matches
AFTER_HEIGHT=$(curl -s -X POST http://127.0.0.1:18545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | jq -r '.result.bestHeight')
AFTER_HASH=$(curl -s -X POST http://127.0.0.1:18545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | jq -r '.result.bestHash')
echo "After restart: height=$AFTER_HEIGHT hash=$AFTER_HASH"

# 7. Compare (height should be >= before, hash at same height should match)
if [ "$AFTER_HEIGHT" -ge "$BEFORE_HEIGHT" ]; then
  echo "✓ Chain height preserved (before: $BEFORE_HEIGHT, after: $AFTER_HEIGHT)"
else
  echo "✗ ERROR: Chain height lost!"
fi
```

### Expected Behavior on Restart

```
Node starts
    │
    ├── Opens RocksDB at data/blocks/
    │
    ├── Reads chain tip from storage
    │   └── Recovers: best_height, best_hash, best_slot
    │
    ├── Loads producer set from producer_gset.bin
    │
    ├── Resumes from last known block
    │   └── NO re-sync from genesis required
    │
    └── Ready to produce/validate new blocks
```

### Multi-Node Restart Test

```bash
#!/bin/bash
# test_restart_persistence.sh

echo "=== Testing Data Persistence Across Restart ==="

# Get state from all nodes before restart
echo "Recording pre-restart state..."
for port in 18545 18546 18547 18548 18549; do
  HEIGHT=$(curl -s -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | jq -r '.result.bestHeight')
  echo "Node $port: height=$HEIGHT"
done

# Stop all nodes
echo "Stopping all nodes..."
pkill -f doli-node
sleep 5

# Restart nodes (adjust paths as needed)
echo "Restarting nodes..."
# ./scripts/launch_testnet.sh
sleep 30

# Verify state after restart
echo "Verifying post-restart state..."
for port in 18545 18546 18547 18548 18549; do
  HEIGHT=$(curl -s -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | jq -r '.result.bestHeight')
  echo "Node $port: height=$HEIGHT"
done

echo "=== Persistence Test Complete ==="
```

### Mainnet Upgrade Procedure

For mainnet upgrades with the new reward system:

1. **Announce maintenance window** (coordinate with all producers)
2. **Stop all nodes gracefully** (`SIGTERM`, not `SIGKILL`)
3. **Backup data directories** (optional but recommended)
4. **Upgrade binaries** on all nodes
5. **Restart nodes** (any order, they'll sync)
6. **Verify chain continuity** using checklist above
7. **Monitor first epoch boundary** for correct reward distribution

### Data Recovery

If a node loses its data directory:

```bash
# Option 1: Sync from peers (automatic)
# Just start the node with empty data dir - it will sync from network

# Option 2: Copy from another node (faster for large chains)
rsync -av node1/data/blocks/ node2/data/blocks/

# Option 3: Restore from backup
tar -xzf backup-blocks.tar.gz -C node1/data/
```

---

## Appendix: File Change Summary

| File | Changes |
|------|---------|
| `crates/storage/src/block_store.rs` | Add 3 query methods |
| `bins/node/src/node.rs` | Remove state, add calculation |
| `crates/core/src/validation.rs` | Exact validation |
| `crates/storage/src/chain_state.rs` | Remove/deprecate fields |
| `crates/storage/src/producer.rs` | Optional: remove unused fields |

---

**End of Document**
