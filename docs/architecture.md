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
            ├──► rpc (JSON-RPC API)
            │       │
            │       └──► bins/node (full node binary)
            │
            ├──► updater (auto-update with community veto)
            │
            ├──► bridge (cross-chain atomic swaps)
            │
            └──► channels (payment channels)

crypto ──► wallet (shared wallet library, NO vdf/doli-core)
              │
              ├──► bins/cli (wallet binary)
              └──► bins/gui (Tauri desktop app)
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

**Purpose:** Verifiable Delay Functions (Wesolowski construction over class groups). **NOT used in production.** Both block and registration VDFs use the iterated BLAKE3 hash-chain implementation in `doli-core/src/tpop/heartbeat.rs`.

| Module | Function |
|--------|----------|
| `class_group.rs` | Class group arithmetic (GMP) |
| `vdf.rs` | VDF compute and verify |
| `proof.rs` | Wesolowski proof structures |

**Status:** The `doli-vdf` crate exists but is dead code for production purposes. The actual VDF used is a BLAKE3 hash-chain in `doli-core`.

**Actual VDF Parameters (from NetworkParams defaults):**
- Block VDF: 1,000 iterations (~0.07ms) on mainnet/testnet. Constant `T_BLOCK=800,000` exists but NetworkParams overrides it. Bond is the real Sybil protection.
- Registration VDF: `T_REGISTER_BASE=1,000` iterations (trivial). `T_REGISTER_CAP=5,000,000`.
- Devnet: Block VDF = 1 iteration (instant)

### 3.3. doli-core

**Purpose:** Blockchain types, consensus rules, and validation logic.

| Module | Function |
|--------|----------|
| `block.rs` | Block and BlockHeader types |
| `transaction/` | 27 transaction types, UTXO model |
| `types.rs` | Amount, Slot, Epoch, Era |
| `consensus/` | PoT parameters, producer selection, bonds, tiers |
| `validation/` | Block and tx validation |
| `genesis.rs` | Genesis block generation |
| `network/` | Network configuration |
| `network_params/` | Configurable network parameters (env overrides) |
| `discovery/` | Producer discovery (G-Set CRDT) |
| `tpop/` | Time proof: heartbeat VDF, presence, calibration |
| `attestation.rs` | Attestation types and bitfield encoding |
| `conditions/` | Programmable output conditions (covenants) |
| `pool.rs` | AMM pool logic |
| `lending.rs` | Lending protocol logic |
| `chainspec.rs` | Chainspec parsing and genesis hash |
| `config_validation.rs` | Configuration validation rules |
| `finality.rs` | Finality gadget types |
| `heartbeat.rs` | Heartbeat message types |
| `maintainer.rs` | Maintainer governance types |
| `nft.rs` | NFT types and validation |
| `presence.rs` | Presence tracking types |
| `rewards.rs` | Reward calculation tests |
| `scheduler.rs` | Deterministic producer scheduler |

**Transaction types** (see `crates/core/src/transaction/types.rs` for canonical list):
1. Transfer (0) - Value transfer. Also used as coinbase (Transfer with no inputs, single output to reward pool)
2. Registration (1) - Producer registration
3. Exit (2) - Producer exit
4. ClaimReward (3) - DEPRECATED (replaced by automatic EpochReward)
5. ClaimBond (4) - DEPRECATED (replaced by automatic withdrawal)
6. SlashProducer (5) - Slash equivocator (100% bond burn)
7. Coinbase (6) - DEAD CODE (enum variant exists but `new_coinbase()` creates TxType::Transfer)
8. AddBond (7) - Bond stacking (creates Bond UTXOs with creation_slot in extra_data)
9. RequestWithdrawal (8) - Instant bond withdrawal with FIFO vesting penalty
10. ClaimWithdrawal (9) - Reserved tombstone (wire compat)
11. EpochReward (10) - Epoch reward distribution (ACTIVE — pool drained bond-weighted)
12. RemoveMaintainer (11) - Remove maintainer from governance set (immediate)
13. AddMaintainer (12) - Add maintainer to governance set (immediate, not deferred)
14. DelegateBond (13) - Delegate bonds to another producer
15. RevokeDelegation (14) - Revoke delegated bonds
16. ProtocolActivation (15) - On-chain consensus rule activation (3/5 multisig)
17. MintAsset (17) - Mint fungible asset (issuer-only)
18. BurnAsset (18) - Burn fungible asset
19. CreatePool (19) - Create AMM pool with initial liquidity
20. AddLiquidity (20) - Add liquidity to AMM pool
21. RemoveLiquidity (21) - Remove liquidity from AMM pool
22. Swap (22) - Swap assets through AMM pool
23. CreateLoan (24) - Create collateralized loan
24. RepayLoan (25) - Repay loan and recover collateral
25. LiquidateLoan (26) - Liquidate undercollateralized loan
26. LendingDeposit (27) - Deposit DOLI into lending pool
27. LendingWithdraw (28) - Withdraw DOLI + interest from lending pool

**Note:** TxType 16 and 23 are reserved and not used.

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
| `block_store/` | Blocks indexed by hash/height (RocksDB, 8 CFs) |
| `state_db/` | Unified state: UTXOs, producers, chain state (RocksDB) |
| `utxo/` | In-memory UTXO working set for fast reads |
| `utxo_rocks.rs` | RocksDB-backed UTXO store (`RocksDbUtxoStore`) |
| `chain_state.rs` | Consensus state tracking |
| `producer/` | Producer registry (simplified: pubkey, registered_at, status, seniority_weight — bond tracking via UTXO set) |
| `maintainer.rs` | MaintainerState — governance maintainer set tracking |
| `update.rs` | UpdateState — auto-update state persistence |
| `archiver.rs` | Block archiver — streams blocks to flat files with BLAKE3 checksums, catch-up gap filler, restore/backfill from archive |

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
- **BlockStore** (RocksDB): 8 column families: `headers`, `bodies`, `height_index`, `slot_index`, `presence` (deprecated, cleaned on startup), `hash_to_height`, `tx_index`, `addr_tx_index`
- **StateDb** (RocksDB): Unified state store with atomic WriteBatch per block. Column families (6): `cf_utxo`, `cf_utxo_by_pubkey`, `cf_producers`, `cf_exit_history`, `cf_meta`, `cf_undo`. All state changes (UTXOs, chain state, producers) committed atomically — no crash-inconsistency possible.
- **In-memory UtxoSet**: Loaded from StateDb on startup, mutated in parallel with batch writes for fast mempool/RPC reads

### 3.6. network

**Purpose:** libp2p-based P2P networking.

| Module | Function |
|--------|----------|
| `service/` | Main network service |
| `behaviour.rs` | libp2p behaviour composition |
| `config.rs` | Network configuration |
| `gossip/` | GossipSub management (`mod.rs`, `publish.rs`, `config.rs`) |
| `sync/` | Chain synchronization (sync manager state machine) |
| `discovery.rs` | Kademlia DHT |
| `scoring.rs` | Peer reputation |
| `messages.rs` | Network message types |
| `nat.rs` | NAT traversal |
| `peer.rs` | Peer state tracking |
| `peer_cache.rs` | Persistent peer cache |
| `protocols/` | Sub-protocols (`status.rs`, `sync.rs`, `txfetch.rs`) |
| `rate_limit.rs` | Per-peer rate limiting |
| `transport.rs` | Transport layer configuration |

**Sub-protocols:**
- GossipSub for block/tx propagation (fixed mesh params per network: mainnet mesh_n=12/mesh_n_low=8/mesh_n_high=24; testnet mesh_n=25/mesh_n_low=20/mesh_n_high=50; devnet mesh_n=12/mesh_n_low=8/mesh_n_high=24)
- Kademlia DHT for peer discovery (60s bootstrap interval)
- Request-response for sync
- Identify for peer address exchange

**Connection model (two-tier):**
- **Application layer** (`max_peers`): Tracks scored peers. When full, evicts lowest gossipsub-scored peer. Producers keep slots (high P2 score from first-message delivery).
- **Transport layer** (`max_peers * 1.5`): Allows temporary over-capacity so new peers can be evaluated before eviction decides who stays. Without headroom, libp2p rejects connections at TCP level before scoring runs.
- **Defaults**: Mainnet: 50, Testnet: 25 (halved from 50, INC-I-012 Yamux RAM reduction), Devnet: 150. Override: `DOLI_MAX_PEERS` env var.
- **Peer discovery flow**: Bootstrap node → Identify → DHT → peer cache. Bootnodes are introduction points, not permanent hubs.

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
| `ws.rs` | WebSocket support |
| `methods/` | RPC method handlers (39 methods across 16 files) |
| `methods/dispatch.rs` | Request routing to handlers |
| `methods/block.rs` | Block query methods |
| `methods/balance.rs` | Balance and UTXO methods |
| `methods/producer.rs` | Producer info methods |
| `methods/transaction.rs` | Transaction submission and query |
| `methods/governance.rs` | Governance (votes, maintainers) |
| `methods/backfill.rs` | Block backfill operations |
| `methods/snapshot.rs` | State snapshot methods |
| `methods/schedule.rs` | Slot/producer schedule |
| `methods/history.rs` | Transaction history |
| `methods/network.rs` | Network info methods |
| `methods/stats.rs` | Chain statistics |
| `methods/pool.rs` | AMM pool methods |
| `methods/lending.rs` | Lending protocol methods |
| `types/` | Request/response types |
| `error.rs` | Error codes |

### 3.9. updater

**Purpose:** Auto-update with community veto.

| Module | Function |
|--------|----------|
| `constants.rs` | Bootstrap maintainer keys (per network), GitHub repo URL, fallback mirror |
| `params.rs` | `UpdateParams` — network-aware timing (veto/grace/check intervals from NetworkParams) |
| `types.rs` | Release, UpdateConfig, MaintainerSignature, VoteResult |
| `download.rs` | Fetch releases from GitHub, download binaries, verify SHA-256 hashes |
| `verification.rs` | Ed25519 release signature verification (3/5 maintainer threshold), veto calculation |
| `vote.rs` | VoteTracker — seniority-weighted vote counting (bonds x seniority multiplier) |
| `apply.rs` | Binary swap (backup, install, restart), auto-apply from GitHub |
| `enforcement.rs` | Version enforcement — pauses production if outdated after grace period |
| `watchdog.rs` | Crash detection — 3 crashes within crash_window triggers automatic rollback |
| `hardfork.rs` | Upgrade-at-height mechanism for breaking protocol changes |
| `test_keys.rs` | Test maintainer keys for devnet (DOLI_TEST_KEYS=1) |

**Features:**
- Download from GitHub releases (with fallback mirror at `releases.doli.network`)
- 3/5 maintainer Ed25519 signatures required per release
- Veto period (5 min early network; target 7 days)
- 40% seniority-weighted veto threshold
- SHA-256 hash verification
- Automatic rollback on 3 crashes within crash window
- Version enforcement: outdated producers paused after grace period

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
│ Compute Block VDF     │ (~0.07ms at 1K iters)
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
│                  BlockStore (RocksDB, 8 CFs)             │
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
├─────────────────────────────────────────────────────────┤
│ CF: hash_to_height                                       │
│   Key: Hash (32 bytes)                                   │
│   Value: Height (u64, little-endian)                     │
├─────────────────────────────────────────────────────────┤
│ CF: tx_index                                             │
│   Key: TxHash (32 bytes)                                 │
│   Value: Height (u64, little-endian)                     │
├─────────────────────────────────────────────────────────┤
│ CF: addr_tx_index                                        │
│   Key: PubkeyHash(32) ++ Height(8) (40 bytes)           │
│   Value: empty                                           │
├─────────────────────────────────────────────────────────┤
│ CF: presence (deprecated — cleaned on startup)           │
│   Legacy attestation data, no longer written             │
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
│   └── hash_chain_vdf     (BLAKE3 hash-chain, ~0.07ms)
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

Code: `bins/node/src/node/apply_block/`

### 9.2. State Root

`H(H(chain_state) || H(utxo_set) || H(producer_set))`. Each component uses `serialize_canonical()` — fixed-byte encoding, sorted keys, no bincode. Used by snap sync (quorum agreement) and cached after each `apply_block()`.

Code: `crates/storage/src/snapshot.rs`

### 9.3. Rollback

`rollback_one_block()` uses **undo-based rollback** as first option (O(1) — reverses created/spent UTXOs from stored `UndoData`). Rebuild-from-genesis is fallback only for blocks without undo data.

Code: `bins/node/src/node/rollback.rs`

### 9.4. Deferred Producer Mutations

Producer mutations (Register, AddBond, Exit, Slash, WithdrawalRequest, DelegateBond, RevokeDelegation) are queued as `PendingProducerUpdate` and applied at epoch boundaries only. Exception: epoch 0 (every block) and maintainer changes (AddMaintainer, RemoveMaintainer — immediate).

### 9.5. Genesis Phase

Blocks 1 through `genesis_blocks` are the genesis phase. At `genesis_blocks + 1`, `derive_genesis_producers_from_chain()` runs — consuming genesis coinbase UTXOs to back real bonds. Code: `bins/node/src/node/genesis.rs`

---

## 10. Dead Code Inventory

Code that exists but is never called — kept for serialization backward compatibility:

| What | Where | Why dead |
|------|-------|----------|
| `ChainState::apply_coinbase()` / `total_minted` | `chain_state.rs` | Never called, always 0 |
| `ClaimReward` (TxType 3) | `transaction.rs` | Replaced by automatic EpochReward |
| `ClaimBond` (TxType 4) | `transaction.rs` | Replaced by automatic withdrawal |
| `Coinbase` (TxType 6) | `transaction.rs` | Enum variant exists but coinbase uses `TxType::Transfer` with no inputs |
| `PresenceScore` scoring | `consensus.rs` | Orphaned — scheduler uses `DeterministicScheduler` |
| `Block::total_fees()` | `block.rs` | Always returns 0 |
| `blocks_produced`, `pending_rewards` in ProducerInfo | `producer.rs` | Vestigial from Pull/Claim model |

---

## 11. GUI Desktop Application

The DOLI GUI is a cross-platform desktop wallet built with **Tauri 2.x** (Rust backend + Svelte 5 frontend). It provides a graphical interface to all wallet, transaction, producer, rewards, NFT, bridge, and governance operations.

### 11.1. Architecture Overview

```
┌─────────────────────────────────────────────────────┐
│                 Tauri Desktop App                     │
│  ┌────────────────────────────────────────────────┐ │
│  │         Svelte 5 Frontend (WebView)            │ │
│  │  Wallet | Producer | Rewards | NFT | Settings  │ │
│  └───────────────────┬────────────────────────────┘ │
│                      │ Tauri IPC (invoke)            │
│  ┌───────────────────┴────────────────────────────┐ │
│  │         Rust Backend (Tauri Commands)           │ │
│  │  ┌──────────────────────────────────────────┐  │ │
│  │  │      crates/wallet (shared library)       │  │ │
│  │  │  Wallet + RPC Client + TxBuilder          │  │ │
│  │  │  Depends on: crypto (NOT doli-core/vdf)   │  │ │
│  │  └───────────────────┬──────────────────────┘  │ │
│  └──────────────────────│─────────────────────────┘ │
└─────────────────────────│───────────────────────────┘
                          │ HTTP POST (JSON-RPC)
                          ▼
                   DOLI Node (RPC)
```

### 11.2. New Crates

| Crate | Purpose | Dependencies |
|-------|---------|-------------|
| `crates/wallet/` | Shared wallet + RPC client library (used by CLI and GUI) | `crypto`, `bip39`, `reqwest`, `serde` |
| `bins/gui/` | Tauri desktop application | `wallet`, `tauri`, Svelte 5 frontend |

**Key design decision**: `crates/wallet/` does NOT depend on `doli-core` or `vdf`. This eliminates the GMP (rug) dependency for the GUI, enabling clean Windows builds without MSYS2. Transaction bytes are constructed directly in the wallet crate using the canonical serialization format.

### 11.3. Dependency Graph (Extended)

```
crypto (foundation)
   │
   ├──► vdf (GMP-dependent)
   │       │
   │       └──► doli-core [features = ["vdf"]]
   │               │
   │               ├──► storage, network, mempool, rpc
   │               │       │
   │               │       └──► bins/node
   │               │
   │               └──► bins/cli (uses wallet crate + doli-core for registration VDF)
   │
   └──► crates/wallet (NO vdf, NO doli-core)
           │
           ├──► bins/cli (wallet + RPC operations)
           │
           └──► bins/gui (Tauri desktop app, NO GMP needed)
```

### 11.4. Security Model

- **Private keys** remain exclusively in the Rust backend process. The Svelte frontend (WebView) never receives key material.
- **Tauri IPC** is the trust boundary. All inputs from the frontend are validated in Rust before processing.
- **Wallet file** uses the same JSON format as the CLI (`wallet.json`). Keys are in plaintext (matching CLI behavior). File permissions: 0600.
- **Content Security Policy** restricts the WebView: no eval, no inline scripts, connect-src limited to DOLI RPC endpoints.

### 11.5. CI/CD

GUI builds are added as separate GitHub Actions jobs in the release workflow. They produce:
- `.msi` (Windows x86_64)
- `.dmg` (macOS aarch64 + x86_64)
- `.AppImage` (Linux x86_64)

GUI build failures do not block CLI/node releases. The Windows GUI build does NOT require GMP/MSYS2.

Full architecture specification: `specs/gui-architecture.md`

---

*Architecture version: 2.3 — March 2026 (synced against code 2026-03-29, pass 2)*
