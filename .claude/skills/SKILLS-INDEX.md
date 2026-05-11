<!-- @INDEX
MANIFEST        24-64
KEYWORD-MAP     67-351
COVERAGE        353-376
CROSS-REFS      379-441
INDEX-WARNINGS  443-455
@/INDEX -->

# SKILLS-INDEX — DOLI Master Skill Manifest

Generated: 2026-05-11
Project: DOLI PoS blockchain — Rust, ~166K LOC, ~434 source files
Domains mapped: 14 source code domains + 15 operational/workflow skills = 29 total skill files
Index path: `.claude/skills/SKILLS-INDEX.md`

**Quick-grep instructions:**
- To find a skill for a concept, grep this file for the keyword (e.g., `grep "apply_block" SKILLS-INDEX.md`)
- Each row in the KEYWORD MAP gives: `skill-file : section : line-range`
- Load ONLY that section using `Read` with `offset` + `limit` — never load a full skill file unless required
- @INDEX accuracy warnings are in the INDEX-WARNINGS section at the bottom

---

## MANIFEST

### Source Code Domain Skills (14)

| Skill | Directory | Key Concepts | Entry Points | File Count | Notes |
|-------|-----------|-------------|-------------|-----------|-------|
| core | `core/SKILL.md` | consensus, validation, scheduler, epoch, transactions, activation heights | validate_block, DeterministicScheduler, EpochState, NetworkParams | 86 source files | Base layer — all other crates depend on this |
| node | `node/SKILL.md` | node binary, apply_block, event loop, fork recovery, block production, reorg | Node::new, run_node, apply_block, try_produce_block, execute_reorg | 72 source files | Top-level binary; consumes all domain crates |
| network | `network/SKILL.md` | libp2p, GossipSub, SyncManager, snap sync, peer scoring, rate limiting | NetworkService::new, SyncManager::new, PeerScorer, RateLimiter | 50 source files | P2P transport + sync state machine |
| storage | `storage/SKILL.md` | RocksDB, BlockStore, StateDb, UtxoSet, ProducerSet, snapshot, archiver | BlockStore::open, StateDb::open, UtxoSet::new, ProducerSet::new | 43 source files | 7 column families; atomic batch writes |
| cli | `cli/SKILL.md` | CLI binary `doli`, wallet, producer, NFT, bridge, pool, loan, channel, guardian | main (main.rs:91), RpcClient, all subcommands | 40 source files | No doli_core at runtime (uses wallet crate) |
| rpc | `rpc/SKILL.md` | JSON-RPC 2.0, 45 methods, HTTP POST, WebSocket, admin auth, block/chain/UTXO methods | RpcServer, RpcContext, handle_request, dispatch.rs | 27 source files | HTTP on :8500 (mainnet); admin requires Bearer token from public IPs |
| channels | `channels/SKILL.md` | payment channels, HTLC, commitment transactions, penalty, revocation | ChannelManager::new, ChannelRecord, CommitmentPair | 21 source files | Off-chain bilateral; disputes settled on-chain |
| gui | `gui/SKILL.md` | Tauri 2.x desktop app, NodeManager, embedded node, wallet commands | main (main.rs:21), AppState::new, NodeManager::start | 13 source files | Depends on wallet crate — NOT on bins/node directly |
| updater | `updater/SKILL.md` | auto-update, HardForkSchedule, VoteTracker, enforcement, watchdog | apply_update, check_production_allowed, HardForkSchedule, VoteTracker | 13 source files | Used by both node and cli binaries |
| crypto | `crypto/SKILL.md` | BLAKE3, Ed25519, BLS12-381, Merkle, adaptor signatures, ECIES | Hash, KeyPair, BlsKeyPair, MerkleTree, Signature | 9 source files | Pure leaf — no doli-specific runtime deps |
| wallet | `wallet/SKILL.md` | wallet file, BIP-39, TxBuilder, RpcClient, fee calculation | Wallet, TxBuilder, RpcClient, calculate_registration_cost | 12 source files | CRITICAL: NO doli_core at runtime (dev-dep only) |
| bridge | `bridge/SKILL.md` | cross-chain atomic swaps, BTC/ETH, watcher daemon, HTLC | Watcher::run, SwapRecord, SwapState | 7 source files | Largely standalone; external chain integrations |
| mempool | `mempool/SKILL.md` | tx pool, CPFP, fee validation, replace-by-fee | Mempool::new, MempoolEntry, MempoolPolicy | 4 source files | In-memory; feeds block production |
| testing | `testing/SKILL.md` | integration tests, e2e, fuzz, simulation, benchmarks, test utilities | TestNode, Node::new_for_test, test_two_nodes_sync_basic | 30 test files | Consumes all domain crates |

### Operational / Workflow Skills (15)

| Skill | Directory | Purpose | Sub-files |
|-------|-----------|---------|-----------|
| doli-network | `doli-network/SKILL.md` | RPC reference, server inventory (ai1/ai2/ai3), state root debugging, epoch params | — |
| producers | `producers/SKILL.md` | External producer server management (index only) | servers.md, onboarding.md, upgrade.md, wipe-protocol.md, troubleshooting.md, auto-update.md, auto-bond.md, migration.md |
| delegation | `delegation/SKILL.md` | Bond delegation CLI, RPC fields, constants, epoch-deferred processing | — |
| faucet | `faucet/SKILL.md` | GitHub bot faucet (ai2), hot wallet + vault (ai3), anti-abuse, refill | — |
| guardian | `guardian/SKILL.md` | Mainnet protection, fork detection, emergency halt, checkpoint recovery, canonical anchors | reference/overview.md, procedures.md, node-heal.md, hostile-recovery.md, anchors.md, deployment.md |
| mainnet | `mainnet/SKILL.md` | Full mainnet fleet deploy (ai1-ai5), per-service binary layout, confirmation gates | RECOVERY.md |
| auto-update | `auto-update/SKILL.md` | Auto-update implementation guide, vote weight formula, devnet E2E test scripts | — |
| network-setup | `network-setup/SKILL.md` | Node setup, devnet/testnet/mainnet parameters, DHT rules, producer activation lifecycle | — |
| testnet-deploy | `testnet-deploy/SKILL.md` | Testnet binary deploy (ai1/ai3), compilation on ai2, MD5 verification checklist | — |
| release | `release/SKILL.md` | Per-node binary layout map for all servers, deploy procedure one-liners | — |
| explorer | `explorer/SKILL.md` | explorer.doli.network, PRODUCER_KEYS/NODES config, auto-bond script | — |
| hetzner | `hetzner/SKILL.md` | Hetzner VPS manager, server types/prices, cloud-init provisioning | — |
| sync-docs | `sync-docs/SKILL.md` | Documentation alignment workflow, truth hierarchy, 8-step commit process | — |
| test-script | `test-script/SKILL.md` | Test script management, scripts/README.md registry protocol | — |
| skill-creator | `skill-creator/SKILL.md` | Skill creation guide, progressive disclosure design, frontmatter requirements | — |

---

## KEYWORD MAP

Keyword-to-skill lookup. Grep this table directly. Format: `skill-file : section : line-range`.
Line ranges reflect verified actual content positions, correcting @INDEX inaccuracies where noted.

### A

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| `activate_feature` | `core/SKILL.md` | ACTIVATION-HEIGHTS | 362-400 |
| activation height | `core/SKILL.md` | ACTIVATION-HEIGHTS | 362-400 |
| adaptor signature | `crypto/SKILL.md` | ALGORITHMS | 168-222 |
| `apply_block` | `node/SKILL.md` | FUNCTIONS | 165-307 |
| `apply_block` data flow | `node/SKILL.md` | DATA-FLOWS | 49-99 |
| `apply_block` storage path | `storage/SKILL.md` | DATA-FLOWS | 40-73 |
| atomic swap | `bridge/SKILL.md` | DATA-FLOWS | 178-207 |
| attestation bitfield | `core/SKILL.md` | DATA-FLOWS | 46-80 |
| attestation encode/decode | `core/SKILL.md` | PATTERNS | 447-480 |
| `auto_apply_from_github` | `updater/SKILL.md` | ENTRY-POINTS | 13-38 |
| auto-update | `updater/SKILL.md` | ENTRY-POINTS | 13-38 |
| auto-update implementation | `auto-update/SKILL.md` | full file | — |

### B

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| `backfillFromPeer` | `rpc/SKILL.md` | METHODS | 38-295 |
| `backfill_from_archive` | `node/SKILL.md` | ENTRY-POINTS | 14-47 |
| `bls_aggregate` | `crypto/SKILL.md` | ENTRY-POINTS | 17-30 |
| `bls_sign` / `bls_verify` | `crypto/SKILL.md` | FUNCTIONS | 82-166 |
| BLS12-381 | `crypto/SKILL.md` | ALGORITHMS | 168-222 |
| block archiver | `storage/SKILL.md` | FUNCTIONS-ARCHIVER | 527-560 |
| block production | `node/SKILL.md` | FUNCTIONS | 165-307 |
| `BlockArchiver` | `storage/SKILL.md` | ENTRY-POINTS | 16-38 |
| `BlockBuilder` | `core/SKILL.md` | DATA-FLOWS | 46-80 |
| `BlockStore` | `storage/SKILL.md` | FUNCTIONS-BLOCKSTORE | 207-267 |
| `BlockStore::open` | `storage/SKILL.md` | ENTRY-POINTS | 16-38 |
| bond lifecycle | `core/SKILL.md` | STRUCTS | 98-175 |
| bond withdrawal | `cli/SKILL.md` | COMMANDS | 38-450 |
| `broadcast_block` | `network/SKILL.md` | ENTRY-POINTS | 16-43 |
| bridge | `bridge/SKILL.md` | ENTRY-POINTS | 17-30 |
| bridge CLI | `cli/SKILL.md` | COMMANDS | 38-450 |

### C

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| `calculate_epoch_rewards` | `node/SKILL.md` | FUNCTIONS | 165-307 |
| `calculate_registration_cost` | `wallet/SKILL.md` | ENTRY-POINTS | 17-30 |
| canonical anchors | `guardian/SKILL.md` | full index | 1-30 |
| `ChainState` | `storage/SKILL.md` | ENTRY-POINTS | 16-38 |
| `ChannelManager` | `channels/SKILL.md` | ENTRY-POINTS | 18-32 |
| `ChannelRecord` | `channels/SKILL.md` | STRUCTS | 34-103 |
| `check_producer_eligibility` | `node/SKILL.md` | FUNCTIONS | 165-307 |
| `check_production_allowed` | `updater/SKILL.md` | ENTRY-POINTS | 13-38 |
| checkpoint | `guardian/SKILL.md` | full index | 1-30 |
| `createCheckpoint` RPC | `rpc/SKILL.md` | METHODS | 38-295 |
| commitment transaction | `channels/SKILL.md` | FUNCTIONS | 105-195 |
| consensus params | `core/SKILL.md` | CONSTANTS | 267-360 |
| `ConsensusParams` | `core/SKILL.md` | CONSTANTS | 267-360 |
| CPFP | `mempool/SKILL.md` | FUNCTIONS | 64-130 |
| `createWallet` GUI | `gui/SKILL.md` | COMMANDS | 28-114 |
| cross-chain swap | `bridge/SKILL.md` | DATA-FLOWS | 178-207 |

### D

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| delegate bond | `delegation/SKILL.md` | full file | — |
| `DelegateBondData` | `delegation/SKILL.md` | full file | — |
| delegation | `delegation/SKILL.md` | full file | — |
| `DELEGATE_REWARD_PCT` | `delegation/SKILL.md` | full file | — |
| `DELEGATION_UNBONDING_SLOTS` | `delegation/SKILL.md` | full file | — |
| deploy producers | `producers/SKILL.md` | full index | — |
| deploy testnet | `testnet-deploy/SKILL.md` | full file | — |
| deploy mainnet | `mainnet/SKILL.md` | full file | — |
| `DeterministicScheduler` | `core/SKILL.md` | ENTRY-POINTS | 14-44 |
| devnet params | `network-setup/SKILL.md` | full file | — |
| `doli init` | `cli/SKILL.md` | COMMANDS | 38-450 |
| `doli new` | `cli/SKILL.md` | COMMANDS | 38-450 |
| `doli producer delegate` | `delegation/SKILL.md` | full file | — |
| `doli producer register` | `cli/SKILL.md` | COMMANDS | 38-450 |
| documentation sync | `sync-docs/SKILL.md` | full file | — |

### E

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| ECIES encryption | `crypto/SKILL.md` | ALGORITHMS | 168-222 |
| Ed25519 | `crypto/SKILL.md` | STRUCTS | 32-80 |
| emergency halt | `guardian/SKILL.md` | full index | 1-30 |
| `enterRecoveryMode` RPC | `rpc/SKILL.md` | METHODS | 38-295 |
| epoch boundary | `core/SKILL.md` | DATA-FLOWS | 46-80 |
| `EpochState` | `core/SKILL.md` | ENTRY-POINTS | 14-44 |
| `EpochState::derive_at_boundary` | `core/SKILL.md` | ENTRY-POINTS | 14-44 |
| epoch rewards | `node/SKILL.md` | FUNCTIONS | 165-307 |
| equivocation | `node/SKILL.md` | FUNCTIONS | 165-307 |
| `execute_reorg` | `node/SKILL.md` | FUNCTIONS | 165-307 |
| explorer | `explorer/SKILL.md` | full file | — |

### F

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| faucet | `faucet/SKILL.md` | full file | — |
| fee calculation | `wallet/SKILL.md` | FUNCTIONS | 78-157 |
| fork detection | `guardian/SKILL.md` | full index | 1-30 |
| fork recovery | `node/SKILL.md` | FUNCTIONS | 165-307 |
| `fork_recovery.rs` | `node/SKILL.md` | FUNCTIONS | 165-307 |
| `ForkBlock` | `node/SKILL.md` | DATA-FLOWS | 49-99 |

### G

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| genesis block | `core/SKILL.md` | ENTRY-POINTS | 14-44 |
| `genesis_hash` | `core/SKILL.md` | ENTRY-POINTS | 14-44 |
| `generate_genesis_block` | `core/SKILL.md` | ENTRY-POINTS | 14-44 |
| `getBlockByHash` | `rpc/SKILL.md` | METHODS | 38-295 |
| `getBlockByHeight` | `rpc/SKILL.md` | METHODS | 38-295 |
| `getBlockData` | `rpc/SKILL.md` | METHODS | 38-295 |
| `getBlockRaw` | `rpc/SKILL.md` | METHODS | 38-295 |
| `getChainInfo` | `rpc/SKILL.md` | METHODS | 38-295 |
| `getEpochInfo` | `rpc/SKILL.md` | METHODS | 38-295 |
| `getProducers` | `rpc/SKILL.md` | METHODS | 38-295 |
| `getStateRootDebug` | `rpc/SKILL.md` | METHODS | 38-295 |
| `getStateSnapshot` | `rpc/SKILL.md` | METHODS | 38-295 |
| `getUtxoDiff` | `rpc/SKILL.md` | METHODS | 38-295 |
| `getUtxos` | `rpc/SKILL.md` | METHODS | 38-295 |
| GossipSub | `network/SKILL.md` | STRUCTS | 74-133 |
| GUI desktop app | `gui/SKILL.md` | ENTRY-POINTS | 15-26 |

### H

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| `handle_new_block` | `node/SKILL.md` | FUNCTIONS | 165-307 |
| hard fork | `updater/SKILL.md` | HARDFORK-SCHEDULE | 232-251 |
| `HardForkSchedule` | `updater/SKILL.md` | HARDFORK-SCHEDULE | 232-251 |
| Hetzner VPS | `hetzner/SKILL.md` | full file | — |
| HTLC channels | `channels/SKILL.md` | STRUCTS | 34-103 |
| HTLC bridge | `bridge/SKILL.md` | STRUCTS | 33-93 |

### I

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| `InFlightHtlc` | `channels/SKILL.md` | STRUCTS | 34-103 |
| integration tests | `testing/SKILL.md` | INTEGRATION-TESTS | 15-169 |

### K

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| `KeyPair` | `crypto/SKILL.md` | ENTRY-POINTS | 17-30 |

### L

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| libp2p | `network/SKILL.md` | STRUCTS | 74-133 |
| loan CLI | `cli/SKILL.md` | COMMANDS | 38-450 |

### M

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| `MaintainerState` | `storage/SKILL.md` | ENTRY-POINTS | 16-38 |
| mainnet deploy | `mainnet/SKILL.md` | full file | — |
| mainnet recovery | `guardian/SKILL.md` | full index | 1-30 |
| `Mempool` | `mempool/SKILL.md` | ENTRY-POINTS | 18-30 |
| `MempoolEntry` | `mempool/SKILL.md` | STRUCTS | 32-62 |
| `MerkleTree` | `crypto/SKILL.md` | STRUCTS | 32-80 |
| mint asset | `cli/SKILL.md` | COMMANDS | 38-450 |

### N

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| `NetworkParams` | `core/SKILL.md` | ENTRY-POINTS | 14-44 |
| `NetworkParams::load` | `core/SKILL.md` | ENTRY-POINTS | 14-44 |
| `NetworkService` | `network/SKILL.md` | ENTRY-POINTS | 16-43 |
| `NetworkService::new` | `network/SKILL.md` | ENTRY-POINTS | 16-43 |
| `Node::new` | `node/SKILL.md` | ENTRY-POINTS | 14-47 |
| `Node::new_for_test` | `testing/SKILL.md` | TEST-UTILITIES | 397-468 |
| `NodeManager` GUI | `gui/SKILL.md` | STRUCTS | 116-192 |
| NFT | `cli/SKILL.md` | COMMANDS | 38-450 |

### P

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| `pauseProduction` | `rpc/SKILL.md` | METHODS | 38-295 |
| payment channel | `channels/SKILL.md` | ENTRY-POINTS | 18-32 |
| payment channel CLI | `cli/SKILL.md` | COMMANDS | 38-450 |
| peer scoring | `network/SKILL.md` | FUNCTIONS | 135-201 |
| `PeerScorer` | `network/SKILL.md` | ENTRY-POINTS | 16-43 |
| penalty transaction | `channels/SKILL.md` | FUNCTIONS | 105-195 |
| `ProducerSet` | `storage/SKILL.md` | FUNCTIONS-PRODUCERSET | 422-490 |
| `ProducerSet::new` | `storage/SKILL.md` | ENTRY-POINTS | 16-38 |
| producer onboarding | `producers/SKILL.md` | full index | — |
| producer registration | `cli/SKILL.md` | COMMANDS | 38-450 |
| `ProtocolActivation` | `core/SKILL.md` | ACTIVATION-HEIGHTS | 362-400 |

### R

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| `RateLimiter` | `network/SKILL.md` | ENTRY-POINTS | 16-43 |
| recover chain state | `node/SKILL.md` | ENTRY-POINTS | 14-47 |
| recovery mode | `guardian/SKILL.md` | full index | 1-30 |
| restore wallet | `cli/SKILL.md` | COMMANDS | 38-450 |
| revocation store | `channels/SKILL.md` | STRUCTS | 34-103 |
| `RevokeDelegationData` | `delegation/SKILL.md` | full file | — |
| rewards calculation | `node/SKILL.md` | FUNCTIONS | 165-307 |
| `rollback_one_block` | `node/SKILL.md` | FUNCTIONS | 165-307 |
| RocksDB column families | `storage/SKILL.md` | COLUMN-FAMILIES | 160-205 |
| `RpcClient` | `wallet/SKILL.md` | ENTRY-POINTS | 17-30 |
| RPC methods all 45 | `rpc/SKILL.md` | METHODS | 38-295 |
| `run_event_loop` | `node/SKILL.md` | ENTRY-POINTS | 14-47 |
| `run_node` | `node/SKILL.md` | ENTRY-POINTS | 14-47 |

### S

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| scheduler | `core/SKILL.md` | ENTRY-POINTS | 14-44 |
| seed guardian | `guardian/SKILL.md` | full index | 1-30 |
| `select_producer` | `core/SKILL.md` | ENTRY-POINTS | 14-44 |
| `sendTransaction` | `rpc/SKILL.md` | METHODS | 38-295 |
| serialization formats | `storage/SKILL.md` | SERIALIZATION | 672-720 |
| `sign_release_hash` | `updater/SKILL.md` | ENTRY-POINTS | 13-38 |
| skill creation | `skill-creator/SKILL.md` | full file | — |
| snap sync | `network/SKILL.md` | DATA-FLOWS | 45-72 |
| `StateDb` | `storage/SKILL.md` | FUNCTIONS-STATEDB | 269-360 |
| `StateDb::open` | `storage/SKILL.md` | ENTRY-POINTS | 16-38 |
| `StateSnapshot` | `storage/SKILL.md` | FUNCTIONS-SNAPSHOT | 492-525 |
| state root | `storage/SKILL.md` | FUNCTIONS-SNAPSHOT | 492-525 |
| state root debug | `doli-network/SKILL.md` | full file | — |
| `STAKER_REWARD_PCT` | `delegation/SKILL.md` | full file | — |
| `SyncManager` | `network/SKILL.md` | ENTRY-POINTS | 16-43 |
| `SyncManager::new` | `network/SKILL.md` | ENTRY-POINTS | 16-43 |

### T

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| Tauri desktop | `gui/SKILL.md` | ENTRY-POINTS | 15-26 |
| test node | `testing/SKILL.md` | TEST-UTILITIES | 397-468 |
| `TestNode` | `testing/SKILL.md` | TEST-UTILITIES | 397-468 |
| transaction builder | `wallet/SKILL.md` | FUNCTIONS | 78-157 |
| transaction types | `core/SKILL.md` | STRUCTS | 98-175 |
| `try_produce_block` | `node/SKILL.md` | FUNCTIONS | 165-307 |
| `TxBuilder` | `wallet/SKILL.md` | ENTRY-POINTS | 17-30 |

### U

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| update governance | `updater/SKILL.md` | DATA-FLOWS | 252-308 |
| `UtxoSet` | `storage/SKILL.md` | FUNCTIONS-UTXO | 362-420 |
| `UtxoSet::new` | `storage/SKILL.md` | ENTRY-POINTS | 16-38 |

### V

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| `validate_block` | `core/SKILL.md` | ENTRY-POINTS | 14-44 |
| `validate_transaction` | `core/SKILL.md` | ENTRY-POINTS | 14-44 |
| `verifyChainIntegrity` | `rpc/SKILL.md` | METHODS | 38-295 |
| vote tracker | `updater/SKILL.md` | STRUCTS | 39-78 |
| `VoteTracker` | `updater/SKILL.md` | STRUCTS | 39-78 |

### W

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| wallet file format | `wallet/SKILL.md` | STRUCTS | 32-76 |
| wallet management | `cli/SKILL.md` | COMMANDS | 38-450 |
| `Wallet` | `wallet/SKILL.md` | STRUCTS | 32-76 |
| `Watcher::run` | `bridge/SKILL.md` | ENTRY-POINTS | 17-30 |
| WebSocket subscriptions | `rpc/SKILL.md` | ENTRY-POINTS | 16-35 |
| wipe protocol | `producers/SKILL.md` | full index | — |

---

## COVERAGE

### Source Domain Coverage

| Status | Domain | File Count | Skill File | Notes |
|--------|--------|-----------|-----------|-------|
| COVERED | core | 86 | `core/SKILL.md` | Complete; @INDEX has ~16-line STRUCTS offset (see INDEX-WARNINGS) |
| COVERED | node | 72 | `node/SKILL.md` | Complete; @INDEX PATTERNS range overruns file end (see INDEX-WARNINGS) |
| COVERED | network | 50 | `network/SKILL.md` | Complete |
| COVERED | storage | 43 | `storage/SKILL.md` | Complete; most detailed skill (720 lines, 11 sections) |
| COVERED | cli | 40 | `cli/SKILL.md` | Complete |
| COVERED | rpc | 27 | `rpc/SKILL.md` | Complete; 45 methods documented |
| COVERED | channels | 21 | `channels/SKILL.md` | Complete |
| COVERED | gui | 13 | `gui/SKILL.md` | Complete |
| COVERED | updater | 13 | `updater/SKILL.md` | Complete |
| COVERED | crypto | 9 | `crypto/SKILL.md` | Complete |
| COVERED | wallet | 12 | `wallet/SKILL.md` | Complete |
| COVERED | bridge | 7 | `bridge/SKILL.md` | Complete; doli_core dep undeclared in skill (see CROSS-REFS) |
| COVERED | mempool | 4 | `mempool/SKILL.md` | Complete |
| COVERED | testing | 30 | `testing/SKILL.md` | Complete |

**No source domain coverage gaps.** All 14 mapped source domains have skill files.
All 15 operational workflow skills exist. No orphan skill files detected.

---

## CROSS-REFS

### Dependency Chain (bottom to top)

```
crypto (pure leaf — no doli deps)
  └─> core (consensus types, validation, scheduler, epoch state)
        ├─> storage (RocksDB persistence, BlockStore, StateDb, UtxoSet, ProducerSet)
        ├─> mempool (tx pool — also depends on storage)
        ├─> channels (payment channels)
        ├─> network (P2P transport + SyncManager)
        └─> rpc (JSON-RPC server — also depends on storage, network, mempool)
              └─> node (top-level binary — consumes all above)
                    └─> testing (consumes all for integration tests)

crypto ──> wallet ──> cli (bins/cli)
crypto ──> wallet ──> gui (bins/gui) [via wallet crate, NOT via bins/node]
core   ──> updater ──> node (auto-update governance)
core   ──> updater ──> cli
bridge (standalone — uses DOLI node via HTTP RPC, not as a library dep)
```

### Verified Adjacency Table

| From Skill | To Skill | Relationship | Verified |
|-----------|---------|-------------|---------|
| `node` | `core` | bins/node depends on doli_core for validation, scheduler, epoch state | YES |
| `node` | `storage` | Node::new opens BlockStore + StateDb + ProducerSet + ChainState | YES |
| `node` | `network` | Node drives NetworkService by polling next_event() | YES |
| `node` | `rpc` | Node starts RPC server via start_rpc() | YES |
| `node` | `updater` | Node spawns update service; calls check_production_allowed | YES |
| `node` | `mempool` | Node feeds mempool for block production; purges on error | YES |
| `node` | `crypto` | Node uses Hash, KeyPair directly across many paths | YES |
| `cli` | `wallet` | bins/cli uses wallet crate for TxBuilder + RpcClient + fees | YES |
| `cli` | `updater` | CLI handles update/governance commands via updater crate | YES |
| `cli` | `crypto` | CLI uses Hash, Address for key ops | YES |
| `cli` | `storage` | CLI opens storage for some local operations | YES |
| `gui` | `wallet` | bins/gui uses wallet crate (declared in gui/SKILL.md DEPENDENCIES) | YES |
| `gui` | `crypto` | GUI uses crypto for key operations | YES |
| `rpc` | `core` | RPC uses doli_core types throughout | YES |
| `rpc` | `storage` | RPC reads BlockStore + StateDb + ProducerSet | YES |
| `rpc` | `network` | RPC exposes sync/peer state via network callbacks | YES |
| `rpc` | `mempool` | RPC exposes mempool state + handles tx submission | YES |
| `network` | `core` | SyncManager + status protocol use doli_core types | YES |
| `storage` | `core` | Storage uses Block, Transaction, Amount from doli_core | YES |
| `mempool` | `core` | Mempool uses ConsensusParams, transaction types | YES |
| `mempool` | `storage` | Mempool reads UtxoSet for validation | YES |
| `channels` | `core` | Channels use Amount, BlockHeight, transaction types | YES |
| `channels` | `crypto` | Channels use Hash, KeyPair, adaptor signatures | YES |
| `updater` | `core` | Updater uses NetworkParams for version enforcement | YES |
| `updater` | `crypto` | Updater uses Ed25519 for release signature verification | YES |
| `testing` | all | testing/SKILL.md consumes all domain crates via Node::new_for_test | YES |

### Asymmetry Flags — Issues Requiring Skill File Corrections

| Severity | Skill File | Issue |
|---------|-----------|-------|
| INCOMPLETE | `core/SKILL.md` DEPENDENCIES (lines 402-415) | Lists only `node, storage, network, rpc, mempool` as consumers. Missing from list: `cli`, `gui` (via wallet), `channels`, `updater`. All four verified as real dependents. The skill's own consumer list is incomplete. |
| INCOMPLETE | `updater/SKILL.md` DEPENDENCIES (lines 309-324) | Does not declare "used by: node, cli". Both binaries consume updater but it does not advertise this. |
| INCORRECT | `node/SKILL.md` DEPENDENCIES | States "what depends on this: integration tests, bins/gui". The gui dependency claim is WRONG. `gui/SKILL.md` declares deps as `wallet, crypto, doli-core, vdf` — the GUI binary links against the `wallet` crate at compile time. It embeds the node as a subprocess via NodeManager at runtime, which is not a Rust compile-time dependency. |
| IMPLICIT | `bridge/SKILL.md` DEPENDENCIES (lines 210-222) | Does not list `doli_core` even though bridge swap types use `Amount`, `BlockHeight`, and transaction types from core. Dependency is real but undeclared. |

---

## INDEX-WARNINGS

### @INDEX Accuracy Issues Found During Synthesis

These discrepancies between declared line ranges in skill @INDEX blocks and actual content positions will cause consuming agents to land in wrong sections. Do NOT rewrite the skill files — flag for the skill-writer agents.

| File | @INDEX Declares | Actual Position | Impact |
|------|----------------|-----------------|--------|
| `core/SKILL.md` | `STRUCTS: lines 82-175` | `## STRUCTS` header is at line 98 (~16 lines late). The DATA-FLOWS section (declared 46-80) runs longer than its declared range, shifting all subsequent sections down by ~16 lines. | Agents reading `core/SKILL.md` at offset 82 will land in DATA-FLOWS content, not STRUCTS. Use offset ~98 for STRUCTS. |
| `node/SKILL.md` | `PATTERNS: lines 397-445` | File ends at line 339. Patterns content (toxic TX purging, chain integrity scan, gossip anti-entropy, epoch-deferred mutations, liveness split) appears at lines ~320-339 with no explicit section header. The declared CONSTRAINTS range (335-395) also overruns the actual file end. | Agents requesting PATTERNS at offset 397 will receive an empty read (beyond file end). Use lines 290-339 for CONSTRAINTS+PATTERNS content. |

**Corrected ranges for keyword map:** The KEYWORD MAP in this index uses the verified actual ranges, not the declared @INDEX ranges. Agents should use the ranges from this index, not from the individual skill file @INDEX blocks, until those files are corrected.
