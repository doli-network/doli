# Test Writer Progress: DOLI GUI Desktop Application

## Status: COMPLETE (Rust backend tests)

## Test Summary

| Module | File | Tests Written | Tests Passing | Status |
|--------|------|---------------|---------------|--------|
| wallet (unit) | `crates/wallet/src/wallet.rs` | 49 | 49 | Complete |
| types (unit) | `crates/wallet/src/types.rs` | 18 | 18 | Complete |
| rpc_client (unit) | `crates/wallet/src/rpc_client.rs` | 24 | 24 | Complete |
| tx_builder (unit) | `crates/wallet/src/tx_builder.rs` | 40 | 40 | Complete |
| wallet_compat (integration) | `crates/wallet/tests/wallet_compat.rs` | 8 | 8 | Complete |
| tx_builder (integration) | `crates/wallet/tests/tx_builder.rs` | 8 | 8 | Complete |
| lib.rs | `crates/wallet/src/lib.rs` | 0 | N/A | Re-exports only |
| **TOTAL** | | **147** | **147** (initially, before TxBuilder impl) | |

Note: `TxBuilder::build_for_signing()` and `sign_and_build()` are `todo!()` stubs.
When the developer implements these, additional tests will begin exercising the
serialization path. The tests for transfer construction, add_bond, reward_claim, and
request_withdrawal builders all pass because they test the builder setup (inputs/outputs)
not the final serialization.

## Requirement Coverage

### Must Requirements (all tested)

| Requirement | Tests | Notes |
|-------------|-------|-------|
| GUI-FR-001 (Create wallet) | 12 unit + 3 integration | BIP-39, Ed25519+BLS, JSON format, seed not stored |
| GUI-FR-002 (Restore wallet) | 8 unit | Same seed = same key, invalid rejected, deterministic |
| GUI-FR-003 (Generate address) | 8 unit | Bech32m prefixes (doli/tdoli/ddoli), labels, unique keys |
| GUI-FR-004 (List addresses) | 2 unit | All addresses, primary first |
| GUI-FR-010 (Balance display) | 14 unit + 2 RPC mock | Unit conversions, 8 decimal places, zero handling |
| GUI-FR-011 (Send transaction) | 10 unit + 1 RPC mock + 1 integration | Builder, UTXO selection, insufficient balance, overflow |
| GUI-FR-014 (Transaction history) | 2 RPC mock | Pagination, empty history |
| GUI-FR-020 (Producer registration) | 6 unit + 2 integration | Bond cost, registration fee, scaling, max cap |
| GUI-FR-021 (Producer status) | 1 RPC mock | Producer info from RPC |
| GUI-FR-024 (Add bonds) | 5 unit + 1 integration | 10 DOLI per bond, max 3000, insufficient balance |
| GUI-FR-025 (Request withdrawal) | 7 unit + 2 integration | Vesting penalty, FIFO, net calculation |
| GUI-FR-030 (Rewards list) | 1 RPC mock | Epoch rewards, qualification status |
| GUI-FR-031 (Claim reward) | 2 unit | Reward claim builder, with recipient |
| GUI-FR-032 (Claim all rewards) | (covered by FR-031) | Same builder, iterated |
| GUI-FR-070 (Chain info) | 1 RPC mock | Network, height, slot, genesis hash |
| GUI-FR-080 (Public RPC endpoints) | 4 unit | Mainnet HTTPS, testnet, devnet localhost, unknown |
| GUI-FR-081 (Custom RPC) | 2 RPC mock | Connection test success/failure |
| GUI-FR-082 (Network selector) | 4 unit | doli/tdoli/ddoli prefixes, default |
| GUI-FR-083 (Connection status) | 1 unit | URL storage |
| GUI-NF-004 (Private key security) | 3 unit | Key not exposed, signing internal, wrong address error |
| GUI-NF-008 (Wallet compat) | 6 unit + 8 integration | JSON structure, legacy v1, BLS optional, CLI format match |
| GUI-NF-013 (VDF feature flag) | 1 integration | Wallet crate compiles without VDF |

### Should Requirements (partially tested)

| Requirement | Tests | Notes |
|-------------|-------|-------|
| GUI-FR-005 (Export) | 1 unit + 1 integration | Save to path |
| GUI-FR-006 (Import) | 2 unit | Load and validate format |
| GUI-FR-007 (Wallet info) | 1 unit | Name, version, address count |
| GUI-FR-008 (Add BLS key) | 2 unit | Error if exists, add to wallet without BLS |
| GUI-FR-022 (Bond details) | 1 RPC mock | Bond vesting info from RPC |
| GUI-FR-034 (Epoch info) | 1 RPC mock | Epoch info from RPC |

### Could Requirements (basic coverage)

| Requirement | Tests | Notes |
|-------------|-------|-------|
| GUI-FR-100 (Sign message) | 1 unit | Basic signing |
| GUI-FR-101 (Verify signature) | 3 unit | Valid, wrong message, wrong key |
| GUI-FR-110/111 (Delegation) | 1 integration | TxType enum values only |

### Not Tested (frontend-only or CI/CD)

- GUI-FR-015 (Copy to clipboard) -- frontend Svelte component
- GUI-FR-016 (QR code) -- frontend Svelte component
- GUI-FR-040-044 (NFT/tokens) -- Should priority, TxBuilder deferred
- GUI-FR-050-052 (Bridge) -- Should priority, TxBuilder deferred
- GUI-FR-060-064 (Governance) -- Should priority, RPC query only
- GUI-NF-001 (Platform support) -- CI/CD verification
- GUI-NF-007 (CI pipeline) -- CI/CD verification
- GUI-NF-011 (Loading states) -- frontend Svelte component

## Modules Still Needing Tests

1. **Tauri commands** (`bins/gui/src-tauri/src/commands/`) -- Cannot be tested until the
   Tauri app shell exists. These tests will wrap the wallet crate functions and verify
   the IPC interface. Recommend writing these after the developer creates the Tauri
   app structure in Milestone M2.

2. **TxBuilder serialization** -- The `build_for_signing()` and `sign_and_build()` methods
   are `todo!()`. Once the developer implements canonical serialization, add tests that:
   - Compare output bytes against known-good transactions from the existing node
   - Verify signatures are valid per crypto crate
   - Test round-trip: build -> serialize -> deserialize matches

3. **NFT/Token/Bridge/Governance TxBuilder** -- Should-priority builders for TxType 4, 5,
   11, 12. These follow the same pattern as Transfer/AddBond/RewardClaim builders.

## Specs Gaps Found

1. **Vesting penalty schedule ambiguity**: The requirements document (GUI-FR-025) says
   "75%/50%/25%/0% penalty at Year 1/2/3/4" but doesn't clarify whether Year 1 means
   "during the first year" or "at the end of the first year". The code in `consensus.rs`
   uses `VESTING_QUARTER_SLOTS` intervals. The test assumes: age 0 to 1yr = 75% penalty,
   1yr to 2yr = 50%, 2yr to 3yr = 25%, 4yr+ = 0%. This leaves 3yr to 4yr undefined --
   need to confirm if it's 25% or 0%.

2. **coins_to_units signature mismatch**: The CLI's `rpc_client.rs` uses `coins_to_units(f64) -> u64`
   but the architecture specifies `coins_to_units(&str) -> Result<u64>`. The shared wallet
   crate implements the string-based version for safety (no floating-point precision loss
   from user input). The developer should verify the CLI migration path.

3. **Registration fee formula**: The requirements say fee "scales with pending count" but
   don't specify the exact formula. The test assumes `fee = BASE_FEE * (1 + pending_count)`
   capped at `MAX_REGISTRATION_FEE`, based on reading `consensus.rs`. If the formula changes,
   `calculate_registration_cost()` and its tests need updating.

## Files Created

- `crates/wallet/Cargo.toml`
- `crates/wallet/src/lib.rs`
- `crates/wallet/src/wallet.rs` (49 tests)
- `crates/wallet/src/rpc_client.rs` (24 tests)
- `crates/wallet/src/tx_builder.rs` (40 tests)
- `crates/wallet/src/types.rs` (18 tests)
- `crates/wallet/tests/wallet_compat.rs` (8 tests)
- `crates/wallet/tests/tx_builder.rs` (8 tests)
- `docs/.workflow/test-writer-progress.md` (this file)

## Traceability Matrix

Updated in `specs/gui-desktop-requirements.md` -- all Must requirement Test IDs filled in.
