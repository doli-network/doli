# Design Evaluation: Restructurer (Coupling, Dependency Direction, Data Flow)

## Analysis Lens
Coupling patterns, dependency direction, and data flow in the AMM value-conservation layer. The central question: **Where are module BOUNDARIES wrong, and where should they move?**

## What I Don't Understand
1. Whether the CLI (`cmd_pool.rs`) is the only builder of AMM transactions or if there's also a programmatic builder used in tests or other paths that I haven't found.
2. Whether the existing `compute_remove_liquidity` floor-division behavior is byte-identical to the inline math used in the INC-I-096 patch's proportional binding (both use `shares * reserve / total` with u128 intermediate), or whether there's a subtle rounding difference.
3. Why the `total_output()` method on Transaction already filters via `is_native_amount()` (matching consensus), but the mempool's `calculate_inputs` did NOT filter until the INC-I-096 patch. The asymmetry between output-side filtering and input-side filtering suggests the original design intended non-native amounts to participate in fee calculation -- the D2 parity divergence may be partially intentional legacy behavior.
4. Whether the block-assembly path (`build_block_content`) performs any independent AMM conservation check or relies entirely on the mempool admission + consensus validation pipeline. (Resolved: confirmed it does NOT -- it delegates to validate_transaction_with_utxos.)

## Current State Analysis

### Dependency Graph (Before)

```
                    +--------------------+
                    |   bins/node        |
                    | (apply_block/      |
                    |  tx_processing.rs  |
                    |  production/       |
                    |  assembly.rs)      |
                    +---------+----------+
                              |
            depends on        |       depends on
         +--------------------+--------------------+
         |                                         |
         v                                         v
  +------+--------+                        +-------+------+
  |   mempool     |                        |   crates/    |
  | (pool.rs)     |---depends on---------->|   core       |
  +------+--------+                        |              |
         |                                 | pool.rs      |  <-- AMM math (7 fns)
         |                                 | validation/  |
         |                                 |   utxo.rs    |  <-- conservation + binding
         |                                 |   pool.rs    |  <-- structural validation
         |                                 |   types.rs   |  <-- ValidationContext
         |                                 | transaction/ |
         |                                 |   output.rs  |  <-- PoolMetadata, is_native_amount
         |                                 +-------+------+
         |                                         |
         |                  depends on             |
         +--------------------+--------------------+
                              |
                              v
                      +-------+------+
                      |   storage    |
                      | (UtxoSet     |
                      |  impl        |
                      |  UtxoProvider)|
                      +--------------+
```

**Verified:** `storage::UtxoSet` implements `core::validation::UtxoProvider` (`crates/storage/src/utxo/set.rs:404`). This means any shared AMM verification function in `crates/core` that takes `&U: UtxoProvider` can be called from both consensus (via UtxoSet directly) and mempool (via UtxoSet which mempool already depends on).

### Measured Metrics

| Module | File | LOC | AMM-specific LOC | AMM math refs |
|--------|------|-----|-------------------|---------------|
| Builder math | `crates/core/src/pool.rs` | 488 (135 code, 353 test) | 135 | **DEFINES** 7 functions |
| Consensus validation (UTXO) | `crates/core/src/validation/utxo.rs` | 1177 | ~250 (lines 588-918) | **ZERO** calls to pool.rs |
| Consensus validation (structural) | `crates/core/src/validation/pool.rs` | 420 | 230 | **ZERO** calls to pool.rs |
| Mempool admission | `crates/mempool/src/pool.rs` | 1505 | ~120 (lines 380-456, 926-992) | **ZERO** calls to pool.rs |
| apply_block tx_processing | `bins/node/src/node/apply_block/tx_processing.rs` | 592 | ~10 (delegates to validate_transaction_with_utxos) | **ZERO** calls to pool.rs |
| CLI builder | `bins/cli/src/cmd_pool.rs` | ~1200+ | ~1200 | **CALLS** 4 of 7 functions |
| RPC quote | `crates/rpc/src/methods/pool.rs` | ? | ? | **CALLS** 1 function |

**Key structural finding: The 7 AMM math functions in `pool.rs` are called by CLI and RPC only. ZERO calls from any validation or enforcement site. The consensus validator re-implements ad-hoc checks inline.**

### Callers of pool math (evidence):
- `compute_swap`: CLI (cmd_pool.rs:603), RPC (pool.rs:191), self-tests
- `compute_initial_lp_shares`: CLI (cmd_pool.rs:317), self-tests
- `compute_lp_shares`: CLI (cmd_pool.rs:943), self-tests
- `compute_remove_liquidity`: CLI (cmd_pool.rs:1212), self-tests
- `verify_invariant`: self-tests ONLY (never called outside pool.rs)
- `compute_twap_price`: self-tests ONLY
- `update_twap`: self-tests ONLY

### Conservation Logic Locations (D1/D2 evidence)

**Native DOLI conservation** -- duplicated in 2 sites with divergent semantics:
1. **Consensus** (`utxo.rs:184-262`): `total_input` sums only `is_native_amount()` UTXOs. `total_output` uses `tx.total_output()` which also filters via `is_native_amount()`. Post-INC-I-096 gate: adds pool-aware `lhs = native_in + old_reserve_a >= native_out + new_reserve_a`.
2. **Mempool** (`pool.rs:380-437`): `total_input` via `calculate_inputs()` -- pre-INC-I-096 sums ALL `.amount` unconditionally (D2 parity bug); post-INC-I-096 filters via `is_native_amount()`. `total_output` uses `tx.total_output()` (always filtered). Post-INC-I-096 gate: pool-aware check with identical math.

**Token conservation (token_b)** -- checked in ONE location only:
- Consensus `utxo.rs:660-694`: Swap A->B direction binds `tokens_out_from_pool == tokens_to_user` (exact).
- RemoveLiquidity: `utxo.rs:838-849`: `tokens_out <= actual_token_delta` (post-INC-I-096 gate only).
- **Mempool: ZERO token_b checks.** Mempool does not verify FungibleAsset conservation at all.
- **CreatePool** (`utxo.rs:869-918`): INC-I-092 RC-B checks `net_token_in >= reserve_b`.
- **AddLiquidity: ZERO token_b binding** in either site.
- **Swap B->A: ZERO token_b input binding** (the check only verifies the A->B direction).

### The Three Structural Defects

**SD-1: Compute/Verify Split.** The canonical AMM math (`pool.rs`) and the validation checks (`validation/utxo.rs`) are completely decoupled. The validator NEVER calls the builder's math. Instead it performs ad-hoc comparisons (old_reserve vs new_reserve, new_k >= old_k) that are structurally weaker than the builder's math. For example, `verify_invariant` exists in pool.rs but validation re-implements the k-check inline (utxo.rs:648-655). `compute_remove_liquidity` exists but validation implements its own proportional binding inline (utxo.rs:800-803).

**SD-2: Duplicate Conservation Boundaries.** The conservation check is implemented twice (mempool + consensus) with historically divergent semantics (D2). The INC-I-096 patch fixes the parity by making mempool mirror consensus, but the fix is copy-paste: the same 15-line pool-aware check appears in both files. No shared function.

**SD-3: Asset-Type-Blind Accounting.** The conservation framework only tracks native DOLI. Token_b (FungibleAsset) has no conservation framework at all -- individual per-tx-type checks must manually sum FungibleAsset inputs/outputs. This is why D4 (Swap B->A unbound token inputs) and H2 (token_b drainable) exist.


## Proposals

### P1: Extract `AmmTransitionVerifier` to `crates/core/src/validation/amm.rs` -- conf(0.65, observed)

Move the canonical AMM state-transition verification into a single module within `crates/core/src/validation/` that CALLS `pool.rs` math functions. Both the consensus validator (`utxo.rs`) and the mempool (`pool.rs`) call this module instead of implementing their own checks.

**Evidence:**
- `validation/utxo.rs` has ~250 lines of AMM-specific checks (Swap lines 595-702, AddLiquidity 705-744, RemoveLiquidity 747-854, CreatePool 859-918) that re-implement what `pool.rs` already computes.
- The mempool duplicates ~120 lines of conservation logic (lines 380-456, 926-992).
- Dependency direction is clean: `validation/amm.rs` is within `crates/core`, so mempool can call it (mempool already depends on `doli-core`). No circular dependency.
- `storage::UtxoSet` implements `core::validation::UtxoProvider` (verified at `crates/storage/src/utxo/set.rs:404`), so the shared module can use the UtxoProvider trait and both sites can pass their UTXO source through it.

**Interface (pre-extracted values -- avoids mempool's chain-lookup complexity):**
```
// crates/core/src/validation/amm.rs
pub fn verify_amm_transition(
    tx_type: TxType,
    old_pool: &PoolMetadata,
    new_pool: &PoolMetadata,
    native_inputs: u64,
    native_outputs: u64,
    token_b_inputs: u64,    // sum of FungibleAsset inputs matching asset_b
    token_b_outputs: u64,   // sum of FungibleAsset outputs matching asset_b
    lp_shares_burned: u64,  // sum of consumed LPShare amounts
    lp_shares_minted: u64,  // sum of created LPShare amounts
    height: u64,
    activation_height: u64,
) -> Result<(), ValidationError>
```

**Design choice:** Takes pre-extracted values instead of UtxoProvider because the mempool has a different UTXO lookup strategy (chains mempool entries -> UtxoSet). The caller extracts the values using their own lookup, then the shared function verifies conservation/binding on the pre-extracted totals. This keeps the interface independent of the lookup mechanism.

**Before/After dependency:**
```
BEFORE:
  mempool::pool.rs --(reimplements)--> conservation logic
  validation::utxo.rs --(reimplements)--> conservation logic + binding
  core::pool.rs ---------(unused by validation/mempool)----------->

AFTER:
  mempool::pool.rs ---calls--> core::validation::amm.rs ---calls--> core::pool.rs
  validation::utxo.rs ---calls--> core::validation::amm.rs ---calls--> core::pool.rs
```

**Complexity cost:** +1 module (`validation/amm.rs`, ~200 LOC). -250 LOC from `utxo.rs`, -120 LOC from mempool `pool.rs`. Net reduction ~170 LOC. Number of places conservation logic exists: 2 -> 1.

**Kill test:** Would this create a circular dependency? Answer: No. `validation/amm.rs` is in `crates/core`. Mempool depends on `doli-core`. Direction is clean. Would this break the activation-height gating? The function takes `height` and `activation_height` parameters, so it can gate internally. Kill test PASSED.

**Risk:** The shared function must handle ALL 4 AMM tx types, which have different conservation shapes (CreatePool is asymmetric; Swap has two directions; AddLiquidity increases everything; RemoveLiquidity decreases). A single function handling all cases may become complex. Mitigation: dispatch internally per tx_type but present a single verification interface.

### P2: Introduce per-asset `ValueFlowSummary` for token_b conservation -- conf(0.55, inferred)

Create a small struct that accumulates per-asset value flows across a transaction, producing a balance sheet that the AMM verifier (P1) queries. This addresses SD-3 (asset-type-blind accounting) and VC-009 (token_b conservation).

**Evidence:**
- Token_b conservation is currently checked ad-hoc in 3 separate blocks: Swap A->B (utxo.rs:663-675), CreatePool RC-B (utxo.rs:888-917), RemoveLiquidity (utxo.rs:838-849). AddLiquidity has ZERO token_b binding. Swap B->A has ZERO token_b input binding.
- The mempool has ZERO token_b conservation checks.
- Each check manually iterates `tx.outputs.iter().skip(1).filter(|o| o.output_type == OutputType::FungibleAsset)` -- this pattern appears 3 times.

**Interface:**
```
// Inside crates/core/src/validation/amm.rs
pub struct AmmFlowSummary {
    pub native_in: u64,
    pub native_out: u64,
    pub token_b_in: u64,       // FungibleAsset inputs matching pool's asset_b
    pub token_b_out: u64,      // FungibleAsset outputs matching pool's asset_b
    pub lp_burned: u64,        // LPShare inputs matching pool's pool_id
    pub lp_minted: u64,        // LPShare outputs matching pool's pool_id
    pub old_reserve_a: u64,
    pub new_reserve_a: u64,
    pub old_reserve_b: u64,
    pub new_reserve_b: u64,
    pub old_total_lp: u64,
    pub new_total_lp: u64,
}

impl AmmFlowSummary {
    pub fn native_conserved(&self) -> bool;
    pub fn token_b_conserved(&self) -> bool;
    pub fn lp_consistent(&self) -> bool;
}
```

**Complexity cost:** +1 struct (~80 LOC). Replaces 3 separate token-iteration blocks (~45 LOC each = 135 LOC). Net: ~-55 LOC + cleaner token_b coverage. No HashMap needed -- single (asset_b) tuple per pool.

**Kill test:** Does this require changing `is_native_amount()`? No -- the struct is populated by callers who classify UTXOs by output_type. It does NOT change the system-wide `is_native_amount()` (respecting C10/VC-014). Kill test PASSED.

**Risk:** Over-abstraction. Mitigation: keep it AMM-scoped (not a generic ledger for all 27 tx types). The struct is a data bag, not a framework.

### P3: Shared input-classification function for mempool/consensus parity -- conf(0.60, observed)

Extract the input-summation logic that both sites need (classify by output_type, sum native vs token_b vs LP shares, extract old_pool metadata) into a shared function.

**Evidence:**
- Mempool `calculate_inputs` (lines 927-991) sums amounts with type-conditional filtering.
- Consensus `validate_transaction_with_utxos` (lines 90-191) sums amounts with `is_native_amount()` filtering.
- Both sites separately extract `old_reserve_a` from the first input's pool metadata (mempool lines 402-409, consensus lines 233-239) -- identical logic.

**Interface:**
```
// crates/core/src/validation/amm.rs
pub fn classify_amm_inputs<U: UtxoProvider>(
    tx: &Transaction,
    utxo_provider: &U,
    pool_asset_b: &Hash,
    pool_id: &Hash,
) -> Result<AmmFlowSummary, ValidationError>;
```

**Problem:** The mempool also checks mempool-internal UTXOs (unconfirmed parent outputs). This function would only work for confirmed UTXOs via UtxoProvider. The mempool would still need its own chain-lookup but could delegate the classification step. **Resolution:** Split into two layers: (1) a UTXO-lookup-independent `AmmFlowSummary::add_input(output_type, amount, asset_id)` accumulator that both sites call in their own loops, and (2) the shared conservation checks on the accumulated summary.

**Complexity cost:** +1 accumulator method (~30 LOC). Replaces per-site classification logic (~40 LOC x 2 = 80 LOC). Net: -50 LOC.

**Kill test:** Does the mempool's chain-lookup (mempool entries -> UtxoSet) make this infeasible? No -- the accumulator approach (method on AmmFlowSummary) is lookup-agnostic. Each site iterates its own way but classifies through a shared interface. Kill test PASSED.

**Risk:** Minimal. The accumulator is a thin helper, not a deep abstraction.

### P4: Validator calls `pool.rs` math for re-verification -- conf(0.60, observed)

After the AMM verifier (P1) checks conservation and structural invariants, it also calls `pool.rs` functions to verify that the declared new-pool-state is reachable from the old-pool-state given the actual inputs. This kills D3 and D4 by construction.

**Evidence:**
- `verify_invariant` is defined in pool.rs but NEVER called by validation (grep: zero hits outside pool.rs tests).
- `compute_swap` is NEVER called by validation. The validator only checks `new_k >= old_k` and token conservation, but does not verify that the specific (dx, dy, new_reserves) triple is consistent with the swap formula.
- `compute_remove_liquidity` is NEVER called by validation. The validator only checks proportional bounds `<= max_delta` but does not verify the exact (or dust-tolerated) output.

**Approach:** Inside `verify_amm_transition` (P1), for each AMM tx type:
1. **Swap:** Call `compute_swap(old_ra, old_rb, dx, fee_bps)` where dx is derived from the reserve_a delta. Verify declared new reserves match (with dust tolerance for fee rounding). Also call `verify_invariant`.
2. **RemoveLiquidity:** Call `compute_remove_liquidity(shares_burned, old_ra, old_rb, old_total)`. Verify `reserve_a_delta <= computed_da` and `reserve_b_delta <= computed_db` (dust tolerance per C8).
3. **AddLiquidity:** Call `compute_lp_shares(da, db, old_ra, old_rb, old_total)`. Verify declared new LP shares `<=` computed + dust.
4. **CreatePool:** Call `compute_initial_lp_shares(ra, rb)`. Verify declared total matches `result + MINIMUM_LIQUIDITY`.

**Complexity cost:** +~60 LOC in the AMM verifier. Replaces ad-hoc checks that are individually correct but structurally incomplete.

**Kill test:** Would calling `compute_swap` in validation reject legitimate transactions due to rounding differences? The math is ALL integer (u64/u128), and the CLI builder and the pool.rs functions use identical arithmetic. The validator would use the SAME function the builder used. The only risk is if dx extraction is incorrect. dx for A->B swap = `new_reserve_a - old_reserve_a` (the DOLI that went in). For B->A swap, dx = `new_reserve_b - old_reserve_b` (the tokens that went in). Both are deterministic from the declared pool states. **Kill test result: PASSED -- dx/dy extraction is deterministic from pool metadata deltas.**

**Risk:** If a future builder implementation uses slightly different arithmetic than pool.rs (e.g., a third-party SDK), the validator would reject its transactions. Mitigation: pool.rs IS the canonical math -- any builder must match it. This is by design.

### P5: Activation-height consolidation -- conf(0.50, inferred) -- KILLED by C6

**Killed.** The brief's C6 mandates a NEW `inc_i_096_activation_height`. Cannot reuse `amm_activation_height`. Deployment policy: pin to the same value as `amm_activation_height` (mainnet = u64::MAX). No structural change proposed.

## Constraints Identified

1. **C-STRUCT-1: core must not depend on mempool/node.** Any shared AMM verification module must live in `crates/core`. The mempool and node consume it, not vice versa.

2. **C-STRUCT-2: `UtxoProvider` trait is the abstraction boundary.** Verified: `storage::UtxoSet` implements `core::validation::UtxoProvider` (`crates/storage/src/utxo/set.rs:404`). The shared module can take `&U: UtxoProvider` or pre-extracted values. Pre-extracted values are preferred for the mempool case (which chains unconfirmed parent lookups before UtxoSet).

3. **C-STRUCT-3: Activation-height threading is load-bearing.** Every new field in `ValidationContext` must be threaded through ALL construction sites (mempool `add_transaction`, mempool `add_system_transaction`, apply_block `process_transaction_utxos`, `validation_checks.rs`). Currently there are 4+ construction sites. Missing any one causes mempool/consensus parity divergence. The `inc_i_096_activation_height` is already threaded (verified in working tree).

4. **C-STRUCT-4: `tx.total_output()` already filters `is_native_amount()`.** This is a stable, tested function. The conservation equation's output side is correct. The defect was only on the input side (mempool) and the reserve-accounting side.

5. **C-STRUCT-5: Pool metadata is the single interface for reserve state.** Both old-pool (from UTXO) and new-pool (from tx.outputs[0]) are accessed via `pool_metadata()`. This accessor returns `Option<PoolMetadata>`, a struct with `reserve_a`, `reserve_b`, `total_lp_shares`, etc. Any AMM verifier gets its state from this accessor.

6. **C-STRUCT-6: Input ordering convention is consensus-visible.** input[0] = Pool UTXO is enforced by structural validation (`validation/pool.rs`). output[0] = new Pool UTXO is also enforced. The AMM verifier can rely on this convention.

7. **C-STRUCT-7: Floor-division dust tolerance must use `<=` not `==`.** The proportional binding (`shares * reserve / total`) truncates toward zero. Both builder and verifier use the same integer arithmetic, but the verifier must use `<=` to tolerate the 0-1 unit dust. Already implemented correctly in the INC-I-096 patch's proportional binding (utxo.rs:803,817).

8. **C-STRUCT-8: Mempool cannot call UtxoProvider directly for unconfirmed parents.** The mempool chains its own entries before the UtxoSet. A shared classification function must be lookup-agnostic (accumulator pattern) rather than UtxoProvider-dependent.

## Cross-Perspective Signals

1. **For the Subtractionist:** The INC-I-096 patch adds ~130 lines to `utxo.rs` and ~80 lines to mempool `pool.rs`. If P1 (shared AMM verifier) is adopted, approximately 210 of these lines can be replaced by ~60 lines of delegation code + a ~200-line shared module. Net: roughly equivalent LOC but with a single-authority guarantee instead of copy-paste parity.

2. **For the Pattern Analyst:** The pattern of "builder computes, validator trusts" is not unique to AMM. The EpochReward validation has a similar gap (mentioned in CLAUDE.md: "calculate_epoch_rewards() in rewards.rs AND calculate_expected_epoch_rewards() in validation.rs (currently disconnected)"). This is a recurring architectural pattern in this codebase.

3. **For the Failure-Mode Analyst:** The D2 parity divergence (mempool over-counts non-native amounts) is a class of bug that could recur with ANY new output type. The fix (filtering via `is_native_amount()`) is fragile because adding a new non-native output type requires updating the filter. The `AmmFlowSummary` approach (P2) would be more resilient because it classifies by output type at a single site.

4. **For the Minimal Design evaluator:** The simplest possible fix that is structurally sound is P1 alone (without P2/P3/P4). P1 collapses the 2 conservation sites into 1, which kills D2 by construction and provides a single place to add D3/D4/H2 bindings. P2/P3/P4 are refinements that improve the internal quality of the shared module but are not structurally necessary for conservation correctness.

5. **Security gap discovered:** AddLiquidity has ZERO token_b input binding in ANY site. An attacker could declare `new_reserve_b = old_reserve_b + X` without providing X tokens. This is a gap beyond the 6 defects in the brief.

6. **Security gap discovered:** Swap B->A has ZERO token_b input binding. The declared `new_reserve_b` increase is not verified against actual FungibleAsset inputs. D4 in the brief identifies the DOLI extraction side, but the token_b injection side is also unbound.

## Gaps

1. **RESOLVED: UtxoProvider implementation.** `storage::UtxoSet` implements `core::validation::UtxoProvider` (verified). The shared AMM verifier CAN use the UtxoProvider trait. However, the mempool's chain-lookup complexity means pre-extracted values are preferred.

2. **AddLiquidity token_b binding.** Neither the existing code nor the INC-I-096 patch binds token_b inputs for AddLiquidity. The structural check only verifies `new_reserve_b >= old_reserve_b`. An attacker could declare `new_reserve_b = old_reserve_b + 1000` without providing any token inputs. This gap exists in the current code AND in the INC-I-096 patch. It should be covered by the redesign.

3. **Swap B->A token_b input binding.** The existing Swap validation (utxo.rs:677-694) does NOT verify that the declared `new_reserve_b` increase is backed by actual FungibleAsset inputs for the B->A direction. It only checks `doli_out_from_pool <= old_reserve_a` (a trivial bound). D4 in the brief identifies this gap but the INC-I-096 patch's swap validation at utxo.rs:660-694 still does not bind token_b inputs for B->A.

4. **Block assembly path.** Confirmed that `build_block_content` does NOT perform independent AMM conservation checks -- it relies on mempool admission + consensus validation. This is the correct pipeline architecture.
