# rewards.md - Block Reward System

This document describes DOLI's block reward system, which follows a simple Bitcoin-like coinbase model where block producers receive 100% of the block reward directly.

---

## 1. Overview

DOLI uses a **direct coinbase** model for block rewards. The block producer receives the full block reward via an automatic coinbase transaction included in each block.

### Key Characteristics

| Characteristic | Description |
|----------------|-------------|
| **100% to Producer** | Block producer receives the entire block reward |
| **Automatic** | No claiming required - rewards included via coinbase tx |
| **Maturity Period** | 100 confirmations before spendable |
| **Deflationary** | Halving every ~4 years reduces emission over time |

### Why Direct Coinbase?

| Benefit | Description |
|---------|-------------|
| **Simplicity** | No complex claiming transactions or epoch tracking |
| **Predictability** | Producers know exactly what they earn per block |
| **Proven Model** | Same approach used by Bitcoin for 15+ years |
| **No State Bloat** | No claim registries or presence tracking needed |

---

## 2. Block Reward Structure

### Coinbase Transaction

Every block contains exactly one coinbase transaction as the first transaction:

```
Coinbase (TxType = 6):
  version:    1
  type:       6
  inputs:     []              # No inputs (newly minted)
  outputs:    [{
    output_type: 0,           # Normal
    amount:      block_reward,
    pubkey_hash: producer_pubkey_hash,
    lock_until:  current_height + COINBASE_MATURITY
  }]
  extra_data: []
```

### Coinbase Maturity

Coinbase outputs are **locked** until 100 confirmations have passed:

| Parameter | Value | Notes |
|-----------|-------|-------|
| **COINBASE_MATURITY** | 100 blocks | ~17 minutes at 10s slots |
| **Lock Mechanism** | `lock_until` field | Set to `current_height + 100` |
| **Spending** | After maturity | Normal UTXO spending rules |

This maturity period prevents issues from chain reorganizations affecting recently minted coins.

---

## 3. Emission Schedule

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

Due to integer division, rewards eventually reach zero. The total supply asymptotically approaches but never exceeds 25,228,800 DOLI.

---

## 4. Network-Specific Parameters

| Parameter | Mainnet/Testnet | Devnet |
|-----------|-----------------|--------|
| **Initial Reward** | 1 DOLI | 1 DOLI |
| **Halving Interval** | 12,614,400 blocks | 576 blocks (~96 min) |
| **Coinbase Maturity** | 100 blocks | 100 blocks |
| **Slot Duration** | 10 seconds | 10 seconds |

Devnet uses accelerated parameters for testing the halving mechanism.

---

## 5. Deprecated Transaction Types

The following transaction types are **DEPRECATED** and should not be used:

### ClaimReward (Type 3) - DEPRECATED

```
Status: DEPRECATED
Reason: Rewards are now automatic via coinbase
Action: Nodes will reject new ClaimReward transactions
```

Previously used in a proposed system where producers would manually claim accumulated rewards. This was never activated on mainnet.

### EpochReward (Type 10) - DEPRECATED

```
Status: DEPRECATED
Reason: Weighted presence system was replaced by direct coinbase
Action: Nodes will reject new EpochReward transactions
```

Previously used in a proposed weighted presence system where all present producers would share rewards proportionally. This was replaced by the simpler Bitcoin-like model before mainnet launch.

### Historical Transactions

Any ClaimReward or EpochReward transactions in historical blocks (if any exist from testnet) remain valid in the chain history but the transaction types are no longer accepted for new transactions.

---

## 6. Validation Rules

### Coinbase Validation

A coinbase transaction is valid if:

1. **Type**: Transaction type is 6 (Coinbase)
2. **Position**: First transaction in the block
3. **Inputs**: Empty (no inputs)
4. **Outputs**: Exactly one output
5. **Amount**: Equals calculated block reward for the height
6. **Recipient**: Output pubkey_hash matches block producer
7. **Lock**: `lock_until` equals `height + COINBASE_MATURITY`

### Block Validation

Each block must contain exactly one coinbase transaction:

1. **Existence**: Block must have at least one transaction
2. **First Position**: First transaction must be coinbase type
3. **Uniqueness**: No other coinbase transactions in the block
4. **Correctness**: Coinbase passes all validation rules above

---

## 7. Economic Incentives

### Producer Motivation

Producers are incentivized to:
- **Stay Online**: Missing slots means missing rewards
- **Build Valid Blocks**: Invalid blocks are rejected (no reward)
- **Include Transactions**: Transaction fees add to coinbase reward (future)

### Bond ROI

With the bond stacking system (up to 100 bonds per producer):

| Bonds | Investment | Tickets per Cycle | Expected Blocks/Day* |
|-------|------------|-------------------|---------------------|
| 1 | 100 DOLI | 1 | Proportional |
| 10 | 1,000 DOLI | 10 | 10x base |
| 100 | 10,000 DOLI | 100 | 100x base |

*Actual blocks depend on total network bond count and presence.

---

## 8. Comparison with Other Systems

| Aspect | DOLI | Bitcoin | PoS Chains |
|--------|------|---------|------------|
| **Reward Model** | Direct coinbase | Direct coinbase | Often complex staking rewards |
| **Claiming** | Automatic | Automatic | Often requires claiming |
| **Distribution** | 100% to producer | 100% to miner | Often split with delegators |
| **Maturity** | 100 blocks | 100 blocks | Varies |
| **Emission** | Halving | Halving | Often inflationary |

---

## 9. CLI Commands

### Check Current Reward

```bash
doli-cli info reward
```

Shows current block reward based on chain height:

```
Block Reward Information
------------------------------------------------------------
Current Height:         1,234,567
Current Era:            0
Block Reward:           1.00000000 DOLI
Next Halving:           12,614,400 (11,379,833 blocks remaining)
------------------------------------------------------------
```

### View Producer Earnings

```bash
doli-cli producer earnings [--address <ADDRESS>]
```

Shows coinbase outputs earned by a producer:

```
Producer Earnings
------------------------------------------------------------
Total Blocks Produced:  1,234
Total Earned:           1,234.00000000 DOLI
Mature Balance:         1,134.00000000 DOLI
Pending Maturity:       100.00000000 DOLI (100 blocks)
------------------------------------------------------------
```

---

## 10. RPC Endpoints

### getBlockReward

Returns the block reward for a given height.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| height | number | Block height (optional, defaults to current) |

**Response:**
```json
{
  "height": 1234567,
  "era": 0,
  "reward": 100000000,
  "reward_formatted": "1.00000000 DOLI"
}
```

### getEmissionInfo

Returns emission schedule information.

**Parameters:** None

**Response:**
```json
{
  "total_supply_cap": 2522880000000000,
  "current_supply": 123456700000000,
  "current_height": 1234567,
  "current_era": 0,
  "current_reward": 100000000,
  "blocks_per_era": 12614400,
  "next_halving_height": 12614400,
  "blocks_until_halving": 11379833
}
```

---

## See Also

- [protocol.md](../specs/protocol.md) - Protocol specification
- [becoming_a_producer.md](./becoming_a_producer.md) - Producer guide
- [cli.md](./cli.md) - CLI reference
- [rpc_reference.md](./rpc_reference.md) - RPC API documentation

---

*Last updated: February 2026*
