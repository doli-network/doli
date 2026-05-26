<!--
OUTPUT CONTRACT: N/A — design specification file (not a test file)
INPUT PARTITIONS: N/A — design specification file (not a test file)
-->

# DeFi Subsystem Architecture

## Executive Summary

Ship AMM first, compose a bilateral **escrow-loan** template from existing covenant primitives, defer everything else. Phase 1 delivers 4 native TX types (CreatePool, AddLiquidity, RemoveLiquidity, Swap) with constant-product math behind a new `amm_activation_height`, plus a CLI escrow-loan template using existing guards (AmountGuard, RecipientGuard, Timelock). AMM fees split **25 bps to LPs + 5 bps to the DOLI producer reward pool** (W2 — economic alignment between DEX volume and validator security). No oracle. No liquidation engine. No new lending TX types. No NFT fractionalization TX types. The 7 remaining DeFi TX types (24-30) stay gated at `u64::MAX`. This reduces the audit surface from 11 TX types + oracle + liquidation engine (~6,000-10,000 LOC) to 4 TX types + 1 covenant template (~1,550 LOC), while delivering functional AMM + peer-to-peer escrow-loan on day one.

> **Naming convention (W3):** The Phase 1 covenant-composed loan product is called an **escrow-loan** (a.k.a. "OTC loan" / "bilateral escrow-loan"). The word "lending" is RESERVED for the Phase 2 pooled-lending product (with oracle + liquidation engine) that is NOT shipped in Phase 1. Calling the bilateral product "lending" sets user expectations the protocol cannot meet — Bitcoin has had CLTV+multisig bilateral escrow since 2012 and produced no commercial lending market for exactly this reason. Where this spec describes legacy code paths or Phase 2 work, "lending" retains its original meaning.

---

## The 6 Design Decisions

### D1: Oracle Architecture -- SUBTRACTED (Phase 1)

**Chosen answer:** No oracle in Phase 1. AMM TWAP accumulator remains in Pool metadata for informational queries (getPoolPrice) but is NOT wired into any consensus-critical lending or liquidation path. Overcollateralized lending via covenant composition eliminates oracle dependency entirely.

**Trade-off:** Lenders bear all price risk. No automated price-triggered liquidation. Collateral-to-loan ratio is fixed at creation and agreed bilaterally. This is structurally identical to Bitcoin's HTLC pattern -- trustless, simple, oracle-free.

**Phase 2 sketch (if composed lending proves insufficient):** Producer-attested PriceUpdate TX using existing BLS infrastructure. Each producer posts signed price observations; block producer aggregates via bond-weighted median; stored in PriceOracle UTXO. This matches Band Protocol's Cosmos validator-attested model, reusing DOLI's existing producer trust model and BLS attestation aggregation infrastructure.

**Convergence:** Subtractionist (conf 0.60) and Radical Simplifier (conf 0.65) converge on subtraction. Pattern Matcher (conf 0.65) and Restructurer (conf 0.55) recommend option (a) PriceUpdate TX for full-slate scenarios. Failure Analyst accepts (a) with mitigation but rejects (c). Resolution: Tier 2 subtracts the oracle; if Phase 2 is triggered, option (a) is the pre-selected path.

**Rejected options:** (b) AMM TWAP as oracle -- bootstrap problem (no pools = no prices), trivially manipulable on low-liquidity pools (Compound v1 lesson). (c) Liquidator-provided proof -- condition language cannot verify arbitrary external signatures (F3 filter). (d) Hybrid -- highest complexity for uncertain benefit.

### D2: Liquidation Model -- SUBTRACTED (Phase 1)

**Chosen answer:** Liquidation is timelock-based forfeiture. The collateral condition includes `And(Signature(lender), Timelock(deadline_height))` as the default path. After the deadline, the lender claims collateral directly. No bonus, no keeper network, no MEV, no oracle dependency.

**Trade-off:** Binary outcome only (repay or forfeit). No partial liquidation. No price-triggered protection for lenders during the loan term. Lenders mitigate by requiring higher overcollateralization ratios.

**Phase 2 path:** Permissionless + fixed bonus (Aave v3 pattern, 6+ years battle-tested). Requires oracle (D1) to be solved first.

**Convergence:** Subtractionist (conf 0.60) and Radical (conf 0.65) converge on subtraction. Pattern Matcher (conf 0.60) and Restructurer (conf 0.50) recommend option (a) for full-slate. Failure Analyst rejects (b) keeper-restricted (34 producers too small, INC-I-016 analog) and (c) auction (no infrastructure, 10s slots too slow). Resolution: subtraction for Phase 1; option (a) pre-selected for Phase 2.

**Rejected options:** (b) Keeper-restricted -- 34 producers too small; excluded-producer cascade (INC-I-016) starves liquidation. (c) Auction -- no auction infrastructure, 10s slots impractical for price discovery.

### D3: Collateral Spending Model -- (a) Condition-based

**Chosen answer:** Collateral UTXOs use the existing condition/guard system. The loan condition tree is:

```
Or(
  And(AmountGuard(principal + interest, 0), RecipientGuard(lender_pkh, 0)),
  And(Signature(lender_pubkey), Timelock(deadline_height))
)
```

Depth = 2 (Or -> And -> leaf). Ops = 6. Well within MAX_CONDITION_DEPTH=4 and MAX_CONDITION_OPS=128.

**Witness format:** Borrower repayment path requires no witness data beyond standard signature -- AmountGuard and RecipientGuard evaluate against `ctx.tx.outputs` (the spending transaction's outputs). The spending TX must include an output paying >= `principal + interest` to `lender_pkh` at index 0.

**CLI flow:**
```
doli template escrow-loan \
  --lender <lender_address> \
  --repay-amount <principal_plus_interest> \
  --deadline <block_height> \
  > condition.json

doli send --to <borrower_address> --amount <collateral_amount> \
  --condition condition.json
```

> The subcommand is **`escrow-loan`** (not `loan` or `lending`). This is the W3 naming fix: the bilateral covenant product is a programmable escrow for a single OTC loan, not a lending market. The CLI reflects that to set correct user expectations.

**Trade-off:** Conditions are fixed at UTXO creation. Cannot update loan terms mid-flight. Cannot enforce collateralization ratio at creation via conditions alone (conditions evaluate at spend time). Mitigation: CLI template enforces ratio at construction time; validation can structurally reject Transfer TXs that create collateral with insufficient ratio.

**Convergence:** STRONG -- 5/5 evaluators converge on condition-based (a). Subtractionist (0.65), Restructurer (0.65), Pattern Matcher (0.65), Failure Analyst (accept with mitigation), Radical (0.65). Independent evidence: Subtractionist cites guard SDK maturity (b04a7e83); Restructurer cites analyst Section 3.3; Pattern Matcher cites Cardano eUTXO analog; Radical cites condition depth analysis. TRUE CONVERGENCE.

**Evidence:** `is_conditioned()` in `transaction/types.rs` already includes `Collateral` (INC-I-088 Phase 0). Guard evaluation infrastructure at `conditions/eval.rs:121-165` handles AmountGuard/RecipientGuard composition. Covenant templates (b04a7e83) provide the pattern.

**Rejected options:** (b) State-machine -- current implementation is DEF-1 2-check shell; full state machine adds C2 risk on every transition. (c) Multisig -- deadlock if either party offline; extortion vector.

### D4: AMM Curve Strategy -- (a) Constant-product only

**Chosen answer:** `x * y = k` (Uniswap v2). Integer-only arithmetic with u128 intermediate products on u64 reserves. **Total fee = 30 bps (0.3%), split as 25 bps to LPs + 5 bps to the DOLI producer reward pool** (see Fee Split spec below — W2 economic alignment requirement).

**Swap formula (deterministic):**
```
output = (input_amount * 9970 * reserve_out) / (reserve_in * 10000 + input_amount * 9970)
```

All operations in u128. Truncation toward zero (Rust default for integer division). k must be non-decreasing after every Swap (fees increase k). The 30 bps reduction (`9970/10000`) is the TOTAL economic fee taken from the swap. The 25/5 LP/protocol split is applied at apply_block when the Pool UTXO is updated — see Fee Split spec below.

#### Fee Split (W2 — economic alignment between DEX volume and DOLI security)

**Problem:** A 100%-to-LP fee model (Uniswap v2 default) makes DEX success orthogonal to chain security. Post-halving (Era 1+) the block subsidy decreases; if DEX volume grows but routes zero value to bonded producers, **DOLI security shrinks while DOLI utility grows** — the inverse alignment. LPs and producers also compete for the same DOLI capital — without a protocol fee, LPs strictly dominate, hollowing out the bond stack (Cosmos/Osmosis problem). Fixing post-activation is governance-political poison (Uniswap fought 5 years to flip the fee switch). Ship correct at activation.

**Context — DOLI's broader value model:** The AMM protocol fee (this W2 fix) is the *first* DOLI inflow stream beyond block subsidy. On the outflow side, DOLI already has a **deflationary backstop on producer misbehavior**: 100% of slashed bonds and 100% of early-unbond vesting penalties are BURNED (sources: `consensus/exit.rs:100-118` and `:130-147`). The W2 fee builds the missing *productive* inflow to complement the *punitive* deflation. See `specs/tokenomics.md` for the full value-flow model and Era-by-era security budget projection.

**Pool metadata gains two named fee fields** (replacing the implicit single 30 bps):

```rust
struct PoolMetadata {
    // ... existing reserves + TWAP fields ...
    fee_bps_to_lp: u16,         // default 25 (locked at CreatePool)
    fee_bps_to_protocol: u16,   // default 5  (locked at CreatePool)
    // ... existing fields ...
}
// Invariant: fee_bps_to_lp + fee_bps_to_protocol == TOTAL_FEE_BPS (30)
// Invariant: both fields are immutable after CreatePool
```

Immutability rationale: no governance-mediated rate change avoids reflexive sell pressure and removes a governance-attack surface. Per-pool ratios at creation are permitted (e.g., a stable pool could choose 28/2) — acceptable because CreatePool is permissionless and pools compete on fee.

**Routing at apply_block (Swap path):**
1. Compute `protocol_fee = (input_amount * fee_bps_to_protocol) / 10000` using u128
2. Apply the existing 30 bps swap output formula
3. Subtract `protocol_fee` from the input-asset reserve in the new Pool UTXO (it leaves the pool)
4. Mint a new Coin UTXO of value `protocol_fee` (denominated in the input asset) paying to the canonical DOLI reward pool address: `BLAKE3("REWARD_POOL"||"doli")` — same address used by existing epoch coinbase routing
5. K-invariant check applies to the post-fee reserves; LP K-increase corresponds to the 25 bps LP share

**Economic math (per pool):**
- 30 bps × $1M daily volume = ~$1.1M/year total fee
- 25/5 split: $912K/year LPs + $182K/year producer pool
- At $10M/day/pool: $1.82M/year producer pool — meaningful vs Era 1+ subsidy
- LP yield (25 bps) remains competitive with Uniswap v3 30 bps tier when adjusted for DOLI AMM's lack of concentrated liquidity (wider deployment per dollar = comparable APY)

**Constraint compliance:**
- **C2 (bit-identical rebuild):** `protocol_fee` is pure integer math; deterministic across all nodes
- **C5 (block content):** adding a fee-routing Coin output to Swap is a block-content change → already covered by `amm_activation_height` gate (no NEW gate required since Swap itself is gated by it)
- **C7 (3-question checklist):** (1) user TX triggers — YES; (2) producer mempool/inclusion triggers — YES; (3) bit-identical to old behavior — NO → activation height required, satisfied by `amm_activation_height`

**LOC estimate:** ~50 lines new code (Pool metadata fields + Swap apply_block routing + CreatePool sum-invariant check) + ~80 lines tests (proportionality, sum-invariant, edge cases at 0 bps protocol, max bps protocol, sum-mismatch rejection).

**LP share formula:**
- Initial: `shares = sqrt(reserve_a * reserve_b) - MINIMUM_LIQUIDITY` (MINIMUM_LIQUIDITY locked forever to prevent first-deposit manipulation)
- Subsequent: `shares = min(amount_a * total_lp / reserve_a, amount_b * total_lp / reserve_b)`

**Trade-off:** High slippage for stablecoin pairs. No concentrated liquidity. Acceptable for launch; novel curves belong on L2 via ZKSettle or behind a future activation height.

**Convergence:** UNANIMOUS -- 5/5 evaluators choose (a). All cite determinism (C2), battle-testing (6+ years Uniswap v2), and integer-only feasibility. Independent evidence paths: Pattern Matcher cites Curve exploit 2023; Failure Analyst cites C2 risk of Newton-Raphson; Radical cites LOC savings.

**Evidence:** `pool.rs` already implements constant-product (7 functions: compute_swap, compute_initial_lp_shares, compute_lp_shares, compute_remove_liquidity, update_twap, compute_twap_price, verify_invariant). Protocol spec confirms 30 bps fee and 116-byte Pool metadata.

**Rejected options:** (b) Stableswap -- Newton-Raphson iteration is platform-dependent; C2 violation risk (F2 filter rejects). (c) Pluggable registry -- speculative generality anti-pattern; each curve requires its own activation height; zero proven demand.

### D5: NFT Redemption Model -- NOT SHIPPED (Phase 1)

**Chosen answer:** FractionalizeNft (29) and RedeemNft (30) stay gated at `u64::MAX`. NFT fractionalization is composable using existing primitives for up to 127 shareholders:
1. Lock NFT in Multisig(N, [shareholder_keys]) conditioned output
2. MintAsset (type 17, already live) to create fungible shares with `asset_id = fraction_asset_id(nft_token_id)`
3. Redeem: BurnAsset (type 18, already live) all shares + Multisig spend releases NFT

**Trade-off:** Capped at 127 shareholders (MAX_MULTISIG_KEYS=127). No share-buyback mechanism. No forced auction. Sufficient for small-group fractionalization (business partnerships, collector groups). >127 shareholders requires Phase 3 with dedicated TX types.

**Convergence:** Subtractionist (0.50), Radical (0.55), and Failure Analyst (DEF-4: no spec = do not activate) converge on deferral. Pattern Matcher and Restructurer propose share-buyback (a) for eventual shipping. Resolution: defer in Phase 1; pre-select (a) share-buyback for Phase 3.

**Rejected options (for eventual Phase 3):** (b) Forced auction -- no auction infrastructure. (c) Time-locked unanimity -- MAX_THRESHOLD_CONDITIONS=5 caps at 5 shareholders; deadlock with single dissenter. (d) Fractional governance -- massive complexity, vote-buying attacks.

### D6: Activation Sequencing -- (b) Per-primitive heights

**Chosen answer:** Each DeFi primitive gets its own activation height in NetworkParams:
- `amm_activation_height: u64` -- Phase 1 (defaults `u64::MAX`, lowered when AMM is audit-ready)
- `lending_activation_height: u64` -- Phase 2 (defaults `u64::MAX`, lowered if/when native lending ships)
- `nft_frac_activation_height: u64` -- Phase 3 (defaults `u64::MAX`, lowered if/when NFT-frac ships)
- Existing `defi_activation_height` stays at `u64::MAX` as backstop for any types not covered by specific heights

**Prerequisite:** `guards_activation_height` MUST be activated on mainnet before `amm_activation_height`. Tier 2 lending composition depends on AmountGuard/RecipientGuard which are gated behind `guards_activation_height`. Verified: `guards_activation_height` exists in `consensus/params.rs:159` and is checked in `validation/transaction.rs:396`.

**Trade-off:** 3 new NetworkParams fields instead of reusing 1. Negligible cost -- NetworkParams already has 40+ fields and DOLI has 11+ existing activation heights.

**Convergence:** UNANIMOUS -- 5/5 evaluators choose (b). All cite blast-radius isolation (one bug delays only that primitive) and DOLI's established pattern (11 existing activation heights). Independent evidence: Pattern Matcher cites Bitcoin BIP9/BIP8 pattern; Failure Analyst cites INC-I-054 analog risk of coupled activations; Subtractionist cites activation sequence decoupling.

**Rejected option:** (a) Single height -- one bug in any of 11 TX types delays ALL DeFi; Ethereum bundled-fork anti-pattern (Shanghai delayed by EIP-4844).

---

## Concrete Defect Resolution

### DEF-1: LiquidateLoan is a 2-check shell

**Resolution:** SUBTRACTED in Phase 1. LiquidateLoan (TX type 26) stays gated at `u64::MAX`. Liquidation in Phase 1 is timelock-based forfeiture via the collateral condition's default path: `And(Signature(lender), Timelock(deadline_height))`. No new validation code needed -- the existing condition evaluator handles it.

**Phase 2 resolution:** If native lending ships, implement full LiquidateLoan with oracle price verification, LTV calculation, and fixed liquidation bonus. Estimated 80-120 lines of new validation code + oracle dependency.

### DEF-2: Zero oracle code

**Resolution:** SUBTRACTED in Phase 1. No oracle needed for composed lending (timelock-based default). AMM TWAP accumulator stays in Pool metadata for informational queries only.

**Phase 2 resolution:** Producer-attested PriceUpdate TX (D1a). Estimated 500-1,000 LOC across validation, consensus, RPC.

### DEF-3: Zero AMM tests -- MUST FIX

**Resolution:** Write comprehensive test suite for all 4 AMM TX types before activation.

**Property tests:**
- K-invariant: `reserve_a_after * reserve_b_after >= reserve_a_before * reserve_b_before` for every Swap
- LP share conservation: `sum(all_LP_share_UTXOs_for_pool) == pool.total_lp_shares` after every block
- Total reserves conservation: AddLiquidity increases both reserves; RemoveLiquidity decreases proportionally
- Commutativity: swap(A->B, x) then swap(B->A, y) leaves pool in deterministic state

**Adversarial tests:**
- Zero-amount: CreatePool with 0 reserve, Swap with 0 input, AddLiquidity with 0 of one asset
- Overflow: reserves near u64::MAX, verify u128 intermediates prevent panic
- Dust: tiny swaps that round to 0 output (must be rejected)
- Rounding theft: sequence of small deposits/withdrawals to steal rounding errors
- First-deposit donation attack: attacker creates pool with extreme ratio then sandwiches
- Sandwich attack: verify UTXO contention makes within-block sandwich impossible
- Pool drain: RemoveLiquidity of all shares (MINIMUM_LIQUIDITY prevents full drain)

**E2E on testnet:**
- CreatePool -> AddLiquidity -> Swap -> Swap (reverse) -> RemoveLiquidity -> verify all UTXOs consistent
- Multiple pools for different asset pairs
- Pool UTXO contention: submit 2 Swaps for same pool in same block, verify exactly 1 succeeds

### DEF-4: Zero NFT fractionalization tests + no spec

**Resolution:** SUBTRACTED in Phase 1. FractionalizeNft and RedeemNft stay at `u64::MAX`. No new tests needed for Phase 1. Document the composition recipe (Multisig + MintAsset + BurnAsset) in CLI docs.

### DEF-5: State-machine vs condition tension

**Resolution:** RESOLVED by design choice. AMM uses state in Pool UTXO metadata (necessary for shared mutable pool state -- conditions cannot express this). Lending uses conditions (sufficient for peer-to-peer fixed-term loans). The tension is eliminated by assigning each primitive to the mechanism that fits its requirements. Collateral and LendingDeposit output types (11, 12) are NOT USED in Phase 1.

---

## Activation Sequencing Plan

### New NetworkParams fields

```rust
// Phase 1
pub amm_activation_height: u64,         // defaults u64::MAX

// Phase 2 (future, separate redesign)
pub lending_activation_height: u64,      // defaults u64::MAX

// Phase 3 (future, separate redesign)
pub nft_frac_activation_height: u64,     // defaults u64::MAX
```

### Activation sequence

1. **Prerequisite: `guards_activation_height`** must be lowered to a real mainnet height FIRST. Currently gated (verified in `consensus/params.rs:159`). This enables AmountGuard/RecipientGuard for composed lending. Can share the same activation block as AMM or activate earlier.

2. **Phase 1: `amm_activation_height`** -- when set to a real future height, enables CreatePool (19), AddLiquidity (20), RemoveLiquidity (21), Swap (22). The 7 lending/NFT TX types (24-30) remain gated by `defi_activation_height` = `u64::MAX`.

3. **Escrow-loan template** -- available immediately after `guards_activation_height` activates. Uses standard Transfer TX (type 0) + condition templates. No activation height needed. (W3: this is escrow-loan, not lending.)

4. **`defi_activation_height`** -- stays at `u64::MAX`. INC-I-088 Phase 0 gate remains active for types 24-30. Does NOT activate in Phase 1.

5. **`CURRENT_PROTOCOL_VERSION`** -- NOT bumped. No EpochState format change.

### What stays unchanged

- All existing crossed activation heights: IMMUTABLE (C1)
- INC-I-088 Phase 0 gate: stays as-is
- `[ERRTX-DEFI001]` hard-reject: stays active for types 24-30
- `OutputType::Collateral` in `is_conditioned()`: stays as-is

---

## Oracle Architecture Spec

**Phase 1:** None. No oracle code. AMM TWAP in Pool metadata is informational only (getPoolPrice RPC).

**Phase 2 sketch (producer-attested model):**

```
PriceUpdate TX (new type, e.g., 32):
  - Producer submits signed price observation for a token pair
  - Block producer aggregates observations from current epoch's producers
  - Aggregation: bond-weighted median (same trust model as block production)
  - Stored in PriceOracle UTXO (or extend Pool metadata)
  - Staleness TTL: configurable in NetworkParams (e.g., 100 blocks = ~17 minutes)
  - Manipulation resistance: BLS aggregate signature from 2/3+ of active producers
  - Slashing: conflicting price attestations submitted as SlashingEvidence (reuses type 5)
  - Module location: crates/core/src/defi/oracle.rs (per Restructurer option gamma)
```

This sketch is NOT implemented in Phase 1. It is preserved here as the pre-selected architecture if Phase 2 is triggered.

---

## Collateral Spending Model Spec

### Condition tree (Phase 1 escrow-loan)

```
Or(
  // Path A: Borrower repays
  And(
    AmountGuard(principal_plus_interest, 0),  // TX output[0] >= repay amount
    RecipientGuard(lender_pubkey_hash, 0)     // TX output[0] pays lender
  ),
  // Path B: Lender claims after deadline
  And(
    Signature(lender_pubkey),                 // Lender's Ed25519 signature
    Timelock(deadline_height)                 // Cannot spend before deadline
  )
)
```

### Witness format

- **Path A (repayment):** Borrower signs the spending TX. The TX must include output[0] with `amount >= principal_plus_interest` and `recipient = lender_pubkey_hash`. No additional witness data beyond standard signature. AmountGuard and RecipientGuard evaluate against `ctx.tx.outputs[0]`.

- **Path B (default):** Lender signs after `deadline_height`. Timelock checks `ctx.block_height >= deadline_height`. Lender receives the collateral UTXO directly.

### CLI flow

```bash
# 1. Lender and borrower agree on terms off-chain

# 2. Borrower creates collateral UTXO with escrow-loan condition
doli template escrow-loan \
  --lender <lender_addr> \
  --repay-amount 1050  \
  --deadline 500000 \
  > escrow_condition.json

doli send \
  --to <borrower_addr> \
  --amount 2000 \
  --condition escrow_condition.json

# 3. Lender sends principal to borrower (standard Transfer)
doli send --to <borrower_addr> --amount 1000

# 4a. Borrower repays (before deadline)
doli send \
  --input <collateral_utxo_id> \
  --to <lender_addr> --amount 1050 \
  --to <borrower_addr> --amount 950

# 4b. Lender claims (after deadline, borrower defaulted)
doli send \
  --input <collateral_utxo_id> \
  --to <lender_addr> --amount 2000
```

**Limitation (W3):** This is **escrow-loan**, NOT lending. Each instance is bilateral (one lender, one borrower, found off-chain). No interest-rate market. No capital aggregation. No price-triggered protection. Bitcoin has had this exact pattern (CLTV + multisig) since 2012 and produced no commercial lending market — search costs, capital efficiency, and price discovery are the binding constraints, not crypto. Do not market this as "lending." For Aave-style pooled lending with oracle-driven liquidation, Phase 2 is required.

---

## Liquidation Flow Diagram

### Phase 1: Timelock-based forfeiture

```
CreateLoan (composed as Transfer + condition)
  |
  v
Collateral UTXO locked with Or(repay_path, default_path)
  |
  +---> Borrower repays before deadline?
  |       YES: borrower spends collateral via Path A
  |             (AmountGuard + RecipientGuard satisfied)
  |             Lender receives principal + interest
  |             Borrower receives collateral remainder
  |
  +---> Deadline passes without repayment?
          YES: Lender claims collateral via Path B
               (Signature + Timelock satisfied)
               Lender receives full collateral
               No oracle. No bonus. No partial liquidation.
```

### Phase 2: Permissionless + bonus (future)

```
Oracle posts price -> LTV drops below threshold
  |
  v
Anyone submits LiquidateLoan TX
  |
  v
Validation: oracle price check + LTV calculation + bonus computation
  |
  v
Liquidator receives: collateral * (1 + bonus_bps/10000)
Borrower receives: remaining collateral (if any)
Loan closed.
```

---

## Risk Register

| ID | Risk | Likelihood | Impact | Mitigation | Source |
|----|------|-----------|--------|------------|--------|
| R1 | AMM arithmetic diverges across nodes (C2) | MEDIUM | CRITICAL (chain fork) | u128 intermediates, integer-only math, deterministic rounding, comprehensive property tests (DEF-3) | Failure: INV-DEFI-001, INV-DEFI-016 |
| R2 | Pool UTXO contention limits throughput | HIGH | MEDIUM (1 op/pool/block) | Document as inherent UTXO property. Block builder prioritizes by fee. Mempool warns on collision. | Failure: F5; Pattern: Cardano SundaeSwap analog |
| R3 | Producer MEV on AMM swap ordering | HIGH | MEDIUM | UTXO contention prevents **intra-block atomic** sandwich (1 swap/block). **Cross-slot sandwich and producer-driven reordering remain extractable** (Economist W6). Document honestly. Monitor MEV leakage post-activation. | Failure: attack surface map; Economist: W6 |
| R4 | Escrow-loan terms are rigid (no mid-term adjustment) | MEDIUM | LOW | Acceptable for Phase 1 — bilateral agreement fixed at creation. W3: this is escrow-loan, not lending; don't market as lending. | Subtractionist: Target 4 risk; Economist: W3 |
| R5 | `guards_activation_height` not activated on mainnet | HIGH | BLOCKING | Must be lowered before AMM launch. Add to prerequisite checklist. | Radical: constraint 2 |
| R6 | LPShare (10) NOT in `is_conditioned()` | CONFIRMED | HIGH | MUST add LPShare to `is_conditioned()` before AMM activation. Without this, LP tokens are freely spendable without condition evaluation. | Failure: F10; Verified: `types.rs` |
| R7 | LendingDeposit (12) NOT in `is_conditioned()` | CONFIRMED | LOW (Phase 1) | Not used in Phase 1. Must be added before Phase 2 lending activation. | Failure: F10 |
| R8 | Pool metadata format (116 bytes) becomes immutable once activated | MEDIUM | HIGH | Design carefully before activation. Review format with external auditor. | Radical: constraint 5 |
| R9 | First-use trigger (INC-I-075 analog) | HIGH | HIGH | Exhaustive E2E testing on testnet for every AMM TX type before lowering gate. | Failure: INV-DEFI-015 |
| R10 | Duplicate pool creation | MEDIUM | HIGH | CreatePool validation must check UTXO set for existing pool with same pool_id. | Failure: attack map |
| R11 | Fee-split sum mismatch (W2) | LOW | CRITICAL (consensus split) | CreatePool MUST reject any pool where `fee_bps_to_lp + fee_bps_to_protocol != 30`. Both fields immutable after CreatePool. Property test the invariant. | Economist: W2; INV-DEFI-013, INV-DEFI-019 |
| R12 | LP-vs-bond capital substitution (W8) | MEDIUM | HIGH (Era 1+) | Without W2 fee-split, LP yield strictly dominates bond yield, hollowing out validator security. W2 partly mitigates by routing 5/30 to reward pool. Full fix requires LP-as-bond design (Phase 2+). Document trilemma in `specs/tokenomics.md`. | Economist: W8 (Osmosis analog) |
| R13 | Tokenomics document missing (W7 from economist) | CONFIRMED | HIGH | No `specs/tokenomics.md` exists; words "tokenomics", "protocol fee", "value capture" appear zero times across DeFi design artifacts. MUST publish before lowering `amm_activation_height`. | Economist: methodological note |

---

## Architecture Maps

### Current Architecture (Phase 0 -- all gated)

```
crates/core/src/
  pool.rs              -- AMM types + math (7 functions), root level
  lending.rs           -- Lending types + interest math (7 functions), root level
  nft.rs               -- NFT types, root level
  validation/
    pool.rs            -- AMM validation (CreatePool, AddLiq, RemoveLiq, Swap)
    lending.rs         -- Lending validation (2-check shell for LiquidateLoan)
    fractionalize.rs   -- NFT-frac validation (minimal)
    transaction.rs     -- Dispatch + DeFi gate [ERRTX-DEFI001]
  transaction/types.rs -- OutputType enum (Pool=9, LPShare=10, Collateral=11, LendingDeposit=12)
  conditions/          -- Guard evaluation (AmountGuard, RecipientGuard, Timelock, etc.)
  network_params/      -- defi_activation_height = u64::MAX

bins/node/src/node/apply_block/
  tx_processing.rs     -- DeFi match arms (gated, never reached)
  state_update.rs      -- DeFi UTXO mutations (gated, never reached)

Status: ALL 11 DeFi TX types hard-rejected. Zero live UTXOs.
```

### Proposed Architecture (Phase 1 -- AMM active)

```
crates/core/src/
  defi/                -- NEW subdirectory (module, not crate)
    mod.rs             -- DeFi module root, re-exports
    amm.rs             -- AMM types + math (extracted from pool.rs)
    lending.rs         -- DELETED (B.1 tombstoned 2026-05-26)
    nft.rs             -- TOMBSTONED (gated, "not implemented in Phase 1")
  validation/
    pool.rs            -- AMM validation (EXPANDED: full economic checks)
    lending.rs         -- DELETED (B.1 tombstoned 2026-05-26)
    fractionalize.rs   -- DELETED (B.2 tombstoned 2026-05-26)
    transaction.rs     -- Dispatch + DeFi gate + amm_activation_height check
  conditions/
    templates.rs       -- ADD: overcollateralized_loan() template function
  transaction/types.rs -- LPShare added to is_conditioned()
  network_params/      -- ADD: amm_activation_height = u64::MAX

bins/node/src/node/apply_block/
  tx_processing.rs     -- AMM match arms ACTIVE; lending/NFT arms gated
  state_update.rs      -- AMM UTXO mutations ACTIVE

bins/cli/src/
  cmd_pool.rs          -- Pool CLI commands (existing, verified working)
  cmd_template.rs      -- ADD: escrow-loan template subcommand (W3)

Status: 4 AMM TX types active (with 25/5 fee split — W2). 5 lending types TOMBSTONED (B.1). 2 NFT-frac types TOMBSTONED (B.2). Escrow-loan template via Transfer + conditions (W3 — NOT lending).
```

---

## Migration Path

**Greenfield confirmed.** `defi_activation_height = u64::MAX` on both mainnet and testnet. Zero DeFi UTXOs exist anywhere. No migration of existing state required.

**Verification steps before activation:**
1. Confirm zero DeFi UTXOs: `getPoolList` returns empty, `getLoanList` returns empty
2. Confirm `guards_activation_height` is active on target network
3. Confirm LPShare added to `is_conditioned()`
4. Run full AMM test suite on testnet
5. External audit complete
6. Lower `amm_activation_height` via ProtocolActivation TX

---

## Module Restructure

Move DeFi business logic into `crates/core/src/defi/` subdirectory (module within existing crate, NOT a new crate -- avoids circular dependency risk):

| File | Action |
|------|--------|
| `crates/core/src/pool.rs` | Move to `crates/core/src/defi/amm.rs` |
| `crates/core/src/lending.rs` | Move to `crates/core/src/defi/lending.rs` (tombstone) |
| `crates/core/src/nft.rs` | Move to `crates/core/src/defi/nft.rs` (tombstone) |
| `crates/core/src/defi/mod.rs` | NEW: module root with re-exports |

Import path changes: ~20-40 fixes across validation/, apply_block/, rpc/, cli/. Safe refactor -- no logic changes.

---

## Complexity Comparison

| Metric | Current (gated) | Radical Minimum (Tier 2) | Proposed (Phase 1) |
|--------|-----------------|--------------------------|---------------------|
| DeFi TX types active | 0 | 4 | 4 |
| New output types needed | 0 | 2 (Pool, LPShare) | 2 |
| Validation LOC (new) | 0 | ~600 | ~600 |
| Oracle code | 0 | 0 | 0 |
| Liquidation engine | 0 | 0 | 0 |
| Template code | 0 | ~50 | ~50 |
| Fee-split routing (W2) | 0 | ~50 | ~50 |
| Test LOC (new) | 0 | ~500 | ~580 (+80 for fee-split) |
| CLI additions | 0 | ~150 | ~150 |
| Total new LOC | 0 | ~1,500 | ~1,630 |
| Activation heights (new) | 0 | 1 | 1 (+2 reserved) |
| Design decisions needed | 6 | 1 (D4 only) | 1 (D4 only) |
| External audit weeks | 0 | 3-5 | 3-5 |
| Time to production | N/A | 4-8 weeks | 4-8 weeks |

**Note:** "Proposed" equals "Radical Minimum" because the SSF tiebreaker applies. Radical confidence (0.65) and full-slate confidence (0.40-0.70) are within the 0.1 SSF threshold. Simpler wins.

---

## Estimated Effort

### Phase 1: AMM-First DeFi (4-8 weeks)

| Task | Weeks | Notes |
|------|-------|-------|
| Module restructure (defi/ subdirectory) | 0.5 | Safe refactor, import fixes |
| AMM validation hardening | 1.5 | Expand pool.rs validation, u128 math, edge cases |
| LPShare `is_conditioned()` fix | 0.5 | Add to match, verify no regressions |
| `amm_activation_height` + NetworkParams | 0.5 | New field, gate logic, triplicate enforcement |
| **Pool metadata fee-split fields + apply_block routing (W2)** | **0.5** | `fee_bps_to_lp` / `fee_bps_to_protocol`, sum-invariant, reward-pool Coin mint |
| Escrow-loan covenant template (W3) | 0.5 | ~50 LOC + `doli template escrow-loan` CLI subcommand |
| AMM test suite (DEF-3) | 2.0 | Property + adversarial + E2E (incl. fee-split tests) |
| Pool UTXO contention documentation | 0.5 | Mempool warning, block builder priority |
| Testnet E2E validation | 1.0 | Full lifecycle on local testnet (incl. fee accrual to reward pool) |
| External audit prep | 1.0 | Documentation, code freeze, audit package |
| **Total** | **8.5** | Conservative estimate; 5 weeks optimistic |

### Phase 2: Native Lending (separate redesign, 8-16 weeks)

Triggered only if the escrow-loan template proves insufficient for the lending market. Requires separate `/omega-redesign --scope=lending`. (W3: "lending" properly refers to this Phase 2 pooled-lending product, not the Phase 1 escrow-loan.)

### Phase 3: NFT Fractionalization (separate redesign, 4-8 weeks)

Triggered only if demand exceeds 127-shareholder composition limit.

---

## Audit-Readiness Checklist

- [ ] All AMM math uses u128 intermediates (no floating point, no checked_mul panic paths)
- [ ] K-invariant property test passes for 10,000+ random swap sequences
- [ ] LP share conservation verified via state root assertions
- [ ] Pool UTXO singleton enforced (no duplicate pool_ids)
- [ ] CreatePool rejects zero-reserve creation
- [ ] LPShare added to `is_conditioned()` (R6 fix)
- [ ] `amm_activation_height` gate enforced in mempool, block-builder, AND apply_block (triplicate)
- [ ] Rollback correctly reverses all AMM UTXO mutations
- [ ] Pool metadata format documented and frozen (incl. new W2 fields `fee_bps_to_lp` + `fee_bps_to_protocol`); old 116-byte estimate revised — recompute and document final byte layout
- [ ] TWAP accumulator overflow-safe (u128 or wrapping arithmetic)
- [ ] No DeFi state interaction with epoch-boundary processing
- [ ] `guards_activation_height` activated on target network
- [ ] Escrow-loan covenant template E2E tested (create -> repay and create -> default paths) — W3 naming verified in CLI help text
- [ ] **Fee split verified (W2)**: total deduction = 30 bps; `fee_bps_to_lp + fee_bps_to_protocol == 30` enforced at CreatePool; `protocol_fee` Coin output minted to canonical reward-pool address per Swap; LP K-share matches `fee_bps_to_lp` exactly under property tests
- [ ] Pool metadata fee fields immutable post-CreatePool (no governance setter)
- [ ] Reward-pool address `BLAKE3("REWARD_POOL"||"doli")` verified to match existing epoch coinbase routing
- [ ] No wall-clock time usage (slot numbers only for all time-dependent logic)
- [ ] Three-question checklist (C7) answered in every commit touching DeFi code
- [ ] **Tokenomics document published** (`specs/tokenomics.md`) — MUST-DO before lowering `amm_activation_height`; documents how the 5 bps protocol fee fits into the Era 1+ security budget model

---

## Constraints (from Failure Analyst)

All Phase 1 code must satisfy these invariants:

| ID | Invariant | Applies to Phase 1? |
|----|-----------|---------------------|
| INV-DEFI-001 | AMM arithmetic determinism (bit-identical u128 integer math) | YES |
| INV-DEFI-002 | K-invariant non-decreasing after every Swap | YES |
| INV-DEFI-003 | LP share conservation (sum == pool.total_lp) | YES |
| INV-DEFI-004 | Collateral unspendable without repayment or timeout | YES (via conditions) |
| INV-DEFI-005 | No division by zero in pool math | YES |
| INV-DEFI-006 | Oracle price staleness bound | NO (no oracle in Phase 1) |
| INV-DEFI-007 | Liquidation improves position health | NO (no liquidation in Phase 1) |
| INV-DEFI-008 | Interest accrual determinism | NO (fixed rate at creation) |
| INV-DEFI-009 | NFT ownership continuity | NO (no NFT-frac in Phase 1) |
| INV-DEFI-010 | Pool UTXO singleton (one per pool_id) | YES |
| INV-DEFI-011 | No cross-primitive state leakage to epoch processing | YES |
| INV-DEFI-012 | Rollback consistency for DeFi UTXOs | YES |
| INV-DEFI-013 | Fee non-negative, bounded, and split sum-invariant: `fee_bps_to_lp + fee_bps_to_protocol == 30` (W2) | YES |
| INV-DEFI-014 | Lending pool solvency | NO (no lending pool in Phase 1) |
| INV-DEFI-015 | Activation gate triplicate enforcement | YES |
| INV-DEFI-016 | Integer overflow protection (u128 intermediates) | YES |
| INV-DEFI-017 | extra_data canonical encoding | YES |
| INV-DEFI-018 | Protocol fee Coin routes to canonical reward-pool address every Swap (W2) | YES |
| INV-DEFI-019 | Pool fee fields immutable post-CreatePool (W2) | YES |

---

## Milestones

| Milestone | Deliverables | Gate |
|-----------|-------------|------|
| M1: Module restructure | `defi/` subdirectory, import fixes, LPShare `is_conditioned()` fix | `cargo build --release && cargo clippy -- -D warnings` |
| M2: AMM validation hardened | u128 math, edge case handling, CreatePool duplicate check, **Pool metadata fee-split fields + sum-invariant (W2)** | Property tests pass (incl. sum-invariant + reward-pool routing tests) |
| M3: Test suite complete (DEF-3) | Property + adversarial + E2E tests (incl. fee-split proportionality + protocol-fee Coin minting per Swap) | `cargo test -p doli-core --lib` all green |
| M4: Activation infrastructure | `amm_activation_height` in NetworkParams, triplicate gate, **escrow-loan covenant template + `doli template escrow-loan` CLI (W3)** | Gate enforcement test passes |
| M5: Testnet validation | Full AMM lifecycle + escrow-loan template on local testnet; verify protocol-fee accrual to reward-pool address over 100+ swaps | Manual E2E sign-off |
| M6: Audit-ready | Documentation package, code freeze, external audit engagement, **`specs/tokenomics.md` published** | Audit-readiness checklist 100% (incl. tokenomics doc gate) |

---

## Design Synthesis Quality Gate

```
Evaluators completed:           5/5
Deletion convergence items:     3 (oracle, liquidation, NFT-frac -- 3+/5 agreement)
Restructuring convergence:      1 (defi/ subdirectory -- 2/5 explicit, others compatible)
Addition options presented:     0 (one recommended path per user instruction)
Failure modes identified:       17 (INV-DEFI-001 through INV-DEFI-017)
Failure modes applied as filters: 10/17 (F1-F10; 7 invariants N/A for Phase 1)
Radical floor gap:              current (0 active) -> radical (4 TX types) -> proposed (4 TX types) [EQUAL]
Contradictions found:           2
Contradictions resolved:        2/2
Evidence independence verified: YES
```

**Contradiction 1 (RESOLVED):** Pattern Matcher recommends (c) time-locked unanimity for D5 NFT redemption; all other evaluators recommend (a) share-buyback or deferral. Resolution: both positions are moot in Phase 1 (NFT-frac is deferred). Pattern Matcher's argument (reuse existing Threshold condition) is valid but limited by MAX_THRESHOLD_CONDITIONS=5 which the Failure Analyst's filter exposes as impractical. For Phase 3, share-buyback (a) is pre-selected.

**Contradiction 2 (RESOLVED):** Subtractionist proposes collapsing LPShare (10) into FungibleAsset (7); Restructurer and Pattern Matcher keep LPShare as distinct type. Resolution: Subtractionist's own kill test found that `is_conditioned()` membership differs between LPShare and FungibleAsset, making the collapse change consensus behavior. The collapse is REJECTED -- LPShare stays as type 10. However, LPShare MUST be added to `is_conditioned()` (R6).
