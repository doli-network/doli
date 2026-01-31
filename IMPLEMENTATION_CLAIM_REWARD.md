# IMPLEMENTATION_CLAIM_REWARD.md

# Weighted Presence Rewards with On-Demand Epoch Claims

**Date**: 2026-01-31
**Status**: Design Complete - Ready for Implementation
**Version**: 2.0 (Revised)

---

## 1. Executive Summary

### Problem Statement

The current automatic epoch reward distribution system has critical bugs and fundamental design issues:

1. **Empty Epoch Catch-up Loop**: System stuck trying to reward empty epochs forever
2. **Rewards Only Block Producers**: Non-producing but present validators earn nothing
3. **No Capital Efficiency**: Large bond holders don't earn proportionally more
4. **Complex Epoch Boundary Logic**: ~500 lines of buggy automatic distribution code

### Proposed Solution: Weighted Presence + Epoch Claims

A hybrid approach combining the best of two designs:

| Component | Source | Description |
|-----------|--------|-------------|
| **Weighted Presence** | Alternative Proposal | All present producers earn proportional to bond weight |
| **Epoch-Based Claims** | Original Proposal | Clean boundaries, O(360) scan, no checkpoints |
| **Block-Based Epochs** | New | Epochs defined by block height, not slots |

### Core Formula

```
For each block where producer was present:
  producer_reward += block_reward × producer_weight / total_present_weight

Claim covers one epoch (BLOCKS_PER_REWARD_EPOCH blocks)
```

### Key Benefits

| Benefit | Description |
|---------|-------------|
| **All Present Earn** | Not just block producer - everyone proving presence |
| **Capital Efficient** | 2x bond = 2x reward (proportional) |
| **No Checkpoints** | Epochs provide natural O(360) boundaries |
| **Simple Constant** | `BLOCKS_PER_REWARD_EPOCH = 360` easily tunable |
| **Block-Based** | Sequential heights, no slot gaps to handle |
| **Deterministic** | Same calculation on every node |

---

## 2. Block-Based Epoch System

### Why Block Height Instead of Slots?

| Aspect | Slot-Based Epochs | Block-Based Epochs |
|--------|-------------------|---------------------|
| **Gaps** | Empty slots create uneven epochs | No gaps - heights are sequential |
| **Predictability** | Variable blocks per epoch | Exactly N blocks per epoch |
| **Calculation** | Must handle missing slots | Simple division |
| **Constants** | `SLOTS_PER_REWARD_EPOCH` | `BLOCKS_PER_REWARD_EPOCH` |

### Epoch Constants

```rust
// crates/core/src/consensus.rs

/// Number of blocks per reward epoch
/// Easily modifiable constant for tuning reward frequency
///
/// Examples:
///   360 blocks ≈ 1 hour at 10s blocks (mainnet)
///   60 blocks  ≈ 1 minute at 1s blocks (devnet)
///   8640 blocks ≈ 24 hours (daily rewards)
pub const BLOCKS_PER_REWARD_EPOCH: u64 = 360;

/// Calculate epoch from block height
#[inline]
pub fn height_to_reward_epoch(height: BlockHeight) -> u64 {
    height / BLOCKS_PER_REWARD_EPOCH
}

/// Calculate epoch boundaries from epoch number
#[inline]
pub fn reward_epoch_boundaries(epoch: u64) -> (BlockHeight, BlockHeight) {
    let start = epoch * BLOCKS_PER_REWARD_EPOCH;
    let end = start + BLOCKS_PER_REWARD_EPOCH;
    (start, end)
}

/// Check if height is first block of an epoch
#[inline]
pub fn is_epoch_start(height: BlockHeight) -> bool {
    height % BLOCKS_PER_REWARD_EPOCH == 0
}
```

### Network-Specific Overrides

```rust
impl Network {
    pub fn blocks_per_reward_epoch(&self) -> u64 {
        match self {
            Network::Mainnet => 360,   // ~1 hour
            Network::Testnet => 360,   // ~1 hour
            Network::Devnet => 60,     // ~1 minute
        }
    }
}
```

### Epoch Examples

```
BLOCKS_PER_REWARD_EPOCH = 360

Epoch 0: blocks 0-359     (360 blocks)
Epoch 1: blocks 360-719   (360 blocks)
Epoch 2: blocks 720-1079  (360 blocks)
...
Epoch N: blocks N×360 to (N+1)×360-1

Block 0    → Epoch 0
Block 359  → Epoch 0
Block 360  → Epoch 1
Block 1000 → Epoch 2 (1000/360 = 2)
```

---

## 3. Architecture Overview

### System Diagram

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           PRESENCE TRACKING                                  │
│                                                                              │
│  Each Slot (10s):                                                           │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │ 1. Producers compute VDF heartbeat                                   │   │
│  │ 2. Broadcast heartbeat to network                                    │   │
│  │ 3. Collect 2+ witness signatures                                     │   │
│  │ 4. Block producer records presence in block                          │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                                                                              │
│  Block N:                                                                    │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │ Header: prev_hash, merkle_root, timestamp, slot, producer, vdf...   │   │
│  │                                                                      │   │
│  │ PresenceCommitment:                                                  │   │
│  │   bitfield: [1,1,0,1,1,0,1,1,1,0...]  (1 bit per producer)          │   │
│  │   weights:  [1000, 2000, 1500, ...]   (bond amounts of present)     │   │
│  │   total_weight: 4500                   (sum for quick access)       │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────────┘

                                    │
                                    │ Blocks accumulate in epoch
                                    ▼

┌─────────────────────────────────────────────────────────────────────────────┐
│                           EPOCH ACCUMULATION                                 │
│                                                                              │
│  Epoch 5 (blocks 1800-2159):                                                │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │ Block 1800: Alice present (w=1000), Bob present (w=2000)            │   │
│  │ Block 1801: Alice present (w=1000), Bob absent, Carol present (w=1500)│  │
│  │ Block 1802: Alice absent, Bob present (w=2000), Carol present (w=1500)│  │
│  │ ...                                                                  │   │
│  │ Block 2159: Final block of epoch 5                                   │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                                                                              │
│  Epoch 5 completes when block 2160 (epoch 6) is produced                    │
└─────────────────────────────────────────────────────────────────────────────┘

                                    │
                                    │ Producer decides to claim
                                    ▼

┌─────────────────────────────────────────────────────────────────────────────┐
│                           CLAIM PROCESS                                      │
│                                                                              │
│  ClaimEpochReward Transaction:                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │ epoch: 5                                                             │   │
│  │ producer: Alice (pubkey)                                             │   │
│  │ amount: 47,500,000,000 (47.5 DOLI)                                  │   │
│  │ recipient: Alice's address                                           │   │
│  │ signature: Alice signs claim                                         │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                                                                              │
│  Validation:                                                                 │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │ 1. Epoch 5 complete? (current_height >= 2160)               ✓       │   │
│  │ 2. Already claimed? (check ClaimRegistry)                   ✓       │   │
│  │ 3. Scan blocks 1800-2159, sum Alice's weighted rewards      ✓       │   │
│  │ 4. Amount matches calculated?                               ✓       │   │
│  │ 5. Signature valid?                                         ✓       │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                                                                              │
│  Result: UTXO minted, ClaimRegistry updated                                 │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Component Interactions

```
┌──────────────┐     ┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│   Producer   │     │    Node      │     │  BlockStore  │     │ClaimRegistry │
└──────┬───────┘     └──────┬───────┘     └──────┬───────┘     └──────┬───────┘
       │                    │                    │                    │
       │  1. Heartbeat      │                    │                    │
       │───────────────────▶│                    │                    │
       │                    │                    │                    │
       │  2. Witness sigs   │                    │                    │
       │◀──────────────────▶│                    │                    │
       │                    │                    │                    │
       │                    │  3. Store block    │                    │
       │                    │   with presence    │                    │
       │                    │───────────────────▶│                    │
       │                    │                    │                    │
       │  4. ClaimEpochTx   │                    │                    │
       │───────────────────▶│                    │                    │
       │                    │                    │                    │
       │                    │  5. Get blocks     │                    │
       │                    │   in epoch range   │                    │
       │                    │───────────────────▶│                    │
       │                    │                    │                    │
       │                    │  6. Blocks + presence                   │
       │                    │◀───────────────────│                    │
       │                    │                    │                    │
       │                    │  7. Calculate weighted reward           │
       │                    │     (sum over 360 blocks)               │
       │                    │                    │                    │
       │                    │  8. Check claimed  │                    │
       │                    │───────────────────────────────────────▶│
       │                    │                    │                    │
       │                    │  9. Not claimed    │                    │
       │                    │◀───────────────────────────────────────│
       │                    │                    │                    │
       │                    │  10. Validate & include in block        │
       │                    │                    │                    │
       │                    │  11. Mark claimed  │                    │
       │                    │───────────────────────────────────────▶│
       │                    │                    │                    │
       │  12. UTXO created  │                    │                    │
       │◀───────────────────│                    │                    │
       │                    │                    │                    │
```

### Data Flow Summary

```
1. PRESENCE RECORDING (every block)
   Heartbeats → Witness Sigs → Block Producer → PresenceCommitment in Block

2. REWARD ACCUMULATION (virtual)
   No on-chain state - rewards calculated on-demand from block history

3. CLAIM EXECUTION (on-demand)
   ClaimTx → Validate → Calculate → Mint UTXO → Update Registry
```

---

## 4. Implementation Milestones

### Milestone 1: Epoch Constants and Utilities ✅ COMPLETED
**~100 lines changed**
**Status:** Implemented 2026-01-31, all tests passing

Add block-based epoch system with easily modifiable constants.

**Files Affected:**
- `crates/core/src/consensus.rs`
- `crates/core/src/lib.rs`
- `crates/core/src/network.rs` (added `blocks_per_reward_epoch()` method)

**Changes Summary:**
```rust
// consensus.rs

/// Blocks per reward epoch - THE key tunable constant
/// Change this single value to adjust reward frequency
pub const BLOCKS_PER_REWARD_EPOCH: u64 = 360;

/// Reward epoch utilities
pub mod reward_epoch {
    use super::BLOCKS_PER_REWARD_EPOCH;
    use crate::BlockHeight;

    /// Get epoch number from block height
    #[inline]
    pub fn from_height(height: BlockHeight) -> u64 {
        height / BLOCKS_PER_REWARD_EPOCH
    }

    /// Get (start_height, end_height) for epoch
    /// Note: end is exclusive (start..end)
    #[inline]
    pub fn boundaries(epoch: u64) -> (BlockHeight, BlockHeight) {
        let start = epoch * BLOCKS_PER_REWARD_EPOCH;
        let end = start + BLOCKS_PER_REWARD_EPOCH;
        (start, end)
    }

    /// Check if epoch is complete given current height
    #[inline]
    pub fn is_complete(epoch: u64, current_height: BlockHeight) -> bool {
        let (_, end) = boundaries(epoch);
        current_height >= end
    }

    /// Get current epoch from height
    #[inline]
    pub fn current(height: BlockHeight) -> u64 {
        from_height(height)
    }

    /// Get last complete epoch from height
    #[inline]
    pub fn last_complete(height: BlockHeight) -> Option<u64> {
        let current = from_height(height);
        if current > 0 { Some(current - 1) } else { None }
    }
}
```

**Dependencies:** None

**Test Criteria:**
- [x] `from_height(0)` returns 0
- [x] `from_height(359)` returns 0
- [x] `from_height(360)` returns 1
- [x] `boundaries(0)` returns (0, 360)
- [x] `boundaries(5)` returns (1800, 2160)
- [x] `is_complete(0, 359)` returns false
- [x] `is_complete(0, 360)` returns true

**Additional tests implemented:**
- [x] `last_complete()` returns None for incomplete epochs
- [x] `is_epoch_start()` detects epoch boundaries
- [x] `complete_epochs()` counts completed epochs
- [x] `blocks_per_epoch()` returns constant value

---

### Milestone 2: Presence Commitment Structure
**~200 lines changed**

Add presence tracking data structure to blocks.

**Files Affected:**
- `crates/core/src/block.rs`
- `crates/core/src/presence.rs` (NEW)

**Changes Summary:**
```rust
// presence.rs (NEW FILE)

use crate::{Hash, PublicKey, Amount};
use serde::{Deserialize, Serialize};

/// Compact presence commitment stored in each block
/// Records which producers were present and their weights
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceCommitment {
    /// Bitfield: 1 bit per registered producer (sorted by pubkey)
    /// Bit i = 1 means producer i was present this slot
    pub bitfield: Vec<u8>,

    /// Merkle root of full heartbeat data (for fraud proofs)
    pub merkle_root: Hash,

    /// Bond weights of present producers (in bitfield order)
    /// Only includes weights for producers with bit=1
    pub weights: Vec<Amount>,

    /// Cached sum of all weights (for quick reward calculation)
    pub total_weight: Amount,
}

impl PresenceCommitment {
    /// Create new presence commitment
    pub fn new(
        producer_count: usize,
        present_indices: &[usize],
        weights: Vec<Amount>,
        merkle_root: Hash,
    ) -> Self {
        let mut bitfield = vec![0u8; (producer_count + 7) / 8];
        for &idx in present_indices {
            bitfield[idx / 8] |= 1 << (idx % 8);
        }
        let total_weight = weights.iter().sum();
        Self { bitfield, merkle_root, weights, total_weight }
    }

    /// Check if producer at index was present
    #[inline]
    pub fn is_present(&self, producer_index: usize) -> bool {
        let byte_idx = producer_index / 8;
        let bit_idx = producer_index % 8;
        if byte_idx >= self.bitfield.len() {
            return false;
        }
        (self.bitfield[byte_idx] & (1 << bit_idx)) != 0
    }

    /// Get weight for producer if present
    pub fn get_weight(&self, producer_index: usize) -> Option<Amount> {
        if !self.is_present(producer_index) {
            return None;
        }
        // Count set bits before this index to find weight array position
        let weight_idx = self.count_present_before(producer_index);
        self.weights.get(weight_idx).copied()
    }

    /// Count present producers before index
    fn count_present_before(&self, producer_index: usize) -> usize {
        let mut count = 0;
        for i in 0..producer_index {
            if self.is_present(i) {
                count += 1;
            }
        }
        count
    }

    /// Number of present producers
    pub fn present_count(&self) -> usize {
        self.weights.len()
    }

    /// Serialized size in bytes
    pub fn size(&self) -> usize {
        self.bitfield.len() + 32 + (self.weights.len() * 8) + 8
    }
}

// block.rs - Add presence to Block

/// Block with presence commitment
pub struct Block {
    pub header: BlockHeader,
    pub transactions: Vec<Transaction>,
    pub presence: PresenceCommitment,  // NEW
}
```

**Dependencies:** Milestone 1

**Test Criteria:**
- [ ] Bitfield correctly encodes presence
- [ ] `is_present()` returns correct values
- [ ] `get_weight()` returns correct weight for present producers
- [ ] `get_weight()` returns None for absent producers
- [ ] Serialization/deserialization roundtrips correctly

---

### Milestone 3: Heartbeat VDF and Witness System
**~400 lines changed**

Implement VDF heartbeat proofs and witness signature collection.

**Files Affected:**
- `crates/core/src/heartbeat.rs` (NEW)
- `crates/network/src/gossip.rs`
- `bins/node/src/producer/heartbeat.rs` (NEW)

**Changes Summary:**
```rust
// heartbeat.rs (NEW FILE)

use crate::{Hash, PublicKey, Signature, Slot};
use serde::{Deserialize, Serialize};

/// VDF iterations for heartbeat proof (~1 second)
pub const HEARTBEAT_VDF_ITERATIONS: u64 = 10_000_000;

/// Minimum witness signatures required
pub const MIN_WITNESS_SIGNATURES: usize = 2;

/// Heartbeat proof of presence
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Heartbeat {
    /// Protocol version
    pub version: u8,

    /// Producer's public key
    pub producer: PublicKey,

    /// Current slot number
    pub slot: Slot,

    /// Previous block hash (prevents pre-computation)
    pub prev_block_hash: Hash,

    /// VDF output (hash chain result)
    pub vdf_output: [u8; 32],

    /// Producer's signature over heartbeat data
    pub signature: Signature,

    /// Witness signatures from other producers
    pub witnesses: Vec<WitnessSignature>,
}

/// Witness attestation that heartbeat is valid
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WitnessSignature {
    pub witness: PublicKey,
    pub signature: Signature,
}

impl Heartbeat {
    /// Compute VDF input for heartbeat
    pub fn vdf_input(producer: &PublicKey, slot: Slot, prev_hash: &Hash) -> [u8; 32] {
        use crate::crypto::hash_with_domain;
        hash_with_domain(
            b"DOLI_HEARTBEAT_V1",
            &[
                producer.as_bytes(),
                &slot.to_le_bytes(),
                prev_hash.as_bytes(),
            ].concat(),
        ).into()
    }

    /// Verify heartbeat VDF proof
    pub fn verify_vdf(&self) -> bool {
        let input = Self::vdf_input(&self.producer, self.slot, &self.prev_block_hash);
        let expected = crate::vdf::hash_chain_vdf(&input, HEARTBEAT_VDF_ITERATIONS);
        self.vdf_output == expected
    }

    /// Verify producer signature
    pub fn verify_signature(&self) -> bool {
        let message = self.signing_message();
        crate::crypto::verify_hash(&message, &self.signature, &self.producer).is_ok()
    }

    /// Verify all witness signatures
    pub fn verify_witnesses(&self, active_producers: &[PublicKey]) -> bool {
        if self.witnesses.len() < MIN_WITNESS_SIGNATURES {
            return false;
        }
        for witness in &self.witnesses {
            // Witness must be active producer (not self)
            if !active_producers.contains(&witness.witness) {
                return false;
            }
            if witness.witness == self.producer {
                return false;
            }
            // Verify signature
            let message = self.witness_message();
            if crate::crypto::verify_hash(&message, &witness.signature, &witness.witness).is_err() {
                return false;
            }
        }
        true
    }

    /// Message signed by producer
    fn signing_message(&self) -> Hash {
        crate::crypto::hash_with_domain(
            b"DOLI_HEARTBEAT_SIGN_V1",
            &[
                &[self.version],
                self.producer.as_bytes(),
                &self.slot.to_le_bytes(),
                self.prev_block_hash.as_bytes(),
                &self.vdf_output,
            ].concat(),
        )
    }

    /// Message signed by witnesses
    fn witness_message(&self) -> Hash {
        crate::crypto::hash_with_domain(
            b"DOLI_HEARTBEAT_WITNESS_V1",
            &[
                self.producer.as_bytes(),
                &self.slot.to_le_bytes(),
                &self.vdf_output,
            ].concat(),
        )
    }
}
```

**Dependencies:** Milestone 2

**Test Criteria:**
- [ ] VDF computation takes ~1 second
- [ ] VDF verification is fast (< 10ms)
- [ ] Signature verification works
- [ ] Witness signatures validated
- [ ] Rejects heartbeat with < 2 witnesses
- [ ] Rejects self-witnessing

---

### Milestone 4: Claim Registry Storage
**~150 lines changed**

Add RocksDB storage for tracking claimed (producer, epoch) pairs.

**Files Affected:**
- `crates/storage/src/lib.rs`
- `crates/storage/src/claim_registry.rs` (NEW)

**Changes Summary:**
```rust
// claim_registry.rs (NEW FILE)

use crate::{Hash, PublicKey, Amount, BlockHeight};
use rocksdb::{DB, ColumnFamily};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Column family name for claim registry
pub const CF_CLAIMED_EPOCHS: &str = "claimed_epochs";

/// Record of a completed claim
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClaimRecord {
    /// Transaction hash that executed the claim
    pub tx_hash: Hash,
    /// Block height where claim was confirmed
    pub height: BlockHeight,
    /// Amount claimed
    pub amount: Amount,
    /// Timestamp of claim
    pub timestamp: u64,
}

/// Registry tracking which (producer, epoch) pairs have been claimed
pub struct ClaimRegistry {
    db: Arc<DB>,
}

impl ClaimRegistry {
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    /// Generate deterministic key for (producer, epoch) pair
    fn claim_key(producer: &PublicKey, epoch: u64) -> [u8; 32] {
        use crate::crypto::hash_with_domain;
        hash_with_domain(
            b"DOLI_CLAIM_KEY_V1",
            &[producer.as_bytes(), &epoch.to_le_bytes()].concat(),
        ).into()
    }

    /// Check if (producer, epoch) has been claimed
    pub fn is_claimed(&self, producer: &PublicKey, epoch: u64) -> Result<bool, StorageError> {
        let key = Self::claim_key(producer, epoch);
        let cf = self.cf_handle()?;
        Ok(self.db.get_cf(cf, key)?.is_some())
    }

    /// Mark (producer, epoch) as claimed
    pub fn mark_claimed(
        &self,
        producer: &PublicKey,
        epoch: u64,
        record: &ClaimRecord,
    ) -> Result<(), StorageError> {
        let key = Self::claim_key(producer, epoch);
        let value = bincode::serialize(record)?;
        let cf = self.cf_handle()?;
        self.db.put_cf(cf, key, value)?;
        Ok(())
    }

    /// Get claim record if exists
    pub fn get_claim(
        &self,
        producer: &PublicKey,
        epoch: u64,
    ) -> Result<Option<ClaimRecord>, StorageError> {
        let key = Self::claim_key(producer, epoch);
        let cf = self.cf_handle()?;
        match self.db.get_cf(cf, key)? {
            Some(bytes) => Ok(Some(bincode::deserialize(&bytes)?)),
            None => Ok(None),
        }
    }

    /// Get list of unclaimed epochs for a producer in range
    pub fn get_unclaimed_epochs(
        &self,
        producer: &PublicKey,
        start_epoch: u64,
        end_epoch: u64,
    ) -> Result<Vec<u64>, StorageError> {
        let mut unclaimed = Vec::new();
        for epoch in start_epoch..end_epoch {
            if !self.is_claimed(producer, epoch)? {
                unclaimed.push(epoch);
            }
        }
        Ok(unclaimed)
    }

    /// Revert a claim (for reorg handling)
    pub fn revert_claim(&self, producer: &PublicKey, epoch: u64) -> Result<(), StorageError> {
        let key = Self::claim_key(producer, epoch);
        let cf = self.cf_handle()?;
        self.db.delete_cf(cf, key)?;
        Ok(())
    }

    fn cf_handle(&self) -> Result<&ColumnFamily, StorageError> {
        self.db.cf_handle(CF_CLAIMED_EPOCHS)
            .ok_or(StorageError::ColumnFamilyNotFound(CF_CLAIMED_EPOCHS.to_string()))
    }
}
```

**Dependencies:** Milestone 1

**Test Criteria:**
- [ ] Can mark epoch as claimed
- [ ] `is_claimed()` returns true after marking
- [ ] Double-mark is idempotent
- [ ] `get_unclaimed_epochs()` returns correct list
- [ ] Survives node restart
- [ ] Revert removes claim correctly

---

### Milestone 5: ClaimEpochReward Transaction Type
**~250 lines changed**

Add new transaction type for claiming weighted presence rewards.

**Files Affected:**
- `crates/core/src/transaction.rs`

**Changes Summary:**
```rust
// transaction.rs

/// Transaction type for claiming epoch presence rewards
pub const TX_TYPE_CLAIM_EPOCH_REWARD: u32 = 11;

/// Data for ClaimEpochReward transaction
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimEpochRewardData {
    /// Epoch number being claimed
    pub epoch: u64,

    /// Claiming producer's public key
    pub producer_pubkey: PublicKey,

    /// Recipient address (can differ from producer)
    pub recipient_hash: Hash,
}

impl ClaimEpochRewardData {
    pub fn new(epoch: u64, producer_pubkey: PublicKey, recipient_hash: Hash) -> Self {
        Self { epoch, producer_pubkey, recipient_hash }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(72);
        bytes.extend_from_slice(&self.epoch.to_le_bytes());         // 8 bytes
        bytes.extend_from_slice(self.producer_pubkey.as_bytes());   // 32 bytes
        bytes.extend_from_slice(self.recipient_hash.as_bytes());    // 32 bytes
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 72 { return None; }
        let epoch = u64::from_le_bytes(bytes[0..8].try_into().ok()?);
        let producer_pubkey = PublicKey::from_bytes(&bytes[8..40])?;
        let recipient_hash = Hash::from_bytes(&bytes[40..72])?;
        Some(Self { epoch, producer_pubkey, recipient_hash })
    }
}

impl Transaction {
    /// Create a new claim epoch reward transaction
    pub fn new_claim_epoch_reward(
        epoch: u64,
        producer_pubkey: PublicKey,
        amount: Amount,
        recipient_hash: Hash,
        signature: Signature,
    ) -> Self {
        let data = ClaimEpochRewardData::new(epoch, producer_pubkey, recipient_hash);
        let mut extra_data = data.to_bytes();
        extra_data.extend_from_slice(signature.as_bytes()); // 64 bytes

        Self {
            version: 1,
            tx_type: TxType::ClaimEpochReward,
            inputs: Vec::new(),  // Minted - no inputs
            outputs: vec![Output::normal(amount, recipient_hash)],
            extra_data,
        }
    }

    /// Check if this is a claim epoch reward transaction
    pub fn is_claim_epoch_reward(&self) -> bool {
        self.tx_type == TxType::ClaimEpochReward
    }

    /// Parse claim data from transaction
    pub fn claim_epoch_reward_data(&self) -> Option<ClaimEpochRewardData> {
        if !self.is_claim_epoch_reward() { return None; }
        ClaimEpochRewardData::from_bytes(&self.extra_data)
    }

    /// Get signature from claim transaction
    pub fn claim_signature(&self) -> Option<Signature> {
        if !self.is_claim_epoch_reward() { return None; }
        if self.extra_data.len() < 136 { return None; }
        Signature::from_bytes(&self.extra_data[72..136])
    }
}
```

**Dependencies:** Milestone 4

**Test Criteria:**
- [ ] Transaction serializes correctly
- [ ] Data parsing works
- [ ] Signature extraction works
- [ ] Output contains correct amount and recipient

---

### Milestone 6: Weighted Reward Calculation
**~300 lines changed**

Implement deterministic weighted presence reward calculation.

**Files Affected:**
- `crates/core/src/rewards.rs` (NEW)
- `crates/storage/src/block_store.rs`

**Changes Summary:**
```rust
// rewards.rs (NEW FILE)

use crate::{
    consensus::{reward_epoch, BLOCKS_PER_REWARD_EPOCH},
    block::Block,
    presence::PresenceCommitment,
    Amount, BlockHeight, PublicKey,
};

/// Result of reward calculation
#[derive(Debug, Clone)]
pub struct WeightedRewardCalculation {
    /// Epoch calculated
    pub epoch: u64,
    /// Producer public key
    pub producer: PublicKey,
    /// Blocks where producer was present
    pub blocks_present: u64,
    /// Total blocks in epoch
    pub total_blocks: u64,
    /// Sum of producer's weight across all present blocks
    pub total_producer_weight: Amount,
    /// Sum of all weights across all present blocks
    pub total_all_weights: Amount,
    /// Block reward per block
    pub block_reward: Amount,
    /// Final calculated reward amount
    pub reward_amount: Amount,
}

/// Calculator for weighted presence rewards
pub struct WeightedRewardCalculator<'a, B: BlockSource> {
    block_source: &'a B,
    params: &'a ConsensusParams,
}

/// Trait for accessing blocks (implemented by BlockStore)
pub trait BlockSource {
    fn get_block_by_height(&self, height: BlockHeight) -> Result<Option<Block>, StorageError>;
}

impl<'a, B: BlockSource> WeightedRewardCalculator<'a, B> {
    pub fn new(block_source: &'a B, params: &'a ConsensusParams) -> Self {
        Self { block_source, params }
    }

    /// Calculate weighted presence reward for a producer in an epoch
    ///
    /// Formula for each block where producer was present:
    ///   reward += block_reward × producer_weight / total_present_weight
    ///
    /// Scan: exactly BLOCKS_PER_REWARD_EPOCH blocks (e.g., 360)
    pub fn calculate_producer_reward(
        &self,
        producer: &PublicKey,
        producer_index: usize,
        epoch: u64,
    ) -> Result<WeightedRewardCalculation, RewardError> {
        let (start_height, end_height) = reward_epoch::boundaries(epoch);

        let mut blocks_present: u64 = 0;
        let mut total_blocks: u64 = 0;
        let mut total_producer_weight: Amount = 0;
        let mut total_all_weights: Amount = 0;
        let mut reward_amount: Amount = 0;

        // Scan all blocks in epoch
        for height in start_height..end_height {
            let block = match self.block_source.get_block_by_height(height)? {
                Some(b) => b,
                None => continue,  // Block not yet produced (shouldn't happen for complete epochs)
            };

            total_blocks += 1;
            let presence = &block.presence;

            // Check if producer was present
            if let Some(weight) = presence.get_weight(producer_index) {
                blocks_present += 1;
                total_producer_weight += weight;
                total_all_weights += presence.total_weight;

                // Calculate reward for this block
                // reward = block_reward × weight / total_weight
                // Use u128 to prevent overflow
                let block_reward = self.params.block_reward(height);
                let numerator = (block_reward as u128) * (weight as u128);
                let block_share = (numerator / (presence.total_weight as u128)) as Amount;

                reward_amount += block_share;
            }
        }

        Ok(WeightedRewardCalculation {
            epoch,
            producer: producer.clone(),
            blocks_present,
            total_blocks,
            total_producer_weight,
            total_all_weights,
            block_reward: self.params.block_reward(start_height),
            reward_amount,
        })
    }

    /// Get summary of all epochs for a producer
    pub fn get_claimable_summary(
        &self,
        producer: &PublicKey,
        producer_index: usize,
        claim_registry: &ClaimRegistry,
        current_height: BlockHeight,
    ) -> Result<Vec<ClaimableSummary>, RewardError> {
        let current_epoch = reward_epoch::from_height(current_height);
        let mut summaries = Vec::new();

        // Check all complete epochs
        for epoch in 0..current_epoch {
            let is_claimed = claim_registry.is_claimed(producer, epoch)?;

            if !is_claimed {
                let calc = self.calculate_producer_reward(producer, producer_index, epoch)?;
                if calc.reward_amount > 0 {
                    summaries.push(ClaimableSummary {
                        epoch,
                        blocks_present: calc.blocks_present,
                        estimated_reward: calc.reward_amount,
                        is_claimed: false,
                        claim_tx_hash: None,
                    });
                }
            }
        }

        Ok(summaries)
    }
}

/// Summary for UI display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimableSummary {
    pub epoch: u64,
    pub blocks_present: u64,
    pub estimated_reward: Amount,
    pub is_claimed: bool,
    pub claim_tx_hash: Option<Hash>,
}
```

**Dependencies:** Milestone 5

**Test Criteria:**
- [ ] Single producer, all blocks present → 100% of rewards
- [ ] Two producers 50/50 weight, both present → 50% each
- [ ] Producer absent some blocks → proportionally less
- [ ] Empty epoch → zero reward
- [ ] Calculation deterministic (same result every time)
- [ ] u128 math prevents overflow

---

### Milestone 7: Claim Validation
**~350 lines changed**

Implement complete validation for ClaimEpochReward transactions.

**Files Affected:**
- `crates/core/src/validation.rs`

**Changes Summary:**
```rust
// validation.rs

/// Validate a ClaimEpochReward transaction
pub fn validate_claim_epoch_reward<B: BlockSource>(
    tx: &Transaction,
    context: &ValidationContext,
    block_source: &B,
    claim_registry: &ClaimRegistry,
    producer_set: &ProducerSet,
) -> Result<(), ValidationError> {
    // 1. STRUCTURAL VALIDATION
    if tx.tx_type != TxType::ClaimEpochReward {
        return Err(ValidationError::WrongTxType);
    }
    if !tx.inputs.is_empty() {
        return Err(ValidationError::ClaimMustHaveNoInputs);
    }
    if tx.outputs.len() != 1 {
        return Err(ValidationError::ClaimMustHaveOneOutput);
    }
    if tx.extra_data.len() < 136 {
        return Err(ValidationError::InvalidClaimData);
    }

    // 2. PARSE CLAIM DATA
    let claim_data = ClaimEpochRewardData::from_bytes(&tx.extra_data)
        .ok_or(ValidationError::InvalidClaimData)?;
    let signature = Signature::from_bytes(&tx.extra_data[72..136])
        .ok_or(ValidationError::InvalidSignature)?;

    // 3. VERIFY EPOCH IS COMPLETE
    let current_epoch = reward_epoch::from_height(context.current_height);
    if claim_data.epoch >= current_epoch {
        return Err(ValidationError::EpochNotComplete {
            claimed: claim_data.epoch,
            current: current_epoch,
        });
    }

    // 4. VERIFY NOT ALREADY CLAIMED
    if claim_registry.is_claimed(&claim_data.producer_pubkey, claim_data.epoch)? {
        return Err(ValidationError::EpochAlreadyClaimed {
            producer: claim_data.producer_pubkey.clone(),
            epoch: claim_data.epoch,
        });
    }

    // 5. VERIFY PRODUCER IS REGISTERED
    let producer_info = producer_set.get(&claim_data.producer_pubkey)
        .ok_or(ValidationError::ProducerNotRegistered)?;
    let producer_index = producer_set.get_index(&claim_data.producer_pubkey)
        .ok_or(ValidationError::ProducerNotRegistered)?;

    // 6. CALCULATE EXPECTED REWARD
    let calculator = WeightedRewardCalculator::new(block_source, &context.params);
    let expected = calculator.calculate_producer_reward(
        &claim_data.producer_pubkey,
        producer_index,
        claim_data.epoch,
    )?;

    // 7. VERIFY PRODUCER WAS PRESENT
    if expected.blocks_present == 0 {
        return Err(ValidationError::NoPresenceInEpoch {
            producer: claim_data.producer_pubkey.clone(),
            epoch: claim_data.epoch,
        });
    }

    // 8. VERIFY AMOUNT MATCHES
    let claimed_amount = tx.outputs[0].amount;
    if claimed_amount != expected.reward_amount {
        return Err(ValidationError::IncorrectClaimAmount {
            claimed: claimed_amount,
            expected: expected.reward_amount,
        });
    }

    // 9. VERIFY SIGNATURE
    let signing_message = claim_signing_message(&claim_data, claimed_amount);
    crypto::verify_hash(&signing_message, &signature, &claim_data.producer_pubkey)
        .map_err(|_| ValidationError::InvalidSignature)?;

    // 10. VERIFY OUTPUT RECIPIENT
    if tx.outputs[0].pubkey_hash != claim_data.recipient_hash {
        return Err(ValidationError::RecipientMismatch);
    }

    Ok(())
}

/// Generate signing message for claim
fn claim_signing_message(claim_data: &ClaimEpochRewardData, amount: Amount) -> Hash {
    crypto::hash_with_domain(
        b"DOLI_CLAIM_SIGN_V1",
        &[
            &claim_data.epoch.to_le_bytes(),
            claim_data.producer_pubkey.as_bytes(),
            claim_data.recipient_hash.as_bytes(),
            &amount.to_le_bytes(),
        ].concat(),
    )
}
```

**Dependencies:** Milestone 6

**Test Criteria:**
- [ ] Rejects incomplete epoch
- [ ] Rejects double-claim
- [ ] Rejects wrong amount
- [ ] Rejects non-present producer
- [ ] Rejects invalid signature
- [ ] Accepts valid claim

---

### Milestone 8: Block Production with Presence
**~300 lines changed**

Update block production to include presence commitment.

**Files Affected:**
- `bins/node/src/node.rs`
- `bins/node/src/producer/mod.rs`

**Changes Summary:**
```rust
// In node.rs - Block production

async fn produce_block(&mut self) -> Result<Option<Block>> {
    // ... existing slot/producer checks ...

    // 1. Collect valid heartbeats for this slot
    let heartbeats = self.heartbeat_collector.get_valid_heartbeats(current_slot)?;

    // 2. Build presence commitment from heartbeats
    let presence = self.build_presence_commitment(&heartbeats)?;

    // 3. Build block with presence
    let mut builder = BlockBuilder::new(prev_hash, prev_slot, our_pubkey.clone())
        .with_params(self.params.clone())
        .with_presence(presence);

    // 4. Add transactions from mempool
    let mempool_txs = self.mempool.read().await.select_for_block(100);
    for tx in mempool_txs {
        builder.add_transaction(tx);
    }

    // 5. Build and sign block
    let block = builder.build(now)?;

    // 6. Compute VDF and finalize
    // ... existing VDF logic ...

    Ok(Some(block))
}

fn build_presence_commitment(&self, heartbeats: &[ValidatedHeartbeat]) -> Result<PresenceCommitment> {
    let producers = self.producer_set.active_producers_sorted();
    let producer_count = producers.len();

    let mut present_indices = Vec::new();
    let mut weights = Vec::new();

    for (idx, producer) in producers.iter().enumerate() {
        // Check if we have a valid heartbeat from this producer
        if let Some(hb) = heartbeats.iter().find(|h| &h.producer == producer) {
            present_indices.push(idx);
            let bond = self.producer_set.get_bond(producer)?;
            weights.push(bond);
        }
    }

    // Build merkle root from full heartbeat data
    let merkle_root = self.compute_heartbeat_merkle(&heartbeats)?;

    Ok(PresenceCommitment::new(
        producer_count,
        &present_indices,
        weights,
        merkle_root,
    ))
}
```

**Dependencies:** Milestone 7

**Test Criteria:**
- [ ] Block includes presence commitment
- [ ] All valid heartbeats recorded
- [ ] Weights match producer bonds
- [ ] Merkle root computed correctly

---

### Milestone 9: Block Application with Claims
**~200 lines changed**

Update block application to handle claim transactions and registry.

**Files Affected:**
- `bins/node/src/node.rs`

**Changes Summary:**
```rust
// In node.rs - Block application

async fn apply_block(&mut self, block: Block) -> Result<()> {
    let height = self.chain_state.best_height + 1;

    // Validate block (including presence and claims)
    self.validate_block(&block, height)?;

    // Store block
    self.block_store.put_block(&block)?;

    // Apply transactions
    for (tx_idx, tx) in block.transactions.iter().enumerate() {
        match tx.tx_type {
            TxType::ClaimEpochReward => {
                // Parse claim data
                let claim_data = tx.claim_epoch_reward_data()
                    .ok_or(NodeError::InvalidClaimData)?;

                // Mark as claimed in registry
                let record = ClaimRecord {
                    tx_hash: tx.hash(),
                    height,
                    amount: tx.outputs[0].amount,
                    timestamp: block.header.timestamp,
                };
                self.claim_registry.mark_claimed(
                    &claim_data.producer_pubkey,
                    claim_data.epoch,
                    &record,
                )?;

                // Create UTXO with reward maturity
                let entry = UtxoEntry {
                    output: tx.outputs[0].clone(),
                    height,
                    is_coinbase: false,
                    is_epoch_reward: true,  // Requires 100 confirmations
                };
                self.utxo_set.insert(Outpoint::new(tx.hash(), 0), entry)?;
            }
            // ... other tx types ...
        }
    }

    // Update chain state
    self.chain_state.best_hash = block.hash();
    self.chain_state.best_height = height;

    Ok(())
}

/// Revert a block (for reorg handling)
async fn revert_block(&mut self, block: &Block) -> Result<()> {
    for tx in block.transactions.iter().rev() {
        if tx.tx_type == TxType::ClaimEpochReward {
            let claim_data = tx.claim_epoch_reward_data()
                .ok_or(NodeError::InvalidClaimData)?;

            // Revert claim in registry
            self.claim_registry.revert_claim(
                &claim_data.producer_pubkey,
                claim_data.epoch,
            )?;

            // Remove UTXO
            self.utxo_set.remove(&Outpoint::new(tx.hash(), 0))?;
        }
        // ... revert other tx types ...
    }

    Ok(())
}
```

**Dependencies:** Milestone 8

**Test Criteria:**
- [ ] Claims recorded in registry
- [ ] UTXOs created with maturity flag
- [ ] Reorg correctly reverts claims
- [ ] Multiple claims in block work

---

### Milestone 10: Remove Old Reward System
**~-600 lines changed (deletion)**

Remove the buggy automatic epoch distribution code.

**Files Affected:**
- `bins/node/src/node.rs`
- `crates/core/src/validation.rs`
- `crates/core/src/transaction.rs`

**Changes Summary:**
```rust
// REMOVE from node.rs:
// - should_include_epoch_rewards()
// - calculate_epoch_rewards()
// - All RewardMode::EpochPool handling

// REMOVE from validation.rs:
// - validate_epoch_rewards()
// - epoch_needing_rewards()
// - calculate_expected_epoch_rewards()

// DEPRECATE in transaction.rs:
// - TxType::EpochReward (keep for parsing old blocks only)
// - Add validation to reject new EpochReward transactions
```

**Dependencies:** Milestone 9

**Test Criteria:**
- [ ] Old EpochReward transactions in history still parse
- [ ] New EpochReward transactions rejected
- [ ] No automatic reward distribution
- [ ] Build succeeds without old code

---

### Milestone 11: RPC Endpoints
**~200 lines changed**

Add RPC methods for claim management.

**Files Affected:**
- `bins/node/src/rpc/handlers.rs`

**Changes Summary:**
```rust
// New RPC methods

/// Get claimable epochs for a producer
/// GET /rewards/claimable?producer=<pubkey>
async fn get_claimable_rewards(&self, producer: String) -> Result<ClaimableResponse>;

/// Get claim history
/// GET /rewards/history?producer=<pubkey>&limit=<n>
async fn get_claim_history(&self, producer: String, limit: u32) -> Result<Vec<ClaimRecord>>;

/// Estimate reward for specific epoch
/// GET /rewards/estimate?producer=<pubkey>&epoch=<n>
async fn estimate_epoch_reward(&self, producer: String, epoch: u64) -> Result<RewardEstimate>;

/// Build unsigned claim transaction
/// POST /rewards/build-claim
async fn build_claim_tx(&self, req: BuildClaimRequest) -> Result<UnsignedClaimTx>;

/// Get current reward epoch info
/// GET /rewards/epoch-info
async fn get_epoch_info(&self) -> Result<EpochInfo>;
```

**Dependencies:** Milestone 9

**Test Criteria:**
- [ ] All endpoints return correct data
- [ ] Error handling works
- [ ] Build claim produces valid tx structure

---

### Milestone 12: CLI Commands
**~150 lines changed**

Add CLI commands for claiming rewards.

**Files Affected:**
- `bins/cli/src/commands/rewards.rs` (NEW)
- `bins/cli/src/main.rs`

**Changes Summary:**
```rust
// CLI commands

/// doli-cli rewards list
/// List all claimable epochs with estimated rewards

/// doli-cli rewards claim <epoch>
/// Claim rewards for a specific epoch

/// doli-cli rewards claim-all
/// Claim all available rewards (one tx per epoch)

/// doli-cli rewards history
/// Show claim history

/// doli-cli rewards info
/// Show current epoch info and BLOCKS_PER_REWARD_EPOCH
```

**Dependencies:** Milestone 11

**Test Criteria:**
- [ ] All commands work
- [ ] Clear output formatting
- [ ] Error messages helpful

---

### Milestone 13: Documentation
**~200 lines**

Update all documentation.

**Files Affected:**
- `docs/rewards.md` (NEW)
- `specs/protocol.md`
- `CLAUDE.md`

**Dependencies:** All previous milestones

---

## 5. Data Structures Summary

### Block Structure (Updated)

```rust
pub struct Block {
    pub header: BlockHeader,      // Existing (unchanged)
    pub transactions: Vec<Transaction>,
    pub presence: PresenceCommitment,  // NEW
}

pub struct PresenceCommitment {
    pub bitfield: Vec<u8>,        // 1 bit per producer (~13 bytes for 100 producers)
    pub merkle_root: Hash,        // 32 bytes
    pub weights: Vec<Amount>,     // 8 bytes × present count
    pub total_weight: Amount,     // 8 bytes (cached sum)
}
```

### Storage Schema

```
RocksDB Column Families:

CF_HEADERS         - Block headers (existing)
CF_BODIES          - Block transactions (existing)
CF_HEIGHT_INDEX    - Block by height (existing)
CF_SLOT_INDEX      - Block by slot (existing)
CF_CLAIMED_EPOCHS  - Claim records (NEW)

CF_CLAIMED_EPOCHS:
  Key:   HASH("DOLI_CLAIM_KEY_V1" || producer || epoch)  [32 bytes]
  Value: ClaimRecord { tx_hash, height, amount, timestamp }
```

### Key Constants

```rust
// Easily modifiable - change this to adjust epoch length
pub const BLOCKS_PER_REWARD_EPOCH: u64 = 360;

// Heartbeat
pub const HEARTBEAT_VDF_ITERATIONS: u64 = 10_000_000;
pub const MIN_WITNESS_SIGNATURES: usize = 2;

// Maturity
pub const REWARD_MATURITY: BlockHeight = 100;
```

---

## 6. Validation Rules (Complete)

```
VALIDATE_CLAIM_EPOCH_REWARD(tx, context, blocks, registry, producers):

  1. STRUCTURAL
     ├─ tx.type == ClaimEpochReward (11)
     ├─ tx.inputs.len() == 0
     ├─ tx.outputs.len() == 1
     └─ tx.extra_data.len() >= 136

  2. PARSE
     ├─ claim_data = parse(extra_data[0..72])
     └─ signature = parse(extra_data[72..136])

  3. EPOCH COMPLETE
     ├─ current_epoch = height / BLOCKS_PER_REWARD_EPOCH
     └─ REQUIRE: claim_data.epoch < current_epoch

  4. NOT CLAIMED
     └─ REQUIRE: !registry.is_claimed(producer, epoch)

  5. PRODUCER REGISTERED
     └─ REQUIRE: producers.contains(claim_data.producer)

  6. CALCULATE REWARD
     ├─ (start, end) = epoch_boundaries(claim_data.epoch)
     ├─ For height in start..end:
     │     block = blocks.get(height)
     │     if block.presence.is_present(producer_index):
     │         weight = block.presence.get_weight(producer_index)
     │         total = block.presence.total_weight
     │         reward += block_reward × weight / total
     └─ expected_reward = sum of all block rewards

  7. AMOUNT MATCHES
     └─ REQUIRE: tx.outputs[0].amount == expected_reward

  8. SIGNATURE VALID
     ├─ message = hash(epoch || producer || recipient || amount)
     └─ REQUIRE: verify(message, signature, producer)

  9. OUTPUT CORRECT
     ├─ REQUIRE: tx.outputs[0].pubkey_hash == claim_data.recipient
     └─ REQUIRE: tx.outputs[0].output_type == Normal

  10. SUCCESS
```

---

## 7. Migration Strategy

### Activation Process

1. **Deploy new binary** with presence + claim support
2. **Soft fork**: Old blocks valid, new blocks include presence
3. **Grace period**: 1 epoch for all nodes to upgrade
4. **Activation height**: New reward system active

### Backward Compatibility

- Old `EpochReward` transactions in history remain valid
- New `EpochReward` transactions rejected after activation
- Producers can claim any historical epoch (no expiration)

### Node Operator Actions

1. Stop node
2. Upgrade binary
3. Restart node
4. New CF_CLAIMED_EPOCHS created automatically
5. Start claiming rewards via CLI/RPC

---

## 8. Testing Plan

### Unit Tests

```rust
// Epoch utilities
test_height_to_epoch()
test_epoch_boundaries()
test_is_epoch_complete()

// Presence commitment
test_bitfield_encoding()
test_is_present()
test_get_weight()
test_serialization()

// Heartbeat
test_vdf_computation()
test_vdf_verification()
test_witness_validation()

// Reward calculation
test_single_producer_all_present()
test_two_producers_equal_weight()
test_two_producers_different_weight()
test_producer_partially_present()
test_producer_never_present()
test_empty_epoch()

// Claim validation
test_reject_incomplete_epoch()
test_reject_double_claim()
test_reject_wrong_amount()
test_reject_invalid_signature()
test_accept_valid_claim()

// Registry
test_mark_claimed()
test_is_claimed()
test_revert_claim()
test_persistence()
```

### Integration Tests

```rust
test_full_claim_flow()
test_claim_after_reorg()
test_multiple_producers_claiming()
test_batch_claims()
```

### E2E Tests

```bash
./scripts/test_weighted_presence_rewards.sh
./scripts/test_claim_epoch_reward.sh
./scripts/test_5node_presence_rewards.sh
```

---

## 9. Open Questions (Resolved)

| Question | Decision |
|----------|----------|
| **Epoch basis** | Block height (not slots) - cleaner, no gaps |
| **Checkpoints** | Not needed - epochs provide O(360) boundaries |
| **Reward expiration** | No expiration - producers keep earned rewards |
| **Recipient flexibility** | Yes - producer signs claim to any address |
| **Batch claiming** | One tx per epoch (simple validation) |

---

## 10. Summary

### What This Design Achieves

| Feature | Benefit |
|---------|---------|
| **Weighted presence** | All present producers earn proportionally |
| **Block-based epochs** | Simple `height / BLOCKS_PER_REWARD_EPOCH` |
| **No checkpoints** | Epochs provide O(360) bounded scan |
| **On-demand claims** | Producers control timing |
| **Deterministic** | Same calculation everywhere |
| **Tunable** | Change `BLOCKS_PER_REWARD_EPOCH` constant |

### Storage Overhead

| Component | Per Block | Per Year |
|-----------|-----------|----------|
| Presence bitfield | ~13 bytes | ~41 MB |
| Weights | ~640 bytes | ~2 GB |
| Merkle root | 32 bytes | ~100 MB |
| **Total** | ~700 bytes | ~2.2 GB |

### Complexity Comparison

| Aspect | Old System | New System |
|--------|------------|------------|
| Epoch boundary logic | ~500 lines | 0 lines |
| Claim validation | N/A | ~350 lines |
| Presence tracking | N/A | ~400 lines |
| Checkpoint system | N/A (would be ~500) | 0 lines |
| **Net change** | -500 lines | +750 lines |

Net: ~250 more lines, but much cleaner architecture with no epoch boundary bugs.

---

**End of Implementation Plan v2.0**

---

## Appendix A: Purist Design Summary

# DOLI Blockchain: Proof of Time with Presence and Epoch-based Rewards

## Overview

Implement a presence-based reward system for DOLI's Proof of Time consensus. Producers prove they are online each slot via VDF heartbeats. Rewards accumulate per epoch and are claimed on demand.

## Core Principles

### Purist Genesis (No Special Cases)
- Genesis block creates real coins AND registers founders as producers in one atomic block
- Distribution transaction creates coins, Registration transaction locks them as bond
- After genesis, everything follows normal rules with zero exceptions
- No virtual bonds, no grace periods, no implicit weights

### Proof of Time = Proof of Presence
- Every slot (10 seconds), producers broadcast VDF heartbeat proving ~1 second of work
- Heartbeat requires k=2 witness signatures from other producers
- Only producers with valid witnessed heartbeats are "present"
- Only present producers receive rewards

### Weighted Rewards
- Rewards proportional to bond amount
- 2000 DOLI bond = 2× rewards vs 1000 DOLI bond
- Weight can increase anytime (producer adds more bond)
- Use u128 for calculation to avoid overflow, dust is burned

### Epoch-based Claims
- Epoch = 8,640 blocks (~1 day)
- When epoch closes, calculate rewards once and cache forever
- Producers claim completed epochs whenever they want
- Claim transaction mints coins (no inputs, creates UTXO)
- No checkpoints needed - epochs ARE the natural checkpoints

## Constants

```
DECIMALS = 10 (minimal dust)
UNITS_PER_COIN = 10,000,000,000
EPOCH_LENGTH = 8,640 blocks
BOND_UNIT = 1,000 DOLI minimum
REQUIRED_WITNESSES = 2
HEARTBEAT_VDF_ITERATIONS = ~1 second
```

## Data Structures

### PresenceCommitment (in BlockHeader)
- bitfield: which producers are present (sorted by pubkey)
- weights: bond amounts of present producers
- total_weight: sum of weights
- merkle_root: for fraud proofs
- present_count: number of present

### EpochRewards (cached when epoch closes)
- epoch number
- rewards: map of producer → total reward for epoch
- dust_burned: remainder from integer division

### RewardClaim Transaction
- producer pubkey
- from_epoch (must be last_claimed_epoch + 1)
- to_epoch (must be closed)
- amount (must match calculated sum)

## Flows

### Genesis
1. Create distribution tx for each founder (coins from nothing)
2. Create registration tx for each founder (locks coins as bond)
3. Both in same block, registration references distribution output
4. Result: founders are registered producers with real locked bonds

### Each Slot
1. t=0-1.5s: Producers compute VDF, broadcast heartbeat
2. t=1.5-3s: Producers witness others' heartbeats (sign them)
3. t=3-7s: Block producer collects valid heartbeats (≥2 witnesses)
4. t=7-10s: Build block with presence commitment

### Epoch Close
1. Detect epoch boundary (height % EPOCH_LENGTH == 0)
2. Scan all blocks in completed epoch
3. For each block: distribute block_reward proportional to weights
4. Cache EpochRewards permanently
5. Never recalculate

### Reward Claim
1. Producer submits claim tx (from_epoch, to_epoch)
2. Validate: from_epoch == last_claimed_epoch + 1
3. Validate: to_epoch is closed (current_epoch - 1 or earlier)
4. Calculate: sum epoch_cache[e].rewards[producer] for e in range
5. Validate: tx amount matches calculation
6. Execute: create UTXO, update last_claimed_epoch

## Validation Rules

### Heartbeat Valid If
- Producer in active ProducerSet
- Slot is current or current+1
- prev_block_hash matches chain tip
- VDF proof verifies
- Producer signature valid
- ≥2 witness signatures from other producers

### Block Valid If
- Existing rules plus:
- presence.bitfield size matches producer count
- Block producer is marked present
- present_count matches bitfield popcount
- weights.len() == present_count
- total_weight == sum(weights)
- Each weight matches producer's current bond

### Claim Valid If
- from_epoch == producer.last_claimed_epoch + 1
- to_epoch < current_epoch (epoch is closed)
- amount == sum of cached rewards for range
- recipient == producer's address

## Storage

### In Blockchain
- Block headers with presence commitment
- Claim transactions (the mint record)

### In ProducerSet
- last_claimed_epoch per producer

### Local Cache (regenerable)
- EpochRewards per completed epoch
- PresencePool (memory only, current slot)

## Disk Usage

| Component | Per Year |
|-----------|----------|
| Blocks + presence + weights | ~5 GB |
| Epoch cache | ~2-60 MB |
| Total | ~5 GB (10× smaller than Bitcoin) |

## Anti-DoS

- Heartbeats only from registered producers
- 1 heartbeat per (producer, slot, prev_hash)
- Verify signature before VDF (cheaper first)
- PresencePool keeps only current and next slot

## Implementation Order

1. Genesis with distribution + registration
2. PresenceCommitment structure
3. Heartbeat with witness signatures
4. PresencePool (memory)
5. Block production with presence
6. EpochRewards cache
7. RewardClaim transaction
8. Validation rules
9. Remove old epoch reward code

## Design Summary

| Aspect | Value |
|--------|-------|
| Lines of prompt | ~150 |
| Code included | None |
| Special cases | Zero |
| Approach | Purist |

---

**End of Appendix A**
