# DeFi Foundations Redesign — Analyst Scoping

**Date:** 2026-05-25
**Workflow:** `/omega-defi-redesign` (re-run, L1-only + agent-ready constraints)
**Position in pipeline:** Layered UNDERNEATH the 2026-05-24 AMM-First Phase 1 redesign.

## Summary (one paragraph)

**(a) Constraints confirmed:** All 8 hard constraints from `docs/.workflow/prompt-refinement.md` verified against codebase — DeFi-on-L1 (no L2 code for DeFi), deterministic protocol (no leader auctions), UTXO+Conditions invariant (15 OutputTypes, condition system with 5 guard types), 2026-05-24 redesign as base (AMM Phase 1 spec-only, not yet in code), existing state preserved (`defi_activation_height=u64::MAX`, 100% burn on slash, epoch-pool rewards), activation-height discipline (15 existing heights), no new token, agent-readiness as hard requirement.

**(b) Acceptance criteria defined:** 12 quantified criteria across 6 categories.

**(c) Capability inventory:** 8 candidate primitives assessed — 1 PARTIAL (TWAP accumulator exists but is informational-only), 7 NEW. Free slots: TX types 16 and 23 available; OutputTypes 15+ available; 3 new activation heights planned but not yet implemented.

**(d) Contradictions found:** Two hypotheses in the refined prompt partially contradicted by code. H2 (sandwich MEV) assumes "DOLI's deterministic schedule already forbids" leader-chosen ordering — but the block builder DOES choose which TXs to include from the mempool. H4 (failed-tx) assumes "DOLI is deterministic" eliminates the class — but Pool UTXO contention means valid-at-submission swaps can fail at inclusion due to deterministic contention (different failure mode from Solana's probabilistic admission).

---

## 1. Affected Economic Subsystems

### IN SCOPE

1. **AMM Phase 1** (as consumer of new primitives) — `crates/core/src/pool.rs`, `crates/core/src/validation/pool.rs`, future `crates/core/src/defi/amm.rs`. The 2026-05-24 redesign specifies 4 TX types (CreatePool=19, AddLiquidity=20, RemoveLiquidity=21, Swap=22) with 25/5 fee split.
2. **Escrow-loan Phase 2** (as consumer) — `crates/core/src/conditions/templates.rs` (5 templates exist). Escrow-loan PROPOSED, not yet in code.
3. **Price discovery** (NEW) — No oracle or price-observation primitive exists. The TWAP accumulator (`pool.rs:82-108`) is informational only.
4. **TX ordering / MEV** (NEW) — Block builder selects from mempool by fee priority. No ordering commitment or batch-settlement primitive.
5. **Insurance / restitution** (extension of existing burn) — Two burn paths exist: vesting penalty and 100% slash. No restitution path.
6. **Intent / agent execution** (NEW) — Agent-allowance template is closest existing primitive but is NOT an intent system.
7. **Mempool admission** (extension) — Mempool validates structure + DeFi gate but does not pre-simulate DeFi execution against current state.
8. **Composability isolation** (extension of Conditions) — `is_conditioned()` covers 8 output types; **LPShare is NOT included** (R6 gap from 2026-05-24).

### OUT OF SCOPE

Consensus rule changes, P2P/gossip, frontend/explorer, bridges, NFT-frac Phase 3 mechanics, stablecoin primitives, KYC pools, smart-contract VM, ZKSettle/L2 settlement, novel curves (StableSwap/CL), liquidity-mining emissions, treasury separation, single-proposer migration.

---

## 2. Economic Invariants (preserve-or-fail)

| ID | Invariant | Source |
|----|-----------|--------|
| EI-1 | Determinism of block content given (height, mempool, scheduler state) | CLAUDE.md, hard constraint 2 |
| EI-2 | UTXO atomicity — no partial-tx state | hard constraint 3, atomic batch write |
| EI-3 | 100% slash → burn | hard constraint 5, `consensus/exit.rs:130-147` |
| EI-4 | Epoch-pool reward distribution (no per-block leader incentive) | `bins/node/src/node/rewards.rs` |
| EI-5 | `defi_activation_height = u64::MAX` until audit | INC-I-088, `network_params/defaults.rs` |
| EI-6 | Single DOLI unit of account (no new token) | hard constraint 7 |
| EI-7 | Activation-height discipline (per-feature, never bundled) | INC-I-054, INC-I-075 |
| EI-8 | Bond vesting + 7-day withdrawal delay preserved | hard constraint 5, `consensus/exit.rs:100-118` |
| EI-9 | `CURRENT_PROTOCOL_VERSION` NOT bumped unless EpochState format changes | INC-I-054, CLAUDE.md |
| EI-10 | Block-content changes require synchronized deploy OR activation-height gate | MEMORY.md #0 |
| EI-11 | Pool UTXO singleton — one per `pool_id` | INV-DEFI-010 |
| EI-12 | Fee fields immutable post-CreatePool | INV-DEFI-019 |
| EI-13 | K-invariant non-decreasing after every Swap | INV-DEFI-002, `pool.rs:110-120` |

---

## 3. Quantified Acceptance Criteria

| # | Criterion | Target | Measurement |
|---|-----------|--------|-------------|
| AC-1 | Oracle attack cost (if price primitive ships) | Cost to manipulate TWAP by >5% must exceed TVL × 2 | Adversarial simulation: capital required to move bond-weighted median or TWAP outside 5% band for N consecutive slots, attacker controls ≤1/3 producer stake |
| AC-2 | Sandwich MEV per swap | Net extractable ≤ 5 bps after fees | Replay 1,000 random orderings on representative pool; attacker cross-slot profit minus 2× 30bps fees |
| AC-3 | Failed-tx ratio under DeFi load | ≤ 1% | Testnet stress: 100 concurrent Swaps over 10 pools / 100 blocks; count mempool-valid TXs that fail at apply_block from UTXO contention |
| AC-4 | Agent simulation accuracy | 100% bit-identical | Off-chain simulator using only RPC state reproduces post-state of 100 test TXs |
| AC-5 | Contagion blast radius | Single pool OR single escrow position | Grep + property test: no DeFi TX writes to UTXOs not in inputs/outputs; failure on Pool A leaves Pool B byte-identical |
| AC-6 | Insurance coverage vs shortfall | Max single-primitive loss ≤ sum(all_producer_bonds) | If max_pool_TVL > total slashable bonds, primitive needs cap |
| AC-7 | Governance capture cost | ≥ controlled-value-at-risk | No new mutable parameter that controls more value than capture cost |
| AC-8 | New code surface for foundations | ≤ 800 LOC (excluding AMM Phase 1's ~1,630 LOC) | Line count |
| AC-9 | Activation-height count added | ≤ 3 new (amm, lending, nft-frac already planned) | Count NetworkParams fields |
| AC-10 | Pool UTXO throughput ceiling documented | Honest disclosure | 1 op/pool/block = 8,640 ops/day; multi-pool routing documented as scaling path |
| AC-11 | LP-vs-bond yield ratio monitoring | Document crossover threshold | LP_yield = (25bps × daily_vol × 365) / TVL; Bond_yield = (subsidy + 5bps fee revenue) / total_bonds |
| AC-12 | Pre-simulation admission false-positive rate | ≤ 0.1% | Replay 10,000 valid TXs; verify ≤ 10 false rejections |

---

## 4. Architecture Context

### Module Boundaries

- `crates/core/src/transaction/types.rs`: `OutputType` (15 variants), `is_conditioned()` (8 types).
- `crates/core/src/conditions/`: 6 files; `MAX_CONDITION_DEPTH=4`, `MAX_CONDITION_OPS=128`.
- `crates/core/src/validation/transaction.rs`: DeFi gate at line 116 → `[ERRTX-DEFI001]`.
- `crates/core/src/validation/utxo.rs`: `verify_input_conditions()` at line 966.
- `bins/node/src/node/apply_block/tx_processing.rs`: DeFi match arms (gated).
- `crates/core/src/network_params/mod.rs`: 15+ activation-height fields.
- `crates/mempool/src/pool.rs`: `add_transaction()` at line 205.

### Data Flow

```
User → CLI/RPC → Mempool.add_transaction(tx, utxo_set, height)
  → validate_transaction(&tx, &ctx) [DeFi gate]
  → verify_input_conditions() [conditioned outputs]
  → BlockBuilder selects by fee priority
  → apply_block/tx_processing.rs [match on tx_type]
  → validation/{pool,lending,fractionalize}.rs
  → apply_block/state_update.rs [UTXO mutations]
  → state root hash [must match across nodes]
```

### Constraints

1. **Encoder/decoder index parity** (CLAUDE.md Pillar 2): new sorted-list block-content state needs bitfield discipline.
2. **Snap-sync compatibility**: new state must be in ChainState/UtxoSet/ProducerSet OR derivable from them.
3. **`CURRENT_PROTOCOL_VERSION` rule**: DeFi primitives change UTXOs, not EpochState — no version bump.
4. **Block-content vs consensus-rules** (MEMORY.md Rule #0): block content changes need synchronized deploy OR activation-height gate.
5. **Three-question consensus-shape checklist** (INC-I-075).
6. **Pool UTXO is shared mutable state** — 1 operation per pool per block is a fundamental UTXO-model constraint.

---

## 5. Capability Inventory

| Primitive | Status | Evidence |
|-----------|--------|----------|
| Attestation-rooted oracle / price observation | **NEW** | TWAP math exists in `pool.rs:82-108` but informational only. No PriceUpdate TX. BLS infra exists but not used for price attestation. `lending.rs:42-45` references TWAP but is never called from live code. |
| Slot-batch settlement / uniform clearing | **NEW** | Grep for `batch`/`clearing`/`uniform`: no results. Block builder uses fee-priority. |
| Condition-gated positions / circuit breakers | **PARTIAL** | Condition system supports composition. AmountGuard is closest to slippage guard. No circuit-breaker-specific guards (MaxDelta, ReserveRatioGuard). |
| Intent UTXO + bonded solver | **NEW** | No Intent OutputType. Agent-allowance template is closest existing pattern but not an intent system. |
| Pre-simulation mempool admission for DeFi | **NEW (extension)** | Mempool validates structure + DeFi gate. Does NOT simulate DeFi execution against current state. Pool UTXO contention not detected. |
| Restitution slash path | **NEW** | Only burn paths exist. 100% slash → burn. No restitution. `BurnAsset` burns FungibleAsset only, not DOLI. |
| Escrow-loan template | **PROPOSED** | 5 templates exist. Escrow-loan specified in `specs/defi-subsystem-architecture.md` §D3 but not in code. CLI subcommand absent. Blocked by `guards_activation_height = u64::MAX` on mainnet. |

**Free TX type slots:** 16, 23 (two available without renumbering)
**Free OutputType slots:** 15-255 (u8 range, only 0-14 used)
**Activation heights:** 15+ existing in NetworkParams; 3 planned (amm/lending/nft_frac) not yet in code.

---

## 6. Agent-Readiness Scoring Rubric

Every primitive proposed by evaluators MUST score on this 4-point rubric:

- **(A) Discoverability** — Can an agent enumerate all live instances via standard RPC? Must specify RPC endpoint.
- **(B) Determinism** — Can an off-chain simulator reproduce post-state exactly from published state? No callbacks, no randomness, integer-only arithmetic with specified rounding.
- **(C) Bounded Execution** — Can the agent compute worst-case outcome at submission time? Pool UTXO contention means inclusion isn't guaranteed, but the worst case ("not included") is bounded.
- **(D) Composability** — Can the primitive be consumed by another primitive without bespoke parsing? Standard UTXO semantics + documented `extra_data` layout.

**Scoring:** 4/4 required unless evaluator provides written justification + mitigation plan.

---

## 7. Contradictions Found

### Contradiction 1: H2 (Sandwich MEV)

**Hypothesis:** "DOLI's deterministic schedule already forbids leader-chosen tx ordering."

**Reality:** The block builder chooses inclusion order from the mempool by fee priority. Determinism applies to WHICH producer builds (scheduler), not WHAT they include. Producer reordering MEV is real. Cross-slot sandwich is real. The foundations layer must characterize residual MEV honestly.

### Contradiction 2: H4 (Failed-TX ratio)

**Hypothesis:** "DOLI is deterministic. Pre-simulation admission eliminates the class."

**Reality:** Pool UTXO contention means a mempool-valid TX can fail at apply_block when another TX consumes the same Pool UTXO. This is deterministic contention, not probabilistic admission. Pre-simulation can REDUCE failure rate (warn agents) but cannot prevent contention-based non-inclusion. AC-3 (≤1%) must accommodate this structural limit.

---

## 8. SSF Result

**Can the empty set work?** Partially yes for Phase 1 isolation. The 2026-05-24 redesign can ship without new foundational primitives — it consumes Pool UTXO, condition system, reward pool routing.

**Why the empty set is insufficient:** The user's mandate is foundational primitives for "this AND OTHER PROTOCOLS." The price primitive (for future pooled lending), agent-intent execution (for agent-to-agent DeFi), and MEV characterization (for honest positioning) are foundations Phase 1 cannot provide.

**Recommendation to evaluators:** This pass defines CRITERIA AND SCOPE. Decide whether any specific primitive is worth shipping alongside AMM Phase 1 vs. deferring. SSF subtraction posture remains: justify every primitive.

---

## Open Questions (Honest Gaps)

1. Pool UTXO contention resolution priority — strictly fee-ordered or builder discretion?
2. TWAP accumulator persistence — where stored between blocks? Snap-sync-compatible?
3. `guards_activation_height = u64::MAX` on mainnet — when lowered? Escrow-loan depends on it.
4. Multi-pool routing — `pool_id` derivation rule? (Singleton per `pool_id` but multiple per pair?)
5. Protocol-fee Coin output (W2) — covered by `amm_activation_height` or needs separate treatment for block-content change?
