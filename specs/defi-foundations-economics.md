<!--
OUTPUT CONTRACT: N/A — specification file (not a test file)
INPUT PARTITIONS: N/A — specification file (not a test file)
-->

# DeFi Foundations Economics -- DOLI L1

**Date:** 2026-05-25
**Synthesis of:** 5 parallel DeFi economist evaluators
**Position:** Layered UNDERNEATH the 2026-05-24 AMM-First Phase 1 redesign
**Mode:** Proposal-only (no code changes)

---

## 0. APPROVAL STATUS (User Gate cleared 2026-05-25)

**Approved package:** SSF set + Option A (MaxDelta + ReserveRatio guards).

| # | Item | Status | LOC |
|---|------|--------|-----|
| P8 | LPShare `is_conditioned()` fix | **APPROVED** | ~5 |
| P5 | Pre-sim mempool contention signal | **APPROVED** | ~50-100 |
| P7 | Escrow-loan CLI template | **APPROVED** | ~50 |
| P3 | MaxDeltaGuard + ReserveRatioGuard (Option A) | **APPROVED** | ~30-170 |

**Total approved: ~135-325 LOC, 0 new activation heights, 0 new TX types, 0 new governance surfaces.**

### Locked pre-activation decisions

| # | Decision | Locked Value | Status | Reversibility |
|---|----------|--------------|--------|---------------|
| D1 | `MINIMUM_LIQUIDITY` | **1000** (Uniswap v2 standard) | LOCKED | Must be set before `amm_activation_height` |
| D2 | `pool_id` derivation | **Include `fee_bps` in hash** (`BLAKE3("DOLI_POOL" \|\| fee_bps \|\| sorted(asset_a, asset_b))`) — ~15 LOC | LOCKED | IRREVERSIBLE once `amm_activation_height` crosses |
| D3 | AC-2 sandwich MEV target | **Split** into AC-2a (intra-block = 0 bps, PASS structural) + AC-2b (cross-slot residual scaling with swap-size-to-pool-depth, PASS by honest disclosure) | **ACCEPTED 2026-05-29** | Spec change |
| D4 | AC-6 max-loss vs bonds | **Reframed as monitoring metric** — publish `R = total_active_bonds / max_pool_TVL` ratio via `getDefiHealthMetric` RPC + Prometheus gauge; no hard TVL cap, no TX rejection | **ACCEPTED 2026-05-29** | Spec change |

### Permanently dropped
- **P6 Restitution slash path** — 4/5 independent convergence on DROP. 100% slash-to-burn (EI-3) preserved.

### Deferred (Phase 2+)
- **P1 Oracle (PriceUpdate TX)** — ships when Phase 2 lending commits AND oracle security budget ≥ lending TVL × 2 (AC-1).
- **P4 Intent + bonded solver** — ships when off-chain solver auction with on-chain settlement verification is designed (>2000 LOC, exceeds AC-8 budget today).
- **P2 Batch settlement** — deferred to research. Future designs may solve determinism + IC + agent-readiness simultaneously.

### Next step (for the user)

Implementation is NOT auto-started. The approved items above are `code` Fixability (auto-implementable) and `design` Fixability (decisions to lock into specs/code before AMM activation). To start the milestone loop on the `code` items, run:

```
/omega-defi-redesign --fix
```

The `design` items (D1–D4) should be propagated into `specs/defi-subsystem-architecture.md`, `crates/core/src/consensus.rs` (`MINIMUM_LIQUIDITY` constant), and `crates/core/src/transaction/output.rs` (`compute_pool_id`) BEFORE `amm_activation_height` is set to a real value. These are pre-deployment requirements, not part of the SSF code patches.

---

## 1. TL;DR

**What ships:** 4 primitives totaling ~135-225 LOC, 0 new activation heights, 0 new TX types, 0 new governance surfaces. The SSF candidate is: (1) LPShare `is_conditioned()` fix (~5 LOC), (2) pre-simulation mempool contention signal (~50-150 LOC), (3) escrow-loan CLI template (~50 LOC), and (4) MaxDeltaGuard + ReserveRatioGuard condition guards (~30-170 LOC, see Options section for scope). All four are economically neutral, incentive-compatible, and score 4/4 on agent-readiness. No new emissions. No reward pool drain.

**What is deferred:** Oracle (PriceUpdate TX), Intent UTXO + bonded solver, batch settlement, deterministic TX ordering. All 4 lack either a Phase 1 consumer, a proven incentive-compatible design, or both. Oracle is structurally unsafe at 34 producers (AC-1 fails by 100x+). Intents degenerate without PBS. Batch settlement has no deterministic + incentive-compatible + agent-ready design. These become Phase 2+ candidates when their shipping triggers are met.

**What is permanently dropped:** Restitution slash path (5/5 evaluator convergence on DROP/REJECT). 100% slash-to-burn (EI-3) is strictly superior -- victim identification is ambiguous, fake-victim collusion recovers deterrent, and the governance surface it creates violates AC-7 on a 34-producer chain.

---

## 2. Hard Constraints Reaffirmed (Preserve-or-Fail)

| # | Constraint | Status |
|---|-----------|--------|
| HC-1 | DeFi-on-L1 -- no L2 punt | PRESERVED -- all proposals are L1-native |
| HC-2 | Deterministic protocol invariant | PRESERVED -- batch settlement + encrypted mempool rejected; all shipped primitives are deterministic |
| HC-3 | UTXO + Conditions/Covenants invariant | PRESERVED -- no account model, no VM; new guards extend existing condition system |
| HC-4 | Don't deprecate 2026-05-24 redesign | PRESERVED -- foundations layer is additive underneath, nothing removed |
| HC-5 | Existing protocol state preserved | PRESERVED -- `defi_activation_height=u64::MAX`, 100% burn, epoch-pool rewards, bond lifecycle |
| HC-6 | Activation-height discipline | PRESERVED -- 0 new activation heights (all ship under existing `guards_activation_height` or `amm_activation_height`) |
| HC-7 | No new token | PRESERVED -- all flows denominated in DOLI |
| HC-8 | Agent-readiness 4/4 hard requirement | PRESERVED -- every shipped primitive scores 4/4 |

---

## 3. Convergence Matrix

### 3.1 Per-Primitive Verdict (8 candidates x 5 evaluators)

| Primitive | Mechanism Skeptic | Adversarial Capital | Sustainability | Oracle/MEV | Governance/Minimal | Converged Verdict | Confidence |
|-----------|:-:|:-:|:-:|:-:|:-:|---|---|
| P1: Oracle (PriceUpdate TX) | DEFER (0.82) | DEFER (0.50) | SHIP (0.85) | DEFER (0.95) | DEFER (0.90) | **DEFER** (4/5) | conf(0.85, converged) |
| P2: Batch settlement | DROP (0.78) | DEFER (0.55) | SHIP (0.90) | DEFER (0.90) | DEFER (0.85) | **DEFER** (3/5 DEFER, 1 DROP, 1 SHIP) | conf(0.65, divergent) |
| P3: Condition guards (MaxDelta + ReserveRatio) | SHIP-MOD (0.85) | -- | SHIP (0.95) | -- | MARGINAL (0.70) | **SHIP** (see Options) | conf(0.75, partial) |
| P4: Intent + bonded solver | DEFER (0.75) | DEFER (0.60) | CONDITIONAL (0.75) | -- | DEFER (0.90) | **DEFER** (4/5) | conf(0.80, converged) |
| P5: Pre-simulation mempool | SHIP (0.90) | SHIP (0.75) | SHIP (0.95) | SHIP (0.85) | SHIP (0.80) | **SHIP** (5/5) | conf(0.90, converged) |
| P6: Restitution slash path | DROP (0.88) | DROP (0.90) | DEFER (0.70) | -- | REJECT (0.90) | **DROP** (4/5) | conf(0.88, converged) |
| P7: Escrow-loan template | SHIP (0.92) | SHIP (0.85) | SHIP (0.95) | SHIP (0.90) | SHIP (0.85) | **SHIP** (5/5) | conf(0.90, converged) |
| P8: LPShare `is_conditioned()` fix | SHIP (0.90) | -- | SHIP (0.95) | -- | MUST-FIX (0.95) | **SHIP** (5/5) | conf(0.92, converged) |

**Counts:** 4 SHIP / 3 DEFER / 1 DROP

### 3.2 Deletion Convergence (Permanent Drops)

```
                        MechSkep  AdverseCap  Sustain  Oracle/MEV  Gov/Min
Restitution (P6):         DROP      DROP       DEFER     --        REJECT  -> 4/5 -> DROP
Batch settlement (P2):    DROP      DEFER      SHIP      DEFER     DEFER   -> 1/5 DROP -> NOT converged DROP
```

CONVERGENCE INDEPENDENCE CHECK:
```
Deletion: Restitution slash path (P6)
Converging evaluators: Mechanism Skeptic, Adversarial Capital, Governance/Minimal (+ Sustainability lean-DEFER)
Evidence independence:
  - Mechanism Skeptic: Deviation analysis -- fake-victim collusion recovers 100% of slash via confederate
  - Adversarial Capital: A22 + A23 -- manufactured victim events + Beanstalk-style governance capture
  - Governance/Minimal: Governance surface analysis -- victim-determination is unsolvable governance problem
  - Sustainability: EI-3 signal value > quantitative value (<1% supply change)
  INDEPENDENT? YES -- four distinct analytical lenses (game theory, adversarial economics,
  governance, sustainability invariant) all arrive at rejection independently.
  True convergence -> conf(0.88, converged)
```

---

## 4. SSF Candidate (Minimum-Viable Foundations Layer)

Per Rule 18 (SSF), this is presented ALONE first. All 5 evaluators agree the 2026-05-24 redesign is already the correct shape. The foundations layer adds minimal patches.

### The SSF Set (3 items, ~105-155 LOC, 0 new activation heights)

| # | Primitive | LOC | Heights | TX Types | Gov Surface | Agent 4/4 | Source |
|---|-----------|-----|---------|----------|-------------|-----------|--------|
| 0 | LPShare `is_conditioned()` fix | ~5 | 0 (`amm_activation_height`) | 0 | None | 4/4 | 5/5 SHIP |
| 1 | Pre-sim mempool contention signal | ~50-100 | 0 (mempool only) | 0 | None | 4/4 | 5/5 SHIP |
| 2 | Escrow-loan CLI template | ~50 | 0 (`guards_activation_height`) | 0 | None | 4/4 | 5/5 SHIP |

**Total: ~105-155 LOC. AC-8 (<=800) PASS. AC-9 (<=3 heights) PASS.**

This SSF set satisfies AC-1 through AC-12 at Phase 1 scale:
- AC-1: PASS (vacuous -- no oracle shipped)
- AC-2: PASS for typical swaps (<10% pool reserves); documented residual for large swaps
- AC-3: IMPROVED by pre-sim (structural contention remains)
- AC-4: PASS (all primitives deterministic)
- AC-5: PASS (isolation by UTXO construction)
- AC-6: N/A (no insurance primitive)
- AC-7: PASS (no new governance surface)
- AC-8: PASS (105-155 LOC << 800)
- AC-9: PASS (0 new heights << 3)
- AC-10: 1 op/pool/block = 8,640/day (honest disclosure)
- AC-11: No shift (W2 25/5 split is primary mitigation)
- AC-12: PASS (pre-sim false-positive < 0.1%)

---

## 5. Per-Primitive Specs (SSF Set)

### P8: LPShare `is_conditioned()` Fix

- **What:** Add `OutputType::LPShare` to `is_conditioned()` match in `crates/core/src/transaction/types.rs:230-242`
- **LOC:** ~5
- **Activation:** Ships under `amm_activation_height` (changes consensus behavior for LPShare outputs)
- **Agent-readiness:** 4/4 (A: LPShare UTXOs queryable; B: condition eval deterministic; C: spend rejected if guard fails; D: standard UTXO + condition)
- **Attack cost:** N/A (security fix, not economic primitive)
- **Fixability:** `code` -- single `matches!` arm addition

Evidence: Mechanism Skeptic verified absence at `types.rs:230-242` (conf 0.90). Governance/Minimal confirmed correctness bug (conf 0.95). Identified as R6 in 2026-05-24 redesign.

### P5: Pre-Simulation Mempool Contention Signal

- **What:** When a Swap/AddLiquidity/RemoveLiquidity TX enters the mempool, check if another pending TX references the same Pool UTXO. Return diagnostic warning.
- **LOC:** ~50-100 in `crates/mempool/src/pool.rs` (extension of `add_transaction()` at line 205)
- **Activation:** None (mempool-only, not consensus). Three-question checklist: Q1=NO, Q2=NO, Q3=YES.
- **Agent-readiness:** 4/4 (A: diagnostic in `submitTransaction` response; B: deterministic UTXO check; C: agent gets bounded "contention likely" signal; D: standard TX submission)
- **Attack cost:** N/A (information primitive, no mechanism deviation surface)
- **Fixability:** `code` -- mempool extension

Evidence: Mechanism Skeptic verified mempool code at `pool.rs:205`, confirmed read-only simulation is deviation-free (conf 0.90). Oracle/MEV confirmed Pool UTXO contention model and quantified ~5% per-block deferral at N=5 swaps, M=10 pools (conf 0.75). AC-12 false-positive analysis: between simulation and inclusion, state can change; false-positive rate bounded by TX arrival rate in 10s window.

### P7: Escrow-Loan CLI Template

- **What:** CLI `doli template escrow-loan` that constructs condition-bearing Transfer TX composing AmountGuard + RecipientGuard + Timelock
- **LOC:** ~50 (CLI template function)
- **Activation:** None (free-rides on `guards_activation_height`)
- **Agent-readiness:** 4/4 (A: condition-bearing UTXOs queryable via `getUtxosByType`; B: condition eval deterministic; C: worst case = lose collateral (known at creation); D: standard conditioned UTXO)
- **Attack cost:** N/A (bilateral opt-in, no systemic risk)
- **Fixability:** `code` -- CLI template over existing primitives

Evidence: Mechanism Skeptic verified condition system at `conditions/eval.rs:121-165` (conf 0.92). Already specified in `specs/defi-subsystem-architecture.md` D3. Sustainability confirmed zero reward pool interaction (conf 0.95).

---

## 6. Options for User Decision

### OPTION A: Include MaxDeltaGuard + ReserveRatioGuard (Mechanism Skeptic proposal)

**What:** Two new condition guard types:
- `MaxDeltaGuard { max_change_bps: u16, reference_amount: Amount, output_index: u8 }` -- limits per-operation price movement
- `ReserveRatioGuard { min_ratio_bps: u16, reserve_output_index: u8, debt_output_index: u8 }` -- enforces minimum collateralization

**Evidence:** Mechanism Skeptic: purely restrictive guards with no deviation surface (conf 0.85). Euler Finance ($197M) enabled by lack of per-operation bounds. Sustainability: zero fee impact (conf 0.95).

**Cost:** ~170 LOC (2 guards x ~85 LOC each: new tag bytes in `encoding.rs`, eval logic in `eval.rs`, encoding/decoding, tests). Ships under existing `guards_activation_height`.

**vs. SSF floor:** +170 LOC above SSF (total ~275-325). Still well within AC-8 (800).

**Governance/Minimal dissent:** MaxDelta is "marginal" -- client-side simulation is sufficient for agents. Include only if guard system is already being touched. conf(0.70).

**Failure mode filter:**
- FM-RESTITUTION: NEUTRAL (guards do not interact with slash path)
- FM-ORACLE: NEUTRAL (guards are local per-UTXO, no oracle dependency)
- FM-MEV: NEUTRAL (guards protect user, do not change producer MEV surface)

**Agent-readiness:** 4/4 (discoverable via condition inspection, deterministic integer comparison, bounded at creation, composable via And/Or/Threshold).

**Recommendation:** Include MaxDeltaGuard at minimum (~30-40 LOC). ReserveRatioGuard is Phase 2 lending infrastructure -- can defer. If user wants defense-in-depth for Phase 1 swaps, ship MaxDeltaGuard now.

### OPTION B: Include `fee_bps` in `pool_id` derivation (Oracle/MEV proposal)

**What:** Change `compute_pool_id()` at `output.rs:729` from `BLAKE3("DOLI_POOL" || sorted(asset_a, asset_b))` to `BLAKE3("DOLI_POOL" || sorted(asset_a, asset_b) || fee_bps.to_le_bytes())`. Enables multiple pools per asset pair with different fee tiers.

**Evidence:** Oracle/MEV Analyst confirmed current derivation prevents multi-pool-per-pair permanently once `amm_activation_height` is crossed (conf 0.95, verified at `output.rs:729`, test at `validation/pool.rs:357`). This is an **irreversible design decision**. Changing pool_id later requires a consensus-breaking migration of all existing pools.

**Cost:** ~15 LOC in `compute_pool_id()` + `validate_create_pool()`. No new activation height (covered by `amm_activation_height`). No new TX type.

**vs. SSF floor:** +15 LOC. Negligible complexity. But it is a design decision, not a foundations primitive.

**Dissent:** No evaluator argued against this. The Sustainability auditor's 10-year projection showed 1 pool/pair is sufficient for years of realistic volume, but the Oracle/MEV analyst's point is that retrofitting later costs a consensus migration while adding now costs ~15 LOC.

**MUST BE DECIDED BEFORE `amm_activation_height` IS CROSSED.**

### OPTION C: Deterministic TX ordering (Adversarial Capital mention)

**What:** Sort TXs within a block by `BLAKE3(tx_hash || slot_number)` instead of producer discretion. Removes producer reordering MEV (A3).

**Evidence:** Adversarial Capital quantified producer reordering at 5-20 bps per contended slot (conf 0.65). ~200 LOC. Changes block content (MEMORY.md Rule #0 -- requires activation gate).

**Cost:** ~200 LOC + 1 new activation height.

**vs. SSF floor:** +200 LOC + 1 activation height. Exceeds SSF by meaningful amount.

**Failure mode filter:**
- FM-MEV: RESOLVES (eliminates producer self-deal entirely)
- FM-FEE-MARKET: VULNERABLE (reduces fee-market efficiency -- producer cannot prioritize high-fee TXs)

**Recommendation:** DEFER. The problem is 5-20 bps on contended slots with uncertain frequency. The solution changes block content, requires activation gate, and may reduce fee revenue. Research candidate for Phase 2.

---

## 7. Deferred Tier (Phase 2+)

### P1: Oracle (PriceUpdate TX / PriceObservation UTXO)

**Verdict:** DEFER (4/5 evaluators)
**Confidence:** conf(0.85, converged)

**Shipping trigger:** ALL of the following:
1. Phase 2 pooled lending redesign committed with binding timeline
2. Producer count > 100 (attestation oracle) OR TWAP window >= 3,600 slots (TWAP oracle)
3. AC-1 recalculated for actual producer count and bond distribution at that time
4. `guards_activation_height` already crossed on mainnet

**Why not now:** Zero consumers in Phase 1 (AMM self-prices, escrow-loan oracle-free). Attestation oracle fails AC-1 by >100x at 34 producers (Oracle/MEV: corruption cost ~$17K at $10/DOLI vs $2M target for $1M TVL). TWAP passes AC-1 only at 3,600-slot window (dangerously stale). Adding unused attack surface violates SSF.

**Evaluator evidence:**
- Oracle/MEV: AC-1 attack cost table (6 designs quantified, all FAIL except no-oracle and TWAP W=3600 marginal). conf(0.95).
- Mechanism Skeptic: Oracle design errors empirically catastrophic (Mango $117M, Harvest $34M, bZx $950K). conf(0.82).
- Adversarial Capital: Bond-weighted median capture cost $120-$510K at 34 producers -- less than meaningful lending TVL. conf(0.50).
- Sustainability: Zero-cost public good, sustainable if shipped -- but deferral costs nothing. conf(0.85).

### P4: Intent UTXO + Bonded Solver

**Verdict:** DEFER (4/5 evaluators)
**Confidence:** conf(0.80, converged)

**Shipping trigger:** ALL of the following:
1. Multi-pool routing exists (requires pool_id change per Option B above)
2. Cross-protocol composition exists (Phase 2 lending + AMM)
3. Off-chain solver auction with on-chain settlement verification designed, OR PBS implemented (which HC-2 currently prohibits)
4. Bond sizing formula validated against empirical intent volume data

**Why not now:** Degenerates to "producer fills all intents" without PBS (Mechanism Skeptic: producer = block builder = dominant strategy, conf 0.75). Creates negative value -- users who use it lose to producer MEV (Mechanism Skeptic). Bond sizing requires empirical data (Adversarial Capital: A19 solver extraction, conf 0.60). ~400-600 LOC exceeds AC-8 on its own.

### P2: Batch Settlement

**Verdict:** DEFER (3/5 DEFER, 1 DROP, 1 SHIP -- divergent)
**Confidence:** conf(0.65, divergent)

**Shipping trigger:** (research milestone, not implementation trigger)
1. Deterministic + incentive-compatible + agent-ready batch design published
2. AC-2 demonstrated to fail at production volumes (currently < 2 bps for typical swaps)

**Why not now:** Mechanism Skeptic DROP: no design satisfies HC-2 (determinism) + HC-8 (agent-readiness B/D) + incentive compatibility simultaneously (conf 0.78). Adversarial Capital DEFER: would close AC-2 gap but 500-800 LOC (conf 0.55). Sustainability SHIP: zero fee impact (conf 0.90). Oracle/MEV DEFER: solves problem that doesn't exist at < 2 bps (conf 0.90). Governance DEFER: speculative (conf 0.85).

WARNING -- UNRESOLVED CONTRADICTION:
```
Mechanism Skeptic says DROP (no incentive-compatible design exists).
Sustainability says SHIP (cash-flow neutral, MEV reduction increases volume retention).
Resolution: Mechanism Skeptic's analysis is structural (the IC failure applies to ALL batch
designs on DOLI's architecture). Sustainability's analysis is cash-flow only (it does not
evaluate IC). The IC failure dominates. DEFER (not permanent DROP) because future design
breakthroughs are possible.
```

---

## 8. Dropped Tier (Permanently)

### P6: Restitution Slash Path

**Verdict:** DROP (4/5 evaluators: 2 DROP, 1 REJECT, 1 DEFER)
**Confidence:** conf(0.88, converged)

**Reasoning (independent convergence from 4 lenses):**

1. **Mechanism Skeptic (DROP, 0.88):** No incentive-compatible victim identification. Fake-victim collusion recovers up to 100% of slash. "100% burn is the correct mechanism design."
2. **Adversarial Capital (DROP, 0.90):** A22 manufactured victim events + A23 governance surface. AC-7 violated: governance capture cost < restitution value on 34-producer chain.
3. **Governance/Minimal (REJECT, 0.90):** Victim-determination is unsolvable governance problem. "Preserve the moat" -- no new governance surface.
4. **Sustainability (DEFER, 0.70):** Cash-flow impact negligible (<1% supply over 10 years). Concern is EI-3 signal value, not math. "Breaking invariants has costs beyond the numbers."

**EI-3 preservation:** 100% slash-to-burn is the correct design. Burns are apolitical (benefit all holders via deflation). No PoS chain routes slash to specific victims. Restitution creates an implicit insurance promise the protocol cannot fund.

---

## 9. Critical Pre-Activation Decisions

The user MUST decide these before `amm_activation_height` is crossed:

### Decision 1: MINIMUM_LIQUIDITY value (BLOCKER -- W7)

**Current:** Unspecified. **Required:** >= 1000 (Uniswap v2 standard).

At MINIMUM_LIQUIDITY = 1: first-deposit inflation attack steals up to 100% of first real deposit (Adversarial Capital A4/A7, conf 0.90). At 1000: attack becomes 1:1 cost/payoff = unprofitable (Adversarial Capital, verified against 8-decimal DOLI arithmetic).

### Decision 2: `pool_id` derivation (IRREVERSIBLE)

**Current:** `BLAKE3("DOLI_POOL" || sorted(asset_a, asset_b))` -- one pool per pair.
**Proposed:** Include `fee_bps` in hash to enable multi-fee-tier pools (~15 LOC).

This is irreversible once `amm_activation_height` is crossed. Changing later requires consensus migration. See Option B above.

### Decision 3: AC-2 target split — **ACCEPTED 2026-05-29**

**Previous (pre-2026-05-29):** AC-2 = "sandwich MEV <= 5 bps net" (single target). REJECTED — Adversarial Capital demonstrated this as written FAILS (15-60 bps cross-slot for swaps > 0.6% of reserves, conf 0.90). Oracle/MEV said < 2 bps for typical swaps (conf 0.85).

**Accepted split (verbatim normative wording):**

> **AC-2a (Intra-block sandwich MEV) — PASS, structural.** Within a single block, two swaps against the same Pool UTXO are mutually exclusive by UTXO consumption semantics. Intra-block atomic 3-TX sandwich MEV is **0 bps** and cannot be expressed.
>
> **AC-2b (Cross-slot MEV) — PASS by honest disclosure.** Cross-slot sandwich and producer-driven reordering remain extractable as a documented residual that scales with swap-size-to-pool-depth ratio. The 30 bps round-trip swap fee makes extraction **net-unprofitable for swaps below ~0.6% of pool reserves**. At larger sizes residual MEV scales: ~39 bps at 1% of pool, ~416 bps at 5% of pool. No system claim of "MEV-free" or "≤ 5 bps MEV" is made.

Source: §10 Contradiction 1 (this document). All public-facing documentation (whitepapers EN+ES, architecture docs, READMEs) MUST conform to AC-2a/AC-2b wording. The phrases "MEV-free", "sandwich-proof", and "≤ 5 bps MEV" are PROHIBITED as system-wide claims.

### Decision 4: AC-6 reframing — **ACCEPTED 2026-05-29**

**Previous (pre-2026-05-29):** AC-6 = "max single-primitive loss <= sum(all_producer_bonds)" (hard cap). REJECTED — Adversarial Capital demonstrated this is structurally violated at low DOLI price (uncapped pool TVL exceeds total bonds when P is small, conf 0.90). A hard TVL cap is economically counter-productive (artificially limits AMM utility without preventing loss).

**Accepted reframe (verbatim normative wording):**

> **AC-6 (Economic security ratio) — monitoring metric, not a hard cap.** The protocol publishes the ratio `R = total_active_bonds / max_pool_TVL` where `max_pool_TVL` is the largest single Pool UTXO total reserve value expressed in the same numeraire as bonds (DOLI). No transaction is rejected on the basis of `R`. When `R < 1.0`, the protocol publicly discloses that single-pool capital exceeds the bonded security budget and that economic security against pool-level capture is degraded. The `getDefiHealthMetric` RPC and a corresponding Prometheus gauge (`doli_defi_bond_to_tvl_ratio`) surface this ratio continuously.

Phase-1 numeraire caveat (encoded in the RPC `note` field): pre-oracle, `max_pool_TVL` is computed using the pool's own internal spot price (`tvl ≈ 2 × reserve_a`). This is self-referential by construction and is the most honest measurement available before an external price oracle exists. Phase 2 oracle activation (`oracle_activation_height`) replaces the self-referential formula with an attested DOLI/asset_b price.

---

## 10. Contradiction Reconciliation

### Contradiction 1: MEV Residual Quantification

**Oracle/MEV says:** < 2 bps net for typical swaps (< 10% pool reserves). AC-2 PASSES.
**Adversarial Capital says:** 10-50 bps cross-slot for swaps > 0.6% of pool. AC-2 FAILS.

**Resolution:** Both are correct. The discrepancy is in what counts as "typical":
- Oracle/MEV defines "typical" as swap < 10% of pool reserves. At this size, the 30 bps round-trip fee dominates extraction. Net MEV < 2 bps. CORRECT.
- Adversarial Capital analyzes ALL swap sizes including large ones. At swap = 1% of pool: 39 bps extraction. At 5%: 416 bps. Break-even at 0.6% of reserves. CORRECT.

**Reconciled statement:** "Intra-block sandwich MEV = 0 bps (structural). Cross-slot MEV is net-unprofitable for swaps below 0.6% of pool reserves (~$6K on a $1M pool). For swaps above this threshold, residual MEV scales with swap-size-to-pool-depth ratio: ~39 bps at 1% of pool, ~416 bps at 5% of pool. The 30 bps round-trip fee is the natural MEV deterrent."

**Honest disclosure:** Do NOT claim "MEV-free" or "< 5 bps MEV." State the swap-size-dependent residual with the break-even threshold.

### Contradiction 2: AC-2 Target Validity — **RESOLVED 2026-05-29**

**Resolution:** Split AC-2 into AC-2a (intra-block: 0 bps, PASS structural) and AC-2b (cross-slot residual scaling with swap-size-to-pool-depth, PASS by honest disclosure). User approved 2026-05-29. See §9 Decision 3 for normative wording.

### Contradiction 3: AC-6 Target Validity — **RESOLVED 2026-05-29**

**Adversarial Capital:** AC-6 structurally violated at low DOLI price (pool TVL unbounded, total bonds bounded).
**Resolution:** Reframe AC-6 as monitoring metric `R = total_active_bonds / max_pool_TVL`. When `R < 1.0`, publish the ratio and document degraded economic security. No TX rejection. User approved 2026-05-29. See §9 Decision 4 for normative wording.

### Contradiction 4: Restitution Path

**Mechanism Skeptic + Adversarial + Governance:** DROP (3 independent analyses).
**Sustainability:** CONDITIONAL (lean DEFER) -- cash-flow is clean but EI-3 signal matters.
**Resolution:** DROP. The 3 independent DROP analyses outweigh the 1 conditional DEFER. Sustainability's own analysis concludes "breaking invariants has costs beyond the numbers" and recommends preserving EI-3 in Phase 1. Convergence on DROP.

### Contradiction 5: Batch Settlement

**Mechanism Skeptic:** DROP (no IC design exists).
**Sustainability:** SHIP (cash-flow neutral).
**Others:** DEFER.
**Resolution:** DEFER. IC failure is structural (Mechanism Skeptic's analysis dominates Sustainability's cash-flow-only view). But not permanent DROP because future designs may solve the IC + determinism constraint. Research candidate.

---

## 11. Agent-Readiness Compliance Table

| Primitive | (A) Discoverability | (B) Determinism | (C) Bounded Exec | (D) Composability | Score |
|-----------|:---:|:---:|:---:|:---:|:---:|
| P8: LPShare fix | `getUtxosByType(LPShare)` | Condition eval deterministic | Spend rejected on guard fail | Standard UTXO + condition | 4/4 |
| P5: Pre-sim contention | `submitTransaction` diagnostic | Deterministic UTXO check | "Contention likely" signal | Standard TX submission | 4/4 |
| P7: Escrow-loan template | `getUtxosByType(Collateral)` | Condition eval deterministic | Worst case = lose collateral | Standard conditioned UTXO | 4/4 |
| P3: MaxDeltaGuard (if shipped) | Guard params in UTXO `extra_data` | Integer comparison | Spend rejected on guard fail | And/Or/Threshold composition | 4/4 |

---

## 12. Migration Path (What Changes vs. 2026-05-24 Redesign)

This is **purely additive**. Nothing in the 2026-05-24 redesign is removed or modified.

| 2026-05-24 Redesign Element | Change from This Pass | Status |
|---|---|---|
| 4 AMM TX types (19-22) | None | Preserved |
| W2 fee split (25 bps LP / 5 bps protocol) | None | Preserved |
| Per-primitive activation heights (amm/lending/nft_frac) | None | Preserved |
| Escrow-loan specification (D3) | CLI template added (~50 LOC) | Extended |
| `is_conditioned()` R6 gap | Fix shipped (~5 LOC) | Fixed |
| Pre-sim mempool admission | Added (~50-100 LOC) | New |
| MaxDeltaGuard + ReserveRatioGuard | Added if Option A accepted (~30-170 LOC) | New (optional) |
| `pool_id` derivation | Changed if Option B accepted (~15 LOC) | Changed (optional) |
| Oracle, intent, batch, restitution | Explicitly deferred/dropped with reasoning | Clarified |

---

## 13. Complexity Comparison

| Metric | Current (Phase 0) | SSF (this pass) | SSF + Options A+B | Full (all 8 candidates) |
|--------|:-:|:-:|:-:|:-:|
| New LOC (foundations) | 0 | ~105-155 | ~290-340 | ~1,200-1,800 |
| New activation heights | 0 | 0 | 0 | 3-4 |
| New TX types | 0 | 0 | 0 | 3-4 |
| New OutputType variants | 0 | 0 | 0 | 1-2 |
| New governance surfaces | 0 | 0 | 0 | 3 |
| AC-8 compliance (<=800) | YES | YES | YES | NO |
| AC-9 compliance (<=3 heights) | YES | YES | YES | MARGINAL |
| Gov capture cost | Infinite | Infinite | Infinite | Finite (oracle/solver) |

---

## 14. Fixability Classification

| Primitive | Classification | Notes |
|-----------|:---:|---|
| P8: LPShare `is_conditioned()` | `code` | Single `matches!` arm addition |
| P5: Pre-sim mempool contention | `code` | Mempool extension, no consensus |
| P7: Escrow-loan CLI template | `code` | CLI template function |
| P3: MaxDeltaGuard (if shipped) | `code` | New condition guard evaluation |
| Option B: `pool_id` derivation | `design` | Requires user design decision before implementation |
| AC-2 split | `design` | Requires user approval of reframed acceptance criterion |
| AC-6 reframe | `design` | Requires user approval of monitoring-only approach |
| MINIMUM_LIQUIDITY | `design` | Requires user to set value (recommended: 1000) |

---

## 15. Residual MEV Characterization (Honest Disclosure)

Per W6 from the 2026-05-24 redesign, residual MEV must be documented honestly.

| MEV Source | Intra-block | Cross-slot | Annual USD ($1M/day, P=$1) |
|-----------|:-:|:-:|:-:|
| Sandwich | 0 bps (structural) | 10-50 bps (swap > 0.6% pool) | $18K-$90K |
| Producer reordering | N/A | 5-20 bps (contended slots) | $18K-$73K |
| Toxic LP / LVR | N/A | 5-15 bps (every price move) | $18K-$55K |
| JIT liquidity | 0 bps (structural) | 0 bps (structural) | $0 |
| **Total** | **0 bps** | **20-85 bps (affected swaps)** | **$54K-$218K** |

**Structural defenses already in place:**
1. 30 bps swap fee makes round-trip extraction net-negative for swaps < 0.6% of pool
2. Pool UTXO singleton prevents intra-block atomic attacks
3. No concentrated liquidity eliminates JIT MEV
4. Deterministic public schedule removes information asymmetry

**Do NOT state in user docs:** "MEV-free," "sandwich-proof," "< 5 bps MEV."
**DO state:** "Residual MEV is structurally bounded by DOLI's 30 bps swap fee and single-swap-per-pool-per-block UTXO model. Cross-slot reordering is theoretically possible but net-unprofitable for typical swaps below ~$6K on a $1M pool."

---

## 16. Design Synthesis Quality Gate

```
---- DESIGN SYNTHESIS QUALITY GATE ----
Evaluators completed:             5/5
Primitives evaluated:             8
SHIP verdicts (converged):        4 (P3, P5, P7, P8)
DEFER verdicts (converged):       3 (P1, P2, P4)
DROP verdicts (converged):        1 (P6)
Deletion convergence items:       1 (4/5 agreement on P6)
Options presented to user:        3 (A: guards scope, B: pool_id, C: TX ordering)
Failure modes identified:         6 (A2 sandwich, A3 reordering, A4/A7 first-deposit,
                                     A17 oracle, A19 solver, A22/A23 restitution)
Failure modes applied as filters: 6/6
SSF floor gap:                    Current(0 LOC) -> SSF(105-155 LOC) -> Full(1200-1800 LOC)
Contradictions found:             5
Contradictions resolved:          5/5
Evidence independence verified:   YES (for P6 deletion convergence)
--------------------------------------
```
