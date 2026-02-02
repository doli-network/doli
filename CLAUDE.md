# CLAUDE.md

This file provides comprehensive guidance to Claude Code (claude.ai/code) when working with code in this repository. It serves as the **brain** of the DOLI blockchain project, containing deep technical knowledge of every system component.

## Prerequisites

1. Check `specs/SPECS.md` and `docs/DOCS.md` for instructions on how to build and test the project.
2. **All commands must be run inside the Nix environment:**

```bash
nix --extra-experimental-features "nix-command flakes" develop
```

## Command Output Filtering

Reduce noise when running commands. Redirect output and filter for relevant information:

```bash
# General pattern
command 2>&1 | grep -i "keyword1\|keyword2" | awk '!seen[$0]++' | head -15

# Build commands
build_command > /tmp/output.log 2>&1 && grep -i "error\|warning\|failed\|success" /tmp/output.log | head -10

# Test commands
test_command 2>&1 | grep -i "pass\|fail\|error" | sort -u | head -20

# Test summary
test_command 2>&1 | grep -E "^\+[0-9]+|-[0-9]+|passed|failed|Some tests" | tail -5
```

## Build, Test & Run Commands

### Building

```bash
nix --extra-experimental-features "nix-command flakes" develop --command bash -c "cargo build"                    # Debug build
nix --extra-experimental-features "nix-command flakes" develop --command bash -c "cargo build --release"          # Release build
nix --extra-experimental-features "nix-command flakes" develop --command bash -c "cargo clippy"                   # Linting
nix --extra-experimental-features "nix-command flakes" develop --command bash -c "cargo fmt --check"              # Format check
nix --extra-experimental-features "nix-command flakes" develop --command bash -c "cargo fmt"                      # Auto-format
nix --extra-experimental-features "nix-command flakes" develop --command bash -c "cargo doc --workspace --no-deps --open"  # Generate docs
```

### Testing

```bash
nix --extra-experimental-features "nix-command flakes" develop --command bash -c "cargo test"                     # All workspace tests
nix --extra-experimental-features "nix-command flakes" develop --command bash -c "cargo test -p core"             # Single crate
nix --extra-experimental-features "nix-command flakes" develop --command bash -c "cargo test -p core test_name"   # Single test
```

### Fuzz Testing

Fuzz tests are in a **separate workspace** at `testing/fuzz/`:

```bash
cd testing/fuzz
nix --extra-experimental-features "nix-command flakes" develop --command bash -c "cargo +nightly fuzz run fuzz_block_deserialize"
nix --extra-experimental-features "nix-command flakes" develop --command bash -c "cargo +nightly fuzz run fuzz_tx_deserialize"
nix --extra-experimental-features "nix-command flakes" develop --command bash -c "cargo +nightly fuzz run fuzz_vdf_verify"
```

### Test Scripts

Scripts are in `scripts/`. Check `scripts/README.md` before creating new ones.

```bash
ls scripts/*.sh                          # List scripts
./scripts/launch_testnet.sh              # Example
./scripts/test_3node_proportional_rewards.sh
```

### Running Binaries

```bash
# Node
nix --extra-experimental-features "nix-command flakes" develop --command bash -c "cargo run -p doli-node -- run"                      # mainnet
nix --extra-experimental-features "nix-command flakes" develop --command bash -c "cargo run -p doli-node -- --network testnet run"   # testnet
nix --extra-experimental-features "nix-command flakes" develop --command bash -c "cargo run -p doli-node -- --network devnet run"    # devnet

# CLI wallet
nix --extra-experimental-features "nix-command flakes" develop --command bash -c "cargo run -p doli-cli -- wallet new"
nix --extra-experimental-features "nix-command flakes" develop --command bash -c "cargo run -p doli-cli -- wallet balance <address>"
```

---

## Project Overview

DOLI is a Rust-based cryptocurrency where **Verifiable Delay Functions (VDF)** are the **primary consensus mechanism**—the first blockchain with this design. **Time is the scarce resource**, not energy or stake.

**Consensus**: Proof of Time (PoT) using hash-chain VDF with deterministic round-robin producer selection based on bond count.

---

## Architecture

### Crate Dependency Flow

```
bins/node (doli-node)          bins/cli (doli-cli)
    │                              │
    ├─→ network ─┐                 │
    ├─→ rpc ─────┤                 │
    ├─→ mempool ─┤                 │
    ├─→ storage ─┤                 │
    ├─→ updater ─┤                 │
    │            ▼                 │
    └─────────→ core ←─────────────┘
                 │
                 ▼
         ┌───────┴───────┐
         ▼               ▼
      crypto            vdf
```

### Crate Responsibilities

| Crate | Purpose | Key Files |
|-------|---------|-----------|
| `crypto` | BLAKE3-256 hashing, Ed25519 signatures, merkle trees, domain separation | `hash.rs`, `keys.rs`, `merkle.rs`, `signature.rs` |
| `vdf` | Wesolowski VDF over class groups (registration), hash-chain VDF (blocks). Uses `rug` (GMP bindings) | `vdf.rs`, `class_group.rs` |
| `core` | Types, validation, consensus parameters, deterministic scheduler. `tpop/` provides telemetry (NOT consensus) | `consensus.rs`, `scheduler.rs`, `validation.rs`, `heartbeat.rs` |
| `storage` | RocksDB persistence for blocks, UTXO, chain state, producer registry | `block_store.rs`, `utxo.rs`, `producer.rs` |
| `network` | libp2p P2P layer: gossipsub, Kademlia DHT, sync, equivocation detection | `service.rs`, `sync/`, `gossip.rs`, `equivocation.rs` |
| `mempool` | Transaction pool with fee policies, double-spend detection | `pool.rs` |
| `rpc` | JSON-RPC server (Axum) for wallet/explorer interaction | `methods.rs`, `server.rs` |
| `updater` | Auto-update with 3/5 multisig, 7-day veto period, 40% threshold | `lib.rs` |

---

## Consensus: Proof of Time (PoT)

### Core Design Principles

1. **Time as Scarce Resource**: VDF ensures sequential computation, not parallelizable
2. **Deterministic Selection**: `slot % total_bonds` selects producer (no grinding)
3. **Bond-Weighted Round-Robin**: Each bond = 1 ticket per cycle
4. **Weight-Based Fork Choice**: Accumulated producer seniority weight determines chain

### Block Production Flow

```
1. Eligibility Check
   ├─ Current slot > last_produced_slot?
   ├─ Early block existence check (optimization)
   ├─ Get active producers with weights
   ├─ Deterministic selection: slot % total_bonds
   └─ Check eligibility window based on slot offset

2. Build Block
   ├─ Create BlockBuilder with prev_hash, prev_slot, producer_pubkey
   ├─ Add epoch reward coinbase (if epoch boundary)
   ├─ Select transactions from mempool (max 1MB, highest fees first)
   └─ Build header with merkle root

3. Compute VDF
   ├─ VDF input: HASH(prev_hash || merkle_root || slot || producer_key)
   ├─ Compute hash-chain VDF (10M iterations, ~700ms)
   └─ Safety check: verify no block appeared during VDF

4. Finalize & Broadcast
   ├─ Create final header with VDF output
   ├─ Apply block locally (update UTXO, chain state)
   └─ Broadcast to network via gossipsub
```

### Producer Selection Algorithm

**Ticket-Based Round-Robin** (in `scheduler.rs`):
- Producers sorted by pubkey (deterministic ordering)
- Each producer gets consecutive tickets = bond count
- Primary: `slot % total_bonds` → find ticket owner
- Fallback (ranks 1-9): offset by `total_bonds * rank / 10`

**Example with 3 producers (10 total bonds):**
```
Alice: 1 bond  → tickets 0
Bob:   5 bonds → tickets 1-5
Carol: 4 bonds → tickets 6-9

Slot 0: ticket 0 → Alice
Slot 1: ticket 1 → Bob
...
Slot 5: ticket 5 → Bob
Slot 6: ticket 6 → Carol
...
Slot 10: ticket 0 → Alice (cycle repeats)
```

**Fallback Windows (10-second slot)**:
| Time | Eligible Ranks |
|------|---------------|
| 0-1s | rank 0 only |
| 1-2s | rank 0-1 |
| 2-3s | rank 0-2 |
| ... | ... |
| 9-10s | rank 0-9 |

### Fork Choice: Weight-Based

Producer weight increases with seniority (discrete yearly tiers):
- **Year 0-1**: weight = 1
- **Year 1-2**: weight = 2
- **Year 2-3**: weight = 3
- **Year 3+**: weight = 4 (cap)

**Formula**: `accumulated_weight(block) = accumulated_weight(parent) + producer_weight`

**Heaviest chain wins**: accumulated weight from genesis determines canonical chain.

**Security Property**: Prevents attackers from creating many new-producer blocks to outpace established chains. Attacker needs 3-4x more blocks with junior producers.

---

## Time Structure

| Concept | Mainnet/Testnet | Devnet |
|---------|-----------------|--------|
| **Slot Duration** | 10 seconds | 10 seconds |
| **Slots per Epoch** | 360 (1 hour) | 360 |
| **Blocks per Era** | 12,614,400 (~4 years) | 576 (~96 min accelerated) |
| **Halving Interval** | 12,614,400 blocks | 576 blocks |
| **Slots per Year** | 3,153,600 | 144 (accelerated) |

---

## Network Configuration

| Network | ID | Magic Bytes | P2P Port | RPC Port | Address Prefix | Genesis Time |
|---------|----|-------------|----------|----------|----------------|--------------|
| Mainnet | 1 | `D0 11 00 01` | 30303 | 8545 | `doli` | 2026-02-01 |
| Testnet | 2 | `D0 11 00 02` | 40303 | 18545 | `tdoli` | 2026-01-29 |
| Devnet | 99 | `D0 11 00 63` | 50303 | 28545 | `ddoli` | Dynamic |

---

## Economic Parameters

### Supply and Rewards

| Parameter | Value | Notes |
|-----------|-------|-------|
| **Total Supply** | 25,228,800 DOLI | Fixed cap |
| **Initial Block Reward** | 1 DOLI | Halves every ~4 years |
| **Reward Distribution** | 100% to block producer | Via coinbase transaction |
| **Coinbase Maturity** | 100 blocks | ~17 minutes |

### Emission Schedule

| Era | Years | Reward | Cumulative | % of Total |
|-----|-------|--------|------------|------------|
| 1 | 0-4 | 1.0 DOLI | 12,614,400 | 50.00% |
| 2 | 4-8 | 0.5 DOLI | 18,921,600 | 75.00% |
| 3 | 8-12 | 0.25 DOLI | 22,075,200 | 87.50% |
| 4 | 12-16 | 0.125 DOLI | 23,652,000 | 93.75% |
| 5 | 16-20 | 0.0625 DOLI | 24,440,400 | 96.88% |
| 6 | 20-24 | 0.03125 DOLI | 24,834,600 | 98.44% |

### Bond System

| Parameter | Mainnet/Testnet | Devnet |
|-----------|-----------------|--------|
| **Bond Unit** | 100 DOLI | 1 DOLI |
| **Max Bonds/Producer** | 100 | 100 |
| **Max Stake/Producer** | 10,000 DOLI | 100 DOLI |
| **Commitment Period** | 4 years | Accelerated |
| **Withdrawal Delay** | 7 days (60,480 slots) | 60 slots |

### Bond Stacking Architecture

**Core Structure** (`consensus.rs:537-677`):
```rust
pub struct BondEntry {
    pub creation_slot: Slot,     // When this bond was created
    pub amount: Amount,           // Always BOND_UNIT (100 DOLI)
}

pub struct ProducerBonds {
    pub bonds: Vec<BondEntry>,                    // FIFO ordered (oldest first)
    pub pending_withdrawals: Vec<PendingWithdrawal>,
}

pub struct PendingWithdrawal {
    pub bond_count: u32,
    pub request_slot: Slot,
    pub net_amount: Amount,        // After penalty
    pub penalty_amount: Amount,    // BURNED
    pub destination: Hash,
}
```

**Key Features**:
- Each bond vests independently based on its own `creation_slot`
- FIFO withdrawal: oldest bonds withdrawn first (maximizes recovery)
- Selection weight = total bond count (1:1 mapping)
- Penalties are 100% burned (deflationary)

### Vesting Schedule (Early Withdrawal Penalties)

**Implementation** (`consensus.rs:221`):
```rust
pub fn withdrawal_penalty_rate(bond_age_slots: Slot) -> u8 {
    let years = bond_age_slots / YEAR_IN_SLOTS;
    match years {
        0 => 75,  // Year 1: 75% penalty
        1 => 50,  // Year 2: 50% penalty
        2 => 25,  // Year 3: 25% penalty
        _ => 0,   // Year 4+: 0% penalty (fully vested)
    }
}
```

| Bond Age | Penalty | Net Recovery | Status |
|----------|---------|--------------|--------|
| Year 0-1 | 75% burned | 25% | Early exit |
| Year 1-2 | 50% burned | 50% | Partial vest |
| Year 2-3 | 25% burned | 75% | Near vest |
| Year 3+ | 0% | 100% | Fully vested |

### Registration Fees

Base fee: 0.001 DOLI, scaling with network congestion:
| Pending Registrations | Fee Multiplier |
|----------------------|----------------|
| 0-4 | 1.00x |
| 5-9 | 1.50x |
| 10-19 | 2.00x |
| 20-49 | 3.00x |
| 50-99 | 4.50x |
| 100-199 | 6.50x |
| 200-299 | 8.50x |
| 300+ | 10.00x (cap) |

---

## VDF System (Dual Architecture)

DOLI uses **two VDF types** for different purposes:

### 1. Wesolowski VDF (Registration Anti-Sybil)

| Parameter | Mainnet/Testnet | Devnet |
|-----------|-----------------|--------|
| **Base Iterations** | 600M (~10 min) | 5M (~5s) |
| **Cap** | 86.4B (~24 hours) | - |
| **Discriminant Bits** | 2048 | 256 |
| **Proof Type** | Compact (~512 bytes) | Same |
| **Verification** | O(log t) group ops | Same |

### 2. Hash-Chain VDF (Block Production)

| Parameter | All Networks |
|-----------|-------------|
| **Iterations** | 10,000,000 |
| **Target Time** | ~700ms |
| **Algorithm** | Sequential BLAKE3 hashing |
| **Verification** | Recompute (linear, but fast) |

**Anti-Grinding**: VDF input includes `prev_hash` (unknown until previous block), preventing pre-computation.

### Heartbeat VDF (`heartbeat.rs:257-261`)

```rust
// Input computation
input = H("DOLI_HEARTBEAT_V1" || producer || slot || prev_hash)

// Verification
computed = hash_chain_vdf(input, HEARTBEAT_VDF_ITERATIONS)
assert!(computed == provided_output)
```

---

## Transaction Types

| Type | ID | Purpose | Special Data |
|------|----|---------|--------------|
| Transfer | 0 | Standard value transfer | - |
| Registration | 1 | Producer registration | VDF proof, chained hash, sequence |
| Exit | 2 | Start unbonding | - |
| ClaimReward | 3 | **DEPRECATED** | - |
| ClaimBond | 4 | Claim after unbonding | - |
| SlashProducer | 5 | Slash for double-production | EquivocationProof |
| Coinbase | 6 | Block reward | - |
| AddBond | 7 | Increase stake | Bond count |
| RequestWithdrawal | 8 | Start 7-day withdrawal | Bond count, destination |
| ClaimWithdrawal | 9 | Complete withdrawal | Withdrawal index |
| EpochReward | 10 | **DEPRECATED** | - |
| RemoveMaintainer | 11 | 3/5 multisig remove | Signatures |
| AddMaintainer | 12 | 3/5 multisig add | Signatures |

### Transaction Lifecycle: Bond Withdrawal

**Three-Transaction Process:**

1. **RequestWithdrawal** (`TxType::RequestWithdrawal = 8`)
   - Initiates withdrawal, calculates penalties using FIFO
   - Bonds immediately removed from active set
   - Selection weight decreases immediately
   - Penalties calculated and locked for burning

2. **7-day delay** (60,480 slots)
   - Funds in pending state
   - Prevents flash attacks

3. **ClaimWithdrawal** (`TxType::ClaimWithdrawal = 9`)
   - Retrieves net amount after delay
   - Penalties burned at this point

---

## Block Structure

```rust
BlockHeader {
    version: u32,           // Protocol version (currently 1)
    prev_hash: Hash,        // Previous block hash
    merkle_root: Hash,      // Transaction merkle root
    presence_root: Hash,    // Always Hash::ZERO (legacy field)
    timestamp: u64,         // Unix timestamp
    slot: u32,              // Slot number
    producer: PublicKey,    // Block producer
    vdf_output: VdfOutput,  // Hash-chain result (32 bytes)
    vdf_proof: VdfProof,    // Empty for hash-chain VDF
}
```

---

## Validation Rules

### Block Validation Flow (`validation.rs`)

Block validation performs 7 checks:
1. **Header Validation** (lines 480-553): Version, timestamp progression, slot derivation
2. **Block Size** (lines 561-568): Max 1MB (base), doubles every era up to 32MB cap
3. **Merkle Root** (line 571-573): Must match computed root
4. **Transaction Validation** (lines 579-581): Signatures, UTXO existence, amounts
5. **Internal Double-Spend** (line 584): No duplicate inputs within block
6. **VDF Validation** (lines 1926-1962): Recompute hash-chain and compare
7. **Producer Eligibility** (lines 1971-2004): Deterministic scheduler verification

### Header Validation Rules (`validation.rs:480-553`)

| Rule | Line | Error |
|------|------|-------|
| Version must be 1 | 485-487 | `InvalidVersion(u32)` |
| Timestamp must advance | 490-495 | `InvalidTimestamp { block, expected }` |
| Not too far in future | 498-500 | `TimestampTooFuture(u64)` |
| Slot derives from timestamp | 503-508 | `InvalidSlot { got, expected }` |
| Slot must advance | 512-517 | `SlotNotAdvancing { got, prev }` |
| Timestamp within slot window | 520-527 | `InvalidTimestamp { block, expected }` |
| Slot not too future | 532-540 | `SlotTooFuture { got, current, max_future }` |
| Slot not too past | 542-550 | `SlotTooPast { got, current, max_past }` |

### Transaction Type-Specific Validation

| Type | Validation Function | Key Rules |
|------|---------------------|-----------|
| Registration | `validate_registration_data()` (1205-1277) | Bond output required, VDF proof, chain linkage |
| Exit | `validate_exit_data()` (1343-1373) | No inputs, no outputs |
| ClaimBond | `validate_claim_bond_data()` (1434-1474) | No inputs, exactly one Normal output |
| SlashProducer | `validate_slash_data()` (1486-1571) | Both headers same slot, different hashes, VDF verified |
| AddBond | `validate_add_bond_data()` (1584-1618) | Inputs required, Normal outputs only (change) |
| RequestWithdrawal | `validate_withdrawal_request_data()` (1628-1668) | No inputs/outputs, positive bond count |
| ClaimWithdrawal | `validate_claim_withdrawal_data()` (1678-1712) | No inputs, exactly one Normal output |
| MaintainerChange | `validate_maintainer_change_data()` (1760-1797) | No inputs/outputs, valid 3/5 multisig |

### Validation Error Types (28+ variants)

**Block/Header Errors:**
- `InvalidVersion(u32)`, `InvalidTimestamp { block, expected }`
- `TimestampTooFuture(u64)`, `InvalidSlot { got, expected }`
- `SlotNotAdvancing { got, prev }`, `SlotTooFuture/TooPast`
- `InvalidMerkleRoot`, `InvalidVdfProof`, `InvalidProducer`
- `BlockTooLarge { size, max }`, `MissingCoinbase`, `InvalidCoinbase(String)`

**Transaction Errors:**
- `InvalidTransaction(String)`, `DoubleSpend`
- `InsufficientFunds { inputs, outputs }`, `InvalidSignature { index }`
- `OutputLocked { lock_height, current_height }`
- `OutputNotFound { tx_hash, output_index }`
- `OutputAlreadySpent { tx_hash, output_index }`
- `AmountOverflow { context }`, `AmountExceedsSupply { amount, max }`
- `PubkeyHashMismatch { expected, got }`

**Type-Specific Errors:**
- `InvalidRegistration(String)`, `InvalidBond(String)`
- `InvalidClaim(String)`, `InvalidBondClaim(String)`
- `InvalidSlash(String)`, `InvalidAddBond(String)`
- `InvalidWithdrawalRequest(String)`, `InvalidClaimWithdrawal(String)`
- `InvalidEpochReward(String)`, `UnexpectedEpochReward`
- `MissingEpochReward { epoch }`, `EpochRewardMismatch { reason }`
- `InvalidMaintainerChange(String)`

---

## Network Layer (libp2p)

### Protocol Stack

```
TCP → Noise Encryption (Ed25519) → Yamux Multiplexing
```

### Gossipsub Topics

| Topic | Purpose | Max Size |
|-------|---------|----------|
| `/doli/blocks/1` | Block propagation | 1 MB |
| `/doli/txs/1` | Transaction propagation | - |
| `/doli/producers/1` | Producer announcements | - |
| `/doli/votes/1` | Governance votes | - |

### Kademlia DHT

- Protocol: `/doli/kad/1.0.0`
- Replication factor: 20
- Query timeout: 60 seconds

### Sync Protocol

**Header-First Strategy**:
1. Download headers (up to 2000/request)
2. Validate VDF chain linkage
3. Parallel body download (up to 8 peers, 128 blocks/request)
4. Apply blocks

### Equivocation Detection (`sync/equivocation.rs`)

**Only slashable offense**: Double-production (same slot, different blocks)

**Detection Flow** (lines 118-155):
1. Track `(producer, slot)` pairs with full `BlockHeader`
2. If existing header hash != new block hash → **EQUIVOCATION**
3. Create `EquivocationProof` with both full headers (includes VDF proofs)
4. VDF proofs prevent fabricated evidence attacks
5. LRU eviction: tracks up to `MAX_TRACKED_SLOTS` (1000) entries

**On Detection**:
1. Create `EquivocationProof` with both headers
2. Convert to `SlashProducer` transaction
3. **100% of producer's bond is burned**

### Peer Scoring

| Infraction | Penalty |
|------------|---------|
| Invalid Block | -100 |
| Invalid Transaction | -20 |
| Timeout | -5 × count |
| Spam | -50 |
| Malformed Message | -30 |

**Thresholds**: Disconnect at -200, ban at -500 (1 hour)

### Rate Limiting (Token Bucket)

| Resource | Per-Peer Limit | Global Limit |
|----------|---------------|--------------|
| Blocks | 10/min | 100/min |
| Transactions | 50/sec | 200/sec |
| Bandwidth | 1 MB/sec | 10 MB/sec |

---

## Storage Layer (RocksDB)

### Column Families

| CF | Key | Value |
|----|-----|-------|
| `headers` | Block hash | BlockHeader |
| `bodies` | Block hash | Vec<Transaction> |
| `height_index` | Height (u64 BE) | Block hash |
| `slot_index` | Slot (u32 BE) | Block hash |
| `presence` | Hash | PresenceCommitment |

### UTXO Model

- **HashMap-based** in-memory UTXO set
- **Outpoint**: (tx_hash, output_index)
- **Output**: (output_type, amount, pubkey_hash, lock_until)
- Serialization: Bincode

### Producer Registry (`storage/producer.rs`)

```rust
pub struct ProducerInfo {
    pub public_key: PublicKey,
    pub registered_at: u64,         // Block height when registered
    pub bond_amount: u64,           // Total locked value
    pub bond_outpoint: (Hash, u32), // Primary bond location
    pub status: ProducerStatus,
}

pub enum ProducerStatus {
    Active,
    Unbonding,
    Exited,
    Slashed,
}
```

---

## RPC Server (JSON-RPC)

### Available Methods (18 total)

| Category | Method | Description |
|----------|--------|-------------|
| **Chain** | `getBlockByHash` | Get block by hash |
| | `getBlockByHeight` | Get block by height |
| | `getChainInfo` | Chain height, tip hash, sync status |
| | `getBlockHeader` | Header only, no transactions |
| **Transaction** | `sendTransaction` | Submit signed transaction |
| | `getTransaction` | Get transaction by hash |
| | `getTransactionReceipt` | Confirmation status |
| **UTXO** | `getUtxos` | Get UTXOs for address |
| | `getUtxosByOutpoint` | Get specific UTXO |
| **Producer** | `getProducerInfo` | Producer status, bonds, rewards |
| | `getProducerList` | All active producers |
| | `getSchedule` | Next N slot assignments |
| **Network** | `getPeerInfo` | Connected peers |
| | `getNetworkInfo` | Network stats |
| **Mempool** | `getMempoolInfo` | Size, fee stats |
| | `getRawMempool` | Transaction hashes |
| **Wallet** | `validateAddress` | Check address format |
| | `estimateFee` | Fee estimation |

---

## Update System

### Governance Parameters

| Parameter | Mainnet/Testnet | Devnet |
|-----------|-----------------|--------|
| **Veto Period** | 7 days | 60 seconds |
| **Grace Period** | 48 hours | 30 seconds |
| **Veto Threshold** | 40% of weighted power | Same |
| **Required Signatures** | 3/5 maintainers | Same |

### Maintainer Bootstrap (`core/maintainer.rs`)

- **Initial Set**: First 5 registered producers become maintainers automatically
- **Threshold**: 3/5 multisig for all changes
- **Slashing Integration**: Producers removed from maintainer set automatically when slashed

---

## Key Constants Reference

### Consensus

```rust
PROTOCOL_VERSION = 1
SLOT_DURATION = 10          // seconds
SLOTS_PER_EPOCH = 360       // 1 hour
BLOCKS_PER_ERA = 12_614_400 // ~4 years
MAX_FALLBACK_RANK = 9       // 10 total fallback producers
PRIMARY_WINDOW_MS = 3_000   // 0-3s: rank 0 only
MAX_DRIFT = 60              // Clock drift tolerance (seconds)
MAX_FUTURE_SLOTS = 5        // Maximum slots in future
MAX_PAST_SLOTS = 10         // Maximum slots in past
```

### Economics

```rust
TOTAL_SUPPLY = 2_522_880_000_000_000     // 25,228,800 DOLI in base units
INITIAL_REWARD = 100_000_000             // 1 DOLI
BOND_UNIT = 10_000_000_000               // 100 DOLI
MAX_BONDS_PER_PRODUCER = 100             // 10,000 DOLI max
WITHDRAWAL_DELAY_SLOTS = 60_480          // 7 days
COINBASE_MATURITY = 100                  // blocks
YEAR_IN_SLOTS = 3_153_600                // 365 days
```

### VDF

```rust
T_BLOCK = 10_000_000                     // ~700ms hash-chain
T_REGISTER_BASE = 600_000_000            // ~10 min Wesolowski
DISCRIMINANT_BITS = 2048                 // Class group security
HEARTBEAT_VDF_ITERATIONS = 10_000_000    // Same as T_BLOCK
```

### Cryptography

```rust
HASH_SIZE = 32                           // BLAKE3-256
PUBLIC_KEY_SIZE = 32                     // Ed25519
SIGNATURE_SIZE = 64                      // Ed25519
```

### Domain Separation Tags

```rust
SIGN_DOMAIN = "DOLI_SIGN_V1"
ADDRESS_DOMAIN = "DOLI_ADDR_V1"
TX_DOMAIN = "DOLI_TX_V1"
BLOCK_DOMAIN = "DOLI_BLOCK_V1"
VDF_DOMAIN = "DOLI_VDF_V1"
HEARTBEAT_DOMAIN = "DOLI_HEARTBEAT_V1"
```

### Network Detection

```rust
MAX_TRACKED_SLOTS = 1000                 // Equivocation LRU cache
MIN_WITNESS_SIGNATURES = 2               // Heartbeat witnesses
```

---

## Code Conventions

- **Branches**: `feature/`, `fix/`, `docs/`, `refactor/`
- **Commits**: Conventional Commits (`feat(scope): description`, `fix(scope): description`)
- **Git Author**: All commits must use `--author="E. Weil <weil@doli.network>"`
- **Line length**: 100 characters max
- **Tests**: Unit tests in same file, integration tests in `testing/`
- **Crypto code**: Use property-based testing (proptest)

### File Naming

| Type | Convention | Examples |
|------|------------|----------|
| Documentation | lowercase with underscores | `protocol.md`, `running_a_node.md` |
| Master indexes | UPPERCASE | `README.md`, `CLAUDE.md`, `DOCS.md`, `SPECS.md`, `WHITEPAPER.md` |
| Flat structure | No subdirectories in `docs/`, `specs/` | Exception: `docs/legacy/` |

---

## Documentation Alignment (MANDATORY)

Documentation drift is a protocol liability.

### Truth Hierarchy

```
1. WHITEPAPER.md    ← Defines WHAT the protocol IS (source of truth)
2. specs/*          ← Defines HOW it works technically
3. docs/*           ← Defines HOW to use it
4. Code             ← Implements the above
```

**Rules:**
- Code must implement WHITEPAPER—if they differ, code is wrong
- Specs must reflect code—if they differ, update specs
- Docs must describe reality—never document aspirations

---

## Workflow Rules

### Pre-Commit Gate (BLOCKING)

**BEFORE EVERY COMMIT, you MUST complete this checklist. No exceptions.**

```
┌─────────────────────────────────────────────────────────────────┐
│  PRE-COMMIT DOCUMENTATION GATE                                  │
│  Cannot proceed to `git commit` until ALL boxes are checked     │
├─────────────────────────────────────────────────────────────────┤
│  [ ] 1. Run `/sync-docs` OR manually verify:                    │
│         - specs/protocol.md reflects any protocol changes       │
│         - docs/*.md reflects any user-facing changes            │
│         - docs/rpc_reference.md reflects any RPC changes        │
│         - docs/cli.md reflects any CLI changes                  │
│                                                                 │
│  [ ] 2. If implementation adds/changes behavior:                │
│         - specs/ updated with technical details                 │
│         - docs/ updated with usage instructions                 │
│                                                                 │
│  [ ] 3. If implementation adds/changes tests:                   │
│         - scripts/README.md updated (if test script added)      │
│         - Test coverage documented in relevant docs             │
│                                                                 │
│  [ ] 4. State which docs were checked/updated in commit message │
└─────────────────────────────────────────────────────────────────┘
```

**If you skip this gate, you are violating CLAUDE.md.**

### Milestone Workflow

When working on implementation milestones (e.g., from IMPLEMENTATION_CLAIM_REWARD.md):

1. **Implement** - Write the code/tests for the milestone
2. **Verify** - Run `nix --extra-experimental-features "nix-command flakes" develop --command bash -c "cargo build && cargo clippy && cargo test"` (all must pass)
3. **Document** - Complete Pre-Commit Gate checklist above
4. **Commit** - Using conventions (conventional commits, `--author`, HEREDOC format)
5. **Push** - Push to remote
6. **Stop** - Report completion, wait for next milestone

A milestone is not complete until pushed. Do not stop between steps.

### For All Changes

After tests pass:
1. **EXECUTE Pre-Commit Gate above** (this is not optional)
2. Update relevant documentation immediately
3. Commit code and docs together

### Bug Fixing Protocol (MANDATORY)

**Use `/fix-bug` for all bug fixes.** Quick fixes that mask symptoms are prohibited.

Key constraints:
- You are prohibited from implementing anything not specified in WHITEPAPER.md
- If code differs from whitepaper, whitepaper is truth
- Never commit without explicit user validation

---

## Critical Implementation Details

### Anti-Sybil Protection

1. **Registration VDF**: 10+ minutes sequential work per identity
2. **Chained Hashes**: Each registration references previous, preventing parallel computation
3. **Sequence Numbers**: Monotonic, prevents replay

### Anti-Grinding Protection

1. **VDF Input Unpredictability**: Includes `prev_hash` (unknown until previous block)
2. **Deterministic Selection**: No influence on future selection via block content
3. **~700ms VDF**: Only ~14 attempts possible per 10s slot

### Double-Production Prevention

1. **Signed Slots DB**: Node tracks locally signed slots
2. **Lock File**: Prevents concurrent producer instances
3. **Network Detection**: Equivocation detector catches violations
4. **100% Slashing**: Complete bond burn, permanent exclusion

### Transaction Malleability Prevention

- Signature is **excluded** from transaction hash
- Hash covers: version, type, inputs (minus sigs), outputs, extra_data
- Prevents third-party modification of signed transactions

---

## Testing Infrastructure

### Unit Tests

Each crate has comprehensive unit tests (run with `cargo test -p <crate>`).

### Integration Tests

Located in `testing/integration/`:
- Multi-node scenarios
- Network partitioning
- Reorg handling
- Slashing scenarios
- Bond stacking tests

### Fuzz Tests (6 targets)

```
fuzz_block_deserialize
fuzz_tx_deserialize
fuzz_vdf_verify
fuzz_merkle_proof
fuzz_address_encoding
fuzz_signature_verify
```

### Test Scripts (23+)

See `scripts/README.md` for complete registry. Key scripts:
- `launch_testnet.sh` - Start local testnet
- `test_3node_proportional_rewards.sh` - Multi-node reward distribution
- `test_bond_stacking.sh` - Bond lifecycle tests
- `test_equivocation.sh` - Slashing scenario tests

### Benchmarks

VDF benchmarks in `testing/benchmarks/results/hardware_matrix.md` with performance data across platforms.

---

## File Reference Index

### Core Crate

| File | Purpose | Key Lines |
|------|---------|-----------|
| `crates/core/src/consensus.rs` | Consensus parameters, bond structures | 221 (vesting), 537-677 (bonds) |
| `crates/core/src/scheduler.rs` | Deterministic round-robin scheduler | - |
| `crates/core/src/validation.rs` | Block and transaction validation | 480-553 (header), 1036-1136 (tx) |
| `crates/core/src/block.rs` | Block and header structures | 148 (merkle verify) |
| `crates/core/src/transaction.rs` | Transaction types and structures | - |
| `crates/core/src/types.rs` | Core type aliases (Amount, Slot, etc.) | - |
| `crates/core/src/network.rs` | Network configuration | - |
| `crates/core/src/genesis.rs` | Genesis block generation | - |
| `crates/core/src/maintainer.rs` | Maintainer bootstrap system (702 lines) | - |
| `crates/core/src/heartbeat.rs` | Heartbeat VDF validation | 257-261 (verify), 325-354 (full) |
| `crates/core/src/tpop/` | Telemetry (NOT consensus) | - |

### Crypto Crate

| File | Purpose | Key Lines |
|------|---------|-----------|
| `crates/crypto/src/hash.rs` | BLAKE3-256 hashing | - |
| `crates/crypto/src/keys.rs` | Ed25519 key management (764 lines) | - |
| `crates/crypto/src/signature.rs` | Signature functions | 241-391 (sign/verify) |
| `crates/crypto/src/merkle.rs` | Merkle tree implementation | - |

### VDF Crate

| File | Purpose |
|------|---------|
| `crates/vdf/src/lib.rs` | Public API, constants |
| `crates/vdf/src/vdf.rs` | Wesolowski VDF compute/verify |
| `crates/vdf/src/class_group.rs` | Class group arithmetic (GMP) |

### Network Crate

| File | Purpose | Key Lines |
|------|---------|-----------|
| `crates/network/src/service.rs` | Main network service | - |
| `crates/network/src/gossip.rs` | Gossipsub configuration | - |
| `crates/network/src/sync/manager.rs` | Sync orchestration | - |
| `crates/network/src/sync/equivocation.rs` | Double-production detection (359 lines) | 118-155 (check) |
| `crates/network/src/scoring.rs` | Peer scoring system | - |
| `crates/network/src/rate_limit.rs` | DoS protection | - |

### Storage Crate

| File | Purpose |
|------|---------|
| `crates/storage/src/block_store.rs` | Block persistence |
| `crates/storage/src/utxo.rs` | UTXO set management |
| `crates/storage/src/producer.rs` | Producer registry |

### Mempool Crate

| File | Purpose | Key Lines |
|------|---------|-----------|
| `crates/mempool/src/pool.rs` | Transaction pool (589 lines) | 105-214 (add_tx) |

### Binaries

| File | Purpose |
|------|---------|
| `bins/node/src/node.rs` | Main node logic, block production |
| `bins/node/src/producer/` | Producer-specific logic |
| `bins/cli/src/main.rs` | CLI wallet entry point |

---

## Summary

DOLI is a **production-ready** Proof of Time blockchain featuring:

- **VDF-Based Consensus**: Time as scarce resource, not energy
- **Deterministic Selection**: Fair, unpredictable, ungameable
- **Bond Stacking**: Up to 100 bonds per producer, 4-year vesting with FIFO withdrawal
- **Weight-Based Fork Choice**: Seniority rewards long-term commitment (1-4x multiplier)
- **100% Burn Economics**: All penalties burned (deflationary)
- **Maintainer Bootstrap**: Decentralized from day 1 (first 5 registrations)
- **Auto-Update Governance**: 3/5 multisig with 40% veto threshold
- **Comprehensive Validation**: 28+ error types, multi-layer verification

The codebase is well-tested, documented, and follows strict conventions. All changes must comply with the WHITEPAPER.md as the ultimate source of truth.
