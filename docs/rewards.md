# rewards.md - Block Reward System

This document describes DOLI's block reward system: a pooled coinbase model where all block rewards accumulate in a shared reward pool and are distributed bond-weighted to qualified producers at each epoch boundary.

---

## 1. Overview

DOLI uses a **pooled coinbase with epoch distribution** model. Every block's coinbase output is sent to a deterministic reward pool address (no private key). At the end of each epoch (every 360 blocks, approximately 1 hour), the pool is drained and distributed bond-weighted among attestation-qualified producers via an EpochReward transaction.

### Key Characteristics

| Characteristic | Description |
|----------------|-------------|
| **Pooled Coinbase** | All coinbase goes to `BLAKE3("REWARD_POOL" \|\| "doli")` -- no private key |
| **Epoch Distribution** | Pool drained every 360 blocks (~1 hour) via EpochReward tx (TxType=10) |
| **Attestation Gated** | Producers must attest in 54 of 60 minutes per epoch to qualify |
| **Bond Weighted** | Rewards proportional to qualifying bonds (own + delegated) |
| **Delegation Split** | Delegated bond rewards split: producer keeps fee, delegators get remainder |
| **Deflationary** | Halving every ~4 years reduces emission over time |

### Why Pooled Coinbase?

| Benefit | Description |
|---------|-------------|
| **Attestation Incentive** | Producers must stay online and attest to earn rewards |
| **Fair Distribution** | Bond-weighted sharing prevents winner-take-all dynamics |
| **Delegation Support** | Stakers can delegate bonds and earn proportional rewards |
| **Sybil Resistance** | Non-attesting producers are excluded from rewards |

---

## 2. Reward Flow

### Step 1: Coinbase to Pool

Every block contains a coinbase transaction (implemented as `TxType::Transfer` with no inputs) whose output is sent to the reward pool address, not the block producer. Note: the `TxType::Coinbase` (6) enum variant exists but is dead code -- coinbase uses `TxType::Transfer` (0).

```
Coinbase (implemented as TxType::Transfer with no inputs):
  version:    1
  type:       0 (Transfer)
  inputs:     []              # No inputs (newly minted)
  outputs:    [{
    output_type: 0,           # Normal
    amount:      block_reward,
    pubkey_hash: reward_pool_pubkey_hash(),   # BLAKE3("REWARD_POOL" || "doli")
    lock_until:  0
  }]
  extra_data: []
```

The reward pool address is deterministic and has no corresponding private key. Only the consensus engine can distribute its funds.

### Step 2: Attestation Tracking

During each epoch, every block's `presence_root` field encodes an attestation bitfield. The node decodes these bitfields to track how many of the 60 attestation minutes each producer was attested in.

- Each epoch spans 360 slots = 60 attestation minutes (6 slots per minute)
- A producer is counted as attested in a minute if any block in that minute's 6-slot window includes them in the bitfield

### Step 3: Epoch Boundary Distribution

At height `(epoch + 1) * 360`, the node runs `calculate_epoch_rewards()`:

1. **Collect active producers** at epoch end height, sorted by public key
2. **Scan all blocks** in the epoch, decode attestation bitfields, count attested minutes per producer
3. **Qualify producers** with never-burn fallback tiers:
   - **Tier 1**: must have attested in >= 90% of attestation minutes (`attestation_qualification_threshold()` — 54/60 for mainnet, 5/6 for testnet)
   - **Tier 2 fallback**: if no Tier 1 qualifiers, threshold drops to 80% of median attendance (floor of 1 minute)
   - **Tier 3 fallback**: if all producers have 0 attendance, pool accumulates to next epoch (no distribution)
   - **Epoch 0 exception**: all active producers qualify (no attestation data exists yet)
4. **Sum qualifying bonds** via `selection_weight()` (own bonds + delegated bonds)
5. **Calculate pool total**: accumulated coinbase UTXOs in pool + current block's coinbase
6. **Distribute bond-weighted**: `pool * bonds[i] / qualifying_bonds` using u128 intermediates
7. **Delegation split** (if producer has delegations):
   - `own_share = reward * own_bonds / total_bonds`
   - `delegate_fee = delegated_share * DELEGATE_REWARD_PCT / 100` (producer keeps)
   - `staker_pool = delegated_share - delegate_fee` (split among delegators proportional to bonds)
8. **Integer division remainder** goes to the first qualifier
9. **Create EpochReward transaction** (TxType=10) with all reward outputs
10. **Consume all pool UTXOs** atomically

### Step 4: EpochReward Transaction

The EpochReward transaction (TxType=10) is the active mechanism for distributing rewards:

```
EpochReward (TxType = 10):
  version:    1
  type:       10
  inputs:     []              # Pool UTXOs consumed by consensus engine
  outputs:    [{              # One output per qualifying producer (+ delegators)
    output_type: 0,           # Normal
    amount:      calculated_share,
    pubkey_hash: producer_or_delegator_hash,
    lock_until:  0
  }, ...]
  extra_data: []
```

---

## 3. Coinbase Maturity

Coinbase outputs use `lock_until = 0` (no maturity lock in the output itself). However, the `COINBASE_MATURITY` constant (6 blocks) is used during validation to prevent spending coinbase UTXOs until they have sufficient confirmations.

| Parameter | Value | Notes |
|-----------|-------|-------|
| **COINBASE_MATURITY** | 6 blocks | ~60 seconds at 10s slots |
| **Lock Mechanism** | Validation check | `is_spendable_at(height)` enforces maturity |
| **Spending** | After 6 confirmations | Enforced by validation, not by `lock_until` field |

Note: Both coinbase pool UTXOs and EpochReward outputs have `lock_until = 0`. Maturity is enforced by validation logic, not by the output lock field.

---

## 4. Emission Schedule

### Initial Parameters

| Parameter | Value |
|-----------|-------|
| **Total Supply Cap** | 25,228,800 DOLI |
| **Initial Block Reward** | 1 DOLI (100,000,000 base units) |
| **Halving Interval** | 12,614,400 blocks (~4 years) |

### Halving Schedule

The block reward halves every 12,614,400 blocks (approximately 4 years):

| Era | Block Range | Reward per Block | Era Total |
|-----|-------------|------------------|-----------|
| 0 | 0 - 12,614,399 | 1.00000000 DOLI | 12,614,400 DOLI |
| 1 | 12,614,400 - 25,228,799 | 0.50000000 DOLI | 6,307,200 DOLI |
| 2 | 25,228,800 - 37,843,199 | 0.25000000 DOLI | 3,153,600 DOLI |
| 3 | 37,843,200 - 50,457,599 | 0.12500000 DOLI | 1,576,800 DOLI |
| ... | ... | ... | ... |

### Reward Calculation

```
era = height / BLOCKS_PER_ERA
block_reward = INITIAL_REWARD / (2 ^ era)

where:
  BLOCKS_PER_ERA = 12,614,400
  INITIAL_REWARD = 100,000,000 (1 DOLI in base units)
```

### Asymptotic Supply

Due to integer division, rewards eventually reach zero. The total supply asymptotically approaches but never exceeds 25,228,800 DOLI (2,522,880,000,000,000 base units).

---

## 5. Epoch Reward Pool Math

Each epoch accumulates up to 360 DOLI in the reward pool (360 blocks at 1 DOLI per block in Era 0).

### Qualification

| Parameter | Value |
|-----------|-------|
| **SLOTS_PER_EPOCH** | 360 (1 hour) |
| **Attestation minutes per epoch** | 60 (6 slots per minute) |
| **ATTESTATION_QUALIFICATION_THRESHOLD** | 54 minutes (90%) |
| **Epoch 0 exception** | All active producers qualify |

### Distribution Formula

```
pool = sum(coinbase UTXOs at reward_pool_address) + current_block_coinbase
qualifying_bonds = sum(selection_weight for each qualified producer)
producer_reward = pool * producer_bonds / qualifying_bonds   (u128 intermediate)
remainder = pool - sum(all_producer_rewards)  -> first qualifier
```

### Delegation Split

When a producer has received delegations:

```
own_share      = reward * own_bonds / total_bonds
delegated_share = reward - own_share
delegate_fee   = delegated_share * DELEGATE_REWARD_PCT / 100
staker_pool    = delegated_share - delegate_fee

Each delegator: staker_pool * delegator_bonds / total_delegated
Last delegator: staker_pool - already_distributed  (absorbs rounding dust)
```

| Parameter | Value |
|-----------|-------|
| **DELEGATE_REWARD_PCT** | 10% |
| **BOND_UNIT** | 10 DOLI (1,000,000,000 base units) on mainnet/testnet |
| **MAX_BONDS_PER_PRODUCER** | 3,000 |

### Edge Cases

- **No qualified producers**: all rewards remain in pool (burned effectively)
- **Zero qualifying bonds**: no distribution
- **Disqualified producers**: their share is redistributed to qualifiers (included in the pool, not subtracted)

---

## 6. Network-Specific Parameters

| Parameter | Mainnet | Testnet | Devnet |
|-----------|---------|---------|--------|
| **Initial Reward** | 1 DOLI | 1 DOLI | 20 DOLI |
| **Halving Interval** | 12,614,400 blocks (~4 yr) | 12,614,400 blocks | 576 blocks (~96 min) |
| **Bond Unit** | 10 DOLI | 1 DOLI | 1 DOLI |
| **Coinbase Maturity** | 6 blocks | 6 blocks | 10 blocks |
| **Blocks per Reward Epoch** | 360 blocks (~1 hr) | 36 blocks (~6 min) | 4 blocks (~40s) |
| **Slot Duration** | 10 seconds | 10 seconds | 10 seconds |

Devnet uses accelerated parameters for testing the reward and halving mechanisms.

---

## 7. Deprecated Transaction Types

### ClaimReward (Type 3) - DEPRECATED

```
Status: DEPRECATED
Reason: Replaced by automatic EpochReward distribution at epoch boundaries
Action: Nodes will reject new ClaimReward transactions
```

Previously proposed for manual reward claiming. Never activated on mainnet. The EpochReward mechanism (TxType=10) replaced it entirely.

---

## 8. Validation Rules

### Coinbase Validation

A coinbase transaction is valid if:

1. **Type**: Transaction type is 0 (Transfer) with no inputs and one output (`is_coinbase()` check)
2. **Position**: First transaction in the block
3. **Inputs**: Empty (no inputs)
4. **Outputs**: Exactly one output
5. **Amount**: Equals calculated block reward for the height (plus extra per-byte fees)
6. **Recipient**: Output `pubkey_hash` matches `reward_pool_pubkey_hash()`
7. **Lock**: `lock_until` is 0 (maturity enforced by validation logic, not output field)

### EpochReward Validation

An EpochReward transaction is valid if:

1. **Type**: Transaction type is 10 (EpochReward)
2. **Position**: Appears only at epoch boundary blocks (height divisible by `blocks_per_reward_epoch`)
3. **Inputs**: Empty (pool UTXOs consumed by consensus engine, not via inputs)
4. **Outputs**: One or more outputs to qualified producers and their delegators
5. **Total**: Sum of outputs equals the entire pool balance (no value created or destroyed)

### Block Validation

Each block must contain exactly one coinbase transaction. At epoch boundaries, the block also contains exactly one EpochReward transaction.

---

## 9. Economic Incentives

### Producer Motivation

Producers are incentivized to:
- **Stay Online and Attest**: Must attest in 54/60 minutes to earn epoch rewards
- **Build Valid Blocks**: Invalid blocks are rejected
- **Bond More**: Rewards scale linearly with bond count (own + delegated)
- **Attract Delegators**: More delegated bonds means more total rewards (producer keeps delegate fee)

### Bond ROI

With the bond stacking system (up to 3,000 bonds per producer):

| Bonds | Investment | Selection Weight | Reward Share |
|-------|------------|-----------------|--------------|
| 1 | 10 DOLI | 1 | Proportional to 1/total_qualifying |
| 10 | 100 DOLI | 10 | 10x base share |
| 100 | 1,000 DOLI | 100 | 100x base share |
| 3,000 | 30,000 DOLI | 3,000 | Maximum per-producer share |

Actual rewards depend on total qualifying bonds across the network and attestation qualification.

---

## 10. Key Constants Reference

| Constant | Value | Location |
|----------|-------|----------|
| `SLOTS_PER_EPOCH` | 360 | `crates/core/src/consensus.rs` |
| `BLOCKS_PER_REWARD_EPOCH` | 360 | `crates/core/src/consensus.rs` |
| `ATTESTATION_QUALIFICATION_THRESHOLD` | 54 | `crates/core/src/attestation.rs` |
| `ATTESTATION_MINUTES_PER_EPOCH` | 60 | `crates/core/src/attestation.rs` |
| `DELEGATE_REWARD_PCT` | 10 | `crates/core/src/consensus.rs` |
| `BOND_UNIT` | 1,000,000,000 (10 DOLI) | `crates/core/src/consensus.rs` |
| `MAX_BONDS_PER_PRODUCER` | 3,000 | `crates/core/src/consensus.rs` |
| `COINBASE_MATURITY` | 6 | `crates/core/src/consensus.rs` |
| `INITIAL_REWARD` | 100,000,000 (1 DOLI) | `crates/core/src/consensus.rs` |
| `BLOCKS_PER_ERA` | 12,614,400 | `crates/core/src/consensus.rs` |
| `TOTAL_SUPPLY` | 2,522,880,000,000,000 | `crates/core/src/consensus.rs` |

---

## See Also

- [protocol.md](../specs/protocol.md) - Protocol specification
- [becoming_a_producer.md](./becoming_a_producer.md) - Producer guide
- [cli.md](./cli.md) - CLI reference
- [rpc_reference.md](./rpc_reference.md) - RPC API documentation

---

*Last updated: March 2026 (synced against code 2026-03-29)*
