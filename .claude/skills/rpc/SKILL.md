# rpc — DOLI JSON-RPC API
<!-- @INDEX
ENTRY-POINTS: lines 16-35
METHODS: lines 38-295
DATA-FLOWS: lines 298-330
DEPENDENCIES: lines 333-355
CONSTRAINTS: lines 358-395
PATTERNS: lines 398-440
-->

## ENTRY-POINTS

**Transport**: HTTP POST `/` — JSON-RPC 2.0 envelope (`jsonrpc`, `method`, `params`, `id`).
**WebSocket**: `GET /ws` — subscribe to real-time events (`new_block`, `new_tx`).
**Max request body**: 2 MB (covers NFT hex-encoded data).
**Server struct**: `crates/rpc/src/server.rs:77` `RpcServer { config, context, ws_sender }`
**Config struct**: `crates/rpc/src/server.rs:50` `RpcServerConfig { listen_addr, enable_cors, allowed_origins, admin_token }`
**Default port**: 8500 (mainnet), network-specific via `NetworkParams::load(network).default_rpc_port`.
**Dispatch entry**: `crates/rpc/src/methods/dispatch.rs:12` — `RpcContext::handle_request()`.
**Context struct**: `crates/rpc/src/methods/context.rs:41` — `RpcContext` holds all shared state.

**Admin auth** (`crates/rpc/src/server.rs:31-46`):
- Admin methods require `Authorization: Bearer <token>` from public IPs.
- Loopback (127.x) and RFC-1918 private IPs are always trusted — no token needed.
- Constant-time comparison used to prevent timing side-channels.
- Admin set: `pauseProduction`, `resumeProduction`, `createCheckpoint`, `pruneBlocks`, `backfillFromPeer`, `enterRecoveryMode`, `exitRecoveryMode`, `bridgeFromArchive`, `getUtxoDiff`, `getStateSnapshot`, `getStateRootDebug`, `verifyChainIntegrity`.

## METHODS

### Block Methods (`crates/rpc/src/methods/block.rs`)

**`getBlockByHash`** (line 14)
- Params: `{ "hash": "<64-hex>" }`
- Returns: `BlockResponse` (hash, height=0 always, transactions, producer, slot, timestamp, etc.)
- Note: height always 0 — no reverse hash→height index; use `getBlockByHeight` instead.

**`getBlockByHeight`** (line 38)
- Params: `{ "height": <u64> }` or `[height]`
- Returns: `BlockResponse` with correct height

**`getBlockRaw`** (line 61)
- Params: `{ "height": <u64> }`
- Returns: `{ "block": "<base64>", "blake3": "<hex>", "height": <u64> }`
- Used by backfill system — bincode-serialized block, BLAKE3 checksum for integrity.

**`getBlockData`** (line 90)
- Params: `{ "hash": "<hex>", "output_index": <u32> }`
- Returns: `{ "data": "<base64>", "size": <u64>, "blob_hash": "<hex>", "output_type": "<str>" }`
- Retrieves `extra_data` from a specific output (NFT/document content).

### Transaction Methods (`crates/rpc/src/methods/transaction.rs`)

**`getTransaction`** (line 16)
- Params: `{ "hash": "<64-hex>" }`
- Returns: `TransactionResponse` (hash, tx_type, inputs with resolved amounts/addresses, outputs, block_hash, block_height, confirmations, fee)
- Checks mempool first, then tx index → block lookup.

**`sendTransaction`** (line 165)
- Params: `{ "tx": "<hex>" }` — hex-encoded serialized `Transaction`
- Returns: `"<tx_hash_hex>"`
- State-only txs (Exit, RequestWithdrawal, etc.) bypass UTXO fee accounting.
- Broadcasts to network after mempool acceptance.
- Structured error data on failure: `INVALID_HEX`, `DESERIALIZE_FAILED`, `TX_ALREADY_EXISTS`, `MEMPOOL_FULL`, `INVALID_TX`.

**`getNftByTokenId`** (line 104)
- Params: `["<tokenId_hex>"]` or `{ "tokenId": "<hex>" }`
- Returns: `{ "tokenId", "outpoint", "owner", "amount", "height", "contentHash", "contentSize", "royalty" }`
- Scans UTXO set for matching NFT token_id.

### Balance & UTXO Methods (`crates/rpc/src/methods/balance.rs`)

**`getBalance`** (line 12)
- Params: `{ "address": "<bech32m or 64-hex>" }`
- Returns: `{ "confirmed": <spendable>, "unconfirmed": <mempool_change>, "immature": <u64>, "bonded": <u64>, "total": <u64> }`
- Address formats: bech32m (`doli1…`, `tdoli1…`, `ddoli1…`) or 64-char hex pubkey_hash.
- Applies `coinbase_maturity` (network-specific: mainnet/testnet=100, devnet=10).

**`getUtxos`** (line 57)
- Params: `{ "address": "<bech32m or 64-hex>", "spendable_only": <bool> }`
- Returns: array of `UtxoResponse` (tx_hash, output_index, amount, output_type, lock_until, height, spendable, pending, condition, nft, asset, bridge)
- Includes pending mempool outputs for chained transaction support.
- Output types: `normal`, `bond`, `multisig`, `hashlock`, `htlc`, `vesting`, `nft`, `fungibleAsset`, `bridgeHtlc`, `pool`, `lpShare`, `collateral`, `lendingDeposit`, `zkRollup`, `encryptedContent`.

### Network & Chain Info Methods (`crates/rpc/src/methods/network.rs`)

**`getMempoolInfo`** (line 12)
- Params: none
- Returns: `{ "tx_count", "total_size", "min_fee_rate", "max_size", "max_count" }`

**`getNetworkInfo`** (line 27)
- Params: none
- Returns: `{ "peer_id", "peer_count", "syncing", "sync_progress" }`

**`getPeerInfo`** (line 40)
- Params: none
- Returns: array of `PeerInfoEntry` (detailed per-peer info)

**`getChainInfo`** (line 46)
- Params: none
- Returns: `{ "network", "version", "best_hash", "best_height", "best_slot", "genesis_hash", "reward_pool_balance" }`

**`getNodeInfo`** (line 71)
- Params: none
- Returns: `{ "version", "network", "peerId", "peerCount", "platform", "arch" }`

**`getEpochInfo`** (line 83)
- Params: none
- Returns: `{ "current_height", "current_epoch", "last_complete_epoch", "blocks_per_epoch", "blocks_remaining", "epoch_start_height", "epoch_end_height", "block_reward" }`
- Uses network-specific `blocks_per_reward_epoch`.

**`getNetworkParams`** (line 119)
- Params: none
- Returns: `{ "network", "bondUnit", "slotDuration", "slotsPerEpoch", "blocksPerRewardEpoch", "coinbaseMaturity", "initialReward", "genesisTime" }`
- CLI tools use this instead of hardcoding values.

### Producer Methods (`crates/rpc/src/methods/producer.rs`)

**`getProducer`** (line 46)
- Params: `{ "public_key": "<64-hex>" }`
- Returns: `ProducerResponse` — `{ public_key, address_hash, registration_height, bond_amount, bond_count, status, era, pending_withdrawals, pending_updates, bls_pubkey, delegated_to, delegated_bonds, received_delegations, selection_weight }`
- Status values: `active`, `unbonding`, `exited`, `slashed`.
- Bond data sourced from UTXO set (source of truth), falls back to ProducerInfo for genesis.

**`getProducers`** (line 144)
- Params: `{ "active_only": <bool> }`
- Returns: array of `ProducerResponse` — same as `getProducer`
- Includes `"pending"` status for producers awaiting epoch activation.

**`getBondDetails`** (line 276)
- Params: `{ "public_key": "<64-hex>" }`
- Returns: `BondDetailsResponse` — per-bond granularity with vesting info:
  `{ public_key, bond_count, total_staked, registration_slot, age_slots, penalty_pct, vested, maturation_slot, vesting_quarter_slots, vesting_period_slots, summary: {q1,q2,q3,vested}, bonds: [{creation_slot,amount,age_slots,penalty_pct,vested,maturation_slot}], withdrawal_pending_count }`
- FIFO-ordered bond list (oldest first).

### Schedule & Attestation Methods (`crates/rpc/src/methods/schedule.rs`)

**`getSlotSchedule`** (line 44)
- Params: `{ "from_slot": <u32|null>, "count": <u32|null> }` (count max 360, default 20)
- Returns: `{ "slots": [{slot,producer,rank}], "current_slot", "epoch", "slots_remaining_in_epoch", "total_bonds", "slot_duration", "genesis_time" }`
- Bond-weighted scheduling using `select_producer_for_slot()`.

**`getProducerSchedule`** (line 98)
- Params: `{ "public_key": "<64-hex>" }`
- Returns: `ProducerScheduleResponse` — `{ public_key, current_slot, epoch, next_slot, seconds_until_next, slots_this_epoch, assigned_count, produced_count, fill_rate, bond_count, total_network_bonds, weekly_earnings, doubling_weeks, block_reward }`
- Scans block store for produced-block count; computes economics.

**`getAttestationStats`** (line 211)
- Params: none
- Returns: `{ "epoch", "epoch_start", "current_height", "blocks_in_epoch", "blocks_with_attestations", "blocks_with_bls", "current_minute", "producers": [{public_key,attested_minutes,total_minutes,threshold,qualified,has_bls}] }`
- Decodes presence_root bitfields for entire current epoch.
- Three decode eras based on activation heights (see CONSTRAINTS).

### History Method (`crates/rpc/src/methods/history.rs`)

**`getHistory`** (line 12)
- Params: `{ "address": "<bech32m or hex>", "limit": <u32|max 100>, "before_height": <u64|null> }`
- Returns: array of `HistoryEntryResponse` — `{ hash, tx_type, block_hash, height, timestamp, amount_received, amount_sent, fee, confirmations, from: [addr], to: [addr] }`
- Uses address index (O(1) block height lookup), then resolves TX data.
- tx_type values: `transfer`, `registration`, `exit`, `claim_reward`, `claim_bond`, `slash_producer`, `coinbase`, `add_bond`, `request_withdrawal`, `claim_withdrawal`, `mint_asset`, `epoch_reward`, `remove_maintainer`, `add_maintainer`, `delegate_bond`, `revoke_delegation`, `protocol_activation`, `burn_asset`, `create_pool`, `add_liquidity`, `remove_liquidity`, `swap`, `create_loan`, `repay_loan`, `liquidate_loan`, `lending_deposit`, `lending_withdraw`, `fractionalize_nft`, `redeem_nft`, `zk_settle`.

### Governance Methods (`crates/rpc/src/methods/governance.rs`)

**`submitVote`** (line 12)
- Params: `{ "vote": { "producer_id": "<hex>", "version": "<str>", "vote": "<str>", "timestamp": <u64>, "signature": "<hex>" } }`
- Returns: `{ "status": "submitted", "message": "..." }`
- Validates producer registration, verifies Ed25519 signature over `"version:vote:timestamp"`, broadcasts.

**`getUpdateStatus`** (line 62)
- Params: none
- Returns: live UpdateService state: `{ "pending_update", "veto_period_active", "veto_count", "veto_percent" }`

**`getMaintainerSet`** (line 70)
- Params: none
- Returns: `{ "maintainers": [{pubkey}], "threshold", "member_count", "max_maintainers", "min_maintainers", "initial_maintainer_count", "last_change_block", "source": "on-chain"|"derived"|"none" }`
- Uses on-chain `MaintainerState` if available; derives from producer set as fallback.

**`submitMaintainerChange`** (line 146)
- Params: `{ "action": "add"|"remove", "target_pubkey": "<hex>", "signatures": [{pubkey,signature}], "reason": "<str|null>" }`
- Returns: `{ "status": "accepted", "tx_hash": "<hex>", "message": "..." }`
- Requires at least 3 maintainer signatures (`MAINTAINER_THRESHOLD`).

### Stats & Debug Methods (`crates/rpc/src/methods/stats.rs`)

**`getChainStats`** (line 14)
- Params: none
- Returns: `{ "total_supply", "address_count", "utxo_count", "active_producers", "total_staked", "height", "reward_pool_balance", "total_confirmed" }`

**`getStateRootDebug`** (line 66) — **ADMIN**
- Params: none
- Returns: `{ "height", "bestHash", "stateRoot", "csHash", "utxoHash", "psHash", "utxoCount", "producerCount", "totalMinted", "registrationSeq" }`
- Per-component hashes for diagnosing state divergence between nodes.

**`getUtxoDiff`** (line 112) — **ADMIN**
- Params: `{}` (full dump) or `{ "referenceHashes": ["<hash>", ...] }` (diff mode)
- Returns full dump: `{ "height", "count", "entries": [{outpoint,hash,detail}] }`
- Returns diff: `{ "height", "totalEntries", "diffCount", "diffs": [{outpoint,hash,detail}] }`
- Only works with in-memory UTXO set (not RocksDb).

**`getMempoolTransactions`** (line 189)
- Params: `{ "limit": <u32|max 500, default 100> }`
- Returns: array of `{ "hash", "tx_type", "size", "fee", "fee_rate", "added_time" }` sorted by fee_rate descending.

### Backfill & Integrity Methods (`crates/rpc/src/methods/backfill.rs`)

**`backfillFromPeer`** (line 76) — **ADMIN**
- Params: `{ "rpc_url": "http://peer:8500", "skip_divergence_check": <bool> }`
- Returns: `{ "started": true, "gaps": "1-100,200", "total": <u64> }` or `{ "started": false, "message": "..." }`
- URL must be `http://` or `https://`; mainnet blocks private IPs (SSRF protection).
- Performs tip-agreement preflight check (bypass with `skip_divergence_check=true` for post-genesis-reset).
- Spawns background task; use `backfillStatus` to poll progress.
- Verifies BLAKE3 checksum per block.

**`backfillStatus`** (line 461)
- Params: none
- Returns: `{ "running": <bool>, "imported", "total", "pct", "error": <str|null> }`

**`verifyChainIntegrity`** (line 485) — **ADMIN**
- Params: `[]` or `[up_to_height]` or `{ "up_to_height": <u64>, "from_height": <u64> }`
- Returns: `{ "complete": <bool>, "tip", "scanned", "fromHeight", "missing": ["1-5","100"], "missingCount", "chainCommitment": "<hex|null>" }`
- `chainCommitment` = BLAKE3(BLAKE3(h1_hash || h2_hash) || h3_hash ...) running hash.
- Uses persisted commitment (fast O(1) path) when available; falls back to full scan.

### Snapshot Method (`crates/rpc/src/methods/snapshot.rs`)

**`getStateSnapshot`** (line 24) — **ADMIN**
- Params: none
- Returns: `{ "height", "blockHash", "stateRoot", "chainState": "<hex>", "utxoSet": "<hex>", "producerSet": "<hex>", "epochBondSnapshot": "<hex|null>", "epochAccumulators": "<hex|null>", "totalBytes" }`
- Full snap sync payload — includes epoch metadata for correct bond snapshots and attestation accumulators.

### Pool (AMM) Methods (`crates/rpc/src/methods/pool.rs`)

**`getPoolInfo`** (line 11)
- Params: `{ "poolId": "<hex>" }`
- Returns: `{ "poolId", "assetA", "assetB", "reserveA", "reserveB", "totalShares", "feeBps", "price", "twapCumulativePrice", "lastUpdateSlot", "creationSlot", "status", "txHash", "outputIndex" }`

**`getPoolList`** (line 57)
- Params: any (ignored)
- Returns: array of `{ "poolId", "assetB", "reserveA", "reserveB", "feeBps", "price" }`
- Deduplicates by pool_id (keeps highest reserveA entry).

**`getPoolPrice`** (line 99)
- Params: `{ "poolId": "<hex>", "windowSlots": <u64|optional> }`
- Returns: `{ "spotPrice": <f64>, "twapPrice": <f64|optional>, "twapWindow": <u32|optional> }`
- TWAP computed only if `windowSlots > 0`.

**`getSwapQuote`** (line 153)
- Params: `{ "poolId": "<hex>", "amountIn": <u64>, "direction": "a2b"|"b2a" }`
- Returns: `{ "amountOut": <u64>, "priceImpact": <f64_percent>, "fee": <u64> }`
- Simulated only — no transaction created.

### Lending Methods (`crates/rpc/src/methods/lending.rs`)

**`getLoanInfo`** (line 13)
- Params: `{ "txHash": "<hex>", "outputIndex": <u32> }`
- Returns: full loan details — `{ "outpoint", "poolId", "borrowerHash", "collateralAmount", "collateralAssetId", "principal", "interestRateBps", "creationSlot", "liquidationRatioBps", "accruedInterest", "totalDebt", "elapsedSlots", "ltvBps", "liquidatable" }`

**`getLoanList`** (line 85)
- Params: `{ "borrower": "<hex>"|optional }`
- Returns: array of loan summaries — `{ "outpoint", "borrowerHash", "collateralAmount", "principal", "totalDebt", "interestRateBps", "liquidatable" }`

### Storage Management Methods (`crates/rpc/src/methods/pruning.rs`)

**`pruneBlocks`** (line 19) — **ADMIN**
- Params: `[keep_last_n]` (default 2000, minimum enforced at 2000)
- Returns: `{ "status", "pruned", "lowest_remaining_height", "chain_tip", "keep_last_n", "archive_verified" }`
- Checks archive coverage before pruning (warns but proceeds if archive incomplete).

**`getStorageInfo`** (line 106)
- Params: none
- Returns: `{ "chain_tip", "height_range": {min,max}, "column_families": {cf_name:count}, "prunable_blocks", "min_retention": 2000, "archive_height": <u64|null> }`

### Guardian Methods (`crates/rpc/src/methods/guardian.rs`)

**`pauseProduction`** (line 19) — **ADMIN**
- Params: none
- Returns: `{ "status": "paused", "message": "..." }`
- Blocks production via `SyncManager::block_production()`. Node keeps running.

**`resumeProduction`** (line 39) — **ADMIN**
- Params: none
- Returns: `{ "status": "resumed", "message": "..." }`

**`createCheckpoint`** (line 62) — **ADMIN**
- Params: `["optional/relative/path"]` (default: `{data_dir}/checkpoints/h{height}-{timestamp}/`)
- Returns: `{ "status": "ok", "path", "height", "timestamp", "components": ["state_db","blocks"] }`
- RocksDB hard-link checkpoint — near-instant, near-zero disk overhead.
- Path traversal protection: rejects absolute paths and `..` components.

**`getGuardianStatus`** (line 162)
- Params: none
- Returns: `{ "production_paused", "production_block_reason", "chain_height", "chain_slot", "best_hash", "last_checkpoint", "last_healthy_checkpoint", "recovery_mode" }`

**`enterRecoveryMode`** (line 242) — **ADMIN**
- Params: none
- Returns: `{ "status": "recovery_mode_active", "message": "..." }`
- Activates anti-poisoning gate: drops all inbound blocks and snap sync.

**`exitRecoveryMode`** (line 257) — **ADMIN**
- Params: none
- Returns: `{ "status": "recovery_mode_inactive", "message": "..." }`
- Recommend restarting node after exit to clear cached fork blocks.

**`bridgeFromArchive`** (line 280) — **ADMIN**
- Params: `[force]` or `{ "force": <bool> }` (default false)
- Returns: `{ "status": "ok"|"warning", "blocks_imported", "archive_found", "commitment_deleted" }`
- Step 1: Deletes stale chain_commitment. Step 2: Backfills from local archive dir.
- `force=true` replaces divergent blocks (requires recovery mode active).
- Requires `--archive-to` flag on node start.

**`repairArchiveFromPeer`** (line 411)
- Params: `{ "rpc_url": "http://peer:8500" }` or `["http://peer:8500"]`
- Returns: `{ "status": "ok", "fetched", "already_present", "errors", "peer_tip" }`
- Fills missing `.block` / `.blake3` files in local archive from peer.
- Validates genesis_hash per block to prevent cross-chain contamination.
- Atomic write: `.block.tmp` → rename to `.block`.

## DATA-FLOWS

**Read path (queries)**:
1. HTTP POST → `handle_rpc()` in `server.rs`
2. Auth check (`check_admin_auth`) for admin methods
3. `RpcContext::handle_request()` → dispatches by method name (`dispatch.rs`)
4. Handler reads from: `chain_state` (RwLock), `utxo_set` (RwLock), `producer_set` (RwLock), `mempool` (RwLock), `block_store` (Arc), `state_db` (Arc)
5. Returns `JsonRpcResponse::success(id, value)` or `JsonRpcResponse::error(id, rpc_error)`

**Write path (`sendTransaction`)**:
1. Parse hex → deserialize `Transaction`
2. Lock mempool write → `add_transaction()` or `add_system_transaction()`
3. Unlock → call `broadcast_tx(tx)` callback (fires-and-forgets to network layer)

**Write path (`backfillFromPeer`)**:
1. Validate URL (SSRF protection), tip-agreement preflight
2. Scan block_store for gaps (spawn_blocking)
3. Set `BackfillState` atomics, spawn background tokio task
4. Background: fetch `getBlockRaw` → verify BLAKE3 → `put_block_canonical()`
5. Poll via `backfillStatus`

**WebSocket events**:
- `WsEvent::NewBlock` / `WsEvent::NewTx` broadcast via `tokio::sync::broadcast::Sender<WsEvent>`
- Clients connect to `GET /ws`; max 100 concurrent connections
- Events are serialized to JSON and pushed to all subscribers

**State snapshot flow**:
- `getStateSnapshot` reads all 3 state components under their respective locks
- Includes `epochBondSnapshot` and `epochAccumulators` from `state_db` for correct snap sync convergence

## DEPENDENCIES

**Internal crates**:
- `crates/rpc/` depends on: `doli_core`, `crypto`, `storage`, `network`, `mempool`
- `storage`: `BlockStore`, `ChainState`, `ProducerSet`, `StateDb`, `UtxoSet`, `MaintainerState`
- `network`: `SyncManager` (for production halt)
- `mempool`: `Mempool`, `MempoolEntry`, `MempoolError`, `MempoolPolicy`
- `doli_core`: `Transaction`, `Block`, `ConsensusParams`, `NetworkParams`, `OutputType`, `TxType`
- `crypto`: `Hash`, `PublicKey`, `Signature`, address encoding/decoding

**External crates**:
- `axum` — HTTP router and WebSocket upgrade
- `tower-http` — CORS layer, request body size limit
- `reqwest` — outbound HTTP for `backfillFromPeer` and `repairArchiveFromPeer`
- `serde_json` — JSON ser/de
- `bincode` — block serialization for `getBlockRaw`
- `base64` — block data encoding
- `tokio` — async runtime, `RwLock`, `broadcast`

**Callbacks (injected at construction)**:
- `peer_count: Arc<dyn Fn() -> usize>` — live peer count
- `peer_list: Arc<dyn Fn() -> Vec<PeerInfoEntry>>` — live peer details
- `broadcast_tx: Arc<dyn Fn(Transaction)>` — gossip transaction
- `broadcast_vote: Arc<dyn Fn(Vec<u8>)>` — gossip governance vote
- `sync_status: Arc<dyn Fn() -> SyncStatus>` — live sync state
- `update_status: Arc<dyn Fn() -> Value>` — live UpdateService state

## CONSTRAINTS

**Address resolution** (`context.rs:334`):
- Accepts bech32m (`doli1…` mainnet, `tdoli1…` testnet, `ddoli1…` devnet) or 64-char hex pubkey_hash.
- `resolve_address()` uses `crypto::address::resolve()`.

**Network-specific values** (set from `NetworkParams` at construction):
- `bond_unit`: mainnet/testnet = 1,000,000,000 (10 DOLI), devnet = 100,000,000 (1 DOLI)
- `coinbase_maturity`: mainnet/testnet = 100 blocks, devnet = 10 blocks
- `blocks_per_reward_epoch`: mainnet/testnet = 360, devnet = 60
- `vesting_quarter_slots`: mainnet = 3,153,600, testnet = 2,160

**Admin method auth** (`server.rs:31-46`, `173-231`):
- All admin methods require auth from non-private IPs.
- `admin_token: None` = admin methods disabled from public IPs.
- Loopback + RFC-1918 = always trusted.

**Backfill SSRF protection** (`backfill.rs:17-72`):
- Mainnet: blocks private/loopback/link-local/broadcast/unspecified IP literals.
- Testnet/devnet: allows all IPs (localhost backfill needed on single-server setups).
- URL must use `http://` or `https://` scheme.

**Pruning minimum retention** (`pruning.rs:32-38`):
- `keep_last_n` minimum 2000 blocks — enforced via `max(2000)`.

**Attestation bitfield decode eras** (`schedule.rs:229-287`):
- Pre-`rewards_epoch_list_fix_height`: legacy sort_all by pubkey
- `rewards_epoch_list_fix_height` to `full_bitfield_decode_height`: epoch_state.producer_list only (base)
- Post-`full_bitfield_decode_height`: `[base | extra sorted by pubkey]` — full decode

**Checkpoint path safety** (`guardian.rs:88-109`):
- Rejects absolute paths and `..` components.
- Canonicalizes parent and checks it stays within `data_dir`.

**getBlockByHash height caveat** (`block.rs:28-29`):
- Always returns `height=0` — no reverse hash→height index exists.
- Use `getBlockByHeight` when height is needed.

**getUtxoDiff InMemory-only** (`stats.rs:143-147`):
- Returns error for `UtxoSet::RocksDb` variant.

**WebSocket limit** (`ws.rs:18`):
- Hard cap at 100 concurrent WebSocket connections.

**Error codes** (`error.rs:7-38`):
- Standard: `-32700` parse, `-32600` invalid_request, `-32601` method_not_found, `-32602` invalid_params, `-32603` internal_error
- Custom: `-32000` block_not_found, `-32001` tx_not_found, `-32002` invalid_tx, `-32003` tx_already_known, `-32004` mempool_full, `-32005` utxo_not_found, `-32006` producer_not_found, `-32007` pool_not_found, `-32008` unauthorized

## PATTERNS

**Constructor pattern** (use `new_for_network`, not `new`):
```
RpcContext::new_for_network(chain_state, block_store, utxo_set, mempool, params, network)
    .with_producer_set(producer_set)
    .with_peer_id(peer_id)
    .with_peer_count(|| ...)
    .with_peer_list(|| ...)
    .with_broadcast(|tx| ...)
    .with_sync_status(|| ...)
    .with_sync_manager(sync_manager)
    .with_state_db(state_db)
    .with_data_dir(data_dir)
    .with_recovery_mode(recovery_mode_arc)
```

**Bond data sourcing** — always prefer UTXO set over ProducerInfo:
- `utxo_set.count_bonds(&pubkey_hash, bond_unit)` for count
- `utxo_set.get_bond_entries(&pubkey_hash)` for per-bond details
- `utxo_set.get_bonded_balance(&pubkey_hash)` for total staked
- Fall back to `info.bond_count` / `info.bond_amount` only if UTXO count == 0 (genesis producers).

**Backfill recovery procedure**:
1. `enterRecoveryMode` — blocks inbound state mutations
2. `bridgeFromArchive` — deletes stale commitment, fills block_store from archive
3. `backfillFromPeer` — fills any remaining gaps from a peer RPC
4. `exitRecoveryMode` — restart node to clear fork block cache
5. `verifyChainIntegrity` — confirm no gaps remain

**Diff workflow across nodes**:
1. Call `getStateRootDebug` on all nodes — compare `stateRoot`, `csHash`, `utxoHash`, `psHash`
2. If `utxoHash` differs: call `getUtxoDiff` on node A (full), pass hashes to node B (`referenceHashes`)
3. If `chainCommitment` differs (from `verifyChainIntegrity`): use `backfillFromPeer` to repair

**Governance vote flow**:
1. Producer signs `"version:vote:timestamp"` with Ed25519 key
2. `submitVote` — validates producer registration + signature, broadcasts
3. `getUpdateStatus` — check veto counts

**Checkpoint restore flow** (INC-I-041 / INC-I-055):
1. Stop node → restore RocksDB checkpoint → start node
2. `enterRecoveryMode`
3. `bridgeFromArchive` (optionally `force=true` to replace divergent blocks)
4. `verifyChainIntegrity` to confirm coverage
5. `exitRecoveryMode` → restart node
