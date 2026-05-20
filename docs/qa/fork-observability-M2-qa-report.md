<!-- OUTPUT CONTRACT: N/A — QA report -->

# M2 QA Report -- Fork-Diagnostic Observability

## Verdict: NEEDS-REWORK

Two Must requirements (EMIT-006, EMIT-007) have type-level tests but NO production emit wiring. The traceability matrix maps them to tests that verify struct construction, not integration.

## Acceptance Criteria (REQ-by-REQ)

| Requirement | Priority | Has Tests | Tests Pass | Acceptance Met | Notes |
|---|---|---|---|---|---|
| EMIT-001 | Must | Yes (7) | Yes | Yes | `apply_block/diagnostics.rs:41` emits BlockApplied with all fields |
| EMIT-002 | Must | Yes (1) | Yes | Yes | `apply_block/diagnostics.rs:9` emits BlockRejected on validation error |
| EMIT-003 | Must | Yes (4) | Yes | Yes | `block_handling.rs:168,201,262,311` emit ForkBlockReceived at all 4 dispatch arms |
| EMIT-004 | Must | Yes (2) | Yes | Yes | `rollback.rs:97` emits RollbackStarted with from/to/trigger/depth |
| EMIT-005 | Must | Yes (1) | Yes | Yes | `rollback.rs:318` emits RollbackCompleted with `caused_by_event_id` linking |
| EMIT-006 | Must | Yes (1) | Yes | **NO** | Test `test_reorg_executed_event_structure` validates TYPE only. No emit in `execute_reorg()` or `fork_recovery.rs` |
| EMIT-007 | Must | Yes (1) | Yes | **NO** | Test `test_recovery_classify_event_has_all_11_fields` validates TYPE only. No emit in `periodic.rs` |
| EMIT-008 | Should | No | N/A | No | Snap sync events not implemented |
| EMIT-009 | Should | No | N/A | No | Chain break events not implemented |
| EMIT-010 | Should | Yes (2) | Yes | Partial | CorrelationKey type exists; HeightOccupied arm populates it; other arms pass None |
| EMIT-011/O4 | Must | Yes (4) | Yes | Yes | `Option<BlockProvenance>` on apply_block, 8 call sites updated |
| SEC-001 | Must | Yes (1) | Yes | Yes | PII grep: zero IpAddr/SocketAddr in emit sites |
| SEC-005 | Must | Yes (1) | Yes | Yes | Default-ON: production Node::new() opens ledger, spawns tasks |
| SEC-006 | Must | Yes (2) | Yes | Yes | No activation height, no HardForkSchedule entry |

## DO-NOT-MODIFY Audit

**PASS**. `git diff bccb1bdf..251f5d73 --stat` shows zero changes to:
- `crates/core/src/consensus.rs`
- `crates/core/src/network_params/defaults.rs`
- `crates/storage/src/snapshot.rs`
- `crates/core/src/validation/*`

apply_block changes: only added `provenance` parameter, `validation_start` timer, error-wrapping for emit-on-reject, and emit calls after success. No branches, conditionals, return values, or error types changed.

## Genesis-Mismatch Guard Decision

**Recommendation: ACCEPT as defense-in-depth.**

Rationale: `classify_gossip_block()` (line 56) returns `ExtendsTip` when `prev_hash == best_hash` BEFORE checking genesis_hash (line 61). A block from a different chain whose prev_hash collides (practically impossible but structurally possible) would bypass the classifier's genesis check. The downstream `validate_header()` at `validation/block.rs:18` DOES catch this, so the guard is redundant.

However: the guard is a legitimate short-circuit that avoids calling `apply_block` entirely for wrong-chain blocks, saving validation work. Risk is LOW (redundant, not conflicting). The existing validation would produce a `block_rejected` event anyway. The guard produces a `fork_block_received(Rejected)` event which is arguably more informative for diagnostics.

**Impact**: None. The guard cannot cause consensus divergence (it only adds an early return before validation that validation would also reject). It does NOT change the set of blocks that get applied.

**Action for reviewer**: Confirm acceptance or request refactor to move the genesis check into `classify_gossip_block()` (line 56-57 should check genesis before returning ExtendsTip).

## Architect Decisions (O4, C1, C4, C5)

| Decision | Status | Evidence |
|---|---|---|
| O4 (explicit provenance param) | PASS | `apply_block/mod.rs:42` signature, 8 call sites (1 Some, 7 None) |
| C1 (async-default emit) | PASS | `emitter.rs:143` AsyncChannelEmitter with Mutex<VecDeque>, bounded(1024), drop-oldest at line 176, dropped_count at line 166 |
| C4 (emit at classify dispatch) | PASS | 4 dispatch arms emit at `block_handling.rs:168,201,262,311` |
| C5 (11-field RecoveryContext) | TYPE PASS, INTEGRATION FAIL | EventPayload::RecoveryClassifyCall has 11 fields, but no emit call exists in periodic.rs |

## Performance

**PASS**. The hot-path emit uses `Mutex<VecDeque>::lock()` (nanosecond-scale, uncontended) with no serialization or disk I/O. The mutex is independent of all consensus state locks. Serialization + RocksDB write happens in the background writer task. Writer drains in batches of 16 with 100ms polling.

## PII

**PASS**. Zero occurrences of IpAddr, SocketAddr, or IP literals in any emit site. Only PeerId (libp2p base58) used.

## Modular Discipline

| File | Lines | Status |
|---|---|---|
| diagnostic_writer.rs | 132 | OK |
| diagnostics_pruner.rs | 83 | OK |
| apply_block/diagnostics.rs | 77 | OK |
| apply_block/mod.rs | 504 | MARGINAL (4 lines over 500) |
| block_handling.rs | 1011 | OVER (was already large pre-M2) |
| rollback.rs | 339 | OK |
| init.rs | 1431 | OVER (was already large pre-M2) |
| mod.rs | 408 | OK |

block_handling.rs and init.rs were already over budget before M2. M2 added ~150 lines to block_handling and ~50 to init.

## Exploratory Testing

1. **Shutdown with queued events**: Writer task calls `drain_remaining()` on shutdown signal (`diagnostic_writer.rs:51`), which loops `try_recv()` until empty. Events are NOT lost.

2. **RocksDB write failure in writer**: Caught by `warn!` at line 78; event is skipped, task continues. Does NOT crash or propagate.

3. **Genesis-mismatch guard**: A block with mismatched genesis reaching ExtendsTip arm would hit line 433, emit ForkBlockReceived(Rejected), and return Ok(()) without calling apply_block. Correct behavior.

4. **DOLI_DIAG_RETENTION_DAYS=0 / MAX_EVENTS=0**: Both produce retention_secs=0 / max_events=0, causing the pruner to delete ALL non-pinned events. No clamping, no panic. Deterministic but aggressive. LOW severity.

## Regression

**PASS**. `cargo test -p storage -p doli-node` -- all test suites pass, zero failures. 3 e2e tests pass. 19 emit tests pass. 10 writer/pruner tests pass. All pre-existing tests (fork_recovery, economic_sim, epoch_state, etc.) pass.

## Issues Found

1. **[BLOCKER] EMIT-006 not wired**: `execute_reorg()` at `block_handling.rs:559` and fork_recovery.rs apply paths have NO `reorg_executed` emit call. Test `test_reorg_executed_event_structure` validates struct shape only. Must requirement unmet.

2. **[BLOCKER] EMIT-007 not wired**: `periodic.rs` recovery classify dispatch has NO `recovery_classify_call` emit call. Test `test_recovery_classify_event_has_all_11_fields` validates struct shape only. Must requirement unmet.

3. **[LOW] EMIT-008/009 not implemented**: Snap sync and chain break events not emitted. Should priority -- non-blocking.

4. **[LOW] CorrelationKey sparse**: Only HeightOccupied arm populates CorrelationKey; other fork arms pass None. Should priority requirement partially met.

5. **[LOW] apply_block/mod.rs at 504 lines**: 4 lines over 500-line budget. Trivially fixable.

## Action Items for Reviewer

- **Genesis-mismatch guard** (`block_handling.rs:433`): ACCEPT as defense-in-depth or REFACTOR into classify_gossip_block. See rationale above.
- **EMIT-006 and EMIT-007**: These are BLOCKING. The developer must add emit calls in execute_reorg/fork_recovery and periodic.rs before merge.
- **Traceability misleading**: The milestone-progress matrix maps EMIT-006/007 to type-only tests. Update after wiring is complete.
