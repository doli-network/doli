# Design Brief: AMM Value-Conservation Layer (INC-I-096)

**Mode:** Proposal-only redesign (no `--fix`). **Scope:** `AMM value-conservation`. **Incident:** INC-I-096.

**⚠️ USER DIRECTION (binding for all evaluators):**
1. **CLEAN-SLATE.** Design the AMM value-conservation architecture from FIRST PRINCIPLES. A 6-change patch-set fix already exists in the working tree (gated inert), but you must NOT anchor to it or treat it as the baseline to refine. The user explicitly rejected "refine the existing fix." Design the conservation model you would build if starting fresh. You MAY read the existing code to understand the *problem* and the *constraints*, but do not adopt the patch-set's shape as a given. The fact that per-type patching keeps unmasking sibling vulnerabilities (the "Hydra pattern") is itself evidence that the patch shape is wrong.
2. **token_b (FungibleAsset) per-asset conservation is a hard MUST**, not a deferral. A DOLI-only conservation fix leaves token reserves drainable (SEC-LOGIC-002). Your design MUST conserve and input-bind the non-DOLI asset side too, bounded to AMM tx types (do NOT propose changing the system-wide `is_native_amount()` foundation used by all 27 tx types — that broader generalization is explicitly out of scope).

## Refined Prompt
Redesign the AMM value-conservation layer. The problem is STRUCTURAL, not a single bug. Across three enforcement sites — mempool admission, consensus validation, and apply_block — the AMM value-conservation model treats Pool UTXOs as ordinary native-amount UTXOs, and the validation pipeline trusts attacker-DECLARED pool state (new_reserve_a, new_reserve_b, new_total_lp) instead of binding it to the ACTUAL consumed transaction inputs. Successive code-level patches have each unmasked a sibling vulnerability. Propose a unified value-conservation architecture where pool reserve flows and LP supply changes are conserved and bound to inputs BY CONSTRUCTION, not by per-type ad-hoc checks.

## The Problem Space (the 6 defects + 2 wildcards — these are the PROBLEM, not the fix)
- **D1 — Conservation blind to Pool reserve release.** Native conservation at mempool (`crates/mempool/src/pool.rs:~383`, `total_input < total_output → MPTX008`) AND consensus (`crates/core/src/validation/utxo.rs:~210-217`, `→ InsufficientFunds`) is blind to Pool reserves. Pool UTXO has `output_type=Pool`, `amount=0`; reserves live in `extra_data`. `is_native_amount()` excludes Pool/LPShare/FungibleAsset. DOLI released from reserves in RemoveLiquidity / Swap B→A appears as a Normal output with NO covering native input → legitimate withdrawals falsely rejected. The bug is DIRECTIONAL: AddLiquidity and Swap A→B push DOLI INTO the pool (input>output) so they pass accidentally.
- **D2 — Mempool/consensus parity divergence.** Mempool `calculate_inputs` (`crates/mempool/src/pool.rs:~892,~916`) sums ALL `utxo.output.amount` unconditionally (counts LPShare share-amounts and FungibleAsset as native DOLI); consensus (`utxo.rs:~185`) filters `is_native_amount()`. Mempool OVER-counts → small burns pass mempool but die at consensus/block-assembly ("silent failure: blockHeight=None forever"); large burns fail mempool loudly (MPTX008).
- **D3 / SEC-LOGIC-001 (P0) — RemoveLiquidity unbound to inputs.** Consensus `validate_remove_liquidity` (`utxo.rs:~696-735`) checks pool_id stable, reserves decreased-or-equal, LP shares decreased — but does NOT bind doli_out/tokens_out to reserve deltas, NOR shares_burned to consumed LPShare inputs. `shares_burned = old_total_lp - new_total_lp` is computed from ATTACKER-controlled output `new_total_lp`. Declare `new_total_lp=0`, burn 1 share → proportional cap inflates to full pool → drain all reserves. The buggy conservation check was the only thing accidentally blocking this; removing it unmasks the drain.
- **D4 / SEC-LOGIC-002 (P0) — Swap B→A unbound to token inputs.** Does not bind declared `new_reserve_b` increase to actual FungibleAsset token inputs; k-invariant trivially satisfiable with tiny `new_reserve_a`. Declare huge `new_reserve_b` + 1 token in → extract ~all `reserve_a` DOLI. Same class as INC-I-092 RC-B (which covered CreatePool ONLY).
- **WILDCARD H1 (fix-breaking risk) — floor division.** Pool share math (`crates/core/src/validation/pool.rs` / `crates/core/src/pool.rs`, `da = shares * reserve_a / total_shares`) truncates toward zero. Any binding using EXACT equality between doli_out and reserve delta falsely rejects ~50% of legitimate removes. A correct binding must use `<=` (dust-to-pool) or replicate identical integer arithmetic in builder and validator.
- **WILDCARD H2 (now a MUST per user) — token_b has no conservation.** FungibleAsset (token_b) has NO per-asset supply conservation anywhere; only DOLI is governed by `is_native_amount`. A DOLI-only fix leaves token_b drainable.

## Root Pattern (why this is architectural, not 6 bugs)
Declared pool state (`new_reserve_a/b`, `new_total_lp`) is trusted without binding to actual consumed inputs. INC-I-092 RC-B applied input-backing to CreatePool ONLY. Every patch that fixes one tx-type's binding leaves the others exploitable — a Hydra. The builder helpers in `crates/core/src/pool.rs` (~7 functions: compute_swap, compute_remove_liquidity, verify_invariant, etc.) contain the CORRECT AMM math, but consensus NEVER calls them — it does ad-hoc structural checks. Builder computes; validator trusts.

## Architectural Constraints & Invariants ANY redesign MUST preserve
- **C1 — k-invariant:** `reserve_a_after * reserve_b_after >= reserve_a_before * reserve_b_before` for Swap (fees increase k).
- **C2 — MINIMUM_LIQUIDITY = 1000** locked permanently in first LP mint (`crates/core/src/consensus.rs`).
- **C3 — `compute_pool_id` includes fee_bps; IRREVERSIBLE post-activation** (`crates/core/src/transaction/output.rs:~729`).
- **C4 — Activation-height immutability:** never move a crossed height forward (INC-I-054).
- **C5 — Mempool/consensus MUST produce identical accept/reject** (no silent-failure threshold). This is the parity requirement.
- **C6 — INC-I-075 three-question checklist:** any consensus-visible, user-submittable change needs an activation height. INC-I-096's flip is reject→accept (and accept→reject for drains): Q1=YES, Q2=NO, Q3=NO → **a NEW `inc_i_096_activation_height` is REQUIRED** (do NOT reuse `inc_i_092_activation_height`; immutability).
- **C7 — ~30 external producers on local net:** no synchronized stop possible; activation height + rolling-deploy lead time mandatory. Mainnet pin = `u64::MAX` (with `amm_activation_height`); testnet = future height; devnet = 0.
- **C8 — Floor-division dust** (see H1): tolerance, not exact equality.
- **C9 — Fee split 25/5 bps (LP/protocol):** the conservation equation must account for protocol fee extraction (value leaving the pool to the reward pool) or it will falsely reject Swaps.
- **C10 — Pool UTXO consumed+recreated each op** (UTXO model; no in-place mutation). Pool `amount=0` is a structural given (do not propose moving reserves into `amount`).
- **C11 — Integer-only u64/u128 truncating arithmetic** (determinism; platform-independent).
- **C12 — INC-I-092 RC-A** (AMM Pool-input TXs exempt from fee/signature check) must be preserved.
- **Mainnet `amm_activation_height = u64::MAX`** → AMM NOT live → NO production value at risk. Redesign can land before activation.

## Capability Inventory (verified baseline — counted)
- **AMM tx types (4):** CreatePool, AddLiquidity, RemoveLiquidity, Swap (A→B and B→A directions).
- **Output types relevant (4):** Normal (native, counted), Pool (amount=0, reserves in extra_data, NOT counted), LPShare (NOT counted), FungibleAsset/token_b (NOT counted).
- **`is_native_amount()` set:** IN = Normal/Bond/Reward/Coinbase etc.; OUT = Pool/LPShare/FungibleAsset.
- **Existing conservation/binding checks (9 across 3 sites):** mempool native conservation; consensus native conservation; INC-I-092 RC-A fee/sig exemption; INC-I-092 RC-B CreatePool input-backing; RemoveLiquidity structural; Swap structural; Pool structural; builder-only k-invariant verify (NEVER called by consensus); apply_block duplicate-pool-id guard. Of these: 2 blind to Pool reserves, 1 covers only CreatePool, 2 structural-only, 1 builder-only.

## Redesign Acceptance Criteria (from analyst — REQ IDs)
MUST: VC-001 pool-aware conservation (both sites); VC-002 mempool/consensus parity (shared logic); VC-003 RemoveLiquidity input binding (doli_out/tokens_out to reserve deltas, shares_burned to consumed LPShare — kills SEC-LOGIC-001); VC-004 Swap input binding + consensus k-invariant re-verify (kills SEC-LOGIC-002); VC-005 AddLiquidity input binding; VC-006 floor-division dust tolerance (kills H1 false-rejects); VC-007 new `inc_i_096_activation_height` gating (mainnet=u64::MAX, testnet=future, devnet=0); VC-008 preserve CreatePool RC-B; **VC-009 token_b per-asset conservation (elevated to MUST per user)**.
SHOULD: VC-010 consensus re-verifies AMM math (reuse builder helpers or replicate); VC-011 single shared conservation function across sites; VC-012 back the ignored T10 drain test.
WON'T (this cycle): VC-014 system-wide `is_native_amount` value-delta ledger overhaul (blast radius across all 27 tx types).

## Enforcement Sites (read these — clean-slate, but understand the terrain)
- Mempool: `crates/mempool/src/pool.rs`
- Consensus: `crates/core/src/validation/utxo.rs`, `crates/core/src/validation/pool.rs`, `crates/core/src/validation/types.rs`
- Builder math: `crates/core/src/pool.rs`
- Block assembly + apply_block: `bins/node/src/node/production/assembly.rs`, `bins/node/src/node/apply_block/tx_processing.rs`, `bins/node/src/node/validation_checks.rs`
- Params/gating: `crates/core/src/network_params/{defaults,env_loader,mod}.rs`
- Constants: `crates/core/src/consensus.rs`; Pool id / output: `crates/core/src/transaction/output.rs`
- Existing fix (do NOT anchor; may skim for problem understanding): working-tree modifications + `crates/core/tests/inc_i_096_amm_conservation.rs`

## Specs / prior docs
- `specs/defi-foundations-economics.md`, `specs/defi-subsystem-architecture.md`, `specs/defi-l1-foundations-architecture.md`
- `docs/bugfixes/inc-i-096-amm-balance-check-analysis.md`, `docs/.workflow/security-audit-inc-i-096-logic.md`
- Full scoping: `docs/redesigns/amm-value-conservation-redesign-analysis.md`

## Memory / scope
INC_ID=INC-I-096. RUN_ID=none (proposal-only). DB: `.omega/memory.db` exists — log incident_entries during evaluation.
