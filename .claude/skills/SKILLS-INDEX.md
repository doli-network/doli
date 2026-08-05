<!-- @INDEX
MANIFEST        24-64
KEYWORD-MAP     67-393
COVERAGE        397-426
CROSS-REFS      430-499
INDEX-WARNINGS  503-532
@/INDEX -->

# SKILLS-INDEX — DOLI Master Skill Manifest

Generated: 2026-07-09
Project: DOLI PoS blockchain — Rust, ~170K LOC, ~513 source files across mapped domains (defi is cross-cutting, no dedicated file count)
Domains mapped: 15 source code domains + 15 operational/workflow skills = 30 total skill files (a 16th operational skill, `doli-manager`, was referenced in this session's refresh brief but not found on disk — see INDEX-WARNINGS)
Index path: `.claude/skills/SKILLS-INDEX.md`

**Quick-grep instructions:**
- To find a skill for a concept, grep this file for the keyword (e.g., `grep "apply_block" SKILLS-INDEX.md`)
- Each row in the KEYWORD MAP gives: `skill-file : section : line-range`
- Load ONLY that section using `Read` with `offset` + `limit` — never load a full skill file unless required
- @INDEX accuracy warnings are in the INDEX-WARNINGS section at the bottom

---

## MANIFEST

### Source Code Domain Skills (15)

| Skill | Directory | Key Concepts | Entry Points | File Count | Notes |
|-------|-----------|-------------|-------------|-----------|-------|
| core | `core/SKILL.md` | consensus, validation, scheduler, epoch, transactions, activation heights, oracle (frozen), AMM foundations | validate_block, validate_transaction, DeterministicScheduler::select_producer, EpochState::derive_at_boundary, NetworkParams::load, generate_genesis_block | 109 source files | Base layer — all other crates depend on this. MAJOR DRIFT: mainnet fresh genesis reset 2026-07-08 (GENESIS_TIME=1783532348) — amm/inc_i_092/inc_i_096/large_block activation heights now 0 on mainnet; `oracle_activation_height` still `u64::MAX` (frozen) |
| node | `node/SKILL.md` | node binary, apply_block, event loop, fork recovery, block production, reorg, bootnode, disaster-recovery replay | Node::run, Node::start_network, Node::start_rpc, run_event_loop, handle_network_event, Node::new, Node::new_for_test, Node::new_for_replay | 89 source files | Top-level binary; consumes all domain crates. New: `run_bootnode()` (UDP-only lightweight discovery), `Node::new_for_replay()` (headless disaster-recovery) |
| network | `network/SKILL.md` | libp2p, GossipSub, SyncManager (8 submodules), snap sync, peer scoring, rate limiting, backpressure/watchdog | NetworkService::new, SyncManager::new, PeerScorer, RateLimiter, MemoryWatchdog | 59 source files | P2P transport + sync state machine. New: `sync/manager/` split into 8 submodules; INC-I-114 backpressure (`enqueue_or_shed`) + `MemoryWatchdog` |
| storage | `storage/SKILL.md` | RocksDB, BlockStore, StateDb, UtxoSet, ProducerSet, snapshot, archiver, ContentStore, MMR (unwired) | BlockStore::open, StateDb::open, UtxoSet::from_state_db, ProducerSet::new, ContentStore::open, UpdateState::load | 55 source files | 9 CFs BlockStore / 7 CFs StateDb; atomic batch writes. MAJOR DRIFT: `utxo_rocks.rs` ELIMINATED — `UtxoSet::RocksDb` now wraps `Arc<StateDb>` directly. New modules: metrics, mmr (CompactMmr/IncrementalStateRoot, not yet wired into apply_block), content_store, update |
| cli | `cli/SKILL.md` | CLI binary `doli`, wallet, producer, NFT, bridge, pool, channel, template, guardian, token | main (main.rs:92), RpcClient, all subcommands | 55 source files | Links `doli_core` + `channels` directly (does NOT link the `wallet` crate — has its own parallel `wallet.rs`). REMOVED since 2026-05-11: `cmd_loan.rs`/`Loan` subcommand (lending tombstoned). Oracle attestation has NO CLI subcommand yet (RPC-only, frozen pre-activation) |
| rpc | `rpc/SKILL.md` | JSON-RPC 2.0, 53 methods, HTTP POST, WebSocket, admin auth, oracle (M9-M11), DeFi health | RpcServer, RpcContext, handle_request, dispatch.rs (53-arm match) | 32 source files | HTTP on :8500 (mainnet); admin requires Bearer token from public IPs. Method count corrected 45→53 (verified against dispatch.rs 2026-07-09). New: oracle.rs/oracle_status.rs (Phase 2.1 M9-M11), defi_health.rs, repairArchiveFromPeer (post-ISSUE-174). Removed: lending.rs (tombstoned) |
| channels | `channels/SKILL.md` | payment channels, HTLC, commitment transactions, penalty, revocation, state machine | ChannelManager::new, ChannelRecord, CommitmentPair::build_local_commitment | 24 source files | Off-chain bilateral; disputes settled on-chain. Only `cmd_channel.rs` (funding/close/store/types/commitment/try_activate) is production-wired; `ChannelManager`, `ChainMonitor`, `ChannelGraph`/router, `WatchtowerSession` are library-only, unreachable in production |
| gui | `gui/SKILL.md` | Tauri 2.x desktop app, NodeManager, embedded node (spawned, not linked), wallet commands | main (main.rs:21), AppState::new, NodeManager::start | 13 source files | Depends on `wallet` crate at compile time — NOT on `bins/node` (spawns `doli-node` as a child process via NodeManager, no Rust link) |
| updater | `updater/SKILL.md` | auto-update, HardForkSchedule, VoteTracker, enforcement, watchdog, skill-tarball sync | apply_update, check_production_allowed, HardForkSchedule, VoteTracker | 13 source files | Used by both node and cli binaries (now explicitly declared in DEPENDENCIES). New: `install_skills_from_tarball` (syncs `~/.doli/skills/`), hardened `STAGED_BINARY_PATH` (closes TOCTOU symlink-swap, ISSUE-174 #7) |
| crypto | `crypto/SKILL.md` | BLAKE3, Ed25519, BLS12-381, Merkle, adaptor signatures, ECIES | Hash, KeyPair, BlsKeyPair, MerkleTree, Signature | 9 source files | Pure leaf — no doli-specific runtime deps. "Used By" rows in this skill are self-flagged [UNCLEAR] this session (rg/ripgrep unavailable) — see INDEX-WARNINGS |
| wallet | `wallet/SKILL.md` | wallet file, BIP-39, TxBuilder, RpcClient, fee calculation | Wallet, TxBuilder, RpcClient, calculate_registration_cost | 12 source files | CRITICAL: `bins/cli` does NOT depend on this crate — CLI has its own parallel wallet/tx-building copy in `bins/cli/src/wallet.rs`, kept in sync manually (GUI-NF-008). Only `bins/gui` links this crate |
| bridge | `bridge/SKILL.md` | cross-chain atomic swaps, BTC/ETH, watcher daemon, HTLC | Watcher::run, SwapRecord, SwapState | 7 source files | Largely standalone; external chain integrations. `doli_core`/`crypto` are declared Cargo deps but no direct import found in the 6 source files read this session — [UNCLEAR] possibly dead deps, verify with `cargo tree -p bridge` |
| mempool | `mempool/SKILL.md` | tx pool, CPFP, fee validation, contention diagnostics | Mempool::mainnet/testnet/new, MempoolEntry, AddTransactionResult, ContentionInfo | 6 source files | In-memory; feeds block production. Node-init wiring call site (`share_oracle_sunset_flag`/`share_active_producers_weighted`) not confirmed this session — rg unavailable |
| testing | `testing/SKILL.md` | integration tests, e2e, fuzz, simulation, benchmarks, test utilities | TestNode, Node::new_for_test, test_two_nodes_sync_basic | 30 test files | Consumes all domain crates. CRITICAL DRIFT (verified 2026-07-09): only 7 of 13 `testing/integration/` files are wired into Cargo.toml `[[test]]` entries; 6 orphaned (never run), 2 of those additionally broken against current code (compile errors) |
| defi | `defi/SKILL.md` | AMM (pool/swap/add/remove), bridge HTLC live roundtrips, channel pay+close, covenant templates (vault/escrow/htlc-payment), Phase 2.1 oracle (sunset gradient), MintAsset/NFT, activation gates | scripts/test_defi_e2e.sh (13 phases), cmd_pool.rs, cmd_bridge.rs, cmd_channel.rs, cmd_template/, lp_select.rs, pool_tx.rs, validation/amm.rs | cross-cutting | MAJOR DRIFT: mainnet fresh genesis reset 2026-07-08 moved amm/inc_i_092/inc_i_096/large_block activation heights from future-pinned values to 0 (active from genesis) — CLAUDE.md's "If You Touch → activation heights" section still documents the old pending-pin values and needs a hotfix. Only `oracle_activation_height` remains `u64::MAX` (frozen) |

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
Line ranges reflect verified actual content positions (15 domains re-validated 2026-07-09 against the orchestrator's precomputed @INDEX table; the remaining 15 operational skills are carried forward unchanged from the 2026-05-11 synthesis).

### A

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| `activate_feature` | `core/SKILL.md` | ACTIVATION-HEIGHTS | 567-611 |
| activation height | `core/SKILL.md` | ACTIVATION-HEIGHTS | 567-611 |
| AMM | `defi/SKILL.md` | CLI-SURFACE / ACTIVATION-GATES | 48-119, 121-141 |
| AMM activation gates | `defi/SKILL.md` | ACTIVATION-GATES | 121-141 |
| `amm_activation_height` | `defi/SKILL.md` | ACTIVATION-GATES | 121-141 |
| AMM balance carve-out | `defi/SKILL.md` | COVENANT-MECHANICS / INCIDENT-MAP | 166-196, 287-304 |
| adaptor signature | `crypto/SKILL.md` | ENTRY-POINTS | 11-70 |
| `apply_block` | `node/SKILL.md` | FUNCTIONS | 178-280 |
| `apply_block` data flow | `node/SKILL.md` | DATA-FLOWS | 70-152 |
| `apply_block` storage path | `storage/SKILL.md` | DATA-FLOWS | 63-126 |
| atomic swap | `bridge/SKILL.md` | DATA-FLOW | 50-67 |
| attestation bitfield | `core/SKILL.md` | DATA-FLOWS | 71-153 |
| attestation encode/decode | `core/SKILL.md` | PATTERNS | 666-772 |
| `auto_apply_from_github` | `updater/SKILL.md` | ENTRY-POINTS | 14-41 |
| auto-update | `updater/SKILL.md` | ENTRY-POINTS | 14-41 |
| auto-update implementation | `auto-update/SKILL.md` | full file | — |

### B

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| `backfillFromPeer` | `rpc/SKILL.md` | METHODS | 55-162 |
| `backfill_from_archive` | `node/SKILL.md` | ENTRY-POINTS | 13-50 |
| `bls_aggregate` | `crypto/SKILL.md` | ENTRY-POINTS | 11-70 |
| `bls_sign` / `bls_verify` | `crypto/SKILL.md` | ENTRY-POINTS | 11-70 |
| BLS12-381 | `crypto/SKILL.md` | ENTRY-POINTS | 11-70 |
| block archiver | `storage/SKILL.md` | FUNCTIONS-ARCHIVER | 558-571 |
| block production | `node/SKILL.md` | FUNCTIONS | 178-280 |
| `BlockArchiver` | `storage/SKILL.md` | ENTRY-POINTS | 21-47 |
| `BlockBuilder` | `core/SKILL.md` | OPERATIONS | 55-69 |
| `BlockStore` | `storage/SKILL.md` | FUNCTIONS-BLOCKSTORE | 343-401 |
| `BlockStore::open` | `storage/SKILL.md` | ENTRY-POINTS | 21-47 |
| bond lifecycle | `core/SKILL.md` | STRUCTS | 155-363 |
| bond withdrawal | `cli/SKILL.md` | OPERATIONS | 48-159 |
| `broadcast_block` | `network/SKILL.md` | ENTRY-POINTS | 12-53 |
| bridge | `bridge/SKILL.md` | ENTRY-POINTS | 10-36 |
| bridge CLI | `cli/SKILL.md` | OPERATIONS | 48-159 |

### C

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| `calculate_epoch_rewards` | `node/SKILL.md` | FUNCTIONS | 178-280 |
| `calculate_registration_cost` | `wallet/SKILL.md` | ENTRY-POINTS | 11-41 |
| canonical anchors | `guardian/SKILL.md` | full index | 1-30 |
| `ChainState` | `storage/SKILL.md` | ENTRY-POINTS | 21-47 |
| `ChannelManager` | `channels/SKILL.md` | ENTRY-POINTS | 14-34 |
| `ChannelRecord` | `channels/SKILL.md` | STRUCTS | 35-101 |
| `check_producer_eligibility` | `node/SKILL.md` | FUNCTIONS | 178-280 |
| `check_production_allowed` | `updater/SKILL.md` | ENTRY-POINTS | 14-41 |
| checkpoint | `guardian/SKILL.md` | full index | 1-30 |
| `ContentStore` | `storage/SKILL.md` | ENTRY-POINTS | 21-47 |
| `CompactMmr` / `IncrementalStateRoot` (unwired) | `storage/SKILL.md` | ENTRY-POINTS | 21-47 |
| `createCheckpoint` RPC | `rpc/SKILL.md` | METHODS | 55-162 |
| commitment transaction | `channels/SKILL.md` | FUNCTIONS | 117-221 |
| consensus params | `core/SKILL.md` | CONSTANTS | 444-565 |
| `ConsensusParams` | `core/SKILL.md` | CONSTANTS | 444-565 |
| CPFP | `mempool/SKILL.md` | DATA-FLOW | 116-162 |
| `createWallet` GUI | `gui/SKILL.md` | ENTRY-POINTS | 15-105 |
| cross-chain swap | `bridge/SKILL.md` | DATA-FLOW | 50-67 |

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
| `DeterministicScheduler` | `core/SKILL.md` | ENTRY-POINTS | 15-53 |
| devnet params | `network-setup/SKILL.md` | full file | — |
| `doli init` | `cli/SKILL.md` | OPERATIONS | 48-159 |
| `doli new` | `cli/SKILL.md` | OPERATIONS | 48-159 |
| `doli producer delegate` | `delegation/SKILL.md` | full file | — |
| `doli producer register` | `cli/SKILL.md` | OPERATIONS | 48-159 |
| documentation sync | `sync-docs/SKILL.md` | full file | — |

### E

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| ECIES encryption | `crypto/SKILL.md` | ENTRY-POINTS | 11-70 |
| Ed25519 | `crypto/SKILL.md` | ENTRY-POINTS | 11-70 |
| emergency halt | `guardian/SKILL.md` | full index | 1-30 |
| `enterRecoveryMode` RPC | `rpc/SKILL.md` | METHODS | 55-162 |
| epoch boundary | `core/SKILL.md` | DATA-FLOWS | 71-153 |
| `EpochState` | `core/SKILL.md` | ENTRY-POINTS | 15-53 |
| `EpochState::derive_at_boundary` | `core/SKILL.md` | ENTRY-POINTS | 15-53 |
| epoch rewards | `node/SKILL.md` | FUNCTIONS | 178-280 |
| equivocation | `node/SKILL.md` | FUNCTIONS | 178-280 |
| `execute_reorg` | `node/SKILL.md` | FUNCTIONS | 178-280 |
| explorer | `explorer/SKILL.md` | full file | — |

### F

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| faucet | `faucet/SKILL.md` | full file | — |
| fee calculation | `wallet/SKILL.md` | ENTRY-POINTS | 11-41 |
| fork detection (recovery) | `guardian/SKILL.md` | full index | 1-30 |
| FEE_TOO_LOW | `defi/SKILL.md` | KNOWN-BUGS / INCIDENT-MAP (INC-I-099) | 143-164, 287-304 |
| FundingBroadcast (stuck) | `defi/SKILL.md` | KNOWN-BUGS / INCIDENT-MAP (INC-I-097) | 143-164, 287-304 |
| fresh mainnet genesis reset (2026-07-08) | `core/SKILL.md` | CONSTANTS | 444-565 |
| `getStateRootDebug` | `rpc/SKILL.md` | METHODS | 55-162 |
| `getUtxoDiff` | `rpc/SKILL.md` | METHODS | 55-162 |
| fork recovery | `node/SKILL.md` | FUNCTIONS | 178-280 |
| `fork_recovery.rs` | `node/SKILL.md` | FUNCTIONS | 178-280 |
| `ForkBlock` | `node/SKILL.md` | DATA-FLOWS | 70-152 |

### G

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| genesis block | `core/SKILL.md` | ENTRY-POINTS | 15-53 |
| `genesis_hash` | `core/SKILL.md` | ENTRY-POINTS | 15-53 |
| `generate_genesis_block` | `core/SKILL.md` | ENTRY-POINTS | 15-53 |
| `GENESIS_TIME` (1_783_532_348, changed 2026-07-08) | `core/SKILL.md` | CONSTANTS | 444-565 |
| `getBlockByHash` | `rpc/SKILL.md` | METHODS | 55-162 |
| `getBlockByHeight` | `rpc/SKILL.md` | METHODS | 55-162 |
| `getBlockData` | `rpc/SKILL.md` | METHODS | 55-162 |
| `getBlockRaw` | `rpc/SKILL.md` | METHODS | 55-162 |
| `getChainInfo` | `rpc/SKILL.md` | METHODS | 55-162 |
| `getEpochInfo` | `rpc/SKILL.md` | METHODS | 55-162 |
| `getProducers` | `rpc/SKILL.md` | METHODS | 55-162 |
| `getStateRootDebug` | `rpc/SKILL.md` | METHODS | 55-162 |
| `getStateSnapshot` | `rpc/SKILL.md` | METHODS | 55-162 |
| `getUtxoDiff` | `rpc/SKILL.md` | METHODS | 55-162 |
| `getUtxos` | `rpc/SKILL.md` | METHODS | 55-162 |
| GossipSub | `network/SKILL.md` | ENTRY-POINTS | 12-53 |
| GUI desktop app | `gui/SKILL.md` | ENTRY-POINTS | 15-105 |

### H

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| `handle_new_block` | `node/SKILL.md` | FUNCTIONS | 178-280 |
| hard fork | `updater/SKILL.md` | HARDFORK-SCHEDULE | 282-303 |
| `HardForkSchedule` | `updater/SKILL.md` | HARDFORK-SCHEDULE | 282-303 |
| Hetzner VPS | `hetzner/SKILL.md` | full file | — |
| HTLC channels | `channels/SKILL.md` | STRUCTS | 35-101 |
| HTLC bridge | `bridge/SKILL.md` | ENTRY-POINTS | 10-36 |

### I

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| `InFlightHtlc` | `channels/SKILL.md` | STRUCTS | 35-101 |
| integration tests | `testing/SKILL.md` | INTEGRATION-TESTS | 16-142 |

### K

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| `KeyPair` | `crypto/SKILL.md` | ENTRY-POINTS | 11-70 |

### L

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| libp2p | `network/SKILL.md` | ENTRY-POINTS | 12-53 |
| loan CLI (REMOVED 2026-05-11, lending tombstoned) | `cli/SKILL.md` | ENTRY-POINTS | 11-46 |

### M

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| MintAsset | `defi/SKILL.md` | CLI-SURFACE | 48-119 |
| MPTX007 (covenant) | `defi/SKILL.md` | COVENANT-MECHANICS / INCIDENT-MAP | 166-196, 287-304 |
| MPTX008 (balance) | `defi/SKILL.md` | COVENANT-MECHANICS / INCIDENT-MAP (INC-I-096) | 166-196, 287-304 |
| `MaintainerState` | `storage/SKILL.md` | ENTRY-POINTS | 21-47 |
| mainnet deploy | `mainnet/SKILL.md` | full file | — |
| mainnet recovery | `guardian/SKILL.md` | full index | 1-30 |
| `Mempool` | `mempool/SKILL.md` | ENTRY-POINTS | 11-99 |
| `MempoolEntry` | `mempool/SKILL.md` | ENTRY-POINTS | 11-99 |
| `MerkleTree` | `crypto/SKILL.md` | ENTRY-POINTS | 11-70 |
| mint asset | `cli/SKILL.md` | OPERATIONS | 48-159 |

### N

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| `NetworkParams` | `core/SKILL.md` | ENTRY-POINTS | 15-53 |
| `NetworkParams::load` | `core/SKILL.md` | ENTRY-POINTS | 15-53 |
| `NetworkService` | `network/SKILL.md` | ENTRY-POINTS | 12-53 |
| `NetworkService::new` | `network/SKILL.md` | ENTRY-POINTS | 12-53 |
| `Node::new` | `node/SKILL.md` | ENTRY-POINTS | 13-50 |
| `Node::new_for_test` | `testing/SKILL.md` | TEST-UTILITIES | 301-349 |
| `NodeManager` GUI | `gui/SKILL.md` | STRUCTS | 274-304 |
| NFT | `cli/SKILL.md` | OPERATIONS | 48-159 |

### O

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| `oracle_activation_height` (frozen, `u64::MAX` all networks) | `core/SKILL.md` | ACTIVATION-HEIGHTS | 567-611 |
| oracle frozen / oracle sunset | `core/SKILL.md` | ENTRY-POINTS | 15-53 |
| `OracleSunsetState` / sunset gradient | `defi/SKILL.md` | ENTRY-POINTS | 20-46 |
| `OraclePrice` UTXO | `core/SKILL.md` | ENTRY-POINTS | 15-53 |
| `getOracleStatus` | `rpc/SKILL.md` | METHODS | 55-162 |

### P

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| `pauseProduction` | `rpc/SKILL.md` | METHODS | 55-162 |
| payment channel | `channels/SKILL.md` | ENTRY-POINTS | 14-34 |
| payment channel CLI | `cli/SKILL.md` | OPERATIONS | 48-159 |
| payment channel (DeFi flows) | `defi/SKILL.md` | CLI-SURFACE / KNOWN-BUGS | 48-119, 143-164 |
| pool (AMM) | `defi/SKILL.md` | CLI-SURFACE / TX-CONSTRUCTION | 48-119, 198-247 |
| `pool_id` | `defi/SKILL.md` | CLI-SURFACE / COVENANT-MECHANICS | 48-119, 166-196 |
| `pool_tx::sign_with_covenant_witnesses` | `defi/SKILL.md` | TX-CONSTRUCTION | 198-247 |
| pool covenant witness | `defi/SKILL.md` | COVENANT-MECHANICS / TX-CONSTRUCTION | 166-196, 198-247 |
| LP shares | `defi/SKILL.md` | CLI-SURFACE / COVENANT-MECHANICS | 48-119, 166-196 |
| `lp_select::select_lp_share_utxos` | `defi/SKILL.md` | KNOWN-BUGS (INC-I-095) | 143-164 |
| ERRTX-HTLC001 | `defi/SKILL.md` | KNOWN-BUGS / INCIDENT-MAP (INC-I-098) | 143-164, 287-304 |
| covenant witness | `defi/SKILL.md` | COVENANT-MECHANICS / TX-CONSTRUCTION | 166-196, 198-247 |
| covenant template (vault/escrow) | `defi/SKILL.md` | CLI-SURFACE / KNOWN-BUGS | 48-119, 143-164 |
| ECIES (NFT transfer) | `defi/SKILL.md` | KNOWN-BUGS | 143-164 |
| bridge HTLC (live) | `defi/SKILL.md` | CLI-SURFACE / LIVE-TEST-HARNESS | 48-119, 249-285 |
| `test_defi_e2e.sh` | `defi/SKILL.md` | LIVE-TEST-HARNESS | 249-285 |
| oracle (Phase 2.1) | `defi/SKILL.md` | CLI-SURFACE / ACTIVATION-GATES | 48-119, 121-141 |
| `verify_amm_conservation` | `defi/SKILL.md` | COVENANT-MECHANICS / INCIDENT-MAP (INC-I-096) | 166-196, 287-304 |
| peer scoring | `network/SKILL.md` | ENTRY-POINTS | 12-53 |
| `PeerScorer` | `network/SKILL.md` | ENTRY-POINTS | 12-53 |
| penalty transaction | `channels/SKILL.md` | FUNCTIONS | 117-221 |
| `ProducerSet` | `storage/SKILL.md` | FUNCTIONS-PRODUCERSET | 522-543 |
| `ProducerSet::new` | `storage/SKILL.md` | ENTRY-POINTS | 21-47 |
| producer onboarding | `producers/SKILL.md` | full index | — |
| producer registration | `cli/SKILL.md` | OPERATIONS | 48-159 |
| `ProtocolActivation` | `core/SKILL.md` | ACTIVATION-HEIGHTS | 567-611 |

### R

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| `RateLimiter` | `network/SKILL.md` | ENTRY-POINTS | 12-53 |
| recover chain state | `node/SKILL.md` | ENTRY-POINTS | 13-50 |
| recovery mode | `guardian/SKILL.md` | full index | 1-30 |
| restore wallet | `cli/SKILL.md` | OPERATIONS | 48-159 |
| revocation store | `channels/SKILL.md` | STRUCTS | 35-101 |
| `RevokeDelegationData` | `delegation/SKILL.md` | full file | — |
| rewards calculation | `node/SKILL.md` | FUNCTIONS | 178-280 |
| `rollback_one_block` | `node/SKILL.md` | FUNCTIONS | 178-280 |
| RocksDB column families | `storage/SKILL.md` | COLUMN-FAMILIES | 280-342 |
| `RpcClient` | `wallet/SKILL.md` | ENTRY-POINTS | 11-41 |
| RPC methods all 53 | `rpc/SKILL.md` | METHODS | 55-162 |
| `run_event_loop` | `node/SKILL.md` | ENTRY-POINTS | 13-50 |
| `run_node` | `node/SKILL.md` | ENTRY-POINTS | 13-50 |

### S

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| scheduler | `core/SKILL.md` | ENTRY-POINTS | 15-53 |
| seed guardian | `guardian/SKILL.md` | full index | 1-30 |
| `select_producer` | `core/SKILL.md` | ENTRY-POINTS | 15-53 |
| `sendTransaction` | `rpc/SKILL.md` | METHODS | 55-162 |
| serialization formats | `storage/SKILL.md` | SERIALIZATION | 733-774 |
| `sign_release_hash` | `updater/SKILL.md` | ENTRY-POINTS | 14-41 |
| skill creation | `skill-creator/SKILL.md` | full file | — |
| snap sync | `network/SKILL.md` | DATA-FLOW | 66-77 |
| `StateDb` | `storage/SKILL.md` | FUNCTIONS-STATEDB | 402-488 |
| `StateDb::open` | `storage/SKILL.md` | ENTRY-POINTS | 21-47 |
| `StateSnapshot` | `storage/SKILL.md` | FUNCTIONS-SNAPSHOT | 544-557 |
| state root | `storage/SKILL.md` | FUNCTIONS-SNAPSHOT | 544-557 |
| state root debug | `doli-network/SKILL.md` | full file | — |
| `STAKER_REWARD_PCT` | `delegation/SKILL.md` | full file | — |
| `SyncManager` | `network/SKILL.md` | ENTRY-POINTS | 12-53 |
| `SyncManager::new` | `network/SKILL.md` | ENTRY-POINTS | 12-53 |

### T

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| Tauri desktop | `gui/SKILL.md` | ENTRY-POINTS | 15-105 |
| test node | `testing/SKILL.md` | TEST-UTILITIES | 301-349 |
| `TestNode` | `testing/SKILL.md` | TEST-UTILITIES | 301-349 |
| transaction builder | `wallet/SKILL.md` | ENTRY-POINTS | 11-41 |
| transaction types | `core/SKILL.md` | STRUCTS | 155-363 |
| `try_produce_block` | `node/SKILL.md` | FUNCTIONS | 178-280 |
| `TxBuilder` | `wallet/SKILL.md` | ENTRY-POINTS | 11-41 |
| TxType 24 (constructible; was 27/30 pre-tombstone) | `core/SKILL.md` | CONSTANTS | 444-565 |

### U

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| update governance | `updater/SKILL.md` | DATA-FLOWS | 304-382 |
| `UpdateState` | `storage/SKILL.md` | ENTRY-POINTS | 21-47 |
| `UtxoSet` | `storage/SKILL.md` | FUNCTIONS-UTXO | 489-521 |
| `UtxoSet::new` / `UtxoSet::from_state_db` | `storage/SKILL.md` | ENTRY-POINTS | 21-47 |
| `utxo_rocks.rs` ELIMINATED (RocksDb variant now wraps `Arc<StateDb>`) | `storage/SKILL.md` | ENTRY-POINTS | 21-47 |

### V

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| `validate_block` | `core/SKILL.md` | ENTRY-POINTS | 15-53 |
| `validate_transaction` | `core/SKILL.md` | ENTRY-POINTS | 15-53 |
| `verifyChainIntegrity` | `rpc/SKILL.md` | METHODS | 55-162 |
| vote tracker | `updater/SKILL.md` | STRUCTS | 59-98 |
| `VoteTracker` | `updater/SKILL.md` | STRUCTS | 59-98 |

### W

| Keyword / Concept | Skill File | Section | Lines |
|-------------------|-----------|---------|-------|
| wallet file format | `wallet/SKILL.md` | ENTRY-POINTS | 11-41 |
| wallet management | `cli/SKILL.md` | OPERATIONS | 48-159 |
| `Wallet` | `wallet/SKILL.md` | ENTRY-POINTS | 11-41 |
| `Watcher::run` | `bridge/SKILL.md` | ENTRY-POINTS | 10-36 |
| WebSocket subscriptions | `rpc/SKILL.md` | ENTRY-POINTS | 12-31 |
| wipe protocol | `producers/SKILL.md` | full index | — |

---

## COVERAGE

### Source Domain Coverage

| Status | Domain | File Count | Skill File | Notes |
|--------|--------|-----------|-----------|-------|
| COVERED | core | 109 | `core/SKILL.md` | Complete; @INDEX orchestrator-validated 2026-07-09 |
| COVERED | node | 89 | `node/SKILL.md` | Complete; @INDEX orchestrator-validated 2026-07-09 |
| COVERED | network | 59 | `network/SKILL.md` | Complete |
| COVERED | storage | 55 | `storage/SKILL.md` | Complete; most detailed skill (774 lines, 16 sections) |
| COVERED | cli | 55 | `cli/SKILL.md` | Complete |
| COVERED | rpc | 32 | `rpc/SKILL.md` | Complete; 53 methods documented (corrected from 45) |
| COVERED | channels | 24 | `channels/SKILL.md` | Complete; most library code (ChannelManager, ChainMonitor, router, watchtower) unreachable in production — see CROSS-REFS |
| COVERED | gui | 13 | `gui/SKILL.md` | Complete |
| COVERED | updater | 13 | `updater/SKILL.md` | Complete |
| COVERED | crypto | 9 | `crypto/SKILL.md` | Complete; reverse-dependency rows self-flagged [UNCLEAR] this session (rg unavailable) |
| COVERED | wallet | 12 | `wallet/SKILL.md` | Complete |
| COVERED | bridge | 7 | `bridge/SKILL.md` | Complete; doli_core/crypto dependency now flagged possibly-dead (no import found) — reversed from prior "implicit real dependency" finding |
| COVERED | mempool | 6 | `mempool/SKILL.md` | Complete |
| COVERED | testing | 30 | `testing/SKILL.md` | Complete; CRITICAL DRIFT — 6/13 integration test files orphaned (not in Cargo.toml), 2 additionally broken |
| COVERED | defi | cross-cutting | `defi/SKILL.md` | Complete; MAJOR DRIFT — mainnet genesis reset 2026-07-08 moved all non-oracle DeFi activation heights to 0 |

**No source domain coverage gaps.** All 15 mapped source domains have skill files.

### Operational Skill Coverage

| Status | Domain | Notes |
|--------|--------|-------|
| COVERED | doli-network, producers, delegation, faucet, guardian, mainnet, auto-update, network-setup, testnet-deploy, release, explorer, hetzner, sync-docs, test-script, skill-creator | 15 skills, untouched this session, entries preserved verbatim from the 2026-05-11 synthesis |
| UNRESOLVED | doli-manager | Referenced in this session's refresh brief as a 16th operational skill; no `.claude/skills/doli-manager/SKILL.md` found on disk (verified via direct Read — file does not exist). Either the skill was never created, or the brief's count is stale. Needs resolution before the next synthesis claims 16 operational skills. |

---

## CROSS-REFS

### Dependency Chain (bottom to top)

```
crypto (pure leaf — no doli deps)
  └─> core (consensus types, validation, scheduler, epoch state, oracle, AMM)
        ├─> storage (RocksDB persistence, BlockStore, StateDb, UtxoSet, ProducerSet, ContentStore)
        ├─> mempool (tx pool — also depends on storage)
        ├─> channels (payment channels)
        ├─> network (P2P transport + SyncManager)
        └─> rpc (JSON-RPC server — also depends on storage, network, mempool)
              └─> node (top-level binary — consumes all above)
                    └─> testing (consumes all for integration tests)

crypto ──> wallet ──> gui (bins/gui) [via wallet crate, NOT via bins/node — spawns node as child process]
core   ──> updater ──> node (auto-update governance; both directions now explicitly declared)
core   ──> updater ──> cli
core, crypto, channels ──> cli (bins/cli links doli_core + crypto + channels directly; does NOT link the `wallet` crate — has its own parallel wallet.rs)
bridge (standalone — uses DOLI node via HTTP RPC, not as a library dep; doli_core/crypto Cargo deps possibly unused)
```

### Verified Adjacency Table

| From Skill | To Skill | Relationship | Verified |
|-----------|---------|-------------|---------|
| `node` | `core` | bins/node depends on doli_core for validation, scheduler, epoch state, oracle | YES |
| `node` | `storage` | Node::new opens BlockStore + StateDb + ProducerSet + ChainState | YES |
| `node` | `network` | Node drives NetworkService by polling next_event() | YES |
| `node` | `rpc` | Node starts RPC server via start_rpc() (56-method RpcContext) | YES |
| `node` | `updater` | Node spawns update service; calls check_production_allowed; updater now declares this reverse dependency too | YES |
| `node` | `mempool` | Node feeds mempool for block production; purges on error | YES |
| `node` | `crypto` | Node uses Hash, KeyPair directly across many paths | YES |
| `cli` | `core` | bins/cli links doli_core directly for tx building (bond/pool/template construction) | YES |
| `cli` | `channels` | bins/cli's cmd_channel.rs is the ONLY production consumer of the channels crate | YES |
| `cli` | `updater` | CLI handles update/governance commands via updater crate | YES |
| `cli` | `crypto` | CLI uses Hash, Address for key ops | YES |
| `cli` | `storage` | CLI opens storage for some local operations (recover, checkpoint) | YES |
| `cli` | `wallet` | CORRECTED: bins/cli does NOT depend on the `wallet` crate (no Cargo.toml entry) — it has its own parallel implementation in `bins/cli/src/wallet.rs`, manually kept in sync (GUI-NF-008) | YES — CORRECTED THIS SESSION |
| `gui` | `wallet` | bins/gui uses wallet crate (declared in gui/SKILL.md DEPENDENCIES, Cargo.toml:16) | YES |
| `gui` | `crypto` | GUI uses crypto for key operations | YES |
| `gui` | `core` | GUI's cmd_producer/register.rs builds Registration tx directly with doli_core types (bypasses wallet::TxBuilder) | YES |
| `rpc` | `core` | RPC uses doli_core types + oracle module throughout | YES |
| `rpc` | `storage` | RPC reads BlockStore + StateDb + ProducerSet | YES |
| `rpc` | `network` | RPC exposes sync/peer state via network callbacks | YES |
| `rpc` | `mempool` | RPC exposes mempool state + handles tx submission | YES |
| `network` | `core` | SyncManager + status protocol use doli_core types | YES |
| `storage` | `core` | Storage uses Block, Transaction, Amount, oracle types from doli_core | YES |
| `mempool` | `core` | Mempool uses ConsensusParams, transaction/oracle/AMM types | YES |
| `mempool` | `storage` | Mempool reads UtxoSet for validation | YES |
| `channels` | `core` | Channels use Amount, BlockHeight, transaction types | YES |
| `channels` | `crypto` | Channels use Hash, KeyPair, adaptor signatures | YES |
| `updater` | `core` | Updater uses NetworkParams for version enforcement | YES |
| `updater` | `crypto` | Updater uses Ed25519 for release signature verification | YES |
| `testing` | all | testing/SKILL.md consumes all domain crates via Node::new_for_test | YES |
| `bridge` | `core`/`crypto` | Declared as Cargo deps but NO direct import found in the 6 source files read this session | UNCLEAR — DOWNGRADED THIS SESSION |
| `crypto` | core, storage, network, rpc, mempool, channels, bridge, node, cli (reverse) | All 9 "Used By" rows in crypto/SKILL.md are self-flagged [UNCLEAR] — rg/ripgrep unavailable this session, inferred from Cargo.toml membership only | UNCLEAR — NOT RE-VERIFIED THIS SESSION |

### Asymmetry Flags — Issues Requiring Skill File Corrections

| Severity | Skill File | Issue |
|---------|-----------|-------|
| IMPROVED-BUT-INCOMPLETE | `core/SKILL.md` DEPENDENCIES (613-629) | Now lists `updater` as a consumer (improvement over prior synthesis). Still missing: `cli`, `gui` (via core directly, not just wallet), `channels`. |
| INCORRECT | `node/SKILL.md` DEPENDENCIES (282-296) | Still states "what depends on this: ... bins/gui (GUI producer registration)". `gui/SKILL.md` DEPENDENCIES confirms gui does NOT link `bins/node` as a Rust dependency — it spawns `doli-node` as a child process (runtime, not compile-time). Same drift flagged in the 2026-05-11 synthesis, not yet fixed by the skill-writer. |
| RESOLVED | `updater/SKILL.md` DEPENDENCIES (383-405) | Previously flagged as not declaring "used by: node, cli" — NOW FIXED. This session's refresh explicitly lists both consumers with call-site detail (caveat: exact line numbers not re-verified, rg unavailable). |
| REVERSED | `bridge/SKILL.md` DEPENDENCIES (69-82) | Previously flagged as an undeclared-but-real `doli_core` dependency. This session's skill-writer found the OPPOSITE: `doli_core`/`crypto` ARE declared in Cargo.toml but NO import was found in any of the 6 source files read — now flagged [UNCLEAR], possibly dead dependencies. Needs `cargo tree -p bridge` or a build check to resolve. |
| NEW-CAVEAT | `channels/SKILL.md` DEPENDENCIES (312-333) | "Used By" table asserts no other crate (node/rpc/mempool) depends on channels, but explicitly notes this was NOT exhaustively verified — Grep/Glob failed (`rg` missing) this session. |
| NEW-CAVEAT | `crypto/SKILL.md` DEPENDENCIES (114-145) | Entire "Used By" table (9 rows) is self-flagged [UNCLEAR] — inferred from Cargo.toml workspace membership only, not verified by search, due to missing `rg` this session. |
| NEW-CAVEAT | `mempool/SKILL.md` DEPENDENCIES (164-182) | Node-init wiring call site for `share_oracle_sunset_flag`/`share_active_producers_weighted` not confirmed — `rg` unavailable. |
| NEW-CAVEAT | `bridge/SKILL.md` DEPENDENCIES (69-82) | Two "Used By" CLI call sites (`doli bridge-refund`, `cmd_bridge_status`) could not be located this session — `rg` unavailable. |

---

## INDEX-WARNINGS

### @INDEX Accuracy — This Session

All 15 refreshed skills' @INDEX blocks were validated by the orchestrator against actual section headings before this synthesis (precomputed section→line-range table). No @INDEX drift found this session for: core, node, network, storage, cli, rpc, channels, gui, updater, crypto, wallet, bridge, mempool, testing, defi. **The two @INDEX warnings from the 2026-05-11 synthesis (core STRUCTS offset, node PATTERNS overrun) are RESOLVED** — both files were fully regenerated this session with corrected, validated @INDEX blocks.

### New Warnings — Missing `rg` (ripgrep) This Session

The `rg` binary was unavailable to all 15 skill-writer agents this session (`ENOENT posix_spawn 'rg'`), forcing Grep/Glob fallback failures. This degraded confidence specifically on **reverse-dependency ("Used By") rows**, which require cross-workspace search rather than single-file reads. Flagged files/rows are candidates for a follow-up grep pass once ripgrep is restored:

| File | Section | What's Unverified |
|------|---------|--------------------|
| `crypto/SKILL.md` | DEPENDENCIES (114-145) | All 9 "Used By" rows (core, storage, network, rpc, mempool, channels, bridge, node, cli) — inferred from Cargo.toml membership only |
| `bridge/SKILL.md` | ENTRY-POINTS (10-36) / DEPENDENCIES (69-82) | CLI consumer call sites (`doli bridge-refund`, `cmd_bridge_status`) not located; `doli_core`/`crypto` real-usage status unclear |
| `channels/SKILL.md` | DEPENDENCIES (312-333) | "Used by: none found" for node/rpc/mempool not exhaustively verified |
| `updater/SKILL.md` | DEPENDENCIES (383-405) | node/cli consumer relationship confirmed structurally, but exact call-site line numbers not re-verified |
| `mempool/SKILL.md` | DEPENDENCIES (164-182) | Node-init wiring call site for oracle-sunset/weighted-producer sharing not located |

### Coverage Gap — `doli-manager`

This session's refresh brief listed 16 operational/workflow skills including `doli-manager`, but no `.claude/skills/doli-manager/SKILL.md` exists on disk (confirmed via direct Read attempt — "File does not exist"). Preserved the 15 operational skills that DO exist (unchanged, verbatim). Treat `doli-manager` as an open item: either create the skill or correct the count in the next refresh brief.

### Code-Reality Drift Flagged By Skill-Writers (not @INDEX issues, but worth surfacing)

- `testing/SKILL.md` — 6 of 13 files in `testing/integration/` are NOT wired into `Cargo.toml` `[[test]]` entries (never compiled/run); 2 of those 6 would fail to compile if re-added (`malicious_peer.rs` missing `presence_root` field, `attack_reorg_test.rs` calling `create_transfer` with a stale 7-arg signature). Do not cite these 6 files as passing regression coverage.
- `defi/SKILL.md` — mainnet fresh genesis reset (2026-07-08, commits `61218e90`/`db05c2c5`) moved `amm_activation_height`, `inc_i_092_activation_height`, `inc_i_096_activation_height`, `large_block_activation_height` from future-pinned heights to `0` on mainnet. `CLAUDE.md`'s "If You Touch → activation heights" section still documents the pre-reset pending-pin values (e.g. `amm_activation_height=367_660`) — **CLAUDE.md needs a hotfix**, per the defi skill's own note. Only `oracle_activation_height` remains `u64::MAX` (frozen).
- `storage/SKILL.md` — `utxo_rocks.rs` was eliminated; `UtxoSet::RocksDb` now wraps `Arc<StateDb>` directly via `UtxoSet::from_state_db()`. Any doc/skill still referencing a standalone `utxo_rocks.rs` module is stale.
- `rpc/SKILL.md` — method count corrected 45→53 (verified against `dispatch.rs` match arms 2026-07-09).
- `cli/SKILL.md` — `cmd_loan.rs`/`Loan` subcommand tree no longer exists (lending tombstoned, B.1). Any reference to `doli loan *` commands is stale.
