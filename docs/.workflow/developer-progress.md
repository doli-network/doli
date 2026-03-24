# Developer Progress: Sync Recovery Redesign

## Status: M4 (State Collapse) Complete

### M4: Collapse SyncState from 8 Variants to 3 (2026-03-24)

**Branch**: `fix/sync-state-explosion-root-causes`

#### What was implemented:

1. **SyncState collapsed from 8 to 3 variants** (mod.rs)
   - Old: Idle, DownloadingHeaders{...}, DownloadingBodies{...}, Processing{...}, Synchronized, SnapCollectingRoots{...}, SnapDownloading{...}, SnapReady{...}
   - New: Idle, Syncing { phase: SyncPhase, started_at: Instant }, Synchronized
   - Data that was embedded in variants moved to SyncPipelineData enum

2. **SyncPhase diagnostic label enum** (mod.rs)
   - 5 variants: DownloadingHeaders, DownloadingBodies, ProcessingBlocks, SnapCollecting, SnapDownloading
   - NOT a state machine — just tracks what the sync pipeline is doing

3. **SyncPipelineData enum** (mod.rs)
   - 7 variants: None, Headers{...}, Bodies{...}, Processing{...}, SnapCollecting{...}, SnapDownloading{...}, SnapReady{...}
   - Carries operational data previously embedded in SyncState variants
   - Added `pipeline_data` field to SyncManager

4. **set_syncing() helper** (mod.rs)
   - Atomically sets state + pipeline_data
   - Preserves started_at across phase changes within a sync cycle

5. **is_valid_transition() simplified** (mod.rs)
   - With 3 states, all 3x3 transitions are valid — always returns true

6. **Source files updated** (sync_engine.rs, snap_sync.rs, cleanup.rs, block_lifecycle.rs)
   - All state reads/writes converted to new model
   - State checks use `self.state`, data access uses `self.pipeline_data`

7. **External callers updated**
   - event_loop.rs: `state().is_snap_syncing()` -> `is_snap_syncing()`
   - Re-exports added in sync/mod.rs and lib.rs

8. **Tests fully updated** (tests.rs)
   - All 226 tests pass (was 148 before test expansion)
   - All old SyncState variant references converted to new 3-state model
   - transition_validation_tests rewritten for 3-state model (all transitions valid)
   - regression_tests: state assignments now set both state and pipeline_data
   - State matches: `SyncState::Processing { .. }` -> `SyncState::Syncing { phase: SyncPhase::ProcessingBlocks, .. }`
   - Data extraction: `SyncState::DownloadingHeaders { headers_count, .. }` -> `SyncPipelineData::Headers { headers_count, .. }`

#### Validation:
- `cargo build --release` -- PASS
- `cargo clippy -- -D warnings` -- CLEAN
- `cargo fmt --check` -- CLEAN
- `cargo test -p network` -- 226 passed, 0 failed

---

### M3: Hard Enforcement + ForkAction Wiring (2026-03-23)

**Commit**: `2320a24` on `fix/sync-state-explosion-root-causes`

#### What was implemented:

1. **Step 5: Hard enforcement in set_state()** (mod.rs)
   - Changed from warn-only (M1) to hard-block: `return;` on invalid transitions
   - Updated log message to include "BLOCKED" for observability
   - Updated test T-TV-026 from `test_set_state_warn_only_allows_invalid_transition`
     to `test_set_state_hard_blocks_invalid_transition` — now asserts state remains
     unchanged after invalid transition attempt

2. **Step 6: Wire ForkAction::recommend_action()** (rollback.rs, block_lifecycle.rs, mod.rs)
   - Removed `#[allow(dead_code)]` from `ForkAction` enum and `recommend_action()` method
   - Added `recommend_fork_action()` delegate on SyncManager (block_lifecycle.rs)
   - Exported `ForkAction` through sync/mod.rs and lib.rs
   - Imported `ForkAction` in node/mod.rs
   - Refactored `resolve_shallow_fork()` to call `recommend_fork_action()` and match
     on returned `ForkAction` variant (RollbackOne, StartForkSync, NeedsGenesisResync, None)
   - Death spiral check retained as Node-level policy before recommend_fork_action() call

3. **Step 6b: Reorg floor check** (block_handling.rs)
   - Added `confirmed_height_floor()` check in `execute_reorg()` after target_height computation
   - If `floor > 0 && target_height < floor`, reorg is refused with warning
   - Closes the last gap in monotonic progress enforcement (INC-I-005)

4. **Formatting fix** (tests.rs)
   - Fixed pre-existing formatting issue at line 2140 in regression_tests

#### Validation:
- `cargo build` -- PASS (full workspace)
- `cargo clippy -- -D warnings` -- CLEAN (network + doli-node)
- `cargo fmt --check` -- CLEAN
- `cargo test --package network --lib sync::manager::tests` -- 148 passed, 0 failed
- `cargo test --package network --lib` -- 243 passed, 0 failed
- `cargo test --package doli-core --lib` -- 590 passed, 0 failed

---

### M2: Site Migration + Monotonic Floor Extension (2026-03-23)

**Commit**: `db8283d` on `fix/sync-state-explosion-root-causes`

---

### M1: Recovery Gate + Transition Validation (2026-03-23)

**Commit**: `dc4bb7c` on `fix/sync-state-explosion-root-causes`

#### What was implemented:

1. **RecoveryReason enum** (mod.rs)
   - 8 variants covering all 9 `needs_genesis_resync = true` write sites
   - Derives Clone + Debug for test assertions and logging

2. **request_genesis_resync() method** (production_gate.rs)
   - Central gate with 5 checks: height floor, concurrent recovery, rate limiting, snap availability, snap attempts
   - Returns bool (true = honored, false = refused)
   - Follows signal_stuck_fork() pattern (method on SyncManager, not a coordinator struct)

3. **is_valid_transition() method** (mod.rs)
   - Match-based transition matrix covering all 8 SyncState variants
   - Idle = universal source/target, forward-only paths for sync pipeline
   - Self-transitions allowed for: DownloadingBodies, Processing, Synchronized, SnapDownloading

4. **set_state() updated** (mod.rs)
   - Calls is_valid_transition() before mutation
   - WARN-ONLY in M1: logs invalid transitions but still executes them
   - Hard enforcement completed in M3

5. **label_for() static method** (mod.rs)
   - Refactored state_label() to delegate to static label_for(&SyncState)
   - Needed by set_state() to log the new state label before mutation

6. **Test gates removed** (tests.rs)
   - Removed both `#[cfg(feature = "m1_recovery_gate")]` gates
   - All 120 sync manager tests pass (46 existing + 13 regression + 13 recovery gate + 31 transition validation + 17 other)
