# Developer Progress: DOLI GUI Desktop Application

## Status: Code Review Bug Fixes Complete

### Code Review Bug Fix Round (2026-03-13)

#### C1: Bincode serialization mismatch -- FIXED (Critical)
- `crates/wallet/src/tx_builder.rs`: Added u64 LE length prefixes before all Hash (32 bytes) and Signature (64 bytes) fields in `sign_and_build()`
- `doli-core`'s `Hash` and `Signature` use custom `Serialize` implementations calling `serialize_bytes()`, which in bincode 1.x writes a u64 LE length prefix before the raw bytes
- Before fix: wallet wrote raw bytes (no prefix), causing deserialization failure at the node
- After fix: wallet writes `32u64.to_le_bytes()` before every Hash field and `64u64.to_le_bytes()` before every Signature field
- Verified byte-identical output with `bincode::serialize()` on `doli-core::Transaction`

#### C2: register_producer uses wrong transaction type -- FIXED (Critical)
- `bins/gui/src/commands/producer.rs`: Changed `register_producer()` to return a clear error explaining that VDF proof computation is required and must be done via CLI
- Registration requires `TxType::Registration` (1) with VDF proof, but the wallet crate cannot compute VDF (no doli-core dependency)
- Previous code incorrectly used `TxBuilder::build_add_bond()` which produces `TxType::AddBond` (7) -- wrong type entirely
- All other producer operations (add-bond, withdrawal, simulate, exit, status) remain functional

#### M1: Cross-crate serialization test -- ADDED (Major)
- `crates/wallet/tests/serialization_compat.rs`: 4 tests verifying byte-identical output
- `test_m1_transfer_serialization_matches_core`: Builds Transfer tx via TxBuilder and doli-core, compares byte-for-byte
- `test_m1_add_bond_serialization_matches_core`: Same for AddBond
- `test_m1_hash_field_has_length_prefix`: Verifies u64 LE prefix bytes at expected offsets
- `test_m1_wallet_tx_deserializes_in_core`: Verifies `bincode::deserialize::<Transaction>()` succeeds on wallet output
- Added `doli-core` and `bincode` as dev-dependencies only (runtime wallet still has no doli-core dep)

#### M2: Fix panics in wallet.rs -- FIXED (Major)
- `crates/wallet/src/wallet.rs`: Changed `primary_pubkey_hash()` and `primary_bech32_address()` from `.expect()` to return `Result<String, anyhow::Error>`
- Updated all callers in wallet crate tests, wallet_compat tests, and all GUI command files
- No more panic paths from invalid wallet data

#### M3: Path sanitization for wallet operations -- ADDED (Major)
- `bins/gui/src/commands/wallet.rs`: Added `validate_path()` helper function
- Rejects paths containing `..` (directory traversal), null bytes, and empty paths
- Applied to all path parameters: `create_wallet`, `restore_wallet`, `load_wallet`, `export_wallet`, `import_wallet`
- Added 6 unit tests for path validation

#### M4: Fix list_addresses error handling -- FIXED (Major)
- `bins/gui/src/commands/wallet.rs`: Replaced `unwrap_or_default()` with proper error handling
- Now uses explicit `hex::decode()` and `from_pubkey()` with match/continue pattern
- Addresses with invalid public keys are silently skipped instead of producing empty/garbage bech32 addresses

#### M5: Fix coins_to_units integer-only parsing -- FIXED (Major)
- `crates/wallet/src/types.rs`: Rewrote `coins_to_units()` to use integer-only arithmetic
- Splits on `.`, parses integer and fractional parts separately
- Pads/truncates fractional to exactly 8 digits
- Combines: `integer * UNITS_PER_DOLI + fractional`
- No floating point involved, eliminating precision loss for any valid DOLI amount
- Handles overflow detection for both parse overflow and arithmetic overflow

### Validation Results
- `cargo check -p wallet -p doli-gui` -- PASS
- `cargo test -p wallet` -- 166 tests PASS (146 unit + 4 serialization_compat + 8 tx_builder + 8 wallet_compat)
- `cargo test -p doli-gui` -- 19 tests PASS
- `cargo clippy -p wallet -p doli-gui -- -D warnings` -- CLEAN (0 warnings)
- `cargo build` (full workspace) -- PASS
- `cargo test` (full workspace) -- All tests PASS, 0 failures

### Previously Completed Milestones

#### M1: Core Infrastructure
- [x] `crates/wallet/` with full test coverage
- [x] VDF feature flag approach (wallet crate avoids doli-core)

#### M2: Tauri App Shell + Wallet
- [x] `bins/gui/` -- Tauri 2.x binary crate with 35+ command handlers

#### M3: Transactions + Balance
- [x] Transaction commands, address decoding

#### M4: Producer + Rewards
- [x] Producer and rewards commands

#### M5: NFT + Bridge + Governance
- [x] NFT, bridge, governance command stubs

#### Frontend (Svelte 5 SPA)
- [x] Complete Svelte 5 frontend in `bins/gui/src-ui/`

#### M6: CI/CD Pipeline
- [x] `.github/workflows/release.yml` -- Multi-platform GUI builds (Linux, macOS, Windows)
