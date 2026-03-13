# QA Re-validation Progress (2026-03-13)

## Scope: Re-validate 3 blocking issues from GUI Desktop QA

### ISSUE-001: TxBuilder build_for_signing() and sign_and_build()
- Status: RESOLVED
- build_for_signing(): Signing message format verified identical to doli-core::Transaction::signing_message()
- sign_and_build(): Full implementation present, no todo!() macros
- New concern (OBS-008): bincode serialization compatibility -- Hash/Signature custom serde uses serialize_bytes (with u64 length prefix) in bincode, but wallet writes raw bytes without prefix

### ISSUE-002: Registration fee tiered calculation
- Status: RESOLVED
- fee_multiplier_x100() in wallet matches core line-by-line (all 8 tiers)
- registration_fee() formula matches core exactly
- Constants verified: BASE=100,000, MAX=1,000,000
- test_fr020_registration_fee_matches_protocol passes

### ISSUE-003: CI pipeline for GUI builds
- Status: RESOLVED
- 3 new jobs: build-gui-linux, build-gui-macos, build-gui-windows
- Each installs: Rust, Node.js 20, platform deps, Tauri CLI v2
- Artifacts: .AppImage/.deb, .dmg, .msi
- Release job depends on all 5 build jobs (2 existing + 3 new GUI)

### Test Results
- wallet: 162 tests pass (146 unit + 8 tx_builder integration + 8 wallet_compat integration)
- doli-gui: 14 tests pass
- Total: 176 tests, 0 failures

### Final Verdict
Changed from CONDITIONAL APPROVAL to PASS.
