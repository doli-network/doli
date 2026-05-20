# Role Audit — regression-reproducer

**Auditor**: role-auditor v2.0 (adversarial D1–D12)
**Target**: `.claude/agents/regression-reproducer.md`
**Workflow**: /omega-create-role RUN_ID=344
**Date**: 2026-05-19

## Verdict: HARDENED

| Severity | Count |
|----------|-------|
| CRITICAL | 0 |
| MAJOR    | 0 |
| MINOR    | 10 |

**Anatomy score: 14/14** — identity, boundaries, prerequisite, dir_safety, source_of_truth, context_mgmt, process, output_format, rules, anti_patterns, failure_handling, integration, scope_handling, context_limits — all present.

Zero critical, zero major. The role definition is comprehensive and operationally sound. All DOLI safety invariants present and explicitly enforced (no mainnet, no pkill, codesign, wallet backup, pending_update cleanup, mandatory launch flags, correct script paths, genesis-reset-as-last-resort). Autonomy/safety coexistence handled well (autonomy stated, then immediately bounded by non-negotiable invariants). Boundaries with all sibling agents explicit and non-overlapping. Deterministic process with exact Bash commands. Parseable output template.

### Caller's high-risk audit angles — all WELL DEFENDED
1. Scope-creep (autonomy → mainnet/ai-servers): defended (Rule 8 absolute, line 35 explicit "full autonomy does not relax it").
2. Safety-invariant bypass via autonomy framing: defended (lines 10–11; safety rules use NEVER/absolute, not qualified by autonomy).
3. Prerequisite soundness (refusing non-regressions): defended (Prereq #2 requires baseline, routes elsewhere).
4. Boundary with diagnostician: defended (line 28, Rule 15, anti-pattern #4 — stop at bisect).
5. Determinism rigor (≥3-run): defended (lines 258–264, anti-patterns #2/#3).
6. launchd reality: defended (Rule 4, testnet.sh throughout, correct localdoli script path).

## Minor Findings

| ID | Dim | Finding | Status |
|----|-----|---------|--------|
| D4-2 / D8-3 | D4/D8 | Rule 5 codesigned only `doli-node`; Phase 3 correctly codesigns both. Rule incomplete vs procedure → risk of unsigned CLI binary killed by macOS. | **FIXED** — Rule 5 now mandates copy+codesign BOTH `doli-node` and `doli`, with the failure consequence spelled out. |
| D8-1 | D8 | Rule 16 (anti-overengineering Q0) aspirational/irrelevant to a reproducer. | **FIXED** — reframed Rule 16 as "Minimal trigger sequence" (remove-and-re-run reduction, observable in the recipe). |
| D8-2 | D8 | Rule 17 (500-line file limit) irrelevant — agent writes markdown, not source. | **FIXED** — Rule 17 removed; old Rule 18 (n13-n17 data path) renumbered to 17. |
| D9-1 | D9 | Missing anti-pattern for raw `kill`/`pkill` during rapid bisect cycling (highest-risk LLM trap for this role). | **FIXED** — added explicit anti-pattern explaining launchd respawn → mid-bisect version-mixed fork → wrong result. |
| D6-1 | D6 | No explicit failure handling for out-of-scope user requests (fix / root-cause / write test). | **FIXED** — added Failure Handling row: acknowledge, refuse, record in Hand-Off, name correct agent, continue. |
| D2-1 | D2 | blockchain-debug boundary only in Integration, not in Boundaries section. | **FIXED** — added "Troubleshoot active infrastructure faults" boundary distinguishing operate-to-reproduce vs diagnose-infrastructure. |
| D3-1 | D3 | Baseline commit existence validated late (Failure Handling) not at the gate. | **FIXED** — Prereq #2 now verifies `git rev-parse --verify "<commit>^{commit}"` at the gate with STOP. |
| D1-1 | D1 | Identity lists 4 sub-activities (reproduce/classify/bisect/handoff). | Accepted — coherent single pipeline; cosmetic, not functional. No change. |
| D11-1 | D11 | No companion command (discoverability). | **FIXED** — created `.claude/commands/omega-reproduce.md`; Integration section updated. |
| D2-2 | D2 | No explicit boundary vs investigator/log-forensics. | Accepted — mitigated by strong stop-at-bisect rule. No change. |

## Remediation Summary

8 of 10 minor findings fixed directly in `.claude/agents/regression-reproducer.md` (Phase 3, automatic). 2 accepted as cosmetic with documented rationale (D1-1, D2-2). Companion command `omega-reproduce.md` created. Verdict was already HARDENED before remediation (no critical/major); fixes are quality hardening, not deployment-blockers — no re-audit cycle required per workflow (re-audit is mandated only for broken/degraded verdicts).

## Post-Audit User Correction (2026-05-19)

The audit did NOT flag this and the role-creator missed it: **genesis reset must require explicit user approval**, even under "full autonomy on the local testnet". User caught this during review. The original Rule 11 ("agent may reset autonomously but must record justification") was wrong — destructive operations are never covered by autonomy grants (CLAUDE.md #0 RULE: hours of downtime, loses on-chain state; most DOLI regressions reproduce at a future height without resetting).

Applied to `.claude/agents/regression-reproducer.md`:
- Opening paragraph (line 10) — autonomy statement narrowed to routine ops; genesis reset called out as the ONE explicit exception
- Rule 11 — rewritten: "Genesis reset requires EXPLICIT user approval — always". STOP, present evidence, wait for approval. Never autonomously, never to "save time"
- Phase 3 procedure (genesis-reset path) — now STOPs and asks before any wipe; only proceeds after explicit approval
- Failure Handling row — "Regression appears to require a genesis reset" now requires approval, not autonomous reset

**Audit blind spot worth recording:** D2/D8 dimensions assessed "autonomy/safety coexistence" as well-defended, but only against autonomy leaking outside the local-testnet scope. They did NOT test whether the autonomy framing covered specific destructive ops *inside* scope. Future role audits with autonomy grants should explicitly partition: (a) routine ops covered by autonomy, (b) destructive ops carved out and requiring approval regardless of autonomy. This is a Cortex-shareable auditor improvement.

## Residual Risks (runtime, not specification)
- Runtime compliance with safety invariants is unverifiable from spec alone — recommend an integration shakedown on a known past regression (e.g., INC-I-082 / INC-I-075) before relying on it unattended.
- Long bisect ranges (50+ commits) may pressure context despite output filtering; progress-save heuristic present but not step-quantified.
