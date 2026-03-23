# QA Report: Sync Recovery Redesign - Milestone 1

## Scope Validated

Recovery gate method (`request_genesis_resync`), state transition validation (`is_valid_transition`), `RecoveryReason` enum, `set_state()` warn-only integration, and behavior preservation of all existing sync manager functionality.

Files validated:
- `crates/network/src/sync/manager/mod.rs` (RecoveryReason enum, is_valid_transition, set_state changes)
- `crates/network/src/sync/manager/production_gate.rs` (request_genesis_resync method)
- `crates/network/src/sync/manager/tests.rs` (new test modules: recovery_gate_tests, transition_validation_tests)
- `specs/sync-recovery-architecture.md` (architecture spec, sections 2 and 4)

## Summary

**PASS** -- M1 is correctly implemented as an additive-only change with zero behavioral impact on existing code. The commit (dc4bb7c) adds 3 new constructs: the `RecoveryReason` enum (8 variants), `request_genesis_resync()` method (5 gates), and `is_valid_transition()` method (14-rule transition matrix). No existing call sites were modified. The `set_state()` method now calls `is_valid_transition()` in warn-only mode (logs but does not block invalid transitions). All 120 tests pass. Build and clippy are clean.

## System Entrypoint

- Build: `cargo build --package network`
- Tests: `cargo test --package network --lib sync::manager::tests`
- Clippy: `cargo clippy --package network -- -D warnings`
- No runtime system start required for M1 -- this is an additive refactor with no behavioral changes and no new call sites in production code.

## Verification Results

### 1. Behavior Preservation (CRITICAL) -- PASS

| Check | Result | Evidence |
|-------|--------|----------|
| All 120 tests pass | PASS | `cargo test --package network --lib sync::manager::tests` -- 120 passed, 0 failed |
| Build clean | PASS | `cargo build --package network` -- no errors |
| Clippy clean | PASS | `cargo clippy --package network -- -D warnings` -- no warnings |
| 9 original write sites unchanged | PASS | `grep` confirms all 9 `needs_genesis_resync = true` at original locations (production_gate.rs:1087, sync_engine.rs:274/415/767, block_lifecycle.rs:226/232, cleanup.rs:344/483/524) |
| `set_state()` still executes all transitions | PASS | Code at mod.rs:1113-1133 warns on invalid transitions but does NOT return early. Test T-TV-026 confirms an invalid transition (Synchronized->DownloadingBodies) still executes in M1 warn-only mode |
| `request_genesis_resync()` has no production callers | PASS | grep confirms the method is only called from `tests.rs`, never from production code |
| `RecoveryReason` has no production usage | PASS | grep confirms enum variants only appear in `tests.rs` |
| `is_valid_transition()` only called from `set_state()` | PASS | One production call site (mod.rs:1115); rest are test calls |
| No existing methods modified | PASS | git diff of M1 commit (dc4bb7c) shows only additions to `mod.rs` and `production_gate.rs`, plus a trivial refactor of `state_label()` -> `label_for()` |
| Old `set_needs_genesis_resync()` still exists | PASS | Method at production_gate.rs:1086-1088 is untouched |

### 2. Architecture Compliance -- PASS

| Spec Element | Spec Location | Code Match | Notes |
|-------------|---------------|------------|-------|
| RecoveryReason: 8 variants | Spec lines 331-348 | EXACT MATCH | All 8 variants present with identical names, types, and doc comments |
| request_genesis_resync: 5 gates | Spec lines 367-427 | EXACT MATCH | Gate order matches: floor, ResyncInProgress, rate limit, snap disabled, snap attempts |
| Gate 1: `confirmed_height_floor > 0` | Spec line 369 | MATCH | production_gate.rs:1103 |
| Gate 2: `ResyncInProgress` | Spec line 379 | MATCH | production_gate.rs:1113 |
| Gate 3: `consecutive_resync_count >= MAX` | Spec line 389 | MATCH | production_gate.rs:1123 |
| Gate 4: `snap.threshold == u64::MAX` | Spec line 399 | MATCH | production_gate.rs:1135 |
| Gate 5: `snap.attempts >= 3` | Spec line 409 | MATCH | production_gate.rs:1145 |
| is_valid_transition: transition matrix | Spec lines 205-214, 435-465 | MATCH with note | See OBS-001 below |
| set_state: warn-only mode | Spec step 2 (line 514) | MATCH | Code warns but still executes (no `return;`) |
| Method location: production_gate.rs | Spec line 430 | MATCH | Same file as `signal_stuck_fork()` |
| Follows `signal_stuck_fork()` pattern | Spec line 170 | MATCH | Same gated-method-on-SyncManager pattern |

### 3. Code Quality -- PASS

| Aspect | Assessment |
|--------|-----------|
| Edge case: `confirmed_height_floor == 0` | Handled correctly. Gate 1 uses `> 0`, so floor==0 passes. Test T-RG-010 verifies. |
| Edge case: `snap.attempts == 2` | Accepted correctly. Gate 5 uses `>= 3`, so attempts==2 passes. Test T-RG-005b verifies. |
| Edge case: `snap.attempts == 3` | Refused correctly. `>= 3` catches exactly 3. Test T-RG-012 verifies. |
| Edge case: `consecutive_resync_count == MAX` | Refused correctly. `>= MAX` catches exactly MAX. Test T-RG-011 verifies. |
| Edge case: `consecutive_resync_count == MAX-1` | Accepted correctly. Test T-RG-003b verifies. |
| Log messages | Consistent `[RECOVERY]` prefix for gate method, `[SYNC_STATE]` for transitions. REFUSED messages include reason and current values. ACCEPTED messages include full state. |
| Log levels | `warn!` for gates that indicate a problem (floor, rate limit), `info!` for expected refusals (active resync, snap disabled, snap exhausted). Appropriate differentiation. |
| Idempotent behavior | Flag can be set to true multiple times. Test T-RG-008 verifies no panic or incorrect state on double-call. |
| Method visibility | `request_genesis_resync` is `pub` (needed for Node access in M2). `is_valid_transition` is `fn` (private, only called from `set_state()`). Correct. |

### 4. Test Coverage -- PASS

#### Recovery Gate Tests (recovery_gate_tests module)

| Test ID | What It Tests | Result |
|---------|---------------|--------|
| T-RG-001 | Gate 1: height floor > 0 refuses | PASS |
| T-RG-002 | Gate 2: ResyncInProgress refuses | PASS |
| T-RG-003 | Gate 3: MAX resyncs refuses | PASS |
| T-RG-003b | Gate 3 boundary: MAX-1 accepts | PASS |
| T-RG-004 | Gate 4: snap disabled refuses | PASS |
| T-RG-005 | Gate 5: 3 snap attempts refuses | PASS |
| T-RG-005b | Gate 5 boundary: 2 snap attempts accepts | PASS |
| T-RG-006 | Happy path: all gates pass | PASS |
| T-RG-007 | All 8 RecoveryReason variants work | PASS |
| T-RG-008 | Idempotent double-call | PASS |
| T-RG-009 | Gate ordering: floor checked before snap | PASS |
| T-RG-010 | Edge: floor==0 passes gate 1 | PASS |
| T-RG-011 | Edge: resync_count==MAX refused | PASS |
| T-RG-012 | Edge: snap_attempts==3 refused | PASS |
| T-RG-013 | Gate 2 phase specificity (only ResyncInProgress blocks) | PASS |

15 tests covering all 5 gates, boundary conditions, edge cases, and all RecoveryReason variants.

#### Transition Validation Tests (transition_validation_tests module)

| Test ID | What It Tests | Result |
|---------|---------------|--------|
| T-TV-001 | Idle -> all states valid | PASS |
| T-TV-002 | All states -> Idle valid | PASS |
| T-TV-003 | SnapCollecting -> Synchronized INVALID | PASS |
| T-TV-004 | Processing -> SnapCollecting INVALID | PASS |
| T-TV-005 | Synchronized -> DownloadingBodies INVALID | PASS |
| T-TV-006 | Headers -> Bodies valid | PASS |
| T-TV-007 | Full snap sync forward path valid | PASS |
| T-TV-008 | Full header-first forward path valid | PASS |
| T-TV-009 | Headers -> SnapCollecting valid | PASS |
| T-TV-010 | Headers -> Synchronized valid | PASS |
| T-TV-011 | Bodies -> Bodies (self) valid | PASS |
| T-TV-012 | Bodies -> Synchronized valid | PASS |
| T-TV-013 | Processing -> Processing (self) valid | PASS |
| T-TV-014 | Synchronized -> Synchronized (self) valid | PASS |
| T-TV-015 | SnapDownloading -> SnapDownloading (self) valid | PASS |
| T-TV-016 | Synchronized -> Headers INVALID | PASS |
| T-TV-017 | Synchronized -> Processing INVALID | PASS |
| T-TV-018 | Processing -> Headers INVALID | PASS |
| T-TV-019 | Processing -> Bodies INVALID | PASS |
| T-TV-020 | SnapCollecting -> Processing INVALID | PASS |
| T-TV-021 | SnapCollecting -> Bodies INVALID | PASS |
| T-TV-022 | SnapCollecting -> Headers INVALID | PASS |
| T-TV-023 | SnapDownloading -> Processing INVALID | PASS |
| T-TV-024 | SnapReady -> Headers INVALID | PASS |
| T-TV-025 | Bodies -> Headers INVALID | PASS |
| T-TV-026 | set_state warn-only mode (M1 behavior) | PASS |
| T-TV-027 | SnapCollecting -> SnapCollecting INVALID | PASS |
| T-TV-028 | SnapReady -> SnapReady INVALID | PASS |
| T-TV-029 | SnapReady -> SnapDownloading INVALID | PASS |
| T-TV-030 | SnapDownloading -> Synchronized INVALID | PASS |
| T-TV-031 | SnapDownloading -> SnapCollecting INVALID | PASS |

31 tests covering all valid transitions, all explicitly invalid transitions, self-transitions, forward paths, and the M1 warn-only enforcement mode.

### 5. Exploratory Testing Findings

| # | What Was Tried | Expected | Actual | Severity |
|---|----------------|----------|--------|----------|
| 1 | 9 write sites vs 8 enum variants -- is a variant missing? | Each write site maps to a unique variant | Correct: sites #5 (line 226) and #6 (line 232) both map to `ApplyFailuresSnapThreshold` (they are if/else branches of the same condition). The gap<=50 case at line 210 calls `signal_stuck_fork()`, not `needs_genesis_resync`, so no variant needed. 8 variants for 8 unique recovery reasons is correct. | N/A (no issue) |
| 2 | `request_genesis_resync()` called when `needs_genesis_resync` is already true | No panic, flag stays true | Test T-RG-008 confirms: second call passes all gates (flag-already-set is not a gate), flag remains true. No side effects. | N/A (no issue) |
| 3 | `is_valid_transition()` with self-transitions for SnapCollectingRoots and SnapReady | Should be INVALID (not in the defined matrix) | Correctly returns false for both. Tests T-TV-027 and T-TV-028 confirm. | N/A (no issue) |
| 4 | Could `request_genesis_resync()` have unintended side effects? | Only sets the flag | Confirmed: the method only writes `self.fork.needs_genesis_resync = true` on accept. No other state mutations. No timers started. No phases changed. | N/A (no issue) |

## Specs/Docs Drift

| File | Documented Behavior | Actual Behavior | Severity |
|------|-------------------|-----------------|----------|
| `specs/sync-recovery-architecture.md` section 2 (line 188) | `set_state()` shows `return;` (blocking invalid transitions) | Code implements warn-only (no return). This is correct for M1 per the migration path in section 5 step 2 (line 514), but section 2 shows the final M3 behavior without annotating it as future-state. | low |
| `specs/sync-recovery-architecture.md` section 2 (line 194) | Normal transition log shows `info!` level | Code keeps the pre-existing `debug!` level at mod.rs:1129. Not changed in M1. Spec shows end-state, not M1. | low |
| `specs/sync-recovery-architecture.md` section 1 (lines 65-72) | Write site #5 described as "3+ apply failures, small gap (<=50)" at line 226 with "Signal stuck fork instead" | Line 226 is actually the medium/large gap (>50) branch with snap enabled. The small-gap (<=50) branch that calls `signal_stuck_fork()` is at line 210. The spec table has incorrect line number/condition mapping for site #5. | low |
| `specs/sync-recovery-architecture.md` section 2 (lines 205-207) | Idle transition matrix lists only `DownloadingHeaders, SnapCollectingRoots, Idle` | Code uses `(Idle, _) => true` which allows Idle to transition to ANY state. The spec code snippet at line 441 says "Idle can go anywhere" matching the code, but the spec matrix is narrower. Internal spec inconsistency. | low |

## Blocking Issues (must fix before merge)

None.

## Non-Blocking Observations

- **OBS-001**: The spec's valid transition matrix (section 2, lines 205-207) lists Idle as transitioning to only 3 states, while the code and the spec's own code snippet use `(Idle, _) => true` allowing any state. The code behavior is more permissive but safe (Idle is the reset state). This is a spec matrix completeness issue, not a code bug. Recommend updating the spec matrix to say "Idle -> (any)" for consistency.

- **OBS-002**: The spec's write site table (section 1, line 71) incorrectly describes site #5 (block_lifecycle.rs:226) as "small gap (<=50)" with "Signal stuck fork instead." The actual line 226 is for the gap>50 branch with snap enabled. The signal_stuck_fork call for gap<=50 is at line 210. This is a spec documentation error. Recommend correcting the table in the spec.

- **OBS-003**: Section 2 of the spec shows `set_state()` with `return;` (hard blocking) and `info!` logging, which are M3 targets, not M1. While section 5 step 2 clarifies M1 is warn-only, the section 2 code snippet could confuse future developers. Recommend annotating the section 2 code as "M3 target behavior" or adding a note.

- **OBS-004**: The `request_genesis_resync()` method does not check whether `needs_genesis_resync` is already true before setting it again. This is harmless (idempotent flag set), but in M2+ when this replaces direct writes, the duplicate-set scenario should be logged at `debug!` level for observability, to help distinguish "first request honored" from "redundant request on already-flagged state."

## Modules Not Validated

- **M2 features (call site migration)**: Not implemented yet. M1 is additive-only scaffolding.
- **M3 features (hard-block enforcement)**: Not implemented yet. M1 uses warn-only mode.
- **Extended confirmed_height_floor checks in rollback/reorg paths**: Spec section 4 (lines 470-494) describes these for a future milestone. Confirmed NOT present in M1 code, as expected.

## Final Verdict

**PASS** -- M1 is correctly implemented as an additive-only change. All 120 tests pass. The RecoveryReason enum matches the spec exactly (8 variants for 8 unique recovery signals from 9 write sites). The request_genesis_resync() method implements all 5 gates in the correct order with appropriate boundary behavior. The is_valid_transition() method covers all 14 valid transitions and correctly rejects invalid ones. The set_state() integration is correctly in warn-only mode for M1. No existing behavior was changed. No production call sites were modified. Build and clippy are clean. Approved for review.
