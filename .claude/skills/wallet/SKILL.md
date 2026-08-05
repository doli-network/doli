# wallet — DOLI Wallet Library (`crates/wallet`)
<!-- @INDEX
ENTRY-POINTS    11-41
OPERATIONS      42-59
DATA-FLOW       60-118
DEPENDENCIES    119-140
CONSTRAINTS     141-194
PATTERNS        195-259
@/INDEX -->

## ENTRY POINTS

Public API surface (`crates/wallet/src/lib.rs:20-31`): re-exports `Wallet`, `WalletAddress`, `verify_message` (from `wallet.rs`); `TxBuilder`, `TxType` (from `tx_builder/`); `calculate_registration_cost`, `calculate_withdrawal_net`, `vesting_penalty_pct` (from `tx_builder/fees.rs`); `RpcClient`, `default_endpoints`, `network_prefix` (from `rpc_client.rs`); all of `types.rs` via `pub use types::*`.

| Function/Struct | Location | Signature | Description |
|---|---|---|---|
| `Wallet::new` | `wallet.rs:53` | `fn new(name: &str) -> (Self, String)` | Create wallet + 24-word BIP-39 phrase (phrase NOT persisted) |
| `Wallet::from_seed_phrase` | `wallet.rs:88` | `fn from_seed_phrase(name: &str, phrase: &str) -> Result<Self>` | Restore deterministic Ed25519 key; BLS key is fresh random |
| `Wallet::load` | `wallet.rs:118` | `fn load(path: &Path) -> Result<Self>` | Read `wallet.json` |
| `Wallet::save` | `wallet.rs:127` | `fn save(&self, path: &Path) -> Result<()>` | Write `wallet.json`, creates parent dirs |
| `Wallet::export` / `Wallet::import` | `wallet.rs:137,142` | `fn export(&self, path) -> Result<()>` / `fn import(path) -> Result<Self>` | Aliases for save/load |
| `Wallet::primary_pubkey_hash` | `wallet.rs:175` | `fn primary_pubkey_hash(&self) -> Result<String>` | 32-byte BLAKE3(ADDRESS_DOMAIN, pubkey) hex — RPC query key |
| `Wallet::primary_bech32_address` | `wallet.rs:186` | `fn primary_bech32_address(&self, network_prefix: &str) -> Result<String>` | Display address (bech32m via `crypto::address`) |
| `Wallet::primary_keypair` | `wallet.rs:195` | `fn primary_keypair(&self) -> Result<KeyPair>` | Reconstruct Ed25519 keypair for signing |
| `Wallet::generate_address` | `wallet.rs:217` | `fn generate_address(&mut self, label: Option<&str>) -> Result<String>` | New random Ed25519 address, no BLS key |
| `Wallet::add_bls_key` | `wallet.rs:235` | `fn add_bls_key(&mut self) -> Result<String>` | Add BLS keypair to primary address; errors if one exists |
| `Wallet::sign_message` | `wallet.rs:251` | `fn sign_message(&self, message: &str, address: Option<&str>) -> Result<String>` | Sign BLAKE3(message) with Ed25519 |
| `verify_message` | `wallet.rs:273` | `fn verify_message(message: &str, sig_hex: &str, pubkey_hex: &str) -> Result<bool>` | Verify Ed25519 signature |
| `TxBuilder::build_for_signing` | `tx_builder/builder.rs:93` | `fn build_for_signing(&self) -> Result<Vec<u8>>` | Canonical signing-message bytes |
| `TxBuilder::sign_and_build` | `tx_builder/builder.rs:138` | `fn sign_and_build(&mut self, keypair: &KeyPair) -> Result<String>` | Sign + serialize (bincode 1.x hex), ready for `sendTransaction` |
| `TxBuilder::build_transfer` | `tx_builder/builder.rs:245` | `fn build_transfer(utxos: &[Utxo], recipient_hash: [u8;32], amount: u64, fee: u64, sender_hash: [u8;32]) -> Result<Self>` | Greedy UTXO selection, Transfer tx |
| `TxBuilder::build_add_bond` | `tx_builder/builder.rs:304` | `fn build_add_bond(utxos: &[Utxo], bond_count: u32, sender_hash: [u8;32], fee: u64) -> Result<Self>` | AddBond tx: N bond outputs + change |
| `TxBuilder::build_request_withdrawal` | `tx_builder/builder.rs:371` | `fn build_request_withdrawal(bond_count: u32, sender_hash: [u8;32], destination_hash: Option<[u8;32]>) -> Result<Self>` | RequestWithdrawal tx (dummy input — see CONSTRAINTS) |
| `TxBuilder::build_reward_claim` | `tx_builder/builder.rs:399` | `fn build_reward_claim(epoch: u64, sender_hash: [u8;32], recipient_hash: Option<[u8;32]>) -> Result<Self>` | RewardClaim tx (dummy input — see CONSTRAINTS) |
| `calculate_registration_cost` | `tx_builder/fees.rs:53` | `fn calculate_registration_cost(bond_count: u32, pending_registrations: u32) -> Result<(u64,u64,u64)>` | Returns `(bond_cost, reg_fee, total)` |
| `vesting_penalty_pct` | `tx_builder/fees.rs:83` | `fn vesting_penalty_pct(age_slots: u64) -> u8` | Q1(0-1yr)=75%, Q2(1-2yr)=50%, Q3(2-3yr)=25%, vested(3yr+)=0% |
| `calculate_withdrawal_net` | `tx_builder/fees.rs:97` | `fn calculate_withdrawal_net(bond_amount: u64, penalty_pct: u8) -> u64` | `amount - amount*pct/100` |
| `RpcClient::new` | `rpc_client.rs:62` | `fn new(endpoint: &str) -> Self` | Async JSON-RPC client (connect=5s, req=30s timeout) |
| `RpcClient::{get_balance..test_connection}` | `rpc_client.rs:127-248` | `async fn ... -> Result<T>` | 11 wrapped RPC methods — see OPERATIONS |
| `units_to_coins` / `coins_to_units` / `format_balance` | `types.rs:326,337,399` | `fn(u64)->String` / `fn(&str)->Result<u64,String>` / `fn(u64)->String` | Integer-only DOLI <-> base-unit conversion (no f64) |

## OPERATIONS

| Task | Steps | Commands/Functions | Inputs | Success |
|------|-------|--------------------|--------|---------|
| Create a new wallet | 1. Generate BIP-39 phrase + Ed25519/BLS keys 2. Persist to disk | `Wallet::new(name)` → `wallet.save(&path)` | wallet name, target path | `wallet.json` written; 24-word phrase shown to user once, never stored |
| Restore a wallet from seed phrase | 1. Parse+validate BIP-39 phrase 2. Re-derive Ed25519 key 3. Save | `Wallet::from_seed_phrase(name, phrase)` → `wallet.save(&path)` | 24-word phrase | Ed25519 pubkey/address identical to original; BLS key is a NEW random pair (must re-register on-chain if this was a producer) |
| Load an existing wallet | 1. Read JSON file | `Wallet::load(&path)` | path to `wallet.json` | `Wallet` in memory; errors distinguish "not found" vs "failed to parse" |
| Add a secondary address | 1. Generate random Ed25519 keypair 2. Append, save | `wallet.generate_address(label)` → `wallet.save(&path)` | optional label | New entry in `addresses`, no BLS key attached |
| Add a BLS key to primary address | 1. Check none exists 2. Generate BLS keypair | `wallet.add_bls_key()` | none | Returns BLS pubkey hex; errors "already exists" if present |
| Query balance / UTXOs | 1. Compute `pubkey_hash` 2. Call RPC | `wallet.primary_pubkey_hash()` → `rpc.get_balance(addr)` / `rpc.get_utxos(addr, spendable_only)` | wallet, live RPC endpoint | `Balance{confirmed,unconfirmed,immature,total}` or `Vec<Utxo>` |
| Build + send a transfer | 1. Fetch UTXOs 2. Build Transfer tx (greedy select) 3. Sign 4. Submit | `rpc.get_utxos()` → `TxBuilder::build_transfer(utxos, recipient_hash, amount, fee, sender_hash)` → `builder.sign_and_build(&keypair)` → `rpc.send_transaction(tx_hex)` | sender keypair, recipient pubkey_hash, amount, fee, spendable normal UTXOs | tx hash returned; node accepts into mempool |
| Register as producer (add bonds) | 1. Fetch UTXOs 2. Compute cost 3. Build AddBond tx 4. Sign+submit | `calculate_registration_cost(bond_count, pending)` → `TxBuilder::build_add_bond(utxos, bond_count, sender_hash, fee)` → `sign_and_build` → `rpc.send_transaction` | bond_count (≤`MAX_BONDS_PER_PRODUCER`=3000), pending registrations (fee tier), spendable normal UTXOs ≥ `bond_count*BOND_UNIT + fee` | N Bond UTXOs created (`output_type=1`, `lock_until=u64::MAX`) |
| Request bond withdrawal | 1. Get bond details 2. Compute vesting penalty 3. Build RequestWithdrawal tx | `rpc.get_bond_details(pubkey)` → `vesting_penalty_pct(age_slots)` / `calculate_withdrawal_net` → `TxBuilder::build_request_withdrawal(bond_count, sender_hash, destination)` → `sign_and_build` → `rpc.send_transaction` | bond_count, optional destination address | Withdrawal queued; net amount after FIFO+vesting penalty simulated via `rpc.simulate_withdrawal` first |
| Claim epoch reward | 1. List reward-eligible epochs 2. Build RewardClaim tx | `rpc.get_rewards_list(pubkey)` → `TxBuilder::build_reward_claim(epoch, sender_hash, recipient)` → `sign_and_build` → `rpc.send_transaction` | epoch number, optional recipient | Reward paid to recipient (or sender) |
| Sign / verify an arbitrary message | 1. Sign with chosen address 2. Verify elsewhere | `wallet.sign_message(msg, address)` / `verify_message(msg, sig_hex, pubkey_hex)` | message string, optional address label | Signature hex round-trips through `verify_message` |
| Connect to a network / custom RPC endpoint | 1. Resolve default endpoints or accept custom URL 2. Test connectivity | `default_endpoints(network)`, `network_prefix(network)`, `RpcClient::new(url)`, `rpc.test_connection()` | network name (`mainnet`/`testnet`/`devnet`) or custom URL | `test_connection()` returns `Ok(true)`; `Err` if unreachable |
| Display amounts | 1. Convert base units to/from DOLI string | `units_to_coins(u64)`, `coins_to_units(&str)`, `format_balance(u64)` | base-unit `u64` or decimal string | 8-decimal DOLI string, no floating-point precision loss |

## DATA FLOW

### Wallet Creation
```
Wallet::new(name)
  → Mnemonic::generate(24)       # bip39
  → mnemonic.to_seed("")         # 64-byte seed, empty passphrase
  → seed[..32] → KeyPair::from_seed()   # Ed25519 keypair
  → BlsKeyPair::generate()       # random BLS, NOT derived from seed
  → WalletAddress { address=kp.address().to_hex(), ... }
  → Wallet { version: 2, addresses: [primary] }
```
Location: `wallet.rs:53-84`

### Wallet Restoration
```
Wallet::from_seed_phrase(name, phrase)
  → Mnemonic::parse(phrase)      # validates BIP-39
  → mnemonic.to_seed("")
  → seed[..32] → KeyPair::from_seed()   # SAME Ed25519 key as new()
  → BlsKeyPair::generate()       # NEW random BLS (not deterministic from seed)
```
Location: `wallet.rs:88-115`. **Critical**: BLS key is always random — only Ed25519 is deterministic from seed phrase.

### Transaction Build + Sign
```
TxBuilder::new(TxType::Transfer)
  → add_input(prev_hash, idx)
  → add_output(amount, pubkey_hash, type, lock, extra)
  → sign_and_build(&keypair)
       → build_for_signing()      # signing bytes: version|tx_type|inputs(no sig)|outputs
       → hash = BLAKE3(signing_bytes)
       → sig = Ed25519::sign(hash.as_bytes(), private_key)
       → fill input.signature + input.public_key
       → serialize bincode 1.x manually  # see CONSTRAINTS
       → return hex::encode(buf)
```
Location: `tx_builder/builder.rs:93-237`

### RPC Call Flow
```
RpcClient::get_balance(address)
  → POST endpoint JSON: {"jsonrpc":"2.0","method":"getBalance","params":{"address":...},"id":1}
  → response.error? → Err with code + data["error_code"]
  → response.result? → deserialize to Balance
  → result.is_none()? → Err("No result in response")
```
Location: `rpc_client.rs:78-124`

### UTXO Selection (greedy)
```
build_transfer(utxos, recipient_hash, amount, fee, sender_hash)
  → filter: spendable=true AND output_type="normal"   # excludes bond UTXOs
  → iterate in array order, accumulate until selected >= amount + fee
  → add recipient output (output_type=0, lock_until=0)
  → if selected > amount+fee: add change output (output_type=0, lock_until=0)
```
Location: `tx_builder/builder.rs:245-301`. Same pattern in `build_add_bond` (`tx_builder/builder.rs:304-368`), swapping the recipient output for N bond outputs.

## DEPENDENCIES

**Runtime (no doli-core)** — `Cargo.toml:11-29`:
- `crypto` (workspace) — `KeyPair`, `BlsKeyPair`, `PrivateKey`, `PublicKey`, `Signature`, `hash`, `hash::hash_with_domain`, `signature::sign/verify`, `ADDRESS_DOMAIN`, `address::from_pubkey` (bech32m)
- `bip39` — `Mnemonic::generate`, `Mnemonic::to_seed`, `Mnemonic::parse`
- `zeroize` — Ed25519 seed bytes zeroized after key derivation (`wallet.rs:63,97`)
- `hex` — encoding/decoding
- `serde`, `serde_json` — JSON serialization
- `reqwest` — async HTTP (RPC client)
- `tokio` — async runtime
- `anyhow`, `thiserror` — errors (thiserror listed but `anyhow::Result` used exclusively in the read modules)

**Dev only** (tests) — `Cargo.toml:31-40`:
- `doli-core` — cross-crate serialization compatibility tests (`tests/serialization_compat.rs`)
- `bincode` — byte-identical comparison with core's `bincode::serialize()`
- `wiremock` — mock HTTP server for RPC tests (`rpc_client.rs` test module)
- `tempfile` — temporary directories (`wallet.rs` test module)

**Used By** (cross-domain — verify against consuming crate's own skill):
- `bins/gui` (`doli-gui`) — depends on `wallet = { workspace = true }` directly (`bins/gui/Cargo.toml:16`); `commands::wallet`/`commands::transaction`/`commands::producer` Tauri handlers call `Wallet`/`TxBuilder`/`RpcClient` APIs from this crate.
- `bins/cli` (`doli`) — **does NOT depend on the `wallet` crate** (`bins/cli/Cargo.toml` has no `wallet` entry). It has its OWN parallel copy of wallet/tx-building/RPC-client logic in `bins/cli/src/wallet.rs` (near-identical to `crates/wallet/src/wallet.rs`, kept in sync manually for `wallet.json` format compatibility, GUI-NF-008). Any behavioral change made here does NOT automatically propagate to the CLI — check `bins/cli/src/` separately.

## CONSTRAINTS

### Architectural: No doli-core at Runtime
The wallet crate MUST NOT depend on `doli-core` at runtime (avoids VDF/GMP dependency chain) — see `lib.rs:9` and `tx_builder.rs:1-11`. `doli-core` is a dev-dependency ONLY for compatibility tests. `tx_builder/` duplicates ~200 lines of serialization logic from core.

### Wire Format: bincode 1.x Manual Encoding (`tx_builder/builder.rs:138-237`)
`sign_and_build()` manually produces bincode 1.x format matching `bincode::serialize(doli_core::Transaction)`:
- `u32` → 4 bytes LE
- `u64` → 8 bytes LE
- `Vec<T>` → 8 bytes LE element count + items
- `Vec<u8>` → 8 bytes LE byte count + bytes
- `Hash` (serialize_bytes) → 8 bytes LE (= 32) + 32 bytes raw = 40 bytes total
- `Signature` (serialize_bytes) → 8 bytes LE (= 64) + 64 bytes raw = 72 bytes total
- `enum` → 4 bytes LE variant index
- `Option<T>` → 1 byte discriminant (0=None, 1=Some) + value
- `Input.public_key: Option<PublicKey>` is always `Some` for wallet-built inputs after signing (`builder.rs:203-212`)

### TxType Mapping (`tx_builder/types.rs:37-63`)
Wallet `TxType` discriminants (repr(u8)) differ from core's (repr(u32)) due to historical gaps:
- `RewardClaim(6)` → core `ClaimReward(3)`
- `Coinbase(3)` → core `Coinbase(6)`
- `SlashingEvidence(10)` → core `SlashProducer(5)`
- `NftMint`, `NftTransfer`, `TokenIssuance`, `BridgeLock` → all map to core `Transfer(0)`
- `DelegateBond(13)` → core(13), `RevokeDelegation(14)` → core(14) (direct)
- Core has **24** variants post B.1/B.2 tombstoning (verified by `tests/serialization_compat.rs:test_core_txtype_variant_count`, hardcoded assert `count == 24`); wallet exposes 15 (subset, no DeFi/oracle/ZKSettle types). If core adds/removes types this test fails, reminding the developer to update `TxType` + `to_core_type_id()`.

### Dummy Input for Non-UTXO-Spending Tx Types
`build_request_withdrawal` and `build_reward_claim` call `builder.add_input(sender_hash, 0)` — this passes the **sender's pubkey_hash as if it were a prev_tx_hash**, NOT a real UTXO reference (`tx_builder/builder.rs:392,411`). This satisfies `build_for_signing()`'s "must have ≥1 input" check and carries the signer's identity; it does not reference a spendable output. Node-side validation for these tx types must not treat this input as a real UTXO lookup.

### Signing Message Excludes TX-Level extra_data
`build_for_signing()` excludes `self.extra_data` (SegWit-style). Only output-level `extra_data` is covered by the signature. TX-level extra_data (e.g. for RequestWithdrawal bond_count) is included in the full serialization but NOT signed.

### Constants Must Stay in Sync with doli-core (`tests/serialization_compat.rs:264-340`)
8 constants are duplicated from doli-core in `types.rs:294-318`. Parity verified at test time:
- `BOND_UNIT = 1_000_000_000` (10 DOLI)
- `MAX_BONDS_PER_PRODUCER = 3_000`
- `BLOCKS_PER_REWARD_EPOCH = 360`
- `COINBASE_MATURITY = 6`
- `UNBONDING_PERIOD = 60_480`
- `BASE_REGISTRATION_FEE = 100_000` (0.001 DOLI)
- `MAX_REGISTRATION_FEE = 1_000_000` (0.01 DOLI)
- `VESTING_QUARTER_SLOTS = 3_153_600` (~1 year in mainnet slots)
Fee-tier multiplier (`tx_builder/fees.rs:17-40`) and vesting penalty boundaries are also parity-tested against `doli_core::consensus::fee_multiplier_x100` / `doli_core::withdrawal_penalty_rate_with_quarter`.

### Wallet File Format (GUI-NF-008)
JSON must have exactly 3 top-level keys: `name`, `version`, `addresses`.
Each address: `address`, `public_key`, `private_key`, `label` required; `bls_private_key`/`bls_public_key` optional (omitted from JSON when None, `#[serde(skip_serializing_if = "Option::is_none")]` at `wallet.rs:30-34`).
Version 1 = legacy (no BLS), version 2 = BIP-39 with BLS. Crate can load both. This format is duplicated independently in `bins/cli/src/wallet.rs` — both MUST evolve together or wallet.json files stop being cross-compatible.

### Bond Output Encoding
Bond outputs: `output_type=1`, `lock_until=u64::MAX`, `amount=BOND_UNIT`.
Normal outputs: `output_type=0`, `lock_until=0`.
All wallet-built inputs use `sighash_type=SighashType::All (0)` and `committed_output_count=0` (`tx_builder/builder.rs:197-202`).

## PATTERNS

### Creating and Sending a Transfer
```rust
// 1. Load wallet
let wallet = Wallet::load(&path)?;
let keypair = wallet.primary_keypair()?;
let sender_hash_hex = wallet.primary_pubkey_hash()?;
let sender_hash: [u8; 32] = hex::decode(&sender_hash_hex)?.try_into()?;

// 2. Fetch UTXOs via RPC
let rpc = RpcClient::new("http://127.0.0.1:28500");
let utxos = rpc.get_utxos(&sender_hash_hex, true).await?;

// 3. Build and sign
let recipient_hash: [u8; 32] = hex::decode(recipient_hash_hex)?.try_into()?;
let mut builder = TxBuilder::build_transfer(&utxos, recipient_hash, amount, fee, sender_hash)?;
let tx_hex = builder.sign_and_build(&keypair)?;

// 4. Submit
let tx_hash = rpc.send_transaction(&tx_hex).await?;
```

### Connecting to the Right Network
```rust
let prefix = network_prefix("mainnet"); // "doli"
let endpoints = default_endpoints("mainnet"); // ["https://rpc1.doli.network", ...]
let rpc = RpcClient::new(&endpoints[0]);

// Bech32 address for display
let bech32_addr = wallet.primary_bech32_address(prefix)?; // "doli1..."
// pubkey_hash for RPC queries (not bech32)
let query_addr = wallet.primary_pubkey_hash()?;
```

### Fee Calculation Before Registration
```rust
let epoch_info = rpc.get_epoch_info().await?;
let pending = epoch_info.blocks_remaining as u32; // rough approximation
let (bond_cost, reg_fee, total) = calculate_registration_cost(bond_count, pending)?;
// total = bonds * 10 DOLI + tiered_fee (0.001..0.01 DOLI)
```

### Wallet Restoration (seed phrase recovery)
```rust
// Ed25519 key is deterministic — same address recovered
let wallet = Wallet::from_seed_phrase("restored", &seed_phrase)?;
// BLS key is NEW random — if producer was registered, must update bls_pubkey on-chain
```

### Checking Vesting Before Withdrawal
```rust
let details = rpc.get_bond_details(&pubkey).await?;
for bond in &details.bonds {
    let penalty = vesting_penalty_pct(bond.age_slots);
    let net = calculate_withdrawal_net(bond.amount, penalty);
    // Show user: "Withdrawing this bond costs X% penalty, nets Y DOLI"
}
```

### Custom RPC Endpoint Validation
```rust
let rpc = RpcClient::new(custom_url);
rpc.test_connection().await?; // calls getChainInfo; Err if unreachable
```
