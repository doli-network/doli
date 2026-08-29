━━━ FINDINGS — 8 total (DECISION:8) ━━━

  [F1] DECISION conf(0.90, converged) — bins/node/src/node/network_events.rs:552,329 — derive attester weight + membership from the local ProducerSet at BOTH attestation ingresses (one shared helper); closes INC-I-191 + un-ticketed A2 DoS
  [F2] DECISION conf(0.85, converged) — crates/core/src/finality.rs:133 — eliminate the depth-0 finality grant: finalize only at depth ≥ 2 (≥1 locally-applied descendant), 67% threshold unchanged; closes INC-I-190 D1 (timing half)
  [F3] DECISION conf(0.85, converged) — crates/core/src/epoch_state/floor.rs:143-191 — bound the MIN_PRODUCERS_FLOOR fallback: last-known-good prev.producer_list instead of the uncapped registry; closes INC-I-190 D2 (AH-gated)
  [F4] DECISION conf(0.85, converged) — class-level — adopt INV-AUTH-002 + per-seam enforcement; REJECT the class-wide Authenticated<T> wrapper and the unified ingress facade (REQ-AUTH-021 → Won't)
  [F5] DECISION conf(0.75, converged) — crates/core/src/validation/{registration.rs:85-100,tx_types.rs:547-605} — DERIVE bond_count and withdrawal authorization from the tx's own signed inputs/outputs; closes INC-I-177 + INC-I-182 (AH-gated)
  [F6] DECISION conf(0.70, converged) — crates/core/src/transaction/core.rs:517-604 — bind residual extra_data payloads to the tx signer via a domain-separated payload-region digest folded into the input-signature preimage, tx-type-gated (authmsg.rs:99 discipline); closes INC-I-169, completes INC-I-176 (AH-gated, after M2.5, after G1)
  [F7] DECISION conf(0.70, converged) — crates/core/src/validation/transaction.rs:409-427 — constrain Bond outputs at creation (owner = registering producer; amount = whole multiple of required_bond); closes INC-I-184 (AH-gated, strictly after G1)
  [F8] DECISION conf(0.70, converged) — crates/core/src/attestation.rs:14-33,181 — phase-2 wire subtraction: delete attester_weight + height from the Attestation struct and delete dead RegionAggregate/from_attestations (synchronized deploy / own AH)

  Speculative: 4 (Options for User Decision — below conf(0.7), not auto-actionable: snap-anchor variant, C-snap discard, Seam E gate, full finality-gadget deletion)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

# Network-Input Authentication Architecture

> RUN_ID 531 | INC scope: 191, 178, 169, 160, 182, 184, 177, 176, 181, 155 + INC-I-190 D1/D2 + one un-ticketed finding | proposal-only (no --fix)
> Synthesized from 5 independent design evaluations (subtraction, restructure, patterns, failures, radical) over `main` (9e27bd19).
> Reasoning trace: `docs/.workflow/architecture-reasoning.md`. Analyst requirements: `docs/redesigns/network-input-authentication-redesign-analysis.md` (REQ-AUTH-001..024).
> NOTE: the INC-I-190 diagnosis report file was removed from disk mid-workflow; the authoritative verdict is `incidents.root_cause` for INC-I-190 in `.omega/memory.db`. Every load-bearing claim below is independently code-cited.

## Problem Statement

Ten open incidents are one defect class: **a node accepts a network-supplied value (weight,
identity, ownership, payload, state) and feeds it into a consensus decision without checking it
against what the node already knows.** The class has a precise anti-pattern name (pattern lens):
*self-declared authority / bearer-assertion of privilege*. In almost every case the node already
holds the correct value in local authenticated state and chose to believe the message instead.
The defect lands in 5 mechanically distinct seams (A gossip-attestation, B/C extra_data-signing,
C-snap install, D snap-anchor, E fork-choice admission) plus 2 amplifiers (D1 depth-0 finality,
D2 uncapped floor) — but collapses to **2 rules + 1 subtraction + 1 orthogonal safety fix**
(radical lens, adopted).

**Why a redesign and not ten patches:** the last two point-fixes for this exact shape have
already recurred once (memory.db root_cause evidence, pattern lens): the INC-I-116 floor fix left
`new_list = active_producers.to_vec()` → re-fired as INC-I-190 D2; the INC-I-139 FORK_GUARD fix
left "finality-lock has no non-destructive exit" → re-fired as INC-I-190 D1. (No prior redesign
runs of this domain exist — `workflow_runs` has zero `type='redesign'` rows; recurrence here is
point-fix recurrence, which is exactly what a class-level invariant stops.)

**The migration IS the trigger (failures lens, verdict-corroborated):** a rolling fleet restart
of ANY version reproduces the INC-I-190 wedge→snap ladder until D1/D2 land. This spec's
migration path is therefore ordered so the first deploy carries the finality-safety fix.

━━━ RESOURCE COST — SUMMARY — COST-DECLARED ━━━
Dimensions:
  CPU:      +O(1) ProducerSet lookup per attestation plus one BLAKE3 pass per rare payload tx (inferred)
  Memory:   -bounded minute_tracker, capped at active-set size vs unbounded today (observed)
  IO:       0 (observed)
  Network:  -16 B per attestation after phase-2 wire deletion, zero new tx bytes (measured)
  Disk:     0 (inferred)
  Latency:  +1-2 slots (~10-20 s) to finality grant, the deliberate safety delay (inferred)
Inevitability: AVOIDABLE
Cheaper alternative: keep depth-0 finality and ship only Seam A weight-binding (zero latency delta) — rejected: INC-I-190's 67% was HONEST weight (memory.db verdict), so depth-0 self-finality re-fires on every fleet restart regardless of weight authentication.
Why this proposal anyway: the finality delay IS the fix for the reset class; every other dimension is flat or negative because the design is subtraction-first (derive/delete, not wrap/verify).
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## The Invariant and the Rule Taxonomy

> **INV-AUTH-002 (proposed; record in memory.db `invariants` with per-seam `regression_tests`):**
> **A value that arrives over the network may influence a consensus decision ONLY if the node
> DERIVES it from local authenticated state (ignoring the wire), OR the wire carries a
> signature/commitment BINDING it to an authenticated on-chain identity, collateral, or the
> state root. Everything else is SUBTRACTED from the wire.**

| Rule | Meaning | Sites (6+4+1) | Incidents |
|---|---|---|---|
| **DERIVE** | ignore the wire value; read local state | A1, A2, D1-numerator, C2, D2, E-eligibility | 191, 190-D1(weight), 177, 190-D2, 160(part) |
| **BIND** | demand a commitment to the signer (generalize `authmsg.rs:99`) | C1, C3-authz, C4, D-anchor | 169, 182, 184, 155, 160(sig) |
| **SUBTRACT** | delete the un-rooted/forgeable field | B1 `pending_updates`; phase-2 `attester_weight`+`height` | 181, 191(residue) |
| **FINALITY-SAFETY** (separate invariant, NOT authentication) | no depth-0 finality + a survivable exit | G1, G2 | 190-D1(timing), 190-D2 |

Load-bearing insight (radical, verdict-corroborated): **D1 is two defects fused** — its numerator
is untrusted weight (DERIVE, part of Seam A); its depth-0 timing is a finality-safety defect that
persists even with perfectly authentic weight. They are fixed and named separately.

## Evaluation Summary

| Evaluator | Lens | Top proposal | Confidence | Key finding |
|---|---|---|---|---|
| Subtractionist | removal | derive weight + delete forgeable attestation fields | conf(0.7, measured) | 8 of 12 Must requirements collapse to subtraction/derivation |
| Restructurer | boundaries | one chokepoint helper for both A-ingresses | conf(0.7, observed) | denominator already derives correctly 20 lines from the trusting numerator |
| Pattern-Matcher | patterns | Ethereum validator-lookup for Seam A | conf(0.7, measured) | class = self-declared authority; two prior point-fixes for this shape already recurred |
| Failure-Analyst | failures | G1 ≺ Seam B sequencing constraint | conf(0.7, measured) | the poison-rollback "lifeboat" is a bug coupled to 3 open bugs — fixing them first ⇒ 27/27 fleet snap |
| Radical-Simplifier | minimal | 5 seams collapse to DERIVE/BIND/SUBTRACT + safety | conf(0.6, observed) | Seam A is necessary-but-NOT-sufficient for the reset class; D1 must be split |

## Convergence Matrix

```
                                          Sub  Res  Pat  Fail  Rad   verdict
Seam A derive (both ingresses):            Y    Y    Y    Y     Y    5/5  DEFINITE  [F1]
G1 no depth-0 finality:                    Y*   -    Y    Y     Y    4/5  DEFINITE  [F2]   (*via stronger deletion variant, kept as O4)
G2 bound floor fallback:                   Y    Y    Y    Y     Y    5/5  DEFINITE  [F3]
Reject Authenticated<T>/facade:           (i)   Y    Y    -     Y    3/5+2i DEFINITE [F4]
C2/C3 derive from signed structure:        Y    Y~   -    -     Y    3/5  RECOMMENDED [F5]
Seam B payload commitment (residual):      ~    Y    Y   cls    Y    3/5  RECOMMENDED [F6]
C4 bond owner/amount constraint:           Y!   Y    -   cls    Y    3/5  RECOMMENDED [F7]
Phase-2 wire deletion + dead RegionAgg:    Y    Y    -    -     Y    3/5  RECOMMENDED [F8]
C-snap discard pending_updates:            Y    Y    n    Y     Y    4/5 but Q3-gated → OPTION O2
Snap anchor NOT maintainer-signed:         -    -    Y    Y     Y    3/5  CONSTRAINT; variant → OPTION O1
Seam E pre-fork-choice gate:               Y!   Y    -   cls    Y    3/5  OPTION O3 (genuine addition)
Delete finality gadget entirely:           Y    -    -    n     -    1/5  OPTION O4 (conf 0.5 after filters)
```
(Y~ = same goal, different mechanism; Y! = concedes as genuine ADD; cls = classified only;
i = implicit; n = argued against.) Independence verified per cluster (see reasoning trace):
shared code sites, independent reasoning chains → true convergence; boosts applied.

## Definite Changes (High Convergence)

- ARCHITECTURAL: Seam A — the finality numerator and attendance-tracker admission derive from the local ProducerSet at BOTH attestation ingresses, through ONE shared helper; the wire's self-declared `attester_weight` stops being load-bearing.
    Convergence: 5/5 (Subtractionist P1, Restructurer P1, Pattern-Matcher P1, Failure-Analyst F2, Radical P1) — independent reasoning chains, sites re-verified by the synthesizer.
    Evidence: `bins/node/src/node/network_events.rs:552` (`sync.add_attestation_weight(&attestation.block_hash, attestation.attester_weight)` — wire trusted verbatim) and `:329-345` (`DirectAttestation` → `minute_tracker.record` on an arbitrary pubkey); `crates/core/src/attestation.rs:104-117` (`verify()` signs only `block_hash‖slot`, no membership check); the correct derivation already exists in the denominator (`bins/node/src/node/apply_block/state_update.rs:150-171`).
    Confidence: conf(0.90, converged)
    Shape: helper `authenticated_attester_weight(ps, att, audit_ah) -> Option<u64>` — `None` if `!is_active(att.height)` (reject + never record; INV-ATTEST-001: gate on `is_active()`, NEVER `weight==0`); `Some(selection_weight_at(att.height, audit_ah))` otherwise. Both ingresses route through it (a fix on one is bypassable). Bounds `minute_tracker` to the active set (removes the un-ticketed unbounded-key memory DoS). Denominator untouched. Deploy: restart-only IFF the F2 mixed-version experiment passes, else AH — both paths stay open by design. Closes INC-I-191 (A1+A2).

    ━━━ RESOURCE COST — COST-DECLARED ━━━
    Dimensions:
      CPU:      +1 O(1) ProducerSet lookup per attestation replacing a field read, same lookup as state_update.rs:157 (observed)
      Memory:   -moderate, minute_tracker keys bounded by active-producer count instead of attacker-chosen (observed)
      IO:       0 (observed)
      Network:  0 (observed)
      Disk:     0 (observed)
      Latency:  +sub-microsecond per attestation ingest (inferred)
    Inevitability: INEVITABLE
    Cheaper alternative: NONE-EXISTS
    Why this proposal anyway: the only unforgeable weight source is local state already in hand at both call sites; the lookup is strictly cheaper than the unbounded map growth it removes.
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: G1-depth — eliminate the depth-0 finality grant: a block may finalize only after at least one locally-applied descendant exists (depth ≥ 2); the 67% threshold is UNCHANGED. This is a finality-SAFETY change (own invariant, INV-PROD family), not an authentication change.
    Convergence: 4/5 (Failure-Analyst F3, Pattern-Matcher P4, Radical safety-split, Subtractionist via deeper variant O4). Contradiction C-1 resolved against "Seam A alone suffices": the INC-I-190 weight was HONEST (memory.db verdict: honest adjacent-seniority tie, self-finalized 223 ms at ~72% = 16804/23189 [E1]); a Seam-A-fixed node derives the SAME numerator, so the reset class requires G1.
    Evidence: `crates/core/src/finality.rs:133` (`check_finality` finalizes at ≥67% with no depth requirement); `crates/network/src/sync/manager/recovery.rs:355,391` (rollback refused below finality — the wedge); memory.db INC-I-190 root_cause (223 ms depth-0 self-finality → 18-min reorg refusal → gap>50 → 14-node snap).
    Confidence: conf(0.85, converged)
    Why depth ≥ 2 and not more: finality feeds archive flush (`network_events.rs:566`) and the production gate; raising the threshold or requiring deep confirmation stalls finality during a D2 blackout → future snap body holes (failures LIVENESS filter). A same-height sibling tie physically cannot self-finalize when neither sibling has a child. Deploy: restart-only under the strictly-safer argument IFF the same F2 experiment passes (isolated run — binary differs ONLY in G1); else AH. Closes INC-I-190 D1 (timing half; weight half closed by Seam A).

    ━━━ RESOURCE COST — COST-DECLARED ━━━
    Dimensions:
      CPU:      +1 depth comparison per finality check, values already in FinalityTracker.pending (inferred)
      Memory:   0 (observed)
      IO:       0 (inferred)
      Network:  0 (observed)
      Disk:     0 (inferred)
      Latency:  +1-2 slots (~10-20 s) to finality grant (inferred)
    Inevitability: AVOIDABLE
    Cheaper alternative: keep depth-0 finality and rely on Seam A + G2 alone — rejected because depth-0 self-finality of an HONEST tied sibling is the exact INC-I-190 trigger and is not a weight-binding problem.
    Why this proposal anyway: one child-confirmation is the minimal subtraction that makes a tie non-self-finalizing without touching the 67% threshold.
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: G2 — bound the MIN_PRODUCERS_FLOOR fallback: on attestation blackout / below-floor, reuse the last-known-good `prev.producer_list` instead of admitting the entire registry uncapped. Removes the fork-generator that produced the INC-I-190 tied sibling (active set 30→84, adjacent ranks 48/49).
    Convergence: 5/5 on bounding (Subtractionist P3, Pattern-Matcher P5, Failure-Analyst G2 row, Restructurer + Radical signals). Mechanism: radical tiebreaker fired — last-known-good preferred over cap-to-N (zero new constants; deletes the admit-everyone branch; inductive floor-preservation proof, base case genesis, epoch ≤ 1 already special-cased at `mod.rs:256`).
    Evidence: the uncapped `new_list = active_producers.to_vec()` branch, at `crates/core/src/epoch_state/mod.rs:78-93` when this finding was written; INC-I-116 root_cause recurrence (memory.db — same shape, second firing).
    Implemented (M4): the floor stage was extracted to `crates/core/src/epoch_state/floor.rs`; the bounded fallback is `bounded_fallback()` (floor.rs:143-191), gated by `inc_i_190_floor_bound_activation_height` and reported to callers via `FloorOutcome`. Below the AH the legacy uncapped branch is retained byte-identical (`legacy_fallback()`, floor.rs:127-141). Two consensus-visible refinements landed in the same milestone and are inside the same AH gate: preference (a)'s candidates are DEDUPLICATED before the `>= MIN_PRODUCERS_FLOOR` test (AUDIT-P2-502 — a peer-supplied `prev` need not hold distinct keys, and one repeated key could otherwise satisfy the floor alone and then occupy several `active_list[slot % len]` positions), and an ALL-GHOST last resort (floor.rs:181-187) returns the full seniority-ordered active set when every active producer is a ghost — ghost exclusion yields to liveness there, because an empty `producer_list` stalls the chain permanently, which is the deadlock the floor exists to break.
    Mitigation window (M4, NODE-LOCAL — not consensus, not persisted, not AH-gated): a rebuild (`rewards.rs`) or snap (`fork_recovery.rs`) derivation that takes a floor fallback may produce a `producer_list` the fleet did not compute, so the node validates gossip in `ValidationMode::Light` until the divergence can no longer propagate. `FloorOutcome::PreviousEpochList` is the ONLY floor exit that reads `prev.producer_list`, so any other outcome disarms the window; a sustained blackout is capped at `FLOOR_FALLBACK_WINDOW_MAX_BOUNDARIES` (2) consecutive pinned boundaries, after which the node forces itself back to `Full` and logs at `error!`. Bounded validation against a possibly-divergent list is strictly safer than unbounded skipping of VDF and producer eligibility: the residual mismatch is node-local, is not in the state root, and self-heals at the next non-floor boundary.
    Deploy control (REV-I190-M4-F11): the testnet AH is a compile-time pin and its unit guard is a FLOOR measured at pin time, NOT a liveness check. Re-measure the live tip with `getChainInfo` immediately before deploying and re-pin if the headroom is under ~2 h — a gate crossed pre-deploy is a retroactive consensus change (INC-I-054 class).
    Confidence: conf(0.85, converged)
    Deploy: AH-gated — consensus-visible `active_producers` shape (INV-CONSENSUS-001). Its OWN NEW `*_activation_height` (`epoch_prune_activation_height` is crossed → immutable → NEVER reuse). INV-12 three-question checklist answered in the commit message. Below-AH byte-identical. Residual: an exited producer may be scheduled one extra epoch (bounded liveness cost, not the D2 harm). Closes INC-I-190 D2.

    ━━━ RESOURCE COST — NEGLIGIBLE ━━━
    Dimensions:
      CPU:      0 (inferred)
      Memory:   0 (inferred)
      IO:       0 (inferred)
      Network:  0 (inferred)
      Disk:     0 (inferred)
      Latency:  0 (inferred)
    Inevitability: AVOIDABLE
    Cheaper alternative: cap-to-N seniority-ordered fallback — equal runtime cost; last-known-good chosen because it introduces no new constant and no selection rule.
    Why this proposal anyway: the de-amplification is free — the last-known-good list is already materialized on `prev`; one Vec clone replaces another of equal-or-smaller size.
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: adopt INV-AUTH-002 as the class-level enforcement (memory.db `invariants` row + one linked mutated-field `regression_tests` row per seam) and REJECT both the class-wide `Authenticated<T>` wrapper type and the unified authenticated-ingress facade (REQ-AUTH-021 → Won't). Retain exactly ONE narrow chokepoint: Seam A's `authenticated_attester_weight` (justified because two ingresses must not diverge — not because a type is needed).
    Convergence: 3/5 explicit rejection (Restructurer P5, Pattern-Matcher P5, Radical P4) + 2 implicit (neither Subtractionist nor Failure-Analyst proposed any wrapper).
    Evidence: the 5 seams carry disjoint value kinds (u64 weight / sighash digest / serialization field-set / fork-choice admission / Vec bound) — no shared `T` (restructure enumeration); field-DELETION is a stronger guarantee than a wrapper — the regression becomes uncompilable (radical kill test); Ethereum enforces the same property by consistent lookup + invariant, not a god-facade (patterns).
    Confidence: conf(0.85, converged)
    Effect: one generic type, one trait, one facade, and N migrations avoided. A future 6th ingress is held to INV-AUTH-002 by the documented invariant + the INV-12 consensus-shape checklist + REQ-AUTH-019 regression tests.

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
    Why this proposal anyway: recording the invariant is the zero-cost form of class-level enforcement; the rejected wrapper/facade would have ADDED cost on every ingress.
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Recommended Changes (Medium Convergence)

- ARCHITECTURAL: C-derive — stop trusting two self-declared extra_data facts that are RE-DECLARATIONS of already-signed values. C2: selection weight uses `min(declared bond_count, floor(total_bond / required_bond))` (the sum already exists at `registration.rs:88-94`). C3: `RequestWithdrawal` requires the signed spent-Bond-input owner (`input.public_key`, signature-enforced from genesis at `validation/utxo.rs:124`) to equal `withdrawal_data.producer_pubkey`.
    Convergence: 3/5 (Subtractionist P4, Restructurer P2 B-authz half, Radical DERIVE rule).
    Evidence: `crates/core/src/validation/registration.rs:85-100` (bond_count never compared to posted value — up to 3000× weight inflation, INC-I-177); `crates/core/src/validation/tx_types.rs:547-605` (structural checks only; nothing binds the signer to `producer_pubkey`, INC-I-182). Sidesteps the covenant-witness circularity entirely — no commitment machinery needed for these two.
    Confidence: conf(0.75, converged)
    Caveat (subtraction kill test, PARTIAL on C3): FIFO/partial-withdrawal semantics not fully traced — C3 may need a supplementary node-level holdings check in addition to the input-owner rule (cf. INC-I-180 two-ledger gate). Deploy: AH-gated (own height; changes tx accept/reject) — below-AH bit-identical. Closes INC-I-177 + INC-I-182.

    ━━━ RESOURCE COST — NEGLIGIBLE ━━━
    Dimensions:
      CPU:      0 (observed)
      Memory:   0 (inferred)
      IO:       0 (observed)
      Network:  0 (inferred)
      Disk:     0 (inferred)
      Latency:  0 (inferred)
    Inevitability: INEVITABLE
    Cheaper alternative: NONE-EXISTS
    Why this proposal anyway: one division on an already-computed sum and one pubkey compare on an already-fetched input close two incidents at zero runtime cost, without touching the sighash.
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: Seam B — bind the RESIDUAL extra_data payloads (facts the node cannot derive, e.g. `RegistrationData.bls_pubkey`) to the tx signer: a domain-separated payload-region digest with `authmsg.rs:99` discipline (`DOMAIN ‖ genesis_hash ‖ tx_type ‖ payload ‖ valid_before`, BLAKE3, format-version tag, golden vectors) folded into the input-signature preimage, TX-TYPE-GATED — the Transfer/covenant sighash preimage stays byte-untouched (the SegWit exclusion at `core.rs:511-516` is load-bearing and survives).
    Convergence: 3/5 on the mechanism (Restructurer P2 sighash-fold + Pattern-Matcher P2 authmsg template + Radical P3 hybrid; Pattern's own "cheaper alternative" line concurs with the fold). Scope NARROWED by [F5]: C2/C3 no longer need binding — this closes C1 and completes C5.
    Evidence: `crates/core/src/transaction/core.rs:517-604` (neither sighash arm hashes extra_data — verified root of the C-family); `crates/core/src/maintainer/authmsg.rs:99` (the finished, byte-pinned, reviewed constructor — reuse, not new crypto); `failed_approaches` M2.5 record (testnet block 136690: ungated serialized-shape change bricks sync in BOTH directions).
    Confidence: conf(0.70, converged)
    Preconditions: (1) covenant-path enumeration (analyst Q4) — if any consensus-payload tx can also carry covenant witnesses in the same buffer, escalate to a region split `[payload_len‖payload‖witness]` hashing only the payload; (2) sequence AFTER INC-I-176 M2.5 (Q5); (3) STRICTLY AFTER G1 verified (failures F1 — see Migration). Mempool/consensus accept-reject parity (`pool.rs:512,547,1416,1446` ≡ `validation/utxo.rs:124`) is a hard test obligation. Deploy: AH-gated fenced rolling, own height > 317_861. Closes INC-I-169; completes INC-I-176.

    ━━━ RESOURCE COST — COST-DECLARED ━━━
    Dimensions:
      CPU:      +1 BLAKE3 pass over extra_data at sign/verify, payload tx types only, not per-block-hot (inferred)
      Memory:   +1 transient preimage buffer per payload tx (inferred)
      IO:       0 (inferred)
      Network:  0 (inferred)
      Disk:     0 (inferred)
      Latency:  +sub-millisecond at admission for payload types (inferred)
    Inevitability: INEVITABLE
    Cheaper alternative: NONE-EXISTS
    Why this proposal anyway: facts the node cannot re-derive must be bound to the signer; folding into the existing input signature is the cheapest sound binding (zero wire bytes, zero extra verifies).
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: C4 — constrain Bond outputs at creation: `pubkey_hash` must equal the registering/adding producer, and `amount` must be a whole multiple of `required_bond`. The one C-family item all converging lenses concede is a genuine ADDED constraint (a signed lie is still a lie — binding does not substitute for validity).
    Convergence: 3/5 on need (Subtractionist "does NOT collapse", Restructurer P2 scope, Radical BIND list); mechanism from analyst REQ-AUTH-006 acceptance criteria.
    Evidence: `crates/core/src/validation/transaction.rs:409-427` — today only `lock_until != 0` and `extra_data.len() == 4` are checked; owner and amount unconstrained (INC-I-184).
    Confidence: conf(0.70, converged)
    Deploy: AH-gated, own height > 317_861; below-AH bit-identical. SEQUENCING (failures F1): this closes the poison-AddBond trigger that arms the fleet's only wedge escape — it ships STRICTLY AFTER G1 is verified live, in the same wave as Seam B. Closes INC-I-184.

    ━━━ RESOURCE COST — NEGLIGIBLE ━━━
    Dimensions:
      CPU:      0 (inferred)
      Memory:   0 (inferred)
      IO:       0 (inferred)
      Network:  0 (inferred)
      Disk:     0 (inferred)
      Latency:  0 (inferred)
    Inevitability: INEVITABLE
    Cheaper alternative: NONE-EXISTS
    Why this proposal anyway: no derivation exists for a constraint that must reject values the signer freely chose; two comparisons per Bond output is the minimal form.
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: Phase-2 wire subtraction — delete `attester_weight` and `height` from the `Attestation` wire struct, and delete the production-dead `RegionAggregate`/`from_attestations`. After [F1], nothing reads these fields; deletion makes the regression uncompilable.
    Convergence: 3/5 (Subtractionist P1-phase-2, Restructurer dead-trust-surface signal, Radical wire-shape phasing signal).
    Evidence: synthesizer-verified this session — `attester_weight`'s second consumer (`crates/core/src/attestation.rs:181`, aggregate sum) lives inside `from_attestations`, which has ZERO non-test callers (grep: only a `lib.rs:145` re-export and a doc comment at `block.rs:136`), so deleting the dead aggregate resolves the second-consumer hazard; `height` has zero non-test production consumers. `bls_signature` is KEPT (INC-I-178 Option A composes).
    Confidence: conf(0.70, converged)
    Deploy: bincode is positional → wire-shape change → synchronized deploy (or versioned decoder) + own AH for the shape flip. Mixed-version worst case is silent drop → liveness dip, NEVER forgery acceptance (`from_bytes` → `Option` → `verify()` fails → dropped, `network_events.rs:537`). Scheduled LAST (Migration phase 4). Closes the INC-I-191 residue (REQ-AUTH-003: every carried field signed or deleted).

    ━━━ RESOURCE COST — COST-DECLARED ━━━
    Dimensions:
      CPU:      -tiny, fewer bytes to decode per attestation (inferred)
      Memory:   -1 dead abstraction removed (observed)
      IO:       0 (inferred)
      Network:  -16 B per attestation, two u64 fields removed (measured)
      Disk:     0 (inferred)
      Latency:  0 (inferred)
    Inevitability: AVOIDABLE
    Cheaper alternative: leave the dead fields on the wire (zero deploy cost) — rejected: dead trust surface invites a future consumer to re-read it.
    Why this proposal anyway: field deletion is the strongest enforcement of the DERIVE rule and a net wire/code reduction.
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Options for User Decision

All four are below conf(0.7) — LOW-EVIDENCE tagged. They are choices, not converged verdicts.

**O1 — Snap-sync trust anchor variant (Seam D; closes INC-I-155, hardens INC-I-181) — LOW-EVIDENCE conf(0.65)**
Resolved CONSTRAINT (conf 0.85, converged — patterns kill test + failures F5): the anchor MUST NOT
be maintainer-signed today — INC-I-175 made all 5 bootstrap maintainer keys public, the mainnet
auth-binding AH is `u64::MAX` (below it a maintainer signature is a replayable bearer token), and
a new AH ≤ 317_861 would reorder the pinned chain. This answers policy-Q2's constraint half.
Remaining variant choice (source: Pattern-Matcher P3, single-lens mechanism):
  - **O1a (recommended by patterns): operator-configured weak-subjectivity checkpoint** — an
    out-of-band `(height, state_root)` in node config; candidate roots must descend from it before
    the peer-vote quorum (`snap_sync.rs:17-19`, retained as liveness fast-path) is consulted.
    One config field, one comparison at install. Cost: operator refresh cadence. Restart-only.
  - **O1b: genesis-derived anchor** — zero operator burden, but anchors only early history; weak
    for deep-height snaps.
  - Maintainer-signed variant: revisit ONLY after the INC-I-176 auth-binding AH is live AND keys
    rotated (not foreclosed; not chosen now).

**O2 — C-snap: discard `pending_updates` on install (closes INC-I-181) — LOW-EVIDENCE conf(0.60, converged), GATED on Q3**
4/5 lenses prefer discard (`clear_pending_updates()` already exists, `set_core.rs:69`) over
extending `serialize_canonical` (which changes EVERY state root → AH + root-format hard fork +
cross-AH snap incompatibility — strictly the more dangerous branch, failures F4). BUT all gate on
open Q3: if a snapshot can be cut mid-epoch with a non-empty queue, discarding forks the victim
at its next epoch boundary (INV-4 shape). **Decider probe: trace a mid-epoch snap on the LOCAL
testnet — does the epoch-boundary derivation depend on the serving peer's `pending_updates`?**
If safe → restart-only. If load-bearing → extend-canonical (AH + synchronized) is forced.

**O3 — Seam E: pre-fork-choice admission gate (closes INC-I-160) — LOW-EVIDENCE conf(0.60, converged)**
Authenticate gossip fork blocks (signature + slot-eligibility first, VDF last) BEFORE caching and
fork-choice — but ONLY on the `ReorgCandidate` arm (parent known). The orphan-chase path
(`block_handling.rs:279-296`, Stability Pillar 1) is UNTOUCHED; unknown-parent blocks are
DEFERRED (buffered), never dropped (failures filter 7 — dropping re-creates the INC-I-147 shape).
Kills the `unwrap_or(1)` unknown-producer default weight in fork choice (`block_handling.rs:299-320`,
skip acknowledged at `:1025`). Cost: +sig/VDF verify per fork-candidate block (bounded by
cheap-first ordering). Deploy: AH-gated fenced rolling (AUDIT-P1-203), own height > 317_861.

**O4 — Radical alternative to [F2]: delete the attestation-finality gadget entirely; finality = own confirmation depth (`best_height − 6`) — LOW-EVIDENCE conf(0.50 after failure filters)**
Subtractionist P2: removes `FinalityTracker.pending`, `early_attestations`, `check_finality` —
a whole trusted subsystem — and makes depth-0 finality structurally impossible. Filtered DOWN by
the failures LIVENESS lens: ~60 s product-visible finality (archive flush trails 6 blocks;
whether any external consumer treats ~0.2 s finality as settlement is an unresolved gap), and it
discards the gadget that Seam A + INC-I-178 Option A would make trustworthy. Not chosen; recorded
as the deeper future direction if the user prefers maximum subtraction over fast finality.

## Per-Seam Plan and Deployment Classification

Answers to the two CLAUDE.md deploy questions (Q1 consensus RULES / Q2 block CONTENT) per seam:

| Seam | Root site | Minimal change | Closes | Classification (reason) | Conf |
|---|---|---|---|---|---|
| **A** [F1] | `network_events.rs:552,329` | derive weight/membership via shared helper | INC-I-191, A2-DoS | **RESTART-ONLY iff F2-experiment PASS; else AH.** Q1: honest inputs bit-identical (`startup.rs:604` signer == receiver lookup) → argues NO. Q2: NO (encoder already ProducerSet-filtered, `assembly.rs:400-412`). Empirically open — testnet is the tiebreaker | 0.90 |
| **G1** [F2] | `finality.rs:133` | finalize only at depth ≥ 2, threshold unchanged | INC-I-190 D1 | **RESTART-ONLY under strictly-safer argument, same F2 experiment (isolated run).** Node-local, gates reorg | 0.85 |
| **G2** [F3] | `epoch_state/floor.rs:143-191` | last-known-good fallback | INC-I-190 D2 | **AH-gated, OWN NEW height** (Q1 YES — `active_producers` shape; `epoch_prune` AH crossed/immutable; INV-12 checklist) | 0.85 |
| **C-derive** [F5] | `registration.rs:85-100`, `tx_types.rs:547-605` | bond_count floor + signer==input-owner | INC-I-177, 182 | **AH-gated, own height** (Q1 YES — tx accept/reject; Q2 NO below AH) | 0.75 |
| **B** [F6] | `core.rs:517-604` + `authmsg.rs:99` | tx-type-gated payload-digest in input-sig preimage | INC-I-169; completes 176 | **AH-gated fenced rolling, own height > 317_861, after M2.5, after G1** (Q1 YES; Q2 below-AH byte-identical) | 0.70 |
| **C4** [F7] | `transaction.rs:409-427` | bond owner+amount constraint | INC-I-184 | **AH-gated, own height, STRICTLY after G1** (Q1 YES) | 0.70 |
| **Wire-del** [F8] | `attestation.rs:14-33,181` | delete 2 fields + dead aggregate | 191 residue | **Synchronized deploy (or versioned decoder) + own AH** (bincode positional; worst case silent drop) | 0.70 |
| **D-anchor** O1 | `snap_sync.rs:17-19` | operator/genesis anchor before peer quorum | INC-I-155 | **RESTART-ONLY once variant chosen; maintainer-signed FORBIDDEN today** (Q1/Q2 NO — local admission policy) | 0.65 |
| **C-snap** O2 | `snapshot.rs:239` vs `set_persistence.rs:78`; install `fork_recovery.rs:296-330` | discard on install | INC-I-181 | **discard → RESTART-ONLY (preferred); extend-canonical → AH + root-format hard fork.** Gated on Q3 | 0.60 |
| **E** O3 | `block_handling.rs:299-320,1025` | gate ReorgCandidate arm, defer-not-drop | INC-I-160 | **AH-gated fenced rolling** (Q1 YES — which blocks enter fork choice) | 0.60 |
| **A2-bound** (inside [F1]) | `network_events.rs:329` + `post_commit.rs:405` | membership-gate minute_tracker | un-ticketed DoS | **RESTART-ONLY** (Q1 NO, Q2 NO — pure memory safety; encoder filters) | 0.90 |
| **F** (compose) | — | INC-I-178 Option A — separate approved workstream | INC-I-178 | not this spec; non-foreclosure verified in Constraints | — |

## Incident → Seam Closure Map

Every incident in scope, mapped; none dropped. Residual risk stated per row.

| Incident | Root seam | Requirement | Where in this spec | Residual after closure |
|---|---|---|---|---|
| INC-I-191 (critical, live) | A | REQ-AUTH-001/002/003 | [F1] + [F8] | none for forged weight; bitfield authenticity remains Seam F (178) |
| un-ticketed A2 DirectAttestation DoS | A | REQ-AUTH-018 | [F1] (membership gate) | file the new incident (all 5 lenses concur) |
| INC-I-190 D1 (the RESET) | A-half + G1 | REQ-AUTH-001 + REQ-AUTH-011 | [F1] + [F2] | ≥33%-Byzantine equivocation at depth ≥ 2 — outside the honest model; supersession machinery deliberately NOT built (failures F3: as dangerous as the disease) |
| INC-I-190 D2 (amplifier) | G2 | REQ-AUTH-012 | [F3] | exited producer scheduled ≤1 extra epoch (bounded liveness) |
| INC-I-177 (bond_count 3000×) | C-derive | REQ-AUTH-005 | [F5] | none post-AH |
| INC-I-182 (third-party withdrawal) | C-derive | REQ-AUTH-007 | [F5] | FIFO/partial-withdrawal holdings check pending trace |
| INC-I-169 (registration bearer blob) | B | REQ-AUTH-004 | [F6] | covenant-path enumeration precondition (Q4) |
| INC-I-184 (unconstrained bonds) | C4 | REQ-AUTH-006 | [F7] | none post-AH; sequenced after G1 (lifeboat coupling) |
| INC-I-176 (maintainer bearer token) | B (C5) | REQ-AUTH-004 | M1a+M2 done; M2.5 in flight; [F6] completes the class | AH 317_861 chain untouched |
| INC-I-181 (pending_updates smuggling) | C-snap | REQ-AUTH-008 | O2 (Q3-gated) | until O2 ships: root-check still passes fabricated pending_updates |
| INC-I-155 (Sybil snap quorum) | D-anchor | REQ-AUTH-009 | O1 (variant = user policy) | until O1 ships: peer majority chooses the root |
| INC-I-160 (unauthenticated fork choice) | E | REQ-AUTH-010 | O3 | until O3 ships: forged-producer block reaches fork choice with weight 1 |
| INC-I-178 (bitfield unverified) | F | REQ-AUTH-024 (Won't — compose) | separate approved workstream (Option A) | WHITEPAPER §10.3 hotfix stays ACTIVE until Option A lands |

## Constraints (any chosen path MUST honor)

1. **G1 ≺ {Seam B, C4, INV-PROD-003 builder-parity, INV-PROD-002 enforcement, ai5-cron stop}**
   (failures F1). The poison rollback (`production/mod.rs:630` → `rollback.rs:318`) is the ONLY
   path that clears `last_finality_height` and the fleet's only wedge escape (13/27 in
   INC-I-190). Closing its trigger before G1 → 27/27 fleet snap on the next tie.
2. **The first fleet restart carrying ANY part of this redesign carries G1-depth + G2(code) + Seam A** — deploying is itself the INC-I-190 trigger.
3. **Seam A and G1 keep BOTH deploy paths open** until the F2 mixed-version experiment returns
   (the user's PROVE-FIRST decision — binding). Experiment binaries differ ONLY in the change under test.
4. **No threshold raise, no deep-confirmation finality** (failures LIVENESS filter) — archive
   flush + production gate depend on finality advancing during a blackout.
5. **AH discipline**: every consensus-visible seam gets its OWN NEW `*_activation_height`
   (NetworkParams, never constants); never reuse `epoch_prune`/`addbond_cap`/`inc_i_173`/
   `withdrawal_holdings` (crossed → immutable); no new AH ≤ 317_861 (pinned #20/#21/#22 chain,
   REV-176-M1a-001 ordering asserted by test). CLAUDE.md #0: forward-only activation, NO genesis reset.
6. **Snap anchor may not be maintainer-signed** below the live INC-I-176 auth-binding AH (F5 circularity + INC-I-175 public keys).
7. **Orphan chase untouched** (Stability Pillar 1): any Seam E gate defers, never drops, unknown-parent blocks.
8. **The SegWit exclusion survives**: no change ever folds the covenant WITNESS back into `signing_message*`; Transfer/covenant preimages stay byte-identical at every height.
9. **INV-ATTEST-001**: attestation gating uses `is_active()`, never `weight==0` (delegated-away producers must still attest).
10. **Mempool/consensus parity**: any Seam B change keeps `pool.rs` and `validation/utxo.rs:124` at bit-identical accept/reject at every height; offline signers (channels/CLI) byte-identical below AH.
11. **Non-foreclosure**: INC-I-178 Option A composes ([F1] derives weight at ingress, [F8] keeps `bls_signature`, G1's justified→finalize split strengthens it); the AH-317_861 maintainer activation and INC-I-176 M2.5 proceed unchanged (Seam B sequences after M2.5).
12. **Encoder/decoder index parity** (Full Bitfield Decode pillar) is untouched by every change in this spec — [F1] filters at ingress, before the encoder's already-filtered universe.

## Architecture Maps

### Current (trust flows)
```
wire Attestation.attester_weight --trusted--> finality numerator -> 67% at depth 0 -> reorg refusal -> only exit = poison-rollback bug / full snap
wire Attestation.attester --unbounded--> minute_tracker (HashMap, attacker-keyed)
wire extra_data (bearer blob, outside sighash) --trusted--> producer admission / weight / withdrawal / bond ledger
wire snap payload (bincode superset of root coverage) --trusted--> installed ProducerSet (pending_updates unauthenticated)
peer-vote quorum --chooses--> snap state root (Sybil-votable)
gossip fork block --cached unverified--> fork choice (unknown producer weight := 1)
attestation blackout --> floor fallback := ENTIRE registry (30->84) -> adjacent-rank ties
```

### Proposed (after Definite + Recommended; Options as chosen)
```
local ProducerSet --derives--> finality numerator AND denominator (one helper, both ingresses); non-members never recorded
finality granted only at depth >= 2 -> ties cannot self-finalize -> wedge class removed -> poison-rollback retired (phase 4)
signed tx structure --derives--> bond_count, withdrawal authorization; Bond outputs constrained at creation
input signature --commits (tx-type-gated digest)--> residual extra_data payloads; covenant witness untouched
state root coverage == installed fields (pending_updates discarded, per O2); anchor = operator/genesis checkpoint (per O1)
fork candidates authenticated before choice (per O3); orphan chase unchanged
blackout fallback := last-known-good producer_list (bounded, inductive floor)
wire Attestation = {block_hash, slot, attester, signature, bls_signature} -- nothing forgeable left
```

## Migration Path (strict ordering — violating it reproduces INC-I-190 at fleet scale)

**Phase 0 — probes (no deploy):**
1. F2 mixed-version tied-fork experiment on the LOCAL testnet (12 nodes, half fixed) — TWO
   isolated runs: (a) Seam-A-only binary, (b) G1-depth-only binary. PASS criteria per the
   failures spec (identical finalized-height sequence + canonical tip; no fixed-vs-stock split a
   stock-vs-stock pair would not also show). Decides restart-only vs AH for [F1]/[F2].
2. Q3 trace: mid-epoch snap on the LOCAL testnet — is `pending_updates` load-bearing at install?
   Decides O2's branch.
3. File the new incident for the A2 `DirectAttestation` ingress (REQ-AUTH-018).
4. (Optional, closes the last uncertainty in contradiction C-1): audit the h=314591 mainnet
   attestation stream — carried `attester_weight` vs derived `selection_weight_at(314591)`;
   expected equal (honest).

**Phase 1 — the survivability deploy (ONE binary; the only unprotected restart):**
Contains [F1] Seam A + [F2] G1-depth + [F3] G2 code (dormant below its AH, byte-identical) +
A2 bound (+ O1/O2 if decided). Rolling restart if Phase 0 passed for both A and G1; otherwise
ship the code AH-gated/dormant and flip at heights (or synchronized, per user choice — Q4).
- BRIDGE: the unconditional poison-rollback (`production/mod.rs:630`) is DELIBERATELY RETAINED —
  it stays the fleet's only wedge escape until G1-depth is verified live. Temporary; removed in
  Phase 4. (INV-PROD-002 stays UN-enforced through Phases 1-3.)
- BRIDGE: `attester_weight`/`height` stay on the wire as ignored bytes (read by nobody after
  [F1]) until Phase 4's synchronized wire deletion.

**Phase 2 — AH crossings, no restart:** G2's activation height crosses → fork-generator disarmed
fleet-wide atomically. Verify via gauntlet + one observed natural restart: no wedge, no snap.

**Phase 3 — the tx-validity wave (each its OWN AH > 317_861, fenced so 100% upgrade precedes the flip):**
[F5] C-derive; then [F6] Seam B (AFTER INC-I-176 M2.5 lands — Q5) and [F7] C4 together in one
binary behind separate AHs. HARD GATE: Phase 3 may not begin until Phase 2 verification confirms
G1 live (Constraint 1 — these changes delete the lifeboat's trigger). O3 Seam E rides this wave
on its own AH if chosen.

**Phase 4 — cleanup subtractions (only after G1 verified across ≥1 full epoch + gauntlet):**
[F8] wire deletion + RegionAggregate deletion (synchronized deploy or versioned decoder);
INV-PROD-002 enforcement (remove the poison-rollback BRIDGE); INV-PROD-003 builder-parity; stop
the ai5 auto-bond cron (INC-I-180). All four sit behind Constraint 1's gate.

## The SSF Candidate (single highest-leverage first step, stated alone)

**Seam A weight-binding [F1]: one shared helper, two call sites, using state already in hand.**
It closes the LIVE critical hole (INC-I-191 — any keypair can currently finalize an arbitrary
known block on a remote node) at both ingresses, and the unbounded-memory DoS, with a net
code/trust reduction and near-zero runtime cost. It can ship alone, before everything else,
pending only the Phase-0 experiment for its deploy class.

**What it does NOT do (honest scope — verdict-based):** it does NOT prevent the INC-I-190 reset
class. That event's ~72% finality weight was HONEST (memory.db verdict; a fixed node derives the
same numerator), so an honest tie still self-finalizes at depth 0 and still wedges the fleet —
that requires [F2] G1-depth; the fork-generator remains until [F3] G2; the poison-rollback
remains the only escape until Phase 4. It also closes none of the C-family/snap/fork-choice
incidents. Choose it as a standalone first step for the live hole — not as the reset fix.

## Complexity Comparison

| Metric | Current | Radical Minimum | Proposed |
|---|---|---|---|
| Untrusted ingress rows feeding consensus | 13 | 0 | 0 (after Phase 4 + options) |
| Distinct design mechanisms | 5 seams + 2 amplifiers | 2 rules + 1 subtraction + 1 safety fix | same as radical (framing adopted) |
| Modules (new) | — | 0 | 0 (1 helper fn; 1 constructor sibling of authmsg) |
| Interfaces (new) | proposed facade (REQ-021) | 0 | 0 (facade + wrapper REJECTED) |
| Abstractions (new types) | proposed `Authenticated<T>` | 0 | 0, minus 1 dead (`RegionAggregate`) |
| Dependencies (trust edges wire→consensus) | 13 | 0 | 0; ProducerSet becomes the single authority hub |
| New activation heights | — | ~5 | 5 (G2, C-derive, B, C4, E) + wire-shape AH + conditional A/G1 |
| Unprotected fleet restarts | up to 8 (one/seam) | 1 | 1 (Phase 1) + AH crossings |
| Wire bytes per attestation | baseline | −16 B | −16 B (Phase 4) |

Proposed == radical minimum plus the failures-mandated sequencing. No gold-plating survived the
radical tiebreaker (log in the reasoning trace).

## Open Policy Questions for the User (surfaced, NOT resolved)

- **Q2 (variant half)**: O1a operator-configured checkpoint vs O1b genesis-derived anchor.
  (Constraint half is RESOLVED: not maintainer-signed today.)
- **Q3**: is `pending_updates` load-bearing at snap-install points? Probe named in Phase 0.2;
  decides O2 discard (restart-only) vs extend-canonical (AH + root hard fork).
- **Q4**: exact Phase-1 choreography — rolling vs AH-gated vs synchronized for Seam A + G1,
  pending the F2 experiment result; and the concrete AH values for Phases 2-4 (all > 317_861).
- **Q5**: Seam B vs INC-I-176 M2.5 — this spec sequences B strictly after M2.5; confirm M2.5's
  in-flight status before scheduling Phase 3.
- Also for decision: O3 (Seam E gate) yes/no; O4 (radical finality-gadget deletion) as a future
  direction; whether to run the optional h=314591 log audit.

## Milestones (touches 4+ modules → milestone-gated)

- **M0** = Phase 0 probes (F2 experiment ×2, Q3 trace, new A2 incident, optional log audit).
- **M1** = Phase 1 survivability binary: [F1]+[F2]+[F3-code]+A2-bound, with per-seam mutated-field
  regression tests (REQ-AUTH-019) and the INV-AUTH-002 memory.db row.
- **M2** = Phase 2: G2 AH crossing + gauntlet + restart-survival verification.
- **M3** = Phase 3: [F5], then [F6]+[F7] (post-M2.5), each own AH; mempool-parity + golden-vector
  + wire-compat tests (REQ-AUTH-020); O3 if chosen.
- **M4** = Phase 4: [F8] wire deletion, INV-PROD-002 enforcement, builder parity, cron stop.
- **M5** = docs/spec sync: WHITEPAPER §10.3 hotfix remains ACTIVE until INC-I-178 Option A;
  `specs/security_model.md` gains the ingress-table trust boundaries; fix the misleading
  `attestation.rs:373-375` doc comment.

## Design Synthesis Quality Gate

```
━━━ DESIGN SYNTHESIS QUALITY GATE ━━━
Evaluators completed:           5/5
Deletion convergence items:     4 (3+/5 agreement: Seam A 5/5, G1 4/5, G2 5/5, wrapper-rejection 3/5+2 implicit)
Restructuring convergence:      4 (C-derive, Seam B mechanism, C4, wire-deletion — all 3/5)
Addition options presented:     4 (O1 anchor variant, O2 C-snap, O3 E-gate, O4 gadget-deletion)
Failure modes identified:       9 (failures filters 1-9)
Failure modes applied as filters: 9/9 (matrix in reasoning trace; no unmitigated VULNERABLE survived)
Radical floor gap:              13 untrusted rows → 0 (radical min) → 0 (proposed); proposed == radical minimum + sequencing
Contradictions found:           5 (4 assigned + 1 discovered: G1 mechanism)
Contradictions resolved:        5/5 (C-1 by verdict evidence; C-2 by failures table + open experiment; C-3 encoded as ordering; C-4 by kill test; C-5 by failure-filter + radical tiebreaker)
Evidence independence verified: YES (per-cluster, reasoning trace)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```
