# DOLI — Project Understanding

> Generated from codebase analysis on 2026-03-15. Code is the single source of truth.

## Quick Summary

DOLI is a Proof-of-Time (PoT) blockchain implemented in Rust (edition 2021, MSRV 1.85). It uses a UTXO model, deterministic bond-weighted round-robin scheduling (10-second slots), pooled epoch reward distribution, and 18 transaction types including NFTs, fungible assets, payment channels, and cross-chain atomic swaps. The codebase is a Cargo workspace with 11 library crates and 2 binaries (`doli-node` and `doli` CLI).

## Tech Stack

| Layer | Technology | Purpose |
|-------|-----------|---------|
| Language | Rust 2021, 1.85+ | Primary language |
| Async Runtime | Tokio 1.35 | Full async I/O |
| Networking | libp2p 0.53 | P2P (TCP+Noise+Yamux, GossipSub, Kademlia) |
| Storage | RocksDB 0.22 | Block store, unified state DB |
| RPC | Axum 0.7 | JSON-RPC HTTP + WebSocket |
| Hashing | BLAKE3 1.5 | All hashing (blocks, txs, VDF, addresses) |
| Signatures | Ed25519 (ed25519-dalek 2.1) | Transaction/block signing |
| BLS | blst 0.3 | BLS12-381 aggregate attestations |
| Serialization | bincode + serde | Wire format and disk persistence |
| CLI | clap 4.4 | Command parsing |
| Big Integers | rug (GMP) 1.28 | VDF crate (compiled but NOT used in production) |
| Wallet HD | bip39 2.1 | Mnemonic-based key generation |

## Architecture

### Module Dependency Flow (bottom → top)

```
doli-node (full node binary)
  ├── doli-rpc (31 JSON-RPC methods, Axum)
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

### The Three States

Every node maintains three state objects that must be identical across the network:

1. **ChainState** — height, best hash, slot, genesis timestamp, total minted, protocol version
2. **UtxoSet** — all unspent outputs (loaded into memory from RocksDB on startup)
3. **ProducerSet** — all registered producers with bonds, delegation, status, seniority

**State Root**: `H(H(chain_state) || H(utxo_set) || H(producer_set))` — used for snap sync verification.

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
2. Slot calculated from wall clock: `params.timestamp_to_slot(now)`
3. Scheduling: `slot % total_tickets` → deterministic round-robin proportional to bonds
4. Rank timing: each rank gets exclusive 2s window (5 ranks × 2s = 10s slot)
5. VDF: hash-chain BLAKE3, ~800K iterations, ~55ms
6. Block assembly: coinbase (to reward pool), epoch rewards (at boundary), mempool txs
7. Broadcast via GossipSub

### Block Application (Critical Path)

`apply_block()` in `apply_block/mod.rs`:
1. Validate block (header, producer eligibility, VDF, economics)
2. Begin atomic `WriteBatch`
3. For each transaction: process UTXOs + producer effects + governance
4. At epoch boundary: apply deferred producer updates, distribute epoch rewards
5. Store block, commit WriteBatch atomically
6. Persist undo data (enables O(1) rollback)

### Epoch Reward Distribution

- Trigger: every 360 blocks
- Attestation scanning: count attested minutes per producer per epoch
- Qualification: Tier 1 (90% threshold), Tier 2 (80% of median), Tier 3 (no qualifiers → pool accumulates)
- Distribution: `reward[i] = pool × bonds[i] / qualifying_bonds`

### Producer Selection (Anti-Grinding)

```
sorted_producers = sort by pubkey (deterministic)
total_tickets = sum of all bond counts
primary = producers[slot % total_tickets]
fallback[rank] = producers[(slot + total_tickets * rank / 5) % total_tickets]
```

Key: `prev_hash` is NOT used in selection — prevents grinding. This is "Epoch Lookahead".

## Data Storage

| Store | Technology | Contents |
|-------|-----------|----------|
| BlockStore | RocksDB | Headers by hash/height, bodies, slot index |
| StateDb | RocksDB (unified, 6 CFs) | UTXOs, producers, chain state, exit history, undo data |
| In-Memory | HashMap (loaded from StateDb) | UtxoSet, ProducerSet, ChainState |
| Archive | Flat files | `{height}.block` + `{height}.blake3` + `manifest.json` |

## Key Architectural Patterns

1. **Atomic State Persistence** — All per-block changes in single RocksDB WriteBatch
2. **Undo-Based Rollback** — O(1) rollback per block, genesis-rebuild fallback
3. **Biased Event Loop** — `select! { biased; }` prioritizes network over production
4. **Deferred Producer Mutations** — All changes queued, applied at epoch boundary
5. **Dual-Path State Updates** — In-memory + RocksDB updated in parallel, must match
6. **Embedded Chainspec** — Mainnet/testnet `include_str!()`, cannot be overridden

## Complexity & Risk Map

| Area | Risk | Location |
|------|------|----------|
| apply_block() | **Critical** — any bug = state divergence | `bins/node/src/node/apply_block/` |
| Epoch rewards | **High** — off-by-one = wrong distribution | `bins/node/src/node/rewards.rs` |
| Sync manager | **High** — cascade incident (2026-03-14) | `crates/network/src/sync/manager/` |
| Fork recovery | **High** — must perfectly reverse apply_block | `bins/node/src/node/fork_recovery.rs` |
| Scheduling | **High** — all nodes MUST compute identical results | `crates/core/src/consensus/selection.rs` |
| State root | **Critical** — canonical serialization divergence | `crates/storage/src/snapshot.rs` |
| Reward validation | **Gap** — `validate_block_rewards_exact()` never called | `crates/core/src/validation/rewards_legacy.rs` |

## Technical Debt

1. **Reward validation disconnected** — malicious producer can inflate rewards
2. **VDF crate overhead** — Wesolowski (GMP) compiled but never used
3. **`wallet` serialization duplication** — must manually match `doli-core`
4. **Deprecated constants** — migration to `NetworkParams` incomplete
5. **Test coverage gaps** — no integration tests for snap sync or fork recovery

## Onboarding Reading Order

1. `CLAUDE.md` + `MEMORY.md` — Mental model and operational history
2. `crates/core/src/consensus/constants.rs` — All protocol parameters
3. `crates/core/src/transaction/types.rs` — 18 tx types, 9 output types
4. `crates/core/src/consensus/selection.rs` — Deterministic scheduling
5. `bins/node/src/node/mod.rs` — Node struct (50+ fields)
6. `bins/node/src/run.rs` — Bootstrap sequence
7. `bins/node/src/node/apply_block/mod.rs` — Critical state transition
8. `bins/node/src/node/production/mod.rs` — Block production pipeline
9. `bins/node/src/node/rewards.rs` — Epoch reward distribution
10. `crates/storage/src/snapshot.rs` — State root computation
