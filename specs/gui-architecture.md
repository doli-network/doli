# Architecture: DOLI GUI Desktop Application

## Scope

This architecture covers the DOLI GUI Desktop Wallet -- a cross-platform Tauri 2.x application that provides a graphical interface to DOLI blockchain functionality. It covers:

- New binary crate: `bins/gui/` (Tauri desktop application)
- New library crate: `crates/wallet/` (shared wallet + RPC logic extracted from CLI)
- Modified crate: `crates/core/` (VDF feature flag)
- Modified CI: `.github/workflows/release.yml` (multi-platform GUI builds)
- Frontend: Svelte 5 SPA within Tauri webview

Requirements document: `specs/gui-desktop-requirements.md`

---

## Overview

```
                              DOLI GUI Architecture

  ┌─────────────────────────────────────────────────────────────────────┐
  │                        Tauri Desktop App                             │
  │                                                                      │
  │   ┌───────────────────────────────────────────────┐                 │
  │   │              Frontend (Svelte 5)               │                 │
  │   │                                                │                 │
  │   │  ┌─────────┐ ┌──────────┐ ┌──────────────┐   │                 │
  │   │  │ Wallet  │ │ Producer │ │   Network    │   │                 │
  │   │  │  Views  │ │Dashboard │ │  Settings    │   │                 │
  │   │  └────┬────┘ └────┬─────┘ └──────┬───────┘   │                 │
  │   │       │           │              │            │                 │
  │   │  ┌────┴───────────┴──────────────┴─────────┐  │                 │
  │   │  │         Svelte Stores (state)            │  │                 │
  │   │  └────────────────┬────────────────────────┘  │                 │
  │   └──────────────────│────────────────────────────┘                 │
  │                      │ Tauri IPC (invoke)                           │
  │   ┌──────────────────┴────────────────────────────┐                 │
  │   │           Rust Backend (src-tauri/)             │                 │
  │   │                                                 │                 │
  │   │  ┌───────────┐  ┌──────────┐  ┌─────────────┐ │                 │
  │   │  │  Commands  │  │  State   │  │   Config    │ │                 │
  │   │  │ (wallet,   │  │ Manager  │  │  Manager    │ │                 │
  │   │  │  tx, rpc)  │  │          │  │             │ │                 │
  │   │  └──────┬─────┘  └──────────┘  └─────────────┘ │                 │
  │   │         │                                       │                 │
  │   │  ┌──────┴──────────────────────────────────┐   │                 │
  │   │  │        crates/wallet (shared)            │   │                 │
  │   │  │  ┌──────────┐  ┌──────────────────────┐ │   │                 │
  │   │  │  │ Wallet   │  │  RPC Client          │ │   │                 │
  │   │  │  │ (keys,   │  │  (JSON-RPC over HTTP)│ │   │                 │
  │   │  │  │  sign)   │  │                      │ │   │                 │
  │   │  │  └──────────┘  └──────────┬───────────┘ │   │                 │
  │   │  └───────────────────────────│─────────────┘   │                 │
  │   └──────────────────────────────│─────────────────┘                 │
  └──────────────────────────────────│──────────────────────────────────┘
                                     │ HTTP POST (JSON-RPC)
                                     ▼
                          ┌────────────────────┐
                          │  DOLI Node (RPC)   │
                          │  Public or Local   │
                          └────────────────────┘
```

---

## Milestones

| ID | Name | Scope (Modules) | Scope (Requirements) | Est. Size | Dependencies |
|----|------|-----------------|---------------------|-----------|-------------|
| M1 | Core Infrastructure | `crates/wallet/`, VDF feature flag in `crates/core/`, workspace changes | GUI-NF-006, GUI-NF-008, GUI-NF-013 | M | None |
| M2 | Tauri App Shell + Wallet | `bins/gui/` Tauri app, wallet views, network connection | GUI-FR-001 to GUI-FR-008, GUI-FR-015, GUI-FR-080 to GUI-FR-083, GUI-FR-070, GUI-NF-001, GUI-NF-004, GUI-NF-011 | L | M1 |
| M3 | Transactions + Balance | Balance display, send/receive, transaction history | GUI-FR-010, GUI-FR-011, GUI-FR-014, GUI-FR-016 | M | M1, M2 |
| M4 | Producer + Rewards | Producer registration, dashboard, bonds, rewards | GUI-FR-020 to GUI-FR-027, GUI-FR-030 to GUI-FR-034 | L | M1, M2, M3 |
| M5 | NFT + Bridge + Governance | NFT minting/transfer, bridge HTLC, governance voting | GUI-FR-040 to GUI-FR-044, GUI-FR-050 to GUI-FR-052, GUI-FR-060 to GUI-FR-064 | M | M1, M2, M3 |
| M6 | CI/CD + Packaging | GitHub Actions multi-platform builds, auto-update, code signing hooks | GUI-NF-002, GUI-NF-003, GUI-NF-005, GUI-NF-007, GUI-NF-009, GUI-NF-010, GUI-NF-012 | M | M1, M2 |
| M7 | Could-Priority Features | Covenants, embedded node, sign/verify, delegation, chain verify, i18n | GUI-FR-012, GUI-FR-013, GUI-FR-071, GUI-FR-090 to GUI-FR-092, GUI-FR-100, GUI-FR-101, GUI-FR-110, GUI-FR-111, GUI-NF-014 | L | M1-M6 |

---

## Modules

### Module 1: `crates/wallet/` (Shared Wallet Library)

- **Responsibility**: Wallet file management (create, restore, load, save, export, import), key derivation (BIP-39 to Ed25519), address generation, transaction signing, BLS key management, and JSON-RPC client for communicating with DOLI nodes. This crate is shared between `bins/cli` and `bins/gui` to guarantee wallet format compatibility (GUI-NF-008).
- **Public interface**:
  ```rust
  // Wallet operations
  pub struct Wallet { ... }
  pub struct WalletAddress { ... }
  impl Wallet {
      pub fn new(name: &str) -> (Self, String);               // Returns (wallet, seed_phrase)
      pub fn from_seed_phrase(name: &str, phrase: &str) -> Result<Self>;
      pub fn load(path: &Path) -> Result<Self>;
      pub fn save(&self, path: &Path) -> Result<()>;
      pub fn export(&self, path: &Path) -> Result<()>;
      pub fn import(path: &Path) -> Result<Self>;
      pub fn generate_address(&mut self, label: Option<&str>) -> Result<String>;
      pub fn addresses(&self) -> &[WalletAddress];
      pub fn primary_keypair(&self) -> Result<KeyPair>;
      pub fn primary_bech32_address(&self, prefix: &str) -> String;
      pub fn primary_pubkey_hash(&self) -> String;
      pub fn has_bls_key(&self) -> bool;
      pub fn add_bls_key(&mut self) -> Result<String>;
      pub fn sign_message(&self, message: &str, address: Option<&str>) -> Result<String>;
      pub fn name(&self) -> &str;
      pub fn version(&self) -> u32;
  }
  pub fn verify_message(message: &str, sig_hex: &str, pubkey_hex: &str) -> Result<bool>;

  // RPC client
  pub struct RpcClient { ... }
  impl RpcClient {
      pub fn new(url: &str) -> Self;
      pub fn url(&self) -> &str;
      pub async fn get_balance(&self, pubkey_hash: &str) -> Result<Balance>;
      pub async fn get_utxos(&self, pubkey_hash: &str) -> Result<Vec<Utxo>>;
      pub async fn send_transaction(&self, tx_hex: &str) -> Result<String>;
      pub async fn get_chain_info(&self) -> Result<ChainInfo>;
      pub async fn get_history(&self, pubkey_hash: &str, limit: u32) -> Result<Vec<HistoryEntry>>;
      pub async fn get_producers(&self) -> Result<Vec<ProducerInfo>>;
      pub async fn get_network_params(&self) -> Result<NetworkParams>;
      pub async fn get_rewards_list(&self, pubkey: &str) -> Result<Vec<RewardEpoch>>;
      // ... all 31 RPC methods wrapped
  }

  // Response types
  pub struct Balance { ... }
  pub struct Utxo { ... }
  pub struct ChainInfo { ... }
  pub struct HistoryEntry { ... }
  pub struct ProducerInfo { ... }
  pub struct NetworkParams { ... }
  // ... etc

  // Utility
  pub fn coins_to_units(coins: &str) -> Result<u64>;
  pub fn units_to_coins(units: u64) -> String;
  pub fn format_balance(units: u64) -> String;
  ```
- **Dependencies**: `crypto` (Ed25519, BLAKE3, BLS, bech32m addresses), `bip39`, `serde`/`serde_json` (wallet serialization), `reqwest` (HTTP client), `anyhow`/`thiserror`, `hex`, `zeroize`
- **Does NOT depend on**: `doli-core`, `vdf` -- the wallet crate avoids the VDF dependency chain entirely
- **Implementation order**: 1 (foundation for everything else)

#### Design Decision: No `doli-core` Dependency

The wallet crate does NOT depend on `doli-core`. This is a critical architectural choice:

1. `doli-core` depends on `vdf` which depends on `rug` (GMP). The entire point of the VDF feature flag (GUI-NF-013) is to avoid GMP for wallet-only builds.
2. The CLI currently uses `doli-core` types (`Transaction`, `Input`, `Output`, `RegistrationData`) for building transactions. In the GUI, transaction construction happens differently:
   - **Simple transactions** (send, rewards claim): Built by the Tauri backend using raw byte serialization from `crates/wallet/` without `doli-core` types
   - **Registration transactions**: Require VDF computation (`hash_chain_vdf`). Registration is gated behind the producer feature path which requires running a node -- and producers already have GMP available

The wallet crate provides a `TxBuilder` module that constructs transaction bytes directly using the canonical encoding (matching `doli-core`'s serialization) without importing `doli-core` types. This duplicates the serialization logic (~200 lines) but eliminates the GMP dependency for 90% of users (wallet-only).

**Alternative considered**: Making `doli-core` compilable without VDF via feature flags. Rejected because VDF types (`VdfOutput`, `VdfProof`) are embedded in `BlockHeader` and `Transaction` struct definitions, requiring pervasive `#[cfg(feature = "vdf")]` throughout the codebase. The blast radius is too large and risks breaking consensus-critical code.

**Revised approach for VDF feature flag** (GUI-NF-013): Instead of gating VDF in `doli-core`, we create a `crates/wallet/` crate that depends only on `crypto` and implements its own transaction builder. The `doli-core` Cargo.toml is modified minimally -- the `vdf` dependency gets a feature flag, but the types `VdfOutput` and `VdfProof` are re-defined as simple `Vec<u8>` wrappers in a stub module when VDF is disabled. This allows `doli-core` to compile without GMP while keeping its type signatures stable.

```toml
# crates/core/Cargo.toml (modified)
[features]
default = ["vdf"]
vdf = ["dep:vdf"]

[dependencies]
vdf = { workspace = true, optional = true }
```

In `crates/core/src/lib.rs`:
```rust
#[cfg(feature = "vdf")]
pub use vdf;

#[cfg(not(feature = "vdf"))]
pub mod vdf_stub {
    // Minimal stubs for VdfOutput and VdfProof (just Vec<u8> wrappers)
    // Serde-compatible, sufficient for deserialization
    // No compute/verify functions
}
```

This hybrid approach: the wallet crate itself avoids `doli-core` entirely, but `doli-core` also gains a feature flag for future use cases (e.g., block explorers, analytics tools).

#### Failure Modes

| Failure | Cause | Detection | Recovery | Impact |
|---------|-------|-----------|----------|--------|
| Wallet file not found | Missing file, wrong path | `std::fs::read_to_string` returns Err | Prompt user to create/import wallet | Cannot operate until wallet exists |
| Wallet file corrupt | Disk error, partial write, manual edit | JSON parse failure | Show error, suggest backup restore | Wallet unusable until fixed |
| Wallet file locked | Another process has file lock | File lock check on open | Show "wallet in use" message | Cannot save changes |
| RPC endpoint unreachable | Network down, node offline, wrong URL | reqwest timeout (5s) | Try fallback endpoints, show disconnected status | Read-only mode (can view wallet, not query chain) |
| RPC response malformed | Node version mismatch, protocol change | JSON deserialization error | Show "incompatible node" error with version info | Feature degraded |
| Key derivation failure | Invalid seed phrase, crypto error | Mnemonic parse error | Show specific validation error | Cannot create/restore wallet |

#### Security Considerations

- **Trust boundary**: Wallet file on disk is the trust boundary. Private keys in plaintext JSON (matching CLI behavior per GUI-NF-004 acceptance criteria).
- **Sensitive data**: Ed25519 private keys, BLS private keys. Keys are `zeroize`-on-drop in memory. Never transmitted over RPC. Never exposed to frontend JavaScript.
- **Attack surface**: Wallet file on local filesystem. An attacker with filesystem access can steal keys. Mitigation: standard OS file permissions (0600). Future: wallet encryption (deferred).
- **Mitigations**: All signing operations happen in Rust. The `sign_transaction()` method takes a transaction template and returns a signed byte blob -- private keys never cross the IPC boundary.

#### Performance Budget

- **Wallet load**: < 10ms for a wallet with 100 addresses
- **RPC call latency**: < 200ms p99 (network-dependent)
- **Key derivation**: < 50ms (BIP-39 to Ed25519)
- **Transaction signing**: < 5ms
- **Memory**: < 5 MB for wallet + RPC client state

---

### Module 2: `bins/gui/src-tauri/` (Tauri Rust Backend)

- **Responsibility**: Tauri command handlers that bridge the frontend (Svelte) to the wallet library. Manages application state, configuration persistence, wallet lifecycle, and RPC endpoint management. Enforces that private keys never reach the frontend.
- **Public interface** (Tauri commands exposed to frontend via `#[tauri::command]`):

  ```rust
  // Wallet commands
  #[tauri::command]
  async fn create_wallet(name: String, wallet_path: String) -> Result<CreateWalletResponse, String>;
  #[tauri::command]
  async fn restore_wallet(name: String, seed_phrase: String, wallet_path: String) -> Result<(), String>;
  #[tauri::command]
  async fn load_wallet(wallet_path: String) -> Result<WalletInfo, String>;
  #[tauri::command]
  async fn generate_address(label: Option<String>) -> Result<AddressInfo, String>;
  #[tauri::command]
  async fn list_addresses() -> Result<Vec<AddressInfo>, String>;
  #[tauri::command]
  async fn export_wallet(destination: String) -> Result<(), String>;
  #[tauri::command]
  async fn import_wallet(source: String, destination: String) -> Result<WalletInfo, String>;
  #[tauri::command]
  async fn wallet_info() -> Result<WalletInfo, String>;
  #[tauri::command]
  async fn add_bls_key() -> Result<String, String>;

  // Balance & Transaction commands
  #[tauri::command]
  async fn get_balance(address: Option<String>) -> Result<BalanceResponse, String>;
  #[tauri::command]
  async fn send_doli(to: String, amount: String, fee: Option<String>) -> Result<SendResponse, String>;
  #[tauri::command]
  async fn get_history(limit: Option<u32>) -> Result<Vec<HistoryEntryResponse>, String>;

  // Producer commands
  #[tauri::command]
  async fn register_producer(bond_count: u32) -> Result<TxResponse, String>;
  #[tauri::command]
  async fn producer_status() -> Result<ProducerStatusResponse, String>;
  #[tauri::command]
  async fn add_bonds(count: u32) -> Result<TxResponse, String>;
  #[tauri::command]
  async fn request_withdrawal(bond_count: u32, dest: Option<String>) -> Result<TxResponse, String>;
  #[tauri::command]
  async fn simulate_withdrawal(bond_count: u32) -> Result<SimulateResponse, String>;
  #[tauri::command]
  async fn exit_producer(force: bool) -> Result<TxResponse, String>;

  // Rewards commands
  #[tauri::command]
  async fn list_rewards() -> Result<Vec<RewardEpochResponse>, String>;
  #[tauri::command]
  async fn claim_reward(epoch: u32, recipient: Option<String>) -> Result<TxResponse, String>;
  #[tauri::command]
  async fn claim_all_rewards() -> Result<Vec<TxResponse>, String>;

  // NFT & Token commands
  #[tauri::command]
  async fn mint_nft(content: String, value: Option<String>) -> Result<TxResponse, String>;
  #[tauri::command]
  async fn transfer_nft(utxo_ref: String, to: String) -> Result<TxResponse, String>;
  #[tauri::command]
  async fn nft_info(utxo_ref: String) -> Result<NftInfoResponse, String>;
  #[tauri::command]
  async fn issue_token(ticker: String, supply: String) -> Result<TxResponse, String>;
  #[tauri::command]
  async fn token_info(utxo_ref: String) -> Result<TokenInfoResponse, String>;

  // Bridge commands
  #[tauri::command]
  async fn bridge_lock(params: BridgeLockParams) -> Result<TxResponse, String>;
  #[tauri::command]
  async fn bridge_claim(utxo_ref: String, preimage: String) -> Result<TxResponse, String>;
  #[tauri::command]
  async fn bridge_refund(utxo_ref: String) -> Result<TxResponse, String>;

  // Governance commands
  #[tauri::command]
  async fn check_updates() -> Result<Vec<UpdateInfo>, String>;
  #[tauri::command]
  async fn update_status() -> Result<UpdateStatusResponse, String>;
  #[tauri::command]
  async fn vote_update(version: String, approve: bool) -> Result<TxResponse, String>;

  // Network & chain commands
  #[tauri::command]
  async fn get_chain_info() -> Result<ChainInfoResponse, String>;
  #[tauri::command]
  async fn set_rpc_endpoint(url: String) -> Result<bool, String>;
  #[tauri::command]
  async fn set_network(network: String) -> Result<(), String>;
  #[tauri::command]
  async fn test_connection(url: String) -> Result<ConnectionTestResult, String>;
  #[tauri::command]
  async fn get_connection_status() -> Result<ConnectionStatus, String>;

  // Sign/verify (Could priority)
  #[tauri::command]
  async fn sign_message(message: String, address: Option<String>) -> Result<String, String>;
  #[tauri::command]
  async fn verify_signature(message: String, signature: String, pubkey: String) -> Result<bool, String>;
  ```

- **Dependencies**: `crates/wallet` (wallet + RPC), `tauri` (app framework), `serde`/`serde_json`, `tokio`
- **Does NOT depend on**: `doli-core`, `vdf`, `rug` -- no GMP required
- **Implementation order**: 2 (after wallet crate)

#### State Management (Backend)

The Tauri backend manages two pieces of state via `tauri::State<>`:

```rust
pub struct AppState {
    /// Currently loaded wallet (None if no wallet loaded)
    wallet: RwLock<Option<Wallet>>,
    /// Path to current wallet file
    wallet_path: RwLock<Option<PathBuf>>,
    /// RPC client for node communication
    rpc_client: RwLock<RpcClient>,
    /// Application configuration (persisted)
    config: RwLock<AppConfig>,
}

pub struct AppConfig {
    /// Selected network (mainnet, testnet, devnet)
    network: String,
    /// Custom RPC endpoint (None = use default for network)
    custom_rpc_url: Option<String>,
    /// Default wallet path
    default_wallet_path: PathBuf,
    /// Last used wallet path
    last_wallet_path: Option<PathBuf>,
    /// RPC polling interval (seconds)
    poll_interval: u32,
    /// Fallback RPC endpoints per network
    rpc_endpoints: HashMap<String, Vec<String>>,
}
```

Configuration persists to `~/.doli-gui/config.json` (separate from CLI's `~/.doli/`).

#### Default RPC Endpoints

Since no public RPC endpoints currently exist (Assumption #2 from requirements), the architecture must handle this gracefully:

```rust
const DEFAULT_ENDPOINTS: &[(&str, &[&str])] = &[
    ("mainnet", &["https://rpc1.doli.network", "https://rpc2.doli.network"]),
    ("testnet", &["https://testnet-rpc.doli.network"]),
    ("devnet",  &["http://127.0.0.1:28500"]),
];
```

The app will ship with placeholder URLs. Before GUI release, at least 2 public mainnet RPC endpoints must be deployed. If none are reachable at startup, the app shows a clear "Cannot connect" message with instructions to either wait for public endpoints or configure a custom one.

#### Failure Modes

| Failure | Cause | Detection | Recovery | Impact |
|---------|-------|-----------|----------|--------|
| All RPC endpoints unreachable | Network down, no public endpoints deployed | Sequential connection failures to all configured endpoints | Show "disconnected" indicator, allow wallet operations (view addresses, export), disable chain queries | Wallet management works, balance/send/producer features disabled |
| Wallet file write failure | Disk full, permissions | `std::fs::write` error | Show error, suggest alternative path | Changes not saved, warn user |
| Concurrent wallet access | CLI and GUI open same wallet | File lock (advisory) | Show warning, offer read-only mode | Potential data loss if both write |
| Tauri webview crash | Memory pressure, rendering bug | Tauri crash handler | Auto-restart webview, preserve backend state | Momentary UI disruption |
| Backend panic | Bug in command handler | Tauri catches panics in commands | Return error to frontend, log stack trace | Single operation fails, app continues |

#### Security Considerations

- **Trust boundary**: The Tauri IPC channel is the primary trust boundary. The frontend (JavaScript) is treated as untrusted. All security-critical operations (key access, signing) happen exclusively in Rust.
- **Tauri CSP**: The `tauri.conf.json` enforces a strict Content Security Policy. No inline scripts, no eval, connect-src limited to RPC endpoints. No external resources loaded.
- **IPC security**: Tauri 2.x command permissions restrict which commands the frontend can invoke. All commands validate inputs before processing.
- **Sensitive data flow**: Private keys are loaded in Rust memory, used for signing, then dropped (zeroized). The signed transaction bytes (public data) are returned to the frontend for display. At no point does key material cross the IPC boundary.
- **No raw key exposure**: Commands like `list_addresses` return public keys and addresses only. No command returns private keys to the frontend.

#### Performance Budget

- **Command response**: < 50ms for local operations (wallet read/sign), < 500ms for RPC operations
- **Memory (wallet mode)**: < 80 MB RSS (Tauri webview + Rust backend)
- **Startup**: < 3s to first usable screen (wallet loaded + initial balance fetch)
- **IPC overhead**: < 1ms per invoke call

---

### Module 3: `bins/gui/src/` (Svelte Frontend)

- **Responsibility**: User interface for all wallet, transaction, producer, rewards, NFT, bridge, and governance operations. Renders views, handles user input, invokes Tauri backend commands, displays results. Manages UI state via Svelte stores.
- **Public interface**: N/A (frontend components, not a library)
- **Dependencies**: Svelte 5, `@tauri-apps/api` (IPC), `@tauri-apps/plugin-dialog` (file picker), `@tauri-apps/plugin-clipboard-manager` (copy), `qrcode` (QR generation for GUI-FR-016)
- **Implementation order**: 3 (after Tauri backend shell exists)

#### Framework Choice: Svelte 5

| Criteria | Svelte 5 | React | Vue 3 |
|----------|----------|-------|-------|
| Bundle size | ~5 KB (compiled) | ~45 KB (runtime) | ~33 KB (runtime) |
| Performance | Compiled, no virtual DOM | VDOM diffing | VDOM with reactivity |
| Learning curve | Low | Medium | Medium |
| Tauri ecosystem | Official template | Official template | Community template |
| Component model | Single-file .svelte | JSX + hooks | SFC |

Svelte 5 is chosen for minimal bundle size (GUI-NF-002 installer target < 15 MB), compiled reactivity (no runtime overhead), and first-class Tauri support. The runes-based reactivity in Svelte 5 aligns well with the command-response pattern of Tauri IPC.

#### Page/View Structure

```
src/
├── App.svelte                    # Root layout: sidebar nav + content area
├── lib/
│   ├── stores/
│   │   ├── wallet.ts             # Wallet state (loaded, addresses, balance)
│   │   ├── network.ts            # Network state (connected, chain info, endpoint)
│   │   ├── producer.ts           # Producer state (status, bonds, rewards)
│   │   └── notifications.ts     # Toast notifications
│   ├── api/
│   │   ├── wallet.ts             # Tauri invoke wrappers for wallet commands
│   │   ├── transactions.ts       # Tauri invoke wrappers for tx commands
│   │   ├── producer.ts           # Tauri invoke wrappers for producer commands
│   │   ├── rewards.ts            # Tauri invoke wrappers for rewards commands
│   │   ├── nft.ts                # Tauri invoke wrappers for NFT/token commands
│   │   ├── bridge.ts             # Tauri invoke wrappers for bridge commands
│   │   ├── governance.ts         # Tauri invoke wrappers for governance commands
│   │   └── network.ts            # Tauri invoke wrappers for network commands
│   ├── components/
│   │   ├── Sidebar.svelte        # Navigation sidebar
│   │   ├── StatusBar.svelte      # Bottom bar: connection status, chain height, network
│   │   ├── ConfirmDialog.svelte  # Reusable confirmation dialog
│   │   ├── LoadingSpinner.svelte # Loading indicator
│   │   ├── AmountInput.svelte    # DOLI amount input with validation
│   │   ├── AddressInput.svelte   # Address input with bech32m validation
│   │   └── TxResult.svelte       # Transaction result display
│   └── utils/
│       ├── format.ts             # Amount formatting, date formatting
│       └── validation.ts         # Frontend input validation (bech32m, amounts)
├── routes/
│   ├── setup/
│   │   ├── Welcome.svelte        # First-run: create or restore wallet
│   │   ├── CreateWallet.svelte   # Wallet creation (seed phrase display)
│   │   └── RestoreWallet.svelte  # Wallet restoration (seed phrase input)
│   ├── wallet/
│   │   ├── Overview.svelte       # Balance overview (GUI-FR-010)
│   │   ├── Send.svelte           # Send DOLI form (GUI-FR-011)
│   │   ├── Receive.svelte        # Receive address + QR code (GUI-FR-015, GUI-FR-016)
│   │   ├── History.svelte        # Transaction history (GUI-FR-014)
│   │   └── Addresses.svelte      # Address list (GUI-FR-004)
│   ├── producer/
│   │   ├── Dashboard.svelte      # Producer status (GUI-FR-021)
│   │   ├── Register.svelte       # Registration form (GUI-FR-020)
│   │   ├── Bonds.svelte          # Bond management (GUI-FR-022, GUI-FR-024, GUI-FR-025)
│   │   ├── Producers.svelte      # Network producer list (GUI-FR-023)
│   │   └── Exit.svelte           # Exit producer (GUI-FR-027)
│   ├── rewards/
│   │   ├── List.svelte           # Claimable rewards (GUI-FR-030)
│   │   ├── Claim.svelte          # Claim interface (GUI-FR-031, GUI-FR-032)
│   │   └── History.svelte        # Claim history (GUI-FR-033)
│   ├── nft/
│   │   ├── Mint.svelte           # NFT minting (GUI-FR-040)
│   │   ├── Transfer.svelte       # NFT transfer (GUI-FR-041)
│   │   └── Info.svelte           # NFT/token info (GUI-FR-042, GUI-FR-044)
│   ├── bridge/
│   │   ├── Lock.svelte           # Bridge lock (GUI-FR-050)
│   │   ├── Claim.svelte          # Bridge claim (GUI-FR-051)
│   │   └── Refund.svelte         # Bridge refund (GUI-FR-052)
│   ├── governance/
│   │   ├── Updates.svelte        # Update check/status (GUI-FR-060, GUI-FR-061)
│   │   ├── Vote.svelte           # Vote on updates (GUI-FR-062)
│   │   └── Maintainers.svelte    # Maintainer list (GUI-FR-064)
│   └── settings/
│       ├── Network.svelte        # Network selector + RPC config (GUI-FR-081, GUI-FR-082)
│       ├── Wallet.svelte         # Wallet info, export, import (GUI-FR-005 to GUI-FR-008)
│       └── About.svelte          # Version info, links
```

#### Navigation Structure

```
┌─────────────────────────────────────────────────────┐
│  DOLI Wallet                            [●] Mainnet │
├───────────┬─────────────────────────────────────────┤
│           │                                         │
│  Wallet   │   [Content Area]                        │
│  ├ Overview │                                        │
│  ├ Send    │                                        │
│  ├ Receive │                                        │
│  ├ History │                                        │
│  └ Addresses                                        │
│           │                                         │
│  Producer │                                         │
│  ├ Dashboard                                        │
│  ├ Register│                                        │
│  ├ Bonds   │                                        │
│  └ Network │                                        │
│           │                                         │
│  Rewards  │                                         │
│  ├ Claimable                                        │
│  └ History │                                        │
│           │                                         │
│  Assets   │                                         │
│  ├ NFTs    │                                        │
│  ├ Tokens  │                                        │
│  └ Bridge  │                                        │
│           │                                         │
│  Governance                                         │
│           │                                         │
│  Settings │                                         │
│           │                                         │
├───────────┴─────────────────────────────────────────┤
│  ● Connected | Height: 1,234,567 | Epoch: 3,429    │
└─────────────────────────────────────────────────────┘
```

#### State Management (Frontend)

Svelte 5 runes (`$state`, `$derived`, `$effect`) manage reactive state:

```typescript
// stores/wallet.ts
export const walletState = $state({
  loaded: false,
  info: null as WalletInfo | null,
  addresses: [] as AddressInfo[],
  balance: null as BalanceResponse | null,
  loading: false,
  error: null as string | null,
});

// stores/network.ts
export const networkState = $state({
  connected: false,
  network: 'mainnet',
  chainInfo: null as ChainInfoResponse | null,
  rpcUrl: '',
  status: 'disconnected' as 'connected' | 'syncing' | 'disconnected',
  pollInterval: null as ReturnType<typeof setInterval> | null,
});
```

#### Polling Strategy

The frontend polls the backend at configurable intervals (default 10s) for:
- Chain info (height, slot) -- updates status bar
- Balance -- updates wallet overview
- Connection status -- updates indicator

Polling starts when the wallet is loaded and an RPC connection is established. Polling pauses when the app loses focus (Tauri window focus events) to conserve resources.

#### Failure Modes

| Failure | Cause | Detection | Recovery | Impact |
|---------|-------|-----------|----------|--------|
| Tauri invoke fails | Backend command error | Promise rejection from invoke | Display error toast, do not navigate away | Single operation fails |
| Rendering error | Svelte component bug | Error boundary in App.svelte | Show fallback UI with retry button | View-level failure |
| Polling timeout | RPC slow/down | 5s timeout on poll | Show stale data with "last updated" timestamp, reduce poll frequency | Data may be stale |
| Clipboard access denied | OS permissions | Clipboard API error | Show "copy failed" with manual selection fallback | Minor UX degradation |

#### Security Considerations

- **No key material in JS**: The frontend never receives, stores, or processes private keys. All signing is invoked via Tauri commands that return only public results.
- **CSP enforcement**: `tauri.conf.json` sets `default-src 'self'`, `script-src 'self'`, `connect-src` limited to configured RPC endpoints.
- **Input validation**: All user inputs are validated in both frontend (immediate feedback) and backend (authoritative check). Frontend validation is UX-only, not security.
- **No eval/innerHTML**: Svelte's compiled output does not use eval. User-generated content is escaped by default.

#### Performance Budget

- **Frontend bundle**: < 200 KB gzipped (Svelte compiles to minimal JS)
- **First paint**: < 500ms after webview loads
- **Interaction response**: < 100ms for UI state updates
- **Memory (webview)**: < 50 MB for typical usage

---

### Module 4: VDF Feature Flag (`crates/core/Cargo.toml` modification)

- **Responsibility**: Make the `vdf` crate an optional dependency of `doli-core`, gated behind a `vdf` feature flag that defaults to ON. This ensures existing builds (CLI, node) are unaffected while allowing the GUI to compile `doli-core` without GMP.
- **Public interface**: No new public API. Existing API preserved when `features = ["vdf"]`.
- **Dependencies**: Modifies `crates/core/Cargo.toml` only
- **Implementation order**: 1 (part of M1, parallel with wallet crate extraction)

#### Changes Required

**`crates/core/Cargo.toml`**:
```toml
[features]
default = ["vdf"]
vdf = ["dep:vdf"]

[dependencies]
vdf = { workspace = true, optional = true }
```

**`crates/core/src/lib.rs`** -- add VDF stub module:
```rust
#[cfg(feature = "vdf")]
pub use vdf;

// When VDF feature is disabled, provide stub types for BlockHeader/Transaction deserialization
#[cfg(not(feature = "vdf"))]
pub mod vdf_stubs {
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct VdfOutput {
        pub value: Vec<u8>,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct VdfProof {
        pub pi: Vec<u8>,
    }

    impl VdfProof {
        pub fn empty() -> Self { Self { pi: Vec::new() } }
    }
}
```

**`crates/core/src/block.rs`** -- conditional imports:
```rust
#[cfg(feature = "vdf")]
use vdf::{VdfOutput, VdfProof};
#[cfg(not(feature = "vdf"))]
use crate::vdf_stubs::{VdfOutput, VdfProof};
```

Similar conditional imports in: `genesis.rs`, `network.rs`, `tpop/presence.rs`, `tpop/heartbeat.rs`, `validation.rs`, `rewards.rs`.

Functions that call `vdf::compute()`, `vdf::verify()`, `vdf::block_input()`, `vdf::registration_input()` get `#[cfg(feature = "vdf")]` guards. Without VDF, these functions are unavailable at compile time.

**`bins/cli/Cargo.toml`** -- explicit VDF feature:
```toml
doli-core = { workspace = true, features = ["vdf"] }
```

**`bins/node/Cargo.toml`** -- explicit VDF feature:
```toml
doli-core = { workspace = true, features = ["vdf"] }
```

**`bins/gui/Cargo.toml`** -- no VDF:
```toml
doli-core = { workspace = true, default-features = false }
# OR: don't depend on doli-core at all, only on crates/wallet
```

#### Failure Modes

| Failure | Cause | Detection | Recovery | Impact |
|---------|-------|-----------|----------|--------|
| Feature flag breaks existing builds | Incomplete cfg gating, missed import | `cargo build` / `cargo test` failure | CI catches immediately. Fix: add missing `#[cfg]` guards | Build blocked until fixed |
| Type mismatch between stub and real VDF | Stub fields don't match real struct | Deserialization test across feature flag | Alignment test: serialize with VDF, deserialize without | Data incompatibility |
| Test failures | Tests use VDF types without feature gate | `cargo test` without VDF feature | Gate tests with `#[cfg(feature = "vdf")]` | Tests blocked |

#### Security Considerations

- **No security impact**: The feature flag only affects whether VDF computation/verification code is compiled. The types remain structurally identical (same serialization).
- **Consensus safety**: The feature flag must NEVER be disabled for node builds. The node must always verify VDF proofs. CI enforces `features = ["vdf"]` for doli-node.

---

### Module 5: CI/CD Pipeline (`.github/workflows/release.yml` extension)

- **Responsibility**: Build GUI installers for Windows (x86_64), macOS (aarch64 + x86_64), and Linux (x86_64) on GitHub Actions tag push. Attach installers to GitHub Release alongside existing CLI/node binaries.
- **Public interface**: N/A (CI pipeline)
- **Dependencies**: Tauri CLI (`cargo install tauri-cli`), Node.js (for Svelte build), platform-specific tools
- **Implementation order**: 6 (M6, after functional app works locally)

#### New CI Jobs

```yaml
# Added jobs (alongside existing build-linux-x64, build-macos-arm64)

build-gui-linux:
  name: Build GUI Linux x86_64
  runs-on: ubuntu-latest
  # Needs: Node.js, Rust, webkit2gtk, libappindicator (Tauri deps)
  # Produces: .AppImage, .deb

build-gui-macos-arm64:
  name: Build GUI macOS aarch64
  runs-on: macos-latest
  # Needs: Node.js, Rust
  # Produces: .dmg (aarch64)

build-gui-macos-x64:
  name: Build GUI macOS x86_64
  runs-on: macos-13  # Intel runner
  # Needs: Node.js, Rust
  # Produces: .dmg (x86_64)

build-gui-windows:
  name: Build GUI Windows x86_64
  runs-on: windows-latest
  # Needs: Node.js, Rust, WebView2 (preinstalled on Win 10+)
  # NO GMP/MSYS2 needed (GUI doesn't depend on VDF)
  # Produces: .msi, .exe (NSIS)

release:
  needs: [build-linux-x64, build-macos-arm64, build-gui-linux, build-gui-macos-arm64, build-gui-macos-x64, build-gui-windows]
  # Collects all artifacts, generates checksums, creates release
```

#### Key CI Design Decisions

1. **GUI Windows build does NOT need GMP/MSYS2** because the GUI depends on `crates/wallet/` (pure Rust + crypto) and not on `doli-core[vdf]`. This eliminates the biggest CI risk from the requirements doc.

2. **Separate jobs for GUI and CLI/node**: GUI builds are independent from CLI/node builds. If GUI build fails, CLI/node release still proceeds. This prevents the GUI from blocking node releases.

3. **macOS universal binary vs separate**: Two separate builds (aarch64 on `macos-latest`, x86_64 on `macos-13`) rather than a universal binary. Tauri's universal binary support is experimental. Separate builds are more reliable.

4. **Tauri build process**:
   ```bash
   # Install frontend deps
   cd bins/gui && npm install
   # Build frontend
   npm run build
   # Build Tauri app (includes Rust backend compilation + bundling)
   cargo tauri build --target $TARGET
   ```

5. **Artifact naming**: `doli-gui-{VERSION}-{platform}.{ext}`
   - `doli-gui-v4.0.0-x86_64-linux.AppImage`
   - `doli-gui-v4.0.0-aarch64-macos.dmg`
   - `doli-gui-v4.0.0-x86_64-macos.dmg`
   - `doli-gui-v4.0.0-x86_64-windows.msi`

6. **Code signing integration points** (GUI-NF-010, Should priority):
   - Windows: `TAURI_PRIVATE_KEY` / `TAURI_KEY_PASSWORD` secrets for Tauri's built-in signing
   - macOS: `APPLE_CERTIFICATE` / `APPLE_CERTIFICATE_PASSWORD` / `APPLE_SIGNING_IDENTITY` secrets
   - Both are optional -- build succeeds without them but produces unsigned installers

#### Failure Modes

| Failure | Cause | Detection | Recovery | Impact |
|---------|-------|-----------|----------|--------|
| GUI build fails | Tauri/Svelte compilation error, missing system deps | GitHub Actions job failure | GUI jobs fail, CLI/node jobs succeed independently, release proceeds without GUI | No GUI installer in release |
| Windows build OOM | Rust compilation memory usage | Job timeout or OOM kill | Increase runner size or use `codegen-units = 4` for CI | Windows installer missing |
| macOS signing fails | Invalid/expired certificate | codesign exit code | Release unsigned installer with warning in notes | SmartScreen/Gatekeeper warnings |
| Artifact upload fails | GitHub API rate limit | upload-artifact failure | Retry job | Installer missing from release |

---

## Failure Modes (system-level)

| Scenario | Affected Modules | Detection | Recovery Strategy | Degraded Behavior |
|----------|-----------------|-----------|-------------------|-------------------|
| No public RPC endpoints available | Tauri backend, frontend | Connection test fails for all default endpoints | Show "disconnected" + prompt to configure custom endpoint or start local node | Wallet management (view/export addresses) works, all chain operations disabled |
| Wallet file corrupted | Wallet crate, Tauri backend | JSON parse failure | Suggest restoring from seed phrase or importing backup | Full app unusable until wallet is restored |
| RPC endpoint returns stale data | Tauri backend, frontend | Chain height does not advance for > 60s | Show "possibly stale" warning, suggest switching endpoints | Displayed data may be outdated |
| Network switch mid-transaction | Tauri backend | Network mismatch between signed tx and endpoint | Abort transaction, clear pending state, notify user | Transaction fails safely (not submitted to wrong network) |
| OS revokes webview permissions | Frontend (Tauri) | Webview fails to initialize | Show native error dialog, suggest reinstall | App cannot start |
| Disk full during wallet save | Wallet crate | `std::fs::write` error | Warn user, do not corrupt existing file (write to temp, then rename) | Unsaved changes lost, existing wallet file intact |

---

## Security Model

### Trust Boundaries

```
┌──────────────────────────────────────────────────────────────┐
│                    DOLI GUI Trust Model                        │
│                                                                │
│  ┌─────────────────────────────────────────────────────┐     │
│  │              UNTRUSTED ZONE                          │     │
│  │  ┌──────────────────────────┐                       │     │
│  │  │    Svelte Frontend       │                       │     │
│  │  │  (WebView / JavaScript)  │                       │     │
│  │  │  - No key material       │                       │     │
│  │  │  - Only public data      │                       │     │
│  │  │  - All inputs validated  │                       │     │
│  │  │    server-side           │                       │     │
│  │  └────────────┬─────────────┘                       │     │
│  │               │ Tauri IPC (invoke)                  │     │
│  └───────────────│─────────────────────────────────────┘     │
│                  │                                            │
│  ┌───────────────┴─────────────────────────────────────┐     │
│  │              TRUSTED ZONE (Rust Process)             │     │
│  │                                                      │     │
│  │  ┌──────────────┐  ┌────────────────────────┐      │     │
│  │  │ Wallet       │  │ Tauri Command Handlers │      │     │
│  │  │ (private     │  │ (input validation,     │      │     │
│  │  │  keys live   │  │  signing orchestration)│      │     │
│  │  │  here ONLY)  │  │                        │      │     │
│  │  └──────────────┘  └──────────┬─────────────┘      │     │
│  │                               │                      │     │
│  │                    HTTP POST (JSON-RPC)              │     │
│  └───────────────────────────────│──────────────────────┘     │
│                                  │                            │
│  ┌───────────────────────────────┴──────────────────────┐    │
│  │              SEMI-TRUSTED ZONE                         │    │
│  │  ┌─────────────────────────────────────────────────┐  │    │
│  │  │    RPC Node (public or local)                    │  │    │
│  │  │  - May return incorrect balance/UTXO data        │  │    │
│  │  │  - Cannot forge transactions (signing is local)  │  │    │
│  │  │  - Cannot steal keys (keys never sent via RPC)   │  │    │
│  │  └─────────────────────────────────────────────────┘  │    │
│  └───────────────────────────────────────────────────────┘    │
└──────────────────────────────────────────────────────────────┘
```

### Data Classification

| Data | Classification | Storage | Access Control |
|------|---------------|---------|---------------|
| Ed25519 private keys | **Secret** | wallet.json (plaintext, file permissions 0600) | Rust backend only, zeroized on drop |
| BLS private keys | **Secret** | wallet.json (plaintext, file permissions 0600) | Rust backend only, zeroized on drop |
| BIP-39 seed phrase | **Secret** | NOT stored anywhere -- shown once during creation | Displayed in frontend during creation ONLY, immediately cleared |
| Ed25519 public keys | Internal | wallet.json, displayed in UI | Backend + Frontend |
| Addresses (bech32m) | Public | wallet.json, displayed in UI, shared for receiving | Backend + Frontend |
| Wallet file path | Internal | AppConfig | Backend only |
| RPC endpoint URL | Internal | AppConfig | Backend + Frontend |
| Transaction data | Public (after submission) | Not persisted locally | Backend + Frontend |
| Balance/UTXO data | Internal | In-memory only (fetched from RPC) | Backend + Frontend |

### Attack Surface

- **Wallet file theft**: Attacker with filesystem access reads wallet.json. **Mitigation**: OS file permissions (0600). Future: password-based encryption (out of scope for v1, matching CLI behavior).
- **RPC MITM**: Attacker intercepts RPC traffic and returns fake balance/UTXO data. **Mitigation**: HTTPS for public endpoints. Cannot steal funds (signing is local). Could trick user into sending to wrong address -- but address is user-provided, not from RPC.
- **Malicious RPC endpoint**: User configures attacker-controlled RPC. **Mitigation**: Same as MITM. Transactions are signed locally. Attacker could refuse to relay or return false data, but cannot steal funds.
- **WebView XSS**: Injected script in webview. **Mitigation**: Strict CSP, no eval, no innerHTML, Svelte's automatic escaping. Even if XSS occurred, attacker cannot invoke Tauri commands with arguments outside the defined API (Tauri 2.x permission system).
- **Supply chain attack**: Compromised npm dependency. **Mitigation**: Lock files, minimal frontend dependencies, audit before release. All security-critical code is in Rust (not JavaScript).

---

## Graceful Degradation

| Dependency | Normal Behavior | Degraded Behavior | User Impact |
|-----------|----------------|-------------------|-------------|
| Public RPC endpoint | Full functionality: balance, send, producer, rewards | Wallet-only: view addresses, export wallet, sign messages locally | "Disconnected" indicator, chain features disabled |
| Primary RPC endpoint | Connected to preferred endpoint | Automatic failover to secondary endpoint | Brief interruption, transparent to user |
| Chain height advancing | Live balance and confirmation updates | "Chain may be stalled" warning after 120s | Data shown may be stale |
| Wallet file | Full read/write operations | Read-only if file permissions prevent write | Warning shown, changes not saved |
| Clipboard API | One-click copy | "Copy failed" with selectable text fallback | Minor UX friction |
| QR code library | QR display for addresses | Address shown as text without QR | Minor UX reduction |

---

## Performance Budgets

| Operation | Latency (p50) | Latency (p99) | Memory | Notes |
|-----------|---------------|---------------|--------|-------|
| App startup (existing wallet) | 1.5s | 3s | 60 MB | GUI-NF-005: < 5s target |
| Wallet creation | 100ms | 200ms | +2 MB | BIP-39 + Ed25519 + BLS keygen |
| Balance fetch | 100ms | 500ms | +1 MB | Single RPC call + deserialize |
| Send transaction | 50ms | 100ms | +1 MB | Sign locally + submit via RPC (network not included) |
| Transaction history (100 items) | 200ms | 800ms | +5 MB | RPC fetch + render |
| Producer list (1000 entries) | 500ms | 2s | +10 MB | Large RPC response |
| Installer size (wallet-only) | - | - | < 15 MB | GUI-NF-002 target |
| RAM steady state | - | - | < 80 MB | GUI-NF-003: < 100 MB target |

---

## Data Flow

### Send Transaction Flow

```
User enters recipient + amount
        │
        ▼
  [Frontend: validation]
  - bech32m address format check
  - amount > 0, valid decimal
  - amount <= displayed balance (UX check, not authoritative)
        │
        ▼
  [Frontend: confirmation dialog]
  - Shows: To, Amount, Fee, Total
  - User clicks "Confirm"
        │
        ▼
  [Tauri invoke: send_doli(to, amount, fee)]
        │
        ▼
  [Rust Backend: send_doli command handler]
  1. Validate inputs (bech32m decode, amount parse)
  2. Fetch UTXOs via RPC (get_utxos)
  3. Select UTXOs (greedy, oldest first)
  4. Build transaction (inputs, outputs, change)
  5. Load private key from wallet (in memory)
  6. Sign transaction (Ed25519)
  7. Zeroize private key
  8. Serialize signed transaction to hex
  9. Submit via RPC (send_transaction)
  10. Return tx hash to frontend
        │
        ▼
  [Frontend: display result]
  - Show tx hash (clickable/copyable)
  - Trigger balance refresh
  - Add to recent activity
```

### Balance Refresh Flow

```
  [Timer tick (10s) OR manual refresh]
        │
        ▼
  [Tauri invoke: get_balance()]
        │
        ▼
  [Rust Backend]
  1. For each address in wallet:
     - Compute pubkey_hash
     - Call RPC get_balance(pubkey_hash)
  2. For primary address:
     - Call RPC get_producers() to check bonded amount
  3. Aggregate: spendable, bonded, immature, unconfirmed, total
  4. Return BalanceResponse
        │
        ▼
  [Frontend: update wallet store]
  - Reactively updates balance display
  - Updates status bar
```

### Producer Registration Flow

```
User enters bond count → Registration form
        │
        ▼
  [Frontend: validation]
  - bond_count >= 1
  - bond_count <= MAX_BONDS_PER_PRODUCER (3000)
  - Wallet has BLS key (check via wallet_info)
  - Estimated cost: bond_count * 10 DOLI + registration fee
        │
        ▼
  [Frontend: confirmation dialog]
  - Shows: Bond count, Total cost, Registration fee
  - BLS public key
  - Warning about activation delay
        │
        ▼
  [Tauri invoke: register_producer(bond_count)]
        │
        ▼
  [Rust Backend: register_producer command handler]
  1. Verify wallet has BLS key
  2. Fetch chain info (current epoch, height)
  3. Compute hash-chain VDF for registration
     (hash_chain_vdf is BLAKE3-based, NO GMP needed)
  4. Fetch UTXOs for bond funding
  5. Build Registration transaction (TxType 1):
     - RegistrationData with VDF output, BLS pubkey, bond_count
     - Inputs: UTXOs covering bond_count * BOND_UNIT + fee
     - Outputs: Bond outputs, change output
  6. Sign transaction
  7. Submit via RPC
  8. Return tx hash
        │
        ▼
  [Frontend: display result]
  - Show tx hash
  - Navigate to Producer Dashboard
  - Show "Pending activation (10 block delay)" status
```

---

## Design Decisions

| Decision | Alternatives Considered | Justification |
|----------|------------------------|---------------|
| **Svelte 5 frontend** | React, Vue 3, Solid | Smallest bundle size (~5 KB compiled vs 45 KB React runtime). Tauri's official template supports Svelte. Compiled reactivity has no runtime overhead. Matches GUI-NF-002 installer size target. |
| **Separate `crates/wallet/` crate** | Keep wallet code in CLI, duplicate in GUI; or add GUI-specific code to CLI crate | Shared crate guarantees wallet format compatibility (GUI-NF-008). Avoids code duplication. Clean dependency: wallet depends only on `crypto`, not `doli-core`/`vdf`. |
| **Wallet crate does NOT depend on `doli-core`** | Depend on `doli-core` with `default-features = false` | `doli-core` has deep VDF integration (BlockHeader, Transaction structs contain VDF types). Feature-flagging all of it is high-risk. The wallet crate only needs: key management, address encoding, transaction byte construction, and RPC. These can be implemented with `crypto` alone (~200 lines of tx builder). |
| **VDF feature flag in `doli-core` (additive)** | No feature flag; only rely on wallet crate isolation | Belt-and-suspenders. The wallet crate avoids `doli-core` entirely, but having the feature flag in `doli-core` enables future tools (explorers, analytics) that need core types without GMP. Low risk since `default = ["vdf"]` preserves all existing behavior. |
| **Transaction building in wallet crate** | Use `doli-core` Transaction type | The wallet crate builds transactions by serializing the canonical byte format directly. This duplicates ~200 lines of serialization logic but eliminates the entire `doli-core` -> `vdf` -> `rug` -> GMP dependency chain. The serialization format is stable (changing it would require a chain reset per CLAUDE.md). Integration tests verify byte-level compatibility. |
| **Separate CI jobs for GUI and CLI/node** | Single job builds everything | Isolation: GUI build failure does not block CLI/node release. Different system dependencies (Tauri needs webkit2gtk on Linux, not RocksDB/GMP). Different runners may be optimal. |
| **Advisory file lock on wallet** | No lock; or mandatory lock | Advisory lock warns if CLI and GUI access the same wallet simultaneously. Mandatory lock would prevent CLI usage while GUI is open -- too restrictive. |
| **Polling for balance updates** | WebSocket subscription, server-sent events | RPC nodes currently support only HTTP POST (no WebSocket subscriptions for state changes). Polling at 10s intervals is simple, predictable, and matches the 10s slot time. Future: add WebSocket RPC support to nodes, then switch to push-based updates. |
| **Config in `~/.doli-gui/`** | Use `~/.doli/` (same as CLI) | Separation prevents GUI config from interfering with CLI/node config. Wallet files can be shared (user chooses path), but app settings are isolated. |

---

## External Dependencies

### Rust (Tauri Backend)

| Crate | Version | Purpose |
|-------|---------|---------|
| `tauri` | 2.x | Desktop app framework |
| `tauri-plugin-dialog` | 2.x | Native file picker dialogs |
| `tauri-plugin-clipboard-manager` | 2.x | Clipboard access |
| `tauri-plugin-shell` | 2.x | Open URLs in browser (Could: embedded node) |
| `tauri-plugin-updater` | 2.x | Auto-update from GitHub Releases (GUI-NF-009) |
| `crypto` | workspace | Ed25519, BLAKE3, BLS, bech32m |
| `bip39` | workspace | Seed phrase generation/validation |
| `serde` / `serde_json` | workspace | Serialization |
| `reqwest` | 0.11 | HTTP client for JSON-RPC |
| `tokio` | workspace | Async runtime |
| `anyhow` | workspace | Error handling |
| `hex` | workspace | Hex encoding |
| `zeroize` | workspace | Secure memory cleanup |

### JavaScript (Svelte Frontend)

| Package | Purpose |
|---------|---------|
| `svelte` | 5.x | UI framework |
| `@tauri-apps/api` | 2.x | Tauri IPC bridge |
| `@tauri-apps/plugin-dialog` | 2.x | File picker frontend bindings |
| `@tauri-apps/plugin-clipboard-manager` | 2.x | Clipboard frontend bindings |
| `vite` | 5.x | Build tool / dev server |
| `qrcode` | ~1.5 | QR code generation (GUI-FR-016) |

### System Dependencies (build-time only)

| Platform | Dependencies | Notes |
|----------|-------------|-------|
| Linux | `webkit2gtk-4.1-dev`, `libappindicator3-dev`, `librsvg2-dev` | Tauri Linux requirements |
| macOS | Xcode CLI tools | WKWebView included in macOS SDK |
| Windows | WebView2 Runtime | Preinstalled on Windows 10+ (1803+). Tauri bundles bootstrapper. |

**Notably absent from Windows build**: GMP, MSYS2, MinGW. The GUI's Rust backend does not compile `rug` or `vdf`.

---

## Directory Structure

```
bins/gui/
├── Cargo.toml                    # Rust crate for Tauri backend
├── tauri.conf.json               # Tauri configuration
├── build.rs                      # Tauri build script
├── icons/                        # App icons (all platforms)
│   ├── icon.ico                  # Windows
│   ├── icon.icns                 # macOS
│   ├── 32x32.png
│   ├── 128x128.png
│   └── icon.png                  # 512x512 or 1024x1024
├── src-tauri/
│   └── src/
│       ├── main.rs               # Tauri app entry point
│       ├── commands/
│       │   ├── mod.rs            # Command module root
│       │   ├── wallet.rs         # Wallet commands
│       │   ├── transactions.rs   # Send/balance/history commands
│       │   ├── producer.rs       # Producer registration/management commands
│       │   ├── rewards.rs        # Rewards commands
│       │   ├── nft.rs            # NFT/token commands
│       │   ├── bridge.rs         # Bridge commands
│       │   ├── governance.rs     # Governance commands
│       │   └── network.rs        # Network/chain commands
│       ├── state.rs              # AppState and AppConfig
│       └── error.rs              # Error types for command responses
├── src/                          # Svelte frontend
│   ├── main.ts                   # Svelte mount point
│   ├── App.svelte                # Root component
│   ├── app.css                   # Global styles
│   ├── lib/                      # Shared code
│   │   ├── stores/               # Svelte stores
│   │   ├── api/                  # Tauri invoke wrappers
│   │   ├── components/           # Reusable components
│   │   └── utils/                # Utilities
│   └── routes/                   # Page components
│       ├── setup/
│       ├── wallet/
│       ├── producer/
│       ├── rewards/
│       ├── nft/
│       ├── bridge/
│       ├── governance/
│       └── settings/
├── package.json                  # Frontend dependencies
├── vite.config.ts                # Vite configuration
├── svelte.config.js              # Svelte configuration
└── tsconfig.json                 # TypeScript configuration

crates/wallet/
├── Cargo.toml
├── src/
│   ├── lib.rs                    # Crate root, re-exports
│   ├── wallet.rs                 # Wallet struct, key management (extracted from CLI)
│   ├── rpc_client.rs             # RPC client (extracted from CLI)
│   ├── tx_builder.rs             # Transaction byte construction (no doli-core dep)
│   └── types.rs                  # Response types (Balance, Utxo, ChainInfo, etc.)
└── tests/
    ├── wallet_compat.rs          # Wallet format compatibility tests (CLI <-> wallet crate)
    └── tx_builder.rs             # Transaction serialization compatibility tests
```

---

## Tauri Configuration (`tauri.conf.json`) Key Settings

```json
{
  "productName": "DOLI Wallet",
  "version": "1.0.0",
  "identifier": "network.doli.wallet",
  "build": {
    "frontendDist": "../src/dist",
    "devUrl": "http://localhost:5173",
    "beforeBuildCommand": "npm run build",
    "beforeDevCommand": "npm run dev"
  },
  "app": {
    "windows": [
      {
        "title": "DOLI Wallet",
        "width": 1024,
        "height": 768,
        "minWidth": 800,
        "minHeight": 600,
        "resizable": true
      }
    ],
    "security": {
      "csp": "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self' https://*.doli.network http://127.0.0.1:*; img-src 'self' data:"
    }
  },
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": [
      "icons/32x32.png",
      "icons/128x128.png",
      "icons/icon.ico",
      "icons/icon.icns"
    ]
  },
  "plugins": {
    "updater": {
      "endpoints": [
        "https://github.com/e-weil/doli/releases/latest/download/latest.json"
      ],
      "dialog": true,
      "pubkey": ""
    }
  }
}
```

---

## Auto-Update Mechanism (GUI-NF-009)

Tauri 2.x includes a built-in updater plugin that checks GitHub Releases for new versions:

1. On startup (and periodically), the app fetches `latest.json` from GitHub Releases
2. If a newer version exists, a dialog prompts the user
3. User approves -> download + replace binary
4. App restarts with new version

The `latest.json` file is generated by Tauri's build process and attached to each GitHub Release. It contains version, platform-specific download URLs, and signature for integrity verification.

**Security**: The updater uses Ed25519 signatures (Tauri's built-in key pair) to verify downloads. The public key is embedded in `tauri.conf.json`. The private key is a GitHub secret used during CI build.

---

## Workspace Changes

### `Cargo.toml` (workspace root)

Add new members:
```toml
[workspace]
members = [
    "crates/*",
    "bins/*",          # Already includes bins/gui via glob
    "testing/benchmarks",
    "testing/integration",
]
```

Since `bins/*` is already a glob pattern, adding `bins/gui/` requires no workspace Cargo.toml change.

Add wallet crate to workspace dependencies:
```toml
[workspace.dependencies]
# ... existing deps ...
wallet = { path = "crates/wallet" }
```

### Migration Path for CLI

After `crates/wallet/` is created, `bins/cli/Cargo.toml` changes to:
```toml
[dependencies]
wallet = { workspace = true }   # Replaces local wallet.rs and rpc_client.rs
doli-core = { workspace = true, features = ["vdf"] }
# ... rest unchanged
```

`bins/cli/src/main.rs` changes:
```rust
// Before:
mod rpc_client;
mod wallet;
use rpc_client::RpcClient;
use wallet::Wallet;

// After:
use wallet::{Wallet, RpcClient, coins_to_units, format_balance};
```

The CLI continues to depend on `doli-core` (with VDF) for producer registration, which needs `RegistrationData` and `Transaction` types. The wallet crate handles everything else.

---

## Requirement Traceability

| Requirement ID | Architecture Section | Module(s) |
|---------------|---------------------|-----------|
| GUI-FR-001 | Module 1: Wallet, Module 2: Tauri Backend (create_wallet cmd) | `crates/wallet/src/wallet.rs`, `bins/gui/src-tauri/src/commands/wallet.rs`, `bins/gui/src/routes/setup/CreateWallet.svelte` |
| GUI-FR-002 | Module 1: Wallet, Module 2: Tauri Backend (restore_wallet cmd) | `crates/wallet/src/wallet.rs`, `bins/gui/src-tauri/src/commands/wallet.rs`, `bins/gui/src/routes/setup/RestoreWallet.svelte` |
| GUI-FR-003 | Module 1: Wallet, Module 2: Tauri Backend (generate_address cmd) | `crates/wallet/src/wallet.rs`, `bins/gui/src-tauri/src/commands/wallet.rs` |
| GUI-FR-004 | Module 2: Tauri Backend (list_addresses cmd), Module 3: Frontend | `bins/gui/src-tauri/src/commands/wallet.rs`, `bins/gui/src/routes/wallet/Addresses.svelte` |
| GUI-FR-005 | Module 1: Wallet (export), Module 2: Tauri Backend | `crates/wallet/src/wallet.rs`, `bins/gui/src-tauri/src/commands/wallet.rs`, `bins/gui/src/routes/settings/Wallet.svelte` |
| GUI-FR-006 | Module 1: Wallet (import), Module 2: Tauri Backend | `crates/wallet/src/wallet.rs`, `bins/gui/src-tauri/src/commands/wallet.rs`, `bins/gui/src/routes/settings/Wallet.svelte` |
| GUI-FR-007 | Module 2: Tauri Backend (wallet_info cmd), Module 3: Frontend | `bins/gui/src-tauri/src/commands/wallet.rs`, `bins/gui/src/routes/settings/Wallet.svelte` |
| GUI-FR-008 | Module 1: Wallet (add_bls_key), Module 2: Tauri Backend | `crates/wallet/src/wallet.rs`, `bins/gui/src-tauri/src/commands/wallet.rs` |
| GUI-FR-010 | Module 1: RPC Client (get_balance), Module 2: Tauri Backend, Module 3: Frontend | `crates/wallet/src/rpc_client.rs`, `bins/gui/src-tauri/src/commands/transactions.rs`, `bins/gui/src/routes/wallet/Overview.svelte` |
| GUI-FR-011 | Module 1: Wallet + TxBuilder + RPC, Module 2: Tauri Backend | `crates/wallet/src/tx_builder.rs`, `bins/gui/src-tauri/src/commands/transactions.rs`, `bins/gui/src/routes/wallet/Send.svelte` |
| GUI-FR-012 | Module 1: TxBuilder (covenant support), Module 2: Tauri Backend | `crates/wallet/src/tx_builder.rs`, `bins/gui/src-tauri/src/commands/transactions.rs` |
| GUI-FR-013 | Module 1: TxBuilder, Module 2: Tauri Backend | `crates/wallet/src/tx_builder.rs`, `bins/gui/src-tauri/src/commands/transactions.rs` |
| GUI-FR-014 | Module 1: RPC Client (get_history), Module 3: Frontend | `crates/wallet/src/rpc_client.rs`, `bins/gui/src/routes/wallet/History.svelte` |
| GUI-FR-015 | Module 3: Frontend (Clipboard plugin) | `bins/gui/src/lib/components/`, `bins/gui/src/routes/wallet/Receive.svelte` |
| GUI-FR-016 | Module 3: Frontend (qrcode lib) | `bins/gui/src/routes/wallet/Receive.svelte` |
| GUI-FR-020 | Module 1: TxBuilder + RPC, Module 2: Tauri Backend | `crates/wallet/src/tx_builder.rs`, `bins/gui/src-tauri/src/commands/producer.rs`, `bins/gui/src/routes/producer/Register.svelte` |
| GUI-FR-021 | Module 1: RPC Client (get_producers), Module 3: Frontend | `crates/wallet/src/rpc_client.rs`, `bins/gui/src/routes/producer/Dashboard.svelte` |
| GUI-FR-022 | Module 1: RPC Client, Module 3: Frontend | `crates/wallet/src/rpc_client.rs`, `bins/gui/src/routes/producer/Bonds.svelte` |
| GUI-FR-023 | Module 1: RPC Client (get_producers), Module 3: Frontend | `crates/wallet/src/rpc_client.rs`, `bins/gui/src/routes/producer/Producers.svelte` |
| GUI-FR-024 | Module 1: TxBuilder + RPC, Module 2: Tauri Backend | `crates/wallet/src/tx_builder.rs`, `bins/gui/src-tauri/src/commands/producer.rs`, `bins/gui/src/routes/producer/Bonds.svelte` |
| GUI-FR-025 | Module 1: TxBuilder + RPC, Module 2: Tauri Backend | `crates/wallet/src/tx_builder.rs`, `bins/gui/src-tauri/src/commands/producer.rs`, `bins/gui/src/routes/producer/Bonds.svelte` |
| GUI-FR-026 | Module 1: RPC Client, Module 2: Tauri Backend | `crates/wallet/src/rpc_client.rs`, `bins/gui/src-tauri/src/commands/producer.rs`, `bins/gui/src/routes/producer/Bonds.svelte` |
| GUI-FR-027 | Module 1: TxBuilder + RPC, Module 2: Tauri Backend | `crates/wallet/src/tx_builder.rs`, `bins/gui/src-tauri/src/commands/producer.rs`, `bins/gui/src/routes/producer/Exit.svelte` |
| GUI-FR-030 | Module 1: RPC Client, Module 3: Frontend | `crates/wallet/src/rpc_client.rs`, `bins/gui/src/routes/rewards/List.svelte` |
| GUI-FR-031 | Module 1: TxBuilder + RPC, Module 2: Tauri Backend | `crates/wallet/src/tx_builder.rs`, `bins/gui/src-tauri/src/commands/rewards.rs`, `bins/gui/src/routes/rewards/Claim.svelte` |
| GUI-FR-032 | Module 1: TxBuilder + RPC, Module 2: Tauri Backend | `crates/wallet/src/tx_builder.rs`, `bins/gui/src-tauri/src/commands/rewards.rs`, `bins/gui/src/routes/rewards/Claim.svelte` |
| GUI-FR-033 | Module 1: RPC Client, Module 3: Frontend | `crates/wallet/src/rpc_client.rs`, `bins/gui/src/routes/rewards/History.svelte` |
| GUI-FR-034 | Module 1: RPC Client, Module 3: Frontend | `crates/wallet/src/rpc_client.rs`, `bins/gui/src/routes/rewards/List.svelte` |
| GUI-FR-040 | Module 1: TxBuilder + RPC, Module 2: Tauri Backend, Module 3: Frontend | `crates/wallet/src/tx_builder.rs`, `bins/gui/src-tauri/src/commands/nft.rs`, `bins/gui/src/routes/nft/Mint.svelte` |
| GUI-FR-041 | Module 1: TxBuilder + RPC, Module 2: Tauri Backend | `crates/wallet/src/tx_builder.rs`, `bins/gui/src-tauri/src/commands/nft.rs`, `bins/gui/src/routes/nft/Transfer.svelte` |
| GUI-FR-042 | Module 1: RPC Client, Module 3: Frontend | `crates/wallet/src/rpc_client.rs`, `bins/gui/src/routes/nft/Info.svelte` |
| GUI-FR-043 | Module 1: TxBuilder + RPC, Module 2: Tauri Backend | `crates/wallet/src/tx_builder.rs`, `bins/gui/src-tauri/src/commands/nft.rs` |
| GUI-FR-044 | Module 1: RPC Client, Module 3: Frontend | `crates/wallet/src/rpc_client.rs`, `bins/gui/src/routes/nft/Info.svelte` |
| GUI-FR-050 | Module 1: TxBuilder + RPC, Module 2: Tauri Backend | `crates/wallet/src/tx_builder.rs`, `bins/gui/src-tauri/src/commands/bridge.rs`, `bins/gui/src/routes/bridge/Lock.svelte` |
| GUI-FR-051 | Module 1: TxBuilder + RPC, Module 2: Tauri Backend | `crates/wallet/src/tx_builder.rs`, `bins/gui/src-tauri/src/commands/bridge.rs`, `bins/gui/src/routes/bridge/Claim.svelte` |
| GUI-FR-052 | Module 1: TxBuilder + RPC, Module 2: Tauri Backend | `crates/wallet/src/tx_builder.rs`, `bins/gui/src-tauri/src/commands/bridge.rs`, `bins/gui/src/routes/bridge/Refund.svelte` |
| GUI-FR-060 | Module 1: RPC Client, Module 3: Frontend | `crates/wallet/src/rpc_client.rs`, `bins/gui/src/routes/governance/Updates.svelte` |
| GUI-FR-061 | Module 1: RPC Client, Module 3: Frontend | `crates/wallet/src/rpc_client.rs`, `bins/gui/src/routes/governance/Updates.svelte` |
| GUI-FR-062 | Module 1: TxBuilder + RPC, Module 2: Tauri Backend | `crates/wallet/src/tx_builder.rs`, `bins/gui/src-tauri/src/commands/governance.rs`, `bins/gui/src/routes/governance/Vote.svelte` |
| GUI-FR-063 | Module 1: RPC Client, Module 3: Frontend | `crates/wallet/src/rpc_client.rs`, `bins/gui/src/routes/governance/Vote.svelte` |
| GUI-FR-064 | Module 1: RPC Client, Module 3: Frontend | `crates/wallet/src/rpc_client.rs`, `bins/gui/src/routes/governance/Maintainers.svelte` |
| GUI-FR-070 | Module 1: RPC Client (get_chain_info), Module 3: Frontend StatusBar | `crates/wallet/src/rpc_client.rs`, `bins/gui/src/lib/components/StatusBar.svelte` |
| GUI-FR-071 | Module 1: RPC Client (verify_chain_integrity), Module 3: Frontend | `crates/wallet/src/rpc_client.rs`, `bins/gui/src/routes/settings/` |
| GUI-FR-080 | Module 2: Tauri Backend (default endpoints), Module 2: State (AppConfig) | `bins/gui/src-tauri/src/state.rs` |
| GUI-FR-081 | Module 2: Tauri Backend (set_rpc_endpoint cmd), Module 3: Frontend | `bins/gui/src-tauri/src/commands/network.rs`, `bins/gui/src/routes/settings/Network.svelte` |
| GUI-FR-082 | Module 2: Tauri Backend (set_network cmd), Module 3: Frontend | `bins/gui/src-tauri/src/commands/network.rs`, `bins/gui/src/routes/settings/Network.svelte` |
| GUI-FR-083 | Module 2: Tauri Backend (get_connection_status cmd), Module 3: Frontend StatusBar | `bins/gui/src-tauri/src/commands/network.rs`, `bins/gui/src/lib/components/StatusBar.svelte` |
| GUI-FR-090 | Module 2: Tauri Backend (embedded node process manager) | `bins/gui/src-tauri/src/commands/node.rs` (Could priority, M7) |
| GUI-FR-091 | Module 2: Tauri Backend + Frontend | `bins/gui/src-tauri/src/commands/node.rs`, frontend sync view (Could, M7) |
| GUI-FR-092 | Module 2: Tauri Backend (auto-connect logic) | `bins/gui/src-tauri/src/commands/node.rs` (Could, M7) |
| GUI-FR-100 | Module 1: Wallet (sign_message), Module 2: Tauri Backend | `crates/wallet/src/wallet.rs`, `bins/gui/src-tauri/src/commands/wallet.rs` (Could, M7) |
| GUI-FR-101 | Module 1: Wallet (verify_message), Module 2: Tauri Backend | `crates/wallet/src/wallet.rs`, `bins/gui/src-tauri/src/commands/wallet.rs` (Could, M7) |
| GUI-FR-110 | Module 1: TxBuilder (DelegateBond TxType 13), Module 2: Tauri Backend | `crates/wallet/src/tx_builder.rs`, `bins/gui/src-tauri/src/commands/producer.rs` (Could, M7) |
| GUI-FR-111 | Module 1: TxBuilder (RevokeDelegation TxType 14), Module 2: Tauri Backend | `crates/wallet/src/tx_builder.rs`, `bins/gui/src-tauri/src/commands/producer.rs` (Could, M7) |
| GUI-NF-001 | Module 5: CI/CD (multi-platform builds) | `.github/workflows/release.yml`, `bins/gui/tauri.conf.json` |
| GUI-NF-002 | Module 3: Frontend (Svelte bundle size), Module 5: CI | `bins/gui/src/`, `bins/gui/tauri.conf.json` |
| GUI-NF-003 | Module 2: Tauri Backend (memory budget), Module 3: Frontend | All modules, verified by profiling |
| GUI-NF-004 | Module 1: Wallet (signing in Rust), Module 2: Tauri Backend (no key in IPC), Security Model | `crates/wallet/src/wallet.rs`, `bins/gui/src-tauri/src/commands/` |
| GUI-NF-005 | Module 2: Tauri Backend (startup sequence), Performance Budgets | `bins/gui/src-tauri/src/main.rs` |
| GUI-NF-006 | Overview, all modules | `bins/gui/` (Tauri 2.x), `bins/gui/src/` (Svelte) |
| GUI-NF-007 | Module 5: CI/CD Pipeline | `.github/workflows/release.yml` |
| GUI-NF-008 | Module 1: Wallet (shared crate, identical format) | `crates/wallet/src/wallet.rs`, `crates/wallet/tests/wallet_compat.rs` |
| GUI-NF-009 | Auto-Update Mechanism section | `bins/gui/tauri.conf.json` (updater plugin config) |
| GUI-NF-010 | Module 5: CI/CD (signing integration points) | `.github/workflows/release.yml` (signing secrets) |
| GUI-NF-011 | Module 3: Frontend (loading states, error handling) | `bins/gui/src/lib/components/LoadingSpinner.svelte`, all route components |
| GUI-NF-012 | Module 3: Frontend (accessibility) | All `.svelte` components (aria labels, keyboard nav, contrast) |
| GUI-NF-013 | Module 4: VDF Feature Flag | `crates/core/Cargo.toml`, `crates/core/src/lib.rs`, `bins/gui/Cargo.toml` |
| GUI-NF-014 | Module 3: Frontend (i18n-ready string externalization) | `bins/gui/src/lib/i18n/` (Could, M7) |
