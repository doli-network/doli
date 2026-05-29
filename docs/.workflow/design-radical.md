# Design Evaluation: Radical Simplifier

## Analysis Lens
First-principles minimum viable architecture for AMM value-conservation. Starting from acceptance criteria alone (VC-001..VC-009), ignoring the existing 9-check patchwork. Key question: what is the MINIMUM architecture that closes all 6 defects + token_b MUST?

## What I Don't Understand
1. Whether the `fungible_asset_metadata()` parser can reliably extract `asset_id` from ALL token_b UTXO variants consumed in Swap/RemoveLiquidity/AddLiquidity (it skips a condition prefix; if a FungibleAsset UTXO has a malformed condition prefix, does it return None and silently skip the token?).
2. Whether existing test infrastructure for `validate_transaction_with_utxos` can provide a `UtxoProvider` mock that covers multi-asset scenarios (the existing test file `inc_i_096_amm_conservation.rs` already does this, but I have not verified the token_b coverage depth).
3. How the mempool's `UtxoSet` API (`utxo_set.get(&outpoint)`) differs structurally from the consensus `UtxoProvider` trait — and whether a shared function can take both as input without a wrapper trait.
4. Whether the 25/5 bps fee split (LP vs protocol) happens at the builder layer only (CLI `cmd_pool.rs`) or whether consensus also needs to verify the protocol-fee extraction. This affects whether my conservation equation needs a protocol-fee term.

## Current State Analysis

### Conservation enforcement sites (measured)

| Site | File | Lines | What it checks |
|------|------|-------|----------------|
| Native conservation (pool-aware) | `utxo.rs:210-262` | 52 | DOLI input+old_reserve_a >= output+new_reserve_a |
| Swap structural+invariant | `utxo.rs:595-702` | 107 | pool_id, asset_b, fee_bps, LP supply preserved; k-invariant; partial token conservation (A->B only); trivial B->A bound |
| AddLiquidity structural | `utxo.rs:705-744` | 39 | pool_id, reserves increased, LP supply increased — NO input binding |
| RemoveLiquidity structural+binding | `utxo.rs:747-854` | 107 | pool_id, reserves decreased, LP supply decreased; proportional binding (INC-I-096 gated); token output binding |
| CreatePool RC-B | `utxo.rs:869-918` | 49 | Declared reserves backed by net inputs |
| Mempool native conservation | `pool.rs:380-437` | 57 | Duplicates utxo.rs pool-aware conservation |
| Mempool calculate_inputs parity | `pool.rs:926-982` | 56 | is_native_amount filter (gated) |
| Structural validation (pool.rs) | `validation/pool.rs:1-230` | 230 | Input/output counts, output types, MINIMUM_LIQUIDITY, pool_id derivation |

**Total conservation logic across sites**: ~467 lines in utxo.rs + ~113 lines in mempool + ~230 lines structural = ~810 lines.

**Number of declared-state trust points** (where attacker-controlled output fields are checked against inputs):
- Current: 5 partial (pool_id preserved: 3 sites; reserves direction: 2; proportional binding: 1 gated; token output: 2 partial; k-invariant: 1). Each tx type has its OWN custom check with different coverage gaps.
- D1 (blind to reserves): patched at 2 sites (mempool + consensus)
- D2 (parity): patched at 1 site (mempool `calculate_inputs`)
- D3 (RemoveLiquidity unbound): patched at 1 site (proportional binding)
- D4 (Swap B->A unbound): NOT fully patched (trivial bound `doli_out_from_pool > old_meta.reserve_a`, no real input binding)
- H2 (token_b conservation): partially patched (Swap A->B tokens_to_user==tokens_out_from_pool; RemoveLiquidity tokens_out <= actual_token_delta)

### Structural diagnosis

The root problem is **fragmentation**: each tx type has bespoke checks scattered across utxo.rs (lines 595-918), and the mempool duplicates the DOLI conservation at a different abstraction level. Conservation is NOT a single principled equation; it is an accumulation of ad-hoc patches. This means:

1. **Adding a new AMM tx type** (e.g., concentrated liquidity) requires adding conservation checks at 3+ sites.
2. **Verifying correctness** requires auditing 5 independent code blocks, each with its own data-extraction logic.
3. **Parity** between mempool and consensus is maintained by manual duplication, not shared code.

## First-Principles Derivation

### The Single-Equation Insight

In a UTXO model, an AMM transaction consumes a set of inputs (including the old Pool UTXO) and produces a set of outputs (including the new Pool UTXO). Three asset classes flow through an AMM tx:

1. **DOLI** (native): tracked in `output.amount` for Normal/Bond types AND in `pool_metadata.reserve_a` for Pool type.
2. **token_b** (FungibleAsset): tracked in `output.amount` for FungibleAsset type AND in `pool_metadata.reserve_b` for Pool type.
3. **LP supply**: tracked in `output.amount` for LPShare type AND in `pool_metadata.total_lp_shares` for Pool type.

**The conservation law**: For EACH asset class independently:

```
sum(asset_in) >= sum(asset_out)
```

Where `asset_in` and `asset_out` include BOTH explicit UTXOs AND pool reserve contributions.

### Per-asset balance equations

Define helper functions:
```
doli_value(utxo) = utxo.amount if is_native_amount(utxo.output_type), 
                   else pool_metadata.reserve_a if Pool, else 0
token_value(utxo, asset_b_id) = utxo.amount if FungibleAsset with matching asset_id, 
                                 else pool_metadata.reserve_b if Pool, else 0
lp_value(utxo, pool_id) = utxo.amount if LPShare with matching pool_id, 
                            else pool_metadata.total_lp_shares if Pool, else 0
```

Then for each AMM tx:
```
DOLI:    sum_inputs(doli_value) >= sum_outputs(doli_value)         ... (E1)
token_b: sum_inputs(token_value) >= sum_outputs(token_value)       ... (E2)
LP:      sum_inputs(lp_value) >= sum_outputs(lp_value)             ... (E3, only for Remove)
         sum_inputs(lp_value) <= sum_outputs(lp_value)             ... (E3', only for Add)
         sum_inputs(lp_value) == sum_outputs(lp_value)             ... (E3'', only for Swap)
```

The `>=` in E1/E2 absorbs floor-division dust (goes to pool). The difference is the implicit fee for DOLI or slippage dust for token_b.

### How this kills each defect

| Defect | How single-equation fixes it |
|--------|------------------------------|
| D1 (reserves invisible) | Pool reserves are explicitly included in `doli_value`/`token_value` sums |
| D2 (mempool/consensus parity) | Both sites call the SAME `verify_amm_conservation()` function |
| D3 (RemoveLiquidity unbound) | E1 bounds DOLI out: user cannot extract more DOLI than (old_reserve_a - new_reserve_a). E3 bounds shares_burned to actual LPShare inputs. Together they cap proportional withdrawal. |
| D4 (Swap B->A unbound) | E2 bounds token_b: declared new_reserve_b increase MUST be covered by FungibleAsset inputs. E1 bounds DOLI out to reserve_a release. |
| H1 (floor division) | `>=` (not `==`) absorbs truncation. Dust stays in pool. |
| H2 (token_b conservation) | E2 is a first-class equation on the same footing as E1. |

### Is the k-invariant still needed?

**Yes.** Conservation prevents THEFT (no asset created from nothing). The k-invariant prevents VALUE LEAK via mispricing. Consider: with conservation alone, an attacker could create a Swap where `new_reserve_a = old_reserve_a + 1000` and `new_reserve_b = old_reserve_b - 999`, satisfying conservation on both sides, but the swap rate (1000 for 999) is far from the market rate (it should be ~997 with 30bps fee). The k-invariant bounds the output to the correct AMM curve.

**Minimum model = 3 conservation equations (E1, E2, E3) + k-invariant for Swap.** These are independent: conservation prevents creation-from-nothing; k-invariant prevents mispricing.

### Per-tx-type equations

**CreatePool**: E1 (DOLI conservation: inputs fund reserve_a), E2 (token conservation: inputs fund reserve_b). LP: structural check (MINIMUM_LIQUIDITY). Already covered by RC-B + structural pool.rs.

**AddLiquidity**: E1 (native_in + old_reserve_a >= native_out + new_reserve_a), E2 (token_in + old_reserve_b >= token_out + new_reserve_b), E3' (LP supply must increase; new LPShare outputs must not exceed the delta `new_total_lp - old_total_lp`).

**RemoveLiquidity**: E1, E2 (with >= direction), E3 (LP_in >= LP_out; where LP_in includes LPShare inputs + old_total_lp_from_pool, LP_out includes LPShare outputs + new_total_lp_from_pool. Since there are typically no LPShare outputs, this becomes: LPShare_input_amount + old_total_lp >= new_total_lp, i.e., LP shares burned = LPShare inputs consumed).

**Swap**: E1, E2, E3'' (LP supply unchanged), plus k-invariant (new_k >= old_k).

## Proposals

### P1: Single `verify_amm_conservation()` function — conf(0.65, observed)

**The proposal**: Create ONE function in `crates/core/src/validation/amm_conservation.rs` (~100-120 lines) that takes a `Transaction` + consumed UTXOs (as a slice of `(Input, Output)` pairs) and returns `Result<(), ValidationError>`. It implements E1/E2/E3 + k-invariant as described above. Both mempool and consensus call it identically.

**Evidence**:
- Current: 5 separate check blocks in utxo.rs (354 lines) + 2 duplicated blocks in mempool pool.rs (113 lines) = 467 lines of conservation logic across 2 crates.
- Proposed: 1 function (~120 lines) in 1 new file, called from 2 sites (mempool + utxo.rs) with ~5 lines of call-site glue each = ~130 total lines.
- The existing `pool.rs` builder math (`compute_swap`, `compute_remove_liquidity`, `verify_invariant`) is NOT reused by this function; the conservation check is INDEPENDENT of the builder math (conservation checks balance, not correctness of AMM pricing).

**Complexity cost**:
- +1 new module (`amm_conservation.rs`)
- +1 new public function (`verify_amm_conservation`)
- -5 scattered check blocks in utxo.rs (lines 595-918 reduce to ~10 lines of delegation)
- -2 duplicated blocks in mempool pool.rs (reduce to ~5 lines)
- Net: +1 module, -6 scattered code blocks, -330 lines

**Kill test**: What if the consumed UTXO data is not available to the mempool at the call site (mempool has `UtxoSet` not `UtxoProvider`)?

**Kill test result**: The mempool already resolves `pool_metadata()` from its `utxo_set.get(&outpoint)` at line 404. The function signature can take `&[(Output,)]` (just the outputs of consumed UTXOs), not the full `UtxoProvider` trait. The mempool can construct this slice from its existing lookups. NOT killed.

**Kill test 2**: What if the LP conservation equation (E3) for RemoveLiquidity falsely rejects legitimate transactions where the user burns LP shares from multiple LPShare UTXOs?

**Kill test result**: The function sums ALL LPShare inputs for the matching pool_id. Multiple LPShare UTXOs are naturally summed. The consumed UTXOs are already iterated in the input loop (utxo.rs line 92). NOT killed.

**Risk**: The refactor changes the shape of error messages (new error codes), which could affect monitoring/alerting. Mitigated by preserving error type variants (InvalidSwap, InvalidLiquidity, etc.) and gating behind `inc_i_096_activation_height`.

**Before/After**:
```
BEFORE: 5 check blocks in utxo.rs (354 lines) + 2 mempool blocks (113 lines)
        Each tx type has bespoke extraction, bespoke conditions, bespoke error messages.
        token_b partially covered. LP binding only for RemoveLiquidity.

AFTER:  1 function in amm_conservation.rs (~120 lines) + 2 call sites (~10 lines each)
        Uniform per-asset conservation for all 4 tx types.
        token_b fully covered. LP binding for all tx types.
```

### P2: Retain structural validation in pool.rs, unmodified — conf(0.7, observed)

**The proposal**: The existing structural checks in `crates/core/src/validation/pool.rs` (230 lines) are ORTHOGONAL to conservation. They validate tx shape (output counts, output types, MINIMUM_LIQUIDITY, pool_id derivation). Do NOT touch them. They are correct and independent.

**Evidence**: Pool.rs checks are structural: they fire BEFORE UTXO resolution (no `UtxoProvider` needed). Conservation is a UTXO-context concern. These are separate layers. The existing pool.rs tests pass and cover the structural invariants.

**Complexity cost**: +0 (no change).

**Kill test**: What if some structural check in pool.rs overlaps with the conservation function, causing double-rejection of the same invariant?

**Kill test result**: Pool.rs checks are output-shape-only (e.g., "first output must be Pool type", "fee_bps < max", "MINIMUM_LIQUIDITY gap = 1000"). The conservation function checks value flows (sum of assets in >= sum out). Zero overlap. NOT killed.

**Risk**: None; this is the conservative choice.

### P3: Mempool calls `verify_amm_conservation` via adapter, eliminating duplication — conf(0.6, observed)

**The proposal**: The mempool currently duplicates pool-aware DOLI conservation (pool.rs:380-437) and the `is_native_amount` filter (pool.rs:926-982). Instead, the mempool constructs the consumed-UTXO slice from its `UtxoSet` lookups and calls `verify_amm_conservation()`. This eliminates D2 by construction.

**Evidence**: The mempool already resolves each input UTXO at line 940-961 (`utxo_set.get(&outpoint)`). It already has access to every consumed output. The only gap: the mempool gets `Utxo` (from `UtxoSet`) while consensus gets `UtxoInfo` (from `UtxoProvider`). Both contain `output: Output`. The shared function only needs `&Output`, so both callers can provide it.

**Complexity cost**:
- +1 adapter pattern (mempool extracts `Vec<Output>` from its UTXO lookups)
- -57 lines of duplicated pool-aware conservation in mempool
- -56 lines of duplicated `is_native_amount` gating in mempool
- Net: cleaner, but requires the mempool to resolve ALL input UTXOs upfront (it already does this in `calculate_inputs`).

**Kill test**: What if the mempool's UTXO set contains parent-chain mempool entries (unconfirmed outputs) that consensus would not see?

**Kill test result**: The mempool already handles this at line 940 ("First check if output is in mempool"). The shared conservation function operates on resolved `Output` structs regardless of source (confirmed UTXO set or mempool parent). NOT killed.

**Risk**: The mempool currently computes `fee` as a by-product of the conservation check (line 401-427). If conservation is delegated, the fee computation needs a separate path. This is manageable: `verify_amm_conservation` returns the DOLI surplus as part of its result, which IS the fee.

### P4: k-invariant as a sub-check within the conservation function, not separate — conf(0.55, inferred)

**The proposal**: Include the k-invariant check (`new_k >= old_k`) inside `verify_amm_conservation()` for Swap tx types, rather than leaving it as a standalone block in utxo.rs.

**Evidence**: The k-invariant is already checked in utxo.rs lines 648-655 for Swap. Moving it inside the shared function means ONE function contains ALL AMM value-safety logic. Current location: interleaved with structural checks (pool_id, asset_b, fee_bps preservation). The structural checks can remain in the shared function (they are cheap, ~5 comparisons).

**Complexity cost**: +0 (moving existing code, not adding new code).

**Kill test**: What if a future tx type (e.g., limit orders, concentrated liquidity) needs the k-invariant with different semantics?

**Kill test result**: The function dispatches on `tx.tx_type`, so future types can have their own invariant variant. The k-invariant's exact form (`new_k >= old_k`) is specific to constant-product AMMs. If a future curve type changes this, the function grows a new match arm. Acceptable. NOT killed.

**Risk**: Coupling the k-invariant with conservation could make unit testing harder. Mitigated: the function is still a pure function of (Transaction, consumed UTXOs, height).

### P5: Return the surplus per asset class, enabling mempool fee computation — conf(0.5, inferred)

**The proposal**: `verify_amm_conservation()` returns `Result<AmmConservationResult, ValidationError>` where `AmmConservationResult` contains `doli_surplus: u64` (= the implicit fee) and `token_b_surplus: u64` (= dust to pool). The mempool uses `doli_surplus` as the fee; consensus ignores it (AMM txs are fee-exempt).

**Evidence**: The mempool currently computes fee at lines 401-427 as `(total_input + old_reserve_a) - (total_output + new_reserve_a)`. This IS the DOLI surplus from E1. If the conservation function returns this value, the mempool avoids duplicating the computation.

**Complexity cost**: +1 struct (`AmmConservationResult`), +0 lines net.

**Kill test**: What if the mempool needs the fee BEFORE calling conservation (e.g., for prioritization sorting)?

**Kill test result**: The mempool sorts by fee_rate which requires fee. Currently, fee is computed inline (lines 401-427). The proposed function computes it as part of conservation. The mempool would call conservation first, then use the returned surplus as fee. This changes the order of operations but not the result. The mempool would reject invalid txs before sorting (which it already does). NOT killed.

**Risk**: Adding a return struct increases the API surface. Mitigated: the struct is trivial (2 fields).

## Complexity Comparison Table

| Metric | Current (patch-set) | Proposed minimum (P1+P2+P3) |
|--------|--------------------|-----------------------------|
| Conservation enforcement sites | 3 (mempool, utxo.rs native, utxo.rs per-type) | 1 (shared function, called from 2 sites) |
| Lines of conservation logic | ~467 (354 utxo.rs + 113 mempool) | ~130 (120 function + 10 call-site glue) |
| Declared-state trust points | 5 partial (varying coverage by tx type) | 3 universal (E1, E2, E3 cover ALL tx types) |
| Per-tx-type special cases | 4 bespoke blocks (Swap 107L, Add 39L, Remove 107L, Create 49L) | 1 dispatch with 4 arms (~20L each = ~80L total, inside 1 function) |
| Mempool/consensus parity mechanism | Manual duplication (113 lines mirror) | Shared function call (guaranteed parity) |
| token_b conservation coverage | Partial (Swap A->B: exact; Swap B->A: trivial; Remove: gated) | Full (E2 applies uniformly) |
| LP supply conservation | RemoveLiquidity only (gated) | All 4 tx types (Create: structural; Add: increase bound; Remove: decrease bound; Swap: unchanged) |
| k-invariant location | utxo.rs:648-655 (Swap only, inline) | Inside shared function (Swap only, same logic) |
| New modules | 0 (patches in-place) | 1 (`amm_conservation.rs`) |
| Functions added | 0 | 1 (`verify_amm_conservation`) |
| Functions deleted | 0 | 0 (check blocks become delegation calls) |
| Code blocks removed | 0 | 7 (5 in utxo.rs + 2 in mempool pool.rs) |

## Constraints Identified

1. **C10 (Pool amount=0)**: The conservation function MUST extract reserves from `pool_metadata()`, not from `output.amount`. This is already the pattern in the existing code. The function cannot assume `amount` carries reserves.

2. **C8 (Floor-division dust)**: Conservation uses `>=` not `==`. This absorbs up to `(total_shares - 1) / total_shares` units of dust per asset per operation. The dust stays in the pool (benefits remaining LPs).

3. **C11 (Integer determinism)**: All arithmetic is u64/u128. No floats. The conservation function must use `u128` for intermediate sums to avoid overflow when adding large reserves to large native amounts.

4. **C12 (INC-I-092 RC-A preserved)**: The sig/fee exemption for AMM pool inputs at utxo.rs:164-170 is ORTHOGONAL to conservation. Do not touch it.

5. **C6/C7 (Activation gating)**: The shared function takes `height` and `activation_height` as parameters. Below the gate, the existing (buggy) per-type checks remain in place. Above the gate, the shared function takes over. This preserves bit-identical behavior for blocks before the activation height.

6. **Mempool UtxoSet vs consensus UtxoProvider**: The shared function must accept consumed outputs as data (e.g., `&[Output]`), not as a trait. This allows both callers (mempool via `UtxoSet`, consensus via `UtxoProvider`) to construct the input data.

7. **Fee computation coupling**: The mempool needs the DOLI surplus for fee computation. The conservation function should return it (P5), or the mempool must compute it separately. Either way, the conservation check must happen before the fee is used.

8. **CreatePool retains RC-B**: The existing RC-B check (utxo.rs:869-918) is subsumed by E1+E2 in the shared function. But RC-B was shipped with `inc_i_092_activation_height`. The shared function (gated at `inc_i_096`) supersedes it for heights >= inc_i_096. For heights between inc_i_092 and inc_i_096, RC-B remains active. This layering is safe.

## Cross-Perspective Signals

1. **Dead code opportunity**: If the shared conservation function subsumes the per-type blocks in utxo.rs (lines 595-918), those blocks can be gated to only run below `inc_i_096_activation_height`. Above it, they are dead. A subtractionist might propose removing them entirely after the activation height is crossed on all networks.

2. **Pattern concern**: The existing code at utxo.rs:660-694 handles Swap directions (A->B vs B->A) with different logic. The B->A branch deliberately does NOT bind DOLI outputs to reserve_a delta (line 680-693 comment). The conservation equation E1 replaces this entirely: E1 binds DOLI output + new_reserve_a to DOLI input + old_reserve_a, regardless of direction. This is a directional coverage improvement that a coupling evaluator might want to trace through E2E tests.

3. **Builder math reuse**: The brief mentions that "consensus NEVER calls builder helpers". The proposed conservation function also does NOT call builder helpers (compute_swap, compute_remove_liquidity). Conservation is independent of pricing. However, VC-010 (SHOULD) asks for consensus to re-verify AMM math. This is a separate concern from conservation and could be addressed by calling `verify_invariant()` from the conservation function for Swap types (already proposed in P4).

4. **Mempool contention tests**: `crates/mempool/src/contention_tests.rs` references `inc_i_096`. Any refactor of mempool conservation must update these tests.

## Gaps

1. I did not trace the apply_block path (`tx_processing.rs`, `validation_checks.rs`) in detail. The brief lists them as enforcement sites, but from my grep, their inc_i_096 references are for ValidationContext construction (passing the activation height to the validator), not for independent conservation logic. This should be verified.

2. I did not examine the builder code in `bins/cli/src/cmd_pool.rs` that constructs AMM transactions. Understanding how the builder handles fee change and token change outputs would strengthen the E1/E2/E3 equations. The fee change concern (utxo.rs line 822-829) suggests that DOLI change outputs and swap proceeds are mixed in the user's Normal outputs.

3. I did not verify whether the existing `pool_metadata()` parser correctly handles ALL edge cases (truncated extra_data, wrong version byte) under adversarial conditions. The conservation function relies on it returning `None` for malformed data, which would cause the `unwrap_or(0)` pattern to treat a malformed Pool UTXO as having zero reserves — potentially allowing theft if reserves are hidden by a crafted malformed pool.

4. The interaction between `inc_i_096_activation_height` and `amm_activation_height` (both currently `u64::MAX` on mainnet) means the order of activation matters. If AMM activates before I-096, there is a window where the old (buggy) conservation runs. This is a deployment sequencing concern, not an architecture concern, but any redesign must ensure `inc_i_096_activation_height <= amm_activation_height` or they activate simultaneously.
