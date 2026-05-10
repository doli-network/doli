# gui — DOLI Desktop GUI (Tauri 2.x)
<!-- @INDEX
ENTRY-POINTS: lines 15-26
COMMANDS: lines 28-114
STRUCTS: lines 116-192
DATA-FLOWS: lines 194-226
DEPENDENCIES: lines 228-244
CONSTRAINTS: lines 246-262
PATTERNS: lines 264-295
-->

## ENTRY-POINTS

Binary: `bins/gui/src/main.rs` — `doli-gui` Tauri 2.x application.

`main()` (main.rs:21):
1. Creates `AppState::new()` — loads persisted config, creates `NodeManager`, initializes `RpcClient`
2. Auto-starts embedded `doli-node` via `NodeManager::start()` (best-effort; silently continues if binary absent)
3. Registers all Tauri command handlers via `tauri::generate_handler![]`
4. Registers plugins: `tauri_plugin_dialog`, `tauri_plugin_clipboard_manager`
5. Runs the event loop via `tauri::Builder::default().run()`

On exit: `NodeManager::drop()` gracefully shuts down the embedded node process (SIGTERM → 10s wait → SIGKILL).

## COMMANDS

All commands are async, return `Result<T, String>`, and receive `State<'_, AppState>` via Tauri injection.

### Wallet — `bins/gui/src/commands/wallet.rs`
| Command | Signature | Line | Description |
|---------|-----------|------|-------------|
| `create_wallet` | `(name: String, wallet_path: String, state)` → `CreateWalletResponse` | :38 | Creates wallet, returns seed phrase once (never stored). Updates state + persists config. |
| `restore_wallet` | `(name, seed_phrase, wallet_path, state)` → `WalletInfo` | :73 | Restores from BIP-39 mnemonic, saves to disk. |
| `load_wallet` | `(wallet_path, state)` → `WalletInfo` | :100 | Loads wallet file from disk, updates state. |
| `generate_address` | `(label: Option<String>, state)` → `AddressInfo` | :122 | Derives new address, saves wallet. |
| `list_addresses` | `(state)` → `Vec<AddressInfo>` | :167 | Lists all wallet addresses (skips corrupted entries). |
| `export_wallet` | `(destination, state)` → `()` | :205 | Exports wallet to a file path. |
| `import_wallet` | `(source, destination, state)` → `WalletInfo` | :217 | Imports wallet from file, saves to new path. |
| `wallet_info` | `(state)` → `WalletInfo` | :243 | Returns public wallet metadata (no keys). |
| `add_bls_key` | `(state)` → `String` | :252 | Generates BLS key for producer registration, saves wallet, returns BLS public key hex. |

### Transaction — `bins/gui/src/commands/transaction.rs`
| Command | Signature | Line | Description |
|---------|-----------|------|-------------|
| `get_balance` | `(address: Option<String>, state)` → `BalanceResponse` | :13 | Gets balance for address or primary wallet address via RPC. |
| `send_doli` | `(to, amount, fee: Option<String>, state)` → `SendResponse` | :52 | Full signing flow in Rust: fetch UTXOs → build TX → sign → submit. |
| `get_history` | `(limit: Option<u32>, state)` → `Vec<HistoryEntryResponse>` | :126 | Transaction history for primary address (default limit: 50). |

### Producer — `bins/gui/src/commands/producer.rs`
| Command | Signature | Line | Description |
|---------|-----------|------|-------------|
| `producer_status` | `(state)` → `ProducerStatusResponse` | :13 | Checks registration status by querying producers list via RPC. |
| `register_producer` | `(bond_count: u32, state)` → `TxResponse` | :52 | Builds full `Registration` TX: hash-chain VDF proof, BLS PoP, bond outputs. Requires BLS key in wallet. |
| `add_bonds` | `(count: u32, state)` → `TxResponse` | :255 | Adds bond UTXOs via `TxBuilder::build_add_bond`. |
| `request_withdrawal` | `(bond_count, dest: Option<String>, state)` → `TxResponse` | :307 | Initiates bond withdrawal (7-day delay). |
| `simulate_withdrawal` | `(bond_count, state)` → `SimulateResponse` | :363 | Preview penalty amounts without submitting TX. |
| `exit_producer` | `(_force: bool, state)` → `TxResponse` | :393 | Submits `ProducerExit` transaction. |

### Rewards — `bins/gui/src/commands/rewards.rs`
| Command | Signature | Line | Description |
|---------|-----------|------|-------------|
| `list_rewards` | `(state)` → `Vec<RewardEpochResponse>` | :11 | Lists all epochs with reward status (qualified/claimed). |
| `claim_reward` | `(epoch: u64, recipient: Option<String>, state)` → `TxResponse` | :41 | Claims reward for one specific epoch. |
| `claim_all_rewards` | `(state)` → `Vec<TxResponse>` | :96 | Iterates all unclaimed qualified epochs, submits separate TX for each. |

### Network — `bins/gui/src/commands/network.rs`
| Command | Signature | Line | Description |
|---------|-----------|------|-------------|
| `get_chain_info` | `(state)` → `ChainInfoResponse` | :12 | Returns network, best_hash, best_height, best_slot, genesis_hash. |
| `set_rpc_endpoint` | `(url: String, state)` → `bool` | :29 | Tests then switches RPC endpoint; persists to config if reachable. |
| `set_network` | `(network: String, state)` → `()` | :50 | Switches to mainnet/testnet/devnet. Restarts embedded node, updates RPC client. |
| `test_connection` | `(url: String)` → `ConnectionTestResult` | :87 | No-state probe: attempts `get_chain_info` against any URL. |
| `get_connection_status` | `(state)` → `ConnectionStatus` | :107 | Returns connected/disconnected + height + endpoint. |

### Node — `bins/gui/src/commands/node.rs`
| Command | Signature | Line | Description |
|---------|-----------|------|-------------|
| `start_node` | `(state)` → `()` | :22 | Starts embedded `doli-node` process. |
| `stop_node` | `(state)` → `()` | :29 | Stops embedded node (graceful). |
| `node_status` | `(state)` → `NodeStatus` | :36 | Returns running status, network, rpc_url, log_path. |
| `restart_node` | `(network: Option<String>, state)` → `()` | :51 | Restarts node (optionally switching network). Updates RPC client. |
| `get_node_logs` | `(lines: Option<usize>, state)` → `Vec<String>` | :72 | Returns last N lines from node.log (default: 100). |

### NFT/Token — `bins/gui/src/commands/nft.rs`
| Command | Signature | Line | Description |
|---------|-----------|------|-------------|
| `mint_nft` | `(content, value: Option<String>, state)` → `TxResponse` | :11 | Mints NFT via `NftMint` TX type with content as `extra_data`. |
| `transfer_nft` | `(_utxo_ref, _to, state)` → `TxResponse` | :64 | NOT IMPLEMENTED — returns `Err`. |
| `nft_info` | `(_utxo_ref, state)` → `NftInfoResponse` | :74 | NOT IMPLEMENTED — returns `Err`. |
| `issue_token` | `(ticker, supply, state)` → `TxResponse` | :83 | Issues token via `TokenIssuance` TX; encodes ticker+supply in `extra_data`. |
| `token_info` | `(_utxo_ref, state)` → `TokenInfoResponse` | :136 | NOT IMPLEMENTED — returns `Err`. |

### Bridge — `bins/gui/src/commands/bridge.rs`
| Command | Signature | Line | Description |
|---------|-----------|------|-------------|
| `bridge_lock` | `(params: BridgeLockParams, state)` → `TxResponse` | :11 | Locks funds in HTLC: encodes hash_lock + timeout_height in extra_data. |
| `bridge_claim` | `(_utxo_ref, _preimage, state)` → `TxResponse` | :79 | NOT IMPLEMENTED — returns `Err`. |
| `bridge_refund` | `(_utxo_ref, state)` → `TxResponse` | :88 | NOT IMPLEMENTED — returns `Err`. |

### Governance — `bins/gui/src/commands/governance.rs`
| Command | Signature | Line | Description |
|---------|-----------|------|-------------|
| `check_updates` | `(state)` → `Vec<UpdateInfo>` | :12 | Returns empty list (placeholder). |
| `update_status` | `(state)` → `UpdateStatusResponse` | :20 | Returns current binary version, update_available=false (placeholder). |
| `vote_update` | `(_version, _approve, state)` → `TxResponse` | :30 | NOT IMPLEMENTED — returns `Err`. |
| `sign_message` | `(message, address: Option<String>, state)` → `String` | :40 | Signs arbitrary message with wallet key; returns signature string. |
| `verify_signature` | `(message, signature, pubkey)` → `bool` | :54 | Stateless: verifies message signature. No wallet required. |

## STRUCTS

All structs defined in `bins/gui/src/commands/mod.rs` with `#[serde(rename_all = "camelCase")]`.

### Response Types (mod.rs)
| Struct | Fields | Line |
|--------|--------|------|
| `CreateWalletResponse` | name, seed_phrase, primary_address, bech32_address | :24 |
| `WalletInfo` | name, version, address_count, primary_address, primary_public_key, bech32_address, has_bls_key | :33 |
| `AddressInfo` | address, public_key, label, bech32_address, has_bls_key | :47 |
| `BalanceResponse` | confirmed, unconfirmed, immature, total, formatted_total, formatted_confirmed | :57 |
| `SendResponse` | tx_hash, amount, fee, formatted_amount | :69 |
| `TxResponse` | tx_hash, tx_type, message | :79 |
| `HistoryEntryResponse` | hash, tx_type, height, timestamp, amount_received, amount_sent, fee, confirmations, formatted_received, formatted_sent, net_amount | :88 |
| `ProducerStatusResponse` | is_registered, status, bond_count, bond_amount, formatted_bond_amount, registration_height, era | :105 |
| `SimulateResponse` | bond_count, total_staked, total_penalty, net_amount, formatted_total_staked, formatted_penalty, formatted_net | :119 |
| `RewardEpochResponse` | epoch, estimated_reward, formatted_reward, qualified, claimed | :131 |
| `NftInfoResponse` | utxo_ref, content, value, formatted_value | :142 |
| `TokenInfoResponse` | utxo_ref, ticker, supply | :153 |
| `BridgeLockParams` | recipient, amount, hash_lock, timeout_height | :161 |
| `UpdateInfo` | version, description, votes_for, votes_against, status | :171 |
| `UpdateStatusResponse` | current_version, latest_version, update_available | :183 |
| `ChainInfoResponse` | network, best_hash, best_height, best_slot, genesis_hash | :192 |
| `ConnectionStatus` | connected, endpoint, network, chain_height, status | :202 |
| `ConnectionTestResult` | success, network, height, error | :213 |

### State Types
| Struct | Fields | File |
|--------|--------|------|
| `AppState` | wallet: RwLock<Option<Wallet>>, wallet_path: RwLock<Option<PathBuf>>, rpc_client: RwLock<RpcClient>, config: RwLock<AppConfig>, node_manager: RwLock<NodeManager> | state.rs:17 |
| `AppConfig` | network, custom_rpc_url, default_wallet_path, last_wallet_path, poll_interval, rpc_endpoints | state.rs:75 |
| `NodeManager` | process: Option<Child>, data_dir, network, rpc_port, log_path | node_manager.rs:28 |
| `NodeStatus` | running, network, rpc_url, log_path | node.rs:14 |

## DATA-FLOWS

### Send DOLI (transaction.rs:52)
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

### Register Producer (producer.rs:52)
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

### Network Switch (network.rs:50)
```
Frontend → set_network("testnet")
  → validate: must be mainnet/testnet/devnet
  → node_manager.restart("testnet") [stop + start with new network]
  → config.network = new, config.custom_rpc_url = None → config.save()
  → rpc_client = RpcClient::new(node_manager.rpc_url())
```

### Config Load (state.rs:35)
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
  (tail_log_bytes also exists for large files: seeks to file_len - max_bytes)
```

## DEPENDENCIES

### Internal Crates
| Crate | Usage |
|-------|-------|
| `wallet` | `Wallet`, `RpcClient`, `TxBuilder`, `format_balance`, `coins_to_units`, `units_to_coins`, `default_endpoints`, `network_prefix`, `BOND_UNIT` |
| `crypto` | `address::from_pubkey`, `address::resolve`, `address::encode`, `Hash`, `PublicKey`, `BlsSecretKey`, `bls_sign_pop`, `signature::sign_hash` |
| `doli-core` | `Transaction`, `Input`, `Output`, `transaction::TxType`, `transaction::RegistrationData`, `consensus::BASE_FEE`, `consensus::FEE_PER_BYTE`, `consensus::FEE_DIVISOR`, `tpop::heartbeat::hash_chain_vdf` |
| `vdf` | `registration_input`, `T_REGISTER_BASE` |
| `bincode` | Serialization of `RegistrationData` into `extra_data` bytes |

### External Crates
| Crate | Usage |
|-------|-------|
| `tauri` 2.x | Framework, `State<>`, `#[tauri::command]`, `generate_handler!`, plugins |
| `tauri-plugin-dialog` | File open/save dialogs |
| `tauri-plugin-clipboard-manager` | Clipboard access |
| `tokio` | Async runtime (RwLock, block_on at startup) |
| `serde` / `serde_json` | JSON serialization of all response types |
| `hex` | Encode/decode hex strings for hashes/keys |
| `dirs` | Platform-specific paths (`home_dir`, `data_dir`) |

### Config File Location
- Config: `~/.doli-gui/config.json`
- Default wallet dir: `~/.doli-gui/wallets/`
- Node data: `~/.doli/` (Unix) or `%APPDATA%/doli/` (Windows)
- Node log: `~/.doli/node.log`

## CONSTRAINTS

### Security Invariant: GUI-NF-004
Private keys NEVER cross the IPC boundary. All signing happens in Rust. Frontend only receives public data (addresses, public keys, tx hashes, balances).

Enforced by:
- `WalletInfo` / `AddressInfo` contain no private fields
- `build_wallet_info_with_prefix()` explicitly excludes private data
- `create_wallet` test: `assert!(!json.contains("private"))`
- `wallet.primary_keypair()` called in Rust handler, not exposed to frontend

### Path Sanitization
`validate_path()` (wallet.rs:19) rejects:
- Paths containing `..` (traversal prevention)
- Paths with null bytes (C syscall protection)
- Empty paths

Applied to: `create_wallet`, `restore_wallet`, `load_wallet`, `export_wallet`, `import_wallet`.

### Network Validation
`set_network()` only accepts: `"mainnet"`, `"testnet"`, `"devnet"` (network.rs:52).

### Bond Count
`register_producer()` enforces: `1 <= bond_count <= 10_000` (producer.rs:57).

### RPC Port Mapping (node_manager.rs:13)
- mainnet: 8500
- testnet: 18500
- devnet: 28500
- unknown: falls back to 8500

### Node Binary Discovery (node_manager.rs:255)
Search order:
1. Same directory as current executable (sibling)
2. System PATH

### Shutdown Timeout (node_manager.rs:25)
SIGTERM → wait up to 10 seconds → SIGKILL. Implemented in `graceful_shutdown()`.

## PATTERNS

### All-commands-must-acquire-state-per-RPC
Every command acquires `state.rpc_client.read().await` fresh for each RPC call. Commands do NOT hold `rpc_client` across await boundaries — they drop the guard, then re-acquire for the next call. This avoids deadlocks with `RwLock`.

Pattern:
```rust
let result = {
    let rpc = state.rpc_client.read().await;
    rpc.some_method().await.map_err(|e| e.to_string())?
};
```

### Wallet guard released before re-acquiring
When a command needs both wallet data AND RPC data, it reads from wallet, drops the guard, then does RPC. E.g., `send_doli`: reads pubkey_hash → drops guard → fetches UTXOs → re-acquires wallet for signing.

### TxBuilder pattern
Low-level TX construction:
```rust
let mut builder = wallet::TxBuilder::new(wallet::TxType::SomeType);
builder.add_input(sender_hash, 0);
builder.add_output(amount, recipient_hash, output_type, lock_until, extra_data);
builder.set_extra_data(extra);
let tx_hex = builder.sign_and_build(&keypair)?;
```

High-level helpers exist for common operations: `TxBuilder::build_transfer`, `TxBuilder::build_add_bond`, `TxBuilder::build_request_withdrawal`, `TxBuilder::build_reward_claim`.

### State mutation pattern
Config changes always call `config.save()` (best-effort, error ignored):
```rust
let mut config = state.config.write().await;
config.some_field = new_value;
let _ = config.save();
```

### Unimplemented stubs return Err
Not-yet-implemented commands (`transfer_nft`, `nft_info`, `token_info`, `bridge_claim`, `bridge_refund`, `vote_update`) return `Err("... not yet implemented")` rather than panicking. Governance commands (`check_updates`, `update_status`) return empty/static data.

### Amount format conversions
- Frontend sends amounts as human-readable strings: `"1.5"` DOLI
- `wallet::coins_to_units(&str)` → u64 base units
- `wallet::format_balance(u64)` → human-readable string
- `wallet::units_to_coins(u64)` → DOLI string

### Address resolution
`crypto::address::resolve(address, None)` handles both:
- Hex pubkey hashes (64 hex chars = 32 bytes)
- Bech32m addresses (prefix `doli`)
Returns `crypto::Hash` → cast to `[u8; 32]`.
