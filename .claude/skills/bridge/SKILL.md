# bridge — DOLI Cross-Chain Bridge
<!-- @INDEX
ENTRY-POINTS: lines 17-30
STRUCTS: lines 33-93
FUNCTIONS: lines 96-175
DATA-FLOWS: lines 178-207
DEPENDENCIES: lines 210-222
CONSTRAINTS: lines 225-248
PATTERNS: lines 251-270
-->

## ENTRY-POINTS

`crates/bridge/src/lib.rs` — public re-exports:
- `BridgeError`, `Result` (from `error`)
- `SwapRecord`, `SwapRole`, `SwapState` (from `swap`)
- `Watcher`, `WatcherConfig` (from `watcher`)

`watcher.rs:61` — `Watcher::run(&mut self) -> Result<()>`
  Top-level async daemon loop. Call once; runs until SIGINT.
  Prerequisite: DOLI node reachable. Saves state on shutdown.

`watcher.rs:107` — `Watcher::tick(&mut self) -> Result<()>` (private)
  Single poll cycle. Called every `poll_interval_secs`. Scans DOLI blocks, updates swap state machine, persists to disk.

## STRUCTS

### swap.rs

`SwapState` (enum, line 15) — lifecycle states:
- `DoliLocked` → initial state after DOLI HTLC detected
- `BothLocked` → counterparty locked on target chain
- `PreimageRevealed` → preimage found on either chain
- `Complete` → claimed on both chains (terminal)
- `Expired` → HTLC timed out, refund path available (NOT terminal)
- `Refunded` → refunded on DOLI (terminal)
- `Failed(String)` → error during processing (terminal)

`SwapRole` (enum, line 41):
- `Initiator` — we locked DOLI first, expecting counterparty BTC lock
- `Responder` — counterparty locked DOLI, we lock on target chain

`SwapRecord` (struct, line 50) — full swap state record:
- `id: String` — `"{doli_tx_hash}:{doli_output_index}"`
- `state: SwapState`, `role: SwapRole`, `target_chain: u8` (1=BTC, 2=ETH)
- DOLI side: `doli_tx_hash`, `doli_output_index`, `doli_amount`, `doli_hash` (BLAKE3 domain-separated), `doli_lock_height`, `doli_expiry_height`, `doli_creator`
- Counterparty side: `counter_tx_hash?`, `counter_amount?`, `counter_hash?` (SHA256 for BTC)
- Preimage: `preimage?`, `preimage_source?` ("doli" or "bitcoin"/"ethereum")
- Results: `doli_claim_tx?`, `counter_claim_tx?`, `doli_refund_tx?`
- Timestamps: `created_at`, `updated_at` (UTC)

### watcher.rs

`WatcherConfig` (struct, line 23):
- `doli_rpc: String` — e.g., `"http://127.0.0.1:8500"`
- `our_pubkey_hash: String` — identifies our own HTLCs vs counterparty HTLCs
- `btc_rpc: Option<String>`, `eth_rpc: Option<String>`
- `data_dir: PathBuf` — swap JSON files at `{data_dir}/swaps/*.json`
- `poll_interval_secs: u64`

`Watcher` (struct, line 39):
- `config: WatcherConfig`, `doli: DoliClient`
- `swaps: HashMap<String, SwapRecord>` — keyed by swap ID
- `last_scanned_height: u64`

### doli.rs

`DoliClient` (struct, line 37) — HTTP JSON-RPC client (10s timeout):
- `endpoint: String`, `client: reqwest::Client`

`ChainInfo` (struct, line 44): `best_height`, `best_hash`, `best_slot` (camelCase RPC)

`DoliUtxo` (struct, line 53): `tx_hash`, `output_index`, `amount`, `output_type`, `lock_until`, `spendable`, `pubkey_hash?`, `bridge?: BridgeMetadata`

`BridgeMetadata` (struct, line 67): `target_chain?`, `target_chain_id?`, `target_address?`

`DetectedHtlc` (struct, line 75): `tx_hash`, `output_index`, `amount`, `hash`, `lock_height`, `expiry_height`, `creator_pubkey_hash`, `target_chain`, `target_address`

`RevealedPreimage` (struct, line 89): `htlc_tx_hash`, `htlc_output_index`, `claim_tx_hash`, `preimage`

### bitcoin.rs

`BitcoinClient` (struct, line 23) — Bitcoin Core JSON-RPC (30s timeout, basic auth):
- `endpoint`, `auth` (`"user:pass"` format), `client`

`DetectedBtcHtlc` (struct, line 31): `txid`, `vout`, `amount_sat`, `confirmations`, `hash`, `locktime`

`BtcPreimageReveal` (struct, line 47): `htlc_txid`, `htlc_vout`, `claim_txid`, `preimage`

### ethereum.rs

`EthereumClient` (struct, line 16) — Ethereum JSON-RPC (30s timeout):
- `endpoint`, `client`

`DetectedEthHtlc` (struct, line 23): `tx_hash`, `block_number`, `confirmations`, `amount: String` (wei as string), `token_address`

`EthPreimageReveal` (struct, line 38): `tx_hash`, `preimage` (hex, no 0x prefix)

## FUNCTIONS

### SwapRecord methods (swap.rs)

`SwapRecord::new(...)` line 126 — constructor; 10 args; initial state always `DoliLocked`; ID = `"{doli_tx_hash}:{doli_output_index}"`

`SwapRecord::transition(&mut self, new_state: SwapState)` line 167 — update state + timestamp

`SwapRecord::is_terminal(&self) -> bool` line 173 — true for `Complete | Refunded | Failed(_)`

### Watcher methods (watcher.rs)

`Watcher::new(config: WatcherConfig) -> Self` line 50 — creates DoliClient; swaps empty; last_scanned_height=0

`Watcher::run(&mut self) -> Result<()>` line 61 — verifies DOLI ping; loads swaps; starts from `best_height - 10`; loops on SIGINT or interval

`Watcher::active_swaps(&self) -> Vec<&SwapRecord>` line 319 — non-terminal swaps

`Watcher::get_swap(&self, id: &str) -> Option<&SwapRecord>` line 323

`Watcher::swap_dir(&self) -> PathBuf` line 314 — `{data_dir}/swaps`

`Watcher::load_swaps(&mut self) -> Result<()>` line 273 — reads `{data_dir}/swaps/*.json`; skip non-JSON files; warn on parse failure (no abort)

`Watcher::save_swaps(&self) -> Result<()>` line 300 — writes each swap to `{swap.id.replace(':','_')}.json`; called after every tick

### DoliClient methods (doli.rs)

`DoliClient::new(endpoint: &str) -> Self` line 101

`DoliClient::get_chain_info() -> Result<ChainInfo>` line 140 — calls `getChainInfo`

`DoliClient::get_bridge_utxos(pubkey_hash) -> Result<Vec<DoliUtxo>>` line 145 — calls `getUtxos`, filters `output_type == "bridgeHtlc"`

`DoliClient::get_transaction(tx_hash) -> Result<Value>` line 164 — calls `getTransaction`

`DoliClient::send_transaction(tx_hex) -> Result<String>` line 170 — calls `sendTransaction`, returns hash

`DoliClient::scan_for_htlcs(from_height) -> Result<Vec<DetectedHtlc>>` line 183 — scans up to 100 blocks at a time via `getBlockByHeight`; parses `outputType == "bridgeHtlc"`; extracts condition tree (Or/And/Hashlock/Timelock)

`DoliClient::scan_for_preimage_reveals(from_height) -> Result<Vec<RevealedPreimage>>` line 359 — scans `covenantWitnesses[i].preimage` in spending transactions

`DoliClient::height() -> Result<u64>` line 447

`DoliClient::ping() -> bool` line 453

### BitcoinClient methods (bitcoin.rs)

`BitcoinClient::new(endpoint, auth) -> Self` line 59 — auth = `"user:pass"`

`BitcoinClient::sha256_hash(preimage: &[u8]) -> String` line 142 — returns hex SHA256; used for BTC HTLC script matching (DOLI uses BLAKE3, Bitcoin uses SHA256 of SAME preimage)

`BitcoinClient::scan_for_htlcs(from_height, watch_hashes) -> Result<Vec<(DetectedBtcHtlc, String)>>` line 152 — scans up to 10 BTC blocks; matches P2WSH by witness_script_hash; returns `(htlc, swap_id)` pairs

`BitcoinClient::scan_for_preimage_reveals(from_height, watch_outpoints) -> Result<Vec<(BtcPreimageReveal, String)>>` line 281 — scans up to 10 BTC blocks; checks `txinwitness[1]` (32 bytes = preimage) for known outpoints

`BitcoinClient::get_block_count() -> Result<u64>` line 108

`BitcoinClient::ping() -> bool` line 387

### EthereumClient methods (ethereum.rs)

`EthereumClient::new(endpoint) -> Self` line 46

`EthereumClient::scan_for_htlc(counter_hash, from_block) -> Result<Option<DetectedEthHtlc>>` line 109 — uses `eth_getLogs` with topic[2]=hashlock; returns first matching `LogHTLCNew` event

`EthereumClient::scan_for_preimage(htlc_tx_hash) -> Result<Option<String>>` line 186 — reads `eth_getTransactionReceipt`, extracts preimage from `LogHTLCWithdraw` log data bytes 2..66

`EthereumClient::get_block_number() -> Result<u64>` line 96 — calls `eth_blockNumber` (hex decode)

`EthereumClient::keccak256(data: &[u8]) -> Vec<u8>` line 217 — returns 32-byte hash via `tiny_keccak`

`EthereumClient::ping() -> bool` line 91

## DATA-FLOWS

### Swap initiation (Initiator path)
```
User builds BridgeHTLC tx (DOLI chain)
  → doli_tx broadcast
  → DoliClient::scan_for_htlcs() detects it
  → Watcher creates SwapRecord (state=DoliLocked, role=Initiator)
  → Counterparty detects DOLI lock → locks BTC/ETH HTLC
  → [TODO] BitcoinClient/EthereumClient detects counter lock
  → SwapRecord transitions DoliLocked → BothLocked
  → Counterparty claims DOLI → reveals preimage
  → DoliClient::scan_for_preimage_reveals() detects it
  → SwapRecord: state=PreimageRevealed, preimage stored
  → [TODO] auto-claim on BTC/ETH using preimage
  → SwapRecord → Complete
```

### Swap initiation (Responder path)
```
Counterparty creates DOLI HTLC (creator_pubkey_hash != our_pubkey_hash)
  → Watcher detects as role=Responder
  → SwapRecord state=DoliLocked
  → We must lock BTC/ETH [manual / TODO]
  → SwapRecord → BothLocked
  → We claim DOLI with preimage → preimage revealed on DOLI
  → DoliClient::scan_for_preimage_reveals() picks it up
  → SwapRecord → PreimageRevealed → Complete
```

### Expiry / Refund path
```
SwapRecord.doli_expiry_height reached (watcher tick check)
  → SwapRecord → Expired
  → Initiator role + UTXO still exists:
      watcher logs "refund available (use `doli bridge-refund {swap_id}`)"
      NOTE: auto-refund NOT implemented — requires wallet signing
  → If UTXO already spent → SwapRecord → Refunded
```

### State persistence
```
load_swaps() at startup: {data_dir}/swaps/*.json → HashMap<id, SwapRecord>
save_swaps() after every tick: SwapRecord → {id.replace(':','_')}.json
File naming: "aabbccdd:0" → "aabbccdd_0.json"
```

## DEPENDENCIES

Crate-level (`Cargo.toml` implied by imports):
- `reqwest` — async HTTP for all RPC calls
- `serde`, `serde_json` — serialization
- `thiserror` — BridgeError derive
- `chrono` — SwapRecord timestamps (UTC)
- `tracing` — structured logging
- `tokio` — async runtime, signal handling, sleep
- `sha2` — SHA256 for BTC hash matching (`bitcoin.rs`)
- `tiny_keccak` — keccak256 for ETH hash matching (`ethereum.rs`)
- `hex` — hex encode/decode
- `tempfile` — test-only (watcher tests)

DOLI RPC methods used by bridge:
- `getChainInfo` — height/hash/slot
- `getUtxos` — bridge UTXO query (filter `output_type=bridgeHtlc`)
- `getTransaction` — tx lookup
- `sendTransaction` — broadcast raw tx
- `getBlockByHeight` — block scan for HTLCs and reveals

Bitcoin Core RPC methods used:
- `getblockcount`, `getblockhash`, `getblock` (verbosity=2), `getrawtransaction`

Ethereum RPC methods used:
- `eth_blockNumber`, `eth_getLogs`, `eth_getTransactionReceipt`

## CONSTRAINTS

**Hash function asymmetry**: DOLI hashlock = BLAKE3("DOLI_HASHLOCK" || preimage). Bitcoin hashlock = SHA256(preimage). Ethereum hashlock = keccak256(preimage). All three chains use the SAME raw preimage; only the hash function differs. (`bitcoin.rs:140-146`)

**P2WSH matching**: Bitcoin HTLC detection matches by `witness_script_hash` (SHA256 of the full witness script), NOT by the HTLC preimage hash directly. The watcher must compute SHA256(witness_script) and pass it as `watched_hash`. (`bitcoin.rs:227-257`)

**Block scan cap**: DOLI scans max 100 blocks/tick (`doli.rs:196`). Bitcoin/Ethereum scan max 10 blocks/tick (`bitcoin.rs:168`, `ethereum.rs`). Do not remove — prevents runaway backfills.

**Auto-refund NOT implemented**: The watcher detects expired HTLCs and logs the condition, but does NOT sign or broadcast refund transactions — requires wallet access. Users must execute `doli bridge-refund {swap_id}` manually. (`watcher.rs:239`)

**Auto-claim on counter-chain NOT implemented** (TODOs in tick() at lines 200-201, 210-211, 215): BothLocked counter-chain detection and PreimageRevealed auto-claim are stubs.

**Responder role persistence**: `our_pubkey_hash` in WatcherConfig identifies our own HTLCs. HTLCs where `creator_pubkey_hash != our_pubkey_hash` → `SwapRole::Responder`. (`watcher.rs:126-129`)

**Initiator starts from current height minus 10**: On startup, watcher does not replay full history. Only reprocesses last 10 blocks. Older active swaps are recovered from persisted JSON. (`watcher.rs:84`)

**BTC witness preimage index**: Expects preimage at `txinwitness[1]` (sig=0, preimage=1, OP_TRUE=2, script=3). Validates exactly 64 hex chars (32 bytes). (`bitcoin.rs:358-362`)

**ETH preimage extraction**: Reads bytes [2..66] of `LogHTLCWithdraw` event data (skipping 0x prefix). Skips if all zeros. (`ethereum.rs:204-208`)

**Expired ≠ terminal**: `SwapState::Expired` is NOT a terminal state — the watcher continues checking for UTXO spend to transition to `Refunded`. Only `Complete | Refunded | Failed(_)` are terminal. (`swap.rs:173-178`)

## PATTERNS

**Role determination pattern**: Compare `htlc.creator_pubkey_hash == config.our_pubkey_hash`. Match → Initiator. Mismatch → Responder. (`watcher.rs:126-129`)

**RPC call pattern (DOLI)**: Generic `DoliClient::call<P,R>(method, params) -> Result<R>` with jsonrpc 2.0. Checks `resp.error` first, then unwraps `resp.result`. (`doli.rs:112-137`)

**RPC call pattern (BTC/ETH)**: Both use raw `serde_json::Value` response with inline error check on `resp["error"]`. BTC adds basic auth header. (`bitcoin.rs:72-105`, `ethereum.rs:58-88`)

**Condition tree traversal**: DOLI HTLC conditions serialized as nested JSON tree (Or/And/Hashlock/Timelock). `find_in_condition()` recurses through `left`, `right`, `a`, `b`, `conditions` keys. (`doli.rs:329-356`)

**Swap ID format**: Always `"{tx_hash}:{output_index}"` for both lookup and file naming (colon replaced with underscore for filenames). (`swap.rs:138`, `watcher.rs:121`, `watcher.rs:305`)

**Scan continuity**: `last_scanned_height` advances to `current_height` at end of every successful tick. Errors in individual block scans are logged and skipped (not fatal) to preserve scan continuity. (`doli.rs:200-203`, `bitcoin.rs:172-175`)

**Graceful degradation**: `get_bridge_utxos()` called with `unwrap_or_default()` in expired swap check — RPC failure does not crash the watcher. (`watcher.rs:226-228`)

**Persistence-first design**: Swaps persisted after EVERY tick (not just on state change). Enables crash recovery with at-most-one-tick data loss. (`watcher.rs:256`)
