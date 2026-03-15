# DOLI — Project Understanding

> Deep analysis of the codebase — architecture, domain, data flows, patterns, and risk areas.
> Generated from code as the single source of truth on 2026-03-15.
> Updated 2026-03-15 with 14 spec drifts fixed. Verified against commit `1248d3e`.

## Index

- [Quick Summary](#quick-summary)
- [Tech Stack](#tech-stack)
- [Architecture](#architecture)
  - [Module Dependency Flow](#module-dependency-flow-bottom--top)
  - [System Boundaries](#system-boundaries)
  - [The Three States](#the-three-states)
  - [Time Structure](#time-structure)
  - [18 Transaction Types](#18-transaction-types)
  - [9 Output Types](#9-output-types)
- [Key Workflows](#key-workflows)
  - [Block Production](#block-production)
  - [Block Application (Critical Path)](#block-application-critical-path)
  - [Epoch Reward Distribution](#epoch-reward-distribution)
  - [Producer Selection (Anti-Grinding)](#producer-selection-anti-grinding)
  - [Rollback (Undo-Based)](#rollback-undo-based)
- [Data Storage](#data-storage)
  - [Data Flow](#data-flow)
  - [How Data Enters](#how-data-enters)
  - [How Data Exits](#how-data-exits)
- [Consensus Parameters](#consensus-parameters-from-code)
- [Recent Additions](#recent-additions-since-last-analysis)
  - [Internal Code Inconsistencies](#internal-code-inconsistencies-code-vs-code)
- [Key Architectural Patterns](#key-architectural-patterns)
- [Complexity & Risk Map](#complexity--risk-map)
- [Technical Debt](#technical-debt)
- [Convention Breaks](#convention-breaks)
- [The Template: Adding a New Transaction Type](#the-template-adding-a-new-transaction-type)
- [Onboarding Reading Order](#onboarding-reading-order)
- [Key Files](#key-files)

## Quick Summary

DOLI is a Proof-of-Time (PoT) blockchain implemented as a Rust workspace (edition 2021, MSRV 1.85, v3.7.2). It uses a UTXO model with deterministic bond-weighted round-robin scheduling (10-second slots, 1-hour epochs), pooled epoch reward distribution with attestation-based qualification, and 18 transaction types spanning transfers, producer lifecycle, governance, NFTs, fungible assets, payment channels, and cross-chain atomic swaps. The system comprises 11 library crates and 2 binaries (`doli-node` and `doli` CLI), with RocksDB for persistence, libp2p for networking, and an embedded auto-update system with community veto power.

## Tech Stack

| Layer | Technology | Version | Purpose |
|-------|-----------|---------|---------|
| Language | Rust | 2021 edition, MSRV 1.85 | Primary language |
| Async Runtime | Tokio | 1.35 | Full async I/O, timers, channels |
| Networking | libp2p | 0.53 | P2P (TCP+Noise+Yamux, GossipSub, Kademlia, Relay, AutoNAT) |
| Storage | RocksDB | 0.22 | Block store + unified state DB (6 column families) |
| RPC | Axum | 0.7 | JSON-RPC HTTP server + WebSocket push |
| Hashing | BLAKE3 | 1.5 | All hashing — blocks, txs, VDF, addresses, state roots |
| Signatures | Ed25519 (ed25519-dalek) | 2.1 | Transaction/block signing |
| BLS | blst | 0.3 | BLS12-381 aggregate attestation signatures |
| Serialization | bincode + serde | 1.3 / 1.0 | Wire format and disk persistence |
| Protobuf | prost | 0.13 | Producer discovery protocol messages |
| CLI | clap | 4.4 | Command-line parsing (derive mode) |
| Big Integers | rug (GMP) | 1.28 | VDF crate (compiled but NOT used in production) |
| HD Wallet | bip39 | 2.1 | Mnemonic-based key generation |
| HTTP Client | reqwest | 0.11 | Auto-update downloads, CLI RPC calls |
| Metrics | Prometheus | 0.13 | Metrics endpoint |

## Architecture

### Module Dependency Flow (bottom → top)

```
doli-node (full node binary)
  ├── doli-rpc (32 JSON-RPC methods, Axum)
  ├── doli-network (libp2p P2P, sync manager)
  ├── doli-mempool (transaction pool)
  ├── doli-storage (RocksDB: block store, state DB, UTXO, producer set, archiver)
  ├── doli-updater (auto-update with community veto)
  └── doli-core (consensus rules, validation, types, scheduling)
       └── doli-crypto (BLAKE3, Ed25519, BLS, merkle)

doli (CLI binary)
  ├── doli-core
  ├── doli-crypto
  ├── doli-updater
  ├── doli-wallet (HD wallet, NO doli-core dep — reimplements tx serialization)
  ├── doli-channels (LN-Penalty payment channels)
  └── doli-vdf (Wesolowski VDF — compiled but NOT used in production)
```

The dependency graph is a **clean DAG** — no circular dependencies. Key constraint: `wallet` does NOT depend on `doli-core` at runtime (intentional, keeps CLI lightweight, but creates maintenance risk).

### System Boundaries

```
                          ┌─────────────────────────────┐
                          │       External World         │
                          │  RPC Clients  │  P2P Peers   │
                          └──────┬───────┬──────────────┘
                                 │       │
            ┌────────────────────┴───┐  ┌┴──────────────────────┐
            │     RPC Server (Axum)  │  │  Network (libp2p)      │
            │  32 methods + WebSocket│  │  GossipSub + Kademlia  │
            └────────────┬──────────┘  │  Sync Manager           │
                         │             └──────────┬──────────────┘
                         │                        │
            ┌────────────┴────────────────────────┴──────────────┐
            │                  Node Core                          │
            │  Event Loop (biased select!)                        │
            │  ┌──────────┐ ┌────────────┐ ┌─────────────────┐   │
            │  │ Mempool  │ │ Validation │ │ Block Production │   │
            │  └────┬─────┘ └──────┬─────┘ └───────┬─────────┘   │
            │       │              │               │              │
            │  ┌────┴──────────────┴───────────────┴────────┐    │
            │  │            apply_block()                    │    │
            │  │  (atomic WriteBatch — all-or-nothing)       │    │
            │  └────────────────────┬───────────────────────┘    │
            └───────────────────────┼───────────────────────────┘
                                    │
            ┌───────────────────────┼───────────────────────────┐
            │              Storage Layer                         │
            │  ┌──────────┐ ┌──────┴──────┐ ┌──────────────┐   │
            │  │BlockStore│ │  StateDb     │ │   Archiver   │   │
            │  │(RocksDB) │ │  (RocksDB)  │ │ (flat files) │   │
            │  │          │ │  cf_utxo    │ │              │   │
            │  │ headers  │ │  cf_pubkey  │ │ {h}.block    │   │
            │  │ bodies   │ │  cf_producers│ │ {h}.blake3  │   │
            │  │ h→hash   │ │  cf_exit    │ │ manifest.json│   │
            │  │ slot→hash│ │  cf_meta    │ └──────────────┘   │
            │  └──────────┘ │  cf_undo    │                    │
            │               └─────────────┘                    │
            └──────────────────────────────────────────────────┘
```

### The Three States

Every node maintains three state objects that must be identical across the network:

1. **ChainState** — height, best hash, slot, genesis timestamp, total minted, protocol version
2. **UtxoSet** — all unspent outputs (loaded into memory from RocksDB on startup)
3. **ProducerSet** — all registered producers with bonds, delegation, status, seniority

**State Root**: `H(H(chain_state) || H(utxo_set) || H(producer_set))` — uses canonical serialization for determinism across architectures and versions. Used for snap sync verification.

### Time Structure

| Unit | Slots | Duration |
|------|-------|----------|
| Slot | 1 | 10 seconds |
| Epoch | 360 | 1 hour |
| Era | 12,614,400 | ~4 years (halving interval) |

### 18 Transaction Types

| ID | Type | Purpose |
|----|------|---------|
| 0 | Transfer | Regular coin transfer |
| 1 | Registration | Producer registration (VDF proof + bonds) |
| 2 | Exit | Start unbonding period (7 days) |
| 3 | ClaimReward | (deprecated) |
| 4 | ClaimBond | Claim bond after unbonding |
| 5 | SlashProducer | Slash with equivocation evidence |
| 6 | Coinbase | Block reward to pool |
| 7 | AddBond | Increase stake (up to 3,000 bonds) |
| 8 | RequestWithdrawal | FIFO withdrawal with vesting penalty |
| 9 | ClaimWithdrawal | (tombstone — reserved, unused) |
| 10 | EpochReward | Automatic weighted epoch rewards |
| 11 | RemoveMaintainer | 3/5 multisig remove maintainer |
| 12 | AddMaintainer | 3/5 multisig add maintainer |
| 13 | DelegateBond | Delegate weight to Tier 1/2 validator |
| 14 | RevokeDelegation | Revoke delegation (7-day unbonding) |
| 15 | ProtocolActivation | On-chain consensus rule activation |
| 17 | MintAsset | Mint fungible asset (issuer-only) |
| 18 | BurnAsset | Burn fungible asset |

Note: ID 16 is skipped (reserved).

### 9 Output Types

Normal, Bond, Multisig, Hashlock, HTLC, Vesting, NFT, FungibleAsset, BridgeHTLC.

## Key Workflows

### Block Production

1. Timer fires (1s interval) → `try_produce_block()` in `production/mod.rs`
2. Version enforcement: check if update service has blocked production
3. Slot calculated from wall clock: `params.timestamp_to_slot(now)`
4. Duplicate check: skip if `last_produced_slot == current_slot`
5. Production authorization: peer connectivity, sync state, fork detection (`production/gates.rs`)
6. Early block check: skip if `block_store.has_block_for_slot()`
7. Scheduling: `slot % total_tickets` → deterministic round-robin proportional to bonds
8. Rank timing: each rank gets exclusive 2s window (5 ranks × 2s = 10s slot)
9. Signed slots guard: check `signed_slots_db` to prevent double-signing after restart
10. VDF: hash-chain BLAKE3, ~800K iterations, ~55ms
11. Post-VDF safety check: re-check `has_block_for_slot()` (block may have arrived during VDF)
12. Block assembly: coinbase (to reward pool), epoch rewards (at boundary), mempool txs
13. Record signed slot in signed_slots_db
14. Broadcast via GossipSub
15. Self-apply via `apply_block()`

### Block Application (Critical Path)

`apply_block()` in `apply_block/mod.rs`:
1. Validate block (header, producer eligibility, VDF, economics)
2. Begin atomic `WriteBatch`
3. Serialize ProducerSet for undo snapshot
4. For each transaction:
   - `process_transaction_utxos()`: spend inputs, create outputs, track undo log
   - `process_transaction_producer_effects()`: queue Registration/Exit/Slash/AddBond/etc.
   - `process_transaction_governance()`: maintainer changes, protocol activation
5. Process completed unbonding periods
6. Update chain state: height, hash, slot, protocol activation
7. At epoch boundary: apply deferred producer updates, recompute tier
8. Genesis completion: register VDF-proven producers at block 361
9. Mempool cleanup: remove applied txs, revalidate remaining
10. Store block + set canonical chain
11. Commit WriteBatch atomically
12. Persist undo data for rollback support
13. Post-commit: tier recompute, archive, websocket, attestation broadcast

### Epoch Reward Distribution

- Trigger: every 360 blocks (epoch boundary)
- Pool: accumulated coinbase rewards from reward pool address
- Attestation scanning: count attested minutes per producer per epoch (60 possible)
- Qualification tiers:
  - **Tier 1**: >= 54/60 minutes (90% threshold via `ATTESTATION_QUALIFICATION_THRESHOLD`)
  - **Tier 2 fallback**: if no Tier 1, use 80% of median attendance (floor of 1)
  - **Tier 3 accumulation**: if ALL have 0 attendance, pool accumulates
- Distribution: `reward[i] = pool × bonds[i] / qualifying_bonds` (u128 intermediates)
- Delegation split: delegate gets 10%, stakers get 90%

### Producer Selection (Anti-Grinding)

```
sorted_producers = sort by pubkey (deterministic)
total_tickets = sum of all bond counts
primary = producers[slot % total_tickets]
fallback[rank] = producers[(slot + total_tickets * rank / 5) % total_tickets]
```

Key: `prev_hash` is NOT used in selection — prevents grinding. This is "Epoch Lookahead".

### Rollback (Undo-Based)

1. Check `state_db.get_undo(height)` — returns spent UTXOs, created UTXOs, ProducerSet snapshot
2. **If undo exists** (fast path, O(1)): restore UTXOs, deserialize ProducerSet
3. **If no undo** (legacy fallback, O(chain_height)): replay from genesis
4. Update chain state to parent block
5. Rebuild liveness map from last `LIVENESS_WINDOW_MIN` blocks
6. Atomic persist via `state_db.atomic_replace()`

## Data Storage

| Store | Technology | Contents |
|-------|-----------|----------|
| BlockStore | RocksDB | Headers by hash/height, bodies, slot index, canonical chain |
| StateDb | RocksDB (unified, 6 CFs) | UTXOs, pubkey index, producers, exit history, meta, undo data |
| In-Memory | HashMap (loaded from StateDb) | UtxoSet, ProducerSet, ChainState |
| Archive | Flat files | `{height}.block` + `{height}.blake3` + `manifest.json` |

### Data Flow

```
Network Event → biased select! → handle_network_event()
    │
    ├── NewBlock → slot check → fork check → eligibility → apply_block()
    ├── NewTransaction → validate → mempool.add() → broadcast
    └── SyncResponse → sync_manager → handle_new_block()

apply_block():
    validate → begin_batch() → [per-tx: utxos + producers + governance]
    → update_chain_state → epoch_boundary_processing
    → block_store.put_block() → batch.commit() → put_undo()
    → post_commit_actions()
```

### How Data Enters

1. **P2P Gossip** (primary): GossipSub topics `/doli/blocks` and `/doli/transactions`
2. **RPC API**: `sendTransaction` method → mempool → broadcast
3. **Sync Protocol**: Request-response for headers and bodies (during initial sync)
4. **Snap Sync**: Full state snapshot from peer, verified against quorum state root
5. **Block Production**: Locally produced blocks
6. **CLI**: Wallet constructs transactions, signs, submits via RPC

### How Data Exits

1. **P2P Gossip**: Produced blocks broadcast, transactions rebroadcast
2. **RPC Responses**: JSON-RPC 2.0
3. **WebSocket Push**: Real-time new blocks and transactions on `/ws`
4. **Archive Files**: Flat files for disaster recovery (after finality)
5. **Metrics**: Prometheus on configurable port (default 9000)

## Consensus Parameters (from code)

| Parameter | Value | Source |
|-----------|-------|--------|
| Slot duration | 10 seconds | `constants.rs:46` |
| Slots per epoch | 360 | `constants.rs:50` |
| Blocks per reward epoch | 360 | `constants.rs:66` |
| Slots per era | 12,614,400 (~4 years) | `constants.rs:91` |
| Initial block reward | 1 DOLI (100,000,000 units) | `constants.rs:161` |
| Total supply | 25,228,800 DOLI | `constants.rs:297` |
| Bond unit | 10 DOLI (1,000,000,000 units) | `constants.rs:188` |
| Max bonds per producer | 3,000 | `constants.rs:195` |
| Vesting period | 4 years (4 quarters) | `constants.rs:207` |
| Vesting penalties | 75%/50%/25%/0% at Y1/Y2/Y3/Y4 | `constants.rs:242-249` |
| Unbonding period | 60,480 blocks (~7 days) | `constants.rs:215` |
| Coinbase maturity | 6 blocks | `constants.rs:175` |
| Max future slots | 1 | `constants.rs:143` |
| Max past slots | 192 (~32 minutes) | `constants.rs:149` |
| Max clock drift | 200ms | `constants.rs:383` |
| Fallback timeout | 2,000ms per rank | `constants.rs:375` |
| Max fallback ranks | 5 | `constants.rs:379` |
| Block VDF iterations | 800,000 (~55ms) | `vdf.rs:15` |
| Registration VDF iterations | 5,000,000 (~30s) | `vdf.rs:72` |
| Delegate reward | 10% | `constants.rs:399` |
| Staker reward | 90% | `constants.rs:402` |
| Bootstrap blocks | 60,480 (~1 week) | `constants.rs:101` |
| Genesis time (mainnet) | 1773186873 | `constants.rs:25` |
| Block size (Era 0) | 2 MB | `constants.rs:268` |
| Block size (max) | 32 MB | `constants.rs:271` |
| Tier 1 max validators | 500 | `constants.rs:389` |
| Tier 2 max attestors | 15,000 | `constants.rs:392` |
| Gossip regions | 15 | `constants.rs:396` |
| Activation delay | 10 blocks | `producer/constants.rs` |
| Liveness window min | 500 blocks | `constants.rs:107` |
| Re-entry interval | 50 slots | `constants.rs:112` |

## Recent Additions (since last analysis)

1. **BLS Key Support** — `RegistrationData` now includes `bls_pubkey` (48 bytes) and `bls_pop` (96 bytes) for BLS12-381 aggregate attestation signatures. The Node struct has `bls_key: Option<crypto::BlsKeyPair>`. Both fields are `#[serde(default)]` for backwards compatibility.

2. **Attestation-Based Epoch Rewards** — The reward system uses attestation bitfield qualification. Producers must be attested in >= 54/60 minutes (90% via `ATTESTATION_QUALIFICATION_THRESHOLD`) to qualify. The `presence_root` header field stores attestation bitfields.

3. **Finality Gadget** — `FinalityTracker` (`crates/core/src/finality.rs`) tracks attestation weight for blocks and determines finality at 67% of total weight (`FINALITY_WEIGHT_THRESHOLD`). Finality timeout is 3 slots.

4. **32 RPC Methods** — `getUtxoDiff` was added as the 32nd method (dispatch.rs:47).

5. **Conditions System** — Programmable output conditions (`crates/core/src/conditions/`) with composable predicates: Signature, Multisig, Hashlock, Timelock, And, Or, Threshold.

### Internal Code Inconsistencies (code vs code)

| What | Correct | Wrong | Risk |
|------|---------|-------|------|
| `BOND_UNIT` | `consensus/constants.rs:188` = 1B (10 DOLI) | `storage/producer/constants.rs:95` = 10B (100 DOLI) | **High** — storage crate's constant is deprecated but still exists. Direct reference would break consensus. |
| `HEARTBEAT_VDF_ITERATIONS` | `network_params/defaults.rs:47` = 800K | `heartbeat.rs:45` = 10M | **Medium** — legacy constant 12.5x too high. Any new code referencing it directly would produce incorrect VDF proofs. |
| VDF comment | `vdf.rs:15` T_BLOCK=800K (~55ms) | `constants.rs:41` says "~7s" | **Low** — stale comment only |

## Key Architectural Patterns

1. **Atomic State Machine** — `apply_block()` is the core state transition. All changes in single RocksDB WriteBatch. Either everything commits or nothing does.

2. **Undo-Based Rollback** — Each block records undo data (spent UTXOs, created UTXOs, ProducerSet snapshot). O(1) rollback per block. Undo data pruned after 2000 blocks.

3. **Biased Event Loop** — `tokio::select! { biased; }` ensures network events processed before production timer. Prevents producing on stale tips when gossip blocks are queued.

4. **Deferred Producer Mutations** — All producer state changes queued via `queue_update()`, applied at epoch boundaries via `apply_pending_updates()`. Exception: epoch 0 applies every block; maintainer changes are immediate.

5. **Dual-Path State Updates** — In-memory state (UtxoSet, ProducerSet) and disk state (WriteBatch) updated in parallel within `apply_block()`. Both must produce identical results.

6. **Embedded Chainspec** — Mainnet/testnet `include_str!()`, cannot be overridden. Prevents genesis_timestamp tampering.

7. **CRDT Producer Discovery** — Grow-Set (GSet) CRDT with cryptographically signed announcements. Bloom filter delta sync for large networks (>50 producers).

8. **Weight-Based Fork Choice** — Each block's weight = producer's `effective_weight()` (seniority 1-4). Heavier chain wins. Not longest-chain.

9. **Producer Safety Trifecta** — Lock file (prevents two instances), signed slots DB (prevents double-signing after restart), equivocation detector (slashes double-signers).

## Complexity & Risk Map

| Area | Risk | Location |
|------|------|----------|
| apply_block() | **Critical** — any bug = state divergence | `bins/node/src/node/apply_block/` |
| State root computation | **Critical** — canonical serialization divergence | `crates/storage/src/snapshot.rs` |
| Epoch rewards | **High** — off-by-one = wrong distribution | `bins/node/src/node/rewards.rs` |
| Sync manager | **High** — cascade incident (2026-03-14) | `crates/network/src/sync/manager/` |
| Fork recovery | **High** — must perfectly reverse apply_block | `bins/node/src/node/fork_recovery.rs` |
| execute_reorg() | **High** — rollback + apply, weight comparison | `bins/node/src/node/block_handling.rs` |
| Scheduling | **High** — all nodes MUST compute identical results | `crates/core/src/consensus/selection.rs` |
| Producer rebuilding | **High** — replays ALL producer txs from genesis | `bins/node/src/node/rewards.rs:366` |
| Reward validation | **Gap** — `validate_block_rewards_exact()` never called | `crates/core/src/validation/rewards_legacy.rs` |

## Technical Debt

1. **Reward validation disconnected** — malicious producer can inflate rewards (Open Item)
2. **VDF crate overhead** — Wesolowski (GMP) compiled but never used in production
3. **`wallet` serialization duplication** — must manually match `doli-core`, silent divergence risk
4. **Deprecated constants** — many `#[allow(deprecated)]` in core lib.rs
5. **Test coverage gaps** — no integration tests for snap sync or fork recovery
6. **ProducerSet snapshot every block** — undo data serializes entire ProducerSet (scales poorly)
7. **`presence_root` naming mismatch** — repurposed for attestation bitfields, retains old name

## Convention Breaks

1. **wallet crate isolation** — deliberately avoids `doli-core` runtime dep (`Cargo.toml` line 37-39), reimplements tx serialization
2. **VDF crate compiled but unused** — `doli-vdf` (Wesolowski/GMP) exists but both block and registration VDFs use hash-chain from `tpop/calibration.rs`
3. **ID 16 skipped** — TxType IDs go 15 (ProtocolActivation) → 17 (MintAsset), intentional reservation
4. **EpochReward validation disconnected** — tracked in MEMORY.md as Open Item
5. **`presence_root` dual purpose** — originally for presence commitments (deprecated), now attestation bitfields

## The Template: Adding a New Transaction Type

1. Add variant to `TxType` enum in `crates/core/src/transaction/types.rs`
2. Add `from_u32()` mapping
3. Create data struct in `crates/core/src/transaction/data.rs`
4. Add validation in `crates/core/src/validation/tx_types.rs`
5. Handle in `process_transaction_producer_effects()` or `process_transaction_utxos()` in `bins/node/src/node/apply_block/tx_processing.rs`
6. Handle in `rebuild_producer_set_from_blocks()` in `bins/node/src/node/rewards.rs`
7. Add CLI command in `bins/cli/src/cmd_*.rs`
8. Add RPC method if needed in `crates/rpc/src/methods/`
9. Add integration test in `testing/integration/`

## Onboarding Reading Order

1. `CLAUDE.md` + `MEMORY.md` — Mental model and operational history
2. `crates/core/src/consensus/constants.rs` — All protocol parameters
3. `crates/core/src/transaction/types.rs` — 18 tx types, 9 output types
4. `crates/core/src/consensus/selection.rs` — Deterministic scheduling
5. `bins/node/src/node/mod.rs` — Node struct (50+ fields)
6. `bins/node/src/run.rs` — Bootstrap sequence
7. `bins/node/src/node/apply_block/mod.rs` — Critical state transition
8. `bins/node/src/node/event_loop.rs` — Main event loop (biased select!)
9. `bins/node/src/node/production/mod.rs` — Block production pipeline
10. `bins/node/src/node/rewards.rs` — Epoch reward distribution
11. `crates/storage/src/snapshot.rs` — State root computation
12. `bins/node/src/node/rollback.rs` — Undo-based rollback

## Key Files

| File | Why It Matters |
|------|---------------|
| `bins/node/src/node/apply_block/mod.rs` | **THE** critical path — every block state transition |
| `bins/node/src/node/mod.rs` | Node struct (50+ fields) — central coordination |
| `bins/node/src/run.rs` | Bootstrap sequence — wires all components |
| `bins/node/src/node/event_loop.rs` | Main loop — biased select!, all event handling |
| `bins/node/src/node/rewards.rs` | Epoch rewards — the economic heart |
| `bins/node/src/node/production/mod.rs` | Block production pipeline |
| `bins/node/src/node/rollback.rs` | Undo-based rollback — fork recovery |
| `crates/core/src/consensus/constants.rs` | ALL protocol constants — the "DNA" |
| `crates/core/src/transaction/types.rs` | 18 tx types, 9 output types |
| `crates/core/src/scheduler.rs` | Deterministic scheduling |
| `crates/storage/src/snapshot.rs` | State root — canonical serialization |
| `crates/storage/src/state_db/mod.rs` | Unified state DB — atomic WriteBatch |
| `crates/network/src/sync/manager/` | Sync state machine |
