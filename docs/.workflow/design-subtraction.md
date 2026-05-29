# Design Evaluation: Subtractionist

## Analysis Lens
Dead code, unused abstractions, redundant checks, and unnecessary complexity in the AMM value-conservation layer. Key question: **What can be REMOVED to solve INC-I-096?**

## What I Don't Understand
1. Why does the mempool need to compute a fee for AMM txs if they are fee-exempt? The `fee_rate` computed at mempool:pool.rs:438 is used for eviction priority, but AMM txs are fee-exempt — so fee_rate is 0 or meaningless. The eviction ordering for AMM txs is unclear.
2. Whether a `MempoolUtxoProvider` wrapper (combining mempool entries + UtxoSet) exists or has been attempted before. If it was tried and rejected, there may be a constraint I am missing.
3. Whether the Swap direction (A->B vs B->A) can always be determined from the old pool state + new pool state (it can: compare reserve_a before/after), but whether there are edge cases where reserves change in both directions simultaneously (a multi-pool batch?) that would break this.
4. The exact semantics of the protocol fee extraction (25/5 bps split) and whether it creates a DOLI flow that would break a strict recompute-and-verify model. If the protocol fee is extracted as a separate output (to the reward pool), it must be accounted for in the recomputation.

## Current State Analysis

**Checked inventory: 15 AMM-related checks across 4 enforcement sites.**

| Site | File | Check | Lines | Status |
|------|------|-------|-------|--------|
| Mempool | mempool/pool.rs:381-437 | Pool-aware DOLI conservation | 57 | DUPLICATE of consensus (utxo.rs:210-262) with different semantics (D2) |
| Mempool | mempool/pool.rs:927-992 | calculate_inputs (is_native filter) | 66 | DUPLICATE reimplementation of input summing logic |
| Mempool | mempool/pool.rs:275-377 | Signature + RC-A exemption | 103 | DUPLICATE of consensus (utxo.rs:150-182) |
| Mempool | mempool/pool.rs:445-449 | Fee check exemption | 5 | DUPLICATE of consensus (utxo.rs:268-290) |
| Consensus/structural | validation/pool.rs:11-143 | CreatePool structural | 133 | NECESSARY — shape validation, pool_id derivation, MINIMUM_LIQUIDITY |
| Consensus/structural | validation/pool.rs:146-173 | Swap structural | 28 | THIN — only checks input/output counts + Pool type. Earns its existence |
| Consensus/structural | validation/pool.rs:177-205 | AddLiquidity structural | 29 | THIN — same. Earns its existence |
| Consensus/structural | validation/pool.rs:208-230 | RemoveLiquidity structural | 23 | THIN — same. Earns its existence |
| Consensus/UTXO | utxo.rs:595-702 | Swap k-invariant + token conservation | 108 | PARTIALLY BROKEN (D4: B->A token input unbound) |
| Consensus/UTXO | utxo.rs:704-744 | AddLiquidity context | 41 | STRUCTURALLY WEAK — no input binding at all |
| Consensus/UTXO | utxo.rs:746-854 | RemoveLiquidity context + proportional binding (096 gated) | 109 | PARTIALLY FIXED by INC-I-096 patch |
| Consensus/UTXO | utxo.rs:210-262 | Pool-aware DOLI conservation (096 gated) | 53 | NECESSARY concept but trusts declared new_reserve_a |
| Consensus/UTXO | utxo.rs:857-918 | CreatePool RC-B input backing (092 gated) | 62 | CORRECT and NECESSARY |
| Consensus/UTXO | utxo.rs:164-170 | RC-A: Pool input signature exemption | 7 | NECESSARY — preserve |
| Apply_block | tx_processing.rs:134-143 | Duplicate pool_id guard | 10 | NECESSARY — state-level uniqueness |

**Dead code identified:**
- `pool.rs:verify_invariant` — exported from crate lib.rs (line 267), called ONLY in pool.rs own tests. Consensus hand-rolls the same check at utxo.rs:648-654. Dead export, never called by any production code outside pool.rs tests.

**Total AMM-related validation code: ~691 lines across 3 files** (excluding tests).
- Mempool reimplementation: ~231 lines (REDUNDANT with consensus)
- Consensus structural (pool.rs): ~213 lines (NECESSARY, lightweight)
- Consensus UTXO-context (utxo.rs): ~380 lines in AMM blocks
- Apply_block: ~10 lines (NECESSARY)

## Proposals

### P1: Eliminate mempool conservation reimplementation entirely — conf(0.6, observed)

**Concept:** Remove the mempool's reimplemented DOLI conservation (pool-aware check, calculate_inputs filter, signature verification, fee check) for AMM tx types. Have the mempool call `validate_transaction_with_utxos` via a `MempoolUtxoProvider` that resolves unconfirmed parents.

**Evidence:**
- Mempool pool.rs:275-449 + pool.rs:927-992 = ~231 lines of reimplemented validation.
- Block assembly already calls `validate_transaction_with_utxos` (assembly.rs:235) before including ANY mempool tx.
- The D2 defect (mempool/consensus parity) exists BECAUSE the mempool reimplements conservation differently.
- UtxoSet already implements UtxoProvider (storage/utxo/set.rs:404).

**Complexity cost:** -231 lines (mempool reimplementation). +~30 lines (MempoolUtxoProvider wrapper struct + impl). Net: ~-200 lines. Eliminates D2 permanently by construction (single code path).

**Kill test:** Does the mempool NEED to know the fee amount for non-AMM txs? Yes — for eviction priority. But for AMM txs, fee is 0 (exempt). So the fee computation is only needed for non-AMM, where the current non-pool-aware path works fine.

**Kill test result:** Partial concern found. The mempool uses `calculate_inputs` for ALL txs, not just AMM. Changing it to use `validate_transaction_with_utxos` for AMM txs while keeping the old path for non-AMM creates a branching point. A cleaner approach: implement `MempoolUtxoProvider` and call `validate_transaction_with_utxos` for ALL txs (replacing the reimplemented signature/spending checks too), then compute fee separately from the total_input/total_output it already has. This is a larger refactor (~400 lines touched) but eliminates the entire class of mempool/consensus divergence.

**Risk:** `validate_transaction_with_utxos` might reject some mempool-chain scenarios that the current code handles (e.g., spending outputs from unconfirmed parents). The `MempoolUtxoProvider` must correctly resolve these. Testing burden is moderate.

**Before/After:**
- Before: Mempool reimplements ~231 lines of validation logic with different semantics (D2). Conservation check and consensus check can disagree.
- After: Single validation code path. Mempool wraps UtxoSet as MempoolUtxoProvider, delegates to consensus validator. D2 structurally impossible.

---

### P2: Delete all ad-hoc UTXO-context AMM checks; replace with recompute-and-verify — conf(0.55, inferred)

**Concept:** Instead of checking ad-hoc structural properties of declared pool state (reserves increased/decreased, LP changed, k-invariant holds), the validator RECOMPUTES the expected new pool state from (old pool state + actual inputs), using the same `pool.rs` math the builder uses. Then verifies: `declared_new_state == computed_new_state` (with dust tolerance for floor division).

**Evidence:**
- utxo.rs:595-854 = ~260 lines of ad-hoc checks that are partially broken (D3, D4) and miss input binding (AddLiquidity).
- pool.rs already has all the correct math: `compute_swap`, `compute_remove_liquidity`, `compute_lp_shares`, `compute_initial_lp_shares`.
- The builder (CLI cmd_pool.rs) calls these functions and they produce correct results.
- Consensus NEVER calls these functions — it hand-rolls weaker checks.
- `verify_invariant` is exported but dead (only called in pool.rs tests).

**What gets deleted:**
- utxo.rs:595-702 (Swap context): 108 lines
- utxo.rs:704-744 (AddLiquidity context): 41 lines
- utxo.rs:746-854 (RemoveLiquidity context): 109 lines
- Total: ~258 lines deleted

**What gets added:**
A single function `validate_amm_pool_transition(tx, old_pool_utxo, utxo_provider, ctx) -> Result<()>` that:
1. Reads old pool state from input[0]
2. For each tx type, determines what the user provided (sum DOLI inputs, sum FungibleAsset inputs by asset_id, sum LPShare inputs by pool_id)
3. Calls the appropriate pool.rs math function
4. Compares declared new pool state to computed, with `<=` tolerance for floor-division dust
5. Returns Ok or error

Estimated: ~80-120 lines. Net deletion: ~140-180 lines.

**Complexity cost:** -258 lines (ad-hoc checks) + ~100 lines (recompute function) = net -158 lines. More importantly: eliminates the "declared vs actual" trust boundary that is the root cause of D3, D4, and the unbound AddLiquidity.

**Kill test:** Does the recompute-and-verify model handle protocol fee extraction? If the protocol fee (5 bps) is extracted from reserves before LP share computation, the recompute must use the same extraction formula. If it is extracted differently at apply_block vs build time, the verify would fail.

**Kill test result:** Need to verify. The fee split (25/5) is part of the swap computation. `compute_swap` in pool.rs applies the full `fee_bps` deduction. The 25/5 split may happen at the builder level (CLI distributes the fee portion), not in the pool math. If so, the pool state transition is: `compute_swap(old_reserve_a, old_reserve_b, dx, fee_bps)` gives `(dy, new_reserve_a, new_reserve_b)` which the validator can verify. The protocol fee portion would be an additional output, not a reserve change. This needs verification but is likely sound.

**Risk:** The swap direction determination from inputs could be ambiguous if the user provides both DOLI and FungibleAsset inputs (e.g., A->B swap with token change). The recompute function would need robust direction detection. Medium risk.

**Before/After:**
- Before: ~258 lines of ad-hoc structural checks that trust declared state, have 3 binding gaps (D3, D4, AddLiquidity), and hand-roll the k-invariant.
- After: ~100 lines of recompute-and-verify that uses the same math as the builder, closing all binding gaps by construction. `verify_invariant` export deleted (dead code).

---

### P3: Remove mempool's pool-aware conservation (smallest viable subtraction) — conf(0.65, observed)

**Concept:** The SMALLEST deletion that addresses the most defects. Remove ONLY the mempool's INC-I-096 pool-aware conservation logic (lines 384-437) and the is_native_amount gating in calculate_inputs (lines 942-982). Instead: for AMM txs, skip the mempool's native conservation check entirely (let consensus be authoritative). Set fee=0 for AMM txs in mempool (they are fee-exempt anyway).

**Evidence:**
- Block assembly (assembly.rs:235) runs `validate_transaction_with_utxos` before including any mempool tx.
- AMM txs are fee-exempt (utxo.rs:268-276, mempool:pool.rs:445).
- The mempool conservation check has DIFFERENT semantics than consensus (D2).
- Removing it eliminates D2 by removing one side of the parity equation.
- D1 (false rejection) goes away because mempool no longer checks conservation for AMM txs.

**What gets deleted:**
- mempool/pool.rs:384-437 (pool-aware conservation): ~54 lines
- mempool/pool.rs:942-952 (is_native_amount gate in mempool chain resolution): ~11 lines  
- mempool/pool.rs:975-982 (is_native_amount gate in UTXO resolution): ~8 lines
- Total: ~73 lines deleted, 0 lines added.

**Complexity cost:** Pure deletion. -73 lines. 0 new abstractions.

**Kill test:** Does removing mempool conservation create a DoS vector? Attacker submits AMM txs with inflated outputs that pass mempool but fail consensus. They consume mempool space.

**Kill test result:** Bounded. Mempool has max_count and max_size limits. AMM txs have pool contention (only one pool tx per pool per block), limiting throughput. The attacker wastes mempool slots but cannot fill blocks. Additionally, the structural validation (validate_transaction, which IS called by mempool) catches malformed txs early. Only semantically invalid (wrong reserves/LP) txs would slip through — a narrow attack surface with bounded impact.

**Risk:** Low. Slightly more invalid AMM txs in mempool, bounded by pool contention and mempool limits.

**Before/After:**
- Before: Mempool has its own pool-aware conservation with parity bugs (D1, D2). 73 lines of fragile logic.
- After: Mempool skips conservation for AMM txs. Consensus is the single authority. D1 and D2 eliminated by removal.

**NOTE:** This does NOT fix D3, D4, or H2. It is the smallest subtraction. Combine with P2 or P4 for full coverage.

---

### P4: Bind token_b via input-sum-matches-reserve-delta (pure addition, no removal) — conf(0.6, inferred)

**Concept:** Add FungibleAsset input binding to Swap B->A and AddLiquidity, closing D4 and H2. This is NOT subtraction, but it is the smallest addition that closes the remaining security gaps once P3 is applied.

**Evidence:**
- D4: Swap B->A has no binding of `new_reserve_b` increase to actual FungibleAsset inputs (utxo.rs:677-694).
- H2: No token_b conservation anywhere.
- A->B Swap already has token OUTPUT binding (utxo.rs:660-676).
- CreatePool already has token INPUT binding via RC-B (utxo.rs:869-918).
- The pattern exists; it just was not applied to B->A Swap and AddLiquidity.

**What gets added:**
For B->A Swap (inside the `new_meta.reserve_b > old_meta.reserve_b` branch at utxo.rs:677):
```
// Sum FungibleAsset inputs for the specific asset_b
let token_in: u64 = sum of FungibleAsset inputs matching pool's asset_b_id
let expected_increase = new_meta.reserve_b - old_meta.reserve_b
if token_in < expected_increase { reject }
```
~15 lines.

For AddLiquidity (inside utxo.rs:704-744):
```
// Bind DOLI input increase and token input increase to reserve deltas
let doli_added = new_m.reserve_a - old_m.reserve_a
let token_added = new_m.reserve_b - old_m.reserve_b
// Verify DOLI inputs cover doli_added (already handled by pool-aware conservation)
// Verify FungibleAsset inputs cover token_added
let token_in: u64 = sum of FungibleAsset inputs matching asset_b_id
if token_in < token_added { reject }
```
~20 lines.

**Complexity cost:** +35 lines. No new abstractions. Follows existing patterns (RC-B, A->B token conservation).

**Kill test:** Does the FungibleAsset input sum correctly exclude token change outputs? For AddLiquidity, the user may provide more tokens than needed, getting change back as a FungibleAsset output. The binding should use `token_in - token_change >= reserve_delta`, not just `token_in >= reserve_delta`.

**Kill test result:** Valid concern. The existing RC-B (CreatePool) handles this correctly (utxo.rs:900-910: subtracts token_change). The same pattern must be replicated. This is straightforward.

**Risk:** Low. Same pattern as RC-B, proven correct.

**Before/After:**
- Before: B->A Swap can declare arbitrary reserve_b increase without providing tokens (D4/SEC-LOGIC-002). AddLiquidity has no input binding. Token_b has no conservation (H2).
- After: All reserve_b increases must be backed by actual FungibleAsset inputs. H2 closed for all AMM tx types.

---

### P5: Delete verify_invariant export (dead code cleanup) — conf(0.7, measured)

**Concept:** Remove the `verify_invariant` function from the public API export (lib.rs:267). It is only called in pool.rs's own tests. Consensus hand-rolls the k-invariant check at utxo.rs:648-654.

**Evidence:**
- `verify_invariant` defined at pool.rs:112. Exported at lib.rs:267.
- Grep across entire codebase: called ONLY in pool.rs::tests (6 call sites) and referenced in specs/docs (not code).
- Consensus at utxo.rs:648-654 reimplements: `let old_k = ...; let new_k = ...; if new_k < old_k { reject }`.
- No CLI, no node, no test outside pool.rs calls it.

**Complexity cost:** -1 export. Optionally: refactor utxo.rs:648-654 to call `pool::verify_invariant` instead of hand-rolling (5 lines changed, not added).

**Kill test:** Is there any reason to keep the export for third-party or future use?

**Kill test result:** AMM is not yet activated. No third parties consume this API. If P2 (recompute-and-verify) is adopted, the consensus code would call `compute_swap` instead, making the hand-rolled check AND `verify_invariant` both redundant for Swap. For now, either promote it (have consensus call it) or remove it. Promoting is lower risk.

**Risk:** None. Pure dead-code cleanup.

**Before/After:**
- Before: Dead export in lib.rs. Consensus reimplements the same 5-line check.
- After: Either consensus calls the pool.rs function (promoting shared code) or export is removed and function becomes `pub(crate)` (cleaning dead export).

## Constraints Identified

1. **C10 is immutable:** Pool UTXO must be consumed and recreated per operation. The declared `new_reserve_a/b` in the output extra_data cannot be removed — downstream consumers (other nodes, indexers) need the pool state without replaying. The subtraction target is the TRUST in declared values, not the values themselves.

2. **Floor-division dust (C8/H1):** Any binding that replaces ad-hoc checks with recompute-and-verify MUST use `<=` tolerance (declared_delta <= computed_max_delta), not `==`. The builder's floor-division truncation means the declared state will sometimes be 1 unit less than the exact proportional calculation. Using `==` would reject ~50% of legitimate removes.

3. **Mempool needs fee for non-AMM txs:** The mempool cannot fully delegate to `validate_transaction_with_utxos` without losing the fee computation. The fee is needed for eviction priority on non-AMM txs. AMM txs are fee-exempt, so their fee is irrelevant.

4. **Activation height is mandatory (C6/C7):** Any change to consensus-visible behavior requires `inc_i_096_activation_height`. The existing gating infrastructure is already in place. All proposals must be gated.

5. **RC-A must be preserved (C12):** The Pool input signature exemption (utxo.rs:164-170) is load-bearing. Without it, all pool operations are permanently unspendable. Any refactoring must preserve this.

6. **RC-B must be preserved (C8 brief):** CreatePool input backing (utxo.rs:857-918) is correct and addresses INC-I-092. Must not be deleted or weakened.

7. **Structural validators (pool.rs) serve defense-in-depth:** Even under recompute-and-verify, the cheap structural checks (input count >= 2, output[0] = Pool, etc.) should be preserved. They reject malformed txs before expensive UTXO lookups.

## Cross-Perspective Signals

1. **For the Restructurer:** The mempool's validation is a ~231-line reimplementation of consensus validation. A `MempoolUtxoProvider` adapter would allow the mempool to call `validate_transaction_with_utxos` directly, eliminating the entire class of mempool/consensus parity bugs. This is an architectural pattern change, not just AMM-scoped.

2. **For the Patterns evaluator:** The "builder computes, validator trusts" anti-pattern is not unique to AMM. Check whether other tx types (NFT royalties, BridgeHTLC) have the same shape where the validator trusts declared output fields instead of recomputing from inputs.

3. **For the Failures evaluator:** The protocol fee split (25/5 bps) creates a value flow that any recompute-and-verify model must account for. If the protocol fee is extracted as a separate DOLI output to the reward pool, the pool-aware conservation equation changes. Verify the fee flow before approving P2.

4. **For the Radical evaluator:** The most aggressive subtraction — making declared pool state fields PURE METADATA that the validator ignores and recomputes entirely — would eliminate the trust boundary permanently. But it requires all consumers of Pool UTXOs to recompute state from the transaction chain, which may be too expensive for indexers/explorers.

## Gaps

1. **Protocol fee mechanics not verified.** I did not trace the exact 25/5 bps fee split implementation in the builder to verify whether it affects pool state or is a separate output. This is critical for P2 (recompute-and-verify).

2. **MempoolUtxoProvider feasibility not fully verified.** The mempool resolves unconfirmed parents and checks maturity — I verified the pattern but did not build a proof-of-concept. There may be edge cases with spending in-mempool outputs of AMM txs.

3. **AddLiquidity LP share proportional binding not analyzed in depth.** The brief lists VC-005 (AddLiquidity input binding) but the existing code has NO binding at all for AddLiquidity — only "reserves increased, LP increased." A subtraction lens cannot propose what to add; I noted the gap in P4.

4. **The existing INC-I-096 patch in the working tree** was read for problem understanding only (per brief direction). I did not compare it against these proposals to check for conflicts.
