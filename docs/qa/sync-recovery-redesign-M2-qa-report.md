# QA Report: Sync Recovery Redesign - Milestone 2

## Scope Validated

Site migration of `needs_genesis_resync = true` write sites to `request_genesis_resync(RecoveryReason::...)`, monotonic floor extension to `reset_sync_for_rollback()`, and behavioral verification that gates actually refuse recovery when conditions are met.

Files validated:
- `crates/network/src/sync/manager/cleanup.rs` (3 sites migrated)
- `crates/network/src/sync/manager/sync_engine.rs` (3 sites migrated)
- `crates/network/src/sync/manager/block_lifecycle.rs` (1 site migrated + floor check added)
- `crates/network/src/sync/manager/production_gate.rs` (`request_genesis_resync()` gate, `set_needs_genesis_resync()` NOT migrated)
- `crates/network/src/sync/manager/mod.rs` (RecoveryReason enum, ForkState field)
- `crates/network/src/sync/manager/tests.rs` (M2 site migration + floor extension tests)
- `bins/node/src/node/block_handling.rs` (Node-level caller of `set_needs_genesis_resync()`)
- `bins/node/src/node/rollback.rs` (Node-level caller of `set_needs_genesis_resync()`)
- `specs/sync-recovery-architecture.md` (architecture spec, sections 3-5)

## Summary

**FAIL** -- M2 migrated 7 of 9 `needs_genesis_resync = true` write sites within the SyncManager, but the 9th site (`set_needs_genesis_resync()` at production_gate.rs:1087) was NOT migrated as the spec requires. The method still exists as a direct flag write that bypasses all 5 recovery gates. Critically, 2 Node-level callers (`block_handling.rs:601` and `rollback.rs:244`) invoke this ungated method, meaning the rollback death spiral and fork-sync-too-deep paths can still bypass the monotonic floor, rate limiting, and all other gate protections. The spec explicitly states at line 304 that `set_needs_genesis_resync()` should be replaced by `request_genesis_resync(RecoveryReason::RollbackDeathSpiral)`. This is the one remaining gate bypass that defeats the entire purpose of M2.

The other 7 SyncManager-internal site migrations are correct and well-tested. The monotonic floor extension to `reset_sync_for_rollback()` is correctly implemented and tested with proper edge cases. All 148 tests pass. Build and clippy are clean.

## System Entrypoint

- Build: `cargo build --package network`
- Tests: `cargo test --package network --lib sync::manager::tests`
- Clippy: `cargo clippy --package network -- -D warnings`
- No runtime system required -- M2 is a behavioral refactor tested via unit tests. The system is a PoS blockchain node that cannot be trivially started in isolation for integration testing.

## Traceability Matrix Status

| Requirement ID | Priority | Has Tests | Tests Pass | Acceptance Met | Notes |
|---|---|---|---|---|---|
| REQ-SYNC-102 | Must | Yes | Yes | Partial | Floor extended to `reset_sync_for_rollback()` -- PASS. But `set_needs_genesis_resync()` bypass circumvents the floor at the Node level. |
| REQ-SYNC-103 | Must | Yes | Yes | No | Spec says "needs_genesis_resync set from 1 path, not 9". Currently set from 2 paths: `request_genesis_resync()` (gated, line 1164) AND `set_needs_genesis_resync()` (ungated, line 1087). |
| REQ-SYNC-105 | Must | Yes | Yes | Partial | RecoveryReason logging works for 7 of 8 migrated sites. The RollbackDeathSpiral path (via `set_needs_genesis_resync()`) has no reason logging. |
| PRESERVE-5 | Must | Yes | Yes | Yes | All 148 tests pass. Fresh node behavior preserved. |

### Gaps Found

- **BLOCKING: `set_needs_genesis_resync()` not migrated** -- The spec at line 304 explicitly lists this as a method to be removed and replaced by `request_genesis_resync(RecoveryReason::RollbackDeathSpiral)`. It still exists and is called from 2 Node-level sites.
- **Test T-M2-023 documents this gap explicitly** -- The test at line 4032 says: "If set_needs_genesis_resync() is removed in M2, this test should be UPDATED to verify the method no longer exists." The test confirms the method still exists and still bypasses the gate. This is correct as a transitional test -- but M2's scope was to complete the migration.
- **Test T-M2-006 tests the replacement reason** -- The test at line 3520 verifies that `RecoveryReason::RollbackDeathSpiral` is correctly refused by the gate. This proves the gate WOULD work if the callers were migrated. They just are not migrated yet.

## Acceptance Criteria Results

### Must Requirements

#### REQ-SYNC-103: Single gated path for needs_genesis_resync

**Spec statement** (line 15): "the primary intervention is removing the ability to write needs_genesis_resync directly and replacing it with a gated method that rejects most requests"

- [x] cleanup.rs site #7 (AllPeersBlacklistedDeepFork, line 344) -- PASS: Uses `request_genesis_resync()`
- [x] cleanup.rs site #8 (StuckSyncLargeGap, line 485) -- PASS: Uses `request_genesis_resync()`
- [x] cleanup.rs site #9 (HeightOffsetDetected, line 529) -- PASS: Uses `request_genesis_resync()`
- [x] sync_engine.rs site #2 (PostRollbackSnapEscalation, line 274) -- PASS: Uses `request_genesis_resync()`
- [x] sync_engine.rs site #3 (GenesisFallbackEmptyHeaders, line 415) -- PASS: Uses `request_genesis_resync()`
- [x] sync_engine.rs site #4 (BodyDownloadPeerError, line 768) -- PASS: Uses `request_genesis_resync()`
- [x] block_lifecycle.rs site #6 (ApplyFailuresSnapThreshold, line 224) -- PASS: Uses `request_genesis_resync()`
- [ ] production_gate.rs site #1 (set_needs_genesis_resync, line 1087) -- **FAIL**: Still directly writes `self.fork.needs_genesis_resync = true` without gate checks. Spec line 304 says replace with `request_genesis_resync(RecoveryReason::RollbackDeathSpiral)`.

Node-level callers that use the ungated method:
- `bins/node/src/node/block_handling.rs:601` -- fork sync reorg too deep (calls `sync.set_needs_genesis_resync()`)
- `bins/node/src/node/rollback.rs:244` -- rollback death spiral (calls `sync.set_needs_genesis_resync()`)

#### REQ-SYNC-102: Monotonic progress floor in rollback paths

- [x] `reset_sync_for_rollback()` checks floor -- PASS (block_lifecycle.rs:627-634)
- [x] Floor=0 allows rollback -- PASS (test T-M2-012)
- [x] Height=floor refused -- PASS (test T-M2-010)
- [x] Height<floor refused -- PASS (test T-M2-010b)
- [x] Height=floor+1 allowed -- PASS (test T-M2-013)
- [x] Height=0 with floor>0 allowed (genesis bypass) -- PASS (test T-M2-012b)
- [x] `reset_sync_after_successful_reorg()` NOT blocked by floor -- PASS (test T-M2-014)
- [x] Both `reset_local_state()` and `reset_sync_for_rollback()` respect floor -- PASS (test T-M2-015)

#### PRESERVE-5: All existing tests pass

- [x] All 148 tests pass -- PASS
- [x] Build clean -- PASS
- [x] Clippy clean -- PASS

### Should Requirements

#### REQ-SYNC-105: Recovery reason logging

- [x] All 7 migrated sites log via `request_genesis_resync()` with reason -- PASS
- [ ] RollbackDeathSpiral path does not go through gated method, so no structured reason logging -- FAIL (non-blocking, tracked as observation)

## End-to-End Flow Results

| Flow | Steps | Result | Notes |
|---|---|---|---|
| Fresh node genesis resync | Floor=0, snap enabled, trigger any RecoveryReason | PASS | All 8 RecoveryReason variants accepted (test T-M2-009b) |
| Floor-protected node refuses resync | Floor>0, trigger each RecoveryReason | PASS | All 8 variants refused (test T-M2-009) |
| Rollback floor enforcement | Various height/floor combos via reset_sync_for_rollback | PASS | 6 edge case tests all pass (T-M2-010 through T-M2-013) |
| Apply failure escalation (large gap) | 3+ failures, gap>50 on fresh node | PASS | Correctly sets needs_genesis_resync via gate (test T-M2-007) |
| Apply failure escalation (small gap) | 3+ failures, gap<=50 | PASS | Correctly calls signal_stuck_fork instead (test T-M2-022) |
| Death spiral bypass | Node calls set_needs_genesis_resync() with floor>0 | **BYPASS** | Flag set directly, floor not checked. Gate is circumvented. |

## Exploratory Testing Findings

| # | What Was Tried | Expected | Actual | Severity |
|---|---|---|---|---|
| 1 | Searched for all `needs_genesis_resync = true` writes in SyncManager | Only inside `request_genesis_resync()` | Also in `set_needs_genesis_resync()` at line 1087 | high |
| 2 | Searched for Node-level callers of `set_needs_genesis_resync()` | Zero (all migrated to use gated method) | 2 callers: block_handling.rs:601, rollback.rs:244 | high |
| 3 | Checked if `set_needs_genesis_resync()` has any gate logic | Has gate logic (or is removed) | No gate logic -- single line `self.fork.needs_genesis_resync = true` | high |
| 4 | Verified `reset_local_state()` floor still works | Floor check present | PASS -- floor check at block_lifecycle.rs:247-284 still active | -- |
| 5 | Checked if `block_lifecycle.rs` site #5 (small gap) was intentionally NOT migrated | Intentional -- small gap uses `signal_stuck_fork()` not genesis resync | PASS -- confirmed at block_lifecycle.rs:203-211, tested by T-M2-022 | -- |
| 6 | Checked `needs_genesis_resync = false` reset sites for consistency | `reset_local_state()` clears the flag | PASS -- block_lifecycle.rs:262 and 319 clear the flag during actual reset | -- |
| 7 | Checked if fresh node joining network could be TOO restricted by gates | Fresh nodes (floor=0) should pass all gates | PASS -- floor=0 passes gate 1, no prior resyncs passes gates 2-3, snap enabled passes gate 4-5 | -- |

## Failure Mode Validation

| Failure Scenario | Triggered | Detected | Recovered | Degraded OK | Notes |
|---|---|---|---|---|---|
| Floor blocks genesis resync | Yes (test) | Yes (warn log) | N/A | Yes | Node stays at current height, manual intervention required |
| Concurrent resync blocked | Yes (test) | Yes (info log) | N/A | Yes | Second request ignored while first in progress |
| Max resyncs exceeded | Yes (test) | Yes (warn log) | N/A | Yes | Node stops retrying, manual intervention required |
| Snap sync disabled | Yes (test) | Yes (info log) | N/A | Yes | Falls back to header-first recovery |
| Snap attempts exhausted | Yes (test) | Yes (info log) | N/A | Yes | Node stops snap sync attempts |
| Death spiral via set_needs_genesis_resync | Yes (code analysis) | No | No | **No** | The ungated method bypasses all 5 gates. A node with floor=100 at height 50 can still be reset to 0. |

## Security Validation

Not applicable to this milestone. M2 is an internal sync state machine refactor with no external data surfaces, no network protocol changes, and no user-facing API modifications.

## Specs/Docs Drift

| File | Documented Behavior | Actual Behavior | Severity |
|------|-------------------|-----------------|----------|
| specs/sync-recovery-architecture.md line 304 | `set_needs_genesis_resync()` replaced by `request_genesis_resync(RollbackDeathSpiral)` | `set_needs_genesis_resync()` still exists and is used by 2 Node-level callers | high |
| specs/sync-recovery-architecture.md line 527 | Step 3 item 4: "production_gate.rs (1 site -- line 1087, replace set_needs_genesis_resync())" | Site #1 not migrated in M2 | high |
| specs/sync-recovery-architecture.md line 534 | M2 scope includes production_gate.rs (1 site) | The 1 site in production_gate.rs was not migrated | high |

## Blocking Issues (must fix before merge)

- **[ISSUE-001]**: `production_gate.rs:1086-1088` -- `set_needs_genesis_resync()` still exists as an ungated direct write. The spec (line 304) says it should be replaced by `request_genesis_resync(RecoveryReason::RollbackDeathSpiral)`. Two Node-level callers (`block_handling.rs:601` and `rollback.rs:244`) use this method, completely bypassing the monotonic floor, concurrent recovery check, rate limiting, snap availability check, and snap attempt limit. This defeats the core purpose of M2: routing ALL 9 write sites through the gated method.

  **Expected**: `set_needs_genesis_resync()` removed. Both Node-level callers updated to call `sync.request_genesis_resync(RecoveryReason::RollbackDeathSpiral { peak, current })` and handle the `false` return (gate refused) gracefully.

  **Actual**: `set_needs_genesis_resync()` is a 3-line method that directly sets the flag with no checks.

  **Impact**: A previously-healthy node (confirmed_height_floor > 0) that enters the rollback death spiral path will bypass ALL gate protections and be reset to height 0, reintroducing the exact cascade that M2 was designed to prevent.

## Non-Blocking Observations

- **[OBS-001]**: `tests.rs:4032-4052` -- Test T-M2-023 explicitly documents that `set_needs_genesis_resync()` still exists as a bypass. The test comment says "When the developer replaces set_needs_genesis_resync() with request_genesis_resync(RollbackDeathSpiral), this test should be changed to verify the new behavior." This is a good transitional test but should be updated after the fix.

- **[OBS-002]**: The `block_lifecycle.rs` original spec listed "2 sites" to migrate but only 1 was actually a `needs_genesis_resync = true` write (the large-gap path at line 224). The small-gap path at line 203 correctly uses `signal_stuck_fork()` instead. The numbering in the spec (lines 232-233) references two sites but the second one was already `signal_stuck_fork()` pre-M2. No issue -- just a spec numbering discrepancy.

- **[OBS-003]**: `block_lifecycle.rs:262` and `block_lifecycle.rs:319` -- The flag is cleared (`needs_genesis_resync = false`) during `reset_local_state()`. These are correct -- once the actual reset happens, the flag should be consumed. These are not write sites that need gating (they CLEAR the flag, not SET it).

## Modules Not Validated (if context limited)

- **M3 scope (hard enforcement, ForkAction wiring)**: Not yet implemented. Out of scope for M2.
- **Node-level `execute_reorg()` floor check**: Spec line 486-493 documents this as an M2 contract for the developer. `fork_recovery.rs:604` has a floor check for `reset_state_only()` but not for `execute_reorg()` in `block_handling.rs`. This may be M3 scope based on the milestone table (line 567).

## Final Verdict

**FAIL** -- The following Must requirement is not met: REQ-SYNC-103 ("needs_genesis_resync set from 1 path, not 9"). Currently set from 2 paths: the gated `request_genesis_resync()` and the ungated `set_needs_genesis_resync()`. The spec explicitly identifies this as site #1 of the 9-site migration that M2 must complete. The two Node-level callers (`block_handling.rs:601`, `rollback.rs:244`) provide a live bypass around all 5 recovery gates.

**To resolve**: Replace `set_needs_genesis_resync()` with a call to `request_genesis_resync(RecoveryReason::RollbackDeathSpiral { peak, current })` in both Node-level callers. Remove or deprecate `set_needs_genesis_resync()`. Handle the `false` return (gate refused recovery) -- likely by logging and continuing without reset.
