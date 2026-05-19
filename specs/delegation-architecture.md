<!-- OUTPUT CONTRACT: N/A — architecture specification file, not a test -->
<!-- INPUT PARTITIONS: N/A — architecture specification file -->

# Delegation Concentration Architecture (INC-I-078)

> **APPROVED SCOPE (User Gate, 2026-05-17, round 2)**: SSF lead **plus** M4 DelegateBond authentication. The user rejected SSF-alone with the specific reason that the Failure Analyst's confirmed live forgery exploit on DelegateBond is unacceptable. The approved scope ships both as a bundle: cap + auth. Implementation plan in §8; M4 design detail in §7.3 (now promoted from "menu" to "approved").

## Summary

**Approved scope** = two changes, each with its own activation height, deployable as a bundle:

1. **Per-producer `received_delegation_cap`** in `NetworkParams`, checked in DelegateBond validation. All five evaluators converged on this. Adds 2 NetworkParams fields, 1 error variant, ~8 lines of enforcement. Touches one consensus surface (DelegateBond acceptance), zero block-content surfaces. Bounds delegation concentration. See §2.
2. **DelegateBond / RevokeDelegation authentication** via Ed25519 signature appended to `extra_data`. Closes the live forgery exploit (Failure Analyst FM-1: confirmed at file:line; cost ~10 DOLI). Backward-compatible wire format (extra_data append, NOT a new tx type — F3-compliant). Same auth applies to RevokeDelegation (C7). See §7.3.

**Deferred (still in menu, NOT in approved scope)**: M2 weight saturation curve (§7.1) and Q3 Variant-1 force-revoke + cooldown (§7.2.3). Available as future defense-in-depth options.

**Removed from scope after iteration**:
- **Q5 Governance veto** — flagged as "ABSENT" by the analyst but independently confirmed by all five evaluators as INTENTIONAL design. Vote weight uses self-bonds only (`crates/updater/src/params.rs:92-104`); this is a DEFENSE against delegation-amplified governance capture, not a gap.
- **Q3 burning slashing variants (Polkadot/Cosmos-style)** — Q3 deep-dive (re-iteration) found that delegator-principal burning would violate DOLI's UTXO-ownership invariant (CS1). DOLI is architecturally Tezos LPoS (no delegator slash) already. Removed; replaced with documentation hotfix. A single optional non-burning variant (force-revoke + cooldown) is documented in §7.2 but deferred indefinitely.

---

## 1. Evaluation Summary

| Evaluator | Lens | Top Proposal | Confidence | Key Finding |
|-----------|------|-------------|------------|-------------|
| Subtractionist | removal | P3: bounded `received_delegations` via cap | conf(0.65, observed) | Saturation curve unnecessary if cap adopted; governance veto already solved; DeterministicScheduler is dead code |
| Restructurer | boundaries | P2: per-producer cap in validation + defensive storage | conf(0.6, inferred) | `selection_weight_at` is single funnel for all delegation effects; cap needs two-layer enforcement |
| Pattern Matcher | patterns | P1: MAX_DELEGATIONS_PER_PRODUCER mirroring MAX_BONDS_PER_PRODUCER | conf(0.65, observed) | Pattern mirrors INC-I-077; Tezos 1:9 ratio as saturation reference |
| Failure Analyst | failures | M4: DelegateBond auth (highest security priority) + M1: cap | conf(0.65, observed) / conf(0.6, observed) | CRITICAL: DelegateBond forgery is live exploit (zero signatures, zero inputs, confirmed at file:line) |
| Radical Simplifier | minimal | P1: `received_delegation_cap` in NetworkParams (~5 lines) | conf(0.65, inferred) | Cap alone defers all other mitigations; blast radius bounded |

---

## 2. SSF Lead Proposal: Per-Producer Delegation Cap

### 2.1 NetworkParams Fields

```rust
/// Maximum total delegated bonds a single producer can receive.
/// 0 = no limit (pre-activation behavior).
pub received_delegation_cap: u64,

/// Height at which received_delegation_cap enforcement begins.
pub received_delegation_cap_activation_height: u64,
```

These follow the established pattern (e.g., `inc_i_068_weight_filter_activation_height`). The cap is expressed in bond units (the same unit as the `u32` in `received_delegations` entries), consistent with `MAX_BONDS_PER_PRODUCER`.

### 2.2 Enforcement Site

**Decision: Primary enforcement in `crates/core/src/validation/tx_types.rs` (`validate_delegate_bond_data`), with a defensive fallback in `crates/storage/src/producer/set_delegation.rs` (`delegate_bonds`).**

Rationale for this choice:

The Restructurer (P2) proposed two-layer enforcement: primary rejection at validation time (prevents invalid txs from entering blocks) plus defensive check at epoch-apply time (handles race conditions where multiple DelegateBonds targeting the same producer appear in different blocks within the same epoch). The Radical Simplifier (P1) proposed checking only in `set_delegation.rs` (the apply path). The Failure Analyst's analysis supports primary validation rejection because:

1. **Waste prevention**: Validation-time rejection prevents over-cap DelegateBond txs from consuming block space. The storage-only approach would include txs in committed blocks that silently fail at epoch boundary -- the worst UX pattern (Restructurer BF-3 analysis, option b).
2. **Consistency**: All other DelegateBond checks (format, self-delegation, status, existence, available bonds) already live in `validate_delegate_bond_data`. Adding the cap check there is consistent.
3. **Race-condition defense**: Because delegation is epoch-deferred (architecture.md 9.4), two DelegateBond txs in separate blocks within the same epoch could each pass validation individually but together exceed the cap. The defensive check in `set_delegation.rs` catches this edge case at epoch-apply time.

The check reads the target producer's current `received_delegations` sum from ProducerSet. This coupling already exists for other delegation checks in the same function.

### 2.3 Cap Check (Pseudo-code)

In `validate_delegate_bond_data` (primary, ~5 lines):
```rust
if height >= params.received_delegation_cap_activation_height
    && params.received_delegation_cap > 0
{
    let current_total: u64 = target_producer.received_delegations
        .iter()
        .map(|(_, count)| *count as u64)
        .sum();
    if current_total + data.bond_count as u64 > params.received_delegation_cap {
        return Err(ValidationError::DelegationCapExceeded);
    }
}
```

In `delegate_bonds` (defensive fallback, ~3 lines):
```rust
if height >= params.received_delegation_cap_activation_height
    && params.received_delegation_cap > 0
{
    let total: u64 = target.received_delegations.iter().map(|(_, c)| *c as u64).sum();
    if total + bond_count as u64 > params.received_delegation_cap {
        log::warn!("DelegateBond rejected at epoch boundary: cap exceeded");
        return; // silently skip -- tx already in block
    }
}
```

### 2.4 Cap Value and Arithmetic

**The cap should be on TOTAL DELEGATED BONDS (sum), not on NUMBER OF DELEGATORS (count).** The Restructurer's kill test (BF-3) found that a count-based cap enables a griefing attack where an attacker fills the cap with dust (1-bond) delegations. A sum-based cap requires proportional capital to fill.

**Recommended starting value**: Conservative and generous. With `MAX_BONDS_PER_PRODUCER = 3,000` as the self-bond ceiling:

- A cap of `3,000` received bonds means max weight per producer = 6,000 (self + delegated). With 64 producers at max, one maxed producer = 6,000/384,000 = 1.56% of total weight. Well below 33% finality-blocking threshold.
- A cap of `9,000` received bonds (3x self-bond max, mirroring Tezos' 1:9 ratio scaled) means max weight per producer = 12,000. One maxed producer = 12,000/768,000 = 1.56% (same ratio, higher absolute numbers).

**The exact value is a policy decision for the network operator.** The architecture provides the field and the check. The value can be adjusted at a future activation height with a `received_delegation_cap_v2_activation_height`.

### 2.5 Migration: Grandfathering at Activation

**Option A (adopted)**: Producers already over the cap at activation height are NOT forced to shed delegations. The check applies only to NEW DelegateBond transactions after the activation height. Existing over-cap producers cannot receive additional delegations. Natural attrition (RevokeDelegation, bond expiry) brings them down over time.

Rationale: Zero disruption, zero forced state changes, zero retroactive effects. The check is reject-only (no state mutation), making it the simplest possible migration. The Radical Simplifier and Restructurer both independently recommended this approach.

### 2.6 Three-Question Gate

- Q1: Can a user-submittable transaction trigger this? **YES** (DelegateBond is user-submitted).
- Q2: Can any producer-action or attestation pattern trigger it? **YES** (cap affects epoch-boundary processing).
- Q3: Is new behavior bit-identical to old for ALL reachable inputs? **NO** (previously accepted delegations are now rejected above cap).
- **Verdict**: Activation height REQUIRED.

### 2.7 Hard Filter Compliance

| Filter | Status | Reasoning |
|--------|--------|-----------|
| F1: No CURRENT_PROTOCOL_VERSION bump | PASS | No EpochState serialization change |
| F2: No HardForkSchedule entries | PASS | Uses activation height, not fork schedule |
| F3: Block-content changes need synchronized deploy | PASS | DelegateBond tx format unchanged; cap is a validation rule |
| F4: Activation height required | PASS | `received_delegation_cap_activation_height` proposed |
| F5: Bitfield encoder/decoder parity preserved | PASS | No producers removed from active set; only delegations capped |
| F6: No genesis reset | PASS | Forward-only activation |
| F7: No retroactive activation | PASS | Grandfathering preserves all existing state |
| F8: Slot scheduler stays unweighted | PASS | Cap has zero effect on `slot % N` |

### 2.8 Acknowledged Limitations

1. **DelegateBond forgery remains**: The cap bounds the blast radius of a forged delegation (attacker can push a victim's weight to at most cap + self-bond) but does NOT eliminate the attack. A single griefer can still bounce any undelegated producer's delegation around freely below the cap at a cost of 10 DOLI. See section 7.3 (M4: DelegateBond Authentication) for the defense-in-depth option.
2. **Multi-producer Sybil collusion is out of scope**: The cap limits per-producer concentration but not coordinated multi-producer attacks. Registering a Sybil producer requires minimum self-bond + activation delay + operational costs, so the cap is a cost multiplier, not a silver bullet.
3. **Cap value tuning is future work**: The initial cap should be set conservatively generous. If concentration emerges despite the cap, the value can be tightened at a new activation height.

---

## 3. Hard Filters (from Failure Analyst -- apply to ALL proposals)

These are non-negotiable constraints verified against every proposal:

- **F1**: No `CURRENT_PROTOCOL_VERSION` bump. INC-I-054 proved unnecessary bumps cause `delete_epoch_state()` on restart, leading to non-deterministic rebuilds and guaranteed forks on snap-synced nodes.
- **F2**: No `HardForkSchedule` entries. `current_fork_id(u64::MAX)` makes ALL entries active immediately, breaking rolling deploys.
- **F3**: Block-content changes need synchronized deploy. DelegateBond authentication (M4) must use `extra_data` append, NOT a new tx type number.
- **F4**: Activation height REQUIRED for any consensus-visible change that fails the three-question gate.
- **F5**: Bitfield encoder/decoder parity preserved. Saturation MUST NOT produce `weight=0`; self-bond floor >= 1. The `inc_i_068_weight_filter` removes `weight=0` producers from the active set, breaking bitfield parity.
- **F6**: No genesis reset.
- **F7**: No retroactive activation.
- **F8**: Slot scheduler stays unweighted.

**Ordering Constraint (C3 from Failure Analyst)**:

If weight-scaled slashing (M3) is ever adopted, `delegation_auth_activation_height` MUST be activated BEFORE `weighted_slash_activation_height`. Without authentication, an attacker can: forge delegation from victim V to malicious P, then P equivocates, and V's bonds are partially burned without V's consent. This is a wealth-destruction griefing attack.

---

## 4. Convergence Matrix

| Question | Subtractionist | Restructurer | Patterns | Failures | Radical | Convergence |
|----------|----------------|--------------|----------|----------|---------|-------------|
| Q1: Per-producer cap | ALIVE (P3, 0.65) | ALIVE (P2, 0.6) | ALIVE (P1, 0.65) | SURVIVES (M1, 0.6) | SSF candidate (P1, 0.65) | **5/5 STRONG** |
| Q2: Saturation curve | DROP if cap (0.55) | ALIVE (P1, 0.65) | ALIVE (P2, 0.55) | SURVIVES (M2, 0.5) | DEFER | **MIXED** |
| Q3: Weight-scaled slashing | implicit drop | partial KILL (0.4) | DEPRIORITIZED → REMOVE BURNING (0.6, deep) | NEEDS-MORE-DESIGN → V1 OPTIONAL (0.55, deep) | DEFER | **RESOLVED: burning REMOVED; Tezos = current; V1 force-revoke OPTIONAL** |
| Q4: DelegateBond auth | not addressed | ALIVE (P3, 0.55) | ALIVE (P3, 0.50) | CRITICAL (M4, 0.65) | DEFER | **MIXED** |
| Q5: Governance veto | DROP (0.4) | KILLED | FALSE POSITIVE (0.60) | NO CHANGE (0.65) | DEFER | **5/5 KILL** |
| Q6: Activation heights | per-proposal | 4 heights | 4 heights | independent heights | 1 height | **5/5 STRONG** |

### Convergence Independence Checks

**Q1 (Per-producer cap) -- 5/5 convergence:**
```
CONVERGENCE INDEPENDENCE CHECK:
Addition: Per-producer delegation cap
Converging evaluators: All 5
Evidence independence:
  - Subtractionist: kill-test of Vec replacement (P2 KILLED); cap is next simplest
  - Restructurer: BF-3 boundary analysis (where should enforcement live?)
  - Pattern Matcher: external parallel (Cosmos, Polkadot) + internal mirror (MAX_BONDS)
  - Failure Analyst: FM-3 quantification (finality liveness attack)
  - Radical Simplifier: SSF first-principles (single dumbest change)
  INDEPENDENT? YES -- five different analytical paths
```

**Q5 (Governance veto) -- 5/5 kill convergence:**
```
CONVERGENCE INDEPENDENCE CHECK:
Deletion: Remove governance veto from scope
Converging evaluators: All 5
Evidence independence:
  - Subtractionist: vote weight uses self-bonds only per protocol spec
  - Restructurer: kill test -- including delegations INCREASES concentration
  - Pattern Matcher: confirmed current design is feature not bug
  - Failure Analyst: verified params.rs:92-104 code directly
  - Radical Simplifier: governance already decoupled from delegation
  INDEPENDENT? YES -- multiple evidence sources
```

### Q4 Note (DelegateBond Auth)

The Failure Analyst confirmed a CONCRETE LIVE EXPLOIT with code evidence:

- `transaction/data.rs:178-185`: DelegateBondData has zero signature field
- `validation/tx_types.rs:590-621`: validate_delegate_bond_data has zero auth check
- `transaction/core.rs:830-838`: new_delegate_bond creates tx with zero inputs
- Global search for `delegate.*sign`, `delegate.*verify`, `delegate.*auth`: ZERO matches

Anyone who can get a transaction into a block can forge a DelegateBond for any undelegated producer. Cost: 10 DOLI. Severity: Medium (disruption, not theft).

**Trade-off at the SSF gate:**
- **SSF path (this spec's lead)**: Ship cap alone. Accept auth exploit remains (bounded by cap). Ship auth later if attacks observed.
- **Defense-in-depth path (section 7.3)**: Ship cap + auth together. Higher complexity, eliminates forgery.

The user decides at the gate.

---

## 5. Removed from Scope

### Q5: Governance Veto on Outsized Producers

**Status**: UNANIMOUSLY KILLED by all 5 evaluators.

**Rationale**: The analyst's gap #5 flagged governance vote weight as "ABSENT" (delegation weight not included in governance). All five evaluators independently confirmed this is INTENTIONAL DESIGN, not a gap:

- `crates/updater/src/params.rs:92-104`: `calculate_vote_weight()` takes `bond_count` (self-bonds) and `blocks_active` (seniority). No delegation parameter.
- Including delegation weight in governance would INCREASE concentration risk: a producer with majority delegation would gain majority governance power, enabling veto of protocol updates.
- Sybil analysis (Failure Analyst): 100 puppets with 1 bond and 0 seniority = weight 100. One honest producer with 100 bonds and 4 years seniority = weight 400. Sybil is economically dominated.

**Required documentation hotfix**: Add a comment or doc note confirming: "Governance vote weight uses self-bonds only -- specifically to prevent delegation-amplified governance capture."

---

## 6. Documentation Hotfixes

### 6.1 Scheduler Staleness

Two evaluators (Subtractionist, Restructurer) independently flagged a contradiction between documentation and code. Verified by direct code read (scheduler-clarification.md):

- **`specs/protocol.md`** (lines ~983-1027): Describes ticket-weighted selection via DeterministicScheduler as the production mechanism. STALE. Production uses `slot % active_producer_count` in `production/scheduling.rs:446`.
- **`docs/architecture.md`** (section 5.2): Same staleness.

Both must be updated to reflect the actual production scheduler: unweighted round-robin `slot % N`.

### 6.2 DeterministicScheduler Dead Code

`DeterministicScheduler` in `crates/core/src/scheduler.rs:171-184` is not instantiated in any production scheduling path. It exists only in tests. The Subtractionist proposes deletion (P1, conf 0.5) as a separate cleanup item. This is NOT part of the delegation concentration mitigation scope but should be tracked as a low-priority dead-code cleanup.

### 6.3 Tezos-LPoS Slashing Equivalence (added in iteration)

DOLI's current `slash_producer` + `cleanup_all_delegations` behavior — producer's self-bond burns, delegated principal is returned to delegators, delegators lose only weight contribution — is **architecturally equivalent to Tezos LPoS slashing**, and this is the result of DOLI's UTXO-ownership invariant (only the bond UTXO's owner key can spend it).

This was an undocumented intentional design. Action:
- Add to `specs/security_model.md`: "DOLI delegator principal is never slashed. Equivocation slashing burns only the offending producer's self-bond. This is a structural consequence of UTXO ownership (CS1: only the delegator's Ed25519 key can spend their bond UTXOs). Functionally equivalent to Tezos LPoS slashing."
- Add to `docs/architecture.md` slashing section: same note.

---

## 7. Menu (only if SSF rejected with specific reason)

If the per-producer cap alone (section 2) is rejected with a specific reason, the following additional mitigations are available. Each is independently deployable with its own activation height.

### 7.1 M2: Weight Saturation Curve

**Surface**: `selection_weight_at()` in `crates/storage/src/producer/info.rs:390-407`.

**Mechanism**: Replace pure linear addition with a piecewise-linear knee:
```rust
fn selection_weight_at(&self, ..., params: &NetworkParams) -> u64 {
    if height >= params.delegation_saturation_activation_height {
        let max_effective = self_bonds * params.delegation_leverage_ratio;
        let effective = min(delegated_bonds, max_effective);
        self_bonds + effective
    } else {
        self_bonds + delegated_bonds
    }
}
```

The Restructurer confirmed `selection_weight_at` is the SINGLE FUNNEL through which all delegation effects flow to attestation weight, rewards, and RPC. Saturation placed here propagates automatically. The scheduler-clarification verified this does NOT affect slot selection.

**Activation height**: `delegation_saturation_activation_height`

**Three-question gate**: Q1=NO, Q2=YES, Q3=NO. REQUIRED.

**Constraints from Failure Analyst**:
- C2 (INTEGER-ONLY-ARITHMETIC): All weight computations must use integer arithmetic. Piecewise-linear (multiplication + min) is pure integer. Sqrt/log require careful fixed-point or are rejected.
- C4 (SELF-BOND-FLOOR): Saturation MUST NOT reduce effective weight below self-bond count. The proposed formula satisfies this: `self_bonds + min(delegated, max_effective)` is always >= `self_bonds`.

**Unverified gap**: Whether `calculate_epoch_rewards()` in `rewards.rs` calls `selection_weight_at` or computes weight independently. If independent, the saturation curve must be applied in both places. Critical for M2 correctness.

**Trade-offs**: Changes reward economics for all existing delegations above the knee. This is a redistribution event requiring community communication.

### 7.2 M3: Weight-Scaled Slashing — Deep-Dive Verdict

**Surface**: `crates/storage/src/producer/set_lifecycle.rs:165-192`.

The Q3 deep-dive (`docs/.workflow/design-q3-deep-failures.md` + `design-q3-deep-patterns.md`) sharpened the M3 verdict considerably. Three sub-variants were evaluated against DOLI's UTXO-ownership constraint.

#### 7.2.1 Burning variants (Polkadot/Cosmos-style) — REMOVED FROM MENU

**Pattern verdict (conf 0.6, observed)**: Polkadot NPoS and Cosmos SDK slashing **DO NOT PORT** to DOLI. Both assume account-model balance subtraction. DOLI is UTXO-model; only the bond UTXO's owner key can spend it. Burning delegated principal would require a new **consensus-forced UTXO destruction** mechanism — the first and only exception to DOLI's UTXO-ownership invariant (constraint CS1 from the patterns deep-dive). This is a foundational architectural violation, not a parameter change.

Combined with the AUTH-BEFORE-BURN constraint (C3a) — burning without M4 enables wealth-destruction griefing via forged delegations — burning variants are **formally removed from the redesign menu**.

#### 7.2.2 Current behavior IS Tezos LPoS — documentation hotfix

**Pattern verdict (conf 0.6, observed)**: DOLI's current `slash_producer` + `cleanup_all_delegations` is already **architecturally equivalent to Tezos LPoS slashing** — producer's own bond burns, delegators are made whole, delegators lose only the weight contribution. This was an undocumented intentional design.

**Action**: register documentation hotfix in `specs/security_model.md` and `docs/architecture.md` noting that DOLI deliberately follows Tezos LPoS (no delegator principal slash) by virtue of UTXO ownership. This closes the Q3 gap as a documentation issue, not a code issue.

#### 7.2.3 Optional additional penalty — Force-Revoke + Cooldown (Variant 1)

**Failures deep-dive verdict (conf 0.55, observed)**: There is one non-burning, M4-independent slashing extension that survives all filters — **force-revoke delegators on slash AND lock them out of re-delegation for K epochs**. No principal burned. No M4 dependency (constraint C3b is relaxed for non-burning variants).

Mechanism:
- On `slash_producer`, in addition to current `cleanup_all_delegations`, mark each delegator with a `re_delegation_lockout_until: u64` height = current + K epochs (e.g., K=2, ~6 hours).
- Pre-lockout, any DelegateBond from a locked-out delegator key is rejected.
- Griefing arithmetic: attacker pays 10 DOLI to forge a delegation; victim is locked out for 6 hours and loses 0 DOLI principal. Ratio is attacker-unfavorable.

Trade-off:
- Adds a `re_delegation_lockout_until` field to ProducerSet's delegator records → triggers constraint **C9 (SERIALIZATION-COMPAT)** added by the deep-dive. Requires careful `EPOCH_STATE_FORMAT_VERSION` consideration if delegator records are in the state root (open gap #1).
- Adds a low-value, low-cost incentive for delegators to choose producers carefully.

**Status**: OPTIONAL menu item, deferred. Not recommended for first deployment. Adds complexity for a low-frequency event (equivocation never observed on mainnet). Listed here for completeness; the deep-dive's recommendation is **defer indefinitely unless equivocation incidents emerge**.

**Activation height (if ever deployed)**: `delegation_lockout_activation_height`.

#### 7.2.4 Summary

| Variant | Verdict | Reason |
|---|---|---|
| Polkadot-style proportional burn | REMOVED FROM MENU | Violates CS1 (UTXO-ownership) + requires M4 first |
| Cosmos-style proportional burn | REMOVED FROM MENU | Same as above |
| Tezos-style (no delegator slash) | ALREADY IMPLEMENTED | Documentation hotfix only |
| Variant 1: Force-revoke + cooldown | OPTIONAL / DEFER | Survives all filters; low marginal value; defer indefinitely |

**Net effect on the spec**: Section 7.2 no longer "needs more design." Q3 is resolved.

### 7.3 M4: DelegateBond Authentication — **APPROVED (promoted from menu)**

> **Status**: APPROVED as part of the bundle (User Gate, 2026-05-17 round 2). Reason: the live forgery exploit (FM-1) is unacceptable in production. Ships alongside the cap with its own activation height.

**Surface**: `crates/core/src/transaction/data.rs` + `crates/core/src/validation/tx_types.rs`

**Mechanism**: Add signature field to `extra_data` (Restructurer Path A):
```rust
DelegateBondData {
    delegator: PublicKey,     // 32 bytes
    delegate: PublicKey,      // 32 bytes
    bond_count: u32,          // 4 bytes
    signature: Signature,     // 64 bytes (NEW, post-activation)
}
```
Delegator signs `HASH("DELEGATE_BOND" || delegate || bond_count)` with Ed25519 key.

**Why NOT a new tx type number**: F3 filter -- a new type causes old nodes to reject blocks containing unknown types, requiring synchronized deploy. Appending bytes to `extra_data` is backward-compatible in block storage; validation gated on height.

**Additional concern (C7)**: RevokeDelegation MUST also get authentication if DelegateBond does. Same zero-input vulnerability.

**Activation height**: `delegation_auth_activation_height`

**Deploy note**: Set height 500+ blocks in the future (~1.4 hours) to ensure all nodes upgrade before activation.

### 7.4 M5 Cleanup: Delete DeterministicScheduler Dead Code

**Surface**: `crates/core/src/scheduler.rs`

Remove dead code. Not consensus-visible. No activation height needed. Low-priority cleanup separate from delegation concentration scope.

---

## 8. Activation-Height Plan

### 8.1 Approved bundle (this redesign)

| # | Name | Purpose | Ordering | Three-Question Gate |
|---|------|---------|----------|---------------------|
| 1 | `received_delegation_cap_activation_height` | Per-producer delegation cap (§2) | Deploy first OR same-height as #2 | Q1=YES, Q3=NO: REQUIRED |
| 2 | `delegation_auth_activation_height` | DelegateBond + RevokeDelegation Ed25519 signature in extra_data (§7.3) | Deploy first OR same-height as #1 (no strict ordering between them) | Q1=YES, Q3=NO: REQUIRED |

**Ordering between #1 and #2**: No strict ordering is required. Both pass hard filters F1–F8 independently. The Failure Analyst's C3 constraint (AUTH-BEFORE-SLASH) only applies to slashing variants, which are NOT in the approved scope. Recommended: deploy at the **same activation height** to ship the bundle atomically and avoid an interim window where the cap is binding but auth is not (still safe — the forgery exploit predates this redesign — but cleaner for messaging).

### 8.2 Future / deferred heights (NOT in approved scope)

| # | Name | Purpose | Ordering | Three-Question Gate |
|---|------|---------|----------|---------------------|
| 3 | `delegation_saturation_activation_height` | Weight saturation curve (§7.1) | Independent | Q2=YES, Q3=NO: REQUIRED |
| 4 | `delegation_lockout_activation_height` | Q3 V1 force-revoke + cooldown (§7.2.3) | Independent | Q1=YES, Q3=NO: REQUIRED |

(Weighted-burning-slash height removed — variant killed in iteration round 2.)

**Rules**:
- Each height is independent. No bundling beyond the explicit §8.1 same-height co-deployment.
- Each defaults to `u64::MAX` (disabled) until deployment-ready.
- Heights are immutable once crossed on mainnet.
- Pattern: `{descriptive_name}_activation_height` in `NetworkParams`.

---

## 9. Open Gaps

These must be confirmed during implementation:

1. **Apply-path code**: No evaluator could directly read the full `set_delegation.rs` apply path. Exact insertion point needs line-level verification.
2. **Dual-path reward distribution**: Whether `calculate_epoch_rewards()` calls `selection_weight_at` or computes weight independently. Critical for M2, not for SSF lead.
3. **Current delegation distribution on mainnet**: Whether any producer currently exceeds a reasonable cap. Grandfathering makes this non-blocking.
4. **Bond denomination in received_delegations**: The `u32` is assumed to be bond count. If DOLI amount, cap unit changes but check shape is identical.
5. **Mempool re-validation**: Whether mempool evicts over-cap DelegateBond txs upon learning of new blocks.
6. **PendingProducerUpdate ordering**: Must be deterministic (block-height + tx-index) for cap enforcement at epoch boundary (C1).
7. **RevokeDelegation authentication**: Same zero-input vulnerability. If M4 adopted, both tx types need auth (C7).

---

## 10. Complexity Comparison

| Metric | Current | SSF (cap only) | SSF + auth | SSF + auth + saturation | Full menu |
|--------|---------|----------------|------------|--------------------------|-----------|
| NetworkParams fields | 0 delegation-specific | +2 | +4 | +6 | +8+ |
| Error variants | 0 | +1 | +2 | +2 | +3+ |
| Modified functions | 0 | +2 (~8 lines) | +4 (~30 lines) | +6 (~50 lines) | +8+ (~80 lines) |
| New modules | 0 | 0 | 0 | 0 | 0 |
| Activation heights | 0 | +1 | +2 | +3 | +4 |
| Consensus surfaces | 0 | 1 | 2 | 3 | 4+ |
| Deploy risk | N/A | Minimal | Medium | Medium-High | High |

---

## Design Synthesis Quality Gate

```
--- DESIGN SYNTHESIS QUALITY GATE ---
Evaluators completed:           5/5
Deletion convergence items:     2 (Q5 governance: 5/5 kill; saturation if cap: 1/5 conditional drop)
Restructuring convergence:      1 (cap enforcement site: two-layer pattern, 2/5 explicit)
Addition options presented:     4 (M2 saturation, M3 slashing, M4 auth, M5 dead code cleanup)
Failure modes identified:       3 (FM-1 forgery, FM-2 moral hazard, FM-3 finality liveness)
Failure modes applied as filters: 3/3
Radical floor gap:              0 modules (current) -> 0 modules (SSF) -> 0 modules (full menu)
Contradictions found:           2 (scheduler docs vs code; enforcement site: validation vs storage)
Contradictions resolved:        2/2 (scheduler verified by code read; enforcement: two-layer adopted)
Evidence independence verified: YES (5 independent paths for Q1 cap; 5 for Q5 kill)
---
```
