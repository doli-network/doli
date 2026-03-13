# Requirements: DOLI GUI Desktop Application

## Scope

This requirements document covers the DOLI GUI Desktop Application -- a cross-platform desktop wallet built with Tauri 2.x that provides a graphical interface to all DOLI CLI functionality. It connects to DOLI nodes via the existing JSON-RPC API, manages wallet files locally, and optionally bundles `doli-node` for users who want to produce blocks.

### Domains/Modules Affected
- **New crate**: `bins/gui/` -- Tauri desktop application (Rust backend + web frontend)
- **Modified crate**: `crates/core/Cargo.toml` -- feature flag to gate VDF dependency
- **Modified crate**: `crates/vdf/Cargo.toml` -- optional dependency behind feature flag
- **Modified crate**: `bins/cli/Cargo.toml` -- may need to enable VDF feature explicitly
- **Modified file**: `.github/workflows/release.yml` -- add Windows + macOS x86_64 targets, GUI build steps
- **Reused code**: `bins/cli/src/wallet.rs` (wallet format), `bins/cli/src/rpc_client.rs` (JSON-RPC client)
- **Reused crate**: `crates/crypto/` (Ed25519, BLAKE3, BLS, address encoding)

## Summary (plain language)

Build a desktop application for Windows and macOS that lets anyone -- including people who have never used a command line -- manage DOLI coins. The app provides a visual interface to create wallets, send/receive DOLI, register as a block producer, manage bonds, claim rewards, mint NFTs, use the bridge, and participate in governance. It downloads from GitHub Releases as a standard installer (MSI for Windows, DMG for macOS, AppImage for Linux). Advanced users can optionally run a full node from within the app.

## Stakeholders

| Stakeholder | Interest |
|-------------|----------|
| **Token holders (primary)** | Send/receive DOLI, check balances, manage coins without CLI |
| **Producers/stakers** | Register, add bonds, claim rewards, monitor production via dashboard |
| **NFT/token creators** | Mint NFTs, issue tokens, transfer assets through visual interface |
| **Bridge users** | Cross-chain atomic swaps without command-line knowledge |
| **Maintainers** | Governance voting, release signing, protocol activation |
| **Project team** | Grow user base beyond technical users, fulfill Q2 2026 roadmap milestone |

## User Stories

- As a token holder, I want to install a desktop app and create a wallet so that I can receive DOLI without learning CLI commands.
- As a token holder, I want to see my balance and transaction history so that I can track my funds.
- As a token holder, I want to send DOLI to an address so that I can make payments.
- As a producer, I want to register and add bonds through the GUI so that I can participate in block production.
- As a producer, I want to see a dashboard with my scheduled slots, blocks produced, and rewards so that I can monitor my performance.
- As a producer, I want to run an embedded node from within the app so that I don't need to manage systemd services.
- As an NFT creator, I want to mint and transfer NFTs through a visual form so that I can manage digital assets.
- As a bridge user, I want to lock, claim, and refund bridge HTLCs through guided steps so that I can perform cross-chain swaps safely.
- As a maintainer, I want to vote on updates and sign releases from the GUI so that governance is accessible.
- As a non-technical user on Windows, I want to download an MSI installer from GitHub so that I can use DOLI without building from source.

## Requirements

### Functional Requirements -- Wallet Management

| ID | Requirement | Priority | Acceptance Criteria | CLI Command(s) |
|----|------------|----------|---------------------|----------------|
| GUI-FR-001 | Create new wallet with BIP-39 seed phrase (24 words) | Must | - [ ] Generates 24-word BIP-39 mnemonic<br>- [ ] Displays seed phrase for backup with numbered words<br>- [ ] Generates Ed25519 + BLS keypair<br>- [ ] Saves wallet.json to configurable path<br>- [ ] Warns user to back up seed phrase<br>- [ ] Does not store seed phrase in wallet file | `new` |
| GUI-FR-002 | Restore wallet from seed phrase | Must | - [ ] Accepts 24-word input<br>- [ ] Validates mnemonic before restoring<br>- [ ] Derives identical Ed25519 key from phrase<br>- [ ] Generates new BLS keypair<br>- [ ] Creates wallet.json | `restore` |
| GUI-FR-003 | Generate new addresses with optional labels | Must | - [ ] Creates new Ed25519 keypair<br>- [ ] Displays bech32m address (doli1... / tdoli1... / ddoli1...)<br>- [ ] Stores in wallet addresses array<br>- [ ] Label is optional free text | `address` |
| GUI-FR-004 | List all wallet addresses | Must | - [ ] Shows all addresses with labels<br>- [ ] Shows bech32m format<br>- [ ] Highlights primary address<br>- [ ] Copy-to-clipboard button per address | `addresses` |
| GUI-FR-005 | Export wallet file | Should | - [ ] Saves wallet.json to user-chosen path<br>- [ ] File picker dialog<br>- [ ] Success confirmation | `export` |
| GUI-FR-006 | Import wallet file | Should | - [ ] File picker for JSON wallet<br>- [ ] Validates wallet format before importing<br>- [ ] Warns if wallet already exists at target path | `import` |
| GUI-FR-007 | Show wallet info (name, version, address count) | Should | - [ ] Displays wallet name, version (1 or 2), address count<br>- [ ] Shows BLS key presence | `info` |
| GUI-FR-008 | Add BLS attestation key to existing wallet | Should | - [ ] Generates BLS keypair<br>- [ ] Errors if BLS key already exists<br>- [ ] Saves to wallet file<br>- [ ] Shows BLS public key | `add-bls` |

### Functional Requirements -- Balance & Transactions

| ID | Requirement | Priority | Acceptance Criteria | CLI Command(s) |
|----|------------|----------|---------------------|----------------|
| GUI-FR-010 | View balance (confirmed, unconfirmed, immature, bonded) | Must | - [ ] Shows spendable balance<br>- [ ] Shows bonded balance (from ProducerSet)<br>- [ ] Shows immature balance (coinbase pending maturity)<br>- [ ] Shows unconfirmed (mempool)<br>- [ ] Shows pending activation bonds<br>- [ ] Per-address and total views<br>- [ ] Amounts in DOLI with 8 decimal places (1 DOLI = 100,000,000 base units) | `balance` |
| GUI-FR-011 | Send DOLI to address | Must | - [ ] Address input with validation (bech32m)<br>- [ ] Amount input with DOLI denomination<br>- [ ] Optional fee override (default: auto)<br>- [ ] Confirmation dialog before signing<br>- [ ] Shows transaction hash on success<br>- [ ] Error messages for insufficient balance | `send` |
| GUI-FR-012 | Send with covenant condition | Could | - [ ] Optional covenant field: multisig, hashlock, htlc, timelock, vesting<br>- [ ] Condition syntax help/builder<br>- [ ] Preview before submission | `send --condition` |
| GUI-FR-013 | Spend covenant-conditioned UTXO | Could | - [ ] UTXO selector (txhash:index)<br>- [ ] Witness data input<br>- [ ] Amount and recipient fields | `spend` |
| GUI-FR-014 | Transaction history view | Must | - [ ] Paginated list of transactions<br>- [ ] Shows: hash, type, amount sent/received, fee, height, confirmations, timestamp<br>- [ ] Configurable limit<br>- [ ] Click to view transaction details | `history` |
| GUI-FR-015 | Copy address to clipboard | Must | - [ ] One-click copy button<br>- [ ] Visual feedback on copy | N/A (UX) |
| GUI-FR-016 | QR code display for receive address | Should | - [ ] Generates QR code from bech32m address<br>- [ ] Displayed in "Receive" view | N/A (UX) |

### Functional Requirements -- Producer Management

| ID | Requirement | Priority | Acceptance Criteria | CLI Command(s) |
|----|------------|----------|---------------------|----------------|
| GUI-FR-020 | Register as block producer | Must | - [ ] Bond count input (1-10000, each bond = 10 DOLI)<br>- [ ] Shows total cost (bonds x 10 DOLI + registration fee)<br>- [ ] Requires BLS key in wallet<br>- [ ] Shows estimated registration fee (BASE_REGISTRATION_FEE = 0.001 DOLI, scales with pending count)<br>- [ ] Confirmation dialog<br>- [ ] Shows tx hash on success | `producer register` |
| GUI-FR-021 | Producer status dashboard | Must | - [ ] Shows: public key, registration height, bond amount, bond count, status (active/pending/exited/slashed)<br>- [ ] Shows presence score, liveness score, attestation count<br>- [ ] Shows qualified-for-rewards status<br>- [ ] Shows last produced block height and slot | `producer status` |
| GUI-FR-022 | View per-bond vesting details | Should | - [ ] Shows each bond with creation slot, current penalty rate, quarters vested<br>- [ ] Shows summary (total bonds, fully vested count, average penalty) | `producer bonds` |
| GUI-FR-023 | List all network producers | Should | - [ ] Paginated list of all producers<br>- [ ] Filter: active-only toggle<br>- [ ] Shows: pubkey, bond amount, status, presence score | `producer list` |
| GUI-FR-024 | Add bonds (bond stacking) | Must | - [ ] Bond count input (1-10000)<br>- [ ] Shows cost (count x 10 DOLI)<br>- [ ] Requires active producer status<br>- [ ] Confirmation dialog | `producer add-bond` |
| GUI-FR-025 | Request bond withdrawal (FIFO, instant with penalty) | Must | - [ ] Bond count input<br>- [ ] Shows estimated penalty (vesting-based, FIFO order)<br>- [ ] Optional destination address<br>- [ ] Confirmation with penalty warning<br>- [ ] Displays returned amount after penalty | `producer request-withdrawal` |
| GUI-FR-026 | Simulate bond withdrawal (dry run) | Should | - [ ] Bond count input<br>- [ ] Shows penalty without submitting transaction<br>- [ ] Shows per-bond breakdown | `producer simulate-withdrawal` |
| GUI-FR-027 | Exit producer set | Should | - [ ] Force-exit option with penalty warning<br>- [ ] Confirmation dialog<br>- [ ] Shows unbonding period (60,480 blocks ~ 7 days) | `producer exit` |
| GUI-FR-028 | Submit slashing evidence | Won't | N/A -- deferred. Slashing is a rare, technical operation. CLI is sufficient for v1. | `producer slash` |

### Functional Requirements -- Rewards

| ID | Requirement | Priority | Acceptance Criteria | CLI Command(s) |
|----|------------|----------|---------------------|----------------|
| GUI-FR-030 | List claimable epoch rewards | Must | - [ ] Shows epochs with estimated reward amounts<br>- [ ] Shows qualification status per epoch<br>- [ ] Amounts in DOLI | `rewards list` |
| GUI-FR-031 | Claim rewards for specific epoch | Must | - [ ] Epoch selector<br>- [ ] Optional recipient address<br>- [ ] Shows estimated reward<br>- [ ] Confirmation dialog | `rewards claim` |
| GUI-FR-032 | Claim all available rewards | Must | - [ ] One-click claim all<br>- [ ] Shows total estimated rewards<br>- [ ] Submits one tx per epoch<br>- [ ] Progress indicator | `rewards claim-all` |
| GUI-FR-033 | Rewards claim history | Should | - [ ] Paginated list of past claims<br>- [ ] Shows epoch, amount, tx hash, timestamp | `rewards history` |
| GUI-FR-034 | Epoch info display | Should | - [ ] Current epoch number<br>- [ ] BLOCKS_PER_REWARD_EPOCH (360)<br>- [ ] Blocks remaining in current epoch<br>- [ ] Epoch start/end heights | `rewards info` |

### Functional Requirements -- NFT & Tokens

| ID | Requirement | Priority | Acceptance Criteria | CLI Command(s) |
|----|------------|----------|---------------------|----------------|
| GUI-FR-040 | Mint NFT | Should | - [ ] Content field (IPFS CID, URL, or hex hash)<br>- [ ] Optional condition<br>- [ ] Optional DOLI value attachment<br>- [ ] Confirmation dialog | `mint` |
| GUI-FR-041 | Transfer NFT | Should | - [ ] UTXO selector for NFT<br>- [ ] Recipient address<br>- [ ] Witness input (default: none)<br>- [ ] Confirmation dialog | `nft-transfer` |
| GUI-FR-042 | View NFT info | Should | - [ ] UTXO reference input<br>- [ ] Shows content hash, owner, value, condition | `nft-info` |
| GUI-FR-043 | Issue fungible token | Should | - [ ] Ticker input (max 16 chars)<br>- [ ] Supply input<br>- [ ] Optional condition<br>- [ ] Confirmation dialog | `issue-token` |
| GUI-FR-044 | View token info | Should | - [ ] UTXO reference input<br>- [ ] Shows ticker, supply, owner, condition | `token-info` |

### Functional Requirements -- Bridge

| ID | Requirement | Priority | Acceptance Criteria | CLI Command(s) |
|----|------------|----------|---------------------|----------------|
| GUI-FR-050 | Bridge lock (create HTLC) | Should | - [ ] Amount input<br>- [ ] Hash or preimage input (auto-computes hashlock from preimage)<br>- [ ] Lock height and expiry height<br>- [ ] Target chain selector (bitcoin, ethereum, monero, litecoin, cardano)<br>- [ ] Recipient address on target chain<br>- [ ] Shows computed hashlock if preimage provided<br>- [ ] Confirmation dialog | `bridge-lock` |
| GUI-FR-051 | Bridge claim | Should | - [ ] UTXO selector<br>- [ ] Preimage input<br>- [ ] Confirmation dialog | `bridge-claim` |
| GUI-FR-052 | Bridge refund | Should | - [ ] UTXO selector<br>- [ ] Confirmation dialog (only after expiry) | `bridge-refund` |

### Functional Requirements -- Governance

| ID | Requirement | Priority | Acceptance Criteria | CLI Command(s) |
|----|------------|----------|---------------------|----------------|
| GUI-FR-060 | Check for available updates | Should | - [ ] Shows available updates<br>- [ ] Version info and status | `update check` |
| GUI-FR-061 | View update status and veto progress | Should | - [ ] Shows pending updates<br>- [ ] Veto progress (count vs 40% threshold)<br>- [ ] Deadline display | `update status` |
| GUI-FR-062 | Vote on pending update (approve/veto) | Should | - [ ] Version selector<br>- [ ] Approve or veto toggle<br>- [ ] Requires producer wallet<br>- [ ] Confirmation dialog | `update vote` |
| GUI-FR-063 | View votes for version | Should | - [ ] Version input<br>- [ ] Shows vote counts | `update votes` |
| GUI-FR-064 | List maintainers | Should | - [ ] Shows current maintainer set | `maintainer list` |

### Functional Requirements -- Chain Information

| ID | Requirement | Priority | Acceptance Criteria | CLI Command(s) |
|----|------------|----------|---------------------|----------------|
| GUI-FR-070 | Chain info display | Must | - [ ] Network name, best hash, height, slot, genesis hash<br>- [ ] Auto-refreshes periodically | `chain` |
| GUI-FR-071 | Chain integrity verification | Could | - [ ] Triggers verifyChainIntegrity RPC<br>- [ ] Shows completeness, scanned blocks, missing ranges, chain commitment | `chain-verify` |

### Functional Requirements -- Network & Connection

| ID | Requirement | Priority | Acceptance Criteria | CLI Command(s) |
|----|------------|----------|---------------------|----------------|
| GUI-FR-080 | Connect to public RPC endpoints by default | Must | - [ ] Pre-configured mainnet public RPC endpoints<br>- [ ] Falls back to secondary endpoints on failure<br>- [ ] No node setup required for basic wallet use | N/A (new) |
| GUI-FR-081 | Custom RPC endpoint configuration | Must | - [ ] Text field for RPC URL<br>- [ ] Connection test button<br>- [ ] Saves to app settings | `--rpc` flag |
| GUI-FR-082 | Network selector (mainnet/testnet/devnet) | Must | - [ ] Dropdown or toggle<br>- [ ] Changes address prefix (doli/tdoli/ddoli)<br>- [ ] Changes default RPC endpoint<br>- [ ] Persists selection | `--network` flag |
| GUI-FR-083 | Connection status indicator | Must | - [ ] Green/yellow/red indicator<br>- [ ] Shows: connected, syncing, disconnected<br>- [ ] Shows current chain height | N/A (UX) |

### Functional Requirements -- Embedded Node

| ID | Requirement | Priority | Acceptance Criteria | CLI Command(s) |
|----|------------|----------|---------------------|----------------|
| GUI-FR-090 | Start/stop embedded doli-node | Could | - [ ] "Start Node" button launches bundled doli-node process<br>- [ ] Node runs in background<br>- [ ] "Stop Node" button sends graceful shutdown<br>- [ ] Node logs visible in app | N/A (new) |
| GUI-FR-091 | Sync progress display | Could | - [ ] Shows sync percentage<br>- [ ] Shows current/target height<br>- [ ] Shows peer count<br>- [ ] Shows estimated time remaining | N/A (new) |
| GUI-FR-092 | Auto-connect to embedded node | Could | - [ ] When embedded node is running, RPC auto-connects to localhost<br>- [ ] Switches back to public endpoint when node stops | N/A (new) |

### Functional Requirements -- Signing & Verification

| ID | Requirement | Priority | Acceptance Criteria | CLI Command(s) |
|----|------------|----------|---------------------|----------------|
| GUI-FR-100 | Sign a message | Could | - [ ] Message input<br>- [ ] Address selector<br>- [ ] Displays hex signature | `sign` |
| GUI-FR-101 | Verify a signature | Could | - [ ] Message, signature, public key inputs<br>- [ ] Shows valid/invalid result | `verify` |

### Functional Requirements -- Delegation

| ID | Requirement | Priority | Acceptance Criteria | CLI Command(s) |
|----|------------|----------|---------------------|----------------|
| GUI-FR-110 | Delegate bonds to a producer | Could | - [ ] Producer public key input<br>- [ ] Bond count input<br>- [ ] Confirmation dialog<br>- [ ] Submits DelegateBond (TxType 13) transaction | N/A (no CLI command exists; protocol supports TxType 13) |
| GUI-FR-111 | Revoke delegation | Could | - [ ] Shows active delegations<br>- [ ] Unbonding period warning (60,480 blocks ~ 7 days)<br>- [ ] Submits RevokeDelegation (TxType 14) transaction | N/A (no CLI command exists; protocol supports TxType 14) |

### Functional Requirements -- Maintenance & Advanced

| ID | Requirement | Priority | Acceptance Criteria | CLI Command(s) |
|----|------------|----------|---------------------|----------------|
| GUI-FR-120 | Release signing (maintainer) | Won't | N/A -- maintainer-only operation, CLI sufficient for v1 | `release sign` |
| GUI-FR-121 | Protocol activation signing | Won't | N/A -- maintainer-only operation requiring 3/5 multisig, CLI sufficient | `protocol sign`, `protocol activate` |
| GUI-FR-122 | Binary upgrade | Won't | N/A -- Tauri has its own auto-update mechanism (see GUI-NF-009) | `upgrade` |
| GUI-FR-123 | Wipe chain data | Won't | N/A -- dangerous operation, CLI sufficient | `wipe` |
| GUI-FR-124 | Update apply/rollback | Won't | N/A -- administrative operations, CLI sufficient | `update apply`, `update rollback` |

### Non-Functional Requirements

| ID | Requirement | Priority | Acceptance Criteria |
|----|------------|----------|---------------------|
| GUI-NF-001 | Platform support: Windows 10+ (x86_64), macOS 12+ (aarch64 + x86_64), Linux (x86_64 AppImage) | Must | - [ ] Installs and runs on all three platforms<br>- [ ] Native look/feel (WebView2 on Windows, WKWebView on macOS) |
| GUI-NF-002 | Installer size under 15 MB (without embedded node) | Should | - [ ] Tauri app bundle under 15 MB<br>- [ ] Measured for each platform |
| GUI-NF-003 | RAM usage under 100 MB during normal wallet operation | Should | - [ ] Measured with 100 addresses and active RPC polling |
| GUI-NF-004 | Private keys never leave the local machine | Must | - [ ] Keys stored in local wallet.json only<br>- [ ] No key material in RPC requests<br>- [ ] Transaction signing happens in Rust backend<br>- [ ] No key material in frontend JavaScript memory longer than signing operation |
| GUI-NF-005 | Startup to usable state within 5 seconds (existing wallet) | Should | - [ ] Measured on standard hardware<br>- [ ] Wallet load + RPC connection + initial balance fetch |
| GUI-NF-006 | Tauri 2.x framework with Rust backend | Must | - [ ] Backend: Rust (Tauri commands)<br>- [ ] Frontend: web technology (Svelte or React)<br>- [ ] IPC via Tauri invoke protocol |
| GUI-NF-007 | GitHub Actions CI builds all platform installers on tag push | Must | - [ ] Tag `v*` triggers build<br>- [ ] Produces: .msi (Windows), .dmg (macOS aarch64 + x86_64), .AppImage (Linux)<br>- [ ] Artifacts attached to GitHub Release<br>- [ ] SHA-256 checksums generated |
| GUI-NF-008 | Wallet file format compatible with CLI | Must | - [ ] Same JSON format as CLI wallet.json<br>- [ ] Wallet created in GUI works in CLI and vice versa<br>- [ ] Same Ed25519 key derivation from BIP-39 seed |
| GUI-NF-009 | Auto-update via Tauri updater | Should | - [ ] Checks GitHub Releases for new versions<br>- [ ] Notifies user of updates<br>- [ ] Downloads and applies update with user consent |
| GUI-NF-010 | Code signing for Windows and macOS | Should | - [ ] Windows: signed with code-signing certificate (no SmartScreen warning)<br>- [ ] macOS: signed + notarized with Apple Developer ID (no Gatekeeper block)<br>- [ ] Graceful fallback without certificates (warning in release notes) |
| GUI-NF-011 | Responsive UI with loading states | Must | - [ ] All RPC calls show loading spinner<br>- [ ] Error states show user-friendly messages<br>- [ ] No UI freeze during network operations |
| GUI-NF-012 | Accessibility baseline | Should | - [ ] Keyboard navigation for all primary actions<br>- [ ] Screen reader compatible labels<br>- [ ] Sufficient color contrast |
| GUI-NF-013 | VDF dependency gated behind feature flag | Must | - [ ] `doli-core` compiles without VDF/GMP on Windows for wallet-only builds<br>- [ ] Feature flag: `default = ["vdf"]` in `doli-core/Cargo.toml`<br>- [ ] GUI crate depends on `doli-core` without `vdf` feature |
| GUI-NF-014 | Internationalization-ready | Could | - [ ] All user-facing strings in separate locale files<br>- [ ] English as default<br>- [ ] Framework supports adding languages later |

## Acceptance Criteria (detailed)

### GUI-FR-001: Create New Wallet
- [ ] Given the app is open and no wallet exists, when the user clicks "Create Wallet", then a 24-word BIP-39 mnemonic is displayed with numbered words
- [ ] Given the seed phrase is shown, when the user confirms they have backed it up, then a wallet.json is saved with Ed25519 + BLS keypairs
- [ ] Given the wallet is created, the seed phrase is NOT stored in the wallet file
- [ ] Given the wallet is created, the primary address is displayed in bech32m format matching the selected network prefix

### GUI-FR-002: Restore Wallet
- [ ] Given the user enters a valid 24-word seed phrase, when they click "Restore", then the same Ed25519 key is derived as the CLI would produce
- [ ] Given the user enters an invalid phrase, when they click "Restore", then an error is shown without creating a file

### GUI-FR-010: View Balance
- [ ] Given a wallet with funds, when the balance view loads, then it shows: spendable, bonded, immature, unconfirmed, and total -- all in DOLI with 8 decimal places
- [ ] Given a wallet with bonds in ProducerSet, then bonded amount is fetched via `getProducers` RPC (bonds are NOT UTXOs after registration)
- [ ] Given multiple addresses in wallet, then per-address breakdown is available

### GUI-FR-011: Send DOLI
- [ ] Given sufficient balance, when the user enters a valid bech32m address and amount, then a Transfer transaction is constructed, signed locally, and submitted via `sendTransaction` RPC
- [ ] Given an invalid address format, when the user clicks Send, then validation error is shown before submission
- [ ] Given insufficient balance, when the user enters an amount exceeding available funds, then error is shown
- [ ] Given a successful send, the transaction hash is displayed and clickable

### GUI-FR-020: Register as Producer
- [ ] Given a wallet with BLS key and sufficient balance (bonds x 10 DOLI + registration fee), when the user registers, then a Registration (TxType 1) transaction is submitted
- [ ] Given no BLS key in wallet, the "Register" button is disabled with explanation
- [ ] Given bond_unit = 1,000,000,000 base units (10 DOLI), the GUI displays "10 DOLI per bond" consistently
- [ ] Given registration fee BASE_REGISTRATION_FEE = 100,000 base units (0.001 DOLI), fee is shown and scales with pending registrations

### GUI-FR-024: Add Bonds
- [ ] Given an active producer, when bonds are added, then an AddBond (TxType 7) transaction is submitted
- [ ] Given MAX_BONDS_PER_PRODUCER = 3,000, the GUI prevents adding bonds beyond this limit
- [ ] Given each bond = 10 DOLI, the cost is displayed clearly before confirmation

### GUI-FR-025: Request Withdrawal
- [ ] Given bonds with varying ages, when withdrawal is requested, then FIFO order is applied (oldest bonds withdrawn first)
- [ ] Given the vesting schedule (75%/50%/25%/0% penalty at Year 1/2/3/4), the penalty is calculated and displayed before confirmation
- [ ] Given VESTING_QUARTER_SLOTS = 3,153,600 (1 year on mainnet), penalty is shown with time-to-full-vest

### GUI-FR-080: Public RPC Endpoints
- [ ] Given a fresh install, the app connects to pre-configured public mainnet RPC endpoints without any user setup
- [ ] Given primary endpoint is unreachable, the app falls back to secondary endpoints
- [ ] Given all public endpoints fail, the user sees a clear "Cannot connect to DOLI network" message with instructions

### GUI-FR-082: Network Selector
- [ ] Given network is "mainnet", addresses use "doli" prefix and RPC defaults to mainnet endpoints
- [ ] Given network is "testnet", addresses use "tdoli" prefix
- [ ] Given network is "devnet", addresses use "ddoli" prefix
- [ ] Given network change, all displayed addresses and balances refresh

### GUI-NF-004: Private Key Security
- [ ] Given the Tauri security model, the frontend webview cannot directly access private keys -- all signing happens via Tauri Rust commands
- [ ] Given a send operation, the private key is loaded in Rust, signs the transaction, and the key reference is dropped before the signed tx is returned to the frontend
- [ ] Given the wallet file on disk, it follows the same JSON format as CLI with private keys in plaintext (matching current CLI behavior)

### GUI-NF-007: CI Pipeline
- [ ] Given a tag push `v*`, GitHub Actions builds the GUI for all three platforms
- [ ] Given the Windows build, GMP is installed via MSYS2/vcpkg in CI (not on user machine)
- [ ] Given the macOS build, both aarch64 and x86_64 targets are produced (universal binary or separate installers)
- [ ] Given successful builds, all artifacts (.msi, .dmg, .AppImage, .tar.gz) are attached to the GitHub Release alongside existing CLI/node binaries

### GUI-NF-013: VDF Feature Flag
- [ ] Given `doli-core` with `default-features = false`, it compiles without `rug`/GMP
- [ ] Given the GUI crate depends on `doli-core` without VDF, it compiles on Windows without MSYS2
- [ ] Given the CLI and node crates enable the VDF feature, their builds are unchanged
- [ ] Given feature-gated code, VDF-related functions (proof creation/verification) are behind `#[cfg(feature = "vdf")]`

## Impact Analysis

### Existing Code Affected

| File/Module | How Affected | Risk |
|-------------|-------------|------|
| `crates/core/Cargo.toml` | Add `vdf` feature flag, make VDF dependency optional | **High** -- must not break any existing builds. All code paths using VDF types need `#[cfg(feature = "vdf")]` guards. |
| `crates/vdf/Cargo.toml` | No change to crate itself, but core's reference becomes optional | **Low** |
| `bins/cli/Cargo.toml` | Add `doli-core = { features = ["vdf"] }` explicitly | **Low** -- restoring current behavior as explicit opt-in |
| `bins/node/Cargo.toml` | Add `doli-core = { features = ["vdf"] }` explicitly | **Low** -- restoring current behavior as explicit opt-in |
| `.github/workflows/release.yml` | Add Windows build job, macOS x86_64 job, GUI build jobs (3 platforms) | **Medium** -- CI changes affect all releases. Must not break existing Linux/macOS CLI builds. |
| `bins/cli/src/wallet.rs` | Code reuse -- may extract to shared crate or depend directly | **Medium** -- if extracted to shared crate, must not break CLI. If duplicated, maintenance burden increases. |
| `bins/cli/src/rpc_client.rs` | Code reuse -- may extract to shared crate | **Medium** -- same as wallet.rs |

### What Breaks If This Changes

| Module/Function | What Happens | Mitigation |
|----------------|-------------|------------|
| `doli-core` without VDF feature | Any code importing VDF types fails to compile | Audit all `use vdf::` imports in core, gate behind feature flag |
| Existing release workflow | Adding jobs may break artifact names or release structure | Test workflow changes on a branch before merging |
| Wallet file format | If GUI changes format, CLI wallets become incompatible | GUI MUST use identical wallet.rs code -- shared crate preferred |

### Regression Risk Areas

| Area | Why It Might Break |
|------|-------------------|
| `cargo build` for existing crates | Feature flag changes can cause compilation failures if VDF types are used in non-optional paths |
| `cargo test` | Tests depending on VDF types need feature gates |
| Release pipeline | New CI jobs may exceed GitHub Actions time limits or hit resource constraints |
| Wallet interop | Any deviation in key derivation or wallet format breaks cross-tool compatibility |

## Traceability Matrix

| Requirement ID | Priority | Test IDs | Architecture Section | Implementation Module |
|---------------|----------|----------|---------------------|---------------------|
| GUI-FR-001 | Must | W-001 to W-012 (wallet::tests::test_fr001_*), WC-001 to WC-003 (wallet_compat) | Module 1: Wallet, Module 2: Tauri Backend (create_wallet cmd) | wallet @ `crates/wallet/src/wallet.rs`, create_wallet @ `bins/gui/src/commands/wallet.rs`, CreateWallet.svelte @ `bins/gui/src-ui/src/routes/setup/CreateWallet.svelte` |
| GUI-FR-002 | Must | W-013 to W-020 (wallet::tests::test_fr002_*) | Module 1: Wallet, Module 2: Tauri Backend (restore_wallet cmd) | wallet @ `crates/wallet/src/wallet.rs`, restore_wallet @ `bins/gui/src/commands/wallet.rs`, RestoreWallet.svelte @ `bins/gui/src-ui/src/routes/setup/RestoreWallet.svelte` |
| GUI-FR-003 | Must | W-021 to W-028 (wallet::tests::test_fr003_*) | Module 1: Wallet, Module 2: Tauri Backend (generate_address cmd) | wallet @ `crates/wallet/src/wallet.rs`, generate_address @ `bins/gui/src/commands/wallet.rs` |
| GUI-FR-004 | Must | W-029 to W-030 (wallet::tests::test_fr004_*) | Module 2: Tauri Backend (list_addresses cmd), Module 3: Frontend | list_addresses @ `bins/gui/src/commands/wallet.rs`, Addresses.svelte @ `bins/gui/src-ui/src/routes/wallet/Addresses.svelte` |
| GUI-FR-005 | Should | W-031 (wallet::tests::test_fr005_*), WC-005 (wallet_compat::test_nf008_export_import_preserves_wallet) | Module 1: Wallet (export), Module 2: Tauri Backend | wallet @ `crates/wallet/src/wallet.rs`, export_wallet @ `bins/gui/src/commands/wallet.rs` |
| GUI-FR-006 | Should | W-032 to W-033 (wallet::tests::test_fr006_*), WC-005 | Module 1: Wallet (import), Module 2: Tauri Backend | wallet @ `crates/wallet/src/wallet.rs`, import_wallet @ `bins/gui/src/commands/wallet.rs` |
| GUI-FR-007 | Should | W-034 (wallet::tests::test_fr007_wallet_info) | Module 2: Tauri Backend (wallet_info cmd), Module 3: Frontend | wallet_info @ `bins/gui/src/commands/wallet.rs`, Wallet.svelte @ `bins/gui/src-ui/src/routes/settings/Wallet.svelte` |
| GUI-FR-008 | Should | W-035 to W-036 (wallet::tests::test_fr008_*) | Module 1: Wallet (add_bls_key), Module 2: Tauri Backend | wallet @ `crates/wallet/src/wallet.rs`, add_bls_key @ `bins/gui/src/commands/wallet.rs` |
| GUI-FR-010 | Must | R-001 to R-002 (rpc_client::tests::test_fr010_*), T-001 to T-012 (types::tests::test_fr010_*) | Module 1: RPC Client (get_balance), Module 2: Tauri Backend, Module 3: Frontend | rpc_client @ `crates/wallet/src/rpc_client.rs`, get_balance @ `bins/gui/src/commands/transaction.rs`, Overview.svelte @ `bins/gui/src-ui/src/routes/wallet/Overview.svelte` |
| GUI-FR-011 | Must | TX-001 to TX-010 (tx_builder::tests::test_fr011_*), R-003 (rpc_client::tests::test_fr011_*), TXI-001 (integration) | Module 1: Wallet + TxBuilder + RPC, Module 2: Tauri Backend | tx_builder @ `crates/wallet/src/tx_builder.rs`, send_doli @ `bins/gui/src/commands/transaction.rs`, Send.svelte @ `bins/gui/src-ui/src/routes/wallet/Send.svelte` |
| GUI-FR-012 | Could | (deferred to M7) | Module 1: TxBuilder (covenant support), Module 2: Tauri Backend (M7) | deferred |
| GUI-FR-013 | Could | (deferred to M7) | Module 1: TxBuilder, Module 2: Tauri Backend (M7) | deferred |
| GUI-FR-014 | Must | R-004 to R-005 (rpc_client::tests::test_fr014_*) | Module 1: RPC Client (get_history), Module 3: Frontend | rpc_client @ `crates/wallet/src/rpc_client.rs`, get_history @ `bins/gui/src/commands/transaction.rs`, History.svelte @ `bins/gui/src-ui/src/routes/wallet/History.svelte` |
| GUI-FR-015 | Must | (frontend-only, no Rust test) | Module 3: Frontend (Clipboard plugin) | Receive.svelte @ `bins/gui/src-ui/src/routes/wallet/Receive.svelte` |
| GUI-FR-016 | Should | (frontend-only, no Rust test) | Module 3: Frontend (qrcode lib) | deferred (QR code lib not yet integrated) |
| GUI-FR-020 | Must | TX-011 to TX-016 (tx_builder::tests::test_fr020_*), TXI-002 to TXI-003 (integration) | Module 1: TxBuilder + RPC, Module 2: Tauri Backend | tx_builder @ `crates/wallet/src/tx_builder.rs`, register_producer @ `bins/gui/src/commands/producer.rs`, Register.svelte @ `bins/gui/src-ui/src/routes/producer/Register.svelte` |
| GUI-FR-021 | Must | R-006 (rpc_client::tests::test_fr021_get_producers) | Module 1: RPC Client (get_producers), Module 3: Frontend | rpc_client @ `crates/wallet/src/rpc_client.rs`, producer_status @ `bins/gui/src/commands/producer.rs`, Dashboard.svelte @ `bins/gui/src-ui/src/routes/producer/Dashboard.svelte` |
| GUI-FR-022 | Should | R-007 (rpc_client::tests::test_fr022_get_bond_details) | Module 1: RPC Client, Module 3: Frontend | rpc_client @ `crates/wallet/src/rpc_client.rs`, Bonds.svelte @ `bins/gui/src-ui/src/routes/producer/Bonds.svelte` |
| GUI-FR-023 | Should | (covered by R-006 get_producers) | Module 1: RPC Client (get_producers), Module 3: Frontend | rpc_client @ `crates/wallet/src/rpc_client.rs`, Producers.svelte @ `bins/gui/src-ui/src/routes/producer/Producers.svelte` |
| GUI-FR-024 | Must | TX-017 to TX-021 (tx_builder::tests::test_fr024_*), TXI-004 (integration) | Module 1: TxBuilder + RPC, Module 2: Tauri Backend | tx_builder @ `crates/wallet/src/tx_builder.rs`, add_bonds @ `bins/gui/src/commands/producer.rs`, Bonds.svelte @ `bins/gui/src-ui/src/routes/producer/Bonds.svelte` |
| GUI-FR-025 | Must | TX-022 to TX-028 (tx_builder::tests::test_fr025_*), R-008 (rpc_client::tests::test_fr025_*), TXI-005 to TXI-006 (integration) | Module 1: TxBuilder + RPC, Module 2: Tauri Backend | tx_builder @ `crates/wallet/src/tx_builder.rs`, request_withdrawal @ `bins/gui/src/commands/producer.rs`, Bonds.svelte @ `bins/gui/src-ui/src/routes/producer/Bonds.svelte` |
| GUI-FR-026 | Should | R-008 (rpc_client::tests::test_fr025_simulate_withdrawal) | Module 1: RPC Client, Module 2: Tauri Backend | simulate_withdrawal @ `bins/gui/src/commands/producer.rs`, Bonds.svelte @ `bins/gui/src-ui/src/routes/producer/Bonds.svelte` |
| GUI-FR-027 | Should | (no Rust backend test -- builds on exit TxBuilder) | Module 1: TxBuilder + RPC, Module 2: Tauri Backend | exit_producer @ `bins/gui/src/commands/producer.rs`, Exit.svelte @ `bins/gui/src-ui/src/routes/producer/Exit.svelte` |
| GUI-FR-028 | Won't | N/A | N/A | N/A |
| GUI-FR-030 | Must | R-009 (rpc_client::tests::test_fr030_get_rewards_list) | Module 1: RPC Client, Module 3: Frontend | rpc_client @ `crates/wallet/src/rpc_client.rs`, list_rewards @ `bins/gui/src/commands/rewards.rs`, List.svelte @ `bins/gui/src-ui/src/routes/rewards/List.svelte` |
| GUI-FR-031 | Must | TX-029 to TX-030 (tx_builder::tests::test_fr031_*) | Module 1: TxBuilder + RPC, Module 2: Tauri Backend | tx_builder @ `crates/wallet/src/tx_builder.rs`, claim_reward @ `bins/gui/src/commands/rewards.rs` |
| GUI-FR-032 | Must | (covered by TX-029 to TX-030, claim_all uses same builder) | Module 1: TxBuilder + RPC, Module 2: Tauri Backend | claim_all_rewards @ `bins/gui/src/commands/rewards.rs` |
| GUI-FR-033 | Should | (covered by R-004 get_history) | Module 1: RPC Client, Module 3: Frontend | rpc_client @ `crates/wallet/src/rpc_client.rs`, History.svelte @ `bins/gui/src-ui/src/routes/rewards/History.svelte` |
| GUI-FR-034 | Should | R-010 (rpc_client::tests::test_fr034_get_epoch_info) | Module 1: RPC Client, Module 3: Frontend | rpc_client @ `crates/wallet/src/rpc_client.rs`, List.svelte @ `bins/gui/src-ui/src/routes/rewards/List.svelte` |
| GUI-FR-040 | Should | (no Rust test -- NFT TxBuilder deferred) | Module 1: TxBuilder + RPC, Module 2: Tauri Backend, Module 3: Frontend | mint_nft @ `bins/gui/src/commands/nft.rs`, Mint.svelte @ `bins/gui/src-ui/src/routes/nft/Mint.svelte` |
| GUI-FR-041 | Should | (no Rust test -- NFT TxBuilder deferred) | Module 1: TxBuilder + RPC, Module 2: Tauri Backend | transfer_nft @ `bins/gui/src/commands/nft.rs`, Transfer.svelte @ `bins/gui/src-ui/src/routes/nft/Transfer.svelte` |
| GUI-FR-042 | Should | (no Rust test -- RPC query only) | Module 1: RPC Client, Module 3: Frontend | nft_info @ `bins/gui/src/commands/nft.rs`, Info.svelte @ `bins/gui/src-ui/src/routes/nft/Info.svelte` |
| GUI-FR-043 | Should | (no Rust test -- token TxBuilder deferred) | Module 1: TxBuilder + RPC, Module 2: Tauri Backend | issue_token @ `bins/gui/src/commands/nft.rs` |
| GUI-FR-044 | Should | (no Rust test -- RPC query only) | Module 1: RPC Client, Module 3: Frontend | token_info @ `bins/gui/src/commands/nft.rs`, Info.svelte @ `bins/gui/src-ui/src/routes/nft/Info.svelte` |
| GUI-FR-050 | Should | (no Rust test -- bridge TxBuilder deferred) | Module 1: TxBuilder + RPC, Module 2: Tauri Backend | bridge_lock @ `bins/gui/src/commands/bridge.rs`, Lock.svelte @ `bins/gui/src-ui/src/routes/bridge/Lock.svelte` |
| GUI-FR-051 | Should | (no Rust test -- bridge TxBuilder deferred) | Module 1: TxBuilder + RPC, Module 2: Tauri Backend | bridge_claim @ `bins/gui/src/commands/bridge.rs`, Claim.svelte @ `bins/gui/src-ui/src/routes/bridge/Claim.svelte` |
| GUI-FR-052 | Should | (no Rust test -- bridge TxBuilder deferred) | Module 1: TxBuilder + RPC, Module 2: Tauri Backend | bridge_refund @ `bins/gui/src/commands/bridge.rs`, Refund.svelte @ `bins/gui/src-ui/src/routes/bridge/Refund.svelte` |
| GUI-FR-060 | Should | (no Rust test -- RPC query only) | Module 1: RPC Client, Module 3: Frontend | check_updates @ `bins/gui/src/commands/governance.rs`, Updates.svelte @ `bins/gui/src-ui/src/routes/governance/Updates.svelte` |
| GUI-FR-061 | Should | (no Rust test -- RPC query only) | Module 1: RPC Client, Module 3: Frontend | update_status @ `bins/gui/src/commands/governance.rs`, Updates.svelte @ `bins/gui/src-ui/src/routes/governance/Updates.svelte` |
| GUI-FR-062 | Should | (no Rust test -- governance TxBuilder deferred) | Module 1: TxBuilder + RPC, Module 2: Tauri Backend | vote_update @ `bins/gui/src/commands/governance.rs`, Vote.svelte @ `bins/gui/src-ui/src/routes/governance/Vote.svelte` |
| GUI-FR-063 | Should | (no Rust test -- RPC query only) | Module 1: RPC Client, Module 3: Frontend | sign_message @ `bins/gui/src/commands/governance.rs` |
| GUI-FR-064 | Should | (no Rust test -- RPC query only) | Module 1: RPC Client, Module 3: Frontend | Maintainers.svelte @ `bins/gui/src-ui/src/routes/governance/Maintainers.svelte` |
| GUI-FR-070 | Must | R-011 (rpc_client::tests::test_fr070_get_chain_info) | Module 1: RPC Client (get_chain_info), Module 3: Frontend StatusBar | rpc_client @ `crates/wallet/src/rpc_client.rs`, get_chain_info @ `bins/gui/src/commands/network.rs`, StatusBar.svelte @ `bins/gui/src-ui/lib/components/StatusBar.svelte` |
| GUI-FR-071 | Could | (deferred to M7) | Module 1: RPC Client (verify_chain_integrity), Module 3: Frontend (M7) | deferred |
| GUI-FR-080 | Must | R-012 to R-015 (rpc_client::tests::test_fr080_*) | Module 2: Tauri Backend (default endpoints), State (AppConfig) | state @ `bins/gui/src/state.rs` |
| GUI-FR-081 | Must | R-016 to R-017 (rpc_client::tests::test_fr081_*) | Module 2: Tauri Backend (set_rpc_endpoint cmd), Module 3: Frontend | set_rpc_endpoint @ `bins/gui/src/commands/network.rs`, Network.svelte @ `bins/gui/src-ui/src/routes/settings/Network.svelte` |
| GUI-FR-082 | Must | R-018 to R-021 (rpc_client::tests::test_fr082_*) | Module 2: Tauri Backend (set_network cmd), Module 3: Frontend | set_network @ `bins/gui/src/commands/network.rs`, Network.svelte @ `bins/gui/src-ui/src/routes/settings/Network.svelte` |
| GUI-FR-083 | Must | R-022 (rpc_client::tests::test_fr083_rpc_client_stores_url) | Module 2: Tauri Backend (get_connection_status cmd), Module 3: Frontend StatusBar | get_connection_status @ `bins/gui/src/commands/network.rs`, StatusBar.svelte @ `bins/gui/src-ui/lib/components/StatusBar.svelte` |
| GUI-FR-090 | Could | (deferred to M7) | Module 2: Tauri Backend (embedded node process manager, M7) | deferred |
| GUI-FR-091 | Could | (deferred to M7) | Module 2: Tauri Backend + Frontend (M7) | deferred |
| GUI-FR-092 | Could | (deferred to M7) | Module 2: Tauri Backend (auto-connect logic, M7) | deferred |
| GUI-FR-100 | Could | W-037 (wallet::tests::test_fr100_sign_message) | Module 1: Wallet (sign_message), Module 2: Tauri Backend (M7) | wallet @ `crates/wallet/src/wallet.rs`, sign_message @ `bins/gui/src/commands/governance.rs` |
| GUI-FR-101 | Could | W-038 to W-040 (wallet::tests::test_fr101_*) | Module 1: Wallet (verify_message), Module 2: Tauri Backend (M7) | wallet @ `crates/wallet/src/wallet.rs`, verify_signature @ `bins/gui/src/commands/governance.rs` |
| GUI-FR-110 | Could | TXI-007 (tx_type_enum_values test covers DelegateBond=13) | Module 1: TxBuilder (DelegateBond TxType 13), Module 2: Tauri Backend (M7) | tx_builder @ `crates/wallet/src/tx_builder.rs` (type defined, UI deferred) |
| GUI-FR-111 | Could | TXI-007 (tx_type_enum_values test covers RevokeDelegation=14) | Module 1: TxBuilder (RevokeDelegation TxType 14), Module 2: Tauri Backend (M7) | tx_builder @ `crates/wallet/src/tx_builder.rs` (type defined, UI deferred) |
| GUI-FR-120 | Won't | N/A | N/A | N/A |
| GUI-FR-121 | Won't | N/A | N/A | N/A |
| GUI-FR-122 | Won't | N/A | N/A | N/A |
| GUI-FR-123 | Won't | N/A | N/A | N/A |
| GUI-FR-124 | Won't | N/A | N/A | N/A |
| GUI-NF-001 | Must | (CI/CD test -- not unit testable) | Module 5: CI/CD (multi-platform builds) | deferred (M6) |
| GUI-NF-002 | Should | (CI/CD measurement -- not unit testable) | Module 3: Frontend (Svelte bundle size), Module 5: CI | frontend @ `bins/gui/src-ui/` (Svelte 5, minimal bundle) |
| GUI-NF-003 | Should | (runtime measurement -- not unit testable) | Module 2: Tauri Backend (memory budget), Module 3: Frontend | state @ `bins/gui/src/state.rs` (tokio::sync::RwLock for async state) |
| GUI-NF-004 | Must | W-041 to W-043 (wallet::tests::test_nf004_*) | Module 1: Wallet (signing in Rust), Module 2: Tauri Backend, Security Model | wallet @ `crates/wallet/src/wallet.rs`, commands/wallet.rs @ `bins/gui/src/commands/wallet.rs` (build_wallet_info strips keys) |
| GUI-NF-005 | Should | (runtime measurement -- not unit testable) | Module 2: Tauri Backend (startup sequence), Performance Budgets | main @ `bins/gui/src/main.rs` |
| GUI-NF-006 | Must | (structural -- verified by project compilation) | Overview (Tauri 2.x + Svelte 5) | bins/gui/ (Tauri 2.x), bins/gui/src-ui/ (Svelte 5) |
| GUI-NF-007 | Must | (CI/CD test -- not unit testable) | Module 5: CI/CD Pipeline | deferred (M6) |
| GUI-NF-008 | Must | W-044 to W-049 (wallet::tests::test_nf008_*), WC-001 to WC-008 (wallet_compat::*) | Module 1: Wallet (shared crate, identical format) | wallet @ `crates/wallet/src/wallet.rs` (shared between CLI and GUI) |
| GUI-NF-009 | Should | (Tauri updater config -- not unit testable) | Auto-Update Mechanism section | deferred (M6) |
| GUI-NF-010 | Should | (CI/CD -- not unit testable) | Module 5: CI/CD (signing integration points) | deferred (M6) |
| GUI-NF-011 | Must | (frontend-only, no Rust test) | Module 3: Frontend (loading states, error handling) | LoadingSpinner.svelte @ `bins/gui/src-ui/lib/components/LoadingSpinner.svelte`, all route components |
| GUI-NF-012 | Should | (frontend-only, no Rust test) | Module 3: Frontend (accessibility: aria labels, keyboard nav) | all .svelte components (aria-label, role attributes throughout) |
| GUI-NF-013 | Must | TXI-008 (test_nf013_wallet_crate_no_vdf_dependency) | Module 4: VDF Feature Flag | wallet @ `crates/wallet/Cargo.toml` (no doli-core dependency) |
| GUI-NF-014 | Could | (deferred to M7) | Module 3: Frontend (i18n-ready string externalization, M7) | deferred |

## Specs Drift Detected

| Spec File | What's Outdated |
|-----------|----------------|
| `specs/protocol.md` line 507 | States "TxType 9 (ClaimWithdrawal) is reserved but unused -- withdrawal is instant." This matches the code (no ClaimWithdrawal CLI command), but the idea brief mentions "Claim Withdrawal" as a feature. The GUI should NOT implement ClaimWithdrawal -- it does not exist in the protocol. Withdrawal is instant via RequestWithdrawal. |
| `docs/protocol.md` line 261 | Lists `ClaimWithdrawal = 8` but the actual TxType is 9 in `crates/core/src/transaction.rs`. The docs have the wrong enum value. |
| `docs/roadmap.md` | Lists "Bond stacking (1-100 bonds per producer)" but `MAX_BONDS_PER_PRODUCER = 3_000` in consensus.rs. The roadmap is outdated. |
| Idea brief | States "bond minimum 10 DOLI" and "minimum 10 DOLI per bond" -- this is correct. `BOND_UNIT = 1_000_000_000 base units = 10 DOLI`. However, the idea brief also mentions "Claim Withdrawal" which does not exist -- withdrawal is instant. |
| Idea brief | States delegation as a feature, but no CLI commands exist for DelegateBond/RevokeDelegation. Protocol supports TxType 13 and 14, validation exists, but no user interface. The GUI would be the FIRST user interface for delegation. |

## Assumptions

| # | Assumption (technical) | Explanation (plain language) | Confirmed |
|---|----------------------|---------------------------|-----------|
| 1 | `doli-core` can be compiled without VDF by gating `use vdf::` behind `#[cfg(feature = "vdf")]` | The wallet/GUI only needs types, serialization, and crypto -- not VDF computation. We assume the VDF feature can be cleanly separated. | No -- needs audit of all `use vdf::` in core |
| 2 | Public RPC endpoints will exist for mainnet before GUI release | The GUI defaults to public endpoints. If none exist, every user must run their own node. | No -- need confirmation from project team |
| 3 | Wallet file format will not change between now and GUI release | The GUI reuses the same wallet.json format. Any format change would require both CLI and GUI updates. | No |
| 4 | Tauri 2.x supports WebView2 (Windows) and WKWebView (macOS) with consistent rendering | Tauri documentation claims this, but rendering differences may exist. | Yes -- Tauri 2.x docs confirm |
| 5 | GitHub Actions Windows runners have sufficient resources to compile rug/GMP via MSYS2 | GMP cross-compilation on Windows CI is notoriously fragile. | No -- needs CI spike |
| 6 | Code signing certificates will be available by v1.1 (not blocking v1.0) | v1.0 ships unsigned. Windows SmartScreen and macOS Gatekeeper will show warnings. | No |
| 7 | The existing RPC API (31 methods) is sufficient for all GUI features | No new RPC methods are needed. The GUI builds transactions locally and submits via sendTransaction. | Yes -- verified against CLI command implementations |
| 8 | `bins/cli/src/wallet.rs` and `rpc_client.rs` will be extracted to a shared crate (e.g., `crates/wallet/`) | This avoids code duplication between CLI and GUI. | No -- architectural decision for architect agent |
| 9 | Delegation (TxType 13/14) is stable enough for GUI exposure | Protocol-level support exists with validation, but no user-facing interface has been tested. | No -- needs confirmation |
| 10 | 1 DOLI = 100,000,000 base units (8 decimal places), matching `units_to_coins()` in rpc_client.rs | All GUI amount displays use this conversion. | Yes -- confirmed in code |
| 11 | BOND_UNIT = 1,000,000,000 base units = 10 DOLI per bond | Registration and add-bond operations use this constant. | Yes -- confirmed in `consensus.rs:251` |

## Identified Risks

| Risk | Severity | Mitigation |
|------|----------|------------|
| **GMP/VDF feature flag breaks existing builds** | High | Spike the feature flag change on a branch first. Run full CI. Ensure `default = ["vdf"]` so existing crates get VDF by default. Only the GUI crate opts out. |
| **Windows CI compilation failure (GMP via MSYS2)** | High | Test Windows CI job independently before integrating into release workflow. Consider vcpkg as alternative to MSYS2. The embedded node build (which needs VDF/GMP) is Could priority -- wallet-only build avoids GMP entirely via feature flag. |
| **Code signing cost blocks non-technical adoption** | High | v1.0 ships unsigned with warning. Budget ~$300-600/year for certificates. Document manual bypass steps for SmartScreen/Gatekeeper in release notes. |
| **No public RPC endpoints** | High | Must deploy at least 2 public mainnet RPC endpoints before GUI release. Without them, the GUI is useless for non-node-runners. |
| **Wallet format divergence** | Medium | Extract wallet.rs to shared crate. Integration tests that create wallet in CLI and open in GUI (and vice versa). |
| **Tauri webview rendering differences** | Medium | Test on all platforms during development. Use CSS normalization. Avoid platform-specific web APIs. |
| **Embedded node UX complexity** | Medium | Embedded node is Could priority. Basic wallet functionality works without it via public RPC. |
| **Delegation feature untested in user flows** | Medium | Delegation (GUI-FR-110, GUI-FR-111) is Could priority. If protocol-level delegation is not battle-tested, defer to v1.1. |
| **Large scope risks timeline** | Medium | MoSCoW prioritization ensures Must items ship first. Should and Could items can be deferred to v1.1 without blocking release. |

## Out of Scope (Won't)

| Feature | Reason |
|---------|--------|
| GUI-FR-028: Slashing evidence submission | Rare technical operation. Slashing evidence is submitted by automated detection on nodes. CLI is sufficient. |
| GUI-FR-120: Release signing | Maintainer-only workflow. Requires producer key files and specific signing ceremony. CLI is the right tool. |
| GUI-FR-121: Protocol activation | Requires 3/5 maintainer multisig coordination. CLI + JSON file exchange is the correct workflow. |
| GUI-FR-122: Binary upgrade | Tauri has its own update mechanism (GUI-NF-009). The CLI upgrade command manages systemd services -- irrelevant for GUI. |
| GUI-FR-123: Wipe chain data | Destructive operation that should require CLI intentionality. Not appropriate for GUI. |
| GUI-FR-124: Update apply/rollback | Administrative node management. Embedded node (Could priority) would handle this internally. |
| Mobile wallet | Phase 2 per roadmap (Q3 2026). Separate project. |
| Light client / SPV mode | Phase 4+ per roadmap. Requires protocol specification work. |
| Web wallet / browser extension | Not planned. Desktop-first strategy. |
| Hardware wallet integration (Ledger/Trezor) | Phase 4+ per roadmap (Q4 2026). Requires device SDKs and custom app development. |

## Priority Summary

| Priority | Count | Examples |
|----------|-------|---------|
| **Must** | 21 (13 FR + 8 NF) | Wallet create/restore, balance, send, producer register/add-bond/withdrawal, rewards claim, chain info, network connection, platform support, key security, CI pipeline, VDF feature flag |
| **Should** | 24 (17 FR + 7 NF) | Wallet export/import, QR code, bond details, producer list/exit/simulate, rewards history, NFTs, bridge, governance, code signing, auto-update, installer size, accessibility |
| **Could** | 12 (12 FR + 1 NF) | Covenants, embedded node, sign/verify messages, delegation, chain verify, i18n |
| **Won't** | 5 FR | Slashing, release signing, protocol activation, upgrade, wipe |

## Key Constants Reference (from codebase)

| Constant | Value | Source |
|----------|-------|--------|
| BOND_UNIT | 1,000,000,000 base units (10 DOLI) | `consensus.rs:251` |
| MAX_BONDS_PER_PRODUCER | 3,000 | `consensus.rs:258` |
| INITIAL_REWARD | 100,000,000 base units (1 DOLI/block) | `consensus.rs:224` |
| BLOCKS_PER_REWARD_EPOCH | 360 | `consensus.rs:129` |
| COINBASE_MATURITY | 6 blocks | `consensus.rs:238` |
| UNBONDING_PERIOD | 60,480 blocks (~7 days) | `consensus.rs:278` |
| VESTING_QUARTER_SLOTS | 3,153,600 (1 year mainnet) | `consensus.rs:267` |
| BASE_REGISTRATION_FEE | 100,000 base units (0.001 DOLI) | `consensus.rs:1007` |
| MAX_REGISTRATION_FEE | 1,000,000 base units (0.01 DOLI) | `consensus.rs:1016` |
| 1 DOLI | 100,000,000 base units | `rpc_client.rs:617` |
| Mainnet RPC default | `http://127.0.0.1:8500` | `main.rs:780` |
| Testnet RPC default | `http://127.0.0.1:18500` | `main.rs:778` |
| Devnet RPC default | `http://127.0.0.1:28500` | `main.rs:779` |
| Address prefixes | doli (mainnet), tdoli (testnet), ddoli (devnet) | `main.rs:768-773` |