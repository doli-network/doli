# rewards.md - Weighted Presence Rewards System

This document describes DOLI's weighted presence reward system, where all present producers earn proportional to their bond weight.

---

## 1. Overview

DOLI uses a **weighted presence** model for block rewards. Instead of rewarding only block producers, all producers who prove presence during a slot receive a proportional share of the block reward.

### Core Formula

```
For each block where a producer was present:
  producer_reward += block_reward × producer_weight / total_present_weight
```

### Key Benefits

| Benefit | Description |
|---------|-------------|
| **All Present Earn** | Not just block producer - everyone proving presence |
| **Capital Efficient** | 2× bond = 2× reward (proportional) |
| **On-Demand Claims** | Producers claim when ready, no automatic distribution |
| **Deterministic** | Same calculation on every node |

---

## 2. Block-Based Epochs

Rewards are organized into **epochs** based on block height, not slots.

### Why Block Height?

| Aspect | Slot-Based | Block-Based |
|--------|------------|-------------|
| **Gaps** | Empty slots create uneven epochs | No gaps - heights are sequential |
| **Predictability** | Variable blocks per epoch | Exactly N blocks per epoch |
| **Calculation** | Must handle missing slots | Simple division |

### Epoch Constants

```
BLOCKS_PER_REWARD_EPOCH = 360

Epoch 0: blocks 0-359     (360 blocks)
Epoch 1: blocks 360-719   (360 blocks)
Epoch 2: blocks 720-1079  (360 blocks)
...
Epoch N: blocks N×360 to (N+1)×360-1
```

### Network-Specific Values

| Network | Blocks per Epoch | Approximate Duration |
|---------|------------------|---------------------|
| Mainnet | 360 | ~1 hour (at 10s blocks) |
| Testnet | 360 | ~1 hour (at 10s blocks) |
| Devnet | 60 | ~1 minute (at 1s blocks) |

---

## 3. Presence Tracking

### PresenceCommitment Structure

Each block contains a `PresenceCommitment` recording which producers were present:

```
PresenceCommitment:
  bitfield:      Vec<u8>    1 bit per registered producer
  merkle_root:   Hash       Merkle root of heartbeat data
  weights:       Vec<u64>   Bond weights of present producers
  total_weight:  u64        Sum of weights (cached)
```

### How Presence is Proven

1. **Heartbeat VDF**: Each slot (~10s), producers compute a VDF proof (~700ms)
2. **Witness Signatures**: Heartbeat requires 2+ witness signatures from other producers
3. **Block Inclusion**: Block producer records all valid heartbeats in `PresenceCommitment`

### Heartbeat Structure

```
Heartbeat:
  producer:       PublicKey
  slot:           u32
  prev_block_hash: Hash
  vdf_output:     [u8; 32]
  signature:      Signature
  witnesses:      Vec<WitnessSignature>
```

---

## 4. Reward Calculation

### Per-Block Reward Distribution

For each block, rewards are distributed proportionally:

```
For producer P with weight W in block B:
  share = block_reward × W / total_present_weight
```

### Example

Block 1000 with block_reward = 100,000,000 (1 DOLI):

| Producer | Weight | Present | Share |
|----------|--------|---------|-------|
| Alice | 1,000 | Yes | 20,000,000 (0.2 DOLI) |
| Bob | 2,000 | Yes | 40,000,000 (0.4 DOLI) |
| Carol | 2,000 | Yes | 40,000,000 (0.4 DOLI) |
| Dave | 1,500 | No | 0 |

Total present weight: 5,000
- Alice: 100M × 1000/5000 = 20M
- Bob: 100M × 2000/5000 = 40M
- Carol: 100M × 2000/5000 = 40M

### Epoch Accumulation

Rewards accumulate over an entire epoch. A producer's total reward for epoch N is the sum of their share from every block where they were present.

---

## 5. Claiming Rewards

### ClaimEpochReward Transaction

Producers claim rewards by submitting a `ClaimEpochReward` transaction:

```
ClaimEpochReward (TxType = 11):
  version:    1
  type:       11
  inputs:     []              # Minted, no inputs
  outputs:    [{
    output_type: 0,           # Normal
    amount:      calculated_reward,
    pubkey_hash: recipient,
    lock_until:  0
  }]
  extra_data: {
    epoch:          u64       # Epoch being claimed
    producer_pubkey: 32 bytes # Claiming producer
    recipient_hash:  32 bytes # Where to send reward
    signature:       64 bytes # Producer signature
  }
```

### Claim Validation

A claim is valid if:

1. **Epoch complete**: Current height ≥ epoch end height
2. **Not already claimed**: ClaimRegistry shows unclaimed
3. **Producer registered**: Producer exists in active set
4. **Producer was present**: At least 1 block in epoch
5. **Amount correct**: Matches calculated weighted reward
6. **Signature valid**: Producer signed the claim
7. **Recipient correct**: Output matches claim data

### Claim Registry

The node maintains a `ClaimRegistry` tracking which (producer, epoch) pairs have been claimed:

```
ClaimRecord:
  tx_hash:   Hash        # Transaction that claimed
  height:    u64         # Block height of claim
  amount:    u64         # Amount claimed
  timestamp: u64         # Claim timestamp
```

---

## 6. CLI Commands

### List Claimable Rewards

```bash
doli rewards list
```

Shows all unclaimed epochs with estimated rewards:

```
Claimable Rewards
------------------------------------------------------------
Epoch    Blocks Present    Estimated Reward
------------------------------------------------------------
5        358/360           47.50000000 DOLI
6        360/360           48.00000000 DOLI
7        355/360           46.25000000 DOLI
------------------------------------------------------------
Total:   3 epochs          141.75000000 DOLI
```

### Claim Specific Epoch

```bash
doli rewards claim <EPOCH> [--recipient <ADDRESS>]
```

Example:
```bash
doli rewards claim 5
doli rewards claim 5 --recipient a1b2c3d4...
```

### Claim All Epochs

```bash
doli rewards claim-all [--recipient <ADDRESS>]
```

Claims all available epochs (one transaction per epoch).

### View Claim History

```bash
doli rewards history [--limit <N>]
```

Shows previous claims:

```
Claim History
------------------------------------------------------------
Epoch    Amount              Tx Hash              Height
------------------------------------------------------------
4        46.00000000 DOLI    a1b2c3d4...          1440
3        45.50000000 DOLI    e5f6a7b8...          1080
2        44.75000000 DOLI    c9d0e1f2...          720
------------------------------------------------------------
```

### Show Epoch Info

```bash
doli rewards info
```

Displays current epoch status:

```
Epoch Information
------------------------------------------------------------
Current Epoch:          8
Current Height:         2950
Epoch Progress:         190/360 (52.8%)
Blocks per Epoch:       360
Next Epoch Starts At:   2880

Last Complete Epoch:    7
------------------------------------------------------------
```

---

## 7. RPC Endpoints

### getClaimableRewards

Returns unclaimed epochs for a producer.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| producer_pubkey | string | Producer public key (hex) |

**Response:**
```json
{
  "epochs": [
    {
      "epoch": 5,
      "blocks_present": 358,
      "total_blocks": 360,
      "estimated_reward": 4750000000,
      "is_claimed": false
    }
  ],
  "total_claimable": 14175000000
}
```

### getClaimHistory

Returns claim history for a producer.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| producer_pubkey | string | Producer public key (hex) |
| limit | number | Maximum entries (default: 10) |

**Response:**
```json
{
  "claims": [
    {
      "epoch": 4,
      "amount": 4600000000,
      "tx_hash": "0xa1b2c3d4...",
      "height": 1440,
      "timestamp": 1706500000
    }
  ],
  "total_claimed": 13625000000
}
```

### estimateEpochReward

Estimates reward for a specific epoch.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| producer_pubkey | string | Producer public key (hex) |
| epoch | number | Epoch number |

**Response:**
```json
{
  "epoch": 5,
  "blocks_present": 358,
  "total_blocks": 360,
  "total_producer_weight": 358000,
  "total_all_weights": 1790000,
  "block_reward": 100000000,
  "estimated_reward": 4750000000
}
```

### buildClaimTx

Builds an unsigned claim transaction.

**Parameters:**
| Name | Type | Description |
|------|------|-------------|
| producer_pubkey | string | Producer public key (hex) |
| epoch | number | Epoch to claim |
| recipient | string | Optional recipient address |

**Response:**
```json
{
  "unsigned_tx": "0x...",
  "signing_message": "0x...",
  "epoch": 5,
  "amount": 4750000000,
  "recipient": "0x..."
}
```

### getEpochInfo

Returns current reward epoch information.

**Parameters:** None

**Response:**
```json
{
  "current_epoch": 8,
  "current_height": 2950,
  "blocks_per_epoch": 360,
  "epoch_start_height": 2880,
  "epoch_end_height": 3240,
  "epoch_progress": 70,
  "last_complete_epoch": 7
}
```

---

## 8. Migration from Old System

### What Changed

| Old System | New System |
|------------|------------|
| Automatic distribution at epoch boundaries | On-demand claims |
| Only block producers rewarded | All present producers rewarded |
| Slot-based epochs | Block-based epochs |
| Complex epoch boundary logic | Simple height division |

### Backward Compatibility

- Old `EpochReward` (type 10) transactions in history remain valid
- New `EpochReward` transactions rejected after activation
- Producers can claim any historical epoch (no expiration)

### Node Operator Actions

1. Upgrade binary
2. Restart node
3. ClaimRegistry created automatically
4. Use CLI to claim accumulated rewards

---

## 9. Storage Overhead

| Component | Per Block | Per Year |
|-----------|-----------|----------|
| Presence bitfield | ~13 bytes | ~41 MB |
| Weights | ~640 bytes | ~2 GB |
| Merkle root | 32 bytes | ~100 MB |
| **Total** | ~700 bytes | ~2.2 GB |

---

## See Also

- [protocol.md](../specs/protocol.md) - Protocol specification (Section 3.15)
- [cli.md](./cli.md) - CLI reference
- [rpc_reference.md](./rpc_reference.md) - RPC API documentation
- [becoming_a_producer.md](./becoming_a_producer.md) - Producer guide

---

*Last updated: January 2026*
