# QA Report — INC-I-034 · M-RC9 (silent `vec![]` fail-fast fix)

**Date:** 2026-04-16
**Branch:** `synmgrefactor`
**Incident:** INC-I-034 (live mainnet cascade 2026-04-16, Santiago/ivan/N5 rejected EpochReward blocks)
**Milestone:** M-RC9
**Requirement anchor:** REQ-REDESIGN-002 row 9 (specs/scheduler-state-architecture.md line 273)
**File under test:** `bins/node/src/node/rewards.rs` — `Node::calculate_epoch_rewards`
**Test contract:** `bins/node/tests/m_rc9_silent_vec_regression.rs`

## Scope Validated
- Fail-fast behavior of `calculate_epoch_rewards` on an incomplete block_store within the epoch scan window.
- Happy-path regression (complete block_store) preserves byte-identical qualifier output.
- All adjacent test surfaces (node lib, epoch reward explicit inputs, fork recovery, checkpoint rotation, large-scale network).
- Build/lint/format gates.
- Surgical quality of the diff vs HEAD.
- Caller interactions (production/assembly.rs, validation_checks.rs).

Out of scope: testnet/mainnet deployment, signature/encoding-level protocol spec review, epoch_state_regression.rs (pre-existing compile errors — see Non-Blocking).

## Summary

**Verdict: APPROVE.** All 9 acceptance gates pass. The adversarial test (FAIL→PASS) verifies the M-RC9 fix prevents the silent subset output that caused the 2026-04-16 cascade. The fix is surgical (+74/−12 lines, single function touched, no refactoring). REQ-REDESIGN-002 row 9 is met via the fail-fast branch of the contract; the snapshot branch was deliberately not chosen and the developer's changelog documents why (attestation_accum not populated by `put_block_canonical` — snapshot path would have passed unit tests but been fragile in the field).

## System Entrypoint

`doli-node` is a library + test binary for this milestone; no running node required. The fix is localized to one function; validation is via:

- `cargo test -p doli-node --test m_rc9_silent_vec_regression` (primary)
- Adjacent suites (lib, epoch_reward_explicit_inputs, fork_recovery, checkpoint_rotation, test_network)
- `cargo build --release -p doli-node`, `cargo clippy -p doli-node -- -D warnings`, `cargo fmt --check`

## Traceability Matrix Status

| Requirement | Priority | Has Tests | Tests Pass | Acceptance Met | Notes |
|---|---|---|---|---|---|
| REQ-REDESIGN-002 row 9 — silent `vec![]` deleted at `rewards.rs:55-63`; compute refuses to proceed on incomplete block_store OR uses persisted qualifier snapshot | Must | Yes (3 tests in `m_rc9_silent_vec_regression.rs` covering P1, P2, P5) | 3/3 PASS | Yes — fail-fast branch of contract | Snapshot branch not chosen; rationale documented |
| REQ-REDESIGN-001 — byte-identical HAPPY-path output preserved | Must | Yes (`test_regression_complete_store_all_producers_qualify`) | 1/1 PASS | Yes | O1/O2/O3/O4 all assertions pass |

### Gaps Found
- Epoch 0 + gap behavior is not unit-tested. The code structure (short-circuit at `if epoch > 0 && ...`) is correct by construction and the genesis branch auto-qualifies all producers without reading `attested_minutes`, but the cell is not under an assertion. Recommendation is LOW — test writer can add this in a follow-up.

## Acceptance Criteria Results

### REQ-REDESIGN-002 row 9 (Must)
- [x] **RC#9 structural test — adversarial gap:** `test_adversarial_gap_in_middle_must_not_silently_undercount` PASSES by fail-fast (len=0, sum=0 per the contract's `is_fail_fast` branch). The silent subset output pattern that caused the 2026-04-16 cascade is now rejected at construction time.
- [x] **RC#9 santiago cascade replay:** `test_santiago_cascade_replay_mainnet_scale` PASSES with 37 producers and 11 gap heights (`h=45..=55`) — fail-fast empty Vec returned instead of the silent subset.
- [x] **RC#9 regression (happy path):** `test_regression_complete_store_all_producers_qualify` PASSES — full-store path still awards all 6 producers with sum == pool_total and deterministic back-to-back calls.

### REQ-REDESIGN-001 (Must, byte-identical HAPPY path)
- [x] O1 (len), O2 (pkh set), O3 (sum), O4 (determinism) all assertions pass on the happy-path regression anchor. Byte-identity of the returned `Vec<(u64, Hash)>` is preserved when `missing_block_count == 0 && silent_bitfield_count == 0`.

## End-to-End Flow Results

The fix affects one function on one process (local block producer). No multi-module end-to-end flow is stood up. The adjacent suites (`fork_recovery`, `test_network`, `checkpoint_rotation`) validate that the larger block-production / fork / snap-sync machinery still converges with the fail-fast branch in place.

| Flow | Steps | Result | Notes |
|---|---|---|---|
| Block producer at epoch boundary — complete store | `try_produce_block` → `calculate_epoch_rewards(completed_epoch)` → non-empty Vec → `Transaction::new_epoch_reward_coinbase` included | PASS | Regression anchor (test A) verifies |
| Block producer at epoch boundary — incomplete store | `try_produce_block` → `calculate_epoch_rewards(completed_epoch)` → empty Vec → `debug!("No qualified producers ... — pool accumulates")` → EpochReward tx NOT included | PASS | Matches `assembly.rs:87-92` existing Tier 3 path |
| Gossip validator — complete peer validates own block | `validate_block_economics` → `calculate_epoch_rewards(completed_epoch)` recomputes, expected == actual | PASS | 11/11 fork_recovery tests pass |
| Gossip validator — incomplete peer validates peer block | `validate_block_economics` on incomplete-store node → `calculate_epoch_rewards` returns empty → expected(0) != actual(N) → block REJECTED at line 591 | ACCEPTABLE | Same reject behavior as pre-fix (pre-fix would also mismatch, just at a silently-wrong count). Recovery path is block backfill, same as today. |

## Exploratory Testing Findings

### Finding #1 — Epoch 0 fail-fast exemption
- **Tried**: trace the control flow for `calculate_epoch_rewards(0)` when `missing_block_count > 0`.
- **Expected**: exemption works because epoch 0 auto-qualifies without reading `attested_minutes`.
- **Actual**: Line 126 short-circuits the fail-fast guard (`if epoch > 0 && ...`). Line 150-152 auto-qualifies all producers. **Correct.** No unit test covers this path but it is correct by structure.
- **Severity**: LOW (observation, not a bug)
- **Reproducible**: static analysis only

### Finding #2 — Err vs None observability
- **Tried**: verify `Err(e)` from `block_store.get_block_by_height(h)` is distinguishable in logs from `Ok(None)`.
- **Expected**: `Err(e)` logs at `warn!` with the error, `Ok(None)` is counted but not individually logged; aggregate `error!` fires at the end.
- **Actual**: Matches expectation. `warn!` at line 111 (`[ECON_EPOCH_DISTRIBUTION] block_store read error at height={} ... {} — treating as missing`) gives ops the per-height signal; aggregate `error!` at line 127 (`incomplete_block_store: gap_count=N silent_bitfield_count=M — refusing to compute ... Pool accumulates to next epoch.`) gives the cumulative signal. Aggregate does NOT split `gap_count` into `None` vs `Err` subcomponents — both bucket into `missing_block_count`.
- **Severity**: LOW (minor observability gap, not load-bearing)
- **Reproducible**: yes, by static reading

### Finding #3 — Caller retry-loop risk
- **Tried**: read all callers of `calculate_epoch_rewards` to confirm empty Vec does not cause a retry loop.
- **Expected**: empty Vec is already the agreed no-distribution shape (rewards.rs:201, 226, 256); callers accept it as-is.
- **Actual**:
  - `production/assembly.rs:52` — checks `if !epoch_outputs.is_empty()` then either adds EpochReward tx OR logs `debug!("No qualified producers ... — pool accumulates to next epoch")`. **No retry.**
  - `validation_checks.rs:567,648` — compare expected vs actual. On incomplete-store node, expected=empty vs actual=N outputs → block rejected via `anyhow::bail!`. No retry inside that function. Recovery is upstream (block backfill).
- **Severity**: LOW (no risk identified)
- **Reproducible**: static analysis

### Finding #4 — Log level appropriate for alerting
- **Tried**: confirm fail-fast emits at `error!` (not `info`) so ops alerts trigger.
- **Expected**: `error!` macro.
- **Actual**: `rewards.rs:127` uses `error!`. Per-iteration Err uses `warn!`. **Correct.**
- **Severity**: PASS

### Finding #5 — Log tags consistent with existing econ alerts
- **Tried**: verify `[ECON_EPOCH_DISTRIBUTION]` prefix is consistent with the other validation alerts.
- **Expected**: match existing alerts (validation_checks.rs already uses `[ECON_EPOCH_DISTRIBUTION]`, `[ECON_EPOCH_OVERFLOW]`, `[ECON_EPOCH_NO_INPUTS]`, `[ECON_EPOCH_INPUTS_MISMATCH]`, `[ECON_EPOCH_PRE_INPUTS]`, `[ECON_EPOCH_MISSING]`).
- **Actual**: Consistent. The new `incomplete_block_store` alert uses the same umbrella prefix as the existing `EpochReward mismatch` alert — ops dashboards watching `[ECON_EPOCH_DISTRIBUTION]` will catch both.
- **Severity**: PASS

### Finding #6 — Validator-side behavior on incomplete-store peer
- **Tried**: trace: does an incomplete-store validator now reject valid EpochReward blocks from peers with complete stores?
- **Expected**: yes (by the symmetry of expected-vs-actual comparison), but this is not a regression relative to HEAD-without-fix.
- **Actual**: Pre-fix: incomplete-store peer computes silently-wrong subset (e.g. 31 outputs). Peer-block has 37 outputs. `expected_sorted != actual_sorted` → reject. Post-fix: incomplete-store peer computes empty. Peer-block has 37 outputs. `expected_sorted != actual_sorted` → reject. **Identical behavior.** The fix does NOT make validation stricter on the incomplete-store side; it only prevents that peer from producing a wrong block.
- **Severity**: PASS (not a regression)

### Finding #7 — Determinism preserved under gap
- **Tried**: `test_adversarial_gap_in_middle` and `test_santiago_cascade_replay` both call the function twice and require identical output (test B line 560-564).
- **Actual**: O4 determinism assertion passes under gap.
- **Severity**: PASS

## Failure Mode Validation

| Failure Scenario | Triggered | Detected | Recovered | Degraded OK | Notes |
|---|---|---|---|---|---|
| Block missing at epoch height (`Ok(None)`) | Yes (test B drops 7 heights, test C drops 11 heights) | Yes (`missing_block_count++`, aggregate `error!`) | N/A per-call (caller-side recovery = backfill) | Yes (empty Vec → "pool accumulates to next epoch" — matches existing Tier 3 shape) | Core M-RC9 scenario |
| Block present but body bitfield empty post-activation (`silent_bitfield_count`) | Not explicitly triggered; code path exercised by static inspection. `BITFIELD_BODY_ACTIVATION_HEIGHT` is 0 on HEAD so this branch is only reachable with a synthetic header-only block | Yes (counter incremented before `vec![]`) | Same as above | Same as above | The test file documents P6 as dead-on-HEAD (MATRIX line 54). Covered by path P2 generality. |
| Block_store read error (`Err(e)`) | Not triggered (requires disk/RocksDB failure injection — untestable in this environment) | Yes via `match` branch: `missing_block_count += 1` + per-iteration `warn!` | Same as above | Same | **Not Triggered (untestable in this environment)** — the static path is verified by reading the code. |
| Epoch 0 with gap | Not triggered as a unit test | Fail-fast guard is bypassed by `if epoch > 0 && ...`; epoch-0 branch auto-qualifies all producers | N/A — genesis epoch has no attestation data | Yes | **Not Triggered (structural guarantee verified by inspection)** |

## Security Validation

No `security-probe-report.md` present under `docs/.workflow/`. M-RC9 has no new external-data ingestion surface — it tightens an internal computation. No injection/auth/XSS vectors apply. The one relevant trust-boundary concern is the interaction with validator path (Finding #6), which is not a regression.

| Attack Surface | Test Performed | Result | Notes |
|---|---|---|---|
| Malicious peer sends EpochReward block with ragged (silently-wrong) outputs | Implicit via `validate_block_economics` expected-vs-actual comparison | PASS | Mismatch rejected; same path pre- and post-fix |
| Producer with a gap signs a block that a clean network would reject | Adversarial test B replicates the Santiago pattern | PASS | Fail-fast prevents the signature from being created in the first place |
| DoS by forcing a producer into permanent fail-fast (feeding it bad peers to keep the gap) | Out of scope — operational issue, not consensus | Out of Scope | Recovery is the existing block backfill path. `error!` log ensures ops visibility |

## Specs/Docs Drift

| File | Documented Behavior | Actual Behavior | Severity |
|---|---|---|---|
| `specs/scheduler-state-architecture.md:273` | "Silent `vec![]` at `rewards.rs:55-63` deleted (S-11); attestation scan moves to epoch boundary; `calculate_epoch_rewards` reads `qualifier_set` from `EpochSnapshot`" | M-RC9 implements the fail-fast branch of REQ-REDESIGN-002 row 9 — the silent `vec![]` is tracked (not literally deleted) and produces empty output; the `EpochSnapshot.qualifier_set` read path is NOT implemented in M-RC9 | LOW — both branches are acceptance-contract-valid per REQ text "refuses to proceed on incomplete block_store OR uses persisted qualifier snapshot"; fail-fast is the explicit OR-branch. Spec language leans toward snapshot; recommend spec note that fail-fast is the shipping choice and attestation-snapshot is deferred. |
| `docs/bugfixes/inc-i-034-m-rc9-silent-vec-fix.md` | Documents fail-fast choice and cites Choice 2 (RUNTIME PERIODIC) rationale | Matches code exactly | PASS — developer's changelog is accurate |

## Code Quality Observations

- **Surgical diff:** +74/−12 net, single function (`calculate_epoch_rewards`) touched. No refactoring of surrounding code.
- **Doc block:** FAIL-FAST SEMANTICS block (lines 15-33) cites both INC-I-034 and 2026-04-16, and explains why the return type is unchanged.
- **Counter naming:** `missing_block_count` (covers `Ok(None)` + `Err`) and `silent_bitfield_count` (covers post-activation empty-body) are clearly named and domain-faithful.
- **Log idiom:** `error!` aggregate + `warn!` per-Err is appropriate (rare aggregate → ops alert, per-iteration → debugging signal).
- **Tag consistency:** `[ECON_EPOCH_DISTRIBUTION]` matches existing econ alerts in `validation_checks.rs`.
- **Epoch 0 exemption:** explicit `if epoch > 0 && ...` with inline comment citing line 150-158 for why it's safe.

## Test Run Outputs (quoted)

```
$ cargo test -p doli-node --test m_rc9_silent_vec_regression
running 3 tests
test test_santiago_cascade_replay_mainnet_scale ... ok
test test_regression_complete_store_all_producers_qualify ... ok
test test_adversarial_gap_in_middle_must_not_silently_undercount ... ok
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
```

```
$ cargo test -p doli-node --lib
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.08s
```

```
$ cargo test -p doli-node --test epoch_reward_explicit_inputs
test result: ok. 7 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

```
$ cargo test -p doli-node --test fork_recovery
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.14s
```

```
$ cargo test -p doli-node --test checkpoint_rotation   (bundled with test_network)
test result: ok. 16 passed; 0 failed; 12 ignored; 0 measured; 0 filtered out; finished in 175.64s
```

```
$ cargo test -p doli-node --test test_network
test result: ok. 13 passed; 0 failed; 12 ignored; 0 measured; 0 filtered out; finished in 179.67s
```

```
$ cargo build --release -p doli-node
    Finished `release` profile [optimized] target(s) in 1m 29s
```

```
$ cargo clippy -p doli-node -- -D warnings
    Finished `dev` profile [optimized + debuginfo] target(s) in 4.61s
```

```
$ cargo fmt --check
(no output — clean)
```

## Blocking Issues

None.

## Non-Blocking Observations

- **OBS-M-RC9-01 (LOW)**: `bins/node/tests/epoch_state_regression.rs` has 8 pre-existing compile errors (`node.best_hash()` method no longer exists; suggestion `best_height` but types differ — `best_hash` returned a `Hash`, `best_height` returns `u64`). Confirmed NOT caused by M-RC9 (developer's changelog Non-goals + git log for rewards.rs shows no touch to this file; the drift is from the EpochState refactor at `3d267217` / `42740269`). Flag for separate triage (scheduler-state refactor follow-up).
- **OBS-M-RC9-02 (LOW)**: `bins/node/src/node/rewards.rs` is 1162 lines, exceeding the 500-line modular coding policy (CLAUDE.md Rule 19). Not caused by M-RC9 — the file was already over budget before this milestone and M-RC9 added only ~62 net lines. Flag for separate refactor (the natural split is `calculate_epoch_rewards` + `rebuild_epoch_state_from_blocks` + `rebuild_producer_set_from_blocks` are each independently sizable).
- **OBS-M-RC9-03 (LOW)**: Aggregate `error!` at rewards.rs:127 folds `Ok(None)` and `Err(_)` into a single `gap_count` field. Ops diagnosing a real incident would benefit from seeing the split (e.g. `gap_count_none=3 gap_count_err=0`). Not blocking — the per-iteration `warn!` already distinguishes, but a single structured log line per failure is cleaner. Suggest enhancement in a future polish pass.
- **OBS-M-RC9-04 (LOW)**: Epoch 0 + gap behavior is structurally correct (guard at `if epoch > 0 && ...`, epoch-0 branch auto-qualifies all producers) but not covered by a unit test. Adding `test_epoch0_with_gap_still_auto_qualifies` would close the matrix cell at O(10) lines. Non-blocking.
- **OBS-M-RC9-05 (LOW)**: `specs/scheduler-state-architecture.md:273` language leans toward the snapshot branch of the contract. Recommend a one-line spec note that M-RC9 ships the fail-fast branch and the `EpochSnapshot.qualifier_set` path is deferred. See Specs/Docs Drift table above.

## Modules Not Validated (context scope)

- Mainnet/testnet deployment — explicitly out of scope per orchestrator constraints.
- `epoch_state_regression.rs` — cannot compile; tracked as OBS-M-RC9-01.
- Heavy stress suites (`test_network_10000_nodes`, `test_realistic_gossip_10k_clustered`, etc.) are `#[ignore]` by default and were not run. Not M-RC9-relevant.

## Final Verdict

**APPROVE.**

All 9 acceptance gates pass. REQ-REDESIGN-002 row 9 is satisfied via the fail-fast branch of its OR-contract. The adversarial test (Test B) transitioned from FAIL-on-HEAD to PASS-with-fix. Santiago cascade replay (Test C) PASS at 37-producer / 11-gap scale. The happy-path regression anchor (Test A) confirms byte-identical output is preserved on complete stores. The diff is surgical, the logging is appropriately tiered, and the epoch-0 exemption is correctly guarded. No blocking issues found. The 5 non-blocking observations are either pre-existing (OBS-01, OBS-02) or improvement suggestions (OBS-03, OBS-04, OBS-05).

Recommendation to orchestrator: proceed to reviewer. Defer OBS-01 (epoch_state_regression.rs compile errors) and OBS-02 (rewards.rs module size) to separate triage.
