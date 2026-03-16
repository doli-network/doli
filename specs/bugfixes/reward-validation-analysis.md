# Bugfix Analysis: Epoch Reward Validation Gaps

## Investigation Summary

**Original claim**: "`validate_block_rewards_exact()` is never called, so a malicious producer can include incorrect EpochReward transactions and no peer will reject the block."

**Verdict**: PARTIALLY CORRECT. The legacy function is indeed dead code, but the claim that "no peer will reject the block" is wrong. The node layer has its own reward validation (`validate_block_economics` at `validation_checks.rs:263-399`) that IS called from `apply_block()`. In Full mode (gossip/production), it performs exact-match validation of EpochReward amounts and recipients.

However, the replacement has **three real gaps** and **dead legacy code**:

## Scope

| Domain | Files Affected |
|--------|---------------|
| Node validation | `bins/node/src/node/validation_checks.rs` |
| Node apply_block | `bins/node/src/node/apply_block/mod.rs` |
| Core validation (legacy) | `crates/core/src/validation/rewards_legacy.rs` |
| Core validation exports | `crates/core/src/validation/mod.rs` |
| Core validation tests | `crates/core/src/validation/tests.rs` |
| Core validation types | `crates/core/src/validation/types.rs` |
| Storage trait impls | `crates/storage/src/block_store/trait_impls.rs` |

## Gaps Found

### GAP-1 (Should): Light mode skips exact-match validation

At `validation_checks.rs:371`: During sync (`ValidationMode::Light`), only the conservation check (`total <= pool`) runs. A malicious peer could serve crafted blocks that redirect rewards to wrong recipients.

### GAP-2 (Must): Missing EpochReward TX not detected

At `validation_checks.rs:305`: All validation is inside `if !epoch_reward_txs.is_empty()`. A malicious producer at an epoch boundary can omit the EpochReward TX entirely. Rewards stay in pool (delayed, not lost), but violates protocol expectations.

### GAP-3 (Must): Truncated extra_data bypasses epoch check

At `validation_checks.rs:331`: The epoch/height check only runs when `extra_data.len() >= 16`. A crafted EpochReward TX with empty `extra_data` skips epoch number validation entirely.

### GAP-4 (Should): 458 lines of dead legacy code + 15 tests

`rewards_legacy.rs` is entirely dead code using a deprecated reward model. The `EpochBlockSource` trait exists only to serve this dead code.

## Requirements

| ID | Requirement | Priority |
|----|------------|----------|
| REQ-RWV-001 | Enforce EpochReward TX presence at epoch boundaries when rewards are due | Must |
| REQ-RWV-002 | Reject EpochReward TX with truncated extra_data (< 16 bytes) | Must |
| REQ-RWV-003 | Add exact-match reward validation in Light mode | Should |
| REQ-RWV-004 | Remove legacy `validate_block_rewards_exact` and related dead code | Should |
| REQ-RWV-005 | Update MEMORY.md Open Items to reflect actual risk profile | Must |

## Acceptance Criteria

### REQ-RWV-001
- When `is_epoch_boundary && epoch > 0 && calculate_epoch_rewards() non-empty && no EpochReward TX`, reject block
- Skip when `calculate_epoch_rewards()` returns empty (pool accumulates legitimately)
- All existing chain blocks pass (verify no historical blocks omit expected rewards)

### REQ-RWV-002
- If `extra_data.len() < 16`, reject with error
- All existing blocks pass (production always writes 16 bytes)

### REQ-RWV-003
- Light mode performs same exact-match check as Full mode at epoch boundaries
- Performance impact < 100ms per epoch boundary block (once per 360 blocks)

### REQ-RWV-004
- Delete `rewards_legacy.rs` or reduce to tombstone
- Remove `EpochBlockSource` trait if no production consumers
- Remove associated tests from `tests.rs`
- Full gate passes

### REQ-RWV-005
- MEMORY.md Open Items entry updated to reflect actual gaps

## Impact Analysis

- **Consensus-critical**: REQ-RWV-001 and REQ-RWV-002 add new rejection conditions — must not reject valid historical blocks
- **Deployment**: Requires simultaneous deploy (Law 7)
- **Sync performance**: REQ-RWV-003 runs `calculate_epoch_rewards()` once per 360 blocks in Light mode — minimal impact

## Implementation Status (2026-03-16)

All four gaps fixed:
- **GAP-1**: Removed `if mode == ValidationMode::Full` guard. Exact-match now runs in both modes.
- **GAP-2**: Added `else if is_epoch_boundary` branch detecting missing EpochReward TX.
- **GAP-3**: Changed `if len >= 16` to `if len < 16 { bail }`. Truncated extra_data now rejected.
- **GAP-4**: `rewards_legacy.rs` reduced to tombstone. `EpochBlockSource` trait removed from `types.rs`. `trait_impls.rs` EpochBlockSource impl removed. ~35 legacy tests removed from `tests.rs`.

Files changed:
- `bins/node/src/node/validation_checks.rs` (gaps 1-3)
- `crates/core/src/validation/rewards_legacy.rs` (tombstone)
- `crates/core/src/validation/mod.rs` (removed legacy exports)
- `crates/core/src/validation/types.rs` (removed EpochBlockSource trait)
- `crates/core/src/validation/tests.rs` (removed ~35 legacy tests)
- `crates/storage/src/block_store/trait_impls.rs` (removed EpochBlockSource impl)

Gate: `cargo build` + `cargo clippy -- -D warnings` + `cargo test` all pass (except pre-existing `test_rust_vs_json_genesis_hash` failure).

## Specs Drift Detected

- `MEMORY.md` Open Items: Says "Malicious producer can inflate rewards" — misleading. Conservation check prevents inflation. Real risks were reward redirection (Light mode) and reward suppression (missing TX). Both now fixed. Open Item should be removed.
- `PROJECT-UNDERSTANDING.md:379`: Lists gap as dead function without noting `validate_block_economics` exists.
