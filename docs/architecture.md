# architecture.md - System Architecture

This document describes the DOLI system architecture, component design, and data flows.

---

## 1. High-Level Architecture

```
┌──────────────────────────────────────────────────────────────┐
│                      DOLI Full Node                          │
├──────────────────────────────────────────────────────────────┤
│                                                              │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐  │
│  │   RPC API   │  │   Metrics   │  │   Auto-Updater      │  │
│  │  (Axum)     │  │ (Prometheus)│  │   (with veto)       │  │
│  └──────┬──────┘  └─────────────┘  └─────────────────────┘  │
│         │                                                    │
│  ┌──────┴──────────────────────────────────────────────┐    │
│  │                    Node Core                         │    │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────────────┐   │    │
│  │  │ Mempool  │  │Validation│  │ Block Production │   │    │
│  │  └────┬─────┘  └────┬─────┘  └────────┬─────────┘   │    │
│  └───────┼─────────────┼─────────────────┼─────────────┘    │
│          │             │                 │                   │
│  ┌───────┴─────────────┴─────────────────┴───────────────┐  │
│  │                     Storage Layer                      │  │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌────────┐ │  │
│  │  │BlockStore│  │  StateDb │  │ UtxoSet  │  │Archiver│ │  │
│  │  │(RocksDB) │  │(RocksDB) │  │(In-mem)  │  │(Files) │ │  │
│  │  └──────────┘  └──────────┘  └──────────┘  └────────┘ │  │
│  └───────────────────────────────────────────────────────┘  │
│                                                              │
│  ┌───────────────────────────────────────────────────────┐  │
│  │                   Network Layer                        │  │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────┐            │  │
│  │  │GossipSub │  │   Sync   │  │ Kademlia │            │  │
│  │  │(blocks/  │  │ Manager  │  │   DHT    │            │  │
│  │  │  txs)    │  │          │  │          │            │  │
│  │  └──────────┘  └──────────┘  └──────────┘            │  │
│  └───────────────────────────────────────────────────────┘  │
│                                                              │
└──────────────────────────────────────────────────────────────┘
```

---

## 2. Crate Dependency Graph

```
crypto (foundation - no internal deps)
   │
   ├──► vdf (VDF computation)
   │
   └──► doli-core (types, consensus, validation)
            │
            ├──► storage (RocksDB persistence)
            │
            ├──► network (P2P communication)
            │
            ├──► mempool (transaction pool)
            │
            └──► rpc (JSON-RPC API)
                    │
                    └──► bins/node (full node binary)

crypto ──► bins/cli (wallet binary)
```

---

## 3. Crate Responsibilities

### 3.1. crypto

**Purpose:** Foundation cryptographic primitives.

| Module | Function |
|--------|----------|
| `hash.rs` | BLAKE3-256 hashing |
| `keys.rs` | Ed25519 key management |
| `signature.rs` | Ed25519 signing/verification |
| `merkle.rs` | Merkle tree construction |

**Key types:**
- `Hash` - 32-byte BLAKE3 output
- `PublicKey`, `PrivateKey`, `KeyPair` - Ed25519 keys
- `Signature` - 64-byte Ed25519 signature
- `Address` - 20-byte truncated pubkey hash

**Security features:**
- Constant-time operations
- Zeroization on drop for secrets
- Domain separation tags

### 3.2. vdf

**Purpose:** Verifiable Delay Functions using Wesolowski construction over class groups.

| Module | Function |
|--------|----------|
| `class_group.rs` | Class group arithmetic (GMP) |
| `vdf.rs` | VDF compute and verify |
| `proof.rs` | Wesolowski proof structures |

**Key functions:**
- `compute(input, t)` - Compute y = x^(2^t) with proof
- `verify(input, output, proof, t)` - Verify proof
- `block_input()` - Build block VDF preimage
- `registration_input()` - Build registration VDF preimage

**Parameters:**
- Block: 800K iterations (~55ms)
- Registration base: 600M iterations (~10 minutes)
- Discriminant: 2048 bits

### 3.3. doli-core

**Purpose:** Blockchain types, consensus rules, and validation logic.

| Module | Function |
|--------|----------|
| `block.rs` | Block and BlockHeader types |
| `transaction.rs` | 16 transaction types, UTXO model |
| `types.rs` | Amount, Slot, Epoch, Era |
| `consensus.rs` | PoT parameters, producer selection |
| `validation.rs` | Block and tx validation |
| `genesis.rs` | Genesis block generation |
| `network.rs` | Network configuration |
| `discovery/` | Producer discovery (G-Set CRDT) |

**Transaction types** (see `crates/core/src/transaction.rs` for canonical list):
1. Transfer - Value transfer
2. Registration - Producer registration
3. Exit - Producer exit
4. Coinbase - Block reward (→ reward pool, not direct to producer)
5. ClaimReward - DEPRECATED (replaced by automatic EpochReward)
6. ClaimBond - DEPRECATED (replaced by automatic withdrawal)
7. AddBond - Bond stacking (creates Bond UTXOs with creation_slot in extra_data)
8. WithdrawalRequest - Bond withdrawal request (7-day delay, FIFO vesting penalty)
9. ClaimWithdrawal - Claim matured withdrawal
10. EpochReward - Epoch reward distribution (ACTIVE — pool drained bond-weighted)
11. SlashProducer - Slash equivocator (100% bond burn)
12. DelegateBond - Delegate bonds to another producer
13. RevokeDelegation - Revoke delegated bonds
14. SetPresenceCommitment - Set attestation commitment
15. AddMaintainer - Add maintainer to governance set (immediate, not deferred)
16. RemoveMaintainer - Remove maintainer from governance set (immediate)

### 3.4. Configuration Hierarchy

DOLI uses a strict 3-level configuration hierarchy:

```
┌─────────────────────────────────────────────────────────────────┐
│  Level 1: consensus.rs (RAW CONSTANTS - DNA)                    │
│  Immutable protocol constants: BOND_UNIT, SLOTS_PER_EPOCH, etc. │
└─────────────────────────┬───────────────────────────────────────┘
                          ▼
┌─────────────────────────────────────────────────────────────────┐
│  Level 2: network_params.rs (CONFIGURATION MANAGER)            │
│  • Mainnet → LOCKED (cannot override critical params)           │
│  • Devnet  → Allows .env overrides for testing                  │
└─────────────────────────┬───────────────────────────────────────┘
                          ▼
┌─────────────────────────────────────────────────────────────────┐
│  Level 3: CONSUMERS (chainspec, scheduler, validation, etc.)   │
│  ALL consumers request values from NetworkParams, never DNA     │
└─────────────────────────────────────────────────────────────────┘
```

**Key principle**: Consumer code never imports constants directly from `consensus.rs`. Instead, all configuration flows through `NetworkParams`, which enforces mainnet security while allowing devnet flexibility.

**Parameter loading order** (during node startup):
1. `.env` file loaded (with fallback from `{data_dir}/.env` to `~/.doli/{network}/.env`)
2. Chainspec defaults applied (sets env vars for params not already set; skipped for mainnet)
3. `NetworkParams::load()` reads env vars into OnceLock (frozen for process lifetime)

**Priority**: Parent ENV > `.env` file > Chainspec > `consensus.rs` defaults

### 3.5. storage

**Purpose:** RocksDB-backed persistence.

| Module | Function |
|--------|----------|
| `block_store.rs` | Blocks indexed by hash/height (RocksDB) |
| `state_db.rs` | Unified state: UTXOs, producers, chain state (RocksDB) |
| `utxo.rs` | In-memory UTXO working set for fast reads |
| `chain_state.rs` | Consensus state tracking |
| `producer.rs` | Producer registry (simplified: pubkey, registered_at, status, seniority_weight — bond tracking via UTXO set) |

**ChainState fields:**
- `best_hash` - Current chain tip hash
- `best_height` - Current chain height
- `best_slot` - Current slot number
- `genesis_hash` - Genesis block hash
- `genesis_timestamp` - Genesis time (for devnet dynamic genesis)
- `last_registration_hash` - Chained registration anti-Sybil
- `registration_sequence` - Global registration counter
- `total_minted` - Total coins issued

**Storage technologies:**
- **BlockStore** (RocksDB): Column families `headers`, `bodies`, `height_index`, `slot_index`
- **StateDb** (RocksDB): Unified state store with atomic WriteBatch per block. Column families: `cf_utxo`, `cf_utxo_by_pubkey`, `cf_producers`, `cf_exit_history`, `cf_meta`. All state changes (UTXOs, chain state, producers) committed atomically — no crash-inconsistency possible.
- **In-memory UtxoSet**: Loaded from StateDb on startup, mutated in parallel with batch writes for fast mempool/RPC reads

### 3.6. network

**Purpose:** libp2p-based P2P networking.

| Module | Function |
|--------|----------|
| `service.rs` | Main network service |
| `behaviour.rs` | libp2p behaviour composition |
| `gossip.rs` | GossipSub topic management |
| `sync/` | Chain synchronization |
| `discovery.rs` | Kademlia DHT |
| `scoring.rs` | Peer reputation |

**Sub-protocols:**
- GossipSub for block/tx propagation
- Kademlia for peer discovery
- Request-response for sync

### 3.7. mempool

**Purpose:** Pending transaction management with fee-based prioritization.

| Module | Function |
|--------|----------|
| `pool.rs` | Transaction pool with fee-based selection |
| `entry.rs` | Tx metadata (fee, size, time, ancestors) |
| `policy.rs` | Fee and size policies per network |

**Key behaviors:**
- **Fee-based prioritization**: Transactions selected by descending fee rate
- **CPFP support**: Child-Pays-For-Parent via ancestor tracking
- **Eviction policy**: Removes lowest fee rate transactions without descendants
- **Dynamic fees**: Minimum fee increases when pool >90% full
- **System transactions**: SlashProducer etc. bypass fee requirements
- **14-day expiration**: Old transactions automatically removed
- **Revalidation**: After chain reorg, invalid transactions are purged

**Default policy (mainnet):**
| Parameter | Value |
|-----------|-------|
| Max transactions | 5,000 |
| Max size | 10 MB |
| Min fee rate | 1 sat/byte |
| Max tx size | 100 KB |
| Max ancestors | 25 |
| Max age | 14 days |

### 3.8. rpc

**Purpose:** JSON-RPC API server.

| Module | Function |
|--------|----------|
| `server.rs` | Axum HTTP server |
| `methods.rs` | RPC method handlers |
| `types.rs` | Request/response types |
| `error.rs` | Error codes |

### 3.9. updater

**Purpose:** Auto-update with community veto.

**Features:**
- Download from release server
- Veto period (5 min early network; target 7 days)
- 40% weighted veto threshold
- Hash verification

---

## 4. Data Flow

### 4.1. Transaction Flow

```
User/Wallet
    │
    ▼ sendTransaction(tx)
┌───────┐
│  RPC  │
└───┬───┘
    │ validate syntax
    ▼
┌─────────┐
│ Mempool │ ◄─── verify against UTXO set
└───┬─────┘
    │
    ▼ broadcast
┌─────────┐
│ Network │ ──► GossipSub /doli/txs/1 ──► Peers
└─────────┘
    │
    ▼ (when selected as producer)
┌───────────────┐
│Block Producer │ ──► include in block
└───────────────┘
```

### 4.2. Block Flow

```
Producer Selection (deterministic round-robin)
    │
    ▼
┌───────────────────────┐
│ Compute Block VDF     │ (~700ms)
│ - prev_hash           │
│ - tx_root             │
│ - slot                │
│ - producer_key        │
└───────────┬───────────┘
            │
            ▼
┌───────────────────────┐
│ Create Block          │
│ - Header + VDF proof  │
│ - Transactions        │
└───────────┬───────────┘
            │
            ▼ broadcast
┌───────────────────────┐
│ Network (GossipSub)   │ ──► /doli/blocks/1 ──► Peers
└───────────┬───────────┘
            │
            ▼ (on receiving node)
┌───────────────────────┐
│ Validation            │
│ - VDF proof           │
│ - Producer eligibility│
│ - Transaction validity│
└───────────┬───────────┘
            │
            ▼
┌───────────────────────┐
│ Apply to Storage      │
│ - BlockStore          │
│ - UtxoSet             │
│ - ChainState          │
└───────────────────────┘
```

### 4.3. Sync Flow

```
┌────────────┐                      ┌────────────┐
│ New Node   │                      │   Peer     │
└─────┬──────┘                      └─────┬──────┘
      │                                   │
      │ StatusRequest                     │
      │──────────────────────────────────►│
      │                                   │
      │ StatusResponse(best_height=1000)  │
      │◄──────────────────────────────────│
      │                                   │
      │ GetHeaders(0, 2000)               │
      │──────────────────────────────────►│
      │                                   │
      │ Headers([h0..h1000])              │
      │◄──────────────────────────────────│
      │                                   │
      │ GetBodies([h0..h127]) ────────┐   │
      │ GetBodies([h128..h255]) ──────┼──►│ (parallel)
      │ GetBodies([h256..h383]) ──────┘   │
      │                                   │
      │ Bodies([b0..b127]) ◄──────────────│
      │ Bodies([b128..b255]) ◄────────────│
      │ Bodies([b256..b383]) ◄────────────│
      │                                   │
      │ (apply blocks to storage)         │
      │                                   │
```

**Restart behavior:** On restart, the SyncManager initializes from stored ChainState (not genesis). This means sync resumes from the chain tip, avoiding re-download of already-stored blocks. As a defense-in-depth measure, `apply_block()` also rejects blocks already present in BlockStore.

---

## 5. Consensus Architecture

### 5.1. Time Structure

```
┌─────────────────────────────────────────────────────────────┐
│                           Era (~4 years)                     │
│  ┌───────────────────────────────────────────────────────┐  │
│  │                    Epoch (1 hour)                      │  │
│  │  ┌─────┬─────┬─────┬─────┬─────┬─────┬─────┬─────┐   │  │
│  │  │Slot │Slot │Slot │ ... │Slot │Slot │Slot │Slot │   │  │
│  │  │  0  │  1  │  2  │     │ 357 │ 358 │ 359 │  0  │   │  │
│  │  │10s  │10s  │10s  │     │10s  │10s  │10s  │10s  │   │  │
│  │  └─────┴─────┴─────┴─────┴─────┴─────┴─────┴─────┘   │  │
│  │       360 slots = 1 hour                              │  │
│  └───────────────────────────────────────────────────────┘  │
│        12,614,400 slots = ~4 years                          │
└─────────────────────────────────────────────────────────────┘
```

### 5.2. Producer Selection

Deterministic round-robin based on bond count from epoch bond snapshot:

```python
def select_producer(slot, epoch_bond_snapshot):
    # epoch_bond_snapshot: Dict[PublicKey, u32] — frozen at epoch boundary from UTXO set
    # Sort by public key for determinism
    sorted_producers = sorted(epoch_bond_snapshot.items(), key=lambda p: p[0])

    # Calculate total tickets (sum of all bond counts)
    total_tickets = sum(count for _, count in sorted_producers)

    # Deterministic ticket index
    ticket_index = slot % total_tickets

    # Find producer owning this ticket
    accumulated = 0
    for pubkey, bond_count in sorted_producers:
        accumulated += bond_count
        if ticket_index < accumulated:
            return pubkey
```

The **epoch bond snapshot** is built at each epoch boundary by scanning the UTXO set for Bond UTXOs (`output_type=1`, `lock_until=u64::MAX`). This snapshot is frozen for the entire epoch, providing consistent scheduling even if bonds change mid-epoch.

### 5.3. Fork Choice

Weight-based selection (not longest chain):

```
Chain weight = sum of producer weights for all blocks

Producer weight based on seniority only (discrete yearly steps):
  - Year 1: weight = 1
  - Year 2: weight = 2
  - Year 3: weight = 3
  - Year 4+: weight = 4 (maximum)

Note: Bond count affects slot allocation (more bonds = more slots),
NOT producer weight. Weight is purely seniority-based.

There is NO activity gap penalty. Producers who miss slots simply
miss rewards - no slashing or weight reduction occurs.
```

---

## 6. Storage Schema

### 6.1. BlockStore (RocksDB)

```
┌─────────────────────────────────────────────────────────┐
│                  BlockStore (RocksDB)                    │
├─────────────────────────────────────────────────────────┤
│ CF: headers                                              │
│   Key: Hash (32 bytes)                                   │
│   Value: BlockHeader (serialized)                        │
├─────────────────────────────────────────────────────────┤
│ CF: bodies                                               │
│   Key: Hash (32 bytes)                                   │
│   Value: Vec<Transaction> (serialized)                   │
├─────────────────────────────────────────────────────────┤
│ CF: height_index                                         │
│   Key: Height (u64, little-endian)                       │
│   Value: Hash (32 bytes)                                 │
├─────────────────────────────────────────────────────────┤
│ CF: slot_index                                           │
│   Key: Slot (u32, little-endian)                         │
│   Value: Hash (32 bytes)                                 │
└─────────────────────────────────────────────────────────┘
```

### 6.2. StateDb (Unified State Store)

All mutable state is stored in a single RocksDB instance (`state_db/`) with
one atomic WriteBatch per block:

- **cf_utxo**: `Outpoint → UtxoEntry` (primary UTXO index)
- **cf_utxo_by_pubkey**: `pubkey_hash ++ outpoint → 0x00` (secondary index)
- **cf_producers**: `pubkey_hash → ProducerInfo` (simplified: public_key, registered_at, status, seniority_weight — bond data derived from UTXO set)
- **cf_exit_history**: `pubkey_hash → exit_height` (anti-Sybil tracking)
- **cf_meta**: `"chain_state"`, `"pending_updates"`, `"last_applied"` (bookkeeping)

**Atomicity**: `apply_block()` collects all state changes in a `BlockBatch`,
then commits them in a single `WriteBatch::write()`. Crash between any two
state updates is impossible. Reorgs and rollbacks use `atomic_replace()`.

**In-memory working set**: UTXOs are also kept in an `InMemoryUtxoStore` for
fast reads by mempool, RPC, and state root computation. StateDb is authoritative.

**Migration**: On first startup after upgrade, old files (`chain_state.bin`,
`producers.bin`, `utxo_rocks/`) are automatically migrated into StateDb.

---

## 7. Binary Architecture

### 7.1. doli-node

Full node with optional block production:

```
doli-node
├── config::NodeConfig     (configuration)
├── node::Node             (core orchestration)
│   ├── NetworkService     (P2P)
│   ├── BlockStore         (blocks)
│   ├── UtxoSet            (UTXOs)
│   ├── ChainState         (state)
│   ├── Mempool            (pending txs)
│   └── RpcServer          (API)
├── producer::Producer     (optional block production)
│   └── VdfWorker          (VDF computation)
├── metrics::MetricsServer (Prometheus)
└── updater::Updater       (auto-updates)
```

### 7.2. doli (CLI)

Lightweight wallet:

```
doli
├── wallet::Wallet         (key management)
│   ├── KeyStore           (Ed25519 keys)
│   └── AddressBook        (labeled addresses)
└── rpc_client::RpcClient  (node communication)
```

---

## 8. Security Boundaries

```
┌─────────────────────────────────────────────────────────┐
│                    Trust Boundary                        │
│  ┌───────────────────────────────────────────────────┐  │
│  │                   User Space                       │  │
│  │                                                    │  │
│  │  ┌──────────┐                    ┌──────────┐     │  │
│  │  │  Wallet  │ ◄── signed tx ──► │   Node   │     │  │
│  │  │ (cli)    │                    │          │     │  │
│  │  └──────────┘                    └────┬─────┘     │  │
│  │       │                               │           │  │
│  │       │ RPC (localhost only)          │           │  │
│  │       └───────────────────────────────┘           │  │
│  └───────────────────────────────────────────────────┘  │
│                          │                               │
│                          │ P2P (encrypted)               │
│                          ▼                               │
│  ┌───────────────────────────────────────────────────┐  │
│  │                   Network                          │  │
│  │                                                    │  │
│  │    Untrusted peers - all data validated            │  │
│  │                                                    │  │
│  └───────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────┘
```

**Validation at boundaries:**
- All blocks validated (VDF proof, transactions)
- All transactions validated (signatures, UTXO existence)
- All messages size-limited and rate-limited
- Peer scoring and disconnection for misbehavior

---

## 9. Critical Invariants

### 9.1. Dual UTXO Paths

`apply_block()` writes UTXOs to TWO stores that MUST stay in sync:
- **In-memory**: `utxo.add_transaction(tx, height, is_reward_tx, slot)` — used for state root, live queries
- **Disk batch**: `batch.add_transaction_utxos(tx, height, is_reward_tx, slot)` — persisted to RocksDB, loaded on restart

Both stamp Bond `extra_data` with the block slot. If they diverge → state roots diverge across nodes after restart → snap sync breaks (quorum impossible). This was the root cause of the snap sync failure discovered 2026-03-11.

Code: `bins/node/src/node.rs:~3561-3574`

### 9.2. State Root

`H(H(chain_state) || H(utxo_set) || H(producer_set))`. Each component uses `serialize_canonical()` — fixed-byte encoding, sorted keys, no bincode. Used by snap sync (quorum agreement) and cached after each `apply_block()`.

Code: `crates/storage/src/snapshot.rs`

### 9.3. Rollback

`rollback_one_block()` uses **undo-based rollback** as first option (O(1) — reverses created/spent UTXOs from stored `UndoData`). Rebuild-from-genesis is fallback only for blocks without undo data.

Code: `bins/node/src/node.rs:~6492`

### 9.4. Deferred Producer Mutations

Producer mutations (Register, AddBond, Exit, Slash, WithdrawalRequest, DelegateBond, RevokeDelegation) are queued as `PendingProducerUpdate` and applied at epoch boundaries only. Exception: epoch 0 (every block) and maintainer changes (AddMaintainer, RemoveMaintainer — immediate).

### 9.5. Genesis Phase

Blocks 1 through `genesis_blocks` are the genesis phase. At `genesis_blocks + 1`, `derive_genesis_producers_from_chain()` runs — consuming genesis coinbase UTXOs to back real bonds. Code: `bins/node/src/node.rs:~6129`

---

## 10. Dead Code Inventory

Code that exists but is never called — kept for serialization backward compatibility:

| What | Where | Why dead |
|------|-------|----------|
| `ChainState::apply_coinbase()` / `total_minted` | `chain_state.rs` | Never called, always 0 |
| `ClaimReward` (TxType 3) | `transaction.rs` | Replaced by automatic EpochReward |
| `ClaimBond` (TxType 6) | `transaction.rs` | Replaced by automatic withdrawal |
| `PresenceScore` scoring | `consensus.rs` | Orphaned — scheduler uses `DeterministicScheduler` |
| `Block::total_fees()` | `block.rs` | Always returns 0 |
| `blocks_produced`, `pending_rewards` in ProducerInfo | `producer.rs` | Vestigial from Pull/Claim model |

---

*Architecture version: 2.0 — March 2026*
