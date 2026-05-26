# Oracle Structural-Anchored Redesign Analysis — DOLI L1 Phase 2.1

**Date:** 2026-05-25
**Mode:** Proposal-only (Pass 1 of 4 sequential design passes)
**Author:** Antonio Lozada <antonio@omegacortex.ai>
**Position:** Additive foundation ABOVE Phase 1 (specs/defi-foundations-economics.md §0 — LOCKED) and the 2026-05-24 AMM-First base (specs/defi-subsystem-architecture.md — LOCKED)

---

## 1. Affected Economic Subsystems

### 1.1 AMM Phase 1 (Pool pricing, TWAP, slippage)

**Does AMM NEED an oracle? NO.** The AMM self-prices via constant-product math. The Pool UTXO already stores a `cumulative_price` accumulator (u128) and `last_update_slot` (u32) — verified at `crates/core/src/transaction/output.rs:81,137`. The RPC `getPoolPrice` already exposes both spot price and TWAP over a caller-specified `windowSlots` parameter (`crates/rpc/src/methods/pool.rs:99-150`). An oracle adds NO value to the AMM's own pricing.

However, the AMM TWAP is a **candidate input TO the oracle** (as one price source) and the oracle is a **potential consumer interface** for lending/liquidation that references AMM-traded assets. The interaction is one-directional: oracle reads AMM TWAP, AMM never reads oracle.

### 1.2 Lending (Phase 2.3 — NOT scoped here)

Lending is the PRIMARY consumer of oracle. The interface contract oracle must expose:
- A deterministic on-chain price for any asset pair that the lending system references
- A finality guarantee: the price was committed in block B, lending reads it at block B+N where N >= finality depth
- A staleness bound: the price was updated within the last T blocks (configurable per pair)

This is the sole reason oracle exists. Without lending, oracle has ZERO consumers (Phase 1 proved this — see `specs/defi-foundations-economics.md` §7 P1 verdict: "Zero consumers in Phase 1").

### 1.3 Liquidations

Liquidations ONLY exist if lending exists. Liquidation triggers are oracle-price-dependent thresholds (collateralization ratio falls below minimum). The oracle's latency budget is directly determined by liquidation safety margins.

### 1.4 Escrow-Loan (Phase 1 M4)

Escrow-loans are explicitly oracle-FREE by design (`specs/defi-subsystem-architecture.md` §D1: "Overcollateralized lending via covenant composition eliminates oracle dependency entirely"). No oracle interaction.

### 1.5 Reward Pool Denomination

Already DOLI-denominated. No oracle needed. Confirmed: reward pool is addressed at `BLAKE3("REWARD_POOL"||"doli")` and all subsidy + W2 fees route there in native DOLI.

### 1.6 NFT Royalties / Cross-Asset Operations

NFTs (OutputType 6, 14) use fixed-amount conditions. No price reference. If cross-asset royalties ever require conversion (e.g., "2% of sale price in DOLI"), that becomes an oracle consumer — but this is NOT scoped for Phase 2.1.

### 1.7 Summary: Oracle Consumer Map

| Consumer | Phase | Dependency Level |
|----------|-------|-----------------|
| AMM TWAP | Phase 1 (live) | NONE — AMM is a price SOURCE, not consumer |
| Lending | Phase 2.3 (future) | CRITICAL — primary consumer |
| Liquidations | Phase 2.3 (future) | CRITICAL — triggered by oracle price |
| Escrow-loan | Phase 1 (live) | NONE — explicitly oracle-free |
| Reward pool | Live | NONE — denominated in DOLI |
| NFT cross-asset | Unscoped | POTENTIAL future consumer |

---

## 2. Economic Invariants That MUST Be Preserved

### EI-ORACLE-1: Manipulation Cost Floor (37.3% adversary)

An external attacker controlling the maximum acquirable bond weight (currently ~37.3% = ~105,067 DOLI = ~10,507 bond units) MUST NOT be able to push the oracle price more than X% from true market price for more than 1 attestation window without sustaining a cumulative slashing cost exceeding the oracle-dependent TVL multiplied by the maximum exploitable deviation.

**Testable:** Simulate attacker submitting deviating attestations at max weight for N consecutive windows. Verify that bond-weighted-median (or chosen aggregation) produces at most Y% deviation AND that sustained deviation triggers slashing exceeding $Z.

### EI-ORACLE-2: Slashing Follows EI-3 (100% Burn)

Any provable oracle misreport triggers 100% bond burn via the existing `calculate_slash()` path (`crates/core/src/consensus/exit.rs:142-145`). No new slash destinations. No partial penalties. No restitution.

**Testable:** Slash TX for oracle misreport produces `SlashResult { burned_amount: full_bond_amount }` and routes to burn (supply reduction), not to any recipient.

### EI-ORACLE-3: Oracle Finality = Block Finality

No oracle price update is visible to downstream consumers (lending, liquidation) until its containing block is finalized. Oracle cannot race ahead of block finality.

**Testable:** Query `getOraclePrice` at height H returns only prices committed in blocks at height <= H - finality_depth.

### EI-ORACLE-4: Bond Withdrawal Vesting Applies Uniformly

Attesters who also hold producer bonds are subject to the existing 7-day unbonding period (UNBONDING_PERIOD = 60,480 slots, `crates/core/src/consensus/constants.rs:344`) and the Y1-75%/Y2-50%/Y3-25%/Y4-0% vesting penalty schedule (`crates/core/src/consensus/exit.rs:100-118`). No separate "attester bond" with a faster exit. Oracle participation must reuse the existing Bond OutputType (disc=1), not introduce a new bond type.

**Testable:** An attester who `RequestWithdrawal`s after 1 year loses 75% of withdrawn bonds to burn, identical to a non-attester producer.

### EI-ORACLE-5: No `delete_epoch_state()` Trigger (INC-I-054)

The oracle primitive MUST NOT bump `CURRENT_PROTOCOL_VERSION`. If oracle state is stored in EpochState, it MUST be additive (new fields only) and trigger `EPOCH_STATE_FORMAT_VERSION` bump — never `CURRENT_PROTOCOL_VERSION`.

**Testable:** Binary with oracle feature activated has identical `CURRENT_PROTOCOL_VERSION` to binary without. `delete_epoch_state()` is never called on upgrade.

### EI-ORACLE-6: Three-Question Consensus-Shape Checklist Compliance (INC-I-075)

Oracle attestation TX — Q1: Can any user-submittable transaction trigger this code path? **YES** (attesters submit attestation TXs). Q2: Can any producer-action or attestation pattern trigger it? **YES** (block proposer includes attestation TXs). Q3: Is the new behavior bit-identical to the old behavior for ALL reachable inputs? **NO** (new TX type, new state). Therefore: `oracle_activation_height` in `NetworkParams` is REQUIRED. Never bundle.

**Testable:** At height < `oracle_activation_height`, node rejects oracle TX types with a stable error code. At height >= `oracle_activation_height`, oracle TXs are processed normally.

### EI-ORACLE-7: No New Token (HC-7)

All attestation rewards and slashing penalties are denominated exclusively in native DOLI. No oracle token, no governance token, no staking derivative.

**Testable:** After oracle activation, `OutputType::from_u8(N)` for any oracle-related output returns a type whose `is_native_amount()` returns true (if amount represents DOLI) or false only for non-DOLI metadata (analogous to Pool's amount=0 pattern).

### EI-ORACLE-8: Snap-Sync Reproducibility

Oracle state MUST be included in the state root such that a snap-synced node (which receives a state snapshot without full block history) can correctly validate all subsequent oracle-dependent operations. No "rebuild oracle from block 0" path that breaks snap sync.

**Testable:** Node A snap-syncs from Node B at height H. Node A processes blocks H+1..H+N. Oracle state on Node A is bit-identical to Node B after both apply block H+N.

### EI-ORACLE-9: Attestation Reward Sustainability

Oracle attestation incentives MUST be funded from the EXISTING reward pool (epoch subsidy + W2 protocol fee). No new emission. The fraction routed to attestation rewards must not exceed a threshold that degrades producer block-production incentives below minimum viable security budget.

**Testable:** Compute `attestation_reward_fraction = oracle_attestation_rewards / total_epoch_pool`. Verify this fraction stays below a defined ceiling (evaluators to recommend — likely 5-15% of epoch pool) across all eras.

### EI-ORACLE-10: Oracle State Deterministic from Block Stream

Any node replaying the block stream from genesis (or from a snapshot) MUST derive identical oracle state. No side channels, no external data sources at validation time. The oracle attestation is a TX IN a block — validation depends only on the block contents and prior state.

**Testable:** Two nodes, both starting from the same snapshot, processing the same blocks, produce bit-identical oracle state and state root. (This is the standard determinism invariant, applied specifically to oracle state.)

---

## 3. Quantified Acceptance Criteria for the Redesign

### AC-ORACLE-1: Manipulation Cost at 37.3% Adversarial Bound

At current bond distribution (external attacker max = 105,067 DOLI = ~10,507 bond units), the cost to deviate the oracle price by >= 10% for >= 1 attestation window:
- If using bond-weighted median: attacker at 37.3% cannot deviate median at all (needs > 50% weight). Cost = infinite. PASS.
- If using bond-weighted mean: attacker at 37.3% can deviate by up to 37.3% × deviation_submitted. To achieve 10% effective deviation: submit price 26.8% off (37.3% × 26.8% = 10%). Slashing cost = 105,067 DOLI if caught. Manipulation value = 10% × oracle-dependent TVL.
- Required: manipulation_cost > 2× oracle-dependent TVL for any aggregation rule chosen.

Derivation: At DOLI price P, attacker bond = 105,067 × P. For oracle to be safe for lending TVL = V, require 105,067 × P > 2V. At P = $1: safe up to $52,533 TVL. At P = $10: safe up to $525,335 TVL.

### AC-ORACLE-2: Manipulation Cost at Post-Sunset 50.1% Adversarial Bound

After the structural-anchored model sunsets (structural share drops below threshold X), the system must handle an adversary with > 50% bond weight. Under bond-weighted median, a 50.1% attacker controls the median completely. Fallback mechanism must degrade gracefully:
- TWAP-only mode: manipulation requires sustained on-chain AMM trades, costing fees + capital lockup
- Halt-new-attestations: oracle goes stale but does not produce false data
- Required: explicit degradation path, not silent failure

### AC-ORACLE-3: Attestation Reward Budget

Oracle attestation incentives funded by existing reward pool — no new emission.
- Era 0 epoch pool: 360 DOLI/hour (SLOTS_PER_REWARD_EPOCH=360 × INITIAL_REWARD=1 DOLI per slot)
- At 5% allocation to attestation: 18 DOLI/hour = 0.3 DOLI/minute
- At 10% allocation: 36 DOLI/hour = 0.6 DOLI/minute
- Must justify: why attesters participate at this rate, given their existing producer rewards plus slashing risk.

Evaluators must compute: attestation_reward / slashing_risk ratio per attester per epoch, at the recommended allocation percentage.

### AC-ORACLE-4: Sunset Trigger Threshold

Structural bond share drops below X% (evaluators recommend X, with justification). Current: 62.7%. Plausible thresholds:
- 50%: structural set loses majority → attestation oracle loses integrity guarantee immediately
- 55%: provides 5% buffer above 50% threshold
- 40%: structural set is minority — oracle already operating on social trust, not structural guarantee

Evaluators must specify X AND the fallback behavior (TWAP-only, halt, rotate to full-set committee).

### AC-ORACLE-5: Latency Budget

Time from true price move to on-chain oracle price <= N seconds.
- At per-block attestation: N = 1 slot = 10 seconds + finality depth
- At per-K-block attestation: N = K × 10 seconds + finality depth
- At per-epoch attestation: N = 360 × 10 seconds = 3,600 seconds = too stale for liquidation

Evaluators must recommend N with derivation from lending liquidation margin requirements. Liquidation requires: oracle_latency < (collateralization_ratio - 1) × position_lifetime_in_blocks × max_expected_price_move_per_block.

### AC-ORACLE-6: Agent-Readiness 4/4

- (A) RPC discoverability: `getOraclePrice` and `getOracleAttestations` methods exist, return structured JSON
- (B) Determinism: off-chain node replaying blocks produces identical oracle state
- (C) Bounded execution: attestation TX submission has bounded cost (known fee), rejection is immediate with stable error code
- (D) Composability: oracle price is readable from a UTXO or state field without parsing block-specific formats

### AC-ORACLE-7: Producer Revenue Non-Degradation

Post-activation, per-epoch producer revenue (subsidy + W2 + attestation_share) must NOT decrease for any producer who participates in attestation. Specifically:
- If attestation_fraction is carved from the epoch pool, non-attesting producers lose that fraction
- Attestation participation must be opt-in (producers who don't attest keep their current reward share from the remaining pool)
- OR: attestation participation must be mandatory for all active producers (no opt-in complexity, but creates liveness risk if some producers cannot attest)

---

## 4. Architecture Context (Constraints on the Redesign)

### 4.1 Module Boundaries

| Crate | Oracle Interaction | Direction |
|-------|-------------------|-----------|
| `crates/core/src/transaction/types.rs` | New TxType variant (e.g., `PriceAttestation = 16` or `= 23`) + possibly new OutputType variant (e.g., `OraclePrice = 15`) | Modified |
| `crates/core/src/transaction/output.rs` | New `extra_data` layout for OraclePrice output (if used) | Modified |
| `crates/core/src/validation/` | New validation function for oracle TX; height gate at `oracle_activation_height` | Modified |
| `crates/core/src/conditions/` | Possible new guard (e.g., `OraclePriceGuard`) for consumers to reference oracle price | Modified (optional) |
| `crates/core/src/network_params/mod.rs` | New field: `pub oracle_activation_height: u64` | Modified |
| `crates/core/src/epoch_state/mod.rs` | Possible new field for aggregated oracle price (if stored in EpochState) | Modified (risky — requires EPOCH_STATE_FORMAT_VERSION bump) |
| `crates/core/src/scheduler.rs` | NOT modified — oracle attestation MUST NOT interfere with block production scheduling | Read-only |
| `crates/mempool/src/` | Admit oracle attestation TXs; contention diagnostic (P5 pattern) | Modified |
| `crates/rpc/src/methods/` | New `getOraclePrice`, `getOracleAttestations` methods (likely new file `oracle.rs`) | Modified |
| `bins/node/src/node/apply_block.rs` | Process oracle attestation TXs, update oracle state | Modified |
| `bins/node/src/node/rewards.rs` | Potentially route attestation reward fraction | Modified (if rewards for attestation) |
| `crates/storage/` | Persist oracle state across restarts (UTXO-based, or EpochState-based, or separate KV) | Modified |

### 4.2 Data Flows

```
External price (off-chain) → Producer signs attestation TX → Gossip/mempool propagation →
Block proposer includes attestation TXs → apply_block() processes them →
Oracle state updated (UTXO or EpochState field) → State root includes oracle state →
Consumers read via RPC (getOraclePrice) OR via condition guards at TX validation time
```

Key data flow questions:
- **Where does oracle price END UP?** Options: (a) dedicated OraclePrice UTXO (condition-prefixed extra_data, consumed and recreated each update cycle); (b) EpochState field (aggregated at epoch boundary only); (c) separate KV in state_db (requires snap-sync inclusion).
- **Where do consumers READ it?** Options: (a) from UTXO set directly (standard UTXO query); (b) from RPC endpoint (off-chain read); (c) from condition guard evaluation context (on-chain enforcement at spend time).

### 4.3 Dependency Direction

```
Oracle DEPENDS ON:
  - Producer bond state (EpochState.bond_snapshot) — for weight calculation
  - Block production (to include attestation TXs)
  - Existing slash infrastructure (calculate_slash, SlashResult)
  - Existing TX propagation (gossip, mempool)

DEPENDS ON Oracle:
  - Lending (Phase 2.3) — reads aggregated price for collateral valuation
  - Liquidation (Phase 2.3) — triggers when oracle price crosses threshold
  - Future condition guards (OraclePriceGuard) — evaluated at TX validation time
  - RPC consumers (agents, frontends) — informational query
```

### 4.4 Snap-Sync Compatibility

The oracle state MUST be included in the state root calculation (`crates/storage/src/snapshot.rs`). Options:
- If oracle price is a UTXO: it's already in the UTXO set → included in state root automatically. This is the simplest option.
- If oracle price is in EpochState: included via `epoch_state_hash` in state root. Requires EPOCH_STATE_FORMAT_VERSION bump (not CURRENT_PROTOCOL_VERSION).
- If oracle price is in a separate KV: must be explicitly included in snapshot serialization and state root calculation. More complex.

### 4.5 Three-Question Consensus-Shape Checklist (Applied)

For a hypothetical `PriceAttestation` TX type:
- **Q1:** Can any user-submittable transaction trigger this code path? **YES** — attesters submit PriceAttestation TXs to the mempool.
- **Q2:** Can any producer-action or attestation pattern trigger it? **YES** — block proposer includes attestation TXs; aggregation logic runs in apply_block.
- **Q3:** Is the new behavior bit-identical to the old behavior for ALL reachable inputs? **NO** — new TX type processes new logic, new state emerges.

**VERDICT:** Activation height REQUIRED. `oracle_activation_height` field in `NetworkParams`, set to `u64::MAX` until explicitly activated. Never bundle with `defi_activation_height` or `amm_activation_height`.

---

## 5. Capability Inventory (Existing Primitives)

### 5.1 OutputType Variants

| Disc | Name | Status |
|------|------|--------|
| 0 | Normal | Used |
| 1 | Bond | Used |
| 2 | Multisig | Used |
| 3 | Hashlock | Used |
| 4 | HTLC | Used |
| 5 | Vesting | Used |
| 6 | NFT | Used |
| 7 | FungibleAsset | Used |
| 8 | BridgeHTLC | Used |
| 9 | Pool | Used |
| 10 | LPShare | Used |
| 11 | Collateral | Used (frozen per INC-I-088) |
| 12 | LendingDeposit | Used (frozen per INC-I-088) |
| 13 | ZKRollup | Used |
| 14 | EncryptedContent | Used |
| **15** | — | **FREE** |
| **16+** | — | **FREE** |

Source: `crates/core/src/transaction/types.rs:143-189`, `from_u8()` at :192-211.

### 5.2 TxType Discriminants

| Disc | Name | Status |
|------|------|--------|
| 0 | Transfer | Used |
| 1 | Registration | Used |
| 2 | Exit | Used |
| 3 | ClaimReward | Used |
| 4 | ClaimBond | Used |
| 5 | SlashProducer | Used |
| 6 | Coinbase | Used |
| 7 | AddBond | Used |
| 8 | RequestWithdrawal | Used |
| 9 | ClaimWithdrawal | Tombstone (DO NOT REUSE) |
| 10 | EpochReward | Used |
| 11 | RemoveMaintainer | Used |
| 12 | AddMaintainer | Used |
| 13 | DelegateBond | Used |
| 14 | RevokeDelegation | Used |
| 15 | ProtocolActivation | Used |
| **16** | — | **FREE** |
| 17 | MintAsset | Used |
| 18 | BurnAsset | Used |
| 19 | CreatePool | Used |
| 20 | AddLiquidity | Used |
| 21 | RemoveLiquidity | Used |
| 22 | Swap | Used |
| **23** | — | **FREE** |
| 24 | CreateLoan | Used (frozen) |
| 25 | RepayLoan | Used (frozen) |
| 26 | LiquidateLoan | Used (frozen) |
| 27 | LendingDeposit | Used (frozen) |
| 28 | LendingWithdraw | Used (frozen) |
| 29 | FractionalizeNft | Used (frozen) |
| 30 | RedeemNft | Used (frozen) |
| 31 | ZKSettle | Used |

Source: `crates/core/src/transaction/types.rs:7-99`, `from_u32()` at :104-134.

**Free TX type discriminants: 16 and 23.** The oracle can use one of these (likely 16 — next sequential after ProtocolActivation).

### 5.3 Existing Condition Guard Patterns

| Guard | Evaluation Logic | Relevant to Oracle |
|-------|-----------------|-------------------|
| `Signature(Hash)` | Verify signature from pubkey matching hash | Standard attestation signing |
| `Multisig { threshold, keys }` | N-of-M signature verification | Could enforce multi-attester threshold |
| `AmountGuard { min_amount, output_index }` | `tx.outputs[i].amount >= min_amount` | Could compose with oracle price check |
| `RecipientGuard { expected_pubkey_hash, output_index }` | `tx.outputs[i].pubkey_hash == expected` | N/A |
| `OutputTypeGuard { expected_type, output_index }` | `tx.outputs[i].output_type == expected` | Enforce oracle output type |
| `MaxDeltaGuard { max_change_bps, reference_amount, output_index }` | `|output - reference| / reference <= max_bps` | **Directly applicable** — could constrain oracle price deviation from reference |
| `ReserveRatioGuard { min_ratio_bps, reserve_output_index, debt_output_index }` | `reserve / debt >= min_ratio_bps / 10000` | **Directly applicable** — collateralization checks referencing oracle |
| `Timelock(height)` / `TimelockExpiry(height)` | Height-based spend constraints | Attestation window enforcement |
| `And` / `Or` / `Threshold` | Boolean composition | Multi-guard oracle conditions |

Source: `crates/core/src/conditions/mod.rs:116-188`.

A new `OraclePriceGuard` could follow the same pattern: evaluate against a stored oracle price at validation time. However, this introduces a dependency from condition evaluation to oracle state — increasing coupling.

### 5.4 Producer Scheduling Primitives

- **DeterministicScheduler** (`crates/core/src/scheduler.rs:91-99`): `slot % total_tickets` → binary search in `ticket_boundaries` → producer
- **EpochState.producer_list** (`crates/core/src/epoch_state/mod.rs:71`): frozen sorted pubkey list for the epoch
- **EpochState.bond_snapshot** (`crates/core/src/epoch_state/mod.rs:67`): `HashMap<Hash, u64>` of bond counts per pubkey_hash
- **EpochState.active_list** (`crates/core/src/epoch_state/mod.rs:76`): subset entering round-robin

Oracle attestation MUST NOT collide with scheduling. The attester set may overlap with the producer set (likely: all attesters ARE producers), but the attestation action must be independent of the block-proposal action to prevent self-dealing.

### 5.5 Slashing Infrastructure

- `calculate_slash(bond_amount: Amount) -> SlashResult { burned_amount: bond_amount }` (`exit.rs:142-145`)
- Slash is triggered by `SlashProducer` TX (type 5) with evidence
- 100% burn, no recipient, permanent exclusion
- Existing evidence model: equivocation (two blocks signed for same slot)

For oracle: new evidence type needed (contradicting attestations, or provable deviation from median). Must fit within the existing `SlashProducer` TX type OR a new slash TX type (uses one of the free discriminants).

### 5.6 Withdrawal Vesting

The 7-day unbonding (UNBONDING_PERIOD = 60,480 slots) and vesting penalty (Y1-75%/Y2-50%/Y3-25%/Y4-0%) apply to ALL bonds via `RequestWithdrawal` (TxType 8). The logic is in `calculate_exit()` / `calculate_exit_with_quarter()` at `crates/core/src/consensus/exit.rs`. It operates on `Bond` OutputType (disc=1) regardless of whether the bond holder is a producer, attester, or delegator.

**Key fact:** Since oracle attestation would reuse the existing Bond OutputType (per EI-ORACLE-4), the vesting penalty applies automatically. No new code needed for withdrawal discipline.

### 5.7 RPC Naming Convention

All RPC methods use camelCase with verb-first naming: `getBlockByHash`, `getPoolPrice`, `getProducerSchedule`, `sendTransaction`, `submitVote`, `backfillFromPeer`, `pauseProduction`. New oracle methods should follow: `getOraclePrice`, `getOracleAttestations`, `getOracleStatus`.

Source: `crates/rpc/src/methods/dispatch.rs:16-72`.

### 5.8 Gossip/Mempool TX Propagation

All TXs follow the same path: constructed off-chain → submitted via `sendTransaction` RPC → admitted to mempool (with admission checks per TX type) → gossipped to peers → included in next block by proposer. Oracle attestation TXs would follow this exact path. No special gossip channel needed.

---

## 6. Open Design Questions (For Evaluators)

### Q1: Attestation Cadence

Which attestation cadence (per-block, per-N-blocks, per-epoch) minimizes manipulation surface while satisfying the latency budget for lending liquidations?
- Per-block (every 10s): lowest latency, highest TX volume (34+ attestation TXs per block × blocksize budget)
- Per-N-blocks (e.g., every 6 blocks = 1 minute): balances latency vs throughput
- Per-epoch (every 360 blocks = 1 hour): too stale for liquidation — likely ruled out

What is the maximum attestation TX size impact on block capacity? At 34 producers × ~200 bytes per attestation TX = ~6.8KB per block. BASE_BLOCK_SIZE is 2MB. Impact < 0.35%.

### Q2: Attestation TX Type vs Coinbase Extension vs EpochState Field

Should attestation be:
- **(a) Separate TX type** (e.g., `PriceAttestation = 16`): cleanest separation, standard mempool propagation, each attestation independently verifiable. Uses 1 free TxType discriminant.
- **(b) Coinbase `extra_data` extension**: block proposer aggregates attestations received during the slot and includes in coinbase. No new TX type. But: gives proposer censorship power over attestations (violates HC-2 determinism if proposer can selectively exclude).
- **(c) EpochState field updated at boundary only**: aggregated once per epoch. Minimal block impact. But: 1-hour latency (unacceptable for liquidation).
- **(d) A combination**: attestation TXs per-block (a), aggregated into EpochState at epoch boundary (c). Gives both low-latency reads AND epoch-level finality.

### Q3: Attestation Independence from Block Proposer

How does an attester prove independence from the block proposer in the same slot? The risk: proposer sees all attestations in their mempool and can (1) censor non-agreeing ones, (2) front-run with their own attestation knowing others' values.

Options for evaluators:
- **(a) Attester != proposer constraint**: the scheduled block producer for slot S is excluded from attesting in slot S. They attest in slot S+1 instead (deferred-slot attestation).
- **(b) Commit-then-reveal**: attesters submit `commitment = BLAKE3(price || nonce)` in slot S, reveal `price || nonce` in slot S+K. Aggregation only possible after reveals. Adds K-slot latency.
- **(c) Attestation aggregation at epoch boundary only**: moot for per-block models.
- **(d) Gossip-level deduplication before proposal**: attestation TXs are gossiped BEFORE the proposer builds the block; proposer must include all valid attestation TXs received (enforced by validation — reject blocks missing valid attestations in proposer's mempool). But: unenforceable unless attestations are separately tracked.
- **(e) Threshold cryptography**: K-of-N BLS threshold signature where no single party (including proposer) sees the aggregated price until threshold is met. Highest anti-collusion, highest complexity.

### Q4: Slashing Trigger for Misreport

What constitutes provable oracle misreport?
- **(a) Deviation from consensus median**: attester reported price that deviates > T% from the final aggregated price. Problem: honest dissenting minority is also slashed.
- **(b) Cross-attestation contradiction**: same attester reported two different prices in the same window. Problem: only catches double-report, not coordinated wrong-price.
- **(c) Off-chain dispute proof**: external party submits evidence that the attested price was > T% from a verifiable external source at that timestamp. Problem: introduces external dependency (who is the authoritative external source?).
- **(d) Retrospective divergence**: if the attested price at time T diverges by > T% from the AMM TWAP at time T+W (where W is a comparison window), slash. Problem: AMM can itself be manipulated.
- **(e) No slashing for honest divergence; only slash for equivocation** (same attester, same window, two prices). Simplest. Weakest deterrent.

### Q5: Sunset Trigger — Concrete Threshold X

At what structural bond share X% does the structural-anchored model lose its integrity guarantee?
- **50%**: structural set loses majority → bond-weighted median can be captured by external coalition. Mathematical boundary.
- **55%**: 5% safety buffer above 50%. Practical recommendation to trigger sunset BEFORE the boundary is crossed.
- **40%**: would trigger sunset while structural set is still a majority (conservative).

What is the fallback behavior when X is crossed?
- **(a) TWAP-only mode**: oracle stops accepting attestations, consumers fall back to AMM TWAP (which has its own manipulation surface — stale, manipulable on low-liquidity pools).
- **(b) Halt new attestations**: oracle goes stale (last aggregated price frozen). Lending continues using last-known price until a governance action restarts. Safest but blocks new lending operations.
- **(c) Switch to rotating committee**: structural-anchor assumption drops; all bonded producers rotate into attestation duty (full decentralized model). Most complex to implement.
- **(d) Emergency halt on lending**: oracle stops, lending halts new positions, existing positions use frozen oracle price for grace period, then auto-liquidate at protocol level.

### Q6: Single Oracle Output vs Tiered Consumer Interface

Should the oracle produce one price or multiple:
- **Single output (simplest)**: one aggregated price per asset pair, one latency class. All consumers read the same value.
- **Tiered output**: fast/provisional price (low latency, low finality guarantee, for AMM TWAP substitution) + slow/finalized price (high latency, high finality, for lending/liquidation). Adds complexity.

### Q7: Aggregation Rule — Quantified Comparison

Bond-weighted median vs alternatives under 37.3% adversary (105,067 DOLI):

| Rule | 37.3% Adversary Impact | 50.1% Adversary Impact |
|------|----------------------|----------------------|
| Bond-weighted median | 0% deviation (needs > 50% to move median) | Complete control |
| Arithmetic mean | Up to 37.3% × max_deviation_submitted | Up to 50.1% × max_deviation |
| Trimmed mean (drop top/bottom 10%) | Reduced impact if attacker in extremes | Still significant |
| Geometric median | Similar to arithmetic median but different edge cases | Complete control |

Evaluators must quantify: for each rule, what is the COST (in DOLI slashed) per UNIT of price deviation achieved, at both adversarial bounds?

### Q8: Anti-Collusion for Non-Rotating Attester Set

If all 34 producers can attest (no rotation), what prevents the 12 structural-set producers from coordinating their attestations off-chain? This is the "centralization disclosure" issue.

Options:
- **(a) Honest disclosure only**: "the structural set CAN coordinate; the system trusts their economic alignment (they'd be slashing their own bonds)." Acceptable for Phase 2.1 IF sunset trigger exists.
- **(b) Cryptographic commit-reveal**: attesters cannot see each other's submissions until after commitment. Prevents coordination but adds latency.
- **(c) Randomized attester subset per window**: randomly select K-of-N attesters each window (using block hash as seed). Prevents targeted coordination because you don't know who's attesting. But: fewer attesters = weaker aggregation.

### Q9: Attestation Reward Funding

Who pays for oracle attestation effort?
- **(a) From reward pool (epoch subsidy)**: carve out X% of epoch rewards for attesters. Pro: no new emission. Con: dilutes producer rewards.
- **(b) From consumer fees**: lending/liquidation operations pay an oracle fee that funds attesters. Pro: users-pay-for-what-they-use. Con: no lending yet = zero revenue = no incentive.
- **(c) Neither (altruistic + slash threat)**: producers attest because (1) they want the oracle to work (DeFi brings fees) and (2) NOT attesting could eventually result in reduced epoch eligibility. Pro: simplest. Con: free-rider problem.
- **(d) From W2 protocol fee**: a fraction of the 5 bps AMM protocol fee routes to attestation rewards. Pro: funded by DeFi activity. Con: creates dependency between AMM volume and oracle security budget.

### Q10: Centralization Disclosure Language

What honest disclosure accompanies the structural-anchored model?

Proposed (evaluators to critique/improve): "DOLI's oracle price in Phase 2.1 is secured by the economic interest of the operator-controlled producer set (N1-N12), which currently holds 62.7% of total bonded stake. The oracle's integrity guarantee depends on this structural majority maintaining honest behavior. This is NOT a decentralized oracle. The security boundary is operator key custody. A sunset trigger at X% structural bond share will transition the oracle to [fallback mechanism]. Users of oracle-dependent DeFi primitives (lending, liquidation) accept this trust model."

---

## 7. Out of Scope (Won't)

| Item | Reason |
|------|--------|
| Smart-contract VM additions | HC-3 violation |
| External oracle integration (Chainlink, Pyth, UMA) | HC-1 violation |
| New oracle/governance tokens | HC-7 violation |
| Reducing/migrating structural set N1-N12 bond share | Separate decentralization workstream; oracle ships on CURRENT bond distribution |
| Frontend/wallet oracle visualization | Downstream of this design pass |
| Lending implementation | Phase 2.3; only the interface contract (how lending READS oracle) is defined here |
| AMM curve changes | Phase 2.4; AMM is a price SOURCE, not oracle consumer |
| Event subscription/push notifications for oracle updates | Pass 2 (Events) dependency |
| Leader auctions / PBS | HC-2 violation |
| Encrypted mempool for attestations | HC-2 violation |
| VRF-based attester selection | HC-2 violation (non-deterministic from validators' perspective) |
| L2 oracle bridges | HC-1 violation |
| Multi-asset oracle (beyond DOLI-denominated pairs traded on L1 AMM) | Phase 3+; requires cross-chain data source which violates HC-1 |
| Changing `defi_activation_height` value | HC-5 explicitly preserved |
| Bumping `CURRENT_PROTOCOL_VERSION` | EI-ORACLE-5 / INC-I-054 lesson |
| Modifying Phase 1 deliverables (P3, P5, P7, P8) | HC-4; additive only |

---

## What I Don't Understand (Intellectual Honesty — MANDATORY)

1. **BLS attestation aggregation infrastructure** — The defi-subsystem spec §D1 Phase 2 sketch mentions "existing BLS infrastructure" and "BLS attestation aggregation." I grep'd for `bls` in the codebase and found attestation-related bitfield encoding/decoding in `epoch_state`, but did NOT verify whether full BLS signature aggregation (threshold signatures, multi-party signing) is implemented in production code or only the simpler bitfield presence tracking. This matters for Q3(e) threshold cryptography feasibility.

2. **State root calculation specifics** — I know oracle state must be in the state root for snap-sync (EI-ORACLE-8), but I did NOT read `crates/storage/src/snapshot.rs` to understand the exact state root composition. Whether adding a new state component (oracle prices) requires modifications to snapshot serialization is an open question.

3. **Mainnet producer count** — The prompt states 34 producers and 12 structural nodes. I verified the 12 structural N1-N12 from CLAUDE.md, but the total producer count of 34 comes from the prompt context, not from a codebase grep. The bond distribution (281,717 total, 176,650 structural) is accepted as a domain fact from the user.

4. **AMM TWAP accuracy at low volume** — The TWAP accumulator (`cumulative_price` in Pool extra_data) updates only on Swap operations. If no swaps occur for many slots, TWAP becomes stale. I did NOT verify how `compute_twap_price()` handles long gaps between swaps (it divides by window_slots, which could undercount if accumulator didn't increment during those slots). This matters for the fallback-to-TWAP-only scenario at sunset.
