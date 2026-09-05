# rpc — DOLI JSON-RPC API
<!-- @INDEX
ENTRY-POINTS: lines 12-31
OPERATIONS: lines 33-53
METHODS: lines 55-162
DATA-FLOWS: lines 164-197
DEPENDENCIES: lines 199-221
CONSTRAINTS: lines 223-257
PATTERNS: lines 259-314
-->

## ENTRY-POINTS

**Crate**: `crates/rpc/` — 32 `.rs` files (grew from 27; +oracle.rs/oracle_status.rs/tests_oracle*.rs, +defi_health.rs/tests_defi_health.rs, -lending.rs [tombstoned]).
**Transport**: HTTP POST `/` — JSON-RPC 2.0 envelope (`jsonrpc`, `method`, `params`, `id`).
**WebSocket**: `GET /ws` — subscribe to real-time events (`new_block`, `new_tx`). Max 100 concurrent connections.
**Max request body**: 2 MB (covers NFT hex-encoded data) — `crates/rpc/src/server.rs:27`.
**Method count**: **53 RPC methods** (verified against `dispatch.rs` match arms 2026-07-09 — old skill said 45).
**Server struct**: `crates/rpc/src/server.rs:86` `RpcServer { config, context, ws_sender }`
**Config struct**: `crates/rpc/src/server.rs:53` `RpcServerConfig { listen_addr, enable_cors, allowed_origins, admin_token, trusted_proxies }`
**Default port**: 8500 (mainnet), network-specific via `NetworkParams::load(network).default_rpc_port`.
**Dispatch entry**: `crates/rpc/src/methods/dispatch.rs:12` — `RpcContext::handle_request()` (77-arm match).
**Context struct**: `crates/rpc/src/methods/context.rs:41` — `RpcContext` holds all shared state.
**Types module**: `crates/rpc/src/types/` (was single file) — split into `block.rs`, `chain.rs`, `producer.rs`, `protocol.rs` (JSON-RPC envelope: `crates/rpc/src/types/protocol.rs:10,24`).

**Admin auth** (`crates/rpc/src/server.rs:31-49, 189-241`):
- Admin methods require `Authorization: Bearer <token>` from public IPs.
- Loopback (127.x) and RFC-1918 private IPs are always trusted — no token needed.
- Constant-time comparison (`constant_time_eq`, `server.rs:244`) prevents timing side-channels.
- `trusted_proxies: Vec<IpAddr>` (ISSUE-174 fix): when the immediate TCP peer is a configured trusted reverse proxy, `X-Real-IP`/`X-Forwarded-For` (leftmost) is resolved to the real client IP before the trust check (`resolve_client_ip`, `server.rs:269`). Default empty = headers ignored, peer IP used directly — an untrusted caller cannot forge trust by setting its own XFF.
- Admin set (13 methods, `server.rs:31-49`): `pauseProduction`, `resumeProduction`, `createCheckpoint`, `pruneBlocks`, `backfillFromPeer`, `enterRecoveryMode`, `exitRecoveryMode`, `bridgeFromArchive`, `getUtxoDiff`, `getStateSnapshot`, `getStateRootDebug`, `verifyChainIntegrity`, `repairArchiveFromPeer` (added post-ISSUE-174 — outbound HTTP fetcher, SSRF risk if left unauthenticated).

## OPERATIONS

| Task | Steps | Commands/Functions | Inputs | Success |
|------|-------|--------------------|--------|---------|
| Query chain tip | 1. POST `getChainInfo` | `getChainInfo` | none | `{network,version,best_hash,best_height,best_slot,genesis_hash,reward_pool_balance}` |
| Look up a block | 1. POST `getBlockByHeight` (preferred) or `getBlockByHash` | `getBlockByHeight`, `getBlockByHash` | height or hash | `BlockResponse`; **by-hash always returns height=0** (no reverse index) |
| Submit a transaction | 1. Build+sign tx client-side 2. hex-encode 3. POST `sendTransaction` | `sendTransaction` | `{tx:"<hex>"}` | tx hash string, or `{hash, warnings:[POOL_CONTENTION]}` on AMM pool contention |
| Check a balance | 1. Resolve address (bech32m or hex) 2. POST `getBalance` | `getBalance` | `{address}` | `{confirmed,unconfirmed,immature,bonded,total}` |
| List spendable UTXOs | 1. POST `getUtxos` with `spendable_only` | `getUtxos` | `{address, spendable_only}` | array of `UtxoResponse` incl. pending mempool outputs |
| Inspect a producer | 1. POST `getProducer` (single) or `getProducers` (all) | `getProducer`, `getProducers` | `{public_key}` or `{active_only}` | `ProducerResponse` (status, bonds, delegations) |
| Inspect bond vesting | 1. POST `getBondDetails` | `getBondDetails` | `{public_key}` | per-bond FIFO list with penalty_pct/vested/maturation_slot |
| Authenticate an admin call | 1. Call from loopback/private IP (no token needed) OR 2. Set `Authorization: Bearer <admin_token>` header from a public IP | any method in `ADMIN_METHODS` | admin_token configured server-side | 200 OK; else `-32008 unauthorized` |
| Halt/resume production (Guardian) | 1. `pauseProduction` (admin) 2. verify via `getGuardianStatus` 3. `resumeProduction` (admin) | `pauseProduction`, `resumeProduction`, `getGuardianStatus` | none | `{status:"paused"/"resumed"}` |
| Diagnose state divergence across nodes | 1. `getStateRootDebug` on all nodes, compare `stateRoot`/`csHash`/`utxoHash`/`psHash` 2. if `utxoHash` differs: `getUtxoDiff` (admin) full-dump on A, feed `referenceHashes` to B | `getStateRootDebug`, `getUtxoDiff` | none / `{referenceHashes}` | per-component hash comparison narrows divergence to CS/UTXO/PS |
| Repair a gapped/forked chain | 1. `enterRecoveryMode` (admin) 2. `bridgeFromArchive` (admin, optional `force`) 3. `backfillFromPeer` (admin) for remaining gaps 4. `verifyChainIntegrity` 5. `exitRecoveryMode` (admin) + restart node | `enterRecoveryMode`, `bridgeFromArchive`, `backfillFromPeer`, `verifyChainIntegrity`, `exitRecoveryMode` | archive dir / peer RPC URL | `verifyChainIntegrity` returns `complete:true`, `missingCount:0` |
| Take a hot backup | 1. `createCheckpoint` (admin), optional relative path | `createCheckpoint` | optional `[path]` | RocksDB hard-link checkpoint under `{data_dir}/checkpoints/h{height}-{ts}/` |
| Reclaim disk space | 1. `pruneBlocks` (admin) with `keep_last_n` (min 2000, clamped) | `pruneBlocks` | `[keep_last_n]` | pruned count + `archive_verified` flag |
| Query an AMM pool | 1. `getPoolInfo`/`getPoolList` for state 2. `getPoolPrice` for spot/TWAP 3. `getSwapQuote` to simulate | `getPoolInfo`, `getPoolList`, `getPoolPrice`, `getSwapQuote` | `{poolId}` | reserves/price/simulated swap output — **frozen pre-`amm_activation_height`** |
| Submit a governance vote | 1. Producer signs `"version:vote:timestamp"` Ed25519 2. `submitVote` | `submitVote` | `{vote:{producer_id,version,vote,timestamp,signature}}` | `{status:"submitted"}`; broadcasts to network |
| Query oracle state (Phase 2.1, frozen) | 1. `getOraclePrice` for current median 2. `getOracleAttestations` for audit trail 3. `getOracleStatus` for health/sunset | `getOraclePrice`, `getOracleAttestations`, `getOracleStatus` | `{pair_id}` / `{epoch,pair_id}` / none | `null`/`active:false` pre-activation (`oracle_activation_height=u64::MAX` on every network) |
| Get DeFi bond/TVL health | 1. `getDefiHealthMetric` | `getDefiHealthMetric` | none | `{totalActiveBonds,maxPoolTvl,bondToTvlRatio,status}` — monitoring only, never rejects |

## METHODS

53 methods total (verified against `crates/rpc/src/methods/dispatch.rs` match arms).

### Block Methods (`crates/rpc/src/methods/block.rs`)

**`getBlockByHash`** (line 14) — `{hash:"<64-hex>"}` → `BlockResponse`, **height always 0** (no reverse hash→height index; use `getBlockByHeight`).
**`getBlockByHeight`** (line 38) — `{height:<u64>}` or `[height]` → `BlockResponse` with correct height.
**`getBlockRaw`** (line 61) — `{height:<u64>}` → `{block:"<base64>", blake3:"<hex>", height}`. Used by backfill (bincode + BLAKE3 checksum).
**`getBlockData`** (line 90) — `{hash, output_index}` → `{data:"<base64>", size, blob_hash, output_type}`. Retrieves `extra_data` from a specific output (NFT/document content).

### Transaction Methods (`crates/rpc/src/methods/transaction.rs`)

**`getTransaction`** (line 16) — `{hash}` → `TransactionResponse`. Checks mempool first, then tx index → block lookup.
**`getNftByTokenId`** (line 104) — `["<tokenId_hex>"]` or `{tokenId}` → NFT owner/metadata. Scans UTXO set.
**`sendTransaction`** (line 165) — `{tx:"<hex>"}` → tx hash hex, or `{hash, warnings}` on pool contention. State-only txs bypass UTXO fee accounting. Structured errors: `INVALID_HEX`, `DESERIALIZE_FAILED`, `TX_ALREADY_EXISTS`, `MEMPOOL_FULL`, plus `MempoolError::to_structured_json()` for other invalid-tx reasons.

### Balance & UTXO Methods (`crates/rpc/src/methods/balance.rs`)

**`getBalance`** (line 12) — `{address}` → `{confirmed,unconfirmed,immature,bonded,total}`. Address: bech32m (`doli1…`/`tdoli1…`/`ddoli1…`) or 64-hex pubkey_hash. Applies `coinbase_maturity`.
**`getUtxos`** (line 57) — `{address, spendable_only}` → `UtxoResponse[]`, includes pending mempool outputs for chained-tx support.
- **Output types (14, current)**: `normal`, `bond`, `multisig`, `hashlock`, `htlc`, `vesting`, `nft`, `fungibleAsset`, `bridgeHtlc`, `pool`, `lpShare`, `zkRollup`, `encryptedContent`, `oraclePrice`. **DRIFT FIX**: old skill listed `collateral`/`lendingDeposit` (removed — lending B.1 tombstoned) and was missing `oraclePrice` (new, OutputType=15).

### Network & Chain Info Methods (`crates/rpc/src/methods/network.rs`)

**`getMempoolInfo`** (line 12) — none → `{tx_count,total_size,min_fee_rate,max_size,max_count}`.
**`getNetworkInfo`** (line 27) — none → `{peer_id,peer_count,syncing,sync_progress}`.
**`getPeerInfo`** (line 40) — none → `PeerInfoEntry[]`.
**`getChainInfo`** (line 46) — none → `{network,version,best_hash,best_height,best_slot,genesis_hash,reward_pool_balance}`.
**`getNodeInfo`** (line 71) — none → `{version,network,peerId,peerCount,platform,arch}`.
**`getEpochInfo`** (line 83) — none → `{current_height,current_epoch,last_complete_epoch,blocks_per_epoch,blocks_remaining,epoch_start_height,epoch_end_height,block_reward}`.
**`getNetworkParams`** (line 119) — none → `{network,bondUnit,slotDuration,slotsPerEpoch,blocksPerRewardEpoch,coinbaseMaturity,initialReward,genesisTime}`. CLI tools use this instead of hardcoding.

### Producer Methods (`crates/rpc/src/methods/producer.rs`)

**`getProducer`** (line 46) — `{public_key}` → `ProducerResponse` (status/era/bonds/delegations/selection_weight). Status: `active`,`unbonding`,`exited`,`slashed`. Bond data sourced from UTXO set, falls back to ProducerInfo for genesis.
**`getProducers`** (line 144) — `{active_only}` → `ProducerResponse[]`, includes `"pending"` status for awaiting-activation registrations.
**`getBondDetails`** (line 276) — `{public_key}` → `BondDetailsResponse`: per-bond FIFO list `{creation_slot,amount,age_slots,penalty_pct,vested,maturation_slot}` + `summary:{q1,q2,q3,vested}`.

### Schedule & Attestation Methods (`crates/rpc/src/methods/schedule.rs`)

**`getSlotSchedule`** (line 44) — `{from_slot,count}` (count max 360, default 20) → upcoming slot→producer assignments. Bond-weighted via `select_producer_for_slot()`.
**`getProducerSchedule`** (line 98) — `{public_key}` → assigned/produced slot counts, fill_rate, weekly_earnings, doubling_weeks.
**`getAttestationStats`** (line 211) — none → per-producer attestation-minute stats for current epoch. Decodes the **body** attestation bit array across **three decode eras** (see CONSTRAINTS). `presence_root` is a commitment hash, never decoded into indices — it is only the has-attestations discriminator, and `schedule.rs:300-304` skips both `Hash::ZERO` and the canonical-empty commitment `presence_commitment(&[],&[])`, so a post-activation zero-attester block is NOT counted in `blocks_with_attestations`.

### History Method (`crates/rpc/src/methods/history.rs`)

**`getHistory`** (line 12) — `{address, limit(max 100), before_height}` → `HistoryEntryResponse[]` via address index (O(1) height lookup).
- **tx_type values (23, current)**: `transfer`, `registration`, `exit`, `claim_reward`, `claim_bond`, `slash_producer`, `coinbase`, `add_bond`, `request_withdrawal`, `claim_withdrawal`, `mint_asset`, `epoch_reward`, `remove_maintainer`, `add_maintainer`, `delegate_bond`, `revoke_delegation`, `protocol_activation`, `burn_asset`, `create_pool`, `add_liquidity`, `remove_liquidity`, `swap`, `zk_settle`, `price_attestation`. **DRIFT FIX**: old skill listed 7 lending/NFT-fractionalization types (`create_loan`,`repay_loan`,`liquidate_loan`,`lending_deposit`,`lending_withdraw`,`fractionalize_nft`,`redeem_nft`) — ALL REMOVED (lending B.1 + NFT-frac B.2 tombstoned per CLAUDE.md). `price_attestation` is new (Phase 2.1 oracle, TxType=16).

### Governance Methods (`crates/rpc/src/methods/governance.rs`)

**`submitVote`** (line 12) — `{vote:{producer_id,version,vote,timestamp,signature}}` → `{status:"submitted"}`. Validates producer registration + Ed25519 sig over `"version:vote:timestamp"`, broadcasts.
**`getUpdateStatus`** (line 62) — none → live `{pending_update,veto_period_active,veto_count,veto_percent}`.
**`getMaintainerSet`** (line 70) — none → `{maintainers,threshold,member_count,max_maintainers,min_maintainers,initial_maintainer_count,last_change_block,source:"on-chain"|"derived"|"none"}`.
**`submitMaintainerChange`** (line 146) — `{action:"add"|"remove",target_pubkey,signatures[],reason}` → `{status:"accepted",tx_hash}`. Requires ≥3 signatures (`MAINTAINER_THRESHOLD`).

### Stats & Debug Methods (`crates/rpc/src/methods/stats.rs`)

**`getChainStats`** (line 14) — none → `{total_supply,address_count,utxo_count,active_producers,total_staked,height,reward_pool_balance,total_confirmed}`.
**`getStateRootDebug`** (line 66) — **ADMIN** — none → `{height,bestHash,stateRoot,csHash,utxoHash,psHash,utxoCount,producerCount,totalMinted,registrationSeq}`. Per-component hashes for diagnosing divergence.
**`getUtxoDiff`** (line 112) — **ADMIN** — `{}` (full dump) or `{referenceHashes:[...]}` (diff mode) → `{height,count/diffCount,entries/diffs}`. **InMemory UtxoSet only** — errors on RocksDb variant.
**`getMempoolTransactions`** (line 189) — `{limit(max 500, default 100)}` → `{hash,tx_type,size,fee,fee_rate,added_time}[]` sorted by fee_rate descending.

### Backfill & Integrity Methods (`crates/rpc/src/methods/backfill.rs`)

**`backfillFromPeer`** (line 76) — **ADMIN** — `{rpc_url,skip_divergence_check}` → `{started,gaps,total}` or `{started:false,message}`. SSRF-blocked private IPs on mainnet; tip-agreement preflight (bypass via `skip_divergence_check` post-genesis-reset); spawns background task; polls via `backfillStatus`; per-block BLAKE3 verify; also detects+repairs divergent (forked) blocks via reverse fork-point scan, not just gaps.
**`backfillStatus`** (line 461) — none → `{running,imported,total,pct,error}`.
**`verifyChainIntegrity`** (line 485) — **ADMIN** — `[]`/`[up_to_height]`/`{up_to_height,from_height}` → `{complete,tip,scanned,fromHeight,missing[],missingCount,chainCommitment}`. Uses persisted commitment (O(1) fast path) when it covers the requested range; else full BLAKE3 chain-scan.

### Snapshot Method (`crates/rpc/src/methods/snapshot.rs`)

**`getStateSnapshot`** (line 24) — **ADMIN** — none → `{height,blockHash,stateRoot,chainState,utxoSet,producerSet,epochBondSnapshot,epochAccumulators,totalBytes}` (all hex). Full snap-sync payload incl. epoch bond snapshot + attestation accumulators for correct convergence.

### Pool (AMM) Methods (`crates/rpc/src/methods/pool.rs`)

**`getPoolInfo`** (line 11) — `{poolId}` → `{poolId,assetA,assetB,reserveA,reserveB,totalShares,feeBps,price,twapCumulativePrice,lastUpdateSlot,creationSlot,status,txHash,outputIndex}`.
**`getPoolList`** (line 57) — any → `{poolId,assetB,reserveA,reserveB,feeBps,price}[]`. Dedupes by pool_id (keeps highest reserveA).
**`getPoolPrice`** (line 99) — `{poolId,windowSlots?}` → `{spotPrice,twapPrice?,twapWindow?}`. TWAP only if `windowSlots>0`.
**`getSwapQuote`** (line 153) — `{poolId,amountIn,direction:"a2b"|"b2a"}` → `{amountOut,priceImpact,fee}`. Simulated only, no tx.

### DeFi Health Method (`crates/rpc/src/methods/defi_health.rs`) — NEW since 2026-05-11

**`getDefiHealthMetric`** (line 47) — none → `{totalActiveBonds,maxPoolTvl,maxPoolId,bondToTvlRatio,status:"ok"|"degraded"|"no_pools",disclosure,note}`. D4/AC-6 monitoring metric `R = total_active_bonds / max_pool_TVL` — disclosure only, never a tx-rejection gate. Pre-oracle: TVL self-denominated in DOLI via pool's own spot price.

### Oracle Methods (`crates/rpc/src/methods/oracle.rs`, `oracle_status.rs`) — NEW since 2026-05-11, Phase 2.1 M9-M11

Read-only consumers of `OraclePrice` UTXO (OutputType=15) + `PriceAttestation` tx (TxType=16). **Frozen pre-activation**: `oracle_activation_height = u64::MAX` on every `NetworkParams` variant (mainnet/testnet/devnet) — see CLAUDE.md.

**`getOraclePrice`** (`oracle.rs:91`) — `{pair_id:"<64-hex>"}` → `{pair_id,price_cents,last_update_height,contributor_count,is_stale,trust_model:"structural-anchored"}` or `null` if UTXO absent. `is_stale` when `current_height - last_update_height > blocks_per_reward_epoch`.
**`getOracleAttestations`** (`oracle.rs:167`) — `{epoch,pair_id}` → `{epoch,pair_id,attestations:[{attester_pubkey,attester_pubkey_hash,price_cents,bond_weight}]}`. Sorted by `attester_pubkey_hash` bytes (deterministic). `bond_weight` is `null` unless queried epoch matches the single persisted bond-snapshot epoch. Empty-list contract for unknown/future/pruned epochs — never errors.
**`getOracleStatus`** (`oracle.rs:310`) — none → `{active,health,trust_model,structural_share,sunset_threshold:0.55,sunset_triggered,last_update_height,attester_count,activation_height,centralization_disclosure}`. `active = current_height>=activation_height && !sunset_triggered`. `health` ∈ `healthy`/`warning`/`halted_recoverable` (zone-derived from `structural_share_bps`: ≥6000 healthy, 5500-5999 warning, <5500 halt). `centralization_disclosure` is byte-equal-locked to `specs/oracle-structural-anchored-economics.md` §6 (`oracle_status.rs:167-191`, drift-gate test `m11_centralization_disclosure_byte_equal_to_spec`).

### Storage Management Methods (`crates/rpc/src/methods/pruning.rs`)

**`pruneBlocks`** (line 19) — **ADMIN** — `[keep_last_n]` (default 2000, min-clamped to 2000) → `{status,pruned,lowest_remaining_height,chain_tip,keep_last_n,archive_verified}`. Checks archive coverage first (warns, proceeds anyway if incomplete).
**`getStorageInfo`** (line 106) — none → `{chain_tip,height_range,column_families,prunable_blocks,min_retention:2000,archive_height}`.

### Guardian Methods (`crates/rpc/src/methods/guardian.rs`)

**`pauseProduction`** (line 19) — **ADMIN** — none → `{status:"paused"}`. Blocks production via `SyncManager::block_production()`; node keeps running.
**`resumeProduction`** (line 39) — **ADMIN** — none → `{status:"resumed"}`.
**`createCheckpoint`** (line 62) — **ADMIN** — `["optional/relative/path"]` → `{status,path,height,timestamp,components:["state_db","blocks"]}`. RocksDB hard-link checkpoint, path-traversal protected (rejects absolute + `..`).
**`getGuardianStatus`** (line 162) — none → `{production_paused,production_block_reason,chain_height,chain_slot,best_hash,last_checkpoint,last_healthy_checkpoint,recovery_mode}`.
**`enterRecoveryMode`** (line 242) — **ADMIN** — none → `{status:"recovery_mode_active"}`. Activates anti-poisoning gate: drops inbound blocks + snap sync.
**`exitRecoveryMode`** (line 257) — **ADMIN** — none → `{status:"recovery_mode_inactive"}`. Recommend restarting node to clear cached fork blocks (fork_block_cache lives on Node, not RpcContext).
**`bridgeFromArchive`** (line 280) — **ADMIN** — `[force]` or `{force}` (default false) → `{status:"ok"|"warning",blocks_imported,archive_found,commitment_deleted}`. Deletes stale chain_commitment, then backfills from local archive dir. `force=true` replaces divergent blocks (requires recovery mode active). Requires `--archive-to` on node start.
**`repairArchiveFromPeer`** (line 411) — `{rpc_url}` or `["url"]` → `{status,fetched,already_present,errors,peer_tip}`. Fills missing `.block`/`.blake3` in local archive from peer; validates genesis_hash per block; atomic `.tmp`→rename writes. SSRF-checked via shared `validate_backfill_url()`.

## DATA-FLOWS

**Read path (queries)**:
1. HTTP POST → `handle_rpc()` in `server.rs:299`
2. `check_admin_auth()` (`server.rs:189`) — resolves effective client IP via `resolve_client_ip()` if peer is a trusted proxy, then trust/token check
3. `RpcContext::handle_request()` → dispatches by method name (`dispatch.rs`, 53-arm match)
4. Handler reads from: `chain_state` (RwLock), `utxo_set` (RwLock), `producer_set` (RwLock, Option), `mempool` (RwLock), `block_store` (Arc), `state_db` (Arc, Option)
5. Returns `JsonRpcResponse::success(id, value)` or `JsonRpcResponse::error(id, rpc_error)`

**Write path (`sendTransaction`)**:
1. Parse hex → deserialize `Transaction`
2. Lock mempool write → `add_transaction()` (normal) or `add_system_transaction()` (state-only: Exit, RequestWithdrawal, etc.)
3. Unlock → call `broadcast_tx(tx)` callback (fire-and-forget to network layer)
4. If AMM pool contention detected during add, surface `POOL_CONTENTION` warning without failing the submission

**Write path (`backfillFromPeer`)**:
1. Validate URL (SSRF protection), tip-agreement preflight (skippable)
2. Scan block_store for gaps AND divergent (forked) blocks via reverse fork-point walk (spawn_blocking)
3. Set `BackfillState` atomics, spawn background tokio task
4. Background: fetch `getBlockRaw` → verify BLAKE3 → `put_block_canonical()`
5. Poll via `backfillStatus`

**Oracle read path (`getOraclePrice`/`getOracleAttestations`/`getOracleStatus`)**:
1. `getOraclePrice`: outpoint derived deterministically from `pair_id` via `doli_core::oracle::oracle_price_outpoint()` → direct UTXO lookup, no scan
2. `getOracleAttestations`: walks `[epoch*blocks_per_epoch, (epoch+1)*blocks_per_epoch)` block range, filters `PriceAttestation` txs by pair_id+epoch, dedupes by latest-per-signer
3. `getOracleStatus`: reads `state_db` meta (`get_oracle_last_update_height()`, cached — NOT a UTXO-set scan, per AUDIT-P2-001 fix) + `STRUCTURAL_PUBKEY_HASHES_HEX` constant + bond snapshot to compute `structural_share_bps` via `doli_core::oracle::compute_structural_share_bps()`

**WebSocket events**:
- `WsEvent::NewBlock`/`WsEvent::NewTx` broadcast via `tokio::sync::broadcast::Sender<WsEvent>` (`ws.rs:26`)
- Clients connect to `GET /ws`; hard cap 100 concurrent connections (`ws.rs:18`), rejects with 503 above limit

**State snapshot flow**:
- `getStateSnapshot` reads all 3 state components under their respective locks
- Includes `epochBondSnapshot` and `epochAccumulators` from `state_db` for correct snap-sync convergence

## DEPENDENCIES

**Internal crates**:
- `crates/rpc/` depends on: `doli_core`, `crypto`, `storage`, `network`, `mempool`
- `storage`: `BlockStore`, `ChainState`, `ProducerSet`, `StateDb`, `UtxoSet`, `MaintainerState`, `StateSnapshot`
- `network`: `SyncManager` (for production halt)
- `mempool`: `Mempool`, `MempoolEntry`, `MempoolError`, `MempoolPolicy`
- `doli_core`: `Transaction`, `Block`, `ConsensusParams`, `NetworkParams`, `OutputType`, `TxType`, `oracle::{oracle_price_outpoint, compute_structural_share_bps, SUNSET_THRESHOLD_BPS, SUNSET_WARNING_BPS, OracleHealthState}`
- `crypto`: `Hash`, `PublicKey`, `Signature`, address encoding/decoding

**External crates**:
- `axum` — HTTP router and WebSocket upgrade
- `tower-http` — CORS layer, request body size limit
- `reqwest` — outbound HTTP for `backfillFromPeer`, `repairArchiveFromPeer`
- `serde_json` — JSON ser/de
- `bincode` — block serialization for `getBlockRaw`, epoch meta for snapshots
- `base64` — block data encoding
- `tokio` — async runtime, `RwLock`, `broadcast`

**Callbacks (injected at construction, `RpcContext`)**:
- `peer_count: Arc<dyn Fn() -> usize>`, `peer_list: Arc<dyn Fn() -> Vec<PeerInfoEntry>>`
- `broadcast_tx: Arc<dyn Fn(Transaction)>`, `broadcast_vote: Arc<dyn Fn(Vec<u8>)>`
- `sync_status: Arc<dyn Fn() -> SyncStatus>`, `update_status: Arc<dyn Fn() -> Value>`

## CONSTRAINTS

**Address resolution** (`context.rs:344`): bech32m (`doli1…` mainnet, `tdoli1…` testnet, `ddoli1…` devnet) or 64-char hex pubkey_hash via `crypto::address::resolve()`.

**Network-specific values** (from `NetworkParams` at construction, `context.rs:118-160`):
- `bond_unit`: mainnet/testnet = 1,000,000,000 (10 DOLI), devnet = 100,000,000 (1 DOLI)
- `coinbase_maturity`: mainnet/testnet = 100 blocks, devnet = 10 blocks
- `blocks_per_reward_epoch`: mainnet/testnet = 360, devnet = 60
- `vesting_quarter_slots`: mainnet = 3,153,600, testnet = 2,160
- `oracle_activation_height`: **u64::MAX on every variant** — Phase 2.1 oracle frozen (see CLAUDE.md)

**Admin method auth** (`server.rs:31-49, 189-241`): non-private-IP callers need `Authorization: Bearer <token>`; `admin_token:None` disables admin methods from public IPs; loopback+RFC-1918 always trusted; trusted-proxy header resolution is opt-in (empty `trusted_proxies` = no header trust, closes the historical "Nginx makes every request look like 127.0.0.1" bypass, ISSUE-174).

**Backfill SSRF protection** (`backfill.rs:17-72`): mainnet blocks private/loopback/link-local/broadcast/unspecified IP literals; testnet/devnet allow all IPs (single-server localhost backfill); URL must be `http://`/`https://`. Shared by `backfillFromPeer` and `repairArchiveFromPeer` (`super::backfill::validate_backfill_url`).

**Pruning minimum retention** (`pruning.rs:32-38`): `keep_last_n` floored at 2000 blocks via `.max(2000)`.

**Attestation bitfield decode eras** (`schedule.rs:226-287`):
- Pre-`rewards_epoch_list_fix_height`: legacy sort_all by pubkey
- `rewards_epoch_list_fix_height`..`full_bitfield_decode_height`: `epoch_state.producer_list` only (base)
- Post-`full_bitfield_decode_height`: `[base | extra sorted by pubkey]` — full decode, shows all producers

**Checkpoint path safety** (`guardian.rs:84-116`): rejects absolute paths and `..` components; canonicalizes parent and checks it stays within `data_dir`.

**getBlockByHash height caveat** (`block.rs:27-29`): always returns `height=0` — no reverse hash→height index exists; use `getBlockByHeight` when height is needed.

**getUtxoDiff InMemory-only** (`stats.rs:143-147`): errors for `UtxoSet::RocksDb` variant.

**WebSocket limit** (`ws.rs:18`): hard cap 100 concurrent connections, 503 above.

**Oracle byte-equality drift gate** (`oracle_status.rs:159-191`): `CENTRALIZATION_DISCLOSURE` constant must match `specs/oracle-structural-anchored-economics.md` §6 verbatim — enforced by test `m11_centralization_disclosure_byte_equal_to_spec`; edit both together or the test fails.

**Bond-weight historical limit** (`oracle.rs:151-157`): `getOracleAttestations` only returns non-null `bond_weight` when the queried epoch matches the SINGLE persisted bond snapshot's epoch (state_db keeps only the most-recently-closed epoch's snapshot) — historical bond weights are not preserved.

**Error codes** (`error.rs:7-38`): Standard: `-32700` parse, `-32600` invalid_request, `-32601` method_not_found, `-32602` invalid_params, `-32603` internal_error. Custom: `-32000` block_not_found, `-32001` tx_not_found, `-32002` invalid_tx, `-32003` tx_already_known, `-32004` mempool_full, `-32005` utxo_not_found, `-32006` producer_not_found, `-32007` pool_not_found, `-32008` unauthorized.

## PATTERNS

**Constructor pattern** (use `new_for_network`, not deprecated `new`):
```
RpcContext::new_for_network(chain_state, block_store, utxo_set, mempool, params, network)
    .with_producer_set(producer_set)
    .with_peer_id(peer_id)
    .with_peer_count(|| ...)
    .with_peer_list(|| ...)
    .with_broadcast(|tx| ...)
    .with_sync_status(|| ...)
    .with_broadcast_vote(|v| ...)
    .with_update_status(|| ...)
    .with_sync_manager(sync_manager)
    .with_state_db(state_db)
    .with_data_dir(data_dir)
    .with_archive_dir(dir)
    .with_recovery_mode(recovery_mode_arc)
```
Note: `oracle_activation_height` has NO builder method — it is set directly from `NetworkParams` inside `new_for_network()` (`context.rs:148`) and hardcoded to `u64::MAX` in the deprecated `new()` path (`context.rs:212`).

**Bond data sourcing** — always prefer UTXO set over ProducerInfo (`producer.rs:75-90`, `144-286`):
- `utxo_set.count_bonds(&pubkey_hash, bond_unit)` for count
- `utxo_set.get_bond_entries(&pubkey_hash)` for per-bond details (FIFO-sorted)
- `utxo_set.get_bonded_balance(&pubkey_hash)` for total staked
- Fall back to `info.bond_count`/`info.bond_amount` only if UTXO count == 0 (genesis producers).

**Pure-function extraction for test injectability** (`oracle_status.rs`): `build_oracle_status_response(OracleStatusInputs)` is split from the async handler so M11 tests can inject mock `structural_hashes` — production always uses the real mainnet-derived `STRUCTURAL_PUBKEY_HASHES_HEX` constant (real hash preimages cannot be forged in tests).

**500-LOC module split under size budget**: `oracle.rs` (handlers) + `oracle_status.rs` (pure builder + disclosure constant) + `tests_oracle.rs`/`tests_oracle_m11.rs` (tests split at ~800 LOC) — mirrors Rule 19 (max 500 lines source, 800 tests). Follow this pattern when adding new RPC surface areas.

**Backfill recovery procedure**:
1. `enterRecoveryMode` — blocks inbound state mutations
2. `bridgeFromArchive` — deletes stale commitment, fills block_store from archive
3. `backfillFromPeer` — fills remaining gaps from a peer RPC
4. `exitRecoveryMode` — restart node to clear fork block cache
5. `verifyChainIntegrity` — confirm no gaps remain

**Diff workflow across nodes**:
1. `getStateRootDebug` on all nodes — compare `stateRoot`, `csHash`, `utxoHash`, `psHash`
2. If `utxoHash` differs: `getUtxoDiff` on node A (full), pass hashes to node B (`referenceHashes`)
3. If `chainCommitment` differs (from `verifyChainIntegrity`): `backfillFromPeer` to repair

**Governance vote flow**:
1. Producer signs `"version:vote:timestamp"` with Ed25519 key
2. `submitVote` — validates producer registration + signature, broadcasts
3. `getUpdateStatus` — check veto counts

**Checkpoint restore flow** (INC-I-041/INC-I-055):
1. Stop node → restore RocksDB checkpoint → start node
2. `enterRecoveryMode`
3. `bridgeFromArchive` (optionally `force=true` to replace divergent blocks)
4. `verifyChainIntegrity` to confirm coverage
5. `exitRecoveryMode` → restart node

**SSRF-safe outbound HTTP**: any RPC method making an outbound HTTP call (`backfillFromPeer`, `repairArchiveFromPeer`) MUST call `validate_backfill_url(url, network)` before dialing — mainnet blocks private IP literals, testnet/devnet allow them.
