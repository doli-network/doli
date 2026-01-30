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

#### Block Store
```
blocks/
├── headers/          # Block headers by height
│   ├── 000000.bin   # Headers 0-99999
│   ├── 000001.bin   # Headers 100000-199999
│   └── ...
├── bodies/           # Block bodies by hash
│   └── {hash}.bin
└── index/
    ├── height.idx    # Height → Hash
    └── slot.idx      # Slot → Hash
```

#### UTXO Set
- In-memory structure with disk persistence
- Key: `txid || output_index`
- Value: `amount || pubkey_hash || lock_height`

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
│ 60s │ 60s │ 60s │ 60s │ 60s │ 60s │ 60s │ 60s │ 60s │ ...     │
└───────────────────────────────────────────────────────────────┘

slot = floor((now - GENESIS_TIME) / 60)
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

### 3. Epoch-Based Reward Accumulation (No Dust)

Rewards are NOT per-block UTXOs. They accumulate in `pending_rewards`:

```
┌─────────────────────────────────────────────────────────────────┐
│                    Reward Flow                                  │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  Block produced → Reward added to ProducerInfo.pending_rewards  │
│                                                                 │
│  1,440 blocks (1 day) → Epoch ends → Rewards distributed       │
│                                                                 │
│  Producer decides to withdraw → Single UTXO created            │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

**Example with 10,000 producers:**
```
Daily reward pool: 5 DOLI × 1,440 blocks = 7,200 DOLI
Per producer: 7,200 ÷ 10,000 = 0.72 DOLI/day

Accumulates in memory, NOT as UTXOs
Producer withdraws when convenient → Single UTXO
```

**Why this prevents dust:**
- No UTXO per block (would be 0.0005 DOLI per block with 10K producers)
- Rewards accumulate in compact `u64` field
- Single withdrawal transaction when producer claims
- Can enforce minimum withdrawal amount

### 4. Seniority-Weighted Consensus

Producers gain weight over time, preventing Sybil attacks:

```
Weight = 1 + sqrt(months_active / 12), capped at 4.0

Month 0:  weight = 1.0
Month 12: weight = 2.0
Month 48: weight = 3.0
Month 108: weight = 4.0 (maximum)
```

**Enterprise implications:**
- New producers start at weight 1
- Long-running producers have 4x influence
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
├── cmd/
│   ├── doli-node/      # Full node binary
│   └── doli-wallet/    # Wallet CLI
├── pkg/
│   ├── chain/          # Chain management
│   ├── consensus/      # Consensus logic
│   ├── crypto/         # Cryptographic primitives
│   │   ├── blake3/
│   │   ├── ed25519/
│   │   └── vdf/
│   ├── mempool/        # Transaction pool
│   ├── network/        # P2P networking
│   ├── storage/        # Database layer
│   ├── types/          # Core data types
│   └── validation/     # Validation rules
├── configs/            # Configuration files
├── docs/               # Documentation
└── tests/              # Integration tests
```

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
