# QA Report: INC-I-089 M1 — Producer Self-Fork on Restart (Startup Lockout)

## Scope Validated
- `crates/network/src/sync/manager/production_gate.rs` — `engage_post_restart_lockout()` + `POST_RESTART_LOCKOUT_SLOTS` constant
- `bins/node/src/node/init.rs:729-744` — call site with `best_height > 0` guard
- `crates/network/src/sync/manager/cleanup.rs:557-571` — 60s safety timeout (shared with snap-sync)
- `crates/network/src/sync/manager/snap_sync.rs:265-285` — unchanged snap-sync SET path
- `bins/node/tests/inc_i_089_startup_lockout.rs` — all 7 integration tests

## Summary
**PASS** — All 6 requirements verified (5 Must, 1 Should). All 371 tests pass (7 INC-I-089 integration + 336 network lib + 28 node lib). No consensus files touched. The fix is a minimal, additive 3-line change in `init.rs` that reuses the existing `AwaitingCanonicalBlock` gate mechanism.

## System Entrypoint
Tests run via: `cargo test -p doli-node --test inc_i_089_startup_lockout`
Supporting: `cargo test -p network --lib`, `cargo test -p doli-node --lib`

## Traceability Matrix Status

| Requirement ID | Priority | Has Tests | Tests Pass | Acceptance Met | Notes |
|---|---|---|---|---|---|
| REQ-INC-089-001 | Must | Yes | Yes | Yes | `test_startup_lockout_must_engage_on_restart_with_height`, `test_can_produce_must_block_on_restart`, `test_unlock_via_gossip_clears_gate` |
| REQ-INC-089-002 | Must | Yes (shared) | Yes | Yes | 60s timeout at cleanup.rs:563-569 shared with snap-sync. Existing tests at manager/tests.rs:1304,1334 cover the timeout mechanism. POST_RESTART_LOCKOUT_SLOTS=3 constant at production_gate.rs:19 |
| REQ-INC-089-003 | Must | Yes | Yes | Yes | `test_snap_sync_gate_regression`. snap_sync.rs:272-275 unchanged (verified via Read) |
| REQ-INC-089-004 | Must | Yes | Yes | Yes | `test_fresh_genesis_no_lockout`, `test_node_at_genesis_no_lockout`. Guard at init.rs:731 (`if state.best_height > 0`) |
| REQ-INC-089-005 | Should | N/A (log) | N/A | Yes | Engage log at production_gate.rs:311-314 `[STARTUP_GATE]`. Clear logs at line 325 (gossip) and cleanup.rs:565-568 (timeout) |
| REQ-INC-089-006 | Must | N/A | N/A | Yes | `git diff --stat crates/core/src/ crates/updater/src/` shows zero changes. Only init.rs + production_gate.rs modified |

### Gaps Found
- None. All Must and Should requirements have corresponding tests or verification paths.

## Acceptance Criteria Results

### Must Requirements

#### REQ-INC-089-001: Production gate blocks on normal restart
- [x] On restart with height > 0, production blocked — `test_startup_lockout_must_engage_on_restart_with_height` PASSES
- [x] Cleared by first gossip block that extends local tip — `clear_awaiting_canonical_block()` called at `block_handling.rs:501`
- [x] No self-fork during lockout — `test_can_produce_must_block_on_restart` returns `BlockedAwaitingCanonicalBlock`

#### REQ-INC-089-002: Safety timer unlocks for single-producer
- [x] If no gossip arrives within timeout, production resumes — `cleanup.rs:564` fires at 60s
- [x] N is named constant — `POST_RESTART_LOCKOUT_SLOTS: u32 = 3` at production_gate.rs:19
- [x] Existing tests prove timeout mechanism — `test_awaiting_canonical_block_has_timeout` (tests.rs:1304), `test_awaiting_canonical_block_no_premature_timeout` (tests.rs:1334)

#### REQ-INC-089-003: Snap-sync gate regression-free
- [x] All existing snap-sync gate tests pass — 336 network lib tests pass
- [x] AwaitingCanonicalBlock still set after snap sync — snap_sync.rs:272-275 UNCHANGED
- [x] 60s timeout still works — cleanup.rs:563-569 UNCHANGED

#### REQ-INC-089-004: Fresh genesis unaffected
- [x] New node at height=0 does NOT enter AwaitingCanonicalBlock — `test_fresh_genesis_no_lockout` PASSES
- [x] Bootstrap mode guards remain functional — `test_node_at_genesis_no_lockout` PASSES (real Node at genesis)
- [x] Guard verified: init.rs:731 `if state.best_height > 0`

#### REQ-INC-089-006: No protocol/consensus changes
- [x] No CURRENT_PROTOCOL_VERSION change — crates/core/src/ untouched
- [x] No EPOCH_STATE_FORMAT_VERSION change — crates/core/src/ untouched
- [x] No NetworkParams activation height — crates/core/src/ untouched
- [x] No HardForkSchedule entry — crates/updater/src/ untouched
- [x] No chainspec change — crates/core/src/ untouched

### Should Requirements

#### REQ-INC-089-005: Startup lockout logged
- [x] Log emitted when startup lockout engages — production_gate.rs:311-314: `[STARTUP_GATE] Production gated: awaiting first canonical gossip block (INC-I-089)`
- [x] Log emitted when lockout clears via gossip — production_gate.rs:325: `[SNAP_SYNC] Canonical gossip block received — production gate cleared`
- [x] Log emitted when lockout clears via timeout — cleanup.rs:565-568: `AwaitingCanonicalBlock timeout: 60s without canonical gossip block. Clearing gate...`

## Test Suite Results

| Suite | Command | Result |
|---|---|---|
| INC-I-089 integration | `cargo test -p doli-node --test inc_i_089_startup_lockout` | 7 passed, 0 failed |
| Network lib | `cargo test -p network --lib` | 336 passed, 0 failed, 1 ignored |
| Node lib | `cargo test -p doli-node --lib` | 28 passed, 0 failed |
| **Total** | | **371 passed, 0 failed** |

## Exploratory Testing Findings

| # | What Was Tried | Expected | Actual | Severity |
|---|---|---|---|---|
| 1 | Check if `reset_sync_for_rollback()` can prematurely clear lockout | Could bypass gate | Semantically correct: rollback requires peer headers (proves connectivity), then node is on canonical chain. Not a bypass — alternative alignment proof. | N/A (not a bug) |
| 2 | Check if `reset_sync_after_successful_reorg()` can clear lockout | Could bypass gate | Semantically correct: successful reorg proves canonical alignment via fork choice. | N/A (not a bug) |
| 3 | Check if no-peer scenario deadlocks | Node stuck forever | 60s cleanup timeout fires unconditionally based on elapsed time, independent of peer count. No deadlock. | N/A (safe) |
| 4 | Check if `block_lifecycle.rs:111` (PostRecoveryGrace block-count clear) could affect lockout | Could bypass gate | Only matches `PostRecoveryGrace` pattern, not `AwaitingCanonicalBlock`. Safe by Rust enum matching. | N/A (safe) |
| 5 | Check `update_local_tip` at height=0 | Gate should not engage | Guard at init.rs:731 prevents both `update_local_tip` and `engage_post_restart_lockout` when `best_height == 0`. Safe. | N/A (safe) |

## Failure Mode Validation

| Failure Scenario | Triggered | Detected | Recovered | Degraded OK | Notes |
|---|---|---|---|---|---|
| Restart with stale tip | Yes (test) | Yes (gate blocks) | Yes (gossip clears) | Yes | Core fix target |
| No peers ever connect | Yes (logic trace) | Yes (gate blocks) | Yes (60s timeout) | Yes | Safety timer |
| Snap sync completion | Yes (existing test) | Yes (gate blocks) | Yes (gossip/timeout) | Yes | Regression-free |

## Security Validation
Not applicable — this fix is internal to the production gate (no external data, no user input, no network protocol change).

## Constraint Compliance

| Constraint | Verified | Evidence |
|---|---|---|
| No CURRENT_PROTOCOL_VERSION bump | Yes | crates/core/src/ has 0 modifications |
| No EPOCH_STATE_FORMAT_VERSION bump | Yes | crates/core/src/ has 0 modifications |
| No NetworkParams activation height | Yes | crates/core/src/ has 0 modifications |
| No HardForkSchedule entry | Yes | crates/updater/src/ has 0 modifications |
| Rolling-deploy safe | Yes | No consensus/content changes; local gate only |
| POST_RESTART_LOCKOUT_SLOTS named constant | Yes | production_gate.rs:19 |

## Specs/Docs Drift
| File | Documented Behavior | Actual Behavior | Severity |
|------|-------------------|-----------------|----------|
| production_gate.rs:325 | Clear log says `[SNAP_SYNC]` | Gate is now shared (snap-sync + startup). Log prefix doesn't distinguish which path set the gate. | low |

## Blocking Issues
None.

## Non-Blocking Observations
- **[OBS-001]**: `production_gate.rs:325` — The clear log message says `[SNAP_SYNC]` but the gate is now shared between snap-sync and startup lockout. Consider updating to a generic prefix like `[PRODUCTION_GATE]` for clarity in log analysis. Non-blocking because both paths use the same mechanism and the engage log distinguishes them.

## Final Verdict

**PASS** — All Must and Should requirements met. No blocking issues. 371 tests pass. The fix is minimal (additive 3 LOC in init.rs + 1 constant + 1 method in production_gate.rs), reuses proven existing mechanisms (AwaitingCanonicalBlock + 60s timeout), and introduces no consensus or protocol changes. Approved for review.
