# DeFi Subsystem Redesign — Analyst Scoping

## Evidence Quality Notice

A stale `.diag_active` pipeline flag blocked direct source code reads for non-exempt paths (`crates/`, `bins/`) during the analyst pass. Evidence is assembled from:

- `specs/protocol.md` (marked `[SPEC]`)
- `specs/state-of-the-art-architecture.md` + `docs/redesigns/state-of-the-art-redesign-analysis.md` — built from direct codebase reading 2 days prior (marked `[PRIOR-VERIFIED]`)
- `specs/sdk-templates-requirements.md` + `specs/sdk-templates-architecture.md` (marked `[SPEC]`)
- `specs/l2-settlement.md` (marked `[SPEC]`)
- User-confirmed INC-I-088 Phase 0 state (marked `[USER-CONFIRMED]`)
- `CLAUDE.md` (marked `[CLAUDE.md]`)

Direct code-level line quotation for DEF-1 through DEF-5 could NOT be completed by the analyst. **The 5 design evaluators MUST perform their own code reads** for `validation/pool.rs`, `validation/lending.rs`, `validation/fractionalize.rs`, `apply_block/tx_processing.rs`, `apply_block/state_update.rs`, `transaction/types.rs`.

---

## 1. Strategic Posture (FIXED)

DeFi belongs on L1. A prior session proposed pushing DeFi to L2 via ZKSettle — the user REJECTED that. This redesign's goal is to make L1 DeFi production-grade so `defi_activation_height` can be lowered to a real future block via ProtocolActivation. Do not propose L2. Do not propose deprecation.

---

## 2. Current Shipped State (Greenfield Context)

INC-I-088 Phase 0 is shipped [USER-CONFIRMED]:

- `defi_activation_height` set to `u64::MAX` on BOTH mainnet AND testnet (pin reverted in `a196dd51`).
- 11 DeFi TX types hard-rejected with `[ERRTX-DEFI001]` when `current_height < defi_activation_height` [SPEC: protocol.md:1441-1451].
- `OutputType::Collateral` is in `is_conditioned()` [USER-CONFIRMED] (was D3 in prior architecture doc — now resolved).
- `verify_input_conditions` enforces the DeFi gate [USER-CONFIRMED].

Adjacent work already landed (available as building blocks):

- `b04a7e83`: covenant template functions (vault, escrow, htlc-payment, subscription, agent-allowance).
- `dfa54582`, `322be732`: CLI parsers + `doli template` subcommand for guard conditions.
- `c0efc39e`: E2E test for guard round-trip.

**Implication:** This redesign is GREENFIELD — no live DeFi UTXOs exist anywhere. No backward-compat constraints. Condition/guard SDK is now mature enough to make Decision 3 option (a) viable.

---

## 3. Capability Inventory

### 3.1 Transaction Types (31 defined)

| ID | Name | Category | DeFi? | Status |
|----|------|----------|-------|--------|
| 0 | Transfer | Value | No | Live |
| 1 | Registration | Producer | No | Live |
| 2 | Exit | Producer | No | Live |
| 3 | ClaimReward | Producer | No | DEPRECATED |
| 4 | ClaimBond | Producer | No | Live |
| 5 | SlashProducer | Governance | No | Live |
| 6 | Coinbase | Value | No | Live (consensus-gen) |
| 7 | AddBond | Producer | No | Live |
| 8 | RequestWithdrawal | Producer | No | Live |
| 9 | ClaimWithdrawal | Tombstone | No | Dead |
| 10 | EpochReward | Value | No | Live (consensus-gen) |
| 11 | RemoveMaintainer | Governance | No | Live |
| 12 | AddMaintainer | Governance | No | Live |
| 13 | DelegateBond | Delegation | No | Live (h=254344 gate) |
| 14 | RevokeDelegation | Delegation | No | Live |
| 15 | ProtocolActivation | Governance | No | Live |
| 17 | MintAsset | Assets | No | Live |
| 18 | BurnAsset | Assets | No | Live |
| 19 | **CreatePool** | **AMM** | **Yes** | **Gated u64::MAX** |
| 20 | **AddLiquidity** | **AMM** | **Yes** | **Gated u64::MAX** |
| 21 | **RemoveLiquidity** | **AMM** | **Yes** | **Gated u64::MAX** |
| 22 | **Swap** | **AMM** | **Yes** | **Gated u64::MAX** |
| 24 | **CreateLoan** | **Lending** | **Yes** | **Gated u64::MAX** |
| 25 | **RepayLoan** | **Lending** | **Yes** | **Gated u64::MAX** |
| 26 | **LiquidateLoan** | **Lending** | **Yes** | **Gated u64::MAX** |
| 27 | **LendingDeposit** | **Lending** | **Yes** | **Gated u64::MAX** |
| 28 | **LendingWithdraw** | **Lending** | **Yes** | **Gated u64::MAX** |
| 29 | **FractionalizeNft** | **NFT Frac** | **Yes** | **Gated u64::MAX** |
| 30 | **RedeemNft** | **NFT Frac** | **Yes** | **Gated u64::MAX** |
| 31 | ZKSettle | L2 | No | Gated u64::MAX (separate scope) |

### 3.2 Output Types (15 defined)

| ID | Name | DeFi? | `is_conditioned()`? | Purpose |
|----|------|-------|---------------------|---------|
| 0 | Normal | No | No | Standard spendable |
| 1 | Bond | No | Yes | Time-locked bond |
| 2 | Multisig | No | Yes | Threshold-N |
| 3 | Hashlock | No | Yes | Preimage |
| 4 | HTLC | No | Yes | Hash+time |
| 5 | Vesting | No | Yes | Sig+time |
| 6 | NFT | No | Yes | Non-fungible |
| 7 | FungibleAsset | No | Yes | User token |
| 8 | BridgeHTLC | No | Yes | Cross-chain |
| 9 | **Pool** | **Yes** | **Yes** [PRIOR-VERIFIED] | AMM reserves + TWAP |
| 10 | **LPShare** | **Yes** | **VERIFY** | LP receipt |
| 11 | **Collateral** | **Yes** | **Yes** [USER-CONFIRMED] | Lending collateral |
| 12 | **LendingDeposit** | **Yes** | **VERIFY** | Lending deposit receipt |
| 13 | ZKRollup | No | Expected | ZK rollup state |
| 14 | EncryptedContent | No | Unknown | Encrypted (activation-gated) |

**Open question for evaluators:** Are LPShare (10) and LendingDeposit (12) in `is_conditioned()`?

### 3.3 Condition/Guard Primitives Available

| Tag | Condition | Expresses | DeFi use |
|-----|-----------|-----------|----------|
| 0x00 | Signature | Single Ed25519 | Borrower/lender ID |
| 0x01 | Multisig | Threshold-N (≤127 keys) | Multi-party escrow |
| 0x02 | Hashlock | Preimage reveal | Atomic swap |
| 0x03 | Timelock (min_h) | "Cannot spend before H" | Time bound |
| 0x04 | TimelockExpiry (max_h) | "Only before H" | Loan/auction expiry |
| 0x10 | And | Both | Composition |
| 0x11 | Or | Either | Happy path OR timeout |
| 0x12 | Threshold | N-of-M conditions | Governance |
| 0x13 | **AmountGuard** | "spending TX output[i] ≥ X" | **Limit orders, min repayment** |
| 0x14 | **OutputTypeGuard** | "spending TX output[i] type T" | **Enforce collateral type** |
| 0x15 | **RecipientGuard** | "spending TX output[i] pays A" | **Enforce repay to lender** |

Bounds: MAX_CONDITION_DEPTH=4, MAX_CONDITION_OPS=128, MAX_MULTISIG_KEYS=127, MAX_THRESHOLD_CONDITIONS=5.

**Can express for DeFi:**
1. Lock-until-repayment: `And(Timelock(H), Signature(borrower))` — time-based only
2. Min repayment: `AmountGuard(loan+interest, idx)`
3. Repay-to-lender: `RecipientGuard(lender, idx)`
4. **Atomic collateral release**: `And(AmountGuard(repay, 0), RecipientGuard(lender, 0))` — collateral spendable iff spending TX pays lender ≥ repayment
5. Liquidation OR repayment: `Or(<repay condition>, And(Signature(liquidator), TimelockExpiry(deadline)))`

**Cannot express:**
1. Oracle price verification (conditions only see `ctx.tx.outputs`, not external data or chain state)
2. Cross-UTXO state queries
3. Arithmetic beyond `≥` on amount (no ratios, division, percentages)
4. Dynamic parameters (all fixed at UTXO creation time)

**Implication for Decision 3:** Condition language can enforce happy-path repayment. CANNOT replace oracle-informed liquidation.

### 3.4 Producer/Consensus Primitives Reusable for Oracle

| Primitive | Reusability |
|-----------|-------------|
| BLS12-381 attestation aggregation | **High** — producers already have BLS keys signing every block; a PriceUpdate TX can use the same crypto path |
| Attestation bitfield | Medium — could track price-posting producers (but consensus-visible, C5/C7) |
| Slashing pattern (`SlashProducer`/`SlashingEvidence`) | Low direct, high pattern — well-established detect-misbehavior→burn-bonds |
| Bond-weighted producer set | Medium — bigger bonds → more loss from manipulation |
| Epoch-boundary processing | Low — could aggregate per epoch but adds critical-path complexity |

**Most directly reusable:** BLS keypair + attestation crypto. A `PriceUpdate` TX signed by producer BLS keys verifies through the same path as attestations.

**What does NOT exist:** Zero oracle code. No `PriceUpdate` TX. No `price_feed` or `oracle` module. The Pool TWAP (`cumulative_price`, `last_slot` in Pool metadata) is internal AMM accounting, NOT an external oracle.

### 3.5 DeFi Test Coverage Audit

| File | Tests | Status |
|------|-------|--------|
| `validation/pool.rs` | UNVERIFIED — evaluators MUST count | User: zero for AddLiq/RemoveLiq |
| `validation/lending.rs` | UNVERIFIED — evaluators MUST count | User: zero for LiquidateLoan; prior analysis: LiquidateLoan is "2-check shell" at `lending.rs:133-152` |
| `validation/fractionalize.rs` | UNVERIFIED | Spec has NO fractionalization section — circumstantial evidence of thin tests |
| `apply_block/tx_processing.rs` (DeFi branches) | UNVERIFIED | |
| `apply_block/state_update.rs` (DeFi state) | UNVERIFIED | |
| `bins/cli/src/cmd_pool.rs` | Exists, uses `set_covenant_witnesses` [PRIOR] | |

---

## 4. Confirmed Defects (Evidence)

**Evaluators MUST quote 3-5 lines from each cited file:line directly.**

### DEF-1: LiquidateLoan is a 2-check shell

- `lending.rs:133-152` per user prompt + state-of-the-art-architecture.md:29, 131, 157
- Spec [protocol.md:842] says "When collateral value falls below the liquidation threshold, anyone can liquidate" but provides NO valuation mechanism
- Missing: collateral ratio calc, price source, liquidation bonus, partial liquidation, interest accrual verification
- **Status: CONSISTENT** — three independent references confirm

### DEF-2: Zero oracle code; lending has no price source

- Grep "oracle"/"price_feed"/"PriceUpdate" across specs → zero matches
- `protocol.md:832` Pool `cumulative_price (TWAP)` is the ONLY price-related structure — internal AMM accounting, not wired to lending
- `protocol.md:842` LiquidateLoan spec gap — does not specify HOW value is determined
- **Status: CONSISTENT** — no oracle exists

### DEF-3: Zero tests for AddLiquidity/RemoveLiquidity in AMM

- `protocol.md:824-832` documents AddLiq/RemoveLiq/Swap but no test vectors
- `state-of-the-art-architecture.md:159` — "11 TX types unreachable until E2E testing complete" — testing was deferred
- **Status: UNVERIFIED but plausible** — evaluators must count tests

### DEF-4: Zero tests for FractionalizeNft/RedeemNft + no redemption design

- Protocol spec has NO fractionalization section (types 29/30 only mentioned in gating paragraph at 1444)
- `docs/cli.md` — no fractionalization commands
- **Status: UNVERIFIED but HIGHLY PLAUSIBLE** — complete spec/doc absence is strong evidence

### DEF-5: Pool/Collateral/LP outputs carry semantic state in `extra_data` (state-machine, not condition-based)

- Per user prompt — `transaction/types.rs` output types 9-12 store reserves/loan terms/share counts in `extra_data`
- Collateral was added to `is_conditioned()` (INC-I-088, D3) — but the actual condition placed on Collateral UTXOs is unverified
- Section 3.3 shows the condition language IS now expressive enough for happy-path collateral lock, but CANNOT do oracle-informed liquidation alone
- **Status: design choice, not bug** — core tension for Decision 3

---

## 5. The 6 Open Design Decisions

### Decision 1: ORACLE ARCHITECTURE

**(a) Producer-posted PriceUpdate TX** — BLS infra exists; new TX type needed; producers have skin in game; risk = collusion (34 producers)
**(b) AMM TWAP** — already partially built in Pool metadata; risk = sandwich attacks, bootstrapping (no pools → no TWAP), liquidity required
**(c) Liquidator-provided proof** — condition language can't verify arbitrary external signatures (only Ed25519); centralization risk
**(d) Hybrid (a+b)** — most robust, most complex

Constraints: All trigger C7. (a) and (c) need new TX types (C5). (b) and (d) need deterministic TWAP rebuild (C2).

### Decision 2: LIQUIDATION MODEL

**(a) Permissionless + bonus** — standard DeFi (Aave/Compound); MEV/mempool ordering risk
**(b) Keeper-restricted to bonded producers** — aligns with DOLI's producer-centric trust; risk = producer offline/collusion
**(c) Auction** — best price discovery; no auction infra exists; latency in 10s-slot system

### Decision 3: COLLATERAL SPENDING MODEL

**(a) Condition-based** `And(AmountGuard(repay, 0), RecipientGuard(lender, 0))` for happy path; `Or(<repay>, And(Signature(liquidator), TimelockExpiry(deadline)))` for liquidation — deterministic by construction; CANNOT enforce price-triggered liquidation alone (needs Decision 1 oracle)
**(b) State-machine via lending state (current)** — can express complex business logic; current impl is shallow shell
**(c) Multisig (borrower + lender 2-of-2)** — incompatible with autonomous liquidation; deadlock risk

### Decision 4: AMM CURVE STRATEGY

**(a) Constant-product only** — simplest; battle-tested; poor for correlated assets
**(b) Add stableswap** — better for stables; complex fixed-point invariant (`A * sum(x_i) + D = A * D + D^(n+1) / (n^n * prod(x_i))`); determinism risk (C2)
**(c) Pluggable curve registry** — most future-proof; most complex; each curve needs activation height

### Decision 5: NFT REDEMPTION MODEL

**(a) Share-buyback** — market-driven; holdout problem
**(b) Forced auction** — needs auction infra; may force sale below fair value
**(c) Time-locked unanimity** — `Threshold(N, [Sig…])` capped at MAX_THRESHOLD_CONDITIONS=5; deadlock risk
**(d) Fractional governance** — most flexible; most complex; governance is notoriously hard

### Decision 6: ACTIVATION SEQUENCING

**(a) Single `defi_activation_height`** — current; simplest; coupling risk (one bug delays all 11)
**(b) Per-primitive heights** (`amm_activation_height`, `lending_activation_height`, `nft_frac_activation_height`) — decoupled; pattern well-established in DOLI (11 distinct heights already); more testing combinations; oracle dependency if (1b) chosen

---

## 6. Hard Constraints

| ID | Constraint |
|----|-----------|
| C1 (INC-I-054) | Activation heights IMMUTABLE once crossed. New features = NEW heights. |
| C2 (INC-I-082) | Online apply_block and offline rebuild must be bit-identical. |
| C3 (Pillar) | Encoder/decoder index parity across ALL consumers. |
| C4 (INC-I-016) | Local-state HashSets modifying scheduler inputs MUST be capped. |
| C5 (INC-I-062) | Block-content changes require synchronized deploy. |
| C6 | Producer mutations deferred to epoch boundary. |
| C7 (INC-I-075) | 3-question checklist in commit. If (1)+(2)=YES, (3)=NO → activation height REQUIRED. |
| UTXO | All DeFi state in UTXOs. No accounts. Pool UTXOs are shared resources by design. |
| Protocol version | DO NOT bump `CURRENT_PROTOCOL_VERSION` unless EpochState format changes. |
| #0 | NO GENESIS RESETS. Forward-only activation. |

---

## 7. In Scope / Out of Scope

**In scope:**
- 11 DeFi TX types (CreatePool, AddLiq, RemoveLiq, Swap, CreateLoan, RepayLoan, LiquidateLoan, LendingDeposit, LendingWithdraw, FractionalizeNft, RedeemNft)
- Output types 9-12 (Pool, LPShare, Collateral, LendingDeposit)
- `validation/pool.rs`, `validation/lending.rs`, `validation/fractionalize.rs`
- `apply_block/tx_processing.rs` (DeFi branches), `apply_block/state_update.rs` (DeFi state)
- Oracle infrastructure (per chosen design)
- `NetworkParams` additions for new activation heights / oracle params
- Test coverage: property-based + adversarial + E2E for every primitive
- CLI exposure (`cmd_pool.rs`, lending CLI, NFT CLI)
- New docs: `docs/defi-architecture.md`, `specs/defi-protocol.md`

**Out of scope:**
- ZKSettle / L2
- Node god-object refactor
- Wallet unification
- Producer-set decentralization
- Scheduler, attestation, rewards, VDF, network layer (non-DeFi consensus)
- Existing crossed activation heights (immutable)
- `CURRENT_PROTOCOL_VERSION`
- `defi_activation_height` itself — leave at `u64::MAX` until implemented + audited

---

## 8. Acceptance Criteria for the Redesign Proposal

The synthesizer must hit every box:

- [ ] One chosen answer (with trade-off analysis) for each of the 6 design decisions
- [ ] Concrete fix spec for each of DEF-1 through DEF-5
- [ ] Activation sequencing plan (which primitive ships 1st, 2nd, 3rd)
- [ ] Test strategy per primitive (property tests, adversarial, testnet scenarios)
- [ ] Oracle architecture spec (if option requires new TX/producer behavior)
- [ ] Collateral spending model spec — exactly how Collateral UTXO becomes spendable, by whom, under what conditions
- [ ] Liquidation flow diagram — who initiates, what proof required, where bonus goes
- [ ] Risk register — what can go wrong with each chosen design + mitigation
- [ ] Migration path for any existing UTXOs (greenfield — none exist, but verify)
- [ ] Estimated effort per primitive (weeks) + total timeline
- [ ] Audit-readiness checklist — what an external auditor needs

---

## 9. Architectural Map

**Modules (direct impact):**
- `crates/core/src/validation/{pool,lending,fractionalize}.rs`
- `crates/core/src/transaction/types.rs` (possibly new types)
- `bins/node/src/node/apply_block/{tx_processing,state_update}.rs`
- `crates/core/src/network_params/` (new activation heights)

**Modules (indirect impact):**
- `crates/core/src/conditions/` (if new condition types needed)
- `crates/mempool/` (DeFi TX mempool validation)
- `crates/rpc/src/methods/{pool,lending}.rs`
- `bins/cli/src/cmd_pool.rs` + new lending/NFT CLI
- `crates/storage/src/{utxo,utxo_rocks}.rs` (if new output type fields)
- `crates/core/src/validation/transaction.rs` (dispatch)

**No impact (invariants preserved):**
- Consensus engine (scheduler, epochs, rewards)
- Network layer (gossip, sync)
- Storage layer (block_store, state_db)
- Producer set management
- Attestation system

**Data flow:**
```
User → CLI/RPC → Mempool validate → BlockBuilder selects → apply_block/tx_processing.rs
  → validation/{pool,lending,fractionalize}.rs (defi gate + structural + DeFi rules)
  → apply_block/state_update.rs (UTXO mutations)
  → state root hash (must match across all nodes, C2)
```

---

## 10. Redesign Acceptance Criteria — "Better" Looks Like

**Must (non-negotiable):**
- All non-DeFi consensus rules untouched (scheduler, epoch boundaries, state root, producer set)
- Deterministic validation (C2): bit-identical results on every node — especially oracle price computation, interest accrual
- Activation gating via standard mechanism (no genesis reset — #0)
- Three-question checklist in every commit (C7)
- CURRENT_PROTOCOL_VERSION NOT bumped (unless EpochState format changes)
- All 11 DeFi TX types have production-grade validation — no 2-check shells

**Should:**
- Leverage condition/guard SDK for collateral enforcement where possible
- Per-primitive activation heights (decouple AMM/lending/NFT readiness)
- Comprehensive test coverage (positive + negative + edge + economic attack vectors)
- Oracle architecture reuses producer trust model (no new trust assumptions)
- Protocol spec updated with full fractionalization + oracle/price sections

**Could:**
- Stableswap option for correlated assets
- Full CLI/RPC for all DeFi operations
- TWAP manipulation protection (min observation window)

**Won't:**
- L2 migration (rejected)
- ZKSettle activation (separate scope)
- Node binary restructuring
- Account-based state
- Turing-complete smart contracts
- Cross-chain oracle integration
- Governance token

---

## What I Don't Understand (Intellectual Honesty)

1. `LPShare` and `LendingDeposit` `is_conditioned()` membership — unverified
2. Current Collateral condition content — what condition is placed by `CreateLoan`?
3. Pool UTXO contention — when multiple Swaps target same pool same block, how is contention resolved?
4. Interest accrual timing — per-block, per-epoch, or at repayment?
5. FractionalizeNft share denomination — as FungibleAsset(type 7) or new type?
6. Per-module line count breakdown for DeFi validation

---

## Spec Drift Detected

1. `specs/protocol.md` TX enum comment (126-139): missing types 29, 30, 31
2. `specs/protocol.md` output type table (175): says "13 total" but missing 13 (ZKRollup), 14 (EncryptedContent)
3. `specs/protocol.md`: no section 3.24 for FractionalizeNft/RedeemNft format/rules

---

**Stale pipeline flag:** `.claude/hooks/.diag_active` (PID 78953) may need removal if evaluators are blocked from source reads.

---

## Postscript — 2026-05-25 Code Verification

The original analysis (above) was produced under a stale pipeline-gate flag that blocked direct source reads. Several items were tagged `UNVERIFIED` / `UNABLE TO VERIFY` / `evaluators MUST do their own reads`. Downstream evaluators ran in sub-agent processes that bypassed the gate; subsequent code-reads (Explore agent + DeFi economist) verified or refuted the open items. This postscript records the resolved facts so future readers don't re-do the work.

### Resolved items (originally `UNVERIFIED` or `UNABLE TO VERIFY`)

| Original gap | Status | Source |
|---|---|---|
| LPShare (10) in `is_conditioned()`? | **NOT in `is_conditioned()`** — CONFIRMED. Must be added before AMM activation (R6 in `specs/defi-subsystem-architecture.md`) | Failure Analyst code-read; Synthesizer R6 |
| LendingDeposit (12) in `is_conditioned()`? | **NOT in `is_conditioned()`** — but unused in Phase 1. R7. | Failure Analyst code-read |
| Collateral condition content (what `CreateLoan` places) | INC-I-088 Phase 0 confirmed Collateral IS in `is_conditioned()`; condition is currently a placeholder pending Phase 2 native lending. Phase 1 escrow-loan template uses the `Or(And(AmountGuard, RecipientGuard), And(Signature, Timelock))` tree (`specs/defi-subsystem-architecture.md` § D3) | User-confirmed + spec |
| DeFi validation test counts (DEF-3, DEF-4) | Confirmed thin/zero — DEF-3 (AMM AddLiq/RemoveLiq) and DEF-4 (NFT-frac) addressed in milestone M3 test suite | Synthesizer + Failure Analyst |
| Pool UTXO contention resolution | Block builder priority + mempool warning; documented as inherent L1-AMM limit | DeFi Economist Q3 |
| FractionalizeNft share denomination | DEFERRED to Phase 3 — Phase 1 uses compose-via-Multisig(127) + MintAsset/BurnAsset (existing primitives) | Synthesizer D5 |
| Per-module DeFi validation line count | ~2,959 lines total (cited in prior state-of-the-art analysis); per-module breakdown deferred until refactor in M1 | Prior verified analysis |

### Newly discovered facts (NOT in original scoping)

| Fact | Source | Implication |
|---|---|---|
| `TOTAL_SUPPLY = 25,228,800 DOLI` (8 decimals) | `consensus/constants.rs:426` | Supply cap is fixed and final |
| `INITIAL_REWARD = 1 DOLI/block`, halving every `SLOTS_PER_ERA = 12,614,400` (~4 years) | `consensus/constants.rs:289`, `205-212` | Era 0 issuance = 3.15M DOLI/year |
| Tail emission = 0 after era 63 (~year 252) | `consensus/params.rs:225` | Post-Era-63 security depends entirely on protocol fees |
| Bond vesting penalty + slashing = 100% BURN (no recipient) | `consensus/exit.rs:100-118, 130-147` | DOLI has a deflationary backstop on producer misbehavior |
| No treasury exists distinct from producer reward pool | grep — none found | All protocol revenue routes to producers; no separate treasury allocation possible in current code |
| `reward_pool_pubkey_hash()` = `BLAKE3("REWARD_POOL" \|\| "doli")` | `consensus/constants.rs:43-45` | W2 AMM protocol-fee routing target verified to match existing coinbase routing |

### Spec drift confirmed (already noted; left for separate sync)

- `specs/protocol.md` TX enum comment missing types 29, 30, 31 — confirmed
- `specs/protocol.md` output type table missing 13 (ZKRollup), 14 (EncryptedContent) — confirmed
- `specs/protocol.md` has no section 3.24 for FractionalizeNft / RedeemNft — confirmed; will be addressed when Phase 3 ships

### Downstream artifacts spawned from this analysis

- `specs/defi-subsystem-architecture.md` — synthesized spec (the deliverable)
- `specs/tokenomics.md` — drafted 2026-05-25 to fill the value-capture gap surfaced by the DeFi economist review
- `docs/.workflow/defi-economic-review-2026-05-24-defi-subsystem.md` — economist review (W1-W10 findings, Verdict: APPROVE-WITH-SIGNIFICANT-CHANGES)
- `docs/.workflow/architecture-reasoning.md` — synthesizer reasoning trace

### What this postscript does NOT change

The body of this analysis above is preserved as the original snapshot. It documents what the analyst knew at the time and serves as the input that downstream evaluators received. Modifying the body would erase the reasoning provenance.
