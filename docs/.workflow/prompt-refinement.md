# Prompt Refinement — /omega-redesign --incident=INC-I-139 --fix (RUN 455)

Original: /omega-redesign --incident=INC-I-139 --fix (continuation: implement the spec approved 2026-07-10 in this session)

Anchors detected: (none detected — continuation of an approved, evaluator-converged spec; no layer/depth/cause anchors in the invocation)

Domain context preserved: Incident INC-I-139 (N1/N4 disproportionate SnapSync at gap≈50 on fresh mainnet, 4th recurrence of the unguarded-snap-admission class INC-I-005/033/138/139). Approved spec: specs/sync-snap-admission-architecture.md. Phase 1 rolling-safe subtraction: DC-1 delete Route A (decision.rs:168), DC-2 floor-gate forward exemption (production_gate.rs:674-681, atomic with DC-1), DC-3 delete A1 redirect (dispatch.rs:96-117), DC-4 counter single-owner (dispatch.rs:84), RC-1 threshold demotion + discv5 h==0 gate, RC-2 emergency taxonomy sentinel. 8 regression-test classes defined in spec; classes 2 and 3 must FAIL against current code before any source edit; class 4 must FAIL against Route-A deletion without DC-2.

⚠️ CONSTRAINT: Phase 2 (M8-M10, fork_choice_weight_tiebreak_activation_height wedge-escape) is OUT of this run — separate deploy and decision session.
⚠️ CONSTRAINT: strict TDD gate; no version bumps without explicit approval; commits authored "Antonio Lozada <antonio@omegacortex.ai>"; rolling-safe verdict already established (node-local recovery behavior, no block content, no consensus rules).
⚠️ CONSTRAINT: system-impact protocol applies (.omega/gauntlet.conf present): Failure-Modes: commit blocks, protection_mechanisms registration, passing gauntlet before close.

Refined: Execute Phase 1 (M1-M7) of the approved sync-snap-admission architecture via the standard milestone loop — tests first (M1, FAIL-verified), then the atomic DC-2+DC-1 commit, then DC-3, DC-4, RC-1/RC-2, then institutional close-out with gauntlet. Follow the spec's migration path exactly; do not freelance beyond it; Phase 2 deferred.
