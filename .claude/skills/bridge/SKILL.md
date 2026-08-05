<!-- @INDEX
ENTRY-POINTS    10-36
OPERATIONS      38-48
DATA-FLOW       50-67
DEPENDENCIES    69-82
CONSTRAINTS     84-100
PATTERNS        102-114
@/INDEX -->

## ENTRY POINTS

| Function/Endpoint | Location | Signature | Description |
|-------------------|----------|-----------|--------------|
| `Watcher::run` | `crates/bridge/src/watcher.rs:61` | `async fn run(&mut self) -> Result<()>` | Top-level daemon loop. Pings DOLI, loads persisted swaps, starts scan at `best_height-10`, loops on `poll_interval_secs` or SIGINT |
| `Watcher::tick` (private) | `crates/bridge/src/watcher.rs:107` | `async fn tick(&mut self) -> Result<()>` | Single poll cycle: scan new HTLCs, scan preimage reveals, advance each active swap's state machine, persist |
| `Watcher::new` | `crates/bridge/src/watcher.rs:50` | `fn new(config: WatcherConfig) -> Self` | Construct watcher; builds `DoliClient`; empty swap map; `last_scanned_height=0` |
| `Watcher::active_swaps` | `crates/bridge/src/watcher.rs:319` | `fn active_swaps(&self) -> Vec<&SwapRecord>` | Non-terminal swaps (for status reporting) |
| `Watcher::get_swap` | `crates/bridge/src/watcher.rs:324` | `fn get_swap(&self, id: &str) -> Option<&SwapRecord>` | Lookup one swap by ID |
| `Watcher::swap_dir` | `crates/bridge/src/watcher.rs:314` | `fn swap_dir(&self) -> PathBuf` | `{data_dir}/swaps` |
| `SwapRecord::new` | `crates/bridge/src/swap.rs:126` | `fn new(doli_tx_hash, doli_output_index, doli_amount, doli_hash, doli_lock_height, doli_expiry_height, doli_creator, target_chain, target_address, role) -> Self` | Constructs a swap record; always starts in `SwapState::DoliLocked`; `id = "{tx_hash}:{output_index}"` |
| `SwapRecord::transition` | `crates/bridge/src/swap.rs:167` | `fn transition(&mut self, new_state: SwapState)` | Updates state + `updated_at` |
| `SwapRecord::is_terminal` | `crates/bridge/src/swap.rs:173` | `fn is_terminal(&self) -> bool` | True for `Complete \| Refunded \| Failed(_)` |
| `DoliClient::scan_for_htlcs` | `crates/bridge/src/doli.rs:183` | `async fn scan_for_htlcs(&self, from_height: u64) -> Result<Vec<DetectedHtlc>>` | Scans up to 100 DOLI blocks for new `bridgeHtlc` outputs |
| `DoliClient::scan_for_preimage_reveals` | `crates/bridge/src/doli.rs:359` | `async fn scan_for_preimage_reveals(&self, from_height: u64) -> Result<Vec<RevealedPreimage>>` | Scans up to 100 DOLI blocks for `covenantWitnesses[i].preimage` on inputs spending HTLCs |
| `DoliClient::get_bridge_utxos` | `crates/bridge/src/doli.rs:145` | `async fn get_bridge_utxos(&self, pubkey_hash: &str) -> Result<Vec<DoliUtxo>>` | Fetches UTXOs via `getUtxos`, filters `output_type=="bridgeHtlc"` — used to detect refund availability |
| `DoliClient::send_transaction` | `crates/bridge/src/doli.rs:170` | `async fn send_transaction(&self, tx_hex: &str) -> Result<String>` | Broadcasts a raw signed tx (claim/refund) via `sendTransaction` — NOT currently called by the watcher itself (see CONSTRAINTS) |
| `DoliClient::ping` | `crates/bridge/src/doli.rs:453` | `async fn ping(&self) -> bool` | Connectivity check, gates `Watcher::run` startup |
| `BitcoinClient::scan_for_htlcs` | `crates/bridge/src/bitcoin.rs:152` | `async fn scan_for_htlcs(&self, from_height: u64, watch_hashes: &[(String,String)]) -> Result<Vec<(DetectedBtcHtlc,String)>>` | Scans up to 10 BTC blocks; matches P2WSH witness-script-hash against watched hashes. **Not wired into `Watcher::tick` yet** (see CONSTRAINTS) |
| `BitcoinClient::scan_for_preimage_reveals` | `crates/bridge/src/bitcoin.rs:281` | `async fn scan_for_preimage_reveals(&self, from_height: u64, watch_outpoints: &[(String,u32,String)]) -> Result<Vec<(BtcPreimageReveal,String)>>` | Scans up to 10 BTC blocks for spends of watched HTLC outpoints, extracts preimage from `txinwitness[1]` |
| `BitcoinClient::sha256_hash` | `crates/bridge/src/bitcoin.rs:142` | `fn sha256_hash(preimage: &[u8]) -> String` | Hex SHA256 — used for BTC-side hash matching (public, sync, no RPC) |
| `EthereumClient::scan_for_htlc` | `crates/bridge/src/ethereum.rs:109` | `async fn scan_for_htlc(&self, counter_hash: &str, from_block: u64) -> Result<Option<DetectedEthHtlc>>` | `eth_getLogs` with `topics=[null,null,hashlock]`, returns first `LogHTLCNew` match |
| `EthereumClient::scan_for_preimage` | `crates/bridge/src/ethereum.rs:186` | `async fn scan_for_preimage(&self, htlc_tx_hash: &str) -> Result<Option<String>>` | Reads `eth_getTransactionReceipt` logs, extracts preimage from `LogHTLCWithdraw` data bytes `[2..66]` |
| `EthereumClient::keccak256` | `crates/bridge/src/ethereum.rs:217` | `fn keccak256(data: &[u8]) -> Vec<u8>` | 32-byte hash via `tiny_keccak` — ETH-side hash matching |
| lib re-exports | `crates/bridge/src/lib.rs:13-15` | `pub use {error::{BridgeError,Result}, swap::{SwapRecord,SwapRole,SwapState}, watcher::{Watcher,WatcherConfig}}` | Public surface of the `bridge` crate (package name is literally `bridge`, not `doli-bridge` — see Cargo.toml) |
| CLI: `doli bridge-refund {swap_id}` | outside domain (bins/cli, not located — `rg` unavailable this session) | n/a | Referenced by `watcher.rs:236` log message as the manual refund entry point. **[UNCLEAR]** exact CLI file/line — verify with grep in `bins/cli/src/` |
| CLI: `cmd_bridge_status` | outside domain (bins/cli, not located — `rg` unavailable this session) | n/a | Referenced by doc comment `ethereum.rs:4` as consumer of `EthereumClient`. **[UNCLEAR]** exact CLI file/line |

## OPERATIONS

| Task | Steps | Commands/Functions | Inputs | Success |
|------|-------|--------------------|--------|---------|
| Run the bridge watcher daemon | 1. Build `WatcherConfig` (doli_rpc, our_pubkey_hash, data_dir, poll_interval_secs) 2. `Watcher::new(config)` 3. `watcher.run().await` | `Watcher::new()`, `Watcher::run()` | Reachable DOLI RPC endpoint; writable `data_dir` | Logs `"Bridge watcher started"`; ticks every `poll_interval_secs`; SIGINT saves state and exits cleanly |
| Initiate a swap (lock DOLI first, as Initiator) | 1. [outside this crate] build+broadcast a `BridgeHTLC` output tx on DOLI 2. Watcher's next tick calls `DoliClient::scan_for_htlcs()` 3. `creator_pubkey_hash == our_pubkey_hash` → role=`Initiator` 4. `SwapRecord::new(...)` inserted, state=`DoliLocked` | `DoliClient::scan_for_htlcs`, `SwapRecord::new` | Signed DOLI tx with `outputType=bridgeHtlc`, valid hashlock/timelock condition tree | New swap appears in `active_swaps()`; log `"[WATCHER] New BridgeHTLC detected"` |
| Detect a counterparty response (Responder path) | 1. Counterparty creates DOLI HTLC where `creator_pubkey_hash != our_pubkey_hash` 2. Watcher tags role=`Responder` 3. Operator manually locks BTC/ETH HTLC (**no automation**) 4. [TODO, unimplemented] transition to `BothLocked` when counter-chain lock detected | `DoliClient::scan_for_htlcs` (role assignment only) | — | State stays `DoliLocked` until counter-chain scanning is wired in (currently a TODO, `watcher.rs:200-201`) |
| Claim a swap (reveal preimage on DOLI) | 1. [outside this crate] operator/wallet spends the BridgeHTLC UTXO with the preimage 2. Watcher tick calls `DoliClient::scan_for_preimage_reveals()` 3. Matches `htlc_tx_hash:htlc_output_index` to tracked swap 4. `SwapRecord.preimage` set, state→`PreimageRevealed` 5. Next tick auto-transitions →`Complete` (DOLI side only; counter-chain auto-claim is a TODO) | `DoliClient::scan_for_preimage_reveals`, `SwapRecord::transition` | A spend tx of the HTLC UTXO with `covenantWitnesses[i].preimage` set | Swap reaches `state=Complete` (terminal); `is_terminal()==true` |
| Refund an expired swap | 1. Watcher tick detects `current_height >= swap.doli_expiry_height` → transitions to `Expired` 2. For `role=Initiator` with no `doli_refund_tx`, watcher checks `get_bridge_utxos()` for UTXO still present 3. If still locked, watcher **only logs** `"refund available (use doli bridge-refund {swap_id})"` — operator must run CLI refund manually 4. Once UTXO is gone (spent), watcher transitions →`Refunded` | `DoliClient::get_bridge_utxos`, manual `doli bridge-refund` CLI (outside crate) | Chain height past `doli_expiry_height`; wallet access to sign refund tx | Swap reaches `state=Refunded` (terminal) after refund tx confirms and next tick observes UTXO gone |
| Check swap status | 1. `watcher.active_swaps()` for in-flight swaps, or `watcher.get_swap(id)` for one | `Watcher::active_swaps`, `Watcher::get_swap` | Swap ID (`"{doli_tx_hash}:{doli_output_index}"`) | Returns `&SwapRecord` with current `state`/`role`/timestamps |
| Recover watcher state after crash/restart | 1. `Watcher::run()` calls `load_swaps()` before scanning 2. Reads every `*.json` in `{data_dir}/swaps/` 3. Non-JSON files skipped; parse failures logged and skipped (not fatal) | `Watcher::load_swaps` (private, invoked by `run`) | Prior `save_swaps()` output on disk | All persisted swaps reappear in `swaps` map; log `"Loaded N persisted swap(s)"` |

## DATA FLOW

| Input | Transform | Output | Location |
|-------|-----------|--------|----------|
| DOLI `getBlockByHeight` block JSON | `scan_block_for_htlcs`: filter outputs where `outputType=="bridgeHtlc"`, parse condition tree for hashlock/timelock/expiry | `DetectedHtlc` per matching output | `crates/bridge/src/doli.rs:221-292` |
| `DetectedHtlc` + `our_pubkey_hash` | Role assignment: `creator_pubkey_hash == our_pubkey_hash ? Initiator : Responder` | `SwapRecord` (state=`DoliLocked`) inserted into `HashMap<id,SwapRecord>` | `crates/bridge/src/watcher.rs:126-152` |
| DOLI block JSON (spending tx) | `scan_block_for_reveals`: match `covenantWitnesses[i].preimage` to `inputs[i]` prev-outpoint | `RevealedPreimage{htlc_tx_hash, htlc_output_index, claim_tx_hash, preimage}` | `crates/bridge/src/doli.rs:391-444` |
| `RevealedPreimage` | Lookup swap by `"{htlc_tx_hash}:{htlc_output_index}"`; if `preimage.is_none()` set preimage/source/claim_tx, `transition(PreimageRevealed)` | Mutated `SwapRecord` | `crates/bridge/src/watcher.rs:160-175` |
| `SwapRecord.state==PreimageRevealed` (next tick) | Direct transition, no further chain check (DOLI-side auto-complete) | `state=Complete` | `crates/bridge/src/watcher.rs:213-219` |
| `current_height >= doli_expiry_height` | Transition check on `DoliLocked`/`BothLocked` states | `state=Expired` | `crates/bridge/src/watcher.rs:191-198`, `204-208` |
| `Expired` + `role==Initiator` + `doli_refund_tx==None` | `get_bridge_utxos(doli_creator)` → check if HTLC UTXO still present | Log-only (no auto-refund) OR `state=Refunded` if UTXO already spent | `crates/bridge/src/watcher.rs:220-247` |
| BTC raw preimage (32 bytes, hex) | `sha256_hash()` — same preimage, different hash function than DOLI's BLAKE3 | SHA256 hex digest for BTC P2WSH witness-script matching | `crates/bridge/src/bitcoin.rs:142-146` |
| BTC block JSON | `scan_block_for_htlcs`: validate `scriptPubKey.type=="witness_v0_scripthash"` AND `scriptPubKey.hex` is `0020<32-byte-hash>`, compare `witness_script_hash` to `watch_hashes` | `(DetectedBtcHtlc, swap_id)` pairs | `crates/bridge/src/bitcoin.rs:192-275` |
| BTC block JSON (spending tx) | `scan_block_for_reveals`: match `vin.{txid,vout}` to watched outpoints, extract `txinwitness[1]` (32-byte preimage) | `(BtcPreimageReveal, swap_id)` pairs | `crates/bridge/src/bitcoin.rs:316-384` |
| ETH `eth_getLogs` response | Filter by `topics=[null,null,hashlock]`, take first match, decode `blockNumber`/`data` | `DetectedEthHtlc` | `crates/bridge/src/ethereum.rs:109-181` |
| ETH `eth_getTransactionReceipt` logs | Scan for non-zero 32-byte value in `data[2..66]` | `Option<String>` preimage hex | `crates/bridge/src/ethereum.rs:186-214` |
| `SwapRecord` (any state) | `save_swaps()`: serialize each swap to JSON, filename = `id.replace(':','_') + ".json"` | `{data_dir}/swaps/{tx}_{idx}.json` | `crates/bridge/src/watcher.rs:300-311` |
| `{data_dir}/swaps/*.json` | `load_swaps()`: parse each file into `SwapRecord`, insert into map keyed by `swap.id` | `HashMap<String,SwapRecord>` | `crates/bridge/src/watcher.rs:273-297` |

## DEPENDENCIES

| This Domain Uses | Skill File | What For |
|-------------------|-----------|----------|
| DOLI RPC: `getChainInfo`, `getUtxos`, `getTransaction`, `sendTransaction`, `getBlockByHeight` | `.claude/skills/doli-network/SKILL.md` | Watcher's only channel into DOLI chain state — block scanning, UTXO lookups, tx broadcast |
| `bridgeHtlc` output type + condition tree (`Hashlock`/`Timelock`/`TimelockExpiry`) | (core/transaction domain skill, not this crate) | Bridge watcher parses these from RPC JSON; the wire format itself is defined in `crates/core` (transaction.rs), not in `crates/bridge` |
| `doli-core`, `crypto` (declared in `Cargo.toml:11-12`) | n/a | **Declared workspace dependencies with NO import found in any of the 6 source files read** ([UNCLEAR] — possibly dead deps, or used only via re-export/feature not exercised in current code; verify with `cargo tree -p bridge` or a build check before removing) |
| External: Bitcoin Core JSON-RPC (`getblockcount`,`getblockhash`,`getblock` verbosity=2,`getrawtransaction`) | none (external chain, no in-repo skill) | P2WSH HTLC detection + preimage extraction on Bitcoin |
| External: Ethereum JSON-RPC (`eth_blockNumber`,`eth_getLogs`,`eth_getTransactionReceipt`) | none (external chain, no in-repo skill) | Standard HashedTimelock contract event scanning |

| Used By | Skill File | What For |
|---------|-----------|----------|
| CLI `doli bridge-refund {swap_id}` (referenced in `watcher.rs:236` log text) | `.claude/skills/cli/SKILL.md` | Manual refund execution once watcher flags an `Expired` swap still locked — **not located in this session** ([UNCLEAR], `rg` unavailable; verify path in `bins/cli/src/`) |
| CLI `cmd_bridge_status` (referenced in `ethereum.rs:4` doc comment) | `.claude/skills/cli/SKILL.md` | Presumed consumer of `EthereumClient` for swap status display — **not located in this session** ([UNCLEAR]) |

## CONSTRAINTS

| Constraint | Type | Location | Detail |
|-----------|------|----------|--------|
| Hash function asymmetry across chains | invariant | `crates/bridge/src/bitcoin.rs:138-146` | DOLI hashlock = BLAKE3(`"DOLI_HASHLOCK"` \|\| preimage). Bitcoin = SHA256(preimage). Ethereum = keccak256(preimage). Same raw preimage bytes, three different hash functions — never compare hashes cross-chain directly |
| P2WSH script-type validation (AUDIT-BRIDGE-002) | security | `crates/bridge/src/bitcoin.rs:226-257` | Previously matched by substring on script ASM (spoofable via OP_RETURN or other script types embedding the hash bytes). Now REQUIRES `scriptPubKey.type=="witness_v0_scripthash"` AND `scriptPubKey.hex` exactly 68 hex chars starting with `"0020"` before comparing the witness-script hash. Do not regress to substring matching |
| Auto-refund NOT implemented | edge-case | `crates/bridge/src/watcher.rs:220-247` | Watcher detects `Expired` + UTXO-still-locked and only logs a message pointing at manual CLI refund. No wallet-signing path exists in this crate |
| Auto-claim on counter-chain NOT implemented | edge-case | `crates/bridge/src/watcher.rs:200-201, 210-211, 215` | `BothLocked` counter-chain detection and `PreimageRevealed` auto-claim on BTC/ETH are TODO stubs. `BitcoinClient`/`EthereumClient` scan methods exist but are **not called from `Watcher::tick`** — they are currently dead code from the watcher's perspective |
| Block scan caps | performance | `crates/bridge/src/doli.rs:196` (100 DOLI blocks/tick), `crates/bridge/src/bitcoin.rs:168` (10 BTC blocks/tick) | Prevents runaway backfills after long downtime. Do not remove without adding pagination/resumability |
| Watcher starts near chain tip, not from genesis | edge-case | `crates/bridge/src/watcher.rs:84` | `last_scanned_height = best_height.saturating_sub(10)` on startup. Active swaps older than 10 blocks are recovered ONLY from persisted JSON, not re-derived from chain scan |
| `Expired` is NOT a terminal state | invariant | `crates/bridge/src/swap.rs:173-178` | Only `Complete \| Refunded \| Failed(_)` are terminal (`is_terminal()`). The watcher must keep polling `Expired` swaps to detect eventual `Refunded` |
| BTC witness preimage index and length | invariant | `crates/bridge/src/bitcoin.rs:356-362` | Expects preimage at `txinwitness[1]` in a `[sig, preimage, OP_TRUE, script]` stack (len>=3 required); validates exactly 64 hex chars (32 bytes) before accepting |
| ETH preimage extraction assumes fixed byte offset | edge-case | `crates/bridge/src/ethereum.rs:204-208` | Reads `data[2..66]` (first 32 bytes after `0x`) of the FIRST log with a non-zero value in every `LogHTLCWithdraw`-shaped log on the receipt — does not verify event signature/topic0 before extracting, so a receipt with multiple logs could mis-attribute the preimage to the wrong event |
| Role determination is one-shot at detection time | invariant | `crates/bridge/src/watcher.rs:126-130` | `creator_pubkey_hash == our_pubkey_hash` decides `Initiator` vs `Responder` permanently at `SwapRecord::new()` — never re-evaluated |
| Persistence is best-effort, not atomic | edge-case | `crates/bridge/src/watcher.rs:300-311` | `save_swaps()` writes each swap file individually via `std::fs::write` (not a temp-file+rename pattern) — a crash mid-write could leave a partially-written/corrupt swap JSON, caught only as a parse-warning on next `load_swaps()` |
| Graceful RPC-failure degradation on refund check | edge-case | `crates/bridge/src/watcher.rs:224-228` | `get_bridge_utxos()` failure defaults to `unwrap_or_default()` (empty vec) — an RPC hiccup during expiry check is silently treated as "UTXO not found", which would incorrectly transition the swap to `Refunded` if this ever races with a real outage. Worth flagging: a transient DOLI RPC error at exactly the wrong tick could falsely mark a swap `Refunded` |
| Package name is `bridge`, not `doli-bridge` | invariant | `crates/bridge/Cargo.toml:2` | Dependency declarations elsewhere in the workspace must reference `bridge = { workspace = true }`, not `doli-bridge` |

## PATTERNS

| Pattern | Example Location | Usage |
|---------|-------------------|-------|
| Role determination pattern | `crates/bridge/src/watcher.rs:126-129` | Compare `htlc.creator_pubkey_hash == config.our_pubkey_hash`. Match → `Initiator`. Mismatch → `Responder` |
| Generic JSON-RPC call pattern (DOLI) | `crates/bridge/src/doli.rs:112-137` | `DoliClient::call<P,R>(method, params) -> Result<R>` — jsonrpc 2.0 envelope, checks `resp.error` before unwrapping `resp.result` |
| Raw `serde_json::Value` RPC pattern (BTC/ETH) | `crates/bridge/src/bitcoin.rs:72-105`, `crates/bridge/src/ethereum.rs:58-88` | Both build the request inline as `json!({...})`, check `resp["error"]` for non-null, then `resp["result"]`. BTC additionally sets HTTP Basic Auth from `"user:pass"` split |
| Condition-tree recursive traversal | `crates/bridge/src/doli.rs:329-356` | DOLI HTLC conditions serialize as a nested JSON tree (`Or`/`And`/`Hashlock`/`Timelock`). `find_in_condition()` recurses through `left`/`right`/`a`/`b`/`conditions` keys to locate a condition type by string tag |
| Swap ID format | `crates/bridge/src/swap.rs:138`, `crates/bridge/src/watcher.rs:120-121`, `crates/bridge/src/watcher.rs:305` | Always `"{tx_hash}:{output_index}"` for map lookup; colon replaced with underscore only for filesystem filenames |
| Scan-continuity-over-perfection | `crates/bridge/src/doli.rs:198-206`, `crates/bridge/src/bitcoin.rs:170-177` | Per-block scan errors are logged (`debug!`) and skipped, not propagated — `last_scanned_height` still advances so one bad block doesn't wedge the watcher |
| Persistence-after-every-tick | `crates/bridge/src/watcher.rs:256` | Swaps are re-serialized to disk after EVERY tick, not just on state change — bounds crash-recovery data loss to at most one tick |
| Script-type allowlist before substring match (security hardening) | `crates/bridge/src/bitcoin.rs:241-251` | Validate `scriptPubKey.type` and exact byte-length/prefix BEFORE comparing embedded hash bytes — replaces the old (spoofable) raw-ASM substring search. Replicate this shape for any future "match bytes inside untrusted structured data" code |
| Test structure: unit tests colocated per-file | `crates/bridge/src/watcher.rs:329-529`, `crates/bridge/src/bitcoin.rs:392-414`, `crates/bridge/src/ethereum.rs:227-249` | `#[cfg(test)] mod tests` at the bottom of each file; watcher tests use `tempfile::TempDir` for on-disk persistence round-trips, hash tests use known golden vectors |
