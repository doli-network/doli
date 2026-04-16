# Code Review: INC-I-034 M-RC10 — Apply-after-reject path desync fix

**Date**: 2026-04-16
**Reviewer**: reviewer
**Milestone**: M-RC10 (second of INC-I-034 trigger-removal trio)
**Verdict**: **APPROVE-WITH-CAVEAT** (commit; CAVEAT items are test-writer follow-up scope)

## Root-cause fix verification — PASSED

Today's santiago cascade causal chain:

```
(1) peer gossips block h=39599 with EpochReward at non-boundary
(2) handle_new_block → returns Ok(()) with reject log (correct)
(3) SAME block arrives via Light-mode entry (periodic.rs:112 / execute_reorg / try_apply_cached_chain)
(4) validate_block_economics SKIPS ECON_EPOCH_NOT_BOUNDARY under Light    ← M-RC10 fix
(5) (height / blocks_per_epoch) - 1 underflows → wraps u64::MAX in release
(6) downstream ops on garbage epoch → partial UTXO mutation, misleading log
(7) [BLOCK] Applied logged before [UTXO] FAIL tears it down
```

The lifted `?`-propagated `Err` returns from `validate_block_economics` BEFORE the misleading `[BLOCK] Applied` log at `apply_block/mod.rs:75`, BEFORE `producer_liveness.insert` at `:92`, BEFORE batch creation. Apply-after-reject window closed at the source.

## Lift / keep classification audit — PASSED

Independently re-classified all 10 ECON_EPOCH_* checks against fork-sensitivity:

**LIFTED (5/5 correct — fork-independent):**
- ECON_EPOCH_NOT_BOUNDARY (line 495) — pure arithmetic. The santiago root cause.
- ECON_EPOCH_EXTRA_DATA (line 541) — wire format
- ECON_EPOCH_HEIGHT (line 550) — pure consistency
- ECON_EPOCH_NUMBER (line 557) — pure arithmetic
- ECON_EPOCH_OVERFLOW (line 583) — conservation cap (strictly conservative even on transient fork)

**KEPT Full-only (5/5 correct — fork-sensitive):**
- ECON_EPOCH_DISTRIBUTION (line 611) — depends on calculate_epoch_rewards()
- ECON_EPOCH_NO_INPUTS (line 637) — conservative; downstream OVERFLOW catches it
- ECON_EPOCH_INPUTS_MISMATCH (line 659) — local pool composition
- ECON_EPOCH_PRE_INPUTS (line 667) — dead branch (EPOCH_REWARD_EXPLICIT_INPUTS_HEIGHT=0)
- ECON_EPOCH_MISSING (line 686) — depends on calculate_expected_epoch_rewards

## Defensive arithmetic — PASSED

- `checked_sub(1)` at line 509: returns `anyhow::bail!("[ECON_EPOCH_UNDERFLOW]")` cleanly via `?` propagation, NOT panic.
- `saturating_sub(1)` at line 682 + `if completed_epoch > 0` guard: degenerate case degrades to no-op rather than spurious `[ECON_EPOCH_MISSING]`.

## Unintended changes — NONE

Grep verified: only 2 `matches!(mode, ValidationMode::Full)` gates remain in `validation_checks.rs` (lines 598, 674) — both legitimate. No accidental scope creep. Function signature unchanged. Coinbase validation untouched. `validate_block_for_apply` untouched.

## Test B intermittency — confirmed pre-existing fixture, NOT M-RC10

Traced the error string `"outside time window (offset_secs=..., eligible_count=...)"` to `crates/core/src/validation/producer.rs:223`, called from `validate_block_for_apply → validate_block_with_mode` BEFORE `validate_block_economics` is reached. Strictly upstream of M-RC10's code path. Cause: random `KeyPair::generate()` interaction with bootstrap eligibility window at slot=3. ~60-80% pass rate is entropy.

## Acceptance evidence

| Gate | Result |
|---|---|
| `test_c_non_boundary_light_mode_must_also_reject` (PRIMARY) | FAIL→PASS, 5/5 deterministic |
| `test_a_plain_block_applies_cleanly_in_light_mode` (anchor) | PASS deterministic |
| `test_d_duplicate_reject_no_ratcheting_damage` | PASS deterministic |
| `test_b_non_boundary_full_mode_rejects_cleanly` | INTERMITTENT (pre-existing fixture, F1) |
| `cargo test -p doli-node --lib` | 10/10 PASS |
| `cargo test -p doli-node --test fork_recovery` | 11/11 PASS |
| `cargo test -p doli-node --test m_rc9_silent_vec_regression` | 3/3 PASS |
| `cargo test -p doli-node --test test_network` | 13/13 PASS |
| `cargo build --release / clippy / fmt` | clean (drift confined to test file F3 + pre-existing) |

## Findings (all LOW, non-blocking)

- **F1 (test-writer)**: Test B intermittency. Recommend deterministic seed.
- **F2 (docs)**: CLAUDE.md "If You Touch → apply_block()" could note Light/Full semantic-vs-fork distinction.
- **F3 (style)**: 3 `cargo fmt` nits in test file (lines 397, 488, 625). Test-writer scope.
- **F4 (enhancement)**: ECON_EPOCH_NO_INPUTS / PRE_INPUTS could be lifted for symmetry. Not a defect.

## Pre-existing issues NOT caused by M-RC10

- `epoch_state_regression.rs` compile errors (from prior EpochState refactor)
- `validation_checks.rs` `cargo fmt` drift (~36 lines, pre-existing per stash verification)

## Verdict

**APPROVE-WITH-CAVEAT — proceed to commit M-RC10.** Bundle F1 + F3 in a test-writer follow-up. F2 + F4 to backlog. Auto-continue to M-RC11.
