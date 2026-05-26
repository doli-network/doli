<!--
OUTPUT CONTRACT: N/A — architecture specification (not a test file)
INPUT PARTITIONS: N/A — architecture specification (not a test file)
-->

# DeFi L1 Foundations -- Architecture (2026-05-26)

## Executive Decision

DOLI's DeFi L1 surface should be pruned from 13 frozen tx types to 5 active DeFi tx types (4 AMM + 1 ZKSettle) plus 1 gated oracle type, with 7 tx types tombstoned (5 lending + 2 NFT-frac) and their ~3,500 lines of dead code removed. The condition guard system (5 guards, 6 templates) and FungibleAsset/BridgeHTLC primitives remain unchanged. Three pre-activation fixes are mandatory before any activation height is set to a real value: MintAsset issuer authorization dead code, compute_swap overflow, and oracle sunset gradient.

This proposal reflects 4/5 evaluator convergence on tombstoning lending and 3/5 on tombstoning NFT-frac, confirmed by independent evidence from each lens. The AMM is retained as an L1 primitive (4/5 convergence) despite the Radical Simplifier's dissent -- the 0.10 confidence delta triggers the SSF gate, so the radical alternative is presented in Section F for the User Gate decision. The oracle is retained as an architecturally independent Phase 2.1 primitive with its own activation height, contingent on the sunset gradient fix (D.3).

## Reasoning Trace Reference

See `docs/.workflow/architecture-reasoning.md` for the full reasoning trace including per-evaluator evidence, convergence independence checks, failure mode filtering, and contradiction resolution.

---

## A. KEEP -- Foundation Primitives

### A.1 AMM (CreatePool=19, AddLiquidity=20, RemoveLiquidity=21, Swap=22 + Pool/LPShare outputs)

**Status:** Shipped, frozen at `amm_activation_height = u64::MAX`. Zero UTXOs on any chain.

**Why foundational:** Constant-product AMM is DOLI's price-discovery layer for FungibleAsset tokens. The DOLI-only pairing constraint creates a natural routing hub (Osmosis early-stage pattern). The 25/5 fee split (LP/protocol) is the Uniswap v3 "fee switch" shipped at activation, avoiding retroactive governance fights. Integer-only math (u64/u128) with truncation-toward-pool rounding is deterministic across all platforms.

**L2 consumption:** L2 builders use AMM pools for on-chain price reference via TWAP accumulator (`pool.rs:82-108`). ZKSettle L2s can use Pool UTXO state as a price anchor for their own DeFi products.

**Agent readiness:** 4/4. Discoverable via `getPoolList`/`getPoolInfo` RPC. Deterministic (integer math, no callbacks). Bounded execution (worst case = not included due to Pool UTXO contention). Composable via standard UTXO semantics.

**Convergence:** 4/5 evaluators (Subtraction, Restructure, Patterns, Failure) support KEEP. Radical dissents at conf(0.55, inferred). See Section F.

**Evidence:** `pool.rs:13-37` implements Uniswap v2 formula. `validation/pool.rs:112-121` enforces k-invariant. `consensus/constants.rs:336` sets MINIMUM_LIQUIDITY=1000.

**Confidence:** conf(0.70, converged)

### A.2 Oracle (PriceAttestation=16 + OraclePrice=15)

**Status:** Shipped (M1-M11 complete), frozen at `oracle_activation_height = u64::MAX`. Architecturally independent from AMM.

**Why foundational:** Bond-weighted median provides on-chain price data without external oracle dependency. Sunset HALT at 55% structural share is a genuine defense that Chainlink/Terra lacked. The oracle is a general-purpose data availability primitive, not strictly a DeFi dependency.

**L2 consumption:** L2 builders can consume OraclePrice UTXOs as trustworthy L1 price references for their own lending/perps/stablecoin protocols.

**Convergence:** 3/5 evaluators support KEEP (Subtraction implicit, Patterns explicit, Failure conditional). Radical proposes remove (L2 can use own oracles). Restructure says placement is correct (B2, conf 0.60).

**Constraints:** MUST fix sunset gradient (D.3) before activation. The 12-producer hardcoded set is honestly disclosed as centralized (spec S6). Growth path to on-chain derivable set is Phase 2.3 work (not this spec).

**Evidence:** `oracle/mod.rs:106-153` implements bond-weighted median. `SUNSET_THRESHOLD_BPS=5500` at `oracle/mod.rs:193`. All 11 milestones committed (M1 d80f127f through M11 2d28c4bf).

**Confidence:** conf(0.60, converged)

### A.3 ZKSettle (=31) + ZKRollup output (L2 verification surface)

**Status:** Shipped as stub, frozen. 259 LOC in `validation/zk.rs`.

**Why foundational:** This IS the L2 strategy. ZKSettle provides universal settlement for arbitrary L2 DeFi (AMM, lending, perps, stablecoins) with no UTXO contention ceiling. L1's job: finality + asset custody + proof verification. ZKSettle delivers item (c).

**L2 consumption:** L2 builders deploy arbitrary DeFi circuits off-chain, settle state transitions on L1 via a single ZKSettle tx per batch. No per-pool contention.

**L1/L2 boundary (Restructure B6, conf 0.55):** Clean. L1 validates "does this proof verify?" L1 has zero knowledge of L2 state transitions. `ZkRollupData` contains only opaque fields: `rollup_id`, `verifying_key`, `state_root`, `proof_system_id`.

**Evidence:** `validation/zk.rs:108` self-contained. `validation/utxo.rs:849-960` enforces ZKRollup spend rules (ERRTX-ZK012 through ERRTX-ZK020).

**Confidence:** conf(0.55, observed) -- stub only, no verifier wired yet.

### A.4 Condition guards (5 shipped: Amount, OutputType, Recipient, MaxDelta, ReserveRatio)

**Status:** Live. ~3,392 LOC in `crates/core/src/conditions/`. MAX_CONDITION_DEPTH=4, MAX_CONDITION_OPS=128.

**Why foundational:** These are DOLI's covenant system -- stronger than Bitcoin OP_CTV (output introspection, not just hash commitment). They enable bilateral DeFi without new tx types. The escrow-loan template composes AmountGuard + RecipientGuard + Timelock to achieve collateralized lending -- zero dependency on the tombstoned native lending code.

**Restructure verdict (B4, conf 0.55):** MaxDeltaGuard and ReserveRatioGuard belong in `conditions/mod.rs`, NOT a separate `defi.rs`. They are generic guards usable for non-DeFi patterns.

**Evidence:** `eval.rs:121-191` implements guard evaluation. `templates.rs:229-246` composes escrow-loan from guards. 6/6 Phase 1 DeFi patterns expressible without new tx types (Patterns kill test).

**Confidence:** conf(0.65, observed)

### A.5 Covenant templates (6 shipped: vault, escrow, htlc_payment, subscription, agent_allowance, escrow_loan)

**Status:** Live (pending `guards_activation_height` on mainnet). ~250 LOC in `templates.rs`.

**Why foundational:** These are composable DeFi building blocks. The escrow-loan template specifically replaces native lending types, validating the "compose from guards" strategy. All 6 templates use depth <= 3, leaving 1 level for user composition around the template.

**Evidence:** `templates.rs` demonstrates all 6 patterns. The approved `defi-foundations-economics.md` spec lists escrow-loan CLI template as P7 (approved).

**Confidence:** conf(0.60, observed)

### A.6 FungibleAsset + MintAsset/BurnAsset (token issuance, types 17/18)

**Status:** Live (MintAsset/BurnAsset are behind `defi_activation_height`). OutputType 7 (FungibleAsset).

**Why foundational:** User-issued tokens are the AMM's counterparty asset. Every AMM pool pairs DOLI with a FungibleAsset. Token issuance is irreducibly L1 -- L2 cannot create L1-native assets.

**CRITICAL PRE-ACTIVATION FIX REQUIRED:** MintAsset issuer authorization is dead code in production (D.1). Must fix before `defi_activation_height` is set to a real value.

**Evidence:** `output.rs` FungibleAsset constructors. `validation/utxo.rs:376-468` UTXO-level validation. `storage/utxo/set.rs:417` returns `pubkey: None` (the F1 bug).

**Confidence:** conf(0.60, observed)

### A.7 BridgeHTLC + adaptor signatures (cross-chain)

**Status:** Live. OutputType 8 (BridgeHTLC). Adaptor sigs in `crates/crypto/src/adaptor.rs`.

**Why foundational:** Enables atomic swaps between DOLI and Bitcoin/Ethereum. AUDIT-BRIDGE-001 fixed (signed refund at `conditions/mod.rs:347`). No operator required -- minimal maintenance surface.

**Evidence:** `conditions/mod.rs:347-363` implements `htlc_signed_refund`. `output.rs:704-708` BridgeHTLC metadata.

**Confidence:** conf(0.55, observed)

---

## B. TOMBSTONE -- Subtraction Targets

### B.1 Native lending subsystem (5 tx types + 2 output types + ~3,237 LOC + RPCs + CLI)

**What:** CreateLoan=24, RepayLoan=25, LiquidateLoan=26, LendingDeposit=27, LendingWithdraw=28. OutputType Collateral=11, LendingDeposit=12. Files: `lending.rs` (496), `validation/lending.rs` (568), `rpc/methods/lending.rs` (128), `cmd_loan.rs` (1,109), output.rs helpers (~300), UTXO storage helpers (~30), lib.rs re-exports (~20), validation match arms (~50), INC-I-088 freeze guard (~18), partial test files (~600).

**Convergence:** 4/5 evaluators independently agree on removal.
- Subtraction: conf(0.70, measured) -- full caller analysis, wire-format verification
- Restructure: conf(0.65, measured) -- false coupling across 4 crates (B1)
- Patterns: conf(0.65, observed) -- Aave-on-L1 anti-pattern, escrow-loan supersedes
- Radical: conf(0.55, inferred) -- remove all native DeFi including lending

CONVERGENCE INDEPENDENCE CHECK:
- Subtraction based on: dead code inventory (LOC counts, grep for callers, gate verification)
- Restructure based on: cross-crate dependency chain analysis (core -> validation -> RPC -> storage)
- Patterns based on: industry pattern matching (Aave-on-L1 anti-pattern, CTV equivalence)
- Radical based on: first-principles L1/L2 boundary argument
INDEPENDENT? YES -- four different evidence sources. True convergence. conf boost applies.

**Mechanism:**
- DELETE files: `lending.rs`, `validation/lending.rs`, `rpc/methods/lending.rs`, `cmd_loan.rs`
- TOMBSTONE discriminants: TxType 24-28 return `None` from `from_u32` with tombstone comment
- TOMBSTONE discriminants: OutputType 11, 12 return `None` from `from_u8` with tombstone comment
- DELETE: output.rs constructors/constants/structs (`collateral()`, `lending_deposit()`, `CollateralMetadata`, etc.)
- DELETE: `get_all_collateral()` from 3 UTXO storage files
- DELETE: validation match arms, RPC dispatch entries, lib.rs re-exports
- UPDATE: `is_conditioned()` and `is_native_amount()` to remove Collateral/LendingDeposit arms
- UPDATE: RPC block.rs and balance.rs match arms (option (a): keep variants, mark tombstoned)

**Wire-format risk:** NONE. Three independent protections confirm no lending UTXOs exist: (1) `defi_activation_height = u64::MAX` since inception, (2) no apply_block code path creates these output types, (3) INC-I-088 Phase 0 freeze.

**LOC reduction:** ~3,237 lines

**Confidence:** conf(0.85, converged)

### B.2 NFT fractionalization (2 tx types + ~720 LOC + CLI)

**What:** FractionalizeNft=29, RedeemNft=30. Files: `validation/fractionalize.rs` (151), `cmd_nft/fractionalize.rs` (244), `cmd_nft/redeem.rs` (253), output.rs helpers/constants (~72).

**Convergence:** 3/5 evaluators independently agree on removal.
- Subtraction: conf(0.65, measured) -- full analysis, deferred Phase 3, no consumer
- Patterns: conf(0.55, observed) -- no L1 ships native fractionalization, composable via Multisig+MintAsset
- Radical: conf(0.55, inferred) -- remove all native DeFi

CONVERGENCE INDEPENDENCE CHECK:
- Subtraction based on: dead code analysis (LOC, callers, gate verification)
- Patterns based on: industry pattern matching (no precedent for L1 fractionalization)
- Radical based on: first-principles L1/L2 boundary
INDEPENDENT? YES -- three different evidence sources.

**Mechanism:**
- DELETE files: `validation/fractionalize.rs`, `cmd_nft/fractionalize.rs`, `cmd_nft/redeem.rs`
- TOMBSTONE discriminants: TxType 29, 30 return `None` from `from_u32`
- DELETE: output.rs helpers (`is_fractionalized()`, etc.) and constants (`FRAC_MARKER`, etc.)
- DELETE: validation match arms in `transaction.rs`

**Wire-format risk:** NONE. Same reasoning as B.1 -- gated at u64::MAX, never activated.

**LOC reduction:** ~720 lines

**Confidence:** conf(0.75, converged)

---

## C. ADD-NEXT -- Net-New L1 Primitives Justified

**None pass the SSF filter for this proposal cycle.**

The analyst's coverage matrix shows 6/10 report problems already addressed structurally. The remaining 4 (cross-chain laundering, yield contagion, key compromise, regulatory) are not L1-addressable or are adequately defended by absence (no stablecoin, no looping vault, no flash loans).

The Patterns evaluator proposed three future adoptions (DLC recipes, EUTXO batching, Sapio-style DSL), but all three were self-assessed as Phase 2+ and explicitly killed by their own kill tests for Phase 1 necessity. The analyst's acceptance criterion AC-8 (<=800 LOC new code for foundations) reinforces this constraint.

---

## D. PRE-ACTIVATION FIXES (mandatory before any DeFi un-gate)

These three fixes are BLOCKING. No `defi_activation_height`, `amm_activation_height`, or `oracle_activation_height` may be set to a real value until all three are resolved.

### D.1 Fix MintAsset issuer authorization (F1)

**Finding:** Failure Analyst, conf(0.65, observed)

**Location:** `crates/storage/src/utxo/set.rs:417` -- `UtxoSet::get_utxo` always returns `pubkey: None`. The issuer authorization check at `validation/utxo.rs:405` (`if let Some(ref pk) = genesis_utxo.pubkey`) is skipped when `pubkey` is `None`, meaning the issuer check is NEVER executed in production.

**Impact:** Any holder of a FungibleAsset UTXO (not just the original issuer) can submit a MintAsset tx using their UTXO as the first input. The spend authorization passes (they own it), structural validation passes (has inputs/outputs/correct types), and the issuer check is bypassed. Supply minting is bounded only by the `total_supply` cap set by the genesis issuer.

**Fix:** The `UtxoProvider` implementation must return the pubkey when available, or the issuer check must use `pubkey_hash` comparison instead of `pubkey` comparison. The structural validation at `tx_types.rs:617-638` should also be hardened (currently enforces 3/5 claimed rules).

**Blast radius if unfixed:** Per-asset (each FungibleAsset independently mintable), but cascades to DOLI via AMM pools containing the affected token.

**Evidence:** `set.rs:417` confirmed: `pubkey: None, // pay-to-pubkey-hash`. `utxo.rs:405` confirmed: `if let Some(ref pk) = genesis_utxo.pubkey {` -- skipped when None.

### D.2 Add checked_add in compute_swap (F2)

**Finding:** Failure Analyst, conf(0.60, observed)

**Location:** `crates/core/src/pool.rs:34` -- `let reserve_a_new = reserve_a + dx;` is unchecked u64 addition.

**Impact:** If `reserve_a` is near `u64::MAX` (possible with FungibleAsset tokens of extreme supply), the addition panics in debug mode or wraps in release mode. A crafted pool with extreme token supply can DoS producers during block assembly.

**Fix:** Replace with `reserve_a.checked_add(dx).ok_or(PoolError::Overflow)?` or equivalent. The validation path does NOT call `compute_swap` (it checks reserves directly from tx metadata), so this is primarily a block-production safety fix.

**Evidence:** `pool.rs:34` confirmed: `let reserve_a_new = reserve_a + dx; // full dx goes in (fee stays in pool)`.

### D.3 Add oracle sunset gradient + recovery path (F3)

**Finding:** Failure Analyst, conf(0.60, observed)

**Location:** `oracle/mod.rs:193` -- `SUNSET_THRESHOLD_BPS = 5500`. The sunset is a cliff: at 55.00% structural share the oracle is active; at 54.99% it HALTs permanently with no on-chain recovery. No warning, no cooldown, no partial degradation.

**Impact:** A small decrease in structural bond weight below 55% permanently disables the oracle until a binary upgrade is distributed to every node. Multi-hour to multi-day outage for all oracle-dependent features.

**Approved fix (user decision 2026-05-26): (a) + (b) combined — warning zone at 60% + symmetric recovery.**

**State machine (3 zones):**

| Structural share | State | Behavior |
|---|---|---|
| ≥ 60% (6000 bps) | HEALTHY | Oracle fully active. Aggregation proceeds. `getOracleStatus.health = "healthy"`. |
| 55–59.99% (5500–5999 bps) | WARNING | Oracle still aggregates and publishes prices. Metric/log emits `oracle_warning_active=true`. RPC reports `health: "warning"`. NO consensus rule change. |
| < 55% (< 5500 bps) | HALT | Oracle stops aggregating. Existing behavior. **NEW:** if share recovers to ≥ 55% within `ORACLE_RECOVERY_EPOCHS` epochs (= 4, ≈ 1 day at current epoch length), aggregation resumes automatically. If share stays below 55% for ≥ 4 epochs, HALT becomes permanent (current binary-upgrade path). |

**Constants added (`oracle/mod.rs`):**
```rust
pub const SUNSET_THRESHOLD_BPS: u64 = 5500;       // existing — HALT floor (unchanged)
pub const SUNSET_WARNING_BPS: u64 = 6000;         // NEW — warning zone start
pub const ORACLE_RECOVERY_EPOCHS: u64 = 4;        // NEW — auto-recovery window
```

**State transitions:**
- HEALTHY → WARNING: when `structural_share_bps()` first drops below 6000. Set `oracle_warning_since_epoch`.
- WARNING → HEALTHY: when share rises back ≥ 6000. Clear `oracle_warning_since_epoch`.
- WARNING → HALT: when share drops below 5500. Set `oracle_halt_since_epoch`. Stop publishing OraclePrice UTXOs.
- HALT → WARNING (NEW recovery path): when share rises back ≥ 5500 AND `(current_epoch - oracle_halt_since_epoch) < ORACLE_RECOVERY_EPOCHS`. Resume publishing.
- HALT permanent: when `(current_epoch - oracle_halt_since_epoch) >= ORACLE_RECOVERY_EPOCHS`. Binary upgrade required.

**RPC surface (`getOracleStatus`):**
- New field `health: "healthy" | "warning" | "halted_recoverable" | "halted_permanent"`
- Existing centralization disclosure §6 byte-equal-lock UNTOUCHED (drift-gate test continues to enforce)

**Consensus impact:**
- WARNING zone: zero consensus rule change. RPC/metric only. No activation height required.
- Recovery path: CHANGES consensus behavior (HALT is no longer terminal). Requires `oracle_activation_height` to gate the new state machine. Since `oracle_activation_height = u64::MAX`, the new behavior is dormant until activation — safe to ship as part of the same un-gate decision.

**Persistence:**
- `oracle_warning_since_epoch` and `oracle_halt_since_epoch` stored in chain state (NOT just an AtomicBool). Restart-safe per analyst's gap finding.

**Evidence:** `oracle/mod.rs:193` confirmed: `pub const SUNSET_THRESHOLD_BPS: u64 = 5500;`. No gradient or recovery logic exists today.

**Estimated LOC:** ~80 net new (2 constants + 2 state fields + state machine match + RPC field + 5–6 unit tests).

---

## E. DEFER

Items mentioned in evaluator reports that DOLI explicitly will NOT pursue at L1 in this cycle:

| Item | Evaluator | Why deferred |
|------|-----------|-------------|
| Concentrated liquidity (Uniswap v3/v4) | Patterns REJECT-2 | Newton-Raphson iteration is platform-dependent (C2 risk). UTXO model fragments positions. |
| Encrypted mempool (Shutter/Penumbra) | Patterns REJECT-1 | Requires threshold cryptography coordination. Single offline decryptor stalls mempool. |
| Automated liquidation engine | Patterns REJECT-3 | Requires oracle + keeper network + liquidation bonus. Escrow-loan forfeiture replaces. |
| Flash loans | Patterns REJECT-4 | Structurally impossible on UTXO chains. |
| Intent UTXO + bonded solver | Analyst | Agent-allowance template covers current use cases. Phase 2+. |
| Batch settlement / uniform clearing | Analyst, Patterns | Requires protocol changes. Phase 2+. |
| Sapio-style covenant DSL | Patterns | Ergonomic improvement, not capability expansion. 6 templates sufficient for Phase 1. |
| DLC composition recipe | Patterns | Documentation-only. Adaptor sig primitives exist. Phase 2+. |
| Restitution slash path | Analyst | 4/5 evaluators in foundations economics dropped permanently. |
| On-chain oracle set derivation | Patterns | Phase 2.3. Current hardcoded set acceptable at 12-34 producers. |

---

## F. SSF Alternative -- Radical Minimum

**Source:** Radical Simplifier evaluator

**Proposal:** DOLI needs exactly TWO L1 DeFi primitives: condition guards (already shipped) and ZKSettle (stub shipped). Everything else -- including AMM, oracle, MintAsset/BurnAsset -- is an L2 concern or stays as-is.

**Active DeFi surface:** Transfer (with conditions) + MintAsset/BurnAsset (already live) + ZKSettle. ~3,651 LOC (conditions 3,392 + ZK stub 259).

**Tombstoned:** ALL 13 DeFi tx types (4 AMM + 5 lending + 2 frac + 1 oracle + 1 ZKSettle stays). Plus Pool/LPShare output types.

**Arguments for:**
1. L1 AMM has a structural throughput ceiling: 1 swap/pool/block = 6/minute. L2 AMM via ZKSettle has no such ceiling.
2. 13 DeFi tx types with 0 active is the definition of dead code. The radical minimum tombstones ~3,544 LOC.
3. L2 builders can integrate any oracle, not limited to DOLI's 12-producer set.
4. User explicitly said "we don't want 1.5 billion primitives."

**Arguments against:**
1. **Temporal gap:** ZKSettle has no verifier wired. 3-6 months minimum before any L2 DeFi is possible. During this gap, DOLI has zero pooled DeFi capability.
2. **L2 builder ecosystem = 0.** No DOLI L2 builders exist. Native AMM serves as a bootstrapping signal.
3. **L1+DeFi atomicity:** Native AMM can be atomically composed with L1 operations in the same block. L2 AMM requires round-trip through L2 sequencer.
4. **Fee revenue:** The 25/5 fee split routes protocol fees to the producer reward pool. Radical minimum has no AMM fee revenue stream.

**Confidence:** conf(0.55, inferred)

**Confidence delta vs. main proposal:** -0.10 (within SSF threshold)

**SSF verdict:** The delta of 0.10 between the main proposal (conf 0.65 aggregate) and the radical minimum (conf 0.55) triggers the SSF gate. The User Gate must choose between:
- **Main proposal:** AMM + Oracle KEEP, lending/frac TOMBSTONE, 3 pre-activation fixes
- **Radical proposal:** Everything TOMBSTONE except conditions + ZKSettle, pre-activation fixes still mandatory for MintAsset/BurnAsset

---

## G. Invariants (Filters)

These invariants from the Failure Analyst and analyst MUST be respected by any implementation:

| ID | Invariant | Source |
|----|-----------|--------|
| INV-F-001 | UTXO singleton per pool -- only ONE swap per pool per block | `tx_processing.rs:145` |
| INV-F-002 | k-invariant rounding MUST favor pool (new_k >= old_k) | `pool.rs:30`, `validation/utxo.rs:586` |
| INV-F-003 | MINIMUM_LIQUIDITY lock enforced at validation, NOT apply_block | `validation/pool.rs:112-121` |
| INV-F-004 | Oracle sunset is currently a cliff (binary halt), not a gradient | `oracle.rs:109-116` |
| INV-F-005 | FungibleAsset supply relies on UTXO-level validation, not structural | `tx_types.rs:617-638` vs `utxo.rs:376-468` |
| INV-F-006 | Pool UTXO uniqueness enforced at apply_block, NOT validation | `tx_processing.rs:128-138` |
| INV-F-007 | ALL DeFi types gated at u64::MAX. MintAsset bug unexploitable while gate holds | `network_params/defaults.rs` |
| INV-F-008 | Token conservation in Swap relies on 3-part interplay | `validation/utxo.rs:200-207,583-632` |
| INV-F-009 | compute_swap has unchecked u64 addition at pool.rs:34 | `pool.rs:34` |
| INV-F-010 | MaxDeltaGuard is user-opt-in, NOT protocol-enforced | `conditions/eval.rs:166-191` |
| EI-11 | Pool UTXO singleton -- one per pool_id | INV-DEFI-010 |
| EI-12 | Fee fields immutable post-CreatePool | INV-DEFI-019 |
| EI-13 | K-invariant non-decreasing after every Swap | INV-DEFI-002 |

---

## H. Migration Sequencing

### Phase 0: Pre-Activation Fixes (BLOCKING)

Order: D.2 (compute_swap overflow) -> D.1 (MintAsset issuer) -> D.3 (oracle sunset gradient)

Rationale: D.2 is the smallest change (1 line). D.1 requires storage layer change + validation hardening. D.3 requires design decision on gradient vs. cooldown vs. recovery.

Tests required before each fix:
- D.2: Unit test proving panic with extreme reserves, then PASS with checked_add
- D.1: Integration test proving non-issuer MintAsset is rejected after fix
- D.3: Unit tests for gradient/recovery behavior under structural share changes

### Phase 1: Tombstoning (after Phase 0)

Order: B.1 lending -> B.2 NFT-frac

1. Remove lib.rs re-exports for lending math/types
2. Remove RPC dispatch entries + `lending.rs` file
3. Remove `get_all_collateral()` from 3 UTXO storage files
4. Remove `validation/lending.rs` file
5. Remove `lending.rs` file
6. Remove `cmd_loan.rs` file
7. Remove output.rs lending constructors/constants/structs
8. Tombstone TxType 24-28 in `from_u32` (return None + comment)
9. Tombstone OutputType 11, 12 in `from_u8` (return None + comment)
10. Remove `is_conditioned()` and `is_native_amount()` arms for Collateral/LendingDeposit
11. Remove INC-I-088 freeze guard at `utxo.rs:975-993`
12. Update `inc_i_088_phase0_defi_gate.rs` -- keep gate-level tests, remove lending-specific cases
13. Repeat equivalent steps for NFT-frac files
14. Tombstone TxType 29-30 in `from_u32`

Specs to update: `defi-subsystem-architecture.md`, `docs/rpc_reference.md`, `docs/cli.md`, `CLAUDE.md`

### Phase 2: Activation Decisions (separate sessions)

Each activation height decision is a separate session per HC-6 / INC-I-075:
- `amm_activation_height` -- requires Phase 0 fixes complete + AMM-specific audit
- `oracle_activation_height` -- requires D.3 sunset gradient fix + oracle-specific audit
- `guards_activation_height` -- prerequisite for escrow-loan on mainnet

### Open questions for tombstoning implementation

1. **Enum variants:** Keep with `#[deprecated]` attributes vs. just tombstone comments. Recommendation: `#[deprecated]` for compiler enforcement.
2. **INC-I-088 test file:** Rewrite as focused tombstone regression test.
3. **`defi_activation_height` field:** Keep as documentation -- removing a NetworkParams field is wire-format-adjacent.

---

## I. Acceptance Criteria -- Met / Not Met

### 8 Must-Decide Questions

| # | Question | Answer | Met? |
|---|----------|--------|------|
| 1 | Tombstone 5 native lending types + 2 output types? | YES. 4/5 convergence, conf(0.85, converged). | MET |
| 2 | Tombstone 2 NFT-frac types? | YES. 3/5 convergence, conf(0.75, converged). | MET |
| 3 | AMM protocol fee routing? | Per-pool field (fee_bps in pool_id). Already decided (D2, IRREVERSIBLE). | MET |
| 4 | Swap supply conservation for FungibleAsset? | 3-part interplay (INV-F-008). Plus D.2 overflow fix. | MET |
| 5 | MAX_CONDITION_DEPTH=4 / MAX_CONDITION_OPS=128 sufficient? | YES for Phase 1. All 6 templates depth <= 3. | MET |
| 6 | Oracle independence? | YES. Separate activation height, own spec, general-purpose. | MET |
| 7 | L1/L2 boundary for lending? | Pooled lending = L2. Bilateral lending = L1 escrow-loan. | MET |
| 8 | BridgeHTLC stays L1? | YES. No operator, tiny footprint. | MET |

### 5 Must-Quantify Metrics

| # | Metric | Target | Achieved |
|---|--------|--------|----------|
| 1 | Dead code ratio | < 20% | ~0% after tombstoning. EXCEEDED. |
| 2 | Activation surface | 1 height for Phase 1 AMM | 1 (`amm_activation_height`). MET. |
| 3 | Report problem coverage | >= 6/10 | 7/10 partial-or-stronger. MET. |
| 4 | Condition guard composability | >= 6 DeFi patterns | 6/6 (swap, bilateral loan, vault, subscription, agent delegation, cross-chain). MET. |
| 5 | Wire-format stability | 0 changes | 0. Tombstone-only. MET. |

---

## J. Open Questions for User

1. **SSF Gate:** Radical Simplifier proposes tombstoning AMM alongside lending/frac (delta = 0.10). Main proposal (KEEP AMM) or radical minimum (conditions + ZKSettle only)?

2. **Oracle sunset gradient (D.3):** (a) warning zone at 60%, (b) recovery if share rises, or (c) cooldown of 3 epochs?

3. **Tombstone style:** `#[deprecated]` attributes (compiler-enforced) or comments only?

4. **Devnet/testnet history:** Have Collateral or LendingDeposit UTXOs ever existed on ANY chain?

5. **`defi_activation_height` field:** Remove, keep as documentation, or repurpose?

---

## Design Synthesis Quality Gate

```
--- DESIGN SYNTHESIS QUALITY GATE ---
Evaluators completed:           5/5
Deletion convergence items:     2 (lending 4/5, frac 3/5)
Restructuring convergence:      3 (oracle placement, validation split, guard placement)
Addition options presented:     0 (none passed SSF filter)
Failure modes identified:       10 (INV-F-001 through INV-F-010)
Failure modes applied as filters: 10/10
Radical floor gap:              13 frozen types -> 0 (radical) -> 5 active (proposed)
Contradictions found:           2 (AMM KEEP vs radical REMOVE; oracle KEEP vs radical REMOVE)
Contradictions resolved:        2/2 (via SSF gate + confidence delta)
Evidence independence verified: YES (all convergence clusters checked)
-------------------------------------
```

**Overall proposal confidence:** conf(0.65, converged)
