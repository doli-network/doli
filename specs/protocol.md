# DOLI Protocol Specification

This document provides the technical specification for implementing a DOLI-compatible node.

## Table of Contents

1. [Encoding Rules](#1-encoding-rules)
2. [Cryptographic Primitives](#2-cryptographic-primitives)
3. [Transactions](#3-transactions)
4. [Blocks](#4-blocks)
5. [Consensus Rules](#5-consensus-rules)
6. [Producer Registration](#6-producer-registration)
7. [Network Protocol](#7-network-protocol)
8. [Networks](#8-networks)
9. [Test Vectors](#9-test-vectors)

---

## 1. Encoding Rules

### 1.1 Integers

All integers are encoded in **little-endian** format.

| Type   | Size    | Usage                           |
|--------|---------|----------------------------------|
| uint32 | 4 bytes | slot, epoch, index, version, type |
| uint64 | 8 bytes | amount, timestamp                |

**Example**: Slot 1000 = `0xE8030000`

### 1.2 Byte Strings

Byte strings are concatenated directly without length prefixes or separators in hash preimages.

### 1.3 Literals

ASCII literals are encoded without NUL terminator:

| Literal | Bytes                      |
|---------|----------------------------|
| "BLK"   | `0x42 0x4C 0x4B` (3 bytes) |
| "REG"   | `0x52 0x45 0x47` (3 bytes) |
| "SEED"  | `0x53 0x45 0x45 0x44` (4 bytes) |

### 1.4 Addresses

```
address = HASH(public_key)[0:20]   // First 20 bytes of hash
```

### 1.5 Serialization

Transactions and blocks are serialized using a simple TLV-like format:

```
field := type (1 byte) || length (varint) || data (length bytes)
```

Varint encoding (unsigned):
- 0x00-0xFC: 1 byte (value as-is)
- 0xFD: 3 bytes (0xFD || uint16)
- 0xFE: 5 bytes (0xFE || uint32)
- 0xFF: 9 bytes (0xFF || uint64)

---

## 2. Cryptographic Primitives

### 2.1 Hash Function

```
HASH(x) = BLAKE3-256(x)
```

Output: 32 bytes

### 2.2 Signatures

Algorithm: **Ed25519**

| Component   | Size     |
|-------------|----------|
| Private key | 32 bytes |
| Public key  | 32 bytes |
| Signature   | 64 bytes |

Signing message for transactions:
```
message = HASH(tx_without_signatures)
```

### 2.3 Verifiable Delay Function (Hash-Chain VDF)

Construction: **Iterated SHA-256 hash chain**

DOLI uses a hash-chain VDF with dynamic calibration to maintain consistent timing across all networks:

| Parameter       | All Networks |
|-----------------|--------------|
| Target time     | ~700ms       |
| Iterations      | ~10,000,000 (calibrated) |
| Output          | 32 bytes     |
| Verification    | Recompute    |

```
VDF_compute(input, iterations) -> output
VDF_verify(input, output, iterations) -> bool  // Recomputes the chain
```

**Dynamic Calibration:**
- Iterations adjusted ±20% per cycle to maintain ~700ms target timing
- Min: 100,000 | Max: 100,000,000 iterations
- Calibration runs every 60 seconds

**Note**: All networks use the same ~700ms VDF heartbeat. Grinding prevention comes from Epoch Lookahead (deterministic leader selection), not VDF timing.

---

## 3. Transactions

### 3.1 Transaction Structure

```
transaction = {
    version:    uint32,          // Currently 1
    type:       uint32,          // 0 = transfer, 1 = registration, 2 = exit,
                                 // 3 = claim_reward, 4 = claim_bond, 5 = slash_producer,
                                 // 6 = coinbase, 7 = add_bond, 8 = request_withdrawal,
                                 // 9 = claim_withdrawal, 10 = epoch_reward
    inputs:     input[],
    outputs:    output[],
    extra_data: bytes            // Type-specific data
}
```

### 3.2 Input Structure

```
input = {
    prev_tx_hash:  32 bytes,     // Hash of previous transaction
    output_index:  uint32,       // Index of output being spent
    signature:     64 bytes      // Ed25519 signature
}
```

### 3.3 Output Structure

```
output = {
    output_type:   uint8,        // 0 = normal, 1 = bond
    amount:        uint64,       // Amount in base units
    pubkey_hash:   32 bytes,     // HASH(public_key)
    lock_until:    uint64        // 0 for normal, height for bonds
}
```

### 3.4 Transaction Hash

```
tx_hash = HASH(version || type || inputs_without_sigs || outputs || extra_data)
```

The signature field is replaced with zeros for hashing.

### 3.5 Transaction Validation

A transaction is valid if:

1. **Format**: All fields are properly encoded
2. **Inputs exist**: Each input references an unspent output
3. **Signatures valid**: Each signature matches the referenced output's pubkey
4. **Amounts balance**: `sum(inputs) >= sum(outputs)`
5. **Positive amounts**: All output amounts > 0
6. **No double-spend**: No output is spent twice within the same tx
7. **Sufficient fee**: `sum(inputs) - sum(outputs) >= min_fee`

Minimum fee:
```
min_fee = tx_size_bytes * BASE_FEE_RATE
```

### 3.6 Coinbase Transaction

In DirectCoinbase mode, the first transaction in each block rewards the producer directly.
In EpochPool mode, coinbase is omitted and rewards are distributed via EpochReward
transactions at epoch boundaries (see section 3.11).

```
coinbase_tx = {
    version: 1,
    type: 6,                     // TxType::Coinbase
    inputs: [],                  // Empty
    outputs: [{
        output_type: 0,
        amount: block_reward + total_fees,
        pubkey_hash: producer_pubkey_hash,
        lock_until: 0
    }],
    extra_data: block_height as uint64
}
```

Coinbase outputs require 100 confirmations before spending (REWARD_MATURITY).

### 3.7 Exit Transaction

```
exit_tx = {
    version: 1,
    type: 2,
    inputs: [...],               // To pay fee
    outputs: [...],              // Change
    extra_data: {
        public_key: 32 bytes     // Producer public key
    }
}
```

Initiates the unbonding period. Producer is removed from active set.

### 3.8 Claim Reward Transaction

```
claim_reward_tx = {
    version: 1,
    type: 3,
    inputs: [],
    outputs: [{
        output_type: 0,
        amount: accumulated_rewards,
        pubkey_hash: producer_pubkey_hash,
        lock_until: 0
    }],
    extra_data: {
        public_key: 32 bytes     // Producer public key
    }
}
```

### 3.9 Claim Bond Transaction

```
claim_bond_tx = {
    version: 1,
    type: 4,
    inputs: [],
    outputs: [{
        output_type: 0,
        amount: bond_to_return,
        pubkey_hash: producer_pubkey_hash,
        lock_until: 0
    }],
    extra_data: {
        public_key: 32 bytes     // Producer public key
    }
}
```

Only valid after unbonding period is complete.

### 3.10 Slash Producer Transaction

Slashing is reserved ONLY for double production (creating two different blocks for the same slot). Invalid blocks are simply rejected by the network - no slashing.

```
slash_tx = {
    version: 1,
    type: 5,
    inputs: [],
    outputs: [],                 // Bond is burned
    extra_data: {
        producer_pubkey: 32 bytes,
        evidence: evidence_data,
        reporter_signature: 64 bytes
    }
}

evidence_data = {
    block_hash_1: 32 bytes,      // First block
    block_hash_2: 32 bytes,      // Second block (must differ)
    slot: uint32                 // Same slot for both
}
```

Burns 100% of producer's bond. This is the only slashable offense because it's the only one that cannot happen by accident.

### 3.11 Epoch Reward Transaction

Epoch rewards are distributed at epoch boundaries (every `slots_per_reward_epoch` slots).
In EpochPool mode, block rewards accumulate in a pool and are distributed proportionally
to the number of blocks each producer created during the epoch.

```
epoch_reward_tx = {
    version: 1,
    type: 10,
    inputs: [],                  // Minted, no inputs
    outputs: [{
        output_type: 0,
        amount: proportional_share,  // (pool * blocks_by_producer) / total_blocks
        pubkey_hash: producer_pubkey_hash,
        lock_until: 0
    }],
    extra_data: {
        epoch: uint64,           // Epoch number
        public_key: 32 bytes     // Recipient producer's public key
    }
}
```

**Pool-First Distribution:**
1. Block rewards accumulate in epoch pool (no coinbase per block)
2. At epoch boundary, pool is distributed proportionally based on blocks produced
3. Last producer (by sorted public key) receives any remainder from rounding
4. Epoch reward outputs require 100 confirmations before spending (same as coinbase)

This ensures fair distribution proportional to each producer's contribution to the epoch.

---

## 4. Blocks

### 4.1 Block Header

```
block_header = {
    version:       uint32,       // Currently 1
    prev_hash:     32 bytes,     // Hash of previous block header
    merkle_root:   32 bytes,     // Merkle root of transactions
    timestamp:     uint64,       // Unix timestamp (seconds)
    slot:          uint32,       // Derived from timestamp
    producer:      32 bytes,     // Producer's public key
    vdf_output:    bytes,        // VDF computation result (~256 bytes)
    vdf_proof:     bytes         // VDF proof (~256 bytes)
}
```

### 4.2 Block Body

```
block_body = {
    tx_count:      varint,
    transactions:  transaction[]
}
```

### 4.3 Block Hash

```
block_hash = HASH(header_without_vdf || vdf_output)
```

### 4.4 Merkle Root

Binary Merkle tree using BLAKE3:

```
merkle_root = merkle_tree([tx_hash for tx in transactions])

def merkle_tree(hashes):
    if len(hashes) == 0:
        return HASH("")
    if len(hashes) == 1:
        return hashes[0]
    if len(hashes) % 2 == 1:
        hashes.append(hashes[-1])  // Duplicate last
    next_level = []
    for i in range(0, len(hashes), 2):
        next_level.append(HASH(hashes[i] || hashes[i+1]))
    return merkle_tree(next_level)
```

### 4.5 VDF Preimage

```
vdf_input = HASH("BLK" || prev_hash || merkle_root || slot || producer)

// Breakdown:
// "BLK"        = 3 bytes
// prev_hash    = 32 bytes
// merkle_root  = 32 bytes
// slot         = 4 bytes (uint32 LE)
// producer     = 32 bytes
// Total: 103 bytes before hashing
```

---

## 5. Consensus Rules

### 5.1 Time Constants

```
GENESIS_TIME = 1769904000         // 2026-02-01T00:00:00Z
SLOT_DURATION = 60                // seconds
EPOCH_SLOTS = 60                  // 1 hour
ERA_BLOCKS = 2_102_400            // ~4 years
BOOTSTRAP_BLOCKS = 10_080         // ~1 week
```

### 5.2 Slot Derivation

```
slot = floor((timestamp - GENESIS_TIME) / SLOT_DURATION)
```

The slot is NOT a free field; it must be derived from the timestamp.

### 5.3 Block Validity

A block B is valid if ALL conditions hold:

```
1. FORMAT:
   B.version == 1
   B.prev_hash references a known valid block

2. TIMING:
   B.timestamp > prev_block.timestamp
   B.timestamp <= local_time + DRIFT (120 seconds)
   B.timestamp >= slot_start + (SLOT_DURATION - NETWORK_MARGIN)
   B.timestamp <= slot_start + SLOT_DURATION + DRIFT

3. SLOT:
   B.slot == floor((B.timestamp - GENESIS_TIME) / SLOT_DURATION)
   B.slot > prev_block.slot

4. PRODUCER (if height >= BOOTSTRAP_BLOCKS):
   B.producer == selected_producer(prev_hash, B.slot)
   B.producer is in active_producer_set

5. VDF:
   vdf_input = HASH("BLK" || prev_hash || B.merkle_root || B.slot || B.producer)
   VDF_verify(vdf_input, B.vdf_output, B.vdf_proof, T_BLOCK) == true

6. TRANSACTIONS:
   B.merkle_root == merkle_tree([tx.hash for tx in B.transactions])
   All transactions are valid
   First transaction is valid coinbase
   No double-spends within block
```

### 5.4 Producer Selection (Deterministic Round-Robin)

DOLI uses **deterministic round-robin rotation**, NOT probabilistic lottery:

```python
def selected_producer(slot, active_producers):
    """
    Deterministic rotation based on bond count (tickets).

    Example with Alice:1, Bob:5, Carol:4 bonds (total 10):
      Tickets: [Alice, Bob, Bob, Bob, Bob, Bob, Carol, Carol, Carol, Carol]
      Slot 0 → Alice, Slot 1-5 → Bob, Slot 6-9 → Carol

    Bob ALWAYS produces exactly 5 of every 10 blocks. No variance, no luck.
    """
    # Sort by pubkey for deterministic ordering
    sorted_producers = sorted(active_producers, key=lambda p: p.pubkey)

    # Calculate total tickets (sum of bond counts)
    total_tickets = sum(p.bond_count for p in sorted_producers)

    # Deterministic selection: slot mod total_tickets
    ticket_index = slot % total_tickets

    # Find ticket owner
    cumulative = 0
    for producer in sorted_producers:
        cumulative += producer.bond_count
        if ticket_index < cumulative:
            return producer.pubkey
```

**Key properties:**
- NOT probabilistic: Each producer gets EXACTLY their proportion of slots
- Deterministic: All nodes compute the same result for any slot
- Equitable ROI: 10 bonds = 10x absolute return, same % ROI as 1 bond

### 5.4.1 Bond Stacking

Producers can stake multiple bonds (1-100) to increase their block production share:

| Parameter | Value | Notes |
|-----------|-------|-------|
| BOND_UNIT | 1,000 DOLI | 1 bond = 1,000 DOLI |
| MIN_BONDS | 1 | Minimum to register |
| MAX_BONDS | 100 | Anti-whale cap (100,000 DOLI max) |

**Example distribution:**
```
Producer  Bonds   Tickets   Blocks/100   ROI/Bond
────────────────────────────────────────────────
Alice       1        1          1         1.0
Bob         5        5          5         1.0
Carol       4        4          4         1.0
Total      10       10         10         1.0 (equal)
```

**ROI Calculation:**
- Alice: 1 bond → 1 block/cycle → ROI = 1/1 = 1.0
- Bob: 5 bonds → 5 blocks/cycle → ROI = 5/5 = 1.0
- Carol: 4 bonds → 4 blocks/cycle → ROI = 4/4 = 1.0

All producers earn the **same percentage return** on their investment.

### 5.5 Chain Selection (Weight-Based Fork Choice)

DOLI uses a weight-based fork choice rule. The chain with the highest accumulated producer weight wins:

```python
def should_reorg(current_chain, new_chain):
    current_weight = accumulated_weight(current_chain.tip)
    new_weight = accumulated_weight(new_chain.tip)
    return new_weight > current_weight

def accumulated_weight(block):
    if block.is_genesis():
        return 0
    return accumulated_weight(block.parent) + block.producer.effective_weight
```

**Weight calculation:**
- New producers: weight = 1
- Weight increases with seniority: `1 + sqrt(months_active / 12)`, capped at 4

This prevents Sybil attacks where an attacker creates many low-weight blocks.

### 5.6 Emission Schedule

```
def block_reward(height):
    era = height // ERA_BLOCKS
    return INITIAL_REWARD >> era   // Right shift = halving

INITIAL_REWARD = 500_000_000      // 5.0 coins (8 decimals)
```

| Era | Block Reward (base units) | Block Reward (coins) |
|-----|---------------------------|----------------------|
| 0   | 500,000,000               | 5.0                  |
| 1   | 250,000,000               | 2.5                  |
| 2   | 125,000,000               | 1.25                 |
| 3   | 62,500,000                | 0.625                |
| ... | ...                       | ...                  |

---

## 6. Producer Registration

### 6.1 Registration Transaction

```
registration_tx = {
    version: 1,
    type: 1,
    inputs: [...],               // To pay fee
    outputs: [
        {
            output_type: 1,      // BOND
            amount: bond_amount(height),
            pubkey_hash: HASH(producer_pubkey),
            lock_until: height + LOCK_DURATION
        },
        ...                      // Change outputs
    ],
    extra_data: {
        public_key: 32 bytes,
        epoch: uint32,
        vdf_output: bytes,
        vdf_proof: bytes
    }
}
```

### 6.2 Bond Amount

```
def bond_amount(height):
    era = height // ERA_BLOCKS
    return INITIAL_BOND * (0.7 ** era)

INITIAL_BOND = 100_000_000_000   // 1000 coins
LOCK_DURATION = ERA_BLOCKS       // ~4 years
```

### 6.3 Registration VDF

```
reg_input = HASH("REG" || public_key || epoch)

// Breakdown:
// "REG"        = 3 bytes
// public_key   = 32 bytes
// epoch        = 4 bytes (uint32 LE)
// Total: 39 bytes before hashing

vdf_output = VDF(reg_input, T_REGISTER(epoch))
```

### 6.4 Dynamic Registration Difficulty

```
def T_REGISTER(epoch):
    R_prev = registrations_in_epoch(epoch - 1)
    D_prev = smoothed_demand(epoch - 1)
    D = (D_prev + R_prev) / 2
    T = T_BASE * max(1, D / R_TARGET)
    return min(T, T_CAP)

T_BASE = 600           // 10 minutes
R_TARGET = 10          // registrations per epoch
T_CAP = 86400          // 24 hours
```

### 6.5 Registration Validity

A registration is valid if:

1. VDF proof verifies with `T_REGISTER(declared_epoch)`
2. Declared epoch is current or previous
3. Public key is not already registered
4. Bond output has correct amount and lock duration
5. Fee is sufficient

### 6.6 Producer Activation

Producer becomes active at start of epoch following confirmation.

### 6.7 Inactivity Rule

```
if producer.consecutive_misses >= MAX_FAILURES:
    producer.status = INACTIVE
    // Bond remains locked
    // Must re-register with new VDF to reactivate

MAX_FAILURES = 50
```

---

## 7. Network Protocol

### 7.1 Message Format

```
message = {
    magic:    4 bytes,           // Network identifier
    command:  12 bytes,          // Command name (ASCII, null-padded)
    length:   uint32,            // Payload length
    checksum: 4 bytes,           // First 4 bytes of HASH(payload)
    payload:  bytes
}
```

### 7.2 Commands

| Command      | Payload                              |
|--------------|--------------------------------------|
| `version`    | Protocol version, height, timestamp  |
| `verack`     | (empty)                              |
| `getblocks`  | Locator hashes, stop hash            |
| `inv`        | Type, hash list                      |
| `getdata`    | Type, hash list                      |
| `block`      | Block header + body                  |
| `tx`         | Transaction                          |
| `getheaders` | Locator hashes, stop hash            |
| `headers`    | Block headers                        |
| `ping`       | Nonce                                |
| `pong`       | Nonce                                |
| `addr`       | Peer addresses                       |
| `getaddr`    | (empty)                              |

### 7.3 Connection Handshake

```
Initiator                     Responder
    |                             |
    |-------- version ----------->|
    |<------- version ------------|
    |<------- verack -------------|
    |-------- verack ------------>|
    |                             |
```

### 7.4 Block Propagation

```
Producer                      Peer
    |                          |
    |------ inv (block) ------>|
    |<----- getdata -----------|
    |------ block ------------>|
    |                          |
```

---

## 8. Networks

DOLI defines three networks with distinct parameters. A single binary connects to any network via the `--network` flag.

**Development workflow:**
```
Devnet (local development) → Testnet (public testing) → Mainnet (production)
```

- **Mainnet**: Production network with real economic value
- **Testnet**: Public test network for integration testing before mainnet
- **Devnet**: Local development network with fast blocks and minimal requirements

### 8.1 Network Identifiers

| Network | ID | Address Prefix | Magic Bytes |
|---------|-----|----------------|-------------|
| Mainnet | 1   | `doli`  | `D0 11 00 01` |
| Testnet | 2   | `tdoli` | `D0 11 00 02` |
| Devnet  | 99  | `ddoli` | `D0 11 00 63` |

### 8.2 Network Parameters

| Parameter | Mainnet | Testnet | Devnet |
|-----------|---------|---------|--------|
| Genesis Time | 2026-02-01T00:00:00Z | 2025-06-01T00:00:00Z | Dynamic |
| Slot Duration | 60s | 10s | 5s |
| P2P Port | 30303 | 40303 | 50303 |
| RPC Port | 8545 | 18545 | 28545 |
| Initial Bond | 1,000 DOLI | 10 DOLI | 1 DOLI |
| Initial Reward | 5 DOLI | 50 DOLI | 500 DOLI |
| VDF Target Time | ~700ms | ~700ms | ~700ms |
| Bootstrap Blocks | 10,080 | 1,000 | 100 |
| Veto Period | 7 days | 1 day | 1 minute |
| Data Directory | `~/.doli/mainnet/` | `~/.doli/testnet/` | `~/.doli/devnet/` |

All networks use hash-chain VDF with ~700ms target time for block production heartbeat.

### 8.3 Network Isolation

Networks are isolated at multiple levels:

1. **Magic bytes**: P2P messages include network-specific magic bytes
2. **Network ID**: Exchanged during peer handshake
3. **Genesis hash**: Validated during peer status exchange
4. **Address prefix**: Prevents cross-network address confusion
5. **Ports**: Different default ports allow running multiple networks simultaneously

### 8.4 Peer Validation

During connection handshake, nodes exchange status messages:

```
status_request = {
    version:      uint32,
    network_id:   uint32,     // Must match local network
    genesis_hash: 32 bytes    // Must match local genesis
}
```

Peers with mismatched `network_id` or `genesis_hash` are immediately disconnected.

### 8.5 Bootstrap Nodes

| Network | Bootstrap Nodes |
|---------|-----------------|
| Mainnet | `/dns4/seed1.doli.network/tcp/30303`<br>`/dns4/seed2.doli.network/tcp/30303` |
| Testnet | `/dns4/testnet-seed1.doli.network/tcp/40303`<br>`/dns4/testnet-seed2.doli.network/tcp/40303` |
| Devnet  | None (local development) |

---

## 9. Test Vectors

### 8.1 SEED Hash (Slot 0)

```
Input:
  literal     = "SEED" = 0x53 0x45 0x45 0x44
  prev_hash   = 0x00 * 32
  slot        = 0 = 0x00000000

Concatenation (40 bytes):
  53454544
  0000000000000000000000000000000000000000000000000000000000000000
  00000000

Result:
  f3b4b63bfa289f7b4b2f11f08cfc26bd38ccdbdd9dae33ef9b77c1fc3b96ebb2
```

### 8.2 SEED Hash (Slot 1)

```
Input:
  literal     = "SEED"
  prev_hash   = 0x00 * 32
  slot        = 1 = 0x01000000 (little-endian)

Result:
  ac1d2a15e55cc413c69036ba29cd08066a560a5bf152ac89a35089eae1fd6bbe
```

### 8.3 SEED Hash (Non-zero prev_hash)

```
Input:
  literal     = "SEED"
  prev_hash   = 0x01 followed by 31 zeros
  slot        = 0

Result:
  1cf7ca92b30ec36c921c1f0f899bb6304b9bb9606ef986ed23afe3baa6b265d1
```

### 8.4 REG Hash

```
Input:
  literal     = "REG" = 0x52 0x45 0x47
  public_key  = 0x00 * 32
  epoch       = 0 = 0x00000000

Concatenation (39 bytes):
  524547
  0000000000000000000000000000000000000000000000000000000000000000
  00000000

Result:
  [compute with BLAKE3-256]
```

### 8.5 BLK Hash

```
Input:
  literal     = "BLK" = 0x42 0x4C 0x4B
  prev_hash   = 0x00 * 32
  merkle_root = 0x00 * 32
  slot        = 0 = 0x00000000
  producer    = 0x00 * 32

Concatenation (103 bytes):
  424C4B
  0000...0000  (32 bytes prev_hash)
  0000...0000  (32 bytes merkle_root)
  00000000     (4 bytes slot)
  0000...0000  (32 bytes producer)

Result:
  [compute with BLAKE3-256]
```

---

## Parameters Summary

| Parameter          | Value                    |
|--------------------|--------------------------|
| GENESIS_TIME       | 1769904000               |
| SLOT_DURATION      | 60                       |
| EPOCH_SLOTS        | 60                       |
| ERA_BLOCKS         | 2,102,400                |
| BOOTSTRAP_BLOCKS   | 10,080                   |
| DRIFT              | 120                      |
| NETWORK_MARGIN     | 15                       |
| VDF_ITERATIONS_DEFAULT | 10,000,000           |
| VDF_ITERATIONS_MIN | 100,000                  |
| VDF_ITERATIONS_MAX | 100,000,000              |
| VDF_TARGET_TIME_MS | 700                      |
| T_REGISTER_BASE    | 600,000,000              |
| T_REGISTER_CAP     | 86,400,000,000           |
| R_TARGET           | 10                       |
| R_CAP              | 100                      |
| INITIAL_REWARD     | 500,000,000              |
| INITIAL_BOND       | 100,000,000,000          |
| COMMITMENT_PERIOD  | 2,102,400                |
| UNBONDING_PERIOD   | 43,200                   |
| MAX_FAILURES       | 50                       |
| REWARD_MATURITY    | 100                      |
| BASE_BLOCK_SIZE    | 1,000,000                |
| MAX_BLOCK_SIZE_CAP | 32,000,000               |
| BLOCK_SIZE_GROWTH  | ×2 per era               |
| EXCLUSION_PERIOD   | 10,080                   |
| TOTAL_SUPPLY       | 2,102,400,000,000,000    |

| VETO_PERIOD        | 604,800 (7 days)         |
| VETO_THRESHOLD     | 33%                      |
| REQUIRED_SIGS      | 3 of 5                   |

---

## 9. Auto-Update System

### 9.1 Release Structure

```
release = {
    version: string,             // Semantic version
    binary_sha256: string,       // SHA-256 hash of binary (hex)
    binary_url_template: string, // URL with {platform} placeholder
    changelog: string,
    published_at: uint64,        // Unix timestamp
    signatures: signature[]
}

signature = {
    public_key: string,          // Maintainer public key (hex)
    signature: string            // Signature over "version:binary_sha256"
}
```

### 9.2 Verification

```
message = version + ":" + binary_sha256
valid_sigs = count(verify(message, sig, maintainer_key) for sig in signatures)
release_valid = valid_sigs >= 3
```

### 9.3 Veto Voting

```
vote_message = {
    version: string,
    vote: uint8,                 // 0 = APPROVE, 1 = VETO
    producer_id: string,
    signature: bytes
}
```

Only active producers can vote. Votes propagate via gossip.

### 9.4 Veto Calculation

```
veto_percent = (veto_count * 100) / total_active_producers

if veto_percent >= 33:
    update REJECTED
else:
    update APPROVED after 7 days
```

---

*For architecture overview, see [architecture.md](architecture.md)*
