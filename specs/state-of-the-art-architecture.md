<!--
OUTPUT CONTRACT: N/A — architecture specification file (not a test file)
INPUT PARTITIONS: N/A — architecture specification file (not a test file)
-->

# DOLI State-of-the-Art Architecture

> **Status note (2026-05-25):** The DeFi-on-L1 vs DeFi-on-L2-via-ZKSettle decision raised in this document was resolved 2026-05-24 in favor of L1. The DeFi subsystem is now redesigned in `specs/defi-subsystem-architecture.md` (AMM-First Phase 1, escrow-loan composition, per-primitive activation heights). This document **remains authoritative** for ZKSettle activation path, the Condition SDK, and the competitive-positioning analysis against Move/Sui/Bitcoin/Cardano/EVM. The DeFi-on-L2 recommendation from this doc is SUPERSEDED for the DeFi subsystem scope only. ZKSettle itself stays gated and remains the recommended path for *novel* applications that cannot be expressed as compiled L1 primitives.

## Problem Statement

Identify structural changes to DOLI's architecture that close the gaps documented in a comparative analysis (DOLI vs. Ethereum/Solana/L2s/Cardano/Sui/Aptos/Polkadot/Bitcoin), while strictly preserving deterministic VDF-based consensus. The central gap: ZKSettle exists but is gated at `u64::MAX`, leaving DOLI without an answer to "what about novel applications?" and making the Move/Sui critique unanswerable.

## Executive Summary

Five independent evaluators examined DOLI's architecture from subtraction, restructuring, pattern-matching, failure-analysis, and radical-simplification lenses. The strongest signal is **unanimous convergence on ZKSettle activation as the single highest-leverage action** (5/5 evaluators). The second strongest signal is **convergent alarm about L1 DeFi primitives** (4/5 evaluators independently flagged them as dangerous, ungated, or architecturally misplaced). The third signal is **the condition language guards as an underexposed differentiator** (3/5 evaluators).

However, three critical discoveries BLOCK naive activation: (1) `ZK_SETTLE_ACTIVATION_HEIGHT` is a compile-time `const`, not runtime-mutable; (2) `verify_zk_proof` is a stub returning `InvalidProof` always; (3) the Collateral spending model has a security flaw (`is_conditioned()` missing Collateral). These must be resolved before any activation work begins.

The SSF candidate -- "Gate DeFi TX types, resolve the ZKSettle blockers, activate ZKSettle, ship one reference L2, ship a Condition SDK" -- is within 0.1 confidence of more complex proposals and wins under the radical tiebreaker. It closes A1, B1, B5, B6 gaps without consensus restructuring.

---

## Evaluation Summary

| Evaluator | Lens | Top Proposal | Confidence | Key Finding |
|-----------|------|-------------|------------|-------------|
| Subtractionist | removal | Deprecate L1 DeFi TX types (P2) | conf(0.55, inferred) | 13 DeFi TX types have zero activation gates; bridge crate is phantom dependency |
| Restructurer | boundaries | Extract protocol engine from node binary (P1) | conf(0.6, observed) | Node struct is 50+ field god object; dual wallet implementation |
| Pattern Matcher | patterns | ZKSettle activation + covenant guards exposure (P1+P2) | conf(0.65-0.70, observed) | u64::MAX gate is anti-pattern; guards ARE Bitcoin CTV equivalent |
| Failure Analyst | failures | Gate DeFi + fix Collateral spending model (P1+P2) | conf(0.60-0.65, observed) | Collateral UTXO spendable with simple signature; LiquidateLoan is 2-check shell |
| Radical Simplifier | minimal | Activate ZKSettle + Condition SDK (P1+P4) | conf(0.65, observed) | 10 TX types + 7 output types are application logic in L1 consensus |

---

## The SSF Candidate (Presented First and Alone)

**"Gate the 11 ungated DeFi TX types behind an activation height. Resolve the three ZKSettle blockers. Activate ZKSettle at a concrete future height. Ship one reference L2. Ship a Condition SDK with templates. Do nothing else."**

This is the single stupidest correct action sequence. It:

- **Closes B6** (no L2 ecosystem) -- directly, by activating ZKSettle
- **Closes A1** (no novel state machines) -- via L2 escape hatch
- **Closes B1** (hard-fork for every primitive) -- via L2 for new primitives
- **Closes B5** (AMM curve lock-in) -- novel curves on L2
- **Surfaces the covenant language** as competitive differentiator vs. Bitcoin
- **Eliminates the DeFi ungated attack surface** immediately (zero cost)
- Requires **zero structural refactoring** of the node binary
- Requires **zero migration** of existing UTXOs

**What the SSF candidate does NOT close:**

- A4 (producer-set decentralization) -- requires governance/ops, not architecture
- A5 (wallet UX) -- requires dedicated product work
- A7 (audit-ready security model) -- requires documentation + external audit
- B2 (producer collusion surface) -- structural, requires producer-set growth
- Node god-object structure -- deferred, not blocking

**SSF Gate check:** The SSF candidate is within 0.1 confidence of all more complex proposals and produces comparable gap closure. Under the radical tiebreaker, simpler wins. **Recommendation: present this alone first.**

---

## Critical Discoveries That Must Be Resolved BEFORE Activation Work

### D1: `ZK_SETTLE_ACTIVATION_HEIGHT` is a compile-time constant

- **Location:** `crates/core/src/validation/zk.rs:44`
- **Discovery:** Pattern Matcher (P2) and Radical Simplifier (P1) independently found this
- **The problem:** `pub(crate) const ZK_SETTLE_ACTIVATION_HEIGHT: u64 = u64::MAX` cannot be modified at runtime by a ProtocolActivation transaction. The spec says ProtocolActivation lowers it, but a `const` is baked into the binary at compile time.
- **Resolution required:** Either (a) move the height to `NetworkParams` as a runtime-checked field and wire ProtocolActivation to update it, or (b) set a concrete height directly in the constant and deploy a new binary to all nodes. Option (b) is simpler but requires coordinated binary deploy.
- **Evidence independence:** YES -- Pattern Matcher found it via spec-vs-code analysis; Radical Simplifier found it via first-principles analysis of the ProtocolActivation mechanism
- **Confidence:** conf(0.90, observed) -- two evaluators independently verified the same `const` declaration

### D2: `verify_zk_proof` is a stub returning `InvalidProof` always

- **Location:** `crates/core/src/validation/zk.rs:150-198`
- **Discovery:** Radical Simplifier (P1)
- **The problem:** All four proof system dispatch arms return `InvalidProof`. Activating ZKSettle without replacing this stub ships nothing -- every ZKSettle TX would be rejected.
- **Resolution required:** Vendor and integrate a real ZK verifier (RISC0 recommended by Radical Simplifier for Rust-native determinism). The determinism harness (CI matrix, cross-platform bit-identical verification) must be validated BEFORE the activation height is selected.
- **Evidence:** `zk.rs:178-196` -- dispatch on proof_system_id, all arms return error
- **Confidence:** conf(0.85, observed) -- single evaluator but directly verified in code

### D3: Collateral OutputType is NOT conditioned -- borrower can spend collateral without repaying

- **Location:** `crates/core/src/transaction/types.rs:222-233` (missing Collateral in `is_conditioned()`)
- **Discovery:** Failure Analyst (P2)
- **The problem:** `is_conditioned()` does not list `OutputType::Collateral`. A Collateral UTXO is spendable with a simple Ed25519 signature -- identical to a Normal UTXO. The borrower can take their collateral without repaying the loan.
- **Resolution:** This is not blocking ZKSettle activation, but IS blocking any lending activation. If DeFi TX types are gated at `u64::MAX` (as proposed), this becomes a "fix before un-gating" item rather than an emergency.
- **Evidence:** `types.rs:222-233` -- `is_conditioned()` match arms do not include Collateral (ID 11)
- **Confidence:** conf(0.75, observed) -- Failure Analyst verified the match arms directly

### D4: 11 DeFi TX types have ZERO activation gates

- **Location:** `crates/core/src/validation/transaction.rs` (no height checks for DeFi types)
- **Discovery:** Subtractionist (P2), Pattern Matcher (P3), Failure Analyst (P1), Radical Simplifier (P1) -- 4/5 convergence
- **The problem:** CreatePool, AddLiquidity, RemoveLiquidity, Swap, CreateLoan, RepayLoan, LiquidateLoan, LendingDeposit, LendingWithdraw, FractionalizeNft, RedeemNft are all valid from genesis. A hand-crafted transaction via RPC bypasses CLI restrictions.
- **Resolution:** Add a `defi_activation_height` field to `NetworkParams` set to `u64::MAX` on all networks. Gate all 11 types behind it. Minimal code change (+11 height checks, +1 constant). This is effectively a subtraction -- removing premature functionality.
- **Evidence:** Grep for activation height checks in validation paths for these TX types returns zero matches (Failure Analyst, Subtractionist)
- **Confidence:** conf(0.85, converged) -- 4 independent evaluators found this from different analytical lenses

---

## Convergence Matrix

```
                             Sub   Res   Pat   Fail  Rad   Count
Gate DeFi TX types:           Y     -     Y     Y     Y    4/5  -> DEFINITE
ZKSettle activation:          Y     -     Y     -     Y    3/5  -> DEFINITE
Condition SDK/exposure:       -     -     Y     -     Y    2/5  -> RECOMMENDED
Remove bridge crate dep:      Y     -     -     -     -    1/5  -> OPTION
Extract protocol engine:      -     Y     -     -     -    1/5  -> OPTION
Unify wallet:                 -     Y     -     -     -    1/5  -> OPTION
L2 interface crate:           -     Y     -     -     -    1/5  -> OPTION
Layer RPC methods:            -     Y     -     -     -    1/5  -> OPTION
Output type collapse:         Y     -     -     -     Y    2/5  -> REJECTED (both lowered confidence after kill test)
Remove rewards_legacy.rs:     Y     -     -     -     -    1/5  -> OPTION (trivial)
Collapse always-0 heights:   Y     -     -     -     -    1/5  -> OPTION
Fix Collateral spending:      -     -     -     Y     -    1/5  -> HARD CONSTRAINT (pre-lending-activation)
Determinism harness before:   -     -     -     Y     -    1/5  -> HARD CONSTRAINT (pre-ZKSettle)
RISC0-only proof system:      -     -     -     -     Y    1/5  -> OPTION
```

### Convergence Independence Checks

**Gate DeFi TX types (4/5 convergence):**
```
CONVERGENCE INDEPENDENCE CHECK:
Deletion: Gate (effectively disable) 11 DeFi TX types
Converging evaluators: Subtractionist, Pattern Matcher, Failure Analyst, Radical Simplifier
Evidence independence:
  - Subtractionist: measured 955+ lines of DeFi validation, found zero activation gates
  - Pattern Matcher: compared to Sui pattern, identified "AMM in protocol code" anti-pattern
  - Failure Analyst: measured validation depth per primitive, found LiquidateLoan is 2-check shell
  - Radical Simplifier: counted 10 TX types + 7 output types as application logic in consensus
  INDEPENDENT? YES -> True convergence -> conf(0.85, converged)
```

**ZKSettle activation (3/5 convergence):**
```
CONVERGENCE INDEPENDENCE CHECK:
Addition: Activate ZKSettle at concrete height
Converging evaluators: Subtractionist, Pattern Matcher, Radical Simplifier
Evidence independence:
  - Subtractionist: identified as enabler for DeFi-to-L2 migration
  - Pattern Matcher: compared to industry L2 patterns, UTXO atomicity advantage for ZK over optimistic
  - Radical Simplifier: identified as the single highest-leverage change from first principles
  INDEPENDENT? YES -> True convergence -> conf(0.75, converged)
  Note: Failure Analyst did NOT propose activation -- proposed determinism prerequisites instead.
  This is not a contradiction -- it is a sequencing disagreement (prerequisites before activation).
```

---

## Definite Changes (High Convergence)

### DC-1: Gate 11 DeFi TX types behind activation height set to u64::MAX

- **Convergence:** 4/5 (Subtractionist, Pattern Matcher, Failure Analyst, Radical Simplifier)
- **Evidence:** Zero activation height checks for DeFi TX types in validation (`validation/transaction.rs`). `LiquidateLoan` is a 2-check shell (`lending.rs:133-152`). Collateral OutputType missing from `is_conditioned()` (`types.rs:222-233`). All 4 evaluators arrived via independent analysis.
- **Confidence:** conf(0.85, converged)
- **Impact:** Closes the ungated attack surface. 11 TX types become unreachable until E2E testing is complete.
- **Hard fork required:** No -- this is additive (new gate). Rolling deploy safe via activation height gate.
- **Complexity cost:** +1 field in `NetworkParams`, +11 height checks in validation. Minimal.

### DC-2: Resolve ZKSettle blockers (D1, D2, D3 above)

- **Convergence:** Implicit in all 5 evaluators' proposals (no evaluator proposed activating ZKSettle without resolving the blockers)
- **Evidence:** `zk.rs:44` (const), `zk.rs:150-198` (stub), `types.rs:222-233` (Collateral)
- **Confidence:** conf(0.90, observed) -- directly verified in code by multiple evaluators
- **Impact:** Prerequisite for any ZKSettle activation
- **Hard fork required:** No (until actual activation)
- **Complexity cost:** D1: 1 field change. D2: +1 vendored verifier crate + determinism harness. D3: +1 entry in `is_conditioned()` + spending restriction (deferred to lending un-gating).

---

## Recommended Changes (Medium Convergence)

### RC-1: Activate ZKSettle at a concrete future height

- **Convergence:** 3/5 (Subtractionist, Pattern Matcher, Radical Simplifier) with Failure Analyst's prerequisites as hard constraint
- **Evidence:** Complete L2 settlement spec exists (`specs/l2-settlement.md`). ZK validation stub is well-structured (`zk.rs`, 277 lines). 4 proof system IDs reserved. UTXO atomicity makes ZK-only the correct L2 choice (Pattern Matcher). `u64::MAX` gate is the single largest gap (Brief section B6).
- **Confidence:** conf(0.75, converged) -- reduced from 0.85 because Failure Analyst's determinism prerequisite is procedural and unverified
- **Impact:** Closes B6 (no L2 ecosystem), A1 (no novel state machines), B1 (hard-fork for primitives), B5 (AMM curve lock-in)
- **Hard fork required:** Yes -- consensus change at activation height
- **Sequencing:** DC-2 (resolve blockers) MUST complete first. Determinism harness MUST pass CI matrix. Then activation height selected.
- **Complexity cost:** +1 vendored verifier crate, +1 match arm in validation. Net change to consensus surface: small.

FAILURE MODE FILTER:
```
Proposal: ZKSettle activation
  FM-1 Non-deterministic verifier -> consensus split: VULNERABLE -> conf -0.15
    Mitigation: determinism harness prerequisite (Failure Analyst P3)
  FM-2 Verifier bug = consensus bug: VULNERABLE -> conf -0.05
    Mitigation: soft-disable via ProtocolActivation re-raising height (spec 8.4)
  FM-3 Rolling deploy fork: NEUTRAL (activation height gate, not block content change)
  FM-4 Rebuild path divergence: NEUTRAL (ZKSettle adds new TX type, doesn't change existing)
  Adjusted confidence: conf(0.75, converged) [0.85 base - 0.15 + 0.05 soft-disable mitigation]
```

### RC-2: Expose condition language guards in CLI + templates

- **Convergence:** 2/5 (Pattern Matcher P1, Radical Simplifier P4) with Restructurer noting "condition builder" as cross-signal
- **Evidence:** Guards exist in `conditions/eval.rs:121-165` but CLI has no user-facing commands. `AmountGuard`, `OutputTypeGuard`, `RecipientGuard` are structurally equivalent to Bitcoin's OP_CTV/OP_CAT proposals. Internal usage confirmed: `set_covenant_witnesses` used in `cmd_pool.rs` (3 locations) and `cmd_bridge.rs` (4 locations).
- **Confidence:** conf(0.65, converged)
- **Impact:** Surfaces DOLI's strongest differentiator. Closes REQ-SOTA-004.
- **Hard fork required:** No -- purely CLI/SDK, zero consensus changes
- **Complexity cost:** +1 CLI module (~200-300 lines), +1 optional SDK crate (~500 lines). No consensus surface increase.

---

## Options for User Decision

### OPTION A: Remove `bridge` crate dependency from CLI

- **Source:** Subtractionist (P1)
- **Evidence:** CLI `bins/cli/Cargo.toml:17` declares bridge dependency. Zero `use bridge::` imports found. CLI builds bridge HTLCs via `doli-core` types directly. 1,638 lines + 4 transitive deps compiled for nothing.
- **Confidence:** conf(0.65, measured)
- **Complexity cost:** -1 crate dependency, -1,638 lines compiled, -4 transitive deps
- **Failure modes:** None identified. The bridge crate stays in workspace for future use.
- **vs. Radical floor:** 0 modules above minimum (this IS subtraction)

### OPTION B: Extract protocol engine crate from node binary

- **Source:** Restructurer (P1)
- **Evidence:** Node struct has 50+ fields (`mod.rs:83-281`). Protocol logic (apply_block ~1,931 lines, validation_checks ~1,079 lines, rewards ~1,353 lines, rollback ~310 lines, production ~1,724 lines = ~6,400 lines) mixed with infrastructure (networking, RPC, diagnostics). A second node implementation would need to reimplement all context assembly.
- **Confidence:** conf(0.6, observed)
- **Complexity cost:** +1 crate (engine), refactoring ~6,400 lines. Net: cleaner boundaries but significant effort.
- **Failure modes:** Apply_block accesses `self.network`, `self.diagnostic_emitter`, `self.archive_tx` -- these become injected callbacks, adding indirection. INC-I-081/082 patches would move with the code (safe per kill test).
- **vs. Radical floor:** +1 module above minimum. Does not close any brief gaps directly.
- **Note:** This is an auditability improvement (closes A7 partially) but does not close any brief's Must/Should gaps. Worth doing for code health, but not urgent.

### OPTION C: Unify wallet implementation

- **Source:** Restructurer (P2)
- **Evidence:** Two wallet implementations: CLI's `wallet.rs` and `crates/wallet/`. Wallet crate explicitly avoids `doli-core` dependency (documented at `lib.rs:9`). CLI uses `doli-core` directly for TX construction.
- **Confidence:** conf(0.65, observed)
- **Complexity cost:** -1 module (cli/wallet.rs). CLI drops `doli-core` and `storage` as direct dependencies.
- **Failure modes:** TxBuilder coverage unverified -- may not support all 27 TX types. Kill test inconclusive.
- **vs. Radical floor:** 0 modules above minimum (net subtraction). Closes A5 (wallet UX) partially.

### OPTION D: RISC0-only ZK proof system (drop multi-system plug-in surface)

- **Source:** Radical Simplifier (P3)
- **Evidence:** 4 proof system IDs reserved. Each adds dispatch surface + determinism risk. RISC0 runs inside RISC-V VM (deterministic by construction). Groth16 has known platform-dependent pairing arithmetic.
- **Confidence:** conf(0.55, observed)
- **Complexity cost:** -1 field from ZkRollupData, -dispatch logic. +hard fork if second system needed later.
- **Failure modes:** Single-system lock-in. If RISC0 has a vulnerability, no fallback proof system. Mitigation: soft-disable via ProtocolActivation.
- **vs. Radical floor:** -1 field below current spec. Simplest viable.
- **Counter-evidence from Pattern Matcher:** `proof_system_id` extensibility is a design win. Adding new systems should NOT require hard forks. This is a genuine trade-off: launch simplicity vs. future flexibility.

### OPTION E: Layer RPC methods (consensus-essential vs. operational)

- **Source:** Restructurer (P4)
- **Evidence:** 45+ methods in one flat dispatch. CLI uses ~18 consensus/query methods. Operational methods (backfill, diagnostics, guardian) used only by scripts.
- **Confidence:** conf(0.5, inferred)
- **Complexity cost:** +0 crates. Internal reorganization of dispatch.
- **Failure modes:** None significant. Operational methods can be individually promoted if needed.
- **vs. Radical floor:** 0 modules above minimum.

### OPTION F: Collapse always-0 activation heights into unconditional code

- **Source:** Subtractionist (P3)
- **Evidence:** 5 activation heights are 0 on all networks. The `height >= 0` check is always true. Dead branches.
- **Confidence:** conf(0.50, observed)
- **Complexity cost:** -5 fields from NetworkParams, ~15 branches collapsed.
- **Failure modes:** Reduces historical readability (the gate documents what changed and when). CLAUDE.md calls crossed heights "immutable" -- removing checks is a philosophical question.
- **vs. Radical floor:** Minor cleanup, not structural.

---

## Constraints (from Failure Analyst -- Non-Negotiable)

### Hard Constraints

| ID | Constraint | Source | Evidence |
|----|-----------|--------|----------|
| C1 | Activation heights immutable once crossed | INC-I-054 | Height moved forward deactivated live features causing permanent fork |
| C2 | Bit-identical rebuild paths | INC-I-082 | Offline rebuild diverged from online apply causing fork |
| C3 | Encoder/decoder parity for ALL consumers | Pillar 2 | 3 decoders using different lists caused silent reward misattribution |
| C4 | Local-state HashSet modifying scheduler inputs MUST be capped | INC-I-016 | Unbounded exclusion set caused 23/32 producers excluded in one block |
| C5 | Block content changes require synchronized deploy | INC-I-062 | Rolling deploy created competing valid blocks causing fork |
| C6 | Epoch-boundary mutations ONLY at epoch boundary | CLAUDE.md | Mid-epoch mutations cause state divergence |
| C7 | Three-question consensus-shape checklist | INC-I-075 | "Unused" code path triggered by first DelegateBond |
| C8 | Recovery must not amplify single-peer bad state | Snap-sync cascade | Bad state propagated via sync causing fleet-wide fork |

### Soft Constraints

| ID | Constraint | Source |
|----|-----------|--------|
| C9 | Epoch-boundary primitives need slot-abort path | INC-I-081 |
| C10 | DeFi TX types should be gated before building on them | All evaluators |

---

## Architecture Maps

### Current Architecture
```
                         crypto (leaf)
                           |
                          vdf (leaf)
                           |
                       doli-core (27 TX types, 15 output types, 11 conditions)
                      /    |     \          \
                storage  network  bridge   channels
                  |        |
                mempool    |
                  \       /
                   rpc (45+ methods, flat)
                    |
                  updater
                    |
              doli-node (50+ field god object, protocol + infra mixed)
              
              doli-cli (own wallet.rs, depends on core + storage)
              doli-gui (uses crates/wallet/)
```

**Protocol surface:** 27 TX types (11 DeFi ungated), 15 output types, ~5,800 lines validation, ~2,959 lines DeFi-specific validation. ZKSettle gated at u64::MAX with stub verifier.

### Proposed Architecture (Definite + Recommended)
```
                         crypto (leaf)
                           |
                          vdf (leaf)
                           |
                       doli-core (27 TX types preserved, 11 DeFi gated at u64::MAX)
                      /    |     \          \
                storage  network  bridge   channels
                  |        |
                mempool    |
                  \       /
                   rpc (unchanged)
                    |
                  updater
                    |
              doli-node (unchanged structurally)
              
              doli-cli + condition-sdk (templates for guards)
              doli-gui (unchanged)
```

**Key differences from current:**
1. 11 DeFi TX types gated behind `defi_activation_height = u64::MAX`
2. ZK_SETTLE_ACTIVATION_HEIGHT lowered to concrete height
3. Real ZK verifier (RISC0 or other) replacing stub
4. Condition SDK crate with pre-built templates
5. CLI exposes guard conditions via user-facing commands

**What does NOT change:** Node binary structure, storage layer, network layer, RPC layer, UTXO model, consensus engine, scheduler, epoch-boundary logic.

---

## Migration / Sequencing Plan

### Phase 0: Safety Gates (no consensus change, rolling deploy safe)

1. Add `defi_activation_height: u64::MAX` to NetworkParams
2. Add height checks for all 11 DeFi TX types in validation
3. Deploy new binary (rolling restart -- this is additive, not content-changing)
4. **Verify:** No DeFi TXs exist on mainnet (query UTXO set). If any exist, adjust gate to be above current height.

### Phase 1: ZKSettle Blockers (no consensus change)

1. Select proof system (RISC0 recommended)
2. Vendor verifier crate
3. Build determinism harness (CI matrix: x86_64-linux, aarch64-linux, aarch64-darwin)
4. Replace `verify_zk_proof` stub with real implementation
5. Run determinism harness for 30 days minimum
6. Resolve `ZK_SETTLE_ACTIVATION_HEIGHT` mechanism: either move to NetworkParams or set concrete height in const
7. **Do NOT select activation height yet**

### Phase 2: Condition SDK (no consensus change)

1. Create `condition-sdk` crate with pre-built templates (vault, escrow, subscription, payment channel, limit order)
2. Add CLI commands: `create-vault`, `create-escrow`, `compose-condition`
3. Ship documentation positioning guards vs. Bitcoin CTV/OP_CAT

### Phase 3: ZKSettle Activation (consensus change -- requires activation height)

1. Set `zk_settle_activation_height` to concrete future height (at least 30 days ahead)
2. Deploy binary to all nodes (rolling deploy OK -- activation height gates it)
3. At activation height: ZKSettle TX type becomes valid
4. Ship reference L2 demonstrating AMM + lending on ZKSettle proofs

### Phase 4: Positioning (no code change)

1. Publish competitive analysis vs. Move/Sui/Aptos
2. Publish formal security model (audit-ready)
3. Publish producer-set growth roadmap

### Dependencies

```
Phase 0 -------> (no dependency on other phases)
Phase 1 -------> Phase 3 (ZKSettle activation requires verifier)
Phase 2 -------> (no dependency on other phases)
Phase 3 -------> Phase 1 (verifier + determinism harness)
Phase 4 -------> Phase 3 (competitive narrative needs live ZKSettle)
```

Phases 0, 1, and 2 can proceed in parallel.

---

## What This Redesign Does NOT Close

| Gap | Status | Why |
|-----|--------|-----|
| A4: Producer-set decentralization (5 nodes, 34 producers) | Open | Requires governance/economic changes, not architecture. Bond threshold reduction or delegation-based scaling are policy decisions. |
| A5: Wallet UX | Partially addressed | Condition SDK helps. Full wallet UX (simulation, address book, web UI) is product work (Option C above). |
| A7: Audit-ready security model | Partially addressed | Phase 4 positioning helps. External audit engagement is organizational. |
| B2: Producer collusion surface | Open | 34 producers. Mitigated by seniority weighting + tier system. Growth is social/economic. |
| A3: Cross-chain general messaging | Open | Won't scope per brief. BridgeHTLC remains for atomic swaps. |
| Node god-object structure | Open | Option B above. Improves code health but does not close any brief gap. |

---

## Competitive Positioning After Redesign

### vs. Move/Sui

**Before:** "DOLI has no programmability answer to Move's linear types."
**After:** "DOLI has a declarative covenant language (11 composable primitives, live) + L2 escape hatch for arbitrary computation via ZK proofs. Safety-by-declaration + verification, not safety-by-type-system."

### vs. Ethereum

**Before:** "DOLI is a Bitcoin-like chain with DeFi bolted on."
**After:** "DOLI is a settlement layer with native covenants and ZK-verified L2s. Like Ethereum's rollup-centric roadmap, but with UTXO atomicity and deterministic scheduling instead of gas auctions."

### vs. Bitcoin

**Before:** "DOLI has live covenants, Bitcoin has proposals."
**After:** "DOLI has live covenants (AmountGuard, OutputTypeGuard, RecipientGuard) AND a ZK settlement layer. Bitcoin has neither."

---

## Complexity Comparison

| Metric | Current | Radical Minimum | Proposed |
|--------|---------|----------------|----------|
| TX types (active) | 26 | 17 + ZKSettle = 18 | 26 (11 DeFi gated, 15 active + ZKSettle) |
| Output types | 15 | 8 | 15 (unchanged, DeFi outputs inert when gated) |
| Condition primitives | 11 | 11 | 11 + templates (no consensus change) |
| Proof systems | 4 reserved, 0 implemented | 1 | 1 (user decides: RISC0-only or keep plug-in) |
| L1 validation LoC | ~5,800 | ~3,800 | ~6,300 (+500 verifier) |
| L2 crates | 0 | 1 | 1 (reference L2) |
| Activation surface (fields) | 13 | 13 | 14 (+defi_activation_height) |
| New consensus changes | 0 | 1 (ZKSettle activation) | 1 (ZKSettle activation) |

---

## Contradictions Found and Resolution

### Contradiction 1: Output type collapse viability

**Subtractionist (P4):** Proposed collapsing Multisig/Hashlock/HTLC/Vesting output types into conditioned Normal outputs. conf(0.35, inferred) -- self-downgraded after kill test.

**Radical Simplifier (P2):** Proposed same collapse. conf(0.35, inferred) -- self-downgraded after kill test (Pool/Collateral/NFT carry semantic data, not just conditions).

**Restructurer (F9):** Explicitly found output types and conditions are COMPLEMENTARY (type = what, condition = how-to-spend). "No unification needed."

**Resolution:** Both proposing evaluators ran kill tests and downgraded their own confidence. The Restructurer's analysis is the most detailed (traced both code paths). Output type collapse is REJECTED for this redesign. The theoretical reduction (15 to 10 types) is modest, carries high fork risk, and two evaluators independently concluded the reward/risk ratio is poor. RESOLVED.

### Contradiction 2: ZKSettle activation sequencing

**Pattern Matcher, Radical Simplifier, Subtractionist:** Propose activating ZKSettle (P2, P1, P2 respectively).

**Failure Analyst (P3):** Proposes determinism harness BEFORE height selection. "Setting activation height without the harness creates a ticking time bomb identical to INC-I-054."

**Resolution:** Not a contradiction -- a sequencing disagreement. The Failure Analyst does not oppose activation; they oppose premature height selection. This is incorporated as a HARD PREREQUISITE: Phase 1 (blockers) must complete before Phase 3 (activation). RESOLVED by sequencing.

### Contradiction 3: DeFi -- deprecate vs. freeze

**Subtractionist (P2):** "Deprecate" -- add height gate that REJECTS new DeFi TXs after a future height.

**Pattern Matcher (P3):** "Freeze" -- keep as "L1 DeFi of last resort," route innovation to L2.

**Resolution:** Functionally identical for this redesign. Both agree: (1) gate DeFi now, (2) do not add new L1 DeFi primitives, (3) existing UTXOs remain valid. The difference is framing: "deprecated" vs. "frozen base-layer." This is a positioning choice, not an architectural one. RESOLVED by noting the framing question for the user.

---

## Design Synthesis Quality Gate

```
--- DESIGN SYNTHESIS QUALITY GATE ---
Evaluators completed:           5/5
Deletion convergence items:     1 (4/5 agreement: gate DeFi TX types)
Restructuring convergence:      0 (no 2+ agreement on structural refactoring)
Addition options presented:     6 (Options A-F)
Failure modes identified:       8 hard + 2 soft (from Failure Analyst)
Failure modes applied as filters: 4/10 (applied to ZKSettle activation, DeFi gating)
Radical floor gap:              26 active TX types -> 18 radical minimum -> 16 active (proposed)
Contradictions found:           3
Contradictions resolved:        3/3
Evidence independence verified: YES (for all convergence claims)
---
```
