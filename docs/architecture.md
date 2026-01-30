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
│  │  ┌──────────┐  ┌──────────┐  ┌──────────────────────┐ │  │
│  │  │BlockStore│  │ UtxoSet  │  │   ChainState         │ │  │
│  │  │(RocksDB) │  │(RocksDB) │  │   (RocksDB)          │ │  │
│  │  └──────────┘  └──────────┘  └──────────────────────┘ │  │
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
- Block: 10M iterations (~700ms)
- Registration base: 600M iterations (~10 minutes)
- Discriminant: 2048 bits

### 3.3. doli-core

**Purpose:** Blockchain types, consensus rules, and validation logic.

| Module | Function |
|--------|----------|
| `block.rs` | Block and BlockHeader types |
| `transaction.rs` | 11 transaction types, UTXO model |
| `types.rs` | Amount, Slot, Epoch, Era |
| `consensus.rs` | PoT parameters, producer selection |
| `validation.rs` | Block and tx validation |
| `genesis.rs` | Genesis block generation |
| `network.rs` | Network configuration |
| `discovery/` | Producer discovery (G-Set CRDT) |

**Transaction types:**
1. Transfer - Value transfer
2. Registration - Producer registration
3. Exit - Producer exit
4. Coinbase - Block reward
5. ClaimReward - Claim epoch rewards
6. ClaimBond - Claim bond after unbonding
7. AddBond - Bond stacking
8. RequestWithdrawal - Start 7-day withdrawal
9. ClaimWithdrawal - Complete withdrawal
10. EpochReward - Epoch distribution
11. SlashProducer - Slash equivocator

### 3.4. storage

**Purpose:** RocksDB-backed persistence.

| Module | Function |
|--------|----------|
| `block_store.rs` | Blocks indexed by hash/height |
| `utxo.rs` | UTXO set with O(1) lookup |
| `chain_state.rs` | Consensus state tracking |
| `producer.rs` | Producer registry |

**ChainState fields:**
- `best_hash` - Current chain tip hash
- `best_height` - Current chain height
- `best_slot` - Current slot number
- `genesis_hash` - Genesis block hash
- `genesis_timestamp` - Genesis time (for devnet dynamic genesis)
- `last_registration_hash` - Chained registration anti-Sybil
- `registration_sequence` - Global registration counter
- `total_minted` - Total coins issued

**Column families:**
- `blocks` - Block headers and bodies
- `utxos` - Unspent outputs
- `state` - Chain tip, active producers
- `index` - Height-to-hash index

### 3.5. network

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

### 3.6. mempool

**Purpose:** Pending transaction management.

| Module | Function |
|--------|----------|
| `pool.rs` | Transaction pool |
| `entry.rs` | Tx metadata (fee, size, time) |
| `policy.rs` | Fee and size policies |

### 3.7. rpc

**Purpose:** JSON-RPC API server.

| Module | Function |
|--------|----------|
| `server.rs` | Axum HTTP server |
| `methods.rs` | RPC method handlers |
| `types.rs` | Request/response types |
| `error.rs` | Error codes |

### 3.8. updater

**Purpose:** Auto-update with community veto.

**Features:**
- Download from release server
- 7-day veto period
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

Deterministic round-robin based on bond count:

```python
def select_producer(slot, active_producers):
    # Sort by public key for determinism
    sorted_producers = sorted(active_producers, key=lambda p: p.public_key)

    # Calculate total tickets (sum of all bond counts)
    total_tickets = sum(p.bond_count for p in sorted_producers)

    # Deterministic ticket index
    ticket_index = slot % total_tickets

    # Find producer owning this ticket
    accumulated = 0
    for producer in sorted_producers:
        accumulated += producer.bond_count
        if ticket_index < accumulated:
            return producer
```

### 5.3. Fork Choice

Weight-based selection (not longest chain):

```
Chain weight = sum of producer weights for all blocks

Producer weight based on seniority:
  - 0-1 years: weight 1
  - 1-2 years: weight 2
  - 2-3 years: weight 3
  - 3+ years:  weight 4
```

---

## 6. Storage Schema

### 6.1. RocksDB Column Families

```
┌─────────────────────────────────────────────────────────┐
│                     RocksDB                              │
├─────────────────────────────────────────────────────────┤
│ CF: blocks                                               │
│   Key: Hash (32 bytes)                                   │
│   Value: Block (serialized)                              │
├─────────────────────────────────────────────────────────┤
│ CF: utxos                                                │
│   Key: Outpoint (txid + index)                          │
│   Value: UtxoEntry (output + height + spent)            │
├─────────────────────────────────────────────────────────┤
│ CF: state                                                │
│   Key: "best_hash" | "best_height" | "producers"        │
│   Value: Varies                                          │
├─────────────────────────────────────────────────────────┤
│ CF: index                                                │
│   Key: Height (u64)                                      │
│   Value: Hash (32 bytes)                                 │
└─────────────────────────────────────────────────────────┘
```

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

### 7.2. doli-cli

Lightweight wallet:

```
doli-cli
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

*Architecture version: 1.0*
