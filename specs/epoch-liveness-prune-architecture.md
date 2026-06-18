# Epoch-Boundary Liveness Prune Architecture (INC-I-116)

> **PROPOSAL-ONLY** -- no implementation code. Produced by 5-evaluator design synthesis.
> Date: 2026-06-18. Domain: consensus-scheduler. Severity: high.

## Problem Statement

DOLI's deterministic leader schedule assigns every registered+bonded producer a ticket range proportional to their bonds. When a large cohort is absent, their ranges become dead air. Observed on mainnet during INC-I-114 recovery: 12 of 57 registered producers present, yet the chain produced only ~21-24% of slots. Crossing epoch boundary E1167 changed nothing because the existing attestation filter inside `derive_at_boundary()` (epoch_state/mod.rs:156-171) is systematically overridden by a deadlock safety floor (mod.rs:209) whose denominator uses the full registered set (`effective_active`=57), not the attested set (12). The filter works; the floor neutralizes it.

**Root-cause reframe:** This is NOT a missing-feature problem. The attestation filter already prunes absent producers at every epoch boundary. The defect is the 2/3 proportional floor at mod.rs:209 computing `new_list.len() < effective_active * 2/3` where `effective_active` includes all registered producers minus multi-epoch ghosts. When 45/57 are absent, `12 < 38` fires and re-includes everyone. The fix is replacing this proportional floor (against the wrong denominator) with an absolute minimum floor.

━━━ RESOURCE COST — SUMMARY — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed)
  Memory:   0 (observed)
  IO:       0 (observed)
  Network:  0 (observed)
  Disk:     0 (observed)
  Latency:  0 (observed)
Inevitability: INEVITABLE
Cheaper alternative: NONE-EXISTS
Why this proposal anyway: Fixes a denominator defect in a pure function with zero runtime cost change.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Evaluation Summary

| Evaluator | Lens | Top Proposal | Confidence | Key Finding |
|-----------|------|-------------|------------|-------------|
| Subtractionist | removal | Replace proportional floor with absolute MIN_PRODUCERS_FLOOR=3 | conf(0.60, inferred) | The attestation filter already prunes; ghost exclusion becomes dead logic once the proportional floor it adjusts is replaced |
| Restructurer | boundaries | Extract `compute_live_producer_list()` shared function | conf(0.65, observed) | 170-line inline dupe in rewards.rs:867-941 is the structural root cause of INC-I-082 class bugs; 3 call sites must agree |
| Pattern Matcher | patterns | Hybrid floor `max(3, attested*2/3)` (Polkadot chilling analogue) | conf(0.60, inferred) | 3-epoch `attested_union` lookback IS the hysteresis (3:1 asymmetry); no new mechanism needed |
| Failure Analyst | failures | 14-filter constraint set; rewards.rs:777 `active.clone()` WILL BREAK | conf(0.70, observed) | rebuild path uses wrong bitfield decode list -- EXISTING latent bug that pruning activates into INC-I-017 shape |
| Radical Simplifier | minimal | Absolute floor + activation gate + rewards sync, ~15 lines | conf(0.65, observed) | Self-referential proportional floor (attested vs attested) always passes; absolute floor is the only correct replacement |

## Convergence Matrix

### Deletion/Replacement Convergence

```
                                Subtraction  Restructure  Patterns  Failures  Radical
Replace proportional 2/3 floor:     Y            Y           Y         Y         Y    -> 5/5 DEFINITE
Use attested_union (3-epoch):       Y            Y           Y         Y         Y    -> 5/5 DEFINITE
Absolute floor (MIN=3):             Y            Y           Y         Y         Y    -> 5/5 DEFINITE
Activation height gate:             Y            Y           Y         Y         Y    -> 5/5 DEFINITE
No version bump:                    Y            Y           Y         Y         Y    -> 5/5 DEFINITE
No new EpochState fields:           Y            Y           Y         Y         Y    -> 5/5 DEFINITE
No new hysteresis mechanism:        Y            Y           Y         Y         Y    -> 5/5 DEFINITE
No grace period needed:             Y            -           -         -         Y    -> 2/5 OPTION
Bond snapshot left inclusive:       Y            Y           Y         Y         Y    -> 5/5 DEFINITE
```

### Restructuring Convergence

```
                                Subtraction  Restructure  Patterns  Failures  Radical
Dedup rewards.rs rebuild path:      Y            Y           -         Y         -    -> 3/5 RECOMMENDED
Delete ghost exclusion (post-AH):   Y            -           -         -         -    -> 1/5 OPTION
```

### Divergences

```
Floor formula:
  Pure absolute (MIN=3):           Subtraction, Restructure, Radical
  Hybrid max(3, attested*2/3):     Patterns
  max(3, registered*1/3):          Failures (F7 adversarial concern)

Grace period:
  Not needed (lookback covers):    Subtraction, Radical
  1-epoch explicit grace:          Patterns (P4, conf 0.55)
  Implicit via lookback:           Failures (FILTER-12)

Rewards dedup timing:
  Prerequisite (before prune):     Restructure (P1), Failures (root cause)
  Separate follow-up:              Patterns (P5)
  Mandatory coupling (lockstep):   Radical, Subtraction
```

## Definite Changes (High Convergence)

- ARCHITECTURAL: Replace proportional 2/3 floor with absolute MIN_PRODUCERS_FLOOR
    Convergence: ALL 5 evaluators independently
    Evidence: `epoch_state/mod.rs:209` -- `if new_list.len() < (effective_active * 2 / 3)` where `effective_active` = all registered minus ghosts. With 12/57 attesting: `12 < 38` fires, overrides the correct filter result. The proportional floor against the attested base is self-referential (always passes, per Radical analysis). The absolute floor is the only structurally correct replacement.
    Confidence: conf(0.88, converged)

    CONVERGENCE INDEPENDENCE CHECK:
      Subtractionist: arrived via dead-code analysis of ghost exclusion interaction (mod.rs:178-223)
      Restructurer: arrived via coupling analysis of the two conflicting responsibilities in derive_at_boundary
      Patterns: arrived via Polkadot minimum_validator_count analogy
      Failures: arrived via adversarial F7 (34% partition attack with ratio-only floor)
      Radical: arrived via first-principles self-referentiality proof
      INDEPENDENT? YES -- 5 different evidence paths to the same conclusion

    Post-activation, derive_at_boundary() step 3 changes from:
    BEFORE (~50 lines, 3 concerns): ghost identification + denominator adjustment + proportional floor + conditional override
    AFTER (~8 lines, 1 concern): if new_list.len() < MIN_PRODUCERS_FLOOR then fallback to top-N by bond weight from active_producers

    Pre-activation code path preserved verbatim for bit-identity (FILTER-08). Both paths coexist behind the activation height gate.

    ━━━ RESOURCE COST — NEGLIGIBLE ━━━
    Dimensions:
      CPU:      0 (observed)
      Memory:   0 (observed)
      IO:       0 (observed)
      Network:  0 (observed)
      Disk:     0 (observed)
      Latency:  0 (observed)
    Inevitability: INEVITABLE
    Cheaper alternative: NONE-EXISTS
    Why this proposal anyway: Fixes the root cause (wrong denominator) with minimal structural change.
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: Gate behind new epoch_prune_activation_height in NetworkParams
    Convergence: ALL 5 evaluators independently
    Evidence: INC-I-075 three-question checklist: Q1=NO (no user tx), Q2=YES (producer/attestation pattern triggers), Q3=NO (behavior differs post-activation). Activation height REQUIRED. Existing pattern: ghost_exclusion_activation_height threaded through EpochDerivationInput (mod.rs:38) and used identically at all 3 call sites (post_commit.rs:303, fork_recovery.rs:713, rewards.rs:883).
    Confidence: conf(0.90, converged)

    New field epoch_prune_activation_height: u64 added to:
    NetworkParams (mod.rs + defaults.rs): mainnet=u64::MAX, testnet=0, devnet=0
    EpochDerivationInput (epoch_state/mod.rs:38): +1 field, not serialized (function parameter struct)

    Gate checked inside derive_at_boundary() keyed on input.height. External producers (~30) need upgrade lead time before mainnet pinning. No genesis reset.

    ━━━ RESOURCE COST — NEGLIGIBLE ━━━
    Dimensions:
      CPU:      0 (observed)
      Memory:   0 (observed)
      IO:       0 (observed)
      Network:  0 (observed)
      Disk:     0 (observed)
      Latency:  0 (observed)
    Inevitability: INEVITABLE
    Cheaper alternative: NONE-EXISTS
    Why this proposal anyway: Activation height gating is a hard constraint per INC-I-075.
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: Rewards.rs parallel filter must apply identical prune logic
    Convergence: 5/5 (Subtraction P5/Q10, Restructure P1/Q10, Patterns AP-3, Failures FILTER-03/FILTER-02, Radical Change 4)
    Evidence: rewards.rs:919-941 mirrors mod.rs:209-223 -- explicitly commented as "same logic as derive_at_boundary" (rewards.rs:861). Additionally, rewards.rs:777 uses active.clone() for bitfield decode instead of epoch_state.producer_list -- this is an EXISTING latent bug (confirmed by Failure Analyst, conf 0.70) that pruning will activate into an INC-I-017 death-spiral shape.
    Confidence: conf(0.88, converged)

    The decode-list bug at rewards.rs:777 is structural: after pruning, active (all registered) differs from producer_list (pruned), so bitfield indices misalign during rebuild, producing wrong attested_sets, wrong prune decisions at the next boundary, and a persistent fork after restart/reorg.

    ━━━ RESOURCE COST — NEGLIGIBLE ━━━
    Dimensions:
      CPU:      0 (observed)
      Memory:   0 (observed)
      IO:       0 (observed)
      Network:  0 (observed)
      Disk:     0 (observed)
      Latency:  0 (observed)
    Inevitability: INEVITABLE
    Cheaper alternative: NONE-EXISTS
    Why this proposal anyway: Parity between canonical and rebuild paths is a consensus-correctness requirement.
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Recommended Changes (Medium Convergence)

- ARCHITECTURAL: Extract shared compute_live_producer_list() to eliminate the 170-line inline duplicate
    Convergence: 3/5 (Restructure P1 conf 0.65, Failures root-cause analysis conf 0.70, Subtraction Q10 lockstep)
    Evidence: rewards.rs:867-941 is a ~170-line inline reimplementation of derive_at_boundary() steps 2-3. It has already caused 3 measured divergence defects (INC-I-082 Defects 1-3). The bitfield decode list at rewards.rs:777 (active.clone()) is a 4th latent defect that pruning activates. DOLI incident history shows parallel reimplementations of consensus logic ALWAYS drift and fork.
    Confidence: conf(0.72, converged)

    CONVERGENCE INDEPENDENCE CHECK:
      Restructurer: arrived via coupling metric (3 call sites, only 2 call canonical function)
      Failures: arrived via tracing the decode-list divergence (rewards.rs:777 vs epoch_state.producer_list)
      Subtraction: arrived via maintenance-hazard analysis of the Q10 lockstep burden
      INDEPENDENT? YES -- coupling lens, adversarial lens, and elimination lens

    Proposed function: compute_live_producer_list(active, attested_union, registered_at, blocks_per_epoch, epoch, height, ghost_ah, prune_ah) -> Vec<PK>

    Both derive_at_boundary() and the rebuild path call this one function. Net: ~-100 lines.

    Verdict on timing -- CO-DELIVERABLE (not prerequisite):
    Ship both the floor fix and the extraction in the same binary, but develop/test as separate milestones (M1 and M2).

    ━━━ RESOURCE COST — NEGLIGIBLE ━━━
    Dimensions:
      CPU:      0 (observed)
      Memory:   0 (observed)
      IO:       0 (observed)
      Network:  0 (observed)
      Disk:     0 (observed)
      Latency:  0 (observed)
    Inevitability: AVOIDABLE
    Cheaper alternative: Manual lockstep update of rewards.rs floor and decode list without extraction
    Why this proposal anyway: Eliminates the structural root cause of INC-I-082 class bugs.
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Options for User Decision

### OPTION A: Floor formula -- pure absolute vs hybrid

**A1: Pure absolute floor MIN_PRODUCERS_FLOOR = 3**
From: Subtraction P2, Restructure P2, Radical P1
Evidence: Simplest. With MAX_FALLBACK_RANKS=2, 3 producers gives every slot 2 candidates from a pool of 3. The proportional component against the attested base is self-referential (always passes) and adds zero value.
Complexity: +1 constant, 0 additional lines
Failure filter: Satisfies FILTER-05. F7 (34% partition attack) residual: an attacker who sustains a partition >3 epochs could theoretically exhaust the lookback and control the 3-producer floor. The 3-epoch inertia is the primary defense, not the floor value.
vs. Radical floor: +0 above minimum viable

**A2: Hybrid floor max(MIN_PRODUCERS_FLOOR, attested_union.len() * 2/3)**
From: Patterns P3
Evidence: Kill-tested across 12/57, 4/100, 2/100 scenarios -- no scenario worse than current floor; strictly superior to A1 in the partial-bitfield-bug case (Failures F6). Retains 2/3 safety property against attestation decode bugs while fixing the denominator.
Complexity: +1 constant, +2 lines (one max() call)
Failure filter: Strictly stronger against F6 (partial bitfield bug shrinking the set monotonically). The 2/3 check against the attested base catches cases where a secondary filter over-prunes the already-attested set.
vs. Radical floor: +2 lines above minimum viable

Synthesis recommendation: A1 (pure absolute) is within 0.05 confidence of A2. Radical tiebreaker favors A1 (simpler). The F6 partial-bitfield-bug scenario is mitigated by the Full Bitfield Decode pillar (v6.17.1), which was the root cause of the original floor's existence. Present both; user decides.

### OPTION B: New-registration grace period

**B1: No explicit grace (rely on 3-epoch lookback)**
From: Subtraction P4 (conf 0.65), Radical (conf 0.60)
Evidence: A newly registered producer who is online and producing will be in attested_sets[0] via accumulate_block() (mod.rs:112 unconditionally inserts the block producer). If they only attest without producing, they face a one-epoch delay. The 3-epoch attested_union lookback gives them 3 epochs to appear. This is a consequence of the attestation-tracking design, not a bug.
Complexity: 0 lines

**B2: Explicit 1-epoch grace via registered_at**
From: Patterns P4 (conf 0.55), Failures FILTER-12
Evidence: Mirrors existing GHOST_EXCLUSION_GRACE_EPOCHS=3 pattern. registered_at is already in EpochDerivationInput (mod.rs:36). A producer registered at epoch N is exempt from pruning at epoch N+1. Prevents the edge case where a producer registers mid-epoch and is pruned before their first full epoch.
Complexity: +1 constant (PRUNE_GRACE_EPOCHS=1), +5 lines

Synthesis recommendation: B1. The lookback covers this. If using attested_union (3-epoch lookback) as the liveness signal, FILTER-12 is implicitly satisfied. An explicit grace adds a second constant with a different semantic for marginal benefit.

### OPTION C: Ghost exclusion deletion

**C1: Keep ghost exclusion as-is (coexisting with prune)**
From: Restructure P4, Failures (FILTER-07: coexistence assumed), Radical (ghost exclusion NOT dead post-fix)
Evidence: Ghost exclusion still adjusts effective_active for the pre-activation code path and for the floor fallback logic when new_list.len() < MIN_PRODUCERS_FLOOR. The Radical evaluator specifically noted: "Ghost exclusion effective_active is NOT dead post-fix -- still needed for the include all non-ghost active fallback when the absolute floor fires."
Complexity: 0 change

**C2: Delete ghost exclusion post-activation (conditional on absolute floor)**
From: Subtraction P3 (conf 0.55)
Evidence: Ghost exclusion's sole purpose is adjusting the proportional floor denominator. With an absolute floor, the denominator adjustment is unnecessary. ~70 lines removable across derive_at_boundary() + rewards.rs. But the "include all non-ghost active" fallback when the floor fires still uses is_ghost() to exclude chronically absent producers from the fallback set.
Complexity: -70 lines (but only after all networks cross activation height)

Synthesis resolution: The Subtractionist and Radical contradict on whether ghost exclusion is dead post-fix. The Radical's evidence is stronger: when new_list.len() < MIN_PRODUCERS_FLOOR, the fallback currently includes "all non-ghost producers" (mod.rs:214-219). Deleting ghost exclusion would change this fallback to "all active producers" including chronically absent ones. Recommend C1 (keep) unless the user explicitly wants the subtraction.

## Constraints (from Failure Analyst)

### 14-Filter Satisfaction Table

| # | Filter | Tag | How Proposal Satisfies |
|---|--------|-----|----------------------|
| 01 | Determinism: derive only from EpochState fields | STRUCTURAL | Uses existing attested_sets[0..3] (persisted, snap-safe). No node-local state. SATISFIED. |
| 02 | Bitfield decode parity: ALL paths use pruned list | STRUCTURAL | post_commit.rs:34 (OK), rewards.rs:92 (OK), assembly.rs:358 (OK). rewards.rs:777 MUST be fixed -- addressed by Definite Change 3 + Recommended Change. SATISFIED. |
| 03 | Rebuild path parity: identical prune logic | STRUCTURAL | rewards.rs:919-941 MUST mirror new floor logic. Addressed by extraction (recommended) or lockstep update (definite). SATISFIED. |
| 04 | Rewards ordering: completed epoch's list before rotation | STRUCTURAL | Existing ordering preserved. rewards.rs:92 reads self.epoch_state.producer_list BEFORE post_commit.rs:354 updates it. NO CHANGE. SATISFIED. |
| 05 | Absolute producer floor | STRUCTURAL | MIN_PRODUCERS_FLOOR = 3 (or hybrid per Option A). SATISFIED. |
| 06 | Activation height gate | STRUCTURAL | New epoch_prune_activation_height in NetworkParams, mainnet=u64::MAX, devnet/testnet=0. SATISFIED. |
| 07 | No version bump | STRUCTURAL | Zero new EpochState fields. No CURRENT_PROTOCOL_VERSION or EPOCH_STATE_FORMAT_VERSION bump. SATISFIED. |
| 08 | Pre-activation bit-identity | STRUCTURAL | Pre-activation code path preserved verbatim inside if height < activation_height branch. SATISFIED. |
| 09 | Undo data covers pre-prune state | STRUCTURAL | Existing epoch_state_snapshot in undo data captures full EpochState. No new fields = no change needed. SATISFIED. |
| 10 | Re-inclusion via attestation, no on-chain tx | STRUCTURAL | post_commit.rs:37-57 "extra" decode path adds pruned-but-attesting producers to attested_sets[0]. SATISFIED. |
| 11 | Bond/status preservation | STRUCTURAL | Prune filters input.active_producers -> new_list. Does NOT modify ProducerStatus, bonds, delegations, or bond_snapshot. SATISFIED. |
| 12 | New-registrant protection | IMPLEMENTATION | Using attested_union (3-epoch lookback): producers have 3 epochs to demonstrate liveness. Implicitly SATISFIED. |
| 13 | Flapping resistance / hysteresis | IMPLEMENTATION | 3-epoch lookback union IS the hysteresis. 3:1 asymmetry (3 epochs to prune, 1 to re-include). SATISFIED. |
| 14 | Epoch-boundary ordering preserved | STRUCTURAL | No change to validate -> apply -> post_commit -> derive -> assign pipeline. SATISFIED. |

## Architecture Maps

### Current Architecture (mod.rs:141-300)

```
derive_at_boundary(prev, input) -> EpochState
  Step 1: bond_snapshot = input.bond_counts.clone()                    [1 line]
  Step 2: attested_union = union(prev.attested_sets[0..3])             [20 lines]
          new_list = active_producers.filter(in attested_union)
  Step 3: effective_active = active_count - ghost_count                [50 lines]
          if new_list.len() < effective_active * 2/3 -> OVERRIDE       <-- BUG
  Step 4: sort new_list by pubkey                                      [1 line]
  Step 5: tier system (ACTIVE_PRODUCERS_CAP)                           [50 lines]
  Step 6: rotate accumulators                                          [10 lines]

  3 call sites:
    post_commit.rs:312  -> calls derive_at_boundary()     [canonical]
    fork_recovery.rs:719 -> calls derive_at_boundary()    [snap sync]
    rewards.rs:867-941  -> INLINE DUPLICATE (170 lines)   [rebuild]
```

### Proposed Architecture (Definite + Recommended)

```
compute_live_producer_list(active, attested_union, registered_at,       [NEW shared fn]
    blocks_per_epoch, epoch, height, ghost_ah, prune_ah) -> Vec<PK>
  if height < prune_ah:
    [existing proportional floor logic, verbatim]
  else:
    new_list = active.filter(in attested_union)
    if new_list.len() < MIN_PRODUCERS_FLOOR:
      fallback to non-ghost active (or all active)
    return new_list

derive_at_boundary(prev, input) -> EpochState
  Step 1: bond_snapshot = input.bond_counts.clone()                    [1 line]
  Step 2: attested_union = union(prev.attested_sets[0..3])             [3 lines]
  Step 3: new_list = compute_live_producer_list(...)                   [1 line call]
  Step 4: sort new_list by pubkey                                      [1 line]
  Step 5: tier system (ACTIVE_PRODUCERS_CAP)                           [50 lines]
  Step 6: rotate accumulators                                          [10 lines]

  3 call sites:
    post_commit.rs:312   -> calls derive_at_boundary()          [canonical]
    fork_recovery.rs:719 -> calls derive_at_boundary()          [snap sync]
    rewards.rs:~870      -> calls compute_live_producer_list()  [rebuild, -170 lines]
```

## Adversarial Analysis: 34%-Bond Partition Attack

**Scenario:** Attacker A controls 34% of bonds. A partitions the network so honest producers (66% bonds) cannot see each other's blocks for >3 epochs (~3 hours).

**Attack progression:**
- Epoch N: Partition begins. Honest producers still in attested_union from epochs N-1, N-2.
- Epoch N+1 boundary: Honest producers still in attested_union. NOT pruned.
- Epoch N+2 boundary: Honest producers in attested_union from epoch N-1 only. NOT pruned.
- Epoch N+3 boundary: Honest producers' last attestation rotated out. attested_union contains ONLY the attacker's cohort. Honest producers PRUNED.
- Attacker now controls 100% of the schedule on their fork.

**Defense stack:**
1. 3-epoch inertia (~3 hours): attacker must sustain the partition for 3+ full epochs.
2. Absolute floor (MIN_PRODUCERS_FLOOR=3): prevents degenerate floor-only cases.
3. Fork choice (weight-based): honest chain with 66% weight wins on reconnect.
4. Monitoring (REQ-PRUNE-011): epoch boundary logs include prune count.

**Residual risk (DOCUMENTED LIMITATION):** A partition sustained beyond 3 epochs CAN exhaust the lookback and allow an attacker to dominate the schedule on their fork. This is inherent to any liveness-based prune with a finite lookback window. Comparable to Polkadot's session-duration attack window (~4 hours) and Cosmos's signed_blocks_window (~19 hours).

## INC-I-075 Three-Question Consensus Checklist

1. **Can any user-submittable transaction trigger this code path?** NO.
2. **Can any producer-action or attestation pattern trigger it?** YES.
3. **Is the new behavior bit-identical to the old behavior for ALL reachable inputs?** NO.

Conclusion: Q2=YES, Q3=NO. Activation height REQUIRED. Satisfied by epoch_prune_activation_height.

## Migration Path

1. **M1 -- Absolute floor + activation gate** (~15 lines new logic in mod.rs, ~10 lines in rewards.rs for lockstep update, +1 constant in constants.rs, +1 field in NetworkParams, +1 field in EpochDerivationInput, 2-3 constructor updates). Pre-activation code path preserved verbatim. Ship with activation heights: mainnet=u64::MAX, testnet=0, devnet=0. No genesis reset. No version bump.

2. **M2 -- Extract compute_live_producer_list() shared function** (~-100 net lines). Refactor rewards.rs:867-941 to call the extracted function. Fix the decode-list bug at rewards.rs:777 (use epoch_state.producer_list instead of active.clone()). Pure refactor -- no behavioral change, no activation height needed.

   BRIDGE: During the interim between M1 code-complete and M2 code-complete, the rewards.rs floor logic at line 919 and decode list at line 777 MUST be manually updated to match the new floor. This is a lockstep-update requirement, not a structural fix. M2 eliminates the need for future lockstep updates.

3. **M3 -- Mainnet activation height pinning** (separate decision session per HC-6/INC-I-075). Requires: all structural fleet nodes upgraded; external producers (~30) notified with upgrade lead time; testnet validation complete.

4. **M4 -- Post-activation cleanup** (future, after all networks cross activation height). Pre-activation proportional-floor branch becomes dead code. Can be removed. Ghost exclusion decision (Option C) revisited.

Forward-only activation: once epoch_prune_activation_height is set to a real value on mainnet and the chain crosses it, the value is IMMUTABLE per INC-I-054.

## Complexity Comparison

| Metric | Current | Radical Minimum | Proposed (Definite + Recommended) |
|--------|---------|----------------|----------------------------------|
| Filters/stages in derive_at_boundary | 6 | 6 | 6 |
| New constants | 0 | 1 (MIN_PRODUCERS_FLOOR) | 1 (MIN_PRODUCERS_FLOOR) |
| New NetworkParams fields | 0 | 1 (epoch_prune_activation_height) | 1 |
| New EpochDerivationInput fields | 0 | 1 (epoch_prune_activation_height) | 1 |
| New EpochState fields | 0 | 0 | 0 |
| New shared functions | 0 | 0 | 1 (compute_live_producer_list) |
| Lines added (derive_at_boundary) | 0 | ~15 (if/else gate) | ~15 (if/else gate) |
| Lines removed (rewards.rs) | 0 | 0 | ~100 (dedup extraction) |
| Net line change | 0 | +15 | -85 |
| Parallel logic copies | 1 (rewards.rs) | 1 (unchanged) | 0 (eliminated) |
| Latent decode-list bug | 1 (rewards.rs:777) | 1 (unfixed) | 0 (fixed) |

SSF Candidate: The Radical Simplifier's minimum (absolute floor + gate + rewards lockstep update, ~15 lines, 0 extraction) satisfies all MUST acceptance criteria and all 14 failure filters. The proposed definite+recommended adds the extraction which REDUCES total complexity by ~100 lines while closing a known INC-I-082-class defect.

## Milestones

| Milestone | Scope | Modules | Behavioral Change | Activation Required |
|-----------|-------|---------|-------------------|-------------------|
| M1 | Absolute floor + activation gate + rewards lockstep | epoch_state/mod.rs, constants.rs, network_params/, post_commit.rs, fork_recovery.rs, rewards.rs | YES (post-activation) | YES (epoch_prune_activation_height) |
| M2 | Extract compute_live_producer_list + fix decode-list bug | epoch_state/mod.rs (or new submodule), rewards.rs | NO (pure refactor) | NO |
| M3 | Mainnet activation height pinning | network_params/defaults.rs | YES (pin height) | Inherent |
| M4 | Post-activation cleanup (dead pre-activation branch) | epoch_state/mod.rs, rewards.rs | NO | NO |

## Design Synthesis Quality Gate

```
Evaluators completed:           5/5
Deletion convergence items:     1 (proportional floor replacement, 5/5 agreement)
Restructuring convergence:      1 (rewards dedup extraction, 3/5 agreement)
Addition options presented:     3 (floor formula, grace period, ghost exclusion)
Failure modes identified:       14 (from Failure Analyst)
Failure modes applied as filters: 14/14
Radical floor gap:              current(6 stages + 1 bug) -> radical(6 stages + 15 lines) -> proposed(6 stages - 100 lines)
Contradictions found:           2
  1. Ghost exclusion post-fix: Subtraction says dead, Radical says needed for floor fallback
     RESOLVED: Radical evidence stronger (fallback path uses is_ghost at mod.rs:214-219)
  2. Rewards dedup timing: Restructure/Failures say prerequisite, Patterns says follow-up
     RESOLVED: CO-DELIVERABLE (ship together, develop as separate milestones)
Contradictions resolved:        2/2
Evidence independence verified: YES (5 different analytical lenses, different evidence paths)
```
