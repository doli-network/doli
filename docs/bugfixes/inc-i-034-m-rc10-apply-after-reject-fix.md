# INC-I-034 — M-RC10: Apply-after-reject desync in block validation pipeline

**Date:** 2026-04-16
**Branch:** `synmgrefactor`
**File touched:** `bins/node/src/node/validation_checks.rs`
**Tests:** `bins/node/tests/m_rc10_apply_after_reject_regression.rs` (4 tests; C — primary milestone — PASS; A + D PASS; B is a pre-existing fixture issue in `validate_block_for_apply` producer-eligibility wiring, not an M-RC10 concern)
**Incident:** INC-I-034 (live mainnet cascade 2026-04-16 05:11 UTC, santiago node h=39599)
**Prior milestone:** M-RC9 (`inc-i-034-m-rc9-silent-vec-fix.md`)

## Symptom

Santiago (ai3) mainnet log, 2026-04-16 05:11 UTC:

```
05:11:04.795Z  [BLOCK] REJECT slot=40913 h=39599 producer=c368a55f
               error=[ECON_EPOCH_NOT_BOUNDARY] EpochReward at non-boundary
               height=39599 (blocks_per_epoch=360) — skipping, sync will catch up
05:11:05.787Z  [BLOCK] Applied h=39599 hash=ed9bab0b... producer=c368a55f
05:11:05.788Z  [UTXO] FAIL h=39599 type=EpochReward error=output not found
```

**The same block is rejected by one code path and half-applied by another, within one second.** The 05:11:05 "Applied" log fires before transaction processing runs; the 05:11:05.788 "UTXO FAIL" tears the apply down, but by then `producer_liveness` and other side-effect state have already been mutated. On mainnet this returns `Err` from `apply_block`, but under-the-hood state has drifted.

## Root cause

In `bins/node/src/node/validation_checks.rs::validate_block_economics`, the `ECON_EPOCH_NOT_BOUNDARY` check — and a surrounding block of `ECON_EPOCH_*` checks — were gated behind `matches!(mode, ValidationMode::Full)`:

```rust
// Pre-fix (line 482)
if !is_epoch_boundary && matches!(mode, ValidationMode::Full) {
    anyhow::bail!("[ECON_EPOCH_NOT_BOUNDARY] ...");
}
let completed_epoch = (height / blocks_per_epoch) - 1;   // (*)
```

Entry points that call `apply_block` with `ValidationMode::Light`:

- `bins/node/src/node/periodic.rs:112` (periodic catch-up)
- `bins/node/src/node/block_handling.rs:548` (`execute_reorg`)
- `bins/node/src/node/block_handling.rs:200` (`try_apply_cached_chain`)

…bypassed the boundary check entirely. The subtraction at (\*) then executed with a non-boundary height in the first epoch window. For `height < blocks_per_epoch`, `height / blocks_per_epoch == 0`, so `0 - 1` underflowed:

- **Debug builds:** panic at `attempted to subtract with overflow`
- **Release builds (mainnet):** wraps to `u64::MAX`, and downstream consistency checks (`embedded_epoch != completed_epoch`, pool-balance conservation, `calculate_epoch_rewards`) operate on garbage — producing the "ghost Applied followed by UTXO FAIL" pattern observed on santiago.

### Why the Full-only gate was wrong

`ValidationMode::Light` is intended to skip checks that depend on **current local fork state** during sync (e.g., producer eligibility against a transient GSet composition). The `ECON_EPOCH_*` family divides into two classes:

| Check | Class | Depends on local fork state? |
|---|---|---|
| `ECON_EPOCH_NOT_BOUNDARY` | Structural — pure `height % blocks_per_epoch` arithmetic | **No** |
| `ECON_EPOCH_ZERO` | Structural — `completed_epoch == 0` | **No** |
| `ECON_EPOCH_DUPLICATE` | Structural — `epoch_reward_txs.len()` | **No** |
| `ECON_EPOCH_EXTRA_DATA` | Structural — wire-format length check | **No** |
| `ECON_EPOCH_HEIGHT` | Structural — `embedded_height == block.height` | **No** |
| `ECON_EPOCH_NUMBER` | Structural — `embedded_epoch == completed_epoch` | **No** |
| `ECON_EPOCH_OVERFLOW` | Conservation — `total_distributed <= pool_balance` | No (read-only pool read) |
| `ECON_EPOCH_DISTRIBUTION` | Fork-sensitive — exact `calculate_epoch_rewards` match | **Yes** |
| `ECON_EPOCH_NO_INPUTS` / `ECON_EPOCH_INPUTS_MISMATCH` / `ECON_EPOCH_PRE_INPUTS` | Fork-sensitive — exact sorted pool-UTXO outpoints | **Yes** |
| `ECON_EPOCH_MISSING` (`else if`) | Fork-sensitive — requires `calculate_epoch_rewards` on the boundary | **Yes** |

The first seven are structural or conservation checks. A block carrying an `EpochReward` TX at a non-boundary height — or with mismatched embedded `(height, epoch)` fields, or whose payouts exceed the reward pool — is **invalid regardless of sync mode**. These checks are cheap constant-time arithmetic and MUST fire for every `apply_block` path.

The last four genuinely depend on the local node's reward computation and pool composition; in Light mode the node may be behind or on a transient micro-fork, so comparing byte-for-byte against `calculate_epoch_rewards` output can spuriously reject valid canonical blocks. These remain `Full`-only.

## Fix

Two surgical changes in `validation_checks.rs`:

1. **Lift the boundary and structural gates out of `matches!(mode, ValidationMode::Full)`.** `ECON_EPOCH_NOT_BOUNDARY`, `ECON_EPOCH_EXTRA_DATA`, `ECON_EPOCH_HEIGHT`, `ECON_EPOCH_NUMBER`, and `ECON_EPOCH_OVERFLOW` now fire in both `Full` and `Light` modes. `ECON_EPOCH_DISTRIBUTION` and the explicit-input checks stay `Full`-only (fork-sensitive).

2. **Defense-in-depth on the `completed_epoch` arithmetic at the old line 490.** Replace `(height / blocks_per_epoch) - 1` with `checked_sub(1)`; return `[ECON_EPOCH_UNDERFLOW]` if the invariant is ever violated by a future refactor. Even though lifting the boundary gate proves `height >= blocks_per_epoch` at that point, the defensive form eliminates an entire class of underflow bugs at zero runtime cost. The parallel subtraction in the `else if is_epoch_boundary` missing-epoch-reward branch uses `saturating_sub(1)` with the same rationale (clippy prefers `saturating_sub` for `unwrap_or(0)` patterns).

Before the fix, the santiago replay at small scale (test C, devnet `blocks_per_epoch=4`, `height=3`) either panicked in debug builds or returned `Ok(())` on release after wrapping `completed_epoch` to `u64::MAX`. After the fix, `apply_block` returns a clean `Err("[ECON_EPOCH_NOT_BOUNDARY] …")` with zero side-effects on `producer_liveness`, the UTXO set, or `chain_state`.

## Test evidence

`cargo test --test m_rc10_apply_after_reject_regression -p doli-node`

- `test_a_plain_block_applies_cleanly_in_light_mode` — **PASS** (regression anchor, happy path)
- `test_b_non_boundary_full_mode_rejects_cleanly` — **FAIL (pre-existing, HEAD also fails with identical error)**. The test's block fixture fails `validate_block_for_apply`'s bootstrap producer-eligibility time window in Full mode before reaching the economics validation (`producer=..., slot=3, reason=outside time window`). This is a fixture/test-writer issue — the block's `slot=3` timestamp does not land within the eligible time window for the chosen producer. **Not an M-RC10 concern.** A fix would require either relaxing the time window check (consensus risk) or rotating producer selection in the fixture — both out of scope for this milestone and requiring test-file changes, which the milestone contract explicitly forbids.
- `test_c_non_boundary_light_mode_must_also_reject` — **PASS** (primary milestone objective, HEAD = FAIL). Light-mode `apply_block` now returns `Err("[ECON_EPOCH_NOT_BOUNDARY] ...")` cleanly; `pool_utxo_count`, `pool_utxo_total_amount`, `utxo_total_count`, `best_height`, `best_hash`, and `block_store` are all unchanged.
- `test_d_duplicate_reject_no_ratcheting_damage` — **PASS** (no state drift across repeated rejections)

Additional gates (all PASS, no regressions):

- `cargo test --test m_rc9_silent_vec_regression -p doli-node` — 3/3 PASS
- `cargo test -p doli-node --lib` — 10/10 PASS
- `cargo test -p doli-node --test fork_recovery` — 11/11 PASS
- `cargo test -p doli-node --test test_network` — 13/13 PASS (12 `#[ignore]` skipped)
- `cargo build --release -p doli-node` — clean
- `cargo clippy -p doli-node -- -D warnings` — clean

Workspace-level `cargo fmt --check` has 3 drift locations in the test file and 36-line drift in `validation_checks.rs`; both are **pre-existing on HEAD** and unchanged by this commit (verified by `git stash` / recompute). Test file drift cannot be fixed per the milestone contract. The `validation_checks.rs` drift predates this milestone and is unrelated to the M-RC10 change.

## Deferred follow-ups

Per the milestone brief, the following test-writer findings are intentionally **not** addressed in this commit:

1. `apply_block/mod.rs:75-83` — the `[BLOCK] Applied` log fires before `batch.commit()`. This is observability polish, not a consensus defect; log after commit to prevent misleading operator-facing traces.
2. `apply_block/mod.rs:92` — `producer_liveness.insert(...)` precedes the transaction loop. If transaction processing later fails, this mutation is not reverted. Low-impact scheduling concern; not the M-RC10 cascade trigger.

Both should land in separate, scoped commits. Bundling them here would bloat the diff and mask the structural-vs-fork-sensitive audit that is the core of the M-RC10 fix.

## Consensus impact

**No hard-fork required.** The lifted checks are strictly tightenings that reject previously-undetected invalid blocks. Any block that was producing a valid path through both `Full` and `Light` validators pre-fix still does — only blocks that were *silently accepted / half-applied by Light-mode entry points but correctly rejected by Full-mode gossip* are now consistently rejected across all apply paths. This aligns Light-mode rigor with Full-mode for structural invariants while preserving Light-mode's fork-tolerance for local-state-sensitive comparisons.

`CURRENT_PROTOCOL_VERSION` bump: **not required** (wire format unchanged, peer behavior unchanged).
