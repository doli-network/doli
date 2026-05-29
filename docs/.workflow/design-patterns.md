# Design Evaluation: Pattern Matcher

## Analysis Lens
Industry patterns, codebase patterns, and anti-patterns applied to the AMM value-conservation layer. Central question: **What KNOWN PATTERN solves this class of problem, and what anti-pattern is the current design?**

## What I Don't Understand
1. **LP share input verification scope**: Does the validator verify that LPShare inputs consumed in RemoveLiquidity actually sum to `shares_burned`? The proportional binding in the INC-I-096 patch computes `shares_burned = old_total_lp - new_total_lp` from the DECLARED output, but I did not find where the consumed LPShare UTXO amounts are verified to equal `shares_burned`. If this binding is missing, `shares_burned` is still attacker-controlled.
2. **Fee-change output distinguishability**: The existing patch comments (utxo.rs:682-688) say the user's Normal outputs "mix swap proceeds with DOLI fee change" making them indistinguishable. Is the fee-change output architecturally necessary, or is it an artifact of the CLI builder that a clean-slate design could eliminate by requiring exact-amount inputs?
3. **AddLiquidity LP minting binding**: Is there any check that `new_total_lp - old_total_lp` (the LP shares minted) is proportional to the reserve deltas? I found no such check. This means an attacker could declare `new_total_lp = old_total_lp + 1` while adding large reserves, diluting all existing LPs.
4. **Protocol fee extraction path for Swaps**: The brief mentions a 25/5 bps LP/protocol fee split. Where does the protocol fee physically leave the pool state? If it stays in reserves, it inflates k. If it is extracted as a separate output, it must be accounted in the conservation equation.

## Current State Analysis

### Anti-Pattern Identification: "Trust the Client's Computed Result" (TCCR)

The current AMM validation exhibits a well-known anti-pattern variously called:
- **"Trust the client's computed result"** (web security)
- **"Client-side authority"** (game development)
- **"Oracle-free declared state"** (blockchain/DeFi)

In all cases, the pattern is identical: **the system accepts a declared new state from an untrusted party instead of deriving it from the transition inputs.**

**Evidence in the DOLI codebase** (measured, not inferred):

| AMM Operation | What validator TRUSTS (declared) | What validator should DERIVE (from inputs) |
|---|---|---|
| Swap B->A | `new_reserve_b` (attacker sets it) | `old_reserve_b + actual_token_inputs` |
| Swap A->B | `new_reserve_a` (attacker sets it) | `old_reserve_a + actual_doli_inputs` (partially covered by native conservation) |
| RemoveLiquidity | `new_total_lp` (attacker sets it) | `old_total_lp - sum(consumed LPShare UTXOs)` |
| AddLiquidity | `new_reserve_a`, `new_reserve_b`, `new_total_lp` (all declared) | `old_reserve_a + net_doli_in`, `old_reserve_b + net_token_in`, computed LP shares |

The TCCR anti-pattern appears at **4 out of 4** AMM tx types. Only CreatePool (INC-I-092 RC-B, utxo.rs:857-918) has been fixed — and it was fixed with the CORRECT pattern: the validator recomputes `net_doli_in` and `net_token_in` from actual consumed UTXOs and checks `net_doli_in >= declared_reserve_a`.

### Quantified Scope of the Anti-Pattern

- **9 declared values** across 4 AMM types are trusted without derivation
- **1 of 9** (CreatePool reserve backing) has been fixed (RC-B)
- **3 enforcement sites** (mempool, consensus, apply_block) with divergent logic
- **2 separate conservation equations** needed: DOLI (native) and token_b (non-native)
- **0 of 4** AMM types have token_b input binding at consensus level (A->B swap has OUTPUT binding only)

### Codebase Pattern Inventory: "Validator Recomputes"

DOLI already uses the correct pattern in multiple places. The validator independently computes the expected result and checks equality/inequality:

1. **Coinbase conservation** (`validation_checks.rs:446-529`): validator computes `expected = block_reward + extra_fees`, checks `coinbase_amount == expected`. conf(0.7, observed)
2. **INC-I-092 RC-B CreatePool** (`utxo.rs:857-918`): validator computes `net_doli_in = native_input - native_change_out`, checks `net_doli_in >= pool_meta.reserve_a`. This is **the exact pattern** that should be generalized. conf(0.7, observed)
3. **Native DOLI conservation** (`utxo.rs:210-262`): `sum(native_inputs) >= sum(native_outputs)`. The canonical Bitcoin conservation law. conf(0.7, observed)
4. **Pool ID derivation** (`pool.rs:68-79`): validator recomputes `expected_pool_id = compute_pool_id(...)`, checks `pool_meta.pool_id == expected_pool_id`. conf(0.7, observed)
5. **MINIMUM_LIQUIDITY lock** (`pool.rs:111-121`): validator verifies `creator_share + MINIMUM_LIQUIDITY == declared_total`. conf(0.7, observed)

The codebase has the RIGHT pattern in 5 places. The AMM validation simply failed to apply it to the other 4 tx types.

### Industry Pattern: "Derived State" in AMMs

**Account-model AMMs (Uniswap v2/v3, Balancer, Curve):**
The contract reads `reserve0` and `reserve1` from its own storage. The caller provides tokens via `transfer()`. The contract computes the new state: `new_reserve = old_reserve + transferred_amount`. The caller NEVER declares the new reserves. The contract derives them.

**UTXO-model analogue:**
In UTXO systems, there is no "contract storage." The consumed Pool UTXO's `extra_data` IS the "contract storage read." The actual token flows are the consumed/created UTXOs. The validator must:
1. Read old state from consumed Pool UTXO (input[0])
2. Measure actual flows from consumed non-Pool UTXOs and created outputs
3. Derive new state: `new_reserve = old_reserve +/- measured_flow`
4. Check declared output matches derived state (with tolerance for floor division)

This is identical to Cardano's "datum + redeemer + validator script" pattern, where the on-chain validator reads the previous datum (old state), measures the value flows in the transaction, and verifies the new datum (new state) is consistent.

**DOLI's `CreatePool` RC-B already implements step 1-4 for CreatePool.** The generalization is to apply the same 4-step pattern to Swap, AddLiquidity, and RemoveLiquidity.

### Industry Pattern: Multi-Asset Conservation (Cardano/Nervos CKB)

Cardano uses `Value = Map<PolicyId, Map<AssetName, Integer>>`. Every transaction must satisfy:
```
forall asset: sum(inputs[asset]) >= sum(outputs[asset])
```

This generalizes Bitcoin's single-asset conservation to N assets. DOLI's `is_native_amount()` is the single-asset version. The brief explicitly says a system-wide overhaul is out of scope (VC-014 WON'T), but the pattern can be applied BOUNDED to AMM tx types:

For each AMM tx type, define a per-asset conservation equation:
- **DOLI (asset_a)**: `native_input + old_reserve_a >= native_output + new_reserve_a`
- **token_b**: `token_input + old_reserve_b >= token_output + new_reserve_b`
- **LP shares**: `lp_input + old_total_lp >= lp_output + new_total_lp` (for Remove); `old_total_lp + lp_minted >= new_total_lp` (for Add)

This is the "per-asset value-conservation ledger" pattern, applied only at the AMM tx-type level.

### Floor Division Pattern: "Round in Protocol's Favor"

Uniswap v2 and Balancer both follow a consistent rounding rule: **truncate toward the pool (protocol)**. When computing `dy = reserve_b * dx_eff / (reserve_a + dx_eff)`, the integer division truncates DOWN, meaning the pool retains any fractional dust. The user gets slightly less than the mathematical result.

DOLI's `compute_remove_liquidity` (pool.rs:77-78) does the same: `da = shares * reserve_a / total_shares` truncates toward zero, meaning the user gets slightly less and the pool retains the dust. This is correct.

The bug the brief identifies (H1) is that validation uses `==` (exact equality) when comparing the user's output to the computed delta. Since the builder truncates, the user's output will be `<= computed_delta`. The fix pattern is:
- Validator computes `max_delta = shares * reserve / total_shares` using identical arithmetic
- Check: `actual_delta <= max_delta` (not `==`)
- The existing patch at utxo.rs:803 already uses `<=`, which is the correct pattern

However, the `<=` creates a second-order concern: the user could claim LESS than their proportional share. This is fine — it's a donation to the pool. The attacker cannot extract MORE because `<=` bounds the maximum.

## Proposals

### P1: Generalize RC-B ("derive new state from old state + measured flows") to all 4 AMM types — conf(0.65, observed)

**The pattern**: For each AMM tx type, the validator:
1. Reads `old_pool_meta` from consumed Pool UTXO (input[0]) — already done
2. Measures actual flows: `net_doli_in`, `net_token_in`, `lp_shares_consumed` from all non-Pool inputs and non-Pool outputs
3. Derives expected new state using the SAME math as `crates/core/src/pool.rs` (compute_swap, compute_remove_liquidity, compute_lp_shares)
4. Checks: `declared_new_reserve_a == derived_reserve_a` (with floor tolerance `<=`)
5. Checks: `declared_new_reserve_b == derived_reserve_b` (with floor tolerance `<=`)
6. Checks: `declared_new_total_lp == derived_total_lp` (exact for remove; with tolerance for add)

**Evidence**: RC-B at utxo.rs:857-918 already implements steps 1-4 for CreatePool. The generalization is mechanical.

**Complexity cost**: No new modules. One new shared function (e.g., `validate_amm_flows`) in `validation/utxo.rs` (~80-120 lines). Replaces 4 separate per-type ad-hoc blocks (~200 lines). Net reduction: ~80-100 lines.

**Kill test**: What would make this a bad idea? If the validator cannot measure actual flows without the UtxoProvider. But the validator already HAS the UtxoProvider (it's a parameter to `validate_transaction_with_utxos`), and RC-B already demonstrates the flow-measurement loop (utxo.rs:888-915). Kill test PASSED — no disabling evidence found.

**Risk**: The derived math must be IDENTICAL to the builder math, including truncation direction. If the validator uses `compute_swap()` but the builder uses a slightly different formula, valid transactions get rejected. Mitigation: use the same functions from `crates/core/src/pool.rs` in both builder and validator.

**Before**: 4 separate ad-hoc structural checks (reserves increased/decreased, LP changed direction) + 1 RC-B input-backing (CreatePool only). 9 declared values trusted.
**After**: 1 unified flow-measurement + derivation check for all 4 types. 0 declared values trusted — all derived from measured inputs and the canonical AMM math.

### P2: Dual-asset conservation equation ("per-asset ledger" bounded to AMM) — conf(0.6, observed)

**The pattern**: Define a `validate_amm_conservation(tx, utxo_provider, old_pool_meta, new_pool_meta)` function that checks:
```
For DOLI:  native_input + old_reserve_a >= native_output + new_reserve_a
For token_b: token_b_input + old_reserve_b >= token_b_output + new_reserve_b  
For LP:    lp_input + old_total_lp >= lp_output + new_total_lp  (remove)
           old_total_lp + lp_minted_output >= new_total_lp      (add)
```

This is the multi-asset conservation law from Cardano, scoped to AMM tx types only (no system-wide `is_native_amount` change).

**Evidence**: The DOLI half already exists in the INC-I-096 patch (utxo.rs:230-256). The token_b and LP halves are missing. The pattern is proven in Cardano's multi-asset ledger (Shelley formal specification, Section 9.1) and in Nervos CKB's cell model.

**Complexity cost**: +1 function (~60 lines), callable from both mempool and consensus. Subsumes the existing native conservation check for AMM types + all the ad-hoc structural checks. Net: replaces ~150 lines of scattered checks with ~60 lines of unified conservation.

**Kill test**: Does the fee-change output problem (utxo.rs:682-688 comment) break this? No — the DOLI conservation equation `native_input + old_reserve_a >= native_output + new_reserve_a` correctly accounts for fee change: the fee-change output is included in `native_output`, and the fee input is included in `native_input`. The inequality `>=` absorbs the fee. Kill test PASSED.

**Risk**: Token_b accounting requires matching `asset_id` from FungibleAsset metadata for each input/output. A pool with `asset_b_id = X` could receive tokens with `asset_id = Y` if the validator doesn't filter. Mitigation: filter by `asset_id == pool_meta.asset_b_id` (RC-B already does this at utxo.rs:892-894).

**Before**: 1 native conservation equation blind to Pool reserves + 0 token_b conservation + 0 LP conservation.
**After**: 3 conservation equations (DOLI, token_b, LP) per AMM tx, all in one function, shared by mempool and consensus.

### P3: Shared validation function between mempool and consensus — conf(0.65, observed)

**The pattern**: Extract AMM conservation logic into a single function in `crates/core/src/validation/` that both the mempool (`crates/mempool/src/pool.rs`) and the consensus validator (`crates/core/src/validation/utxo.rs`) call. This eliminates the D2 divergence by construction.

**Evidence of the divergence**: 
- Mempool `calculate_inputs` (pool.rs:927-992) sums ALL `output.amount` unconditionally pre-gate (line 951: `total += output.amount`). 
- Consensus `validate_transaction_with_utxos` (utxo.rs:185) filters `is_native_amount()`. 
- The INC-I-096 patch adds gated `is_native_amount()` filtering to the mempool, but the logic is copy-pasted, not shared. Copy-paste is an anti-pattern that creates future divergence risk.

**Industry pattern**: "Single Source of Truth for validation rules." Ethereum's EVM executes the same bytecode in mempool (pre-validation) and consensus (block validation). Bitcoin's `CheckTransaction()` is called by both `AcceptToMemoryPool` and `ConnectBlock`. Sharing the validation function is the canonical way to prevent mempool/consensus divergence.

**Complexity cost**: +1 shared function in `crates/core/src/validation/` (~40 lines). The mempool crate already depends on `doli_core`, so no new dependency. The mempool removes its copy-pasted conservation logic and calls the shared function instead.

**Kill test**: Can the mempool call a function from `crates/core/src/validation/`? Check the dependency graph.

Checking:
- `crates/mempool/Cargo.toml` depends on `doli-core` (it uses `doli_core::transaction`, `doli_core::consensus`, etc.)

Kill test PASSED — mempool already depends on doli-core.

**Risk**: The mempool may need slightly different error types than consensus. Mitigation: the shared function returns a bool or a generic result, and each caller wraps it in their own error type.

**Before**: 2 independent conservation implementations with different semantics (mempool counts all amounts; consensus filters by `is_native_amount()`). Divergence is structural.
**After**: 1 shared function, called from 2 sites. Divergence eliminated by construction.

### P4: Consensus calls builder math functions (validate via recomputation) — conf(0.55, inferred)

**The pattern**: Instead of ad-hoc inequality checks, the consensus validator calls the same `compute_swap()`, `compute_remove_liquidity()`, `compute_lp_shares()` functions from `crates/core/src/pool.rs` that the builder uses. It then checks that the declared output state matches the computed state (with floor-division tolerance).

**Evidence**: The brief notes: "Builder computes; validator trusts." The builder helpers in `pool.rs` contain the CORRECT AMM math (~7 functions, 136 lines) but consensus NEVER calls them. The validator does ad-hoc structural checks instead.

**Complexity cost**: +0 modules (functions already exist). +1 call per AMM tx type in the validator (~20 lines per type, ~80 total). Replaces the ad-hoc structural checks.

**Kill test**: Can the validator determine the swap direction and input amount (dx) from the transaction alone? For a Swap, dx is the difference between the native/token inputs and the change outputs. But this requires the validator to classify which outputs are "change" vs "swap result." The builder knows this; the validator must infer it. This is fragile.

Actually, a cleaner approach: the validator derives `expected_new_reserves` from `old_reserves + measured_net_flows`, then checks `new_k >= old_k`. It does NOT need to call `compute_swap()` directly — the conservation equations (P2) plus the k-invariant check are sufficient. Calling `compute_swap()` would be over-constraining: it would force the exact fee calculation, when the conservation law + k-invariant already bound the result.

Kill test result: **PARTIALLY KILLED**. Using `compute_swap()` directly is over-constraining for Swap. However, `compute_remove_liquidity()` and `compute_lp_shares()` could be used for LP operations where the math is simpler (proportional). Revised scope: use `compute_remove_liquidity` for RemoveLiquidity proportional cap; use conservation + k-invariant for Swap.

**Risk**: Over-constraining the validator to match the exact builder arithmetic rejects valid transactions built by external/older builders that use equivalent but not identical arithmetic. Mitigation: use `<=` tolerance, not `==`.

**Before**: Builder computes correct math; validator does ad-hoc structural checks.
**After**: Validator reuses builder math for LP operations; uses conservation + k-invariant for Swap.

### P5: LPShare input binding ("consumed shares == declared burn") — conf(0.65, observed)

**The pattern**: For RemoveLiquidity, the validator must verify:
```
sum(consumed LPShare UTXOs where pool_id matches) == old_total_lp - new_total_lp
```

Currently, `shares_burned = old_total_lp - new_total_lp` is computed from the DECLARED `new_total_lp` in the output Pool UTXO. An attacker declares `new_total_lp = 0`, making `shares_burned = old_total_lp`, but only consumes 1 LPShare UTXO worth 1 share. The proportional cap (`actual_doli_delta <= max_doli_delta`) then uses the inflated `shares_burned` to compute a huge `max_doli_delta`.

Wait — let me re-verify this. The proportional binding at utxo.rs:796-808:
```rust
let shares_burned = old_m.total_lp_shares - new_m.total_lp_shares;
let max_doli_delta = ((shares_burned as u128) * (old_m.reserve_a as u128)
    / (old_total_lp as u128)) as u64;
```

If `new_total_lp = 0` (attacker-declared), then `shares_burned = old_total_lp`, and `max_doli_delta = old_m.reserve_a`. The attacker can drain the entire reserve.

CONTRADICTION CHECK: The existing patch at utxo.rs:796 computes `shares_burned` from the declared output. Does anything else bind `new_total_lp` to consumed inputs?

Searching...

The check at utxo.rs:778-780:
```rust
if new_m.total_lp_shares >= old_m.total_lp_shares {
    return Err(...);
}
```
This only ensures LP shares DECREASED, not that the decrease matches consumed inputs.

**CONFIRMED**: `shares_burned` is attacker-controlled via `new_total_lp`. The INC-I-096 patch's proportional binding does NOT fully close the drain vector unless `shares_burned` is independently verified against consumed LPShare UTXOs.

This is the TCCR anti-pattern at its most critical: the proportional cap formula is correct, but its input is attacker-controlled.

**Evidence**: No code in utxo.rs verifies `sum(consumed LPShare amounts) == shares_burned`. The LPShare inputs are consumed (input[1+] with `output_type == LPShare`), and each has an `amount` field representing the share count. The validator must sum these and check equality.

**Complexity cost**: +5 lines in the RemoveLiquidity validation block.

**Kill test**: Can the validator access LPShare input amounts? Yes — `utxo_provider.get_utxo(...)` returns `UtxoInfo` which contains `output.amount` for LPShare UTXOs. Kill test PASSED.

**Risk**: If LPShare UTXOs for multiple pools are mixed in one transaction, the sum would be wrong. Mitigation: filter by `pool_id` match (LPShare metadata contains pool_id, verified via `lp_share_metadata()`).

**Before**: `shares_burned` computed from attacker-declared `new_total_lp`. Proportional cap is circumvented.
**After**: `shares_burned` derived from consumed LPShare UTXOs. Proportional cap becomes sound.

## Constraints Identified

1. **C-FLOOR**: All conservation `<=` checks must use the SAME truncation-direction arithmetic as the builder (toward zero / toward pool). Using `>=` or `==` causes false rejects on ~50% of legitimate transactions (H1).

2. **C-ACTIVATION**: All changes MUST be gated behind `inc_i_096_activation_height` with pre-gate behavior bit-identical to current code. Mainnet=u64::MAX, testnet=future, devnet=0 (C6, C7).

3. **C-SHARED-MATH**: If the validator calls pool.rs functions, those functions become consensus-critical. Any future change to `compute_swap()` etc. requires an activation height. Currently they are builder-only and non-consensus.

4. **C-FEE-CHANGE**: The DOLI conservation equation must use `>=` (not `==`) to absorb the fee-change output that mixes with swap proceeds in Normal outputs.

5. **C-LPSHARE-BINDING**: Any proportional cap that uses `shares_burned` MUST derive it from consumed LPShare UTXOs, NOT from `old_total_lp - new_total_lp`. This is a critical finding — the existing patch is vulnerable without this binding.

6. **C-TOKEN-FILTER**: token_b conservation must filter inputs/outputs by `asset_id == pool_meta.asset_b_id`. A transaction could include FungibleAsset UTXOs for different assets; only the matching ones count.

7. **C-NO-SYSTEM-REWRITE**: The `is_native_amount()` function and its users across 27 tx types MUST NOT be modified (VC-014 WON'T). The dual-asset conservation is AMM-scoped only.

## Cross-Perspective Signals

1. **For the Coupling Evaluator**: The mempool crate (`crates/mempool/src/pool.rs`) duplicates conservation logic from `crates/core/src/validation/utxo.rs`. This is coupling via copy-paste (the worst kind). The duplication created the D2 divergence bug.

2. **For the Dead Code Evaluator**: The builder math functions in `crates/core/src/pool.rs` (136 lines, 7 functions) are NEVER called by the consensus validator. They are "live code" in the builder but "dead" from the validator's perspective. Making the validator call them would make them consensus-critical (double-edged).

3. **For the Failure Mode Evaluator**: The `if let (Some(old_m), Some(new_m)) = ...` pattern at utxo.rs:723 silently passes if metadata parsing fails (None). This means a malformed Pool UTXO bypasses ALL structural checks for AddLiquidity. The `ok_or_else` pattern (used for Swap at line 611-613) is safer.

4. **For the Subtractionist**: The ad-hoc structural checks for each AMM type (reserves increased/decreased, LP changed direction) at utxo.rs:730-741 become REDUNDANT if a proper dual-asset conservation equation (P2) is implemented. The conservation law subsumes them — removal would simplify the code.

5. **For all evaluators**: The `shares_burned` binding gap (P5) may mean the existing patch-set is NOT secure even as a stopgap. If the user gated it at devnet=0 for testing, this drain vector is open on devnet.

## Gaps

1. **AddLiquidity LP minting proportionality**: I confirmed reserves are not input-bound, but did not fully trace whether an attacker can inflate reserves via AddLiquidity to extract them later via RemoveLiquidity. The attack requires two transactions — AddLiquidity (inflate reserves for free) then RemoveLiquidity (extract). This two-tx attack needs further analysis.

2. **Protocol fee extraction mechanics**: The 25/5 bps fee split path is not fully traced. If protocol fees are extracted as a separate output in Swap transactions, the conservation equation needs to account for them. I did not find the extraction code.

3. **apply_block enforcement**: The brief mentions `bins/node/src/node/apply_block/tx_processing.rs` as an enforcement site, but I focused on the consensus validation layer. apply_block may have additional checks or gaps that I did not examine.

4. **B->A swap token input binding completeness**: I confirmed the gap exists (validator does not bind `new_reserve_b` increase to actual token inputs), but did not quantify the exact attack parameters (minimum token input needed to pass the k-invariant while draining maximum DOLI).
