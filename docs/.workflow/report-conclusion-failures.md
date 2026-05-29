# Report Conclusion — Failure Analyst

Source: `docs/.workflow/design-failures.md`

## Top finding
The INC-I-096 WIP patch fixes liveness but UNMASKS five OPEN drain vectors — conf(0.70, measured) — because the buggy pool-blind conservation check was the only thing (accidentally) blocking them. 12 safety / 5 liveness / 3 determinism / 3 deploy failure modes catalogued. 5 SAFETY failures OPEN: FM-S2 (LP-input underburn, full drain with 1 LP share — proven by ignored T10 at `inc_i_096_amm_conservation.rs:852`), FM-S3 (B→A phantom token injection / SEC-LOGIC-002), FM-S4 (AddLiquidity inflated LP minting), FM-S5/S12 (token_b phantom reserve inflation), **FM-S11 (NEW: FungibleAsset output asset_id mismatch → token counterfeiting across pools; `utxo.rs:663-670` filters on OutputType but NOT asset_id)**.

## HARD FILTERS (load-bearing — synthesizer applies to EVERY proposal)
- **FILTER-1 (ATOMIC LIVENESS-SAFETY COUPLING — CRITICAL):** MUST NOT fix liveness without simultaneously closing ALL drains. A proposal adding pool-aware conservation but not (a) proportional reserve-delta binding, (b) LP-input binding, (c) token_b input binding for B→A + AddLiquidity, (d) proportional LP minting check, (e) asset_id cross-check on FungibleAsset outputs → FAILS.
- **FILTER-2 (LP-INPUT BINDING MANDATORY):** bind `shares_burned = old_total_lp - new_total_lp` to sum of consumed LPShare input amounts for the pool_id. Root of FM-S2.
- **FILTER-3 (TOKEN_B CONSERVATION MANDATORY):** per-asset conservation for token_b scoped to AMM types, matching `asset_b_id`. MUST NOT modify `is_native_amount()` / `total_output()`.
- **FILTER-4 (MEMPOOL/CONSENSUS PARITY):** identical accept/reject for all AMM txs.
- **FILTER-5 (FLOOR-DIVISION TOLERANCE):** use `<=` not `==`; formula bit-identical builder vs validator, u128 intermediate.
- **FILTER-6 (ACTIVATION HEIGHT ISOLATION):** use `inc_i_096_activation_height` (`network_params/mod.rs:482`); MUST NOT reuse inc_i_092; MUST NOT activate until all drains closed.
- **FILTER-7 (ADDLIQUIDITY PROPORTIONAL BINDING):** verify LP minted proportional to reserves added + declared increases match consumed inputs. Current `utxo.rs:705-743` only checks monotonic increase.
- **FILTER-8 (NO SILENT FALLBACKS):** reject AMM tx where `pool_metadata()` is None; no `unwrap_or(0)` (`utxo.rs:238-244`).
- **FILTER-9 (ASSET_ID CROSS-CHECK):** FungibleAsset outputs must carry pool's `asset_b_id`. Likely also affects RemoveLiquidity token output `utxo.rs:838-844` (unverified).

## Per-P0 kill-criteria
- **SEC-LOGIC-001 (RemoveLiquidity):** 5 checks ALL necessary — lp_inputs_consumed sum; declared_burned = old-new; reject burned>consumed (FM-S2); reject reserve_delta > proportional cap (FM-S1); reject tokens_out > reserve_b_delta by matching asset_b_id.
- **SEC-LOGIC-002 (B→A Swap):** 6 checks — token_b_inputs sum by asset_id; declared_reserve_b_increase; reject increase > token_b_inputs (NEW, closes FM-S3); k-invariant new_k>=old_k; pool-aware DOLI conservation; asset_id on outputs.

## Invariants
INV-SAFETY-001 `native_input+old_ra >= native_output+new_ra`; INV-SAFETY-002 `new_k>=old_k`; INV-SAFETY-003 MIN_LIQ=1000 locked; INV-SAFETY-004 pool_id includes fee_bps; INV-DEPLOY-001 gate not real until all filters met; **INV-DEPLOY-002 `amm_activation_height >= inc_i_096_activation_height`** (independent heights — add assertion); INV-COMPAT-001 bit-identical below gate; INV-DETERM-001 u64/u128 same formula.

## Disproved (NOT real attacks)
FM-S6 (A→B token output inflation — `utxo.rs:671` exact equality), FM-S7 (dust drain — dust benefits LPs), FM-S8 (first-deposit donation — `pool.rs:111-121` MIN_LIQ guard), FM-S9 (cross-pool confusion — UTXO + pool_id check), FM-S10 (u128 overflow — (2^64-1)^2 < 2^128-1, safe).
