<!--
OUTPUT CONTRACT: N/A — workflow reasoning trace (not a test file)
INPUT PARTITIONS: N/A — workflow reasoning trace (not a test file)
-->

# Architecture Reasoning Trace: AMM Value-Conservation

INC-I-096 | 2026-05-29

## Evaluator Reports Summary

### Subtractionist (removal lens)
- **Proposals:** P1 (eliminate mempool reimplementation -231 lines), P2 (replace ad-hoc checks with recompute-and-verify -158 net), P3 (smallest viable subtraction: remove mempool pool-aware conservation only -73 lines), P4 (add FungibleAsset input binding +35 lines), P5 (delete `verify_invariant` dead export)
- **Key evidence:** Measured 15 AMM checks across 4 sites (~691 lines). Identified 231 mempool lines as redundant reimplementations. Confirmed block assembly at `assembly.rs:235` runs full consensus validation.
- **Gaps:** 25/5 bps fee mechanics not traced. Existing INC-I-096 patch not compared.
- **Disproved:** "Removing mempool conservation = unbounded DoS" -- bounded by mempool count/size limits + one-pool-tx-per-block contention.

### Restructurer (coupling/boundary lens)
- **Proposals:** P1 (extract `AmmTransitionVerifier` to `validation/amm.rs`), P2 (`AmmFlowSummary` struct), P3 (shared input-classification accumulator), P4 (validator calls pool.rs math), P5 KILLED by C6
- **Key evidence:** SD-1 Compute/Verify Split -- 7 pool.rs math functions have ZERO calls from validation. SD-2 Duplicate Conservation. SD-3 Asset-Blind Accounting. Verified `UtxoProvider` impl at `set.rs:404`.
- **Gaps:** AddLiquidity token_b binding (confirmed ZERO in any site). Swap B-to-A token_b input binding (confirmed ZERO).
- **Cross-layer:** SD-1 mirrors known `calculate_epoch_rewards`/`calculate_expected_epoch_rewards` disconnection.

### Pattern Matcher (anti-pattern/industry lens)
- **Proposals:** P1 (generalize RC-B to all 4 types), P2 (dual-asset conservation equation), P3 (shared validation function), P4 PARTIALLY KILLED (over-constraining for Swap), P5 (LPShare input binding)
- **Key evidence:** Named the anti-pattern (TCCR). Matched against Uniswap v2, Cardano multi-asset, CKB cell model. Confirmed 5 existing correct-pattern sites in the codebase (coinbase, RC-B, native conservation, pool_id, MINIMUM_LIQUIDITY).
- **Critical finding:** P5 confirmed existing patch is NOT secure (T10 drain OPEN on devnet). `shares_burned` from `new_total_lp` is attacker-controlled.
- **Gaps:** 25/5 bps fee path. apply_block enforcement.

### Failure Analyst (adversarial lens)
- **Proposals:** P1 (unified AMM validation function), P2 (input-binding pre-pass), P3 (do not activate until drains closed), P4 (token_b conservation scoped to AMM)
- **Key output:** 23 failure modes catalogued (12 safety, 5 liveness, 3 determinism, 3 deploy). 5 SAFETY failures OPEN. 9 HARD FILTERS. Per-P0 kill-criteria for SEC-LOGIC-001 and SEC-LOGIC-002.
- **New finding:** FM-S11 (asset_id counterfeiting on FungibleAsset outputs) -- not in original 6 defects.
- **Disproved (not real attacks):** FM-S6 (A-to-B output inflation, blocked by exact equality), FM-S7 (dust drain, benefits LPs), FM-S8 (first-deposit donation, blocked by MINIMUM_LIQUIDITY), FM-S9 (cross-pool confusion, blocked by UTXO model + pool_id), FM-S10 (u128 overflow, mathematically safe).

### Radical Simplifier (minimum-viable lens)
- **Proposals:** P1 (single `verify_amm_conservation()` with 3 per-asset equations), P2 (retain structural pool.rs), P3 (mempool calls shared fn via adapter), P4 (k-invariant inside conservation fn), P5 (return surplus for mempool fee)
- **Key evidence:** First-principles derivation of per-asset value extractors and E1/E2/E3 equations. Complexity comparison: 467 lines -> 130 lines, 3 sites -> 1 site, 7 code blocks removed.
- **Key insight:** Conservation prevents THEFT; k-invariant prevents VALUE LEAK -- independent guarantees, both needed.
- **Gaps:** `unwrap_or(0)` on malformed metadata. `inc_i_096 <= amm` activation ordering. Mempool contention tests references.

## Deletion Convergence Analysis

### Remove mempool conservation reimplementation (4/5)
- Subtractionist: P3 (-73 lines, smallest) + P1 (-231 lines, full delegation)
- Restructurer: SD-2 (copy-paste anti-pattern), P1 (extract to shared module)
- Pattern Matcher: P3 (Bitcoin CheckTransaction pattern)
- Radical: P3 (mempool calls shared fn)
- **Independence:** Line measurement, structural defect, industry pattern, mathematical derivation -- 4 independent evidence sources.
- **Verdict:** conf(0.85, converged). Execute.

### Remove ad-hoc utxo.rs per-type checks above gate (4/5)
- Subtractionist: P2 (-258 lines of ad-hoc trust-declared-state)
- Restructurer: P4 (validator calls pool.rs math instead)
- Pattern Matcher: P1 (generalize RC-B to all types)
- Radical: P1 (single function replaces 5 scattered blocks)
- **Independence:** Yes -- each evaluator approaches from a different angle.
- **Verdict:** conf(0.85, converged). Execute (above gate; preserve below gate for INV-COMPAT-001).

### Remove `verify_invariant` dead export (1/5)
- Subtractionist only (P5, conf(0.7))
- **Verdict:** Present as option within RC-2. Not a standalone definite change.

## Restructuring Analysis

### Shared function (5/5)
All 5 evaluators independently proposed a shared function. Independence verified across 5 different lenses. The structural form converges on a function in `crates/core/src/validation/` that takes data (`&[Output]`) rather than a trait, enabling both mempool and consensus to call it. conf(0.85, converged).

### Per-asset balance equations (4/5)
Restructurer (SD-3), Pattern Matcher (Cardano model), Failure Analyst (FM-S3/S5/S11/S12), Radical (E1/E2/E3 derivation). The Subtractionist did not independently propose equations (focused on deletion) but P4 (adding FungibleAsset input binding) is consistent. conf(0.85, converged).

### New module location (2/5)
Only Restructurer and Radical explicitly propose `amm.rs` / `amm_conservation.rs` as a new file. Others assume the shared function but don't specify location. conf(0.7, converged) -- recommended, not definite.

## Addition Options Analysis

### OPTION A: Validator re-verifies builder math
- Source: Restructurer P4 (conf(0.60)) + Pattern Matcher P4 (conf(0.55, partially killed))
- Pattern Matcher killed `compute_swap()` for Swap (over-constraining), kept it for LP ops
- Failure mode filter: NEUTRAL (conservation + k-invariant already sufficient)
- vs. Radical floor: +60 lines. Radical does NOT call builder math -- conservation is independent of pricing
- **Verdict:** User choice. The radical minimum (conservation + k-invariant) is mathematically complete without builder math re-verification.

### OPTION B: Return surplus for mempool fee
- Source: Radical P5 (conf(0.5))
- Failure mode filter: NEUTRAL
- The mempool could alternatively set fee=0 for AMM txs (they are fee-exempt)
- **Verdict:** Low-stakes user choice. Clean API if implemented; not load-bearing.

## Failure Mode Filtering Log

### DC-1 (shared per-asset conservation function) vs all failure modes:

| Failure Mode | Result | Adjustment |
|-------------|--------|------------|
| FM-S1 (RemoveLiquidity full drain) | RESOLVES via E1+E3 proportional binding | +0.1 |
| FM-S2 (LP-input underburn) | RESOLVES via E3 (lp_value extractor binds consumed LPShare amounts) | +0.1 |
| FM-S3 (B-to-A phantom injection) | RESOLVES via E2 (token_b conservation by asset_id) | +0.1 |
| FM-S4 (AddLiquidity inflated LP) | RESOLVES via E3 (LP supply bounded) + E1/E2 (reserve increases bounded) | +0.1 |
| FM-S5/S12 (phantom reserve inflation) | RESOLVES via E2 (token_b conservation) | +0.1 |
| FM-S11 (asset_id counterfeiting) | RESOLVES via token_b_value filtering on asset_id | +0.1 |
| FM-L1/L2 (false rejection) | RESOLVES via pool-aware E1 equation | +0.05 |
| FM-L3 (mempool/consensus parity) | RESOLVES via shared function | +0.05 |
| FM-L4 (floor-division false reject) | RESOLVES via >= tolerance | +0.05 |
| FM-D1 (arithmetic divergence) | NEUTRAL (function uses its own formula, not builder math) | 0 |
| FM-D3 (unwrap_or(0) fallback) | RESOLVES via DC-4 explicit rejection | +0.05 |
| FM-P1 (height reuse) | NEUTRAL (separate height already allocated) | 0 |
| FM-P3 (premature activation) | RESOLVES via FILTER-6 + INV-DEPLOY-002 assertion | +0.05 |

**Adjusted confidence:** base 0.65 + 0.85 adjustments (capped at 0.95) = conf(0.90, converged)

However, per evidence floor rules, I cap at conf(0.85, converged) because the design has not been implemented and tested yet -- the confidence is for the DESIGN, not the implementation.

## Radical Tiebreaker Log

| Proposal | Lines | Modules | vs. Radical | Gap | Verdict |
|----------|-------|---------|-------------|-----|---------|
| DC-1+DC-2+DC-3 (converged) | ~130 | 1 new | = Radical | 0 | Same design |
| OPTION A (+builder math) | ~190 | 1 new | +60 lines | 0.1 conf gap | Radical wins (SSF) unless user chooses Option A |
| OPTION B (+surplus return) | ~135 | 1 new | +5 lines | 0.05 conf gap | Negligible; include if convenient |

The converged design IS the radical minimum. No tiebreaker needed.

## Contradiction Analysis

### Contradiction: Subtractionist "remove mempool conservation" vs. Failure Analyst FILTER-4 (parity requirement)

**Subtractionist P3:** Remove mempool pool-aware conservation entirely (mempool skips AMM conservation, lets consensus be authoritative).

**Failure Analyst FILTER-4:** Mempool and consensus MUST produce identical accept/reject decisions.

**Surface contradiction:** Removing mempool conservation means mempool admits invalid AMM txs that consensus would reject. This violates FILTER-4.

**Resolution:** The contradiction resolves by DELEGATION, not deletion. The Subtractionist's P1 (not P3) replaces mempool reimplementation with a call to the shared function. P3 (pure deletion) was the "smallest viable subtraction" but is insufficient alone. The converged design adopts P1's delegation approach: mempool CALLS `verify_amm_conservation()`, achieving both the deletion of duplicate logic AND parity-by-construction.

**Evidence quality comparison:**
- Subtractionist P3: conf(0.65, observed) -- bounded DoS risk is real but the parity violation is also real
- Failure Analyst FILTER-4: conf(0.70, measured) -- FM-L3 proves silent failure from parity divergence

**Verdict:** RESOLVED. Shared function delegation satisfies both: mempool's reimplemented logic is deleted (Subtractionist goal), AND identical accept/reject is achieved (Failure Analyst requirement).

## Confidence Evolution

| Stage | Confidence | Basis |
|-------|-----------|-------|
| Individual evaluator proposals | conf(0.55-0.70) | Each evaluator's own assessment |
| Deletion convergence (4/5) | conf(0.85) | Independent evidence convergence |
| Restructuring convergence (3-5/5) | conf(0.85) | Cross-lens agreement on per-asset model |
| Failure mode filtering | conf(0.85, capped) | All 5 OPEN safety failures RESOLVED by design |
| Radical tiebreaker | conf(0.85) | Converged design = radical minimum (no gap) |
| Contradiction resolution | conf(0.85) | 1/1 contradiction resolved (delegation not deletion) |

Final design confidence: conf(0.85, converged). The design is the simplest correct architecture for AMM value-conservation in DOLI's UTXO model. The gap from current state (~467 lines, 3 sites, 5 partial trust points) to proposed state (~130 lines, 1 site, 3 universal equations) is a net -330 lines with strictly stronger security properties.

## Unresolved Gaps (inherited from evaluators)

1. **25/5 bps protocol fee extraction path:** No evaluator fully traced whether the protocol fee (5 bps) creates a DOLI output that exits the pool. If it does, E1's `>=` would absorb it (the fee is an outflow from the pool, reducing surplus but not violating conservation). If it stays in reserves, k increases. Either way, `>=` handles it. LOW RISK but should be verified during implementation.

2. **`fungible_asset_metadata()` parser robustness:** Radical Simplifier flagged that a malformed FungibleAsset UTXO's `asset_id` extraction could return None, silently excluding the token from conservation sums. The `ok_or_else` pattern (matching FILTER-8) should be applied to FungibleAsset metadata extraction too, not just Pool metadata.

3. **`crates/mempool/src/contention_tests.rs` references `inc_i_096`:** Any refactor of mempool conservation must update these tests. Identified by Radical Simplifier.
