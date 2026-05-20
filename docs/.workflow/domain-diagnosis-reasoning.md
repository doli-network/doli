# Domain Diagnostic Reasoning Trace

**INC_ID:** INC-I-083
**RUN_ID:** 345
**Date:** 2026-05-19

---

## Domain Reports Summary

### Fork (127 lines, relevance: HIGH)
- **Relevance assessment**: Fundamentally a fork/divergence problem — fleet split into multiple chain tips.
- **Top hypothesis**: H1 — Single natural tip race at h=110360, amplified by block store height-index corruption and sparse canonical indexes. conf(0.65, measured).
- **Key evidence**: (1) Fork at h=110360: two producers built valid blocks on same parent h=110359, confirmed via RPC. (2) Header desert: only 4-5/18 nodes have blocks at h=110389 in their height index. (3) Block store height-index offset (-1 or -2) on n1, n6, n7, n9 — pre-existing, not from current incident. (4) Some nodes spontaneously recovered; others remain frozen. (5) psHash identical fleet-wide — ProducerSet not diverged.
- **Killed hypotheses**: H4 (two advancing clusters are lag, not fork — KILLED, different blocks at same height), H5 (AH crossing caused fork — KILLED, divergence 801 blocks after activation).
- **Gaps**: Origin of height-index offset; why some nodes self-recover; seed-cluster fork origin; slot gaps indicating production failures.

### Connectivity (177 lines, relevance: LOW)
- **Relevance assessment**: NOT connectivity-starved. Every node has 17 transport peers. Problem is sync state machine, not communication.
- **Top hypothesis**: H5 — Recovery path exhaustion, not connectivity. conf(0.70, measured).
- **Key evidence**: (1) Full 17-peer mesh on every node. (2) --no-snap-sync on n9/n10/n11/n12 in launchd plists. (3) n3 (same fork as n10, snap enabled) recovered; n10 (snap disabled) stuck. (4) Snap quorum failure during fleet divergence (no 2-peer agreement on state root). (5) n7/n8 genuinely gossip-isolated (zero gossip blocks, 1988s silence) but n10/seed receive gossip blocks.
- **Killed hypotheses**: H1 (peer starvation — DEAD), H2 (eclipse pair — DEAD), H4 (restart damage — DEAD).
- **Gaps**: Gossip mesh member identity on n7/n8; why n9/n11 with --no-snap-sync are NOT frozen (they're on canonical chain); seed's fork entry point.

### Parameters (214 lines, relevance: HIGH)
- **Relevance assessment**: Root cause of deadlock PERSISTENCE is in parameters/configuration. Coordinator classifies correctly; dispatch gates block execution.
- **Top hypothesis**: H1 — CoordinatorSnapEscalation not classified as emergency blocks --no-snap-sync nodes. conf(0.70, measured). H2 — SNAP_ATTEMPTS_MAX=3 with no reset creates permanent exhaustion. conf(0.70, measured).
- **Key evidence**: (1) Three independent dispatch gates (Gate 4: snap disabled + non-emergency; Gate 5: snap_attempts >= 3; Gate 1: confirmed_height_floor). (2) STALE_TIP_SECS=300 IS reachable from HeaderFirstSync loop (Rule 2 has no recently_synced precondition). (3) recently_synced=60s is NOT the bottleneck — Rule 2 bypasses it. (4) Gossipsub mesh_n_low=20 > 17 available peers — perpetual underflow. (5) No snap-attempt reset when gap < 50.
- **Killed hypotheses**: H5 (AH edge case — DEAD, psHash identical).
- **Gaps**: Why --no-snap-sync was set on n9-n12; n7's snap-attempt history; n13's confirmed_height_floor origin.

### Code (190 lines, relevance: HIGH)
- **Relevance assessment**: Dead-fork nodes stuck in code-level deadlock — classify() has no escalation path for their state.
- **Top hypothesis**: H1 — recently_synced() precondition creates fundamental coverage hole. conf(0.65, measured). H4 — signal_stuck_fork() is dead code. conf(0.65, measured).
- **Key evidence**: (1) Exhaustive trace of ALL recovery paths: Rule 1 unreachable (recently_synced=false), Rule 2 unreachable (gap<500, no rollback exhaustion), Rule 3 catches all. (2) signal_stuck_fork() has zero production callers. (3) height_fallback_attempted is one-shot. (4) reset_empty_headers() oscillates counter. (5) INC-I-081 fixes are NOT causal — they target different code paths that frozen nodes never reach.
- **Killed hypotheses**: None fully killed, but H2 (counter oscillation) was partially killed — some nodes DO reach 10+ empties.
- **Gaps**: Fork trigger analysis; n2's recovery/re-freeze; cluster A vs seed-cluster relationship.

---

## Domain Relevance Analysis

**3 domains HIGH, 1 domain LOW**. Per classification rules: 2+ domains HIGH means cross-domain causation likely. The LOW domain (Connectivity) correctly ruled itself out as root cause while surfacing critical cross-domain evidence (--no-snap-sync finding, snap quorum failure, n3 vs n10 comparison).

The three HIGH domains each identify a DIFFERENT layer of the problem:
- **Fork**: Identifies the TRIGGER (natural tip race + sparse height indexes)
- **Code**: Identifies the STRUCTURAL DEFECT (classify() coverage hole, dead signals)
- **Parameters**: Identifies the RECOVERY BLOCKADE (dispatch gates preventing snap-sync)

This is a textbook cross-domain layered causation. No single domain tells the full story.

---

## Cross-Domain Causation Analysis

### Causal direction test 1: Could the fork (Layer 1) cause the deadlock WITHOUT the code defect (Layer 2)?
If classify() had a Rule 1.5 that handled dead-fork nodes (not recently_synced, gap < 500) by escalating to ShallowRollback or SnapSync after a timeout, the fork would self-resolve. **YES, the fork alone cannot cause permanent deadlock.** The code defect is necessary.

### Causal direction test 2: Could the code defect (Layer 2) cause the deadlock WITHOUT the fork trigger (Layer 1)?
If a node is on the canonical chain, classify() returns HeaderFirstSync but the headers succeed (valid chain). No deadlock occurs. The code defect is latent — it only manifests when a node is on a dead fork. **YES, the code defect alone cannot cause deadlock.** A fork trigger is necessary.

### Causal direction test 3: Could the parameter gates (Layer 3) cause the deadlock WITHOUT the code defect (Layer 2)?
If classify() correctly escalated to SnapSync, some nodes would still be blocked by --no-snap-sync (Gate 4). But nodes WITHOUT --no-snap-sync (n3, n7, n13, n16, seed) would recover. The parameter gates alone block some nodes but not all. **PARTIALLY — gates alone block --no-snap-sync nodes but not the full fleet.**

### Causal direction test 4: Did the trigger (fork) precede the structural defect's activation?
YES. The fork at h=110360 occurred at slot 218663 (~21:27 UTC). The first COORDINATOR logs showing HeaderFirstSync on frozen nodes appear at ~21:30+. The fork preceded the deadlock by minutes. **Temporal order confirmed.**

### Causal direction test 5: Is there a mechanism connecting each layer?
Fork -> empty headers -> recently_synced expires -> Rule 1 dead -> Rule 3 HeaderFirstSync loop -> classify returns SnapSync after STALE_TIP_SECS -> dispatch gates block snap -> permanent deadlock. **Full mechanism traced with measured evidence at every step.**

**Conclusion**: Three-layer cross-domain causation confirmed. Fork (trigger) -> Code (structural defect) -> Parameters (recovery blockade) -> permanent deadlock.

---

## Convergence Analysis

### Convergence cluster 1: Snap-sync gate blockade (3/4 domains)
- **Connectivity**: Found --no-snap-sync on n9-n12 via plist grep; identified n3 vs n10 as structural difference (measured, log analysis).
- **Parameters**: Found three gates (Gate 4/5/1) via source code tracing at production_gate.rs (measured, code evidence).
- **Code**: Found snap_attempts never reset when gap < 50, snap redirect requires gap > 50 (measured, code evidence).
- **Independence**: Connectivity used plist configuration + log comparison. Parameters used source code line numbers. Code used dispatch.rs + cleanup.rs path analysis. Three completely independent evidence sources.

### Convergence cluster 2: classify() coverage hole (2/4 domains)
- **Code**: Exhaustive 4-rule trace showing no escalation path for dead-fork nodes (measured, code + log evidence).
- **Parameters**: Found that STALE_TIP_SECS path IS reachable but dispatch gates block it (measured, code + log evidence).
- **Independence**: Code focused on classify() rules; Parameters focused on dispatch gates. Different functions, same pipeline.

### Convergence cluster 3: Natural tip race as trigger (4/4 domains)
- All four investigators agree the initial fork is a natural PoS tip race, not caused by the deployed code changes. ProducerSet consistency (psHash identical) rules out INC-I-082 or activation-height divergence.
- **Independence**: Fork measured via RPC block comparison. Connectivity inferred from log timestamps. Parameters measured via psHash check. Code inferred from INC-I-081 commit analysis.

---

## Contradiction Analysis

### Contradiction 1: Connectivity LOW but surfaces critical --no-snap-sync finding
- **Who**: Connectivity (LOW relevance) vs Parameters (HIGH relevance)
- **What**: --no-snap-sync is a parameter/configuration issue surfaced by the connectivity investigator
- **Evidence**: Connectivity found it via plist grep (measured); Parameters found it via source code (measured)
- **Resolution**: Finding correctly belongs to Parameters domain. Connectivity's LOW rating is for its OWN domain (peer connectivity), not for the cross-domain signal. Properly routed.

### Contradiction 2: Code says counter oscillation prevents escalation; Parameters says STALE_TIP_SECS IS reachable
- **Who**: Code H2 vs Parameters Key Evidence #4
- **What**: Whether deep_fork_confirmed can become true
- **Evidence**: Code's H2 kill test found it PARTIALLY true (some nodes reach 10+). Parameters traced the exact code path.
- **Resolution**: Both correct at different stages. Counter oscillation SLOWS escalation but does not permanently prevent it. After 300s+, STALE_TIP_SECS path reaches SnapSync classification. But dispatch gates then block it. The disagreement is about whether classify() or dispatch() is the bottleneck — answer: both, in sequence.

### Contradiction 3: Fork says "some nodes spontaneously recover" vs Code says "NO automatic recovery path"
- **Who**: Fork Key Evidence #5 vs Code Definitive Answer #2
- **What**: Whether automatic recovery exists
- **Evidence**: Fork observed recovery via RPC snapshots (measured). Code traced all code paths (measured).
- **Resolution**: Recoveries are snap-sync successes on nodes with snap-sync available. Code's statement is correct for nodes with all paths exhausted. Different node populations.

---

## Confidence Evolution

1. **Initial read (brief only)**: Suspected recently_synced() gap per handoff hypothesis. Noted the brief warns against anchoring on this.

2. **After Fork report**: Shifted toward "header desert" as amplifier. Natural tip race confirmed as non-code-related trigger. Noted sparse height indexes as a fleet-wide structural issue.

3. **After Connectivity report**: Confirmed connectivity is not the issue. Critical --no-snap-sync finding raised parameter domain importance. n3-vs-n10 comparison is the most actionable evidence in the entire investigation.

4. **After Parameters report**: Three independent dispatch gates identified. Key insight: classify() works correctly (returns SnapSync) but dispatch blocks it. This reframes the problem from "classify doesn't escalate" to "classify escalates but dispatch refuses." Confidence in parameters as contributing domain rose to 0.70.

5. **After Code report**: Exhaustive classify() trace confirmed the coverage hole. But the Parameters finding that STALE_TIP_SECS IS reachable means the code gap is partially bridgeable. The permanent blockade is in the dispatch gates. Confidence in code as primary domain confirmed at 0.65 (the structural defect exists even though it's partially bridged by STALE_TIP_SECS).

6. **Final synthesis**: Cross-domain causal chain confirmed. Primary domain CODE (structural defect makes nodes vulnerable), Contributing domain PARAMETERS (gates make it permanent), Trigger domain FORK (natural event that activates the defect). Overall confidence: conf(0.80, converged) — 3/4 domains converge with independent evidence, all contradictions resolved, full causal chain traced with measured evidence at every step.
