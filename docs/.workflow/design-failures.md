# Design Evaluation: Failure Analyst

## Analysis Lens
ERROR PATHS, BLAST RADIUS, INVARIANT VIOLATIONS, ATTACK VECTORS, and RECOVERY BEHAVIOR in the AMM value-conservation layer. Adversarial reasoning. Central question: **What BREAKS in the current design, and what constraints must ANY redesign respect?**

## What I Don't Understand
1. Whether the `spend_transaction` path in apply_block double-checks Pool output integrity or just trusts consensus validation. If apply_block has ADDITIONAL checks beyond validation, a redesign needs to account for them.
2. Whether the mempool's `total_output` function uses the same `tx.total_output()` (which filters `is_native_amount()`) or a custom sum. The INC-I-096 patch fixes `calculate_inputs` but I need to confirm `total_output` parity.
3. What happens if `pool_metadata()` returns `None` at the conservation check site (utxo.rs:241-244) -- the code uses `unwrap_or(0)`, which would silently zero-out old/new reserves. Is this reachable? If so, it could falsely PASS conservation.
4. Whether there's a cross-pool confusion attack where the same FungibleAsset outputs could be claimed by two pools in the same block (intra-block ordering).
5. Exact behavior of apply_block when validation passes but spend_transaction fails in Replay mode -- the `warn!` + continue pattern could mask state divergence.

## Current State Analysis

### Enforcement Architecture
Three enforcement sites, each partially implemented:

| Site | File | DOLI Conservation | Token Conservation | Input Binding | LP Binding |
|------|------|-------------------|-------------------|---------------|------------|
| Mempool | `crates/mempool/src/pool.rs:380-430` | Patched (INC-I-096 WIP, gated) | NONE | NONE | NONE |
| Consensus | `crates/core/src/validation/utxo.rs:210-855` | Patched (INC-I-096 WIP, gated) | Partial (A->B Swap: exact; B->A: NONE; RemoveLiq: patched token_out <= delta) | Partial (RemoveLiq proportional; Swap B->A: NONE; AddLiq: NONE) | NONE (T10 ignored) |
| apply_block | `bins/node/src/node/apply_block/tx_processing.rs:134-143` | Delegates to consensus | Delegates to consensus | Delegates to consensus | pool_id unique guard only |

### Key Metrics (from actual code)
- **Builder math helpers** (`crates/core/src/pool.rs`): 7 functions, 136 lines. Called by CLI + RPC ONLY. Zero calls from consensus validation or block assembly.
- **Consensus AMM validation** (`crates/core/src/validation/utxo.rs`): ~260 lines (595-855) of ad-hoc per-type checks. Inline k-invariant at line 648 duplicates `verify_invariant()` logic.
- **Structural validation** (`crates/core/src/validation/pool.rs`): 230 lines. No UTXO context. Validates output types, metadata decode, pool_id derivation, MINIMUM_LIQUIDITY.
- **Activation heights**: `inc_i_096_activation_height` exists (mainnet=`u64::MAX`, testnet=`u64::MAX`, devnet=0). `inc_i_092_activation_height` separate and immutable.

## Complete Failure-Mode Table

### Category: SAFETY failures (invalid tx accepted / value stolen)

| ID | Failure Mode | Type | Blast Radius | Status | Evidence |
|----|-------------|------|-------------|--------|----------|
| FM-S1 | **SEC-LOGIC-001: RemoveLiquidity 1-share full drain.** Attacker declares `new_total_lp=0`, burns 1 LP share, extracts entire pool reserves. `shares_burned = old_total_lp - new_total_lp` is attacker-controlled. | SAFETY (drain) | Full pool drain of both reserves. ALL LP holders lose everything. | BLOCKED by INC-I-096 proportional binding (gated, WIP). Unblocked without it. | utxo.rs:796-808 blocks proportional violation, BUT utxo.rs:796 computes `shares_burned` from DECLARED `new_total_lp`, not from consumed LPShare inputs. T10 test (#[ignore]) proves this is STILL exploitable via LP-input underburn. |
| FM-S2 | **SEC-LOGIC-001 variant: LP-input underburn.** Attacker holds 1 LP share, declares `new_total_lp=0` (pretending to burn 1000). Proportional binding passes because `shares_burned=1000` allows full drain. The consumed LPShare input is only 1 share. | SAFETY (drain) | Full pool drain. | OPEN. T10 test is `#[ignore]`. No LP-input binding exists. | utxo.rs:796 computes `shares_burned` without verifying LPShare inputs consumed. Test at `crates/core/tests/inc_i_096_amm_conservation.rs:852` proves it. |
| FM-S3 | **SEC-LOGIC-002: Swap B->A phantom token injection.** Attacker declares huge `new_reserve_b` (e.g., +9,999,000) with only 1 actual FungibleAsset token input. k-invariant satisfied with tiny `new_reserve_a`. Pool-aware DOLI conservation passes. No token_b conservation exists. | SAFETY (drain) | Drains reserve_a (DOLI). Pool's reserve_b inflated with phantom tokens, making all LP positions worthless. | OPEN. No token_b input binding for B->A swaps. | utxo.rs:677-694: B->A path has only trivial bound `doli_out <= old_reserve_a`. No check that FungibleAsset inputs back `new_reserve_b - old_reserve_b`. |
| FM-S4 | **AddLiquidity inflated LP minting.** Attacker adds 1 DOLI + 1 token, declares `new_total_lp_shares = old + 10_000_000`. Mints massive LPShare output. Validation checks only that reserves increased and LP increased -- not proportionality. | SAFETY (LP dilution) | All existing LPs are diluted. Attacker's inflated LP share claims disproportionate future reserves. | OPEN. No proportional LP minting check. | utxo.rs:705-743: checks `new_m.reserve_a >= old_m.reserve_a`, `new_m.reserve_b >= old_m.reserve_b`, `new_m.total_lp_shares > old_m.total_lp_shares`. No proportional binding. |
| FM-S5 | **AddLiquidity phantom reserve inflation (token_b side).** Attacker declares `new_reserve_b = old + 1_000_000` with only 1 token input. DOLI side is blocked by pool-aware conservation; token_b side has NO conservation. | SAFETY (reserve inflation) | Pool reserves inflated; LP proportional claims broken. | PARTIALLY BLOCKED: DOLI side blocked by pool-aware conservation; token_b side OPEN. | utxo.rs:230-256 (DOLI conservation); no token_b conservation. |
| FM-S6 | **Swap A->B token output inflation.** Attacker declares `new_reserve_b = old - 100` but outputs 200 FungibleAsset tokens. | SAFETY (token creation) | Tokens minted from nothing. | BLOCKED for A->B by strict equality check. | utxo.rs:671: `tokens_to_user != tokens_out_from_pool` enforces exact match. |
| FM-S7 | **Dust-rounding cumulative drain.** Over many txs, floor-division dust (`da = shares * reserve_a / total_shares`) rounds down. Each tx leaks dust INTO the pool (pool keeps the fraction). Not exploitable for draining, but an LP who deposits and immediately withdraws gets slightly less back. | SAFETY (minor value loss) | Dust per tx. Not exploitable at scale because dust stays in pool (benefits remaining LPs). | INHERENT in integer arithmetic. Not a real attack. | pool.rs:77: `da = (shares as u128) * (reserve_a as u128) / (total_shares as u128)`. |
| FM-S8 | **First-deposit donation attack (MINIMUM_LIQUIDITY interaction).** Attacker creates pool with 1000 DOLI + 1 token. `total_lp = sqrt(1000) = 31`. `MINIMUM_LIQUIDITY = 1000 > 31`. Pool rejected because `declared_total < MINIMUM_LIQUIDITY`. If attacker uses huge initial deposit to make `sqrt(amount_a * amount_b) > MINIMUM_LIQUIDITY`, the donation attack cost is real. | SAFETY (limited) | First depositor loses MINIMUM_LIQUIDITY shares permanently. This is BY DESIGN (Uniswap v2 pattern). With MINIMUM_LIQUIDITY=1000 and reasonable initial deposits, attack cost exceeds profit. | BLOCKED BY DESIGN. | pool.rs:111-121: validates `declared_total >= MINIMUM_LIQUIDITY` and `creator_share + MINIMUM_LIQUIDITY == declared_total`. |
| FM-S9 | **Cross-pool confusion: same asset pair, different fee_bps.** Two pools for DOLI/TKN at 30bps and 50bps. Different pool_ids (C3: `compute_pool_id` includes `fee_bps`). Could a Swap target one pool but output FungibleAssets from another? | SAFETY (theoretical) | Pool_id in Swap is validated: `old_meta.pool_id != new_meta.pool_id` rejects (utxo.rs:620-624). UTXO model prevents confusion: input[0] is a specific Pool UTXO. | BLOCKED by UTXO model + pool_id check. | utxo.rs:620-624 + C3. |
| FM-S10 | **u128 overflow in k-invariant.** If both reserves approach u64::MAX, `(u64::MAX as u128) * (u64::MAX as u128)` overflows u128. | SAFETY (consensus fork) | Nodes that overflow differently fork. | SAFE: `u64::MAX^2 = (2^64-1)^2 ~ 2^128 - 2^65`, fits in u128 (max 2^128 - 1). | utxo.rs:648-650. Mathematical proof: (2^64-1)^2 = 2^128 - 2^65 + 1 < 2^128 - 1. |
| FM-S11 | **Swap A->B asset_id mismatch: FungibleAsset output with wrong asset_id.** The A->B Swap token conservation check (utxo.rs:663-676) sums ALL FungibleAsset outputs regardless of `asset_id`. If an attacker creates FungibleAsset outputs with a DIFFERENT `asset_id` than the pool's `asset_b_id`, the equality check passes (amount matches), but the user receives tokens of a different (potentially valuable) asset counterfeit. The pool's actual `reserve_b` decreases correctly but the created tokens are of a foreign asset. | SAFETY (token counterfeiting) | Attacker can create FungibleAsset tokens of ANY asset_id, limited by pool's reserve_b decrease. Cross-pool attack: counterfeit valuable-token using cheap-token pool, then inject into valuable-token pool. | OPEN. No `asset_id` cross-check between FungibleAsset outputs and `pool.asset_b_id`. | utxo.rs:663-670: `.filter(o.output_type == OutputType::FungibleAsset)` has no asset_id filter. |
| FM-S12 | **AddLiquidity reserve-increase not bound to inputs (token_b).** Same as FM-S5. Declare `new_reserve_b = old + 1M` with 1 token input. No token_b conservation. | SAFETY (reserve inflation) | Pool state becomes inconsistent. Future withdrawals reference phantom reserves. | OPEN. | utxo.rs:731: only checks `new_m.reserve_b >= old_m.reserve_b`. |

### Category: LIVENESS failures (valid tx rejected)

| ID | Failure Mode | Type | Blast Radius | Status | Evidence |
|----|-------------|------|-------------|--------|----------|
| FM-L1 | **D1: RemoveLiquidity always rejected.** Native conservation blind to Pool reserve release. DOLI output counted in total_output but pool reserve delta invisible in total_input. | LIVENESS (total block) | ALL valid RemoveLiquidity rejected. Feature completely broken. | PATCHED by INC-I-096 pool-aware conservation (gated). Pre-gate: broken. | utxo.rs:230-256. Test T1 proves fix. Test T5 proves pre-gate rejection preserved. |
| FM-L2 | **D1: Swap B->A always rejected.** Same mechanism as FM-L1. DOLI released from reserve_a appears as Normal output with no covering native input. | LIVENESS (total block) | ALL valid B->A Swaps rejected. | PATCHED by INC-I-096 pool-aware conservation (gated). | utxo.rs:230-256. Test T9 proves B->A swap with fee change passes. |
| FM-L3 | **D2: Mempool/consensus input-count parity.** Pre-INC-I-096: mempool counts LPShare.amount and FungibleAsset.amount as native DOLI. Consensus filters them out. For RemoveLiquidity with LPShare(500): mempool sees total_input=1500 (1000 DOLI + 500 LP), consensus sees 1000. Mempool admits txs that consensus rejects (silent failure: blockHeight=None). | LIVENESS (silent failure) | User's tx enters mempool but never mines. No error feedback. | PATCHED by INC-I-096 `is_native_amount()` filter in mempool `calculate_inputs` (gated). | mempool/pool.rs:946-952, 976-982. |
| FM-L4 | **H1: Floor-division false reject.** If binding uses exact equality (`==`) instead of `<=`, ~50% of removes rejected due to integer truncation. | LIVENESS (intermittent) | Random valid RemoveLiquidity txs rejected. | PATCHED by INC-I-096 using `>` (greater-than) for proportional binding. | utxo.rs:803: `if actual_doli_delta > max_doli_delta` (not `!=`). Test T6 proves rounding accepted. |
| FM-L5 | **Fee-change output false reject.** If DOLI-output binding sums ALL native outputs (including fee change), valid removes with change are rejected. | LIVENESS (most real txs) | Nearly all real RemoveLiquidity/B->A Swap rejected (almost always have change). | AVOIDED by design: DOLI not separately bound for removes. Comment at utxo.rs:821-830 explains. | utxo.rs:821-830. |

### Category: DETERMINISM failures (consensus fork)

| ID | Failure Mode | Type | Blast Radius | Status | Evidence |
|----|-------------|------|-------------|--------|----------|
| FM-D1 | **Arithmetic divergence between builder and validator.** If builder uses different floor-division formula than validator's proportional binding, valid txs rejected on some nodes. | DETERMINISM (fork) | Chain split on first RemoveLiquidity with rounding. | LOW RISK: both use u128 integer division, deterministic. But builder uses `compute_remove_liquidity` (pool.rs:77-78) while validator inlines the same formula (utxo.rs:801-802). If these ever diverge: fork. | pool.rs:77 vs utxo.rs:801. Same formula currently. |
| FM-D2 | **Activation height deployed without matching code.** Node A has INC-I-096 code + active height; Node B has old code + no height field. Node A accepts RemoveLiquidity; Node B rejects. | DETERMINISM (fork) | Chain split at activation height. | MANAGED by activation height pattern. `inc_i_096_activation_height` exists with mainnet=`u64::MAX`. | network_params/defaults.rs:190,340,473. |
| FM-D3 | **`pool_metadata().unwrap_or(0)` silent failure.** If Pool UTXO has malformed extra_data, `pool_metadata()` returns `None`, conservation check uses `old_reserve_a=0`. `lhs = total_input + 0`, `rhs = total_output + 0`. Falls back to naive check. For AMM txs, this means the conservation check runs as if no pool exists. Could accept or reject depending on total_input vs total_output. | DETERMINISM (inconsistent) | Could cause divergent accept/reject if metadata parsing differs between nodes (e.g., different binary versions). | LOW RISK: metadata parsing is deterministic binary decode. But the `unwrap_or(0)` is a silent fallback that should probably be an explicit reject. | utxo.rs:238-244. |

### Category: DEPLOY failures

| ID | Failure Mode | Type | Blast Radius | Status | Evidence |
|----|-------------|------|-------------|--------|----------|
| FM-P1 | **Reusing `inc_i_092_activation_height` for INC-I-096 changes.** Heights are immutable once crossed (C4, INC-I-054). | DEPLOY (chain split) | All external producers fork. | AVOIDED: `inc_i_096_activation_height` is separate. | network_params/mod.rs:482. |
| FM-P2 | **Mixed fleet: external producers without new code.** ~30 external producers auto-update on different schedules. | DEPLOY (temporary fork) | Short fork until all producers update. | MANAGED by activation height + lead time. | CLAUDE.md external producers rule. |
| FM-P3 | **Activation height set to real value before LP-input binding lands (FM-S2).** If `inc_i_096` is activated before T10 drain is fixed, SEC-LOGIC-001 variant is unmasked. | DEPLOY (P0 drain exposed) | Pool drain possible between activation and fix. | CRITICAL GATE: must NOT activate until LP-input binding lands. | T10 test `#[ignore]`. |

## HARD FILTERS (for synthesizer to apply to ALL proposals)

### FILTER-1: ATOMIC LIVENESS-SAFETY COUPLING (CRITICAL)
**Any redesign MUST NOT fix liveness (admit RemoveLiquidity/B->A Swap) without simultaneously closing ALL drain vectors (FM-S1, FM-S2, FM-S3, FM-S4, FM-S11).**

Rationale: The buggy conservation check (D1) is currently the ONLY thing blocking SEC-LOGIC-001/002. It blocks them for the WRONG reason (it's blind to pool reserves), but the effect is that drain txs are rejected alongside valid ones. ANY change that makes the conservation check pool-aware UNMASKS the drains unless input binding is added in the same atomic change.

Kill-criteria: If a proposal fixes `total_input < total_output` to account for pool reserves but does NOT simultaneously add: (a) proportional reserve-delta binding, (b) LP-input binding (consumed LPShare amounts == declared shares_burned), (c) token_b input binding for B->A swaps and AddLiquidity, (d) proportional LP minting check for AddLiquidity, (e) asset_id cross-check on FungibleAsset outputs -- then the proposal FAILS this filter.

### FILTER-2: LP-INPUT BINDING IS MANDATORY
**Any redesign MUST bind `shares_burned = old_total_lp - new_total_lp` to the sum of consumed LPShare input amounts for the target pool_id.**

Rationale: FM-S2 (T10 drain). The current INC-I-096 proportional binding computes `shares_burned` from DECLARED `new_total_lp`, which the attacker controls. An attacker who holds 1 LP share can declare `new_total_lp=0`, making `shares_burned=old_total_lp`, and the proportional binding allows full drain because `1000/1000 * reserve = reserve`. The binding must verify that the tx actually CONSUMES LPShare inputs totaling `shares_burned`.

Evidence: T10 test (`#[ignore]`) at line 852 of `crates/core/tests/inc_i_096_amm_conservation.rs` proves this drain is OPEN.

### FILTER-3: TOKEN_B CONSERVATION IS MANDATORY (PER USER DIRECTION)
**Any redesign MUST enforce per-asset conservation for token_b (FungibleAsset) within AMM operations, scoped to AMM tx types. Conservation MUST match on asset_id (pool's `asset_b_id`), not just OutputType.**

Rationale: FM-S3 (SEC-LOGIC-002), FM-S5, FM-S11, FM-S12. Without token_b conservation, B->A Swap can inject phantom tokens to inflate `reserve_b` and drain `reserve_a` (DOLI). Token_b has NO system-wide conservation (not in `is_native_amount`). The conservation must be: for the specific `asset_b_id`, `sum(FungibleAsset inputs with matching asset_id) >= sum(FungibleAsset outputs with matching asset_id) + net_reserve_b_increase`.

This MUST NOT modify `is_native_amount()` or `total_output()` (VC-014 is explicitly WON'T scope).

### FILTER-4: MEMPOOL/CONSENSUS PARITY
**Any redesign MUST ensure mempool admission and consensus validation produce identical accept/reject decisions for all AMM transactions.**

Rationale: FM-L3. Divergence creates two failure modes:
- Mempool admits, consensus rejects: silent failure (blockHeight=None forever), no user feedback.
- Mempool rejects, consensus accepts: mempool DoS (attacker floods valid txs that peers reject).

Implementation: shared validation function called by both sites, OR identical logic replicated with a parity test. The current architecture has separate implementations that diverged (D2).

### FILTER-5: FLOOR-DIVISION TOLERANCE
**Any redesign MUST use `<=` (not `==`) for reserve-delta bindings, and the floor-division formula MUST be bit-identical between builder and validator.**

Rationale: FM-L4, FM-D1. Integer truncation means `shares * reserve / total_shares` loses the fractional part. Exact equality rejects ~50% of valid removes. The dust stays in the pool (benefits remaining LPs, never benefits the withdrawer). Builder and validator must use the same `u128` intermediate formula or nodes fork.

### FILTER-6: ACTIVATION HEIGHT ISOLATION
**Any redesign MUST use `inc_i_096_activation_height` (already allocated). MUST NOT reuse `inc_i_092_activation_height`. MUST NOT activate until ALL drain vectors (FM-S1 through FM-S5, FM-S11, FM-S12) are closed.**

Rationale: FM-P1, FM-P3. C4 (immutability) + C6 (INC-I-075 checklist). The existing `inc_i_096_activation_height` field exists in `NetworkParams` (mainnet=`u64::MAX`, devnet=0). Activation before drain fixes = catastrophic.

### FILTER-7: ADDLIQUIDITY PROPORTIONAL BINDING
**Any redesign MUST verify that LP shares minted are proportional to reserves added, AND that declared reserve increases match actual consumed inputs (both DOLI and token_b).**

Rationale: FM-S4, FM-S5, FM-S12. Current AddLiquidity validation (utxo.rs:705-743) checks only that reserves increased and LP increased. An attacker can add 1 DOLI + 1 token and mint millions of LP shares. Must bind: `new_lp - old_lp <= min(da * old_total / old_ra, db * old_total / old_rb)` (matching `compute_lp_shares` in pool.rs:57-58).

### FILTER-8: NO SILENT FALLBACKS ON MALFORMED METADATA
**Any redesign MUST explicitly reject AMM transactions where `pool_metadata()` returns `None`, rather than using `unwrap_or(0)` as a silent fallback.**

Rationale: FM-D3. The current `unwrap_or(0)` at utxo.rs:238-244 silently falls back to zero reserves on malformed Pool UTXOs, making the conservation check vacuous. Should be an explicit `ValidationError`.

### FILTER-9: ASSET_ID CROSS-CHECK ON FUNGIBLEASSET OUTPUTS
**Any redesign MUST verify that FungibleAsset outputs in Swap/AddLiquidity/RemoveLiquidity carry the pool's `asset_b_id`, not an arbitrary asset_id.**

Rationale: FM-S11. The current A->B Swap token conservation check (utxo.rs:663-670) sums ALL FungibleAsset outputs regardless of asset_id. An attacker can counterfeit tokens of a different (valuable) asset. The check must filter by `asset_b_id` from the pool being operated on.

## Attack-Vector Kill-Criteria for Each P0

### SEC-LOGIC-001 Kill-Criteria
An attacker constructing a RemoveLiquidity tx that burns fewer LP shares than declared MUST be rejected. Specifically:
1. Compute `lp_inputs_consumed = sum(LPShare inputs where lp_pool_id == pool_id).amount`
2. Compute `declared_burned = old_total_lp - new_total_lp`
3. Reject if `declared_burned > lp_inputs_consumed`
4. Reject if `actual_reserve_delta > (declared_burned * reserve / old_total_lp)` (proportional binding)
5. Reject if `tokens_out > actual_reserve_b_delta` (where tokens_out is filtered by matching asset_b_id)

All 5 checks are necessary. Without check 3, FM-S2 remains. Without check 4, FM-S1 remains. Without check 5, token inflation remains.

### SEC-LOGIC-002 Kill-Criteria
An attacker constructing a B->A Swap that injects phantom tokens MUST be rejected. Specifically:
1. Compute `token_b_inputs = sum(FungibleAsset inputs where asset_id == pool.asset_b_id).amount`
2. Compute `declared_reserve_b_increase = new_reserve_b - old_reserve_b`
3. Reject if `declared_reserve_b_increase > token_b_inputs` (token not backed by actual inputs)
4. The k-invariant (`new_k >= old_k`) remains as a secondary guard
5. Pool-aware DOLI conservation (`native_input + old_ra >= native_output + new_ra`) remains
6. FungibleAsset outputs must have `asset_id == pool.asset_b_id` (prevents FM-S11 counterfeiting)

Check 3 is the critical addition. Without it, phantom token injection is trivial.

## Proposals

### P1: Unified AMM Validation Function — conf(0.65, observed)
- Evidence: Builder math helpers in `pool.rs` contain correct AMM math but are never called by validation. Consensus validation at `utxo.rs` reimplements ad-hoc checks per tx-type. This duplication is the root cause of the binding gaps.
- Complexity cost: +1 function (shared validator), -4 per-type ad-hoc blocks in utxo.rs. Net reduction.
- Kill test: Would a single validation function handle all 4 AMM tx types without becoming a god function? Answer: Yes -- the function needs ~5 inputs (old_pool_meta, new_pool_meta, consumed_lp_shares, consumed_token_inputs, native_delta) and dispatches by tx_type internally. ~100-150 lines.
- Kill test result: Not found. The function is tractable.
- Risk: Single point of failure. A bug in the unified function breaks all 4 AMM types simultaneously.
- Before/After: Before: 260 lines of per-type ad-hoc checks in utxo.rs. After: ~150-line shared function + per-type dispatch calls.

### P2: Input-Binding Layer as Pre-Validation Pass — conf(0.55, inferred)
- Evidence: The root pattern is "declared pool state trusted without binding to inputs." An input-binding pass that computes (actual DOLI consumed, actual token_b consumed, actual LP shares consumed) BEFORE the structural/invariant checks would make the structural checks operate on verified quantities.
- Complexity cost: +1 pass (input aggregation), 0 new modules. Adds ~50 lines before existing checks.
- Kill test: Does the input-binding pass have access to UTXO provider? Yes -- it runs inside `validate_transaction_with_utxos` which already has the provider.
- Kill test result: Not found. Viable.
- Risk: Must handle all 4 tx types' input layouts correctly (Pool at input[0], LPShare at input[1+], FungibleAsset scattered). Layout assumptions could break with future tx types.
- Before/After: Before: validation trusts declared values. After: validation first computes actual consumed amounts from UTXO lookups, then checks declared values against actual.

### P3: Do Not Activate Until ALL Drains Closed — conf(0.70, measured)
- Evidence: `inc_i_096_activation_height` is `u64::MAX` on mainnet and testnet. `amm_activation_height` is `u64::MAX` on mainnet. AMM is NOT live. There is NO production value at risk. The fix can land and be tested thoroughly before activation.
- Complexity cost: 0. Pure process constraint.
- Kill test: Is there any scenario where AMM could be activated before INC-I-096 is ready? Answer: Only if `amm_activation_height` is pinned to a real value before INC-I-096. Since they're independent heights, someone could activate AMM without INC-I-096. This is a failure mode.
- Kill test result: FOUND. The heights are independent. AMM could theoretically be activated without INC-I-096 protection. Need a compile-time or runtime assertion: `amm_activation_height >= inc_i_096_activation_height`.
- Risk: Someone sets `amm_activation_height` to a real value while INC-I-096 is still `u64::MAX`.
- Before/After: Before: two independent heights. After: linked assertion ensuring INC-I-096 is always <= AMM activation.

### P4: Token_b Conservation Scoped to AMM Tx Types — conf(0.60, observed)
- Evidence: FungibleAsset has no conservation anywhere. `is_native_amount()` excludes it. For AMM tx types specifically, need: `sum(FungibleAsset inputs for asset_b_id) >= sum(FungibleAsset outputs for asset_b_id) + net_reserve_b_increase`. This is scoped to AMM types, NOT a system-wide change.
- Complexity cost: +1 per-asset aggregation (~30 lines per AMM type, or ~60 lines shared).
- Kill test: Does this break non-AMM FungibleAsset operations (MintAsset, BurnAsset, Transfer of tokens)? Answer: No -- the conservation is ONLY for AMM tx types (CreatePool, Swap, AddLiquidity, RemoveLiquidity). MintAsset/BurnAsset have their own validation.
- Kill test result: Not found. Safe scope.
- Risk: Must correctly identify `asset_b_id` from the pool being operated on. Must handle token change outputs correctly (change has matching asset_id and must be counted on output side).
- Before/After: Before: no token_b conservation. After: per-asset balance check for AMM operations, filtering by `asset_b_id`.

## Constraints Identified

1. **INV-SAFETY-001**: DOLI conservation equation for AMM: `native_input + old_reserve_a >= native_output + new_reserve_a`. ANY redesign must preserve this (it blocks DOLI creation from nothing).
2. **INV-SAFETY-002**: k-invariant for Swap: `new_k >= old_k`. ANY redesign must preserve this (it bounds extraction).
3. **INV-SAFETY-003**: MINIMUM_LIQUIDITY = 1000 permanently locked in first LP mint. ANY redesign must preserve.
4. **INV-SAFETY-004**: Pool_id derivation includes fee_bps. Irreversible post-activation (C3). No proposal can change this.
5. **INV-DEPLOY-001**: `inc_i_096_activation_height` MUST NOT be set to a real value until ALL FILTER-1 through FILTER-9 are satisfied.
6. **INV-DEPLOY-002**: `amm_activation_height` MUST be >= `inc_i_096_activation_height` (prevent AMM going live without conservation).
7. **INV-COMPAT-001**: Below `inc_i_096_activation_height`, behavior MUST be bit-identical to current (buggy) behavior. No silent semantic changes.
8. **INV-DETERM-001**: All AMM arithmetic MUST use u64/u128 integer division. No floating point. Builder and validator MUST use the same formula.

## Cross-Perspective Signals

1. **For Pattern Analyst**: The "builder computes, validator trusts" pattern is the structural root cause. Builder helpers (`pool.rs`) encode correct AMM math. Validator (`utxo.rs`) does ad-hoc checks. A clean-slate redesign should likely have the validator CALL the builder math (or a shared library) rather than reimplementing it.
2. **For Coupling Analyst**: The mempool and consensus validation have separate implementations of the same logic. The INC-I-096 patch duplicates the pool-aware conservation in both. A shared function would reduce coupling.
3. **For Subtractionist**: The existing per-type structural validation in `pool.rs` (230 lines) does useful work (metadata decode, pool_id check, MINIMUM_LIQUIDITY). It should NOT be removed. But the per-type UTXO checks in `utxo.rs` (~260 lines) could potentially be replaced by a single unified function that handles all 4 AMM types.
4. **For Minimal Design**: The simplest correct change is ADDING input-binding checks alongside the existing structural checks, not replacing the architecture. But the user explicitly requested clean-slate design, not patches.
5. **For All**: The `unwrap_or(0)` pattern in conservation checks (utxo.rs:238-244) is a latent defect. Not currently reachable in production (Pool UTXOs always have valid metadata), but architecturally unsound.
6. **For All**: FM-S11 (asset_id mismatch on FungibleAsset outputs) is a new finding not in the original 6 defects. Any token conservation check MUST filter by the pool's `asset_b_id`, not just by `OutputType::FungibleAsset`.

## Gaps

1. Could not verify whether `spend_transaction` in apply_block performs additional validation beyond what consensus does. If it does, the constraint set is larger.
2. Did not trace the full CLI builder code (`bins/cli/src/cmd_pool.rs`) to verify that builder and validator use bit-identical formulas. Assumed identical based on both using `u128` division, but a line-by-line comparison was not performed.
3. Did not analyze rollback behavior: if a block containing AMM txs is rolled back, does the undo data correctly restore the Pool UTXO? This could be a separate failure mode.
4. Did not examine gossip propagation: if a tx passes one node's mempool but fails another's (during rolling deploy), could gossip propagation of the tx cause peer scoring penalties?
5. Examined FM-S11 (asset_id mismatch) for A->B Swap direction. Did NOT verify whether the same issue exists in RemoveLiquidity token output checking (utxo.rs:838-844: also filters by `OutputType::FungibleAsset` without asset_id check). Likely same vulnerability.
