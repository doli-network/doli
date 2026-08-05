<!-- @INDEX
ENTRY-POINTS    15-105
OPERATIONS      107-129
DATA-FLOW       131-183
DEPENDENCIES    185-208
CONSTRAINTS     210-225
PATTERNS        227-272
STRUCTS         274-304
@/INDEX -->

# gui — DOLI Desktop GUI (Tauri 2.x)

`bins/gui` — Tauri 2.x desktop wallet/node-manager binary (`doli-gui`). Rust backend bridges a Svelte frontend to the `wallet` crate. Depends on `wallet` crate, NOT `bins/node` directly (spawns `doli-node` as a child process instead of linking it).

## ENTRY POINTS

| Function/Endpoint | Location | Signature | Description |
|-------------------|----------|-----------|--------------|
| `main` | `bins/gui/src/main.rs:21` | `fn main()` | Creates `AppState`, auto-starts embedded node (best-effort), registers all Tauri commands + plugins, runs event loop. |
| `AppState::new` | `bins/gui/src/state.rs:35` | `fn new() -> Self` | Loads persisted config, creates `NodeManager`, initializes `RpcClient` (local node URL unless custom override configured). |
| `NodeManager::start` | `bins/gui/src/node_manager.rs:68` | `fn start(&mut self) -> Result<(), String>` | Spawns `doli-node` child process; finds binary via sibling-dir then PATH. |
| `NodeManager::stop` | `bins/gui/src/node_manager.rs:121` | `fn stop(&mut self) -> Result<(), String>` | Graceful shutdown (SIGTERM → 10s wait → SIGKILL). |
| `NodeManager::drop` | `bins/gui/src/node_manager.rs:232` (impl Drop) | `fn drop(&mut self)` | Ensures node is shut down on GUI exit even without explicit `stop_node` call. |

### Tauri Commands (all `async fn`, `Result<T, String>`, `State<'_, AppState>` injected)

**Wallet** — `bins/gui/src/commands/wallet.rs`
| Command | Line | Signature (args) | Description |
|---------|------|-------------------|--------------|
| `create_wallet` | :39 | `(name, wallet_path, state)` → `CreateWalletResponse` | New BIP-39 wallet; returns seed phrase once (never persisted). |
| `restore_wallet` | :74 | `(name, seed_phrase, wallet_path, state)` → `WalletInfo` | Restore from mnemonic, saves to disk. |
| `load_wallet` | :101 | `(wallet_path, state)` → `WalletInfo` | Load wallet file from disk. |
| `generate_address` | :123 | `(label: Option<String>, state)` → `AddressInfo` | Derives new address, saves wallet. |
| `list_addresses` | :168 | `(state)` → `Vec<AddressInfo>` | Lists addresses; silently skips corrupted entries. |
| `export_wallet` | :206 | `(destination, state)` → `()` | Exports wallet file. |
| `import_wallet` | :218 | `(source, destination, state)` → `WalletInfo` | Imports wallet, saves to new path. |
| `wallet_info` | :244 | `(state)` → `WalletInfo` | Public wallet metadata (no keys). |
| `add_bls_key` | :253 | `(state)` → `String` | Generates BLS key for producer registration, returns pubkey hex. |

**Transaction** — `bins/gui/src/commands/transaction.rs`
| Command | Line | Signature (args) | Description |
|---------|------|-------------------|--------------|
| `get_balance` | :14 | `(address: Option<String>, state)` → `BalanceResponse` | Balance for address or primary wallet address via RPC. |
| `send_doli` | :53 | `(to, amount, fee: Option<String>, state)` → `SendResponse` | Full signing flow in Rust: fetch UTXOs → build TX → sign → submit. |
| `get_history` | :127 | `(limit: Option<u32>, state)` → `Vec<HistoryEntryResponse>` | TX history for primary address (default limit 50). |

**Producer** — `bins/gui/src/commands/producer.rs`
| Command | Line | Signature (args) | Description |
|---------|------|-------------------|--------------|
| `producer_status` | :14 | `(state)` → `ProducerStatusResponse` | Checks registration by scanning producers list via RPC. |
| `register_producer` | :53 | `(bond_count: u32, state)` → `TxResponse` | Builds `Registration` TX: hash-chain VDF proof, BLS PoP, bond outputs. Requires BLS key in wallet. |
| `add_bonds` | :256 | `(count: u32, state)` → `TxResponse` | Adds bond UTXOs via `TxBuilder::build_add_bond`. |
| `request_withdrawal` | :308 | `(bond_count, dest: Option<String>, state)` → `TxResponse` | Initiates bond withdrawal (7-day delay, enforced node-side). |
| `simulate_withdrawal` | :364 | `(bond_count, state)` → `SimulateResponse` | Preview penalty amounts (RPC-computed, no TX). |
| `exit_producer` | :394 | `(_force: bool, state)` → `TxResponse` | Submits `ProducerExit` TX. `_force` param unused. |

**Rewards** — `bins/gui/src/commands/rewards.rs`
| Command | Line | Signature (args) | Description |
|---------|------|-------------------|--------------|
| `list_rewards` | :12 | `(state)` → `Vec<RewardEpochResponse>` | All epochs with reward status (qualified/claimed). |
| `claim_reward` | :42 | `(epoch: u64, recipient: Option<String>, state)` → `TxResponse` | Claims reward for one epoch. |
| `claim_all_rewards` | :97 | `(state)` → `Vec<TxResponse>` | Iterates unclaimed qualified epochs, one TX each. |

**Network** — `bins/gui/src/commands/network.rs`
| Command | Line | Signature (args) | Description |
|---------|------|-------------------|--------------|
| `get_chain_info` | :13 | `(state)` → `ChainInfoResponse` | network, best_hash, best_height, best_slot, genesis_hash. |
| `set_rpc_endpoint` | :30 | `(url: String, state)` → `bool` | Tests then switches RPC endpoint; persists to config if reachable. |
| `set_network` | :51 | `(network: String, state)` → `()` | Switches mainnet/testnet/devnet: restarts embedded node, updates RPC client. |
| `test_connection` | :88 | `(url: String)` → `ConnectionTestResult` | No-state probe against any URL. |
| `get_connection_status` | :108 | `(state)` → `ConnectionStatus` | connected/disconnected + height + endpoint. |

**Node** — `bins/gui/src/commands/node.rs`
| Command | Line | Signature (args) | Description |
|---------|------|-------------------|--------------|
| `start_node` | :23 | `(state)` → `()` | Starts embedded `doli-node` process. |
| `stop_node` | :30 | `(state)` → `()` | Stops embedded node (graceful). |
| `node_status` | :37 | `(state)` → `NodeStatus` | running, network, rpc_url, log_path. |
| `restart_node` | :52 | `(network: Option<String>, state)` → `()` | Restarts node (optionally switching network); updates RPC client. |
| `get_node_logs` | :73 | `(lines: Option<usize>, state)` → `Vec<String>` | Last N lines from node.log (default 100). |

**NFT/Token** — `bins/gui/src/commands/nft.rs`
| Command | Line | Signature (args) | Description |
|---------|------|-------------------|--------------|
| `mint_nft` | :12 | `(content, value: Option<String>, state)` → `TxResponse` | Mints NFT via `NftMint` TX, content as `extra_data` (output_type=4). |
| `transfer_nft` | :65 | `(_utxo_ref, _to, state)` → `TxResponse` | NOT IMPLEMENTED — always `Err`. |
| `nft_info` | :75 | `(_utxo_ref, state)` → `NftInfoResponse` | NOT IMPLEMENTED — always `Err`. |
| `issue_token` | :84 | `(ticker, supply, state)` → `TxResponse` | Issues token via `TokenIssuance` TX; ticker+supply encoded in `extra_data`. |
| `token_info` | :137 | `(_utxo_ref, state)` → `TokenInfoResponse` | NOT IMPLEMENTED — always `Err`. |

**Bridge** — `bins/gui/src/commands/bridge.rs`
| Command | Line | Signature (args) | Description |
|---------|------|-------------------|--------------|
| `bridge_lock` | :12 | `(params: BridgeLockParams, state)` → `TxResponse` | Locks funds in HTLC: hash_lock + timeout_height in extra_data. |
| `bridge_claim` | :79 | `(_utxo_ref, _preimage, state)` → `TxResponse` | NOT IMPLEMENTED — always `Err`. |
| `bridge_refund` | :89 | `(_utxo_ref, state)` → `TxResponse` | NOT IMPLEMENTED — always `Err`. |

**Governance** — `bins/gui/src/commands/governance.rs`
| Command | Line | Signature (args) | Description |
|---------|------|-------------------|--------------|
| `check_updates` | :13 | `(state)` → `Vec<UpdateInfo>` | Returns empty list (placeholder — no governance RPC yet). |
| `update_status` | :21 | `(state)` → `UpdateStatusResponse` | Returns `env!("CARGO_PKG_VERSION")`, `update_available=false` (placeholder). |
| `vote_update` | :31 | `(_version, _approve, state)` → `TxResponse` | NOT IMPLEMENTED — always `Err`. |
| `sign_message` | :41 | `(message, address: Option<String>, state)` → `String` | Signs arbitrary message with wallet key. |
| `verify_signature` | :55 | `(message, signature, pubkey)` → `bool` | Stateless — verifies signature, no wallet/state required. |

## OPERATIONS

| User goal | Steps | Commands/Functions | Inputs | Success |
|-----------|-------|---------------------|--------|---------|
| Launch app (auto-start local node) | 1. Run `doli-gui` binary 2. `AppState::new()` loads config 3. `NodeManager::start()` best-effort spawns `doli-node` sibling binary | `main()` main.rs:21 | none (uses `~/.doli-gui/config.json` defaults) | Frontend loads; if node binary missing, app still opens (connect to remote node via Settings). |
| Create wallet | 1. `create_wallet(name, wallet_path)` 2. show `seed_phrase` to user ONCE 3. user backs it up | `commands::wallet::create_wallet` wallet.rs:39 | name, wallet_path (validated, no `..`/null bytes) | `CreateWalletResponse` with primary_address + bech32_address; seed phrase never persisted or re-shown. |
| Restore wallet from seed phrase | 1. `restore_wallet(name, seed_phrase, wallet_path)` | `commands::wallet::restore_wallet` wallet.rs:74 | valid BIP-39 mnemonic | `WalletInfo` returned; wallet saved + loaded into state. |
| Check balance | 1. `get_balance(address?)` — defaults to primary wallet address if omitted | `commands::transaction::get_balance` transaction.rs:14 | wallet loaded (if address omitted) | `BalanceResponse` (confirmed/unconfirmed/immature/total, human-formatted). |
| Send DOLI | 1. `send_doli(to, amount, fee?)` 2. fetch spendable UTXOs 3. build+sign TX in Rust 4. submit via RPC | `commands::transaction::send_doli` transaction.rs:53 | wallet loaded, recipient (hex or bech32m), amount as decimal string | `SendResponse` with tx_hash; funds move once block confirms. |
| View transaction history | 1. `get_history(limit?)` | `commands::transaction::get_history` transaction.rs:127 | wallet loaded | `Vec<HistoryEntryResponse>`, newest-first, default 50 entries. |
| Register as producer | 1. `add_bls_key()` if wallet has none 2. `register_producer(bond_count)` — builds hash-chain VDF proof, BLS PoP, bond outputs | `commands::wallet::add_bls_key` wallet.rs:253, `commands::producer::register_producer` producer.rs:53 | 1 <= bond_count <= 10,000; sufficient balance; BLS key present | `TxResponse`; producer activates at next epoch boundary (deferred, per consensus rules). |
| Add bonds to existing producer | 1. `add_bonds(count)` | `commands::producer::add_bonds` producer.rs:256 | already-registered producer, sufficient balance | `TxResponse` for `AddBond` TX. |
| Preview + request bond withdrawal | 1. `simulate_withdrawal(bond_count)` to preview penalty 2. `request_withdrawal(bond_count, dest?)` to submit | `commands::producer::simulate_withdrawal` producer.rs:364, `commands::producer::request_withdrawal` producer.rs:308 | registered producer with bond_count bonds | `SimulateResponse` (preview, no TX) then `TxResponse` (submits); 7-day delay before claim. |
| Exit as producer | 1. `exit_producer(_force)` | `commands::producer::exit_producer` producer.rs:394 | registered producer | `TxResponse` for `ProducerExit` TX. |
| Claim epoch rewards | 1. `list_rewards()` to see qualified/unclaimed epochs 2. `claim_reward(epoch, recipient?)` for one, or `claim_all_rewards()` for all | `commands::rewards::list_rewards` rewards.rs:12, `commands::rewards::claim_reward` rewards.rs:42, `commands::rewards::claim_all_rewards` rewards.rs:97 | registered producer with qualified epochs | `TxResponse` (or `Vec<TxResponse>`) per claimed epoch. |
| Switch network (mainnet/testnet/devnet) | 1. `set_network(network)` — restarts embedded node on new network, updates RPC client | `commands::network::set_network` network.rs:51 | network in {mainnet, testnet, devnet} | Node restarted, RPC client repointed, config persisted. |
| Point GUI at a remote/custom RPC endpoint | 1. `test_connection(url)` to verify reachability 2. `set_rpc_endpoint(url)` to switch | `commands::network::test_connection` network.rs:88, `commands::network::set_rpc_endpoint` network.rs:30 | reachable RPC URL | `bool` success; config persists `custom_rpc_url` only if reachable. |
| Start/stop/restart the embedded node manually | 1. `start_node()` / `stop_node()` / `restart_node(network?)` | `commands::node::start_node` node.rs:23, `commands::node::stop_node` node.rs:30, `commands::node::restart_node` node.rs:52 | `doli-node` binary present (sibling dir or PATH) | Node process spawned/terminated; `node_status()` reflects new state. |
| View embedded node logs | 1. `get_node_logs(lines?)` | `commands::node::get_node_logs` node.rs:73 | node has been started at least once (log file exists) | `Vec<String>` of last N lines (default 100). |
| Mint NFT | 1. `mint_nft(content, value?)` | `commands::nft::mint_nft` nft.rs:12 | wallet loaded | `TxResponse` for `NftMint` TX. |
| Issue token | 1. `issue_token(ticker, supply)` | `commands::nft::issue_token` nft.rs:84 | wallet loaded | `TxResponse` for `TokenIssuance` TX. |
| Lock funds in bridge HTLC | 1. `bridge_lock(params)` — recipient, amount, hash_lock, timeout_height | `commands::bridge::bridge_lock` bridge.rs:12 | wallet loaded, valid hex hash_lock | `TxResponse` for `BridgeLock` TX. |
| Sign / verify an arbitrary message | 1. `sign_message(message, address?)` 2. (elsewhere) `verify_signature(message, signature, pubkey)` | `commands::governance::sign_message` governance.rs:41, `commands::governance::verify_signature` governance.rs:55 | wallet loaded for signing; verify is stateless | signature string; `bool` for verify. |

## DATA FLOW

### Send DOLI (transaction.rs:53)
```
Frontend → send_doli(to, amount, fee)
  → wallet.primary_pubkey_hash() [no IPC crossing]
  → rpc.get_utxos(pubkey_hash, spendable=true)
  → TxBuilder::build_transfer(utxos, recipient, amount, fee, sender)
  → wallet.primary_keypair() [stays in Rust]
  → builder.sign_and_build(keypair) [signs in Rust, keypair zeroized on drop]
  → rpc.send_transaction(tx_hex)
  → tx_hash → Frontend
```

### Register Producer (producer.rs:53)
```
Frontend → register_producer(bond_count)
  → wallet BLS private key check (must exist)
  → rpc.get_producers() → duplicate check
  → rpc.get_chain_info() + rpc.get_network_params()
  → rpc.get_utxos() → UTXO selection + fee calc
  → vdf::registration_input(pubkey, epoch) → hash_chain_vdf(input, T_REGISTER_BASE)
  → bls_sign_pop(bls_sk, bls_pk) → BLS proof-of-possession
  → bincode::serialize(RegistrationData) → extra_data
  → build bond outputs (Output::bond per bond_count)
  → sign each input with keypair
  → rpc.send_transaction(tx_hex) → TxResponse
```

### Network Switch (network.rs:51)
```
Frontend → set_network("testnet")
  → validate: must be mainnet/testnet/devnet
  → node_manager.restart("testnet") [stop + start with new network]
  → config.network = new, config.custom_rpc_url = None → config.save()
  → rpc_client = RpcClient::new(node_manager.rpc_url())
```

### Config + State Load (state.rs:35)
```
AppState::new()
  → AppConfig::load_or_default() [~/.doli-gui/config.json]
  → NodeManager::new(default_data_dir(), network) [~/.doli/ or %APPDATA%/doli/]
  → RpcClient::new(custom_rpc_url OR node_manager.rpc_url())
```

### Log Tail (node_manager.rs:189)
```
get_node_logs(lines) → node_manager.tail_log(n)
  → BufReader on ~/.doli/node.log
  → collect all lines → return last N
  (tail_log_bytes exists but unused by any command -- #[allow(dead_code)], node_manager.rs:202)
```

## DEPENDENCIES

### This Domain Uses
| This Domain Uses | Skill File | What For |
|-------------------|-----------|----------|
| `wallet::Wallet`, `wallet::RpcClient`, `wallet::TxBuilder` | (no dedicated wallet-crate skill found; see `crates/wallet/src/`) | Wallet lifecycle, RPC calls, all TX-building helpers (`build_transfer`, `build_add_bond`, `build_request_withdrawal`, `build_reward_claim`). GUI depends on this crate, NOT `bins/node` directly. |
| `wallet::format_balance`, `coins_to_units`, `units_to_coins`, `default_endpoints`, `network_prefix`, `BOND_UNIT` | same | Amount formatting/parsing, network prefix + default RPC endpoint lookup. |
| `crypto::address::{from_pubkey, resolve, encode}`, `crypto::Hash`, `crypto::PublicKey`, `crypto::BlsSecretKey`, `crypto::bls_sign_pop`, `crypto::signature::sign_hash` | `.claude/skills/` (crypto crate, no dedicated skill found) | Address encode/decode, BLS key ops for producer registration, raw TX input signing. |
| `doli_core::{Transaction, Input, Output, transaction::TxType, transaction::RegistrationData}`, `doli_core::consensus::{BASE_FEE, FEE_PER_BYTE, FEE_DIVISOR}`, `doli_core::tpop::heartbeat::hash_chain_vdf` | `CLAUDE.md` code map → `crates/core/` | Low-level TX construction for `register_producer` (bypasses `wallet::TxBuilder` high-level helper because Registration needs custom VDF/BLS extra_data). |
| `vdf::{registration_input, T_REGISTER_BASE}` | `crates/vdf` (no GUI-relevant skill found) | Hash-chain VDF input/const for registration proof. **Note: MEMORY.md says "DOLI does NOT use VDF [for block production]" — this crate use is registration-proof-of-work only, unrelated to VDF-based slot production.** |
| `doli-node` (spawned as child process, NOT linked) | `CLAUDE.md` code map → `bins/node/` | Embedded local node for wallet-connected-to-self usage. GUI never calls into node Rust code — only spawns the binary and talks RPC. |
| `tauri` 2.x, `tauri-plugin-dialog`, `tauri-plugin-clipboard-manager` | external crate | IPC framework, file dialogs, clipboard. |

### Used By
| Used By | Skill File | What For |
|---------|-----------|----------|
| Svelte frontend (not `.rs`, outside this domain) | none in this domain | Calls every `#[tauri::command]` via Tauri's `invoke()` IPC bridge. |
| _(no other Rust crate/binary depends on `bins/gui` — it is a terminal binary target, not a library)_ | — | — |

### Config / Data File Locations
- Config: `~/.doli-gui/config.json`
- Default wallet dir: `~/.doli-gui/wallets/`
- Node data (embedded, spawned child): `~/.doli/` (Unix) or `%APPDATA%/doli/` (Windows)
- Node log: `~/.doli/node.log`

## CONSTRAINTS

| Constraint | Type | Location | Detail |
|-----------|------|----------|--------|
| Private keys never cross IPC boundary (GUI-NF-004) | security | `bins/gui/src/commands/wallet.rs:281` (`build_wallet_info_with_prefix`) | All signing happens in Rust handlers. `WalletInfo`/`AddressInfo` structs contain no private fields. Enforced by tests: `test_build_wallet_info_no_private_keys`, `test_address_info_no_private_keys` (wallet.rs:297-329). |
| Path traversal / null-byte rejection | security | `bins/gui/src/commands/wallet.rs:19` (`validate_path`) | Rejects paths containing `..` or `\0`, or empty paths. Applied to `create_wallet`, `restore_wallet`, `load_wallet`, `export_wallet`, `import_wallet`. |
| `set_network` only accepts 3 values | invariant | `bins/gui/src/commands/network.rs:52` | Must be exactly `"mainnet"`, `"testnet"`, or `"devnet"` — else `Err`. |
| Bond count bounds | invariant | `bins/gui/src/commands/producer.rs:57` | `register_producer` requires `1 <= bond_count <= 10_000`. No equivalent explicit check in `add_bonds` (producer.rs:256) — relies on node-side/TxBuilder validation. |
| RPC port per network | invariant | `bins/gui/src/node_manager.rs:241` (`rpc_port_for_network`) | mainnet=8500, testnet=18500, devnet=28500, unknown→8500 (mainnet fallback). MUST match `crates/core/src/network_params.rs` defaults (comment at node_manager.rs:12). |
| Node binary discovery order | edge-case | `bins/gui/src/node_manager.rs:255` (`find_node_binary`) | 1. sibling of current exe 2. system PATH. If neither has `doli-node`(`.exe`), `start()` returns `Err` but does not crash the GUI (main.rs:29-32 catches and logs). |
| Graceful shutdown timeout | performance | `bins/gui/src/node_manager.rs:25` (`SHUTDOWN_TIMEOUT`) | SIGTERM (via `/bin/kill` on Unix, `Child::kill()` on Windows) → wait up to 10s, polling every 100ms → SIGKILL if still alive. |
| `NodeManager::drop` swallows errors | edge-case | `bins/gui/src/node_manager.rs:232` | `Drop` impl calls `graceful_shutdown` and discards the `Result` — process-exit path cannot surface shutdown failures to the user. |
| Fixed flat fee in `send_doli` when unset | edge-case | `bins/gui/src/commands/transaction.rs:63` | `fee: None` defaults to `1` (base unit), NOT a fee-rate calculation — likely too low for real network conditions; frontend should always supply a fee. |
| `blocks_per_era` hardcoded per network in registration | edge-case | `bins/gui/src/commands/producer.rs:165-168` | devnet=576, all others (incl. testnet/mainnet)=12,614,400. Drifts silently if `crates/core` consensus constants change — no shared const import for this specific value. |
| Registration `prev_registration_hash` always `Hash::ZERO` | edge-case | `bins/gui/src/commands/producer.rs:189` | GUI does not track registration chaining across re-registrations — always sends zero hash and `sequence_number: 0`. |
| Several commands are stubs that always `Err` | invariant | `nft.rs:65,75,142`; `bridge.rs:79,89`; `governance.rs:31` | `transfer_nft`, `nft_info`, `token_info`, `bridge_claim`, `bridge_refund`, `vote_update` — not yet implemented. Frontend must handle these as permanently unavailable, not transient errors. |

## PATTERNS

### RwLock guard released before re-acquiring
Every command acquires `state.rpc_client.read().await` / `state.wallet.read().await` fresh per call and drops the guard before the next `.await` on a *different* lock, avoiding deadlocks:
```rust
let result = {
    let rpc = state.rpc_client.read().await;
    rpc.some_method().await.map_err(|e| e.to_string())?
};
```
Example: `send_doli` (transaction.rs:53) reads wallet pubkey_hash → drops guard → fetches UTXOs via RPC → re-acquires wallet guard for signing.

### TxBuilder pattern (low-level)
```rust
let mut builder = wallet::TxBuilder::new(wallet::TxType::SomeType);
builder.add_input(sender_hash, 0);
builder.add_output(amount, recipient_hash, output_type, lock_until, extra_data);
builder.set_extra_data(extra);
let tx_hex = builder.sign_and_build(&keypair)?;
```
Used directly by `exit_producer` (producer.rs:406), `mint_nft` (nft.rs:33), `issue_token` (nft.rs:107), `bridge_lock` (bridge.rs:40) — where no high-level helper exists yet.

High-level helpers used instead where available: `TxBuilder::build_transfer` (transaction.rs:90), `TxBuilder::build_add_bond` (producer.rs:276), `TxBuilder::build_request_withdrawal` (producer.rs:335), `TxBuilder::build_reward_claim` (rewards.rs:69, rewards.rs:125).

### Config mutation always persists (best-effort)
```rust
let mut config = state.config.write().await;
config.some_field = new_value;
let _ = config.save();
```
Save errors are discarded (`let _ =`) — config persistence is fire-and-forget. Seen in `create_wallet` (wallet.rs:59-61), `set_network` (network.rs:71-74), `set_rpc_endpoint` (network.rs:36-39).

### Unimplemented stubs return `Err`, never panic
`transfer_nft`, `nft_info`, `token_info`, `bridge_claim`, `bridge_refund`, `vote_update` return `Err("... not yet implemented")`. `check_updates`/`update_status` (governance.rs:13,21) return placeholder empty/static data instead of erroring — inconsistent stub convention, worth flagging to frontend devs.

### Amount format conversions
- Frontend sends amounts as human-readable decimal strings: `"1.5"` DOLI
- `wallet::coins_to_units(&str) -> Result<u64, String>` — parse to base units
- `wallet::format_balance(u64) -> String` — human-readable display string
- `wallet::units_to_coins(u64) -> String` — DOLI decimal string

### Address resolution (dual format)
`crypto::address::resolve(address, None)` (transaction.rs:188) accepts both:
- Hex pubkey hash (64 hex chars = 32 bytes)
- Bech32m address (prefix `doli`)
Returns `crypto::Hash` → cast to `[u8; 32]`. Used wherever a recipient/destination address is accepted from the frontend.

## STRUCTS

All response structs defined in `bins/gui/src/commands/mod.rs` with `#[serde(rename_all = "camelCase")]`.

| Struct | Fields | Line |
|--------|--------|------|
| `CreateWalletResponse` | name, seed_phrase, primary_address, bech32_address | mod.rs:26 |
| `WalletInfo` | name, version, address_count, primary_address, primary_public_key, bech32_address, has_bls_key | mod.rs:36 |
| `AddressInfo` | address, public_key, label, bech32_address, has_bls_key | mod.rs:49 |
| `BalanceResponse` | confirmed, unconfirmed, immature, total, formatted_total, formatted_confirmed | mod.rs:60 |
| `SendResponse` | tx_hash, amount, fee, formatted_amount | mod.rs:72 |
| `TxResponse` | tx_hash, tx_type, message | mod.rs:82 |
| `HistoryEntryResponse` | hash, tx_type, height, timestamp, amount_received, amount_sent, fee, confirmations, formatted_received, formatted_sent, net_amount | mod.rs:91 |
| `ProducerStatusResponse` | is_registered, status, bond_count, bond_amount, formatted_bond_amount, registration_height, era | mod.rs:108 |
| `SimulateResponse` | bond_count, total_staked, total_penalty, net_amount, formatted_total_staked, formatted_penalty, formatted_net | mod.rs:121 |
| `RewardEpochResponse` | epoch, estimated_reward, formatted_reward, qualified, claimed | mod.rs:134 |
| `NftInfoResponse` | utxo_ref, content, value, formatted_value | mod.rs:145 |
| `TokenInfoResponse` | utxo_ref, ticker, supply | mod.rs:155 |
| `BridgeLockParams` | recipient, amount, hash_lock, timeout_height | mod.rs:164 |
| `UpdateInfo` | version, description, votes_for, votes_against, status | mod.rs:174 |
| `UpdateStatusResponse` | current_version, latest_version, update_available | mod.rs:185 |
| `ChainInfoResponse` | network, best_hash, best_height, best_slot, genesis_hash | mod.rs:194 |
| `ConnectionStatus` | connected, endpoint, network, chain_height, status | mod.rs:205 |
| `ConnectionTestResult` | success, network, height, error | mod.rs:216 |

| State Struct | Fields | Location |
|---------------|--------|----------|
| `AppState` | wallet: RwLock\<Option\<Wallet\>\>, wallet_path: RwLock\<Option\<PathBuf\>\>, rpc_client: RwLock\<RpcClient\>, config: RwLock\<AppConfig\>, node_manager: RwLock\<NodeManager\> | `state.rs:17` |
| `AppConfig` | network, custom_rpc_url, default_wallet_path, last_wallet_path, poll_interval, rpc_endpoints | `state.rs:76` |
| `NodeManager` | process: Option\<Child\>, data_dir, network, rpc_port, log_path | `node_manager.rs:28` |
| `NodeStatus` | running, network, rpc_url, log_path | `node.rs:14` |
