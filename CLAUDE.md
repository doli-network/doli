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
nix --extra-experimental-features "nix-command flakes" develop --command bash -c "cargo run -p doli-cli -- new"
nix --extra-experimental-features "nix-command flakes" develop --command bash -c "cargo run -p doli-cli -- balance"
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

| Crate | Purpose | Key Files | Lines |
|-------|---------|-----------|-------|
| `crypto` | BLAKE3-256 hashing, Ed25519 signatures, merkle trees, domain separation | `hash.rs`, `keys.rs`, `merkle.rs`, `signature.rs` | ~2,400 |
| `vdf` | Wesolowski VDF over class groups (registration), hash-chain VDF (blocks). Uses `rug` (GMP bindings) | `vdf.rs`, `class_group.rs`, `proof.rs` | ~1,200 |
| `core` | Types, validation, consensus parameters, deterministic scheduler, producer discovery | `consensus.rs`, `scheduler.rs`, `validation.rs`, `heartbeat.rs`, `maintainer.rs`, `discovery/` | ~23,000 |
| `storage` | RocksDB persistence for blocks, UTXO, chain state, producer registry | `block_store.rs`, `utxo.rs`, `producer.rs` | ~4,500 |
| `network` | libp2p P2P layer: gossipsub, Kademlia DHT, sync, equivocation detection | `service.rs`, `sync/`, `gossip.rs` | ~5,900 |
| `mempool` | Transaction pool with fee policies, double-spend detection | `pool.rs`, `entry.rs`, `policy.rs` | ~760 |
| `rpc` | JSON-RPC server (Axum) for wallet/explorer interaction | `methods.rs`, `server.rs`, `types.rs` | ~1,700 |
| `updater` | Auto-update with 3/5 multisig, 7-day veto period, 40% threshold | `lib.rs`, `vote.rs`, `apply.rs`, `download.rs` | ~1,500 |

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
   ├─ Add block reward coinbase (100% to producer)
   ├─ Select transactions from mempool (max 1MB, highest fees first)
   └─ Build header with merkle root

3. Compute VDF
   ├─ VDF input: HASH("DOLI_VDF_BLOCK_V1" || prev_hash || merkle_root || slot || producer_key)
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
- Primary: `slot % total_bonds` → find ticket owner (O(log n) binary search)
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
| 0-3s | rank 0 only (PRIMARY_WINDOW_MS = 3000) |
| 3-6s | rank 0-1 (SECONDARY_WINDOW_MS = 6000) |
| 6-10s | rank 0-2 (TERTIARY_WINDOW_MS = 10000) |

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
| **Blocks per Reward Epoch** | 360 | 4 |
| **Blocks per Era** | 12,614,400 (~4 years) | 576 (~96 min accelerated) |
| **Halving Interval** | 12,614,400 blocks | 576 blocks |
| **Slots per Year** | 3,153,600 | 144 (accelerated) |

---

## Network Configuration

| Network | ID | Magic Bytes | P2P Port | RPC Port | Address Prefix | Genesis Time |
|---------|----|-------------|----------|----------|----------------|--------------|
| Mainnet | 1 | `D0 11 00 01` | 30303 | 8545 | `doli` | 2026-02-01T00:00:00Z |
| Testnet | 2 | `D0 11 00 02` | 40303 | 18545 | `tdoli` | 2026-01-29T22:00:00Z |
| Devnet | 99 | `D0 11 00 63` | 50303 | 28545 | `ddoli` | Dynamic |

### Genesis Configuration

| Parameter | Mainnet | Testnet | Devnet |
|-----------|---------|---------|--------|
| Genesis Timestamp | 1769904000 | 1769738400 | Dynamic |
| Genesis Producers | 5 (placeholder) | 5 (real keys) | 0 (bootstrap) |
| Genesis Phase | N/A | N/A | 40 blocks |
| Initial Reward | 1 DOLI | 1 DOLI | 20 DOLI |

---

## Economic Parameters

### Supply and Rewards

| Parameter | Value | Base Units | Notes |
|-----------|-------|------------|-------|
| **Total Supply** | 25,228,800 DOLI | 2,522,880,000,000,000 | Fixed cap |
| **Initial Block Reward** | 1 DOLI | 100,000,000 | Halves every ~4 years |
| **Reward Distribution** | 100% to block producer | - | Via coinbase transaction |
| **Coinbase Maturity** | 100 blocks (mainnet/testnet) | - | 10 blocks (devnet) |
| **Decimals** | 8 | - | 1 DOLI = 10^8 base units |

### Emission Schedule

| Era | Years | Reward | Cumulative | % of Total |
|-----|-------|--------|------------|------------|
| 0 | 0-4 | 1.0 DOLI | 12,614,400 | 50.00% |
| 1 | 4-8 | 0.5 DOLI | 18,921,600 | 75.00% |
| 2 | 8-12 | 0.25 DOLI | 22,075,200 | 87.50% |
| 3 | 12-16 | 0.125 DOLI | 23,652,000 | 93.75% |
| 4 | 16-20 | 0.0625 DOLI | 24,440,400 | 96.88% |
| 5 | 20-24 | 0.03125 DOLI | 24,834,600 | 98.44% |

**Formula**: `block_reward(height) = INITIAL_REWARD >> era` (right-shift = halving)

### Bond System

| Parameter | Mainnet/Testnet | Devnet |
|-----------|-----------------|--------|
| **Bond Unit** | 100 DOLI (10,000,000,000) | 1 DOLI (100,000,000) |
| **Max Bonds/Producer** | 100 | 100 |
| **Max Stake/Producer** | 10,000 DOLI | 100 DOLI |
| **Commitment Period** | 4 years (12,614,400 blocks) | ~96 min (576 blocks) |
| **Withdrawal Delay** | 7 days (60,480 slots) | ~10 min (60 slots) |

### Bond Stacking Architecture

**Core Structures** (`consensus.rs`):
```rust
pub struct BondEntry {
    pub creation_slot: Slot,     // When this bond was created
    pub amount: Amount,          // Always BOND_UNIT (100 DOLI mainnet)
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

**Implementation** (`consensus.rs`):
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

Base fee: 0.001 DOLI (100,000 base units), scaling with congestion:
| Pending Registrations | Fee Multiplier | Fee |
|----------------------|----------------|-----|
| 0-4 | 1.00x | 0.001 DOLI |
| 5-9 | 1.50x | 0.0015 DOLI |
| 10-19 | 2.00x | 0.002 DOLI |
| 20-49 | 3.00x | 0.003 DOLI |
| 50-99 | 4.50x | 0.0045 DOLI |
| 100-199 | 6.50x | 0.0065 DOLI |
| 200-299 | 8.50x | 0.0085 DOLI |
| 300+ | 10.00x (cap) | 0.01 DOLI |

---

## VDF System (Dual Architecture)

DOLI uses **two VDF types** for different purposes:

### 1. Wesolowski VDF (Registration Anti-Sybil)

| Parameter | Mainnet/Testnet | Devnet |
|-----------|-----------------|--------|
| **Base Iterations** | 600,000,000 (~10 min) | 5,000,000 (~5s) |
| **Cap** | 86,400,000,000 (~24 hours) | - |
| **Discriminant Bits** | 2048 | 256 |
| **Proof Type** | Compact (~512 bytes) | Same |
| **Verification** | O(log t) group ops | Same |

**Scaling Formula**: `t = min(T_REGISTER_CAP, T_REGISTER_BASE * (1 + registered_count / 100))`

### 2. Hash-Chain VDF (Block Production)

| Parameter | All Networks |
|-----------|-------------|
| **Iterations** | 10,000,000 (T_BLOCK) |
| **Target Time** | ~700ms |
| **Algorithm** | Sequential BLAKE3 hashing |
| **Verification** | Recompute (linear, but fast) |

**Anti-Grinding**: VDF input includes `prev_hash` (unknown until previous block), preventing pre-computation.

### VDF Input Construction

```rust
// Block VDF input
input = BLAKE3("DOLI_VDF_BLOCK_V1" || prev_hash || merkle_root || slot || producer_key)

// Heartbeat VDF input
input = BLAKE3("DOLI_HEARTBEAT_V1" || producer || slot || prev_hash)

// Registration VDF input
input = BLAKE3("DOLI_VDF_REGISTER_V1" || producer_pubkey || epoch)
```

### Class Group Arithmetic

- **Discriminant**: Imaginary quadratic with Δ ≡ 1 (mod 4)
- **Group Elements**: Binary quadratic forms (a, b, c) where Δ = b² - 4ac
- **Group Operation**: Shanks' composition algorithm
- **Reduction**: Ensures canonical representative (|b| ≤ a ≤ c)

---

## Transaction Types

| Type | ID | Purpose | Inputs | Outputs | Extra Data |
|------|----|---------|--------|---------|------------|
| Transfer | 0 | Standard value transfer | Yes | Yes | - |
| Registration | 1 | Producer registration | Yes | 1 Bond | RegistrationData (bincode) |
| Exit | 2 | Start unbonding | No | No | ExitData (bincode) |
| ClaimReward | 3 | **DEPRECATED** | No | 1 | ClaimData (bincode) |
| ClaimBond | 4 | Claim after unbonding | No | 1 | ClaimBondData (bincode) |
| SlashProducer | 5 | Slash for double-production | No | No | SlashData (bincode) |
| Coinbase | 6 | Block reward | No | 1+ | Height (u64 LE) |
| AddBond | 7 | Increase stake | Yes | No | AddBondData (36 bytes) |
| RequestWithdrawal | 8 | Start 7-day withdrawal | No | No | WithdrawalRequestData (68 bytes) |
| ClaimWithdrawal | 9 | Complete withdrawal | No | 1 | ClaimWithdrawalData (36 bytes) |
| EpochReward | 10 | Epoch reward distribution | No | 1+ | EpochRewardData (40 bytes) |
| RemoveMaintainer | 11 | 3/5 multisig remove | No | No | MaintainerChangeData (bincode) |
| AddMaintainer | 12 | 3/5 multisig add | No | No | MaintainerChangeData (bincode) |

### Transaction Hash Computation (Malleability Prevention)

**Signature is EXCLUDED from transaction hash** to prevent third-party malleability:

```rust
hash = BLAKE3(
    version (4 bytes LE) ||
    tx_type (4 bytes LE) ||
    input_count (4 bytes LE) ||
    for each input: prev_tx_hash (32) || output_index (4 LE) ||  // NO SIGNATURE
    output_count (4 bytes LE) ||
    for each output: serialize(output) ||
    extra_data_len (4 bytes LE) ||
    extra_data
)
```

### Transaction Lifecycle: Bond Withdrawal

**Three-Transaction Process:**

1. **RequestWithdrawal** (`TxType::RequestWithdrawal = 8`)
   - Initiates withdrawal, calculates penalties using FIFO
   - Bonds immediately removed from active set
   - Selection weight decreases immediately
   - Penalties calculated and locked for burning

2. **7-day delay** (60,480 slots on mainnet)
   - Funds in pending state
   - Prevents flash attacks

3. **ClaimWithdrawal** (`TxType::ClaimWithdrawal = 9`)
   - Retrieves net amount after delay
   - Penalties burned at this point

---

## Block Structure

```rust
pub struct BlockHeader {
    pub version: u32,           // Protocol version (currently 1)
    pub prev_hash: Hash,        // Previous block hash (32 bytes)
    pub merkle_root: Hash,      // Transaction merkle root (32 bytes)
    pub presence_root: Hash,    // Always Hash::ZERO (legacy field)
    pub timestamp: u64,         // Unix timestamp (8 bytes)
    pub slot: Slot,             // Slot number (4 bytes)
    pub producer: PublicKey,    // Block producer (32 bytes)
    pub vdf_output: VdfOutput,  // Hash-chain result (~32 bytes)
    pub vdf_proof: VdfProof,    // Empty for hash-chain VDF
}

pub struct Block {
    pub header: BlockHeader,
    pub transactions: Vec<Transaction>,  // First tx = coinbase
}
```

**Block Hash**: Computed via BLAKE3 over all header fields including VDF output.

**Merkle Root**: Binary merkle tree with domain-separated leaf (0x00) and internal (0x01) prefixes.

---

## Validation Rules

### Block Validation Flow (`validation.rs`)

Block validation performs 7 checks:
1. **Header Validation** (lines 480-553): Version, timestamp progression, slot derivation
2. **Block Size** (lines 561-568): Max 1MB (base), doubles every era up to 32MB cap
3. **Merkle Root** (lines 571-573): Must match computed root
4. **Transaction Validation** (lines 579-581): Signatures, UTXO existence, amounts
5. **Internal Double-Spend** (lines 583-584): No duplicate inputs within block
6. **VDF Validation** (lines 1926-1962): Recompute hash-chain and compare
7. **Producer Eligibility** (lines 1971-2004): Deterministic scheduler verification

### Header Validation Rules

| Rule | Error |
|------|-------|
| Version must be 1 | `InvalidVersion(u32)` |
| Timestamp must advance | `InvalidTimestamp { block, expected }` |
| Timestamp not too far in future (MAX_DRIFT = 10s) | `TimestampTooFuture(u64)` |
| Slot derives correctly from timestamp | `InvalidSlot { got, expected }` |
| Slot must advance | `SlotNotAdvancing { got, prev }` |
| Timestamp within slot window | `InvalidTimestamp { block, expected }` |
| Slot not too far in future (MAX_FUTURE_SLOTS = 1) | `SlotTooFuture { got, current, max_future }` |
| Slot not too far in past (MAX_PAST_SLOTS = 192) | `SlotTooPast { got, current, max_past }` |

### Validation Error Types (37 variants)

**Block/Header Errors:**
- `InvalidVersion(u32)`, `InvalidTimestamp { block, expected }`
- `TimestampTooFuture(u64)`, `InvalidSlot { got, expected }`
- `SlotNotAdvancing { got, prev }`, `SlotTooFuture/TooPast`
- `InvalidMerkleRoot`, `InvalidVdfProof`, `InvalidProducer`
- `BlockTooLarge { size, max }`, `MissingCoinbase`, `InvalidCoinbase(String)`
- `InvalidBlock(String)`, `UnexpectedEpochReward`
- `MissingEpochReward { epoch }`, `EpochRewardMismatch { reason }`

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
- `InvalidEpochReward(String)`, `InvalidMaintainerChange(String)`

---

## Network Layer (libp2p)

### Protocol Stack

```
TCP → Noise Encryption (Ed25519) → Yamux Multiplexing
```

### Gossipsub Topics (5 topics)

| Topic | Purpose | Max Size |
|-------|---------|----------|
| `/doli/blocks/1` | Block propagation | 1 MB |
| `/doli/txs/1` | Transaction propagation | - |
| `/doli/producers/1` | Producer announcements | - |
| `/doli/votes/1` | Governance votes | - |
| `/doli/heartbeats/1` | Presence heartbeats (deprecated) | - |

### Gossipsub Configuration

- Heartbeat interval: 1 second
- Validation mode: Strict
- Mesh peers: 4-12 (target 6)
- Gossip factor: 0.25
- History length: 5 messages
- Max transmit size: 1 MB
- Duplicate cache time: 60 seconds

### Request-Response Protocols

| Protocol | Max Size | Timeout | Purpose |
|----------|----------|---------|---------|
| `/doli/status/1.0.0` | 64 KB | 30s | Peer handshake and chain state |
| `/doli/sync/1.0.0` | 16 MB | 120s | Block synchronization |

### Kademlia DHT

- Protocol: `/doli/kad/1.0.0`
- Replication factor: 20
- Query timeout: 60 seconds

### Sync Protocol

**Header-First Strategy**:
1. Download headers (up to 2000/request)
2. Validate VDF chain linkage
3. Parallel body download (up to 8 peers, 128 blocks/request)
4. Apply blocks in order

**Sync Submodules** (`network/src/sync/`):

| File | LOC | Purpose | Key Items |
|------|-----|---------|-----------|
| `manager.rs` | 744 | Sync orchestration | `SyncManager`, 5 `SyncState` variants, weight-based fork choice |
| `reorg.rs` | 595 | Chain reorganization | `ReorgHandler`, `BlockWeight`, MAX_REORG_DEPTH = 100 |
| `equivocation.rs` | 359 | Double-production detection | `EquivocationDetector`, `EquivocationProof` |
| `bodies.rs` | 340 | Parallel body downloader | `BodyDownloader`, round-robin peer selection |
| `headers.rs` | 221 | Header-first downloader | `HeaderDownloader`, chain linkage validation |
| `state.rs` | 19 | Module exports | - |

**Five SyncState Variants**:
- `Idle` - waiting for peers
- `DownloadingHeaders { target_slot, peer, headers_count }` - header phase
- `DownloadingBodies { pending, total }` - parallel body phase
- `Processing { height }` - applying downloaded blocks
- `Synchronized` - fully synced

**Sync Configuration**:
- `max_headers_per_request`: 2000
- `max_bodies_per_request`: 128
- `max_concurrent_body_requests`: 8
- `request_timeout`: 30 seconds
- `stale_timeout`: 300 seconds

### Equivocation Detection (`sync/equivocation.rs`)

**Only slashable offense**: Double-production (same slot, different blocks)

**Detection Flow**:
1. Track `(producer, slot)` pairs with full `BlockHeader` in LRU cache
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
| Timeout | -5 × count (max -50) |
| Spam | -50 |
| Malformed Message | -30 |
| Duplicate | -5 |

**Thresholds**: Disconnect at -200, ban at -500 (1 hour)

### Rate Limiting (Token Bucket)

| Resource | Per-Peer Limit | Global Limit |
|----------|----------------|--------------|
| Blocks | 10/min | 100/min |
| Transactions | 50/sec | 200/sec |
| Bandwidth | 1 MB/sec | 10 MB/sec |

---

## Storage Layer (RocksDB)

### Column Families (5 total)

| CF | Key Format | Value Format |
|----|------------|--------------|
| `headers` | Block hash (32 bytes) | Bincode(BlockHeader) |
| `bodies` | Block hash (32 bytes) | Bincode(Vec<Transaction>) |
| `height_index` | Height (8 bytes LE) | Block hash (32 bytes raw) |
| `slot_index` | Slot (4 bytes LE) | Block hash (32 bytes raw) |
| `presence` | Hash | PresenceCommitment (legacy) |

### UTXO Model

- **HashMap-based** in-memory UTXO set
- **Outpoint Key**: tx_hash (32 bytes) || output_index (4 bytes LE) = 36 bytes
- **Output Value**: (output_type, amount, pubkey_hash, lock_until)
- **Serialization**: Bincode

### Producer Registry (`storage/producer.rs`)

```rust
pub struct ProducerInfo {
    pub public_key: PublicKey,
    pub registered_at: u64,         // Registration slot
    pub bond_amount: u64,           // Total locked value
    pub bond_outpoint: (Hash, u32), // Primary bond UTXO
    pub status: ProducerStatus,
    pub bond_count: u32,            // Number of bonds (1-100)
    pub additional_bonds: Vec<(Hash, u32)>,  // Secondary bond UTXOs
}

pub enum ProducerStatus {
    Active,
    Unbonding { started_at: u64 },
    Exited,
    Slashed { slashed_at: u64 },
}
```

### Chain State

```rust
pub struct ChainState {
    pub best_hash: Hash,
    pub best_height: u64,
    pub best_slot: u32,
    pub total_work: u64,              // Legacy
    pub genesis_hash: Hash,
    pub genesis_timestamp: u64,
    pub last_registration_hash: Hash, // Anti-Sybil chain
    pub registration_sequence: u64,   // Monotonic counter
    pub total_minted: Amount,         // Supply tracking
}
```

---

## Mempool

### Configuration

| Parameter | Mainnet | Testnet | Devnet |
|-----------|---------|---------|--------|
| Max transactions | 5,000 | 10,000 | 1,000 |
| Max size | 10 MB | 10 MB | 1 MB |
| Min fee rate | 1 sat/byte | 0 | 0 |
| Max tx size | 100 KB | 100 KB | 100 KB |
| Max ancestors | 25 | 25 | 25 |
| Expiry | 14 days | 14 days | 14 days |

### Double-Spend Detection

- Maintains `spent_outputs: HashMap<Outpoint, TxHash>`
- For each input, checks if already claimed by different tx
- First-come-first-served: first transaction to spend wins

### Eviction Policy

- Evicts transactions with **lowest fee rate** and **no descendants**
- Uses `BTreeSet<(fee_rate, hash)>` for O(log n) selection
- Dynamic fee threshold at 90% capacity

---

## RPC Server (JSON-RPC)

### Available Methods (17+ total)

| Category | Method | Description |
|----------|--------|-------------|
| **Chain** | `getBlockByHash` | Get block by hash |
| | `getBlockByHeight` | Get block by height |
| | `getChainInfo` | Chain height, tip hash, sync status |
| | `getBlockHeader` | Header only, no transactions |
| **Transaction** | `sendTransaction` | Submit signed transaction (hex-encoded bincode) |
| | `getTransaction` | Get transaction by hash (mempool only) |
| | `getHistory` | Transaction history for address |
| **UTXO** | `getBalance` | Get balance (confirmed, unconfirmed, immature, total) |
| | `getUtxos` | Get UTXOs for address |
| **Producer** | `getProducer` | Producer status, bonds |
| | `getProducers` | All producers (active_only filter) |
| **Network** | `getNetworkInfo` | Peer count, sync status |
| | `getNodeInfo` | Node version, network, peer ID |
| **Mempool** | `getMempoolInfo` | Size, tx count, fee rates |
| **Epoch** | `getEpochInfo` | Current epoch, blocks remaining, reward rate |
| **Governance** | `getMaintainerSet` | Current maintainer set |
| | `submitMaintainerChange` | Submit 3/5 multisig maintainer change |
| | `submitVote` | Submit governance veto vote |
| | `getUpdateStatus` | Auto-update status |

### Error Codes

| Code | Name | Description |
|------|------|-------------|
| -32700 | Parse Error | Invalid JSON |
| -32600 | Invalid Request | Not valid Request object |
| -32601 | Method Not Found | Method doesn't exist |
| -32602 | Invalid Params | Wrong parameters |
| -32603 | Internal Error | Server error |
| -32000 | Block Not Found | Block missing |
| -32001 | TX Not Found | Transaction missing |
| -32002 | Invalid TX | Validation failure |
| -32003 | TX Already Known | Duplicate in mempool |
| -32004 | Mempool Full | Capacity exceeded |
| -32005 | UTXO Not Found | UTXO missing |
| -32006 | Producer Not Found | Producer not registered |

---

## CLI Wallet (doli-cli)

### Commands

| Command | Purpose | Network Required |
|---------|---------|------------------|
| `new` | Create new wallet | No |
| `address` | Generate new address | No |
| `addresses` | List all addresses | No |
| `balance` | Query balance | Yes |
| `send <TO> <AMOUNT>` | Send coins | Yes |
| `history` | Transaction history | Yes |
| `export <OUTPUT>` | Export wallet | No |
| `import <INPUT>` | Import wallet | No |
| `info` | Wallet details | No |
| `sign <MESSAGE>` | Sign message | No |
| `verify <MSG> <SIG> <PUBKEY>` | Verify signature | No |
| `chain` | Chain info | Yes |

### Producer Subcommands

| Command | Purpose |
|---------|---------|
| `producer register [--bonds N]` | Register as producer (1-100 bonds) |
| `producer status [--pubkey KEY]` | Check producer status |
| `producer list [--active]` | List all producers |
| `producer add-bond --count N` | Increase stake |
| `producer request-withdrawal --count N` | Start 7-day withdrawal |
| `producer claim-withdrawal [--index N]` | Claim after delay |
| `producer exit [--force]` | Exit producer set |
| `producer slash --block1 H --block2 H` | Submit slashing evidence |

---

## Update System

### Governance Parameters

| Parameter | Mainnet/Testnet | Devnet |
|-----------|-----------------|--------|
| **Veto Period** | 7 days (604,800s) | 60 seconds |
| **Grace Period** | 48 hours (172,800s) | 30 seconds |
| **Veto Threshold** | 40% of weighted power | Same |
| **Required Signatures** | 3/5 maintainers | Same |
| **Min Voting Age** | 30 days (2,592,000s) | 60 seconds |

### Maintainer System (`core/maintainer.rs`)

- **Initial Set**: First 5 registered producers become maintainers automatically
- **Threshold**: 3/5 multisig for all changes
- **Slashing Integration**: Producers removed from maintainer set automatically when slashed
- **Min Maintainers**: 3 (cannot remove below this)
- **Max Maintainers**: 5

---

## Producer Discovery System (`core/discovery/`)

Cryptographically signed producer discovery using CRDT (Conflict-free Replicated Data Type).

### Discovery Module (~3,200 lines)

| File | LOC | Purpose | Key Items |
|------|-----|---------|-----------|
| `mod.rs` | 171 | Module root, error types | `ProducerSetError` (6 variants), `MergeResult` |
| `gset.rs` | 1,434 | Grow-Only Set CRDT | `ProducerGSet`, version vectors, disk persistence |
| `announcement.rs` | 376 | Signed announcements | `ProducerAnnouncement`, replay protection |
| `bloom.rs` | 353 | Probabilistic delta sync | `ProducerBloomFilter`, ~1% false positive rate |
| `gossip.rs` | 442 | Adaptive gossip controller | `AdaptiveGossip`, network-aware backoff |
| `proto.rs` | 428 | Serialization | Encode/decode functions, bincode format |

### Architecture Flow

```
ProducerAnnouncement (signed) → ProducerGSet (CRDT) → Persistence
                                      ↓
                              ProducerBloomFilter (delta sync)
                                      ↓
                              AdaptiveGossip (interval control)
```

### Discovery Error Types (6 variants)

| Error | Description |
|-------|-------------|
| `InvalidSignature` | Signature verification failed |
| `StaleAnnouncement` | Timestamp > 1 hour old |
| `FutureTimestamp` | Timestamp > 5 minutes in future |
| `NetworkMismatch { expected, got }` | Cross-network protection |
| `SequenceRegression { current, received }` | Replay protection |
| `SerializationError` | Bincode encode/decode failure |

### Seniority-Weighted Voting

Vote weight increases with producer age:
| Age | Weight |
|-----|--------|
| 0-1 year | 1.00x |
| 1-2 years | 1.75x |
| 2-3 years | 2.50x |
| 3-4 years | 3.25x |
| 4+ years | 4.00x (capped) |

**Formula**: `weight = 1.0 + min(years, 4) * 0.75`

---

## Key Constants Reference

### Consensus (`consensus.rs`)

```rust
PROTOCOL_VERSION = 1
SLOT_DURATION = 10                      // seconds
SLOTS_PER_EPOCH = 360                   // 1 hour
SLOTS_PER_REWARD_EPOCH = 360            // 1 hour
BLOCKS_PER_ERA = 12_614_400             // ~4 years
MAX_FALLBACK_PRODUCERS = 3              // primary + 2 fallbacks
PRIMARY_WINDOW_MS = 3_000               // 0-3s: rank 0 only
SECONDARY_WINDOW_MS = 6_000             // 0-6s: rank 0-1
TERTIARY_WINDOW_MS = 10_000             // 0-10s: rank 0-2
MAX_DRIFT = 10                          // Clock drift tolerance (seconds)
MAX_FUTURE_SLOTS = 1                    // Maximum slots in future
MAX_PAST_SLOTS = 192                    // Maximum slots in past (~32 min)
BOOTSTRAP_BLOCKS = 60_480               // ~1 week
MAX_FAILURES = 50                       // Consecutive misses before inactive
```

### Economics (`consensus.rs`)

```rust
TOTAL_SUPPLY = 2_522_880_000_000_000    // 25,228,800 DOLI in base units
INITIAL_REWARD = 100_000_000            // 1 DOLI
EPOCH_REWARD_POOL = 36_000_000_000      // 360 × 1 DOLI
BOND_UNIT = 10_000_000_000              // 100 DOLI (mainnet/testnet)
MAX_BONDS_PER_PRODUCER = 100            // 10,000 DOLI max
WITHDRAWAL_DELAY_SLOTS = 60_480         // 7 days
COINBASE_MATURITY = 100                 // blocks
YEAR_IN_SLOTS = 3_153_600               // 365 days
COMMITMENT_PERIOD = 12_614_400          // 4 years (YEAR_IN_SLOTS × 4)
BASE_REGISTRATION_FEE = 100_000         // 0.001 DOLI
MAX_FEE_MULTIPLIER_X100 = 1000          // 10.00x cap
```

### VDF (`vdf/lib.rs`, `consensus.rs`)

```rust
T_BLOCK = 10_000_000                    // ~700ms hash-chain
T_REGISTER_BASE = 600_000_000           // ~10 min Wesolowski
T_REGISTER_CAP = 86_400_000_000         // ~24 hours max
DISCRIMINANT_BITS = 2048                // Class group security (mainnet)
HEARTBEAT_VDF_ITERATIONS = 10_000_000   // Same as T_BLOCK
VDF_TARGET_MS = 700                     // Target duration
```

### Cryptography (`crypto/`)

```rust
HASH_SIZE = 32                          // BLAKE3-256
PUBLIC_KEY_SIZE = 32                    // Ed25519
PRIVATE_KEY_SIZE = 32                   // Ed25519 seed
SIGNATURE_SIZE = 64                     // Ed25519
ADDRESS_SIZE = 20                       // Truncated hash
DECIMALS = 8                            // Currency precision
UNITS_PER_COIN = 100_000_000            // 10^8
```

### Domain Separation Tags

```rust
SIGN_DOMAIN = b"DOLI_SIGN_V1"
ADDRESS_DOMAIN = b"DOLI_ADDR_V1"
TX_DOMAIN = b"DOLI_TX_V1"
BLOCK_DOMAIN = b"DOLI_BLOCK_V1"
VDF_DOMAIN = b"DOLI_VDF_V1"
HEARTBEAT_DOMAIN = b"DOLI_HEARTBEAT_V1"
HEARTBEAT_SIGN_DOMAIN = b"DOLI_HEARTBEAT_SIGN_V1"
HEARTBEAT_WITNESS_DOMAIN = b"DOLI_HEARTBEAT_WITNESS_V1"
VDF_BLOCK_DOMAIN = b"DOLI_VDF_BLOCK_V1"
VDF_REGISTER_DOMAIN = b"DOLI_VDF_REGISTER_V1"
VDF_CHALLENGE_DOMAIN = b"DOLI_VDF_CHALLENGE_V1"
DISCRIMINANT_EXPANSION_DOMAIN = b"DOLI_DISCRIMINANT_EXPANSION_V1"
GENESIS_VDF_DOMAIN = b"DOLI_GENESIS_VDF"
GENESIS_RECIPIENT_DOMAIN = b"DOLI_GENESIS_RECIPIENT"
BURN_ADDRESS_DOMAIN = b"DOLI_BURN_ADDRESS_V1"
```

### Network (`network.rs`)

```rust
MAX_TRACKED_SLOTS = 1000                // Equivocation LRU cache
MIN_WITNESS_SIGNATURES = 2              // Heartbeat witnesses
HEARTBEAT_VERSION = 1                   // Protocol version
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
│  [ ] 4. If documentation files added/removed/renamed:           │
│         - docs/DOCS.md index updated                            │
│         - specs/SPECS.md index updated (if specs changed)       │
│                                                                 │
│  [ ] 5. If architecture/consensus/constants/crates change:      │
│         - CLAUDE.md updated (project brain must stay current)   │
│                                                                 │
│  [ ] 6. State which docs were checked/updated in commit message │
└─────────────────────────────────────────────────────────────────┘
```

**If you skip this gate, you are violating CLAUDE.md.**

### Milestone Workflow

When working on implementation milestones:

1. **Implement** - Write the code/tests for the milestone
2. **Verify** - Run `cargo build && cargo clippy && cargo test` (all must pass)
3. **Document** - Complete Pre-Commit Gate checklist above
4. **Commit** - Using conventions (conventional commits, `--author`, HEREDOC format)
5. **Push** - Push to remote
6. **Stop** - Report completion, wait for next milestone

A milestone is not complete until pushed. Do not stop between steps.

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
2. **Chained Hashes**: Each registration references previous (`last_registration_hash`)
3. **Sequence Numbers**: Monotonic (`registration_sequence`), prevents replay

### Anti-Grinding Protection

1. **VDF Input Unpredictability**: Includes `prev_hash` (unknown until previous block)
2. **Deterministic Selection**: No influence on future selection via block content
3. **~700ms VDF**: Only ~14 attempts possible per 10s slot

### Double-Production Prevention (Three Layers)

1. **Lock File** (`producer/guard.rs`): Prevents concurrent producer instances
2. **Signed Slots DB** (`producer/signed_slots.rs`): Tracks locally signed slots, prevents restart-time re-signing
3. **Network Detection** (`sync/equivocation.rs`): Catches violations from malicious actors

**Result**: 100% slashing (complete bond burn, permanent exclusion)

### Transaction Malleability Prevention

- Signature is **excluded** from transaction hash
- Hash covers: version, type, inputs (minus sigs), outputs, extra_data
- Prevents third-party modification of signed transactions

---

## Testing Infrastructure

### Unit Tests

Each crate has comprehensive unit tests (run with `cargo test -p <crate>`).

### Integration Tests (12 tests)

Located in `testing/integration/`:

| Test File | Purpose |
|-----------|---------|
| `two_node_sync.rs` | Header-first sync between two nodes |
| `partition_heal.rs` | Network partition recovery |
| `attack_reorg_test.rs` | Reorg attack resilience |
| `two_producer_pop.rs` | Two-node consensus (basic PoT) |
| `staggered_validator_rewards.rs` | Producer joining/epoch rewards |
| `epoch_rewards.rs` | Epoch reward distribution consistency |
| `bond_stacking.rs` | Bond lifecycle, vesting, withdrawal penalties |
| `malicious_peer.rs` | Malicious peer detection/handling |
| `presence_manipulation_test.rs` | Presence gossip integrity |
| `reorg_test.rs` | Chain reorganization handling |
| `mempool_stress.rs` | Mempool under load |
| `equivocation_slashing.rs` | Double-production detection and slashing |

### Simulation Tests

Located in `testing/simulation/`:
- `existential_risks.rs` (~1,800 lines) - Validates DOLI's core premise

**Three Critical Simulations**:
1. **Onboarding stress test** - Liveness under viral growth (queue management)
2. **Elite simulation** - Power concentration over time (Gini coefficient tracking)
3. **Slow infiltration attack** - Economic security analysis

**Alert Thresholds**:
- Queue wait critical: 60 blocks
- Fee multiplier critical: 100.0x
- Abandonment rate critical: 50%
- Gini healthy: <0.3, concerning: >0.5

### Fuzz Tests (6 targets)

```
fuzz_block_deserialize    fuzz_merkle
fuzz_tx_deserialize       fuzz_signature
fuzz_vdf_verify           fuzz_hash
```

### Test Scripts (23 scripts)

See `scripts/README.md` for complete registry. Key scripts:
| Script | Nodes | Purpose |
|--------|-------|---------|
| `launch_testnet.sh` | 2 | Basic devnet |
| `test_3node_proportional_rewards.sh` | 3 | Reward distribution |
| `test_5node_epoch_rewards_consistency.sh` | 5 | Deterministic rewards |
| `test_whitepaper_full.sh` | 3 | WHITEPAPER verification |
| `test_critical_features.sh` | 3 | E2E validation |
| `test_governance_scenarios.sh` | 5 | Veto threshold testing |
| `test_maintainer_bootstrap.sh` | 10 | Maintainer bootstrap |

### Benchmarks

VDF benchmarks in `testing/benchmarks/`:
- `compute` - Block VDF computation (~700ms)
- `verify` - VDF verification (<1s)
- `full` - Complete suite with system characterization
- Results in `results/hardware_matrix.md`

---

## File Reference Index

### Core Crate (~23,000 lines)

| File | Purpose | Key Items |
|------|---------|-----------|
| `consensus.rs` | Consensus parameters, bond structures | Constants, BondEntry, ProducerBonds, withdrawal_penalty_rate() |
| `scheduler.rs` | Deterministic round-robin scheduler | DeterministicScheduler, select_producer(), ticket_boundaries |
| `validation.rs` | Block and transaction validation | 37 error types, validate_block(), validate_transaction() |
| `block.rs` | Block and header structures | BlockHeader, Block, BlockBuilder, compute_merkle_root() |
| `transaction.rs` | Transaction types and structures | 13 TxType variants, Input, Output, signing_message() |
| `types.rs` | Core type aliases | Amount, BlockHeight, Slot, Epoch, Era, DisplayAmount |
| `network.rs` | Network configuration | Network enum, magic bytes, ports, genesis times |
| `genesis.rs` | Genesis block generation | GenesisConfig, generate_genesis_block(), verify_genesis_block() |
| `maintainer.rs` | Maintainer bootstrap system | MaintainerSet, 3/5 multisig, derive_maintainer_set() |
| `heartbeat.rs` | Heartbeat VDF validation | Heartbeat, WitnessSignature, compute_vdf(), verify_full() |
| `discovery/mod.rs` | Producer discovery module | ProducerSetError (6 variants), MergeResult |
| `discovery/gset.rs` | Grow-Only Set CRDT | ProducerGSet, version vectors, disk persistence |
| `discovery/announcement.rs` | Signed announcements | ProducerAnnouncement, replay protection |
| `discovery/bloom.rs` | Delta sync optimization | ProducerBloomFilter, ~1% false positive rate |
| `discovery/gossip.rs` | Adaptive gossip | AdaptiveGossip, network-aware backoff |
| `discovery/proto.rs` | Serialization | Encode/decode, bincode format |

### Crypto Crate (~2,400 lines)

| File | Purpose | Key Items |
|------|---------|-----------|
| `hash.rs` | BLAKE3-256 hashing | Hash, Hasher, hash(), hash_with_domain(), hash_concat() |
| `keys.rs` | Ed25519 key management | PublicKey, PrivateKey, KeyPair, Address, burn() |
| `signature.rs` | Signature functions | Signature, sign(), verify(), sign_hash(), verify_hash() |
| `merkle.rs` | Merkle tree implementation | MerkleTree, MerkleProof, merkle_root(), transaction_root() |

### VDF Crate (~1,200 lines)

| File | Purpose | Key Items |
|------|---------|-----------|
| `lib.rs` | Public API, constants | T_BLOCK, T_REGISTER_BASE, compute(), verify() |
| `vdf.rs` | Wesolowski VDF | VdfOutput, VdfProof, compute_vdf(), verify_vdf() |
| `class_group.rs` | Class group arithmetic | ClassGroupElement, compose(), pow(), hash_to_group() |
| `proof.rs` | Proof structures | VdfProof serialization |

### Network Crate (~5,900 lines)

| File | Purpose | Key Items |
|------|---------|-----------|
| `service.rs` | Main network service | NetworkService, NetworkEvent, NetworkCommand |
| `gossip.rs` | Gossipsub configuration | 5 topics, mesh configuration |
| `sync/manager.rs` | Sync orchestration | SyncManager, 5 SyncState variants, weight-based fork choice |
| `sync/reorg.rs` | Chain reorganization | ReorgHandler, BlockWeight, MAX_REORG_DEPTH = 100 |
| `sync/equivocation.rs` | Double-production detection | EquivocationDetector, EquivocationProof |
| `sync/bodies.rs` | Parallel body downloader | BodyDownloader, round-robin peer selection |
| `sync/headers.rs` | Header-first downloader | HeaderDownloader, chain linkage validation |
| `scoring.rs` | Peer scoring system | 6 infraction types, disconnect/ban thresholds |
| `rate_limit.rs` | DoS protection | Token bucket algorithm, per-peer and global limits |

### Storage Crate (~4,500 lines)

| File | Purpose | Key Items |
|------|---------|-----------|
| `block_store.rs` | Block persistence | BlockStore, 5 column families |
| `utxo.rs` | UTXO set management | UtxoSet, UtxoEntry, Outpoint |
| `producer.rs` | Producer registry | ProducerSet, ProducerInfo, ProducerStatus |

### Mempool Crate (~760 lines)

| File | Purpose | Key Items |
|------|---------|-----------|
| `pool.rs` | Transaction pool | Mempool, double-spend detection, eviction |
| `entry.rs` | Transaction entry | MempoolEntry, ancestors, descendants, CPFP |
| `policy.rs` | Policy configuration | MempoolPolicy, mainnet/testnet/local presets |

### RPC Crate (~1,700 lines)

| File | Purpose | Key Items |
|------|---------|-----------|
| `server.rs` | Axum server setup | RpcServer, CORS, handle_rpc() |
| `methods.rs` | RPC methods | RpcContext, 17+ methods |
| `types.rs` | Request/response types | BlockResponse, BalanceResponse, etc. |
| `error.rs` | Error definitions | RpcError, 12 error codes |

### Updater Crate (~1,500 lines)

| File | Purpose | Key Items |
|------|---------|-----------|
| `lib.rs` | Main system | Release, VersionEnforcement, verify_release_signatures() |
| `vote.rs` | Voting system | Vote, VoteMessage, VoteTracker, seniority weighting |
| `apply.rs` | Application logic | apply_update(), backup_current(), rollback() |
| `download.rs` | Binary management | Download sources, hash verification |

### Binaries

| File | Purpose | Key Items |
|------|---------|-----------|
| `bins/node/src/main.rs` | Node entry point | CLI parsing, run_node() |
| `bins/node/src/node.rs` | Main node logic | Node struct, run(), try_produce_block(), apply_block() |
| `bins/node/src/producer/` | Producer safety | ProducerGuard, SignedSlotsDb, startup_checks() |
| `bins/cli/src/main.rs` | CLI wallet | 13 commands, producer subcommands |
| `bins/cli/src/wallet.rs` | Wallet management | Wallet, WalletAddress, key generation |

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
- **Triple Slashing Protection**: Lock file + signed slots DB + network detection
- **Comprehensive Validation**: 37 error types, multi-layer verification

The codebase is well-tested, documented, and follows strict conventions. All changes must comply with the WHITEPAPER.md as the ultimate source of truth.

---

## Quick Reference Tables

### Type Aliases

| Alias | Type | Purpose |
|-------|------|---------|
| `Amount` | `u64` | Currency in base units (10^8 per DOLI) |
| `BlockHeight` | `u64` | Block position in chain |
| `Slot` | `u32` | Time-based slot number |
| `Epoch` | `u32` | Epoch grouping (360 slots) |
| `Era` | `u32` | Long-term era (~4 years) |

### Output Types

| Type | ID | lock_until | Purpose |
|------|-----|------------|---------|
| Normal | 0 | 0 | Immediately spendable |
| Bond | 1 | >0 | Time-locked to height |

### Producer Seniority Weight

| Years Active | Weight | Multiplier |
|--------------|--------|------------|
| 0-1 | 1 | 1.00x |
| 1-2 | 2 | 2.00x |
| 2-3 | 3 | 3.00x |
| 3+ | 4 | 4.00x (cap) |
