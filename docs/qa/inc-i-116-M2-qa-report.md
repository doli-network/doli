# QA Report: INC-I-116 Milestone M2 -- Epoch-Boundary Liveness Prune (Extract + Decode-List Fix)

## Scope Validated
- `crates/core/src/epoch_state/mod.rs` -- new `compute_live_producer_list()` extraction + refactored `derive_at_boundary()`
- `bins/node/src/node/rewards.rs` -- refactored floor logic + decode-list fix
- `crates/core/src/epoch_state/tests_m2.rs` -- 17 M2 tests
- EpochState struct (FILTER-07: no field changes)
- Protocol/format version constants (FILTER-07: no bumps)
- All 3 call sites of `EpochDerivationInput` (post_commit.rs, fork_recovery.rs, rewards.rs)

## Summary
**PASS** -- All acceptance criteria verified. The extraction is correct, the decode-list fix is properly gated, no EpochState struct changes, no version bumps. 39 epoch_state tests pass. Clippy clean. Full release build succeeds.

## System Entrypoint
- Build: `cargo build --release` (success, 1m 52s)
- Tests: `cargo test -p doli-core --lib -- epoch_state` (39 passed, 0 failed)
- Lint: `cargo clippy --workspace --all-targets -- -D warnings` (clean)
- Format: `cargo fmt --check` (clean)

## Traceability Matrix Status

| Requirement ID | Priority | Has Tests | Tests Pass | Acceptance Met | Notes |
|---|---|---|---|---|---|
| FILTER-08 (pre-activation bit-identity) | Must | Yes | Yes | Yes | `test_filter08_tier_cap_active_diverges` + `test_filter08_tier_cap_floor_passes` + `test_extract_equiv_pre_act_floor_fires` + `test_extract_equiv_pre_act_floor_passes` |
| FILTER-02/03 (post-activation canonical/rebuild parity) | Must | Yes | Yes | Yes | `test_extract_equiv_post_act_floor_fires` + `test_extract_equiv_post_act_floor_passes` |
| FILTER-07 (no EpochState changes, no version bump) | Must | N/A (structural) | N/A | Yes | EpochState struct unchanged (7 fields). CURRENT_PROTOCOL_VERSION=8, EPOCH_STATE_FORMAT_VERSION=1 -- both untouched. |
| Extraction Quality | Must | Yes | Yes | Yes | Pure function, no self/IO/logging/side-effects. Lives in epoch_state/mod.rs. Both derive_at_boundary and rewards.rs call it. |
| Decode-list Fix Quality | Must | Yes | Yes | Yes | Gated on epoch_prune_activation_height. Pre-activation: active.clone(). Post-activation: self.epoch_state.producer_list. Clear comment at rewards.rs:780-783. |

### Gaps Found
- None. All requirements have tests. All tests correspond to requirements.

## Acceptance Criteria Results

### Must Requirements

#### FILTER-08: Pre-activation bit-identity (REGRESSION)
- [x] Tests with >50 active producers exist (`test_filter08_tier_cap_active_diverges`: 60 active, 55 attested; `test_extract_equiv_tier_cap`: 60 active, 40 attested)
- [x] Floor-not-fired test exists (`test_filter08_tier_cap_floor_passes`: 55 active, 40 attested, 40 >= 55*2/3=36)
- [x] Pre-activation `compute_live_producer_list` output is byte-identical to `derive_at_boundary` output (verified by `assert_extraction_matches` helper in 7 equivalence tests)

#### FILTER-02/03: Post-activation canonical/rebuild parity
- [x] `test_extract_equiv_post_act_floor_passes`: 12/57 attested, prune ON, 12 >= MIN_PRODUCERS_FLOOR=3, prunes to 12
- [x] `test_extract_equiv_post_act_floor_fires`: 2/57 attested, prune ON, 2 < 3, fallback to all 57
- [x] Both tests verify extraction matches derive_at_boundary via `assert_extraction_matches`

#### FILTER-07: No new EpochState fields, no version bump
- [x] EpochState struct has exactly 7 fields: epoch, bond_snapshot, producer_list, active_list, attested_sets, attestation_accum, blocks_produced
- [x] EpochState struct not in git diff (completely untouched)
- [x] CURRENT_PROTOCOL_VERSION = 8, EPOCH_STATE_FORMAT_VERSION = 1 (both unchanged)

#### Extraction Quality
- [x] `compute_live_producer_list()` is a free function (no `self`)
- [x] Takes only references/scalars, returns `Vec<PublicKey>` -- pure
- [x] No I/O: no file/network/async/await
- [x] No logging: no info!/warn!/error! macros
- [x] Lives in `crates/core/src/epoch_state/mod.rs` (line 32)
- [x] `derive_at_boundary()` calls it at line 262
- [x] `rewards.rs::rebuild_epoch_state_from_blocks()` calls it at line 856
- [x] Old inline floor logic (~120 lines) removed from rewards.rs (128 lines deleted, 42 added, net -86)

#### Decode-list Fix Quality
- [x] Fix gated on `epoch_prune_activation_height` at rewards.rs:784
- [x] Pre-activation: `active.clone()` sorted (historical behavior preserved)
- [x] Post-activation: `self.epoch_state.producer_list.clone()` (correct encoder-matching list)
- [x] Clear comment explaining gating and rationale at rewards.rs:780-783
- [x] `test_decode_list_post_act_divergence`: proves that pruned list (12) != full list (57), and post-activation derives 12-entry producer_list
- [x] `test_decode_list_pre_act_identity`: proves pre-activation floor fires, producer_list == active_sorted, decode is identical
- [x] `test_decode_list_correct_vs_wrong`: demonstrates index 5 decodes to different pubkeys in pruned vs full lists

## End-to-End Flow Results

| Flow | Steps | Result | Notes |
|---|---|---|---|
| Build gate | cargo build --release + clippy + fmt | PASS | Full workspace compiles, zero warnings |
| Unit tests | cargo test -p doli-core --lib -- epoch_state | PASS | 39/39 passed (22 existing + 17 new M2) |
| Diff review | git diff analysis | PASS | Only 2 source files changed, clean extraction |

## Exploratory Testing Findings

| # | What Was Tried | Expected | Actual | Severity |
|---|---|---|---|---|
| 1 | Verified old rewards.rs floor used different attested_union source (epoch_state.attested_sets) vs new code (block-scan attested) | Semantic equivalence | New code is actually MORE correct: uses same attested source for both filter and floor, matching derive_at_boundary. Pre-activation: floor fires to "include all", ghost check only active post-ghost-AH. No semantic divergence for pre-activation history. | low |
| 2 | Verified old rewards.rs re-read producer_set in floor block | Single read | New code reuses `active` vec from first read. Same data (same function call, same height), fewer lock acquisitions. Correct. | low |
| 3 | Checked all 3 EpochDerivationInput construction sites (post_commit.rs:308, fork_recovery.rs:718, rewards.rs:135) | All pass epoch_prune_activation_height | All three correctly pass it from NetworkParams | N/A |
| 4 | Verified mainnet epoch_prune_activation_height = u64::MAX | Disabled on mainnet | Confirmed in defaults.rs:101 | N/A |

## Failure Mode Validation

| Failure Scenario | Triggered | Detected | Recovered | Degraded OK | Notes |
|---|---|---|---|---|---|
| Pre-activation proportional floor override | Yes (test) | Yes | N/A | Yes | `test_extract_equiv_pre_act_floor_fires`: 12/57 attested, floor fires, all 57 retained -- identical to old behavior |
| Post-activation absolute floor fallback | Yes (test) | Yes | N/A | Yes | `test_extract_equiv_post_act_floor_fires`: 2/57 attested, 2 < MIN_PRODUCERS_FLOOR=3, falls back to all |
| Ghost exclusion interaction | Yes (test) | Yes | N/A | Yes | `test_extract_equiv_ghost_pre_act` and `test_extract_equiv_ghost_post_act`: ghost producers excluded from fallback |
| Activation boundary behavior | Yes (test) | Yes | N/A | Yes | `test_edge_activation_boundary`: height=AH prunes to 10, height=AH-1 floors to 20 |

## Security Validation

Not applicable for M2. This milestone is a pure refactor (code extraction + gated bug fix). No new external data surfaces, no new trust boundaries, no new attack vectors. The gating ensures pre-activation behavior is byte-identical.

## Specs/Docs Drift

| File | Documented Behavior | Actual Behavior | Severity |
|------|-------------------|-----------------|----------|
| specs/epoch-liveness-prune-architecture.md | M2 described as "Extract compute_live_producer_list + fix decode-list bug" with "-100 net lines" | Actual: -86 net lines in rewards.rs, +95 lines in epoch_state/mod.rs (new function). Net across both files is close to spec estimate. | low |
| specs/epoch-liveness-prune-architecture.md:338 | "rewards.rs:867-941" cited as inline dupe location | Old code's floor block was at lines 853-971 (larger than cited). Line numbers drift is expected as the file evolves. | low |

## Blocking Issues (must fix before merge)
None.

## Non-Blocking Observations
- **[OBS-001]**: The old rewards.rs floor logic used `self.epoch_state.attested_sets` for ghost identification while the new code uses the block-scan `attested` set. Both are valid -- the block-scan set is actually more correct because it uses the same data source for filter and floor, matching derive_at_boundary's behavior. No semantic divergence for pre-activation history because the floor fires to "include all active" regardless.

## Modules Not Validated
- **M3 (mainnet activation height pinning)**: Not yet implemented -- separate decision session per spec.
- **Integration test with running node**: Not attempted (unit tests are sufficient for a pure function extraction + gated fix).

## Final Verdict

**PASS** -- All Must requirements met. No blocking issues. The extraction is semantically correct, the decode-list fix is properly gated, no EpochState struct changes, no version bumps. 39/39 tests pass. Clean clippy, clean build, clean format. Approved for review.
