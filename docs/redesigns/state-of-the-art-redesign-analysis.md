# DOLI State-of-the-Art Redesign — Analyst Scoping Document

**Workflow**: `/omega-redesign` (proposal-only, no `--fix`)
**Date**: 2026-05-22
**Source material**: DOLI vs. Web3 Pain Stack comparative analysis (conversation context)

> **Status note (2026-05-25):** The DeFi-on-L1 vs DeFi-on-L2 question this analysis surfaced was resolved 2026-05-24 in favor of L1. See `docs/redesigns/defi-subsystem-redesign-analysis.md` and `specs/defi-subsystem-architecture.md`. The DeFi scope of this analysis is SUPERSEDED; the ZKSettle / Condition SDK / competitive-positioning scope remains authoritative.

---

## 1. Capability Inventory (Verified Baseline)

Built from codebase reading, not docs. Every item verified against source.

### Consensus Primitives (Live on Mainnet)

| Primitive | Implementation | Status |
|-----------|---------------|--------|
| Iterated hash-chain delay proof (BLAKE3, T=1000) | `consensus/vdf.rs` | Live |
| Deterministic bond-weighted round-robin scheduler | `scheduler.rs` | Live |
| `missed_producers` exclusion + re-entry slots | `scheduler.rs`, `consensus/constants.rs` | Live |
| BLS12-381 attestation aggregation | `attestation.rs` | Live |
| Epoch-boundary producer set freeze | `epoch_state/mod.rs` | Live |
| 3-epoch lookback liveness filter | `consensus/constants.rs` | Live |
| Ghost producer exclusion (h>=18152) | `network_params/defaults.rs` | Live |
| Weight-based fork choice (seniority 1.0-4.0) | Whitepaper S9.1 | Live |
| Tier system (50 active producers cap) | `TIER_SYSTEM_ACTIVATION_HEIGHT=0` | Live |
| Tier promotion by attestation count | `TIER_PROMOTION_ACTIVATION_HEIGHT=0` | Live |
| Inactivity leak (10%/epoch after 1 epoch missed) | `consensus/constants.rs` | Live |

### Transaction Types (27 defined, 26 active)

| Category | Types | Status |
|----------|-------|--------|
| Value transfer | Transfer, Coinbase, EpochReward | Live |
| Producer lifecycle | Registration, Exit, AddBond, RequestWithdrawal, ClaimBond, ClaimReward, SlashProducer | Live |
| Governance | AddMaintainer, RemoveMaintainer, ProtocolActivation | Live |
| Delegation | DelegateBond, RevokeDelegation | Live (auth gate at h=254344) |
| Assets | MintAsset, BurnAsset | Live (structural validation, no activation gate) |
| AMM/DeFi | CreatePool, AddLiquidity, RemoveLiquidity, Swap | Live (no activation gate) |
| Lending | CreateLoan, RepayLoan, LiquidateLoan, LendingDeposit, LendingWithdraw | Live (no activation gate) |
| NFT | FractionalizeNft, RedeemNft | Live (no activation gate) |
| L2 Settlement | ZKSettle | **Gated at `u64::MAX` — requires ProtocolActivation** |
| Tombstone | ClaimWithdrawal (type 9) | Dead (wire compat only) |

### Output Types (15 defined)

| Type | ID | Status |
|------|-----|--------|
| Normal | 0 | Live |
| Bond | 1 | Live |
| Multisig | 2 | Live |
| Hashlock | 3 | Live |
| HTLC | 4 | Live |
| Vesting | 5 | Live |
| NFT | 6 | Live |
| FungibleAsset | 7 | Live |
| BridgeHTLC | 8 | Live (6 chains: BTC/ETH/XMR/LTC/ADA/BSC) |
| Pool | 9 | Live |
| LPShare | 10 | Live |
| Collateral | 11 | Live |
| LendingDeposit | 12 | Live |
| ZKRollup | 13 | **Live structurally; ZKSettle TX gated** |
| EncryptedContent | 14 | Live |

### Condition Language (Programmable Spending — NOT Turing-complete)

| Condition | Tag | Status |
|-----------|-----|--------|
| Signature | 0x00 | Live |
| Multisig (threshold-of-N) | 0x01 | Live |
| Hashlock | 0x02 | Live |
| Timelock (min_height) | 0x03 | Live |
| TimelockExpiry (max_height) | 0x04 | Live |
| And | 0x10 | Live |
| Or | 0x11 | Live |
| Threshold (n-of-m conditions) | 0x12 | Live |
| **AmountGuard** (min output amount) | 0x13 | Live — covenant primitive |
| **OutputTypeGuard** | 0x14 | Live — covenant primitive |
| **RecipientGuard** | 0x15 | Live — covenant primitive |

Bounds: MAX_CONDITION_DEPTH=4, MAX_CONDITION_OPS=128, MAX_MULTISIG_KEYS=127, MAX_THRESHOLD_CONDITIONS=5.

**Critical insight**: DOLI already has covenant-like output introspection (the guard conditions). The spending transaction's outputs can be constrained declaratively. This is what Bitcoin's covenant proposals (OP_CTV, OP_CAT) are trying to add — DOLI has them today.

---

## 2. Current Architecture Map

### Core Data Flow
```
TX submitted → mempool → producer selects → BlockBuilder → VDF → broadcast
All nodes: receive → validate_block() → apply_block() → 3 states updated → state_root cached
Epoch boundary: producer set frozen from 3-epoch lookback → rewards distributed
```

### Three States (consensus-critical, must converge across all nodes)
- `ChainState`: height, best_hash, slot, genesis_time
- `UtxoSet`: every unspent output (in-memory + RocksDB)
- `ProducerSet`: registered producers, bonds, delegations, pending updates

### Blast Radius of Any Redesign Change
- **New TX types**: `transaction/types.rs` + `validation/transaction.rs` + `validation/tx_types.rs` + `apply_block/tx_processing.rs` + `apply_block/state_update.rs` + mempool + RPC + CLI
- **New output types**: `transaction/types.rs` + `transaction/output.rs` + `validation/utxo.rs` + `storage/utxo.rs` + `storage/utxo_rocks.rs`
- **Scheduler changes**: `scheduler.rs` + `epoch_state/mod.rs` + `apply_block/post_commit.rs` + `rewards.rs`
- **Condition language extensions**: `conditions/mod.rs` + `conditions/encoding.rs` + `conditions/eval.rs`

### Invariants That Must Be Preserved (Non-Negotiable)
1. VDF deterministic consensus: `slot % total_tickets` produces the same producer on every node.
2. UTXO model: no shared mutable state. Every output is independent.
3. `missed_producers` list: local state HashSet modifying scheduler inputs MUST be capped (INC-I-016).
4. Epoch-boundary producer set freeze: mutations DEFERRED to epoch boundary, never mid-epoch.
5. State root convergence: all 3 states must produce identical state root across all nodes.
6. Activation heights: once crossed on mainnet, IMMUTABLE. New features get new heights.

---

## 3. Gap Re-Analysis (Report Claims vs. Code Reality)

| Report Gap | Status After Code Reading |
|------------|--------------------------|
| A1: "No arbitrary application logic" | **Partially false.** DOLI has 27 TX types + 15 output types + 11 condition primitives + native AMM/lending/NFT. The real gap is "no novel state machines beyond these." ZKSettle is the escape hatch. |
| A2: UTXO concurrency | **Real but reframed.** Pool UTXOs are shared resources — by design, same as Sui's shared objects. This is the MEV-resistance feature, not a bug. |
| A3: Cross-chain general messaging | **Real.** BridgeHTLC = atomic swaps. No IBC/XCMP equivalent. L2 would be the path. |
| A4: Bootstrap decentralization (5 nodes, 34 producers) | **Real.** Acknowledged by design. ProtocolActivation TX type is the mechanism. |
| A5: Wallet UX | **Implementation gap, not architectural.** GUI spec exists. |
| A6: VDF adversarial bounds | **Real but framed honestly in whitepaper.** T=1000 is anti-grinding, not time-proving. Bond is real Sybil defense. |
| A7: Auditor pool | **Real.** No published independent audit. |
| A8: Tokenomics/governance opacity | **False per code.** Whitepaper documents 25.2M supply, halving, no premine, 3/5 maintainer multisig. Communication gap, not transparency gap. |
| A9: Operational track record | **Real but young.** ~254K+ blocks, 10+ documented incidents. |
| B1: Hard-fork for primitives | **Real but bounded.** ZKSettle activation removes the bound. |
| B2: 34-producer collusion surface | **Real.** Mitigated by seniority weighting + tier system. |
| B3: VDF performance assumptions | **Real, documented.** |
| B4: Storage doubling (512KB → 8MB) | **Real, deterministic.** |
| B5: AMM curve lock-in | **Real.** Constant-product only. |
| B6: No L2 ecosystem | **The single most leveraged gap.** ZKSettle exists but is unactivated. |

---

## 4. The Sui/Aptos/Move Critique — DOLI's Actual Position

Move's innovation: **safety-by-construction via linear types**. Resources can't be copied or dropped.

DOLI's counter-position: **safety-by-elimination + safety-by-declaration**. No programs to write incorrectly. All validation rules are compiled Rust. Spending policies are declarative conditions with bounded evaluation cost.

What Move/Sui enables that DOLI L1 cannot:
1. Novel auction mechanisms (custom state machines)
2. Complex DAO governance beyond threshold multisig
3. Novel DeFi instruments not anticipated by maintainers
4. On-chain games with persistent shared state

**DOLI's answer**: ZKSettle (TX type 31, gated at `u64::MAX`). Any computation that doesn't fit native types runs on an L2 that posts ZK proofs to DOLI's UTXO chain. The verifying key lives in the UTXO — permissionless by construction.

**The risk**: if ZKSettle stays at `u64::MAX` indefinitely, DOLI has NO escape hatch and the Move critique lands.

---

## 5. Acceptance Criteria for the Redesign

### Must (preservation constraints)
- Preserve VDF deterministic consensus
- Preserve all crossed activation heights (mainnet has crossed — IMMUTABLE)
- Preserve UTXO model semantics
- Preserve 3-state convergence guarantee

### Should (gap-closing structural improvements)
- Address A3 (cross-chain messaging) and B6 (no L2) with a concrete activation path
- Address A1/B1 (programmability vs. hard-fork dependency) via the L2 escape hatch
- Address A7 (auditor pool) with audit-ready documentation
- Address A2/B5 (AMM concurrency + curve lock-in) with concurrency-aware design or explicit positioning
- Address B2 (producer collusion) with a concrete producer-set growth roadmap

### Could
- Surface the condition language guards in user-facing tooling (CLI/GUI)
- Document DOLI's position vs. Move-based chains explicitly
- Define a clean "what DOLI is and isn't" competitive narrative

### Won't
- Migrate to account-based state
- Abandon UTXO
- Introduce probabilistic finality
- Ship a Solidity-compatible VM
- Reuse or move existing activation heights

---

## 6. Requirements (REQ-SOTA-NNN)

| ID | Title | Priority | Hard Fork? | Blast Radius |
|----|-------|----------|-----------|--------------|
| REQ-SOTA-001 | Select & commit to a ZK proof system | Must | No (until activation) | `validation/zk.rs` |
| REQ-SOTA-002 | Activate ZKSettle at concrete future height via ProtocolActivation | Must | Yes (rolling deploy via constant gate) | `consensus/constants.rs`, `network_params/defaults.rs` |
| REQ-SOTA-003 | Build reference L2 demonstrating settlement path | Must | No | New crate or external repo |
| REQ-SOTA-004 | Expose condition language guards in CLI | Should | No | `bins/cli/src/` |
| REQ-SOTA-005 | Publish formal security model as standalone audit-ready doc | Should | No | `specs/security_model.md` |
| REQ-SOTA-006 | E2E test all DeFi primitives on testnet (currently untested in practice?) | Should | No | `tests/`, `bins/cli/src/` |
| REQ-SOTA-007 | Publish competitive analysis vs. Move/Sui/Aptos | Could | No | `docs/` |
| REQ-SOTA-008 | Producer-set growth roadmap (target N producers by date) | Should | No | `docs/`, ops |
| REQ-SOTA-009 | IBC/XCMP equivalent for general cross-chain messaging | Won't | — | — |
| REQ-SOTA-010 | On-chain governance beyond maintainer multisig | Won't | — | — |

---

## 7. Notes for Design Evaluators

1. **The "no programmability" framing in the comparative analysis report was undercalibrated.** DOLI's condition language with guards is closer to Bitcoin-with-MAST-and-covenants than to "scriptless Bitcoin." The Restructurer and Pattern Matcher should both engage with this.

2. **ZKSettle is the central architectural lever.** The Radical Simplifier should consider: "what if the redesign is ENTIRELY about activating ZKSettle and shipping one L2?" That is the SSF candidate.

3. **DeFi TX types may be untested in practice.** AMM/lending/fractionalization have no activation height gate — they appear live from genesis. Whether anyone has actually created a Pool on mainnet is unknown. The Failure Analyst should treat any "DOLI already has X" claim as conditional on E2E verification.

4. **Storage doubling (B4) is deterministic but not free.** Per-era growth from 512KB to 8MB caps per-node storage cost growth. Evaluators should weigh whether to propose pruning/archival-node separation now or defer.

5. **B2 (producer collusion) is the most quantifiably-weak axis.** 34 producers. Any redesign must include a producer-set growth path or it will be torn apart by external decentralization critics.

6. **Move/Sui linear types ≠ DOLI's condition guards.** The honest comparison is: Move gives developers safe programmability; DOLI gives developers safe declarability + an L2 escape hatch. The evaluators should NOT propose adopting Move — they should sharpen DOLI's distinct position.

7. **Hard constraint reminder**: VDF deterministic consensus is non-negotiable. Any proposal that introduces validator-decided ordering or probabilistic finality is invalid.

---

## 8. Open Questions for Design Evaluators

- Should ZKSettle's proof system be Groth16 (mature, small proofs, trusted setup per circuit), PLONK (universal trusted setup), or STARK (no trusted setup, larger proofs)?
- Should DOLI ship its OWN reference L2 (canonical sequencer) or define the L2 interface and let third parties build?
- Should the AMM curve become pluggable (multiple invariants) at the protocol level, or should novel curves be relegated to L2?
- Is the producer-set growth path bond-threshold-based, geographic-quota-based, or auction-based?
- Should DOLI ship a "covenant SDK" that exposes the guard conditions as templates (vault, escrow, payment channel, subscription) without requiring users to compose conditions manually?
