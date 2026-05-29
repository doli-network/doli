<!--
OUTPUT CONTRACT: N/A — architecture specification file (not a test file)
INPUT PARTITIONS: N/A — architecture specification file (not a test file)
-->

# AMM Value-Conservation Architecture

INC-I-096 | Proposal-Only | 2026-05-29

## Problem Statement

The AMM value-conservation layer has a structural defect class -- "Trust the Client's Computed Result" (TCCR) -- where the validator accepts attacker-declared pool state (`new_reserve_a/b`, `new_total_lp`) instead of deriving it from consumed transaction inputs. Successive patches each unmask a sibling vulnerability (the "Hydra pattern"). Six defects (D1-D4, H1-H2) plus two new findings (FM-S11, P5) demonstrate the architecture is structurally unsound.

### Root Anti-Pattern: TCCR (Trust the Client's Computed Result)

9 declared values across 4 AMM tx types are trusted without derivation. Only 1 (CreatePool reserve backing, INC-I-092 RC-B at `utxo.rs:869-918`) has been fixed. The codebase already implements the correct "derive from flows" pattern in 5 other places (coinbase conservation, RC-B, native DOLI conservation, pool_id derivation, MINIMUM_LIQUIDITY). The AMM validator failed to generalize it.

### Why Refining the Existing Patch Was Rejected (P5)

The working-tree INC-I-096 patch derives `shares_burned = old_total_lp - new_total_lp` from the attacker-declared `new_total_lp` (`utxo.rs:796-808`), not from consumed LPShare UTXOs. An attacker holding 1 LP share can declare `new_total_lp=0`, inflating the proportional cap to the entire pool. **This drain is OPEN on devnet (gate=0)**. The ignored T10 test (`crates/core/tests/inc_i_096_amm_conservation.rs:852`) proves it. A clean-slate design was required because the patch shape inherits the TCCR anti-pattern in its core binding formula.

### FM-S11: Asset-ID Counterfeiting (New Finding)

The A-to-B Swap token conservation check (`utxo.rs:663-670`) filters on `OutputType::FungibleAsset` but NOT on `asset_id`. An attacker can produce FungibleAsset outputs with a foreign `asset_id`, effectively counterfeiting tokens of any asset using a cheap-token pool's reserves. The same gap likely exists at `utxo.rs:838-844` (RemoveLiquidity token output). Any token conservation must match on `asset_b_id`.

## Evaluation Summary

| Evaluator | Lens | Top Proposal | Confidence | Key Finding |
|-----------|------|-------------|------------|-------------|
| Subtractionist | removal | Remove mempool conservation reimplementation (-231 lines) | conf(0.65, observed) | 15 checks across 4 sites; ~231 mempool lines are redundant with consensus |
| Restructurer | boundaries | Extract shared `verify_amm_transition` to `validation/amm.rs` | conf(0.65, observed) | Builder math (pool.rs 7 fns) has 0 calls from any validator; SD-1 Compute/Verify split |
| Pattern Matcher | patterns | Generalize RC-B derive-from-flows to all 4 AMM types | conf(0.65, observed) | TCCR anti-pattern in 4/4 AMM types; only CreatePool (RC-B) fixed |
| Failure Analyst | failures | 9 hard filters; 5 OPEN drains; activation must not precede drain closure | conf(0.70, measured) | FM-S2 (LP-input underburn = full drain) and FM-S11 (asset_id counterfeiting) are new |
| Radical Simplifier | minimal | Single `verify_amm_conservation()` with 3 per-asset balance equations | conf(0.65, observed) | ~467 lines collapse to ~130; 3 equations + k-invariant = complete model |

## Convergence Matrix

### Deletion Convergence

```
                             Subtrac  Restruct  Pattern  Failure  Radical
Remove mempool conservation:   Y        Y         Y        -        Y     -> 4/5 -> DEFINITE
Remove ad-hoc utxo.rs checks:  Y        Y         Y        -        Y     -> 4/5 -> DEFINITE
Remove verify_invariant export: Y       -         -        -        -     -> 1/5 -> OPTION
```

### Restructuring Convergence

```
                                    Subtrac  Restruct  Pattern  Failure  Radical
Shared fn (mempool+consensus):        Y        Y         Y        Y        Y     -> 5/5 -> DEFINITE
New module crates/core/validation/:   -        Y         -        -        Y     -> 2/5 -> RECOMMEND
Per-asset value equations (E1/E2/E3): -        Y         Y        Y        Y     -> 4/5 -> DEFINITE
LP-input binding (shares_burned):     -        -         Y        Y        Y     -> 3/5 -> DEFINITE
k-invariant retained (Swap):          -        -         Y        Y        Y     -> 3/5 -> DEFINITE
Data interface (&[Output]):           Y        Y         -        -        Y     -> 3/5 -> DEFINITE
```

### Convergence Independence Check

**Deletion: Remove mempool conservation reimplementation**
- Subtractionist: measured 231 redundant lines; block assembly runs `validate_transaction_with_utxos` (assembly.rs:235) before inclusion
- Restructurer: SD-2 Duplicate Conservation; copy-paste anti-pattern; INC-I-096 deepens duplication
- Pattern Matcher: Bitcoin `CheckTransaction()` pattern (single validator, both sites)
- Radical: shared function eliminates the need for mempool's own copy
- INDEPENDENT? YES -- four different evidence sources (line count, structural defect, industry pattern, first-principles derivation)

**Restructuring: Per-asset balance equations (E1/E2/E3)**
- Restructurer: SD-3 Asset-Blind Accounting; AddLiquidity has ZERO token_b binding
- Pattern Matcher: Cardano multi-asset conservation (`Value = Map<PolicyId, Map<AssetName, Int>>`)
- Failure Analyst: FM-S3/S5/S11/S12 require token_b conservation; SEC-LOGIC-002 kill-criteria
- Radical: first-principles derivation from UTXO value extractors
- INDEPENDENT? YES -- structural defect observation, industry pattern, adversarial analysis, mathematical derivation

**Restructuring: LP-input binding**
- Pattern Matcher: P5 -- confirmed `shares_burned` attacker-controlled via `new_total_lp`
- Failure Analyst: FM-S2 + T10 test proves drain; FILTER-2 mandatory
- Radical: E3 equation binds LP supply through `lp_value()` extractor on consumed LPShare UTXOs
- INDEPENDENT? YES -- code audit, adversarial proof, mathematical model

## Definite Changes (High Convergence)

### DC-1: Single Shared Per-Asset Conservation Function

**What:** Create `verify_amm_conservation()` in a new `crates/core/src/validation/amm.rs` module (~120-130 lines) that implements three universal per-asset balance equations plus the k-invariant for Swap. Both mempool and consensus delegate to this single function.

**Convergence:** 5/5 evaluators (shared function); 4/5 (per-asset equations); conf(0.85, converged)

**Evidence:**
- `utxo.rs:595-918` (~320 lines of ad-hoc per-type checks) + mempool `pool.rs:380-437,926-992` (~113 lines) = ~467 lines of fragmented conservation logic
- `pool.rs` builder math (7 functions, 136 lines) called by CLI/RPC only -- ZERO calls from any validator (`utxo.rs`, `validation/pool.rs`, `mempool/pool.rs`)
- `storage::UtxoSet` implements `core::validation::UtxoProvider` (`crates/storage/src/utxo/set.rs:404`) -- dependency-clean
- RC-B at `utxo.rs:869-918` already implements the correct derive-from-flows pattern for CreatePool

**Per-Asset Value Extractors:**
```
doli_value(utxo)            = amount        if is_native_amount(output_type)
                              reserve_a     if Pool
                              0             otherwise

token_b_value(utxo, aid)    = amount        if FungibleAsset AND asset_id == aid
                              reserve_b     if Pool
                              0             otherwise

lp_value(utxo, pid)         = amount        if LPShare AND pool_id == pid
                              total_lp      if Pool
                              0             otherwise
```

**Balance Equations (per AMM tx type):**

| Tx Type | E1 (DOLI) | E2 (token_b) | E3 (LP supply) | k-invariant |
|---------|-----------|--------------|----------------|-------------|
| CreatePool | sum_in(doli) >= sum_out(doli) | sum_in(tok) >= sum_out(tok) | structural (MINIMUM_LIQUIDITY) | N/A |
| AddLiquidity | sum_in(doli) >= sum_out(doli) | sum_in(tok) >= sum_out(tok) | sum_in(lp) <= sum_out(lp) [minted LP bounded] | N/A |
| RemoveLiquidity | sum_in(doli) >= sum_out(doli) | sum_in(tok) >= sum_out(tok) | sum_in(lp) >= sum_out(lp) [burned LP bounded] | N/A |
| Swap | sum_in(doli) >= sum_out(doli) | sum_in(tok) >= sum_out(tok) | sum_in(lp) == sum_out(lp) [LP unchanged] | new_k >= old_k |

The `>=` absorbs floor-division dust (stays in pool, benefits remaining LPs). The consumed Pool UTXO contributes `reserve_a/b` and `total_lp` to the input-side sums; the output Pool UTXO contributes to the output-side sums.

**Data Interface:**
```rust
pub fn verify_amm_conservation(
    tx_type: TxType,
    consumed_outputs: &[Output],  // resolved by caller (mempool or consensus)
    tx_outputs: &[Output],        // the transaction's output list
    pool_asset_b_id: &Hash,       // from old pool metadata
    pool_id: &Hash,               // from old pool metadata
) -> Result<AmmConservationResult, ValidationError>
```

Takes `&[Output]` data, not a trait -- both mempool (via `UtxoSet`) and consensus (via `UtxoProvider`) construct the slice from their own lookups. Returns `AmmConservationResult { doli_surplus: u64 }` so the mempool can use it for fee computation without a separate path.

**k-Invariant Retained (Independent Guard):**
Conservation prevents THEFT (no asset created from nothing). The k-invariant prevents VALUE LEAK via mispriced swaps. Both are needed. Conservation does NOT call `compute_swap()` -- it is independent of AMM pricing math. The k-invariant check (currently inline at `utxo.rs:648-654`) moves into `verify_amm_conservation()` for Swap types.

**How This Kills Each Defect:**

| Defect | Kill Mechanism |
|--------|---------------|
| D1 (reserves invisible) | Pool reserves explicitly in `doli_value`/`token_b_value` sums |
| D2 (mempool/consensus parity) | Single function, two call sites -- parity by construction |
| D3 (RemoveLiquidity unbound) | E1 bounds DOLI out; E3 bounds shares_burned to consumed LPShare inputs |
| D4 (Swap B-to-A unbound) | E2 bounds token_b; declared `new_reserve_b` increase must be covered by FungibleAsset inputs |
| H1 (floor division) | `>=` (not `==`) absorbs truncation; u128 intermediate arithmetic |
| H2 (token_b conservation) | E2 is a first-class equation, same footing as E1 |
| FM-S2 (LP-input underburn) | E3 binds `new_total_lp` to consumed LPShare input amounts -- `shares_burned` derived from inputs, not declared |
| FM-S11 (asset_id counterfeiting) | `token_b_value` filters on `asset_id == pool.asset_b_id`, not just OutputType |
| P5 (existing patch still drainable) | `shares_burned` no longer computed from attacker-declared `new_total_lp` |

### DC-2: Delete Mempool Conservation Reimplementation

**What:** Remove the mempool's pool-aware DOLI conservation (`pool.rs:380-437`) and the `is_native_amount` gating in `calculate_inputs` (`pool.rs:926-982`). Replace with a call to `verify_amm_conservation()`. Use the returned `doli_surplus` as the fee for AMM txs.

**Convergence:** 4/5 evaluators; conf(0.85, converged)

**Evidence:**
- Mempool `pool.rs:380-437` (57 lines) + `pool.rs:926-982` (66 lines) = ~113 lines of reimplemented conservation with historically divergent semantics (D2)
- Block assembly runs `validate_transaction_with_utxos` (`assembly.rs:235`) before including any mempool tx -- consensus is authoritative
- AMM txs are fee-exempt (`utxo.rs:268-276`) -- mempool fee computation for AMM is only for eviction priority, not a security check

### DC-3: Delete Ad-Hoc UTXO-Context AMM Checks Above Gate

**What:** Above `inc_i_096_activation_height`, the per-type ad-hoc check blocks in `utxo.rs` (lines 595-918) are replaced by delegation to `verify_amm_conservation()`. Below the gate, they remain bit-identical. The structural validation in `crates/core/src/validation/pool.rs` (230 lines -- input/output counts, output types, MINIMUM_LIQUIDITY, pool_id derivation) is RETAINED unmodified as defense-in-depth.

**Convergence:** 4/5 evaluators; conf(0.85, converged)

**Evidence:**
- `utxo.rs:595-918` = ~320 lines of ad-hoc checks that trust declared state (root cause of D3, D4) and have 5 binding gaps
- Structural validators in `validation/pool.rs` are orthogonal (shape validation before UTXO resolution) and correct -- no change needed

### DC-4: Reject Malformed Pool Metadata (No Silent Fallbacks)

**What:** Replace `unwrap_or(0)` at `utxo.rs:238-244` and `mempool/pool.rs:409,414` with explicit rejection (`ValidationError`/`MempoolError`) when `pool_metadata()` returns `None` on AMM transactions. Above the gate only.

**Convergence:** 3/5 evaluators (Failure, Pattern, Radical); conf(0.85, converged)

**Evidence:** `utxo.rs:239` `.unwrap_or(0)` silently zeros reserves on malformed Pool UTXOs, making conservation vacuous. Not currently reachable in production (Pool UTXOs always have valid metadata), but architecturally unsound.

## Recommended Changes (Medium Convergence)

### RC-1: New Module at `crates/core/src/validation/amm.rs`

**What:** House the shared function in a new `amm.rs` file within the existing `validation/` directory rather than inlining it in `utxo.rs`.

**Convergence:** 2/5 evaluators (Restructurer, Radical); conf(0.7, converged)

**Evidence:**
- `validation/utxo.rs` is already 1177 lines; adding a ~130-line function would push it further
- `validation/pool.rs` already handles structural AMM validation; `amm.rs` for conservation is a clean parallel
- Dependency direction clean: `crates/core` internal module, no new cross-crate deps

### RC-2: Promote `verify_invariant` to Consensus (or Remove Dead Export)

**What:** Either have the shared function call `pool::verify_invariant()` for the k-invariant check (promoting it from builder-only to consensus-critical), or demote the export to `pub(crate)` (currently exported at `lib.rs:267`, called only in `pool.rs` tests).

**Convergence:** 1/5 (Subtractionist conf(0.7)); recommendation based on the shared function already needing a k-invariant check

**Evidence:** Consensus at `utxo.rs:648-654` reimplements the same 5-line check that `pool::verify_invariant()` already provides. The shared function will need one or the other.

## Adopted Additions (User-Approved 2026-05-29)

Both options below were APPROVED by the user at the redesign gate and are now part of the locked design (no longer optional).

### OPTION A — ADOPTED: Validator Re-Verifies Builder Math (VC-010)

**From:** Restructurer (P4, conf(0.60)) + Pattern Matcher (P4, conf(0.55))

**What:** Inside the shared function, for LP operations, call `pool::compute_remove_liquidity()` and `pool::compute_lp_shares()` to re-verify that declared new pool state is reachable from old state + inputs. For Swap, use conservation + k-invariant only (calling `compute_swap()` is over-constraining per Pattern Matcher's kill test). This makes the `pool.rs` LP-math functions consensus-critical (see RC-2) — any future change to them requires an activation height.

**Evidence:** The 7 `pool.rs` math functions are correct but never called by validation. Making them consensus-critical means any future change requires an activation height.

**Complexity cost:** +~60 lines in the shared function. Makes `pool.rs` functions consensus-critical.

**Failure mode filter:** NEUTRAL -- conservation + k-invariant already bound all 4 types; re-verification is belt-and-suspenders.

**vs. Radical floor:** +60 lines above minimum viable. The radical minimum uses only conservation equations + k-invariant without calling builder math.

**Confidence:** conf(0.55, inferred) -- Pattern Matcher partially killed it for Swap (over-constraining). Useful for LP ops only.

### OPTION B — ADOPTED: Return Surplus for Mempool Fee Computation

**From:** Radical Simplifier (P5, conf(0.5))

**What:** `verify_amm_conservation()` returns `AmmConservationResult { doli_surplus: u64 }`. The mempool uses `doli_surplus` as the fee; consensus ignores it (AMM txs are fee-exempt).

**Evidence:** The mempool currently computes fee at `pool.rs:401-427` as `(total_input + old_reserve_a) - (total_output + new_reserve_a)`. This IS the DOLI surplus from E1.

**Complexity cost:** +1 struct (2 fields). Avoids the mempool needing a separate fee-computation path.

**Failure mode filter:** NEUTRAL.

**vs. Radical floor:** Already part of the radical minimum.

**Confidence:** conf(0.55, inferred) -- implementable but the mempool could also just compute fee = 0 for AMM txs (they are fee-exempt).

## Constraints (from Failure Analyst -- HARD FILTERS)

All 9 filters are binding on any implementation of this spec.

| Filter | Requirement | Evidence |
|--------|-------------|----------|
| FILTER-1 | ATOMIC LIVENESS-SAFETY COUPLING: must not fix liveness (admit RemoveLiquidity/B-to-A Swap) without simultaneously closing ALL drains (FM-S1 through FM-S5, FM-S11) | The buggy conservation check is the only thing blocking SEC-LOGIC-001/002 |
| FILTER-2 | LP-INPUT BINDING MANDATORY: bind `shares_burned` to sum of consumed LPShare input amounts (not from declared `new_total_lp`) | FM-S2 (T10 drain), utxo.rs:796 |
| FILTER-3 | TOKEN_B CONSERVATION MANDATORY: per-asset conservation for token_b, matching `asset_b_id`. MUST NOT modify `is_native_amount()` | FM-S3, FM-S5, FM-S11, FM-S12 |
| FILTER-4 | MEMPOOL/CONSENSUS PARITY: identical accept/reject for all AMM txs | FM-L3, D2 |
| FILTER-5 | FLOOR-DIVISION TOLERANCE: `<=` not `==`; bit-identical u128 formula in builder and validator | FM-L4, FM-D1 |
| FILTER-6 | ACTIVATION HEIGHT ISOLATION: use `inc_i_096_activation_height` (mod.rs:482); MUST NOT reuse `inc_i_092`; MUST NOT activate until all drains closed | FM-P1, FM-P3, C4, C6 |
| FILTER-7 | ADDLIQUIDITY PROPORTIONAL BINDING: LP minted proportional to reserves added; declared reserve increases match consumed inputs | FM-S4, FM-S5, FM-S12 |
| FILTER-8 | NO SILENT FALLBACKS: reject AMM tx where `pool_metadata()` is None; no `unwrap_or(0)` | FM-D3, utxo.rs:238-244 |
| FILTER-9 | ASSET_ID CROSS-CHECK: FungibleAsset outputs must carry pool's `asset_b_id` | FM-S11, utxo.rs:663-670 |

### Per-P0 Kill-Criteria

**SEC-LOGIC-001 (RemoveLiquidity):** 5 checks ALL necessary:
1. `lp_inputs_consumed = sum(LPShare inputs where pool_id matches)`
2. `declared_burned = old_total_lp - new_total_lp`
3. Reject if `declared_burned > lp_inputs_consumed` (FM-S2)
4. Reject if `actual_reserve_delta > (declared_burned * reserve / old_total_lp)` (FM-S1)
5. Reject if `tokens_out > actual_reserve_b_delta` filtered by `asset_b_id`

**SEC-LOGIC-002 (B-to-A Swap):** 6 checks ALL necessary:
1. `token_b_inputs = sum(FungibleAsset inputs where asset_id == pool.asset_b_id)`
2. `declared_reserve_b_increase = new_reserve_b - old_reserve_b`
3. Reject if `declared_reserve_b_increase > token_b_inputs` (FM-S3)
4. `new_k >= old_k` (k-invariant)
5. Pool-aware DOLI conservation (E1)
6. FungibleAsset outputs must have `asset_id == pool.asset_b_id` (FM-S11)

## Invariant Table

| ID | Invariant | Source | Test Required |
|----|-----------|--------|---------------|
| INV-SAFETY-001 | `native_input + old_reserve_a >= native_output + new_reserve_a` for all AMM tx types | E1 equation | YES -- per tx type |
| INV-SAFETY-002 | `new_k >= old_k` for Swap (fees increase k) | k-invariant | YES -- A-to-B and B-to-A |
| INV-SAFETY-003 | `MINIMUM_LIQUIDITY = 1000` locked in first LP mint | C2 | Existing tests |
| INV-SAFETY-004 | `compute_pool_id` includes `fee_bps` (IRREVERSIBLE post-activation) | C3 | Existing tests |
| INV-SAFETY-005 | `token_b_input + old_reserve_b >= token_b_output + new_reserve_b` (per `asset_b_id`) | E2 equation | YES -- per tx type |
| INV-SAFETY-006 | LP supply conservation: direction-appropriate E3 per tx type | E3 equation | YES -- Create, Add, Remove, Swap |
| INV-SAFETY-007 | `shares_burned` derived from consumed LPShare inputs, NOT declared `new_total_lp` | FILTER-2 | YES -- T10 must PASS |
| INV-SAFETY-008 | FungibleAsset outputs carry pool's `asset_b_id` | FILTER-9 / FM-S11 | YES -- cross-pool counterfeit |
| INV-DEPLOY-001 | `inc_i_096_activation_height` MUST NOT be set to a real value until ALL FILTER-1..9 are satisfied | FILTER-6 | Process gate |
| INV-DEPLOY-002 | `amm_activation_height >= inc_i_096_activation_height` (assertion) | Failure Analyst P3 | Compile-time or runtime assert |
| INV-COMPAT-001 | Below `inc_i_096_activation_height`, behavior is bit-identical to current (buggy) code | C4 / C7 | YES -- pre-gate regression |
| INV-DETERM-001 | All AMM arithmetic uses u64/u128 integer division; builder and validator use same formula | C11 / FM-D1 | YES -- golden vectors |

## Architecture Maps

### Current Architecture

```
CLI (cmd_pool.rs)
  calls pool.rs: compute_swap, compute_remove_liquidity, compute_lp_shares
  builds Transaction with declared new_pool_state in output[0].extra_data

Mempool (mempool/pool.rs)
  ~113 lines: reimplements pool-aware DOLI conservation + calculate_inputs
  ZERO token_b checks
  ZERO LP binding
  Divergent semantics from consensus (D2)

Consensus (validation/utxo.rs)
  ~320 lines: per-type ad-hoc checks (Swap:107, Add:39, Remove:107, Create:62)
  ZERO calls to pool.rs math
  Partial token_b (A-to-B Swap only)
  Partial LP binding (RemoveLiquidity only, gated, attacker-controlled shares_burned)

Structural (validation/pool.rs)
  ~230 lines: shape checks, pool_id, MINIMUM_LIQUIDITY
  CORRECT, INDEPENDENT of conservation

apply_block (tx_processing.rs)
  ~10 lines: delegates to validate_transaction_with_utxos + pool_id uniqueness guard
```

### Proposed Architecture (Definite + Recommended)

```
CLI (cmd_pool.rs)
  calls pool.rs: compute_swap, compute_remove_liquidity, compute_lp_shares
  builds Transaction with declared new_pool_state in output[0].extra_data
  (UNCHANGED)

  +-- NEW: crates/core/src/validation/amm.rs (~130 lines)
  |   verify_amm_conservation(tx_type, consumed_outputs, tx_outputs, asset_b_id, pool_id)
  |   -> AmmConservationResult or ValidationError
  |   Implements: E1 (DOLI), E2 (token_b by asset_id), E3 (LP supply), k-invariant
  |   Data interface: &[Output] -- no trait dependency
  |

Mempool (mempool/pool.rs)
  -113 lines: conservation logic DELETED
  +~10 lines: resolves consumed UTXOs, calls verify_amm_conservation()
  Uses returned doli_surplus for fee

Consensus (validation/utxo.rs)
  -320 lines: per-type ad-hoc blocks (above gate, delegated)
  +~10 lines: calls verify_amm_conservation()
  Below gate: existing code preserved bit-identical

Structural (validation/pool.rs)
  ~230 lines: UNCHANGED (shape checks, pool_id, MINIMUM_LIQUIDITY)

apply_block (tx_processing.rs)
  UNCHANGED (delegates to validate_transaction_with_utxos)
```

## Migration Path

### Step 1: Create `crates/core/src/validation/amm.rs`

- Implement `verify_amm_conservation()` with the three per-asset balance equations + k-invariant
- Takes `&[Output]` for consumed inputs and `&[Output]` for tx outputs
- Gating: function takes `height` and `activation_height` parameters; returns `Ok(bypass)` below gate
- Add module to `validation/mod.rs`
- Unit tests with golden vectors for all 4 AMM tx types, both directions of Swap, floor-division edge cases, and all 9 drain vectors (including T10 / FM-S2 and FM-S11)

### Step 2: Wire consensus call site

- In `utxo.rs`, after the structural validation blocks, add a call to `verify_amm_conservation()` gated by `inc_i_096_activation_height`
- Resolve consumed UTXOs from `utxo_provider` (already done in the existing per-type blocks)
- The existing per-type blocks (`utxo.rs:595-918`) remain for below-gate bit-identical behavior; above gate, the shared function takes over
- CreatePool RC-B (`utxo.rs:869-918`, gated by `inc_i_092`) is subsumed by E1+E2 above `inc_i_096` gate; remains active for heights between `inc_i_092` and `inc_i_096`

### Step 3: Wire mempool call site

- In `mempool/pool.rs`, replace the pool-aware conservation block (`pool.rs:380-437`) and the `is_native_amount` gating in `calculate_inputs` (`pool.rs:926-982`) with a call to `verify_amm_conservation()`
- The mempool resolves consumed UTXOs from its `UtxoSet` (already done at `pool.rs:940-961`)
- Use the returned `doli_surplus` as the fee for AMM txs

### Step 4: Replace `unwrap_or(0)` with explicit rejection

- At `utxo.rs:238-244` and `mempool/pool.rs:409,414`: above `inc_i_096` gate, return error on `None` pool metadata for AMM txs

### Step 5: Add `amm_activation_height >= inc_i_096_activation_height` assertion

- In `NetworkParams` validation (or a `debug_assert!` / runtime check on startup): ensure AMM cannot activate before INC-I-096 conservation is live

### Step 6: Activation

- Devnet: `inc_i_096_activation_height = 0` (already set)
- Testnet: set to future height after all tests pass
- Mainnet: `u64::MAX` (co-pinned with `amm_activation_height`); set to real value only after full audit

### What Gets Deleted (above gate)

| File | Lines | Content |
|------|-------|---------|
| `mempool/pool.rs` | 380-437 | Pool-aware DOLI conservation |
| `mempool/pool.rs` | 926-982 | `calculate_inputs` `is_native_amount` gating |
| `utxo.rs` | 595-702 | Swap ad-hoc checks (above gate) |
| `utxo.rs` | 705-744 | AddLiquidity ad-hoc checks (above gate) |
| `utxo.rs` | 747-854 | RemoveLiquidity ad-hoc checks (above gate) |

**Total: ~467 lines of scattered conservation logic replaced by ~130-line shared function + ~20 lines of call-site glue.**

### What Is Preserved

| File | Lines | Content | Reason |
|------|-------|---------|--------|
| `validation/pool.rs` | 1-230 | Structural validation | Orthogonal; defense-in-depth |
| `utxo.rs` | 164-170 | RC-A Pool input sig exemption | Load-bearing (C12) |
| `utxo.rs` | 869-918 | RC-B CreatePool input backing | Active between inc_i_092 and inc_i_096 gates |
| `utxo.rs` | 595-854 | Per-type ad-hoc checks (below gate) | Bit-identical pre-gate behavior (INV-COMPAT-001) |
| `tx_processing.rs` | 134-143 | Duplicate pool_id guard | State-level uniqueness |

## Complexity Comparison

| Metric | Current (patch-set) | Radical Minimum | Proposed |
|--------|--------------------|--------------------|----------|
| Conservation enforcement sites | 3 (mempool, utxo native, utxo per-type) | 1 (shared fn, 2 call sites) | 1 (shared fn, 2 call sites) |
| Lines of conservation logic | ~467 | ~130 | ~130 |
| Declared-state trust points | 5 partial (varying by tx type) | 3 universal (E1, E2, E3) | 3 universal (E1, E2, E3) |
| Per-tx-type special cases | 4 bespoke blocks (302 lines) | 4 arms in 1 dispatch (~80 lines) | 4 arms in 1 dispatch (~80 lines) |
| Mempool/consensus parity | Manual duplication | Shared fn (guaranteed) | Shared fn (guaranteed) |
| token_b coverage | Partial (3 of 4 tx types) | Full (4 of 4) | Full (4 of 4) |
| LP supply coverage | 1 tx type (gated, attacker-controlled) | All 4 | All 4 |
| New modules | 0 | 1 | 1 |
| Code blocks removed | 0 | 7 | 7 |
| Net line delta | 0 | ~-330 | ~-330 |

**SSF Verdict:** The Radical minimum and the converged proposal are the SAME design. There is no gap to bridge. The per-asset balance equation model IS the simplest correct architecture.

## Acceptance Criteria

### MUST (VC-001..009)

| ID | Criterion | How Met |
|----|-----------|---------|
| REQ-AMM-VC-001 | Pool-aware conservation (both sites) | E1 equation in shared fn, called by both mempool and consensus |
| REQ-AMM-VC-002 | Mempool/consensus parity (shared logic) | Single `verify_amm_conservation()` function; FILTER-4 |
| REQ-AMM-VC-003 | RemoveLiquidity input binding (shares_burned to consumed LPShare, doli_out/tokens_out to reserve deltas) | E3 binds LP inputs; E1/E2 bind reserve deltas; SEC-LOGIC-001 kill-criteria |
| REQ-AMM-VC-004 | Swap input binding + consensus k-invariant re-verify | E2 binds token_b; k-invariant in shared fn; SEC-LOGIC-002 kill-criteria |
| REQ-AMM-VC-005 | AddLiquidity input binding | E1/E2 bind reserve increases to inputs; E3 bounds LP minted; FILTER-7 |
| REQ-AMM-VC-006 | Floor-division dust tolerance | `>=` in E1/E2/E3; u128 intermediate; FILTER-5 |
| REQ-AMM-VC-007 | New `inc_i_096_activation_height` gating | `mod.rs:482` field; mainnet=u64::MAX; FILTER-6 |
| REQ-AMM-VC-008 | Preserve CreatePool RC-B | RC-B active between inc_i_092 and inc_i_096 gates; E1+E2 subsume above inc_i_096 |
| REQ-AMM-VC-009 | token_b per-asset conservation (MUST per user) | E2 equation with `asset_b_id` matching; FILTER-3, FILTER-9 |

### SHOULD (VC-010..012)

| ID | Criterion | Status |
|----|-----------|--------|
| REQ-AMM-VC-010 | Consensus re-verifies AMM math | ADOPTED (OPTION A) — LP ops call pool::compute_remove_liquidity/compute_lp_shares; Swap uses conservation + k-invariant |
| REQ-AMM-VC-011 | Single shared conservation function | DC-1 (definite) |
| REQ-AMM-VC-012 | Back the ignored T10 drain test | Required for INV-SAFETY-007 |
| REQ-AMM-VC-013 | `verify_amm_conservation` returns `AmmConservationResult { doli_surplus }` for mempool fee | ADOPTED (OPTION B) |

### WON'T (this cycle)

| ID | Criterion | Reason |
|----|-----------|--------|
| REQ-AMM-VC-014 | System-wide `is_native_amount` value-delta ledger overhaul | Blast radius across all 27 tx types; bounded AMM-scoped conservation achieves the goal |

## Activation-Height Plan

| Network | `inc_i_096_activation_height` | `amm_activation_height` | Assertion |
|---------|-------------------------------|--------------------------|-----------|
| Mainnet | `u64::MAX` (co-pinned) | `u64::MAX` (AMM not live) | `amm_ah >= inc_i_096_ah` |
| Testnet | Future height (set after all tests pass) | `u64::MAX` | `amm_ah >= inc_i_096_ah` |
| Devnet | `0` (always-on) | `u64::MAX` | `amm_ah >= inc_i_096_ah` |

**Critical:** `inc_i_096_activation_height` MUST NOT be set to a real mainnet value until ALL FILTER-1 through FILTER-9 are verified to hold in the implementation.

## Milestones

This redesign touches 3 modules (amm.rs new, utxo.rs modified, mempool/pool.rs modified) -- below the 4-module threshold for mandatory milestones. However, for safety:

| Milestone | Scope | Gate |
|-----------|-------|------|
| M1 | `amm.rs` shared function + unit tests (golden vectors for all 4 tx types + all 9 drain vectors) | All tests pass; T10 drain test PASSES (not ignored) |
| M2 | Wire consensus call site (utxo.rs delegation above gate) + integration tests | Pre-gate behavior bit-identical; above-gate drain vectors rejected |
| M3 | Wire mempool call site + delete duplication + `unwrap_or(0)` fix | Mempool/consensus parity tests pass; fee computation correct |
| M4 | Activation-height assertion + testnet deployment | Testnet live with real height; no forks |

## Design Synthesis Quality Gate

```
--- DESIGN SYNTHESIS QUALITY GATE ---
Evaluators completed:           5/5
Deletion convergence items:     2 (4/5 agreement)
Restructuring convergence:      6 (3+/5 agreement on all 6)
Addition options presented:     2
Failure modes identified:       12 safety + 5 liveness + 3 determinism + 3 deploy = 23 (from Failure Analyst)
Failure modes applied as filters: 9/9 hard filters applied
Radical floor gap:              [467 lines, 3 sites] -> [130 lines, 1 site] -> [130 lines, 1 site]
Contradictions found:           1 (resolved)
Contradictions resolved:        1/1
Evidence independence verified: YES (3 convergence clusters verified)
--------------------------------------
```
