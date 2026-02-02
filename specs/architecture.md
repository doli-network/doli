# DOLI Architecture

This document describes the high-level architecture of the DOLI network and its components.

## System Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│                           DOLI Network                               │
├─────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐             │
│  │   Node A    │◄──►│   Node B    │◄──►│   Node C    │             │
│  │  (Producer) │    │  (Full)     │    │  (Producer) │             │
│  └─────────────┘    └─────────────┘    └─────────────┘             │
│         ▲                  ▲                  ▲                     │
│         │                  │                  │                     │
│         ▼                  ▼                  ▼                     │
│  ┌─────────────────────────────────────────────────────────┐       │
│  │                    P2P Network Layer                     │       │
│  └─────────────────────────────────────────────────────────┘       │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
```

## Node Types

### Full Node
A complete participant that:
- Validates all blocks and transactions
- Maintains the full UTXO set
- Propagates blocks and transactions
- Provides RPC interface for wallets

### Producer Node
A full node that additionally:
- Holds a registered producer identity
- Computes VDF proofs for assigned slots
- Creates and broadcasts blocks
- Manages activation bond

### Light Client
A minimal client that:
- Downloads and verifies block headers only
- Uses Merkle proofs for transaction verification
- Relies on full nodes for UTXO queries
- Suitable for mobile devices

---

## Component Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         DOLI Node                                │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │                      RPC Interface                        │   │
│  │  (Wallet API, Block Explorer API, Node Management)       │   │
│  └──────────────────────────────────────────────────────────┘   │
│                              │                                   │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │                    Application Layer                      │   │
│  ├────────────────┬─────────────────┬───────────────────────┤   │
│  │   Mempool      │   Block Builder │   Producer Manager    │   │
│  └────────────────┴─────────────────┴───────────────────────┘   │
│                              │                                   │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │                    Consensus Layer                        │   │
│  ├────────────────┬─────────────────┬───────────────────────┤   │
│  │  Chain Manager │  Fork Choice    │   Slot Scheduler      │   │
│  └────────────────┴─────────────────┴───────────────────────┘   │
│                              │                                   │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │                    Validation Layer                       │   │
│  ├────────────────┬─────────────────┬───────────────────────┤   │
│  │  TX Validator  │  Block Validator│   VDF Verifier        │   │
│  └────────────────┴─────────────────┴───────────────────────┘   │
│                              │                                   │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │                    Cryptography Layer                     │   │
│  ├────────────────┬─────────────────┬───────────────────────┤   │
│  │  BLAKE3        │   Ed25519       │   VDF (Hash-Chain)    │   │
│  └────────────────┴─────────────────┴───────────────────────┘   │
│                              │                                   │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │                     Storage Layer                         │   │
│  ├────────────────┬─────────────────┬───────────────────────┤   │
│  │  Block Store   │   UTXO Set      │   Producer Registry   │   │
│  └────────────────┴─────────────────┴───────────────────────┘   │
│                              │                                   │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │                    Network Layer                          │   │
│  ├────────────────┬─────────────────┬───────────────────────┤   │
│  │  Peer Manager  │  Block Gossip   │   TX Gossip           │   │
│  └────────────────┴─────────────────┴───────────────────────┘   │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

---

## Core Components

### 1. Network Layer

#### Peer Discovery
```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│  Bootstrap  │────►│    DHT      │────►│   Peers     │
│   Nodes     │     │  (Kademlia) │     │   Found     │
└─────────────┘     └─────────────┘     └─────────────┘
```

- Uses Kademlia DHT for peer discovery
- Bootstrap nodes provide initial entry points
- Peers exchange peer lists periodically

#### Message Types
| Message      | Description                          | Priority |
|--------------|--------------------------------------|----------|
| `Block`      | New block announcement               | High     |
| `Transaction`| New transaction broadcast            | Medium   |
| `GetBlocks`  | Request blocks by hash/range         | Medium   |
| `GetHeaders` | Request headers for light clients    | Low      |
| `Peers`      | Peer list exchange                   | Low      |

#### Propagation
- Blocks: Flood propagation with deduplication
- Transactions: Inventory-based (announce hash, fetch on request)

### 2. Storage Layer

#### Block Store (RocksDB)
```
Column Families:
├── headers           # Hash → BlockHeader (serialized)
├── bodies            # Hash → Vec<Transaction> (serialized)
├── height_index      # Height (u64 LE) → Hash
└── slot_index        # Slot (u32 LE) → Hash
```

#### UTXO Set (File I/O)
- In-memory HashMap with file-based persistence
- Key: `Outpoint (txid, output_index)`
- Value: `UtxoEntry { output, height, is_coinbase, is_epoch_reward }`
- Serialized via bincode

#### ChainState (File I/O)
- Single file persistence via bincode
- Contains: best_hash, best_height, best_slot, genesis info, epoch state

#### Producer Registry
- List of registered producers with:
  - Public key
  - Registration epoch
  - Bond output reference
  - Failure counter
  - Status (active/inactive/excluded)

### 3. Validation Layer

#### Transaction Validation Pipeline
```
┌─────────┐    ┌─────────┐    ┌─────────┐    ┌─────────┐
│ Format  │───►│ Inputs  │───►│ Outputs │───►│  Fees   │
│ Check   │    │ Exist   │    │ Valid   │    │ Check   │
└─────────┘    └─────────┘    └─────────┘    └─────────┘
                    │
                    ▼
              ┌─────────┐
              │Signature│
              │ Verify  │
              └─────────┘
```

#### Block Validation Pipeline
```
┌─────────┐    ┌─────────┐    ┌─────────┐    ┌─────────┐
│ Header  │───►│ Timing  │───►│ Producer│───►│  VDF    │
│ Format  │    │ Window  │    │ Check   │    │ Verify  │
└─────────┘    └─────────┘    └─────────┘    └─────────┘
                                                  │
                                                  ▼
                                            ┌─────────┐
                                            │  TX     │
                                            │Validate │
                                            └─────────┘
```

### 4. Consensus Layer

#### Chain Manager
Maintains:
- Current best chain (canonical)
- Fork tree for competing chains
- Finality depth counter

#### Fork Choice Rule (Weight-Based)

DOLI uses a **weight-based fork choice rule**. The chain with the highest accumulated producer weight wins.

```python
def should_reorg(current_chain, new_chain):
    current_weight = sum(producer_weight(block) for block in current_chain)
    new_weight = sum(producer_weight(block) for block in new_chain)
    return new_weight > current_weight

def producer_weight(block):
    # Based on producer's seniority and activity
    # Weight ranges from 1 (new producer) to 4 (veteran)
    return block.producer.effective_weight
```

This prevents Sybil attacks where an attacker creates many low-weight blocks.

#### Slot Scheduler
```
┌───────────────────────────────────────────────────────────────┐
│                       Time                                     │
├─────┬─────┬─────┬─────┬─────┬─────┬─────┬─────┬─────┬─────────┤
│ S0  │ S1  │ S2  │ S3  │ S4  │ S5  │ S6  │ S7  │ S8  │ ...     │
├─────┴─────┴─────┴─────┴─────┴─────┴─────┴─────┴─────┴─────────┤
│ 10s │ 10s │ 10s │ 10s │ 10s │ 10s │ 10s │ 10s │ 10s │ ...     │
└───────────────────────────────────────────────────────────────┘

slot = floor((now - GENESIS_TIME) / 10)  // 10 seconds per slot
```

### 5. Producer Manager

#### Producer Selection (Deterministic Round-Robin)

DOLI uses **deterministic round-robin rotation** based on bond count, NOT probabilistic lottery:

```python
def select_producer(slot, active_producers):
    """
    Deterministic rotation by bond tickets.

    Alice: 1 bond  → 1 turn per cycle  (tickets: [0])
    Bob:   5 bonds → 5 turns per cycle (tickets: [1,2,3,4,5])
    Carol: 4 bonds → 4 turns per cycle (tickets: [6,7,8,9])

    slot % 10 → ticket index → producer
    """
    sorted_producers = sorted(active_producers, key=lambda p: p.public_key)
    total_tickets = sum(p.bond_count for p in sorted_producers)
    ticket_index = slot % total_tickets

    cumulative = 0
    for producer in sorted_producers:
        cumulative += producer.bond_count
        if ticket_index < cumulative:
            return producer
```

**Key Difference from PoS Lottery:**
| Aspect | PoS Lottery | DOLI Round-Robin |
|--------|-------------|------------------|
| Selection | Random weighted | Deterministic rotation |
| Variance | High (Bob could win 10 or 0) | Zero (Bob wins exactly 5/10) |
| Fairness | Probabilistic | Guaranteed |
| ROI | Variable | Fixed, equal % for all |

#### Bond Stacking

Producers can stake 1-100 bonds to increase their block production share:

```
┌─────────────────────────────────────────────────────────────────┐
│  BOND STACKING: Deterministic Ticket Assignment                 │
│                                                                 │
│  Alice: 1 bond  → 1 ticket  → 1 block per 10 slots             │
│  Bob:   5 bonds → 5 tickets → 5 blocks per 10 slots            │
│  Carol: 4 bonds → 4 tickets → 4 blocks per 10 slots            │
│                                                                 │
│  Rotation: [Alice, Bob, Bob, Bob, Bob, Bob, Carol, Carol, ...]  │
│             slot 0   1    2    3    4    5     6      7         │
│                                                                 │
│  EQUITABLE ROI: All producers earn same % return on investment │
└─────────────────────────────────────────────────────────────────┘
```

#### Block Production Flow
```
┌─────────────────────────────────────────────────────────────┐
│                    Block Production                          │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  1. Wait for assigned slot                                   │
│         │                                                    │
│         ▼                                                    │
│  2. Select transactions from mempool                         │
│         │                                                    │
│         ▼                                                    │
│  3. Build block header                                       │
│         │                                                    │
│         ▼                                                    │
│  4. Compute VDF proof (~700ms)                              │
│         │                                                    │
│         ▼                                                    │
│  5. Broadcast block                                          │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

### 6. Cryptography Layer

#### VDF Implementation

DOLI uses two VDF types:

**Block/Heartbeat VDF (Hash-Chain):**
```
Hash-Chain VDF:
├── Input: HASH("DOLI_HEARTBEAT_V1" || producer_key || slot || prev_hash)
├── Iterations: ~10,000,000 (dynamically calibrated)
├── Output: 32 bytes (final hash)
└── Verification: Recompute (no separate proof)
```

- Target time: ~700ms
- Iterations adjusted ±20% per cycle
- Min iterations: 100,000 | Max iterations: 100,000,000

**Registration VDF (Wesolowski Class Groups):**
```
Wesolowski VDF:
├── Input: HASH("DOLI_VDF_REGISTER_V1" || producer_key || epoch)
├── Group: Imaginary quadratic class group (2048-bit discriminant)
├── Iterations: T_REGISTER_BASE = 600M (~10 min)
├── Output: ~512 bytes (class group element)
└── Verification: O(log T) using Wesolowski proof
```

- Provides anti-Sybil protection via time investment
- Difficulty scales with network size (capped at ~24 hours)

#### Operations
| Operation        | Time (mainnet) | Time (testnet) | Time (devnet) |
|------------------|----------------|----------------|---------------|
| VDF compute      | ~700 ms        | ~70 ms         | Configurable  |
| VDF verify       | ~10 ms (recompute) | ~1 ms     | N/A           |
| BLAKE3 hash      | < 1 μs         | < 1 μs         | < 1 μs        |
| Ed25519 sign     | < 1 ms         | < 1 ms         | < 1 ms        |
| Ed25519 verify   | < 1 ms         | < 1 ms         | < 1 ms        |

---

## Data Flow

### Transaction Flow
```
┌────────┐    ┌────────┐    ┌────────┐    ┌────────┐    ┌────────┐
│ Wallet │───►│  Node  │───►│Mempool │───►│ Block  │───►│ Chain  │
│        │    │        │    │        │    │Builder │    │        │
└────────┘    └────────┘    └────────┘    └────────┘    └────────┘
   │              │              │             │             │
   │  Create TX   │   Validate   │   Store     │  Include    │  Confirm
   │  & Sign      │   & Gossip   │   pending   │  in block   │
```

### Block Flow
```
┌──────────┐    ┌──────────┐    ┌──────────┐    ┌──────────┐
│ Producer │───►│  Gossip  │───►│ Validate │───►│  Apply   │
│          │    │          │    │          │    │          │
└──────────┘    └──────────┘    └──────────┘    └──────────┘
     │              │                │               │
     │  Broadcast   │   Propagate    │   Check all   │  Update UTXO
     │  new block   │   to peers     │   rules       │  & chain state
```

---

## State Machine

### Node States
```
┌─────────────┐
│   SYNCING   │◄────────────────────┐
└──────┬──────┘                     │
       │ synced                     │ fell behind
       ▼                            │
┌─────────────┐    slot missed ┌────┴────────┐
│   RUNNING   │───────────────►│  CATCHING   │
│ (producing) │◄───────────────│    UP       │
└─────────────┘    caught up   └─────────────┘
```

### Producer States
```
┌────────────┐    registration    ┌────────────┐
│ UNREGISTERED│────confirmed────►│   ACTIVE   │
└────────────┘                    └─────┬──────┘
                                        │
                  ┌─────────────────────┼─────────────────────┐
                  │                     │                     │
                  ▼                     ▼                     ▼
           ┌────────────┐       ┌────────────┐       ┌────────────┐
           │  INACTIVE  │       │  EXCLUDED  │       │   EXITED   │
           │(50 misses) │       │ (slashed)  │       │(bond unlock)│
           └────────────┘       └────────────┘       └────────────┘
```

---

## Security Boundaries

### Trust Zones
```
┌─────────────────────────────────────────────────────────────────┐
│                        Untrusted Zone                            │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                    Network Messages                      │    │
│  │  (blocks, transactions, peer info from any source)      │    │
│  └─────────────────────────────────────────────────────────┘    │
│                              │                                   │
│                      ┌───────┴───────┐                          │
│                      │   Validation  │                          │
│                      │    Barrier    │                          │
│                      └───────┬───────┘                          │
│                              │                                   │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                     Trusted Zone                         │    │
│  │  (validated blocks, confirmed transactions, local state) │    │
│  └─────────────────────────────────────────────────────────┘    │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Critical Code Paths
1. **VDF Verification**: Must reject invalid proofs
2. **Signature Verification**: Must reject invalid signatures
3. **UTXO Validation**: Must prevent double-spending
4. **Timing Checks**: Must enforce slot boundaries

---

## Scalability Considerations

### Current Limits
| Resource          | Limit              | Notes                    |
|-------------------|--------------------|-----------------------------|
| Block size        | 1 MB               | Protocol parameter          |
| TX per block      | ~3,000             | Depends on TX size          |
| TPS (theoretical) | ~50                | 1 block/minute             |
| Header size       | ~340 bytes         | Fixed                       |
| Headers per year  | ~178 MB            | Linear growth               |

### Future Optimizations
- **Sharding**: Not planned (simplicity over throughput)
- **Layer 2**: Payment channels possible
- **Pruning**: UTXO set only required for validation
- **Light clients**: Header-only sync is efficient

---

## Enterprise-Grade Design for Global Scale

DOLI is designed to support **thousands of producers worldwide** with the following enterprise-grade features:

### 1. Deterministic Round-Robin Selection

Producer selection uses **deterministic round-robin rotation** based on bond count:

```python
def select_producer(slot, active_producers):
    """
    Deterministic rotation by bond tickets.
    Bob with 5 bonds gets exactly 5 of every N slots.
    """
    sorted_producers = sorted(active_producers, key=lambda p: p.public_key)
    total_tickets = sum(p.bond_count for p in sorted_producers)
    ticket_index = slot % total_tickets

    cumulative = 0
    for producer in sorted_producers:
        cumulative += producer.bond_count
        if ticket_index < cumulative:
            return producer
```

**Enterprise properties:**
- O(n) computation for n producers (linear scaling)
- All nodes compute identical results (no coordination needed)
- Works for millions of producers
- Zero variance: guaranteed slot allocation per cycle
- Equitable ROI: same % return for all producers regardless of bond count

### 2. Sync-Before-Produce (No Arbitrary Delays)

New nodes use state-based production gating, not time-based warmup:

```
┌─────────────────────────────────────────────────────────────────┐
│  Node starts → Discovers peers → Checks sync status            │
│                                          ↓                      │
│                    ┌─────────────────────┴──────────────────┐  │
│                    │                                         │  │
│                    ▼                                         ▼  │
│            No peers?                              Has peers?   │
│            (Seed node)                           (Joining)     │
│                    │                                   │       │
│                    ▼                                   ▼       │
│         Produce immediately              Sync first, then      │
│                                          produce when within   │
│                                          2 slots of peers      │
└─────────────────────────────────────────────────────────────────┘
```

**Why this scales globally:**
- No arbitrary time delays (warmup periods don't work for global networks)
- Thousands of nodes can start simultaneously
- Each node naturally syncs before competing
- Seed nodes bootstrap immediately

### 3. Epoch-Based Reward Distribution (Deterministic)

Rewards are distributed as **EpochReward transactions** at epoch boundaries, calculated deterministically from the BlockStore:

```
┌─────────────────────────────────────────────────────────────────┐
│                    Reward Flow                                  │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  Epoch ends (slot % 360 == 0)                                   │
│         │                                                       │
│         ├── Scan BlockStore: count blocks per producer          │
│         │                                                       │
│         ├── Calculate pool: produced_blocks × block_reward      │
│         │   (empty slots do NOT contribute to pool)             │
│         │                                                       │
│         ├── Distribute proportionally to block producers        │
│         │                                                       │
│         └── Create EpochReward transactions → UTXO set          │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

**Deterministic Calculation from BlockStore:**
```
Epoch boundary detected:
  current_epoch = current_slot / 360
  last_rewarded = scan BlockStore for most recent EpochReward tx

If current_epoch > last_rewarded:
  1. Query blocks in epoch slot range from BlockStore
  2. Count blocks per producer (exclude null producer)
  3. Pool = total_blocks × block_reward(current_height)
  4. Distribute proportionally (last producer by sorted pubkey gets dust)
  5. Create EpochReward transactions with exact amounts
```

**Key Properties:**
- **Deterministic**: Any node calculates identical rewards from same blocks
- **Restart-safe**: No local state to lose (reads from BlockStore)
- **Sync-safe**: All nodes derive same rewards independently
- **Exact validation**: Validators recalculate and must match exactly

### 4. Seniority-Weighted Consensus

Producers gain weight over time using discrete yearly steps:

```
Weight based on years active (seniority only):

Year 1:  weight = 1
Year 2:  weight = 2
Year 3:  weight = 3
Year 4+: weight = 4 (maximum)
```

**Key distinction:**
- **Weight** (seniority): Affects fork choice. Senior producers' chains are preferred.
- **Bond count**: Affects slot allocation. More bonds = more slots per cycle.
- Bond count does NOT increase weight. A 100-bond producer has the same weight as a 1-bond producer with the same seniority.

**No activity penalty:**
- Producers who miss slots simply miss rewards
- No slashing, no weight reduction for inactivity
- Only slashable offense: double production (equivocation)

**Enterprise implications:**
- New producers start at weight 1
- Long-running producers have up to 4x influence
- Prevents flash attacks with newly registered producers

### 5. VDF Anti-Grinding (Not Timing)

**Critical design decision:** Wall-clock time determines slots, NOT VDF duration.

```
WRONG: Faster VDF hardware = Faster blocks
RIGHT: Faster VDF hardware = Same speed (wall clock enforced)
```

VDF serves as a heartbeat proof of presence:
- ~700ms computation across all networks
- Cannot be parallelized (sequential by design)
- Grinding prevention comes from Epoch Lookahead (deterministic leader selection)

### 6. Fork Convergence (Weight-Based)

During network partitions or races, weight-based fork choice ensures convergence:

```python
def should_reorg(current_tip, new_tip):
    current_weight = accumulated_weight(current_tip)
    new_weight = accumulated_weight(new_tip)
    return new_weight > current_weight

def accumulated_weight(block):
    # Sum of producer weights from genesis to this block
    return sum(b.producer.effective_weight for b in chain_to(block))
```

All nodes converge to the heaviest chain regardless of when they received blocks.
This prevents Sybil attacks with many low-weight producers.

### 7. Bootstrap Mode (Development Only)

For devnet/testnet without registered producers:

| Feature | Bootstrap Mode | Production Mode |
|---------|----------------|-----------------|
| Producer set | Connected peers | On-chain registry |
| Election seed | Slot number only | prev_hash + slot |
| Registration | Not required | VDF + bond required |
| Forks | Expected, converge | Rare, converge |

**Important:** Bootstrap mode is for development. Production networks require producer registration.

---

## Directory Structure

```
doli/
├── bins/
│   ├── node/           # Full node binary (doli-node) (~6,000 lines)
│   │   └── producer/   # Producer-specific logic (signed slots, guards)
│   └── cli/            # Wallet CLI (doli-cli) (~2,500 lines)
├── crates/
│   ├── core/           # Types, consensus, validation (~23,000 lines)
│   │   ├── tpop/       # Telemetry (heartbeat, presence - NOT consensus)
│   │   └── discovery/  # Producer discovery (bloom, gossip, gset)
│   ├── crypto/         # BLAKE3, Ed25519, signatures, merkle (~2,500 lines)
│   ├── vdf/            # Hash-chain VDF (blocks), Wesolowski (registration) (~2,200 lines)
│   ├── mempool/        # Transaction pool with fee policies (~760 lines)
│   ├── network/        # libp2p P2P networking (~5,900 lines)
│   │   ├── sync/       # Sync manager, headers, bodies, equivocation, reorg
│   │   └── protocols/  # Status and sync request/response protocols
│   ├── storage/        # RocksDB blocks, File I/O for UTXO/state (~4,500 lines)
│   ├── rpc/            # JSON-RPC API server (Axum) (~1,700 lines)
│   └── updater/        # Auto-update with 3/5 multisig, 7-day veto (~1,750 lines)
├── docker/             # Docker configuration files
├── docs/               # Documentation
├── specs/              # Technical specifications
├── scripts/            # Test and utility scripts
└── testing/            # Integration tests, benchmarks, fuzz tests
```

---

## Crate Responsibilities

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

### Crate Details

| Crate | Lines | Purpose | Key Files |
|-------|-------|---------|-----------|
| `crypto` | ~2,500 | BLAKE3-256 hashing, Ed25519 signatures, merkle trees | `hash.rs` (591), `keys.rs` (764), `signature.rs` (552), `merkle.rs` (496) |
| `vdf` | ~2,200 | Wesolowski VDF (registration), hash-chain VDF (blocks) | `vdf.rs` (619), `class_group.rs` (880), `proof.rs` (280) |
| `core` | ~23,000 | Types, validation, consensus, scheduler, maintainer bootstrap | `validation.rs` (4,501), `consensus.rs` (3,688), `transaction.rs` (1,628), `network.rs` (1,120), `heartbeat.rs` (803), `maintainer.rs` (701), `scheduler.rs` (653), `block.rs` (383) |
| `storage` | ~4,500 | RocksDB blocks, UTXO, chain state, producer registry | `producer.rs` (2,698), `block_store.rs` (846), `chain_state.rs` (406), `utxo.rs` (432) |
| `network` | ~5,900 | libp2p P2P: gossipsub, Kademlia, sync, equivocation detection | `service.rs` (1,081), `sync/manager.rs` (744), `sync/reorg.rs` (595), `sync/equivocation.rs` (359), `scoring.rs` (450) |
| `mempool` | ~760 | Transaction pool with fee policies, double-spend detection | `pool.rs` (589), `policy.rs` (57) |
| `rpc` | ~1,700 | JSON-RPC server (Axum) for wallet/explorer | `methods.rs` (839), `types.rs` (527), `server.rs` (121) |
| `updater` | ~1,750 | Auto-update with 3/5 multisig, 7-day veto, 40% threshold | `lib.rs` (783), `vote.rs` (357), `download.rs` (241), `apply.rs` (233) |

### Core Crate Submodules

**tpop/** - Telemetry Proof of Presence (NOT consensus):
| File | Lines | Purpose |
|------|-------|---------|
| `presence.rs` | 1,136 | Presence commitment tracking |
| `producer.rs` | 724 | Producer telemetry state |
| `mod.rs` | 625 | Module coordination |
| `heartbeat.rs` | 612 | VDF heartbeat telemetry |
| `calibration.rs` | 527 | VDF iteration calibration |

**discovery/** - Producer Discovery:
| File | Lines | Purpose |
|------|-------|---------|
| `gset.rs` | 1,434 | Grow-only set for producer tracking |
| `gossip.rs` | 442 | Discovery gossip protocol |
| `proto.rs` | 428 | Protocol buffer definitions |
| `announcement.rs` | 376 | Producer announcements |
| `bloom.rs` | 353 | Bloom filter for efficient queries |

### Network Crate Submodules

**sync/** - Block Synchronization:
| File | Lines | Purpose |
|------|-------|---------|
| `manager.rs` | 744 | Sync orchestration |
| `reorg.rs` | 595 | Chain reorganization handling |
| `equivocation.rs` | 359 | Double-production detection and slashing |
| `bodies.rs` | 340 | Block body download |
| `headers.rs` | 221 | Header-first sync |

**protocols/** - Request/Response Protocols:
| File | Lines | Purpose |
|------|-------|---------|
| `status.rs` | 227 | Peer status exchange |
| `sync.rs` | 198 | Sync request/response |

### Maintainer Bootstrap System

The maintainer system (`core/maintainer.rs`, 701 lines) implements decentralized governance from genesis:

1. **Bootstrap**: First 5 registered producers become initial maintainers
2. **Threshold**: 3/5 multisig required for maintainer changes
3. **Operations**: AddMaintainer (tx type 12), RemoveMaintainer (tx type 11)
4. **Slashing**: Producers removed from maintainer set automatically when slashed
5. **Update Governance**: 7-day veto period, 40% veto threshold to block updates

### Binary Crates

**bins/node** (doli-node) - ~6,000 lines:
| File | Lines | Purpose |
|------|-------|---------|
| `node.rs` | 2,721 | Main node logic, block production |
| `main.rs` | 1,527 | CLI parsing, node startup |
| `updater.rs` | 884 | Update management integration |
| `metrics.rs` | 388 | Prometheus metrics |
| `producer/signed_slots.rs` | 194 | Double-sign prevention |
| `producer/guard.rs` | 112 | Lock file for single instance |

**bins/cli** (doli-cli) - ~2,500 lines:
| File | Lines | Purpose |
|------|-------|---------|
| `main.rs` | 1,667 | CLI commands (wallet, tx, producer) |
| `rpc_client.rs` | 623 | JSON-RPC client |
| `wallet.rs` | 232 | Wallet file management |

---

## Dependencies

### Core Libraries
| Library       | Purpose                    | Language |
|---------------|----------------------------|----------|
| libblake3     | BLAKE3 hashing             | C/Rust   |
| ed25519-donna | Ed25519 signatures         | C        |
| gmp           | Class group arithmetic     | C        |
| libp2p        | P2P networking             | Rust/Go  |

### Storage
| Component     | Technology                 |
|---------------|----------------------------|
| Block store   | RocksDB / LevelDB          |
| UTXO set      | In-memory + snapshots      |
| Index         | Custom B-tree              |

---

*For implementation details, see [protocol.md](protocol.md)*
