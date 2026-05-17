# INC-I-079 — INV-4 Wrong Halving Assumption (Test-Only Fix)

**Status**: Resolved (pending background long-sim verification)
**Branch**: main
**Workflow run**: 332 (`/omega-doctor --investigate` redirected mid-pipeline to a FAST-path test fix)
**Touched files**: `bins/node/tests/economic_sim_s1.rs` (test only — NO production code)

## Symptom

`bins/node/tests/economic_sim_s1.rs::economic_sim_s1_baseline` panicked at epoch 4499 (era 8→9 boundary):

```
INV-4 VIOLATED at epoch 4499 (era 8 -> 9):
  new era pool (781248) != prev_era_pool/2 (781250).
  Halving not applied correctly!
```

## Root Cause

INV-4 asserted `pool[N+1] == pool[N] / 2`, but the protocol formula is:

```rust
// crates/core/src/consensus/params.rs:223
pub fn block_reward(&self, height: BlockHeight) -> Amount {
    let era = self.height_to_era(height);
    if era >= 64 { return 0; }
    self.initial_reward >> era
}
```

The **per-slot reward** halves via integer right-shift — not the pool. The right-shift discards the low bit each era. Once `initial_reward >> prev_era` becomes odd, pool-level integer division (`prev_pool / 2`) keeps the lost half-unit, while per-slot right-shift discards it. First divergence with devnet `INITIAL_REWARD = 100_000_000`:

| era | reward (`initial_reward >> era`) | pool (`× 4 slots`) | OLD invariant expected |
|----:|---------------------------------:|-------------------:|------------------------:|
| 8 | 390_625 | 1_562_500 | — |
| 9 | 195_312 (truncated from 195_312.5) | **781_248** | 781_250 (`1_562_500 / 2`) |

Diff = 2 base units, increasing each subsequent era as more low bits are lost.

## Fix

`bins/node/tests/economic_sim_s1.rs` only. Two changes:

**1. INV-4 rewritten** (lines ~485-540) — strict protocol-formula invariant that catches real halving regressions:

```rust
let prev_era_reward = prev_pool / blocks_per_epoch;
let expected_era_reward = prev_era_reward >> 1;
assert_eq!(era_reward, expected_era_reward, ...);
assert_eq!(expected_pool, blocks_per_epoch * expected_era_reward, ...);
```

Catches:
- wrong halving direction (`<<` vs `>>`)
- wrong era stepping (halving every 2 eras instead of 1)
- pool/reward derivation drift (off-by-one in blocks_per_epoch)

Does NOT catch: `prev_pool / 2` (the OLD assumption was never the protocol contract).

**2. Focused regression unit test** `inc_i_079_inv4_protocol_halving_regression` — pure arithmetic, runs in 0ms, iterates eras 1..64 and pins:
- NEW invariant (`era_reward == prev_era_reward >> 1`) holds at every reachable era
- OLD invariant (`new_pool == prev_pool / 2`) first diverges at exactly era 9
- Concrete divergence values: 390_625 / 195_312 / pool 1_562_500 / 781_248 / diff 2

The pin on "first divergence at era 9" guards against future changes to `INITIAL_REWARD` or `BLOCKS_PER_EPOCH` that would silently shift the era-shape — if either changes, this test fires loudly before the long-form baseline burns 4500 epochs.

## Why No Production Code Change

Per the refined-prompt constraints:
- The protocol formula in `consensus/params.rs:223` is **correct as written** (matches whitepaper §10.1 updated this session).
- The bug is in the test's *invariant assumption*, not in any production code path.
- Per CLAUDE.md `#0 RULE`: no genesis reset, no `CURRENT_PROTOCOL_VERSION` bump, no activation height — protocol behavior is unchanged bit-for-bit.

## Verification

| Check | Result |
|------|--------|
| `cargo test --release --test economic_sim_s1` (7 non-ignored) | ✓ all pass |
| `inc_i_079_inv4_protocol_halving_regression` (focused) | ✓ pass (0ms) |
| `economic_sim_s1_smoke` (10 epochs) | ✓ pass |
| `test_gini_*` (3 unit tests) | ✓ pass |
| Two complementary regressions added by parallel session: `test_inv4_old_formula_fails_at_era9`, `test_inv4_protocol_formula_diverges_from_pure_halving` | ✓ pass |
| `economic_sim_s1_baseline` with `DOLI_SIM_EPOCHS=4520` (crosses era 8→9 at epoch 4500) | running in background — see monitor task `bob52anjr` |

## Confidence

`conf(0.95, measured)` once long-sim completes; `conf(0.9, measured)` from focused regression alone (arithmetic identity verified across all reachable eras 1..27 where reward > 0).

## Files Changed

- `bins/node/tests/economic_sim_s1.rs` — INV-4 rewrite (test logic, ~lines 485-540) + new regression test (~lines 704-790)
