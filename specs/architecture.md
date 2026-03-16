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

### Archiver / Seed Node
A full sync-only node that serves three roles simultaneously:
- **P2P Seed**: DNS-registered network entry point (`seed1/seed2.doli.network`) with relay capability
- **Block Archive**: Streams every applied block to flat files with BLAKE3 checksums (`--archive-to`)
- **Public RPC**: Bound to `0.0.0.0`, serves the block explorer (`doli.network/explorer.html`) and external queries

Key properties:
- Atomic writes (tmp + rename) — crash-safe
- Non-blocking streaming via `mpsc::channel` — never stalls sync
- Catches up missed blocks on restart from local BlockStore
- Multiple seeds per network (DNS: `seed1/seed2.doli.network` for mainnet, `bootstrap1/2.testnet.doli.network` for testnet)

Recovery methods (all verify BLAKE3 + genesis_hash):
- **File-based** (offline): `doli-node restore --from /path/to/archive --backfill --yes`
- **RPC-based** (offline): `doli-node restore --from-rpc http://seed2.doli.network:8500 --backfill --yes`
- **Hot backfill** (live): `backfillFromPeer` RPC — fills gaps without restart, with chain-linking + anchor verification

See [docs/archiver.md](/docs/archiver.md) for full details.

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

#### StateDb (Unified RocksDB)

All mutable state (UTXOs, chain state, producers) is stored in a single RocksDB
instance (`state_db/`) with one atomic WriteBatch per block. This eliminates
crash-inconsistency between state components.

```
Column Families:
├── cf_utxo             # Outpoint (36B) → UtxoEntry (bincode)
│                       #   Normal: {output_type:0, amount, pubkey_hash, lock_until:0, extra_data:[]}
│                       #   Bond:   {output_type:1, amount, pubkey_hash, lock_until:MAX, extra_data:[creation_slot as u32 LE]}
├── cf_utxo_by_pubkey   # pubkey_hash(32B) ++ outpoint(36B) → 0x00
├── cf_producers        # pubkey_hash (32B) → ProducerInfo (bincode)
│                       #   Simplified: {public_key, registered_at, status, seniority_weight}
│                       #   Bond data derived from UTXO set (no bond fields in ProducerInfo)
├── cf_exit_history     # pubkey_hash (32B) → exit_height (8B LE)
└── cf_meta             # string key → varies
    ├── "chain_state"       → ChainState (bincode)
    ├── "pending_updates"   → Vec<PendingProducerUpdate> (bincode)
    └── "last_applied"      → {height: u64, hash: Hash, slot: u32} (44B)
```

**Bond tracking via UTXO set:** Bond count and vesting data are not stored in
ProducerInfo. Instead, they are derived from Bond UTXOs in the UTXO set:
- `bond_count`: count of Bond UTXOs for a given pubkey_hash
- `creation_slot`: read from 4-byte extra_data in each Bond UTXO
- `vesting_penalty`: computed from `creation_slot` vs current slot
- **Epoch bond snapshot**: at each epoch boundary, a `HashMap<PublicKey, u32>`
  is built by scanning all Bond UTXOs. This snapshot drives scheduling for the
  entire epoch. Mid-epoch changes take effect at next epoch boundary.

**Atomicity**: `apply_block()` creates a `BlockBatch` that collects all UTXO
spends/adds, producer mutations, and chain state updates, then commits them
in a single `WriteBatch::write()`. Reorgs and rollbacks use `atomic_replace()`
which deletes all CFs and writes new state in one batch.

**In-memory working set**: UTXOs are also kept in-memory (`InMemoryUtxoStore`)
for fast reads by mempool, RPC, and state root computation. The in-memory set
is mutated in parallel with the batch — StateDb is the authoritative store.

**Migration**: On first startup with new binary, if `state_db/` doesn't exist
but old files do (`chain_state.bin`, `producers.bin`, `utxo_rocks/`), the node
migrates all state into StateDb and renames old files to `.backup`.

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

Producers can stake 1-3,000 bonds to increase their block production share:

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
│  4. Compute VDF proof (~55ms)                               │
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
├── Iterations: ~800,000 (dynamically calibrated)
├── Output: 32 bytes (final hash)
└── Verification: Recompute (no separate proof)
```

- Target time: ~55ms
- Iterations adjusted ±20% per cycle
- Min iterations: 100,000 | Max iterations: 100,000,000

**Registration VDF (Hash-Chain):**
```
Hash-Chain VDF:
├── Input: HASH("DOLI_VDF_REGISTER_V1" || producer_key || epoch)
├── Iterations: T_REGISTER_BASE = 5,000,000 (~30 seconds)
├── Output: 32 bytes (final hash)
└── Verification: Recompute (no separate proof)
```

- Provides anti-Sybil protection via time investment
- Fixed iterations (no dynamic scaling) — bond provides primary Sybil resistance at scale

#### Operations
| Operation        | Time (mainnet) | Time (testnet) | Time (devnet) |
|------------------|----------------|----------------|---------------|
| VDF compute      | ~55 ms         | ~55 ms         | Configurable  |
| VDF verify       | ~1 ms (recompute)  | ~1 ms     | N/A           |
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
| Block size        | 2 MB (Era 0)       | Doubles per era, max 32 MB  |
| TX per block      | ~3,000             | Depends on TX size          |
| TPS (theoretical) | ~300               | 1 block/10 seconds          |
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

### 2. Sync-Before-Produce (Multi-Layer Production Gating)

New nodes use state-based production gating with multiple safety layers:

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
│         Produce after               Sync → 30s grace → check  │
│         bootstrap grace             slot proximity → produce   │
└─────────────────────────────────────────────────────────────────┘
```

**Production gating layers** (in `SyncManager::can_produce()`):

| Layer | Check | Purpose |
|-------|-------|---------|
| 1 | Explicit block | Block production explicitly disabled |
| 2 | Resync in progress | Block during active resync |
| 3 | Syncing state | Block while downloading headers/bodies |
| 4 | Bootstrap grace period | Wait at genesis for chain evidence |
| 5 | Post-resync grace | Wait after resync completes |
| 5.5 | Minimum peers | Require min peers (devnet=1, mainnet=2) |
| 6 | Behind peers (slots only) | Block if >2 slots behind best peer |
| 7 | Ahead of network | Fork detection — too far ahead of peers |
| 9 | Chain hash mismatch | Fork detection — different hash at same height |

**Key design decisions:**
- **Slot-only peer comparison** (Layer 6): Heights are unreliable because forked nodes accumulate inflated block counts (h > slot). Slots are time-based and cannot be inflated.
- **Chain hash mismatch** (Layer 9): Detects forks by comparing block hashes at the same height with peers.
- **Restart-safe initialization** (ISSUE-5 fix): On startup, `Node::new()` initializes the SyncManager with the stored ChainState tip (`best_height`, `best_hash`, `best_slot`). Without this, the SyncManager starts at genesis and re-downloads the entire chain, causing height double-counting. Additionally, `apply_block()` includes a defense-in-depth duplicate check via `BlockStore::has_block()` to prevent height corruption from re-applied blocks.

**Why this scales globally:**
- No arbitrary time delays at genesis (warmup periods don't work for global networks)
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
- ~55ms computation across all networks (800K iterations)
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
│   └── cli/            # Wallet CLI (doli) (~2,500 lines)
├── crates/
│   ├── core/           # Types, consensus, validation (~23,000 lines)
│   │   ├── tpop/       # Telemetry (heartbeat, presence - NOT consensus)
│   │   └── discovery/  # Producer discovery (bloom, gossip, gset)
│   ├── crypto/         # BLAKE3, Ed25519, signatures, merkle (~2,500 lines)
│   ├── vdf/            # Wesolowski VDF crate (compiled, NOT used in production) (~2,200 lines)
│   ├── mempool/        # Transaction pool with fee policies (~760 lines)
│   ├── network/        # libp2p P2P networking (~5,900 lines)
│   │   ├── sync/       # Sync manager, headers, bodies, equivocation, reorg
│   │   └── protocols/  # Status and sync request/response protocols
│   ├── storage/        # RocksDB blocks + unified StateDb for UTXO/state (~5,500 lines)
│   ├── rpc/            # JSON-RPC API server (Axum) (~1,700 lines)
│   └── updater/        # Auto-update with 3/5 multisig, 2-epoch veto (~1,750 lines)
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
bins/node (doli-node)          bins/cli (doli)
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
| `vdf` | ~2,200 | Wesolowski VDF crate (compiled but NOT used in production — both block and registration VDFs use hash-chain BLAKE3 in doli-core) | `vdf.rs` (619), `class_group.rs` (880), `proof.rs` (280) |
| `core` | ~24,000 | Types, validation, consensus, scheduler, maintainer bootstrap, network params | `validation.rs` (4,501), `consensus.rs` (3,688), `transaction.rs` (1,628), `network.rs` (1,120), `network_params.rs` (~500), `heartbeat.rs` (803), `maintainer.rs` (701), `scheduler.rs` (653), `block.rs` (383) |
| `storage` | ~4,500 | RocksDB blocks, UTXO, chain state, producer registry | `producer.rs` (2,698), `block_store.rs` (846), `chain_state.rs` (406), `utxo.rs` (432) |
| `network` | ~5,900 | libp2p P2P: gossipsub, Kademlia, sync, equivocation detection | `service.rs` (1,081), `sync/manager.rs` (744), `sync/reorg.rs` (595), `sync/equivocation.rs` (359), `scoring.rs` (450) |
| `mempool` | ~760 | Transaction pool with fee policies, double-spend detection | `pool.rs` (589), `policy.rs` (57) |
| `rpc` | ~1,700 | JSON-RPC server (Axum) for wallet/explorer | `methods.rs` (839), `types.rs` (527), `server.rs` (121) |
| `updater` | ~1,750 | Auto-update with 3/5 multisig, 2-epoch veto, 40% threshold | `lib.rs` (783), `vote.rs` (357), `download.rs` (241), `apply.rs` (233) |

### Core Crate Submodules

**Environment Configuration** (`network_params.rs`, `config_validation.rs`):

Network parameters are configurable via environment variables loaded from `~/.doli/{network}/.env`.

#### Configuration Hierarchy

DOLI uses a strict 3-level configuration hierarchy to prevent inconsistencies:

```
┌─────────────────────────────────────────────────────────────────┐
│  Level 1: consensus.rs (RAW CONSTANTS - DNA)                    │
│  ┌─────────────────────────────────────────────────────────────┐│
│  │  BOND_UNIT = 1_000_000_000                                  ││
│  │  SLOTS_PER_EPOCH = 360                                      ││
│  │  T_BLOCK = 800_000                                            ││
│  │  (immutable protocol constants)                             ││
│  └─────────────────────────────────────────────────────────────┘│
└─────────────────────────┬───────────────────────────────────────┘
                          │ defaults flow down
                          ▼
┌─────────────────────────────────────────────────────────────────┐
│  Level 2: network_params.rs (CONFIGURATION MANAGER)            │
│  ┌─────────────────────────────────────────────────────────────┐│
│  │  IF Network == Mainnet → LOCKED (use consensus.rs values)   ││
│  │  IF Network == Devnet  → Allow .env override                ││
│  │                                                             ││
│  │  NetworkParams::bond_unit() → returns appropriate value     ││
│  └─────────────────────────────────────────────────────────────┘│
└─────────────────────────┬───────────────────────────────────────┘
                          │ values requested
                          ▼
┌─────────────────────────────────────────────────────────────────┐
│  Level 3: CONSUMERS (ask Manager, NEVER access DNA directly)   │
│  ┌──────────────┐ ┌──────────────┐ ┌────────────────────────┐  │
│  │ chainspec.rs │ │ scheduler.rs │ │ storage/producer.rs    │  │
│  └──────────────┘ └──────────────┘ └────────────────────────┘  │
│  ┌──────────────┐ ┌──────────────┐ ┌────────────────────────┐  │
│  │validation.rs │ │ heartbeat.rs │ │ tpop/presence.rs       │  │
│  └──────────────┘ └──────────────┘ └────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

**Key Rule**: Consumers get values from `NetworkParams` ONLY, never directly from `consensus.rs`.

**Why this matters:**
- Mainnet security: Critical parameters (VDF iterations, bond amounts, timing) cannot be accidentally overridden
- Devnet flexibility: Developers can test with faster epochs, lower bonds, etc.
- Single source of truth: No duplicate constants scattered across crates

**Example - Correct Usage:**
```rust
// ✓ CORRECT: Get bond_unit from NetworkParams
let params = NetworkParams::for_network(Network::Mainnet);
let bond_units = amount / params.bond_unit;

// ✗ WRONG: Import directly from consensus.rs
use core::consensus::BOND_UNIT;  // DON'T DO THIS in consumers
```

| File | Purpose |
|------|---------|
| `network_params.rs` | NetworkParams struct with all configurable parameters, loads .env files, applies chainspec defaults |
| `config_validation.rs` | Validates params and enforces mainnet locks |

**Configurable parameters** (~25 total):
- Networking: ports, bootstrap nodes
- Timing: slot duration, veto periods, unbonding
- Economics: bond unit, rewards, fees
- VDF: iterations for blocks and registration
- Time structure: blocks per year/epoch

**Parameter loading order** (during node startup):
1. `load_env_for_network()` — Loads `.env` from `{data_dir}/.env`, with fallback to `~/.doli/{network}/.env`
2. `apply_chainspec_defaults()` — Sets env vars from chainspec for params not already set (skipped for mainnet)
3. `NetworkParams::load()` — Reads env vars into OnceLock (frozen for process lifetime)

**Priority hierarchy**: Parent ENV > `.env` file > Chainspec > `consensus.rs` defaults

**Chainspec overrides**: When a chainspec file is provided (`--chainspec`), consensus-critical parameters (`genesis_time`, `slot_duration`) are read from the chainspec rather than from `.env`. This ensures all nodes on the same network use identical timing parameters regardless of local configuration. **These overrides are applied on every startup**, not just on first initialization — this is critical because nodes that restart with existing data (`producers.bin`) must still load consensus parameters from the chainspec to avoid divergent slot computation.

**Security**: Critical parameters (VDF, emission, timing) are **locked for mainnet** and cannot be overridden via environment or chainspec. Attempting to override logs a warning and uses hardcoded values.

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
| `manager.rs` | ~2,200 | Sync orchestration, production gating (10-layer), first-sync grace period, restart-safe ChainState init |
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
5. **Update Governance**: 2-epoch (~2h) veto period, 40% veto threshold to block updates

### Binary Crates

**bins/node** (doli-node) - modularized:
| Directory/File | Purpose |
|---------------|---------|
| `node/mod.rs` | Node struct (50+ fields), public API |
| `node/init.rs` | `Node::new()` — storage, migration, state load |
| `node/startup.rs` | `run()`, `start_network()`, `start_rpc()` |
| `node/event_loop.rs` | Main event loop (biased select!) |
| `node/block_handling.rs` | `handle_new_block()`, `execute_reorg()` |
| `node/validation_checks.rs` | Producer eligibility, block validation |
| `node/rewards.rs` | Epoch reward calculation |
| `node/rollback.rs` | Undo-based rollback |
| `node/fork_recovery.rs` | 9 fork recovery functions |
| `node/apply_block/` | Core state transition (5 sub-modules) |
| `node/production/` | Block production pipeline (4 sub-modules) |
| `node/periodic.rs` | Periodic tasks, maintainer bootstrap |
| `node/genesis.rs` | Genesis producer derivation |
| `main.rs` | CLI parsing, node startup |
| `updater.rs` | Update management integration |
| `producer.rs` | Lock file, signed slots DB |

**bins/cli** (doli) - modularized:
| File | Purpose |
|------|---------|
| `main.rs` | Entry point, 30+ subcommands |
| `commands.rs` | Cli/Commands enums |
| `rpc_client.rs` | JSON-RPC HTTP client |
| `wallet.rs` | Key management |
| `cmd_producer.rs/` | Producer lifecycle (7 sub-modules) |
| `cmd_nft.rs/` | NFT operations (7 sub-modules) |
| `cmd_chain.rs` | Chain info, rewards |
| `cmd_token.rs` | Fungible asset operations |
| `cmd_bridge.rs` | Cross-chain atomic swaps |
| `cmd_channel.rs` | Payment channels |
| `cmd_governance.rs` | Governance operations |

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
| Block store   | RocksDB (headers, bodies, height/slot indexes) |
| State DB      | RocksDB (6 CFs: cf_utxo, cf_utxo_by_pubkey, cf_producers, cf_exit_history, cf_meta, cf_undo) |
| UTXO set      | In-memory HashMap (loaded from StateDb on startup) |
| Archive       | Flat files ({height}.block + {height}.blake3 + manifest.json) |

---

*For implementation details, see [protocol.md](protocol.md)*
