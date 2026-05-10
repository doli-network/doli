# wallet — DOLI Wallet Library
<!-- @INDEX
ENTRY-POINTS: lines 17-30
STRUCTS: lines 32-76
FUNCTIONS: lines 78-157
DATA-FLOWS: lines 159-197
DEPENDENCIES: lines 199-215
CONSTRAINTS: lines 217-250
PATTERNS: lines 252-285
-->

## ENTRY-POINTS

Public re-exports from `crates/wallet/src/lib.rs`:
- `Wallet`, `WalletAddress`, `verify_message` — from `wallet.rs`
- `TxBuilder`, `TxType` — from `tx_builder/builder.rs`, `tx_builder/types.rs`
- `calculate_registration_cost`, `calculate_withdrawal_net`, `vesting_penalty_pct` — from `tx_builder/fees.rs`
- `RpcClient`, `default_endpoints`, `network_prefix` — from `rpc_client.rs`
- All types from `types.rs` (Balance, Utxo, ChainInfo, etc.) via `pub use types::*`

Shared between CLI (`bins/cli`) and GUI (`bins/gui`). Wallet file format is binary-compatible with both.

## STRUCTS

### `wallet.rs`

**`Wallet`** (serialize/deserialize, JSON file format):
- `name: String`
- `version: u32` — 1 = legacy random key, 2 = BIP-39 derived
- `addresses: Vec<WalletAddress>`

**`WalletAddress`** (serialize/deserialize):
- `address: String` — 20-byte truncated hash, hex
- `public_key: String` — Ed25519 pubkey, 32 bytes hex (64 chars)
- `private_key: String` — Ed25519 privkey, 32 bytes hex, field is NOT `pub`
- `label: Option<String>`
- `bls_private_key: Option<String>` — 32 bytes hex, skip_serializing_if None
- `bls_public_key: Option<String>` — 48 bytes hex (96 chars), skip_serializing_if None

### `tx_builder/types.rs`

**`TxType`** (repr(u8), Copy):
- Transfer=0, Registration=1, ProducerExit=2, Coinbase=3, NftMint=4, NftTransfer=5
- RewardClaim=6, AddBond=7, RequestWithdrawal=8, ClaimWithdrawal=9 (tombstone)
- SlashingEvidence=10, TokenIssuance=11, BridgeLock=12, DelegateBond=13, RevokeDelegation=14
- `to_core_type_id() -> u32` — maps wallet variants to doli-core bincode discriminants

**`TxInput`**:
- `prev_tx_hash: [u8; 32]`, `output_index: u32`
- `signature: Option<Vec<u8>>` — filled during `sign_and_build()`
- `public_key: Option<Vec<u8>>` — filled during `sign_and_build()`

**`TxOutput`**:
- `amount: u64`, `pubkey_hash: [u8; 32]`, `output_type: u8`, `lock_until: u64`, `extra_data: Vec<u8>`

**`TxBuilder`** (`tx_builder/builder.rs`):
- `tx_type: TxType`, `inputs: Vec<TxInput>`, `outputs: Vec<TxOutput>`, `extra_data: Vec<u8>`

### `types.rs` (RPC response types)

`Balance`, `Utxo`, `ChainInfo`, `HistoryEntry`, `ProducerInfo`, `PendingWithdrawalInfo`,
`PendingUpdateInfo`, `BondDetailsInfo`, `BondsSummaryInfo`, `BondEntryInfo`, `RewardEpoch`,
`EpochInfo`, `NetworkParams`, `WithdrawalSimulation`, `BondWithdrawalDetail`

All use `#[serde(rename_all = "camelCase")]` to match node JSON-RPC responses.

### `rpc_client.rs`

**`RpcClient`**: `endpoint: String`, `client: reqwest::Client`
- Timeouts: connect=5s, request=30s

## FUNCTIONS

### `wallet.rs` — `Wallet` impl

| Method | Signature | Notes |
|--------|-----------|-------|
| `new` | `(name: &str) -> (Self, String)` | Creates BIP-39 wallet; returns (wallet, 24-word phrase). Phrase NOT stored. |
| `from_seed_phrase` | `(name: &str, phrase: &str) -> Result<Self>` | Same Ed25519 key as `new()` from same phrase; BLS key is random (not derived). |
| `load` | `(path: &Path) -> Result<Self>` | Reads JSON file; errors say "wallet file not found" or "failed to parse". |
| `save` | `(&self, path: &Path) -> Result<()>` | Pretty JSON; creates parent dirs. |
| `export` | `(&self, path: &Path) -> Result<()>` | Alias for `save`. |
| `import` | `(path: &Path) -> Result<Self>` | Alias for `load`. |
| `name` | `(&self) -> &str` | |
| `version` | `(&self) -> u32` | |
| `addresses` | `(&self) -> &[WalletAddress]` | |
| `primary_address` | `(&self) -> &str` | `addresses[0].address` |
| `primary_public_key` | `(&self) -> &str` | `addresses[0].public_key` |
| `primary_pubkey_hash` | `(&self) -> Result<String>` | 32-byte BLAKE3 with ADDRESS_DOMAIN, hex. Used for RPC queries. |
| `primary_bech32_address` | `(&self, network_prefix: &str) -> Result<String>` | Prefixes: `doli`, `tdoli`, `ddoli` |
| `primary_keypair` | `(&self) -> Result<KeyPair>` | Reconstructs Ed25519 keypair from stored private key hex. |
| `has_bls_key` | `(&self) -> bool` | |
| `primary_bls_public_key` | `(&self) -> Option<&str>` | |
| `generate_address` | `(&mut self, label: Option<&str>) -> Result<String>` | Random Ed25519, no BLS key. Appends to `addresses`. |
| `add_bls_key` | `(&mut self) -> Result<String>` | Errors if BLS key exists. Returns BLS pubkey hex. |
| `sign_message` | `(&self, message: &str, address: Option<&str>) -> Result<String>` | Signs BLAKE3(message), returns Ed25519 sig hex. |

**`verify_message`** (free fn): `(message: &str, sig_hex: &str, pubkey_hex: &str) -> Result<bool>`

### `tx_builder/builder.rs` — `TxBuilder` impl

| Method | Signature | Notes |
|--------|-----------|-------|
| `new` | `(tx_type: TxType) -> Self` | |
| `add_input` | `(&mut self, prev_tx_hash: [u8; 32], output_index: u32) -> &mut Self` | Signature/pubkey filled later. |
| `add_output` | `(&mut self, amount: u64, pubkey_hash: [u8; 32], output_type: u8, lock_until: u64, extra_data: Vec<u8>) -> &mut Self` | |
| `set_extra_data` | `(&mut self, data: Vec<u8>) -> &mut Self` | TX-level extra_data (NOT output extra_data). |
| `input_count`, `output_count`, `tx_type` | getters | |
| `build_for_signing` | `(&self) -> Result<Vec<u8>>` | Canonical signing bytes; excludes TX-level extra_data (SegWit-style). Errors if no inputs (non-Coinbase) or no outputs. |
| `sign_and_build` | `(&mut self, keypair: &KeyPair) -> Result<String>` | Signs → fills input sigs → serializes bincode 1.x → returns hex. |
| `build_transfer` | `(utxos: &[Utxo], recipient_hash: [u8; 32], amount: u64, fee: u64, sender_hash: [u8; 32]) -> Result<Self>` | Greedy UTXO selection (spendable + output_type="normal" only). Change output added if any. |
| `build_add_bond` | `(utxos: &[Utxo], bond_count: u32, sender_hash: [u8; 32], fee: u64) -> Result<Self>` | Creates N bond outputs (output_type=1, lock_until=u64::MAX, amount=BOND_UNIT) + change. |
| `build_request_withdrawal` | `(bond_count: u32, sender_hash: [u8; 32], destination_hash: Option<[u8; 32]>) -> Result<Self>` | bond_count in extra_data as LE u32; optional destination appended. |
| `build_reward_claim` | `(epoch: u64, sender_hash: [u8; 32], recipient_hash: Option<[u8; 32]>) -> Result<Self>` | epoch in extra_data as LE u64. |

### `tx_builder/fees.rs`

| Function | Signature | Notes |
|----------|-----------|-------|
| `calculate_registration_cost` | `(bond_count: u32, pending_registrations: u32) -> Result<(u64, u64, u64)>` | Returns (bond_cost, reg_fee, total). Fee = BASE_REGISTRATION_FEE × multiplier / 100, capped at MAX. |
| `vesting_penalty_pct` | `(age_slots: u64) -> u8` | Q1 (0..QUARTER)=75%, Q2=50%, Q3=25%, vested=0%. |
| `calculate_withdrawal_net` | `(bond_amount: u64, penalty_pct: u8) -> u64` | `bond_amount - bond_amount * pct / 100` |

### `rpc_client.rs` — `RpcClient` impl (all async)

| Method | RPC call |
|--------|---------|
| `get_balance(&str) -> Result<Balance>` | `getBalance` |
| `get_utxos(&str, bool) -> Result<Vec<Utxo>>` | `getUtxos` |
| `send_transaction(&str) -> Result<String>` | `sendTransaction` → returns tx hash |
| `get_chain_info() -> Result<ChainInfo>` | `getChainInfo` |
| `get_history(&str, u32) -> Result<Vec<HistoryEntry>>` | `getHistory` |
| `get_producers() -> Result<Vec<ProducerInfo>>` | `getProducers` |
| `get_network_params() -> Result<NetworkParams>` | `getNetworkParams` |
| `get_epoch_info() -> Result<EpochInfo>` | `getEpochInfo` |
| `get_rewards_list(&str) -> Result<Vec<RewardEpoch>>` | `getRewardsList` — param: `publicKey` |
| `get_bond_details(&str) -> Result<BondDetailsInfo>` | `getBondDetails` — param: `publicKey` |
| `simulate_withdrawal(&str, u32) -> Result<WithdrawalSimulation>` | `simulateWithdrawal` |
| `test_connection() -> Result<bool>` | calls `get_chain_info` |

**Free fns**:
- `default_endpoints(network: &str) -> Vec<String>` — mainnet: 2 HTTPS, testnet: 1, devnet: `http://127.0.0.1:28500`, unknown: empty
- `network_prefix(network: &str) -> &str` — `"doli"` / `"tdoli"` / `"ddoli"` / default `"doli"`

### `types.rs` — unit conversion

| Function | Notes |
|----------|-------|
| `units_to_coins(u64) -> String` | `"X.XXXXXXXX"`, 8 decimal places, no float |
| `coins_to_units(&str) -> Result<u64, String>` | Parses decimal string, rejects negative/overflow/empty/multi-dot |
| `format_balance(u64) -> String` | `"X.XXXXXXXX DOLI"` |

## DATA-FLOWS

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

### Wallet Restoration
```
Wallet::from_seed_phrase(name, phrase)
  → Mnemonic::parse(phrase)      # validates BIP-39
  → mnemonic.to_seed("")
  → seed[..32] → KeyPair::from_seed()   # SAME Ed25519 key as new()
  → BlsKeyPair::generate()       # NEW random BLS (not deterministic from seed)
```

**Critical**: BLS key is always random. Only Ed25519 is deterministic from seed phrase.

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
       → serialize bincode 1.x manually  # see Constraints
       → return hex::encode(buf)
```

### RPC Call Flow
```
RpcClient::get_balance(address)
  → POST endpoint JSON: {"jsonrpc":"2.0","method":"getBalance","params":{"address":...},"id":1}
  → response.error? → Err with code + data["error_code"]
  → response.result? → deserialize to Balance
  → result.is_none()? → Err("No result in response")
```

### UTXO Selection (greedy)
```
build_transfer(utxos, recipient_hash, amount, fee, sender_hash)
  → filter: spendable=true AND output_type="normal"
  → iterate in array order, accumulate until selected >= amount + fee
  → add recipient output (output_type=0, lock_until=0)
  → if selected > amount+fee: add change output (output_type=0, lock_until=0)
```

## DEPENDENCIES

**Runtime (no doli-core)**:
- `crypto` (workspace) — `KeyPair`, `BlsKeyPair`, `PrivateKey`, `PublicKey`, `Signature`, `hash`, `signature::sign/verify`, `ADDRESS_DOMAIN`
- `bip39` — `Mnemonic::generate`, `Mnemonic::to_seed`, `Mnemonic::parse`
- `zeroize` — Ed25519 seed bytes zeroized after key derivation (`wallet.rs:63`)
- `hex` — encoding/decoding
- `serde`, `serde_json` — JSON serialization
- `reqwest` — async HTTP (RPC client)
- `tokio` — async runtime
- `anyhow`, `thiserror` — errors

**Dev only** (tests):
- `doli-core` — cross-crate serialization compatibility tests (`tests/serialization_compat.rs`)
- `bincode` — byte-identical comparison with core's `bincode::serialize()`
- `wiremock` — mock HTTP server for RPC tests
- `tempfile` — temporary directories

## CONSTRAINTS

### Architectural: No doli-core at Runtime
The wallet crate MUST NOT depend on `doli-core` at runtime (avoids VDF/GMP chain).
`doli-core` is a dev-dependency ONLY for compatibility tests. `tx_builder/` duplicates ~200 lines of serialization logic from core.

### Wire Format: bincode 1.x Manual Encoding (`builder.rs:138-237`)
`sign_and_build()` manually produces bincode 1.x format matching `bincode::serialize(doli_core::Transaction)`:
- `u32` → 4 bytes LE
- `u64` → 8 bytes LE
- `Vec<T>` → 8 bytes LE element count + items
- `Vec<u8>` → 8 bytes LE byte count + bytes
- `Hash` (serialize_bytes) → 8 bytes LE (= 32) + 32 bytes raw = 40 bytes total
- `Signature` (serialize_bytes) → 8 bytes LE (= 64) + 64 bytes raw = 72 bytes total
- `enum` → 4 bytes LE variant index
- `Option<T>` → 1 byte discriminant (0=None, 1=Some) + value

### TxType Mapping (`tx_builder/types.rs:44-62`)
Wallet `TxType` discriminants (repr(u8)) differ from core's (repr(u32)) due to historical gaps:
- `RewardClaim(6)` → core `ClaimReward(3)`
- `Coinbase(3)` → core `Coinbase(6)`
- `SlashingEvidence(10)` → core `SlashProducer(5)`
- NftMint, NftTransfer, TokenIssuance, BridgeLock → all map to core `Transfer(0)`
- DelegateBond(13) → core(13), RevokeDelegation(14) → core(14) (direct)
- Core has 30 variants; wallet exposes 15 (subset). If core adds types, `serialization_compat.rs:test_core_txtype_variant_count` will fail.

### Signing Message Excludes TX-Level extra_data
`build_for_signing()` excludes `self.extra_data` (SegWit-style). Only output-level `extra_data` is covered by the signature. TX-level extra_data (e.g. for RequestWithdrawal bond_count) is included in the full serialization but NOT signed.

### Constants Must Stay in Sync with doli-core (`tests/serialization_compat.rs:268-340`)
8 constants are duplicated from doli-core. Parity verified at test time:
- `BOND_UNIT = 1_000_000_000` (10 DOLI)
- `MAX_BONDS_PER_PRODUCER = 3_000`
- `BLOCKS_PER_REWARD_EPOCH = 360`
- `COINBASE_MATURITY = 6`
- `UNBONDING_PERIOD = 60_480`
- `BASE_REGISTRATION_FEE = 100_000` (0.001 DOLI)
- `MAX_REGISTRATION_FEE = 1_000_000` (0.01 DOLI)
- `VESTING_QUARTER_SLOTS = 3_153_600` (~1 year in mainnet slots)

### Wallet File Format (GUI-NF-008)
JSON must have exactly 3 top-level keys: `name`, `version`, `addresses`.
Each address: `address`, `public_key`, `private_key`, `label` required; `bls_private_key`/`bls_public_key` optional (omitted from JSON when None).
Version 1 = legacy (no BLS), version 2 = BIP-39 with BLS. Crate can load both.

### Bond Output Encoding
Bond outputs: `output_type=1`, `lock_until=u64::MAX`, `amount=BOND_UNIT`.
Normal outputs: `output_type=0`, `lock_until=0`.
All wallet-built inputs use `sighash_type=SighashType::All (0)` and `committed_output_count=0`.

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
