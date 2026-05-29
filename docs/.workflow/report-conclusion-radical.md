# Report Conclusion — Radical Simplifier

Source: `docs/.workflow/design-radical.md`

## Minimum viable architecture
A single shared function `verify_amm_conservation()` in a new `crates/core/src/validation/amm_conservation.rs` (~120 lines) implements THREE universal per-asset balance equations + k-invariant, replacing ~467 lines of scattered per-type conservation logic across `utxo.rs` and the mempool. conf(0.65, observed).

## Core insight (per-asset balance)
In the UTXO model the consumed Pool UTXO carries old reserves (extra_data), the output Pool UTXO carries new reserves. Define per-asset value extractors:
- `doli_value(utxo) = amount if native, reserve_a if Pool, else 0`
- `token_b_value(utxo) = amount if FungibleAsset(asset_b_id), reserve_b if Pool, else 0`
- `lp_value(utxo) = amount if LPShare(pool_id), total_lp if Pool, else 0`

Then conservation is simply, per asset class: `sum(value_in) >= sum(value_out)` (the `>=` absorbs floor-division dust to pool). This single pattern closes:
- D1 (reserves counted), D2 (one equation, same both sites), D3 (shares_burned bound: new_total_lp = old_total_lp − sum(LPShare inputs)), D4 (new_reserve_b bound to token inputs), H2 (token_b is just another asset class).

## Balance equations per tx type
E1 DOLI conservation, E2 token_b conservation, E3 LP-supply conservation — applied (with direction) to CreatePool / AddLiquidity / RemoveLiquidity / Swap.

## k-invariant: STILL NEEDED (P4, conf 0.55)
Conservation prevents THEFT (no asset created from nothing). k-invariant prevents VALUE LEAK via mispriced swaps. Independent guarantees — both required. `verify_amm_conservation()` does NOT call `compute_swap`; conservation is independent of pricing.

## Complexity comparison
| Metric | Current (patch-set) | Proposed minimum |
|--------|--------------------|--------------------|
| Conservation sites | 3 | 1 (shared fn, 2 call sites) |
| Lines of conservation logic | ~467 | ~130 |
| Declared-state trust points | 5 partial | 3 universal |
| Per-tx-type special cases | 4 bespoke (302L) | 4 arms in 1 dispatch (~80L) |
| Mempool/consensus parity | manual duplication | shared fn (guaranteed) |
| token_b coverage | partial (3 of 4) | full (4 of 4) |
| LP supply coverage | 1 tx type (gated) | all 4 |
| New modules | 0 | 1 |
| Code blocks removed | 0 | 7 |
Net ~ −330 lines.

## Key implementation constraint
Shared function must accept consumed outputs as `&[Output]` DATA (not a trait), so both mempool (`UtxoSet`) and consensus (`UtxoProvider`) callers construct it without a wrapper. Gate via `height`/`activation_height` params: below gate → existing per-type checks bit-identical; above gate → shared fn.

## Gaps
- `unwrap_or(0)` on malformed Pool metadata could hide reserves (theft vector) — must reject None.
- Must enforce `inc_i_096_activation_height <= amm_activation_height` or old buggy conservation runs in the gap.
- `crates/mempool/src/contention_tests.rs` references inc_i_096 — refactor must update.
