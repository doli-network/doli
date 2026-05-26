<!--
OUTPUT CONTRACT: N/A — tokenomics specification file (not a test file)
INPUT PARTITIONS: N/A — tokenomics specification file (not a test file)
-->

# DOLI Tokenomics

> **Status: DRAFT — populated from code.** Initial skeleton produced from the DeFi-subsystem economist review (2026-05-24, `docs/.workflow/defi-economic-review-2026-05-24-defi-subsystem.md`). On 2026-05-25, code-verified values were filled in for supply, halving, burn paths, and penalty routing (most TODOs resolved). Remaining `DECISION-PENDING` items are genuine policy / measurement gaps, not lookups — see § 9.
>
> **MUST-DO before lowering `amm_activation_height`.** The DeFi subsystem redesign (`specs/defi-subsystem-architecture.md`) introduces the first non-subsidy revenue stream for the producer reward pool (5 bps AMM protocol fee — W2). That change is not safe to activate without a published model of how it fits the chain's broader value-capture and security-budget design. This file fills that gap.
>
> **Source of truth precedent:** Per CLAUDE.md, code is SoT. Where this document conflicts with code, the code wins and this document is wrong. Each section cites the authoritative file:line so drift is detectable.

---

## 0. Why This Document Exists

The economist review verified by grep that the words **"tokenomics"**, **"protocol fee"**, and **"value capture"** appear ZERO times across:

- `specs/defi-subsystem-architecture.md` (before W2 edit)
- `docs/.workflow/architecture-reasoning.md`
- All 5 `docs/.workflow/design-*.md` evaluator reports

Five evaluators (Subtractionist, Restructurer, Pattern Matcher, Failure Analyst, Radical Simplifier) — none economic. The workflow optimized for "minimize new LOC while delivering native DeFi primitives" and never asked "how does DeFi success help DOLI?" This document forces that question and gives the team — and future economists/auditors — a place to find the answer.

---

## 1. Token Identity and Supply Mechanics

### 1.1 Native asset

- **Token:** DOLI (native consensus asset; not a smart-contract token)
- **Units:** 8 decimals (1 DOLI = 10^8 base units / "satoshis"). Source: `crates/core/src/types.rs` — `DECIMALS = 8`.
- **Supply cap:** `TOTAL_SUPPLY = 2_522_880_000_000_000` base units = **25,228,800 DOLI**. Source: `crates/core/src/consensus/constants.rs:426`.
- **Genesis allocation:** Framework in code via `GenesisProducer { pubkey, bond_count }` (`crates/core/src/chainspec.rs`); per-network actual addresses + amounts live in chainspec JSON (not in Rust source). **DECISION-PENDING:** mainnet genesis allocation breakdown by category (producer bootstrap / team / treasury / community) is not centrally documented and should be summarized here from the mainnet chainspec JSON before activation.

### 1.2 Issuance schedule

- **Slot duration:** 10 seconds. **Slots per year:** 3,153,600 (365 × 24 × 360). Source: `crates/core/src/consensus/constants.rs:183-185`.
- **Block subsidy:** `INITIAL_REWARD = 100_000_000` base units = **1.0 DOLI per block** (Era 0). Source: `crates/core/src/consensus/constants.rs:289`.
- **Era / halving:** `SLOTS_PER_ERA = HALVING_INTERVAL = 12_614_400` slots ≈ **4 years per era**. Reward formula: `block_reward(height) = INITIAL_REWARD >> era_index`. Source: `crates/core/src/consensus/constants.rs:205-212` and `crates/core/src/consensus/params.rs:223-230`.
- **Era 0 annual issuance:** 1 DOLI × 3,153,600 blocks/year = **3,153,600 DOLI/year** (~12.5% of TOTAL_SUPPLY in Era 0).
- **Era 1 (post-first-halving):** 0.5 DOLI/block × 3,153,600 = 1,576,800 DOLI/year (~6.25% of TOTAL_SUPPLY).
- **Asymptotic supply:** Converges to `TOTAL_SUPPLY` (25,228,800 DOLI). Verified by `crates/core/src/consensus/tests.rs::test_supply_converges`.
- **Tail emission:** ZERO. After **era 63** (height ≈ 12,614,400 × 63 ≈ 795M blocks ≈ 252 years), `INITIAL_REWARD >> 64+` is undefined and code caps reward at 0. Source: `consensus/params.rs:225`. **Implication for security budget:** post-Era-63 security depends entirely on protocol fees (AMM W2, future Phase 2/3 streams). Long horizon, but the model must be honest about it. See § 6.

### 1.3 Burn mechanisms

Native DOLI IS burned, in two paths (both 100%, no recipient):

1. **RequestWithdrawal vesting penalties.** Source: `crates/core/src/consensus/exit.rs:8-10, 100-118`. Penalty schedule: Y1 75%, Y2 50%, Y3 25%, Y4+ 0%. Penalty destination: `PenaltyDestination::Burn`. Penalty amount disappears from supply.
2. **Slashing on equivocation / double-sign.** Source: `crates/core/src/consensus/exit.rs:130-147`. `SlashResult.burned_amount = 100% of bond`; producer permanently excluded. Slashed bond disappears from supply.

**Net effect:** DOLI supply is monotonically non-decreasing from issuance only; burns from penalties/slashing make actual circulating supply LESS THAN cumulative issuance. The chain has a deflationary backstop tied to producer misbehavior.

**Cross-check:** `BurnAsset` (TX type 18) applies only to FungibleAsset (output type 7), NOT to native DOLI. Native DOLI cannot be burned by user TX; only by consensus-driven slash/penalty paths.

---

## 2. Value Inflows (Where DOLI Value Enters the Producer/Treasury System)

| Source | Status | Authoritative code path | Notes |
|--------|--------|------------------------|-------|
| Block subsidy (coinbase) | LIVE | `bins/node/src/node/rewards.rs` (`calculate_epoch_rewards`); reward-pool address derived in `crates/core/src/consensus/constants.rs:43-45` (`reward_pool_pubkey_hash()`) | Pool drained at epoch boundary, distributed bond-weighted to qualified producers. Era 0 inflow: 3,153,600 DOLI/year. |
| Producer bond slashing | LIVE | `crates/core/src/consensus/exit.rs:130-147` (SlashProducer, type 5) | **NOT an inflow — 100% burned** (`SlashResult.burned_amount`). Net effect: deflationary, supply leaves the system. |
| Bond withdrawal vesting penalty | LIVE | `crates/core/src/consensus/exit.rs:8-10, 100-118` (RequestWithdrawal, type 8) | **NOT an inflow — 100% burned** (`PenaltyDestination::Burn`). Y1 75% / Y2 50% / Y3 25% / Y4+ 0%. Deflationary. |
| **AMM protocol fee (5 bps)** | **PROPOSED (W2)** | `specs/defi-subsystem-architecture.md` § Fee Split | Phase 1 deliverable. Routes to canonical reward-pool address `BLAKE3("REWARD_POOL"\|\|"doli")` — **same address used by existing epoch coinbase**. Becomes the first non-subsidy producer revenue stream. |
| Future: Phase 2 lending interest spread | DEFERRED | N/A | If/when native lending ships, protocol can take a spread on interest |
| Future: Phase 3 NFT-frac mint/redeem fees | DEFERRED | N/A | |
| Future: Bridge fees | DEFERRED | N/A (bridge is separate workstream) | |

**Resolved:** No treasury distinct from the producer reward pool exists in code. Penalties and slashing go to burn (not treasury, not pool). All protocol revenue (subsidy + W2 fee) accrues directly to producers via the reward-pool flow. If a separate treasury is ever desired, it requires a new redesign cycle.

---

## 3. Value Outflows (Where DOLI Value Leaves the System)

| Sink | Status | Authoritative code path | Notes |
|------|--------|------------------------|-------|
| Producer epoch rewards | LIVE | `bins/node/src/node/rewards.rs` | Bond-weighted distribution to qualified producers; reduces pool to zero each epoch |
| Bond unbonding (with vesting penalty) | LIVE | `crates/core/src/transaction.rs` (RequestWithdrawal, type 8) + `crates/core/src/consensus/exit.rs:100-118` | 7-day delay + vesting penalty. **Penalty = BURNED** (PenaltyDestination::Burn). Principal returns to holder. |
| LP yield (AMM, 25 bps) | PROPOSED (Phase 1) | `specs/defi-subsystem-architecture.md` § D4 | Implicit via K-increase; not a TX-visible outflow |
| Future: Lending borrower interest | DEFERRED | N/A | Goes to lenders + protocol spread |
| Slash → burn | LIVE | `crates/core/src/consensus/exit.rs:130-147` (SlashProducer, type 5) | Net outflow — slashed bonds 100% disappear from supply |

---

## 4. Capital Allocation Game: Bond vs LP

> **The Cosmos/Osmosis problem** flagged by the economist as W8.

DOLI holders face an allocation choice:

| Option | Yield source | Currently routes to | Security impact |
|--------|-------------|---------------------|-----------------|
| Bond as producer | Block subsidy share + protocol fees | Holder | INCREASES validator security |
| Provide AMM liquidity | 25 bps × volume / TVL (LP K-increase) | Holder | NEUTRAL or NEGATIVE (capital leaves bond stack) |

Without W2 protocol fee, LP capture 100% of DEX growth, bond captures 0%. Strictly dominant equilibrium = capital migrates from bond to LP → security shrinks as DeFi grows. W2's 5 bps fix partially closes this loop by routing 5/30 of swap fees back to bonded producers.

**Required modeling:**
- At what DEX TVL does LP yield exceed bond yield in DOLI terms?
- What is the LP/bond yield ratio at the W2-spec 25/5 split vs. 30/0?
- Should DOLI ship LP-shares-as-bond-collateral (Phase 2+, Osmosis-style)?
- **Decision needed.**

---

## 5. Fee-Switch Policy

| Fee | Total | LP share | Protocol share | Mutable? | Governance? |
|-----|-------|---------|---------------|----------|-------------|
| AMM swap fee (W2) | 30 bps | 25 bps | 5 bps | NO (immutable per-pool after CreatePool) | None (per-pool choice at creation) |
| Phase 2 lending interest spread | TBD | N/A | TBD | TBD | TBD |
| Phase 3 NFT-frac mint/redeem | TBD | N/A | TBD | TBD | TBD |

**Policy principles** (proposed; require approval):

1. **Immutability over governance.** Fee rates are set at primitive creation and locked. Avoids governance attack surface (vote to set fees to zero) and reflexive sell pressure on rate-change proposals.
2. **Pool-level competition.** CreatePool is permissionless; different pools may choose different fee ratios within the 30 bps total. Market chooses which pools to use.
3. **No fee rebates.** Protocol fees are not refunded to traders or LPs via any rebate program. Adds complexity and creates governance attack surface.
4. **Routing target is canonical.** All protocol fees → `BLAKE3("REWARD_POOL"||"doli")`. No "treasury" sub-bucket in Phase 1. Future treasury allocation, if any, requires a separate redesign.

---

## 6. Era-by-Era Security Budget Model

> The single most important section. Without this, the chain has no defensible answer to "what funds your security after halving N?"

### Assumptions (code-verified — see § 1.2)

- **Initial block subsidy:** 1 DOLI/block (`consensus/constants.rs:289`)
- **Halving interval:** 12,614,400 slots ≈ 4 years (`consensus/constants.rs:205-212`)
- **Slots/year:** 3,153,600
- **Average producer count over Era 0:** DECISION-PENDING (chain is live; pull from `getProducers` RPC averaged over Era 0 to date)
- **Average bond size (median):** DECISION-PENDING (snapshot from current `ProducerSet`)

### Era 0 (current — subsidy-dominated)

- **Annual subsidy issuance:** 3,153,600 DOLI/year
- **In USD terms:** at DOLI = $X, that's $(3,153,600 × X)/year. Worked example at $1: $3.15M/year. At $10: $31.5M/year.
- **Burn offset:** Era 0 burns = slash events × bond size + early-unbond penalties. Real Era 0 net inflation < 3,153,600 DOLI/year by that amount. DECISION-PENDING to track this as a metric.
- **Protocol fee revenue:** $0 (no AMM activated yet)
- **Security budget ≈** annual subsidy − burns + slashing-deterrent value (bonds at risk)
- **Producer ROI** = subsidy share + 0 fees

### Era 1+ (post-first-halving, ~year 4)

- **Annual subsidy:** 1,576,800 DOLI/year (halved from Era 0)
- **Protocol fee revenue (W2 Phase 1):** 5 bps × AMM volume. At $10M/day across all pools × 365 = **$1.825M/year producer revenue from AMM** at DOLI price $1.
- **Break-even calc** (to keep Era-1 producer revenue ≥ Era-0 producer revenue at same DOLI price $X):
  - Era-0 producer revenue ≈ 3,153,600 × $X/year
  - Era-1 producer revenue from subsidy alone ≈ 1,576,800 × $X/year (half)
  - Gap to close = 1,576,800 × $X/year
  - Required AMM daily volume = (1,576,800 × $X) / 365 / 0.0005 = **~$8,635 × $X/day**
  - At $X = $1: $8,635/day. At $X = $10: $86,353/day. At $X = $100: $863,535/day.
- **Sustainability claim under test:** if AMM volume reaches the daily target above at the DOLI price prevailing at Era-1 boundary, the security budget keeps pace with Era 0 in DOLI terms. (Real-world reference: Uniswap v2 average mainnet daily volume circa 2024 ≈ $500M+; even a single mid-popularity pool clears the bar at $X ≤ $10.)

### Era N (asymptotic)

- **Subsidy → 0** after era 63 (~year 252)
- **Security budget = sum of all protocol fee streams**
- **Required protocol fee diversification:** AMM alone is not enough at asymptote. Phase 2 lending spread + Phase 3 NFT-frac + Bridge fees become necessary, not optional. Long horizon — but the model must be honest about it from day one.

**Action items (BLOCK before lowering `amm_activation_height`):**
1. Snapshot current producer count + median bond → fill the two DECISION-PENDING lines above
2. Adopt a target DOLI USD price for the Era 1 break-even calc; compute and publish the required daily AMM volume target
3. Treat that target as a published KPI

---

## 7. Reflexivity and Death-Spiral Resistance

| Reflexive loop | Trigger | Spiral direction | Mitigation |
|---------------|---------|-----------------|------------|
| Token price ↓ → bond value ↓ → security ↓ → confidence ↓ → price ↓ | Market crash | Negative | Fixed bond requirement in DOLI units (not USD); large bond floor; W2 fee independent of price |
| Emission-funded yield → token sell pressure → price ↓ → APY ↓ → flight | DOLI has NO emission-funded yield protocols (no liquidity mining proposed) | N/A | Avoid emission-funded LP rewards entirely. Real-yield-only policy. |
| Governance capture → fee-switch turned off → producers leave | DOLI fee fields are IMMUTABLE post-creation (W2 policy) | Pre-empted | No governance-mutable fee switch in Phase 1 |
| LP > bond yield → bond stack hollows out → security ↓ | LP-vs-bond capital migration (§ 4) | Negative | W2 routes 5 bps to bonds; full mitigation requires LP-as-bond (Phase 2+) |

---

## 8. Cross-References

| Document | Relevance |
|----------|-----------|
| `specs/defi-subsystem-architecture.md` | Phase 1 AMM with W2 fee split; this is the first source of non-subsidy producer revenue |
| `specs/protocol.md` | Authoritative TX/output type definitions, gating |
| `specs/security_model.md` | Validator security model (bond-weighted PoS) |
| `bins/node/src/node/rewards.rs` | Reward pool distribution (where protocol fees ultimately land) |
| `crates/core/src/network_params/` | All activation heights, including future tokenomics-relevant gates |
| `docs/.workflow/defi-economic-review-2026-05-24-defi-subsystem.md` | Economist review that surfaced this doc as a MUST-DO |
| `CLAUDE.md` | Constraints on changing consensus-relevant parameters (incl. fee fields, activation heights) |

---

## 9. Open Decisions (Required Before Activation)

| # | Decision | Status | Owner | Deadline |
|---|----------|--------|-------|----------|
| T1 | Confirm halving schedule + cumulative supply curve | **RESOLVED IN CODE** — `consensus/constants.rs:205-212`, `consensus/params.rs:223-230`, test `test_supply_converges`. 4-yr eras, 63 halvings, 25.2M cap. | — | — |
| T2 | Confirm bond-vesting-penalty routing | **RESOLVED IN CODE** — `consensus/exit.rs:100-118`. Penalty = 100% BURN. | — | — |
| T3 | Adopt "fee fields immutable post-CreatePool" as default policy | **DECISION-PENDING** — not implemented yet (no fee fields exist on Pool today). Ratify with W2 PR. | Architect + Economist | Before W2 ships in M2 |
| T4 | Compute + publish Era 1 break-even AMM volume target | **DECISION-PENDING** — formula in § 6; needs target DOLI price + producer-count snapshot | Economist | Before lowering `amm_activation_height` |
| T5 | LP-as-bond design (Cosmos/Osmosis pattern) | **DEFERRED** — Phase 2+; no code today | Architect | When Phase 2 redesign starts |
| T6 | Treasury vs producer-pool-only allocation | **RESOLVED IN CODE** — no treasury exists; reward pool is the only sink. Any change requires new redesign. | — | — |
| T7 | Tail-emission policy (zero vs non-zero post-final-halving) | **RESOLVED IN CODE** — zero after era 63 (`consensus/params.rs:225`). Changing this is a hard fork. | — | — |
| T8 | Genesis allocation breakdown (mainnet) — categorize and document | **DECISION-PENDING** — chainspec JSON exists but no narrative doc of which addresses serve which purpose (producer bootstrap / team / community / etc.) | Architect | Before next material redesign cycle |
| T9 | Snapshot current Era 0 producer count + median bond | **DECISION-PENDING** — pull from live `getProducers` RPC; record in this doc | Economist | Before lowering `amm_activation_height` |

---

## 10. What This Document Does NOT Decide

- DOLI's market positioning, marketing claims, or competitive narrative — out of scope
- Price predictions or target valuations — explicitly excluded (per `defi-economist` role boundaries)
- The structure or rate of future Phase 2/3 fee streams — those decisions belong in their respective redesign cycles
- Whether to ship a separate treasury smart contract — DOLI is UTXO-native; if a treasury is needed, it is a Multisig UTXO with documented signers, not a separate code path

---

## 11. Change Control

Tokenomics is consensus-relevant when:

- It documents activation heights, fee fields, or routing addresses that map 1:1 to consensus parameters
- A change to this doc that conflicts with code MUST be resolved by updating code OR updating this doc to match code (CLAUDE.md SoT precedent)
- A change to any policy in this doc that affects consensus (fee rates, immutability rules, routing addresses) MUST go through `/omega-redesign` and follow the three-question checklist (C7)

Non-consensus changes (narrative, modeling assumptions, era projections) may be updated freely with `/sync-docs` review.

---

## Appendix A: Verification Commands (Run Before Activation)

```bash
# 1. Verify reward-pool address derivation
grep -n "REWARD_POOL" crates/core/src/

# 2. Verify W2 fee fields exist in Pool metadata
grep -n "fee_bps_to_lp\|fee_bps_to_protocol" crates/core/src/

# 3. Verify CreatePool sum-invariant
grep -n "fee_bps_to_lp + fee_bps_to_protocol\|TOTAL_FEE_BPS" crates/core/src/validation/

# 4. Verify protocol-fee Coin routing in Swap apply
grep -n "REWARD_POOL\|protocol_fee" bins/node/src/node/apply_block/

# 5. Verify no governance setter on fee fields
grep -n "set_fee_bps\|update_fee" crates/

# 6. Compute Era 1 break-even fee volume
# (manual: subsidy_at_era_1_year_1 / 0.0005 / 365 = required_daily_volume_usd)
```

If any of these checks return unexpected results, this document is stale or the implementation diverged from spec. Resolve before activation.
