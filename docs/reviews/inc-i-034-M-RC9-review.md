# Code Review: INC-I-034 M-RC9 — Silent `vec![]` fail-fast fix

**Date**: 2026-04-16
**Reviewer**: reviewer (parallel agent)
**Milestone**: M-RC9 (first of INC-I-034 trigger-removal trio)
**Verdict**: **APPROVED FOR COMMIT**

## Scope reviewed

- `bins/node/src/node/rewards.rs` — the fix (+62 net)
- `bins/node/tests/m_rc9_silent_vec_regression.rs` — 3 tests, 692 lines
- `docs/bugfixes/inc-i-034-m-rc9-silent-vec-fix.md` — changelog
- `docs/qa/inc-i-034-M-RC9-qa-report.md` — QA corroboration
- Caller sites: `production/assembly.rs:52`, `validation_checks.rs:567,648`
- Supporting context: `specs/scheduler-state-architecture.md:260-273`, `docs/.workflow/blockchain-investigation-consensus.md:125-148`

## Root cause fix verification — PASSED

The causal chain (from `blockchain-investigation-consensus.md`):

```
(1) gap in block_store
(2) epoch scan iterates epoch_start..epoch_end
(3) SILENT skip / SILENT vec![]    ← M-RC9 patch site
(4) wrong attested_minutes
(5) wrong qualifier count emitted by producer
(6) validator rejects EpochReward
(7) brief fork
```

The fix is at step (3), the ACTUAL silent-skip and silent-vec![] sites — NOT downstream at step (6) where the mismatch is detected:

- `rewards.rs:70-118`: `if let Ok(Some(block))` converted to explicit `match`; both `Ok(None)` and `Err(_)` increment `missing_block_count`
- `rewards.rs:85-95`: post-activation `else` branch (previously `vec![]` with no counter) now increments `silent_bitfield_count`
- `rewards.rs:126-138`: when either counter > 0 (and epoch > 0), returns `Vec::new()` with `error!` logging — pre-decoding, before bad data reaches the qualifier filter

This is REQ-REDESIGN-006 layer (a) — trigger removal. Bad data never enters `attested_minutes`.

## Unintended behavior changes — NONE

- Signature unchanged: `Vec<(u64, Hash)>`
- Happy path byte-identical (both counters 0 → guard skipped → pre-existing logic unchanged)
- Epoch 0 genesis exemption preserved (`if epoch > 0 && ...` guard)
- Callers verified safe with empty-Vec contract:
  - `assembly.rs:52`: `if !epoch_outputs.is_empty() { ... } else { debug!(...) }` — no retry
  - `validation_checks.rs:567,648`: pre-fix also rejected on incomplete-store (just with silently-wrong expected)

## Test quality — ADEQUATE

- Test A (happy path): regression anchor, PASS
- Test B (gap in middle): adversarial, FAIL on HEAD → PASS after fix
- Test C (Santiago mainnet-scale replay, 37 producers / 11 gap heights): FAIL on HEAD → PASS after fix
- Output Contract matrix: 12/12 reachable cells asserted; 12 justified N/A (P3/P4 subsumed by P2, P6 dead on HEAD due to BITFIELD_BODY_ACTIVATION_HEIGHT=0)
- FAIL→PASS evidence is genuine — satisfies CLAUDE.md Rule 21 for conf ≥ 0.7 bugfix

## Minor findings (all non-blocking)

- **M1 (LOW)**: `specs/scheduler-state-architecture.md:273` language leans toward snapshot branch; shipped fix is fail-fast OR-branch. One-line note suggested. Docs-sync task, not code.
- **M2 (LOW)**: Aggregate `error!` at `rewards.rs:127` folds `Ok(None)` + `Err(_)` into single `gap_count`. Splitting `gap_count_absent` vs `gap_count_err` would improve ops diagnostics. Future polish.
- **M3 (LOW)**: Epoch-0-with-gap path is structurally correct by construction but not asserted by test. ~10-line test would close matrix cell.
- **M4 (LOW cosmetic)**: Test B wastefully builds and discards a first node. No correctness impact.

## Pre-existing issues confirmed NOT caused by M-RC9

- `bins/node/tests/epoch_state_regression.rs` compile errors (`node.best_hash()` removed during EpochState refactor in `42740269` and `3d267217`). Tracked for separate triage.
- `rewards.rs` exceeds 500-line budget (1162 lines). Pre-existing.

## Injection pattern scan — CLEAN

No SQL injection, shell escape, eval/exec, or subprocess vectors in the diff. Pure internal computation.

## Acceptance evidence

| Gate | Result |
|---|---|
| `cargo test --test m_rc9_silent_vec_regression -p doli-node` | 3/3 PASS |
| `cargo test -p doli-node --lib` | 10/10 PASS |
| `cargo test -p doli-node --test epoch_reward_explicit_inputs` | 7/7 PASS |
| `cargo test -p doli-node --test fork_recovery` | 11/11 PASS |
| `cargo test -p doli-node --test checkpoint_rotation` | 16/16 PASS |
| `cargo test -p doli-node --test test_network` | 13/13 PASS |
| `cargo build --release -p doli-node` | clean |
| `cargo clippy -p doli-node -- -D warnings` | clean |
| `cargo fmt --check` | clean |

## Verdict

**APPROVED FOR COMMIT.** Milestone-loop iteration: 1/2 reviewer iterations used.

The fix is surgical, at the root cause, with FAIL→PASS evidence. Proceed to commit + auto-continue to M-RC10 (apply-after-reject).
