<!--
OUTPUT CONTRACT: N/A — specification file (not a test file)
INPUT PARTITIONS: N/A — specification file (not a test file)
-->

# Oracle Structural-Anchored Economics -- DOLI L1 Phase 2.1

**Date:** 2026-05-25
**Status:** PROPOSAL-ONLY (pending User Gate approval)
**Author:** Antonio Lozada <antonio@omegacortex.ai>
**Mode:** No code, no commits, no real activation heights
**Position:** Additive to Phase 1 (defi-foundations-economics.md S0 -- LOCKED) and AMM-First base (defi-subsystem-architecture.md -- LOCKED)
**Synthesis:** 5-evaluator convergence (Mechanism Skeptic, Adversarial Capital, Sustainability, Oracle/MEV, Governance/Minimal)
**Reasoning trace:** `docs/.workflow/oracle-reasoning.md`

---

## S0 SSF Locked Package

The primary recommendation. This is the minimum viable oracle that passed all 10 economic invariants, all 8 hard constraints, and satisfied 6/7 acceptance criteria (AC-ORACLE-5 latency deferred to Phase 2.3).

### Locked Design (5/5 evaluator convergence unless noted)

| Item | Decision | Confidence | Source |
|------|----------|------------|--------|
| Aggregation rule | Bond-weighted median | conf(0.95, converged) | 5/5 -- unique rule providing 0% deviation at 37.3% adversary |
| Slashing trigger | Equivocation only (same attester, same epoch, two different prices) | conf(0.93, converged) | 5/5 -- no herding, no false positives, provable on-chain |
| TX representation | TxType 16 = PriceAttestation | conf(0.95, converged) | 5/5 -- standard mempool propagation, independent verifiability |
| State storage | OutputType 15 = OraclePrice UTXO (consumed-and-recreated at epoch boundary) | conf(0.93, converged) | 5/5 -- automatic state root inclusion via UTXO set |
| Attestation reward | ZERO in Phase 2.1 | conf(0.90, converged) | 4-5/5 -- altruistic model; structural set self-motivated |
| Anti-collusion | Honest centralization disclosure only | conf(0.82, converged) | 4/5 (Adversarial-Capital dissents; resolved in reasoning trace S2.1) |
| Cadence | Per-epoch (360 blocks = 1 hour) | conf(0.78, converged) | 2/5 direct; SSF tiebreaker applied (resolved in reasoning trace S2.3) |
| Sunset threshold | 55% structural bond share (1-epoch lagged metric) | conf(0.85, converged) | 4/5 on 55%; resolved vs time-based in reasoning trace S2.4 |
| Sunset fallback | HALT (immediate, no grace period) | conf(0.87, converged) | 4/5 -- HALT is the only fallback where attacker gains nothing |
| Consumer interface | Single output per asset pair | conf(0.95, converged) | 5/5 -- zero consumers in Phase 2.1; no tiering needed |
| Activation height | `oracle_activation_height = u64::MAX` PLACEHOLDER | conf(1.0, observed) | MANDATORY per INC-I-075 |
| CURRENT_PROTOCOL_VERSION | NOT bumped | conf(1.0, observed) | MANDATORY per INC-I-054 |
| Governance surfaces introduced | ZERO | conf(0.95, converged) | 5/5 -- all parameters hardcoded |

### NEVER Constraints (carry forward from locked context)

- NEVER bundle `oracle_activation_height` with `amm_activation_height` or any other (HC-6, INC-I-075).
- NEVER bump `CURRENT_PROTOCOL_VERSION` (HC-5, INC-I-054) -- OraclePrice is stored as UTXO, not in EpochState.
- NEVER modify Phase 1 deliverables (HC-4) -- oracle is strictly additive.
- NEVER introduce a new token (HC-7) -- all denominated in DOLI.

### Centralization Disclosure (locked verbatim language)

See S6 below for the full disclosure paragraph.

---

## S1 Design Specification

### S1.1 New TX Type: `PriceAttestation = 16`

**TxType discriminant:** 16 (first free per analyst S5.2).

**Field layout:**

| Field | Type | Description |
|-------|------|-------------|
| `signer_pubkey` | [u8; 32] | Ed25519 public key of the attesting producer |
| `price_cents` | u64 | Attested price in USD cents (e.g., 150 = $1.50) |
| `pair_id` | [u8; 32] | Asset pair identifier: `BLAKE3("ORACLE_PAIR" || pair_string)` where `pair_string` = "DOLI/USD" |
| `epoch_number` | u64 | The epoch in which this attestation is valid |
| `signature` | [u8; 64] | Ed25519 signature over `BLAKE3(pair_id || price_cents || epoch_number)` |

**Validation rules (at `validation.rs`):**
1. Height gate: reject if `current_height < oracle_activation_height` with error code `[ERRTX-ORACLE001]`.
2. Attester must be in `active_producers` with active bonds (from `bond_snapshot` in current EpochState).
3. `epoch_number` must match the current epoch (reject stale/future attestations).
4. `pair_id` must correspond to an existing AMM pool with liquidity >= `MINIMUM_LIQUIDITY` (1000, per D1 locked constant).
5. At most ONE attestation per attester per epoch per pair. Reject duplicates with `[ERRTX-ORACLE002]`.
6. Signature must verify against `signer_pubkey`.
7. Standard TX fee applies (bounded cost -- AC-ORACLE-6 C).

### S1.2 New OutputType: `OraclePrice = 15`

**OutputType discriminant:** 15 (first free per analyst S5.1).

**Field layout (in `extra_data`):**

| Field | Offset | Type | Description |
|-------|--------|------|-------------|
| `price_cents` | 0 | u64 (8 bytes) | Last aggregated price in USD cents |
| `last_update_height` | 8 | u64 (8 bytes) | Block height of last aggregation |
| `contributor_count` | 16 | u16 (2 bytes) | Number of valid attestations aggregated |
| `pair_id` | 18 | [u8; 32] (32 bytes) | Asset pair identifier |

**Total `extra_data` size:** 50 bytes.

**UTXO address:** Deterministic system address: `BLAKE3("ORACLE_PRICE" || pair_id)`. Consistent with existing reward pool address pattern (`BLAKE3("REWARD_POOL" || "doli")`).

**Spend semantics:** Consumed-and-recreated as singleton. Only `apply_block()` at epoch boundary may spend the previous OraclePrice UTXO and create the new one (system TX pattern, analogous to Coinbase/EpochReward). No user TX may spend an OraclePrice UTXO.

**Snap-sync:** OraclePrice UTXO is in the UTXO set -> automatically included in the state root via `snapshot.rs`. No EpochState modification needed. No `EPOCH_STATE_FORMAT_VERSION` bump required (EI-ORACLE-5 satisfied).

### S1.3 Aggregation Rule: Bond-Weighted Median

**Locked from 5/5 convergence.**

At epoch boundary (every 360 blocks), `apply_block()` for the epoch-boundary block:

1. Collects all valid PriceAttestation TXs for the closing epoch from the block chain (all blocks in the epoch).
2. For each attester, takes the LATEST attestation if multiple were included across blocks (should be at most 1 per validation rule, but defense-in-depth).
3. Sorts attestations by `price_cents` ascending.
4. Walks sorted list, accumulating bond weight from `bond_snapshot[attester_pubkey_hash]`.
5. The price at which cumulative weight crosses 50% of total-attesting-weight is the median.
6. Creates new OraclePrice UTXO with the computed median.
7. Consumes the previous OraclePrice UTXO (if one exists; first epoch creates from nothing).

**Why bond-weighted median (evidence from all 5 evaluators):**
- At 37.3% adversary: 0% deviation achievable (infinite cost per unit of deviation). All mean-based alternatives allow proportional deviation.
- At 50.1% adversary: complete median control. This is addressed by the sunset trigger.
- Mechanism-Skeptic Section 3: "Bond-weighted median is the ONLY rule where the cost of ANY deviation at <50% adversary is infinite."
- Adversarial-Capital Table Section 3: "Mean: $2,815 per 1% deviation at P=$1. Median: infinite."
- Oracle/MEV Table 3.3: "Bond-weighted median yields 0 MEV from proposer front-running, censorship, or JIT-bond attacks."
- Governance-Minimal Q7: "No further comparison needed."
- Sustainability Q7: "Set-and-forget. Zero maintenance cost."

### S1.4 Slashing: Equivocation Only

**Locked from 5/5 convergence.**

**Evidence type:** Two `PriceAttestation` TXs with:
- Same `signer_pubkey`
- Same `epoch_number`
- Same `pair_id`
- Different `price_cents`
- Both with valid signatures

**Trigger:** Any node observing both attestations can construct a `SlashProducer` TX (TxType 5) with both attestations as evidence.

**Penalty:** Existing slash infrastructure -- 100% bond burn (`calculate_slash()` at `exit.rs:142-145`), permanent exclusion. Per EI-ORACLE-2.

**What is NOT slashed:**
- Honest deviation from median (prevents herding -- Mechanism-Skeptic Q4).
- Failure to attest (no liveness penalty in Phase 2.1).
- Attestation of a "wrong" price (no external reference to compare against per HC-1).

### S1.5 Cadence: Per-Epoch (Phase 2.1)

Each active producer MAY submit one PriceAttestation TX per epoch (360 blocks = 1 hour). Attestation TXs can appear in any block within the epoch -- they propagate via standard mempool/gossip. Aggregation occurs at the epoch-boundary block.

**Block size impact:** 34 attestation TXs x ~250 bytes = ~8.5KB per epoch. Negligible (0.0024% of 360 blocks x 2MB/block capacity).

**Phase 2.3 upgrade path:** When lending ships, cadence tightens to per-6-blocks (60s) via a new `oracle_cadence_v2_activation_height`. This requires only a constant change (`ORACLE_ATTESTATION_WINDOW: u32 = 6`) and updated aggregation timing. Separate activation height per INC-I-075.

### S1.6 Anti-Collusion: Honest Disclosure

No cryptographic anti-collusion mechanism in Phase 2.1. The structural set (N1-N12) is a single operator -- commit-reveal, threshold crypto, and randomized subsets are all cosmetic against intra-operator coordination (Mechanism-Skeptic Q3 analysis, confirmed by Governance-Minimal Q3, Oracle/MEV Q3).

The defense is: (1) economic alignment (176,650 DOLI bonded + future reward stream); (2) sunset trigger; (3) honest public disclosure.

**Post-sunset upgrade path:** Commit-reveal becomes recommended when structural share approaches 55% or when lending ships. Ships under its own activation height.

### S1.7 Attestation Reward: ZERO in Phase 2.1

No explicit reward. No epoch-pool carve-out. No consumer-fee routing.

**Justification (Sustainability Section 7):** "Phase 2.1 avoids the 'real yield' question entirely by having NO yield." Structural set attests as a strategic investment in protocol value (oracle enables lending which brings W2 fees). Non-structural producers may free-ride -- this is acceptable because bond-weighted median only needs >50% honest weight, and the structural set provides 62.7%.

**Phase 2.3 funding path:** Consumer fees from lending interest spread become the permanent funding source. At 50% protocol share of 5% borrower rate: self-sustaining at 770,880 DOLI lending TVL (3.1% of total supply -- achievable). See Sustainability Section 3.

**Contingent reward (deferred):** If attestation participation falls below 67% of total bond weight, a 5% epoch-pool carve-out activates under its own activation height. Do not pre-solve.

### S1.8 Sunset Trigger

**Threshold:** Structural bond share < 55%.

**Metric computation:** At each epoch boundary:
```
structural_bonds = sum(bond_snapshot[k] for k in STRUCTURAL_PUBKEY_HASHES)
total_bonds_eligible = sum(bond_snapshot[k] for k where bond_age >= 1 epoch)
structural_share = structural_bonds / total_bonds_eligible
```

Where `STRUCTURAL_PUBKEY_HASHES` is a hardcoded constant array of the 12 N1-N12 pubkey hashes (same pattern as `BOOTSTRAP_MAINTAINER_KEYS_MAINNET` in `updater/src/constants.rs`).

**Anti-dilution:** `total_bonds_eligible` excludes bonds younger than 1 epoch (Adversarial-Capital Section 2 recommendation). This prevents flash-registration of Sybil bonds to artificially depress structural share.

**1-epoch lag:** The metric uses the PREVIOUS epoch's bond snapshot, not the current one (Adversarial-Capital: "adds 1 epoch of lead time, preventing same-epoch manipulation").

**Fallback behavior (HALT):**
1. When `structural_share < 0.55`: oracle stops accepting new PriceAttestation TXs at validation time (return `[ERRTX-ORACLE003]`).
2. Last committed OraclePrice UTXO remains readable but is marked stale (consumers check `last_update_height` vs `current_height`).
3. No new lending positions can open (staleness check will fail -- Phase 2.3 design).
4. Recovery requires binary upgrade (new structural set definition, or transition to decentralized attestation model).
5. No grace period, no TWAP interim, no rotating committee. Immediate halt.

**Why HALT not TWAP (Adversarial-Capital evidence):** TWAP on $100K AMM pool is manipulable for $15K-$50K (50-167x cheaper than attested oracle). Full-set committee at sunset allows the dilution-attacker to simultaneously hold 50% of total bonds. HALT is the only fallback where forcing sunset gains the attacker nothing.

### S1.9 New RPC Methods

| Method | Response Fields | Notes |
|--------|----------------|-------|
| `getOraclePrice` | `{ pair_id, price_cents, last_update_height, contributor_count, is_stale, trust_model }` | `is_stale = (current_height - last_update_height > MAX_STALENESS)`. `trust_model = "structural-anchored"`. `MAX_STALENESS` = hardcoded constant (Phase 2.3: 36 blocks; Phase 2.1: epoch-width). |
| `getOracleAttestations` | `{ epoch, pair_id, attestations: [{ attester, price_cents, bond_weight }] }` | Returns all attestations for a given epoch. Useful for transparency/auditing. |
| `getOracleStatus` | `{ active, structural_share, sunset_threshold, sunset_triggered, trust_model, last_update_height, attester_count }` | `trust_model: "structural-anchored"` field for agent-readable disclosure (Governance-Minimal Q10). |

### S1.10 New NetworkParams Field

```rust
pub oracle_activation_height: u64  // PLACEHOLDER = u64::MAX
```

Same pattern as `defi_activation_height`. Set in `NetworkParams::defaults()` for each network variant (mainnet, testnet, devnet). Defaults to `u64::MAX` until explicitly activated.

### S1.11 State Storage

OraclePrice is a UTXO (OutputType 15) in the UTXO set. This provides:
- Automatic state root inclusion (EI-ORACLE-8 snap-sync).
- No EpochState modification (EI-ORACLE-5 no version bump).
- Standard UTXO lifecycle (consume/recreate per epoch boundary).
- Deterministic from block stream (EI-ORACLE-10).

---

## S2 Three-Question Consensus-Shape Checklist (INC-I-075)

For the new `PriceAttestation` TX type:

**Q1: Can any user-submittable transaction trigger this code path?**
**YES.** Attesters (any active bonded producer) submit PriceAttestation TXs to the mempool via `sendTransaction` RPC. These propagate via standard gossip.

**Q2: Can any producer-action or attestation pattern trigger it?**
**YES.** Block proposers include PriceAttestation TXs in blocks. The aggregation logic runs in `apply_block()` at epoch boundary, triggered by the epoch-boundary block proposer's actions.

**Q3: Is the new behavior bit-identical to the old behavior for ALL reachable inputs?**
**NO.** New TX type (16) introduces new validation logic, new state updates (OraclePrice UTXO), and new apply_block behavior at epoch boundary. All nodes must process PriceAttestation TXs identically.

**VERDICT:** `oracle_activation_height` in `NetworkParams` is REQUIRED. At height < `oracle_activation_height`, nodes reject TxType 16 with `[ERRTX-ORACLE001]`. At height >= `oracle_activation_height`, PriceAttestation TXs are processed normally.

This activation height is NEVER bundled with `amm_activation_height`, `defi_activation_height`, or any other (HC-6).

---

## S3 Economic Invariants Locked (EI-ORACLE-1..10)

| Invariant | Spec Field/Constant | Status |
|-----------|---------------------|--------|
| EI-ORACLE-1: Manipulation cost floor at 37.3% | Bond-weighted median aggregation (S1.3) | **SATISFIED** -- 37.3% adversary achieves 0% deviation; cost = infinite |
| EI-ORACLE-2: 100% burn slash | Equivocation-only slashing (S1.4) via existing `calculate_slash()` | **SATISFIED** -- full bond burn, permanent exclusion |
| EI-ORACLE-3: Oracle finality = block finality | OraclePrice UTXO created at epoch-boundary block (S1.2) | **SATISFIED** -- finalized when containing block is final |
| EI-ORACLE-4: Bond withdrawal vesting uniform | Reuses existing Bond OutputType (disc=1); no separate attester bond (S1.4) | **SATISFIED** -- 7-day unbonding + vesting penalty apply to all bonds |
| EI-ORACLE-5: No CURRENT_PROTOCOL_VERSION bump | OraclePrice stored as UTXO, not EpochState (S1.11) | **SATISFIED** -- no EpochState format change, no version bump |
| EI-ORACLE-6: Three-question checklist compliance | S2 above: Q1=YES, Q2=YES, Q3=NO -> activation height REQUIRED and provided | **SATISFIED** |
| EI-ORACLE-7: No new token | All denominated in DOLI (S0, HC-7) | **SATISFIED** |
| EI-ORACLE-8: Snap-sync reproducibility | OraclePrice UTXO in UTXO set -> state root automatically (S1.11) | **SATISFIED** |
| EI-ORACLE-9: Attestation reward sustainability | Zero reward in Phase 2.1 (S1.7) -> zero drain from epoch pool | **SATISFIED** (trivially) |
| EI-ORACLE-10: Deterministic from block stream | Aggregation uses only PriceAttestation TXs from blocks + bond_snapshot from EpochState (S1.3) | **SATISFIED** |

No unsatisfied invariants.

---

## S4 Acceptance Criteria Satisfied (AC-ORACLE-1..7)

| Criterion | Status | Derivation |
|-----------|--------|------------|
| AC-ORACLE-1: Manipulation cost at 37.3% | **SATISFIED** | Bond-weighted median: 0% deviation at 37.3% adversary. Cost = infinite. |
| AC-ORACLE-2: Manipulation cost at 50.1% | **SATISFIED** | Sunset fires at 55% -> HALT before 50.1% adversary is possible. Post-halt: no manipulation surface. |
| AC-ORACLE-3: Attestation reward budget | **SATISFIED** | Zero reward -> 0% of epoch pool. No sustainability concern. |
| AC-ORACLE-4: Sunset trigger | **SATISFIED** | 55% threshold + immediate HALT + 1-epoch lag + anti-dilution. |
| AC-ORACLE-5: Latency budget | **DEFERRED** | Per-epoch = 1 hour latency. Acceptable for Phase 2.1 (zero consumers). Tighten to 60s via new activation height when lending ships (Phase 2.3). |
| AC-ORACLE-6: Agent-readiness 4/4 | **SATISFIED** | (A) 3 RPC methods; (B) deterministic; (C) bounded TX cost; (D) UTXO-based composability. |
| AC-ORACLE-7: Producer revenue non-degradation | **SATISFIED** | Zero carve-out -> zero revenue change for any producer. |

---

## S5 Manipulation Surface (Quantified)

### At 37.3% Adversary (Pre-Sunset)

| Metric | Value | Source |
|--------|-------|--------|
| Total bonds | 281,717 DOLI | Domain fact |
| Structural bonds (N1-N12) | 176,650 DOLI (62.7%) | Domain fact |
| External acquirable | 105,067 DOLI (37.3%) | Domain fact |
| Max deviation under bond-weighted median | 0% | Mechanism-Skeptic S3, all 5 evaluators |
| Cost per 1% deviation | INFINITE | All 5 evaluators |
| Slashing cost if caught | N/A (0% deviation = no trigger) | All 5 evaluators |
| Safe TVL at P=$1 | Unlimited (at 37.3%) | Mechanism-Skeptic S1 |
| Safe TVL at P=$10 | Unlimited (at 37.3%) | Mechanism-Skeptic S1 |

### At 50.1% Post-Sunset Adversary

| Metric | Value | Source |
|--------|-------|--------|
| Capital to reach 50.1% | 140,897 DOLI (105,067 external + 35,830 new) | Mechanism-Skeptic S3 |
| Max deviation under median | 100% (full control) | All 5 evaluators |
| Slashing cost (equivocation) | 140,897 DOLI (100% burn) | Mechanism-Skeptic S3 |
| Safe TVL at P=$1 | $0 (oracle halted at 55%) | Adversarial-Capital S2 |
| Safe TVL at P=$10 | $0 (oracle halted at 55%) | Adversarial-Capital S2 |
| Fallback behavior | HALT -- no manipulation surface | 4/5 converged |

### 3-Key Compromise + Full External Acquisition

| Metric | Value | Source |
|--------|-------|--------|
| Bonds controlled | 44,163 (3 structural) + 105,067 (all external) = 149,230 (53.0%) | Adversarial-Capital S1 |
| Bribe cost for 3 operators | ~2,503,968 x P (includes opportunity cost) | Adversarial-Capital Q4 |
| At P=$1 | ~$2.5M | Adversarial-Capital S1 |
| At P=$10 | ~$25M | Adversarial-Capital S1 |
| Safe TVL (3-operator bribe) | TVL < 2,503,968 x P | Adversarial-Capital S1 |

### Structural Operator Manipulation Threshold

| Metric | Value | Source |
|--------|-------|--------|
| Structural set total bond | 176,650 DOLI | Domain fact |
| PV of future rewards (4yr, 50% discount) | ~3,954,615 DOLI | Mechanism-Skeptic S1 |
| Break-even TVL for manipulation | ~$4.13M at P=$1 / ~$41.3M at P=$10 | Mechanism-Skeptic S1 |
| Recommended safe lending TVL cap | < 50% structural bond value = 88,325 x P | Governance-Minimal S1 |

---

## S6 Centralization Disclosure (Verbatim Public Language)

This paragraph ships with the oracle primitive. It MUST appear in: (1) this spec; (2) `getOracleStatus` RPC response as `trust_model` field; (3) any user-facing documentation for oracle-dependent products.

> **DOLI Trust Disclosure -- Phase 2.1 Oracle**
>
> DOLI's Phase 2.1 oracle price is reported by bonded producers using bond-weighted median aggregation. As of activation, the operator-controlled structural set (N1-N12) holds 62.7% of total bonded stake (176,650 of 281,717 DOLI), giving them unilateral control over the oracle median. The oracle's correctness depends on this structural majority maintaining honest behavior. The security model is operator economic alignment (176,650 DOLI at risk plus a future epoch reward stream valued at approximately 1.98M DOLI per year), NOT distributed consensus. An external attacker with the remaining 37.3% of bonds cannot manipulate the oracle under any circumstances. An automatic sunset fires when structural bond share falls below 55% -- at that point, the oracle halts and the protocol must be upgraded to either restore structural majority or transition to a decentralized attestation model. This is explicitly NOT a decentralized oracle and makes no claim to be one. Users of oracle-dependent DeFi primitives (lending, liquidation) in Phase 2.3 and beyond explicitly accept this trust model.
>
> During Phase 2.1, oracle attestation is funded entirely by the structural set's implicit economic alignment with DOLI value capture. No explicit emission or fee carve-out funds attestation. Oracle compensation becomes fee-funded when lending (Phase 2.3) activates and generates sufficient consumer fees.

---

## S7 Implementation Estimate

| Component | Estimated LOC | Module |
|-----------|-------------|-------|
| PriceAttestation TX type definition | ~30 | `crates/core/src/transaction/types.rs` |
| PriceAttestation validation | ~80 | `crates/core/src/validation/` |
| OraclePrice OutputType + extra_data layout | ~40 | `crates/core/src/transaction/output.rs` |
| Bond-weighted median aggregation in apply_block | ~100 | `bins/node/src/node/apply_block.rs` |
| Equivocation evidence + slash integration | ~50 | `crates/core/src/consensus/exit.rs` |
| Sunset check at epoch boundary | ~30 | `bins/node/src/node/apply_block.rs` or `rewards.rs` |
| oracle_activation_height in NetworkParams | ~10 | `crates/core/src/network_params/mod.rs` |
| getOraclePrice RPC | ~40 | `crates/rpc/src/methods/oracle.rs` (new file) |
| getOracleAttestations RPC | ~50 | `crates/rpc/src/methods/oracle.rs` |
| getOracleStatus RPC | ~40 | `crates/rpc/src/methods/oracle.rs` |
| Mempool admission for TxType 16 | ~20 | `crates/mempool/src/` |
| Structural pubkey_hashes constant | ~20 | `crates/core/src/consensus/constants.rs` |
| **Total** | **~510** | |

**Range:** 400-600 LOC (Governance-Minimal estimated 200-300 for purest SSF; Oracle/MEV estimated ~270; analyst estimated 500-1500 for full design. Converged SSF + specification completions lands at ~510.)

---

## S8 Out of Scope

### From Analyst S7 (preserved)

| Item | Reason |
|------|--------|
| Smart-contract VM | HC-3 violation |
| External oracle (Chainlink, Pyth, UMA) | HC-1 violation |
| New oracle/governance token | HC-7 violation |
| Reducing structural set bond share | Separate decentralization workstream |
| Frontend/wallet oracle visualization | Downstream |
| Lending implementation | Phase 2.3 |
| AMM curve changes | Phase 2.4 |
| Event subscription for oracle updates | Pass 2 (Events) |
| Leader auctions / PBS / encrypted mempool | HC-2 violation |
| L2 oracle bridges | HC-1 violation |
| Multi-asset oracle (cross-chain) | Phase 3+ |
| Changing defi_activation_height | HC-5 |
| Bumping CURRENT_PROTOCOL_VERSION | EI-ORACLE-5 / INC-I-054 |

### Synthesizer Additions

| Item | Reason |
|------|--------|
| Commit-reveal (Phase 2.1) | Rejected 4/5 -- theater against unitary operator; median already eliminates proposer MEV |
| Deviation-based slashing | Rejected 5/5 -- creates herding equilibrium (Keynesian beauty contest) |
| Attestation rewards (carve-out) | Rejected 4-5/5 -- Anchor Protocol pattern (paying for supply before demand) |
| TWAP fallback at sunset | Rejected 4/5 -- 50-167x cheaper to manipulate than attested oracle |
| Tiered consumer interface | Rejected 5/5 -- zero consumers in Phase 2.1 |
| Randomized attester subset | Rejected 5/5 -- reduces statistical power for zero anti-collusion benefit |
| BLS threshold crypto | Rejected 5/5 -- theater against structural operator + infrastructure status unknown |
| Retrospective slashing | Rejected 4/5 -- complex, subjective, governance attack surface |
| Mandatory attestation inclusion rule | Deferred -- enforceability concern (race condition on "known" attestations) |
| Attestation priority in mempool | Deferred -- violates neutral block building; attestation volume is negligible |

### Phase 2.3 Dependencies

| Item | Interface Contract |
|------|-------------------|
| Lending circuit breaker | If `abs(amm_spot - oracle_price) > 10%`, lending freezes new positions |
| Oracle-fee routing | Lending interest spread -> attestation reward sub-pool |
| Staleness flag | `MAX_STALENESS` constant (36 blocks at per-6-block cadence) |
| Cadence upgrade | `oracle_cadence_v2_activation_height` for per-6-block |
| TVL monitoring metric | `getOracleStatus` includes `recommended_max_tvl` |

---

## S9 Fixability Classification (Per-Item)

### Code Items (eligible for `--fix` in future implementation pass)

| Item | LOC Est. | Module |
|------|---------|--------|
| TxType 16 (PriceAttestation) definition + validation | ~110 | core/transaction, core/validation |
| OutputType 15 (OraclePrice) definition + extra_data | ~40 | core/transaction/output |
| Bond-weighted median aggregation | ~100 | node/apply_block |
| Equivocation evidence + slash | ~50 | core/consensus/exit |
| Sunset check (55% threshold + halt) | ~30 | node/apply_block or node/rewards |
| oracle_activation_height in NetworkParams | ~10 | core/network_params |
| Structural pubkey_hashes constant array | ~20 | core/consensus/constants |
| 3 RPC methods (getOraclePrice, getOracleAttestations, getOracleStatus) | ~130 | rpc/methods/oracle |
| Mempool admission for TxType 16 | ~20 | mempool |

### Design Items (require further human decision)

| Item | Decision Needed |
|------|----------------|
| Asset pair scoping | Which pairs? All with pools above MINIMUM_LIQUIDITY? Only DOLI-paired? |
| System TX pattern for OraclePrice consume/recreate | Confirm apply_block system-TX approach matches existing Coinbase/EpochReward pattern |
| Post-sunset transition model | What replaces structural-anchored when sunset fires? (Phase 3 design) |
| Lending interaction with frozen oracle | Grace period before auto-liquidation at frozen price? (Phase 2.3 design) |

### External Items (governance/key-custody/documentation actions)

| Item | Action |
|------|--------|
| Centralization disclosure text (S6) | Publish alongside oracle activation in docs + RPC |
| Per-node bond breakdown verification | Query `getProducers` to verify N1-N12 distribution -- affects 55% threshold protection quality |
| Lending TVL monitoring metric | Documentation + dashboard (not consensus-enforced) |

---

## S10 Milestones

This redesign touches 5+ modules. Milestones per architect.md rules:

| Milestone | Scope | Dependencies | Gate |
|-----------|-------|-------------|------|
| M1: Core types | TxType 16, OutputType 15, extra_data layout, NetworkParams field | None | `cargo build --release && cargo clippy -- -D warnings` |
| M2: Validation + mempool | PriceAttestation validation rules, mempool admission, height gate | M1 | Unit tests for all 7 validation rules |
| M3: Aggregation + state | Bond-weighted median in apply_block, OraclePrice UTXO create/consume | M1, M2 | Integration test: 34 attestations -> correct median |
| M4: Slashing | Equivocation evidence type, SlashProducer integration | M1, M2 | Unit test: two conflicting attestations -> full bond burn |
| M5: Sunset | Structural pubkey_hashes, 55% threshold check, HALT behavior | M3 | Integration test: simulate share drop -> oracle halts |
| M6: RPC | getOraclePrice, getOracleAttestations, getOracleStatus | M3 | RPC tests with trust_model field |
| M7: Disclosure | Documentation, RPC field, spec finalization | M6 | Human review |
