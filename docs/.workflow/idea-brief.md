# Idea Brief: DOLI GUI Wallet — Cross-Platform Desktop Application

## One-Line Summary
A graphical wallet application for Windows and macOS that lets non-technical users manage DOLI coins, view balances, send/receive transactions, and interact with the DOLI network — without touching a command line. Built and released automatically from GitHub tags.

## Problem Statement
DOLI currently requires command-line proficiency. All user interaction goes through the `doli` CLI binary or raw JSON-RPC calls. This excludes the vast majority of potential users who expect installable desktop software with a visual interface. Additionally, DOLI has zero Windows support today — the entire build and release pipeline targets only Linux (x86_64) and macOS (aarch64).

## Current State
- **Two binaries**: `doli-node` (full node) and `doli` (CLI wallet)
- **Release pipeline** (`.github/workflows/release.yml`): builds for `x86_64-unknown-linux-gnu` and `aarch64-apple-darwin` only — no Windows target
- **Nix dev shell** (`flake.nix`): Linux and macOS only (with Darwin-specific iconv handling)
- **Existing platform stubs**: some `#[cfg(unix)]` / `#[cfg(not(unix))]` / `#[cfg(windows)]` stubs exist in the codebase, indicating Windows was considered but never completed
- **CLI is a thin RPC client**: the `doli` CLI communicates with nodes entirely via JSON-RPC over HTTP. Wallet operations (key generation, signing) happen locally. All chain queries and transaction submission go through RPC. **The GUI does NOT need to embed a full node.**
- **Wallet format**: simple JSON file (`~/.doli/wallet.json`) containing Ed25519 keys, BLS keys, and address labels
- **Roadmap** (`docs/roadmap.md`): "GUI wallet application" listed in Phase 4 (Post-Mainnet), targeted for Q2 2026

### CLI Feature Surface (what the GUI would need to expose)
~40+ commands across: Wallet, Transactions, Producer, Rewards, NFT/Tokens, Bridge, Chain, Governance, Maintenance.

## Proposed Solution
Build a cross-platform desktop GUI wallet using **Tauri 2.x** (Rust backend + web frontend) that connects to DOLI nodes via the existing JSON-RPC API. Package as native installers for Windows (MSI/NSIS) and macOS (DMG/.app). **Release automatically from GitHub tags** via the existing CI pipeline (extended with Windows + GUI targets).

### Why Tauri over Alternatives
| Option | Binary Size | RAM | Windows/Mac | Rust Integration | Security |
|--------|-------------|-----|-------------|------------------|----------|
| **Tauri 2.x** | ~3-10 MB | ~30-40 MB | Native installers | Native (IS Rust) | Isolated webview |
| Electron | ~80-120 MB | ~100+ MB | Native installers | FFI/NAPI required | Chromium attack surface |
| Native (egui/iced) | ~5-15 MB | ~20-30 MB | Yes | Pure Rust | Minimal |
| Flutter | ~20-30 MB | ~50-80 MB | Yes | FFI bridge | Medium |

### Release Strategy (GitHub Tags → Installers)
Extend the existing `.github/workflows/release.yml` to:
1. On tag push (`v*`), build the GUI wallet alongside the CLI/node binaries
2. Cross-compile for: Linux (x86_64), macOS (aarch64 + x86_64), Windows (x86_64)
3. Produce platform-specific installers: `.msi` (Windows), `.dmg` (macOS), `.AppImage` (Linux)
4. Attach all artifacts to the GitHub Release automatically
5. Users download from GitHub Releases — same workflow as now, but with GUI installers added

## Target Users
- **Primary**: Token holders / casual users — own DOLI, want to send, receive, check balances. Non-technical. 90% use case.
- **Secondary**: Producers / stakers — manage bonds, claim rewards. Semi-technical.
- **Tertiary**: NFT/token creators, bridge users — advanced features after MVP.

## Success Criteria
- A non-technical user can download an installer from GitHub, install it, create a wallet, and receive DOLI within 5 minutes
- The wallet connects to public DOLI RPC endpoints by default (no node setup)
- Private keys never leave the local machine
- Windows and macOS users have native-feeling installer experiences
- GitHub tag push triggers automated build + release with all platform installers

## Full Scope — All CLI Features in the GUI

The app ships with ALL functionality from day one. No "wallet-only MVP" — users get the complete DOLI experience.

### 1. Wallet Management
- Create new wallet (BIP-39 seed phrase, shown once for backup)
- Restore wallet from seed phrase
- Generate new addresses with labels
- View all addresses
- Export/import wallet file
- Add BLS keys (required for producers)

### 2. Balance & Transactions
- View total balance (confirmed + unconfirmed)
- View per-address balances
- Send DOLI to an address
- Spend covenant UTXOs
- Transaction history view
- Copy address to clipboard / QR code display

### 3. Producer Management (embedded node)
- **Register as producer** — bond minimum 10 DOLI, register with the network
- **Add Bond (re-bond)** — if user has more than 10 DOLI, add additional bond to increase rewards weight
- **Producer status dashboard** — scheduled slots, blocks produced, missed slots, uptime
- **Embedded node** — GUI bundles `doli-node`, user clicks "Start Node" → syncs and runs in background
- **Node controls** — start/stop, sync progress, peer count, chain height
- **Exit producer** — graceful exit from producer set

### 4. Bond & Withdrawal Management
- View all bonds (active, pending activation, pending withdrawal)
- **Add Bond** — re-bond additional DOLI (minimum 10 DOLI per bond)
- **Request Withdrawal** — FIFO, vesting penalty, 7-day delay
- **Claim Withdrawal** — claim after delay period
- **Simulate Withdrawal** — preview penalty before committing

### 5. Rewards
- View pending rewards
- Claim individual rewards
- Claim all rewards
- Rewards history

### 6. Delegation
- Delegate stake to an existing producer (for users who don't want to run a node)
- View delegation status and rewards

### 7. NFT & Tokens
- Mint NFTs
- Transfer NFTs
- View NFT info
- Issue custom tokens
- View token info

### 8. Bridge
- Bridge lock
- Bridge claim
- Bridge refund

### 9. Governance
- View pending updates
- Vote on updates
- View votes
- Maintainer management

### 10. Network Connection
- Connect to public mainnet RPC endpoints by default
- Option to configure custom RPC endpoint (for embedded node or remote)
- Network status indicator (connected/syncing/disconnected)
- Chain info display (height, epoch, network)
- Network selector (mainnet/testnet/devnet)

### 11. Platform Packaging & CI
- Windows: MSI/NSIS installer
- macOS: DMG with .app bundle
- Linux: AppImage
- GitHub Actions workflow: tag push → build all platforms → attach to release
- Auto-update via Tauri's built-in updater (checks GitHub Releases)
- Code signing (Windows + macOS) when certificates available

## Out of Scope (v1)
- Mobile versions
- Light client / SPV mode
- Web wallet / browser extension

## Key Technical Decisions

### 1. The GMP Problem (BIGGEST BLOCKER)
`doli-core` → `vdf` → `rug` → GMP (C library). GMP requires MSYS2/MinGW on Windows.

**Recommended**: Add feature flags to `doli-core` — `features = ["vdf"]` with VDF optional. The wallet only needs types + serialization + crypto, NOT VDF.

### 2. GUI Framework
**Recommended**: Tauri 2.x with Svelte (lightweight) or React (larger ecosystem) frontend.

### 3. Code Signing
- Windows: code-signing certificate needed (~$200-500/yr) or SmartScreen warnings
- macOS: Apple Developer ID + notarization (~$99/yr) or Gatekeeper blocks
- **Without signing, non-technical users will be scared off by security warnings**

### 4. Auto-Update
Use Tauri's built-in updater pointing at GitHub Releases — simpler than adapting the node's updater crate.

## Risks

### High
- **Code signing cost**: Without certificates, Windows SmartScreen and macOS Gatekeeper show scary warnings. Non-technical users will not proceed.
- **Embedded node on Windows**: The node binary needs GMP (via rug/MSYS2) and RocksDB — both compile on GitHub Actions Windows runners but need CI configuration.

### Medium
- Wallet file compatibility between GUI and CLI
- Private key security in desktop environment
- RPC endpoint reliability (need redundancy)
- Tauri webview rendering differences (WebView2 vs WKWebView)

### Low
- Transaction building (well-tested pattern from CLI)
- Ed25519/BLAKE3 (pure Rust, cross-platform)

## Architecture

```
+------------------+       JSON-RPC        +------------------+
|   DOLI GUI App   | <------------------> |   DOLI Node(s)   |
|  (Tauri/Desktop) |     HTTP POST         | (existing infra) |
+------------------+                       +------------------+
|  Frontend (Web)  |
|  - Svelte/React  |
|  - Wallet views  |
|  - Producer mgmt |
|  - Bond/Rewards  |
|  - NFT/Bridge/Gov|
+------------------+
|  Backend (Rust)  |
|  - crypto crate  |  ← key management, signing
|  - wallet.rs     |  ← wallet file I/O
|  - rpc_client.rs |  ← existing RPC client (from CLI)
|  - tx builder    |  ← transaction construction
|  - node manager  |  ← start/stop embedded doli-node
+------------------+

GitHub Actions (on tag push):
  → Build Tauri app for Linux/macOS/Windows
  → Produce .msi, .dmg, .AppImage
  → Attach to GitHub Release
  → Users download from Releases page
```

## Phasing

Single release — all features ship together. No phased rollout. The GUI is a 1:1 mirror of the CLI.

| Phase | Scope | Target |
|-------|-------|--------|
| **v1.0** | Full-featured GUI: wallet + producer + bonds + rewards + NFT + bridge + governance + embedded node | Q2 2026 |
| **v1.1** | UX polish, code signing, auto-update | Q2-Q3 2026 |
| **v2.0** | Mobile wallet | Q3 2026 |

## Reusable Code
- `bins/cli/src/wallet.rs` — wallet creation, key management, BIP-39
- `bins/cli/src/rpc_client.rs` — JSON-RPC client (all 31 methods)
- `crates/crypto/` — Ed25519, BLAKE3, BLS, address encoding
- `crates/core/` types — Transaction, Input, Output (with VDF gated behind feature flag)
